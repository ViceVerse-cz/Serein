//! The `resvg` rasteriser behind the UI's on-demand Unicode emoji.
use eframe::egui;

/// Rasterises bundled Twemoji on low-priority worker threads.
pub(crate) fn backend() -> ui::emoji::Backend {
	ui::emoji::Backend {
		rasterize: std::sync::Arc::new(rasterize),
		on_worker_start: platform::thread::lower_priority,
	}
}

/// Only trusted bundled path/shape SVGs: image references are never resolved.
fn rasterize(source: &[u8], edge: u32) -> Option<egui::ColorImage> {
	if !(ui::emoji::MIN_EDGE..=ui::emoji::MAX_EDGE).contains(&edge) {
		return None;
	}
	let svg = ui::emoji::decode_bundled_svg(source)?;
	let options = resvg::usvg::Options {
		image_href_resolver: resvg::usvg::ImageHrefResolver {
			resolve_data: Box::new(|_, _, _| None),
			resolve_string: Box::new(|_, _| None),
		},
		..Default::default()
	};
	let tree = resvg::usvg::Tree::from_data(&svg, &options).ok()?;
	let mut pixels = resvg::tiny_skia::Pixmap::new(edge, edge)?;
	let padding = edge as f32 / 32.0;
	let scale = (edge as f32 - padding * 2.0) / tree.size().width().max(tree.size().height());
	let transform = resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, padding, padding);
	resvg::render(&tree, transform, &mut pixels.as_mut());
	Some(egui::ColorImage::new(
		[edge as usize, edge as usize],
		pixels
			.data()
			.as_chunks::<4>()
			.0
			.iter()
			.map(|p| egui::Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
			.collect(),
	))
}

#[cfg(test)]
mod tests {
	#[test]
	fn bundled_artwork_rasterises_only_at_bounded_edges() {
		let (source, _) = ui::emoji::bundled_svg("emoji-unicode-0-37").unwrap();
		let image = super::rasterize(source, 37).unwrap();
		assert_eq!(image.resolution(), [37, 37]);
		assert!(image.pixels().iter().any(|pixel| pixel.a() > 0));
		assert!(super::rasterize(source, 4096).is_none());
		assert!(super::rasterize(b"not compressed svg", 37).is_none());
	}
}
