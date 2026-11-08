/*
 * Ported from Equicord/Vencord at commit
 * 200dba124066146a8f043de7fc8b05e3dc5e68e.
 *
 * baseDecoder (composer `decode` and message `decode-message` actions):
 * Copyright (c) 2022 Vendicated and contributors
 *
 * writeUpperCase:
 * Copyright (c) 2023 Vendicated and contributors
 *
 * Copyright (c) 2026 Serein port contributors
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use serein_extension_sdk::{
	AppOutput, Element, HostEffect, MessageActionInvocation, Output, serde_json,
};

// Serein's draft proposals are bounded before Apply. The UTF-8 byte bound also
// leaves room for JSON escaping in the complete 256 KiB response.
const MAX_DRAFT_BYTES: usize = 8_000;
const MAX_DRAFT_CHARS: usize = 2_000;

// The message action is bounded before it crosses the clipboard-effect limit.
// Upstream lists every decoded string; here the count, each decoded string, the
// displayed text and the clipboard effect all have explicit ceilings.
const MAX_MESSAGE_MATCHES: usize = 8;
const MAX_MATCH_BYTES: usize = 4_096;
const MAX_DISPLAY_CHARS: usize = 1_024;
const MAX_COPY_BYTES: usize = 6_144;
const MAX_EFFECT_BYTES: usize = 8_000;

// WriteUpperCase's default `blockedWords` setting, verbatim. The package has no
// panel or storage action, so these defaults are intentionally not configurable.
const BLOCKED_WORDS: [&str; 3] = ["http", "https", "ok"];

const DRAFT_LIMIT: &str = "The draft or transformed result exceeds the 2,000-character or 8,000-UTF-8-byte limit. The draft was left unchanged.";
const NO_BASE64: &str =
	"No Base64 text decoding to valid UTF-8 was found. The draft was left unchanged.";

fn panel(message: impl Into<String>) -> Output {
	Output {
		panel: vec![Element::Text {
			text: message.into(),
		}],
		..Default::default()
	}
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	let action = input.invocation.invocation.action.clone();
	match action.as_str() {
		"decode" => composer_output(decode_action(input.invocation.invocation.composer)),
		"uppercase" => composer_output(uppercase_action(input.invocation.invocation.composer)),
		"decode-message" => decode_message(&input),
		_ => AppOutput::default(),
	}
}

/// Adapt a composer result to the effect-capable envelope. Composer actions
/// never propose a host effect.
fn composer_output(output: Output) -> AppOutput {
	AppOutput {
		output,
		effects: Vec::new(),
	}
}

fn text_within_bounds(text: &str) -> bool {
	text.len() <= MAX_DRAFT_BYTES && text.chars().count() <= MAX_DRAFT_CHARS
}

fn decode_action(composer: Option<String>) -> Output {
	let Some(draft) = composer else {
		return Output::default();
	};
	if !text_within_bounds(&draft) {
		return panel(DRAFT_LIMIT);
	}
	let (decoded, count) = decode_base64_text(&draft);
	if count == 0 {
		return panel(NO_BASE64);
	}
	if !text_within_bounds(&decoded) {
		return panel(DRAFT_LIMIT);
	}
	Output {
		replacement: Some(decoded),
		..Default::default()
	}
}

fn uppercase_action(composer: Option<String>) -> Output {
	let Some(draft) = composer else {
		return Output::default();
	};
	if !text_within_bounds(&draft) {
		return panel(DRAFT_LIMIT);
	}
	let transformed = uppercase_sentences(&draft);
	if !text_within_bounds(&transformed) {
		return panel(DRAFT_LIMIT);
	}
	Output {
		replacement: Some(transformed),
		..Default::default()
	}
}

fn is_base64_byte(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/')
}

fn is_ascii_word_byte(byte: u8) -> bool {
	byte.is_ascii_alphanumeric() || byte == b'_'
}

fn base64_value(byte: u8) -> Option<u8> {
	Some(match byte {
		b'A'..=b'Z' => byte - b'A',
		b'a'..=b'z' => byte - b'a' + 26,
		b'0'..=b'9' => byte - b'0' + 52,
		b'+' => 62,
		b'/' => 63,
		_ => return None,
	})
}

/// Decode standard-alphabet Base64 with optional omitted trailing padding.
///
/// Upstream passes `atob` output through a fatal UTF-8 decoder, so invalid bytes
/// make the whole candidate unsupported. Rust's `String::from_utf8` implements
/// that same fatal validation.
fn decode_base64_candidate(candidate: &str) -> Option<String> {
	let bytes = candidate.as_bytes();
	let remainder = bytes.len() % 4;
	if remainder == 1 {
		return None;
	}
	let mut values = [0_u8; 4];
	let mut decoded = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
	for (index, &byte) in bytes.iter().enumerate() {
		values[index % 4] = base64_value(byte)?;
		if index % 4 == 3 {
			decoded.push((values[0] << 2) | (values[1] >> 4));
			decoded.push(((values[1] & 0x0f) << 4) | (values[2] >> 2));
			decoded.push(((values[2] & 0x03) << 6) | values[3]);
		}
	}
	match remainder {
		2 => decoded.push((values[0] << 2) | (values[1] >> 4)),
		3 => {
			decoded.push((values[0] << 2) | (values[1] >> 4));
			decoded.push(((values[1] & 0x0f) << 4) | (values[2] >> 2));
		}
		_ => {}
	}
	String::from_utf8(decoded).ok()
}

/// One replaceable standard-Base64 span in a larger text.
struct DecodedSpan {
	/// First byte of the token, excluding leading `+`/`/`.
	start: usize,
	/// First byte after the token, including consumed `=` padding.
	consumed: usize,
	decoded: String,
}

/// Port of the useful part of DecodeBase64's finder: a maximal standard Base64
/// alphabet run, requiring at least four characters and word boundaries at its
/// alphanumeric ends. `+` and `/` may occur inside the run but not at its edges.
///
/// Every other byte, including invalid UTF-8 candidates and URL-safe Base64, is
/// skipped. Only one pass is made, as in the upstream modal.
fn scan_base64(text: &str) -> Vec<DecodedSpan> {
	let bytes = text.as_bytes();
	let mut spans = Vec::new();
	let mut index = 0;

	while index < bytes.len() {
		if !is_base64_byte(bytes[index]) {
			index += 1;
			continue;
		}
		let run_start = index;
		while index < bytes.len() && is_base64_byte(bytes[index]) {
			index += 1;
		}
		let run_end = index;

		let mut start = run_start;
		while start < run_end && !bytes[start].is_ascii_alphanumeric() {
			start += 1;
		}
		let mut end = run_end;
		while end > start && !bytes[end - 1].is_ascii_alphanumeric() {
			end -= 1;
		}
		if end - start < 4 {
			continue;
		}
		// JavaScript's `\b` uses ASCII word characters. Runs touching `_` do not
		// satisfy the upstream boundaries.
		if start > 0 && is_ascii_word_byte(bytes[start - 1]) {
			continue;
		}
		if end < bytes.len() && is_ascii_word_byte(bytes[end]) {
			continue;
		}

		let Some(decoded) = decode_base64_candidate(&text[start..end]) else {
			continue;
		};

		// A standard padded token may have had its `=` ignored by the upstream
		// regex. Consume the valid padding together with the token so a composer
		// replacement does not leave stray separators behind.
		let mut consumed = end;
		let expected_padding = (4 - (end - start) % 4) % 4;
		if expected_padding > 0
			&& consumed + expected_padding <= bytes.len()
			&& bytes[consumed..consumed + expected_padding]
				.iter()
				.all(|&byte| byte == b'=')
		{
			consumed += expected_padding;
		}

		spans.push(DecodedSpan {
			start,
			consumed,
			decoded,
		});
		index = index.max(consumed);
	}

	spans
}

/// Replace every decodable span in place. Text outside a span is copied
/// byte-for-byte. Returns the replacement and the number of spans decoded.
fn decode_base64_text(text: &str) -> (String, usize) {
	let spans = scan_base64(text);
	if spans.is_empty() {
		return (text.to_owned(), 0);
	}
	let mut output = String::with_capacity(text.len());
	let mut copied_through = 0;
	for span in &spans {
		output.push_str(&text[copied_through..span.start]);
		output.push_str(&span.decoded);
		copied_through = span.consumed;
	}
	output.push_str(&text[copied_through..]);
	(output, spans.len())
}

fn is_sentence_punctuation(character: char) -> bool {
	matches!(character, '.' | '!' | '?')
}

fn is_sentence_suffix(character: char) -> bool {
	matches!(character, '\'' | '"' | ')' | ']')
}

fn next_char(text: &str, index: usize) -> Option<char> {
	text.get(index..)?.chars().next()
}

fn capitalize_sentence(sentence: &str) -> String {
	if sentence.is_empty() {
		return String::new();
	}
	let lowercase = sentence.to_lowercase();
	if BLOCKED_WORDS
		.iter()
		.any(|blocked| lowercase.starts_with(blocked))
	{
		return sentence.to_owned();
	}
	let Some(first) = sentence.chars().next() else {
		return String::new();
	};
	// Upstream uppercases only the first character, including Unicode case
	// expansions such as ß -> SS. It does not lowercase or title-case the rest.
	let mut result: String = first.to_uppercase().collect();
	result.push_str(&sentence[first.len_utf8()..]);
	result
}

/// Port of WriteUpperCase's split:
/// `/(?<=[.!?]+['")\]]*)(\s+)/`.
///
/// Separators stay in the output. A sentence beginning with a quote or bracket
/// is left alone because that character, not the following letter, is upstream's
/// first character. Leading whitespace is part of the first segment and likewise
/// does not trigger capitalization.
fn uppercase_sentences(text: &str) -> String {
	let mut output = String::with_capacity(text.len());
	let mut segment_start = 0;
	let mut index = 0;

	while let Some(character) = next_char(text, index) {
		if !is_sentence_punctuation(character) {
			index += character.len_utf8();
			continue;
		}

		let mut punctuation_end = index;
		while let Some(next) = next_char(text, punctuation_end)
			&& is_sentence_punctuation(next)
		{
			punctuation_end += next.len_utf8();
		}

		let mut sentence_end = punctuation_end;
		while let Some(next) = next_char(text, sentence_end)
			&& is_sentence_suffix(next)
		{
			sentence_end += next.len_utf8();
		}

		let mut separator_end = sentence_end;
		while let Some(next) = next_char(text, separator_end)
			&& next.is_whitespace()
		{
			separator_end += next.len_utf8();
		}

		if separator_end > sentence_end {
			output.push_str(&capitalize_sentence(&text[segment_start..sentence_end]));
			output.push_str(&text[sentence_end..separator_end]);
			segment_start = separator_end;
			index = separator_end;
		} else {
			index += character.len_utf8();
		}
	}

	output.push_str(&capitalize_sentence(&text[segment_start..]));
	output
}

fn message_output(panel: Vec<Element>, effect: Option<HostEffect>) -> AppOutput {
	AppOutput {
		output: Output {
			panel,
			..Default::default()
		},
		effects: effect.into_iter().collect(),
	}
}

fn excerpt(text: &str) -> String {
	if text.chars().count() <= MAX_DISPLAY_CHARS {
		return text.to_owned();
	}
	let mut shown: String = text.chars().take(MAX_DISPLAY_CHARS).collect();
	shown.push('…');
	shown
}

/// Port of DecodeBase64's message-popover action: decode standard Base64 spans
/// found in the selected message text and propose copying the decoded content.
///
/// The upstream modal lists every decoded string and copies on a per-string
/// button. Serein allows one reviewed effect per result, so this handler shows
/// the bounded decoded matches and proposes one copy of the matches joined by a
/// blank line. Nothing is copied before Apply, and the panel states exactly how
/// many matches the copy contains.
fn decode_message(input: &MessageActionInvocation) -> AppOutput {
	let Some(message) = input.invocation.invocation.selected_message.as_deref() else {
		return message_output(
			vec![Element::Heading {
				text: "Decode message Base64".into(),
			}, Element::Text {
				text: "The host did not provide the selected message text. Nothing was decoded or proposed."
					.into(),
			}],
			None,
		);
	};

	let spans = scan_base64(message);
	if spans.is_empty() {
		return message_output(
			vec![
				Element::Heading {
					text: "Decode message Base64".into(),
				},
				Element::Text {
					text: "No Base64 span decoding to valid UTF-8 was found in the selected message. Nothing was proposed."
						.into(),
				},
			],
			None,
		);
	}

	let total = spans.len();
	let mut matches: Vec<String> = Vec::new();
	for span in spans {
		if matches.len() == MAX_MESSAGE_MATCHES {
			break;
		}
		if span.decoded.len() > MAX_MATCH_BYTES {
			continue;
		}
		matches.push(span.decoded);
	}
	let omitted = total.saturating_sub(matches.len());

	let mut copy = String::new();
	let mut copied = 0;
	for decoded in &matches {
		let mut candidate = copy.clone();
		if !candidate.is_empty() {
			candidate.push_str("\n\n");
		}
		candidate.push_str(decoded);
		if candidate.len() > MAX_COPY_BYTES
			|| !serde_json::to_vec(&HostEffect::CopyText {
				text: candidate.clone(),
			})
			.is_ok_and(|bytes| bytes.len() <= MAX_EFFECT_BYTES)
		{
			break;
		}
		copy = candidate;
		copied += 1;
	}

	let mut panel = vec![
		Element::Heading {
			text: "Decode message Base64".into(),
		},
		Element::Text {
			text: format!("Decoded {total} Base64 span(s) in the selected message."),
		},
	];
	for (index, decoded) in matches.iter().enumerate() {
		panel.push(Element::Text {
			text: format!("Decoded match {}:", index + 1),
		});
		panel.push(Element::Text {
			text: excerpt(decoded),
		});
	}
	if omitted > 0 {
		panel.push(Element::Text {
			text: format!(
				"{omitted} further decoded match(es) were omitted by the {MAX_MESSAGE_MATCHES}-match or {MAX_MATCH_BYTES}-byte-per-match bound."
			),
		});
	}
	if copied == 0 {
		panel.push(Element::Text {
			text: "No decoded match fits the reviewed clipboard bound, so nothing was proposed."
				.into(),
		});
		return message_output(panel, None);
	}
	if copied < matches.len() {
		panel.push(Element::Text {
			text: format!(
				"Only the first {copied} of {} decoded matches fit the reviewed clipboard bound.",
				matches.len()
			),
		});
	}
	panel.push(Element::Text {
		text: format!(
			"Apply copies {copied} decoded match(es), separated by a blank line. Nothing was copied before Apply."
		),
	});
	message_output(panel, Some(HostEffect::CopyText { text: copy }))
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	fn dispatch_output(value: serde_json::Value) -> AppOutput {
		let bytes = dispatch_typed(&serde_json::to_vec(&value).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	fn invoke(action: &str, composer: &str) -> Output {
		let output = dispatch_output(serde_json::json!({
			"action": action,
			"composer": composer
		}));
		assert!(output.effects.is_empty());
		output.output
	}

	fn base64(bytes: &[u8]) -> String {
		const ALPHABET: &[u8; 64] =
			b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
		let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
		for chunk in bytes.chunks(3) {
			let first = u32::from(chunk[0]);
			let second = u32::from(*chunk.get(1).unwrap_or(&0));
			let third = u32::from(*chunk.get(2).unwrap_or(&0));
			let value = (first << 16) | (second << 8) | third;
			encoded.push(ALPHABET[(value >> 18) as usize & 0x3f] as char);
			encoded.push(ALPHABET[(value >> 12) as usize & 0x3f] as char);
			encoded.push(if chunk.len() > 1 {
				ALPHABET[(value >> 6) as usize & 0x3f] as char
			} else {
				'='
			});
			encoded.push(if chunk.len() > 2 {
				ALPHABET[value as usize & 0x3f] as char
			} else {
				'='
			});
		}
		encoded
	}

	fn decoded_message(message: &str) -> AppOutput {
		dispatch_output(serde_json::json!({
			"action": "decode-message",
			"selected_message": message,
			"selected_message_ref": {
				"channel_id": "10", "message_id": "20", "author_id": "30"
			}
		}))
	}

	fn copied(output: &AppOutput) -> &str {
		assert_eq!(output.effects.len(), 1);
		match &output.effects[0] {
			HostEffect::CopyText { text } => text,
			other => panic!("unexpected effect: {other:?}"),
		}
	}

	#[test]
	fn decode_replaces_only_standard_base64_that_is_valid_utf8() {
		assert_eq!(
			invoke("decode", "SGVsbG8=, d29ybGQh and test AP8A").replacement,
			Some("Hello, world! and test AP8A".into())
		);
		assert_eq!(
			invoke("decode", "padding: SGVsbG8").replacement,
			Some("padding: Hello".into())
		);
		assert_eq!(invoke("decode", "SGVsbG8_ and QQ==").replacement, None);
		let output = invoke("decode", "SGVsbG8_ and QQ==");
		assert!(matches!(
			output.panel.as_slice(),
			[Element::Text { text }] if text.contains("left unchanged")
		));
	}

	#[test]
	fn decode_preserves_oversized_drafts_without_a_proposal() {
		let oversized = "A".repeat(MAX_DRAFT_BYTES + 1);
		let output = invoke("decode", &oversized);
		assert_eq!(output.replacement, None);
		assert!(matches!(
			output.panel.as_slice(),
			[Element::Text { text }] if text.contains("8,000")
		));
	}

	#[test]
	fn uppercase_matches_upstream_sentence_and_blocked_word_semantics() {
		assert_eq!(
			invoke(
				"uppercase",
				"hello world. this is fine! \"quoted\" start? ok then\nhttp://example.test next"
			)
			.replacement,
			Some(
				"Hello world. This is fine! \"quoted\" start? ok then\nhttp://example.test next"
					.into()
			)
		);
		assert_eq!(
			invoke("uppercase", "hello...  next. one.two").replacement,
			Some("Hello...  Next. One.two".into())
		);
		assert_eq!(
			invoke("uppercase", "ßeta. straße. 🦀 crab").replacement,
			Some("SSeta. Straße. 🦀 crab".into())
		);
		assert_eq!(
			invoke("uppercase", "\"hello\" world. HTTP://X. Okay then").replacement,
			Some("\"hello\" world. HTTP://X. Okay then".into())
		);
	}

	#[test]
	fn typed_dispatch_routes_declared_actions_and_ignores_unknown_ones() {
		let bytes =
			dispatch_typed(br#"{"action":"decode","composer":"SGVsbG8="}"#, handle).unwrap();
		let output: AppOutput = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(output.output.replacement.as_deref(), Some("Hello"));

		let bytes = dispatch_typed(
			br#"{"action":"uppercase","composer":"hello. next"}"#,
			handle,
		)
		.unwrap();
		let output: AppOutput = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(output.output.replacement.as_deref(), Some("Hello. Next"));

		let bytes =
			dispatch_typed(br#"{"action":"unknown","composer":"SGVsbG8="}"#, handle).unwrap();
		assert_eq!(
			serde_json::from_slice::<AppOutput>(&bytes).unwrap(),
			AppOutput::default()
		);
	}

	#[test]
	fn message_action_proposes_one_reviewed_copy_and_never_the_source_text() {
		let single = decoded_message("look: SGVsbG8= end");
		assert_eq!(copied(&single), "Hello");
		assert!(single.output.replacement.is_none());
		assert!(single.output.storage.is_none());
		assert!(single.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("Nothing was copied before Apply")
		)));
		assert!(!serde_json::to_string(&single).unwrap().contains("look: "));

		let multiple = decoded_message("SGVsbG8= and d29ybGQh");
		assert_eq!(copied(&multiple), "Hello\n\nworld!");
		assert!(multiple.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("copies 2 decoded match")
		)));

		let invalid = decoded_message("AP8A is not UTF-8 text");
		assert!(invalid.effects.is_empty());
		let prose = decoded_message("no encoded spans here");
		assert!(prose.effects.is_empty());
		let missing = dispatch_output(serde_json::json!({"action": "decode-message"}));
		assert!(missing.effects.is_empty());
	}

	#[test]
	fn message_action_is_bounded_by_match_count_size_and_effect_bytes() {
		let many = (0..10)
			.map(|index| base64(format!("match-{index}").as_bytes()))
			.collect::<Vec<_>>()
			.join(" ");
		let output = decoded_message(&many);
		let text = copied(&output);
		assert_eq!(text.matches("match-").count(), MAX_MESSAGE_MATCHES);
		assert!(text.len() <= MAX_COPY_BYTES);
		assert!(serde_json::to_vec(&output.effects[0]).unwrap().len() <= MAX_EFFECT_BYTES);
		assert!(output.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("2 further decoded match")
		)));

		let oversized_bytes = vec![b'A'; MAX_MATCH_BYTES + 1];
		let oversized = base64(&oversized_bytes);
		// The upstream-compatible scanner requires at least four non-padding bytes.
		let mixed = decoded_message(&format!("{oversized} {small}", small = base64(b"okay")));
		assert_eq!(copied(&mixed), "okay");
		assert!(mixed.output.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("1 further decoded match")
		)));

		let all_oversized = decoded_message(&oversized);
		assert!(all_oversized.effects.is_empty());
	}
}
