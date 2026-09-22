//! Device notification sections; message alerts, sounds and badges are stored locally.
use crate::{MessagingUi, design};
use model::notification_preferences::Sound;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Tab {
	#[default]
	Overview,
	Sounds,
	Badges,
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn notification_controls_change_real_state_and_emit_preview_requests() {
		fn texts(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(t) => {
					out.push((t.galley.job.text.clone(), t.visual_bounding_rect()))
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						texts(shape, out);
					}
				}
				_ => {}
			}
		}
		for (width, dark) in [(320.0, true), (900.0, false)] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut view = MessagingUi::default();
			let render = |view: &mut MessagingUi, events| {
				let mut labels = vec![];
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 3600.0),
						)),
						events,
						focused: true,
						..Default::default()
					},
					|ui| {
						view.notification_settings(ui, true);
						assert!(
							ui.min_rect().width() <= width,
							"notification settings overflow"
						);
					},
				);
				for shape in &output.shapes {
					texts(&shape.shape, &mut labels);
				}
				output.drop_without_applying_deltas();
				labels
			};
			let labels = render(&mut view, vec![]);
			for label in [
				"Overview",
				"Sounds",
				"Badges",
				"Enable Unread Message Badge",
				"Incoming Ring",
				"Microphone Muted",
				"Microphone Unmuted",
				"Deafen",
				"Undeafen",
				"Sound Volume",
				"Disable All Notification Sounds",
			] {
				assert!(
					labels.iter().any(|(s, _)| s.eq_ignore_ascii_case(label)),
					"missing {label}"
				);
			}
			for label in ["Email", "Advanced", "Friends come online"] {
				assert!(!labels.iter().any(|(s, _)| s == label), "stale {label}");
			}
			for (label, sound) in [
				("Outgoing Ring", Sound::OutgoingRing),
				("Camera On", Sound::CameraOn),
				("Screen Share Started", Sound::ScreenShareOn),
				("Call Joined", Sound::UserJoin),
				("User Left Call", Sound::UserLeave),
			] {
				let point = labels
					.iter()
					.skip_while(|(text, _)| text != label)
					.find(|(text, _)| text == "Preview Sound")
					.unwrap_or_else(|| panic!("missing preview for {label}"))
					.1
					.center();
				for pressed in [true, false] {
					render(
						&mut view,
						vec![
							egui::Event::PointerMoved(point),
							egui::Event::PointerButton {
								pos: point,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					);
				}
				assert_eq!(view.notification_preview.take(), Some(sound));
			}
			let labels = render(&mut view, vec![]);
			let point = labels
				.iter()
				.find(|(text, _)| text == "Preview Sound")
				.unwrap()
				.1
				.center();
			for pressed in [true, false] {
				render(
					&mut view,
					vec![
						egui::Event::PointerMoved(point),
						egui::Event::PointerButton {
							pos: point,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(view.notification_preview.take(), Some(Sound::Message));
		}
	}
}
impl Tab {
	pub const ALL: [Self; 3] = [Self::Overview, Self::Sounds, Self::Badges];
	pub fn label(self) -> &'static str {
		match self {
			Self::Overview => "Overview",
			Self::Sounds => "Sounds",
			Self::Badges => "Badges",
		}
	}
}
#[derive(Default)]
pub(super) struct Navigation {
	pub active: Tab,
	pub jump: Option<Tab>,
}
impl Navigation {
	fn heading(&mut self, ui: &mut egui::Ui, tab: Tab) {
		if tab != Tab::Overview {
			ui.add_space(12.0);
		}
		let heading = ui.label(design::eyebrow(ui, tab.label(), design::palette(ui).muted));
		if heading.rect.top() <= ui.clip_rect().top() + 28.0 {
			self.active = tab;
		}
		if self.jump == Some(tab) {
			ui.scroll_to_rect(heading.rect.expand(8.0), Some(egui::Align::Min));
			self.jump = None;
		}
	}
}
impl MessagingUi {
	pub(super) fn notification_settings(&mut self, ui: &mut egui::Ui, demo: bool) {
		if ui.available_width() < 500.0 {
			ui.horizontal_wrapped(|ui| {
				for tab in Tab::ALL {
					if ui
						.selectable_label(self.settings.notifications.active == tab, tab.label())
						.clicked()
					{
						self.settings.notifications.jump = Some(tab);
					}
				}
			});
		}
		self.settings.notifications.heading(ui, Tab::Overview);
		design::card(ui, |ui| {
			design::switch(
				ui,
				"Enable Desktop Notifications",
				Some(
					"For per-channel or per-server notifications, right-click the channel or server and select Notification Settings.",
				),
				&mut self.notifications_enabled,
			);
			if !demo && !self.notification_status.is_empty() {
				design::hint(ui, self.notification_status);
			}
		});
		self.settings.notifications.heading(ui, Tab::Sounds);
		design::card(ui, |ui| {
			design::slider_row(
				ui,
				"Sound Volume",
				Some("Adjusts the volume of all notification sounds and ringtones."),
				&mut self.notification_options.volume,
				0..=100,
				"%",
			);
			design::card_divider(ui);
			design::switch(
				ui,
				"Disable All Notification Sounds",
				Some(
					"Disables notification sounds. Your individual sound preferences are saved and restored when you turn this off.",
				),
				&mut self.notification_options.disable_sounds,
			);
			design::card_divider(ui);
			let sounds = vec![
				(
					"New Message",
					&mut self.notification_options.new_message,
					Sound::Message,
				),
				(
					"New Message in the channel I'm currently reading",
					&mut self.notification_options.current_channel,
					Sound::CurrentChannel,
				),
				(
					"Incoming Ring",
					&mut self.notification_options.incoming_ring,
					Sound::IncomingRing,
				),
				(
					"Outgoing Ring",
					&mut self.notification_options.outgoing_ring,
					Sound::OutgoingRing,
				),
				(
					"Microphone Muted",
					&mut self.notification_options.mute,
					Sound::Mute,
				),
				(
					"Microphone Unmuted",
					&mut self.notification_options.unmute,
					Sound::Unmute,
				),
				(
					"Deafen",
					&mut self.notification_options.deafen,
					Sound::Deafen,
				),
				(
					"Undeafen",
					&mut self.notification_options.undeafen,
					Sound::Undeafen,
				),
				(
					"Camera On",
					&mut self.notification_options.camera_on,
					Sound::CameraOn,
				),
				(
					"Screen Share Started",
					&mut self.notification_options.screen_share_on,
					Sound::ScreenShareOn,
				),
				(
					"Call Joined",
					&mut self.notification_options.user_join,
					Sound::UserJoin,
				),
				(
					"User Left Call",
					&mut self.notification_options.user_leave,
					Sound::UserLeave,
				),
			];
			for (index, (label, value, sound)) in sounds.into_iter().enumerate() {
				if index > 0 {
					design::card_divider(ui);
				}
				design::switch(ui, label, None, value);
				if design::text_action(ui, "Preview Sound").clicked() {
					self.notification_preview = Some(sound);
				}
			}
			if !self.notification_sound_status.is_empty() {
				design::card_divider(ui);
				design::notice(ui, design::Level::Warning, self.notification_sound_status);
			}
		});
		ui.add_space(4.0);
		design::card(ui, |ui| {
			if design::row(
				ui,
				"Voice & Video",
				Some("Ringtones, call devices and microphone processing."),
				|ui| design::button(ui, "Open", design::ButtonKind::Outline),
			)
			.clicked()
			{
				self.open_voice_settings();
			}
		});
		self.settings.notifications.heading(ui, Tab::Badges);
		design::card(ui, |ui| {
			ui.add_enabled_ui(cfg!(target_os = "windows"), |ui| {
				design::switch(
					ui,
					"Enable Unread Message Badge",
					Some(if cfg!(target_os = "windows") {
						"Shows a red badge on the app icon when you have unread messages."
					} else {
						"App icon badges are not available on this platform yet."
					}),
					&mut self.notification_options.unread_badge,
				);
			});
		});
	}
}
