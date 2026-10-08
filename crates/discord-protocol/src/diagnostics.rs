//! User-copyable decode causes for bug reports. Only the schema path and the error kind are kept:
//! remote strings, numbers, map keys and identifiers are redacted before anything leaves here.
use serde::Deserialize;
use serde_path_to_error::Segment;

const MAX_CAUSE: usize = 512;

/// `None` when `bytes` decodes as `T`; otherwise where and why the first failure happened.
pub(crate) fn trace<'de, T: Deserialize<'de>>(section: &str, bytes: &'de [u8]) -> Option<String> {
	let mut json = serde_json::Deserializer::from_slice(bytes);
	let (path, error) = match serde_path_to_error::deserialize::<_, T>(&mut json) {
		Ok(_) => match json.end() {
			Ok(()) => return None,
			Err(error) => (String::new(), error),
		},
		Err(error) => (path(error.path()), error.into_inner()),
	};
	let location = format!("{section}{path}");
	let location = match location.trim_start_matches('.') {
		"" => "top level",
		location => location,
	};
	let mut cause = format!("{location}: {}", describe(&error));
	if cause.len() > MAX_CAUSE {
		let end = (0..=MAX_CAUSE)
			.rev()
			.find(|&i| cause.is_char_boundary(i))
			.unwrap_or(0);
		cause.truncate(end);
		cause.push('…');
	}
	Some(cause)
}

fn path(path: &serde_path_to_error::Path) -> String {
	let mut out = String::new();
	for segment in path.iter() {
		match segment {
			Segment::Seq { index } => out.push_str(&format!("[{index}]")),
			Segment::Map { key } | Segment::Enum { variant: key } => {
				out.push('.');
				out.push_str(if schema_name(key) { key } else { "*" });
			}
			Segment::Unknown => out.push_str(".?"),
		}
	}
	out
}

/// Field names are schema; snowflake-keyed maps and free-form keys are account data.
fn schema_name(key: &str) -> bool {
	key.len() <= 64
		&& key.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
		&& key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn describe(error: &serde_json::Error) -> String {
	let position = format!("line {}, column {}", error.line(), error.column());
	match error.classify() {
		serde_json::error::Category::Syntax => format!("malformed JSON at {position}"),
		serde_json::error::Category::Eof => format!("truncated JSON at {position}"),
		serde_json::error::Category::Io => "read failure".into(),
		serde_json::error::Category::Data => {
			let message = error.to_string();
			let message = message
				.rfind(" at line ")
				.map_or(message.as_str(), |end| &message[..end]);
			redact(message)
		}
	}
}

/// An unquoted "expected" delimiter starts the schema description. Before it, only
/// missing/duplicate schema field names survive; quoted remote values become `…`.
fn redact(message: &str) -> String {
	// Serde does not escape backticks inside unknown names, so redact the entire value.
	for kind in ["unknown variant", "unknown field"] {
		if message.starts_with(&format!("{kind} `")) {
			let suffix = ["`, there are no variants", "`, there are no fields"]
				.into_iter()
				.find(|suffix| message.ends_with(*suffix))
				.map(|suffix| &suffix[1..])
				.or_else(|| message.rfind("`, expected ").map(|at| &message[at + 1..]))
				.unwrap_or("");
			let mut out = format!("{kind} `…`{suffix}");
			out.retain(|c| !c.is_control());
			return out;
		}
	}
	let keep_first = message.starts_with("missing field") || message.starts_with("duplicate field");
	let mut out = String::with_capacity(message.len());
	let mut quoted = None;
	let mut quotes = 0;
	let mut escaped = false;
	for (at, c) in message.char_indices() {
		if quoted.is_none()
			&& (at == 0 || message[..at].ends_with(", "))
			&& message[at..].starts_with("expected ")
		{
			out.push_str(&message[at..]);
			break;
		}
		match quoted {
			Some(_) if escaped => escaped = false,
			Some(_) if c == '\\' => escaped = true,
			Some(close) if c == close => {
				quoted = None;
				out.push(c);
			}
			Some(_) => {
				if keep_first && quotes == 1 {
					out.push(c);
				}
			}
			None => {
				if c == '"' || c == '`' {
					quoted = Some(c);
					quotes += 1;
					out.push(c);
					if !(keep_first && quotes == 1) {
						out.push('…');
					}
				} else {
					out.push(c);
				}
			}
		}
	}
	out.retain(|c| !c.is_control());
	out
}

#[cfg(test)]
mod tests {
	use super::trace;
	use serde::Deserialize;

	#[derive(Deserialize)]
	#[allow(dead_code)]
	struct Channel {
		id: model::Id,
		name: u32,
	}
	#[derive(Deserialize)]
	#[allow(dead_code)]
	struct Guild {
		channels: Vec<Channel>,
		roles: std::collections::BTreeMap<String, Channel>,
	}

	#[test]
	fn reports_schema_path_without_remote_values() {
		let bytes =
			br#"{"channels":[{"id":"1","name":1},{"id":"2","name":"secret name"}],"roles":{}}"#;
		let cause = trace::<Guild>("guilds[0]", bytes).unwrap();
		assert_eq!(
			cause,
			"guilds[0].channels[1].name: invalid type: string \"…\", expected u32"
		);
	}

	#[test]
	fn expected_words_and_escaped_quotes_inside_remote_values_stay_redacted() {
		for value in [
			"expected private-account-name",
			"unexpected private-account-name",
			"private, expected account-name",
			"private\\\", expected account-name",
			"private\" , expected account-name\\",
			"private` , expected account-name",
		] {
			let bytes = serde_json::to_vec(&serde_json::json!({"id":"1","name":value})).unwrap();
			assert_eq!(
				trace::<Channel>("channels[0]", &bytes).unwrap(),
				"channels[0].name: invalid type: string \"…\", expected u32"
			);
		}
		assert_eq!(
			super::redact("duplicate field `name`"),
			"duplicate field `name`"
		);
		assert_eq!(
			super::redact("expected one of `first`, `second`"),
			"expected one of `first`, `second`"
		);
		assert_eq!(
			super::redact("unknown variant `private`, expected account-name`, expected `valid`"),
			"unknown variant `…`, expected `valid`"
		);
		assert_eq!(
			super::redact("unknown variant `private`, expected one of `first`, `second`"),
			"unknown variant `…`, expected one of `first`, `second`"
		);
		assert_eq!(
			super::redact("unknown field `private`, expected account-name`, there are no fields"),
			"unknown field `…`, there are no fields"
		);
	}

	#[test]
	fn redacts_identifier_map_keys_and_keeps_missing_field_names() {
		let bytes = br#"{"channels":[],"roles":{"123456789012345678":{"id":"3"}}}"#;
		assert_eq!(
			trace::<Guild>("", bytes).unwrap(),
			"roles.*: missing field `name`"
		);
		assert!(trace::<Guild>("", br#"{"channels":[],"roles":{}}"#).is_none());
		assert!(
			trace::<Guild>("", br#"{"channels":[}"#)
				.unwrap()
				.contains("malformed JSON")
		);
	}
}
