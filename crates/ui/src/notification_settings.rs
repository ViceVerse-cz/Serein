//! Device notification sections; message alerts, sounds and badges are stored locally.
use crate::{MessagingUi, design};
use egui::RichText;
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
			] {
				assert!(labels.iter().any(|(s, _)| s == label), "missing {label}");
			}
			for label in ["Email", "Advanced", "Friends come online"] {
				assert!(!labels.iter().any(|(s, _)| s == label), "stale {label}");
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
			ui.add_space(32.0);
			ui.separator();
			ui.add_space(32.0);
		}
		let heading = ui.label(
			RichText::new(tab.label())
				.size(26.0)
				.color(design::palette(ui).text_strong),
		);
		if heading.rect.top() <= ui.clip_rect().top() + 28.0 {
			self.active = tab;
		}
		if self.jump == Some(tab) {
			ui.scroll_to_rect(heading.rect.expand(8.0), Some(egui::Align::Min));
			self.jump = None;
		}
		ui.add_space(22.0);
	}
}
fn row(ui: &mut egui::Ui, label: &str, detail: Option<&str>, value: &mut bool) {
	design::switch(ui, label, detail, value);
	ui.add_space(10.0);
}
impl MessagingUi {
	pub(super) fn notification_settings(&mut self, ui: &mut egui::Ui, demo: bool) {
		let colors = design::palette(ui);
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
		row(
			ui,
			"Enable Desktop Notifications",
			Some(
				"For per-channel or per-server notifications, right-click the channel or server and select Notification Settings.",
			),
			&mut self.notifications_enabled,
		);
		ui.label(
			RichText::new(if demo {
				"Offline preview"
			} else {
				self.notification_status
			})
			.size(12.0)
			.color(colors.muted),
		);
		self.settings.notifications.heading(ui, Tab::Sounds);
		for (label, value, sound) in [
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
		] {
			row(ui, label, None, value);
			if ui.link("Preview Sound").clicked() {
				self.notification_preview = Some(sound);
			}
			ui.add_space(14.0);
			ui.separator();
			ui.add_space(10.0);
		}
		row(
			ui,
			"Disable All Notification Sounds",
			Some(
				"Disables notification sounds. Your individual sound preferences are saved and restored when you turn this off.",
			),
			&mut self.notification_options.disable_sounds,
		);
		if !self.notification_sound_status.is_empty() {
			ui.label(
				RichText::new(self.notification_sound_status)
					.size(12.0)
					.color(colors.muted),
			);
		}
		ui.add_space(12.0);
		ui.label(design::medium(ui, "Related Settings", 15.0).color(colors.muted));
		if ui
			.add(
				egui::Button::new("Voice & Video  ›")
					.min_size(egui::vec2(ui.available_width(), 48.0)),
			)
			.clicked()
		{
			self.open_voice_settings();
		}
		self.settings.notifications.heading(ui, Tab::Badges);
		ui.add_enabled_ui(cfg!(target_os = "windows"), |ui| {
			row(
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
	}
}
