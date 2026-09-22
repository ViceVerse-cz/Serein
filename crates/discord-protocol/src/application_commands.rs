//! Unofficial account command indexes; JSON schemas become bounded typed commands.
//! Reference: discord.py-self http.py application_command_index and commands.py.
use crate::{DecodeError, permissions::List};
use model::{
	Id,
	application_commands::{Command, MAX_COMMANDS, valid_catalog},
};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Application {
	id: Id,
	name: String,
}
#[derive(Deserialize)]
struct Index {
	application_commands: List<Box<RawValue>, MAX_COMMANDS>,
	#[serde(default)]
	applications: Option<List<Application, MAX_COMMANDS>>,
}
#[derive(Deserialize)]
struct Header {
	#[serde(rename = "type")]
	kind: u8,
	#[serde(default)]
	guild_id: Option<Id>,
	#[serde(default)]
	contexts: Option<List<u8, 3>>,
	#[serde(default)]
	dm_permission: Option<bool>,
}

pub fn decode(bytes: &[u8], guild: Option<Id>) -> Result<Vec<Command>, DecodeError> {
	let index: Index = crate::decode(bytes)?;
	let mut applications = BTreeMap::new();
	for application in index.applications.unwrap_or_default().0 {
		if application.name.is_empty()
			|| application.name.chars().count() > 100
			|| applications
				.insert(application.id, application.name)
				.is_some()
		{
			return Err(DecodeError);
		}
	}
	let mut commands = Vec::new();
	for raw in index.application_commands.0 {
		let header: Header = serde_json::from_str(raw.get()).map_err(|_| DecodeError)?;
		if header.kind != 1
			|| header.guild_id.is_some_and(|id| Some(id) != guild)
			|| header
				.contexts
				.as_ref()
				.is_some_and(|contexts| !contexts.0.contains(&u8::from(guild.is_none())))
			|| (guild.is_none() && header.dm_permission == Some(false))
		{
			continue;
		}
		let mut command: Command = serde_json::from_str(raw.get()).map_err(|_| DecodeError)?;
		command.application_name = applications
			.get(&command.application_id)
			.cloned()
			.unwrap_or_else(|| command.application_id.to_string());
		commands.push(command);
	}
	if !valid_catalog(&commands) {
		return Err(DecodeError);
	}
	commands.shrink_to_fit();
	Ok(commands)
}
