//! Visible-avatar requests and a small texture working set. Disk/network work lives outside egui.
use egui::{ColorImage, TextureHandle};
use model::User;
use std::{
	collections::{HashMap, HashSet},
	time::{Duration, Instant},
};

pub type GifFrames = Vec<(Duration, std::sync::Arc<ColorImage>)>;
const ANIMATIONS: usize = 128;
const ANIMATION_BYTES: usize = 128 * 1024 * 1024;
const ANIMATION_INTERVAL: Duration = Duration::from_millis(16);
struct Animation {
	frames: GifFrames,
	texture: Option<TextureHandle>,
	total: Duration,
	started: Instant,
	next_upload: Instant,
	frame: usize,
	bytes: usize,
}

// Emoji artwork has its own working set so media cannot evict it.
const EMOJI_TEXTURES: usize = 1024;
const EMOJI_TEXTURE_BYTES: usize = 16 * 1024 * 1024;
const TEXTURES: usize = 512;
const TEXTURE_BYTES: usize = 64 * 1024 * 1024;
/// Longest edge requested for thumbnails and for the full-screen viewer.
pub const EMBED_EDGE: u32 = 512;
pub const LARGE_EDGE: u32 = 2048;
const REQUESTS: usize = 128;
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
	textures: HashMap<String, (u64, TextureHandle)>,
	emoji_textures: HashMap<String, (u64, TextureHandle)>,
	emoji_bytes: usize,
	avatar_keys: HashMap<model::Id, AvatarKey>,
	clock: u64,
	bytes: usize,
	pub revision: u64,
	attempts: HashMap<String, (Instant, bool)>,
	requests: Vec<String>,
}
/// Scale `width`×`height` so the longer side is at most `edge`, never upscaling.
pub fn fit_edge(width: u32, height: u32, edge: u32) -> (u32, u32) {
	let longest = u64::from(width.max(height)).max(u64::from(edge));
	(
		(u64::from(width) * u64::from(edge) / longest).max(1) as u32,
		(u64::from(height) * u64::from(edge) / longest).max(1) as u32,
	)
}

fn is_animated_profile_or_avatar_key(key: &str) -> bool {
	if let Some(source) = key.strip_prefix("anim:") {
		let path = source.split('?').next().unwrap_or(source);
		if path.ends_with(".gif")
			&& (path.contains("/avatars/") || path.contains("/banners/"))
			&& path.contains("/a_")
		{
			return true;
		}
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
	}
	pub fn set_animation(&mut self, enabled: bool) {
		if self.animate_gifs == enabled {
			return;
		}
		self.animate_gifs = enabled;
		self.no_animations.clear();
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
		if !self.animate_gifs
			|| !self.textures.contains_key(&key)
			|| frames.len() < 2
			|| frames.len() > 200
		{
			if frames.len() < 2 {
				if self.no_animations.len() >= 2048 {
					self.no_animations.clear();
				}
				self.no_animations.insert(key);
			}
			return;
		}
		self.no_animations.remove(&key);
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
					|| image.size[0] > LARGE_EDGE as usize
					|| image.size[1] > LARGE_EDGE as usize
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
				next_upload: Instant::now(),
				frame: usize::MAX,
				bytes,
			},
		);
	}
	pub fn take_requests(&mut self) -> Vec<String> {
		std::mem::take(&mut self.requests)
	}
	fn request(&mut self, key: String) {
		let now = Instant::now();
		self.attempts
			.retain(|_, (at, failed)| !*failed || now.duration_since(*at) < RETRY);
		if key.len() <= 2054
			&& self.attempts.len() < 2048
			&& self.requests.len() < REQUESTS
			&& self
				.attempts
				.get(&key)
				.is_none_or(|(at, failed)| *failed && now.duration_since(*at) >= RETRY)
		{
			self.attempts.insert(key.clone(), (now, false));
			self.requests.push(key);
		}
	}
	pub fn accept(&mut self, ctx: &egui::Context, key: String, image: Option<ColorImage>) {
		if !self.attempts.contains_key(&key) {
			return;
		}
		let limit = if key.starts_with("large:") {
			LARGE_EDGE as usize
		} else if key.starts_with("anim:")
			|| key.starts_with("embed:")
			|| key.starts_with("gif:")
			|| key.starts_with("spotify-")
			|| key.starts_with("banner-")
			|| key.starts_with("member-banner-")
		{
			EMBED_EDGE as usize
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
		self.clock += 1;
		self.revision += 1;
		textures.insert(key, (self.clock, texture));
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
			self.clock += 1;
			entry.0 = self.clock;
			let image = egui::Image::new(&entry.1).fit_to_exact_size(egui::Vec2::splat(size));
			Some(image)
		} else {
			if !demo {
				self.request(key);
			}
			None
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
		let prefix = if self.animate_gifs && matches!(sticker.format_type, 2 | 4) {
			"anim"
		} else {
			"embed"
		};
		let key = format!("{prefix}:sticker-{}-{}", sticker.id, sticker.format_type);
		#[cfg(any(test, feature = "demo"))]
		if demo && !self.textures.contains_key(&key) {
			// Original synthetic mascot, generated locally; never downloaded service artwork.
			let mut image = ColorImage::filled([128, 128], egui::Color32::TRANSPARENT);
			let tint = [
				egui::Color32::from_rgb(103, 192, 177),
				egui::Color32::from_rgb(250, 181, 98),
				egui::Color32::from_rgb(172, 155, 241),
			][sticker.id.0 as usize % 3];
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
			self.accept(ui.ctx(), key.clone(), Some(image));
		}
		if !self.paint(ui, &key, rect, 0) {
			let failed = self.attempts.get(&key).is_some_and(|(_, failed)| *failed);
			let supported = sticker.id.0 != 0 && matches!(sticker.format_type, 1..=4);
			let colors = crate::design::palette(ui);
			let placeholder = rect.shrink(8.0);
			ui.painter().rect_filled(placeholder, 8, colors.raised);
			if failed || !supported {
				response.clone().on_hover_text("Image unavailable");
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
			self.clock += 1;
			entry.0 = self.clock;
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
				self.clock += 1;
				entry.0 = self.clock;
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
			egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), "Server profile picture")
		});
		response
	}
	fn advance_animation(
		&mut self,
		ctx: &egui::Context,
		key: &str,
		hovered: bool,
	) -> Option<TextureHandle> {
		if self.animate_gifs
			&& ctx.input(|input| input.focused)
			&& (hovered || self.avatar_animation || !is_animated_profile_or_avatar_key(key))
			&& let Some(animation) = self.animations.get_mut(key)
		{
			let total_nanos = animation.total.as_nanos();
			if total_nanos == 0 {
				return None;
			}
			let now = Instant::now();
			let elapsed_nanos = (animation.started.elapsed().as_nanos() % total_nanos) as u64;
			let mut elapsed = Duration::from_nanos(elapsed_nanos);
			let mut target_index = 0;
			let mut frame_remaining = Duration::ZERO;
			for (index, (delay, _)) in animation.frames.iter().enumerate() {
				if elapsed < *delay {
					target_index = index;
					frame_remaining = *delay - elapsed;
					break;
				}
				elapsed -= *delay;
			}

			if animation.frame != target_index {
				if now < animation.next_upload {
					ctx.request_repaint_after(animation.next_upload - now);
					return animation.texture.clone();
				}
				let image = &animation.frames[target_index].1;
				if let Some(texture) = &mut animation.texture {
					texture.set(image.clone(), egui::TextureOptions::LINEAR);
				} else {
					animation.texture = Some(ctx.load_texture(
						"service-animation",
						image.clone(),
						egui::TextureOptions::LINEAR,
					));
				}
				animation.frame = target_index;
				animation.next_upload = now + ANIMATION_INTERVAL;
			}

			ctx.request_repaint_after(frame_remaining.max(Duration::from_millis(1)));
			return animation.texture.clone();
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
			{
				self.request(key.to_string());
			}
			animated_texture =
				self.advance_animation(ui.ctx(), key, ui.rect_contains_pointer(rect));
		}
		let Some(entry) = self.textures.get_mut(key) else {
			return false;
		};
		self.clock += 1;
		entry.0 = self.clock;
		// paint_at stretches to its rectangle; fit actual pixels inside the stable layout slot.
		let texture = animated_texture.as_ref().unwrap_or(&entry.1);
		let source = texture.size_vec2();
		if cover {
			let scale = (rect.width() / source.x).max(rect.height() / source.y);
			let uv_size = rect.size() / (source * scale);
			egui::Image::new(texture)
				.uv(egui::Rect::from_center_size(egui::pos2(0.5, 0.5), uv_size))
				.corner_radius(egui::CornerRadius {
					nw: radius,
					ne: radius,
					sw: 0,
					se: 0,
				})
				.paint_at(ui, rect);
		} else {
			let scale = (rect.width() / source.x).min(rect.height() / source.y);
			let fitted = egui::Rect::from_center_size(rect.center(), source * scale);
			egui::Image::new(texture)
				.corner_radius(radius)
				.paint_at(ui, fitted);
		}
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
		self.guild_avatar(ui, guild, selected, demo, 46.0, false)
	}
	pub fn paint_guild(
		&mut self,
		ui: &mut egui::Ui,
		guild: &model::Guild,
		rect: egui::Rect,
		demo: bool,
		radius: u8,
	) {
		self.paint_guild_face(ui, guild, rect, demo, false, radius);
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
		self.paint_guild_face(ui, guild, rect, demo, highlight, (size * 0.29) as u8);
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
		highlight: bool,
		radius: u8,
	) {
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
		if ui.is_rect_visible(rect)
			&& let Some(key) = guild.icon_key()
		{
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
				if highlight {
					colors.accent
				} else {
					colors.raised
				},
			);
			ui.painter().text(
				rect.center(),
				egui::Align2::CENTER_CENTER,
				short,
				egui::FontId::new(initials_size, crate::design::medium_family(ui.ctx())),
				if highlight {
					colors.accent_text
				} else {
					colors.text
				},
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
		let original = gif
			.filter(|gif| self.animate_gifs && gif.url.ends_with(".gif"))
			.map(|gif| model::EmbedMedia {
				url: Some(gif.url.clone()),
				proxy_url: None,
				width: gif.width,
				height: gif.height,
				..Default::default()
			});
		let media = original
			.as_ref()
			.or(embed.image.as_ref())
			.or(embed.thumbnail.as_ref());
		self.show_media(
			ui,
			media.unwrap_or(&model::EmbedMedia::default()),
			size,
			demo,
			false,
			false,
		)
	}

	pub fn show_embed(
		&mut self,
		ui: &mut egui::Ui,
		media: &model::EmbedMedia,
		max_size: egui::Vec2,
		demo: bool,
	) -> egui::Response {
		self.show_media(ui, media, max_size, demo, false, false)
	}
	pub fn show_large(
		&mut self,
		ui: &mut egui::Ui,
		media: &model::EmbedMedia,
		max_size: egui::Vec2,
		demo: bool,
	) -> egui::Response {
		self.show_media(ui, media, max_size, demo, true, false)
	}
	pub fn show_banner(
		&mut self,
		ui: &mut egui::Ui,
		media: &model::EmbedMedia,
		size: egui::Vec2,
		demo: bool,
	) -> egui::Response {
		self.show_media(ui, media, size, demo, false, true)
	}
	#[allow(clippy::too_many_arguments)]
	fn show_media(
		&mut self,
		ui: &mut egui::Ui,
		media: &model::EmbedMedia,
		max_size: egui::Vec2,
		demo: bool,
		large: bool,
		cover: bool,
	) -> egui::Response {
		// Reserve geometry from bounded metadata so image arrivals do not move the reading anchor.
		let max_size = egui::vec2(
			max_size
				.x
				.min(ui.available_width())
				.clamp(1.0, if large { 4096.0 } else { 512.0 }),
			max_size.y.clamp(1.0, if large { 4096.0 } else { 512.0 }),
		);
		let original = if media.width > 0 && media.height > 0 {
			egui::vec2(
				media.width.min(16384) as f32,
				media.height.min(16384) as f32,
			)
		} else {
			egui::vec2(320.0, 180.0)
		};
		let scale = (max_size.x / original.x)
			.min(max_size.y / original.y)
			.min(if large { f32::INFINITY } else { 1.0 });
		let size = if cover {
			max_size
		} else {
			(original * scale).max(egui::vec2(1.0, 1.0))
		};
		let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
		if ui.is_rect_visible(rect) {
			let original_gif = media.url.as_deref().filter(|url| {
				self.animate_gifs
					&& (model::valid_gif_url(url)
						|| url.starts_with("https://cdn.discordapp.com/")
						|| url.starts_with("https://media.discordapp.net/"))
					&& url
						.split('?')
						.next()
						.is_some_and(|path| path.ends_with(".gif"))
			});
			let source = original_gif
				.or(media.proxy_url.as_deref())
				.or(media.url.as_deref())
				.filter(|source| source.len() <= 2048);
			let animated = source.is_some_and(|source| {
				self.animate_gifs
					&& source
						.split('?')
						.next()
						.and_then(|path| path.rsplit_once('.'))
						.is_some_and(|(_, ext)| {
							ext.eq_ignore_ascii_case("gif") || ext.eq_ignore_ascii_case("webp")
						})
			});
			// Parse once for both renditions; borrow query pairs so signatures and their order
			// keep the same URL encoding without an owned string/vector for every parameter.
			let parsed = source
				.filter(|_| original_gif.is_none() && media.width > 0 && media.height > 0)
				.and_then(|source| url::Url::parse(source).ok());
			let sized = |edge: u32, prefix: &str| {
				source.map(|source| {
					if let Some(parsed) = &parsed {
						let mut url = parsed.clone();
						let (width, height) = fit_edge(media.width, media.height, edge);
						url.set_query(None);
						url.query_pairs_mut()
							.extend_pairs(
								parsed
									.query_pairs()
									.filter(|(key, _)| key != "width" && key != "height"),
							)
							.append_pair("width", &width.to_string())
							.append_pair("height", &height.to_string());
						format!("{prefix}:{url}")
					} else {
						format!("{prefix}:{source}")
					}
				})
			};
			let key = sized(EMBED_EDGE, if animated { "anim" } else { "embed" });
			// The viewer wants real pixels: request a larger rendition and show the thumbnail
			// already in memory until it arrives. A GIF/WebP suffix alone does not mean the
			// decoded image animates: single-frame files need the larger rendition too.
			let still = key.as_ref().is_some_and(|key| {
				self.textures.contains_key(key) && !self.animations.contains_key(key)
			});
			let large = (large && (!animated || still))
				.then(|| sized(LARGE_EDGE, "large"))
				.flatten();
			#[cfg(any(test, feature = "demo"))]
			if demo
				&& let Some(key) = &key
				&& !self.textures.contains_key(key)
			{
				let mut image = ColorImage::filled([320, 180], egui::Color32::from_rgb(40, 50, 70));
				for y in 0..180 {
					for x in 0..320 {
						image.pixels[y * 320 + x] = if x < 8 || y < 8 || x >= 312 || y >= 172 {
							egui::Color32::WHITE
						} else if x % 40 < 4 || y % 40 < 4 {
							egui::Color32::from_rgb(80, 180, 160)
						} else {
							egui::Color32::from_rgb(40, 50, 70)
						};
					}
				}
				self.attempts.insert(key.clone(), (Instant::now(), false));
				self.accept(ui.ctx(), key.clone(), Some(image));
			}
			let radius = if cover { 8 } else { 5 };
			let painted_large = large
				.as_deref()
				.is_some_and(|key| self.paint_fitted(ui, key, rect, radius, cover));
			if !painted_large
				&& !demo && let Some(key) = large.as_ref()
			{
				self.request(key.clone());
			}
			let painted = painted_large
				|| key
					.as_deref()
					.is_some_and(|key| self.paint_fitted(ui, key, rect, radius, cover));
			if !painted {
				let colors = crate::design::palette(ui);
				// Discord's ThumbHash stands in for the pixels until the real rendition lands.
				let placeholder =
					self.paint_placeholder(ui, &media.placeholder, rect, radius, cover);
				if !placeholder {
					ui.painter().rect_filled(rect, 5, colors.canvas);
				}
				#[cfg(any(test, feature = "demo"))]
				if demo && !placeholder {
					// Original native landscape, never a service request or bundled third-party image.
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
						rect.min + size * egui::vec2(0.8, 0.25),
						size.y * 0.08,
						colors.accent,
					);
				}
				if !demo && let Some(key) = key.as_ref() {
					self.request(key.clone());
				}
				if !placeholder && size.x >= 100.0 && size.y >= 32.0 {
					ui.painter().text(
						rect.center(),
						egui::Align2::CENTER_CENTER,
						if demo {
							"Synthetic preview"
						} else if key.as_ref().is_some_and(|key| {
							self.attempts.get(key).is_some_and(|(_, failed)| *failed)
						}) {
							"Preview unavailable"
						} else {
							"Image preview"
						},
						egui::FontId::proportional(11.0),
						colors.muted,
					);
				}
			}
		}
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), "Embedded image")
		});
		response
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
			images.show_embed(ui, &media, egui::vec2(320.0, 320.0), false);
		});
		output.textures_delta.clear();
		let key = "thumb:d507121d0467878f77578748878797875878909508";
		assert!(images.texture_id(key).is_some());
		assert_eq!(images.textures[key].1.size(), [23, 32]);
		// The real rendition is still requested; the placeholder only fills the wait.
		let requests = images.take_requests();
		assert!(requests.iter().any(|request| request.starts_with("embed:")));
		assert!(!requests.iter().any(|request| request.starts_with("thumb:")));
		// Garbage never becomes a texture and is not retried every frame.
		let broken = model::EmbedMedia {
			placeholder: vec![0xff; 6],
			..media.clone()
		};
		for _ in 0..2 {
			let mut output = ctx.run_ui(Default::default(), |ui| {
				images.show_embed(ui, &broken, egui::vec2(320.0, 320.0), false);
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
	fn group_fallback_stacks_two_members_and_preserves_custom_icons() {
		for size in [24.0, 32.0, 144.0] {
			for count in [0, 1, 2, 3] {
				for custom in [false, true] {
					let ctx = egui::Context::default();
					crate::design::apply(&ctx);
					let mut images = Avatars::default();
					let mut channel = test_support::demo_state()
						.channels
						.into_iter()
						.find(|c| c.kind == 3)
						.unwrap();
					let user = test_support::message(1, channel.id).author;
					channel.recipients = (0..count)
						.map(|i| {
							let mut user = user.clone();
							user.id = model::Id(i + 100);
							user
						})
						.collect();
					channel.icon = custom.then(|| "0123456789abcdef0123456789abcdef".into());
					if let Some(hash) = &channel.icon {
						let key = format!("group-icon-{}-{hash}", channel.id);
						images.attempts.insert(key.clone(), (Instant::now(), false));
						images.accept(
							&ctx,
							key,
							Some(ColorImage::filled([32, 32], egui::Color32::RED)),
						);
					}
					let mut slot = egui::Rect::NOTHING;
					let mut output = ctx.run_ui(Default::default(), |ui| {
						slot = images.show_group(ui, &channel, size, true).rect;
					});
					output.textures_delta.clear();
					let artwork: Vec<_> = output
						.shapes
						.iter()
						.filter_map(|s| match &s.shape {
							egui::Shape::Rect(m)
								if images
									.textures
									.values()
									.any(|(_, texture)| texture.id() == m.fill_texture_id()) =>
							{
								Some(m.rect)
							}
							_ => None,
						})
						.collect();
					assert_eq!(
						artwork.len(),
						if custom { 1 } else { count.min(2) as usize }
					);
					for rect in &artwork {
						assert!(slot.contains_rect(*rect));
					}
					if artwork.len() == 2 {
						assert!(artwork[0].intersects(artwork[1]));
						assert!(
							artwork[0].left() < artwork[1].left()
								&& artwork[0].top() < artwork[1].top()
						);
					}
					assert!(images.take_requests().is_empty());
					output.drop_without_applying_deltas();
				}
			}
		}
	}
	#[test]
	fn avatar_artwork_matches_fallback_in_justified_layout() {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let user = User {
			id: model::Id(1),
			name: "Synthetic user".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		let mut response_rect = egui::Rect::NOTHING;
		let output = ctx.run_ui(Default::default(), |ui| {
			ui.with_layout(
				egui::Layout::top_down(egui::Align::Min).with_cross_justify(true),
				|ui| {
					ui.set_width(200.0);
					response_rect = images.show(ui, &user, 36.0, false).rect;
				},
			);
		});
		let fallback = output
			.shapes
			.iter()
			.find_map(|shape| match &shape.shape {
				egui::Shape::Circle(circle) if circle.radius == 18.0 => Some(circle.center),
				_ => None,
			})
			.unwrap();
		output.drop_without_applying_deltas();
		let requests = images.take_requests();
		assert_eq!(requests.len(), 1);
		let key = &requests[0];
		images.accept(
			&ctx,
			key.clone(),
			Some(ColorImage::filled([32, 32], egui::Color32::WHITE)),
		);
		let texture = images.texture_id(key).unwrap();
		let output = ctx.run_ui(Default::default(), |ui| {
			ui.with_layout(
				egui::Layout::top_down(egui::Align::Min).with_cross_justify(true),
				|ui| {
					ui.set_width(200.0);
					assert_eq!(images.show(ui, &user, 36.0, false).rect, response_rect);
				},
			);
		});
		let artwork = output
			.shapes
			.iter()
			.find_map(|shape| match &shape.shape {
				egui::Shape::Rect(rect) if rect.fill_texture_id() == texture => Some(rect.rect),
				_ => None,
			})
			.unwrap();
		assert!(response_rect.width() > 36.0);
		assert_ne!(fallback, response_rect.center());
		assert_eq!(artwork.center(), fallback);
		assert_eq!(artwork.size(), egui::Vec2::splat(36.0));
		output.drop_without_applying_deltas();
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
		let canonical = "https://cdn.discordapp.com/attachments/1/2/a.png?ex=abc&is=def&hm=a%2Fb%2Bc+d&tag=x&tag=y&empty=&quality=lossless";
		let keys = media_requests(&media, false, true);
		assert_eq!(
			keys,
			[
				format!("large:{canonical}&width=2048&height=512"),
				format!("embed:{canonical}&width=512&height=128"),
			]
		);
		let renewed = model::EmbedMedia {
			url: Some(source.replace("ex=abc", "ex=renewed")),
			..media.clone()
		};
		assert_ne!(keys, media_requests(&renewed, false, true));
		// Unknown dimensions preserve the exact source, including its existing sizing.
		for (width, height) in [(0, 1024), (4096, 0)] {
			let unknown = model::EmbedMedia {
				width,
				height,
				..media.clone()
			};
			assert_eq!(
				media_requests(&unknown, false, true),
				[format!("large:{source}"), format!("embed:{source}"),]
			);
		}
		let small = model::EmbedMedia {
			width: 64,
			height: 32,
			..media
		};
		assert_eq!(
			media_requests(&small, false, true),
			[
				format!("large:{canonical}&width=64&height=32"),
				format!("embed:{canonical}&width=64&height=32"),
			]
		);
	}

	fn media_requests(media: &model::EmbedMedia, animate: bool, large: bool) -> Vec<String> {
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		images.set_animation(animate);
		let mut frame = || {
			ctx.run_ui(Default::default(), |ui| {
				images.show_media(ui, media, egui::vec2(100.0, 80.0), false, large, false);
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
	fn media_keys_keep_source_selection_animation_and_url_boundaries() {
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
			media_requests(&media, true, true),
			[format!("anim:{original}")]
		);
		assert_eq!(
			media_requests(&media, false, true),
			[
				format!("large:{proxy}&width=1024&height=512"),
				format!("embed:{proxy}&width=512&height=256"),
			]
		);
		// A non-provider original never overrides the service proxy.
		media.url = Some("https://example.test/clip.gif".into());
		assert_eq!(
			media_requests(&media, true, true),
			[format!("anim:{proxy}&width=512&height=256"),]
		);
		media.proxy_url = Some(proxy.replace(".WeBp", ".GIF"));
		assert_eq!(
			media_requests(&media, true, true),
			[format!(
				"anim:{}&width=512&height=256",
				media.proxy_url.as_deref().unwrap()
			),]
		);
		// Preserve fragments/credentials/ports for the download worker's rejection.
		for source in [
			"https://user:pass@media.discordapp.net:444/attachments/1/2/a.png?hm=signed#fragment",
			"http://media.discordapp.net.evil.test/attachments/1/2/a.png?hm=signed#fragment",
		] {
			media.proxy_url = Some(source.into());
			assert_eq!(
				media_requests(&media, false, false),
				[format!(
					"embed:{}&width=512&height=256#fragment",
					source.strip_suffix("#fragment").unwrap()
				),]
			);
		}
		media.proxy_url = Some("not a URL".into());
		assert_eq!(media_requests(&media, false, false), ["embed:not a URL"]);
		media.proxy_url = Some(format!("https://example.test/{}", "a".repeat(2027)));
		assert_eq!(media.proxy_url.as_ref().unwrap().len(), 2048);
		media.width = 0;
		assert_eq!(media_requests(&media, false, false)[0].len(), 2054);
		media.width = 1024;
		assert!(
			media_requests(&media, false, false).is_empty(),
			"Expanded keys keep the request length bound"
		);
		media.proxy_url.as_mut().unwrap().push('a');
		assert!(
			media_requests(&media, false, true).is_empty(),
			"An oversized proxy must not fall back to the original"
		);
		media.url = None;
		media.proxy_url = None;
		assert!(media_requests(&media, true, true).is_empty());
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
		ctx.run_ui(Default::default(), |ui| {
			images.show_large(ui, &media, egui::vec2(100.0, 80.0), false);
		})
		.drop_without_applying_deltas();
		let keys = images.take_requests();
		assert_eq!(keys.len(), 2);
		for (key, size) in [(&keys[1], [2, 1]), (&keys[0], [4, 2])] {
			images.accept(
				&ctx,
				key.clone(),
				Some(ColorImage::filled(size, egui::Color32::WHITE)),
			);
			let texture = images.texture_id(key).unwrap();
			let output = ctx.run_ui(Default::default(), |ui| {
				images.show_large(ui, &media, egui::vec2(100.0, 80.0), false);
			});
			assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
				egui::Shape::Rect(rect) if rect.fill_texture_id() == texture)));
			output.drop_without_applying_deltas();
			assert!(images.take_requests().is_empty());
		}
		assert_eq!(images.textures.len(), 2);
		assert_eq!(images.bytes, (2 + 8) * 4);
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
				.show_large(ui, &avatar_media, egui::vec2(100.0, 80.0), false)
				.rect;
		})
		.drop_without_applying_deltas();
		let keys = images.take_requests();
		assert_eq!(
			keys,
			vec![
				"anim:https://cdn.discordapp.com/avatars/123/a_0123456789abcdef0123456789abcdef.gif?size=2048"
			]
		);
		let key = keys[0].clone();
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
		assert!(images.animations.contains_key(&key));
		images.animations.get_mut(&key).unwrap().started =
			Instant::now() - Duration::from_millis(1500);
		ctx.run_ui(
			egui::RawInput {
				focused: true,
				events: vec![egui::Event::PointerMoved(rect.center())],
				..Default::default()
			},
			|ui| {
				images.show_large(ui, &avatar_media, egui::vec2(100.0, 80.0), false);
			},
		)
		.drop_without_applying_deltas();
		assert_eq!(images.animations[&key].frame, 1);
	}

	/// Offline settled media frames; no window, GPU, network, or account access.
	#[test]
	#[ignore = "release media workload; one warmup and five measured batches"]
	fn media_frame_benchmark() {
		for (scenario, large, animate) in [
			("thumbnail", false, false),
			("viewer", true, false),
			("animated", true, true),
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
								std::hint::black_box(images.show_media(
									ui,
									media,
									egui::vec2(48.0, 32.0),
									false,
									large,
									false,
								));
							}
						});
					},
				)
				.drop_without_applying_deltas();
			};
			frame(&mut images);
			let requests = images.take_requests();
			assert_eq!(requests.len(), if large && !animate { 24 } else { 12 });
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
	fn media_pixels_keep_aspect_without_moving_the_loading_slot() {
		for dimensions in [[120, 40], [40, 120], [80, 80]] {
			for metadata in [(640, 240), (0, 0), (240, 640)] {
				for large in [false, true] {
					let ctx = egui::Context::default();
					let mut images = Avatars::default();
					let media = model::EmbedMedia {
						url: Some("https://cdn.discordapp.com/attachments/1/2/test.png".into()),
						width: metadata.0,
						height: metadata.1,
						..Default::default()
					};
					let mut slot = egui::Rect::NOTHING;
					let output = ctx.run_ui(Default::default(), |ui| {
						slot = images
							.show_media(ui, &media, egui::vec2(280.0, 180.0), false, large, false)
							.rect;
					});
					output.drop_without_applying_deltas();
					let key = images.take_requests().pop().unwrap();
					images.accept(
						&ctx,
						key.clone(),
						Some(ColorImage::filled(dimensions, egui::Color32::WHITE)),
					);
					let texture = images.texture_id(&key).unwrap();
					let output = ctx.run_ui(Default::default(), |ui| {
						assert_eq!(
							slot,
							images
								.show_media(
									ui,
									&media,
									egui::vec2(280.0, 180.0),
									false,
									large,
									false
								)
								.rect
						);
					});
					let meshes = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
					let bounds = meshes
						.iter()
						.filter_map(|shape| match &shape.primitive {
							egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id == texture => {
								Some(mesh.calc_bounds())
							}
							_ => None,
						})
						.reduce(|a, b| a.union(b))
						.unwrap();
					// paint_at rounds to device pixels; allow one pixel of rounding.
					assert!(
						(bounds.width()
							- bounds.height() * dimensions[0] as f32 / dimensions[1] as f32)
							.abs() <= 2.0
					);
					assert!(slot.expand(1.0).contains_rect(bounds));
					output.drop_without_applying_deltas();
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
