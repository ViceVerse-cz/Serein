//! One explicitly requested, read-only account standing read. No polling and no persistence.
use crate::{Command, State, auth};
use model::safety_hub::Snapshot;

#[derive(Default)]
pub struct View {
	pub snapshot: Option<Snapshot>,
	pub loading: bool,
	pub error: Option<&'static str>,
	request: u64,
	generation: u64,
}

impl State {
	/// Starts at most one explicit Safety Hub read; opening the panel alone never requests it.
	pub fn load_safety_hub(&mut self) -> Option<Command> {
		if self.safety_hub.loading {
			return None;
		}
		if !self.demo && (self.auth != auth::AuthState::Authenticated || !self.gateway_connected) {
			self.safety_hub.error = Some("Reconnect to check account standing");
			return None;
		}
		self.safety_hub.request = self.safety_hub.request.wrapping_add(1);
		self.safety_hub.generation = self.generation;
		self.safety_hub.loading = true;
		self.safety_hub.error = None;
		Some(Command::SafetyHub {
			request: self.safety_hub.request,
		})
	}
	fn pending(&self, request: u64) -> bool {
		self.safety_hub.request == request
			&& self.safety_hub.generation == self.generation
			&& self.safety_hub.loading
	}
	pub(crate) fn reject_safety_hub(&mut self, request: u64) {
		if self.pending(request) {
			self.safety_hub.loading = false;
			self.safety_hub.error = Some("Account standing request was not queued; try again");
		}
	}
	pub(crate) fn apply_safety_hub(
		&mut self,
		request: u64,
		result: Result<Snapshot, auth::Failure>,
	) {
		if !self.pending(request) {
			return;
		}
		self.safety_hub.loading = false;
		match result {
			Ok(snapshot) if snapshot.valid() => {
				self.safety_hub.snapshot = Some(snapshot);
				self.safety_hub.error = None;
			}
			Ok(_) => {
				// A malformed or oversized response is unavailable, never All Good.
				self.safety_hub.snapshot = None;
				self.safety_hub.error = Some("Safety Hub response was invalid or too large");
			}
			Err(failure) => {
				self.safety_hub.snapshot = None;
				self.safety_hub.error = Some(failure.label());
				if failure.ends_session() && failure != auth::Failure::Capacity {
					self.fail(failure);
				}
			}
		}
	}
	/// A lost session clears the service answer and rejects any in-flight generation.
	pub(crate) fn disconnect_safety_hub(&mut self) {
		self.safety_hub = View {
			request: self.safety_hub.request.wrapping_add(1),
			error: Some("Reconnect to check account standing"),
			..Default::default()
		};
	}
	pub(crate) fn interrupt_safety_hub(&mut self) {
		if self.safety_hub.loading || self.safety_hub.snapshot.is_some() {
			self.safety_hub.error =
				Some("Account standing was interrupted; reconnect and check again");
		}
		self.safety_hub = View {
			request: self.safety_hub.request.wrapping_add(1),
			error: self.safety_hub.error,
			..Default::default()
		};
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event, auth::Failure};
	use model::safety_hub::{AccountStanding, Snapshot};

	fn state() -> State {
		State {
			auth: auth::AuthState::Authenticated,
			gateway_connected: true,
			demo: false,
			..Default::default()
		}
	}
	fn apply(state: &mut State, event: Event) {
		let generation = state.generation;
		state.apply(Envelope { generation, event });
	}

	#[test]
	fn loading_is_explicit_single_flight_and_rejects_stale_requests() {
		let mut state = state();
		assert!(state.safety_hub.snapshot.is_none());
		let Some(Command::SafetyHub { request }) = state.load_safety_hub() else {
			panic!("explicit request")
		};
		assert!(state.safety_hub.loading);
		assert!(state.load_safety_hub().is_none(), "one in flight only");

		state.apply_safety_hub(
			request,
			Ok(Snapshot {
				standing: Some(AccountStanding::AllGood),
				..Default::default()
			}),
		);
		assert_eq!(
			state.safety_hub.snapshot.as_ref().unwrap().standing,
			Some(AccountStanding::AllGood)
		);

		let Some(Command::SafetyHub { request: stale }) = state.load_safety_hub() else {
			panic!("second explicit request")
		};
		assert_ne!(stale, request);
		state.apply_safety_hub(
			request,
			Ok(Snapshot {
				standing: Some(AccountStanding::Suspended),
				..Default::default()
			}),
		);
		assert!(
			state.safety_hub.loading,
			"an older request cannot settle the new one"
		);
		assert_eq!(
			state.safety_hub.snapshot.as_ref().unwrap().standing,
			Some(AccountStanding::AllGood)
		);
	}

	#[test]
	fn failure_missing_standing_and_disconnect_are_unavailable_not_all_good() {
		let mut state = state();
		let Some(Command::SafetyHub { request }) = state.load_safety_hub() else {
			panic!("explicit request")
		};
		apply(
			&mut state,
			Event::SafetyHub {
				request,
				result: Err(Failure::Network),
			},
		);
		assert!(state.safety_hub.snapshot.is_none());
		assert!(state.safety_hub.error.is_some());

		let Some(Command::SafetyHub { request }) = state.load_safety_hub() else {
			panic!("retry")
		};
		state.apply_safety_hub(request, Ok(Snapshot::default()));
		assert!(state.safety_hub.snapshot.is_some());
		assert_eq!(state.safety_hub.snapshot.as_ref().unwrap().standing, None);

		apply(&mut state, Event::Disconnected);
		assert!(state.safety_hub.snapshot.is_none());
		assert!(!state.safety_hub.loading);

		state.safety_hub.snapshot = Some(Snapshot {
			standing: Some(AccountStanding::AllGood),
			..Default::default()
		});
		state.logout();
		assert!(state.safety_hub.snapshot.is_none());
		assert!(!state.safety_hub.loading);
	}

	#[test]
	fn a_generation_change_rejects_the_in_flight_answer() {
		let mut state = state();
		let Some(Command::SafetyHub { request }) = state.load_safety_hub() else {
			panic!("explicit request")
		};
		let old_generation = state.generation;
		state.generation = old_generation.wrapping_add(1);
		state.apply_safety_hub(
			request,
			Ok(Snapshot {
				standing: Some(AccountStanding::AllGood),
				..Default::default()
			}),
		);
		assert!(state.safety_hub.snapshot.is_none());
	}
}
