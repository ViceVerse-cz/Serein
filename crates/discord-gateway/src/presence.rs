use crate::{Event, Instant};
use client_core::{MAX_MEMBER_PRESENCE_BYTES, presence::Update};
use discord_protocol::presence::PresenceUpdate;
use model::Id;
use std::{
	collections::{BTreeMap, BTreeSet},
	time::Duration,
};

/// Bounds each shared-presence eligibility set; one `Id` per entry.
const MAX_SHARED_USERS: usize = 4096;

#[derive(Default)]
pub(crate) struct Pending {
	/// Batches guild-scoped presence for DM recipients who are not friends instead of
	/// global presence; see `Event::SharedGuildPresence`.
	pub shared: bool,
	/// Shared only: DM recipients and friends, kept current so unrelated guild traffic cannot
	/// fill the batch before an eligible recipient's update.
	recipients: BTreeSet<Id>,
	friends: BTreeSet<Id>,
	updates: BTreeMap<Id, Update>,
	bytes: usize,
	pub bootstrap_users: std::collections::BTreeSet<Id>,
	pub deadline: Option<Instant>,
}
impl Pending {
	pub fn shared() -> Self {
		Self {
			shared: true,
			..Self::default()
		}
	}
	pub fn push(&mut self, update: PresenceUpdate, now: Instant) {
		// Guild-scoped presence is not authoritative for a user's global/DM presence; it is
		// batched separately as a fallback for DM recipients Discord reports no other way.
		if update.guild.is_some() != self.shared
			|| (self.shared
				&& (!self.recipients.contains(&update.user) || self.friends.contains(&update.user)))
		{
			return;
		}
		self.insert(update, now);
	}
	pub fn add_recipient(&mut self, user: Id) {
		if self.recipients.len() < MAX_SHARED_USERS {
			self.recipients.insert(user);
		}
	}
	pub fn set_friend(&mut self, user: Id, friend: bool) {
		if !friend {
			self.friends.remove(&user);
		} else if self.friends.len() < MAX_SHARED_USERS {
			self.friends.insert(user);
		}
	}
	fn insert(&mut self, update: PresenceUpdate, now: Instant) {
		let update = Update {
			user: update.user,
			status: update.status,
			custom_status: update.custom_status,
			activities: update.activities,
			clients: update.clients,
		};
		let mut merged = self
			.updates
			.get(&update.user)
			.cloned()
			.unwrap_or_else(|| update.clone());
		merged.merge(update);
		if self.updates.len() >= 100 && !self.updates.contains_key(&merged.user) {
			return;
		}
		let bytes = self.bytes - self.updates.get(&merged.user).map_or(0, Update::heap_bytes)
			+ merged.heap_bytes();
		let projected = bytes + 100 * size_of::<Update>() + size_of::<Event>();
		if projected > MAX_MEMBER_PRESENCE_BYTES {
			return;
		}
		self.bytes = bytes;
		self.updates.insert(merged.user, merged);
		self.deadline
			.get_or_insert(now + Duration::from_millis(100));
	}
	pub fn take(&mut self) -> Option<Event> {
		self.deadline = None;
		self.bytes = 0;
		let updates: Vec<_> = std::mem::take(&mut self.updates).into_values().collect();
		(!updates.is_empty()).then(|| {
			if self.shared {
				Event::SharedGuildPresence(updates)
			} else {
				Event::DirectPresence(updates)
			}
		})
	}
	/// `merged_presences.guilds`: one presence list per guild, for the bootstrapped users only.
	pub fn guilds(
		&mut self,
		guilds: &serde_json::value::RawValue,
		now: Instant,
		emit: &impl Fn(Event) -> Result<(), client_core::auth::Failure>,
	) -> Result<(), client_core::auth::Failure> {
		let Ok(guilds) = serde_json::from_str::<Vec<&serde_json::value::RawValue>>(guilds.get())
		else {
			return Ok(());
		};
		for guild in guilds {
			self.friends(guild, now, emit)?;
		}
		Ok(())
	}
	#[cfg(test)]
	pub fn supplemental(
		&mut self,
		bytes: &[u8],
		now: Instant,
		emit: &impl Fn(Event) -> Result<(), client_core::auth::Failure>,
	) -> Result<(), client_core::auth::Failure> {
		// Identify sends no DEDUPE_USER_OBJECTS capability, so READY uses presences.
		// Also accept merged_presences.friends from the unofficial supplemental format.
		// Decode only bounded wire data; no assets/secrets survive the presence decoder.
		#[derive(serde::Deserialize)]
		struct Snapshot<'a> {
			#[serde(default, borrow)]
			presences: Option<&'a serde_json::value::RawValue>,
			#[serde(default, borrow)]
			merged_presences: Option<Merged<'a>>,
		}
		#[derive(serde::Deserialize)]
		struct Merged<'a> {
			#[serde(default, borrow)]
			friends: Option<&'a serde_json::value::RawValue>,
		}
		if bytes.len() > discord_protocol::MAX_GATEWAY_WIRE {
			return Ok(());
		}
		let Ok(data) = serde_json::from_slice::<Snapshot<'_>>(bytes) else {
			return Ok(());
		};
		let Some(friends) = data
			.merged_presences
			.and_then(|m| m.friends)
			.or(data.presences)
		else {
			return Ok(());
		};
		self.friends(friends, now, emit)
	}
	pub fn friends(
		&mut self,
		friends: &serde_json::value::RawValue,
		now: Instant,
		emit: &impl Fn(Event) -> Result<(), client_core::auth::Failure>,
	) -> Result<(), client_core::auth::Failure> {
		let Ok(friends) =
			serde_json::from_str::<discord_protocol::presence::Friends<'_>>(friends.get())
		else {
			return Ok(());
		};
		for friend in friends.0 {
			if let Ok(update) = discord_protocol::presence::decode(friend.get().as_bytes()) {
				if !self.bootstrap_users.contains(&update.user) {
					continue;
				}
				if (self.updates.len() >= 100 || self.bytes > 96 * 1024)
					&& let Some(event) = self.take()
				{
					emit(event)?;
				}
				self.insert(update, now);
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::Patch;

	#[test]
	fn guild_presence_is_batched_apart_for_dm_recipients_who_are_not_friends() {
		let now = Instant::now();
		let decode = |wire: &[u8]| discord_protocol::presence::decode(wire).unwrap();
		let guild_idle = br#"{"guild_id":"10","user":{"id":"3"},"status":"idle"}"#;
		let mut direct = Pending::default();
		direct.push(decode(guild_idle), now);
		assert!(
			direct.take().is_none(),
			"guild presence never enters the direct feed"
		);
		let mut shared = Pending::shared();
		shared.add_recipient(Id(3));
		shared.push(decode(br#"{"user":{"id":"3"},"status":"online"}"#), now);
		assert!(
			shared.take().is_none(),
			"global presence never enters the fallback"
		);
		shared.push(decode(guild_idle), now);
		let Some(Event::SharedGuildPresence(updates)) = shared.take() else {
			panic!()
		};
		assert_eq!(
			(updates[0].user, &updates[0].status),
			(Id(3), &Patch::Value("idle".into()))
		);
		// READY_SUPPLEMENTAL bootstrap keeps only the users it was asked for.
		shared.bootstrap_users.insert(Id(3));
		let guilds = serde_json::value::RawValue::from_string(
			r#"[[{"user_id":"3","status":"idle"},{"user_id":"4","status":"online"}],[]]"#.into(),
		)
		.unwrap();
		shared.guilds(&guilds, now, &|_| Ok(())).unwrap();
		let Some(Event::SharedGuildPresence(updates)) = shared.take() else {
			panic!()
		};
		assert_eq!(updates.len(), 1);
		assert_eq!(
			(updates[0].user, &updates[0].status),
			(Id(3), &Patch::Value("idle".into()))
		);
	}

	#[test]
	fn unrelated_guild_traffic_cannot_crowd_out_dm_recipients() {
		let now = Instant::now();
		let guild_update = |user: u64| {
			discord_protocol::presence::decode(
				format!(r#"{{"guild_id":"10","user":{{"id":"{user}"}},"status":"idle"}}"#)
					.as_bytes(),
			)
			.unwrap()
		};
		let mut shared = Pending::shared();
		shared.add_recipient(Id(3));
		shared.add_recipient(Id(4));
		shared.set_friend(Id(4), true);
		for user in 1_000..1_150 {
			shared.push(guild_update(user), now);
		}
		shared.push(guild_update(4), now);
		shared.push(guild_update(3), now);
		let Some(Event::SharedGuildPresence(updates)) = shared.take() else {
			panic!()
		};
		assert_eq!(
			updates.iter().map(|u| u.user).collect::<Vec<_>>(),
			[Id(3)],
			"only non-friend DM recipients enter the shared batch"
		);
		shared.set_friend(Id(4), false);
		shared.push(guild_update(4), now);
		assert!(matches!(shared.take(), Some(Event::SharedGuildPresence(u)) if u[0].user == Id(4)));
	}

	#[test]
	fn direct_patches_coalesce_without_losing_absent_fields_and_stay_bounded() {
		let now = Instant::now();
		let mut pending = Pending::default();
		pending.bootstrap_users.insert(Id(3));
		pending.supplemental(br#"{"merged_presences":{"friends":[{"user_id":"3","status":"online","activities":[{"type":0,"name":"Synthetic game"}]}]}}"#, now, &|_| Ok(())).unwrap();
		pending.push(
			discord_protocol::presence::decode(br#"{"user":{"id":"3"},"status":"idle"}"#).unwrap(),
			now + Duration::from_millis(50),
		);
		assert_eq!(pending.deadline, Some(now + Duration::from_millis(100)));
		let event = pending.take().unwrap();
		assert!(event.bytes() <= MAX_MEMBER_PRESENCE_BYTES);
		let Event::DirectPresence(updates) = event else {
			panic!()
		};
		assert_eq!(updates.len(), 1);
		assert_eq!(updates[0].status, Patch::Value("idle".into()));
		assert!(
			matches!(&updates[0].activities, Patch::Value(a) if a.len()==1 && a[0].name=="Synthetic game")
		);
		assert!(pending.take().is_none());
		for id in 1..1000 {
			let bytes = format!(
				r#"{{"user":{{"id":"{id}"}},"activities":[{{"type":0,"name":"Synthetic"}}]}}"#
			);
			pending.push(
				discord_protocol::presence::decode(bytes.as_bytes()).unwrap(),
				now,
			);
		}
		assert_eq!(pending.updates.len(), 100);
		assert!(pending.take().unwrap().bytes() <= MAX_MEMBER_PRESENCE_BYTES);
		for wire in [
            br#"{"user":{"id":"3"},"status":"online","activities":[{"type":0,"name":"Old activity"}]}"#.as_slice(),
            br#"{"user":{"id":"3"},"status":"offline"}"#,
            br#"{"user":{"id":"3"},"status":"online"}"#,
        ] { pending.push(discord_protocol::presence::decode(wire).unwrap(), now); }
		let Event::DirectPresence(updates) = pending.take().unwrap() else {
			panic!()
		};
		assert_eq!(updates[0].activities, Patch::Null);
		assert_eq!(updates[0].custom_status, Patch::Null);
	}
	#[test]
	fn supplemental_filters_unknown_users_and_streams_more_than_one_batch() {
		let now = Instant::now();
		let mut pending = Pending::default();
		pending.bootstrap_users.extend((100..=255).map(Id));
		let friends: Vec<_> = (1..=300).map(|id| serde_json::json!({"user_id":id.to_string(),"status":"online","activities":[{"type":0,"name":"Synthetic"}]})).collect();
		let bytes =
			serde_json::to_vec(&serde_json::json!({"merged_presences":{"friends":friends}}))
				.unwrap();
		let received = std::cell::RefCell::new(Vec::new());
		pending
			.supplemental(&bytes, now, &|event| {
				assert!(event.bytes() <= MAX_MEMBER_PRESENCE_BYTES);
				let Event::DirectPresence(updates) = event else {
					panic!()
				};
				received
					.borrow_mut()
					.extend(updates.into_iter().map(|u| u.user));
				Ok(())
			})
			.unwrap();
		let Event::DirectPresence(updates) = pending.take().unwrap() else {
			panic!()
		};
		received
			.borrow_mut()
			.extend(updates.into_iter().map(|u| u.user));
		assert_eq!(*received.borrow(), (100..=255).map(Id).collect::<Vec<_>>());
	}
}
