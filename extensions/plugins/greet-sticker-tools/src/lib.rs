/*
 * Vencord/Equicord, a Discord client mod
 * Copyright (c) 2023 Vendicated and contributors
 *
 * Upstream source at Equicord commit
 * 200dba124066146a8f043fde7fc8b05e3dc5e68e:
 * src/plugins/greetStickerPicker/index.tsx
 *
 * Changed November 8, 2026: this port does not patch the welcome CTA, does not
 * hold a fixed private sticker catalog and does not send automatically. It
 * offers the stickers already attached to the selected loaded message and
 * proposes one host-reviewed sticker send after Apply.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppAction, AppOutput, Element, HostEffect, MessageActionInvocation, Output,
};

const MAX_ID_BYTES: usize = 20;
const MAX_NAME_BYTES: usize = 40;
const MAX_OPTION_BYTES: usize = 128;

const MISSING_IDS: &str =
	"The host did not provide the selected message IDs, so no loaded sticker can be matched.";
const NO_CONTENT: &str = "The host did not provide loaded sticker metadata for this chat. Reopen the message and try again.";
const NO_STICKERS: &str = "This loaded message has no stickers. Nothing can be re-sent from it.";

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
	panel_output(vec![heading("Re-send sticker"), text(message)])
}

fn is_snowflake(value: &str) -> bool {
	let bytes = value.as_bytes();
	!bytes.is_empty()
		&& bytes.len() <= MAX_ID_BYTES
		&& bytes[0] != b'0'
		&& bytes.iter().all(u8::is_ascii_digit)
		&& value.parse::<u64>().is_ok_and(|id| id != 0)
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

fn sanitize_name(value: &str) -> String {
	let cleaned = value
		.chars()
		.map(|character| {
			if character.is_control() || character == ':' || character == '|' {
				' '
			} else {
				character
			}
		})
		.collect::<String>();
	let cleaned = cleaned.trim();
	if cleaned.is_empty() {
		"Sticker".into()
	} else {
		truncate(cleaned, MAX_NAME_BYTES)
	}
}

/// Panel actions lose `selected_message_ref`, so the option carries the channel,
/// message and sticker IDs that the `send` action revalidates against the
/// snapshot. Names follow the third colon and are display-only.
fn sticker_option(channel_id: &str, message_id: &str, sticker_id: &str, name: &str) -> String {
	let prefix = format!("{channel_id}:{message_id}:{sticker_id}:");
	let remaining = MAX_OPTION_BYTES.saturating_sub(prefix.len());
	truncate(
		&format!("{prefix}{}", truncate(name, remaining)),
		MAX_OPTION_BYTES,
	)
}

fn selected_message(
	input: &MessageActionInvocation,
) -> Option<(&str, &str, &serein_extension_sdk::MessageContentSnapshot)> {
	let reference = input.selected_message_ref.as_ref()?;
	if !is_snowflake(&reference.channel_id) || !is_snowflake(&reference.message_id) {
		return None;
	}
	let content = input.invocation.app.as_ref()?.message_content.as_ref()?;
	(content.channel_id == reference.channel_id).then_some((
		reference.channel_id.as_str(),
		reference.message_id.as_str(),
		content,
	))
}

fn open(input: &MessageActionInvocation) -> AppOutput {
	let Some((channel_id, message_id, content)) = selected_message(input) else {
		return error_output(MISSING_IDS);
	};
	let Some(message) = content
		.items
		.iter()
		.find(|message| message.id == message_id)
	else {
		return error_output(
			"The selected message is not in the loaded sticker window. No sticker was selected.",
		);
	};
	if message.stickers.is_empty() {
		return error_output(NO_STICKERS);
	}
	let mut options = Vec::new();
	for sticker in &message.stickers {
		if !is_snowflake(&sticker.id) {
			continue;
		}
		options.push(sticker_option(
			channel_id,
			message_id,
			&sticker.id,
			&sanitize_name(&sticker.name),
		));
	}
	if options.is_empty() {
		return error_output(NO_STICKERS);
	}
	let mut elements = vec![
		heading("Re-send sticker"),
		text(format!(
			"Choose one of the {} loaded sticker(s) on this message. Apply sends it as a new message; no text, mention or attachment is added.",
			options.len()
		)),
	];
	if message.stickers_truncated {
		elements.push(text(
			"The host truncated this message's sticker list, so only the loaded stickers above are offered.",
		));
	}
	elements.push(Element::Select {
		id: "sticker".into(),
		label: "Sticker on this message".into(),
		options: options.clone(),
		value: options[0].clone(),
	});
	elements.push(Element::Button {
		id: "send".into(),
		label: "Review sticker send".into(),
	});
	panel_output(elements)
}

fn send(input: &MessageActionInvocation) -> AppOutput {
	let Some(value) = input.invocation.invocation.value("sticker") else {
		return error_output("No sticker was selected in the panel.");
	};
	let mut parts = value.splitn(4, ':');
	let (Some(channel_id), Some(message_id), Some(sticker_id)) =
		(parts.next(), parts.next(), parts.next())
	else {
		return error_output("The selected sticker reference is malformed; nothing was proposed.");
	};
	if !is_snowflake(channel_id) || !is_snowflake(message_id) || !is_snowflake(sticker_id) {
		return error_output("The selected sticker reference is invalid; nothing was proposed.");
	}
	let Some(content) = input
		.invocation
		.app
		.as_ref()
		.and_then(|app| app.message_content.as_ref())
	else {
		return error_output(NO_CONTENT);
	};
	if content.channel_id != channel_id {
		return error_output(
			"The selected sticker belongs to a different loaded chat, so no send was proposed.",
		);
	}
	let Some(message) = content
		.items
		.iter()
		.find(|message| message.id == message_id)
	else {
		return error_output(
			"The selected message is no longer in the loaded sticker window. Nothing was sent.",
		);
	};
	if !message
		.stickers
		.iter()
		.any(|sticker| sticker.id == sticker_id)
	{
		return error_output(
			"That sticker is not one of the loaded stickers on this message. Nothing was sent.",
		);
	}
	AppOutput {
		output: Output {
			panel: vec![
				heading("Send sticker"),
				text(format!(
					"Apply sends sticker {sticker_id} to channel {channel_id} as a new message. Nothing is sent before Apply, and the host may reject a stale request."
				)),
			],
			..Default::default()
		},
		effects: vec![HostEffect::AppAction {
			action: AppAction::SendSticker {
				channel_id: channel_id.into(),
				sticker_id: sticker_id.into(),
			},
		}],
	}
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	match input.invocation.invocation.action.as_str() {
		"open" => open(&input),
		"send" => send(&input),
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

	fn app(channel_id: &str, message_id: &str, sticker_id: &str) -> serde_json::Value {
		serde_json::json!({
			"message_content": {
				"channel_id": channel_id,
				"items": [{
					"id": message_id,
					"embeds": [],
					"embeds_truncated": false,
					"embeds_suppressed": false,
					"stickers": [{
						"id": sticker_id,
						"name": "Wave",
						"format_type": 1
					}],
					"stickers_truncated": false,
					"poll": "absent"
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
	fn open_offers_only_loaded_stickers_and_never_sends() {
		let (output, wire) = run(serde_json::json!({
			"action": "open",
			"selected_message": "",
			"selected_message_ref": selected("100", "500"),
			"app": app("100", "500", "77")
		}));
		assert!(output.effects.is_empty());
		assert!(!wire.to_string().contains("send_sticker"));
		let select = output
			.output
			.panel
			.iter()
			.find_map(|element| match element {
				Element::Select { options, .. } => Some(options),
				_ => None,
			})
			.expect("sticker select");
		assert_eq!(select, &vec!["100:500:77:Wave".to_string()]);
	}

	#[test]
	fn send_requires_an_exact_loaded_channel_message_sticker_triple() {
		let valid = "100:500:77:Wave";
		let (output, wire) = run(serde_json::json!({
			"action": "send",
			"values": {"sticker": valid},
			"app": app("100", "500", "77")
		}));
		assert_eq!(
			output.effects,
			vec![HostEffect::AppAction {
				action: AppAction::SendSticker {
					channel_id: "100".into(),
					sticker_id: "77".into()
				}
			}]
		);
		assert!(wire.to_string().contains("send_sticker"));

		for (value, app) in [
			("100:500:88:Other", app("100", "500", "77")),
			("999:500:77:Wave", app("100", "500", "77")),
			("100:500:77:Wave", app("100", "501", "77")),
			("100:500:notid:Wave", app("100", "500", "77")),
			("", app("100", "500", "77")),
		] {
			let (output, _) = run(serde_json::json!({
				"action": "send",
				"values": {"sticker": value},
				"app": app
			}));
			assert!(output.effects.is_empty(), "{value}");
		}
	}

	#[test]
	fn missing_loaded_sticker_metadata_proposes_nothing() {
		let (open, _) = run(serde_json::json!({
			"action": "open",
			"selected_message": "",
			"selected_message_ref": selected("100", "500")
		}));
		assert!(open.effects.is_empty());
		assert!(
			!open
				.output
				.panel
				.iter()
				.any(|element| matches!(element, Element::Select { .. } | Element::Button { .. }))
		);

		let (send, _) = run(serde_json::json!({
			"action": "send",
			"values": {"sticker": "100:500:77:Wave"}
		}));
		assert!(send.effects.is_empty());
	}
}
