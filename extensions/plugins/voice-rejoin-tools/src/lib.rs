/*
 * Serein voice rejoin tools
 * Copyright (c) 2026 Serein contributors
 *
 * Safe port of Equicord's `voiceRejoin` plugin (Copyright (c) 2024 Vendicated
 * and contributors), GPL-3.0-or-later, at upstream commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e.
 * Changed November 8, 2026: only one last voice channel ID is remembered, and
 * rejoining is an explicit Apply-gated JoinVoice proposal. No automatic
 * reconnection, no timers, no ring and no unmuted join.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppAction, AppEventKind, AppInvocation, AppOutput, AppSnapshot, ChannelSnapshot, Element,
	HostEffect, Output, VoiceSnapshot, serde_json,
};

/// Only a tiny strict object is persisted. The host storage budget is far
/// larger, but a saved channel is a single decimal ID.
const MAX_STORAGE_BYTES: usize = 512;
/// The complete panel stays well below the 256 KiB ABI limit.
const MAX_RESPONSE_BYTES: usize = 4 * 1024;
const MAX_CHANNEL_ID_BYTES: usize = 20;
const MAX_LABEL_BYTES: usize = 128;

/// A clear value is the valid empty object, never an empty string. The host
/// persists `Some("")` as a file, so `{}` keeps the next load non-corrupt.
const EMPTY_STORAGE: &str = "{}";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Saved {
	Empty,
	Valid(String),
	/// Preserved exactly as received. Only an explicit Save or Forget replaces it.
	Corrupt,
}

fn valid_id(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= MAX_CHANNEL_ID_BYTES
		&& !value.starts_with('0')
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn load_storage(raw: Option<&str>) -> Saved {
	let Some(raw) = raw else {
		return Saved::Empty;
	};
	if raw.len() > MAX_STORAGE_BYTES {
		return Saved::Corrupt;
	}
	let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
		return Saved::Corrupt;
	};
	let Some(object) = value.as_object() else {
		return Saved::Corrupt;
	};
	if object.is_empty() {
		return Saved::Empty;
	}
	if object.len() != 1 {
		return Saved::Corrupt;
	}
	let Some(channel_id) = object.get("channel_id").and_then(|value| value.as_str()) else {
		return Saved::Corrupt;
	};
	if !valid_id(channel_id) {
		return Saved::Corrupt;
	}
	Saved::Valid(channel_id.to_owned())
}

fn saved_storage(channel_id: &str) -> String {
	format!("{{\"channel_id\":\"{channel_id}\"}}")
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
	if value.len() <= max_bytes {
		return value;
	}
	let mut end = max_bytes;
	while end > 0 && !value.is_char_boundary(end) {
		end -= 1;
	}
	&value[..end]
}

fn display_name(value: &str) -> String {
	let cleaned = value
		.chars()
		.filter(|character| !character.is_control())
		.collect::<String>();
	let trimmed = cleaned.trim();
	if trimmed.is_empty() {
		"Unnamed".into()
	} else {
		truncate_utf8(trimmed, MAX_LABEL_BYTES).to_owned()
	}
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn button(id: &str, label: &str) -> Element {
	Element::Button {
		id: id.into(),
		label: label.into(),
	}
}

fn current_call(app: Option<&AppSnapshot>) -> Option<&VoiceSnapshot> {
	app.and_then(|app| app.voice.as_ref())
}

fn current_channel_id(app: Option<&AppSnapshot>) -> Option<&str> {
	current_call(app)
		.and_then(|voice| voice.channel_id.as_deref())
		.filter(|id| valid_id(id))
}

fn loaded_channel<'a>(
	app: Option<&'a AppSnapshot>,
	channel_id: &str,
) -> Option<&'a ChannelSnapshot> {
	let directory = app?.channels.as_ref()?;
	directory
		.items
		.iter()
		.find(|channel| channel.id == channel_id)
}

fn channel_label(app: Option<&AppSnapshot>, channel_id: &str) -> String {
	loaded_channel(app, channel_id).map_or_else(
		|| format!("channel [{channel_id}]"),
		|channel| format!("{} [{channel_id}]", display_name(&channel.name)),
	)
}

fn is_voice_kind(kind: u8) -> bool {
	// DM, group DM, guild voice and stage channels can host a call.
	matches!(kind, 1 | 2 | 3 | 13)
}

fn in_call_note(app: Option<&AppSnapshot>, saved: &Saved) -> Option<String> {
	let current = current_channel_id(app)?;
	let label = channel_label(app, current);
	match saved {
		Saved::Valid(saved_id) if saved_id.as_str() == current => {
			Some(format!("You are already in the saved channel {label}."))
		}
		Saved::Valid(_) => Some(format!(
			"You are currently in {label}. Leave that call before proposing a rejoin; Serein will not switch an active call."
		)),
		_ => Some(format!("Current call: {label}.")),
	}
}

fn panel(saved: &Saved, app: Option<&AppSnapshot>, notice: Option<&str>) -> Vec<Element> {
	let current = current_channel_id(app);
	let mut panel = vec![
		Element::Heading {
			text: "Voice rejoin".into(),
		},
		text(
			"This stores one last voice channel locally. It never rejoins automatically and never rings anyone.",
		),
	];
	if let Some(note) = in_call_note(app, saved) {
		panel.push(text(note));
	}
	match saved {
		Saved::Empty => panel.push(text(
			"No channel is saved yet. Join a voice channel and use Save current channel, or let the Voice event remember it.",
		)),
		Saved::Valid(channel_id) => {
			panel.push(text(format!(
				"Saved channel: {}",
				channel_label(app, channel_id)
			)));
			if let Some(channel) = loaded_channel(app, channel_id) {
				panel.push(text(format!(
					"The loaded channel entry reports kind {} (2 is a guild voice channel; 13 is a stage).",
					channel.kind
				)));
			}
		}
		Saved::Corrupt => panel.push(text(
			"Saved channel data is unreadable and was preserved. Use Save current channel or Forget to replace it explicitly.",
		)),
	}
	if current.is_some() {
		panel.push(button("save-current", "Save current channel"));
	}
	if matches!(saved, Saved::Valid(_)) && current.is_none() {
		panel.push(button("rejoin", "Propose rejoining saved channel"));
	}
	if !matches!(saved, Saved::Empty) {
		panel.push(button("forget", "Forget saved channel"));
	}
	if let Some(notice) = notice {
		panel.push(text(notice));
	}
	panel.push(text(
		"Rejoin is a proposal. Serein joins only after you choose Apply, joins muted with no ring, and does not retry. Unmute after joining.",
	));
	panel.push(text(
		"The saved channel is kept until you replace or forget it. App snapshots carry no clock and no roster for other channels, so this port cannot enforce a disconnect time window or verify that the call is still populated.",
	));
	panel
}

fn panel_output(panel: Vec<Element>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		..Default::default()
	}
}

fn bounded(mut output: AppOutput) -> AppOutput {
	let too_large =
		serde_json::to_vec(&output).map_or(true, |bytes| bytes.len() > MAX_RESPONSE_BYTES);
	if too_large {
		output.output.panel = vec![text(
			"The voice rejoin panel exceeded its 4 KiB response limit. No action was taken.",
		)];
		output.effects.clear();
	}
	output
}

fn observe(input: &AppInvocation) -> AppOutput {
	if input.app_event != Some(AppEventKind::Voice) {
		return AppOutput::default();
	}
	let Some(app) = input.app.as_ref() else {
		return AppOutput::default();
	};
	let Some(channel_id) = current_channel_id(Some(app)) else {
		// A disconnect keeps the last channel for an explicit rejoin proposal.
		return AppOutput::default();
	};
	match load_storage(input.invocation.storage.as_deref()) {
		Saved::Valid(saved) if saved.as_str() == channel_id => AppOutput::default(),
		Saved::Corrupt => AppOutput::default(),
		Saved::Empty | Saved::Valid(_) => AppOutput {
			output: Output {
				storage: Some(saved_storage(channel_id)),
				..Default::default()
			},
			..Default::default()
		},
	}
}

fn save_current(input: &AppInvocation) -> AppOutput {
	let app = input.app.as_ref();
	let Some(channel_id) = current_channel_id(app) else {
		return bounded(panel_output(panel(
			&load_storage(input.invocation.storage.as_deref()),
			app,
			Some("Join a voice channel before saving the current channel. Nothing was changed."),
		)));
	};
	let saved = load_storage(input.invocation.storage.as_deref());
	if saved == Saved::Valid(channel_id.to_owned()) {
		return bounded(panel_output(panel(
			&saved,
			app,
			Some("That channel is already the saved channel. Nothing was changed."),
		)));
	}
	let next = Saved::Valid(channel_id.to_owned());
	let mut output = panel_output(panel(
		&next,
		app,
		Some("Saved the current channel locally. Nothing was joined or changed."),
	));
	output.output.storage = Some(saved_storage(channel_id));
	bounded(output)
}

fn forget(input: &AppInvocation) -> AppOutput {
	let app = input.app.as_ref();
	match load_storage(input.invocation.storage.as_deref()) {
		Saved::Empty => bounded(panel_output(panel(
			&Saved::Empty,
			app,
			Some("There is no saved channel to forget."),
		))),
		Saved::Valid(_) | Saved::Corrupt => {
			let mut output = panel_output(panel(
				&Saved::Empty,
				app,
				Some("Forgot the saved channel. Nothing else was changed."),
			));
			output.output.storage = Some(EMPTY_STORAGE.into());
			bounded(output)
		}
	}
}

fn rejoin(input: &AppInvocation) -> AppOutput {
	let app = input.app.as_ref();
	let saved = load_storage(input.invocation.storage.as_deref());
	let channel_id = match saved {
		Saved::Valid(channel_id) => channel_id,
		other => {
			return bounded(panel_output(panel(
				&other,
				app,
				Some("No saved channel can be rejoined. Nothing was proposed."),
			)));
		}
	};
	let Some(voice) = current_call(app) else {
		return bounded(panel_output(panel(
			&Saved::Valid(channel_id),
			app,
			Some("Voice state is unavailable. Nothing was proposed."),
		)));
	};
	if let Some(current) = voice.channel_id.as_deref() {
		let notice = if current == channel_id.as_str() {
			"You are already in the saved channel. Nothing was proposed."
		} else {
			"You are in another call. Leave it before proposing a rejoin. Nothing was proposed."
		};
		return bounded(panel_output(panel(
			&Saved::Valid(channel_id),
			app,
			Some(notice),
		)));
	}
	if let Some(directory) = app.and_then(|app| app.channels.as_ref())
		&& !directory.truncated
	{
		match loaded_channel(app, &channel_id) {
			None => {
				return bounded(panel_output(panel(
					&Saved::Valid(channel_id),
					app,
					Some(
						"The saved channel is not in the complete loaded channel list. Nothing was proposed because Serein will not guess a missing destination.",
					),
				)));
			}
			Some(channel) if !is_voice_kind(channel.kind) => {
				return bounded(panel_output(panel(
					&Saved::Valid(channel_id),
					app,
					Some(
						"The saved channel is loaded as a non-voice channel. Nothing was proposed.",
					),
				)));
			}
			Some(_) => {}
		}
	}
	let label = channel_label(app, &channel_id);
	let output = AppOutput {
		output: Output {
			panel: vec![
				Element::Heading {
					text: "Voice rejoin proposal".into(),
				},
				text(format!("Proposed channel: {label}")),
				text(
					"Serein proposes joining muted, with no ring and no retry, only after you choose Apply. Unmute manually after joining.",
				),
			],
			..Default::default()
		},
		effects: vec![HostEffect::AppAction {
			action: AppAction::JoinVoice {
				channel_id: channel_id.clone(),
				ring: false,
				muted: true,
				deafened: false,
			},
		}],
	};
	bounded(output)
}

fn handle(input: AppInvocation) -> AppOutput {
	if input.invocation.action == "observe" {
		return observe(&input);
	}
	if input.app_event.is_some() || input.message_event.is_some() {
		return AppOutput::default();
	}
	let app = input.app.as_ref();
	let saved = load_storage(input.invocation.storage.as_deref());
	match input.invocation.action.as_str() {
		"show" => bounded(panel_output(panel(&saved, app, None))),
		"save-current" => save_current(&input),
		"forget" => forget(&input),
		"rejoin" => rejoin(&input),
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	fn run(input: serde_json::Value) -> AppOutput {
		let bytes = dispatch_typed(&serde_json::to_vec(&input).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	fn panel_text(output: &AppOutput) -> String {
		output
			.output
			.panel
			.iter()
			.filter_map(|element| match element {
				Element::Text { text } => Some(text.as_str()),
				Element::Button { label, .. } => Some(label.as_str()),
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n")
	}

	fn app_json(channel: Option<&str>) -> serde_json::Value {
		let phase = if channel.is_some() {
			"connected"
		} else {
			"idle"
		};
		serde_json::json!({
			"voice": {
				"channel_id": channel,
				"phase": phase,
				"muted": false,
				"deafened": false,
				"camera": false,
				"streaming": false,
				"participants": []
			}
		})
	}

	fn app_with_channels(
		channel: Option<&str>,
		id: &str,
		name: &str,
		kind: u8,
		truncated: bool,
	) -> serde_json::Value {
		let mut app = app_json(channel);
		app["channels"] = serde_json::json!({
			"items": [{"id": id, "guild_id": "9", "name": name, "kind": kind}],
			"truncated": truncated
		});
		app
	}

	#[test]
	fn observe_remembers_only_a_changed_valid_channel() {
		let first = run(serde_json::json!({
			"action": "observe",
			"app_event": "voice",
			"app": app_json(Some("123"))
		}));
		let stored: serde_json::Value =
			serde_json::from_str(first.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(stored["channel_id"], "123");
		assert!(first.output.panel.is_empty());
		assert!(first.effects.is_empty());

		let unchanged = run(serde_json::json!({
			"action": "observe",
			"app_event": "voice",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_json(Some("123"))
		}));
		assert!(unchanged.output.storage.is_none());

		let disconnected = run(serde_json::json!({
			"action": "observe",
			"app_event": "voice",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_json(None)
		}));
		assert!(disconnected.output.storage.is_none());

		let invalid = run(serde_json::json!({
			"action": "observe",
			"app_event": "voice",
			"app": app_json(Some("not-an-id"))
		}));
		assert!(invalid.output.storage.is_none());

		let other_event = run(serde_json::json!({
			"action": "observe",
			"app_event": "ready",
			"app": app_json(Some("123"))
		}));
		assert!(other_event.output.storage.is_none());
	}

	#[test]
	fn observe_never_repairs_corrupt_storage_implicitly() {
		for storage in [
			"not-json",
			"[]",
			"{\"channel_id\":\"abc\"}",
			"{\"channel_id\":\"123\",\"extra\":1}",
			"{\"channel_id\":123}",
		] {
			let observed = run(serde_json::json!({
				"action": "observe",
				"app_event": "voice",
				"storage": storage,
				"app": app_json(Some("123"))
			}));
			assert!(observed.output.storage.is_none(), "{storage}");

			let shown = run(serde_json::json!({
				"action": "show",
				"storage": storage,
				"app": app_json(None)
			}));
			assert!(
				panel_text(&shown).contains("unreadable and was preserved"),
				"{storage}"
			);
			assert!(shown.effects.is_empty());
		}
	}

	#[test]
	fn show_resolves_a_loaded_channel_and_offers_only_rejoin_or_forget() {
		let output = run(serde_json::json!({
			"action": "show",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_with_channels(None, "123", "General", 2, false)
		}));
		let shown = panel_text(&output);
		assert!(shown.contains("Saved channel: General [123]"));
		assert!(shown.contains("reports kind 2"));
		assert!(shown.contains("Propose rejoining saved channel"));
		assert!(shown.contains("Forget saved channel"));
		assert!(!shown.contains("Save current channel"));
		assert!(output.effects.is_empty());
	}

	#[test]
	fn rejoin_proposes_exactly_one_muted_no_ring_action() {
		let output = run(serde_json::json!({
			"action": "rejoin",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_json(None)
		}));
		assert_eq!(
			output.effects,
			vec![HostEffect::AppAction {
				action: AppAction::JoinVoice {
					channel_id: "123".into(),
					ring: false,
					muted: true,
					deafened: false
				}
			}]
		);
	}

	#[test]
	fn rejoin_refuses_missing_saved_active_and_unavailable_voice_state() {
		let missing = run(serde_json::json!({
			"action": "rejoin",
			"app": app_json(None)
		}));
		assert!(missing.effects.is_empty());
		assert!(panel_text(&missing).contains("No saved channel can be rejoined"));

		let same = run(serde_json::json!({
			"action": "rejoin",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_json(Some("123"))
		}));
		assert!(same.effects.is_empty());
		assert!(panel_text(&same).contains("already in the saved channel"));

		let other = run(serde_json::json!({
			"action": "rejoin",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_json(Some("999"))
		}));
		assert!(other.effects.is_empty());
		assert!(panel_text(&other).contains("will not switch an active call"));

		let no_snapshot = run(serde_json::json!({
			"action": "rejoin",
			"storage": "{\"channel_id\":\"123\"}",
			"app": {}
		}));
		assert!(no_snapshot.effects.is_empty());
		assert!(panel_text(&no_snapshot).contains("Voice state is unavailable"));
	}

	#[test]
	fn rejoin_checks_a_complete_channel_directory_but_not_a_partial_one() {
		let missing = run(serde_json::json!({
			"action": "rejoin",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_with_channels(None, "999", "Other", 2, false)
		}));
		assert!(missing.effects.is_empty());
		assert!(panel_text(&missing).contains("not in the complete loaded channel list"));

		let text_channel = run(serde_json::json!({
			"action": "rejoin",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_with_channels(None, "123", "Text", 0, false)
		}));
		assert!(text_channel.effects.is_empty());
		assert!(panel_text(&text_channel).contains("non-voice channel"));

		let partial = run(serde_json::json!({
			"action": "rejoin",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_with_channels(None, "999", "Other", 2, true)
		}));
		assert_eq!(partial.effects.len(), 1);
	}

	#[test]
	fn save_current_requires_a_call_and_forget_clears_both_valid_and_corrupt_state() {
		let no_call = run(serde_json::json!({
			"action": "save-current",
			"app": app_json(None)
		}));
		assert!(no_call.output.storage.is_none());
		assert!(panel_text(&no_call).contains("Join a voice channel before saving"));

		let saved = run(serde_json::json!({
			"action": "save-current",
			"app": app_json(Some("456"))
		}));
		assert_eq!(
			saved.output.storage.as_deref(),
			Some("{\"channel_id\":\"456\"}")
		);

		let repaired = run(serde_json::json!({
			"action": "save-current",
			"storage": "not-json",
			"app": app_json(Some("456"))
		}));
		assert_eq!(
			repaired.output.storage.as_deref(),
			Some("{\"channel_id\":\"456\"}")
		);

		for storage in ["{\"channel_id\":\"456\"}", "not-json"] {
			let forgotten = run(serde_json::json!({
				"action": "forget",
				"storage": storage,
				"app": app_json(None)
			}));
			assert_eq!(forgotten.output.storage.as_deref(), Some("{}"));
			assert!(panel_text(&forgotten).contains("Forgot the saved channel"));
		}

		let empty = run(serde_json::json!({
			"action": "forget",
			"storage": "{}",
			"app": app_json(None)
		}));
		assert!(empty.output.storage.is_none());
	}

	#[test]
	fn long_names_are_sanitized_and_the_response_stays_bounded() {
		let name = format!("{}\u{0007}", "🦀".repeat(100));
		let output = run(serde_json::json!({
			"action": "show",
			"storage": "{\"channel_id\":\"123\"}",
			"app": app_with_channels(None, "123", &name, 2, false)
		}));
		let bytes = serde_json::to_vec(&output).unwrap();
		assert!(bytes.len() <= MAX_RESPONSE_BYTES);
		for element in &output.output.panel {
			if let Element::Text { text } = element {
				assert!(text.len() <= 4096);
				assert!(!text.chars().any(char::is_control));
			}
		}
	}
}
