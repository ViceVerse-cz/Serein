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
	let (mut stops, mut len) = ([0; 3], 0);
	if let Some(colors) = colors.filter(|colors| colors.secondary.is_some() && colors.valid()) {
		for stop in [Some(colors.primary), colors.secondary, colors.tertiary]
			.into_iter()
			.flatten()
		{
			stops[len] = stop;
			len += 1;
		}
	}
	let primary = colors.map(|colors| colors.primary);
	let stops = (len >= 2).then_some(&stops[..len]);
	gradient(ui, text, font, primary, stops, background, fallback, width)
}

/// A name filled with `primary`, or with a left-to-right gradient through two or more `stops`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn gradient(
	ui: &Ui,
	text: &str,
	font: FontId,
	primary: Option<u32>,
	stops: Option<&[u32]>,
	background: Color32,
	fallback: Color32,
	width: f32,
) -> Arc<Galley> {
	let primary = primary.map_or(fallback, |primary| {
		crate::design::role_name_color(primary, background, fallback)
	});
	let mut job = LayoutJob::simple(text.to_owned(), font, primary, width.max(0.0));
	job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
	let mut galley = ui.painter().layout_job(job);
	let Some(stops) = stops.filter(|stops| stops.len() >= 2) else {
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
			let vertices = &mut row.visuals.mesh.vertices[start..start + 4];
			for index in 0..vertices.len() {
				let x = vertices[index].pos.x;
				// A glyph's top/bottom vertices share x; reuse their exact adjusted color.
				let color = vertices[..index]
					.iter()
					.find(|vertex| vertex.pos.x == x)
					.map(|vertex| vertex.color)
					.unwrap_or_else(|| {
						let at = ((placed.pos.x + x - bounds.left()) / bounds.width().max(1.0))
							.clamp(0.0, 1.0);
						crate::design::role_name_color(interpolate(stops, at), background, fallback)
					});
				vertices[index].color = color;
			}
		}
	}
	galley
}

/// Evenly spaced `stops`; `at` runs from 0 at the first to 1 at the last.
fn interpolate(stops: &[u32], at: f32) -> u32 {
	let span = at.clamp(0.0, 1.0) * (stops.len() - 1) as f32;
	let segment = (span as usize).min(stops.len() - 2);
	let (from, to, at) = (stops[segment], stops[segment + 1], span - segment as f32);
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
