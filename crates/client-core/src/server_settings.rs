//! One permission-gated server editor; writes are never automatically retried.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{
	Id, Patch, permissions,
	server_settings::{Edit, Settings},
};

pub struct Event {
	pub guild: Id,
	pub request: u64,
	pub result: Result<Box<Settings>, Failure>,
	/// Read-only reconciliation after an uncertain or partially completed write.
	pub refreshed: Option<Box<Settings>>,
}
#[derive(Default)]
pub struct Editor {
	pub(crate) notification_default_changed: bool,
	pub guild: Option<Id>,
	pub snapshot: Option<Settings>,
	pub pending: bool,
	pub saving: bool,
	pub error: Option<&'static str>,
	pub revision: u64,
	sequence: u64,
	pub needs_refresh: bool,
}
impl Editor {
	pub(crate) fn reset(&mut self) {
		*self = Self {
			sequence: self.sequence.wrapping_add(1),
			revision: self.revision.wrapping_add(1),
			..Self::default()
		};
	}
}
impl State {
	pub fn can_manage_guild(&self, guild: Id) -> bool {
		let Some(user) = &self.user else {
			return false;
		};
		let Some(permissions) = self.permissions.guilds.get(&guild) else {
			return false;
		};
		self.guild(guild).is_some()
			&& self
				.permissions
				.effective(
					guild,
					permissions,
					user.id,
					Some(&[]),
					Self::permission_time(),
				)
				.is_some_and(|bits| bits & permissions::MANAGE_GUILD != 0)
	}
	pub fn load_server_settings(&mut self, guild: Id) -> Option<Command> {
		if !self.can_manage_guild(guild)
			|| self.server_settings.pending
			|| (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
		{
			return None;
		}
		if self.server_settings.guild != Some(guild) {
			self.server_settings.reset();
		}
		self.server_settings.guild = Some(guild);
		self.server_settings.sequence = self.server_settings.sequence.wrapping_add(1);
		self.server_settings.pending = true;
		self.server_settings.notification_default_changed = false;
		self.server_settings.saving = false;
		self.server_settings.error = None;
		Some(Command::ServerSettings {
			guild,
			request: self.server_settings.sequence,
			edit: None,
		})
	}
	pub fn save_server_settings(&mut self, edit: Edit) -> Option<Command> {
		let guild = self.server_settings.guild?;
		if self.server_settings.pending
			|| self.server_admin.saving
			|| self.server_settings.needs_refresh
			|| edit.is_empty()
			|| !self.valid_server_settings_edit(guild, &edit)
			|| (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
		{
			return None;
		}
		self.server_settings.sequence = self.server_settings.sequence.wrapping_add(1);
		self.server_settings.pending = true;
		self.server_settings.notification_default_changed = false;
		self.server_settings.saving = true;
		self.server_settings.error = None;
		Some(Command::ServerSettings {
			guild,
			request: self.server_settings.sequence,
			edit: Some(Box::new(edit)),
		})
	}
	pub fn server_settings_command_allowed(
		&self,
		guild: Id,
		request: u64,
		edit: &Option<Box<Edit>>,
	) -> bool {
		self.server_settings.guild == Some(guild)
			&& self.server_settings.sequence == request
			&& self.server_settings.pending
			&& self.can_manage_guild(guild)
			&& edit
				.as_ref()
				.is_none_or(|edit| self.valid_server_settings_edit(guild, edit))
	}
	fn valid_server_settings_edit(&self, guild: Id, edit: &Edit) -> bool {
		let Some(snapshot) = self
			.server_settings
			.snapshot
			.as_ref()
			.filter(|settings| settings.guild == guild)
		else {
			return false;
		};
		if !self.can_manage_guild(guild)
			|| !edit.valid()
			|| !edit.keeps_community_requirements(snapshot)
		{
			return false;
		}
		for (patch, voice) in [
			(&edit.system_channel_id, false),
			(&edit.afk_channel_id, true),
		] {
			if let Patch::Value(id) = patch
				&& (!self.can_view(*id)
					|| !self.channel(*id).is_some_and(|channel| {
						channel.guild == Some(guild)
							&& if voice {
								channel.kind == 2
							} else {
								matches!(channel.kind, 0 | 5)
							}
					})) {
				return false;
			}
		}
		let mut value = snapshot.clone();
		edit.apply(&mut value);
		value.valid()
	}
	pub fn close_server_settings(&mut self) {
		// Keep the sole outstanding write admitted until its completion arrives.
		if !self.server_settings.saving {
			self.server_settings.reset();
		}
	}
	pub(crate) fn cancel_server_settings(&mut self) {
		if self.server_settings.pending {
			self.server_settings.pending = false;
			self.server_settings.error = Some(if self.server_settings.saving {
				Failure::Ambiguous.label()
			} else {
				"Server settings disconnected; reload to continue"
			});
			self.server_settings.needs_refresh = self.server_settings.saving;
			self.server_settings.saving = false;
			self.server_settings.sequence = self.server_settings.sequence.wrapping_add(1);
		}
	}
	pub(crate) fn apply_server_settings(&mut self, event: Event) -> Result<(), &'static str> {
		if self.server_settings.guild != Some(event.guild)
			|| self.server_settings.sequence != event.request
			|| !self.server_settings.pending
		{
			return Ok(());
		}
		if !self.can_manage_guild(event.guild) {
			self.server_settings.reset();
			return Ok(());
		}
		let writing = self.server_settings.saving;
		self.server_settings.pending = false;
		self.server_settings.saving = false;
		let (snapshot, error) = match event.result {
			Ok(snapshot) => (Some(snapshot), None),
			Err(failure) => {
				if failure.ends_session() {
					self.fail(failure);
					return Ok(());
				}
				(event.refreshed, Some(failure))
			}
		};
		self.server_settings.error = error.map(Failure::label);
		self.server_settings.needs_refresh = error.is_some()
			&& snapshot.is_none()
			&& (writing || self.server_settings.needs_refresh);
		if let Some(snapshot) = snapshot {
			if snapshot.guild != event.guild || !snapshot.valid() {
				self.server_settings.error =
					Some("Server settings response was invalid; reload to continue");
				self.server_settings.needs_refresh = true;
				return Ok(());
			}
			if let Some(guild) = self.guilds.iter_mut().find(|guild| guild.id == event.guild) {
				guild.name.clone_from(&snapshot.name);
				guild.icon.clone_from(&snapshot.icon);
				if !self.server_settings.notification_default_changed {
					guild.default_message_notifications =
						Some(snapshot.default_message_notifications);
				}
			}
			self.invalidate_navigation();
			self.server_settings.snapshot = Some(*snapshot);
			self.server_settings.revision = self.server_settings.revision.wrapping_add(1);
		}
		Ok(())
	}
}
