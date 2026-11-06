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

/// Viewers retained from one `STREAM_UPDATE`; the rest of a longer list is counted only.
pub const MAX_VIEWERS: usize = 64;

/// Viewer roster refresh for a stream. Discord sends every viewer ID; only a bounded prefix
/// is retained, while the count saturates so the join cue still sees a full roster.
#[derive(Deserialize)]
pub struct Update {
	pub stream_key: String,
	#[serde(default, rename = "viewer_ids", deserialize_with = "viewers")]
	pub viewers: Viewers,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Viewers {
	/// Number of `viewer_ids`, saturating at `u16::MAX`.
	pub count: u16,
	/// At most [`MAX_VIEWERS`] distinct, nonzero IDs, in the order Discord sent them.
	pub ids: Vec<Id>,
}

fn viewers<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Viewers, D::Error> {
	struct ViewersVisitor;
	impl<'de> serde::de::Visitor<'de> for ViewersVisitor {
		type Value = Viewers;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("a viewer ID list or null")
		}
		fn visit_unit<E>(self) -> Result<Self::Value, E> {
			Ok(Viewers::default())
		}
		fn visit_none<E>(self) -> Result<Self::Value, E> {
			Ok(Viewers::default())
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(
			self,
			mut seq: A,
		) -> Result<Self::Value, A::Error> {
			let mut viewers = Viewers::default();
			while let Some(viewer) = seq.next_element::<Viewer>()? {
				viewers.count = viewers.count.saturating_add(1);
				let Some(id) = viewer.0 else { continue };
				// Discord omits duplicates; a repeated ID must not spend another slot.
				if viewers.ids.len() < MAX_VIEWERS && !viewers.ids.contains(&id) {
					viewers.ids.push(id);
				}
			}
			Ok(viewers)
		}
	}
	deserializer.deserialize_any(ViewersVisitor)
}

/// One `viewer_ids` entry. A malformed or null entry is skipped but still counted.
struct Viewer(Option<Id>);
impl<'de> serde::Deserialize<'de> for Viewer {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct Element;
		// The visitor declares its own `'de`; an anonymous `Visitor<'_>` cannot name the
		// lifetime its `visit_some`/`visit_map`/`visit_seq` signatures need.
		impl<'de> serde::de::Visitor<'de> for Element {
			type Value = Viewer;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("a Discord ID string")
			}
			fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Viewer, E> {
				Ok(Viewer(value.parse().ok()))
			}
			fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Viewer, E> {
				Ok(Viewer((value != 0).then_some(Id(value))))
			}
			fn visit_unit<E: serde::de::Error>(self) -> Result<Viewer, E> {
				Ok(Viewer(None))
			}
			fn visit_none<E: serde::de::Error>(self) -> Result<Viewer, E> {
				Ok(Viewer(None))
			}
			fn visit_some<D: serde::Deserializer<'de>>(
				self,
				deserializer: D,
			) -> Result<Viewer, D::Error> {
				deserializer.deserialize_any(Element)
			}
			fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Viewer, E> {
				Ok(Viewer(None))
			}
			fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Viewer, E> {
				Ok(Viewer(
					u64::try_from(value).ok().filter(|id| *id != 0).map(Id),
				))
			}
			fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Viewer, E> {
				Ok(Viewer(None))
			}
			fn visit_map<A: serde::de::MapAccess<'de>>(
				self,
				mut map: A,
			) -> Result<Viewer, A::Error> {
				while map
					.next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
					.is_some()
				{}
				Ok(Viewer(None))
			}
			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut seq: A,
			) -> Result<Viewer, A::Error> {
				while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
				Ok(Viewer(None))
			}
		}
		deserializer.deserialize_any(Element)
	}
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
	fn viewer_ids_are_bounded_and_malformed_entries_are_skipped() {
		let update: Update = serde_json::from_str(
			r#"{"stream_key":"guild:1:2:3","viewer_ids":["4","x","5","4",null,"6"],"paused":false}"#,
		)
		.unwrap();
		assert_eq!(update.stream_key, "guild:1:2:3");
		assert_eq!(
			update.viewers,
			super::Viewers {
				count: 6,
				ids: vec![Id(4), Id(5), Id(6)],
			}
		);
		for data in [
			r#"{"stream_key":"k","viewer_ids":null}"#,
			r#"{"stream_key":"k"}"#,
			r#"{"stream_key":"k","viewer_ids":[]}"#,
		] {
			assert_eq!(
				serde_json::from_str::<Update>(data).unwrap().viewers,
				super::Viewers::default()
			);
		}
		let many = (1..=100).map(|id| format!("\"{id}\"")).collect::<Vec<_>>();
		let update: Update = serde_json::from_str(&format!(
			r#"{{"stream_key":"k","viewer_ids":[{}]}}"#,
			many.join(",")
		))
		.unwrap();
		// The count still spans every ID so the join cue sees a complete roster.
		assert_eq!(update.viewers.count, 100);
		assert_eq!(update.viewers.ids.len(), super::MAX_VIEWERS);
		assert_eq!(update.viewers.ids.last(), Some(&Id(64)));
	}
}
