// Native editing adapted from GPUI examples/input.rs (Apache-2.0).
use std::ops::Range;

use gpui::{
	App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
	EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding,
	LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
	ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div,
	fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::*;

actions!(
	serein_input,
	[
		Backspace,
		Delete,
		Left,
		Right,
		SelectLeft,
		SelectRight,
		SelectAll,
		Home,
		End,
		ShowCharacterPalette,
		Paste,
		Cut,
		Copy,
		Submit,
		Newline,
		Up,
		Down,
		SelectUp,
		SelectDown,
	]
);

pub struct Input {
	focus_handle: FocusHandle,
	content: SharedString,
	placeholder: SharedString,
	selected_range: Range<usize>,
	selection_reversed: bool,
	marked_range: Option<Range<usize>>,
	last_layout: Vec<(usize, Point<Pixels>, ShapedLine)>,
	is_selecting: bool,
}

impl Input {
	pub fn new(cx: &mut Context<Self>) -> Self {
		Self {
			focus_handle: cx.focus_handle().tab_stop(true),
			content: "".into(),
			placeholder: "Message…".into(),
			selected_range: 0..0,
			selection_reversed: false,
			marked_range: None,
			last_layout: Vec::new(),
			is_selecting: false,
		}
	}

	pub fn set_placeholder(&mut self, placeholder: String, cx: &mut Context<Self>) {
		if self.placeholder != placeholder {
			self.placeholder = placeholder.into();
			cx.notify();
		}
	}

	pub fn value(&self) -> &str {
		&self.content
	}

	pub fn set_value(&mut self, value: String, cx: &mut Context<Self>) {
		self.content = value
			.chars()
			.take(client_core::MAX_CONTENT)
			.collect::<String>()
			.into();
		self.selected_range = self.content.len()..self.content.len();
		self.selection_reversed = false;
		self.marked_range = None;
		cx.notify();
	}

	fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
		if self.marked_range.is_none() {
			cx.emit(Submit);
		}
	}

	fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
		self.replace_text_in_range(None, "\n", window, cx);
	}

	fn accepts(&self, range: &Range<usize>, text: &str) -> bool {
		range.start <= range.end
			&& range.end <= self.content.len()
			&& self.content.is_char_boundary(range.start)
			&& self.content.is_char_boundary(range.end)
			&& text.len() <= client_core::MAX_CONTENT * 4
			&& self.content.len() - (range.end - range.start) + text.len()
				<= client_core::MAX_CONTENT * 4
			&& self.content[..range.start].chars().count()
				+ text.chars().count()
				+ self.content[range.end..].chars().count()
				<= client_core::MAX_CONTENT
	}

	fn vertical_offset(&self, down: bool) -> usize {
		let cursor = self.cursor_offset();
		let start = self.content[..cursor]
			.rfind('\n')
			.map_or(0, |index| index + 1);
		let column = self.content[start..cursor].graphemes(true).count();
		let target = if down {
			self.content[cursor..]
				.find('\n')
				.map(|index| cursor + index + 1)
		} else if start > 0 {
			Some(
				self.content[..start - 1]
					.rfind('\n')
					.map_or(0, |index| index + 1),
			)
		} else {
			None
		};
		let Some(target) = target else {
			return cursor;
		};
		let line = self.content[target..].split('\n').next().unwrap_or("");
		target
			+ line
				.grapheme_indices(true)
				.nth(column)
				.map_or(line.len(), |(index, _)| index)
	}

	fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
		self.move_to(self.vertical_offset(false), cx);
	}
	fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
		self.move_to(self.vertical_offset(true), cx);
	}
	fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
		self.select_to(self.vertical_offset(false), cx);
	}
	fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
		self.select_to(self.vertical_offset(true), cx);
	}

	fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
		if self.selected_range.is_empty() {
			self.move_to(self.previous_boundary(self.cursor_offset()), cx);
		} else {
			self.move_to(self.selected_range.start, cx)
		}
	}

	fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
		if self.selected_range.is_empty() {
			self.move_to(self.next_boundary(self.selected_range.end), cx);
		} else {
			self.move_to(self.selected_range.end, cx)
		}
	}

	fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
		self.select_to(self.previous_boundary(self.cursor_offset()), cx);
	}

	fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
		self.select_to(self.next_boundary(self.cursor_offset()), cx);
	}

	fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
		self.move_to(0, cx);
		self.select_to(self.content.len(), cx)
	}

	fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
		self.move_to(0, cx);
	}

	fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
		self.move_to(self.content.len(), cx);
	}

	fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
		if self.selected_range.is_empty() {
			self.select_to(self.previous_boundary(self.cursor_offset()), cx)
		}
		self.replace_text_in_range(None, "", window, cx)
	}

	fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
		if self.selected_range.is_empty() {
			self.select_to(self.next_boundary(self.cursor_offset()), cx)
		}
		self.replace_text_in_range(None, "", window, cx)
	}

	fn on_mouse_down(
		&mut self,
		event: &MouseDownEvent,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		window.focus(&self.focus_handle, cx);
		self.is_selecting = true;

		if event.modifiers.shift {
			self.select_to(self.index_for_mouse_position(event.position), cx);
		} else {
			self.move_to(self.index_for_mouse_position(event.position), cx)
		}
	}

	fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
		self.is_selecting = false;
	}

	fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
		if self.is_selecting {
			self.select_to(self.index_for_mouse_position(event.position), cx);
		}
	}

	fn show_character_palette(
		&mut self,
		_: &ShowCharacterPalette,
		window: &mut Window,
		_: &mut Context<Self>,
	) {
		window.show_character_palette();
	}

	fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
		if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
			self.replace_text_in_range(None, &text, window, cx);
		}
	}

	fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
		if !self.selected_range.is_empty() {
			cx.write_to_clipboard(ClipboardItem::new_string(
				self.content[self.selected_range.clone()].to_string(),
			));
		}
	}
	fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
		if !self.selected_range.is_empty() {
			cx.write_to_clipboard(ClipboardItem::new_string(
				self.content[self.selected_range.clone()].to_string(),
			));
			self.replace_text_in_range(None, "", window, cx)
		}
	}

	fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
		let offset = self
			.content
			.floor_char_boundary(offset.min(self.content.len()));
		self.selected_range = offset..offset;
		self.selection_reversed = false;
		cx.notify()
	}

	fn cursor_offset(&self) -> usize {
		if self.selection_reversed {
			self.selected_range.start
		} else {
			self.selected_range.end
		}
	}

	fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
		if self.content.is_empty() {
			return 0;
		}

		let Some((_, first, _)) = self.last_layout.first() else {
			return 0;
		};
		let index = ((position.y - first.y) / px(24.)).floor().max(0.) as usize;
		let Some((start, origin, line)) =
			self.last_layout.get(index.min(self.last_layout.len() - 1))
		else {
			return 0;
		};
		start + line.closest_index_for_x(position.x - origin.x)
	}

	fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
		let offset = self
			.content
			.floor_char_boundary(offset.min(self.content.len()));
		if self.selection_reversed {
			self.selected_range.start = offset
		} else {
			self.selected_range.end = offset
		};
		if self.selected_range.end < self.selected_range.start {
			self.selection_reversed = !self.selection_reversed;
			self.selected_range = self.selected_range.end..self.selected_range.start;
		}
		cx.notify()
	}

	fn offset_from_utf16(&self, offset: usize) -> usize {
		let mut utf8_offset = 0;
		let mut utf16_count = 0;

		for ch in self.content.chars() {
			if utf16_count >= offset {
				break;
			}
			utf16_count += ch.len_utf16();
			utf8_offset += ch.len_utf8();
		}

		utf8_offset
	}

	fn offset_to_utf16(&self, offset: usize) -> usize {
		let mut utf16_offset = 0;
		let mut utf8_count = 0;

		for ch in self.content.chars() {
			if utf8_count >= offset {
				break;
			}
			utf8_count += ch.len_utf8();
			utf16_offset += ch.len_utf16();
		}

		utf16_offset
	}

	fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
		self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
	}

	fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
		{
			let start = self.offset_from_utf16(range_utf16.start);
			start..self.offset_from_utf16(range_utf16.end).max(start)
		}
	}

	fn previous_boundary(&self, offset: usize) -> usize {
		self.content
			.grapheme_indices(true)
			.rev()
			.find_map(|(idx, _)| (idx < offset).then_some(idx))
			.unwrap_or(0)
	}

	fn next_boundary(&self, offset: usize) -> usize {
		self.content
			.grapheme_indices(true)
			.find_map(|(idx, _)| (idx > offset).then_some(idx))
			.unwrap_or(self.content.len())
	}
}

impl EntityInputHandler for Input {
	fn text_for_range(
		&mut self,
		range_utf16: Range<usize>,
		actual_range: &mut Option<Range<usize>>,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Option<String> {
		let range = self.range_from_utf16(&range_utf16);
		actual_range.replace(self.range_to_utf16(&range));
		Some(self.content[range].to_string())
	}

	fn selected_text_range(
		&mut self,
		_ignore_disabled_input: bool,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Option<UTF16Selection> {
		Some(UTF16Selection {
			range: self.range_to_utf16(&self.selected_range),
			reversed: self.selection_reversed,
		})
	}

	fn marked_text_range(
		&self,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Option<Range<usize>> {
		self.marked_range
			.as_ref()
			.map(|range| self.range_to_utf16(range))
	}

	fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
		self.marked_range = None;
	}

	fn replace_text_in_range(
		&mut self,
		range_utf16: Option<Range<usize>>,
		new_text: &str,
		_: &mut Window,
		cx: &mut Context<Self>,
	) {
		let range = range_utf16
			.as_ref()
			.map(|range_utf16| self.range_from_utf16(range_utf16))
			.or(self.marked_range.clone())
			.unwrap_or(self.selected_range.clone());

		if !self.accepts(&range, new_text) {
			return;
		}
		self.content =
			(self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
				.into();
		self.selected_range = range.start + new_text.len()..range.start + new_text.len();
		self.selection_reversed = false;
		self.marked_range.take();
		cx.notify();
	}

	fn replace_and_mark_text_in_range(
		&mut self,
		range_utf16: Option<Range<usize>>,
		new_text: &str,
		new_selected_range_utf16: Option<Range<usize>>,
		_window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let range = range_utf16
			.as_ref()
			.map(|range_utf16| self.range_from_utf16(range_utf16))
			.or(self.marked_range.clone())
			.unwrap_or(self.selected_range.clone());

		if !self.accepts(&range, new_text) {
			return;
		}
		self.content =
			(self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
				.into();
		if !new_text.is_empty() {
			self.marked_range = Some(range.start..range.start + new_text.len());
		} else {
			self.marked_range = None;
		}
		self.selected_range = new_selected_range_utf16
			.as_ref()
			.map(|selection| {
				let offset = |units| {
					let mut count = 0;
					new_text
						.chars()
						.take_while(|ch| {
							let take = count < units;
							count += ch.len_utf16();
							take
						})
						.map(char::len_utf8)
						.sum::<usize>()
				};
				let start = offset(selection.start);
				let end = offset(selection.end).max(start);
				range.start + start..range.start + end
			})
			.unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

		cx.notify();
	}

	fn bounds_for_range(
		&mut self,
		range_utf16: Range<usize>,
		bounds: Bounds<Pixels>,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Option<Bounds<Pixels>> {
		let range = self.range_from_utf16(&range_utf16);
		let (start, origin, line) = self
			.last_layout
			.iter()
			.rev()
			.find(|(start, _, _)| *start <= range.start)?;
		let left = origin.x + line.x_for_index((range.start - start).min(line.text.len()));
		let right =
			origin.x + line.x_for_index(range.end.saturating_sub(*start).min(line.text.len()));
		let _ = bounds;
		Some(Bounds::from_corners(
			point(left, origin.y),
			point(right, origin.y + px(24.)),
		))
	}

	fn character_index_for_point(
		&mut self,
		point: Point<Pixels>,
		_window: &mut Window,
		_cx: &mut Context<Self>,
	) -> Option<usize> {
		Some(self.offset_to_utf16(self.index_for_mouse_position(point)))
	}
}

struct TextElement {
	input: Entity<Input>,
}

struct PrepaintState {
	lines: Vec<(usize, Point<Pixels>, ShapedLine)>,
	cursor: Option<PaintQuad>,
	selections: Vec<PaintQuad>,
}

impl IntoElement for TextElement {
	type Element = Self;
	fn into_element(self) -> Self {
		self
	}
}

impl Element for TextElement {
	type RequestLayoutState = ();
	type PrepaintState = PrepaintState;

	fn id(&self) -> Option<ElementId> {
		None
	}
	fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
		None
	}

	fn request_layout(
		&mut self,
		_id: Option<&GlobalElementId>,
		_inspector_id: Option<&gpui::InspectorElementId>,
		window: &mut Window,
		cx: &mut App,
	) -> (LayoutId, ()) {
		let rows = self.input.read(cx).content.split('\n').count().min(4);
		let mut style = Style::default();
		style.size.width = relative(1.).into();
		style.size.height = px(24. * rows as f32).into();
		(window.request_layout(style, [], cx), ())
	}

	fn prepaint(
		&mut self,
		_id: Option<&GlobalElementId>,
		_inspector_id: Option<&gpui::InspectorElementId>,
		bounds: Bounds<Pixels>,
		_: &mut (),
		window: &mut Window,
		cx: &mut App,
	) -> PrepaintState {
		let input = self.input.read(cx);
		let cursor = input.cursor_offset();
		let cursor_row = input.content[..cursor]
			.bytes()
			.filter(|byte| *byte == b'\n')
			.count();
		let first_row = cursor_row.saturating_sub(3);
		let style = window.text_style();
		let font_size = style.font_size.to_pixels(window.rem_size());
		let mut state = PrepaintState {
			lines: Vec::new(),
			cursor: None,
			selections: Vec::new(),
		};
		let mut offset = 0;
		// ponytail: four unwrapped rows; use a full editor when wrapping/navigation needs grow.
		for (row, text) in input.content.split('\n').enumerate() {
			let start = offset;
			offset += text.len() + 1;
			if row < first_row {
				continue;
			}
			if row >= first_row + 4 {
				break;
			}
			let display: SharedString = if input.content.is_empty() {
				input.placeholder.clone()
			} else {
				text.to_owned().into()
			};
			let base = TextRun {
				len: display.len(),
				font: style.font(),
				color: if input.content.is_empty() {
					super::color(super::palette().muted).into()
				} else {
					style.color
				},
				background_color: None,
				underline: None,
				strikethrough: None,
			};
			let runs = if let Some(marked) = input
				.marked_range
				.as_ref()
				.filter(|marked| marked.start < start + text.len() && marked.end > start)
			{
				let a = marked.start.saturating_sub(start).min(text.len());
				let b = marked.end.saturating_sub(start).min(text.len());
				vec![
					TextRun {
						len: a,
						..base.clone()
					},
					TextRun {
						len: b - a,
						underline: Some(UnderlineStyle {
							color: Some(base.color),
							thickness: px(1.),
							wavy: false,
						}),
						..base.clone()
					},
					TextRun {
						len: display.len() - b,
						..base
					},
				]
				.into_iter()
				.filter(|run| run.len > 0)
				.collect::<Vec<_>>()
			} else {
				vec![base]
			};
			let line = window
				.text_system()
				.shape_line(display, font_size, &runs, None);
			let cursor_x = line.x_for_index(cursor.saturating_sub(start).min(text.len()));
			let shift = if row == cursor_row {
				(cursor_x - bounds.size.width + px(8.)).max(px(0.))
			} else {
				px(0.)
			};
			let origin = point(
				bounds.left() - shift,
				bounds.top() + px((row - first_row) as f32 * 24.),
			);
			if row == cursor_row && input.selected_range.is_empty() {
				state.cursor = Some(fill(
					Bounds::new(point(origin.x + cursor_x, origin.y), size(px(2.), px(24.))),
					super::color(super::palette().accent),
				));
			}
			let selection = &input.selected_range;
			if selection.start < start + text.len() && selection.end > start {
				let a = selection.start.saturating_sub(start).min(text.len());
				let b = selection.end.saturating_sub(start).min(text.len());
				state.selections.push(fill(
					Bounds::from_corners(
						point(origin.x + line.x_for_index(a), origin.y),
						point(origin.x + line.x_for_index(b), origin.y + px(24.)),
					),
					super::color(super::palette().selected),
				));
			}
			state.lines.push((start, origin, line));
		}
		state
	}

	fn paint(
		&mut self,
		_id: Option<&GlobalElementId>,
		_inspector_id: Option<&gpui::InspectorElementId>,
		bounds: Bounds<Pixels>,
		_: &mut (),
		state: &mut PrepaintState,
		window: &mut Window,
		cx: &mut App,
	) {
		let focus = self.input.read(cx).focus_handle.clone();
		window.handle_input(
			&focus,
			ElementInputHandler::new(bounds, self.input.clone()),
			cx,
		);
		for selection in state.selections.drain(..) {
			window.paint_quad(selection);
		}
		for (_, origin, line) in &state.lines {
			let _ = line.paint(*origin, px(24.), gpui::TextAlign::Left, None, window, cx);
		}
		if focus.is_focused(window)
			&& let Some(cursor) = state.cursor.take()
		{
			window.paint_quad(cursor);
		}
		self.input.update(cx, |input, _| {
			input.last_layout = std::mem::take(&mut state.lines);
		});
	}
}

impl Render for Input {
	fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		div()
			.flex()
			.key_context("SereinInput")
			.track_focus(&self.focus_handle(cx))
			.cursor(CursorStyle::IBeam)
			.on_action(cx.listener(Self::submit))
			.on_action(cx.listener(Self::newline))
			.on_action(cx.listener(Self::up))
			.on_action(cx.listener(Self::down))
			.on_action(cx.listener(Self::select_up))
			.on_action(cx.listener(Self::select_down))
			.on_action(cx.listener(Self::backspace))
			.on_action(cx.listener(Self::delete))
			.on_action(cx.listener(Self::left))
			.on_action(cx.listener(Self::right))
			.on_action(cx.listener(Self::select_left))
			.on_action(cx.listener(Self::select_right))
			.on_action(cx.listener(Self::select_all))
			.on_action(cx.listener(Self::home))
			.on_action(cx.listener(Self::end))
			.on_action(cx.listener(Self::show_character_palette))
			.on_action(cx.listener(Self::paste))
			.on_action(cx.listener(Self::cut))
			.on_action(cx.listener(Self::copy))
			.on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
			.on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
			.on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
			.on_mouse_move(cx.listener(Self::on_mouse_move))
			.w_full()
			.overflow_hidden()
			.text_color(super::color(super::palette().text))
			.line_height(px(24.))
			.text_size(px(15.))
			.child(
				div()
					.w_full()
					.py(px(4.))
					.child(TextElement { input: cx.entity() }),
			)
	}
}

impl Focusable for Input {
	fn focus_handle(&self, _: &App) -> FocusHandle {
		self.focus_handle.clone()
	}
}

impl EventEmitter<Submit> for Input {}

pub fn init(cx: &mut App) {
	cx.bind_keys([
		KeyBinding::new("backspace", Backspace, Some("SereinInput")),
		KeyBinding::new("delete", Delete, Some("SereinInput")),
		KeyBinding::new("left", Left, Some("SereinInput")),
		KeyBinding::new("right", Right, Some("SereinInput")),
		KeyBinding::new("shift-left", SelectLeft, Some("SereinInput")),
		KeyBinding::new("shift-right", SelectRight, Some("SereinInput")),
		KeyBinding::new("cmd-a", SelectAll, Some("SereinInput")),
		KeyBinding::new("cmd-v", Paste, Some("SereinInput")),
		KeyBinding::new("cmd-c", Copy, Some("SereinInput")),
		KeyBinding::new("cmd-x", Cut, Some("SereinInput")),
		KeyBinding::new("home", Home, Some("SereinInput")),
		KeyBinding::new("end", End, Some("SereinInput")),
		KeyBinding::new("cmd-left", Home, Some("SereinInput")),
		KeyBinding::new("cmd-right", End, Some("SereinInput")),
		KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, Some("SereinInput")),
		KeyBinding::new("enter", Submit, Some("SereinInput")),
		KeyBinding::new("shift-enter", Newline, Some("SereinInput")),
		KeyBinding::new("up", Up, Some("SereinInput")),
		KeyBinding::new("down", Down, Some("SereinInput")),
		KeyBinding::new("shift-up", SelectUp, Some("SereinInput")),
		KeyBinding::new("shift-down", SelectDown, Some("SereinInput")),
	]);
}
