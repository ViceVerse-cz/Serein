/*
 * Serein plus reacts tools
 * Copyright (c) 2026 Serein port contributors
 *
 * Safe port of Equicord's `betterPlusReacts` plugin
 * (Copyright (c) 2024 Vendicated and contributors), licensed
 * GPL-3.0-or-later, at upstream commit
 * 200dba124066146a8f043de7fc8b05e3dc5e68e.
 * Changed November 8, 2026: the upstream composer patch and MessageStore lookup
 * are replaced by an explicit composer tool over the already-loaded bounded
 * timeline. It proposes exactly one Apply-reviewed reaction and never sends a
 * message, fetches history, patches Discord modules or applies the reaction
 * automatically.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppAction, AppInvocation, AppOutput, Element, HostEffect, MAX_IO_BYTES, Output,
	TimelineSnapshot, serde_json,
};

/// The upstream plugin reads the whole loaded message store. Serein only exposes
/// the bounded timeline window, so a leading-plus count may never exceed this.
const MAX_LOOKBACK: usize = 50;
/// A composer draft is normally tiny; bound it before scanning so hostile input
/// cannot force a large allocation or unbounded parsing.
const MAX_DRAFT_BYTES: usize = 512;
/// The host's reaction validator accepts at most 128 bytes per emoji.
const MAX_EMOJI_BYTES: usize = 128;
/// Keep the preview short; the panel is a confirmation, not a message viewer.
const MAX_PREVIEW_BYTES: usize = 160;
/// Stays well below the host's 64-element panel cap; this panel uses six.
const MAX_PANEL_ELEMENTS: usize = 32;

const MISSING_DRAFT: &str =
	"Open this tool from the composer tools menu after typing a bounded draft.";
const NO_TIMELINE: &str = "Serein has not loaded a readable timeline for this conversation, so there is no message to react to.";
const TRUNCATED_NOTE: &str =
	"Only the currently loaded timeline is searched. Older messages are not fetched or guessed.";
const APPLY_NOTE: &str =
	"Apply adds only your reaction. No message is sent and the draft is left unchanged.";

struct ParsedDraft {
	plus_count: usize,
	emoji: String,
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

fn is_snowflake(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 20
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn valid_custom_name(name: &str) -> bool {
	(2..=32).contains(&name.len())
		&& name
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn custom_emoji(name: &str, id: &str) -> Option<String> {
	(valid_custom_name(name) && is_snowflake(id)).then(|| format!("{name}:{id}"))
}

/// Accept only a single Unicode emoji or a custom `name:id`/`<:name:id>` value.
/// Shortcode text such as `:smile:` is rejected because the host would treat it
/// as literal Unicode text, not as a resolver.
fn parse_emoji(raw: &str) -> Result<String, &'static str> {
	let value = raw.trim();
	if value.is_empty() {
		return Err("Add one emoji after the leading plus signs.");
	}
	if value.len() > MAX_EMOJI_BYTES {
		return Err("The emoji is longer than the 128-byte reaction limit.");
	}
	if value
		.chars()
		.any(|character| character.is_control() || character.is_whitespace())
	{
		return Err("Use one emoji without spaces or control characters.");
	}
	if value.contains('+') {
		return Err("Separate the leading plus signs from the emoji.");
	}
	if value.starts_with('<') || value.ends_with('>') {
		let inner = value
			.strip_prefix('<')
			.and_then(|value| value.strip_suffix('>'))
			.ok_or("Use the emoji itself, `name:id`, or `<:name:id>`.")?;
		let inner = inner.strip_prefix("a:").unwrap_or(inner);
		let inner = inner.strip_prefix(':').unwrap_or(inner);
		let (name, id) = inner
			.split_once(':')
			.ok_or("Use a custom emoji as `name:id` or `<:name:id>`.")?;
		if id.contains(':') {
			return Err("A custom emoji must name exactly one emoji ID.");
		}
		return custom_emoji(name, id)
			.ok_or("A custom emoji needs a 2-32 character name and a nonzero numeric ID.");
	}
	if let Some((name, id)) = value.split_once(':') {
		if id.contains(':') {
			return Err("A custom emoji must name exactly one emoji ID.");
		}
		return custom_emoji(name, id)
			.ok_or("A custom emoji needs a 2-32 character name and a nonzero numeric ID.");
	}
	Ok(value.to_owned())
}

fn parse_draft(draft: &str) -> Result<ParsedDraft, &'static str> {
	if draft.len() > MAX_DRAFT_BYTES {
		return Err(
			"The draft is too long for this tool. Keep the plus-prefixed emoji under 512 bytes.",
		);
	}
	let trimmed = draft.trim();
	let plus_count = trimmed.bytes().take_while(|byte| *byte == b'+').count();
	if plus_count == 0 {
		return Err("Start the draft with one or more '+' signs, then one emoji.");
	}
	if plus_count > MAX_LOOKBACK {
		return Err("At most 50 leading plus signs are supported by the loaded timeline.");
	}
	let emoji = parse_emoji(&trimmed[plus_count..])?;
	Ok(ParsedDraft { plus_count, emoji })
}

fn preview(content: &str) -> String {
	let cleaned = content
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
		"(no text)".into()
	} else {
		truncate_utf8(trimmed, MAX_PREVIEW_BYTES).to_owned()
	}
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn panel_output(panel: Vec<Element>, effect: Option<HostEffect>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		effects: effect.into_iter().collect(),
	}
}

fn error_output(message: &str) -> AppOutput {
	panel_output(
		vec![
			Element::Heading {
				text: "Plus reacts".into(),
			},
			text(message),
		],
		None,
	)
}

/// The newest loaded message is one plus; every further plus walks one row back.
/// This is a documented bounded interpretation of upstream's store-index math.
fn target_for(
	timeline: &TimelineSnapshot,
	plus_count: usize,
) -> Result<&serein_extension_sdk::MessageSnapshot, &'static str> {
	if !is_snowflake(&timeline.channel_id) {
		return Err("The loaded timeline has no usable channel ID.");
	}
	timeline
		.messages
		.iter()
		.rev()
		.nth(plus_count - 1)
		.filter(|message| is_snowflake(&message.id))
		.ok_or("There are not enough loaded messages for that many plus signs. Remove some plus signs or load older history.")
}

fn plus_react(input: &AppInvocation) -> Result<AppOutput, &'static str> {
	let draft = input.invocation.composer.as_deref().ok_or(MISSING_DRAFT)?;
	let parsed = parse_draft(draft)?;
	let timeline = input
		.app
		.as_ref()
		.and_then(|app| app.timeline.as_ref())
		.ok_or(NO_TIMELINE)?;
	let target = target_for(timeline, parsed.plus_count)?;

	let panel = vec![
		Element::Heading {
			text: "Plus reacts".into(),
		},
		text("This proposes adding your reaction to an earlier loaded message."),
		text(format!(
			"Target ({} back from the newest loaded message): {} - {}",
			parsed.plus_count,
			preview(&target.author.name),
			preview(&target.content)
		)),
		text(format!("Reaction: {}", parsed.emoji)),
		text(if timeline.truncated {
			TRUNCATED_NOTE
		} else {
			"Every message in the loaded window is available to this count."
		}),
		text(APPLY_NOTE),
	];
	if panel.len() > MAX_PANEL_ELEMENTS {
		return Err("The confirmation panel exceeded its element budget.");
	}

	Ok(panel_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::SetReaction {
				channel_id: timeline.channel_id.clone(),
				message_id: target.id.clone(),
				emoji: parsed.emoji,
				add: true,
			},
		}),
	))
}

fn handle(input: AppInvocation) -> AppOutput {
	let output = plus_react(&input).unwrap_or_else(error_output);
	match serde_json::to_vec(&output) {
		Ok(bytes) if bytes.len() <= MAX_IO_BYTES => output,
		_ => error_output(
			"The response exceeded the extension's 256 KiB limit. Nothing was proposed.",
		),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json, serde_json::json};

	fn run(value: serde_json::Value) -> AppOutput {
		let bytes = dispatch_typed(&serde_json::to_vec(&value).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	fn app() -> serde_json::Value {
		json!({
			"timeline": {
				"channel_id": "100",
				"messages": [
					{"id":"200","author":{"id":"10","name":"Root"},"content":"root","attachment_count":0,"edited":false},
					{"id":"300","author":{"id":"11","name":"Second"},"content":"second\nline","attachment_count":0,"edited":false},
					{"id":"400","author":{"id":"12","name":"Latest"},"content":"latest","attachment_count":0,"edited":false}
				],
				"truncated": true
			}
		})
	}

	fn action(draft: &str) -> serde_json::Value {
		json!({
			"action": "plus-react",
			"composer": draft,
			"app": app()
		})
	}

	#[test]
	fn counts_back_from_the_newest_loaded_message() {
		let output = run(action("++\u{1f389}"));
		assert_eq!(
			output.effects,
			vec![HostEffect::AppAction {
				action: AppAction::SetReaction {
					channel_id: "100".into(),
					message_id: "300".into(),
					emoji: "\u{1f389}".into(),
					add: true
				}
			}]
		);
		assert!(output.output.replacement.is_none());
		assert!(output.output.storage.is_none());
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("second line")
		)));
	}

	#[test]
	fn normalizes_custom_emoji_markup_but_rejects_shortcodes_and_bad_ids() {
		for (draft, expected) in [
			("+party:123456789", "party:123456789"),
			("+<:party:123456789>", "party:123456789"),
			("+<a:party:123456789>", "party:123456789"),
		] {
			let output = run(action(draft));
			assert_eq!(
				output.effects,
				vec![HostEffect::AppAction {
					action: AppAction::SetReaction {
						channel_id: "100".into(),
						message_id: "400".into(),
						emoji: expected.into(),
						add: true
					}
				}]
			);
		}
		for draft in [
			":smile:",
			"+:smile:",
			"+party:0",
			"+x:123456789",
			"+party:not-an-id",
			"+party:1:2",
			"+bad name:123456789",
		] {
			let output = run(action(draft));
			assert!(output.effects.is_empty(), "{draft}");
			assert!(output.output.panel.iter().any(|element| matches!(
				element,
				Element::Text { text } if text.contains("emoji") || text.contains("plus")
			)));
		}
	}

	#[test]
	fn refuses_missing_signs_short_timelines_and_oversized_counts() {
		for draft in ["", "\u{1f389}", "+"] {
			let output = run(action(draft));
			assert!(output.effects.is_empty(), "{draft}");
		}
		let too_few = run(action("++++\u{1f389}"));
		assert!(too_few.effects.is_empty());
		assert!(too_few.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("not enough loaded messages")
		)));

		let too_many = format!("{}{}", "+".repeat(MAX_LOOKBACK + 1), "\u{1f389}");
		let output = run(action(&too_many));
		assert!(output.effects.is_empty());
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("At most 50")
		)));
	}

	#[test]
	fn rejects_a_timeline_without_a_usable_channel_or_message_id() {
		let mut value = action("+\u{1f389}");
		value["app"]["timeline"]["channel_id"] = json!("0");
		assert!(run(value).effects.is_empty());

		let mut value = action("+\u{1f389}");
		value["app"]["timeline"]["messages"][2]["id"] = json!("not-an-id");
		assert!(run(value).effects.is_empty());
	}
}
