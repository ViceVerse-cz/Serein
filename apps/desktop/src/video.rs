//! One explicit inline player; native decoding and network reads stay on one lazy worker.
#[cfg(target_os = "macos")]
mod fallback;
mod output;
mod source;
use crate::gpu_video::Wake;
use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use ui::{VideoCommand, VideoOrigin, VideoState, VideoUi};

#[derive(Default)]
struct Update {
	state: VideoState,
	position: f64,
	duration: f64,
	frame: Option<crate::gpu_video::Frame>,
	/// The picture the ui showed last, on its way back to the worker, which reuses its
	/// buffer where the decoder can and frees it off the ui thread otherwise.
	spent: Option<crate::gpu_video::Frame>,
}
struct Session {
	cancelled: Arc<AtomicBool>,
	paused: Arc<AtomicBool>,
	volume: Arc<AtomicU32>,
	seek: Arc<AtomicU64>,
	/// The ui drew the seek bar and timestamp in its last pass, so position ticks need a
	/// ui pass; while they are hidden only new pictures ask for a frame.
	position_shown: AtomicBool,
	update: Mutex<Update>,
}
impl Session {
	fn new(volume: f32) -> Self {
		Self {
			cancelled: Arc::new(AtomicBool::new(false)),
			paused: Arc::new(AtomicBool::new(false)),
			volume: Arc::new(AtomicU32::new(volume.to_bits())),
			seek: Arc::new(AtomicU64::new(u64::MAX)),
			position_shown: AtomicBool::new(true),
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
	/// `None` for embed videos: the first range response reports the length.
	size: Option<usize>,
}
/// One idle-stage poster: decode a single first frame, never playback.
struct PosterJob {
	id: model::Id,
	url: Option<url::Url>,
	size: Option<usize>,
	/// Set when the session ends; skips the job and aborts its range reads.
	cancelled: Arc<AtomicBool>,
}
/// Decoded poster answer waiting for UI delivery through `poll`.
struct PosterDone {
	id: model::Id,
	frame: Option<(u32, u32, Vec<u8>)>,
	cancelled: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct Video {
	session: Option<Arc<Session>>,
	requests: Option<tokio::sync::watch::Sender<Option<Request>>>,
	poster_jobs: Option<std::sync::mpsc::Sender<PosterJob>>,
	poster_results: Arc<Mutex<Vec<PosterDone>>>,
	/// Attachments with a queued or decoding poster; dedupes the per-frame UI
	/// queue and bounds it so a scroll storm cannot pile up stale downloads.
	poster_pending: std::collections::HashSet<model::Id>,
	/// Shared by every job of the current session; replaced by [`Video::cancel_posters`].
	poster_cancelled: Arc<AtomicBool>,
	/// GPU texture of the playing attachment.
	texture: crate::gpu_video::VideoTexture,
	/// Asks the renderer for a frame that only presents a new picture.
	presenter: Option<eframe::PaintOnlyRequester>,
	/// The playback worker, parked between pictures; commands wake it.
	worker: Option<std::thread::Thread>,
}

/// In-flight poster ceiling; excess requests retry while still visible.
const POSTER_JOBS_MAX: usize = 32;
impl Video {
	pub fn stop(&mut self) {
		if let Some(session) = self.session.take() {
			session.cancelled.store(true, Ordering::Release);
			self.wake_worker();
		}
		self.texture = Default::default();
		if let Some(requests) = &self.requests {
			requests.send_replace(None);
		}
	}
	fn wake_worker(&self) {
		if let Some(worker) = &self.worker {
			worker.unpark();
		}
	}
	/// Hands over the renderer's paint-only handle once it exists; `None` keeps full repaints.
	pub fn set_presenter(&mut self, presenter: Option<eframe::PaintOnlyRequester>) {
		if self.presenter.is_none() {
			self.presenter = presenter;
		}
	}
	/// A session keeps the last pass's shapes for [`Self::paint_only`].
	pub fn active(&self) -> bool {
		self.session.is_some()
	}
	/// Tells the worker whether the ui pass that just ran drew the playback position.
	pub fn sync(&self, player: &VideoUi) {
		if let Some(session) = &self.session {
			session
				.position_shown
				.store(player.position_shown(), Ordering::Release);
		}
	}
	/// Shows the worker's newest picture in the texture the last ui pass drew, without a
	/// ui pass. `false` when the ui has to run instead: the playback state changed, or the
	/// picture needs a texture of another size. Nothing new to show is `true`.
	pub fn paint_only(&mut self, player: &VideoUi, ctx: &eframe::egui::Context) -> bool {
		let Some(session) = &self.session else {
			return true;
		};
		let Ok(mut update) = session.update.lock() else {
			return false;
		};
		if player.active.is_none() || update.state != player.state {
			return false;
		}
		match &update.frame {
			None => true,
			Some(frame) if !self.texture.fits(frame) => false,
			Some(_) => {
				let frame = update.frame.take().expect("frame checked");
				drop(update);
				let shown = self.texture.show(ctx, "inline-video", &frame).is_some();
				if let Ok(mut update) = session.update.lock() {
					update.spent = Some(frame);
				}
				shown
			}
		}
	}
	/// Session boundary: abandons queued and in-flight posters so none of their downloads
	/// continue or land in the next session's cache. Channel switches keep posters.
	pub fn cancel_posters(&mut self) {
		self.poster_cancelled.store(true, Ordering::Release);
		self.poster_cancelled = Arc::new(AtomicBool::new(false));
		self.poster_pending.clear();
		if let Ok(mut results) = self.poster_results.lock() {
			results.clear();
		}
	}
	pub fn poll(&mut self, player: &mut VideoUi, ctx: &eframe::egui::Context) {
		// Posters belong to idle stages, so they drain even with no active session.
		if let Ok(mut results) = self.poster_results.try_lock() {
			for done in results.drain(..) {
				if done.cancelled.load(Ordering::Acquire) {
					continue;
				}
				self.poster_pending.remove(&done.id);
				match done.frame {
					Some((width, height, rgba)) => {
						player.accept_poster(ctx, done.id, width as usize, height as usize, &rgba);
					}
					None => player.poster_failed(done.id),
				}
			}
		}
		let Some(session) = &self.session else {
			return;
		};
		if let Ok(mut update) = session.update.try_lock() {
			player.state = update.state;
			player.position = update.position;
			player.duration = update.duration;
			let frame = update.frame.take();
			drop(update);
			if let Some(frame) = frame
				&& player.active.is_some()
			{
				if let Some(texture) = self.texture.show(ctx, "inline-video", &frame) {
					player.show_frame(texture);
				}
				if let Ok(mut update) = session.update.lock() {
					update.spent = Some(frame);
				}
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
				if let Err(error) =
					self.start(attachment, player.origin, player.volume, runtime, ctx, demo)
				{
					player.state = VideoState::Failed(error);
				}
			}
			VideoCommand::Pause(paused) => {
				if let Some(s) = &self.session {
					s.paused.store(paused, Ordering::Release);
					self.wake_worker();
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
					self.wake_worker();
				}
			}
		}
	}
	fn start(
		&mut self,
		attachment: model::Attachment,
		origin: VideoOrigin,
		volume: f32,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) -> Result<(), &'static str> {
		if origin == VideoOrigin::Attachment
			&& (!attachment.is_video()
				|| attachment.size == 0
				|| attachment.size > 100 * 1024 * 1024)
		{
			return Err("Video preview limit: 100 MiB");
		}
		let (url, size) = source_of(&attachment, origin, demo).ok_or(match origin {
			VideoOrigin::Attachment => "Video attachment unavailable",
			VideoOrigin::Embed => "Video unavailable",
		})?;
		if self.requests.is_none() {
			let (sender, mut receiver) = tokio::sync::watch::channel::<Option<Request>>(None);
			let runtime = runtime.clone();
			let wake = Wake::new(ctx, self.presenter.clone());
			let worker = std::thread::Builder::new()
				.name("serein-attachment-video".into())
				.spawn(move || {
					while runtime.block_on(receiver.changed()).is_ok() {
						let Some(request) = receiver.borrow_and_update().clone() else {
							continue;
						};
						let result = play(&request, &runtime, &wake);
						platform::memory::release_freed_memory();
						if let Err(error) = result
							&& !request.session.cancelled.load(Ordering::Acquire)
						{
							if let Ok(mut update) = request.session.update.lock() {
								update.state = VideoState::Failed(error);
							}
							wake.ui();
						}
					}
				})
				.map_err(|_| "Could not start video worker")?;
			self.worker = Some(worker.thread().clone());
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
			size,
		}));
		self.session = Some(session);
		Ok(())
	}
	/// Queues one batch of idle-stage posters; failures surface through `poll`,
	/// never playback state. Already-pending attachments are skipped, and past
	/// the ceiling the rest retry while still visible.
	pub fn posters(
		&mut self,
		queue: Vec<(model::Attachment, VideoOrigin)>,
		player: &mut VideoUi,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) {
		for (attachment, origin) in queue {
			if self.poster_pending.contains(&attachment.id)
				|| self.poster_pending.len() >= POSTER_JOBS_MAX
			{
				continue;
			}
			self.poster(attachment, origin, player, runtime, ctx, demo);
		}
	}
	/// Queues one idle-stage poster; failures surface through `poll`, never playback state.
	fn poster(
		&mut self,
		attachment: model::Attachment,
		origin: VideoOrigin,
		player: &mut VideoUi,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) {
		let id = attachment.id;
		let Some((url, size)) = source_of(&attachment, origin, demo) else {
			player.poster_failed(id);
			return;
		};
		if self.poster_jobs.is_none() {
			let (sender, receiver) = std::sync::mpsc::channel::<PosterJob>();
			let runtime = runtime.clone();
			let ctx = ctx.clone();
			let results = self.poster_results.clone();
			if std::thread::Builder::new()
				.name("serein-video-poster".into())
				.spawn(move || {
					// Newest first: after a scroll or channel switch the visible stages
					// decode before posters requested for stages already scrolled past.
					let mut stack = Vec::new();
					loop {
						if stack.is_empty() {
							let Ok(job) = receiver.try_recv().or_else(|_| {
								// The batch is done: return its decoders' memory once.
								platform::memory::release_freed_memory();
								receiver.recv()
							}) else {
								break;
							};
							stack.push(job);
						}
						stack.extend(receiver.try_iter());
						let Some(job) = stack.pop() else {
							continue;
						};
						if job.cancelled.load(Ordering::Acquire) {
							continue;
						}
						let frame = decode_poster(&job, &runtime);
						if let Ok(mut results) = results.lock() {
							results.push(PosterDone {
								id: job.id,
								frame,
								cancelled: job.cancelled,
							});
						}
						ctx.request_repaint();
					}
				})
				.is_err()
			{
				player.poster_failed(id);
				return;
			}
			self.poster_jobs = Some(sender);
		}
		if self.poster_jobs.as_ref().is_some_and(|jobs| {
			jobs.send(PosterJob {
				id,
				url,
				size,
				cancelled: self.poster_cancelled.clone(),
			})
			.is_ok()
		}) {
			self.poster_pending.insert(id);
			return;
		}
		player.poster_failed(id);
	}
}

/// The admitted URL and known byte size of a playable video; the demo plays its fixture.
/// Attachments stream their signed CDN original; embeds stream Discord's proxied copy and
/// learn the size on load.
fn source_of(
	attachment: &model::Attachment,
	origin: VideoOrigin,
	demo: bool,
) -> Option<(Option<url::Url>, Option<usize>)> {
	let size = match origin {
		VideoOrigin::Attachment => {
			let size = attachment.size as usize;
			if size == 0 || size > 100 * 1024 * 1024 {
				return None;
			}
			Some(size)
		}
		VideoOrigin::Embed => None,
	};
	if demo {
		return Some((None, size));
	}
	let url = match origin {
		VideoOrigin::Attachment => crate::downloads::original_url(attachment)?,
		VideoOrigin::Embed => {
			crate::avatars::embed_video_url(attachment.media.proxy_url.as_deref()?)?
		}
	};
	Some((Some(url), size))
}

/// Opens the attachment and returns its first decoded video frame, downscaled for
/// a light poster; bounded reads only.
fn decode_poster(job: &PosterJob, runtime: &tokio::runtime::Handle) -> Option<(u32, u32, Vec<u8>)> {
	use platform::video::Sample;
	use std::{
		task::Poll::Ready,
		time::{Duration, Instant},
	};
	let source = source::source(
		job.url.clone(),
		job.size,
		job.cancelled.clone(),
		runtime.clone(),
	)
	.ok()?;
	let mut decoder = platform::video::Decoder::open(source).ok()?;
	// Backends that signal `Pending` (Linux GStreamer) need waiting, not a poll
	// count: give the first frame a bounded ten seconds, then mark it failed.
	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		if Instant::now() >= deadline || job.cancelled.load(Ordering::Acquire) {
			return None;
		}
		match decoder.poll_video().ok()? {
			// Posters are converted once on the CPU, then cached as ordinary textures.
			Ready(Some(Sample::Video { picture, .. })) => {
				let (width, height, rgba) = picture.to_rgba()?;
				return Some(downscale_poster(width, height, rgba));
			}
			Ready(None) => return None,
			Ready(Some(_)) => {}
			std::task::Poll::Pending => {
				// Unsynchronised GStreamer sinks do not drop: drain queued audio
				// so a full audio branch cannot stall the first video sample.
				for _ in 0..64 {
					if !matches!(decoder.poll_audio(), Ok(Ready(Some(_)))) {
						break;
					}
				}
				std::thread::sleep(Duration::from_millis(5));
			}
		}
	}
}

/// Longest poster edge in pixels; a 1080p frame shrinks ~10x before caching.
const POSTER_EDGE: u32 = 320;

/// Area-average downscale to [`POSTER_EDGE`]; small frames pass through untouched.
fn downscale_poster(width: u32, height: u32, rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
	let edge = width.max(height);
	if edge <= POSTER_EDGE
		|| width == 0
		|| height == 0
		|| rgba.len() != width as usize * height as usize * 4
	{
		return (width, height, rgba);
	}
	let out_width = (u64::from(width) * u64::from(POSTER_EDGE) / u64::from(edge)).max(1) as u32;
	let out_height = (u64::from(height) * u64::from(POSTER_EDGE) / u64::from(edge)).max(1) as u32;
	let mut out = vec![0; out_width as usize * out_height as usize * 4];
	for y in 0..out_height {
		let y0 = y * height / out_height;
		let y1 = ((y + 1) * height / out_height).max(y0 + 1);
		for x in 0..out_width {
			let x0 = x * width / out_width;
			let x1 = ((x + 1) * width / out_width).max(x0 + 1);
			let mut sum = [0_u64; 4];
			let mut count = 0_u64;
			for sy in y0..y1 {
				for sx in x0..x1 {
					let pixel = &rgba[(sy as usize * width as usize + sx as usize) * 4..][..4];
					for (acc, byte) in sum.iter_mut().zip(pixel) {
						*acc += u64::from(*byte);
					}
					count += 1;
				}
			}
			let pixel = &mut out[(y as usize * out_width as usize + x as usize) * 4..][..4];
			for (slot, acc) in pixel.iter_mut().zip(sum) {
				*slot = (acc / count) as u8;
			}
		}
	}
	(out_width, out_height, out)
}
impl Drop for Video {
	fn drop(&mut self) {
		self.stop();
	}
}

fn play(
	request: &Request,
	runtime: &tokio::runtime::Handle,
	wake: &Wake,
) -> Result<(), &'static str> {
	use platform::video::Decoder;
	let session = &request.session;
	let source = source::source(
		request.url.clone(),
		request.size,
		session.cancelled.clone(),
		runtime.clone(),
	)?;
	let result = Decoder::open(source).and_then(|decoder| play_decoded(decoder, session, wake));
	#[cfg(target_os = "macos")]
	let result = match result {
		Err(error) if fallback::eligible(error) && !session.cancelled.load(Ordering::Acquire) => {
			// Native codecs can fail on the first packet or later in the movie, not just
			// while opening it. Retry conversion once, after releasing the old decoder
			// and audio output, and resume from the last displayed position.
			let resume = if let Ok(mut update) = session.update.lock() {
				update.state = VideoState::Loading;
				update.position
			} else {
				0.
			};
			wake.ui();
			let source = source::source(
				request.url.clone(),
				request.size,
				session.cancelled.clone(),
				runtime.clone(),
			)?;
			let decoder = fallback::open(source, &session.cancelled)?;
			// A seek made during conversion takes precedence over automatic resumption.
			let _ = session.seek.compare_exchange(
				u64::MAX,
				(resume * 1000.) as u64,
				Ordering::AcqRel,
				Ordering::Acquire,
			);
			play_decoded(decoder, session, wake)
		}
		result => result,
	};
	// Cancellation aborts in-flight source reads; that is a clean stop, not a decode failure.
	if session.cancelled.load(Ordering::Acquire) {
		return Ok(());
	}
	result
}

/// Beyond this gap the playback clock jumps to the audio device's position instead of
/// being pulled toward it.
const CLOCK_SNAP: f64 = 0.1;
/// Share of the gap to the audio device's position closed per second.
const CLOCK_PULL: f64 = 4.0;
/// Longest sleep with every queue full: the audio ring holds a second, so refilling it
/// this often is ample, and a finished clip is noticed this soon.
const IDLE_WAIT: f64 = 0.05;
/// Pictures are handed over this long before they are due: the renderer shows them at
/// its next refresh.
const PICTURE_LEAD: f64 = 0.01;

/// The playback clock after `elapsed` seconds: `wall` has already advanced by them and is
/// pulled toward the audio `device`'s position. `hold` (paused, or waiting for a first
/// picture) and gaps beyond [`CLOCK_SNAP`] take the device's position as it is.
fn steer(wall: f64, device: f64, elapsed: f64, hold: bool) -> f64 {
	let gap = device - wall;
	if hold || gap.abs() > CLOCK_SNAP {
		device
	} else {
		wall + gap * (elapsed * CLOCK_PULL).min(1.0)
	}
}

fn play_decoded(
	mut decoder: platform::video::Decoder,
	session: &Session,
	wake: &Wake,
) -> Result<(), &'static str> {
	use crate::gpu_video::Frame;
	use platform::video::Sample;
	use std::{
		collections::VecDeque,
		task::Poll::{Pending, Ready},
		time::{Duration, Instant},
	};
	let info = decoder.info();
	// The GPU samples VideoToolbox's own surfaces; no plane is copied through memory.
	#[cfg(target_os = "macos")]
	decoder.surfaces(crate::gpu_video::installed());
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
				wake.ui();
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
				// The device reports its position a buffer at a time, in steps of about
				// 10 ms. Showing pictures by those steps puts 60 fps frames a refresh early
				// or late, so the wall clock runs between the steps and is pulled toward the
				// device, which keeps the two from drifting apart.
				wall = steer(wall, audio_position, elapsed, paused || preview_needed);
				wall
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
				.is_some_and(|(pts, _)| *pts <= current + PICTURE_LEAD || preview_needed)
			{
				let (_, picture) = frames.pop_front().expect("front exists");
				frame = Some(picture);
				preview_needed = false;
			}
			let finished = video_ended && audio_drained && frames.is_empty();
			let (mut changed, mut moved) = (false, false);
			let pictured = frame.is_some();
			let mut spent = None;
			if let Ok(mut update) = session.update.lock() {
				spent = update.spent.take();
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
				changed = update.state != state;
				moved = (update.position * 10.) as u64 != (current.min(info.duration) * 10.) as u64;
				update.state = state;
				update.position = current.min(info.duration);
				update.duration = info.duration;
				if frame.is_some() {
					update.frame = frame;
				}
			}
			if changed || (moved && session.position_shown.load(Ordering::Acquire)) {
				wake.ui();
			} else if pictured {
				wake.picture();
			}
			#[cfg(target_os = "linux")]
			if let Some(Frame::Packed(picture)) = spent {
				decoder.recycle(picture);
			}
			#[cfg(not(target_os = "linux"))]
			drop(spent);
			if finished {
				return Ok(());
			}
			if paused && !preview_needed {
				last_progress = now;
				// Resume, seek and stop unpark the worker.
				std::thread::park_timeout(Duration::from_millis(250));
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
			// Two frames ahead (<=25 MiB as 4K NV12, 64 MiB as RGBA), plus one bounded
			// audio packet and one second of PCM.
			if !video_ended && frames.len() < 2 {
				let read_started = Instant::now();
				let sample = match decoder.poll_video()? {
					Ready(Some(Sample::Video { pts, picture })) => {
						Ready(Some((pts, Frame::Packed(picture))))
					}
					#[cfg(target_os = "macos")]
					Ready(Some(Sample::Surface { pts, surface })) => Ready(Some((pts, Frame::Surface(surface)))),
					Ready(Some(Sample::Audio { .. })) => return Err("Unexpected video track"),
					Ready(None) => Ready(None),
					Pending => Pending,
				};
				decoded |= sample.is_ready();
				match sample {
					Ready(Some((pts, picture))) => {
						if pts >= target - 0.01 {
							seek_preview = None;
							frames.push_back((pts, picture));
						} else {
							seek_preview = Some((target, picture));
						}
					}
					Ready(None) => {
						video_ended = true;
						if preview_needed && let Some(frame) = seek_preview.take() {
							frames.push_back(frame);
						}
					}
					Pending => {}
				}
				// Freeze the silent/finished-audio clock across a blocking buffer refill.
				if (output.is_none() || audio_drained)
					&& read_started.elapsed() > Duration::from_millis(100)
				{
					last_tick = Instant::now();
				}
			}
			if !decoded {
				// With both queues full the next thing to do is hand over the next picture,
				// so sleep until it is due (commands unpark the worker). A decoder that has
				// nothing ready yet is asked again shortly.
				let queued =
					(video_ended || frames.len() >= 2) && (audio_ended || pending_audio.is_some());
				let wait = if queued {
					frames
						.front()
						.map_or(IDLE_WAIT, |(pts, _)| pts - PICTURE_LEAD - current)
						.clamp(0.001, IDLE_WAIT)
				} else {
					0.005
				};
				std::thread::park_timeout(Duration::from_secs_f64(wait));
			}
		}
	}
}

#[cfg(test)]
mod clock_tests {
	use super::*;
	/// The audio device reports its position in buffer-sized steps and runs slightly off
	/// the wall clock. The playback clock must advance evenly between those steps (60 fps
	/// pictures are scheduled by it), stay with the device over time, and jump only when
	/// the device does.
	#[test]
	fn clock_runs_evenly_between_device_steps_and_follows_drift() {
		const TICK: f64 = 1.0 / 60.0;
		// 512 frames at 48 kHz per step; the device runs 0.1% fast.
		let device = |time: f64| (time * 1.001 / (512.0 / 48_000.0)).floor() * (512.0 / 48_000.0);
		let mut wall = 0.0;
		for tick in 1..=3_600 {
			let time = f64::from(tick) * TICK;
			let next = steer(wall + TICK, device(time), TICK, false);
			let step = next - wall;
			assert!((step - TICK).abs() < 0.002, "uneven step {step} at {time}");
			wall = next;
		}
		// A minute in, the device is 60 ms ahead of the wall clock; the playback clock is with it.
		assert!((wall - device(60.0)).abs() < 0.012, "{wall}");
		// A seek or a stall moves the device far away at once, and the clock goes with it.
		assert_eq!(steer(wall, wall + 5.0, TICK, false), wall + 5.0);
		assert_eq!(steer(wall, 1.0, TICK, true), 1.0);
	}
}

#[cfg(test)]
mod poster_tests {
	use super::*;
	/// Small frames pass through untouched; large ones shrink to the poster edge averaged.
	#[test]
	fn downscale_shrinks_and_averages() {
		// 4x2 gradient: left half black, right half white.
		let mut rgba = vec![0; 4 * 2 * 4];
		for pixel in rgba.as_chunks_mut::<4>().0.iter_mut().skip(4) {
			pixel.copy_from_slice(&[255, 255, 255, 255]);
		}
		let (width, height, small) = downscale_poster(4, 2, rgba);
		assert_eq!((width, height, small.len()), (4, 2, 4 * 2 * 4));
		// A 640x480 frame lands on a 320px edge with averaged texels.
		let wide = vec![128; 640 * 480 * 4];
		let (width, height, small) = downscale_poster(640, 480, wide);
		assert_eq!((width, height), (320, 240));
		assert_eq!(small.len(), 320 * 240 * 4);
		assert!(small.iter().all(|&byte| byte == 128));
		let tall = vec![64; 720 * 1280 * 4];
		let (width, height, small) = downscale_poster(720, 1280, tall);
		assert_eq!((width, height), (180, 320));
		assert_eq!(small.len(), 180 * 320 * 4);
	}
}
