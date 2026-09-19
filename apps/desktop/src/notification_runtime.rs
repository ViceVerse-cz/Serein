//! Applies notification device choices to the existing filtered message and incoming-call state.
use client_core::{State, auth::AuthState};
use model::{
	Id, PresenceStatus,
	notification_preferences::{Device, Sound},
};
use std::time::{Duration, Instant};

#[derive(Default)]
struct VoiceCues {
	channel: Option<Id>,
	participants: Vec<(Id, bool)>,
	controls: Option<(bool, bool)>,
}
impl VoiceCues {
	fn clear(&mut self) {
		self.channel = None;
		self.participants.clear();
		self.controls = None;
	}
	fn update(
		&mut self,
		state: &State,
		muted: bool,
		deafened: bool,
		live: bool,
	) -> Option<crate::notification_sounds::Cue> {
		use crate::notification_sounds::Cue;
		let controls = self.controls.replace((muted, deafened));
		if !live {
			self.channel = None;
			self.participants.clear();
			return None;
		}
		let control = controls.and_then(|(was_muted, was_deafened)| {
			if was_deafened != deafened {
				Some(if deafened { Cue::Deafen } else { Cue::Undeafen })
			} else if was_muted != muted {
				Some(if muted { Cue::Mute } else { Cue::Unmute })
			} else {
				None
			}
		});
		let Some(call) = state.voice.active.as_ref() else {
			let left = self.channel.take().is_some();
			self.participants.clear();
			return control.or(left.then_some(Cue::Leave));
		};
		if !matches!(
			call.phase,
			client_core::voice::Phase::Connected | client_core::voice::Phase::Waiting
		) {
			return control;
		}
		let current: Vec<_> = call
			.participants
			.iter()
			.map(|participant| (participant.user, participant.streaming))
			.collect();
		let channel_changed = self.channel.replace(call.channel) != Some(call.channel);
		let joined = current
			.iter()
			.any(|(user, _)| !self.participants.iter().any(|(known, _)| known == user));
		let left = self
			.participants
			.iter()
			.any(|(user, _)| !current.iter().any(|(known, _)| known == user));
		let streaming = current.iter().any(|(user, streaming)| {
			*streaming
				&& self
					.participants
					.iter()
					.find(|(known, _)| known == user)
					.is_some_and(|(_, was_streaming)| !was_streaming)
		});
		self.participants = current;
		control.or_else(|| {
			channel_changed
				.then_some(Cue::Join)
				.or_else(|| joined.then_some(Cue::Join))
				.or_else(|| left.then_some(Cue::Leave))
				.or_else(|| streaming.then_some(Cue::StreamStart))
		})
	}
}

pub enum Alert {
	Message {
		title: String,
		body: String,
		avatar_key: String,
		image_path: Option<String>,
	},
}
#[derive(Default)]
pub struct Runtime {
	sounds: crate::notification_sounds::Sounds,
	options: Device,
	was_audible: bool,
	ring: Option<(Id, Instant)>,
	voice: VoiceCues,
	badge: Option<u32>,
	badge_check: Option<Instant>,
	badge_status: &'static str,
}
impl Runtime {
	pub fn clear(&mut self, window: &winit::window::Window) {
		self.sounds.stop();
		self.ring = None;
		self.voice.clear();
		if self.badge.is_some_and(|count| count > 0) {
			let _ = platform::badge::set(window, 0);
		}
		self.badge = None;
	}
	/// Returns a coalesced desktop alert request. Sound is independent of desktop alerts.
	pub fn poll(
		&mut self,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		window: &winit::window::Window,
		ctx: &eframe::egui::Context,
		fixture: bool,
	) -> Option<Alert> {
		let live = !fixture && !state.demo && state.auth == AuthState::Authenticated;
		let audible = live && ui.own_presence.status != PresenceStatus::DoNotDisturb;
		let options = ui.notification_options;
		let badges = platform::badge::supported() && options.unread_badge;
		if options != self.options || (self.was_audible && !audible) {
			self.sounds.stop();
			self.options = options;
		}
		self.was_audible = audible;
		let focused = ctx.input(|i| {
			i.focused
				&& i.viewport().visible() != Some(false)
				&& i.viewport().minimized != Some(true)
		});
		let mut alert = None;
		let mut sound: Option<crate::notification_sounds::Cue> = None;
		while let Some(notification) = state.take_notification() {
			let current = focused && ui.viewing_latest(notification.channel);
			let cue = if current {
				Sound::CurrentChannel
			} else {
				Sound::Message
			};
			if audible {
				if ui.notifications_enabled && !current {
					let image_path = state.user.as_ref().and_then(|user| {
						crate::avatars::notification_image_path(user.id, &notification.avatar_key)
					});
					alert = Some(Alert::Message {
						title: notification.sender,
						body: notification.preview,
						avatar_key: notification.avatar_key,
						image_path,
					});
				}
				if options.allows(cue) {
					sound = Some(cue.into());
				}
			}
		}
		let incoming = state.voice.incoming.filter(|id| {
			audible && state.notification_allowed(*id) && options.allows(Sound::IncomingRing)
		});
		if self.ring.map(|(id, _)| id) != incoming {
			if self.ring.is_some() {
				self.sounds.stop();
			}
			self.ring = incoming.map(|id| (id, Instant::now()));
			if incoming.is_some() {
				sound = Some(Sound::IncomingRing.into());
			}
		} else if let Some((_, played)) = &mut self.ring
			&& played.elapsed() >= crate::notification_sounds::RING_INTERVAL
		{
			*played = Instant::now();
			sound = Some(Sound::IncomingRing.into());
		}
		if self.ring.is_some() {
			ctx.request_repaint_after(Duration::from_millis(250));
		}
		// Explicit previews are allowed in the offline demo and intentionally ignore automatic mute choices.
		if let Some(preview) = ui.notification_preview.take() {
			sound = Some(preview.into());
		}
		let voice = self
			.voice
			.update(state, ui.voice_muted, ui.voice_deafened, live);
		let sound = sound.or(voice);
		if let Some(sound) = sound {
			self.sounds.play(sound, ctx);
		}
		if self
			.badge_check
			.is_none_or(|time| time.elapsed() >= Duration::from_secs(1))
			|| !badges
			|| !live
		{
			self.badge_check = Some(Instant::now());
			let pings = if live && badges {
				state
					.channels
					.iter()
					.try_fold(0u32, |total, channel| {
						if total >= 100 {
							return None;
						}
						Some(
							total
								.saturating_add(state.mention_count(channel.id))
								.min(100),
						)
					})
					.unwrap_or(100)
			} else {
				0
			};
			if platform::badge::supported() && self.badge != Some(pings) {
				self.badge_status = platform::badge::set(window, pings).err().unwrap_or("");
				self.badge = Some(pings);
			}
		}
		if live && badges {
			ctx.request_repaint_after(Duration::from_secs(1));
		}
		ui.notification_sound_status = if self.sounds.status().is_empty() {
			self.badge_status
		} else {
			self.sounds.status()
		};
		alert
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::notification_sounds::Cue;
	use client_core::voice::{Call, Participant, Phase};

	fn participant(user: u64) -> Participant {
		Participant {
			user: Id(user),
			muted: false,
			deafened: false,
			server_muted: false,
			server_deafened: false,
			video: false,
			streaming: false,
		}
	}

	#[test]
	fn voice_cues_follow_controls_members_streams_and_call_lifecycle() {
		let mut state = State::default();
		let mut cues = VoiceCues::default();
		assert_eq!(cues.update(&state, false, false, true), None);
		state.voice.active = Some(Call {
			channel: Id(20),
			guild: Some(Id(10)),
			connected_at: Some(Instant::now()),
			server_muted: false,
			server_deafened: false,
			request: 1,
			phase: Phase::Connected,
			muted: false,
			deafened: false,
			participants: vec![participant(2)],
			camera: false,
			watching: None,
			error: None,
		});
		assert_eq!(cues.update(&state, false, false, true), Some(Cue::Join));
		state.voice.active.as_mut().unwrap().phase = Phase::Securing;
		assert_eq!(cues.update(&state, false, false, true), None);
		state.voice.active.as_mut().unwrap().phase = Phase::Connected;
		state
			.voice
			.active
			.as_mut()
			.unwrap()
			.participants
			.push(participant(3));
		assert_eq!(cues.update(&state, false, false, true), Some(Cue::Join));
		state.voice.active.as_mut().unwrap().participants[1].streaming = true;
		assert_eq!(
			cues.update(&state, false, false, true),
			Some(Cue::StreamStart)
		);
		state.voice.active.as_mut().unwrap().participants.remove(0);
		assert_eq!(cues.update(&state, false, false, true), Some(Cue::Leave));
		assert_eq!(cues.update(&state, true, false, true), Some(Cue::Mute));
		assert_eq!(cues.update(&state, true, true, true), Some(Cue::Deafen));
		assert_eq!(cues.update(&state, false, false, true), Some(Cue::Undeafen));
		state.voice.active = None;
		assert_eq!(cues.update(&state, false, false, true), Some(Cue::Leave));
		assert_eq!(cues.update(&state, false, false, false), None);
	}
}
