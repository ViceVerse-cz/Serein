/*
 * Safe Serein adaptation of the Equicord Ingtoninator plugin pinned at
 * Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e:
 *
 * Ingtoninator (src/equicordplugins/ingtoninator/index.tsx)
 * Copyright (c) 2025 Vendicated and contributors
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

const MAX_DRAFT_CHARS: usize = 2_000;
const MAX_DRAFT_BYTES: usize = 8_000;

const DRAFT_LIMIT: &str = "The draft or transformed result exceeds the 2,000-character or 8,000-UTF-8-byte limit. The draft was left unchanged.";

#[derive(Clone, Copy, Debug)]
enum Piece<'a> {
	Text(&'a str),
	Protected(&'a str),
}

#[derive(Clone, Copy, Debug)]
struct Candidate<'a> {
	end: usize,
	word: &'a str,
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn draft_within_bounds(value: &str) -> bool {
	value.len() <= MAX_DRAFT_BYTES && value.chars().count() <= MAX_DRAFT_CHARS
}

/// Return the exclusive end of a protected span beginning at `index`, if any.
///
/// Fenced and inline code, HTTP(S)/`www.` URLs and Discord angle-bracket wire
/// tokens (mentions, channels, custom emoji and timestamps) are protected. An
/// unclosed code delimiter or an unmatched `<` is ordinary text.
fn protected_end(value: &str, index: usize) -> Option<usize> {
	let rest = &value[index..];
	if rest.starts_with("```") {
		let after = &value[index + 3..];
		return after.find("```").map(|offset| index + 3 + offset + 3);
	}
	if rest.starts_with('`') {
		let after = &value[index + 1..];
		return after.find('`').map(|offset| index + 1 + offset + 1);
	}
	if rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("www.") {
		for (offset, character) in rest.char_indices() {
			if character.is_whitespace() || matches!(character, '<' | '>') {
				return (offset > 0).then_some(index + offset);
			}
		}
		return Some(value.len());
	}
	if rest.starts_with('<') {
		let mut read = 0;
		for (offset, character) in rest.char_indices().skip(1) {
			read += 1;
			if read > 100 || character == '<' || character.is_whitespace() {
				return None;
			}
			if character == '>' {
				return Some(index + offset + character.len_utf8());
			}
		}
	}
	None
}

fn pieces(value: &str) -> Vec<Piece<'_>> {
	let mut result = Vec::new();
	let mut text_start = 0;
	let mut index = 0;
	while index < value.len() {
		if let Some(end) = protected_end(value, index) {
			if text_start < index {
				result.push(Piece::Text(&value[text_start..index]));
			}
			result.push(Piece::Protected(&value[index..end]));
			index = end;
			text_start = index;
		} else {
			let character = value[index..].chars().next().expect("index is in bounds");
			index += character.len_utf8();
		}
	}
	if text_start < value.len() {
		result.push(Piece::Text(&value[text_start..]));
	}
	result
}

/// Upstream's eligibility rule: standalone `i`/`I` and words ending in a
/// vowel (`a`, `e`, `o`, `u`, `y`, compared case-insensitively) are rejected.
fn is_legal(word: &str) -> bool {
	if word == "i" || word == "I" {
		return false;
	}
	let Some(last) = word.chars().next_back() else {
		return false;
	};
	!matches!(last.to_ascii_lowercase(), 'a' | 'e' | 'o' | 'u' | 'y')
}

fn push_candidates<'a>(value: &'a str, base: usize, candidates: &mut Vec<Candidate<'a>>) {
	let mut index = 0;
	while index < value.len() {
		let character = value[index..].chars().next().expect("index is in bounds");
		if !character.is_alphabetic() {
			index += character.len_utf8();
			continue;
		}
		let start = index;
		while index < value.len() {
			let next = value[index..].chars().next().expect("index is in bounds");
			if next.is_alphabetic() {
				index += next.len_utf8();
			} else {
				break;
			}
		}
		let word = &value[start..index];
		if is_legal(word) {
			candidates.push(Candidate {
				end: base + index,
				word,
			});
		}
	}
}

/// Collect eligible letter runs only from unprotected text, so a token that
/// merely touches a URL, code span or Discord token is never selected.
fn eligible_candidates(value: &str) -> Vec<Candidate<'_>> {
	let mut candidates = Vec::new();
	let mut offset = 0;
	for piece in pieces(value) {
		match piece {
			Piece::Text(text) => {
				push_candidates(text, offset, &mut candidates);
				offset += text.len();
			}
			Piece::Protected(text) => offset += text.len(),
		}
	}
	candidates
}

/// FNV-1a over the bounded draft bytes. This is a stable selector, not a
/// cryptographic hash and not a source of randomness.
fn stable_hash(bytes: &[u8]) -> u64 {
	let mut hash = 0xcbf2_9ce4_8422_2325u64;
	for &byte in bytes {
		hash ^= u64::from(byte);
		hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
	}
	hash
}

fn choose<'a>(draft: &'a str, candidates: &[Candidate<'a>]) -> Option<Candidate<'a>> {
	if candidates.is_empty() {
		return None;
	}
	let index = (stable_hash(draft.as_bytes()) % candidates.len() as u64) as usize;
	candidates.get(index).copied()
}

/// Upstream's suffix completion, applied to an uppercase word. `INGTON` is
/// already complete; the caller inserts an empty string so the draft is kept.
fn ington_suffix(uppercase_word: &str) -> &'static str {
	if uppercase_word.ends_with("INGTON") {
		""
	} else if uppercase_word.ends_with("INGTO") {
		"N"
	} else if uppercase_word.ends_with("INGT") {
		"ON"
	} else if uppercase_word.ends_with("ING") {
		"TON"
	} else if uppercase_word.ends_with("IN") {
		"GTON"
	} else if uppercase_word.ends_with("I") {
		"NGTON"
	} else {
		"INGTON"
	}
}

fn ingtoninate(input: &Invocation) -> Output {
	let Some(draft) = input.composer.as_deref() else {
		return Output::default();
	};
	if !draft_within_bounds(draft) {
		return Output {
			panel: vec![text(DRAFT_LIMIT)],
			..Default::default()
		};
	}
	let candidates = eligible_candidates(draft);
	let Some(candidate) = choose(draft, &candidates) else {
		return Output::default();
	};

	let uppercase_word = candidate.word.to_uppercase();
	let suffix = ington_suffix(&uppercase_word);
	let insertion = if candidate.word == uppercase_word {
		suffix.to_owned()
	} else {
		suffix.to_lowercase()
	};

	let mut result = String::with_capacity(draft.len().saturating_add(insertion.len()));
	result.push_str(&draft[..candidate.end]);
	result.push_str(&insertion);
	result.push_str(&draft[candidate.end..]);

	if !draft_within_bounds(&result) {
		return Output {
			panel: vec![text(DRAFT_LIMIT)],
			..Default::default()
		};
	}
	Output {
		replacement: Some(result),
		..Default::default()
	}
}

fn handle(input: Invocation) -> Output {
	match input.action.as_str() {
		"ingtoninate" => ingtoninate(&input),
		_ => Output::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch, dispatch_typed, serde_json};

	fn invoke(composer: &str) -> Output {
		handle(Invocation {
			action: "ingtoninate".into(),
			composer: Some(composer.into()),
			..Default::default()
		})
	}

	#[test]
	fn suffix_completion_matches_upstream_table() {
		assert_eq!(ington_suffix("PINGTON"), "");
		assert_eq!(ington_suffix("PINGTO"), "N");
		assert_eq!(ington_suffix("PINGT"), "ON");
		assert_eq!(ington_suffix("PING"), "TON");
		assert_eq!(ington_suffix("PIN"), "GTON");
		assert_eq!(ington_suffix("PI"), "NGTON");
		assert_eq!(ington_suffix("P"), "INGTON");
	}

	#[test]
	fn action_appends_suffix_with_lowercase_and_uppercase_words() {
		assert_eq!(invoke("bring").replacement.as_deref(), Some("brington"));
		assert_eq!(invoke("plingt").replacement.as_deref(), Some("plington"));
		assert_eq!(invoke("pin").replacement.as_deref(), Some("pington"));
		assert_eq!(invoke("ski").replacement.as_deref(), Some("skington"));
		assert_eq!(invoke("test").replacement.as_deref(), Some("testington"));

		assert_eq!(invoke("PIN").replacement.as_deref(), Some("PINGTON"));
		assert_eq!(invoke("PLINGT").replacement.as_deref(), Some("PLINGTON"));
		assert_eq!(invoke("Pin").replacement.as_deref(), Some("Pington"));
		assert_eq!(
			invoke("washington").replacement.as_deref(),
			Some("washington")
		);
	}

	#[test]
	fn urls_code_mentions_and_emoji_are_preserved() {
		let output = invoke("https://example.test/hello `code` <@123> 😀 world");
		assert_eq!(
			output.replacement.as_deref(),
			Some("https://example.test/hello `code` <@123> 😀 worldington")
		);

		let output = invoke("`code` <#456> <:fox:789> https://example.test add");
		assert_eq!(
			output.replacement.as_deref(),
			Some("`code` <#456> <:fox:789> https://example.test addington")
		);
	}

	#[test]
	fn ineligible_drafts_propose_no_replacement() {
		for draft in [
			"i",
			"I",
			"a e o u y",
			"sky",
			"movie",
			"area",
			"https://example.test",
		] {
			let output = invoke(draft);
			assert!(output.replacement.is_none(), "{draft}");
			assert!(output.panel.is_empty(), "{draft}");
		}
	}

	#[test]
	fn selection_is_deterministic_for_the_same_draft() {
		let first = invoke("red blue green black");
		let second = invoke("red blue green black");
		assert_eq!(
			first.replacement.as_deref(),
			Some("redington blue green black")
		);
		assert_eq!(first, second);
	}

	#[test]
	fn draft_bounds_block_oversized_input() {
		let output = invoke(&"x".repeat(MAX_DRAFT_BYTES + 1));
		assert!(output.replacement.is_none());
		assert!(matches!(
			output.panel.as_slice(),
			[Element::Text { text }] if text.contains("8,000")
		));
	}

	#[test]
	fn typed_dispatch_routes_declared_action_and_ignores_unknown_ones() {
		let bytes =
			dispatch_typed(br#"{"action":"ingtoninate","composer":"pin"}"#, handle).unwrap();
		let output: Output = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(output.replacement.as_deref(), Some("pington"));

		let bytes = dispatch(br#"{"action":"unknown","composer":"pin"}"#, handle).unwrap();
		assert_eq!(
			serde_json::from_slice::<Output>(&bytes).unwrap(),
			Output::default()
		);
	}
}
