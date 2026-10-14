//! Logical chat selection across ordinary and bidi-shaped runs.
use egui::{Event, Id, Pos2, Rect, Response, Stroke};
use std::{ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const COPY_BYTES: usize = 4 * 1024 * 1024;
const RUNS: usize = 4096;

#[derive(Clone, Copy)]
struct End {
	id: Id,
	order: usize,
	byte: usize,
	hash: u64,
}

#[derive(Clone)]
pub(super) enum Source {
	Mapped(Arc<crate::rtl::Layout>),
	Native(Arc<egui::epaint::Galley>),
}
impl Source {
	fn text(&self) -> &str {
		match self {
			Self::Mapped(layout) => &layout.source,
			Self::Native(galley) => &galley.job.text,
		}
	}
	fn bytes(&self) -> usize {
		match self {
			Self::Mapped(layout) => layout.allocated_bytes(),
			Self::Native(galley) => crate::rtl::galley_bytes(galley),
		}
	}
}
struct Observed {
	order: usize,
	rect: Rect,
	source: Source,
}

#[derive(Default)]
pub(super) struct Selection {
	anchor: Option<End>,
	focus: Option<End>,
	order: usize,
	copy: String,
	requested: bool,
	select_all: bool,
	dragging: bool,
	mapped: bool,
	anchor_seen: bool,
	focus_seen: bool,
	observed: Vec<Observed>,
	observed_bytes: usize,
	overflow: bool,
	press: bool,
	claimed: bool,
}

impl egui::Plugin for Selection {
	fn debug_name(&self) -> &'static str {
		"Logical chat selection"
	}
	fn input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
		if input.events.iter().any(|event| {
			matches!(
				event,
				Event::Key {
					key: egui::Key::Escape,
					pressed: true,
					..
				}
			)
		}) {
			self.anchor = None;
			self.focus = None;
			self.mapped = false;
			self.dragging = false;
		}
		self.press = !egui::Popup::is_any_open(ctx)
			&& input.events.iter().any(|event| {
				matches!(
					event,
					Event::PointerButton {
						button: egui::PointerButton::Primary,
						pressed: true,
						..
					}
				)
			});
		self.select_all = input.events.iter().any(|event| {
			matches!(event,
			Event::Key { key: egui::Key::A, pressed: true, modifiers, .. } if modifiers.command)
		});
		self.requested = input
			.events
			.iter()
			.any(|event| matches!(event, Event::Copy));
	}
	fn on_begin_pass(&mut self, _: &mut egui::Ui) {
		self.order = 0;
		self.claimed = false;
		self.anchor_seen = false;
		self.focus_seen = false;
		self.copy.clear();
		self.observed.clear();
		self.observed_bytes = 0;
		self.overflow = false;
	}
	fn on_end_pass(&mut self, ui: &mut egui::Ui) {
		// Virtualized/offscreen runs may be absent from a pass. Keep only their two
		// bounded endpoints; copy remains disabled until both matching sources return.
		if self.press && !self.claimed {
			self.anchor = None;
			self.focus = None;
			self.mapped = false;
		}
		let observed = std::mem::take(&mut self.observed);
		if self.mapped && self.requested && self.anchor_seen && self.focus_seen && !self.overflow {
			self.assemble_copy(&observed);
		}
		if !self.anchor_seen || !self.focus_seen || self.overflow {
			self.copy.clear();
		}
		let editing = ui
			.ctx()
			.memory(|memory| memory.focused())
			.is_some_and(|id| egui::text_edit::TextEditState::load(ui.ctx(), id).is_some());
		if self.mapped && self.requested && !self.copy.is_empty() && !editing {
			ui.ctx().copy_text(std::mem::take(&mut self.copy));
		}
		if !ui.input(|input| input.pointer.primary_down()) {
			self.dragging = false;
		}
		self.requested = false;
		self.select_all = false;
	}
}

impl Selection {
	pub fn active(&self) -> bool {
		self.mapped
			&& matches!((self.anchor,self.focus),(Some(a),Some(b)) if a.id != b.id || a.byte != b.byte)
	}

	/// Returns a logical UTF-8 range. The caller supplies exact hit testing for its layout.
	pub fn run(
		&mut self,
		ui: &egui::Ui,
		response: &Response,
		source: Source,
		is_mapped: bool,
		cursor: impl Fn(Pos2) -> usize,
	) -> Range<usize> {
		let order = self.order;
		self.order += 1;
		if order >= RUNS {
			self.overflow = true;
			return 0..0;
		}
		let text = source.text();
		let hash = egui::epaint::util::hash(text);
		let pointer = ui.input(|input| input.pointer.hover_pos());
		let pressed = !egui::Popup::is_any_open(ui.ctx())
			&& ui.input(|input| input.pointer.primary_pressed());
		let down =
			!egui::Popup::is_any_open(ui.ctx()) && ui.input(|input| input.pointer.primary_down());
		let shift = ui.input(|input| input.modifiers.shift);
		if pressed
			&& response.contains_pointer()
			&& let Some(pos) = pointer
		{
			self.claimed = true;
			self.mapped = is_mapped || (shift && self.mapped);
			let end = End {
				id: response.id,
				order,
				byte: cursor(pos).min(text.len()),
				hash,
			};
			if !shift || self.anchor.is_none() {
				self.anchor = Some(end);
			}
			self.focus = Some(end);
			self.dragging = true;
			if self.mapped {
				ui.ctx()
					.plugin::<egui::text_selection::LabelSelectionState>()
					.lock()
					.clear_selection();
			}
		}
		if self.dragging
			&& down && response.rect.contains(pointer.unwrap_or(Pos2::ZERO))
			&& let Some(pos) = pointer
		{
			if is_mapped {
				self.mapped = true;
				ui.ctx()
					.plugin::<egui::text_selection::LabelSelectionState>()
					.lock()
					.clear_selection();
			}
			let new = End {
				id: response.id,
				order,
				byte: cursor(pos).min(text.len()),
				hash,
			};
			if self
				.focus
				.is_none_or(|old| old.id != new.id || old.byte != new.byte)
			{
				ui.ctx().request_repaint();
			}
			self.focus = Some(new);
		}
		// Preserve native label word/line selection while endpoints stay logical.
		if (is_mapped || self.mapped)
			&& (response.double_clicked() || response.triple_clicked())
			&& let Some(pos) = pointer
		{
			let byte = cursor(pos).min(text.len());
			let range = if response.triple_clicked() {
				let start = text[..byte]
					.rfind(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}'])
					.map_or(0, |at| at + text[at..].chars().next().unwrap().len_utf8());
				let end = text[byte..]
					.find(['\n', '\r', '\u{0085}', '\u{2028}', '\u{2029}'])
					.map_or(text.len(), |at| byte + at);
				start..end
			} else {
				text.split_word_bound_indices()
					.find_map(|(start, word)| {
						let end = start + word.len();
						(start <= byte && byte < end).then_some(start..end)
					})
					.unwrap_or(byte..byte)
			};
			self.anchor = Some(End {
				id: response.id,
				order,
				byte: range.start,
				hash,
			});
			self.focus = Some(End {
				id: response.id,
				order,
				byte: range.end,
				hash,
			});
			self.mapped = true;
			self.claimed = true;
			self.dragging = false;
			ui.ctx()
				.plugin::<egui::text_selection::LabelSelectionState>()
				.lock()
				.clear_selection();
		}
		if [self.anchor, self.focus]
			.iter()
			.flatten()
			.any(|end| end.id == response.id && end.hash != hash)
		{
			self.anchor = None;
			self.focus = None;
			self.dragging = false;
			return 0..0;
		}
		if self.mapped && self.select_all && self.focus.is_some_and(|end| end.id == response.id) {
			self.anchor = Some(End {
				id: response.id,
				order,
				byte: 0,
				hash,
			});
			self.focus = Some(End {
				id: response.id,
				order,
				byte: text.len(),
				hash,
			});
		}
		for end in [&mut self.anchor, &mut self.focus]
			.into_iter()
			.flatten()
			.filter(|end| end.id == response.id)
		{
			if end.order != order {
				ui.ctx().request_repaint();
				end.order = order;
			}
		}

		if self.anchor.is_some_and(|end| end.id == response.id) {
			self.anchor_seen = true;
		}
		if self.focus.is_some_and(|end| end.id == response.id) {
			self.focus_seen = true;
		}
		if self.requested || (self.mapped && egui::Popup::is_any_open(ui.ctx())) {
			let bytes = source.bytes();
			if self
				.observed_bytes
				.saturating_add(bytes)
				.saturating_add((self.observed.len() + 1) * std::mem::size_of::<Observed>())
				> COPY_BYTES
			{
				self.overflow = true;
			} else {
				self.observed.reserve_exact(1);
				if self.observed_bytes
					+ bytes + self.observed.capacity() * std::mem::size_of::<Observed>()
					> COPY_BYTES
				{
					self.overflow = true;
				} else {
					self.observed_bytes += bytes;
					self.observed.push(Observed {
						order,
						rect: response.rect,
						source: source.clone(),
					});
				}
			}
		}
		let (Some(a), Some(b)) = (self.anchor, self.focus) else {
			return 0..0;
		};
		let (lo, hi) = if (a.order, a.byte) <= (b.order, b.byte) {
			(a, b)
		} else {
			(b, a)
		};
		if order < lo.order || order > hi.order {
			return 0..0;
		}
		let from = if order == lo.order { lo.byte } else { 0 };
		let to = if order == hi.order {
			hi.byte
		} else {
			text.len()
		};
		let Some(_) = text.get(from..to) else {
			return 0..0;
		};

		from..to
	}
	// Resolve all current-pass endpoint ordinals before constructing any clipboard text.
	fn assemble_copy(&mut self, observed: &[Observed]) {
		let (Some(a), Some(b)) = (self.anchor, self.focus) else {
			return;
		};
		let (lo, hi) = if (a.order, a.byte) <= (b.order, b.byte) {
			(a, b)
		} else {
			(b, a)
		};
		let mut previous: Option<Rect> = None;
		for run in observed
			.iter()
			.filter(|run| (lo.order..=hi.order).contains(&run.order))
		{
			let text = run.source.text();
			let from = if run.order == lo.order { lo.byte } else { 0 };
			let to = if run.order == hi.order {
				hi.byte
			} else {
				text.len()
			};
			let Some(selected) = text.get(from..to) else {
				self.overflow = true;
				return;
			};
			if selected.is_empty() {
				continue;
			}
			let newline = !self.copy.ends_with('\n')
				&& !selected.starts_with('\n')
				&& previous.is_some_and(|previous| run.rect.top() >= previous.bottom() - 1.0);
			let needed = self.copy.len() + selected.len() + usize::from(newline);
			if needed > COPY_BYTES {
				self.overflow = true;
				return;
			}
			if needed > self.copy.capacity() {
				self.copy.reserve_exact(needed - self.copy.len());
			}
			if self.copy.capacity() > COPY_BYTES {
				self.copy = String::new();
				self.overflow = true;
				return;
			}
			if newline {
				self.copy.push('\n');
			}
			self.copy.push_str(selected);
			previous = Some(run.rect);
		}
	}
}

// Revalidate visible source hashes/order under an open menu without pointer handling.
pub(super) fn observe(ui: &mut egui::Ui, id: Id, rect: Rect, source: Source) {
	let response = ui.interact(rect, id, egui::Sense::hover());
	ui.ctx()
		.plugin_or_default::<Selection>()
		.lock()
		.run(ui, &response, source, false, |_| 0);
}

pub(super) fn request_copy(ctx: &egui::Context) -> bool {
	let Some(plugin) = ctx.plugin_opt::<Selection>() else {
		return false;
	};
	let mut selection = plugin.lock();
	if !selection.mapped {
		return false;
	}
	// An invalidated mapped range must never fall back to a previous native copy.
	selection.requested = true;
	true
}

pub(super) fn native(
	ui: &mut egui::Ui,
	response: &Response,
	pos: Pos2,
	mut galley: std::sync::Arc<egui::epaint::Galley>,
	color: egui::Color32,
) {
	let text = &galley.job.text;
	let bytes = |point: Pos2| {
		let index = galley.cursor_from_pos(point - pos).index.0;
		text.char_indices()
			.nth(index)
			.map_or(text.len(), |(byte, _)| byte)
	};
	let selected = ui.ctx().plugin_or_default::<Selection>().lock().run(
		ui,
		response,
		Source::Native(galley.clone()),
		false,
		bytes,
	);
	if !ui.ctx().plugin_or_default::<Selection>().lock().mapped {
		egui::text_selection::LabelSelectionState::label_text_selection(
			ui,
			response,
			pos,
			galley,
			color,
			Stroke::NONE,
		);
		return;
	}
	if !selected.is_empty() {
		let from = text[..selected.start].chars().count();
		let to = text[..selected.end].chars().count();
		let range = egui::text_selection::CCursorRange::two(
			egui::text::CCursor::new(from),
			egui::text::CCursor::new(to),
		);
		egui::text_selection::visuals::paint_text_selection(
			&mut galley,
			ui.visuals(),
			&range,
			None,
		);
	}
	ui.painter()
		.add(egui::epaint::TextShape::new(pos, galley, color));
}
