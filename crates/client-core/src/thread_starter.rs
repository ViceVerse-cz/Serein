//! The message a thread hangs off, fetched once per selected thread and shown at its top.
use crate::{Command, State, auth::AuthState, auth::Failure};
use model::{Id, Message};

/// One bounded starter fetch at a time; the result belongs to exactly one thread.
#[derive(Default)]
pub struct Starter {
	/// Loaded starter message, keyed by the thread it introduces.
	pub loaded: Option<(Id, Message)>,
	pending: Option<(Id, u64)>,
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
			&& self.can_read_history(parent.id))
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
		match result {
			Ok(message)
				if message.id == thread
					&& self
						.channel(thread)
						.is_some_and(|c| c.parent_id == Some(message.channel))
					&& message.bytes() <= crate::MAX_EVENT_BYTES =>
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
	/// Forget everything about starters; navigation or session changes make them stale.
	pub(crate) fn reset_thread_starter(&mut self) {
		self.thread_starter = Starter {
			sequence: self.thread_starter.sequence,
			..Starter::default()
		};
	}
}
