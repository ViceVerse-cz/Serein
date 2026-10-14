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
	/// Pitch-preserving playback speed.
	Speed(f32),
	Stop,
}

/// Discord's voice message speeds, in the order its pill cycles through them.
pub const SPEEDS: [f32; 4] = [1.0, 1.5, 2.0, 0.75];
pub struct AudioUi {
	pub active: Option<(Id, Id, Attachment)>,
	pub state: AudioState,
	pub position: f64,
	pub duration: f64,
	pub command: Option<AudioCommand>,
	pub seen: bool,
	pub volume: f32,
	/// Voice message playback speed, kept for the session like Discord's.
	pub speed: f32,
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
			speed: 1.0,
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
					let label = crate::i18n::translate(match state {
						AudioState::Loading => "audio-control-cancel",
						AudioState::Playing => "audio-control-pause",
						AudioState::Ended => "audio-control-replay",
						AudioState::Failed(_) => "audio-control-retry",
						_ => "audio-control-play",
					});
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
						egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label)
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
					if play.on_hover_text(&label).clicked() {
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
					// Buffering after a seek reports Loading; stay seekable so clicks are not dropped.
					let can_seek = duration > 0.0
						&& matches!(
							state,
							AudioState::Playing | AudioState::Paused | AudioState::Loading
						);
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
						seek.widget_info(|| {
							egui::WidgetInfo::slider(
								can_seek,
								position,
								crate::i18n::translate("audio-show-seek"),
							)
						});
						let seek = seek.on_hover_text(crate::i18n::translate("audio-show-seek"));
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
					if voice {
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							if let Some(volume) = volume_popup(
								ui,
								("voice-volume", message.id, attachment.id),
								self.volume,
								&colors,
							) {
								self.volume = volume;
								self.command = Some(AudioCommand::Volume(volume));
							}
							if let Some(speed) = speed_pill(ui, self.speed, &colors) {
								self.speed = speed;
								self.command = Some(AudioCommand::Speed(speed));
							}
						});
					} else {
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
									crate::i18n::translate("audio-show-volume"),
								)
							});
							if volume
								.on_hover_text(format!(
									"{}: {:.0}%",
									crate::i18n::translate("audio-show-volume"),
									self.volume * 100.0
								))
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
						ui.small(crate::i18n::translate("audio-show-loading-audio"));
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
/// The "1X" pill; each click moves to the next of Discord's speeds.
fn speed_pill(ui: &mut egui::Ui, speed: f32, colors: &crate::design::Palette) -> Option<f32> {
	let label = if speed.fract() == 0.0 {
		format!("{speed:.0}X")
	} else {
		format!("{speed}X")
	};
	let galley = ui.painter().layout_no_wrap(
		label.clone(),
		egui::FontId::new(11.0, crate::design::semibold_family(ui.ctx())),
		colors.text_strong,
	);
	let size = egui::vec2(galley.size().x + 14.0, 20.0);
	let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
	let fill = if response.hovered() || response.has_focus() {
		colors.hover
	} else {
		colors.muted.gamma_multiply(0.25)
	};
	ui.painter().rect_filled(rect, 5, fill);
	ui.painter().galley(
		rect.center() - galley.size() * 0.5,
		galley,
		colors.text_strong,
	);
	let name = crate::i18n::translate("audio-playback-speed");
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{name}: {label}"))
	});
	response.on_hover_text(name).clicked().then(|| {
		let index = SPEEDS.iter().position(|value| *value == speed).unwrap_or(0);
		SPEEDS[(index + 1) % SPEEDS.len()]
	})
}

/// The speaker icon; hovering it raises a vertical volume slider above, as on Discord.
fn volume_popup(
	ui: &mut egui::Ui,
	salt: impl std::hash::Hash + std::fmt::Debug,
	volume: f32,
	colors: &crate::design::Palette,
) -> Option<f32> {
	// Scoped to this card, so a message drawn twice in one frame cannot collide with itself.
	let id = ui.make_persistent_id(salt);
	let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::click());
	let tint = if response.hovered() {
		colors.text_strong
	} else {
		colors.muted
	};
	crate::icons::paint(
		ui.painter(),
		crate::icons::Icon::Speaker,
		rect.shrink(3.0),
		tint,
	);
	if volume == 0.0 {
		// Muted: struck through.
		ui.painter().line_segment(
			[
				rect.left_top() + egui::vec2(4.0, 4.0),
				rect.right_bottom() - egui::vec2(4.0, 4.0),
			],
			egui::Stroke::new(2.0, colors.danger),
		);
	}
	let name = crate::i18n::translate("audio-show-volume");
	response.widget_info(|| egui::WidgetInfo::slider(true, f64::from(volume), &name));
	let mut changed = None;
	// A click mutes or restores, like the official control.
	if response.clicked() {
		let restore = ui
			.ctx()
			.data(|d| d.get_temp::<f32>(id.with("restore")))
			.unwrap_or(1.0);
		let next = if volume > 0.0 { 0.0 } else { restore.max(0.05) };
		if volume > 0.0 {
			ui.ctx()
				.data_mut(|d| d.insert_temp(id.with("restore"), volume));
		}
		changed = Some(next);
	}
	let open_id = id.with("open");
	let mut open = ui
		.ctx()
		.data(|d| d.get_temp::<bool>(open_id))
		.unwrap_or(false);
	if response.hovered() || response.has_focus() {
		open = true;
	}
	if open {
		let mut value = changed.unwrap_or(volume);
		let mut dragging = false;
		let area = egui::Area::new(id.with("area"))
			.order(egui::Order::Foreground)
			// Sits right on the icon, so moving up from it never crosses a gap.
			.pivot(egui::Align2::CENTER_BOTTOM)
			.fixed_pos(rect.center_top())
			.show(ui.ctx(), |ui| {
				egui::Frame::new()
					.fill(colors.raised)
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(8)
					.inner_margin(egui::Margin::symmetric(8, 10))
					.show(ui, |ui| {
						ui.spacing_mut().slider_width = 84.0;
						ui.visuals_mut().selection.bg_fill = colors.accent;
						let slider = ui.add(
							egui::Slider::new(&mut value, 0.0..=1.0)
								.vertical()
								.show_value(false)
								.trailing_fill(true),
						);
						slider.widget_info(|| {
							egui::WidgetInfo::slider(true, f64::from(value), &name)
						});
						dragging = slider.dragged();
						if slider.changed() {
							changed = Some(value);
						}
					});
			});
		// Stays while the pointer is anywhere over the icon and slider, or a drag is under way.
		let pointer = ui.ctx().pointer_hover_pos();
		open = dragging
			|| response.has_focus()
			|| pointer.is_some_and(|pointer| {
				rect.union(area.response.rect).expand(6.0).contains(pointer)
			});
	}
	ui.ctx().data_mut(|d| d.insert_temp(open_id, open));
	if !open {
		response.on_hover_text(format!("{name}: {:.0}%", volume * 100.0));
	}
	changed
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
	let mut seek = false;
	if enabled {
		// Dragging only previews; each seek restarts decoding, so commit on click or release.
		if (response.clicked() || response.dragged() || response.drag_stopped())
			&& let Some(pointer) = response.interact_pointer_pos()
		{
			*position =
				f64::from(((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0)) * duration;
			seek = !response.dragged();
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
						seek = true;
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
		egui::WidgetInfo::slider(
			enabled && ui.is_enabled(),
			*position,
			crate::i18n::translate("audio-waveform-seek-voice-message"),
		)
	});
	response.on_hover_text(crate::i18n::translate("audio-waveform-seek-voice-message"));
	seek && *position != before
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
							crate::design::MessageCardSurface::Conversation,
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

	#[test]
	fn waveform_seeks_while_buffering_and_commits_drags_on_release() {
		let state = test_support::audio_demo_state();
		let message = state.timeline.iter().next().unwrap().clone();
		let file = message.attachments[0].clone();
		assert!(file.is_voice_message());
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut audio = AudioUi {
			active: Some((message.channel, message.id, file.clone())),
			// A seek or network stall reports Loading until output resumes.
			state: AudioState::Loading,
			..Default::default()
		};
		let frame = |audio: &mut AudioUi, events: Vec<egui::Event>| {
			let mut card = egui::Rect::NOTHING;
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 200.0),
					)),
					events,
					..Default::default()
				},
				|ui| card = audio.show(ui, &message, &file).rect,
			)
			.drop_without_applying_deltas();
			card
		};
		let card = frame(&mut audio, Vec::new());
		// Border plus padding, then the 32-point play button and row spacing.
		let (left, right) = (card.left() + 55.0, card.right() - 13.0);
		let at = |fraction: f32| egui::pos2(left + (right - left) * fraction, card.top() + 29.0);
		let button = |pos, pressed| egui::Event::PointerButton {
			pos,
			button: egui::PointerButton::Primary,
			pressed,
			modifiers: egui::Modifiers::NONE,
		};
		let seek = |audio: &mut AudioUi| match audio.command.take() {
			Some(AudioCommand::Seek(seconds)) => Some(seconds),
			None => None,
			_ => panic!("unexpected audio command"),
		};
		frame(
			&mut audio,
			vec![egui::Event::PointerMoved(at(0.5)), button(at(0.5), true)],
		);
		frame(&mut audio, vec![button(at(0.5), false)]);
		let seconds = seek(&mut audio).expect("click seeks while buffering");
		assert!((seconds - 1.5).abs() < 0.15, "{seconds}");
		frame(
			&mut audio,
			vec![egui::Event::PointerMoved(at(0.25)), button(at(0.25), true)],
		);
		for fraction in [0.4, 0.6, 0.75] {
			frame(&mut audio, vec![egui::Event::PointerMoved(at(fraction))]);
			assert_eq!(
				seek(&mut audio),
				None,
				"dragging previews without restarting"
			);
		}
		frame(&mut audio, vec![button(at(0.75), false)]);
		let seconds = seek(&mut audio).expect("release commits the drag");
		assert!((seconds - 2.25).abs() < 0.15, "{seconds}");
	}

	#[test]
	fn voice_messages_cycle_speeds_and_raise_a_volume_slider_on_hover() {
		let state = test_support::audio_demo_state();
		let message = state.timeline.iter().next().unwrap().clone();
		let file = message.attachments[0].clone();
		assert!(file.is_voice_message());
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut audio = AudioUi::default();
		let frame = |audio: &mut AudioUi, events: Vec<egui::Event>| {
			let mut pill = None;
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 300.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					ui.add_space(150.0);
					audio.show(ui, &message, &file);
				},
			);
			for shape in &output.shapes {
				if let egui::Shape::Text(text) = &shape.shape
					&& text.galley.job.text.ends_with('X')
				{
					pill = Some(text.galley.rect.translate(text.pos.to_vec2()));
				}
			}
			output.drop_without_applying_deltas();
			pill.expect("speed pill painted")
		};
		let click = |pos: egui::Pos2| {
			let mut events = vec![egui::Event::PointerMoved(pos)];
			for pressed in [true, false] {
				events.push(egui::Event::PointerButton {
					pos,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				});
			}
			events
		};
		let mut pill = frame(&mut audio, vec![]);
		for expected in [1.5, 2.0, 0.75, 1.0] {
			pill = frame(&mut audio, click(pill.center()));
			match audio.command.take() {
				Some(AudioCommand::Speed(speed)) => assert_eq!(speed, expected),
				_ => panic!("the pill sets speed {expected}"),
			}
			frame(&mut audio, vec![]);
		}
		assert_eq!(audio.speed, 1.0);
		// The speaker sits right of the pill; hovering it raises the vertical slider right
		// above it, and the slider stays while the pointer moves up onto it.
		let speaker = egui::pos2(pill.right() + 8.0 + 7.0 + 11.0, pill.center().y);
		let slider = |ctx: &egui::Context| {
			ctx.memory(|memory| {
				memory
					.areas()
					.visible_layer_ids()
					.into_iter()
					.filter(|layer| layer.order == egui::Order::Foreground)
					.filter_map(|layer| memory.area_rect(layer.id))
					.find(|rect| rect.bottom() <= speaker.y && rect.height() > 60.0)
			})
		};
		frame(&mut audio, vec![egui::Event::PointerMoved(speaker)]);
		frame(&mut audio, vec![egui::Event::PointerMoved(speaker)]);
		let shown = slider(&ctx).expect("slider shown above the speaker");
		for y in [shown.bottom() + 2.0, shown.center().y, shown.top() + 6.0] {
			frame(
				&mut audio,
				vec![egui::Event::PointerMoved(egui::pos2(shown.center().x, y))],
			);
			assert!(slider(&ctx).is_some(), "still open at {y}");
		}
		let visible = |ctx: &egui::Context| slider(ctx).is_some();
		assert!(visible(&ctx));
		frame(
			&mut audio,
			vec![egui::Event::PointerMoved(egui::pos2(5.0, 295.0))],
		);
		frame(&mut audio, vec![]);
		frame(&mut audio, vec![]);
		assert!(!visible(&ctx), "leaving both closes it");
	}

	/// The timeline can draw one message twice in a frame; its volume popup must not clash.
	#[test]
	fn a_voice_message_drawn_twice_has_no_widget_id_clash() {
		let state = test_support::audio_demo_state();
		let message = state.timeline.iter().next().unwrap().clone();
		let file = message.attachments[0].clone();
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut audio = AudioUi::default();
		let mut speaker = None;
		for _ in 0..4 {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(400.0, 600.0),
					)),
					// Hover the first card's speaker so its popup opens.
					events: speaker
						.map(|pos| vec![egui::Event::PointerMoved(pos)])
						.unwrap_or_default(),
					..Default::default()
				},
				|ui| {
					ui.add_space(150.0);
					for copy in 0..2 {
						ui.push_id(copy, |ui| audio.show(ui, &message, &file));
					}
				},
			);
			for shape in &output.shapes {
				if let egui::Shape::Text(text) = &shape.shape {
					let label = &text.galley.job.text;
					assert!(!label.contains("widget ID"), "{label}");
					if speaker.is_none() && label.ends_with('X') {
						let pill = text.galley.rect.translate(text.pos.to_vec2());
						speaker = Some(egui::pos2(pill.right() + 26.0, pill.center().y));
					}
				}
			}
			output.drop_without_applying_deltas();
		}
		assert!(speaker.is_some());
	}
}
