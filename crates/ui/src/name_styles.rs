//! Nitro display name styles: the account's colours and effect drawn on a name, opt-in like
//! the other profile cosmetics. Bundled OFL faces stand in for Discord's name fonts.
use egui::{Color32, Galley, Pos2, Shape, Ui, vec2};
use model::{NameEffect, NameStyle, server_roles::Colors};
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
};

/// Set from Settings › Appearance › Profile cosmetics each frame.
static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
	static ROW_HOVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Runs `add` as part of a row (member, message, search hit) the pointer is over, so a styled
/// name inside it shows its colours and effect for the whole row, like Discord's.
pub(crate) fn in_row<R>(hovered: bool, add: impl FnOnce() -> R) -> R {
	let before = ROW_HOVER.replace(hovered);
	let result = add();
	ROW_HOVER.set(before);
	result
}

pub(crate) fn set_enabled(enabled: bool) {
	ENABLED.store(enabled, Ordering::Relaxed);
}

/// The style to draw for `user`, when name styles are switched on.
pub(crate) fn style(user: &model::User) -> Option<&NameStyle> {
	ENABLED
		.load(Ordering::Relaxed)
		.then_some(())
		.and(user.cosmetics.as_deref())
		.and_then(|cosmetics| cosmetics.name_style.as_ref())
		.filter(|style| style.valid())
}

/// A display name: plain names are ordinary labels; styled names use the style's font. Its
/// colours and effect show while the name or its row ([`in_row`]) is hovered, or always when
/// `always` is set (the account panel and profile cards). A styled font keeps `font`'s size but takes only `font`'s
/// row height, so a tall face overflows its line like Discord's instead of pushing rows apart.
#[allow(clippy::too_many_arguments)]
pub(crate) fn label(
	ui: &mut Ui,
	style: Option<&NameStyle>,
	text: &str,
	font: egui::FontId,
	colors: Option<Colors>,
	background: Color32,
	fallback: Color32,
	sense: egui::Sense,
	always: bool,
) -> egui::Response {
	let width = ui.available_width();
	// Twemoji art replaces the font's monochrome emoji (flags show as boxed letters), as in chat.
	let atlas = if text.is_ascii() {
		None
	} else {
		crate::emoji::atlas(ui.ctx())
	};
	let spans = atlas.map_or_else(Vec::new, |_| crate::emoji::name_spans(text));
	let Some(style) = style else {
		let size = ui.fonts_mut(|fonts| fonts.row_height(&font));
		let galley = crate::role_names::galley_with_emoji(
			ui, text, font, colors, background, fallback, width, &spans,
		);
		let (Some(atlas), false) = (atlas, spans.is_empty()) else {
			return ui.add(
				egui::Label::new(galley)
					.truncate()
					.selectable(false)
					.sense(sense),
			);
		};
		// `Label::ui` with the emoji painted over their slots.
		let interactive = sense != egui::Sense::hover();
		let (pos, galley, mut response) = egui::Label::new(galley)
			.truncate()
			.selectable(false)
			.sense(sense)
			.layout_in_ui(ui);
		response
			.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), text));
		if ui.is_rect_visible(response.rect) {
			if galley.elided {
				response = response.on_hover_text(text);
			}
			let color = if interactive {
				ui.style().interact(&response).text_color()
			} else {
				ui.visuals().text_color()
			};
			let underline = if response.has_focus() || response.highlighted() {
				egui::Stroke::new(1.0, color)
			} else {
				egui::Stroke::NONE
			};
			ui.painter().add(
				egui::epaint::TextShape::new(pos, galley.clone(), color).with_underline(underline),
			);
			crate::emoji::paint_name_slots(ui, atlas, &galley, pos, text, &spans, size);
		}
		return response;
	};
	let backdrop = ui.painter().add(Shape::Noop);
	let row = ui.fonts_mut(|fonts| fonts.row_height(&font));
	let font = crate::fonts::name_family(ui.ctx(), style.font)
		.map_or(font.clone(), |family| egui::FontId::new(font.size, family));
	// Emoji slots are as tall as the font laid out, which may differ from `row`.
	let size = ui.fonts_mut(|fonts| fonts.row_height(&font));
	// Colours never change the layout, so the plain galley sizes the allocation.
	let plain = crate::role_names::galley_with_emoji(
		ui,
		text,
		font.clone(),
		colors,
		background,
		fallback,
		width,
		&spans,
	);
	let (rect, mut response) = ui.allocate_exact_size(vec2(plain.size().x, row), sense);
	// `contains_pointer` ignores the row or message widget that sits over the name.
	let styled = always || ROW_HOVER.get() || response.contains_pointer();
	let galley = if styled {
		crate::role_names::gradient(
			ui,
			text,
			font.clone(),
			style.colors.first().copied(),
			stops(style),
			background,
			fallback,
			width,
			&spans,
		)
	} else {
		plain
	};
	let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
	if styled {
		paint_backdrop(ui, backdrop, style, &galley, pos);
	}
	ui.painter().galley(pos, galley.clone(), fallback);
	if let (Some(atlas), false) = (atlas, spans.is_empty()) {
		crate::emoji::paint_name_slots(ui, atlas, &galley, pos, text, &spans, size);
	}
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
	if galley.elided {
		response = response.on_hover_text(text);
	}
	response
}

/// Gradient stops for a style: every colour for the gradient effects, none for solid ones.
pub(crate) fn stops(style: &NameStyle) -> Option<&[u32]> {
	match style.effect {
		NameEffect::Gradient | NameEffect::Prism | NameEffect::Gummy if style.colors.len() >= 2 => {
			Some(&style.colors)
		}
		_ => None,
	}
}

fn rgb(color: u32) -> Color32 {
	Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

/// Effect layers painted behind the name's `galley` at `pos`: a soft halo for Neon and Glow,
/// a dark outline for Toon, and a drop shadow for Pop and Gummy.
pub(crate) fn backdrop(style: &NameStyle, galley: &Arc<Galley>, pos: Pos2) -> Vec<Shape> {
	let copy = |offset: egui::Vec2, color: Color32| {
		Shape::galley_with_override_text_color(pos + offset, galley.clone(), color)
	};
	let ring = |radius: f32, color: Color32| {
		(0..8)
			.map(|step| {
				let angle = step as f32 * std::f32::consts::FRAC_PI_4;
				copy(vec2(angle.cos(), angle.sin()) * radius, color)
			})
			.collect::<Vec<_>>()
	};
	let glow = rgb(style.colors[0]);
	let shadow = rgb(style.colors.get(1).copied().unwrap_or(0));
	match style.effect {
		// Faint, so the halo lifts the name without smearing its letters.
		NameEffect::Neon => {
			let mut shapes = ring(2.0, glow.gamma_multiply(0.1));
			shapes.extend(ring(1.0, glow.gamma_multiply(0.16)));
			shapes
		}
		NameEffect::Glow => ring(1.2, glow.gamma_multiply(0.12)),
		NameEffect::Toon => {
			let mut shapes = vec![copy(vec2(0.0, 1.5), Color32::from_black_alpha(220))];
			shapes.extend(ring(1.0, Color32::from_black_alpha(220)));
			shapes
		}
		NameEffect::Pop => vec![copy(vec2(1.5, 1.5), shadow.gamma_multiply(0.9))],
		NameEffect::Gummy => vec![copy(vec2(0.0, 1.2), Color32::from_black_alpha(140))],
		NameEffect::Solid | NameEffect::Gradient | NameEffect::Prism => Vec::new(),
	}
}

/// Paints `style`'s effect layers behind the label already laid out from `galley` at `pos`.
pub(crate) fn paint_backdrop(
	ui: &Ui,
	layer: egui::layers::ShapeIdx,
	style: &NameStyle,
	galley: &Arc<Galley>,
	pos: Pos2,
) {
	ui.painter()
		.set(layer, Shape::Vec(backdrop(style, galley, pos)));
}
