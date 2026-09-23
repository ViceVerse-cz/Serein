//! Appearance popover behind the user panel's gear: mode, colour variant and accent.
//! Choices are process-wide; `persist` saves them to the experiment's own local store.
use crate::theme::{self, Appearance, Icon, color, icon, palette, solid};
use crate::tooltip;
use gpui::{prelude::*, *};
use ui::design::Variant;

/// Width of the user panel card, so the popover sits flush above it.
const WIDTH: f32 = crate::sidebar::RAIL_WIDTH + crate::sidebar::LIST_WIDTH - 16.;
/// Quick accent choices; `None` is the Serein default.
const ACCENTS: [(Option<[u8; 3]>, &str); 6] = [
	(None, "Serein azure"),
	(Some([0x8b, 0x5c, 0xf6]), "Violet"),
	(Some([0xd9, 0x4f, 0x9a]), "Orchid"),
	(Some([0x2f, 0xb8, 0x7a]), "Green"),
	(Some([0xe8, 0xa3, 0x3d]), "Amber"),
	(Some([0xef, 0x55, 0x61]), "Rose"),
];

/// Account actions the popover hands to the app.
pub enum Event {
	LogOut,
	/// Opt-in, remembered by `persist`; macOS may ask for permission the first time.
	Notifications(bool),
}

pub struct Menu {
	demo: bool,
	notifications: bool,
	open: bool,
	focus: FocusHandle,
	/// Focus before opening, restored on close.
	restore: Option<FocusHandle>,
	/// The mouse-down that dismissed the popover, so the same press on the gear does not reopen it.
	dismissed_at: Option<Point<Pixels>>,
	_appearance: Subscription,
}

impl Menu {
	/// With `demo`, `--demo-settings` opens the popover, `--demo-light`/`--demo-dark` pick a mode
	/// and `--demo-theme=KEY` a variant (`standard`, `eclipse`, `slate`, `nightfall`, ...).
	pub fn new(demo: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
		theme::set_system_appearance(window.appearance());
		let appearance = cx.observe_window_appearance(window, |_, window, _| {
			theme::set_system_appearance(window.appearance());
			window.refresh();
		});
		let mut this = Self {
			demo,
			notifications: false,
			open: false,
			focus: cx.focus_handle(),
			restore: None,
			dismissed_at: None,
			_appearance: appearance,
		};
		if demo {
			let args = std::env::args().collect::<Vec<_>>();
			let flag = |name: &str| args.iter().any(|arg| arg == name);
			if flag("--demo-light") {
				theme::set_appearance(Appearance::Light);
			} else if flag("--demo-dark") {
				theme::set_appearance(Appearance::Dark);
			}
			if let Some(variant) = args
				.iter()
				.find_map(|arg| arg.strip_prefix("--demo-theme="))
				.and_then(Variant::from_key)
			{
				ui::design::set_variant(variant);
			}
			this.open = flag("--demo-settings");
		}
		this
	}

	pub fn notifications(&self) -> bool {
		self.notifications
	}

	/// Restores a saved opt-in without emitting [`Event::Notifications`].
	pub fn set_notifications(&mut self, on: bool, cx: &mut Context<Self>) {
		self.notifications = on;
		cx.notify();
	}

	fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		if self.open {
			self.close(window, cx);
		} else {
			self.open = true;
			self.restore = window.focused(cx);
			window.focus(&self.focus, cx);
			cx.notify();
		}
	}

	fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		self.open = false;
		if let Some(previous) = self.restore.take() {
			window.focus(&previous, cx);
		}
		cx.notify();
	}

	fn popover(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let dark = theme::dark();
		let current = ui::design::variant();
		let modes = Appearance::ALL.map(|mode| {
			let selected = theme::appearance() == mode;
			div()
				.id(mode.label())
				.focusable()
				.tab_stop(true)
				.flex_1()
				.h(px(28.))
				.flex()
				.items_center()
				.justify_center()
				.rounded(px(6.))
				.cursor_pointer()
				.text_size(px(13.))
				.font_weight(FontWeight::MEDIUM)
				.when(selected, |d| {
					d.bg(color(p.selected)).text_color(color(p.text_strong))
				})
				.when(!selected, |d| {
					d.text_color(color(p.muted))
						.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
				})
				.focus(|d| d.bg(color(p.hover)))
				.on_click(move |_, window, _| {
					theme::set_appearance(mode);
					window.refresh();
				})
				.child(mode.label())
		});
		let variants = Variant::ALL.map(|variant| {
			let selected = variant == current;
			let swatch = ui::design::builtin_colors(dark, variant);
			div()
				.id(variant.key())
				.focusable()
				.tab_stop(true)
				.h(px(32.))
				.px_2()
				.flex()
				.items_center()
				.gap(px(10.))
				.rounded(px(6.))
				.cursor_pointer()
				.when(selected, |d| d.bg(color(p.selected)))
				.when(!selected, |d| d.hover(|d| d.bg(color(p.hover))))
				.focus(|d| d.bg(color(p.hover)))
				.on_click(move |_, window, _| {
					ui::design::set_variant(variant);
					window.refresh();
				})
				.child(variant_swatch(&swatch, color(p.border)))
				.child(
					div()
						.flex_1()
						.text_size(px(14.))
						.font_weight(if selected {
							FontWeight::MEDIUM
						} else {
							FontWeight::NORMAL
						})
						.text_color(color(if selected { p.text_strong } else { p.text }))
						.child(variant.label()),
				)
				.when(selected, |d| {
					d.child(icon(Icon::Check, px(14.), color(p.accent)))
				})
		});
		let primary = ui::design::primary_color();
		let accents = ACCENTS.map(|(value, label)| {
			let selected = primary == value;
			let [r, g, b] = value.unwrap_or(ui::design::DEFAULT_PRIMARY_COLOR);
			let fill = egui::Color32::from_rgb(r, g, b);
			div()
				.id(label)
				.focusable()
				.tab_stop(true)
				.size(px(28.))
				.flex_none()
				.rounded_full()
				.border_2()
				.border_color(if selected {
					color(p.text_strong)
				} else {
					gpui::transparent_black().into()
				})
				.focus(|d| d.border_color(color(p.muted)))
				.flex()
				.items_center()
				.justify_center()
				.cursor_pointer()
				.tooltip(tooltip(label))
				.on_click(move |_, window, _| {
					ui::design::set_primary_color(value);
					window.refresh();
				})
				.child(
					div()
						.size(px(20.))
						.rounded_full()
						.bg(color(fill))
						.flex()
						.items_center()
						.justify_center()
						.when(selected, |d| {
							d.child(icon(Icon::Check, px(12.), gpui::white()))
						}),
				)
		});
		let eyebrow = |text: &'static str| {
			div()
				.px_1()
				.pt_1()
				.text_size(px(11.))
				.font_weight(FontWeight::SEMIBOLD)
				.text_color(color(p.muted))
				.child(text.to_uppercase())
		};
		div()
			.id("settings-popover")
			.track_focus(&self.focus)
			.occlude()
			.w(px(WIDTH))
			.p_2()
			.flex()
			.flex_col()
			.gap_1()
			.rounded(px(10.))
			.bg(solid(p.raised))
			.border_1()
			.border_color(color(p.border))
			.shadow_lg()
			.font_family(theme::FONT)
			.on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
				if event.keystroke.key == "escape" {
					this.close(window, cx);
					cx.stop_propagation();
				}
			}))
			.on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, window, cx| {
				this.dismissed_at = Some(event.position);
				this.close(window, cx);
			}))
			.child(eyebrow("Mode"))
			.child(
				div()
					.p(px(3.))
					.flex()
					.gap(px(2.))
					.rounded(px(8.))
					.bg(solid(p.base))
					.border_1()
					.border_color(color(p.border))
					.children(modes),
			)
			.child(eyebrow("Theme"))
			.child(div().flex().flex_col().gap(px(2.)).children(variants))
			.child(eyebrow("Accent"))
			.child(div().px_1().flex().gap(px(6.)).children(accents))
			.when(!self.demo, |d| {
				let on = self.notifications;
				d.child(div().h(px(1.)).my_1().bg(color(p.border)))
					.child(
						div()
							.id("notifications")
							.h(px(32.))
							.px_2()
							.rounded(px(6.))
							.flex()
							.items_center()
							.justify_between()
							.cursor_pointer()
							.text_size(px(14.))
							.text_color(color(p.text_strong))
							.hover(|d| d.bg(color(p.hover)))
							.on_click(cx.listener(|this, _, _, cx| {
								this.notifications = !this.notifications;
								cx.emit(Event::Notifications(this.notifications));
								cx.notify();
							}))
							.child("Desktop notifications")
							.child(
								div()
									.w(px(34.))
									.h(px(20.))
									.p(px(2.))
									.rounded_full()
									.bg(color(if on { p.accent } else { p.selected }))
									.flex()
									.when(on, |d| d.justify_end())
									.child(div().size(px(16.)).rounded_full().bg(white())),
							),
					)
					.child(
						div()
							.id("log-out")
							.h(px(32.))
							.px_2()
							.rounded(px(6.))
							.flex()
							.items_center()
							.cursor_pointer()
							.text_size(px(14.))
							.font_weight(FontWeight::MEDIUM)
							.text_color(color(p.danger))
							.hover(|d| d.bg(crate::theme::tint(p.danger, 0.12)))
							.on_click(cx.listener(|this, _, window, cx| {
								this.close(window, cx);
								cx.emit(Event::LogOut);
							}))
							.child("Log out"),
					)
			})
	}
}

impl EventEmitter<Event> for Menu {}

/// The egui preset swatch in miniature: chat colour with a base dot, or the gradient stops.
fn variant_swatch(swatch: &ui::design::Palette, border: Rgba) -> impl IntoElement {
	let circle = div()
		.size(px(20.))
		.flex_none()
		.relative()
		.rounded_full()
		.border_1()
		.border_color(border);
	match swatch.backdrop {
		Some([top, bottom]) => circle.bg(linear_gradient(
			135.,
			linear_color_stop(color(top), 0.),
			linear_color_stop(color(bottom), 1.),
		)),
		None => circle.bg(color(swatch.chat)).child(
			div()
				.absolute()
				.right(px(2.))
				.bottom(px(2.))
				.size(px(9.))
				.rounded_full()
				.bg(color(swatch.base)),
		),
	}
}

impl Render for Menu {
	fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		div()
			.relative()
			.flex_none()
			.child(
				div()
					.id("settings-gear")
					.focusable()
					.tab_stop(true)
					.size(px(32.))
					.rounded(px(6.))
					.flex()
					.items_center()
					.justify_center()
					.cursor_pointer()
					.when(self.open, |d| d.bg(color(p.hover)))
					.hover(|d| d.bg(color(p.hover)))
					.focus(|d| d.bg(color(p.hover)))
					.when(!self.open, |d| d.tooltip(tooltip("Appearance")))
					.on_mouse_down(
						MouseButton::Left,
						cx.listener(|this, event: &MouseDownEvent, window, cx| {
							if this.dismissed_at.take() != Some(event.position) {
								this.toggle(window, cx);
							}
						}),
					)
					.on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
						if event.is_keyboard() {
							this.toggle(window, cx);
						}
					}))
					.child(icon(
						Icon::Gear,
						px(20.),
						color(if self.open { p.text_strong } else { p.muted }),
					)),
			)
			.when(self.open, |d| {
				// Anchor the popover's bottom-right to the gear's top-right, aligned with the card.
				d.child(
					div().absolute().top_0().right_0().child(
						deferred(
							anchored()
								.anchor(Anchor::BottomRight)
								.offset(point(px(8.), px(-14.)))
								.snap_to_window_with_margin(px(8.))
								.child(self.popover(cx)),
						)
						.with_priority(1),
					),
				)
			})
	}
}

#[cfg(test)]
mod tests {
	use crate::theme::{self, Appearance};
	use gpui::WindowAppearance;
	use ui::design::{Variant, colors};

	#[test]
	fn appearance_and_variant_choose_the_palette() {
		theme::set_system_appearance(WindowAppearance::Light);
		theme::set_appearance(Appearance::System);
		assert!(!theme::dark());
		assert_eq!(theme::palette(), colors(false, ui::design::variant()));
		theme::set_system_appearance(WindowAppearance::VibrantDark);
		assert!(theme::dark());
		theme::set_appearance(Appearance::Light);
		assert!(!theme::dark());
		theme::set_appearance(Appearance::Dark);
		ui::design::set_variant(Variant::Eclipse);
		assert_eq!(theme::palette(), colors(true, Variant::Eclipse));
		ui::design::set_variant(Variant::Standard);
		theme::set_appearance(Appearance::System);
		assert_eq!(theme::appearance(), Appearance::System);
	}
}
