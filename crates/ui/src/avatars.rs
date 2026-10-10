//! Visible-avatar requests and a small texture working set. Disk/network work lives outside egui.
pub(crate) mod media;

use egui::{ColorImage, TextureHandle};
use model::User;
use std::{
	collections::{HashMap, HashSet},
	time::{Duration, Instant},
};

pub(crate) use media::{Quality, Surface};

pub type GifFrames = Vec<(Duration, std::sync::Arc<ColorImage>)>;
const ANIMATIONS: usize = 128;
const ANIMATION_BYTES: usize = 128 * 1024 * 1024;
const ANIMATION_INTERVAL: Duration = Duration::from_millis(16);
/// Decoded frames unplayed for this long are released; the still stays, and the frames are decoded
/// again from the disk cache once the artwork would play. Off-screen GIFs and hover-only avatars
/// otherwise held their full RGBA frame sets until a whole pool filled.
pub(crate) const IDLE_FRAMES: Duration = Duration::from_secs(5);
const RELEASED: usize = 2048;
struct Animation {
	frames: GifFrames,
	texture: Option<TextureHandle>,
	total: Duration,
	started: Instant,
	played: Instant,
	next_upload: Instant,
	frame: usize,
	bytes: usize,
}

// Emoji artwork has its own working set so media cannot evict it.
const EMOJI_TEXTURES: usize = 1024;
const EMOJI_TEXTURE_BYTES: usize = 16 * 1024 * 1024;
const TEXTURES: usize = 512;
const TEXTURE_BYTES: usize = 64 * 1024 * 1024;
const IDLE_TEXTURES: Duration = Duration::from_secs(60);
/// Longest edge for string-keyed artwork: stickers, picker previews, banners and activity art.
pub const EMBED_EDGE: u32 = 512;
const REQUESTS: usize = 128;
const ATTEMPTS: usize = 2048;
const RETRY: Duration = Duration::from_secs(5);

struct AvatarKey {
	avatar: Option<String>,
	discriminator: u16,
	demo: bool,
	key: std::sync::Arc<str>,
	used: u64,
}

#[derive(Default)]
pub(crate) struct Avatars {
	pub animate_gifs: bool,
	avatar_animation: bool,
	animations: HashMap<String, Animation>,
	no_animations: HashSet<String>,
	/// Animated keys whose frames were released or declined; they reload only when they would play.
	released: HashSet<String>,
	/// Hover-only artwork that wanted to play while its frames were missing.
	wanted: HashSet<String>,
	textures: HashMap<String, (Instant, TextureHandle)>,
	emoji_textures: HashMap<String, (Instant, TextureHandle)>,
	emoji_bytes: usize,
	avatar_keys: HashMap<model::Id, AvatarKey>,
	clock: u64,
	bytes: usize,
	pub revision: u64,
	attempts: HashMap<String, (Instant, bool)>,
	requests: Vec<String>,
	media: media::MediaLibrary,
}

impl Animation {
	fn advance(&mut self, ctx: &egui::Context) -> Option<TextureHandle> {
		let total_nanos = self.total.as_nanos();
		if total_nanos == 0 {
			return None;
		}
		let now = Instant::now();
		self.played = now;
		let elapsed_nanos = (self.started.elapsed().as_nanos() % total_nanos) as u64;
		let mut elapsed = Duration::from_nanos(elapsed_nanos);
		let mut target_index = 0;
		let mut frame_remaining = Duration::ZERO;
		for (index, (delay, _)) in self.frames.iter().enumerate() {
			if elapsed < *delay {
				target_index = index;
				frame_remaining = *delay - elapsed;
				break;
			}
			elapsed -= *delay;
		}

		if self.frame != target_index {
			if now < self.next_upload {
				ctx.request_repaint_after(self.next_upload - now);
				return self.texture.clone();
			}
			let image = &self.frames[target_index].1;
			if let Some(texture) = &mut self.texture {
				texture.set(image.clone(), egui::TextureOptions::LINEAR);
			} else {
				self.texture = Some(ctx.load_texture(
					"service-animation",
					image.clone(),
					egui::TextureOptions::LINEAR,
				));
			}
			self.frame = target_index;
			self.next_upload = now + ANIMATION_INTERVAL;
		}

		ctx.request_repaint_after(frame_remaining.max(Duration::from_millis(1)));
		self.texture.clone()
	}
}

fn paint_texture(
	ui: &egui::Ui,
	texture: &TextureHandle,
	rect: egui::Rect,
	radius: u8,
	cover: bool,
	tint: egui::Color32,
) -> egui::Rect {
	let source = texture.size_vec2();
	let image = egui::Image::new(texture).tint(tint);
	if cover {
		let scale = (rect.width() / source.x).max(rect.height() / source.y);
		let uv_size = rect.size() / (source * scale);
		image
			.uv(egui::Rect::from_center_size(egui::pos2(0.5, 0.5), uv_size))
			.corner_radius(egui::CornerRadius {
				nw: radius,
				ne: radius,
				sw: 0,
				se: 0,
			})
			.paint_at(ui, rect);
		rect
	} else {
		let scale = (rect.width() / source.x).min(rect.height() / source.y);
		let fitted = egui::Rect::from_center_size(rect.center(), source * scale);
		image.corner_radius(radius).paint_at(ui, fitted);
		fitted
	}
}

fn is_animated_profile_or_avatar_key(key: &str) -> bool {
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
		&& id.parse::<model::Id>().is_ok()
	{
		return hash.starts_with("a_");
	}
	false
}

impl Avatars {
	/// Expand playback to a hovered row or an open profile, still gated by settings and focus.
	pub fn with_avatar_animation<R>(
		&mut self,
		active: bool,
		draw: impl FnOnce(&mut Self) -> R,
	) -> R {
		let previous = std::mem::replace(&mut self.avatar_animation, active);
		let result = draw(self);
		self.avatar_animation = previous;
		result
	}
	#[cfg(test)]
	pub(crate) fn texture_id(&self, key: &str) -> Option<egui::TextureId> {
		self.textures
			.get(key)
			.or_else(|| self.emoji_textures.get(key))
			.map(|(_, texture)| texture.id())
			.or_else(|| self.media.texture_id(&media::Rendition::parse(key)?))
	}
	pub fn set_animation(&mut self, enabled: bool) {
		if self.animate_gifs == enabled {
			return;
		}
		self.animate_gifs = enabled;
		self.no_animations.clear();
		self.released.clear();
		self.wanted.clear();
		self.media.set_animation(enabled);
		if !enabled {
			self.animations.clear();
			self.textures.retain(|key, (_, texture)| {
				if key.starts_with("anim:") {
					self.bytes -= texture.byte_size();
					false
				} else {
					true
				}
			});
			self.attempts.retain(|key, _| !key.starts_with("anim:"));
			self.requests.retain(|key| !key.starts_with("anim:"));
		}
	}
	pub fn accept_animation(&mut self, key: String, frames: GifFrames) {
		if key.starts_with("media:") {
			if self.animate_gifs
				&& let Some(rendition) = media::Rendition::parse(&key)
			{
				self.media.accept_frames(rendition, frames);
			}
			return;
		}
		if !self.animate_gifs
			|| !self.textures.contains_key(&key)
			|| frames.len() < 2
			|| frames.len() > 200
		{
			if frames.len() < 2 {
				self.released.remove(&key);
				if self.no_animations.len() >= 2048 {
					self.no_animations.clear();
				}
				self.no_animations.insert(key);
			}
			return;
		}
		self.no_animations.remove(&key);
		// The worker decodes an animated avatar's frames with its still. Hover-only artwork keeps
		// them only when it is about to play; otherwise they would sit unused in the pool.
		if is_animated_profile_or_avatar_key(&key) && !self.wanted.remove(&key) {
			self.release(key);
			return;
		}
		self.released.remove(&key);
		let total: Duration = frames.iter().map(|(delay, _)| *delay).sum();
		if total.is_zero() {
			return;
		}
		// Reserve the playback texture too; the shared still stays unchanged for other widgets.
		let frame_bytes = || frames.iter().map(|(_, image)| image.pixels.len() * 4);
		let bytes = frame_bytes().sum::<usize>() + frame_bytes().max().unwrap_or(0);
		if bytes > ANIMATION_BYTES
			|| frames.iter().any(|(delay, image)| {
				*delay < Duration::from_millis(20)
					|| image.size[0] > EMBED_EDGE as usize
					|| image.size[1] > EMBED_EDGE as usize
			}) {
			return;
		}
		while self.animations.len() >= ANIMATIONS
			|| self.animations.values().map(|a| a.bytes).sum::<usize>() + bytes > ANIMATION_BYTES
		{
			let Some(oldest) = self
				.animations
				.keys()
				.min_by_key(|key| self.textures.get(*key).map(|v| v.0))
				.cloned()
			else {
				break;
			};
			self.animations.remove(&oldest);
		}
		self.animations.insert(
			key,
			Animation {
				frames,
				texture: None,
				total,
				started: Instant::now(),
				played: Instant::now(),
				next_upload: Instant::now(),
				frame: usize::MAX,
				bytes,
			},
		);
	}
	fn release(&mut self, key: String) {
		if self.released.len() >= RELEASED {
			self.released.clear();
		}
		self.released.insert(key);
	}
	/// Drained once per frame after the UI pass.
	pub fn take_requests(&mut self) -> Vec<String> {
		self.media.end_frame();
		self.release_idle(Instant::now());
		std::mem::take(&mut self.requests)
	}
	/// Releases idle decoded frames and artwork textures; visible artwork refreshes its deadline.
	fn release_idle(&mut self, now: Instant) {
		let idle: Vec<String> = self
			.animations
			.iter()
			.filter(|(_, animation)| now.saturating_duration_since(animation.played) >= IDLE_FRAMES)
			.map(|(key, _)| key.clone())
			.collect();
		for key in idle {
			self.animations.remove(&key);
			self.release(key);
		}
		if self.wanted.len() > REQUESTS {
			self.wanted.clear();
		}
		for (textures, bytes) in [
			(&mut self.textures, &mut self.bytes),
			(&mut self.emoji_textures, &mut self.emoji_bytes),
		] {
			textures.retain(|key, (used, texture)| {
				if now.saturating_duration_since(*used) < IDLE_TEXTURES {
					return true;
				}
				*bytes -= texture.byte_size();
				self.revision += 1;
				self.animations.remove(key);
				self.released.remove(key);
				self.wanted.remove(key);
				false
			});
		}
		self.media.release_idle(now);
	}
	/// Next idle frame/artwork release; the host wakes then to trim an idle window.
	pub fn next_release(&self) -> Option<Instant> {
		self.animations
			.values()
			.map(|animation| animation.played + IDLE_FRAMES)
			.chain(self.media.next_release())
			.chain(
				self.textures
					.values()
					.chain(self.emoji_textures.values())
					.map(|(used, _)| *used + IDLE_TEXTURES),
			)
			.min()
	}
	fn request(&mut self, key: String) {
		let now = Instant::now();
		if key.len() <= 2054
			&& self.requests.len() < REQUESTS
			&& self
				.attempts
				.get(&key)
				.is_none_or(|(at, failed)| *failed && now.duration_since(*at) >= RETRY)
			&& self.reserve_attempt(now)
		{
			self.attempts.insert(key.clone(), (now, false));
			self.requests.push(key);
		}
	}
	fn reserve_attempt(&mut self, now: Instant) -> bool {
		self.attempts
			.retain(|_, (at, failed)| !*failed || now.duration_since(*at) < RETRY);
		if self.attempts.len() >= ATTEMPTS
			&& let Some(oldest) = self
				.attempts
				.iter()
				.filter(|(_, (_, failed))| *failed)
				.min_by_key(|(_, (at, _))| *at)
				.map(|(key, _)| key.clone())
		{
			self.attempts.remove(&oldest);
		}
		self.attempts.len() < ATTEMPTS
	}
	pub fn accept(&mut self, ctx: &egui::Context, key: String, image: Option<ColorImage>) {
		if key.starts_with("media:") {
			if let Some(rendition) = media::Rendition::parse(&key)
				&& self.media.accept_still(ctx, rendition, image)
			{
				self.revision += 1;
			}
			return;
		}
		if !self.attempts.contains_key(&key) {
			return;
		}
		let limit = if key.starts_with("anim:")
			|| key.starts_with("embed:")
			|| key.starts_with("gif:")
			|| key.starts_with("spotify-")
			|| key.starts_with("banner-")
			|| key.starts_with("member-banner-")
		{
			EMBED_EDGE as usize
		} else if key.starts_with("emoji-unicode-") {
			256
		} else {
			128
		};
		let Some(image) = image.filter(|image| {
			image.size[0] > 0
				&& image.size[1] > 0
				&& image.size[0] <= limit
				&& image.size[1] <= limit
				&& image.pixels.len() == image.size[0] * image.size[1]
		}) else {
			if let Some(attempt) = self.attempts.get_mut(&key) {
				*attempt = (Instant::now(), true);
			}
			if self.no_animations.len() >= 2048 {
				self.no_animations.clear();
			}
			self.no_animations.insert(key);
			return;
		};
		self.attempts.remove(&key);
		let (textures, bytes, limit, byte_limit) = if key.starts_with("emoji-") {
			(
				&mut self.emoji_textures,
				&mut self.emoji_bytes,
				EMOJI_TEXTURES,
				EMOJI_TEXTURE_BYTES,
			)
		} else {
			(&mut self.textures, &mut self.bytes, TEXTURES, TEXTURE_BYTES)
		};
		if let Some((_, old)) = textures.remove(&key) {
			*bytes -= old.byte_size();
		}
		while textures.len() >= limit || *bytes + image.pixels.len() * 4 > byte_limit {
			let oldest = textures
				.iter()
				.min_by_key(|(_, (age, _))| *age)
				.map(|(key, _)| key.clone())
				.expect("texture cache over budget");
			self.animations.remove(&oldest);
			*bytes -= textures
				.remove(&oldest)
				.expect("oldest texture")
				.1
				.byte_size();
		}
		let texture = ctx.load_texture("service-image", image, egui::TextureOptions::LINEAR);
		*bytes += texture.byte_size();
		self.revision += 1;
		textures.insert(key, (Instant::now(), texture));
	}
	pub(crate) fn unicode_image(
		&mut self,
		ctx: &egui::Context,
		cell: usize,
		size: f32,
	) -> Option<egui::Image<'static>> {
		let physical = size * ctx.pixels_per_point();
		if !physical.is_finite() || physical <= 30.0 {
			return None;
		}
		let edge = if physical <= 64.0 {
			64
		} else if physical <= 128.0 {
			128
		} else {
			256
		};
		let key = format!("emoji-unicode-{cell}-{edge}");
		if let Some(entry) = self.emoji_textures.get_mut(&key) {
			entry.0 = Instant::now();
			Some(egui::Image::new(&entry.1).fit_to_exact_size(egui::Vec2::splat(size)))
		} else {
			self.request(key);
			None
		}
	}
	pub(crate) fn custom_image(
		&mut self,
		_ctx: &egui::Context,
		id: model::Id,
		size: f32,
		demo: bool,
	) -> Option<egui::Image<'static>> {
		let key = format!("emoji-{id}");
		#[cfg(any(test, feature = "demo"))]
		if demo && matches!(id.0, 9001 | 9002) && !self.emoji_textures.contains_key(&key) {
			let mut image = ColorImage::filled([32, 32], egui::Color32::TRANSPARENT);
			for y in 3..29 {
				for x in 3..29 {
					if (x + y + id.0 as usize) % 10 < 7 {
						image.pixels[y * 32 + x] = if id.0 == 9001 {
							egui::Color32::from_rgb(55, 180, 165)
						} else {
							egui::Color32::from_rgb(240, 150, 70)
						};
					}
				}
			}
			self.attempts.insert(key.clone(), (Instant::now(), false));
			self.accept(_ctx, key.clone(), Some(image));
		}
		if let Some(entry) = self.emoji_textures.get_mut(&key) {
			entry.0 = Instant::now();
			let image = egui::Image::new(&entry.1).fit_to_exact_size(egui::Vec2::splat(size));
			Some(image)
		} else {
			if !demo {
				self.request(key);
			}
			None
		}
	}
	fn sticker_key(
		&mut self,
		_ctx: &egui::Context,
		id: model::Id,
		format_type: u8,
		_demo: bool,
	) -> String {
		let prefix = if self.animate_gifs && matches!(format_type, 2 | 4) {
			"anim"
		} else {
			"embed"
		};
		let key = format!("{prefix}:sticker-{id}-{format_type}");
		#[cfg(any(test, feature = "demo"))]
		if _demo && !self.textures.contains_key(&key) {
			// Original synthetic mascot, generated locally; never downloaded service artwork.
			let mut image = ColorImage::filled([128, 128], egui::Color32::TRANSPARENT);
			let tint = [
				egui::Color32::from_rgb(103, 192, 177),
				egui::Color32::from_rgb(250, 181, 98),
				egui::Color32::from_rgb(172, 155, 241),
			][id.0 as usize % 3];
			for y in 0..128_i32 {
				for x in 0..128_i32 {
					let body = (x - 64).pow(2) + (y - 65).pow(2) < 46 * 46;
					let ears = (x - 35).pow(2) + (y - 28).pow(2) < 17 * 17
						|| (x - 93).pow(2) + (y - 28).pow(2) < 17 * 17;
					if body || ears {
						let eye = (x - 47).pow(2) + (y - 59).pow(2) < 5 * 5
							|| (x - 81).pow(2) + (y - 59).pow(2) < 5 * 5;
						let smile = (52..=76).contains(&x) && (80..=84).contains(&y);
						image.pixels[(y * 128 + x) as usize] = if eye || smile {
							egui::Color32::from_rgb(35, 39, 48)
						} else {
							tint
						};
					}
				}
			}
			self.attempts.insert(key.clone(), (Instant::now(), false));
			self.accept(_ctx, key.clone(), Some(image));
		}
		key
	}
	/// Composer and named artwork links share the same bounded native image cache.
	pub(crate) fn share_image(
		&mut self,
		ctx: &egui::Context,
		asset: model::ImageShare,
		size: f32,
		demo: bool,
	) -> Option<egui::Image<'static>> {
		match asset {
			model::ImageShare::Emoji { id, .. } => self.custom_image(ctx, id, size, demo),
			model::ImageShare::Sticker { id, format_type } => {
				let key = self.sticker_key(ctx, id, format_type, demo);
				let animated = self.advance_animation(ctx, &key, false);
				if let Some(entry) = self.textures.get_mut(&key) {
					entry.0 = Instant::now();
					Some(
						egui::Image::new(animated.as_ref().unwrap_or(&entry.1))
							.fit_to_exact_size(egui::Vec2::splat(size)),
					)
				} else {
					if !demo {
						if let Some((attempted, _)) = self.attempts.get(&key) {
							ctx.request_repaint_after(
								RETRY
									.saturating_sub(attempted.elapsed())
									.max(Duration::from_secs(1)),
							);
						}
						self.request(key);
					}
					None
				}
			}
		}
	}
	/// Layout reads cached artwork without refreshing expiry or requesting clipped assets.
	pub(crate) fn lookup_share_image(
		&mut self,
		ctx: &egui::Context,
		asset: model::ImageShare,
		size: f32,
		demo: bool,
	) -> Option<egui::Image<'static>> {
		// Demo artwork is generated locally, with no network or worker requests.
		if demo {
			return self.share_image(ctx, asset, size, true);
		}
		let texture = match asset {
			model::ImageShare::Emoji { id, .. } => self.emoji_textures.get(&format!("emoji-{id}")),
			model::ImageShare::Sticker { id, format_type } => {
				let key = self.sticker_key(ctx, id, format_type, false);
				self.textures.get(&key)
			}
		};
		texture.map(|(_, texture)| {
			egui::Image::new(texture).fit_to_exact_size(egui::Vec2::splat(size))
		})
	}

	/// Visible composer artwork stays warm without advancing unused animation frames.
	pub(crate) fn touch_share_image(
		&mut self,
		ctx: &egui::Context,
		asset: model::ImageShare,
		demo: bool,
	) {
		let (key, emoji) = match asset {
			model::ImageShare::Emoji { id, .. } => (format!("emoji-{id}"), true),
			model::ImageShare::Sticker { id, format_type } => {
				(self.sticker_key(ctx, id, format_type, false), false)
			}
		};
		let textures = if emoji {
			&mut self.emoji_textures
		} else {
			&mut self.textures
		};
		if let Some((used, _)) = textures.get_mut(&key) {
			*used = Instant::now();
		} else if !demo {
			if let Some((attempted, failed)) = self.attempts.get(&key)
				&& *failed
			{
				ctx.request_repaint_after(
					RETRY
						.saturating_sub(attempted.elapsed())
						.max(Duration::from_secs(1)),
				);
			}
			self.request(key);
		}
	}

	/// Transparent, clickable sticker artwork using the shared bounded media working set.
	pub(crate) fn sticker_image(
		&mut self,
		ui: &mut egui::Ui,
		sticker: &model::Sticker,
		size: egui::Vec2,
		demo: bool,
	) -> egui::Response {
		let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &sticker.name)
		});
		if !ui.is_rect_visible(rect) {
			return response;
		}
		let key = self.sticker_key(ui.ctx(), sticker.id, sticker.format_type, demo);
		if !self.paint(ui, &key, rect, 0) {
			let failed = self.attempts.get(&key).is_some_and(|(_, failed)| *failed);
			let supported = sticker.id.0 != 0 && matches!(sticker.format_type, 1..=4);
			let colors = crate::design::palette(ui);
			let placeholder = rect.shrink(8.0);
			ui.painter().rect_filled(placeholder, 8, colors.raised);
			if failed || !supported {
				response.clone().on_hover_text(crate::i18n::translate(
					"avatars-sticker-image-image-unavailable",
				));
			}
			if !demo && supported {
				// Retry uses the shared bounded cooldown, including when the pointer is idle.
				if let Some((attempted, _)) = self.attempts.get(&key) {
					ui.ctx().request_repaint_after(
						RETRY
							.saturating_sub(attempted.elapsed())
							.max(Duration::from_secs(1)),
					);
				}
				self.request(key);
			}
		}
		response
	}
	/// Picker preview texture; a small GIF preview plays, never the full-size original.
	/// Synthetic previews are painted locally.
	pub(crate) fn gif_texture(
		&mut self,
		ctx: &egui::Context,
		gif: &model::Gif,
		demo: bool,
	) -> Option<(egui::TextureId, [usize; 2])> {
		if !model::valid_gif_preview(&gif.preview) {
			return None;
		}
		#[cfg(any(test, feature = "demo"))]
		if demo && gif.preview.contains("/synthetic/") {
			let key = self.preview_key(&gif.preview);
			if !self.textures.contains_key(&key) {
				self.attempts.insert(key.clone(), (Instant::now(), false));
				self.accept(ctx, key, Some(synthetic_gif(gif)));
			}
		}
		self.preview_texture(ctx, &gif.preview, demo)
	}
	fn preview_key(&self, preview: &str) -> String {
		if self.animate_gifs && (preview.ends_with(".gif") || preview.ends_with(".webp")) {
			format!("anim:{preview}")
		} else {
			format!("gif:{preview}")
		}
	}
	/// Provider artwork such as a GIF category tile; it plays when animation is enabled.
	pub(crate) fn preview_texture(
		&mut self,
		ctx: &egui::Context,
		preview: &str,
		demo: bool,
	) -> Option<(egui::TextureId, [usize; 2])> {
		let key = self.preview_key(preview);
		let animated_texture = self.advance_animation(ctx, &key, false);
		if let Some(entry) = self.textures.get_mut(&key) {
			entry.0 = Instant::now();
			let texture = animated_texture.as_ref().unwrap_or(&entry.1);
			let texture = (texture.id(), texture.size());
			Some(texture)
		} else {
			if !demo {
				self.request(key);
			}
			None
		}
	}
	/// Paints the profile banner (or its accent color) into `rect`; corners follow the card.
	pub fn paint_banner(
		&mut self,
		ui: &mut egui::Ui,
		profile: &model::UserProfile,
		rect: egui::Rect,
		corner: egui::CornerRadius,
		demo: bool,
	) {
		let color = profile
			.accent_color
			.or(profile.theme_colors.map(|c| c[0]))
			.map(|rgb| egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
			.unwrap_or(crate::design::palette(ui).accent.gamma_multiply(0.4));
		ui.painter().rect_filled(rect, corner, color);
		if ui.is_rect_visible(rect)
			&& let Some(key) = profile.banner_key()
		{
			let is_animated = is_animated_profile_or_avatar_key(&key);
			if self.animate_gifs
				&& is_animated
				&& !self.animations.contains_key(&key)
				&& !self.no_animations.contains(&key)
				&& !self.released.contains(&key)
				&& !demo
			{
				self.request(key.clone());
			}
			let animated_texture = self.advance_animation(ui.ctx(), &key, true);
			#[cfg(any(test, feature = "demo"))]
			if demo && !self.textures.contains_key(&key) {
				let mut image = ColorImage::filled([128, 48], color);
				let stripe = color.lerp_to_gamma(egui::Color32::WHITE, 0.16);
				for y in 0..48 {
					for x in 0..128 {
						if (x + y) % 48 < 12 {
							image.pixels[y * 128 + x] = stripe;
						}
					}
				}
				self.attempts.insert(key.clone(), (Instant::now(), false));
				self.accept(ui.ctx(), key.clone(), Some(image));
			}
			if let Some(entry) = self.textures.get_mut(&key) {
				entry.0 = Instant::now();
				let texture = animated_texture.as_ref().unwrap_or(&entry.1);
				let source = texture.size_vec2();
				let scale = (rect.width() / source.x).max(rect.height() / source.y);
				let uv_size = rect.size() / (source * scale);
				let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), uv_size);
				egui::Image::new(texture)
					.uv(uv)
					.corner_radius(corner)
					.paint_at(ui, rect);
			} else if !demo {
				self.request(key);
			}
		}
	}
	/// Small square artwork (badge or server tag). Falls back to a neutral disc until loaded.
	pub fn show_icon(
		&mut self,
		ui: &mut egui::Ui,
		key: Option<String>,
		size: f32,
		demo: bool,
		label: &str,
	) -> egui::Response {
		let (rect, response) =
			ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
		if ui.is_rect_visible(rect) {
			let colors = crate::design::palette(ui);
			if let Some(key) = key {
				#[cfg(any(test, feature = "demo"))]
				if demo && !self.textures.contains_key(&key) {
					// Original synthetic emblem; never bundled third-party badge artwork.
					let seed = key
						.bytes()
						.fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
					let tint = egui::Color32::from_rgb(
						90 + (seed % 120) as u8,
						120 + ((seed >> 8) % 100) as u8,
						150 + ((seed >> 16) % 90) as u8,
					);
					let mut image = ColorImage::filled([32, 32], egui::Color32::TRANSPARENT);
					for y in 0..32_i32 {
						for x in 0..32_i32 {
							let d = (x - 16).pow(2) + (y - 16).pow(2);
							if d < 196 {
								image.pixels[(y * 32 + x) as usize] =
									if d < 36 { egui::Color32::WHITE } else { tint };
							}
						}
					}
					self.attempts.insert(key.clone(), (Instant::now(), false));
					self.accept(ui.ctx(), key.clone(), Some(image));
				}
				if !self.paint(ui, &key, rect, (size * 0.25) as u8) {
					ui.painter()
						.circle_filled(rect.center(), size * 0.4, colors.raised);
					if !demo {
						self.request(key);
					}
				}
			} else {
				ui.painter()
					.circle_filled(rect.center(), size * 0.4, colors.raised);
				ui.painter().circle_stroke(
					rect.center(),
					size * 0.4,
					egui::Stroke::new(1.0, colors.border),
				);
			}
		}
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), label));
		response
	}
	/// Portrait game cover for the profile board, cropped to fill; a rounded tile until loaded.
	pub fn show_cover(
		&mut self,
		ui: &mut egui::Ui,
		key: Option<String>,
		size: egui::Vec2,
		demo: bool,
		label: &str,
	) -> egui::Response {
		let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
		if ui.is_rect_visible(rect) {
			let colors = crate::design::palette(ui);
			#[cfg(any(test, feature = "demo"))]
			if demo
				&& let Some(key) = &key
				&& !self.textures.contains_key(key)
			{
				// Original synthetic two-tone artwork; never bundled third-party covers.
				let seed = key
					.bytes()
					.fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
				let top = egui::Color32::from_rgb(
					60 + (seed % 140) as u8,
					70 + ((seed >> 8) % 120) as u8,
					110 + ((seed >> 16) % 120) as u8,
				);
				let mut image = ColorImage::filled([24, 32], top);
				for y in 0..32 {
					let shade = top.lerp_to_gamma(egui::Color32::BLACK, y as f32 / 48.0);
					for x in 0..24 {
						image.pixels[y * 24 + x] = shade;
					}
				}
				self.attempts.insert(key.clone(), (Instant::now(), false));
				self.accept(ui.ctx(), key.clone(), Some(image));
			}
			if !key
				.as_ref()
				.is_some_and(|key| self.paint_fitted(ui, key, rect, 8, true))
			{
				ui.painter().rect_filled(rect, 8, colors.hover);
				crate::icons::paint(
					ui.painter(),
					crate::icons::Icon::GameController,
					egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(28.0)),
					colors.muted,
				);
				if let Some(key) = key
					&& !demo
				{
					self.request(key);
				}
			}
		}
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), label));
		response
	}
	pub fn show_profile_avatar(
		&mut self,
		ui: &mut egui::Ui,
		profile: &model::UserProfile,
		size: f32,
		demo: bool,
	) -> egui::Response {
		if profile.guild.as_ref().is_none_or(|g| g.avatar.is_none()) {
			return self.show(ui, &profile.user, size, demo);
		}
		let (_, response) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
		let response = response.on_hover_text(&profile.user.name);
		if ui.is_rect_visible(response.rect) {
			let key = profile.avatar_key();
			#[cfg(any(test, feature = "demo"))]
			if demo && !self.textures.contains_key(&key) {
				let image = ColorImage::filled([32, 32], crate::design::palette(ui).accent);
				self.attempts.insert(key.clone(), (Instant::now(), false));
				self.accept(ui.ctx(), key.clone(), Some(image));
			}
			if !self.paint(
				ui,
				&key,
				ui.layout()
					.align_size_within_rect(egui::Vec2::splat(size), response.rect),
				(size * 0.5) as u8,
			) {
				crate::design::paint_avatar(ui, &profile.user.name, size, response.rect);
				if !demo {
					self.request(key);
				}
			}
		}
		response.widget_info(|| {
			egui::WidgetInfo::labeled(
				egui::Role::Image,
				ui.is_enabled(),
				crate::i18n::translate("avatars-show-profile-avatar-server-profile-picture"),
			)
		});
		response
	}
	fn advance_animation(
		&mut self,
		ctx: &egui::Context,
		key: &str,
		hovered: bool,
	) -> Option<TextureHandle> {
		if !self.animate_gifs
			|| !ctx.input(|input| input.focused)
			|| !(hovered || self.avatar_animation || !is_animated_profile_or_avatar_key(key))
		{
			return None;
		}
		if let Some(animation) = self.animations.get_mut(key) {
			return animation.advance(ctx);
		}
		if is_animated_profile_or_avatar_key(key) {
			if self.wanted.len() >= REQUESTS {
				self.wanted.clear();
			}
			self.wanted.insert(key.to_owned());
		}
		if self.released.contains(key)
			&& self.textures.contains_key(key)
			&& !self.no_animations.contains(key)
		{
			self.request(key.to_owned());
		}
		None
	}
	fn paint(&mut self, ui: &mut egui::Ui, key: &str, rect: egui::Rect, radius: u8) -> bool {
		self.paint_fitted(ui, key, rect, radius, false)
	}
	/// Paints a decoded ThumbHash placeholder into `rect`; decoding happens once per hash.
	fn paint_placeholder(
		&mut self,
		ui: &mut egui::Ui,
		hash: &[u8],
		rect: egui::Rect,
		radius: u8,
		cover: bool,
	) -> bool {
		if hash.is_empty() || hash.len() > model::MAX_PLACEHOLDER_BYTES {
			return false;
		}
		let mut key = String::with_capacity(6 + hash.len() * 2);
		key.push_str("thumb:");
		for byte in hash {
			use std::fmt::Write;
			let _ = write!(key, "{byte:02x}");
		}
		if !self.textures.contains_key(&key) {
			if self.attempts.get(&key).is_some_and(|(_, failed)| *failed) {
				return false;
			}
			if !self.reserve_attempt(Instant::now()) {
				return false;
			}
			self.attempts.insert(key.clone(), (Instant::now(), false));
			self.accept(ui.ctx(), key.clone(), crate::thumbhash::decode(hash));
		}
		self.paint_fitted(ui, &key, rect, radius, cover)
	}
	fn paint_fitted(
		&mut self,
		ui: &mut egui::Ui,
		key: &str,
		rect: egui::Rect,
		radius: u8,
		cover: bool,
	) -> bool {
		let mut animated_texture = None;
		if ui.is_rect_visible(rect) {
			let is_animated = key.starts_with("anim:") || is_animated_profile_or_avatar_key(key);
			if is_animated
				&& self.animate_gifs
				&& !self.animations.contains_key(key)
				&& !self.no_animations.contains(key)
				&& !self.released.contains(key)
			{
				self.request(key.to_string());
			}
			animated_texture =
				self.advance_animation(ui.ctx(), key, ui.rect_contains_pointer(rect));
		}
		let Some(entry) = self.textures.get_mut(key) else {
			return false;
		};
		if ui.is_rect_visible(rect) {
			entry.0 = Instant::now();
		}
		let texture = animated_texture.as_ref().unwrap_or(&entry.1);
		paint_texture(ui, texture, rect, radius, cover, egui::Color32::WHITE);
		true
	}
	pub fn show_group(
		&mut self,
		ui: &mut egui::Ui,
		channel: &model::Channel,
		size: f32,
		demo: bool,
	) -> egui::Response {
		self.group_avatar(ui, channel, size, demo, true)
	}
	pub fn show_group_rail(
		&mut self,
		ui: &mut egui::Ui,
		channel: &model::Channel,
		size: f32,
		demo: bool,
	) -> egui::Response {
		self.group_avatar(ui, channel, size, demo, false)
	}
	fn group_avatar(
		&mut self,
		ui: &mut egui::Ui,
		channel: &model::Channel,
		size: f32,
		demo: bool,
		hover_name: bool,
	) -> egui::Response {
		let (rect, response) =
			ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click());
		let colors = crate::design::palette(ui);
		let mut painted = false;
		if ui.is_rect_visible(rect)
			&& let Some(hash) = channel
				.icon
				.as_deref()
				.filter(|hash| model::valid_avatar_hash(hash))
		{
			let key = format!("group-icon-{}-{hash}", channel.id);
			painted = self.paint(ui, &key, rect, (size / 2.0) as u8);
			if !painted && !demo {
				self.request(key);
			}
		}
		if !painted && channel.icon.is_none() && !channel.recipients.is_empty() {
			let users = &channel.recipients;
			let diameter = if users.len() > 1 { size * 0.66 } else { size };
			for (index, user) in users.iter().take(2).enumerate() {
				let corner = if index == 0 {
					rect.left_top()
				} else {
					rect.right_bottom() - egui::Vec2::splat(diameter)
				};
				let avatar = egui::Rect::from_min_size(corner, egui::Vec2::splat(diameter));
				if index == 1 {
					ui.painter().circle_filled(
						avatar.center(),
						diameter * 0.5 + size * 0.04,
						colors.sidebar,
					);
				}
				self.paint_user(ui, user, diameter, avatar, demo);
			}
			painted = true;
		}
		if !painted {
			ui.painter()
				.circle_filled(rect.center(), size / 2.0, colors.raised);
			crate::icons::paint(
				ui.painter(),
				crate::icons::Icon::People,
				rect.shrink(size * 0.22),
				colors.muted,
			);
		}
		response.widget_info(|| {
			egui::WidgetInfo::labeled(
				egui::Role::Button,
				ui.is_enabled(),
				format!("Group {}", channel.name),
			)
		});
		if hover_name {
			response.on_hover_text(&channel.name)
		} else {
			response
		}
	}
	pub fn show_guild_rail(
		&mut self,
		ui: &mut egui::Ui,
		guild: &model::Guild,
		selected: bool,
		demo: bool,
	) -> egui::Response {
		self.guild_avatar(
			ui,
			guild,
			selected,
			demo,
			crate::notifications::RAIL_TILE,
			false,
		)
	}
	pub fn paint_guild(
		&mut self,
		ui: &mut egui::Ui,
		guild: &model::Guild,
		rect: egui::Rect,
		demo: bool,
		radius: u8,
	) {
		self.paint_guild_face(ui, guild, rect, demo, 0.0, radius);
	}
	pub fn show_guild_sized(
		&mut self,
		ui: &mut egui::Ui,
		guild: &model::Guild,
		selected: bool,
		demo: bool,
		size: f32,
	) -> egui::Response {
		self.guild_avatar(ui, guild, selected, demo, size, true)
	}
	fn guild_avatar(
		&mut self,
		ui: &mut egui::Ui,
		guild: &model::Guild,
		selected: bool,
		demo: bool,
		size: f32,
		hover_name: bool,
	) -> egui::Response {
		let (rect, response) =
			ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click_and_drag());
		let highlight = selected || response.hovered() || response.has_focus();
		let squircle = size * 0.29;
		// Only the rail animates; settings previews and drag previews keep the static squircle.
		if hover_name || !ui.is_rect_visible(rect) {
			self.paint_guild_face(ui, guild, rect, demo, f32::from(highlight), squircle as u8);
		} else {
			let time = crate::notifications::rail_motion(ui);
			let lit = ui.ctx().animate_bool_with_time_and_easing(
				response.id.with("rail-morph"),
				highlight,
				time,
				egui::emath::easing::cubic_out,
			);
			let pressed = response.is_pointer_button_down_on()
				&& !ui.input(|input| input.pointer.is_decidedly_dragging());
			let press = ui.ctx().animate_bool_with_time(
				response.id.with("rail-press"),
				pressed,
				time * 0.5,
			);
			let face =
				egui::Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.05 * press));
			// Rests rounder and morphs to the squircle when hovered or selected, like Discord.
			let radius = egui::lerp(size * 0.4..=squircle, lit).round() as u8;
			self.paint_guild_face(ui, guild, face, demo, lit, radius);
		}
		response.widget_info(|| {
			egui::WidgetInfo::selected(
				egui::Role::Button,
				ui.is_enabled(),
				selected,
				format!("Server {}", guild.name),
			)
		});
		if hover_name {
			response.on_hover_text(&guild.name)
		} else {
			response
		}
	}
	fn paint_guild_face(
		&mut self,
		ui: &mut egui::Ui,
		guild: &model::Guild,
		rect: egui::Rect,
		demo: bool,
		highlight: f32,
		radius: u8,
	) {
		// The rail is not virtualized; skip initials layout for scrolled-out servers.
		if !ui.is_rect_visible(rect) {
			return;
		}
		let size = rect.width().min(rect.height());
		let initials_size = (size * 0.5).clamp(7.0, 16.0);
		let short: String = guild
			.name
			.split_whitespace()
			.filter_map(|word| word.chars().next())
			.take(2)
			.collect();
		let colors = crate::design::palette(ui);
		let mut painted = false;
		if let Some(key) = guild.icon_key() {
			#[cfg(any(test, feature = "demo"))]
			if demo && !self.textures.contains_key(&key) {
				let seed = key
					.bytes()
					.fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
				let fill = egui::Color32::from_rgb(
					70 + (seed % 140) as u8,
					90 + ((seed >> 8) % 120) as u8,
					110 + ((seed >> 16) % 100) as u8,
				);
				let mut image = ColorImage::filled([32, 32], fill);
				for row in [8, 14, 20] {
					for y in row..row + 3 {
						for x in 7..25 {
							image.pixels[y * 32 + x] = egui::Color32::WHITE;
						}
					}
				}
				self.attempts.insert(key.clone(), (Instant::now(), false));
				self.accept(ui.ctx(), key.clone(), Some(image));
			}
			painted = self.paint(ui, &key, rect, radius);
			if !painted && !demo {
				self.request(key);
			}
		}
		if !painted {
			ui.painter().rect_filled(
				rect,
				radius,
				colors.raised.lerp_to_gamma(colors.accent, highlight),
			);
			ui.painter().text(
				rect.center(),
				egui::Align2::CENTER_CENTER,
				short,
				egui::FontId::new(initials_size, crate::design::medium_family(ui.ctx())),
				colors.text.lerp_to_gamma(colors.accent_text, highlight),
			);
		}
	}
	pub fn show_gif_embed(
		&mut self,
		ui: &mut egui::Ui,
		embed: &model::Embed,
		gif: Option<&model::Gif>,
		size: egui::Vec2,
		demo: bool,
	) -> egui::Response {
		let poster = embed.image.as_ref().or(embed.thumbnail.as_ref());
		let video = self
			.animate_gifs
			.then_some(embed.video.as_ref())
			.flatten()
			.and_then(|video| {
				let url = video
					.url
					.as_deref()
					.filter(|url| media::is_motion_video(url))
					.or(video
						.proxy_url
						.as_deref()
						.filter(|url| media::is_motion_video(url)))
					.filter(|url| self.media.playable(url))?;
				Some(model::EmbedMedia {
					url: Some(url.to_owned()),
					proxy_url: video
						.proxy_url
						.clone()
						.filter(|proxy| media::is_motion_video(proxy)),
					width: if video.width > 0 {
						video.width
					} else {
						poster.map(|poster| poster.width).unwrap_or(0)
					},
					height: if video.height > 0 {
						video.height
					} else {
						poster.map(|poster| poster.height).unwrap_or(0)
					},
					placeholder: if video.placeholder.is_empty() {
						poster
							.map(|poster| poster.placeholder.clone())
							.unwrap_or_default()
					} else {
						video.placeholder.clone()
					},
				})
			});
		let original = gif
			.filter(|gif| self.animate_gifs && gif.url.ends_with(".gif"))
			.map(|gif| model::EmbedMedia {
				url: Some(gif.url.clone()),
				proxy_url: (!model::valid_gif_preview(&gif.url)
					&& !model::valid_discord_media_url(&gif.url))
				.then(|| gif.preview.clone()),
				width: gif.width,
				height: gif.height,
				placeholder: poster
					.map(|poster| poster.placeholder.clone())
					.unwrap_or_default(),
			});
		let media = video
			.as_ref()
			.or(original.as_ref())
			.or(embed.image.as_ref())
			.or(embed.thumbnail.as_ref())
			.or(embed.video.as_ref());
		self.show_media(
			ui,
			media.unwrap_or(&model::EmbedMedia::default()),
			size,
			demo,
			Surface::Inline,
		)
		.response
	}
	pub(crate) fn paint_user(
		&mut self,
		ui: &mut egui::Ui,
		user: &User,
		size: f32,
		rect: egui::Rect,
		demo: bool,
	) {
		if ui.is_rect_visible(rect) {
			self.clock += 1;
			let avatar = user
				.avatar
				.as_deref()
				.filter(|hash| model::valid_avatar_hash(hash));
			let valid = self.avatar_keys.get(&user.id).is_some_and(|entry| {
				entry.avatar.as_deref() == avatar
					&& entry.discriminator == user.discriminator
					&& entry.demo == demo
			});
			if !valid {
				if self.avatar_keys.len() >= TEXTURES {
					let oldest = *self
						.avatar_keys
						.iter()
						.min_by_key(|(_, entry)| entry.used)
						.expect("avatar key cache")
						.0;
					self.avatar_keys.remove(&oldest);
				}
				let key = if demo {
					format!("preview-{}", user.id)
				} else {
					user.avatar_key()
				};
				self.avatar_keys.insert(
					user.id,
					AvatarKey {
						avatar: avatar.map(str::to_owned),
						discriminator: user.discriminator,
						demo,
						key: key.into(),
						used: self.clock,
					},
				);
			}
			let entry = self.avatar_keys.get_mut(&user.id).expect("avatar key");
			entry.used = self.clock;
			let key = entry.key.clone();
			#[cfg(any(test, feature = "demo"))]
			if demo && !self.textures.contains_key(key.as_ref()) {
				// Original, synthetic silhouettes exercise the image path without network or assets.
				let background = if user.id.0.is_multiple_of(2) {
					egui::Color32::from_rgb(63, 99, 111)
				} else {
					egui::Color32::from_rgb(103, 86, 124)
				};
				let foreground = egui::Color32::from_rgb(224, 237, 227);
				let mut image = ColorImage::filled([32, 32], background);
				for y in 0..32_i32 {
					for x in 0..32_i32 {
						if (x - 16).pow(2) + (y - 11).pow(2) < 36
							|| (x - 16).pow(2) + (y - 31).pow(2) < 121
						{
							image.pixels[(y * 32 + x) as usize] = foreground;
						}
					}
				}
				self.attempts
					.insert(key.to_string(), (Instant::now(), false));
				self.accept(ui.ctx(), key.to_string(), Some(image));
			}
			if !self.paint(ui, &key, rect, (size * 0.5) as u8) {
				crate::design::paint_avatar(ui, &user.name, size, rect);
				if !demo {
					self.request(key.to_string());
				}
			}
		}
	}
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		user: &User,
		size: f32,
		demo: bool,
	) -> egui::Response {
		self.user_avatar(ui, user, size, demo, true, true)
	}
	/// Avatar that never opens a profile: rows that already own their click keep it quiet.
	pub fn show_plain(
		&mut self,
		ui: &mut egui::Ui,
		user: &User,
		size: f32,
		demo: bool,
	) -> egui::Response {
		self.user_avatar(ui, user, size, demo, false, true)
	}
	/// Hover-only avatar without a tooltip, for rows whose own label already names the user.
	pub fn show_quiet(
		&mut self,
		ui: &mut egui::Ui,
		user: &User,
		size: f32,
		demo: bool,
	) -> egui::Response {
		self.user_avatar(ui, user, size, demo, false, false)
	}
	pub fn show_rail(
		&mut self,
		ui: &mut egui::Ui,
		user: &User,
		size: f32,
		demo: bool,
	) -> egui::Response {
		self.user_avatar(ui, user, size, demo, true, false)
	}
	fn user_avatar(
		&mut self,
		ui: &mut egui::Ui,
		user: &User,
		size: f32,
		demo: bool,
		opens_profile: bool,
		hover_name: bool,
	) -> egui::Response {
		let (_, response) = ui.allocate_exact_size(
			egui::Vec2::splat(size),
			if opens_profile {
				egui::Sense::click()
			} else {
				egui::Sense::hover()
			},
		);
		let response = if hover_name {
			response.on_hover_text(&user.name)
		} else {
			response
		};
		let rect = ui
			.layout()
			.align_size_within_rect(egui::Vec2::splat(size), response.rect);
		self.paint_user(ui, user, size, rect, demo);
		response.widget_info(|| {
			if opens_profile {
				egui::WidgetInfo::labeled(
					egui::Role::Button,
					ui.is_enabled(),
					format!("View profile for {}", user.name),
				)
			} else {
				egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), &user.name)
			}
		});
		response
	}
}

/// Offline fixture artwork: a soft two-tone gradient with a highlight, sized like the GIF.
#[cfg(any(test, feature = "demo"))]
fn synthetic_gif(gif: &model::Gif) -> ColorImage {
	let seed = gif.id.bytes().fold(7usize, |acc, b| {
		acc.wrapping_mul(31).wrapping_add(b as usize)
	});
	let width = 256usize;
	let height = ((256.0 * gif.height as f32 / gif.width.max(1) as f32) as usize).clamp(64, 512);
	let hue = ((seed % 97) as f32 * 0.618_034) % 1.0;
	let a = egui::ecolor::Hsva::new(hue, 0.62, 0.78, 1.0).to_rgba_premultiplied();
	let b = egui::ecolor::Hsva::new((hue + 0.12) % 1.0, 0.58, 0.42, 1.0).to_rgba_premultiplied();
	let (cx, cy) = (
		0.3 + (seed % 5) as f32 * 0.1,
		0.35 + (seed % 3) as f32 * 0.12,
	);
	let mut image = ColorImage::filled([width, height], egui::Color32::BLACK);
	for y in 0..height {
		for x in 0..width {
			let (u, v) = (x as f32 / width as f32, y as f32 / height as f32);
			let t = ((u + v) * 0.5).clamp(0.0, 1.0);
			let mut rgb = [0.0f32; 3];
			for (i, channel) in rgb.iter_mut().enumerate() {
				*channel = a[i] * (1.0 - t) + b[i] * t;
			}
			let d = ((u - cx).powi(2) + ((v - cy) * height as f32 / width as f32).powi(2)).sqrt();
			let glow = (1.0 - d / 0.5).clamp(0.0, 1.0).powi(2) * 0.3;
			let band = (((u * 3.0 - v * 2.0) * std::f32::consts::PI).sin() * 0.5 + 0.5) * 0.06;
			image.pixels[y * width + x] = egui::Color32::from_rgb(
				((rgb[0] + glow + band) * 255.0).min(255.0) as u8,
				((rgb[1] + glow + band) * 255.0).min(255.0) as u8,
				((rgb[2] + glow + band) * 255.0).min(255.0) as u8,
			);
		}
	}
	image
}

#[cfg(test)]
mod tests {
	#[test]
	fn malformed_placeholders_keep_attempts_bounded_and_leave_room_for_real_images() {
		use super::*;
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		ctx.run_ui(Default::default(), |ui| {
			for seed in 0..ATTEMPTS as u32 * 2 {
				let mut hash = seed.to_le_bytes().to_vec();
				hash.push(0);
				assert!(!avatars.paint_placeholder(ui, &hash, ui.max_rect(), 0, false));
				assert!(avatars.attempts.len() <= ATTEMPTS);
			}
			assert_eq!(avatars.attempts.len(), ATTEMPTS);
			avatars.request("default-0".into());
			assert_eq!(avatars.take_requests(), ["default-0"]);
			avatars.accept(
				&ctx,
				"default-0".into(),
				Some(ColorImage::filled([1, 1], egui::Color32::WHITE)),
			);
			assert!(avatars.textures.contains_key("default-0"));
		})
		.drop_without_applying_deltas();
	}

	#[test]
	fn idle_artwork_expires_without_evicting_visible_images_and_reloads() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		for key in ["visible", "hidden", "emoji-1", "emoji-2"] {
			avatars.request(key.into());
			avatars.take_requests();
			avatars.accept(
				&ctx,
				key.into(),
				Some(ColorImage::filled([64, 64], egui::Color32::WHITE)),
			);
		}
		let now = Instant::now();
		for (used, _) in avatars
			.textures
			.values_mut()
			.chain(avatars.emoji_textures.values_mut())
		{
			*used = now - IDLE_TEXTURES;
		}
		assert_eq!(avatars.next_release(), Some(now));
		avatars.release_idle(now - Duration::from_millis(1));
		assert_eq!(avatars.textures.len() + avatars.emoji_textures.len(), 4);
		ctx.run_ui(Default::default(), |ui| {
			let visible = egui::Rect::from_min_size(ui.min_rect().min, egui::vec2(64.0, 64.0));
			assert!(avatars.paint(ui, "visible", visible, 0));
			let hidden = visible.translate(egui::vec2(100_000.0, 100_000.0));
			assert!(avatars.paint(ui, "hidden", hidden, 0));
		})
		.drop_without_applying_deltas();
		assert!(
			avatars
				.custom_image(&ctx, model::Id(1), 20.0, false)
				.is_some()
		);
		avatars.release_idle(now);
		assert!(avatars.texture_id("visible").is_some());
		assert!(avatars.texture_id("emoji-1").is_some());
		assert!(avatars.texture_id("hidden").is_none());
		assert!(avatars.texture_id("emoji-2").is_none());
		assert_eq!(avatars.bytes, 64 * 64 * 4);
		assert_eq!(avatars.emoji_bytes, 64 * 64 * 4);
		assert!(avatars.next_release().unwrap() > now);
		avatars.request("hidden".into());
		assert_eq!(avatars.take_requests(), vec!["hidden"]);
		avatars.accept(
			&ctx,
			"hidden".into(),
			Some(ColorImage::filled([64, 64], egui::Color32::WHITE)),
		);
		assert!(avatars.texture_id("hidden").is_some());
	}

	#[test]
	fn cached_composer_artwork_refreshes_when_painted() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		avatars.request("emoji-1".into());
		avatars.accept(
			&ctx,
			"emoji-1".into(),
			Some(ColorImage::filled([64, 64], egui::Color32::WHITE)),
		);
		let mut layout = crate::composer_text::Layout::default();
		let mut text =
			"<:synthetic:1> [sticker](https://media.discordapp.net/stickers/2.gif)".to_owned();
		avatars.set_animation(true);
		avatars.request("anim:sticker-2-4".into());
		avatars.accept(
			&ctx,
			"anim:sticker-2-4".into(),
			Some(ColorImage::filled([64, 64], egui::Color32::WHITE)),
		);
		avatars.accept_animation(
			"anim:sticker-2-4".into(),
			vec![
				(
					Duration::from_millis(100),
					std::sync::Arc::new(ColorImage::filled([64, 64], egui::Color32::WHITE)),
				),
				(
					Duration::from_millis(100),
					std::sync::Arc::new(ColorImage::filled([64, 64], egui::Color32::BLACK)),
				),
			],
		);
		avatars.take_requests();
		let now = Instant::now();
		let played = now - IDLE_FRAMES;
		avatars
			.animations
			.get_mut("anim:sticker-2-4")
			.unwrap()
			.played = played;
		ctx.run_ui(Default::default(), |ui| {
			let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width| {
				layout.galley(
					ui,
					buffer.as_str(),
					width,
					&[],
					&[],
					&[],
					false,
					&mut avatars,
					false,
				)
			};
			let edit = egui::TextEdit::multiline(&mut text)
				.layouter(&mut layouter)
				.show(ui);
			avatars.emoji_textures.get_mut("emoji-1").unwrap().0 = now - IDLE_TEXTURES;
			avatars.textures.get_mut("anim:sticker-2-4").unwrap().0 = now - IDLE_TEXTURES;
			layout.paint(ui, &edit, &mut avatars);
		})
		.drop_without_applying_deltas();
		assert_eq!(avatars.animations["anim:sticker-2-4"].played, played);
		avatars.release_idle(now);
		assert!(avatars.texture_id("emoji-1").is_some());
		assert!(avatars.texture_id("anim:sticker-2-4").is_some());
		assert!(!avatars.animations.contains_key("anim:sticker-2-4"));
		assert!(avatars.take_requests().is_empty());
	}

	#[test]
	fn clipped_composer_rebuilds_neither_refresh_nor_reload_artwork() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		for key in ["emoji-1", "embed:sticker-2-1"] {
			avatars.request(key.into());
			avatars.take_requests();
			avatars.accept(
				&ctx,
				key.into(),
				Some(ColorImage::filled([64, 64], egui::Color32::WHITE)),
			);
		}
		let now = Instant::now();
		for (used, _) in avatars
			.textures
			.values_mut()
			.chain(avatars.emoji_textures.values_mut())
		{
			*used = now - IDLE_TEXTURES;
		}
		let mut layout = crate::composer_text::Layout::default();
		let mut text =
			"<:synthetic:1> [sticker](https://cdn.discordapp.com/stickers/2.png)".to_owned();
		for _ in 0..3 {
			avatars.revision += 1; // Unrelated arrivals force the long draft to relayout.
			ctx.run_ui(Default::default(), |ui| {
				ui.set_clip_rect(egui::Rect::NOTHING);
				let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width| {
					layout.galley(
						ui,
						buffer.as_str(),
						width,
						&[],
						&[],
						&[],
						false,
						&mut avatars,
						false,
					)
				};
				let edit = egui::TextEdit::multiline(&mut text)
					.layouter(&mut layouter)
					.show(ui);
				layout.paint(ui, &edit, &mut avatars);
			})
			.drop_without_applying_deltas();
			avatars.release_idle(now);
			assert_eq!(avatars.bytes + avatars.emoji_bytes, 0);
			assert!(
				avatars.take_requests().is_empty(),
				"clipped layout must not reload expired artwork"
			);
		}
		// A cached layout returning to the viewport must request the missing artwork.
		ctx.run_ui(Default::default(), |ui| {
			let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width| {
				layout.galley(
					ui,
					buffer.as_str(),
					width,
					&[],
					&[],
					&[],
					false,
					&mut avatars,
					false,
				)
			};
			let edit = egui::TextEdit::multiline(&mut text)
				.layouter(&mut layouter)
				.show(ui);
			layout.paint(ui, &edit, &mut avatars);
		})
		.drop_without_applying_deltas();
		let mut requests = avatars.take_requests();
		requests.sort();
		assert_eq!(requests, ["embed:sticker-2-1", "emoji-1"]);
	}

	#[test]
	fn clipped_timeline_artwork_expires_without_reloading() {
		for (text, key) in [
			("<:synthetic:1>", "emoji-1".to_owned()),
			(
				"[sticker](https://cdn.discordapp.com/stickers/2.png)",
				"embed:sticker-2-1".to_owned(),
			),
			(
				"🙂",
				format!("emoji-unicode-{}-64", crate::emoji::lookup("🙂").unwrap()),
			),
		] {
			let ctx = egui::Context::default();
			let mut avatars = Avatars::default();
			avatars.request(key.clone());
			avatars.take_requests();
			avatars.accept(
				&ctx,
				key.clone(),
				Some(ColorImage::filled([64, 64], egui::Color32::WHITE)),
			);
			let now = Instant::now();
			for (used, _) in avatars
				.textures
				.values_mut()
				.chain(avatars.emoji_textures.values_mut())
			{
				*used = now - IDLE_TEXTURES;
			}
			for _ in 0..2 {
				ctx.run_ui(Default::default(), |ui| {
					crate::design::jumbo_emoji(ui);
					ui.set_clip_rect(egui::Rect::from_min_size(
						ui.min_rect().min,
						egui::vec2(100.0, 0.0),
					));
					ui.add_space(100.0);
					crate::markdown::Formatted::parse(text).show_with_images(
						ui,
						&mut None,
						&[],
						None,
						&mut crate::profiles::ProfileSession::default(),
						(&mut avatars, false, &[]),
						crate::design::MessageCardSurface::Opaque,
					);
				})
				.drop_without_applying_deltas();
				avatars.release_idle(now);
				assert_eq!(avatars.bytes + avatars.emoji_bytes, 0);
				assert!(avatars.take_requests().is_empty());
			}
		}
	}

	#[test]
	#[ignore = "release idle-artwork workload; synthetic textures, no network or window"]
	fn idle_artwork_workload() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		for index in 0..932 {
			let key = if index < 512 {
				format!("avatar-{index}")
			} else {
				format!("emoji-{index}")
			};
			avatars.request(key.clone());
			avatars.accept(
				&ctx,
				key,
				Some(ColorImage::filled([64, 64], egui::Color32::WHITE)),
			);
		}
		assert_eq!(avatars.textures.len(), REQUESTS);
		assert_eq!(avatars.emoji_textures.len(), 0);
		let peak = avatars.bytes + avatars.emoji_bytes;
		avatars.release_idle(Instant::now() + Duration::from_secs(61));
		println!(
			"idle-artwork: peak_bytes={peak} settled_bytes={} textures={}",
			avatars.bytes + avatars.emoji_bytes,
			avatars.textures.len() + avatars.emoji_textures.len()
		);
	}

	#[test]
	fn stickers_obey_animation_preferences_and_demo_stays_offline() {
		let ctx = egui::Context::default();
		let sticker = model::Sticker {
			id: model::Id(7),
			name: "Synthetic wave".into(),
			description: String::new(),
			tags: String::new(),
			format_type: 2,
			guild_id: None,
			pack_id: None,
			available: true,
		};
		let mut images = super::Avatars::default();
		for (enabled, prefix) in [(false, "embed"), (true, "anim")] {
			images.set_animation(enabled);
			ctx.run_ui(Default::default(), |ui| {
				images.sticker_image(ui, &sticker, egui::Vec2::splat(160.0), false);
			})
			.drop_without_applying_deltas();
			assert_eq!(
				images.take_requests(),
				vec![format!("{prefix}:sticker-7-2")]
			);
			images.accept(&ctx, format!("{prefix}:sticker-7-2"), None);
			let output = ctx.run_ui(Default::default(), |ui| {
				images.sticker_image(ui, &sticker, egui::Vec2::splat(160.0), false);
			});
			assert!(
				!output
					.shapes
					.iter()
					.any(|shape| matches!(&shape.shape, egui::Shape::Text(_)))
			);
			output.drop_without_applying_deltas();
			assert!(images.take_requests().is_empty());
		}
		let mut demo = super::Avatars::default();
		ctx.run_ui(Default::default(), |ui| {
			demo.sticker_image(ui, &sticker, egui::Vec2::splat(160.0), true);
		})
		.drop_without_applying_deltas();
		assert!(demo.take_requests().is_empty());
		assert_eq!(demo.textures.len(), 1);
		assert_eq!(demo.bytes, 128 * 128 * 4);
	}
	use super::*;
	#[test]
	fn placeholder_paints_decoded_thumbhash_while_the_image_is_requested() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let media = model::EmbedMedia {
			url: Some("https://cdn.discordapp.com/attachments/1/2/a.png".into()),
			width: 230,
			height: 320,
			placeholder: vec![
				0xd5, 0x07, 0x12, 0x1d, 0x04, 0x67, 0x87, 0x8f, 0x77, 0x57, 0x87, 0x48, 0x87, 0x87,
				0x97, 0x87, 0x58, 0x78, 0x90, 0x95, 0x08,
			],
			..Default::default()
		};
		let mut output = ctx.run_ui(Default::default(), |ui| {
			images.show_media(ui, &media, egui::vec2(320.0, 320.0), false, Surface::Inline);
		});
		output.textures_delta.clear();
		let key = "thumb:d507121d0467878f77578748878797875878909508";
		assert!(images.texture_id(key).is_some());
		assert_eq!(images.textures[key].1.size(), [23, 32]);
		// The real rendition is still requested; the placeholder only fills the wait.
		let requests = images.take_requests();
		assert!(
			requests
				.iter()
				.any(|request| request.starts_with("media:is:"))
		);
		assert!(!requests.iter().any(|request| request.starts_with("thumb:")));
		// Garbage never becomes a texture and is not retried every frame.
		let broken = model::EmbedMedia {
			placeholder: vec![0xff; 6],
			..media.clone()
		};
		for _ in 0..2 {
			let mut output = ctx.run_ui(Default::default(), |ui| {
				images.show_media(
					ui,
					&broken,
					egui::vec2(320.0, 320.0),
					false,
					Surface::Inline,
				);
			});
			output.textures_delta.clear();
		}
		assert_eq!(images.textures.len(), 1);
		assert!(images.attempts["thumb:ffffffffffff"].1);
	}
	#[test]
	fn gif_picker_requests_animation_advances_frames_and_respects_setting() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		images.set_animation(true);
		let gif = model::Gif {
			id: "test".into(),
			title: "Synthetic".into(),
			url: "https://static.klipy.com/synthetic/test.gif".into(),
			preview: "https://static.klipy.com/synthetic/preview.gif".into(),
			width: 2,
			height: 2,
		};
		let embed = model::Embed {
			kind: "gifv".into(),
			thumbnail: Some(model::EmbedMedia {
				url: Some(gif.preview.clone()),
				width: 2,
				height: 2,
				..Default::default()
			}),
			..Default::default()
		};
		let mut output = ctx.run_ui(Default::default(), |ui| {
			images.show_gif_embed(ui, &embed, Some(&gif), egui::vec2(320.0, 320.0), false);
		});
		output.textures_delta.clear();
		assert!(images.gif_texture(&ctx, &gif, false).is_none());
		let key = images.take_requests().pop().unwrap();
		assert_eq!(key, format!("anim:{}", gif.preview));
		let first = ColorImage::filled([2, 2], egui::Color32::RED);
		images.accept(&ctx, key.clone(), Some(first.clone()));
		images.accept_animation(
			key.clone(),
			vec![
				(Duration::from_secs(1), std::sync::Arc::new(first)),
				(
					Duration::from_secs(1),
					std::sync::Arc::new(ColorImage::filled([2, 2], egui::Color32::BLUE)),
				),
			],
		);
		images.animations.get_mut(&key).unwrap().started =
			Instant::now() - Duration::from_millis(1500);
		let mut output = ctx.run_ui(
			egui::RawInput {
				focused: true,
				..Default::default()
			},
			|_ui| {
				assert!(images.gif_texture(&ctx, &gif, false).is_some());
			},
		);
		output.textures_delta.clear();
		assert_eq!(images.animations[&key].frame, 1);
		// A different source frame cannot upload again before the existing deadline.
		let animation = images.animations.get_mut(&key).unwrap();
		animation.started = Instant::now();
		animation.next_upload = Instant::now() + ANIMATION_INTERVAL;
		let deadline = animation.next_upload;
		images.gif_texture(&ctx, &gif, false);
		assert_eq!(images.animations[&key].frame, 1);
		assert_eq!(images.animations[&key].next_upload, deadline);
		images.set_animation(false);
		assert!(images.animations.is_empty());
		assert!(images.gif_texture(&ctx, &gif, false).is_none());
		assert_eq!(images.take_requests(), vec![format!("gif:{}", gif.preview)]);
	}

	#[test]
	fn media_keys_preserve_signed_queries_and_rendition_dimensions() {
		let source = "https://cdn.discordapp.com/attachments/1/2/a.png?ex=abc&is=def&hm=a%2fb%2Bc+d&width=99&%68eight=88&width=1&tag=x&tag=y&empty=&quality=lossless";
		let media = model::EmbedMedia {
			url: Some(source.into()),
			width: 4096,
			height: 1024,
			..Default::default()
		};
		let canonical = "https://cdn.discordapp.com/attachments/1/2/a.png?ex=abc&is=def&hm=a%2Fb%2Bc+d&tag=x&tag=y&empty=";
		let keys = media_requests(&media, false, Surface::Viewer);
		assert_eq!(keys, [format!("media:vs:128x32:{canonical}")]);
		assert_eq!(
			media_requests(&media, false, Surface::Inline),
			[format!("media:is:128x32:{canonical}")]
		);
		let renewed = model::EmbedMedia {
			url: Some(source.replace("ex=abc", "ex=renewed")),
			..media.clone()
		};
		assert_ne!(keys, media_requests(&renewed, false, Surface::Viewer));
		for (width, height) in [(0, 1024), (4096, 0)] {
			let unknown = model::EmbedMedia {
				width,
				height,
				..media.clone()
			};
			// Unknown dimensions request the largest rendition instead of a thumbnail.
			assert_eq!(
				media_requests(&unknown, false, Surface::Viewer),
				[format!("media:vs:e4096:{canonical}")]
			);
		}
		let small = model::EmbedMedia {
			width: 64,
			height: 32,
			..media
		};
		assert_eq!(
			media_requests(&small, false, Surface::Viewer),
			[format!("media:vs:64x32:{canonical}")]
		);

		{
			let original = "https://media.tenor.com/synthetic/clip.gif";
			let proxy = "https://media.discordapp.net/attachments/1/2/clip.WeBp?hm=signed";
			let mut media = model::EmbedMedia {
				url: Some(original.into()),
				proxy_url: Some(proxy.into()),
				width: 1024,
				height: 512,
				..Default::default()
			};
			assert_eq!(
				media_requests(&media, true, Surface::Viewer),
				[format!("media:va:128x64:{original}")]
			);
			assert_eq!(
				media_requests(&media, false, Surface::Viewer),
				[format!("media:vs:128x64:{proxy}")]
			);
			// A non-provider original never overrides the service proxy.
			media.url = Some("https://example.test/clip.gif".into());
			assert_eq!(
				media_requests(&media, true, Surface::Viewer),
				[format!("media:va:128x64:{proxy}")]
			);
			media.proxy_url = Some(proxy.replace(".WeBp", ".GIF"));
			assert_eq!(
				media_requests(&media, true, Surface::Viewer),
				[format!(
					"media:va:128x64:{}",
					media.proxy_url.as_deref().unwrap()
				)]
			);
			// Preserve fragments/credentials/ports for the download worker's rejection.
			for source in [
				"https://user:pass@media.discordapp.net:444/attachments/1/2/a.png?hm=signed#fragment",
				"http://media.discordapp.net.evil.test/attachments/1/2/a.png?hm=signed#fragment",
			] {
				media.proxy_url = Some(source.into());
				assert_eq!(
					media_requests(&media, false, Surface::Inline),
					[format!("media:is:128x64:{source}")]
				);
			}
			media.proxy_url = Some("not a URL".into());
			assert_eq!(
				media_requests(&media, false, Surface::Inline),
				["media:is:128x64:not a URL"]
			);
			media.proxy_url = Some(format!("https://example.test/{}", "a".repeat(2027)));
			assert_eq!(media.proxy_url.as_ref().unwrap().len(), 2048);
			let bound = 2048 + "media:is:4096x4096:".len();
			media.width = 0;
			assert!(media_requests(&media, false, Surface::Inline)[0].len() <= bound);
			media.width = 1024;
			assert!(
				media_requests(&media, false, Surface::Inline)[0].len() <= bound,
				"Sized keys keep the request length bound"
			);
			media.proxy_url.as_mut().unwrap().push('a');
			assert!(
				media_requests(&media, false, Surface::Viewer).is_empty(),
				"An oversized proxy must not fall back to the original"
			);
			media.url = None;
			media.proxy_url = None;
			assert!(media_requests(&media, true, Surface::Viewer).is_empty());
		}
	}

	fn media_requests(media: &model::EmbedMedia, animate: bool, surface: Surface) -> Vec<String> {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		images.set_animation(animate);
		let mut frame = || {
			ctx.run_ui(Default::default(), |ui| {
				images.show_media(ui, media, egui::vec2(100.0, 80.0), false, surface);
			})
			.drop_without_applying_deltas();
			images.take_requests()
		};
		let requests = frame();
		assert!(
			frame().is_empty(),
			"Pending media must not be requested again"
		);
		requests
	}

	#[test]
	fn media_viewer_uses_cached_thumbnail_until_large_pixels_arrive() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let media = model::EmbedMedia {
			url: Some("https://cdn.discordapp.com/attachments/1/2/a.png?hm=signed".into()),
			width: 4096,
			height: 2048,
			..Default::default()
		};
		let frame = |images: &mut Avatars, surface: Surface, size: egui::Vec2| {
			let mut quality = None;
			let output = ctx.run_ui(Default::default(), |ui| {
				quality = Some(images.show_media(ui, &media, size, false, surface).quality);
			});
			(output, quality)
		};
		let (output, quality) = frame(&mut images, Surface::Inline, egui::vec2(40.0, 40.0));
		output.drop_without_applying_deltas();
		assert_eq!(quality, Some(Quality::Placeholder { failed: false }));
		let chat = images.take_requests();
		assert_eq!(
			chat,
			[format!("media:is:64x32:{}", media.url.as_ref().unwrap())]
		);
		images.accept(
			&ctx,
			chat[0].clone(),
			Some(ColorImage::filled([2, 1], egui::Color32::WHITE)),
		);
		let chat_texture = images.texture_id(&chat[0]).unwrap();
		let (output, quality) = frame(&mut images, Surface::Viewer, egui::vec2(100.0, 80.0));
		assert_eq!(quality, Some(Quality::Upgrading));
		assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
			egui::Shape::Rect(rect) if rect.fill_texture_id() == chat_texture)));
		output.drop_without_applying_deltas();
		let viewer = images.take_requests();
		assert_eq!(
			viewer,
			[format!("media:vs:128x64:{}", media.url.as_ref().unwrap())]
		);
		images.accept(
			&ctx,
			viewer[0].clone(),
			Some(ColorImage::filled([4, 2], egui::Color32::WHITE)),
		);
		let texture = images.texture_id(&viewer[0]).unwrap();
		let (output, quality) = frame(&mut images, Surface::Viewer, egui::vec2(100.0, 80.0));
		assert_eq!(quality, Some(Quality::Full));
		assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
			egui::Shape::Rect(rect) if rect.fill_texture_id() == texture)));
		output.drop_without_applying_deltas();
		assert!(images.take_requests().is_empty());
		assert!(images.textures.is_empty());
		assert_eq!(images.media.bytes(), (2 + 8) * 4);
	}

	#[test]
	fn inline_gif_frames_release_when_unplayed_and_reload_once_playable() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		images.set_animation(true);
		let media = model::EmbedMedia {
			url: Some("https://cdn.discordapp.com/attachments/1/2/a.gif?hm=signed".into()),
			width: 64,
			height: 32,
			..Default::default()
		};
		let paint = |images: &mut Avatars, focused: bool| {
			ctx.run_ui(
				egui::RawInput {
					focused,
					..Default::default()
				},
				|ui| {
					images.show_media(ui, &media, egui::vec2(64.0, 32.0), false, Surface::Inline);
				},
			)
			.drop_without_applying_deltas();
		};
		let deliver = |images: &mut Avatars, key: &str| {
			let frame = |color| std::sync::Arc::new(ColorImage::filled([2, 1], color));
			images.accept(
				&ctx,
				key.to_owned(),
				Some(ColorImage::filled([2, 1], egui::Color32::RED)),
			);
			images.accept_animation(
				key.to_owned(),
				vec![
					(Duration::from_secs(1), frame(egui::Color32::RED)),
					(Duration::from_secs(1), frame(egui::Color32::BLUE)),
				],
			);
		};
		paint(&mut images, true);
		let key = images.take_requests().pop().unwrap();
		let rendition = media::Rendition::parse(&key).unwrap();
		deliver(&mut images, &key);
		assert!(images.media.animation(&rendition).is_some());
		let playing = images.media.bytes();
		images.release_idle(Instant::now());
		assert!(
			images.media.animation(&rendition).is_some(),
			"Recently played frames stay"
		);
		assert!(images.next_release().is_some());
		// Still on screen, but an unfocused window paints without playing it.
		paint(&mut images, false);
		images.release_idle(Instant::now() + IDLE_FRAMES);
		assert!(images.media.animation(&rendition).is_none());
		assert!(images.media.bytes() < playing);
		assert!(images.texture_id(&key).is_some(), "The still is kept");
		assert!(
			images.next_release().is_some(),
			"The still has its own expiry"
		);
		paint(&mut images, false);
		assert!(
			images.take_requests().is_empty(),
			"Frames are not decoded again while they cannot play"
		);
		// Focus (or scrolling back) shows the still and asks the disk cache for frames again.
		paint(&mut images, true);
		assert_eq!(images.take_requests(), vec![key.clone()]);
		deliver(&mut images, &key);
		assert!(images.media.animation(&rendition).is_some());
	}

	#[test]
	fn hover_only_avatar_frames_load_on_hover_and_release_when_idle() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		images.set_animation(true);
		let key = "123-a_0123456789abcdef0123456789abcdef".to_owned();
		let rect = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(32.0, 32.0));
		let paint = |images: &mut Avatars, hovered: bool| {
			ctx.run_ui(
				egui::RawInput {
					focused: true,
					events: vec![egui::Event::PointerMoved(if hovered {
						rect.center()
					} else {
						egui::pos2(500.0, 500.0)
					})],
					..Default::default()
				},
				|ui| {
					images.paint(ui, &key, rect, 4);
				},
			)
			.drop_without_applying_deltas();
		};
		let deliver = |images: &mut Avatars| {
			let frame = |color| std::sync::Arc::new(ColorImage::filled([2, 2], color));
			images.accept(
				&ctx,
				key.clone(),
				Some(ColorImage::filled([2, 2], egui::Color32::RED)),
			);
			images.accept_animation(
				key.clone(),
				vec![
					(Duration::from_secs(1), frame(egui::Color32::RED)),
					(Duration::from_secs(1), frame(egui::Color32::BLUE)),
				],
			);
		};
		paint(&mut images, false);
		assert_eq!(images.take_requests(), vec![key.clone()]);
		deliver(&mut images);
		assert!(
			images.animations.is_empty(),
			"An avatar that is not hovered keeps only its still"
		);
		assert!(images.texture_id(&key).is_some());
		paint(&mut images, false);
		assert!(
			images.take_requests().is_empty(),
			"Declined frames are not decoded again until they would play"
		);
		paint(&mut images, true);
		assert_eq!(images.take_requests(), vec![key.clone()]);
		deliver(&mut images);
		assert!(images.animations.contains_key(&key));
		images.release_idle(Instant::now() + IDLE_FRAMES);
		assert!(images.animations.is_empty());
		paint(&mut images, false);
		assert!(images.take_requests().is_empty());
	}

	#[test]
	fn animated_profile_avatar_and_banner_viewer_requests_anim_and_plays() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		images.set_animation(true);
		let avatar_media = model::EmbedMedia {
			url: Some("https://cdn.discordapp.com/avatars/123/a_0123456789abcdef0123456789abcdef.gif?size=2048".into()),
			width: 2048,
			height: 2048,
			..Default::default()
		};
		let mut rect = egui::Rect::NOTHING;
		ctx.run_ui(Default::default(), |ui| {
			rect = images
				.show_media(
					ui,
					&avatar_media,
					egui::vec2(100.0, 80.0),
					false,
					Surface::Viewer,
				)
				.response
				.rect;
		})
		.drop_without_applying_deltas();
		let keys = images.take_requests();
		assert_eq!(
			keys,
			vec![
				"media:va:128x128:https://cdn.discordapp.com/avatars/123/a_0123456789abcdef0123456789abcdef.gif"
			]
		);
		let key = keys[0].clone();
		let rendition = media::Rendition::parse(&key).unwrap();
		let first = ColorImage::filled([2, 2], egui::Color32::RED);
		images.accept(&ctx, key.clone(), Some(first.clone()));
		images.accept_animation(
			key.clone(),
			vec![
				(Duration::from_secs(1), std::sync::Arc::new(first)),
				(
					Duration::from_secs(1),
					std::sync::Arc::new(ColorImage::filled([2, 2], egui::Color32::BLUE)),
				),
			],
		);
		images.media.animation(&rendition).unwrap().started =
			Instant::now() - Duration::from_millis(1500);
		ctx.run_ui(
			egui::RawInput {
				focused: true,
				events: vec![egui::Event::PointerMoved(rect.center())],
				..Default::default()
			},
			|ui| {
				images.show_media(
					ui,
					&avatar_media,
					egui::vec2(100.0, 80.0),
					false,
					Surface::Viewer,
				);
			},
		)
		.drop_without_applying_deltas();
		assert_eq!(images.media.animation(&rendition).unwrap().frame, 1);
	}

	#[test]
	fn gifv_falls_back_to_its_poster_when_the_clip_cannot_decode() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		images.set_animation(true);
		let poster = "https://media.discordapp.net/external/a/https/static.klipy.com/poster.png";
		let embed = model::Embed {
			kind: "gifv".into(),
			thumbnail: Some(model::EmbedMedia {
				url: Some(poster.into()),
				width: 64,
				height: 32,
				..Default::default()
			}),
			video: Some(model::EmbedMedia {
				url: Some("https://static.klipy.com/clip.mp4".into()),
				width: 64,
				height: 32,
				..Default::default()
			}),
			..Default::default()
		};
		let frame = |images: &mut Avatars| {
			ctx.run_ui(Default::default(), |ui| {
				images.show_gif_embed(ui, &embed, None, egui::vec2(320.0, 320.0), false);
			})
			.drop_without_applying_deltas();
			images.take_requests()
		};
		let clip = frame(&mut images);
		assert_eq!(
			clip,
			["media:ia:64x32:https://static.klipy.com/clip.mp4".to_owned()]
		);
		images.accept(&ctx, clip[0].clone(), None);
		let fallback = frame(&mut images);
		assert_eq!(fallback, [format!("media:is:64x32:{poster}")]);
		// The rejected clip is never requested again.
		assert!(!frame(&mut images).iter().any(|key| key.ends_with(".mp4")));
	}

	#[test]
	fn closing_the_viewer_releases_its_pixels_on_the_next_frame() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let media = model::EmbedMedia {
			url: Some("https://cdn.discordapp.com/attachments/1/2/a.png".into()),
			width: 64,
			height: 32,
			..Default::default()
		};
		let frame = |images: &mut Avatars, viewer: bool| {
			ctx.run_ui(Default::default(), |ui| {
				if viewer {
					images.show_media(ui, &media, egui::vec2(100.0, 80.0), false, Surface::Viewer);
				}
			})
			.drop_without_applying_deltas();
			images.take_requests()
		};
		let key = frame(&mut images, true).pop().unwrap();
		images.accept(
			&ctx,
			key,
			Some(ColorImage::filled([64, 32], egui::Color32::WHITE)),
		);
		frame(&mut images, true);
		assert_eq!(images.media.bytes(), 64 * 32 * 4);
		frame(&mut images, false);
		assert_eq!(images.media.bytes(), 0);
	}

	#[test]
	fn closing_viewer_releases_full_resolution_pixels_after_inline_repaint() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let media = model::EmbedMedia {
			url: Some("https://cdn.discordapp.com/attachments/1/2/a.png".into()),
			width: 4096,
			height: 2048,
			..Default::default()
		};
		let frame = |images: &mut Avatars, surface: Surface| {
			ctx.run_ui(Default::default(), |ui| {
				let size = if surface == Surface::Viewer {
					egui::vec2(100.0, 80.0)
				} else {
					egui::vec2(40.0, 40.0)
				};
				images.show_media(ui, &media, size, false, surface);
			})
			.drop_without_applying_deltas();
			images.take_requests()
		};
		// A viewer request can finish before the inline thumbnail already in flight.
		let inline = frame(&mut images, Surface::Inline).pop().unwrap();
		let viewer = frame(&mut images, Surface::Viewer).pop().unwrap();
		assert_ne!(inline, viewer);
		images.accept(
			&ctx,
			viewer.clone(),
			Some(ColorImage::filled([4, 2], egui::Color32::WHITE)),
		);
		frame(&mut images, Surface::Viewer);
		assert_eq!(images.media.bytes(), 4 * 2 * 4);
		// The timeline paints its inline image before the closed viewer is swept.
		frame(&mut images, Surface::Inline);
		assert!(images.texture_id(&viewer).is_none());
		assert_eq!(images.media.bytes(), 0);
		images.accept(
			&ctx,
			inline.clone(),
			Some(ColorImage::filled([2, 1], egui::Color32::WHITE)),
		);
		frame(&mut images, Surface::Inline);
		assert!(images.texture_id(&inline).is_some());
		assert_eq!(images.media.bytes(), 2 * 4);
	}

	/// Synthetic chat scroll past GIF embeds beside a member list of animated avatars, then a
	/// settled view. Prints retained decoded frame bytes and process RSS; no window, GPU, network
	/// or account access.
	#[test]
	#[ignore = "release memory workload; prints retained frame bytes and RSS"]
	fn animation_memory_workload() {
		let rss = || {
			let output = std::process::Command::new("ps")
				.args(["-o", "rss=", "-p", &std::process::id().to_string()])
				.output()
				.unwrap();
			String::from_utf8_lossy(&output.stdout)
				.trim()
				.parse::<f64>()
				.unwrap() / 1024.0
		};
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		images.set_animation(true);
		// A typical provider GIF and Nitro avatar: 498x280 and 128x128, 40 frames at 50 ms.
		let gifs: Vec<_> = (0..12)
			.map(|index| model::EmbedMedia {
				url: Some(format!(
					"https://cdn.discordapp.com/attachments/1/{index}/clip.gif?hm=signed"
				)),
				width: 498,
				height: 280,
				..Default::default()
			})
			.collect();
		let avatars: Vec<String> = (0..60)
			.map(|index| format!("{}-a_{index:032x}", 1000 + index))
			.collect();
		let frames = |width: usize, height: usize| -> GifFrames {
			(0..40)
				.map(|index| {
					(
						Duration::from_millis(50),
						std::sync::Arc::new(ColorImage::filled(
							[width, height],
							egui::Color32::from_gray(index as u8),
						)),
					)
				})
				.collect()
		};
		let frame = |images: &mut Avatars, shown: &[usize], members: &[usize]| {
			ctx.run_ui(
				egui::RawInput {
					focused: true,
					..Default::default()
				},
				|ui| {
					for &index in shown {
						images.show_media(
							ui,
							&gifs[index],
							egui::vec2(498.0, 280.0),
							false,
							Surface::Inline,
						);
					}
					for &index in members {
						let rect = egui::Rect::from_min_size(
							egui::pos2(0.0, index as f32 * 40.0),
							egui::vec2(32.0, 32.0),
						);
						images.paint(ui, &avatars[index], rect, 16);
					}
				},
			)
			.drop_without_applying_deltas();
			for key in images.take_requests() {
				let (width, height, limit) = match media::Rendition::parse(&key) {
					Some(rendition) => (498, 280, rendition.size.longest() as usize),
					None => (128, 128, 128),
				};
				let (width, height) = (width.min(limit), height.min(limit));
				images.accept(
					&ctx,
					key.clone(),
					Some(ColorImage::filled([width, height], egui::Color32::GRAY)),
				);
				images.accept_animation(key, frames(width, height));
			}
		};
		let retained = |images: &Avatars| {
			(images.media.bytes() + images.animations.values().map(|a| a.bytes).sum::<usize>())
				as f64 / (1024.0 * 1024.0)
		};
		let start = rss();
		let mut peak = (0.0f64, 0.0f64);
		// Two embeds and twenty member rows are on screen at a time while scrolling.
		for step in 0..11 {
			let members: Vec<usize> = (step * 4..(step * 4 + 20).min(60)).collect();
			frame(&mut images, &[step, step + 1], &members);
			frame(&mut images, &[step, step + 1], &members);
			peak = (peak.0.max(retained(&images)), peak.1.max(rss()));
		}
		std::thread::sleep(Duration::from_secs(6));
		let settled: Vec<usize> = (40..60).collect();
		frame(&mut images, &[10, 11], &settled);
		println!(
			"animation memory workload: start_rss={start:.1}MiB peak_retained={:.1}MiB peak_rss={:.1}MiB settled_retained={:.1}MiB settled_rss={:.1}MiB",
			peak.0,
			peak.1,
			retained(&images),
			rss()
		);
	}

	/// Offline settled media frames; no window, GPU, network, or account access.
	#[test]
	#[ignore = "release media workload; one warmup and five measured batches"]
	fn media_frame_benchmark() {
		for (scenario, surface, animate) in [
			("thumbnail", Surface::Inline, false),
			("viewer", Surface::Viewer, false),
			("animated", Surface::Viewer, true),
		] {
			let ctx = egui::Context::default();
			let mut images = Avatars::default();
			images.set_animation(animate);
			let media: Vec<_> = (0..12)
				.map(|id| model::EmbedMedia {
					url: Some(format!(
						"https://cdn.discordapp.com/attachments/1/{}/image.{}?ex=abc&is=def&hm=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef&format=webp&width=4096&height=2048&quality=lossless",
						id + 1,
						if animate { "WeBp" } else { "png" }
					)),
					width: 4096,
					height: 2048,
					..Default::default()
				})
				.collect();
			let frame = |images: &mut Avatars| {
				ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(1200.0, 300.0),
						)),
						..Default::default()
					},
					|ui| {
						ui.horizontal(|ui| {
							for media in &media {
								std::hint::black_box(
									images
										.show_media(
											ui,
											media,
											egui::vec2(48.0, 32.0),
											false,
											surface,
										)
										.quality,
								);
							}
						});
					},
				)
				.drop_without_applying_deltas();
			};
			frame(&mut images);
			let requests = images.take_requests();
			assert_eq!(requests.len(), 12);
			for key in requests {
				images.accept(
					&ctx,
					key,
					Some(ColorImage::filled([4, 2], egui::Color32::WHITE)),
				);
			}
			for sample in 0..6 {
				let started = Instant::now();
				for _ in 0..1000 {
					frame(&mut images);
				}
				let elapsed = started.elapsed();
				assert!(images.take_requests().is_empty());
				if sample > 0 {
					println!(
						"media scenario={scenario} frames=1000 images=12 sample={sample} elapsed_ms={:.3}",
						elapsed.as_secs_f64() * 1000.0
					);
				}
			}
		}
	}

	#[test]
	fn pending_requests_survive_retry_and_capacity_until_resolved() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		let expired = Instant::now() - RETRY - Duration::from_secs(1);
		for i in 0..2048 {
			avatars.attempts.insert(i.to_string(), (expired, false));
		}
		avatars.request("0".into());
		avatars.request("new".into());
		assert!(avatars.take_requests().is_empty());
		assert_eq!(avatars.attempts.len(), 2048);
		avatars.accept(&ctx, "0".into(), None);
		assert!(avatars.attempts["0"].0 > expired);
		avatars.request("0".into());
		assert!(avatars.take_requests().is_empty());
		avatars.attempts.get_mut("0").unwrap().0 = expired;
		avatars.request("0".into());
		assert_eq!(avatars.take_requests(), vec!["0"]);
		avatars.accept(
			&ctx,
			"1".into(),
			Some(ColorImage::filled([1, 1], egui::Color32::WHITE)),
		);
		assert!(avatars.texture_id("1").is_some());
		avatars.request("new".into());
		assert_eq!(avatars.take_requests(), vec!["new"]);
	}

	#[test]
	fn texture_requests_and_decoded_memory_stay_bounded() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		for i in 0..256 {
			avatars.request(i.to_string());
		}
		assert_eq!(avatars.take_requests().len(), REQUESTS);
		assert_eq!(avatars.attempts.len(), REQUESTS);
		avatars.request("0".into());
		assert!(avatars.take_requests().is_empty());
		avatars.accept(
			&ctx,
			"0".into(),
			Some(ColorImage::filled([129, 128], egui::Color32::WHITE)),
		);
		assert!(avatars.textures.is_empty());
		for i in 0..TEXTURES * 2 {
			if i >= REQUESTS {
				avatars.request(i.to_string());
				avatars.take_requests();
			}
			avatars.accept(
				&ctx,
				i.to_string(),
				Some(ColorImage::filled([128, 128], egui::Color32::WHITE)),
			);
		}
		assert_eq!(avatars.textures.len(), TEXTURES);
		assert_eq!(
			avatars
				.textures
				.values()
				.map(|(_, texture)| texture.byte_size())
				.sum::<usize>(),
			TEXTURES * 128 * 128 * 4
		);
		avatars.accept(
			&ctx,
			"unsolicited".into(),
			Some(ColorImage::filled([128, 128], egui::Color32::WHITE)),
		);
		assert_eq!(avatars.textures.len(), TEXTURES);
		avatars.request("emoji-9001".into());
		avatars.take_requests();
		avatars.accept(
			&ctx,
			"emoji-9001".into(),
			Some(ColorImage::filled([64, 64], egui::Color32::WHITE)),
		);
		let emoji_texture = avatars.texture_id("emoji-9001").unwrap();
		for index in 0..80 {
			let key = format!("embed:synthetic-{index}");
			avatars.request(key.clone());
			avatars.accept(
				&ctx,
				key,
				Some(ColorImage::filled([512, 512], egui::Color32::WHITE)),
			);
		}
		assert_eq!(avatars.textures.len(), 64);
		assert_eq!(avatars.texture_id("emoji-9001"), Some(emoji_texture));
		assert_eq!(avatars.emoji_bytes, 64 * 64 * 4);
		avatars.take_requests();
		assert!(
			avatars
				.custom_image(&ctx, model::Id(9001), 20.0, false)
				.is_some()
		);
		assert!(avatars.take_requests().is_empty());
		assert_eq!(
			avatars
				.textures
				.values()
				.map(|(_, texture)| texture.byte_size())
				.sum::<usize>(),
			TEXTURE_BYTES
		);
		let mut preview = Avatars::default();
		let guild = model::Guild {
			default_message_notifications: None,
			stickers: None,
			emojis: None,
			id: model::Id(10),
			name: "Synthetic server".into(),
			icon: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
		};
		let mut output = ctx.run_ui(Default::default(), |ui| {
			preview.show_guild_sized(ui, &guild, true, true, 48.0);
		});
		output.textures_delta.clear();
		assert!(preview.take_requests().is_empty());
		assert_eq!(preview.textures.len(), 1);
		assert_eq!(
			preview.textures[&guild.icon_key().unwrap()].1.size(),
			[32, 32]
		);
		avatars.request("x".repeat(2055));
		assert!(avatars.attempts.keys().all(|key| key.len() <= 2054));
	}
}
