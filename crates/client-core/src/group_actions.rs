//! One explicit group write at a time; image data is never retained in state or echoed back.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
	user_actions::MAX_GROUP_RECIPIENTS,
};
use model::{ChannelPatch, Id, Patch};

pub const MAX_ICON_DATA_URI: usize = 22 + 4 * (256_usize * 1024).div_ceil(3);

pub enum Action {
	Leave(Id),
	AddRecipient {
		channel: Id,
		user: Id,
	},
	Edit {
		channel: Id,
		name: Option<String>,
		icon: Patch<String>,
	},
}

impl Action {
	pub fn channel(&self) -> Id {
		match self {
			Self::Leave(channel)
			| Self::Edit { channel, .. }
			| Self::AddRecipient { channel, .. } => *channel,
		}
	}
	pub fn valid(&self) -> bool {
		self.channel().0 != 0
			&& match self {
				Self::Leave(_) => true,
				Self::AddRecipient { user, .. } => user.0 != 0,
				Self::Edit { name, icon, .. } => {
					(name.is_some() || !matches!(icon, Patch::Absent))
						&& name.as_ref().is_none_or(|name| {
							!name.trim().is_empty()
								&& name.chars().count() <= 100
								&& name.capacity() <= 400
								&& !name.chars().any(char::is_control)
						}) && match icon {
						Patch::Value(uri) => {
							uri.capacity() <= MAX_ICON_DATA_URI
								&& uri
									.strip_prefix("data:image/png;base64,")
									.is_some_and(|data| {
										!data.is_empty()
											&& data.bytes().all(|b| {
												b.is_ascii_alphanumeric() || b"+/=".contains(&b)
											})
									})
						}
						_ => true,
					}
				}
			}
	}
}
pub enum Event {
	Written {
		channel: Id,
		request: u64,
		result: Result<Option<ChannelPatch>, Failure>,
	},
}
#[derive(Clone, Copy)]
enum PendingKind {
	Leave,
	Edit { renaming: bool },
	AddRecipient(Id),
}

#[derive(Clone, Copy)]
struct Pending {
	channel: Id,
	request: u64,
	kind: PendingKind,
	observed: bool,
}

#[derive(Default)]
pub struct Actions {
	sequence: u64,
	pending: Option<Pending>,
	status: Option<(Id, &'static str)>,
	completed: Option<(Id, u64, bool)>,
}
impl Actions {
	pub(crate) fn reset(&mut self) {
		*self = Self {
			sequence: self.sequence,
			// READY can follow a canceled write without changing session generation.
			// Keep its bounded outcome so an open editor can leave the busy state.
			completed: self.completed,
			status: self.status,
			..Self::default()
		};
	}
}
impl State {
	pub fn group_action_pending(&self) -> bool {
		self.group_actions.pending.is_some()
	}
	pub fn group_action_status(&self, channel: Id) -> Option<&'static str> {
		self.group_actions
			.status
			.filter(|(id, _)| *id == channel)
			.map(|(_, text)| text)
	}
	pub fn group_action_completed(&self, channel: Id, request: u64) -> Option<bool> {
		self.group_actions
			.completed
			.filter(|(id, seq, _)| *id == channel && *seq == request)
			.map(|(_, _, success)| success)
	}
	pub fn clear_group_action_result(&mut self, channel: Id) {
		if self
			.group_actions
			.status
			.is_some_and(|(id, _)| id == channel)
		{
			self.group_actions.status = None;
		}
	}
	pub fn is_group_dm(&self, channel: Id) -> bool {
		self.channel(channel)
			.is_some_and(|c| c.guild.is_none() && c.kind == 3)
	}
	/// Remaining seats besides the signed-in account, which some snapshots include.
	pub fn group_dm_slots(&self, channel: Id) -> Option<usize> {
		let owner = self.user.as_ref()?.id;
		let group = self
			.channel(channel)
			.filter(|c| c.guild.is_none() && c.kind == 3)?;
		Some(
			MAX_GROUP_RECIPIENTS.saturating_sub(
				group
					.recipients
					.iter()
					.filter(|user| user.id != owner)
					.count(),
			),
		)
	}
	pub fn group_recipient_eligible(&self, channel: Id, user: Id) -> bool {
		self.dm_target_allowed(user, true)
			&& self.channel(channel).is_some_and(|group| {
				group.guild.is_none()
					&& group.kind == 3
					&& group.recipients.iter().all(|member| member.id != user)
			})
	}
	pub fn add_group_recipient(&mut self, channel: Id, user: Id) -> Option<Command> {
		if self.selected != Some(channel)
			|| self.group_dm_slots(channel).is_none_or(|slots| slots == 0)
			|| !self.group_recipient_eligible(channel, user)
		{
			return None;
		}
		self.request_group_action(Action::AddRecipient { channel, user })
	}
	pub fn leave_group_reason(&self, channel: Id) -> Option<&'static str> {
		if !self.is_group_dm(channel) {
			return Some("Group is no longer available");
		}
		if self.pending.iter().any(|p| p.channel == channel)
			|| self
				.voice
				.active
				.as_ref()
				.is_some_and(|call| call.channel == channel)
		{
			return Some("Finish pending messages and leave the call before leaving this group");
		}
		None
	}
	pub fn leave_group(&mut self, channel: Id) -> Option<Command> {
		if let Some(reason) = self.leave_group_reason(channel) {
			self.group_actions.status = Some((channel, reason));
			return None;
		}
		self.request_group_action(Action::Leave(channel))
	}
	pub fn edit_group(
		&mut self,
		channel: Id,
		name: Option<String>,
		icon: Patch<String>,
	) -> Option<Command> {
		self.request_group_action(Action::Edit {
			channel,
			name,
			icon,
		})
	}
	fn request_group_action(&mut self, action: Action) -> Option<Command> {
		let channel = action.channel();
		if self.group_action_pending() || !self.is_group_dm(channel) {
			return None;
		}
		if !action.valid() {
			self.group_actions.status = Some((
				channel,
				"Use a group name of 1–100 characters and a supported icon",
			));
			return None;
		}
		if !self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected) {
			self.group_actions.status =
				Some((channel, "Group actions unavailable while disconnected"));
			return None;
		}
		self.group_actions.sequence = self.group_actions.sequence.wrapping_add(1);
		let request = self.group_actions.sequence;
		self.group_actions.pending = Some(Pending {
			channel,
			request,
			kind: match &action {
				Action::Leave(_) => PendingKind::Leave,
				Action::Edit { name, .. } => PendingKind::Edit {
					renaming: name.is_some(),
				},
				Action::AddRecipient { user, .. } => PendingKind::AddRecipient(*user),
			},
			observed: false,
		});
		self.group_actions.status = None;
		self.group_actions.completed = None;
		Some(Command::GroupAction { action, request })
	}
	pub(crate) fn cancel_group_action(&mut self) {
		if let Some(Pending {
			channel, request, ..
		}) = self.group_actions.pending.take()
		{
			self.group_actions.status =
				Some((channel, "Outcome unknown; check Discord before retrying"));
			self.group_actions.completed = Some((channel, request, false));
		}
	}
	pub(crate) fn observe_group_change(&mut self, channel: Id, recreated: bool) {
		if let Some(pending) = &mut self.group_actions.pending
			&& pending.channel == channel
			&& (recreated || matches!(pending.kind, PendingKind::Edit { .. }))
		{
			pending.observed = true;
		}
	}
	pub(crate) fn observe_group_recipient(&mut self, channel: Id, user: Id) {
		if let Some(pending) = &mut self.group_actions.pending
			&& pending.channel == channel
			&& matches!(pending.kind, PendingKind::AddRecipient(target) if target == user)
		{
			pending.observed = true;
		}
	}
	pub(crate) fn apply_group_action(&mut self, event: Event) -> Result<(), &'static str> {
		let Event::Written {
			channel,
			request,
			result,
		} = event;
		let Some(pending) = self.group_actions.pending else {
			return Ok(());
		};
		if channel != pending.channel || request != pending.request {
			return Ok(());
		}
		self.group_actions.pending = None;
		let result = result.and_then(|patch| {
			let PendingKind::Edit { renaming } = pending.kind else {
				return patch.is_none().then_some(None).ok_or(Failure::Ambiguous);
			};
			if patch.as_ref().is_some_and(|p| {
				p.id == channel
					&& match &p.name {
						Patch::Value(name) => {
							!name.is_empty()
								&& name.chars().count() <= 100
								&& name.capacity() <= 400
						}
						Patch::Absent => !renaming,
						Patch::Null => false,
					} && match &p.icon {
					Patch::Value(hash) => model::valid_avatar_hash(hash) && hash.capacity() <= 128,
					Patch::Null => true,
					Patch::Absent => false,
				}
			}) {
				Ok(patch)
			} else {
				Err(Failure::Ambiguous)
			}
		});
		self.group_actions.completed = Some((channel, request, result.is_ok()));
		let status = match (result, pending.kind) {
			(Err(failure), _) => {
				if failure.ends_session() {
					self.fail(failure);
				}
				failure.label()
			}
			(Ok(_), PendingKind::AddRecipient(user)) => {
				// The successful empty response confirms the write. A newer recipient event
				// or group snapshot already has the authoritative roster.
				if !pending.observed
					&& self.group_dm_slots(channel).is_some_and(|slots| slots > 0)
					&& self.group_recipient_eligible(channel, user)
					&& let Some(friend) = self.friend(user).cloned()
					&& !self.apply_dm_recipient_added(channel, friend)
				{
					return Ok(());
				}
				"Request completed; latest group members shown"
			}
			(Ok(_), _) if pending.observed => "Request completed; latest group settings shown",
			(Ok(Some(patch)), _) => {
				if let Some(group) = self
					.channels
					.iter_mut()
					.find(|c| c.id == channel && c.kind == 3 && c.guild.is_none())
				{
					if let Patch::Value(name) = patch.name {
						group.name = name;
					}
					group.icon = match patch.icon {
						Patch::Value(hash) => Some(hash),
						_ => None,
					};
					self.invalidate_navigation();
				}
				"Group updated"
			}
			(Ok(None), _) => {
				self.remove_channels(&std::collections::BTreeSet::from([channel]));
				if self.selected == Some(channel) {
					self.arrived_home();
				}
				"Left group"
			}
		};
		self.group_actions.status = Some((channel, status));
		self.status = status;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event as CoreEvent};
	fn state() -> State {
		State {
			demo: true,
			selected: Some(Id(10)),
			channels: vec![model::Channel {
				user_limit: None,
				slowmode: 0,
				id: Id(10),
				guild: None,
				kind: 3,
				name: "Group".into(),
				icon: None,
				last_message: None,
				parent_id: None,
				position: 0,
				recipients: vec![],
				member_list_id: None,
				tags: None,
				message_count: None,
			}],
			..State::default()
		}
	}
	fn patch(name: &str) -> ChannelPatch {
		ChannelPatch {
			user_limit: Patch::Absent,
			slowmode: Patch::Absent,
			id: Id(10),
			name: Patch::Value(name.into()),
			icon: Patch::Null,
			last_message: Patch::Absent,
			parent_id: Patch::Absent,
			position: Patch::Absent,
			kind: Patch::Absent,
			message_count: Patch::Absent,
			tags: Patch::Absent,
		}
	}
	fn finish(state: &mut State, command: Command, result: Result<Option<ChannelPatch>, Failure>) {
		let Command::GroupAction { action, request } = command else {
			panic!("wrong command");
		};
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::GroupAction(Event::Written {
				channel: action.channel(),
				request,
				result,
			}),
		});
	}
	#[test]
	fn group_actions_confirm_writes_keep_drafts_and_respect_newer_service_state() {
		{
			let mut state = state();
			assert!(state.close_dm(Id(10)).is_none());
			assert!(state.set_dm_muted(Id(10), true).is_some());
			assert!(
				state
					.edit_group(Id(10), Some(" ".into()), Patch::Absent)
					.is_none()
			);
			assert!(
				state
					.edit_group(Id(10), Some("x".repeat(101)), Patch::Absent)
					.is_none()
			);
			let edit = state
				.edit_group(Id(10), Some("New name".into()), Patch::Absent)
				.unwrap();
			assert_eq!(state.channel(Id(10)).unwrap().name, "Group");
			assert!(state.leave_group(Id(10)).is_none());
			finish(&mut state, edit, Err(Failure::Forbidden));
			assert_eq!(state.group_action_completed(Id(10), 1), Some(false));
			assert_eq!(state.channel(Id(10)).unwrap().name, "Group");
			let edit = state
				.edit_group(Id(10), Some("New name".into()), Patch::Absent)
				.unwrap();
			finish(&mut state, edit, Ok(Some(patch("New name"))));
			assert_eq!(state.channel(Id(10)).unwrap().name, "New name");
			assert_eq!(state.group_action_completed(Id(10), 2), Some(true));
			let edit = state
				.edit_group(Id(10), Some("Older".into()), Patch::Absent)
				.unwrap();
			state.apply(Envelope {
				generation: state.generation,
				event: CoreEvent::ChannelChanged(patch("Latest")),
			});
			finish(&mut state, edit, Ok(Some(patch("Older"))));
			assert_eq!(state.channel(Id(10)).unwrap().name, "Latest");
			let stale = state.leave_group(Id(10)).unwrap();
			state.cancel_group_action();
			let current = state.leave_group(Id(10)).unwrap();
			finish(&mut state, stale, Ok(None));
			assert!(state.group_action_pending());
			state.drafts.insert(Id(10), "Keep draft".into());
			finish(&mut state, current, Ok(None));
			assert!(state.channel(Id(10)).is_none());
			assert_eq!(state.selected, None);
			assert_eq!(state.drafts[&Id(10)], "Keep draft");
		}
		{
			let mut state = state();
			let name = "A long recipient name, ".repeat(8);
			state.channels[0].name = name.clone();
			state.channels[0].icon = Some("0123456789abcdef0123456789abcdef".into());
			assert!(state.edit_group(Id(10), None, Patch::Absent).is_none());
			let edit = state.edit_group(Id(10), None, Patch::Null).unwrap();
			let mut response = patch("unused");
			response.name = Patch::Absent;
			finish(&mut state, edit, Ok(Some(response)));
			assert_eq!(state.channels[0].name, name);
			assert!(state.channels[0].icon.is_none());
			let rename = state
				.edit_group(Id(10), Some("Renamed".into()), Patch::Absent)
				.unwrap();
			let mut response = patch("unused");
			response.name = Patch::Absent;
			finish(&mut state, rename, Ok(Some(response)));
			assert_eq!(state.group_action_completed(Id(10), 2), Some(false));
			assert_eq!(state.channels[0].name, name);
		}
	}
	#[test]
	fn adding_a_friend_checks_scope_and_capacity_and_keeps_newer_recipient_events() {
		let mut state = state();
		let make_user = |id| model::User {
			id: Id(id),
			name: format!("Synthetic {id}"),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			username: None,
			primary_guild: None,
			cosmetics: None,
		};
		let owner = make_user(1);
		let friend = make_user(2);
		let other_friend = make_user(3);
		state.user = Some(owner.clone());
		let apply = |state: &mut State, event| {
			state.apply(Envelope {
				generation: state.generation,
				event,
			})
		};
		apply(
			&mut state,
			CoreEvent::UserAction(crate::user_actions::Event::Relationships(Some(vec![]))),
		);
		apply(
			&mut state,
			CoreEvent::UserAction(crate::user_actions::Event::Friends(Some(vec![
				(friend.clone(), "friend".into()),
				(other_friend.clone(), "other_friend".into()),
			]))),
		);
		assert!(state.add_group_recipient(Id(10), owner.id).is_none());
		assert!(state.add_group_recipient(Id(10), Id(99)).is_none());
		state.selected = Some(Id(11));
		assert!(state.add_group_recipient(Id(10), friend.id).is_none());
		state.selected = Some(Id(10));
		state.channels[0].recipients.push(owner);
		assert_eq!(state.group_dm_slots(Id(10)), Some(MAX_GROUP_RECIPIENTS));
		state.channels[0].recipients.extend((11..20).map(make_user));
		assert!(state.add_group_recipient(Id(10), friend.id).is_none());
		state.channels[0].recipients.truncate(1);
		let add = state.add_group_recipient(Id(10), friend.id).unwrap();
		assert!(state.add_group_recipient(Id(10), other_friend.id).is_none());
		finish(&mut state, add, Err(Failure::Forbidden));
		assert!(
			!state
				.channel(Id(10))
				.unwrap()
				.recipients
				.iter()
				.any(|user| user.id == friend.id)
		);
		let add = state.add_group_recipient(Id(10), friend.id).unwrap();
		finish(&mut state, add, Ok(None));
		assert!(
			state
				.channel(Id(10))
				.unwrap()
				.recipients
				.iter()
				.any(|user| user.id == friend.id)
		);
		assert!(state.add_group_recipient(Id(10), friend.id).is_none());
		let add = state.add_group_recipient(Id(10), other_friend.id).unwrap();
		apply(
			&mut state,
			CoreEvent::RecipientAdded {
				channel: Id(10),
				user: other_friend.clone(),
			},
		);
		apply(
			&mut state,
			CoreEvent::RecipientRemoved {
				channel: Id(10),
				user: other_friend.id,
			},
		);
		finish(&mut state, add, Ok(None));
		assert!(
			!state
				.channel(Id(10))
				.unwrap()
				.recipients
				.iter()
				.any(|user| user.id == other_friend.id)
		);
	}

	#[test]
	fn group_leave_guards_rejoined_channel_pending_messages_and_session_reset() {
		let mut state = state();
		state.pending.push(crate::Pending {
			sticker: None,
			channel: Id(10),
			nonce: "pending".into(),
			content: "pending".into(),
			attachments: vec![],
			delivery: crate::Delivery::Sending,
			confirmed: None,
			reply: None,
		});
		assert!(state.leave_group(Id(10)).is_none());
		state.pending.clear();
		let leave = state.leave_group(Id(10)).unwrap();
		let channel = state.channel(Id(10)).unwrap().clone();
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Unavailable(Id(10)),
		});
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::ChannelCreated(channel),
		});
		finish(&mut state, leave, Ok(None));
		assert!(state.channel(Id(10)).is_some());
		let edit = state
			.edit_group(Id(10), Some("New name".into()), Patch::Absent)
			.unwrap();
		state.command_rejected(edit);
		assert!(!state.group_action_pending());
		let interrupted = state
			.edit_group(Id(10), Some("Interrupted".into()), Patch::Absent)
			.unwrap();
		let request = match &interrupted {
			Command::GroupAction { request, .. } => *request,
			_ => unreachable!(),
		};
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Ready {
				user: model::User {
					username: None,
					primary_guild: None,
					cosmetics: None,
					id: Id(1),
					name: "Synthetic".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				},
				channels: state.channels.clone(),
				guilds: vec![],
				permissions: Default::default(),
			},
		});
		assert_eq!(state.group_action_completed(Id(10), request), Some(false));
		finish(&mut state, interrupted, Ok(Some(patch("Interrupted"))));
		assert_eq!(state.channel(Id(10)).unwrap().name, "Group");
		state.demo = false;
		state.gateway_connected = false;
		assert!(state.leave_group(Id(10)).is_none());
		state.demo = true;
		let leave = state.leave_group(Id(10)).unwrap();
		let Command::GroupAction { action, request } = leave else {
			unreachable!()
		};
		let generation = state.generation;
		state.logout();
		state.apply(Envelope {
			generation,
			event: CoreEvent::GroupAction(Event::Written {
				channel: action.channel(),
				request,
				result: Ok(None),
			}),
		});
		assert_eq!(state.group_action_completed(Id(10), request), None);
	}
}
