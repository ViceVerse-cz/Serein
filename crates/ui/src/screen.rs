//! Screen selection is a local intent; the desktop owns capture and stream credentials.
use client_core::{
	State,
	screen::{
		Resolution, Settings, Source, SourceId, offered_frame_rates, offered_resolutions, picture,
	},
	voice::Phase,
};
use model::Id;
use std::borrow::Cow;

pub enum Request {
	Start(Settings),
	Stop,
}

/// The selected source and quality shown while this device's share is live.
pub(crate) struct LiveShare<'a> {
	/// `None` when the platform picker exposes only a placeholder source name.
	pub title: Option<&'a str>,
	pub height: u32,
	pub fps: u32,
}

pub struct ScreenUi {
	pub context: Option<(u64, Id, u64)>,
	pub open: bool,
	pub sources: Vec<Source>,
	pub selected: Option<SourceId>,
	pub refresh_requested: bool,
	pub request: Option<Request>,
	pub busy: bool,
	/// Capture has produced a frame for this device's live share.
	pub sharing: bool,
	/// Owned when it carries a run-time detail such as a Windows error code.
	pub status: Cow<'static, str>,
	pub capture_status: Option<&'static str>,
	pub supported: bool,
	pub preview: Option<egui::TextureHandle>,
	/// The share is running and the local picture is intentionally not updating.
	pub preview_paused: bool,
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
			sharing: false,
			status: Cow::Borrowed(""),
			capture_status: None,
			supported: false,
			preview: None,
			preview_paused: false,
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
			self.status = "Offline preview · no screen is captured".into();
		} else {
			self.refresh_requested = true;
			self.status = "Looking for screens and windows…".into();
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

	/// Whether the selected source is sized only after the system picker.
	fn sized_after_pick(&self) -> bool {
		self.selected_source()
			.is_some_and(|source| source.id.sized_after_pick())
	}

	fn align_quality(&mut self) {
		let (width, height, refresh) = self.geometry();
		let offered: Vec<_> = offered_resolutions(width, height, self.sized_after_pick()).collect();
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
		let (width, height) = picture(
			self.resolution,
			source.width,
			source.height,
			source.id.sized_after_pick(),
		)?;
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

	/// Details for the stream bar once capture is live. Platform pickers report a
	/// placeholder name until the OS chooses a source, so the caller supplies a fallback.
	pub(crate) fn live_share(&self) -> Option<LiveShare<'_>> {
		let source = self.selected_source()?;
		let settings = self.settings()?;
		Some(LiveShare {
			title: (!matches!(source.id, SourceId::SystemPicker | SourceId::Portal))
				.then_some(source.name.as_str()),
			height: settings.height,
			fps: settings.fps,
		})
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
			self.sharing = false;
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
					self.status = "Looking for screens and windows…".into();
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
			for resolution in offered_resolutions(width, height, self.sized_after_pick()) {
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
				} else if cfg!(target_os = "windows") {
					"screen-body-share-the-shared-apps-sound-or-other-apps-for-a-screen"
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
							egui::RichText::new(&*self.status)
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
			.unwrap_or(offered[offered.len() - 1])
	} else {
		// Source when the size is known, otherwise the largest preset (listed last).
		offered[offered.len() - 1]
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
	fn stale_source_cannot_start_capture() {
		let mut picker = ScreenUi {
			selected: Some(SourceId::Window(7)),
			..Default::default()
		};
		assert!(picker.settings().is_none());
		picker.sources.push(Source {
			id: SourceId::Window(7),
			name: "Synthetic window".into(),
			width: 1920,
			height: 1080,
			refresh_hz: 60,
		});
		assert!(picker.settings().is_some());
		picker.sources.clear();
		assert!(picker.settings().is_none());
	}

	#[test]
	fn live_share_reports_the_chosen_quality_and_hides_picker_placeholders() {
		let mut picker = ScreenUi {
			selected: Some(SourceId::Window(7)),
			sources: vec![Source {
				id: SourceId::Window(7),
				name: "Synthetic window".into(),
				width: 2560,
				height: 1440,
				refresh_hz: 60,
			}],
			resolution: Resolution::Preset {
				width: 1280,
				height: 720,
			},
			fps: 60,
			..Default::default()
		};
		let share = picker.live_share().unwrap();
		assert_eq!(share.title, Some("Synthetic window"));
		assert_eq!((share.height, share.fps), (720, 60));

		picker.selected = Some(SourceId::SystemPicker);
		picker.sources = vec![Source {
			id: SourceId::SystemPicker,
			name: "Choose with the macOS system picker".into(),
			width: 2560,
			height: 1440,
			refresh_hz: 60,
		}];
		assert_eq!(picker.live_share().unwrap().title, None);
	}

	#[test]
	fn quality_row_runs_from_480p_to_source_for_any_source_size() {
		let heights = |offered: &[Resolution]| -> Vec<u32> {
			offered
				.iter()
				.map(|resolution| match resolution {
					Resolution::Source => 0,
					Resolution::Preset { height, .. } => *height,
				})
				.collect()
		};
		// Like Discord, every choice is offered even for a smaller source.
		let offered: Vec<_> = offered_resolutions(1280, 720, false).collect();
		assert_eq!(heights(&offered), [480, 720, 1080, 1440, 2160, 0]);
		assert_eq!(
			preferred_resolution(&offered),
			if cfg!(target_os = "linux") {
				Resolution::Preset {
					width: 1280,
					height: 720,
				}
			} else {
				Resolution::Source
			}
		);
		// The Linux portal is sized after the pick, so it offers them all too.
		let portal: Vec<_> = offered_resolutions(0, 0, true).collect();
		assert_eq!(heights(&portal), [480, 720, 1080, 1440, 2160, 0]);
		// Other unknown sizes (macOS picker, whole X11 desktop) keep the 1080p cap.
		let unknown: Vec<_> = offered_resolutions(0, 0, false).collect();
		assert_eq!(heights(&unknown), [480, 720, 1080]);
	}

	#[test]
	fn presets_bound_the_picture_without_upscaling() {
		let preset = |width, height| Resolution::Preset { width, height };
		// A 1080p window shared at 1440p or 2160p keeps its native size.
		assert_eq!(
			picture(preset(2560, 1440), 1920, 1080, false),
			Some((1920, 1080))
		);
		assert_eq!(
			picture(preset(3840, 2160), 1919, 1081, false),
			Some((1918, 1080))
		);
		// A larger source is scaled into the preset, as before.
		assert_eq!(
			picture(preset(1920, 1080), 2560, 1440, false),
			Some((1920, 1080))
		);
		assert_eq!(
			picture(Resolution::Source, 2560, 1440, false),
			Some((2560, 1440))
		);
		// The portal sends the bound; the worker narrows it to the picked display.
		assert_eq!(picture(Resolution::Source, 0, 0, true), Some((3840, 2160)));
		assert_eq!(picture(preset(2560, 1440), 0, 0, true), Some((2560, 1440)));
		assert_eq!(picture(preset(2560, 1440), 0, 0, false), None);
		assert_eq!(picture(Resolution::Source, 0, 0, false), None);
		let bound = Settings {
			source: SourceId::Portal,
			width: 3840,
			height: 2160,
			fps: 30,
			cursor: true,
			audio: false,
		};
		let picked = bound.fitted_to(2560, 1440);
		assert_eq!((picked.width, picked.height), (2560, 1440));
		assert!(picked.bit_rate() < bound.bit_rate());
		// Larger than the bound, or unknown: the bound stays.
		let full_hd = Settings {
			width: 1920,
			height: 1080,
			..bound
		};
		assert_eq!(full_hd.fitted_to(3440, 1440).width, 1920);
		assert_eq!(bound.fitted_to(0, 0).width, 3840);
	}

	#[test]
	fn system_picker_offers_every_choice_and_starts_with_the_bound() {
		let mut picker = ScreenUi {
			selected: Some(SourceId::Portal),
			sources: vec![Source {
				id: SourceId::Portal,
				name: "Choose in the system picker".into(),
				width: 0,
				height: 0,
				refresh_hz: 0,
			}],
			..Default::default()
		};
		picker.align_quality();
		picker.resolution = Resolution::Source;
		let settings = picker.settings().unwrap();
		assert_eq!((settings.width, settings.height), (3840, 2160));
		picker.resolution = Resolution::Preset {
			width: 2560,
			height: 1440,
		};
		let settings = picker.settings().unwrap();
		assert_eq!((settings.width, settings.height), (2560, 1440));
	}
}
