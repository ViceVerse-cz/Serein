//! Inline artwork with the original wire text retained in egui's editing/undo model.
use crate::{avatars::Avatars, emoji};
use egui::{Color32, Image, text::CCursor, text::LayoutJob, text::TextFormat};
use model::User;
use std::{
	hash::{DefaultHasher, Hash, Hasher},
	ops::Range,
	sync::Arc,
};
use unicode_segmentation::UnicodeSegmentation;

struct Inline {
	source: Range<usize>,
	projected: usize,
	width: f32,
	label: Option<Arc<egui::Galley>>,
	background: Color32,
	image: Option<Image<'static>>,
	asset: Option<model::ImageShare>,
	icon: Option<crate::icons::Icon>,
	game_icon: Option<String>,
}

#[derive(Default)]
pub(crate) struct Layout {
	inlines: Vec<Inline>,
	cache: Option<(u64, Arc<egui::Galley>)>,
	demo: bool,
	bidi: Option<Bidi>,
	/// The pointer gesture in progress over a right-to-left draft.
	bidi_gesture: Option<Gesture>,
}

/// A press selects from a char; a double-click selects whole words while the button is held.
#[derive(Clone)]
enum Gesture {
	Chars(usize),
	Words(Range<usize>),
}

/// The word (or space/punctuation run) holding char `index`, as a char range.
fn word_at(text: &str, index: usize) -> Range<usize> {
	let byte = text
		.char_indices()
		.nth(index)
		.map_or(text.len(), |(byte, _)| byte);
	let chars = |byte: usize| text[..byte].chars().count();
	text.split_word_bound_indices()
		.find(|(from, word)| (*from..from + word.len()).contains(&byte))
		.map_or(index..index, |(from, word)| {
			chars(from)..chars(from + word.len())
		})
}

/// The bidi paragraph holding byte `byte`; the end of the text belongs to the last one.
fn paragraph_at<'a>(
	bidi: &'a unicode_bidi::BidiInfo<'_>,
	byte: usize,
) -> Option<&'a unicode_bidi::ParagraphInfo> {
	bidi.paragraphs
		.iter()
		.find(|paragraph| paragraph.range.contains(&byte))
		.or_else(|| bidi.paragraphs.last().filter(|p| p.range.end == byte))
}

/// Visual placement of a draft with right-to-left text. egui's editor assumes glyphs run left
/// to right in logical order, so for these drafts the composer reorders each row, paints its
/// own caret and selection, and maps the pointer back to logical characters.
struct Bidi {
	/// Per source char: its visual cell within its row.
	cells: Vec<Cell>,
	rows: Vec<BidiRow>,
}

#[derive(Clone, Copy, Default)]
struct Cell {
	left: f32,
	width: f32,
	rtl: bool,
}

struct BidiRow {
	chars: Range<usize>,
	rect: egui::Rect,
	rtl: bool,
}

/// True when `text` has right-to-left letters, so the composer lays it out in visual order.
pub(crate) fn has_rtl(text: &str) -> bool {
	text.chars().any(|chr| {
		matches!(
			unicode_bidi::bidi_class(chr),
			unicode_bidi::BidiClass::R | unicode_bidi::BidiClass::AL
		)
	})
}

/// Whether the paragraph holding char `index` of `text` reads right to left.
pub(crate) fn paragraph_rtl(text: &str, index: usize) -> bool {
	let byte = text
		.char_indices()
		.nth(index)
		.map_or(text.len(), |(byte, _)| byte);
	let bidi = unicode_bidi::BidiInfo::new(text, None);
	paragraph_at(&bidi, byte).is_some_and(|paragraph| paragraph.level.is_rtl())
}

impl Layout {
	#[cfg(any(test, debug_assertions))]
	#[allow(clippy::too_many_arguments)]
	pub fn galley(
		&mut self,
		ui: &egui::Ui,
		text: &str,
		width: f32,
		users: &[User],
		roles: &[model::permissions::Role],
		channels: &[model::Channel],
		mass_mentions: bool,
		avatars: &mut Avatars,
		demo: bool,
	) -> Arc<egui::Galley> {
		self.galley_games(
			ui,
			text,
			width,
			users,
			roles,
			channels,
			&Default::default(),
			mass_mentions,
			avatars,
			demo,
		)
	}
	#[allow(clippy::too_many_arguments)]
	pub fn galley_games(
		&mut self,
		ui: &egui::Ui,
		text: &str,
		width: f32,
		users: &[User],
		roles: &[model::permissions::Role],
		channels: &[model::Channel],
		games: &client_core::game_mentions::Catalog,
		mass_mentions: bool,
		avatars: &mut Avatars,
		demo: bool,
	) -> Arc<egui::Galley> {
		self.demo = demo;
		let mut key = DefaultHasher::new();
		text.hash(&mut key);
		games.revision.hash(&mut key);
		width.to_bits().hash(&mut key);
		crate::fonts::revision(ui.ctx()).hash(&mut key);
		ui.ctx().pixels_per_point().to_bits().hash(&mut key);
		egui::TextStyle::Body.resolve(ui.style()).hash(&mut key);
		let colors = crate::design::palette(ui);
		colors.text.hash(&mut key);
		colors.accent.hash(&mut key);
		colors.mention_text.hash(&mut key);
		colors.mention_bg.hash(&mut key);
		colors.muted.hash(&mut key);
		avatars.revision.hash(&mut key);
		emoji::ready(ui.ctx()).hash(&mut key);
		demo.hash(&mut key);
		if text.contains("<:") || text.contains("<a:") || text.contains("](https://") {
			static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
			(START
				.get_or_init(std::time::Instant::now)
				.elapsed()
				.as_secs() / 60)
				.hash(&mut key);
		}
		for user in users {
			user.id.hash(&mut key);
			user.name.hash(&mut key);
		}
		for role in roles {
			role.id.hash(&mut key);
			role.name.hash(&mut key);
		}
		for (start, _) in text.match_indices("<#") {
			if let Some((id, _)) = model::channel_mention_prefix(&text[start..]) {
				channels
					.iter()
					.find(|c| c.id == id)
					.map(|c| (&c.name, crate::channel_pill::icon(c, channels) as u8))
					.hash(&mut key);
			}
		}
		mass_mentions.hash(&mut key);
		let key = key.finish();
		if let Some((cached, galley)) = &self.cache
			&& *cached == key
		{
			return galley.clone();
		}
		self.inlines.clear();
		let font = egui::TextStyle::Body.resolve(ui.style());
		let colors = crate::design::palette(ui);
		let format = TextFormat::simple(font.clone(), colors.text);
		let size = emoji::inline_size(ui);
		let mut job = LayoutJob::default();
		job.wrap.max_width = width;
		if text.is_empty() {
			job.append("", 0.0, format.clone());
		}
		let mut byte = 0;
		let mut source = 0;
		let mut projected = 0;
		while byte < text.len() {
			let tail = &text[byte..];
			let mut label = None;
			let mut background = colors.accent.gamma_multiply(0.12);
			let mut image = None;
			let mut asset = None;
			let mut icon = None;
			let mut game_icon = None;
			let mut artwork = false;
			let length = if let Some((id, len)) = model::game_mention_prefix(tail) {
				label = Some((games.label(id), colors.mention_text));
				background = colors.mention_bg;
				icon = Some(crate::icons::Icon::GameController);
				game_icon = games.find(id).map(model::GameMention::icon_key);
				len
			} else if tail.starts_with("@game ")
				&& text[..byte]
					.chars()
					.next_back()
					.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{'))
			{
				label = Some(("@game".into(), colors.text));
				background = colors.selected;
				5
			} else if let Some((id, len)) = model::user_mention_prefix(tail) {
				let name = users
					.iter()
					.find(|u| u.id == id)
					.map_or_else(|| id.to_string(), |user| user.name.clone());
				label = Some((format!("@{name}"), colors.mention_text));
				background = colors.mention_bg;
				len
			} else if let Some((id, len)) = model::role_mention_prefix(tail) {
				let name = roles
					.iter()
					.find(|role| role.id == id)
					.map_or_else(|| format!("unknown-role ({id})"), |role| role.name.clone());
				label = Some((format!("@{name}"), colors.mention_text));
				background = colors.mention_bg;
				len
			} else if let Some((id, len)) = model::channel_mention_prefix(tail) {
				let channel = channels.iter().find(|c| c.id == id && c.guild.is_some());
				let name =
					channel.map_or_else(|| format!("unknown-channel ({id})"), |c| c.name.clone());
				icon = Some(channel.map_or(crate::icons::Icon::Hash, |c| {
					crate::channel_pill::icon(c, channels)
				}));
				label = Some((name, colors.mention_text));
				background = colors.mention_bg;
				len
			} else if mass_mentions && let Some(len) = model::mass_mention_prefix(tail) {
				label = Some((tail[..len].to_owned(), colors.mention_text));
				background = colors.mention_bg;
				len
			} else if let Some((share, name, len)) = model::ImageShare::markdown_prefix(tail) {
				asset = Some(share);
				image = avatars.lookup_share_image(ui.ctx(), share, size, demo);
				if image.is_none() {
					label = Some((name, colors.muted));
				}
				len
			} else if let Some((id, len)) = emoji::custom_prefix(tail) {
				asset = Some(model::ImageShare::Emoji {
					id,
					animated: false,
				});
				image = avatars.lookup_share_image(ui.ctx(), asset.unwrap(), size, demo);
				// A useful name remains visible while artwork is unavailable/loading.
				if image.is_none() {
					let name = tail[..len]
						.trim_start_matches("<a:")
						.trim_start_matches("<:")
						.split(':')
						.next()
						.unwrap_or("emoji");
					label = Some((format!(":{name}:"), colors.muted));
				}
				len
			} else {
				let grapheme = tail.graphemes(true).next().expect("nonempty tail");
				image = emoji::image(ui.ctx(), grapheme, size);
				artwork = image.is_some() || emoji::lookup(grapheme).is_some();
				grapheme.len()
			};
			let label = label.map(|(text, color)| {
				let mut job = LayoutJob::simple_singleline(text, font.clone(), color);
				job.wrap.max_width =
					(width - 6.0 - if icon.is_some() { size } else { 0.0 }).max(1.0);
				job.wrap.max_rows = 1;
				ui.fonts_mut(|f| f.layout_job(job))
			});
			let raw = &tail[..length];
			let count = raw.chars().count();
			if label.is_some() || image.is_some() || artwork {
				let slot = label.as_ref().map_or(size, |g| {
					g.size().x + 6.0 + if icon.is_some() { size } else { 0.0 }
				});
				// One blank glyph forms an unbroken inline object, even at a row break.
				// Expand its character slots below, so native selection/copy/undo use wire text.
				job.append(" ", 0.0, emoji::inline_format(ui, slot, size));
				self.inlines.push(Inline {
					source: source..source + count,
					projected,
					width: slot,
					label,
					background,
					image,
					asset,
					icon,
					game_icon,
				});
				projected += 1;
			} else {
				job.append(raw, 0.0, format.clone());
				projected += count;
			}
			byte += length;
			source += count;
		}
		let mut galley = ui.fonts_mut(|f| f.layout_job(job));
		let reordered = has_rtl(text)
			.then(|| reorder(ui, &mut galley, width))
			.flatten();
		if self.inlines.is_empty() {
			self.bidi = reordered.map(|(cells, rtl)| Bidi::new(cells, &rtl, &galley, &[]));
			self.cache = Some((key, galley.clone()));
			return galley;
		}
		let galley_mut = Arc::make_mut(&mut galley);
		let chars: Vec<_> = text.chars().collect();
		let mut projected = 0;
		let mut next = 0;
		for placed in &mut galley_mut.rows {
			let row = Arc::make_mut(&mut placed.row);
			let mut glyphs = Vec::with_capacity(row.glyphs.len());
			for glyph in &row.glyphs {
				if let Some(inline) = self.inlines.get(next).filter(|i| i.projected == projected) {
					let count = inline.source.len();
					for (index, chr) in chars[inline.source.clone()].iter().enumerate() {
						let mut slot = *glyph;
						slot.chr = *chr;
						slot.pos.x = glyph.pos.x + inline.width * index as f32 / count as f32;
						slot.advance_width = inline.width / count as f32;
						glyphs.push(slot);
					}
					next += 1;
				} else {
					glyphs.push(*glyph);
				}
				projected += 1;
			}
			row.glyphs = glyphs;
			projected += usize::from(placed.ends_with_newline);
		}
		// TextEdit's galley and TextBuffer must expose exactly the same original characters.
		galley_mut.job = Arc::new(LayoutJob::simple(text.to_owned(), font, colors.text, width));
		self.bidi = reordered.map(|(cells, rtl)| Bidi::new(cells, &rtl, &galley, &self.inlines));
		self.cache = Some((key, galley.clone()));
		galley
	}

	pub fn paint(
		&self,
		ui: &mut egui::Ui,
		output: &egui::text_edit::TextEditOutput,
		avatars: &mut Avatars,
	) {
		// TextEdit keeps the empty hint galley on the first keystroke.
		if self
			.cache
			.as_ref()
			.is_none_or(|(_, galley)| galley.job.text != output.galley.job.text)
		{
			return;
		}
		// The editor is taller than a scrolled composer; stay inside the scroll viewport too.
		let clip = output.text_clip_rect.intersect(ui.clip_rect());
		let painter = ui.painter().with_clip_rect(clip);
		for inline in &self.inlines {
			let mut cursor = CCursor::new(inline.source.start);
			cursor.prefer_next_row = true;
			let position = output
				.galley
				.pos_from_cursor(cursor)
				.translate(output.galley_pos.to_vec2());
			let rect = egui::Rect::from_min_size(
				position.min,
				egui::vec2(inline.width, position.height()),
			);
			if !rect.intersect(clip).is_positive() {
				continue;
			}
			if let Some(asset) = inline.asset {
				// Only visible slots may request or refresh artwork, even on a cache hit.
				avatars.touch_share_image(ui.ctx(), asset, self.demo);
			}
			if let Some(label) = &inline.label {
				painter.rect_filled(rect, 3, inline.background);
				let offset = if let Some(icon) = inline.icon {
					let size = emoji::inline_size(ui);
					if let Some(key) = &inline.game_icon {
						let icon_rect = egui::Rect::from_center_size(
							egui::pos2(rect.left() + 3.0 + size / 2.0, rect.center().y),
							egui::Vec2::splat(size),
						);
						let mut child = ui.new_child(egui::UiBuilder::new().max_rect(icon_rect));
						child.set_clip_rect(clip);
						avatars.show_icon(&mut child, Some(key.clone()), size, self.demo, "Game");
					} else {
						crate::icons::paint(
							&painter,
							icon,
							egui::Rect::from_center_size(
								egui::pos2(rect.left() + 3.0 + size / 2.0, rect.center().y),
								egui::Vec2::splat(size * 0.65),
							),
							crate::design::palette(ui).mention_text,
						);
					}
					size
				} else {
					0.0
				};
				painter.galley(
					egui::pos2(
						rect.left() + 3.0 + offset,
						rect.center().y - label.size().y / 2.0,
					),
					label.clone(),
					Color32::PLACEHOLDER,
				);
			} else if let Some(image) = &inline.image {
				let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
				child.set_clip_rect(clip);
				image.paint_at(
					&child,
					egui::Rect::from_center_size(
						rect.center(),
						image.calc_size(egui::Vec2::splat(inline.width), image.size()),
					),
				);
			}
		}
	}

	/// Keep mouse/arrow selection endpoints outside encoded inline objects.
	pub fn snap_cursor(&self, output: &mut egui::text_edit::TextEditOutput, ctx: &egui::Context) {
		let Some(mut range) = output.cursor_range else {
			return;
		};
		let original = range;
		let direction = ctx.input(|i| {
			if i.key_pressed(egui::Key::ArrowLeft) {
				-1
			} else if i.key_pressed(egui::Key::ArrowRight) {
				1
			} else {
				0
			}
		});
		for cursor in [&mut range.primary, &mut range.secondary] {
			for inline in &self.inlines {
				let index = cursor.index.0;
				if inline.source.start < index && index < inline.source.end {
					cursor.index = if direction < 0
						|| (direction == 0
							&& index - inline.source.start < inline.source.end - index)
					{
						inline.source.start.into()
					} else {
						inline.source.end.into()
					};
					break;
				}
			}
		}
		if range != original {
			output.cursor_range = Some(range);
			output.state.cursor.set_char_range(Some(range));
			output.state.clone().store(ctx, output.response.id);
		}
	}

	/// Let the native editor delete a selected token, preserving its normal undo history.
	pub fn select_deleted_inline(&self, ctx: &egui::Context, id: egui::Id) {
		if !ctx.memory(|m| m.has_focus(id)) {
			return;
		}
		let Some(mut state) = egui::text_edit::TextEditState::load(ctx, id) else {
			return;
		};
		let Some(range) = state.cursor.char_range().filter(|r| r.is_empty()) else {
			return;
		};
		let (back, forward) = ctx.input(|i| {
			(
				i.key_pressed(egui::Key::Backspace),
				i.key_pressed(egui::Key::Delete),
			)
		});
		if let Some(inline) = self.inlines.iter().find(|i| {
			(back && i.source.end == range.primary.index.0)
				|| (forward && i.source.start == range.primary.index.0)
		}) {
			state
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::two(
					CCursor::new(inline.source.start),
					CCursor::new(inline.source.end),
				)));
			state.store(ctx, id);
		}
	}
}

/// Lay out a draft holding right-to-left text in visual order. Lines break in logical order
/// (wrapping at spaces by measured cluster advances), then Unicode bidi rules L1/L2 apply per
/// row and each row is shaped once as a directional line. egui's own rows cannot be reordered:
/// it emits one glyph per shaped glyph, so an Arabic letter drawn as a base and a dot becomes two.
/// Each row here has one glyph per char, in logical order at its visual position, and rows of
/// right-to-left paragraphs are right-aligned. Returns each projected char's cell and each
/// row's paragraph direction.
fn reorder(
	ui: &egui::Ui,
	galley: &mut Arc<egui::Galley>,
	width: f32,
) -> Option<(Vec<Cell>, Vec<bool>)> {
	use egui::epaint::text::PlacedRow;
	let job = galley.job.clone();
	let text = job.text.as_str();
	let bidi = unicode_bidi::BidiInfo::new(text, None);
	if !bidi.has_rtl() || text.contains('\r') {
		return None;
	}
	let offsets: Vec<usize> = text
		.char_indices()
		.map(|(byte, _)| byte)
		.chain([text.len()])
		.collect();
	let char_at = |byte: usize| offsets.partition_point(|offset| *offset < byte);
	let mut cells = vec![Cell::default(); offsets.len() - 1];
	let mut directions = Vec::new();
	let mut rows = Vec::new();
	let mut y = 0.0;
	let mut place = |row: egui::epaint::text::Row, rtl: bool, ends_with_newline: bool| {
		let x = if rtl && width.is_finite() {
			(width - row.size.x).max(0.0)
		} else {
			0.0
		};
		let height = row.size.y;
		rows.push(PlacedRow {
			pos: egui::pos2(x, y),
			row: Arc::new(row),
			ends_with_newline,
		});
		directions.push(rtl);
		y += height;
	};
	let mut hard_start = 0;
	for hard in text.split('\n') {
		let line = hard_start..hard_start + hard.len();
		hard_start = line.end + 1;
		let ends_with_newline = line.end < text.len();
		let paragraph = paragraph_at(&bidi, line.start)?;
		let rtl = paragraph.level.is_rtl();
		if line.is_empty() {
			let (row, _) = shape_line(ui, &job, &bidi, paragraph, line);
			place(row, rtl, ends_with_newline);
			continue;
		}
		// Advances of the whole hard line choose the logical breaks; rows are shaped again.
		let (_, measured) = shape_line(ui, &job, &bidi, paragraph, line.clone());
		let mut advances: Vec<(Range<usize>, f32)> = measured
			.into_iter()
			.map(|(range, rect, _)| (range, rect.width()))
			.collect();
		advances.sort_by_key(|(range, _)| range.start);
		let mut start = line.start;
		while start < line.end {
			let mut end = start;
			let mut used = 0.0;
			let mut word = None;
			for (range, advance) in advances.iter().filter(|(range, _)| range.start >= start) {
				if used + advance > width && end > start {
					break;
				}
				used += advance;
				end = range.end;
				if text[start..end].ends_with(char::is_whitespace) {
					word = Some(end);
				}
			}
			if end < line.end
				&& let Some(word) = word
			{
				end = word;
			}
			if end <= start {
				// A cluster wider than the composer still gets its own row.
				end = text[start..]
					.chars()
					.next()
					.map_or(line.end, |c| start + c.len_utf8());
			}
			let (mut row, clusters) = shape_line(ui, &job, &bidi, paragraph, start..end);
			for (range, rect, rtl) in clusters {
				let chars = char_at(range.start)..char_at(range.end);
				let share = rect.width() / chars.len().max(1) as f32;
				for (index, char) in chars.enumerate() {
					let left = if rtl {
						rect.max.x - share * (index + 1) as f32
					} else {
						rect.min.x + share * index as f32
					};
					cells[char] = Cell {
						left,
						width: share,
						rtl,
					};
				}
			}
			let template = row.glyphs.first().copied();
			let glyph_end = row.visuals.glyph_vertex_range.end as u32;
			row.glyphs = template
				.map(|template| {
					text[start..end]
						.chars()
						.zip(&cells[char_at(start)..char_at(end)])
						.map(|(chr, cell)| {
							let mut glyph = template;
							glyph.chr = chr;
							glyph.pos.x = cell.left;
							glyph.advance_width = cell.width;
							// egui recolours selected glyphs by vertex order; the composer
							// paints the selection itself.
							glyph.first_vertex = glyph_end;
							glyph
						})
						.collect()
				})
				.unwrap_or_default();
			start = end;
			place(row, rtl, ends_with_newline && start >= line.end);
		}
	}
	let galley_mut = Arc::make_mut(galley);
	galley_mut.rect = rows.iter().fold(egui::Rect::NOTHING, |rect, placed| {
		rect.union(egui::Rect::from_min_size(placed.pos, placed.row.size))
	});
	galley_mut.mesh_bounds = rows.iter().fold(egui::Rect::NOTHING, |rect, placed| {
		rect.union(
			placed
				.row
				.visuals
				.mesh_bounds
				.translate(placed.pos.to_vec2()),
		)
	});
	galley_mut.num_vertices = rows.iter().map(|p| p.row.visuals.mesh.vertices.len()).sum();
	galley_mut.num_indices = rows.iter().map(|p| p.row.visuals.mesh.indices.len()).sum();
	galley_mut.elided = false;
	galley_mut.rows = rows;
	Some((cells, directions))
}

/// Shape `line` (job bytes inside one bidi paragraph) as one visual-order line. Returns its row
/// and each native cluster's job bytes, row-local rect and direction.
fn shape_line(
	ui: &egui::Ui,
	job: &LayoutJob,
	bidi: &unicode_bidi::BidiInfo<'_>,
	paragraph: &unicode_bidi::ParagraphInfo,
	line: Range<usize>,
) -> (
	egui::epaint::text::Row,
	Vec<(Range<usize>, egui::Rect, bool)>,
) {
	use egui::text::{ByteIndex, LayoutSection};
	let text = job.text.as_str();
	let mut shaped = LayoutJob::default();
	let mut pieces = Vec::new();
	let mut directions = Vec::new();
	if line.is_empty() {
		// An empty row still takes the height of the text around it.
		let format = job
			.sections
			.first()
			.map(|section| section.format.clone())
			.unwrap_or_default();
		shaped.sections.push(LayoutSection {
			leading_space: 0.0,
			byte_range: ByteIndex(0)..ByteIndex(0),
			format,
		});
		directions.push(paragraph.level.is_rtl());
	} else {
		let (levels, runs) = bidi.visual_runs(paragraph, line);
		for run in runs {
			let rtl = levels[run.start].is_rtl();
			let mut parts: Vec<_> = job
				.sections
				.iter()
				.filter_map(|section| {
					let from = run.start.max(section.byte_range.start.0);
					let to = run.end.min(section.byte_range.end.0);
					(from < to).then_some((from..to, &section.format))
				})
				.flat_map(|(range, format)| {
					crate::rtl::script_parts(text, range)
						.into_iter()
						.map(move |range| (range, format))
				})
				.collect();
			if rtl {
				parts.reverse();
			}
			for (range, format) in parts {
				let from = shaped.text.len();
				shaped.text.push_str(&text[range.clone()]);
				// append() merges equal formats and would erase a bidi boundary.
				shaped.sections.push(LayoutSection {
					leading_space: 0.0,
					byte_range: ByteIndex(from)..ByteIndex(shaped.text.len()),
					format: format.clone(),
				});
				directions.push(rtl);
				pieces.push((range, from, rtl));
			}
		}
	}
	let native = ui.fonts_mut(|fonts| fonts.layout_single_line_with_clusters(shaped, &directions));
	let placed = &native.galley.rows[0];
	let origin = placed.pos.to_vec2();
	let clusters = native
		.clusters
		.iter()
		.filter_map(|cluster| {
			let (range, from, rtl) = pieces.iter().find(|(range, from, _)| {
				(*from..from + range.len()).contains(&cluster.bytes.start)
			})?;
			let bytes =
				range.start + cluster.bytes.start - from..range.start + cluster.bytes.end - from;
			Some((bytes, cluster.rect.translate(-origin), *rtl))
		})
		.collect();
	((*placed.row).clone(), clusters)
}

impl Bidi {
	/// Expand projected cells to source chars: an inline object's wire text shares its slot.
	fn new(projected: Vec<Cell>, rtl: &[bool], galley: &egui::Galley, inlines: &[Inline]) -> Self {
		let mut cells = Vec::with_capacity(projected.len());
		let mut next = inlines.iter().peekable();
		for (index, cell) in projected.into_iter().enumerate() {
			match next.peek() {
				Some(inline) if inline.projected == index => {
					cells.extend(std::iter::repeat_n(cell, inline.source.len()));
					next.next();
				}
				_ => cells.push(cell),
			}
		}
		let mut rows = Vec::with_capacity(galley.rows.len());
		let mut start = 0;
		for (placed, rtl) in galley.rows.iter().zip(rtl) {
			let end = start + placed.row.glyphs.len();
			rows.push(BidiRow {
				chars: start..end,
				rect: egui::Rect::from_min_size(placed.pos, placed.row.size),
				rtl: *rtl,
			});
			start = end + usize::from(placed.ends_with_newline);
		}
		Self { cells, rows }
	}

	/// The caret's x for logical position `index`, on the side of the char it touches.
	fn caret(&self, index: usize) -> Option<(egui::Rect, f32)> {
		let row = self
			.rows
			.iter()
			.find(|row| row.chars.contains(&index) || row.chars.end == index)?;
		let x = if index < row.chars.end {
			let cell = self.cells.get(index)?;
			if cell.rtl {
				cell.left + cell.width
			} else {
				cell.left
			}
		} else if index > row.chars.start {
			let cell = self.cells.get(index - 1)?;
			if cell.rtl {
				cell.left
			} else {
				cell.left + cell.width
			}
		} else if row.rtl {
			row.rect.width()
		} else {
			0.0
		};
		Some((row.rect, row.rect.left() + x))
	}

	/// The logical position nearest `pos` (galley coordinates), on the visual side clicked.
	fn index_at(&self, pos: egui::Pos2) -> usize {
		let Some(row) = self.rows.iter().min_by(|a, b| {
			let distance =
				|row: &BidiRow| (pos.y - pos.y.clamp(row.rect.top(), row.rect.bottom())).abs();
			distance(a).total_cmp(&distance(b))
		}) else {
			return 0;
		};
		let x = pos.x - row.rect.left();
		let Some((index, cell)) = row
			.chars
			.clone()
			.map(|i| (i, self.cells[i]))
			.min_by(|a, b| {
				let distance = |cell: &Cell| (x - x.clamp(cell.left, cell.left + cell.width)).abs();
				distance(&a.1).total_cmp(&distance(&b.1))
			})
		else {
			return row.chars.start;
		};
		let before = (x < cell.left + cell.width / 2.0) != cell.rtl;
		if before { index } else { index + 1 }
	}
}

impl Layout {
	/// Whether `text` is laid out in visual order, so the composer draws its caret and selection.
	pub fn bidi_active(&self, text: &str) -> bool {
		self.bidi.is_some()
			&& self
				.cache
				.as_ref()
				.is_some_and(|(_, galley)| galley.job.text == text)
	}

	/// Map the pointer to logical characters for a right-to-left draft. egui hit-tests its
	/// galley assuming glyphs grow left to right, which picks the wrong char once rows reorder.
	pub fn bidi_pointer(
		&mut self,
		output: &mut egui::text_edit::TextEditOutput,
		ctx: &egui::Context,
	) {
		let Some(bidi) = &self.bidi else {
			self.bidi_gesture = None;
			return;
		};
		let response = &output.response;
		let Some(pointer) = response
			.interact_pointer_pos()
			.filter(|_| ctx.input(|i| i.pointer.primary_down() || i.pointer.primary_released()))
		else {
			self.bidi_gesture = None;
			return;
		};
		let index = bidi.index_at((pointer - output.galley_pos).to_pos2());
		let text = output.galley.job.text.as_str();
		let (pressed, shift) = ctx.input(|i| (i.pointer.primary_pressed(), i.modifiers.shift));
		if response.triple_clicked() {
			let chars: Vec<char> = text.chars().collect();
			let start = chars[..index.min(chars.len())]
				.iter()
				.rposition(|c| *c == '\n')
				.map_or(0, |i| i + 1);
			let end = chars[index.min(chars.len())..]
				.iter()
				.position(|c| *c == '\n')
				.map_or(chars.len(), |i| index + i);
			self.bidi_gesture = Some(Gesture::Words(start..end));
		} else if response.double_clicked() {
			self.bidi_gesture = Some(Gesture::Words(word_at(text, index)));
		} else if pressed || self.bidi_gesture.is_none() {
			self.bidi_gesture = Some(Gesture::Chars(match output.state.cursor.char_range() {
				Some(range) if shift => range.secondary.index.0,
				_ => index,
			}));
		}
		let range = match self.bidi_gesture.clone() {
			Some(Gesture::Words(words)) => {
				// Dragging after a double-click extends by whole words in either direction.
				let under = word_at(text, index.min(text.chars().count().saturating_sub(1)));
				if under.start < words.start {
					(words.end, under.start)
				} else if under.end > words.end {
					(words.start, under.end)
				} else {
					(words.start, words.end)
				}
			}
			Some(Gesture::Chars(anchor)) => (anchor, index),
			None => (index, index),
		};
		let range = egui::text::CCursorRange::two(CCursor::new(range.0), CCursor::new(range.1));
		if output.cursor_range != Some(range) {
			output.cursor_range = Some(range);
			output.state.cursor.set_char_range(Some(range));
			output.state.clone().store(ctx, output.response.id);
		}
	}

	/// Paint the caret and selection of a right-to-left draft; `slot` is reserved behind the text.
	pub fn paint_bidi(
		&self,
		ui: &egui::Ui,
		output: &egui::text_edit::TextEditOutput,
		slot: egui::layers::ShapeIdx,
	) {
		let Some(bidi) = &self.bidi else {
			return;
		};
		if !output.response.has_focus() {
			return;
		}
		let Some(range) = output.cursor_range else {
			return;
		};
		let origin = output.galley_pos.to_vec2();
		let clip = output.text_clip_rect.intersect(ui.clip_rect());
		let visuals = ui.visuals();
		let [min, max] = range.sorted_cursors();
		let selection: Vec<egui::Shape> = (min.index.0..max.index.0)
			.filter_map(|index| {
				let row = bidi.rows.iter().find(|row| row.chars.contains(&index))?;
				let cell = bidi.cells.get(index)?;
				let rect = egui::Rect::from_min_size(
					egui::pos2(row.rect.left() + cell.left, row.rect.top()),
					egui::vec2(cell.width, row.rect.height()),
				)
				.translate(origin)
				.intersect(clip);
				rect.is_positive()
					.then(|| egui::Shape::rect_filled(rect, 0.0, visuals.selection.bg_fill))
			})
			.collect();
		ui.painter().set(slot, selection);
		if let Some((row, x)) = bidi.caret(range.primary.index.0) {
			let top = egui::pos2(x, row.top()) + origin;
			let bottom = egui::pos2(x, row.bottom()) + origin;
			if clip.contains(top) || clip.contains(bottom) {
				ui.painter()
					.with_clip_rect(clip)
					.line_segment([top, bottom], visuals.text_cursor.stroke);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn users() -> Vec<User> {
		vec![User {
			id: model::Id(42),
			name: "Zoë".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
		}]
	}

	/// #918: right-to-left drafts read in visual order, and the caret and pointer follow it.
	#[test]
	fn rtl_drafts_are_drawn_in_visual_order_with_logical_hit_testing() {
		let ctx = egui::Context::default();
		crate::fonts::install(&ctx);
		let mut avatars = Avatars::default();
		let mut layout = Layout::default();
		let mut check = |text: &str, rtl_paragraph: bool| {
			let mut galley = None;
			ctx.run_ui(Default::default(), |ui| {
				galley =
					Some(layout.galley(ui, text, 300.0, &[], &[], &[], false, &mut avatars, true));
			})
			.drop_without_applying_deltas();
			let galley = galley.unwrap();
			let bidi = layout.bidi.as_ref().expect("RTL draft uses visual order");
			let row = &galley.rows[0];
			assert_eq!(
				row.row.glyphs.len(),
				text.chars().count(),
				"one glyph per char"
			);
			assert_eq!(
				row.pos.x > 0.0,
				rtl_paragraph,
				"only RTL paragraphs align right"
			);
			let x = |index: usize| bidi.caret(index).unwrap().1;
			let len = text.chars().count();
			if rtl_paragraph {
				assert!(x(0) > x(len), "reading starts at the right");
				assert_eq!(bidi.index_at(egui::pos2(x(0) + 20.0, 5.0)), 0);
				assert_eq!(bidi.index_at(egui::pos2(x(len) - 20.0, 5.0)), len);
			} else {
				// "abc שלום": Latin first on the left, then Hebrew right to left.
				assert!(x(0) < x(4), "Latin precedes the Hebrew word");
				assert!(x(4) > x(8), "the Hebrew word reads right to left");
				assert_eq!(bidi.index_at(egui::pos2(x(4) - 1.0, 5.0)), 4);
			}
		};
		check("مرحبا بك", true);
		check("abc שלום", false);
		// Wrapped and multi-line drafts: egui maps the cursor through one glyph per char.
		let text = "مرحبا بك في سيرين مرحبا بك في سيرين\n\nabc שלום";
		let mut galley = None;
		ctx.run_ui(Default::default(), |ui| {
			galley = Some(layout.galley(ui, text, 60.0, &[], &[], &[], false, &mut avatars, true));
		})
		.drop_without_applying_deltas();
		let galley = galley.unwrap();
		assert!(galley.rows.len() > 4, "the Arabic paragraph wraps");
		let mapped: usize = galley
			.rows
			.iter()
			.map(|row| row.row.glyphs.len() + usize::from(row.ends_with_newline))
			.sum();
		assert_eq!(mapped, text.chars().count());
		// A line after a right-to-left paragraph keeps its own direction.
		let text = "مرحبا\nabc";
		let mut galley = None;
		ctx.run_ui(Default::default(), |ui| {
			galley = Some(layout.galley(ui, text, 300.0, &[], &[], &[], false, &mut avatars, true));
		})
		.drop_without_applying_deltas();
		let rows = &galley.unwrap().rows;
		assert!(rows[0].pos.x > 0.0 && rows[1].pos.x == 0.0);
		assert!(paragraph_rtl(text, 0) && !paragraph_rtl(text, 6));
		ctx.run_ui(Default::default(), |ui| {
			layout.galley(ui, "plain", 300.0, &[], &[], &[], false, &mut avatars, true);
		})
		.drop_without_applying_deltas();
		assert!(
			layout.bidi.is_none(),
			"left-to-right drafts keep egui's own caret"
		);
	}

	/// A double-click selects the whole Arabic word, and the release frames keep it.
	#[test]
	fn rtl_double_click_keeps_the_whole_word() {
		let ctx = egui::Context::default();
		crate::fonts::install(&ctx);
		let mut avatars = Avatars::default();
		let mut layout = Layout::default();
		let mut text = "مرحبا بك".to_owned();
		let id = egui::Id::unique("rtl-double-click");
		let point = std::cell::Cell::new(egui::Pos2::ZERO);
		let mut frame = |time: f64, events: Vec<egui::Event>| {
			ctx.run_ui(
				egui::RawInput {
					time: Some(time),
					events,
					..Default::default()
				},
				|ui| {
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
							true,
						)
					};
					let mut output = egui::TextEdit::multiline(&mut text)
						.id(id)
						.layouter(&mut layouter)
						.show(ui);
					layout.bidi_pointer(&mut output, ui.ctx());
					// Inside the first word "مرحبا", between its second and third letters.
					let (row, x) = layout.bidi.as_ref().unwrap().caret(2).unwrap();
					point.set(egui::pos2(x, row.center().y) + output.galley_pos.to_vec2());
				},
			)
			.drop_without_applying_deltas();
		};
		frame(0.0, vec![]);
		let button = |pressed| egui::Event::PointerButton {
			pos: point.get(),
			button: egui::PointerButton::Primary,
			pressed,
			modifiers: egui::Modifiers::NONE,
		};
		frame(
			1.0,
			vec![egui::Event::PointerMoved(point.get()), button(true)],
		);
		for (time, pressed) in [(1.05, false), (1.1, true), (1.15, false)] {
			frame(time, vec![button(pressed)]);
		}
		frame(1.2, vec![]);
		let range = egui::text_edit::TextEditState::load(&ctx, id)
			.and_then(|state| state.cursor.char_range())
			.unwrap();
		let [min, max] = range.sorted_cursors();
		assert_eq!((min.index.0, max.index.0), (0, 5), "the whole first word");
	}

	#[test]
	fn native_delete_undo_copy_and_ime_keep_wire_mentions() {
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		let mut layout = Layout::default();
		let mut text = "hi <@42>!".to_owned();
		let id = egui::Id::unique("rich-editor-test");
		let mut frame = |time, events: Vec<egui::Event>, cursor: Option<usize>| {
			if let Some(cursor) = cursor {
				ctx.memory_mut(|m| m.request_focus(id));
				let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap_or_default();
				state
					.cursor
					.set_char_range(Some(egui::text::CCursorRange::one(CCursor::new(cursor))));
				state.store(&ctx, id);
			}
			let copying = events
				.iter()
				.any(|event| matches!(event, egui::Event::Copy));
			let composing = events
				.iter()
				.any(|event| matches!(event, egui::Event::Ime(_)));
			let output = ctx.run_ui(
				egui::RawInput {
					time: Some(time),
					events,
					..Default::default()
				},
				|ui| {
					layout.galley(
						ui,
						&text,
						300.0,
						&users(),
						&[],
						&[],
						false,
						&mut avatars,
						true,
					);
					layout.select_deleted_inline(&ctx, id);
					let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, width| {
						layout.galley(
							ui,
							buffer.as_str(),
							width,
							&users(),
							&[],
							&[],
							false,
							&mut avatars,
							true,
						)
					};
					let mut edit = egui::TextEdit::multiline(&mut text)
						.id(id)
						.layouter(&mut layouter)
						.show(ui);
					if !composing {
						layout.snap_cursor(&mut edit, &ctx);
					}
					layout.paint(ui, &edit, &mut avatars);
				},
			);
			if copying {
				assert!(output.platform_output.commands.iter().any(|command| {
					matches!(command, egui::OutputCommand::CopyText(copied) if copied == &text)
				}));
			}
			output.drop_without_applying_deltas();
			text.clone()
		};
		let key = |key, modifiers| egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers,
		};
		assert_eq!(frame(0.0, vec![], Some(8)), "hi <@42>!");
		assert_eq!(
			frame(
				1.0,
				vec![key(egui::Key::Backspace, egui::Modifiers::NONE)],
				None
			),
			"hi !"
		);
		assert_eq!(
			frame(2.0, vec![key(egui::Key::Z, egui::Modifiers::COMMAND)], None),
			"hi <@42>!"
		);
		frame(
			2.1,
			vec![key(egui::Key::ArrowLeft, egui::Modifiers::NONE)],
			Some(8),
		);
		assert_eq!(
			egui::text_edit::TextEditState::load(&ctx, id)
				.unwrap()
				.cursor
				.char_range()
				.unwrap()
				.primary
				.index
				.0,
			3
		);
		frame(
			2.2,
			vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)],
			None,
		);
		assert_eq!(
			egui::text_edit::TextEditState::load(&ctx, id)
				.unwrap()
				.cursor
				.char_range()
				.unwrap()
				.primary
				.index
				.0,
			8
		);
		assert_eq!(
			frame(3.0, vec![egui::Event::Text("č".into())], Some(8)),
			"hi <@42>č!"
		);
		assert_eq!(
			frame(
				4.0,
				vec![egui::Event::Ime(egui::ImeEvent::Preedit {
					text: "ni".into(),
					active_range_chars: None,
				})],
				Some(9)
			),
			"hi <@42>čni!"
		);
		assert_eq!(
			frame(
				5.0,
				vec![egui::Event::Ime(egui::ImeEvent::Commit("你".into()))],
				None
			),
			"hi <@42>č你!"
		);
		assert_eq!(
			frame(
				6.0,
				vec![
					key(egui::Key::A, egui::Modifiers::COMMAND),
					egui::Event::Copy
				],
				None
			),
			"hi <@42>č你!"
		);
	}
}
