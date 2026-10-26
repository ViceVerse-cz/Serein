//! Dormant, account-session-local conversation windows. Entries are moved, never cloned.
//! Live events evict a window, so one that survives untouched still holds the latest page and
//! reopens without a request; anything older or changed revalidates. Exact loaded message links
//! may use an eligible, previously fresh window without another page.
use crate::{Event, State, reactions};
use model::{Channel, Freshness, Id, Message};
use session_cache::Timeline;
use std::{
	collections::VecDeque,
	time::{Duration, Instant},
};

const MAX_WINDOWS: usize = 8; // The shared row/byte budget below evicts sooner for long windows.
/// Hover preloads may hold only a few windows, so a sweep cannot push out visited conversations.
const MAX_SPECULATIVE: usize = 3;
/// ponytail: trusting an untouched window assumes the gateway delivered every edit, deletion and
/// reaction for the channel since it was parked (unverified for very large guilds). The age cap
/// bounds that risk; a per-channel server revision would allow longer trust.
const TRUST_AGE: Duration = Duration::from_secs(5 * 60);
const MAX_ROWS: usize = 1500 - 25; // Reserve the one bounded search/pins page.
const MAX_BYTES: usize = 16 * 1024 * 1024 - 66 * 1024; // Page plus query/view metadata.

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
	id: Id,
	guild: Option<Id>,
	parent: Option<Id>,
	kind: u8,
}

impl From<&Channel> for Identity {
	fn from(channel: &Channel) -> Self {
		Self {
			id: channel.id,
			guild: channel.guild,
			parent: channel.parent_id,
			kind: channel.kind,
		}
	}
}
struct Window {
	identity: Identity,
	timeline: Timeline,
	parked: Instant,
	older_exhausted: bool,
	/// Parked by a hover preload and never opened; evicted before visited windows.
	speculative: bool,
	/// For a preloaded page, the channel's latest known message when it was parked. The page is
	/// the service's latest, so it stays current while that is unchanged, even when the
	/// latest known message was deleted and the page ends earlier.
	validated: Option<Id>,
}
/// A restored window that may be shown as the current page without revalidation.
pub(crate) struct Trusted {
	pub older_exhausted: bool,
}
/// Opaque resident state, public so external synthetic State struct updates remain valid.
#[derive(Default)]
pub struct Windows {
	entries: VecDeque<Window>,
}
impl Windows {
	pub(crate) fn remove(&mut self, channel: Id) {
		self.entries.retain(|entry| entry.identity.id != channel);
	}
	/// Removes the oldest preloaded window, or the oldest window when none was preloaded.
	fn evict(&mut self, speculative_only: bool) -> Option<Window> {
		let index = self
			.entries
			.iter()
			.position(|entry| entry.speculative)
			.or_else(|| (!speculative_only && !self.entries.is_empty()).then_some(0))?;
		self.entries.remove(index)
	}
}
impl State {
	/// Session-only extension policy. Disabling releases every retained deleted payload.
	pub fn set_preserve_deleted_messages(&mut self, enabled: bool) {
		if self.preserve_deleted_messages != enabled {
			self.preserve_deleted_messages = enabled;
			self.revision += 1;
		}
		self.timeline.set_preserve_deleted_messages(enabled);
		for entry in &mut self.resident.entries {
			entry.timeline.set_preserve_deleted_messages(enabled);
		}
	}

	/// Remove one session-retained deleted row from the active timeline.
	pub fn discard_preserved_deleted(&mut self, id: Id) {
		if self.timeline.discard_preserved(id) {
			self.revision += 1;
		}
	}

	/// Drop dormant previews when disk history is cleared; preserve the visible conversation.
	pub fn clear_cached_history(&mut self) {
		self.resident = Windows::default();
	}
	/// Number of dormant windows, excluding the active timeline.
	pub fn resident_window_count(&self) -> usize {
		self.resident.entries.len()
	}
	/// Inspect an eligible dormant window without changing the active conversation.
	pub(crate) fn resident_timeline(&self, channel: Id) -> Option<&Timeline> {
		let current = self
			.channels
			.iter()
			.find(|entry| entry.id == channel && entry.supports_text())?;
		if self.archived_thread == Some(channel) || !self.can_read_history(channel) {
			return None;
		}
		self.resident
			.entries
			.iter()
			.find(|entry| entry.identity == Identity::from(current))
			.map(|entry| &entry.timeline)
	}
	/// Active plus dormant reading rows, including deletion placeholders.
	pub fn resident_history_rows(&self) -> usize {
		self.timeline.row_count()
			+ self
				.resident
				.entries
				.iter()
				.map(|entry| entry.timeline.row_count())
				.sum::<usize>()
	}
	/// Conservative active plus dormant allocation charge (not process RSS).
	pub fn resident_history_bytes(&self) -> usize {
		self.timeline.retained_bytes()
			+ size_of::<Windows>()
			+ self.resident.entries.capacity() * size_of::<Window>()
			+ self
				.resident
				.entries
				.iter()
				.map(|entry| {
					entry
						.timeline
						.retained_bytes()
						.saturating_sub(size_of::<Timeline>())
				})
				.sum::<usize>()
	}
	/// Also call after direct disk hydration of the public active timeline.
	pub fn enforce_resident_budget(&mut self) {
		if self.timeline.older_evicted() {
			self.older_exhausted = false;
		}
		while !self.resident.entries.is_empty()
			&& (self.resident_history_rows() > MAX_ROWS
				|| self.resident_history_bytes() > MAX_BYTES)
		{
			self.evict_resident(false);
		}
	}
	/// Evicts one window, preloaded ones first. An evicted preload may be loaded again later.
	fn evict_resident(&mut self, speculative_only: bool) {
		if let Some(window) = self.resident.evict(speculative_only)
			&& window.speculative
		{
			self.preload.conversations.forget(window.identity.id);
		}
	}
	pub(crate) fn prune_resident(&mut self) {
		let mut entries = std::mem::take(&mut self.resident.entries);
		entries.retain(|entry| {
			self.archived_thread != Some(entry.identity.id)
				&& self.can_read_history(entry.identity.id)
				&& self.channels.iter().any(|channel| {
					channel.supports_text() && Identity::from(channel) == entry.identity
				})
		});
		self.resident.entries = entries;
	}
	/// A window that may be shown as the latest page: recent, untouched since it was fresh,
	/// and ending at the channel's latest known message.
	fn current(&self, entry: &Window) -> bool {
		let latest = self
			.channel(entry.identity.id)
			.and_then(|channel| channel.last_message);
		self.gateway_connected
			&& entry.parked.elapsed() < TRUST_AGE
			&& latest.is_some()
			&& (entry.timeline.iter().next_back().map(|message| message.id) == latest
				|| entry.validated == latest)
	}
	/// Whether `channel` already has a dormant window that would reopen without a request.
	pub(crate) fn resident_current(&self, channel: Id) -> bool {
		self.resident
			.entries
			.iter()
			.any(|entry| entry.identity.id == channel && self.current(entry))
	}
	/// Keeps a preloaded recent page as `channel`'s dormant window, replacing an older one.
	/// The page is speculative, so anything unexpected drops it rather than failing the session.
	pub(crate) fn park_preloaded(&mut self, channel: Id, mut messages: Vec<Message>) {
		let Some(identity) = self
			.channels
			.iter()
			.find(|entry| entry.id == channel && entry.supports_text())
			.map(Identity::from)
		else {
			return;
		};
		messages.retain(|message| !message.ephemeral);
		let mut ids = std::collections::BTreeSet::new();
		if self.selected == Some(channel)
			|| self.archived_thread == Some(channel)
			|| !self.can_read_history(channel)
			|| messages.len() > 50
			|| messages
				.iter()
				.any(|message| message.channel != channel || !ids.insert(message.id))
		{
			return;
		}
		let Some(latest) = ids.last().copied() else {
			return;
		};
		self.observe_last_message(channel, latest);
		let older_exhausted = messages.len() < 50;
		let mut timeline = Timeline::default();
		timeline.set_preserve_deleted_messages(self.preserve_deleted_messages);
		timeline.begin_page(false);
		if timeline.finish_page(messages, false).is_err() {
			return;
		}
		self.resident.remove(channel);
		if self
			.resident
			.entries
			.iter()
			.filter(|entry| entry.speculative)
			.count() >= MAX_SPECULATIVE
		{
			self.evict_resident(true);
		}
		if self.resident.entries.len() == MAX_WINDOWS {
			self.evict_resident(false);
		}
		self.resident.entries.push_back(Window {
			identity,
			timeline,
			parked: Instant::now(),
			older_exhausted,
			speculative: true,
			validated: self.channel(channel).and_then(|entry| entry.last_message),
		});
		self.enforce_resident_budget();
	}
	/// Swap the active timeline for `channel`'s dormant window. Returns `Some` when the restored
	/// window is recent, untouched and ends at the channel's latest known message.
	pub(crate) fn select_resident(&mut self, channel: Id) -> Option<Trusted> {
		self.prune_resident();
		// Take the destination first, before parking can evict the oldest entry.
		let restored = self
			.resident
			.entries
			.iter()
			.position(|entry| entry.identity.id == channel)
			.and_then(|index| self.resident.entries.remove(index));
		let trusted = restored
			.as_ref()
			.filter(|entry| self.current(entry))
			.map(|entry| Trusted {
				older_exhausted: entry.older_exhausted,
			});
		let previous = self.selected.and_then(|id| {
			self.channels
				.iter()
				.find(|entry| entry.id == id)
				.map(Identity::from)
		});
		let eligible = previous.is_some_and(|identity| {
			self.search_target.is_none()
				&& !self.history_targeted
				&& self.archived_thread != Some(identity.id)
				&& self.freshness == Freshness::Fresh
				&& self.can_read_history(identity.id)
				&& self
					.channels
					.iter()
					.any(|entry| entry.id == identity.id && entry.supports_text())
				&& self.timeline.row_count() != 0
		});
		let mut timeline = std::mem::take(&mut self.timeline);
		timeline.cancel_page();
		if let Some(identity) = previous.filter(|_| eligible) {
			if identity.id == channel {
				self.timeline = timeline;
			} else {
				self.resident.remove(identity.id);
				if self.resident.entries.len() == MAX_WINDOWS {
					self.evict_resident(false);
				}
				let older_exhausted = self.older_exhausted && !timeline.older_evicted();
				self.resident.entries.push_back(Window {
					identity,
					timeline,
					parked: Instant::now(),
					older_exhausted,
					speculative: false,
					validated: None,
				});
			}
		}
		if let Some(entry) = restored {
			self.timeline = entry.timeline;
		}
		self.timeline
			.set_preserve_deleted_messages(self.preserve_deleted_messages);
		self.enforce_resident_budget();
		trusted
	}
	pub(crate) fn invalidate_resident_event(&mut self, event: &Event) {
		let deletion = match event {
			Event::Delete { channel, id } => Some((*channel, std::slice::from_ref(id))),
			Event::DeleteBulk { channel, ids } if ids.len() <= 100 => {
				Some((*channel, ids.as_slice()))
			}
			_ => None,
		};
		if let Some((channel, ids)) = deletion {
			self.preload.conversations.touch(Some(channel));
			if let Some(window) = self
				.resident
				.entries
				.iter_mut()
				.find(|w| w.identity.id == channel)
				&& ids
					.iter()
					.try_for_each(|id| window.timeline.delete(*id))
					.is_err()
			{
				self.resident.remove(channel);
			}
			return;
		}
		let channel = match event {
			Event::Message(message)
			| Event::SendResult {
				result: Ok(message),
				..
			} => Some(message.channel),
			Event::Patch(patch) => Some(patch.channel),
			Event::Edited { channel, .. }
			| Event::Delete { channel, .. }
			| Event::DeleteBulk { channel, .. }
			| Event::Reactions(
				reactions::Event::Changed { channel, .. }
				| reactions::Event::Delta { channel, .. }
				| reactions::Event::Cleared { channel, .. }
				| reactions::Event::Written { channel, .. },
			) => Some(*channel),
			Event::RecipientRemoved { channel, user }
				if self.user.as_ref().is_some_and(|own| own.id == *user) =>
			{
				Some(*channel)
			}
			Event::Ready { .. } | Event::Resync | Event::PermissionsChanged => {
				self.clear_cached_history();
				self.preload.conversations.touch(None);
				None
			}
			_ => None,
		};
		if let Some(channel) = channel {
			self.preload.conversations.touch(Some(channel));
			if !matches!(event, Event::RecipientRemoved { .. })
				&& let Some(window) = self
					.resident
					.entries
					.iter_mut()
					.find(|w| w.identity.id == channel)
			{
				window.timeline.drop_live_history();
				if window.timeline.display_iter().next().is_some() {
					return;
				}
			}
			self.resident.remove(channel);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Command, Envelope, Reply, auth};
	use model::{Message, MessagePatch, Patch, User};

	fn message(channel: u64, id: u64) -> Message {
		Message {
			poll: None,
			sticker_items: Vec::new(),
			id: Id(id),
			channel: Id(channel),
			kind: 0,
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
			content: "Resident synthetic content".into(),
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
		State {
			user: Some(message(1, 1).author),
			auth: auth::AuthState::Authenticated,
			gateway_connected: true,
			channels: (1..=MAX_WINDOWS as u64 + 2)
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
		}
	}
	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
	fn load(state: &mut State, channel: u64) {
		if state.selected == Some(Id(channel)) {
			assert!(state.history_pending && state.history_before.is_none());
		} else if let Some(command) = state.select(Id(channel)) {
			assert!(matches!(command, Command::History { before: None, .. }));
		} else {
			// An untouched dormant window holding the latest page reopens without a request.
			assert!(!state.history_pending);
			assert_eq!(state.freshness, Freshness::Fresh);
			return;
		}
		let request = state.request;
		apply(
			state,
			Event::History {
				channel: Id(channel),
				request,
				older: false,
				messages: (1..=50)
					.map(|id| message(channel, channel * 1000 + id))
					.collect(),
			},
		);
		assert_eq!(state.freshness, Freshness::Fresh);
	}
	fn patch(channel: u64) -> MessagePatch {
		MessagePatch {
			poll: model::Patch::Absent,
			sticker_items: model::Patch::Absent,
			channel: Id(channel),
			id: Id(channel * 1000 + 1),
			content: Patch::Value("Updated".into()),
			components: model::Patch::Absent,
			flags: model::Patch::Absent,
			application_id: model::Patch::Absent,
			extra_content: Default::default(),
			reactions: Patch::Absent,
			mentions: Patch::Absent,
			edited: Patch::Absent,
			embeds: Patch::Absent,
			attachments: Patch::Absent,
			embeds_suppressed: Patch::Absent,
		}
	}

	#[test]
	fn live_eviction_reopens_older_history_after_a_trusted_return() {
		for byte_limit in [false, true] {
			let mut state = state();
			state.select(Id(1)).unwrap();
			let request = state.request;
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: false,
					messages: vec![message(1, 1)],
				},
			);
			assert!(state.older_exhausted);
			assert!(!state.can_load_older());
			let count = if byte_limit { 100 } else { 501 };
			for id in 2..=count {
				let mut row = message(1, id);
				if byte_limit {
					row.content = "x".repeat(64 * 1024);
				}
				apply(&mut state, Event::Message(row));
			}
			assert!(state.timeline.get(Id(1)).is_none());
			assert!(!state.older_exhausted);
			assert!(state.can_load_older(), "capacity eviction loses the start");
			load(&mut state, 2);
			assert!(
				state.select(Id(1)).is_none(),
				"trusted return avoids revalidation"
			);
			assert!(state.can_load_older());
			let before = state.timeline.row_ids().next().unwrap();
			let Some(Command::History {
				before: Some(cursor),
				request,
				..
			}) = state.older_history()
			else {
				panic!()
			};
			assert_eq!(cursor, before);
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: true,
					messages: vec![message(1, 1)],
				},
			);
			assert!(state.timeline.get(Id(1)).is_some());
			assert!(
				!state.can_load_older(),
				"the short older page establishes the start again"
			);
		}
	}

	#[test]
	fn reselecting_current_channel_preserves_history_request_and_composer() {
		{
			let mut state = state();
			load(&mut state, 1);
			state.drafts.insert(Id(1), "Unsent draft".into());
			state.reply = Some(Reply::to(Id(1001)));
			let content = state.timeline.get(Id(1001)).unwrap().content.as_ptr();
			for loading in [false, true] {
				if loading {
					state.history(Some(Id(1001)));
				}
				state.search_target = Some(Id(1001));
				let before = (state.request, state.revision, state.freshness);
				assert!(state.select(Id(1)).is_none());
				assert_eq!((state.request, state.revision, state.freshness), before);
				assert_eq!(state.history_pending, loading);
				assert_eq!(state.search_target, Some(Id(1001)));
				assert_eq!(state.reply_target(), Some(Id(1001)));
				assert_eq!(state.drafts[&Id(1)], "Unsent draft");
				assert_eq!(
					state.timeline.get(Id(1001)).unwrap().content.as_ptr(),
					content
				);
				assert_eq!(state.timeline.row_count(), 50);
			}
		}
		{
			let mut state = state();
			load(&mut state, 1);
			let content = state.timeline.get(Id(1001)).unwrap().content.as_ptr();
			let old_request = state.request;
			state.drafts.insert(Id(1), "Unsent draft".into());
			load(&mut state, 2);
			load(&mut state, 3);
			assert_eq!(state.resident_window_count(), 2);
			assert_eq!(state.resident_history_rows(), 150);
			// Newer activity than the dormant page (e.g. a read-state update) revalidates it.
			state.observe_last_message(Id(1), Id(1999));
			assert!(matches!(
				state.select(Id(1)),
				Some(Command::History {
					channel: Id(1),
					before: None,
					..
				})
			));
			assert_eq!(
				state.timeline.get(Id(1001)).unwrap().content.as_ptr(),
				content
			);
			assert_eq!(state.timeline.row_count(), 50);
			assert!(
				state
					.timeline
					.iter()
					.all(|message| message.channel == Id(1))
			);
			assert_eq!(state.freshness, Freshness::Loading);
			assert!(!state.can_load_older());
			assert_eq!(state.drafts[&Id(1)], "Unsent draft");
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request: old_request,
					older: false,
					messages: vec![],
				},
			);
			assert_eq!(state.timeline.row_count(), 50);
			let request = state.request;
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: false,
					messages: vec![message(1, 2000)],
				},
			);
			assert_eq!(state.timeline.row_ids().collect::<Vec<_>>(), vec![Id(2000)]);
			assert_eq!(state.freshness, Freshness::Fresh);
		}
	}

	#[test]
	fn deletion_only_windows_survive_promotion_and_reject_old_page_bodies() {
		{
			let mut state = state();
			load(&mut state, 1);
			apply(
				&mut state,
				Event::DeleteBulk {
					channel: Id(1),
					ids: (1001..=1050).map(Id).collect(),
				},
			);
			assert!(state.timeline.is_empty());
			load(&mut state, 2);
			state.select(Id(1)).unwrap();
			assert_eq!(state.timeline.row_count(), 50);
			assert!(state.timeline.is_empty());
			let request = state.request;
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: false,
					messages: vec![message(1, 1001)],
				},
			);
			assert!(state.timeline.get(Id(1001)).is_none());
		}
		{
			for event in [Event::Message(message(1, 1099)), Event::Patch(patch(1))] {
				let mut state = state();
				state.set_preserve_deleted_messages(true);
				load(&mut state, 1);
				load(&mut state, 2);
				apply(
					&mut state,
					Event::Delete {
						channel: Id(1),
						id: Id(1001),
					},
				);
				apply(&mut state, event);
				state.select(Id(1)).unwrap();
				assert!(state.timeline.get_display(Id(1001)).is_some());
				assert!(state.timeline.get(Id(1001)).is_none());
				assert_eq!(state.timeline.row_count(), 1);
				let request = state.request;
				apply(
					&mut state,
					Event::History {
						channel: Id(1),
						request,
						older: false,
						messages: vec![message(1, 1099)],
					},
				);
				assert!(state.timeline.get_display(Id(1001)).is_some());
				state.discard_preserved_deleted(Id(1001));
				assert!(state.timeline.get_display(Id(1001)).is_none());
				assert!(state.timeline.is_deleted(Id(1001)));
			}
		}
	}

	#[test]
	fn lru_and_aggregate_row_budget_evict_whole_windows() {
		{
			let mut state = state();
			for channel in 1..=MAX_WINDOWS as u64 + 2 {
				load(&mut state, channel);
			}
			assert_eq!(state.resident_window_count(), MAX_WINDOWS);
			state.select(Id(1)).unwrap();
			assert_eq!(state.timeline.row_count(), 0); // Least recent was evicted.
			for channel in 1..=3 {
				load(&mut state, channel);
				state
					.timeline
					.seed_cache(
						(1..=500)
							.map(|id| message(channel, channel * 10000 + id))
							.collect(),
					)
					.unwrap();
				state.enforce_resident_budget();
				assert!(state.resident_history_rows() <= MAX_ROWS);
				assert!(state.resident_history_bytes() <= MAX_BYTES);
			}
			assert_eq!(state.timeline.row_count(), 500);
			assert_eq!(state.resident_window_count(), 1);
			assert_eq!(state.resident_history_rows(), 1000);
		}
		{
			let mut state = state();
			for channel in 1..=3 {
				load(&mut state, channel);
				for id in 1..=500 {
					let mut item = message(channel, channel * 10000 + id);
					item.content = "x".repeat(64 * 1024);
					state.timeline.insert(item, false, false).unwrap();
				}
				state.enforce_resident_budget();
				assert!(state.resident_history_bytes() <= MAX_BYTES);
				assert!(state.resident_history_rows() <= MAX_ROWS);
			}
			let request = state.history(None);
			assert!(matches!(request, Command::History { .. }));
			for id in 1..=12 {
				let mut update = patch(3);
				update.id = Id(90000 + id);
				update.content = Patch::Value("p".repeat(64 * 1024));
				apply(&mut state, Event::Patch(update));
				assert!(state.resident_history_bytes() <= MAX_BYTES);
			}
		}
	}

	#[test]
	fn inactive_mutations_evict_only_the_affected_window_and_ignore_old_generations() {
		{
			for event in [
				Event::Message(message(1, 1099)),
				Event::Patch(patch(1)),
				Event::Delete {
					channel: Id(1),
					id: Id(1001),
				},
				Event::DeleteBulk {
					channel: Id(1),
					ids: vec![Id(1001)],
				},
				Event::Reactions(reactions::Event::Changed {
					channel: Id(1),
					message: Id(1001),
				}),
				Event::Reactions(reactions::Event::Delta {
					channel: Id(1),
					message: Id(1001),
					user: Id(2),
					emoji: model::ReactionEmoji {
						id: None,
						name: Some("x".into()),
					},
					add: true,
					burst: false,
				}),
				Event::Reactions(reactions::Event::Cleared {
					channel: Id(1),
					message: Id(1001),
					emoji: None,
				}),
				Event::SendResult {
					nonce: "synthetic".into(),
					result: Ok(message(1, 1099)),
				},
			] {
				let mut state = state();
				load(&mut state, 1);
				load(&mut state, 2);
				load(&mut state, 3);
				state.apply(Envelope {
					generation: state.generation - 1,
					event: Event::Delete {
						channel: Id(2),
						id: Id(2001),
					},
				});
				let delete = matches!(&event, Event::Delete { .. } | Event::DeleteBulk { .. });
				apply(&mut state, event);
				if delete {
					assert_eq!(state.resident_window_count(), 2);
					// Deletions apply in place, so the window still ends at the latest message.
					assert!(state.select(Id(1)).is_none());
					assert_eq!(state.freshness, Freshness::Fresh);
					assert!(state.timeline.get_display(Id(1001)).is_none());
					assert!(state.timeline.get(Id(1001)).is_none());
					assert_eq!(state.timeline.row_count(), 50);
				} else {
					assert_eq!(state.resident_window_count(), 1);
					assert!(state.select(Id(2)).is_none(), "untouched window");
					assert_eq!(state.timeline.row_count(), 50);
					state.select(Id(1)).unwrap();
					assert_eq!(state.timeline.row_count(), 0);
				}
			}
		}
		{
			for event in [
				Event::ChannelChanged(model::ChannelPatch {
					icon: model::Patch::Absent,
					id: Id(1),
					parent_id: Patch::Value(Id(99)),
					name: Patch::Absent,
					kind: Patch::Absent,
					message_count: Patch::Absent,
					tags: Patch::Absent,
					position: Patch::Absent,
					last_message: Patch::Absent,
				}),
				Event::Unavailable(Id(1)),
				Event::Resync,
				Event::PermissionsChanged,
				Event::Failure(auth::Failure::Expired),
				Event::RecipientRemoved {
					channel: Id(1),
					user: Id(9),
				},
			] {
				let mut state = state();
				load(&mut state, 1);
				load(&mut state, 2);
				apply(&mut state, event);
				assert_eq!(state.resident_window_count(), 0);
			}
			let mut state = state();
			load(&mut state, 1);
			load(&mut state, 2);
			state.clear_cached_history();
			assert_eq!(state.resident_window_count(), 0);
			assert_eq!(state.timeline.row_count(), 50);
			load(&mut state, 1);
			state.logout();
			assert_eq!(state.resident_history_rows(), 0);
		}
	}

	#[test]
	fn revoked_dormant_guild_history_is_pruned_while_selected_dm_stays_visible() {
		use model::permissions as p;
		let mut state = state();
		state.guilds.push(model::Guild {
			default_message_notifications: None,
			stickers: None,
			id: Id(10),
			name: "Synthetic guild".into(),
			icon: None,
			emojis: None,
		});
		state.channels[0].guild = Some(Id(10));
		state.channels[0].kind = 0;
		state
			.permissions
			.replace(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(10),
					owner: Some(Id(8)),
					roles: Some(vec![p::Role {
						name: String::new(),
						color: 0,
						secondary_color: None,
						tertiary_color: None,
						position: 0,
						hoist: false,
						managed: false,
						id: Id(10),
						bits: p::VIEW_CHANNEL | p::READ_MESSAGE_HISTORY,
					}]),
					member: Some(p::Member {
						roles: vec![],
						timeout_until: None,
					}),
				}],
				channels: vec![p::Channel {
					id: Id(1),
					guild: Id(10),
					overwrites: Some(vec![]),
				}],
			})
			.unwrap();
		load(&mut state, 1);
		load(&mut state, 2);
		apply(
			&mut state,
			Event::Permissions(crate::permissions::Event::Channel {
				channel: Id(1),
				guild: Some(Id(10)),
				overwrites: Patch::Null,
			}),
		);
		assert_eq!(state.resident_window_count(), 0);
		assert_eq!(state.timeline.row_count(), 50);
		assert_eq!(state.freshness, Freshness::Fresh);
		assert!(!state.can_read_history(Id(1)));
	}

	#[test]
	fn untouched_recent_windows_reopen_without_a_request() {
		{
			let mut state = state();
			load(&mut state, 1);
			state.older_exhausted = true; // A short conversation already reached its start.
			load(&mut state, 2);
			let request = state.request;
			assert!(state.select(Id(1)).is_none());
			assert_eq!(state.freshness, Freshness::Fresh);
			assert!(!state.history_pending && state.older_exhausted);
			assert_eq!(state.timeline.row_count(), 50);
			assert_ne!(
				state.request, request,
				"an in-flight page for the old channel is stale"
			);
		}
		for revalidate in [
			|state: &mut State| {
				let entry = state.resident.entries.back_mut().unwrap();
				entry.parked = Instant::now().checked_sub(TRUST_AGE).unwrap();
			},
			|state: &mut State| state.gateway_connected = false,
			|state: &mut State| state.observe_last_message(Id(2), Id(2999)),
		] {
			let mut state = state();
			load(&mut state, 2);
			load(&mut state, 1);
			revalidate(&mut state);
			assert!(matches!(
				state.select(Id(2)),
				Some(Command::History {
					channel: Id(2),
					before: None,
					..
				})
			));
			assert_eq!(state.timeline.row_count(), 50, "the preview stays visible");
			assert_eq!(state.freshness, Freshness::Loading);
		}
	}

	#[test]
	fn search_targets_and_transient_archive_windows_are_not_parked() {
		let mut state = state();
		load(&mut state, 1);
		state.search_target = Some(Id(1001));
		load(&mut state, 2);
		assert_eq!(state.resident_window_count(), 0);
		load(&mut state, 1);
		// The archive UI admits a temporary navigation row before selecting it.
		state.archived_thread = Some(Id(2));
		load(&mut state, 2);
		assert_eq!(state.resident_window_count(), 1);
		assert!(state.select(Id(1)).is_none(), "untouched window");
		assert_eq!(state.timeline.row_count(), 50);
		assert_eq!(state.freshness, Freshness::Fresh);
		assert_eq!(state.resident_window_count(), 0);
		assert!(!state.channels.iter().any(|channel| channel.id == Id(2)));
	}
}
