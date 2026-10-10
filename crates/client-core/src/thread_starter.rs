//! The message a thread hangs off, fetched once per selected thread and shown at its top.
use crate::{Command, Event, State, auth::AuthState, auth::Failure};
use model::{Id, Message};

/// One bounded starter fetch at a time; the result belongs to exactly one thread.
#[derive(Default)]
pub struct Starter {
	/// Loaded starter message, keyed by the thread it introduces.
	pub loaded: Option<(Id, Message)>,
	pending: Option<(Id, u64)>,
	/// A matching live edit invalidated the in-flight response; finish it before retrying.
	pending_invalidated: bool,
	/// The last thread whose starter could not be loaded; never retried automatically.
	failed: Option<Id>,
	sequence: u64,
}

impl State {
	/// The parent channel and starter id of the selected thread, when it started from a message.
	fn thread_starter_target(&self) -> Option<(Id, Id)> {
		let thread = self.channel(self.selected?)?;
		if !matches!(thread.kind, 10..=12) {
			return None;
		}
		let parent = self.channel(thread.parent_id?)?;
		// Forum posts own their first message; only text/announcement threads have a starter.
		(parent.guild == thread.guild
			&& matches!(parent.kind, 0 | 5)
			&& self.can_read_history(parent.id)
			&& self.can_read_history(thread.id))
		.then_some((parent.id, thread.id))
	}
	/// Starter message of the selected thread, once loaded.
	pub fn thread_starter(&self) -> Option<&Message> {
		let (_, thread) = self.thread_starter_target()?;
		self.thread_starter
			.loaded
			.as_ref()
			.filter(|(id, _)| *id == thread)
			.map(|(_, message)| message)
	}
	/// Requests the selected thread's starter once; call each frame like other lazy reads.
	pub fn request_thread_starter(&mut self) -> Option<Command> {
		let (parent, thread) = self.thread_starter_target()?;
		if (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
			|| self.thread_starter.failed == Some(thread)
			|| self.thread_starter.pending.is_some()
			|| self
				.thread_starter
				.loaded
				.as_ref()
				.is_some_and(|(id, _)| *id == thread)
		{
			return None;
		}
		self.thread_starter.sequence = self.thread_starter.sequence.wrapping_add(1);
		let request = self.thread_starter.sequence;
		self.thread_starter.pending = Some((thread, request));
		self.thread_starter.pending_invalidated = false;
		Some(Command::ThreadStarter {
			thread,
			parent,
			request,
		})
	}
	pub(crate) fn apply_thread_starter(
		&mut self,
		thread: Id,
		request: u64,
		result: Result<Message, Failure>,
	) {
		if self.thread_starter.pending != Some((thread, request)) {
			return;
		}
		self.thread_starter.pending = None;
		if std::mem::take(&mut self.thread_starter.pending_invalidated) {
			if let Err(failure) = result
				&& failure.ends_session()
				&& failure != Failure::Capacity
			{
				self.fail(failure);
			}
			return;
		}
		match result {
			Ok(message)
				if message.id == thread
					&& self.thread_starter_target() == Some((message.channel, thread))
					&& (self.demo
						|| (self.auth == AuthState::Authenticated && self.gateway_connected))
					&& !message.ephemeral
					&& message.flags & 64 == 0
					&& session_cache::Timeline::valid_message(&message) =>
			{
				self.thread_starter.loaded = Some((thread, message));
				self.revision += 1;
			}
			Err(failure) if failure.ends_session() && failure != Failure::Capacity => {
				self.fail(failure);
			}
			_ => self.thread_starter.failed = Some(thread),
		}
	}
	/// Reconcile the parent message independently of the selected thread's own timeline.
	pub(crate) fn observe_thread_starter_event(&mut self, event: &Event) {
		if matches!(
			event,
			Event::Ready { .. }
				| Event::Disconnected
				| Event::Resumed
				| Event::Resync
				| Event::PermissionsChanged
		) {
			self.reset_thread_starter();
			return;
		}
		match event {
			Event::Message(message)
				if self.matches_thread_starter(message.channel, message.id)
					&& self.thread_starter.failed != Some(message.id)
					&& !message.ephemeral
					&& message.flags & 64 == 0
					&& session_cache::Timeline::valid_message(message) =>
			{
				if self.thread_starter.loaded.as_ref().is_some_and(|(_, old)| {
					old.edited_at
						.is_some_and(|at| message.edited_at.is_none_or(|new| new < at))
				}) {
					return;
				}
				let mut next = message.clone();
				if let Some((_, previous)) = &self.thread_starter.loaded {
					if let (Some(next), Some(old)) = (&mut next.poll, &previous.poll) {
						next.retain_results(old);
					}
					if next.author_roles.is_empty() {
						next.author_roles.clone_from(&previous.author_roles);
					}
					if next.author_nick.is_none() {
						next.author_nick.clone_from(&previous.author_nick);
					}
					next.revision = previous.revision.wrapping_add(1);
				}
				self.thread_starter.loaded =
					session_cache::Timeline::valid_message(&next).then_some((message.id, next));
				self.thread_starter.pending_invalidated |= self.thread_starter.pending.is_some();
			}
			Event::Patch(patch)
				if self.matches_thread_starter(patch.channel, patch.id)
					&& self.thread_starter.failed != Some(patch.id) =>
			{
				if let Some((thread, mut message)) = self.thread_starter.loaded.take() {
					session_cache::apply_patch(&mut message, patch);
					self.thread_starter.loaded = (!message.ephemeral
						&& message.flags & 64 == 0
						&& session_cache::Timeline::valid_message(&message))
					.then_some((thread, message));
				}
				self.thread_starter.pending_invalidated |= self.thread_starter.pending.is_some();
			}
			Event::Delete { channel, id } => self.forget_deleted_thread_starter(*channel, *id),
			Event::DeleteBulk { channel, ids } if ids.len() <= 100 => {
				for id in ids {
					self.forget_deleted_thread_starter(*channel, *id);
				}
			}
			_ => {}
		}
	}
	fn matches_thread_starter(&self, channel: Id, id: Id) -> bool {
		self.selected == Some(id) && self.thread_starter_target() == Some((channel, id))
	}
	fn forget_deleted_thread_starter(&mut self, channel: Id, id: Id) {
		if self.matches_thread_starter(channel, id) {
			self.reset_thread_starter();
			// One ID-only deletion guard prevents an older read or replay from restoring the body.
			self.thread_starter.failed = Some(id);
		}
	}
	pub(crate) fn prune_thread_starter(&mut self) {
		if self.thread_starter.loaded.is_none()
			&& self.thread_starter.pending.is_none()
			&& self.thread_starter.failed.is_none()
		{
			return;
		}
		let Some((parent, thread)) = self.thread_starter_target() else {
			self.reset_thread_starter();
			return;
		};
		if self
			.thread_starter
			.loaded
			.as_ref()
			.is_some_and(|(id, message)| *id != thread || message.channel != parent)
			|| self
				.thread_starter
				.pending
				.is_some_and(|(id, _)| id != thread)
			|| self.thread_starter.failed.is_some_and(|id| id != thread)
		{
			self.reset_thread_starter();
		}
	}
	/// Forget everything about starters; navigation or session changes make them stale.
	pub(crate) fn reset_thread_starter(&mut self) {
		self.thread_starter = Starter {
			sequence: self.thread_starter.sequence,
			..Starter::default()
		};
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event, auth::AuthState, message_actions::content_patch};
	use model::{Channel, Freshness, Guild, Patch};

	fn state() -> State {
		let parent = Channel {
			id: Id(1),
			guild: Some(Id(10)),
			parent_id: None,
			kind: 0,
			name: "Synthetic parent".into(),
			position: 0,
			recipients: vec![],
			last_message: Some(Id(3)),
			icon: None,
			member_list_id: None,
			tags: None,
			message_count: None,
		};
		let mut thread = parent.clone();
		thread.id = Id(3);
		thread.parent_id = Some(parent.id);
		thread.kind = 11;
		let mut state = State {
			user: Some(crate::tests::message(3).author),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			selected: Some(thread.id),
			guilds: vec![Guild {
				id: Id(10),
				name: "Synthetic guild".into(),
				default_message_notifications: None,
				icon: None,
				stickers: None,
				emojis: None,
			}],
			channels: vec![parent, thread],
			..State::default()
		};
		crate::tests::grant_permissions(&mut state);
		state
	}

	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}

	fn request(state: &mut State) -> u64 {
		let Some(Command::ThreadStarter { request, .. }) = state.request_thread_starter() else {
			panic!("starter request expected");
		};
		request
	}

	fn complete(state: &mut State, request: u64, content: &str) {
		let mut message = crate::tests::message(3);
		message.content = content.into();
		apply(
			state,
			Event::ThreadStarter {
				thread: Id(3),
				request,
				result: Ok(message),
			},
		);
	}

	#[test]
	fn parent_message_edits_update_loaded_starters_without_unrelated_or_stale_changes() {
		let mut state = state();
		let request = request(&mut state);
		complete(&mut state, request, "Original starter");
		let mut patch = content_patch(Id(1), Id(3), "Edited starter".into());
		patch.edited = Patch::Value(20);
		apply(&mut state, Event::Patch(patch));
		assert_eq!(state.thread_starter().unwrap().content, "Edited starter");
		assert_eq!(state.thread_starter().unwrap().edited_at, Some(20));
		assert!(state.request_thread_starter().is_none());
		for (channel, message, edited) in [(1, 4, 30), (9, 3, 30), (1, 3, 10)] {
			let mut patch = content_patch(Id(channel), Id(message), "Unrelated or old".into());
			patch.edited = Patch::Value(edited);
			apply(&mut state, Event::Patch(patch));
			assert_eq!(state.thread_starter().unwrap().content, "Edited starter");
		}
		for event in [
			Event::Delete {
				channel: Id(1),
				id: Id(4),
			},
			Event::Delete {
				channel: Id(9),
				id: Id(3),
			},
			Event::DeleteBulk {
				channel: Id(9),
				ids: vec![Id(3)],
			},
		] {
			apply(&mut state, event);
			assert_eq!(state.thread_starter().unwrap().content, "Edited starter");
		}
		let mut message = crate::tests::message(3);
		message.content = "New gateway starter".into();
		message.edited = true;
		message.edited_at = Some(30);
		apply(&mut state, Event::Message(message));
		assert_eq!(
			state.thread_starter().unwrap().content,
			"New gateway starter"
		);
	}

	#[test]
	fn edits_racing_starter_fetches_reject_the_old_response_and_rearm_the_read() {
		let mut state = state();
		let old_request = request(&mut state);
		apply(
			&mut state,
			Event::Patch(content_patch(Id(1), Id(3), "Edited while loading".into())),
		);
		complete(&mut state, old_request, "Stale response");
		assert!(state.thread_starter().is_none());
		let next_request = request(&mut state);
		assert_ne!(next_request, old_request);
		complete(&mut state, next_request, "Edited while loading");
		assert_eq!(
			state.thread_starter().unwrap().content,
			"Edited while loading"
		);
	}

	#[test]
	fn deleted_starters_release_payloads_and_cannot_return_from_inflight_reads() {
		for loaded in [false, true] {
			for bulk in [false, true] {
				let mut state = state();
				let request = request(&mut state);
				if loaded {
					complete(&mut state, request, "Deleted starter");
				}
				let event = if bulk {
					Event::DeleteBulk {
						channel: Id(1),
						ids: vec![Id(3), Id(4)],
					}
				} else {
					Event::Delete {
						channel: Id(1),
						id: Id(3),
					}
				};
				apply(&mut state, event);
				assert!(state.thread_starter.loaded.is_none());
				assert!(state.thread_starter.pending.is_none());
				complete(&mut state, request, "Late deleted starter");
				apply(&mut state, Event::Message(crate::tests::message(3)));
				assert!(state.thread_starter().is_none());
				assert!(state.request_thread_starter().is_none());
			}
		}
	}

	#[test]
	fn starter_session_boundaries_release_payloads_and_reject_late_responses() {
		for loaded in [false, true] {
			for event in [
				Event::Disconnected,
				Event::Resumed,
				Event::Resync,
				Event::PermissionsChanged,
				Event::Failure(Failure::Expired),
			] {
				let mut state = state();
				let request = request(&mut state);
				if loaded {
					complete(&mut state, request, "Previous session starter");
				}
				apply(&mut state, event);
				assert!(state.thread_starter.loaded.is_none());
				assert!(state.thread_starter.pending.is_none());
				complete(&mut state, request, "Late previous session starter");
				assert!(state.thread_starter.loaded.is_none());
			}
		}
	}

	#[test]
	fn a_new_gateway_starter_cannot_be_replaced_or_frozen_by_the_older_read() {
		for failure in [None, Some(Failure::Forbidden)] {
			let mut state = state();
			let old_request = request(&mut state);
			let mut message = crate::tests::message(3);
			message.content = "Current gateway starter".into();
			apply(&mut state, Event::Message(message));
			assert_eq!(
				state.thread_starter().unwrap().content,
				"Current gateway starter"
			);
			if let Some(failure) = failure {
				apply(
					&mut state,
					Event::ThreadStarter {
						thread: Id(3),
						request: old_request,
						result: Err(failure),
					},
				);
			} else {
				complete(&mut state, old_request, "Old HTTP starter");
			}
			apply(
				&mut state,
				Event::Patch(content_patch(Id(1), Id(3), "Next live edit".into())),
			);
			assert_eq!(state.thread_starter().unwrap().content, "Next live edit");
			assert!(state.thread_starter.pending.is_none());
			assert!(state.request_thread_starter().is_none());
		}
	}

	#[test]
	fn a_fresh_ready_releases_starters_and_keeps_the_next_request_distinct() {
		for loaded in [false, true] {
			let mut state = state();
			let old_request = request(&mut state);
			if loaded {
				complete(&mut state, old_request, "Previous READY starter");
			}
			let permissions = model::permissions::Snapshot {
				guilds: vec![model::permissions::Guild {
					id: Id(10),
					owner: Some(state.user.as_ref().unwrap().id),
					roles: Some(vec![]),
					member: None,
				}],
				channels: state
					.channels
					.iter()
					.map(|c| model::permissions::Channel {
						id: c.id,
						guild: Id(10),
						overwrites: Some(vec![]),
					})
					.collect(),
			};
			let event = Event::Ready {
				user: state.user.as_ref().unwrap().clone(),
				guilds: state.guilds.clone(),
				channels: state.channels.clone(),
				permissions,
			};
			apply(&mut state, event);
			assert!(state.thread_starter.loaded.is_none());
			assert!(state.thread_starter.pending.is_none());
			complete(&mut state, old_request, "Late previous READY starter");
			assert!(state.thread_starter().is_none());
			assert_ne!(request(&mut state), old_request);
		}
	}

	#[test]
	fn access_loss_and_home_navigation_cancel_starter_reads() {
		for loaded in [false, true] {
			let mut state = state();
			let old_request = request(&mut state);
			if loaded {
				complete(&mut state, old_request, "Revoked starter");
			}
			apply(
				&mut state,
				Event::Permissions(crate::permissions::Event::UnavailableGuild(Id(10))),
			);
			assert!(state.thread_starter.loaded.is_none());
			assert!(state.thread_starter.pending.is_none());
			crate::tests::grant_permissions(&mut state);
			complete(&mut state, old_request, "Late revoked starter");
			assert!(state.thread_starter().is_none());
			let next_request = request(&mut state);
			complete(&mut state, next_request, "Current starter");
			state.open_home();
			assert!(state.thread_starter.loaded.is_none());
		}
	}
}
