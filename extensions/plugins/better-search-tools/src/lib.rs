/*
 * Vencord/Equicord, a Discord client mod
 * Copyright (c) 2026 Vendicated and contributors
 *
 * Upstream source at Equicord commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e:
 * src/plugins/betterSearch/index.ts
 *
 * Changed November 8, 2026: this port does not patch Discord's search filter
 * registry or autocomplete. It provides one bounded panel that validates the
 * same extra token shapes and proposes the exact native search query after
 * Apply. No JavaScript, Webpack module or Discord internals are touched.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{AppInvocation, AppOutput, Element, HostEffect, Output};

/// The host accepts a search query of at most 256 UTF-8 bytes.
const MAX_QUERY_BYTES: usize = 256;
/// Upstream treats embed providers, command names and query words as single tokens.
const MAX_WORD_BYTES: usize = 64;
const MAX_ID_BYTES: usize = 20;
const EMBED_TYPES: [&str; 5] = ["image", "video", "gif", "sound", "article"];

const UNSAFE_QUERY: &str =
	"The composed query exceeds the host's 256 UTF-8 byte search limit. Remove or shorten fields.";
const EMPTY_QUERY: &str = "Enter search text or choose at least one filter.";
const CONTROL_TEXT: &str = "Search fields cannot contain control characters.";

fn heading(value: &str) -> Element {
	Element::Heading { text: value.into() }
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
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
	panel_output(vec![heading("Extended search"), text(message)])
}

fn option(id: &'static str, label: &'static str, options: &[&str]) -> Element {
	Element::Select {
		id: id.into(),
		label: label.into(),
		options: options.iter().map(|value| (*value).into()).collect(),
		value: "any".into(),
	}
}

fn input(id: &'static str, label: &'static str) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: String::new(),
	}
}

fn open() -> AppOutput {
	panel_output(vec![
		heading("Extended Discord search"),
		text(
			"Build a query with the extra token shapes Equicord's BetterSearch adds. Serein passes the exact reviewed query to native search. Discord server support for these extra tokens is unverified.",
		),
		input("query", "Search text"),
		option("include-nsfw", "includeNsfw", &["any", "true", "false"]),
		option(
			"mention-everyone",
			"mentionEveryone",
			&["any", "true", "false"],
		),
		input("replied-to-message-id", "repliedToMessageId"),
		input("replied-to-user-id", "repliedToUserId"),
		input("mentions-role-id", "mentionsRoleId"),
		option(
			"embed-type",
			"embedType",
			&["any", "image", "video", "gif", "sound", "article"],
		),
		input("embed-provider", "embedProvider"),
		input("command-id", "commandId"),
		input("command-name", "commandName"),
		input("slop", "slop (0-100)"),
		Element::Button {
			id: "search".into(),
			label: "Review extended search".into(),
		},
	])
}

fn selected<'a>(input: &'a AppInvocation, id: &str) -> Option<&'a str> {
	input
		.invocation
		.value(id)
		.map(str::trim)
		.filter(|value| !value.is_empty())
}

fn has_control(value: &str) -> bool {
	value.chars().any(char::is_control)
}

fn is_search_id(value: &str) -> bool {
	(17..=MAX_ID_BYTES).contains(&value.len())
		&& value.as_bytes()[0] != b'0'
		&& value.bytes().all(|byte| byte.is_ascii_digit())
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
}

fn is_word(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= MAX_WORD_BYTES
		&& !value.chars().any(char::is_whitespace)
		&& !has_control(value)
}

fn push_bool(
	parts: &mut Vec<String>,
	input: &AppInvocation,
	field: &str,
	token: &str,
) -> Result<(), &'static str> {
	let Some(value) = selected(input, field) else {
		return Ok(());
	};
	if value == "any" {
		return Ok(());
	}
	if value != "true" && value != "false" {
		return Err("Boolean search filters accept only true or false.");
	}
	parts.push(format!("{token}:{value}"));
	Ok(())
}

fn push_word(
	parts: &mut Vec<String>,
	input: &AppInvocation,
	field: &str,
	token: &str,
) -> Result<(), &'static str> {
	let Some(value) = selected(input, field) else {
		return Ok(());
	};
	if !is_word(value) {
		return Err(
			"Embed provider and command name filters must be one non-control word of at most 64 UTF-8 bytes.",
		);
	}
	parts.push(format!("{token}:{value}"));
	Ok(())
}

fn push_id(
	parts: &mut Vec<String>,
	input: &AppInvocation,
	field: &str,
	token: &str,
) -> Result<(), &'static str> {
	let Some(value) = selected(input, field) else {
		return Ok(());
	};
	if !is_search_id(value) {
		return Err("ID filters must be canonical nonzero decimal Discord IDs of 17 to 20 digits.");
	}
	parts.push(format!("{token}:{value}"));
	Ok(())
}

fn compose(input: &AppInvocation) -> Result<String, &'static str> {
	let mut parts = Vec::new();
	if let Some(query) = selected(input, "query") {
		if has_control(query) {
			return Err(CONTROL_TEXT);
		}
		parts.push(query.to_owned());
	}
	push_bool(&mut parts, input, "include-nsfw", "includeNsfw")?;
	push_bool(&mut parts, input, "mention-everyone", "mentionEveryone")?;
	push_id(
		&mut parts,
		input,
		"replied-to-message-id",
		"repliedToMessageId",
	)?;
	push_id(&mut parts, input, "replied-to-user-id", "repliedToUserId")?;
	push_id(&mut parts, input, "mentions-role-id", "mentionsRoleId")?;
	if let Some(value) = selected(input, "embed-type")
		&& value != "any"
	{
		if !EMBED_TYPES.contains(&value) {
			return Err("embedType must be image, video, gif, sound or article.");
		}
		parts.push(format!("embedType:{value}"));
	}
	push_word(&mut parts, input, "embed-provider", "embedProvider")?;
	push_id(&mut parts, input, "command-id", "commandId")?;
	push_word(&mut parts, input, "command-name", "commandName")?;
	if let Some(value) = selected(input, "slop") {
		if !value.bytes().all(|byte| byte.is_ascii_digit()) {
			return Err("slop must be a whole number from 0 to 100.");
		}
		let level = value
			.parse::<u8>()
			.map_err(|_| "slop must be a whole number from 0 to 100.")?;
		if level > 100 {
			return Err("slop must be a whole number from 0 to 100.");
		}
		parts.push(format!("slop:{level}"));
	}
	let query = parts.join(" ");
	if query.is_empty() {
		return Err(EMPTY_QUERY);
	}
	if query.len() > MAX_QUERY_BYTES {
		return Err(UNSAFE_QUERY);
	}
	Ok(query)
}

fn search(input: &AppInvocation) -> AppOutput {
	match compose(input) {
		Ok(query) => AppOutput {
			output: Output {
				panel: vec![
					heading("Extended search"),
					text(format!("Reviewed query: {query}")),
					text(
						"Apply proposes this exact query to the host's native search. Nothing is searched before Apply.",
					),
				],
				..Default::default()
			},
			effects: vec![HostEffect::Search { query }],
		},
		Err(message) => error_output(message),
	}
}

fn handle(input: AppInvocation) -> AppOutput {
	match input.invocation.action.as_str() {
		"open" => open(),
		"search" => search(&input),
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

	#[test]
	fn composes_the_reviewed_custom_filters_exactly() {
		let (output, wire) = run(serde_json::json!({
			"action": "search",
			"values": {
				"query": "hello world",
				"include-nsfw": "true",
				"mention-everyone": "false",
				"replied-to-message-id": "300000000000000001",
				"replied-to-user-id": "300000000000000002",
				"mentions-role-id": "300000000000000003",
				"embed-type": "image",
				"embed-provider": "tenor",
				"command-id": "300000000000000004",
				"command-name": "ping",
				"slop": "25"
			}
		}));
		let expected = "hello world includeNsfw:true mentionEveryone:false \
			repliedToMessageId:300000000000000001 repliedToUserId:300000000000000002 \
			mentionsRoleId:300000000000000003 embedType:image embedProvider:tenor \
			commandId:300000000000000004 commandName:ping slop:25";
		assert_eq!(
			output.effects,
			vec![HostEffect::Search {
				query: expected.into()
			}]
		);
		assert!(wire.to_string().contains("hello world"));
		assert!(output.output.replacement.is_none());
		assert!(output.output.storage.is_none());
	}

	#[test]
	fn open_offers_only_bounded_controls_and_never_searches() {
		let (output, _) = run(serde_json::json!({"action": "open"}));
		assert!(output.effects.is_empty());
		let options = output
			.output
			.panel
			.iter()
			.filter_map(|element| match element {
				Element::Select { options, .. } => Some(options),
				_ => None,
			})
			.collect::<Vec<_>>();
		assert_eq!(options.len(), 3);
		assert_eq!(
			options[0],
			&vec!["any".to_string(), "true".into(), "false".into()]
		);
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Button { id, .. } if id == "search"
		)));
	}

	#[test]
	fn invalid_tokens_and_oversized_queries_propose_nothing() {
		let long = "x".repeat(MAX_QUERY_BYTES + 1);
		let cases = [
			serde_json::json!({"action": "search", "values": {"slop": "101"}}),
			serde_json::json!({"action": "search", "values": {"slop": "-1"}}),
			serde_json::json!({"action": "search", "values": {"slop": "1.5"}}),
			serde_json::json!({"action": "search", "values": {"slop": "+5"}}),
			serde_json::json!({"action": "search", "values": {"embed-type": "audio"}}),
			serde_json::json!({"action": "search", "values": {"include-nsfw": "yes"}}),
			serde_json::json!({"action": "search", "values": {"replied-to-message-id": "123"}}),
			serde_json::json!({"action": "search", "values": {"replied-to-message-id": "0300000000000000001"}}),
			serde_json::json!({"action": "search", "values": {"embed-provider": "two words"}}),
			serde_json::json!({"action": "search", "values": {"command-name": "bad\u{7}name"}}),
			serde_json::json!({"action": "search", "values": {"query": long}}),
			serde_json::json!({"action": "search", "values": {}}),
			serde_json::json!({"action": "search", "values": {
				"embed-type": "any", "include-nsfw": "any", "mention-everyone": "any"
			}}),
		];
		for case in cases {
			let (output, _) = run(case.clone());
			assert!(output.effects.is_empty(), "{case}");
			assert!(output.output.replacement.is_none());
		}
	}

	#[test]
	fn empty_optional_fields_are_omitted_and_query_only_is_allowed() {
		let (output, _) = run(serde_json::json!({
			"action": "search",
			"values": {
				"query": "only text",
				"include-nsfw": "any",
				"embed-provider": "  ",
				"slop": ""
			}
		}));
		assert_eq!(
			output.effects,
			vec![HostEffect::Search {
				query: "only text".into()
			}]
		);
	}
}
