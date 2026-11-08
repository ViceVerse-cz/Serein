/*
 * Vencord/Equicord, a Discord client mod
 * Copyright (c) 2023-2026 Vendicated and contributors
 *
 * Upstream sources at Equicord commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e:
 * src/equicordplugins/pendingFriendRequest/index.tsx,
 * src/equicordplugins/normalizeMessageLinks/index.ts and
 * src/equicordplugins/repeatMessages/index.tsx.
 *
 * Changed November 8, 2026: no JavaScript injection, automatic send or
 * network access. Cancellation is one Apply-reviewed ResolveFriendRequest
 * with accept=false only for a currently loaded outgoing request. Message
 * link normalization only rewrites the authority of exact canary/ptb HTTPS
 * Discord message links and proposes a clipboard copy. Repeat proposes one
 * Apply-reviewed send, optionally as a reply with reply mention disabled,
 * with user/role and @everyone/@here mentions neutralized.
 *
 * This program is free software: you can redistribute it and/or modify it
 * under the terms of the GNU General Public License version 3 or (at your
 * option) any later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppAction, AppOutput, Element, HostEffect, MessageActionInvocation, Output, RelationshipKind,
	RelationshipsSnapshot, SelectedMessageRef,
};

/// Only this many loaded outgoing requests are listed and selectable.
const MAX_OUTGOING_ROWS: usize = 20;
/// A single response may normalize at most this many links.
const MAX_LINKS: usize = 20;
/// The copied normalized text stays within the clipboard contract.
const MAX_NORMALIZED_BYTES: usize = 2048;
/// Repeat uses the ordinary Discord message limit.
const MAX_REPEAT_CHARS: usize = 2000;
/// Aligned with the ordinary composer/message byte ceiling.
const MAX_REPEAT_BYTES: usize = 8000;
const MAX_ID_BYTES: usize = 20;

const MISSING_SELECTION: &str =
	"The host did not provide the selected message IDs, so nothing was proposed.";
const STALE_SELECTION: &str =
	"The selected message no longer matches its loaded context. Reopen the message and try again.";
const NO_LINKS: &str = "No exact canary.discord.com or ptb.discord.com HTTPS message links were found. Nothing was copied.";
const TOO_MANY_LINKS: &str = "More than 20 eligible message links were found. Nothing was copied.";
const LONG_LINKS: &str = "The normalized text exceeds 2,048 UTF-8 bytes. Nothing was copied.";
const NO_REPEAT: &str =
	"The selected message has no text to repeat. Attachments and stickers are never fabricated.";

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
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
		"Unnamed user".into()
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

fn relationships(input: &MessageActionInvocation) -> Result<&RelationshipsSnapshot, &'static str> {
	let app = input
		.invocation
		.app
		.as_ref()
		.ok_or("The host did not provide relationship data, so nothing was proposed.")?;
	let snapshot = app.relationships.as_ref().ok_or(
		"The relationships grant is unavailable in this snapshot, so nothing was proposed.",
	)?;
	if !snapshot.requests_known {
		return Err(
			"The host has not loaded the friend-request list, so no cancellation can be proposed.",
		);
	}
	Ok(snapshot)
}

fn outgoing_requests(
	snapshot: &RelationshipsSnapshot,
) -> Vec<&serein_extension_sdk::RelationshipSnapshot> {
	snapshot
		.items
		.iter()
		.filter(|item| {
			item.kind == RelationshipKind::OutgoingRequest && valid_snowflake(&item.user.id)
		})
		.collect()
}

fn open_requests(input: &MessageActionInvocation) -> AppOutput {
	let snapshot = match relationships(input) {
		Ok(snapshot) => snapshot,
		Err(message) => return error_output("Outgoing friend requests", message),
	};
	let outgoing = outgoing_requests(snapshot);
	let mut panel = vec![heading("Outgoing friend requests")];
	if outgoing.is_empty() {
		panel.push(text(
			"No current outgoing friend requests are loaded. Incoming requests, friends and other relationships are not shown.",
		));
		return panel_output(panel, None);
	}
	panel.push(text(
		"Only current outgoing friend requests are listed. Incoming requests, friends, blocked and ignored relationships are never selectable.",
	));
	if snapshot.truncated {
		panel.push(text(
			"The relationship snapshot is partial; only the loaded outgoing requests below are available.",
		));
	}
	let shown = outgoing.len().min(MAX_OUTGOING_ROWS);
	for item in &outgoing[..shown] {
		panel.push(Element::Row {
			children: vec![text(format!(
				"{} — {}",
				sanitize_name(&item.user.name),
				item.user.id
			))],
		});
	}
	if outgoing.len() > shown {
		panel.push(text(format!(
			"Showing {shown} of {} loaded outgoing requests.",
			outgoing.len()
		)));
	}
	let options: Vec<String> = outgoing[..shown]
		.iter()
		.map(|item| item.user.id.clone())
		.collect();
	let value = options[0].clone();
	panel.push(Element::Select {
		id: "user-id".into(),
		label: "Outgoing request user ID".into(),
		options,
		value,
	});
	panel.push(Element::Button {
		id: "cancel".into(),
		label: "Review cancellation".into(),
	});
	panel.push(text(
		"Apply rechecks the current relationship. If it is no longer an outgoing request, nothing is sent.",
	));
	panel_output(panel, None)
}

fn cancel_request(input: &MessageActionInvocation) -> AppOutput {
	let user_id = input
		.invocation
		.invocation
		.value("user-id")
		.unwrap_or_default();
	if !valid_snowflake(user_id) {
		return error_output(
			"Cancel outgoing friend request",
			"Choose a valid loaded outgoing request user ID. Nothing was proposed.",
		);
	}
	let snapshot = match relationships(input) {
		Ok(snapshot) => snapshot,
		Err(message) => return error_output("Cancel outgoing friend request", message),
	};
	let mut matches = snapshot.items.iter().filter(|item| item.user.id == user_id);
	let Some(item) = matches.next() else {
		return error_output(
			"Cancel outgoing friend request",
			"That user is not in the current loaded relationship snapshot. Nothing was proposed.",
		);
	};
	if matches.next().is_some() {
		return error_output(
			"Cancel outgoing friend request",
			"The relationship snapshot is ambiguous for that user. Nothing was proposed.",
		);
	}
	if item.kind != RelationshipKind::OutgoingRequest {
		return error_output(
			"Cancel outgoing friend request",
			"Only a current outgoing friend request can be cancelled. Incoming requests, friends and other users are never targeted.",
		);
	}
	if input
		.invocation
		.app
		.as_ref()
		.and_then(|app| app.context.as_ref())
		.and_then(|context| context.user.as_ref())
		.is_some_and(|user| user.id == user_id)
	{
		return error_output(
			"Cancel outgoing friend request",
			"The signed-in account is never a cancellation target.",
		);
	}
	let name = sanitize_name(&item.user.name);
	let panel = vec![
		heading("Cancel outgoing friend request"),
		text(format!(
			"Cancel the pending outgoing friend request to {name} ({user_id})."
		)),
		text(
			"Apply uses the typed ResolveFriendRequest operation with accept=false, which cancels an outgoing request. Serein rechecks the current kind at Apply; a stale or changed relationship fails instead of targeting another user.",
		),
	];
	panel_output(
		panel,
		Some(HostEffect::AppAction {
			action: AppAction::ResolveFriendRequest {
				user_id: user_id.to_owned(),
				accept: false,
			},
		}),
	)
}

/// Rewrite only the authority of exact `https://canary.discord.com` and
/// `https://ptb.discord.com` links. Every other byte, including path, query,
/// fragment and percent escapes, is copied unchanged.
fn normalize_message_links(source: &str) -> Result<(String, usize), &'static str> {
	let bytes = source.as_bytes();
	let mut output = String::with_capacity(source.len());
	let mut last = 0;
	let mut index = 0;
	let mut count = 0;
	while index + 8 <= bytes.len() {
		if !bytes[index..index + 8].eq_ignore_ascii_case(b"https://") {
			index += 1;
			continue;
		}
		if index > 0 {
			let previous = bytes[index - 1];
			if previous.is_ascii_alphanumeric() || previous == b'_' {
				index += 1;
				continue;
			}
		}
		let authority_start = index + 8;
		let mut authority_end = authority_start;
		while authority_end < bytes.len() {
			let byte = bytes[authority_end];
			if matches!(byte, b'/' | b'?' | b'#')
				|| byte.is_ascii_whitespace()
				|| byte.is_ascii_control()
			{
				break;
			}
			authority_end += 1;
		}
		let authority = &source[authority_start..authority_end];
		if !authority.eq_ignore_ascii_case("canary.discord.com")
			&& !authority.eq_ignore_ascii_case("ptb.discord.com")
		{
			index = authority_start;
			continue;
		}
		count += 1;
		if count > MAX_LINKS {
			return Err(TOO_MANY_LINKS);
		}
		output.push_str(&source[last..authority_start]);
		output.push_str("discord.com");
		last = authority_end;
		index = authority_end;
	}
	if count == 0 {
		return Err(NO_LINKS);
	}
	output.push_str(&source[last..]);
	if output.len() > MAX_NORMALIZED_BYTES {
		return Err(LONG_LINKS);
	}
	Ok((output, count))
}

fn normalize_links(input: &MessageActionInvocation) -> AppOutput {
	let reference = match selected_reference(input) {
		Ok(reference) => reference,
		Err(message) => return error_output("Normalize Discord message links", message),
	};
	let source = input
		.invocation
		.invocation
		.selected_message
		.as_deref()
		.unwrap_or_default();
	let (normalized, count) = match normalize_message_links(source) {
		Ok(result) => result,
		Err(message) => return error_output("Normalize Discord message links", message),
	};
	let panel = vec![
		heading("Normalize Discord message links"),
		text(format!(
			"Selected message {}. Found {count} eligible canary/ptb message link(s).",
			reference.message_id
		)),
		text(
			"Apply copies this exact normalized text. Only the host is rewritten; path, query, fragment and all other text stay byte-for-byte unchanged. Nothing is fetched, opened or sent.",
		),
		text(normalized.clone()),
	];
	panel_output(panel, Some(HostEffect::CopyText { text: normalized }))
}

/// U+ZERO WIDTH SPACE inserted after `@` to stop Discord from resolving a
/// user/role mention or an `@everyone`/`@here` ping while keeping the text
/// visually close to the original.
const MENTION_BREAK: char = '\u{200B}';

fn mention_boundary_before(source: &str, index: usize) -> bool {
	if index == 0 {
		return true;
	}
	let previous = source[..index].chars().next_back().unwrap_or(' ');
	!previous.is_alphanumeric() && previous != '_' && previous != '\\'
}

fn mention_boundary_after(source: &str, index: usize) -> bool {
	source[index..]
		.chars()
		.next()
		.is_none_or(|next| !next.is_alphanumeric() && next != '_')
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
	value.len() >= prefix.len()
		&& value.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Neutralize Discord mention syntax without changing ordinary text. Fenced
/// code, inline code and raw HTTP(S) URL spans are copied unchanged.
fn neutralize_mentions(source: &str) -> String {
	let mut output = String::with_capacity(source.len());
	let mut index = 0;
	while index < source.len() {
		let rest = &source[index..];
		if rest.starts_with("```") || rest.starts_with("~~~") {
			let fence = &rest[..3];
			output.push_str(fence);
			index += 3;
			if let Some(end) = source[index..].find(fence) {
				let end = index + end + 3;
				output.push_str(&source[index..end]);
				index = end;
			} else {
				output.push_str(&source[index..]);
				break;
			}
			continue;
		}
		if rest.starts_with('`') {
			output.push('`');
			index += 1;
			if let Some(end) = source[index..].find('`') {
				let end = index + end + 1;
				output.push_str(&source[index..end]);
				index = end;
			} else {
				output.push_str(&source[index..]);
				break;
			}
			continue;
		}
		if starts_with_ignore_ascii_case(rest, "http://")
			|| starts_with_ignore_ascii_case(rest, "https://")
		{
			let start = index;
			while index < source.len() {
				let character = source[index..].chars().next().unwrap();
				if character.is_whitespace()
					|| character.is_control()
					|| matches!(character, '<' | '>' | '"')
				{
					break;
				}
				index += character.len_utf8();
			}
			output.push_str(&source[start..index]);
			continue;
		}
		if rest.starts_with("<@")
			&& let Some(end) = rest.find('>')
		{
			let token = &rest[2..end];
			let digits = token.trim_start_matches(['!', '&']);
			if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
				output.push_str("<@");
				output.push(MENTION_BREAK);
				output.push_str(token);
				output.push('>');
				index += end + 1;
				continue;
			}
		}
		if rest.starts_with('@') && mention_boundary_before(source, index) {
			let mut replaced = false;
			for target in ["@everyone", "@here"] {
				if starts_with_ignore_ascii_case(rest, target)
					&& mention_boundary_after(source, index + target.len())
				{
					output.push('@');
					output.push(MENTION_BREAK);
					output.push_str(&rest[1..target.len()]);
					index += target.len();
					replaced = true;
					break;
				}
			}
			if replaced {
				continue;
			}
		}
		let character = rest.chars().next().unwrap();
		output.push(character);
		index += character.len_utf8();
	}
	output
}

fn push_payload_text(panel: &mut Vec<Element>, payload: &str) {
	let mut start = 0;
	while start < payload.len() {
		let mut end = (start + 4000).min(payload.len());
		while end > start && !payload.is_char_boundary(end) {
			end -= 1;
		}
		if end == start {
			end = payload[start..]
				.char_indices()
				.nth(1)
				.map_or(payload.len(), |(offset, _)| start + offset);
		}
		panel.push(text(payload[start..end].to_owned()));
		start = end;
	}
}

fn repeat_message(input: &MessageActionInvocation, reply: bool) -> AppOutput {
	let reference = match selected_reference(input) {
		Ok(reference) => reference,
		Err(message) => return error_output("Repeat message", message),
	};
	let source = input
		.invocation
		.invocation
		.selected_message
		.as_deref()
		.unwrap_or_default();
	if source.trim().is_empty() {
		return error_output("Repeat message", NO_REPEAT);
	}
	let sanitized = neutralize_mentions(source);
	if sanitized.trim().is_empty() {
		return error_output("Repeat message", NO_REPEAT);
	}
	if sanitized.chars().count() > MAX_REPEAT_CHARS || sanitized.len() > MAX_REPEAT_BYTES {
		return error_output(
			"Repeat message",
			"The sanitized text exceeds the 2,000-character or 8,000-byte message ceiling. Nothing was proposed.",
		);
	}
	if sanitized
		.chars()
		.any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
	{
		return error_output(
			"Repeat message",
			"The selected text contains unsupported control characters. Nothing was proposed.",
		);
	}
	let destination = if reply {
		format!(
			"Apply sends this reviewed text as a reply to message {} in channel {}. Reply mention is disabled; no attachment or sticker is added.",
			reference.message_id, reference.channel_id
		)
	} else {
		format!(
			"Apply sends this reviewed text to channel {}. No attachment, sticker or reply target is added.",
			reference.channel_id
		)
	};
	let mut panel = vec![
		heading(if reply {
			"Repeat message and reply"
		} else {
			"Repeat message"
		}),
		text(destination),
		text(
			"Mentions are neutralized with U+200B ZERO WIDTH SPACE before sending so this repeat cannot notify users, roles, @everyone or @here merely because the old message did. The payload shown below is the exact text that would be sent, not a byte-identical copy of the original.",
		),
		text("Exact sanitized payload:".to_owned()),
	];
	push_payload_text(&mut panel, &sanitized);
	let action = if reply {
		AppAction::SendReply {
			channel_id: reference.channel_id.clone(),
			message_id: reference.message_id.clone(),
			content: sanitized,
			mention: false,
		}
	} else {
		AppAction::SendMessage {
			channel_id: reference.channel_id.clone(),
			content: sanitized,
		}
	};
	panel_output(panel, Some(HostEffect::AppAction { action }))
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	match input.invocation.invocation.action.as_str() {
		"open" => open_requests(&input),
		"cancel" => cancel_request(&input),
		"normalize-links" => normalize_links(&input),
		"repeat" => repeat_message(&input, false),
		"repeat-reply" => repeat_message(&input, true),
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

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

	fn message(action: &str, selected: &str) -> serde_json::Value {
		serde_json::json!({
			"action": action,
			"selected_message": selected,
			"selected_message_ref": reference()
		})
	}

	fn relationships(requests_known: bool) -> serde_json::Value {
		serde_json::json!({
			"relationships": {
				"items": [
					{"user": {"id": "10", "name": "OutgoingName"}, "kind": "outgoing_request"},
					{"user": {"id": "11", "name": "IncomingName"}, "kind": "incoming_request"},
					{"user": {"id": "12", "name": "FriendName"}, "kind": "friend"}
				],
				"truncated": false,
				"friends_known": true,
				"requests_known": requests_known,
				"restricted_known": true
			}
		})
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

	fn select_options(output: &AppOutput) -> Vec<String> {
		output
			.output
			.panel
			.iter()
			.find_map(|element| match element {
				Element::Select { options, .. } => Some(options.clone()),
				_ => None,
			})
			.unwrap_or_default()
	}

	#[test]
	fn open_lists_only_loaded_outgoing_requests() {
		let (output, _) = run(serde_json::json!({
			"action": "open",
			"app": relationships(true)
		}));
		assert!(output.effects.is_empty());
		assert_eq!(select_options(&output), vec!["10"]);
		let text = panel_text(&output);
		assert!(text.contains("OutgoingName"));
		assert!(!text.contains("IncomingName"));
		assert!(!text.contains("FriendName"));
	}

	#[test]
	fn cancellation_rejects_wrong_kind_stale_and_unknown_scope() {
		for (user, expected) in [
			("11", "Only a current outgoing"),
			("12", "Only a current outgoing"),
			("99", "not in the current loaded"),
		] {
			let (output, _) = run(serde_json::json!({
				"action": "cancel",
				"values": {"user-id": user},
				"app": relationships(true)
			}));
			assert!(output.effects.is_empty(), "{user}");
			assert!(panel_text(&output).contains(expected), "{user}");
		}
		let (unknown, _) = run(serde_json::json!({
			"action": "cancel",
			"values": {"user-id": "10"},
			"app": relationships(false)
		}));
		assert!(unknown.effects.is_empty());
		let (missing, _) = run(serde_json::json!({
			"action": "cancel",
			"values": {"user-id": "10"}
		}));
		assert!(missing.effects.is_empty());
	}

	#[test]
	fn cancellation_proposes_exactly_one_typed_outgoing_cancel() {
		let (output, wire) = run(serde_json::json!({
			"action": "cancel",
			"values": {"user-id": "10"},
			"app": relationships(true)
		}));
		assert_eq!(output.effects.len(), 1);
		let HostEffect::AppAction {
			action: AppAction::ResolveFriendRequest { user_id, accept },
		} = &output.effects[0]
		else {
			panic!("expected resolve_friend_request");
		};
		assert_eq!(user_id, "10");
		assert!(!accept);
		assert_eq!(
			wire["effects"][0]["action"]["type"],
			"resolve_friend_request"
		);
	}

	#[test]
	fn link_normalization_preserves_every_other_byte() {
		let source = "before 😀 https://canary.discord.com/channels/1/2/3?x=%2F#frag \
			and https://ptb.discord.com/channels/4/5/6 after";
		let (normalized, count) = normalize_message_links(source).unwrap();
		assert_eq!(count, 2);
		assert_eq!(
			normalized,
			"before 😀 https://discord.com/channels/1/2/3?x=%2F#frag \
			 and https://discord.com/channels/4/5/6 after"
		);
	}

	#[test]
	fn link_normalization_rejects_lookalikes_and_non_https() {
		for source in [
			"https://discord.com/channels/1/2/3",
			"http://canary.discord.com/channels/1/2/3",
			"https://canary.discord.com.evil/channels/1/2/3",
			"https://user@canary.discord.com/channels/1/2/3",
			"https://canary.discord.com:443/channels/1/2/3",
			"xhttps://canary.discord.com/channels/1/2/3",
		] {
			assert!(normalize_message_links(source).is_err(), "{source}");
		}
	}

	#[test]
	fn link_normalization_enforces_link_and_byte_limits() {
		let twenty = (0..20)
			.map(|index| format!("https://canary.discord.com/channels/{index}"))
			.collect::<Vec<_>>()
			.join(" ");
		assert_eq!(normalize_message_links(&twenty).unwrap().1, 20);
		let twenty_one = format!("{twenty} https://ptb.discord.com/channels/21");
		assert_eq!(normalize_message_links(&twenty_one), Err(TOO_MANY_LINKS));
		let long = (0..20)
			.map(|index| {
				format!(
					"https://canary.discord.com/channels/{index}/{}",
					"x".repeat(100)
				)
			})
			.collect::<Vec<_>>()
			.join(" ");
		assert_eq!(normalize_message_links(&long), Err(LONG_LINKS));
	}

	#[test]
	fn link_action_copies_exact_text_and_requires_selection() {
		let (output, _) = run(message(
			"normalize-links",
			"see https://canary.discord.com/channels/1/2/3?q=%2F#x",
		));
		assert_eq!(output.effects.len(), 1);
		let HostEffect::CopyText { text } = &output.effects[0] else {
			panic!("expected copy_text");
		};
		assert_eq!(text, "see https://discord.com/channels/1/2/3?q=%2F#x");
		let (missing, _) = run(serde_json::json!({
			"action": "normalize-links",
			"selected_message": "https://canary.discord.com/channels/1/2/3"
		}));
		assert!(missing.effects.is_empty());
	}

	#[test]
	fn mention_neutralization_targets_only_mentions() {
		let source = "hi <@123> <@!456> <@&789> @everyone @here";
		assert_eq!(
			neutralize_mentions(source),
			"hi <@\u{200B}123> <@\u{200B}!456> <@\u{200B}&789> @\u{200B}everyone @\u{200B}here"
		);
		for unchanged in [
			"https://example.com/@everyone",
			"`@everyone <@123>`",
			"```\n@here <@&1>\n```",
			"foo@everyone",
			"\\@everyone",
		] {
			assert_eq!(neutralize_mentions(unchanged), unchanged);
		}
	}

	#[test]
	fn repeat_proposes_one_exact_sanitized_send_without_alternate_output() {
		let source = "hello 😀 <@123> @everyone https://example.com/@here\n`@here`";
		let (output, wire) = run(message("repeat", source));
		let expected =
			"hello 😀 <@\u{200B}123> @\u{200B}everyone https://example.com/@here\n`@here`";
		assert_eq!(output.effects.len(), 1);
		let HostEffect::AppAction {
			action: AppAction::SendMessage {
				channel_id,
				content,
			},
		} = &output.effects[0]
		else {
			panic!("expected send_message");
		};
		assert_eq!(channel_id, "100");
		assert_eq!(content, expected);
		assert!(output.output.replacement.is_none());
		assert_eq!(wire["effects"][0]["action"]["type"], "send_message");
		assert_eq!(wire["effects"][0]["action"]["channel_id"], "100");
		assert_eq!(wire["effects"][0]["action"]["content"], expected);
		assert!(wire.get("replacement").is_none_or(|value| value.is_null()));
		let wire = wire.to_string();
		assert!(!wire.contains("send_reply"));
		assert!(!wire.contains("send_sticker"));
		assert!(!wire.contains("message_reference"));
	}

	#[test]
	fn repeat_reply_proposes_one_typed_reply_with_mention_disabled() {
		let source = "hello <@123> @everyone";
		let (output, wire) = run(message("repeat-reply", source));
		let expected = "hello <@\u{200B}123> @\u{200B}everyone";
		assert_eq!(output.effects.len(), 1);
		let HostEffect::AppAction {
			action:
				AppAction::SendReply {
					channel_id,
					message_id,
					content,
					mention,
				},
		} = &output.effects[0]
		else {
			panic!("expected send_reply");
		};
		assert_eq!(channel_id, "100");
		assert_eq!(message_id, "200");
		assert_eq!(content, expected);
		assert!(!mention);
		assert_eq!(wire["effects"][0]["action"]["type"], "send_reply");
		assert_eq!(wire["effects"][0]["action"]["mention"], false);
		assert!(!wire.to_string().contains("send_message"));
		let (missing, _) = run(serde_json::json!({
			"action": "repeat-reply",
			"selected_message": "hello"
		}));
		assert!(missing.effects.is_empty());
	}

	#[test]
	fn repeat_missing_empty_oversized_or_invalid_is_inactive() {
		let (missing, _) = run(serde_json::json!({
			"action": "repeat",
			"selected_message": "hello"
		}));
		assert!(missing.effects.is_empty());
		assert!(missing.output.replacement.is_none());
		let (empty, _) = run(message("repeat", " \n\t "));
		assert!(empty.effects.is_empty());
		assert!(empty.output.replacement.is_none());
		let exact_limit = format!("@everyone {}", "x".repeat(MAX_REPEAT_CHARS - 10));
		assert_eq!(exact_limit.chars().count(), MAX_REPEAT_CHARS);
		let (over_after_sanitize, _) = run(message("repeat", &exact_limit));
		assert!(over_after_sanitize.effects.is_empty());
		assert!(over_after_sanitize.output.replacement.is_none());
		let (oversized, _) = run(message("repeat", &"x".repeat(MAX_REPEAT_CHARS + 1)));
		assert!(oversized.effects.is_empty());
		assert!(oversized.output.replacement.is_none());
		let (control, _) = run(message("repeat", "hello\u{0}world"));
		assert!(control.effects.is_empty());
		assert!(control.output.replacement.is_none());
	}
}
