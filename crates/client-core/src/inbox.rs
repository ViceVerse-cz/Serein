//! One bounded, on-demand page of the account's recent mentions.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Id, Message};

pub const PAGE_SIZE: usize = 25;
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_WIRE: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub struct View {
	pub messages: Vec<Message>,
	pub loading: bool,
	pub error: Option<&'static str>,
	pub before: Option<Id>,
	pub next: Option<Id>,
	pub loaded: bool,
	pub request: u64,
}

impl State {
	pub fn can_load_mentions(&self) -> bool {
		self.demo || (self.auth == AuthState::Authenticated && self.gateway_connected)
	}
	pub fn request_mentions(&mut self, before: Option<Id>) -> Option<Command> {
		if !self.can_load_mentions()
			|| self.inbox.loading
			|| before.is_some_and(|before| {
				Some(before) != self.inbox.next
					&& !(self.inbox.error.is_some() && Some(before) == self.inbox.before)
			}) {
			return None;
		}
		self.inbox = View {
			request: self.inbox.request.wrapping_add(1),
			loading: true,
			before,
			..Default::default()
		};
		Some(Command::Mentions {
			before,
			request: self.inbox.request,
		})
	}
	pub fn clear_mentions(&mut self) -> Command {
		self.inbox = View {
			request: self.inbox.request.wrapping_add(1),
			..Default::default()
		};
		Command::CancelMentions
	}
	pub fn apply_mentions(&mut self, request: u64, result: Result<Vec<Message>, Failure>) {
		if !self.can_load_mentions() || !self.inbox.loading || self.inbox.request != request {
			return;
		}
		self.inbox.loading = false;
		match result {
			Ok(mut messages)
				if messages.len() <= PAGE_SIZE
					&& messages.capacity() <= PAGE_SIZE
					&& messages.iter().map(Message::bytes).sum::<usize>() <= MAX_BYTES
					&& messages.iter().all(|message| {
						message.id.0 != 0
							&& message.channel.0 != 0
							&& self.inbox.before.is_none_or(|before| message.id < before)
					}) && messages.windows(2).all(|pair| pair[0].id > pair[1].id) =>
			{
				self.inbox.next =
					(messages.len() == PAGE_SIZE).then(|| messages.last().unwrap().id);
				messages.retain(|message| self.can_read_history(message.channel));
				self.inbox.messages = messages;
				self.inbox.loaded = true;
			}
			Ok(_) => self.inbox.error = Some("Mention results were invalid or too large"),
			Err(failure) => {
				self.inbox.error = Some(failure.label());
				if failure.ends_session() && failure != Failure::Capacity {
					self.fail(failure);
				}
			}
		}
	}
}
