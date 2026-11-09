/*
 * Vencord, a Discord client mod
 * Copyright (c) Vendicated and contributors
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Changed 2026-11-08: safe, user-invoked Rust port for Serein composer actions.
 */

use serein_extension_sdk::{Element, Invocation, Output};
use url::Url;

const TRACKING_KEYS: &[&str] = &[
	"fbclid", "gclid", "dclid", "msclkid", "mc_cid", "mc_eid", "igshid", "ref_src", "spm", "scm",
];

fn handle(input: Invocation) -> Output {
	if input.action == "character-count" {
		let counts = Counts::new(input.composer.as_deref().unwrap_or(""));
		return Output {
			panel: vec![
				Element::Heading {
					text: "Character count".into(),
				},
				Element::Text {
					text: format!(
						"Unicode scalar values: {}\nUTF-8 bytes: {}\nUTF-16 code units: {}\nWhitespace-delimited words: {}",
						counts.scalars, counts.utf8_bytes, counts.utf16_units, counts.words
					),
				},
			],
			..Default::default()
		};
	}
	let transform: fn(&str) -> String = match input.action.as_str() {
		"clear-urls" => clear_urls,
		"unindent" => unindent,
		"convert-units" => convert_units,
		"transform-text" => transform_text,
		_ => return Output::default(),
	};
	text_replacement(input.composer, transform)
}

fn text_replacement(composer: Option<String>, transform: fn(&str) -> String) -> Output {
	let Some(original) = composer else {
		return Output::default();
	};
	let transformed = transform(&original);
	// A small expansion always remains valid after worst-case JSON escaping. For larger
	// inputs, never return a larger allocation than the already bounded invocation.
	let limit = original.len().max(serein_extension_sdk::MAX_IO_BYTES / 8);
	Output {
		replacement: Some(if transformed.len() <= limit {
			transformed
		} else {
			original
		}),
		..Default::default()
	}
}

fn clear_urls(text: &str) -> String {
	let mut output = String::with_capacity(text.len());
	let mut copied_through = 0;
	while let Some(start) = find_url_start(text, copied_through) {
		output.push_str(&text[copied_through..start]);
		let raw_end = url_candidate_end(text, start);
		let end = trimmed_url_end(text, start, raw_end);
		let raw = &text[start..end];
		if let Some(cleaned) = clear_url(raw) {
			output.push_str(&cleaned);
		} else {
			output.push_str(raw);
		}
		// Delimiters trimmed from the candidate remain in the source and are copied next.
		copied_through = end;
	}
	output.push_str(&text[copied_through..]);
	output
}

fn find_url_start(text: &str, from: usize) -> Option<usize> {
	for (offset, character) in text[from..].char_indices() {
		if character != 'h' && character != 'H' {
			continue;
		}
		let start = from + offset;
		let rest = &text[start..];
		let is_http = rest
			.get(..7)
			.is_some_and(|s| s.eq_ignore_ascii_case("http://"));
		let is_https = rest
			.get(..8)
			.is_some_and(|s| s.eq_ignore_ascii_case("https://"));
		if !(is_http || is_https) {
			continue;
		}
		let boundary = text[..start]
			.chars()
			.next_back()
			.is_none_or(|previous| !previous.is_alphanumeric() && !matches!(previous, '_' | '.'));
		if boundary {
			return Some(start);
		}
	}
	None
}

fn url_candidate_end(text: &str, start: usize) -> usize {
	// A URL that begins a Markdown link target ends at the parenthesis that closes
	// the link, not at the first whitespace. Balanced parentheses inside the URL
	// stay part of it, and a Markdown-escaped `\(` or `\)` never changes the depth.
	let markdown_link = text[..start].ends_with("](");
	let mut depth = 0usize;
	let mut escaped = false;
	let mut end = start;
	for (offset, character) in text[start..].char_indices() {
		if character.is_whitespace() || matches!(character, '<' | '>' | '"' | '`') {
			break;
		}
		if markdown_link {
			match character {
				_ if escaped => escaped = false,
				'\\' => escaped = true,
				'(' => depth += 1,
				')' if depth == 0 => break,
				')' => depth -= 1,
				_ => {}
			}
		}
		end = start + offset + character.len_utf8();
	}
	end
}

fn trimmed_url_end(text: &str, start: usize, mut end: usize) -> usize {
	let bytes = &text.as_bytes()[start..end];
	let mut parens = bracket_counts(bytes, b'(', b')');
	let mut brackets = bracket_counts(bytes, b'[', b']');
	let mut braces = bracket_counts(bytes, b'{', b'}');
	while end > start {
		let character = text[start..end]
			.chars()
			.next_back()
			.expect("nonempty URL candidate");
		// A Markdown-escaped closer belongs to the URL; an even run of backslashes escapes
		// only itself, so `\\\\)` still ends with a real closer.
		let escaped_closer = matches!(character, ')' | ']' | '}')
			&& text.as_bytes()[start..end - 1]
				.iter()
				.rev()
				.take_while(|&&byte| byte == b'\\')
				.count() % 2 == 1;
		let trim = match character {
			_ if escaped_closer => false,
			'.' | ',' | ';' | ':' | '!' | '?' | '\'' => true,
			')' => trim_closer(&mut parens),
			']' => trim_closer(&mut brackets),
			'}' => trim_closer(&mut braces),
			_ => false,
		};
		if !trim {
			break;
		}
		end -= character.len_utf8();
	}
	end
}

fn bracket_counts(bytes: &[u8], opener: u8, closer: u8) -> (usize, usize) {
	bytes.iter().fold((0, 0), |(open, close), &byte| {
		(
			open + usize::from(byte == opener),
			close + usize::from(byte == closer),
		)
	})
}

fn trim_closer(counts: &mut (usize, usize)) -> bool {
	if counts.1 <= counts.0 {
		return false;
	}
	counts.1 -= 1;
	true
}

fn clear_url(raw: &str) -> Option<String> {
	let parsed = Url::parse(raw).ok()?;
	if !matches!(parsed.scheme(), "http" | "https")
		|| parsed.host_str().is_none()
		|| parsed.query().is_none()
	{
		return None;
	}
	let query_start = raw.find('?')?;
	let query_end = raw[query_start + 1..]
		.find('#')
		.map_or(raw.len(), |offset| query_start + 1 + offset);
	let mut kept = Vec::new();
	let mut removed = false;
	for pair in raw[query_start + 1..query_end].split('&') {
		if tracking_pair(pair) {
			removed = true;
		} else {
			kept.push(pair);
		}
	}
	if !removed {
		return None;
	}

	// Rebuild only the raw query. Url performed standards-based validation and
	// form decoding, while untouched parameter encodings and fragments stay exact.
	let mut output = String::with_capacity(raw.len());
	output.push_str(&raw[..query_start]);
	if kept.iter().any(|pair| !pair.is_empty()) {
		output.push('?');
		output.push_str(&kept.join("&"));
	}
	output.push_str(&raw[query_end..]);
	Some(output)
}

fn tracking_pair(pair: &str) -> bool {
	url::form_urlencoded::parse(pair.as_bytes())
		.next()
		.is_some_and(|(key, _)| {
			let key = key.to_ascii_lowercase();
			key.starts_with("utm_") || TRACKING_KEYS.contains(&key.as_str())
		})
}

fn unindent(text: &str) -> String {
	let lines: Vec<&str> = text.split_inclusive('\n').collect();
	let mut output = String::with_capacity(text.len());
	let mut unchanged_through = 0;
	let mut index = 0;
	while index < lines.len() {
		let Some((marker, fence_length)) = opening_fence(lines[index]) else {
			index += 1;
			continue;
		};
		let Some(close) =
			(index + 1..lines.len()).find(|&line| closing_fence(lines[line], marker, fence_length))
		else {
			// Preserve an unmatched opener and everything following it byte for byte.
			break;
		};
		for line in &lines[unchanged_through..=index] {
			output.push_str(line);
		}
		let body: Vec<String> = lines[index + 1..close]
			.iter()
			.map(|line| line.replace('\t', "    "))
			.collect();
		let common_indent = body
			.iter()
			.filter_map(|line| {
				let content = without_line_ending(line);
				(!content.trim_matches(' ').is_empty()).then(|| leading_spaces(content))
			})
			.min()
			.unwrap_or(0);
		for line in &body {
			let removable = leading_spaces(line).min(common_indent);
			output.push_str(&line[removable..]);
		}
		output.push_str(lines[close]);
		unchanged_through = close + 1;
		index = close + 1;
	}
	for line in &lines[unchanged_through..] {
		output.push_str(line);
	}
	output
}

fn without_line_ending(line: &str) -> &str {
	let line = line.strip_suffix('\n').unwrap_or(line);
	line.strip_suffix('\r').unwrap_or(line)
}

fn opening_fence(line: &str) -> Option<(u8, usize)> {
	let content = without_line_ending(line).trim_start_matches([' ', '\t']);
	let bytes = content.as_bytes();
	let marker = *bytes.first()?;
	if !matches!(marker, b'`' | b'~') {
		return None;
	}
	let length = bytes.iter().take_while(|&&byte| byte == marker).count();
	// A backtick fence's info string may not contain a backtick; otherwise an
	// inline code span such as ```inline``` would be mistaken for a fence opener.
	(length >= 3 && (marker != b'`' || !content[length..].contains('`')))
		.then_some((marker, length))
}

fn closing_fence(line: &str, marker: u8, minimum_length: usize) -> bool {
	let content = without_line_ending(line).trim_start_matches([' ', '\t']);
	let bytes = content.as_bytes();
	let length = bytes.iter().take_while(|&&byte| byte == marker).count();
	length >= minimum_length && content[length..].trim_matches([' ', '\t']).is_empty()
}

fn leading_spaces(text: &str) -> usize {
	text.as_bytes()
		.iter()
		.take_while(|&&byte| byte == b' ')
		.count()
}

fn convert_units(text: &str) -> String {
	let mut output = String::with_capacity(text.len());
	let mut index = 0;
	while index < text.len() {
		if numeric_boundary(text, index)
			&& let Some((consumed, replacement)) = converted_at(text, index)
		{
			output.push_str(&replacement);
			index += consumed;
			continue;
		}
		let character = text[index..]
			.chars()
			.next()
			.expect("valid character boundary");
		output.push(character);
		index += character.len_utf8();
	}
	output
}

fn numeric_boundary(text: &str, index: usize) -> bool {
	let mut previous = text[..index].chars().rev();
	let Some(last) = previous.next() else {
		return true;
	};
	// A comma directly after a digit can only belong to the same number. Never
	// restart parsing inside a malformed or repeated group such as `1,00 km` or
	// `1,,000 km`; a comma run preceded by a digit is still part of that number.
	if last == ',' {
		return !previous
			.next()
			.is_some_and(|before| before.is_ascii_digit() || before == ',');
	}
	!last.is_alphanumeric() && !matches!(last, '_' | '.' | '+' | '-')
}

fn converted_at(text: &str, start: usize) -> Option<(usize, String)> {
	let (value, number_length) = parse_number(&text[start..])?;
	let unit_start = start + number_length;
	let after_gap = text[unit_start..]
		.char_indices()
		.find_map(|(offset, character)| {
			(!matches!(character, ' ' | '\t')).then_some(unit_start + offset)
		})
		.unwrap_or(text.len());
	let gap = &text[unit_start..after_gap];
	for source in [
		"°C", "°F", "km", "mi", "kg", "lb", "cm", "in", "m", "ft", "C", "F",
	] {
		let Some(rest) = text[after_gap..].strip_prefix(source) else {
			continue;
		};
		if rest
			.chars()
			.next()
			.is_some_and(|next| next.is_alphanumeric() || matches!(next, '_' | '°'))
		{
			continue;
		}
		let (target, converted) = converted_quantity(value, source);
		let mut replacement = format_number(converted)?;
		replacement.push_str(gap);
		replacement.push_str(target);
		return Some((after_gap - start + source.len(), replacement));
	}
	None
}

fn parse_number(text: &str) -> Option<(f64, usize)> {
	let bytes = text.as_bytes();
	let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
	let integer_start = index;
	while bytes.get(index).is_some_and(u8::is_ascii_digit) {
		index += 1;
	}
	let first_group_length = index - integer_start;
	// Accept conventional comma grouping (`1,000`, `12,345,678`). If a comma is
	// not followed by exactly three digits, parsing stops before it and the
	// boundary check prevents the trailing group from being converted alone.
	if (1..=3).contains(&first_group_length) {
		loop {
			if bytes.get(index) != Some(&b',') {
				break;
			}
			let group_start = index + 1;
			let Some(group) = bytes.get(group_start..group_start + 3) else {
				break;
			};
			if !group.iter().all(u8::is_ascii_digit) {
				break;
			}
			index = group_start + 3;
		}
	}
	let mut has_digits = index > integer_start;
	if bytes.get(index) == Some(&b'.') {
		index += 1;
		let fraction_start = index;
		while bytes.get(index).is_some_and(u8::is_ascii_digit) {
			index += 1;
		}
		has_digits |= index > fraction_start;
	}
	if !has_digits {
		return None;
	}
	let exponent_start = index;
	if matches!(bytes.get(index), Some(b'e' | b'E')) {
		index += 1;
		if matches!(bytes.get(index), Some(b'+' | b'-')) {
			index += 1;
		}
		let exponent_digits = index;
		while bytes.get(index).is_some_and(u8::is_ascii_digit) {
			index += 1;
		}
		if index == exponent_digits {
			index = exponent_start;
		}
	}
	let token = text[..index].replace(',', "");
	let value = token.parse::<f64>().ok()?;
	value.is_finite().then_some((value, index))
}

fn converted_quantity(value: f64, source: &str) -> (&'static str, f64) {
	match source {
		"C" => ("F", value * 9.0 / 5.0 + 32.0),
		"°C" => ("°F", value * 9.0 / 5.0 + 32.0),
		"F" => ("C", (value - 32.0) * 5.0 / 9.0),
		"°F" => ("°C", (value - 32.0) * 5.0 / 9.0),
		"km" => ("mi", value / 1.609_344),
		"mi" => ("km", value * 1.609_344),
		"kg" => ("lb", value * 2.204_622_621_848_775_7),
		"lb" => ("kg", value / 2.204_622_621_848_775_7),
		"cm" => ("in", value / 2.54),
		"in" => ("cm", value * 2.54),
		"m" => ("ft", value / 0.3048),
		"ft" => ("m", value * 0.3048),
		_ => unreachable!("only declared units are converted"),
	}
}

fn format_number(value: f64) -> Option<String> {
	if !value.is_finite() {
		return None;
	}
	let mut formatted = format!("{value:.2}");
	while formatted.ends_with('0') {
		formatted.pop();
	}
	if formatted.ends_with('.') {
		formatted.pop();
	}
	if formatted == "-0" {
		formatted = "0".into();
	}
	Some(formatted)
}

fn transform_text(draft: &str) -> String {
	for command in ["/mock", "/reverse", "/morse", "/wordcount"] {
		let Some(body) = command_body(draft, command) else {
			continue;
		};
		return match command {
			"/mock" => mock(body),
			"/reverse" => body.chars().rev().collect(),
			"/morse" => morse(body),
			"/wordcount" => word_count(body).to_string(),
			_ => unreachable!(),
		};
	}
	draft.to_owned()
}

fn command_body<'a>(draft: &'a str, command: &str) -> Option<&'a str> {
	let rest = draft.strip_prefix(command)?;
	if rest.is_empty() {
		return Some("");
	}
	rest.chars()
		.next()
		.is_some_and(char::is_whitespace)
		.then(|| rest.trim_start_matches(char::is_whitespace))
}

fn mock(text: &str) -> String {
	let mut output = String::with_capacity(text.len());
	let mut cased_index = 0;
	for character in text.chars() {
		if character.is_lowercase() || character.is_uppercase() {
			if cased_index % 2 == 0 {
				output.extend(character.to_lowercase());
			} else {
				output.extend(character.to_uppercase());
			}
			cased_index += 1;
		} else {
			output.push(character);
		}
	}
	output
}

fn morse(text: &str) -> String {
	let mut output = String::new();
	for (word_index, word) in text.split_whitespace().enumerate() {
		if word_index > 0 {
			output.push_str(" / ");
		}
		for (symbol_index, character) in word.chars().enumerate() {
			if symbol_index > 0 {
				output.push(' ');
			}
			if let Some(symbol) = morse_symbol(character) {
				output.push_str(symbol);
			} else {
				output.push(character);
			}
		}
	}
	output
}

fn morse_symbol(character: char) -> Option<&'static str> {
	Some(match character.to_ascii_uppercase() {
		'A' => ".-",
		'B' => "-...",
		'C' => "-.-.",
		'D' => "-..",
		'E' => ".",
		'F' => "..-.",
		'G' => "--.",
		'H' => "....",
		'I' => "..",
		'J' => ".---",
		'K' => "-.-",
		'L' => ".-..",
		'M' => "--",
		'N' => "-.",
		'O' => "---",
		'P' => ".--.",
		'Q' => "--.-",
		'R' => ".-.",
		'S' => "...",
		'T' => "-",
		'U' => "..-",
		'V' => "...-",
		'W' => ".--",
		'X' => "-..-",
		'Y' => "-.--",
		'Z' => "--..",
		'0' => "-----",
		'1' => ".----",
		'2' => "..---",
		'3' => "...--",
		'4' => "....-",
		'5' => ".....",
		'6' => "-....",
		'7' => "--...",
		'8' => "---..",
		'9' => "----.",
		'.' => ".-.-.-",
		',' => "--..--",
		'?' => "..--..",
		'\'' => ".----.",
		'!' => "-.-.--",
		'/' => "-..-.",
		'(' => "-.--.",
		')' => "-.--.-",
		'&' => ".-...",
		':' => "---...",
		';' => "-.-.-.",
		'=' => "-...-",
		'+' => ".-.-.",
		'-' => "-....-",
		'_' => "..--.-",
		'"' => ".-..-.",
		'$' => "...-..-",
		'@' => ".--.-.",
		_ => return None,
	})
}

fn word_count(text: &str) -> usize {
	text.split_whitespace().count()
}

#[derive(Debug, PartialEq, Eq)]
struct Counts {
	scalars: usize,
	utf8_bytes: usize,
	utf16_units: usize,
	words: usize,
}

impl Counts {
	fn new(text: &str) -> Self {
		Self {
			scalars: text.chars().count(),
			utf8_bytes: text.len(),
			utf16_units: text.encode_utf16().count(),
			words: word_count(text),
		}
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn url_cleaning_preserves_markdown_other_parameters_and_is_idempotent() {
		let draft = "See [docs](https://example.com/a?utm_source=news&keep=a%20b&fbclid=x#part-2), then http://example.test/?q=utm_source&gclid=x.";
		let expected = "See [docs](https://example.com/a?keep=a%20b#part-2), then http://example.test/?q=utm_source.";
		let cleaned = clear_urls(draft);
		assert_eq!(cleaned, expected);
		assert_eq!(clear_urls(&cleaned), cleaned);

		let untouched = "ftp://example.com/?utm_source=x nothttps://example.com/?utm=x https://";
		assert_eq!(clear_urls(untouched), untouched);
		assert_eq!(
			clear_urls("https://example.com/?UTM_MEDIUM=x&a=1%2F2&mc_eid=y#utm_source=z"),
			"https://example.com/?a=1%2F2#utm_source=z"
		);
	}

	#[test]
	fn url_cleaning_stops_at_markdown_link_closers_and_keeps_prose() {
		let draft = "[docs](https://example.com/?utm_source=x),next and [x](https://e.test/a_(b)?utm_source=y) tail";
		let expected = "[docs](https://example.com/),next and [x](https://e.test/a_(b)) tail";
		let cleaned = clear_urls(draft);
		assert_eq!(cleaned, expected);
		assert_eq!(clear_urls(&cleaned), cleaned);

		// An escaped parenthesis inside the target neither opens nor closes the link.
		let draft =
			r"[x](https://e.test/a_\(b?utm_source=y)next [y](https://e.test/c\)?utm_source=z) tail";
		let expected = r"[x](https://e.test/a_\(b)next [y](https://e.test/c\)) tail";
		assert_eq!(clear_urls(draft), expected);
		// An escaped backslash leaves the following closer as prose.
		assert_eq!(
			clear_urls(r"(see https://e.test/a?utm_source=x\\)"),
			r"(see https://e.test/a)"
		);

		// A long run of unmatched closers is trimmed without rescanning the whole
		// candidate for every removed byte.
		let mut draft = String::from("https://example.com/?utm_source=x");
		draft.push_str(&")".repeat(4096));
		let cleaned = clear_urls(&draft);
		assert_eq!(cleaned, format!("https://example.com/{}", ")".repeat(4096)));
		assert_eq!(clear_urls(&cleaned), cleaned);
	}

	#[test]
	fn unindent_changes_only_complete_fenced_bodies_and_preserves_crlf() {
		let draft = "Before\tprose\r\n```rust\r\n\tfn main() {\r\n\t\tprintln!(\"x\");\t\r\n\t}\r\n```\r\nAfter\r\n";
		let expected = "Before\tprose\r\n```rust\r\nfn main() {\r\n    println!(\"x\");    \r\n}\r\n```\r\nAfter\r\n";
		assert_eq!(unindent(draft), expected);

		let unmatched = "Prose\r\n~~~text\r\n\tstill indented\r\n";
		assert_eq!(unindent(unmatched), unmatched);
		assert_eq!(
			unindent("```inline```\t\nprose\t"),
			"```inline```\t\nprose\t"
		);
		// A backtick fence's info string may not contain a backtick, so this is
		// inline code followed by prose, not an opener for the later fence.
		let inline = "```inline```\n\tprose\n```\n";
		assert_eq!(unindent(inline), inline);
		// Tilde fences still accept backticks in their info string, and a fence
		// shorter than three markers is not an opener.
		assert_eq!(unindent("~~~ a`b\n\tbody\n~~~\n"), "~~~ a`b\nbody\n~~~\n");
		assert_eq!(unindent("~~\n\tbody\n~~\n"), "~~\n\tbody\n~~\n");
	}

	#[test]
	fn unit_conversion_is_bounded_and_does_not_match_substrings() {
		assert_eq!(
			convert_units("20C 68F 10 km 5mi 2kg 4lb 10cm 12in 3m 6ft 20°C"),
			"68F 20C 6.21 mi 8.05km 4.41lb 1.81kg 3.94in 30.48cm 9.84ft 1.83m 68°F"
		);
		assert_eq!(
			convert_units("1,000 km 12,345.5 m 1,000,000 kg 1,234.5 km"),
			"621.37 mi 40503.61 ft 2204622.62 lb 767.08 mi"
		);
		assert_eq!(convert_units("1,000 km, 2,000 mi"), "621.37 mi, 3218.69 km");
		for unchanged in [
			"20Celsius abc20kg 10cmx 12information NaNC 1e309m",
			"1e308kg",
			"10kg_name",
			"1,00 km",
			"1,0000 km",
			"1,000,00 km",
			"1,5 km",
			"1,,000 km",
			"1,,,000 kg",
		] {
			assert_eq!(convert_units(unchanged), unchanged);
		}
	}

	#[test]
	fn transforms_and_counts_use_explicit_unicode_semantics() {
		assert_eq!(mock("AßéÇ"), "aSSéÇ");
		assert_eq!(mock("A1b"), "a1B");
		assert_eq!(transform_text("/reverse A🦀e\u{301}"), "\u{301}e🦀A");
		assert_eq!(transform_text("/morse SOS 🦀"), "... --- ... / 🦀");
		assert_eq!(transform_text("/morse a# b"), ".- # / -...");
		assert_eq!(transform_text("/wordcount one\u{2003}two\nthree"), "3");
		assert_eq!(transform_text("/mockery abc"), "/mockery abc");
		assert_eq!(transform_text("prefix /mock abc"), "prefix /mock abc");

		let counts = Counts::new("A🦀e\u{301}\tβ γ");
		assert_eq!(
			counts,
			Counts {
				scalars: 8,
				utf8_bytes: 14,
				utf16_units: 9,
				words: 3,
			}
		);
		let output = handle(Invocation {
			action: "character-count".into(),
			composer: Some("A🦀e\u{301}\tβ γ".into()),
			..Default::default()
		});
		assert_eq!(output.replacement, None);
		assert!(matches!(
			output.panel.as_slice(),
			[Element::Heading { .. }, Element::Text { .. }]
		));
	}
}
