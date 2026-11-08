/*
 * Equicord Community Tools — bounded Serein adaptations of several
 * Equicord/Vencord workflows.
 *
 * Upstream: Equicord/Equicord at commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e.
 * Upstream files inspected:
 *   src/equicordplugins/exportMessages/index.tsx
 *   src/equicordplugins/hopOn/index.tsx
 *   src/equicordplugins/fastDeleteChannels/index.tsx
 *   src/equicordplugins/ignoreCalls/index.tsx
 *
 * This adaptation is intentionally not JavaScript parity. It uses only the
 * bounded Serein extension SDK: one selected-message clipboard copy, one
 * literal trigger-to-HTTPS proposal, one Apply-reviewed channel deletion and
 * an Apply-reviewed decline for conversations the user has explicitly marked
 * in a local ignore list. No Webpack patch, raw Gateway/store access, network
 * request, file save, automatic URL launch, camera change or background call
 * handling is implemented.
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License version 3 or (at your
 * option) any later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	AppAction, AppOutput, ChannelSnapshot, Element, HostEffect, MessageActionInvocation,
	SelectedMessageRef, serde_json,
};

/// One clipboard export stays well below the host's 256 KiB I/O budget.
const MAX_EXPORT_BYTES: usize = 4096;
/// A HopOn configuration is a short list of literal phrases, not a regex engine.
const MAX_TRIGGERS: usize = 8;
const MAX_TRIGGER_CHARS: usize = 64;
const MAX_TRIGGER_BYTES: usize = 256;
/// The host's external-link confirmation accepts a complete public HTTPS URL.
const MAX_URL_BYTES: usize = 2048;
/// At most this many conversations can be kept in the local call ignore list.
const MAX_IGNORED_CALL_CHANNELS: usize = 16;
/// Keep the complete encoded storage value comfortably bounded.
const MAX_STORAGE_BYTES: usize = 48 * 1024;
const MAX_ID_BYTES: usize = 20;
const STORAGE_VERSION: u8 = 1;

const MISSING_SELECTION: &str =
	"The host did not provide the selected message IDs, so nothing was proposed.";
const STALE_SELECTION: &str =
	"The selected message no longer matches its loaded context. Reopen the message and try again.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HopOnConfig {
	triggers: Vec<String>,
	url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
	version: u8,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	hopon: Option<HopOnConfig>,
	#[serde(default)]
	ignored_call_channels: Vec<String>,
}

impl Stored {
	fn empty() -> Self {
		Self {
			version: STORAGE_VERSION,
			hopon: None,
			ignored_call_channels: Vec::new(),
		}
	}

	fn valid(&self) -> bool {
		self.version == STORAGE_VERSION
			&& self.hopon.as_ref().is_none_or(|config| {
				!config.triggers.is_empty()
					&& config.triggers.len() <= MAX_TRIGGERS
					&& config.triggers.iter().all(|trigger| valid_trigger(trigger))
					&& valid_https_url(&config.url)
			}) && self.ignored_call_channels.len() <= MAX_IGNORED_CALL_CHANNELS
			&& unique_valid_ids(&self.ignored_call_channels)
	}
}

#[derive(Clone, Debug)]
enum StorageState {
	Empty,
	Valid(Stored),
	/// The exact saved value is preserved and never rewritten implicitly.
	Corrupt,
}

impl StorageState {
	fn stored(&self) -> Option<&Stored> {
		match self {
			Self::Valid(stored) => Some(stored),
			Self::Empty | Self::Corrupt => None,
		}
	}

	fn hopon(&self) -> Option<&HopOnConfig> {
		self.stored().and_then(|stored| stored.hopon.as_ref())
	}

	fn ignored(&self) -> &[String] {
		self.stored()
			.map_or(&[], |stored| stored.ignored_call_channels.as_slice())
	}

	fn is_corrupt(&self) -> bool {
		matches!(self, Self::Corrupt)
	}
}

fn load_storage(input: &MessageActionInvocation) -> StorageState {
	let Some(raw) = input.invocation.invocation.storage.as_deref() else {
		return StorageState::Empty;
	};
	if raw.len() > MAX_STORAGE_BYTES {
		return StorageState::Corrupt;
	}
	match input.invocation.invocation.storage_json::<Stored>() {
		Ok(Some(stored)) if stored.valid() => StorageState::Valid(stored),
		_ => StorageState::Corrupt,
	}
}

fn save_storage(output: &mut AppOutput, stored: &Stored) -> Result<(), ()> {
	if !stored.valid() {
		return Err(());
	}
	let encoded = serde_json::to_vec(stored).map_err(|_| ())?;
	if encoded.len() > MAX_STORAGE_BYTES {
		return Err(());
	}
	output.output.set_storage_json(stored).map_err(|_| ())
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

fn input(id: &str, label: &str, value: &str) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: value.into(),
	}
}

fn button(id: &str, label: &str) -> Element {
	Element::Button {
		id: id.into(),
		label: label.into(),
	}
}

fn panel_output(panel: Vec<Element>, effect: Option<HostEffect>) -> AppOutput {
	AppOutput {
		output: serein_extension_sdk::Output {
			panel,
			..Default::default()
		},
		effects: effect.into_iter().collect(),
	}
}

fn error_output(title: &str, message: &str) -> AppOutput {
	panel_output(vec![heading(title), text(message)], None)
}

fn valid_snowflake(value: &str) -> bool {
	let bytes = value.as_bytes();
	!bytes.is_empty()
		&& bytes.len() <= MAX_ID_BYTES
		&& bytes[0] != b'0'
		&& bytes.iter().all(u8::is_ascii_digit)
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn unique_valid_ids(values: &[String]) -> bool {
	let mut seen = std::collections::BTreeSet::new();
	values
		.iter()
		.all(|value| valid_snowflake(value) && seen.insert(value.as_str()))
}

fn sanitize_name(value: &str) -> String {
	let cleaned = value
		.chars()
		.map(|character| {
			if character.is_control() {
				' '
			} else {
				character
			}
		})
		.collect::<String>();
	let trimmed = cleaned.trim();
	if trimmed.is_empty() {
		"Untitled channel".into()
	} else {
		trimmed.chars().take(100).collect()
	}
}

fn selected_reference(
	input: &MessageActionInvocation,
) -> Result<&SelectedMessageRef, &'static str> {
	let reference = input
		.selected_message_ref
		.as_ref()
		.ok_or(MISSING_SELECTION)?;
	for id in [
		&reference.channel_id,
		&reference.message_id,
		&reference.author_id,
	]
	.into_iter()
	.chain(reference.guild_id.iter())
	{
		if !valid_snowflake(id) {
			return Err(STALE_SELECTION);
		}
	}
	if input.invocation.invocation.selected_message.is_none() {
		return Err(MISSING_SELECTION);
	}
	Ok(reference)
}

fn selected_text(input: &MessageActionInvocation) -> Result<&str, &'static str> {
	selected_reference(input)?;
	input
		.invocation
		.invocation
		.selected_message
		.as_deref()
		.filter(|text| !text.trim().is_empty())
		.ok_or("The selected message has no text. Attachments are never exported by this tool.")
}

fn current_channel(input: &MessageActionInvocation) -> Result<&ChannelSnapshot, &'static str> {
	let channel = input
		.invocation
		.app
		.as_ref()
		.and_then(|app| app.context.as_ref())
		.and_then(|context| context.channel.as_ref())
		.ok_or("The host did not provide a current conversation, so nothing was proposed.")?;
	if !valid_snowflake(&channel.id) {
		return Err("The current conversation ID is invalid. Nothing was proposed.");
	}
	Ok(channel)
}

/// DM and group DM kinds are the only conversations eligible for the local
/// call ignore list; server voice channels are deliberately excluded.
fn direct_message(channel: &ChannelSnapshot) -> bool {
	matches!(channel.kind, 1 | 3)
}

/// Kinds the host can delete through its ordinary channel-management check.
/// Threads are included because the host applies its separate thread rule.
fn deletable_kind(channel: &ChannelSnapshot) -> bool {
	matches!(channel.kind, 0 | 2 | 4 | 5 | 10..=13 | 15 | 16)
}

fn valid_trigger(trigger: &str) -> bool {
	let trimmed = trigger.trim();
	!trimmed.is_empty()
		&& trimmed == trigger
		&& trigger.chars().count() <= MAX_TRIGGER_CHARS
		&& trigger.len() <= MAX_TRIGGER_BYTES
		&& !trigger.chars().any(char::is_control)
}

/// Accept only a complete public HTTPS URL. The host performs its own external
/// link validation and a second confirmation before opening anything.
fn valid_https_url(value: &str) -> bool {
	if value.is_empty()
		|| value.len() > MAX_URL_BYTES
		|| value.chars().any(|character| {
			character.is_control() || character.is_whitespace() || character == '\\'
		}) {
		return false;
	}
	let Some(rest) = value
		.get(8..)
		.filter(|_| value[..8].eq_ignore_ascii_case("https://"))
	else {
		return false;
	};
	let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
	let authority = &rest[..authority_end];
	!authority.is_empty()
		&& authority.contains('.')
		&& !authority.starts_with('.')
		&& !authority.ends_with('.')
		&& !authority.contains('@')
		&& !authority.contains(':')
}

fn parse_triggers(raw: &str) -> Result<Vec<String>, &'static str> {
	let mut triggers = Vec::new();
	for part in raw.split(',') {
		let trigger = part.trim();
		if trigger.is_empty() {
			continue;
		}
		if !valid_trigger(trigger) {
			return Err(
				"Each trigger must be a trimmed phrase of at most 64 characters without control characters.",
			);
		}
		if triggers
			.iter()
			.any(|existing: &String| existing.eq_ignore_ascii_case(trigger))
		{
			continue;
		}
		if triggers.len() >= MAX_TRIGGERS {
			return Err("At most eight distinct HopOn trigger phrases are supported.");
		}
		triggers.push(trigger.to_owned());
	}
	Ok(triggers)
}

fn hopon_matches(config: &HopOnConfig, source: &str) -> bool {
	let lower = source.to_lowercase();
	config
		.triggers
		.iter()
		.any(|trigger| lower.contains(&trigger.to_lowercase()))
}

fn export_message(input: &MessageActionInvocation) -> AppOutput {
	let reference = match selected_reference(input) {
		Ok(reference) => reference,
		Err(message) => return error_output("Export message", message),
	};
	let source = input
		.invocation
		.invocation
		.selected_message
		.as_deref()
		.unwrap_or_default();
	if source.trim().is_empty() {
		return error_output(
			"Export message",
			"The selected message has no text. Attachments, embeds and images are not exported by this bounded tool.",
		);
	}
	let mut export = format!(
		"Message {} in channel {} by author {}\n",
		reference.message_id, reference.channel_id, reference.author_id
	);
	export.push_str(source);
	if export.len() > MAX_EXPORT_BYTES {
		return error_output(
			"Export message",
			"The text export exceeds the 4,096-byte clipboard limit. Nothing was copied.",
		);
	}
	let panel = vec![
		heading("Export message"),
		text(format!(
			"Selected message {} from channel {}.",
			reference.message_id, reference.channel_id
		)),
		text(
			"Apply copies one header line and the exact message text to the clipboard. No attachment URL, embed, image, file save or network request is made.",
		),
	];
	panel_output(panel, Some(HostEffect::CopyText { text: export }))
}

fn hopon_message(input: &MessageActionInvocation) -> AppOutput {
	let source = match selected_text(input) {
		Ok(source) => source,
		Err(message) => return error_output("HopOn link", message),
	};
	let storage = load_storage(input);
	let Some(config) = storage.hopon() else {
		return error_output(
			"HopOn link",
			"Save a trigger phrase and a complete HTTPS link in the community tools panel first. Nothing was opened.",
		);
	};
	if !hopon_matches(config, source) {
		return panel_output(
			vec![
				heading("HopOn link"),
				text(
					"The selected message does not contain any saved trigger phrase. Nothing was proposed.",
				),
			],
			None,
		);
	}
	let panel = vec![
		heading("HopOn link"),
		text(format!(
			"The selected message matched a saved trigger. Apply asks Serein to open this HTTPS link:\n{}",
			config.url
		)),
		text(
			"The plugin never opens a link automatically. Serein performs its ordinary external-link confirmation after this Apply.",
		),
	];
	panel_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::OpenExternalLink {
				url: config.url.clone(),
			},
		}),
	)
}

fn save_hopon(input: &MessageActionInvocation) -> AppOutput {
	let triggers = match parse_triggers(
		input
			.invocation
			.invocation
			.value("triggers")
			.unwrap_or_default(),
	) {
		Ok(triggers) => triggers,
		Err(message) => return error_output("Save HopOn configuration", message),
	};
	let url = input
		.invocation
		.invocation
		.value("url")
		.unwrap_or_default()
		.trim();
	let hopon = if triggers.is_empty() {
		if !url.is_empty() {
			return error_output(
				"Save HopOn configuration",
				"Add at least one trigger phrase or clear the link. Nothing was saved.",
			);
		}
		None
	} else {
		if !valid_https_url(url) {
			return error_output(
				"Save HopOn configuration",
				"Enter a complete public HTTPS link without credentials, spaces or control characters. Nothing was saved.",
			);
		}
		Some(HopOnConfig {
			triggers,
			url: url.to_owned(),
		})
	};
	let storage = load_storage(input);
	let mut stored = storage.stored().cloned().unwrap_or_else(Stored::empty);
	stored.hopon = hopon;
	let mut output = panel_output(Vec::new(), None);
	if save_storage(&mut output, &stored).is_err() {
		return error_output(
			"Save HopOn configuration",
			"The configuration could not be stored within the local bound. Nothing was saved.",
		);
	}
	let mut panel = vec![heading("HopOn configuration saved")];
	if storage.is_corrupt() {
		panel.push(text(
			"The previous saved value was unreadable and has been replaced only because you explicitly saved this configuration.",
		));
	}
	panel.push(text(if stored.hopon.is_some() {
		"Use the message action on a selected message to check the saved triggers. Any match still proposes one Apply-reviewed HTTPS link."
	} else {
		"HopOn is disabled because no trigger phrases are saved. The local call ignore list is unchanged."
	}));
	output.output.panel = panel;
	output
}

fn delete_channel(input: &MessageActionInvocation) -> AppOutput {
	let channel = match current_channel(input) {
		Ok(channel) => channel,
		Err(message) => return error_output("Delete channel", message),
	};
	if channel.guild_id.is_none() {
		return error_output(
			"Delete channel",
			"Only a server channel or thread can be deleted. Direct messages are never targeted.",
		);
	}
	if !deletable_kind(channel) {
		return error_output(
			"Delete channel",
			"This channel kind is not handled by Serein's channel deletion action. Nothing was proposed.",
		);
	}
	let name = sanitize_name(&channel.name);
	let panel = vec![
		heading("Delete channel"),
		text(format!(
			"Propose permanently deleting the current channel \"{name}\" ({}).",
			channel.id
		)),
		text(
			"Apply runs Serein's native destructive confirmation and rechecks your current MANAGE_CHANNELS or thread management permission. A missing permission, a pending message or an active call in the channel fails without deleting anything.",
		),
	];
	panel_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::DeleteChannel {
				channel_id: channel.id.clone(),
			},
		}),
	)
}

fn toggle_ignored_call(input: &MessageActionInvocation) -> AppOutput {
	let channel = match current_channel(input) {
		Ok(channel) => channel,
		Err(message) => return error_output("Call ignore list", message),
	};
	if !direct_message(channel) {
		return error_output(
			"Call ignore list",
			"Only direct messages and group direct messages can be added to the local call ignore list. Nothing changed.",
		);
	}
	let storage = load_storage(input);
	let mut stored = storage.stored().cloned().unwrap_or_else(Stored::empty);
	let now_ignored = match stored
		.ignored_call_channels
		.iter()
		.position(|id| id == &channel.id)
	{
		Some(index) => {
			stored.ignored_call_channels.remove(index);
			false
		}
		None => {
			if stored.ignored_call_channels.len() >= MAX_IGNORED_CALL_CHANNELS {
				return error_output(
					"Call ignore list",
					"The local call ignore list already holds 16 conversations. Remove one before adding another.",
				);
			}
			stored.ignored_call_channels.push(channel.id.clone());
			true
		}
	};
	let mut output = panel_output(Vec::new(), None);
	if save_storage(&mut output, &stored).is_err() {
		return error_output(
			"Call ignore list",
			"The local call ignore list could not be stored within its bound. Nothing changed.",
		);
	}
	let mut panel = vec![heading("Call ignore list updated")];
	if storage.is_corrupt() {
		panel.push(text(
			"The previous saved value was unreadable and has been replaced only because you explicitly changed the list.",
		));
	}
	panel.push(text(format!(
		"{} ({}) is now {} the local call ignore list.",
		sanitize_name(&channel.name),
		channel.id,
		if now_ignored { "in" } else { "removed from" }
	)));
	panel.push(text(
		"Serein cannot suppress Discord's ring automatically. The list only lets you propose declining a call in a conversation you marked; every decline needs Apply and the host rejects stale calls.",
	));
	output.output.panel = panel;
	output
}

fn decline_ignored_call(input: &MessageActionInvocation) -> AppOutput {
	let channel = match current_channel(input) {
		Ok(channel) => channel,
		Err(message) => return error_output("Decline ignored call", message),
	};
	if !direct_message(channel) {
		return error_output(
			"Decline ignored call",
			"Only an incoming direct or group call can be declined. Nothing was proposed.",
		);
	}
	let storage = load_storage(input);
	if !storage.ignored().iter().any(|id| id == &channel.id) {
		return error_output(
			"Decline ignored call",
			"This conversation is not in the local call ignore list. Add it first; nothing was proposed.",
		);
	}
	let panel = vec![
		heading("Decline ignored call"),
		text(format!(
			"Propose declining the incoming call in \"{}\" ({}).",
			sanitize_name(&channel.name),
			channel.id
		)),
		text(
			"Apply calls Serein's typed DeclineCall action. The host rechecks that this exact conversation still has an incoming call; no ringing state is modified in the background.",
		),
	];
	panel_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::DeclineCall {
				channel_id: channel.id.clone(),
			},
		}),
	)
}

fn open_panel(invocation: &MessageActionInvocation) -> AppOutput {
	let storage = load_storage(invocation);
	let mut panel = vec![
		heading("Equicord community tools"),
		text(
			"Bounded tools adapted from ExportMessages, HopOn, FastDeleteChannels and IgnoreCalls. Every copy, deletion, link opening and call decline is an explicit Apply proposal.",
		),
	];
	if storage.is_corrupt() {
		panel.push(text(
			"The saved configuration is unreadable and is preserved unchanged. Saving a setting replaces it only after an explicit action.",
		));
	}
	match invocation
		.invocation
		.app
		.as_ref()
		.and_then(|app| app.context.as_ref())
		.and_then(|context| context.channel.as_ref())
	{
		Some(channel) if valid_snowflake(&channel.id) => panel.push(text(format!(
			"Current conversation: {} ({})",
			sanitize_name(&channel.name),
			channel.id
		))),
		_ => panel.push(text(
			"No current conversation is available. Delete and call actions require one.",
		)),
	}
	panel.push(Element::Separator);
	panel.push(heading("HopOn link"));
	match storage.hopon() {
		Some(config) => {
			panel.push(text(format!(
				"Saved triggers: {}",
				config.triggers.join(", ")
			)));
			panel.push(text(format!("Saved HTTPS link: {}", config.url)));
		}
		None => panel.push(text("HopOn is disabled.")),
	}
	panel.push(input(
		"triggers",
		"Trigger phrases (comma-separated, at most eight)",
		&storage
			.hopon()
			.map_or(String::new(), |config| config.triggers.join(", ")),
	));
	panel.push(input(
		"url",
		"HTTPS link to propose when a trigger matches",
		storage.hopon().map_or("", |config| config.url.as_str()),
	));
	panel.push(button("save-hopon", "Save HopOn configuration"));
	panel.push(text(
		"Literal case-insensitive phrase matching replaces upstream regular expressions. Automatic opening is not supported; check a selected message instead.",
	));
	panel.push(Element::Separator);
	panel.push(heading("Channel deletion"));
	panel.push(text(
		"Only the current server channel or thread can be proposed. Direct messages are never targeted.",
	));
	panel.push(button(
		"delete-channel",
		"Review deleting the current channel",
	));
	panel.push(Element::Separator);
	panel.push(heading("Calls from ignored conversations"));
	panel.push(text(
		"Serein cannot rewrite ringing state in the background. Mark a DM or group DM here, then explicitly review declining its current incoming call.",
	));
	if storage.ignored().is_empty() {
		panel.push(text("The local call ignore list is empty."));
	} else {
		for id in storage.ignored() {
			panel.push(text(format!("Ignored conversation ID: {id}")));
		}
	}
	panel.push(button(
		"toggle-ignored-call",
		"Add or remove the current conversation",
	));
	panel.push(button(
		"decline-ignored-call",
		"Review declining the current ignored conversation call",
	));
	panel_output(panel, None)
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	match input.invocation.invocation.action.as_str() {
		"open" => open_panel(&input),
		"save-hopon" => save_hopon(&input),
		"delete-channel" => delete_channel(&input),
		"toggle-ignored-call" => toggle_ignored_call(&input),
		"decline-ignored-call" => decline_ignored_call(&input),
		"export-message" => export_message(&input),
		"hopon-message" => hopon_message(&input),
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::dispatch_typed;

	fn run(value: serde_json::Value) -> (AppOutput, serde_json::Value) {
		let bytes = dispatch_typed(&serde_json::to_vec(&value).unwrap(), handle).unwrap();
		let output: AppOutput = serde_json::from_slice(&bytes).unwrap();
		let wire: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
		(output, wire)
	}

	fn reference() -> serde_json::Value {
		serde_json::json!({
			"channel_id": "100",
			"message_id": "200",
			"author_id": "300",
			"guild_id": "400"
		})
	}

	fn message(action: &str, selected: &str, storage: Option<&Stored>) -> serde_json::Value {
		let mut value = serde_json::json!({
			"action": action,
			"selected_message": selected,
			"selected_message_ref": reference()
		});
		if let Some(storage) = storage {
			value["storage"] = serde_json::Value::String(serde_json::to_string(storage).unwrap());
		}
		value
	}

	fn context(kind: u8, guild_id: Option<&str>) -> serde_json::Value {
		let mut channel = serde_json::json!({
			"id": "100",
			"name": "Current",
			"kind": kind
		});
		if let Some(guild_id) = guild_id {
			channel["guild_id"] = serde_json::Value::String(guild_id.into());
		}
		serde_json::json!({
			"action": "open",
			"app": { "context": { "connected": true, "channel": channel } }
		})
	}

	fn panel(
		action: &str,
		values: serde_json::Value,
		storage: Option<&Stored>,
	) -> serde_json::Value {
		let mut value = context(1, None);
		value["action"] = serde_json::Value::String(action.into());
		value["values"] = values;
		if let Some(storage) = storage {
			value["storage"] = serde_json::Value::String(serde_json::to_string(storage).unwrap());
		}
		value
	}

	fn panel_text(output: &AppOutput) -> String {
		fn walk(elements: &[Element], out: &mut Vec<String>) {
			for element in elements {
				match element {
					Element::Text { text } | Element::Heading { text } => out.push(text.clone()),
					Element::Row { children } => walk(children, out),
					_ => {}
				}
			}
		}
		let mut out = Vec::new();
		walk(&output.output.panel, &mut out);
		out.join("\n")
	}

	fn config() -> Stored {
		Stored {
			version: STORAGE_VERSION,
			hopon: Some(HopOnConfig {
				triggers: vec!["hop on".into(), "join me".into()],
				url: "https://example.com/launch?mode=safe".into(),
			}),
			ignored_call_channels: Vec::new(),
		}
	}

	#[test]
	fn export_copies_bounded_text_and_requires_a_selection() {
		let (output, _) = run(message("export-message", "hello\nworld", None));
		assert_eq!(output.effects.len(), 1);
		let HostEffect::CopyText { text } = &output.effects[0] else {
			panic!("expected clipboard effect");
		};
		assert!(text.contains("Message 200 in channel 100 by author 300"));
		assert!(text.ends_with("hello\nworld"));

		let (missing, _) = run(serde_json::json!({"action": "export-message"}));
		assert!(missing.effects.is_empty());
		let long = "x".repeat(MAX_EXPORT_BYTES);
		let (oversized, _) = run(message("export-message", &long, None));
		assert!(oversized.effects.is_empty());
		assert!(panel_text(&oversized).contains("4,096-byte"));
	}

	#[test]
	fn hopon_matches_literal_triggers_case_insensitively_and_only_opens_https() {
		let stored = config();
		let (matched, wire) = run(message("hopon-message", "please HOP ON now", Some(&stored)));
		assert_eq!(matched.effects.len(), 1);
		assert_eq!(wire["effects"][0]["action"]["type"], "open_external_link");
		assert_eq!(
			wire["effects"][0]["action"]["url"],
			"https://example.com/launch?mode=safe"
		);

		let (miss, _) = run(message("hopon-message", "unrelated", Some(&stored)));
		assert!(miss.effects.is_empty());

		let mut invalid = stored;
		invalid.hopon.as_mut().unwrap().url = "com.epicgames.launcher://launch".into();
		let encoded = serde_json::to_string(&invalid).unwrap();
		let (rejected, _) = run(serde_json::json!({
			"action": "hopon-message",
			"selected_message": "hop on",
			"selected_message_ref": reference(),
			"storage": encoded
		}));
		assert!(rejected.effects.is_empty());
	}

	#[test]
	fn url_validation_rejects_credentials_whitespace_and_non_https() {
		for url in [
			"http://example.com",
			"https://user@example.com/x",
			"https://example.com:443/x",
			"https://example.com/a b",
			"https://example.com\\@evil.test",
			"com.epicgames.launcher://apps/fn",
		] {
			assert!(!valid_https_url(url), "{url}");
		}
		assert!(valid_https_url("https://example.com"));
		assert!(valid_https_url("https://example.com/a?b=c#d"));
	}

	#[test]
	fn save_hopon_bounds_triggers_and_merges_the_ignore_list() {
		let mut existing = config();
		existing.ignored_call_channels = vec!["7".into()];
		let (saved, wire) = run(panel(
			"save-hopon",
			serde_json::json!({
				"triggers": " Alpha, alpha, Beta ",
				"url": "https://example.com/go"
			}),
			Some(&existing),
		));
		assert!(saved.effects.is_empty());
		let stored: Stored = serde_json::from_str(wire["storage"].as_str().unwrap()).unwrap();
		assert_eq!(
			stored.hopon.as_ref().unwrap().triggers,
			vec!["Alpha", "Beta"]
		);
		assert_eq!(stored.ignored_call_channels, vec!["7"]);

		let too_many = (0..9)
			.map(|index| format!("t{index}"))
			.collect::<Vec<_>>()
			.join(",");
		let (rejected, wire) = run(panel(
			"save-hopon",
			serde_json::json!({"triggers": too_many, "url": "https://example.com"}),
			None,
		));
		assert!(rejected.effects.is_empty());
		assert!(wire.get("storage").is_none_or(|value| value.is_null()));
	}

	#[test]
	fn delete_channel_rejects_dms_and_proposes_for_manageable_kinds() {
		let (dm, _) = run(context(1, None));
		// `open` has no effect; the same context through delete must also refuse.
		let (dm_delete, _) = run(panel("delete-channel", serde_json::json!({}), None));
		assert!(dm_delete.effects.is_empty());
		assert!(panel_text(&dm_delete).contains("Direct messages"));
		assert!(dm.effects.is_empty());

		let (voice, wire) = run(serde_json::json!({
			"action": "delete-channel",
			"app": { "context": { "connected": true, "channel": {
				"id": "100", "guild_id": "400", "name": "Voice", "kind": 2
			}}}
		}));
		assert_eq!(voice.effects.len(), 1);
		assert_eq!(wire["effects"][0]["action"]["type"], "delete_channel");
		assert_eq!(wire["effects"][0]["action"]["channel_id"], "100");

		let (unknown, _) = run(serde_json::json!({
			"action": "delete-channel",
			"app": { "context": { "connected": true, "channel": {
				"id": "100", "guild_id": "400", "name": "Directory", "kind": 14
			}}}
		}));
		assert!(unknown.effects.is_empty());
	}

	#[test]
	fn ignored_call_list_is_local_bounded_and_required_before_decline() {
		let (added, wire) = run(panel("toggle-ignored-call", serde_json::json!({}), None));
		assert!(added.effects.is_empty());
		let stored: Stored = serde_json::from_str(wire["storage"].as_str().unwrap()).unwrap();
		assert_eq!(stored.ignored_call_channels, vec!["100"]);

		let (not_listed, _) = run(panel(
			"decline-ignored-call",
			serde_json::json!({}),
			Some(&Stored::empty()),
		));
		assert!(not_listed.effects.is_empty());
		assert!(panel_text(&not_listed).contains("not in the local call ignore list"));

		let mut listed = Stored::empty();
		listed.ignored_call_channels.push("100".into());
		let (declined, wire) = run(panel(
			"decline-ignored-call",
			serde_json::json!({}),
			Some(&listed),
		));
		assert_eq!(declined.effects.len(), 1);
		assert_eq!(wire["effects"][0]["action"]["type"], "decline_call");
		assert_eq!(wire["effects"][0]["action"]["channel_id"], "100");

		let mut full = Stored::empty();
		full.ignored_call_channels = (1..=16).map(|id| id.to_string()).collect();
		let (full_output, full_wire) = run(panel(
			"toggle-ignored-call",
			serde_json::json!({}),
			Some(&full),
		));
		assert!(full_output.effects.is_empty());
		assert!(full_wire.get("storage").is_none_or(|value| value.is_null()));
		assert!(panel_text(&full_output).contains("already holds 16"));
	}

	#[test]
	fn corrupt_storage_is_preserved_until_an_explicit_save() {
		let (opened, wire) = run(serde_json::json!({
			"action": "open",
			"storage": "{\"version\":1,\"hopon\":{\"triggers\":[],\"url\":\"http://bad\"}}",
			"app": { "context": { "connected": true } }
		}));
		assert!(wire.get("storage").is_none_or(|value| value.is_null()));
		assert!(panel_text(&opened).contains("unreadable and is preserved"));
	}
}
