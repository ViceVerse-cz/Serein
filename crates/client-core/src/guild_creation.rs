//! One explicit, bounded guild creation write. Gateway state remains authoritative.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Id, Patch};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
	name: String,
	icon: Option<String>,
}

impl Request {
	pub fn new(name: String, icon: Option<String>) -> Option<Self> {
		let request = Self {
			name: name.trim().to_owned(),
			icon,
		};
		request.valid().then_some(request)
	}
	pub fn valid(&self) -> bool {
		model::server_settings::Edit {
			name: Some(self.name.clone()),
			icon: self.icon.clone().map_or(Patch::Null, Patch::Value),
			..Default::default()
		}
		.valid()
	}
	pub fn name(&self) -> &str {
		&self.name
	}
	pub fn icon(&self) -> Option<&str> {
		self.icon.as_deref()
	}
}

#[derive(Default)]
pub struct Creation {
	pub pending: bool,
	pub result: Option<Result<Id, Failure>>,
	sequence: u64,
}

impl State {
	pub fn create_guild(&mut self, name: String, icon: Option<String>) -> Option<Command> {
		if self.demo
			|| self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.guild_creation.pending
		{
			return None;
		}
		let request = Request::new(name, icon)?;
		self.guild_creation.sequence = self.guild_creation.sequence.wrapping_add(1);
		self.guild_creation.pending = true;
		self.guild_creation.result = None;
		Some(Command::CreateGuild {
			request,
			sequence: self.guild_creation.sequence,
		})
	}

	pub(crate) fn cancel_guild_creation(&mut self) {
		if self.guild_creation.pending {
			self.guild_creation.pending = false;
			self.guild_creation.result = Some(Err(Failure::Ambiguous));
		}
	}

	pub(crate) fn apply_guild_created(&mut self, sequence: u64, result: Result<Id, Failure>) {
		if !self.guild_creation.pending || sequence != self.guild_creation.sequence {
			return;
		}
		self.guild_creation.pending = false;
		self.guild_creation.result = Some(result);
		self.status = match result {
			Ok(_) => "Server created · waiting for Discord to load it",
			Err(failure) => failure.label(),
		};
		if let Err(failure) = result
			&& failure.ends_session()
		{
			self.fail(failure);
		}
	}
}
