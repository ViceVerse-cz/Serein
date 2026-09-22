//! Session-only invite metadata: 32 entries, at most 4 KiB each, five-minute lifetime.
use crate::{Command, State, auth::Failure};
use std::{
	collections::BTreeMap,
	time::{Duration, Instant},
};
pub type Cache = BTreeMap<String, (Instant, Option<Result<model::InvitePreview, Failure>>)>;
pub use model::server_invites::valid_code;

impl State {
	pub fn expire_invite_challenge(&mut self) {
		if self
			.invite_join
			.challenge
			.as_ref()
			.is_some_and(|(at, _)| at.elapsed() >= crate::captcha::LIFETIME)
		{
			self.invites.remove(&self.invite_join.code);
			self.cancel_invite_join();
			self.invite_join.result =
				Some(Err(Failure::ProtocolAt("Verification expired; join again")));
		}
	}
	pub fn invite_challenge(&self) -> Option<(u64, &crate::captcha::Challenge)> {
		let (at, challenge) = self.invite_join.challenge.as_ref()?;
		(self.invite_join.pending && at.elapsed() < crate::captcha::LIFETIME)
			.then_some((self.invite_join.sequence, challenge))
	}
	pub fn cancel_invite_challenge(&mut self, request: u64) {
		if request == self.invite_join.sequence && self.invite_join.challenge.is_some() {
			self.cancel_invite_join();
			self.invite_join.result = Some(Err(Failure::ProtocolAt(
				"Verification cancelled; join again",
			)));
		}
	}
	/// Builds the one explicit retry that resumes a solved invite challenge.
	pub fn resume_invite_challenge(
		&mut self,
		request: u64,
		solution: crate::captcha::Solution,
	) -> Option<Command> {
		if self.demo
			|| !self.gateway_connected
			|| self.auth != crate::auth::AuthState::Authenticated
			|| self.invite_challenge()?.0 != request
		{
			return None;
		}
		let (at, challenge) = self.invite_join.challenge.take()?;
		// The new identity makes a duplicated response to the first attempt stale.
		self.invite_join.sequence = self.invite_join.sequence.wrapping_add(1);
		Some(Command::JoinInvite {
			code: self.invite_join.code.clone(),
			request: self.invite_join.sequence,
			captcha: Some(Box::new(crate::captcha::Retry {
				target: crate::captcha::Target::Invite {
					code: self.invite_join.code.clone(),
				},
				request: self.invite_join.sequence,
				challenge,
				solution,
				expires: (at + crate::captcha::LIFETIME)
					.min(Instant::now() + Duration::from_secs(120)),
			})),
		})
	}
	pub(crate) fn apply_invite_challenge(
		&mut self,
		request: u64,
		challenge: crate::captcha::Challenge,
	) {
		if self.invite_join.pending
			&& self.invite_join.sequence == request
			&& self.invite_join.challenge.is_none()
		{
			self.invite_join.challenge = Some((Instant::now(), challenge));
			self.status = "Verify to join this server";
		}
	}
	pub fn request_invite(&mut self, code: String) -> Option<Command> {
		if !self.selected.is_some_and(|c| self.can_read_history(c)) {
			return None;
		}
		self.request_join_preview(code)
	}
	/// Explicit invite lookup from the join dialog does not require a selected conversation.
	pub fn request_join_preview(&mut self, code: String) -> Option<Command> {
		if self.demo
			|| !valid_code(&code)
			|| self.auth != crate::auth::AuthState::Authenticated
			|| !self.gateway_connected
		{
			return None;
		}
		self.invites
			.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(300));
		if self.invites.contains_key(&code) || self.invites.values().any(|(_, v)| v.is_none()) {
			return None;
		}
		if self.invites.len() >= 32 {
			let oldest = self
				.invites
				.iter()
				.min_by_key(|(_, (at, _))| *at)
				.map(|(code, _)| code.clone());
			if let Some(oldest) = oldest {
				self.invites.remove(&oldest);
			}
		}
		self.invites.insert(code.clone(), (Instant::now(), None));
		Some(Command::Invite { code })
	}
	pub fn apply_invite(&mut self, code: String, result: Result<model::InvitePreview, Failure>) {
		let result = result.and_then(|embed| {
			if embed.guild.0 != 0
				&& model::valid_embeds(std::slice::from_ref(&embed.embed))
				&& embed.bytes() <= 4096
			{
				Ok(embed)
			} else {
				Err(Failure::Capacity)
			}
		});
		if let Some((at, value)) = self.invites.get_mut(&code) {
			*at = Instant::now();
			*value = Some(result);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	#[test]
	fn explicit_preview_works_without_a_channel_but_keeps_session_and_cache_guards() {
		let mut state = State::default();
		assert!(state.request_join_preview("synthetic".into()).is_none());
		state.auth = crate::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		assert!(state.request_invite("synthetic".into()).is_none());
		assert!(state.request_join_preview("../bad".into()).is_none());
		assert!(matches!(
			state.request_join_preview("synthetic".into()),
			Some(Command::Invite { .. })
		));
		assert!(state.request_join_preview("synthetic".into()).is_none());
		assert!(state.request_join_preview("another".into()).is_none());
		state.apply_invite(
			"synthetic".into(),
			Ok(model::InvitePreview {
				guild: model::Id(2),
				embed: model::Embed::default(),
			}),
		);
		assert!(state.guild(model::Id(2)).is_none());
		assert!(matches!(
			state.join_invite("synthetic".into()),
			Some(Command::JoinInvite { .. })
		));
		state.demo = true;
		assert!(state.request_join_preview("another".into()).is_none());
		state.demo = false;
		state.gateway_connected = false;
		assert!(state.request_join_preview("another".into()).is_none());
	}

	#[test]
	fn boxed_invite_charges_payload_and_reaches_the_pending_preview() {
		let mut title = String::with_capacity(1024);
		title.push_str("Synthetic invite");
		let embed = model::Embed {
			title: Some(title),
			..Default::default()
		};
		let event = Event::Invite {
			code: "synthetic".into(),
			result: Ok(Box::new(model::InvitePreview {
				guild: model::Id(2),
				embed,
			})),
		};
		assert!(event.bytes() >= size_of::<Event>() + size_of::<model::Embed>() + 1024);
		let mut state = State::default();
		state
			.invites
			.insert("synthetic".into(), (Instant::now(), None));
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
		assert_eq!(
			state.invites["synthetic"]
				.1
				.as_ref()
				.unwrap()
				.as_ref()
				.unwrap()
				.embed
				.title
				.as_deref(),
			Some("Synthetic invite")
		);
	}
}

/// One bounded, session-scoped write. Gateway membership remains authoritative.
#[derive(Default)]
pub struct Join {
	pub code: String,
	pub pending: bool,
	pub result: Option<Result<model::Id, Failure>>,
	sequence: u64,
	challenge: Option<(Instant, crate::captcha::Challenge)>,
}
impl State {
	pub fn can_join_invite(&self, code: &str) -> bool {
		!self.demo
			&& self.auth == crate::auth::AuthState::Authenticated
			&& self.gateway_connected
			&& !self.invite_join.pending
			&& valid_code(code)
			&& self.invites.get(code).is_some_and(|(at, value)| {
				at.elapsed() < Duration::from_secs(300)
					&& value
						.as_ref()
						.is_some_and(|v| v.as_ref().is_ok_and(|p| self.guild(p.guild).is_none()))
			}) && !(self.invite_join.code == code && matches!(self.invite_join.result, Some(Ok(_))))
	}
	pub fn join_invite(&mut self, code: String) -> Option<Command> {
		if !self.can_join_invite(&code) {
			return None;
		}
		self.invite_join.sequence = self.invite_join.sequence.wrapping_add(1);
		self.invite_join.code = code;
		self.invite_join.pending = true;
		self.invite_join.result = None;
		self.invite_join.challenge = None;
		Some(Command::JoinInvite {
			code: self.invite_join.code.clone(),
			request: self.invite_join.sequence,
			captcha: None,
		})
	}
	pub(crate) fn cancel_invite_join(&mut self) {
		self.invite_join.challenge = None;
		if self.invite_join.pending {
			self.invite_join.pending = false;
			self.invite_join.result = Some(Err(Failure::Ambiguous));
		}
	}
	pub(crate) fn apply_invite_join(&mut self, request: u64, result: Result<model::Id, Failure>) {
		if !self.invite_join.pending
			|| request != self.invite_join.sequence
			|| self.invite_join.challenge.is_some()
		{
			return;
		}
		self.invite_join.pending = false;
		self.invite_join.challenge = None;
		self.invite_join.result = Some(result);
		self.status = match result {
			Ok(_) => {
				"Invite accepted · waiting for server access; complete any server rules in Discord"
			}
			Err(f) => f.label(),
		};
		if let Err(f) = result
			&& f.ends_session()
		{
			self.fail(f);
		}
	}
}

#[cfg(test)]
mod join_tests {
	use super::*;
	/// Regression: invite challenges are single-use, scoped, expiring and redacted.
	#[test]
	fn invite_challenge_is_single_use_scoped_expiring_and_redacted() {
		let challenge = || {
			crate::captcha::Challenge::new(
				"synthetic-sitekey".into(),
				Some("private-rqdata".into()),
				Some("private-rqtoken".into()),
				None,
				false,
			)
			.unwrap()
		};
		let solution = || crate::captcha::Solution::new("private-solution".into()).unwrap();
		assert!(!format!("{:?}", challenge()).contains("private"));
		assert!(!format!("{:?}", solution()).contains("private"));
		assert!(crate::captcha::Solution::new("bad\r\nheader".into()).is_none());
		assert!(crate::captcha::Solution::new("x".repeat(8193)).is_none());
		let mut state = State {
			auth: crate::auth::AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		};
		state.invite_join.pending = true;
		state.invite_join.code = "synthetic".into();
		state.apply_invite_challenge(1, challenge());
		assert!(state.invite_challenge().is_none());
		state.apply_invite_challenge(0, challenge());
		assert!(state.resume_invite_challenge(1, solution()).is_none());
		assert!(state.invite_challenge().is_some());
		let Command::JoinInvite {
			code,
			request,
			captcha: Some(retry),
		} = state.resume_invite_challenge(0, solution()).unwrap()
		else {
			panic!("retry missing")
		};
		assert!(retry.matches(
			&crate::captcha::Target::Invite { code: code.clone() },
			request
		));
		assert!(!retry.matches(
			&crate::captcha::Target::Invite {
				code: "another".into()
			},
			request
		));
		assert!(state.resume_invite_challenge(0, solution()).is_none());
		state.apply_invite_challenge(0, challenge());
		assert!(state.invite_challenge().is_none());
		state.apply_invite_challenge(request, challenge());
		state.invite_join.challenge.as_mut().unwrap().0 = Instant::now() - crate::captcha::LIFETIME;
		assert!(state.resume_invite_challenge(request, solution()).is_none());
		state.invites.insert(
			"synthetic".into(),
			(Instant::now() - crate::captcha::LIFETIME, None),
		);
		state
			.invites
			.insert("another".into(), (Instant::now(), None));
		state.expire_invite_challenge();
		assert!(!state.invites.contains_key("synthetic"));
		assert!(state.invites.contains_key("another"));
		assert!(!state.invite_join.pending);
		state.apply_invite_challenge(request, challenge());
		assert!(state.invite_challenge().is_none());
		assert_eq!(state.auth, crate::auth::AuthState::Authenticated);
		state.invite_join.pending = true;
		state.apply_invite_challenge(request, challenge());
		state.cancel_invite_challenge(request);
		assert!(state.resume_invite_challenge(request, solution()).is_none());
		assert_eq!(
			state.invite_join.result,
			Some(Err(Failure::ProtocolAt(
				"Verification cancelled; join again"
			)))
		);
		state.invite_join.pending = true;
		state.apply_invite_challenge(request, challenge());
		state.logout();
		assert!(state.invite_challenge().is_none());
	}
	#[test]
	fn join_requires_a_fresh_preview_and_ignores_duplicate_and_stale_writes() {
		let mut state = State {
			auth: crate::auth::AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		};
		assert!(state.join_invite("synthetic".into()).is_none());
		state.invites.insert(
			"synthetic".into(),
			(
				Instant::now(),
				Some(Ok(model::InvitePreview {
					guild: model::Id(2),
					embed: model::Embed::default(),
				})),
			),
		);
		assert!(state.join_invite("../bad".into()).is_none());
		let Some(Command::JoinInvite { request, .. }) = state.join_invite("synthetic".into())
		else {
			panic!("join command missing")
		};
		assert!(state.join_invite("synthetic".into()).is_none());
		state.apply_invite_join(request.wrapping_add(1), Ok(model::Id(2)));
		assert!(state.invite_join.pending);
		state.cancel_invite_join();
		state.apply_invite_join(request, Ok(model::Id(2)));
		assert_eq!(state.invite_join.result, Some(Err(Failure::Ambiguous)));
		let command = state.join_invite("synthetic".into()).unwrap();
		state.command_rejected(command);
		assert!(!state.invite_join.pending);
		let Some(Command::JoinInvite { request, .. }) = state.join_invite("synthetic".into())
		else {
			panic!("retry missing")
		};
		state.apply_invite_join(request, Ok(model::Id(2)));
		assert!(
			state.guild(model::Id(2)).is_none(),
			"HTTP must not grant access"
		);
		assert!(state.join_invite("synthetic".into()).is_none());
		for _ in 0..2 {
			state.apply(crate::Envelope {
				generation: state.generation,
				event: crate::Event::GuildJoined(model::Guild {
					stickers: None,
					id: model::Id(2),
					name: "Synthetic".into(),
					icon: None,
					emojis: None,
				}),
			});
		}
		assert_eq!(state.guilds.len(), 1);
		state.logout();
		assert!(state.invite_join.code.is_empty());
	}
}
