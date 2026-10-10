//! Session-only, bounded soundboard catalog for the joined server voice channel.
//! Sound audio never enters this state; the desktop fetches and plays it.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
	voice::Phase,
};
use model::{Id, permissions as p, soundboard::Sound};
/// Largest accepted catalog response.
pub const MAX_WIRE_BYTES: usize = 256 * 1024;
pub enum Request {
	Default,
	Guild(Id),
	Send {
		channel: Id,
		sound: Id,
		/// Required by the service only for a sound owned by another server.
		source_guild: Option<Id>,
	},
}
pub enum Event {
	Default(Result<Vec<Sound>, Failure>),
	Guild {
		guild: Id,
		result: Result<Vec<Sound>, Failure>,
	},
	Sent {
		channel: Id,
		sound: Id,
		result: Result<(), Failure>,
	},
	/// Another participant played a sound in a voice channel. Not retained.
	Effect {
		channel: Id,
		user: Id,
		sound: Id,
		volume: f32,
	},
	/// A server's sounds changed; its catalog is reloaded when next shown.
	Changed(Id),
}
impl Event {
	pub fn bytes(&self) -> usize {
		match self {
			Self::Default(Ok(sounds))
			| Self::Guild {
				result: Ok(sounds), ..
			} => model::soundboard::sound_bytes(sounds),
			_ => 0,
		}
	}
}
#[derive(Default)]
pub struct Catalog {
	pub sounds: Vec<Sound>,
	pub loading: bool,
	pub loaded: bool,
	/// The sounds changed while a read was outstanding; its response is shown but reloaded.
	pub stale: bool,
	pub error: Option<&'static str>,
}
impl Catalog {
	/// A failed load waits for an explicit retry instead of repeating every frame.
	fn begin(&mut self) -> bool {
		if self.loading || self.loaded || self.error.is_some() {
			return false;
		}
		self.loading = true;
		self.stale = false;
		self.error = None;
		true
	}
	fn finish(&mut self, result: Result<Vec<Sound>, Failure>, guild: Option<Id>) {
		if !self.loading {
			return;
		}
		self.loading = false;
		match result {
			Ok(sounds)
				if model::soundboard::valid_sounds(&sounds)
					&& sounds.iter().all(|sound| sound.guild == guild) =>
			{
				self.sounds = sounds;
				self.loaded = !std::mem::take(&mut self.stale);
				self.error = None;
			}
			Ok(_) => self.error = Some(Failure::Capacity.label()),
			Err(error) => self.error = Some(error.label()),
		}
	}
	fn interrupt(&mut self) {
		if self.loading {
			self.loading = false;
			self.error = Some("Soundboard loading interrupted; try again");
		}
	}
}
#[derive(Default)]
pub struct Soundboard {
	pub default: Catalog,
	/// Sounds of the one server whose voice channel was last shown.
	pub guild: Option<(Id, Catalog)>,
	/// One unanswered play request; another waits for its outcome.
	pub sending: Option<(Id, Id)>,
	pub error: Option<&'static str>,
}
impl Soundboard {
	pub fn sound(&self, id: Id) -> Option<&Sound> {
		self.default
			.sounds
			.iter()
			.chain(self.guild.iter().flat_map(|(_, catalog)| &catalog.sounds))
			.find(|sound| sound.id == id)
	}
}
impl State {
	pub(crate) fn interrupt_soundboard(&mut self) {
		self.soundboard.default.interrupt();
		if let Some((_, catalog)) = &mut self.soundboard.guild {
			catalog.interrupt();
		}
		if self.soundboard.sending.take().is_some() {
			self.soundboard.error = Some("Soundboard request interrupted; try again");
		}
	}
	/// The connected server voice channel and its guild; private calls have no soundboard.
	pub fn soundboard_channel(&self) -> Option<(Id, Id)> {
		let call = self.voice.active.as_ref()?;
		matches!(call.phase, Phase::Connected | Phase::Waiting)
			.then_some(call.guild)
			.flatten()
			.map(|guild| (call.channel, guild))
	}
	/// Why playing is unavailable, as a fixed label; `None` when a sound may be sent.
	pub fn soundboard_unavailable(&self) -> Option<&'static str> {
		let (Some((channel, _)), Some(call)) = (self.soundboard_channel(), &self.voice.active)
		else {
			return Some("Join a server voice channel to use the soundboard");
		};
		// The offline preview has no session; its play requests are answered locally.
		if !self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected) {
			Some("Soundboard is unavailable while offline")
		} else if self.permission(channel, p::SPEAK | p::USE_SOUNDBOARD) != Some(true) {
			Some("You do not have permission to use the soundboard in this channel")
		} else if call.server_muted || call.server_deafened {
			Some("Soundboard is unavailable while server muted or deafened")
		} else if call.deafened {
			Some("Undeafen to use the soundboard")
		} else {
			None
		}
	}
	/// Load the catalogs for the connected channel once; at most two bounded reads.
	pub fn request_soundboard(&mut self) -> [Option<Command>; 2] {
		let Some((_, guild)) = self.soundboard_channel() else {
			return [None, None];
		};
		if self.demo || self.auth != AuthState::Authenticated || !self.gateway_connected {
			return [None, None];
		}
		if self.soundboard.guild.as_ref().map(|(id, _)| *id) != Some(guild) {
			self.soundboard.guild = Some((guild, Catalog::default()));
		}
		[
			self.soundboard
				.default
				.begin()
				.then_some(Command::Soundboard(Request::Default)),
			self.soundboard
				.guild
				.as_mut()
				.is_some_and(|(_, catalog)| catalog.begin())
				.then_some(Command::Soundboard(Request::Guild(guild))),
		]
	}
	/// Forget a failed load so the next `request_soundboard` tries again.
	pub fn retry_soundboard(&mut self) {
		self.soundboard.default.error = None;
		if let Some((_, catalog)) = &mut self.soundboard.guild {
			catalog.error = None;
		}
		self.soundboard.error = None;
	}
	pub fn can_play_sound(&self, sound: &Sound) -> bool {
		let Some((channel, guild)) = self.soundboard_channel() else {
			return false;
		};
		sound.valid()
			&& sound.available
			&& self.soundboard.sending.is_none()
			&& self.soundboard_unavailable().is_none()
			&& (sound.guild.is_none()
				|| sound.guild == Some(guild)
				|| self.permission(channel, p::USE_EXTERNAL_SOUNDS) == Some(true))
	}
	pub fn play_sound(&mut self, sound: &Sound) -> Option<Command> {
		if !self.can_play_sound(sound) {
			return None;
		}
		let (channel, guild) = self.soundboard_channel()?;
		self.soundboard.sending = Some((channel, sound.id));
		self.soundboard.error = None;
		Some(Command::Soundboard(Request::Send {
			channel,
			sound: sound.id,
			source_guild: sound.guild.filter(|owner| *owner != guild),
		}))
	}
	pub(crate) fn soundboard_rejected(&mut self, request: &Request) {
		const MESSAGE: &str = "Soundboard request was not queued; try again";
		match request {
			Request::Default => {
				self.soundboard.default.loading = false;
				self.soundboard.default.error = Some(MESSAGE);
			}
			Request::Guild(guild) => {
				if let Some((id, catalog)) = &mut self.soundboard.guild
					&& id == guild
				{
					catalog.loading = false;
					catalog.error = Some(MESSAGE);
				}
			}
			Request::Send { channel, sound, .. } => {
				if self.soundboard.sending == Some((*channel, *sound)) {
					self.soundboard.sending = None;
					self.soundboard.error = Some(MESSAGE);
				}
			}
		}
	}
	pub(crate) fn apply_soundboard(&mut self, event: Event) {
		match event {
			Event::Default(result) => self.soundboard.default.finish(result, None),
			Event::Guild { guild, result } => {
				if let Some((id, catalog)) = &mut self.soundboard.guild
					&& *id == guild
				{
					catalog.finish(result, Some(guild));
				}
			}
			Event::Sent {
				channel,
				sound,
				result,
			} => {
				if self.soundboard.sending == Some((channel, sound)) {
					self.soundboard.sending = None;
					self.soundboard.error = result.err().map(|error| match error {
						Failure::Forbidden => "Discord did not allow this sound in this channel",
						other => other.label(),
					});
				}
			}
			Event::Changed(guild) => {
				if let Some((id, catalog)) = &mut self.soundboard.guild
					&& *id == guild
				{
					// A read already in flight may predate the change; it is repeated.
					catalog.stale = catalog.loading;
					catalog.loaded = false;
				}
				// The open management page reloads too, once its current request settles.
				if self.server_admin.guild == Some(guild) && self.server_admin.sounds.is_some() {
					self.server_admin.sounds_stale = true;
				}
			}
			Event::Effect { .. } => {}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::voice::Call;
	fn sound(id: u64, guild: Option<u64>) -> Sound {
		Sound {
			id: Id(id),
			name: "Quack".into(),
			volume: 1.0,
			emoji_id: None,
			emoji_name: None,
			guild: guild.map(Id),
			available: true,
		}
	}
	fn role(bits: u128) -> p::Snapshot {
		p::Snapshot {
			guilds: vec![p::Guild {
				id: Id(9),
				owner: Some(Id(999)),
				roles: Some(vec![p::Role {
					name: String::new(),
					color: 0,
					secondary_color: None,
					tertiary_color: None,
					position: 0,
					hoist: false,
					id: Id(9),
					bits,
				}]),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
			}],
			channels: vec![p::Channel {
				id: Id(2),
				guild: Id(9),
				overwrites: Some(vec![]),
			}],
		}
	}
	const ALLOWED: u128 = p::VIEW_CHANNEL | p::CONNECT | p::SPEAK | p::USE_SOUNDBOARD;
	fn connected(bits: u128) -> State {
		let mut state = State {
			user: Some(model::User {
				primary_guild: None,
				id: Id(1),
				name: "Owner".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				default_message_notifications: None,
				stickers: None,
				id: Id(9),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			channels: vec![model::Channel {
				id: Id(2),
				guild: Some(Id(9)),
				parent_id: None,
				kind: 2,
				name: "Voice".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				tags: None,
				message_count: None,
			}],
			auth: AuthState::Authenticated,
			freshness: crate::Freshness::Fresh,
			gateway_connected: true,
			..State::default()
		};
		state.permissions.replace(role(bits)).unwrap();
		state.voice.active = Some(Call {
			channel: Id(2),
			guild: Some(Id(9)),
			connected_at: None,
			channel_started_at: None,
			server_muted: false,
			server_deafened: false,
			request: 1,
			phase: Phase::Connected,
			muted: false,
			deafened: false,
			participants: vec![],
			camera: false,
			watching: None,
			error: None,
		});
		state
	}
	#[test]
	fn catalogs_load_once_per_guild_and_reload_after_a_change_or_failure() {
		let mut state = connected(ALLOWED);
		let [default, guild] = state.request_soundboard();
		assert!(matches!(
			default,
			Some(Command::Soundboard(Request::Default))
		));
		assert!(matches!(
			guild,
			Some(Command::Soundboard(Request::Guild(Id(9))))
		));
		assert!(state.request_soundboard().iter().all(Option::is_none));

		state.apply_soundboard(Event::Default(Err(Failure::Network)));
		assert!(state.soundboard.default.error.is_some());
		assert!(state.request_soundboard().iter().all(Option::is_none));
		state.retry_soundboard();
		assert!(state.request_soundboard()[0].is_some());
		state.apply_soundboard(Event::Default(Ok(vec![sound(1, None)])));
		// A catalog for another server, or naming a foreign owner, is never retained.
		state.apply_soundboard(Event::Guild {
			guild: Id(8),
			result: Ok(vec![sound(3, Some(8))]),
		});
		assert!(state.soundboard.guild.as_ref().unwrap().1.loading);
		state.apply_soundboard(Event::Guild {
			guild: Id(9),
			result: Ok(vec![sound(3, Some(8))]),
		});
		assert!(state.soundboard.guild.as_ref().unwrap().1.error.is_some());
		state.retry_soundboard();
		assert!(state.request_soundboard()[1].is_some());
		state.apply_soundboard(Event::Guild {
			guild: Id(9),
			result: Ok(vec![sound(3, Some(9))]),
		});
		assert!(state.soundboard.sound(Id(1)).is_some());
		assert!(state.soundboard.sound(Id(3)).is_some());
		assert!(state.request_soundboard().iter().all(Option::is_none));

		state.apply_soundboard(Event::Changed(Id(8)));
		assert!(state.request_soundboard().iter().all(Option::is_none));
		state.apply_soundboard(Event::Changed(Id(9)));
		assert!(matches!(
			state.request_soundboard(),
			[None, Some(Command::Soundboard(Request::Guild(Id(9))))]
		));
		// A change during that read keeps its response but schedules another read.
		state.apply_soundboard(Event::Changed(Id(9)));
		state.apply_soundboard(Event::Guild {
			guild: Id(9),
			result: Ok(vec![sound(4, Some(9))]),
		});
		assert!(state.soundboard.sound(Id(4)).is_some());
		assert!(matches!(
			state.request_soundboard(),
			[None, Some(Command::Soundboard(Request::Guild(Id(9))))]
		));
		state.apply_soundboard(Event::Guild {
			guild: Id(9),
			result: Ok(vec![sound(4, Some(9))]),
		});
		assert!(state.request_soundboard().iter().all(Option::is_none));

		state.voice.active.as_mut().unwrap().guild = None;
		assert!(state.request_soundboard().iter().all(Option::is_none));
		assert!(state.soundboard_unavailable().is_some());
	}
	#[test]
	fn playing_requires_permission_an_undeafened_call_and_one_request_at_a_time() {
		let quack = sound(1, None);
		// Missing SPEAK or USE_SOUNDBOARD never authorizes a write.
		for bits in [ALLOWED & !p::USE_SOUNDBOARD, ALLOWED & !p::SPEAK] {
			let mut denied = connected(bits);
			assert!(denied.soundboard_unavailable().is_some());
			assert!(denied.play_sound(&quack).is_none());
		}
		for edit in [
			(|call: &mut Call| call.deafened = true) as fn(&mut Call),
			|call| call.server_muted = true,
			|call| call.server_deafened = true,
			|call| call.phase = Phase::Securing,
		] {
			let mut blocked = connected(ALLOWED);
			edit(blocked.voice.active.as_mut().unwrap());
			assert!(blocked.play_sound(&quack).is_none());
		}
		let mut state = connected(ALLOWED);
		let mut unavailable = quack.clone();
		unavailable.available = false;
		assert!(state.play_sound(&unavailable).is_none());
		// Another server's sound needs the external-sounds permission and names its source.
		assert!(state.play_sound(&sound(4, Some(8))).is_none());
		let mut external = connected(ALLOWED | p::USE_EXTERNAL_SOUNDS);
		assert!(matches!(
			external.play_sound(&sound(4, Some(8))),
			Some(Command::Soundboard(Request::Send {
				source_guild: Some(Id(8)),
				..
			}))
		));

		assert!(matches!(
			state.play_sound(&sound(3, Some(9))),
			Some(Command::Soundboard(Request::Send {
				channel: Id(2),
				sound: Id(3),
				source_guild: None
			}))
		));
		assert!(state.play_sound(&quack).is_none());
		// An outcome for another request leaves the pending one in place.
		state.apply_soundboard(Event::Sent {
			channel: Id(2),
			sound: Id(1),
			result: Ok(()),
		});
		assert!(state.soundboard.sending.is_some());
		state.apply_soundboard(Event::Sent {
			channel: Id(2),
			sound: Id(3),
			result: Err(Failure::Forbidden),
		});
		assert!(state.soundboard.sending.is_none());
		assert!(state.soundboard.error.is_some());
		assert!(state.play_sound(&quack).is_some());
		assert!(state.soundboard.error.is_none());
		state.interrupt_soundboard();
		assert!(state.soundboard.sending.is_none());
	}
}
