use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Freshness, Id, Patch};
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
	/// `acked` leading channels were confirmed before any failure.
	AllAck {
		request: u64,
		acked: usize,
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
		mention_count: Option<u32>,
	},
	Guild {
		guild: Id,
		request: u64,
		channels: Vec<(Id, u64, Id)>,
	},
	All {
		request: u64,
		/// In request order; a forgotten channel leaves `None` so `acked` still lines up.
		channels: Vec<Option<(Id, u64, Id)>>,
		scope: Scope,
		/// More unread channels remained past `MAX_BULK_ACK`.
		more: bool,
	},
}

/// Channels a bulk acknowledgement covers.
#[derive(Clone)]
enum Scope {
	/// Every server channel and direct message.
	All,
	/// Channels of these servers, sorted and bounded by the folder guild limit.
	Guilds(Box<[Id]>),
}
impl Scope {
	fn guilds(guilds: &[Id]) -> Self {
		let mut guilds = guilds
			.iter()
			.copied()
			.take(model::guild_folders::MAX_GUILDS)
			.collect::<Vec<_>>();
		guilds.sort_unstable();
		guilds.dedup();
		Self::Guilds(guilds.into())
	}
	fn covers(&self, guild: Option<Id>) -> bool {
		match self {
			Self::All => true,
			Self::Guilds(guilds) => guild.is_some_and(|guild| guilds.binary_search(&guild).is_ok()),
		}
	}
}

#[derive(Default)]
pub struct ReadState {
	pub(crate) activity: crate::notifications::Activity,
	entries: BTreeMap<Id, (Option<Id>, u64)>,
	known: bool,
	version: Option<u64>,
	revision: u64,
	pending: Option<Pending>,
	/// A capped or reconnect-interrupted bulk acknowledgement asks for its next batch.
	continue_all: Option<Scope>,
	pub(crate) status: Option<(Id, &'static str)>,
}
impl ReadState {
	/// True once a complete service snapshot arrived, so a channel without a row was never read.
	pub(crate) fn known(&self) -> bool {
		self.known
	}
	/// The service's acknowledged message for a channel that has a read row.
	pub(crate) fn row(&self, channel: Id) -> Option<Option<Id>> {
		self.entries.get(&channel).map(|(id, _)| *id)
	}
	pub(crate) fn busy(&self) -> bool {
		self.pending.is_some()
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
	/// Drops pending work; an interrupted bulk acknowledgement resumes once reads can start again.
	pub fn cancel(&mut self) {
		self.activity.clear_notifications();
		if let Some(Pending::All { scope, .. }) = self.pending.take() {
			self.continue_all = Some(scope);
		}
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
			Some(Pending::All { channels, .. }) => {
				for slot in channels.iter_mut() {
					if slot.is_some_and(|(id, ..)| id == channel) {
						*slot = None;
					}
				}
				channels.iter().all(Option::is_none)
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
				.channel(channel)
				.is_some_and(|c| c.supports_text() || matches!(c.kind, 15 | 16))
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
		if !self.gateway_connected
			|| !self.can_view(channel.id)
			|| !(channel.supports_text() || matches!(channel.kind, 15 | 16))
		{
			return None;
		}
		let read = match self.read_state.entries.get(&channel.id) {
			Some((id, _)) => *id,
			None if self.read_state.known && matches!(channel.kind, 10..=12) => None,
			None => return None,
		};
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
			&& self
				.timeline
				.iter()
				.next_back()
				.is_some_and(|message| message.id <= marker)
		{
			return Some(false);
		}
		Some(unread)
	}
	pub fn missed(&self, channel: Id) -> Option<bool> {
		match self.unread(channel) {
			Some(unread) => Some(unread),
			None if self.unread_count(channel) > 0 => Some(true),
			None => None,
		}
	}
	fn history_window_ready(&self) -> bool {
		if self.freshness == Freshness::Fresh {
			!self.history_pending
		} else {
			self.freshness == Freshness::Loading
				&& self.history_pending
				&& !self.history_targeted
				&& self.history_before.is_none()
				&& self.history_after.is_none()
		}
	}
	pub fn show_missed_banner(&self) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.history_window_ready()
			&& self.selected.is_some_and(|channel| {
				self.can_read_history(channel) && self.missed(channel) == Some(true)
			})
	}
	pub fn can_jump_unread(&self) -> bool {
		self.show_missed_banner()
			&& self
				.selected
				.is_some_and(|channel| self.read_marker(channel).is_some())
	}
	/// Scroll to a loaded unread boundary, or request its first bounded page.
	/// Zero is only a pagination cursor for a known empty marker.
	pub fn open_unread(&mut self) -> Option<crate::Command> {
		if !self.can_jump_unread() {
			return None;
		}
		let after = self.read_marker(self.selected?)?.unwrap_or(Id(0));
		if self.freshness == Freshness::Fresh
			&& !self.history_pending
			&& (self.older_exhausted
				|| self
					.timeline
					.row_ids()
					.next()
					.is_some_and(|first| first <= after))
			&& let Some(target) = self
				.timeline
				.iter()
				.find(|message| message.id > after)
				.map(|message| message.id)
		{
			self.search_target = Some(target);
			self.revision += 1;
			return None;
		}
		Some(self.open_after_window(after))
	}
	/// The message the service counts as the selected channel's latest while the loaded page
	/// is the live edge: the newest page, without targeted browsing or a page in flight.
	/// Older pagination must still retain the exact latest message. On the newest page, metadata
	/// above the timeline's newest row has outlived a deleted message. Acknowledging that ID,
	/// as the official client does, is what clears the phantom unread.
	pub fn live_edge_latest(&self) -> Option<Id> {
		let channel = self.selected?;
		if self.history_targeted
			|| self.history_after.is_some()
			|| self.history_pending
			|| self.freshness != Freshness::Fresh
			|| !self.gateway_connected
			|| !self.can_read_history(channel)
		{
			return None;
		}
		let latest = self.channel(channel)?.last_message?;
		if self.history_before.is_some() {
			return self
				.timeline
				.iter()
				.next_back()
				.is_some_and(|newest| newest.id == latest)
				.then_some(latest);
		}
		self.timeline
			.iter()
			.next_back()
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
			.next_back()
			.map(|m| m.id)
			.max(self.newer_cursor)
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
		Some(self.mark_read_command(channel, message, false, None))
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
		Some(self.mark_read_command(channel, message, false, None))
	}
	/// Leaving a channel acknowledges the latest message the reader had on screen there,
	/// even when a short conversation never let them scroll toward the bottom.
	pub fn prepare_mark_left_channel_read(
		&mut self,
		channel: Id,
		message: Id,
	) -> Option<crate::Command> {
		if self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.read_state.pending.is_some()
			|| self
				.channel(channel)?
				.last_message
				.is_none_or(|latest| message > latest)
			|| self
				.read_marker(channel)?
				.is_some_and(|read| message <= read)
		{
			return None;
		}
		Some(self.mark_read_command(channel, message, false, None))
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
		let cursor = Id(message.0 - 1);
		let mention_count = self
			.private_loaded_unread_count(channel, cursor)
			.or_else(|| self.loaded_guild_mentions(channel, cursor));
		Some(self.mark_read_command(channel, cursor, true, mention_count))
	}
	fn private_loaded_unread_count(&self, channel: Id, after: Id) -> Option<u32> {
		self.channel(channel)
			.filter(|known| known.guild.is_none())
			.map(|_| self.loaded_private_unreads(channel, Some(after)))
	}
	fn loaded_private_unreads(&self, channel: Id, after: Option<Id>) -> u32 {
		const RECIPIENT_REMOVE: u8 = 8;
		let count = self
			.timeline
			.iter()
			.filter(|message| {
				message.channel == channel
					&& after.is_none_or(|cursor| message.id > cursor)
					&& message.kind != RECIPIENT_REMOVE
			})
			.count();
		u32::try_from(count).unwrap_or(u32::MAX)
	}
	fn loaded_guild_mentions(&self, channel: Id, after: Id) -> Option<u32> {
		if self
			.channel(channel)
			.is_none_or(|known| known.guild.is_none())
		{
			return None;
		}
		let Some(previous) = self.read_marker(channel).flatten() else {
			return Some(self.mention_count(channel));
		};
		if after >= previous {
			return Some(self.mention_count(channel));
		}
		let added = self
			.timeline
			.iter()
			.filter(|message| {
				message.channel == channel
					&& message.id > after
					&& message.id <= previous
					&& self.counts_toward_mention_badge(message)
			})
			.count();
		Some(
			self.mention_count(channel)
				.saturating_add(u32::try_from(added).unwrap_or(u32::MAX)),
		)
	}
	fn manual_private_count(
		&self,
		channel: Id,
		after: Option<Id>,
		echoed: Option<u32>,
	) -> Option<u32> {
		if echoed.is_some() {
			return echoed;
		}
		if let Some(Pending::Channel {
			channel: pending,
			message,
			manual: true,
			mention_count,
			..
		}) = &self.read_state.pending
			&& *pending == channel
			&& after == Some(*message)
		{
			return *mention_count;
		}
		if self
			.channel(channel)
			.is_some_and(|known| known.guild.is_some())
		{
			return after.and_then(|cursor| self.loaded_guild_mentions(channel, cursor));
		}
		let loaded = self.loaded_private_unreads(channel, after);
		if loaded > 0 {
			Some(loaded)
		} else {
			Some(self.unread_count(channel))
		}
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
			.unread_acks(&Scope::Guilds(Box::new([guild])))
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
	/// Unread channels in `scope` the account can see.
	fn unread_acks<'a>(&'a self, scope: &'a Scope) -> impl Iterator<Item = (Id, u64, Id)> + 'a {
		self.channels
			.iter()
			.filter(move |channel| {
				scope.covers(channel.guild) && self.unread(channel.id) == Some(true)
			})
			.filter_map(|channel| {
				let message = channel.last_message?;
				let epoch = self
					.read_state
					.entries
					.get(&channel.id)
					.map_or(0, |(_, epoch)| *epoch);
				Some((channel.id, epoch, message))
			})
	}
	fn can_bulk_ack(&self, scope: &Scope) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.read_state.pending.is_none()
			&& self.unread_acks(scope).next().is_some()
	}
	pub fn can_mark_all_read(&self) -> bool {
		self.can_bulk_ack(&Scope::All)
	}
	/// Any unread channel in these servers, such as a server folder.
	pub fn can_mark_guilds_read(&self, guilds: &[Id]) -> bool {
		self.can_bulk_ack(&Scope::guilds(guilds))
	}
	/// One bulk acknowledgement for every unread channel; capped batches continue on success.
	pub fn prepare_mark_all_read(&mut self) -> Option<crate::Command> {
		self.prepare_bulk_ack(Scope::All)
	}
	/// One bulk acknowledgement for the unread channels of these servers.
	pub fn prepare_mark_guilds_read(&mut self, guilds: &[Id]) -> Option<crate::Command> {
		self.prepare_bulk_ack(Scope::guilds(guilds))
	}
	fn prepare_bulk_ack(&mut self, scope: Scope) -> Option<crate::Command> {
		if !self.can_bulk_ack(&scope) {
			return None;
		}
		let (channels, more) = {
			let mut unread = self.unread_acks(&scope);
			let channels = unread
				.by_ref()
				.take(crate::MAX_BULK_ACK)
				.collect::<Vec<_>>();
			(channels, unread.next().is_some())
		};
		self.read_state.revision = self.read_state.revision.wrapping_add(1);
		let request = self.read_state.revision;
		let acks = channels
			.iter()
			.map(|(channel, _, message)| (*channel, *message))
			.collect();
		self.read_state.pending = Some(Pending::All {
			request,
			channels: channels.into_iter().map(Some).collect(),
			scope,
			more,
		});
		self.read_state.continue_all = None;
		self.read_state.status = None;
		Some(crate::Command::MarkAllRead {
			request,
			channels: acks,
		})
	}
	/// Next batch after a capped bulk acknowledgement succeeded or a reconnect interrupted one.
	pub fn continue_mark_all_read(&mut self) -> Option<crate::Command> {
		// Kept while another read or a reconnect is in progress.
		if self.read_state.continue_all.is_none()
			|| self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.read_state.pending.is_some()
		{
			return None;
		}
		let scope = self.read_state.continue_all.take()?;
		self.prepare_bulk_ack(scope)
	}
	pub fn pending_all_ack(&self, request: u64) -> bool {
		matches!(
			&self.read_state.pending,
			Some(Pending::All { request: pending, .. }) if *pending == request
		)
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
	pub(crate) fn mark_read_command(
		&mut self,
		channel: Id,
		message: Id,
		manual: bool,
		mention_count: Option<u32>,
	) -> crate::Command {
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
			mention_count,
		});
		self.read_state.status = None;
		crate::Command::MarkRead {
			channel,
			message,
			request,
			manual,
			mention_count,
		}
	}
	/// Advance the observed latest message ID for a channel and record read-state activity.
	pub fn observe_last_message(&mut self, channel: Id, message: Id) {
		if let Some(index) = self.channel_index(channel) {
			let channel = &mut self.channels[index];
			self.read_state.activity.observe_latest(channel.id, message);
			if channel.last_message.is_none_or(|id| message > id) {
				channel.last_message = Some(message);
			}
		}
	}
	pub fn apply_read_state(&mut self, event: Event) -> Result<(), &'static str> {
		let new_channel = match &event {
			Event::Ack { channel, .. } | Event::Result { channel, .. } => Some(*channel),
			_ => None,
		};
		if new_channel.is_some_and(|id| !self.read_state.entries.contains_key(&id))
			&& self.read_state.entries.len() >= crate::MAX_NAV
		{
			self.read_state.cancel();
			return Err("Read-state capacity exceeded");
		}
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
					// Unjoined forum posts load after READY; keep their service cursors.
					if self
						.channel(channel)
						.is_some_and(|c| !c.supports_text() && !matches!(c.kind, 15 | 16))
					{
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
					.any(|c| c.id == channel && (c.supports_text() || matches!(c.kind, 15 | 16)))
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
				let mention_count = if manual {
					self.manual_private_count(channel, message, mention_count)
				} else {
					mention_count
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
					mention_count,
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
							self.read_state.activity.ack(
								channel,
								next,
								if manual { mention_count } else { None },
							);
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
				self.finish_bulk_ack(channels, result);
			}
			Event::AllAck {
				request,
				acked,
				result,
			} => {
				if !self.pending_all_ack(request) {
					return Ok(());
				}
				let Some(Pending::All {
					mut channels,
					scope,
					more,
					..
				}) = self.read_state.pending.take()
				else {
					return Ok(());
				};
				let failed = channels.split_off(acked.min(channels.len()));
				self.finish_bulk_ack(channels.into_iter().flatten().collect(), Ok(()));
				match result {
					Ok(()) => self.read_state.continue_all = more.then_some(scope),
					Err(failure) => {
						self.finish_bulk_ack(failed.into_iter().flatten().collect(), Err(failure))
					}
				}
			}
		}
		Ok(())
	}
	fn finish_bulk_ack(&mut self, channels: Vec<(Id, u64, Id)>, result: Result<(), Failure>) {
		match result {
			Ok(()) => {
				self.read_state.revision = self.read_state.revision.wrapping_add(1);
				let revision = self.read_state.revision;
				for (channel, epoch, message) in channels {
					if !self.read_state.entries.contains_key(&channel)
						&& self.read_state.entries.len() >= crate::MAX_NAV
					{
						continue;
					}
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
						// Messages newer than this cursor keep their badge until the Gateway ACK.
						.ack(channel, Some(message), None);
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
							_ => "Read status could not sync · unread markers may be out of date",
						},
					));
				}
			}
		}
	}
}

#[cfg(test)]
mod navigation_tests {
	use super::*;
	use crate::{Command, Envelope, Event as CoreEvent, Reply};
	use model::{Channel, Message, User};
	fn message(id: u64) -> Message {
		Message {
			poll: None,
			sticker_items: Vec::new(),
			id: Id(id),
			channel: Id(1),
			author: User {
				primary_guild: None,
				cosmetics: None,
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
			interaction: None,
			forwarded: false,
			kind: 0,
			unsupported: false,
			components: vec![],
			application_id: None,
			flags: 0,
			ephemeral: false,
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
				tags: None,
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
	#[test]
	fn newest_live_cursor_skips_deleted_tail_payloads_and_placeholders() {
		for preserve in [false, true] {
			let mut state = state(Some(Id(100)));
			state.timeline.clear();
			state.set_preserve_deleted_messages(preserve);
			for id in 1..=500 {
				state.timeline.insert(message(id), false, false).unwrap();
			}
			for id in [Id(499), Id(500)] {
				state.timeline.delete(id).unwrap();
			}
			assert_eq!(state.timeline.get_display(Id(500)).is_some(), preserve);
			assert_eq!(state.forward_cursor(), Some(Id(498)));
			// Recent history may acknowledge metadata for an already deleted latest row.
			assert_eq!(state.live_edge_latest(), Some(Id(500)));
			state.history_before = Some(Id(1));
			assert_eq!(state.live_edge_latest(), None);
			state.channels[0].last_message = Some(Id(498));
			assert_eq!(state.live_edge_latest(), Some(Id(498)));
			state.newer_cursor = Some(Id(501));
			assert_eq!(state.forward_cursor(), Some(Id(501)));
			state.newer_cursor = None;
			for id in 1..=498 {
				state.timeline.delete(Id(id)).unwrap();
			}
			assert_eq!(state.forward_cursor(), None);
			assert_eq!(state.live_edge_latest(), None);
			state.history_before = None;
			assert_eq!(state.live_edge_latest(), Some(Id(498)));
		}
	}
	#[test]
	fn unread_tail_check_preserves_partial_markers_and_deleted_only_histories() {
		for preserve in [false, true] {
			let mut state = state(Some(Id(497)));
			state.timeline.clear();
			state.set_preserve_deleted_messages(preserve);
			for id in 1..=500 {
				state.timeline.insert(message(id), false, false).unwrap();
			}
			for id in [Id(499), Id(500)] {
				state.timeline.delete(id).unwrap();
			}
			assert_eq!(state.unread(Id(1)), Some(true));
			state.read_state.entries.get_mut(&Id(1)).unwrap().0 = Some(Id(498));
			assert_eq!(state.unread(Id(1)), Some(false));
			for id in 1..=498 {
				state.timeline.delete(Id(id)).unwrap();
			}
			// Deleted-only history cannot establish that every unread row is loaded.
			assert_eq!(state.unread(Id(1)), Some(true));
		}
	}
	#[test]
	fn marking_a_dm_or_group_unread_restores_the_badge_count() {
		{
			for kind in [1, 3] {
				for ack_first in [false, true] {
					let mut state = state(Some(Id(500)));
					state.channels[0].kind = kind;
					state.timeline.insert(message(498), false, false).unwrap();
					state.timeline.insert(message(499), false, false).unwrap();
					let mut removed = message(501);
					removed.kind = 8;
					state.timeline.insert(removed, false, false).unwrap();
					assert_eq!(state.unread_count(Id(1)), 0);
					let Command::MarkRead {
						channel,
						message,
						request,
						manual,
						mention_count,
					} = state.prepare_mark_unread(Id(498)).unwrap()
					else {
						panic!("mark unread");
					};
					assert!(manual);
					assert_eq!((message, mention_count), (Id(497), Some(3)));
					let ack = Event::Ack {
						channel,
						message: Some(message),
						manual: true,
						mention_count: None,
						version: None,
					};
					let result = Event::Result {
						channel,
						message,
						request,
						result: Ok(()),
					};
					if ack_first {
						state.apply_read_state(ack).unwrap();
						state.apply_read_state(result).unwrap();
					} else {
						state.apply_read_state(result).unwrap();
						state.apply_read_state(ack).unwrap();
					}
					assert_eq!(state.unread(Id(1)), Some(true));
					assert_eq!(state.unread_directs(None), vec![Id(1)]);
					assert_eq!(state.unread_count(Id(1)), 3);
				}
			}
		}
		{
			let mut state = state(Some(Id(100)));
			state.selected = None;
			assert!(state.can_mark_channel_read(Id(1)));
			let Some(Command::MarkRead {
				channel,
				message,
				request,
				manual: false,
				mention_count: None,
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
	}
	#[test]
	fn mark_all_read_keeps_confirmed_chunks_when_a_later_one_fails() {
		let mut state = state(Some(Id(100)));
		let mut other = state.channels[0].clone();
		other.id = Id(2);
		state.channels.push(other);
		state
			.apply_read_state(Event::Snapshot {
				entries: Some(vec![(Id(1), Some(Id(100)), 0), (Id(2), Some(Id(100)), 0)]),
				version: None,
				partial: false,
			})
			.unwrap();
		let Some(Command::MarkAllRead { request, channels }) = state.prepare_mark_all_read() else {
			panic!("mark all read");
		};
		assert_eq!(channels, vec![(Id(1), Id(500)), (Id(2), Id(500))]);
		assert!(!state.can_mark_all_read());
		state
			.apply_read_state(Event::AllAck {
				request,
				acked: 1,
				result: Err(Failure::RateLimited),
			})
			.unwrap();
		assert_eq!(state.unread(Id(1)), Some(false));
		assert_eq!(state.unread(Id(2)), Some(true));
		assert!(state.read_state.status(Id(2)).is_some());
		assert!(state.continue_mark_all_read().is_none());
		let Some(Command::MarkAllRead { request, channels }) = state.prepare_mark_all_read() else {
			panic!("retry");
		};
		assert_eq!(channels, vec![(Id(2), Id(500))]);
		state
			.apply_read_state(Event::AllAck {
				request,
				acked: 1,
				result: Ok(()),
			})
			.unwrap();
		assert_eq!(state.unread(Id(2)), Some(false));
		assert!(!state.can_mark_all_read());
		assert!(state.continue_mark_all_read().is_none());
	}
	fn with_unread_dms(count: u64) -> State {
		let mut state = state(Some(Id(100)));
		let template = state.channels[0].clone();
		state.channels.extend((2..=count).map(|id| Channel {
			id: Id(id),
			..template.clone()
		}));
		state
			.apply_read_state(Event::Snapshot {
				entries: Some((1..=count).map(|id| (Id(id), Some(Id(100)), 0)).collect()),
				version: None,
				partial: false,
			})
			.unwrap();
		state
	}
	#[test]
	fn mark_all_read_keeps_ack_positions_when_a_pending_channel_is_forgotten() {
		let mut state = with_unread_dms(3);
		let Some(Command::MarkAllRead { request, channels }) = state.prepare_mark_all_read() else {
			panic!("mark all read");
		};
		assert_eq!(channels.len(), 3);
		state.read_state.forget(Id(1));
		state
			.apply_read_state(Event::AllAck {
				request,
				acked: 2,
				result: Err(Failure::RateLimited),
			})
			.unwrap();
		assert_eq!(state.unread(Id(2)), Some(false));
		assert_eq!(
			state.unread(Id(3)),
			Some(true),
			"the failed channel stays unread"
		);
	}
	#[test]
	fn mark_all_read_continuation_waits_for_another_pending_read() {
		let count = crate::MAX_BULK_ACK as u64 + 2;
		let mut state = with_unread_dms(count);
		let Some(Command::MarkAllRead { request, channels }) = state.prepare_mark_all_read() else {
			panic!("first batch");
		};
		assert_eq!(channels.len(), crate::MAX_BULK_ACK);
		state
			.apply_read_state(Event::AllAck {
				request,
				acked: channels.len(),
				result: Ok(()),
			})
			.unwrap();
		let Some(Command::MarkRead {
			channel,
			message,
			request,
			..
		}) = state.prepare_mark_channel_read(Id(count))
		else {
			panic!("channel read");
		};
		assert!(state.continue_mark_all_read().is_none());
		state
			.apply_read_state(Event::Result {
				channel,
				message,
				request,
				result: Ok(()),
			})
			.unwrap();
		let Some(Command::MarkAllRead { channels, .. }) = state.continue_mark_all_read() else {
			panic!("remaining batch");
		};
		assert_eq!(channels, vec![(Id(count - 1), Id(500))]);
	}
	#[test]
	fn mark_all_read_resumes_after_a_disconnect() {
		let mut state = with_unread_dms(2);
		assert!(state.prepare_mark_all_read().is_some());
		apply(&mut state, CoreEvent::Disconnected);
		assert!(state.continue_mark_all_read().is_none());
		apply(&mut state, CoreEvent::Resumed);
		let Some(Command::MarkAllRead { channels, .. }) = state.continue_mark_all_read() else {
			panic!("interrupted batch retries");
		};
		assert_eq!(channels.len(), 2);
		assert!(state.continue_mark_all_read().is_none());
	}
	fn apply(state: &mut State, event: CoreEvent) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
	#[test]
	fn read_failure_is_scoped_and_service_ack_resolves_it_in_either_order() {
		{
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
					mention_count: None,
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
		{
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
				// Opened at the first message, the window keeps the start through newer pages.
				assert_eq!(state.older_exhausted, marker.is_none());
				assert_eq!(state.can_load_older(), marker.is_some());
				assert_eq!(state.read_marker(Id(1)), Some(marker));
				let Command::History { before, after, .. } = state.history(None) else {
					panic!()
				};
				assert!(before.is_none() && after.is_none() && !state.history_targeted);
			}
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
	/// Verify SendResult reconciles own read state without briefly showing the DM channel as unread.
	#[test]
	fn send_result_reconciles_own_read_state_without_flashing_unread_directs() {
		let mut state = state(Some(Id(500)));
		assert_eq!(state.unread_directs(None), Vec::<Id>::new());
		assert_eq!(state.channel_unread(&state.channels[0]), Some(false));

		let Command::Send { nonce, .. } = state.prepare_send().unwrap() else {
			panic!("expected send command");
		};
		let mut sent = message(501);
		sent.author = state.user.clone().unwrap();
		sent.channel = Id(1);
		sent.nonce = Some(nonce.clone());

		apply(
			&mut state,
			CoreEvent::SendResult {
				nonce,
				result: Ok(sent.clone()),
			},
		);

		assert_eq!(
			state.channel_unread(&state.channels[0]),
			Some(false),
			"own sent message in SendResult must not mark channel unread"
		);
		assert_eq!(
			state.unread_directs(None),
			Vec::<Id>::new(),
			"own sent message in SendResult must not place DM on unread direct rail"
		);

		apply(&mut state, CoreEvent::Message(sent));
		assert_eq!(state.channel_unread(&state.channels[0]), Some(false));
		assert_eq!(state.unread_directs(None), Vec::<Id>::new());

		let mut incoming = message(502);
		incoming.author = model::User {
			primary_guild: None,
			cosmetics: None,
			id: Id(999),
			name: "Recipient".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		incoming.channel = Id(1);
		apply(&mut state, CoreEvent::Message(incoming));
		assert_eq!(state.channel_unread(&state.channels[0]), Some(true));
		assert_eq!(state.unread_directs(None), vec![Id(1)]);
	}

	#[test]
	fn historical_sends_confirm_without_splicing_a_live_tail_in_either_order() {
		{
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
		{
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
			page(&mut state, vec![]);
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
	#[test]
	fn deleted_pages_and_unknown_latest_keep_a_forward_cursor_without_resurrecting_messages() {
		{
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
		{
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
	}
}
