use crate::{Command, State, auth::AuthState};
use model::{Freshness, Id, Message};
use session_cache::Timeline;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
	target: Id,
	pub mention: bool,
}

impl Reply {
	pub fn to(target: Id) -> Self {
		Self {
			target,
			mention: true,
		}
	}

	pub fn target(self) -> Id {
		self.target
	}
}

/// Accepted known-deleted reply targets from the last applied event (at most 50).
#[derive(Default)]
pub struct ReplyDeletions(pub(crate) Vec<(Id, Id)>);

/// Unsent reply targets of conversations the user left, by session generation and
/// channel, oldest first; session RAM only.
#[derive(Default)]
pub struct ParkedReplies(Vec<(u64, Id, Reply)>);

impl ParkedReplies {
	const MAX: usize = 64;

	fn park(&mut self, generation: u64, channel: Id, reply: Reply) {
		self.0.retain(|(_, id, _)| *id != channel);
		if self.0.len() >= Self::MAX {
			self.0.remove(0);
		}
		self.0.push((generation, channel, reply));
	}

	fn take(&mut self, generation: u64, channel: Id) -> Option<Reply> {
		let index = self.0.iter().position(|(_, id, _)| *id == channel)?;
		let (parked, _, reply) = self.0.remove(index);
		(parked == generation).then_some(reply)
	}

	/// Drops parked replies whose target in `channel` was deleted.
	pub(crate) fn forget_targets(&mut self, channel: Id, targets: &[Id]) {
		self.0
			.retain(|(_, id, reply)| *id != channel || !targets.contains(&reply.target));
	}

	pub(crate) fn forget_channels(&mut self, removed: &std::collections::BTreeSet<Id>) {
		self.0.retain(|(_, id, _)| !removed.contains(id));
	}

	#[cfg(test)]
	pub(crate) fn len(&self) -> usize {
		self.0.len()
	}
}

/// One bounded, request-scoped target, independent of the UI's one-shot scroll cue.
#[derive(Default)]
pub struct HistoryNavigation {
	target: Option<(Id, u64, Id)>,
	previous: Option<NavigationMetadata>,
	back: Option<ReturnWindow>,
	provisional: bool,
}

#[derive(Clone, Copy)]
struct NavigationMetadata {
	before: Option<Id>,
	after: Option<Id>,
	targeted: bool,
	newer_cursor: Option<Id>,
	newer_more: bool,
	older_exhausted: bool,
	freshness: Freshness,
}
struct ReturnWindow {
	channel: Id,
	timeline: Option<Timeline>,
	metadata: NavigationMetadata,
	cursor: Option<crate::ReadingCursor>,
	stored: std::time::Instant,
}

impl HistoryNavigation {
	pub(crate) fn clear(&mut self) {
		self.target = None;
		self.previous = None;
		self.provisional = false;
	}

	fn start(&mut self, channel: Id, request: u64, target: Id) {
		self.target = Some((channel, request, target));
	}

	pub(crate) fn target(&self, channel: Id, request: u64) -> Option<Id> {
		self.target
			.filter(|(id, pending, _)| *id == channel && *pending == request)
			.map(|(_, _, target)| target)
	}
	pub(crate) fn bytes(&self) -> usize {
		self.back
			.as_ref()
			.and_then(|back| back.timeline.as_ref())
			.map_or(0, Timeline::retained_bytes)
	}
	pub(crate) fn back_original(&self, channel: Id, id: Id) -> Option<&Message> {
		let back = self.back.as_ref()?;
		if back.channel != channel
			|| back.metadata.freshness != Freshness::Fresh
			|| back.stored.elapsed() > std::time::Duration::from_secs(5 * 60)
		{
			return None;
		}
		back.timeline
			.as_ref()?
			.get(id)
			.filter(|message| message.channel == channel)
	}
	pub(crate) fn rows(&self) -> usize {
		self.back
			.as_ref()
			.and_then(|back| back.timeline.as_ref())
			.map_or(0, Timeline::row_count)
	}
	pub(crate) fn evict_back(&mut self) -> bool {
		self.back
			.as_mut()
			.and_then(|back| back.timeline.take())
			.is_some()
	}
	pub(crate) fn invalidate_back(&mut self, channel: Id) {
		if self
			.back
			.as_ref()
			.is_some_and(|back| back.channel == channel)
		{
			self.evict_back();
		}
	}
	pub(crate) fn clear_back(&mut self) {
		self.back = None;
	}
	pub(crate) fn provisional(&self) -> bool {
		self.provisional
	}
	/// A nested jump keeps its existing Back window until the service confirms the new target.
	pub(crate) fn can_stage_cached_target(&self) -> bool {
		!self.provisional && self.back.is_none()
	}
}

impl State {
	fn navigation_metadata(&self) -> NavigationMetadata {
		NavigationMetadata {
			before: self.history_before,
			after: self.history_after,
			targeted: self.history_targeted,
			newer_cursor: self.newer_cursor,
			newer_more: self.newer_may_have_more,
			older_exhausted: self.older_exhausted,
			freshness: if self.history_pending && self.freshness == Freshness::Loading {
				if self.gateway_connected
					&& !self.timeline.is_empty()
					&& (self.history_before.is_some() || self.history_after.is_some())
				{
					Freshness::Fresh
				} else {
					Freshness::Stale
				}
			} else {
				self.freshness
			},
		}
	}
	fn restore_navigation_metadata(&mut self, metadata: NavigationMetadata) {
		self.history_before = metadata.before;
		self.history_after = metadata.after;
		self.history_targeted = metadata.targeted;
		self.newer_cursor = metadata.newer_cursor;
		self.newer_may_have_more = metadata.newer_more;
		self.older_exhausted = metadata.older_exhausted;
		self.freshness = metadata.freshness;
	}
	pub fn can_return_from_reply(&self) -> bool {
		self.history_navigation.back.as_ref().is_some_and(|back| {
			self.selected == Some(back.channel) && self.can_read_history(back.channel)
		})
	}
	pub fn return_from_reply(&mut self) -> Option<Command> {
		if !self.can_return_from_reply() {
			return None;
		}
		let mut back = self.history_navigation.back.take()?;
		if back.stored.elapsed() > std::time::Duration::from_secs(5 * 60)
			|| back.metadata.freshness != Freshness::Fresh
		{
			back.timeline = None;
		}
		self.cancel_history();
		if let Some(cursor) = back.cursor {
			self.remember_reading(back.channel, cursor);
		}
		if let Some(mut timeline) = back.timeline {
			timeline.cancel_page();
			self.timeline = timeline;
			self.restore_navigation_metadata(back.metadata);
			self.search_target = back.cursor.and_then(|cursor| cursor.message);
			self.restore_scroll = true;
			self.revision += 1;
			return None;
		}
		match back.cursor.and_then(|cursor| cursor.message) {
			Some(message) if self.timeline.is_deleted(message) || message.0 <= 1 => {
				self.timeline.clear_window_preserving_deletions();
				let command = self.history_range(None, Some(Id(message.0.saturating_sub(1))));
				self.history_targeted = true;
				self.restore_scroll = true;
				Some(command)
			}
			Some(message) => self.open_scrolled_window(message),
			None => {
				let command = self.history(None);
				self.restore_scroll = true;
				Some(command)
			}
		}
	}
	/// Restore the previous window while retaining the in-flight request and its mutation fences.
	pub(crate) fn rollback_provisional_history(&mut self) -> bool {
		if !self.history_navigation.provisional {
			return false;
		}
		let Some(mut back) = self.history_navigation.back.take() else {
			return false;
		};
		let Some(timeline) = back.timeline.take() else {
			self.history_navigation.back = Some(back);
			return false;
		};
		self.timeline = timeline;
		self.history_navigation.provisional = false;
		self.history_targeted = back.metadata.targeted;
		self.search_target = back.cursor.and_then(|cursor| cursor.message);
		self.restore_scroll = self.search_target.is_some();
		self.revision = self.revision.wrapping_add(1);
		true
	}

	/// A failed around request restores the old window and its reading metadata.
	pub(crate) fn fail_history_navigation(&mut self, failure: crate::auth::Failure) -> bool {
		if matches!(
			failure,
			crate::auth::Failure::Expired
				| crate::auth::Failure::Challenged
				| crate::auth::Failure::InvalidCredential
		) {
			return false;
		}
		self.rollback_provisional_history();
		let Some(previous) = self.history_navigation.previous.take() else {
			return false;
		};
		self.history_navigation.clear();
		self.history_pending = false;
		self.request = self.request.wrapping_add(1);
		self.timeline.cancel_page();
		self.restore_navigation_metadata(previous);
		self.status = match failure {
			crate::auth::Failure::Forbidden => {
				"Original message is unavailable with the current permissions; your reading position was kept"
			}
			_ => {
				"Could not load the original message; your reading position was kept. Try View original again"
			}
		};
		self.revision += 1;
		true
	}
	pub(crate) fn cancel_reply_navigation(&mut self) {
		self.rollback_provisional_history();
		if let Some(previous) = self.history_navigation.previous.take() {
			self.history_navigation.clear();
			self.history_pending = false;
			self.request = self.request.wrapping_add(1);
			self.timeline.cancel_page();
			self.restore_navigation_metadata(previous);
		}
	}
	pub(crate) fn prepare_target_history(
		&mut self,
		channel: Id,
		request: u64,
		messages: &[Message],
		provisional: bool,
	) -> Result<(), &'static str> {
		let Some(target) = self.history_navigation.target(channel, request) else {
			return Ok(());
		};
		if !messages
			.iter()
			.any(|message| message.id == target && message.channel == channel)
			|| self.timeline.is_deleted(target)
		{
			if self.fail_history_navigation(crate::auth::Failure::Protocol) {
				self.status = "Original message was not returned; your reading position was kept";
			}
			return Err("Original message was not returned");
		}
		let ids: Vec<_> = messages.iter().map(|message| message.id).collect();
		if self.history_navigation.provisional {
			let result = self
				.history_navigation
				.back
				.as_ref()
				.and_then(|back| back.timeline.as_ref())
				.ok_or("Previous history window is unavailable")
				.and_then(|previous| previous.history_range(&ids));
			let fresh = match result {
				Ok(fresh) => fresh,
				Err(error) => {
					self.fail_history_navigation(crate::auth::Failure::Protocol);
					return Err(error);
				}
			};
			if let Some(previous) = self
				.history_navigation
				.back
				.as_mut()
				.and_then(|back| back.timeline.as_mut())
			{
				previous.cancel_page();
			}
			self.timeline = fresh;
			self.history_navigation.previous = None;
			self.history_navigation.provisional = false;
			self.search_target = Some(target);
			self.restore_scroll = false;
			return Ok(());
		}
		if self.history_navigation.previous.is_none() {
			return Ok(());
		}
		let fresh = match self.timeline.history_range(&ids) {
			Ok(fresh) => fresh,
			Err(error) => {
				self.fail_history_navigation(crate::auth::Failure::Protocol);
				return Err(error);
			}
		};
		let metadata = self
			.history_navigation
			.previous
			.take()
			.expect("checked navigation metadata");
		let mut previous = std::mem::replace(&mut self.timeline, fresh);
		if !provisional {
			previous.cancel_page();
		}
		self.history_navigation.back = Some(ReturnWindow {
			channel,
			timeline: Some(previous),
			metadata,
			cursor: self.reading(channel),
			stored: std::time::Instant::now(),
		});
		self.history_targeted = true;
		self.search_target = Some(target);
		self.restore_scroll = false;
		if provisional {
			self.history_navigation.previous = Some(metadata);
			self.history_navigation.provisional = true;
		}
		Ok(())
	}
	pub fn reply_target(&self) -> Option<Id> {
		self.reply.map(Reply::target)
	}

	/// Keeps the open conversation's unsent reply target for its next visit.
	pub(crate) fn park_reply(&mut self) {
		if let (Some(channel), Some(reply)) = (self.selected, self.reply.take()) {
			self.parked_replies.park(self.generation, channel, reply);
		}
	}

	/// Takes `channel`'s parked reply before parking the open conversation's, so parking at
	/// capacity cannot evict the destination. Reopening the same conversation keeps its reply.
	pub(crate) fn swap_parked_reply(&mut self, channel: Id) -> Option<Reply> {
		if self.selected == Some(channel) {
			return self.reply.take();
		}
		let destination = self.parked_replies.take(self.generation, channel);
		self.park_reply();
		destination.or_else(|| self.parked_replies.take(self.generation, channel))
	}

	/// Restores `channel`'s parked reply after selection, only while it can still be sent
	/// there and its target is not known to be deleted.
	pub(crate) fn restore_reply(&mut self, channel: Id, parked: Option<Reply>) {
		self.reply = parked.filter(|reply| {
			self.selected == Some(channel)
				&& self.can_compose(channel)
				&& !self.timeline.is_deleted(reply.target)
				&& self
					.timeline
					.get(reply.target)
					.is_none_or(|message| message.channel == channel)
		});
	}

	/// Navigate a Discord chat link without sending messages or joining voice.
	///
	/// Only fresh active rows or eligible, previously fresh session-resident rows
	/// can jump locally. Disconnect makes the active window stale, so it must be
	/// revalidated online; a dormant resident may still be restored offline.
	/// Destinations need metadata in navigation or the current safe archive page.
	pub fn open_chat_link(
		&mut self,
		guild: Option<Id>,
		channel: Id,
		message: Option<Id>,
	) -> Result<Option<Command>, &'static str> {
		if channel.0 == 0 || guild.is_some_and(|id| id.0 == 0) {
			return Err("This chat link is invalid");
		}
		if message.is_some_and(|id| id.0 == 0 || id.0.checked_add(1).is_none()) {
			return Err("This message link is invalid");
		}
		if self.channel(channel).is_none() {
			if message.is_some()
				&& (self.auth != AuthState::Authenticated || !self.gateway_connected)
			{
				// Admission can retire another transient thread. A message link
				// needing metadata/history must pass session gates before that mutation.
				return Err("Message links are unavailable while disconnected");
			}
			// Only the existing bounded, permission-checked archive page may resolve
			// metadata absent from navigation. Never guess a parent or open a DM.
			let view = self.archives.as_ref().ok_or("This chat is unavailable")?;
			let thread = view
				.page
				.as_ref()
				.and_then(|page| page.threads.iter().find(|thread| thread.id == channel))
				.ok_or("This chat is unavailable")?;
			if thread.guild != guild {
				return Err("This chat does not belong to the linked server");
			}
			if !self
				.channel(view.parent)
				.is_some_and(|parent| parent.guild == Some(view.guild))
			{
				return Err("This chat is unavailable");
			}
			self.admit_archived_thread(channel)
				.ok_or("This chat is unavailable")?;
		}
		let target = self.channel(channel).ok_or("This chat is unavailable")?;
		if target.guild != guild {
			return Err("This chat does not belong to the linked server");
		}
		if !crate::navigable(target) {
			return Err("This channel kind is unsupported");
		}
		if !self.can_view(channel) {
			return Err("Channel permissions are unavailable or access was revoked");
		}
		let Some(message) = message else {
			return Ok(self.select(channel));
		};
		if !target.supports_text() || !self.can_read_history(channel) {
			return Err("Message history is unavailable with the current permissions");
		}
		if self.auth != AuthState::Authenticated {
			return Err("Message links are unavailable while disconnected");
		}
		if self.selected == Some(channel) {
			if self.timeline.is_deleted(message) {
				return Err("This message was deleted");
			}
			if self.history_pending
				&& self.history_navigation.target(channel, self.request) == Some(message)
			{
				// A repeat click shares the in-flight page instead of invalidating it.
				return Ok(None);
			}
			if self.freshness == Freshness::Fresh
				&& !self.history_pending
				&& self
					.timeline
					.get(message)
					.is_some_and(|m| m.channel == channel)
			{
				self.search_target = Some(message);
				self.restore_scroll = false;
				self.history_navigation.clear();
				self.revision += 1;
				return Ok(None);
			}
		}
		if self.selected != Some(channel)
			&& self
				.resident_timeline(channel)
				.is_some_and(|timeline| timeline.is_deleted(message))
		{
			self.status = "This message was deleted";
			return Err(self.status);
		}
		if !self.gateway_connected
			&& (self.selected == Some(channel)
				|| !self.resident_timeline(channel).is_some_and(|timeline| {
					timeline.get(message).is_some_and(|m| m.channel == channel)
				})) {
			// Check the exact dormant destination before touching selection,
			// drafts, the navigation trail, or another conversation's timeline.
			return Err("Message links are unavailable while disconnected");
		}
		// Restore the resident window without first starting a recent-history or
		// saved-reading-cursor request. Selection alone never sends or joins voice.
		self.select_chat_link_channel(channel, message)?;
		if self.timeline.is_deleted(message) {
			self.status = "This message was deleted";
			// A window that could not be restored as loaded still needs its recent page.
			if self.freshness != Freshness::Fresh && self.gateway_connected {
				return Ok(Some(self.history(None)));
			}
			return Err("This message was deleted");
		}
		if self.freshness == Freshness::Fresh
			&& !self.history_pending
			&& self
				.timeline
				.get(message)
				.is_some_and(|m| m.channel == channel)
		{
			self.search_target = Some(message);
			self.restore_scroll = false;
			self.revision += 1;
			return Ok(None);
		}
		if !self.gateway_connected {
			self.status = "Message links are unavailable while disconnected";
			return Err(self.status);
		}
		Ok(self.open_target_window(message))
	}

	pub(crate) fn deleted_navigation_target(&mut self, channel: Id, target: Id) {
		if self.selected == Some(channel)
			&& (self.search_target == Some(target)
				|| self.history_navigation.target(channel, self.request) == Some(target))
		{
			self.search_target = None;
			self.restore_scroll = false;
			self.status = "This message was deleted";
		}
	}

	pub(crate) fn finish_history_navigation(&mut self, channel: Id, request: u64) {
		let Some(target) = self.history_navigation.target(channel, request) else {
			return;
		};
		self.history_navigation.clear();
		if self.timeline.is_deleted(target) {
			self.search_target = None;
			self.restore_scroll = false;
			self.status = "This message was deleted";
		} else if !self
			.timeline
			.get(target)
			.is_some_and(|m| m.channel == channel)
		{
			// Retain the cue for the timeline to mark this detached window as
			// browsing, but report the outcome even without a UI consuming it.
			self.status =
				"Message was not returned; it may have been removed or become unavailable";
		}
	}

	/// All effects belong to one channel, with at most 50 distinct targets.
	pub fn take_reply_deletions(&mut self) -> Vec<(Id, Id)> {
		std::mem::take(&mut self.reply_deletions.0)
	}
	pub(crate) fn accepts_reply_source(&self, message: &Message) -> bool {
		message.id.0 != 0
			&& message.channel.0 != 0
			&& Timeline::valid_message(message)
			&& self.can_view(message.channel)
			&& self
				.channels
				.iter()
				.any(|channel| channel.id == message.channel && channel.supports_text())
			&& (self.selected != Some(message.channel) || self.freshness != Freshness::Unavailable)
	}
	pub(crate) fn record_reply_deletion(&mut self, message: &Message) {
		if !message.reply_deleted {
			return;
		}
		let Some(target) = message
			.reply_to
			.filter(|target| target.0 != 0 && *target < message.id)
		else {
			return;
		};
		let deletion = (message.channel, target);
		self.search_message_changed(&crate::Event::Delete {
			channel: message.channel,
			id: target,
		});
		self.forget_pending_replies(message.channel, &[target]);
		// Only one message or an already validated <=50-message history page calls this.
		if !self.reply_deletions.0.contains(&deletion) && self.reply_deletions.0.len() < 50 {
			self.reply_deletions.0.push(deletion);
		}
		self.resident.remove(message.channel);
		self.parked_replies
			.forget_targets(message.channel, &[target]);
		self.read_state.activity.delete(message.channel, target);
		if let Some(channel) = self
			.channels
			.iter_mut()
			.find(|channel| channel.id == message.channel && channel.last_message == Some(target))
		{
			self.read_state.activity.observe_latest(channel.id, target);
			channel.last_message = None;
		}
		if self.selected == Some(message.channel) {
			self.deleted_navigation_target(message.channel, target);
			if self.reply_target() == Some(target) {
				self.reply = None;
			}
		}
	}
	pub fn can_open_reply_target(&self, target: Id) -> bool {
		let Some(channel) = self.selected else {
			return false;
		};
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& (self.freshness == Freshness::Fresh
				|| (self.freshness == Freshness::Loading
					&& self.history_pending
					&& (self.history_before.is_some() || self.history_after.is_some())))
			&& target.0 != 0
			&& target.0.checked_add(1).is_some()
			&& !self.timeline.is_deleted(target)
			&& !self.timeline.iter().any(|message| {
				message.channel == channel
					&& message.reply_to == Some(target)
					&& message.reply_deleted
			}) && self.can_read_history(channel)
			&& self
				.channels
				.iter()
				.any(|entry| entry.id == channel && entry.supports_text())
			&& self
				.timeline
				.get(target)
				.is_none_or(|message| message.channel == channel)
			&& (self.reply_target() == Some(target)
				|| self
					.timeline
					.iter()
					.any(|message| message.channel == channel && message.reply_to == Some(target)))
	}

	/// Loaded targets only request a local scroll. Unloaded targets use one bounded
	/// history page; None can therefore mean either a local jump or a rejected action.
	pub fn open_reply_target(&mut self, target: Id) -> Option<Command> {
		if !self.can_open_reply_target(target) {
			return None;
		}
		if self.selected.is_some_and(|channel| {
			self.history_pending
				&& self.history_navigation.target(channel, self.request) == Some(target)
		}) {
			return None;
		}
		if self.timeline.get(target).is_some() {
			if self.history_pending {
				self.cancel_history();
				self.freshness = Freshness::Fresh;
			}
			self.search_target = Some(target);
			self.revision += 1;
			return None;
		}
		self.open_target_window(target)
	}

	pub(crate) fn open_target_window(&mut self, target: Id) -> Option<Command> {
		if target.0 == 0 || self.timeline.is_deleted(target) {
			return None;
		}
		target.0.checked_add(1)?;
		let channel = self.selected?;
		if self.history_pending
			&& self.history_navigation.target(channel, self.request) == Some(target)
		{
			return None;
		}
		self.rollback_provisional_history();
		let previous = self
			.history_navigation
			.previous
			.unwrap_or_else(|| self.navigation_metadata());
		self.timeline.begin_append();
		self.request = self.request.wrapping_add(1);
		self.history_before = None;
		self.history_after = None;
		self.history_pending = true;
		self.search_target = None;
		self.restore_scroll = false;
		self.history_navigation.start(channel, self.request, target);
		self.history_navigation.previous = Some(previous);
		// Desktop cancels its reaction read when it starts either kind of history request.
		self.reactions.cancel_read();
		self.reply_previews.cancel_pending();
		self.revision += 1;
		Some(Command::HistoryAround {
			channel,
			target,
			request: self.request,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event, Pending, auth::Failure};
	use model::{Channel, Delivery, Message, User};

	fn message(id: u64) -> Message {
		Message {
			reply_preview: None,
			poll: None,
			sticker_items: Vec::new(),
			id: Id(id),
			channel: Id(1),
			author: User {
				member_avatar: None,
				username: None,
				primary_guild: None,
				cosmetics: None,
				id: Id(9),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			content: "Synthetic reply".into(),
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
	fn state() -> State {
		let mut state = State {
			user: Some(message(1).author),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			selected: Some(Id(1)),
			freshness: Freshness::Fresh,
			channels: (1..=3)
				.map(|id| Channel {
					id: Id(id),
					guild: None,
					parent_id: None,
					kind: 1,
					position: 0,
					name: "Synthetic DM".into(),
					recipients: vec![],
					last_message: None,
					icon: None,
					member_list_id: None,
					tags: None,
					message_count: None,
				})
				.collect(),
			..State::default()
		};
		let mut source = message(100);
		source.reply_to = Some(Id(50));
		state.timeline.insert(source, false, false).unwrap();
		state
	}
	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}

	#[test]
	fn preview_cancellation_is_emitted_only_for_actual_work_and_clear_keeps_request_fences() {
		use crate::reply_previews::ResultEvent;
		let mut state = state();
		assert!(state.sync_visible_reply_previews(&[]).is_empty());
		let request = state
			.sync_visible_reply_previews(&[Id(50)])
			.into_iter()
			.find_map(|command| match command {
				Command::ReplyPreview { request, .. } => Some(request),
				_ => None,
			})
			.unwrap();
		state.clear_cached_history();
		let commands = state.sync_visible_reply_previews(&[Id(50)]);
		assert!(matches!(
			commands.first(),
			Some(Command::CancelReplyPreviews)
		));
		let next = commands
			.iter()
			.find_map(|command| match command {
				Command::ReplyPreview { request, .. } => Some(*request),
				_ => None,
			})
			.unwrap();
		assert_ne!(
			request, next,
			"cleared reads must not reuse a request ID in this session"
		);
		let preview = Box::new(model::ReplyPreview::from_message(message(50)).unwrap());
		apply(
			&mut state,
			Event::ReplyPreview(ResultEvent {
				channel: Id(1),
				message: Id(50),
				request,
				result: Ok(Some(preview)),
			}),
		);
		assert!(state.reply_original(Id(50)).is_none());
		assert_eq!(
			state.reply_preview_status(Id(50)),
			crate::reply_previews::Status::Loading
		);
		assert!(matches!(
			state.sync_visible_reply_previews(&[]).as_slice(),
			[Command::CancelReplyPreviews]
		));
		assert!(state.sync_visible_reply_previews(&[]).is_empty());
	}

	#[test]
	fn new_embedded_previews_retire_older_inflight_bodies() {
		use crate::reply_previews::ResultEvent;
		let mut state = state();
		let request = state
			.sync_visible_reply_previews(&[Id(50)])
			.into_iter()
			.find_map(|command| match command {
				Command::ReplyPreview { request, .. } => Some(request),
				_ => None,
			})
			.unwrap();
		let mut original = message(50);
		original.content = "Newer embedded body".into();
		let mut source = message(101);
		source.reply_to = Some(Id(50));
		source.reply_preview = Some(Box::new(
			model::ReplyPreview::from_message(original).unwrap(),
		));
		apply(&mut state, Event::Message(source));
		apply(
			&mut state,
			Event::ReplyPreview(ResultEvent {
				channel: Id(1),
				message: Id(50),
				request,
				result: Ok(Some(Box::new(
					model::ReplyPreview::from_message(message(50)).unwrap(),
				))),
			}),
		);
		assert_eq!(
			state.reply_original(Id(50)).unwrap().content,
			"Newer embedded body"
		);
		assert!(matches!(
			state.sync_visible_reply_previews(&[Id(50)]).as_slice(),
			[Command::CancelReplyPreviews]
		));
	}

	#[test]
	fn cached_top_replies_do_not_starve_lower_visible_originals() {
		let mut state = state();
		state.timeline.clear();
		for target in 1..=80 {
			let mut source = message(1000 + target);
			source.kind = 19;
			source.reply_to = Some(Id(target));
			if target <= 40 {
				source.reply_preview = Some(Box::new(
					model::ReplyPreview::from_message(message(target)).unwrap(),
				));
			}
			apply(&mut state, Event::Message(source));
		}
		let visible: Vec<_> = (1..=80).map(Id).collect();
		let commands = state.sync_visible_reply_previews(&visible);
		let targets: Vec<_> = commands
			.into_iter()
			.filter_map(|command| match command {
				Command::ReplyPreview { message, .. } => Some(message),
				_ => None,
			})
			.collect();
		assert_eq!(targets, vec![Id(41), Id(42)]);
	}

	#[test]
	fn fresh_return_window_supplies_display_only_originals_without_a_read() {
		let mut state = state();
		state.timeline.insert(message(50), false, false).unwrap();
		let mut source = message(100);
		source.reply_to = Some(Id(20));
		state.timeline.insert(source, false, false).unwrap();
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(20)) else {
			panic!()
		};
		let mut reply = message(60);
		reply.reply_to = Some(Id(50));
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(20), reply],
			},
		);
		assert!(state.timeline.get(Id(50)).is_none());
		assert_eq!(
			state.reply_original(Id(50)).unwrap().content,
			"Synthetic reply"
		);
		assert!(state.sync_visible_reply_previews(&[Id(50)]).is_empty());
		state.history_navigation.back.as_mut().unwrap().stored =
			std::time::Instant::now() - std::time::Duration::from_secs(301);
		assert!(state.reply_original(Id(50)).is_none());
		assert!(
			state
				.sync_visible_reply_previews(&[Id(50)])
				.iter()
				.any(|command| matches!(
					command,
					Command::ReplyPreview {
						message: Id(50),
						..
					}
				))
		);
	}

	#[test]
	fn stale_latest_page_hydration_is_not_trusted_as_a_fresh_return_window() {
		let mut state = state();
		state.remember_reading(
			Id(1),
			crate::ReadingCursor {
				message: Some(Id(100)),
				inset: 4.0,
			},
		);
		state.freshness = Freshness::Stale;
		state.history(None);
		assert_eq!(state.freshness, Freshness::Loading);
		assert!(
			!state.can_open_reply_target(Id(50)),
			"initial cached history awaits validation"
		);
		let Some(Command::HistoryAround { request, .. }) = state.open_target_window(Id(50)) else {
			panic!()
		};
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(50)],
			},
		);
		assert!(
			state.reply_original(Id(100)).is_none(),
			"stale cached return payload cannot supply previews"
		);
		assert!(
			matches!(
				state.return_from_reply(),
				Some(Command::History {
					after: Some(Id(99)),
					..
				})
			),
			"Back revalidates the stale window"
		);
		assert!(state.timeline.get(Id(100)).is_none());
	}

	#[test]
	fn cached_reply_jump_restores_previous_window_on_failure_or_missing_target() {
		for missing in [false, true] {
			let mut state = state();
			state.reply = Some(Reply::to(Id(50)));
			state.remember_reading(
				Id(1),
				crate::ReadingCursor {
					message: Some(Id(100)),
					inset: 8.0,
				},
			);
			let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50))
			else {
				panic!("target request")
			};
			assert!(state.hydrate_cached_history_page(Id(1), request, vec![message(50)]));
			assert!(state.timeline.get(Id(50)).is_some());
			assert!(state.timeline.get(Id(100)).is_none());
			let event = if missing {
				Event::History {
					channel: Id(1),
					request,
					older: false,
					messages: vec![message(49)],
				}
			} else {
				Event::HistoryFailed {
					channel: Id(1),
					request,
					failure: Failure::Network,
				}
			};
			apply(&mut state, event);
			assert!(!state.history_pending);
			assert_eq!(state.freshness, Freshness::Fresh);
			assert!(
				state.timeline.get(Id(100)).is_some(),
				"the original window must be restored"
			);
			assert!(
				state.timeline.get(Id(50)).is_none(),
				"the unvalidated cache must be retired"
			);
			assert_eq!(state.reading(Id(1)).unwrap().message, Some(Id(100)));
		}
	}

	#[test]
	fn cached_reply_jump_preserves_live_changes_in_the_return_window() {
		let mut state = state();
		state.reply = Some(Reply::to(Id(50)));
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("target request")
		};
		assert!(state.hydrate_cached_history_page(Id(1), request, vec![message(50)]));
		apply(&mut state, Event::Message(message(101)));
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(50)],
			},
		);
		assert!(state.timeline.get(Id(50)).is_some());
		assert!(state.can_return_from_reply());
		assert!(state.return_from_reply().is_none());
		assert!(state.timeline.get(Id(100)).is_some());
		assert!(
			state.timeline.get(Id(101)).is_some(),
			"a live message must survive Back"
		);
	}

	#[test]
	fn cached_reply_jump_survives_unrelated_channel_activity() {
		let mut state = state();
		state.reply = Some(Reply::to(Id(50)));
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("target request")
		};
		assert!(state.hydrate_cached_history_page(Id(1), request, vec![message(50)]));
		let mut other = message(101);
		other.channel = Id(2);
		apply(&mut state, Event::Message(other));
		assert!(state.timeline.get(Id(50)).is_some());
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request,
				failure: Failure::Network,
			},
		);
		assert!(state.timeline.get(Id(100)).is_some());
	}

	#[test]
	fn nested_cached_reply_jump_keeps_the_existing_back_window_on_failure() {
		let mut state = state();
		state.reply = Some(Reply::to(Id(50)));
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("first request")
		};
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(50)],
			},
		);
		state.reply = Some(Reply::to(Id(25)));
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(25)) else {
			panic!("second request")
		};
		assert!(!state.hydrate_cached_history_page(Id(1), request, vec![message(25)]));
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request,
				failure: Failure::Network,
			},
		);
		assert!(state.timeline.get(Id(50)).is_some());
		assert!(state.return_from_reply().is_none());
		assert!(state.timeline.get(Id(100)).is_some());
	}

	#[test]
	fn cached_reply_jump_keeps_prior_live_patches_and_rejects_duplicate_cache_pages() {
		let mut state = state();
		state.reply = Some(Reply::to(Id(50)));
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("target request")
		};
		apply(
			&mut state,
			Event::Patch(model::MessagePatch {
				id: Id(60),
				channel: Id(1),
				content: model::Patch::Value("Live edit".into()),
				poll: model::Patch::Absent,
				sticker_items: model::Patch::Absent,
				flags: model::Patch::Absent,
				components: model::Patch::Absent,
				application_id: model::Patch::Absent,
				extra_content: Default::default(),
				reactions: model::Patch::Absent,
				mentions: model::Patch::Absent,
				edited: model::Patch::Absent,
				embeds: model::Patch::Absent,
				embeds_suppressed: model::Patch::Absent,
				attachments: model::Patch::Absent,
			}),
		);
		assert!(state.hydrate_cached_history_page(Id(1), request, vec![message(50)]));
		assert!(!state.hydrate_cached_history_page(Id(1), request, vec![message(50)]));
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(50), message(60)],
			},
		);
		assert_eq!(state.timeline.get(Id(60)).unwrap().content, "Live edit");
		assert!(state.return_from_reply().is_none());
		assert!(state.timeline.get(Id(100)).is_some());
	}

	#[test]
	fn returning_before_cached_reply_validation_cancels_the_old_page() {
		let mut state = state();
		state.reply = Some(Reply::to(Id(50)));
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("target request")
		};
		assert!(state.hydrate_cached_history_page(Id(1), request, vec![message(50)]));
		assert!(state.return_from_reply().is_none());
		for id in 1000..1000 + session_cache::MAX_MUTATIONS as u64 + 5 {
			apply(&mut state, Event::Message(message(id)));
		}
		assert_eq!(state.freshness, Freshness::Fresh);
		assert_eq!(state.auth, AuthState::Authenticated);
		assert!(
			state
				.timeline
				.get(Id(1000 + session_cache::MAX_MUTATIONS as u64 + 4))
				.is_some()
		);
	}

	#[test]
	fn failed_reply_navigation_stops_page_reconciliation_before_live_message_churn() {
		let mut state = state();
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!()
		};
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request,
				failure: crate::auth::Failure::Network,
			},
		);
		for id in 1000..1000 + session_cache::MAX_MUTATIONS as u64 + 5 {
			apply(&mut state, Event::Message(message(id)));
		}
		assert_eq!(state.freshness, Freshness::Fresh);
		assert_eq!(state.auth, AuthState::Authenticated);
		assert!(
			state
				.timeline
				.get(Id(1000 + session_cache::MAX_MUTATIONS as u64 + 4))
				.is_some()
		);
	}

	#[test]
	fn reply_preview_keeps_generic_heic_image_classification() {
		let mut original = message(50);
		original.attachments.push(model::Attachment {
			id: Id(51),
			filename: "photo.HEIC".into(),
			description: None,
			content_type: Some("application/octet-stream".into()),
			size: 100,
			media: model::EmbedMedia {
				url: Some("https://example.invalid/private".into()),
				..Default::default()
			},
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		});
		assert!(original.attachments[0].is_image());
		let preview = model::ReplyPreview::from_message(original).unwrap();
		let attachment = &preview.message().attachments[0];
		assert!(attachment.is_image());
		assert!(attachment.media.url.is_none());
	}

	#[test]
	fn old_reply_previews_deduplicate_reads_and_ignore_cancelled_results() {
		use crate::reply_previews::{ResultEvent, Status};
		let mut state = state();
		let commands = state.sync_visible_reply_previews(&[Id(50), Id(50)]);
		let request = commands
			.iter()
			.find_map(|command| match command {
				Command::ReplyPreview { request, .. } => Some(*request),
				_ => None,
			})
			.unwrap();
		assert_eq!(state.reply_preview_status(Id(50)), Status::Loading);
		assert!(state.sync_visible_reply_previews(&[Id(50)]).is_empty());
		let preview = model::ReplyPreview::from_message(message(50)).unwrap();
		apply(
			&mut state,
			Event::ReplyPreview(ResultEvent {
				channel: Id(1),
				message: Id(50),
				request,
				result: Ok(Some(Box::new(preview))),
			}),
		);
		assert_eq!(
			state.reply_original(Id(50)).unwrap().content,
			"Synthetic reply"
		);
		assert!(
			state.timeline.get(Id(50)).is_none(),
			"preview is never an actionable history row"
		);
		assert_eq!(state.timeline.len(), 1);
		state.sync_visible_reply_previews(&[]);
		apply(
			&mut state,
			Event::Delete {
				channel: Id(1),
				id: Id(50),
			},
		);
		assert!(state.reply_original(Id(50)).is_none());
		assert_eq!(state.reply_preview_status(Id(50)), Status::Deleted);
		let preview = model::ReplyPreview::from_message(message(50)).unwrap();
		apply(
			&mut state,
			Event::ReplyPreview(ResultEvent {
				channel: Id(1),
				message: Id(50),
				request,
				result: Ok(Some(Box::new(preview))),
			}),
		);
		assert!(state.reply_original(Id(50)).is_none());
	}

	#[test]
	fn every_visible_original_is_read_once_even_after_the_128_body_cache_rolls_over() {
		use crate::reply_previews::{MAX_VISIBLE_REPLIES, ResultEvent, Status};
		let mut state = state();
		state.timeline.clear();
		let visible: Vec<_> = (1..=MAX_VISIBLE_REPLIES as u64).map(Id).collect();
		state
			.timeline
			.seed_cache(
				visible
					.iter()
					.map(|id| {
						let mut source = message(1000 + id.0);
						source.reply_to = Some(*id);
						source
					})
					.collect(),
			)
			.unwrap();
		let mut reads = vec![0; MAX_VISIBLE_REPLIES];
		for _ in 0..=MAX_VISIBLE_REPLIES / 2 {
			let commands = state.sync_visible_reply_previews(&visible);
			for command in commands {
				let Command::ReplyPreview {
					channel,
					message: id,
					request,
				} = command
				else {
					panic!("the unchanged viewport must not cancel work")
				};
				reads[id.0 as usize - 1] += 1;
				apply(
					&mut state,
					Event::ReplyPreview(ResultEvent {
						channel,
						message: id,
						request,
						result: Ok(Some(Box::new(
							model::ReplyPreview::from_message(message(id.0)).unwrap(),
						))),
					}),
				);
			}
		}
		assert!(
			reads.iter().all(|reads| *reads == 1),
			"evicted bodies must not restart service reads or starve later replies"
		);
		assert_eq!(
			visible
				.iter()
				.filter(|id| state.reply_preview_status(**id) == Status::Ready)
				.count(),
			128
		);
		assert_eq!(state.reply_preview_status(Id(1)), Status::Failed);
		for _ in 0..5 {
			assert!(state.sync_visible_reply_previews(&visible).is_empty());
		}
		let retry = state.retry_reply_preview(Id(1));
		assert!(matches!(
			retry.as_slice(),
			[Command::ReplyPreview { message: Id(1), .. }]
		));
		state.command_rejected(retry.into_iter().next().unwrap());
		assert_eq!(state.reply_preview_status(Id(1)), Status::Failed);
		assert!(state.sync_visible_reply_previews(&visible).is_empty());
		assert!(state.resident_history_bytes() <= SHARED_HISTORY_BYTES);
	}

	#[test]
	fn preview_attempts_reset_for_real_cancellation_edits_visibility_and_access() {
		use crate::reply_previews::Status;
		let mut state = state();
		let mut second = message(101);
		second.reply_to = Some(Id(51));
		state.timeline.insert(second, false, false).unwrap();
		for command in state.sync_visible_reply_previews(&[Id(50), Id(51)]) {
			state.command_rejected(command);
		}
		while state.reply_previews.evict() {}
		assert_eq!(state.reply_preview_status(Id(50)), Status::Failed);
		assert_eq!(state.reply_preview_status(Id(51)), Status::Failed);
		apply(
			&mut state,
			Event::Patch(model::MessagePatch {
				id: Id(50),
				channel: Id(1),
				content: model::Patch::Value("Edited original".into()),
				poll: model::Patch::Absent,
				sticker_items: model::Patch::Absent,
				flags: model::Patch::Absent,
				components: model::Patch::Absent,
				application_id: model::Patch::Absent,
				extra_content: Default::default(),
				reactions: model::Patch::Absent,
				mentions: model::Patch::Absent,
				edited: model::Patch::Absent,
				embeds: model::Patch::Absent,
				embeds_suppressed: model::Patch::Absent,
				attachments: model::Patch::Absent,
			}),
		);
		assert_eq!(state.reply_preview_status(Id(50)), Status::Unrequested);
		assert_eq!(
			state.reply_preview_status(Id(51)),
			Status::Failed,
			"an edit must invalidate only its own attempt"
		);
		assert!(matches!(
			state
				.sync_visible_reply_previews(&[Id(50), Id(51)])
				.as_slice(),
			[Command::ReplyPreview {
				message: Id(50),
				..
			}]
		));
		assert!(matches!(
			state.sync_visible_reply_previews(&[]).as_slice(),
			[Command::CancelReplyPreviews]
		));
		let pending = state.sync_visible_reply_previews(&[Id(50), Id(51)]);
		assert_eq!(
			pending.len(),
			2,
			"canceled and no-longer-visible attempts may start again"
		);
		for command in pending {
			state.command_rejected(command);
		}
		while state.reply_previews.evict() {}
		apply(&mut state, Event::PermissionsChanged);
		assert_eq!(state.reply_preview_status(Id(50)), Status::Unrequested);
		let mut source = message(100);
		source.reply_to = Some(Id(50));
		state.history(None);
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![source],
			},
		);
		assert!(matches!(
			state.sync_visible_reply_previews(&[Id(50)]).as_slice(),
			[Command::ReplyPreview {
				message: Id(50),
				..
			}]
		));
		state.select(Id(2)).unwrap();
		let mut source = message(200);
		source.channel = Id(2);
		source.reply_to = Some(Id(50));
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(2),
				request,
				older: false,
				messages: vec![source],
			},
		);
		assert!(
			state
				.sync_visible_reply_previews(&[Id(50)])
				.iter()
				.any(|command| matches!(
					command,
					Command::ReplyPreview {
						channel: Id(2),
						message: Id(50),
						..
					}
				)),
			"attempt IDs must not cross channels"
		);
	}

	#[test]
	fn visible_reply_work_is_bounded_and_failures_require_a_deliberate_retry() {
		use crate::reply_previews::{ResultEvent, Status};
		let mut state = state();
		for id in 1..=40 {
			let mut source = message(100 + id);
			source.reply_to = Some(Id(id));
			state.timeline.insert(source, false, false).unwrap();
		}
		let ids: Vec<_> = (1..=40).map(Id).collect();
		let commands = state.sync_visible_reply_previews(&ids);
		assert_eq!(
			commands
				.iter()
				.filter(|command| matches!(command, Command::ReplyPreview { .. }))
				.count(),
			2
		);
		let request = commands
			.iter()
			.find_map(|command| match command {
				Command::ReplyPreview {
					message: Id(1),
					request,
					..
				} => Some(*request),
				_ => None,
			})
			.unwrap();
		apply(
			&mut state,
			Event::ReplyPreview(ResultEvent {
				channel: Id(1),
				message: Id(1),
				request,
				result: Err(crate::auth::Failure::Network),
			}),
		);
		assert_eq!(state.reply_preview_status(Id(1)), Status::Failed);
		let commands = state.sync_visible_reply_previews(&ids);
		assert!(
			!commands
				.iter()
				.any(|command| matches!(command, Command::ReplyPreview { message: Id(1), .. }))
		);
		state.sync_visible_reply_previews(&[]);
		state.sync_visible_reply_previews(&[Id(1)]);
		assert!(
			state
				.retry_reply_preview(Id(1))
				.iter()
				.any(|command| matches!(command, Command::ReplyPreview { message: Id(1), .. }))
		);
		assert!(state.resident_history_bytes() < 16 * 1024 * 1024);
	}

	#[test]
	fn rejected_preview_cancellation_preserves_history_and_fences_old_results() {
		use crate::reply_previews::{ResultEvent, Status};
		let mut state = state();
		let request = |commands: Vec<Command>| {
			commands
				.into_iter()
				.find_map(|command| match command {
					Command::ReplyPreview { request, .. } => Some(request),
					_ => None,
				})
				.unwrap()
		};
		let old = request(state.sync_visible_reply_previews(&[Id(50)]));
		assert!(matches!(
			state.sync_visible_reply_previews(&[]).as_slice(),
			[Command::CancelReplyPreviews]
		));
		let current = request(state.sync_visible_reply_previews(&[Id(50)]));
		assert_ne!(old, current);
		state.status = "Keep this unrelated outcome";
		state.command_rejected(Command::CancelReplyPreviews);
		assert_eq!(state.status, "Keep this unrelated outcome");
		assert_eq!(state.freshness, Freshness::Fresh);
		assert!(!state.history_pending);
		for request in [old, current] {
			apply(
				&mut state,
				Event::ReplyPreview(ResultEvent {
					channel: Id(1),
					message: Id(50),
					request,
					result: Ok(Some(Box::new(
						model::ReplyPreview::from_message(message(50)).unwrap(),
					))),
				}),
			);
			assert_eq!(
				state.reply_preview_status(Id(50)),
				if request == old {
					Status::Loading
				} else {
					Status::Ready
				}
			);
			assert_eq!(state.status, "Keep this unrelated outcome");
			assert_eq!(state.freshness, Freshness::Fresh);
		}
	}

	const SHARED_HISTORY_BYTES: usize = 16 * 1024 * 1024 - 66 * 1024;

	fn byte_limited_reply_state() -> State {
		let mut state = state();
		let mut fourth = state.channels[0].clone();
		fourth.id = Id(4);
		state.channels.push(fourth);
		for channel in 1..=4 {
			if channel == 1 {
				state.history(None);
			} else {
				state.select(Id(channel)).unwrap();
			}
			let mut source = message(channel * 10000);
			source.channel = Id(channel);
			if channel == 4 {
				source.reply_to = Some(Id(50));
			}
			let request = state.request;
			apply(
				&mut state,
				Event::History {
					channel: Id(channel),
					request,
					older: false,
					messages: vec![source],
				},
			);
			if channel < 4 {
				// A real cache may hold many individually valid 64 KiB messages. The
				// timeline bounds this hydration before the shared budget is applied.
				state
					.timeline
					.seed_cache(
						(1..=80)
							.map(|offset| {
								let mut item = message(channel * 10000 + offset);
								item.channel = Id(channel);
								item.content = "x".repeat(64 * 1024);
								item
							})
							.collect(),
					)
					.unwrap();
				state
					.channels
					.iter_mut()
					.find(|item| item.id == Id(channel))
					.unwrap()
					.last_message = Some(Id(channel * 10000 + 80));
				state.enforce_resident_budget();
			}
		}
		assert_eq!(state.resident_window_count(), 3);
		let source_capacity = state.timeline.get(Id(40000)).unwrap().content.capacity();
		for id in 40001..40128 {
			let remaining = SHARED_HISTORY_BYTES - state.resident_history_bytes() - 1024;
			if remaining <= 64 * 1024 - source_capacity {
				break;
			}
			let mut incoming = message(id);
			incoming.channel = Id(4);
			incoming.content = "x".repeat(48 * 1024);
			apply(&mut state, Event::Message(incoming));
		}
		let mut source = state.timeline.get(Id(40000)).unwrap().clone();
		let remaining = SHARED_HISTORY_BYTES - state.resident_history_bytes() - 1024;
		source.content = "x".repeat(source.content.capacity() + remaining);
		assert!(Timeline::valid_message(&source));
		apply(&mut state, Event::Message(source));
		assert_eq!(state.resident_window_count(), 3);
		assert_eq!(state.resident_history_bytes(), SHARED_HISTORY_BYTES - 1024);
		state
	}

	#[test]
	fn preview_growth_obeys_the_shared_budget_with_large_dormant_windows() {
		use crate::reply_previews::ResultEvent;
		for path in ["fetched", "live", "history"] {
			let mut state = byte_limited_reply_state();
			let source = state.timeline.get(Id(40000)).unwrap().clone();
			let mut original = message(50);
			original.channel = Id(4);
			original.content = "p".repeat(4096);
			let preview = Box::new(model::ReplyPreview::from_message(original).unwrap());
			assert!(preview.bytes() > 1024);
			let request = state
				.sync_visible_reply_previews(&[Id(50)])
				.into_iter()
				.find_map(|command| match command {
					Command::ReplyPreview { request, .. } => Some(request),
					_ => None,
				})
				.unwrap();
			if path == "fetched" {
				apply(
					&mut state,
					Event::ReplyPreview(ResultEvent {
						channel: Id(4),
						message: Id(50),
						request,
						result: Ok(Some(preview)),
					}),
				);
			} else {
				let mut incoming = source.clone();
				incoming.reply_preview = Some(preview);
				if path == "history" {
					state.history(Some(Id(40001)));
					let request = state.request;
					apply(
						&mut state,
						Event::History {
							channel: Id(4),
							request,
							older: true,
							messages: vec![incoming],
						},
					);
				} else {
					apply(&mut state, Event::Message(incoming));
				}
			}
			assert!(
				state.resident_history_bytes() <= SHARED_HISTORY_BYTES,
				"{path}"
			);
			assert!(state.resident_history_rows() <= 1475);
			assert_eq!(
				state.resident_window_count(),
				3,
				"preview payloads give way before dormant windows"
			);
			assert_eq!(
				state.timeline.get(source.id).unwrap().content,
				source.content
			);
			assert!(
				state.reply_original(Id(50)).is_none(),
				"the over-budget {path} preview must be evicted"
			);
			assert_eq!(state.freshness, Freshness::Fresh);
			assert_eq!(
				state.reply_preview_status(Id(50)),
				crate::reply_previews::Status::Failed,
				"{path}"
			);
			for _ in 0..5 {
				assert!(
					!state
						.sync_visible_reply_previews(&[Id(50)])
						.iter()
						.any(|command| matches!(command, Command::ReplyPreview { .. })),
					"memory pressure must not trigger {path} reads on repaint"
				);
			}
			let retry = state.retry_reply_preview(Id(50));
			assert!(retry.iter().any(|command| matches!(
				command,
				Command::ReplyPreview {
					message: Id(50),
					..
				}
			)));
			for command in retry {
				state.command_rejected(command);
			}
			assert_eq!(
				state.reply_preview_status(Id(50)),
				crate::reply_previews::Status::Failed
			);
			assert!(state.sync_visible_reply_previews(&[Id(50)]).is_empty());
		}
	}

	#[test]
	fn visible_reply_metadata_and_direct_queue_failures_obey_the_shared_budget() {
		{
			let mut state = byte_limited_reply_state();
			let content = state.timeline.get(Id(40000)).unwrap().content.clone();
			let visible: Vec<_> = (1..=crate::reply_previews::MAX_VISIBLE_REPLIES as u64)
				.map(Id)
				.collect();
			state.sync_visible_reply_previews(&visible);
			assert!(state.resident_history_bytes() <= SHARED_HISTORY_BYTES);
			assert_eq!(
				state.resident_window_count(),
				2,
				"new visible metadata is charged outside Event::apply"
			);
			assert_eq!(state.timeline.get(Id(40000)).unwrap().content, content);
		}
		{
			let mut state = byte_limited_reply_state();
			let request = state
				.sync_visible_reply_previews(&[Id(50)])
				.into_iter()
				.find(|command| matches!(command, Command::ReplyPreview { .. }))
				.unwrap();
			let mut source = state.timeline.get(Id(40001)).unwrap().clone();
			let remaining = SHARED_HISTORY_BYTES - state.resident_history_bytes() - 64;
			source.content = "x".repeat(source.content.capacity() + remaining);
			assert!(Timeline::valid_message(&source));
			apply(&mut state, Event::Message(source));
			assert_eq!(state.resident_history_bytes(), SHARED_HISTORY_BYTES - 64);
			state.command_rejected(request);
			assert!(state.resident_history_bytes() <= SHARED_HISTORY_BYTES);
			assert_eq!(
				state.reply_preview_status(Id(50)),
				crate::reply_previews::Status::Failed
			);
			assert!(state.sync_visible_reply_previews(&[Id(50)]).is_empty());
			assert_eq!(state.freshness, Freshness::Fresh);
		}
	}

	#[test]
	fn atomic_reply_install_obeys_shared_byte_budget_and_preserves_back_cursor() {
		let mut state = byte_limited_reply_state();
		state.remember_reading(
			Id(4),
			crate::ReadingCursor {
				message: Some(Id(40000)),
				inset: 19.0,
			},
		);
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!()
		};
		let originals = (25..75)
			.map(|id| {
				let mut original = message(id);
				original.channel = Id(4);
				original.content = "t".repeat(64 * 1024);
				original
			})
			.collect();
		apply(
			&mut state,
			Event::History {
				channel: Id(4),
				request,
				older: false,
				messages: originals,
			},
		);
		assert!(state.resident_history_bytes() <= SHARED_HISTORY_BYTES);
		assert_eq!(state.resident_window_count(), 3);
		assert_eq!(state.timeline.get(Id(50)).unwrap().content.len(), 64 * 1024);
		assert_eq!(state.search_target, Some(Id(50)));
		assert_eq!(
			state.history_navigation.bytes(),
			0,
			"retire the return payload under pressure"
		);
		assert!(state.can_return_from_reply());
		assert!(matches!(
			state.return_from_reply(),
			Some(Command::History {
				channel: Id(4),
				before: None,
				after: Some(Id(39999)),
				..
			})
		));
		assert_eq!(state.reading(Id(4)).unwrap().inset, 19.0);
	}

	#[test]
	fn atomic_reply_install_obeys_shared_row_budget_and_preserves_back_cursor() {
		let mut state = state();
		for (channel, rows) in [(1, 500), (2, 475), (3, 500)] {
			if channel == 1 {
				state.history(None);
			} else {
				state.select(Id(channel)).unwrap();
			}
			let mut first = message(channel * 10000);
			first.channel = Id(channel);
			first.reply_to = Some(Id(50));
			let request = state.request;
			apply(
				&mut state,
				Event::History {
					channel: Id(channel),
					request,
					older: false,
					messages: vec![first],
				},
			);
			// Production disk hydration can restore the rest of a bounded 500-row window.
			state
				.timeline
				.seed_cache(
					(1..rows)
						.map(|offset| {
							let mut item = message(channel * 10000 + offset);
							item.channel = Id(channel);
							item
						})
						.collect(),
				)
				.unwrap();
			state
				.channels
				.iter_mut()
				.find(|item| item.id == Id(channel))
				.unwrap()
				.last_message = Some(Id(channel * 10000 + rows - 1));
			state.enforce_resident_budget();
		}
		assert_eq!(state.resident_window_count(), 2);
		assert_eq!(state.resident_history_rows(), 1475);
		state.remember_reading(
			Id(3),
			crate::ReadingCursor {
				message: Some(Id(30000)),
				inset: 23.0,
			},
		);
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!()
		};
		apply(
			&mut state,
			Event::History {
				channel: Id(3),
				request,
				older: false,
				messages: (25..75)
					.map(|id| {
						let mut item = message(id);
						item.channel = Id(3);
						item
					})
					.collect(),
			},
		);
		assert_eq!(state.timeline.row_count(), 50);
		assert_eq!(state.resident_history_rows(), 1025);
		assert!(state.resident_history_bytes() <= SHARED_HISTORY_BYTES);
		assert_eq!(state.resident_window_count(), 2);
		assert_eq!(state.search_target, Some(Id(50)));
		assert_eq!(state.history_navigation.rows(), 0);
		assert!(state.can_return_from_reply());
		assert!(matches!(
			state.return_from_reply(),
			Some(Command::History {
				channel: Id(3),
				before: None,
				after: Some(Id(29999)),
				..
			})
		));
		assert_eq!(state.reading(Id(3)).unwrap().inset, 23.0);
	}

	#[test]
	fn reply_jump_keeps_the_source_until_success_and_restores_its_anchor() {
		let mut state = state();
		state.remember_reading(
			Id(1),
			crate::ReadingCursor {
				message: Some(Id(100)),
				inset: 23.0,
			},
		);
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("around request")
		};
		assert!(state.timeline.get(Id(100)).is_some());
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request,
				failure: crate::auth::Failure::Network,
			},
		);
		assert!(state.timeline.get(Id(100)).is_some());
		assert_eq!(state.freshness, Freshness::Fresh);
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("around retry")
		};
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(49), message(50), message(51)],
			},
		);
		assert!(state.timeline.get(Id(100)).is_none());
		assert_eq!(state.search_target, Some(Id(50)));
		assert!(state.can_return_from_reply());
		assert!(state.navigate_back().is_some());
		assert!(state.timeline.get(Id(100)).is_some());
		assert_eq!(state.search_target, Some(Id(100)));
		assert_eq!(state.reading(Id(1)).unwrap().inset, 23.0);
		assert!(state.restore_scroll);
	}

	#[test]
	fn reply_jump_requeues_the_reaction_read_cancelled_by_history() {
		use crate::reactions::{Command as R, Event as E};
		let mut state = state();
		let (channel, message_id) = (Id(1), Id(100));
		let emoji = model::ReactionEmoji {
			id: None,
			name: Some("👍".into()),
		};
		let Some(Command::Reactions(R::Set { request, .. })) =
			state.prepare_reaction(message_id, emoji.clone())
		else {
			panic!("reaction write")
		};
		apply(
			&mut state,
			Event::Reactions(E::Written {
				channel,
				message: message_id,
				request,
				result: Ok(()),
			}),
		);
		let Some(Command::Reactions(R::Read {
			request: cancelled, ..
		})) = state.next_reaction_read()
		else {
			panic!("first readback")
		};
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("around request")
		};
		apply(
			&mut state,
			Event::History {
				channel,
				request,
				older: false,
				messages: vec![message(50), message(100)],
			},
		);
		let Some(Command::Reactions(R::Read {
			message, request, ..
		})) = state.next_reaction_read()
		else {
			panic!("reply navigation must not strand the readback")
		};
		assert_eq!(message, message_id);
		assert_ne!(request, cancelled);
		assert!(state.reactions.busy());
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message,
				request: cancelled,
				result: Ok(vec![]),
			}),
		);
		assert!(state.reactions.busy());
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message,
				request,
				result: Ok(vec![model::Reaction {
					emoji,
					count: 1,
					me: true,
					me_burst: false,
				}]),
			}),
		);
		assert!(!state.reactions.busy());
	}

	#[test]
	fn changed_return_windows_revalidate_instead_of_restoring_old_payloads() {
		for deleted in [false, true] {
			let mut state = state();
			state.remember_reading(
				Id(1),
				crate::ReadingCursor {
					message: Some(Id(100)),
					inset: 17.0,
				},
			);
			let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50))
			else {
				panic!()
			};
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: false,
					messages: vec![message(50)],
				},
			);
			assert!(state.history_navigation.bytes() > 0);
			if deleted {
				apply(
					&mut state,
					Event::Delete {
						channel: Id(1),
						id: Id(100),
					},
				);
			} else {
				let mut edited = message(100);
				edited.content = "New live content".into();
				edited.edited = true;
				edited.edited_at = Some(123);
				apply(&mut state, Event::Message(edited));
			}
			assert_eq!(
				state.history_navigation.bytes(),
				0,
				"mutated return payload must be retired"
			);
			let step = state.navigate_back().unwrap();
			assert!(
				matches!(
					step.command,
					Some(Command::History {
						after: Some(Id(99)),
						..
					})
				),
				"Back revalidates around the saved anchor"
			);
			assert!(state.timeline.get(Id(100)).is_none());
			assert_eq!(state.reading(Id(1)).unwrap().inset, 17.0);
		}
	}

	#[test]
	fn clearing_history_retires_reply_cache_and_pending_navigation() {
		let mut state = state();
		let mut source = message(101);
		source.reply_to = Some(Id(50));
		source.reply_preview = Some(Box::new(
			model::ReplyPreview::from_message(message(50)).unwrap(),
		));
		apply(&mut state, Event::Message(source));
		assert!(state.reply_original(Id(50)).is_some());
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!()
		};
		state.clear_cached_history();
		assert!(state.reply_original(Id(50)).is_none());
		assert!(!state.history_pending && !state.can_return_from_reply());
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(50)],
			},
		);
		assert!(state.timeline.get(Id(50)).is_none());
		assert!(state.timeline.get(Id(100)).is_some());
	}

	#[test]
	fn preview_accounting_survives_live_replacement_eviction_and_deletion() {
		let mut state = state();
		for id in 1..=180 {
			let mut source = message(1000 + id);
			source.reply_to = Some(Id(id));
			let mut original = message(id);
			original.content = "Bounded preview. ".repeat((id % 20 + 1) as usize);
			source.reply_preview = Some(Box::new(
				model::ReplyPreview::from_message(original).unwrap(),
			));
			apply(&mut state, Event::Message(source));
			state.reply_previews.assert_accounting();
		}
		for id in [60, 100, 180] {
			apply(
				&mut state,
				Event::Delete {
					channel: Id(1),
					id: Id(id),
				},
			);
			state.reply_previews.assert_accounting();
		}
		state.clear_cached_history();
		state.reply_previews.assert_accounting();
	}

	#[test]
	fn report_navigation_submits_only_explicitly_and_rejects_stale_or_duplicate_results() {
		use crate::reporting::{Event as ReportEvent, Outcome};
		use model::reporting::{Button, Menu, Node, Target};
		let mut state = state();
		let mut original = message(100);
		original.author.id = Id(10);
		apply(&mut state, Event::Message(original));
		let target = Target {
			channel: Id(1),
			message: Id(100),
		};
		let Command::Report(read) = state.open_report(target).unwrap() else {
			panic!()
		};
		assert!(state.report_command_allowed(&read));
		assert!(state.submit_report().is_none());
		let menu = Menu {
			version: "1.0".into(),
			variant: "test".into(),
			language: "en".into(),
			root: 1,
			nodes: [
				(
					1,
					Node {
						header: "Reason".into(),
						detail: String::new(),
						children: vec![("Spam".into(), 2)],
						elements: vec![],
						button: Button::None,
						required: false,
					},
				),
				(
					2,
					Node {
						header: "Confirm".into(),
						detail: String::new(),
						children: vec![],
						elements: vec![],
						button: Button::Submit,
						required: false,
					},
				),
			]
			.into_iter()
			.collect(),
		};
		apply(
			&mut state,
			Event::Report(ReportEvent {
				target,
				request: read.request,
				result: Ok(Outcome::Menu(Box::new(menu.clone()))),
			}),
		);
		assert!(
			state.submit_report().is_none(),
			"opening or choosing a reason never submits"
		);
		state.choose_report_node(999);
		assert_eq!(state.reporting.breadcrumbs, [1]);
		state.choose_report_node(2);
		assert_eq!(state.reporting.breadcrumbs, [1, 2]);
		state.report_back();
		assert_eq!(state.reporting.breadcrumbs, [1]);
		state.choose_report_node(2);
		let Command::Report(write) = state.submit_report().unwrap() else {
			panic!()
		};
		assert!(state.report_command_allowed(&write));
		assert!(state.submit_report().is_none());
		apply(
			&mut state,
			Event::Report(ReportEvent {
				target,
				request: write.request,
				result: Err(Failure::Ambiguous),
			}),
		);
		assert!(
			state.submit_report().is_none(),
			"uncertain reports must not be repeated"
		);
		state.reporting.close();
		apply(
			&mut state,
			Event::Report(ReportEvent {
				target,
				request: write.request,
				result: Ok(Outcome::Sent(Id(99))),
			}),
		);
		assert!(state.reporting.target.is_none() && state.reporting.sent.is_none());
		let Command::Report(read) = state.open_report(target).unwrap() else {
			panic!()
		};
		apply(
			&mut state,
			Event::Report(ReportEvent {
				target,
				request: read.request,
				result: Ok(Outcome::Menu(Box::new(menu))),
			}),
		);
		state.choose_report_node(2);
		let Command::Report(write) = state.submit_report().unwrap() else {
			panic!()
		};
		apply(
			&mut state,
			Event::Report(ReportEvent {
				target,
				request: write.request,
				result: Ok(Outcome::Sent(Id(99))),
			}),
		);
		assert_eq!(state.reporting.sent, Some(Id(99)));
		assert!(state.submit_report().is_none());
		apply(
			&mut state,
			Event::Delete {
				channel: Id(1),
				id: Id(100),
			},
		);
		assert!(state.reporting.target.is_none());
		assert!(state.open_report(target).is_none());
		let own = message(101);
		apply(&mut state, Event::Message(own));
		assert!(
			state
				.open_report(Target {
					channel: Id(1),
					message: Id(101)
				})
				.is_none()
		);
	}

	#[test]
	fn missing_target_never_replaces_current_history_and_new_jump_supersedes_pagination() {
		let mut state = state();
		let _ = state.history(Some(Id(100)));
		assert!(state.can_open_reply_target(Id(50)));
		let Some(Command::HistoryAround { request, .. }) = state.open_reply_target(Id(50)) else {
			panic!("superseding around request")
		};
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(49), message(51)],
			},
		);
		assert!(state.timeline.get(Id(100)).is_some());
		assert!(!state.history_pending);
	}

	#[test]
	fn explicit_reply_send_preserves_composer_state_and_requires_a_loaded_safe_target() {
		let mut state = state();
		state.drafts.insert(Id(1), "Keep draft".into());
		state.reply = Some(Reply::to(Id(50)));
		let Command::Send {
			channel,
			content,
			reply,
			sticker,
			..
		} = state
			.prepare_reply_send(Id(100), "Approved reply", false)
			.unwrap()
		else {
			panic!("expected reply send")
		};
		assert_eq!(channel, Id(1));
		assert_eq!(content, "Approved reply");
		assert_eq!(sticker, None);
		assert!(reply.is_some_and(|reply| reply.target() == Id(100) && !reply.mention));
		assert_eq!(state.drafts[&Id(1)], "Keep draft");
		assert_eq!(state.reply, Some(Reply::to(Id(50))));

		assert!(state.prepare_reply_send(Id(999), "Unknown", true).is_none());
		let mut ephemeral = self::state();
		ephemeral.timeline.clear();
		let mut message = message(100);
		message.ephemeral = true;
		ephemeral.timeline.insert(message, false, false).unwrap();
		assert!(
			ephemeral
				.prepare_reply_send(Id(100), "Ephemeral", true)
				.is_none()
		);
		state.selected = Some(Id(2));
		assert!(
			state
				.prepare_reply_send(Id(100), "Wrong channel", true)
				.is_none()
		);
		assert_eq!(state.pending.len(), 1);
	}
	fn deleted_source(id: u64, target: u64, channel: u64) -> Message {
		let mut source = message(id);
		source.kind = 19;
		source.channel = Id(channel);
		source.reply_to = Some(Id(target));
		source.reply_deleted = true;
		source
	}
	fn chat_link_state() -> State {
		let mut state = state();
		let mut dm = state.channels[0].clone();
		dm.id = Id(4);
		for channel in &mut state.channels {
			channel.guild = Some(Id(if channel.id == Id(3) { 20 } else { 10 }));
			channel.kind = 0;
		}
		state.channels.push(dm);
		state.guilds = [10, 20]
			.into_iter()
			.map(|id| model::Guild {
				default_message_notifications: None,
				stickers: None,
				id: Id(id),
				name: "Synthetic guild".into(),
				icon: None,
				emojis: None,
			})
			.collect();
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: [10, 20]
					.into_iter()
					.map(|id| model::permissions::Guild {
						id: Id(id),
						owner: Some(Id(9)),
						roles: None,
						member: None,
					})
					.collect(),
				channels: vec![],
			})
			.unwrap();
		state.drafts.insert(Id(1), "Unsent draft".into());
		state
	}

	#[test]
	fn chat_links_select_same_server_other_server_and_dm_without_joining_voice() {
		{
			for (guild, channel, kind) in [
				(Some(Id(10)), Id(2), 0),
				(Some(Id(20)), Id(3), 0),
				(None, Id(4), 1),
				(Some(Id(20)), Id(3), 15),
				(Some(Id(20)), Id(3), 2),
			] {
				let mut state = chat_link_state();
				state
					.channels
					.iter_mut()
					.find(|c| c.id == channel)
					.unwrap()
					.kind = kind;
				let command = state.open_chat_link(guild, channel, None).unwrap();
				assert_eq!(state.selected, Some(channel));
				assert!(command.is_none_or(
					|command| matches!(command, Command::History { channel: id, .. } if id == channel)
				));
				assert!(state.voice.active.is_none());
				assert_eq!(state.drafts[&Id(1)], "Unsent draft");
			}
		}
		{
			let mut state = chat_link_state();
			let request = state.request;
			assert!(
				state
					.open_chat_link(Some(Id(10)), Id(1), Some(Id(100)))
					.unwrap()
					.is_none()
			);
			assert_eq!(state.search_target, Some(Id(100)));
			assert_eq!(state.request, request);
			assert!(state.timeline.get(Id(100)).is_some());
			for (guild, channel) in [(Some(Id(10)), Id(1)), (Some(Id(20)), Id(3)), (None, Id(4))] {
				let mut state = chat_link_state();
				let command = state
					.open_chat_link(guild, channel, Some(Id(50)))
					.unwrap()
					.unwrap();
				assert!(
					matches!(command, Command::HistoryAround { channel: id, target: Id(50), request } if id == channel && request == state.request)
				);
				assert_eq!(state.selected, Some(channel));
				assert!(state.search_target.is_none());
				assert!(state.history_pending);
				assert_eq!(state.drafts[&Id(1)], "Unsent draft");
			}
		}
	}

	#[test]
	fn chat_links_reject_invalid_unavailable_and_forbidden_targets_before_navigation() {
		{
			for blocked in 0..11 {
				let mut state = chat_link_state();
				let mut guild = Some(Id(20));
				let mut channel = Id(3);
				let mut message = Some(Id(50));
				match blocked {
					0 => channel = Id(0),
					1 => channel = Id(999),
					2 => guild = None,
					3 => message = Some(Id(0)),
					4 => message = Some(Id(u64::MAX)),
					5 => state.channels[2].kind = 4,
					6 => state.user = None,
					7 => state.gateway_connected = false,
					8 => state.auth = AuthState::Unauthenticated,
					9 => state.channels[2].kind = 15,
					_ => {
						guild = Some(Id(10));
						channel = Id(1);
						state.timeline.delete(Id(50)).unwrap();
					}
				}
				let revision = state.revision;
				let request = state.request;
				assert!(
					state.open_chat_link(guild, channel, message).is_err(),
					"case {blocked}"
				);
				assert_eq!(state.selected, Some(Id(1)));
				assert_eq!(state.revision, revision);
				assert_eq!(state.request, request);
				assert!(state.search_target.is_none());
				assert!(state.timeline.get(Id(100)).is_some());
				assert_eq!(state.drafts[&Id(1)], "Unsent draft");
			}
		}
		{
			let mut state = chat_link_state();
			state.timeline.delete(Id(50)).unwrap();
			state.select(Id(2));
			assert_eq!(state.resident_window_count(), 1);
			let request = state.request;
			assert_eq!(
				state
					.open_chat_link(Some(Id(10)), Id(1), Some(Id(50)))
					.err(),
				Some("This message was deleted")
			);
			assert_eq!(state.status, "This message was deleted");
			assert!(state.search_target.is_none());
			assert_eq!(state.selected, Some(Id(2)));
			assert_eq!(state.request, request);
			assert!(
				state.history_pending,
				"keep the unrelated selected-channel request"
			);
		}
	}

	#[test]
	fn a_resident_without_the_live_target_is_not_presented_as_loaded() {
		for delete_live in [false, true] {
			let parked = || {
				let mut state = chat_link_state();
				state.timeline.delete(Id(50)).unwrap();
				if delete_live {
					// Only deletion placeholders remain in the parked window.
					state.timeline.delete(Id(100)).unwrap();
				}
				state.select(Id(2));
				assert_eq!(state.resident_window_count(), 1);
				state
			};
			// The restore step alone must not claim a window without the target is loaded.
			let mut state = parked();
			state.select_chat_link_channel(Id(1), Id(60)).unwrap();
			assert_eq!(state.selected, Some(Id(1)));
			assert_ne!(state.freshness, Freshness::Fresh, "case {delete_live}");
			let mut state = parked();
			state.select_chat_link_channel(Id(1), Id(100)).unwrap();
			assert_eq!(
				state.freshness == Freshness::Fresh,
				!delete_live,
				"only a live target restores locally (case {delete_live})"
			);
			// The full link then requests the target's page.
			let mut state = parked();
			let command = state
				.open_chat_link(Some(Id(10)), Id(1), Some(Id(60)))
				.unwrap();
			assert!(matches!(
				command,
				Some(Command::HistoryAround {
					channel: Id(1),
					target: Id(60),
					..
				})
			));
		}
	}

	#[test]
	fn accepted_deletion_effects_are_scoped_bounded_and_reset_on_rejected_generation() {
		{
			let mut state = state();
			state.timeline.insert(message(50), false, false).unwrap();
			apply(&mut state, Event::Message(deleted_source(101, 50, 2)));
			assert_eq!(state.take_reply_deletions(), vec![(Id(2), Id(50))]);
			assert!(state.timeline.get(Id(50)).is_some()); // Inactive channel is independent.
			apply(&mut state, Event::Message(deleted_source(101, 50, 1)));
			assert!(state.timeline.get(Id(50)).is_none());
			state.apply(Envelope {
				generation: state.generation - 1,
				event: Event::Message(deleted_source(102, 40, 1)),
			});
			assert!(state.take_reply_deletions().is_empty());
			assert!(!state.timeline.is_deleted(Id(40)));
			state.history(Some(Id(201)));
			let request = state.request;
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: true,
					messages: (1..=50)
						.map(|target| deleted_source(100 + target, target, 1))
						.collect(),
				},
			);
			let effects = state.take_reply_deletions();
			assert_eq!(effects.len(), 50);
			assert!(effects.iter().all(|(channel, _)| *channel == Id(1)));
			assert!(state.take_reply_deletions().is_empty());
		}
		{
			for invalid in 0..6 {
				let mut state = state();
				state.timeline.insert(message(50), false, false).unwrap();
				state.history(Some(Id(150)));
				let mut request = state.request;
				let mut source = deleted_source(101, 50, 1);
				let mut messages = vec![];
				match invalid {
					0 => request += 1,
					1 => source.channel = Id(2),
					2 => source.id = Id(151),
					3 => messages.push(source.clone()),
					4 => source.kind = 0,
					_ => {
						let mut oversized = message(102);
						oversized.content = "x".repeat(64 * 1024 + 1);
						messages.push(oversized);
					}
				}
				messages.push(source);
				apply(
					&mut state,
					Event::History {
						channel: Id(1),
						request,
						older: true,
						messages,
					},
				);
				assert!(state.take_reply_deletions().is_empty());
				assert!(state.timeline.get(Id(50)).is_some());
				assert!(!state.timeline.is_deleted(Id(50)));
			}
		}
	}

	#[test]
	fn send_deletion_requires_correlation_and_handles_gateway_first_without_replacing_text() {
		{
			for invalid in 0..3 {
				let mut state = state();
				state.timeline.insert(message(50), false, false).unwrap();
				if invalid != 0 {
					state.pending.push(Pending {
						sticker: None,
						channel: Id(if invalid == 1 { 2 } else { 1 }),
						content: "Pending".into(),
						attachments: vec![],
						nonce: "synthetic".into(),
						delivery: Delivery::Ambiguous,
						confirmed: None,
						reply: None,
					});
				}
				let mut source = deleted_source(101, 50, 1);
				if invalid == 2 {
					source.author.id = Id(8);
				}
				apply(
					&mut state,
					Event::SendResult {
						nonce: "synthetic".into(),
						result: Ok(source),
					},
				);
				assert!(state.take_reply_deletions().is_empty());
				assert!(state.timeline.get(Id(50)).is_some());
			}
			let mut state = state();
			state.timeline.insert(message(50), false, false).unwrap();
			state.pending.push(Pending {
				sticker: None,
				channel: Id(1),
				content: "Pending".into(),
				attachments: vec![],
				nonce: "synthetic".into(),
				delivery: Delivery::Ambiguous,
				confirmed: None,
				reply: None,
			});
			let mut source = deleted_source(101, 50, 1);
			source.reply_deleted = false;
			source.nonce = Some("synthetic".into());
			source.content = "Newer Gateway body".into();
			source.edited_at = Some(20);
			apply(&mut state, Event::Message(source));
			assert!(state.pending.is_empty());
			apply(
				&mut state,
				Event::SendResult {
					nonce: "synthetic".into(),
					result: Ok(deleted_source(101, 50, 1)),
				},
			);
			assert_eq!(state.take_reply_deletions(), vec![(Id(1), Id(50))]);
			assert!(state.timeline.is_deleted(Id(50)));
			let source = state.timeline.get(Id(101)).unwrap();
			assert_eq!(source.content, "Newer Gateway body");
			assert!(source.reply_deleted);
			state.pending.push(Pending {
				sticker: None,
				channel: Id(2),
				content: "Pending".into(),
				attachments: vec![],
				nonce: "other".into(),
				delivery: Delivery::Ambiguous,
				confirmed: None,
				reply: None,
			});
			apply(
				&mut state,
				Event::SendResult {
					nonce: "other".into(),
					result: Ok(deleted_source(102, 40, 2)),
				},
			);
			assert_eq!(state.take_reply_deletions(), vec![(Id(2), Id(40))]);
			assert!(!state.timeline.is_deleted(Id(40)));
		}
		{
			let mut state = state();
			state.drafts.insert(Id(1), "Unsent draft".into());
			state.timeline.insert(message(50), false, false).unwrap();
			let mut source = deleted_source(101, 50, 1);
			source.reply_deleted = false;
			source.nonce = Some("synthetic".into());
			state.timeline.insert(source, false, false).unwrap();
			for id in 1000..1000 + session_cache::MAX_MUTATIONS as u64 {
				state.timeline.delete(Id(id)).unwrap();
			}
			apply(
				&mut state,
				Event::SendResult {
					nonce: "synthetic".into(),
					result: Ok(deleted_source(101, 50, 1)),
				},
			);
			assert!(state.timeline.is_empty());
			assert_eq!(state.freshness, Freshness::Stale);
			assert_eq!(state.take_reply_deletions(), vec![(Id(1), Id(50))]);
			assert_eq!(state.drafts[&Id(1)], "Unsent draft");
		}
	}

	#[test]
	fn loaded_reply_target_only_scrolls_and_preserves_composer_work() {
		{
			let mut state = state();
			state.timeline.insert(message(50), false, false).unwrap();
			state.reply = Some(Reply::to(Id(100)));
			state.drafts.insert(Id(1), "Unsent draft".into());
			state.pending.push(Pending {
				sticker: None,
				channel: Id(1),
				content: "Pending".into(),
				attachments: vec![],
				nonce: "synthetic".into(),
				delivery: Delivery::Ambiguous,
				confirmed: None,
				reply: None,
			});
			let request = state.request;
			let revision = state.revision;
			let content = state.timeline.get(Id(50)).unwrap().content.as_ptr();
			assert!(state.can_open_reply_target(Id(50)));
			assert!(state.open_reply_target(Id(50)).is_none());
			assert_eq!(state.search_target, Some(Id(50)));
			assert_eq!(state.request, request);
			assert_eq!(state.revision, revision + 1);
			assert!(!state.history_pending);
			assert_eq!(state.freshness, Freshness::Fresh);
			assert_eq!(
				state.timeline.get(Id(50)).unwrap().content.as_ptr(),
				content
			);
			assert_eq!(state.reply_target(), Some(Id(100)));
			assert_eq!(state.drafts[&Id(1)], "Unsent draft");
			assert_eq!(state.pending[0].delivery, Delivery::Ambiguous);
			assert!(state.can_open_reply_target(Id(100))); // Composer context is also a source.
		}
		{
			let mut state = state();
			state.reply = Some(Reply::to(Id(50)));
			state.drafts.insert(Id(1), "Unsent draft".into());
			apply(
				&mut state,
				Event::Delete {
					channel: Id(1),
					id: Id(40),
				},
			);
			let command = state.open_reply_target(Id(50)).unwrap();
			let Command::HistoryAround {
				channel,
				target,
				request,
				..
			} = command
			else {
				panic!()
			};
			assert_eq!(channel, Id(1));
			assert_eq!(target, Id(50));
			assert_eq!(state.timeline.row_count(), 1);
			assert!(state.timeline.is_deleted(Id(40)));
			assert_eq!(state.freshness, Freshness::Fresh);
			assert_eq!(state.reply_target(), Some(Id(50)));
			assert_eq!(state.drafts[&Id(1)], "Unsent draft");
			assert!(state.open_reply_target(Id(50)).is_none());
			assert_eq!(state.request, request);
			// A delete racing the request must also defeat both live and REST bodies.
			apply(
				&mut state,
				Event::Delete {
					channel: Id(1),
					id: Id(50),
				},
			);
			apply(&mut state, Event::Message(message(50)));
			apply(
				&mut state,
				Event::History {
					channel,
					request,
					older: false,
					messages: vec![message(50), message(40), message(30)],
				},
			);
			assert_eq!(state.timeline.row_ids().collect::<Vec<_>>(), vec![Id(100)]);
			assert!(state.timeline.is_deleted(Id(50)));
			assert!(!state.can_open_reply_target(Id(50)));
			assert_eq!(state.freshness, Freshness::Fresh);
		}
	}

	#[test]
	fn reply_targets_require_a_current_scoped_reference_and_safe_state() {
		{
			let state = state();
			assert!(!state.can_open_reply_target(Id(25)));
			assert!(!state.can_open_reply_target(Id(100))); // Loaded alone is not a reference.
			for target in [Id(0), Id(u64::MAX)] {
				let mut invalid = self::state();
				invalid.reply = Some(Reply::to(target));
				assert!(!invalid.can_open_reply_target(target));
				assert!(invalid.open_reply_target(target).is_none());
			}
			for blocked in 0..8 {
				let mut state = self::state();
				match blocked {
					0 => state.auth = AuthState::Unauthenticated,
					1 => state.gateway_connected = false,
					2 => state.freshness = Freshness::Stale,
					3 => {
						state.history_pending = true;
						state.freshness = Freshness::Unavailable;
					}
					4 => state.selected = Some(Id(2)),
					5 => state.channels[0].kind = 2,
					6 => {
						state.timeline.delete(Id(50)).unwrap();
					}
					_ => {
						let mut target = message(50);
						target.channel = Id(2);
						state.timeline.insert(target, false, false).unwrap();
					}
				}
				assert!(!state.can_open_reply_target(Id(50)));
				assert!(state.open_reply_target(Id(50)).is_none());
				assert!(state.search_target.is_none());
			}
		}
		{
			for transition in 0..4 {
				let mut state = state();
				state.open_reply_target(Id(50)).unwrap();
				let request = state.request;
				let generation = state.generation;
				match transition {
					0 => apply(
						&mut state,
						Event::HistoryFailed {
							channel: Id(1),
							request,
							failure: Failure::Network,
						},
					),
					1 => {
						state.select(Id(2));
					}
					2 => state.logout(),
					_ => apply(&mut state, Event::Unavailable(Id(1))),
				}
				assert!(state.search_target.is_none());
				state.apply(Envelope {
					generation,
					event: Event::History {
						channel: Id(1),
						request,
						older: true,
						messages: vec![message(50)],
					},
				});
				assert!(state.timeline.get(Id(50)).is_none());
			}
		}
	}

	#[test]
	fn reply_targets_survive_a_round_trip_independently_per_conversation() {
		let mut state = state();
		state.reply = Some(Reply {
			target: Id(100),
			mention: false,
		});
		state.select(Id(2));
		assert_eq!(state.selected, Some(Id(2)));
		assert_eq!(state.reply, None);
		state.reply = Some(Reply::to(Id(200)));
		state.select(Id(1));
		assert_eq!(
			state.reply,
			Some(Reply {
				target: Id(100),
				mention: false,
			})
		);
		state.select(Id(3));
		assert_eq!(state.reply, None);
		state.select(Id(2));
		assert_eq!(state.reply, Some(Reply::to(Id(200))));
		// Home keeps the reply parked too; only the restored conversation sees it.
		state.open_home();
		assert_eq!(state.reply, None);
		state.select(Id(1));
		assert_eq!(state.reply_target(), Some(Id(100)));
		// Cancelling in the composer removes it for good.
		state.reply = None;
		state.select(Id(2));
		state.select(Id(1));
		assert_eq!(state.reply, None);
		assert_eq!(state.parked_replies.len(), 1);
	}

	#[test]
	fn returning_at_capacity_restores_the_oldest_parked_reply() {
		let mut state = state();
		state
			.parked_replies
			.park(state.generation, Id(2), Reply::to(Id(200)));
		for channel in 1..ParkedReplies::MAX as u64 {
			state
				.parked_replies
				.park(state.generation, Id(1000 + channel), Reply::to(Id(5)));
		}
		state.reply = Some(Reply::to(Id(100)));
		state.select(Id(2));
		assert_eq!(state.reply, Some(Reply::to(Id(200))));
		assert_eq!(state.parked_replies.len(), ParkedReplies::MAX);
		state.select(Id(1));
		assert_eq!(state.reply_target(), Some(Id(100)));
		// Reselecting the open conversation keeps its reply without parking it at capacity.
		state.select(Id(1));
		assert_eq!(state.reply_target(), Some(Id(100)));
		assert_eq!(state.parked_replies.len(), ParkedReplies::MAX);
		assert_eq!(
			state.parked_replies.take(state.generation, Id(1001)),
			Some(Reply::to(Id(5)))
		);
	}

	#[test]
	fn parked_replies_are_bounded_and_drop_deleted_or_unavailable_targets() {
		let mut state = state();
		state.reply = Some(Reply::to(Id(100)));
		state.select(Id(2));
		state.reply = Some(Reply::to(Id(200)));
		state.select(Id(3));
		apply(
			&mut state,
			Event::Delete {
				channel: Id(1),
				id: Id(100),
			},
		);
		apply(
			&mut state,
			Event::DeleteBulk {
				channel: Id(2),
				ids: vec![Id(200)],
			},
		);
		assert_eq!(state.parked_replies.len(), 0);
		state.select(Id(1));
		assert_eq!(state.reply, None);

		// A conversation removed from navigation forgets its parked reply.
		state.reply = Some(Reply::to(Id(100)));
		state.select(Id(2));
		state.remove_channels(&[Id(1)].into_iter().collect());
		assert_eq!(state.parked_replies.len(), 0);

		// Restoring requires the conversation to still accept messages.
		let mut forum = chat_link_state();
		forum.reply = Some(Reply::to(Id(100)));
		forum.select(Id(2));
		assert_eq!(forum.parked_replies.len(), 1);
		forum.channels[0].kind = 15;
		forum.invalidate_navigation();
		forum.select(Id(1));
		assert_eq!(forum.selected, Some(Id(1)));
		assert_eq!(forum.reply, None);
		assert_eq!(forum.parked_replies.len(), 0);

		let mut parked = ParkedReplies::default();
		for id in 1..=100 {
			parked.park(1, Id(id), Reply::to(Id(1000 + id)));
		}
		parked.park(1, Id(100), Reply::to(Id(7)));
		assert_eq!(parked.len(), ParkedReplies::MAX);
		assert_eq!(parked.take(1, Id(36)), None);
		assert_eq!(parked.take(1, Id(37)), Some(Reply::to(Id(1037))));
		assert_eq!(parked.take(1, Id(100)), Some(Reply::to(Id(7))));
		// Another session generation never receives this one's reply.
		assert_eq!(parked.take(2, Id(38)), None);
		assert_eq!(parked.len(), ParkedReplies::MAX - 3);

		// A session change bumps the generation; logout also starts with nothing parked.
		assert_eq!(state.selected, Some(Id(2)));
		state.reply = Some(Reply::to(Id(300)));
		state.select(Id(3));
		state.reply = Some(Reply::to(Id(301)));
		state.generation += 1;
		state.select(Id(2));
		assert_eq!(state.reply, None);
		state.select(Id(3));
		assert_eq!(state.reply_target(), Some(Id(301)));
		state.select(Id(2));
		state.logout();
		assert_eq!(state.parked_replies.len(), 0);
	}

	#[test]
	fn explicit_reference_deletion_retires_composer_and_loaded_target() {
		{
			for history in [false, true] {
				let mut state = state();
				state.timeline.insert(message(50), false, false).unwrap();
				state.reply = Some(Reply::to(Id(50)));
				let mut source = message(101);
				source.reply_to = Some(Id(50));
				source.kind = 19;
				source.reply_deleted = true;
				if history {
					state.history(None);
					let request = state.request;
					apply(
						&mut state,
						Event::History {
							channel: Id(1),
							request,
							older: false,
							messages: vec![message(50), source],
						},
					);
				} else {
					apply(&mut state, Event::Message(source));
				}
				assert!(state.timeline.get(Id(50)).is_none());
				assert!(state.timeline.is_deleted(Id(50)));
				assert!(state.reply.is_none());
				assert!(!state.can_open_reply_target(Id(50)));
				assert!(state.open_reply_target(Id(50)).is_none());
			}
		}
		{
			let mut state = state();
			for channel in [2, 3, 1] {
				state.select(Id(channel)).unwrap();
				let request = state.request;
				let mut loaded = message(100);
				loaded.channel = Id(channel);
				loaded.reply_to = Some(Id(50));
				apply(
					&mut state,
					Event::History {
						channel: Id(channel),
						request,
						older: false,
						messages: vec![loaded],
					},
				);
			}
			assert_eq!(state.resident_window_count(), 2);
			state.open_reply_target(Id(50)).unwrap();
			let request = state.request;
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: false,
					messages: vec![message(50)],
				},
			);
			assert_eq!(state.search_target.take(), Some(Id(50))); // The UI consumes only the cue.
			assert!(state.history_targeted);
			assert!(state.resident_history_rows() <= 1475);
			assert!(state.resident_history_bytes() <= 16 * 1024 * 1024 - 66 * 1024);
			assert!(state.select(Id(2)).is_none(), "untouched window");
			assert!(!state.history_targeted);
			state.select(Id(1)).unwrap();
			assert_eq!(state.timeline.row_count(), 0);
			assert!(state.search_target.is_none());
		}
	}

	#[test]
	fn search_and_pin_target_windows_preserve_prior_deletions() {
		for pins in [false, true] {
			let mut state = state();
			state.timeline.delete(Id(40)).unwrap();
			state.search = Some(crate::search::SearchView {
				pins,
				channel: Id(1),
				guild: None,
				query: "Synthetic".into(),
				before: None,
				oldest: false,
				offset: 0,
				total: None,
				pin_before: None,
				request: 1,
				loading: false,
				error: None,
				changed: false,
				mutations: Vec::new(),
				page: Some(model::SearchPage {
					hits: vec![model::SearchHit {
						id: Id(50),
						channel: Id(1),
						author: model::User {
							member_avatar: None,
							username: None,
							kind: model::AccountKind::Human,
							webhook: false,
							id: Id(7),
							name: "Synthetic".into(),
							avatar: None,
							discriminator: 0,
							primary_guild: None,
							cosmetics: None,
						},
						mentions: vec![],
						excerpt: "Synthetic".into(),
						attachments: vec![],
						embeds: vec![],
					}],
					total: 1,
					partial: false,
					pin_cursor: None,
				}),
			});
			let command = state.open_search_hit(Id(50)).unwrap();
			let Command::HistoryAround { request, .. } = command else {
				panic!()
			};
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: false,
					messages: vec![message(50), message(40)],
				},
			);
			assert!(state.timeline.get(Id(50)).is_some());
			assert!(state.timeline.get(Id(40)).is_none());
			assert!(state.timeline.is_deleted(Id(40)));
		}
	}
}
