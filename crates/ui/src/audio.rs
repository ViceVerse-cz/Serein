//! Inline controls only. The desktop owns downloading, decoding and the output device.
use model::{Attachment, Id, Message};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum AudioState {
	#[default]
	Idle,
	Loading,
	Playing,
	Paused,
	Ended,
	Failed(&'static str),
}
pub enum AudioCommand {
	Play(Attachment),
	Pause(bool),
	Seek(f64),
	Volume(f32),
	Stop,
}
pub struct AudioUi {
	pub active: Option<(Id, Id, Attachment)>,
	pub state: AudioState,
	pub position: f64,
	pub duration: f64,
	pub command: Option<AudioCommand>,
	pub seen: bool,
	pub volume: f32,
}
impl Default for AudioUi {
	fn default() -> Self {
		Self {
			active: None,
			state: AudioState::Idle,
			position: 0.0,
			duration: 0.0,
			command: None,
			seen: false,
			volume: 1.0,
		}
	}
}
impl AudioUi {
	pub fn stop(&mut self) {
		self.active = None;
		self.state = AudioState::Idle;
		self.command = Some(AudioCommand::Stop);
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		message: &Message,
		attachment: &Attachment,
	) -> egui::Response {
		let colors = crate::design::palette(ui);
		let voice = attachment.is_voice_message();
		let button_color = if voice {
			colors.text_strong
		} else {
			colors.accent
		};
		let icon_color = if voice {
			colors.raised
		} else {
			colors.accent_text
		};
		let active = self.active.as_ref().is_some_and(|(channel, id, file)| {
			*channel == message.channel && *id == message.id && file == attachment
		});
		let state = if active { self.state } else { AudioState::Idle };
		let duration = if active && self.duration > 0.0 {
			self.duration
		} else {
			attachment.duration_ms.map_or(0.0, |ms| ms as f64 / 1000.0)
		};
		let width = ui.available_width().min(if voice { 320.0 } else { 380.0 });
		let card = egui::Frame::new()
			.fill(colors.raised)
			.stroke(egui::Stroke::new(1.0, colors.border))
			.corner_radius(if voice { 12 } else { 8 })
			.inner_margin(12)
			.show(ui, |ui| {
				ui.set_width((width - 26.0).max(1.0)); // Padding plus the one-point border.
				ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
				ui.spacing_mut().slider_rail_height = 3.0;
				// Chat buttons use the card fill; give media sliders their own visible track.
				ui.visuals_mut().widgets.inactive.bg_fill = colors.muted.gamma_multiply(0.45);
				ui.visuals_mut().widgets.inactive.fg_stroke = egui::Stroke::new(1.0, colors.muted);
				ui.visuals_mut().widgets.hovered.bg_fill = colors.text_strong;
				ui.visuals_mut().widgets.active.bg_fill = colors.text_strong;
				ui.visuals_mut().selection.bg_fill = colors.accent;
				// Small painted thumbs, with the shared 32-point slider hit targets intact.
				let widgets = &mut ui.visuals_mut().widgets;
				for widget in [
					&mut widgets.noninteractive,
					&mut widgets.inactive,
					&mut widgets.hovered,
					&mut widgets.active,
				] {
					widget.expansion = -6.0;
				}
				if !voice {
					ui.horizontal(|ui| {
						crate::icons::inline(
							ui,
							crate::icons::Icon::Soundboard,
							16.0,
							colors.muted,
						);
						ui.add(
							egui::Label::new(
								crate::design::semibold(ui, &attachment.filename, 13.0)
									.color(colors.text_strong),
							)
							.truncate(),
						)
						.on_hover_text(&attachment.filename);
					});
				}
				ui.horizontal(|ui| {
					let label = match state {
						AudioState::Loading => "Cancel",
						AudioState::Playing => "Pause",
						AudioState::Ended => "Replay",
						AudioState::Failed(_) => "Retry",
						_ => "Play",
					};
					let (_, play) =
						ui.allocate_exact_size(egui::Vec2::splat(32.0), egui::Sense::click());
					ui.painter().circle_filled(
						play.rect.center(),
						16.0,
						if play.hovered() {
							button_color.gamma_multiply(0.85)
						} else {
							button_color
						},
					);
					play.widget_info(|| {
						egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label)
					});
					let center = play.rect.center();
					match state {
						AudioState::Playing => {
							for x in [-4.0, 4.0] {
								ui.painter().rect_filled(
									egui::Rect::from_center_size(
										center + egui::vec2(x, 0.0),
										egui::vec2(3.0, 12.0),
									),
									1,
									icon_color,
								);
							}
						}
						AudioState::Loading | AudioState::Ended | AudioState::Failed(_) => {
							crate::icons::paint(
								ui.painter(),
								if state == AudioState::Loading {
									crate::icons::Icon::Close
								} else {
									crate::icons::Icon::Reload
								},
								play.rect.shrink(7.0),
								icon_color,
							);
						}
						_ => {
							ui.painter().add(egui::Shape::convex_polygon(
								vec![
									center + egui::vec2(-4.0, -6.0),
									center + egui::vec2(6.0, 0.0),
									center + egui::vec2(-4.0, 6.0),
								],
								icon_color,
								egui::Stroke::NONE,
							));
						}
					}
					if play.has_focus() {
						ui.painter().circle_stroke(
							center,
							19.0,
							egui::Stroke::new(2.0, colors.accent),
						);
					}
					if play.on_hover_text(label).clicked() {
						self.command = Some(match state {
							AudioState::Loading => {
								self.active = None;
								AudioCommand::Stop
							}
							AudioState::Playing => AudioCommand::Pause(true),
							AudioState::Paused => AudioCommand::Pause(false),
							_ => {
								self.active =
									Some((message.channel, message.id, attachment.clone()));
								self.state = AudioState::Loading;
								self.position = 0.0;
								self.duration = 0.0;
								AudioCommand::Play(attachment.clone())
							}
						});
					}
					let mut position = if active { self.position } else { 0.0 };
					ui.spacing_mut().slider_width = ui.available_width().max(24.0);
					let can_seek =
						duration > 0.0 && matches!(state, AudioState::Playing | AudioState::Paused);
					if voice {
						if waveform(
							ui,
							&attachment.waveform,
							&mut position,
							duration,
							can_seek,
							&colors,
						) {
							self.command = Some(AudioCommand::Seek(position));
						}
					} else {
						let seek = ui.add_enabled(
							can_seek,
							egui::Slider::new(&mut position, 0.0..=duration.max(1.0))
								.show_value(false)
								.trailing_fill(true)
								.handle_shape(egui::style::HandleShape::Circle),
						);
						seek.widget_info(|| egui::WidgetInfo::slider(can_seek, position, "Seek"));
						let seek = seek.on_hover_text("Seek");
						if seek.changed() {
							self.command = Some(AudioCommand::Seek(position));
						}
					}
				});
				ui.horizontal(|ui| {
					let position = if active { self.position } else { 0.0 };
					let duration_label = if duration > 0.0 {
						timestamp(duration)
					} else {
						"--:--".into()
					};
					ui.label(
						egui::RichText::new(if voice {
							if duration > 0.0 {
								timestamp((duration - position).max(0.0))
							} else {
								"--:--".into()
							}
						} else {
							format!("{} / {duration_label}", timestamp(position))
						})
						.size(11.0)
						.color(colors.muted),
					);
					if !voice {
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							ui.spacing_mut().item_spacing.x = 6.0;
							ui.spacing_mut().slider_width = 52.0;
							let volume = ui.add(
								egui::Slider::new(&mut self.volume, 0.0..=1.0)
									.show_value(false)
									.trailing_fill(true)
									.handle_shape(egui::style::HandleShape::Circle),
							);
							volume.widget_info(|| {
								egui::WidgetInfo::slider(
									ui.is_enabled(),
									self.volume as f64,
									"Volume",
								)
							});
							if volume
								.on_hover_text(format!("Volume: {:.0}%", self.volume * 100.0))
								.changed()
							{
								self.command = Some(AudioCommand::Volume(self.volume));
							}
							crate::icons::inline(
								ui,
								crate::icons::Icon::Speaker,
								16.0,
								colors.muted,
							);
						});
					}
				});
				match state {
					AudioState::Loading => {
						ui.small("Loading audio…");
					}
					AudioState::Failed(error) => {
						ui.colored_label(colors.danger, error);
					}
					_ => {}
				}
			});
		if ui.is_rect_visible(card.response.rect)
			&& self.active.as_ref().is_some_and(|(channel, id, file)| {
				*channel == message.channel && *id == message.id && file == attachment
			}) {
			self.seen = true;
			if matches!(state, AudioState::Playing | AudioState::Loading) {
				ui.ctx()
					.request_repaint_after(std::time::Duration::from_millis(100));
			}
		}
		card.response
	}
}
fn waveform(
	ui: &mut egui::Ui,
	samples: &[u8],
	position: &mut f64,
	duration: f64,
	enabled: bool,
	colors: &crate::design::Palette,
) -> bool {
	let enabled = enabled && ui.is_enabled();
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(ui.available_width().max(1.0), 32.0),
		if enabled {
			egui::Sense::click_and_drag()
		} else {
			egui::Sense::hover()
		},
	);
	let before = *position;
	if enabled {
		if (response.clicked() || response.dragged())
			&& let Some(pointer) = response.interact_pointer_pos()
		{
			*position =
				f64::from(((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0)) * duration;
		}
		if response.has_focus() {
			ui.input_mut(|input| {
				for (key, value) in [
					(egui::Key::ArrowLeft, *position - 1.0),
					(egui::Key::ArrowRight, *position + 1.0),
					(egui::Key::Home, 0.0),
					(egui::Key::End, duration),
				] {
					if input.consume_key(egui::Modifiers::NONE, key) {
						*position = value.clamp(0.0, duration);
					}
				}
			});
		}
	}
	let bars = ((rect.width() / 4.0).floor() as usize).clamp(1, 80);
	for index in 0..bars {
		let start = index * samples.len() / bars;
		let end = ((index + 1) * samples.len() / bars)
			.max(start + 1)
			.min(samples.len());
		let amplitude = samples
			.get(start..end)
			.and_then(|part| part.iter().max())
			.copied()
			.unwrap_or(0);
		let height = 3.0 + f32::from(amplitude) / 255.0 * 25.0;
		let x = rect.left() + (index as f32 + 0.5) * rect.width() / bars as f32;
		let played = duration > 0.0 && (index as f64 / bars as f64) < *position / duration;
		ui.painter().rect_filled(
			egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(2.0, height)),
			1,
			if played {
				colors.text_strong
			} else {
				colors.muted.gamma_multiply(0.65)
			},
		);
	}
	if response.has_focus() {
		ui.painter().rect_stroke(
			rect,
			3,
			egui::Stroke::new(1.0, colors.accent),
			egui::StrokeKind::Inside,
		);
	}
	response.widget_info(|| {
		egui::WidgetInfo::slider(enabled && ui.is_enabled(), *position, "Seek voice message")
	});
	response.on_hover_text("Seek voice message");
	*position != before
}

fn timestamp(seconds: f64) -> String {
	let seconds = seconds.max(0.0) as u64;
	format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn audio_is_explicit_keyboard_operable_and_fits_narrow_cards() {
		let state = test_support::audio_demo_state();
		let mut message = state.timeline.iter().last().unwrap().clone();
		message.attachments.truncate(1);
		message.attachments[0].content_type = Some("text/plain".into());
		let file = &message.attachments[0];
		for (width, theme) in [
			(220.0, egui::Theme::Dark),
			(380.0, egui::Theme::Dark),
			(220.0, egui::Theme::Light),
			(380.0, egui::Theme::Light),
		] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.set_theme(theme);
			let mut audio = AudioUi::default();
			let mut images = crate::avatars::Avatars::default();
			let mut viewing = None;
			let mut opening = None;
			let mut download = crate::attachments::DownloadUi::default();
			let mut frame = |audio: &mut AudioUi, key: Option<egui::Key>| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width + 16.0, 300.0),
						)),
						events: key
							.into_iter()
							.map(|key| egui::Event::Key {
								key,
								physical_key: None,
								pressed: true,
								repeat: false,
								modifiers: egui::Modifiers::NONE,
							})
							.collect(),
						..Default::default()
					},
					|ui| {
						ui.set_width(width);
						ui.spacing_mut().item_spacing.x = 16.0;
						crate::attachments::show(
							ui,
							&message,
							&mut images,
							&mut viewing,
							&mut opening,
							&mut download,
							audio,
							&mut crate::video::VideoUi::default(),
							false,
							&mut crate::select::Surface::new(ui, "attachment-test"),
						);
						assert!(ui.min_rect().width() <= width + 2.0);
					},
				)
				.drop_without_applying_deltas();
			};
			frame(&mut audio, None);
			assert!(audio.active.is_none() && audio.command.is_none());
			for key in [egui::Key::Tab, egui::Key::Enter] {
				frame(&mut audio, Some(key));
			}
			assert!(matches!(audio.command.take(), Some(AudioCommand::Play(a)) if a == *file));
			assert!(audio.seen);
			audio.state = AudioState::Playing;
			audio.duration = 12.0;
			frame(&mut audio, Some(egui::Key::Enter));
			assert!(matches!(
				audio.command.take(),
				Some(AudioCommand::Pause(true))
			));
			audio.state = AudioState::Paused;
			frame(&mut audio, Some(egui::Key::Enter));
			assert!(matches!(
				audio.command.take(),
				Some(AudioCommand::Pause(false))
			));
			for key in [egui::Key::Tab, egui::Key::ArrowRight] {
				frame(&mut audio, Some(key));
			}
			assert!(matches!(audio.command.take(), Some(AudioCommand::Seek(value)) if value > 0.0));
			for key in [egui::Key::Tab, egui::Key::ArrowLeft] {
				frame(&mut audio, Some(key));
			}
			assert!(
				matches!(audio.command.take(), Some(AudioCommand::Volume(value)) if value < 1.0)
			);
			audio.stop();
			assert!(audio.active.is_none());
			assert!(matches!(audio.command, Some(AudioCommand::Stop)));
		}
		assert_eq!(timestamp(125.4), "2:05");
	}
}
