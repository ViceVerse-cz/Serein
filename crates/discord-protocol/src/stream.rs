//! Unofficial normal-user screen-stream Gateway dispatch payloads.
use model::Id;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Key {
	pub stream_key: String,
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
	use super::Update;

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
