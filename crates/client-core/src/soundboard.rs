//! Session-only default sounds. No audio download, upload or microphone access.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
	voice::Phase,
};
use model::Id;

pub const MAX_SOUNDS: usize = 32;
pub const MAX_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sound {
	pub id: Id,
	pub name: String,
	pub emoji: Option<String>,
	pub available: bool,
}

impl Sound {
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& !self.name.trim().is_empty()
			&& self.name.len() <= 128
			&& !self.name.chars().any(char::is_control)
			&& self
				.emoji
				.as_ref()
				.is_none_or(|s| s.len() <= 128 && !s.chars().any(char::is_control))
	}
}
pub fn bytes(sounds: &Vec<Sound>) -> usize {
	sounds.capacity() * size_of::<Sound>()
		+ sounds
			.iter()
			.map(|s| s.name.capacity() + s.emoji.as_ref().map_or(0, String::capacity))
			.sum::<usize>()
}
pub fn valid(sounds: &Vec<Sound>) -> bool {
	sounds.len() <= MAX_SOUNDS
		&& bytes(sounds) <= MAX_BYTES
		&& sounds
			.iter()
			.enumerate()
			.all(|(i, s)| s.valid() && !sounds[..i].iter().any(|other| other.id == s.id))
}
#[derive(Clone, Copy, Debug)]
pub enum Request {
	List {
		request: u64,
	},
	Play {
		request: u64,
		channel: Id,
		call: u64,
		sound: Id,
	},
}
pub enum Event {
	Listed {
		request: u64,
		result: Result<Vec<Sound>, Failure>,
	},
	Played {
		request: u64,
		channel: Id,
		call: u64,
		result: Result<(), Failure>,
	},
}
#[derive(Default)]
pub struct Catalog {
	pub sounds: Vec<Sound>,
	pub loaded: bool,
	pub loading: Option<u64>,
	pub playing: Option<(u64, Id, u64)>,
	pub error: Option<&'static str>,
	sequence: u64,
}
impl Catalog {
	pub(crate) fn reset(&mut self) {
		let sequence = self.sequence;
		*self = Self {
			sequence,
			..Self::default()
		};
	}
	fn next(&mut self) -> u64 {
		self.sequence = self.sequence.wrapping_add(1);
		self.sequence
	}
}
impl State {
	pub fn request_default_sounds(&mut self) -> Option<Command> {
		if self.demo
			|| self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.soundboard.loaded
			|| self.soundboard.loading.is_some()
		{
			return None;
		}
		let request = self.soundboard.next();
		self.soundboard.loading = Some(request);
		self.soundboard.error = None;
		Some(Command::Soundboard(Request::List { request }))
	}
	pub fn can_soundboard(&self, channel: Id) -> bool {
		!self.demo
			&& self.can_call(channel)
			&& self
				.channel(channel)
				.is_some_and(|c| c.guild.is_some() && c.kind == 2)
			&& self.permission(
				channel,
				model::permissions::SPEAK | model::permissions::USE_SOUNDBOARD,
			) == Some(true)
			&& self.voice.active.as_ref().is_some_and(|call| {
				call.channel == channel
					&& matches!(call.phase, Phase::Connected | Phase::Waiting)
					&& !call.deafened
					&& !call.server_deafened
					&& !call.server_muted
			})
	}
	pub fn play_default_sound(&mut self, channel: Id, sound: Id) -> Option<Command> {
		if !self.can_soundboard(channel)
			|| self.soundboard.playing.is_some()
			|| !self
				.soundboard
				.sounds
				.iter()
				.any(|s| s.id == sound && s.available)
		{
			return None;
		}
		let call = self.voice.active.as_ref()?.request;
		let request = self.soundboard.next();
		self.soundboard.playing = Some((request, channel, call));
		self.soundboard.error = None;
		Some(Command::Soundboard(Request::Play {
			request,
			channel,
			call,
			sound,
		}))
	}
	pub(crate) fn apply_soundboard(&mut self, event: Event) {
		match event {
			Event::Listed { request, result } if self.soundboard.loading == Some(request) => {
				self.soundboard.loading = None;
				match result {
					Ok(sounds) if valid(&sounds) => {
						self.soundboard.sounds = sounds;
						self.soundboard.loaded = true;
					}
					Ok(_) => self.soundboard.error = Some(Failure::Protocol.label()),
					Err(e) => self.soundboard.error = Some(e.label()),
				}
			}
			Event::Played {
				request,
				channel,
				call,
				result,
			} if self.soundboard.playing == Some((request, channel, call)) => {
				self.soundboard.playing = None;
				if self
					.voice
					.active
					.as_ref()
					.is_some_and(|active| active.channel == channel && active.request == call)
				{
					self.soundboard.error = result.err().map(Failure::label);
				}
			}
			_ => {}
		}
	}
	pub(crate) fn reject_soundboard(&mut self, request: Request) {
		self.apply_soundboard(match request {
			Request::List { request } => Event::Listed {
				request,
				result: Err(Failure::Capacity),
			},
			Request::Play {
				request,
				channel,
				call,
				..
			} => Event::Played {
				request,
				channel,
				call,
				result: Err(Failure::Capacity),
			},
		});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn fixture() -> State {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				id: Id(1),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
			}),
			guilds: vec![model::Guild {
				id: Id(10),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
				stickers: None,
				default_message_notifications: None,
			}],
			channels: vec![model::Channel {
				id: Id(20),
				guild: Some(Id(10)),
				kind: 2,
				name: "Voice".into(),
				last_message: None,
				parent_id: None,
				position: 0,
				recipients: vec![],
				icon: None,
				member_list_id: None,
				tags: None,
				message_count: None,
			}],
			..State::default()
		};
		crate::tests::grant_permissions(&mut state);
		assert!(state.start_call(Id(20), false).is_some());
		state.voice.active.as_mut().unwrap().phase = Phase::Waiting;
		state.soundboard.sounds = vec![Sound {
			id: Id(1),
			name: "Synthetic quack".into(),
			emoji: None,
			available: true,
		}];
		state.soundboard.loaded = true;
		state
	}
	#[test]
	fn explicit_sound_send_requires_current_join_and_permissions_and_never_replays() {
		let mut state = fixture();
		assert!(state.play_default_sound(Id(20), Id(9)).is_none());
		state.voice.active.as_mut().unwrap().muted = true; // Self-mute does not prohibit service soundboard.
		assert!(state.can_soundboard(Id(20)));
		let Some(Command::Soundboard(Request::Play {
			request,
			channel,
			call,
			sound,
		})) = state.play_default_sound(Id(20), Id(1))
		else {
			panic!("explicit send");
		};
		assert_eq!(sound, Id(1));
		assert!(state.play_default_sound(Id(20), Id(1)).is_none());
		state.voice.active.as_mut().unwrap().request += 1;
		state.apply_soundboard(Event::Played {
			request,
			channel,
			call,
			result: Err(Failure::Forbidden),
		});
		assert!(state.soundboard.playing.is_none());
		assert!(state.soundboard.error.is_none()); // Old call's failure does not affect the new call.
		state.voice.active.as_mut().unwrap().server_muted = true;
		assert!(!state.can_soundboard(Id(20)));
		state.voice.active.as_mut().unwrap().server_muted = false;
		state.voice.active.as_mut().unwrap().deafened = true;
		assert!(!state.can_soundboard(Id(20)));
		state.voice.active.as_mut().unwrap().deafened = false;
		state.channels[0].kind = 13;
		assert!(!state.can_soundboard(Id(20))); // Suppressed stage states are outside this slice.
		state.channels[0].kind = 2;
		state.permissions.replace(Default::default()).unwrap();
		assert!(!state.can_soundboard(Id(20))); // Unknown permissions fail closed.
	}
	#[test]
	fn default_catalog_is_bounded_and_session_changes_reject_stale_results() {
		let mut state = fixture();
		state.soundboard.reset();
		let Some(Command::Soundboard(request @ Request::List { .. })) =
			state.request_default_sounds()
		else {
			panic!("load");
		};
		assert!(state.request_default_sounds().is_none());
		state.command_rejected(Command::Soundboard(request));
		assert!(state.soundboard.loading.is_none());
		let Some(Command::Soundboard(Request::List { request })) = state.request_default_sounds()
		else {
			panic!("retry");
		};
		state.soundboard.reset();
		state.apply_soundboard(Event::Listed {
			request,
			result: Ok(vec![Sound {
				id: Id(1),
				name: "old".into(),
				emoji: None,
				available: true,
			}]),
		});
		assert!(state.soundboard.sounds.is_empty());
		let mut sounds = vec![
			Sound {
				id: Id(1),
				name: "x".into(),
				emoji: None,
				available: true
			};
			MAX_SOUNDS + 1
		];
		assert!(!valid(&sounds));
		sounds.truncate(1);
		sounds[0].name = "x".repeat(MAX_BYTES + 1);
		assert!(!valid(&sounds));
		state.demo = true;
		assert!(state.request_default_sounds().is_none());
	}
}
