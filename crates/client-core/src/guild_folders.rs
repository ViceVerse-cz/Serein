//! Account server organization, acknowledged by Discord before replacing the visible layout.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::guild_folders::Settings;

impl State {
	pub fn load_guild_folders(&mut self) -> Option<Command> {
		if self.folders_pending
			|| (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
		{
			return None;
		}
		self.folders_error = None;
		self.folders_stale = false;
		if self.demo {
			self.guild_folders.get_or_insert_with(Settings::default);
			return None;
		}
		self.folders_pending = true;
		Some(Command::GuildFolders(None))
	}

	pub fn save_guild_folders(&mut self, mut settings: Settings) -> Option<Command> {
		if self.folders_pending
			|| self.guild_folders.is_none()
			|| (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
		{
			return None;
		}
		settings.folders.shrink_to_fit();
		for folder in &mut settings.folders {
			folder.guild_ids.shrink_to_fit();
		}
		if !settings.valid() {
			self.folders_error = Some("Server folder layout exceeds supported limits");
			return None;
		}
		self.folders_error = None;
		let base = self.guild_folders.clone()?;
		if base == settings {
			return None;
		}
		if self.demo {
			self.guild_folders = Some(settings);
			self.revision = self.revision.wrapping_add(1);
			return None;
		}
		self.folders_pending = true;
		Some(Command::GuildFolders(Some((base, settings))))
	}

	pub fn apply_guild_folders(&mut self, result: Result<Settings, Failure>) {
		if !self.folders_pending {
			return;
		}
		self.folders_pending = false;
		match result {
			Ok(settings) if settings.valid() => {
				self.guild_folders = Some(settings);
				self.folders_error = None;
			}
			Ok(_) => self.folders_error = Some("Server folder response exceeds supported limits"),
			Err(failure) => {
				self.folders_error = Some(failure.label());
				if failure.ends_session() && failure != Failure::Capacity {
					self.fail(failure);
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};

	#[test]
	fn folder_writes_wait_for_ack_and_rejection_preserves_the_layout() {
		let original = Settings::default();
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			guild_folders: Some(original.clone()),
			..State::default()
		};
		let changed = Settings {
			version: 1,
			..original.clone()
		};
		let command = state.save_guild_folders(changed.clone()).unwrap();
		assert_eq!(state.guild_folders.as_ref(), Some(&original));
		assert!(state.save_guild_folders(changed.clone()).is_none());
		state.command_rejected(command);
		assert!(!state.folders_pending);
		assert!(state.folders_error.is_some());
		assert_eq!(state.guild_folders.as_ref(), Some(&original));
		assert!(state.save_guild_folders(changed.clone()).is_some());
		let generation = state.generation;
		state.apply(Envelope {
			generation,
			event: Event::GuildFolders(Ok(changed.clone())),
		});
		assert_eq!(state.guild_folders.as_ref(), Some(&changed));
		state.logout();
		state.apply(Envelope {
			generation,
			event: Event::GuildFolders(Ok(changed)),
		});
		assert!(state.guild_folders.is_none());
	}
}
