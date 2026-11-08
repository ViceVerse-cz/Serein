/*
 * Ported from Equicord/Vencord at commit
 * 200dba124066146a8f043de7fc8b05e3dc5e68e.
 *
 * baseDecoder:
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

use serein_extension_sdk::{Element, Invocation, Output};

// Serein's draft proposals are bounded before Apply. The UTF-8 byte bound also
// leaves room for JSON escaping in the complete 256 KiB response.
const MAX_DRAFT_BYTES: usize = 8_000;
const MAX_DRAFT_CHARS: usize = 2_000;

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

fn handle(input: Invocation) -> Output {
	match input.action.as_str() {
		"decode" => decode_action(input.composer),
		"uppercase" => uppercase_action(input.composer),
		_ => Output::default(),
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

/// Port of the useful part of DecodeBase64's finder: a maximal standard Base64
/// alphabet run, requiring at least four characters and word boundaries at its
/// alphanumeric ends. `+` and `/` may occur inside the run but not at its edges.
///
/// Decodable spans, including valid trailing `=` padding, are replaced in place.
/// Every other byte, including invalid UTF-8 candidates and URL-safe Base64, is
/// copied unchanged. Only one pass is made, as in the upstream modal.
fn decode_base64_text(text: &str) -> (String, usize) {
	let bytes = text.as_bytes();
	let mut output = String::with_capacity(text.len());
	let mut copied_through = 0;
	let mut index = 0;
	let mut decoded_count = 0;

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

		output.push_str(&text[copied_through..start]);
		output.push_str(&decoded);
		copied_through = consumed;
		index = index.max(consumed);
		decoded_count += 1;
	}

	output.push_str(&text[copied_through..]);
	(output, decoded_count)
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

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch, dispatch_typed, serde_json};

	fn invoke(action: &str, composer: &str) -> Output {
		handle(Invocation {
			action: action.into(),
			composer: Some(composer.into()),
			..Default::default()
		})
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
		let output: Output = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(output.replacement.as_deref(), Some("Hello"));

		let bytes = dispatch(
			br#"{"action":"uppercase","composer":"hello. next"}"#,
			handle,
		)
		.unwrap();
		let output: Output = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(output.replacement.as_deref(), Some("Hello. Next"));

		let bytes = dispatch(br#"{"action":"unknown","composer":"SGVsbG8="}"#, handle).unwrap();
		assert_eq!(
			serde_json::from_slice::<Output>(&bytes).unwrap(),
			Output::default()
		);
	}
}
