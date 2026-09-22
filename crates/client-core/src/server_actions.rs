//! Deliberate server writes: one pending request and one bounded, session-only result.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Id, permissions::VIEW_CHANNEL};
use std::time::{Duration, Instant};

const CREATE_INSTANT_INVITE: u128 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InviteOptions {
	pub max_age: u32,
	pub max_uses: u16,
	pub temporary: bool,
}
impl Default for InviteOptions {
	fn default() -> Self {
		Self {
			max_age: 30 * 86400,
			max_uses: 0,
			temporary: false,
		}
	}
}
impl InviteOptions {
	pub fn valid(self) -> bool {
		// Normal-user 30-day invites exceed the public developer API's documented 7 days.
		self.max_age <= 30 * 86400 && self.max_uses <= 100
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteStatus {
	Sending,
	Sent,
	Failed(Failure),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
	CreateInvite {
		guild: Id,
		channel: Id,
		options: InviteOptions,
	},
	Leave(Id),
}
impl Action {
	pub fn guild(self) -> Id {
		match self {
			Self::CreateInvite { guild, .. } | Self::Leave(guild) => guild,
		}
	}
}
pub enum Event {
	InviteSent {
		guild: Id,
		user: Id,
		request: u64,
		result: Result<Box<(model::Channel, model::Message)>, Failure>,
	},
	Written {
		action: Action,
		request: u64,
		result: Result<Option<String>, Failure>,
	},
}
#[derive(Default)]
pub struct Actions {
	sequence: u64,
	pending: Option<(Action, u64, bool)>,
	status: Option<(Id, &'static str)>,
	invite: Option<(Id, Id, String, Instant, InviteOptions)>,
	sending: Option<(Id, Id, u64, String)>,
	sent: std::collections::BTreeMap<Id, InviteStatus>,
}
impl Actions {
	pub(crate) fn reset(&mut self) {
		*self = Self {
			sequence: self.sequence,
			..Self::default()
		};
	}
}
impl State {
	pub fn server_invite_pending(&self) -> bool {
		self.server_actions.sending.is_some()
	}
	pub fn server_invite_status(&self, guild: Id, user: Id) -> Option<InviteStatus> {
		if self
			.server_actions
			.sending
			.as_ref()
			.is_some_and(|(g, u, _, _)| *g == guild && *u == user)
		{
			return Some(InviteStatus::Sending);
		}
		self.server_actions
			.invite
			.as_ref()
			.filter(|i| i.0 == guild)?;
		self.server_actions.sent.get(&user).copied()
	}
	pub fn send_server_invite(&mut self, guild: Id, user: Id) -> Option<Command> {
		if self.server_invite_pending()
			|| self.server_action_pending()
			|| (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
			|| !self.friends().any(|friend| friend.id == user)
			|| self.user_blocked(user) != Some(false)
			|| matches!(
				self.server_invite_status(guild, user),
				Some(InviteStatus::Sent | InviteStatus::Failed(Failure::Ambiguous))
			) || self.server_actions.sent.len() >= crate::user_actions::MAX_RELATIONSHIPS
		{
			return None;
		}
		let url = self.created_invite(guild)?.to_owned();
		let code = url.strip_prefix("https://discord.gg/")?.to_owned();
		self.server_actions.sequence = self.server_actions.sequence.wrapping_add(1);
		let request = self.server_actions.sequence;
		self.send_sequence = self.send_sequence.wrapping_add(1);
		let epoch = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_millis();
		let nonce = crate::fingerprint::nonce(epoch, self.send_sequence);
		self.server_actions.sending = Some((guild, user, request, nonce.clone()));
		self.server_actions.status = None;
		Some(Command::SendServerInvite {
			guild,
			user,
			code,
			nonce,
			request,
		})
	}
	fn apply_server_invite_sent(
		&mut self,
		guild: Id,
		user: Id,
		request: u64,
		result: Result<(model::Channel, model::Message), Failure>,
	) -> Result<(), &'static str> {
		let Some((g, u, r, nonce)) = self.server_actions.sending.as_ref() else {
			return Ok(());
		};
		if (*g, *u, *r) != (guild, user, request) {
			return Ok(());
		}
		let result = result.and_then(|(channel, message)| {
			if channel.guild.is_some()
				|| channel.kind != 1
				|| channel.recipients.len() != 1
				|| channel.recipients[0].id != user
				|| channel.id.0 == 0
				|| message.id.0 == 0
				|| message.channel != channel.id
				|| message.nonce.as_deref() != Some(nonce.as_str())
				|| self
					.user
					.as_ref()
					.is_none_or(|owner| owner.id != message.author.id)
				|| self
					.server_actions
					.invite
					.as_ref()
					.is_none_or(|i| i.0 != guild || i.2 != message.content)
			{
				Err(Failure::Ambiguous)
			} else {
				Ok((channel, message))
			}
		});
		self.server_actions.sending = None;
		match result {
			Ok((channel, message)) => {
				self.server_actions.sent.insert(user, InviteStatus::Sent);
				self.server_actions.status = Some((guild, "Invite sent"));
				// Feed acknowledgements through normal bounded channel/message reconciliation.
				if self.channel(channel.id).is_none() {
					self.apply(crate::Envelope {
						generation: self.generation,
						event: crate::Event::ChannelCreated(channel),
					});
				}
				self.apply(crate::Envelope {
					generation: self.generation,
					event: crate::Event::Message(message),
				});
			}
			Err(failure) => {
				self.server_actions
					.sent
					.insert(user, InviteStatus::Failed(failure));
				self.server_actions.status = Some((guild, failure.label()));
				if failure.ends_session() {
					self.fail(failure);
				}
			}
		}
		Ok(())
	}
	pub fn server_action_pending(&self) -> bool {
		self.server_actions.pending.is_some()
	}
	pub fn server_action_status(&self, guild: Id) -> Option<&'static str> {
		self.server_actions
			.status
			.filter(|(id, _)| *id == guild)
			.map(|(_, text)| text)
	}
	pub fn created_invite(&self, guild: Id) -> Option<&str> {
		self.server_actions
			.invite
			.as_ref()
			.filter(|(id, channel, _, at, options)| {
				*id == guild
					&& self.can_create_server_invite(guild, *channel)
					&& (options.max_age == 0
						|| at.elapsed() < Duration::from_secs(u64::from(options.max_age)))
			})
			.map(|(_, _, url, _, _)| url.as_str())
	}
	pub fn created_invite_options(&self, guild: Id) -> Option<InviteOptions> {
		self.created_invite(guild)?;
		self.server_actions.invite.as_ref().map(|i| i.4)
	}
	pub fn clear_server_action_result(&mut self, guild: Id) {
		if self
			.server_actions
			.sending
			.as_ref()
			.is_some_and(|s| s.0 == guild)
		{
			return;
		}
		if self
			.server_actions
			.status
			.is_some_and(|(id, _)| id == guild)
		{
			self.server_actions.status = None;
		}
		if self
			.server_actions
			.invite
			.as_ref()
			.is_some_and(|(id, _, _, _, _)| *id == guild)
		{
			self.server_actions.invite = None;
			self.server_actions.sent.clear();
		}
	}
	pub fn can_create_server_invite(&self, guild: Id, channel: Id) -> bool {
		self.channel(channel)
			.is_some_and(|c| c.guild == Some(guild) && matches!(c.kind, 0 | 2 | 5 | 13 | 15 | 16))
			&& self.permission(channel, VIEW_CHANNEL | CREATE_INSTANT_INVITE) == Some(true)
	}
	pub fn invite_channel(&self, guild: Id) -> Option<Id> {
		self.selected
			.filter(|id| self.can_create_server_invite(guild, *id))
			.or_else(|| {
				self.channels
					.iter()
					.find(|c| self.can_create_server_invite(guild, c.id))
					.map(|c| c.id)
			})
	}
	pub fn leave_server_reason(&self, guild: Id) -> Option<&'static str> {
		if self.guild(guild).is_none() {
			return Some("Server is no longer available");
		}
		let Some(owner) = self.permissions.guilds.get(&guild).and_then(|g| g.owner) else {
			return Some("Server ownership is not available yet");
		};
		let Some(user) = self.user.as_ref() else {
			return Some("Sign in before leaving a server");
		};
		if owner == user.id {
			return Some("Transfer ownership in Discord before leaving this server");
		}
		if self.pending.iter().any(|p| {
			self.channel(p.channel)
				.is_some_and(|c| c.guild == Some(guild))
		}) || self
			.voice
			.active
			.as_ref()
			.is_some_and(|call| call.guild == Some(guild))
		{
			return Some("Finish pending messages and leave the call before leaving this server");
		}
		None
	}
	pub fn create_server_invite(&mut self, guild: Id, channel: Id) -> Option<Command> {
		self.create_server_invite_with_options(guild, channel, InviteOptions::default())
	}
	pub fn create_server_invite_with_options(
		&mut self,
		guild: Id,
		channel: Id,
		options: InviteOptions,
	) -> Option<Command> {
		if !options.valid() || self.server_invite_pending() {
			return None;
		}
		if !self.can_create_server_invite(guild, channel) {
			self.server_actions.status = Some((
				guild,
				"You need Create Invite permission in a visible channel",
			));
			return None;
		}
		self.request_server_action(Action::CreateInvite {
			guild,
			channel,
			options,
		})
	}
	pub fn leave_server(&mut self, guild: Id) -> Option<Command> {
		if let Some(reason) = self.leave_server_reason(guild) {
			self.server_actions.status = Some((guild, reason));
			return None;
		}
		self.request_server_action(Action::Leave(guild))
	}
	fn request_server_action(&mut self, action: Action) -> Option<Command> {
		if self.server_action_pending() || self.server_invite_pending() {
			return None;
		}
		if !self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected) {
			self.server_actions.status = Some((
				action.guild(),
				"Server actions unavailable while disconnected",
			));
			return None;
		}
		self.server_actions.sequence = self.server_actions.sequence.wrapping_add(1);
		let request = self.server_actions.sequence;
		self.server_actions.pending = Some((action, request, false));
		self.clear_server_action_result(action.guild());
		Some(Command::ServerAction { action, request })
	}
	pub(crate) fn cancel_server_action(&mut self) {
		if let Some((guild, user, _, _)) = self.server_actions.sending.take() {
			self.server_actions
				.sent
				.insert(user, InviteStatus::Failed(Failure::Ambiguous));
			self.server_actions.status = Some((guild, Failure::Ambiguous.label()));
		}
		if let Some((action, _, _)) = self.server_actions.pending.take() {
			self.server_actions.status = Some((
				action.guild(),
				"Outcome unknown; check Discord before retrying",
			));
		}
	}
	pub(crate) fn observe_server_joined(&mut self, guild: Id) {
		if let Some((Action::Leave(id), _, observed)) = &mut self.server_actions.pending
			&& *id == guild
		{
			*observed = true;
		}
	}
	pub(crate) fn apply_server_action(&mut self, event: Event) -> Result<(), &'static str> {
		if let Event::InviteSent {
			guild,
			user,
			request,
			result,
		} = event
		{
			return self.apply_server_invite_sent(guild, user, request, result.map(|sent| *sent));
		}
		let Event::Written {
			action,
			request,
			result,
		} = event
		else {
			unreachable!()
		};
		let Some((pending, sequence, observed)) = self.server_actions.pending else {
			return Ok(());
		};
		if pending != action || sequence != request {
			return Ok(());
		}
		self.server_actions.pending = None;
		let result = result.and_then(|code| match action {
			Action::CreateInvite { .. }
				if code.as_ref().is_some_and(|code| {
					crate::invites::valid_code(code) && code.capacity() <= 1024
				}) =>
			{
				Ok(code)
			}
			Action::Leave(_) if code.is_none() => Ok(None),
			_ => Err(Failure::Ambiguous),
		});
		let status = match result {
			Err(failure) => {
				if failure.ends_session() {
					self.fail(failure);
				}
				failure.label()
			}
			Ok(code) => match action {
				Action::CreateInvite {
					guild,
					channel,
					options,
				} => {
					if self.can_create_server_invite(guild, channel) {
						self.server_actions.sent.clear();
						self.server_actions.invite = Some((
							guild,
							channel,
							format!("https://discord.gg/{}", code.unwrap()),
							Instant::now(),
							options,
						));
						"Invite created"
					} else {
						"Invite created, but channel access changed; check Discord"
					}
				}
				Action::Leave(guild) => {
					if observed {
						"Request completed; latest server membership shown"
					} else {
						self.remove_server(guild);
						"Left server"
					}
				}
			},
		};
		self.server_actions.status = Some((action.guild(), status));
		self.status = status;
		Ok(())
	}
	fn remove_server(&mut self, guild: Id) {
		let removed = self
			.channels
			.iter()
			.filter(|c| c.guild == Some(guild))
			.map(|c| c.id)
			.collect();
		self.remove_channels(&removed);
		if self.selected.is_some_and(|id| removed.contains(&id)) {
			self.arrived_home();
		}
		self.guilds.retain(|g| g.id != guild);
		self.permissions.guilds.remove(&guild);
		self.permissions.clear_cache();
		self.invalidate_navigation();
		self.clear_server_action_result(guild);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event as CoreEvent};
	fn state() -> State {
		let mut state = State {
			user: Some(model::User {
				primary_guild: None,
				id: Id(1),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				stickers: None,
				id: Id(2),
				name: "Synthetic server".into(),
				icon: None,
				emojis: None,
			}],
			channels: vec![model::Channel {
				id: Id(3),
				guild: Some(Id(2)),
				name: "general".into(),
				kind: 0,
				parent_id: None,
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			selected: Some(Id(3)),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		};
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: vec![model::permissions::Guild {
					id: Id(2),
					owner: Some(Id(9)),
					roles: Some(vec![model::permissions::Role {
						id: Id(2),
						bits: VIEW_CHANNEL | CREATE_INSTANT_INVITE,
						name: String::new(),
						color: 0,
						position: 0,
						hoist: false,
					}]),
					member: Some(model::permissions::Member {
						roles: vec![],
						timeout_until: None,
					}),
				}],
				channels: vec![model::permissions::Channel {
					id: Id(3),
					guild: Id(2),
					overwrites: Some(vec![]),
				}],
			})
			.unwrap();
		state
	}
	fn finish(state: &mut State, command: Command, result: Result<Option<String>, Failure>) {
		let Command::ServerAction { action, request } = command else {
			panic!("wrong command");
		};
		let event = CoreEvent::ServerAction(Event::Written {
			action,
			request,
			result,
		});
		if matches!(
			&event,
			CoreEvent::ServerAction(Event::Written {
				action: Action::Leave(_),
				result: Ok(None),
				..
			})
		) {
			assert!(
				event.changes_access(),
				"confirmed leave must invalidate cached access"
			);
		}
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
	#[test]
	fn server_invites_options_friends_acknowledgement_and_cancellation() {
		let mut state = state();
		let friend = model::User {
			primary_guild: None,
			id: Id(8),
			name: "Friend".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		state
			.apply_user_action(crate::user_actions::Event::Relationships(Some(vec![(
				friend.id, false,
			)])))
			.unwrap();
		state
			.apply_user_action(crate::user_actions::Event::Friends(Some(vec![(
				friend.clone(),
				"friend_user".into(),
			)])))
			.unwrap();
		assert!(state.send_server_invite(Id(2), friend.id).is_none());
		assert!(
			!InviteOptions {
				max_age: 2592001,
				..Default::default()
			}
			.valid()
		);
		assert!(
			!InviteOptions {
				max_uses: 101,
				..Default::default()
			}
			.valid()
		);
		let options = InviteOptions {
			max_age: 0,
			max_uses: 10,
			temporary: true,
		};
		let command = state
			.create_server_invite_with_options(Id(2), Id(3), options)
			.unwrap();
		finish(&mut state, command, Ok(Some("invite_test".into())));
		assert_eq!(state.created_invite_options(Id(2)), Some(options));
		assert!(state.send_server_invite(Id(2), Id(99)).is_none());
		state.drafts.insert(Id(3), "Keep the draft".into());
		let Command::SendServerInvite {
			guild,
			user,
			nonce,
			request,
			..
		} = state.send_server_invite(Id(2), friend.id).unwrap()
		else {
			panic!()
		};
		assert_eq!(
			state.server_invite_status(guild, user),
			Some(InviteStatus::Sending)
		);
		assert!(state.send_server_invite(guild, user).is_none());
		assert!(state.create_server_invite(guild, Id(3)).is_none());
		state.clear_server_action_result(guild);
		assert!(state.created_invite(guild).is_some());
		let mut channel = state.channel(Id(3)).unwrap().clone();
		channel.id = Id(10);
		channel.guild = None;
		channel.kind = 1;
		channel.recipients = vec![friend];
		let mut message = crate::tests::message(100);
		message.channel = channel.id;
		message.author = state.user.clone().unwrap();
		message.nonce = Some(nonce);
		message.content = "https://discord.gg/invite_test".into();
		state
			.apply_server_action(Event::InviteSent {
				guild,
				user,
				request: request + 1,
				result: Ok(Box::new((channel.clone(), message.clone()))),
			})
			.unwrap();
		assert!(state.server_invite_pending());
		let expected_bytes = size_of::<CoreEvent>() + channel.bytes() + message.bytes();
		let event = CoreEvent::ServerAction(Event::InviteSent {
			guild,
			user,
			request,
			result: Ok(Box::new((channel, message))),
		});
		assert_eq!(event.bytes(), expected_bytes);
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
		assert_eq!(
			state.server_invite_status(guild, user),
			Some(InviteStatus::Sent)
		);
		assert!(state.send_server_invite(guild, user).is_none());
		assert_eq!(state.selected, Some(Id(3)));
		assert_eq!(state.drafts[&Id(3)], "Keep the draft");
		assert!(state.channel(Id(10)).is_some());
		let command = state.create_server_invite(guild, Id(3)).unwrap();
		finish(&mut state, command, Ok(Some("new_link".into())));
		let command = state.send_server_invite(guild, user).unwrap();
		state.command_rejected(command);
		assert!(!state.server_invite_pending());
		let command = state.send_server_invite(guild, user).unwrap();
		state.cancel_server_action();
		assert_eq!(
			state.server_invite_status(guild, user),
			Some(InviteStatus::Failed(Failure::Ambiguous))
		);
		assert!(state.send_server_invite(guild, user).is_none());
		state.command_rejected(command);
		assert_eq!(
			state.server_invite_status(guild, user),
			Some(InviteStatus::Failed(Failure::Ambiguous))
		);
	}
	#[test]
	fn server_actions_scope_permissions_and_confirmed_removal() {
		let mut state = state();
		assert_eq!(state.invite_channel(Id(2)), Some(Id(3)));
		assert_eq!(state.invite_channel(Id(8)), None);
		assert!(state.create_server_invite(Id(8), Id(3)).is_none());
		let invite = state.create_server_invite(Id(2), Id(3)).unwrap();
		assert!(state.leave_server(Id(2)).is_none());
		finish(&mut state, invite, Ok(Some("safe-code_1".into())));
		assert_eq!(
			state.created_invite(Id(2)),
			Some("https://discord.gg/safe-code_1")
		);
		assert_eq!(state.created_invite(Id(8)), None);
		let invite = state.create_server_invite(Id(2), Id(3)).unwrap();
		finish(&mut state, invite, Ok(Some("../malicious".into())));
		assert_eq!(state.created_invite(Id(2)), None);
		assert_eq!(
			state.server_action_status(Id(2)),
			Some(Failure::Ambiguous.label())
		);
		state.permissions.guilds.get_mut(&Id(2)).unwrap().owner = Some(Id(1));
		assert!(state.leave_server(Id(2)).is_none());
		state.permissions.guilds.get_mut(&Id(2)).unwrap().owner = None;
		assert!(state.leave_server(Id(2)).is_none());
		state.permissions.guilds.get_mut(&Id(2)).unwrap().owner = Some(Id(9));
		state.drafts.insert(Id(3), "Keep draft".into());
		let leave = state.leave_server(Id(2)).unwrap();
		finish(&mut state, leave, Err(Failure::Ambiguous));
		assert!(state.guild(Id(2)).is_some());
		let old = state.leave_server(Id(2)).unwrap();
		state.cancel_server_action();
		let new = state.leave_server(Id(2)).unwrap();
		finish(&mut state, old, Ok(None));
		assert!(state.server_action_pending());
		assert!(state.guild(Id(2)).is_some());
		finish(&mut state, new, Ok(None));
		assert!(state.guild(Id(2)).is_none());
		assert!(state.channel(Id(3)).is_none());
		assert_eq!(state.selected, None);
		assert_eq!(state.drafts[&Id(3)], "Keep draft");
	}
	#[test]
	fn server_actions_reject_stale_sessions_and_keep_rejoined_guilds() {
		let mut state = state();
		let leave = state.leave_server(Id(2)).unwrap();
		state.observe_server_joined(Id(2));
		finish(&mut state, leave, Ok(None));
		assert!(state.guild(Id(2)).is_some());
		let invite = state.create_server_invite(Id(2), Id(3)).unwrap();
		state.command_rejected(invite);
		assert!(!state.server_action_pending());
		state.gateway_connected = false;
		assert!(state.create_server_invite(Id(2), Id(3)).is_none());
		state.gateway_connected = true;
		let old = state.create_server_invite(Id(2), Id(3)).unwrap();
		let Command::ServerAction { action, request } = old else {
			unreachable!()
		};
		let generation = state.generation;
		state.logout();
		state.apply(Envelope {
			generation,
			event: CoreEvent::ServerAction(Event::Written {
				action,
				request,
				result: Ok(Some("old".into())),
			}),
		});
		assert_eq!(state.created_invite(Id(2)), None);
	}
}
