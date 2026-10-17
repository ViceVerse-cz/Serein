//! egui-wgpu keeps a single mip level and minifies with a 2×2 sample.
//! A texture much larger than the pixels it covers skips texels and aliases.

use egui::ColorImage;

const MAX_GPU_MINIFY: f32 = 1.45;
const RUNGS: [u32; 11] = [16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512];

fn sample_edge(physical: f32) -> u32 {
	RUNGS
		.into_iter()
		.find(|rung| *rung as f32 >= physical)
		.unwrap_or(RUNGS[RUNGS.len() - 1])
}

pub(crate) fn fitted_edge(source: u32, physical: f32) -> Option<u32> {
	if source <= 1 || !physical.is_finite() || physical <= 0.0 {
		return None;
	}
	if (source as f32) <= physical * MAX_GPU_MINIFY {
		return None;
	}
	let edge = sample_edge(physical).min(source);
	(edge < source).then_some(edge)
}

pub(crate) fn covering_edge(physical: f32, edges: &[u32]) -> u32 {
	let Some(&smallest) = edges.iter().min() else {
		return 1;
	};
	if !physical.is_finite() || physical <= 0.0 {
		return edges.iter().copied().max().unwrap_or(smallest);
	}
	let limit = physical * MAX_GPU_MINIFY;
	edges
		.iter()
		.copied()
		.filter(|edge| *edge as f32 <= limit)
		.max()
		.unwrap_or(smallest)
}

pub(crate) fn downscale(image: &ColorImage, edge: u32) -> ColorImage {
	let width = image.size[0] as u32;
	let height = image.size[1] as u32;
	let longest = width.max(height);
	if edge == 0 || longest <= edge {
		return image.clone();
	}
	let filter = if longest <= edge.saturating_mul(2) {
		image::imageops::FilterType::Triangle
	} else {
		image::imageops::FilterType::Lanczos3
	};
	downscale_filter(image, edge, filter)
}

pub(crate) fn downscale_filter(
	image: &ColorImage,
	edge: u32,
	filter: image::imageops::FilterType,
) -> ColorImage {
	let width = image.size[0] as u32;
	let height = image.size[1] as u32;
	let longest = width.max(height);
	if edge == 0 || longest <= edge || image.pixels.len() != image.size[0] * image.size[1] {
		return image.clone();
	}
	let nw = ((u64::from(width) * u64::from(edge) + u64::from(longest) / 2) / u64::from(longest))
		.max(1) as u32;
	let nh = ((u64::from(height) * u64::from(edge) + u64::from(longest) / 2) / u64::from(longest))
		.max(1) as u32;
	let mut raw = Vec::with_capacity(image.pixels.len() * 4);
	for pixel in &image.pixels {
		raw.extend(pixel.to_array());
	}
	let Some(buffer) = image::RgbaImage::from_raw(width, height, raw) else {
		return image.clone();
	};
	let scaled = image::DynamicImage::ImageRgba8(buffer)
		.resize_exact(nw, nh, filter)
		.into_rgba8();
	ColorImage::from_rgba_premultiplied(
		[scaled.width() as usize, scaled.height() as usize],
		scaled.as_raw(),
	)
}
