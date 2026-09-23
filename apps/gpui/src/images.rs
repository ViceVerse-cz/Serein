//! Credential-free avatars, guild icons and inline attachment previews.
//!
//! Keys are validated and turned into CDN addresses here; service metadata never chooses a
//! host or path. One worker thread downloads and decodes with byte and pixel limits, and the
//! UI keeps decoded images in an LRU bounded by item count and bytes. The offline preview never
//! starts the worker, so it never touches the network.
use gpui::{App, RenderImage, Window};
use model::Id;
use std::{
	cell::RefCell,
	collections::{BTreeMap, HashMap},
	io::Cursor,
	sync::Arc,
	time::{Duration, Instant},
};
use tokio::sync::mpsc;

/// Decoded images held in RAM; shown ones are mirrored in the GPU atlas until evicted.
const MAX_ITEMS: usize = 256;
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
/// Waiting keys cost their text plus bookkeeping.
const WAITING_BYTES: usize = 64;
const REQUESTS: usize = 64;
const RESULTS: usize = 32;
const RESULTS_PER_TICK: usize = 16;
const JOBS: usize = 6;
/// A failed or stalled key is requested again only after this long.
const RETRY: Duration = Duration::from_secs(120);
const MAX_AVATAR_ENCODED: usize = 512 * 1024;
const MAX_MEDIA_ENCODED: usize = 8 * 1024 * 1024;
/// Longest kept edge for avatars and guild icons: twice the 46px rail tile.
const AVATAR_EDGE: u32 = 96;
/// Inline attachment layout box, in logical pixels.
pub const MEDIA_BOX: (u32, u32) = (420, 320);
/// Pixels kept per inline attachment: the layout box at 2x.
const MEDIA_PIXELS: (u32, u32) = (MEDIA_BOX.0 * 2, MEDIA_BOX.1 * 2);

/// Scales `width`x`height` down (never up) to fit the box, keeping the aspect ratio.
/// Unknown dimensions reserve a 16:9 slot so arrivals do not move rows.
pub fn fit(width: u32, height: u32, (max_w, max_h): (u32, u32)) -> (u32, u32) {
	if width == 0 || height == 0 {
		return fit(320, 180, (max_w, max_h));
	}
	if width <= max_w && height <= max_h {
		return (width, height);
	}
	let (w, h, mw, mh) = (
		u64::from(width),
		u64::from(height),
		u64::from(max_w),
		u64::from(max_h),
	);
	if w * mh >= h * mw {
		(max_w, (h * mw / w).max(1) as u32)
	} else {
		((w * mh / h).max(1) as u32, max_h)
	}
}

/// Cache key for an attachment preview; `None` unless it is a Discord attachment address.
/// The worker validates the full address before fetching.
pub fn media_key(media: &model::EmbedMedia) -> Option<String> {
	let source = media
		.proxy_url
		.as_deref()
		.or(media.url.as_deref())
		.filter(|source| {
			source.len() <= 2048
				// The offline preview draws every image locally, whatever its address.
				&& (preview()
					|| source.starts_with("https://cdn.discordapp.com/attachments/")
					|| source.starts_with("https://media.discordapp.net/attachments/")
					|| source.starts_with("https://images-ext-1.discordapp.net/external/")
					|| source.starts_with("https://images-ext-2.discordapp.net/external/"))
		})?;
	Some(format!("media:{}x{}:{source}", media.width, media.height))
}

// Build, rather than accept, URLs. Even malformed service metadata cannot choose a host/path.
fn cdn_url(key: &str) -> Option<String> {
	if let Some(value) = key.strip_prefix("media:") {
		let (size, source) = value.split_once(':')?;
		let (width, height) = size.split_once('x')?;
		return media_url(source, width.parse().ok()?, height.parse().ok()?);
	}
	// GIF picker stills: only provider media hosts and image formats the model allows.
	if let Some(preview) = key.strip_prefix("gif:") {
		return model::valid_gif_preview(preview).then(|| preview.to_owned());
	}
	if let Some(id) = key.strip_prefix("emoji-") {
		let id: Id = id.parse().ok()?;
		return Some(format!(
			"https://cdn.discordapp.com/emojis/{id}.png?size=64"
		));
	}
	if let Some(icon) = key.strip_prefix("guild-") {
		let (id, hash) = icon.split_once('-')?;
		let id: Id = id.parse().ok()?;
		return model::valid_avatar_hash(hash)
			.then(|| format!("https://cdn.discordapp.com/icons/{id}/{hash}.png?size=128"));
	}
	if let Some(index) = key.strip_prefix("default-") {
		return (index.len() == 1 && matches!(index.as_bytes()[0], b'0'..=b'5'))
			.then(|| format!("https://cdn.discordapp.com/embed/avatars/{index}.png"));
	}
	// Animated hashes are requested as their static PNG rendition.
	let (id, hash) = key.split_once('-')?;
	let id: Id = id.parse().ok()?;
	model::valid_avatar_hash(hash)
		.then(|| format!("https://cdn.discordapp.com/avatars/{id}/{hash}.png?size=128"))
}

/// Only Discord attachment addresses; the proxy resizes them to the kept pixel box.
fn media_url(source: &str, width: u32, height: u32) -> Option<String> {
	if source.len() > 2048 || source.bytes().any(|b| b.is_ascii_control() || b == b'\\') {
		return None;
	}
	let mut url = url::Url::parse(source).ok()?;
	if url.scheme() != "https"
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.port().is_some()
		|| url.fragment().is_some()
	{
		return None;
	}
	let host = url.host_str()?.to_owned();
	let mut parts = url.path().trim_start_matches('/').split('/');
	let valid = match parts.next() {
		Some("attachments") => {
			matches!(host.as_str(), "cdn.discordapp.com" | "media.discordapp.net")
				&& parts.next()?.parse::<Id>().is_ok()
				&& parts.next()?.parse::<Id>().is_ok()
				&& parts.next().is_some_and(|name| !name.is_empty())
				&& parts.next().is_none()
		}
		// Discord's own media proxy for embed images; the original host is never contacted.
		Some("external") => {
			matches!(
				host.as_str(),
				"images-ext-1.discordapp.net" | "images-ext-2.discordapp.net"
			) && parts.next().is_some_and(|hash| {
				(16..=256).contains(&hash.len())
					&& hash
						.bytes()
						.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
			}) && matches!(parts.next(), Some("https" | "http"))
				&& parts.next().is_some_and(|domain| !domain.is_empty())
		}
		_ => false,
	};
	if !valid {
		return None;
	}
	if host == "cdn.discordapp.com" {
		url.set_host(Some("media.discordapp.net")).ok()?;
	}
	let query: Vec<_> = url
		.query_pairs()
		.filter(|(key, _)| {
			!matches!(
				key.as_ref(),
				"format" | "width" | "height" | "quality" | "animated" | "fit"
			)
		})
		.map(|(key, value)| (key.into_owned(), value.into_owned()))
		.collect();
	url.set_query(None);
	let mut pairs = url.query_pairs_mut();
	pairs.extend_pairs(query).append_pair("format", "png");
	if width > 0 && height > 0 {
		let (width, height) = fit(width, height, MEDIA_PIXELS);
		pairs
			.append_pair("width", &width.to_string())
			.append_pair("height", &height.to_string());
	} else {
		pairs.append_pair("width", &MEDIA_PIXELS.0.to_string());
	}
	drop(pairs);
	Some(url.into())
}

/// Decode one still image within the pixel budget and shrink it to the kept size.
fn decode(bytes: &[u8], media: bool) -> Option<Arc<RenderImage>> {
	let limit = if media {
		MAX_MEDIA_ENCODED
	} else {
		MAX_AVATAR_ENCODED
	};
	if bytes.len() > limit {
		return None;
	}
	let mut reader = image::ImageReader::new(Cursor::new(bytes))
		.with_guessed_format()
		.ok()?;
	let mut limits = image::Limits::default();
	let side = if media { 2048 } else { 256 };
	limits.max_image_width = Some(side);
	limits.max_image_height = Some(side);
	limits.max_alloc = Some(if media { 24 } else { 1 } * 1024 * 1024);
	reader.limits(limits);
	let mut image = reader.decode().ok()?;
	let (width, height) = if media {
		fit(image.width(), image.height(), MEDIA_PIXELS)
	} else {
		fit(image.width(), image.height(), (AVATAR_EDGE, AVATAR_EDGE))
	};
	if (width, height) != (image.width(), image.height()) {
		let filter = if media {
			image::imageops::FilterType::Triangle
		} else {
			image::imageops::FilterType::Lanczos3
		};
		image = image.resize(width, height, filter);
	}
	let mut pixels = image.into_rgba8();
	// GPUI's atlas takes BGRA.
	for pixel in pixels.as_chunks_mut::<4>().0 {
		pixel.swap(0, 2);
	}
	Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
}

fn image_bytes(image: &RenderImage) -> usize {
	let size = image.size(0);
	(size.width.0.max(0) as usize) * (size.height.0.max(0) as usize) * 4
}

struct Loaded {
	key: String,
	image: Option<Arc<RenderImage>>,
}

fn spawn() -> Option<(mpsc::Sender<String>, mpsc::Receiver<Loaded>)> {
	let (requests, receive) = mpsc::channel(REQUESTS);
	let (send, results) = mpsc::channel(RESULTS);
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.ok()?;
	std::thread::Builder::new()
		.name("gpui-images".into())
		.spawn(move || runtime.block_on(run(receive, send)))
		.ok()?;
	Some((requests, results))
}

async fn run(mut requests: mpsc::Receiver<String>, results: mpsc::Sender<Loaded>) {
	// The shared browser fingerprint, never a custom agent; CDN reads carry no credentials.
	let Ok(client) = reqwest::Client::builder()
		.https_only(true)
		.no_proxy()
		.redirect(reqwest::redirect::Policy::none())
		.timeout(Duration::from_secs(15))
		.connect_timeout(Duration::from_secs(5))
		.pool_max_idle_per_host(2)
		.user_agent(client_core::fingerprint::user_agent())
		.build()
	else {
		return;
	};
	let mut cooldown = Instant::now();
	let mut jobs = tokio::task::JoinSet::new();
	loop {
		tokio::select! {
			done = jobs.join_next(), if !jobs.is_empty() => {
				let Some(Ok((loaded, until))) = done else { continue };
				cooldown = cooldown.max(until);
				if results.send(loaded).await.is_err() {
					break;
				}
				crate::backend::WAKE.notify_one();
			}
			key = requests.recv(), if jobs.len() < JOBS => {
				let Some(key) = key else { break };
				let Some(url) = cdn_url(&key) else { continue };
				jobs.spawn(load(client.clone(), key, url, cooldown));
			}
		}
	}
	jobs.abort_all();
}

async fn load(
	client: reqwest::Client,
	key: String,
	url: String,
	cooldown: Instant,
) -> (Loaded, Instant) {
	let media = key.starts_with("media:") || key.starts_with("gif:");
	let limit = if media {
		MAX_MEDIA_ENCODED
	} else {
		MAX_AVATAR_ENCODED
	};
	let mut until = cooldown;
	let image = match download(&client, &url, &mut until, limit).await {
		Some(bytes) => tokio::task::spawn_blocking(move || decode(&bytes, media))
			.await
			.ok()
			.flatten(),
		None => None,
	};
	(Loaded { key, image }, until)
}

async fn download(
	client: &reqwest::Client,
	url: &str,
	cooldown: &mut Instant,
	limit: usize,
) -> Option<Vec<u8>> {
	if Instant::now() < *cooldown {
		return None;
	}
	let mut response = client.get(url).send().await.ok()?;
	if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
		let seconds = response
			.headers()
			.get(reqwest::header::RETRY_AFTER)
			.and_then(|value| value.to_str().ok())
			.and_then(|value| value.parse::<f64>().ok())
			.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
			.unwrap_or(60.0)
			.clamp(1.0, 3600.0);
		*cooldown = Instant::now() + Duration::from_secs_f64(seconds);
		return None;
	}
	if !response.status().is_success()
		|| response
			.content_length()
			.is_some_and(|length| length > limit as u64)
	{
		return None;
	}
	let mut bytes =
		Vec::with_capacity(response.content_length().unwrap_or(4096).min(limit as u64) as usize);
	while let Some(chunk) = response.chunk().await.ok()? {
		if bytes.len().checked_add(chunk.len())? > limit {
			return None;
		}
		bytes.extend_from_slice(&chunk);
	}
	Some(bytes)
}

enum Slot {
	Ready(Arc<RenderImage>),
	/// Requested or failed at this time.
	Waiting(Instant),
}
struct Entry {
	slot: Slot,
	bytes: usize,
	tick: u64,
}

/// Least-recently-used images, bounded by item count and bytes.
#[derive(Default)]
struct Lru {
	entries: HashMap<String, Entry>,
	order: BTreeMap<u64, String>,
	tick: u64,
	bytes: usize,
	/// Evicted images whose atlas textures must be released with the app context.
	evicted: Vec<Arc<RenderImage>>,
}
impl Lru {
	fn touch(&mut self, key: &str) -> Option<&Slot> {
		let entry = self.entries.get_mut(key)?;
		self.tick += 1;
		if let Some(key) = self.order.remove(&entry.tick) {
			self.order.insert(self.tick, key);
		}
		entry.tick = self.tick;
		Some(&entry.slot)
	}
	fn insert(&mut self, key: String, slot: Slot) {
		let bytes = key.len()
			+ match &slot {
				Slot::Ready(image) => image_bytes(image),
				Slot::Waiting(_) => WAITING_BYTES,
			};
		self.remove(&key);
		self.tick += 1;
		self.bytes += bytes;
		self.order.insert(self.tick, key.clone());
		self.entries.insert(
			key,
			Entry {
				slot,
				bytes,
				tick: self.tick,
			},
		);
		while (self.entries.len() > MAX_ITEMS || self.bytes > MAX_BYTES) && self.entries.len() > 1 {
			let Some((_, oldest)) = self.order.pop_first() else {
				break;
			};
			if let Some(entry) = self.entries.remove(&oldest) {
				self.bytes -= entry.bytes;
				if let Slot::Ready(image) = entry.slot {
					self.evicted.push(image);
				}
			}
		}
	}
	/// Ready images and the bytes they hold, excluding waiting keys.
	fn ready(&self) -> (usize, usize) {
		self.entries
			.values()
			.filter(|entry| matches!(entry.slot, Slot::Ready(_)))
			.fold((0, 0), |(count, bytes), entry| {
				(count + 1, bytes + entry.bytes)
			})
	}
	/// Empties the cache; ready images move to `evicted` so their textures are released.
	fn clear(&mut self) {
		self.order.clear();
		self.bytes = 0;
		for (_, entry) in self.entries.drain() {
			if let Slot::Ready(image) = entry.slot {
				self.evicted.push(image);
			}
		}
		self.entries.shrink_to_fit();
	}
	fn remove(&mut self, key: &str) {
		if let Some(entry) = self.entries.remove(key) {
			self.order.remove(&entry.tick);
			self.bytes -= entry.bytes;
			if let Slot::Ready(image) = entry.slot {
				self.evicted.push(image);
			}
		}
	}
}

struct Store {
	requests: mpsc::Sender<String>,
	results: mpsc::Receiver<Loaded>,
	lru: Lru,
}
impl Store {
	fn get(&mut self, key: &str) -> Option<Arc<RenderImage>> {
		match self.lru.touch(key) {
			Some(Slot::Ready(image)) => return Some(image.clone()),
			Some(Slot::Waiting(since)) if since.elapsed() < RETRY => return None,
			_ => {}
		}
		// Invalid keys wait too, so they are not validated again every frame.
		// A full queue records nothing; the key is requested again on a later frame.
		if cdn_url(key).is_none() || self.requests.try_send(key.to_owned()).is_ok() {
			self.lru
				.insert(key.to_owned(), Slot::Waiting(Instant::now()));
			if !self.lru.evicted.is_empty() {
				crate::backend::WAKE.notify_one();
			}
		}
		None
	}
	fn accept(&mut self, loaded: Loaded) -> bool {
		// Keys evicted while loading are not wanted any more.
		if !matches!(
			self.lru.entries.get(&loaded.key).map(|e| &e.slot),
			Some(Slot::Waiting(_))
		) {
			return false;
		}
		match loaded.image {
			Some(image) => {
				self.lru.insert(loaded.key, Slot::Ready(image));
				true
			}
			None => {
				self.lru.insert(loaded.key, Slot::Waiting(Instant::now()));
				false
			}
		}
	}
}

thread_local! {
	static STORE: RefCell<Option<Store>> = const { RefCell::new(None) };
}

/// Starts the image worker for a signed-in session; the offline preview never calls this.
pub fn init() {
	STORE.with_borrow_mut(|store| {
		if store.is_none() {
			*store = spawn().map(|(requests, results)| Store {
				requests,
				results,
				lru: Lru::default(),
			});
		}
	});
}

/// Whether images load at all; false in the offline preview.
pub fn enabled() -> bool {
	STORE.with_borrow(Option::is_some)
}

/// The decoded image for `key`, requesting it on a miss. `None` while loading, after a
/// failure, or when images are disabled.
pub fn get(key: &str) -> Option<Arc<RenderImage>> {
	STORE.with_borrow_mut(|store| store.as_mut()?.get(key))
}

/// Decoded images held and their bytes; `None` when images are disabled (offline preview).
pub fn usage() -> Option<(usize, usize)> {
	STORE.with_borrow(|store| store.as_ref().map(|store| store.lru.ready()))
}

/// Drops every decoded image and releases its texture; shown ones download again. Loads in
/// flight are discarded when they arrive. Returns how many images were dropped.
pub fn clear(window: &mut Window, cx: &mut App) -> usize {
	let evicted = STORE.with_borrow_mut(|store| {
		let Some(store) = store else {
			return Vec::new();
		};
		store.lru.clear();
		std::mem::take(&mut store.lru.evicted)
	});
	let count = evicted.len();
	for image in evicted {
		cx.drop_image(image, Some(window));
	}
	count
}

/// Accepts finished loads and releases evicted textures. Returns whether any image arrived.
pub fn drain(window: &mut Window, cx: &mut App) -> bool {
	let (changed, evicted) = STORE.with_borrow_mut(|store| {
		let Some(store) = store else {
			return (false, Vec::new());
		};
		let mut changed = false;
		for accepted in 0..=RESULTS_PER_TICK {
			if accepted == RESULTS_PER_TICK {
				crate::backend::WAKE.notify_one();
				break;
			}
			let Ok(loaded) = store.results.try_recv() else {
				break;
			};
			changed |= store.accept(loaded);
		}
		(changed, std::mem::take(&mut store.lru.evicted))
	});
	for image in evicted {
		cx.drop_image(image, Some(window));
	}
	changed
}

thread_local! {
	/// Offline preview artwork by key, drawn on first use; `None` outside the preview.
	static DEMO: RefCell<Option<HashMap<String, Arc<RenderImage>>>> = const { RefCell::new(None) };
}

/// Turns on the offline preview's synthetic avatars and server icons (no network, no assets).
pub fn init_demo() {
	DEMO.with_borrow_mut(|demo| {
		demo.get_or_insert_default();
	});
}

/// 32x32 RGB artwork as the atlas's BGRA image, smoothed to 96 px so it scales like egui's
/// linearly filtered 32 px texture.
fn demo_image(pixels: impl Fn(u32, u32) -> [u8; 3]) -> Arc<RenderImage> {
	let image = image::RgbaImage::from_fn(32, 32, |x, y| {
		let [r, g, b] = pixels(x, y);
		image::Rgba([b, g, r, 255])
	});
	let image = image::imageops::resize(&image, 96, 96, image::imageops::FilterType::Triangle);
	Arc::new(RenderImage::new(vec![image::Frame::new(image)]))
}

/// The preview image for `key`, drawn by `paint` once; `None` outside the preview.
fn demo(key: String, paint: impl FnOnce() -> Arc<RenderImage>) -> Option<Arc<RenderImage>> {
	DEMO.with_borrow_mut(|demo| {
		let demo = demo.as_mut()?;
		if demo.len() >= MAX_ITEMS && !demo.contains_key(&key) {
			demo.clear();
		}
		Some(demo.entry(key).or_insert_with(paint).clone())
	})
}

/// Same synthetic silhouette as egui's `Avatars::paint_user` in the offline preview.
fn demo_user(id: Id) -> Arc<RenderImage> {
	let background = if id.0.is_multiple_of(2) {
		[63, 99, 111]
	} else {
		[103, 86, 124]
	};
	demo_image(|x, y| {
		let (x, y) = (x as i32, y as i32);
		if (x - 16).pow(2) + (y - 11).pow(2) < 36 || (x - 16).pow(2) + (y - 31).pow(2) < 121 {
			[224, 237, 227]
		} else {
			background
		}
	})
}

/// Same synthetic server icon as egui's `Avatars::paint_guild_face`: three white bars on a
/// colour seeded by the icon key.
fn demo_guild(key: &str) -> Arc<RenderImage> {
	let seed = key
		.bytes()
		.fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
	let fill = [
		70 + (seed % 140) as u8,
		90 + ((seed >> 8) % 120) as u8,
		110 + ((seed >> 16) % 100) as u8,
	];
	demo_image(|x, y| {
		if (7..25).contains(&x) && [8, 14, 20].iter().any(|row| (*row..row + 3).contains(&y)) {
			[255; 3]
		} else {
			fill
		}
	})
}

/// A user's avatar: the CDN picture once loaded, or the preview's synthetic silhouette.
pub fn user(user: &model::User) -> Option<Arc<RenderImage>> {
	demo(format!("preview-{}", user.id), || demo_user(user.id)).or_else(|| get(&user.avatar_key()))
}

/// A server's icon, if it has one: from the CDN, or the preview's synthetic artwork.
pub fn guild(guild: &model::Guild) -> Option<Arc<RenderImage>> {
	let key = guild.icon_key()?;
	demo(key.clone(), || demo_guild(&key)).or_else(|| get(&key))
}

/// A server tag badge: the preview's synthetic emblem (egui's `Avatars::show_icon`), else
/// whatever the cache holds.
pub fn badge(key: &str) -> Option<Arc<RenderImage>> {
	let seed = key
		.bytes()
		.fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
	let tint = [
		90 + (seed % 120) as u8,
		120 + ((seed >> 8) % 100) as u8,
		150 + ((seed >> 16) % 90) as u8,
	];
	let emblem = || {
		let image = image::RgbaImage::from_fn(32, 32, |x, y| {
			let d = (x as i32 - 16).pow(2) + (y as i32 - 16).pow(2);
			if d < 36 {
				image::Rgba([255; 4])
			} else if d < 196 {
				image::Rgba([tint[2], tint[1], tint[0], 255])
			} else {
				image::Rgba([0; 4])
			}
		});
		Arc::new(RenderImage::new(vec![image::Frame::new(image)]))
	};
	demo(key.to_owned(), emblem).or_else(|| get(key))
}

/// The offline preview is running (synthetic artwork, never the network).
fn preview() -> bool {
	DEMO.with_borrow(Option::is_some)
}

/// Whether media previews are shown: from the CDN, or drawn for the offline preview.
pub fn media_enabled() -> bool {
	enabled() || preview()
}

thread_local! {
	/// The one synthetic picture every preview media key shares, so the map holds no copies.
	static DEMO_MEDIA: std::cell::OnceCell<Arc<RenderImage>> = const { std::cell::OnceCell::new() };
}

/// Same synthetic picture as egui's offline media: a teal grid inside a white frame, 320x180.
fn demo_media() -> Arc<RenderImage> {
	DEMO_MEDIA.with(|media| {
		media
			.get_or_init(|| {
				let image = image::RgbaImage::from_fn(320, 180, |x, y| {
					let [r, g, b] = if x < 8 || y < 8 || x >= 312 || y >= 172 {
						[255, 255, 255]
					} else if x % 40 < 4 || y % 40 < 4 {
						[80, 180, 160]
					} else {
						[40, 50, 70]
					};
					image::Rgba([b, g, r, 255])
				});
				Arc::new(RenderImage::new(vec![image::Frame::new(image)]))
			})
			.clone()
	})
}

/// Same synthetic custom emoji as egui's offline preview for the fixture's two server emoji.
fn demo_emoji(id: Id) -> Option<Arc<RenderImage>> {
	let tone = match id.0 {
		9001 => [55, 180, 165],
		9002 => [240, 150, 70],
		_ => return None,
	};
	let image = image::RgbaImage::from_fn(32, 32, |x, y| {
		let inside = (3..29).contains(&x) && (3..29).contains(&y);
		if inside && (u64::from(x + y) + id.0) % 10 < 7 {
			image::Rgba([tone[2], tone[1], tone[0], 255])
		} else {
			image::Rgba([0, 0, 0, 0])
		}
	});
	Some(Arc::new(RenderImage::new(vec![image::Frame::new(image)])))
}

/// Same synthetic GIF still as egui's offline picker: a two-tone gradient with a highlight.
fn demo_gif(seed_text: &str, width: u32, height: u32) -> Arc<RenderImage> {
	let seed = seed_text.bytes().fold(7usize, |acc, b| {
		acc.wrapping_mul(31).wrapping_add(b as usize)
	});
	let w = 256u32;
	let h = ((256.0 * height as f32 / width.max(1) as f32) as u32).clamp(64, 512);
	let hue = ((seed % 97) as f32 * 0.618_034) % 1.0;
	let a = egui::ecolor::Hsva::new(hue, 0.62, 0.78, 1.0).to_rgba_premultiplied();
	let b = egui::ecolor::Hsva::new((hue + 0.12) % 1.0, 0.58, 0.42, 1.0).to_rgba_premultiplied();
	let (cx, cy) = (
		0.3 + (seed % 5) as f32 * 0.1,
		0.35 + (seed % 3) as f32 * 0.12,
	);
	let image = image::RgbaImage::from_fn(w, h, |x, y| {
		let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
		let t = ((u + v) * 0.5).clamp(0.0, 1.0);
		let d = ((u - cx).powi(2) + ((v - cy) * h as f32 / w as f32).powi(2)).sqrt();
		let glow = (1.0 - d / 0.5).clamp(0.0, 1.0).powi(2) * 0.3;
		let band = (((u * 3.0 - v * 2.0) * std::f32::consts::PI).sin() * 0.5 + 0.5) * 0.06;
		let channel =
			|i: usize| ((a[i] * (1.0 - t) + b[i] * t + glow + band) * 255.0).min(255.0) as u8;
		image::Rgba([channel(2), channel(1), channel(0), 255])
	});
	Arc::new(RenderImage::new(vec![image::Frame::new(image)]))
}

/// An attachment or embed preview by [`media_key`]: from the CDN, or the offline preview's grid.
pub fn media(key: &str) -> Option<Arc<RenderImage>> {
	demo(key.to_owned(), demo_media).or_else(|| get(key))
}

/// A custom emoji image: from the CDN, or the offline preview's synthetic artwork.
pub fn emoji(id: Id) -> Option<Arc<RenderImage>> {
	let key = format!("emoji-{id}");
	if preview() {
		let image = demo_emoji(id)?;
		return demo(key, || image);
	}
	get(&key)
}

/// A GIF picker still (first frame only): from its provider host, or drawn for the offline
/// preview at the GIF's aspect.
pub fn gif_still(gif: &model::Gif) -> Option<Arc<RenderImage>> {
	if !model::valid_gif_preview(&gif.preview) {
		return None;
	}
	let key = format!("gif:{}", gif.preview);
	if preview() {
		return demo(key, || demo_gif(&gif.id, gif.width, gif.height));
	}
	get(&key)
}

/// GIF category artwork from its provider host; the offline preview has none (flat tiles).
pub fn gif_category(preview_url: &str) -> Option<Arc<RenderImage>> {
	(!preview() && model::valid_gif_preview(preview_url))
		.then(|| get(&format!("gif:{preview_url}")))
		.flatten()
}

#[cfg(test)]
mod tests {
	use super::{Lru, MAX_BYTES, MAX_ITEMS, Slot, cdn_url, decode, fit, media_key};
	use gpui::RenderImage;
	use std::{sync::Arc, time::Instant};

	fn image(width: u32, height: u32) -> Slot {
		Slot::Ready(Arc::new(RenderImage::new(vec![image::Frame::new(
			image::RgbaImage::new(width, height),
		)])))
	}

	#[test]
	fn urls_are_built_only_for_discord_cdn_keys() {
		let hash = "0123456789abcdef0123456789abcdef";
		assert_eq!(
			cdn_url(&format!("42-a_{hash}")).as_deref(),
			Some(format!("https://cdn.discordapp.com/avatars/42/a_{hash}.png?size=128").as_str())
		);
		assert_eq!(
			cdn_url(&format!("guild-7-{hash}")).as_deref(),
			Some(format!("https://cdn.discordapp.com/icons/7/{hash}.png?size=128").as_str())
		);
		assert!(cdn_url("default-3").is_some());
		assert_eq!(
			cdn_url("emoji-9001").as_deref(),
			Some("https://cdn.discordapp.com/emojis/9001.png?size=64")
		);
		for bad in [
			"default-6",
			"0-abc",
			"42-../x",
			"guild-7-xyz",
			"42-",
			"emoji-x",
			"emoji-1/2",
		] {
			assert!(cdn_url(bad).is_none(), "{bad}");
		}
		let media = model::EmbedMedia {
			proxy_url: Some(
				"https://cdn.discordapp.com/attachments/1/2/photo.jpg?ex=a&hm=b&width=9&format=webp"
					.into(),
			),
			width: 4000,
			height: 3000,
			..Default::default()
		};
		let proxied = cdn_url(
			"media:10x10:https://images-ext-2.discordapp.net/external/abcdefghijklmnop/https/example.com/a.png",
		)
		.unwrap();
		assert!(proxied.starts_with(
			"https://images-ext-2.discordapp.net/external/abcdefghijklmnop/https/example.com/a.png?"
		));
		assert_eq!(
			cdn_url(&media_key(&media).unwrap()).as_deref(),
			Some(
				"https://media.discordapp.net/attachments/1/2/photo.jpg?ex=a&hm=b&format=png&width=840&height=630"
			)
		);
		for source in [
			"https://example.com/attachments/1/2/a.png",
			"http://cdn.discordapp.com/attachments/1/2/a.png",
			"https://user@cdn.discordapp.com/attachments/1/2/a.png",
			"https://cdn.discordapp.com:444/attachments/1/2/a.png",
			"https://cdn.discordapp.com/attachments/1/2/a.png#x",
			"https://cdn.discordapp.com/avatars/1/2.png",
			"https://cdn.discordapp.com/attachments/1/x/a.png",
			"https://cdn.discordapp.com/attachments/1/2/a/b.png",
			"https://images-ext-3.discordapp.net/external/abcdefghijklmnop/https/example.com/a.png",
			"https://images-ext-1.discordapp.net/external/short/https/example.com/a.png",
			"https://images-ext-1.discordapp.net/external/abcdefghijklmnop/ftp/example.com/a.png",
			"https://media.discordapp.net/external/abcdefghijklmnop/https/example.com/a.png",
			"https://images-ext-1.discordapp.net/attachments/1/2/a.png",
		] {
			assert!(
				cdn_url(&format!("media:10x10:{source}")).is_none(),
				"{source}"
			);
		}
	}

	#[test]
	fn decoding_shrinks_to_budget_and_swaps_to_bgra() {
		let mut png = Vec::new();
		image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
			200,
			100,
			image::Rgba([255, 0, 0, 255]),
		))
		.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
		.unwrap();
		let avatar = decode(&png, false).unwrap();
		assert_eq!((avatar.size(0).width.0, avatar.size(0).height.0), (96, 48));
		assert_eq!(&avatar.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
		assert!(decode(b"not an image", true).is_none());
		let mut huge = Vec::new();
		image::DynamicImage::ImageLuma8(image::GrayImage::new(300, 1))
			.write_to(
				&mut std::io::Cursor::new(&mut huge),
				image::ImageFormat::Png,
			)
			.unwrap();
		assert!(decode(&huge, false).is_none(), "avatar side limit");
	}

	#[test]
	fn fitting_keeps_aspect_and_never_upscales() {
		assert_eq!(fit(4000, 3000, (420, 320)), (420, 315));
		assert_eq!(fit(1000, 4000, (420, 320)), (80, 320));
		assert_eq!(fit(100, 50, (420, 320)), (100, 50));
		assert_eq!(fit(0, 0, (420, 320)), (320, 180));
	}

	#[test]
	fn lru_evicts_oldest_by_bytes_and_items() {
		let mut lru = Lru::default();
		// Each 1024x1024 image costs 4 MiB plus its key, so the byte budget holds seven.
		for index in 0..10 {
			lru.insert(format!("{index}"), image(1024, 1024));
			assert!(lru.touch("0").is_some());
		}
		assert!(lru.bytes <= MAX_BYTES);
		assert_eq!(lru.entries.len(), 7);
		assert!(lru.touch("0").is_some(), "recently used entry survives");
		assert!(["1", "2", "3"].iter().all(|key| lru.touch(key).is_none()));
		assert_eq!(lru.evicted.len(), 3);
		for index in 0..MAX_ITEMS + 10 {
			lru.insert(format!("w{index}"), Slot::Waiting(Instant::now()));
		}
		assert_eq!(lru.entries.len(), MAX_ITEMS);
		assert_eq!(lru.evicted.len(), 10);
		assert_eq!(
			lru.bytes,
			lru.entries.values().map(|entry| entry.bytes).sum::<usize>()
		);
	}

	#[test]
	fn clearing_drops_every_image_and_releases_its_texture() {
		let mut lru = Lru::default();
		lru.insert("a".into(), image(16, 16));
		lru.insert("b".into(), image(8, 8));
		lru.insert("c".into(), Slot::Waiting(Instant::now()));
		let (count, bytes) = lru.ready();
		assert_eq!(count, 2);
		assert_eq!(bytes, 1 + 16 * 16 * 4 + 1 + 8 * 8 * 4);
		lru.clear();
		assert_eq!(lru.ready(), (0, 0));
		assert!(lru.entries.is_empty() && lru.order.is_empty());
		assert_eq!(lru.bytes, 0);
		assert_eq!(lru.evicted.len(), 2);
		// The cache keeps working afterwards.
		lru.insert("a".into(), image(4, 4));
		assert!(lru.touch("a").is_some());
	}
}
