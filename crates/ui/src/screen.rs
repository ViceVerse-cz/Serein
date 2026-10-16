//! Screen selection is a local intent; the desktop owns capture and stream credentials.
use client_core::{
	State,
	screen::{
		Resolution, Settings, Source, SourceId, offered_frame_rates, offered_resolutions, picture,
	},
	voice::Phase,
};
use model::Id;

pub enum Request {
	Start(Settings),
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
	/// Selection the current resolution was chosen for. `None` until sources arrive.
	quality_for: Option<SourceId>,
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
			resolution: Resolution::Source,
			quality_for: None,
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
		self.quality_for = None;
		self.sources.clear();
		if state.demo {
			self.sources = if cfg!(target_os = "macos") {
				vec![Source {
					id: SourceId::SystemPicker,
					name: "Choose with the macOS system picker".into(),
					width: 0,
					height: 0,
					refresh_hz: 0,
				}]
			} else {
				vec![
					Source {
						id: SourceId::Display(1),
						name: "Display 1 · Synthetic preview".into(),
						width: 2560,
						height: 1440,
						refresh_hz: 165,
					},
					Source {
						id: SourceId::Window(2),
						name: "Project notes · Synthetic window".into(),
						width: 1280,
						height: 720,
						refresh_hz: 60,
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
	fn selected_source(&self) -> Option<&Source> {
		self.sources
			.iter()
			.find(|source| Some(source.id) == self.selected)
	}

	fn geometry(&self) -> (u32, u32, u32) {
		self.selected_source().map_or((0, 0, 0), |source| {
			(source.width, source.height, source.refresh_hz)
		})
	}

	fn align_quality(&mut self) {
		let (width, height, refresh) = self.geometry();
		let offered: Vec<_> = offered_resolutions(width, height).collect();
		if offered.is_empty() {
			return;
		}
		if self.quality_for.is_none() && self.quality_for != self.selected {
			self.resolution = preferred_resolution(&offered);
		}
		self.quality_for = self.selected;
		if !offered.contains(&self.resolution) {
			self.resolution = preferred_resolution(&offered);
		}
		let rates: Vec<_> = offered_frame_rates(refresh).collect();
		if !rates.contains(&self.fps) {
			self.fps = rates
				.into_iter()
				.rev()
				.find(|fps| *fps <= self.fps)
				.unwrap_or(30);
		}
	}

	fn settings(&self) -> Option<Settings> {
		let source = self.selected_source()?;
		let (width, height) = picture(self.resolution, source.width, source.height)?;
		let settings = Settings {
			source: source.id,
			width,
			height,
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
			self.quality_for = None;
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
		.width(520.0)
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

	/// Source list, then the capture options Discord exposes without an entitlement.
	fn body(&mut self, ui: &mut egui::Ui, state: &State) {
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
					self.selected = None;
					self.quality_for = None;
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
		self.align_quality();
		let (width, height, refresh) = self.geometry();
		ui.add_space(14.0);
		ui.label(crate::design::eyebrow(
			ui,
			crate::i18n::translate("screen-body-quality"),
			colors.muted,
		));
		ui.add_space(6.0);
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
			for resolution in offered_resolutions(width, height) {
				let label = match resolution {
					Resolution::Source => "screen-body-source".to_owned(),
					Resolution::Preset { height, .. } => format!("{height}p"),
				};
				if segment(ui, &label, self.resolution == resolution).clicked() {
					self.resolution = resolution;
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
			for fps in offered_frame_rates(refresh) {
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

fn preferred_resolution(offered: &[Resolution]) -> Resolution {
	if cfg!(target_os = "linux") {
		offered
			.iter()
			.copied()
			.find(|item| matches!(item, Resolution::Preset { height: 720, .. }))
			.unwrap_or(offered[0])
	} else {
		offered[0]
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
	let color = if selected {
		colors.accent_text
	} else {
		colors.text
	};
	ui.painter().galley(
		egui::pos2(
			rect.center().x - galley.size().x * 0.5,
			rect.center().y - galley.size().y * 0.5,
		),
		galley,
		color,
	);
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, &label)
	});
	response
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn settings_require_a_current_source_and_offer_high_quality_without_entitlements() {
		let mut picker = ScreenUi {
			selected: Some(SourceId::Window(7)),
			..Default::default()
		};
		assert!(picker.settings().is_none());
		picker.sources.push(Source {
			id: SourceId::Window(7),
			name: "Notes".into(),
			width: 0,
			height: 0,
			refresh_hz: 0,
		});
		picker.resolution = Resolution::Preset {
			width: 1920,
			height: 1080,
		};
		picker.fps = 60;
		let settings = picker.settings().unwrap();
		assert_eq!(
			(settings.width, settings.height, settings.fps),
			(1920, 1080, 60)
		);
		assert_eq!(settings.bit_rate(), 16_000_000);
		let resolutions: Vec<_> = offered_resolutions(2560, 1440).collect();
		assert!(resolutions.contains(&Resolution::Source));
		assert!(resolutions.contains(&Resolution::Preset {
			width: 2560,
			height: 1440,
		}));
		assert!(!offered_resolutions(1920, 1080).any(|item| item
			== Resolution::Preset {
				width: 2560,
				height: 1440,
			}));
		assert_eq!(picture(Resolution::Source, 2560, 1440), Some((2560, 1440)));
		assert_eq!(picture(Resolution::Source, 5120, 1440), Some((3840, 1080)));
		assert_eq!(
			offered_frame_rates(165).collect::<Vec<_>>(),
			vec![15, 30, 60, 90, 120]
		);
		assert_eq!(
			offered_frame_rates(60).collect::<Vec<_>>(),
			vec![15, 30, 60]
		);
		picker.audio = true;
		assert!(picker.settings().unwrap().audio);
		picker.audio = false;
		assert!(!picker.settings().unwrap().audio);
		picker.sources.clear();
		assert!(picker.settings().is_none());
	}
}
