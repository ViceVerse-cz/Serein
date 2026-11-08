//! Offline native framebuffer capture of the real history browser. No IO or account is used.
use eframe::egui;
use model::{
	Id,
	message_logger::{AttachmentMeta, Change, Entry, GhostMode, Kind},
};
use std::{
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	thread::JoinHandle,
	time::{Duration, Instant},
};
use ui::message_history::{HistoryPage, Preferences};

struct Preview {
	messaging: ui::MessagingUi,
	state: client_core::State,
	output: PathBuf,
	frames: u8,
	requested: bool,
	screenshot: Option<std::sync::mpsc::Receiver<Arc<egui::ColorImage>>>,
	writer: Option<JoinHandle<Result<(), String>>>,
	saved: Arc<AtomicBool>,
	started: Instant,
}

impl eframe::App for Preview {
	fn persist_egui_memory(&self) -> bool {
		false
	}

	fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		ui::design::paint_backdrop(&ctx);
		// This example intentionally has no adapter that could execute host commands.
		let _ = self.messaging.show(ui, &mut self.state);
		if self.requested && self.writer.is_none() {
			if let Some(image) = self.screenshot.as_ref().and_then(|rx| rx.try_recv().ok()) {
				let output = self.output.clone();
				self.writer = Some(std::thread::spawn(move || {
					let pixels: Vec<u8> = image
						.pixels
						.iter()
						.flat_map(|pixel| pixel.to_srgba_unmultiplied())
						.collect();
					let image = image::RgbaImage::from_raw(
						image.size[0] as u32,
						image.size[1] as u32,
						pixels,
					)
					.ok_or("Invalid screenshot pixel count")?;
					image
						.save_with_format(output, image::ImageFormat::Png)
						.map_err(|error| error.to_string())
				}));
			}
		}
		if self.writer.as_ref().is_some_and(JoinHandle::is_finished) {
			match self.writer.take().unwrap().join() {
				Ok(Ok(())) => {
					self.saved.store(true, Ordering::Release);
					println!(
						"Saved offline native framebuffer: {}",
						self.output.display()
					);
				}
				Ok(Err(error)) => eprintln!("Screenshot save failed: {error}"),
				Err(_) => eprintln!("Screenshot worker failed"),
			}
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			return;
		}
		if self.started.elapsed() > Duration::from_secs(20) {
			eprintln!("Native screenshot callback did not complete within 20 seconds");
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
			return;
		}
		self.frames = self.frames.saturating_add(1);
		if self.frames >= 5 && self.started.elapsed() >= Duration::from_secs(1) && !self.requested {
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

fn entry(channel: u64, message: u64, body: &str, deleted: bool) -> Entry {
	let mut events = vec![Change {
		kind: Kind::Edit,
		revision: 6,
		before: Some("Before: synthetic editable body".into()),
		after: Some(body.into()),
	}];
	if deleted {
		events.push(Change {
			kind: Kind::Delete,
			revision: 7,
			before: Some(body.into()),
			after: None,
		});
	}
	Entry {
		channel: Id(channel),
		message: Id(message),
		author: Id(2),
		revision: 7,
		content: body.into(),
		deleted,
		events,
		attachments: vec![AttachmentMeta {
			id: Id(8000 + message),
			filename: "synthetic-preview.png".into(),
			description: Some("Generated metadata only; no URL or bytes".into()),
			content_type: Some("image/png".into()),
			size: 64 * 1024,
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
			deleted,
		}],
		mention_everyone: false,
		mentions_self: true,
		edited_at: Some((1_794_128_400_i128) * 1_000_000_000),
		observed_at_ms: 1_794_128_400_000,
	}
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
	let args: Vec<_> = std::env::args().skip(1).collect();
	if !args.iter().any(|arg| arg == "--demo") {
		return Err("Usage: message_history_preview --demo --output=PATH.png [--light] [--width=1120] [--height=760]".into());
	}
	let value = |prefix: &str| args.iter().find_map(|arg| arg.strip_prefix(prefix));
	let output = PathBuf::from(value("--output=").ok_or("Missing --output=PATH.png")?);
	let width: f32 = value("--width=").unwrap_or("1120").parse()?;
	let height: f32 = value("--height=").unwrap_or("760").parse()?;
	if !(500.0..=1920.0).contains(&width) || !(520.0..=1200.0).contains(&height) {
		return Err("Viewport must be 500-1920 by 520-1200".into());
	}
	let output_directory = output.parent().unwrap_or_else(|| std::path::Path::new("."));
	if output_directory.as_os_str().is_empty() || !output_directory.is_dir() {
		return Err("Choose an existing screenshot directory".into());
	}
	let light = args.iter().any(|arg| arg == "--light");
	let saved = Arc::new(AtomicBool::new(false));
	let completed = saved.clone();
	eframe::run_native(
		"Serein · offline message-history preview",
		eframe::NativeOptions {
			viewport: egui::ViewportBuilder::default()
				.with_inner_size([width, height])
				.with_decorations(false),
			renderer: eframe::Renderer::Wgpu,
			persist_window: false,
			..Default::default()
		},
		Box::new(move |cc| {
			ui::fonts::install(&cc.egui_ctx);
			let _ = ui::emoji::install(&cc.egui_ctx);
			ui::design::apply(&cc.egui_ctx);
			cc.egui_ctx.set_theme(if light {
				egui::ThemePreference::Light
			} else {
				egui::ThemePreference::Dark
			});
			let mut messaging = ui::MessagingUi::default();
			let mut state = test_support::demo_state();
			state.message_logger.set_session_enabled(true);
			let account = state.user.as_ref().map(|user| user.id);
			let generation = state.generation;
			messaging
				.message_history_mut()
				.sync_scope(generation, account);
			messaging.message_history_mut().set_preferences(
				generation,
				account,
				Preferences {
					enabled: true,
					retention_days: 30,
					max_messages: 10_000,
					ghost: GhostMode::Mentions,
					download_images: true,
					download_audio: false,
					download_video: false,
					ignore_users: "10001,10002".into(),
					ignore_channels: "20001".into(),
				},
			);
			messaging
				.message_history_mut()
				.set_capture_controls(generation, account, true);
			messaging
				.message_history_mut()
				.open_all(generation, account);
			messaging.accept_message_history_page(HistoryPage {
				generation,
				account,
				channel: None,
				message: None,
				entries: vec![
					entry(20, 42, "After: synthetic edited body", true),
					entry(20, 41, "Before: synthetic editable body", false),
					entry(30, 8, "Unicode history: Příliš žluťoučký kůň 🦀", false),
				],
				has_older: false,
				has_newer: false,
			});
			Ok(Box::new(Preview {
				messaging,
				state,
				output,
				frames: 0,
				requested: false,
				screenshot: None,
				writer: None,
				saved: completed,
				started: Instant::now(),
			}))
		}),
	)?;
	if !saved.load(Ordering::Acquire) {
		return Err("No screenshot saved".into());
	}
	Ok(())
}
