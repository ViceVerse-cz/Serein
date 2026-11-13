//! Offline video profiling; no network or audio devices.
//! cargo run --release --locked -p serein --features demo --example video_handoff -- --demo
//! Default: synthetic 1080p frame uploads without a decoder. Add --headless to omit the GPU.
//! --decode=/path/to/synthetic.mp4 instead measures only the native decoder, without a window.
use eframe::egui;
use std::time::{Duration, Instant};

const WIDTH: usize = 1920;
const HEIGHT: usize = 1080;

fn pixels() -> Vec<u8> {
	(0..WIDTH * HEIGHT)
		.flat_map(|i| [(i % WIDTH / 8) as u8, (i / WIDTH / 4) as u8, 128, 255])
		.collect()
}

fn player() -> (ui::VideoUi, model::Message) {
	let state = test_support::video_demo_state();
	let message = state.timeline.get(model::Id(601)).unwrap().clone();
	let mut player = ui::VideoUi::default();
	player.active = Some((message.channel, message.id, message.attachments[0].clone()));
	player.state = ui::VideoState::Playing;
	player.duration = 30.;
	(player, message)
}

fn deliver(player: &mut ui::VideoUi, ctx: &egui::Context, rgba: Vec<u8>) {
	assert!(player.accept_frame(ctx, WIDTH, HEIGHT, &rgba));
}

fn decode(path: &str) {
	use platform::video::{Decoder, Sample};
	use std::{io::Read, task::Poll};
	let mut bytes = Vec::new();
	std::fs::File::open(path)
		.unwrap()
		.take(100 * 1024 * 1024 + 1)
		.read_to_end(&mut bytes)
		.unwrap();
	assert!(bytes.len() <= 100 * 1024 * 1024);
	let started = Instant::now();
	let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(bytes))).unwrap();
	println!("info={:?}", decoder.info());
	let (mut video_done, mut audio_done, mut frames) = (false, false, 0);
	while !video_done || !audio_done {
		// Cooperative deadline between native calls; cannot interrupt a blocked OS decoder.
		assert!(
			started.elapsed() < Duration::from_secs(120),
			"Decode deadline exceeded"
		);
		let mut progressed = false;
		if !video_done {
			match decoder.poll_video().unwrap() {
				Poll::Ready(Some(Sample::Video { rgba, .. })) => {
					std::hint::black_box(rgba);
					frames += 1;
					progressed = true;
				}
				Poll::Ready(None) => video_done = true,
				Poll::Pending => {}
				_ => panic!("Wrong video track"),
			}
		}
		if !audio_done {
			match decoder.poll_audio().unwrap() {
				Poll::Ready(Some(Sample::Audio { frames, .. })) => {
					std::hint::black_box(frames);
					progressed = true;
				}
				Poll::Ready(None) => audio_done = true,
				Poll::Pending => {}
				_ => panic!("Wrong audio track"),
			}
		}
		if !progressed {
			std::thread::sleep(Duration::from_millis(1));
		}
	}
	println!(
		"decoded_frames={frames} elapsed_ms={:.3}",
		started.elapsed().as_secs_f64() * 1000.
	);
}

struct Preview {
	player: ui::VideoUi,
	message: model::Message,
	pixels: Vec<u8>,
	started: Instant,
	next: Instant,
	frames: usize,
	handoff: Duration,
}

impl eframe::App for Preview {
	fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
		let now = Instant::now();
		if now >= self.next {
			let started = Instant::now();
			deliver(&mut self.player, ui.ctx(), self.pixels.clone());
			self.handoff += started.elapsed();
			self.frames += 1;
			self.next = now + Duration::from_secs_f64(1. / 60.);
		}
		self.player.position = self.started.elapsed().as_secs_f64();
		self.player.show(
			ui,
			&self.message,
			&self.message.attachments[0],
			&mut ui::DownloadUi::default(),
			&mut None,
			true,
		);
		self.player.command = None;
		if self.started.elapsed() >= Duration::from_secs(30) {
			println!(
				"frames={} handoff_ms={:.3}",
				self.frames,
				self.handoff.as_secs_f64() * 1000.
			);
			ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
		} else {
			ui.ctx()
				.request_repaint_after(self.next.saturating_duration_since(Instant::now()));
		}
	}
}

fn main() -> eframe::Result {
	let args: Vec<_> = std::env::args().collect();
	assert!(args.iter().any(|arg| arg == "--demo"), "Requires --demo");
	if let Some(path) = args.iter().find_map(|arg| arg.strip_prefix("--decode=")) {
		decode(path);
		return Ok(());
	}
	let (mut player, message) = player();
	let pixels = pixels();
	if args.iter().any(|arg| arg == "--headless") {
		let ctx = egui::Context::default();
		let started = Instant::now();
		for _ in 0..600 {
			deliver(&mut player, &ctx, std::hint::black_box(pixels.clone()));
			// A real renderer releases each texture delta after uploading it.
			drop(ctx.tex_manager().write().take_delta());
		}
		println!(
			"frames=600 handoff_ms={:.3}",
			started.elapsed().as_secs_f64() * 1000.
		);
		return Ok(());
	}
	eframe::run_native(
		"Synthetic video handoff — no decoder",
		eframe::NativeOptions {
			viewport: egui::ViewportBuilder::default().with_inner_size([800., 600.]),
			..Default::default()
		},
		Box::new(move |cc| {
			ui::design::apply(&cc.egui_ctx);
			Ok(Box::new(Preview {
				player,
				message,
				pixels,
				started: Instant::now(),
				next: Instant::now(),
				frames: 0,
				handoff: Duration::ZERO,
			}))
		}),
	)
}
