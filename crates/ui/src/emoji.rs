//! Bundled Twemoji: scalable artwork rasterised on demand at the size it is drawn.
mod raster;

pub use raster::{Backend, MAX_EDGE, MIN_EDGE};

use egui::{Context, Image, TextureHandle};
use std::sync::OnceLock;

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
	if cell >= VECTOR_COUNT || !(raster::MIN_EDGE..=raster::MAX_EDGE).contains(&edge) {
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
				(text, index.parse().expect("bundled artwork cell"))
			})
			.collect()
	})
}

/// Starts the Unicode emoji service: nothing is decoded until an emoji is drawn, and the
/// backend's worker threads start with the first request.
pub fn install(ctx: &Context, backend: Backend) {
	raster::install(ctx, backend);
	entries();
}

/// Whether Unicode emoji can be drawn as artwork in this context.
pub(crate) fn installed(ctx: &Context) -> bool {
	raster::service(ctx).is_some()
}

/// The bundled artwork cell to reserve a slot for: a known emoji whose artwork can be drawn
/// here. Artwork that failed to rasterise lays out as text instead.
pub(crate) fn artwork(ctx: &Context, text: &str) -> Option<usize> {
	let cell = lookup(text)?;
	let service = raster::service(ctx)?;
	(!service.failed(cell)).then_some(cell)
}

/// Once per frame: uploads finished rasters and releases idle ones even when no emoji is
/// drawn. Returns when the next idle raster can be released, for the host's existing
/// maintenance deadline; it never requests a repaint itself.
pub(crate) fn maintain(ctx: &Context) -> Option<std::time::Instant> {
	raster::service(ctx)?.maintain(ctx)
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

/// Mark text-default symbols that Twemoji draws as emoji (✌, ❤) with U+FE0F, so font fallback
/// colours them in plain-text names as messages do. Emoji-default and explicit text forms stay.
pub(crate) fn presentation(text: &str) -> std::borrow::Cow<'_, str> {
	use unicode_segmentation::UnicodeSegmentation as _;
	let bare = |grapheme: &str| {
		grapheme.chars().nth(1).is_none()
			&& !egui::epaint::text::has_emoji_presentation(grapheme)
			&& lookup(grapheme).is_some()
	};
	if text.is_ascii() || !text.graphemes(true).any(bare) {
		return std::borrow::Cow::Borrowed(text);
	}
	let mut marked = String::with_capacity(text.len() + 3);
	for grapheme in text.graphemes(true) {
		marked.push_str(grapheme);
		if bare(grapheme) {
			marked.push('\u{fe0f}');
		}
	}
	std::borrow::Cow::Owned(marked)
}

/// Square artwork for a known emoji at `size` points, drawn from a raster made for its exact
/// pixel size. While that raster is on its way the slot stays empty (a transparent image of
/// the same size). `None` when `text` is not a bundled emoji, the service is not installed or
/// its artwork failed, so callers show the text instead.
pub(crate) fn image(ctx: &Context, text: &str, size: f32) -> Option<Image<'static>> {
	let cell = lookup(text)?;
	let service = raster::service(ctx)?;
	let image = match service.paint(ctx, (cell, raster::edge(ctx, size))) {
		raster::Lookup::Ready(texture) => Image::new(egui::load::SizedTexture::new(
			texture,
			egui::Vec2::splat(size),
		)),
		raster::Lookup::Pending => blank(ctx, size),
		raster::Lookup::Failed => return None,
	};
	Some(image.alt_text(text))
}

/// Queues `text`'s artwork at `size` points at low priority, ahead of being drawn.
pub(crate) fn prefetch(ctx: &Context, text: &str, size: f32) {
	if let Some(cell) = lookup(text) {
		prefetch_cell(ctx, cell, size);
	}
}

/// [`prefetch`] for a bundled artwork cell from [`lookup`].
pub(crate) fn prefetch_cell(ctx: &Context, cell: usize, size: f32) {
	if let Some(service) = raster::service(ctx) {
		service.prefetch(ctx, (cell, raster::edge(ctx, size)));
	}
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

/// Whether custom emoji markup is the animated `<a:name:id>` form.
pub(crate) fn custom_animated(text: &str) -> bool {
	text.starts_with("<a:") && custom_prefix(text).is_some()
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

	/// Plain-text names colour ✌ like Discord does, without touching emoji or text forms.
	#[test]
	fn names_mark_text_default_twemoji_for_colour() {
		assert_eq!(presentation("✌ oznámení"), "✌\u{fe0f} oznámení");
		for unchanged in ["🛒-bazar", "✌\u{fe0f}", "✌\u{fe0e}", "✌🏻", "plain", "→ x"] {
			assert_eq!(presentation(unchanged), unchanged);
		}
	}
	#[test]
	fn animated_custom_markup_is_recognized() {
		assert!(custom_animated("<a:wave_animated:1169784511555452928>"));
		assert!(!custom_animated("<:wave:1169784511555452928>"));
		assert!(!custom_animated("<a:x:1>"), "names need two characters");
		assert!(!custom_animated("<a:wave:not-an-id>"));
		assert!(!custom_animated("\u{1f44b}"));
	}
	#[test]
	fn vector_artwork_keys_and_decompression_are_bounded() {
		for edge in [16, 37, 256] {
			let (source, parsed) = bundled_svg(&format!("emoji-unicode-0-{edge}")).unwrap();
			assert_eq!(parsed, edge);
			let svg = decode_bundled_svg(source).unwrap();
			assert!(svg.starts_with(b"<svg"));
			assert!(svg.len() <= 65536);
		}
		for key in [
			"emoji-unicode-4009-64",
			"emoji-unicode-0-15",
			"emoji-unicode-0-257",
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
	fn index_is_bounded_and_matches_complete_sequences() {
		assert!(entries().len() <= 4096);
		assert!(entries().windows(2).all(|pair| pair[0].0 < pair[1].0));
		assert!(entries().iter().all(|(_, cell)| *cell < VECTOR_COUNT));
		for text in ["👍", "👍🏽", "❤️", "👩🏽‍💻", "🇨🇿", "1️⃣", "🏳️‍🌈", "🏳‍🌈", "👩‍⚕", "👨‍👩‍👧‍👦"]
		{
			assert!(lookup(text).is_some(), "missing {text}");
		}
		for text in ["hello", "1", "©\u{fe0e}", "👩\u{200d}🦀", "<:custom:123>"] {
			assert!(lookup(text).is_none(), "unexpected {text}");
		}
	}
}

/// Installs a synthetic rasteriser without worker threads; [`run_jobs`] renders queued work.
#[cfg(test)]
pub(crate) fn install_for_tests(ctx: &Context) {
	raster::install_inline(
		ctx,
		Backend {
			rasterize: std::sync::Arc::new(|_, edge| {
				Some(egui::ColorImage::filled(
					[edge as usize; 2],
					egui::Color32::WHITE,
				))
			}),
			on_worker_start: || {},
		},
	);
	entries();
}
