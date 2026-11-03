//! The latest message of a thread, read once for the card under its starter message.
//!
//! The gateway's channel list carries a thread's name and message count but no message body, so
//! the card's last-message line needs one bounded read. Admission mirrors a hover preload: one
//! answer in flight, a per-minute cap, a small cache whose entries are trusted for a few minutes,
//! and a thread whose read failed is not retried until that trust would have expired. Nothing
//! here joins, subscribes, marks read or sends.
use crate::{Command, State, auth::AuthState, auth::Failure};
use model::{Channel, Id, Message};
use std::{
	collections::VecDeque,
	time::{Duration, Instant},
};

/// Cards a session keeps a preview for; the least recently read goes first.
const MAX_PREVIEWS: usize = 24;
/// One answer: the latest message plus, for a thread this session had not loaded, its channel.
const MAX_PREVIEW_BYTES: usize = 64 * 1024;
/// All previews together stay below this.
const MAX_BYTES: usize = 192 * 1024;
/// A cached preview is trusted this long, then refreshed when its card is on screen again.
const TRUST_AGE: Duration = Duration::from_secs(5 * 60);
/// Reads per minute, matching the hover-preload budget.
const PER_MINUTE: usize = 12;
/// Longer than the REST timeout: an in-flight marker this old lost its answer.
const LOST: Duration = Duration::from_secs(30);
/// Failed threads remembered without retrying.
const MAX_FAILED: usize = 64;

/// What one bounded read of a thread's card returns.
pub struct Loaded {
	/// The thread channel, when this session had not loaded it.
	pub channel: Option<Channel>,
	/// The thread's newest message; absent for a thread without messages.
	pub last: Option<Message>,
}
impl Loaded {
	pub(crate) fn bytes(&self) -> usize {
		self.channel.as_ref().map_or(0, Channel::bytes)
			+ self.last.as_ref().map_or(0, Message::bytes)
	}
	/// The answer describes `thread` within one preview's budget.
	fn valid(&self, thread: Id) -> bool {
		(self.channel.is_some() || self.last.is_some())
			&& self.bytes() <= MAX_PREVIEW_BYTES
			&& self.channel.as_ref().is_none_or(|channel| {
				channel.id == thread
					&& matches!(channel.kind, 10..=12)
					&& channel.parent_id.is_some()
					&& !channel.name.trim().is_empty()
			}) && self
			.last
			.as_ref()
			.is_none_or(|message| message.id.0 != 0 && message.channel == thread)
	}
}

/// The card-facing parts of a cached preview.
pub struct Snapshot<'a> {
	pub channel: Option<&'a Channel>,
	pub last: Option<&'a Message>,
}

struct Entry {
	thread: Id,
	channel: Option<Channel>,
	last: Option<Message>,
	/// The thread's latest known message when this preview was read, so a newer one refreshes it.
	latest: Option<Id>,
	loaded: Instant,
	/// The previewed message was edited after this answer, so the card must read it again.
	stale: bool,
}
impl Entry {
	fn bytes(&self) -> usize {
		self.channel.as_ref().map_or(0, Channel::bytes)
			+ self.last.as_ref().map_or(0, Message::bytes)
	}
}

/// Bounded session cache of thread-card previews and the one read that fills one.
#[derive(Default)]
pub struct Previews {
	entries: VecDeque<Entry>,
	pending: Option<(Id, u64, Instant)>,
	failed: VecDeque<(Id, Instant)>,
	sent: VecDeque<Instant>,
	sequence: u64,
}

impl Previews {
	fn entry(&self, thread: Id) -> Option<&Entry> {
		self.entries.iter().find(|entry| entry.thread == thread)
	}
	/// Drops failures and sent reads that no longer bound a new read.
	fn prune(&mut self, now: Instant) {
		while self
			.failed
			.front()
			.is_some_and(|(_, at)| now.saturating_duration_since(*at) >= TRUST_AGE)
		{
			self.failed.pop_front();
		}
		while self
			.sent
			.front()
			.is_some_and(|at| now.saturating_duration_since(*at) >= Duration::from_secs(60))
		{
			self.sent.pop_front();
		}
	}
	fn bytes(&self) -> usize {
		self.entries.iter().map(Entry::bytes).sum()
	}
	/// Whether `thread`'s card asks for a read: never read, expired, edited, or superseded.
	fn needs_read(&self, state: &State, thread: Id, now: Instant) -> bool {
		// A refused thread waits out its failure window. Its card must not keep the first
		// slot the timeline proposes each frame, which would starve the cards below it.
		if self
			.failed
			.iter()
			.any(|(failed, at)| *failed == thread && now.saturating_duration_since(*at) < TRUST_AGE)
		{
			return false;
		}
		let Some(entry) = self.entry(thread) else {
			return true;
		};
		if entry.stale || now.saturating_duration_since(entry.loaded) >= TRUST_AGE {
			return true;
		}
		let latest = state
			.channel(thread)
			.and_then(|channel| channel.last_message)
			.or_else(|| {
				entry
					.channel
					.as_ref()
					.and_then(|channel| channel.last_message)
			});
		latest.is_some_and(|latest| entry.latest != Some(latest))
	}
	fn store(&mut self, thread: Id, loaded: Loaded, latest: Option<Id>, now: Instant) {
		self.entries.retain(|entry| entry.thread != thread);
		self.entries.push_back(Entry {
			thread,
			channel: loaded.channel,
			last: loaded.last,
			latest,
			loaded: now,
			stale: false,
		});
		while self.entries.len() > MAX_PREVIEWS || self.bytes() > MAX_BYTES {
			if self.entries.pop_front().is_none() {
				break;
			}
		}
	}
	/// Marks a preview stale after its message was edited, so its card reads the new text.
	pub(crate) fn mark_stale(&mut self, channel: Id, message: Id) {
		for entry in &mut self.entries {
			if entry
				.last
				.as_ref()
				.is_some_and(|last| last.channel == channel && last.id == message)
			{
				entry.stale = true;
			}
		}
	}
	/// Drops previews of deleted messages; their cards read the thread again when shown.
	pub(crate) fn forget(&mut self, channel: Id, messages: &[Id]) {
		self.entries.retain(|entry| {
			entry
				.last
				.as_ref()
				.is_none_or(|last| last.channel != channel || !messages.contains(&last.id))
		});
	}
}

impl State {
	/// The cached preview of a thread's card, whether or not it still needs a refresh.
	pub fn thread_preview(&self, thread: Id) -> Option<Snapshot<'_>> {
		let entry = self.thread_previews.entry(thread)?;
		Some(Snapshot {
			channel: entry.channel.as_ref(),
			last: entry.last.as_ref(),
		})
	}
	/// Whether `thread`'s card lacks a trusted preview, so the timeline may propose its read.
	pub fn thread_preview_needs_read(&self, thread: Id) -> bool {
		self.thread_previews
			.needs_read(self, thread, Instant::now())
	}
	/// Proposes the card under `message`; admission sends at most one bounded read at a time.
	pub fn request_thread_preview(&mut self, message: Id) -> Option<Command> {
		let (parent, thread) = {
			let message = self.timeline.get_display(message)?;
			// Only a message the service flagged as a starter names a thread, and only a
			// server channel holds threads at all.
			if message.flags & model::HAS_THREAD == 0 {
				return None;
			}
			let parent = self.channel(message.channel)?;
			if !self.can_view(parent.id) || !matches!(parent.kind, 0 | 5 | 10..=12 | 15 | 16) {
				return None;
			}
			let thread = if self.unloaded_thread(message).is_some() {
				message.id
			} else {
				self.thread_of(message)?.id
			};
			(parent.id, thread)
		};
		let guild = self.channel(parent)?.guild?;
		if !self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected) {
			return None;
		}
		let now = Instant::now();
		if !self.thread_previews.needs_read(self, thread, now) {
			return None;
		}
		let needs_channel = self.channel(thread).is_none();
		self.admit_thread_preview(guild, thread, needs_channel, now)
	}
	/// The admission half of [`Self::request_thread_preview`], shared with its tests.
	fn admit_thread_preview(
		&mut self,
		guild: Id,
		thread: Id,
		needs_channel: bool,
		now: Instant,
	) -> Option<Command> {
		let previews = &mut self.thread_previews;
		if previews
			.pending
			.is_some_and(|(_, _, at)| now.saturating_duration_since(at) < LOST)
		{
			return None;
		}
		previews.pending = None;
		previews.prune(now);
		if previews.failed.iter().any(|(failed, _)| *failed == thread)
			|| previews.sent.len() >= PER_MINUTE
		{
			return None;
		}
		previews.sequence = previews.sequence.wrapping_add(1);
		let request = previews.sequence;
		previews.pending = Some((thread, request, now));
		previews.sent.push_back(now);
		Some(Command::ThreadPreview {
			guild,
			thread,
			needs_channel,
			request,
		})
	}
	pub(crate) fn apply_thread_preview(
		&mut self,
		thread: Id,
		request: u64,
		result: Result<Loaded, Failure>,
	) {
		if self
			.thread_previews
			.pending
			.is_none_or(|pending| pending.0 != thread || pending.1 != request)
		{
			return;
		}
		self.thread_previews.pending = None;
		let now = Instant::now();
		match result {
			Ok(loaded) if loaded.valid(thread) => {
				// The session's own channel wins over the read; only an unloaded thread keeps
				// the channel the answer carried.
				let channel = loaded.channel.filter(|_| self.channel(thread).is_none());
				let latest = channel
					.as_ref()
					.and_then(|channel| channel.last_message)
					.or_else(|| {
						self.channel(thread)
							.and_then(|channel| channel.last_message)
					});
				let loaded = Loaded {
					channel,
					last: loaded.last,
				};
				self.thread_previews.store(thread, loaded, latest, now);
				self.revision += 1;
			}
			Err(failure) if failure.ends_session() && failure != Failure::Capacity => {
				self.fail(failure);
			}
			_ => {
				if self.thread_previews.failed.len() >= MAX_FAILED {
					self.thread_previews.failed.pop_front();
				}
				self.thread_previews.failed.push_back((thread, now));
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::{Channel, Message, User};

	fn channel(id: Id, kind: u8, parent: Option<Id>) -> Channel {
		Channel {
			id,
			guild: Some(Id(1)),
			parent_id: parent,
			kind,
			position: 0,
			name: format!("Thread {}", id.0),
			recipients: vec![],
			last_message: None,
			member_list_id: None,
			tags: None,
			message_count: Some(2),
			icon: None,
		}
	}
	fn message(id: Id, channel: Id) -> Message {
		Message {
			poll: None,
			sticker_items: Vec::new(),
			flags: 0,
			ephemeral: false,
			components: vec![],
			application_id: None,
			reactions: None,
			id,
			channel,
			author: User {
				id: Id(9),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
			},
			author_roles: vec![],
			author_nick: None,
			content: "Synthetic reply".into(),
			mentions: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			forwarded: false,
			interaction: None,
			unsupported: false,
			extra_content: Default::default(),
			embeds: vec![],
			embeds_suppressed: false,
			attachments: vec![],
		}
	}
	fn answer(thread: Id, last: Id) -> Loaded {
		Loaded {
			channel: Some(channel(thread, 11, Some(Id(1)))),
			last: Some(message(last, thread)),
		}
	}
	fn submitted(command: Option<Command>) -> u64 {
		let Some(Command::ThreadPreview { request, .. }) = command else {
			panic!("the card asks for one bounded read");
		};
		request
	}

	#[test]
	fn an_answer_fills_only_the_thread_it_describes_and_failures_are_not_retried() {
		let mut state = State::default();
		let thread = Id(50);
		let request = submitted(state.admit_thread_preview(Id(1), thread, true, Instant::now()));
		state.apply_thread_preview(
			thread,
			request,
			Ok(Loaded {
				channel: None,
				last: Some(message(Id(81), Id(60))),
			}),
		);
		assert!(state.thread_preview(thread).is_none());
		assert!(
			!state.thread_preview_needs_read(thread),
			"a refused card leaves the first slot to the cards below it"
		);
		assert!(
			state
				.admit_thread_preview(Id(1), thread, true, Instant::now())
				.is_none(),
			"a wrong answer is remembered, not retried"
		);

		let other = Id(70);
		let request = submitted(state.admit_thread_preview(Id(1), other, true, Instant::now()));
		state.apply_thread_preview(other, request, Ok(answer(other, Id(71))));
		let preview = state
			.thread_preview(other)
			.expect("the answer fills the card");
		assert_eq!(preview.last.map(|message| message.id), Some(Id(71)));
		assert_eq!(
			preview.channel.map(|channel| channel.name.as_str()),
			Some("Thread 70")
		);
		assert!(!state.thread_preview_needs_read(other));

		// The cache keeps only a bounded number of cards.
		for id in 100..200 {
			state
				.thread_previews
				.store(Id(id), answer(Id(id), Id(id + 1)), None, Instant::now());
		}
		assert_eq!(state.thread_previews.entries.len(), MAX_PREVIEWS);
		assert!(state.thread_preview(other).is_none());
	}

	#[test]
	fn one_read_is_in_flight_and_a_per_minute_budget_bounds_a_sweep() {
		let mut state = State::default();
		let now = Instant::now();
		assert!(
			state
				.admit_thread_preview(Id(1), Id(10), false, now)
				.is_some()
		);
		assert!(
			state
				.admit_thread_preview(Id(1), Id(11), false, now)
				.is_none(),
			"a second card waits for the answer already in flight"
		);
		state.thread_previews.pending = None;
		for id in 100..111 {
			state.thread_previews.pending = None;
			assert!(
				state
					.admit_thread_preview(Id(1), Id(id), false, now)
					.is_some()
			);
		}
		state.thread_previews.pending = None;
		assert!(
			state
				.admit_thread_preview(Id(1), Id(999), false, now)
				.is_none(),
			"the minute's budget bounds a sweep across many cards"
		);
		state.thread_previews.sent.clear();
		assert!(
			state
				.admit_thread_preview(Id(1), Id(999), false, now)
				.is_some()
		);
	}

	#[test]
	fn an_edit_makes_a_preview_stale_and_a_deletion_forgets_it() {
		let mut state = State::default();
		let thread = Id(70);
		state
			.thread_previews
			.store(thread, answer(thread, Id(80)), Some(Id(80)), Instant::now());
		assert!(!state.thread_preview_needs_read(thread));

		// Another channel's message cannot stale this card.
		state.thread_previews.mark_stale(Id(71), Id(80));
		assert!(!state.thread_preview_needs_read(thread));

		// An edit keeps the old text on screen while the card reads the reply again.
		state.thread_previews.mark_stale(thread, Id(80));
		assert!(state.thread_preview_needs_read(thread));
		assert!(state.thread_preview(thread).is_some());
		assert!(
			state
				.admit_thread_preview(Id(1), thread, false, Instant::now())
				.is_some()
		);
		state.thread_previews.pending = None;

		// A deletion drops the preview outright; a card never shows a deleted reply.
		state.thread_previews.forget(thread, &[Id(80)]);
		assert!(state.thread_preview(thread).is_none());
		assert!(state.thread_preview_needs_read(thread));
	}

	#[test]
	fn a_newer_known_message_makes_a_trusted_preview_stale() {
		let mut state = State {
			channels: vec![channel(Id(70), 11, Some(Id(1)))],
			..State::default()
		};
		let thread = Id(70);
		state.channels[0].last_message = Some(Id(80));
		let now = Instant::now();
		state
			.thread_previews
			.store(thread, answer(thread, Id(80)), Some(Id(80)), now);
		assert!(!state.thread_preview_needs_read(thread));
		state.channels[0].last_message = Some(Id(81));
		state.invalidate_navigation();
		assert!(state.thread_preview_needs_read(thread));
	}
}
