//! Static role gradients painted on shaped glyphs, without animation or extra textures.
use egui::{Color32, FontId, Galley, Ui, text::LayoutJob};
use model::server_roles::Colors;
use std::{
	ops::Range,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
};

/// Set from Settings › Appearance › Profile cosmetics each frame.
static GRADIENTS: AtomicBool = AtomicBool::new(true);

pub(crate) fn set_gradients(enabled: bool) {
	GRADIENTS.store(enabled, Ordering::Relaxed);
}

pub(crate) fn galley(
	ui: &Ui,
	text: &str,
	font: FontId,
	colors: Option<Colors>,
	background: Color32,
	fallback: Color32,
	width: f32,
) -> Arc<Galley> {
	galley_with_emoji(ui, text, font, colors, background, fallback, width, &[])
}

/// [`galley`] with blank slots in place of the emoji `spans` ([`crate::emoji::name_spans`]),
/// for the caller to paint over with [`crate::emoji::paint_name_slots`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn galley_with_emoji(
	ui: &Ui,
	text: &str,
	font: FontId,
	colors: Option<Colors>,
	background: Color32,
	fallback: Color32,
	width: f32,
	spans: &[(Range<usize>, usize)],
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
	gradient(
		ui, text, font, primary, stops, background, fallback, width, spans,
	)
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
	spans: &[(Range<usize>, usize)],
) -> Arc<Galley> {
	let primary = primary.map_or(fallback, |primary| {
		crate::design::role_name_color(primary, background, fallback)
	});
	if spans.is_empty() {
		let mut job = LayoutJob::simple(text.to_owned(), font, primary, width.max(0.0));
		job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
		return gradient_fill(ui.painter().layout_job(job), stops, background, fallback);
	}
	// Each emoji becomes one blank slot as tall as the name's row.
	let height = ui.fonts_mut(|fonts| fonts.row_height(&font));
	let slot = crate::emoji::inline_format(ui, height, height);
	let normal = egui::TextFormat::simple(font, primary);
	let mut job = slot_job(text, spans, &slot, &normal, usize::MAX);
	job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(0.0));
	let mut galley = ui.painter().layout_job(job);
	// epaint formats the `…` like the glyph it replaces, so one cut at an emoji would draw it
	// in the slot's transparent oversized format. Lay the kept text out again with a normal one.
	let ellipsis_in_slot = galley.elided
		&& galley
			.rows
			.last()
			.and_then(|row| row.glyphs.last())
			.is_some_and(|glyph| glyph.chr == '…' && glyph.line_height < glyph.font_height);
	if ellipsis_in_slot {
		// Glyphs are one per char, and the last one is the ellipsis.
		let kept = galley.rows.last().map_or(0, |row| row.glyphs.len() - 1);
		let mut job = slot_job(text, spans, &slot, &normal, kept);
		job.append("…", 0.0, normal);
		let mut cut = ui.painter().layout_job(job);
		Arc::make_mut(&mut cut).elided = true;
		galley = cut;
	}
	gradient_fill(galley, stops, background, fallback)
}

/// `text` with each emoji of `spans` replaced by a one-char `slot`, cut after `keep` chars.
fn slot_job(
	text: &str,
	spans: &[(Range<usize>, usize)],
	slot: &egui::TextFormat,
	normal: &egui::TextFormat,
	keep: usize,
) -> LayoutJob {
	let mut job = LayoutJob {
		text: String::with_capacity(text.len()),
		..Default::default()
	};
	let mut remaining = keep;
	let mut start = 0;
	let mut push = |job: &mut LayoutJob, piece: &str, format: &egui::TextFormat| {
		let end = piece
			.char_indices()
			.nth(remaining)
			.map_or(piece.len(), |(end, _)| end);
		if end > 0 {
			job.append(&piece[..end], 0.0, format.clone());
		}
		remaining -= piece[..end].chars().count();
	};
	for (range, _) in spans {
		push(&mut job, &text[start..range.start], normal);
		push(&mut job, " ", slot);
		start = range.end;
	}
	push(&mut job, &text[start..], normal);
	job
}

/// Paints `galley` with the left-to-right gradient through `stops`, when there are any.
fn gradient_fill(
	mut galley: Arc<Galley>,
	stops: Option<&[u32]>,
	background: Color32,
	fallback: Color32,
) -> Arc<Galley> {
	let Some(stops) = stops.filter(|stops| stops.len() >= 2 && GRADIENTS.load(Ordering::Relaxed))
	else {
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

	#[test]
	fn truncating_at_an_emoji_keeps_a_visible_ellipsis_and_only_kept_slots() {
		let ctx = egui::Context::default();
		crate::fonts::install(&ctx);
		let text = "Mario 🇮🇹 Rossi ⭐ Bianchi";
		let spans = crate::emoji::name_spans(text);
		assert_eq!(spans.len(), 2);
		// Each emoji is one char in the laid-out text, so its slot is kept once more chars
		// than precede it are.
		let before: Vec<_> = spans
			.iter()
			.enumerate()
			.map(|(index, (range, _))| {
				let earlier: usize = spans[..index]
					.iter()
					.map(|(range, _)| text[range.clone()].chars().count() - 1)
					.sum();
				text[..range.start].chars().count() - earlier
			})
			.collect();
		let mut cut_at_slot = false;
		ctx.run_ui(Default::default(), |ui| {
			let layout = |width: f32| {
				gradient(
					ui,
					text,
					FontId::proportional(15.0),
					Some(0xff0000),
					Some(&[0xff0000, 0x0000ff]),
					Color32::BLACK,
					Color32::WHITE,
					width,
					&spans,
				)
			};
			let full = layout(1000.0);
			assert!(!full.elided);
			for width in (20..full.size().x as usize).map(|width| width as f32) {
				let galley = layout(width);
				let glyphs = &galley.rows[0].glyphs;
				let slots = glyphs
					.iter()
					.filter(|g| g.line_height < g.font_height)
					.count();
				if !galley.elided {
					assert_eq!(slots, spans.len());
					continue;
				}
				let (ellipsis, kept) = glyphs.split_last().unwrap();
				assert_eq!(ellipsis.chr, '…', "width {width}");
				assert_eq!(ellipsis.line_height, ellipsis.font_height, "width {width}");
				let expected = before.iter().filter(|&&chars| chars < kept.len()).count();
				assert_eq!(slots, expected, "width {width}");
				cut_at_slot |= kept.last().is_some_and(|g| g.line_height < g.font_height);
			}
		})
		.drop_without_applying_deltas();
		assert!(cut_at_slot, "no width cut the name right after an emoji");
	}
}
