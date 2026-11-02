//! One bounded, on-demand page of the account's recent mentions.
use crate::{
	Command, Event, State,
	auth::{AuthState, Failure},
};
use model::{Id, Message};

pub const PAGE_SIZE: usize = 25;
pub const MAX_BYTES: usize = 1024 * 1024;
// At most 16 KiB of message identities while one response is in flight.
const MAX_MUTATIONS: usize = 1024;

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
	mutated: Vec<(Id, Id)>,
}

impl View {
	fn record_mutation(&mut self, channel: Id, id: Id) -> bool {
		if self.mutated.contains(&(channel, id)) {
			return true;
		}
		if self.mutated.len() == MAX_MUTATIONS {
			return false;
		}
		self.mutated.push((channel, id));
		true
	}
}

impl State {
	pub(crate) fn observe_mentions_mutation(&mut self, event: &Event) {
		if !self.inbox.loading {
			return;
		}
		let retained = match event {
			Event::Patch(patch) => self.inbox.record_mutation(patch.channel, patch.id),
			Event::Delete { channel, id } => self.inbox.record_mutation(*channel, *id),
			Event::DeleteBulk { channel, ids } => ids
				.iter()
				.all(|id| self.inbox.record_mutation(*channel, *id)),
			_ => true,
		};
		// Fail closed if an unusually busy request exceeds the bounded mutation set.
		if !retained {
			self.clear_mentions();
		}
	}

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
		let mutated = std::mem::take(&mut self.inbox.mutated);
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
				messages.retain(|message| {
					self.can_read_history(message.channel)
						&& !mutated.contains(&(message.channel, message.id))
				});
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};

	fn state() -> State {
		State {
			demo: true,
			channels: vec![model::Channel {
				id: Id(1),
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic DM".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				tags: None,
				message_count: None,
			}],
			..Default::default()
		}
	}

	#[test]
	fn unrelated_mutations_preserve_results_and_mutation_storage_is_bounded() {
		let mut state = state();
		let Command::Mentions { request, .. } = state.request_mentions(None).unwrap() else {
			panic!()
		};
		// The same ID in another channel must not hide this page's message.
		state.observe_mentions_mutation(&Event::Delete {
			channel: Id(2),
			id: Id(80),
		});
		state.apply_mentions(request, Ok(vec![crate::tests::message(80)]));
		assert_eq!(state.inbox.messages[0].id, Id(80));
		state.request_mentions(None).unwrap();
		for id in 0..MAX_MUTATIONS as u64 {
			state.observe_mentions_mutation(&Event::Delete {
				channel: Id(2),
				id: Id(id),
			});
		}
		assert!(state.inbox.loading);
		assert!(state.inbox.mutated.capacity() * std::mem::size_of::<(Id, Id)>() <= 16 * 1024);
		state.observe_mentions_mutation(&Event::Delete {
			channel: Id(2),
			id: Id(0),
		});
		assert!(state.inbox.loading, "duplicates do not consume capacity");
		state.observe_mentions_mutation(&Event::Delete {
			channel: Id(2),
			id: Id(MAX_MUTATIONS as u64),
		});
		assert!(!state.inbox.loading && state.inbox.mutated.is_empty());
	}

	#[test]
	fn filtered_page_preserves_its_older_cursor() {
		let mut state = state();
		let Command::Mentions { request, .. } = state.request_mentions(None).unwrap() else {
			panic!()
		};
		let ids = (1..=PAGE_SIZE as u64).map(Id).collect();
		state.observe_mentions_mutation(&Event::DeleteBulk {
			channel: Id(1),
			ids,
		});
		state.apply_mentions(
			request,
			Ok((1..=PAGE_SIZE as u64)
				.rev()
				.map(crate::tests::message)
				.collect()),
		);
		assert!(state.inbox.loaded && state.inbox.messages.is_empty());
		assert_eq!(state.inbox.next, Some(Id(1)));
		assert!(state.request_mentions(state.inbox.next).is_some());
	}

	#[test]
	fn message_mutations_filter_in_flight_mentions() {
		for event in [
			Event::Patch(crate::message_actions::content_patch(
				Id(1),
				Id(80),
				"edited".into(),
			)),
			Event::Delete {
				channel: Id(1),
				id: Id(80),
			},
			Event::DeleteBulk {
				channel: Id(1),
				ids: vec![Id(80)],
			},
		] {
			let mut state = state();
			let Command::Mentions { request, .. } = state.request_mentions(None).unwrap() else {
				panic!()
			};
			state.apply(Envelope {
				generation: state.generation,
				event,
			});
			assert!(state.inbox.loading);
			assert_eq!(state.inbox.request, request);
			// The response captured before the gateway mutation must not restore old text.
			state.apply_mentions(
				request,
				Ok(vec![crate::tests::message(81), crate::tests::message(80)]),
			);
			assert!(state.inbox.loaded);
			assert_eq!(
				state
					.inbox
					.messages
					.iter()
					.map(|m| m.id)
					.collect::<Vec<_>>(),
				[Id(81)]
			);
			assert!(
				state.request_mentions(None).is_some(),
				"refresh remains available"
			);
		}
	}
}
