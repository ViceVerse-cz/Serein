//! One explicit, bounded soundboard action for the current guild voice connection.
use crate::{State, auth::Failure, voice::Phase};
use model::Id;
use std::time::{Duration, Instant};

pub use model::soundboard::{MAX_BYTES, MAX_SOUNDS, Sound, catalog_bytes, valid_catalog};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
	pub generation: u64,
	pub channel: Id,
	pub guild: Id,
	pub call_request: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
	Load,
	Play(Id),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
	pub scope: Scope,
	pub request: u64,
	pub action: Action,
}
pub enum Outcome {
	Loaded(Vec<Sound>),
	Played,
}
pub struct Event {
	pub scope: Scope,
	pub request: u64,
	pub result: Result<Outcome, Failure>,
}
impl Event {
	pub fn bytes(&self) -> usize {
		match &self.result {
			Ok(Outcome::Loaded(sounds)) => {
				catalog_bytes(sounds) + (sounds.capacity() - sounds.len()) * size_of::<Sound>()
			}
			_ => 0,
		}
	}
}
#[derive(Default)]
pub struct Soundboard {
	pub scope: Option<Scope>,
	pub sounds: Vec<Sound>,
	pub pending: Option<(Scope, u64)>,
	pub error: Option<Failure>,
	sequence: u64,
	last_play: Option<Instant>,
}
impl Soundboard {
	pub fn clear(&mut self) {
		self.scope = None;
		self.sounds = Vec::new();
		self.pending = None;
		self.error = None;
		self.last_play = None;
	}
}
impl State {
	pub fn soundboard_scope(&self) -> Option<Scope> {
		let call = self.voice.active.as_ref()?;
		let guild = call.guild?;
		(self.can_call(call.channel)
			&& matches!(call.phase, Phase::Connected | Phase::Waiting)
			&& !call.server_muted
			&& !call.server_deafened
			&& !call.deafened
			&& self
				.channel(call.channel)
				.is_some_and(|channel| channel.guild == Some(guild) && channel.kind == 2)
			&& self.permission(
				call.channel,
				model::permissions::SPEAK | model::permissions::USE_SOUNDBOARD,
			) == Some(true))
		.then_some(Scope {
			generation: self.generation,
			channel: call.channel,
			guild,
			call_request: call.request,
		})
	}
	pub fn revalidate_soundboard(&mut self) {
		let current = self.soundboard_scope();
		if self
			.soundboard
			.scope
			.is_some_and(|scope| Some(scope) != current)
			|| self
				.soundboard
				.pending
				.is_some_and(|(scope, _)| Some(scope) != current)
		{
			self.soundboard.clear();
		}
	}
	pub fn request_soundboard(&mut self, action: Action) -> Option<crate::Command> {
		self.revalidate_soundboard();
		let scope = self.soundboard_scope()?;
		if self.soundboard.pending.is_some() {
			return None;
		}
		if let Action::Play(id) = action {
			if self.soundboard.scope != Some(scope)
				|| !self
					.soundboard
					.sounds
					.iter()
					.any(|sound| sound.id == id && sound.available)
				|| self
					.soundboard
					.last_play
					.is_some_and(|last| last.elapsed() < Duration::from_secs(1))
			{
				return None;
			}
			self.soundboard.last_play = Some(Instant::now());
		}
		self.soundboard.sequence = self.soundboard.sequence.wrapping_add(1);
		let request = self.soundboard.sequence;
		self.soundboard.pending = Some((scope, request));
		self.soundboard.error = None;
		Some(crate::Command::Soundboard(Request {
			scope,
			request,
			action,
		}))
	}
	pub fn can_play_soundboard(&self, id: Id) -> bool {
		self.soundboard.pending.is_none()
			&& self
				.soundboard_scope()
				.is_some_and(|scope| self.soundboard.scope == Some(scope))
			&& self
				.soundboard
				.sounds
				.iter()
				.any(|sound| sound.id == id && sound.available)
			&& self
				.soundboard
				.last_play
				.is_none_or(|last| last.elapsed() >= Duration::from_secs(1))
	}
	pub(crate) fn apply_soundboard(&mut self, event: Event) {
		self.revalidate_soundboard();
		if self.soundboard_scope() != Some(event.scope)
			|| self.soundboard.pending != Some((event.scope, event.request))
		{
			return;
		}
		self.soundboard.pending = None;
		match event.result {
			Ok(Outcome::Loaded(sounds))
				if valid_catalog(&sounds, event.scope.guild)
					&& catalog_bytes(&sounds)
						+ (sounds.capacity() - sounds.len()) * size_of::<Sound>()
						<= MAX_BYTES =>
			{
				self.soundboard.scope = Some(event.scope);
				self.soundboard.sounds = sounds;
			}
			Ok(Outcome::Loaded(_)) => self.soundboard.error = Some(Failure::Protocol),
			Ok(Outcome::Played) => {}
			Err(failure) => self.soundboard.error = Some(failure),
		}
	}
}
