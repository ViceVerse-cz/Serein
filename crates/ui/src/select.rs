//! Chat text selection. The block is the selection target, not the glyph.

use egui::{
	Color32, CursorIcon, Event, FullOutput, Id, InteractOptions, LayerId, Order, PointerButton,
	Popup, PopupAnchor, Pos2, RawInput, Rect, Response, Sense,
	epaint::{Galley, TextShape},
	text_selection::LabelSelectionState,
};
use std::sync::Arc;
mod mapped;

/// Inline artwork positioned inside a run's galley (custom and Unicode emoji).
pub struct Artwork {
	pub rect: Rect,
	pub image: Option<egui::Image<'static>>,
	pub fallback: Option<Arc<Galley>>,
}

struct Run {
	band: egui::Id,
	galley_pos: Pos2,
	galley: Arc<Galley>,
	rect: Rect,
	/// One band per wrapped galley row, so a run never covers a neighbour's line.
	lines: Vec<Rect>,
	painted: bool,
	mapping: Option<Arc<crate::rtl::Layout>>,
	artwork: Vec<Artwork>,
	highlights: Vec<Rect>,
}

struct Hole {
	rect: Rect,
	clickable: bool,
}

/// A widget that brings its own galley (fenced code), selected in body order.
struct Embed {
	/// How many runs preceded it, so `finish` replays it between them.
	after: usize,
	response: Response,
	galley_pos: Pos2,
	galley: Arc<Galley>,
}

struct Overlay {
	id: egui::Id,
	rect: Rect,
}

/// One text block's runs, in layout order.
pub struct Surface {
	base: egui::Id,
	runs: Vec<Run>,
	embeds: Vec<Embed>,
	holes: Vec<Hole>,
	overlays: Vec<Overlay>,
	cover: Option<Rect>,
}

impl Surface {
	/// `salt` distinguishes several blocks under one `Ui` id (body, forwarded preview, …).
	pub fn new(ui: &egui::Ui, salt: impl egui::AsIdSalt) -> Self {
		Self {
			base: ui.scope_id().with(salt),
			runs: Vec::new(),
			embeds: Vec::new(),
			holes: Vec::new(),
			overlays: Vec::new(),
			cover: None,
		}
	}

	/// Stretch the tiled bands to `rect` so a drag can start on empty chat chrome.
	pub fn cover(&mut self, rect: Rect) {
		if rect.is_positive() {
			self.cover = Some(self.cover.map_or(rect, |cover| cover.union(rect)));
		}
	}

	pub fn keep(&mut self, response: &Response) {
		if response.rect.is_positive() {
			self.holes.push(Hole {
				rect: response.rect,
				clickable: response.enabled() && response.sense.senses_click(),
			});
		}
	}

	pub fn exclude(&mut self, rect: Rect) {
		if rect.is_positive() {
			self.holes.push(Hole {
				rect,
				clickable: false,
			});
		}
	}

	pub fn through(&mut self, response: &Response) {
		if response.rect.is_positive() {
			self.overlays.push(Overlay {
				id: response.id,
				rect: response.rect,
			});
		}
	}

	/// Record a run, paint it like a label, and claim a later band slot.
	pub fn run(
		&mut self,
		ui: &mut egui::Ui,
		response: &Response,
		galley_pos: Pos2,
		galley: Arc<Galley>,
		artwork: Vec<Artwork>,
	) {
		let band = self.base.with(self.runs.len());
		let painted =
			!artwork.is_empty() && galley.rows.iter().all(|row| row.visuals.mesh.is_empty());
		if painted {
			ui.painter().add(TextShape::new(
				galley_pos,
				galley.clone(),
				Color32::TRANSPARENT,
			));
		}
		for art in &artwork {
			paint_artwork(ui, art);
		}
		self.runs.push(Run {
			band,
			galley_pos,
			lines: line_bands(&galley, galley_pos, response.rect),
			galley,
			rect: response.rect,
			painted,
			mapping: None,
			artwork: Vec::new(),
			highlights: Vec::new(),
		});
	}

	/// A bidi paragraph keeps native painting separate from its logical cursor map.
	pub(crate) fn mapped_run(
		&mut self,
		ui: &egui::Ui,
		response: &Response,
		pos: Pos2,
		layout: Arc<crate::rtl::Layout>,
		artwork: Vec<Artwork>,
		highlights: Vec<Rect>,
	) {
		let galley = ui.painter().layout_no_wrap(
			String::new(),
			egui::FontId::proportional(1.0),
			Color32::TRANSPARENT,
		);
		let lines = layout
			.lines
			.iter()
			.map(|line| Rect::from_min_size(pos + line.position, line.galley.size()))
			.collect();
		self.runs.push(Run {
			band: self.base.with(self.runs.len()),
			galley_pos: pos,
			galley,
			rect: response.rect,
			lines,
			painted: true,
			mapping: Some(layout),
			artwork,
			highlights,
		});
	}

	/// Record a widget that laid out its own galley (fenced code), so that `finish` registers
	/// its selection between the runs around it.
	///
	/// egui pairs the two ends of a selection by the order labels are registered in, and
	/// treats every label registered in between as fully selected. A code block registers
	/// where it is drawn, in the middle of the body, while the surrounding runs only register
	/// in `finish`: selecting into a block that way puts the whole message — and every earlier
	/// run — "between" the two ends. Deferring the block to the same pass keeps both in
	/// reading order.
	pub fn embed(&mut self, response: &Response, galley_pos: Pos2, galley: Arc<Galley>) {
		self.embeds.push(Embed {
			after: self.runs.len(),
			response: response.clone(),
			galley_pos,
			galley,
		});
	}

	#[cfg(test)]
	pub(crate) fn mapped_layouts(&self) -> Vec<(Pos2, Arc<crate::rtl::Layout>)> {
		self.runs
			.iter()
			.filter_map(|run| {
				run.mapping
					.as_ref()
					.map(|layout| (run.galley_pos, layout.clone()))
			})
			.collect()
	}

	/// True when `pos` is on no glyph line, widget or excluded card of this block, so a
	/// row gesture there cannot be a word selection, link or media interaction.
	pub fn blank_at(&self, pos: Pos2) -> bool {
		!self
			.runs
			.iter()
			.flat_map(|run| &run.lines)
			.chain(self.embeds.iter().map(|embed| &embed.response.rect))
			.chain(self.holes.iter().map(|hole| &hole.rect))
			.chain(self.overlays.iter().map(|over| &over.rect))
			.any(|rect| rect.contains(pos))
	}

	/// Tile the block and register selection on the remaining bands.
	pub fn finish(self, ui: &mut egui::Ui) {
		let block = block_rect(ui, &self.runs, self.cover);
		let embeds = self.embeds;
		let mut embedded = 0;
		let mut runs = self.runs;
		if runs.is_empty() && block.is_positive() {
			runs.push(blank_run(ui, self.base, block));
		}
		let covered = self.cover.is_some();
		tile(&mut runs, block, covered);
		let pointer = ui.input(|input| input.pointer.hover_pos());
		let over_click = pointer.is_some_and(|pos| {
			self.holes
				.iter()
				.any(|hole| hole.clickable && hole.rect.contains(pos))
				|| self.overlays.iter().any(|over| over.rect.contains(pos))
		});
		let over_reserved =
			pointer.is_some_and(|pos| self.holes.iter().any(|hole| hole.rect.contains(pos)));
		let menu_open = Popup::is_any_open(ui.ctx());
		let holes: Vec<Rect> = self.holes.iter().map(|hole| hole.rect).collect();
		for (position, run) in runs.into_iter().enumerate() {
			while embeds
				.get(embedded)
				.is_some_and(|embed| embed.after <= position)
			{
				show_embed(ui, &embeds[embedded], menu_open);
				embedded += 1;
			}
			if !run.rect.is_positive() || !ui.is_rect_visible(run.rect) {
				continue;
			}
			if menu_open {
				if let Some(layout) = &run.mapping {
					mapped::observe(
						ui,
						run.band,
						run.rect,
						mapped::Source::Mapped(layout.clone()),
					);

					for rect in &run.highlights {
						ui.painter().rect_filled(
							*rect,
							0.0,
							Color32::from_rgba_unmultiplied(200, 160, 30, 85),
						);
					}
					layout.paint(ui, run.galley_pos);
					for art in &run.artwork {
						paint_artwork(ui, art);
					}
				}
				if !run.galley.job.text.is_empty() {
					mapped::observe(
						ui,
						run.band,
						run.rect,
						mapped::Source::Native(run.galley.clone()),
					);
					let color = if run.painted {
						Color32::TRANSPARENT
					} else {
						ui.visuals().text_color()
					};
					ui.painter()
						.add(TextShape::new(run.galley_pos, run.galley, color));
				}
				continue;
			}
			let mut response: Option<Response> = None;
			let mut index = 0;
			for line in &run.lines {
				for piece in punch(*line, &holes) {
					let id = if index == 0 {
						run.band
					} else {
						run.band.with(index)
					};
					index += 1;
					let piece = ui.interact(piece, id, band_sense());
					response = Some(match response.take() {
						Some(prev) => prev.union(piece),
						None => piece,
					});
				}
			}
			let Some(response) = response else {
				continue;
			};
			if let Some(layout) = &run.mapping {
				for rect in &run.highlights {
					ui.painter().rect_filled(
						*rect,
						0.0,
						Color32::from_rgba_unmultiplied(200, 160, 30, 85),
					);
				}
				let selected = ui
					.ctx()
					.plugin_or_default::<mapped::Selection>()
					.lock()
					.run(
						ui,
						&response,
						mapped::Source::Mapped(layout.clone()),
						true,
						|point| layout.cursor((point - run.galley_pos).to_pos2()),
					);
				if !selected.is_empty() {
					for cell in &layout.cells {
						if cell.source.start < selected.end && cell.source.end > selected.start {
							ui.painter().rect_filled(
								cell.rect.translate(run.galley_pos.to_vec2()),
								0.0,
								ui.visuals().selection.bg_fill,
							);
						}
					}
				}
				layout.paint(ui, run.galley_pos);
				for art in &run.artwork {
					paint_artwork(ui, art);
				}
				continue;
			}
			if run.galley.job.text.is_empty() {
				continue;
			}
			let color = if run.painted {
				Color32::TRANSPARENT
			} else {
				ui.visuals().text_color()
			};
			mapped::native(ui, &response, run.galley_pos, run.galley, color);
		}
		for embed in &embeds[embedded..] {
			show_embed(ui, embed, menu_open);
		}
		for over in self.overlays {
			ui.interact_opt(
				over.rect,
				over.id,
				Sense::click(),
				InteractOptions { move_to_top: true },
			);
		}
		if over_click {
			ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
		} else if pointer.is_some_and(|pos| block.contains(pos)) && !over_reserved {
			ui.ctx().set_cursor_icon(CursorIcon::Default);
		}
	}
}

#[derive(Default)]
struct Pointer {
	menu: bool,
	silent: bool,
	cached: String,
	selection_secondary: bool,
	#[cfg(target_os = "macos")]
	control_primary: bool,
}

/// Remember the button chosen on press: Control may be released before the mouse button.
#[cfg(any(target_os = "macos", test))]
fn normalize_control_click(input: &mut RawInput, control_primary: &mut bool) {
	for event in &mut input.events {
		if let Event::PointerButton {
			button,
			pressed,
			modifiers,
			..
		} = event && *button == PointerButton::Primary
		{
			if *pressed {
				*control_primary = modifiers.ctrl;
			}
			if *control_primary {
				*button = PointerButton::Secondary;
			}
			if !*pressed {
				*control_primary = false;
			}
		}
	}
	// Process a delivered release before abandoning an interrupted gesture.
	if !input.focused {
		*control_primary = false;
	}
}

impl egui::Plugin for Pointer {
	fn debug_name(&self) -> &'static str {
		"Chat selection pointer"
	}

	fn input_hook(&mut self, ctx: &egui::Context, input: &mut RawInput) {
		#[cfg(target_os = "macos")]
		normalize_control_click(input, &mut self.control_primary);
		let selecting = has_selection(ctx);
		let secondary = input.events.iter().any(|event| {
			matches!(
				event,
				Event::PointerButton {
					button: PointerButton::Secondary,
					pressed: true,
					..
				}
			)
		});
		let secondary_release = input.events.iter().any(|event| {
			matches!(
				event,
				Event::PointerButton {
					button: PointerButton::Secondary,
					pressed: false,
					..
				}
			)
		});
		if self.selection_secondary || (selecting && secondary) {
			self.selection_secondary |= secondary;
			if secondary {
				self.cached = String::new();
			}
			input.events.retain(|event| {
				!matches!(
					event,
					Event::PointerButton {
						button: PointerButton::Secondary,
						..
					}
				)
			});
			if secondary
				&& !input
					.events
					.iter()
					.any(|event| matches!(event, Event::Copy))
			{
				input.events.push(Event::Copy);
				self.silent = true;
			}
			self.menu = secondary;
		} else {
			self.menu = false;
		}
		if secondary_release || !input.focused {
			self.selection_secondary = false;
		}
	}

	fn output_hook(&mut self, ctx: &egui::Context, output: &mut FullOutput) {
		if let Some(text) = output
			.platform_output
			.commands
			.iter()
			.find_map(|command| match command {
				egui::OutputCommand::CopyText(text) => Some(text),
				_ => None,
			}) {
			self.cached = if text.len() <= mapped::COPY_BYTES {
				text.clone()
			} else {
				String::new()
			};
			if self.cached.capacity() > mapped::COPY_BYTES {
				self.cached = String::new();
			}
		}
		if self.silent {
			output
				.platform_output
				.commands
				.retain(|command| !matches!(command, egui::OutputCommand::CopyText(_)));
		}
		self.silent = false;
		if output.platform_output.cursor_icon == CursorIcon::Text && !hovering_edit(ctx) {
			output.platform_output.cursor_icon = CursorIcon::Default;
		}
	}
}

/// Draw outside a Plugin hook: egui notifies plugins when a popup widget is under
/// the pointer, so creating it while holding Pointer's plugin mutex would reenter it.
pub(crate) fn show_menu(ctx: &egui::Context) {
	let requested = ctx
		.plugin_opt::<Pointer>()
		.is_some_and(|plugin| plugin.lock().menu);
	let id = Id::unique("chat-selection-copy");
	if !Popup::is_id_open(ctx, id) && (!requested || Popup::is_any_open(ctx)) {
		return;
	}
	Popup::new(
		id,
		ctx.clone(),
		PopupAnchor::PointerFixed,
		LayerId::new(Order::Foreground, id),
	)
	.open_memory(requested.then_some(egui::SetOpenCommand::Bool(true)))
	.kind(egui::PopupKind::Menu)
	.show(|ui| {
		if ui
			.button(crate::i18n::translate("select-on-end-pass-copy"))
			.clicked()
		{
			request_copy(ui.ctx());
			ui.close();
		}
	});
}

fn hovering_edit(ctx: &egui::Context) -> bool {
	let hovered = ctx.interaction_snapshot(|snapshot| snapshot.hovered.clone());
	hovered
		.iter()
		.any(|id| egui::text_edit::TextEditState::load(ctx, *id).is_some())
}

pub fn install(ctx: &egui::Context) {
	ctx.add_plugin(Pointer::default());
}

pub(crate) fn clear(ctx: &egui::Context) {
	Popup::close_id(ctx, Id::unique("chat-selection-copy"));
	if let Some(plugin) = ctx.plugin_opt::<mapped::Selection>() {
		*plugin.lock() = Default::default();
	}
	if let Some(plugin) = ctx.plugin_opt::<LabelSelectionState>() {
		plugin.lock().clear_selection();
	}
	if let Some(plugin) = ctx.plugin_opt::<Pointer>() {
		*plugin.lock() = Default::default();
	}
}

/// True when a label range is active.
pub fn has_selection(ctx: &egui::Context) -> bool {
	ctx.plugin::<LabelSelectionState>().lock().has_selection()
		|| ctx.plugin_or_default::<mapped::Selection>().lock().active()
}

pub fn open_menu(ctx: &egui::Context) -> bool {
	ctx.plugin_opt::<Pointer>()
		.is_some_and(|plugin| plugin.lock().menu)
}

/// Copy the text cached from the last selected range.
pub fn request_copy(ctx: &egui::Context) {
	if mapped::request_copy(ctx) {
		return;
	}
	let text = ctx
		.plugin_opt::<Pointer>()
		.map(|plugin| plugin.lock().cached.clone())
		.unwrap_or_default();
	if !text.is_empty() {
		ctx.copy_text(text);
	}
}

pub(crate) fn band_sense() -> Sense {
	Sense::CLICK | Sense::DRAG
}

/// Paint a deferred widget galley, registering its selection unless a menu owns the pointer.
fn show_embed(ui: &mut egui::Ui, embed: &Embed, menu_open: bool) {
	if !embed.response.rect.is_positive() || !ui.is_rect_visible(embed.response.rect) {
		return;
	}
	// The galley carries its own per-token colours; the fallback only covers unstyled glyphs.
	let color = ui.visuals().text_color();
	if menu_open {
		mapped::observe(
			ui,
			embed.response.id,
			embed.response.rect,
			mapped::Source::Native(embed.galley.clone()),
		);
		ui.painter().add(TextShape::new(
			embed.galley_pos,
			embed.galley.clone(),
			color,
		));
		return;
	}
	mapped::native(
		ui,
		&embed.response,
		embed.galley_pos,
		embed.galley.clone(),
		color,
	);
}

fn punch(rect: Rect, holes: &[Rect]) -> Vec<Rect> {
	let mut parts = vec![rect];
	for hole in holes {
		if !hole.is_positive() {
			continue;
		}
		let mut next = Vec::new();
		for part in parts {
			next.extend(subtract(part, *hole));
		}
		parts = next;
		if parts.is_empty() {
			break;
		}
	}
	parts
		.into_iter()
		.filter(|part| part.is_positive() && part.width() >= 1.0 && part.height() >= 1.0)
		.collect()
}

fn subtract(rect: Rect, hole: Rect) -> Vec<Rect> {
	let cut = rect.intersect(hole);
	if !cut.is_positive() {
		return vec![rect];
	}
	let mut parts = Vec::new();
	if rect.top() < cut.top() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), rect.top()),
			egui::pos2(rect.right(), cut.top()),
		));
	}
	if cut.bottom() < rect.bottom() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), cut.bottom()),
			egui::pos2(rect.right(), rect.bottom()),
		));
	}
	if rect.left() < cut.left() {
		parts.push(Rect::from_min_max(
			egui::pos2(rect.left(), cut.top()),
			egui::pos2(cut.left(), cut.bottom()),
		));
	}
	if cut.right() < rect.right() {
		parts.push(Rect::from_min_max(
			egui::pos2(cut.right(), cut.top()),
			egui::pos2(rect.right(), cut.bottom()),
		));
	}
	parts
}

fn block_rect(ui: &egui::Ui, runs: &[Run], cover: Option<Rect>) -> Rect {
	let from_runs = runs.first().map(|first| {
		let pad = ui.spacing().item_spacing.y / 2.0;
		let top = runs
			.iter()
			.map(|run| run.rect.top())
			.fold(first.rect.top(), f32::min);
		let bottom = runs
			.iter()
			.map(|run| run.rect.bottom())
			.fold(first.rect.bottom(), f32::max);
		Rect::from_min_max(
			egui::pos2(ui.max_rect().left(), top - pad),
			egui::pos2(ui.max_rect().right(), bottom + pad),
		)
	});
	match (from_runs, cover) {
		(Some(runs), Some(cover)) => runs.union(cover),
		(Some(runs), None) => runs,
		(None, Some(cover)) => cover,
		(None, None) => Rect::NOTHING,
	}
}

/// Screen-space band per galley row. A wrapped run's bounding rect spans several
/// lines, and its last line shares a line with whatever follows it: one band for the
/// whole run would sit on top of those neighbours and steal their selection hits.
fn line_bands(galley: &Galley, galley_pos: Pos2, rect: Rect) -> Vec<Rect> {
	if galley.rows.len() < 2 {
		return vec![rect];
	}
	let last = galley.rows.len() - 1;
	galley
		.rows
		.iter()
		.enumerate()
		.map(|(index, row)| {
			let row = row.rect().translate(galley_pos.to_vec2());
			let top = if index == 0 { rect.top() } else { row.top() };
			let bottom = if index == last {
				rect.bottom()
			} else {
				row.bottom()
			};
			Rect::from_min_max(egui::pos2(row.left(), top), egui::pos2(row.right(), bottom))
		})
		.collect()
}

fn tile(runs: &mut [Run], block: Rect, stitch: bool) {
	let mut index: Vec<(usize, usize)> = Vec::new();
	let mut lines: Vec<Rect> = Vec::new();
	for (run_index, run) in runs.iter().enumerate() {
		for (line_index, line) in run.lines.iter().enumerate() {
			index.push((run_index, line_index));
			lines.push(*line);
		}
	}
	if lines.is_empty() {
		return;
	}
	let mut rows = Vec::new();
	let mut start = 0;
	let mut top = lines[0].top();
	let mut bottom = lines[0].bottom();
	for (line_index, line) in lines.iter().enumerate().skip(1) {
		let center = line.center().y;
		if (top..=bottom).contains(&center) {
			top = top.min(line.top());
			bottom = bottom.max(line.bottom());
		} else {
			rows.push(start..line_index);
			start = line_index;
			top = line.top();
			bottom = line.bottom();
		}
	}
	rows.push(start..lines.len());

	let last = rows.len() - 1;
	let mut previous_bottom = block.top();
	for (row_index, range) in rows.into_iter().enumerate() {
		let natural_top = lines[range.clone()]
			.iter()
			.map(|line| line.top())
			.fold(f32::INFINITY, f32::min);
		let natural_bottom = lines[range.clone()]
			.iter()
			.map(|line| line.bottom())
			.fold(f32::NEG_INFINITY, f32::max);
		let row_top = if row_index == 0 {
			block.top()
		} else if stitch || natural_top <= previous_bottom + 2.0 {
			previous_bottom
		} else {
			natural_top
		};
		let row_bottom = if row_index == last {
			block.bottom()
		} else {
			natural_bottom
		};
		let first = range.start;
		let end = range.end;
		for line in &mut lines[range] {
			line.min.y = row_top;
			line.max.y = row_bottom;
		}
		lines[first].min.x = block.left();
		lines[end - 1].max.x = block.right();
		previous_bottom = row_bottom;
	}

	for ((run_index, line_index), line) in index.into_iter().zip(lines) {
		runs[run_index].lines[line_index] = line;
	}
	for run in runs {
		if let Some(rect) = run.lines.iter().copied().reduce(Rect::union) {
			run.rect = rect;
		}
	}
}

fn blank_run(ui: &egui::Ui, base: egui::Id, block: Rect) -> Run {
	let galley = ui.painter().layout_no_wrap(
		String::new(),
		egui::FontId::proportional(1.0),
		Color32::TRANSPARENT,
	);
	Run {
		band: base.with("blank"),
		galley_pos: block.min,
		galley,
		rect: block,
		lines: vec![block],
		painted: true,
		mapping: None,
		artwork: Vec::new(),
		highlights: Vec::new(),
	}
}

fn paint_artwork(ui: &egui::Ui, art: &Artwork) {
	if !ui.is_rect_visible(art.rect) {
		return;
	}
	let size = art.rect.width().min(art.rect.height());
	if let Some(image) = &art.image {
		let painted = image.calc_size(egui::Vec2::splat(size), image.size());
		image.paint_at(ui, Rect::from_center_size(art.rect.center(), painted));
	} else if let Some(label) = &art.fallback {
		ui.painter().galley(
			Pos2::new(art.rect.left(), art.rect.center().y - label.size().y / 2.0),
			label.clone(),
			ui.visuals().text_color(),
		);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn control_click_keeps_secondary_release_after_control_is_released() {
		let mut held = false;
		let button = |pressed, ctrl| Event::PointerButton {
			pos: egui::pos2(10.0, 10.0),
			button: PointerButton::Primary,
			pressed,
			modifiers: egui::Modifiers {
				ctrl,
				..Default::default()
			},
		};
		for (pressed, ctrl, expected) in [
			(true, true, PointerButton::Secondary),
			(false, false, PointerButton::Secondary),
			(true, false, PointerButton::Primary),
			(false, true, PointerButton::Primary),
		] {
			let mut input = RawInput {
				focused: true,
				events: vec![button(pressed, ctrl)],
				..Default::default()
			};
			normalize_control_click(&mut input, &mut held);
			assert!(
				matches!(input.events[0], Event::PointerButton { button, .. } if button == expected)
			);
		}
		assert!(!held);
	}

	#[test]
	fn control_click_release_matches_press_in_an_unfocused_frame() {
		let event = |pressed| Event::PointerButton {
			pos: egui::pos2(10.0, 10.0),
			button: PointerButton::Primary,
			pressed,
			modifiers: egui::Modifiers {
				ctrl: pressed,
				..Default::default()
			},
		};
		let mut held = false;
		let mut press = RawInput {
			focused: true,
			events: vec![event(true)],
			..Default::default()
		};
		normalize_control_click(&mut press, &mut held);
		assert!(held);
		let mut release = RawInput {
			focused: false,
			events: vec![event(false)],
			..Default::default()
		};
		normalize_control_click(&mut release, &mut held);
		assert!(matches!(
			release.events[0],
			Event::PointerButton {
				button: PointerButton::Secondary,
				pressed: false,
				..
			}
		));
		assert!(!held);
		let mut focus_loss = RawInput {
			focused: false,
			..Default::default()
		};
		held = true;
		normalize_control_click(&mut focus_loss, &mut held);
		assert!(!held);
	}

	const WIDTH: f32 = 220.0;

	/// A body like `text …link` where the leading run wraps across several rows.
	fn show(ui: &mut egui::Ui) {
		let mut surface = Surface::new(ui, "body");
		ui.allocate_ui_with_layout(
			egui::vec2(ui.available_width(), 0.0),
			egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
			|ui| {
				ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
				ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
				for (text, link) in [
					("alpha bravo charlie delta echo foxtrot golf hotel ", false),
					("https://example.com", true),
				] {
					let label = egui::Label::new(text).wrap().selectable(false);
					let (pos, galley, response) = label.layout_in_ui(ui);
					surface.run(ui, &response, pos, galley, Vec::new());
					if link {
						let overlay = ui.interact(
							response.rect,
							response.id.with("link"),
							egui::Sense::click(),
						);
						surface.through(&overlay);
					}
				}
			},
		);
		surface.finish(ui);
	}

	fn input(events: Vec<Event>) -> RawInput {
		RawInput {
			screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(WIDTH, 400.0))),
			events,
			..Default::default()
		}
	}

	fn press(pos: Pos2, pressed: bool) -> Vec<Event> {
		vec![
			Event::PointerMoved(pos),
			Event::PointerButton {
				pos,
				button: PointerButton::Primary,
				pressed,
				modifiers: Default::default(),
			},
		]
	}

	#[cfg(target_os = "macos")]
	fn menu_copy(
		ctx: &egui::Context,
		render: &mut dyn FnMut(&mut egui::Ui),
		from: Pos2,
		prepare: impl FnOnce(),
	) -> Option<String> {
		fn label(shape: &egui::Shape) -> Option<Pos2> {
			match shape {
				egui::Shape::Text(text)
					if text.galley.job.text
						== crate::i18n::translate("select-on-end-pass-copy") =>
				{
					Some(text.pos + text.galley.size() / 2.0)
				}
				egui::Shape::Vec(shapes) => shapes.iter().find_map(label),
				_ => None,
			}
		}
		for pressed in [true, false] {
			let output = ctx.run_ui(
				input(vec![
					Event::PointerMoved(from),
					Event::PointerButton {
						pos: from,
						button: PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers {
							ctrl: pressed,
							..Default::default()
						},
					},
				]),
				&mut *render,
			);
			if pressed {
				assert!(
					has_selection(ctx),
					"Control-click press cannot clear the selected range"
				);
			}
			assert!(
				!output
					.platform_output
					.commands
					.iter()
					.any(|command| matches!(command, egui::OutputCommand::CopyText(_))),
				"opening a menu does not write the clipboard"
			);
			output.drop_without_applying_deltas();
		}
		assert!(
			Popup::is_id_open(ctx, Id::unique("chat-selection-copy")),
			"the release keeps the selection menu open"
		);
		prepare();
		let mut copy_pos = None;
		for _ in 0..3 {
			let output = ctx.run_ui(input(Vec::new()), &mut *render);
			copy_pos = output
				.shapes
				.iter()
				.find_map(|shape| label(&shape.shape))
				.or(copy_pos);
			output.drop_without_applying_deltas();
		}
		let copy_pos = copy_pos.expect("the native Copy menu is actually painted");
		let mut copied = None;
		for pressed in [true, false] {
			let output = ctx.run_ui(input(press(copy_pos, pressed)), &mut *render);
			copied = copied.or_else(|| {
				output
					.platform_output
					.commands
					.iter()
					.find_map(|command| match command {
						egui::OutputCommand::CopyText(value) => Some(value.clone()),
						_ => None,
					})
			});
			output.drop_without_applying_deltas();
		}
		Popup::close_all(ctx);
		copied
	}

	#[cfg(target_os = "macos")]
	#[test]
	fn native_selection_control_click_copies_after_releasing_control_first() {
		let ctx = egui::Context::default();
		install(&ctx);
		let positions = std::cell::Cell::new((Pos2::ZERO, Pos2::ZERO));
		let text = "ordinary Latin selection";
		let mut render = |ui: &mut egui::Ui| {
			let mut surface = Surface::new(ui, "native-menu-selection");
			let (pos, galley, response) = egui::Label::new(text).selectable(false).layout_in_ui(ui);
			positions.set((
				pos + egui::vec2(0.1, galley.size().y / 2.0),
				pos + egui::vec2(galley.size().x + 1.0, galley.size().y / 2.0),
			));
			surface.run(ui, &response, pos, galley, Vec::new());
			surface.finish(ui);
			show_menu(ui.ctx());
		};
		ctx.run_ui(input(Vec::new()), &mut render)
			.drop_without_applying_deltas();
		let (from, to) = positions.get();
		for events in [
			press(from, true),
			vec![Event::PointerMoved(to)],
			press(to, false),
			vec![],
		] {
			ctx.run_ui(input(events), &mut render)
				.drop_without_applying_deltas();
		}
		assert_eq!(menu_copy(&ctx, &mut render, from, || {}), Some(text.into()));
		Popup::open_id(&ctx, Id::unique("chat-selection-copy"));
		clear(&ctx);
		assert!(
			!Popup::is_any_open(&ctx),
			"a conversation/account reset closes the old selection menu"
		);
		let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
		assert!(!has_selection(&ctx) && !Popup::is_any_open(&ctx));
		assert!(
			!output
				.platform_output
				.commands
				.iter()
				.any(|command| matches!(command, egui::OutputCommand::CopyText(_))),
			"the reset cannot emit retained native or mapped text"
		);
		output.drop_without_applying_deltas();
		let unrelated = Id::unique("unrelated-popup");
		Popup::open_id(&ctx, unrelated);
		clear(&ctx);
		assert!(
			Popup::is_id_open(&ctx, unrelated),
			"only selection popup memory is reset"
		);
		Popup::close_all(&ctx);
	}

	/// Drag from `from` to `to` and return what a copy would yield.
	fn drag(from: Pos2, to: Pos2) -> String {
		let ctx = egui::Context::default();
		for events in [
			Vec::new(),
			press(from, true),
			vec![Event::PointerMoved(to)],
			press(to, false),
			vec![Event::Copy],
		] {
			let output = ctx.run_ui(input(events), show);
			let copied = output
				.platform_output
				.commands
				.iter()
				.find_map(|command| match command {
					egui::OutputCommand::CopyText(text) => Some(text.clone()),
					_ => None,
				});
			output.drop_without_applying_deltas();
			if let Some(copied) = copied {
				return copied;
			}
		}
		String::new()
	}

	// Layout is "alpha … foxtrot " / "golf hotel " + "https://example.com", so the
	// wrapped first run ends on the same visual row as the link.

	#[test]
	fn a_drag_inside_the_wrapped_row_stops_before_the_link_row() {
		assert_eq!(
			drag(Pos2::new(4.0, 7.0), Pos2::new(WIDTH - 4.0, 7.0)),
			"lpha bravo charlie delta echo foxtrot"
		);
	}

	#[test]
	fn a_drag_on_the_link_row_starts_where_the_pointer_is() {
		assert_eq!(
			drag(Pos2::new(4.0, 22.0), Pos2::new(WIDTH - 4.0, 22.0)),
			"olf hotel https://example.com"
		);
	}

	#[test]
	fn a_drag_across_rows_ends_under_the_pointer() {
		assert_eq!(
			drag(Pos2::new(4.0, 7.0), Pos2::new(40.0, 22.0)),
			"lpha bravo charlie delta echo foxtrot golf ho"
		);
		assert_eq!(
			drag(Pos2::new(120.0, 22.0), Pos2::new(60.0, 7.0)),
			"o charlie delta echo foxtrot golf hotel https://exa"
		);
	}
	#[test]
	fn rtl_pointer_drag_copies_partial_arabic_in_logical_order_and_crosses_to_latin() {
		for across in [false, true] {
			let ctx = egui::Context::default();
			crate::fonts::install(&ctx);
			install(&ctx);
			let text = "مرحبا بالعالم English 123";
			let changed = std::cell::Cell::new(false);
			let clipped = std::cell::Cell::new(0_u8);
			let prepended = std::cell::Cell::new(false);
			let source_pressure = std::cell::Cell::new(false);
			let external_copy = std::cell::Cell::new(false);
			let start_pos = std::cell::Cell::new(None);
			let end_pos = std::cell::Cell::new(None);
			let mut render = |ui: &mut egui::Ui| {
				if prepended.get() {
					let mut prefix = Surface::new(ui, "prepended-row");
					let (pos, galley, response) = egui::Label::new("unselected prefix")
						.selectable(false)
						.layout_in_ui(ui);
					prefix.run(ui, &response, pos, galley, Vec::new());
					prefix.finish(ui);
				}
				let mut tail_top = None;
				let mut surface = Surface::new(ui, "rtl-pointer-test");
				let spans = [crate::rtl::Span {
					text: if changed.get() {
						"مرحبا changed"
					} else {
						text
					}
					.into(),
					format: egui::TextFormat::simple(
						egui::FontId::proportional(15.0),
						Color32::WHITE,
					),
					action: 0,
					object: None,
					copy: true,
				}];
				let layout = crate::rtl::layout(ui.ctx(), &spans, WIDTH).unwrap();
				let (rect, response) = ui.allocate_exact_size(layout.size, Sense::hover());
				let first = layout
					.cells
					.iter()
					.find(|cell| cell.source.start == 0)
					.unwrap();
				start_pos.set(Some(
					rect.min + egui::vec2(first.rect.right() - 0.1, first.rect.center().y),
				));
				let last = layout
					.cells
					.iter()
					.find(|cell| cell.source.end == "مرحبا".len())
					.unwrap();
				end_pos.set(Some(
					rect.min + egui::vec2(last.rect.left() + 0.1, last.rect.center().y),
				));
				surface.mapped_run(ui, &response, rect.min, layout, Vec::new(), Vec::new());
				if across {
					let (pos, mut galley, response) =
						egui::Label::new("tail").selectable(false).layout_in_ui(ui);
					if source_pressure.get() {
						Arc::make_mut(&mut Arc::make_mut(&mut galley).job)
							.text
							.reserve_exact(4 * 1024 * 1024);
					}
					end_pos.set(Some(
						pos + egui::vec2(galley.size().x + 1.0, galley.size().y / 2.0),
					));
					tail_top = Some(pos.y + 0.5);
					surface.run(ui, &response, pos, galley, Vec::new());
				}
				if clipped.get() == 1 {
					ui.set_clip_rect(Rect::NOTHING);
				} else if clipped.get() == 2 {
					let mut clip = ui.clip_rect();
					clip.min.y = tail_top.unwrap();
					ui.set_clip_rect(clip);
				}
				surface.finish(ui);
				if external_copy.replace(false) {
					request_copy(ui.ctx());
				}
				show_menu(ui.ctx());
			};
			ctx.run_ui(input(Vec::new()), &mut render)
				.drop_without_applying_deltas();
			// Keep the actual native hit positions, rather than asserting a synthetic index map.
			let from = start_pos.get().unwrap();
			let to = end_pos.get().unwrap();
			let mut copied = None;
			for events in [
				press(from, true),
				vec![Event::PointerMoved(to)],
				press(to, false),
				vec![],
				vec![Event::Copy],
			] {
				let output = ctx.run_ui(input(events), &mut render);
				copied = copied.or_else(|| {
					output
						.platform_output
						.commands
						.iter()
						.find_map(|command| match command {
							egui::OutputCommand::CopyText(text) => Some(text.clone()),
							_ => None,
						})
				});
				output.drop_without_applying_deltas();
			}
			assert_eq!(
				copied,
				Some(if across {
					format!("{text}\ntail")
				} else {
					"مرحبا".into()
				})
			);
			#[cfg(target_os = "macos")]
			assert_eq!(menu_copy(&ctx, &mut render, from, || {}), copied);

			// The app's ordinary message menu owns a different popup ID and uses
			// this same explicit Copy action after body rendering.
			Popup::open_id(&ctx, Id::unique("message-context-copy"));
			assert!(
				has_selection(&ctx),
				"the valid mapped range remains active before message-menu Copy"
			);
			external_copy.set(true);
			let output = ctx.run_ui(input(Vec::new()), &mut render);
			assert!(output.platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(value) if Some(value) == copied.as_ref())), "other message menus also copy current logical source");
			output.drop_without_applying_deltas();
			Popup::close_all(&ctx);
			clipped.set(1);
			let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
			assert!(
				!output
					.platform_output
					.commands
					.iter()
					.any(|command| matches!(command, egui::OutputCommand::CopyText(_))),
				"an incomplete offscreen selection never copies a partial or retained buffer"
			);
			output.drop_without_applying_deltas();
			clipped.set(0);
			ctx.run_ui(input(Vec::new()), &mut render)
				.drop_without_applying_deltas();
			let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
			assert!(output.platform_output.commands.iter().any(|command|
				matches!(command, egui::OutputCommand::CopyText(value) if value == copied.as_ref().unwrap())),
				"scrolling back restores the original logical selection after source revalidation");
			output.drop_without_applying_deltas();
			if across {
				// Only A leaves the viewport: B now has order0 while the retained A
				// endpoint still has order0. Restoring both must resolve both IDs first.
				clipped.set(2);
				ctx.run_ui(input(Vec::new()), &mut render)
					.drop_without_applying_deltas();
				let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
				assert!(
					!output
						.platform_output
						.commands
						.iter()
						.any(|command| matches!(command, egui::OutputCommand::CopyText(_)))
				);
				output.drop_without_applying_deltas();
				clipped.set(0);
				// Copy on the first restored pass, before a settling/repaint pass.
				let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
				assert!(output.platform_output.commands.iter().any(|command|
					matches!(command, egui::OutputCommand::CopyText(value) if value == &format!("{text}\ntail"))),
					"a partial viewport cannot truncate A using B's old visible ordinal");
				output.drop_without_applying_deltas();
				prepended.set(true);
				let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
				assert!(output.platform_output.commands.iter().any(|command|
					matches!(command, egui::OutputCommand::CopyText(value) if value == &format!("{text}\ntail"))),
					"prepending a visible run cannot truncate either endpoint or copy the prefix");
				output.drop_without_applying_deltas();
				prepended.set(false);
				ctx.run_ui(input(Vec::new()), &mut render)
					.drop_without_applying_deltas();
				source_pressure.set(true);
				let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
				assert!(
					!output
						.platform_output
						.commands
						.iter()
						.any(|command| matches!(command, egui::OutputCommand::CopyText(_))),
					"large spare source capacity rejects clipboard assembly even when visible text is short"
				);
				output.drop_without_applying_deltas();
				source_pressure.set(false);
				let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
				assert!(output.platform_output.commands.iter().any(|command|
					matches!(command, egui::OutputCommand::CopyText(value) if value == &format!("{text}\ntail"))),
					"a new bounded pass can copy; the prior rejected request is not retried");
				output.drop_without_applying_deltas();
			}
			// The menu click is a separate gesture from the word double-click below.
			let mut settled = input(Vec::new());
			settled.time = Some(10.0);
			ctx.run_ui(settled, &mut render)
				.drop_without_applying_deltas();
			for events in [
				press(from, true),
				press(from, false),
				press(from, true),
				press(from, false),
				Vec::new(),
			] {
				ctx.run_ui(input(events), &mut render)
					.drop_without_applying_deltas();
			}
			let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
			assert!(
				output.platform_output.commands.iter().any(
					|command| matches!(command, egui::OutputCommand::CopyText(value) if value == "مرحبا")
				),
				"double clicking selects the logical Arabic word"
			);
			output.drop_without_applying_deltas();
			ctx.run_ui(
				input(vec![Event::Key {
					key: egui::Key::A,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers {
						command: true,
						..Default::default()
					},
				}]),
				&mut render,
			)
			.drop_without_applying_deltas();
			let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
			assert!(
				output.platform_output.commands.iter().any(
					|command| matches!(command, egui::OutputCommand::CopyText(value) if value == text)
				),
				"select all retains the logical source rather than visual line order"
			);
			output.drop_without_applying_deltas();
			if across {
				ctx.run_ui(
					input(vec![Event::Key {
						key: egui::Key::Escape,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: Default::default(),
					}]),
					&mut render,
				)
				.drop_without_applying_deltas();
			} else {
				changed.set(true);
				ctx.run_ui(input(Vec::new()), &mut render)
					.drop_without_applying_deltas();
			}
			let output = ctx.run_ui(input(vec![Event::Copy]), &mut render);
			assert!(
				!output
					.platform_output
					.commands
					.iter()
					.any(|command| matches!(command, egui::OutputCommand::CopyText(_))),
				"Escape and edited source retire mapped selection rather than copy old text"
			);
			output.drop_without_applying_deltas();
			#[cfg(target_os = "macos")]
			{
				for invalidation in 0..3 {
					changed.set(false);
					clipped.set(0);
					source_pressure.set(false);
					let mut restored = input(Vec::new());
					restored.time = Some(20.0 + f64::from(invalidation) * 2.0);
					ctx.run_ui(restored, &mut render)
						.drop_without_applying_deltas();
					for events in [
						press(from, true),
						vec![Event::PointerMoved(to)],
						press(to, false),
						vec![],
						vec![Event::Copy],
					] {
						ctx.run_ui(input(events), &mut render)
							.drop_without_applying_deltas();
					}
					assert!(
						has_selection(&ctx),
						"invalidation {invalidation} begins with an actual selected range"
					);
					assert_eq!(
						menu_copy(&ctx, &mut render, from, || match invalidation {
							0 => clipped.set(1),
							1 => changed.set(true),
							_ if across => source_pressure.set(true),
							_ => clipped.set(1),
						}),
						None,
						"menu Copy cannot reuse earlier text after clipping, editing, or retained-source overflow"
					);
				}
			}
		}
	}
}
