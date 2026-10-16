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
	let Some(style) = style else {
		let galley = crate::role_names::galley(ui, text, font, colors, background, fallback, width);
		return ui.add(
			egui::Label::new(galley)
				.truncate()
				.selectable(false)
				.sense(sense),
		);
	};
	let backdrop = ui.painter().add(Shape::Noop);
	let row = ui.fonts_mut(|fonts| fonts.row_height(&font));
	let font = crate::fonts::name_family(ui.ctx(), style.font)
		.map_or(font.clone(), |family| egui::FontId::new(font.size, family));
	// Colours never change the layout, so the plain galley sizes the allocation.
	let plain =
		crate::role_names::galley(ui, text, font.clone(), colors, background, fallback, width);
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
		)
	} else {
		plain
	};
	let pos = egui::pos2(rect.left(), rect.center().y - galley.size().y * 0.5);
	if styled {
		paint_backdrop(ui, backdrop, style, &galley, pos);
	}
	ui.painter().galley(pos, galley.clone(), fallback);
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

#[cfg(test)]
mod tests {
	use super::*;

	fn style(effect: NameEffect, colors: &[u32]) -> NameStyle {
		NameStyle {
			font: 0,
			effect,
			colors: colors.to_vec(),
		}
	}

	#[test]
	fn gradients_use_every_colour_and_solid_effects_only_the_first() {
		let four = [0x112233, 0x445566, 0x778899, 0xaabbcc];
		assert_eq!(stops(&style(NameEffect::Gradient, &four)), Some(&four[..]));
		assert_eq!(stops(&style(NameEffect::Gradient, &four[..1])), None);
		assert_eq!(stops(&style(NameEffect::Neon, &four[..2])), None);
	}

	#[test]
	fn effects_show_on_name_or_row_hover_or_always() {
		let shapes = |pointer: Option<egui::Pos2>, always: bool, row: bool| {
			let ctx = egui::Context::default();
			let mut count = 0;
			for _ in 0..2 {
				let mut input = egui::RawInput::default();
				if let Some(pos) = pointer {
					input.events.push(egui::Event::PointerMoved(pos));
				}
				let mut output = ctx.run_ui(input, |ui| {
					in_row(row, || {
						label(
							ui,
							Some(&style(NameEffect::Neon, &[0xff0000])),
							"Synthetic",
							egui::FontId::proportional(14.0),
							None,
							Color32::BLACK,
							Color32::WHITE,
							egui::Sense::hover(),
							always,
						)
					});
				});
				fn texts(shape: &Shape) -> usize {
					match shape {
						Shape::Text(_) => 1,
						Shape::Vec(shapes) => shapes.iter().map(texts).sum(),
						_ => 0,
					}
				}
				count = output
					.shapes
					.iter()
					.map(|clipped| texts(&clipped.shape))
					.sum();
				output.textures_delta.clear();
			}
			count
		};
		let plain = shapes(None, false, false);
		assert!(shapes(Some(egui::pos2(12.0, 8.0)), false, false) > plain);
		assert!(
			shapes(None, false, true) > plain,
			"hovering the row shows the effect"
		);
		assert!(shapes(None, true, false) > plain);
	}

	#[test]
	fn names_are_unstyled_unless_switched_on() {
		let mut user = test_support::message(1, model::Id(2)).author;
		user.cosmetics = Some(Box::new(model::Cosmetics {
			name_style: Some(style(NameEffect::Pop, &[0xff0000])),
			..Default::default()
		}));
		set_enabled(false);
		assert!(super::style(&user).is_none());
		set_enabled(true);
		assert!(super::style(&user).is_some());
		set_enabled(false);
	}
}
