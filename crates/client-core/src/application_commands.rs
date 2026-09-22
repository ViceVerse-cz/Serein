//! One active conversation's received application commands. Never persisted or auto-run.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
	interactions,
};
use model::{Freshness, Id, application_commands as schema};

// The account command index supplies the current user's override, role overrides,
// and channel overrides. Threads inherit the parent channel's command rules.
// https://docs.discord.com/developers/interactions/application-commands#permissions
fn overrides(
	permissions: &schema::CommandPermissions,
	roles: &[Id],
	guild: Id,
	channel: Id,
) -> (Option<bool>, Option<bool>) {
	let target = permissions.user.or_else(|| {
		if permissions.roles.is_empty() {
			return None;
		}
		roles
			.iter()
			.filter_map(|id| permissions.roles.get(id))
			.copied()
			.reduce(|allowed, next| allowed || next)
			.or_else(|| permissions.roles.get(&guild).copied())
	});
	let location = permissions
		.channels
		.get(&channel)
		.or_else(|| permissions.channels.get(&Id(guild.0 - 1)))
		.copied();
	(target, location)
}

/// One guild's (or one bot DM's) command index. Permissions are re-evaluated live from
/// `State::permissions`, so member, role and channel updates never invalidate the index;
/// only a scope change, a session reset or an explicit refresh replaces it.
#[derive(Default)]
pub struct Catalog {
	/// Guild id for guild channels, the channel id for a bot DM.
	pub scope: Option<Id>,
	pub request: u64,
	pub loading: bool,
	pub commands: Vec<schema::Command>,
	pub error: Option<&'static str>,
}
impl Catalog {
	pub fn clear(&mut self) {
		self.request = self.request.wrapping_add(1);
		self.scope = None;
		self.loading = false;
		self.commands = Vec::new();
		self.error = None;
	}
	/// Drop the index only when it belongs to another guild or conversation.
	pub fn retain(&mut self, scope: Option<Id>) {
		if self.scope.is_some() && self.scope != scope {
			self.clear();
		}
	}
}

impl State {
	/// Shared by command discovery and submission; Discord remains authoritative.
	pub fn can_use_application_command(&self, channel: Id, command: &schema::Command) -> bool {
		if command.kind != 1 || !self.can_request_application_commands(channel) {
			return false;
		}
		let Some(channel) = self.channel(channel) else {
			return false;
		};
		let context = u8::from(channel.guild.is_none());
		if command.guild_id.is_some() && command.guild_id != channel.guild
			|| command
				.contexts
				.as_ref()
				.is_some_and(|contexts| !contexts.contains(&context))
		{
			return false;
		}
		let Some(guild) = channel.guild else {
			return true;
		};
		if self.guild_permission(guild, model::permissions::ADMINISTRATOR) {
			return true;
		}
		let Some(member) = self
			.permissions
			.guilds
			.get(&guild)
			.and_then(|g| g.member.as_ref())
		else {
			return false;
		};
		let Some(target_channel) = self.overwrite_target(channel) else {
			return false;
		};
		let mut allowed = command
			.default_member_permissions
			.is_none_or(|bits| bits != 0 && self.permission(channel.id, bits) == Some(true));
		let (app_target, app_location) = overrides(
			&command.application_permissions,
			&member.roles,
			guild,
			target_channel,
		);
		// An application allow preserves the command's default member requirement;
		// an explicit command overwrite can replace that requirement.
		if app_target == Some(false) {
			allowed = false;
		}
		let (command_target, command_location) =
			overrides(&command.permissions, &member.roles, guild, target_channel);
		command_target.unwrap_or(allowed) && command_location.or(app_location).unwrap_or(true)
	}
	pub fn can_request_application_commands(&self, channel: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness != Freshness::Unavailable
			&& self.selected == Some(channel)
			&& self.can_compose(channel)
			&& self.channel(channel).is_some_and(|c| {
				c.supports_text()
					&& if c.guild.is_some() {
						self.permission(channel, model::permissions::USE_APPLICATION_COMMANDS)
							== Some(true)
					} else {
						c.kind == 1
							&& c.recipients.iter().any(|u| {
								matches!(u.kind, model::AccountKind::Bot | model::AccountKind::App)
							})
					}
			})
	}
	/// The index shared by every channel of a guild; bot DMs have their own.
	pub fn application_command_scope(&self, channel: Id) -> Option<Id> {
		self.channel(channel).map(|c| c.guild.unwrap_or(channel))
	}
	/// The current index covers this channel (loaded, loading or failed).
	pub fn application_commands_cover(&self, channel: Id) -> bool {
		let scope = self.application_command_scope(channel);
		scope.is_some() && self.application_commands.scope == scope
	}
	pub fn request_application_commands(&mut self, channel: Id, refresh: bool) -> Option<Command> {
		if !self.can_request_application_commands(channel) {
			return None;
		}
		let catalog = &self.application_commands;
		if self.application_commands_cover(channel) && (catalog.loading || !refresh) {
			return None;
		}
		let guild = self.channel(channel)?.guild;
		let scope = self.application_command_scope(channel);
		let catalog = &mut self.application_commands;
		catalog.clear();
		catalog.scope = scope;
		catalog.loading = true;
		Some(Command::ApplicationCommands {
			channel,
			guild,
			request: catalog.request,
		})
	}
	pub fn apply_application_commands(
		&mut self,
		channel: Id,
		request: u64,
		result: Result<Vec<schema::Command>, Failure>,
	) {
		// The reply may land after a switch to a sibling channel; the guild index still applies.
		if self.auth != AuthState::Authenticated
			|| !self.application_commands_cover(channel)
			|| self.application_commands.request != request
			|| !self.application_commands.loading
		{
			return;
		}
		self.application_commands.loading = false;
		match result {
			Ok(commands) => {
				let guild = self.channel(channel).and_then(|c| c.guild);
				let context = if guild.is_some() { 0 } else { 1 };
				if !schema::valid_catalog(&commands)
					|| schema::catalog_bytes(&commands)
						+ (commands.capacity() - commands.len()) * size_of::<schema::Command>()
						> schema::MAX_CATALOG_BYTES
					|| commands.iter().any(|command| {
						command.guild_id.is_some() && command.guild_id != guild
							|| command
								.contexts
								.as_ref()
								.is_some_and(|contexts| !contexts.contains(&context))
					}) {
					self.application_commands.error =
						Some("Application command list is invalid or exceeds its limits");
				} else {
					self.application_commands.commands = commands;
					self.application_commands.error = None;
				}
			}
			Err(failure) => {
				self.application_commands.error = Some(failure.label());
				if failure.ends_session() {
					self.fail(failure);
				}
			}
		}
		self.revision = self.revision.wrapping_add(1);
	}
	pub fn prepare_application_command(
		&mut self,
		command_id: Id,
		path: &[String],
		values: &[(String, String)],
	) -> Result<Command, &'static str> {
		let channel = self.selected.ok_or("Choose a conversation first")?;
		if !self.can_request_application_commands(channel) || !self.interactions_allowed() {
			return Err("Application commands are unavailable in this conversation");
		}
		if self.interactions.busy() || self.interactions.modal.is_some() {
			return Err("Finish the current application interaction first");
		}
		let catalog = &self.application_commands;
		if !self.application_commands_cover(channel) || catalog.loading || catalog.error.is_some() {
			return Err("Refresh the application command list first");
		}
		let command = catalog
			.commands
			.iter()
			.find(|c| c.id == command_id)
			.ok_or("This command is no longer available")?;
		if !self.can_use_application_command(channel, command) {
			return Err("You don't have permission to use this command here");
		}
		let guild = self.channel(channel).and_then(|c| c.guild);
		let invocation = command.invocation(path, values)?;
		for option in command.options_at(path)?.iter().filter(|o| o.kind == 7) {
			if let Some((_, value)) = values
				.iter()
				.find(|(name, value)| *name == option.name && !value.is_empty())
			{
				let id = value
					.parse::<Id>()
					.map_err(|_| "Enter a channel identifier")?;
				if !self.channel(id).is_some_and(|c| {
					c.guild == guild
						&& self.can_view(id)
						&& (option.channel_types.is_empty()
							|| option.channel_types.contains(&c.kind))
				}) {
					return Err("Choose an accessible channel of the requested type");
				}
			}
		}
		let application = command.application_id;
		self.begin_interaction(
			application,
			None,
			0,
			interactions::Data::ApplicationCommand {
				invocation: Box::new(invocation),
			},
		)
		.ok_or("Application interaction could not be started")
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	use model::{AccountKind, Channel, User};
	fn bot_state() -> State {
		let bot = User {
			id: Id(3),
			name: "Synthetic app".into(),
			kind: AccountKind::Bot,
			webhook: false,
			avatar: None,
			discriminator: 0,
			primary_guild: None,
		};
		State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			selected: Some(Id(2)),
			channels: vec![Channel {
				id: Id(2),
				guild: None,
				name: "Synthetic bot DM".into(),
				kind: 1,
				recipients: vec![bot],
				icon: None,
				last_message: None,
				parent_id: None,
				position: 0,
				member_list_id: None,
				message_count: None,
			}],
			..State::default()
		}
	}
	fn command() -> schema::Command {
		schema::Command {
			id: Id(4),
			version: Id(5),
			application_id: Id(3),
			guild_id: None,
			kind: 1,
			name: "sample".into(),
			description: "Synthetic command".into(),
			application_name: "Synthetic app".into(),
			application_icon: None,
			default_member_permissions: None,
			permissions: Default::default(),
			application_permissions: Default::default(),
			contexts: Some(vec![1]),
			integration_types: None,
			options: vec![schema::CommandOption {
				kind: 1,
				name: "run".into(),
				description: "Run sample".into(),
				options: vec![
					schema::CommandOption {
						kind: 4,
						name: "count".into(),
						description: "A bounded count".into(),
						required: true,
						min_value: Some(1.0),
						max_value: Some(3.0),
						choices: vec![schema::Choice {
							name: "Two".into(),
							value: schema::Value::Integer(2),
						}],
						..Default::default()
					},
					schema::CommandOption {
						kind: 11,
						name: "file".into(),
						description: "An unsupported attachment".into(),
						..Default::default()
					},
				],
				..Default::default()
			}],
		}
	}
	#[test]
	fn command_permissions_filter_defaults_overrides_threads_and_submission() {
		use model::permissions as p;
		let mut state = bot_state();
		state.user = Some(state.channels[0].recipients[0].clone());
		state.channels[0].guild = Some(Id(10));
		state.channels[0].kind = 0;
		state.guilds.push(model::Guild {
			id: Id(10),
			name: "Synthetic guild".into(),
			icon: None,
			emojis: None,
			stickers: None,
		});
		let role = |id, bits| p::Role {
			id: Id(id),
			bits,
			name: "Synthetic role".into(),
			color: 0,
			position: 0,
			hoist: false,
		};
		state
			.permissions
			.replace(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(10),
					owner: Some(Id(99)),
					roles: Some(vec![
						role(10, p::VIEW_CHANNEL | p::USE_APPLICATION_COMMANDS),
						role(11, 0),
						role(12, 0),
					]),
					member: Some(p::Member {
						roles: vec![Id(11), Id(12)],
						timeout_until: None,
					}),
				}],
				channels: vec![p::Channel {
					id: Id(2),
					guild: Id(10),
					overwrites: Some(vec![]),
				}],
			})
			.unwrap();
		let mut command = command();
		command.contexts = None;
		assert!(!state.can_use_application_command(Id(2), &command));
		assert!(!state.can_compose(Id(2)));
		state
			.update_permissions(crate::permissions::Event::Role {
				guild: Id(10),
				role: role(
					10,
					p::VIEW_CHANNEL
						| p::USE_APPLICATION_COMMANDS
						| p::SEND_MESSAGES | p::SEND_MESSAGES_IN_THREADS,
				),
			})
			.unwrap();
		assert!(state.can_use_application_command(Id(2), &command));
		command.default_member_permissions = Some(p::KICK_MEMBERS);
		assert!(!state.can_use_application_command(Id(2), &command));
		command.default_member_permissions = Some(p::VIEW_CHANNEL | p::USE_APPLICATION_COMMANDS);
		assert!(state.can_use_application_command(Id(2), &command));
		command.default_member_permissions = Some(0);
		command.application_permissions.user = Some(true);
		assert!(
			!state.can_use_application_command(Id(2), &command),
			"app-wide allow must preserve command defaults"
		);
		command.permissions.roles.insert(Id(10), true);
		assert!(state.can_use_application_command(Id(2), &command));
		command.permissions.roles.insert(Id(11), false);
		assert!(
			!state.can_use_application_command(Id(2), &command),
			"member role overrides everyone"
		);
		command.permissions.roles.insert(Id(12), true);
		assert!(
			state.can_use_application_command(Id(2), &command),
			"allow wins between matched roles"
		);
		command.permissions.user = Some(false);
		assert!(
			!state.can_use_application_command(Id(2), &command),
			"current user overrides roles"
		);
		command.permissions.user = Some(true);
		command.application_permissions.user = Some(false);
		assert!(
			state.can_use_application_command(Id(2), &command),
			"command-specific allow overrides app denial"
		);
		command
			.application_permissions
			.channels
			.insert(Id(9), false);
		assert!(
			!state.can_use_application_command(Id(2), &command),
			"a user allow does not override a channel denial"
		);
		command.application_permissions.channels.insert(Id(2), true);
		assert!(
			state.can_use_application_command(Id(2), &command),
			"specific channel overrides all channels"
		);
		command.permissions.channels.insert(Id(9), false);
		assert!(
			!state.can_use_application_command(Id(2), &command),
			"command channel layer replaces app channel layer"
		);
		command.permissions.channels.insert(Id(2), true);
		assert!(state.can_use_application_command(Id(2), &command));
		let mut thread = state.channels[0].clone();
		thread.id = Id(21);
		thread.kind = 11;
		thread.parent_id = Some(Id(2));
		state.channels.push(thread);
		state.selected = Some(Id(21));
		command.permissions.channels.insert(Id(21), false);
		assert!(
			state.can_use_application_command(Id(21), &command),
			"thread uses parent rules"
		);
		command.permissions.channels.insert(Id(2), false);
		assert!(!state.can_use_application_command(Id(21), &command));
		state.selected = Some(Id(2));
		state.application_commands.scope = Some(Id(10));
		state.application_commands.commands = vec![command.clone()];
		assert!(
			state
				.prepare_application_command(
					command.id,
					&["run".into()],
					&[("count".into(), "2".into())]
				)
				.is_err()
		);
		assert!(
			!state.interactions.busy(),
			"permission denial must not queue an interaction"
		);
		state
			.update_permissions(crate::permissions::Event::Role {
				guild: Id(10),
				role: role(12, p::ADMINISTRATOR),
			})
			.unwrap();
		assert!(
			state.can_use_application_command(Id(2), &command),
			"administrator bypasses command restrictions"
		);
		state
			.update_permissions(crate::permissions::Event::Role {
				guild: Id(10),
				role: role(12, 0),
			})
			.unwrap();
		state
			.update_permissions(crate::permissions::Event::Member {
				guild: Id(10),
				roles: model::Patch::Null,
				timeout_until: model::Patch::Absent,
			})
			.unwrap();
		assert!(
			!state.can_use_application_command(Id(2), &command),
			"unknown member metadata is not a grant"
		);
		state
			.update_permissions(crate::permissions::Event::Owner {
				guild: Id(10),
				owner: model::Patch::Value(Id(3)),
			})
			.unwrap();
		assert!(
			state.can_use_application_command(Id(2), &command),
			"owner bypasses defaults and explicit denials"
		);
		let dm = bot_state();
		assert!(
			dm.can_use_application_command(Id(2), &command),
			"guild restrictions do not restrict a supported bot DM"
		);
	}
	#[test]
	fn catalog_scope_and_schema_guard_explicit_interaction_submission() {
		let mut state = bot_state();
		let Some(Command::ApplicationCommands {
			channel, request, ..
		}) = state.request_application_commands(Id(2), false)
		else {
			panic!()
		};
		let apply = |state: &mut State, generation, request, commands| {
			state.apply(Envelope {
				generation,
				event: Event::ApplicationCommands {
					channel,
					request,
					result: Ok(commands),
				},
			})
		};
		let generation = state.generation;
		apply(&mut state, generation + 1, request, vec![command()]);
		apply(&mut state, generation, request + 1, vec![command()]);
		assert!(state.application_commands.commands.is_empty());
		apply(&mut state, generation, request, vec![command()]);
		assert_eq!(state.application_commands.commands.len(), 1);
		let mut required_text = command();
		required_text.options = vec![schema::CommandOption {
			kind: 3,
			name: "text".into(),
			description: "Required text without an explicit length limit".into(),
			required: true,
			..Default::default()
		}];
		state.application_commands.commands[0] = required_text.clone();
		assert!(
			state
				.prepare_application_command(Id(4), &[], &[("text".into(), String::new())])
				.is_err()
		);
		assert!(!state.interactions.busy());
		assert!(
			!schema::Invocation {
				command: required_text,
				options: vec![schema::Argument {
					kind: 3,
					name: "text".into(),
					value: Some(schema::Value::String(String::new())),
					options: Vec::new(),
				}],
			}
			.valid()
		);
		state.application_commands.commands[0] = command();
		let path = vec!["run".into()];
		for values in [
			vec![],
			vec![("count".into(), "4".into())],
			vec![("count".into(), "1".into())],
			vec![("count".into(), "2".into()), ("count".into(), "2".into())],
			vec![("count".into(), "2".into()), ("file".into(), "123".into())],
		] {
			assert!(
				state
					.prepare_application_command(Id(4), &path, &values)
					.is_err()
			);
			assert!(!state.interactions.busy());
		}
		state.drafts.insert(channel, "Keep this draft".into());
		let Command::Interaction(request) = state
			.prepare_application_command(
				Id(4),
				&path,
				&[("count".into(), "2".into()), ("file".into(), String::new())],
			)
			.unwrap()
		else {
			panic!()
		};
		assert!(request.valid());
		assert!(request.message_id.is_none());
		assert_eq!(state.drafts[&channel], "Keep this draft");
		assert!(state.interactions.busy());
		let interactions::Data::ApplicationCommand { invocation } = request.data else {
			panic!()
		};
		assert_eq!(
			invocation.options[0].options[0].value,
			Some(schema::Value::Integer(2))
		);
		state.open_home();
		assert!(state.application_commands.commands.is_empty());
		apply(&mut state, generation, request.request, vec![command()]);
		assert!(state.application_commands.commands.is_empty());
		state.selected = Some(channel);
		state.interactions.reset();
		state.request_application_commands(channel, true).unwrap();
		state.apply(Envelope {
			generation,
			event: Event::Disconnected,
		});
		assert!(!state.application_commands.loading && state.application_commands.scope.is_none());
	}
	#[test]
	fn guild_index_survives_sibling_channels_and_access_events() {
		let mut state = bot_state();
		state.user = Some(state.channels[0].recipients[0].clone());
		state.channels[0].guild = Some(Id(10));
		state.channels[0].kind = 0;
		let mut sibling = state.channels[0].clone();
		sibling.id = Id(3);
		state.channels.push(sibling);
		state.guilds.push(model::Guild {
			id: Id(10),
			name: "Synthetic guild".into(),
			icon: None,
			emojis: None,
			stickers: None,
		});
		use model::permissions as p;
		state
			.permissions
			.replace(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(10),
					owner: Some(Id(99)),
					roles: Some(vec![p::Role {
						id: Id(10),
						bits: p::VIEW_CHANNEL | p::USE_APPLICATION_COMMANDS | p::SEND_MESSAGES,
						name: "Synthetic role".into(),
						color: 0,
						position: 0,
						hoist: false,
					}]),
					member: Some(p::Member {
						roles: vec![],
						timeout_until: None,
					}),
				}],
				channels: [Id(2), Id(3)]
					.into_iter()
					.map(|id| p::Channel {
						id,
						guild: Id(10),
						overwrites: Some(vec![]),
					})
					.collect(),
			})
			.unwrap();
		let Some(Command::ApplicationCommands { guild, request, .. }) =
			state.request_application_commands(Id(2), false)
		else {
			panic!("guild index request");
		};
		assert_eq!(guild, Some(Id(10)));
		assert!(
			state.request_application_commands(Id(2), false).is_none(),
			"one request per index while loading"
		);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Member {
				guild: Id(10),
				roles: model::Patch::Value(vec![]),
				timeout_until: model::Patch::Absent,
			}),
		});
		assert!(
			state.application_commands.loading,
			"member updates must not restart the index request"
		);
		state.select(Id(3));
		let mut command = command();
		command.contexts = None;
		command.guild_id = Some(Id(10));
		state.apply_application_commands(Id(2), request, Ok(vec![command]));
		assert_eq!(state.application_commands.commands.len(), 1);
		assert!(state.application_commands_cover(Id(3)));
		assert!(
			state.request_application_commands(Id(3), false).is_none(),
			"sibling channels share the guild index"
		);
		assert!(state.request_application_commands(Id(3), true).is_some());
	}
}
