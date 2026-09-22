//! One active conversation's received application commands. Never persisted or auto-run.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
	interactions,
};
use model::{Freshness, Id, application_commands as schema};

#[derive(Default)]
pub struct Catalog {
	pub channel: Option<Id>,
	pub request: u64,
	pub loading: bool,
	pub commands: Vec<schema::Command>,
	pub error: Option<&'static str>,
}
impl Catalog {
	pub fn clear(&mut self) {
		self.request = self.request.wrapping_add(1);
		self.channel = None;
		self.loading = false;
		self.commands = Vec::new();
		self.error = None;
	}
}

impl State {
	pub fn can_request_application_commands(&self, channel: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness != Freshness::Unavailable
			&& self.selected == Some(channel)
			&& self.can_view(channel)
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
	pub fn request_application_commands(&mut self, channel: Id, refresh: bool) -> Option<Command> {
		if !self.can_request_application_commands(channel) {
			return None;
		}
		let catalog = &self.application_commands;
		if catalog.channel == Some(channel) && (catalog.loading || !refresh) {
			return None;
		}
		let guild = self.channel(channel)?.guild;
		let catalog = &mut self.application_commands;
		catalog.clear();
		catalog.channel = Some(channel);
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
		if !self.can_request_application_commands(channel)
			|| self.application_commands.channel != Some(channel)
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
		if catalog.channel != Some(channel) || catalog.loading || catalog.error.is_some() {
			return Err("Refresh the application command list first");
		}
		let command = catalog
			.commands
			.iter()
			.find(|c| c.id == command_id)
			.ok_or("This command is no longer available")?;
		let guild = self.channel(channel).and_then(|c| c.guild);
		let context = if guild.is_some() { 0 } else { 1 };
		if command.guild_id.is_some() && command.guild_id != guild
			|| command
				.contexts
				.as_ref()
				.is_some_and(|contexts| !contexts.contains(&context))
		{
			return Err("This command is not available in the current context");
		}
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
	fn state() -> State {
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
	fn catalog_scope_and_schema_guard_explicit_interaction_submission() {
		let mut state = state();
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
		assert!(
			!state.application_commands.loading && state.application_commands.channel.is_none()
		);
	}
}
