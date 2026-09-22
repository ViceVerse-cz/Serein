//! Bounded received slash-command schemas and values validated against those schemas.
use crate::Id;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_COMMANDS: usize = 2000;
pub const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024 - 1024;
pub const MAX_SUBMISSION_BYTES: usize = 256 * 1024;
pub const MAX_PERMISSION_OVERWRITES: usize = 100;
const MAX_COMMAND_BYTES: usize = 128 * 1024;
const MAX_OPTION_NODES: usize = 1024;
const SAFE_INTEGER: i64 = 9_007_199_254_740_991;

fn chat_input() -> u8 {
	1
}
fn is_false(value: &bool) -> bool {
	!value
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
	pub id: Id,
	pub version: Id,
	pub application_id: Id,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub guild_id: Option<Id>,
	#[serde(rename = "type", default = "chat_input")]
	pub kind: u8,
	pub name: String,
	pub description: String,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub options: Vec<CommandOption>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub contexts: Option<Vec<u8>>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub integration_types: Option<Vec<u8>>,
	#[serde(skip)]
	pub application_name: String,
	#[serde(skip)]
	pub application_icon: Option<String>,
	#[serde(skip)]
	pub default_member_permissions: Option<u128>,
	#[serde(skip)]
	pub permissions: CommandPermissions,
	#[serde(skip)]
	pub application_permissions: CommandPermissions,
}

/// Account-index overrides; `user` applies only to the current account.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandPermissions {
	pub user: Option<bool>,
	pub roles: BTreeMap<Id, bool>,
	pub channels: BTreeMap<Id, bool>,
}
impl CommandPermissions {
	pub fn valid(&self) -> bool {
		self.roles.len() + self.channels.len() + usize::from(self.user.is_some())
			<= MAX_PERMISSION_OVERWRITES
			&& self
				.roles
				.keys()
				.chain(self.channels.keys())
				.all(|id| id.0 != 0)
	}
	pub fn bytes(&self) -> usize {
		// Include conservative B-tree node allocation allowances, not just entries.
		size_of::<Self>()
			+ [&self.roles, &self.channels]
				.into_iter()
				.map(|map| map.len() * 64 + usize::from(!map.is_empty()) * 512)
				.sum::<usize>()
	}
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandOption {
	#[serde(rename = "type")]
	pub kind: u8,
	pub name: String,
	pub description: String,
	#[serde(default, skip_serializing_if = "is_false")]
	pub required: bool,
	#[serde(default, skip_serializing_if = "is_false")]
	pub autocomplete: bool,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub options: Vec<CommandOption>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub choices: Vec<Choice>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub channel_types: Vec<u8>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub min_value: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_value: Option<f64>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub min_length: Option<u16>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub max_length: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
	pub name: String,
	pub value: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
	String(String),
	Integer(i64),
	Number(f64),
	Boolean(bool),
}
impl Value {
	fn number(&self) -> Option<f64> {
		match self {
			Self::Integer(n) => Some(*n as f64),
			Self::Number(n) => Some(*n),
			_ => None,
		}
	}
	fn heap_bytes(&self) -> usize {
		match self {
			Self::String(s) => s.capacity(),
			_ => 0,
		}
	}
	fn wire_bytes(&self) -> usize {
		match self {
			Self::String(s) => text_bytes(s),
			_ => 32,
		}
	}
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Argument {
	#[serde(rename = "type")]
	pub kind: u8,
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub value: Option<Value>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub options: Vec<Argument>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
	pub command: Command,
	pub options: Vec<Argument>,
}

fn name(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 128
		&& value.chars().count() <= 32
		&& value.chars().all(|c| !c.is_control() && !c.is_whitespace())
}
fn description(value: &str) -> bool {
	!value.is_empty()
		&& value.len() <= 400
		&& value.chars().count() <= 100
		&& !value.chars().any(char::is_control)
}
fn metadata(values: &Option<Vec<u8>>, max: u8) -> bool {
	values.as_ref().is_none_or(|values| {
		values.len() <= max as usize + 1
			&& values
				.iter()
				.enumerate()
				.all(|(i, value)| *value <= max && !values[..i].contains(value))
	})
}
// JSON strings escape each byte by at most six bytes. Conservative estimates avoid
// adding a JSON dependency to the model; the transport checks the encoded body too.
fn text_bytes(value: &str) -> usize {
	value.len().saturating_mul(6).saturating_add(2)
}

impl CommandOption {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.name.capacity()
			+ self.description.capacity()
			+ self.options.capacity() * size_of::<Self>()
			+ self
				.options
				.iter()
				.map(|o| o.bytes() - size_of::<Self>())
				.sum::<usize>()
			+ self.choices.capacity() * size_of::<Choice>()
			+ self
				.choices
				.iter()
				.map(|c| c.name.capacity() + c.value.heap_bytes())
				.sum::<usize>()
			+ self.channel_types.capacity()
	}
	fn wire_bytes(&self) -> usize {
		384 + text_bytes(&self.name)
			+ text_bytes(&self.description)
			+ self.options.iter().map(Self::wire_bytes).sum::<usize>()
			+ self
				.choices
				.iter()
				.map(|c| 64 + text_bytes(&c.name) + c.value.wire_bytes())
				.sum::<usize>()
	}
	fn value_valid(&self, value: &Value) -> bool {
		let typed = match (self.kind, value) {
			(3, Value::String(s)) => {
				s.len() <= 24_000
					&& (!self.required || !s.is_empty())
					&& s.chars().count() >= usize::from(self.min_length.unwrap_or(0))
					&& s.chars().count() <= usize::from(self.max_length.unwrap_or(6000))
			}
			(4, Value::Integer(n)) => (-SAFE_INTEGER..=SAFE_INTEGER).contains(n),
			(5, Value::Boolean(_)) => true,
			(6..=9, Value::String(s)) => s.parse::<Id>().is_ok(),
			(10, Value::Integer(_) | Value::Number(_)) => value
				.number()
				.is_some_and(|n| n.is_finite() && n.abs() <= SAFE_INTEGER as f64),
			_ => false,
		};
		typed
			&& value.number().is_none_or(|n| {
				self.min_value.is_none_or(|min| n >= min)
					&& self.max_value.is_none_or(|max| n <= max)
			}) && (self.choices.is_empty()
			|| self.choices.iter().any(|choice| {
				choice.value == *value
					|| choice
						.value
						.number()
						.zip(value.number())
						.is_some_and(|(a, b)| a == b)
			}))
	}
	/// Why a typed composer value cannot be submitted, or `None` for an acceptable value.
	/// An empty value is acceptable here; required-but-missing is a submission concern.
	pub fn problem(&self, text: &str) -> Option<String> {
		if text.is_empty() {
			return None;
		}
		fn bound(n: f64) -> String {
			if n.fract() == 0.0 && n.abs() < 1e15 {
				format!("{}", n as i64)
			} else {
				format!("{n}")
			}
		}
		let value = match self.kind {
			3 | 6..=9 => Value::String(text.to_owned()),
			4 => match text.parse::<i64>() {
				Ok(n) => Value::Integer(n),
				Err(_) => return Some("Enter a whole number".into()),
			},
			5 => match text.parse::<bool>() {
				Ok(b) => Value::Boolean(b),
				Err(_) => return Some("Choose true or false".into()),
			},
			10 => match text.parse::<f64>() {
				Ok(n) if n.is_finite() => Value::Number(n),
				_ => return Some("Enter a number".into()),
			},
			11 => return Some("Attachment options are not supported yet".into()),
			_ => return Some("This option is unsupported".into()),
		};
		if matches!(self.kind, 6..=9) && text.parse::<Id>().is_err() {
			return Some("Choose from the list or enter an ID".into());
		}
		if self.kind == 3 {
			let count = text.chars().count();
			let min = usize::from(self.min_length.unwrap_or(0));
			let max = usize::from(self.max_length.unwrap_or(6000));
			if count < min || count > max {
				return Some(match (self.min_length, self.max_length) {
					(Some(min), Some(max)) => format!("Use between {min} and {max} characters"),
					(Some(min), None) => format!("Use at least {min} characters"),
					_ => format!("Use at most {max} characters"),
				});
			}
		}
		if let Some(n) = value.number()
			&& (self.min_value.is_some_and(|min| n < min)
				|| self.max_value.is_some_and(|max| n > max))
		{
			return Some(match (self.min_value, self.max_value) {
				(Some(min), Some(max)) => {
					format!("Enter a value between {} and {}", bound(min), bound(max))
				}
				(Some(min), None) => format!("Enter at least {}", bound(min)),
				(None, Some(max)) => format!("Enter at most {}", bound(max)),
				(None, None) => unreachable!(),
			});
		}
		if !self.value_valid(&value) {
			return Some(if self.choices.is_empty() {
				"Value does not match this option's limits".into()
			} else {
				"Choose one of the listed options".into()
			});
		}
		None
	}
	fn parse(&self, text: &str) -> Result<Value, &'static str> {
		if text.len() > 24_000 {
			return Err("Command value exceeds its byte limit");
		}
		let value = match self.kind {
			3 | 6..=9 => Value::String(text.to_owned()),
			4 => Value::Integer(text.parse().map_err(|_| "Enter a whole number")?),
			5 => Value::Boolean(text.parse().map_err(|_| "Choose true or false")?),
			10 => Value::Number(text.parse().map_err(|_| "Enter a finite number")?),
			11 => return Err("Attachment command options are not supported yet"),
			_ => return Err("This command option is unsupported"),
		};
		if !self.value_valid(&value) {
			return Err("Command value does not match its choices or limits");
		}
		Ok(value)
	}
}

fn valid_options(options: &[CommandOption], depth: usize, nodes: &mut usize) -> bool {
	if depth > 2 || options.len() > 25 {
		return false;
	}
	let mut names = BTreeSet::new();
	let branches = options.iter().any(|o| o.kind <= 2);
	for option in options {
		*nodes += 1;
		if *nodes > MAX_OPTION_NODES
			|| !name(&option.name)
			|| !description(&option.description)
			|| !names.insert(&option.name)
			|| !(1..=11).contains(&option.kind)
			|| branches != (option.kind <= 2)
			|| option.choices.len() > 25
			|| option.channel_types.len() > 32
			|| !option.min_value.is_none_or(f64::is_finite)
			|| !option.max_value.is_none_or(f64::is_finite)
			|| option
				.min_value
				.zip(option.max_value)
				.is_some_and(|(a, b)| a > b)
			|| option.min_length.is_some_and(|v| v > 6000)
			|| option.max_length.is_some_and(|v| v > 6000)
			|| option
				.min_length
				.zip(option.max_length)
				.is_some_and(|(a, b)| a > b)
		{
			return false;
		}
		if option.kind <= 2 {
			if option.required
				|| option.autocomplete
				|| !option.choices.is_empty()
				|| !option.channel_types.is_empty()
				|| option.min_value.is_some()
				|| option.max_value.is_some()
				|| option.min_length.is_some()
				|| option.max_length.is_some()
				|| option.kind == 2
					&& (depth != 0
						|| option.options.is_empty()
						|| option.options.iter().any(|o| o.kind != 1))
				|| option.kind == 1 && option.options.iter().any(|o| o.kind <= 2)
				|| !valid_options(&option.options, depth + 1, nodes)
			{
				return false;
			}
		} else {
			if !option.options.is_empty()
				|| option.autocomplete
					&& (!matches!(option.kind, 3 | 4 | 10) || !option.choices.is_empty())
				|| !option.choices.is_empty() && !matches!(option.kind, 3 | 4 | 10)
				|| !option.channel_types.is_empty() && option.kind != 7
				|| (option.min_value.is_some() || option.max_value.is_some())
					&& !matches!(option.kind, 4 | 10)
				|| (option.min_length.is_some() || option.max_length.is_some()) && option.kind != 3
			{
				return false;
			}
			for choice in &option.choices {
				if !description(&choice.name) || !option.value_valid(&choice.value) {
					return false;
				}
			}
		}
	}
	true
}

impl Command {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.name.capacity()
			+ self.description.capacity()
			+ self.application_name.capacity()
			+ self.application_icon.as_ref().map_or(0, String::capacity)
			+ self.permissions.bytes()
			- size_of::<CommandPermissions>()
			+ self.application_permissions.bytes()
			- size_of::<CommandPermissions>()
			+ self.options.capacity() * size_of::<CommandOption>()
			+ self
				.options
				.iter()
				.map(|o| o.bytes() - size_of::<CommandOption>())
				.sum::<usize>()
			+ self.contexts.as_ref().map_or(0, Vec::capacity)
			+ self.integration_types.as_ref().map_or(0, Vec::capacity)
	}
	fn wire_bytes(&self) -> usize {
		512 + text_bytes(&self.name)
			+ text_bytes(&self.description)
			+ self
				.options
				.iter()
				.map(CommandOption::wire_bytes)
				.sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& self.version.0 != 0
			&& self.application_id.0 != 0
			&& self.guild_id.is_none_or(|id| id.0 != 0)
			&& self.kind == 1
			&& name(&self.name)
			&& description(&self.description)
			&& self.application_name.len() <= 400
			&& !self.application_name.chars().any(char::is_control)
			&& self
				.application_icon
				.as_deref()
				.is_none_or(crate::valid_avatar_hash)
			&& metadata(&self.contexts, 2)
			&& metadata(&self.integration_types, 1)
			&& self.permissions.valid()
			&& self.application_permissions.valid()
			&& valid_options(&self.options, 0, &mut 0)
			&& self.bytes() <= MAX_COMMAND_BYTES
			&& self.wire_bytes() <= MAX_COMMAND_BYTES
	}
	pub fn options_at(&self, path: &[String]) -> Result<&[CommandOption], &'static str> {
		if path.len() > 2 {
			return Err("Invalid subcommand path");
		}
		let mut options = self.options.as_slice();
		for part in path {
			let option = options
				.iter()
				.find(|o| o.name == *part && o.kind <= 2)
				.ok_or("This subcommand is no longer available")?;
			options = &option.options;
		}
		if options.iter().any(|o| o.kind <= 2) {
			return Err("Choose a subcommand");
		}
		Ok(options)
	}
	pub fn invocation(
		&self,
		path: &[String],
		values: &[(String, String)],
	) -> Result<Invocation, &'static str> {
		if !self.valid() || values.len() > 25 {
			return Err("Invalid command schema or too many values");
		}
		let schema = self.options_at(path)?;
		let mut seen = BTreeSet::new();
		for (name, _) in values {
			if !seen.insert(name) || !schema.iter().any(|o| o.name == *name) {
				return Err("Unknown or repeated command option");
			}
		}
		let mut options = Vec::new();
		for option in schema {
			let value = values
				.iter()
				.find(|(name, value)| *name == option.name && !value.is_empty());
			if option.kind == 11 && (option.required || value.is_some()) {
				return Err("Attachment command options are not supported yet");
			}
			let Some((_, value)) = value else {
				if option.required {
					return Err("Complete every required command option");
				}
				continue;
			};
			options.push(Argument {
				kind: option.kind,
				name: option.name.clone(),
				value: Some(option.parse(value)?),
				options: Vec::new(),
			});
		}
		for depth in (0..path.len()).rev() {
			let mut branch = &self.options;
			for part in &path[..depth] {
				branch = &branch.iter().find(|o| o.name == *part).unwrap().options;
			}
			let selected = branch.iter().find(|o| o.name == path[depth]).unwrap();
			options = vec![Argument {
				kind: selected.kind,
				name: selected.name.clone(),
				value: None,
				options,
			}];
		}
		let invocation = Invocation {
			command: self.clone(),
			options,
		};
		if !invocation.valid() {
			return Err("Command submission exceeds its limits");
		}
		Ok(invocation)
	}
}

fn arguments_valid(schema: &[CommandOption], values: &[Argument]) -> bool {
	if values.len() > 25 || schema.iter().any(|o| o.kind <= 2) && values.len() != 1 {
		return false;
	}
	let mut names = BTreeSet::new();
	for value in values {
		let Some(option) = schema
			.iter()
			.find(|o| o.name == value.name && o.kind == value.kind)
		else {
			return false;
		};
		if !names.insert(&value.name) {
			return false;
		}
		if option.kind <= 2 {
			if value.value.is_some() || !arguments_valid(&option.options, &value.options) {
				return false;
			}
		} else if !value.options.is_empty()
			|| !value.value.as_ref().is_some_and(|v| option.value_valid(v))
		{
			return false;
		}
	}
	schema
		.iter()
		.all(|o| !o.required || values.iter().any(|v| v.name == o.name))
}
impl Argument {
	fn wire_bytes(&self) -> usize {
		64 + text_bytes(&self.name)
			+ self.value.as_ref().map_or(0, Value::wire_bytes)
			+ self.options.iter().map(Self::wire_bytes).sum::<usize>()
	}
}
impl Invocation {
	pub fn valid(&self) -> bool {
		self.command.valid()
			&& arguments_valid(&self.command.options, &self.options)
			&& self.command.wire_bytes()
				+ self.options.iter().map(Argument::wire_bytes).sum::<usize>()
				+ 1024 <= MAX_SUBMISSION_BYTES
	}
}

pub fn catalog_bytes(commands: &[Command]) -> usize {
	commands.iter().map(Command::bytes).sum()
}
pub fn valid_catalog(commands: &[Command]) -> bool {
	let mut ids = BTreeSet::new();
	commands.len() <= MAX_COMMANDS
		&& commands
			.iter()
			.all(|command| command.valid() && ids.insert(command.id))
		&& catalog_bytes(commands) <= MAX_CATALOG_BYTES
}
