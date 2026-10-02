//! Bundled Twemoji: an inline atlas and bounded scalable artwork decoded by the media worker.
use egui::{Context, Image, TextureHandle};
use image::ImageDecoder;
use std::sync::OnceLock;

const ATLAS: &[u8] = include_bytes!("../../../assets/twemoji/atlas.png");
const INDEX: &str = include_str!("../../../assets/twemoji/index.tsv");
const VECTORS: &[u8] = include_bytes!("../../../assets/twemoji/vectors.bin");
const VECTOR_COUNT: usize = 4009;
const VECTOR_HEADER: usize = (VECTOR_COUNT + 1) * 4;

/// Parse only bounded local artwork keys. No URL or filesystem input is accepted.
pub fn bundled_svg(key: &str) -> Option<(&'static [u8], u32)> {
	if key.len() > 64 {
		return None;
	}
	let (cell, edge) = key.strip_prefix("emoji-unicode-")?.split_once('-')?;
	let cell = cell.parse::<usize>().ok()?;
	let edge = edge.parse::<u32>().ok()?;
	if cell >= VECTOR_COUNT || !matches!(edge, 64 | 128 | 256) {
		return None;
	}
	let offset = |index| {
		Some(u32::from_le_bytes(VECTORS.get(index * 4..index * 4 + 4)?.try_into().ok()?) as usize)
	};
	let start = offset(cell)?;
	let end = offset(cell + 1)?;
	if start >= end || end - start > 65536 {
		return None;
	}
	Some((
		VECTORS.get(VECTOR_HEADER + start..VECTOR_HEADER + end)?,
		edge,
	))
}

/// Call outside rendering. Individual trusted SVGs have a 64 KiB expansion/window limit.
pub fn decode_bundled_svg(source: &[u8]) -> Option<Vec<u8>> {
	use std::io::Read;
	if source.is_empty() || source.len() > 65536 {
		return None;
	}
	let decoder =
		ruzstd::decoding::StreamingDecoder::new_with_max_window_size(source, 65536).ok()?;
	let mut svg = Vec::new();
	decoder.take(65537).read_to_end(&mut svg).ok()?;
	(svg.len() <= 65536).then_some(svg)
}

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

// The bundled RGBA8 PNG decodes directly into its final pixel allocation. Color32's
// safe byte view is converted from straight alpha before the image reaches egui.
fn decode_atlas() -> Result<egui::ColorImage, image::ImageError> {
	let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(ATLAS))?;
	assert_eq!(
		decoder.color_type(),
		image::ColorType::Rgba8,
		"bundled atlas format"
	);
	let (width, height) = decoder.dimensions();
	assert!(u64::from(width) * u64::from(height) * 4 <= 16 * 1024 * 1024);
	let mut image = egui::ColorImage::filled(
		[width as usize, height as usize],
		egui::Color32::TRANSPARENT,
	);
	decoder.read_image(image.as_raw_mut())?;
	for pixel in &mut image.pixels {
		let [r, g, b, a] = pixel.to_array();
		*pixel = egui::Color32::from_rgba_unmultiplied(r, g, b, a);
	}
	Ok(image)
}

/// Decode once during application creation, outside the render callback.
pub fn install(ctx: &Context) -> Result<(), image::ImageError> {
	let texture = ctx.load_texture(
		"Twemoji 17.0.3",
		decode_atlas()?,
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

pub(crate) fn inline_size(ui: &egui::Ui) -> f32 {
	egui::TextStyle::Body.resolve(ui.style()).size * 1.6
}

/// A blank glyph with real advance width keeps inline objects intact across row breaks.
pub(crate) fn inline_format(ui: &egui::Ui, width: f32, height: f32) -> egui::TextFormat {
	let mut font_id = egui::FontId::monospace(height);
	let space = ui.fonts_mut(|fonts| fonts.glyph_width(&font_id, ' '));
	font_id.size *= width / space.max(f32::EPSILON);
	egui::TextFormat {
		font_id,
		color: egui::Color32::TRANSPARENT,
		line_height: Some(height),
		..Default::default()
	}
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

pub(crate) fn blank(ctx: &Context, size: f32) -> Image<'static> {
	let id = egui::Id::unique("emoji-blank");
	let texture = ctx.data(|data| data.get_temp::<TextureHandle>(id));
	let texture = texture.unwrap_or_else(|| {
		let texture = ctx.load_texture(
			"emoji-blank",
			egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
			egui::TextureOptions::LINEAR,
		);
		ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
		texture
	});
	Image::new(&texture).fit_to_exact_size(egui::Vec2::splat(size))
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
	fn vector_artwork_keys_and_decompression_are_bounded() {
		for edge in [64, 128, 256] {
			let (source, parsed) = bundled_svg(&format!("emoji-unicode-0-{edge}")).unwrap();
			assert_eq!(parsed, edge);
			let svg = decode_bundled_svg(source).unwrap();
			assert!(svg.starts_with(b"<svg"));
			assert!(svg.len() <= 65536);
		}
		for key in [
			"emoji-unicode-4009-64",
			"emoji-unicode-0-4096",
			"emoji-unicode-../0-64",
			"emoji-unicode-0-64?url=x",
			"emoji-9001",
		] {
			assert!(bundled_svg(key).is_none(), "{key}");
		}
		assert!(decode_bundled_svg(b"not compressed svg").is_none());
		for cell in 0..VECTOR_COUNT {
			let (source, _) = bundled_svg(&format!("emoji-unicode-{cell}-64")).unwrap();
			assert!(decode_bundled_svg(source).is_some(), "cell {cell}");
		}
	}
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
	fn direct_decode_preserves_every_premultiplied_pixel() {
		let rgba = image::load_from_memory_with_format(ATLAS, image::ImageFormat::Png)
			.unwrap()
			.into_rgba8();
		let expected = egui::ColorImage::from_rgba_unmultiplied(
			[rgba.width() as usize, rgba.height() as usize],
			&rgba,
		);
		assert_eq!(decode_atlas().unwrap(), expected);
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
