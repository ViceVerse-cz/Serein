//! Screen selection is a local intent; the desktop owns capture and stream credentials.
use client_core::{
	State,
	screen::{AudioScope, RequestSettings, Resolution, Source, SourceId},
	voice::Phase,
};
use model::{Id, voice_settings::VideoResolution};

fn audio_control(source: Option<SourceId>) -> (&'static str, &'static str) {
	match source.map(SourceId::audio_scope) {
		Some(AudioScope::Application) => (
			"screen-audio-application",
			"screen-audio-application-description",
		),
		Some(AudioScope::System) => (
			"screen-body-share-system-audio",
			"screen-audio-system-description",
		),
		Some(AudioScope::Pending) | None => (
			"screen-audio-selected-source",
			"screen-audio-selected-source-description",
		),
	}
}

pub enum Request {
	Start(RequestSettings),
	Stop,
}

pub struct ScreenUi {
	pub context: Option<(u64, Id, u64)>,
	pub open: bool,
	pub sources: Vec<Source>,
	pub selected: Option<SourceId>,
	pub refresh_requested: bool,
	pub request: Option<Request>,
	pub busy: bool,
	pub status: &'static str,
	pub capture_status: Option<&'static str>,
	pub adapting: bool,
	pub supported: bool,
	pub preview: Option<egui::TextureHandle>,
	/// Native visibility (including tray hiding), published by the desktop owner.
	pub preview_window_visible: bool,
	preview_painted_at: Option<u64>,
	resolution: Resolution,
	fps: u32,
	cursor: bool,
	audio: bool,
}
impl Default for ScreenUi {
	fn default() -> Self {
		Self {
			context: None,
			open: false,
			sources: Vec::new(),
			selected: None,
			refresh_requested: false,
			request: None,
			busy: false,
			status: "",
			capture_status: None,
			adapting: false,
			supported: false,
			preview: None,
			preview_window_visible: true,
			preview_painted_at: None,
			resolution: Resolution::Preset(if cfg!(target_os = "linux") {
				VideoResolution::P720
			} else {
				VideoResolution::P1080
			}),
			fps: 30,
			cursor: true,
			audio: cfg!(target_os = "macos"),
		}
	}
}
impl ScreenUi {
	/// Local presentation demand is independent of whether the share has viewers.
	pub fn preview_requested(&self, ctx: &egui::Context) -> bool {
		self.preview_window_visible
			&& ctx.input(|input| {
				input.viewport().visible() != Some(false) && !input.viewport().close_requested()
			}) && self.preview_demand(ctx)
	}

	/// The tile was painted in this pass or the immediately preceding one. The
	/// desktop polls before painting, so a single previous frame is intentional.
	pub fn preview_demand(&self, ctx: &egui::Context) -> bool {
		let current = ctx.cumulative_frame_nr();
		self.preview_painted_at
			.is_some_and(|painted| painted == current || painted.checked_add(1) == Some(current))
	}

	pub(crate) fn preview_painted(&mut self, ctx: &egui::Context) {
		let newly_visible = !self.preview_demand(ctx);
		self.preview_painted_at = Some(ctx.cumulative_frame_nr());
		if newly_visible {
			// Let the desktop publish demand even if this newly visible tile has
			// otherwise static content and would not schedule another frame.
			ctx.request_repaint();
		}
	}

	pub(crate) fn launch(&mut self, state: &State) {
		let Some(call) = &state.voice.active else {
			return;
		};
		if self.busy {
			self.request = Some(Request::Stop);
			return;
		}
		self.context = Some((state.generation, call.channel, call.request));
		self.open = true;
		self.selected = None;
		self.sources.clear();
		if state.demo {
			self.sources = if cfg!(target_os = "macos") {
				vec![Source {
					id: SourceId::SystemPicker,
					name: "Choose with the macOS system picker".into(),
					dimensions: None,
				}]
			} else {
				vec![
					Source {
						id: SourceId::Display(1),
						name: "Display 1 · Synthetic preview".into(),
						dimensions: Some((3840, 2160)),
					},
					Source {
						id: SourceId::Window(2),
						name: "Project notes · Synthetic window".into(),
						dimensions: Some((1600, 1000)),
					},
				]
			};
			self.selected = self.sources.first().map(|source| source.id);
			self.status = "screen-source-status-offline";
		} else {
			self.refresh_requested = true;
			self.status = "screen-source-status-loading";
		}
	}
	fn resolution_choice(
		&mut self,
		ui: &mut egui::Ui,
		mode: Resolution,
		label: &str,
	) -> egui::Response {
		let allowed = self
			.selected_dimensions()
			.is_none_or(|size| mode.dimensions(size).is_some());
		let response = ui
			.add_enabled_ui(allowed, |ui| segment(ui, label, self.resolution == mode))
			.inner;
		if response.clicked() {
			self.resolution = mode;
		}
		response
	}
	fn selected_dimensions(&self) -> Option<(u32, u32)> {
		self.sources
			.iter()
			.find(|source| Some(source.id) == self.selected)?
			.dimensions
	}
	fn limit_resolution(&mut self) {
		if let Some(size) = self.selected_dimensions() {
			self.resolution = self.resolution.fitting(size);
		}
	}
	pub fn replace_sources(&mut self, sources: Vec<Source>) {
		self.sources = sources;
		self.selected = self
			.selected
			.filter(|id| self.sources.iter().any(|source| source.id == *id))
			.or_else(|| self.sources.first().map(|source| source.id));
		self.limit_resolution();
	}
	fn settings(&self) -> Option<RequestSettings> {
		let source = self
			.sources
			.iter()
			.find(|source| Some(source.id) == self.selected)?;
		if source
			.dimensions
			.is_some_and(|size| self.resolution.dimensions(size).is_none())
		{
			return None;
		}
		let settings = RequestSettings {
			source: source.id,
			resolution: self.resolution,
			fps: self.fps,
			cursor: self.cursor,
			audio: self.audio,
		};
		settings.valid().then_some(settings)
	}
	pub(super) fn show(&mut self, ctx: &egui::Context, state: &State) {
		let current = state
			.voice
			.active
			.as_ref()
			.filter(|c| c.phase != Phase::Failed)
			.map(|c| (state.generation, c.channel, c.request));
		if self.context.is_some() && current != self.context {
			self.preview = None;
			self.open = false;
			self.sources.clear();
			self.selected = None;
			self.request = None;
			self.refresh_requested = false;
		}
		if !self.open {
			return;
		}
		let mut cancel = false;
		let mut share = false;
		let response = crate::dialog::Dialog::new(
			"screen-share-settings",
			crate::i18n::translate("screen-show-share-your-screen"),
		)
		.subtitle(crate::i18n::translate(
			"screen-show-choose-what-people-in-this-call-can-see",
		))
		.width(460.0)
		.show(ctx, |d| {
			d.scroll(260.0, |ui| self.body(ui, state));
			d.footer(|ui| {
				let allowed = !state.demo
					&& self.supported
					&& !self.busy && self.settings().is_some()
					&& state.voice.active.as_ref().is_some_and(|call| {
						matches!(call.phase, Phase::Connected | Phase::Waiting)
							&& state.can_stream(call.channel)
					});
				ui.add_enabled_ui(allowed, |ui| {
					share = crate::dialog::action(
						ui,
						"screen-show-share-screen",
						crate::dialog::Action::Primary,
					)
					.clicked();
				});
				cancel |=
					crate::dialog::action(ui, "screen-show-cancel", crate::dialog::Action::Neutral)
						.clicked();
			});
		});
		cancel |= response.close;
		if share && !cancel {
			self.request = self.settings().map(Request::Start);
		}
		if cancel || share {
			self.open = false;
		}
	}

	/// Source list, then local output presets and capture options.
	fn body(&mut self, ui: &mut egui::Ui, state: &State) {
		self.limit_resolution();
		let colors = crate::design::palette(ui);
		ui.horizontal(|ui| {
			ui.label(crate::design::eyebrow(
				ui,
				crate::i18n::translate("screen-body-screen-or-window"),
				colors.muted,
			));
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				if ui
					.add_enabled_ui(!state.demo && !cfg!(target_os = "linux"), |ui| {
						crate::design::text_action(
							ui,
							&crate::i18n::translate("screen-body-refresh"),
						)
					})
					.inner
					.clicked()
				{
					self.sources.clear();
					self.refresh_requested = true;
					self.status = "screen-source-status-loading";
				}
			});
		});
		ui.add_space(6.0);
		egui::ScrollArea::vertical()
			.id_salt("screen-sources")
			.max_height(196.0)
			.show(ui, |ui| self.source_list(ui));
		self.limit_resolution();
		ui.add_space(14.0);
		ui.label(crate::design::eyebrow(
			ui,
			crate::i18n::translate("screen-body-quality"),
			colors.muted,
		));
		ui.add_space(6.0);
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
			let source_label = crate::i18n::translate("screen-body-source-resolution");
			let source_label = self
				.selected_dimensions()
				.map_or(source_label.clone(), |(w, h)| {
					format!("{source_label} · {w} × {h}")
				});
			self.resolution_choice(ui, Resolution::Source, &source_label);
			for resolution in VideoResolution::ALL {
				let mode = Resolution::Preset(resolution);
				let response = self.resolution_choice(ui, mode, resolution.label());
				if let Some((width, height)) = self
					.selected_dimensions()
					.and_then(|size| mode.dimensions(size))
				{
					response.on_hover_text(format!("{width} × {height}"));
				}
			}
		});
		ui.add_space(8.0);
		ui.label(crate::design::eyebrow(
			ui,
			crate::i18n::translate("screen-body-frame-rate"),
			colors.muted,
		));
		ui.add_space(6.0);
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
			for fps in [15, 30, 60] {
				if segment(ui, &format!("{fps} fps"), self.fps == fps).clicked() {
					self.fps = fps;
				}
			}
		});
		ui.add_space(4.0);
		ui.add(
			egui::Label::new(
				egui::RichText::new(crate::i18n::translate(
					"screen-body-quality-selection-does-not-require-nitro",
				))
				.size(12.0)
				.color(colors.muted),
			)
			.wrap(),
		);
		ui.add_space(10.0);
		crate::design::hint(ui, "screen-adaptive-quality-hint");
		ui.add(
			egui::Label::new(
				egui::RichText::new(crate::i18n::translate("screen-body-resolution-description"))
					.size(12.0)
					.color(colors.muted),
			)
			.wrap(),
		);
		crate::design::switch(
			ui,
			"screen-body-show-cursor",
			Some("screen-body-include-the-pointer-in-the-shared-video"),
			&mut self.cursor,
		);
		if self.supported {
			ui.add_space(6.0);
			let (label, description) = audio_control(self.selected);
			crate::design::switch(ui, label, Some(description), &mut self.audio);
		}
		ui.add_space(4.0);
		ui.add(
			egui::Label::new(
				egui::RichText::new(crate::i18n::translate(
					"screen-body-your-call-microphone-keeps-its-current-settings",
				))
				.size(12.0)
				.color(colors.muted),
			)
			.wrap(),
		);
		if !self.status.is_empty() {
			ui.add_space(10.0);
			egui::Frame::new()
				.fill(colors.base)
				.corner_radius(8)
				.inner_margin(egui::Margin::symmetric(12, 10))
				.show(ui, |ui| {
					ui.set_width(ui.available_width());
					ui.add(
						egui::Label::new(
							egui::RichText::new(crate::i18n::translate_if_key(self.status))
								.size(12.0)
								.color(colors.muted),
						)
						.wrap(),
					);
				});
		}
	}

	/// Selectable source rows; the caller bounds the height so options stay visible.
	fn source_list(&mut self, ui: &mut egui::Ui) {
		let colors = crate::design::palette(ui);
		if self.sources.is_empty() {
			egui::Frame::new()
				.fill(colors.base)
				.corner_radius(8)
				.inner_margin(egui::Margin::symmetric(12, 14))
				.show(ui, |ui| {
					ui.set_width(ui.available_width());
					ui.add(
						egui::Label::new(
							egui::RichText::new(crate::i18n::translate(
								"screen-source-list-no-screens-or-windows-are-available-yet",
							))
							.size(13.0)
							.color(colors.muted),
						)
						.wrap(),
					);
				});
		}
		for source in &self.sources {
			let selected = self.selected == Some(source.id);
			let (rect, response) = ui
				.allocate_exact_size(egui::vec2(ui.available_width(), 48.0), egui::Sense::click());
			if selected {
				crate::design::selection_surface(ui, rect);
			} else {
				let fill = if response.hovered() || response.has_focus() {
					colors.hover
				} else {
					colors.base
				};
				ui.painter().rect_filled(rect, 8, fill);
			}
			crate::design::focus_outline(ui, &response, 8);
			let display = matches!(source.id, SourceId::Display(_) | SourceId::X11Desktop);
			crate::icons::paint(
				ui.painter(),
				if display {
					crate::icons::Icon::Television
				} else {
					crate::icons::Icon::ScreenShare
				},
				egui::Rect::from_center_size(
					egui::pos2(rect.left() + 26.0, rect.center().y),
					egui::Vec2::splat(20.0),
				),
				if selected {
					colors.accent
				} else {
					colors.muted
				},
			);
			let text_left = (rect.left() + 46.0).min(rect.right());
			let text_width = (rect.right() - 36.0 - text_left).max(0.0);
			let source_name = if source.id == SourceId::SystemPicker {
				crate::i18n::translate("screen-macos-system-picker")
			} else {
				source.name.clone()
			};
			// Both labels occupy one line inside the fixed-height row, even for
			// long window titles and large text scales. Keep the full name in the
			// accessible label and hover text below.
			let layout = |text: String, font: egui::FontId, color: egui::Color32| {
				let mut job = egui::text::LayoutJob::simple_singleline(text, font, color);
				job.wrap.max_width = text_width;
				job.wrap.max_rows = 1;
				job.wrap.break_anywhere = true;
				job.wrap.overflow_character = Some('…');
				ui.painter().layout_job(job)
			};
			let name = layout(
				source_name.clone(),
				egui::FontId::new(14.0, crate::design::medium_family(ui.ctx())),
				colors.text_strong,
			);
			let kind = layout(
				match source.id {
					SourceId::Display(_) | SourceId::X11Desktop => {
						crate::i18n::translate("screen-source-kind-screen")
					}
					SourceId::Window(_) => crate::i18n::translate("screen-source-kind-window"),
					SourceId::SystemPicker => {
						crate::i18n::translate("screen-macos-system-picker-kind")
					}
					#[allow(unreachable_patterns)] // Portal may be absent outside Linux.
					_ => crate::i18n::translate("screen-source-kind-portal"),
				},
				egui::FontId::proportional(11.0),
				colors.muted,
			);
			let total = name.size().y + kind.size().y;
			let mut y = rect.center().y - total * 0.5;
			let painter = ui.painter().with_clip_rect(rect);
			painter.galley(egui::pos2(text_left, y), name, colors.text_strong);
			y += total - kind.size().y;
			painter.galley(egui::pos2(text_left, y), kind, colors.muted);
			if selected {
				crate::icons::paint(
					ui.painter(),
					crate::icons::Icon::Check,
					egui::Rect::from_center_size(
						egui::pos2(rect.right() - 20.0, rect.center().y),
						egui::Vec2::splat(16.0),
					),
					colors.accent,
				);
			}
			response.widget_info(|| {
				egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, &source_name)
			});
			let response = response.on_hover_text(&source_name);
			if response.clicked() {
				self.selected = Some(source.id);
			}
			ui.add_space(4.0);
		}
	}
}

/// Compact segmented choice used by the quality and frame-rate rows.
fn segment(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
	let label = crate::i18n::translate_if_key(label);
	let colors = crate::design::palette(ui);
	let galley = ui.painter().layout_no_wrap(
		label.to_owned(),
		egui::FontId::new(13.0, crate::design::medium_family(ui.ctx())),
		colors.text_strong,
	);
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(galley.size().x + 24.0, 32.0),
		egui::Sense::click(),
	);
	let fill = if selected {
		colors.accent
	} else if response.hovered() || response.has_focus() {
		colors.hover
	} else {
		colors.base
	};
	ui.painter().rect_filled(rect, 8, fill);
	let color = if !ui.is_enabled() {
		colors.muted
	} else if selected {
		colors.accent_text
	} else {
		colors.text
	};
	ui.painter().galley_with_override_text_color(
		egui::pos2(
			rect.center().x - galley.size().x * 0.5,
			rect.center().y - galley.size().y * 0.5,
		),
		galley,
		color,
	);
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::Role::RadioButton, ui.is_enabled(), selected, &label)
	});
	response
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn long_source_names_stay_inside_rows_and_keep_full_accessible_names() {
		let long_name = "A very long window title with many words and details ".repeat(8);
		for width in [120.0, 220.0] {
			for scale in [1.0, 2.0] {
				let ctx = egui::Context::default();
				ctx.enable_accesskit();
				ctx.set_pixels_per_point(scale);
				let mut picker = ScreenUi::default();
				picker.replace_sources(vec![
					Source {
						id: SourceId::Window(1),
						name: long_name.clone(),
						dimensions: Some((1920, 1080)),
					},
					Source {
						id: SourceId::Window(2),
						name: "Second window".into(),
						dimensions: Some((1600, 900)),
					},
				]);
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(400.0, 300.0),
						)),
						focused: true,
						..Default::default()
					},
					|ui| {
						ui.set_width(width);
						picker.source_list(ui);
					},
				);
				let label = output
					.shapes
					.iter()
					.find(
						|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == long_name),
					)
					.expect("source name is rendered");
				let egui::Shape::Text(text) = &label.shape else {
					unreachable!()
				};
				assert_eq!(
					text.galley.rows.len(),
					1,
					"source names must not wrap into adjacent rows"
				);
				assert!(
					label.clip_rect.height() <= 48.0,
					"source text must be clipped to its row"
				);
				assert!(
					output
						.platform_output
						.accesskit_update
						.as_ref()
						.unwrap()
						.nodes
						.iter()
						.any(|(_, node)| node.label() == Some(long_name.as_str())),
					"the full source name remains accessible"
				);
				let next = output.shapes.iter().find(|shape| {
					matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Second window")
				}).expect("second source is rendered").clip_rect.center();
				output.drop_without_applying_deltas();
				for pressed in [true, false] {
					ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(400.0, 300.0),
							)),
							events: vec![
								egui::Event::PointerMoved(next),
								egui::Event::PointerButton {
									pos: next,
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE,
								},
							],
							focused: true,
							..Default::default()
						},
						|ui| {
							ui.set_width(width);
							picker.source_list(ui);
						},
					)
					.drop_without_applying_deltas();
				}
				assert_eq!(picker.selected, Some(SourceId::Window(2)));
			}
		}
	}

	#[test]
	fn source_status_translates_known_keys_and_preserves_backend_errors() {
		crate::i18n::set_current(crate::i18n::Language::English);
		let state = test_support::demo_state();
		for (status, expected) in [
			(
				"screen-source-status-loading",
				"Looking for screens and windows…",
			),
			(
				"screen-source-status-empty",
				"No shareable screens or windows were found",
			),
			("A backend error", "A backend error"),
		] {
			let ctx = egui::Context::default();
			let mut picker = ScreenUi {
				status,
				..Default::default()
			};
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(600.0, 1000.0),
					)),
					..Default::default()
				},
				|ui| picker.body(ui, &state),
			);
			assert!(
				output.shapes.iter().any(|shape| matches!(
					&shape.shape, egui::Shape::Text(text) if text.galley.job.text == expected
				)),
				"missing rendered status: {expected}"
			);
			output.drop_without_applying_deltas();
		}
	}
	#[test]
	fn audio_controls_describe_the_selected_source_without_widening_picker_scope() {
		assert_eq!(
			audio_control(Some(SourceId::Window(7))).0,
			"screen-audio-application"
		);
		for source in [SourceId::Display(1), SourceId::X11Desktop] {
			assert_eq!(
				audio_control(Some(source)).0,
				"screen-body-share-system-audio"
			);
		}
		for source in [Some(SourceId::Portal), Some(SourceId::SystemPicker), None] {
			assert_eq!(audio_control(source).0, "screen-audio-selected-source");
		}
	}

	fn source(id: u64, size: Option<(u32, u32)>) -> Source {
		Source {
			id: SourceId::Display(id),
			name: "Synthetic display".into(),
			dimensions: size,
		}
	}
	#[test]
	fn preview_demand_expires_when_the_tile_stops_painting_and_recovers() {
		let ctx = egui::Context::default();
		let mut picker = ScreenUi::default();
		assert!(!picker.preview_demand(&ctx));
		assert!(!picker.preview_requested(&ctx));
		picker.preview_window_visible = false;
		assert!(!picker.preview_requested(&ctx));
		picker.preview_window_visible = true;
		picker.preview = Some(ctx.load_texture(
			"synthetic-screen",
			egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
			Default::default(),
		));
		assert!(!picker.preview_requested(&ctx));
		ctx.run_ui(Default::default(), |ui| picker.preview_painted(ui.ctx()))
			.drop_without_applying_deltas();
		assert!(picker.preview_demand(&ctx));
		assert!(picker.preview_requested(&ctx));
		for (minimized, occluded) in [(true, false), (false, true)] {
			let mut input = egui::RawInput::default();
			let viewport = input.viewports.get_mut(&input.viewport_id).unwrap();
			viewport.minimized = Some(minimized);
			viewport.occluded = Some(occluded);
			ctx.run_ui(input, |_| {
				assert!(!picker.preview_requested(&ctx));
			})
			.drop_without_applying_deltas();
		}
		ctx.run_ui(Default::default(), |_| {})
			.drop_without_applying_deltas();
		assert!(!picker.preview_demand(&ctx));
		ctx.run_ui(Default::default(), |ui| picker.preview_painted(ui.ctx()))
			.drop_without_applying_deltas();
		assert!(picker.preview_demand(&ctx));
	}

	#[test]
	fn oversized_resolution_refuses_pointer_and_keyboard_activation() {
		for dark in [false, true] {
			let context = egui::Context::default();
			context.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut picker = ScreenUi::default();
			picker.replace_sources(vec![source(1, Some((1920, 1080)))]);
			picker.resolution = Resolution::Preset(VideoResolution::P1080);
			let mode = Resolution::Preset(VideoResolution::P2160);
			let mut center = egui::Pos2::ZERO;
			context
				.run_ui(Default::default(), |ui| {
					let response = picker.resolution_choice(ui, mode, "4K");
					assert!(!response.enabled());
					center = response.rect.center();
					response.request_focus();
				})
				.drop_without_applying_deltas();
			for events in [
				vec![egui::Event::Key {
					key: egui::Key::Enter,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}],
				vec![
					egui::Event::PointerMoved(center),
					egui::Event::PointerButton {
						pos: center,
						button: egui::PointerButton::Primary,
						pressed: true,
						modifiers: egui::Modifiers::NONE,
					},
				],
				vec![egui::Event::PointerButton {
					pos: center,
					button: egui::PointerButton::Primary,
					pressed: false,
					modifiers: egui::Modifiers::NONE,
				}],
			] {
				context
					.run_ui(
						egui::RawInput {
							events,
							..Default::default()
						},
						|ui| {
							assert!(!picker.resolution_choice(ui, mode, "4K").clicked());
						},
					)
					.drop_without_applying_deltas();
				assert_eq!(
					picker.resolution,
					Resolution::Preset(VideoResolution::P1080)
				);
			}
		}
	}

	#[test]
	fn settings_require_a_current_source_and_keep_camera_and_fps_defaults() {
		let mut picker = ScreenUi::default();
		assert!(picker.settings().is_none());
		assert_eq!(picker.fps, 30);
		let expected = if cfg!(target_os = "linux") {
			VideoResolution::P720
		} else {
			VideoResolution::P1080
		};
		assert_eq!(picker.resolution, Resolution::Preset(expected));
		picker.replace_sources(vec![source(1, Some((7680, 4320)))]);
		for preset in VideoResolution::ALL {
			picker.resolution = Resolution::Preset(preset);
			let settings = picker.settings().unwrap().resolve((7680, 4320)).unwrap();
			assert_eq!((settings.width, settings.height), preset.dimensions());
		}
		picker.fps = 60;
		assert_eq!(
			picker
				.settings()
				.unwrap()
				.resolve((7680, 4320))
				.unwrap()
				.bit_rate(),
			24_000_000
		);
		picker.audio = true;
		assert!(picker.settings().unwrap().audio);
		picker.sources.clear();
		assert!(picker.settings().is_none());
	}
	#[test]
	fn switching_and_refreshing_preserve_the_source_and_limit_quality() {
		let mut picker = ScreenUi::default();
		picker.replace_sources(vec![
			source(1, Some((6144, 2560))),
			source(2, Some((1920, 1080))),
		]);
		picker.resolution = Resolution::Preset(VideoResolution::P2160);
		assert!(picker.settings().is_some());
		picker.selected = Some(SourceId::Display(2));
		assert!(picker.settings().is_none()); // An oversized request cannot leave the UI.
		picker.limit_resolution();
		assert_eq!(
			picker.resolution,
			Resolution::Preset(VideoResolution::P1080)
		);
		picker.replace_sources(vec![
			source(2, Some((1080, 1920))),
			source(1, Some((6144, 2560))),
		]);
		assert_eq!(picker.selected, Some(SourceId::Display(2)));
		let output = picker.settings().unwrap().resolve((1080, 1920)).unwrap();
		assert_eq!((output.width, output.height), (1080, 1920));
		picker.replace_sources(vec![source(3, Some((320, 240)))]);
		assert_eq!(picker.selected, Some(SourceId::Display(3)));
		assert_eq!(picker.resolution, Resolution::Source);
		picker.replace_sources(vec![]);
		assert!(picker.settings().is_none());
	}
	#[test]
	fn unknown_picker_geometry_is_deferred_and_source_uses_native_pixels() {
		let mut picker = ScreenUi::default();
		picker.replace_sources(vec![source(1, None)]);
		picker.resolution = Resolution::Source;
		let request = picker.settings().unwrap();
		let output = request.resolve((6144, 2560)).unwrap();
		assert_eq!((output.width, output.height), (6144, 2560));
		picker.replace_sources(vec![source(1, Some((6144, 2560)))]);
		assert_eq!(picker.resolution, Resolution::Source);
	}
}
