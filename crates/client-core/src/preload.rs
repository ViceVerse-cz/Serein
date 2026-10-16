//! Intent preloading: the command a click would send, sent while the pointer rests on its target.
//! A preload never crawls: one per kind is in flight, a per-minute cap bounds a sweep across a
//! list, and an unchanged target is not requested twice. A click on the target adopts the
//! in-flight request instead of sending another; otherwise the result is kept (a dormant window
//! or a cached profile) for the click to use.
use crate::{Command, State, auth::AuthState, navigable};
use model::{Freshness, Id};
use std::{
	collections::VecDeque,
	time::{Duration, Instant},
};

/// A pointer passing over a list rests on each row for less than this; intent rests longer.
pub const CONVERSATION_DWELL: Duration = Duration::from_millis(120);
/// Profiles use an unofficial, rate-limit-sensitive route, so they need a longer rest.
pub const PROFILE_DWELL: Duration = Duration::from_millis(300);
/// Speculative requests per minute and kind, so sweeping across many rows stays a handful.
pub(crate) const PER_MINUTE: usize = 12;
/// Remembered targets, so revisiting an unchanged one does not repeat its request.
const ATTEMPTS: usize = 64;
/// A preloaded page is trusted this long, so an unchanged target may be loaded again afterwards.
const ATTEMPT_AGE: Duration = Duration::from_secs(5 * 60);
/// Longer than the REST timeout: an in-flight marker this old lost its result to a session change.
const LOST: Duration = Duration::from_secs(30);
/// Preload request ids carry this bit, so they never equal a click's own request id.
const REQUEST_BIT: u64 = 1 << 63;

pub(crate) fn is_preload(request: u64) -> bool {
	request & REQUEST_BIT != 0
}

/// Dwell, admission, in-flight and adoption tracking for one kind of preload target.
pub struct Intent<K> {
	hovered: Option<(K, Instant)>,
	in_flight: Option<InFlight<K>>,
	/// A click took over this preload: `(preload request, the click's own request)`.
	adopted: Option<(u64, u64)>,
	sent: VecDeque<Instant>,
	/// Each target with the version (latest message) it was last requested at.
	attempts: VecDeque<(K, Option<Id>, Instant)>,
	next: u64,
}
struct InFlight<K> {
	target: K,
	request: u64,
	since: Instant,
	/// A live change for the target arrived meanwhile, so its result may already be outdated.
	stale: bool,
}
impl<K> Default for Intent<K> {
	fn default() -> Self {
		Self {
			hovered: None,
			in_flight: None,
			adopted: None,
			sent: VecDeque::new(),
			attempts: VecDeque::new(),
			next: 0,
		}
	}
}
impl<K: Copy + PartialEq> Intent<K> {
	/// The target once the pointer has rested on it for `dwell`, or at once while pressed.
	fn intent(
		&mut self,
		hovered: Option<K>,
		pressed: Option<K>,
		dwell: Duration,
		now: Instant,
	) -> Option<K> {
		if let Some(target) = pressed {
			self.hovered = Some((target, now));
			return Some(target);
		}
		let Some(target) = hovered else {
			self.hovered = None;
			return None;
		};
		match self.hovered {
			Some((current, since)) if current == target => {
				(now.saturating_duration_since(since) >= dwell).then_some(target)
			}
			_ => {
				self.hovered = Some((target, now));
				None
			}
		}
	}
	/// Time left before the resting target becomes intent.
	fn remaining(&self, dwell: Duration, now: Instant) -> Option<Duration> {
		let (_, since) = self.hovered?;
		dwell
			.checked_sub(now.saturating_duration_since(since))
			.filter(|left| !left.is_zero())
	}
	/// One request id for `target` at `version`, unless one is in flight, the per-minute cap is
	/// reached, or this exact version was already requested.
	fn admit(&mut self, target: K, version: Option<Id>, now: Instant) -> Option<u64> {
		if self
			.in_flight
			.as_ref()
			.is_some_and(|flight| now.saturating_duration_since(flight.since) < LOST)
		{
			return None;
		}
		while self
			.sent
			.front()
			.is_some_and(|at| now.saturating_duration_since(*at) >= Duration::from_secs(60))
		{
			self.sent.pop_front();
		}
		if self.sent.len() >= PER_MINUTE
			|| self.attempts.iter().any(|(known, known_version, at)| {
				(*known, *known_version) == (target, version)
					&& now.saturating_duration_since(*at) < ATTEMPT_AGE
			}) {
			return None;
		}
		self.attempts.retain(|(known, ..)| *known != target);
		if self.attempts.len() == ATTEMPTS {
			self.attempts.pop_front();
		}
		self.attempts.push_back((target, version, now));
		self.sent.push_back(now);
		self.next = self.next.wrapping_add(1) & !REQUEST_BIT;
		let request = REQUEST_BIT | self.next;
		self.in_flight = Some(InFlight {
			target,
			request,
			since: now,
			stale: false,
		});
		Some(request)
	}
	/// Lets a click take over the in-flight request for `target` as its own `request`, unless a
	/// live change may already be missing from it.
	pub(crate) fn adopt(&mut self, target: K, request: u64, now: Instant) -> bool {
		let Some(preload) =
			self.in_flight
				.as_ref()
				.filter(|flight| {
					flight.target == target
						&& !flight.stale && now.saturating_duration_since(flight.since) < LOST
				})
				.map(|flight| flight.request)
		else {
			return false;
		};
		self.adopted = Some((preload, request));
		true
	}
	/// Forgets that `target` was requested, so a later rest may load it again.
	pub(crate) fn forget(&mut self, target: K) {
		self.attempts.retain(|(known, ..)| *known != target);
	}
	fn interrupt(&mut self) {
		self.hovered = None;
		self.in_flight = None;
		self.adopted = None;
		self.attempts.clear();
	}
	/// Marks an in-flight result outdated when its target changes, or any result for `None`.
	pub(crate) fn touch(&mut self, target: Option<K>) {
		if let Some(flight) = &mut self.in_flight
			&& target.is_none_or(|target| flight.target == target)
		{
			flight.stale = true;
		}
	}
	/// Routes a preload's result. A failed target is forgotten so a later rest may try again.
	pub(crate) fn arrived(&mut self, request: u64, ok: bool) -> Arrival {
		let flight = self.in_flight.take_if(|flight| flight.request == request);
		if let Some(flight) = &flight
			&& !ok
		{
			self.attempts.retain(|(known, ..)| *known != flight.target);
		}
		match self.adopted {
			Some((preload, own)) if preload == request => {
				self.adopted = None;
				Arrival::Adopted(own, flight.is_some_and(|flight| !flight.stale))
			}
			_ if ok && flight.is_some_and(|flight| !flight.stale) => Arrival::Keep,
			_ => Arrival::Drop,
		}
	}
}

#[derive(Default)]
pub struct Preload {
	pub(crate) conversations: Intent<Id>,
	/// Profiles by `(user, guild)`, as a card would request them.
	pub(crate) profiles: Intent<(Id, Option<Id>)>,
	/// The recent-page request of a click whose adopted preload was turned away.
	pub(crate) retry: Option<Command>,
	/// The same for a profile card.
	pub(crate) profile_retry: Option<Command>,
}

/// Where a preload's result goes.
pub(crate) enum Arrival {
	/// A click adopted it: its own request id and whether the result may enter a cache.
	Adopted(u64, bool),
	/// Keep it for a later click: a dormant window or a cached profile.
	Keep,
	Drop,
}

impl State {
	/// The request a history result answers: a click's own id when it adopted this preload.
	pub fn history_request(&self, request: u64) -> u64 {
		match self.preload.conversations.adopted {
			Some((preload, own)) if preload == request => own,
			_ => request,
		}
	}
	/// A click's own requests, sent when the preloads it adopted were turned away.
	pub fn take_preload_retries(&mut self) -> impl Iterator<Item = Command> {
		[self.preload.retry.take(), self.preload.profile_retry.take()]
			.into_iter()
			.flatten()
	}
	/// Forget work owned by the replaced connection, retaining the minute caps and request ids.
	pub fn interrupt_preloads(&mut self) {
		self.preload.retry = None;
		self.preload.profile_retry = None;
		self.preload.conversations.interrupt();
		self.preload.profiles.interrupt();
	}

	/// Call once per frame with the conversation under the pointer and the one pressed, if any.
	pub fn preload_conversation(
		&mut self,
		hovered: Option<Id>,
		pressed: Option<Id>,
		now: Instant,
	) -> Option<Command> {
		let channel =
			self.preload
				.conversations
				.intent(hovered, pressed, CONVERSATION_DWELL, now)?;
		if !self.preloadable(channel) {
			return None;
		}
		let version = self.channel(channel)?.last_message;
		let request = self.preload.conversations.admit(channel, version, now)?;
		Some(Command::Prefetch(Box::new(Command::History {
			channel,
			before: None,
			after: None,
			request,
		})))
	}
	/// Call once per frame with the person under the pointer and the one pressed, if any, each
	/// with the server context their card would open in.
	pub fn preload_profile(
		&mut self,
		hovered: Option<(Id, Option<Id>)>,
		pressed: Option<(Id, Option<Id>)>,
		now: Instant,
	) -> Option<Command> {
		let target = self
			.preload
			.profiles
			.intent(hovered, pressed, PROFILE_DWELL, now)?;
		let (user, guild) = target;
		if !self.can_preload()
			|| user.0 == 0
			|| guild.is_some_and(|id| !self.guilds.iter().any(|g| g.id == id))
			|| self
				.profile
				.as_ref()
				.is_some_and(|view| view.user == user && view.guild == guild)
			|| self.profile_cached(user, guild, now)
		{
			return None;
		}
		// Profiles carry no version: a person is preloaded again only once their attempt expires.
		let request = self.preload.profiles.admit(target, None, now)?;
		Some(Command::Prefetch(Box::new(
			self.profile_command(user, guild, request),
		)))
	}
	/// How long until a resting pointer becomes intent, so the UI can wake without input.
	pub fn preload_wake(&self, now: Instant) -> Option<Duration> {
		let conversation = self
			.preload
			.conversations
			.remaining(CONVERSATION_DWELL, now);
		let profile = self.preload.profiles.remaining(PROFILE_DWELL, now);
		conversation.into_iter().chain(profile).min()
	}
	fn can_preload(&self) -> bool {
		!self.demo && self.auth == AuthState::Authenticated && self.gateway_connected
	}
	fn preloadable(&self, channel: Id) -> bool {
		self.can_preload()
			&& self.selected != Some(channel)
			// A conversation the user is already waiting for keeps the connection to itself.
			&& !(self.history_pending && self.freshness == Freshness::Loading)
			&& self.channel(channel).is_some_and(|entry| {
				navigable(entry) && entry.supports_text() && entry.last_message.is_some()
			})
			&& !self.is_forum(channel)
			&& self.archived_thread != Some(channel)
			&& self.can_read_history(channel)
			&& !self.resident_current(channel)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	use model::{Channel, Message, User};

	fn message(channel: u64, id: u64) -> Message {
		let mut message = test_message();
		message.id = Id(id);
		message.channel = Id(channel);
		message
	}
	fn test_message() -> Message {
		Message {
			poll: None,
			sticker_items: Vec::new(),
			id: Id(1),
			channel: Id(1),
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
			content: "Synthetic preload".into(),
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
			user: Some(test_message().author),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			channels: (1..=20)
				.map(|id| Channel {
					id: Id(id),
					guild: None,
					parent_id: None,
					kind: 1,
					position: 0,
					name: "Synthetic DM".into(),
					recipients: vec![],
					last_message: Some(Id(id * 1000 + 50)),
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
	fn page(channel: u64, request: u64) -> Event {
		Event::History {
			channel: Id(channel),
			request,
			older: false,
			messages: (1..=50)
				.map(|id| message(channel, channel * 1000 + id))
				.collect(),
		}
	}
	/// The conversation and request of a preloaded recent page.
	fn preloaded(command: Option<Command>) -> Option<(Id, u64)> {
		match command? {
			Command::Prefetch(command) => match *command {
				Command::History {
					channel,
					before: None,
					after: None,
					request,
				} => Some((channel, request)),
				_ => None,
			},
			_ => None,
		}
	}
	fn rest(state: &mut State, channel: u64, now: Instant) -> Option<Command> {
		assert!(
			state
				.preload_conversation(Some(Id(channel)), None, now)
				.is_none()
		);
		state.preload_conversation(Some(Id(channel)), None, now + CONVERSATION_DWELL)
	}

	#[test]
	fn a_click_whose_adopted_preload_gave_way_sends_its_own_request() {
		let mut state = state();
		let Some((_, request)) =
			preloaded(state.preload_conversation(None, Some(Id(1)), Instant::now()))
		else {
			panic!()
		};
		assert!(state.select(Id(1)).is_none());
		let own = state.request;
		assert_eq!(state.history_request(request), own);
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request,
				failure: crate::auth::Failure::Capacity,
			},
		);
		let retries: Vec<_> = state.take_preload_retries().collect();
		assert!(matches!(
			retries.as_slice(),
			[Command::History { channel: Id(1), before: None, after: None, request }]
				if *request == state.request && *request != own
		));
		assert!(state.history_pending);
		assert_eq!(state.freshness, Freshness::Loading);
		assert!(state.gateway_connected);
		assert_eq!(state.auth, AuthState::Authenticated);
		assert_ne!(state.status, "Work queue full; action was not sent");
		assert_eq!(state.take_preload_retries().count(), 0);
	}

	#[test]
	fn preloaded_windows_give_way_to_visited_ones_and_can_load_again() {
		let mut state = state();
		let now = Instant::now();
		// Three visited conversations park as dormant windows.
		for channel in 1..=4 {
			if let Some(Command::History { request, .. }) = state.select(Id(channel)) {
				apply(&mut state, page(channel, request));
			}
		}
		// A sweep preloads more conversations than the speculative share.
		for (step, channel) in (10..=14).enumerate() {
			let at = now + CONVERSATION_DWELL * 4 * step as u32;
			let Some((_, request)) = preloaded(rest(&mut state, channel, at)) else {
				panic!("conversation {channel} preloads");
			};
			apply(&mut state, page(channel, request));
		}
		// Visited conversations still reopen without a request.
		for channel in 1..=3 {
			assert!(state.resident_current(Id(channel)), "{channel}");
		}
		assert!(!state.resident_current(Id(10)) && !state.resident_current(Id(11)));
		assert!(state.resident_current(Id(14)));
		// An evicted preload may be loaded again.
		let later = now + Duration::from_secs(61);
		assert!(preloaded(rest(&mut state, 10, later)).is_some());
	}

	#[test]
	fn a_preloaded_page_stays_current_when_the_latest_known_message_was_deleted() {
		let mut state = state();
		// The latest known message (1051) is gone; the service returns through 1050.
		state.observe_last_message(Id(1), Id(1051));
		let Some((_, request)) = preloaded(rest(&mut state, 1, Instant::now())) else {
			panic!()
		};
		apply(&mut state, page(1, request));
		assert!(state.resident_current(Id(1)));
		assert!(
			state.select(Id(1)).is_none(),
			"opens without another request"
		);
		assert_eq!(state.freshness, Freshness::Fresh);
	}

	#[test]
	fn resting_preloads_once_and_a_click_opens_the_parked_page_without_a_request() {
		let now = Instant::now();
		let mut state = state();
		// Passing over a row is not intent; leaving it resets the dwell.
		assert!(state.preload_conversation(Some(Id(1)), None, now).is_none());
		assert!(state.preload_conversation(None, None, now).is_none());
		assert!(
			state
				.preload_conversation(Some(Id(1)), None, now + CONVERSATION_DWELL)
				.is_none()
		);
		let Some((channel, request)) =
			preloaded(state.preload_conversation(Some(Id(1)), None, now + CONVERSATION_DWELL * 2))
		else {
			panic!("rested preload");
		};
		assert_eq!(channel, Id(1));
		assert!(is_preload(request) && state.request & REQUEST_BIT == 0);
		// One in flight, and resting longer repeats nothing.
		assert!(
			state
				.preload_conversation(Some(Id(1)), None, now + CONVERSATION_DWELL * 3)
				.is_none()
		);
		assert!(rest(&mut state, 2, now).is_none());
		apply(&mut state, page(1, request));
		assert_eq!(state.resident_window_count(), 1);
		assert!(state.selected.is_none(), "a preload never navigates");
		assert!(state.select(Id(1)).is_none(), "the parked page is current");
		assert_eq!(state.freshness, Freshness::Fresh);
		assert_eq!(state.timeline.row_count(), 50);
		// A current dormant window or the open conversation is not requested again.
		assert!(rest(&mut state, 1, now).is_none());
		state.select(Id(2)).unwrap();
		assert!(rest(&mut state, 1, now).is_none());
	}

	#[test]
	fn a_click_adopts_the_in_flight_preload_instead_of_requesting_again() {
		let now = Instant::now();
		let mut state = state();
		let Some((_, request)) = preloaded(state.preload_conversation(None, Some(Id(3)), now))
		else {
			panic!("pressing skips the dwell");
		};
		assert!(state.select(Id(3)).is_none(), "adopted, not repeated");
		assert!(state.history_pending);
		assert_eq!(state.freshness, Freshness::Loading);
		apply(&mut state, page(3, request));
		assert_eq!(state.freshness, Freshness::Fresh);
		assert_eq!(state.timeline.row_count(), 50);
		assert!(!state.history_pending);
		// A failed adopted preload reports like the conversation's own request.
		let Some((_, request)) = preloaded(state.preload_conversation(None, Some(Id(4)), now))
		else {
			panic!("second preload");
		};
		assert!(state.select(Id(4)).is_none());
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(4),
				request,
				failure: crate::auth::Failure::Forbidden,
			},
		);
		assert_eq!(state.freshness, Freshness::Unavailable);
		// A message that arrived after the preload was sent may be missing from its page.
		let Some((_, request)) = preloaded(state.preload_conversation(None, Some(Id(9)), now))
		else {
			panic!("third preload");
		};
		apply(&mut state, Event::Message(message(9, 9051)));
		let Some(Command::History { request: own, .. }) = state.select(Id(9)) else {
			panic!("an outdated preload is not adopted");
		};
		apply(&mut state, page(9, request));
		assert!(state.history_pending, "the outdated page is dropped");
		let mut fresh = page(9, own);
		if let Event::History { messages, .. } = &mut fresh {
			messages.remove(0);
			messages.push(message(9, 9051));
		}
		apply(&mut state, fresh);
		assert!(state.timeline.get(Id(9051)).is_some());
	}

	#[test]
	fn outdated_failed_and_superseded_preloads_are_not_kept() {
		let now = Instant::now();
		{
			let mut state = state();
			let Some((_, request)) = preloaded(rest(&mut state, 5, now)) else {
				panic!("preload");
			};
			// An edit while the page was in flight may be missing from it.
			apply(
				&mut state,
				Event::Patch(model::MessagePatch {
					poll: model::Patch::Absent,
					sticker_items: model::Patch::Absent,
					channel: Id(5),
					id: Id(5001),
					content: model::Patch::Value("Edited".into()),
					components: model::Patch::Absent,
					flags: model::Patch::Absent,
					application_id: model::Patch::Absent,
					extra_content: Default::default(),
					reactions: model::Patch::Absent,
					mentions: model::Patch::Absent,
					edited: model::Patch::Absent,
					embeds: model::Patch::Absent,
					attachments: model::Patch::Absent,
					embeds_suppressed: model::Patch::Absent,
				}),
			);
			apply(&mut state, page(5, request));
			assert_eq!(state.resident_window_count(), 0);
		}
		{
			let mut state = state();
			let Some((_, request)) = preloaded(rest(&mut state, 6, now)) else {
				panic!("preload");
			};
			apply(
				&mut state,
				Event::HistoryFailed {
					channel: Id(6),
					request,
					failure: crate::auth::Failure::RateLimited,
				},
			);
			assert_eq!(state.resident_window_count(), 0);
			assert_eq!(state.freshness, Freshness::Stale, "nothing was selected");
			assert!(
				rest(&mut state, 6, now).is_some(),
				"a failure may be tried again"
			);
		}
		{
			// A click elsewhere before the page arrives leaves the page as a dormant window.
			let mut state = state();
			let Some((_, request)) = preloaded(rest(&mut state, 7, now)) else {
				panic!("preload");
			};
			state.select(Id(8)).unwrap();
			apply(&mut state, page(7, request));
			assert_eq!(state.selected, Some(Id(8)));
			assert_eq!(state.freshness, Freshness::Loading);
			assert_eq!(state.resident_window_count(), 1);
		}
	}

	#[test]
	fn sweeping_a_list_is_capped_and_unchanged_targets_are_not_repeated() {
		let now = Instant::now();
		let mut state = state();
		let mut sent = 0;
		for channel in 1..=20 {
			if let Some((_, request)) = preloaded(rest(&mut state, channel, now)) {
				sent += 1;
				apply(
					&mut state,
					Event::HistoryFailed {
						channel: Id(channel),
						request,
						failure: crate::auth::Failure::Network,
					},
				);
			}
		}
		assert_eq!(sent, PER_MINUTE);
		let later = now + Duration::from_secs(61);
		let Some((_, request)) = preloaded(rest(&mut state, 1, later)) else {
			panic!("the cap refills");
		};
		apply(&mut state, page(1, request));
		let Some(Command::History { request, .. }) = state.select(Id(2)) else {
			panic!("unloaded conversation");
		};
		apply(&mut state, page(2, request));
		state.observe_last_message(Id(1), Id(1999));
		assert!(
			rest(&mut state, 1, later).is_some(),
			"a newer message is a new version"
		);
		state.demo = true;
		assert!(rest(&mut state, 9, later + Duration::from_secs(61)).is_none());
	}
}
