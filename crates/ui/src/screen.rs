//! Screen selection is a local intent; the desktop owns capture and stream credentials.
use client_core::{
	State,
	screen::{Settings, Source, SourceId},
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
	height: u32,
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
			height: if cfg!(target_os = "linux") { 720 } else { 1080 },
			fps: 30,
			cursor: true,
			audio: cfg!(target_os = "macos"),
		}
	}
}
impl ScreenUi {
	pub(super) fn launch(&mut self, state: &State) {
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
			self.sources = vec![
				Source {
					id: SourceId::Display(1),
					name: "Display 1 · Synthetic preview".into(),
				},
				Source {
					id: SourceId::Window(2),
					name: "Project notes · Synthetic window".into(),
				},
			];
			self.selected = Some(SourceId::Display(1));
			self.status = "Offline preview · no screen is captured";
		} else {
			self.refresh_requested = true;
			self.status = "Looking for screens and windows…";
		}
	}
	fn settings(&self) -> Option<Settings> {
		let source = self
			.selected
			.filter(|id| self.sources.iter().any(|s| s.id == *id))?;
		let settings = Settings {
			source,
			width: match self.height {
				480 => 854,
				1080 => 1920,
				_ => 1280,
			},
			height: self.height,
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
		let response = crate::dialog::Dialog::new("screen-share-settings", "Share your screen")
			.subtitle("Choose what people in this call can see.")
			.width(460.0)
			.show(ctx, |d| {
				d.scroll(260.0, |ui| self.body(ui, state));
				d.footer(|ui| {
					let allowed = !state.demo
						&& self.supported && !self.busy
						&& self.settings().is_some()
						&& state.voice.active.as_ref().is_some_and(|call| {
							matches!(call.phase, Phase::Connected | Phase::Waiting)
								&& state.can_stream(call.channel)
						});
					ui.add_enabled_ui(allowed, |ui| {
						share = crate::dialog::action(
							ui,
							"Share Screen",
							crate::dialog::Action::Primary,
						)
						.clicked();
					});
					cancel |= crate::dialog::action(ui, "Cancel", crate::dialog::Action::Neutral)
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
			ui.label(crate::design::eyebrow(ui, "Screen or window", colors.muted));
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				if ui
					.add_enabled(
						!state.demo && !cfg!(target_os = "linux"),
						egui::Button::new(
							egui::RichText::new("Refresh").size(12.0).color(colors.link),
						)
						.frame(false),
					)
					.clicked()
				{
					self.selected = None;
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
		ui.add_space(14.0);
		ui.label(crate::design::eyebrow(ui, "Quality", colors.muted));
		ui.add_space(6.0);
		ui.horizontal_wrapped(|ui| {
			ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
			for height in [480, 720, 1080] {
				if segment(ui, &format!("{height}p"), self.height == height).clicked() {
					self.height = height;
				}
			}
		});
		ui.add_space(8.0);
		ui.label(crate::design::eyebrow(ui, "Frame rate", colors.muted));
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
				egui::RichText::new("Quality selection does not require Nitro.")
					.size(12.0)
					.color(colors.muted),
			)
			.wrap(),
		);
		ui.add_space(10.0);
		crate::design::switch(
			ui,
			"Show cursor",
			Some("Include the pointer in the shared video."),
			&mut self.cursor,
		);
		if self.supported {
			ui.add_space(6.0);
			crate::design::switch(
				ui,
				"Share system audio",
				Some(if cfg!(target_os = "macos") {
					"Send what your Mac plays along with the screen. Serein's own call audio is left out."
				} else {
					"Share sound from other apps, even when sharing one window. Serein's own audio is left out."
				}),
				&mut self.audio,
			);
		}
		ui.add_space(4.0);
		ui.add(
			egui::Label::new(
				egui::RichText::new("Your call microphone keeps its current settings.")
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
							egui::RichText::new("No screens or windows are available yet.")
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
			let name = ui.painter().layout(
				source.name.clone(),
				egui::FontId::new(14.0, crate::design::medium_family(ui.ctx())),
				colors.text_strong,
				text_width,
			);
			let kind = ui.painter().layout_no_wrap(
				match source.id {
					SourceId::Display(_) | SourceId::X11Desktop => "Screen",
					SourceId::Window(_) => "Window",
					#[allow(unreachable_patterns)] // Portal may be absent outside Linux.
					_ => "System permission dialog",
				}
				.to_owned(),
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
				egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, &source.name)
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
	response
		.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, label));
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
		});
		picker.height = 1080;
		picker.fps = 60;
		let settings = picker.settings().unwrap();
		assert_eq!(
			(settings.width, settings.height, settings.fps),
			(1920, 1080, 60)
		);
		assert_eq!(settings.bit_rate(), 16_000_000);
		picker.audio = true;
		assert!(picker.settings().unwrap().audio);
		picker.audio = false;
		assert!(!picker.settings().unwrap().audio);
		picker.sources.clear();
		assert!(picker.settings().is_none());
	}
}
