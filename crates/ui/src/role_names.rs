//! Static role gradients painted on shaped glyphs, without animation or extra textures.
use egui::{Color32, FontId, Galley, Ui, text::LayoutJob};
use model::server_roles::Colors;
use std::sync::Arc;

pub(crate) fn galley(
	ui: &Ui,
	text: &str,
	font: FontId,
	colors: Option<Colors>,
	background: Color32,
	fallback: Color32,
	width: f32,
) -> Arc<Galley> {
	let primary = colors.map_or(fallback, |colors| {
		crate::design::role_name_color(colors.primary, background, fallback)
	});
	let mut job = LayoutJob::simple(text.to_owned(), font, primary, width.max(0.0));
	job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
	let mut galley = ui.painter().layout_job(job);
	let Some(colors) = colors.filter(|colors| colors.secondary.is_some() && colors.valid()) else {
		return galley;
	};
	// Leave the font cache untouched; shaping, elision and accessibility retain the original text.
	let galley_mut = Arc::make_mut(&mut galley);
	let bounds = galley_mut.mesh_bounds;
	for placed in &mut galley_mut.rows {
		let row = Arc::make_mut(&mut placed.row);
		for glyph in &row.glyphs {
			if glyph.is_color || glyph.uv_rect.is_nothing() {
				continue;
			}
			let start = glyph.first_vertex as usize;
			for vertex in &mut row.visuals.mesh.vertices[start..start + 4] {
				let at = ((placed.pos.x + vertex.pos.x - bounds.left()) / bounds.width().max(1.0))
					.clamp(0.0, 1.0);
				vertex.color =
					crate::design::role_name_color(interpolate(colors, at), background, fallback);
			}
		}
	}
	galley
}

fn interpolate(colors: Colors, at: f32) -> u32 {
	let secondary = colors.secondary.unwrap_or(colors.primary);
	let (from, to, at) = if let Some(tertiary) = colors.tertiary {
		if at < 0.5 {
			(colors.primary, secondary, at * 2.0)
		} else {
			(secondary, tertiary, (at - 0.5) * 2.0)
		}
	} else {
		(colors.primary, secondary, at)
	};
	let channel = |shift: u32| {
		let a = ((from >> shift) & 255) as f32;
		let b = ((to >> shift) & 255) as f32;
		(a + (b - a) * at).round() as u32
	};
	(channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gradient_stops_and_holographic_midpoint() {
		let mut colors = Colors {
			primary: 0xff0000,
			secondary: Some(0x0000ff),
			tertiary: None,
		};
		assert_eq!(interpolate(colors, 0.0), 0xff0000);
		assert_eq!(interpolate(colors, 0.5), 0x800080);
		assert_eq!(interpolate(colors, 1.0), 0x0000ff);
		colors.tertiary = Some(0x00ff00);
		assert_eq!(interpolate(colors, 0.5), 0x0000ff);
		assert_eq!(interpolate(colors, 1.0), 0x00ff00);
	}

	#[test]
	fn gradient_preserves_layout_and_cached_solid_galley() {
		let ctx = egui::Context::default();
		ctx.run_ui(Default::default(), |ui| {
			for text in ["W", "", "Long synthetic username e\u{301} مرحبا", "👨‍👩‍👧"]
			{
				let colors = Colors {
					primary: 0xe78284,
					secondary: Some(0x89b4fa),
					tertiary: None,
				};
				let make = |colors| {
					galley(
						ui,
						text,
						FontId::proportional(15.0),
						colors,
						Color32::BLACK,
						Color32::WHITE,
						100.0,
					)
				};
				let solid = make(Some(Colors {
					primary: colors.primary,
					..Default::default()
				}));
				let gradient = make(Some(colors));
				assert_eq!(solid.rect, gradient.rect);
				assert_eq!(solid.elided, gradient.elided);
				assert_eq!(solid.job.text, gradient.job.text);
				assert_eq!(
					solid,
					make(Some(Colors {
						primary: colors.primary,
						..Default::default()
					}))
				);
				if text == "W" {
					let vertices = &gradient.rows[0].visuals.mesh.vertices;
					assert!(
						vertices
							.windows(2)
							.any(|pair| pair[0].color != pair[1].color)
					);
				}
			}
		})
		.drop_without_applying_deltas();
	}

	#[test]
	fn gradient_vertices_keep_contrast_in_light_and_dark_modes() {
		let ctx = egui::Context::default();
		for (background, fallback) in [
			(Color32::WHITE, Color32::BLACK),
			(Color32::BLACK, Color32::WHITE),
		] {
			ctx.run_ui(Default::default(), |ui| {
				let text = galley(
					ui,
					"Synthetic gradient role",
					FontId::proportional(15.0),
					Some(Colors {
						primary: 0xff0000,
						secondary: Some(0x00ff00),
						tertiary: Some(0x0000ff),
					}),
					background,
					fallback,
					240.0,
				);
				for row in &text.rows {
					for vertex in &row.visuals.mesh.vertices {
						let a = crate::design::luminance(vertex.color) + 0.05;
						let b = crate::design::luminance(background) + 0.05;
						assert!(a.max(b) / a.min(b) >= 4.5);
					}
				}
			})
			.drop_without_applying_deltas();
		}
	}
}
