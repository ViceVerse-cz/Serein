use crate::{State, auth::AuthState};
use model::{Freshness, Id};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub struct Signal {
	pub channel: Id,
	pub user: Id,
	pub timestamp: u64,
}

/// How long a received indicator lasts, and how often this account's own is re-sent.
const TYPING_LIFETIME: Duration = Duration::from_secs(10);
const TYPING_RESEND: Duration = Duration::from_secs(8);
/// DM and group DM conversations tracked at once for the conversation list.
const DIRECT_TYPING: usize = 16;
/// Recent outgoing channels; a full store suppresses new channels until a slot expires.
const SENT_TYPING: usize = 16;

/// Selected-conversation typists, plus who is typing in any direct conversation for the
/// conversation list. Fixed storage with no per-signal allocation.
#[derive(Default)]
pub struct Typing {
	channel: Option<Id>,
	users: [Option<(Id, Instant)>; 8],
	/// (DM channel, typist, deadline).
	direct: [Option<(Id, Id, Instant)>; DIRECT_TYPING],
	/// Recent indicators this account sent, to re-send no more than every `TYPING_RESEND`.
	sent: [Option<(Id, Instant)>; SENT_TYPING],
}
impl Typing {
	/// Navigation clears only the selected conversation's typists.
	pub(crate) fn clear(&mut self) {
		self.channel = None;
		self.users = Default::default();
	}
	/// Session, failure and access changes clear everything.
	pub(crate) fn clear_all(&mut self) {
		*self = Self::default();
	}
	fn remove(&mut self, channel: Id, user: Id) {
		for slot in &mut self.users {
			if self.channel == Some(channel) && slot.is_some_and(|(id, _)| id == user) {
				*slot = None;
			}
		}
		for slot in &mut self.direct {
			if slot.is_some_and(|(at, id, _)| at == channel && id == user) {
				*slot = None;
			}
		}
	}
	fn observe_direct(&mut self, channel: Id, user: Id, expires: Instant, now: Instant) {
		for slot in &mut self.direct {
			if slot.is_some_and(|(_, _, deadline)| deadline <= now) {
				*slot = None;
			}
		}
		if let Some(slot) = self
			.direct
			.iter_mut()
			.flatten()
			.find(|(at, id, _)| *at == channel && *id == user)
		{
			slot.2 = slot.2.max(expires);
		} else if let Some(slot) = self.direct.iter_mut().find(|slot| slot.is_none()) {
			*slot = Some((channel, user, expires));
		}
	}
}
impl State {
	pub fn typing_scope(&self) -> Option<Id> {
		let channel = self.selected?;
		(self.auth == AuthState::Authenticated
			&& self.user.as_ref().is_some_and(|user| user.id.0 != 0)
			&& self.gateway_connected
			&& self.freshness == Freshness::Fresh
			&& !self.history_pending
			&& self.can_read_history(channel)
			&& self
				.channel(channel)
				.is_some_and(|known| known.supports_text()))
		.then_some(channel)
	}
	/// A DM or group DM this account can see, whose typing shows in the conversation list.
	fn direct_typing_scope(&self, channel: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.channel(channel).is_some_and(|known| {
				known.guild.is_none() && matches!(known.kind, 1 | 3) && self.can_view(channel)
			})
	}
	/// Wall time validates the wire timestamp; monotonic time controls display expiry.
	/// Explicit clocks support deterministic offline fixtures without a timer task.
	pub fn observe_typing_at(&mut self, signal: Signal, wall: SystemTime, now: Instant) {
		let selected = self.typing_scope() == Some(signal.channel);
		if signal.channel.0 == 0
			|| signal.user.0 == 0
			|| !(selected || self.direct_typing_scope(signal.channel))
			|| self
				.user
				.as_ref()
				.is_some_and(|owner| owner.id == signal.user)
		{
			return;
		}
		let Some(sent) = UNIX_EPOCH.checked_add(Duration::from_secs(signal.timestamp)) else {
			return;
		};
		if sent
			.duration_since(wall)
			.is_ok_and(|future| future > Duration::from_secs(5))
		{
			return;
		}
		let age = wall.duration_since(sent).unwrap_or_default();
		let Some(remaining) = TYPING_LIFETIME
			.checked_sub(age)
			.filter(|remaining| !remaining.is_zero())
		else {
			return;
		};
		let Some(expires) = now.checked_add(remaining) else {
			return;
		};
		if self.direct_typing_scope(signal.channel) {
			self.typing
				.observe_direct(signal.channel, signal.user, expires, now);
		}
		if !selected {
			return;
		}
		if self.typing.channel != Some(signal.channel) {
			self.typing.clear();
			self.typing.channel = Some(signal.channel);
		}
		for slot in &mut self.typing.users {
			if slot.is_some_and(|(_, deadline)| deadline <= now) {
				*slot = None;
			}
		}
		if let Some(slot) = self
			.typing
			.users
			.iter_mut()
			.flatten()
			.find(|(user, _)| *user == signal.user)
		{
			slot.1 = slot.1.max(expires);
		} else if let Some(slot) = self.typing.users.iter_mut().find(|slot| slot.is_none()) {
			*slot = Some((signal.user, expires));
		}
		// A ninth distinct user is ignored until a slot expires; existing users can refresh.
	}
	pub fn typing_users(&self, now: Instant) -> impl Iterator<Item = Id> + '_ {
		let visible = self.typing.channel.is_some() && self.typing.channel == self.typing_scope();
		self.typing.users.iter().filter_map(move |entry| {
			entry
				.filter(|(_, deadline)| visible && *deadline > now)
				.map(|(user, _)| user)
		})
	}
	pub fn typing_deadline(&self, now: Instant) -> Option<Instant> {
		if self.typing.channel.is_none() || self.typing.channel != self.typing_scope() {
			return None;
		}
		self.typing
			.users
			.iter()
			.flatten()
			.map(|(_, deadline)| *deadline)
			.filter(|deadline| *deadline > now)
			.min()
	}
	/// Whether someone other than this account is typing in a DM or group DM.
	pub fn direct_typing(&self, channel: Id, now: Instant) -> bool {
		self.direct_typing_scope(channel)
			&& self
				.typing
				.direct
				.iter()
				.flatten()
				.any(|(at, _, deadline)| *at == channel && *deadline > now)
	}
	/// Earliest expiry of any indicator the conversation list shows, for one repaint.
	pub fn direct_typing_deadline(&self, now: Instant) -> Option<Instant> {
		self.typing
			.direct
			.iter()
			.flatten()
			.filter(|(channel, _, deadline)| *deadline > now && self.direct_typing_scope(*channel))
			.map(|(_, _, deadline)| *deadline)
			.min()
	}
	/// This account's typing indicator for `channel`, at most once per `TYPING_RESEND`, and
	/// only while it could send a message there.
	pub fn prepare_typing(&mut self, channel: Id, now: Instant) -> Option<crate::Command> {
		if !self.gateway_connected
			|| self.freshness == Freshness::Unavailable
			|| !self.can_send(channel)
		{
			return None;
		}
		for slot in &mut self.typing.sent {
			if slot.is_some_and(|(_, sent)| now.saturating_duration_since(sent) >= TYPING_RESEND) {
				*slot = None;
			}
		}
		if self
			.typing
			.sent
			.iter()
			.flatten()
			.any(|(at, _)| *at == channel)
		{
			return None;
		}
		let slot = self.typing.sent.iter_mut().find(|slot| slot.is_none())?;
		*slot = Some((channel, now));
		Some(crate::Command::Typing { channel })
	}
	pub(crate) fn typing_message(&mut self, message: &model::Message) {
		if message.author.id.0 == 0 || !session_cache::Timeline::valid_message(message) {
			return;
		}
		// Discord ends the sender's indicator when their message arrives; so does sending ours.
		if self
			.user
			.as_ref()
			.is_some_and(|owner| owner.id == message.author.id)
		{
			for slot in &mut self.typing.sent {
				if slot.is_some_and(|(at, _)| at == message.channel) {
					*slot = None;
				}
			}
		}
		self.typing.remove(message.channel, message.author.id);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	use model::{Channel, Message, User};

	fn state() -> State {
		State {
			user: Some(User {
				primary_guild: None,
				cosmetics: None,
				id: Id(1),
				name: "Owner".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			selected: Some(Id(10)),
			freshness: Freshness::Fresh,
			channels: [10, 11]
				.into_iter()
				.map(|id| Channel {
					id: Id(id),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Synthetic DM".into(),
					kind: 1,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					tags: None,
					message_count: None,
					last_message: None,
				})
				.collect(),
			..State::default()
		}
	}
	fn signal(user: u64, timestamp: u64) -> Signal {
		Signal {
			channel: Id(10),
			user: Id(user),
			timestamp,
		}
	}
	fn message(user: u64, channel: u64) -> Message {
		Message {
			poll: None,
			sticker_items: Vec::new(),
			id: Id(100),
			channel: Id(channel),
			author: User {
				primary_guild: None,
				cosmetics: None,
				id: Id(user),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			content: "Synthetic".into(),
			reactions: None,
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
			kind: 0,
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
	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}

	#[test]
	fn direct_conversations_keep_typing_without_being_selected() {
		let mut state = state();
		let (wall, now) = (SystemTime::now(), Instant::now());
		let seconds = wall.duration_since(UNIX_EPOCH).unwrap().as_secs();
		let at = |channel: u64, user: u64| Signal {
			channel: Id(channel),
			user: Id(user),
			timestamp: seconds,
		};
		// DM 11 is not the selected conversation.
		state.observe_typing_at(at(11, 2), wall, now);
		assert!(state.direct_typing(Id(11), now));
		assert_eq!(
			state.typing_users(now).count(),
			0,
			"selected typists are separate"
		);
		assert!(state.direct_typing_deadline(now).is_some());
		assert!(!state.direct_typing(Id(11), now + Duration::from_secs(11)));
		// Navigation keeps conversation-list typing; the typist's message ends it.
		state.typing.clear();
		assert!(state.direct_typing(Id(11), now));
		state.typing_message(&message(2, 11));
		assert!(!state.direct_typing(Id(11), now));
		// This account's own typing and unselected guild channels are not tracked.
		state.observe_typing_at(at(11, 1), wall, now);
		assert!(!state.direct_typing(Id(11), now));
		state.channels[1].guild = Some(Id(5));
		state.observe_typing_at(at(11, 3), wall, now);
		state.channels[1].guild = None;
		assert!(!state.direct_typing(Id(11), now));
		// The store is bounded; a session failure clears it.
		for user in 2..100 {
			state.observe_typing_at(at(11, user), wall, now);
		}
		assert_eq!(state.typing.direct.iter().flatten().count(), DIRECT_TYPING);
		state.typing.clear_all();
		assert!(!state.direct_typing(Id(11), now));
	}

	#[test]
	fn own_typing_is_sent_while_composing_at_most_every_eight_seconds() {
		let mut state = state();
		let now = Instant::now();
		assert!(matches!(
			state.prepare_typing(Id(10), now),
			Some(crate::Command::Typing { channel: Id(10) })
		));
		assert!(
			state
				.prepare_typing(Id(10), now + Duration::from_secs(7))
				.is_none()
		);
		assert!(state.prepare_typing(Id(10), now + TYPING_RESEND).is_some());
		// Sending a message ends the indicator, so the next keystroke starts a new one.
		state.typing_message(&message(1, 10));
		assert!(
			state
				.prepare_typing(Id(10), now + Duration::from_secs(9))
				.is_some()
		);
		// Only for the open conversation, and only while connected.
		assert!(
			state
				.prepare_typing(Id(11), now + Duration::from_secs(30))
				.is_none()
		);
		state.gateway_connected = false;
		assert!(
			state
				.prepare_typing(Id(10), now + Duration::from_secs(60))
				.is_none()
		);
	}

	#[test]
	fn switching_drafts_keeps_each_channels_outgoing_throttle() {
		let mut state = state();
		let now = Instant::now();
		assert!(state.prepare_typing(Id(10), now).is_some());
		state.select(Id(11)).unwrap();
		state.freshness = Freshness::Fresh;
		state.history_pending = false;
		assert!(
			state
				.prepare_typing(Id(11), now + Duration::from_secs(1))
				.is_some()
		);
		state.select(Id(10)).unwrap();
		state.freshness = Freshness::Fresh;
		state.history_pending = false;
		assert!(
			state
				.prepare_typing(Id(10), now + Duration::from_secs(2))
				.is_none()
		);
		assert!(state.prepare_typing(Id(10), now + TYPING_RESEND).is_some());
		// Only the channel whose own message arrived gets a fresh indicator immediately.
		state.typing_message(&message(1, 11));
		state.select(Id(11)).unwrap();
		state.freshness = Freshness::Fresh;
		state.history_pending = false;
		assert!(state.prepare_typing(Id(11), now + TYPING_RESEND).is_some());
		state.typing.sent = [Some((Id(99), now)); SENT_TYPING];
		assert!(state.prepare_typing(Id(11), now).is_none());
		assert!(state.prepare_typing(Id(11), now + TYPING_RESEND).is_some());
	}

	#[test]
	fn expiry_uses_wire_age_and_monotonic_deadline_without_mutating_queries() {
		{
			let mut state = state();
			let now = Instant::now();
			let wall = UNIX_EPOCH + Duration::from_millis(100_500);
			state.observe_typing_at(signal(2, 98), wall, now);
			let deadline = now + Duration::from_millis(7500);
			assert_eq!(state.typing_deadline(now), Some(deadline));
			assert_eq!(state.typing_users(now).collect::<Vec<_>>(), vec![Id(2)]);
			assert_eq!(state.typing_users(deadline).count(), 0);
			assert_eq!(state.typing_deadline(deadline), None);
			assert_eq!(state.revision, 0);
			assert_eq!(state.timeline.row_count(), 0);
			assert!(state.typing.users[0].is_some()); // Queries need no pruning mutation.
		}
		{
			let now = Instant::now();
			let wall = UNIX_EPOCH + Duration::from_secs(100);
			for timestamp in [0, 89, 90, 106, u64::MAX] {
				let mut state = state();
				state.observe_typing_at(signal(2, timestamp), wall, now);
				assert_eq!(state.typing_users(now).count(), 0);
				assert_eq!(state.typing_deadline(now), None);
			}
			for (timestamp, remaining) in [(91, 1), (100, 10), (105, 10)] {
				let mut state = state();
				state.observe_typing_at(signal(2, timestamp), wall, now);
				assert_eq!(
					state.typing_deadline(now),
					Some(now + Duration::from_secs(remaining))
				);
			}
		}
	}

	#[test]
	fn bounded_flood_refresh_and_out_of_order_signals_have_fixed_storage() {
		{
			let mut state = state();
			let now = Instant::now();
			let wall = UNIX_EPOCH + Duration::from_secs(100);
			for user in 2..=1000 {
				state.observe_typing_at(signal(user, 100), wall, now);
			}
			assert_eq!(state.typing_users(now).count(), 8);
			// Selected typists plus the conversation-list store: still fixed, allocation-free storage.
			assert!(size_of::<Typing>() <= 1536);
			state.observe_typing_at(signal(2, 95), wall, now);
			assert_eq!(
				state.typing.users[0].unwrap().1,
				now + Duration::from_secs(10)
			);
			let later = now + Duration::from_secs(5);
			state.observe_typing_at(signal(2, 105), wall + Duration::from_secs(5), later);
			assert_eq!(
				state.typing.users[0].unwrap().1,
				later + Duration::from_secs(10)
			);
			let expired = now + Duration::from_secs(10);
			state.observe_typing_at(signal(1001, 110), wall + Duration::from_secs(10), expired);
			assert_eq!(
				state.typing_users(expired).collect::<Vec<_>>(),
				vec![Id(2), Id(1001)]
			);
		}
		{
			let now = Instant::now();
			let wall = UNIX_EPOCH + Duration::from_secs(100);
			for blocked in 0..9 {
				let mut state = state();
				let mut signal = signal(2, 100);
				match blocked {
					0 => signal.user = Id(0),
					1 => signal.user = Id(1),
					2 => signal.channel = Id(11),
					3 => state.auth = AuthState::Expired,
					4 => state.gateway_connected = false,
					5 => state.freshness = Freshness::Loading,
					6 => state.channels[0].kind = 2,
					7 => state.channels[0].guild = Some(Id(99)), // Unknown permissions.
					_ => state.history_pending = true,
				}
				state.observe_typing_at(signal, wall, now);
				assert_eq!(state.typing_users(now).count(), 0);
			}
			let mut state = state();
			state.apply(Envelope {
				generation: state.generation - 1,
				event: Event::Typing(signal(2, 100)),
			});
			assert_eq!(state.typing_users(now).count(), 0);
		}
	}

	#[test]
	fn incoming_typing_does_not_invalidate_timeline_residents_or_revision() {
		let mut state = state();
		state.timeline.insert(message(2, 10), false, false).unwrap();
		state.select(Id(11)).unwrap();
		state.freshness = Freshness::Fresh;
		state.history_pending = false;
		let revision = state.revision;
		let windows = state.resident_window_count();
		let bytes = state.resident_history_bytes();
		let timestamp = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.unwrap()
			.as_secs();
		apply(
			&mut state,
			Event::Typing(Signal {
				channel: Id(11),
				user: Id(2),
				timestamp,
			}),
		);
		assert_eq!(
			state.typing_users(Instant::now()).collect::<Vec<_>>(),
			vec![Id(2)]
		);
		assert_eq!(state.revision, revision);
		assert_eq!(state.resident_window_count(), windows);
		assert_eq!(state.resident_history_bytes(), bytes);
	}

	#[test]
	fn navigation_session_and_permission_transitions_clear_typing() {
		{
			let now = Instant::now();
			let wall = UNIX_EPOCH + Duration::from_secs(100);
			for transition in 0..7 {
				let mut state = state();
				state.observe_typing_at(signal(2, 100), wall, now);
				match transition {
					0 => {
						state.select(Id(11));
					}
					1 => apply(&mut state, Event::Disconnected),
					2 => apply(&mut state, Event::Resync),
					3 => apply(&mut state, Event::PermissionsChanged),
					4 => apply(&mut state, Event::Unavailable(Id(10))),
					5 => apply(&mut state, Event::Failure(crate::auth::Failure::Expired)),
					_ => state.logout(),
				}
				assert!(state.typing.users.iter().all(Option::is_none));
				assert_eq!(state.typing_deadline(now), None);
			}
		}
		{
			let mut state = state();
			let now = Instant::now();
			let wall = UNIX_EPOCH + Duration::from_secs(100);
			for user in [2, 3] {
				state.observe_typing_at(signal(user, 100), wall, now);
			}
			apply(&mut state, Event::Message(message(2, 11)));
			assert_eq!(state.typing_users(now).count(), 2);
			apply(&mut state, Event::Message(message(2, 10)));
			assert_eq!(state.typing_users(now).collect::<Vec<_>>(), vec![Id(3)]);
		}
	}
}
