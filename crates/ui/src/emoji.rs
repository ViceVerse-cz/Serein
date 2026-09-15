//! Bundled Twemoji. One fixed atlas, no runtime requests or per-message image cache.
use egui::{Context, Image, TextureHandle};
use std::sync::OnceLock;

const ATLAS: &[u8] = include_bytes!("../../../assets/twemoji/atlas.png");
const INDEX: &str = include_str!("../../../assets/twemoji/index.tsv");
static ENTRIES: OnceLock<Vec<(&'static str, usize)>> = OnceLock::new();

fn entries() -> &'static [(&'static str, usize)] {
	ENTRIES.get_or_init(|| {
		INDEX
			.lines()
			.map(|line| {
				let (text, index) = line.split_once('\t').expect("bundled emoji index");
				(text, index.parse().expect("bundled atlas cell"))
			})
			.collect()
	})
}

/// Decode once during application creation, outside the render callback.
pub fn install(ctx: &Context) -> Result<(), image::ImageError> {
	let image = image::load_from_memory_with_format(ATLAS, image::ImageFormat::Png)?.into_rgba8();
	let size = [image.width() as usize, image.height() as usize];
	let texture = ctx.load_texture(
		"Twemoji 17.0.3",
		egui::ColorImage::from_rgba_unmultiplied(size, &image),
		egui::TextureOptions::LINEAR,
	);
	ctx.data_mut(|data| data.insert_temp(egui::Id::unique("twemoji"), texture));
	entries();
	Ok(())
}

pub(crate) fn ready(ctx: &Context) -> bool {
	ctx.data(|data| {
		data.get_temp::<TextureHandle>(egui::Id::unique("twemoji"))
			.is_some()
	})
}

/// Startup decoding uses a single bounded worker; the context is thread-safe.
pub fn install_async(ctx: &Context) -> std::io::Result<()> {
	let ctx = ctx.clone();
	std::thread::Builder::new()
		.name("emoji-atlas".into())
		.spawn(move || {
			if install(&ctx).is_ok() {
				ctx.request_repaint();
			}
		})
		.map(|_| ())
}

pub(crate) fn inline_size(ui: &egui::Ui) -> f32 {
	egui::TextStyle::Body.resolve(ui.style()).size * 1.6
}

pub(crate) fn lookup(text: &str) -> Option<usize> {
	// Explicit text presentation must stay text. Do not partially match unknown sequences.
	if text.is_ascii() || text.contains('\u{fe0e}') || text.len() > 128 {
		return None;
	}
	let mut normalized = [0; 128];
	let key = if text.contains('\u{fe0f}') {
		let mut len = 0;
		for c in text.chars().filter(|c| *c != '\u{fe0f}') {
			len += c.encode_utf8(&mut normalized[len..]).len();
		}
		std::str::from_utf8(&normalized[..len]).expect("normalized UTF-8")
	} else {
		text
	};
	entries()
		.binary_search_by_key(&key, |(name, _)| *name)
		.ok()
		.map(|index| entries()[index].1)
}

pub(crate) fn image(ctx: &Context, text: &str, size: f32) -> Option<Image<'static>> {
	let cell = lookup(text)?;
	Some(image_cell(atlas(ctx)?, text, cell, size))
}

pub(crate) fn atlas(ctx: &Context) -> Option<egui::load::SizedTexture> {
	let texture = ctx.data(|data| data.get_temp::<TextureHandle>(egui::Id::unique("twemoji")))?;
	Some((&texture).into())
}

pub(crate) fn image_cell(
	atlas: egui::load::SizedTexture,
	text: &str,
	cell: usize,
	size: f32,
) -> Image<'static> {
	let x = (cell % 64) as f32 * 32.0;
	let y = (cell / 64) as f32 * 32.0;
	let uv = egui::Rect::from_min_max(
		egui::pos2(x / atlas.size.x, y / atlas.size.y),
		egui::pos2((x + 32.0) / atlas.size.x, (y + 32.0) / atlas.size.y),
	);
	Image::new((atlas.id, egui::Vec2::splat(size)))
		.uv(uv)
		.alt_text(text)
}

pub(crate) fn button(ctx: &Context, emoji: &str, text: String) -> egui::Button<'static> {
	if let Some(image) = image(ctx, emoji, 18.0) {
		egui::Button::image_and_text(image, text).image_tint_follows_text_color(false)
	} else {
		egui::Button::new(format!("{emoji} {text}"))
	}
}

pub(crate) fn custom_prefix(text: &str) -> Option<(model::Id, usize)> {
	let body = text
		.strip_prefix("<:")
		.or_else(|| text.strip_prefix("<a:"))?;
	let end = body.as_bytes().iter().take(54).position(|b| *b == b'>')?;
	let (name, id) = body[..end].split_once(':')?;
	if !(2..=32).contains(&name.len())
		|| !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
	{
		return None;
	}
	Some((id.parse().ok()?, text.len() - body.len() + end + 1))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn custom_markup_is_bounded_and_never_an_arbitrary_url() {
		for token in ["<:serein_wave:9001>", "<a:serein_party:9001>"] {
			assert_eq!(
				custom_prefix(&format!("{token} trailing")),
				Some((model::Id(9001), token.len()))
			);
		}
		for token in [
			"<:x:1>",
			"<:hello:0>",
			"<:hello:-1>",
			"<:hello:18446744073709551616>",
			"<:../x:1>",
			"<:hello:1/2>",
			"<a:hello:1",
			"<:hello:https://example.com>",
		] {
			assert!(custom_prefix(token).is_none(), "{token}");
		}
	}
	#[test]
	fn atlas_is_bounded_and_matches_complete_sequences() {
		let image = image::load_from_memory(ATLAS).unwrap();
		assert!(image.width() as usize * image.height() as usize * 4 <= 16 * 1024 * 1024);
		assert!(ATLAS.len() <= 8 * 1024 * 1024);
		assert!(entries().len() <= 4096);
		assert!(entries().windows(2).all(|pair| pair[0].0 < pair[1].0));
		assert!(
			entries()
				.iter()
				.all(|(_, cell)| *cell < (image.width() / 32 * (image.height() / 32)) as usize)
		);
		for text in ["👍", "👍🏽", "❤️", "👩🏽‍💻", "🇨🇿", "1️⃣", "🏳️‍🌈", "🏳‍🌈", "👩‍⚕", "👨‍👩‍👧‍👦"]
		{
			assert!(lookup(text).is_some(), "missing {text}");
		}
		for text in ["hello", "1", "©\u{fe0e}", "👩\u{200d}🦀", "<:custom:123>"] {
			assert!(lookup(text).is_none(), "unexpected {text}");
		}
	}
}
