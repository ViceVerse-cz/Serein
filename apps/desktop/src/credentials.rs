//! A single worker orders credential-store reads/writes/deletions, even during logout.
use client_core::auth::SessionSecret;
use eframe::egui;
use platform::CredentialError;
use std::sync::{
	Arc,
	mpsc::{self, Receiver, SyncSender},
};
use std::time::{Duration, Instant};

const LOAD_TIMEOUT: Duration = Duration::from_secs(10);

pub enum Operation {
	Load,
	Save(Arc<SessionSecret>),
	Forget,
	/// Per-account entries back the switcher; the active entry still drives launch restore.
	LoadAccount(model::Id),
	SaveAccount(model::Id, Arc<SessionSecret>),
	ForgetAccount(model::Id),
}
pub enum Outcome {
	Loaded(Result<Option<SessionSecret>, CredentialError>),
	Saved(Result<(), CredentialError>),
	/// A per-account entry, so the roster can record that the entry now exists.
	AccountSaved(model::Id, Result<(), CredentialError>),
	Forgotten(Result<(), CredentialError>),
	/// A per-account entry; never touches the launch-restore status or `forgetting`.
	AccountForgotten(Result<(), CredentialError>),
}
/// Identifies one queued credential operation. Writes and deletions use `Request::NONE`,
/// which never matches a read in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request(u64);
impl Request {
	pub const NONE: Self = Self(0);
}
pub struct Store {
	pub send: SyncSender<(u64, Request, Operation)>,
	receive: Receiver<(u64, Request, Outcome)>,
	/// The read in flight: its request, the session it belongs to and its deadline. The
	/// request distinguishes a timed-out read from the one that replaced it, which a
	/// generation alone cannot once a switch reuses the session it just started.
	loading: Option<(u64, Request, Instant)>,
	next_request: u64,
}
impl Store {
	pub fn start(ctx: egui::Context) -> Self {
		let (send, commands) = mpsc::sync_channel::<(u64, Request, Operation)>(4);
		let (events, receive) = mpsc::sync_channel(4);
		std::thread::spawn(move || {
			while let Ok((generation, request, operation)) = commands.recv() {
				let outcome = match operation {
					Operation::Load => Outcome::Loaded(platform::load_session()),
					Operation::Save(secret) => Outcome::Saved(platform::save_session(&secret)),
					Operation::Forget => Outcome::Forgotten(platform::forget_session()),
					Operation::LoadAccount(account) => {
						Outcome::Loaded(platform::load_account_session(account))
					}
					Operation::SaveAccount(account, secret) => Outcome::AccountSaved(
						account,
						platform::save_account_session(account, &secret),
					),
					Operation::ForgetAccount(account) => {
						Outcome::AccountForgotten(platform::forget_account_session(account))
					}
				};
				if events.send((generation, request, outcome)).is_err() {
					break;
				}
				ctx.request_repaint();
			}
		});
		Self {
			send,
			receive,
			loading: None,
			next_request: 0,
		}
	}
	pub fn load(&mut self, generation: u64, now: Instant) -> bool {
		self.begin_load(generation, Operation::Load, now)
	}
	fn next_request(&mut self) -> Request {
		// Skips NONE on wrap, so a write can never answer a read.
		self.next_request = self.next_request.wrapping_add(1).max(1);
		Request(self.next_request)
	}
	/// Reads one saved account's token for an account switch, under the same timeout.
	pub fn load_account(&mut self, generation: u64, account: model::Id, now: Instant) -> bool {
		self.begin_load(generation, Operation::LoadAccount(account), now)
	}
	fn begin_load(&mut self, generation: u64, operation: Operation, now: Instant) -> bool {
		let request = self.next_request();
		if self
			.send
			.try_send((generation, request, operation))
			.is_err()
		{
			return false;
		}
		self.loading = Some((generation, request, now + LOAD_TIMEOUT));
		true
	}
	pub fn cancel_load(&mut self) {
		self.loading = None;
	}
	pub fn remaining(&self, now: Instant) -> Option<Duration> {
		self.loading
			.map(|(_, _, deadline)| deadline.saturating_duration_since(now))
	}
	pub fn poll(&mut self, now: Instant) -> Option<(u64, Outcome)> {
		if let Some((generation, _, deadline)) = self.loading
			&& now >= deadline
		{
			self.loading = None;
			return Some((generation, Outcome::Loaded(Err(CredentialError::TimedOut))));
		}
		for _ in 0..4 {
			match self.receive.try_recv() {
				Ok((generation, request, outcome)) => {
					if matches!(outcome, Outcome::Loaded(_)) {
						// A read answers only the request that is still waiting: a token that
						// arrives after its own timeout must never satisfy a later read.
						if !self
							.loading
							.is_some_and(|(_, current, _)| current == request)
						{
							continue;
						}
						self.loading = None;
					}
					return Some((generation, outcome));
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					return self.loading.take().map(|(generation, _, _)| {
						(
							generation,
							Outcome::Loaded(Err(CredentialError::Unavailable)),
						)
					});
				}
				Err(mpsc::TryRecvError::Empty) => return None,
			}
		}
		None
	}
}

pub fn loaded_status(result: &Result<Option<SessionSecret>, CredentialError>) -> &'static str {
	match result {
		Ok(Some(_)) => "Saved login found; connecting to Discord",
		Ok(None) => "No saved login found. Sign in with Discord to save one.",
		Err(CredentialError::Invalid) => "Saved login is invalid. Sign in with Discord again.",
		Err(CredentialError::Unavailable) => {
			"Saved login unavailable; sign in with Discord. No plaintext fallback."
		}
		Err(CredentialError::TimedOut) => {
			"Saved-login check timed out. Sign in with Discord; the credential store did not respond."
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn saved_lookup_finishes_times_out_and_ignores_late_results() {
		let (send, commands) = mpsc::sync_channel(4);
		let (events, receive) = mpsc::sync_channel(4);
		let mut store = Store {
			send,
			receive,
			loading: None,
			next_request: 0,
		};
		let request =
			|commands: &Receiver<(u64, Request, Operation)>| commands.try_recv().unwrap().1;
		let now = Instant::now();
		assert!(store.load(1, now));
		let first = request(&commands);
		assert!(store.poll(now).is_none());
		assert_eq!(store.remaining(now), Some(LOAD_TIMEOUT));
		assert!(events.send((1, first, Outcome::Loaded(Ok(None)))).is_ok());
		let (_, Outcome::Loaded(result)) = store.poll(now).unwrap() else {
			panic!()
		};
		assert!(loaded_status(&result).contains("No saved login"));
		assert!(store.remaining(now).is_none());
		assert!(store.load(2, now));
		let timed_out = request(&commands);
		let (_, Outcome::Loaded(result)) = store.poll(now + LOAD_TIMEOUT).unwrap() else {
			panic!()
		};
		assert_eq!(result.err(), Some(CredentialError::TimedOut));
		let synthetic = || SessionSecret::from_owner_input("SYNTHETIC_SAVED_LOGIN".into()).unwrap();
		assert!(
			events
				.send((2, timed_out, Outcome::Loaded(Ok(Some(synthetic())))))
				.is_ok()
		);
		assert!(store.poll(now + LOAD_TIMEOUT).is_none());
		// A read that timed out cannot answer the read that replaced it, even when a switch
		// reuses the same session: the token belongs to whichever account was asked for first.
		assert!(store.load(2, now));
		let current = request(&commands);
		assert_ne!(current, timed_out);
		assert!(
			events
				.send((2, timed_out, Outcome::Loaded(Ok(Some(synthetic())))))
				.is_ok()
		);
		assert!(store.poll(now).is_none());
		assert_eq!(store.remaining(now), Some(LOAD_TIMEOUT));
		assert!(
			events
				.send((2, current, Outcome::Loaded(Ok(Some(synthetic())))))
				.is_ok()
		);
		assert!(matches!(
			store.poll(now),
			Some((2, Outcome::Loaded(Ok(Some(_)))))
		));
		assert!(store.load(3, now));
		let cancelled = request(&commands);
		store.cancel_load();
		assert!(
			events
				.send((3, cancelled, Outcome::Loaded(Ok(Some(synthetic())))))
				.is_ok()
		);
		assert!(store.poll(now).is_none());
		assert!(loaded_status(&Ok(Some(synthetic()))).contains("found"));
		assert!(loaded_status(&Err(CredentialError::Invalid)).contains("invalid"));
		assert!(store.load(4, now));
		drop(events);
		assert!(matches!(
			store.poll(now),
			Some((4, Outcome::Loaded(Err(CredentialError::Unavailable))))
		));
	}
}
