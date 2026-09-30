//! One bounded in-flight poll action, with authoritative readback and no replay.
use crate::{
	State,
	auth::{AuthState, Failure},
};
use model::{Freshness, Id, Message, Patch, polls::Create};

pub enum Action {
	Create(Box<Create>),
	Vote(Vec<u32>),
	End,
	Read,
}
pub struct Request {
	pub channel: Id,
	pub message: Option<Id>,
	pub request: u64,
	pub nonce: String,
	pub action: Action,
}
pub enum Event {
	Result {
		channel: Id,
		message: Option<Id>,
		request: u64,
		result: Result<Message, Failure>,
	},
	Vote {
		channel: Id,
		message: Id,
		user: Id,
		answer: u32,
		add: bool,
	},
}
#[derive(Default)]
pub struct Polls {
	pub pending: Option<(Id, Option<Id>, u64)>,
	pub created: u64,
	pub error: Option<&'static str>,
	sequence: u64,
	revision: Option<u64>,
}
impl Polls {
	pub fn reset(&mut self) {
		self.pending = None;
		self.error = None;
		self.sequence = self.sequence.wrapping_add(1);
	}
}
pub fn now_ms() -> i128 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_millis() as i128
}
impl State {
	pub fn can_create_poll(&self, channel: Id) -> bool {
		self.can_send(channel)
			&& self.permission(channel, model::permissions::SEND_POLLS) == Some(true)
	}
	pub fn can_vote_poll(&self, message: &Message) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.selected == Some(message.channel)
			&& self.freshness == Freshness::Fresh
			&& self.can_read_history(message.channel)
			&& !message.forwarded
			&& !message.ephemeral
			&& !self.timeline.is_deleted(message.id)
			&& message.poll.as_ref().is_some_and(|p| !p.ended(now_ms()))
	}
	pub fn prepare_poll(&mut self, message: Option<Id>, action: Action) -> Option<crate::Command> {
		let channel = self.selected?;
		if self.polls.pending.is_some() {
			self.status = "Wait for the current poll action";
			return None;
		}
		let valid = match &action {
			Action::Create(create) => {
				message.is_none() && create.valid() && self.can_create_poll(channel)
			}
			_ => {
				let source = self.timeline.get(message?)?;
				let poll = source.poll.as_ref()?;
				if source.channel != channel
					|| source.forwarded
					|| source.ephemeral
					|| self.timeline.is_deleted(source.id)
				{
					return None;
				}
				match &action {
					Action::Vote(answers) => {
						self.can_vote_poll(source)
							&& answers.len() <= model::polls::MAX_ANSWERS
							&& (poll.multiselect || answers.len() <= 1)
							&& answers.iter().enumerate().all(|(i, id)| {
								poll.answers.iter().any(|a| a.id == *id)
									&& !answers[..i].contains(id)
							})
					}
					Action::End => {
						self.can_vote_poll(source)
							&& self.user.as_ref().is_some_and(|u| u.id == source.author.id)
					}
					Action::Read => {
						self.auth == AuthState::Authenticated
							&& self.gateway_connected
							&& self.can_read_history(channel)
					}
					Action::Create(_) => false,
				}
			}
		};
		if !valid {
			self.status = "Poll action is unavailable with the current permissions or poll state";
			return None;
		}
		self.polls.sequence = self.polls.sequence.wrapping_add(1);
		let request = self.polls.sequence;
		self.polls.pending = Some((channel, message, request));
		self.polls.revision = message.and_then(|id| self.timeline.get(id).map(|m| m.revision));
		self.polls.error = None;
		self.send_sequence = self.send_sequence.wrapping_add(1);
		let nonce = crate::fingerprint::nonce(now_ms() as u128, self.send_sequence);
		Some(crate::Command::Polls(Request {
			channel,
			message,
			request,
			nonce,
			action,
		}))
	}
	pub(crate) fn apply_poll(&mut self, event: Event) -> Result<(), &'static str> {
		match event {
			Event::Result {
				channel,
				message,
				request,
				result,
			} => {
				if self.polls.pending != Some((channel, message, request)) {
					return Ok(());
				}
				self.polls.pending = None;
				match result {
					Ok(updated)
						if updated.channel == channel
							&& message.is_none_or(|id| id == updated.id)
							&& updated.poll.is_some()
							&& session_cache::Timeline::valid_message(&updated) =>
					{
						self.resident.remove(channel);
						if message.is_none() {
							self.polls.created = self.polls.created.wrapping_add(1);
							self.apply(crate::Envelope {
								generation: self.generation,
								event: crate::Event::Message(updated),
							});
						} else if self.selected == Some(channel)
							&& self.can_read_history(channel)
							&& self.timeline.get(updated.id).is_some_and(|m| {
								Some(m.revision) == self.polls.revision
									|| updated.poll.as_ref().is_some_and(|p| p.finalized)
							}) {
							// Later Gateway mutations win unless this is the finalized tally.
							let mut patch = crate::message_actions::content_patch(
								channel,
								updated.id,
								String::new(),
							);
							patch.content = Patch::Absent;
							patch.poll = Patch::Value(updated.poll);
							patch.extra_content.poll = Patch::Value(true);
							self.timeline.patch(patch)?;
							self.revision += 1;
						}
					}
					result => {
						let failure = result.err().unwrap_or(Failure::Protocol);
						if failure.ends_session() {
							self.fail(failure);
						}
						self.polls.error = Some(
							if matches!(failure, Failure::Network | Failure::Ambiguous) {
								"Poll action could not be confirmed. Refresh before trying again."
							} else {
								"Poll action failed. Check permissions and refresh the conversation."
							},
						);
						self.status = self.polls.error.unwrap();
					}
				}
			}
			Event::Vote {
				channel,
				message,
				user,
				answer,
				add,
			} => {
				self.resident.remove(channel);
				if self.selected != Some(channel) || !self.can_read_history(channel) {
					return Ok(());
				}
				let Some(source) = self.timeline.get(message).filter(|m| !m.forwarded) else {
					return Ok(());
				};
				let Some(mut poll) = source
					.poll
					.clone()
					.filter(|p| p.results_known && !p.finalized)
				else {
					return Ok(());
				};
				let Some(option) = poll.answers.iter_mut().find(|a| a.id == answer) else {
					return Ok(());
				};
				let own = self.user.as_ref().is_some_and(|u| u.id == user);
				if own && option.me == add {
					return Ok(());
				}
				option.votes = if add {
					option.votes.saturating_add(1)
				} else {
					option.votes.saturating_sub(1)
				};
				if own {
					option.me = add;
				}
				let mut patch =
					crate::message_actions::content_patch(channel, message, String::new());
				patch.content = Patch::Absent;
				patch.poll = Patch::Value(Some(poll));
				self.timeline.patch(patch)?;
				self.revision += 1;
			}
		}
		Ok(())
	}
}
