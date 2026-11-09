//! On-demand reads of people and messages the session has not loaded, for mentions of
//! unknown users and replies whose original is outside the loaded history.
//!
//! Each person or message is read at most once per session: results are kept in small
//! bounded caches, and a failed read is remembered so it is never retried in a loop.
use crate::{State, auth::Failure};
use model::{Id, Message, User};
use std::collections::{BTreeSet, VecDeque};

/// People and messages kept from lookups; the oldest leave first.
const KEPT: usize = 256;
/// Lookups in flight at once; more wait for a later hover.
const IN_FLIGHT: usize = 4;

/// A read of one unloaded person or message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
	/// The public profile basics of `user`.
	User { user: Id },
	/// The message `message` in `channel`, read from the history around it.
	Message { channel: Id, message: Id },
}

/// The result of a [`Command`].
pub enum Event {
	User {
		user: Id,
		result: Result<User, Failure>,
	},
	Message {
		channel: Id,
		message: Id,
		result: Result<Option<Box<Message>>, Failure>,
	},
}

/// Bounded caches of people and messages read on demand, with what is pending or failed.
#[derive(Default)]
pub struct Lookups {
	users: VecDeque<User>,
	messages: VecDeque<Message>,
	/// Lookups in flight, by target.
	pending: BTreeSet<Target>,
	/// Targets that could not be read; never asked for again this session.
	missing: VecDeque<Target>,
	/// Messages edited or deleted while their lookup was in flight; that result is stale.
	superseded: BTreeSet<Id>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Target {
	User(Id),
	Message(Id),
}

impl Lookups {
	/// A person read by an earlier lookup.
	pub fn user(&self, id: Id) -> Option<&User> {
		self.users.iter().find(|user| user.id == id)
	}
	/// A message read by an earlier lookup.
	pub fn message(&self, id: Id) -> Option<&Message> {
		self.messages.iter().find(|message| message.id == id)
	}
	/// Whether `target` may be asked for now: not kept, pending, failed, or over the limit.
	fn admits(&self, target: Target) -> bool {
		let kept = match target {
			Target::User(id) => self.user(id).is_some(),
			Target::Message(id) => self.message(id).is_some(),
		};
		id_of(target).0 != 0
			&& !kept && !self.pending.contains(&target)
			&& !self.missing.contains(&target)
			&& self.pending.len() < IN_FLIGHT
	}
	/// Ends a lookup without a result. Transient failures may be asked for again on a later
	/// hover; a refusal, a bad response or a missing message is remembered as missing.
	/// Forgets lookups in flight when the connection fails, so a later hover can ask again.
	pub(crate) fn release_pending(&mut self) {
		self.pending.clear();
		self.superseded.clear();
	}
	/// Drops the kept copy of an edited or deleted message so it is never shown stale; an
	/// edited one is read again on the next hover, a deleted one is remembered as missing.
	pub(crate) fn forget_message(&mut self, id: Id, deleted: bool) {
		let kept = self.messages.iter().any(|message| message.id == id);
		self.messages.retain(|message| message.id != id);
		let target = Target::Message(id);
		let pending = self.pending.contains(&target);
		if pending {
			self.superseded.insert(id);
		}
		// Only messages this cache knew are remembered, so unrelated deletions elsewhere
		// never push real failures out of the bounded list.
		if deleted && (kept || pending) && !self.missing.contains(&target) {
			if self.missing.len() == KEPT {
				self.missing.pop_front();
			}
			self.missing.push_back(target);
		}
	}
	/// Ends the lookup of `target`; a lasting failure is remembered so it is not asked for again.
	fn fail(&mut self, target: Target, failure: Option<Failure>) {
		self.pending.remove(&target);
		if failure.is_some_and(|failure| {
			matches!(
				failure,
				Failure::RateLimited
					| Failure::Network
					| Failure::Ambiguous
					| Failure::Capacity
					| Failure::CapacityAt(_)
			)
		}) {
			return;
		}
		if self.missing.len() == KEPT {
			self.missing.pop_front();
		}
		self.missing.push_back(target);
	}
}

/// The ID a lookup target names.
fn id_of(target: Target) -> Id {
	match target {
		Target::User(id) | Target::Message(id) => id,
	}
}

impl State {
	/// Asks once for an unknown person, for a mention that names only an ID.
	pub fn prepare_user_lookup(&mut self, user: Id) -> Option<crate::Command> {
		let target = Target::User(user);
		if !self.lookups_ready() || !self.lookups.admits(target) {
			return None;
		}
		self.lookups.pending.insert(target);
		Some(crate::Command::Lookup(Command::User { user }))
	}
	/// Asks once for a reply's original message that the loaded history does not hold.
	pub fn prepare_message_lookup(&mut self, channel: Id, message: Id) -> Option<crate::Command> {
		let target = Target::Message(message);
		// Reading around a message needs history access, not only channel visibility.
		if !self.lookups_ready()
			|| !self.can_read_history(channel)
			|| self.timeline.get(message).is_some()
			|| !self.lookups.admits(target)
		{
			return None;
		}
		self.lookups.pending.insert(target);
		Some(crate::Command::Lookup(Command::Message {
			channel,
			message,
		}))
	}
	/// Whether lookups may be sent: signed in and not a demo.
	fn lookups_ready(&self) -> bool {
		self.auth == crate::auth::AuthState::Authenticated && !self.demo
	}
	/// Keeps a finished lookup's result, or records why it failed.
	pub(crate) fn apply_lookup(&mut self, event: Event) {
		match event {
			Event::User { user, result } => {
				let target = Target::User(user);
				match result {
					Ok(found) if found.id == user => {
						self.lookups.pending.remove(&target);
						if self.lookups.users.len() == KEPT {
							self.lookups.users.pop_front();
						}
						self.lookups.users.push_back(found);
					}
					Ok(_) => self.lookups.fail(target, None),
					Err(failure) => self.lookups.fail(target, Some(failure)),
				}
			}
			Event::Message {
				channel,
				message,
				result,
			} => {
				let target = Target::Message(message);
				if self.lookups.superseded.remove(&message) {
					self.lookups.pending.remove(&target);
					return;
				}
				match result {
					// A result arriving after history access ended is never kept.
					Ok(Some(found)) if found.id == message && found.channel == channel => {
						self.lookups.pending.remove(&target);
						if !self.can_read_history(channel) {
							return;
						}
						if self.lookups.messages.len() == KEPT {
							self.lookups.messages.pop_front();
						}
						self.lookups.messages.push_back(*found);
					}
					Ok(_) => self.lookups.fail(target, None),
					Err(failure) => self.lookups.fail(target, Some(failure)),
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::auth::AuthState;

	/// A signed-in state with no lookups.
	fn state() -> State {
		let mut state = State {
			auth: AuthState::Authenticated,
			..Default::default()
		};
		state.channels.push(model::Channel {
			id: Id(10),
			guild: None,
			parent_id: None,
			kind: 1,
			position: 0,
			name: "Synthetic lookup conversation".into(),
			last_message: None,
			member_list_id: None,
			tags: None,
			message_count: None,
			icon: None,
			recipients: vec![],
		});
		state
	}

	/// A user with `id` and a generated name.
	fn user(id: u64) -> User {
		User {
			username: String::new(),
			id: Id(id),
			name: format!("person-{id}"),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
		}
	}

	#[test]
	fn a_person_is_looked_up_once_and_kept() {
		let mut state = state();
		assert!(matches!(
			state.prepare_user_lookup(Id(5)),
			Some(crate::Command::Lookup(Command::User { user: Id(5) }))
		));
		assert!(
			state.prepare_user_lookup(Id(5)).is_none(),
			"already pending"
		);
		state.apply_lookup(Event::User {
			user: Id(5),
			result: Ok(user(5)),
		});
		assert_eq!(state.lookups.user(Id(5)).unwrap().name, "person-5");
		assert!(state.prepare_user_lookup(Id(5)).is_none(), "already kept");
		assert!(
			state.prepare_user_lookup(Id(0)).is_none(),
			"never the zero ID"
		);
	}

	#[test]
	fn failures_and_mismatches_are_not_retried() {
		let mut state = state();
		state.prepare_user_lookup(Id(6));
		state.apply_lookup(Event::User {
			user: Id(6),
			result: Err(Failure::Protocol),
		});
		assert!(state.prepare_user_lookup(Id(6)).is_none());
		state.apply_lookup(Event::User {
			user: Id(9),
			result: Err(Failure::Network),
		});
		state.prepare_user_lookup(Id(9));
		state.apply_lookup(Event::User {
			user: Id(9),
			result: Err(Failure::RateLimited),
		});
		assert!(
			state.prepare_user_lookup(Id(9)).is_some(),
			"a transient failure is asked for again later"
		);
		state.prepare_user_lookup(Id(7));
		state.apply_lookup(Event::User {
			user: Id(7),
			result: Ok(user(8)),
		});
		assert!(
			state.lookups.user(Id(8)).is_none(),
			"another person is not kept"
		);
		assert!(state.prepare_user_lookup(Id(7)).is_none());
	}

	#[test]
	fn lookups_are_bounded_and_wait_for_a_session() {
		let mut state = state();
		for id in 1..=IN_FLIGHT as u64 {
			assert!(state.prepare_user_lookup(Id(id)).is_some());
		}
		assert!(
			state.prepare_user_lookup(Id(99)).is_none(),
			"in-flight limit"
		);
		for id in 1..=IN_FLIGHT as u64 {
			state.apply_lookup(Event::User {
				user: Id(id),
				result: Ok(user(id)),
			});
		}
		for id in 100..100 + KEPT as u64 {
			state.prepare_user_lookup(Id(id));
			state.apply_lookup(Event::User {
				user: Id(id),
				result: Ok(user(id)),
			});
		}
		assert!(state.lookups.user(Id(1)).is_none(), "oldest left first");
		assert_eq!(state.lookups.users.len(), KEPT);
		let mut offline = State::default();
		assert!(offline.prepare_user_lookup(Id(5)).is_none());
	}

	#[test]
	fn a_reply_original_is_read_only_when_unloaded_and_visible() {
		let mut state = state();
		assert!(
			state.prepare_message_lookup(Id(11), Id(50)).is_none(),
			"unknown channel"
		);
		assert!(matches!(
			state.prepare_message_lookup(Id(10), Id(50)),
			Some(crate::Command::Lookup(Command::Message {
				channel: Id(10),
				message: Id(50)
			}))
		));
		let mut found = crate::tests::message(50);
		found.channel = Id(10);
		state.apply_lookup(Event::Message {
			channel: Id(10),
			message: Id(50),
			result: Ok(Some(Box::new(found))),
		});
		assert_eq!(state.lookups.message(Id(50)).unwrap().id, Id(50));
		state.prepare_message_lookup(Id(10), Id(51));
		state.apply_lookup(Event::Message {
			channel: Id(10),
			message: Id(51),
			result: Ok(None),
		});
		assert!(
			state.prepare_message_lookup(Id(10), Id(51)).is_none(),
			"deleted stays missing"
		);
	}

	#[test]
	fn edits_and_deletes_never_leave_a_stale_copy() {
		let mut state = state();
		let found = |id: u64| {
			let mut found = crate::tests::message(id);
			found.channel = Id(10);
			Ok(Some(Box::new(found)))
		};
		state.prepare_message_lookup(Id(10), Id(50));
		state.apply_lookup(Event::Message {
			channel: Id(10),
			message: Id(50),
			result: found(50),
		});
		state.lookups.forget_message(Id(50), false);
		assert!(
			state.lookups.message(Id(50)).is_none(),
			"edited copy dropped"
		);
		assert!(
			state.prepare_message_lookup(Id(10), Id(50)).is_some(),
			"an edited message is read again"
		);
		// Edited while in flight: that older result is discarded.
		state.lookups.forget_message(Id(50), false);
		state.apply_lookup(Event::Message {
			channel: Id(10),
			message: Id(50),
			result: found(50),
		});
		assert!(state.lookups.message(Id(50)).is_none());
		assert!(state.prepare_message_lookup(Id(10), Id(50)).is_some());
		// Deleted: dropped and never asked for again.
		state.apply_lookup(Event::Message {
			channel: Id(10),
			message: Id(50),
			result: found(50),
		});
		state.lookups.forget_message(Id(50), true);
		assert!(state.lookups.message(Id(50)).is_none());
		assert!(state.prepare_message_lookup(Id(10), Id(50)).is_none());
	}
}
