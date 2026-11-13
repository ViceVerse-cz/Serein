//! Static loading placeholders shaped like the content they stand in for.
//!
//! Every preset allocates the space its real content will occupy, so nothing shifts when data
//! arrives, paints at most what is visible, and exposes one labelled widget instead of a pile of
//! anonymous shapes. All colour comes from [`fill`], so the look can change in one place.

use egui::{Color32, Painter, Pos2, Rect, Response, Sense, Ui, Vec2, pos2, vec2};

/// Repeating text-width factors that keep placeholder lines from looking like a grid.
const WIDTHS: [f32; 4] = [0.85, 0.65, 0.95, 0.55];
/// Upper bound on painted items, whatever the requested count or available height.
pub(crate) const MAX_ITEMS: usize = 128;

/// The single placeholder colour. Translucent so it reads on every surface and theme preset.
pub(crate) fn fill(ui: &Ui) -> Color32 {
	crate::design::palette(ui).muted.gamma_multiply(0.22)
}

pub(crate) fn width_factor(index: usize) -> f32 {
	WIDTHS[index % WIDTHS.len()]
}

/// A text-line placeholder: a bar with fully rounded ends.
pub(crate) fn bar(painter: &Painter, fill: Color32, min: Pos2, size: Vec2) {
	if size.x > 0.5 && size.y > 0.5 {
		let radius = (size.y * 0.5).min(4.0);
		painter.rect_filled(Rect::from_min_size(min, size), radius, fill);
	}
}

pub(crate) fn circle(painter: &Painter, fill: Color32, center: Pos2, radius: f32) {
	painter.circle_filled(center, radius, fill);
}

pub(crate) fn block(
	painter: &Painter,
	fill: Color32,
	rect: Rect,
	radius: impl Into<egui::CornerRadius>,
) {
	if rect.width() > 0.5 && rect.height() > 0.5 {
		painter.rect_filled(rect, radius, fill);
	}
}

/// The tint of [`outline`] frames, for shapes that must match a frame's colour exactly.
pub(crate) fn frame_tint(fill: Color32) -> Color32 {
	fill.gamma_multiply(0.5)
}

/// The faint frame of a card, panel or group that holds other placeholder shapes, rounded by
/// `radius` like the frame it stands in for.
pub(crate) fn outline(
	painter: &Painter,
	fill: Color32,
	rect: Rect,
	radius: impl Into<egui::CornerRadius>,
) {
	block(painter, frame_tint(fill), rect, radius);
}

/// Corner radius of the shared inputs, selects and buttons in [`crate::design`].
pub(crate) const CONTROL_RADIUS: f32 = 8.0;

/// An input, select or button: lighter than text bars so a form reads as fields, not lines.
/// Shared form controls are rounded by [`CONTROL_RADIUS`].
pub(crate) fn control(painter: &Painter, fill: Color32, rect: Rect, radius: f32) {
	block(painter, fill.gamma_multiply(0.6), rect, radius);
}

/// A toggle switch centred on `center`.
fn switch(painter: &Painter, fill: Color32, center: Pos2) {
	block(
		painter,
		fill,
		Rect::from_center_size(center, vec2(40.0, 24.0)),
		12.0,
	);
}

/// How many items of `pitch` fill the visible part of the available space, at least one and at
/// most `max`.
pub(crate) fn visible_count(ui: &Ui, pitch: f32, max: usize) -> usize {
	let room = ui.clip_rect().bottom() - ui.cursor().top();
	((room / pitch).ceil().max(1.0) as usize).min(max.min(MAX_ITEMS))
}

/// How many items fit `room` without overflowing it, at least one and at most [`MAX_ITEMS`]:
/// each takes `pitch`, which includes the `gap` that the last one does not need.
pub(crate) fn fitting_count(room: f32, pitch: f32, gap: f32) -> usize {
	(((room + gap) / pitch).floor().max(1.0) as usize).min(MAX_ITEMS)
}

/// Allocate `size`, label it once for assistive technology and return a clipped painter.
/// An empty `label` marks a decorative part of a placeholder that is labelled elsewhere.
fn region(ui: &mut Ui, size: Vec2, label: &str) -> (Rect, Response, Painter) {
	let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
	if !label.is_empty() {
		let label = label.to_owned();
		response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, false, label.clone()));
	}
	let painter = ui.painter().with_clip_rect(ui.clip_rect().intersect(rect));
	(rect, response, painter)
}

/// A bespoke placeholder of `size`: `paint` receives the clipped painter, the allocated rect and
/// the shared fill. Use the primitives above so one-off layouts still match the presets.
pub(crate) fn custom(
	ui: &mut Ui,
	label: &str,
	size: Vec2,
	paint: impl FnOnce(&Painter, Rect, Color32),
) -> Response {
	let fill = fill(ui);
	let (rect, response, painter) = region(ui, size, label);
	paint(&painter, rect, fill);
	response
}

/// Text lines stacked from `origin`: a 12px heading followed by 10px body lines.
fn lines(painter: &Painter, fill: Color32, origin: Pos2, width: f32, count: u8, seed: usize) {
	let mut y = origin.y;
	for line in 0..count {
		let (height, share) = if line == 0 {
			// A lone line stands for a name or title; above body lines it is a short heading.
			(
				12.0,
				if count == 1 { 0.6 } else { 0.45 } * width_factor(seed),
			)
		} else {
			(10.0, width_factor(seed + line as usize))
		};
		bar(
			painter,
			fill,
			pos2(origin.x, y),
			vec2(width * share, height),
		);
		y += height + 6.0;
	}
}

fn lines_height(count: u8) -> f32 {
	match count {
		0 => 0.0,
		count => 12.0 + f32::from(count - 1) * 16.0,
	}
}

/// A list row: optional leading avatar (circle) or icon (rounded square) and text lines.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Row {
	pub height: f32,
	/// Diameter of the leading circle; `0.0` for none.
	pub avatar: f32,
	/// Draw the leading shape as a rounded square (icons, server art) instead of a circle.
	pub square: bool,
	pub lines: u8,
	/// Left padding before the leading shape or first line.
	pub inset: f32,
}

impl Row {
	/// Member list and other people lists: 32px avatar, name and status.
	pub(crate) const PERSON: Self = Self {
		height: 42.0,
		avatar: 32.0,
		square: false,
		lines: 2,
		inset: 8.0,
	};
}

/// `count` rows of `row`, at most [`MAX_ITEMS`].
pub(crate) fn rows(ui: &mut Ui, label: &str, row: Row, count: usize) -> Response {
	rows_spaced(ui, label, row, count, 0.0)
}

/// [`rows`] with `gap` between consecutive rows, for lists that space their rows apart.
pub(crate) fn rows_spaced(ui: &mut Ui, label: &str, row: Row, count: usize, gap: f32) -> Response {
	let pitch = row.height + gap;
	let count = count.min(MAX_ITEMS);
	let width = ui.available_width();
	let height = (count as f32 * pitch - gap).max(0.0);
	let (rect, response, painter) = region(ui, vec2(width, height), label);
	let fill = fill(ui);
	for index in 0..count {
		let top = rect.top() + index as f32 * pitch;
		if top > painter.clip_rect().bottom() {
			break;
		}
		let slot = Rect::from_min_size(pos2(rect.left(), top), vec2(width, row.height));
		paint_row(&painter, fill, slot, row, index);
	}
	response
}

/// One `row` painted into `rect`, for lists that lay out their own slots (virtualized lists).
/// `seed` varies the line widths between neighbouring rows.
pub(crate) fn paint_row(painter: &Painter, fill: Color32, rect: Rect, row: Row, seed: usize) {
	let text_left = row.inset
		+ if row.avatar > 0.0 {
			row.avatar + 12.0
		} else {
			0.0
		};
	let text_width = (rect.width() - text_left - 8.0).clamp(0.0, 360.0);
	let middle = rect.center().y;
	if row.avatar > 0.0 {
		let center = pos2(rect.left() + row.inset + row.avatar * 0.5, middle);
		if row.square {
			block(
				painter,
				fill,
				Rect::from_center_size(center, Vec2::splat(row.avatar)),
				row.avatar * 0.25,
			);
		} else {
			circle(painter, fill, center, row.avatar * 0.5);
		}
	}
	let origin = pos2(
		rect.left() + text_left,
		middle - lines_height(row.lines) * 0.5,
	);
	lines(painter, fill, origin, text_width, row.lines, seed);
}

/// A wrapping grid of `count` rounded tiles, as in GIF, sticker and emoji pickers.
pub(crate) fn tiles(ui: &mut Ui, label: &str, tile: Vec2, gap: f32, count: usize) -> Response {
	let width = ui.available_width();
	// Tiles sized as `(width - gap * (n - 1)) / n` must give back exactly `n` columns, so allow
	// for the float error that division leaves behind.
	let columns = (((width + gap) / (tile.x + gap) + 1e-3).floor() as usize).max(1);
	let count = count.min(MAX_ITEMS);
	let row_count = count.div_ceil(columns);
	let height = (row_count as f32 * (tile.y + gap) - gap).max(0.0);
	let (rect, response, painter) = region(ui, vec2(width, height), label);
	let fill = fill(ui);
	for index in 0..count {
		let (row, column) = (index / columns, index % columns);
		let min = rect.min + vec2(column as f32 * (tile.x + gap), row as f32 * (tile.y + gap));
		block(&painter, fill, Rect::from_min_size(min, tile), 8.0);
	}
	response
}

/// A content card: a faint outline with an optional leading avatar, a title and body lines.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Card {
	pub height: f32,
	/// Leading avatar diameter (search results, pins); `0.0` for none.
	pub avatar: f32,
	pub gap: f32,
	/// Text lines including the heading, for cards with a fixed row count; `0` fits as many
	/// 16px-pitched body lines below the heading as the height holds, at most three.
	pub lines: u8,
}

/// `count` cards stacked vertically, at most [`MAX_ITEMS`].
pub(crate) fn cards(ui: &mut Ui, label: &str, card: Card, count: usize) -> Response {
	let pitch = card.height + card.gap;
	let count = count.min(MAX_ITEMS);
	let width = ui.available_width();
	let height = (count as f32 * pitch - card.gap).max(0.0);
	let (rect, response, painter) = region(ui, vec2(width, height), label);
	let fill = fill(ui);
	for index in 0..count {
		let top = rect.top() + index as f32 * pitch;
		if top > painter.clip_rect().bottom() {
			break;
		}
		let frame = Rect::from_min_size(pos2(rect.left(), top), vec2(width, card.height));
		outline(&painter, fill, frame, 8.0);
		let inner = frame.shrink(12.0);
		let mut left = inner.left();
		if card.avatar > 0.0 {
			circle(
				&painter,
				fill,
				pos2(left + card.avatar * 0.5, inner.top() + card.avatar * 0.5),
				card.avatar * 0.5,
			);
			left += card.avatar + 12.0;
		}
		let text_width = (inner.right() - left).max(0.0);
		let line_count = match card.lines {
			0 => ((inner.height() - 12.0) / 16.0).floor().clamp(0.0, 3.0) as u8 + 1,
			count => count,
		};
		lines(
			&painter,
			fill,
			pos2(left, inner.top()),
			text_width,
			line_count,
			index,
		);
	}
	response
}

/// Loading stand-ins for the shared form controls of settings pages, profile editing, channel
/// settings and messaging permissions. Each takes its size from the text and fonts the loaded
/// control lays out, so whatever follows it stays put when the data arrives. `label` names the
/// placeholder for assistive technology; pass "" for all but the first.
pub(crate) mod form {
	use super as skeleton;
	use crate::design;
	use egui::{FontId, Response, Ui, pos2, vec2};

	/// Height of `text` (a key or literal) laid out in `font` at `width`.
	fn text_height(ui: &Ui, text: &str, font: FontId, width: f32) -> f32 {
		let text = crate::i18n::translate_if_key(text);
		ui.fonts_mut(|fonts| {
			fonts
				.layout(text, font, egui::Color32::PLACEHOLDER, width)
				.size()
				.y
		})
	}

	fn row_height(ui: &Ui, font: &FontId) -> f32 {
		ui.fonts_mut(|fonts| fonts.row_height(font))
	}

	/// A [`design::input`] of `rows` text rows in `font` (the body font when `None`).
	pub(crate) fn input(ui: &mut Ui, label: &str, font: Option<FontId>, rows: usize) -> Response {
		let font = font.unwrap_or_else(|| egui::TextStyle::Body.resolve(ui.style()));
		let line = row_height(ui, &font) + ui.spacing().extra_text_line_spacing;
		// The input pads its rows by 9px above and below.
		let height = rows as f32 * line + 18.0;
		skeleton::custom(
			ui,
			label,
			vec2(ui.available_width(), height),
			|painter, rect, fill| skeleton::control(painter, fill, rect, skeleton::CONTROL_RADIUS),
		)
	}

	/// Buttons or pickers of the given widths in a row, 8px apart.
	pub(crate) fn controls(ui: &mut Ui, label: &str, widths: &[f32], height: f32) -> Response {
		let available = ui.available_width();
		let total = widths.iter().sum::<f32>() + 8.0 * widths.len().saturating_sub(1) as f32;
		skeleton::custom(
			ui,
			label,
			vec2(total.min(available), height),
			|painter, rect, fill| {
				let mut left = rect.left();
				for width in widths {
					let slot = egui::Rect::from_min_size(
						pos2(left, rect.top()),
						vec2(width.min(rect.right() - left).max(0.0), height),
					);
					skeleton::control(painter, fill, slot, skeleton::CONTROL_RADIUS);
					left += width + 8.0;
				}
			},
		)
	}

	/// `count` cells of `cell` size, `columns` to a row, 8px apart both ways.
	pub(crate) fn grid(
		ui: &mut Ui,
		label: &str,
		count: usize,
		columns: usize,
		cell: egui::Vec2,
	) -> Response {
		let rows = count.div_ceil(columns.max(1));
		let height = rows as f32 * (cell.y + 8.0) - 8.0;
		skeleton::custom(
			ui,
			label,
			vec2(ui.available_width(), height),
			|painter, rect, fill| {
				for index in 0..count {
					let (row, column) = (index / columns, index % columns);
					let min = rect.min
						+ vec2(column as f32 * (cell.x + 8.0), row as f32 * (cell.y + 8.0));
					skeleton::control(
						painter,
						fill,
						egui::Rect::from_min_size(min, cell),
						skeleton::CONTROL_RADIUS,
					);
				}
			},
		)
	}

	/// A [`design::switch`] row: title and optional detail beside the pill.
	pub(crate) fn switch(ui: &mut Ui, label: &str, title: &str, detail: Option<&str>) -> Response {
		let text_width = (ui.available_width() - 64.0).max(80.0);
		let title_font = FontId::new(16.0, design::medium_family(ui.ctx()));
		let line = row_height(ui, &title_font);
		let title = text_height(ui, title, title_font, text_width);
		let detail =
			detail.map(|detail| text_height(ui, detail, FontId::proportional(13.0), text_width));
		let height = (title + detail.map_or(0.0, |d| d + 4.0)).max(24.0) + 16.0;
		skeleton::custom(
			ui,
			label,
			vec2(ui.available_width(), height),
			|painter, rect, fill| {
				let width = (text_width * 0.6).min(320.0);
				let top = rect.top() + 8.0;
				skeleton::bar(
					painter,
					fill,
					pos2(rect.left(), top + (line - 12.0) * 0.5),
					vec2(width, 12.0),
				);
				if detail.is_some() {
					skeleton::bar(
						painter,
						fill,
						pos2(rect.left(), top + title + 6.5),
						vec2(width * 1.2, 10.0),
					);
				}
				skeleton::switch(painter, fill, pos2(rect.right() - 20.0, top + title * 0.5));
			},
		)
	}

	/// A [`design::radio_row`]: marker, title and optional detail.
	pub(crate) fn radio(ui: &mut Ui, label: &str, title: &str, detail: Option<&str>) -> Response {
		let text_width = (ui.available_width() - 44.0).max(80.0);
		let title_font = FontId::new(15.0, design::medium_family(ui.ctx()));
		let line = row_height(ui, &title_font);
		let title = text_height(ui, title, title_font, text_width);
		let detail =
			detail.map(|detail| text_height(ui, detail, FontId::proportional(13.0), text_width));
		let height = (title + detail.map_or(0.0, |d| d + 2.0)).max(20.0) + 16.0;
		skeleton::custom(
			ui,
			label,
			vec2(ui.available_width(), height),
			|painter, rect, fill| {
				let top = rect.top() + 8.0;
				let width = (text_width * 0.5).min(280.0);
				skeleton::circle(
					painter,
					fill,
					pos2(rect.left() + 10.0, top + title * 0.5),
					9.0,
				);
				skeleton::bar(
					painter,
					fill,
					pos2(rect.left() + 32.0, top + (line - 12.0) * 0.5),
					vec2(width, 12.0),
				);
				if detail.is_some() {
					skeleton::bar(
						painter,
						fill,
						pos2(rect.left() + 32.0, top + title + 5.0),
						vec2(width * 1.3, 10.0),
					);
				}
			},
		)
	}
}
