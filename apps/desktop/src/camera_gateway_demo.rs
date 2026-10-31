//! Offline replay and native framebuffer evidence for camera behavior after Gateway loss.
use client_core::{Envelope, Event, State, voice};
use eframe::egui;
use std::{
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	thread::JoinHandle,
	time::{Duration, Instant},
};

struct Replay {
	state: State,
	channel: model::Id,
	request: u64,
	initial_preview_eligible: bool,
	retained_preview_eligible: bool,
	new_activation_eligible: bool,
	camera_off_emitted: bool,
	final_phase: Option<voice::Phase>,
	final_camera: bool,
}

fn replay() -> Replay {
	let mut state = test_support::call_demo_state();
	let call = state.voice.active.as_mut().expect("synthetic active call");
	call.camera = true;
	call.request = 1;
	let channel = call.channel;
	let request = call.request;
	state.gateway_connected = true;
	// Demo fixtures bypass permission checks; exercise the same predicates as a live state.
	state.demo = false;
	let initial_preview_eligible = crate::voice::camera_preview_allowed(&state);
	assert!(
		initial_preview_eligible,
		"fixture must begin camera eligible"
	);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Disconnected,
	});
	let retained_preview_eligible = crate::voice::camera_preview_allowed(&state);
	let mut camera_off_emitted = false;
	if !retained_preview_eligible {
		let command = state
			.set_call_camera(false)
			.expect("ineligible active camera should yield camera-off command");
		let client_core::Command::Voice(voice::Command::SetCamera {
			channel: command_channel,
			request: command_request,
			enabled: false,
		}) = command
		else {
			panic!("camera-off reducer command expected");
		};
		assert_eq!((command_channel, command_request), (channel, request));
		camera_off_emitted = true;
		let failure =
			crate::connection::voice_offline_failure(state.gateway_connected, channel, request)
				.expect("offline call admission must fail");
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Voice(failure),
		});
	}
	let new_activation_eligible = state.can_camera(channel);
	let (final_phase, final_camera) = state
		.voice
		.active
		.as_ref()
		.map_or((None, false), |call| (Some(call.phase), call.camera));
	state.demo = true;
	Replay {
		state,
		channel,
		request,
		initial_preview_eligible,
		retained_preview_eligible,
		new_activation_eligible,
		camera_off_emitted,
		final_phase,
		final_camera,
	}
}

pub(crate) fn check() {
	let replay = replay();
	assert!(replay.initial_preview_eligible);
	assert_eq!(replay.final_phase, Some(voice::Phase::Connected));
	assert!(
		replay.final_camera,
		"ongoing camera preview should be retained"
	);
	assert!(
		!replay.camera_off_emitted,
		"no camera-off command should be emitted"
	);
	assert!(
		!replay.new_activation_eligible,
		"new camera activation requires Gateway"
	);
	println!(
		"Offline camera Gateway replay passed: active preview retained; new activation remains unavailable."
	);
}

pub(crate) fn capture(output: PathBuf) -> eframe::Result {
	let replay = replay();
	let saved = Arc::new(AtomicBool::new(false));
	let saved_on_exit = saved.clone();
	let options = eframe::NativeOptions {
		viewport: egui::ViewportBuilder::default()
			.with_inner_size([1040.0, 720.0])
			.with_resizable(false),
		persist_window: false,
		persistence_path: None,
		..Default::default()
	};
	eframe::run_native(
		"Serein synthetic camera Gateway replay",
		options,
		Box::new(move |cc| {
			ui::fonts::install(&cc.egui_ctx);
			let _ = ui::emoji::install(&cc.egui_ctx);
			ui::design::apply(&cc.egui_ctx);
			cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
			cc.egui_ctx.set_pixels_per_point(1.0);
			Ok(Box::new(EvidenceApp::new(replay, output, saved.clone())))
		}),
	)?;
	if !saved_on_exit.load(Ordering::Acquire) {
		return Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(
			"offline screenshot was not saved",
		))));
	}
	Ok(())
}

struct EvidenceApp {
	replay: Replay,
	output: PathBuf,
	saved: Arc<AtomicBool>,
	requested: bool,
	screenshot: Option<std::sync::mpsc::Receiver<Arc<egui::ColorImage>>>,
	writer: Option<JoinHandle<Result<(), String>>>,
	started: Instant,
	messaging: ui::MessagingUi,
}

impl EvidenceApp {
	fn new(replay: Replay, output: PathBuf, saved: Arc<AtomicBool>) -> Self {
		Self {
			replay,
			output,
			saved,
			requested: false,
			screenshot: None,
			writer: None,
			started: Instant::now(),
			messaging: ui::MessagingUi::default(),
		}
	}
}

impl eframe::App for EvidenceApp {
	fn persist_egui_memory(&self) -> bool {
		false
	}

	fn raw_input_hook(&mut self, _: &egui::Context, raw_input: &mut egui::RawInput) {
		raw_input.events.clear();
	}

	fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		ui::design::paint_backdrop(&ctx);
		egui::Panel::top("camera_gateway_evidence_header").show(ui, |ui| {
			ui.heading("Synthetic offline replay");
			ui.label(format!(
				"{} · Synthetic state/guard replay; no camera/audio/network adapters",
				if self.replay.final_camera && self.replay.retained_preview_eligible {
					"Gateway disconnected · camera request retained"
				} else {
					"Gateway disconnected · camera-off command failed the call"
				}
			));
			ui.label(format!(
				"Gateway: {}  |  Preview eligible: {}  |  New activation eligible: {}  |  Camera-off command: {}",
				self.replay.state.gateway_connected,
				self.replay.retained_preview_eligible,
				self.replay.new_activation_eligible,
				self.replay.camera_off_emitted
			));
			ui.label(format!(
				"Call {} · request {} · final phase: {} · camera: {}",
				self.replay.channel.0,
				self.replay.request,
				self.replay.final_phase.map_or("Ended", voice::Phase::label),
				self.replay.final_camera
			));
		});
		egui::CentralPanel::default().show(ui, |ui| {
			let _ignored_synthetic_commands = self.messaging.show(ui, &mut self.replay.state);
		});
		if self.writer.is_none()
			&& self.requested
			&& let Some(image) = self.screenshot.as_ref().and_then(|rx| rx.try_recv().ok())
		{
			let output = self.output.clone();
			self.writer = Some(std::thread::spawn(move || {
				if image.size[0] > 4096 || image.size[1] > 4096 {
					return Err("Screenshot exceeds the 4096-pixel dimension limit".into());
				}
				let bytes: Vec<u8> = image
					.pixels
					.iter()
					.flat_map(|pixel| pixel.to_srgba_unmultiplied())
					.collect();
				let image =
					image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, bytes)
						.ok_or("Invalid screenshot pixel count")?;
				image
					.save_with_format(&output, image::ImageFormat::Png)
					.map_err(|error| error.to_string())?;
				if std::fs::metadata(&output)
					.map_err(|error| error.to_string())?
					.len() > 2 * 1024 * 1024
				{
					return Err("Screenshot exceeds the 2 MiB evidence limit".into());
				}
				Ok(())
			}));
		}
		if self.writer.as_ref().is_some_and(JoinHandle::is_finished) {
			match self.writer.take().unwrap().join() {
				Ok(Ok(())) => {
					self.saved.store(true, Ordering::Release);
					println!(
						"Saved offline camera Gateway evidence: {}",
						self.output.display()
					);
				}
				Ok(Err(error)) => eprintln!("Offline screenshot save failed: {error}"),
				Err(_) => eprintln!("Offline screenshot writer failed"),
			}
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			return;
		}
		if self.started.elapsed() > Duration::from_secs(20) {
			eprintln!("Native screenshot callback did not complete within 20 seconds");
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			return;
		}
		if !self.requested && self.started.elapsed() >= Duration::from_secs(1) {
			self.requested = true;
			let (send, receive) = std::sync::mpsc::sync_channel(1);
			self.screenshot = Some(receive);
			let wake = ctx.clone();
			ctx.request_screenshot(move |image| {
				let _ = send.try_send(image);
				wake.request_repaint();
			});
		}
		ctx.request_repaint_after(Duration::from_millis(100));
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn disconnected_gateway_retains_active_camera_but_blocks_new_activation() {
		super::check();
	}
}
