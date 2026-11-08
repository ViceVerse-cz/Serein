//! Friend invite links for the account: list, create and revoke them from Add Friend.
//!
//! One request runs at a time and nothing is retried automatically. Links are kept only
//! for the session and cleared when the session ends.
use crate::{State, auth::Failure};

/// Friend invite links shown at once; the service keeps few, and extras are dropped.
pub const MAX_INVITES: usize = 32;

/// One friend invite link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendInvite {
	/// The invite code; the link is `https://discord.gg/<code>`.
	pub code: String,
	/// RFC 3339 expiry, if the link expires.
	pub expires_at: Option<String>,
	/// Uses so far, and the limit (zero for unlimited).
	pub uses: u32,
	pub max_uses: u32,
}

impl FriendInvite {
	/// The shareable link for this invite.
	pub fn link(&self) -> String {
		format!("https://discord.gg/{}", self.code)
	}
}

/// A friend invite request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
	/// Lists the account's friend invite links.
	Load { request: u64 },
	/// Creates a new link with the service's defaults.
	Create { request: u64 },
	/// Revokes every friend invite link.
	RevokeAll { request: u64 },
}

/// The result of a [`Command`].
pub enum Event {
	Loaded {
		request: u64,
		result: Result<Vec<FriendInvite>, Failure>,
	},
	Created {
		request: u64,
		result: Result<FriendInvite, Failure>,
	},
	Revoked {
		request: u64,
		result: Result<(), Failure>,
	},
}

/// The account's friend invite links and the one request in flight.
#[derive(Default)]
pub struct FriendInvites {
	/// `None` until the first list arrives.
	invites: Option<Vec<FriendInvite>>,
	pending: Option<u64>,
	sequence: u64,
	/// The last failure, for one line under the links.
	pub error: Option<Failure>,
}

impl FriendInvites {
	/// Links listed so far, newest first; `None` before the first list.
	pub fn invites(&self) -> Option<&[FriendInvite]> {
		self.invites.as_deref()
	}
	/// Forgets the request in flight when the connection fails, so the user can ask again.
	pub(crate) fn release_pending(&mut self) {
		self.pending = None;
	}
	/// Whether a request is in flight.
	pub fn busy(&self) -> bool {
		self.pending.is_some()
	}
}

impl State {
	fn friend_invite_command(&mut self, make: fn(u64) -> Command) -> Option<crate::Command> {
		if self.demo
			|| self.auth != crate::auth::AuthState::Authenticated
			|| self.friend_invites.pending.is_some()
		{
			return None;
		}
		let invites = &mut self.friend_invites;
		invites.sequence += 1;
		invites.pending = Some(invites.sequence);
		invites.error = None;
		Some(crate::Command::FriendInvites(make(invites.sequence)))
	}
	/// Lists the account's friend invite links, once per visit to Add Friend.
	pub fn load_friend_invites(&mut self) -> Option<crate::Command> {
		self.friend_invite_command(|request| Command::Load { request })
	}
	/// Creates a new friend invite link.
	pub fn create_friend_invite(&mut self) -> Option<crate::Command> {
		self.friend_invite_command(|request| Command::Create { request })
	}
	/// Revokes every friend invite link; only offered while links exist.
	pub fn revoke_friend_invites(&mut self) -> Option<crate::Command> {
		if self
			.friend_invites
			.invites
			.as_ref()
			.is_none_or(Vec::is_empty)
		{
			return None;
		}
		self.friend_invite_command(|request| Command::RevokeAll { request })
	}
	pub(crate) fn apply_friend_invites(&mut self, event: Event) {
		let invites = &mut self.friend_invites;
		let request = match &event {
			Event::Loaded { request, .. }
			| Event::Created { request, .. }
			| Event::Revoked { request, .. } => *request,
		};
		if invites.pending != Some(request) {
			return;
		}
		invites.pending = None;
		match event {
			Event::Loaded { result, .. } => match result {
				Ok(mut list) => {
					list.truncate(MAX_INVITES);
					invites.invites = Some(list);
				}
				Err(failure) => invites.error = Some(failure),
			},
			Event::Created { result, .. } => match result {
				// Before the first list, the new link arrives with the list that follows.
				Ok(invite) => {
					if let Some(list) = &mut invites.invites {
						list.retain(|known| known.code != invite.code);
						list.insert(0, invite);
						list.truncate(MAX_INVITES);
					}
				}
				Err(failure) => invites.error = Some(failure),
			},
			Event::Revoked { result, .. } => match result {
				Ok(()) => invites.invites = Some(Vec::new()),
				Err(failure) => invites.error = Some(failure),
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::auth::AuthState;

	fn invite(code: &str) -> FriendInvite {
		FriendInvite {
			code: code.into(),
			expires_at: None,
			uses: 0,
			max_uses: 5,
		}
	}

	fn request(command: Option<crate::Command>) -> u64 {
		match command {
			Some(crate::Command::FriendInvites(
				Command::Load { request }
				| Command::Create { request }
				| Command::RevokeAll { request },
			)) => request,
			_ => panic!("expected a friend invite command"),
		}
	}

	#[test]
	fn one_request_at_a_time_and_stale_results_are_ignored() {
		let mut state = State {
			auth: AuthState::Authenticated,
			..Default::default()
		};
		assert!(
			state.revoke_friend_invites().is_none(),
			"nothing to revoke yet"
		);
		let load = request(state.load_friend_invites());
		assert!(state.create_friend_invite().is_none(), "busy");
		state.apply_friend_invites(Event::Loaded {
			request: load + 1,
			result: Ok(vec![invite("stale")]),
		});
		assert!(state.friend_invites.busy());
		state.apply_friend_invites(Event::Loaded {
			request: load,
			result: Ok(vec![invite("old")]),
		});
		let create = request(state.create_friend_invite());
		state.apply_friend_invites(Event::Created {
			request: create,
			result: Ok(invite("new")),
		});
		let codes: Vec<_> = state
			.friend_invites
			.invites()
			.unwrap()
			.iter()
			.map(|invite| invite.code.as_str())
			.collect();
		assert_eq!(codes, ["new", "old"]);
		assert_eq!(
			state.friend_invites.invites().unwrap()[0].link(),
			"https://discord.gg/new"
		);
		let revoke = request(state.revoke_friend_invites());
		state.apply_friend_invites(Event::Revoked {
			request: revoke,
			result: Err(Failure::Network),
		});
		assert_eq!(state.friend_invites.error, Some(Failure::Network));
		assert_eq!(
			state.friend_invites.invites().unwrap().len(),
			2,
			"kept on failure"
		);
		let revoke = request(state.revoke_friend_invites());
		state.apply_friend_invites(Event::Revoked {
			request: revoke,
			result: Ok(()),
		});
		assert_eq!(state.friend_invites.invites(), Some(&[][..]));
	}

	#[test]
	fn signed_out_and_demo_sessions_send_nothing() {
		let mut state = State::default();
		assert!(state.load_friend_invites().is_none());
		state.auth = AuthState::Authenticated;
		state.demo = true;
		assert!(state.create_friend_invite().is_none());
	}
}
