/*
 * Safe Serein adaptations of Equicord/Vencord plugins pinned at
 * Equicord/Equicord@200dba124066146a8f043fde7fc8b05e3dc5e68e:
 *
 * talkInReverse (src/equicordplugins/talkInReverse/index.tsx)
 * Copyright (c) 2023 Vendicated and contributors
 *
 * signature (src/equicordplugins/signature/index.tsx)
 * Copyright (c) 2024 Vendicated and contributors
 *
 * polishWording (src/equicordplugins/polishWording/index.ts)
 * Copyright (c) 2024 Vendicated and contributors
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

use serde::{Deserialize, Serialize};
use serein_extension_sdk::{Element, Invocation, MAX_IO_BYTES, Output};
use std::collections::{BTreeMap, BTreeSet};

const MAX_DRAFT_CHARS: usize = 2_000;
const MAX_DRAFT_BYTES: usize = 8_000;
const MAX_SIGNATURE_BYTES: usize = 512;
const MAX_BLOCKED_WORDS: usize = 64;
const MAX_BLOCKED_WORD_BYTES: usize = 64;
const MAX_BLOCKED_WORDS_BYTES: usize = 2_048;
const MAX_STORAGE_BYTES: usize = 64 * 1024;

const DRAFT_LIMIT: &str = "The draft or transformed result exceeds the 2,000-character or 8,000-UTF-8-byte limit. The draft was left unchanged.";
const CORRUPT_STORAGE: &str =
	"Saved settings are invalid or too large. Nothing was changed; original storage was preserved.";
const STORAGE_LIMIT: &str = "The settings exceed the 64 KiB storage limit. Nothing was saved.";
const SIGNATURE_LIMIT: &str = "The header and signature must compose to at most 512 UTF-8 bytes with no control characters. Nothing was saved.";
const BLOCKED_WORDS_LIMIT: &str = "Use at most 64 comma- or newline-separated blocked words, each at most 64 UTF-8 bytes and 2,048 bytes in total. Nothing was saved.";
const NO_SIGNATURE: &str =
	"No signature is configured. Open Draft Style Tools and save a signature name first.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Saved {
	signature_name: String,
	signature_header: String,
	polish_fix_apostrophes: bool,
	polish_expand_contractions: bool,
	polish_fix_capitalization: bool,
	polish_fix_punctuation: bool,
	polish_blocked_words: Vec<String>,
}

impl Default for Saved {
	fn default() -> Self {
		Self {
			signature_name: String::new(),
			signature_header: ">".into(),
			polish_fix_apostrophes: true,
			polish_expand_contractions: false,
			polish_fix_capitalization: false,
			polish_fix_punctuation: false,
			polish_blocked_words: Vec::new(),
		}
	}
}

#[derive(Clone, Debug)]
struct Fields {
	signature_name: String,
	signature_header: String,
	blocked_words: String,
	fix_apostrophes: bool,
	expand_contractions: bool,
	fix_capitalization: bool,
	fix_punctuation: bool,
}

impl Fields {
	fn from_saved(saved: &Saved) -> Self {
		Self {
			signature_name: saved.signature_name.clone(),
			signature_header: saved.signature_header.clone(),
			blocked_words: saved.polish_blocked_words.join(", "),
			fix_apostrophes: saved.polish_fix_apostrophes,
			expand_contractions: saved.polish_expand_contractions,
			fix_capitalization: saved.polish_fix_capitalization,
			fix_punctuation: saved.polish_fix_punctuation,
		}
	}

	fn from_input(input: &Invocation, saved: &Saved) -> Result<Self, &'static str> {
		let mut fields = Self::from_saved(saved);
		if let Some(value) = input.value("signature-name") {
			fields.signature_name = value.to_owned();
		}
		if let Some(value) = input.value("signature-header") {
			fields.signature_header = value.to_owned();
		}
		if let Some(value) = input.value("blocked-words") {
			fields.blocked_words = value.to_owned();
		}
		let boolean = |id: &str, target: &mut bool| -> Result<(), &'static str> {
			if let Some(value) = input.value(id) {
				*target = match value {
					"true" => true,
					"false" => false,
					_ => return Err("A checkbox value is invalid. Nothing was saved."),
				};
			}
			Ok(())
		};
		boolean("fix-apostrophes", &mut fields.fix_apostrophes)?;
		boolean("expand-contractions", &mut fields.expand_contractions)?;
		boolean("fix-capitalization", &mut fields.fix_capitalization)?;
		boolean("fix-punctuation", &mut fields.fix_punctuation)?;
		Ok(fields)
	}
}

fn parse_blocked_words(raw: &str) -> Result<Vec<String>, &'static str> {
	let mut words: Vec<String> = Vec::new();
	for item in raw.split([',', '\n']) {
		let word = item.trim();
		if word.is_empty() {
			continue;
		}
		if word.len() > MAX_BLOCKED_WORD_BYTES || word.chars().any(char::is_control) {
			return Err(BLOCKED_WORDS_LIMIT);
		}
		if !words
			.iter()
			.any(|existing| existing.to_lowercase() == word.to_lowercase())
		{
			words.push(word.to_owned());
		}
	}
	if words.len() > MAX_BLOCKED_WORDS
		|| words.iter().map(String::len).sum::<usize>() > MAX_BLOCKED_WORDS_BYTES
	{
		return Err(BLOCKED_WORDS_LIMIT);
	}
	Ok(words)
}

fn signature_block(saved: &Saved) -> Option<String> {
	let name = saved.signature_name.trim();
	if name.is_empty() {
		return None;
	}
	let header = saved.signature_header.trim();
	if header.is_empty() {
		Some(name.to_owned())
	} else {
		Some(format!("{header} {name}"))
	}
}

fn validate_saved(saved: &Saved) -> Result<(), &'static str> {
	let signature = saved.signature_name.trim();
	let header = saved.signature_header.trim();
	if signature.chars().any(char::is_control)
		|| header.chars().any(char::is_control)
		|| saved.signature_name.chars().any(char::is_control)
		|| saved.signature_header.chars().any(char::is_control)
		|| saved.signature_name.len() > MAX_SIGNATURE_BYTES
		|| saved.signature_header.len() > MAX_SIGNATURE_BYTES
	{
		return Err(CORRUPT_STORAGE);
	}
	if let Some(block) = signature_block(saved)
		&& block.len() > MAX_SIGNATURE_BYTES
	{
		return Err(CORRUPT_STORAGE);
	}
	if saved.polish_blocked_words.len() > MAX_BLOCKED_WORDS
		|| saved
			.polish_blocked_words
			.iter()
			.map(String::len)
			.sum::<usize>()
			> MAX_BLOCKED_WORDS_BYTES
	{
		return Err(CORRUPT_STORAGE);
	}
	let mut unique = BTreeSet::new();
	for word in &saved.polish_blocked_words {
		if word.is_empty()
			|| word.len() > MAX_BLOCKED_WORD_BYTES
			|| word.chars().any(char::is_control)
			|| !unique.insert(word.to_lowercase())
		{
			return Err(CORRUPT_STORAGE);
		}
	}
	Ok(())
}

fn load_saved(input: &Invocation) -> Result<Saved, &'static str> {
	let Some(raw) = input.storage.as_deref() else {
		return Ok(Saved::default());
	};
	if raw.len() > MAX_STORAGE_BYTES {
		return Err(CORRUPT_STORAGE);
	}
	let saved: Saved = serde_json::from_str(raw).map_err(|_| CORRUPT_STORAGE)?;
	validate_saved(&saved)?;
	Ok(saved)
}

fn text(value: impl Into<String>) -> Element {
	Element::Text { text: value.into() }
}

fn heading(value: impl Into<String>) -> Element {
	Element::Heading { text: value.into() }
}

fn button(id: &str, label: &str) -> Element {
	Element::Button {
		id: id.into(),
		label: label.into(),
	}
}

fn checkbox(id: &str, label: &str, checked: bool) -> Element {
	Element::Checkbox {
		id: id.into(),
		label: label.into(),
		checked,
	}
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
	if value.len() <= max_bytes {
		return value;
	}
	let mut boundary = max_bytes;
	while !value.is_char_boundary(boundary) {
		boundary -= 1;
	}
	&value[..boundary]
}

fn input(id: &str, label: &str, value: &str, max_bytes: usize) -> Element {
	Element::TextInput {
		id: id.into(),
		label: label.into(),
		value: truncate_utf8(value, max_bytes).into(),
	}
}

fn preview(value: &str, max_bytes: usize) -> String {
	let clean: String = value
		.chars()
		.map(|character| {
			if character.is_control() {
				' '
			} else {
				character
			}
		})
		.collect();
	let clipped = truncate_utf8(&clean, max_bytes);
	if clipped.len() == clean.len() {
		clean
	} else {
		format!("{clipped}…")
	}
}

fn panel(saved: &Saved, fields: &Fields, notice: &str) -> Output {
	let mut elements = vec![
		heading("Draft Style Tools"),
		text(
			"Local, deterministic draft transformations. Every composer result is a proposal that requires Apply and never sends automatically.",
		),
	];
	if !notice.is_empty() {
		elements.push(text(notice));
	}
	elements.extend([
		heading("Signature"),
		text("The signature is appended on a new line once; re-running the action does not duplicate an existing signature. Header plus name must fit in 512 UTF-8 bytes."),
		input("signature-name", "Signature name", &fields.signature_name, MAX_SIGNATURE_BYTES),
		input("signature-header", "Signature header (for example >)", &fields.signature_header, 128),
	]);
	if let Some(block) = signature_block(saved) {
		elements.push(text(format!(
			"Saved block preview: `{}`",
			preview(&block, 512)
		)));
	} else {
		elements.push(text(
			"No signature is saved; the Signature action will explain that one is required.",
		));
	}
	elements.extend([
		heading("Polish wording"),
		text("Polish edits only unprotected prose. Fenced and inline code, HTTP(S)/www URLs and Discord angle-bracket tokens are preserved exactly. It is a local deterministic port, not an AI grammar service."),
		checkbox(
			"fix-apostrophes",
			"Restore apostrophes in common contractions (dont -> don't; on by default)",
			fields.fix_apostrophes,
		),
		checkbox(
			"expand-contractions",
			"Expand common contractions (don't -> do not)",
			fields.expand_contractions,
		),
		checkbox(
			"fix-capitalization",
			"Capitalize sentence starts",
			fields.fix_capitalization,
		),
		checkbox(
			"fix-punctuation",
			"Add missing sentence periods at line ends",
			fields.fix_punctuation,
		),
		input(
			"blocked-words",
			"Words not capitalized at a sentence start (comma separated)",
			&fields.blocked_words,
			MAX_BLOCKED_WORDS_BYTES,
		),
		Element::Row {
			children: vec![button("save", "Save settings")],
		},
	]);
	Output {
		panel: elements,
		..Default::default()
	}
}

fn save_saved(saved: &Saved, fields: &Fields, notice: &str) -> Output {
	let encoded = match serde_json::to_string(saved) {
		Ok(encoded) => encoded,
		Err(_) => {
			return panel(
				saved,
				fields,
				"Could not encode settings. Nothing was saved.",
			);
		}
	};
	if encoded.len() > MAX_STORAGE_BYTES {
		return panel(saved, fields, STORAGE_LIMIT);
	}
	let mut output = panel(saved, fields, notice);
	output.storage = Some(encoded);
	match serde_json::to_vec(&output) {
		Ok(bytes) if bytes.len() <= MAX_IO_BYTES => output,
		_ => panel(
			saved,
			fields,
			"The saved response is too large. Nothing was saved.",
		),
	}
}

fn handle_panel(input: &Invocation) -> Output {
	let saved = match load_saved(input) {
		Ok(saved) => saved,
		Err(error) => {
			return panel(
				&Saved::default(),
				&Fields::from_saved(&Saved::default()),
				error,
			);
		}
	};
	if input.action == "open" {
		let fields = if input.values.is_empty() {
			Fields::from_saved(&saved)
		} else {
			match Fields::from_input(input, &saved) {
				Ok(fields) => fields,
				Err(error) => return panel(&saved, &Fields::from_saved(&saved), error),
			}
		};
		return panel(&saved, &fields, "");
	}

	let fields = match Fields::from_input(input, &saved) {
		Ok(fields) => fields,
		Err(error) => return panel(&saved, &Fields::from_saved(&saved), error),
	};
	let blocked_words = match parse_blocked_words(&fields.blocked_words) {
		Ok(words) => words,
		Err(error) => return panel(&saved, &fields, error),
	};
	let updated = Saved {
		signature_name: fields.signature_name.trim().to_owned(),
		signature_header: fields.signature_header.trim().to_owned(),
		polish_fix_apostrophes: fields.fix_apostrophes,
		polish_expand_contractions: fields.expand_contractions,
		polish_fix_capitalization: fields.fix_capitalization,
		polish_fix_punctuation: fields.fix_punctuation,
		polish_blocked_words: blocked_words,
	};
	if let Err(error) = validate_saved(&updated) {
		let notice = if error == CORRUPT_STORAGE {
			SIGNATURE_LIMIT
		} else {
			error
		};
		return panel(&saved, &fields, notice);
	}
	save_saved(&updated, &fields, "Draft style settings saved.")
}

fn draft_within_bounds(text: &str) -> bool {
	text.len() <= MAX_DRAFT_BYTES && text.chars().count() <= MAX_DRAFT_CHARS
}

#[derive(Clone, Copy, Debug)]
enum Piece<'a> {
	Text(&'a str),
	Protected(&'a str),
}

fn protected_end(text: &str, index: usize) -> Option<usize> {
	let rest = &text[index..];
	if rest.starts_with("```") {
		let after = &text[index + 3..];
		return after.find("```").map(|offset| index + 3 + offset + 3);
	}
	if rest.starts_with('`') {
		let after = &text[index + 1..];
		return after.find('`').map(|offset| index + 1 + offset + 1);
	}
	if rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("www.") {
		for (offset, character) in rest.char_indices() {
			if character.is_whitespace() || matches!(character, '<' | '>') {
				return (offset > 0).then_some(index + offset);
			}
		}
		return Some(text.len());
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

fn pieces(text: &str) -> Vec<Piece<'_>> {
	let mut result = Vec::new();
	let mut text_start = 0;
	let mut index = 0;
	while index < text.len() {
		if let Some(end) = protected_end(text, index) {
			if text_start < index {
				result.push(Piece::Text(&text[text_start..index]));
			}
			result.push(Piece::Protected(&text[index..end]));
			index = end;
			text_start = index;
		} else {
			let character = text[index..].chars().next().expect("index is in bounds");
			index += character.len_utf8();
		}
	}
	if text_start < text.len() {
		result.push(Piece::Text(&text[text_start..]));
	}
	result
}

fn reverse_action(input: &Invocation) -> Output {
	let Some(draft) = input.composer.as_deref() else {
		return Output::default();
	};
	if !draft_within_bounds(draft) {
		return Output {
			panel: vec![text(DRAFT_LIMIT)],
			..Default::default()
		};
	}
	let mut reversed = String::with_capacity(draft.len());
	for piece in pieces(draft).into_iter().rev() {
		match piece {
			Piece::Text(value) => reversed.extend(value.chars().rev()),
			Piece::Protected(value) => reversed.push_str(value),
		}
	}
	Output {
		replacement: Some(reversed),
		..Default::default()
	}
}

fn signature_action(input: &Invocation) -> Output {
	let Some(draft) = input.composer.as_deref() else {
		return Output::default();
	};
	if !draft_within_bounds(draft) {
		return Output {
			panel: vec![text(DRAFT_LIMIT)],
			..Default::default()
		};
	}
	let saved = match load_saved(input) {
		Ok(saved) => saved,
		Err(error) => {
			return Output {
				panel: vec![text(error)],
				..Default::default()
			};
		}
	};
	let Some(block) = signature_block(&saved) else {
		return Output {
			panel: vec![text(NO_SIGNATURE)],
			..Default::default()
		};
	};
	if block.len() > MAX_SIGNATURE_BYTES {
		return Output {
			panel: vec![text(SIGNATURE_LIMIT)],
			..Default::default()
		};
	}

	let trimmed = draft.trim_end_matches(char::is_whitespace);
	let already_present = trimmed == block.as_str()
		|| trimmed
			.strip_suffix(block.as_str())
			.is_some_and(|prefix| prefix.is_empty() || prefix.ends_with(char::is_whitespace));
	let transformed = if already_present {
		draft.to_owned()
	} else if draft.is_empty() {
		block
	} else if draft.ends_with('\n') {
		format!("{draft}{block}")
	} else {
		format!("{draft}\n{block}")
	};
	if !draft_within_bounds(&transformed) {
		return Output {
			panel: vec![text(DRAFT_LIMIT)],
			..Default::default()
		};
	}
	Output {
		replacement: Some(transformed),
		..Default::default()
	}
}

#[derive(Debug)]
struct Masked {
	text: String,
	markers: BTreeMap<char, String>,
}

/// Replace protected spans with private-use markers absent from the draft, so
/// whole-text capitalization and line punctuation can run without touching
/// code, URLs or Discord tokens.
fn mask_protected(text: &str) -> Option<Masked> {
	let used: BTreeSet<char> = text.chars().collect();
	let mut available = (0xE000..=0xF8FF)
		.filter_map(char::from_u32)
		.filter(|character| !used.contains(character));
	let mut masked = String::with_capacity(text.len());
	let mut markers = BTreeMap::new();
	for piece in pieces(text) {
		match piece {
			Piece::Text(value) => masked.push_str(value),
			Piece::Protected(value) => {
				let marker = available.next()?;
				markers.insert(marker, value.to_owned());
				masked.push(marker);
			}
		}
	}
	Some(Masked {
		text: masked,
		markers,
	})
}

fn restore_masked(masked: &str, markers: &BTreeMap<char, String>) -> String {
	if markers.is_empty() {
		return masked.to_owned();
	}
	let mut restored = String::with_capacity(masked.len());
	for character in masked.chars() {
		if let Some(value) = markers.get(&character) {
			restored.push_str(value);
		} else {
			restored.push(character);
		}
	}
	restored
}

const CONTRACTIONS: [(&str, &str); 43] = [
	("wasn't", "was not"),
	("can't", "cannot"),
	("don't", "do not"),
	("won't", "will not"),
	("isn't", "is not"),
	("aren't", "are not"),
	("haven't", "have not"),
	("hasn't", "has not"),
	("hadn't", "had not"),
	("doesn't", "does not"),
	("didn't", "did not"),
	("shouldn't", "should not"),
	("wouldn't", "would not"),
	("couldn't", "could not"),
	("that's", "that is"),
	("what's", "what is"),
	("there's", "there is"),
	("how's", "how is"),
	("where's", "where is"),
	("when's", "when is"),
	("who's", "who is"),
	("why's", "why is"),
	("you'll", "you will"),
	("i'll", "I will"),
	("they'll", "they will"),
	("it'll", "it will"),
	("i'm", "I am"),
	("you're", "you are"),
	("they're", "they are"),
	("he's", "he is"),
	("she's", "she is"),
	("i've", "I have"),
	("you've", "you have"),
	("we've", "we have"),
	("they've", "they have"),
	("you'd", "you would"),
	("he'd", "he would"),
	("she'd", "she would"),
	("it'd", "it would"),
	("we'd", "we would"),
	("they'd", "they would"),
	("y'all", "you all"),
	("here's", "here is"),
];

fn lowercase_without_apostrophes(value: &str) -> String {
	value
		.chars()
		.filter(|character| *character != '\'')
		.flat_map(char::to_lowercase)
		.collect()
}

fn case_flags(source: &str) -> Vec<bool> {
	source
		.chars()
		.filter(char::is_ascii_alphabetic)
		.map(|character| character.is_ascii_uppercase())
		.collect()
}

fn restore_case(replacement: &str, flags: &[bool]) -> String {
	let mut result = String::with_capacity(replacement.len());
	let mut index = 0;
	for character in replacement.chars() {
		if !character.is_ascii_alphabetic() {
			result.push(character);
			continue;
		}
		let uppercase = flags
			.get(index)
			.copied()
			.unwrap_or_else(|| flags.last().copied().unwrap_or(false));
		result.push(if uppercase {
			character.to_ascii_uppercase()
		} else {
			character.to_ascii_lowercase()
		});
		if index < flags.len().saturating_sub(1) {
			index += 1;
		}
	}
	result
}

fn is_word_byte(character: char) -> bool {
	character.is_ascii_alphanumeric() || character == '_'
}

fn map_words(text: &str, mut lookup: impl FnMut(&str) -> Option<String>) -> String {
	let mut result = String::with_capacity(text.len());
	let mut index = 0;
	while index < text.len() {
		let character = text[index..].chars().next().expect("index is in bounds");
		if !character.is_ascii_alphabetic() && character != '\'' {
			result.push(character);
			index += character.len_utf8();
			continue;
		}
		let start = index;
		while index < text.len() {
			let next = text[index..].chars().next().expect("index is in bounds");
			if next.is_ascii_alphabetic() || next == '\'' {
				index += next.len_utf8();
			} else {
				break;
			}
		}
		let run = &text[start..index];
		let previous = text[..start].chars().next_back();
		let following = text[index..].chars().next();
		if previous.is_some_and(is_word_byte) || following.is_some_and(is_word_byte) {
			result.push_str(run);
			continue;
		}
		let leading = run.len() - run.trim_start_matches('\'').len();
		let trailing = run.len() - run.trim_end_matches('\'').len();
		if leading + trailing >= run.len() {
			result.push_str(run);
			continue;
		}
		let core = &run[leading..run.len() - trailing];
		let Some(replacement) = lookup(&core.to_ascii_lowercase()) else {
			result.push_str(run);
			continue;
		};
		result.push_str(&run[..leading]);
		result.push_str(&restore_case(&replacement, &case_flags(core)));
		result.push_str(&run[run.len() - trailing..]);
	}
	result
}

fn ensure_apostrophes(text: &str) -> String {
	let missing: Vec<(String, &str)> = CONTRACTIONS
		.iter()
		.map(|(contraction, _)| (lowercase_without_apostrophes(contraction), *contraction))
		.collect();
	map_words(text, |word| {
		missing
			.iter()
			.find(|(from, _)| from.as_str() == word)
			.map(|(_, to)| (*to).to_owned())
	})
}

fn expand_contractions(text: &str) -> String {
	map_words(text, |word| {
		CONTRACTIONS
			.iter()
			.find(|(from, _)| *from == word)
			.map(|(_, to)| (*to).to_owned())
	})
}

fn first_word(part: &str) -> String {
	part.trim_start()
		.chars()
		.take_while(|character| {
			character.is_ascii_alphanumeric() || matches!(character, '_' | '\'' | '-')
		})
		.collect::<String>()
		.to_lowercase()
}

fn uppercase_first_non_space(part: &str) -> String {
	let Some(offset) = part
		.char_indices()
		.find_map(|(offset, character)| (!character.is_whitespace()).then_some(offset))
	else {
		return part.to_owned();
	};
	let character = part[offset..].chars().next().expect("offset is a boundary");
	let mut result = String::with_capacity(part.len() + 4);
	result.push_str(&part[..offset]);
	result.extend(character.to_uppercase());
	result.push_str(&part[offset + character.len_utf8()..]);
	result
}

fn capitalize_part(part: &str, blocked: &[String]) -> String {
	if part.trim().is_empty() || part.starts_with("http") {
		return part.to_owned();
	}
	let word = first_word(part);
	if !word.is_empty() && blocked.iter().any(|blocked| blocked == &word) {
		return part.to_owned();
	}
	uppercase_first_non_space(part)
}

fn split_sentence_boundary(text: &str, index: usize) -> Option<usize> {
	// `index` points at `.`, `!` or `?`. Return the index after the following
	// whitespace run when this punctuation ends a sentence.
	let mut cursor = index;
	let mut punctuation = Vec::new();
	while let Some(character) = text[cursor..].chars().next() {
		if matches!(character, '.' | '!' | '?') {
			punctuation.push(character);
			cursor += character.len_utf8();
		} else {
			break;
		}
	}
	if punctuation.is_empty() {
		return None;
	}
	if punctuation.len() > 1 && punctuation.iter().all(|character| *character == '.') {
		return None;
	}
	if punctuation.len() == 1 && punctuation[0] == '.' {
		let before = &text[..index];
		if before.ends_with('.') {
			return None;
		}
		let previous: Vec<char> = before.chars().collect();
		let last = previous.last().copied();
		// Upstream guards against `e.g.`, `Mr.` and `U.S.` style abbreviations.
		if previous.len() >= 3
			&& last.is_some_and(is_word_byte)
			&& previous[previous.len() - 2] == '.'
			&& is_word_byte(previous[previous.len() - 3])
		{
			return None;
		}
		if previous.len() >= 2
			&& previous[previous.len() - 2].is_ascii_uppercase()
			&& last.is_some_and(|character| character.is_ascii_lowercase())
		{
			return None;
		}
		if last.is_some_and(|character| character.is_ascii_uppercase()) {
			return None;
		}
	}
	let mut end = cursor;
	while let Some(character) = text[end..].chars().next() {
		if character.is_whitespace() {
			end += character.len_utf8();
		} else {
			break;
		}
	}
	(end > cursor).then_some(end)
}

fn uppercase_standalone_i(text: &str) -> String {
	let mut result = String::with_capacity(text.len());
	let mut index = 0;
	while index < text.len() {
		let character = text[index..].chars().next().expect("index is in bounds");
		if character != 'i' || text[..index].chars().next_back().is_some_and(is_word_byte) {
			result.push(character);
			index += character.len_utf8();
			continue;
		}
		let after = &text[index + 1..];
		let Some(next) = after.chars().next() else {
			result.push('I');
			index += 1;
			continue;
		};
		if next == '\'' {
			result.push('I');
			index += 1;
			continue;
		}
		if next.is_whitespace() {
			let rest = after.trim_start();
			if !(rest.starts_with("is")
				&& rest.get(2..).is_none_or(|tail| {
					tail.chars()
						.next()
						.is_none_or(|character| !is_word_byte(character))
				})) {
				result.push('I');
				index += 1;
				continue;
			}
		}
		result.push(character);
		index += character.len_utf8();
	}
	result
}

fn capitalize_sentences(text: &str, blocked: &[String]) -> String {
	let mut result = String::with_capacity(text.len());
	let mut segment_start = 0;
	let mut index = 0;
	while index < text.len() {
		let character = text[index..].chars().next().expect("index is in bounds");
		if character == '\n' {
			let mut end = index;
			while text[end..].starts_with('\n') {
				end += 1;
			}
			result.push_str(&capitalize_part(&text[segment_start..index], blocked));
			result.push_str(&text[index..end]);
			segment_start = end;
			index = end;
			continue;
		}
		if matches!(character, '.' | '!' | '?')
			&& let Some(end) = split_sentence_boundary(text, index)
		{
			result.push_str(&capitalize_part(&text[segment_start..index], blocked));
			result.push_str(&text[index..end]);
			segment_start = end;
			index = end;
			continue;
		}
		index += character.len_utf8();
	}
	result.push_str(&capitalize_part(&text[segment_start..], blocked));
	uppercase_standalone_i(&result)
}

fn ends_with_url(line: &str) -> bool {
	let tail = line.split_whitespace().next_back().unwrap_or(line);
	tail.starts_with("http://") || tail.starts_with("https://") || tail.starts_with("www.")
}

fn add_periods(text: &str) -> String {
	let lines: Vec<&str> = text.split('\n').collect();
	let mut processed = Vec::with_capacity(lines.len());
	for (index, line) in lines.iter().enumerate() {
		let stripped = line.trim_end();
		if stripped.is_empty() {
			if index + 1 < lines.len() {
				processed.push(String::new());
			}
		} else if stripped
			.chars()
			.next_back()
			.is_some_and(|character| character.is_ascii_alphanumeric())
			&& !ends_with_url(stripped)
		{
			processed.push(format!("{stripped}."));
		} else {
			processed.push(stripped.to_owned());
		}
	}
	processed.join("\n")
}

fn polish_text(text: &str, saved: &Saved) -> String {
	let Some(masked) = mask_protected(text) else {
		return text.to_owned();
	};
	let mut result = masked.text;
	if saved.polish_fix_apostrophes || saved.polish_expand_contractions {
		result = ensure_apostrophes(&result);
	}
	if saved.polish_fix_capitalization {
		let blocked: Vec<String> = saved
			.polish_blocked_words
			.iter()
			.map(|word| word.to_lowercase())
			.collect();
		result = capitalize_sentences(&result, &blocked);
	}
	if saved.polish_fix_punctuation {
		result = add_periods(&result);
	}
	if saved.polish_expand_contractions {
		result = expand_contractions(&result);
	}
	restore_masked(&result, &masked.markers)
}

fn polish_action(input: &Invocation) -> Output {
	let Some(draft) = input.composer.as_deref() else {
		return Output::default();
	};
	if !draft_within_bounds(draft) {
		return Output {
			panel: vec![text(DRAFT_LIMIT)],
			..Default::default()
		};
	}
	let saved = match load_saved(input) {
		Ok(saved) => saved,
		Err(error) => {
			return Output {
				panel: vec![text(error)],
				..Default::default()
			};
		}
	};
	let transformed = polish_text(draft, &saved);
	if !draft_within_bounds(&transformed) {
		return Output {
			panel: vec![text(DRAFT_LIMIT)],
			..Default::default()
		};
	}
	Output {
		replacement: Some(transformed),
		..Default::default()
	}
}

fn handle(input: Invocation) -> Output {
	match input.action.as_str() {
		"open" | "save" => handle_panel(&input),
		"reverse" => reverse_action(&input),
		"signature" => signature_action(&input),
		"polish" => polish_action(&input),
		_ => Output::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch, dispatch_typed, serde_json};

	fn saved(storage: &Saved) -> String {
		serde_json::to_string(storage).unwrap()
	}

	fn invoke(action: &str, storage: &Saved, composer: &str) -> Output {
		handle(Invocation {
			action: action.into(),
			composer: Some(composer.into()),
			storage: Some(saved(storage)),
			..Default::default()
		})
	}

	#[test]
	fn reverse_uses_unicode_scalars_and_protects_tokens_urls_and_code() {
		let output = invoke(
			"reverse",
			&Saved::default(),
			"a🦀 b <@123> https://example.test `x`",
		);
		assert_eq!(
			output.replacement.as_deref(),
			Some("`x` https://example.test <@123> b 🦀a")
		);
		let output = invoke("reverse", &Saved::default(), "😀ab");
		assert_eq!(output.replacement.as_deref(), Some("ba😀"));
	}

	#[test]
	fn signature_appends_once_with_spacing_and_is_idempotent() {
		let saved = Saved {
			signature_name: "Ada".into(),
			signature_header: ">".into(),
			..Default::default()
		};
		let first = invoke("signature", &saved, "hello");
		assert_eq!(first.replacement.as_deref(), Some("hello\n> Ada"));
		let second = invoke("signature", &saved, first.replacement.as_deref().unwrap());
		assert_eq!(second.replacement.as_deref(), Some("hello\n> Ada"));
		let third = invoke("signature", &saved, "hello\n> Ada \n");
		assert_eq!(third.replacement.as_deref(), Some("hello\n> Ada \n"));
		let empty = invoke("signature", &saved, "");
		assert_eq!(empty.replacement.as_deref(), Some("> Ada"));
	}

	#[test]
	fn signature_rejects_corrupt_and_oversized_storage() {
		let mut oversized = Saved {
			signature_name: "x".repeat(MAX_SIGNATURE_BYTES + 1),
			signature_header: ">".into(),
			..Default::default()
		};
		let output = invoke("signature", &oversized, "hello");
		assert!(output.replacement.is_none());
		assert!(matches!(
			output.panel.as_slice(),
			[Element::Text { text }] if text.contains("invalid or too large")
		));
		oversized.signature_name.clear();
		let corrupt = handle(Invocation {
			action: "save".into(),
			storage: Some("{broken".into()),
			..Default::default()
		});
		assert!(corrupt.storage.is_none());
		assert!(corrupt.panel.iter().any(|element| matches!(
			element,
			Element::Text { text } if text.contains("invalid or too large")
		)));
	}

	#[test]
	fn polish_preserves_code_mentions_urls_and_edits_prose_deterministically() {
		let saved = Saved {
			polish_fix_apostrophes: true,
			polish_expand_contractions: true,
			polish_fix_capitalization: true,
			polish_fix_punctuation: true,
			..Default::default()
		};
		let output = invoke(
			"polish",
			&saved,
			"hello `dont.` <@42> https://example.test/dont. i dont know\nnext line",
		);
		assert_eq!(
			output.replacement.as_deref(),
			Some("Hello `dont.` <@42> https://example.test/dont. I do not know.\nNext line.")
		);
	}

	#[test]
	fn polish_keeps_blocked_words_and_fenced_code_unchanged() {
		let saved = Saved {
			polish_fix_capitalization: true,
			polish_fix_punctuation: true,
			polish_blocked_words: vec!["ok".into()],
			..Default::default()
		};
		let output = invoke("polish", &saved, "ok fine. end ```code. dont```");
		assert_eq!(
			output.replacement.as_deref(),
			Some("ok fine. End ```code. dont```")
		);
	}

	#[test]
	fn draft_bounds_block_all_composer_actions() {
		let oversized = "x".repeat(MAX_DRAFT_BYTES + 1);
		for action in ["reverse", "signature", "polish"] {
			let output = invoke(action, &Saved::default(), &oversized);
			assert!(output.replacement.is_none(), "{action}");
			assert!(matches!(
				output.panel.as_slice(),
				[Element::Text { text }] if text.contains("8,000")
			));
		}
	}

	#[test]
	fn panel_ids_are_host_compatible_and_values_use_hyphen_ids() {
		// Mirrors crates/extensions/src/lib.rs `valid_id`.
		fn host_valid_id(value: &str) -> bool {
			!value.is_empty()
				&& value.len() <= 64
				&& value
					.bytes()
					.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
				&& value.as_bytes()[0].is_ascii_alphanumeric()
				&& !matches!(
					value,
					"con"
						| "prn" | "aux" | "nul"
						| "com1" | "com2" | "com3"
						| "com4" | "com5" | "com6"
						| "com7" | "com8" | "com9"
						| "lpt1" | "lpt2" | "lpt3"
						| "lpt4" | "lpt5" | "lpt6"
						| "lpt7" | "lpt8" | "lpt9"
				)
		}

		fn collect_ids(elements: &[Element], ids: &mut Vec<String>) {
			for element in elements {
				match element {
					Element::Row { children } => collect_ids(children, ids),
					Element::Button { id, .. }
					| Element::TextInput { id, .. }
					| Element::Checkbox { id, .. }
					| Element::Select { id, .. }
					| Element::Slider { id, .. } => ids.push(id.clone()),
					_ => {}
				}
			}
		}

		let opened = handle(Invocation {
			action: "open".into(),
			..Default::default()
		});
		let mut ids = Vec::new();
		collect_ids(&opened.panel, &mut ids);
		assert!(ids.contains(&"signature-name".to_owned()));
		assert!(ids.contains(&"blocked-words".to_owned()));
		let mut unique = BTreeSet::new();
		for id in ids {
			assert!(host_valid_id(&id), "invalid panel ID: {id}");
			assert!(unique.insert(id), "duplicate panel ID");
		}

		let saved = handle(Invocation {
			action: "save".into(),
			values: BTreeMap::from([
				("signature-name".into(), "Ada".into()),
				("signature-header".into(), ">".into()),
				("blocked-words".into(), "ok, fine".into()),
				("fix-apostrophes".into(), "false".into()),
				("expand-contractions".into(), "true".into()),
				("fix-capitalization".into(), "true".into()),
				("fix-punctuation".into(), "true".into()),
			]),
			..Default::default()
		});
		let stored: Saved = serde_json::from_str(saved.storage.as_deref().unwrap()).unwrap();
		assert_eq!(stored.signature_name, "Ada");
		assert_eq!(
			stored.polish_blocked_words,
			vec!["ok".to_owned(), "fine".to_owned()]
		);
		assert!(!stored.polish_fix_apostrophes);
		assert!(stored.polish_expand_contractions);
	}

	#[test]
	fn typed_dispatch_routes_declared_actions_and_ignores_unknown_ones() {
		let bytes = dispatch_typed(br#"{"action":"reverse","composer":"ab"}"#, handle).unwrap();
		let output: Output = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(output.replacement.as_deref(), Some("ba"));

		let storage = saved(&Saved {
			signature_name: "Ada".into(),
			signature_header: ">".into(),
			..Default::default()
		});
		let input = serde_json::json!({
			"action": "signature",
			"composer": "hello",
			"storage": storage,
		});
		let bytes = dispatch(input.to_string().as_bytes(), handle).unwrap();
		let output: Output = serde_json::from_slice(&bytes).unwrap();
		assert_eq!(output.replacement.as_deref(), Some("hello\n> Ada"));

		let bytes = dispatch(br#"{"action":"unknown","composer":"hello"}"#, handle).unwrap();
		assert_eq!(
			serde_json::from_slice::<Output>(&bytes).unwrap(),
			Output::default()
		);
	}
}
