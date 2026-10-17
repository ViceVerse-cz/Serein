//! Chat text selection. The block is the selection target, not the glyph.

use egui::{
	Color32, CursorIcon, Event, FullOutput, Id, InteractOptions, LayerId, Order, PointerButton,
	Popup, PopupAnchor, Pos2, RawInput, Rect, Response, Sense, Stroke,
	epaint::{Galley, TextShape},
	text_selection::LabelSelectionState,
};
use std::sync::Arc;

/// Inline artwork positioned inside a run's galley (custom and Unicode emoji).
pub struct Artwork {
	pub rect: Rect,
	pub image: Option<egui::Image<'static>>,
}

struct Run {
	band: egui::Id,
	galley_pos: Pos2,
	galley: Arc<Galley>,
	rect: Rect,
	/// One band per wrapped galley row, so a run never covers a neighbour's line.
	lines: Vec<Rect>,
	painted: bool,
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
	sense: Sense,
}

/// One text block's runs, in layout order.
pub struct Surface {
	base: egui::Id,
	/// Next band id. It does not restart when a flush registers the body early.
	band_count: usize,
	runs: Vec<Run>,
	embeds: Vec<Embed>,
	holes: Vec<Hole>,
	overlays: Vec<Overlay>,
	cover: Option<Rect>,
	/// Line rects already registered. `finish` unions them after a flush.
	seen: Vec<Rect>,
	/// Glyph rows. Hit bands are wider than the glyphs, and a double-click in that
	/// spare space is still a blank row.
	glyphs: Vec<Rect>,
	/// Embeds registered by an earlier flush. `blank_at` still has to see them.
	blocked: Vec<Rect>,
}

impl Surface {
	/// `salt` distinguishes several blocks under one `Ui` id (body, forwarded preview, …).
	pub fn new(ui: &egui::Ui, salt: impl egui::AsIdSalt) -> Self {
		Self {
			base: ui.scope_id().with(salt),
			band_count: 0,
			runs: Vec::new(),
			embeds: Vec::new(),
			holes: Vec::new(),
			overlays: Vec::new(),
			cover: None,
			seen: Vec::new(),
			glyphs: Vec::new(),
			blocked: Vec::new(),
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
				sense: response.sense,
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
		let band = self.base.with(self.band_count);
		self.band_count += 1;
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
		self.glyphs.extend(
			galley
				.rows
				.iter()
				.map(|row| row.rect().translate(galley_pos.to_vec2())),
		);
		self.runs.push(Run {
			band,
			galley_pos,
			lines: line_bands(&galley, galley_pos, response.rect),
			galley,
			rect: response.rect,
			painted,
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
		self.blocked.push(response.rect);
		self.embeds.push(Embed {
			after: self.runs.len(),
			response: response.clone(),
			galley_pos,
			galley,
		});
	}

	/// True when `pos` is on no glyph, embed, widget or excluded card of this block, so a
	/// row gesture there cannot be a word selection, link or media interaction.
	pub fn blank_at(&self, pos: Pos2) -> bool {
		!self
			.glyphs
			.iter()
			.chain(self.blocked.iter())
			.chain(self.embeds.iter().map(|embed| &embed.response.rect))
			.chain(self.holes.iter().map(|hole| &hole.rect))
			.chain(self.overlays.iter().map(|over| &over.rect))
			.any(|rect| rect.contains(pos))
	}

	/// Register the runs queued so far, in the order they were recorded.
	///
	/// A card under the message registers its own labels while it is drawn. If the
	/// body waits until `finish`, egui copies the card first and the text above it
	/// lands at the end.
	pub fn flush(&mut self, ui: &mut egui::Ui) {
		if self.runs.is_empty() && self.embeds.is_empty() {
			return;
		}
		let block = block_rect(ui, &self.runs, self.cover);
		let mut runs = std::mem::take(&mut self.runs);
		let embeds = std::mem::take(&mut self.embeds);
		if runs.is_empty() && block.is_positive() {
			runs.push(blank_run(ui, self.base, block));
		}
		let covered = self.cover.is_some();
		tile(&mut runs, block, covered);
		let menu_open = Popup::is_any_open(ui.ctx());
		let holes: Vec<Rect> = self.holes.iter().map(|hole| hole.rect).collect();
		let mut embedded = 0;
		for (position, run) in runs.iter().enumerate() {
			while embeds
				.get(embedded)
				.is_some_and(|embed| embed.after <= position)
			{
				show_embed(ui, &embeds[embedded], menu_open);
				self.seen.push(embeds[embedded].response.rect);
				embedded += 1;
			}
			self.seen.extend(run.lines.iter().copied());
			register_run(ui, run, &holes, menu_open);
		}
		for embed in &embeds[embedded..] {
			show_embed(ui, embed, menu_open);
			self.seen.push(embed.response.rect);
		}
		self.cover = None;
	}

	/// Tile the block and register selection on the remaining bands.
	pub fn finish(mut self, ui: &mut egui::Ui) {
		let block = block_rect(ui, &self.runs, self.cover);
		let pointer = ui.input(|input| input.pointer.hover_pos());
		let over_click = pointer.is_some_and(|pos| {
			self.holes
				.iter()
				.any(|hole| hole.clickable && hole.rect.contains(pos))
				|| self.overlays.iter().any(|over| over.rect.contains(pos))
		});
		let over_reserved =
			pointer.is_some_and(|pos| self.holes.iter().any(|hole| hole.rect.contains(pos)));
		self.flush(ui);
		let block = self
			.seen
			.iter()
			.fold(block, |block, rect| block.union(*rect));
		for over in self.overlays {
			ui.interact_opt(
				over.rect,
				over.id,
				over.sense,
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

fn register_run(ui: &mut egui::Ui, run: &Run, holes: &[Rect], menu_open: bool) {
	if !run.rect.is_positive() || !reaches(ui, run.rect) {
		return;
	}
	// An open menu must not take the click, but the range still has to be
	// registered or egui drops it for lack of both ends.
	let sense = if menu_open {
		Sense::empty()
	} else {
		band_sense()
	};
	let mut response: Option<Response> = None;
	let mut index = 0;
	for line in &run.lines {
		for piece in punch(*line, holes) {
			let id = if index == 0 {
				run.band
			} else {
				run.band.with(index)
			};
			index += 1;
			let piece = ui.interact(piece, id, sense);
			response = Some(match response.take() {
				Some(prev) => prev.union(piece),
				None => piece,
			});
		}
	}
	let Some(mut response) = response else {
		return;
	};
	if menu_open {
		response
			.flags
			.set(egui::response::Flags::CONTAINS_POINTER, false);
	}
	if run.galley.job.text.is_empty() {
		return;
	}
	let color = if run.painted {
		Color32::TRANSPARENT
	} else {
		ui.visuals().text_color()
	};
	register(ui, &response, run.galley_pos, run.galley.clone(), color);
}

#[derive(Default)]
struct Pointer {
	menu: bool,
	silent: bool,
	/// The last copy command from this right-click. Empty when that pass copied nothing,
	/// so an earlier right-click cannot leak onto the next menu action.
	/// An earlier command on the same pass can be a selected draft in the composer.
	cached: String,
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

	fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
		// Chrome above and below the timeline shares the window layer. A drag that
		// started on a message must not continue into it. Mouse-up is still a drag
		// until the end of that pass, so the release frame counts too.
		let block_chrome = drag_outside_viewport(ui);
		ui.style_mut().interaction.multi_widget_text_select = !block_chrome;
	}

	fn input_hook(&mut self, ctx: &egui::Context, input: &mut RawInput) {
		#[cfg(target_os = "macos")]
		normalize_control_click(input, &mut self.control_primary);
		let selecting = ctx.plugin::<LabelSelectionState>().lock().has_selection();
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
		if selecting && secondary {
			input.events.retain(|event| {
				!matches!(
					event,
					Event::PointerButton {
						button: PointerButton::Secondary,
						..
					}
				)
			});
			if !input
				.events
				.iter()
				.any(|event| matches!(event, Event::Copy))
			{
				input.events.push(Event::Copy);
				self.silent = true;
			}
			self.menu = true;
		} else if ctx.output(|output| output.num_completed_passes) == 0 {
			// A discarded replay has empty input. Clearing here would drop the menu
			// before the pass that is actually painted.
			self.menu = false;
		}
	}

	fn on_end_pass(&mut self, ui: &mut egui::Ui) {
		if !self.menu || Popup::is_any_open(ui.ctx()) {
			return;
		}
		let id = Id::unique("chat-selection-copy");
		Popup::new(
			id,
			ui.ctx().clone(),
			PopupAnchor::PointerFixed,
			LayerId::new(Order::Foreground, id),
		)
		.open_memory(Some(egui::SetOpenCommand::Bool(true)))
		.kind(egui::PopupKind::Menu)
		.show(|ui| {
			if ui
				.button(crate::i18n::translate("select-on-end-pass-copy"))
				.clicked()
			{
				let text = std::mem::take(&mut self.cached);
				if !text.is_empty() {
					ui.ctx().copy_text(text);
				}
				ui.close();
			}
		});
	}

	fn output_hook(&mut self, ctx: &egui::Context, output: &mut FullOutput) {
		if self.silent {
			self.cached = output
				.platform_output
				.commands
				.iter()
				.rev()
				.find_map(|command| match command {
					egui::OutputCommand::CopyText(text) => Some(text.clone()),
					_ => None,
				})
				.unwrap_or_default();
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

fn hovering_edit(ctx: &egui::Context) -> bool {
	let hovered = ctx.interaction_snapshot(|snapshot| snapshot.hovered.clone());
	hovered
		.iter()
		.any(|id| egui::text_edit::TextEditState::load(ctx, *id).is_some())
}

pub fn install(ctx: &egui::Context) {
	ctx.add_plugin(Pointer::default());
}

/// True when a label range is active.
pub fn has_selection(ctx: &egui::Context) -> bool {
	ctx.plugin::<LabelSelectionState>().lock().has_selection()
}

/// Drop a label range, e.g. the word a row double-click selected beside its text.
pub fn clear(ctx: &egui::Context) {
	ctx.plugin::<LabelSelectionState>().lock().clear_selection();
}

pub fn open_menu(ctx: &egui::Context) -> bool {
	ctx.plugin_opt::<Pointer>()
		.is_some_and(|plugin| plugin.lock().menu)
}

/// Copy the channel highlight captured on the right-click.
///
/// That pass also copies a focused draft, and the draft command comes first.
/// The clipboard keeps the last command, so this stores that one.
pub fn request_copy(ctx: &egui::Context) {
	let text = ctx
		.plugin_opt::<Pointer>()
		.map(|plugin| std::mem::take(&mut plugin.lock().cached))
		.unwrap_or_default();
	if !text.is_empty() {
		ctx.copy_text(text);
	}
}

const OFFSTAGE: &str = "serein-selection-offstage";
pub(crate) const PIN: &str = "serein-selection-pin";

/// A zero-clip child for a selection end that has scrolled off the timeline.
///
/// The clip is `Rect::NOTHING`, so paint and pointer hits miss. The tag stays on
/// every nested `Ui`, and `reaches` treats that tag as permission to register.
pub fn offstage(parent: &mut egui::Ui, max_rect: Rect, salt: &str) -> egui::Ui {
	let mut ui = parent.new_child(
		egui::UiBuilder::new()
			.id_salt((OFFSTAGE, salt))
			.max_rect(max_rect)
			.ui_stack_info(egui::UiStackInfo::default().with_tag(OFFSTAGE)),
	);
	ui.set_clip_rect(Rect::NOTHING);
	ui
}

/// Label text at `rect` registers this pass when it is on screen or inside [`offstage`].
fn reaches(ui: &egui::Ui, rect: Rect) -> bool {
	ui.is_rect_visible(rect)
		|| ui
			.stack()
			.iter()
			.any(|frame| frame.tags().contains(OFFSTAGE) || frame.tags().contains(PIN))
}

/// The scrollbar strip on the trailing edge of `clip`.
pub(crate) fn scroll_bar_strip(ui: &egui::Ui, clip: Rect) -> Rect {
	let scroll = ui.spacing().scroll;
	let width = (scroll.bar_inner_margin + scroll.bar_width + scroll.bar_outer_margin).max(12.0);
	Rect::from_min_max(
		Pos2::new(clip.right() - width, clip.top()),
		clip.right_bottom(),
	)
}

/// A press inside `rect` is a hovered label, so egui does not treat it as a click away from the selection.
///
/// The scrollbar is that press. Selection fields are private, and the bar is the widget under the
/// pointer, so the hover flag on a registration here is the lever that keeps the range.
pub(crate) fn hold(ui: &mut egui::Ui, rect: Rect) {
	if !rect.is_positive() || !has_selection(ui.ctx()) {
		return;
	}
	let over = ui
		.input(|input| input.pointer.interact_pos().or(input.pointer.hover_pos()))
		.is_some_and(|pos| rect.contains(pos));
	if !over {
		return;
	}
	let mut response = ui.interact(rect, ui.scope_id().with("selection-hold"), Sense::hover());
	response.flags.set(egui::response::Flags::HOVERED, true);
	let galley = ui.fonts_mut(|fonts| {
		fonts.layout_no_wrap(
			String::new(),
			egui::FontId::proportional(1.0),
			Color32::TRANSPARENT,
		)
	});
	LabelSelectionState::label_text_selection(
		ui,
		&response,
		rect.min,
		galley,
		Color32::TRANSPARENT,
		Stroke::NONE,
	);
}

/// A press inside an open selection menu is still on the selection.
///
/// egui clears a label range when a press hits nothing hovered. The menu button is not a
/// label, so the press would drop the range and the copy action would leave the menu.
pub(crate) fn keep(ui: &mut egui::Ui) {
	if !has_selection(ui.ctx()) {
		return;
	}
	let rect = Rect::from_min_size(Pos2::new(-10_000.0, -10_000.0), egui::vec2(1.0, 1.0));
	let mut response = ui.interact(rect, ui.scope_id().with("selection-keep"), Sense::hover());
	response.flags.set(egui::response::Flags::HOVERED, true);
	response
		.flags
		.set(egui::response::Flags::CONTAINS_POINTER, false);
	let galley = ui.fonts_mut(|fonts| {
		fonts.layout_no_wrap(
			String::new(),
			egui::FontId::proportional(1.0),
			Color32::TRANSPARENT,
		)
	});
	LabelSelectionState::label_text_selection(
		ui,
		&response,
		rect.min,
		galley,
		Color32::TRANSPARENT,
		Stroke::NONE,
	);
}

/// Channel clip for this pass. The timeline publishes it so a nested row can tell the
/// viewport edge from its own short clip.
fn viewport_id() -> egui::Id {
	egui::Id::unique("serein-selection-viewport")
}

pub(crate) fn publish_viewport(ui: &egui::Ui, clip: Rect) {
	ui.data_mut(|data| data.insert_temp(viewport_id(), clip));
}

/// The pointer is held outside the timeline while a message range is active.
fn drag_outside_viewport(ui: &egui::Ui) -> bool {
	let Some(viewport) = ui.data(|data| data.get_temp::<Rect>(viewport_id())) else {
		return false;
	};
	if !viewport.is_positive() || !has_selection(ui.ctx()) {
		return false;
	}
	let (down, released, pos) = ui.input(|input| {
		(
			input.pointer.primary_down(),
			input.pointer.primary_released(),
			input.pointer.interact_pos(),
		)
	});
	(down || released) && pos.is_none_or(|pos| !viewport.contains(pos))
}

/// The pointer is held still, so a scroll must not retarget the drag onto whatever glyph
/// slid underneath it. The viewport edge is the exception: holding there extends the range.
///
/// Mouse-up is still inside egui's drag. `is_dragging` clears only at the end of that pass,
/// so a release on top of text that scrolled under the pointer would steal the range.
fn drag_is_still(ui: &egui::Ui) -> bool {
	let (down, pressed, released, delta, pos) = ui.input(|input| {
		(
			input.pointer.primary_down(),
			input.pointer.any_pressed(),
			input.pointer.primary_released(),
			input.pointer.delta(),
			input.pointer.interact_pos(),
		)
	});
	if (!down && !released) || pressed || delta.length_sq() >= 0.25 {
		return false;
	}
	let Some(pos) = pos else {
		return true;
	};
	let Some(viewport) = ui.data(|data| data.get_temp::<Rect>(viewport_id())) else {
		return true;
	};
	if !viewport.is_positive() {
		return true;
	}
	let band = 36.0;
	let at_edge = pos.y < viewport.min.y + band || pos.y > viewport.max.y - band;
	!at_edge
}

/// A still drag must not move onto another label. egui only updates the dragged end when
/// `multi_widget_text_select` is set, or when the label already owns that end.
pub(crate) fn lock_still_drag(ui: &mut egui::Ui) {
	if drag_is_still(ui) {
		ui.style_mut().interaction.multi_widget_text_select = false;
	}
}

/// The one call into egui's label selection. Registers when `reaches` allows it.
///
/// A run that misses the clip still has to register, or egui drops the range. While a drag
/// is held still, `cursor_for` would also retarget that range onto text that scrolled under
/// the pointer. An empty clip skips that retarget. The painted shape is then unclipped so
/// the same characters stay visible.
pub(crate) fn register(
	ui: &mut egui::Ui,
	response: &Response,
	galley_pos: Pos2,
	galley: Arc<Galley>,
	color: Color32,
) {
	if !reaches(ui, response.rect) {
		return;
	}
	let clip = ui.clip_rect();
	let offstage = !response.rect.intersects(clip);
	let freeze = !offstage && drag_is_still(ui);
	let mut response = response.clone();
	if freeze {
		response
			.flags
			.set(egui::response::Flags::CONTAINS_POINTER, false);
	}
	let hide = offstage || freeze;
	if hide {
		ui.set_clip_rect(Rect::NOTHING);
	}
	let layer = ui.layer_id();
	let shape_idx = hide.then(|| {
		ui.ctx()
			.graphics(|gfx| gfx.get(layer).map(|list| list.next_idx()))
	});
	LabelSelectionState::label_text_selection(
		ui,
		&response,
		galley_pos,
		galley.clone(),
		color,
		Stroke::NONE,
	);
	if hide {
		ui.set_clip_rect(clip);
	}
	// The last on-screen label is the one a pointer below the text should drag.
	// Offstage rows register later and must not replace it.
	if !offstage
		&& ui.input(|input| input.pointer.primary_down() && !input.pointer.primary_released())
	{
		ui.data_mut(|data| {
			data.insert_temp(
				drag_claim_id(),
				DragClaim {
					response: response.clone(),
					galley_pos,
					galley,
					color,
				},
			);
		});
	}
	if freeze && let Some(Some(idx)) = shape_idx {
		ui.ctx().graphics_mut(|gfx| {
			if let Some(list) = gfx.get_mut(layer) {
				list.mutate_shape(idx, |shape| shape.clip_rect = clip);
			}
		});
	}
}

#[derive(Clone)]
struct DragClaim {
	response: Response,
	galley_pos: Pos2,
	galley: Arc<Galley>,
	color: Color32,
}

fn drag_claim_id() -> egui::Id {
	egui::Id::unique("serein-drag-claim")
}

/// A pointer below the last label never hits one, so egui keeps the moving end
/// where an upward drag left it. That end stays above the anchor, and the rows
/// below the anchor never join the selection. Put the moving end on the last
/// label without painting it a second time.
pub(crate) fn follow_pointer_below(ui: &mut egui::Ui) {
	let id = drag_claim_id();
	let claim = ui.data(|data| data.get_temp::<DragClaim>(id));
	ui.data_mut(|data| {
		data.remove::<DragClaim>(id);
	});
	let Some(mut claim) = claim else {
		return;
	};
	// Only a pointer that has left the channel. A click on a button under the last
	// line is still inside the viewport, and retargeting it would select that line.
	let pos = ui.input(|input| input.pointer.interact_pos());
	let viewport = ui.data(|data| data.get_temp::<Rect>(viewport_id()));
	let below = pos.is_some_and(|pos| {
		pos.y > claim.response.rect.bottom()
			&& viewport.is_some_and(|viewport| pos.y > viewport.max.y)
	});
	if !below {
		return;
	}
	claim
		.response
		.flags
		.set(egui::response::Flags::CONTAINS_POINTER, true);
	claim
		.response
		.flags
		.set(egui::response::Flags::HOVERED, true);
	let clip = ui.clip_rect();
	ui.set_clip_rect(Rect::NOTHING);
	// The labels already copied this pass. A second registration would append the
	// same line again.
	let copied = ui.input_mut(|input| {
		let mut copied = Vec::new();
		let mut kept = Vec::new();
		for event in std::mem::take(&mut input.events) {
			match event {
				Event::Copy | Event::Cut => copied.push(event),
				other => kept.push(other),
			}
		}
		input.events = kept;
		copied
	});
	LabelSelectionState::label_text_selection(
		ui,
		&claim.response,
		claim.galley_pos,
		claim.galley,
		claim.color,
		Stroke::NONE,
	);
	ui.input_mut(|input| input.events.extend(copied));
	ui.set_clip_rect(clip);
}

/// Rows that must keep registering after they leave the screen.
///
/// egui drops a label selection when either end widget misses a pass. This records
/// those rows. It resets when egui reports no selection and the primary button is up.
pub struct Span<K> {
	ends: Ends<K>,
	/// The pointer left the press. A click that never moves is not an edge drag.
	pulled: bool,
}

impl<K> Default for Span<K> {
	fn default() -> Self {
		Self {
			ends: Ends::None,
			pulled: false,
		}
	}
}

enum Ends<K> {
	None,
	/// Primary is held. Both rows can leave the screen during a wheel tick.
	Dragging {
		anchor: K,
		focus: K,
	},
	/// Released. The focus end may sit on `focus` or the neighbour toward `anchor`.
	Settled {
		anchor: K,
		focus: K,
	},
}

/// Row indices to build this pass even when they are off screen. Ascending and unique.
pub struct Pins(Vec<usize>);

impl Pins {
	pub fn contains(&self, index: usize) -> bool {
		self.0.binary_search(&index).is_ok()
	}

	/// Pinned rows above `index`, ascending.
	pub fn below(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
		self.0.iter().copied().take_while(move |&pin| pin < index)
	}

	pub fn last(&self) -> Option<usize> {
		self.0.last().copied()
	}
}

impl<K: Copy + Eq> Span<K> {
	/// Rows that must register this pass although they may be off screen.
	///
	/// A copy, cut, or primary release pins every row from anchor to focus, because
	/// egui only copies labels registered in that pass. Any other pass pins the ends.
	/// A missing key pins nothing.
	pub fn pins(&self, ctx: &egui::Context, index_of: impl Fn(K) -> Option<usize>) -> Pins {
		let (anchor, focus) = match self.ends {
			Ends::None => return Pins(Vec::new()),
			Ends::Dragging { anchor, focus } | Ends::Settled { anchor, focus } => (anchor, focus),
		};
		let (Some(anchor_at), Some(focus_at)) = (index_of(anchor), index_of(focus)) else {
			return Pins(Vec::new());
		};
		let collecting = ctx.input(|input| {
			input.pointer.primary_released()
				|| input
					.events
					.iter()
					.any(|event| matches!(event, Event::Copy | Event::Cut))
		});
		if collecting {
			let (lo, hi) = if anchor_at <= focus_at {
				(anchor_at, focus_at)
			} else {
				(focus_at, anchor_at)
			};
			return Pins((lo..=hi).collect());
		}
		let mut indices = match self.ends {
			Ends::Dragging { .. } => vec![anchor_at, focus_at],
			Ends::Settled { .. } => {
				let neighbor = if focus_at < anchor_at {
					focus_at + 1
				} else if focus_at > anchor_at {
					focus_at - 1
				} else {
					focus_at
				};
				vec![anchor_at, focus_at, neighbor]
			}
			Ends::None => return Pins(Vec::new()),
		};
		indices.sort_unstable();
		indices.dedup();
		Pins(indices)
	}

	pub fn dragging(&self) -> bool {
		matches!(self.ends, Ends::Dragging { .. })
	}

	/// The press moved far enough to be a drag, not a click on a control.
	pub fn pulled(&self) -> bool {
		self.pulled
	}

	/// Follow this pass's primary button over the rows on screen.
	///
	/// `viewport` is the timeline clip in screen space. `on_screen` lists the rows
	/// built in the visible stage, top to bottom. `exclude` is the scrollbar strip.
	/// A press there must not start a new drag. Call once per pass after those rows.
	pub fn track(
		&mut self,
		ctx: &egui::Context,
		viewport: Rect,
		on_screen: &[(K, Rect)],
		exclude: Rect,
	) {
		let (down, released, pressed, shift, origin, pointer, delta_sq) = ctx.input(|input| {
			(
				input.pointer.primary_down(),
				input.pointer.primary_released(),
				input.pointer.primary_pressed(),
				input.modifiers.shift,
				input.pointer.press_origin(),
				input.pointer.interact_pos().or(input.pointer.hover_pos()),
				input.pointer.delta().length_sq(),
			)
		});
		if !down && !released && !has_selection(ctx) {
			self.ends = Ends::None;
			self.pulled = false;
			return;
		}
		if let (Some(origin), Some(pointer)) = (origin, pointer)
			&& origin.distance_sq(pointer) > 16.0
		{
			self.pulled = true;
		}
		// The copy menu's press lands on a message row. A new drag there forgets the
		// rows already selected, and the next scroll drops the highlight.
		if pressed
			&& !egui::Popup::is_any_open(ctx)
			&& let Some(origin) =
				origin.filter(|pos| viewport.contains(*pos) && !exclude.contains(*pos))
			&& let Some(&(hit, _)) = on_screen.iter().find(|(_, rect)| rect.contains(origin))
		{
			let anchor = if shift {
				self.anchor_key().unwrap_or(hit)
			} else {
				hit
			};
			self.ends = Ends::Dragging {
				anchor,
				focus: anchor,
			};
			self.pulled = false;
		}
		// A wheel tick slides new rows under a pointer that did not move. Following
		// that row abandons the end that scrolled away, and the highlight drops.
		// The viewport edge is the exception: holding there extends the range.
		let at_edge = pointer.is_some_and(|pos| {
			let band = 36.0;
			pos.y < viewport.min.y + band || pos.y > viewport.max.y - band
		});
		let still = (down || released) && !pressed && delta_sq < 0.25 && !at_edge;
		if !still
			&& let Ends::Dragging { anchor, .. } = self.ends
			&& let Some(pointer) = pointer
		{
			let y = pointer.y.clamp(viewport.min.y, viewport.max.y);
			if let Some(hit) = row_at_y(on_screen, y) {
				self.ends = Ends::Dragging { anchor, focus: hit };
			}
		}
		if released && let Ends::Dragging { anchor, focus } = self.ends {
			self.ends = Ends::Settled { anchor, focus };
		}
	}

	fn anchor_key(&self) -> Option<K> {
		match self.ends {
			Ends::None => None,
			Ends::Dragging { anchor, .. } | Ends::Settled { anchor, .. } => Some(anchor),
		}
	}
}

fn row_at_y<K: Copy>(rows: &[(K, Rect)], y: f32) -> Option<K> {
	if let Some((key, _)) = rows
		.iter()
		.find(|(_, rect)| rect.min.y <= y && y <= rect.max.y)
	{
		return Some(*key);
	}
	rows.iter()
		.min_by(|(_, left), (_, right)| {
			(left.center().y - y)
				.abs()
				.total_cmp(&(right.center().y - y).abs())
		})
		.map(|(key, _)| *key)
}

pub(crate) fn band_sense() -> Sense {
	Sense::CLICK | Sense::DRAG
}

/// Paint a deferred widget galley and register its selection.
///
/// An open menu clears the pointer flag so the click stays with the menu.
fn show_embed(ui: &mut egui::Ui, embed: &Embed, menu_open: bool) {
	if !embed.response.rect.is_positive() || !reaches(ui, embed.response.rect) {
		return;
	}
	// The galley carries its own per-token colours; the fallback only covers unstyled glyphs.
	let color = ui.visuals().text_color();
	let mut response = embed.response.clone();
	if menu_open {
		response
			.flags
			.set(egui::response::Flags::CONTAINS_POINTER, false);
	}
	register(ui, &response, embed.galley_pos, embed.galley.clone(), color);
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
	}
}

fn paint_artwork(ui: &egui::Ui, art: &Artwork) {
	if !ui.is_rect_visible(art.rect) {
		return;
	}
	let size = art.rect.width();
	if let Some(image) = &art.image {
		let painted = image.calc_size(egui::Vec2::splat(size), image.size());
		image.paint_at(ui, Rect::from_center_size(art.rect.center(), painted));
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
}
