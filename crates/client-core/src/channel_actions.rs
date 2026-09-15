//! One deliberate channel operation at a time; Discord remains authoritative.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{
	Id, Patch,
	permissions::{MANAGE_CHANNELS, MANAGE_ROLES, VIEW_CHANNEL},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
	pub name: String,
	pub topic: String,
	pub slowmode: u32,
	pub nsfw: bool,
	pub overwrites: Vec<model::permissions::Overwrite>,
}
impl Edit {
	pub fn valid(&self) -> bool {
		valid_name(&self.name)
			&& self.name.capacity() <= 400
			&& self.topic.chars().count() <= 4096
			&& self.topic.capacity() <= 16384
			&& self.slowmode <= 21600
			&& !self.topic.contains('\0')
			&& self.overwrites.capacity() <= model::permissions::MAX_OVERWRITES
			&& self.overwrites.iter().enumerate().all(|(index, row)| {
				row.id.0 != 0
					&& row.kind <= 1
					&& !self.overwrites[..index]
						.iter()
						.any(|other| other.id == row.id)
			})
	}
	pub fn bytes(&self) -> usize {
		self.name.capacity()
			+ self.topic.capacity()
			+ self.overwrites.capacity() * size_of::<model::permissions::Overwrite>()
	}
}
pub fn valid_name(name: &str) -> bool {
	!name.trim().is_empty()
		&& name.chars().count() <= 100
		&& name.len() <= 400
		&& !name.chars().any(char::is_control)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mute {
	Unmute,
	For(u32),
	Forever,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
	Reference,
	Load,
	Edit { before: Edit, after: Edit },
	Duplicate { name: String },
	CreateText { name: String },
	CreateCategory { name: String },
	Delete,
	Mute(Mute),
	Notifications(u8),
}
impl Action {
	pub fn valid(&self) -> bool {
		match self {
			Self::Edit { before, after } => before.valid() && after.valid(),
			Self::Duplicate { name }
			| Self::CreateText { name }
			| Self::CreateCategory { name } => valid_name(name) && name.capacity() <= 400,
			Self::Mute(Mute::For(seconds)) => matches!(seconds, 900 | 3600 | 10800 | 28800 | 86400),
			Self::Notifications(level) => *level <= 3,
			_ => true,
		}
	}
}
pub enum Outcome {
	Details(Edit),
	Channel {
		channel: Box<model::Channel>,
		permissions: Option<model::permissions::Channel>,
	},
	Deleted,
	Preferences {
		muted: Option<bool>,
		level: Option<u8>,
		mute_until: Option<i64>,
	},
}
impl Outcome {
	pub fn bytes(&self) -> usize {
		match self {
			Self::Details(edit) => edit.bytes(),
			Self::Channel {
				channel,
				permissions,
			} => {
				channel.bytes()
					+ permissions
						.as_ref()
						.and_then(|p| p.overwrites.as_ref())
						.map_or(0, |o| {
							o.capacity() * size_of::<model::permissions::Overwrite>()
						})
			}
			_ => 0,
		}
	}
}
pub enum Event {
	Finished {
		guild: Id,
		channel: Id,
		request: u64,
		result: Result<Outcome, Failure>,
	},
}
#[derive(Default)]
pub struct Actions {
	sequence: u64,
	pending: Option<(Id, Id, u64, Action, bool)>,
	details: Option<(Id, Edit)>,
	status: Option<(Id, &'static str, bool)>,
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
	pub fn can_manage_channel(&self, channel: Id) -> bool {
		self.channel(channel)
			.is_some_and(|c| c.guild.is_some() && matches!(c.kind, 0 | 2 | 4 | 5 | 13 | 15 | 16))
			&& self.permission(channel, VIEW_CHANNEL | MANAGE_CHANNELS) == Some(true)
	}
	pub fn can_manage_channel_permissions(&self, channel: Id) -> bool {
		self.can_manage_channel(channel) && self.permission(channel, MANAGE_ROLES) == Some(true)
	}
	pub fn can_open_channel_settings(&self, channel: Id) -> bool {
		self.can_manage_channel(channel)
	}
	pub fn can_edit_channel_permission(&self, channel: Id, bits: u128) -> bool {
		if !self.can_manage_channel_permissions(channel) {
			return false;
		}
		let Some(source) = self.channel(channel) else {
			return false;
		};
		let Some(guild) = source.guild.and_then(|id| self.permissions.guilds.get(&id)) else {
			return false;
		};
		let Some(user) = self.user.as_ref().map(|u| u.id) else {
			return false;
		};
		// Discord permits guild/parent bits, or any bit when the actor has an
		// applicable MANAGE_ROLES channel overwrite. Existing unknown bits stay intact.
		let elevated = self
			.permissions
			.channels
			.get(&channel)
			.and_then(|c| c.overwrites.as_ref())
			.is_some_and(|rows| {
				rows.iter().any(|row| {
					row.allow & MANAGE_ROLES != 0
						&& match row.kind {
							0 => {
								row.id == guild.id
									|| guild
										.member
										.as_ref()
										.is_some_and(|m| m.roles.contains(&row.id))
							}
							1 => row.id == user,
							_ => false,
						}
				})
			});
		if elevated {
			return true;
		}
		let now = Self::permission_time();
		let Some(mut available) = model::permissions::effective(guild, user, Some(&[]), now) else {
			return false;
		};
		if let Some(parent) = source
			.parent_id
			.and_then(|id| self.channel(id))
			.filter(|p| p.guild == Some(guild.id) && p.kind == 4)
			&& let Some(overwrites) = self
				.permissions
				.channels
				.get(&parent.id)
				.filter(|p| p.guild == guild.id)
				.and_then(|p| p.overwrites.as_deref())
			&& let Some(parent_bits) =
				model::permissions::effective(guild, user, Some(overwrites), now)
		{
			available |= parent_bits;
		}
		available & bits == bits
	}
	fn channel_edit_allowed(&self, channel: Id, before: &Edit, after: &Edit) -> bool {
		let overview = before.name != after.name
			|| before.topic != after.topic
			|| before.slowmode != after.slowmode
			|| before.nsfw != after.nsfw;
		if before.topic != after.topic && after.topic.chars().count() > 1024 {
			return false;
		}
		if overview && !self.can_manage_channel(channel) {
			return false;
		}
		if (before.topic != after.topic
			|| before.slowmode != after.slowmode
			|| before.nsfw != after.nsfw)
			&& !self
				.channel(channel)
				.is_some_and(|c| matches!(c.kind, 0 | 5))
		{
			return false;
		}
		if before.overwrites != after.overwrites {
			if !self.can_manage_channel_permissions(channel) {
				return false;
			}
			for row in before.overwrites.iter().chain(&after.overwrites) {
				let old = before
					.overwrites
					.iter()
					.find(|o| o.id == row.id && o.kind == row.kind);
				let new = after
					.overwrites
					.iter()
					.find(|o| o.id == row.id && o.kind == row.kind);
				let changed = old.map_or(0, |o| o.allow) ^ new.map_or(0, |o| o.allow)
					| (old.map_or(0, |o| o.deny) ^ new.map_or(0, |o| o.deny));
				if !self.can_edit_channel_permission(channel, changed) {
					return false;
				}
			}
		}
		self.can_open_channel_settings(channel)
	}
	pub fn channel_action_pending(&self) -> bool {
		self.channel_actions.pending.is_some()
	}
	pub fn channel_details(&self, channel: Id) -> Option<&Edit> {
		self.channel_actions
			.details
			.as_ref()
			.filter(|(id, _)| *id == channel && self.can_open_channel_settings(channel))
			.map(|(_, edit)| edit)
	}
	pub fn channel_action_status(&self, channel: Id) -> Option<&'static str> {
		self.channel_actions
			.status
			.filter(|(id, _, _)| *id == channel)
			.map(|(_, status, _)| status)
	}
	pub fn channel_action_succeeded(&self, channel: Id) -> bool {
		self.channel_actions
			.status
			.is_some_and(|(id, _, success)| id == channel && success)
	}
	pub fn clear_channel_action_result(&mut self, channel: Id) {
		if self
			.channel_actions
			.status
			.is_some_and(|(id, _, _)| id == channel)
		{
			self.channel_actions.status = None;
		}
	}
	pub fn request_channel_action(&mut self, channel: Id, action: Action) -> Option<Command> {
		if self.channel_action_pending() {
			return None;
		}
		self.clear_channel_action_result(channel);
		let source = self.channel(channel)?;
		let guild = source.guild?;
		let personal = matches!(action, Action::Mute(_) | Action::Notifications(_));
		if !action.valid()
			|| !self.can_view(channel)
			|| (!personal
				&& !match &action {
					Action::Reference => false,
					Action::Load => self.can_open_channel_settings(channel),
					Action::Edit { before, after } => {
						self.channel_edit_allowed(channel, before, after)
					}
					_ => self.can_manage_channel(channel),
				}) {
			self.channel_actions.status =
				Some((channel, "Channel action unavailable or invalid", false));
			return None;
		}
		if !self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected) {
			self.channel_actions.status = Some((
				channel,
				"Channel actions unavailable while disconnected",
				false,
			));
			return None;
		}
		if matches!(action, Action::Delete)
			&& (self.pending.iter().any(|p| p.channel == channel)
				|| self
					.voice
					.active
					.as_ref()
					.is_some_and(|call| call.channel == channel))
		{
			self.channel_actions.status = Some((
				channel,
				"Finish pending messages and leave the call before deleting this channel",
				false,
			));
			return None;
		}
		if let Action::Edit { before, .. } = &action
			&& self.channel_details(channel) != Some(before)
		{
			self.channel_actions.status = Some((
				channel,
				"Channel settings changed; reopen the editor",
				false,
			));
			return None;
		}
		if matches!(action, Action::Load) && !self.demo {
			self.channel_actions.details = None;
		}
		self.channel_actions.sequence = self.channel_actions.sequence.wrapping_add(1);
		let request = self.channel_actions.sequence;
		self.channel_actions.pending = Some((guild, channel, request, action.clone(), false));
		Some(Command::ChannelAction {
			guild,
			channel,
			request,
			action,
		})
	}
	pub fn request_channel_reference(&mut self, guild: Id, channel: Id) -> Option<Command> {
		if self.channel_action_pending()
			|| self.channel(channel).is_some()
			|| self.channel_action_status(channel).is_some()
			|| self.guild(guild).is_none()
			|| self
				.selected
				.and_then(|id| self.channel(id))
				.and_then(|source| source.guild)
				!= Some(guild)
			|| (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
		{
			return None;
		}
		self.channel_actions.sequence = self.channel_actions.sequence.wrapping_add(1);
		let request = self.channel_actions.sequence;
		let action = Action::Reference;
		self.channel_actions.pending = Some((guild, channel, request, action.clone(), false));
		Some(Command::ChannelAction {
			guild,
			channel,
			request,
			action,
		})
	}
	pub(crate) fn cancel_channel_action(&mut self) {
		if let Some((_, channel, _, action, _)) = self.channel_actions.pending.take() {
			self.channel_actions.status = Some((
				channel,
				if matches!(action, Action::Reference | Action::Load) {
					"Channel load interrupted"
				} else {
					Failure::Ambiguous.label()
				},
				false,
			));
		}
		self.channel_actions.details = None;
	}
	pub(crate) fn observe_channel_action(&mut self, event: &crate::Event) {
		if let Some((channel, _)) = &self.channel_actions.details {
			let invalid = match event {
				crate::Event::ChannelChanged(p) => p.id == *channel,
				crate::Event::Unavailable(id) => id == channel,
				crate::Event::ChannelCreated(c) => c.id == *channel,
				crate::Event::Permissions(crate::permissions::Event::Channel {
					channel: id,
					..
				}) => id == channel,
				_ => false,
			};
			if invalid {
				self.channel_actions.details = None;
			}
		}
		if let Some((guild, channel, _, action, changed)) = &mut self.channel_actions.pending {
			*changed |= match event {
				crate::Event::ChannelChanged(p) => {
					p.id == *channel && !matches!(action, Action::Delete)
				}
				crate::Event::Unavailable(id) => id == channel && !matches!(action, Action::Delete),
				crate::Event::ChannelCreated(c) => c.id == *channel,
				crate::Event::Permissions(crate::permissions::Event::Channel {
					channel: id,
					..
				}) => id == channel && !matches!(action, Action::Delete),
				crate::Event::NotificationPreferences(crate::notifications::Event::Settings {
					entries,
					..
				}) if matches!(action, Action::Mute(_) | Action::Notifications(_)) => {
					entries.iter().any(|s| s.guild == Some(*guild))
				}
				_ => false,
			};
		}
	}
	pub(crate) fn apply_channel_action(&mut self, event: Event) -> Result<(), &'static str> {
		let Event::Finished {
			guild,
			channel,
			request,
			result,
		} = event;
		let Some((g, c, r, _, _)) = &self.channel_actions.pending else {
			return Ok(());
		};
		if (*g, *c, *r) != (guild, channel, request) {
			return Ok(());
		}
		let (_, _, _, action, observed) = self.channel_actions.pending.take().unwrap();
		let result = result.and_then(|outcome| {
			let valid = match (&action, &outcome) {
				(
					Action::Reference,
					Outcome::Channel {
						channel: target, ..
					},
				) => {
					target.id == channel
						&& target.guild == Some(guild)
						&& matches!(target.kind, 10..=12)
						&& target.parent_id.is_some_and(|parent| {
							self.channel(parent).is_some_and(|source| {
								source.guild == Some(guild) && self.can_view(parent)
							})
						})
				}
				(Action::Load, Outcome::Details(edit)) => edit.valid(),
				(Action::Edit { .. }, Outcome::Channel { channel: c, .. }) => {
					c.id == channel && c.guild == Some(guild)
				}
				(
					Action::Duplicate { .. }
					| Action::CreateText { .. }
					| Action::CreateCategory { .. },
					Outcome::Channel { channel: c, .. },
				) => c.id.0 != 0 && c.id != channel && c.guild == Some(guild),
				(Action::Delete, Outcome::Deleted) => true,
				(Action::Mute(mute), Outcome::Preferences { muted, .. }) => {
					*muted == Some(*mute != Mute::Unmute)
				}
				(Action::Notifications(wanted), Outcome::Preferences { level, .. }) => {
					*level == Some(*wanted)
				}
				_ => false,
			};
			if valid && outcome.bytes() <= 64 * 1024 {
				Ok(outcome)
			} else {
				Err(Failure::Ambiguous)
			}
		});
		let status = match result {
			Err(failure) => {
				if failure.ends_session() {
					self.fail(failure);
				}
				self.channel_actions.status = Some((channel, failure.label(), false));
				self.status = failure.label();
				return Ok(());
			}
			Ok(Outcome::Details(edit)) => {
				if self.can_open_channel_settings(channel) && !observed {
					self.channel_actions.details = Some((channel, edit));
				} else {
					self.channel_actions.status = Some((
						channel,
						"Channel changed while loading; reopen settings",
						false,
					));
				}
				return Ok(());
			}
			Ok(Outcome::Channel {
				channel: updated,
				permissions,
			}) => {
				let reference = action == Action::Reference;
				let creating = matches!(
					action,
					Action::Duplicate { .. }
						| Action::CreateText { .. }
						| Action::CreateCategory { .. }
				);
				if self.guild(guild).is_some()
					&& (if reference {
						true
					} else if creating {
						self.can_manage_channel(channel)
					} else {
						self.can_open_channel_settings(channel)
					}) && (if creating {
					self.channel(updated.id).is_none()
				} else {
					!observed
				}) {
					let target = updated.id;
					self.apply(crate::Envelope {
						generation: self.generation,
						event: crate::Event::ChannelCreated(*updated),
					});
					if reference {
						if self.channel(target).is_none() {
							self.channel_actions.status =
								Some((channel, "Thread could not be loaded", false));
							return Ok(());
						}
						self.retire_archived_thread(None);
						self.archived_thread = Some(target);
					}
					if let Some(p) = permissions.filter(|p| p.id == target && p.guild == guild) {
						self.apply(crate::Envelope {
							generation: self.generation,
							event: crate::Event::Permissions(crate::permissions::Event::Channel {
								channel: target,
								guild: Some(guild),
								overwrites: p.overwrites.map_or(Patch::Absent, Patch::Value),
							}),
						});
					}
				}
				if self.demo
					&& !observed && self.can_open_channel_settings(channel)
					&& let Action::Edit { after, .. } = &action
				{
					self.channel_actions.details = Some((channel, after.clone()));
				}
				if reference {
					"Thread loaded"
				} else if creating {
					"Channel created"
				} else {
					"Channel updated"
				}
			}
			Ok(Outcome::Deleted) => {
				if !observed {
					self.apply(crate::Envelope {
						generation: self.generation,
						event: crate::Event::Unavailable(channel),
					});
				}
				"Channel deleted"
			}
			Ok(Outcome::Preferences {
				muted,
				level,
				mute_until,
			}) => {
				if self.can_view(channel) && !observed {
					self.confirm_channel_preferences(guild, channel, muted, level)?;
				}
				if self.can_view(channel) && !observed {
					self.confirm_channel_mute_timer(channel, muted, mute_until)?;
				}
				"Notification settings updated"
			}
		};
		self.channel_actions.status = Some((channel, status, true));
		self.status = status;
		Ok(())
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event as CoreEvent};
	fn state() -> State {
		let mut state = State {
			demo: true,
			gateway_connected: true,
			user: Some(model::User {
				id: Id(1),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				id: Id(2),
				name: "Synthetic guild".into(),
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
			..State::default()
		};
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: vec![model::permissions::Guild {
					id: Id(2),
					owner: Some(Id(1)),
					roles: None,
					member: None,
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
	fn finish(state: &mut State, command: Command, result: Result<Outcome, Failure>) {
		let Command::ChannelAction {
			guild,
			channel,
			request,
			..
		} = command
		else {
			panic!()
		};
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::ChannelAction(Event::Finished {
				guild,
				channel,
				request,
				result,
			}),
		});
	}
	#[test]
	fn category_settings_preserve_overwrites_and_separate_edit_permissions() {
		let mut state = state();
		state.channels[0].kind = 4;
		let before = Edit {
			name: "Category".into(),
			overwrites: vec![model::permissions::Overwrite {
				id: Id(2),
				kind: 0,
				allow: 1 << 100,
				deny: 0,
			}],
			..Edit::default()
		};
		let load = state.request_channel_action(Id(3), Action::Load).unwrap();
		finish(&mut state, load, Ok(Outcome::Details(before.clone())));
		let after = Edit {
			name: "Renamed".into(),
			..before.clone()
		};
		assert!(
			state
				.request_channel_action(
					Id(3),
					Action::Edit {
						before: before.clone(),
						after
					}
				)
				.is_some()
		);
		state.cancel_channel_action();
		let load = state.request_channel_action(Id(3), Action::Load).unwrap();
		finish(&mut state, load, Ok(Outcome::Details(before.clone())));
		assert_eq!(
			state.channel_details(Id(3)).unwrap().overwrites,
			before.overwrites
		);
		let guild = state.permissions.guilds.get_mut(&Id(2)).unwrap();
		guild.owner = Some(Id(99));
		guild.roles = Some(vec![model::permissions::Role {
			id: Id(2),
			name: String::new(),
			color: 0,
			position: 0,
			hoist: false,
			bits: VIEW_CHANNEL | MANAGE_ROLES,
		}]);
		guild.member = Some(model::permissions::Member {
			roles: vec![],
			timeout_until: None,
		});
		state.permissions.clear_cache();
		assert!(!state.can_manage_channel(Id(3)));
		assert!(!state.can_open_channel_settings(Id(3)));
		assert!(!state.can_edit_channel_permission(Id(3), VIEW_CHANNEL));
		state
			.permissions
			.guilds
			.get_mut(&Id(2))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits |= MANAGE_CHANNELS;
		state.permissions.clear_cache();
		assert!(state.can_open_channel_settings(Id(3)));
		assert!(state.can_edit_channel_permission(Id(3), VIEW_CHANNEL));
		assert!(state.can_edit_channel_permission(Id(3), 0));
		assert!(!state.can_edit_channel_permission(Id(3), model::permissions::MANAGE_GUILD));
		let mut after = before.clone();
		after.topic = "Unsupported category topic".into();
		assert!(
			state
				.request_channel_action(
					Id(3),
					Action::Edit {
						before: before.clone(),
						after
					}
				)
				.is_none()
		);
		let mut after = before.clone();
		after.overwrites[0].deny |= VIEW_CHANNEL;
		assert!(
			state
				.request_channel_action(
					Id(3),
					Action::Edit {
						before: before.clone(),
						after: after.clone()
					}
				)
				.is_some()
		);
		state.cancel_channel_action();
		let load = state.request_channel_action(Id(3), Action::Load).unwrap();
		finish(&mut state, load, Ok(Outcome::Details(before.clone())));
		let mut stale = before.clone();
		stale.overwrites[0].allow = 0;
		assert!(
			state
				.request_channel_action(
					Id(3),
					Action::Edit {
						before: stale,
						after
					}
				)
				.is_none()
		);
		let mut invalid = before;
		invalid.overwrites.push(invalid.overwrites[0]);
		assert!(!invalid.valid());
	}

	#[test]
	fn thread_references_reuse_channel_loading_and_require_a_viewable_parent() {
		let mut valid = state();
		valid.selected = Some(Id(3));
		let command = valid.request_channel_reference(Id(2), Id(4)).unwrap();
		assert!(matches!(
			command,
			Command::ChannelAction {
				action: Action::Reference,
				..
			}
		));
		assert!(valid.request_channel_reference(Id(2), Id(4)).is_none());
		finish(
			&mut valid,
			command,
			Ok(Outcome::Channel {
				channel: Box::new(model::Channel {
					id: Id(4),
					guild: Some(Id(2)),
					parent_id: Some(Id(3)),
					kind: 11,
					name: "Synthetic thread".into(),
					position: 0,
					recipients: vec![],
					last_message: None,
					icon: None,
					member_list_id: None,
					message_count: None,
				}),
				permissions: None,
			}),
		);
		assert_eq!(valid.channel(Id(4)).unwrap().name, "Synthetic thread");
		assert_eq!(valid.archived_thread, Some(Id(4)));

		let mut state = state();
		state.selected = Some(Id(3));
		let command = state.request_channel_reference(Id(2), Id(4)).unwrap();
		finish(
			&mut state,
			command,
			Ok(Outcome::Channel {
				channel: Box::new(model::Channel {
					id: Id(4),
					guild: Some(Id(2)),
					parent_id: Some(Id(99)),
					kind: 11,
					name: "Unknown parent".into(),
					position: 0,
					recipients: vec![],
					last_message: None,
					icon: None,
					member_list_id: None,
					message_count: None,
				}),
				permissions: None,
			}),
		);
		assert!(state.channel(Id(4)).is_none());
		assert!(!state.channel_action_succeeded(Id(4)));
	}

	#[test]
	fn channel_writes_check_access_bounds_queue_failure_and_stale_completions() {
		let mut state = state();
		assert!(state.can_manage_channel(Id(3)));
		for action in [
			Action::Notifications(4),
			Action::Mute(Mute::For(1)),
			Action::Duplicate { name: " ".into() },
			Action::Edit {
				before: Edit {
					name: "general".into(),
					..Edit::default()
				},
				after: Edit {
					name: "valid".into(),
					topic: "x".repeat(1025),
					..Edit::default()
				},
			},
		] {
			assert!(state.request_channel_action(Id(3), action).is_none());
		}
		let pending = state.request_channel_action(Id(3), Action::Load).unwrap();
		assert!(
			state
				.request_channel_action(Id(3), Action::Delete)
				.is_none()
		);
		state.command_rejected(pending);
		assert!(!state.channel_action_pending());
		let pending = state.request_channel_action(Id(3), Action::Load).unwrap();
		state.cancel_channel_action();
		finish(
			&mut state,
			pending,
			Ok(Outcome::Details(Edit {
				name: "late".into(),
				..Edit::default()
			})),
		);
		assert!(state.channel_details(Id(3)).is_none());
		let pending = state
			.request_channel_action(
				Id(3),
				Action::CreateCategory {
					name: "projects".into(),
				},
			)
			.unwrap();
		let mut category = state.channels[0].clone();
		category.id = Id(4);
		category.kind = 4;
		category.name = "projects".into();
		finish(
			&mut state,
			pending,
			Ok(Outcome::Channel {
				channel: Box::new(category),
				permissions: None,
			}),
		);
		assert!(
			state
				.channel(Id(4))
				.is_some_and(|channel| channel.kind == 4)
		);
		state.permissions.guilds.get_mut(&Id(2)).unwrap().owner = Some(Id(9));
		state.permissions.clear_cache();
		assert!(
			state
				.request_channel_action(Id(3), Action::Delete)
				.is_none()
		);
	}
	#[test]
	fn delete_ack_after_rename_removes_channel_but_late_edit_does_not_resurrect_it() {
		let mut state = state();
		let pending = state.request_channel_action(Id(3), Action::Delete).unwrap();
		let mut renamed = state.channels[0].clone();
		renamed.name = "renamed".into();
		state.observe_channel_action(&CoreEvent::ChannelChanged(model::ChannelPatch {
			id: Id(3),
			name: Patch::Value("renamed".into()),
			icon: Patch::Absent,
			last_message: Patch::Absent,
			parent_id: Patch::Absent,
			position: Patch::Absent,
			kind: Patch::Absent,
			message_count: Patch::Absent,
		}));
		finish(&mut state, pending, Ok(Outcome::Deleted));
		assert!(state.channel(Id(3)).is_none());
		assert!(state.channel_action_succeeded(Id(3)));
		let mut state = self::state();
		state.channel_actions.details = Some((
			Id(3),
			Edit {
				name: "general".into(),
				..Edit::default()
			},
		));
		let pending = state
			.request_channel_action(
				Id(3),
				Action::Edit {
					before: Edit {
						name: "general".into(),
						..Edit::default()
					},
					after: Edit {
						name: "rename".into(),
						..Edit::default()
					},
				},
			)
			.unwrap();
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Unavailable(Id(3)),
		});
		finish(
			&mut state,
			pending,
			Ok(Outcome::Channel {
				channel: Box::new(renamed),
				permissions: None,
			}),
		);
		assert!(state.channel(Id(3)).is_none());
	}
	#[test]
	fn timed_mutes_expire_for_delivery_and_matching_gateway_echo_preserves_timer() {
		let mut state = state();
		let setting = crate::notifications::Setting {
			guild: Some(Id(2)),
			muted: Some(false),
			level: Some(0),
			channels: vec![(Id(3), Some(true), Some(3))],
			channel_mute_until: vec![(Id(3), State::permission_time() - 1)],
			..Default::default()
		};
		state
			.apply_notification_preferences(crate::notifications::Event::Settings {
				entries: vec![setting.clone()],
				replace: true,
			})
			.unwrap();
		state
			.apply_notification_preferences(crate::notifications::Event::Presence(Some(false)))
			.unwrap();
		state
			.confirm_channel_mute_timer(Id(3), Some(true), Some(State::permission_time() - 1))
			.unwrap();
		state
			.apply_notification_preferences(crate::notifications::Event::Settings {
				entries: vec![setting.clone()],
				replace: false,
			})
			.unwrap();
		assert_eq!(state.guild_channel_muted(Id(3)), Some(false));
		assert!(state.notification_allowed(Id(3)));
		let mut permanent = setting;
		permanent.channel_mute_until.clear();
		state
			.apply_notification_preferences(crate::notifications::Event::Settings {
				entries: vec![permanent.clone()],
				replace: false,
			})
			.unwrap();
		assert!(!state.notification_allowed(Id(3)));
		state
			.confirm_channel_mute_timer(Id(3), Some(true), Some(State::permission_time() - 1))
			.unwrap();
		state
			.apply_notification_preferences(crate::notifications::Event::Settings {
				entries: vec![permanent],
				replace: true,
			})
			.unwrap();
		assert_eq!(state.guild_channel_muted(Id(3)), Some(true));
		assert!(!state.notification_allowed(Id(3)));
	}
}
