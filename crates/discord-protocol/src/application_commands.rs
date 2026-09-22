//! Unofficial account command indexes; JSON schemas become bounded typed commands.
//! Reference: discord.py-self http.py application_command_index and commands.py.
use crate::{DecodeError, permissions::List};
use model::{
	Id,
	application_commands::{
		Command, CommandPermissions, MAX_CATALOG_BYTES, MAX_COMMANDS, MAX_PERMISSION_OVERWRITES,
		valid_catalog,
	},
};
use serde::{
	Deserialize, Deserializer,
	de::{MapAccess, Visitor},
};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Bits(#[serde(deserialize_with = "crate::permissions::bits")] u128);

#[derive(Default, Deserialize)]
struct Permissions {
	#[serde(default)]
	user: Option<bool>,
	#[serde(default, deserialize_with = "permission_map")]
	roles: BTreeMap<Id, bool>,
	#[serde(default, deserialize_with = "permission_map")]
	channels: BTreeMap<Id, bool>,
}
impl Permissions {
	fn checked(self) -> Result<CommandPermissions, DecodeError> {
		let permissions = CommandPermissions {
			user: self.user,
			roles: self.roles,
			channels: self.channels,
		};
		permissions
			.valid()
			.then_some(permissions)
			.ok_or(DecodeError)
	}
}
fn permission_map<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<Id, bool>, D::Error> {
	struct PermissionMap;
	impl<'de> Visitor<'de> for PermissionMap {
		type Value = BTreeMap<Id, bool>;
		fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("bounded command permission overrides")
		}
		fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
			let mut permissions = BTreeMap::new();
			while let Some((id, allowed)) = map.next_entry()? {
				if permissions.len() == MAX_PERMISSION_OVERWRITES
					|| permissions.insert(id, allowed).is_some()
				{
					return Err(serde::de::Error::custom(
						"Invalid command permission overrides",
					));
				}
			}
			Ok(permissions)
		}
	}
	d.deserialize_map(PermissionMap)
}

#[derive(Deserialize)]
struct Application {
	id: Id,
	name: String,
	#[serde(default)]
	icon: Option<String>,
	#[serde(default)]
	permissions: Option<Permissions>,
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
	#[serde(default)]
	nsfw: bool,
	#[serde(default)]
	default_member_permissions: Option<Bits>,
	#[serde(default)]
	permissions: Option<Permissions>,
}

pub fn decode(bytes: &[u8], guild: Option<Id>) -> Result<Vec<Command>, DecodeError> {
	let index: Index = crate::decode(bytes)?;
	let mut applications = BTreeMap::new();
	let mut application_bytes = 0;
	for application in index.applications.unwrap_or_default().0 {
		let permissions = application.permissions.unwrap_or_default().checked()?;
		application_bytes += application.name.capacity()
			+ application.icon.as_ref().map_or(0, String::capacity)
			+ permissions.bytes()
			+ 512;
		if application.name.is_empty()
			|| application.name.chars().count() > 100
			|| application_bytes > MAX_CATALOG_BYTES
			|| applications
				.insert(
					application.id,
					(
						application.name,
						application
							.icon
							.filter(|hash| model::valid_avatar_hash(hash)),
						permissions,
					),
				)
				.is_some()
		{
			return Err(DecodeError);
		}
	}
	let mut commands = Vec::new();
	let mut command_bytes = 0;
	for raw in index.application_commands.0 {
		let header: Header = serde_json::from_str(raw.get()).map_err(|_| DecodeError)?;
		// Account/channel age eligibility is not represented by this client yet.
		if header.kind != 1
			|| header.nsfw
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
		command.default_member_permissions = header.default_member_permissions.map(|bits| bits.0);
		command.permissions = header.permissions.unwrap_or_default().checked()?;
		if let Some((name, icon, permissions)) = applications.get(&command.application_id) {
			command.application_name = name.clone();
			command.application_icon = icon.clone();
			command.application_permissions = permissions.clone();
		} else {
			command.application_name = command.application_id.to_string();
		}
		command_bytes += command.bytes();
		if command_bytes > MAX_CATALOG_BYTES {
			return Err(DecodeError);
		}
		commands.push(command);
	}
	if !valid_catalog(&commands) {
		return Err(DecodeError);
	}
	commands.shrink_to_fit();
	Ok(commands)
}
