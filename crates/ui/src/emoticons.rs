//! Convert standalone emoticons on submission, leaving code and embedded tokens alone.
pub(crate) fn convert(text: &str) -> String {
	let mut result = String::with_capacity(text.len());
	let mut offset = 0;
	let mut code_ticks = 0;
	while offset < text.len() {
		let tail = &text[offset..];
		if tail.starts_with('`') {
			let ticks = tail.bytes().take_while(|byte| *byte == b'`').count();
			if code_ticks == 0 {
				code_ticks = ticks;
			} else if code_ticks == ticks {
				code_ticks = 0;
			}
			result.push_str(&tail[..ticks]);
			offset += ticks;
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
		let replacement = if code_ticks == 0 {
			match token {
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
			}
		} else {
			token
		};
		result.push_str(replacement);
		offset += length;
	}
	result
}
