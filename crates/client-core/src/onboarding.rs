//! Rules screening and onboarding questions that stand between a fresh member and the composer.
//! Writes are never retried automatically; the gateway member stays authoritative.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{
	Id,
	onboarding::{
		COMPLETED_ONBOARDING, Features, Form, GuildGate, MemberGate, Outcome, Submission,
	},
};
use std::collections::BTreeMap;

pub enum Action {
	Load {
		verification: bool,
		onboarding: bool,
	},
	Submit {
		form: Box<Form>,
		submission: Box<Submission>,
	},
}
pub enum Event {
	/// Gateway observations; a snapshot replaces every known gate.
	Gates {
		snapshot: bool,
		gates: Vec<GuildGate>,
	},
	Loaded {
		guild: Id,
		request: u64,
		result: Result<Box<Form>, Failure>,
	},
	Submitted {
		guild: Id,
		request: u64,
		result: Result<Outcome, Failure>,
	},
}
impl Event {
	pub fn bytes(&self) -> usize {
		match self {
			Self::Gates { gates, .. } => gates.capacity() * size_of::<GuildGate>(),
			Self::Loaded { result, .. } => result.as_ref().map_or(0, |form| form.bytes()),
			Self::Submitted { .. } => 0,
		}
	}
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gate {
	pub features: Features,
	pub member: Option<MemberGate>,
}
impl Gate {
	fn relevant(&self) -> bool {
		self.features.verification
			|| self.features.onboarding
			|| self.member.is_some_and(|m| m.pending)
	}
}
pub struct Flow {
	pub guild: Id,
	pub form: Option<Form>,
	pub loading: bool,
	pub submitting: bool,
	pub error: Option<&'static str>,
	pub outcome: Option<Outcome>,
	/// Opened by the member rather than by a fresh join.
	pub explicit: bool,
	submitted_onboarding: bool,
	request: u64,
}
#[derive(Default)]
pub struct Onboarding {
	pub gates: BTreeMap<Id, Gate>,
	pub flow: Option<Flow>,
	/// A server joined in this session whose gate is still being discovered.
	watch: Option<Id>,
	sequence: u64,
}

impl State {
	fn gate(&self, guild: Id) -> Option<&Gate> {
		self.onboarding.gates.get(&guild)
	}
	/// Discord still holds this member behind the server rules; they can read but not talk.
	pub fn verification_pending(&self, guild: Id) -> bool {
		self.gate(guild)
			.and_then(|g| g.member)
			.is_some_and(|m| m.pending)
	}
	pub fn onboarding_incomplete(&self, guild: Id) -> bool {
		self.gate(guild).is_some_and(|g| {
			g.features.onboarding
				&& g.member
					.is_some_and(|m| m.flags & COMPLETED_ONBOARDING == 0)
		})
	}
	pub fn needs_onboarding(&self, guild: Id) -> bool {
		self.verification_pending(guild) || self.onboarding_incomplete(guild)
	}
	pub fn open_onboarding(&mut self, guild: Id) {
		if self
			.onboarding
			.flow
			.as_ref()
			.is_some_and(|f| f.guild == guild)
		{
			return;
		}
		self.start_flow(guild, true);
	}
	fn start_flow(&mut self, guild: Id, explicit: bool) {
		self.onboarding.sequence = self.onboarding.sequence.wrapping_add(1);
		self.onboarding.flow = Some(Flow {
			guild,
			form: None,
			loading: false,
			submitting: false,
			error: None,
			outcome: None,
			explicit,
			submitted_onboarding: false,
			request: self.onboarding.sequence,
		});
	}
	fn online(&self) -> bool {
		self.demo || (self.auth == AuthState::Authenticated && self.gateway_connected)
	}
	/// Issues the one form load the open dialog still needs.
	pub fn load_onboarding(&mut self) -> Option<Command> {
		let online = self.online();
		let gate = self
			.onboarding
			.flow
			.as_ref()
			.and_then(|f| self.gate(f.guild).copied());
		let flow = self.onboarding.flow.as_mut()?;
		if flow.form.is_some() || flow.loading || flow.error.is_some() || !online {
			return None;
		}
		// An unknown member (a server that hides itself until the rules are accepted) loads both.
		let member = gate.and_then(|g| g.member);
		let verification = member.is_none_or(|m| m.pending);
		let onboarding = member.is_none_or(|m| m.flags & COMPLETED_ONBOARDING == 0)
			&& gate.is_none_or(|g| g.features.onboarding || g.member.is_none());
		self.onboarding.sequence = self.onboarding.sequence.wrapping_add(1);
		flow.request = self.onboarding.sequence;
		flow.loading = true;
		Some(Command::Onboarding {
			guild: flow.guild,
			request: flow.request,
			action: Action::Load {
				verification,
				onboarding,
			},
		})
	}
	pub fn retry_onboarding(&mut self) {
		if let Some(flow) = &mut self.onboarding.flow
			&& !flow.loading
			&& !flow.submitting
		{
			flow.error = None;
			if flow.outcome.is_none() && flow.form.as_ref().is_none_or(Form::is_empty) {
				flow.form = None;
			}
		}
	}
	pub fn submit_onboarding(&mut self, mut submission: Submission) -> Option<Command> {
		let online = self.online();
		let guild = self.onboarding.flow.as_ref()?.guild;
		// Answers the gateway already recorded (for example after a partial failure) are not resent.
		let member = self.gate(guild).and_then(|g| g.member);
		if member.is_some_and(|m| m.flags & COMPLETED_ONBOARDING != 0) {
			submission.onboarding = None;
		}
		if member.is_some_and(|m| !m.pending) {
			submission.verification = None;
		}
		let flow = self.onboarding.flow.as_mut()?;
		let form = flow.form.as_ref()?;
		if flow.loading || flow.submitting || !online {
			return None;
		}
		let onboarding_ok = match (&form.onboarding, &submission.onboarding) {
			(Some(o), Some(chosen)) => o.complete(chosen),
			(_, None) => true,
			(None, Some(_)) => false,
		};
		let verification_ok = match (&form.verification, &submission.verification) {
			(Some(v), Some(answers)) => v.complete(answers),
			(_, None) => true,
			(None, Some(_)) => false,
		};
		if !onboarding_ok
			|| !verification_ok
			|| (submission.onboarding.is_none() && submission.verification.is_none())
		{
			return None;
		}
		self.onboarding.sequence = self.onboarding.sequence.wrapping_add(1);
		flow.request = self.onboarding.sequence;
		flow.submitting = true;
		flow.error = None;
		flow.submitted_onboarding = submission.onboarding.is_some();
		Some(Command::Onboarding {
			guild,
			request: flow.request,
			action: Action::Submit {
				form: Box::new(form.clone()),
				submission: Box::new(submission),
			},
		})
	}
	pub fn close_onboarding(&mut self) {
		// Keep an outstanding write admitted until its completion arrives.
		if self.onboarding.flow.as_ref().is_some_and(|f| f.submitting) {
			return;
		}
		self.onboarding.flow = None;
		self.onboarding.watch = None;
	}
	pub fn onboarding_command_allowed(&self, guild: Id, request: u64) -> bool {
		self.onboarding.flow.as_ref().is_some_and(|f| {
			f.guild == guild && f.request == request && (f.loading || f.submitting)
		})
	}
	pub(crate) fn cancel_onboarding(&mut self) {
		if let Some(flow) = &mut self.onboarding.flow
			&& (flow.loading || flow.submitting)
		{
			flow.error = Some(if flow.submitting {
				Failure::Ambiguous.label()
			} else {
				"Disconnected; try again"
			});
			flow.loading = false;
			flow.submitting = false;
			self.onboarding.sequence = self.onboarding.sequence.wrapping_add(1);
			flow.request = self.onboarding.sequence;
		}
	}
	pub(crate) fn reset_onboarding(&mut self) {
		self.onboarding = Onboarding {
			sequence: self.onboarding.sequence.wrapping_add(1),
			..Onboarding::default()
		};
	}
	/// A fresh invite join: ask for anything the server requires before the member can talk.
	pub(crate) fn watch_onboarding(&mut self, guild: Id) {
		self.onboarding.watch = Some(guild);
		if self.gate(guild).is_some() {
			self.reconcile_onboarding();
		} else if self.guild(guild).is_none() {
			// The server may stay hidden until its rules are accepted; probe the forms directly.
			if self.onboarding.flow.is_none() {
				self.start_flow(guild, false);
			}
		} else {
			// Already visible without a gate: nothing stands in the way.
			self.onboarding.watch = None;
		}
	}
	fn reconcile_onboarding(&mut self) {
		if let Some(guild) = self.onboarding.watch
			&& self.onboarding.flow.is_none()
			&& self.gate(guild).is_some_and(|g| g.member.is_some())
		{
			if self.needs_onboarding(guild) {
				self.start_flow(guild, false);
			} else {
				self.onboarding.watch = None;
			}
		}
		// Completed elsewhere (another client, a moderator): close an automatic prompt.
		if let Some(flow) = &self.onboarding.flow
			&& !flow.explicit
			&& !flow.submitting
			&& flow.outcome.is_none()
			&& self.gate(flow.guild).is_some_and(|g| g.member.is_some())
			&& !self.needs_onboarding(flow.guild)
		{
			self.onboarding.flow = None;
			self.onboarding.watch = None;
		}
	}
	pub(crate) fn apply_onboarding(&mut self, event: Event) -> Result<(), &'static str> {
		match event {
			Event::Gates { snapshot, gates } => {
				if gates.len() > crate::MAX_NAV {
					return Err("Onboarding gate batch exceeds capacity");
				}
				if snapshot {
					self.onboarding.gates.clear();
				}
				for update in gates {
					let gate = self.onboarding.gates.entry(update.guild).or_default();
					if let Some(features) = update.features {
						gate.features = features;
					}
					if let Some(member) = update.member {
						gate.member = Some(member);
					}
					if !gate.relevant() {
						self.onboarding.gates.remove(&update.guild);
					}
				}
				if self.onboarding.gates.len() > crate::MAX_NAV {
					return Err("Onboarding gates exceed capacity");
				}
				self.reconcile_onboarding();
			}
			Event::Loaded {
				guild,
				request,
				result,
			} => {
				let gate = self.gate(guild).copied();
				let Some(flow) = self
					.onboarding
					.flow
					.as_mut()
					.filter(|f| f.guild == guild && f.request == request && f.loading)
				else {
					return Ok(());
				};
				flow.loading = false;
				match result {
					Ok(form) if form.valid() => {
						let mut form = *form;
						let member = gate.and_then(|g| g.member);
						if member.is_some_and(|m| !m.pending) {
							form.verification = None;
						}
						if member.is_some_and(|m| m.flags & COMPLETED_ONBOARDING != 0) {
							form.onboarding = None;
						}
						if form.is_empty() && !flow.explicit {
							// Nothing to answer; keep watching only while the member is unknown.
							self.onboarding.flow = None;
							if member.is_some() {
								self.onboarding.watch = None;
							}
							return Ok(());
						}
						flow.form = Some(form);
					}
					Ok(_) => flow.error = Some("Server onboarding response was invalid"),
					Err(failure) => {
						if failure.ends_session() {
							self.fail(failure);
							return Ok(());
						}
						flow.error = Some(failure.label());
					}
				}
			}
			Event::Submitted {
				guild,
				request,
				result,
			} => {
				let Some(flow) = self
					.onboarding
					.flow
					.as_mut()
					.filter(|f| f.guild == guild && f.request == request && f.submitting)
				else {
					return Ok(());
				};
				flow.submitting = false;
				match result {
					Ok(outcome) => {
						let onboarding = flow.submitted_onboarding;
						flow.outcome = Some(outcome);
						self.onboarding.watch = None;
						if let Some(member) = self
							.onboarding
							.gates
							.get_mut(&guild)
							.and_then(|g| g.member.as_mut())
						{
							if outcome == Outcome::Approved {
								member.pending = false;
							}
							if onboarding {
								member.flags |= COMPLETED_ONBOARDING;
							}
						}
						if outcome == Outcome::Approved {
							self.onboarding.flow = None;
							self.status = "You're in · welcome to the server";
						}
					}
					Err(failure) => {
						if failure.ends_session() {
							self.fail(failure);
							return Ok(());
						}
						flow.error = Some(failure.label());
					}
				}
			}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::onboarding::{Answer, FieldKind, Verification};

	fn rules() -> Form {
		Form {
			verification: Some(Verification {
				version: None,
				description: None,
				fields: vec![model::onboarding::Field {
					kind: FieldKind::Terms,
					wire_kind: "TERMS".into(),
					label: "Rules".into(),
					description: None,
					placeholder: None,
					rules: vec!["Be kind".into()],
					choices: Vec::new(),
					required: true,
				}],
			}),
			onboarding: None,
		}
	}

	#[test]
	fn pending_member_is_prompted_after_join_and_unlocked_on_approval() {
		let mut state = State {
			demo: true,
			..State::default()
		};
		let guild = Id(10);
		state
			.apply_onboarding(Event::Gates {
				snapshot: false,
				gates: vec![GuildGate {
					guild,
					features: Some(Features {
						verification: true,
						onboarding: false,
					}),
					member: Some(MemberGate {
						pending: true,
						flags: 0,
					}),
				}],
			})
			.unwrap();
		assert!(state.verification_pending(guild));
		state.watch_onboarding(guild);
		let Some(Command::Onboarding { request, .. }) = state.load_onboarding() else {
			panic!("expected a form load");
		};
		assert!(state.load_onboarding().is_none());
		state
			.apply_onboarding(Event::Loaded {
				guild,
				request,
				result: Ok(Box::new(rules())),
			})
			.unwrap();
		assert!(
			state
				.submit_onboarding(Submission {
					onboarding: None,
					verification: Some(vec![Answer::Terms(false)]),
				})
				.is_none()
		);
		let Some(Command::Onboarding { request, .. }) = state.submit_onboarding(Submission {
			onboarding: None,
			verification: Some(vec![Answer::Terms(true)]),
		}) else {
			panic!("expected a submission");
		};
		state
			.apply_onboarding(Event::Submitted {
				guild,
				request,
				result: Ok(Outcome::Approved),
			})
			.unwrap();
		assert!(!state.verification_pending(guild));
		assert!(state.onboarding.flow.is_none());
	}

	#[test]
	fn open_member_is_not_prompted_after_join() {
		let mut state = State {
			demo: true,
			..State::default()
		};
		state
			.apply_onboarding(Event::Gates {
				snapshot: false,
				gates: vec![GuildGate {
					guild: Id(10),
					features: Some(Features {
						verification: false,
						onboarding: true,
					}),
					member: Some(MemberGate {
						pending: false,
						flags: COMPLETED_ONBOARDING,
					}),
				}],
			})
			.unwrap();
		state.watch_onboarding(Id(10));
		assert!(state.onboarding.flow.is_none());
	}
}
