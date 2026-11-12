//! Session-only soundboard catalog, playback admission and received soundboard effects.
//!
//! Discord never mixes soundboard audio into participants' RTP streams. A played sound reaches
//! every client in the channel as `VOICE_CHANNEL_EFFECT_SEND`; each client downloads the clip
//! from the CDN and plays it locally. Outgoing sounds use the documented REST endpoint. Whether
//! the sender also receives its own event is not documented, so each send is tracked until its
//! REST reply and, if one comes, its echo have arrived: the sender hears it exactly once
//! whichever comes first, even with several sends (of the same or different sounds) in flight.
//! Previews never touch Discord.
use crate::{
	Command as CoreCommand, State,
	auth::{AuthState, Failure},
	voice::Phase,
};
use model::{
	Id, permissions as p,
	soundboard::{Sound, SoundEffect},
};
use std::{collections::VecDeque, time::Instant};

/// Server catalogs retained at once; normal accounts are limited to 200 servers.
pub const MAX_GUILD_CATALOGS: usize = 256;
pub const MAX_CATALOG_BYTES: usize = 2 * 1024 * 1024;
/// Received effects waiting for the desktop player; older ones are dropped first.
pub const MAX_PENDING_EFFECTS: usize = 8;
/// A RESUME may replay an effect; the same sender, sound and channel inside this window is one.
const DUPLICATE_WINDOW: std::time::Duration = std::time::Duration::from_millis(400);
/// An echo of this account's own sound arriving this long after the send succeeded is the
/// same playback, already heard locally; later, the send stops waiting for its echo.
const OWN_ECHO_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);
/// Sends in flight at once. Discord rate-limits the endpoint; this only bounds local state.
pub const MAX_OUTSTANDING_SENDS: usize = 3;
/// Own sends remembered at once, in flight or waiting for their echo; the oldest answered
/// one is forgotten first.
const MAX_TRACKED_SENDS: usize = 16;
/// A send with no reply after this is released so it cannot hold a slot forever.
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// A catalog request the voice queue refused waits this long before it is tried again.
const CATALOG_RETRY: std::time::Duration = std::time::Duration::from_secs(2);
/// More newly missing servers than this are requested together with every known server.
const MAX_SINGLE_CATALOG_REQUESTS: usize = 4;
/// Dropped effects kept for the opt-in diagnostics; older reasons are discarded first.
const MAX_DROPPED: usize = 8;

/// Why a sound is about to play on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
	/// Someone else's `VOICE_CHANNEL_EFFECT_SEND`, or this account's from another device.
	Received,
	/// A sound this device sent, played once for its sender.
	Own,
	/// A local preview; nothing was sent to Discord.
	Preview,
}
impl Source {
	/// Short name used in diagnostics.
	pub fn label(self) -> &'static str {
		match self {
			Self::Received => "effect",
			Self::Own => "own",
			Self::Preview => "preview",
		}
	}
}
/// One sound for the desktop player to fetch, decode and mix into the call's playback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playback {
	pub effect: SoundEffect,
	pub source: Source,
}

/// One of this account's sends, kept until both halves of its lifecycle (the REST reply and
/// Discord's echo) are known, so the sender hears it exactly once.
#[derive(Clone, Copy)]
struct OwnSend {
	request: u64,
	effect: SoundEffect,
	sent_at: Instant,
	/// When Discord accepted the send; `None` while the request is in flight.
	accepted_at: Option<Instant>,
	/// Discord's event for this send arrived.
	echoed: bool,
	/// The sound was queued for local playback, by whichever half came first.
	played: bool,
}
impl OwnSend {
	/// Still waiting for Discord's reply and not yet past `SEND_TIMEOUT`.
	fn awaiting_reply(&self, now: Instant) -> bool {
		self.accepted_at.is_none() && now.saturating_duration_since(self.sent_at) < SEND_TIMEOUT
	}
}

#[derive(Clone, Copy, Debug)]
pub enum Command {
	/// `GET /soundboard-default-sounds`.
	Defaults,
	/// `POST /channels/{channel}/send-soundboard-sound`.
	Play {
		request: u64,
		channel: Id,
		sound: Id,
		/// Required by Discord when the sound belongs to another server.
		source_guild: Option<Id>,
	},
}

pub enum Event {
	Defaults(Result<Vec<Sound>, Failure>),
	/// A whole server catalog: the opcode 31 reply or `GUILD_SOUNDBOARD_SOUNDS_UPDATE`.
	Guild {
		guild: Id,
		sounds: Vec<Sound>,
	},
	Upsert(Sound),
	Removed {
		guild: Id,
		sound: Id,
	},
	Effect(SoundEffect),
	Played {
		request: u64,
		result: Result<(), Failure>,
	},
	/// The desktop could not hand a catalog request to the Gateway; `None` stands for a
	/// request for every server.
	CatalogUnsent(Option<Id>),
}
impl Event {
	/// Retained catalog bytes this event would add, for the session memory budget.
	pub fn bytes(&self) -> usize {
		match self {
			Self::Defaults(Ok(sounds)) | Self::Guild { sounds, .. } => catalog_bytes(sounds),
			Self::Upsert(sound) => sound.bytes(),
			_ => 0,
		}
	}
}
/// Heap and inline bytes of one catalog, counting reserved capacity.
fn catalog_bytes(sounds: &Vec<Sound>) -> usize {
	sounds.capacity() * size_of::<Sound>()
		+ sounds
			.iter()
			.map(|sound| sound.bytes() - size_of::<Sound>())
			.sum::<usize>()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Load {
	#[default]
	Idle,
	Loading,
	Loaded,
	Failed(&'static str),
}

#[derive(Default)]
pub struct Soundboard {
	pub defaults: Vec<Sound>,
	pub defaults_load: Load,
	/// Server catalogs received since the last READY, in arrival order.
	pub guilds: Vec<(Id, Vec<Sound>)>,
	/// Servers whose catalog was requested with opcode 31 on this connection, and whether it
	/// arrived. Replies come per server without a final marker.
	requested: Vec<(Id, bool)>,
	/// A refused catalog request is retried no earlier than this.
	catalog_retry_at: Option<Instant>,
	pub error: Option<&'static str>,
	effects: VecDeque<Playback>,
	last_effect: Option<(SoundEffect, Instant)>,
	/// This account's sends in flight or waiting for their echo, oldest first.
	own: VecDeque<OwnSend>,
	/// Effects that will not play and why, for the opt-in diagnostics only.
	dropped: VecDeque<(Id, &'static str)>,
	sequence: u64,
}
impl Soundboard {
	/// The catalog received for `guild`, if any.
	pub fn guild(&self, guild: Id) -> Option<&[Sound]> {
		self.guilds
			.iter()
			.find(|(id, _)| *id == guild)
			.map(|(_, sounds)| sounds.as_slice())
	}
	/// Any known sound by ID, for naming received effects.
	pub fn sound(&self, id: Id) -> Option<&Sound> {
		self.defaults
			.iter()
			.chain(self.guilds.iter().flat_map(|(_, sounds)| sounds))
			.find(|sound| sound.id == id)
	}
	/// Whether a send of `sound` is waiting for Discord's reply, for the picker's busy state.
	pub fn is_sending(&self, sound: Id) -> bool {
		let now = Instant::now();
		self.own
			.iter()
			.any(|own| own.effect.sound == sound && own.awaiting_reply(now))
	}
	/// Sends waiting for Discord's reply. A send older than `SEND_TIMEOUT` no longer counts,
	/// even before `prune_own` has removed it.
	pub fn outstanding_sends(&self) -> usize {
		self.outstanding_sends_at(Instant::now())
	}
	/// `outstanding_sends` at an explicit time, so expiry is testable.
	fn outstanding_sends_at(&self, now: Instant) -> usize {
		self.own
			.iter()
			.filter(|own| own.awaiting_reply(now))
			.count()
	}
	/// Bytes held by every retained catalog.
	fn bytes(&self) -> usize {
		catalog_bytes(&self.defaults)
			+ self
				.guilds
				.iter()
				.map(|(_, sounds)| catalog_bytes(sounds))
				.sum::<usize>()
	}
	/// Queue a sound for the desktop player; past the bound the oldest one is dropped and
	/// recorded for diagnostics.
	fn queue(&mut self, playback: Playback) {
		if self.effects.len() == MAX_PENDING_EFFECTS
			&& let Some(oldest) = self.effects.pop_front()
		{
			self.drop_effect(oldest.effect.sound, "queue-full");
		}
		self.effects.push_back(playback);
	}
	/// Record why a sound will not play, for the opt-in diagnostics. Bounded; oldest first out.
	fn drop_effect(&mut self, sound: Id, reason: &'static str) {
		if self.dropped.len() == MAX_DROPPED {
			self.dropped.pop_front();
		}
		self.dropped.push_back((sound, reason));
	}
	/// Forget sends that can no longer be answered or echoed: an echo window that passed, a
	/// request that never replied, or a call this device left.
	fn prune_own(&mut self, call: Option<Id>, now: Instant) {
		let mut timed_out = false;
		self.own.retain(|own| {
			let keep = Some(own.effect.channel) == call
				&& match own.accepted_at {
					Some(at) => now.saturating_duration_since(at) < OWN_ECHO_WINDOW,
					None => now.saturating_duration_since(own.sent_at) < SEND_TIMEOUT,
				};
			timed_out |= !keep && own.accepted_at.is_none() && !own.played;
			keep
		});
		if timed_out {
			self.error = Some("The sound may not have played");
		}
	}
	/// The Gateway connection changed. A resume (`fresh_session == false`) keeps everything
	/// that is still valid: REST sends are independent of the Gateway socket and still report,
	/// and catalogs that arrived stay current. Only catalog requests the old connection never
	/// answered are asked again. A fresh session forgets server catalogs and abandons sends,
	/// whose replies and echoes belong to the old session.
	pub(crate) fn interrupt(&mut self, fresh_session: bool) {
		if fresh_session {
			if self.own.iter().any(|own| own.accepted_at.is_none()) {
				self.error = Some("The connection dropped; the sound may not have played");
			}
			self.own.clear();
			self.guilds.clear();
			self.requested.clear();
		} else {
			self.requested.retain(|(_, arrived)| *arrived);
		}
		self.catalog_retry_at = None;
		if self.defaults_load == Load::Loading {
			self.defaults_load = Load::Idle;
		}
	}
}

impl State {
	/// Load whatever the picker still lacks: default sounds by REST, server sounds by opcode 31.
	/// A failed default catalog is retried only when `retry` marks an explicit open, never on
	/// every frame the picker stays open. Server catalogs are tracked per server, so a server
	/// joined later is requested on its own and none is asked for twice on one connection.
	pub fn request_soundboard(&mut self, retry: bool) -> Vec<CoreCommand> {
		self.request_soundboard_at(retry, Instant::now())
	}
	/// `request_soundboard` at an explicit time, so retry delays are testable.
	fn request_soundboard_at(&mut self, retry: bool, now: Instant) -> Vec<CoreCommand> {
		let mut commands = Vec::new();
		if self.demo || self.auth != AuthState::Authenticated || !self.gateway_connected {
			return commands;
		}
		if self.soundboard.defaults_load == Load::Idle
			|| (retry && matches!(self.soundboard.defaults_load, Load::Failed(_)))
		{
			self.soundboard.defaults_load = Load::Loading;
			commands.push(CoreCommand::Soundboard(Command::Defaults));
		}
		// A server this account left is neither requested nor shown any more.
		let guilds = &self.guilds;
		let known = |id: &Id| guilds.iter().any(|guild| guild.id == *id);
		self.soundboard.requested.retain(|(id, _)| known(id));
		self.soundboard.guilds.retain(|(id, _)| known(id));
		if self.soundboard.catalog_retry_at.is_some_and(|at| now < at) {
			return commands;
		}
		let missing: Vec<Id> = self
			.guilds
			.iter()
			.map(|guild| guild.id)
			.filter(|id| {
				!self
					.soundboard
					.requested
					.iter()
					.any(|(known, _)| known == id)
			})
			.collect();
		if missing.is_empty() {
			return commands;
		}
		let everything =
			self.soundboard.requested.is_empty() || missing.len() > MAX_SINGLE_CATALOG_REQUESTS;
		self.soundboard
			.requested
			.extend(missing.iter().map(|id| (*id, false)));
		if everything {
			commands.push(CoreCommand::Voice(
				crate::voice::Command::RequestSoundboard { guild: None },
			));
		} else {
			commands.extend(missing.into_iter().map(|guild| {
				CoreCommand::Voice(crate::voice::Command::RequestSoundboard { guild: Some(guild) })
			}));
		}
		commands
	}
	/// Why this account cannot play `sound` in its current call, following Discord's documented
	/// requirements: a connected, undeafened, unsuppressed voice state with Speak and Use
	/// Soundboard, plus Use External Sounds for another server's sound. In private and group
	/// calls every server sound comes from another server; default sounds need nothing extra.
	pub fn soundboard_block(&self, sound: &Sound) -> Option<&'static str> {
		self.soundboard_block_at(sound, Instant::now())
	}
	/// `soundboard_block` at an explicit time, so send expiry is testable.
	fn soundboard_block_at(&self, sound: &Sound, now: Instant) -> Option<&'static str> {
		if self.demo {
			return Some("Sounds don't play in the offline preview");
		}
		let Some(call) = self.voice.active.as_ref() else {
			return Some("Join a call to use the soundboard");
		};
		if !matches!(call.phase, Phase::Connected | Phase::Waiting) {
			return Some("Wait for the call to connect");
		}
		if !self.gateway_connected || self.auth != AuthState::Authenticated {
			return Some("Reconnect to use the soundboard");
		}
		if call.deafened || call.server_deafened {
			return Some("Undeafen to use the soundboard");
		}
		if call.server_muted {
			return Some("You are server muted");
		}
		if !sound.available {
			return Some("This sound is unavailable");
		}
		if self.permission(call.channel, p::SPEAK | p::USE_SOUNDBOARD) != Some(true) {
			return Some("You don't have permission to use the soundboard here");
		}
		if sound.guild.is_some_and(|source| Some(source) != call.guild) {
			if !self.stickers.external_allowed {
				return Some("Sounds from other servers require Nitro");
			}
			if self.permission(call.channel, p::USE_EXTERNAL_SOUNDS) != Some(true) {
				return Some("This server doesn't allow sounds from other servers");
			}
		}
		if self.soundboard.outstanding_sends_at(now) >= MAX_OUTSTANDING_SENDS {
			return Some("Playing a sound…");
		}
		None
	}
	/// Release sends that never got a reply or echo. The picker calls this every frame it is
	/// open, so stalled sends free their slots without waiting for an incoming sound effect.
	pub fn expire_soundboard_sends(&mut self) {
		self.expire_soundboard_sends_at(Instant::now());
	}
	/// `expire_soundboard_sends` at an explicit time, so expiry is testable.
	fn expire_soundboard_sends_at(&mut self, now: Instant) {
		let call = self.voice.active.as_ref().map(|call| call.channel);
		self.soundboard.prune_own(call, now);
	}
	/// Send `sound` to the current call, or record why it cannot be sent. The send is tracked
	/// until its REST reply and its echo both arrive, so the sender hears it exactly once.
	pub fn play_sound(&mut self, sound: &Sound) -> Option<CoreCommand> {
		if let Some(reason) = self.soundboard_block(sound) {
			self.soundboard.error = Some(reason);
			return None;
		}
		let call = self.voice.active.as_ref()?;
		let user = self.user.as_ref()?.id;
		let channel = call.channel;
		let guild = call.guild;
		let source_guild = sound.guild.filter(|source| Some(*source) != call.guild);
		let now = Instant::now();
		self.soundboard.prune_own(Some(channel), now);
		if self.soundboard.own.len() == MAX_TRACKED_SENDS {
			// Forget the oldest send already answered; at most `MAX_OUTSTANDING_SENDS` are not.
			let answered = self
				.soundboard
				.own
				.iter()
				.position(|own| own.accepted_at.is_some());
			self.soundboard.own.remove(answered.unwrap_or(0));
		}
		self.soundboard.sequence = self.soundboard.sequence.wrapping_add(1);
		let request = self.soundboard.sequence;
		self.soundboard.own.push_back(OwnSend {
			request,
			effect: SoundEffect {
				channel,
				guild,
				user,
				sound: sound.id,
				volume: sound.volume,
			},
			sent_at: now,
			accepted_at: None,
			echoed: false,
			played: false,
		});
		self.soundboard.error = None;
		Some(CoreCommand::Soundboard(Command::Play {
			request,
			channel,
			sound: sound.id,
			source_guild,
		}))
	}
	/// Why a local preview cannot play now. Previews share the call's local playback mix, so
	/// they need connected media and an undeafened call, but no permission or Nitro.
	pub fn soundboard_preview_block(&self) -> Option<&'static str> {
		if self.demo {
			return Some("Sounds don't play in the offline preview");
		}
		let Some(call) = self.voice.active.as_ref() else {
			return Some("Join a call to preview sounds");
		};
		if !matches!(call.phase, Phase::Connected | Phase::Waiting) {
			return Some("Wait for the call to connect");
		}
		if call.deafened || call.server_deafened {
			return Some("Undeafen to preview sounds");
		}
		None
	}
	/// Play `sound` on this device only. Nothing is sent to Discord and no Gateway event is
	/// awaited: the desktop downloads (or reuses) the clip and mixes it into local playback,
	/// which never reaches capture or the encoder, so no participant hears it.
	pub fn preview_sound(&mut self, sound: &Sound) -> bool {
		if let Some(reason) = self.soundboard_preview_block() {
			self.soundboard.error = Some(reason);
			return false;
		}
		let (Some(call), Some(user)) = (self.voice.active.as_ref(), self.user.as_ref()) else {
			return false;
		};
		let effect = SoundEffect {
			channel: call.channel,
			guild: call.guild,
			user: user.id,
			sound: sound.id,
			volume: sound.volume,
		};
		self.soundboard.error = None;
		self.soundboard.queue(Playback {
			effect,
			source: Source::Preview,
		});
		true
	}
	/// Sounds to play for the call this device is in, oldest first. The desktop player drains them.
	pub fn take_sound_effects(&mut self) -> Vec<Playback> {
		self.soundboard.effects.drain(..).collect()
	}
	/// Effects that were received but will not play, with the reason, for diagnostics.
	pub fn take_dropped_sound_effects(&mut self) -> Vec<(Id, &'static str)> {
		self.soundboard.dropped.drain(..).collect()
	}
	/// Why the current call cannot play `effect` now, if it cannot.
	fn effect_block(&self, effect: &SoundEffect) -> Option<&'static str> {
		let Some(call) = self.voice.active.as_ref() else {
			return Some("no-call");
		};
		if call.channel != effect.channel
			|| (call.guild.is_some() && effect.guild.is_some() && call.guild != effect.guild)
		{
			return Some("other-channel");
		}
		if !matches!(call.phase, Phase::Connected | Phase::Waiting) {
			return Some("call-not-connected");
		}
		if call.deafened || call.server_deafened {
			return Some("deafened");
		}
		None
	}
	/// The desktop could not queue `command`: release what it held and say so.
	pub(crate) fn soundboard_rejected(&mut self, command: Command) {
		match command {
			Command::Defaults => {
				self.soundboard.defaults_load = Load::Failed("Sounds were not queued; try again");
			}
			Command::Play { request, .. } => {
				let before = self.soundboard.own.len();
				self.soundboard.own.retain(|own| own.request != request);
				if self.soundboard.own.len() != before {
					self.soundboard.error = Some("The sound was not sent; try again");
				}
			}
		}
	}
	/// Apply one soundboard event from the Gateway, the REST worker or the desktop.
	pub fn apply_soundboard(&mut self, event: Event) {
		self.apply_soundboard_at(event, Instant::now());
	}
	/// `apply_soundboard` at an explicit time, so echo windows and retries are testable.
	fn apply_soundboard_at(&mut self, event: Event, now: Instant) {
		match event {
			Event::Defaults(Ok(sounds)) => {
				let previous = std::mem::take(&mut self.soundboard.defaults);
				if self.soundboard.bytes() + catalog_bytes(&sounds) > MAX_CATALOG_BYTES {
					self.soundboard.defaults = previous;
					self.soundboard.defaults_load = Load::Failed("Too many sounds to show");
					return;
				}
				self.soundboard.defaults = sounds;
				self.soundboard.defaults_load = Load::Loaded;
			}
			Event::Defaults(Err(failure)) => {
				self.soundboard.defaults_load = Load::Failed(match failure {
					Failure::RateLimited => "Rate limited · try again shortly",
					_ => "Default sounds couldn't load",
				});
			}
			Event::Guild { guild, sounds } => {
				if !self.guilds.iter().any(|known| known.id == guild) {
					return;
				}
				let position = self
					.soundboard
					.guilds
					.iter()
					.position(|(id, _)| *id == guild);
				let previous = position.map(|index| self.soundboard.guilds.remove(index));
				// Mark the server answered even over budget, so it is not requested in a loop.
				match self
					.soundboard
					.requested
					.iter_mut()
					.find(|(id, _)| *id == guild)
				{
					Some((_, arrived)) => *arrived = true,
					None => self.soundboard.requested.push((guild, true)),
				}
				if self.soundboard.guilds.len() >= MAX_GUILD_CATALOGS
					|| self.soundboard.bytes() + catalog_bytes(&sounds) > MAX_CATALOG_BYTES
				{
					// Keep the previous catalog rather than exceed the budget.
					if let Some(previous) = previous {
						self.soundboard.guilds.push(previous);
					}
					return;
				}
				self.soundboard.guilds.push((guild, sounds));
			}
			Event::Upsert(sound) => {
				let Some(guild) = sound.guild else {
					return;
				};
				let budget = self.soundboard.bytes() + sound.bytes();
				// A server joined after the catalogs loaded may announce a sound before its own
				// catalog arrives: start the catalog with it; the full reply then replaces it.
				if !self.soundboard.guilds.iter().any(|(id, _)| *id == guild) {
					if self.guilds.iter().any(|known| known.id == guild)
						&& self.soundboard.guilds.len() < MAX_GUILD_CATALOGS
						&& budget <= MAX_CATALOG_BYTES
					{
						self.soundboard.guilds.push((guild, vec![sound]));
					}
					return;
				}
				let Some((_, sounds)) = self
					.soundboard
					.guilds
					.iter_mut()
					.find(|(id, _)| *id == guild)
				else {
					return;
				};
				if let Some(known) = sounds.iter_mut().find(|known| known.id == sound.id) {
					*known = sound;
				} else if sounds.len() < model::soundboard::MAX_GUILD_SOUNDS
					&& budget <= MAX_CATALOG_BYTES
				{
					sounds.push(sound);
				}
			}
			Event::Removed { guild, sound } => {
				if let Some((_, sounds)) = self
					.soundboard
					.guilds
					.iter_mut()
					.find(|(id, _)| *id == guild)
				{
					sounds.retain(|known| known.id != sound);
				}
			}
			Event::Effect(effect) => {
				// Only the call this device has media for; a deafened call hears nothing.
				if let Some(reason) = self.effect_block(&effect) {
					self.soundboard.drop_effect(effect.sound, reason);
					return;
				}
				let mine = self
					.user
					.as_ref()
					.is_some_and(|user| user.id == effect.user);
				let mut source = Source::Received;
				if mine {
					self.soundboard.prune_own(Some(effect.channel), now);
					// Echoes pair with this account's sends of the same sound oldest first, so
					// repeated sends of one sound each claim their own echo.
					if let Some(index) = self.soundboard.own.iter().position(|own| {
						!own.echoed
							&& own.effect.sound == effect.sound
							&& own.effect.channel == effect.channel
					}) {
						let own = &mut self.soundboard.own[index];
						own.echoed = true;
						let already = std::mem::replace(&mut own.played, true);
						if own.accepted_at.is_some() {
							self.soundboard.own.remove(index);
						}
						if already {
							self.soundboard.drop_effect(effect.sound, "own-echo");
							return;
						}
						// The echo beat the REST reply: play it now, not again on success.
						source = Source::Own;
					}
				}
				// Own echoes pair with sends above, so two quick sends of one sound both count;
				// anything else repeated inside the window is a replay.
				if source == Source::Received
					&& self.soundboard.last_effect.is_some_and(|(last, at)| {
						last == effect && now.saturating_duration_since(at) < DUPLICATE_WINDOW
					}) {
					self.soundboard.drop_effect(effect.sound, "duplicate");
					return;
				}
				self.soundboard.last_effect = Some((effect, now));
				self.soundboard.queue(Playback { effect, source });
			}
			Event::CatalogUnsent(guild) => {
				// Make the refused servers missing again; the next request retries them shortly.
				self.soundboard
					.requested
					.retain(|(id, arrived)| *arrived || guild.is_some_and(|guild| guild != *id));
				self.soundboard.catalog_retry_at = Some(now + CATALOG_RETRY);
			}
			Event::Played { request, result } => {
				// Replies for sends a fresh session abandoned, or already answered, are stale.
				let Some(index) = self
					.soundboard
					.own
					.iter()
					.position(|own| own.request == request && own.accepted_at.is_none())
				else {
					return;
				};
				let own = &mut self.soundboard.own[index];
				let effect = own.effect;
				if result.is_err() {
					self.soundboard.own.remove(index);
				} else {
					own.accepted_at = Some(now);
					// The sender hears a sound Discord accepted, once, even when alone in the call.
					let already = std::mem::replace(&mut own.played, true);
					if own.echoed {
						self.soundboard.own.remove(index);
					}
					if !already {
						match self.effect_block(&effect) {
							Some(reason) => self.soundboard.drop_effect(effect.sound, reason),
							None => self.soundboard.queue(Playback {
								effect,
								source: Source::Own,
							}),
						}
					}
				}
				self.soundboard.error = result.err().map(|failure| match failure {
					Failure::Forbidden => "Discord didn't allow this sound here",
					Failure::RateLimited => "Slow down · wait before playing another sound",
					Failure::Ambiguous => "The sound may not have played",
					Failure::ProtocolAt(reason) | Failure::CapacityAt(reason) => reason,
					_ => "The sound couldn't be played",
				});
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::voice::{Call, Phase};

	/// A synthetic sound with full volume, available, optionally owned by `guild`.
	fn sound(id: u64, guild: Option<u64>) -> Sound {
		Sound {
			id: Id(id),
			name: format!("sound {id}"),
			volume: 1.0,
			emoji_id: None,
			emoji_name: None,
			guild: guild.map(Id),
			available: true,
		}
	}
	/// An authenticated account connected to voice channel 20 of server 10.
	fn connected() -> State {
		let mut state = crate::voice::tests::stream_preview_state();
		state.voice.active = Some(Call {
			channel: Id(20),
			guild: Some(Id(10)),
			connected_at: None,
			channel_started_at: None,
			server_muted: false,
			server_deafened: false,
			request: 1,
			phase: Phase::Connected,
			muted: true,
			deafened: false,
			participants: Vec::new(),
			camera: false,
			watching: None,
			error: None,
			reconnecting: None,
		});
		state
	}
	/// A received effect for sound `sound` from `user` in `channel` of server 10.
	fn effect(channel: u64, user: u64, sound: u64) -> SoundEffect {
		SoundEffect {
			channel: Id(channel),
			guild: Some(Id(10)),
			user: Id(user),
			sound: Id(sound),
			volume: 0.5,
		}
	}

	/// Only the connected, undeafened call plays effects; replays and overflow are dropped.
	#[test]
	fn effects_are_scoped_to_the_connected_call_deduplicated_and_bounded() {
		let mut state = connected();
		let now = Instant::now();
		// Another channel, and an emoji-free duplicate replayed by a RESUME, are ignored.
		state.apply_soundboard_at(Event::Effect(effect(21, 30, 1)), now);
		state.apply_soundboard_at(Event::Effect(effect(20, 30, 1)), now);
		state.apply_soundboard_at(
			Event::Effect(effect(20, 30, 1)),
			now + std::time::Duration::from_millis(100),
		);
		// Two people playing the same sound at once both play.
		state.apply_soundboard_at(Event::Effect(effect(20, 31, 1)), now);
		assert_eq!(state.take_sound_effects().len(), 2);
		state.apply_soundboard_at(
			Event::Effect(effect(20, 30, 1)),
			now + std::time::Duration::from_secs(1),
		);
		assert_eq!(state.take_sound_effects().len(), 1);
		for user in 0..20 {
			state.apply_soundboard_at(Event::Effect(effect(20, 100 + user, 2)), now);
		}
		let kept = state.take_sound_effects();
		assert_eq!(kept.len(), MAX_PENDING_EFFECTS);
		assert_eq!(kept[0].effect.user, Id(112));
		// Deafened, still securing, or a stale call after leaving: nothing is queued.
		state.voice.active.as_mut().unwrap().deafened = true;
		state.apply_soundboard_at(Event::Effect(effect(20, 40, 3)), now);
		state.voice.active.as_mut().unwrap().deafened = false;
		state.voice.active.as_mut().unwrap().phase = Phase::Securing;
		state.apply_soundboard_at(Event::Effect(effect(20, 41, 3)), now);
		state.voice.active = None;
		state.apply_soundboard_at(Event::Effect(effect(20, 42, 3)), now);
		assert!(state.take_sound_effects().is_empty());
	}

	/// Sends stuck behind a rate limit stop blocking the picker once `SEND_TIMEOUT` passes.
	#[test]
	fn stalled_sends_expire_and_free_the_picker() {
		let mut state = connected();
		let default = sound(1, None);
		for _ in 0..MAX_OUTSTANDING_SENDS {
			assert!(state.play_sound(&default).is_some());
		}
		let now = Instant::now();
		assert_eq!(state.soundboard_block(&default), Some("Playing a sound…"));
		let later = now + SEND_TIMEOUT + std::time::Duration::from_secs(1);
		assert_eq!(state.soundboard_block_at(&default, later), None);
		assert_eq!(state.soundboard.outstanding_sends_at(later), 0);
		state.expire_soundboard_sends_at(later);
		assert!(state.soundboard.own.is_empty());
		assert_eq!(
			state.soundboard.error,
			Some("The sound may not have played")
		);
	}

	/// Discord's voice-state and permission requirements, the outstanding-send bound and
	/// `source_guild_id` for another server's sound.
	#[test]
	fn playing_requires_voice_state_permissions_and_names_external_sources() {
		let mut state = connected();
		let local = sound(5, Some(10));
		let external = sound(6, Some(11));
		let default = sound(1, None);
		// Self-mute is allowed by Discord; deafen and server mute are not.
		state.voice.active.as_mut().unwrap().deafened = true;
		assert!(state.play_sound(&local).is_none());
		state.voice.active.as_mut().unwrap().deafened = false;
		state.voice.active.as_mut().unwrap().server_muted = true;
		assert!(state.play_sound(&local).is_none());
		state.voice.active.as_mut().unwrap().server_muted = false;
		// Another server's sound needs Nitro before any request is made.
		assert!(state.play_sound(&external).is_none());
		assert_eq!(
			state.soundboard.error,
			Some("Sounds from other servers require Nitro")
		);
		let Some(CoreCommand::Soundboard(Command::Play {
			channel,
			sound: id,
			source_guild,
			request,
		})) = state.play_sound(&default)
		else {
			panic!("default sound is playable");
		};
		assert_eq!((channel, id, source_guild), (Id(20), Id(1), None));
		// A reply for an unknown request releases nothing.
		state.apply_soundboard(Event::Played {
			request: request + 100,
			result: Ok(()),
		});
		assert!(state.soundboard.is_sending(Id(1)));
		// Sends overlap up to a bound, so a quick second sound is not refused.
		let more: Vec<_> = (1..MAX_OUTSTANDING_SENDS)
			.map(|_| state.play_sound(&default).is_some())
			.collect();
		assert!(more.iter().all(|sent| *sent));
		assert_eq!(state.soundboard_block(&default), Some("Playing a sound…"));
		state.apply_soundboard(Event::Played {
			request,
			result: Err(Failure::RateLimited),
		});
		assert_eq!(
			state.soundboard.outstanding_sends(),
			MAX_OUTSTANDING_SENDS - 1
		);
		assert!(state.soundboard.error.is_some());
		state.soundboard.own.clear();
		state.stickers.external_allowed = true;
		let Some(CoreCommand::Soundboard(Command::Play { source_guild, .. })) =
			state.play_sound(&external)
		else {
			panic!("external sound is playable with Nitro and permission");
		};
		assert_eq!(source_guild, Some(Id(11)));
		state.soundboard.own.clear();
		// Losing ownership drops the implicit Use Soundboard permission.
		state
			.permissions
			.update(crate::permissions::Event::Owner {
				guild: Id(10),
				owner: model::Patch::Value(Id(99)),
			})
			.unwrap();
		assert!(state.play_sound(&local).is_none());
		assert_eq!(state.soundboard.outstanding_sends(), 0);
	}

	/// A preview needs no permission, sends nothing and is never deduplicated.
	#[test]
	fn previews_play_locally_without_a_request_or_send_permission() {
		let mut state = connected();
		// No Nitro and no Use Soundboard: sending is refused, previewing is not.
		state
			.permissions
			.update(crate::permissions::Event::Owner {
				guild: Id(10),
				owner: model::Patch::Value(Id(99)),
			})
			.unwrap();
		let mut external = sound(6, Some(11));
		external.volume = 0.4;
		assert!(state.soundboard_block(&external).is_some());
		assert!(state.preview_sound(&external));
		assert_eq!(state.soundboard.outstanding_sends(), 0);
		let queued = state.take_sound_effects();
		assert_eq!(
			(
				queued.len(),
				queued[0].effect.sound,
				queued[0].effect.user,
				queued[0].effect.volume,
				queued[0].source
			),
			(1, Id(6), Id(1), 0.4, Source::Preview)
		);
		// Repeated previews are not deduplicated like replayed Gateway events.
		assert!(state.preview_sound(&external) && state.preview_sound(&external));
		assert_eq!(state.take_sound_effects().len(), 2);
		state.voice.active.as_mut().unwrap().deafened = true;
		assert!(!state.preview_sound(&external));
		assert!(state.take_sound_effects().is_empty());
	}

	/// Sounds queued for this device since the last call, by source.
	fn played(state: &mut State) -> Vec<(u64, Source)> {
		state
			.take_sound_effects()
			.into_iter()
			.map(|playback| (playback.effect.sound.0, playback.source))
			.collect()
	}
	/// Send default sound `id` and return its request number.
	fn send(state: &mut State, id: u64) -> u64 {
		match state.play_sound(&sound(id, None)) {
			Some(CoreCommand::Soundboard(Command::Play { request, .. })) => request,
			_ => panic!("sound {id} is playable"),
		}
	}
	/// Discord's echo of this account's own sound `id`.
	fn echo(id: u64) -> Event {
		Event::Effect(SoundEffect {
			user: Id(1),
			volume: 1.0,
			..effect(20, 1, id)
		})
	}
	/// A successful REST reply for `request`.
	fn accepted(request: u64) -> Event {
		Event::Played {
			request,
			result: Ok(()),
		}
	}

	/// Regression: one remembered send let a second send's reply overwrite the first, so the
	/// first sound's echo played it again. Covers different and repeated sounds, both reply/echo
	/// orders, a resume in between, expiry and the tracking bound.
	#[test]
	fn overlapping_own_sends_each_play_exactly_once_in_either_order() {
		let mut state = connected();
		let now = Instant::now();
		let later = |ms| now + std::time::Duration::from_millis(ms);
		// Two different sounds whose replies both beat their echoes: neither echo replays.
		let (a, b) = (send(&mut state, 1), send(&mut state, 2));
		state.apply_soundboard_at(accepted(a), later(10));
		state.apply_soundboard_at(accepted(b), later(20));
		assert_eq!(played(&mut state), [(1, Source::Own), (2, Source::Own)]);
		state.apply_soundboard_at(echo(1), later(300));
		state.apply_soundboard_at(echo(2), later(310));
		assert!(played(&mut state).is_empty());
		// The same sound three times with the halves interleaved: every send is heard once.
		let first = send(&mut state, 3);
		let second = send(&mut state, 3);
		let third = send(&mut state, 3);
		state.apply_soundboard_at(echo(3), later(400)); // first's echo beats its reply
		state.apply_soundboard_at(accepted(first), later(410));
		state.apply_soundboard_at(accepted(second), later(420));
		state.apply_soundboard_at(echo(3), later(430)); // second's echo, already heard
		state.apply_soundboard_at(echo(3), later(440)); // third's echo beats its reply
		state.apply_soundboard_at(accepted(third), later(450));
		assert_eq!(played(&mut state), [(3, Source::Own); 3]);
		assert_eq!(state.soundboard.own.len(), 0);
		// Echoes delayed past a Gateway resume still pair with sends answered before it.
		let (c, d) = (send(&mut state, 4), send(&mut state, 5));
		state.soundboard.interrupt(false);
		state.apply_soundboard_at(accepted(d), later(500));
		state.apply_soundboard_at(accepted(c), later(510));
		state.apply_soundboard_at(echo(5), later(2_000));
		state.apply_soundboard_at(echo(4), later(2_100));
		assert_eq!(played(&mut state), [(5, Source::Own), (4, Source::Own)]);
		// Once the echo window passed, the send is forgotten and the list stays bounded.
		let e = send(&mut state, 6);
		state.apply_soundboard_at(accepted(e), later(3_000));
		played(&mut state);
		state.apply_soundboard_at(echo(7), later(3_000) + OWN_ECHO_WINDOW);
		assert!(state.soundboard.own.is_empty());
		for _ in 0..MAX_TRACKED_SENDS * 2 {
			let request = send(&mut state, 8);
			state.apply_soundboard_at(accepted(request), later(4_000));
		}
		assert!(state.soundboard.own.len() <= MAX_TRACKED_SENDS);
	}

	/// Regression: a refused catalog request was never retried, and servers joined after the
	/// first request never got a catalog. Covers per-server requests, retries, updates that
	/// arrive before the catalog, and leaving and rejoining.
	#[test]
	fn catalog_requests_track_each_server_and_survive_a_full_voice_queue() {
		/// The catalog requests among `commands`, by server.
		fn requests(commands: Vec<CoreCommand>) -> Vec<Option<Id>> {
			commands
				.into_iter()
				.filter_map(|command| match command {
					CoreCommand::Voice(crate::voice::Command::RequestSoundboard { guild }) => {
						Some(guild)
					}
					_ => None,
				})
				.collect()
		}
		let mut state = connected();
		let now = Instant::now();
		// The first request of a connection covers every known server at once.
		assert_eq!(requests(state.request_soundboard_at(true, now)), [None]);
		assert!(requests(state.request_soundboard_at(false, now)).is_empty());
		// The voice queue was full: the request is not lost, only delayed.
		state.apply_soundboard_at(Event::CatalogUnsent(None), now);
		assert!(requests(state.request_soundboard_at(false, now)).is_empty());
		let retry = now + CATALOG_RETRY;
		assert_eq!(requests(state.request_soundboard_at(false, retry)), [None]);
		state.apply_soundboard_at(
			Event::Guild {
				guild: Id(10),
				sounds: vec![sound(5, Some(10))],
			},
			retry,
		);
		assert!(requests(state.request_soundboard_at(false, retry)).is_empty());
		// A server joined later is requested on its own, once.
		let mut joined = state.guilds[0].clone();
		joined.id = Id(11);
		state.guilds.push(joined);
		assert_eq!(
			requests(state.request_soundboard_at(false, retry)),
			[Some(Id(11))]
		);
		assert!(requests(state.request_soundboard_at(false, retry)).is_empty());
		state.apply_soundboard_at(Event::CatalogUnsent(Some(Id(11))), retry);
		let again = retry + CATALOG_RETRY;
		assert_eq!(
			requests(state.request_soundboard_at(false, again)),
			[Some(Id(11))]
		);
		// Its sound updates are kept before and after its catalog arrives.
		state.apply_soundboard_at(Event::Upsert(sound(12, Some(11))), again);
		assert_eq!(state.soundboard.guild(Id(11)).unwrap()[0].id, Id(12));
		state.apply_soundboard_at(
			Event::Guild {
				guild: Id(11),
				sounds: vec![sound(12, Some(11)), sound(13, Some(11))],
			},
			again,
		);
		state.apply_soundboard_at(Event::Upsert(sound(14, Some(11))), again);
		assert_eq!(state.soundboard.guild(Id(11)).unwrap().len(), 3);
		assert!(requests(state.request_soundboard_at(false, again)).is_empty());
		// Leaving the server forgets its catalog; rejoining asks for it again.
		let joined = state.guilds.pop().unwrap();
		assert!(requests(state.request_soundboard_at(false, again)).is_empty());
		assert!(state.soundboard.guild(Id(11)).is_none());
		state.guilds.push(joined);
		assert_eq!(
			requests(state.request_soundboard_at(false, again)),
			[Some(Id(11))]
		);
	}

	/// Regression: alone in a channel the sender heard nothing. The reply plays its sound, a
	/// later echo does not, and a refused send stays silent.
	#[test]
	fn own_sounds_play_once_for_the_sender_even_when_alone() {
		// Alone in the channel: DAVE waits for a peer, yet sending and hearing both work.
		let mut state = connected();
		state.voice.active.as_mut().unwrap().phase = Phase::Waiting;
		let now = Instant::now();
		let mine = |sound| SoundEffect {
			user: Id(1),
			volume: 1.0,
			..effect(20, 1, sound)
		};
		let send = |state: &mut State, sound| match state.play_sound(&self::sound(sound, None)) {
			Some(CoreCommand::Soundboard(Command::Play { request, .. })) => request,
			_ => panic!("default sound is playable while alone"),
		};
		// Success first: the sound plays on the reply and its later echo is dropped.
		let request = send(&mut state, 1);
		assert!(state.take_sound_effects().is_empty());
		state.apply_soundboard_at(
			Event::Played {
				request,
				result: Ok(()),
			},
			now,
		);
		let played = state.take_sound_effects();
		assert_eq!(played.len(), 1);
		assert_eq!(played[0].source, Source::Own);
		state.apply_soundboard_at(
			Event::Effect(mine(1)),
			now + std::time::Duration::from_secs(1),
		);
		assert!(state.take_sound_effects().is_empty());
		// Echo first: it plays immediately and the reply adds nothing.
		let request = send(&mut state, 2);
		state.apply_soundboard_at(Event::Effect(mine(2)), now);
		assert_eq!(state.take_sound_effects()[0].source, Source::Own);
		state.apply_soundboard_at(
			Event::Played {
				request,
				result: Ok(()),
			},
			now,
		);
		assert!(state.take_sound_effects().is_empty());
		// A refused send is never heard; the same sound from another device still plays.
		let request = send(&mut state, 3);
		state.apply_soundboard_at(
			Event::Played {
				request,
				result: Err(Failure::Forbidden),
			},
			now,
		);
		assert!(state.take_sound_effects().is_empty());
		state.apply_soundboard_at(
			Event::Effect(mine(3)),
			now + std::time::Duration::from_secs(1),
		);
		assert_eq!(state.take_sound_effects()[0].source, Source::Received);
		assert!(
			state
				.take_dropped_sound_effects()
				.iter()
				.any(|(_, reason)| *reason == "own-echo")
		);
	}

	/// Private and group calls have no server, so every server sound is sent as external.
	#[test]
	fn private_and_group_calls_play_defaults_and_send_server_sounds_as_external() {
		for kind in [1, 3] {
			let mut state = crate::voice::tests::dm_state();
			state.channels[0].kind = kind;
			state.voice.active = Some(Call {
				guild: None,
				channel: Id(2),
				..connected().voice.active.unwrap()
			});
			let Some(CoreCommand::Soundboard(Command::Play {
				channel,
				source_guild,
				request,
				..
			})) = state.play_sound(&sound(1, None))
			else {
				panic!("default sounds play in private calls");
			};
			assert_eq!((channel, source_guild), (Id(2), None));
			state.apply_soundboard(Event::Played {
				request,
				result: Ok(()),
			});
			assert_eq!(state.take_sound_effects()[0].source, Source::Own);
			// No server owns a private call, so every server sound is external.
			assert!(state.play_sound(&sound(5, Some(10))).is_none());
			assert_eq!(
				state.soundboard.error,
				Some("Sounds from other servers require Nitro")
			);
			state.stickers.external_allowed = true;
			let Some(CoreCommand::Soundboard(Command::Play { source_guild, .. })) =
				state.play_sound(&sound(5, Some(10)))
			else {
				panic!("server sounds play in private calls with Nitro");
			};
			assert_eq!(source_guild, Some(Id(10)));
			// Received effects in the private call play too; they carry no guild.
			state.apply_soundboard(Event::Effect(SoundEffect {
				guild: None,
				..effect(2, 3, 1)
			}));
			assert_eq!(state.take_sound_effects().len(), 1);
		}
	}

	/// Catalog loading, updates and budgets, and which state a Gateway resume or a fresh session
	/// keeps: sends in flight survive a resume and still play.
	#[test]
	fn catalogs_follow_gateway_updates_within_budgets() {
		let mut state = connected();
		let commands = state.request_soundboard(true);
		assert_eq!(commands.len(), 2);
		assert!(state.request_soundboard(true).is_empty());
		// A failure waits for the next explicit open instead of retrying every frame.
		state.apply_soundboard(Event::Defaults(Err(Failure::Network)));
		assert!(state.request_soundboard(false).is_empty());
		assert_eq!(state.request_soundboard(true).len(), 1);
		state.apply_soundboard(Event::Defaults(Ok(vec![sound(1, None)])));
		assert_eq!(state.soundboard.defaults_load, Load::Loaded);
		// Unknown servers are not retained.
		state.apply_soundboard(Event::Guild {
			guild: Id(99),
			sounds: vec![sound(9, Some(99))],
		});
		state.apply_soundboard(Event::Upsert(sound(9, Some(99))));
		assert!(state.soundboard.guilds.is_empty());
		// A known server's sound arriving before its catalog starts one; the reply replaces it.
		state.apply_soundboard(Event::Upsert(sound(7, Some(10))));
		assert_eq!(state.soundboard.guild(Id(10)).unwrap().len(), 1);
		state.apply_soundboard(Event::Guild {
			guild: Id(10),
			sounds: vec![sound(5, Some(10))],
		});
		let mut renamed = sound(5, Some(10));
		renamed.name = "renamed".into();
		state.apply_soundboard(Event::Upsert(renamed));
		state.apply_soundboard(Event::Upsert(sound(7, Some(10))));
		assert_eq!(
			state
				.soundboard
				.guild(Id(10))
				.unwrap()
				.iter()
				.map(|s| s.name.as_str())
				.collect::<Vec<_>>(),
			["renamed", "sound 7"]
		);
		state.apply_soundboard(Event::Removed {
			guild: Id(10),
			sound: Id(5),
		});
		assert_eq!(state.soundboard.guild(Id(10)).unwrap().len(), 1);
		// A replacement over budget keeps the previous catalog.
		let oversized = (0..model::soundboard::MAX_GUILD_SOUNDS as u64)
			.map(|id| Sound {
				name: "x".repeat(40_000),
				..sound(1000 + id, Some(10))
			})
			.collect();
		state.apply_soundboard(Event::Guild {
			guild: Id(10),
			sounds: oversized,
		});
		assert_eq!(state.soundboard.guild(Id(10)).unwrap().len(), 1);
		// A Gateway-only interruption keeps a send in flight: its REST reply still plays it,
		// and answered catalogs are not asked for again.
		let Some(CoreCommand::Soundboard(Command::Play { request, .. })) =
			state.play_sound(&sound(1, None))
		else {
			panic!("default sound is playable");
		};
		state.soundboard.interrupt(false);
		assert!(state.soundboard.is_sending(Id(1)));
		assert_eq!(state.soundboard.error, None);
		state.apply_soundboard(Event::Played {
			request,
			result: Ok(()),
		});
		assert_eq!(state.take_sound_effects()[0].source, Source::Own);
		assert!(state.request_soundboard(false).is_empty());
		// A fresh session abandons sends, forgets server catalogs and asks again.
		let Some(CoreCommand::Soundboard(Command::Play { request, .. })) =
			state.play_sound(&sound(1, None))
		else {
			panic!("default sound is playable");
		};
		state.soundboard.interrupt(true);
		assert_eq!(state.soundboard.outstanding_sends(), 0);
		assert!(state.soundboard.error.is_some());
		state.apply_soundboard(Event::Played {
			request,
			result: Ok(()),
		});
		assert!(state.take_sound_effects().is_empty());
		assert!(state.soundboard.guilds.is_empty());
		assert_eq!(state.request_soundboard(false).len(), 1);
	}
}
