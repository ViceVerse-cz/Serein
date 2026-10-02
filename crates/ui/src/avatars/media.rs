use super::{Animation, Avatars, GifFrames, REQUESTS, RETRY, paint_texture};
use egui::{Color32, ColorImage, TextureHandle};
use std::{
	collections::HashMap,
	fmt,
	sync::Arc,
	time::{Duration, Instant},
};

/// Powers of two up to 256, then about sqrt(2) apart, so a texture is minified by at most ~1.41x.
/// egui-wgpu textures have no mip chain, and larger ratios alias.
const LADDER: [u32; 12] = [
	32, 64, 128, 256, 368, 512, 720, 1024, 1440, 2048, 2880, 4096,
];
pub(crate) const MEDIA_MAX_WIDTH: f32 = 550.0;
pub(crate) const MEDIA_MAX_HEIGHT: f32 = 350.0;
const MAX_SOURCE: usize = 2048;
pub const MAX_FRAMES: usize = 240;
const SIZING: [&str; 7] = [
	"width", "height", "size", "format", "quality", "animated", "fit",
];
const INLINE_STILL_BYTES: usize = 96 * 1024 * 1024;
const INLINE_STILLS: usize = 384;
const INLINE_FRAME_BYTES: usize = 128 * 1024 * 1024;
const INLINE_ANIMATIONS: usize = 96;
const VIEWER_BYTES: usize = 128 * 1024 * 1024;
const SLOTS: usize = 2048;
const CANONICAL: usize = 4096;
const HELD: usize = 4;
const ATTEMPTS: usize = 8;
const FADE: Duration = Duration::from_millis(150);
const DIM: f32 = 48.0;

/// Scale `width`×`height` so the longer side is at most `edge`, never upscaling.
pub fn fit_edge(width: u32, height: u32, edge: u32) -> (u32, u32) {
	let longest = u64::from(width.max(height)).max(u64::from(edge));
	(
		(u64::from(width) * u64::from(edge) / longest).max(1) as u32,
		(u64::from(height) * u64::from(edge) / longest).max(1) as u32,
	)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Edge(u32);

impl Edge {
	pub fn for_target(needed_px: f32, native_longest: Option<u32>) -> Self {
		let rung = LADDER
			.into_iter()
			.find(|rung| *rung as f32 >= needed_px)
			.unwrap_or(LADDER[LADDER.len() - 1]);
		Self(native_longest.map_or(rung, |native| rung.min(native.max(1))))
	}
	pub fn get(self) -> u32 {
		self.0
	}
}

/// What the proxy is asked for. `Exact` keeps metadata aspect; `Longest` when it has no size.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Size {
	Exact { width: u32, height: u32 },
	Longest(Edge),
}

impl Size {
	pub fn new(edge: Edge, native: Option<[u32; 2]>) -> Self {
		match native {
			Some([width, height]) => {
				let (width, height) = fit_edge(width, height, edge.0);
				Self::Exact { width, height }
			}
			None => Self::Longest(edge),
		}
	}
	pub fn longest(self) -> u32 {
		match self {
			Self::Exact { width, height } => width.max(height),
			Self::Longest(edge) => edge.0,
		}
	}
	fn parse(text: &str) -> Option<Self> {
		let side = |text: &str| {
			text.parse::<u32>()
				.ok()
				.filter(|side| (1..=LADDER[LADDER.len() - 1]).contains(side))
		};
		match text.strip_prefix('e') {
			Some(edge) => Some(Self::Longest(Edge(side(edge)?))),
			None => {
				let (width, height) = text.split_once('x')?;
				Some(Self::Exact {
					width: side(width)?,
					height: side(height)?,
				})
			}
		}
	}
}

impl fmt::Display for Size {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Exact { width, height } => write!(f, "{width}x{height}"),
			Self::Longest(edge) => write!(f, "e{}", edge.0),
		}
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Motion {
	Still,
	Animated,
}

/// Which working set pays for the pixels and which worker queue fetches them first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Lane {
	Inline,
	Viewer,
}

impl Lane {
	/// Decoded frame budget for one animation in this lane.
	pub const fn frame_bytes(self) -> usize {
		match self {
			Self::Inline => 40 * 1024 * 1024,
			Self::Viewer => 96 * 1024 * 1024,
		}
	}
}

/// Canonical identity of one picture: signatures kept in order, sizing and format removed.
/// Host and path trust stays in the desktop worker.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Source(Arc<str>);

impl Source {
	fn canonical(raw: &str) -> Option<Self> {
		if raw.len() > MAX_SOURCE {
			return None;
		}
		if model::valid_gif_url(raw) || model::valid_gif_preview(raw) {
			return Some(Self(raw.into()));
		}
		let Ok(mut url) = url::Url::parse(raw) else {
			return Some(Self(raw.into()));
		};
		if url.query().is_some() {
			let kept: Vec<(String, String)> = url
				.query_pairs()
				.filter(|(key, _)| !SIZING.contains(&key.as_ref()))
				.map(|(key, value)| (key.into_owned(), value.into_owned()))
				.collect();
			url.set_query(None);
			if !kept.is_empty() {
				url.query_pairs_mut().extend_pairs(kept);
			}
		}
		let text = String::from(url);
		(text.len() <= MAX_SOURCE).then(|| Self(text.into()))
	}
	pub fn as_str(&self) -> &str {
		&self.0
	}
}

/// One requested picture. The only producer and parser of `media:` keys.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Rendition {
	pub source: Source,
	pub motion: Motion,
	pub size: Size,
	pub lane: Lane,
}

impl Rendition {
	pub fn key(&self) -> String {
		let lane = match self.lane {
			Lane::Inline => 'i',
			Lane::Viewer => 'v',
		};
		let motion = match self.motion {
			Motion::Still => 's',
			Motion::Animated => 'a',
		};
		format!(
			"media:{lane}{motion}:{}:{}",
			self.size,
			self.source.as_str()
		)
	}
	/// Re-canonicalizes the source, so a key built elsewhere cannot smuggle sizing params.
	pub fn parse(key: &str) -> Option<Self> {
		let (class, rest) = key.strip_prefix("media:")?.split_once(':')?;
		let (size, source) = rest.split_once(':')?;
		let (lane, motion) = match class.as_bytes() {
			[lane, motion] => (*lane, *motion),
			_ => return None,
		};
		Some(Self {
			lane: match lane {
				b'i' => Lane::Inline,
				b'v' => Lane::Viewer,
				_ => return None,
			},
			motion: match motion {
				b's' => Motion::Still,
				b'a' => Motion::Animated,
				_ => return None,
			},
			size: Size::parse(size)?,
			source: Source::canonical(source)?,
		})
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Surface {
	Inline,
	Banner,
	Viewer,
}

impl Surface {
	fn lane(self) -> Lane {
		match self {
			Self::Viewer => Lane::Viewer,
			Self::Inline | Self::Banner => Lane::Inline,
		}
	}
	fn point_limit(self) -> f32 {
		match self {
			Self::Viewer => 4096.0,
			Self::Inline | Self::Banner => 512.0,
		}
	}
	fn allows_upscale(self) -> bool {
		matches!(self, Self::Viewer)
	}
	fn covers(self) -> bool {
		matches!(self, Self::Banner)
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Quality {
	Placeholder { failed: bool },
	Upgrading,
	Degraded,
	Full,
}

enum StandIn {
	ThumbHash,
	Label,
}

pub(crate) struct Shown {
	pub response: egui::Response,
	pub quality: Quality,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Learned {
	#[default]
	Unknown,
	Still,
	/// A gifv clip the platform decoder cannot play; the embed falls back to its GIF or poster.
	Unplayable,
}

#[derive(Clone, Copy)]
enum Attempt {
	Pending,
	Failed(Instant),
}

enum Pixels {
	Still(TextureHandle),
	Playing {
		still: TextureHandle,
		animation: Animation,
	},
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pool {
	Stills,
	Frames,
	Viewer,
}

impl Pool {
	fn limits(self) -> (usize, usize) {
		match self {
			Self::Stills => (INLINE_STILL_BYTES, INLINE_STILLS),
			Self::Frames => (INLINE_FRAME_BYTES, INLINE_ANIMATIONS),
			Self::Viewer => (VIEWER_BYTES, usize::MAX),
		}
	}
}

struct Held {
	motion: Motion,
	size: Size,
	lane: Lane,
	pixels: Pixels,
	arrived: Instant,
	used: u64,
	painted_pass: u64,
}

impl Held {
	fn playing(&self) -> bool {
		matches!(self.pixels, Pixels::Playing { .. })
	}
	fn bytes(&self) -> usize {
		match &self.pixels {
			Pixels::Still(still) => still.byte_size(),
			Pixels::Playing { still, animation } => still.byte_size() + animation.bytes,
		}
	}
	fn pool(&self) -> Pool {
		match (self.lane, &self.pixels) {
			(Lane::Viewer, _) => Pool::Viewer,
			(Lane::Inline, Pixels::Still(_)) => Pool::Stills,
			(Lane::Inline, Pixels::Playing { .. }) => Pool::Frames,
		}
	}
}

#[derive(Default)]
struct Slot {
	held: Vec<Held>,
	attempts: Vec<(Motion, Size, Attempt)>,
	learned: Learned,
	used: u64,
}

impl Slot {
	fn index(&self, motion: Motion, size: Size) -> Option<usize> {
		self.held
			.iter()
			.position(|held| held.motion == motion && held.size == size)
	}
	fn attempt(&self, motion: Motion, size: Size) -> Option<Attempt> {
		self.attempts
			.iter()
			.find(|(m, s, _)| *m == motion && *s == size)
			.map(|(_, _, attempt)| *attempt)
	}
	fn pending(&self, motion: Motion, size: Size) -> bool {
		matches!(self.attempt(motion, size), Some(Attempt::Pending))
	}
	fn record(&mut self, motion: Motion, size: Size, attempt: Attempt) {
		if let Some(entry) = self
			.attempts
			.iter_mut()
			.find(|(m, s, _)| *m == motion && *s == size)
		{
			entry.2 = attempt;
			return;
		}
		if self.attempts.len() >= ATTEMPTS {
			// A dropped pending entry would discard its result on arrival; drop settled ones first.
			let oldest = self
				.attempts
				.iter()
				.position(|(.., attempt)| !matches!(attempt, Attempt::Pending))
				.unwrap_or(0);
			self.attempts.remove(oldest);
		}
		self.attempts.push((motion, size, attempt));
	}
	fn forget(&mut self, motion: Motion, size: Size) {
		self.attempts.retain(|(m, s, _)| *m != motion || *s != size);
	}
	fn carried_start(&self, size: Size, now: Instant) -> Instant {
		self.held
			.iter()
			.filter(|held| held.size != size)
			.filter_map(|held| match &held.pixels {
				Pixels::Playing { animation, .. } => Some((held.size.longest(), animation.started)),
				Pixels::Still(_) => None,
			})
			.max_by_key(|(longest, _)| *longest)
			.map_or(now, |(_, started)| started)
	}
}

struct Choice {
	base: Option<usize>,
	fade: Option<(usize, f32)>,
	quality: Quality,
	request: bool,
}

fn choose(slot: &Slot, motion: Motion, size: Size, now: Instant) -> Choice {
	let wanted = size.longest();
	let candidates = || {
		slot.held
			.iter()
			.enumerate()
			.filter(move |(_, held)| held.motion == motion)
	};
	let moving = motion == Motion::Animated && candidates().any(|(_, held)| held.playing());
	let ranked = || candidates().filter(move |(_, held)| !moving || held.playing());
	let longest = |(_, held): &(usize, &Held)| held.size.longest();
	let sharp = ranked()
		.filter(|(_, held)| held.size.longest() >= wanted)
		.min_by_key(longest);
	if let Some((index, held)) = sharp.filter(|(_, held)| motion == Motion::Still || held.playing())
	{
		let below = ranked()
			.filter(|(_, other)| other.size.longest() < held.size.longest())
			.max_by_key(longest)
			.map(|(index, _)| index);
		let progress =
			now.saturating_duration_since(held.arrived).as_secs_f32() / FADE.as_secs_f32();
		return match below.filter(|_| progress < 1.0) {
			Some(below) => Choice {
				base: Some(below),
				fade: Some((index, progress)),
				quality: Quality::Full,
				request: false,
			},
			None => Choice {
				base: Some(index),
				fade: None,
				quality: Quality::Full,
				request: false,
			},
		};
	}
	let shown = sharp
		.or_else(|| ranked().max_by_key(longest))
		.map(|(index, _)| index);
	let (failed, request) = match slot.attempt(motion, size) {
		Some(Attempt::Pending) => (false, false),
		Some(Attempt::Failed(at)) if now.saturating_duration_since(at) < RETRY => (true, false),
		_ => (false, true),
	};
	Choice {
		base: shown,
		fade: None,
		quality: match (shown, failed) {
			(Some(_), false) => Quality::Upgrading,
			(Some(_), true) => Quality::Degraded,
			(None, failed) => Quality::Placeholder { failed },
		},
		request,
	}
}

#[derive(Default)]
pub(crate) struct MediaLibrary {
	slots: HashMap<Source, Slot>,
	canonical: HashMap<Box<str>, Option<Source>>,
	clock: u64,
	swept_pass: u64,
	viewer_painted: bool,
}

impl MediaLibrary {
	fn source(&mut self, raw: &str) -> Option<Source> {
		if let Some(source) = self.canonical.get(raw) {
			return source.clone();
		}
		if self.canonical.len() >= CANONICAL {
			self.canonical.clear();
		}
		let source = Source::canonical(raw);
		self.canonical.insert(raw.into(), source.clone());
		source
	}

	fn slot(&mut self, source: &Source) -> &mut Slot {
		if !self.slots.contains_key(source)
			&& self.slots.len() >= SLOTS
			&& let Some(oldest) = self
				.slots
				.iter()
				.min_by_key(|(_, slot)| slot.used)
				.map(|(source, _)| source.clone())
		{
			self.slots.remove(&oldest);
		}
		self.clock += 1;
		let slot = self.slots.entry(source.clone()).or_default();
		slot.used = self.clock;
		slot
	}

	fn want(
		&mut self,
		source: &Source,
		may_animate: bool,
		size: Size,
		lane: Lane,
		now: Instant,
	) -> (Rendition, Choice) {
		let slot = self.slot(source);
		let motion = if may_animate && slot.learned == Learned::Unknown {
			Motion::Animated
		} else {
			Motion::Still
		};
		let choice = choose(slot, motion, size, now);
		(
			Rendition {
				source: source.clone(),
				motion,
				size,
				lane,
			},
			choice,
		)
	}

	fn texture(
		&mut self,
		source: &Source,
		index: usize,
		ctx: &egui::Context,
		playing: bool,
		lane: Lane,
		pass: u64,
	) -> Option<TextureHandle> {
		self.clock += 1;
		let held = self.slots.get_mut(source)?.held.get_mut(index)?;
		held.used = self.clock;
		held.painted_pass = pass;
		if lane == Lane::Inline {
			held.lane = Lane::Inline;
		}
		Some(match &mut held.pixels {
			Pixels::Still(still) => still.clone(),
			Pixels::Playing { still, animation } => playing
				.then(|| animation.advance(ctx))
				.flatten()
				.unwrap_or_else(|| still.clone()),
		})
	}

	pub(super) fn set_animation(&mut self, enabled: bool) {
		for slot in self.slots.values_mut() {
			slot.learned = Learned::Unknown;
			if !enabled {
				slot.held.retain(|held| held.motion == Motion::Still);
				slot.attempts
					.retain(|(motion, ..)| *motion == Motion::Still);
			}
		}
	}

	pub(super) fn accept_still(
		&mut self,
		ctx: &egui::Context,
		rendition: Rendition,
		image: Option<ColorImage>,
	) -> bool {
		let Rendition {
			source,
			motion,
			size,
			lane,
		} = rendition;
		let Some(slot) = self.slots.get_mut(&source) else {
			return false;
		};
		if !slot.pending(motion, size) {
			return false;
		}
		let limit = size.longest() as usize;
		let Some(image) = image.filter(|image| {
			image.size[0] > 0
				&& image.size[1] > 0
				&& image.size[0] <= limit
				&& image.size[1] <= limit
				&& image.pixels.len() == image.size[0] * image.size[1]
		}) else {
			if motion == Motion::Animated && is_motion_video(source.as_str()) {
				// Retrying cannot help a clip this decoder rejects, and it is not cached on disk.
				slot.learned = Learned::Unplayable;
				slot.forget(motion, size);
			} else {
				slot.record(motion, size, Attempt::Failed(Instant::now()));
			}
			return true;
		};
		if motion == Motion::Still {
			slot.forget(motion, size);
		}
		let texture = ctx.load_texture("service-image", image, egui::TextureOptions::LINEAR);
		self.clock += 1;
		let used = self.clock;
		let pool = match slot.index(motion, size) {
			Some(index) => {
				let held = &mut slot.held[index];
				match &mut held.pixels {
					Pixels::Still(still) | Pixels::Playing { still, .. } => *still = texture,
				}
				held.used = used;
				held.pool()
			}
			None => {
				if slot.held.len() >= HELD
					&& let Some(oldest) = slot
						.held
						.iter()
						.enumerate()
						.min_by_key(|(_, held)| held.used)
						.map(|(index, _)| index)
				{
					slot.held.remove(oldest);
				}
				let held = Held {
					motion,
					size,
					lane,
					pixels: Pixels::Still(texture),
					arrived: Instant::now(),
					used,
					painted_pass: self.swept_pass,
				};
				let pool = held.pool();
				slot.held.push(held);
				pool
			}
		};
		self.evict(pool, (&source, motion, size));
		true
	}

	pub(super) fn accept_frames(&mut self, rendition: Rendition, frames: GifFrames) {
		let Rendition {
			source,
			motion,
			size,
			lane,
		} = rendition;
		let Some(slot) = self.slots.get_mut(&source) else {
			return;
		};
		if motion != Motion::Animated || !slot.pending(motion, size) {
			return;
		}
		slot.forget(motion, size);
		let Some(index) = slot.index(motion, size) else {
			return;
		};
		let video = is_motion_video(source.as_str());
		if frames.len() < 2 && video {
			slot.learned = Learned::Unplayable;
			slot.held.remove(index);
			return;
		}
		if frames.len() < 2 {
			slot.learned = Learned::Still;
			if slot.index(Motion::Still, size).is_some() {
				slot.held.remove(index);
			} else {
				slot.held[index].motion = Motion::Still;
			}
			return;
		}
		let now = Instant::now();
		let total: Duration = frames.iter().map(|(delay, _)| *delay).sum();
		let frame_bytes = || frames.iter().map(|(_, image)| image.pixels.len() * 4);
		let bytes = frame_bytes().sum::<usize>() + frame_bytes().max().unwrap_or(0);
		let limit = size.longest() as usize;
		if total.is_zero()
			|| frames.len() > MAX_FRAMES
			|| bytes > lane.frame_bytes()
			|| frames.iter().any(|(delay, image)| {
				*delay < Duration::from_millis(20) || image.size[0] > limit || image.size[1] > limit
			}) {
			// The decoder applies the same budget, so a retry would be rejected again.
			slot.held.remove(index);
			slot.learned = if video {
				Learned::Unplayable
			} else {
				Learned::Still
			};
			return;
		}
		let started = slot.carried_start(size, now);
		let held = &mut slot.held[index];
		let still = match &held.pixels {
			Pixels::Still(still) | Pixels::Playing { still, .. } => still.clone(),
		};
		held.pixels = Pixels::Playing {
			still,
			animation: Animation {
				frames,
				texture: None,
				total,
				started,
				next_upload: now,
				frame: usize::MAX,
				bytes,
			},
		};
		held.arrived = now;
		let pool = held.pool();
		self.evict(pool, (&source, motion, size));
	}

	fn evict(&mut self, pool: Pool, keep: (&Source, Motion, Size)) {
		let (byte_limit, count_limit) = pool.limits();
		loop {
			let (mut bytes, mut count) = (0, 0);
			let mut oldest: Option<(u64, &Source, usize)> = None;
			for (source, slot) in &self.slots {
				for (index, held) in slot.held.iter().enumerate() {
					if held.pool() != pool {
						continue;
					}
					bytes += held.bytes();
					count += 1;
					let kept = source == keep.0 && held.motion == keep.1 && held.size == keep.2;
					if !kept && oldest.is_none_or(|(used, ..)| held.used < used) {
						oldest = Some((held.used, source, index));
					}
				}
			}
			if bytes <= byte_limit && count <= count_limit {
				return;
			}
			let Some((_, source, index)) = oldest else {
				return;
			};
			let source = source.clone();
			if let Some(slot) = self.slots.get_mut(&source) {
				slot.held.remove(index);
			}
		}
	}

	/// Once per frame after painting: a closed viewer releases its full-size pixels at once.
	pub(super) fn end_frame(&mut self) {
		if !std::mem::take(&mut self.viewer_painted) {
			for slot in self.slots.values_mut() {
				slot.held.retain(|held| held.lane == Lane::Inline);
			}
		}
	}

	/// False once this clip failed to decode, so a gifv embed can use its GIF or poster instead.
	pub(super) fn playable(&mut self, raw: &str) -> bool {
		self.source(raw).is_none_or(|source| {
			self.slots
				.get(&source)
				.is_none_or(|slot| slot.learned != Learned::Unplayable)
		})
	}

	fn sweep(&mut self, pass: u64) {
		if self.swept_pass == pass {
			return;
		}
		self.swept_pass = pass;
		for slot in self.slots.values_mut() {
			slot.held
				.retain(|held| held.lane == Lane::Inline || held.painted_pass + 1 >= pass);
		}
	}

	#[cfg(any(test, feature = "demo"))]
	fn accept_demo(&mut self, ctx: &egui::Context, rendition: &Rendition) {
		let (width, height) = match rendition.size {
			Size::Exact { width, height } => (width as usize, height as usize),
			Size::Longest(edge) => {
				let (width, height) = fit_edge(320, 180, edge.0);
				(width as usize, height as usize)
			}
		};
		let mut image = ColorImage::filled([width, height], Color32::from_rgb(40, 50, 70));
		for y in 0..height {
			for x in 0..width {
				let (u, v) = (x * 320 / width, y * 180 / height);
				image.pixels[y * width + x] = if u < 8 || v < 8 || u >= 312 || v >= 172 {
					Color32::WHITE
				} else if u % 40 < 4 || v % 40 < 4 {
					Color32::from_rgb(80, 180, 160)
				} else {
					Color32::from_rgb(40, 50, 70)
				};
			}
		}
		self.slot(&rendition.source)
			.record(rendition.motion, rendition.size, Attempt::Pending);
		self.accept_still(ctx, rendition.clone(), Some(image));
	}
}

#[cfg(test)]
impl MediaLibrary {
	fn find(&self, rendition: &Rendition) -> Option<&Held> {
		let slot = self.slots.get(&rendition.source)?;
		slot.held.get(slot.index(rendition.motion, rendition.size)?)
	}
	pub(super) fn texture_id(&self, rendition: &Rendition) -> Option<egui::TextureId> {
		match &self.find(rendition)?.pixels {
			Pixels::Still(still) | Pixels::Playing { still, .. } => Some(still.id()),
		}
	}
	pub(super) fn animation(&mut self, rendition: &Rendition) -> Option<&mut Animation> {
		let slot = self.slots.get_mut(&rendition.source)?;
		let index = slot.index(rendition.motion, rendition.size)?;
		match &mut slot.held[index].pixels {
			Pixels::Playing { animation, .. } => Some(animation),
			Pixels::Still(_) => None,
		}
	}
	pub(super) fn bytes(&self) -> usize {
		self.slots
			.values()
			.flat_map(|slot| &slot.held)
			.map(Held::bytes)
			.sum()
	}
}

pub fn is_motion_video(url: &str) -> bool {
	let path = url.split(['?', '#']).next().unwrap_or(url);
	path.rsplit_once('.').is_some_and(|(_, extension)| {
		extension.eq_ignore_ascii_case("mp4")
			|| extension.eq_ignore_ascii_case("mov")
			|| extension.eq_ignore_ascii_case("m4v")
	})
}

fn pick(media: &model::EmbedMedia, animate: bool) -> Option<(&str, bool)> {
	fn path(url: &str) -> &str {
		url.split('?').next().unwrap_or(url)
	}
	if animate
		&& let Some(video) = media
			.url
			.as_deref()
			.filter(|url| is_motion_video(url))
			.or(media
				.proxy_url
				.as_deref()
				.filter(|url| is_motion_video(url)))
	{
		return Some((video, true));
	}
	let original_gif = media.url.as_deref().filter(|url| {
		animate
			&& (model::valid_gif_url(url)
				|| url.starts_with("https://cdn.discordapp.com/")
				|| url.starts_with("https://media.discordapp.net/"))
			&& path(url).ends_with(".gif")
	});
	let raw = original_gif
		.or(media.proxy_url.as_deref())
		.or(media.url.as_deref())?;
	let may_animate = animate
		&& path(raw).rsplit_once('.').is_some_and(|(_, extension)| {
			extension.eq_ignore_ascii_case("gif") || extension.eq_ignore_ascii_case("webp")
		});
	Some((raw, may_animate))
}

impl Avatars {
	/// Service dimensions when present, otherwise the largest decoded rendition.
	pub(crate) fn media_dimensions(&mut self, media: &model::EmbedMedia) -> Option<[u32; 2]> {
		if media.width > 0 && media.height > 0 {
			return Some([media.width.min(16384), media.height.min(16384)]);
		}
		let (raw, _) = pick(media, self.animate_gifs)?;
		let source = self.media.source(raw)?;
		self.media
			.slots
			.get(&source)?
			.held
			.iter()
			.map(|held| {
				let texture = match &held.pixels {
					Pixels::Still(still) | Pixels::Playing { still, .. } => still,
				};
				let [width, height] = texture.size();
				[width as u32, height as u32]
			})
			.max_by_key(|[width, height]| u64::from(*width) * u64::from(*height))
	}

	pub(crate) fn show_media(
		&mut self,
		ui: &mut egui::Ui,
		media: &model::EmbedMedia,
		max_size: egui::Vec2,
		demo: bool,
		surface: Surface,
	) -> Shown {
		let viewer = surface == Surface::Viewer;
		let bound = surface.point_limit();
		let max_size = egui::vec2(
			max_size.x.min(ui.available_width()).clamp(1.0, bound),
			max_size.y.clamp(1.0, bound),
		);
		let native = (media.width > 0 && media.height > 0)
			.then(|| [media.width.min(16384), media.height.min(16384)]);
		let original = self
			.media_dimensions(media)
			.map_or(egui::vec2(320.0, 180.0), |[width, height]| {
				egui::vec2(width as f32, height as f32)
			});
		let scale = (max_size.x / original.x).min(max_size.y / original.y).min(
			if surface.allows_upscale() || native.is_none() {
				f32::INFINITY
			} else {
				1.0
			},
		);
		let size = if surface.covers() {
			max_size
		} else {
			(original * scale).max(egui::vec2(1.0, 1.0))
		};
		let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
		let quality = if ui.is_rect_visible(rect) {
			self.paint_media(ui, media, rect, native, demo, surface)
		} else {
			Quality::Placeholder { failed: false }
		};
		let label = if viewer && quality == Quality::Upgrading {
			"Embedded image, loading full quality"
		} else {
			"Embedded image"
		};
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), label));
		Shown { response, quality }
	}

	fn paint_media(
		&mut self,
		ui: &mut egui::Ui,
		media: &model::EmbedMedia,
		rect: egui::Rect,
		native: Option<[u32; 2]>,
		demo: bool,
		surface: Surface,
	) -> Quality {
		let cover = surface.covers();
		let viewer = surface == Surface::Viewer;
		let radius = if cover { 8 } else { 5 };
		let Some((source, may_animate)) = pick(media, self.animate_gifs)
			.and_then(|(raw, may_animate)| Some((self.media.source(raw)?, may_animate)))
		else {
			self.paint_stand_in(ui, media, rect, radius, cover, demo, false);
			return Quality::Placeholder { failed: true };
		};
		let now = Instant::now();
		let pass = ui.ctx().cumulative_pass_nr();
		self.media.sweep(pass);
		let ppp = ui.ctx().pixels_per_point();
		let needed = match native {
			Some([width, height]) => {
				let (width, height) = (width as f32, height as f32);
				let (x, y) = (rect.width() / width, rect.height() / height);
				width.max(height) * if cover { x.max(y) } else { x.min(y) } * ppp
			}
			None => rect.size().max_elem() * ppp,
		};
		let heic = source
			.as_str()
			.split('?')
			.next()
			.and_then(|path| path.rsplit_once('.'))
			.is_some_and(|(_, ext)| {
				ext.eq_ignore_ascii_case("heic") || ext.eq_ignore_ascii_case("heif")
			});
		let size = if viewer && (heic || native.is_none()) {
			// Use the maximum existing rendition instead of treating a thumbnail as native size.
			Size::Longest(Edge::for_target(4096.0, None))
		} else {
			Size::new(
				Edge::for_target(needed, native.map(|[width, height]| width.max(height))),
				native,
			)
		};
		let lane = surface.lane();
		self.media.viewer_painted |= viewer;
		let (want, choice) = self
			.media
			.want(&source, may_animate && !demo, size, lane, now);
		#[cfg(any(test, feature = "demo"))]
		let choice = if demo && choice.quality != Quality::Full {
			self.media.accept_demo(ui.ctx(), &want);
			self.media.want(&source, false, size, lane, now).1
		} else {
			choice
		};
		if choice.request && !demo && self.requests.len() < REQUESTS {
			self.requests.push(want.key());
			self.media
				.slot(&source)
				.record(want.motion, want.size, Attempt::Pending);
		}
		let playing = self.animate_gifs && ui.ctx().input(|input| input.focused);
		let mut texture = |index| {
			self.media
				.texture(&source, index, ui.ctx(), playing, lane, pass)
		};
		let base = choice.base.and_then(&mut texture);
		let fade = choice
			.fade
			.and_then(|(index, progress)| Some((texture(index)?, progress)));
		let loading = matches!(
			choice.quality,
			Quality::Upgrading | Quality::Placeholder { failed: false }
		);
		let dim = match fade {
			Some((_, progress)) => 1.0 - progress,
			None if loading => 1.0,
			None => 0.0,
		};
		let painted = match &base {
			Some(texture) => Some(paint_texture(
				ui,
				texture,
				rect,
				radius,
				cover,
				Color32::WHITE,
			)),
			None => {
				let failed = choice.quality == Quality::Placeholder { failed: true };
				match self.paint_stand_in(ui, media, rect, radius, cover, demo, failed) {
					StandIn::ThumbHash => Some(rect),
					StandIn::Label => None,
				}
			}
		};
		if viewer
			&& dim > 0.0
			&& let Some(painted) = painted
		{
			ui.painter().rect_filled(
				painted,
				radius,
				Color32::from_black_alpha((DIM * dim) as u8),
			);
		}
		if let Some((texture, progress)) = &fade {
			paint_texture(
				ui,
				texture,
				rect,
				radius,
				cover,
				Color32::WHITE.gamma_multiply(*progress),
			);
			ui.ctx().request_repaint();
		}
		choice.quality
	}

	#[allow(clippy::too_many_arguments)]
	fn paint_stand_in(
		&mut self,
		ui: &mut egui::Ui,
		media: &model::EmbedMedia,
		rect: egui::Rect,
		radius: u8,
		cover: bool,
		demo: bool,
		failed: bool,
	) -> StandIn {
		let colors = crate::design::palette(ui);
		if self.paint_placeholder(ui, &media.placeholder, rect, radius, cover) {
			return StandIn::ThumbHash;
		}
		ui.painter().rect_filled(rect, 5, colors.canvas);
		#[cfg(any(test, feature = "demo"))]
		if demo {
			let ridge = vec![
				rect.left_bottom(),
				rect.left_center(),
				rect.center_top() + egui::vec2(0.0, rect.height() * 0.35),
				rect.right_bottom(),
			];
			ui.painter().add(egui::Shape::convex_polygon(
				ridge,
				colors.accent.gamma_multiply(0.4),
				egui::Stroke::NONE,
			));
			ui.painter().circle_filled(
				rect.min + rect.size() * egui::vec2(0.8, 0.25),
				rect.height() * 0.08,
				colors.accent,
			);
		}
		if rect.width() >= 100.0 && rect.height() >= 32.0 {
			ui.painter().text(
				rect.center(),
				egui::Align2::CENTER_CENTER,
				if demo {
					"Synthetic preview"
				} else if failed {
					"Preview unavailable"
				} else {
					"Image preview"
				},
				egui::FontId::proportional(11.0),
				colors.muted,
			);
		}
		StandIn::Label
	}
}

#[cfg(feature = "demo")]
pub fn debug_heic_layout_check() {
	let ctx = egui::Context::default();
	let mut images = Avatars::default();
	let media = model::EmbedMedia {
		url: Some("https://cdn.discordapp.com/attachments/1/2/photo.heic".into()),
		..Default::default()
	};
	let frame = |images: &mut Avatars, surface| {
		let mut rect = egui::Rect::NOTHING;
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1000.0, 800.0),
				)),
				..Default::default()
			},
			|ui| {
				rect = images
					.show_media(ui, &media, egui::vec2(550.0, 350.0), false, surface)
					.response
					.rect;
			},
		);
		output.drop_without_applying_deltas();
		rect
	};
	frame(&mut images, Surface::Inline);
	let key = images.take_requests().pop().unwrap();
	images.accept(
		&ctx,
		key,
		Some(ColorImage::filled([300, 225], Color32::WHITE)),
	);
	let rect = frame(&mut images, Surface::Inline);
	assert!((rect.width() / rect.height() - 4.0 / 3.0).abs() < 0.01);
	assert!((rect.height() - 350.0).abs() < 0.01);
	assert_eq!(images.media_dimensions(&media), Some([300, 225]));
	images.take_requests();
	frame(&mut images, Surface::Viewer);
	let key = images.take_requests().pop().unwrap();
	assert!(key.starts_with("media:vs:e4096:"), "{key}");
	images.accept(
		&ctx,
		key,
		Some(ColorImage::filled([1024, 768], Color32::WHITE)),
	);
	assert_eq!(images.media_dimensions(&media), Some([1024, 768]));
	let rect = frame(&mut images, Surface::Viewer);
	assert!((rect.width() / rect.height() - 4.0 / 3.0).abs() < 0.01);
}
