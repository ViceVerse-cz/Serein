/*
 * Vencord, a Discord client mod
 * Copyright (c) 2026 Vendicated and contributors
 *
 * Serein adaptation copyright (c) 2026 Serein contributors.
 * Changed November 8, 2026: replaces the upstream composer hook and arbitrary
 * regex modes with an explicit, foreground own-message edit proposal.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License as published by the Free Software
 * Foundation, either version 3 of the License, or (at your option) any later
 * version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */

use serein_extension_sdk::{
	AppAction, AppInvocation, AppOutput, Element, HostEffect, MessageDetailSnapshot,
	MessageDetailsSnapshot, MessageSnapshot, Output, TimelineSnapshot,
};

/// The literal search text is deliberately small; it never becomes a regex.
const MAX_MATCH_BYTES: usize = 256;
/// Replacement text stays below the ordinary 2,000-character message limit.
const MAX_REPLACE_BYTES: usize = 2_000;
/// Host message edits are bounded by both characters and UTF-8 bytes.
const MAX_RESULT_CHARS: usize = 2_000;
const MAX_RESULT_BYTES: usize = 8_000;

const MISSING_APP: &str =
	"Serein did not provide the current account and conversation, so nothing can be edited.";
const STALE_CONTEXT: &str = "The loaded timeline no longer belongs to the selected conversation. Reopen the editor and try again.";
const MISSING_METADATA: &str = "Serein did not provide loaded message metadata for this conversation, so the target cannot be verified.";
const MISSING_TARGET: &str = "The target message is no longer in the bounded window Serein has loaded. Nothing was proposed.";
const FOREIGN_TARGET: &str =
	"The target message was not authored by the current account. Nothing was proposed.";
const NON_ORDINARY_TARGET: &str =
	"The target is not an ordinary loaded message. Nothing was proposed.";

struct Scope<'a> {
	channel_id: &'a str,
	own_id: &'a str,
	timeline: &'a TimelineSnapshot,
	details: Option<&'a MessageDetailsSnapshot>,
}

struct Target<'a> {
	message: &'a MessageSnapshot,
	detail: &'a MessageDetailSnapshot,
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
				text: "Own message sed".into(),
			},
			text(message),
			Element::Button {
				id: "show".into(),
				label: "Refresh editor".into(),
			},
		],
		None,
	)
}

fn no_effect(message: &str) -> AppOutput {
	panel_output(
		vec![
			Element::Heading {
				text: "Own message sed".into(),
			},
			text(message),
			Element::Button {
				id: "show".into(),
				label: "Back to editor".into(),
			},
		],
		None,
	)
}

/// IDs must be nonzero decimal snowflakes in canonical form. Leading zeroes,
/// signs, whitespace and overlong values are stale or malformed, never guessed.
fn canonical_id(value: &str) -> bool {
	let Ok(id) = value.parse::<u64>() else {
		return false;
	};
	id != 0
		&& value.len() <= 20
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value == id.to_string()
}

fn is_ordinary(kind: u8) -> bool {
	matches!(kind, 0 | 19)
}

/// Literal matching keeps upstream's escape removal for `/`, `*`, `+` and `?`.
/// Every other backslash is retained as a literal backslash.
fn unescape_literal(value: &str) -> String {
	let mut result = String::with_capacity(value.len());
	let mut chars = value.chars().peekable();
	while let Some(ch) = chars.next() {
		if ch == '\\' {
			if let Some(&next) = chars.peek() {
				if matches!(next, '/' | '*' | '+' | '?') {
					chars.next();
					result.push(next);
				} else {
					result.push(ch);
				}
			} else {
				result.push(ch);
			}
		} else {
			result.push(ch);
		}
	}
	result
}

fn has_unsupported_control(value: &str) -> bool {
	value
		.chars()
		.any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
}

fn scope(input: &AppInvocation) -> Result<Scope<'_>, &'static str> {
	let app = input.app.as_ref().ok_or(MISSING_APP)?;
	let context = app.context.as_ref().ok_or(MISSING_APP)?;
	if !context.connected {
		return Err(MISSING_APP);
	}
	let user = context.user.as_ref().ok_or(MISSING_APP)?;
	let channel = context.channel.as_ref().ok_or(MISSING_APP)?;
	if !canonical_id(&user.id) || !canonical_id(&channel.id) {
		return Err(STALE_CONTEXT);
	}
	let timeline = app.timeline.as_ref().ok_or(MISSING_METADATA)?;
	if timeline.channel_id != channel.id || !canonical_id(&timeline.channel_id) {
		return Err(STALE_CONTEXT);
	}
	if let Some(details) = &app.message_details
		&& (details.channel_id != channel.id || !canonical_id(&details.channel_id))
	{
		return Err(STALE_CONTEXT);
	}
	Ok(Scope {
		channel_id: &channel.id,
		own_id: &user.id,
		timeline,
		details: app.message_details.as_ref(),
	})
}

/// Only a live, ordinary timeline row with matching bounded metadata is a valid
/// target. A requested ID must match exactly; the editor never guesses another row.
fn select_target<'a>(
	scope: &'a Scope<'a>,
	requested: Option<&str>,
) -> Result<Target<'a>, &'static str> {
	let details = scope.details.ok_or(MISSING_METADATA)?;
	let candidate = match requested {
		Some(id) => {
			if !canonical_id(id) {
				return Err(MISSING_TARGET);
			}
			let message = scope
				.timeline
				.messages
				.iter()
				.find(|message| message.id == id)
				.ok_or(MISSING_TARGET)?;
			let detail = details
				.items
				.iter()
				.find(|detail| detail.id == id)
				.ok_or(MISSING_TARGET)?;
			Some((message, detail))
		}
		None => scope
			.timeline
			.messages
			.iter()
			.rev()
			.filter_map(|message| {
				let detail = details
					.items
					.iter()
					.find(|detail| detail.id == message.id)?;
				(canonical_id(&message.id)
					&& message.author.id == scope.own_id
					&& is_ordinary(detail.kind))
				.then_some((message, detail))
			})
			.next(),
	};
	let Some((message, detail)) = candidate else {
		return Err(MISSING_TARGET);
	};
	if !canonical_id(&message.id) || !canonical_id(&detail.id) {
		return Err(MISSING_TARGET);
	}
	if message.author.id != scope.own_id {
		return Err(FOREIGN_TARGET);
	}
	if !is_ordinary(detail.kind) {
		return Err(NON_ORDINARY_TARGET);
	}
	Ok(Target { message, detail })
}

fn attachments_are_confirmed_empty(target: &Target<'_>) -> bool {
	target.detail.attachments.is_empty()
		&& !target.detail.attachments_truncated
		&& target.message.attachment_count == 0
}

fn target_label(scope: &Scope<'_>, target: &Target<'_>) -> String {
	format!(
		"Target: loaded message {} in conversation {}.",
		target.message.id, scope.channel_id
	)
}

fn show(input: &AppInvocation) -> AppOutput {
	let scope = match scope(input) {
		Ok(scope) => scope,
		Err(message) => return error_output(message),
	};
	let target = match select_target(&scope, None) {
		Ok(target) => target,
		Err(message) => return error_output(message),
	};
	panel_output(
		vec![
			Element::Heading {
				text: "Own message sed".into(),
			},
			text(target_label(&scope, &target)),
			text(
				"Literal matching only. Escape /, *, + or ? with a backslash. Serein proposes one foreground edit or deletion; nothing is sent, scheduled or applied without the Apply confirmation.",
			),
			Element::TextInput {
				id: "target".into(),
				label: "Target loaded message ID".into(),
				value: target.message.id.clone(),
			},
			Element::TextInput {
				id: "match".into(),
				label: "Literal text to find".into(),
				value: String::new(),
			},
			Element::TextInput {
				id: "replace".into(),
				label: "Literal replacement".into(),
				value: String::new(),
			},
			Element::Checkbox {
				id: "global".into(),
				label: "Replace every match (global)".into(),
				checked: false,
			},
			Element::Button {
				id: "show".into(),
				label: "Refresh editor".into(),
			},
			Element::Button {
				id: "apply".into(),
				label: "Review replacement".into(),
			},
		],
		None,
	)
}

fn apply(input: &AppInvocation) -> AppOutput {
	let scope = match scope(input) {
		Ok(scope) => scope,
		Err(message) => return error_output(message),
	};
	let requested = input.invocation.value("target").unwrap_or_default();
	if requested.is_empty() {
		return no_effect("Choose a loaded own message to edit. Nothing was proposed.");
	}
	let target = match select_target(&scope, Some(requested)) {
		Ok(target) => target,
		Err(message) => return no_effect(message),
	};
	let match_raw = input.invocation.value("match").unwrap_or_default();
	if match_raw.is_empty() || match_raw.len() > MAX_MATCH_BYTES {
		return no_effect(
			"Enter literal match text of 1 to 256 UTF-8 bytes. Nothing was proposed.",
		);
	}
	let replace_raw = input.invocation.value("replace").unwrap_or_default();
	if replace_raw.len() > MAX_REPLACE_BYTES {
		return no_effect(
			"Replacement text must be at most 2,000 UTF-8 bytes. Nothing was proposed.",
		);
	}
	let match_text = unescape_literal(match_raw);
	if match_text.is_empty() || match_text.len() > MAX_MATCH_BYTES {
		return no_effect(
			"Enter literal match text of 1 to 256 UTF-8 bytes. Nothing was proposed.",
		);
	}
	let replace_text = unescape_literal(replace_raw);
	if replace_text.len() > MAX_REPLACE_BYTES {
		return no_effect(
			"Replacement text must be at most 2,000 UTF-8 bytes. Nothing was proposed.",
		);
	}
	let global = match input.invocation.value("global") {
		None | Some("false") => false,
		Some("true") => true,
		Some(_) => {
			return no_effect("Choose first-only or global replacement. Nothing was proposed.");
		}
	};
	if target.message.content.is_empty() {
		return no_effect("The target has no editable text. Nothing was proposed.");
	}
	let result = if global {
		target.message.content.replace(&match_text, &replace_text)
	} else {
		target
			.message
			.content
			.replacen(&match_text, &replace_text, 1)
	};
	if result == target.message.content {
		return no_effect("The literal text was not found, so nothing was proposed.");
	}
	if result.chars().count() > MAX_RESULT_CHARS || result.len() > MAX_RESULT_BYTES {
		return no_effect(
			"The replacement exceeds the 2,000-character or 8,000-byte message limit. Nothing was proposed.",
		);
	}
	if result.trim().is_empty() {
		if !attachments_are_confirmed_empty(&target) {
			return no_effect(
				"The replacement is empty, but the target has attachments or unverified attachment metadata. Nothing was deleted or edited.",
			);
		}
		return panel_output(
			vec![
				Element::Heading {
					text: "Own message sed".into(),
				},
				text(target_label(&scope, &target)),
				text(
					"The replacement is empty and the bounded metadata confirms no attachments. Apply will permanently delete this message. This cannot be undone.",
				),
				Element::Button {
					id: "show".into(),
					label: "Back to editor".into(),
				},
			],
			Some(HostEffect::AppAction {
				action: AppAction::DeleteMessage {
					channel_id: scope.channel_id.to_owned(),
					message_id: target.message.id.clone(),
				},
			}),
		);
	}
	if has_unsupported_control(&result) {
		return no_effect(
			"The result contains control characters that Serein cannot propose. Nothing was proposed.",
		);
	}
	panel_output(
		vec![
			Element::Heading {
				text: "Own message sed".into(),
			},
			text(target_label(&scope, &target)),
			text(
				"Apply will replace only this loaded message authored by the current account. The edit is a foreground proposal; nothing has been sent or scheduled.",
			),
			Element::Button {
				id: "show".into(),
				label: "Back to editor".into(),
			},
		],
		Some(HostEffect::AppAction {
			action: AppAction::EditMessage {
				channel_id: scope.channel_id.to_owned(),
				message_id: target.message.id.clone(),
				content: result,
			},
		}),
	)
}

fn handle(input: AppInvocation) -> AppOutput {
	// Panel actions have no event context. Ignoring event invocations prevents
	// background work or a second subsequence send from this plugin.
	if input.app_event.is_some() || input.message_event.is_some() {
		return AppOutput::default();
	}
	match input.invocation.action.as_str() {
		"show" => show(&input),
		"apply" => apply(&input),
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{
		Manifest, dispatch_typed,
		serde_json::{self, Value, json},
	};

	fn attachment(id: &str, filename: &str) -> Value {
		json!({
			"id": id,
			"filename": filename,
			"size": 12,
			"spoiler": false
		})
	}

	fn app_value(
		content: &str,
		author: &str,
		attachments: Vec<Value>,
		timeline_channel: &str,
		details_channel: &str,
	) -> Value {
		json!({
			"context": {
				"connected": true,
				"user": { "id": "1", "name": "Me" },
				"channel": { "id": "20", "name": "general", "kind": 0 }
			},
			"timeline": {
				"channel_id": timeline_channel,
				"truncated": false,
				"messages": [{
					"id": "200",
					"author": { "id": author, "name": "Author" },
					"content": content,
					"attachment_count": attachments.len(),
					"edited": false
				}]
			},
			"message_details": {
				"channel_id": details_channel,
				"truncated": false,
				"items": [{
					"id": "200",
					"kind": 0,
					"mention_ids": [],
					"mentions_truncated": false,
					"mention_everyone": false,
					"attachments": attachments,
					"attachments_truncated": false,
					"reactions_truncated": false
				}]
			}
		})
	}

	fn request(action: &str, target: &str, find: &str, replace: &str, global: bool) -> Value {
		json!({
			"action": action,
			"values": {
				"target": target,
				"match": find,
				"replace": replace,
				"global": if global { "true" } else { "false" }
			},
			"app": app_value("a*b a*b", "1", Vec::new(), "20", "20")
		})
	}

	fn run(value: Value) -> AppOutput {
		let bytes = serde_json::to_vec(&value).unwrap();
		let output = dispatch_typed(&bytes, handle).unwrap();
		serde_json::from_slice(&output).unwrap()
	}

	fn edited(output: &AppOutput) -> Option<(&str, &str)> {
		match output.effects.as_slice() {
			[
				HostEffect::AppAction {
					action:
						AppAction::EditMessage {
							channel_id,
							message_id,
							content,
						},
				},
			] => (channel_id == "20").then_some((message_id.as_str(), content.as_str())),
			_ => None,
		}
	}

	fn deletion(output: &AppOutput) -> Option<&str> {
		match output.effects.as_slice() {
			[
				HostEffect::AppAction {
					action: AppAction::DeleteMessage {
						channel_id,
						message_id,
					},
				},
			] if channel_id == "20" => Some(message_id),
			_ => None,
		}
	}

	#[test]
	fn literal_escapes_cover_slash_star_plus_and_question() {
		let mut value = request("apply", "200", r"a\*b", r"x\/y\+z\?", false);
		value["app"] = app_value("a*b", "1", Vec::new(), "20", "20");
		let output = run(value);
		assert_eq!(edited(&output), Some(("200", "x/y+z?")));

		// A backslash before any other character stays literal.
		let mut value = request("apply", "200", r"a\qb", r"\n", false);
		value["app"] = app_value(r"a\qb", "1", Vec::new(), "20", "20");
		let output = run(value);
		assert_eq!(edited(&output), Some(("200", r"\n")));

		// The first backslash is not consumed when it precedes another escape,
		// matching upstream's non-overlapping global escape removal.
		let mut value = request("apply", "200", r"\\*", r"\\?", false);
		value["app"] = app_value(r"\*", "1", Vec::new(), "20", "20");
		let output = run(value);
		assert_eq!(edited(&output), Some(("200", r"\?")));
	}

	#[test]
	fn first_and_global_replacements_are_distinct() {
		let mut value = request("apply", "200", "a", "b", false);
		value["app"] = app_value("a a a", "1", Vec::new(), "20", "20");
		assert_eq!(edited(&run(value)), Some(("200", "b a a")));

		let mut value = request("apply", "200", "a", "b", true);
		value["app"] = app_value("a a a", "1", Vec::new(), "20", "20");
		assert_eq!(edited(&run(value)), Some(("200", "b b b")));
	}

	#[test]
	fn unicode_literals_are_not_byte_or_char_split() {
		let mut value = request("apply", "200", "héllo 🦀", "salut 🦀", true);
		value["app"] = app_value("héllo 🦀 héllo 🦀", "1", Vec::new(), "20", "20");
		let output = run(value);
		assert_eq!(edited(&output), Some(("200", "salut 🦀 salut 🦀")));
	}

	#[test]
	fn stale_context_foreign_missing_and_tombstone_targets_have_no_effect() {
		for value in [
			{
				let mut value = request("apply", "200", "a", "b", false);
				value["app"] = app_value("a", "1", Vec::new(), "21", "20");
				value
			},
			{
				let mut value = request("apply", "200", "a", "b", false);
				value["app"] = app_value("a", "2", Vec::new(), "20", "20");
				value
			},
			{
				let mut value = request("apply", "0200", "a", "b", false);
				value["app"] = app_value("a", "1", Vec::new(), "20", "20");
				value
			},
			{
				let mut value = request("apply", "999", "a", "b", false);
				value["app"] = app_value("a", "1", Vec::new(), "20", "20");
				value
			},
			{
				// A timeline row without matching bounded details is treated as a
				// tombstone or stale metadata, never an editable target.
				let mut value = request("apply", "200", "a", "b", false);
				let mut app = app_value("a", "1", Vec::new(), "20", "20");
				app["message_details"]["items"] = json!([]);
				value["app"] = app;
				value
			},
		] {
			let output = run(value);
			assert!(output.effects.is_empty());
		}
	}

	#[test]
	fn empty_result_deletes_only_when_attachments_are_confirmed_empty() {
		let mut value = request("apply", "200", "abc", "  ", false);
		value["app"] = app_value("abc", "1", vec![attachment("9", "cat.png")], "20", "20");
		let output = run(value);
		assert!(output.effects.is_empty());
		assert!(deletion(&output).is_none());

		let mut value = request("apply", "200", "abc", "  ", false);
		value["app"] = app_value("abc", "1", Vec::new(), "20", "20");
		let output = run(value);
		assert_eq!(deletion(&output), Some("200"));
		assert_eq!(output.effects.len(), 1);
	}

	#[test]
	fn oversized_inputs_and_results_never_produce_a_proposal() {
		let long_match = "a".repeat(MAX_MATCH_BYTES + 1);
		let mut value = request("apply", "200", &long_match, "b", false);
		value["app"] = app_value("a", "1", Vec::new(), "20", "20");
		assert!(run(value).effects.is_empty());

		let long_replace = "b".repeat(MAX_REPLACE_BYTES + 1);
		let mut value = request("apply", "200", "a", &long_replace, false);
		value["app"] = app_value("a", "1", Vec::new(), "20", "20");
		assert!(run(value).effects.is_empty());

		let mut value = request("apply", "200", "a", "aa", true);
		value["app"] = app_value(&"a".repeat(MAX_RESULT_CHARS), "1", Vec::new(), "20", "20");
		assert!(run(value).effects.is_empty());
	}

	#[test]
	fn show_and_apply_panels_use_declared_button_actions() {
		let manifest: Manifest = serde_json::from_str(include_str!("../manifest.json")).unwrap();
		for action in ["show", "apply"] {
			assert!(manifest.actions.iter().any(|declared| {
				declared.id == action && declared.surface == serein_extension_sdk::Surface::Panel
			}));
			let value = if action == "show" {
				json!({
					"action": "show",
					"app": app_value("a*b a*b", "1", Vec::new(), "20", "20")
				})
			} else {
				request("apply", "200", r"a\*b", "c", true)
			};
			let output = run(value);
			let mut buttons = Vec::new();
			collect_buttons(&output.output.panel, &mut buttons);
			assert!(!buttons.is_empty());
			for id in buttons {
				assert!(manifest.actions.iter().any(|declared| {
					declared.id == id && declared.surface == serein_extension_sdk::Surface::Panel
				}));
			}
		}
	}

	fn collect_buttons(elements: &[Element], buttons: &mut Vec<String>) {
		for element in elements {
			match element {
				Element::Button { id, .. } => buttons.push(id.clone()),
				Element::Row { children } => collect_buttons(children, buttons),
				_ => {}
			}
		}
	}
}
