/*
 * Vencord, a modification for Discord's desktop app
 * Copyright (c) 2022 Vendicated and contributors
 *
 * Serein adaptation copyright (c) 2026 Serein contributors.
 * Changed November 8, 2026: only explicitly granted message text and public IDs cross the ABI.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serein_extension_sdk::{
	AppOutput, Element, MAX_IO_BYTES, MessageActionInvocation, Output, serde_json,
};

const RAW_ELEMENT_BYTES: usize = 4000;
const MAX_RAW_ELEMENTS: usize = 16;

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	let selected = input
		.selected_message_ref
		.as_ref()
		.map(|message| {
			let mut ids = format!(
				"Channel ID: {}\nMessage ID: {}\nAuthor ID: {}",
				message.channel_id, message.message_id, message.author_id
			);
			if let Some(guild) = &message.guild_id {
				ids.push_str(&format!("\nServer ID: {guild}"));
			}
			ids
		})
		.unwrap_or_else(|| "Message IDs are unavailable on this host.".into());
	let raw = input
		.invocation
		.invocation
		.selected_message
		.unwrap_or_default();
	let mut panel = vec![Element::Heading {
		text: "Raw message text".into(),
	}];
	if raw.is_empty() {
		panel.push(text("(empty message)"));
	} else {
		let mut remaining = raw.as_str();
		for _ in 0..MAX_RAW_ELEMENTS {
			if remaining.len() <= RAW_ELEMENT_BYTES {
				panel.push(text(remaining));
				remaining = "";
				break;
			}
			let mut end = RAW_ELEMENT_BYTES;
			while !remaining.is_char_boundary(end) {
				end -= 1;
			}
			panel.push(text(&remaining[..end]));
			remaining = &remaining[end..];
		}
		if !remaining.is_empty() {
			panel.push(text(format!(
				"{} UTF-8 bytes omitted: the 64 KiB raw-message display limit was reached.",
				remaining.len()
			)));
		}
	}
	panel.extend([
		Element::Separator,
		text(selected),
		Element::Separator,
		text(
			"This safe ViewRaw port exposes only the selected text and IDs granted by Serein—not private host objects or raw Gateway data.",
		),
	]);
	let output = AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		..Default::default()
	};
	if serde_json::to_vec(&output).is_ok_and(|bytes| bytes.len() <= MAX_IO_BYTES) {
		output
	} else {
		AppOutput {
			output: Output {
				panel: vec![text(
					"This message is too large to display within the extension's 256 KiB response limit.",
				)],
				..Default::default()
			},
			..Default::default()
		}
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	#[test]
	fn exact_text_and_only_public_ids_cross_the_abi() {
		let input = serde_json::json!({
			"action": "view-raw",
			"selected_message": "**raw** <@42> \\n not a newline",
			"selected_message_ref": {
				"channel_id": "10", "message_id": "20", "author_id": "30", "guild_id": "40"
			}
		});
		let bytes = dispatch_typed(&serde_json::to_vec(&input).unwrap(), handle).unwrap();
		let output: AppOutput = serde_json::from_slice(&bytes).unwrap();
		assert!(output.effects.is_empty());
		assert!(output.output.storage.is_none());
		assert!(output.output.replacement.is_none());
		assert!(
			matches!(&output.output.panel[1], Element::Text { text } if text == "**raw** <@42> \\n not a newline")
		);
		assert!(
			matches!(&output.output.panel[3], Element::Text { text } if text.contains("Server ID: 40") && !text.contains("token"))
		);
	}

	#[test]
	fn older_hosts_without_ids_still_show_the_selected_text() {
		let bytes = dispatch_typed(
			br#"{"action":"view-raw","selected_message":"plain"}"#,
			handle,
		)
		.unwrap();
		let output: AppOutput = serde_json::from_slice(&bytes).unwrap();
		assert!(matches!(&output.output.panel[1], Element::Text { text } if text == "plain"));
		assert!(
			matches!(&output.output.panel[3], Element::Text { text } if text.contains("unavailable"))
		);
	}

	#[test]
	fn long_unicode_messages_stay_within_panel_and_response_limits() {
		let selected_message = format!("{}🦀{}", "a".repeat(3999), "x".repeat(65_000));
		let invocation = MessageActionInvocation {
			invocation: serein_extension_sdk::AppInvocation {
				invocation: serein_extension_sdk::Invocation {
					action: "view-raw".into(),
					selected_message: Some(selected_message),
					..Default::default()
				},
				..Default::default()
			},
			selected_message_ref: None,
		};
		let output = handle(invocation);
		assert_eq!(output.output.panel.len(), 22);
		assert!(serde_json::to_vec(&output).unwrap().len() <= MAX_IO_BYTES);
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("UTF-8 bytes omitted")
		)));
		assert!(output.output.panel.iter().all(|element| match element {
			Element::Text { text } => text.len() <= RAW_ELEMENT_BYTES.max(4096),
			_ => true,
		}));

		let invocation = MessageActionInvocation {
			invocation: serein_extension_sdk::AppInvocation {
				invocation: serein_extension_sdk::Invocation {
					action: "view-raw".into(),
					selected_message: Some("\u{1}".repeat(50_000)),
					..Default::default()
				},
				..Default::default()
			},
			selected_message_ref: None,
		};
		let output = handle(invocation);
		assert_eq!(output.output.panel.len(), 1);
		assert!(matches!(
			&output.output.panel[0],
			Element::Text { text } if text.contains("256 KiB response limit")
		));
	}
}
