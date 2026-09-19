use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Channel, Freshness, Id, Patch};
use std::collections::BTreeMap;

pub enum Event {
	Snapshot {
		entries: Option<Vec<(Id, Option<Id>, u32)>>,
		version: Option<u64>,
		partial: bool,
	},
	Ack {
		channel: Id,
		message: Option<Id>,
		manual: bool,
		mention_count: Option<u32>,
		version: Option<u64>,
	},
	Latest(Vec<(Id, Patch<Id>)>),
	Result {
		channel: Id,
		message: Id,
		request: u64,
		result: Result<(), Failure>,
	},
	GuildAck {
		guild: Id,
		request: u64,
		result: Result<(), Failure>,
	},
}
impl Event {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ match self {
				Self::Snapshot { entries, .. } => entries.as_ref().map_or(0, |entries| {
					entries.capacity() * size_of::<(Id, Option<Id>, u32)>()
				}),
				Self::Latest(entries) => entries.capacity() * size_of::<(Id, Patch<Id>)>(),
				_ => 0,
			}
	}
}
enum Pending {
	Channel {
		channel: Id,
		message: Id,
		request: u64,
		epoch: u64,
		manual: bool,
	},
	Guild {
		guild: Id,
		request: u64,
		channels: Vec<(Id, u64, Id)>,
	},
}

#[derive(Default)]
pub struct ReadState {
	pub(crate) activity: crate::notifications::Activity,
	entries: BTreeMap<Id, (Option<Id>, u64)>,
	known: bool,
	version: Option<u64>,
	revision: u64,
	pending: Option<Pending>,
	pub(crate) status: Option<(Id, &'static str)>,
}
impl ReadState {
	/// True once a complete service snapshot arrived, so a channel without a row was never read.
	pub(crate) fn known(&self) -> bool {
		self.known
	}
	pub fn status(&self, channel: Id) -> Option<&'static str> {
		self.status
			.filter(|(id, _)| *id == channel)
			.map(|(_, text)| text)
	}
	pub fn reset(&mut self) {
		*self = Self {
			revision: self.revision.wrapping_add(1),
			..Self::default()
		};
	}
	pub fn cancel(&mut self) {
		self.activity.clear_notifications();
		self.pending = None;
		self.status = None;
	}
	pub fn forget(&mut self, channel: Id) {
		self.entries.remove(&channel);
		self.activity.forget(channel);
		if self.status(channel).is_some() {
			self.status = None;
		}
		let cancel = match self.pending.as_mut() {
			Some(Pending::Channel {
				channel: pending, ..
			}) => *pending == channel,
			Some(Pending::Guild { channels, .. }) => {
				channels.retain(|(id, ..)| *id != channel);
				channels.is_empty()
			}
			None => false,
		};
		if cancel {
			self.cancel();
		}
	}
}
impl State {
	pub fn read_marker(&self, channel: Id) -> Option<Option<Id>> {
		if !self.gateway_connected
			|| !self.can_view(channel)
			|| !self
				.channels
				.iter()
				.any(|c| c.id == channel && c.supports_text())
		{
			return None;
		}
		(self.read_state.known || self.read_state.entries.contains_key(&channel)).then(|| {
			self.read_state
				.entries
				.get(&channel)
				.and_then(|(id, _)| *id)
		})
	}
	pub fn unread(&self, channel: Id) -> Option<bool> {
		self.channel_unread(self.channel(channel)?)
	}
	/// Shared unread visibility for sidebar rows and notification badges.
	pub fn channel_unread(&self, channel: &model::Channel) -> Option<bool> {
		if !self.gateway_connected || !self.can_view(channel.id) || !channel.supports_text() {
			return None;
		}
		let read = self
			.read_state
			.entries
			.get(&channel.id)
			.map(|(id, _)| *id)?;
		let latest = channel.last_message?;
		let unread = read.is_none_or(|read| latest > read);
		if unread
			&& let Some(marker) = read
			&& self.selected == Some(channel.id)
			&& self.freshness == Freshness::Fresh
			&& !self.history_targeted
			&& self.history_before.is_none()
			&& self.history_after.is_none()
			&& !self.history_pending
			&& !self.timeline.is_empty()
			&& self.timeline.iter().all(|message| message.id <= marker)
		{
			return Some(false);
		}
		Some(unread)
	}
	pub fn can_jump_unread(&self) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness == Freshness::Fresh
			&& !self.history_pending
			&& self.selected.is_some_and(|channel| {
				self.can_read_history(channel) && self.unread(channel) == Some(true)
			})
	}
	/// Request the first bounded page after the service read marker. Zero is only
	/// a pagination cursor for a known empty marker, never a fabricated message ID.
	pub fn open_unread(&mut self) -> Option<crate::Command> {
		if !self.can_jump_unread() {
			return None;
		}
		let after = self.read_marker(self.selected?)?.unwrap_or(Id(0));
		Some(self.open_after_window(after))
	}
	/// The message the service counts as the selected channel's latest while the loaded page
	/// is the live edge: the newest page, without targeted browsing, older pages, or a page in
	/// flight. Any newer message would arrive over the gateway, so latest-message metadata
	/// above the timeline's newest row has outlived a deleted message. Acknowledging that ID,
	/// as the official client does, is what clears the phantom unread.
	pub fn live_edge_latest(&self) -> Option<Id> {
		let channel = self.selected?;
		if self.history_targeted
			|| self.history_before.is_some()
			|| self.history_after.is_some()
			|| self.history_pending
			|| self.freshness != Freshness::Fresh
			|| !self.gateway_connected
			|| !self.can_read_history(channel)
		{
			return None;
		}
		let latest = self.channel(channel)?.last_message?;
		self.timeline
			.iter()
			.last()
			.is_none_or(|newest| newest.id <= latest)
			.then_some(latest)
	}
	pub fn can_load_newer(&self) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness == Freshness::Fresh
			&& !self.history_pending
			// Nothing newer exists beyond the live edge; stale latest metadata is not a page.
			&& self.live_edge_latest().is_none()
			&& self
				.selected
				.is_some_and(|channel| self.can_read_history(channel))
			&& self.forward_cursor().is_some_and(|last| {
				self.channels.iter().any(|channel| {
					Some(channel.id) == self.selected
						&& channel.last_message.map_or(
							self.newer_may_have_more
								|| (self.history_targeted && self.history_after.is_none()),
							|latest| {
								latest > last
									&& (self.newer_may_have_more
										|| self.newer_cursor.is_some()
										|| self.history_after.is_none())
							},
						)
				})
			})
	}
	pub fn newer_history(&mut self) -> Option<crate::Command> {
		if !self.can_load_newer() {
			return None;
		}
		Some(self.history_range(None, Some(self.forward_cursor()?)))
	}
	fn forward_cursor(&self) -> Option<Id> {
		self.timeline
			.iter()
			.last()
			.map(|m| m.id)
			.or(self.newer_cursor)
	}
	fn open_after_window(&mut self, after: Id) -> crate::Command {
		self.timeline.clear_window_preserving_deletions();
		self.history_targeted = true;
		self.revision += 1;
		let command = self.history_range(None, Some(after));
		self.enforce_resident_budget();
		command
	}
	pub fn can_mark_read(&self, message: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness == Freshness::Fresh
			&& self.read_state.pending.is_none()
			&& (self
				.timeline
				.get(message)
				.is_some_and(|m| Some(m.channel) == self.selected)
				|| self.live_edge_latest() == Some(message))
			&& self.selected.is_some_and(|channel| {
				self.can_view(channel)
					&& self
						.read_marker(channel)
						.flatten()
						.is_none_or(|read| message > read)
			})
	}
	pub fn prepare_mark_read(&mut self, message: Id) -> Option<crate::Command> {
		if !self.can_mark_read(message) {
			return None;
		}
		let channel = self.selected?;
		Some(self.mark_read_command(channel, message, false))
	}
	/// Explicit sidebar acknowledgement uses known channel metadata without navigating.
	pub fn can_mark_channel_read(&self, channel: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.read_state.pending.is_none()
			&& self.unread(channel) == Some(true)
	}
	pub fn prepare_mark_channel_read(&mut self, channel: Id) -> Option<crate::Command> {
		if !self.can_mark_channel_read(channel) {
			return None;
		}
		let message = self.channel(channel)?.last_message?;
		Some(self.mark_read_command(channel, message, false))
	}
	pub fn can_mark_unread(&self, message: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness == Freshness::Fresh
			&& self.read_state.pending.is_none()
			&& message.0 > 0
			&& self
				.timeline
				.get(message)
				.is_some_and(|m| Some(m.channel) == self.selected)
			&& self.selected.is_some_and(|channel| {
				self.can_view(channel)
					&& self
						.read_marker(channel)
						.flatten()
						.is_some_and(|read| message <= read)
			})
	}
	pub fn prepare_mark_unread(&mut self, message: Id) -> Option<crate::Command> {
		if !self.can_mark_unread(message) {
			return None;
		}
		let channel = self.selected?;
		Some(self.mark_read_command(channel, Id(message.0 - 1), true))
	}
	pub fn can_mark_guild_read(&self, guild: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.read_state.pending.is_none()
			&& self.guilds.iter().any(|g| g.id == guild)
			&& self.channels.iter().any(|channel| {
				channel.guild == Some(guild) && self.unread(channel.id) == Some(true)
			})
	}
	pub fn prepare_mark_guild_read(&mut self, guild: Id) -> Option<crate::Command> {
		if !self.can_mark_guild_read(guild) {
			return None;
		}
		let channels = self
			.channels
			.iter()
			.filter(|channel| channel.guild == Some(guild) && self.unread(channel.id) == Some(true))
			.filter_map(|channel| {
				let message = channel.last_message?;
				let epoch = self
					.read_state
					.entries
					.get(&channel.id)
					.map_or(0, |(_, epoch)| *epoch);
				Some((channel.id, epoch, message))
			})
			.take(crate::MAX_NAV)
			.collect::<Vec<_>>();
		if channels.is_empty() {
			return None;
		}
		self.read_state.revision = self.read_state.revision.wrapping_add(1);
		let request = self.read_state.revision;
		self.read_state.pending = Some(Pending::Guild {
			guild,
			request,
			channels,
		});
		self.read_state.status = None;
		Some(crate::Command::MarkGuildRead { guild, request })
	}
	pub fn pending_guild_ack(&self, guild: Id, request: u64) -> bool {
		matches!(
			&self.read_state.pending,
			Some(Pending::Guild {
				guild: pending_guild,
				request: pending_request,
				..
			}) if *pending_guild == guild && *pending_request == request
		)
	}
	fn mark_read_command(&mut self, channel: Id, message: Id, manual: bool) -> crate::Command {
		self.read_state.revision = self.read_state.revision.wrapping_add(1);
		let request = self.read_state.revision;
		let epoch = self
			.read_state
			.entries
			.get(&channel)
			.map_or(0, |(_, epoch)| *epoch);
		self.read_state.pending = Some(Pending::Channel {
			channel,
			message,
			request,
			epoch,
			manual,
		});
		self.read_state.status = None;
		crate::Command::MarkRead {
			channel,
			message,
			request,
			manual,
		}
	}
	pub fn observe_last_message(&mut self, channel: Id, message: Id) {
		if let Some(channel) = self.channels.iter_mut().find(|c| c.id == channel) {
			self.read_state.activity.observe_latest(channel.id, message);
			if channel.last_message.is_none_or(|id| message > id) {
				channel.last_message = Some(message);
			}
		}
	}
	pub fn apply_read_state(&mut self, event: Event) -> Result<(), &'static str> {
		match event {
			Event::Snapshot {
				entries,
				version,
				partial,
			} => {
				if entries.as_ref().is_some_and(|items| {
					items.len() > crate::MAX_NAV
						|| items.capacity() * size_of::<(Id, Option<Id>, u32)>() > 16 * 1024 * 1024
				}) {
					self.read_state.reset();
					return Err("Read-state capacity exceeded");
				}
				self.read_state = ReadState {
					revision: self.read_state.revision.wrapping_add(1),
					known: entries.is_some() && !partial,
					version,
					..ReadState::default()
				};
				for channel in &self.channels {
					if let Some(message) = channel.last_message {
						self.read_state.activity.observe_latest(channel.id, message);
					}
				}
				for (channel, message, count) in entries.unwrap_or_default() {
					if !self.channel(channel).is_some_and(Channel::supports_text) {
						continue;
					}
					self.read_state.activity.set_count(channel, count);
					if self
						.read_state
						.entries
						.insert(channel, (message, self.read_state.revision))
						.is_some()
					{
						self.read_state.reset();
						return Err("Duplicate channel read state");
					}
				}
				if self.read_state.known {
					self.startup_warnings.read_state = false;
				}
			}
			Event::Ack {
				channel,
				message,
				manual,
				mention_count,
				version,
			} => {
				if !self
					.channels
					.iter()
					.any(|c| c.id == channel && c.supports_text())
					|| matches!((self.read_state.version,version),(Some(old),Some(new)) if new<old)
				{
					return Ok(());
				}
				self.read_state.version = version.or(self.read_state.version);
				self.read_state.revision = self.read_state.revision.wrapping_add(1);
				let current = self
					.read_state
					.entries
					.get(&channel)
					.and_then(|(id, _)| *id);
				let message = if manual {
					message
				} else {
					message.max(current)
				};
				self.read_state
					.activity
					.ack(channel, message, mention_count);
				self.read_state
					.entries
					.insert(channel, (message, self.read_state.revision));
				if self.read_state.status(channel).is_some() {
					self.read_state.status = None;
				}
			}
			Event::Latest(channels) => {
				if channels.len() > crate::MAX_NAV {
					return Err("Channel update capacity exceeded");
				}
				for (id, latest) in channels {
					if let Some(index) = self.channel_index(id) {
						let channel = &mut self.channels[index];
						if let Some(previous) = channel.last_message {
							self.read_state
								.activity
								.observe_latest(channel.id, previous);
						}
						match latest {
							Patch::Value(id) => channel.last_message = Some(id),
							Patch::Null => channel.last_message = None,
							Patch::Absent => {}
						}
					}
				}
			}
			Event::Result {
				channel,
				message,
				request,
				result,
			} => {
				let Some(Pending::Channel {
					channel: pending_channel,
					message: pending_message,
					request: pending_request,
					epoch,
					manual,
				}) = self.read_state.pending
				else {
					return Ok(());
				};
				if (channel, message, request)
					!= (pending_channel, pending_message, pending_request)
				{
					return Ok(());
				}
				self.read_state.pending = None;
				if !self.can_view(channel) {
					self.read_state.status = None;
					return Ok(());
				}
				match result {
					Ok(()) => {
						if self.channels.iter().any(|c| c.id == channel)
							&& self
								.read_state
								.entries
								.get(&channel)
								.map_or(0, |(_, epoch)| *epoch)
								== epoch
						{
							self.read_state.revision = self.read_state.revision.wrapping_add(1);
							let current = self
								.read_state
								.entries
								.get(&channel)
								.and_then(|(id, _)| *id);
							let next = if manual {
								Some(message)
							} else {
								Some(message).max(current)
							};
							self.read_state.activity.ack(channel, next, None);
							self.read_state
								.entries
								.insert(channel, (next, self.read_state.revision));
						}
						self.read_state.status = None;
					}
					Err(failure) => {
						if failure.ends_session() {
							self.fail(failure);
						} else if self
							.read_state
							.entries
							.get(&channel)
							.map_or(0, |(_, epoch)| *epoch)
							== epoch
						{
							self.read_state.status = Some((
								channel,
								match failure {
									Failure::Ambiguous => {
										"Read status could not be confirmed · unread markers may be out of date"
									}
									_ => {
										"Read status could not sync · unread markers may be out of date"
									}
								},
							));
						}
					}
				}
			}
			Event::GuildAck {
				guild,
				request,
				result,
			} => {
				let Some(Pending::Guild {
					guild: pending_guild,
					request: pending_request,
					..
				}) = &self.read_state.pending
				else {
					return Ok(());
				};
				if (*pending_guild, *pending_request) != (guild, request) {
					return Ok(());
				}
				let Some(Pending::Guild { channels, .. }) = self.read_state.pending.take() else {
					return Ok(());
				};
				match result {
					Ok(()) => {
						self.read_state.revision = self.read_state.revision.wrapping_add(1);
						let revision = self.read_state.revision;
						for (channel, epoch, message) in channels {
							if self
								.read_state
								.entries
								.get(&channel)
								.map_or(0, |(_, current)| *current)
								!= epoch
							{
								continue;
							}
							self.read_state
								.activity
								.ack(channel, Some(message), Some(0));
							self.read_state
								.entries
								.insert(channel, (Some(message), revision));
						}
						self.read_state.status = None;
					}
					Err(failure) => {
						if failure.ends_session() {
							self.fail(failure);
						} else if let Some((channel, ..)) = channels.first() {
							self.read_state.status = Some((
								*channel,
								match failure {
									Failure::Ambiguous => {
										"Read status could not be confirmed · unread markers may be out of date"
									}
									_ => {
										"Read status could not sync · unread markers may be out of date"
									}
								},
							));
						}
					}
				}
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod navigation_tests {
	use super::*;
	use crate::{Command, Envelope, Event as CoreEvent, Reply};
	use model::{Channel, Message, User};
	fn message(id: u64) -> Message {
		Message {
			id: Id(id),
			channel: Id(1),
			author: User {
				id: Id(9),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			content: "Synthetic unread message".into(),
			reactions: Some(vec![]),
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: vec![],
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			reply_deleted: false,
			forwarded: false,
			kind: 0,
			unsupported: false,
			extra_content: Default::default(),
			embeds: vec![],
			attachments: vec![],
			embeds_suppressed: false,
		}
	}
	fn state(marker: Option<Id>) -> State {
		let mut state = State {
			user: Some(message(1).author),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			selected: Some(Id(1)),
			channels: vec![Channel {
				id: Id(1),
				guild: None,
				parent_id: None,
				kind: 1,
				position: 0,
				name: "Synthetic DM".into(),
				recipients: vec![],
				last_message: Some(Id(500)),
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..Default::default()
		};
		state
			.apply_read_state(Event::Snapshot {
				entries: Some(vec![(Id(1), marker, 0)]),
				version: None,
				partial: false,
			})
			.unwrap();
		state.timeline.insert(message(500), false, false).unwrap();
		state.drafts.insert(Id(1), "Preserve draft".into());
		state.reply = Some(Reply::to(Id(500)));
		state
	}
	fn apply(state: &mut State, event: CoreEvent) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
	#[test]
	fn read_failure_is_scoped_and_service_ack_resolves_it_in_either_order() {
		for ack_first in [false, true] {
			let mut state = state(Some(Id(100)));
			let mut other = state.channels[0].clone();
			other.id = Id(2);
			state.channels.push(other);
			let Command::MarkRead {
				channel,
				message,
				request,
				manual: false,
			} = state.prepare_mark_read(Id(500)).unwrap()
			else {
				panic!("expected read acknowledgement");
			};
			state.select(Id(2));
			let ack = Event::Ack {
				channel,
				message: Some(message),
				manual: false,
				mention_count: None,
				version: None,
			};
			if ack_first {
				state.apply_read_state(ack).unwrap();
			}
			state
				.apply_read_state(Event::Result {
					channel,
					message,
					request,
					result: Err(Failure::Ambiguous),
				})
				.unwrap();
			assert!(state.read_state.status(Id(2)).is_none());
			assert_eq!(state.read_state.status(Id(1)).is_some(), !ack_first);
			if !ack_first {
				assert!(
					state
						.read_state
						.status(Id(1))
						.unwrap()
						.starts_with("Read status")
				);
				state
					.apply_read_state(Event::Ack {
						channel,
						message: Some(message),
						manual: false,
						mention_count: None,
						version: None,
					})
					.unwrap();
			}
			assert!(state.read_state.status(Id(1)).is_none());
			assert_eq!(state.read_marker(Id(1)), Some(Some(message)));
		}
	}
	fn page(state: &mut State, messages: Vec<Message>) {
		apply(
			state,
			CoreEvent::History {
				channel: Id(1),
				request: state.request,
				messages,
				older: false,
			},
		);
	}
	#[test]
	fn sidebar_acknowledges_unselected_channel_without_touching_navigation_or_draft() {
		let mut state = state(Some(Id(100)));
		state.selected = None;
		assert!(state.can_mark_channel_read(Id(1)));
		let Some(Command::MarkRead {
			channel,
			message,
			request,
			manual: false,
		}) = state.prepare_mark_channel_read(Id(1))
		else {
			panic!("sidebar acknowledgement")
		};
		assert_eq!((channel, message), (Id(1), Id(500)));
		assert!(state.selected.is_none());
		assert_eq!(state.drafts[&Id(1)], "Preserve draft");
		assert!(state.prepare_mark_channel_read(Id(1)).is_none());
		state
			.apply_read_state(Event::Result {
				channel,
				message,
				request,
				result: Ok(()),
			})
			.unwrap();
		assert!(!state.can_mark_channel_read(Id(1)));
		state.gateway_connected = false;
		assert!(state.prepare_mark_channel_read(Id(1)).is_none());
		assert!(state.prepare_mark_channel_read(Id(999)).is_none());
	}
	#[test]
	fn unread_and_next_pages_are_bounded_scoped_and_do_not_acknowledge() {
		for marker in [None, Some(Id(100))] {
			let mut state = state(marker);
			let start = marker.unwrap_or(Id(0)).0;
			assert!(state.can_jump_unread());
			assert!(
				matches!(state.open_unread(),Some(Command::History {before:None,after:Some(after),..}) if after.0==start)
			);
			assert!(state.history_targeted && state.history_pending);
			assert!(!state.can_jump_unread() && !state.can_load_newer());
			let mut incoming = message(501);
			incoming.author.id = Id(8);
			apply(&mut state, CoreEvent::Message(incoming));
			assert!(state.timeline.get(Id(501)).is_none());
			// Deletion racing the response cannot become the scroll target.
			apply(
				&mut state,
				CoreEvent::Delete {
					channel: Id(1),
					id: Id(start + 1),
				},
			);
			page(
				&mut state,
				(start + 1..=start + 50).map(message).rev().collect(),
			);
			assert_eq!(state.search_target, Some(Id(start + 2)));
			assert_eq!(state.timeline.iter().count(), 49);
			assert_eq!(state.read_marker(Id(1)), Some(marker));
			assert_eq!(state.drafts[&Id(1)], "Preserve draft");
			assert_eq!(state.reply_target(), Some(Id(500)));
			assert!(state.read_state.pending.is_none());
			assert!(
				matches!(state.newer_history(),Some(Command::History {before:None,after:Some(after),..}) if after.0==start+50)
			);
			page(
				&mut state,
				(start + 51..=start + 100).map(message).collect(),
			);
			assert!(state.search_target.is_none());
			assert_eq!(state.timeline.iter().count(), 99);
			assert_eq!(
				state.timeline.iter().next().map(|m| m.id),
				Some(Id(start + 2))
			);
			assert_eq!(
				state.timeline.iter().last().map(|m| m.id),
				Some(Id(start + 100))
			);
			assert!(!state.older_exhausted);
			assert!(state.can_load_older());
			assert_eq!(state.read_marker(Id(1)), Some(marker));
			let Command::History { before, after, .. } = state.history(None) else {
				panic!()
			};
			assert!(before.is_none() && after.is_none() && !state.history_targeted);
		}
	}
	#[test]
	fn historical_sends_confirm_without_splicing_a_live_tail_in_either_order() {
		for gateway_first in [false, true] {
			let mut state = state(Some(Id(100)));
			state.open_unread().unwrap();
			page(&mut state, (101..=150).map(message).collect());
			let Command::Send { nonce, .. } = state.prepare_send().unwrap() else {
				panic!()
			};
			let mut sent = message(501);
			sent.nonce = Some(nonce.clone());
			if gateway_first {
				apply(&mut state, CoreEvent::Message(sent.clone()));
			}
			apply(
				&mut state,
				CoreEvent::SendResult {
					nonce,
					result: Ok(sent.clone()),
				},
			);
			if !gateway_first {
				apply(&mut state, CoreEvent::Message(sent));
			}
			assert!(state.timeline.get(Id(501)).is_none());
			assert_eq!(state.channels[0].last_message, Some(Id(501)));
			assert!(
				state
					.pending
					.iter()
					.all(|p| p.delivery == model::Delivery::Confirmed)
			);
			assert!(matches!(
				state.newer_history(),
				Some(Command::History {
					after: Some(Id(150)),
					..
				})
			));
		}
	}
	#[test]
	fn deleted_pages_and_unknown_latest_keep_a_forward_cursor_without_resurrecting_messages() {
		let mut state = state(Some(Id(100)));
		state.open_unread().unwrap();
		apply(
			&mut state,
			CoreEvent::DeleteBulk {
				channel: Id(1),
				ids: (101..=150).map(Id).collect(),
			},
		);
		apply(
			&mut state,
			CoreEvent::Delete {
				channel: Id(1),
				id: Id(500),
			},
		);
		page(&mut state, (101..=150).map(message).collect());
		assert_eq!(state.timeline.iter().count(), 0);
		assert!(state.search_target.is_none());
		assert!(matches!(
			state.newer_history(),
			Some(Command::History {
				after: Some(Id(150)),
				..
			})
		));
		page(&mut state, (151..=200).map(message).collect());
		let mut reply = message(601);
		reply.reply_to = Some(Id(151));
		reply.reply_deleted = true;
		reply.kind = 19;
		apply(&mut state, CoreEvent::Message(reply));
		assert!(state.timeline.is_deleted(Id(151)));
		assert!(state.timeline.get(Id(601)).is_none());
		assert!(matches!(
			state.newer_history(),
			Some(Command::History {
				after: Some(Id(200)),
				..
			})
		));
		page(&mut state, vec![]);
		assert!(!state.can_load_newer());
	}
	#[test]
	fn replacing_an_after_page_with_a_missing_target_drops_its_forward_cursor() {
		let mut state = state(Some(Id(100)));
		state.open_unread().unwrap();
		page(&mut state, (101..=150).map(message).collect());
		state.open_target_window(Id(50)).unwrap();
		let request = state.request;
		apply(
			&mut state,
			CoreEvent::History {
				channel: Id(1),
				request,
				older: true,
				messages: vec![],
			},
		);
		assert!(state.newer_cursor.is_none());
		assert!(!state.can_load_newer());
		assert_eq!(state.search_target, Some(Id(50)));
	}
	#[test]
	fn unread_navigation_rejects_unavailable_scope_bad_ranges_and_late_results() {
		for invalid in 0..6 {
			let mut state = state(Some(Id(100)));
			match invalid {
				0 => state.gateway_connected = false,
				1 => state.auth = AuthState::Unauthenticated,
				2 => state.freshness = Freshness::Stale,
				3 => state.history_pending = true,
				4 => state.read_state.reset(),
				_ => state.channels.clear(),
			}
			assert!(!state.can_jump_unread());
			assert!(state.open_unread().is_none());
			assert_eq!(state.timeline.row_count(), 1);
			assert_eq!(state.drafts[&Id(1)], "Preserve draft");
		}
		let mut state = state(Some(Id(100)));
		state.open_unread().unwrap();
		page(&mut state, vec![message(100)]);
		assert_ne!(state.freshness, Freshness::Fresh);
		assert!(state.timeline.get(Id(100)).is_none());
		let mut state = self::state(Some(Id(100)));
		state.open_unread().unwrap();
		let stale = state.request;
		state.history(None);
		apply(
			&mut state,
			CoreEvent::History {
				channel: Id(1),
				request: stale,
				older: false,
				messages: vec![message(101)],
			},
		);
		assert!(state.timeline.get(Id(101)).is_none());
		page(&mut state, vec![message(500)]);
		assert!(!state.history_targeted && state.search_target.is_none());
		state.open_unread().unwrap();
		page(&mut state, vec![]);
		assert!(
			state
				.status
				.starts_with("No messages returned after this boundary")
		);
		assert!(state.history_targeted && state.search_target.is_none());
		assert!(!state.can_load_newer());
		assert!(state.read_state.pending.is_none());
	}
}
