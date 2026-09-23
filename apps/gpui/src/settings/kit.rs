//! Settings controls drawn like `ui::design`'s: eyebrow headings, raised cards, pill switches,
//! sliders, inline buttons, hints and notices. Sizes follow the egui helpers of the same name.
use crate::Serein;
use crate::theme::{Icon, color, icon, palette, tint};
use gpui::{prelude::*, *};
use std::{cell::Cell, ops::RangeInclusive, rc::Rc};

thread_local! {
	/// Slider being dragged; only one pointer drag can be in flight.
	static DRAGGING: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// Uppercase section heading, 12 px semibold.
pub fn eyebrow(text: &str) -> Div {
	div()
		.text_size(px(12.))
		.font_weight(FontWeight::SEMIBOLD)
		.text_color(color(palette().muted))
		.child(text.to_uppercase())
}

/// Rounded card on the raised surface that groups related rows.
pub fn card() -> Div {
	let p = palette();
	div()
		.w_full()
		.px_4()
		.py_3()
		.rounded(px(8.))
		.bg(color(p.raised))
		.border_1()
		.border_color(color(p.border))
		.flex()
		.flex_col()
}

/// Group title above a card, the way every settings page introduces a group.
pub fn group(title: &str, body: Div) -> Div {
	div()
		.w_full()
		.pt_1()
		.flex()
		.flex_col()
		.gap_2()
		.child(eyebrow(title))
		.child(body)
}

/// Hairline between rows inside a [`card`].
pub fn divider() -> Div {
	div()
		.w_full()
		.h(px(1.))
		.my(px(6.))
		.bg(color(palette().border))
}

fn title(text: &str) -> Div {
	div()
		.text_size(px(16.))
		.font_weight(FontWeight::MEDIUM)
		.text_color(color(palette().text_strong))
		.child(text.to_owned())
}

fn detail(text: &str) -> Div {
	div()
		.text_size(px(13.))
		.line_height(px(18.))
		.text_color(color(palette().muted))
		.child(text.to_owned())
}

/// Small muted explanation under a control.
pub fn hint(text: &str) -> Div {
	div()
		.pt_1()
		.text_size(px(12.))
		.line_height(px(16.))
		.text_color(color(palette().muted))
		.child(text.to_owned())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
	Info,
	Warning,
}

/// Tinted callout for status and "not available here" explanations.
pub fn notice(level: Level, text: &str) -> Div {
	let p = palette();
	let tone = match level {
		Level::Info => p.accent,
		Level::Warning => p.warning,
	};
	div()
		.w_full()
		.px_3()
		.py(px(10.))
		.rounded(px(8.))
		.bg(tint(tone, 0.13))
		.border_1()
		.border_color(tint(tone, 0.45))
		.text_size(px(13.))
		.line_height(px(18.))
		.text_color(color(p.text))
		.child(text.to_owned())
}

/// Pill switch row: title and optional detail on the left, the switch on the right. Clicking
/// anywhere on the row toggles; `enabled = false` dims it and ignores clicks.
#[allow(clippy::too_many_arguments)]
pub fn switch(
	id: impl Into<ElementId>,
	label: &str,
	description: Option<&str>,
	on: bool,
	enabled: bool,
	cx: &mut Context<Serein>,
	toggle: impl Fn(&mut Serein, bool, &mut Window, &mut Context<Serein>) + 'static,
) -> Stateful<Div> {
	let p = palette();
	let mut fill = if on { color(p.accent) } else { color(p.base) };
	if !enabled {
		fill.a *= 0.4;
	}
	div()
		.id(id)
		.w_full()
		.py_2()
		.flex()
		.items_start()
		.gap_4()
		.when(enabled, |d| {
			d.cursor_pointer()
				.focusable()
				.tab_stop(true)
				.on_click(cx.listener(move |this, _, window, cx| {
					toggle(this, !on, window, cx);
					cx.notify();
				}))
		})
		.when(!enabled, |d| d.opacity(0.6))
		.child(
			div()
				.flex_1()
				.min_w_0()
				.flex()
				.flex_col()
				.gap_1()
				.child(title(label))
				.children(description.map(detail)),
		)
		.child(
			div()
				.flex_none()
				.w(px(40.))
				.h(px(24.))
				.p(px(3.))
				.rounded_full()
				.bg(fill)
				.border_1()
				.border_color(if on { fill } else { color(p.border) })
				.flex()
				.items_center()
				.when(on, |d| d.justify_end())
				.child(div().size(px(16.)).rounded_full().bg(white())),
		)
}

/// Settings row: title and optional detail on the left, `control` on the right.
pub fn row(label: &str, description: Option<&str>, control: impl IntoElement) -> Div {
	div()
		.w_full()
		.py_1()
		.flex()
		.items_center()
		.gap_3()
		.child(
			div()
				.flex_1()
				.min_w_0()
				.flex()
				.flex_col()
				.gap(px(2.))
				.child(
					div()
						.text_size(px(15.))
						.font_weight(FontWeight::MEDIUM)
						.text_color(color(palette().text_strong))
						.child(label.to_owned()),
				)
				.children(description.map(detail)),
		)
		.child(
			div()
				.flex_none()
				.flex()
				.items_center()
				.gap_2()
				.child(control),
		)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
	Primary,
	Danger,
	Outline,
}

/// Compact inline button, 38 px tall like `design::button`; add `on_click` at the call site.
pub fn button(
	id: impl Into<ElementId>,
	label: &str,
	kind: ButtonKind,
	enabled: bool,
) -> Stateful<Div> {
	let p = palette();
	let (fill, text) = match kind {
		ButtonKind::Primary => (color(p.accent), color(p.accent_text)),
		ButtonKind::Danger => (color(p.danger), white().into()),
		ButtonKind::Outline => (transparent_black().into(), color(p.text_strong)),
	};
	div()
		.id(id)
		.h(px(38.))
		.min_w(px(92.))
		.px_4()
		.rounded(px(8.))
		.bg(fill)
		.when(kind == ButtonKind::Outline, |d| {
			d.border_1().border_color(color(p.border))
		})
		.flex()
		.items_center()
		.justify_center()
		.text_size(px(14.))
		.font_weight(FontWeight::MEDIUM)
		.text_color(text)
		.when(enabled, |d| {
			d.cursor_pointer()
				.focusable()
				.tab_stop(true)
				.hover(move |d| match kind {
					ButtonKind::Outline => d.bg(color(p.hover)),
					_ => d.opacity(0.9),
				})
		})
		.when(!enabled, |d| d.opacity(0.4))
		.child(label.to_owned())
}

/// Quiet inline verb such as "Reset": muted text that brightens on hover.
pub fn text_action(id: impl Into<ElementId>, label: &str) -> Stateful<Div> {
	let p = palette();
	div()
		.id(id)
		.px(px(6.))
		.py(px(6.))
		.rounded(px(6.))
		.cursor_pointer()
		.text_size(px(13.))
		.font_weight(FontWeight::MEDIUM)
		.text_color(color(p.muted))
		.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
		.child(label.to_owned())
}

/// Eyebrow heading with a quiet reset on the right, reachable above a tall card.
pub fn header_with_reset(
	id: impl Into<ElementId>,
	heading: &str,
	reset: &str,
	cx: &mut Context<Serein>,
	on_reset: impl Fn(&mut Serein, &mut Context<Serein>) + 'static,
) -> Div {
	div()
		.w_full()
		.pt_1()
		.flex()
		.items_center()
		.justify_between()
		.child(eyebrow(heading))
		.child(
			text_action(id, reset).on_click(cx.listener(move |this, _, _, cx| {
				on_reset(this, cx);
				cx.notify();
			})),
		)
}

/// Titled [`slider`] with an optional explanation.
#[allow(clippy::too_many_arguments)]
pub fn slider_row(
	id: &'static str,
	label: &str,
	description: Option<&str>,
	value: u16,
	range: RangeInclusive<u16>,
	suffix: &'static str,
	cx: &mut Context<Serein>,
	set: impl Fn(&mut Serein, u16, &mut Context<Serein>) + 'static,
) -> Div {
	div()
		.w_full()
		.py_1()
		.flex()
		.flex_col()
		.gap_1()
		.child(
			div()
				.text_size(px(15.))
				.font_weight(FontWeight::MEDIUM)
				.text_color(color(palette().text_strong))
				.child(label.to_owned()),
		)
		.children(description.map(detail))
		.child(slider(id, value, range, suffix, cx, set))
}

/// Accent track with a round knob and the value on the right. Click or drag anywhere on the
/// track; while focused, Left/Right step by one and Shift steps by ten.
pub fn slider(
	id: &'static str,
	value: u16,
	range: RangeInclusive<u16>,
	suffix: &'static str,
	cx: &mut Context<Serein>,
	set: impl Fn(&mut Serein, u16, &mut Context<Serein>) + 'static,
) -> impl IntoElement {
	let p = palette();
	let (low, high) = (*range.start(), *range.end());
	let value = value.clamp(low, high);
	let fraction = f32::from(value - low) / f32::from((high - low).max(1));
	let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
	let set = Rc::new(set);
	let pick = {
		let bounds = bounds.clone();
		move |x: Pixels| {
			let track = bounds.get();
			let width = f32::from(track.size.width).max(1.);
			let t = (f32::from(x - track.origin.x) / width).clamp(0., 1.);
			low + (t * f32::from(high - low)).round() as u16
		}
	};
	let view = cx.entity().downgrade();
	let track = div()
		.id(id)
		.flex_1()
		.h(px(24.))
		.relative()
		.cursor_pointer()
		.focusable()
		.tab_stop(true)
		.child({
			let bounds = bounds.clone();
			let pick = pick.clone();
			let set = set.clone();
			canvas(
				move |b, _, _| bounds.set(b),
				move |_, _, window, _| {
					// Keep following the pointer after it leaves the track, until release.
					let moves = (view.clone(), pick.clone(), set.clone());
					window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
						let (view, pick, set) = &moves;
						if phase == DispatchPhase::Bubble
							&& DRAGGING.get() == Some(id)
							&& event.pressed_button == Some(MouseButton::Left)
						{
							let value = pick(event.position.x);
							let _ = view.update(cx, |this, cx| {
								set(this, value, cx);
								cx.notify();
							});
						}
					});
					window.on_mouse_event(move |_: &MouseUpEvent, _, _, _| {
						if DRAGGING.get() == Some(id) {
							DRAGGING.set(None);
						}
					});
				},
			)
			.absolute()
			.size_full()
		})
		.child(
			div()
				.absolute()
				.left_0()
				.right_0()
				.top(px(9.))
				.h(px(6.))
				.rounded_full()
				.bg(color(p.base)),
		)
		.child(
			div()
				.absolute()
				.left_0()
				.top(px(9.))
				.h(px(6.))
				.w(relative(fraction))
				.rounded_full()
				.bg(color(p.accent)),
		)
		.child(
			div()
				.absolute()
				.top(px(3.))
				.left(relative(fraction))
				.ml(px(-9.))
				.size(px(18.))
				.rounded_full()
				.bg(white())
				.shadow_sm(),
		)
		.on_mouse_down(MouseButton::Left, {
			let set = set.clone();
			cx.listener(move |this, event: &MouseDownEvent, _, cx| {
				DRAGGING.set(Some(id));
				set(this, pick(event.position.x), cx);
				cx.notify();
			})
		})
		.on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
			let step = if event.keystroke.modifiers.shift {
				10
			} else {
				1
			};
			let next = match event.keystroke.key.as_str() {
				"left" | "down" => value.saturating_sub(step).max(low),
				"right" | "up" => value.saturating_add(step).min(high),
				"home" => low,
				"end" => high,
				_ => return,
			};
			cx.stop_propagation();
			set(this, next, cx);
			cx.notify();
		}));
	div()
		.w_full()
		.pt_1()
		.flex()
		.items_center()
		.gap_4()
		.child(track)
		.child(
			div()
				.w(px(56.))
				.flex_none()
				.flex()
				.justify_end()
				.text_size(px(14.))
				.font_weight(FontWeight::MEDIUM)
				.text_color(color(p.text_strong))
				.child(format!("{value}{suffix}")),
		)
}

/// Centered icon, title and explanation for pages this frontend does not offer.
pub fn empty_state(glyph: Icon, heading: &str, explanation: &str) -> Div {
	let p = palette();
	div()
		.w_full()
		.py_10()
		.flex()
		.flex_col()
		.items_center()
		.gap_2()
		.child(icon(glyph, px(40.), color(p.muted)))
		.child(
			div()
				.text_size(px(16.))
				.font_weight(FontWeight::SEMIBOLD)
				.text_color(color(p.text_strong))
				.child(heading.to_owned()),
		)
		.child(
			div()
				.max_w(px(420.))
				.text_center()
				.text_size(px(13.))
				.line_height(px(18.))
				.text_color(color(p.muted))
				.child(explanation.to_owned()),
		)
}
