/*
 * Account & navigation tools
 * Copyright (c) 2026 Serein port contributors
 *
 * Bounded Serein adaptations of Equicord's `profileSets`, `recentDMSwitcher`
 * and `randomVoice` plugins (Copyright (c) Vendicated and contributors),
 * licensed GPL-3.0-or-later, at upstream commit
 * 200dba124066146a8f043de7fc8b05e3dc5e68e.
 *
 * Changed November 8, 2026: patched Discord menus, Flux stores, webpack,
 * global hotkeys, avatar/banner uploads, camera/stream activation, automatic
 * joining and background work are removed. This port uses only bounded SDK
 * snapshots, plugin storage and one Apply-reviewed host proposal per action.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	AppAction, AppInvocation, AppOutput, ChannelSnapshot, Element, HostEffect, Output,
	OwnProfilePatch, serde_json,
};

/// Upstream keeps up to 50 recent DMs. The local history stays smaller so the
/// snapshot and panel remain cheap.
const MAX_RECENT_DMS: usize = 20;
/// At most 16 profile presets fit the select and storage budgets.
const MAX_PROFILES: usize = 16;
/// Encoded plugin storage stays below this many UTF-8 bytes.
const MAX_STORAGE_BYTES: usize = 48 * 1024;
/// Panel and storage share these profile field limits (the host enforces the
/// same character caps; byte caps here keep encoded storage predictable).
const MAX_PROFILE_NAME_CHARS: usize = 32;
const MAX_PROFILE_NAME_BYTES: usize = 128;
const MAX_DISPLAY_NAME_CHARS: usize = 32;
const MAX_DISPLAY_NAME_BYTES: usize = 128;
const MAX_BIO_CHARS: usize = 190;
const MAX_BIO_BYTES: usize = 760;
const MAX_PRONOUNS_CHARS: usize = 40;
const MAX_PRONOUNS_BYTES: usize = 160;
/// Host channel kinds used by the bounded snapshots.
const KIND_DM: u8 = 1;
const KIND_GUILD_VOICE: u8 = 2;
const KIND_GROUP_DM: u8 = 3;
const KIND_STAGE: u8 = 13;
const STORAGE_VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfilePreset {
	name: String,
	/// Empty means "clear the global display name when applied".
	display_name: String,
	bio: String,
	pronouns: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
	version: u8,
	profiles: Vec<ProfilePreset>,
	recent_dms: Vec<String>,
	voice_cursor: u64,
}

impl Stored {
	fn new() -> Self {
		Self {
			version: STORAGE_VERSION,
			profiles: Vec::new(),
			recent_dms: Vec::new(),
			voice_cursor: 0,
		}
	}

	fn valid(&self) -> bool {
		self.version == STORAGE_VERSION
			&& self.profiles.len() <= MAX_PROFILES
			&& self.recent_dms.len() <= MAX_RECENT_DMS
			&& self.profiles.iter().all(ProfilePreset::valid)
			&& self.unique_profile_names()
			&& self.unique_recent_dms()
			&& self.recent_dms.iter().all(|id| valid_entity_id(id))
	}

	fn unique_recent_dms(&self) -> bool {
		let mut ids = std::collections::BTreeSet::new();
		self.recent_dms.iter().all(|id| ids.insert(id.as_str()))
	}

	fn unique_profile_names(&self) -> bool {
		let mut names = std::collections::BTreeSet::new();
		self.profiles
			.iter()
			.all(|profile| names.insert(profile.name.as_str()))
	}
}

impl ProfilePreset {
	fn valid(&self) -> bool {
		valid_profile_name(&self.name)
			&& valid_text(
				&self.display_name,
				MAX_DISPLAY_NAME_CHARS,
				MAX_DISPLAY_NAME_BYTES,
				false,
			) && valid_text(&self.bio, MAX_BIO_CHARS, MAX_BIO_BYTES, true)
			&& valid_text(
				&self.pronouns,
				MAX_PRONOUNS_CHARS,
				MAX_PRONOUNS_BYTES,
				false,
			)
	}
}

#[derive(Clone, Debug)]
enum StorageState {
	Empty,
	Valid(Stored),
	/// Damaged or future storage is shown as-is and never rewritten implicitly.
	Corrupt,
}

impl StorageState {
	fn stored(&self) -> Option<&Stored> {
		match self {
			Self::Valid(stored) => Some(stored),
			Self::Empty | Self::Corrupt => None,
		}
	}

	fn is_corrupt(&self) -> bool {
		matches!(self, Self::Corrupt)
	}
}

fn load(input: &AppInvocation) -> StorageState {
	let Some(raw) = input.invocation.storage.as_deref() else {
		return StorageState::Empty;
	};
	match serde_json::from_str::<Stored>(raw) {
		Ok(stored) if stored.valid() => StorageState::Valid(stored),
		_ => StorageState::Corrupt,
	}
}

fn store(output: &mut AppOutput, stored: &Stored) -> Result<(), &'static str> {
	if !stored.valid() {
		return Err("the preset data is outside its bounds");
	}
	let bytes = serde_json::to_vec(stored).map_err(|_| "the preset data could not be encoded")?;
	if bytes.len() > MAX_STORAGE_BYTES {
		return Err("the saved data exceeds its 48 KiB budget");
	}
	output.output.storage =
		Some(String::from_utf8(bytes).map_err(|_| "the preset data was not UTF-8")?);
	Ok(())
}

fn valid_entity_id(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 20
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn valid_profile_name(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= MAX_PROFILE_NAME_BYTES
		&& value.chars().count() <= MAX_PROFILE_NAME_CHARS
		&& !value.chars().any(char::is_control)
		&& value.trim() == value
}

fn valid_text(value: &str, max_chars: usize, max_bytes: usize, multiline: bool) -> bool {
	value.len() <= max_bytes
		&& value.chars().count() <= max_chars
		&& !value
			.chars()
			.any(|ch| ch.is_control() && !(multiline && matches!(ch, '\n' | '\r' | '\t')))
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

fn button(id: &str, label: &str) -> Element {
	Element::Button {
		id: id.into(),
		label: label.into(),
	}
}

fn text_input(id: &str, label: &str, value: &str) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: value.into(),
	}
}

fn is_direct_message(channel: &ChannelSnapshot) -> bool {
	matches!(channel.kind, KIND_DM | KIND_GROUP_DM)
}

fn loaded_dm_ids(input: &AppInvocation) -> std::collections::BTreeSet<&str> {
	input
		.app
		.as_ref()
		.and_then(|app| app.channels.as_ref())
		.map(|channels| {
			channels
				.items
				.iter()
				.filter(|channel| is_direct_message(channel))
				.map(|channel| channel.id.as_str())
				.collect()
		})
		.unwrap_or_default()
}

fn current_channel(input: &AppInvocation) -> Option<&ChannelSnapshot> {
	input
		.app
		.as_ref()
		.and_then(|app| app.context.as_ref())
		.and_then(|context| context.channel.as_ref())
}

fn current_profile(input: &AppInvocation) -> (String, String, String) {
	let Some(profile) = input
		.app
		.as_ref()
		.and_then(|app| app.account_profile.as_ref())
		.and_then(|account| account.profile.as_ref())
	else {
		return (String::new(), String::new(), String::new());
	};
	(
		profile.display_name.clone().unwrap_or_default(),
		profile.bio.clone(),
		profile.pronouns.clone(),
	)
}

fn recent_dm_lines(input: &AppInvocation, stored: &Stored) -> Vec<String> {
	let loaded = loaded_dm_ids(input);
	let channels = input
		.app
		.as_ref()
		.and_then(|app| app.channels.as_ref())
		.map(|channels| channels.items.as_slice())
		.unwrap_or_default();
	let current = current_channel(input).map(|channel| channel.id.as_str());
	let mut lines = Vec::new();
	for id in stored
		.recent_dms
		.iter()
		.filter(|id| loaded.contains(id.as_str()))
		.take(8)
	{
		let name = channels
			.iter()
			.find(|channel| &channel.id == id)
			.map(|channel| channel.name.as_str())
			.unwrap_or("Direct message");
		let suffix = if current == Some(id.as_str()) {
			" (current)"
		} else {
			""
		};
		lines.push(format!("{name} [{id}]{suffix}"));
	}
	lines
}

fn panel(
	input: &AppInvocation,
	state: &StorageState,
	notice: Option<&str>,
	error: Option<&str>,
) -> AppOutput {
	let profiles = state
		.stored()
		.map(|stored| &stored.profiles[..])
		.unwrap_or_default();
	let (display_name, bio, pronouns) = current_profile(input);
	let selected = input.invocation.value("profile").unwrap_or_default();
	let selected = profiles
		.iter()
		.find(|profile| profile.name == selected)
		.or_else(|| profiles.first())
		.map(|profile| profile.name.clone())
		.unwrap_or_default();

	let mut elements = vec![
		heading("Account & navigation tools"),
		text(
			"Profile presets, a recent-DM switcher and a rotating loaded voice-channel picker. Every change is one host-reviewed Apply proposal; nothing is automatic.",
		),
	];
	if state.is_corrupt() {
		elements.push(text(
			"Saved plugin data could not be read and has been left unchanged. Saving a new profile preset explicitly replaces the damaged value; apply, delete, history and rotation never overwrite it.",
		));
	}
	if let Some(error) = error {
		elements.push(text(format!("Nothing changed: {error}")));
	}
	if let Some(notice) = notice {
		elements.push(text(notice));
	}

	elements.push(Element::Separator);
	elements.push(heading("Profile presets"));
	elements.push(text(
		"Saved fields are display name, bio and pronouns. Avatars, banners, decorations and theme colors are not exposed by the SDK and are not stored.",
	));
	elements.push(text(format!(
		"Current loaded profile: display name ({} characters), bio ({} characters), pronouns ({} characters).",
		display_name.chars().count(),
		bio.chars().count(),
		pronouns.chars().count()
	)));
	if profiles.is_empty() {
		elements.push(text("No profile presets are saved."));
	} else {
		elements.push(Element::Select {
			id: "profile".into(),
			label: "Selected profile preset".into(),
			options: profiles
				.iter()
				.map(|profile| profile.name.clone())
				.collect(),
			value: selected,
		});
		elements.push(Element::Row {
			children: vec![
				button("apply-profile", "Apply selected profile preset"),
				button("delete-profile", "Delete selected profile preset"),
			],
		});
	}
	elements.push(text_input("name", "Preset name (max 32 characters)", ""));
	elements.push(text_input(
		"display-name",
		"Display name (blank clears)",
		&display_name,
	));
	elements.push(text_input(
		"bio",
		"Bio (max 190 characters, blank clears)",
		&bio,
	));
	elements.push(text_input(
		"pronouns",
		"Pronouns (max 40 characters, blank clears)",
		&pronouns,
	));
	elements.push(button("save-profile", "Save profile preset"));

	elements.push(Element::Separator);
	elements.push(heading("Recent direct messages"));
	elements.push(text(
		"Only direct and group-DM IDs are kept, at most 20, in plugin storage. Message content, names and avatars are never stored.",
	));
	let lines = state
		.stored()
		.map(|stored| recent_dm_lines(input, stored))
		.unwrap_or_default();
	if lines.is_empty() {
		elements.push(text("No loaded direct-message history is available."));
	} else {
		for line in lines {
			elements.push(text(line));
		}
	}
	elements.push(Element::Row {
		children: vec![
			button("switch-dm", "Switch to previous loaded DM"),
			button("clear-dm-history", "Clear DM history"),
		],
	});

	elements.push(Element::Separator);
	elements.push(heading("Rotating voice channel"));
	elements.push(text(
		"Picks the next loaded guild voice or stage channel in a stable rotation. The host still enforces CONNECT/STREAM permissions and shows its Apply review. Stages, AFK channels and empty channels are not filtered; camera and stream are never started.",
	));
	elements.push(Element::Checkbox {
		id: "muted".into(),
		label: "Join muted (recommended)".into(),
		checked: input
			.invocation
			.parse_value::<bool>("muted")
			.ok()
			.flatten()
			.unwrap_or(true),
	});
	elements.push(Element::Checkbox {
		id: "deafened".into(),
		label: "Join deafened".into(),
		checked: input
			.invocation
			.parse_value::<bool>("deafened")
			.ok()
			.flatten()
			.unwrap_or(false),
	});
	elements.push(button("join-voice", "Join next loaded voice channel"));

	AppOutput {
		output: Output {
			panel: elements,
			..Default::default()
		},
		..Default::default()
	}
}

fn reply(input: &AppInvocation, error: &str) -> AppOutput {
	panel(input, &load(input), None, Some(error))
}

fn save_profile(input: &AppInvocation) -> AppOutput {
	let name = input.invocation.value("name").unwrap_or_default().trim();
	if !valid_profile_name(name) {
		return reply(
			input,
			"Use a unique preset name of 1 to 32 characters without surrounding whitespace or controls.",
		);
	}
	let display_name = input
		.invocation
		.value("display-name")
		.unwrap_or_default()
		.trim()
		.to_owned();
	let bio = input.invocation.value("bio").unwrap_or_default().to_owned();
	let pronouns = input
		.invocation
		.value("pronouns")
		.unwrap_or_default()
		.trim()
		.to_owned();
	if !valid_text(
		&display_name,
		MAX_DISPLAY_NAME_CHARS,
		MAX_DISPLAY_NAME_BYTES,
		false,
	) {
		return reply(
			input,
			"Display names must be at most 32 characters and contain no control characters.",
		);
	}
	if !valid_text(&bio, MAX_BIO_CHARS, MAX_BIO_BYTES, true) {
		return reply(
			input,
			"Bios must be at most 190 characters and contain only newline, tab or printable characters.",
		);
	}
	if !valid_text(&pronouns, MAX_PRONOUNS_CHARS, MAX_PRONOUNS_BYTES, false) {
		return reply(
			input,
			"Pronouns must be at most 40 characters and contain no control characters.",
		);
	}

	let state = load(input);
	let mut stored = match state {
		StorageState::Valid(stored) => stored,
		StorageState::Empty | StorageState::Corrupt => Stored::new(),
	};
	let preset = ProfilePreset {
		name: name.to_owned(),
		display_name,
		bio,
		pronouns,
	};
	match stored.profiles.iter().position(|item| item.name == name) {
		Some(index) => stored.profiles[index] = preset,
		None if stored.profiles.len() < MAX_PROFILES => stored.profiles.push(preset),
		None => {
			return reply(
				input,
				"Saving another preset would exceed the 16-preset limit; delete one first.",
			);
		}
	}
	let mut output = panel(
		input,
		&StorageState::Valid(stored.clone()),
		Some("Saved. Applying the preset still requires the host's Apply review."),
		None,
	);
	if let Err(error) = store(&mut output, &stored) {
		return reply(input, error);
	}
	output
}

fn apply_profile(input: &AppInvocation) -> AppOutput {
	let Some(stored) = load(input).stored().cloned() else {
		return reply(
			input,
			"No valid profile preset data is available. Save a preset first.",
		);
	};
	let selected = input.invocation.value("profile").unwrap_or_default();
	let Some(preset) = stored.profiles.iter().find(|item| item.name == selected) else {
		return reply(input, "Select a saved profile preset first.");
	};
	let mut patch = OwnProfilePatch::default();
	if preset.display_name.is_empty() {
		patch.clear_global_name = true;
	} else {
		patch.global_name = Some(preset.display_name.clone());
	}
	patch.bio = Some(preset.bio.clone());
	patch.pronouns = Some(preset.pronouns.clone());
	AppOutput {
		output: Output::default(),
		effects: vec![HostEffect::AppAction {
			action: AppAction::SetOwnProfile { profile: patch },
		}],
	}
}

fn delete_profile(input: &AppInvocation) -> AppOutput {
	let Some(mut stored) = load(input).stored().cloned() else {
		return reply(
			input,
			"No valid profile preset data is available. Save a preset first.",
		);
	};
	let selected = input.invocation.value("profile").unwrap_or_default();
	let Some(index) = stored
		.profiles
		.iter()
		.position(|item| item.name == selected)
	else {
		return reply(input, "Select a saved profile preset first.");
	};
	stored.profiles.remove(index);
	let mut output = panel(
		input,
		&StorageState::Valid(stored.clone()),
		Some("Deleted the selected profile preset."),
		None,
	);
	if let Err(error) = store(&mut output, &stored) {
		return reply(input, error);
	}
	output
}

fn track(input: &AppInvocation) -> AppOutput {
	let mut stored = match load(input) {
		StorageState::Valid(stored) => stored,
		StorageState::Empty => Stored::new(),
		StorageState::Corrupt => return AppOutput::default(),
	};
	let Some(channel) = current_channel(input) else {
		return AppOutput::default();
	};
	if !is_direct_message(channel) || !valid_entity_id(&channel.id) {
		return AppOutput::default();
	}
	if stored
		.recent_dms
		.first()
		.is_some_and(|id| id == &channel.id)
	{
		return AppOutput::default();
	}
	stored.recent_dms.retain(|id| id != &channel.id);
	stored.recent_dms.insert(0, channel.id.clone());
	stored.recent_dms.truncate(MAX_RECENT_DMS);
	let mut output = AppOutput::default();
	if store(&mut output, &stored).is_err() {
		return AppOutput::default();
	}
	output
}

fn switch_dm(input: &AppInvocation) -> AppOutput {
	let Some(stored) = load(input).stored().cloned() else {
		return reply(
			input,
			"No valid recent-DM history is available. Open a direct message first.",
		);
	};
	let loaded = loaded_dm_ids(input);
	let current = current_channel(input).map(|channel| channel.id.as_str());
	let target = stored
		.recent_dms
		.iter()
		.find(|id| loaded.contains(id.as_str()) && Some(id.as_str()) != current)
		.cloned();
	let Some(target) = target else {
		return reply(
			input,
			"No other loaded direct message is in the recent history.",
		);
	};
	let mut output = panel(
		input,
		&StorageState::Valid(stored),
		Some("Prepared a navigation proposal for the previous loaded DM."),
		None,
	);
	output.effects = vec![HostEffect::Navigate { channel_id: target }];
	output
}

fn clear_dm_history(input: &AppInvocation) -> AppOutput {
	let Some(mut stored) = load(input).stored().cloned() else {
		return reply(input, "No valid recent-DM history is available to clear.");
	};
	stored.recent_dms.clear();
	let mut output = panel(
		input,
		&StorageState::Valid(stored.clone()),
		Some("Cleared the local direct-message ID history."),
		None,
	);
	if let Err(error) = store(&mut output, &stored) {
		return reply(input, error);
	}
	output
}

fn voice_channels(input: &AppInvocation) -> Vec<&ChannelSnapshot> {
	let mut channels: Vec<&ChannelSnapshot> = input
		.app
		.as_ref()
		.and_then(|app| app.channels.as_ref())
		.map(|channels| {
			channels
				.items
				.iter()
				.filter(|channel| {
					channel.guild_id.is_some()
						&& matches!(channel.kind, KIND_GUILD_VOICE | KIND_STAGE)
				})
				.collect()
		})
		.unwrap_or_default();
	channels.sort_by_key(|channel| channel.id.parse::<u64>().unwrap_or(u64::MAX));
	channels.dedup_by(|left, right| left.id == right.id);
	channels
}

fn join_voice(input: &AppInvocation) -> AppOutput {
	let channels = voice_channels(input);
	if channels.is_empty() {
		return reply(
			input,
			"No loaded guild voice or stage channel is available to join.",
		);
	}
	let state = load(input);
	let (mut stored, can_persist) = match &state {
		StorageState::Valid(stored) => (stored.clone(), true),
		StorageState::Empty => (Stored::new(), true),
		StorageState::Corrupt => (Stored::new(), false),
	};
	let index = (stored.voice_cursor % channels.len() as u64) as usize;
	let channel = channels[index];
	stored.voice_cursor = stored.voice_cursor.wrapping_add(1);
	let muted = input
		.invocation
		.parse_value::<bool>("muted")
		.ok()
		.flatten()
		.unwrap_or(true);
	let deafened = input
		.invocation
		.parse_value::<bool>("deafened")
		.ok()
		.flatten()
		.unwrap_or(false);
	let notice = if can_persist {
		format!(
			"Prepared a join proposal for {} [{}]. The host reviews it and enforces channel permissions.",
			channel.name, channel.id
		)
	} else {
		"Prepared a join proposal, but the damaged saved data was left unchanged, so the rotation cursor was not stored.".to_owned()
	};
	let mut output = panel(input, &state, Some(&notice), None);
	if can_persist && let Err(error) = store(&mut output, &stored) {
		return reply(input, error);
	}
	output.effects = vec![HostEffect::AppAction {
		action: AppAction::JoinVoice {
			channel_id: channel.id.clone(),
			ring: false,
			muted,
			deafened,
		},
	}];
	output
}

fn handle(input: AppInvocation) -> AppOutput {
	match input.invocation.action.as_str() {
		"track" => track(&input),
		"save-profile" => save_profile(&input),
		"apply-profile" => apply_profile(&input),
		"delete-profile" => delete_profile(&input),
		"switch-dm" => switch_dm(&input),
		"clear-dm-history" => clear_dm_history(&input),
		"join-voice" => join_voice(&input),
		"open" => panel(&input, &load(&input), None, None),
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::dispatch_typed;

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
				_ => None,
			})
			.collect::<Vec<_>>()
			.join("\n")
	}

	fn base_app() -> serde_json::Value {
		serde_json::json!({
			"context": {
				"connected": true,
				"user": {"id": "1", "name": "Owner"},
				"channel": {"id": "20", "name": "Alice", "kind": 1}
			},
			"channels": {
				"items": [
					{"id": "20", "name": "Alice", "kind": 1},
					{"id": "21", "name": "Bob", "kind": 1},
					{"id": "30", "guild_id": "100", "name": "Voice", "kind": 2},
					{"id": "31", "guild_id": "100", "name": "Stage", "kind": 13},
					{"id": "10", "guild_id": "100", "name": "General", "kind": 0}
				],
				"truncated": false
			},
			"account_profile": {
				"user": {"id": "1", "name": "Owner"},
				"profile": {
					"display_name": "Owner",
					"bio": "Hello",
					"pronouns": "they/them"
				}
			}
		})
	}

	fn stored(output: &AppOutput) -> Stored {
		serde_json::from_str(
			output
				.output
				.storage
				.as_deref()
				.expect("storage was written"),
		)
		.unwrap()
	}

	#[test]
	fn profile_presets_round_trip_and_blank_display_name_clears() {
		let saved = run(serde_json::json!({
			"action": "save-profile",
			"values": {
				"name": "Main",
				"display-name": "Ren",
				"bio": "Line one\nLine two",
				"pronouns": "they/them"
			},
			"app": base_app()
		}));
		assert!(saved.effects.is_empty());
		let saved_storage = saved.output.storage.clone().expect("preset saved");
		assert_eq!(stored(&saved).profiles[0].display_name, "Ren");

		let applied = run(serde_json::json!({
			"action": "apply-profile",
			"values": {"profile": "Main"},
			"storage": saved_storage,
			"app": base_app()
		}));
		assert!(applied.output.storage.is_none());
		let HostEffect::AppAction {
			action: AppAction::SetOwnProfile { profile },
		} = &applied.effects[0]
		else {
			panic!("expected a set_own_profile proposal");
		};
		assert_eq!(profile.global_name.as_deref(), Some("Ren"));
		assert_eq!(profile.bio.as_deref(), Some("Line one\nLine two"));
		assert_eq!(profile.pronouns.as_deref(), Some("they/them"));
		assert!(!profile.clear_global_name);

		let cleared = run(serde_json::json!({
			"action": "save-profile",
			"values": {"name": "Blank", "display-name": "", "bio": "", "pronouns": ""},
			"app": base_app()
		}));
		let applied = run(serde_json::json!({
			"action": "apply-profile",
			"values": {"profile": "Blank"},
			"storage": cleared.output.storage,
			"app": base_app()
		}));
		let HostEffect::AppAction {
			action: AppAction::SetOwnProfile { profile },
		} = &applied.effects[0]
		else {
			panic!("expected a set_own_profile proposal");
		};
		assert!(profile.clear_global_name);
		assert!(profile.global_name.is_none());
		assert_eq!(profile.bio.as_deref(), Some(""));
		assert_eq!(profile.pronouns.as_deref(), Some(""));
	}

	#[test]
	fn profile_fields_are_bounded_and_corrupt_storage_is_preserved() {
		for (field, value) in [
			("display-name", "x".repeat(MAX_DISPLAY_NAME_CHARS + 1)),
			("bio", "x".repeat(MAX_BIO_CHARS + 1)),
			("pronouns", "x".repeat(MAX_PRONOUNS_CHARS + 1)),
		] {
			let mut values = serde_json::json!({"name": "Too long"});
			values
				.as_object_mut()
				.unwrap()
				.insert(field.to_owned(), serde_json::json!(value));
			let output = run(serde_json::json!({
				"action": "save-profile",
				"values": values,
				"app": base_app()
			}));
			assert!(output.output.storage.is_none(), "{field}");
			assert!(output.effects.is_empty(), "{field}");
			assert!(panel_text(&output).contains("Nothing changed"), "{field}");
		}

		let corrupt = run(serde_json::json!({
			"action": "open",
			"storage": "{not json",
			"app": base_app()
		}));
		assert!(corrupt.output.storage.is_none());
		assert!(panel_text(&corrupt).contains("left unchanged"));
	}

	#[test]
	fn navigation_tracking_keeps_only_bounded_dm_ids_and_preserves_profiles() {
		let mut history = Stored::new();
		history.profiles.push(ProfilePreset {
			name: "Keep".into(),
			display_name: "Owner".into(),
			bio: "Hello".into(),
			pronouns: "they/them".into(),
		});
		history.recent_dms = (100..120).map(|id| id.to_string()).collect();
		let storage = serde_json::to_string(&history).unwrap();
		let mut app = base_app();
		app["context"]["channel"] = serde_json::json!({"id": "500", "name": "New DM", "kind": 1});
		app["channels"]["items"]
			.as_array_mut()
			.unwrap()
			.push(serde_json::json!({"id": "500", "name": "New DM", "kind": 1}));

		let tracked = run(serde_json::json!({
			"action": "track",
			"app_event": "navigation",
			"storage": storage,
			"app": app.clone()
		}));
		let tracked = stored(&tracked);
		assert_eq!(tracked.recent_dms.len(), MAX_RECENT_DMS);
		assert_eq!(tracked.recent_dms[0], "500");
		assert_eq!(tracked.profiles[0].name, "Keep");

		let mut guild = base_app();
		guild["context"]["channel"] =
			serde_json::json!({"id": "10", "guild_id": "100", "name": "General", "kind": 0});
		let ignored = run(serde_json::json!({
			"action": "track",
			"app_event": "navigation",
			"storage": serde_json::to_string(&history).unwrap(),
			"app": guild
		}));
		assert!(ignored.output.storage.is_none());

		let corrupt = run(serde_json::json!({
			"action": "track",
			"app_event": "navigation",
			"storage": "{not json",
			"app": app
		}));
		assert!(corrupt.output.storage.is_none());
	}

	#[test]
	fn dm_switch_skips_stale_and_current_ids() {
		let mut history = Stored::new();
		history.recent_dms = vec!["22".into(), "21".into(), "20".into()];
		let output = run(serde_json::json!({
			"action": "switch-dm",
			"storage": serde_json::to_string(&history).unwrap(),
			"app": base_app()
		}));
		assert_eq!(output.effects.len(), 1);
		let HostEffect::Navigate { channel_id } = &output.effects[0] else {
			panic!("expected a navigation proposal");
		};
		assert_eq!(channel_id.as_str(), "21");
		assert!(output.output.storage.is_none());

		let mut only_current = Stored::new();
		only_current.recent_dms = vec!["20".into()];
		let none = run(serde_json::json!({
			"action": "switch-dm",
			"storage": serde_json::to_string(&only_current).unwrap(),
			"app": base_app()
		}));
		assert!(none.effects.is_empty());
		assert!(panel_text(&none).contains("No other loaded direct message"));
	}

	#[test]
	fn voice_rotation_uses_loaded_guild_voice_and_stage_channels_only() {
		let first = run(serde_json::json!({
			"action": "join-voice",
			"app": base_app()
		}));
		let HostEffect::AppAction {
			action: AppAction::JoinVoice {
				channel_id,
				ring,
				muted,
				deafened,
			},
		} = &first.effects[0]
		else {
			panic!("expected a join_voice proposal");
		};
		assert_eq!(channel_id.as_str(), "30");
		assert!(!*ring);
		assert!(*muted);
		assert!(!*deafened);
		assert_eq!(stored(&first).voice_cursor, 1);

		let second = run(serde_json::json!({
			"action": "join-voice",
			"storage": first.output.storage,
			"app": base_app()
		}));
		let HostEffect::AppAction {
			action: AppAction::JoinVoice { channel_id, .. },
		} = &second.effects[0]
		else {
			panic!("expected a join_voice proposal");
		};
		assert_eq!(channel_id.as_str(), "31");

		let mut no_voice = base_app();
		no_voice["channels"]["items"] = serde_json::json!([
			{"id": "20", "name": "Alice", "kind": 1},
			{"id": "10", "guild_id": "100", "name": "General", "kind": 0}
		]);
		let none = run(serde_json::json!({
			"action": "join-voice",
			"app": no_voice
		}));
		assert!(none.effects.is_empty());
		assert!(panel_text(&none).contains("No loaded guild voice"));
	}
}
