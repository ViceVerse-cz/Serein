//! Unicode emoji rasterised on demand at the exact pixel size they are drawn.
//!
//! The application supplies a [`Backend`] that rasterises one bundled scalable artwork. A few
//! low-priority worker threads run it; the UI thread only queues work, and uploads finished
//! rasters at the start of a pass. Painted emoji go on the "now" queue and prefetched ones on
//! the "later" queue. Painted work that was not requested in this or the previous pass was
//! scrolled away and is skipped. Nothing is retried or written to disk: a raster that fails can
//! only mean a corrupt bundled asset, and its emoji shows as text for the rest of the session.
use egui::{ColorImage, Context, TextureHandle};
use std::{
	collections::{HashMap, HashSet, VecDeque},
	sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
	time::{Duration, Instant},
};

/// Jobs each queue holds; a full queue drops its oldest job.
const QUEUE: usize = 512;
/// Uploaded rasters kept, by count and RGBA bytes; least recently drawn are evicted first.
const TEXTURES: usize = 1024;
const TEXTURE_BYTES: usize = 16 * 1024 * 1024;
/// Rasters not drawn for this long are released on a later pass.
const IDLE: Duration = Duration::from_secs(60);
/// Finished rasters uploaded per pass; the rest wait for the next pass.
const UPLOAD_BYTES: usize = 4 * 1024 * 1024;
/// Finished rasters waiting for upload; workers pause beyond either bound.
const FINISHED: usize = 1024;
const FINISHED_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MIN_EDGE: u32 = 16;
pub(super) const MAX_EDGE: u32 = 256;

/// Renders one bundled compressed SVG into an `edge` × `edge` premultiplied image.
pub type Rasterize = dyn Fn(&[u8], u32) -> Option<ColorImage> + Send + Sync;

/// Rasterises bundled emoji artwork for [`super::install`]; supplied by the application.
#[derive(Clone)]
pub struct Backend {
	/// Renders one bundled compressed SVG (see [`super::decode_bundled_svg`]) into an
	/// `edge` × `edge` premultiplied image, or `None` when the artwork is invalid. Runs on a
	/// worker thread, never during rendering.
	pub rasterize: Arc<Rasterize>,
	/// Runs first on each worker thread, for example to lower its OS scheduling priority.
	pub on_worker_start: fn(),
}

/// Bundled artwork cell and square edge in physical pixels.
type Key = (usize, u32);

#[derive(Default)]
struct Entry {
	/// Last pass that asked for this raster.
	pass: u64,
	now: bool,
	later: bool,
	running: bool,
}

/// The two job queues with per-key request passes. Entries live until their raster is
/// uploaded or every queued copy of the job was dropped or cancelled.
#[derive(Default)]
struct Queues {
	now: VecDeque<Key>,
	later: VecDeque<Key>,
	entries: HashMap<Key, Entry>,
	pass: u64,
}

impl Queues {
	/// Queues `key` unless it is already queued there or running; true when a job was added.
	fn request(&mut self, key: Key, later: bool) -> bool {
		let pass = self.pass;
		let entry = self.entries.entry(key).or_default();
		entry.pass = pass;
		let queued = if later {
			entry.later || entry.now
		} else {
			entry.now
		};
		if entry.running || queued {
			return false;
		}
		if later {
			entry.later = true;
		} else {
			entry.now = true;
		}
		let queue = if later {
			&mut self.later
		} else {
			&mut self.now
		};
		let dropped = (queue.len() >= QUEUE).then(|| queue.pop_front()).flatten();
		queue.push_back(key);
		if let Some(dropped) = dropped {
			self.forget(dropped, later);
		}
		true
	}
	/// One queued copy of `key` left its queue without running.
	fn forget(&mut self, key: Key, later: bool) {
		if let Some(entry) = self.entries.get_mut(&key) {
			if later {
				entry.later = false;
			} else {
				entry.now = false;
			}
			if !entry.now && !entry.later && !entry.running {
				self.entries.remove(&key);
			}
		}
	}
	/// The next job to run, painted work first.
	fn take(&mut self) -> Option<Key> {
		loop {
			let (key, later) = match self.now.pop_front() {
				Some(key) => (key, false),
				None => (self.later.pop_front()?, true),
			};
			let Some(entry) = self.entries.get_mut(&key) else {
				continue;
			};
			if entry.running {
				if later {
					entry.later = false;
				} else {
					entry.now = false;
				}
				continue;
			}
			// Painted work not asked for in this or the previous pass was scrolled away.
			if !later && entry.pass + 1 < self.pass {
				self.forget(key, false);
				continue;
			}
			entry.now = false;
			entry.later = false;
			entry.running = true;
			return Some(key);
		}
	}
	fn idle(&self) -> bool {
		self.entries.is_empty()
	}
}

#[derive(Default)]
struct State {
	queues: Queues,
	finished: VecDeque<(Key, Option<ColorImage>)>,
	finished_bytes: usize,
	/// Held only while work is outstanding, so idle workers do not keep the context alive.
	repaint: Option<Context>,
	started: bool,
	shutdown: bool,
}

#[derive(Default)]
struct Shared {
	state: Mutex<State>,
	wake: Condvar,
}

impl Shared {
	fn lock(&self) -> MutexGuard<'_, State> {
		self.state.lock().unwrap_or_else(PoisonError::into_inner)
	}
}

struct Raster {
	texture: TextureHandle,
	used: Instant,
	pass: u64,
}

#[derive(Default)]
struct Cache {
	rasters: HashMap<Key, Raster>,
	bytes: usize,
	/// Cells whose artwork failed to rasterise; at most one per bundled cell.
	failed: HashSet<usize>,
	pass: Option<u64>,
	now: Option<Instant>,
	swept: Option<Instant>,
}

impl Cache {
	fn insert(&mut self, ctx: &Context, key: Key, image: ColorImage, now: Instant, pass: u64) {
		let bytes = image.pixels().len() * 4;
		if let Some(old) = self.rasters.remove(&key) {
			self.bytes -= old.texture.byte_size();
		}
		while !self.rasters.is_empty()
			&& (self.rasters.len() >= TEXTURES || self.bytes + bytes > TEXTURE_BYTES)
		{
			let oldest = *self
				.rasters
				.iter()
				.min_by_key(|(_, raster)| raster.used)
				.expect("nonempty emoji cache")
				.0;
			let raster = self.rasters.remove(&oldest).expect("oldest emoji raster");
			self.bytes -= raster.texture.byte_size();
		}
		let texture = ctx.load_texture("emoji", image, egui::TextureOptions::LINEAR);
		self.bytes += texture.byte_size();
		self.rasters.insert(
			key,
			Raster {
				texture,
				used: now,
				pass,
			},
		);
	}
	/// Releases rasters idle for [`IDLE`] that the previous pass did not draw either.
	fn sweep(&mut self, now: Instant, pass: u64) {
		if self
			.swept
			.is_some_and(|swept| now.saturating_duration_since(swept) < Duration::from_secs(1))
		{
			return;
		}
		self.swept = Some(now);
		let bytes = &mut self.bytes;
		self.rasters.retain(|_, raster| {
			let keep = raster.pass + 1 >= pass || now.saturating_duration_since(raster.used) < IDLE;
			if !keep {
				*bytes -= raster.texture.byte_size();
			}
			keep
		});
	}
}

pub(super) struct Service {
	shared: Arc<Shared>,
	backend: Backend,
	cache: Mutex<Cache>,
}

impl Drop for Service {
	fn drop(&mut self) {
		self.shared.lock().shutdown = true;
		self.shared.wake.notify_all();
	}
}

/// The raster's state for a painted or prefetched emoji.
pub(super) enum Lookup {
	Ready(TextureHandle),
	Pending,
	Failed,
}

fn id() -> egui::Id {
	egui::Id::unique("serein-emoji-rasters")
}

pub(super) fn install(ctx: &Context, backend: Backend) {
	let service = Arc::new(Service {
		shared: Arc::default(),
		backend,
		cache: Mutex::default(),
	});
	ctx.data_mut(|data| data.insert_temp(id(), service));
}

pub(super) fn service(ctx: &Context) -> Option<Arc<Service>> {
	ctx.data(|data| data.get_temp::<Arc<Service>>(id()))
}

/// Square raster edge for `size` points: the physical pixels it covers.
pub(super) fn edge(ctx: &Context, size: f32) -> u32 {
	let physical = (size * ctx.pixels_per_point()).ceil();
	if physical.is_finite() {
		(physical as u32).clamp(MIN_EDGE, MAX_EDGE)
	} else {
		MIN_EDGE
	}
}

impl Service {
	fn cache(&self) -> MutexGuard<'_, Cache> {
		self.cache.lock().unwrap_or_else(PoisonError::into_inner)
	}
	/// Uploads finished rasters once per pass, within [`UPLOAD_BYTES`].
	fn begin_pass(&self, ctx: &Context, cache: &mut Cache) -> (Instant, u64) {
		let pass = ctx.cumulative_pass_nr();
		if cache.pass == Some(pass)
			&& let Some(now) = cache.now
		{
			return (now, pass);
		}
		let now = Instant::now();
		cache.pass = Some(pass);
		cache.now = Some(now);
		let mut finished = Vec::new();
		{
			let mut state = self.shared.lock();
			state.queues.pass = pass;
			let mut uploaded = 0;
			while let Some((_, image)) = state.finished.front() {
				let bytes = image.as_ref().map_or(0, |image| image.pixels().len() * 4);
				if uploaded > 0 && uploaded + bytes > UPLOAD_BYTES {
					ctx.request_repaint();
					break;
				}
				uploaded += bytes;
				state.finished_bytes -= bytes;
				let (key, image) = state.finished.pop_front().expect("finished raster");
				state.queues.entries.remove(&key);
				finished.push((key, image));
			}
			if state.queues.idle() && state.finished.is_empty() {
				state.repaint = None;
			}
		}
		if !finished.is_empty() {
			self.shared.wake.notify_all();
		}
		for (key, image) in finished {
			match image {
				Some(image) => cache.insert(ctx, key, image, now, pass),
				None => {
					cache.failed.insert(key.0);
				}
			}
		}
		cache.sweep(now, pass);
		(now, pass)
	}
	/// A painted raster: cached, or requested for this pass.
	pub(super) fn paint(&self, ctx: &Context, key: Key) -> Lookup {
		let mut cache = self.cache();
		let (now, pass) = self.begin_pass(ctx, &mut cache);
		if cache.failed.contains(&key.0) {
			return Lookup::Failed;
		}
		if let Some(raster) = cache.rasters.get_mut(&key) {
			raster.used = now;
			raster.pass = pass;
			return Lookup::Ready(raster.texture.clone());
		}
		drop(cache);
		self.request(ctx, key, false);
		Lookup::Pending
	}
	/// Queues a raster at lower priority unless it is cached or failed.
	pub(super) fn prefetch(&self, ctx: &Context, key: Key) {
		let mut cache = self.cache();
		self.begin_pass(ctx, &mut cache);
		if cache.failed.contains(&key.0) || cache.rasters.contains_key(&key) {
			return;
		}
		drop(cache);
		self.request(ctx, key, true);
	}
	fn request(&self, ctx: &Context, key: Key, later: bool) {
		let mut state = self.shared.lock();
		if !state.queues.request(key, later) {
			return;
		}
		if state.repaint.is_none() {
			state.repaint = Some(ctx.clone());
		}
		if !std::mem::replace(&mut state.started, true) {
			self.start_workers();
		}
		drop(state);
		self.shared.wake.notify_one();
	}
	fn start_workers(&self) {
		let workers = std::thread::available_parallelism()
			.map_or(1, |count| count.get().saturating_sub(1))
			.clamp(1, 4);
		for index in 0..workers {
			let shared = Arc::clone(&self.shared);
			let backend = self.backend.clone();
			// A worker that cannot start leaves its share to the others; with none, the
			// slots stay empty rather than failing the UI.
			let _ = std::thread::Builder::new()
				.name(format!("emoji-raster-{index}"))
				.spawn(move || {
					(backend.on_worker_start)();
					while work(&shared, &backend, true) {}
				});
		}
	}
}

/// Takes one job, waiting for one when `wait`; false once the service shuts down or, without
/// waiting, when there is nothing to do.
fn work(shared: &Shared, backend: &Backend, wait: bool) -> bool {
	let key = {
		let mut state = shared.lock();
		loop {
			if state.shutdown {
				return false;
			}
			if state.finished.len() < FINISHED
				&& state.finished_bytes < FINISHED_BYTES
				&& let Some(key) = state.queues.take()
			{
				break key;
			}
			if !wait {
				return false;
			}
			state = shared
				.wake
				.wait(state)
				.unwrap_or_else(PoisonError::into_inner);
		}
	};
	let (cell, edge) = key;
	let image = super::bundled_svg(&format!("emoji-unicode-{cell}-{edge}"))
		.and_then(|(source, edge)| {
			std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				(backend.rasterize)(source, edge)
			}))
			.ok()
			.flatten()
		})
		.filter(|image| {
			image.resolution() == [edge as usize, edge as usize]
				&& image.pixels().len() == (edge * edge) as usize
		});
	let repaint = {
		let mut state = shared.lock();
		state.finished_bytes += image.as_ref().map_or(0, |image| image.pixels().len() * 4);
		state.finished.push_back((key, image));
		state.repaint.clone()
	};
	if let Some(ctx) = repaint {
		ctx.request_repaint();
	}
	true
}

#[cfg(test)]
pub(super) fn install_inline(ctx: &Context, backend: Backend) {
	install(ctx, backend);
	service(ctx).expect("installed").shared.lock().started = true;
}

/// Runs every queued job on the calling thread; for tests that install without workers.
#[cfg(test)]
pub(super) fn run_jobs(ctx: &Context) {
	let service = service(ctx).expect("installed");
	while work(&service.shared, &service.backend, false) {}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn full_queues_drop_their_oldest_jobs() {
		let mut queues = Queues::default();
		for cell in 0..QUEUE + 88 {
			assert!(queues.request((cell, 32), false));
			assert!(queues.request((cell, 64), true));
		}
		assert_eq!(queues.now.len(), QUEUE);
		assert_eq!(queues.later.len(), QUEUE);
		assert_eq!(queues.entries.len(), 2 * QUEUE);
		assert!(!queues.entries.contains_key(&(87, 32)));
		assert_eq!(queues.take(), Some((88, 32)));
		// A key already queued or running is not queued twice.
		assert!(!queues.request((88, 32), false));
		assert!(!queues.request((89, 32), false));
	}

	#[test]
	fn painted_jobs_scrolled_away_are_cancelled_but_prefetches_are_not() {
		let mut queues = Queues {
			pass: 1,
			..Default::default()
		};
		queues.request((1, 32), false); // Scrolled past after this pass.
		queues.request((2, 32), true); // A one-off prefetch, such as a hover prewarm.
		queues.request((3, 32), false);
		queues.pass = 2;
		queues.request((3, 32), false); // Still on screen.
		queues.request((4, 32), false);
		queues.request((2, 32), false); // Painted now, after it was prefetched.
		queues.pass = 3;
		assert_eq!(queues.take(), Some((3, 32)));
		assert_eq!(queues.take(), Some((4, 32)));
		assert_eq!(queues.take(), Some((2, 32)));
		assert_eq!(queues.take(), None);
		assert!(!queues.entries.contains_key(&(1, 32)));
		assert_eq!(queues.entries.len(), 3, "taken jobs stay until uploaded");
	}

	#[test]
	fn rasters_stay_within_item_byte_and_upload_bounds() {
		let ctx = Context::default();
		let now = Instant::now();
		let mut cache = Cache::default();
		for cell in 0..TEXTURES + 100 {
			let image = ColorImage::filled([16, 16], egui::Color32::WHITE);
			cache.insert(&ctx, (cell, 16), image, now, 0);
		}
		assert_eq!(cache.rasters.len(), TEXTURES);
		for cell in 0..100 {
			let image = ColorImage::filled([256, 256], egui::Color32::WHITE);
			cache.insert(&ctx, (cell, 256), image, now, 0);
			assert!(cache.bytes <= TEXTURE_BYTES);
		}
		assert_eq!(
			cache.bytes,
			cache
				.rasters
				.values()
				.map(|raster| raster.texture.byte_size())
				.sum::<usize>()
		);

		// A burst of finished rasters uploads within the per-pass budget.
		install_inline(
			&ctx,
			Backend {
				rasterize: Arc::new(|_, edge| {
					Some(ColorImage::filled([edge as usize; 2], egui::Color32::WHITE))
				}),
				on_worker_start: || {},
			},
		);
		let service = service(&ctx).unwrap();
		ctx.run_ui(Default::default(), |ui| {
			for cell in 0..40 {
				service.prefetch(ui.ctx(), (cell, 256));
			}
		})
		.drop_without_applying_deltas();
		run_jobs(&ctx);
		ctx.run_ui(Default::default(), |ui| {
			service.prefetch(ui.ctx(), (0, 256));
		})
		.drop_without_applying_deltas();
		assert_eq!(service.cache().bytes, UPLOAD_BYTES);
		assert_eq!(service.shared.lock().finished.len(), 40 - 16);
	}

	#[test]
	fn painted_emoji_arrive_once_and_failed_artwork_falls_back_to_text() {
		let ctx = Context::default();
		install_inline(
			&ctx,
			Backend {
				rasterize: Arc::new(|_, edge| {
					(edge != 40).then(|| ColorImage::filled([edge as usize; 2], egui::Color32::RED))
				}),
				on_worker_start: || {},
			},
		);
		let cell = super::super::lookup("🙂").unwrap();
		let pass = |key| {
			let mut lookup = None;
			ctx.run_ui(Default::default(), |ui| {
				lookup = Some(service(ui.ctx()).unwrap().paint(ui.ctx(), key));
			})
			.drop_without_applying_deltas();
			lookup.unwrap()
		};
		assert!(matches!(pass((cell, 24)), Lookup::Pending));
		assert!(
			matches!(pass((cell, 24)), Lookup::Pending),
			"requested once"
		);
		assert_eq!(service(&ctx).unwrap().shared.lock().queues.now.len(), 1);
		run_jobs(&ctx);
		assert!(
			matches!(pass((cell, 24)), Lookup::Ready(texture) if texture.resolution() == [24, 24])
		);
		// Artwork that cannot be rasterised shows as text, without retrying.
		assert!(matches!(pass((cell, 40)), Lookup::Pending));
		run_jobs(&ctx);
		assert!(matches!(pass((cell, 40)), Lookup::Failed));
		assert!(service(&ctx).unwrap().shared.lock().queues.idle());
	}
}
