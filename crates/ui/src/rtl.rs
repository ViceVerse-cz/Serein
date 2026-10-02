//! Bounded, line-first bidi layout. Source and copying remain in logical UTF-8 order.
//! Native shaping supplies exact clusters; characters are never reversed or guessed from glyphs.

use egui::{
	Color32, Rect, TextFormat, Vec2,
	epaint::Galley,
	text::{ByteIndex, LayoutJob, LayoutSection},
};
use std::{ops::Range, sync::Arc};
use unicode_bidi::BidiInfo;

pub(crate) const MAX_BYTES: usize = 8192;
const MAX_SPANS: usize = 512;
const MAX_LINES: usize = 128;

/// An inline object is already admitted by the message renderer; it is never fetched here.
#[derive(Clone, PartialEq)]
pub(crate) struct Span {
	pub text: String,
	pub format: TextFormat,
	pub action: usize,
	pub object: Option<usize>,
	pub copy: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Cell {
	pub source: Range<usize>,
	pub logical: Range<usize>,
	pub rect: Rect,
	pub rtl: bool,
	pub action: usize,
	pub object: Option<usize>,
}

pub(crate) struct Line {
	pub position: Vec2,
	pub galley: Arc<Galley>,
}

pub(crate) struct Layout {
	pub source: String,
	pub lines: Vec<Line>,
	pub cells: Vec<Cell>,
	pub size: Vec2,
}

impl Layout {
	pub(crate) fn allocated_bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ self.source.capacity()
			+ self.cells.capacity() * std::mem::size_of::<Cell>()
			+ self.lines.capacity() * std::mem::size_of::<Line>()
			+ self
				.lines
				.iter()
				.map(|line| galley_bytes(&line.galley))
				.sum::<usize>()
	}
}

pub(crate) fn galley_bytes(galley: &Galley) -> usize {
	std::mem::size_of::<Galley>()
		+ std::mem::size_of::<LayoutJob>()
		+ galley.job.text.capacity()
		+ galley.job.sections.capacity() * std::mem::size_of::<LayoutSection>()
		+ galley.rows.capacity() * std::mem::size_of::<egui::epaint::text::PlacedRow>()
		+ galley
			.rows
			.iter()
			.map(|row| {
				std::mem::size_of::<egui::epaint::text::Row>()
					+ row.glyphs.capacity() * std::mem::size_of::<egui::epaint::text::Glyph>()
					+ row.visuals.mesh.vertices.capacity()
						* std::mem::size_of::<egui::epaint::Vertex>()
					+ row.visuals.mesh.indices.capacity() * std::mem::size_of::<u32>()
			})
			.sum::<usize>()
}

struct Unit {
	logical: Range<usize>,
	source: Range<usize>,
	format: TextFormat,
	action: usize,
	object: Option<usize>,
}

struct Prepared {
	logical: String,
	source: String,
	units: Vec<Unit>,
}

impl Prepared {
	fn new(spans: &[Span]) -> Option<Self> {
		if spans.len() > MAX_SPANS
			|| spans.iter().map(|span| span.text.len()).sum::<usize>() > MAX_BYTES
		{
			return None;
		}
		let mut logical = String::new();
		let mut source = String::new();
		let mut units = Vec::with_capacity(spans.len());
		for span in spans {
			let from = logical.len();
			if span.object.is_some() {
				logical.push('\u{fffc}');
			} else {
				logical.push_str(&span.text);
			}
			let source_from = source.len();
			if span.copy {
				source.push_str(&span.text);
			}
			units.push(Unit {
				logical: from..logical.len(),
				source: source_from..source.len(),
				format: span.format.clone(),
				action: span.action,
				object: span.object,
			});
		}
		Some(Self {
			logical,
			source,
			units,
		})
	}
}

/// R and AL can share one bidi level and one installed font face. Separate their
/// strong-script transitions so the native shaper cannot infer Hebrew for Arabic
/// joining (or Arabic for Hebrew) just from the first strong character.
fn script_parts(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
	let mut parts = Vec::new();
	let mut start = range.start;
	let mut previous = None;
	for (offset, chr) in text[range.clone()].char_indices() {
		let class = unicode_bidi::bidi_class(chr);
		if matches!(
			class,
			unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL
		) {
			if previous.is_some_and(|previous| previous != class) {
				let end = range.start + offset;
				parts.push(start..end);
				start = end;
			}
			previous = Some(class);
		}
	}
	parts.push(start..range.end);
	parts
}

struct Piece<'a> {
	job: Range<usize>,
	logical: Range<usize>,
	unit: &'a Unit,
	rtl: bool,
}

fn shape(
	ctx: &egui::Context,
	input: &Prepared,
	bidi: &BidiInfo<'_>,
	paragraph: &unicode_bidi::ParagraphInfo,
	line: Range<usize>,
) -> (Arc<Galley>, Vec<Cell>) {
	let (levels, runs) = bidi.visual_runs(paragraph, line);
	let mut job = LayoutJob::default();
	let mut pieces = Vec::new();
	let mut directions = Vec::new();
	for run in runs {
		let rtl = levels[run.start].is_rtl();
		let mut parts: Vec<_> = input
			.units
			.iter()
			.filter_map(|unit| {
				let from = run.start.max(unit.logical.start);
				let to = run.end.min(unit.logical.end);
				(from < to).then_some((from..to, unit))
			})
			.flat_map(|(range, unit)| {
				script_parts(&input.logical, range)
					.into_iter()
					.map(move |range| (range, unit))
			})
			.collect();
		if rtl {
			parts.reverse();
		}
		for (logical, unit) in parts {
			let start = job.text.len();
			if unit.object.is_some() {
				job.text.push(' ');
			} else {
				job.text.push_str(&input.logical[logical.clone()]);
			}
			let end = job.text.len();
			let format = unit.format.clone();
			directions.push(rtl);
			// append() merges equal formats and would erase a bidi boundary.
			job.sections.push(LayoutSection {
				leading_space: 0.0,
				byte_range: ByteIndex(start)..ByteIndex(end),
				format,
			});
			pieces.push(Piece {
				job: start..end,
				logical,
				unit,
				rtl,
			});
		}
	}
	let native = ctx.fonts_mut(|fonts| fonts.layout_single_line_with_clusters(job, &directions));
	let mut cells = Vec::with_capacity(native.clusters.len());
	for cluster in native.clusters {
		let Some(piece) = pieces
			.iter()
			.find(|piece| piece.job.contains(&cluster.bytes.start))
		else {
			continue;
		};
		let logical = if piece.unit.object.is_some() {
			piece.logical.clone()
		} else {
			piece.logical.start + cluster.bytes.start - piece.job.start
				..piece.logical.start + cluster.bytes.end - piece.job.start
		};
		let source = if piece.unit.object.is_some() || piece.unit.source.is_empty() {
			piece.unit.source.clone()
		} else {
			piece.unit.source.start + logical.start - piece.unit.logical.start
				..piece.unit.source.start + logical.end - piece.unit.logical.start
		};
		cells.push(Cell {
			source,
			logical,
			rect: cluster.rect,
			rtl: piece.rtl,
			action: piece.unit.action,
			object: piece.unit.object,
		});
	}
	(native.galley, cells)
}

/// Layout only messages that contain RTL. Each soft line is chosen in logical order,
/// then Unicode bidi rules L1/L2 and native directional shaping apply to that line.
fn layout_uncached(ctx: &egui::Context, spans: &[Span], width: f32) -> Option<Layout> {
	let input = Prepared::new(spans)?;
	let bidi = BidiInfo::new(&input.logical, None);
	if !bidi.has_rtl() || !width.is_finite() || width < 1.0 {
		return None;
	}
	let mut line_count = 0;
	let mut out = Layout {
		source: input.source.clone(),
		lines: Vec::new(),
		cells: Vec::new(),
		size: Vec2::new(width, 0.0),
	};
	for paragraph in &bidi.paragraphs {
		let from = paragraph.range.start;
		let end = paragraph.range.end;
		let terminal = input.logical[paragraph.range.clone()].chars().next_back();
		// Remove just the terminal paragraph separator. U+2028 is a physical
		// line break inside a bidi paragraph, so preserve consecutive blank rows.
		let to = end
			- terminal
				.filter(|chr| matches!(chr, '\r' | '\n' | '\u{0085}' | '\u{2029}'))
				.map_or(0, char::len_utf8);
		// Unicode bidi treats CR and LF as separate paragraph separators. A CRLF
		// pair is one physical newline, while copying retains both original bytes.
		if from == to
			&& input.logical[paragraph.range.clone()].starts_with('\n')
			&& input.logical[..from].ends_with('\r')
		{
			continue;
		}
		if from == to {
			line_count += 1;
			if line_count > MAX_LINES {
				return None;
			}
			out.size.y += spans.first().map_or(15.0, |span| span.format.font_id.size);
			continue;
		}
		// Line/paragraph separators are excluded from shaping but remain in source.
		let mut hard_start = from;
		let mut hard_lines = Vec::new();
		for (offset, chr) in input.logical[from..to].char_indices() {
			if matches!(chr, '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}') {
				hard_lines.push(hard_start..from + offset);
				hard_start = from + offset + chr.len_utf8();
			}
		}
		hard_lines.push(hard_start..to);
		for hard in hard_lines {
			let from = hard.start;
			let to = hard.end;
			if from == to {
				line_count += 1;
				if line_count > MAX_LINES {
					return None;
				}
				out.size.y += spans.first().map_or(15.0, |span| span.format.font_id.size);
				continue;
			}

			// Whole-paragraph shaping supplies advances only. It is never painted or wrapped
			// in visual order. Final line shaping is repeated after choosing logical breaks.
			let (_, measured) = shape(ctx, &input, &bidi, paragraph, from..to);
			let mut advances: Vec<(Range<usize>, f32)> = measured
				.into_iter()
				.map(|cell| (cell.logical, cell.rect.width()))
				.collect();
			advances.sort_by_key(|(range, _)| range.start);
			let mut start = from;
			while start < to {
				line_count += 1;
				if line_count > MAX_LINES {
					return None;
				}
				let mut end = start;
				let mut used = 0.0;
				let mut word = None;
				for (range, advance) in advances.iter().filter(|(range, _)| range.start >= start) {
					if used + advance > width && end > start {
						break;
					}
					used += advance;
					end = range.end;
					if input.logical[start..end]
						.chars()
						.next_back()
						.is_some_and(char::is_whitespace)
					{
						word = Some(end);
					}
				}
				if end < to
					&& let Some(word) = word
				{
					end = word;
				}
				if end <= start {
					return None;
				}
				let (galley, mut cells) = loop {
					let result = shape(ctx, &input, &bidi, paragraph, start..end);
					if result.0.size().x <= width + 0.5 {
						break result;
					}
					let earlier = advances
						.iter()
						.filter(|(range, _)| range.end < end && range.end > start)
						.map(|(range, _)| range.end)
						.next_back();
					let Some(earlier) = earlier else {
						// One indivisible native cluster/object cannot fit. Use the
						// explicit bounded preview rather than painting over neighbours.
						return None;
					};
					end = earlier;
				};
				let x = if paragraph.level.is_rtl() {
					(width - galley.size().x).max(0.0)
				} else {
					0.0
				};
				let position = Vec2::new(x, out.size.y);
				for cell in &mut cells {
					cell.rect = cell.rect.translate(position);
				}
				out.cells.extend(cells);
				out.size.y += galley.size().y;
				out.lines.push(Line { position, galley });
				start = end;
			}
		}
	}
	Some(out)
}

impl Layout {
	/// Endpoint affinity follows the visual direction, and never splits a shaping cluster.
	pub fn cursor(&self, pos: egui::Pos2) -> usize {
		let Some(cell) = self
			.cells
			.iter()
			.filter(|cell| !cell.source.is_empty())
			.min_by(|a, b| {
				let metric = |cell: &Cell| {
					(cell.rect.center().y - pos.y).abs() * 10000.0 + cell.rect.distance_to_pos(pos)
				};
				metric(a).total_cmp(&metric(b))
			})
		else {
			return 0;
		};
		let first = (pos.x < cell.rect.center().x) != cell.rtl;
		if first {
			cell.source.start
		} else {
			cell.source.end
		}
	}

	pub fn paint(&self, ui: &egui::Ui, pos: egui::Pos2) {
		for line in &self.lines {
			ui.painter().galley(
				pos + line.position,
				line.galley.clone(),
				Color32::TRANSPARENT,
			);
		}
	}
}

const CACHE_ITEMS: usize = 32;
const CACHE_BYTES: usize = 8 * 1024 * 1024;
const ENTRY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct Cache {
	entries: std::collections::VecDeque<Entry>,
	bytes: usize,
}
struct Entry {
	spans: Vec<Span>,
	width: u32,
	scale: u32,
	fonts: (usize, usize),
	discovered: usize,
	atlas_generation: usize,
	value: Arc<Layout>,
	bytes: usize,
}

/// Retains at most 32 immutable layouts / 8 MiB, with a 4 MiB per-layout limit.
/// Source, mappings, native glyphs and meshes are counted by allocated capacity.
pub(crate) fn layout(ctx: &egui::Context, spans: &[Span], width: f32) -> Option<Arc<Layout>> {
	let id = egui::Id::unique("bounded-rtl-layout-cache");
	let cache = ctx.data_mut(|data| {
		data.get_temp::<Arc<std::sync::Mutex<Cache>>>(id)
			.unwrap_or_else(|| {
				let cache = Arc::new(std::sync::Mutex::new(Cache::default()));
				data.insert_temp(id, cache.clone());
				cache
			})
	});
	{
		let handle = ctx.plugin_or_default::<CacheLife>();
		let mut life = handle.lock();
		life.used = true;
		life.cache = Arc::downgrade(&cache);
	}
	let scale = ctx.pixels_per_point().to_bits();
	let fonts = crate::fonts::revision(ctx);
	let (discovered, atlas_generation) =
		ctx.fonts(|fonts| (fonts.discovered_fonts().len(), fonts.layout_generation()));
	{
		let mut cache = cache.lock().expect("RTL cache lock");
		if let Some(index) = cache.entries.iter().position(|entry| {
			entry.width == width.to_bits()
				&& entry.scale == scale
				&& entry.fonts == fonts
				&& entry.discovered == discovered
				&& entry.atlas_generation == atlas_generation
				&& entry.spans == spans
		}) {
			let entry = cache.entries.remove(index).expect("known cache entry");
			let value = entry.value.clone();
			cache.entries.push_back(entry);
			return Some(value);
		}
	}
	let value = Arc::new(layout_uncached(ctx, spans, width)?);
	let stored: Vec<Span> = spans
		.iter()
		.map(|span| {
			let mut span = span.clone();
			span.text.shrink_to_fit();
			span
		})
		.collect();
	let span_bytes = stored.capacity() * std::mem::size_of::<Span>()
		+ stored
			.iter()
			.map(|span| span.text.capacity())
			.sum::<usize>()
		+ std::mem::size_of::<Entry>();
	let bytes = span_bytes + value.allocated_bytes();
	if bytes > ENTRY_BYTES {
		return None;
	}
	let mut cache = cache.lock().expect("RTL cache lock");
	while cache.entries.len() >= CACHE_ITEMS || cache.bytes + bytes > CACHE_BYTES {
		let old = cache.entries.pop_front()?;
		cache.bytes -= old.bytes;
	}
	cache.bytes += bytes;
	cache.entries.push_back(Entry {
		spans: stored,
		width: width.to_bits(),
		scale,
		fonts,
		discovered,
		atlas_generation,
		value: value.clone(),
		bytes,
	});
	Some(value)
}

#[cfg(test)]
mod tests {
	use super::*;
	fn span(text: &str) -> Span {
		Span {
			text: text.into(),
			format: TextFormat::simple(egui::FontId::proportional(18.0), Color32::WHITE),
			action: 0,
			object: None,
			copy: true,
		}
	}
	fn native(text: &str, width: f32) -> Arc<Layout> {
		let ctx = egui::Context::default();
		crate::fonts::install(&ctx);
		let mut result = None;
		ctx.run_ui(egui::RawInput::default(), |ui| {
			result = layout(ui.ctx(), &[span(text)], width)
		})
		.drop_without_applying_deltas();
		result.expect("bounded RTL layout")
	}
	#[test]
	fn rtl_logical_line_breaks_keep_the_first_arabic_word_on_the_first_row() {
		let text = "مرحبا بكم في محادثة تجريبية طويلة لا تحتوي على بيانات حقيقية وتختبر ترتيب السطور والكلمات العربية";
		let value = native(text, 170.0);
		assert!(value.lines.len() > 2);
		assert_eq!(value.source, text);
		let first = value
			.cells
			.iter()
			.filter(|cell| cell.rect.center().y < value.lines[0].galley.size().y)
			.collect::<Vec<_>>();
		assert_eq!(first.iter().map(|cell| cell.source.start).min(), Some(0));
		let rightmost = first
			.iter()
			.max_by(|a, b| a.rect.right().total_cmp(&b.rect.right()))
			.unwrap();
		assert_eq!(rightmost.source.start, 0);
		let mut ranges = value
			.cells
			.iter()
			.map(|cell| cell.source.clone())
			.collect::<Vec<_>>();
		ranges.sort_by_key(|range| range.start);
		ranges.dedup();
		let mut end = 0;
		for range in ranges {
			assert_eq!(range.start, end);
			end = range.end;
		}
		assert_eq!(end, text.len());
	}
	#[test]
	fn rtl_native_clusters_preserve_arabic_source_and_indivisible_combining_marks() {
		let letters = native("لا", 200.0);
		let mut letter_ranges: Vec<_> = letters
			.cells
			.iter()
			.map(|cell| cell.source.clone())
			.collect();
		letter_ranges.sort_by_key(|range| range.start);
		assert_eq!(
			letter_ranges
				.iter()
				.map(|range| &letters.source[range.clone()])
				.collect::<String>(),
			"لا"
		);
		let marked = native("عَرَبِيَّة", 200.0);
		for cell in &marked.cells {
			assert!(marked.source.is_char_boundary(cell.source.start));
			assert!(marked.source.is_char_boundary(cell.source.end));
		}
		assert!(
			marked
				.cells
				.iter()
				.any(|cell| marked.source[cell.source.clone()].chars().count() > 1)
		);
		for cell in &marked.cells {
			let left = marked.cursor(egui::pos2(cell.rect.left() + 0.01, cell.rect.center().y));
			let right = marked.cursor(egui::pos2(cell.rect.right() - 0.01, cell.rect.center().y));
			assert_eq!(
				(left, right),
				(cell.source.end, cell.source.start),
				"pointer affinity never splits the shaper's base/mark cluster"
			);
		}
		let mut ranges = marked
			.cells
			.iter()
			.map(|cell| cell.source.clone())
			.collect::<Vec<_>>();
		ranges.sort_by_key(|range| range.start);
		ranges.dedup();
		assert_eq!(
			ranges
				.iter()
				.map(|range| &marked.source[range.clone()])
				.collect::<String>(),
			marked.source
		);
	}
	#[test]
	fn rtl_mixed_script_clusters_keep_latin_digits_and_hebrew_in_logical_copy_order() {
		for text in [
			"مرحبا English 123 (نص)",
			"שלום English 123 (עברית)",
			"English العربية 123 ending",
			"שלום العربية שלום",
		] {
			let value = native(text, 180.0);
			assert_eq!(value.source, text);
			for line in &value.lines {
				for section in &line.galley.job.sections {
					let text =
						&line.galley.job.text[section.byte_range.start.0..section.byte_range.end.0];
					let hebrew = text
						.chars()
						.any(|chr| unicode_bidi::bidi_class(chr) == unicode_bidi::BidiClass::R);
					let arabic = text
						.chars()
						.any(|chr| unicode_bidi::bidi_class(chr) == unicode_bidi::BidiClass::AL);
					assert!(
						!(hebrew && arabic),
						"shared installed faces must not guess one shaping script across Hebrew/Arabic transitions"
					);
				}
			}
			for cell in &value.cells {
				if text[cell.source.clone()]
					.chars()
					.any(|chr| chr.is_ascii_alphanumeric())
				{
					assert!(!cell.rtl);
				}
			}
		}
	}
	#[test]
	fn rtl_cache_rebuilds_native_meshes_after_font_atlas_reset() {
		let ctx = egui::Context::default();
		crate::fonts::install(&ctx);
		let mut previous = None;
		let mut generation = None;
		for reset in [false, true, false] {
			if reset {
				ctx.global_style_mut(|style| {
					style.visuals.text_options.font_hinting =
						!style.visuals.text_options.font_hinting;
				});
			}
			ctx.run_ui(egui::RawInput::default(), |ui| {
				let current_generation = ui.fonts(|fonts| fonts.layout_generation());
				let value = layout(ui.ctx(), &[span("مرحبا English")], 180.0).unwrap();
				if let Some(previous) = &previous {
					assert_eq!(Arc::ptr_eq(previous, &value), !reset);
					assert_eq!(generation == Some(current_generation), !reset);
				}
				assert!(value.lines.iter().any(|line| {
					line.galley
						.rows
						.iter()
						.any(|row| !row.visuals.mesh.vertices.is_empty())
				}));
				previous = Some(value);
				generation = Some(current_generation);
			})
			.drop_without_applying_deltas();
		}
	}
	#[test]
	fn rtl_cache_rebuilds_after_same_count_font_definitions_are_replaced() {
		let ctx = egui::Context::default();
		crate::fonts::install(&ctx);
		let mut previous = None;
		let mut generation = None;
		let mut revision = None;
		for replace in [false, true] {
			if replace {
				let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
				let family = definitions
					.families
					.get_mut(&egui::FontFamily::Proportional)
					.unwrap();
				assert!(family.len() > 1);
				family.reverse();
				ctx.set_fonts(definitions);
			}
			ctx.run_ui(egui::RawInput::default(), |ui| {
				let value = layout(ui.ctx(), &[span("مرحبا English")], 180.0).unwrap();
				let current_generation = ui.fonts(|fonts| fonts.layout_generation());
				let current_revision = crate::fonts::revision(ui.ctx());
				if let Some(previous) = &previous {
					assert_eq!(
						revision,
						Some(current_revision),
						"same-count non-custom replacement must exercise the old key collision"
					);
					assert_ne!(generation, Some(current_generation));
					assert!(!Arc::ptr_eq(previous, &value));
				}
				previous = Some(value);
				generation = Some(current_generation);
				revision = Some(current_revision);
			})
			.drop_without_applying_deltas();
		}
	}
	#[test]
	fn rtl_cache_is_bounded_by_items_and_allocated_bytes_and_reuses_exact_inputs() {
		let ctx = egui::Context::default();
		crate::fonts::install(&ctx);
		ctx.run_ui(egui::RawInput::default(), |ui| {
			let spans = [span("مرحبا English")];
			let a = layout(ui.ctx(), &spans, 180.0).unwrap();
			let b = layout(ui.ctx(), &spans, 180.0).unwrap();
			assert!(Arc::ptr_eq(&a, &b));
			for index in 0..64 {
				assert!(
					layout(ui.ctx(), &[span(&format!("مرحبا تجريبي {index}"))], 170.0).is_some()
				);
			}
			let cache = ctx
				.data(|data| {
					data.get_temp::<Arc<std::sync::Mutex<Cache>>>(egui::Id::unique(
						"bounded-rtl-layout-cache",
					))
				})
				.unwrap();
			let cache = cache.lock().unwrap();
			assert!(cache.entries.len() <= CACHE_ITEMS);
			assert!(cache.bytes <= CACHE_BYTES);
			drop(cache);
			assert!(layout(ui.ctx(), &[span(&"ع".repeat(MAX_BYTES))], 170.0).is_none());
		})
		.drop_without_applying_deltas();
	}
	#[test]
	fn rtl_hard_line_admission_preserves_crlf_and_unicode_separators_without_shaper_panics() {
		let text = "مرحبا\r\nالعالم\u{2028}שלום\u{2029}English العربية";
		let value = native(text, 400.0);
		assert_eq!(value.source, text);
		assert_eq!(value.lines.len(), 4);
		let blank = native("مرحبا\u{2028}\u{2028}العالم", 400.0);
		assert_eq!(blank.lines.len(), 2);
		assert!(blank.lines[1].position.y > blank.lines[0].galley.size().y * 1.5);
		let ctx = egui::Context::default();
		crate::fonts::install(&ctx);
		ctx.run_ui(egui::RawInput::default(), |ui| {
			assert!(
				layout(
					ui.ctx(),
					&[span(&format!("مرحبا{}", "\n".repeat(MAX_LINES + 1)))],
					120.0
				)
				.is_none()
			);
			assert!(layout(ui.ctx(), &[span(&"مرحبا ".repeat(512))], 15.0).is_none());
			assert!(layout(ui.ctx(), &[span("العربية")], 1.0).is_none());
			assert!(
				layout(
					ui.ctx(),
					&[span(&format!("مرحبا{}", "\u{2028}".repeat(MAX_LINES + 1)))],
					120.0
				)
				.is_none()
			);
		})
		.drop_without_applying_deltas();
	}
}

#[derive(Default)]
struct CacheLife {
	used: bool,
	cache: std::sync::Weak<std::sync::Mutex<Cache>>,
}
impl egui::Plugin for CacheLife {
	fn debug_name(&self) -> &'static str {
		"Bounded RTL cache lifetime"
	}
	fn on_begin_pass(&mut self, _: &mut egui::Ui) {
		self.used = false;
	}
	fn on_end_pass(&mut self, _: &mut egui::Ui) {
		if !self.used
			&& let Some(cache) = self.cache.upgrade()
		{
			let mut cache = cache.lock().expect("RTL cache lock");
			if !cache.entries.is_empty() {
				cache.entries.clear();
				cache.bytes = 0;
				// Selection's own pass cleanup retires missing mapped endpoints;
				// do not erase a new ordinary-label selection on this idle pass.
			}
		}
	}
}

/// Context storage is session-only: switching conversations/accounts retires private layouts.
pub(crate) fn scope(
	ctx: &egui::Context,
	generation: u64,
	owner: Option<model::Id>,
	channel: Option<model::Id>,
) {
	let current = (generation, owner, channel);
	let id = egui::Id::unique("rtl-layout-session-scope");
	let changed = ctx.data_mut(|data| {
		let previous = data.get_temp::<(u64, Option<model::Id>, Option<model::Id>)>(id);
		data.insert_temp(id, current);
		previous.is_some_and(|previous| previous != current)
	});
	if changed {
		if let Some(cache) = ctx.data(|data| {
			data.get_temp::<Arc<std::sync::Mutex<Cache>>>(egui::Id::unique(
				"bounded-rtl-layout-cache",
			))
		}) {
			let mut cache = cache.lock().expect("RTL cache lock");
			cache.entries.clear();
			cache.bytes = 0;
		}
		crate::select::clear(ctx);
	}
}
