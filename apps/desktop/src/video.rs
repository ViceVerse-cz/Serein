//! One explicit inline player; native decoding and network reads stay on one lazy worker.
#[cfg(target_os = "macos")]
mod fallback;
mod output;
mod source;
use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use ui::{VideoCommand, VideoState, VideoUi};

#[derive(Default)]
struct Update {
	state: VideoState,
	position: f64,
	duration: f64,
	frame: Option<(u32, u32, Vec<u8>)>,
}
struct Session {
	cancelled: Arc<AtomicBool>,
	paused: Arc<AtomicBool>,
	volume: Arc<AtomicU32>,
	seek: Arc<AtomicU64>,
	update: Mutex<Update>,
}
impl Session {
	fn new(volume: f32) -> Self {
		Self {
			cancelled: Arc::new(AtomicBool::new(false)),
			paused: Arc::new(AtomicBool::new(false)),
			volume: Arc::new(AtomicU32::new(volume.to_bits())),
			seek: Arc::new(AtomicU64::new(u64::MAX)),
			update: Mutex::new(Update {
				state: VideoState::Loading,
				..Default::default()
			}),
		}
	}
}
#[derive(Clone)]
struct Request {
	session: Arc<Session>,
	url: Option<url::Url>,
	size: usize,
}
#[derive(Default)]
pub struct Video {
	session: Option<Arc<Session>>,
	requests: Option<tokio::sync::watch::Sender<Option<Request>>>,
}
impl Video {
	pub fn stop(&mut self) {
		if let Some(session) = self.session.take() {
			session.cancelled.store(true, Ordering::Release);
		}
		if let Some(requests) = &self.requests {
			requests.send_replace(None);
		}
	}
	pub fn poll(&self, player: &mut VideoUi, ctx: &eframe::egui::Context) {
		let Some(session) = &self.session else {
			return;
		};
		if let Ok(mut update) = session.update.try_lock() {
			player.state = update.state;
			player.position = update.position;
			player.duration = update.duration;
			let frame = update.frame.take();
			drop(update);
			if let Some((width, height, rgba)) = frame {
				player.accept_frame(ctx, width as usize, height as usize, &rgba);
			}
		}
	}
	pub fn command(
		&mut self,
		command: VideoCommand,
		player: &mut VideoUi,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) {
		match command {
			VideoCommand::Stop => self.stop(),
			VideoCommand::Play(attachment) => {
				self.stop();
				if let Err(error) = self.start(attachment, player.volume, runtime, ctx, demo) {
					player.state = VideoState::Failed(error);
				}
			}
			VideoCommand::Pause(paused) => {
				if let Some(s) = &self.session {
					s.paused.store(paused, Ordering::Release);
				}
			}
			VideoCommand::Volume(volume) => {
				if volume.is_finite()
					&& let Some(s) = &self.session
				{
					s.volume
						.store(volume.clamp(0., 1.).to_bits(), Ordering::Release);
				}
			}
			VideoCommand::Seek(seconds) => {
				if seconds.is_finite()
					&& let Some(s) = &self.session
				{
					s.seek
						.store((seconds.clamp(0., 7200.) * 1000.) as u64, Ordering::Release);
				}
			}
		}
	}
	fn start(
		&mut self,
		attachment: model::Attachment,
		volume: f32,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) -> Result<(), &'static str> {
		if !attachment.is_video() || attachment.size > 100 * 1024 * 1024 {
			return Err("Video preview limit: 100 MiB");
		}
		let url = if demo {
			None
		} else if attachment.size == 0 {
			// Embed videos have no declared size; the worker probes it before decoding.
			Some(embed_url(&attachment.media).ok_or("Video embed unavailable")?)
		} else {
			Some(
				crate::downloads::original_url(&attachment)
					.ok_or("Video attachment unavailable")?,
			)
		};
		if self.requests.is_none() {
			let (sender, mut receiver) = tokio::sync::watch::channel::<Option<Request>>(None);
			let runtime = runtime.clone();
			let ctx = ctx.clone();
			std::thread::Builder::new()
				.name("serein-attachment-video".into())
				.spawn(move || {
					while runtime.block_on(receiver.changed()).is_ok() {
						let Some(request) = receiver.borrow_and_update().clone() else {
							continue;
						};
						if let Err(error) = play(&request, &runtime, &ctx)
							&& !request.session.cancelled.load(Ordering::Acquire)
						{
							if let Ok(mut update) = request.session.update.lock() {
								update.state = VideoState::Failed(error);
							}
							ctx.request_repaint();
						}
					}
				})
				.map_err(|_| "Could not start video worker")?;
			self.requests = Some(sender);
		}
		let requests = self.requests.as_ref().expect("worker created");
		if requests.is_closed() {
			return Err("Video worker stopped; restart Serein");
		}
		let session = Arc::new(Session::new(volume));
		requests.send_replace(Some(Request {
			session: session.clone(),
			url,
			size: attachment.size as usize,
		}));
		self.session = Some(session);
		Ok(())
	}
}
/// Only Discord's media proxy copy of an external MP4/MOV; never the third-party origin.
fn embed_url(media: &model::EmbedMedia) -> Option<url::Url> {
	let url = url::Url::parse(media.url.as_deref()?).ok()?;
	(url.scheme() == "https"
		&& url.host_str() == Some("media.discordapp.net")
		&& url.port_or_known_default() == Some(443)
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.fragment().is_none()
		&& url.path().starts_with("/external/")
		&& !url.path().contains('\\')
		&& ui::is_motion_video(url.path()))
	.then_some(url)
}
impl Drop for Video {
	fn drop(&mut self) {
		self.stop();
	}
}

fn play(
	request: &Request,
	runtime: &tokio::runtime::Handle,
	ctx: &eframe::egui::Context,
) -> Result<(), &'static str> {
	use platform::video::Decoder;
	let session = &request.session;
	let source = source::source(
		request.url.clone(),
		request.size,
		session.cancelled.clone(),
		runtime.clone(),
	)?;
	let decoder = Decoder::open(source);
	#[cfg(target_os = "macos")]
	let decoder = match decoder {
		Err(platform::video::UNSUPPORTED | platform::video::INVALID) => {
			let source = source::source(
				request.url.clone(),
				request.size,
				session.cancelled.clone(),
				runtime.clone(),
			)?;
			fallback::open(source, &session.cancelled)
		}
		result => result,
	};
	let decoder = decoder?;
	let result = play_decoded(decoder, session, ctx);
	// Cancellation aborts in-flight source reads; that is a clean stop, not a decode failure.
	if session.cancelled.load(Ordering::Acquire) {
		return Ok(());
	}
	result
}

fn play_decoded(
	mut decoder: platform::video::Decoder,
	session: &Session,
	ctx: &eframe::egui::Context,
) -> Result<(), &'static str> {
	use platform::video::Sample;
	use std::{
		collections::VecDeque,
		task::Poll::{Pending, Ready},
		time::{Duration, Instant},
	};
	let info = decoder.info();
	let mut target = 0.;
	let mut seeking = false;
	'seek: loop {
		if session.cancelled.load(Ordering::Acquire) {
			return Ok(());
		}
		if seeking {
			decoder.seek(target)?;
		}
		let position = Arc::new(AtomicU64::new(0));
		let eof = Arc::new(AtomicBool::new(false));
		let failed = Arc::new(AtomicBool::new(false));
		let mut output = if info.sample_rate > 0 {
			Some(output::open(
				info.sample_rate,
				output::Controls {
					cancelled: session.cancelled.clone(),
					paused: session.paused.clone(),
					seek: session.seek.clone(),
					volume: session.volume.clone(),
					position: position.clone(),
					eof: eof.clone(),
					failed: failed.clone(),
				},
			)?)
		} else {
			None
		};
		let mut frames = VecDeque::new();
		let mut seek_preview = None;
		let mut pending_audio: Option<(Vec<[f32; 2]>, usize)> = None;
		let mut queued_audio = 0u64;
		let mut video_ended = false;
		let mut audio_ended = output.is_none();
		let mut preview_needed = true;
		let mut wall = target;
		let mut last_tick = Instant::now();
		let mut last_progress = Instant::now();
		let mut previous_position = target;
		loop {
			if session.cancelled.load(Ordering::Acquire) {
				return Ok(());
			}
			let seek = session.seek.swap(u64::MAX, Ordering::AcqRel);
			if seek != u64::MAX {
				// Release the old device/ring before a potentially blocking native seek.
				drop(output);
				target = (seek as f64 / 1000.).min((info.duration - 0.001).max(0.));
				seeking = true;
				if let Ok(mut update) = session.update.lock() {
					update.frame = None;
					update.state = VideoState::Loading;
					update.position = target;
				}
				ctx.request_repaint();
				continue 'seek;
			}
			if failed.load(Ordering::Acquire) {
				return Err("Video audio output stopped");
			}
			let now = Instant::now();
			let elapsed = now.duration_since(last_tick).as_secs_f64();
			last_tick = now;
			let paused = session.paused.load(Ordering::Acquire);
			if !paused && !preview_needed {
				wall += elapsed;
			}
			let audio_position = target
				+ position.load(Ordering::Acquire) as f64 / f64::from(info.sample_rate.max(1));
			let audio_drained = audio_ended
				&& pending_audio.is_none()
				&& position.load(Ordering::Acquire) >= queued_audio;
			let current = if output.is_some() && !audio_drained {
				wall = audio_position;
				audio_position
			} else {
				wall
			};
			if current > previous_position {
				last_progress = now;
				previous_position = current;
			}
			let mut frame = None;
			while frames
				.front()
				.is_some_and(|(pts, _, _, _)| *pts <= current + 0.01 || preview_needed)
			{
				let (_, width, height, rgba) = frames.pop_front().expect("front exists");
				frame = Some((width, height, rgba));
				preview_needed = false;
			}
			let finished = video_ended && audio_drained && frames.is_empty();
			let mut changed = false;
			if let Ok(mut update) = session.update.lock() {
				let state = if finished {
					VideoState::Ended
				} else if paused {
					VideoState::Paused
				} else if preview_needed
					|| now.duration_since(last_progress) > Duration::from_millis(250)
				{
					VideoState::Loading
				} else {
					VideoState::Playing
				};
				changed = update.state != state
					|| (update.position * 10.) as u64 != (current.min(info.duration) * 10.) as u64
					|| frame.is_some();
				update.state = state;
				update.position = current.min(info.duration);
				update.duration = info.duration;
				if frame.is_some() {
					update.frame = frame;
				}
			}
			if changed {
				ctx.request_repaint();
			}
			if finished {
				return Ok(());
			}
			if paused && !preview_needed {
				last_progress = now;
				std::thread::sleep(Duration::from_millis(20));
				continue;
			}
			if now.duration_since(last_progress) > Duration::from_secs(15) {
				return Err("Video buffering stalled; retry or download to play externally");
			}
			if let Some((samples, offset)) = &mut pending_audio {
				let output = output.as_mut().ok_or("Unexpected video audio track")?;
				while *offset < samples.len() && output.producer.push(samples[*offset]).is_ok() {
					*offset += 1;
					queued_audio += 1;
				}
				if *offset == samples.len() {
					pending_audio = None;
				}
			}
			let mut decoded = false;
			// Request each track independently so short/missing audio cannot hold video EOF hostage.
			if !audio_ended && !paused && pending_audio.is_none() {
				let sample = decoder.poll_audio()?;
				decoded |= sample.is_ready();
				match sample {
					Ready(Some(Sample::Audio {
						pts,
						frames: samples,
					})) => {
						let packet_start =
							((pts - target) * f64::from(info.sample_rate)).round() as i64;
						let skip = (queued_audio as i64 - packet_start).max(0) as usize;
						if packet_start > queued_audio as i64 + info.sample_rate as i64 * 2 {
							return Err("Unsupported video audio timing");
						}
						let gap = (packet_start - queued_audio as i64).max(0) as usize;
						if gap > 0 {
							let mut padded = vec![[0.; 2]; gap];
							padded.extend_from_slice(&samples);
							pending_audio = Some((padded, 0));
						} else if skip < samples.len() {
							pending_audio = Some((samples, skip));
						}
					}
					Ready(None) => {
						audio_ended = true;
						eof.store(true, Ordering::Release);
					}
					Pending => {}
					_ => return Err("Unexpected video audio track"),
				}
			}
			// Two frames (<=16 MiB) ahead, plus one bounded audio packet and one second of PCM.
			if !video_ended && frames.len() < 2 {
				let read_started = Instant::now();
				let sample = decoder.poll_video()?;
				decoded |= sample.is_ready();
				match sample {
					Ready(Some(Sample::Video {
						pts,
						width,
						height,
						rgba,
					})) => {
						if pts >= target - 0.01 {
							seek_preview = None;
							frames.push_back((pts, width, height, rgba));
						} else {
							seek_preview = Some((target, width, height, rgba));
						}
					}
					Ready(None) => {
						video_ended = true;
						if preview_needed && let Some(frame) = seek_preview.take() {
							frames.push_back(frame);
						}
					}
					Pending => {}
					_ => return Err("Unexpected video track"),
				}
				// Freeze the silent/finished-audio clock across a blocking buffer refill.
				if (output.is_none() || audio_drained)
					&& read_started.elapsed() > Duration::from_millis(100)
				{
					last_tick = Instant::now();
				}
			}
			if !decoded {
				std::thread::sleep(Duration::from_millis(5));
			}
		}
	}
}

#[cfg(all(test, feature = "demo"))]
mod tests {
	use super::*;
	use std::time::{Duration, Instant};
	/// Synthetic local clip only; zero-volume output, no account or microphone access.
	#[test]
	#[ignore = "SEREIN_VIDEO_SAMPLE supplies an offline clip; opens muted local output"]
	fn local_video_keeps_up_with_realtime() {
		let path = std::env::var("SEREIN_VIDEO_SAMPLE").expect("SEREIN_VIDEO_SAMPLE path");
		assert!(std::fs::metadata(&path).unwrap().len() <= 100 * 1024 * 1024);
		let bytes = std::fs::read(path).unwrap();
		let session = Arc::new(Session::new(0.));
		let worker_session = session.clone();
		let started = Instant::now();
		let thread = std::thread::spawn(move || {
			let decoder = platform::video::Decoder::open(Box::new(std::io::Cursor::new(bytes)))?;
			play_decoded(decoder, &worker_session, &eframe::egui::Context::default())
		});
		while !thread.is_finished() {
			let update = session.update.lock().unwrap();
			let deadline = update.duration + 5.;
			drop(update);
			if started.elapsed().as_secs_f64() > deadline {
				session.cancelled.store(true, Ordering::Release);
				let _ = thread.join();
				panic!("playback could not keep up with realtime");
			}
			std::thread::sleep(Duration::from_millis(20));
		}
		assert_eq!(thread.join().unwrap(), Ok(()));
		let update = session.update.lock().unwrap();
		assert_eq!(update.state, VideoState::Ended);
		assert!(update.position >= update.duration - 0.1);
		eprintln!(
			"{:.3}s clip played in {:.3}s",
			update.duration,
			started.elapsed().as_secs_f64()
		);
	}

	#[test]
	#[ignore = "opens the local audio output device at zero volume; explicit offline playback check"]
	fn inline_video_plays_pauses_seeks_and_cancels() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let session = Arc::new(Session::new(0.));
		let request = Request {
			session: session.clone(),
			url: None,
			size: 120000,
		};
		let handle = runtime.handle().clone();
		let thread =
			std::thread::spawn(move || play(&request, &handle, &eframe::egui::Context::default()));
		let wait = |predicate: &dyn Fn(&Update) -> bool| {
			let start = Instant::now();
			loop {
				let update = session.update.lock().unwrap();
				assert!(
					!matches!(update.state, VideoState::Failed(_)),
					"{:?}",
					update.state
				);
				if predicate(&update) {
					break;
				}
				drop(update);
				assert!(
					start.elapsed() < Duration::from_secs(8),
					"playback timed out"
				);
				std::thread::sleep(Duration::from_millis(20));
			}
		};
		wait(&|s| s.position > 0.2 && s.frame.is_some());
		session.paused.store(true, Ordering::Release);
		wait(&|s| s.state == VideoState::Paused);
		let before = session.update.lock().unwrap().position;
		std::thread::sleep(Duration::from_millis(100));
		assert!((session.update.lock().unwrap().position - before).abs() < 0.03);
		session.seek.store(2990, Ordering::Release);
		wait(&|s| s.position >= 2.98 && s.frame.is_some());
		session.seek.store(1500, Ordering::Release);
		wait(&|s| (1.49..1.6).contains(&s.position) && s.frame.is_some());
		session.paused.store(false, Ordering::Release);
		wait(&|s| s.position > 1.7);
		session.cancelled.store(true, Ordering::Release);
		let result = thread.join().unwrap();
		assert!(result.is_ok(), "{result:?}");
		for bytes in [
			include_bytes!("../tests/fixtures/video-silent.mov").as_slice(),
			include_bytes!("../tests/fixtures/video-short-audio.mov").as_slice(),
		] {
			let session = Arc::new(Session::new(0.));
			let worker_session = session.clone();
			let thread = std::thread::spawn(move || {
				let decoder =
					platform::video::Decoder::open(Box::new(std::io::Cursor::new(bytes))).unwrap();
				play_decoded(decoder, &worker_session, &eframe::egui::Context::default())
			});
			let start = Instant::now();
			while !thread.is_finished() {
				assert!(
					start.elapsed() < Duration::from_secs(8),
					"video tail stalled"
				);
				std::thread::sleep(Duration::from_millis(20));
			}
			assert!(thread.join().unwrap().is_ok());
			assert_eq!(session.update.lock().unwrap().state, VideoState::Ended);
			assert!(session.update.lock().unwrap().position > 2.8);
		}
	}
}
