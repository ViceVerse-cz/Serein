/*
 * Vencord, a modification for Discord's desktop app
 * Copyright (c) 2025 Vendicated and contributors
 *
 * Upstream source at Equicord commit 200dba124066146a8f043fde7fc8b05e3dc5e68e:
 * src/plugins/moreQuickReactions/index.ts.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License as published by the Free Software
 * Foundation, either version 3 of the License, or (at your option) any later
 * version.
 *
 * This program is distributed in the hope that it will be useful, but WITHOUT
 * ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
 * FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License along with
 * this program. If not, see <https://www.gnu.org/licenses/>.
 */
use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	AppAction, AppInvocation, AppOutput, Element, HostEffect, MAX_IO_BYTES, MessageSnapshot,
	Output, TimelineSnapshot, serde_json,
};

/// Upstream's configured ceiling for the quick reaction count.
const MAX_REACTIONS: usize = 42;
/// One reaction token, kept well below the host's 128-byte emoji name limit.
const MAX_EMOJI_BYTES: usize = 32;
/// `id:name`: a 20-byte snowflake, one colon and a 32-byte custom name.
const MAX_CUSTOM_EMOJI_BYTES: usize = 53;
/// The panel configuration field is one bounded line.
const MAX_CONFIG_BYTES: usize = 1024;
/// The app snapshot can hold more messages; the selector stays deliberately small.
const MAX_PANEL_MESSAGES: usize = 20;
/// The host rejects Select options above 128 bytes.
const MAX_LABEL_BYTES: usize = 128;
/// The host bounds plugin storage at 16 KiB; settings stay far below it.
const MAX_SETTINGS_BYTES: usize = 16 * 1024;
const CORRUPT_STORAGE: &str = "Saved quick reactions are invalid or too large. Nothing was changed; the original data was preserved. Clear this plugin's data in Extensions settings to start over.";

const MESSAGE_FIELD: &str = "message";
const REACTIONS_FIELD: &str = "reactions";
const EMOJI_FIELD: &str = "emoji";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
	#[serde(default)]
	reactions: Vec<String>,
}

enum Loaded {
	Missing,
	Valid(Vec<String>),
	Corrupt,
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

/// Discord snowflakes are canonical nonzero u64 decimal strings.
fn canonical_id(value: &str) -> Option<&str> {
	let bytes = value.as_bytes();
	if bytes.is_empty()
		|| bytes.len() > 20
		|| bytes[0] == b'0'
		|| !bytes.iter().all(u8::is_ascii_digit)
	{
		return None;
	}
	value
		.parse::<u64>()
		.ok()
		.filter(|value| *value != 0)
		.map(|_| value)
}

/// Accepts a plain unicode emoji sequence or the host's `id:name` custom form.
///
/// This is intentionally narrower than the host's lenient `ReactionEmoji::valid`:
/// plain ASCII words, whitespace, control characters and oversized tokens are not
/// reaction emoji and are rejected before any proposal is built.
fn normalize_emoji(value: &str) -> Option<String> {
	if value.is_empty() || value.len() > MAX_CUSTOM_EMOJI_BYTES {
		return None;
	}
	if let Some((id, name)) = value.split_once(':') {
		let valid_name = (2..=MAX_EMOJI_BYTES).contains(&name.len())
			&& name
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
		if canonical_id(id).is_some() && valid_name {
			return Some(value.to_owned());
		}
		return None;
	}
	if value.len() > MAX_EMOJI_BYTES
		|| value
			.chars()
			.any(|character| character.is_control() || character.is_whitespace())
	{
		return None;
	}
	if !value.is_ascii() {
		Some(value.to_owned())
	} else {
		None
	}
}

/// Splits one bounded line into at most 42 distinct validated reactions.
fn parse_reactions(input: &str) -> Result<Vec<String>, &'static str> {
	if input.len() > MAX_CONFIG_BYTES {
		return Err("The reaction list exceeds 1,024 UTF-8 bytes. Nothing was saved.");
	}
	let mut reactions: Vec<String> = Vec::new();
	for token in input.split(|character: char| character.is_whitespace() || character == ',') {
		if token.is_empty() {
			continue;
		}
		let Some(emoji) = normalize_emoji(token) else {
			return Err(
				"One entry is not a unicode emoji or a valid id:name custom emoji. Nothing was saved.",
			);
		};
		if !reactions.contains(&emoji) {
			reactions.push(emoji);
		}
		if reactions.len() > MAX_REACTIONS {
			return Err("At most 42 quick reactions are supported. Nothing was saved.");
		}
	}
	Ok(reactions)
}

fn load(invocation: &AppInvocation) -> Loaded {
	if invocation
		.invocation
		.storage
		.as_ref()
		.is_some_and(|storage| storage.len() > MAX_SETTINGS_BYTES)
	{
		return Loaded::Corrupt;
	}
	match invocation.invocation.storage_json::<serde_json::Value>() {
		Ok(None) => Loaded::Missing,
		Ok(Some(value)) => {
			// Structs can otherwise decode from JSON sequences, including `[]`.
			// Persisted settings are always one object with exactly the declared fields.
			if !value.is_object() {
				return Loaded::Corrupt;
			}
			let Ok(settings) = serde_json::from_value::<Settings>(value) else {
				return Loaded::Corrupt;
			};
			if settings.reactions.len() > MAX_REACTIONS
				|| settings.reactions.iter().any(|emoji| {
					emoji.len() > MAX_CUSTOM_EMOJI_BYTES
						|| normalize_emoji(emoji).as_deref() != Some(emoji.as_str())
				}) {
				Loaded::Corrupt
			} else {
				Loaded::Valid(settings.reactions)
			}
		}
		Err(_) => Loaded::Corrupt,
	}
}

/// Inline, single-line display text: drop controls, collapse whitespace and cap bytes.
fn clean_inline(input: &str, limit: usize) -> String {
	let mut out = String::new();
	let mut bytes = 0;
	let mut last_space = false;
	for character in input.chars() {
		let character = if character.is_control() || character.is_whitespace() {
			' '
		} else {
			character
		};
		if character == ' ' {
			if last_space || out.is_empty() {
				continue;
			}
			last_space = true;
		} else {
			last_space = false;
		}
		if bytes + character.len_utf8() > limit {
			break;
		}
		out.push(character);
		bytes += character.len_utf8();
	}
	out.trim_end().to_owned()
}

fn message_label(message: &MessageSnapshot) -> Option<String> {
	let id = canonical_id(&message.id)?;
	let author = clean_inline(&message.author.name, 24);
	let author = if author.is_empty() {
		"unknown".to_owned()
	} else {
		author
	};
	let body = clean_inline(&message.content, 72);
	let body = if body.is_empty() {
		"(no text)".to_owned()
	} else {
		body
	};
	let label = format!("{id} · {author}: {body}");
	Some(clean_inline(&label, MAX_LABEL_BYTES))
}

/// Loaded messages newest first, at most 20 unique labels.
fn message_options(timeline: &TimelineSnapshot) -> Vec<String> {
	let mut options = Vec::new();
	let mut seen = std::collections::BTreeSet::new();
	for message in timeline.messages.iter().rev() {
		let Some(label) = message_label(message) else {
			continue;
		};
		if seen.insert(label.clone()) {
			options.push(label);
		}
		if options.len() == MAX_PANEL_MESSAGES {
			break;
		}
	}
	options
}

fn loaded_timeline(invocation: &AppInvocation) -> Option<&TimelineSnapshot> {
	invocation
		.app
		.as_ref()
		.and_then(|app| app.timeline.as_ref())
}

fn panel(
	reactions: &[String],
	timeline: Option<&TimelineSnapshot>,
	status: Option<&str>,
) -> Vec<Element> {
	let mut panel = vec![
		heading("More Quick Reactions"),
		text(
			"Upstream adds a configurable quick-reaction row to Discord's message hover bar. This sandboxed port cannot render chat rows, so it provides an explicit palette instead: configure up to 42 reactions and add one to a loaded message in the active channel. The host's Apply confirmation is always required.",
		),
	];
	if let Some(status) = status {
		panel.push(text(status));
	}
	let options = timeline.map(message_options).unwrap_or_default();
	if options.is_empty() {
		panel.push(text(
			"Open a readable channel with loaded messages to choose a target. No message was proposed.",
		));
	} else {
		panel.push(Element::Select {
			id: MESSAGE_FIELD.into(),
			label: "Loaded message".into(),
			options: options.clone(),
			value: options[0].clone(),
		});
	}
	panel.push(Element::TextInput {
		id: REACTIONS_FIELD.into(),
		label: "Quick reactions (space or comma separated)".into(),
		value: reactions.join(" "),
	});
	panel.push(Element::TextInput {
		id: EMOJI_FIELD.into(),
		label: "Emoji to add to the selected message".into(),
		value: reactions.first().cloned().unwrap_or_default(),
	});
	panel.push(Element::Row {
		children: vec![
			Element::Button {
				id: "save".into(),
				label: "Save reaction list".into(),
			},
			Element::Button {
				id: "react".into(),
				label: "Add reaction after Apply".into(),
			},
		],
	});
	panel.push(text(format!(
		"Each entry is one unicode emoji (at most {} UTF-8 bytes) or an id:name custom emoji (at most {} bytes). The configured list is capped at {} entries and 1,024 bytes. Loaded-message choices are capped at {}.",
		MAX_EMOJI_BYTES, MAX_CUSTOM_EMOJI_BYTES, MAX_REACTIONS, MAX_PANEL_MESSAGES
	)));
	panel
}

fn corrupt_panel() -> AppOutput {
	AppOutput {
		output: Output {
			panel: vec![heading("More Quick Reactions"), text(CORRUPT_STORAGE)],
			..Default::default()
		},
		..Default::default()
	}
}

fn error_panel(
	reactions: &[String],
	timeline: Option<&TimelineSnapshot>,
	message: &str,
) -> AppOutput {
	AppOutput {
		output: Output {
			panel: panel(reactions, timeline, Some(message)),
			..Default::default()
		},
		..Default::default()
	}
}

fn open(invocation: &AppInvocation) -> AppOutput {
	let reactions = match load(invocation) {
		Loaded::Missing => Vec::new(),
		Loaded::Valid(reactions) => reactions,
		Loaded::Corrupt => return corrupt_panel(),
	};
	AppOutput {
		output: Output {
			panel: panel(&reactions, loaded_timeline(invocation), None),
			..Default::default()
		},
		..Default::default()
	}
}

fn save(invocation: &AppInvocation) -> AppOutput {
	let current = match load(invocation) {
		Loaded::Missing => Vec::new(),
		Loaded::Valid(reactions) => reactions,
		Loaded::Corrupt => return corrupt_panel(),
	};
	let Some(input) = invocation.invocation.value(REACTIONS_FIELD) else {
		return error_panel(
			&current,
			loaded_timeline(invocation),
			"The reaction list field was missing. Nothing was saved.",
		);
	};
	let reactions = match parse_reactions(input) {
		Ok(reactions) => reactions,
		Err(message) => return error_panel(&current, loaded_timeline(invocation), message),
	};
	let settings = Settings {
		reactions: reactions.clone(),
	};
	let status = format!(
		"Saved {} quick reaction{}.",
		reactions.len(),
		if reactions.len() == 1 { "" } else { "s" }
	);
	let mut output = Output {
		panel: panel(
			&reactions,
			loaded_timeline(invocation),
			Some(status.as_str()),
		),
		..Default::default()
	};
	if output.set_storage_json(&settings).is_err() {
		return error_panel(
			&current,
			loaded_timeline(invocation),
			"The saved list exceeded the storage limit. Nothing was saved.",
		);
	}
	AppOutput {
		output,
		..Default::default()
	}
}

fn react(invocation: &AppInvocation) -> AppOutput {
	let reactions = match load(invocation) {
		Loaded::Missing => Vec::new(),
		Loaded::Valid(reactions) => reactions,
		Loaded::Corrupt => return corrupt_panel(),
	};
	let timeline = loaded_timeline(invocation);
	let Some(timeline) = timeline else {
		return error_panel(
			&reactions,
			None,
			"No loaded timeline is available. Nothing was proposed.",
		);
	};
	if canonical_id(&timeline.channel_id).is_none() {
		return error_panel(
			&reactions,
			Some(timeline),
			"The host supplied no valid channel ID. Nothing was proposed.",
		);
	}
	let Some(selected) = invocation.invocation.value(MESSAGE_FIELD) else {
		return error_panel(
			&reactions,
			Some(timeline),
			"No loaded message was selected. Nothing was proposed.",
		);
	};
	let target = timeline
		.messages
		.iter()
		.rev()
		.find(|message| message_label(message).as_deref() == Some(selected));
	let Some(target) = target else {
		return error_panel(
			&reactions,
			Some(timeline),
			"That message is no longer in the loaded window. Nothing was proposed.",
		);
	};
	let Some(message_id) = canonical_id(&target.id) else {
		return error_panel(
			&reactions,
			Some(timeline),
			"The selected message has no valid ID. Nothing was proposed.",
		);
	};
	let Some(emoji) = invocation
		.invocation
		.value(EMOJI_FIELD)
		.and_then(normalize_emoji)
	else {
		return error_panel(
			&reactions,
			Some(timeline),
			"Enter one unicode emoji or a valid id:name custom emoji. Nothing was proposed.",
		);
	};
	AppOutput {
		output: Output::default(),
		effects: vec![HostEffect::AppAction {
			action: AppAction::SetReaction {
				channel_id: timeline.channel_id.clone(),
				message_id: message_id.to_owned(),
				emoji,
				add: true,
			},
		}],
	}
}

fn handle(invocation: AppInvocation) -> AppOutput {
	let output = match invocation.invocation.action.as_str() {
		"open" => open(&invocation),
		"save" => save(&invocation),
		"react" => react(&invocation),
		_ => AppOutput::default(),
	};
	match serde_json::to_vec(&output) {
		Ok(bytes) if bytes.len() <= MAX_IO_BYTES => output,
		_ => error_panel(
			&[],
			None,
			"The response exceeded the extension's 256 KiB limit. Nothing was proposed.",
		),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{
		UserSnapshot, dispatch_typed,
		serde_json::{self, json},
	};

	fn message(id: &str, author: &str, content: &str) -> MessageSnapshot {
		MessageSnapshot {
			id: id.into(),
			author: UserSnapshot {
				id: "9".into(),
				name: author.into(),
			},
			content: content.into(),
			attachment_count: 0,
			edited: false,
		}
	}

	fn timeline(messages: Vec<MessageSnapshot>) -> TimelineSnapshot {
		TimelineSnapshot {
			channel_id: "10".into(),
			messages,
			truncated: false,
		}
	}

	fn run(value: serde_json::Value) -> AppOutput {
		let bytes = dispatch_typed(&serde_json::to_vec(&value).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	fn effect_emoji(output: &AppOutput) -> Option<(&str, &str, &str, bool)> {
		let [
			HostEffect::AppAction {
				action:
					AppAction::SetReaction {
						channel_id,
						message_id,
						emoji,
						add,
					},
			},
		] = output.effects.as_slice()
		else {
			return None;
		};
		Some((
			channel_id.as_str(),
			message_id.as_str(),
			emoji.as_str(),
			*add,
		))
	}

	#[test]
	fn emoji_tokens_are_bounded_and_unambiguous() {
		for valid in ["👍", "❤️", "👨‍👩‍👧‍👦", "1️⃣", "12:husk_emoji"] {
			assert!(
				normalize_emoji(valid).is_some(),
				"{valid} should be accepted"
			);
		}
		for invalid in [
			"",
			"hello",
			"a b",
			"0:name",
			"12:a",
			"12:has space",
			"12:name:extra",
			"123456789012345678901:name",
			"👍\n",
			"👍👍👍👍👍👍👍👍👍👍👍👍👍👍👍👍👍",
		] {
			assert!(
				normalize_emoji(invalid).is_none(),
				"{invalid:?} should be rejected"
			);
		}
	}

	#[test]
	fn save_deduplicates_and_rejects_oversized_lists_without_writing() {
		let output = run(json!({
			"action": "save",
			"values": {"reactions": "👍, 👍 ❤️"},
			"app": {"timeline": timeline(vec![message("11", "alice", "hi")])}
		}));
		assert!(output.effects.is_empty());
		let saved: Settings =
			serde_json::from_str(output.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(saved.reactions, vec!["👍".to_owned(), "❤️".to_owned()]);

		let too_many = (1..=43)
			.map(|id| format!("{id}:aa"))
			.collect::<Vec<_>>()
			.join(" ");
		let output = run(json!({
			"action": "save",
			"values": {"reactions": too_many}
		}));
		assert!(output.output.storage.is_none());
		assert!(output.output.panel.iter().any(
			|element| matches!(element, Element::Text { text } if text.contains("At most 42"))
		));
	}

	#[test]
	fn corrupt_or_oversized_storage_is_preserved() {
		for storage in ["not json", "{\"reactions\":[\"plain word\"]}", "[]"] {
			let output = run(json!({
				"action": "save",
				"storage": storage,
				"values": {"reactions": "👍"}
			}));
			assert!(
				output.output.storage.is_none(),
				"{storage:?} was overwritten"
			);
			assert!(output.effects.is_empty());
			assert!(output.output.panel.iter().any(
				|element| matches!(element, Element::Text { text } if text.contains("original data was preserved"))
			));
		}
	}

	#[test]
	fn react_requires_a_loaded_message_and_proposes_one_reaction() {
		let messages = vec![
			message("11", "alice", "first"),
			message("12", "bob", "second"),
		];
		let output = run(json!({
			"action": "react",
			"values": {"message": "11 · alice: first", "emoji": "👍"},
			"app": {"timeline": timeline(messages.clone())}
		}));
		assert_eq!(effect_emoji(&output), Some(("10", "11", "👍", true)));
		assert!(output.output.panel.is_empty());

		for value in [
			json!({"message": "999 · nobody: missing", "emoji": "👍"}),
			json!({"message": "11 · alice: first", "emoji": "plain"}),
			json!({"message": "11 · alice: first"}),
			json!({"emoji": "👍"}),
		] {
			let output = run(json!({
				"action": "react",
				"values": value,
				"app": {"timeline": timeline(messages.clone())}
			}));
			assert!(output.effects.is_empty());
			assert!(!output.output.panel.is_empty());
		}

		let output = run(json!({
			"action": "react",
			"values": {"message": "11 · alice: first", "emoji": "👍"}
		}));
		assert!(output.effects.is_empty());
	}

	#[test]
	fn panel_is_bounded_and_matches_the_manifest_actions() {
		let messages = (11..75)
			.map(|id| message(&id.to_string(), "author", &"x".repeat(80)))
			.collect::<Vec<_>>();
		let output = run(json!({
			"action": "open",
			"app": {"timeline": timeline(messages)}
		}));
		assert!(output.output.panel.len() <= 12);
		let manifest: serde_json::Value =
			serde_json::from_str(include_str!("../manifest.json")).unwrap();
		let actions = manifest["actions"]
			.as_array()
			.unwrap()
			.iter()
			.map(|action| action["id"].as_str().unwrap().to_owned())
			.collect::<std::collections::BTreeSet<_>>();
		fn walk(elements: &[Element], actions: &std::collections::BTreeSet<String>) {
			for element in elements {
				match element {
					Element::Button { id, .. } => assert!(actions.contains(id)),
					Element::Select {
						options,
						value,
						label,
						..
					} => {
						assert!(!options.is_empty() && options.len() <= MAX_PANEL_MESSAGES);
						assert!(options.contains(value));
						assert!(options.iter().all(|option| option.len() <= MAX_LABEL_BYTES));
						assert!(
							options
								.iter()
								.collect::<std::collections::BTreeSet<_>>()
								.len() == options.len()
						);
						assert!(!label.is_empty());
					}
					Element::Row { children } => walk(children, actions),
					Element::Text { .. }
					| Element::Heading { .. }
					| Element::TextInput { .. }
					| Element::Checkbox { .. }
					| Element::Slider { .. }
					| Element::Separator
					| Element::ActivityPreview { .. } => {}
				}
			}
		}
		walk(&output.output.panel, &actions);
	}
}
