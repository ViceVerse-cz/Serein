//! One explicit inline player; native decoding and network reads stay on one lazy worker.
#[cfg(target_os = "macos")]
mod fallback;
pub(crate) mod output;
mod source;
use platform::video::ffmpeg::AttachmentDecoder;
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
	frame: Option<DisplayFrame>,
}
struct Session {
	cancelled: Arc<AtomicBool>,
	paused: Arc<AtomicBool>,
	volume: Arc<AtomicU32>,
	seek: Arc<AtomicU64>,
	update: Mutex<Update>,
	startup: source::Startup,
	#[cfg(test)]
	synthetic_output: bool,
}
impl Session {
	fn new(volume: f32) -> Self {
		Self {
			cancelled: Arc::new(AtomicBool::new(false)),
			paused: Arc::new(AtomicBool::new(false)),
			volume: Arc::new(AtomicU32::new(volume.to_bits())),
			seek: Arc::new(AtomicU64::new(u64::MAX)),
			startup: source::Startup::new(
				std::time::Instant::now() + std::time::Duration::from_secs(30),
			),
			#[cfg(test)]
			synthetic_output: false,
			update: Mutex::new(Update {
				state: VideoState::Loading,
				..Default::default()
			}),
		}
	}
}
#[derive(Clone)]
struct Request {
	output: Option<String>,
	session: Arc<Session>,
	url: Option<url::Url>,
	fallback: Option<url::Url>,
	size: usize,
	adapter: Option<model::VideoAdapter>,
}
/// One idle-stage poster: decode a single first frame, never playback.
struct PosterJob {
	id: model::Id,
	url: Option<url::Url>,
	size: usize,
	/// Set when the session ends; skips the job and aborts its range reads.
	cancelled: Arc<AtomicBool>,
	_permit: tokio::sync::OwnedSemaphorePermit,
}
struct PosterWorker {
	send: std::sync::mpsc::Sender<PosterJob>,
	slots: Arc<tokio::sync::Semaphore>,
}
/// Decoded poster answer waiting for UI delivery through `poll`.
struct PosterDone {
	id: model::Id,
	frame: Option<(u32, u32, Vec<u8>)>,
	cancelled: Arc<AtomicBool>,
}
fn provider_geometry(
	full: eframe::egui::Rect,
	clip: eframe::egui::Rect,
	scale: f32,
) -> Option<(
	platform::embed_player::Bounds,
	platform::embed_player::Bounds,
)> {
	if !full.is_finite() || !clip.is_finite() || !scale.is_finite() || scale <= 0.0 {
		return None;
	}
	let clip = full.intersect(clip);
	let bounds = |rect: eframe::egui::Rect, clipped: bool| {
		let edge = |value: f32, near: bool| {
			let n = value * scale;
			if clipped {
				if near { n.ceil() } else { n.floor() }
			} else {
				n.round()
			}
		};
		let x = edge(rect.left(), true) as i32;
		let y = edge(rect.top(), true) as i32;
		let (width, height) = if clipped {
			(
				u32::try_from(i64::from(edge(rect.right(), false) as i32) - i64::from(x)).ok()?,
				u32::try_from(i64::from(edge(rect.bottom(), false) as i32) - i64::from(y)).ok()?,
			)
		} else {
			(
				(rect.width() * scale).round() as u32,
				(rect.height() * scale).round() as u32,
			)
		};
		(width > 0 && height > 0).then_some(platform::embed_player::Bounds {
			x,
			y,
			width,
			height,
		})
	};
	Some((bounds(full, false)?, bounds(clip, true)?))
}

struct ProviderSession {
	generation: u64,
	message: model::Id,
	channel: model::Id,
	target: model::YouTubeVideo,
	view: platform::embed_player::ProviderPlayer,
}
#[derive(Default)]
pub struct Video {
	adapter: Option<model::VideoAdapter>,
	pub output: Option<String>,
	session: Option<Arc<Session>>,
	provider: Option<ProviderSession>,
	requests: Option<tokio::sync::watch::Sender<Option<Request>>>,
	poster_worker: Option<PosterWorker>,
	poster_results: Arc<Mutex<Vec<PosterDone>>>,
	/// Attachments with a queued or decoding poster; dedupes the per-frame UI
	/// queue and bounds it so a scroll storm cannot pile up stale downloads.
	poster_pending: std::collections::HashSet<model::Id>,
	/// Shared by every job of the current session; replaced by [`Video::cancel_posters`].
	poster_cancelled: Arc<AtomicBool>,
	/// GPU texture of the playing attachment.
	texture: crate::gpu_video::VideoTexture,
}

/// In-flight poster ceiling; excess requests retry while still visible.
const POSTER_JOBS_MAX: usize = 32;
impl Video {
	pub fn for_adapter(adapter: model::VideoAdapter) -> Self {
		Self {
			adapter: Some(adapter),
			output: None,
			session: None,
			provider: None,
			requests: None,
			poster_worker: None,
			poster_results: Default::default(),
			poster_pending: Default::default(),
			poster_cancelled: Default::default(),
			texture: Default::default(),
		}
	}
	pub fn stop(&mut self) {
		self.provider = None;
		if let Some(session) = self.session.take() {
			session.cancelled.store(true, Ordering::Release);
		}
		self.texture = Default::default();
		if let Some(requests) = &self.requests {
			requests.send_replace(None);
		}
	}
	pub fn sync_provider(
		&mut self,
		player: &mut VideoUi,
		generation: u64,
		window: &Arc<winit::window::Window>,
		ctx: &eframe::egui::Context,
		allowed: bool,
	) {
		if !allowed
			|| self
				.provider
				.as_ref()
				.is_some_and(|session| session.generation != generation)
		{
			self.provider = None;
			player.provider = None;
			return;
		}
		let Some(request) = &mut player.provider else {
			self.provider = None;
			return;
		};
		if request.generation != generation {
			self.provider = None;
			player.provider = None;
			return;
		}
		if self.provider.as_ref().is_some_and(|session| {
			session.message != request.message
				|| session.channel != request.channel
				|| session.target != request.target
		}) {
			self.provider = None;
		}
		if request.error.is_some() {
			self.provider = None;
			return;
		}
		let overlay = eframe::egui::Popup::is_any_open(ctx)
			|| ctx.memory(|memory| {
				memory
					.top_modal_layer()
					.is_some_and(|layer| layer.id != eframe::egui::Id::unique("provider-video"))
			});
		let geometry = request
			.bounds
			.zip(request.clip)
			.and_then(|(full, clip)| provider_geometry(full, clip, ctx.pixels_per_point()));
		let visible = geometry.is_some()
			&& player.seen
			&& request.bounds.is_some()
			&& request.clip.is_some()
			&& !overlay;
		if request.start_requested && visible {
			request.start_requested = false;
			self.provider = None;
			let wake = ctx.clone();
			match platform::embed_player::ProviderPlayer::open(
				window.clone(),
				&request.target,
				move || wake.request_repaint(),
			) {
				Ok(view) => {
					request.standalone = cfg!(target_os = "linux");
					self.provider = Some(ProviderSession {
						generation,
						message: request.message,
						channel: request.channel,
						target: request.target.clone(),
						view,
					});
				}
				Err(error) => {
					request.error = Some(error);
					ctx.request_repaint();
					return;
				}
			}
		}
		let Some(session) = &self.provider else {
			return;
		};

		let result = (|| {
			if visible && let Some((full, clip)) = geometry {
				session.view.set_bounds(full, clip)?;
			}
			session.view.set_visible(visible)?;
			session.view.poll()
		})();
		request.state = match session.view.status().state {
			platform::embed_player::PlaybackState::Playing => VideoState::Playing,
			platform::embed_player::PlaybackState::Paused => VideoState::Paused,
			platform::embed_player::PlaybackState::Ended => VideoState::Ended,
			_ => VideoState::Loading,
		};

		match result {
			Ok(true) => ctx.request_repaint_after(if cfg!(target_os = "linux") {
				std::time::Duration::from_millis(20)
			} else {
				std::time::Duration::from_secs(1)
			}),
			Ok(false) => {
				self.provider = None;
				player.provider = None;
				ctx.request_repaint();
			}
			Err(error) => {
				self.provider = None;
				request.error = Some(error);
				ctx.request_repaint();
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
				match frame {
					DisplayFrame::Native(picture) => {
						if let Some(texture) =
							self.texture.upload(ctx, "inline-video", picture.borrow())
						{
							player.show_frame(texture);
						}
					}
					DisplayFrame::Picture(frame) => {
						let view = media_view(frame, player.media.as_ref());
						self.texture = Default::default();
						player.show_media(view);
					}
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
		let (url, fallback) = if demo {
			(None, None)
		} else if attachment.size == 0 {
			// Embed videos have no declared size; the worker probes it before decoding.
			let mut candidates = ui::embedded_video_candidates(&attachment.media).into_iter();
			let url = candidates.next().ok_or("Video embed unavailable")?;
			(Some(url), candidates.next())
		} else {
			(
				Some(
					crate::downloads::original_url(&attachment)
						.ok_or("Video attachment unavailable")?,
				),
				None,
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
						let result = play(&request, &runtime, &ctx);
						platform::memory::release_freed_memory();
						if let Err(error) = result
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
			return Err("Video worker stopped; restart NuSerein");
		}
		let session = Arc::new(Session::new(volume));
		requests.send_replace(Some(Request {
			output: self.output.clone(),
			session: session.clone(),
			url,
			fallback,
			size: attachment.size as usize,
			adapter: self.adapter,
		}));
		self.session = Some(session);
		Ok(())
	}
	/// Queues one batch of idle-stage posters; failures surface through `poll`,
	/// never playback state. Already-pending attachments are skipped, and past
	/// the ceiling the rest retry while still visible.
	pub fn posters(
		&mut self,
		queue: Vec<model::Attachment>,
		player: &mut VideoUi,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) {
		for attachment in queue {
			if self.poster_pending.contains(&attachment.id)
				|| self.poster_pending.len() >= POSTER_JOBS_MAX
			{
				continue;
			}
			self.poster(attachment, player, runtime, ctx, demo);
		}
	}
	/// Queues one idle-stage poster; failures surface through `poll`, never playback state.
	fn poster(
		&mut self,
		attachment: model::Attachment,
		player: &mut VideoUi,
		runtime: &tokio::runtime::Handle,
		ctx: &eframe::egui::Context,
		demo: bool,
	) {
		let id = attachment.id;
		if attachment.size == 0 || attachment.size > 100 * 1024 * 1024 {
			player.poster_failed(id);
			return;
		}
		let url = if demo {
			None
		} else {
			match crate::downloads::original_url(&attachment) {
				Some(url) => Some(url),
				None => {
					player.poster_failed(id);
					return;
				}
			}
		};
		if self.poster_worker.is_none() {
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
						if !job.cancelled.load(Ordering::Acquire) {
							let frame = decode_poster(&job, &runtime);
							if let Ok(mut results) = results.lock()
								&& !job.cancelled.load(Ordering::Acquire)
							{
								results.push(PosterDone {
									id: job.id,
									frame,
									cancelled: job.cancelled.clone(),
								});
							}
						}
						drop(job);
						ctx.request_repaint();
					}
				})
				.is_err()
			{
				player.poster_failed(id);
				return;
			}
			self.poster_worker = Some(PosterWorker {
				send: sender,
				slots: Arc::new(tokio::sync::Semaphore::new(POSTER_JOBS_MAX)),
			});
		}
		let worker = self.poster_worker.as_ref().expect("poster worker created");
		// Retired jobs keep their slots until drained, including a decoder still running.
		let Ok(permit) = worker.slots.clone().try_acquire_owned() else {
			return;
		};
		if worker
			.send
			.send(PosterJob {
				id,
				url,
				size: attachment.size as usize,
				cancelled: self.poster_cancelled.clone(),
				_permit: permit,
			})
			.is_ok()
		{
			self.poster_pending.insert(id);
			return;
		}
		player.poster_failed(id);
	}
}

/// Opens the attachment and returns its first decoded video frame, downscaled for
/// a light poster; bounded reads only.
fn decode_poster(job: &PosterJob, runtime: &tokio::runtime::Handle) -> Option<(u32, u32, Vec<u8>)> {
	let source = source::source(
		job.url.clone(),
		job.size,
		job.cancelled.clone(),
		runtime.clone(),
	)
	.ok()?;
	poster_frame(source, &job.cancelled)
}

fn poster_frame(
	source: Box<dyn platform::video::ReadSeek>,
	cancelled: &AtomicBool,
) -> Option<(u32, u32, Vec<u8>)> {
	use platform::video::ffmpeg::Sample;
	use std::{
		task::Poll::Ready,
		time::{Duration, Instant},
	};
	if cancelled.load(Ordering::Acquire) {
		return None;
	}
	let mut decoder = AttachmentDecoder::open(source, None).ok()?;
	// Backends that signal `Pending` (Linux GStreamer) need waiting, not a poll
	// count: give the first frame a bounded ten seconds, then mark it failed.
	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		if Instant::now() >= deadline || cancelled.load(Ordering::Acquire) {
			return None;
		}
		match decoder.poll_video().ok()? {
			// Posters are converted once on the CPU, then cached as ordinary textures.
			Ready(Some(Sample::Picture { frame, .. })) => {
				return frame.thumbnail(POSTER_EDGE).ok();
			}
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
		self.poster_cancelled.store(true, Ordering::Release);
	}
}

fn play(
	request: &Request,
	runtime: &tokio::runtime::Handle,
	ctx: &eframe::egui::Context,
) -> Result<(), &'static str> {
	let result = play_candidate(request, runtime, ctx);
	if result.is_ok() || request.session.cancelled.load(Ordering::Acquire) {
		return result;
	}
	if request.session.startup.expired() {
		return Err(request.session.startup.timeout_error());
	}
	let Some(fallback) = &request.fallback else {
		return result;
	};
	let mut retry = request.clone();
	retry.url = Some(fallback.clone());
	retry.fallback = None;
	let resume = if let Ok(mut update) = request.session.update.lock() {
		update.state = VideoState::Loading;
		update.position
	} else {
		0.0
	};
	if resume > 0.0 {
		let _ = request.session.seek.compare_exchange(
			u64::MAX,
			(resume * 1000.0) as u64,
			Ordering::AcqRel,
			Ordering::Acquire,
		);
	}
	ctx.request_repaint();
	let result = play_candidate(&retry, runtime, ctx);
	if request.session.startup.expired() && !request.session.cancelled.load(Ordering::Acquire) {
		Err(request.session.startup.timeout_error())
	} else {
		result
	}
}

fn play_candidate(
	request: &Request,
	runtime: &tokio::runtime::Handle,
	ctx: &eframe::egui::Context,
) -> Result<(), &'static str> {
	let session = &request.session;
	let source = source::source_until(
		request.url.clone(),
		request.size,
		session.cancelled.clone(),
		runtime.clone(),
		Some(session.startup.clone()),
	)?;
	let result = AttachmentDecoder::open(source, request.adapter)
		.and_then(|decoder| play_decoded(decoder, session, request.output.as_deref(), ctx));
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
			ctx.request_repaint();
			let source = source::source_until(
				request.url.clone(),
				request.size,
				session.cancelled.clone(),
				runtime.clone(),
				Some(session.startup.clone()),
			)?;
			let decoder = fallback::open(source, &session.cancelled, request.adapter, || {
				session
					.startup
					.expired()
					.then(|| session.startup.timeout_error())
			})?;
			// A seek made during conversion takes precedence over automatic resumption.
			let _ = session.seek.compare_exchange(
				u64::MAX,
				(resume * 1000.) as u64,
				Ordering::AcqRel,
				Ordering::Acquire,
			);
			play_decoded(decoder, session, request.output.as_deref(), ctx)
		}
		result => result,
	};
	// Cancellation aborts in-flight source reads; that is a clean stop, not a decode failure.
	if session.cancelled.load(Ordering::Acquire) {
		return Ok(());
	}
	result
}

fn play_decoded(
	decoder: impl Into<AttachmentDecoder>,
	session: &Session,
	selected_output: Option<&str>,
	ctx: &eframe::egui::Context,
) -> Result<(), &'static str> {
	let mut decoder = decoder.into();
	use platform::video::ffmpeg::Sample;
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
			let controls = output::Controls {
				cancelled: session.cancelled.clone(),
				paused: session.paused.clone(),
				seek: session.seek.clone(),
				volume: session.volume.clone(),
				position: position.clone(),
				eof: eof.clone(),
				failed: failed.clone(),
				revision: Arc::new(AtomicU64::new(0)),
			};
			#[cfg(test)]
			let synthetic = session.synthetic_output;
			#[cfg(not(test))]
			let synthetic = false;
			if synthetic {
				#[cfg(test)]
				{
					Some(output::synthetic(
						info.sample_rate,
						info.sample_rate as usize,
						controls,
					))
				}
				#[cfg(not(test))]
				{
					unreachable!()
				}
			} else {
				Some(output::open(
					info.sample_rate,
					selected_output,
					info.sample_rate as usize,
					controls,
				)?)
			}
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
		let preview_started = Instant::now();
		let mut wall = target;
		let mut last_tick = Instant::now();
		let mut last_progress = Instant::now();
		let mut previous_position = target;
		loop {
			if session.cancelled.load(Ordering::Acquire) {
				return Ok(());
			}
			if decoder.take_restart() {
				// The shared decoder rewound both tracks after hardware rejected its
				// first picture. Retire queued audio before restarting the same seek.
				drop(output);
				seeking = true;
				continue 'seek;
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
			// A wall clock alone is not decoder progress. Silent video must have a
			// decoded picture queued before its presentation clock can extend the deadline.
			if current > previous_position
				&& ((output.is_some() && !audio_drained) || !frames.is_empty())
			{
				last_progress = now;
				session.startup.progressed();
				previous_position = current;
			}
			let mut frame = None;
			while frames
				.front()
				.is_some_and(|(pts, _)| *pts <= current + 0.01 || preview_needed)
			{
				let (_, picture) = frames.pop_front().expect("front exists");
				frame = Some(picture);
				preview_needed = false;
			}
			let finished = video_ended && audio_drained && frames.is_empty();
			if finished && preview_needed {
				return Err("Video has no playable pictures; retry or open the original");
			}
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
					session.startup.progressed();
					session.startup.first_picture.store(true, Ordering::Release);
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
				session.startup.progressed();
				last_progress = now;
				std::thread::sleep(Duration::from_millis(20));
				continue;
			}
			if session.startup.expired() {
				return Err(session.startup.timeout_error());
			}
			if (preview_needed
				&& session.startup.first_picture.load(Ordering::Acquire)
				&& now.duration_since(preview_started) > Duration::from_secs(15))
				|| (!preview_needed && now.duration_since(last_progress) > Duration::from_secs(15))
			{
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
			// Two byte-accounted pictures ahead, plus one bounded audio packet and one second of PCM.
			if !video_ended && frames.len() < 2 {
				let read_started = Instant::now();
				let sample = decoder.poll_video()?;
				decoded |= sample.is_ready();
				match sample {
					Ready(Some(Sample::Video { pts, picture })) => {
						if pts >= target - 0.01 {
							seek_preview = None;
							frames.push_back((pts, DisplayFrame::Native(picture)));
						} else {
							seek_preview = Some((target, DisplayFrame::Native(picture)));
						}
					}
					Ready(Some(Sample::Picture { pts, frame })) => {
						let frame = DisplayFrame::Picture(Arc::new(frame));
						if pts >= target - 0.01 {
							seek_preview = None;
							frames.push_back((pts, frame));
						} else {
							seek_preview = Some((target, frame));
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

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::{Duration, Instant};

	#[test]
	fn cancelled_poster_jobs_hold_capacity_until_the_worker_retires_them() {
		let runtime = tokio::runtime::Builder::new_current_thread()
			.build()
			.unwrap();
		let ctx = eframe::egui::Context::default();
		let (send, receive) = std::sync::mpsc::channel();
		let mut video = Video::for_adapter(model::VideoAdapter::default());
		video.poster_worker = Some(PosterWorker {
			send,
			slots: Arc::new(tokio::sync::Semaphore::new(POSTER_JOBS_MAX)),
		});
		let mut player = VideoUi::default();
		let attachment = |id| model::Attachment {
			id: model::Id(id),
			filename: "fixture.mov".into(),
			description: None,
			content_type: Some("video/quicktime".into()),
			size: 1024,
			media: Default::default(),
			spoiler: false,
			duration_ms: None,
			waveform: vec![],
		};
		for id in 1..=POSTER_JOBS_MAX as u64 {
			video.poster(attachment(id), &mut player, runtime.handle(), &ctx, true);
		}
		for _ in 0..100 {
			video.cancel_posters();
			video.poster(attachment(100), &mut player, runtime.handle(), &ctx, true);
			assert!(video.poster_pending.is_empty());
		}
		let retired: Vec<_> = receive.try_iter().collect();
		assert_eq!(retired.len(), POSTER_JOBS_MAX);
		assert!(
			retired
				.iter()
				.all(|job| job.cancelled.load(Ordering::Acquire))
		);
		drop(retired);
		video.poster(attachment(100), &mut player, runtime.handle(), &ctx, true);
		let current = receive.try_recv().unwrap();
		assert_eq!(current.id, model::Id(100));
		assert!(!current.cancelled.load(Ordering::Acquire));
		drop(video);
		assert!(current.cancelled.load(Ordering::Acquire));
	}

	#[test]
	fn provider_clipping_rounds_inward_at_fractional_display_scales() {
		let full = eframe::egui::Rect::from_min_size(
			eframe::egui::pos2(-2.25, 10.25),
			eframe::egui::vec2(640.0, 360.0),
		);
		let clip = eframe::egui::Rect::from_min_size(
			eframe::egui::pos2(0.0, 80.1),
			eframe::egui::vec2(600.4, 240.3),
		);
		for scale in [1.0, 1.25, 1.5, 2.0] {
			let (page, visible) = provider_geometry(full, clip, scale).unwrap();
			assert_eq!(page.width, (full.width() * scale).round() as u32);
			assert!(visible.y as f32 >= clip.top() * scale);
			assert!(
				(visible.y + i32::try_from(visible.height).unwrap()) as f32
					<= clip.bottom() * scale
			);
		}
		assert!(provider_geometry(full, clip, 0.0).is_none());
	}

	#[test]
	fn failed_proxy_retries_the_original_once_through_the_production_player() {
		use std::{
			io::{Read, Write},
			net::TcpListener,
		};
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		listener.set_nonblocking(true).unwrap();
		let base = format!("http://{}", listener.local_addr().unwrap());
		let stop = Arc::new(AtomicBool::new(false));
		let stopped = stop.clone();
		let server = std::thread::spawn(move || {
			let deadline = Instant::now() + Duration::from_secs(15);
			let mut count = 0;
			while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
				let (mut socket, _) = match listener.accept() {
					Ok(s) => s,
					Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
						std::thread::sleep(Duration::from_millis(5));
						continue;
					}
					Err(e) => panic!("{e}"),
				};
				socket.set_nonblocking(false).unwrap();
				socket
					.set_read_timeout(Some(Duration::from_secs(5)))
					.unwrap();
				let mut headers = Vec::new();
				while !headers.ends_with(b"\r\n\r\n") {
					assert!(headers.len() < 8192);
					let mut b = [0];
					socket.read_exact(&mut b).unwrap();
					headers.push(b[0]);
				}
				let wire = String::from_utf8(headers).unwrap();
				let headers = wire.to_ascii_lowercase();
				assert!(!headers.contains("authorization:") && !headers.contains("cookie:"));
				assert!(headers.contains("user-agent: serein/"));
				count += 1;
				if headers.starts_with("get /proxy ") {
					write!(
						socket,
						"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
					)
					.unwrap();
					continue;
				}
				assert!(wire.starts_with("GET /original.mov?sig=synthetic%2Fvalue "));
				let range = headers
					.lines()
					.find_map(|l| l.strip_prefix("range: bytes="))
					.unwrap();
				let (start, end) = range.split_once('-').unwrap();
				let start: usize = start.parse().unwrap();
				let end: usize = end.parse().unwrap();
				let movie = include_bytes!("../tests/fixtures/video.mov");
				write!(socket,"HTTP/1.1 206 Partial Content\r\nContent-Type: video/quicktime\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",movie.len(),end-start+1).unwrap();
				socket.write_all(&movie[start..=end]).unwrap();
			}
			count
		});
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let mut session = Session::new(0.0);
		session.synthetic_output = true;
		let session = Arc::new(session);
		let request = Request {
			output: None,
			session: session.clone(),
			url: Some(url::Url::parse(&format!("{base}/proxy")).unwrap()),
			fallback: Some(
				url::Url::parse(&format!("{base}/original.mov?sig=synthetic%2Fvalue")).unwrap(),
			),
			size: 0,
			adapter: None,
		};
		let result = play(
			&request,
			runtime.handle(),
			&eframe::egui::Context::default(),
		);
		stop.store(true, Ordering::Release);
		assert_eq!(
			server.join().unwrap(),
			3,
			"one failed proxy and two bounded original ranges"
		);
		assert_eq!(result, Ok(()));
		assert_eq!(session.update.lock().unwrap().state, VideoState::Ended);
	}

	#[test]
	fn compressed_packet_queue_exhaustion_is_explicit_and_seek_retires_it() {
		// Synthetic 32x16 black H.264 at 240fps + silent AAC, generated with FFmpeg:
		// -f lavfi -i color=c=black:s=32x16:r=240 -f lavfi -i anullsrc=r=48000:cl=mono
		// -t 3 -c:v libx264 -preset ultrafast -crf 40 -c:a aac -b:a 16k -movflags +faststart
		let bytes = test_support::reorder_video_fixture(
			include_bytes!("../../../crates/test-support/fixtures/queued-video.mov"),
			Some(true),
		);
		let mut decoder =
			AttachmentDecoder::open(Box::new(std::io::Cursor::new(bytes)), None).unwrap();
		let mut error = None;
		for _ in 0..128 {
			if let Err(failure) = decoder.poll_audio() {
				error = Some(failure);
				break;
			}
		}
		assert_eq!(
			error,
			Some("Video buffering exceeds the playback memory limit")
		);
		decoder.seek(0.0).unwrap();
		assert!(matches!(
			decoder.poll_video(),
			Ok(std::task::Poll::Ready(Some(_)))
		));
	}
	#[test]
	fn oversized_compressed_sample_reports_the_byte_budget() {
		// Expand one synthetic MOV sample without changing its coded dimensions.
		fn word(bytes: &[u8], at: usize) -> usize {
			u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
		}
		fn tables(
			bytes: &[u8],
			start: usize,
			end: usize,
			offsets: &mut Vec<usize>,
			sample: &mut Option<usize>,
		) {
			let mut at = start;
			while at < end {
				let size = word(bytes, at);
				assert!(size >= 8 && at + size <= end);
				match &bytes[at + 4..at + 8] {
					b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => {
						tables(bytes, at + 8, at + size, offsets, sample)
					}
					b"stco" => offsets.extend((0..word(bytes, at + 12)).map(|i| at + 16 + i * 4)),
					b"stsz" if sample.is_none() => {
						assert_eq!(word(bytes, at + 12), 0);
						*sample = Some(at + 20);
					}
					_ => {}
				}
				at += size;
			}
		}
		let mut bytes =
			include_bytes!("../../../crates/test-support/fixtures/queued-video.mov").to_vec();
		let (mut offsets, mut sample) = (Vec::new(), None);
		tables(&bytes, 0, bytes.len(), &mut offsets, &mut sample);
		let sample = sample.unwrap();
		let size = word(&bytes, sample);
		let insertion = word(&bytes, offsets[0]) + size;
		let padding = 8 * 1024 * 1024;
		for offset in offsets {
			let value = word(&bytes, offset);
			if value >= insertion {
				bytes[offset..offset + 4]
					.copy_from_slice(&((value + padding) as u32).to_be_bytes());
			}
		}
		bytes[sample..sample + 4].copy_from_slice(&((size + padding) as u32).to_be_bytes());
		let mut mdat = 0;
		while &bytes[mdat + 4..mdat + 8] != b"mdat" {
			mdat += word(&bytes, mdat);
		}
		let size = word(&bytes, mdat) + padding;
		bytes[mdat..mdat + 4].copy_from_slice(&(size as u32).to_be_bytes());
		bytes.splice(insertion..insertion, std::iter::repeat_n(0, padding));
		let mut decoder =
			AttachmentDecoder::open(Box::new(std::io::Cursor::new(bytes)), None).unwrap();
		let mut error = None;
		for _ in 0..32 {
			if let Err(failure) = decoder.poll_audio() {
				error = Some(failure);
				break;
			}
		}
		assert_eq!(
			error,
			Some("Video buffering exceeds the playback memory limit")
		);
	}

	#[test]
	fn ffmpeg_packet_layouts_deliver_pictures_audio_and_eof() {
		for order in [None, Some(true), Some(false)] {
			let bytes = test_support::video_fixture(order);
			let mut session = Session::new(0.0);
			session.synthetic_output = true;
			let session = Arc::new(session);
			let playing = session.clone();
			let worker = std::thread::spawn(move || {
				let decoder = AttachmentDecoder::open(Box::new(std::io::Cursor::new(bytes)), None)?;
				play_decoded(decoder, &playing, None, &eframe::egui::Context::default())
			});
			let started = Instant::now();
			let mut progressed = started;
			let mut saw_picture = false;
			let mut maximum = 0.0_f64;
			while !worker.is_finished()
				&& started.elapsed() < Duration::from_secs(40)
				&& (!saw_picture || progressed.elapsed() < Duration::from_secs(5))
			{
				let mut update = session.update.lock().unwrap();
				let picture = update.frame.take().is_some();
				if (picture && !saw_picture) || update.position > maximum {
					progressed = Instant::now();
				}
				saw_picture |= picture;
				maximum = maximum.max(update.position);
				drop(update);
				std::thread::sleep(Duration::from_millis(10));
			}
			if !worker.is_finished() {
				session.cancelled.store(true, Ordering::Release);
				let _ = worker.join();
				panic!("packet layout {order:?} stalled at {maximum}");
			}
			assert_eq!(worker.join().unwrap(), Ok(()));
			let update = session.update.lock().unwrap();
			assert!(
				saw_picture && maximum > 2.8,
				"{order:?}: no presentation progress"
			);
			assert_eq!(update.state, VideoState::Ended);
		}
	}
	#[test]
	fn ffmpeg_synthetic_output_preserves_pause_seek_and_cancellation() {
		let mut session = Session::new(0.0);
		session.synthetic_output = true;
		let session = Arc::new(session);
		let playing = session.clone();
		let worker = std::thread::spawn(move || {
			let decoder = AttachmentDecoder::open(
				Box::new(std::io::Cursor::new(test_support::video_fixture(Some(
					true,
				)))),
				None,
			)?;
			play_decoded(decoder, &playing, None, &eframe::egui::Context::default())
		});
		let wait = |predicate: &dyn Fn(&Update) -> bool| {
			let deadline = Instant::now() + Duration::from_secs(5);
			loop {
				if predicate(&session.update.lock().unwrap()) {
					break;
				}
				assert!(Instant::now() < deadline, "playback condition timed out");
				std::thread::sleep(Duration::from_millis(10));
			}
		};
		wait(&|s| s.position > 0.2 && s.frame.is_some());
		session.paused.store(true, Ordering::Release);
		wait(&|s| s.state == VideoState::Paused);
		let before = session.update.lock().unwrap().position;
		std::thread::sleep(Duration::from_millis(80));
		assert!((session.update.lock().unwrap().position - before).abs() < 0.03);
		session.seek.store(1500, Ordering::Release);
		wait(&|s| s.position >= 1.5 && s.frame.is_some());
		session.paused.store(false, Ordering::Release);
		wait(&|s| s.position > 1.7);
		session.cancelled.store(true, Ordering::Release);
		assert_eq!(worker.join().unwrap(), Ok(()));
	}
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
			play_decoded(
				decoder,
				&worker_session,
				None,
				&eframe::egui::Context::default(),
			)
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

	#[cfg(feature = "demo")]
	#[test]
	#[ignore = "opens the local audio output device at zero volume; explicit offline playback check"]
	fn inline_video_plays_pauses_seeks_and_cancels() {
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let session = Arc::new(Session::new(0.));
		let request = Request {
			output: None,
			session: session.clone(),
			url: None,
			fallback: None,
			size: 120000,
			adapter: None,
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
				play_decoded(
					decoder,
					&worker_session,
					None,
					&eframe::egui::Context::default(),
				)
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

enum DisplayFrame {
	Native(platform::video::Picture),
	Picture(Arc<platform::video::ffmpeg::Frame>),
}

pub(super) fn media_view(
	frame: Arc<platform::video::ffmpeg::Frame>,
	previous: Option<&ui::MediaView>,
) -> ui::MediaView {
	let picture = frame.picture();
	let (width, height) = if matches!(picture.rotation, 90 | 270) {
		(picture.height, picture.width)
	} else {
		(picture.width, picture.height)
	};
	let gpu = previous
		.and_then(|view| {
			view.resource
				.clone()
				.downcast::<Mutex<Option<MediaGpu>>>()
				.ok()
		})
		.unwrap_or_else(|| Arc::new(Mutex::new(None)));
	let resource = gpu.clone();
	let failure = previous
		.map(|view| view.failure.clone())
		.unwrap_or_default();
	let renderer = MediaPaint {
		frame,
		gpu,
		failure: failure.clone(),
	};
	ui::MediaView {
		size: eframe::egui::vec2(width as f32, height as f32),
		resource,
		failure,
		callback: Arc::new(move |rect| {
			eframe::egui_wgpu::Callback::new_paint_callback(rect, renderer.clone())
		}),
	}
}
#[derive(Clone)]
struct MediaPaint {
	frame: Arc<platform::video::ffmpeg::Frame>,
	gpu: Arc<Mutex<Option<MediaGpu>>>,
	failure: Arc<Mutex<Option<&'static str>>>,
}
struct MediaGpu {
	device: eframe::wgpu::Device,
	output: eframe::wgpu::TextureFormat,
	samples: u32,
	depth_format: Option<eframe::wgpu::TextureFormat>,
	pipeline: eframe::wgpu::RenderPipeline,
	group: eframe::wgpu::BindGroup,
	uniform: eframe::wgpu::Buffer,
	luma: eframe::wgpu::Texture,
	chroma: eframe::wgpu::Texture,
	uploaded: std::sync::Weak<platform::video::ffmpeg::Frame>,
	shape: (u32, u32, u32),
	_memory: platform::video::ffmpeg::MemoryLease,
}
// Arc wraps the per-view bounded resources; closing the view releases both its
// picture and GPU handles. Resources are recreated when the render device changes.
impl eframe::egui_wgpu::CallbackTrait for MediaPaint {
	fn prepare(
		&self,
		device: &eframe::wgpu::Device,
		queue: &eframe::wgpu::Queue,
		_: &eframe::egui_wgpu::ScreenDescriptor,
		_: &mut eframe::wgpu::CommandEncoder,
		resources: &mut eframe::egui_wgpu::CallbackResources,
	) -> Vec<eframe::wgpu::CommandBuffer> {
		let output = resources
			.get::<eframe::egui_wgpu::MediaOutput>()
			.copied()
			.unwrap_or_else(|| {
				eframe::egui_wgpu::MediaOutput::sdr(eframe::wgpu::TextureFormat::Bgra8Unorm)
			});
		let Ok(mut slot) = self.gpu.lock() else {
			return Vec::new();
		};
		if slot.as_ref().is_none_or(|gpu| {
			gpu.device != *device
				|| gpu.output != output.format
				|| gpu.samples != output.samples
				|| gpu.depth_format != output.depth_format
				|| gpu.shape
					!= (
						self.frame.picture().width,
						self.frame.picture().height,
						self.frame.picture().format,
					)
		}) {
			match MediaGpu::new(device, &self.frame, output) {
				Ok(gpu) => {
					*slot = Some(gpu);
					if let Ok(mut failure) = self.failure.lock() {
						*failure = None;
					}
				}
				Err(error) => {
					*slot = None;
					if let Ok(mut failure) = self.failure.lock() {
						*failure = Some(error);
					}
				}
			}
		}
		if let Some(gpu) = slot.as_mut() {
			gpu.upload(queue, &self.frame);
			gpu.color(queue, self.frame.picture(), output);
		}
		Vec::new()
	}
	fn paint(
		&self,
		_: eframe::egui::PaintCallbackInfo,
		pass: &mut eframe::wgpu::RenderPass<'static>,
		_: &eframe::egui_wgpu::CallbackResources,
	) {
		if let Ok(slot) = self.gpu.lock()
			&& let Some(gpu) = slot.as_ref()
		{
			pass.set_pipeline(&gpu.pipeline);
			pass.set_bind_group(0, &gpu.group, &[]);
			pass.draw(0..3, 0..1);
		}
	}
}
impl MediaGpu {
	fn new(
		device: &eframe::wgpu::Device,
		frame: &platform::video::ffmpeg::Frame,
		presentation: eframe::egui_wgpu::MediaOutput,
	) -> Result<Self, &'static str> {
		use eframe::wgpu as w;
		let p = frame.picture();
		let output = presentation.format;
		let limit = device.limits().max_texture_dimension_2d;
		if p.width > limit || p.height > limit {
			return Err("This GPU cannot display this video resolution");
		}
		let mut memory = platform::video::ffmpeg::MemoryLease::default();
		memory.resize(p.bytes)?;
		let texture = |width, height, format| {
			device.create_texture(&w::TextureDescriptor {
				label: Some("media plane"),
				size: w::Extent3d {
					width,
					height,
					depth_or_array_layers: 1,
				},
				mip_level_count: 1,
				sample_count: 1,
				dimension: w::TextureDimension::D2,
				format,
				usage: w::TextureUsages::TEXTURE_BINDING | w::TextureUsages::COPY_DST,
				view_formats: &[],
			})
		};
		let luma = texture(
			p.width,
			p.height,
			if p.hdr() {
				w::TextureFormat::R16Uint
			} else {
				w::TextureFormat::Rgba8Uint
			},
		);
		let chroma = texture(
			if p.hdr() { p.width.div_ceil(2) } else { 1 },
			if p.hdr() { p.height.div_ceil(2) } else { 1 },
			w::TextureFormat::Rg16Uint,
		);
		let entries: Vec<_> = (0..2)
			.map(|binding| w::BindGroupLayoutEntry {
				binding,
				visibility: w::ShaderStages::FRAGMENT,
				ty: w::BindingType::Texture {
					sample_type: w::TextureSampleType::Uint,
					view_dimension: w::TextureViewDimension::D2,
					multisampled: false,
				},
				count: None,
			})
			.chain(std::iter::once(w::BindGroupLayoutEntry {
				binding: 2,
				visibility: w::ShaderStages::FRAGMENT,
				ty: w::BindingType::Buffer {
					ty: w::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: std::num::NonZeroU64::new(48),
				},
				count: None,
			}))
			.collect();
		let layout = device.create_bind_group_layout(&w::BindGroupLayoutDescriptor {
			label: Some("media layout"),
			entries: &entries,
		});
		let uniform = device.create_buffer(&w::BufferDescriptor {
			label: Some("media color"),
			size: 48,
			usage: w::BufferUsages::UNIFORM | w::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let group = device.create_bind_group(&w::BindGroupDescriptor {
			label: Some("media planes"),
			layout: &layout,
			entries: &[
				w::BindGroupEntry {
					binding: 0,
					resource: w::BindingResource::TextureView(
						&luma.create_view(&Default::default()),
					),
				},
				w::BindGroupEntry {
					binding: 1,
					resource: w::BindingResource::TextureView(
						&chroma.create_view(&Default::default()),
					),
				},
				w::BindGroupEntry {
					binding: 2,
					resource: uniform.as_entire_binding(),
				},
			],
		});
		let shader = device.create_shader_module(w::ShaderModuleDescriptor {
			label: Some("HDR video color"),
			source: w::ShaderSource::Wgsl(include_str!("video/media.wgsl").into()),
		});
		let pipeline_layout = device.create_pipeline_layout(&w::PipelineLayoutDescriptor {
			label: Some("media pipeline layout"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let pipeline = device.create_render_pipeline(&w::RenderPipelineDescriptor {
			label: Some("HDR video"),
			layout: Some(&pipeline_layout),
			vertex: w::VertexState {
				module: &shader,
				entry_point: Some("vertex"),
				buffers: &[],
				compilation_options: Default::default(),
			},
			fragment: Some(w::FragmentState {
				module: &shader,
				entry_point: Some("fragment"),
				targets: &[Some(w::ColorTargetState {
					format: output,
					blend: None,
					write_mask: w::ColorWrites::ALL,
				})],
				compilation_options: Default::default(),
			}),
			primitive: Default::default(),
			depth_stencil: presentation
				.depth_format
				.map(|format| w::DepthStencilState {
					format,
					depth_write_enabled: Some(false),
					depth_compare: Some(w::CompareFunction::Always),
					stencil: Default::default(),
					bias: Default::default(),
				}),
			multisample: w::MultisampleState {
				count: presentation.samples,
				..Default::default()
			},
			multiview_mask: None,
			cache: None,
		});
		Ok(Self {
			device: device.clone(),
			output,
			samples: presentation.samples,
			depth_format: presentation.depth_format,
			pipeline,
			group,
			uniform,
			luma,
			chroma,
			shape: (p.width, p.height, p.format),
			uploaded: std::sync::Weak::new(),
			_memory: memory,
		})
	}
	fn color(
		&self,
		queue: &eframe::wgpu::Queue,
		p: platform::video::ffmpeg::Picture,
		output: eframe::egui_wgpu::MediaOutput,
	) {
		let words = [
			(if output.format == eframe::wgpu::TextureFormat::Rgba16Float {
				output.linear_unit_nits
			} else {
				203.0
			})
			.to_bits(),
			p.peak_nits.to_bits(),
			output.headroom.to_bits(),
			if output.format == eframe::wgpu::TextureFormat::Rgba16Float {
				1
			} else if output.format.is_srgb() {
				2
			} else {
				0
			},
			p.transfer,
			p.primaries,
			p.matrix,
			p.full_range | (p.chroma_location << 8),
			output.ui_white_scale.to_bits(),
			p.height,
			p.rotation,
			p.format,
		];
		let bytes: Vec<u8> = words.iter().flat_map(|value| value.to_le_bytes()).collect();
		queue.write_buffer(&self.uniform, 0, &bytes);
	}
	fn upload(&mut self, queue: &eframe::wgpu::Queue, frame: &Arc<platform::video::ffmpeg::Frame>) {
		if std::sync::Weak::ptr_eq(&self.uploaded, &Arc::downgrade(frame)) {
			return;
		}
		use eframe::wgpu as w;
		let (y, stride, uv) = frame.planes();
		let write = |texture: &w::Texture, bytes: &[u8], stride: usize| {
			queue.write_texture(
				w::TexelCopyTextureInfo {
					texture,
					mip_level: 0,
					origin: w::Origin3d::ZERO,
					aspect: w::TextureAspect::All,
				},
				bytes,
				w::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(stride as u32),
					rows_per_image: Some(texture.height()),
				},
				texture.size(),
			);
		};
		write(&self.luma, y, stride);
		if let Some((bytes, stride)) = uv {
			write(&self.chroma, bytes, stride);
		}
		self.uploaded = Arc::downgrade(frame);
	}
}

#[cfg(test)]
mod render_tests {
	use super::*;
	use eframe::{egui_wgpu::MediaOutput, wgpu as w};
	use platform::video::ffmpeg::{Frame, Picture, hlg_nits, pq_nits, sdr_rgb};

	fn render(
		device: &w::Device,
		queue: &w::Queue,
		frame: Arc<Frame>,
		output: MediaOutput,
		width: u32,
		height: u32,
	) -> Vec<u8> {
		let mut gpu = MediaGpu::new(device, &frame, output).unwrap();
		gpu.upload(queue, &frame);
		gpu.color(queue, frame.picture(), output);
		let bpp = if output.format == w::TextureFormat::Rgba16Float {
			8
		} else {
			4
		};
		let stride = (width * bpp).next_multiple_of(256);
		let target = device.create_texture(&w::TextureDescriptor {
			label: Some("offline media target"),
			size: w::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: w::TextureDimension::D2,
			format: output.format,
			usage: w::TextureUsages::RENDER_ATTACHMENT | w::TextureUsages::COPY_SRC,
			view_formats: &[],
		});
		let buffer = device.create_buffer(&w::BufferDescriptor {
			label: Some("offline media readback"),
			size: u64::from(stride * height),
			usage: w::BufferUsages::COPY_DST | w::BufferUsages::MAP_READ,
			mapped_at_creation: false,
		});
		let view = target.create_view(&Default::default());
		let mut encoder = device.create_command_encoder(&Default::default());
		{
			let mut pass = encoder.begin_render_pass(&w::RenderPassDescriptor {
				label: Some("offline media"),
				color_attachments: &[Some(w::RenderPassColorAttachment {
					view: &view,
					depth_slice: None,
					resolve_target: None,
					ops: w::Operations {
						load: w::LoadOp::Clear(w::Color::BLACK),
						store: w::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&gpu.pipeline);
			pass.set_bind_group(0, &gpu.group, &[]);
			pass.draw(0..3, 0..1);
		}
		encoder.copy_texture_to_buffer(
			w::TexelCopyTextureInfo {
				texture: &target,
				mip_level: 0,
				origin: w::Origin3d::ZERO,
				aspect: w::TextureAspect::All,
			},
			w::TexelCopyBufferInfo {
				buffer: &buffer,
				layout: w::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(stride),
					rows_per_image: Some(height),
				},
			},
			target.size(),
		);
		queue.submit([encoder.finish()]);
		let (send, receive) = std::sync::mpsc::sync_channel(1);
		buffer.slice(..).map_async(w::MapMode::Read, move |result| {
			let _ = send.send(result);
		});
		device.poll(w::PollType::wait_indefinitely()).unwrap();
		receive
			.recv_timeout(std::time::Duration::from_secs(10))
			.unwrap()
			.unwrap();
		let mapped = buffer.slice(..).get_mapped_range().unwrap();
		let result = mapped
			.chunks_exact(stride as usize)
			.flat_map(|row| row[..(width * bpp) as usize].iter().copied())
			.collect();
		drop(mapped);
		buffer.unmap();
		result
	}

	#[tokio::test]
	#[ignore = "offline render adapter required; no display, capture, audio or Discord access"]
	async fn media_shader_preserves_sdr_rotation_and_pq_hlg_highlights() {
		let instance = w::Instance::new(w::InstanceDescriptor::new_without_display_handle());
		let adapter = instance
			.request_adapter(&w::RequestAdapterOptions {
				force_fallback_adapter: true,
				..Default::default()
			})
			.await
			.expect("offline fallback render adapter");
		let (device, queue) = adapter
			.request_device(&w::DeviceDescriptor::default())
			.await
			.unwrap();
		check_artwork_mipmaps(&device, &queue).await;
		let sdr = MediaOutput::sdr(w::TextureFormat::Rgba8Unorm);
		let frame = Arc::new(
			Frame::new(
				Picture::sdr(2, 2),
				vec![
					0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255,
				],
			)
			.unwrap(),
		);
		let pixels = render(&device, &queue, frame, sdr, 4, 2);
		assert_eq!(
			[pixels[0], pixels[4], pixels[8], pixels[12]],
			[0, 64, 191, 255]
		);
		let frame = Arc::new(
			Frame::new(
				Picture {
					rotation: 90,
					..Picture::sdr(2, 2)
				},
				vec![
					255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
				],
			)
			.unwrap(),
		);
		assert_eq!(
			render(&device, &queue, frame, sdr, 2, 2),
			vec![
				0, 0, 255, 255, 255, 0, 0, 255, 255, 255, 255, 255, 0, 255, 0, 255
			]
		);
		for transfer in [16, 18] {
			let encoded: [f32; 4] = if transfer == 16 {
				[0.0, 0.5080784, 0.7518271, 1.0]
			} else {
				[0.0, 0.5, 0.75, 1.0]
			};
			let mut bytes = Vec::new();
			for _ in 0..2 {
				for code in encoded {
					bytes.extend_from_slice(
						&(((64.0 + 876.0 * code).round() as u16) << 6).to_le_bytes(),
					);
				}
			}
			bytes.extend((0..4).flat_map(|_| (512u16 << 6).to_le_bytes()));
			let frame = Arc::new(
				Frame::new(
					Picture {
						format: 1,
						transfer,
						primaries: 9,
						matrix: 9,
						depth: 10,
						peak_nits: 1000.0,
						bytes: bytes.len(),
						..Picture::sdr(4, 2)
					},
					bytes,
				)
				.unwrap(),
			);
			let pixels = render(&device, &queue, frame.clone(), sdr, 4, 2);
			for (i, code) in encoded.into_iter().enumerate() {
				let code = ((64.0 + 876.0 * code).round() - 64.0) / 876.0;
				let nits = if transfer == 16 {
					[pq_nits(code); 3]
				} else {
					hlg_nits([code; 3], 1000.0)
				};
				let expected = sdr_rgb(nits, 9, 1000.0, 203.0);
				for channel in 0..3 {
					assert!(
						(i16::from(pixels[i * 4 + channel]) - i16::from(expected[channel])).abs()
							<= 2
					);
				}
			}
			let hdr = MediaOutput {
				format: w::TextureFormat::Rgba16Float,
				headroom: 64.0,
				..sdr
			};
			let pixels = render(&device, &queue, frame.clone(), hdr, 4, 2);
			let high = platform::video::ffmpeg::half([pixels[3 * 8], pixels[3 * 8 + 1]]);
			assert!(
				high > 4.0,
				"HDR highlights must survive above SDR white: {high}"
			);
			let weak = Arc::downgrade(&frame);
			drop(frame);
			assert!(weak.upgrade().is_none());
		}
		// An odd crop uses the luma geometry for chroma interpolation. Rounding
		// the texture size up must not stretch its final sample across the crop.
		let bytes: Vec<_> = (0..9)
			.flat_map(|_| (512u16 << 6).to_le_bytes())
			.chain(
				[128u16, 512, 896, 512, 896, 512, 128, 512]
					.into_iter()
					.flat_map(|value| (value << 6).to_le_bytes()),
			)
			.collect();
		let hdr = MediaOutput {
			format: w::TextureFormat::Rgba16Float,
			headroom: 64.0,
			..sdr
		};
		for (chroma_location, u) in [
			(0, 320.0),
			(1, 320.0),
			(2, 416.0),
			(3, 128.0),
			(4, 320.0),
			(5, 512.0),
			(6, 512.0),
		] {
			let frame = Arc::new(
				Frame::new(
					Picture {
						format: 1,
						transfer: 16,
						primaries: 1,
						matrix: 1,
						depth: 10,
						peak_nits: 1000.0,
						bytes: bytes.len(),
						chroma_location,
						..Picture::sdr(3, 3)
					},
					bytes.clone(),
				)
				.unwrap(),
			);
			let pixels = render(&device, &queue, frame, hdr, 3, 3);
			let blue = platform::video::ffmpeg::half([pixels[68], pixels[69]]);
			let expected =
				pq_nits((512.0 - 64.0) / 876.0 + 2.0 * (1.0 - 0.0722) * (u - 512.0) / 896.0)
					/ 203.0;
			assert!(
				(blue - expected).abs() < 0.005,
				"odd crop chroma {chroma_location}: {blue} vs {expected}"
			);
		}
		// Recreating the render device recreates media resources without retaining frames.
		let (next_device, next_queue) = adapter
			.request_device(&w::DeviceDescriptor::default())
			.await
			.unwrap();
		let frame = Arc::new(Frame::new(Picture::sdr(2, 2), vec![128; 16]).unwrap());
		assert_eq!(render(&next_device, &next_queue, frame, sdr, 2, 2)[0], 128);
	}

	async fn check_artwork_mipmaps(device: &w::Device, queue: &w::Queue) {
		use eframe::egui::{Color32, ColorImage, TextureFilter, TextureId, TextureOptions};
		let mut renderer = eframe::egui_wgpu::Renderer::new(
			device,
			w::TextureFormat::Rgba8Unorm,
			Default::default(),
		);
		let mut image = ColorImage::filled([64, 64], Color32::BLACK);
		for y in 0..64 {
			for x in 0..64 {
				if (x + y) % 2 == 0 {
					image.pixels[y * 64 + x] = Color32::WHITE;
				}
			}
		}
		let id = TextureId::Managed(9001);
		renderer.update_texture(
			device,
			queue,
			id,
			&eframe::egui::epaint::ImageDelta::full(image, TextureOptions::LINEAR),
		);
		assert_eq!(
			renderer
				.texture(&id)
				.unwrap()
				.texture
				.as_ref()
				.unwrap()
				.mip_level_count(),
			1
		);
		let options = TextureOptions {
			mipmap_mode: Some(TextureFilter::Linear),
			..TextureOptions::LINEAR
		};
		renderer.update_texture(
			device,
			queue,
			id,
			&eframe::egui::epaint::ImageDelta::partial(
				[0, 0],
				ColorImage::filled([64, 32], Color32::WHITE),
				options,
			),
		);
		let texture = renderer.texture(&id).unwrap().texture.as_ref().unwrap();
		assert_eq!(texture.mip_level_count(), 7);
		let buffer = device.create_buffer(&w::BufferDescriptor {
			label: Some("offline icon mip readback"),
			size: 256,
			usage: w::BufferUsages::COPY_DST | w::BufferUsages::MAP_READ,
			mapped_at_creation: false,
		});
		let mut encoder = device.create_command_encoder(&Default::default());
		encoder.copy_texture_to_buffer(
			w::TexelCopyTextureInfo {
				texture,
				mip_level: 6,
				origin: w::Origin3d::ZERO,
				aspect: w::TextureAspect::All,
			},
			w::TexelCopyBufferInfo {
				buffer: &buffer,
				layout: w::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(256),
					rows_per_image: Some(1),
				},
			},
			w::Extent3d {
				width: 1,
				height: 1,
				depth_or_array_layers: 1,
			},
		);
		queue.submit([encoder.finish()]);
		let (send, receive) = std::sync::mpsc::sync_channel(1);
		buffer.slice(..).map_async(w::MapMode::Read, move |result| {
			let _ = send.send(result);
		});
		device.poll(w::PollType::wait_indefinitely()).unwrap();
		receive
			.recv_timeout(std::time::Duration::from_secs(10))
			.unwrap()
			.unwrap();
		let data = buffer.slice(..).get_mapped_range().unwrap();
		assert!(
			(190..=193).contains(&data[0])
				&& data[0] == data[1]
				&& data[1] == data[2]
				&& data[3] == 255,
			"the final mip must average both the updated white half and the unchanged checkerboard half: {:?}",
			&data[..4]
		);
		drop(data);
		buffer.unmap();
		renderer.free_texture(&id);
		assert!(renderer.texture(&id).is_none());
	}
}

#[cfg(test)]
mod poster_tests {
	use super::*;
	#[test]
	fn posters_use_ffmpeg_and_honor_cancellation() {
		let source = || {
			Box::new(std::io::Cursor::new(
				include_bytes!("../tests/fixtures/video.mov").as_slice(),
			)) as Box<dyn platform::video::ReadSeek>
		};
		assert!(matches!(
			AttachmentDecoder::open(source(), None).unwrap(),
			AttachmentDecoder::Ffmpeg(_)
		));
		let (width, height, rgba) = poster_frame(source(), &AtomicBool::new(false)).unwrap();
		assert!(width > 0 && height > 0 && width.max(height) <= POSTER_EDGE);
		assert_eq!(rgba.len(), width as usize * height as usize * 4);
		assert!(poster_frame(source(), &AtomicBool::new(true)).is_none());
		assert!(
			poster_frame(
				Box::new(std::io::Cursor::new(b"broken video")),
				&AtomicBool::new(false)
			)
			.is_none()
		);
	}
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
