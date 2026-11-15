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

/// Viewer IDs kept from one roster refresh; the count still includes every listed viewer.
pub const MAX_VIEWER_IDS: usize = 64;

/// Viewer roster refresh for a stream.
#[derive(Deserialize)]
pub struct Update {
	pub stream_key: String,
	#[serde(default, rename = "viewer_ids", deserialize_with = "viewers")]
	pub viewers: Viewers,
}

/// `viewer_ids`: how many are listed, saturating at `u16::MAX`, and the first
/// `MAX_VIEWER_IDS` distinct IDs in Discord's order. Later IDs are skipped unread.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Viewers {
	pub count: u16,
	pub ids: Vec<Id>,
}

fn viewers<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Viewers, D::Error> {
	struct Roster;
	impl<'de> serde::de::Visitor<'de> for Roster {
		type Value = Viewers;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("a viewer ID list or null")
		}
		fn visit_unit<E>(self) -> Result<Viewers, E> {
			Ok(Viewers::default())
		}
		fn visit_none<E>(self) -> Result<Viewers, E> {
			Ok(Viewers::default())
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Viewers, A::Error> {
			let mut viewers = Viewers::default();
			loop {
				let more = if viewers.ids.len() < MAX_VIEWER_IDS {
					seq.next_element::<Id>()?.map(|id| {
						if !viewers.ids.contains(&id) {
							viewers.ids.push(id);
						}
					})
				} else {
					seq.next_element::<serde::de::IgnoredAny>()?.map(drop)
				};
				if more.is_none() {
					return Ok(viewers);
				}
				viewers.count = viewers.count.saturating_add(1);
			}
		}
	}
	deserializer.deserialize_any(Roster)
}

#[cfg(test)]
mod tests {
	use super::{Key, MAX_VIEWER_IDS, Update};
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
	fn viewer_roster_is_bounded_deduplicated_and_tolerates_null() {
		let update: Update = serde_json::from_str(
			r#"{"stream_key":"guild:1:2:3","viewer_ids":["4","5","4"],"paused":false}"#,
		)
		.unwrap();
		assert_eq!(update.stream_key, "guild:1:2:3");
		// Every listed entry counts, as Discord's count does; only distinct IDs are kept.
		assert_eq!(
			(update.viewers.count, update.viewers.ids),
			(3, vec![Id(4), Id(5)])
		);
		for data in [
			r#"{"stream_key":"k","viewer_ids":null}"#,
			r#"{"stream_key":"k"}"#,
		] {
			let viewers = serde_json::from_str::<Update>(data).unwrap().viewers;
			assert_eq!((viewers.count, viewers.ids.len()), (0, 0));
		}
		// Past the cap the rest is counted but not parsed, so it may even be malformed.
		let ids: Vec<String> = (1..=MAX_VIEWER_IDS as u64)
			.map(|id| format!("\"{id}\""))
			.chain(["\"999\"".into(), "{}".into()])
			.collect();
		let data = format!(r#"{{"stream_key":"k","viewer_ids":[{}]}}"#, ids.join(","));
		let viewers = serde_json::from_str::<Update>(&data).unwrap().viewers;
		assert_eq!(viewers.count as usize, MAX_VIEWER_IDS + 2);
		assert_eq!(viewers.ids.len(), MAX_VIEWER_IDS);
		assert_eq!(viewers.ids.last(), Some(&Id(MAX_VIEWER_IDS as u64)));
		// A malformed ID within the kept range rejects the refresh instead of dropping a viewer.
		assert!(
			serde_json::from_str::<Update>(r#"{"stream_key":"k","viewer_ids":["4","x"]}"#).is_err()
		);
	}
}
