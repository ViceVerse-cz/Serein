//! Convert standalone emoticons on submission, leaving code and embedded tokens alone.
/// Replaces supported standalone tokens while preserving escaped markers, URLs and code.
pub(crate) fn convert(text: &str) -> String {
	// Index matching runs once: unmatched inline ticks are literal, and repeated
	// unmatched runs must not turn delimiter lookahead into a quadratic scan.
	let mut runs = Vec::new();
	let mut offset = 0;
	while let Some(at) = text[offset..].find('`') {
		let start = offset + at;
		let end = start
			+ text[start..]
				.bytes()
				.take_while(|byte| *byte == b'`')
				.count();
		runs.push((start, end, None));
		offset = end;
	}
	let mut next = std::collections::HashMap::new();
	for (start, end, closing) in runs.iter_mut().rev() {
		*closing = next.insert(*end - *start, *end);
	}
	let mut runs = runs.into_iter().peekable();
	let mut result = String::with_capacity(text.len());
	let mut offset = 0;
	while offset < text.len() {
		let tail = &text[offset..];
		if tail.starts_with('`') {
			while runs.peek().is_some_and(|(start, _, _)| *start < offset) {
				runs.next();
			}
			let (start, end, closing) = runs.next().unwrap();
			let escaped = text[..start]
				.bytes()
				.rev()
				.take_while(|byte| *byte == b'\\')
				.count() % 2 == 1;
			let end = if !escaped && (end - start >= 3 || closing.is_some()) {
				// Backslashes inside code are literal, so even an escaped-looking
				// matching run closes it. Unclosed fences protect through EOF.
				closing.unwrap_or(text.len())
			} else {
				end
			};
			result.push_str(&text[start..end]);
			offset = end;
			continue;
		}
		let first = tail.chars().next().unwrap();
		if first.is_whitespace() {
			result.push(first);
			offset += first.len_utf8();
			continue;
		}
		let length = tail
			.find(|ch: char| ch.is_whitespace() || ch == '`')
			.unwrap_or(tail.len());
		let token = &tail[..length];
		let replacement = match token {
			":)" | ":-)" => "🙂",
			":(" | ":-(" => "🙁",
			";)" | ";-)" => "😉",
			":D" | ":-D" => "😃",
			":P" | ":p" | ":-P" | ":-p" => "😛",
			":o" | ":O" | ":-o" | ":-O" => "😮",
			":/" | ":-/" => "😕",
			":'(" => "😢",
			"<3" => "❤️",
			_ => token,
		};
		result.push_str(replacement);
		offset += length;
	}
	result
}

#[cfg(test)]
mod tests {
	use super::convert;

	#[test]
	fn converts_standalone_emoticons_and_preserves_embedded_tokens() {
		assert_eq!(
			convert("Ahoj :) :-) :( :-( ;) ;-) :D :-D :P :p :-P :-p :o :O :-o :-O :/ :-/ :'( <3"),
			"Ahoj 🙂 🙂 🙁 🙁 😉 😉 😃 😃 😛 😛 😛 😛 😮 😮 😮 😮 😕 😕 😢 ❤️"
		);
		assert_eq!(convert("日本語\u{2003}:)\n<3"), "日本語\u{2003}🙂\n❤️");
		let protected = "https://example.test/:) <:smile:123> <a:wave:456> \\:) word:) :)word";
		assert_eq!(convert(protected), protected);
	}

	#[test]
	fn escaped_and_unmatched_inline_ticks_remain_literal() {
		assert_eq!(convert(r"literal \` :)"), r"literal \` 🙂");
		assert_eq!(convert(r"literal \\\` :)"), r"literal \\\` 🙂");
		assert_eq!(convert("literal ` :)"), "literal ` 🙂");
		assert_eq!(convert("literal `` :) ` :D"), "literal `` 🙂 ` 😃");
		assert_eq!(convert(r"\\` :) ` :)"), r"\\` :) ` 🙂");
	}

	#[test]
	fn matched_inline_code_keeps_backslashes_literal() {
		assert_eq!(
			convert("`:D` :) `` :) ` :D `` ;)"),
			"`:D` 🙂 `` :) ` :D `` 😉"
		);
		assert_eq!(convert(r"` :) \` :)"), r"` :) \` 🙂");
	}

	#[test]
	fn fences_protect_code_and_allow_text_after_same_line_closing() {
		assert_eq!(convert("```js\n:)``` :)"), "```js\n:)``` 🙂");
		assert_eq!(convert("``` :) ``` :D"), "``` :) ``` 😃");
		let unclosed = "```js\n:)\n` :D";
		assert_eq!(convert(unclosed), unclosed);
	}
}
