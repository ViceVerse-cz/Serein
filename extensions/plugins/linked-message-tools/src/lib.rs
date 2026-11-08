/*
 * Vencord/Equicord, a Discord client mod
 * Copyright (c) 2022 Vendicated and contributors
 *
 * Upstream source at Equicord commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e:
 * src/plugins/messageLinkEmbeds/index.tsx
 *
 * Changed November 8, 2026: this port never fetches linked messages, never
 * injects a React accessory and never renders media. It scans only the selected
 * message's text for exact Discord message links, previews messages already in
 * the granted loaded timeline, and proposes one navigation jump after Apply.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppOutput, Element, HostEffect, MessageActionInvocation, Output, TimelineSnapshot,
};
use std::collections::BTreeSet;

/// A message may contain many links; only this many distinct targets are reviewed.
const MAX_LINKS: usize = 5;
/// The selected message is already host-bounded. Refuse anything unexpectedly large.
const MAX_SCAN_BYTES: usize = 8 * 1024;
/// Excerpts stay small enough for the panel and the select option label.
const MAX_EXCERPT_BYTES: usize = 160;
const MAX_OPTION_BYTES: usize = 128;
const MAX_ID_BYTES: usize = 20;

const MISSING_MESSAGE: &str =
	"The host did not provide the selected message text, so no link can be inspected.";
const MISSING_IDS: &str =
	"The host did not provide the selected message IDs, so no loaded message can be matched.";
const NO_LINKS: &str = "No exact Discord message links were found in this message.";
const TOO_LARGE: &str =
	"The selected message is larger than this tool's 8 KiB inspection bound. Nothing was read.";

#[derive(Clone, Debug, PartialEq, Eq)]
struct MessageLink {
	channel_id: String,
	message_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FoundLinks {
	links: Vec<MessageLink>,
	truncated: bool,
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

fn panel_output(elements: Vec<Element>) -> AppOutput {
	AppOutput {
		output: Output {
			panel: elements,
			..Default::default()
		},
		effects: Vec::new(),
	}
}

fn error_output(message: &str) -> AppOutput {
	panel_output(vec![heading("Linked messages"), text(message)])
}

fn is_snowflake(value: &str) -> bool {
	let bytes = value.as_bytes();
	!bytes.is_empty()
		&& bytes.len() <= MAX_ID_BYTES
		&& bytes[0] != b'0'
		&& bytes.iter().all(u8::is_ascii_digit)
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

/// Upstream only recognizes 17-20 digit Discord IDs in message links.
fn is_link_id(value: &str) -> bool {
	(17..=MAX_ID_BYTES).contains(&value.len()) && is_snowflake(value)
}

fn truncate(value: &str, max_bytes: usize) -> String {
	if value.len() <= max_bytes {
		return value.to_owned();
	}
	let mut end = max_bytes;
	while !value.is_char_boundary(end) {
		end -= 1;
	}
	value[..end].to_owned()
}

fn sanitize(value: &str, max_bytes: usize) -> String {
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
	truncate(cleaned.trim(), max_bytes)
}

fn excerpt(value: &str) -> String {
	let cleaned = sanitize(value, MAX_EXCERPT_BYTES);
	if cleaned.is_empty() {
		"(no text)".into()
	} else {
		cleaned
	}
}

/// Walk back to the token start and forward to its first delimiter. A token
/// wrapped in angle brackets is Discord's suppressed-link form and is skipped.
fn token_bounds(text: &str, position: usize) -> (&str, bool) {
	let bytes = text.as_bytes();
	let is_delimiter = |byte: u8| {
		byte.is_ascii_whitespace()
			|| matches!(
				byte,
				b'<' | b'>' | b'"' | b'\'' | b'`' | b'(' | b')' | b'[' | b']' | b'{' | b'}'
			)
	};
	let mut start = position;
	while start > 0 && !is_delimiter(bytes[start - 1]) {
		start -= 1;
	}
	let suppressed = start > 0 && bytes[start - 1] == b'<';
	let mut end = position;
	while end < bytes.len() && !is_delimiter(bytes[end]) {
		end += 1;
	}
	(&text[start..end], suppressed)
}

fn valid_authority(authority: &str) -> bool {
	let lower = authority.to_ascii_lowercase();
	if lower == "discord.com" || lower == "discordapp.com" {
		return true;
	}
	let Some((subdomain, host)) = lower.split_once('.') else {
		return false;
	};
	!subdomain.is_empty()
		&& subdomain
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
		&& (host == "discord.com" || host == "discordapp.com")
}

fn parse_link(token: &str) -> Option<MessageLink> {
	let token = token.trim_end_matches(['.', ',', ';', ':', '!', '?']);
	let rest = token
		.strip_prefix("https://")
		.or_else(|| token.strip_prefix("http://"))?;
	let mut segments = rest.split('/');
	let authority = segments.next()?;
	if !valid_authority(authority) || segments.next()? != "channels" {
		return None;
	}
	let guild = segments.next()?;
	let channel = segments.next()?;
	let message = segments.next()?;
	if guild != "@me" && !is_link_id(guild) {
		return None;
	}
	if !is_link_id(channel) || !is_link_id(message) {
		return None;
	}
	Some(MessageLink {
		channel_id: channel.into(),
		message_id: message.into(),
	})
}

fn extract_links(value: &str) -> FoundLinks {
	if value.len() > MAX_SCAN_BYTES {
		return FoundLinks {
			links: Vec::new(),
			truncated: true,
		};
	}
	let mut links = Vec::new();
	let mut seen = BTreeSet::new();
	let mut truncated = false;
	let mut search = 0;
	while let Some(offset) = value[search..].find("discord") {
		let position = search + offset;
		let (token, suppressed) = token_bounds(value, position);
		if !suppressed && let Some(link) = parse_link(token) {
			let key = (link.channel_id.clone(), link.message_id.clone());
			if seen.insert(key) {
				if links.len() == MAX_LINKS {
					truncated = true;
				} else {
					links.push(link);
				}
			}
		}
		search = position + "discord".len();
	}
	FoundLinks { links, truncated }
}

fn loaded_timeline<'a>(
	input: &'a MessageActionInvocation,
	channel_id: &str,
) -> Option<&'a TimelineSnapshot> {
	input
		.invocation
		.app
		.as_ref()?
		.timeline
		.as_ref()
		.filter(|timeline| timeline.channel_id == channel_id)
}

fn option_for(link: &MessageLink, message: &serein_extension_sdk::MessageSnapshot) -> String {
	let prefix = format!("{}:{} | ", link.channel_id, link.message_id);
	let remaining = MAX_OPTION_BYTES.saturating_sub(prefix.len());
	let author = sanitize(&message.author.name, 32);
	let body = excerpt(&message.content);
	let label = format!("{prefix}{author}: {}", truncate(&body, remaining));
	truncate(&label, MAX_OPTION_BYTES)
}

fn preview(input: &MessageActionInvocation) -> AppOutput {
	let Some(reference) = input.selected_message_ref.as_ref() else {
		return error_output(MISSING_IDS);
	};
	if !is_snowflake(&reference.channel_id) || !is_snowflake(&reference.message_id) {
		return error_output(MISSING_IDS);
	}
	let Some(value) = input.invocation.invocation.selected_message.as_deref() else {
		return error_output(MISSING_MESSAGE);
	};
	let found = extract_links(value);
	if found.links.is_empty() {
		return error_output(if found.truncated { TOO_LARGE } else { NO_LINKS });
	}
	let timeline = loaded_timeline(input, &reference.channel_id);
	let mut elements = vec![
		heading("Linked messages"),
		text(format!(
			"Found {} distinct Discord message link{}. Only messages already loaded in this chat are previewed; no history is fetched.",
			found.links.len(),
			if found.links.len() == 1 { "" } else { "s" }
		)),
	];
	if found.truncated {
		elements.push(text(format!(
			"Only the first {MAX_LINKS} links are considered. The message may have been truncated by this tool's bound."
		)));
	}
	let mut options = Vec::new();
	for link in &found.links {
		match timeline
			.filter(|timeline| timeline.channel_id == link.channel_id)
			.and_then(|timeline| {
				timeline
					.messages
					.iter()
					.find(|message| message.id == link.message_id)
			}) {
			Some(message) => {
				elements.push(Element::Separator);
				elements.push(text(format!(
					"{} in channel {}: {}",
					sanitize(&message.author.name, 80),
					link.channel_id,
					excerpt(&message.content)
				)));
				if message.edited {
					elements.push(text("This loaded message was edited."));
				}
				if message.attachment_count > 0 {
					elements.push(text(format!(
						"This loaded message also has {} attachment(s); only its text is shown.",
						message.attachment_count
					)));
				}
				options.push(option_for(link, message));
			}
			None => {
				elements.push(Element::Separator);
				elements.push(text(format!(
					"Message {} in channel {} is not in the loaded window. No content was fetched or shown.",
					link.message_id, link.channel_id
				)));
			}
		}
	}
	if !options.is_empty() {
		elements.push(Element::Select {
			id: "target".into(),
			label: "Loaded linked message".into(),
			options: options.clone(),
			value: options[0].clone(),
		});
		elements.push(Element::Button {
			id: "jump".into(),
			label: "Open selected linked message".into(),
		});
	}
	panel_output(elements)
}

fn target_value(input: &MessageActionInvocation) -> Option<(&str, &str)> {
	let value = input.invocation.invocation.value("target")?;
	let target = value.split('|').next()?.trim();
	let (channel_id, message_id) = target.split_once(':')?;
	Some((channel_id.trim(), message_id.trim()))
}

fn jump(input: &MessageActionInvocation) -> AppOutput {
	let Some((channel_id, message_id)) = target_value(input) else {
		return error_output("The selected linked message is missing from this panel.");
	};
	if !is_snowflake(channel_id) || !is_snowflake(message_id) {
		return error_output("The selected linked message has invalid IDs; nothing was proposed.");
	}
	let Some(timeline) = loaded_timeline(input, channel_id) else {
		return error_output(
			"The target chat is not the loaded conversation, so no navigation was proposed.",
		);
	};
	if !timeline
		.messages
		.iter()
		.any(|message| message.id == message_id)
	{
		return error_output(
			"The target message is no longer in the loaded window. Reopen the message and try again.",
		);
	}
	AppOutput {
		output: Output {
			panel: vec![
				heading("Open linked message"),
				text(format!(
					"Apply opens message {message_id} in channel {channel_id}. This is navigation only; no content is fetched or sent."
				)),
			],
			..Default::default()
		},
		effects: vec![HostEffect::JumpToMessage {
			channel_id: channel_id.into(),
			message_id: message_id.into(),
		}],
	}
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	match input.invocation.invocation.action.as_str() {
		"preview" => preview(&input),
		"jump" => jump(&input),
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

	fn app_with_message(
		channel_id: &str,
		message_id: &str,
		author: &str,
		content: &str,
	) -> serde_json::Value {
		serde_json::json!({
			"timeline": {
				"channel_id": channel_id,
				"messages": [{
					"id": message_id,
					"author": {"id": "42", "name": author},
					"content": content,
					"attachment_count": 0,
					"edited": false
				}],
				"truncated": false
			}
		})
	}

	fn selected(channel_id: &str, message_id: &str) -> serde_json::Value {
		serde_json::json!({
			"channel_id": channel_id,
			"message_id": message_id,
			"author_id": "42"
		})
	}

	#[test]
	fn extracts_only_exact_canonical_discord_message_links_and_deduplicates() {
		let target = "300000000000000001";
		let channel = "200000000000000001";
		let text = format!(
			"plain https://discord.com/channels/@me/{channel}/{target} \
			 and https://canary.discordapp.com/channels/100000000000000001/{channel}/{target} \
			 plus http://discord.com/channels/100000000000000001/{channel}/{target}. \
			 suppressed <https://discord.com/channels/100000000000000001/{channel}/{target}> \
			 fake https://discord.example/channels/100000000000000001/{channel}/{target} \
			 bad https://discord.com/channels/100000000000000001/{channel}/123"
		);
		let found = extract_links(&text);
		assert_eq!(
			found.links,
			vec![MessageLink {
				channel_id: channel.into(),
				message_id: target.into()
			}]
		);
		assert!(!found.truncated);
	}

	#[test]
	fn more_than_five_distinct_links_is_reported_as_truncated() {
		let links = (1..=6)
			.map(|index| {
				format!(
					"https://discord.com/channels/100000000000000001/20000000000000000{index}/30000000000000000{index}"
				)
			})
			.collect::<Vec<_>>()
			.join(" ");
		let found = extract_links(&links);
		assert_eq!(found.links.len(), MAX_LINKS);
		assert!(found.truncated);
	}

	#[test]
	fn preview_lists_loaded_messages_without_effects_and_offers_a_jump_option() {
		let channel = "200000000000000001";
		let loaded = "300000000000000001";
		let text = format!("look at https://discord.com/channels/@me/{channel}/{loaded}");
		let (output, wire) = run(serde_json::json!({
			"action": "preview",
			"selected_message": text,
			"selected_message_ref": selected(channel, "300000000000000002"),
			"app": app_with_message(channel, loaded, "Alice", "hello there")
		}));
		assert!(output.effects.is_empty());
		assert!(!wire.to_string().contains("jump_to_message"));
		let select = output
			.output
			.panel
			.iter()
			.find_map(|element| match element {
				Element::Select { options, .. } => Some(options),
				_ => None,
			})
			.expect("loaded link should offer a jump option");
		assert!(
			select[0]
				.as_str()
				.starts_with(format!("{channel}:{loaded} | Alice: hello there").as_str())
		);
	}

	#[test]
	fn jump_requires_the_target_to_still_be_in_the_loaded_timeline() {
		let channel = "200000000000000001";
		let loaded = "300000000000000001";
		let app = app_with_message(channel, loaded, "Alice", "hello");
		let valid_target = format!("{channel}:{loaded} | Alice: hello");

		let (valid, wire) = run(serde_json::json!({
			"action": "jump",
			"values": {"target": valid_target},
			"app": app
		}));
		assert_eq!(
			valid.effects,
			vec![HostEffect::JumpToMessage {
				channel_id: channel.into(),
				message_id: loaded.into()
			}]
		);
		assert!(wire.to_string().contains("jump_to_message"));

		for (target, app) in [
			(
				format!("{channel}:{loaded}"),
				app_with_message(channel, "300000000000000099", "Alice", "other"),
			),
			(
				format!("200000000000000099:{loaded}"),
				app_with_message(channel, loaded, "Alice", "other channel"),
			),
			(
				"not-an-id:also-not".into(),
				app_with_message(channel, loaded, "Alice", "x"),
			),
		] {
			let (output, _) = run(serde_json::json!({
				"action": "jump",
				"values": {"target": target},
				"app": app
			}));
			assert!(output.effects.is_empty());
		}
	}

	#[test]
	fn preview_without_a_loaded_timeline_never_fabricates_a_jump() {
		let channel = "200000000000000001";
		let target = "300000000000000001";
		let text = format!("https://discord.com/channels/@me/{channel}/{target}");
		let (output, _) = run(serde_json::json!({
			"action": "preview",
			"selected_message": text,
			"selected_message_ref": selected(channel, "300000000000000002")
		}));
		assert!(output.effects.is_empty());
		assert!(
			!output
				.output
				.panel
				.iter()
				.any(|element| matches!(element, Element::Button { .. } | Element::Select { .. }))
		);
	}
}
