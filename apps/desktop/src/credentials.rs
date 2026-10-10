//! A single worker orders credential-store reads/writes/deletions, even during logout.
use client_core::auth::SessionSecret;
use eframe::egui;
use platform::CredentialError;
use std::collections::BTreeMap;
use std::sync::{
	Arc,
	mpsc::{self, Receiver, SyncSender},
};
use std::time::{Duration, Instant};

const LOAD_TIMEOUT: Duration = Duration::from_secs(10);

pub enum Operation {
	Load,
	/// One worker command removes launch restore and the signed-out account entry.
	Forget(Option<model::Id>),
	/// Per-account entries back the switcher; the active entry still drives launch restore.
	LoadAccount(model::Id),
	/// Save the keyed entry first; launch restore is replaced only after that succeeds.
	SaveAccount(model::Id, Arc<SessionSecret>, bool),
	/// Known account IDs allow ownership checks even after a failed switch signs out.
	ForgetAccount(model::Id, Vec<model::Id>),
}
pub enum Outcome {
	Loaded(Result<Option<SessionSecret>, CredentialError>),
	/// A per-account entry, so the roster can record that the entry now exists.
	AccountSaved(
		model::Id,
		Result<(), CredentialError>,
		Option<Result<(), CredentialError>>,
	),
	Forgotten(Option<model::Id>, Result<(), CredentialError>),
	/// Includes identity so removal is acknowledged before the roster is changed.
	AccountForgotten(model::Id, Result<(), CredentialError>),
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
	removing: BTreeMap<model::Id, Removal>,
}
struct Removal {
	generation: u64,
	/// A later deliberate sign-in keeps its roster/cache when the old deletion completes.
	superseded: bool,
}
impl Store {
	pub fn start(ctx: egui::Context) -> Self {
		let (send, commands) = mpsc::sync_channel::<(u64, Request, Operation)>(4);
		let (events, receive) = mpsc::sync_channel(4);
		std::thread::spawn(move || {
			while let Ok((generation, request, operation)) = commands.recv() {
				let outcome = match operation {
					Operation::Load => Outcome::Loaded(platform::load_session()),
					Operation::Forget(account) => Outcome::Forgotten(
						account,
						forget_both(
							account,
							platform::forget_session,
							platform::forget_account_session,
						),
					),
					Operation::LoadAccount(account) => {
						Outcome::Loaded(platform::load_account_session(account))
					}
					Operation::SaveAccount(account, secret, restore) => {
						let (keyed, launch) = save_account(
							restore,
							|| platform::save_account_session(account, &secret),
							|| platform::save_session(&secret),
							platform::forget_session,
						);
						Outcome::AccountSaved(account, keyed, launch)
					}
					Operation::ForgetAccount(account, known_accounts) => Outcome::AccountForgotten(
						account,
						forget_account(
							account,
							&known_accounts,
							platform::load_account_session,
							platform::load_session,
							platform::forget_session,
							platform::forget_account_session,
						),
					),
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
			removing: BTreeMap::new(),
		}
	}
	/// Coalesce repeated removals and keep accepted work visible until its acknowledgment.
	pub fn forget_account(
		&mut self,
		generation: u64,
		account: model::Id,
		known_accounts: impl IntoIterator<Item = model::Id>,
	) -> bool {
		if self
			.removing
			.get(&account)
			.is_some_and(|removal| removal.generation == generation)
		{
			return true;
		}
		let mut owners = Vec::with_capacity(model::MAX_SAVED_ACCOUNTS);
		for id in known_accounts {
			if id != account && !owners.contains(&id) {
				owners.push(id);
				if owners.len() == model::MAX_SAVED_ACCOUNTS {
					break;
				}
			}
		}
		if (!self.removing.contains_key(&account) && self.removing.len() >= 8)
			|| self
				.send
				.try_send((
					generation,
					Request::NONE,
					Operation::ForgetAccount(account, owners),
				))
				.is_err()
		{
			return false;
		}
		self.removing.insert(
			account,
			Removal {
				generation,
				superseded: false,
			},
		);
		true
	}
	pub fn save_account(
		&mut self,
		generation: u64,
		account: model::Id,
		secret: Arc<SessionSecret>,
		restore: bool,
	) -> bool {
		if self
			.send
			.try_send((
				generation,
				Request::NONE,
				Operation::SaveAccount(account, secret, restore),
			))
			.is_err()
		{
			return false;
		}
		if let Some(removal) = self.removing.get_mut(&account)
			&& removal.generation != generation
		{
			removal.superseded = true;
		}
		true
	}
	pub fn has_pending_removals(&self) -> bool {
		!self.removing.is_empty()
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
					if let Outcome::AccountForgotten(account, _) = &outcome {
						if self
							.removing
							.get(account)
							.is_some_and(|removal| removal.generation != generation)
						{
							continue;
						}
						if self
							.removing
							.remove(account)
							.is_some_and(|removal| removal.superseded)
						{
							continue;
						}
					}
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
					if let Some((account, removal)) = self.removing.pop_first() {
						if removal.superseded {
							continue;
						}
						return Some((
							removal.generation,
							Outcome::AccountForgotten(account, Err(CredentialError::Unavailable)),
						));
					}
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

fn forget_both(
	account: Option<model::Id>,
	forget_launch: impl FnOnce() -> Result<(), CredentialError>,
	forget_account: impl FnOnce(model::Id) -> Result<(), CredentialError>,
) -> Result<(), CredentialError> {
	// Attempt both even when the first fails; a single queue slot owns the whole deletion.
	let launch = forget_launch();
	let account = account.map_or(Ok(()), forget_account);
	launch.and(account)
}

fn save_account(
	restore: bool,
	save_keyed: impl FnOnce() -> Result<(), CredentialError>,
	save_launch: impl FnOnce() -> Result<(), CredentialError>,
	forget_launch: impl FnOnce() -> Result<(), CredentialError>,
) -> (
	Result<(), CredentialError>,
	Option<Result<(), CredentialError>>,
) {
	let keyed = save_keyed();
	let launch = (restore && keyed.is_ok()).then(|| {
		let saved = save_launch();
		if saved.is_err() {
			// The owner explicitly replaced launch restore. Leaving its old token
			// after updating the keyed entry would lose the ownership comparison used
			// by ForgetAccount. Preserve keyed logins, and retire only stale restore.
			forget_launch()?;
		}
		saved
	});
	(keyed, launch)
}

/// Device-scoped removal survives a switch, unless signing back into the same
/// account explicitly replaced the old session that requested its removal.
pub fn account_removal_applies(
	requested_generation: u64,
	current_generation: u64,
	account: model::Id,
	active_account: Option<model::Id>,
) -> bool {
	requested_generation == current_generation || active_account != Some(account)
}

fn forget_account(
	account: model::Id,
	known_accounts: &[model::Id],
	mut load_account: impl FnMut(model::Id) -> Result<Option<SessionSecret>, CredentialError>,
	load_launch: impl FnOnce() -> Result<Option<SessionSecret>, CredentialError>,
	forget_launch: impl FnOnce() -> Result<(), CredentialError>,
	forget_account: impl FnOnce(model::Id) -> Result<(), CredentialError>,
) -> Result<(), CredentialError> {
	// The launch entry can still belong to an inactive account after "Add account".
	// Read both before deletion so a keyring failure leaves enough information to retry.
	let saved = match load_account(account) {
		Ok(saved) => saved,
		Err(CredentialError::Invalid) => None,
		Err(error) => return Err(error),
	};
	let launch = match load_launch() {
		Ok(launch) => launch,
		// An invalid launch value cannot restore any account, so it is safe to remove.
		Err(CredentialError::Invalid) => {
			forget_launch()?;
			return forget_account(account);
		}
		Err(error) => return Err(error),
	};
	if saved
		.as_ref()
		.zip(launch.as_ref())
		.is_some_and(|(saved, launch)| saved.expose() == launch.expose())
	{
		forget_launch()?;
	}
	let mut unknown_owner = saved.is_none() && launch.is_some();
	if unknown_owner && let Some(launch) = &launch {
		// A failed switch clears the active UI session. Check the bounded saved-account
		// roster as well, so its previous account can still identify launch restore.
		for known_account in known_accounts
			.iter()
			.copied()
			.take(model::MAX_SAVED_ACCOUNTS)
		{
			if known_account == account {
				continue;
			}
			let known = match load_account(known_account) {
				Ok(known) => known,
				Err(CredentialError::Invalid) => None,
				Err(error) => return Err(error),
			};
			if known
				.as_ref()
				.is_some_and(|known| known.expose() == launch.expose())
			{
				unknown_owner = false;
				break;
			}
		}
	}
	forget_account(account)?;
	// Older independently saved entries may be inconsistent. Keep the roster visible
	// for recovery instead of claiming the unidentified launch login has been removed.
	if unknown_owner {
		Err(CredentialError::Invalid)
	} else {
		Ok(())
	}
}

pub fn loaded_status(result: &Result<Option<SessionSecret>, CredentialError>) -> &'static str {
	match result {
		Ok(Some(_)) => "Saved login found; connecting to Discord",
		Ok(None) => "No saved login found. Sign in with Discord to save one.",
		Err(CredentialError::Invalid) => "Saved login is invalid. Sign in with Discord again.",
		Err(CredentialError::NoStore) => {
			"No OS keyring found; sign in each launch. Install GNOME Keyring or KWallet to stay signed in."
		}
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
	use std::cell::{Cell, RefCell};

	fn synthetic(marker: &str) -> SessionSecret {
		SessionSecret::from_owner_input(format!("SYNTHETIC_LOGIN_{marker}")).unwrap()
	}

	#[test]
	fn a_later_forget_is_ordered_after_resignin_and_an_old_ack_cannot_release_it() {
		let (send, commands) = mpsc::sync_channel(4);
		let (events, receive) = mpsc::sync_channel(4);
		let mut store = Store {
			send,
			receive,
			loading: None,
			next_request: 0,
			removing: BTreeMap::new(),
		};
		let account = model::Id(7);
		assert!(store.forget_account(1, account, None));
		assert!(store.save_account(2, account, Arc::new(synthetic("ACCOUNT_A")), true));
		assert!(store.forget_account(3, account, None));
		assert!(matches!(
			commands.try_recv().unwrap().2,
			Operation::ForgetAccount(_, _)
		));
		assert!(matches!(
			commands.try_recv().unwrap().2,
			Operation::SaveAccount(_, _, true)
		));
		assert!(matches!(
			commands.try_recv().unwrap().2,
			Operation::ForgetAccount(_, _)
		));
		assert!(
			events
				.send((1, Request::NONE, Outcome::AccountForgotten(account, Ok(()))))
				.is_ok()
		);
		assert!(store.poll(Instant::now()).is_none());
		assert!(store.has_pending_removals());
		assert!(
			events
				.send((3, Request::NONE, Outcome::AccountForgotten(account, Ok(()))))
				.is_ok()
		);
		assert!(matches!(
			store.poll(Instant::now()),
			Some((3, Outcome::AccountForgotten(_, Ok(()))))
		));
		assert!(!store.has_pending_removals());
		assert!(store.forget_account(4, account, None));
		assert!(store.save_account(5, account, Arc::new(synthetic("ACCOUNT_A")), true));
		assert!(
			events
				.send((4, Request::NONE, Outcome::AccountForgotten(account, Ok(()))))
				.is_ok()
		);
		assert!(
			store.poll(Instant::now()).is_none(),
			"a new deliberate sign-in keeps its roster/cache"
		);
		assert!(!store.has_pending_removals());
	}

	#[test]
	fn accepted_account_removals_are_bounded_coalesced_and_acknowledged_across_switches() {
		let (send, commands) = mpsc::sync_channel(4);
		let (events, receive) = mpsc::sync_channel(4);
		let mut store = Store {
			send,
			receive,
			loading: None,
			next_request: 0,
			removing: BTreeMap::new(),
		};
		assert!(store.forget_account(1, model::Id(7), None));
		assert!(store.has_pending_removals());
		assert!(store.forget_account(1, model::Id(7), None));
		assert!(matches!(
			commands.try_recv().unwrap().2,
			Operation::ForgetAccount(model::Id(7), _)
		));
		assert!(
			commands.try_recv().is_err(),
			"duplicate clicks do not enqueue another operation"
		);
		assert!(
			events
				.send((
					1,
					Request::NONE,
					Outcome::AccountForgotten(model::Id(7), Ok(()))
				))
				.is_ok()
		);
		assert!(matches!(
			store.poll(Instant::now()),
			Some((1, Outcome::AccountForgotten(model::Id(7), Ok(()))))
		));
		assert!(!store.has_pending_removals());
		for id in 1..=8 {
			assert!(store.forget_account(2, model::Id(id), None));
			let _ = commands.try_recv().unwrap();
		}
		assert!(!store.forget_account(2, model::Id(9), None));
		drop(events);
		for _ in 0..8 {
			assert!(matches!(
				store.poll(Instant::now()),
				Some((
					2,
					Outcome::AccountForgotten(_, Err(CredentialError::Unavailable))
				))
			));
		}
		assert!(!store.has_pending_removals());
		for generation in 0..4 {
			assert!(
				store
					.send
					.try_send((generation, Request::NONE, Operation::Load))
					.is_ok()
			);
		}
		assert!(!store.forget_account(3, model::Id(10), None));
		assert!(
			!store.has_pending_removals(),
			"rejected work is never reported as pending"
		);
	}

	#[test]
	fn launch_save_requires_a_successful_keyed_save_in_the_same_operation() {
		let launch = Cell::new(false);
		assert_eq!(
			save_account(
				true,
				|| Err(CredentialError::Unavailable),
				|| {
					launch.set(true);
					Ok(())
				},
				|| panic!("keyed save failed; launch must stay untouched")
			),
			(Err(CredentialError::Unavailable), None)
		);
		assert!(!launch.get());
		assert_eq!(
			save_account(
				false,
				|| Ok(()),
				|| panic!("unchanged launch"),
				|| panic!("unchanged launch")
			),
			(Ok(()), None)
		);
		assert_eq!(
			save_account(
				true,
				|| Ok(()),
				|| {
					launch.set(true);
					Ok(())
				},
				|| panic!("successful replacement cannot clear restore")
			),
			(Ok(()), Some(Ok(())))
		);
		assert!(launch.get());
		assert_eq!(
			save_account(
				true,
				|| Ok(()),
				|| Err(CredentialError::Unavailable),
				|| {
					launch.set(false);
					Ok(())
				}
			),
			(Ok(()), Some(Err(CredentialError::Unavailable)))
		);
		assert!(
			!launch.get(),
			"failed replacement retires stale launch restore"
		);
	}

	#[test]
	fn failed_launch_replacement_never_removes_keyed_logins_and_reports_cleanup_failure() {
		let keyed = Cell::new(false);
		let cleanup = Cell::new(false);
		assert_eq!(
			save_account(
				true,
				|| {
					keyed.set(true);
					Ok(())
				},
				|| Err(CredentialError::Unavailable),
				|| {
					cleanup.set(true);
					Err(CredentialError::TimedOut)
				}
			),
			(Ok(()), Some(Err(CredentialError::TimedOut)))
		);
		assert!(keyed.get() && cleanup.get());
	}

	#[test]
	fn later_forget_cannot_restore_old_token_after_failed_launch_replacement() {
		let keyed = RefCell::new(Some("ACCOUNT_B_OLD"));
		let launch = RefCell::new(Some("ACCOUNT_B_OLD"));
		assert_eq!(
			save_account(
				true,
				|| {
					*keyed.borrow_mut() = Some("ACCOUNT_B_NEW");
					Ok(())
				},
				|| Err(CredentialError::Unavailable),
				|| {
					*launch.borrow_mut() = None;
					Ok(())
				}
			),
			(Ok(()), Some(Err(CredentialError::Unavailable)))
		);
		assert_eq!(*keyed.borrow(), Some("ACCOUNT_B_NEW"));
		// Add account keeps keyed sessions; forgetting B on the signed-out account
		// list must not leave B's previously valid launch token behind.
		assert_eq!(
			forget_account(
				model::Id(7),
				&[],
				|_| Ok(keyed.borrow().map(synthetic)),
				|| Ok(launch.borrow().map(synthetic)),
				|| panic!("failed replacement already cleared stale restore"),
				|_| {
					*keyed.borrow_mut() = None;
					Ok(())
				}
			),
			Ok(())
		);
		assert!(keyed.borrow().is_none() && launch.borrow().is_none());
	}

	#[test]
	fn malformed_saved_values_can_be_deleted_without_a_valid_session() {
		let keyed = Cell::new(false);
		assert_eq!(
			forget_account(
				model::Id(7),
				&[],
				|_| Err(CredentialError::Invalid),
				|| Ok(None),
				|| panic!("no launch value"),
				|_| {
					keyed.set(true);
					Ok(())
				}
			),
			Ok(())
		);
		assert!(keyed.get());
		let launch = Cell::new(false);
		assert_eq!(
			forget_account(
				model::Id(7),
				&[],
				|_| Ok(Some(synthetic("ACCOUNT_A"))),
				|| Err(CredentialError::Invalid),
				|| {
					launch.set(true);
					Ok(())
				},
				|_| Ok(())
			),
			Ok(())
		);
		assert!(launch.get());
	}
	#[test]
	fn account_removal_survives_a_switch_but_not_a_new_session_of_that_account() {
		assert!(account_removal_applies(
			1,
			1,
			model::Id(7),
			Some(model::Id(7))
		));
		assert!(account_removal_applies(
			1,
			2,
			model::Id(7),
			Some(model::Id(8))
		));
		assert!(account_removal_applies(1, 2, model::Id(7), None));
		assert!(!account_removal_applies(
			1,
			2,
			model::Id(7),
			Some(model::Id(7))
		));
	}

	#[test]
	fn forgetting_inactive_account_clears_only_its_launch_restore() {
		for matches in [false, true] {
			let launch_removed = Cell::new(false);
			let account_removed = Cell::new(false);
			let account = model::Id(7);
			assert_eq!(
				forget_account(
					account,
					&[],
					|id| {
						assert_eq!(id, account);
						Ok(Some(synthetic("ACCOUNT_A")))
					},
					|| Ok(Some(synthetic(if matches {
						"ACCOUNT_A"
					} else {
						"ACCOUNT_B"
					}))),
					|| {
						launch_removed.set(true);
						Ok(())
					},
					|id| {
						assert_eq!(id, account);
						account_removed.set(true);
						Ok(())
					},
				),
				Ok(())
			);
			assert_eq!(launch_removed.get(), matches);
			assert!(account_removed.get());
		}
	}

	#[test]
	fn missing_account_entry_does_not_delete_an_unidentified_launch_token() {
		let account_removed = Cell::new(false);
		assert_eq!(
			forget_account(
				model::Id(7),
				&[],
				|_| Ok(None),
				|| Ok(Some(synthetic("OTHER_ACCOUNT"))),
				|| panic!("cannot establish launch ownership"),
				|_| {
					account_removed.set(true);
					Ok(())
				},
			),
			Err(CredentialError::Invalid)
		);
		assert!(account_removed.get());
	}

	#[test]
	fn missing_or_invalid_account_can_be_forgotten_when_another_known_account_owns_launch() {
		for invalid in [false, true] {
			let account = model::Id(7);
			let other = model::Id(8);
			let reads = RefCell::new(Vec::new());
			let removed = Cell::new(false);
			assert_eq!(
				forget_account(
					account,
					&[account, other],
					|id| {
						reads.borrow_mut().push(id);
						if id == account {
							if invalid {
								Err(CredentialError::Invalid)
							} else {
								Ok(None)
							}
						} else {
							assert_eq!(id, other);
							Ok(Some(synthetic("OTHER_ACCOUNT")))
						}
					},
					|| Ok(Some(synthetic("OTHER_ACCOUNT"))),
					|| panic!("another account owns launch restore; preserve it"),
					|id| {
						assert_eq!(id, account);
						removed.set(true);
						Ok(())
					},
				),
				Ok(())
			);
			assert_eq!(*reads.borrow(), [account, other]);
			assert!(
				removed.get(),
				"success lets the caller remove the roster and cache"
			);
		}
	}

	#[test]
	fn unrelated_missing_or_invalid_logins_do_not_identify_launch_ownership() {
		for known in [
			Ok(None),
			Err(CredentialError::Invalid),
			Ok(Some(synthetic("UNRELATED"))),
		] {
			let account = model::Id(7);
			let other = model::Id(8);
			let mut known = Some(known);
			assert_eq!(
				forget_account(
					account,
					&[other],
					|id| if id == account {
						Ok(None)
					} else {
						known.take().unwrap()
					},
					|| Ok(Some(synthetic("UNIDENTIFIED"))),
					|| panic!("unidentified launch login must be preserved"),
					|id| {
						assert_eq!(id, account);
						Ok(())
					},
				),
				Err(CredentialError::Invalid)
			);
		}
	}

	#[test]
	fn failed_known_account_lookup_keeps_the_missing_account_for_retry() {
		for error in [CredentialError::Unavailable, CredentialError::TimedOut] {
			let account = model::Id(7);
			assert_eq!(
				forget_account(
					account,
					&[model::Id(8)],
					|id| if id == account { Ok(None) } else { Err(error) },
					|| Ok(Some(synthetic("OTHER_ACCOUNT"))),
					|| panic!("failed lookup cannot justify launch deletion"),
					|_| panic!("failed lookup must remain retryable"),
				),
				Err(error)
			);
		}
	}

	#[test]
	fn queued_ownership_candidates_exclude_the_forgotten_account_and_are_unique_and_bounded() {
		let (send, commands) = mpsc::sync_channel(4);
		let (_, receive) = mpsc::sync_channel(4);
		let mut store = Store {
			send,
			receive,
			loading: None,
			next_request: 0,
			removing: BTreeMap::new(),
		};
		let known = [model::Id(1), model::Id(2), model::Id(2)]
			.into_iter()
			.chain((3..=20).map(model::Id));
		assert!(store.forget_account(1, model::Id(1), known));
		let (_, _, Operation::ForgetAccount(account, known)) = commands.try_recv().unwrap() else {
			panic!("expected account removal");
		};
		assert_eq!(account, model::Id(1));
		assert_eq!(known, (2..=9).map(model::Id).collect::<Vec<_>>());
	}

	#[test]
	fn failed_credential_reads_or_launch_deletion_keep_the_keyed_entry_for_retry() {
		for failure in 0..3 {
			assert_eq!(
				forget_account(
					model::Id(7),
					&[],
					|_| if failure == 0 {
						Err(CredentialError::Unavailable)
					} else {
						Ok(Some(synthetic("ACCOUNT_A")))
					},
					|| if failure == 1 {
						Err(CredentialError::Unavailable)
					} else {
						Ok(Some(synthetic("ACCOUNT_A")))
					},
					|| Err(CredentialError::Unavailable),
					|_| panic!("retain keyed ownership on failure"),
				),
				Err(CredentialError::Unavailable)
			);
		}
	}

	#[test]
	fn logout_attempts_both_entries_even_when_one_delete_fails() {
		let keyed_removed = Cell::new(false);
		assert_eq!(
			forget_both(
				Some(model::Id(7)),
				|| Err(CredentialError::Unavailable),
				|_| {
					keyed_removed.set(true);
					Ok(())
				}
			),
			Err(CredentialError::Unavailable)
		);
		assert!(keyed_removed.get());
		assert_eq!(
			forget_both(None, || Ok(()), |_| panic!("no account entry")),
			Ok(())
		);
		assert_eq!(
			forget_both(
				Some(model::Id(7)),
				|| Ok(()),
				|_| Err(CredentialError::Unavailable)
			),
			Err(CredentialError::Unavailable)
		);
	}

	#[test]
	fn logout_needs_one_remaining_command_slot_for_both_credentials() {
		let (send, receive) = mpsc::sync_channel(4);
		for generation in 0..3 {
			assert!(
				send.try_send((generation, Request::NONE, Operation::Load))
					.is_ok()
			);
		}
		assert!(
			send.try_send((3, Request::NONE, Operation::Forget(Some(model::Id(7)))))
				.is_ok()
		);
		assert!(send.try_send((4, Request::NONE, Operation::Load)).is_err());
		for _ in 0..3 {
			let _ = receive.recv().unwrap();
		}
		assert!(matches!(
			receive.recv().unwrap().2,
			Operation::Forget(Some(model::Id(7)))
		));
	}

	#[test]
	fn saved_lookup_finishes_times_out_and_ignores_late_results() {
		let (send, commands) = mpsc::sync_channel(4);
		let (events, receive) = mpsc::sync_channel(4);
		let mut store = Store {
			send,
			receive,
			loading: None,
			next_request: 0,
			removing: BTreeMap::new(),
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
