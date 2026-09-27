//! Credential-free, viewport-driven static avatars. No tokens enter this worker.
use eframe::egui;
use image::ImageEncoder;
use model::Id;
use rasterlottie::{Animation as Lottie, RenderConfig, Renderer, Rgba8};
use sha2::{Digest, Sha256};
use std::{
	collections::{BinaryHeap, VecDeque},
	fs::{self, OpenOptions},
	io::{self, Cursor, Read, Write},
	path::{Path, PathBuf},
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU64, Ordering},
		mpsc,
	},
	time::{Duration, Instant, SystemTime},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc as async_mpsc, watch};
use ui::{Lane, Motion, Rendition, Size};

const MAX_ENCODED: usize = 2 * 1024 * 1024;
const MAX_ANIMATED_ENCODED: usize = 16 * 1024 * 1024;
const MAX_MEDIA_ENCODED: usize = 32 * 1024 * 1024;
const ANIMATION_CANVAS: u32 = 2048;
// GIF decoding can hold a persistent canvas, a frame and a composited canvas.
const ANIMATION_ALLOC: u64 = 3 * 2048 * 2048 * 4;
const QUEUED: usize = 1024;
const RESULT_BYTES: usize = 128 * 1024 * 1024;
fn is_animated_key(key: &str) -> bool {
	if key.starts_with("anim:") {
		return true;
	}
	if let Some(value) = key.strip_prefix("banner-") {
		return value
			.split_once('-')
			.is_some_and(|(_, hash)| hash.starts_with("a_"));
	}
	if let Some(value) = key.strip_prefix("member-banner-") {
		let mut parts = value.split('-');
		return parts.nth(2).is_some_and(|hash| hash.starts_with("a_"));
	}
	if let Some(value) = key.strip_prefix("member-avatar-") {
		let mut parts = value.split('-');
		return parts.nth(2).is_some_and(|hash| hash.starts_with("a_"));
	}
	if let Some((id, hash)) = key.split_once('-')
		&& id.parse::<Id>().is_ok()
	{
		return hash.starts_with("a_");
	}
	false
}
fn encoded_limit(key: &str) -> usize {
	if let Some(rendition) = Rendition::parse(key) {
		media_encoded(&rendition)
	} else if is_animated_key(key) {
		MAX_ANIMATED_ENCODED
	} else if key.starts_with("gif:") {
		// Provider previews are full clips even when only their first frame is shown.
		MAX_ANIMATED_ENCODED
	} else {
		MAX_ENCODED
	}
}
fn lottie_key(key: &str) -> bool {
	key.starts_with("embed:sticker-") && key.ends_with("-3")
}
fn media_encoded(rendition: &Rendition) -> usize {
	let (width, height) = match rendition.size {
		Size::Exact { width, height } => (width, height),
		Size::Longest(edge) => (edge.get(), edge.get()),
	};
	match rendition.motion {
		Motion::Animated => MAX_ANIMATED_ENCODED,
		Motion::Still => {
			(width as usize * height as usize * 3).clamp(MAX_ENCODED, MAX_MEDIA_ENCODED)
		}
	}
}
fn decode_edge(key: &str) -> u32 {
	if key.starts_with("anim:")
		|| key.starts_with("embed:")
		|| key.starts_with("gif:")
		|| key.starts_with("spotify-")
		|| key.starts_with("banner-")
		|| key.starts_with("member-banner-")
	{
		ui::EMBED_EDGE
	} else {
		128
	}
}

#[derive(Clone, Copy)]
struct Budget {
	fit: u32,
	encoded: usize,
	canvas: u32,
	alloc: u64,
	frames: Option<FrameBudget>,
}

#[derive(Clone, Copy)]
struct FrameBudget {
	fit: u32,
	bytes: usize,
	count: usize,
	shrinks: u32,
}

impl Budget {
	fn legacy(edge: u32) -> Self {
		let embed = edge >= ui::EMBED_EDGE;
		Self {
			fit: if edge <= 128 { 64 } else { edge },
			encoded: if embed {
				MAX_ANIMATED_ENCODED
			} else {
				MAX_AVATAR_ENCODED
			},
			canvas: if embed { 1024 } else { 256 },
			alloc: if embed { 8 * 1024 * 1024 } else { 1024 * 1024 },
			frames: None,
		}
	}
}

impl FrameBudget {
	fn legacy(edge: u32) -> Self {
		Self {
			fit: edge.min(ui::EMBED_EDGE),
			bytes: 12 * 1024 * 1024,
			count: 80,
			shrinks: 0,
		}
	}
}

fn budget(key: &str) -> Budget {
	let Some(rendition) = Rendition::parse(key) else {
		let edge = decode_edge(key);
		return Budget {
			frames: is_animated_key(key).then(|| FrameBudget::legacy(edge)),
			..Budget::legacy(edge)
		};
	};
	let longest = rendition.size.longest();
	match rendition.motion {
		Motion::Still => Budget {
			fit: longest,
			encoded: media_encoded(&rendition),
			canvas: match rendition.size {
				Size::Exact { .. } => (longest * 2).min(8192),
				Size::Longest(_) => 8192,
			},
			alloc: 128 * 1024 * 1024,
			frames: None,
		},
		Motion::Animated => Budget {
			fit: longest,
			encoded: MAX_ANIMATED_ENCODED,
			canvas: ANIMATION_CANVAS,
			alloc: ANIMATION_ALLOC,
			frames: Some(FrameBudget {
				fit: longest,
				bytes: rendition.lane.frame_bytes(),
				count: ui::MAX_FRAMES,
				shrinks: 2,
			}),
		},
	}
}
const MAX_AVATAR_ENCODED: usize = 512 * 1024;
const MAX_LOTTIE_ENCODED: usize = 512 * 1024;
const MAX_APPLICATION_METADATA: usize = 64 * 1024;
const MAX_DISK: u64 = 1024 * 1024 * 1024;
const MAX_FILES: usize = 4096;
const RETENTION: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const CACHE_ERROR: &str = "Images could not be cached; they remain available in memory";
pub type Cleanup = mpsc::Receiver<Result<(), &'static str>>;

pub struct AvatarResult {
	pub key: String,
	pub image: Option<egui::ColorImage>,
	pub frames: ui::GifFrames,
	pub error: Option<&'static str>,
	pub stage: DecodeStage,
	// Stays charged until the UI has consumed or discarded the complete result.
	_reservation: Option<OwnedSemaphorePermit>,
}

impl AvatarResult {
	fn bytes(&self) -> usize {
		let pixels =
			|image: &egui::ColorImage| image.pixels.capacity() * size_of::<egui::Color32>();
		size_of::<Self>()
			+ self.key.capacity()
			+ self.image.as_ref().map_or(0, pixels)
			+ self.frames.capacity() * size_of::<(Duration, Arc<egui::ColorImage>)>()
			+ self
				.frames
				.iter()
				.map(|(_, image)| {
					pixels(image) + size_of::<egui::ColorImage>() + 2 * size_of::<usize>()
				})
				.sum::<usize>()
	}
}

#[derive(Clone)]
struct ResultSender {
	send: async_mpsc::Sender<AvatarResult>,
	bytes: Arc<Semaphore>,
	wake: egui::Context,
}

fn result_channel(wake: egui::Context) -> (ResultSender, async_mpsc::Receiver<AvatarResult>) {
	let (send, receive) = async_mpsc::channel(128);
	(
		ResultSender {
			send,
			bytes: Arc::new(Semaphore::new(RESULT_BYTES)),
			wake,
		},
		receive,
	)
}

impl ResultSender {
	async fn send(&self, mut result: AvatarResult) -> bool {
		if result.bytes() > RESULT_BYTES {
			// A future decoder change must not wait forever for an impossible reservation.
			result.image = None;
			result.frames = Vec::new();
			result.error = Some("Decoded image exceeds the memory limit");
		}
		let bytes = result.bytes();
		if bytes > RESULT_BYTES {
			return false;
		}
		let permit = tokio::select! {
			_ = self.send.closed() => return false,
			permit = self.bytes.clone().acquire_many_owned(bytes as u32) => permit,
		};
		let Ok(permit) = permit else { return false };
		result._reservation = Some(permit);
		if self.send.send(result).await.is_err() {
			return false;
		}
		self.wake.request_repaint();
		true
	}

	fn preview(&self, mut result: AvatarResult) -> bool {
		let bytes = result.bytes();
		if bytes > RESULT_BYTES {
			return false;
		}
		let Ok(permit) = self.bytes.clone().try_acquire_many_owned(bytes as u32) else {
			return false;
		};
		result._reservation = Some(permit);
		// Previews are optional; decoding and shutdown must never wait for a stopped UI.
		if self.send.try_send(result).is_err() {
			return false;
		}
		self.wake.request_repaint();
		true
	}
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DecodeStage {
	Preview,
	Settled,
}

pub struct AvatarWorker {
	requests: async_mpsc::Sender<String>,
	results: async_mpsc::Receiver<AvatarResult>,
	wake: egui::Context,
	cancel: watch::Sender<bool>,
	clear: Arc<AtomicBool>,
	cleanup: Option<Cleanup>,
}
impl AvatarWorker {
	pub fn start(
		runtime: &tokio::runtime::Runtime,
		account: Id,
		ctx: egui::Context,
	) -> Result<Self, &'static str> {
		let root = dirs::data_local_dir().map(|root| {
			root.join("serein")
				.join("avatars")
				.join(account.to_string())
		});
		Self::start_at(runtime, root, ctx)
	}
	fn start_at(
		runtime: &tokio::runtime::Runtime,
		root: Option<PathBuf>,
		ctx: egui::Context,
	) -> Result<Self, &'static str> {
		let (requests, receive) = async_mpsc::channel(1024);
		let (send, results) = result_channel(ctx.clone());
		let wake = ctx.clone();
		let (cancel, cancelled) = watch::channel(false);
		let clear = Arc::new(AtomicBool::new(false));
		let cleanup_flag = clear.clone();
		let (done, cleanup) = mpsc::sync_channel(1);
		let handle = runtime.handle().clone();
		std::thread::Builder::new()
			.name("avatar-cache".into())
			.spawn(move || {
				handle.block_on(run(root.as_deref(), receive, send, cancelled));
				let result = if cleanup_flag.load(Ordering::Acquire) {
					clear_directory(root.as_deref())
				} else {
					Ok(())
				};
				let _ = done.send(result);
				ctx.request_repaint();
			})
			.map_err(|_| "Could not start image worker")?;
		Ok(Self {
			requests,
			results,
			wake,
			cancel,
			clear,
			cleanup: Some(cleanup),
		})
	}
	pub fn request(&self, key: String) -> bool {
		cdn_url(&key).is_some() && self.requests.try_send(key).is_ok()
	}
	pub fn poll(&mut self) -> Option<AvatarResult> {
		let result = self.results.try_recv().ok()?;
		if !self.results.is_empty() {
			self.wake.request_repaint();
		}
		Some(result)
	}
	/// Completion follows the last decode/write. Recreate the account worker only afterwards.
	pub fn shutdown_and_clear(self) -> Cleanup {
		self.finish(true)
	}
	pub fn shutdown(self) -> Cleanup {
		self.finish(false)
	}
	fn finish(mut self, clear: bool) -> Cleanup {
		self.clear.store(clear, Ordering::Release);
		self.cancel.send_replace(true);
		self.cleanup
			.take()
			.expect("worker owns its cleanup receiver")
	}
}
impl Drop for AvatarWorker {
	fn drop(&mut self) {
		self.cancel.send_replace(true);
	}
}

fn clear_directory(root: Option<&Path>) -> Result<(), &'static str> {
	let root = root.ok_or("Could not locate cached images for removal")?;
	match fs::remove_dir_all(root) {
		Ok(()) => Ok(()),
		Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
		Err(_) => Err("Could not remove cached images; files may remain on disk"),
	}
}

fn is_direct_gif_url(url: &str) -> bool {
	if model::valid_gif_url(url) && url.split('?').next().is_some_and(|p| p.ends_with(".gif")) {
		return true;
	}
	let Ok(parsed) = url::Url::parse(url) else {
		return false;
	};
	if parsed.scheme() != "https"
		|| !parsed.username().is_empty()
		|| parsed.password().is_some()
		|| parsed.port().is_some()
		|| parsed.fragment().is_some()
	{
		return false;
	}
	let Some(host) = parsed.host_str() else {
		return false;
	};
	if !matches!(host, "cdn.discordapp.com" | "media.discordapp.net") {
		return false;
	}
	let path = parsed.path();
	if !path.ends_with(".gif") {
		return false;
	}
	if path.starts_with("/attachments/") {
		let mut parts = path.trim_start_matches('/').split('/');
		parts.next();
		parts.next().is_some_and(|id| id.parse::<Id>().is_ok())
			&& parts.next().is_some_and(|id| id.parse::<Id>().is_ok())
			&& parts.next().is_some_and(|name| !name.is_empty())
			&& parts.next().is_none()
	} else if path.starts_with("/stickers/") || path.starts_with("/emojis/") {
		let mut parts = path.trim_start_matches('/').split('/');
		parts.next();
		parts
			.next()
			.and_then(|file| file.strip_suffix(".gif"))
			.is_some_and(|id| id.parse::<Id>().is_ok())
			&& parts.next().is_none()
	} else {
		let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
		matches!(parts.as_slice(), ["avatars" | "icons" | "banners", id, hash]
			if id.parse::<Id>().is_ok() && hash.strip_suffix(".gif").is_some_and(model::valid_avatar_hash))
			|| matches!(parts.as_slice(), ["guilds", guild, "users", user, "avatars" | "banners", hash]
				if guild.parse::<Id>().is_ok() && user.parse::<Id>().is_ok()
					&& hash.strip_suffix(".gif").is_some_and(model::valid_avatar_hash))
	}
}

// Build, rather than accept, URLs. Even malformed service metadata cannot choose a host/path.
fn cdn_url(key: &str) -> Option<String> {
	if key.starts_with("media:") {
		return media_urls(&Rendition::parse(key)?).map(|urls| urls.primary);
	}
	if let Some(value) = key
		.strip_prefix("anim:sticker-")
		.or_else(|| key.strip_prefix("embed:sticker-"))
	{
		let (id, format) = value.split_once('-')?;
		let id: Id = id.parse().ok()?;
		if id.0 == 0 {
			return None;
		}
		return match format {
			"1" | "2" => Some(format!("https://cdn.discordapp.com/stickers/{id}.png")),
			"4" => Some(format!("https://media.discordapp.net/stickers/{id}.gif")),
			"3" if key.starts_with("embed:") => {
				Some(format!("https://cdn.discordapp.com/stickers/{id}.json"))
			}
			_ => None,
		};
	}
	if let Some(value) = key.strip_prefix("role-icon-") {
		let (role, hash) = value.split_once('-')?;
		let role: Id = role.parse().ok()?;
		return (role.0 != 0 && model::valid_avatar_hash(hash))
			.then(|| format!("https://cdn.discordapp.com/role-icons/{role}/{hash}.png?size=128"));
	}
	if let Some(value) = key.strip_prefix("application-icon-") {
		let (application, hash) = value.split_once('-')?;
		let application: Id = application.parse().ok()?;
		return model::valid_avatar_hash(hash).then(|| {
			format!("https://cdn.discordapp.com/app-icons/{application}/{hash}.png?size=128")
		});
	}
	if let Some(value) = key.strip_prefix("group-icon-") {
		let (channel, hash) = value.split_once('-')?;
		let channel: Id = channel.parse().ok()?;
		return model::valid_avatar_hash(hash).then(|| {
			format!("https://cdn.discordapp.com/channel-icons/{channel}/{hash}.png?size=128")
		});
	}
	if let Some(id) = key.strip_prefix("spotify-") {
		return model::ActivityImage::Spotify(id.into())
			.valid()
			.then(|| format!("https://i.scdn.co/image/{id}"));
	}
	if let Some(id) = key.strip_prefix("app-icon-") {
		let id: Id = id.parse().ok()?;
		return Some(format!("https://discord.com/api/v10/applications/{id}/rpc"));
	}
	if let Some(value) = key.strip_prefix("activity-") {
		let (application, asset) = value.split_once('-')?;
		let application: Id = application.parse().ok()?;
		let asset: Id = asset.parse().ok()?;
		return Some(format!(
			"https://cdn.discordapp.com/app-assets/{application}/{asset}.png?size=128"
		));
	}
	if let Some(id) = key.strip_prefix("emoji-") {
		let id: Id = id.parse().ok()?;
		return Some(format!(
			"https://cdn.discordapp.com/emojis/{id}.png?size=64"
		));
	}
	if let Some(value) = key.strip_prefix("banner-") {
		let (id, hash) = value.split_once('-')?;
		let id: Id = id.parse().ok()?;
		let ext = if hash.starts_with("a_") { "gif" } else { "png" };
		return model::valid_avatar_hash(hash)
			.then(|| format!("https://cdn.discordapp.com/banners/{id}/{hash}.{ext}?size=512"));
	}
	if let Some(value) = key.strip_prefix("member-banner-") {
		let (guild, value) = value.split_once('-')?;
		let (user, hash) = value.split_once('-')?;
		let guild: Id = guild.parse().ok()?;
		let user: Id = user.parse().ok()?;
		let ext = if hash.starts_with("a_") { "gif" } else { "png" };
		return model::valid_avatar_hash(hash).then(|| {
			format!(
				"https://cdn.discordapp.com/guilds/{guild}/users/{user}/banners/{hash}.{ext}?size=512"
			)
		});
	}
	if let Some(value) = key.strip_prefix("member-avatar-") {
		let (guild, value) = value.split_once('-')?;
		let (user, hash) = value.split_once('-')?;
		let guild: Id = guild.parse().ok()?;
		let user: Id = user.parse().ok()?;
		let ext = if hash.starts_with("a_") { "gif" } else { "png" };
		return model::valid_avatar_hash(hash).then(|| {
			format!(
				"https://cdn.discordapp.com/guilds/{guild}/users/{user}/avatars/{hash}.{ext}?size=128"
			)
		});
	}
	if let Some(source) = key.strip_prefix("anim:") {
		return (model::valid_gif_preview(source)
			&& (source.ends_with(".gif") || source.ends_with(".webp")))
		.then(|| source.to_owned());
	}
	if let Some(source) = key.strip_prefix("embed:") {
		return embed_url(source, ui::EMBED_EDGE);
	}
	// Provider previews arrive only inside a service GIF result; the address is used verbatim.
	if let Some(source) = key.strip_prefix("gif:") {
		return model::valid_gif_preview(source).then(|| source.to_owned());
	}
	// Profile badge and server-tag artwork hashes arrive only inside a requested profile.
	if let Some(hash) = key.strip_prefix("badge-") {
		return model::valid_avatar_hash(hash)
			.then(|| format!("https://cdn.discordapp.com/badge-icons/{hash}.png?size=64"));
	}
	if let Some(value) = key.strip_prefix("clan-") {
		let (guild, hash) = value.split_once('-')?;
		let guild: Id = guild.parse().ok()?;
		return model::valid_avatar_hash(hash)
			.then(|| format!("https://cdn.discordapp.com/clan-badges/{guild}/{hash}.png?size=64"));
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
	let (id, hash) = key.split_once('-')?;
	let id: Id = id.parse().ok()?;
	let digest = hash.strip_prefix("a_").unwrap_or(hash);
	let ext = if hash.starts_with("a_") { "gif" } else { "png" };
	(digest.len() == 32 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
		.then(|| format!("https://cdn.discordapp.com/avatars/{id}/{hash}.{ext}?size=128"))
}

/// Every value is unofficial proxy behavior.
#[derive(Clone, Copy)]
enum ProxyFormat {
	/// Observed in official client URLs, not documented.
	LosslessWebp,
	/// Unverified for attachments.
	AnimatedWebp,
	Gif,
}

impl ProxyFormat {
	fn query(self) -> &'static [(&'static str, &'static str)] {
		match self {
			Self::LosslessWebp => &[("format", "webp"), ("quality", "lossless")],
			Self::AnimatedWebp => &[("format", "webp"), ("animated", "true")],
			Self::Gif => &[("format", "gif")],
		}
	}
}

struct MediaUrls {
	primary: String,
	fallback: Option<String>,
}

fn media_urls(rendition: &Rendition) -> Option<MediaUrls> {
	let source = rendition.source.as_str();
	let size = rendition.size;
	let path = source.split(['?', '#']).next().unwrap_or(source);
	let webp = path
		.rsplit_once('.')
		.is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("webp"));
	let provider = (model::valid_gif_url(source) && path.ends_with(".gif"))
		|| (model::valid_gif_preview(source)
			&& (path.ends_with(".gif") || path.ends_with(".webp")));
	let (primary, fallback) = match rendition.motion {
		Motion::Still => (proxy_url(source, size, ProxyFormat::LosslessWebp), None),
		Motion::Animated if let Some(video) = motion_video_source(source) => (Some(video), None),
		Motion::Animated if provider => (Some(source.to_owned()), None),
		Motion::Animated if webp => (
			proxy_url(source, size, ProxyFormat::AnimatedWebp),
			proxy_url(source, size, ProxyFormat::Gif),
		),
		Motion::Animated => (
			proxy_url(source, size, ProxyFormat::Gif),
			direct_gif(source, size),
		),
	};
	match primary {
		Some(primary) => Some(MediaUrls { primary, fallback }),
		None => fallback.map(|primary| MediaUrls {
			primary,
			fallback: None,
		}),
	}
}

fn motion_video_source(source: &str) -> Option<String> {
	if !ui::is_motion_video(source) {
		return None;
	}
	let url = url::Url::parse(source).ok()?;
	if url.scheme() != "https"
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.port().is_some()
	{
		return None;
	}
	let host = url.host_str()?;
	let allowed = model::valid_gif_url(source)
		|| matches!(
			host,
			"cdn.discordapp.com"
				| "media.discordapp.net"
				| "images-ext-1.discordapp.net"
				| "images-ext-2.discordapp.net"
		);
	allowed.then(|| source.to_owned())
}

fn direct_gif(source: &str, size: Size) -> Option<String> {
	let mut url = url::Url::parse(source).ok()?;
	if url.host_str() == Some("media.discordapp.net") {
		url.set_host(Some("cdn.discordapp.com")).ok()?;
	}
	if !url.path().starts_with("/attachments/") {
		let side = size.longest().next_power_of_two().clamp(16, 4096);
		url.query_pairs_mut().append_pair("size", &side.to_string());
	}
	let url = String::from(url);
	is_direct_gif_url(&url).then_some(url)
}

fn proxy_url(source: &str, size: Size, format: ProxyFormat) -> Option<String> {
	let url = proxy_base(source)?;
	Some(match size {
		Size::Exact { width, height } => proxy_query(url, format.query(), width, Some(height)),
		Size::Longest(edge) => proxy_query(url, format.query(), edge.get(), None),
	})
}

pub(crate) fn embed_url(source: &str, edge: u32) -> Option<String> {
	let url = proxy_base(source)?;
	let dimension = |name| {
		url.query_pairs()
			.find(|(key, _)| key == name)
			.and_then(|(_, value)| value.parse::<u32>().ok())
			.filter(|value| *value > 0)
	};
	let format = &[("format", "png")];
	Some(
		match dimension("width")
			.zip(dimension("height"))
			.map(|(width, height)| ui::fit_edge(width, height, edge))
		{
			Some((width, height)) => proxy_query(url, format, width, Some(height)),
			None => proxy_query(url, format, edge, None),
		},
	)
}

fn proxy_query(
	mut url: url::Url,
	format: &[(&str, &str)],
	width: u32,
	height: Option<u32>,
) -> String {
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
	{
		let mut pairs = url.query_pairs_mut();
		pairs
			.extend_pairs(query)
			.extend_pairs(format.iter().copied())
			.append_pair("width", &width.to_string());
		if let Some(height) = height {
			pairs.append_pair("height", &height.to_string());
		}
	}
	url.into()
}

fn proxy_base(source: &str) -> Option<url::Url> {
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
	let host = url.host_str()?;
	let path = url.path();
	let valid_path = if path.starts_with("/attachments/") {
		let mut parts = path.trim_start_matches('/').split('/');
		parts.next();
		parts.next()?.parse::<Id>().is_ok()
			&& parts.next()?.parse::<Id>().is_ok()
			&& parts.next().is_some_and(|name| !name.is_empty())
			&& parts.next().is_none()
	} else if path.starts_with("/external/") {
		let mut parts = path.trim_start_matches('/').split('/');
		parts.next();
		parts.next().is_some_and(|hash| {
			(16..=256).contains(&hash.len())
				&& hash
					.bytes()
					.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
		}) && matches!(parts.next(), Some("https" | "http"))
			&& parts.next().is_some_and(|domain| !domain.is_empty())
	} else {
		let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
		matches!(parts.as_slice(), ["avatars" | "icons" | "banners", id, hash]
            if id.parse::<Id>().is_ok() && hash.rsplit_once('.').is_some_and(|(hash, _)| model::valid_avatar_hash(hash)))
			|| matches!(parts.as_slice(), ["guilds", guild, "users", user, "avatars" | "banners", hash]
                if guild.parse::<Id>().is_ok() && user.parse::<Id>().is_ok()
                    && hash.rsplit_once('.').is_some_and(|(hash, _)| model::valid_avatar_hash(hash)))
			|| matches!(parts.as_slice(), ["embed", "avatars", index]
                if matches!(*index, "0.png" | "1.png" | "2.png" | "3.png" | "4.png" | "5.png"))
	};
	if !valid_path
		|| !matches!(
			host,
			"cdn.discordapp.com"
				| "media.discordapp.net"
				| "images-ext-1.discordapp.net"
				| "images-ext-2.discordapp.net"
		) {
		return None;
	}
	if host == "cdn.discordapp.com" {
		url.set_host(Some("media.discordapp.net")).ok()?;
	}
	Some(url)
}

fn application_icon_url(key: &str, bytes: &[u8]) -> Option<String> {
	if bytes.len() > MAX_APPLICATION_METADATA {
		return None;
	}
	let application: Id = key.strip_prefix("app-icon-")?.parse().ok()?;
	let metadata: discord_protocol::presence::ApplicationIcon =
		discord_protocol::decode(bytes).ok()?;
	let icon = metadata.icon?;
	(metadata.id == application && icon.len() == 32 && icon.bytes().all(|b| b.is_ascii_hexdigit()))
		.then(|| format!("https://cdn.discordapp.com/app-icons/{application}/{icon}.png?size=128"))
}

fn disk_key(key: &str) -> Option<String> {
	let url = cdn_url(key)?;
	if key.starts_with("anim:")
		|| key.starts_with("embed:")
		|| key.starts_with("media:")
		|| key.starts_with("gif:")
	{
		Some(format!("embed-{:x}", Sha256::digest(url.as_bytes())))
	} else {
		Some(key.to_owned())
	}
}

/// Cached PNG only; OS alerts never start an image download or block on disk from rendering.
pub(crate) fn notification_image_path(account: Id, key: &str) -> Option<String> {
	let name = disk_key(key)?;
	if name.len() > 160 {
		return None;
	}
	let root = dirs::data_local_dir()?;
	root.join("serein")
		.join("avatars")
		.join(account.to_string())
		.join(format!("{name}.png"))
		.to_str()
		.map(str::to_owned)
}

async fn run(
	root: Option<&Path>,
	mut requests: async_mpsc::Receiver<String>,
	results: ResultSender,
	mut cancelled: watch::Receiver<bool>,
) {
	let mut disk = root.and_then(|root| Disk::open(root.to_owned()).ok());
	let client = reqwest::Client::builder()
		.https_only(true)
		.no_proxy()
		.redirect(reqwest::redirect::Policy::none())
		.timeout(Duration::from_secs(15))
		.connect_timeout(Duration::from_secs(5))
		.pool_max_idle_per_host(1)
		.build()
		.ok();
	let mut cooldown = Instant::now();
	// Eight bounded loads overlap; each downloads and decodes off this loop, which owns the disk.
	let mut jobs = tokio::task::JoinSet::new();
	let (mut viewer, mut inline) = (VecDeque::<String>::new(), VecDeque::<String>::new());
	loop {
		while jobs.len() < JOBS
			&& !*cancelled.borrow()
			&& let Some(key) = viewer.pop_front().or_else(|| inline.pop_front())
		{
			let Some(MediaUrls { primary, fallback }) = job_urls(&key) else {
				continue;
			};
			let mut error = disk.is_none().then_some(CACHE_ERROR);
			let cached = disk.as_mut().and_then(|disk| match disk.read(&key) {
				Ok(bytes) => bytes,
				Err(_) => {
					error = Some(CACHE_ERROR);
					None
				}
			});
			jobs.spawn(load(Job {
				budget: budget(&key),
				key,
				url: primary,
				fallback,
				cached,
				error,
				client: client.clone(),
				cooldown,
				early: results.clone(),
			}));
		}
		let loaded = tokio::select! {
			biased;
			_ = cancelled.changed() => break,
			completed = jobs.join_next(), if !jobs.is_empty() => {
				let Some(Ok(loaded)) = completed else { break };
				loaded
			},
			key = requests.recv(), if viewer.len() + inline.len() < QUEUED => {
				let Some(key) = key else { break };
				if *cancelled.borrow() { break; }
				if Rendition::parse(&key).is_some_and(|rendition| rendition.lane == Lane::Viewer) {
					viewer.push_back(key);
				} else {
					inline.push_back(key);
				}
				continue;
			},
		};
		let Loaded {
			key,
			fetched,
			image,
			frames,
			mut error,
			until,
		} = loaded;
		cooldown = cooldown.max(until);
		if *cancelled.borrow() {
			break;
		}
		if let (Some(disk), Some(bytes)) = (&mut disk, &fetched)
			&& disk.write(&key, bytes).is_err()
		{
			error = Some(CACHE_ERROR);
		}
		// Decoded results can wait for the UI; the encoded source is no longer needed.
		drop(fetched);
		tokio::select! {
			biased;
			_ = cancelled.changed() => break,
			result = results.send(AvatarResult {
			key,
			image,
			frames,
			error,
			stage: DecodeStage::Settled,
			_reservation: None,
		}) => if !result { break },
		}
	}
	jobs.abort_all();
}

fn job_urls(key: &str) -> Option<MediaUrls> {
	match Rendition::parse(key) {
		Some(rendition) => media_urls(&rendition),
		None => cdn_url(key).map(|primary| MediaUrls {
			primary,
			fallback: None,
		}),
	}
}

const JOBS: usize = 8;
struct Job {
	key: String,
	url: String,
	fallback: Option<String>,
	budget: Budget,
	cached: Option<Vec<u8>>,
	error: Option<&'static str>,
	client: Option<reqwest::Client>,
	cooldown: Instant,
	/// Animated sources post their first frame here while the remaining frames decode.
	early: ResultSender,
}
struct Loaded {
	key: String,
	/// Downloaded source that decoded; the worker stores it on disk.
	fetched: Option<Vec<u8>>,
	image: Option<egui::ColorImage>,
	frames: ui::GifFrames,
	error: Option<&'static str>,
	until: Instant,
}

async fn load(job: Job) -> Loaded {
	let Job {
		key,
		url,
		fallback,
		budget,
		cached,
		error,
		client,
		cooldown,
		early,
	} = job;
	let animated = budget.frames.is_some();
	let mut until = cooldown;
	let mut early = animated.then_some(early);
	let mut stale = None;
	if let Some(bytes) = cached {
		let (image, frames, _) = decode_blocking(&key, bytes, false, budget, early.clone()).await;
		// A single stored frame is fetched again: a static rendition may share its cache name.
		if image.is_some() && (!animated || frames.len() >= 2) {
			return Loaded {
				key,
				fetched: None,
				image,
				frames,
				error,
				until,
			};
		}
		if image.is_some() {
			early = None;
		}
		stale = image;
	}
	let (mut image, mut frames, mut fetched) = (None, Vec::new(), None);
	if let Some(client) = &client
		&& Instant::now() >= cooldown
	{
		for url in std::iter::once(url).chain(fallback) {
			let Some(bytes) = fetch(client, &key, url, &mut until).await else {
				continue;
			};
			(image, frames, fetched) =
				decode_blocking(&key, bytes, lottie_key(&key), budget, early.clone()).await;
			if image.is_some() {
				break;
			}
		}
	}
	let fetched = fetched.filter(|_| image.is_some());
	let image = image.or(stale);
	Loaded {
		key,
		fetched,
		image,
		frames,
		error,
		until,
	}
}

async fn fetch(
	client: &reqwest::Client,
	key: &str,
	url: String,
	until: &mut Instant,
) -> Option<Vec<u8>> {
	let url = if key.starts_with("app-icon-") {
		let metadata = download(client, &url, until, MAX_APPLICATION_METADATA).await?;
		application_icon_url(key, &metadata)?
	} else {
		url
	};
	let limit = if lottie_key(key) {
		MAX_LOTTIE_ENCODED
	} else {
		encoded_limit(key)
	};
	download(client, &url, until, limit).await
}

/// Decode on a blocking thread so loads decode in parallel. Returns the stored bytes too:
/// a fetched Lottie source is replaced by its rendered PNG.
async fn decode_blocking(
	key: &str,
	bytes: Vec<u8>,
	lottie: bool,
	budget: Budget,
	early: Option<ResultSender>,
) -> (Option<egui::ColorImage>, ui::GifFrames, Option<Vec<u8>>) {
	let key = key.to_owned();
	tokio::task::spawn_blocking(move || {
		let Some(bytes) = (if lottie {
			render_lottie(&bytes)
		} else {
			Some(bytes)
		}) else {
			return (None, Vec::new(), None);
		};
		let Some(frame_budget) = budget.frames else {
			return (decode(&bytes, &budget), Vec::new(), Some(bytes));
		};
		let post = |image: &egui::ColorImage| {
			if let Some(results) = &early {
				results.preview(AvatarResult {
					key: key.clone(),
					image: Some(image.clone()),
					frames: Vec::new(),
					error: None,
					stage: DecodeStage::Preview,
					_reservation: None,
				});
			}
		};
		let frames = decode_animation(&bytes, &frame_budget, post).unwrap_or_default();
		let image = frames
			.first()
			.map(|(_, image)| image.as_ref().clone())
			.or_else(|| decode(&bytes, &budget));
		(image, frames, Some(bytes))
	})
	.await
	.unwrap_or((None, Vec::new(), None))
}

/// Render one bounded static preview; the resulting PNG is what enters the disk cache.
fn render_lottie(bytes: &[u8]) -> Option<Vec<u8>> {
	if bytes.len() > MAX_LOTTIE_ENCODED {
		return None;
	}
	let source = std::str::from_utf8(bytes).ok()?;
	let animation = Lottie::from_json_str(source).ok()?;
	if animation.width == 0
		|| animation.height == 0
		|| animation.width > 1024
		|| animation.height > 1024
	{
		return None;
	}
	let scale = 160.0 / animation.width.max(animation.height) as f32;
	let frame = Renderer::default()
		.render_frame(
			&animation,
			animation.in_point,
			RenderConfig::new(Rgba8::TRANSPARENT, scale),
		)
		.ok()?;
	let mut pixels = frame.pixels;
	for pixel in pixels.as_chunks_mut::<4>().0 {
		let alpha = u16::from(pixel[3]);
		if alpha == 0 {
			pixel[..3].fill(0);
		} else if alpha < 255 {
			for channel in &mut pixel[..3] {
				*channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
			}
		}
	}
	let mut png = Vec::new();
	image::codecs::png::PngEncoder::new(&mut png)
		.write_image(
			&pixels,
			frame.width,
			frame.height,
			image::ExtendedColorType::Rgba8,
		)
		.ok()?;
	(png.len() <= MAX_ENCODED).then_some(png)
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
			.unwrap_or(60.0);
		// A nonsensical delay suspends this session's CDN requests, rather than retrying early.
		*cooldown = Instant::now()
			.checked_add(Duration::try_from_secs_f64(seconds.max(1.0)).unwrap_or(Duration::MAX))
			.unwrap_or_else(|| Instant::now() + Duration::from_secs(100 * 365 * 24 * 60 * 60));
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

fn decode(bytes: &[u8], budget: &Budget) -> Option<egui::ColorImage> {
	if bytes.len() > budget.encoded {
		return None;
	}
	// Provider previews can be GIF/JPEG/WebP; decode only the first frame, within limits.
	let mut reader = image::ImageReader::new(Cursor::new(bytes))
		.with_guessed_format()
		.ok()?;
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(budget.canvas);
	limits.max_image_height = Some(budget.canvas);
	limits.max_alloc = Some(budget.alloc);
	reader.limits(limits);
	let mut image = reader.decode().ok()?;
	if image.width() > budget.fit || image.height() > budget.fit {
		image = image.resize(
			budget.fit,
			budget.fit,
			image::imageops::FilterType::Lanczos3,
		);
	}
	let image = image.into_rgba8();
	Some(egui::ColorImage::from_rgba_unmultiplied(
		[image.width() as usize, image.height() as usize],
		image.as_raw(),
	))
}

fn resize_to(image: image::RgbaImage, fit: u32) -> image::RgbaImage {
	let (width, height) = ui::fit_edge(image.width(), image.height(), fit);
	if (width, height) == image.dimensions() {
		return image;
	}
	let filter = if image.width().max(image.height()) <= 2 * width.max(height) {
		image::imageops::FilterType::Triangle
	} else {
		image::imageops::FilterType::Lanczos3
	};
	image::imageops::resize(&image, width, height, filter)
}

fn decode_motion_video(
	bytes: &[u8],
	budget: &FrameBudget,
	first: impl Fn(&egui::ColorImage),
) -> Option<ui::GifFrames> {
	let started = Instant::now();
	let mut fit = budget.fit;
	let mut shrinks = budget.shrinks;
	let mut posted = false;
	// One owned copy for the decoder's 'static stream, shared by every shrink pass.
	let bytes: Arc<[u8]> = bytes.into();
	'decode: loop {
		let mut decoder =
			platform::video::Decoder::open(Box::new(Cursor::new(bytes.clone()))).ok()?;
		let duration = decoder.info().duration;
		let mut frames: Vec<(f64, Arc<egui::ColorImage>)> = Vec::new();
		let mut total = 0usize;
		let mut stride = 1usize;
		let mut index = 0usize;
		loop {
			// A partial clip would loop with a visible jump; the embed falls back to its GIF instead.
			if index >= 600 || started.elapsed() > Duration::from_secs(3) {
				return None;
			}
			match decoder.poll_video() {
				Ok(std::task::Poll::Pending) => {
					let _ = decoder.poll_audio();
					std::thread::sleep(Duration::from_millis(2));
				}
				Ok(std::task::Poll::Ready(Some(platform::video::Sample::Video {
					pts,
					width,
					height,
					rgba,
				}))) => {
					let pixels = (width as usize).checked_mul(height as usize)?;
					if width == 0 || height == 0 || rgba.len() != pixels.checked_mul(4)? {
						return None;
					}
					if !index.is_multiple_of(stride) {
						index += 1;
						continue;
					}
					let image = image::RgbaImage::from_raw(width, height, rgba)?;
					let image = resize_to(image, fit);
					let frame_bytes = image.width() as usize * image.height() as usize * 4;
					// accept_frames also keeps one playback texture the size of this frame.
					if total + 2 * frame_bytes > budget.bytes && shrinks > 0 {
						shrinks -= 1;
						fit = ((fit as f32 * 0.707) as u32).max(1);
						posted = false;
						continue 'decode;
					}
					if frames.len() >= budget.count || total + 2 * frame_bytes > budget.bytes {
						frames = frames.chunks(2).map(|pair| pair[0].clone()).collect();
						total = frames.iter().map(|(_, image)| image.pixels.len() * 4).sum();
						stride *= 2;
					}
					let stored = Arc::new(egui::ColorImage::from_rgba_unmultiplied(
						[image.width() as usize, image.height() as usize],
						image.as_raw(),
					));
					if !posted {
						first(&stored);
						posted = true;
					}
					total += frame_bytes;
					frames.push((pts, stored));
					index += 1;
				}
				Ok(std::task::Poll::Ready(Some(_))) => {}
				Ok(std::task::Poll::Ready(None)) => break,
				Err(_) => return None,
			}
		}
		if frames.len() < 2 {
			return None;
		}
		let mut timed = Vec::with_capacity(frames.len());
		for pair in frames.windows(2) {
			timed.push((frame_delay(pair[1].0 - pair[0].0), pair[0].1.clone()));
		}
		let (pts, image) = frames.last()?;
		let tail = if duration.is_finite() && duration > *pts {
			frame_delay(duration - *pts)
		} else {
			timed
				.last()
				.map(|(delay, _)| *delay)
				.unwrap_or_else(|| Duration::from_millis(50))
		};
		timed.push((tail, image.clone()));
		return Some(timed);
	}
}

fn is_isobmff(bytes: &[u8]) -> bool {
	bytes.len() >= 12
		&& matches!(
			&bytes[4..8],
			b"ftyp" | b"moov" | b"mdat" | b"free" | b"skip" | b"wide"
		)
}

fn frame_delay(seconds: f64) -> Duration {
	let millis = if seconds.is_finite() {
		(seconds * 1000.0).round() as u64
	} else {
		50
	};
	Duration::from_millis(millis.clamp(20, 10_000))
}

fn decode_animation(
	bytes: &[u8],
	budget: &FrameBudget,
	first: impl Fn(&egui::ColorImage) + Send,
) -> Option<ui::GifFrames> {
	if bytes.len() > MAX_ANIMATED_ENCODED {
		return None;
	}
	if is_isobmff(bytes) {
		return std::thread::scope(|scope| {
			scope
				.spawn(move || decode_motion_video(bytes, budget, first))
				.join()
				.ok()
				.flatten()
		});
	}
	let started = Instant::now();
	let mut fit = budget.fit;
	let mut shrinks = budget.shrinks;
	let mut posted = false;
	'decode: loop {
		let mut frames: ui::GifFrames = Vec::new();
		let mut total = 0;
		let mut stride = 1;
		for (index, frame) in animation_frames(bytes)?.enumerate() {
			// A partial clip would loop with a visible jump; keep the still first frame instead.
			// The deadline waits for two frames so one slow resize of a short GIF still animates.
			if index == 600 || (frames.len() >= 2 && started.elapsed() > Duration::from_secs(3)) {
				return None;
			}
			let frame = frame.ok()?;
			let (numerator, denominator) = frame.delay().numer_denom_ms();
			let delay = Duration::from_millis(
				(u64::from(numerator) / u64::from(denominator.max(1))).clamp(20, 10_000),
			);
			if !index.is_multiple_of(stride) {
				frames.last_mut()?.0 += delay;
				continue;
			}
			let image = resize_to(frame.into_buffer(), fit);
			let frame_bytes = image.width() as usize * image.height() as usize * 4;
			// accept_frames also keeps one playback texture the size of this frame.
			if total + 2 * frame_bytes > budget.bytes && shrinks > 0 {
				shrinks -= 1;
				fit = ((fit as f32 * 0.707) as u32).max(1);
				posted = false;
				continue 'decode;
			}
			if frames.len() >= budget.count || total + 2 * frame_bytes > budget.bytes {
				frames = frames
					.chunks(2)
					.map(|pair| {
						(
							pair.iter().map(|(delay, _)| *delay).sum(),
							pair[0].1.clone(),
						)
					})
					.collect();
				total = frames.iter().map(|(_, image)| image.pixels.len() * 4).sum();
				stride *= 2;
			}
			let image = Arc::new(egui::ColorImage::from_rgba_unmultiplied(
				[image.width() as usize, image.height() as usize],
				image.as_raw(),
			));
			if !posted {
				first(&image);
				posted = true;
			}
			total += frame_bytes;
			frames.push((delay, image));
		}
		return Some(frames);
	}
}

fn animation_frames(bytes: &[u8]) -> Option<image::Frames<'_>> {
	use image::{AnimationDecoder, ImageDecoder};
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(ANIMATION_CANVAS);
	limits.max_image_height = Some(ANIMATION_CANVAS);
	limits.max_alloc = Some(ANIMATION_ALLOC);
	Some(match image::guess_format(bytes).ok()? {
		image::ImageFormat::Gif => {
			let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
			decoder.set_limits(limits).ok()?;
			decoder.into_frames()
		}
		image::ImageFormat::WebP => {
			let mut decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).ok()?;
			decoder.set_limits(limits).ok()?;
			decoder.into_frames()
		}
		image::ImageFormat::Png => {
			let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes)).ok()?;
			decoder.set_limits(limits).ok()?;
			decoder.apng().ok()?.into_frames()
		}
		_ => return None,
	})
}

struct Disk {
	root: PathBuf,
	bytes: u64,
	files: usize,
	last_prune: Instant,
}
impl Disk {
	fn open(root: PathBuf) -> io::Result<Self> {
		fs::create_dir_all(&root)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
		}
		let mut disk = Self {
			root,
			bytes: 0,
			files: 0,
			last_prune: Instant::now(),
		};
		disk.prune(0, 0)?;
		Ok(disk)
	}
	fn read(&mut self, key: &str) -> io::Result<Option<Vec<u8>>> {
		let name = disk_key(key).ok_or(io::ErrorKind::InvalidInput)?;
		let path = self.root.join(format!("{name}.png"));
		let file = match OpenOptions::new().read(true).write(true).open(&path) {
			Ok(file) => file,
			Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
			Err(error) => return Err(error),
		};
		let metadata = file.metadata()?;
		if metadata.len() > encoded_limit(key) as u64
			|| SystemTime::now()
				.duration_since(metadata.modified()?)
				.unwrap_or_default()
				>= RETENTION
		{
			return Ok(None);
		}
		let mut bytes = Vec::with_capacity(metadata.len() as usize);
		(&file)
			.take(encoded_limit(key) as u64 + 1)
			.read_to_end(&mut bytes)?;
		if bytes.len() > encoded_limit(key) {
			return Ok(None);
		}
		file.set_modified(SystemTime::now())?;
		Ok(Some(bytes))
	}
	fn write(&mut self, key: &str, bytes: &[u8]) -> io::Result<()> {
		if cdn_url(key).is_none() || bytes.len() > encoded_limit(key) {
			return Err(io::ErrorKind::InvalidInput.into());
		}
		let name = disk_key(key).ok_or(io::ErrorKind::InvalidInput)?;
		let path = self.root.join(format!("{name}.png"));
		if self.bytes + bytes.len() as u64 > MAX_DISK
			|| self.files + 1 > MAX_FILES
			|| self.last_prune.elapsed() >= Duration::from_secs(24 * 60 * 60)
		{
			self.prune(bytes.len() as u64, 1)?;
		}
		let previous = fs::metadata(&path).ok().map(|m| m.len());
		static SEQUENCE: AtomicU64 = AtomicU64::new(0);
		let temp = self.root.join(format!(
			"{}.{}.part",
			std::process::id(),
			SEQUENCE.fetch_add(1, Ordering::Relaxed)
		));
		let result = (|| {
			let mut options = OpenOptions::new();
			options.write(true).create_new(true);
			#[cfg(unix)]
			{
				use std::os::unix::fs::OpenOptionsExt;
				options.mode(0o600);
			}
			let mut file = options.open(&temp)?;
			file.write_all(bytes)?;
			drop(file);
			#[cfg(windows)]
			if path.exists() {
				fs::remove_file(&path)?;
			}
			fs::rename(&temp, &path)
		})();
		if result.is_err() {
			let _ = fs::remove_file(&temp);
		}
		result?;
		self.bytes = self.bytes.saturating_sub(previous.unwrap_or(0)) + bytes.len() as u64;
		self.files += usize::from(previous.is_none());
		Ok(())
	}
	fn prune(&mut self, reserve_bytes: u64, reserve_files: usize) -> io::Result<()> {
		// Keep only 32 eviction candidates in RAM even if a pre-existing directory is enormous.
		loop {
			let mut oldest = BinaryHeap::new();
			self.bytes = 0;
			self.files = 0;
			for entry in fs::read_dir(&self.root)? {
				let entry = entry?;
				let metadata = entry.metadata()?;
				if !metadata.is_file() {
					continue;
				}
				let modified = metadata.modified()?;
				if entry
					.path()
					.extension()
					.is_some_and(|extension| extension == "part")
					|| SystemTime::now()
						.duration_since(modified)
						.unwrap_or_default()
						>= RETENTION
				{
					fs::remove_file(entry.path())?;
					continue;
				}
				self.bytes = self.bytes.saturating_add(metadata.len());
				self.files += 1;
				oldest.push((modified, entry.path(), metadata.len()));
				if oldest.len() > 32 {
					oldest.pop();
				}
			}
			if self.bytes + reserve_bytes <= MAX_DISK && self.files + reserve_files <= MAX_FILES {
				break;
			}
			for (_, path, bytes) in oldest.into_sorted_vec() {
				fs::remove_file(path)?;
				self.bytes = self.bytes.saturating_sub(bytes);
				self.files -= 1;
				if self.bytes + reserve_bytes <= MAX_DISK && self.files + reserve_files <= MAX_FILES
				{
					break;
				}
			}
		}
		self.last_prune = Instant::now();
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	fn queued_image(edge: usize) -> AvatarResult {
		AvatarResult {
			key: "synthetic".into(),
			image: Some(egui::ColorImage::filled([edge, edge], egui::Color32::RED)),
			frames: Vec::new(),
			error: None,
			stage: DecodeStage::Settled,
			_reservation: None,
		}
	}

	#[tokio::test]
	async fn decoded_result_budget_covers_consumption_and_cancellation() {
		use std::future::Future;
		let (sender, mut receive) = result_channel(egui::Context::default());
		let charge = queued_image(2).bytes();
		let reserved = sender
			.bytes
			.clone()
			.acquire_many_owned((RESULT_BYTES - charge) as u32)
			.await
			.unwrap();
		assert!(sender.send(queued_image(2)).await);
		let held = receive.try_recv().unwrap();
		// Dequeueing alone does not free pixels still being handed to the UI.
		assert_eq!(sender.bytes.available_permits(), 0);
		assert!(!sender.preview(queued_image(2)));
		{
			let mut pending = std::pin::pin!(sender.send(queued_image(2)));
			assert!(
				pending
					.as_mut()
					.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
					.is_pending()
			);
			drop(held);
			assert!(pending.await);
		}
		let held = receive.try_recv().unwrap();
		{
			let mut cancelled = std::pin::pin!(sender.send(queued_image(2)));
			assert!(
				cancelled
					.as_mut()
					.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
					.is_pending()
			);
		}
		// Cancelling a waiting send leaks no reservation; a disconnected UI also wakes it.
		assert_eq!(sender.bytes.available_permits(), 0);
		let mut closed = std::pin::pin!(sender.send(queued_image(2)));
		assert!(
			closed
				.as_mut()
				.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
				.is_pending()
		);
		drop(receive);
		assert!(!closed.await);
		drop(held);
		drop(reserved);
		assert_eq!(sender.bytes.available_permits(), RESULT_BYTES);

		// Frames must consume the byte budget even without a separate still image.
		let (sender, mut receive) = result_channel(egui::Context::default());
		let frame = Arc::new(egui::ColorImage::filled([1024, 1024], egui::Color32::RED));
		let animated = || {
			let mut result = queued_image(0);
			result.image = None;
			result
				.frames
				.push((Duration::from_millis(100), frame.clone()));
			result
		};
		let reserved = sender
			.bytes
			.clone()
			.acquire_many_owned((RESULT_BYTES - 2 * 1024 * 1024) as u32)
			.await
			.unwrap();
		assert!(!sender.preview(animated()));
		let mut pending = std::pin::pin!(sender.send(animated()));
		assert!(
			pending
				.as_mut()
				.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
				.is_pending()
		);
		drop(reserved);
		assert!(pending.await);
		assert_eq!(receive.try_recv().unwrap().frames.len(), 1);
		assert_eq!(sender.bytes.available_permits(), RESULT_BYTES);
	}

	#[tokio::test]
	async fn decoded_results_reject_oversize_and_repaint_remaining_work() {
		let ctx = egui::Context::default();
		let (sender, results) = result_channel(ctx.clone());
		let (requests, _) = async_mpsc::channel(1);
		let (cancel, _) = watch::channel(false);
		let mut worker = AvatarWorker {
			requests,
			results,
			wake: ctx.clone(),
			cancel,
			clear: Arc::new(AtomicBool::new(false)),
			cleanup: None,
		};
		let mut oversized = queued_image(1);
		oversized
			.image
			.as_mut()
			.unwrap()
			.pixels
			.reserve(RESULT_BYTES / 4);
		assert!(sender.send(oversized).await);
		assert!(sender.send(queued_image(2)).await);
		for _ in 0..3 {
			let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
			output.textures_delta.clear();
		}
		assert!(!ctx.has_requested_repaint());
		let result = worker.poll().unwrap();
		assert!(result.image.is_none());
		assert!(result.frames.is_empty());
		assert!(result.error.is_some());
		assert!(matches!(result.stage, DecodeStage::Settled));
		assert!(ctx.has_requested_repaint());
		drop(result);
		drop(worker.poll().unwrap());
		assert_eq!(sender.bytes.available_permits(), RESULT_BYTES);
		// Item pressure also drops optional previews and returns their byte reservation.
		for _ in 0..128 {
			assert!(sender.preview(queued_image(1)));
		}
		let available = sender.bytes.available_permits();
		assert!(!sender.preview(queued_image(1)));
		assert_eq!(sender.bytes.available_permits(), available);
		drop(worker);
		assert_eq!(sender.bytes.available_permits(), RESULT_BYTES);
	}

	#[tokio::test]
	#[ignore = "synthetic paused-consumer memory workload; allocates up to 512 MiB"]
	async fn decoded_result_queue_workload() {
		use std::future::Future;
		// Explicit comparator reproduces the old item-only admission using the same results.
		let legacy = std::env::var_os("SEREIN_IMAGE_QUEUE_LEGACY").is_some();
		let (sender, mut receive) = result_channel(egui::Context::default());
		for _ in 0..128 {
			let result = queued_image(1024);
			if legacy {
				assert!(sender.send.send(result).await.is_ok());
			} else {
				let mut send = std::pin::pin!(sender.send(result));
				match send
					.as_mut()
					.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
				{
					std::task::Poll::Ready(true) => {}
					std::task::Poll::Pending => break,
					std::task::Poll::Ready(false) => panic!("synthetic result rejected"),
				}
			}
		}
		let items = receive.len();
		let mut pixels = 0;
		let mut charged = 0;
		while let Ok(result) = receive.try_recv() {
			pixels += result.image.as_ref().unwrap().pixels.capacity() * 4;
			charged += result.bytes();
		}
		eprintln!(
			"image_queue legacy={legacy} items={items} pixel_bytes={pixels} estimated_bytes={charged}"
		);
		assert_eq!(items, if legacy { 128 } else { 31 });
		assert!(legacy || charged <= RESULT_BYTES);
	}

	#[test]
	fn apng_sticker_frames_preserve_pixels_and_delays() {
		// Synthetic 1x1 APNG: opaque red for 100 ms, then opaque green for 200 ms.
		let bytes = [
			137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
			8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 8, 97, 99, 84, 76, 0, 0, 0, 2, 0, 0, 0, 0,
			243, 141, 147, 112, 0, 0, 0, 26, 102, 99, 84, 76, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1,
			0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 10, 0, 0, 90, 127, 48, 208, 0, 0, 0, 13, 73, 68, 65,
			84, 120, 156, 99, 248, 207, 192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0,
			26, 102, 99, 84, 76, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
			0, 5, 0, 0, 202, 80, 157, 57, 0, 0, 0, 17, 102, 100, 65, 84, 0, 0, 0, 2, 120, 156, 99,
			96, 248, 207, 240, 31, 0, 4, 1, 1, 255, 98, 231, 233, 156, 0, 0, 0, 0, 73, 69, 78, 68,
			174, 66, 96, 130,
		];
		let frames = super::decode_animation(&bytes, &FrameBudget::legacy(160), |_| {}).unwrap();
		assert_eq!(frames.len(), 2);
		assert_eq!(frames[0].0, std::time::Duration::from_millis(100));
		assert_eq!(frames[1].0, std::time::Duration::from_millis(200));
		assert_eq!(frames[0].1.size, [1, 1]);
		assert_eq!(frames[0].1.pixels[0], eframe::egui::Color32::RED);
		assert_eq!(frames[1].1.pixels[0], eframe::egui::Color32::GREEN);
		assert!(
			super::decode_animation(&bytes[..100], &FrameBudget::legacy(160), |_| {}).is_none()
		);
	}
	#[test]
	fn sticker_urls_and_decode_budgets_are_scoped() {
		for prefix in ["embed", "anim"] {
			for format in [1, 2, 4] {
				let key = format!("{prefix}:sticker-7-{format}");
				let expected = if format == 4 {
					"https://media.discordapp.net/stickers/7.gif"
				} else {
					"https://cdn.discordapp.com/stickers/7.png"
				};
				assert_eq!(super::cdn_url(&key).as_deref(), Some(expected));
				assert_eq!(super::decode_edge(&key), ui::EMBED_EDGE);
				assert!(super::disk_key(&key).unwrap().starts_with("embed-"));
			}
		}
		assert_eq!(
			super::cdn_url("embed:sticker-7-3").as_deref(),
			Some("https://cdn.discordapp.com/stickers/7.json")
		);
		for key in [
			"anim:sticker-7-3",
			"embed:sticker-0-1",
			"embed:sticker-7-5",
			"embed:sticker-7-1?x=1",
			"embed:sticker-../7-1",
			"embed:sticker-https://example.com-1",
		] {
			assert!(super::cdn_url(key).is_none(), "{key}");
		}
		assert!(
			super::decode_animation(
				&vec![0; super::MAX_ANIMATED_ENCODED + 1],
				&FrameBudget::legacy(160),
				|_| {}
			)
			.is_none()
		);
	}

	#[test]
	fn lottie_sticker_renders_to_a_bounded_cached_png() {
		let source = br#"{"v":"5.7.6","fr":30,"ip":0,"op":30,"w":320,"h":320,"layers":[{"ty":4,"ip":0,"op":30,"st":0,"ks":{"o":{"a":0,"k":100},"r":{"a":0,"k":0},"p":{"a":0,"k":[160,160]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]}},"shapes":[{"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[200,200]}},{"ty":"fl","c":{"a":0,"k":[0.2,0.6,1,1]},"o":{"a":0,"k":100},"r":1}]}]}"#;
		let png = super::render_lottie(source).expect("supported Lottie preview");
		assert!(png.len() <= super::MAX_ENCODED);
		assert_eq!(
			super::decode(&png, &Budget::legacy(ui::EMBED_EDGE))
				.unwrap()
				.size,
			[160, 160]
		);
		assert!(super::render_lottie(&vec![b' '; super::MAX_LOTTIE_ENCODED + 1]).is_none());
	}
	#[test]
	fn role_icon_urls_are_confined_to_the_role_cdn_path() {
		assert_eq!(
			super::cdn_url("role-icon-7-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").as_deref(),
			Some(
				"https://cdn.discordapp.com/role-icons/7/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
			)
		);
		for key in [
			"role-icon-0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			"role-icon-7-../private",
			"role-icon-7-a.png?token=secret",
			"role-icon-7-https://example.com",
		] {
			assert!(super::cdn_url(key).is_none());
		}
	}
	#[test]
	fn application_and_group_icon_urls_accept_only_ids_and_hashes() {
		for hash in [
			"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			"a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		] {
			assert_eq!(
				super::cdn_url(&format!("application-icon-7-{hash}")),
				Some(format!(
					"https://cdn.discordapp.com/app-icons/7/{hash}.png?size=128"
				))
			);
		}
		assert_eq!(
			super::cdn_url("group-icon-7-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").as_deref(),
			Some(
				"https://cdn.discordapp.com/channel-icons/7/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
			)
		);
		for key in [
			"application-icon-0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			"application-icon-7-../private",
			"application-icon-7-a.png?token=secret",
			"application-icon-7-https://example.com",
			"application-icon-7-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			"group-icon-0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
			"group-icon-7-../private",
			"group-icon-7-a.png?token=secret",
			"group-icon-7-https://example.com",
			"group-icon-7-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
		] {
			assert!(super::cdn_url(key).is_none());
		}
	}
	#[test]
	fn gif_animation_decodes_full_size_and_partial_frames() {
		let mut bytes = Vec::new();
		{
			let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
			for (width, color) in [(2048, [255, 0, 0, 255]), (2047, [0, 255, 0, 255])] {
				encoder
					.encode_frame(image::Frame::from_parts(
						image::RgbaImage::from_pixel(width, 2048, image::Rgba(color)),
						0,
						0,
						image::Delay::from_numer_denom_ms(100, 1),
					))
					.unwrap();
			}
		}
		let frames = super::decode_animation(&bytes, &FrameBudget::legacy(128), |_| {}).unwrap();
		assert_eq!(frames.len(), 2);
		assert!(frames.iter().all(|(delay, image)| {
			*delay == Duration::from_millis(100) && image.size == [128, 128]
		}));
		assert_ne!(frames[0].1.pixels[0], frames[1].1.pixels[0]);
	}

	#[test]
	fn gif_animation_preserves_long_loop_with_bounded_frames() {
		let mut bytes = Vec::new();
		{
			let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
			for index in 0..100 {
				encoder
					.encode_frame(image::Frame::from_parts(
						// Exercise temporal compaction without spending the decode deadline resizing 100 frames.
						image::RgbaImage::from_pixel(16, 16, image::Rgba([index, 0, 255, 255])),
						0,
						0,
						image::Delay::from_numer_denom_ms(100, 1),
					))
					.unwrap();
			}
		}
		let frames = super::decode_animation(&bytes, &FrameBudget::legacy(160), |_| {}).unwrap();
		assert!(frames.len() > 1 && frames.len() <= 80);
		assert_eq!(
			frames
				.iter()
				.map(|(delay, _)| *delay)
				.sum::<std::time::Duration>(),
			std::time::Duration::from_secs(10)
		);
		assert!(
			frames
				.iter()
				.map(|(_, image)| image.pixels.len() * 4)
				.sum::<usize>()
				<= 8 * 1024 * 1024
		);
		assert!(super::cdn_url("anim:https://media.tenor.com/x/tenor.gif").is_some());
		assert!(super::cdn_url("anim:https://static.klipy.com/x.gif").is_some());
		assert!(super::cdn_url("anim:https://evil.example/x.gif").is_none());
	}

	#[test]
	fn gif_frames_decode_with_timing_and_reject_invalid_data() {
		let mut bytes = Vec::new();
		{
			let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
			for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
				encoder
					.encode_frame(image::Frame::from_parts(
						image::RgbaImage::from_pixel(320, 160, image::Rgba(color)),
						0,
						0,
						image::Delay::from_numer_denom_ms(100, 1),
					))
					.unwrap();
			}
		}
		let frames = super::decode_animation(&bytes, &FrameBudget::legacy(160), |_| {}).unwrap();
		assert_eq!(frames.len(), 2);
		assert_eq!(frames[0].0, std::time::Duration::from_millis(100));
		assert_eq!(frames[0].1.size, [160, 80]);
		assert_ne!(frames[0].1.pixels[0], frames[1].1.pixels[0]);
		assert!(super::decode_animation(b"not a GIF", &FrameBudget::legacy(160), |_| {}).is_none());
		assert!(
			super::decode_animation(
				&vec![0; super::MAX_ENCODED + 1],
				&FrameBudget::legacy(160),
				|_| {}
			)
			.is_none()
		);
	}

	#[test]
	fn animated_banner_and_avatar_urls_and_keys() {
		assert!(super::is_animated_key(
			"banner-123-a_abcdef0123456789abcdef0123456789"
		));
		assert!(super::is_animated_key(
			"member-banner-999-123-a_abcdef0123456789abcdef0123456789"
		));
		assert!(super::is_animated_key(
			"member-avatar-999-123-a_abcdef0123456789abcdef0123456789"
		));
		assert!(super::is_animated_key(
			"123-a_abcdef0123456789abcdef0123456789"
		));
		assert!(!super::is_animated_key(
			"banner-123-abcdef0123456789abcdef0123456789"
		));
		assert!(!super::is_animated_key(
			"member-banner-999-123-abcdef0123456789abcdef0123456789"
		));
		assert!(!super::is_animated_key(
			"member-avatar-999-123-abcdef0123456789abcdef0123456789"
		));
		assert!(!super::is_animated_key(
			"123-abcdef0123456789abcdef0123456789"
		));

		assert_eq!(
			super::cdn_url("banner-123-a_abcdef0123456789abcdef0123456789").as_deref(),
			Some(
				"https://cdn.discordapp.com/banners/123/a_abcdef0123456789abcdef0123456789.gif?size=512"
			)
		);
		assert_eq!(
			super::cdn_url("member-banner-999-123-a_abcdef0123456789abcdef0123456789").as_deref(),
			Some(
				"https://cdn.discordapp.com/guilds/999/users/123/banners/a_abcdef0123456789abcdef0123456789.gif?size=512"
			)
		);
		assert_eq!(
			super::cdn_url("123-a_abcdef0123456789abcdef0123456789").as_deref(),
			Some(
				"https://cdn.discordapp.com/avatars/123/a_abcdef0123456789abcdef0123456789.gif?size=128"
			)
		);
		assert_eq!(
			super::cdn_url("123-abcdef0123456789abcdef0123456789").as_deref(),
			Some(
				"https://cdn.discordapp.com/avatars/123/abcdef0123456789abcdef0123456789.png?size=128"
			)
		);
	}
	#[test]
	fn activity_artwork_urls_and_application_metadata_are_scoped() {
		assert_eq!(
			super::cdn_url("activity-7-8").as_deref(),
			Some("https://cdn.discordapp.com/app-assets/7/8.png?size=128")
		);
		assert_eq!(
			super::cdn_url("app-icon-7").as_deref(),
			Some("https://discord.com/api/v10/applications/7/rpc")
		);
		for key in [
			"activity-0-8",
			"activity-7-0",
			"activity-7-../8",
			"activity-7-8?size=8192",
			"activity-7-https://example.com",
			"app-icon-0",
			"app-icon-7/rpc",
			"app-icon-7?token=secret",
		] {
			assert!(super::cdn_url(key).is_none());
		}
		assert_eq!(
			super::application_icon_url(
				"app-icon-7",
				br#"{"id":"7","icon":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","name":"Ignored"}"#
			)
			.as_deref(),
			Some(
				"https://cdn.discordapp.com/app-icons/7/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
			)
		);
		for bytes in [
			br#"{"id":"8","icon":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#.as_slice(),
			br#"{"id":"7","icon":null}"#,
			br#"{"id":"7"}"#,
			br#"{"id":"7","icon":"../../private"}"#,
			br#"{"id":"7","icon":"https://example.com/icon.png"}"#,
			br#"{"id":"7","icon":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"}"#,
		] {
			assert!(super::application_icon_url("app-icon-7", bytes).is_none());
		}
		assert!(
			super::application_icon_url(
				"app-icon-7",
				&vec![b' '; super::MAX_APPLICATION_METADATA + 1]
			)
			.is_none()
		);
	}

	#[test]
	fn custom_emoji_urls_are_static_and_confined_to_discord_cdn() {
		assert_eq!(
			super::cdn_url("emoji-9001").as_deref(),
			Some("https://cdn.discordapp.com/emojis/9001.png?size=64")
		);
		for key in [
			"emoji-0",
			"emoji-../9001",
			"emoji-9001?size=8192",
			"emoji-https://example.com",
			"emoji-9001/foo",
		] {
			assert!(super::cdn_url(key).is_none());
		}
	}

	use super::*;
	use tokio::io::{AsyncReadExt, AsyncWriteExt};

	fn png(width: u32, height: u32) -> Vec<u8> {
		let image = image::RgbaImage::from_pixel(width, height, image::Rgba([70, 150, 120, 255]));
		let mut encoded = Cursor::new(Vec::new());
		image
			.write_to(&mut encoded, image::ImageFormat::Png)
			.unwrap();
		encoded.into_inner()
	}
	#[test]
	fn bounded_images_cache_reopen_eviction_and_cancelled_cleanup() {
		assert!(cdn_url("../token").is_none());
		assert_eq!(
			cdn_url("banner-1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
			"https://cdn.discordapp.com/banners/1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=512"
		);
		assert_eq!(
			cdn_url("member-banner-2-1-a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
			"https://cdn.discordapp.com/guilds/2/users/1/banners/a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.gif?size=512"
		);
		assert_eq!(
			cdn_url("member-avatar-2-1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
			"https://cdn.discordapp.com/guilds/2/users/1/avatars/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
		);
		assert!(cdn_url("member-banner-2-0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
		assert_eq!(
			cdn_url("badge-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
			"https://cdn.discordapp.com/badge-icons/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=64"
		);
		assert_eq!(
			cdn_url("clan-7-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
			"https://cdn.discordapp.com/clan-badges/7/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=64"
		);
		assert!(cdn_url("badge-../evil").is_none());
		assert!(cdn_url("clan-0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
		assert!(cdn_url("banner-1-../../invalid").is_none());
		assert!(cdn_url("default-6").is_none());
		assert!(cdn_url("0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
		assert!(cdn_url("1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/").is_none());
		assert_eq!(
			cdn_url("1-a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
			"https://cdn.discordapp.com/avatars/1/a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.gif?size=128"
		);
		assert_eq!(
			cdn_url("1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
			"https://cdn.discordapp.com/avatars/1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
		);
		assert_eq!(
			cdn_url("guild-1-a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
			"https://cdn.discordapp.com/icons/1/a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
		);
		for source in [
			"http://media.discordapp.net/attachments/1/2/image.png",
			"https://media.discordapp.net.evil.test/attachments/1/2/image.png",
			"https://user@media.discordapp.net/attachments/1/2/image.png",
			"https://media.discordapp.net:444/attachments/1/2/image.png",
			"https://127.0.0.1/attachments/1/2/image.png",
			"https://media.discordapp.net/attachments/1/2/image.png#fragment",
			"https://cdn.discordapp.com/api/v10/users/@me",
			"https://media.discordapp.net/attachments/../api",
			"https://images-ext-1.discordapp.net/external/short/https/example.com/a.png",
		] {
			assert!(
				embed_url(source, ui::EMBED_EDGE).is_none(),
				"Unsafe or unsupported test URL was accepted"
			);
		}
		assert!(embed_url(
			"https://cdn.discordapp.com/guilds/1/users/2/avatars/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=2048",
			2048,
		).is_some());
		assert!(
			embed_url(
				"https://cdn.discordapp.com/guilds/1/users/2/avatars/invalid.png",
				2048,
			)
			.is_none()
		);
		let embed_key = "embed:https://cdn.discordapp.com/attachments/1/2/image.png?ex=abc&is=def&hm=synthetic&format=webp&width=4096&height=1024&fit=cover";
		let transformed = cdn_url(embed_key).unwrap();
		assert!(transformed.starts_with("https://media.discordapp.net/attachments/1/2/image.png?"));
		assert!(transformed.ends_with("format=png&width=512&height=128"));
		assert!(transformed.contains("hm=synthetic"));
		assert!(!transformed.contains("4096"));
		assert!(!transformed.contains("fit=cover"));
		assert_ne!(
			disk_key(embed_key).unwrap(),
			format!("embed-{:x}", Sha256::digest(embed_key.as_bytes()))
		);
		assert_eq!(disk_key(embed_key).unwrap().len(), 70);
		assert!(
			disk_key(embed_key)
				.unwrap()
				.bytes()
				.all(|b| b.is_ascii_hexdigit() || matches!(b, b'm' | b'-'))
		);
		assert!(
			embed_url(
				"https://images-ext-1.discordapp.net/external/abcdefghijklmnop/https/example.com/image.jpg",
				ui::EMBED_EDGE,
			)
			.is_some()
		);
		let media_key = "media:vs:2048x512:https://cdn.discordapp.com/attachments/1/2/image.png?ex=abc&is=def&hm=synthetic";
		let transformed = cdn_url(media_key).unwrap();
		assert!(transformed.starts_with("https://media.discordapp.net/attachments/1/2/image.png?"));
		assert!(transformed.ends_with("format=webp&quality=lossless&width=2048&height=512"));
		assert_ne!(disk_key(media_key).unwrap(), disk_key(embed_key).unwrap());
		let legacy = Budget::legacy;
		assert!(decode(&png(1025, 1), &legacy(512)).is_none());
		assert_eq!(
			decode(&png(1024, 512), &legacy(512)).unwrap().size,
			[512, 256]
		);
		let media = budget("media:vs:2048x1024:https://cdn.discordapp.com/attachments/1/2/a.png");
		assert_eq!(decode(&png(1024, 512), &media).unwrap().size, [1024, 512]);
		assert!(decode(&png(4097, 1), &media).is_none());
		assert!(decode(&vec![0; MAX_ENCODED + 1], &legacy(128)).is_none());
		assert!(decode(b"not an image", &legacy(128)).is_none());
		assert!(decode(&png(257, 1), &legacy(128)).is_none());
		let bytes = png(256, 256);
		assert_eq!(decode(&bytes, &legacy(128)).unwrap().size, [64, 64]);
		let root = std::env::temp_dir().join(format!(
			"serein-avatar-test-{}-{}",
			std::process::id(),
			SystemTime::now()
				.duration_since(SystemTime::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		let account_a = root.join("1");
		let account_b = root.join("2");
		let mut disk = Disk::open(account_a.clone()).unwrap();
		disk.write("default-0", &bytes).unwrap();
		disk.write(embed_key, &bytes).unwrap();
		disk.write("embed:sticker-7-3", &bytes).unwrap();
		disk.write("app-icon-7", &bytes).unwrap();
		assert!(fs::read_dir(&account_a).unwrap().all(|entry| {
			!entry
				.unwrap()
				.file_name()
				.to_string_lossy()
				.contains("synthetic")
		}));
		drop(disk);
		let mut disk = Disk::open(account_a.clone()).unwrap();
		assert_eq!(disk.read("default-0").unwrap().unwrap(), bytes);
		assert_eq!(disk.read(embed_key).unwrap().unwrap(), bytes);
		assert_eq!(disk.read("embed:sticker-7-3").unwrap().unwrap(), bytes);
		assert_eq!(disk.read("app-icon-7").unwrap().unwrap(), bytes);
		assert!(
			Disk::open(account_b.clone())
				.unwrap()
				.read("default-0")
				.unwrap()
				.is_none()
		);
		let sparse = OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(account_b.join("oversized.png"))
			.unwrap();
		sparse.set_len(MAX_DISK + 1).unwrap();
		drop(sparse);
		assert_eq!(Disk::open(account_b.clone()).unwrap().bytes, 0);
		let old = account_a.join("default-0.png");
		OpenOptions::new()
			.write(true)
			.open(old)
			.unwrap()
			.set_modified(SystemTime::now() - RETENTION - Duration::from_secs(1))
			.unwrap();
		disk.prune(0, 0).unwrap();
		assert!(disk.read("default-0").unwrap().is_none());
		fs::remove_file(account_a.join(format!("{}.png", disk_key(embed_key).unwrap()))).unwrap();
		fs::remove_file(account_a.join(format!("{}.png", disk_key("embed:sticker-7-3").unwrap())))
			.unwrap();
		fs::remove_file(account_a.join("app-icon-7.png")).unwrap();
		drop(disk);
		// Eviction and full directory deletion are disk workloads, not a worker-cancellation
		// deadline: deleting 4096 files can exceed five seconds on a Windows CI filesystem.
		let eviction = root.join("eviction");
		let mut disk = Disk::open(eviction.clone()).unwrap();
		for index in 0..MAX_FILES + 1 {
			fs::write(eviction.join(format!("synthetic-{index}.png")), []).unwrap();
		}
		disk.prune(0, 0).unwrap();
		assert_eq!(disk.files, MAX_FILES);
		disk.write("default-0", &bytes).unwrap();
		assert_eq!(disk.files, MAX_FILES);
		drop(disk);
		clear_directory(Some(&eviction)).unwrap();
		assert!(!eviction.exists());
		// A valid cached image keeps the shutdown fixture entirely offline and tiny.
		let mut disk = Disk::open(account_a.clone()).unwrap();
		disk.write("default-0", &bytes).unwrap();
		assert_eq!(disk.files, 1);
		drop(disk);
		let runtime = tokio::runtime::Runtime::new().unwrap();
		let mut worker =
			AvatarWorker::start_at(&runtime, Some(account_a.clone()), egui::Context::default())
				.unwrap();
		assert!(worker.request("default-0".into()));
		let result = runtime.block_on(async {
			tokio::time::timeout(Duration::from_secs(5), worker.results.recv())
				.await
				.unwrap()
				.unwrap()
		});
		assert_eq!(result.image.unwrap().size, [64, 64]);
		assert!(result.error.is_none());
		// Fill the result channel, then cancel: shutdown must not wait on the renderer.
		for _ in 0..32 {
			assert!(worker.request("default-0".into()));
		}
		runtime.block_on(async {
			tokio::time::timeout(Duration::from_secs(5), async {
				while worker.results.len() < 2 {
					tokio::time::sleep(Duration::from_millis(1)).await;
				}
			})
			.await
			.unwrap();
		});
		worker
			.shutdown_and_clear()
			.recv_timeout(Duration::from_secs(5))
			.unwrap()
			.unwrap();
		assert!(!account_a.exists());
		assert!(account_b.exists());
		fs::remove_dir_all(root).unwrap();
	}

	#[tokio::test]
	async fn synthetic_download_limits_redirects_and_retry_delay() {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let address = listener.local_addr().unwrap();
		let bytes = png(2, 2);
		let expected = bytes.clone();
		let server = tokio::spawn(async move {
			for response in [
                [format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).into_bytes(), bytes].concat(),
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_ENCODED + 1).into_bytes(),
                [b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec(), format!("{:x}\r\n", MAX_ENCODED + 1).into_bytes(), vec![0; MAX_ENCODED + 1], b"\r\n0\r\n\r\n".to_vec()].concat(),
                b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                {
                    let metadata = br#"{"id":"7","icon":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
                    [format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", metadata.len()).into_bytes(), metadata.to_vec()].concat()
                },
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_APPLICATION_METADATA + 1).into_bytes(),
                [b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec(), format!("{:x}\r\n", MAX_APPLICATION_METADATA + 1).into_bytes(), vec![0; MAX_APPLICATION_METADATA + 1], b"\r\n0\r\n\r\n".to_vec()].concat(),
                b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 123.5\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let length = stream.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..length]).to_ascii_lowercase();
                assert!(!request.contains("authorization:"));
                assert!(!request.contains("cookie:"));
                let _ = stream.write_all(&response).await;
            }
		});
		// Only this private test seam accepts HTTP; production constructs fixed HTTPS CDN URLs.
		let client = reqwest::Client::builder()
			.no_proxy()
			.redirect(reqwest::redirect::Policy::none())
			.timeout(Duration::from_secs(2))
			.build()
			.unwrap();
		let mut cooldown = Instant::now();
		let url = format!("http://{address}/synthetic.png");
		assert_eq!(
			download(&client, &url, &mut cooldown, MAX_ENCODED)
				.await
				.unwrap(),
			expected
		);
		assert!(
			download(&client, &url, &mut cooldown, MAX_ENCODED)
				.await
				.is_none()
		);
		assert!(
			download(&client, &url, &mut cooldown, MAX_ENCODED)
				.await
				.is_none()
		);
		let metadata_url = format!("http://{address}/applications/7/rpc");
		let limit = MAX_APPLICATION_METADATA;
		// The metadata path also refuses redirects.
		assert!(
			download(&client, &metadata_url, &mut cooldown, limit)
				.await
				.is_none()
		);
		let metadata = download(&client, &metadata_url, &mut cooldown, limit)
			.await
			.unwrap();
		assert_eq!(
			application_icon_url("app-icon-7", &metadata).as_deref(),
			Some(
				"https://cdn.discordapp.com/app-icons/7/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
			)
		);
		// Oversized declared length, oversized streamed body, then a rate limit.
		for _ in 0..3 {
			assert!(
				download(&client, &metadata_url, &mut cooldown, limit)
					.await
					.is_none()
			);
		}
		assert!(cooldown.duration_since(Instant::now()) > Duration::from_secs(120));
		server.await.unwrap();
		let retry_at = cooldown;
		assert!(
			download(&client, &metadata_url, &mut cooldown, limit)
				.await
				.is_none()
		);
		assert_eq!(cooldown, retry_at);
	}
}
