//! One pending user-solved challenge, either an invite join or a friendship write.
use crate::{Command, State, captcha};

impl State {
	/// The single challenge currently waiting on the user, if any.
	pub fn verification(&self) -> Option<(captcha::Verification, &captcha::Challenge)> {
		if let Some((request, challenge)) = self.invite_challenge() {
			return Some((captcha::Verification::Invite { request }, challenge));
		}
		let (request, challenge) = self.friend_challenge()?;
		Some((captcha::Verification::Friend { request }, challenge))
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
			captcha::Verification::Friend { request } => {
				self.resume_friend_challenge(request, solution)
			}
		}
	}
	/// Cancels the pending challenge and releases its write.
	pub fn cancel_verification(&mut self, verification: captcha::Verification) {
		match verification {
			captcha::Verification::Invite { request } => self.cancel_invite_challenge(request),
			captcha::Verification::Friend { request } => self.cancel_friend_challenge(request),
		}
	}
	/// Releases any challenge that outlived its lifetime.
	pub fn expire_verification(&mut self) {
		self.expire_invite_challenge();
		self.expire_friend_challenge();
	}
}
