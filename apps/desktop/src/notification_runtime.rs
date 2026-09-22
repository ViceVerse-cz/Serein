//! Applies notification device choices to filtered messages and explicit call ringing state.
use client_core::{State, auth::AuthState};
use model::{
	Id, PresenceStatus,
	notification_preferences::{Device, Sound},
};
use std::time::{Duration, Instant};

pub enum Alert {
	Message {
		channel: Id,
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
	ring: Option<((Id, Sound), Instant)>,
	badge: Option<u32>,
	badge_check: Option<Instant>,
	badge_status: &'static str,
}
impl Runtime {
	fn ring_cue(&mut self, ringing: Option<(Id, Sound)>, now: Instant) -> Option<Sound> {
		if self.ring.map(|(key, _)| key) != ringing {
			if self.ring.is_some() {
				self.sounds.stop();
			}
			self.ring = ringing.map(|key| (key, now));
			return ringing.map(|(_, cue)| cue);
		}
		if let Some(((_, cue), played)) = &mut self.ring {
			let interval = if *cue == Sound::OutgoingRing {
				crate::notification_sounds::OUTGOING_RING_INTERVAL
			} else {
				crate::notification_sounds::RING_INTERVAL
			};
			if now.duration_since(*played) >= interval {
				*played = now;
				return Some(*cue);
			}
		}
		None
	}
	pub fn clear(&mut self, window: &winit::window::Window) {
		self.sounds.stop();
		self.ring = None;
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
		let mut sound = None;
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
						channel: notification.channel,
						title: notification.sender,
						body: notification.preview,
						avatar_key: notification.avatar_key,
						image_path,
					});
				}
				if options.allows(cue) {
					sound = Some(cue);
				}
			}
		}
		let incoming = state.voice.incoming.filter(|id| {
			audible && state.notification_allowed(*id) && options.allows(Sound::IncomingRing)
		});
		// Dialing is explicit local feedback, like mute/camera cues; DND only silences alerts.
		let outgoing = state
			.outgoing_ring()
			.filter(|_| live && options.allows(Sound::OutgoingRing));
		let ringing = incoming
			.map(|id| (id, Sound::IncomingRing))
			.or_else(|| outgoing.map(|id| (id, Sound::OutgoingRing)));
		if let Some(cue) = self.ring_cue(ringing, Instant::now()) {
			sound = Some(cue);
		}
		if self.ring.is_some() {
			ctx.request_repaint_after(Duration::from_millis(250));
		}
		// Explicit previews are allowed in the offline demo and intentionally ignore automatic mute choices.
		if let Some(preview) = ui.notification_preview.take() {
			sound = Some(preview);
		}
		if let Some(sound) = sound {
			self.sounds.play(sound, options.volume, ctx);
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

	#[test]
	fn ringtone_timer_repeats_each_cue_and_stops_on_clear() {
		let mut runtime = Runtime::default();
		let now = Instant::now();
		let outgoing = Some((Id(2), Sound::OutgoingRing));
		assert_eq!(runtime.ring_cue(outgoing, now), Some(Sound::OutgoingRing));
		assert_eq!(
			runtime.ring_cue(outgoing, now + Duration::from_secs(2)),
			None
		);
		assert_eq!(
			runtime.ring_cue(outgoing, now + Duration::from_secs(3)),
			Some(Sound::OutgoingRing)
		);
		let incoming = Some((Id(2), Sound::IncomingRing));
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(3)),
			Some(Sound::IncomingRing)
		);
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(6)),
			None
		);
		assert_eq!(
			runtime.ring_cue(incoming, now + Duration::from_secs(9)),
			Some(Sound::IncomingRing)
		);
		assert_eq!(runtime.ring_cue(None, now + Duration::from_secs(9)), None);
		assert!(runtime.ring.is_none());
		assert_eq!(runtime.ring_cue(None, now + Duration::from_secs(15)), None);
	}
}
