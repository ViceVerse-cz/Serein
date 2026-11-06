//! Unofficial normal-user screen-stream Gateway dispatch payloads.
use model::Id;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Key {
	pub stream_key: String,
}
impl Key {
	/// The server, channel and user this stream key names.
	///
	/// Keys this client has seen are `guild:<guild>:<channel>:<user>` for a server channel and
	/// `call:<channel>:<user>` for a private call. Any other shape names nothing.
	pub fn target(&self) -> Option<(Option<Id>, Id, Id)> {
		if self.stream_key.len() > 128 {
			return None;
		}
		let mut parts = self.stream_key.split(':');
		let guild = match parts.next()? {
			"guild" => Some(parse_id(parts.next()?)?),
			"call" => None,
			_ => return None,
		};
		let channel = parse_id(parts.next()?)?;
		let user = parse_id(parts.next()?)?;
		parts.next().is_none().then_some((guild, channel, user))
	}
}
fn parse_id(value: &str) -> Option<Id> {
	value.parse::<u64>().ok().filter(|id| *id != 0).map(Id)
}

#[derive(Deserialize)]
pub struct Created {
	pub stream_key: String,
	pub rtc_server_id: Id,
	/// Viewer payloads have been seen without it; the RTC server then stands in.
	#[serde(default)]
	pub rtc_channel_id: Option<Id>,
}

#[derive(Deserialize)]
pub struct ServerUpdate {
	pub stream_key: String,
	pub token: String,
	pub endpoint: Option<String>,
}

#[derive(Deserialize)]
pub struct Deleted {
	pub stream_key: String,
	/// Why Discord ended the stream, for example `user_requested` or `stream_ended`.
	#[serde(default)]
	pub reason: Option<String>,
}

/// Viewer roster refresh for a stream; only the bounded viewer count is retained.
#[derive(Deserialize)]
pub struct Update {
	pub stream_key: String,
	/// Number of `viewer_ids`, saturating at `u16::MAX`; the IDs themselves are skipped.
	#[serde(default, rename = "viewer_ids", deserialize_with = "count")]
	pub viewers: u16,
}

fn count<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<u16, D::Error> {
	struct Count;
	impl<'de> serde::de::Visitor<'de> for Count {
		type Value = u16;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("a viewer ID list or null")
		}
		fn visit_unit<E>(self) -> Result<u16, E> {
			Ok(0)
		}
		fn visit_none<E>(self) -> Result<u16, E> {
			Ok(0)
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<u16, A::Error> {
			let mut count = 0u16;
			while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
				count = count.saturating_add(1);
			}
			Ok(count)
		}
	}
	deserializer.deserialize_any(Count)
}

#[cfg(test)]
mod tests {
	use super::{Key, Update};
	use model::Id;

	#[test]
	fn stream_keys_name_their_server_or_call_channel_and_user() {
		let target = |key: &str| {
			Key {
				stream_key: key.to_owned(),
			}
			.target()
		};
		assert_eq!(target("guild:1:2:3"), Some((Some(Id(1)), Id(2), Id(3))));
		assert_eq!(target("call:2:3"), Some((None, Id(2), Id(3))));
		for invalid in [
			"",
			"guild:1:2",
			"guild:1:2:3:4",
			"call:2",
			"call:2:3:4",
			"guild:0:2:3",
			"call:2:0",
			"guild:1:2:x",
			"guild:-1:2:3",
			"dm:2:3",
		] {
			assert_eq!(target(invalid), None, "{invalid}");
		}
		let long = format!("guild:1:2:{}", "3".repeat(120));
		assert_eq!(target(&long), None, "overlong keys name nothing");
	}

	#[test]
	fn viewer_count_skips_ids_and_tolerates_null() {
		let update: Update = serde_json::from_str(
			r#"{"stream_key":"guild:1:2:3","viewer_ids":["4","5"],"paused":false}"#,
		)
		.unwrap();
		assert_eq!(
			(update.stream_key.as_str(), update.viewers),
			("guild:1:2:3", 2)
		);
		for data in [
			r#"{"stream_key":"k","viewer_ids":null}"#,
			r#"{"stream_key":"k"}"#,
		] {
			assert_eq!(serde_json::from_str::<Update>(data).unwrap().viewers, 0);
		}
	}
}
