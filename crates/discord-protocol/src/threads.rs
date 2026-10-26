//! Bounded thread snapshots. Omitted parents means the whole guild; an empty list means none.
use crate::{ChannelDto, DecodeError};
use model::{Channel, Id};
use serde::{
	Deserialize, Deserializer,
	de::{SeqAccess, Visitor},
};
use std::collections::BTreeSet;

pub(crate) const MAX_ITEMS: usize = model::account::MAX_ENTRIES;
pub(crate) const MAX_BYTES: usize = model::account::MAX_BYTES;

pub(crate) fn list<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
	d: D,
) -> Result<Vec<T>, D::Error> {
	struct List<T>(std::marker::PhantomData<T>);
	impl<'de, T: Deserialize<'de>> Visitor<'de> for List<T> {
		type Value = Vec<T>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("bounded account navigation entries")
		}
		fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
			let mut values = Vec::new();
			while let Some(value) = seq.next_element()? {
				if values.len() == MAX_ITEMS {
					return Err(serde::de::Error::custom("Thread list capacity exceeded"));
				}
				values.push(value);
			}
			Ok(values)
		}
	}
	d.deserialize_seq(List(std::marker::PhantomData))
}
fn parents<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<Id>>, D::Error> {
	list(d).map(Some)
}

#[derive(Deserialize)]
pub struct ThreadListSync {
	guild_id: Id,
	#[serde(default, deserialize_with = "parents")]
	channel_ids: Option<Vec<Id>>,
	#[serde(deserialize_with = "list")]
	threads: Vec<ChannelDto>,
	/// The account's memberships among `threads`; first-party syncs usually omit it.
	#[serde(default, deserialize_with = "list")]
	members: Vec<serde_json::Value>,
}
pub struct Sync {
	pub guild: Id,
	pub parents: Option<Vec<Id>>,
	pub threads: Vec<Channel>,
	pub removed: Vec<Id>,
	/// Archive metadata of every listed thread, then any membership the sync carried.
	pub changes: Vec<model::threads::Change>,
}
#[derive(Deserialize)]
pub struct ThreadUpdate {
	pub guild_id: Id,
	#[serde(flatten)]
	pub patch: crate::ChannelPatchDto,
	#[serde(default)]
	pub thread_metadata: Option<ThreadMetadata>,
}
#[derive(Deserialize)]
pub struct ThreadMetadata {
	#[serde(default, deserialize_with = "crate::lossy::null_default")]
	pub archived: bool,
	#[serde(default, deserialize_with = "lenient")]
	auto_archive_duration: Option<u32>,
	#[serde(default, deserialize_with = "lenient_ms")]
	archive_timestamp: Option<i64>,
}
impl ThreadMetadata {
	pub fn meta(&self) -> model::threads::Meta {
		model::threads::Meta {
			// Discord's default window is one day.
			auto_archive_minutes: self.auto_archive_duration.unwrap_or(1440),
			activity: self.archive_timestamp.unwrap_or(0),
		}
	}
}
/// A thread member object. Thread-scoped events carry `id` (the thread) and `user_id`; the
/// `member` embedded in a thread object carries neither and always describes the account.
#[derive(Deserialize)]
pub struct ThreadMember {
	#[serde(default, deserialize_with = "lenient")]
	pub id: Option<Id>,
	#[serde(default, deserialize_with = "lenient")]
	pub user_id: Option<Id>,
	#[serde(default, deserialize_with = "lenient")]
	pub guild_id: Option<Id>,
	#[serde(default, deserialize_with = "lenient_ms")]
	join_timestamp: Option<i64>,
	#[serde(default, deserialize_with = "lenient")]
	muted: Option<bool>,
	#[serde(default, deserialize_with = "lenient")]
	mute_config: Option<MuteConfig>,
}
#[derive(Deserialize)]
struct MuteConfig {
	#[serde(default, deserialize_with = "lenient_ms")]
	end_time: Option<i64>,
}
impl ThreadMember {
	pub fn into_model(&self) -> model::threads::Member {
		model::threads::Member {
			joined: self.join_timestamp.unwrap_or(0),
			muted: self.muted.unwrap_or(false),
			mute_end: self.mute_config.as_ref().and_then(|config| config.end_time),
		}
	}
	/// The account's membership, as a change for `thread`.
	pub fn change(&self, thread: Id) -> model::threads::Change {
		model::threads::Change {
			id: thread,
			member: model::Patch::Value(self.into_model()),
			meta: None,
			pinned: None,
		}
	}
}
#[derive(Deserialize)]
pub struct ThreadMembers {
	pub id: Id,
	pub guild_id: Id,
	#[serde(default, deserialize_with = "crate::read_state::entries")]
	pub removed_member_ids: Vec<Id>,
	#[serde(default, deserialize_with = "list")]
	added_members: Vec<serde_json::Value>,
}
impl ThreadMembers {
	/// The account's own entry among `added_members`, when it joined.
	pub fn joined(&self, owner: Id) -> Option<model::threads::Change> {
		self.added_members
			.iter()
			.filter_map(|member| ThreadMember::deserialize(member).ok())
			.find(|member| member.user_id == Some(owner))
			.map(|member| member.change(self.id))
	}
}

/// Optional fields that must never reject the thread carrying them.
pub(crate) fn lenient<'de, D: Deserializer<'de>, T: serde::de::DeserializeOwned>(
	d: D,
) -> Result<Option<T>, D::Error> {
	Ok(serde_json::from_value(serde_json::Value::deserialize(d)?).ok())
}
/// An RFC 3339 timestamp in Unix milliseconds; malformed values read as absent.
pub(crate) fn lenient_ms<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
	Ok(lenient::<D, crate::Timestamp>(d)?.and_then(|t| t.unix_ms()))
}
impl ThreadListSync {
	pub fn into_model(self) -> Result<Sync, DecodeError> {
		let parents: Option<BTreeSet<_>> = self
			.channel_ids
			.as_ref()
			.map(|ids| ids.iter().copied().collect());
		if self
			.channel_ids
			.as_ref()
			.is_some_and(|ids| parents.as_ref().is_some_and(|set| set.len() != ids.len()))
			|| self.threads.len() + self.channel_ids.as_ref().map_or(0, Vec::len) > MAX_ITEMS
		{
			return Err(DecodeError);
		}
		let mut ids = BTreeSet::new();
		let mut bytes = self
			.channel_ids
			.as_ref()
			.map_or(0, |ids| ids.capacity() * size_of::<Id>());
		let mut threads = Vec::with_capacity(self.threads.len());
		let mut removed = Vec::new();
		let mut changes = Vec::new();
		for mut thread in self.threads {
			let hidden = thread.is_obfuscated();
			let change = (!hidden).then(|| thread.thread_change()).flatten();
			// Hidden entries still count toward identity, scope, and byte validation.
			// Omitting them only after validation lets this snapshot revoke old rows.
			thread.flags &= !(1 << 17);
			let thread = into_thread(thread, self.guild_id)?;
			if !ids.insert(thread.id)
				|| parents.as_ref().is_some_and(|parents| {
					!parents.contains(&thread.parent_id.expect("validated parent"))
				}) {
				return Err(DecodeError);
			}
			bytes += thread.bytes();
			if bytes > MAX_BYTES {
				return Err(DecodeError);
			}
			if hidden {
				removed.push(thread.id);
			} else {
				changes.extend(change);
				threads.push(thread);
			}
		}
		changes.extend(
			self.members
				.iter()
				.filter_map(|member| ThreadMember::deserialize(member).ok())
				.filter_map(|member| Some(member.change(member.id.filter(|id| ids.contains(id))?))),
		);
		Ok(Sync {
			guild: self.guild_id,
			parents: self.channel_ids,
			threads,
			removed,
			changes,
		})
	}
}

pub fn into_thread(mut thread: ChannelDto, guild: Id) -> Result<Channel, DecodeError> {
	if thread.is_obfuscated()
		|| !matches!(thread.kind, 10..=12)
		|| thread.parent_id.is_none()
		|| thread.parent_id == Some(thread.id)
		|| thread.guild_id.is_some_and(|id| id != guild)
	{
		return Err(DecodeError);
	}
	thread.guild_id = Some(guild);
	if let Some(name) = &mut thread.name {
		*name = name.chars().take(128).collect();
	}
	let thread = thread.into_model();
	if thread.bytes() > MAX_BYTES {
		return Err(DecodeError);
	}
	Ok(thread)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Ready, decode};
	#[test]
	fn snapshots_preserve_scope_and_reject_cross_guild_duplicates_and_limits() {
		let hidden = br#"{"id":"3","parent_id":"2","type":11,"flags":131072}"#;
		assert!(into_thread(decode(hidden).unwrap(), Id(1)).is_err());
		let hidden = decode::<ThreadListSync>(br#"{"guild_id":"1","channel_ids":["2"],"threads":[{"id":"3","parent_id":"2","type":11,"flags":131072}]}"#).unwrap().into_model().unwrap();
		assert_eq!(hidden.parents, Some(vec![Id(2)]));
		assert_eq!(hidden.removed, vec![Id(3)]);
		assert!(
			hidden.threads.is_empty(),
			"An authoritative empty scope revokes a previously loaded hidden thread"
		);
		for invalid in [
            br#"{"guild_id":"1","threads":[{"id":"3","parent_id":"2","type":11,"flags":131072},{"id":"3","parent_id":"2","type":11}]}"#.as_slice(),
            br#"{"guild_id":"1","channel_ids":["9"],"threads":[{"id":"3","parent_id":"2","type":11,"flags":131072}]}"#.as_slice(),
            br#"{"guild_id":"1","threads":[{"id":"3","guild_id":"9","parent_id":"2","type":11,"flags":131072}]}"#.as_slice(),
        ] {
            assert!(decode::<ThreadListSync>(invalid).unwrap().into_model().is_err(),
                "Hidden rows cannot bypass duplicate or parent/guild scope validation");
        }
		let all = decode::<ThreadListSync>(br#"{"guild_id":"1","threads":[{"id":"3","parent_id":"2","type":11,"name":"Synthetic"}]}"#).unwrap().into_model().unwrap();
		assert_eq!(all.guild, Id(1));
		assert!(all.parents.is_none());
		assert_eq!(all.threads[0].guild, Some(Id(1)));
		let empty = decode::<ThreadListSync>(br#"{"guild_id":"1","channel_ids":[],"threads":[]}"#)
			.unwrap()
			.into_model()
			.unwrap();
		assert_eq!(empty.parents, Some(vec![]));
		assert!(
			decode::<ThreadListSync>(br#"{"guild_id":"1","channel_ids":null,"threads":[]}"#)
				.is_err()
		);
		for body in [
			r#"{"guild_id":"1","channel_ids":[],"threads":[{"id":"3","parent_id":"2","type":11}]}"#,
			r#"{"guild_id":"1","channel_ids":["2","2"],"threads":[]}"#,
			r#"{"guild_id":"1","threads":[{"id":"3","guild_id":"9","parent_id":"2","type":11}]}"#,
			r#"{"guild_id":"1","threads":[{"id":"3","parent_id":"2","type":0}]}"#,
			r#"{"guild_id":"1","threads":[{"id":"3","type":11}]}"#,
			r#"{"guild_id":"1","threads":[{"id":"3","parent_id":"3","type":11}]}"#,
			r#"{"guild_id":"1","threads":[{"id":"3","parent_id":"2","type":11},{"id":"3","parent_id":"2","type":12}]}"#,
		] {
			assert!(
				decode::<ThreadListSync>(body.as_bytes())
					.unwrap()
					.into_model()
					.is_err()
			);
		}
		let many =
			serde_json::json!({"guild_id":"1","channel_ids":vec!["2";MAX_ITEMS+1],"threads":[]});
		assert!(decode::<ThreadListSync>(&serde_json::to_vec(&many).unwrap()).is_err());
		let many = serde_json::json!({"guild_id":"1","threads":vec![serde_json::json!({"id":"3","parent_id":"2","type":11});MAX_ITEMS+1]});
		assert!(decode::<ThreadListSync>(&serde_json::to_vec(&many).unwrap()).is_err());
		let oversized = serde_json::json!({"guild_id":"1","threads":(3..4003).map(|id| serde_json::json!({"id":id.to_string(),"parent_id":"2","type":11,"name":"界".repeat(128),"recipients":[{"id":"9","username":"Synthetic"}]})).collect::<Vec<_>>()});
		assert!(
			decode::<ThreadListSync>(&serde_json::to_vec(&oversized).unwrap())
				.unwrap()
				.into_model()
				.is_ok()
		);
	}
	#[test]
	fn ready_merges_threads_without_overwriting_guild_or_channel_identity() {
		let base = serde_json::json!({"user":{"id":"9","username":"Synthetic"},"session_id":"synthetic","resume_gateway_url":"wss://gateway.discord.gg","guilds":[{"id":"1","name":"Synthetic","channels":[{"id":"2","type":0,"name":"general"}],"threads":[{"id":"3","parent_id":"2","type":11,"name":"thread"}]}]});
		let mut ready: Ready = decode(&serde_json::to_vec(&base).unwrap()).unwrap();
		let (guilds, channels) = ready.navigation().unwrap();
		assert_eq!(guilds.len(), 1);
		assert_eq!(channels.len(), 2);
		assert!(channels.iter().all(|c| c.guild == Some(Id(1))));
		assert_eq!(channels[1].parent_id, Some(Id(2)));
		for (key, value) in [("guild_id", "7"), ("id", "2")] {
			let mut invalid = base.clone();
			invalid["guilds"][0]["threads"][0][key] = value.into();
			let mut ready: Ready = decode(&serde_json::to_vec(&invalid).unwrap()).unwrap();
			let (_, channels) = ready.navigation().unwrap();
			assert!(
				ready.skipped,
				"A conflicting thread is dropped, never merged"
			);
			assert_eq!(
				channels.iter().map(|c| c.id).collect::<Vec<_>>(),
				vec![Id(2)]
			);
		}
	}
}
