//! One session-only recording/review/upload. Never stores audio or starts it implicitly.
use client_core::{Command, State};
use discord_voice::recording::{Config, Recorder, Recording};
use eframe::egui;
use model::Id;
use std::{
	sync::{Arc, mpsc},
	time::{Duration, Instant},
};
use tokio::sync::watch;
use ui::voice_messages::{Phase, Request};

#[derive(Default)]
pub struct VoiceMessages {
	scope: Option<(u64, Id)>,
	recorder: Option<Recorder>,
	retiring: Option<mpsc::Receiver<()>>,
	recording: Option<Recording>,
	upload: Option<(
		watch::Receiver<discord_api::upload::Status>,
		watch::Sender<bool>,
	)>,
	demo_started: Option<Instant>,
}
impl VoiceMessages {
	pub fn cancel(&mut self) {
		if let Some(recorder) = self.recorder.take() {
			self.retiring = Some(recorder.shutdown());
		}
		if let Some((_, cancel)) = self.upload.take() {
			cancel.send_replace(true);
		}
		self.recording = None;
		self.scope = None;
		self.demo_started = None;
	}
	pub fn capture_busy(&self) -> bool {
		self.recorder.is_some() || self.retiring.is_some()
	}
	pub fn has_unsent(&self) -> bool {
		self.scope.is_some()
	}
	#[allow(clippy::too_many_arguments)]
	pub fn poll(
		&mut self,
		state: &mut State,
		ui: &mut ui::MessagingUi,
		ctx: &egui::Context,
		config: Option<extensions::VoiceMessagesConfig>,
		blocked: bool,
		fixture_only: bool,
	) {
		if self
			.retiring
			.as_ref()
			.is_some_and(|done| !matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)))
		{
			self.retiring = None;
		}
		let view = &mut ui.voice_messages;
		view.config = config;
		view.available = view.config.is_some()
			&& !blocked
			&& state
				.selected
				.is_some_and(|channel| state.demo || state.can_send_voice_message(channel));
		if self.scope.is_some_and(|scope| {
			Some(scope) != view.scope
				|| scope.0 != state.generation
				|| state.selected != Some(scope.1)
				|| !view.available
		}) || view.config.is_none()
			|| (state.user.is_none() && !state.demo)
		{
			self.cancel();
			view.scope = None;
		}
		let muted = ui.voice_muted
			|| ui.voice_deafened
			|| ui.voice_ptm_active
			|| (ui.voice_push_to_talk && !ui.voice_ptt_active);
		view.muted = muted;
		if let Some(recorder) = &self.recorder {
			recorder.set_controls(muted, true);
			let snapshot = recorder.poll();
			view.elapsed_ms = u64::from(snapshot.elapsed_ms);
			view.waveform = snapshot.waveform;
			if snapshot.state == discord_voice::recording::State::Recording
				&& view.phase != Phase::Finalizing
			{
				view.phase = Phase::Recording;
			}
			if let Some(result) = recorder.take_result() {
				view.phase = match result {
					Ok(recording) => {
						for (i, point) in view.waveform.iter_mut().enumerate() {
							*point = recording.waveform[i * recording.waveform.len() / 64];
						}
						self.recording = Some(recording);
						Phase::Review
					}
					Err(error) => Phase::Failed(error),
				};
				self.retiring = self.recorder.take().map(Recorder::shutdown);
			}
		}
		if let Some(started) = self.demo_started {
			let max_ms = view.config.map_or(120_000, |config| {
				u64::from(config.max_duration_seconds) * 1000
			});
			view.elapsed_ms = (started.elapsed().as_millis() as u64).min(max_ms);
			for (i, point) in view.waveform.iter_mut().enumerate() {
				*point = if muted {
					0
				} else {
					((i * 47 + view.elapsed_ms as usize / 80) % 180 + 32) as u8
				};
			}
			if view.elapsed_ms >= max_ms {
				self.demo_started = None;
				view.phase = Phase::Review;
			}
		}
		if let Some((progress, _)) = &self.upload {
			use discord_api::upload::Status;
			let status = progress.borrow().clone();
			match status {
				Status::Preparing | Status::Uploading { .. } | Status::Sending
					if progress.has_changed().is_err() =>
				{
					self.upload = None;
					view.phase = Phase::Failed(
						"Upload interrupted; check the conversation before recording again",
					);
				}
				Status::Uploading { sent, total } => view.progress = Some((sent, total)),
				Status::Finished => {
					self.upload = None;
					self.scope = None;
					view.scope = None;
				}
				Status::Cancelled => {
					self.upload = None;
					self.scope = None;
					view.scope = None;
				}
				Status::Failed(error) => {
					self.upload = None;
					view.phase = Phase::Failed(error);
				}
				_ => {}
			}
		}
		view.capture_busy = self.capture_busy();
		if let Some(request) = view.request.take() {
			match request {
				Request::Cancel => self.cancel(),
				Request::Start
					if view.available && !self.capture_busy() && self.upload.is_none() =>
				{
					let Some(scope) = view.scope else {
						return;
					};
					if scope.0 != state.generation || state.selected != Some(scope.1) {
						self.cancel();
						view.scope = None;
						return;
					}
					self.scope = Some(scope);
					self.recording = None;
					view.elapsed_ms = 0;
					view.waveform = [0; 64];
					view.progress = None;
					if state.demo {
						self.demo_started = Some(Instant::now());
						view.phase = Phase::Recording;
					} else if fixture_only {
						view.phase = Phase::Failed("Microphone is unavailable in this fixture");
					} else {
						let preferences = view.config.expect("enabled recorder");
						let mut processing = ui.voice_processing.effective();
						processing.suppression = if preferences.noise_suppression {
							model::voice_settings::NoiseSuppression::RnNoise
						} else {
							model::voice_settings::NoiseSuppression::Off
						};
						match Recorder::start(Config {
							devices: discord_voice::audio::Devices {
								input: ui.voice_input.clone(),
								output: None,
							},
							processing,
							initial_muted: muted,
							input_gain: ui.voice_gain.input_percent,
							max_duration_seconds: preferences.max_duration_seconds,
						}) {
							Ok(recorder) => {
								recorder.set_controls(muted, true);
								self.recorder = Some(recorder);
								view.phase = Phase::Starting;
							}
							Err(error) => view.phase = Phase::Failed(error),
						}
					}
				}
				Request::Stop => {
					if let Some(recorder) = &self.recorder {
						recorder.stop();
						view.phase = Phase::Finalizing;
					}
					if self.demo_started.take().is_some() {
						view.phase = Phase::Review;
					}
				}
				// Send is consumed by take_send, after native/UI scope admission.
				Request::Send => view.request = Some(Request::Send),
				_ => {}
			}
		}
		view.capture_busy = self.capture_busy();
		if self.capture_busy() || self.upload.is_some() || self.demo_started.is_some() {
			ctx.request_repaint_after(Duration::from_millis(50));
		}
	}
	pub fn take_send(
		&mut self,
		state: &mut State,
		view: &mut ui::voice_messages::VoiceMessages,
	) -> Option<crate::uploads::UploadRequest> {
		if view.request.take() != Some(Request::Send) {
			return None;
		}
		if state.demo
			|| self.scope != view.scope
			|| view.phase != Phase::Review
			|| !view.available
			|| self.upload.is_some()
		{
			return None;
		}
		let recording = self.recording.take()?;
		if recording.bytes.len() as u64 > state.upload_limit() {
			view.phase = Phase::Failed("Voice message exceeds this account's upload limit");
			return None;
		}
		let voice_message = match discord_api::upload::VoiceMessage::new(
			Arc::from(recording.bytes),
			f64::from(recording.duration_secs),
			recording.waveform,
		) {
			Ok(voice) => voice,
			Err(error) => {
				view.phase = Phase::Failed(error);
				return None;
			}
		};
		let Some(command): Option<Command> = state.prepare_voice_message() else {
			view.phase = Phase::Failed("Voice message could not be queued; record again");
			return None;
		};
		let (progress, receive) = watch::channel(discord_api::upload::Status::Preparing);
		let (cancel, _) = watch::channel(false);
		self.upload = Some((receive, cancel.clone()));
		view.phase = Phase::Sending;
		Some(crate::uploads::UploadRequest {
			command,
			source: vec![],
			voice_message: Some(voice_message),
			progress,
			cancel,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn fixture() -> (State, ui::MessagingUi, VoiceMessages, egui::Context) {
		let state = test_support::demo_state();
		(
			state,
			ui::MessagingUi::default(),
			VoiceMessages::default(),
			egui::Context::default(),
		)
	}
	#[test]
	fn synthetic_recording_never_opens_devices_and_navigation_releases_it() {
		let (mut state, mut ui, mut host, ctx) = fixture();
		let config = Some(extensions::VoiceMessagesConfig::default());
		host.poll(&mut state, &mut ui, &ctx, config, false, true);
		let channel = state.selected.unwrap();
		ui.voice_messages.open(&state, channel);
		ui.voice_messages.request = Some(Request::Start);
		host.poll(&mut state, &mut ui, &ctx, config, false, true);
		assert!(host.demo_started.is_some());
		assert!(host.recorder.is_none() && !host.capture_busy());
		ui.voice_messages.request = Some(Request::Stop);
		host.poll(&mut state, &mut ui, &ctx, config, false, true);
		assert_eq!(ui.voice_messages.phase, Phase::Review);
		ui.voice_messages.request = Some(Request::Send);
		assert!(host.take_send(&mut state, &mut ui.voice_messages).is_none());
		assert!(host.upload.is_none());
		state.selected = None;
		host.poll(&mut state, &mut ui, &ctx, config, false, true);
		assert!(!host.has_unsent() && ui.voice_messages.scope.is_none());
	}
	#[test]
	fn disable_logout_and_call_scope_discard_review_and_cancel_upload() {
		for case in 0..3 {
			let (mut state, mut ui, mut host, ctx) = fixture();
			let scope = (state.generation, state.selected.unwrap());
			host.scope = Some(scope);
			ui.voice_messages.scope = Some(scope);
			host.recording = Some(Recording {
				bytes: vec![1, 2, 3],
				duration_secs: 1.0,
				waveform: vec![1],
			});
			let (cancel, mut cancelled) = watch::channel(false);
			let (_, progress) = watch::channel(discord_api::upload::Status::Preparing);
			host.upload = Some((progress, cancel));
			let config = if case == 0 {
				None
			} else {
				Some(extensions::VoiceMessagesConfig::default())
			};
			if case == 1 {
				state.generation += 1;
			}
			host.poll(&mut state, &mut ui, &ctx, config, case == 2, true);
			assert!(*cancelled.borrow_and_update());
			assert!(host.recording.is_none() && host.upload.is_none() && !host.has_unsent());
		}
	}
	#[test]
	fn vanished_upload_worker_exits_each_nonterminal_phase() {
		for status in [
			discord_api::upload::Status::Preparing,
			discord_api::upload::Status::Uploading { sent: 2, total: 4 },
			discord_api::upload::Status::Sending,
		] {
			let (mut state, mut ui, mut host, ctx) = fixture();
			host.scope = Some((state.generation, state.selected.unwrap()));
			ui.voice_messages.scope = host.scope;
			ui.voice_messages.phase = Phase::Sending;
			let (publisher, receive) = watch::channel(status);
			drop(publisher);
			let (cancel, _) = watch::channel(false);
			host.upload = Some((receive, cancel));
			host.poll(
				&mut state,
				&mut ui,
				&ctx,
				Some(extensions::VoiceMessagesConfig::default()),
				false,
				true,
			);
			assert!(host.upload.is_none());
			assert!(matches!(ui.voice_messages.phase, Phase::Failed(_)));
		}
	}

	#[test]
	fn mute_and_push_to_talk_gate_synthetic_waveform_and_failed_upload_cannot_retry_audio() {
		let (mut state, mut ui, mut host, ctx) = fixture();
		let config = Some(extensions::VoiceMessagesConfig::default());
		host.scope = Some((state.generation, state.selected.unwrap()));
		ui.voice_messages.scope = host.scope;
		ui.voice_messages.phase = Phase::Recording;
		host.demo_started = Some(Instant::now());
		ui.voice_push_to_talk = true;
		host.poll(&mut state, &mut ui, &ctx, config, false, true);
		assert!(ui.voice_messages.muted);
		assert!(ui.voice_messages.waveform.iter().all(|point| *point == 0));
		ui.voice_ptt_active = true;
		host.poll(&mut state, &mut ui, &ctx, config, false, true);
		assert!(!ui.voice_messages.muted);
		assert!(ui.voice_messages.waveform.iter().any(|point| *point > 0));
		host.demo_started = None;
		let (_, receive) = watch::channel(discord_api::upload::Status::Failed(
			"Uncertain send; record again",
		));
		let (cancel, _) = watch::channel(false);
		host.upload = Some((receive, cancel));
		host.poll(&mut state, &mut ui, &ctx, config, false, true);
		assert!(matches!(ui.voice_messages.phase, Phase::Failed(_)));
		assert!(host.recording.is_none());
	}
}
