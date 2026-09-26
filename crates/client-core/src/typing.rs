use crate::{State, auth::AuthState};
use model::{Freshness, Id};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy)]
pub struct Signal {
	pub channel: Id,
	pub user: Id,
	pub timestamp: u64,
}

/// Selected-conversation state only; fixed storage with no per-signal allocation.
#[derive(Default)]
pub struct Typing {
	channel: Option<Id>,
	users: [Option<(Id, Instant)>; 8],
}
impl Typing {
	pub(crate) fn clear(&mut self) {
		*self = Self::default();
	}
	fn remove(&mut self, user: Id) {
		for slot in &mut self.users {
			if slot.is_some_and(|(id, _)| id == user) {
				*slot = None;
			}
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
	/// Wall time validates the wire timestamp; monotonic time controls display expiry.
	/// Explicit clocks support deterministic offline fixtures without a timer task.
	pub fn observe_typing_at(&mut self, signal: Signal, wall: SystemTime, now: Instant) {
		if signal.channel.0 == 0
			|| signal.user.0 == 0
			|| self.typing_scope() != Some(signal.channel)
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
		let Some(remaining) = Duration::from_secs(10)
			.checked_sub(age)
			.filter(|remaining| !remaining.is_zero())
		else {
			return;
		};
		let Some(expires) = now.checked_add(remaining) else {
			return;
		};
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
	pub(crate) fn typing_message(&mut self, message: &model::Message) {
		if self.typing.channel == Some(message.channel)
			&& self.typing_scope() == Some(message.channel)
			&& message.author.id.0 != 0
			&& session_cache::Timeline::valid_message(message)
		{
			self.typing.remove(message.author.id);
		}
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
			sticker_items: Vec::new(),
			id: Id(100),
			channel: Id(channel),
			author: User {
				primary_guild: None,
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
	fn expiry_uses_wire_age_and_monotonic_deadline_without_mutating_queries() {
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

	#[test]
	fn timestamps_reject_expired_far_future_and_overflow_and_cap_tolerated_skew() {
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

	#[test]
	fn bounded_flood_refresh_and_out_of_order_signals_have_fixed_storage() {
		let mut state = state();
		let now = Instant::now();
		let wall = UNIX_EPOCH + Duration::from_secs(100);
		for user in 2..=1000 {
			state.observe_typing_at(signal(user, 100), wall, now);
		}
		assert_eq!(state.typing_users(now).count(), 8);
		assert!(size_of::<Typing>() <= 512);
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

	#[test]
	fn scope_auth_freshness_owner_and_generation_gate_typing() {
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

	#[test]
	fn valid_same_conversation_message_removes_only_its_author() {
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
