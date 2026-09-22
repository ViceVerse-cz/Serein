//! One explicitly joined DM or guild voice channel; bounded ephemeral participant state.
use crate::{
	State as ClientState,
	auth::{AuthState, Failure},
	screen,
};
use model::{Id, Member};
use std::time::Instant;
pub const MAX_PARTICIPANTS: usize = 64;
pub const MAX_DM_CALLS: usize = 64;
pub const MAX_DM_CALL_BYTES: usize = MAX_DM_CALLS * size_of::<Id>();
pub const MAX_ROSTER: usize = 4096;
pub const MAX_ROSTER_BYTES: usize = 1024 * 1024;
use std::fmt;
use zeroize::Zeroizing;

pub struct Secret(Zeroizing<String>);
impl Secret {
	pub fn new(value: String) -> Result<Self, Failure> {
		let value = Zeroizing::new(value);
		if value.is_empty() || value.len() > 2048 || !value.bytes().all(|b| b.is_ascii_graphic()) {
			return Err(Failure::Protocol);
		}
		Ok(Self(value))
	}
	pub fn expose(&self) -> &str {
		&self.0
	}
	pub(crate) fn bytes(&self) -> usize {
		self.0.capacity()
	}
}
impl fmt::Debug for Secret {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str("VoiceSecret([REDACTED])")
	}
}
pub struct VoiceConnection {
	pub channel: Id,
	pub guild: Option<Id>,
	pub user: Id,
	/// Expected peer for a one-to-one DM; groups use the authenticated voice roster.
	pub peer: Option<Id>,
	pub session: Secret,
	pub token: Secret,
	pub endpoint: String,
	pub request: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
	Connecting,
	ConnectingTransport,
	Discovering,
	OpeningAudio,
	Ringing,
	Securing,
	Connected,
	Waiting,
	Failed,
}
impl Phase {
	pub fn label(self) -> &'static str {
		match self {
			Self::ConnectingTransport => "Connecting to voice server...",
			Self::Discovering => "Checking voice network...",
			Self::OpeningAudio => "Opening audio devices...",
			Self::Connecting => "Connecting call…",
			Self::Ringing => "Ringing…",
			Self::Securing => "Securing audio…",
			Self::Waiting => "Connected · waiting for others",
			Self::Connected => "Voice connected",
			Self::Failed => "Call failed",
		}
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Participant {
	pub user: Id,
	pub muted: bool,
	pub deafened: bool,
	pub server_muted: bool,
	pub server_deafened: bool,
	/// Camera on (`self_video`); frames arrive on the call transport.
	pub video: bool,
	/// Go Live screen share announced (`self_stream`); viewing needs an explicit watch.
	pub streaming: bool,
}
#[derive(Clone)]
pub struct RosterEntry {
	pub guild: Id,
	pub channel: Id,
	pub participant: Participant,
	pub member: Option<Member>,
}
impl RosterEntry {
	pub fn bytes(&self) -> usize {
		size_of::<Self>() + self.member.as_ref().map_or(0, Member::bytes)
	}
}
pub struct Call {
	pub channel: Id,
	pub guild: Option<Id>,
	pub connected_at: Option<Instant>,
	pub server_muted: bool,
	pub server_deafened: bool,
	pub request: u64,
	pub phase: Phase,
	pub muted: bool,
	pub deafened: bool,
	pub participants: Vec<Participant>,
	pub camera: bool,
	/// Participant whose screen share this device explicitly chose to watch.
	pub watching: Option<Id>,
	pub error: Option<&'static str>,
}
#[derive(Default)]
pub struct State {
	pub active: Option<Call>,
	/// Last service-confirmed departure, scoped to its local request.
	pub departed: Option<(Id, u64)>,
	pub incoming: Option<Id>,
	/// One explicit outgoing attempt, confirmed only by service ringing recipients.
	outgoing: Option<(Id, u64, bool)>,
	/// Known service calls, independent of ringing and this device's media session.
	pub(crate) dm_calls: Vec<Id>,
	/// Last reported members of each known DM call, so answering or joining shows them at once.
	pub(crate) dm_participants: Vec<(Id, Vec<Participant>)>,
	pub roster: Vec<RosterEntry>,
	sequence: u64,
}
impl State {
	pub fn has_dm_call(&self, channel: Id) -> bool {
		self.dm_calls.contains(&channel)
	}
}
#[derive(Clone, Copy, Debug)]
pub enum Command {
	Sync {
		channel: Id,
	},
	Ring {
		channel: Id,
		request: u64,
	},
	Join {
		channel: Id,
		request: u64,
		ring: bool,
		mute: bool,
		deaf: bool,
	},
	Leave {
		channel: Id,
		request: u64,
	},
	SetMute {
		channel: Id,
		request: u64,
		mute: bool,
		deaf: bool,
	},
	StartStream {
		channel: Id,
		request: u64,
		stream_request: u64,
	},
	StopStream {
		channel: Id,
		request: u64,
		stream_request: u64,
	},
	SetCamera {
		channel: Id,
		request: u64,
		enabled: bool,
	},
	WatchStream {
		channel: Id,
		request: u64,
		stream_request: u64,
		streamer: Id,
	},
	StopWatching {
		channel: Id,
		request: u64,
		stream_request: u64,
	},
	Decline {
		channel: Id,
	},
}
pub enum Event {
	Departed {
		channel: Id,
		request: u64,
	},
	Snapshot {
		partial: bool,
		guild: Option<Id>,
		participants: Vec<RosterEntry>,
	},
	Call {
		channel: Id,
		ringing: Option<Vec<Id>>,
		participants: Option<Vec<Participant>>,
		unavailable: bool,
	},
	Deleted {
		channel: Id,
	},
	State {
		guild: Option<Id>,
		member: Option<Box<Member>>,
		server_muted: bool,
		server_deafened: bool,
		request: Option<u64>,
		channel: Option<Id>,
		user: Id,
		session: Option<Secret>,
		muted: bool,
		deafened: bool,
		video: bool,
		streaming: bool,
	},
	Server {
		request: u64,
		channel: Id,
		token: Option<Secret>,
		endpoint: Option<String>,
	},
	Progress {
		channel: Id,
		request: u64,
		phase: Phase,
	},
	Failed {
		channel: Id,
		request: u64,
		message: &'static str,
	},
	Stream {
		channel: Id,
		request: u64,
		stream_request: u64,
		event: screen::Event,
	},
	/// Negotiation for watching another participant's screen share.
	Watch {
		channel: Id,
		request: u64,
		stream_request: u64,
		streamer: Id,
		event: screen::Event,
	},
}
impl Event {
	pub fn bytes(&self) -> usize {
		match self {
			Self::Snapshot { participants, .. } => {
				participants.capacity() * size_of::<RosterEntry>()
					+ participants.iter().map(RosterEntry::bytes).sum::<usize>()
			}
			Self::Call {
				ringing,
				participants,
				..
			} => {
				ringing
					.as_ref()
					.map_or(0, |v| v.capacity() * size_of::<Id>())
					+ participants
						.as_ref()
						.map_or(0, |v| v.capacity() * size_of::<Participant>())
			}
			Self::State {
				session, member, ..
			} => {
				session.as_ref().map_or(0, Secret::bytes)
					+ member.as_deref().map_or(0, Member::bytes)
			}
			Self::Server {
				token, endpoint, ..
			} => {
				token.as_ref().map_or(0, Secret::bytes)
					+ endpoint.as_ref().map_or(0, String::capacity)
			}
			Self::Stream { event, .. } | Self::Watch { event, .. } => event.bytes(),
			_ => 0,
		}
	}
}
impl ClientState {
	pub fn outgoing_ring(&mut self) -> Option<Id> {
		let (channel, request, confirmed) = self.voice.outgoing?;
		if !self.can_call(channel)
			|| self.voice.active.as_ref().is_none_or(|call| {
				call.channel != channel
					|| call.request != request
					|| matches!(call.phase, Phase::Connected | Phase::Failed)
			}) {
			self.voice.outgoing = None;
			return None;
		}
		confirmed.then_some(channel)
	}
	pub fn can_call(&self, channel: Id) -> bool {
		!self.demo
			&& self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.has_voice_access(channel)
	}
	pub(crate) fn has_voice_access(&self, channel: Id) -> bool {
		self.permission(
			channel,
			model::permissions::VIEW_CHANNEL | model::permissions::CONNECT,
		) == Some(true)
			&& self.channels.iter().any(|c| {
				c.id == channel
					&& ((c.guild.is_some() && c.kind == 2)
						|| (c.guild.is_none()
							&& ((c.kind == 1 && c.recipients.len() == 1)
								|| (c.kind == 3 && c.recipients.len() < MAX_PARTICIPANTS))))
			})
	}
	fn dm_call_participant(&self, channel: Id, user: Id) -> bool {
		self.channel(channel).is_some_and(|c| {
			c.guild.is_none()
				&& matches!(c.kind, 1 | 3)
				&& (self.user.as_ref().is_some_and(|own| own.id == user)
					|| c.recipients.iter().any(|recipient| recipient.id == user))
		})
	}
	pub fn can_camera(&self, channel: Id) -> bool {
		self.can_call(channel) && self.permission(channel, model::permissions::STREAM) == Some(true)
	}
	pub fn start_call(&mut self, channel: Id, ring: bool) -> Option<crate::Command> {
		self.start_call_with_mute(channel, ring, false, false)
	}
	pub fn start_call_with_mute(
		&mut self,
		channel: Id,
		ring: bool,
		muted: bool,
		deafened: bool,
	) -> Option<crate::Command> {
		if !self.can_call(channel) || self.voice.active.is_some() {
			return None;
		}
		let guild = self.channel(channel)?.guild;
		let ring = ring && guild.is_none() && !self.voice.has_dm_call(channel);
		let participants: Vec<_> = if guild.is_some() {
			self.voice
				.roster
				.iter()
				.filter(|r| r.channel == channel)
				.map(|r| r.participant)
				.collect()
		} else {
			self.voice
				.dm_participants
				.iter()
				.find(|(id, _)| *id == channel)
				.map(|(_, participants)| participants.clone())
				.unwrap_or_default()
		};
		if participants.len() >= MAX_PARTICIPANTS {
			self.status = "Voice channel exceeds the 64 participant limit";
			return None;
		}
		let own = participants
			.iter()
			.find(|p| self.user.as_ref().is_some_and(|u| u.id == p.user));
		let server_muted = own.is_some_and(|p| p.server_muted);
		let server_deafened = own.is_some_and(|p| p.server_deafened);
		let muted = muted || !self.can_speak(channel);
		self.voice.sequence = self.voice.sequence.wrapping_add(1);
		let request = self.voice.sequence;
		self.voice.outgoing = ring.then_some((channel, request, false));
		self.voice.active = Some(Call {
			channel,
			guild,
			connected_at: None,
			server_muted,
			server_deafened,
			request,
			phase: Phase::Connecting,
			muted,
			deafened,
			camera: false,
			watching: None,
			participants,
			error: None,
		});
		if self.voice.incoming == Some(channel) {
			self.voice.incoming = None;
		}
		Some(crate::Command::Voice(Command::Join {
			channel,
			request,
			ring,
			mute: muted || deafened,
			deaf: deafened,
		}))
	}
	pub fn leave_call(&mut self) -> Option<crate::Command> {
		self.voice.outgoing = None;
		let call = self.voice.active.take()?;
		self.voice.departed = None;
		Some(crate::Command::Voice(Command::Leave {
			channel: call.channel,
			request: call.request,
		}))
	}
	pub fn set_call_mute(&mut self, muted: bool, deafened: bool) -> Option<crate::Command> {
		let channel = self.voice.active.as_ref()?.channel;
		if !self.demo && !self.can_call(channel) {
			return None;
		}
		let muted = muted || (!self.demo && !self.can_speak(channel));
		let call = self.voice.active.as_mut()?;
		if call.phase == Phase::Failed {
			return None;
		}
		call.muted = muted;
		call.deafened = deafened;
		Some(crate::Command::Voice(Command::SetMute {
			channel: call.channel,
			request: call.request,
			mute: muted || deafened,
			deaf: deafened,
		}))
	}
	pub fn decline_call(&mut self) -> Option<crate::Command> {
		Some(crate::Command::Voice(Command::Decline {
			channel: self.voice.incoming.take()?,
		}))
	}
	pub fn set_call_camera(&mut self, enabled: bool) -> Option<crate::Command> {
		let call = self.voice.active.as_ref()?;
		if call.camera == enabled
			|| (enabled
				&& (!matches!(call.phase, Phase::Connected | Phase::Waiting)
					|| !self.can_camera(call.channel)))
		{
			return None;
		}
		let call = self.voice.active.as_mut()?;
		call.camera = enabled;
		Some(crate::Command::Voice(Command::SetCamera {
			channel: call.channel,
			request: call.request,
			enabled,
		}))
	}
	/// Choose one streaming participant of the connected call to watch. Nothing connects
	/// until the desktop negotiates the stream; `None` when the choice is not possible.
	pub fn watch_stream(&mut self, streamer: Id) -> Option<()> {
		let own = self.user.as_ref().map(|user| user.id);
		let call = self.voice.active.as_mut()?;
		if !matches!(call.phase, Phase::Connected | Phase::Waiting)
			|| call.watching == Some(streamer)
			|| own == Some(streamer)
			|| !call
				.participants
				.iter()
				.any(|p| p.user == streamer && p.streaming)
		{
			return None;
		}
		call.watching = Some(streamer);
		Some(())
	}
	pub fn stop_watching(&mut self) {
		if let Some(call) = &mut self.voice.active {
			call.watching = None;
		}
	}
	pub fn apply_voice(&mut self, event: Event) {
		match event {
			Event::Departed { channel, request } => {
				self.voice.departed = Some((channel, request));
			}
			Event::Snapshot {
				partial,
				guild,
				participants,
			} => {
				if !partial {
					self.voice
						.roster
						.retain(|r| Some(r.guild) != guild && guild.is_some());
				}
				for entry in participants {
					if guild.is_some_and(|guild| entry.guild != guild) {
						continue;
					}
					if !self.update_roster(entry) {
						break;
					}
				}
				self.refresh_voice_participants();
			}
			Event::Call {
				channel,
				ringing,
				participants,
				unavailable,
			} => {
				if self.auth != AuthState::Authenticated
					|| !self.has_voice_access(channel)
					|| self.channel(channel).is_none_or(|c| c.guild.is_some())
				{
					return;
				}
				if ringing.as_ref().is_some_and(|r| {
					r.len() > MAX_PARTICIPANTS
						|| r.iter()
							.any(|user| !self.dm_call_participant(channel, *user))
				}) || participants.as_ref().is_some_and(|p| {
					p.len() > MAX_PARTICIPANTS
						|| p.iter().enumerate().any(|(index, participant)| {
							!self.dm_call_participant(channel, participant.user)
								|| p[..index]
									.iter()
									.any(|other| other.user == participant.user)
						})
				}) {
					return;
				}
				if unavailable {
					self.end_voice_channel(channel);
					return;
				}
				if !self.voice.has_dm_call(channel) {
					if self.voice.dm_calls.len() >= MAX_DM_CALLS
						|| (self.voice.dm_calls.len() + 1) * size_of::<Id>() > MAX_DM_CALL_BYTES
					{
						// ponytail: oldest call metadata is evicted; viewing that DM queries it again.
						let evicted = self.voice.dm_calls.remove(0);
						self.voice.dm_participants.retain(|(id, _)| *id != evicted);
					}
					self.voice.dm_calls.push(channel);
				}
				if let Some(ringing) = ringing {
					// Service call updates have no request ID; only a current explicit
					// outgoing attempt for this channel may consume their ringing state.
					if let Some((outgoing_channel, request, confirmed)) = self.voice.outgoing
						&& outgoing_channel == channel
						&& self
							.voice
							.active
							.as_ref()
							.is_some_and(|call| call.channel == channel && call.request == request)
					{
						let peer_ringing = self
							.user
							.as_ref()
							.is_some_and(|own| ringing.iter().any(|user| *user != own.id));
						// CALL_CREATE can be empty before the explicit REST ring is sent.
						self.voice.outgoing = if peer_ringing || !confirmed {
							Some((channel, request, peer_ringing))
						} else {
							None
						};
					}
					if self.user.as_ref().is_some_and(|u| ringing.contains(&u.id))
						&& self
							.voice
							.active
							.as_ref()
							.is_none_or(|c| c.channel != channel)
					{
						if self.voice.incoming.is_none() {
							self.voice.incoming = Some(channel);
						}
					} else if self.voice.incoming == Some(channel) {
						self.voice.incoming = None;
					}
				}
				if let Some(mut participants) = participants {
					participants.shrink_to_fit();
					if let Some(call) = &mut self.voice.active
						&& call.channel == channel
					{
						if self.user.as_ref().is_some_and(|own| {
							participants
								.iter()
								.any(|participant| participant.user != own.id)
						}) {
							self.voice.outgoing = None;
						}
						call.participants = participants.clone();
						if call.watching.is_some_and(|user| {
							!participants.iter().any(|p| p.user == user && p.streaming)
						}) {
							call.watching = None;
						}
					}
					self.remember_dm_participants(channel, participants);
				}
			}
			Event::Deleted { channel } => self.end_voice_channel(channel),
			Event::State {
				guild,
				member,
				server_muted,
				server_deafened,
				request,
				channel,
				user,
				muted,
				deafened,
				video,
				streaming,
				..
			} => {
				let participant = Participant {
					user,
					muted,
					deafened,
					server_muted,
					server_deafened,
					video,
					streaming,
				};
				if let Some(guild) = guild {
					let previous = self
						.voice
						.roster
						.iter()
						.find(|r| r.guild == guild && r.participant.user == user)
						.and_then(|r| r.member.clone());
					self.voice
						.roster
						.retain(|r| r.guild != guild || r.participant.user != user);
					if let Some(channel) = channel {
						self.update_roster(RosterEntry {
							guild,
							channel,
							participant,
							member: member.map(|member| *member).or(previous),
						});
					}
				}
				if guild.is_none() {
					if channel.is_some_and(|channel| !self.dm_call_participant(channel, user)) {
						return;
					}
					// DM voice states arrive whether or not this device has joined; keep the
					// known call membership current so a later join shows everyone.
					for (id, participants) in &mut self.voice.dm_participants {
						participants.retain(|p| p.user != user);
						if channel == Some(*id) && participants.len() < MAX_PARTICIPANTS {
							participants.push(participant);
						}
					}
					if let Some(channel) = channel
						&& self.voice.has_dm_call(channel)
						&& !self
							.voice
							.dm_participants
							.iter()
							.any(|(id, _)| *id == channel)
					{
						self.remember_dm_participants(channel, vec![participant]);
					}
				}
				let Some(call) = &mut self.voice.active else {
					return;
				};
				if self.user.as_ref().is_some_and(|u| u.id == user) {
					if request != Some(call.request) {
						return;
					}
					if channel != Some(call.channel) || guild != call.guild {
						self.voice.outgoing = None;
						if call.phase != Phase::Failed {
							self.voice.active = None;
						}
						return;
					}
				}
				if guild != call.guild {
					return;
				}
				if self.user.as_ref().is_some_and(|u| u.id == user) {
					call.server_muted = server_muted;
					call.server_deafened = server_deafened;
				}
				call.participants.retain(|p| p.user != user);
				if channel == Some(call.channel) {
					if self.user.as_ref().is_some_and(|own| own.id != user) {
						self.voice.outgoing = None;
					}
					if call.participants.len() >= MAX_PARTICIPANTS {
						self.disconnect_voice("Voice channel exceeds the 64 participant limit");
						self.status = "Voice channel exceeds the 64 participant limit";
						return;
					}
					call.participants.push(participant);
				}
				if call.watching == Some(user) && (channel != Some(call.channel) || !streaming) {
					call.watching = None;
				}
			}
			Event::Progress {
				channel,
				request,
				phase,
			} => {
				if let Some(call) = &mut self.voice.active
					&& call.channel == channel
					&& call.request == request
					&& call.phase != Phase::Failed
				{
					call.phase = phase;
					if matches!(phase, Phase::Connected | Phase::Failed) {
						self.voice.outgoing = None;
					}
					if matches!(phase, Phase::Connected | Phase::Waiting)
						&& call.connected_at.is_none()
					{
						call.connected_at = Some(Instant::now());
					}
				}
			}
			Event::Failed {
				channel,
				request,
				message,
			} => {
				if request == 0 || self.voice.active.is_none() {
					self.status = message;
				}
				if let Some(call) = &mut self.voice.active
					&& call.channel == channel
					&& call.request == request
				{
					self.voice.outgoing = None;
					call.phase = Phase::Failed;
					call.camera = false;
					call.watching = None;
					call.error.get_or_insert(message);
					call.participants.clear();
				}
			}
			Event::Server { .. } | Event::Stream { .. } | Event::Watch { .. } => {} // The desktop consumes negotiation material; core never retains it.
		}
	}
	fn update_roster(&mut self, entry: RosterEntry) -> bool {
		if !self.can_view(entry.channel)
			|| !self
				.channels
				.iter()
				.any(|c| c.id == entry.channel && c.guild == Some(entry.guild) && c.kind == 2)
		{
			return true;
		}
		self.voice
			.roster
			.retain(|r| r.guild != entry.guild || r.participant.user != entry.participant.user);
		if self.voice.roster.len() >= MAX_ROSTER
			|| self
				.voice
				.roster
				.iter()
				.map(RosterEntry::bytes)
				.sum::<usize>()
				+ entry.bytes()
				> MAX_ROSTER_BYTES
		{
			self.disconnect_voice("Voice roster exceeds safe capacity; reconnect to refresh");
			self.status = "Voice roster exceeds safe capacity; reconnect to refresh";
			return false;
		}
		self.voice.roster.push(entry);
		true
	}
	fn refresh_voice_participants(&mut self) {
		if let Some(call) = &mut self.voice.active
			&& call.guild.is_some()
		{
			call.participants = self
				.voice
				.roster
				.iter()
				.filter(|r| r.channel == call.channel)
				.map(|r| r.participant)
				.collect();
			if let Some(own) = call
				.participants
				.iter()
				.find(|p| self.user.as_ref().is_some_and(|u| u.id == p.user))
			{
				call.server_muted = own.server_muted;
				call.server_deafened = own.server_deafened;
			}
			if call
				.watching
				.is_some_and(|w| !call.participants.iter().any(|p| p.user == w && p.streaming))
			{
				call.watching = None;
			}
			if call.participants.len() > MAX_PARTICIPANTS {
				self.disconnect_voice("Voice channel exceeds the 64 participant limit");
			}
		}
	}
	fn remember_dm_participants(&mut self, channel: Id, participants: Vec<Participant>) {
		if participants.len() > MAX_PARTICIPANTS || !self.voice.has_dm_call(channel) {
			return;
		}
		self.voice.dm_participants.retain(|(id, _)| *id != channel);
		while self.voice.dm_participants.len() >= MAX_DM_CALLS {
			self.voice.dm_participants.remove(0);
		}
		// Fixed capacity bounds every retained private-call roster by both items and bytes.
		let mut retained = Vec::with_capacity(MAX_PARTICIPANTS);
		retained.extend(participants);
		self.voice.dm_participants.push((channel, retained));
	}
	pub(crate) fn end_voice_channel(&mut self, channel: Id) {
		if self.voice.outgoing.is_some_and(|(id, _, _)| id == channel) {
			self.voice.outgoing = None;
		}
		self.voice.dm_calls.retain(|id| *id != channel);
		self.voice.dm_participants.retain(|(id, _)| *id != channel);
		self.voice.roster.retain(|r| r.channel != channel);
		if self.voice.incoming == Some(channel) {
			self.voice.incoming = None;
		}
		if self
			.voice
			.active
			.as_ref()
			.is_some_and(|c| c.channel == channel && c.phase != Phase::Failed)
		{
			self.voice.active = None;
		}
	}
	pub fn disconnect_voice(&mut self, reason: &'static str) {
		self.voice.outgoing = None;
		self.voice.dm_calls.clear();
		self.voice.dm_participants.clear();
		self.voice.roster.clear();
		self.voice.incoming = None;
		if let Some(call) = &mut self.voice.active {
			call.phase = Phase::Failed;
			call.camera = false;
			call.watching = None;
			call.error.get_or_insert(reason);
			call.participants.clear();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event as CoreEvent};
	use model::{Channel, User};
	#[test]
	fn guild_roster_moves_mutes_limits_and_selection_never_join_implicitly() {
		let mut state = ClientState {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(User {
				primary_guild: None,
				id: Id(1),
				name: "Owner".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				stickers: None,
				id: Id(10),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			channels: [20, 21]
				.into_iter()
				.map(|id| Channel {
					id: Id(id),
					guild: Some(Id(10)),
					kind: 2,
					name: "Room".into(),
					last_message: None,
					parent_id: None,
					position: 0,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
				})
				.collect(),
			..ClientState::default()
		};
		crate::tests::grant_permissions(&mut state);
		let entry = |user, channel| RosterEntry {
			guild: Id(10),
			channel: Id(channel),
			participant: Participant {
				user: Id(user),
				muted: true,
				deafened: false,
				server_muted: true,
				server_deafened: false,
				video: false,
				streaming: false,
			},
			member: None,
		};
		state.apply_voice(Event::Snapshot {
			guild: None,
			partial: false,
			participants: vec![entry(2, 20)],
		});
		assert!(state.voice.active.is_none());
		assert!(matches!(
			state.select(Id(20)),
			Some(crate::Command::History {
				channel: Id(20),
				..
			})
		));
		assert_eq!(state.selected, Some(Id(20)));
		assert!(state.history_pending);
		assert!(state.voice.active.is_none());
		state.apply_voice(Event::Snapshot {
			guild: None,
			partial: true,
			participants: vec![entry(3, 21)],
		});
		assert_eq!(state.voice.roster.len(), 2);
		assert!(matches!(
			state.start_call(Id(20), true),
			Some(crate::Command::Voice(Command::Join { ring: false, .. }))
		));
		let call = state.voice.active.as_ref().unwrap();
		let request = call.request;
		assert_eq!(call.guild, Some(Id(10)));
		assert!(call.connected_at.is_none());
		assert_eq!(call.participants.len(), 1);
		state.apply_voice(Event::Progress {
			channel: Id(20),
			request,
			phase: Phase::Waiting,
		});
		let connected_at = state.voice.active.as_ref().unwrap().connected_at;
		assert!(connected_at.is_some());
		state.apply_voice(Event::Progress {
			channel: Id(20),
			request,
			phase: Phase::Connected,
		});
		assert_eq!(
			state.voice.active.as_ref().unwrap().connected_at,
			connected_at
		);
		state.apply_voice(Event::State {
			guild: Some(Id(10)),
			channel: Some(Id(21)),
			user: Id(2),
			request: None,
			session: None,
			member: None,
			muted: false,
			deafened: true,
			server_muted: false,
			server_deafened: true,
			video: false,
			streaming: false,
		});
		assert!(state.voice.active.as_ref().unwrap().participants.is_empty());
		assert_eq!(
			state
				.voice
				.roster
				.iter()
				.find(|r| r.participant.user == Id(2))
				.unwrap()
				.channel,
			Id(21)
		);
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Disconnected,
		});
		assert_eq!(state.voice.roster.len(), 2);
		assert_eq!(state.voice.active.as_ref().unwrap().phase, Phase::Connected);
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::PermissionsChanged,
		});
		assert!(state.voice.roster.is_empty());
		crate::tests::grant_permissions(&mut state);
		let mut oversized = entry(2, 20);
		oversized.member = Some(Member {
			roles: vec![],
			user: User {
				primary_guild: None,
				id: Id(2),
				name: "x".repeat(MAX_ROSTER_BYTES),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			nick: None,
			status: None,
			custom_status: None,
			activities: vec![],
		});
		state.apply_voice(Event::Snapshot {
			guild: None,
			partial: false,
			participants: vec![oversized],
		});
		assert!(state.voice.roster.is_empty());
		assert!(state.status.contains("capacity"));
		state.apply_voice(Event::Snapshot {
			guild: None,
			partial: false,
			participants: (1..=MAX_PARTICIPANTS as u64)
				.map(|user| entry(user, 20))
				.collect(),
		});
		state.leave_call();
		state.gateway_connected = true;
		assert!(state.start_call(Id(20), false).is_none());
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Unavailable(Id(20)),
		});
		assert!(state.voice.roster.is_empty());

		use model::permissions as p;
		state
			.permissions
			.replace(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(10),
					owner: Some(Id(999)),
					roles: Some(vec![p::Role {
						name: String::new(),
						color: 0,
						position: 0,
						hoist: false,
						id: Id(10),
						bits: p::VIEW_CHANNEL | p::CONNECT,
					}]),
					member: Some(p::Member {
						roles: vec![],
						timeout_until: None,
					}),
				}],
				channels: vec![p::Channel {
					id: Id(21),
					guild: Id(10),
					overwrites: Some(vec![]),
				}],
			})
			.unwrap();
		assert!(
			state.start_call(Id(21), false).is_some(),
			"CONNECT permits a listen-only call"
		);
		assert!(state.voice.active.as_ref().unwrap().muted);
		state.voice.active.as_mut().unwrap().phase = Phase::Connected;
		assert!(!state.can_camera(Id(21)));
		assert!(state.set_call_camera(true).is_none());
		assert!(matches!(
			state.set_call_mute(false, false),
			Some(crate::Command::Voice(Command::SetMute { mute: true, .. }))
		));
		state
			.permissions
			.guilds
			.get_mut(&Id(10))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits |= p::SPEAK | p::STREAM;
		state.permissions.clear_cache();
		assert!(state.set_call_camera(true).is_some());
		assert!(matches!(
			state.set_call_mute(false, false),
			Some(crate::Command::Voice(Command::SetMute { mute: false, .. }))
		));
		state
			.permissions
			.guilds
			.get_mut(&Id(10))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits &= !p::CONNECT;
		state.permissions.clear_cache();
		assert!(!state.can_call(Id(21)));
		assert!(state.set_call_camera(false).is_some());
		assert!(!state.voice.active.as_ref().unwrap().camera);
		assert!(state.set_call_mute(false, false).is_none());
		state.leave_call();
		assert!(state.start_call(Id(21), false).is_none());
	}

	#[test]
	fn revoked_view_releases_idle_roster_and_rejects_late_voice_updates() {
		use model::permissions as p;
		let mut state = ClientState {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(User {
				primary_guild: None,
				id: Id(1),
				name: "Owner".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				stickers: None,
				id: Id(10),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			channels: [20, 21]
				.into_iter()
				.map(|id| Channel {
					id: Id(id),
					guild: Some(Id(10)),
					kind: 2,
					name: "Room".into(),
					last_message: None,
					parent_id: None,
					position: 0,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
				})
				.collect(),
			..ClientState::default()
		};
		state
			.permissions
			.replace(p::Snapshot {
				guilds: vec![p::Guild {
					id: Id(10),
					owner: Some(Id(99)),
					roles: Some(vec![p::Role {
						id: Id(10),
						bits: p::VIEW_CHANNEL | p::CONNECT,
						name: String::new(),
						color: 0,
						position: 0,
						hoist: false,
					}]),
					member: Some(p::Member {
						roles: vec![],
						timeout_until: None,
					}),
				}],
				channels: [20, 21]
					.into_iter()
					.map(|id| p::Channel {
						id: Id(id),
						guild: Id(10),
						overwrites: Some(vec![]),
					})
					.collect(),
			})
			.unwrap();
		let entry = |channel| RosterEntry {
			guild: Id(10),
			channel: Id(channel),
			participant: Participant {
				user: Id(channel + 100),
				muted: false,
				deafened: false,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: false,
			},
			member: None,
		};
		state.apply_voice(Event::Snapshot {
			guild: None,
			partial: false,
			participants: vec![entry(20), entry(21)],
		});
		assert_eq!(state.voice.roster.len(), 2);
		assert!(state.voice.active.is_none());
		let access = |bits| {
			CoreEvent::Permissions(crate::permissions::Event::Channel {
				channel: Id(20),
				guild: Some(Id(10)),
				overwrites: model::Patch::Value(vec![p::Overwrite {
					id: Id(10),
					kind: 0,
					allow: 0,
					deny: bits,
				}]),
			})
		};
		// Losing CONNECT does not hide a roster that the account may still view.
		state.apply(Envelope {
			generation: state.generation,
			event: access(p::CONNECT),
		});
		assert_eq!(state.voice.roster.len(), 2);
		state.apply(Envelope {
			generation: state.generation,
			event: access(p::VIEW_CHANNEL),
		});
		assert!(!state.can_view(Id(20)));
		assert_eq!(state.voice.roster.len(), 1);
		assert_eq!(state.voice.roster[0].channel, Id(21));
		state.apply_voice(Event::Snapshot {
			guild: None,
			partial: true,
			participants: vec![entry(20)],
		});
		state.apply_voice(Event::State {
			guild: Some(Id(10)),
			channel: Some(Id(20)),
			user: Id(120),
			request: None,
			session: None,
			member: None,
			muted: false,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			video: false,
			streaming: false,
		});
		assert_eq!(state.voice.roster.len(), 1);
		state.apply(Envelope {
			generation: state.generation,
			event: access(0),
		});
		assert!(state.can_view(Id(20)));
		assert_eq!(
			state.voice.roster.len(),
			1,
			"Restoring access cannot restore discarded participants"
		);
		state.apply_voice(Event::Snapshot {
			guild: None,
			partial: true,
			participants: vec![entry(20)],
		});
		assert_eq!(state.voice.roster.len(), 2);
	}

	#[test]
	fn answering_a_dm_call_seeds_the_caller_and_tracks_their_departure() {
		let mut state = ClientState {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(User {
				primary_guild: None,
				id: Id(1),
				name: "Owner".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			channels: vec![Channel {
				last_message: None,
				id: Id(2),
				name: "DM".into(),
				guild: None,
				parent_id: None,
				position: 0,
				kind: 1,
				recipients: vec![User {
					primary_guild: None,
					id: Id(3),
					name: "Peer".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				}],
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..ClientState::default()
		};
		let peer = Participant {
			user: Id(3),
			muted: false,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			video: false,
			streaming: false,
		};
		// CALL_CREATE arrives before this device joins; the caller must not be forgotten.
		state.apply_voice(Event::Call {
			channel: Id(2),
			ringing: Some(vec![Id(1)]),
			participants: Some(vec![peer]),
			unavailable: false,
		});
		assert!(state.voice.active.is_none());
		assert!(state.start_call(Id(2), false).is_some());
		let call = state.voice.active.as_ref().unwrap();
		assert_eq!(call.participants, vec![peer]);
		let request = call.request;
		// The caller hanging up leaves this device alone in the call rather than ending it.
		state.apply_voice(Event::State {
			request: Some(request),
			guild: None,
			channel: None,
			user: Id(3),
			session: None,
			member: None,
			muted: false,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			video: false,
			streaming: false,
		});
		let call = state.voice.active.as_ref().unwrap();
		assert!(call.participants.is_empty());
		assert_ne!(call.phase, Phase::Failed);
		// Their state while this device is not in the call still updates the known membership.
		assert!(state.leave_call().is_some());
		state.apply_voice(Event::State {
			request: None,
			guild: None,
			channel: Some(Id(2)),
			user: Id(3),
			session: None,
			member: None,
			muted: true,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			video: false,
			streaming: false,
		});
		assert!(state.start_call(Id(2), false).is_some());
		let seeded = &state.voice.active.as_ref().unwrap().participants;
		assert_eq!(seeded.len(), 1);
		assert!(seeded[0].user == Id(3) && seeded[0].muted);
		state.apply_voice(Event::Deleted { channel: Id(2) });
		assert!(state.voice.dm_participants.is_empty());
	}
	fn dm_state() -> ClientState {
		ClientState {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(User {
				primary_guild: None,
				id: Id(1),
				name: "Owner".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			channels: vec![Channel {
				last_message: None,
				id: Id(2),
				name: "DM".into(),
				guild: None,
				parent_id: None,
				position: 0,
				kind: 1,
				recipients: vec![User {
					primary_guild: None,
					id: Id(3),
					name: "Peer".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				}],
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..ClientState::default()
		}
	}
	#[test]
	fn dm_calls_require_gesture_and_reject_late_states() {
		let mut state = dm_state();
		state.apply_voice(Event::Call {
			channel: Id(2),
			ringing: Some(vec![Id(1)]),
			participants: None,
			unavailable: false,
		});
		assert_eq!(state.voice.incoming, Some(Id(2)));
		assert!(state.voice.active.is_none()); // Incoming call never grants microphone access.
		assert!(state.start_call(Id(9), true).is_none());
		assert!(state.start_call(Id(2), false).is_some());
		assert!(!state.voice.active.as_ref().unwrap().camera);
		assert!(state.set_call_camera(true).is_none());
		let request = state.voice.active.as_ref().unwrap().request;
		assert_eq!(
			state.voice.active.as_ref().unwrap().phase,
			Phase::Connecting
		);
		assert!(state.start_call(Id(2), true).is_none());
		state.apply(Envelope {
			generation: state.generation + 1,
			event: CoreEvent::Voice(Event::Progress {
				channel: Id(2),
				request,
				phase: Phase::Connected,
			}),
		});
		state.apply_voice(Event::Progress {
			channel: Id(2),
			request: request + 1,
			phase: Phase::Connected,
		});
		assert_eq!(
			state.voice.active.as_ref().unwrap().phase,
			Phase::Connecting
		);
		state.apply_voice(Event::Progress {
			channel: Id(2),
			request,
			phase: Phase::Connected,
		});
		assert_eq!(state.voice.active.as_ref().unwrap().phase, Phase::Connected);
		assert!(matches!(
			state.set_call_camera(true),
			Some(crate::Command::Voice(Command::SetCamera {
				enabled: true,
				..
			}))
		));
		assert!(state.voice.active.as_ref().unwrap().camera);
		state.command_rejected(crate::Command::Voice(Command::SetCamera {
			channel: Id(2),
			request: request + 1,
			enabled: true,
		}));
		assert!(state.voice.active.as_ref().unwrap().camera);
		state.command_rejected(crate::Command::Voice(Command::SetCamera {
			channel: Id(2),
			request,
			enabled: true,
		}));
		assert!(!state.voice.active.as_ref().unwrap().camera);
		assert!(state.set_call_camera(true).is_some());
		assert!(matches!(
			state.set_call_mute(false, true),
			Some(crate::Command::Voice(Command::SetMute {
				mute: true,
				deaf: true,
				..
			}))
		));
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Disconnected,
		});
		// The voice socket is independent of the gateway: a dropped gateway keeps the call.
		assert_eq!(state.voice.active.as_ref().unwrap().phase, Phase::Connected);
		assert!(state.voice.active.as_ref().unwrap().camera);
		state.apply_voice(Event::Progress {
			channel: Id(2),
			request,
			phase: Phase::Connected,
		});
		assert_eq!(state.voice.active.as_ref().unwrap().phase, Phase::Connected);
		assert!(state.leave_call().is_some());
		state.gateway_connected = true;
		state.channels[0].kind = 13;
		assert!(state.start_call(Id(2), true).is_none());
		let secret = Secret::new("SYNTHETIC_VOICE_SECRET".into()).unwrap();
		assert!(!format!("{secret:?}").contains("SYNTHETIC"));
		assert!(Secret::new("bad\nheader".into()).is_err());
		state.channels[0].kind = 1;
		let command = state.start_call(Id(2), true).unwrap();
		state.command_rejected(command);
		assert_eq!(state.voice.active.as_ref().unwrap().phase, Phase::Failed);
		assert!(
			state
				.voice
				.active
				.as_ref()
				.unwrap()
				.error
				.unwrap()
				.contains("queue")
		);
		state.leave_call();
		let command = state
			.start_call_with_mute(Id(2), false, true, true)
			.unwrap();
		assert!(matches!(
			command,
			crate::Command::Voice(Command::Join {
				ring: false,
				mute: true,
				deaf: true,
				..
			})
		));
		assert!(state.voice.active.as_ref().unwrap().muted);
		assert!(state.voice.active.as_ref().unwrap().deafened);
		let mute = state.set_call_mute(true, false).unwrap();
		state.command_rejected(mute);
		assert!(state.voice.active.as_ref().unwrap().muted);
		assert_eq!(
			state.voice.active.as_ref().unwrap().phase,
			Phase::Connecting
		);
		state.logout();
		assert!(state.voice.active.is_none());
		assert!(state.voice.incoming.is_none());
	}

	fn service_ring(state: &mut ClientState, ringing: &[Id]) {
		state.apply_voice(Event::Call {
			channel: Id(2),
			ringing: Some(ringing.to_vec()),
			participants: None,
			unavailable: false,
		});
	}

	#[test]
	fn outgoing_ring_requires_explicit_intent_and_service_confirmation() {
		let mut state = dm_state();
		state.start_call(Id(2), true).unwrap();
		let first = state.voice.active.as_ref().unwrap().request;
		assert_eq!(state.outgoing_ring(), None);
		service_ring(&mut state, &[]); // Initial call creation can precede the ring request.
		state.apply_voice(Event::Progress {
			channel: Id(2),
			request: first,
			phase: Phase::Waiting,
		});
		assert_eq!(state.outgoing_ring(), None);
		service_ring(&mut state, &[Id(3)]);
		assert_eq!(state.outgoing_ring(), Some(Id(2)));
		service_ring(&mut state, &[]);
		assert_eq!(state.outgoing_ring(), None);
		service_ring(&mut state, &[Id(3)]);
		assert_eq!(
			state.outgoing_ring(),
			None,
			"ended ringing cannot restart itself"
		);
		state.leave_call();
		state.apply_voice(Event::Deleted { channel: Id(2) });
		state.start_call(Id(2), true).unwrap();
		service_ring(&mut state, &[Id(3)]);
		state.apply_voice(Event::Failed {
			channel: Id(2),
			request: first,
			message: "Old attempt failed",
		});
		assert_eq!(
			state.outgoing_ring(),
			Some(Id(2)),
			"stale failure cannot stop a new attempt"
		);
		for ring in [false, true] {
			state.leave_call();
			state.start_call(Id(2), ring).unwrap(); // An existing service call is a join.
			service_ring(&mut state, &[Id(3)]);
			let request = state.voice.active.as_ref().unwrap().request;
			state.apply_voice(Event::Progress {
				channel: Id(2),
				request,
				phase: Phase::Waiting,
			});
			assert_eq!(state.outgoing_ring(), None);
		}
	}

	#[test]
	fn outgoing_ring_stops_permanently_on_answer_or_call_invalidation() {
		for end in [
			"leave",
			"deleted",
			"failed",
			"connected",
			"gateway",
			"disconnect",
			"logout",
			"peer-state",
			"peer-call",
			"owner-left",
			"stale-request",
		] {
			let mut state = dm_state();
			state.start_call(Id(2), true).unwrap();
			let request = state.voice.active.as_ref().unwrap().request;
			service_ring(&mut state, &[Id(3)]);
			assert_eq!(state.outgoing_ring(), Some(Id(2)));
			match end {
				"leave" => {
					state.leave_call();
				}
				"deleted" => state.apply_voice(Event::Deleted { channel: Id(2) }),
				"failed" => state.apply_voice(Event::Failed {
					channel: Id(2),
					request,
					message: "Synthetic failure",
				}),
				"connected" => state.apply_voice(Event::Progress {
					channel: Id(2),
					request,
					phase: Phase::Connected,
				}),
				"gateway" => state.gateway_connected = false,
				"disconnect" => state.disconnect_voice("Synthetic disconnect"),
				"logout" => state.logout(),
				"peer-state" | "owner-left" => state.apply_voice(Event::State {
					guild: None,
					member: None,
					server_muted: false,
					server_deafened: false,
					request: Some(request),
					channel: (end == "peer-state").then_some(Id(2)),
					user: if end == "peer-state" { Id(3) } else { Id(1) },
					session: None,
					muted: false,
					deafened: false,
					video: false,
					streaming: false,
				}),
				"peer-call" => state.apply_voice(Event::Call {
					channel: Id(2),
					ringing: Some(vec![Id(3)]),
					participants: Some(vec![Participant {
						user: Id(3),
						muted: false,
						deafened: false,
						server_muted: false,
						server_deafened: false,
						video: false,
						streaming: false,
					}]),
					unavailable: false,
				}),
				"stale-request" => state.voice.active.as_mut().unwrap().request += 1,
				_ => unreachable!(),
			}
			assert_eq!(state.outgoing_ring(), None, "{end}");
			state.gateway_connected = true;
			if let Some(call) = &mut state.voice.active {
				call.phase = Phase::Waiting;
				call.participants.clear();
			}
			service_ring(&mut state, &[Id(3)]);
			assert_eq!(state.outgoing_ring(), None, "{end} must consume the intent");
		}
	}
}
