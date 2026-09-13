//! Applies notification device choices to the existing filtered message and incoming-call state.
use client_core::{State, auth::AuthState};
use model::{
	Id, PresenceStatus,
	notification_preferences::{Device, Sound},
};
use std::time::{Duration, Instant};
#[derive(Default)]
pub struct Runtime {
	sounds: crate::notification_sounds::Sounds,
	options: Device,
	was_audible: bool,
	ring: Option<(Id, Instant)>,
	badge: Option<bool>,
	badge_check: Option<Instant>,
	badge_status: &'static str,
}
impl Runtime {
	pub fn clear(&mut self, window: &winit::window::Window) {
		self.sounds.stop();
		self.ring = None;
		if self.badge == Some(true) {
			let _ = platform::badge::set(window, false);
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
	) -> Option<platform::notifications::Kind> {
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
					alert = Some(platform::notifications::Kind::Message);
				}
				if options.allows(cue) {
					sound = Some(cue);
				}
			}
		}
		while let Some(notification) = state.take_social_notification() {
			if audible {
				if ui.notifications_enabled {
					alert = Some(match notification.kind {
						model::notification_settings::SocialKind::Streaming => {
							platform::notifications::Kind::Streaming
						}
						model::notification_settings::SocialKind::UpcomingEvent => {
							platform::notifications::Kind::UpcomingEvent
						}
						model::notification_settings::SocialKind::Reaction => {
							platform::notifications::Kind::Reaction
						}
						model::notification_settings::SocialKind::FriendsOnline => {
							platform::notifications::Kind::FriendsOnline
						}
						model::notification_settings::SocialKind::ProfileUpdates => {
							platform::notifications::Kind::ProfileUpdates
						}
					});
				}
				if options.allows(Sound::Message) {
					sound = Some(Sound::Message);
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
			self.sounds.play(sound, ctx);
		}
		if self
			.badge_check
			.is_none_or(|time| time.elapsed() >= Duration::from_secs(1))
			|| !badges
			|| !live
		{
			self.badge_check = Some(Instant::now());
			let unread = live
				&& badges && state
				.channels
				.iter()
				.any(|channel| state.channel_unread(channel) == Some(true));
			if platform::badge::supported() && self.badge != Some(unread) {
				self.badge_status = platform::badge::set(window, unread).err().unwrap_or("");
				self.badge = Some(unread);
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
