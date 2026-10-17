//! Screen selection is a local intent; the desktop owns capture and stream credentials.
use client_core::{
	State,
	screen::{RequestSettings, Resolution, Source, SourceId},
	voice::Phase,
};
use model::{Id, voice_settings::VideoResolution};

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
	pub supported: bool,
	pub preview: Option<egui::TextureHandle>,
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
			supported: false,
			preview: None,
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
			self.status = "Offline preview · no screen is captured";
		} else {
			self.refresh_requested = true;
			self.status = "Looking for screens and windows…";
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
					self.status = "Looking for screens and windows…";
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
			crate::design::switch(
				ui,
				"screen-body-share-system-audio",
				Some(if cfg!(target_os = "macos") {
					"screen-body-send-what-your-mac-plays-along-with-the-screen-serein"
				} else {
					"screen-body-share-sound-from-other-apps-even-when-sharing-one-window"
				}),
				&mut self.audio,
			);
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
							egui::RichText::new(self.status)
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
			let fill = if selected {
				colors.accent.gamma_multiply(0.18)
			} else if response.hovered() || response.has_focus() {
				colors.hover
			} else {
				colors.base
			};
			ui.painter().rect_filled(rect, 8, fill);
			if selected {
				ui.painter().rect_stroke(
					rect,
					8,
					egui::Stroke::new(1.0, colors.accent),
					egui::StrokeKind::Inside,
				);
			}
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
			let text_left = rect.left() + 46.0;
			let text_width = (rect.right() - 36.0 - text_left).max(40.0);
			let source_name = if source.id == SourceId::SystemPicker {
				crate::i18n::translate("screen-macos-system-picker")
			} else {
				source.name.clone()
			};
			let name = ui.painter().layout(
				source_name.clone(),
				egui::FontId::new(14.0, crate::design::medium_family(ui.ctx())),
				colors.text_strong,
				text_width,
			);
			let kind = ui.painter().layout_no_wrap(
				match source.id {
					SourceId::Display(_) | SourceId::X11Desktop => "Screen".to_owned(),
					SourceId::Window(_) => "Window".to_owned(),
					SourceId::SystemPicker => {
						crate::i18n::translate("screen-macos-system-picker-kind")
					}
					#[allow(unreachable_patterns)] // Portal may be absent outside Linux.
					_ => "System permission dialog".to_owned(),
				},
				egui::FontId::proportional(11.0),
				colors.muted,
			);
			let total = name.size().y + kind.size().y;
			let mut y = rect.center().y - total * 0.5;
			ui.painter()
				.galley(egui::pos2(text_left, y), name, colors.text_strong);
			y += total - kind.size().y;
			ui.painter()
				.galley(egui::pos2(text_left, y), kind, colors.muted);
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
	fn source(id: u64, size: Option<(u32, u32)>) -> Source {
		Source {
			id: SourceId::Display(id),
			name: "Synthetic display".into(),
			dimensions: size,
		}
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
			50_000_000
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
