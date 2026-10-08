//! One pending user-solved challenge: an invite join, an account write or a queued message.
use crate::{Command, Delivery, State, captcha};
use std::time::Instant;

/// At most one challenged text message; its content stays in the bounded pending list.
#[derive(Default)]
pub struct SendVerification {
	sequence: u64,
	pending: Option<SendChallenge>,
}

struct SendChallenge {
	at: Instant,
	request: u64,
	nonce: String,
	reply: Option<crate::Reply>,
	challenge: captcha::Challenge,
}

impl State {
	/// The single challenge currently waiting on the user, if any.
	pub fn verification(&self) -> Option<(captcha::Verification, &captcha::Challenge)> {
		if let Some((request, challenge)) = self.invite_challenge() {
			return Some((captcha::Verification::Invite { request }, challenge));
		}
		if let Some(pending) = self.account_verification() {
			return Some(pending);
		}
		let (request, challenge) = self.message_challenge()?;
		Some((captcha::Verification::Message { request }, challenge))
	}
	/// Resumes the pending challenge with the user's solution.
	pub fn resume_verification(
		&mut self,
		verification: captcha::Verification,
		solution: captcha::Solution,
	) -> Option<Command> {
		match verification {
			captcha::Verification::Invite { request } => {
				self.resume_invite_challenge(request, solution)
			}
			captcha::Verification::Friend { request }
			| captcha::Verification::Direct { request } => self.resume_account_challenge(request, solution),
			captcha::Verification::Message { request } => {
				self.resume_message_challenge(request, solution)
			}
		}
	}
	/// Cancels the pending challenge and releases its write.
	pub fn cancel_verification(&mut self, verification: captcha::Verification) {
		match verification {
			captcha::Verification::Invite { request } => self.cancel_invite_challenge(request),
			captcha::Verification::Friend { request }
			| captcha::Verification::Direct { request } => self.cancel_account_challenge(request),
			captcha::Verification::Message { request } => {
				if self
					.message_challenge()
					.is_some_and(|(id, _)| id == request)
				{
					self.release_message_challenge(
						"Verification cancelled; the message was not sent",
					);
				}
			}
		}
	}
	/// Releases any challenge that outlived its lifetime.
	pub fn expire_verification(&mut self) {
		self.expire_invite_challenge();
		self.expire_account_challenge();
		if self
			.send_verification
			.pending
			.as_ref()
			.is_some_and(|pending| pending.at.elapsed() >= captcha::LIFETIME)
		{
			self.release_message_challenge("Verification expired; retry the message");
		}
	}
	/// The challenged message, while its optimistic row is still waiting to be sent.
	pub fn message_challenge(&self) -> Option<(u64, &captcha::Challenge)> {
		let pending = self.send_verification.pending.as_ref()?;
		(pending.at.elapsed() < captcha::LIFETIME
			&& self
				.pending
				.iter()
				.any(|p| p.nonce == pending.nonce && p.delivery == Delivery::Sending))
		.then_some((pending.request, &pending.challenge))
	}
	/// Holds one challenged send for the user; any other challenged send is rejected.
	pub(crate) fn apply_send_challenge(
		&mut self,
		nonce: String,
		reply: Option<crate::Reply>,
		challenge: captcha::Challenge,
	) {
		let waiting = self
			.pending
			.iter()
			.any(|p| p.nonce == nonce && p.delivery == Delivery::Sending);
		if waiting && self.message_challenge().is_none() {
			self.send_verification.sequence = self.send_verification.sequence.wrapping_add(1);
			self.send_verification.pending = Some(SendChallenge {
				at: Instant::now(),
				request: self.send_verification.sequence,
				nonce,
				reply,
				challenge,
			});
			self.status = "Verify to send this message";
		} else if let Some(p) = self
			.pending
			.iter_mut()
			.find(|p| p.nonce == nonce && p.delivery == Delivery::Sending)
		{
			p.delivery = Delivery::Rejected;
			self.status =
				"Discord requires verification for this message; retry it after the current check";
		}
	}
	/// Builds the one explicit resend that carries the user's solution.
	fn resume_message_challenge(
		&mut self,
		request: u64,
		solution: captcha::Solution,
	) -> Option<Command> {
		if self.demo
			|| !self.gateway_connected
			|| self.auth != crate::auth::AuthState::Authenticated
			|| self.message_challenge()?.0 != request
		{
			return None;
		}
		let pending = self.send_verification.pending.take()?;
		let message = self.pending.iter().find(|p| p.nonce == pending.nonce)?;
		let channel = message.channel;
		Some(Command::VerifiedSend {
			sticker: message.sticker.as_ref().map(|sticker| sticker.id),
			channel,
			content: message.content.clone(),
			reply: pending.reply,
			captcha: Box::new(captcha::Retry {
				target: captcha::Target::Message {
					channel,
					nonce: pending.nonce.clone(),
				},
				request,
				challenge: pending.challenge,
				solution,
				expires: pending.at + captcha::LIFETIME,
			}),
			nonce: pending.nonce,
		})
	}
	/// Marks the challenged row as not sent so the existing retry/discard controls apply.
	pub(crate) fn release_message_challenge(&mut self, status: &'static str) {
		let Some(pending) = self.send_verification.pending.take() else {
			return;
		};
		if let Some(p) = self
			.pending
			.iter_mut()
			.find(|p| p.nonce == pending.nonce && p.delivery == Delivery::Sending)
		{
			p.delivery = Delivery::Rejected;
		}
		self.status = status;
	}
}

#[cfg(test)]
mod tests {
	use crate::{Command, Delivery, Envelope, Event, auth::AuthState, captcha};

	fn challenge() -> Box<captcha::Challenge> {
		Box::new(captcha::Challenge::new("synthetic-key".into(), None, None, None, false).unwrap())
	}

	fn live() -> crate::State {
		crate::State {
			channels: vec![model::Channel {
				id: model::Id(1),
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
			selected: Some(model::Id(1)),
			auth: AuthState::Authenticated,
			freshness: model::Freshness::Fresh,
			gateway_connected: true,
			..Default::default()
		}
	}

	fn challenged(state: &mut crate::State) -> String {
		let Some(Command::Send { nonce, .. }) = state.prepare_text_send("hello there") else {
			panic!("text send")
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::SendChallenge {
				nonce: nonce.clone(),
				reply: None,
				challenge: challenge(),
			},
		});
		nonce
	}

	#[test]
	fn challenged_message_resumes_once_with_the_same_nonce_and_content() {
		let mut state = live();
		let channel = state.selected.unwrap();
		let nonce = challenged(&mut state);
		let Some((captcha::Verification::Message { request }, _)) = state.verification() else {
			panic!("message verification")
		};
		let solution = || captcha::Solution::new("synthetic-pass".into()).unwrap();
		assert!(
			state
				.resume_verification(
					captcha::Verification::Message {
						request: request + 1
					},
					solution()
				)
				.is_none()
		);
		let Some(Command::VerifiedSend {
			channel: target,
			content,
			nonce: resumed,
			captcha,
			..
		}) = state.resume_verification(captcha::Verification::Message { request }, solution())
		else {
			panic!("verified resend")
		};
		assert_eq!(
			(target, content.as_str(), resumed.as_str()),
			(channel, "hello there", nonce.as_str())
		);
		assert!(captcha.matches_target(&captcha::Target::Message {
			channel,
			nonce: nonce.clone()
		}));
		assert!(state.verification().is_none());
		assert!(
			state
				.resume_verification(captcha::Verification::Message { request }, solution())
				.is_none()
		);
		assert!(
			state
				.pending
				.iter()
				.any(|p| p.nonce == nonce && p.delivery == Delivery::Sending)
		);
	}

	#[test]
	fn cancelled_or_concurrent_message_challenges_reject_without_sending() {
		let mut state = live();
		let first = challenged(&mut state);
		// A second challenged send cannot replace the visible check.
		let second = challenged(&mut state);
		assert!(
			state
				.pending
				.iter()
				.any(|p| p.nonce == second && p.delivery == Delivery::Rejected)
		);
		let (verification, _) = state.verification().unwrap();
		state.cancel_verification(verification);
		assert!(state.verification().is_none());
		assert!(
			state
				.pending
				.iter()
				.any(|p| p.nonce == first && p.delivery == Delivery::Rejected)
		);
		// A challenge for a discarded row is never shown.
		let third = challenged(&mut state);
		state.pending.retain(|p| p.nonce != third);
		assert!(state.verification().is_none());
	}
}
