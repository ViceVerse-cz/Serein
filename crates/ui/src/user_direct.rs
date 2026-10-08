//! One explicit user action waiting for a confirmed DM; no background contact discovery.
use client_core::{Command, State};
use model::{Id, User};

pub(super) enum Intent {
	Message,
	Call,
	Send(String),
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Envelope, Event, auth::AuthState, user_actions};

	fn fixture() -> (State, User, model::Channel) {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = AuthState::Authenticated;
		state.gateway_connected = true;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::UserAction(user_actions::Event::Relationships(Some(vec![]))),
		});
		let mut channel = state
			.channels
			.iter()
			.find(|channel| channel.kind == 1)
			.unwrap()
			.clone();
		let mut user = channel.recipients[0].clone();
		user.id = Id(700001);
		user.name = "Nonfriend (synthetic)".into();
		channel.id = Id(990001);
		channel.recipients = vec![user.clone()];
		(state, user, channel)
	}

	fn confirmed(
		view: &mut crate::MessagingUi,
		state: &mut State,
		user: Id,
		channel: model::Channel,
		commands: &mut Vec<Command>,
	) {
		let request = commands
			.iter()
			.find_map(|command| match command {
				Command::UserAction {
					action: user_actions::Action::OpenDm(target),
					request,
					..
				} if *target == user => Some(*request),
				_ => None,
			})
			.unwrap();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::UserAction(user_actions::Event::DmOpened {
				user,
				request,
				result: Ok(Box::new(channel)),
			}),
		});
		let before = (state.selected, state.request);
		if let Some(command) = state.select_opened_dm() {
			commands.push(command);
		}
		view.finish_user_direct(state, before, commands);
	}

	#[test]
	fn nonfriend_actions_wait_for_confirmation_and_dispatch_only_the_requested_action() {
		for purpose in 0..3 {
			let (mut state, user, channel) = fixture();
			let mut view = crate::MessagingUi {
				voice_available: true,
				..Default::default()
			};
			view.profile.command_open(user.clone());
			state
				.drafts
				.insert(state.selected.unwrap(), "Original draft".into());
			state.drafts.insert(channel.id, "Destination draft".into());
			let drafts = state.drafts.clone();
			let mut commands = vec![];
			let intent = match purpose {
				0 => Intent::Message,
				1 => Intent::Call,
				_ => Intent::Send("Explicit profile message".into()),
			};
			view.open_user_direct(&mut state, user.clone(), intent, &mut commands);
			assert!(matches!(
				commands.as_slice(),
				[Command::UserAction {
					action: user_actions::Action::OpenDm(_),
					..
				}]
			));
			assert!(view.user_direct.is_some());
			confirmed(
				&mut view,
				&mut state,
				user.id,
				channel.clone(),
				&mut commands,
			);
			assert_eq!(state.selected, Some(channel.id));
			assert_eq!(state.drafts, drafts);
			assert_eq!(commands.iter().filter(|command| matches!(command, Command::Send { channel: target, content, reply: None, .. } if *target == channel.id && content == "Explicit profile message")).count(), usize::from(purpose == 2));
			assert_eq!(commands.iter().filter(|command| matches!(command, Command::Voice(client_core::voice::Command::Join { channel: target, ring: true, .. }) if *target == channel.id)).count(), usize::from(purpose == 1));
			assert!(view.user_direct.is_none());
			let before = (state.selected, state.request);
			view.finish_user_direct(&mut state, before, &mut commands);
			assert_eq!(commands.len(), if purpose == 0 { 2 } else { 3 });
		}
	}

	#[test]
	fn canceled_or_revoked_profile_handoff_never_sends_or_calls() {
		for outcome in 0..5 {
			let (mut state, user, channel) = fixture();
			let mut view = crate::MessagingUi {
				voice_available: true,
				..Default::default()
			};
			view.profile.command_open(user.clone());
			let mut commands = vec![];
			view.open_user_direct(
				&mut state,
				user.clone(),
				Intent::Send("Kept text".into()),
				&mut commands,
			);
			match outcome {
				0 => {
					state.select(Id(21));
				}
				1 => {
					view.profile.close();
				}
				2 => {
					state.generation += 1;
				}
				3 => {
					state.gateway_connected = false;
				}
				_ => state.apply(Envelope {
					generation: state.generation,
					event: Event::UserAction(user_actions::Event::Relationship {
						user: user.id,
						blocked: true,
					}),
				}),
			}
			confirmed(&mut view, &mut state, user.id, channel, &mut commands);
			assert!(
				!commands
					.iter()
					.any(|command| matches!(command, Command::Send { .. } | Command::Voice(_)))
			);
			assert!(view.user_direct.is_none());
		}
	}

	#[test]
	fn existing_dm_profile_send_preserves_composer_reply_and_rejects_oversize() {
		let (mut state, user, channel) = fixture();
		state.channels.push(channel.clone());
		state.select(channel.id);
		state.reply = Some(client_core::Reply::to(Id(123)));
		let mut view = crate::MessagingUi::default();
		view.profile.command_open(user.clone());
		let mut commands = vec![];
		view.open_user_direct(
			&mut state,
			user.clone(),
			Intent::Send("🦀".repeat(client_core::MAX_CONTENT + 1)),
			&mut commands,
		);
		assert!(commands.is_empty());
		view.open_user_direct(
			&mut state,
			user,
			Intent::Send("Direct from profile".into()),
			&mut commands,
		);
		assert!(matches!(
			commands.as_slice(),
			[Command::Send { reply: None, .. }]
		));
		assert_eq!(state.reply.unwrap().target(), Id(123));
	}
}

/// At most one loaded user and 2,000 characters / 8 KiB of explicit message text.
pub(super) struct Pending {
	generation: u64,
	origin: (Option<Id>, u64),
	user: User,
	intent: Intent,
}

fn direct_channel(state: &State, user: Id) -> Option<Id> {
	state.channels.iter().find_map(|channel| {
		(channel.guild.is_none()
			&& channel.kind == 1
			&& channel.recipients.len() == 1
			&& channel.recipients[0].id == user
			&& !channel.recipients[0].webhook)
			.then_some(channel.id)
	})
}

impl crate::MessagingUi {
	pub(super) fn open_user_direct(
		&mut self,
		state: &mut State,
		user: User,
		intent: Intent,
		commands: &mut Vec<Command>,
	) {
		if self.user_direct.is_some() {
			state.status = "Wait for the current direct message action to finish";
			return;
		}
		if matches!(intent, Intent::Call) && (state.demo || !self.voice_available) {
			state.status = "Calls are unavailable in this session";
			return;
		}
		if !state.can_open_user_dm(&user)
			|| user.heap_bytes() > 64 * 1024
			|| matches!(&intent, Intent::Send(content) if content.trim().is_empty()
				|| content.len() > client_core::MAX_CONTENT * 4
				|| content.chars().count() > client_core::MAX_CONTENT)
			|| !self.server_settings.navigate_away(state)
		{
			self.profile.finish_message(user.id, false);
			state.status = "This direct message action is unavailable; your message was kept";
			return;
		}
		let pending = Pending {
			generation: state.generation,
			origin: (state.selected, state.request),
			user,
			intent,
		};
		let command = state.open_user_dm(&pending.user);
		let waiting = matches!(
			&command,
			Some(Command::UserAction {
				action: client_core::user_actions::Action::OpenDm(_),
				..
			})
		);
		if let Some(command) = command {
			commands.push(command);
		}
		if waiting {
			self.user_direct = Some(pending);
		} else if let Some(channel) = direct_channel(state, pending.user.id)
			.filter(|channel| state.selected == Some(*channel))
		{
			self.complete_user_direct(state, pending, channel, commands);
		} else {
			self.profile.finish_message(pending.user.id, false);
		}
	}

	pub(super) fn finish_user_direct(
		&mut self,
		state: &mut State,
		before_selection: (Option<Id>, u64),
		commands: &mut Vec<Command>,
	) {
		let Some(pending) = self.user_direct.take() else {
			return;
		};
		let scope_matches = pending.generation == state.generation
			&& pending.origin == before_selection
			&& (!matches!(pending.intent, Intent::Send(_))
				|| self
					.profile
					.open_user()
					.is_some_and(|user| user.id == pending.user.id));
		let adopted = before_selection != (state.selected, state.request);
		if scope_matches
			&& adopted
			&& let Some(channel) = direct_channel(state, pending.user.id)
				.filter(|channel| state.selected == Some(*channel))
			&& state.can_open_user_dm(&pending.user)
		{
			self.complete_user_direct(state, pending, channel, commands);
		} else if scope_matches && !adopted && state.user_action_pending() {
			self.user_direct = Some(pending);
		} else {
			self.profile.finish_message(pending.user.id, false);
		}
	}

	fn complete_user_direct(
		&mut self,
		state: &mut State,
		pending: Pending,
		channel: Id,
		commands: &mut Vec<Command>,
	) {
		match pending.intent {
			Intent::Message => {
				self.profile.close();
				self.focus_switched_composer = true;
			}
			Intent::Call => {
				if let Some(reason) = self.call_unavailable(state, channel) {
					state.status = reason;
				} else {
					self.profile.close();
					self.request_call(state, channel, true, commands);
				}
			}
			Intent::Send(content) => {
				if let Some(command) = state.prepare_text_send(&content) {
					commands.push(command);
					self.profile.finish_message(pending.user.id, true);
					self.profile.close();
				} else {
					self.profile.finish_message(pending.user.id, false);
				}
			}
		}
	}
}
