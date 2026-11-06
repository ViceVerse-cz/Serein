//! First-run setup: the settings people change most, one step at a time, over the app so each
//! choice shows behind it as it is made. Everything here is also in Settings.
use crate::{MessagingUi, design, dialog, i18n::translate, icons};
use egui::{Color32, FontId, Stroke};

/// Title and body keys per step; the order is the flow.
const STEPS: [(&str, &str); 7] = [
	("setup-appearance-title", "setup-appearance-body"),
	("setup-window-title", "setup-window-body"),
	("setup-desktop-title", "setup-desktop-body"),
	("setup-profiles-title", "setup-profiles-body"),
	("setup-chat-title", "setup-chat-body"),
	("setup-notifications-title", "setup-notifications-body"),
	("setup-voice-title", "setup-voice-body"),
];
const WIDTH: f32 = 600.0;
const HEIGHT: f32 = 640.0;
/// The column every step's words and controls share.
const MEASURE: f32 = 448.0;
const FOOTER: f32 = 72.0;
const PILL: f32 = 36.0;

impl MessagingUi {
	/// Opens setup at `step`: from Settings at the start, from fixtures at any screen.
	pub fn preview_setup(&mut self, step: usize) {
		self.setup_pending = true;
		self.setup_step = step.min(STEPS.len() - 1);
	}
	pub(super) fn show_setup(&mut self, ctx: &egui::Context) {
		let step = self.setup_step.min(STEPS.len() - 1);
		let available = ctx.content_rect().size();
		let width = WIDTH.min(available.x - 32.0).max(320.0);
		let height = HEIGHT.min(available.y - 32.0).max(360.0);
		let mut next = None;
		let mut finish = false;
		// Skip is the only way out: a stray click on the backdrop must not lose the flow.
		egui::Modal::new(egui::Id::unique("first-run-setup"))
			.backdrop_color(dialog::backdrop(ctx))
			.frame(dialog::frame(ctx))
			.show(ctx, |ui| {
				ui.set_width(width);
				ui.set_height(height);
				ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
				let p = design::palette(ui);
				let (title, body) = STEPS[step];
				let room = height - FOOTER;
				ui.allocate_ui_with_layout(
					egui::vec2(width, room),
					egui::Layout::top_down(egui::Align::Center),
					|ui| {
						ui.set_min_height(room);
						ui.add_space(24.0);
						dots(ui, step);
						ui.add_space(28.0);
						let measure = MEASURE.min(width - 48.0);
						ui.allocate_ui_with_layout(
							egui::vec2(measure, 0.0),
							egui::Layout::top_down(egui::Align::Center),
							|ui| {
								ui.label(
									design::semibold(ui, translate(title), 22.0)
										.color(p.text_strong),
								);
								ui.add_space(8.0);
								ui.label(
									egui::RichText::new(translate(body))
										.size(14.0)
										.color(p.muted),
								);
							},
						);
						ui.add_space(24.0);
						egui::ScrollArea::vertical()
							.id_salt(("setup-step", step))
							.auto_shrink([false, false])
							.max_height(ui.available_height() - 8.0)
							.show(ui, |ui| {
								ui.vertical_centered(|ui| {
									ui.allocate_ui_with_layout(
										egui::vec2(measure, 0.0),
										egui::Layout::top_down(egui::Align::Min),
										|ui| {
											ui.spacing_mut().item_spacing.y = 8.0;
											match step {
												0 => self.setup_appearance(ui),
												1 => self.setup_window(ui),
												2 => self.setup_desktop(ui),
												3 => self.setup_profiles(ui),
												4 => self.setup_chat(ui),
												5 => self.setup_notifications(ui),
												_ => self.setup_voice(ui),
											}
										},
									);
								});
							});
					},
				);
				let (footer, _) =
					ui.allocate_exact_size(egui::vec2(width, FOOTER), egui::Sense::hover());
				let mut row = ui.new_child(
					egui::UiBuilder::new()
						.max_rect(footer.shrink2(egui::vec2(24.0, 0.0)))
						.layout(egui::Layout::left_to_right(egui::Align::Center)),
				);
				if step > 0 {
					if pill(
						&mut row,
						"setup-back",
						Some(icons::Icon::ArrowLeft),
						None,
						false,
					)
					.clicked()
					{
						next = Some(step - 1);
					}
				} else {
					row.label(
						egui::RichText::new(translate("setup-later-in-settings"))
							.size(13.0)
							.color(p.muted),
					);
				}
				row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					let last = step + 1 == STEPS.len();
					let forward = if last { "setup-done" } else { "setup-continue" };
					let arrow = (!last).then_some(icons::Icon::ArrowRight);
					if pill(ui, forward, None, arrow, true).clicked() {
						if last {
							finish = true;
						} else {
							next = Some(step + 1);
						}
					}
					if !last && pill(ui, "setup-skip", None, None, false).clicked() {
						finish = true;
					}
				});
			});
		if let Some(next) = next {
			self.setup_step = next;
		}
		if finish {
			self.setup_pending = false;
			self.setup_step = 0;
		}
	}

	fn setup_appearance(&mut self, ui: &mut egui::Ui) {
		crate::settings::theme_preference_cards(ui);
		ui.add_space(8.0);
		self.colour_preset_settings(ui);
		ui.add_space(8.0);
		design::card(ui, |ui| self.primary_color_row(ui));
	}

	fn setup_window(&mut self, ui: &mut egui::Ui) {
		design::card(ui, |ui| {
			design::switch(
				ui,
				"settings-appearance-settings-transparency-blur",
				Some("setup-window-transparency-description"),
				&mut self.transparency_blur,
			);
			if self.transparency_blur {
				design::card_divider(ui);
				design::slider_row(
					ui,
					"settings-appearance-settings-transparency",
					None,
					&mut self.transparency,
					0..=100,
					"%",
				);
				ui.add_space(8.0);
				design::blur_control(
					ui,
					"settings-appearance-settings-blur",
					"settings-appearance-settings-the-system-applies-its-standard-blur-strength",
					&mut self.blur,
				);
			}
		});
		ui.add_space(8.0);
		design::notice(ui, design::Level::Info, "setup-window-restart");
	}

	/// How Serein sits on the desktop: launch at login, its title bar and the tray.
	fn setup_desktop(&mut self, ui: &mut egui::Ui) {
		design::card(ui, |ui| self.startup_switches(ui));
		ui.add_space(8.0);
		design::card(ui, |ui| self.window_switches(ui));
	}

	fn setup_profiles(&mut self, ui: &mut egui::Ui) {
		design::card(ui, |ui| {
			crate::settings::cosmetic_switches(ui, &mut self.cosmetics, false);
		});
	}

	fn setup_chat(&mut self, ui: &mut egui::Ui) {
		let mut value = self.reading_preferences;
		design::card(ui, |ui| {
			self.zoom_row(ui, &mut value);
			design::card_divider(ui);
			design::switch(
				ui,
				"reading-chat-reading-settings-compact-message-spacing",
				Some("reading-chat-reading-settings-tighter-gaps-between-message-groups"),
				&mut value.compact_messages,
			);
			design::card_divider(ui);
			design::switch(
				ui,
				"reading-chat-reading-settings-animate-gifs",
				Some("reading-chat-reading-settings-visible-chat-gifs-play-automatically"),
				&mut value.animate_gifs,
			);
		});
		if value != self.reading_preferences {
			self.apply_reading_preferences(ui.ctx(), value);
		}
		ui.add_space(8.0);
		design::group(ui, "time-format-group", |ui| {
			const FORMATS: [model::TimeFormat; 3] = [
				model::TimeFormat::Auto,
				model::TimeFormat::TwelveHour,
				model::TimeFormat::TwentyFourHour,
			];
			let selected = FORMATS
				.iter()
				.position(|format| *format == self.time_format)
				.unwrap_or(0);
			if let Some(index) = design::segmented(
				ui,
				&[
					"time-format-auto",
					"time-format-12-hour",
					"time-format-24-hour",
				],
				selected,
			) {
				self.time_format = FORMATS[index];
				crate::local_time::set_format(self.time_format, self.language);
			}
		});
	}

	fn setup_notifications(&mut self, ui: &mut egui::Ui) {
		design::card(ui, |ui| {
			design::switch(
				ui,
				"notification-settings-notification-settings-enable-desktop-notifications",
				Some("setup-notifications-desktop-description"),
				&mut self.notifications_enabled,
			);
			design::card_divider(ui);
			let mut sounds = !self.notification_options.disable_sounds;
			design::switch(
				ui,
				"setup-notifications-sounds",
				Some("setup-notifications-sounds-description"),
				&mut sounds,
			);
			self.notification_options.disable_sounds = !sounds;
			design::card_divider(ui);
			design::switch(
				ui,
				"settings-auto-idle",
				Some("settings-auto-idle-description"),
				&mut self.auto_idle,
			);
		});
	}

	fn setup_voice(&mut self, ui: &mut egui::Ui) {
		use model::voice_settings::NoiseSuppression;
		design::card(ui, |ui| {
			let mut suppression =
				self.voice_processing.effective().suppression != NoiseSuppression::Off;
			if design::switch(
				ui,
				"voice-voice-popup-content-noise-suppression",
				Some("setup-voice-noise-description"),
				&mut suppression,
			)
			.changed()
			{
				self.voice_processing.edit().suppression = if suppression {
					NoiseSuppression::default()
				} else {
					NoiseSuppression::Off
				};
			}
			design::card_divider(ui);
			design::switch(
				ui,
				"voice-voice-processing-controls-push-to-talk",
				Some(
					"voice-voice-processing-controls-hold-your-configured-shortcut-when-you-want-to-speak",
				),
				&mut self.voice_push_to_talk,
			);
		});
	}
}

/// Where in the flow this is: the current step a short bar, the rest dots.
fn dots(ui: &mut egui::Ui, step: usize) {
	let p = design::palette(ui);
	const GAP: f32 = 8.0;
	let widths: Vec<f32> = (0..STEPS.len())
		.map(|index| {
			ui.ctx().animate_value_with_time(
				egui::Id::unique(("setup-dot", index)),
				if index == step { 20.0 } else { 6.0 },
				0.18,
			)
		})
		.collect();
	let total = widths.iter().sum::<f32>() + GAP * (STEPS.len() - 1) as f32;
	let (rect, response) = ui.allocate_exact_size(egui::vec2(total, 20.0), egui::Sense::hover());
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Label,
			true,
			format!("{} / {}", step + 1, STEPS.len()),
		)
	});
	let mut x = rect.left();
	for (index, width) in widths.into_iter().enumerate() {
		let dot =
			egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 3.0), egui::vec2(width, 6.0));
		let color = if index == step {
			p.accent
		} else if index < step {
			p.muted
		} else {
			p.border
		};
		ui.painter().rect_filled(dot, 3, color);
		x += width + GAP;
	}
}

/// Setup's footer buttons: the way on is the one filled pill, every other action a ghost pill.
fn pill(
	ui: &mut egui::Ui,
	label: &str,
	before: Option<icons::Icon>,
	after: Option<icons::Icon>,
	filled: bool,
) -> egui::Response {
	let label = translate(label);
	let p = design::palette(ui);
	let galley = ui.painter().layout_no_wrap(
		label.clone(),
		FontId::new(14.0, design::medium_family(ui.ctx())),
		Color32::WHITE,
	);
	const ICON: f32 = 16.0;
	let icons = [before, after].iter().flatten().count() as f32;
	let width = galley.size().x + 32.0 + icons * (ICON + 8.0);
	let (rect, response) = ui.allocate_exact_size(egui::vec2(width, PILL), egui::Sense::click());
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
	let hot = response.hovered() || response.has_focus();
	let (fill, text) = if filled {
		let fill = if response.is_pointer_button_down_on() {
			p.accent.gamma_multiply(0.85)
		} else if hot {
			p.accent.linear_multiply(1.1)
		} else {
			p.accent
		};
		(fill, p.accent_text)
	} else {
		(
			if hot { p.hover } else { Color32::TRANSPARENT },
			if hot { p.text_strong } else { p.text },
		)
	};
	let painter = ui.painter();
	let radius = (PILL / 2.0) as u8;
	painter.rect_filled(rect, radius, fill);
	if response.has_focus() {
		painter.rect_stroke(
			rect.expand(2.0),
			radius + 2,
			Stroke::new(2.0, p.accent),
			egui::StrokeKind::Outside,
		);
	}
	let mut x = rect.left() + 16.0;
	let icon_rect = |x: f32| {
		egui::Rect::from_min_size(
			egui::pos2(x, rect.center().y - ICON / 2.0),
			egui::Vec2::splat(ICON),
		)
	};
	if let Some(icon) = before {
		icons::paint(painter, icon, icon_rect(x), text);
		x += ICON + 8.0;
	}
	let galley_width = galley.size().x;
	painter.galley_with_override_text_color(
		egui::pos2(x, rect.center().y - galley.size().y / 2.0),
		galley,
		text,
	);
	x += galley_width + 8.0;
	if let Some(icon) = after {
		icons::paint(painter, icon, icon_rect(x), text);
	}
	if ui.is_enabled() && response.hovered() {
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
	}
	response
}
