//! Collectible artwork painted around avatars, member rows and profile cards. Layout follows
//! the official client's observed rules; see `model::cosmetics` for the constants.
use super::Avatars;
#[cfg(any(test, feature = "demo"))]
use egui::ColorImage;
use egui::{Color32, Pos2, Rect, Shape, TextureHandle, Vec2, pos2, vec2};
use model::{FrameAnchor, FrameLayerKind, ProfileEffect, ProfileFrame, User};
use std::time::{Duration, Instant};

/// Smaller avatars (replies, reactions, typing) stay undecorated, as in the official client.
pub const DECORATION_MIN: f32 = 30.0;
const EFFECT_FRAME: Duration = Duration::from_millis(33);
const MAX_BORDER_TILES: usize = 64;

/// Image shape showing only the part of `rect` inside `clip`.
fn clipped(texture: &TextureHandle, rect: Rect, clip: Rect, tint: Color32) -> Option<Shape> {
	let visible = rect.intersect(clip);
	if !visible.is_positive() || !rect.is_positive() {
		return None;
	}
	let uv = Rect::from_min_max(
		pos2(
			(visible.min.x - rect.min.x) / rect.width(),
			(visible.min.y - rect.min.y) / rect.height(),
		),
		pos2(
			(visible.max.x - rect.min.x) / rect.width(),
			(visible.max.y - rect.min.y) / rect.height(),
		),
	);
	Some(Shape::image(texture.id(), visible, uv, tint))
}

/// Rounded rectangle filled with a left-to-right gradient.
fn horizontal_gradient(rect: Rect, radius: f32, left: Color32, right: Color32) -> Shape {
	let radius = radius
		.min(rect.width() * 0.5)
		.min(rect.height() * 0.5)
		.max(0.0);
	let mut outline: Vec<Pos2> = Vec::with_capacity(40);
	let corners = [
		(pos2(rect.right() - radius, rect.top() + radius), -90.0_f32),
		(pos2(rect.right() - radius, rect.bottom() - radius), 0.0),
		(pos2(rect.left() + radius, rect.bottom() - radius), 90.0),
		(pos2(rect.left() + radius, rect.top() + radius), 180.0),
	];
	for (center, start) in corners {
		for step in 0..=8 {
			let angle = (start + step as f32 * 90.0 / 8.0).to_radians();
			outline.push(center + vec2(angle.cos(), angle.sin()) * radius);
		}
	}
	let color = |x: f32| {
		let t = ((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
		left.lerp_to_gamma(right, t)
	};
	let mut mesh = egui::Mesh::default();
	mesh.colored_vertex(rect.center(), color(rect.center().x));
	for point in &outline {
		mesh.colored_vertex(*point, color(point.x));
	}
	let count = outline.len() as u32;
	for index in 0..count {
		mesh.add_triangle(0, 1 + index, 1 + (index + 1) % count);
	}
	Shape::mesh(mesh)
}

impl Avatars {
	/// Cached artwork for `key`, requesting it when missing. Demo artwork is generated locally.
	fn cosmetic(&mut self, ctx: &egui::Context, key: &str, demo: bool) -> Option<TextureHandle> {
		#[cfg(any(test, feature = "demo"))]
		if demo && !self.textures.contains_key(key) {
			self.attempts
				.insert(key.to_owned(), (Instant::now(), false));
			self.accept(ctx, key.to_owned(), Some(synthetic(key)));
			if key.starts_with("effect-") && !key.starts_with("effect-still-") {
				self.accept_animation(key.to_owned(), synthetic_frames(key));
			}
		}
		let _ = ctx;
		if let Some((used, texture)) = self.textures.get_mut(key) {
			*used = Instant::now();
			return Some(texture.clone());
		}
		if !demo {
			self.request(key.to_owned());
		}
		None
	}

	/// The user's decoration, 1.2 times the avatar and centered on it.
	pub(crate) fn paint_decoration(
		&mut self,
		ui: &mut egui::Ui,
		user: &User,
		size: f32,
		avatar: Rect,
		demo: bool,
	) {
		if !self.cosmetics.decorations || size < DECORATION_MIN || !ui.is_rect_visible(avatar) {
			return;
		}
		let Some(decoration) = user
			.cosmetics
			.as_deref()
			.and_then(|cosmetics| cosmetics.decoration.as_ref())
			.filter(|decoration| decoration.active(model::unix_now()))
		else {
			return;
		};
		let key = decoration.key();
		let rect = Rect::from_center_size(
			avatar.center(),
			Vec2::splat(size * model::DECORATION_TO_AVATAR),
		);
		self.cosmetic(ui.ctx(), &key, demo);
		self.paint_fitted(ui, &key, rect, 0, false);
	}

	/// Paints the user's nameplate behind a row. Offline rows keep only a faint image until
	/// hovered, and hovered rows play the animated artwork once it has loaded.
	#[allow(clippy::too_many_arguments)]
	pub(crate) fn paint_nameplate(
		&mut self,
		ui: &egui::Ui,
		user: &User,
		row: Rect,
		radius: u8,
		hovered: bool,
		muted: bool,
		demo: bool,
	) {
		if !self.cosmetics.nameplates || !ui.is_rect_visible(row) || !row.is_positive() {
			return;
		}
		let Some(nameplate) = user
			.cosmetics
			.as_deref()
			.and_then(|cosmetics| cosmetics.nameplate.as_ref())
			.filter(|nameplate| nameplate.active(model::unix_now()))
		else {
			return;
		};
		let dim = muted && !hovered;
		let painter = ui.painter().with_clip_rect(row.intersect(ui.clip_rect()));
		if !dim && let Some([dark, light]) = nameplate.palette.colors() {
			let rgb = if ui.visuals().dark_mode { dark } else { light };
			let [r, g, b] = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
			painter.add(horizontal_gradient(
				row,
				f32::from(radius),
				Color32::from_rgba_unmultiplied(r, g, b, 0),
				Color32::from_rgba_unmultiplied(r, g, b, 0x4D),
			));
		}
		let still = nameplate.key();
		let mut texture = self.cosmetic(ui.ctx(), &still, demo);
		if hovered && !demo && self.animate_gifs {
			let animated = nameplate.animated_key();
			if self.textures.contains_key(&animated) {
				if let Some((used, _)) = self.textures.get_mut(&animated) {
					*used = Instant::now();
				}
				if let Some(frame) = self.advance_animation(ui.ctx(), &animated, true) {
					texture = Some(frame);
				}
			} else if !self.no_animations.contains(&animated) {
				self.request(animated);
			}
		}
		let Some(texture) = texture else {
			return;
		};
		let width = row.height() * model::NAMEPLATE_ASPECT;
		let image = Rect::from_min_max(pos2(row.right() - width, row.top()), row.right_bottom());
		let tint = if dim {
			Color32::WHITE.gamma_multiply(0.1)
		} else {
			Color32::WHITE
		};
		let visible = image.intersect(row);
		let uv = Rect::from_min_max(
			pos2((visible.left() - image.left()) / image.width(), 0.0),
			pos2(1.0, 1.0),
		);
		egui::Image::new(&texture)
			.uv(uv)
			.tint(tint)
			.corner_radius(radius)
			.paint_at(ui, visible);
	}

	/// Whether the effect can start playing: motion is allowed and every layer has settled, with
	/// its animation decoded, known to be still, or failed. The worker shows a first frame before
	/// the rest decode, so starting then could let a short one-shot layer pass unseen. All layers
	/// are requested up front for the same reason.
	pub(crate) fn effect_ready(
		&mut self,
		ui: &egui::Ui,
		effect: &ProfileEffect,
		demo: bool,
	) -> bool {
		if !self.animate_gifs || !ui.input(|input| input.focused) {
			return false;
		}
		let mut ready = true;
		for layer in &effect.layers {
			let key = layer.asset.key();
			if self.attempts.get(&key).is_some_and(|(_, failed)| *failed) {
				self.effect_decoding.remove(&key);
				continue;
			}
			if self.cosmetic(ui.ctx(), &key, demo).is_none() {
				// `cosmetic` requested it; its frames arrive with the settled result.
				if self.attempts.contains_key(&key) && self.effect_decoding.len() < 256 {
					self.effect_decoding.insert(key);
				}
				ready = false;
				continue;
			}
			if self.animations.contains_key(&key) || self.no_animations.contains(&key) {
				continue;
			}
			if self.effect_decoding.contains(&key) {
				ready = false;
				continue;
			}
			// Frames lost a second time cannot fit beside the other layers; play it still.
			if self.effect_reloaded.contains(&key) {
				continue;
			}
			ready = false;
			// Frames released while idle, discarded when animation was toggled or evicted from
			// the pool: decode them again, once.
			if self.effect_decoding.len() < 256 && self.effect_reloaded.len() < 256 {
				self.request(key.clone());
				// Only a queued request is pending; a full queue retries next frame.
				if self.attempts.contains_key(&key) {
					self.effect_decoding.insert(key.clone());
					self.effect_reloaded.insert(key);
				}
			}
		}
		ready
	}

	/// Effect layers over a card. `elapsed` counts from when the layers were ready; nothing shows
	/// before then, so the still (the effect's end state) never flashes ahead of the animation.
	/// Only disabled motion or a layerless effect shows the still. One-shot files hold their
	/// last frame; `opacity` fades the effect while the card is hovered.
	pub(crate) fn paint_effect(
		&mut self,
		ui: &egui::Ui,
		effect: &ProfileEffect,
		card: Rect,
		elapsed: Option<Duration>,
		opacity: f32,
		demo: bool,
	) {
		let tint = Color32::WHITE.gamma_multiply(opacity);
		if !ui.is_rect_visible(card) || card.width() <= 0.0 {
			return;
		}
		let clip = card.intersect(ui.clip_rect());
		let painter = ui.painter().with_clip_rect(clip);
		let scale = card.width() / f32::from(model::EFFECT_CANVAS[0]);
		let place = |position: [i16; 2], size: [u16; 2]| {
			Rect::from_min_size(
				card.min + vec2(f32::from(position[0]), f32::from(position[1])) * scale,
				vec2(f32::from(size[0]), f32::from(size[1])) * scale,
			)
		};
		if !self.animate_gifs || effect.layers.is_empty() {
			if let Some(still) = &effect.still
				&& let Some(texture) = self.cosmetic(ui.ctx(), &still.still_key(), demo)
				&& let Some(shape) =
					clipped(&texture, place([0, 0], model::EFFECT_CANVAS), clip, tint)
			{
				painter.add(shape);
			}
			return;
		}
		let Some(elapsed) = elapsed.filter(|_| ui.input(|input| input.focused)) else {
			return;
		};
		let intro = Duration::from_millis(model::EFFECT_INTRO_MS);
		let Some(timeline) = elapsed.checked_sub(intro) else {
			ui.ctx().request_repaint_after(intro - elapsed);
			return;
		};
		let timeline = timeline.as_millis() as u64;
		let mut next: Option<u64> = None;
		for layer in &effect.layers {
			let Some(local) = layer.local_ms(timeline) else {
				if let Some(at) = layer.next_visible_ms(timeline) {
					next = Some(next.map_or(at, |next| next.min(at)));
				}
				continue;
			};
			next = Some(timeline);
			let key = layer.asset.key();
			let Some(still) = self.cosmetic(ui.ctx(), &key, demo) else {
				continue;
			};
			let texture = match self.animations.get_mut(&key) {
				Some(animation) if !animation.total.is_zero() => {
					let total = animation.total.as_millis().max(1) as u64;
					let local = if layer.looping && layer.loop_delay_ms == 0 {
						// A continuous loop follows the file itself: endless files repeat and
						// one-shot files (held last frame) stay stopped, never restarting.
						(timeline - u64::from(layer.start_ms)) % total
					} else if layer.looping {
						local % total
					} else {
						local.min(total - 1)
					};
					animation.seek(ui.ctx(), Duration::from_millis(local))
				}
				_ => {
					// Idle frames were released; decode them again from the disk cache.
					if self.released.contains(&key) {
						self.request(key.clone());
					}
					None
				}
			}
			.unwrap_or(still);
			if let Some(shape) = clipped(&texture, place(layer.position, layer.size), clip, tint) {
				painter.add(shape);
			}
		}
		match next {
			Some(at) if at <= timeline => ui.ctx().request_repaint_after(EFFECT_FRAME),
			Some(at) => ui
				.ctx()
				.request_repaint_after(Duration::from_millis(at - timeline)),
			None => {}
		}
	}

	/// Frame layers of one order (front or back) around `card`, extending past its edges.
	pub(crate) fn frame_shapes(
		&mut self,
		ctx: &egui::Context,
		frame: &ProfileFrame,
		card: Rect,
		clip: Rect,
		front: bool,
		demo: bool,
	) -> Vec<Shape> {
		let mut shapes = Vec::new();
		if !card.is_positive() {
			return shapes;
		}
		let [top, bottom, side] = frame.overflow(card.width());
		let width = card.width() + 2.0 * side;
		let left = card.left() - side;
		// Rails and borders stay within the card's height; staples overflow it.
		let column = Rect::from_min_max(pos2(left, card.top()), pos2(left + width, card.bottom()));
		for layer in frame.layers.iter().filter(|layer| layer.front == front) {
			if layer.kind == FrameLayerKind::Rail
				&& layer.responsive
				&& card.width() / card.height() >= model::FRAME_RAIL_MIN_ASPECT
			{
				continue;
			}
			let Some(texture) = self.cosmetic(ctx, &layer.key(frame.sku), demo) else {
				continue;
			};
			let [w, h] = texture.size();
			if w == 0 || h == 0 {
				continue;
			}
			let height = width * h as f32 / w as f32;
			let at = |y: f32| Rect::from_min_size(pos2(left, y), vec2(width, height));
			match (layer.kind, layer.anchor) {
				(FrameLayerKind::Staple, FrameAnchor::Bottom) => {
					shapes.extend(clipped(
						&texture,
						at(card.bottom() + bottom - height),
						clip,
						Color32::WHITE,
					));
				}
				(FrameLayerKind::Staple, _) => {
					shapes.extend(clipped(
						&texture,
						at(card.top() - top),
						clip,
						Color32::WHITE,
					));
				}
				(FrameLayerKind::Rail, anchor) => {
					let y = match anchor {
						FrameAnchor::Top => card.top(),
						FrameAnchor::Center => card.center().y - height * 0.5,
						FrameAnchor::Bottom => card.bottom() - height,
					};
					shapes.extend(clipped(
						&texture,
						at(y),
						column.intersect(clip),
						Color32::WHITE,
					));
				}
				(FrameLayerKind::Border, _) => {
					let tiles = ((card.height() / height).ceil() as usize).min(MAX_BORDER_TILES);
					for tile in 0..tiles {
						shapes.extend(clipped(
							&texture,
							at(card.top() + tile as f32 * height),
							column.intersect(clip),
							Color32::WHITE,
						));
					}
				}
			}
		}
		shapes
	}
}

/// Original synthetic artwork for offline previews; never downloaded or bundled service art.
#[cfg(any(test, feature = "demo"))]
fn synthetic(key: &str) -> ColorImage {
	synthetic_frame(key, 0)
}

#[cfg(any(test, feature = "demo"))]
fn synthetic_frames(key: &str) -> crate::avatars::GifFrames {
	(0..8)
		.map(|step| {
			(
				Duration::from_millis(90),
				std::sync::Arc::new(synthetic_frame(key, step)),
			)
		})
		.collect()
}

#[cfg(any(test, feature = "demo"))]
fn synthetic_frame(key: &str, step: usize) -> ColorImage {
	let seed = key
		.bytes()
		.fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
	let hue = (seed % 360) as f32 / 360.0;
	let tint = |h: f32, s: f32, v: f32, a: f32| {
		Color32::from(egui::ecolor::Hsva::new(h.rem_euclid(1.0), s, v, a))
	};
	let star = |image: &mut ColorImage, cx: f32, cy: f32, r: f32, color: Color32| {
		let [w, h] = image.size;
		for y in (cy - r).max(0.0) as usize..((cy + r) as usize).min(h) {
			for x in (cx - r).max(0.0) as usize..((cx + r) as usize).min(w) {
				let (dx, dy) = ((x as f32 - cx).abs(), (y as f32 - cy).abs());
				if dx * dy * 4.0 < r * 0.6 && dx + dy < r {
					image.pixels[y * w + x] = color;
				}
			}
		}
	};
	if key.starts_with("decoration-") {
		// A ring of petals around the avatar circle.
		let size = 96usize;
		let mut image = ColorImage::filled([size, size], Color32::TRANSPARENT);
		let center = size as f32 * 0.5;
		for y in 0..size {
			for x in 0..size {
				let (dx, dy) = (x as f32 - center, y as f32 - center);
				let d = (dx * dx + dy * dy).sqrt();
				let angle = dy.atan2(dx);
				let petal = (angle * 8.0).cos() * 3.0;
				if (d - 42.0 - petal).abs() < 3.5 {
					image.pixels[y * size + x] =
						tint(hue + angle / std::f32::consts::TAU * 0.2, 0.55, 0.95, 1.0);
				}
			}
		}
		for index in 0..3 {
			let angle = -2.2 + index as f32 * 0.55;
			star(
				&mut image,
				center + angle.cos() * 42.0,
				center + angle.sin() * 42.0,
				9.0,
				tint(hue + 0.12, 0.3, 1.0, 1.0),
			);
		}
		return image;
	}
	if key.starts_with("nameplate-") {
		let [w, h] = [320usize, 60usize];
		let mut image = ColorImage::filled([w, h], Color32::TRANSPARENT);
		for y in 0..h {
			for x in 0..w {
				let u = x as f32 / w as f32;
				let wave = ((u * 9.0 + y as f32 / h as f32 * 2.0).sin() * 0.5 + 0.5) * u;
				image.pixels[y * w + x] = tint(hue + u * 0.15, 0.6, 0.55 + wave * 0.35, u * 0.85);
			}
		}
		for index in 0..6 {
			star(
				&mut image,
				170.0 + index as f32 * 26.0,
				12.0 + ((index * 17) % 36) as f32,
				6.0,
				tint(hue + 0.1, 0.2, 1.0, 1.0),
			);
		}
		return image;
	}
	if key.starts_with("frame-") {
		// Staple artwork: arching vines along one edge of a 1200-unit card.
		let [w, h] = [328usize, 96usize];
		let mut image = ColorImage::filled([w, h], Color32::TRANSPARENT);
		let bottom = seed % 2 == 0;
		for x in 0..w {
			let u = x as f32 / w as f32;
			let arch = ((u * std::f32::consts::PI).sin() * 40.0 + (u * 31.0).sin() * 4.0) as usize;
			for thickness in 0..5 {
				let y = (h - 20).saturating_sub(arch) + thickness;
				let y = if bottom {
					h - 1 - y.min(h - 1)
				} else {
					y.min(h - 1)
				};
				image.pixels[y * w + x] = tint(hue + u * 0.1, 0.5, 0.8, 1.0);
			}
		}
		for index in 0..7 {
			let x = 18.0 + index as f32 * 48.0;
			let y = if bottom { 70.0 } else { 26.0 } + ((index * 13) % 18) as f32;
			star(&mut image, x, y, 10.0, tint(hue + 0.5, 0.45, 1.0, 1.0));
		}
		return image;
	}
	// Effect canvas: drifting sparkles over a transparent 450 x 880 layout, at quarter size.
	let [w, h] = [112usize, 220usize];
	let mut image = ColorImage::filled([w, h], Color32::TRANSPARENT);
	for index in 0..14usize {
		let x = ((index * 37 + (seed as usize % 11)) % w) as f32;
		let y = ((index * 53 + step * 9) % h) as f32;
		let r = 1.5 + ((index + step) % 4) as f32 * 0.6;
		star(
			&mut image,
			x,
			y,
			r,
			tint(hue + index as f32 * 0.03, 0.35, 1.0, 0.8),
		);
	}
	image
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::Id;

	#[test]
	fn hidden_cosmetics_are_neither_painted_nor_requested() {
		let ctx = egui::Context::default();
		let mut user = test_support::message(1, Id(1)).author;
		user.cosmetics = Some(Box::new(test_support::synthetic_cosmetics()));
		let avatar = Rect::from_min_size(pos2(10.0, 10.0), Vec2::splat(40.0));
		let row = Rect::from_min_size(pos2(0.0, 0.0), vec2(240.0, 44.0));
		let paint = |cosmetics| {
			let mut avatars = Avatars {
				cosmetics,
				..Avatars::default()
			};
			ctx.run_ui(Default::default(), |ui| {
				avatars.paint_decoration(ui, &user, 40.0, avatar, false);
				avatars.paint_nameplate(ui, &user, row, 6, false, false, false);
			})
			.drop_without_applying_deltas();
			avatars.take_requests()
		};
		let shown = paint(model::CosmeticPreferences::SHOWN);
		assert!(shown.iter().any(|key| key.starts_with("decoration-")));
		assert!(shown.iter().any(|key| key.starts_with("nameplate-")));
		// Opt-in: nothing is drawn or requested by default.
		assert!(paint(model::CosmeticPreferences::default()).is_empty());
		let only_nameplates = paint(model::CosmeticPreferences {
			nameplates: true,
			..Default::default()
		});
		assert!(
			only_nameplates
				.iter()
				.all(|key| key.starts_with("nameplate-"))
				&& !only_nameplates.is_empty()
		);
	}

	#[test]
	fn effects_wait_for_decoded_layers_and_recover_discarded_frames() {
		let ctx = egui::Context::default();
		let layer = |hash: char| model::EffectLayer {
			asset: model::EffectAsset {
				shop: false,
				hash: hash.to_string().repeat(64),
			},
			looping: true,
			position: [0, 0],
			size: model::EFFECT_CANVAS,
			start_ms: 0,
			duration_ms: 500,
			loop_delay_ms: 0,
			z: 0,
		};
		let effect = ProfileEffect {
			sku: Id(1),
			title: String::new(),
			label: String::new(),
			still: None,
			layers: vec![layer('a')],
		};
		let key = effect.layers[0].asset.key();
		let mut avatars = Avatars::default();
		avatars.set_animation(true);
		let ready = |avatars: &mut Avatars| {
			let mut ready = false;
			ctx.run_ui(Default::default(), |ui| {
				ready = avatars.effect_ready(ui, &effect, false);
			})
			.drop_without_applying_deltas();
			ready
		};
		let frames = || -> crate::avatars::GifFrames {
			(0..2)
				.map(|_| {
					(
						Duration::from_millis(50),
						std::sync::Arc::new(ColorImage::filled([4, 4], Color32::WHITE)),
					)
				})
				.collect()
		};
		// Missing artwork is requested; the preview frame alone does not start playback.
		assert!(!ready(&mut avatars));
		assert_eq!(avatars.take_requests(), std::slice::from_ref(&key));
		avatars.accept(
			&ctx,
			key.clone(),
			Some(ColorImage::filled([4, 4], Color32::WHITE)),
		);
		assert!(!ready(&mut avatars));
		assert!(avatars.take_requests().is_empty(), "decode already pending");
		avatars.accept_animation(key.clone(), frames());
		assert!(ready(&mut avatars));
		// Toggling animation discards frames but keeps the still: decode again, once.
		avatars.set_animation(false);
		avatars.set_animation(true);
		assert!(!ready(&mut avatars));
		assert_eq!(avatars.take_requests(), std::slice::from_ref(&key));
		assert!(!ready(&mut avatars));
		assert!(avatars.take_requests().is_empty());
		avatars.accept(
			&ctx,
			key.clone(),
			Some(ColorImage::filled([4, 4], Color32::WHITE)),
		);
		avatars.accept_animation(key.clone(), frames());
		assert!(ready(&mut avatars));
		// Evicted again (the effect exceeds the animation pool): play it as a still, no loop.
		avatars.animations.remove(&key);
		assert!(ready(&mut avatars));
		assert!(avatars.take_requests().is_empty());
		// A failed reload clears its pending mark so a later readiness pass can retry.
		avatars.set_animation(false);
		avatars.set_animation(true);
		assert!(!ready(&mut avatars));
		assert_eq!(avatars.take_requests(), std::slice::from_ref(&key));
		avatars.accept(&ctx, key.clone(), None);
		assert!(!avatars.effect_decoding.contains(&key));
	}
}
