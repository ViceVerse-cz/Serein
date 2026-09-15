//! Channel visibility comes from the Gateway flag, never a placeholder name.
use super::voice::Calls;
use client_core::{Event, auth::Failure};
use discord_protocol::{ChannelDto, ChannelPatchDto, Ready, decode};
use model::{Channel, Id};
use std::collections::{BTreeMap, BTreeSet};

type InboxEvent = Option<(Id, bool)>;

pub(super) fn private_call(kind: u8, recipients: usize) -> bool {
	(kind == 1 && recipients == 1)
		|| (kind == 3 && recipients < client_core::voice::MAX_PARTICIPANTS)
}

#[derive(Default)]
pub(super) struct Inbox {
	marks: BTreeMap<Id, (bool, bool)>,
}

impl Inbox {
	pub fn reset(&mut self) {
		self.marks.clear();
	}

	pub fn forget(&mut self, id: Id) {
		self.marks.remove(&id);
	}

	pub fn observe(&mut self, channel: &ChannelDto) -> bool {
		if channel.is_obfuscated() || channel.guild_id.is_some() || channel.kind != 1 {
			self.marks.remove(&channel.id);
			return true;
		}
		self.store(
			channel.id,
			channel.is_message_request,
			channel.pending_spam_direct(),
		)
	}

	pub fn classify_update(&mut self, patch: &ChannelPatchDto) -> (InboxEvent, InboxEvent) {
		let prior = self.marks.get(&patch.id).copied().unwrap_or((false, false));
		if patch.is_obfuscated() {
			self.marks.remove(&patch.id);
			return (
				(prior.0 && !prior.1).then_some((patch.id, false)),
				prior.1.then_some((patch.id, false)),
			);
		}
		let (request, spam) = patch.merged_inbox(prior);
		if !self.store(patch.id, request, spam) {
			return (None, None);
		}
		let new_req = request && !spam;
		let old_req = prior.0 && !prior.1;
		(
			(new_req != old_req).then_some((patch.id, new_req)),
			(spam != prior.1).then_some((patch.id, spam)),
		)
	}

	fn store(&mut self, id: Id, request: bool, spam: bool) -> bool {
		if !(request || spam) {
			self.marks.remove(&id);
			return true;
		}
		if self.marks.len() >= client_core::user_actions::MAX_RELATIONSHIPS
			&& !self.marks.contains_key(&id)
		{
			return false;
		}
		self.marks.insert(id, (request, spam));
		true
	}
}

pub(super) fn ready_calls(ready: &Ready, calls: &mut Calls) -> Result<BTreeSet<Id>, Failure> {
	if ready.guilds.len()
		+ ready.private_channels.len()
		+ ready
			.guilds
			.iter()
			.map(|g| g.channels.len() + g.threads.len())
			.sum::<usize>()
		> client_core::MAX_NAV
	{
		return Err(Failure::CapacityAt(
			"Account navigation exceeds 131,072 entries; connection stopped",
		));
	}
	calls.allowed.clear();
	let guilds: BTreeSet<_> = ready
		.guilds
		.iter()
		.filter(|g| g.id.0 != 0)
		.map(|g| g.id)
		.collect();
	for channel in &ready.private_channels {
		if !channel.is_obfuscated()
			&& channel.guild_id.is_none()
			&& private_call(channel.kind, channel.recipients.len())
		{
			calls.allowed.insert(channel.id, None);
		}
	}
	for guild in &ready.guilds {
		if !guilds.contains(&guild.id) {
			continue;
		}
		for channel in &guild.channels {
			if !channel.is_obfuscated() && channel.kind == 2 {
				calls.allowed.insert(channel.id, Some(guild.id));
			}
		}
	}
	Ok(guilds)
}

pub(super) fn admit_call(channel: &Channel, guilds: &BTreeSet<Id>, calls: &mut Calls) {
	let eligible = channel.id.0 != 0
		&& match channel.guild {
			Some(guild) => guilds.contains(&guild) && channel.kind == 2,
			None => private_call(channel.kind, channel.recipients.len()),
		};
	if eligible
		&& calls
			.allowed
			.get(&channel.id)
			.is_none_or(|guild| *guild == channel.guild)
		&& (calls.allowed.contains_key(&channel.id) || calls.allowed.len() < client_core::MAX_NAV)
	{
		calls.allowed.insert(channel.id, channel.guild);
	}
}

pub(super) fn create(
	bytes: &[u8],
	inbox: &mut Inbox,
) -> Result<(Event, Option<Id>, Option<Id>), Failure> {
	let channel: ChannelDto = decode(bytes).map_err(|_| Failure::Protocol)?;
	if !inbox.observe(&channel) {
		return Ok((created(channel), None, None));
	}
	let visible = !channel.is_obfuscated();
	let request = (visible && channel.pending_message_request()).then_some(channel.id);
	let spam = (visible && channel.pending_spam_direct()).then_some(channel.id);
	Ok((created(channel), request, spam))
}

pub(super) fn permission_metadata(bytes: &[u8], user: Id) -> Result<Option<Event>, Failure> {
	Ok(discord_protocol::permissions::channel(bytes, user)
		.map_err(|_| Failure::Protocol)?
		.map(|update| {
			Event::Permissions(client_core::permissions::Event::Channel {
				channel: update.id,
				guild: update.guild,
				overwrites: update.overwrites,
			})
		}))
}

pub(super) fn created(channel: ChannelDto) -> Event {
	if channel.is_obfuscated() {
		Event::Unavailable(channel.id)
	} else {
		Event::ChannelCreated(channel.into_model())
	}
}

pub(super) struct Update {
	pub restored: Option<Channel>,
	pub event: Event,
	pub message_request: Option<(Id, bool)>,
	pub spam_direct: Option<(Id, bool)>,
}

pub(super) fn update(bytes: &[u8], inbox: &mut Inbox) -> Result<Update, Failure> {
	let patch: ChannelPatchDto = decode(bytes).map_err(|_| Failure::Protocol)?;
	let (message_request, spam_direct) = inbox.classify_update(&patch);
	if patch.is_obfuscated() {
		return Ok(Update {
			restored: None,
			event: Event::Unavailable(patch.id),
			message_request,
			spam_direct,
		});
	}
	let restored = decode::<ChannelDto>(bytes)
		.ok()
		.filter(|channel| {
			channel.id.0 != 0
				&& channel.guild_id.is_some_and(|id| id.0 != 0)
				&& matches!(channel.kind, 0 | 2 | 4 | 5 | 13..=16)
				&& channel.name.as_ref().is_some_and(|name| {
					!name.trim().is_empty()
						&& name.chars().count() <= 100
						&& !name.chars().any(char::is_control)
				})
		})
		.map(ChannelDto::into_model);
	Ok(Update {
		restored,
		event: Event::ChannelChanged(patch.into_model()),
		message_request,
		spam_direct,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::{Id, Patch};

	#[test]
	fn voice_admission_and_snapshots_exclude_hidden_and_unknown_guild_channels() {
		let mut ready: Ready = decode(br#"{"user":{"id":"9","username":"Synthetic"},"session_id":"synthetic","resume_gateway_url":"wss://gateway.discord.gg","guilds":[{"id":"1","channels":[{"id":"2","type":2,"flags":131072},{"id":"3","type":2}],"voice_states":[{"channel_id":"2","user_id":"8"},{"channel_id":"3","user_id":"9"}]}]}"#).unwrap();
		let mut calls = Calls::default();
		let guilds = ready_calls(&ready, &mut calls).unwrap();
		assert!(!calls.allowed.contains_key(&Id(2)));
		assert_eq!(calls.allowed.get(&Id(3)), Some(&Some(Id(1))));
		let Event::Voice(client_core::voice::Event::Snapshot { participants, .. }) =
			calls.snapshot(&mut ready.guilds[0], false).unwrap()
		else {
			panic!("snapshot")
		};
		assert_eq!(participants.len(), 1);
		assert_eq!(participants[0].channel, Id(3));
		for guild in ["99", "1"] {
			let body =
				format!(r#"{{"id":"2","guild_id":"{guild}","type":2,"name":"Restored voice"}}"#);
			let restored = update(body.as_bytes(), &mut Inbox::default())
				.unwrap()
				.restored
				.unwrap();
			admit_call(&restored, &guilds, &mut calls);
			assert_eq!(calls.allowed.contains_key(&Id(2)), guild == "1");
		}
		let hidden: ChannelDto = decode(br#"{"id":"2","type":2,"flags":131072}"#).unwrap();
		let Event::Unavailable(id) = created(hidden) else {
			panic!("revocation")
		};
		calls.allowed.remove(&id);
		let mut supplemental = decode(br#"{"id":"1","voice_states":[{"channel_id":"2","user_id":"8"},{"channel_id":"3","user_id":"9"}]}"#).unwrap();
		let Event::Voice(client_core::voice::Event::Snapshot { participants, .. }) =
			calls.snapshot(&mut supplemental, true).unwrap()
		else {
			panic!("snapshot")
		};
		assert_eq!(participants.len(), 1);
		assert_eq!(participants[0].channel, Id(3));
	}

	#[test]
	fn obfuscation_revokes_and_restoration_preserves_partial_updates() {
		let hidden =
			br#"{"id":"3","guild_id":"1","type":0,"flags":131072,"name":"not-a-placeholder"}"#;
		assert!(matches!(
			create(hidden, &mut Inbox::default()).unwrap(),
			(Event::Unavailable(Id(3)), None, None)
		));
		let update = super::update(hidden, &mut Inbox::default()).unwrap();
		assert!(matches!(update.event, Event::Unavailable(Id(3))));
		assert!(update.restored.is_none());
		assert!(matches!(
			create(
				br#"{"id":"3","guild_id":"1","type":0,"name":"___hidden___"}"#,
				&mut Inbox::default()
			)
			.unwrap(),
			(Event::ChannelCreated(_), None, None)
		));
		for flags in ["", ",\"flags\":0", ",\"flags\":16"] {
			let body = format!(r#"{{"id":"3","guild_id":"1","type":0,"name":"Restored"{flags}}}"#);
			let update = super::update(body.as_bytes(), &mut Inbox::default()).unwrap();
			assert_eq!(update.restored.unwrap().guild, Some(Id(1)));
			let Event::ChannelChanged(patch) = update.event else {
				panic!("patch required")
			};
			assert_eq!(patch.last_message, Patch::Absent);
			assert_eq!(patch.parent_id, Patch::Absent);
		}
		for body in [
			r#"{"id":"3","name":"renamed"}"#,
			r#"{"id":"3","guild_id":"1","type":0,"name":null}"#,
			r#"{"id":"3","type":0,"name":"No guild"}"#,
			r#"{"id":"3","guild_id":"0","type":0,"name":"Invalid guild"}"#,
			r#"{"id":"3","guild_id":"1","type":1,"name":"Not a guild channel"}"#,
		] {
			assert!(
				super::update(body.as_bytes(), &mut Inbox::default())
					.unwrap()
					.restored
					.is_none()
			);
		}
		let partial = super::update(
			br#"{"id":"3","parent_id":null,"position":0}"#,
			&mut Inbox::default(),
		)
		.unwrap();
		let Event::ChannelChanged(patch) = partial.event else {
			panic!("patch required")
		};
		assert_eq!(patch.parent_id, Patch::Null);
		assert_eq!(patch.position, Patch::Value(0));
		assert_eq!(patch.name, Patch::Absent);
	}
}
