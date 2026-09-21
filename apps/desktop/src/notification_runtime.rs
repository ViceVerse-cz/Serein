//! Applies notification device choices to the existing filtered message and incoming-call state.
use client_core::{State, auth::AuthState};
use model::{
	Id, PresenceStatus,
	notification_preferences::{Device, Sound},
};
use std::time::{Duration, Instant};

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
	badge: Option<u32>,
	badge_check: Option<Instant>,
	badge_status: &'static str,
}
impl Runtime {
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
		if self.ring.map(|(id, _)| id) != incoming {
			if self.ring.is_some() {
				self.sounds.stop();
			}
			self.ring = incoming.map(|id| (id, Instant::now()));
			if incoming.is_some() {
				sound = Some(Sound::IncomingRing);
			}
		} else if let Some((_, played)) = &mut self.ring
			&& played.elapsed() >= crate::notification_sounds::RING_INTERVAL
		{
			*played = Instant::now();
			sound = Some(Sound::IncomingRing);
		}
		if self.ring.is_some() {
			ctx.request_repaint_after(Duration::from_millis(250));
		}
		// Explicit previews are allowed in the offline demo and intentionally ignore automatic mute choices.
		if let Some(preview) = ui.notification_preview.take() {
			sound = Some(preview);
		}
		if let Some(sound) = sound {
			self.sounds.play(sound, options.discord_sounds, ctx);
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
