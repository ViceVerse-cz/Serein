/*
 * Vencord, a modification for Discord's desktop app
 * Copyright (c) 2023 Vendicated and contributors
 * Copyright (c) 2024 Vendicated and contributors
 * Copyright (c) 2026 Serein port contributors
 *
 * Upstream sources at Equicord commit 200dba124066146a8f043fde7fc8b05e3dc5e68e:
 * src/plugins/copyUserURLs/index.tsx and src/equicordplugins/copyUserMention/index.tsx.
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
use serein_extension_sdk::{AppOutput, Element, HostEffect, MessageActionInvocation, Output};

/// Discord snowflakes are canonical nonzero u64 decimal strings.
const MAX_ID_BYTES: usize = 20;

fn canonical_id(id: &str) -> Option<&str> {
	let bytes = id.as_bytes();
	if bytes.is_empty()
		|| bytes.len() > MAX_ID_BYTES
		|| bytes[0] == b'0'
		|| !bytes.iter().all(u8::is_ascii_digit)
	{
		return None;
	}
	id.parse::<u64>()
		.ok()
		.filter(|value| *value != 0)
		.map(|_| id)
}

fn title(action: &str) -> Option<&'static str> {
	match action {
		"copy-user-url" => Some("Copy author profile URL"),
		"copy-user-mention" => Some("Copy author mention"),
		_ => None,
	}
}

fn copied_text(action: &str, author_id: &str) -> Option<String> {
	match action {
		"copy-user-url" => Some(format!("https://discord.com/users/{author_id}")),
		"copy-user-mention" => Some(format!("<@{author_id}>")),
		_ => None,
	}
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

/// No clipboard proposal when the host supplied no usable author ID.
fn unavailable(action: &str) -> AppOutput {
	let Some(title) = title(action) else {
		return AppOutput::default();
	};
	AppOutput {
		output: Output {
			panel: vec![
				Element::Heading { text: title.into() },
				text(
					"The host did not provide a valid author ID for this message. Nothing was proposed or copied.",
				),
			],
			..Default::default()
		},
		..Default::default()
	}
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	let action = input.invocation.invocation.action.as_str();
	let Some(author_id) = input
		.selected_message_ref
		.as_ref()
		.and_then(|message| canonical_id(&message.author_id))
	else {
		return unavailable(action);
	};
	let Some(text_value) = copied_text(action, author_id) else {
		return AppOutput::default();
	};
	let Some(title) = title(action) else {
		return AppOutput::default();
	};
	AppOutput {
		output: Output {
			panel: vec![
				Element::Heading { text: title.into() },
				text(format!("Reviewed text: {text_value}")),
				text(
					"Apply copies this exact text. Nothing is copied before Apply, and the host may reject a stale request.",
				),
			],
			..Default::default()
		},
		effects: vec![HostEffect::CopyText { text: text_value }],
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

	fn copied(output: &AppOutput) -> &str {
		assert_eq!(output.effects.len(), 1);
		match &output.effects[0] {
			HostEffect::CopyText { text } => text,
			other => panic!("unexpected effect: {other:?}"),
		}
	}

	#[test]
	fn exact_canonical_outputs_do_not_include_message_text_or_other_ids() {
		let selected_message = "source text that must never be copied <@99>";
		let reference = serde_json::json!({
			"channel_id": "10",
			"message_id": "20",
			"author_id": "42",
			"guild_id": "40"
		});
		let url = run(serde_json::json!({
			"action": "copy-user-url",
			"selected_message": selected_message,
			"selected_message_ref": reference
		}));
		assert_eq!(copied(&url.0), "https://discord.com/users/42");
		assert_eq!(
			url.1["effects"],
			serde_json::json!([
				{"type": "copy_text", "text": "https://discord.com/users/42"}
			])
		);
		assert!(url.0.output.replacement.is_none());
		assert!(url.0.output.storage.is_none());

		let mention = run(serde_json::json!({
			"action": "copy-user-mention",
			"selected_message": selected_message,
			"selected_message_ref": reference
		}));
		assert_eq!(copied(&mention.0), "<@42>");
		assert_eq!(
			mention.1["effects"],
			serde_json::json!([{"type": "copy_text", "text": "<@42>"}])
		);

		for (output, wire) in [url, mention] {
			let serialized = serde_json::to_string(&output).unwrap();
			assert!(!serialized.contains(selected_message));
			assert!(!serialized.contains("<@99>"));
			assert!(!wire.to_string().contains("discord.com/users/40"));
			assert!(!wire.to_string().contains("<@40>"));
		}
	}

	#[test]
	fn ids_must_be_canonical_nonzero_u64_text_and_invalid_ids_propose_nothing() {
		let max = "18446744073709551615";
		let valid = run(serde_json::json!({
			"action": "copy-user-mention",
			"selected_message_ref": {
				"channel_id": "10", "message_id": "20", "author_id": max
			}
		}));
		assert_eq!(copied(&valid.0), format!("<@{max}>").as_str());
		assert!(serde_json::to_vec(&valid.0).unwrap().len() <= serein_extension_sdk::MAX_IO_BYTES);

		let too_large = "18446744073709551616";
		for author_id in [
			"",
			"0",
			"00042",
			"-1",
			"1.0",
			" 42",
			"42 ",
			"٤٢",
			too_large,
			"000000000000000000000",
		] {
			for action in ["copy-user-url", "copy-user-mention"] {
				let (output, wire) = run(serde_json::json!({
					"action": action,
					"selected_message": "private source text",
					"selected_message_ref": {
						"channel_id": "10", "message_id": "20", "author_id": author_id
					}
				}));
				assert!(output.effects.is_empty(), "{action}: {author_id}");
				assert!(output.output.replacement.is_none());
				assert!(output.output.storage.is_none());
				assert!(!wire.to_string().contains("private source text"));
			}
		}

		for action in ["copy-user-url", "copy-user-mention"] {
			let absent = run(serde_json::json!({
				"action": action,
				"selected_message": "private source text",
				"selected_message_ref": serde_json::Value::Null
			}));
			let empty = run(serde_json::json!({
				"action": action,
				"selected_message": "private source text",
				"selected_message_ref": {
					"channel_id": "10", "message_id": "20", "author_id": ""
				}
			}));
			assert!(absent.0.effects.is_empty(), "{action}");
			assert!(empty.0.effects.is_empty(), "{action}");
		}
	}

	#[test]
	fn unknown_actions_do_not_fall_back_to_a_copy() {
		let (output, wire) = run(serde_json::json!({
			"action": "copy-anything",
			"selected_message": "private source text",
			"selected_message_ref": {
				"channel_id": "10", "message_id": "20", "author_id": "42"
			}
		}));
		assert!(output.effects.is_empty());
		assert!(output.output.panel.is_empty());
		assert!(output.output.replacement.is_none());
		assert!(!wire.to_string().contains("private source text"));
	}
}
