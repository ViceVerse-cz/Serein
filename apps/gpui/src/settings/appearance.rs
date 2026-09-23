//! Appearance: theme cards, colour presets and the primary colour, like the main app's page.
//! Window effects are left out: GPUI windows here are opaque.
use super::{format_hex, kit};
use crate::Serein;
use crate::theme::{self, Appearance, Icon, color, icon, palette};
use gpui::{prelude::*, *};
use ui::design::Variant;

/// Quick accent choices beside the hex field; `None` is the Serein default.
const ACCENTS: [(Option<[u8; 3]>, &str); 6] = [
	(None, "Serein azure"),
	(Some([0x8b, 0x5c, 0xf6]), "Violet"),
	(Some([0xd9, 0x4f, 0x9a]), "Orchid"),
	(Some([0x2f, 0xb8, 0x7a]), "Green"),
	(Some([0xe8, 0xa3, 0x3d]), "Amber"),
	(Some([0xef, 0x55, 0x61]), "Rose"),
];

impl Serein {
	pub(super) fn settings_appearance(
		&mut self,
		window: &mut Window,
		cx: &mut Context<Self>,
	) -> Div {
		div()
			.flex()
			.flex_col()
			.gap_3()
			.child(div().pt_1().child(kit::eyebrow("Theme")))
			.child(theme_cards())
			.child(kit::group("Colour preset", colour_presets()))
			.child(kit::group("Accent", self.accent_card(window, cx)))
			.child(self.settings_layout(cx))
	}

	fn accent_card(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
		let p = palette();
		let primary = ui::design::primary_color();
		let themed = ui::design::theme_sets_accent(theme::dark());
		let current = primary.unwrap_or(ui::design::DEFAULT_PRIMARY_COLOR);
		// Show the applied colour unless the field is being edited.
		let hex = self.settings.hex.clone();
		let focused = hex.read(cx).focus_handle(cx).is_focused(window);
		if !focused && hex.read(cx).value() != format_hex(current) {
			hex.update(cx, |input, cx| input.set_value(format_hex(current), cx));
		}
		let [r, g, b] = current;
		let swatches = ACCENTS.map(|(value, label)| {
			let selected = primary == value;
			let [r, g, b] = value.unwrap_or(ui::design::DEFAULT_PRIMARY_COLOR);
			div()
				.id(label)
				.size(px(28.))
				.flex_none()
				.rounded_full()
				.border_2()
				.border_color(if selected {
					color(p.text_strong)
				} else {
					transparent_black().into()
				})
				.flex()
				.items_center()
				.justify_center()
				.cursor_pointer()
				.focusable()
				.tab_stop(true)
				.tooltip(crate::tooltip(label))
				.on_click(move |_, window, _| {
					ui::design::set_primary_color(value);
					window.refresh();
				})
				.child(
					div()
						.size(px(20.))
						.rounded_full()
						.bg(color(egui::Color32::from_rgb(r, g, b)))
						.flex()
						.items_center()
						.justify_center()
						.when(selected, |d| d.child(icon(Icon::Check, px(12.), white()))),
				)
		});
		kit::card()
			.when(themed, |d| d.opacity(0.6))
			.child(kit::row(
				"Primary color",
				Some(if themed {
					"The active theme brings its own accent; it takes over while the theme is in use."
				} else {
					"Used for buttons, selection and message highlights."
				}),
				div()
					.flex()
					.items_center()
					.gap_2()
					.when(primary.is_some(), |d| {
						d.child(kit::text_action("accent-reset", "Reset").on_click(
							|_, window, _| {
								ui::design::set_primary_color(None);
								window.refresh();
							},
						))
					})
					.child(
						div()
							.w(px(40.))
							.h(px(28.))
							.rounded(px(6.))
							.border_1()
							.border_color(color(p.border))
							.bg(color(egui::Color32::from_rgb(r, g, b))),
					)
					.child(
						div()
							.w(px(96.))
							.h(px(32.))
							.px_2()
							.rounded(px(6.))
							.bg(color(p.base))
							.border_1()
							.border_color(color(if focused { p.accent } else { p.border }))
							.flex()
							.items_center()
							.text_size(px(14.))
							.child(hex),
					),
			))
			.child(kit::divider())
			.child(div().pt_1().flex().gap(px(6.)).children(swatches))
			.child(kit::hint(
				"Type a hex colour and press Enter, or pick a quick accent.",
			))
	}
}

/// Dark, Light and Sync-with-system cards with a miniature of each palette and a radio marker.
fn theme_cards() -> Div {
	let p = palette();
	let variant = ui::design::variant();
	let current = theme::appearance();
	let cards = [
		(Appearance::Dark, "Dark"),
		(Appearance::Light, "Light"),
		(Appearance::System, "Sync with system"),
	]
	.map(|(mode, label)| {
		let selected = current == mode;
		let (left, right) = match mode {
			Appearance::Dark => {
				let colors = ui::design::colors(true, variant);
				(colors.sidebar.to_opaque(), colors.chat.to_opaque())
			}
			Appearance::Light => {
				let colors = ui::design::colors(false, variant);
				(colors.sidebar.to_opaque(), colors.chat.to_opaque())
			}
			Appearance::System => (
				ui::design::colors(true, variant).chat.to_opaque(),
				ui::design::colors(false, variant).chat.to_opaque(),
			),
		};
		div()
			.id(label)
			.flex_1()
			.min_w(px(88.))
			.h(px(76.))
			.p_3()
			.relative()
			.rounded(px(8.))
			.bg(color(p.raised))
			.border(px(if selected { 2. } else { 1. }))
			.border_color(color(if selected { p.accent } else { p.border }))
			.cursor_pointer()
			.focusable()
			.tab_stop(true)
			.hover(|d| d.bg(color(p.hover)))
			.on_click(move |_, window, _| {
				theme::set_appearance(mode);
				window.refresh();
			})
			.flex()
			.flex_col()
			.justify_between()
			.child(
				div()
					.w(px(52.))
					.h(px(30.))
					.rounded(px(6.))
					.overflow_hidden()
					.border_1()
					.border_color(color(p.border))
					.flex()
					.child(div().w(relative(0.42)).h_full().bg(color(left)))
					.child(div().flex_1().h_full().bg(color(right))),
			)
			.child(
				div()
					.text_size(px(14.))
					.font_weight(FontWeight::MEDIUM)
					.text_color(color(if selected { p.text_strong } else { p.text }))
					.child(label),
			)
			.child(
				div()
					.absolute()
					.top(px(12.))
					.right(px(12.))
					.size(px(16.))
					.rounded_full()
					.border_2()
					.border_color(color(if selected { p.accent } else { p.muted }))
					.flex()
					.items_center()
					.justify_center()
					.when(selected, |d| {
						d.child(div().size(px(8.)).rounded_full().bg(color(p.accent)))
					}),
			)
	});
	div().w_full().flex().gap(px(10.)).children(cards)
}

/// The built-in presets, one swatch each; community themes stay in the main app.
fn colour_presets() -> Div {
	let p = palette();
	let dark = theme::dark();
	let current = ui::design::variant();
	let cells = Variant::ALL.map(|variant| {
		let selected = variant == current;
		let swatch = ui::design::builtin_colors(dark, variant);
		let circle = div()
			.size(px(40.))
			.relative()
			.rounded_full()
			.overflow_hidden()
			.border(px(if selected { 2.5 } else { 1. }))
			.border_color(color(if selected { p.accent } else { p.border }));
		let circle = match swatch.backdrop {
			Some([top, bottom]) => circle.bg(color(bottom)).child(
				div()
					.absolute()
					.left(px(4.))
					.top(px(4.))
					.size(px(22.))
					.rounded_full()
					.bg(color(top)),
			),
			None => circle.bg(color(swatch.chat)).child(
				div()
					.absolute()
					.right(px(6.))
					.bottom(px(6.))
					.size(px(18.))
					.rounded_full()
					.bg(color(swatch.base)),
			),
		};
		let circle = circle.when(selected, |d| {
			d.flex().items_center().justify_center().child(
				div()
					.size(px(20.))
					.rounded_full()
					.bg(color(p.accent))
					.flex()
					.items_center()
					.justify_center()
					.child(icon(Icon::Check, px(12.), color(p.accent_text))),
			)
		});
		div()
			.id(variant.key())
			.w(px(76.))
			.h(px(70.))
			.pt_1()
			.rounded(px(6.))
			.flex()
			.flex_col()
			.items_center()
			.justify_between()
			.pb_1()
			.cursor_pointer()
			.focusable()
			.tab_stop(true)
			.hover(|d| d.bg(color(p.hover)))
			.tooltip(crate::tooltip(variant.label()))
			.on_click(move |_, window, _| {
				ui::design::set_variant(variant);
				window.refresh();
			})
			.child(circle)
			.child(
				div()
					.text_size(px(11.))
					.text_color(color(if selected { p.text_strong } else { p.muted }))
					.child(variant.label()),
			)
	});
	kit::card()
		.child(div().flex().flex_wrap().gap_3().children(cells))
		.child(kit::hint(&format!(
			"{} · saved with your appearance. Gradient presets always use dark text.",
			current.label()
		)))
}
