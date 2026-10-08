//! Native recorder presentation only. Audio bytes and devices stay in the desktop host.
use crate::{design, dialog, icons};
use client_core::State;
use egui::{Context, RichText};
use extensions::{VoiceMessagesConfig, WaveformStyle};
use model::Id;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Phase {
	#[default]
	Ready,
	Starting,
	Recording,
	Finalizing,
	Review,
	Sending,
	Failed(&'static str),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
	Start,
	Stop,
	Send,
	Cancel,
}
pub struct VoiceMessages {
	pub config: Option<VoiceMessagesConfig>,
	pub available: bool,
	pub capture_busy: bool,
	pub scope: Option<(u64, Id)>,
	pub phase: Phase,
	pub elapsed_ms: u64,
	pub waveform: [u8; 64],
	pub request: Option<Request>,
	pub muted: bool,
	pub progress: Option<(u64, u64)>,
}
impl Default for VoiceMessages {
	fn default() -> Self {
		Self {
			config: None,
			available: false,
			capture_busy: false,
			scope: None,
			phase: Phase::Ready,
			elapsed_ms: 0,
			waveform: [0; 64],
			request: None,
			muted: false,
			progress: None,
		}
	}
}
impl VoiceMessages {
	pub fn open(&mut self, state: &State, channel: Id) {
		if !self.available || self.config.is_none() || self.scope.is_some() {
			return;
		}
		self.scope = Some((state.generation, channel));
		self.phase = Phase::Ready;
		self.elapsed_ms = 0;
		self.waveform = [0; 64];
		self.progress = None;
	}
	pub fn close(&mut self) {
		self.scope = None;
		self.request = Some(Request::Cancel);
	}
	pub fn show(&mut self, ctx: &Context, state: &State) {
		let Some((generation, channel)) = self.scope else {
			return;
		};
		if generation != state.generation
			|| state.selected != Some(channel)
			|| self.config.is_none()
			|| (!state.demo && !state.can_send_voice_message(channel))
		{
			self.close();
			return;
		}
		let config = self.config.as_ref().expect("validated recorder config");
		let mut request = None;
		let response = dialog::Dialog::new(
			"voice-message-recorder",
			crate::i18n::translate("voice-message-title"),
		)
		.icon(icons::Icon::Microphone)
		.width(440.0)
		.show(ctx, |body| {
			body.content(|ui| {
				let colors = design::palette(ui);
				if state.demo {
					design::notice(ui, design::Level::Info, "voice-message-offline");
				}
				ui.horizontal(|ui| {
					ui.label(
						RichText::new(format!(
							"{}:{:02}",
							self.elapsed_ms / 60_000,
							(self.elapsed_ms / 1000) % 60
						))
						.size(28.0)
						.color(colors.text_strong),
					);
					ui.label(
						RichText::new(format!(
							"/ {}:{:02}",
							config.max_duration_seconds / 60,
							config.max_duration_seconds % 60
						))
						.color(colors.muted),
					);
				});
				let (rect, _) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 72.0),
					egui::Sense::hover(),
				);
				ui.painter().rect_filled(rect, 8, colors.raised);
				let inner = rect.shrink(12.0);
				if config.waveform_style == WaveformStyle::Line {
					let points: Vec<_> = self
						.waveform
						.iter()
						.enumerate()
						.map(|(i, value)| {
							egui::pos2(
								inner.left() + i as f32 * inner.width() / 63.0,
								inner.center().y
									- f32::from(*value) / 255.0 * inner.height() * 0.45,
							)
						})
						.collect();
					ui.painter().add(egui::Shape::line(
						points,
						egui::Stroke::new(2.0, colors.accent),
					));
				} else {
					for (i, value) in self.waveform.iter().enumerate() {
						let x = inner.left() + (i as f32 + 0.5) * inner.width() / 64.0;
						let height = (f32::from(*value) / 255.0 * inner.height()).max(3.0);
						ui.painter().line_segment(
							[
								egui::pos2(x, inner.center().y - height / 2.0),
								egui::pos2(x, inner.center().y + height / 2.0),
							],
							egui::Stroke::new(2.5, colors.accent),
						);
					}
				}
				let status = match self.phase {
					Phase::Ready => "voice-message-ready",
					Phase::Starting => "voice-message-starting",
					Phase::Recording if self.muted => "voice-message-muted",
					Phase::Recording => "voice-message-recording",
					Phase::Finalizing => "voice-message-finalizing",
					Phase::Review => "voice-message-review",
					Phase::Sending => "voice-message-sending",
					Phase::Failed(error) => {
						design::notice(ui, design::Level::Error, error);
						"voice-message-retry"
					}
				};
				ui.label(RichText::new(crate::i18n::translate(status)).color(colors.muted));
				if let Some((sent, total)) = self.progress {
					ui.add(egui::ProgressBar::new(sent as f32 / total.max(1) as f32));
				}
				if self.phase == Phase::Sending {
					design::hint(ui, "voice-message-cancel-send");
				}
				design::hint(ui, "voice-message-private");
			});
			body.footer(|ui| {
				match self.phase {
					Phase::Ready | Phase::Failed(_) => {
						if ui
							.add_enabled_ui(self.available && !self.capture_busy, |ui| {
								dialog::action(ui, "voice-message-record", dialog::Action::Primary)
							})
							.inner
							.clicked()
						{
							request = Some(Request::Start);
						}
					}
					Phase::Starting | Phase::Recording => {
						if dialog::action(ui, "voice-message-stop", dialog::Action::Primary)
							.clicked()
						{
							request = Some(Request::Stop);
						}
					}
					Phase::Review => {
						if ui
							.add_enabled_ui(!state.demo && self.available, |ui| {
								dialog::action(ui, "voice-message-send", dialog::Action::Primary)
							})
							.inner
							.clicked()
						{
							request = Some(Request::Send);
						}
					}
					Phase::Finalizing | Phase::Sending => {
						ui.spinner();
					}
				}
				if dialog::action(ui, "voice-message-discard", dialog::Action::Neutral).clicked() {
					request = Some(Request::Cancel);
				}
			});
		});
		if response.close || request == Some(Request::Cancel) {
			self.close();
		} else if let Some(request) = request {
			self.request = Some(request);
		}
		if matches!(
			self.phase,
			Phase::Starting | Phase::Recording | Phase::Finalizing | Phase::Sending
		) {
			ctx.request_repaint_after(std::time::Duration::from_millis(50));
		}
	}
}
