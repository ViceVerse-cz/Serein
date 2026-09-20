//! Credential-free, viewport-driven static avatars. No tokens enter this worker.
use eframe::egui;
use model::Id;
use sha2::{Digest, Sha256};
use std::{
	collections::BinaryHeap,
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
use tokio::sync::{mpsc as async_mpsc, watch};

const MAX_ENCODED: usize = 2 * 1024 * 1024;
const MAX_ANIMATED_ENCODED: usize = 8 * 1024 * 1024;
const MAX_LARGE_ENCODED: usize = 16 * 1024 * 1024;
fn encoded_limit(key: &str) -> usize {
	if key.starts_with("anim:") {
		MAX_ANIMATED_ENCODED
	} else if key.starts_with("large:") {
		MAX_LARGE_ENCODED
	} else {
		MAX_ENCODED
	}
}
/// Decode budget for one key: the longest edge kept in memory.
fn decode_edge(key: &str) -> u32 {
	if key.starts_with("large:") {
		ui::LARGE_EDGE
	} else if key.starts_with("anim:")
		|| key.starts_with("embed:")
		|| key.starts_with("gif:")
		|| key.starts_with("banner-")
		|| key.starts_with("member-banner-")
	{
		ui::EMBED_EDGE
	} else {
		128
	}
}
const MAX_AVATAR_ENCODED: usize = 512 * 1024;
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
}

pub struct AvatarWorker {
	requests: async_mpsc::Sender<String>,
	results: async_mpsc::Receiver<AvatarResult>,
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
		let (requests, receive) = async_mpsc::channel(128);
		let (send, results) = async_mpsc::channel(2);
		let (cancel, cancelled) = watch::channel(false);
		let clear = Arc::new(AtomicBool::new(false));
		let cleanup_flag = clear.clone();
		let (done, cleanup) = mpsc::sync_channel(1);
		let handle = runtime.handle().clone();
		std::thread::Builder::new()
			.name("avatar-cache".into())
			.spawn(move || {
				handle.block_on(run(root.as_deref(), receive, send, cancelled, &ctx));
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
			cancel,
			clear,
			cleanup: Some(cleanup),
		})
	}
	pub fn request(&self, key: String) -> bool {
		cdn_url(&key).is_some() && self.requests.try_send(key).is_ok()
	}
	pub fn poll(&mut self) -> Option<AvatarResult> {
		self.results.try_recv().ok()
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

// Build, rather than accept, URLs. Even malformed service metadata cannot choose a host/path.
fn cdn_url(key: &str) -> Option<String> {
	if let Some(value) = key.strip_prefix("role-icon-") {
		let (role, hash) = value.split_once('-')?;
		let role: Id = role.parse().ok()?;
		return (role.0 != 0 && model::valid_avatar_hash(hash))
			.then(|| format!("https://cdn.discordapp.com/role-icons/{role}/{hash}.png?size=128"));
	}
	if let Some(value) = key.strip_prefix("group-icon-") {
		let (channel, hash) = value.split_once('-')?;
		let channel: Id = channel.parse().ok()?;
		return model::valid_avatar_hash(hash).then(|| {
			format!("https://cdn.discordapp.com/channel-icons/{channel}/{hash}.png?size=128")
		});
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
		return model::valid_avatar_hash(hash)
			.then(|| format!("https://cdn.discordapp.com/banners/{id}/{hash}.png?size=512"));
	}
	for (prefix, kind, size) in [
		("member-avatar-", "avatars", 128),
		("member-banner-", "banners", 512),
	] {
		if let Some(value) = key.strip_prefix(prefix) {
			let (guild, value) = value.split_once('-')?;
			let (user, hash) = value.split_once('-')?;
			let guild: Id = guild.parse().ok()?;
			let user: Id = user.parse().ok()?;
			return model::valid_avatar_hash(hash).then(|| {
				format!(
					"https://cdn.discordapp.com/guilds/{guild}/users/{user}/{kind}/{hash}.png?size={size}"
				)
			});
		}
	}
	if let Some(source) = key.strip_prefix("anim:") {
		if model::valid_gif_url(source) && source.ends_with(".gif") {
			return Some(source.to_owned());
		}
		let mut url = url::Url::parse(&embed_url(source, ui::EMBED_EDGE)?).ok()?;
		let query: Vec<_> = url
			.query_pairs()
			.filter(|(key, _)| key != "format")
			.map(|(k, v)| (k.into_owned(), v.into_owned()))
			.collect();
		url.set_query(None);
		url.query_pairs_mut()
			.extend_pairs(query)
			.append_pair("format", "gif");
		return Some(url.into());
	}
	if let Some(source) = key.strip_prefix("embed:") {
		return embed_url(source, ui::EMBED_EDGE);
	}
	// The media viewer's rendition: same validation, larger proxy edge.
	if let Some(source) = key.strip_prefix("large:") {
		return embed_url(source, ui::LARGE_EDGE);
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
	(digest.len() == 32 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
		.then(|| format!("https://cdn.discordapp.com/avatars/{id}/{hash}.png?size=128"))
}

// Only service-provided image objects reach this path. Never fetch an arbitrary embed source.
pub(crate) fn embed_url(source: &str, edge: u32) -> Option<String> {
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
	// Keep the aspect ratio supplied by the media metadata, within the decode budget.
	let dimension = |name| {
		url.query_pairs()
			.find(|(key, _)| key == name)
			.and_then(|(_, value)| value.parse::<u32>().ok())
			.filter(|value| *value > 0)
	};
	let dimensions = dimension("width")
		.zip(dimension("height"))
		.map(|(width, height)| ui::fit_edge(width, height, edge));
	// Static proxy conversion is unofficial. A rejected/unsupported format stays a placeholder;
	// do not follow redirects, contact the original host, or add animation decoders as fallback.
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
	url.query_pairs_mut()
		.extend_pairs(query)
		.append_pair("format", "png");
	if let Some((width, height)) = dimensions {
		url.query_pairs_mut()
			.append_pair("width", &width.to_string())
			.append_pair("height", &height.to_string());
	} else {
		// Without dimensions, let the proxy derive height rather than request a square crop.
		url.query_pairs_mut()
			.append_pair("width", &edge.to_string());
	}
	Some(url.into())
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
		|| key.starts_with("large:")
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
	results: async_mpsc::Sender<AvatarResult>,
	mut cancelled: watch::Receiver<bool>,
	ctx: &egui::Context,
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
	// Four bounded downloads overlap; disk access and image decode stay on this worker.
	let mut downloads = tokio::task::JoinSet::new();
	loop {
		let (key, bytes, mut error, fetched, cached_image, cached_frames) = tokio::select! {
			biased;
			_ = cancelled.changed() => break,
			completed = downloads.join_next(), if !downloads.is_empty() => {
				let Some(Ok((key, bytes, until))) = completed else { break };
				cooldown = cooldown.max(until);
				(key, bytes, disk.is_none().then_some(CACHE_ERROR), true, None, Vec::new())
			},
			key = requests.recv(), if downloads.len() < 4 => {
				let Some(key) = key else { break };
				if *cancelled.borrow() { break; }
				let Some(url) = cdn_url(&key) else { continue };
				let mut error = disk.is_none().then_some(CACHE_ERROR);
				let cached = disk.as_mut().and_then(|disk| match disk.read(&key) {
					Ok(bytes) => bytes,
					Err(_) => { error = Some(CACHE_ERROR); None }
				});
				let edge = decode_edge(&key);
				let frames = key
					.starts_with("anim:")
					.then(|| cached.as_deref().and_then(decode_animation))
					.flatten()
					.unwrap_or_default();
				let image = frames.first().map(|(_, image)| image.as_ref().clone()).or_else(|| {
					cached.as_deref().and_then(|bytes| decode(bytes, edge))
				});
				if image.is_none() && Instant::now() >= cooldown && let Some(client) = &client {
					let client = client.clone();
					downloads.spawn(async move {
						let mut until = cooldown;
						let bytes = async {
							let url = if key.starts_with("app-icon-") {
								let metadata = download(&client, &url, &mut until, MAX_APPLICATION_METADATA).await?;
								application_icon_url(&key, &metadata)?
							} else { url };
							download(&client, &url, &mut until, encoded_limit(&key)).await
						}.await;
						(key, bytes, until)
					});
					continue;
				}
				(key, cached, error, false, image, frames)
			},
		};
		if *cancelled.borrow() {
			break;
		}
		let edge = decode_edge(&key);
		let frames = if !cached_frames.is_empty() {
			cached_frames
		} else if key.starts_with("anim:") {
			bytes
				.as_deref()
				.and_then(decode_animation)
				.unwrap_or_default()
		} else {
			Vec::new()
		};
		let image = frames
			.first()
			.map(|(_, image)| image.as_ref().clone())
			.or_else(|| {
				cached_image.or_else(|| bytes.as_deref().and_then(|bytes| decode(bytes, edge)))
			});
		if fetched
			&& image.is_some()
			&& let (Some(disk), Some(bytes)) = (&mut disk, &bytes)
			&& disk.write(&key, bytes).is_err()
		{
			error = Some(CACHE_ERROR);
		}
		// Decoded results can wait for the UI; the encoded source is no longer needed.
		drop(bytes);
		if *cancelled.borrow() {
			break;
		}
		tokio::select! {
			biased;
			_ = cancelled.changed() => break,
			result = results.send(AvatarResult { key, image, frames, error }) => if result.is_err() { break },
		}
		ctx.request_repaint();
	}
	downloads.abort_all();
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

/// Decode one still image and keep its longest edge within `edge` pixels.
fn decode(bytes: &[u8], edge: u32) -> Option<egui::ColorImage> {
	let large = edge > ui::EMBED_EDGE;
	let embed = edge >= ui::EMBED_EDGE;
	let encoded_limit = if large {
		MAX_LARGE_ENCODED
	} else if embed {
		MAX_ANIMATED_ENCODED
	} else {
		MAX_AVATAR_ENCODED
	};
	if bytes.len() > encoded_limit {
		return None;
	}
	// Provider previews can be GIF/JPEG/WebP; decode only the first frame, within limits.
	let mut reader = image::ImageReader::new(Cursor::new(bytes))
		.with_guessed_format()
		.ok()?;
	let mut limits = image::Limits::default();
	let side = if large {
		edge * 2
	} else if embed {
		1024
	} else {
		256
	};
	limits.max_image_width = Some(side);
	limits.max_image_height = Some(side);
	limits.max_alloc = Some(if large {
		96 * 1024 * 1024
	} else if embed {
		8 * 1024 * 1024
	} else {
		1024 * 1024
	});
	reader.limits(limits);
	let mut image = reader.decode().ok()?;
	// wgpu textures have no mip chain. A 128px face bilinear-minified into the
	// 48px rail aliases. Lanczos down to 64 leaves a 4/3 sample for that slot
	// and stays near 1:1 at 150% zoom or the 72px settings icon.
	let upload = if edge <= 128 { 64 } else { edge };
	if image.width() > upload || image.height() > upload {
		image = image.resize(upload, upload, image::imageops::FilterType::Lanczos3);
	}
	let image = image.into_rgba8();
	Some(egui::ColorImage::from_rgba_unmultiplied(
		[image.width() as usize, image.height() as usize],
		image.as_raw(),
	))
}

fn decode_animation(bytes: &[u8]) -> Option<ui::GifFrames> {
	use image::{AnimationDecoder, ImageDecoder};
	if bytes.len() > MAX_ANIMATED_ENCODED {
		return None;
	}
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(1024);
	limits.max_image_height = Some(1024);
	limits.max_alloc = Some(8 * 1024 * 1024);
	let decoded = match image::guess_format(bytes).ok()? {
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
		_ => return None,
	};
	let mut frames: ui::GifFrames = Vec::new();
	let mut stride = 1;
	let started = Instant::now();
	for (index, frame) in decoded.take(601).enumerate() {
		if index == 600 || started.elapsed() > Duration::from_secs(3) {
			return None;
		}
		let frame = frame.ok()?;
		let (numerator, denominator) = frame.delay().numer_denom_ms();
		let delay = Duration::from_millis(
			(u64::from(numerator) / u64::from(denominator.max(1))).clamp(20, 10_000),
		);
		if index % stride != 0 {
			frames.last_mut()?.0 += delay;
			continue;
		}
		// Keep the whole loop: reduce temporal detail instead of rejecting ordinary long GIFs.
		// ponytail: at most 80 160px frames (8 MiB); streaming decoding if full fidelity is needed.
		if frames.len() == 80 {
			frames = frames
				.chunks(2)
				.map(|pair| {
					(
						pair.iter().map(|(delay, _)| *delay).sum(),
						pair[0].1.clone(),
					)
				})
				.collect();
			stride *= 2;
		}
		let buffer = frame.into_buffer();
		let (width, height) = (buffer.width().min(160), buffer.height().min(160));
		let image = image::DynamicImage::ImageRgba8(buffer)
			.thumbnail(width, height)
			.into_rgba8();
		frames.push((
			delay,
			Arc::new(egui::ColorImage::from_rgba_unmultiplied(
				[image.width() as usize, image.height() as usize],
				image.as_raw(),
			)),
		));
	}
	Some(frames)
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
	fn group_icon_urls_accept_only_channel_ids_and_hashes() {
		assert_eq!(
			super::cdn_url("group-icon-7-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").as_deref(),
			Some(
				"https://cdn.discordapp.com/channel-icons/7/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
			)
		);
		for key in [
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
		let frames = super::decode_animation(&bytes).unwrap();
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
		let frames = super::decode_animation(&bytes).unwrap();
		assert_eq!(frames.len(), 2);
		assert_eq!(frames[0].0, std::time::Duration::from_millis(100));
		assert_eq!(frames[0].1.size, [160, 80]);
		assert_ne!(frames[0].1.pixels[0], frames[1].1.pixels[0]);
		assert!(super::decode_animation(b"not a GIF").is_none());
		assert!(super::decode_animation(&vec![0; super::MAX_ENCODED + 1]).is_none());
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
			"https://cdn.discordapp.com/guilds/2/users/1/banners/a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=512"
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
			"https://cdn.discordapp.com/avatars/1/a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png?size=128"
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
		let large_key = "large:https://cdn.discordapp.com/attachments/1/2/image.png?ex=abc&is=def&hm=synthetic&width=4096&height=1024";
		let transformed = cdn_url(large_key).unwrap();
		assert!(transformed.ends_with("format=png&width=2048&height=512"));
		assert_ne!(disk_key(large_key).unwrap(), disk_key(embed_key).unwrap());
		assert!(decode(&png(1025, 1), 512).is_none());
		assert_eq!(decode(&png(1024, 512), 512).unwrap().size, [512, 256]);
		assert_eq!(decode(&png(1024, 512), 2048).unwrap().size, [1024, 512]);
		assert!(decode(&png(4097, 1), 2048).is_none());
		assert!(decode(&vec![0; MAX_ENCODED + 1], 128).is_none());
		assert!(decode(b"not an image", 128).is_none());
		assert!(decode(&png(257, 1), 128).is_none());
		let bytes = png(256, 256);
		assert_eq!(decode(&bytes, 128).unwrap().size, [64, 64]);
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
