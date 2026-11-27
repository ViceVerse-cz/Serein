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
	let width = image.resolution()[0] as u32;
	let height = image.resolution()[1] as u32;
	let longest = width.max(height);
	if edge == 0
		|| longest <= edge
		|| image.pixels().len() != image.resolution()[0] * image.resolution()[1]
	{
		return image.clone();
	}
	let nw = ((u64::from(width) * u64::from(edge) + u64::from(longest) / 2) / u64::from(longest))
		.max(1) as u32;
	let nh = ((u64::from(height) * u64::from(edge) + u64::from(longest) / 2) / u64::from(longest))
		.max(1) as u32;
	let Some(scaled) = downscale_rgba(width, height, image.as_raw(), nw, nh) else {
		return image.clone();
	};
	ColorImage::from_rgba_premultiplied([nw as usize, nh as usize], &scaled)
}

/// Separable integer area-average downscaler for tightly packed RGBA8 pixels.
/// Returns `None` when dimensions are invalid, upscaling is requested, or `src` length mismatches.
pub fn downscale_rgba(
	src_w: u32,
	src_h: u32,
	src: &[u8],
	dst_w: u32,
	dst_h: u32,
) -> Option<Vec<u8>> {
	let expected = (src_w as usize)
		.checked_mul(src_h as usize)?
		.checked_mul(4)?;
	if src_w == 0
		|| src_h == 0
		|| dst_w == 0
		|| dst_h == 0
		|| dst_w > src_w
		|| dst_h > src_h
		|| src.len() != expected
	{
		return None;
	}
	if dst_w == src_w && dst_h == src_h {
		return Some(src.to_vec());
	}
	if dst_w < src_w && dst_h == src_h {
		let mut out = vec![0u8; dst_w as usize * dst_h as usize * 4];
		downscale_horizontal(src_w, src_h, src, dst_w, &mut out);
		return Some(out);
	}
	if dst_w == src_w && dst_h < src_h {
		let mut out = vec![0u8; dst_w as usize * dst_h as usize * 4];
		downscale_vertical(dst_w, src_h, src, dst_h, &mut out);
		return Some(out);
	}
	let mut tmp = vec![0u8; dst_w as usize * src_h as usize * 4];
	downscale_horizontal(src_w, src_h, src, dst_w, &mut tmp);
	let mut out = vec![0u8; dst_w as usize * dst_h as usize * 4];
	downscale_vertical(dst_w, src_h, &tmp, dst_h, &mut out);
	Some(out)
}

fn axis_spans(src_len: u32, dst_len: u32) -> Vec<(usize, usize, u64, u64)> {
	let src = u64::from(src_len);
	let dst = u64::from(dst_len);
	(0..u64::from(dst_len))
		.map(|d| {
			let lo = d * src;
			let hi = (d + 1) * src;
			let start = (lo / dst) as usize;
			let end = ((hi - 1) / dst) as usize;
			let first_w = ((start as u64 + 1) * dst).min(hi) - lo;
			let last_w = if end > start {
				hi - end as u64 * dst
			} else {
				0
			};
			(start, end, first_w, last_w)
		})
		.collect()
}

#[inline]
fn div_round_u8(acc: u64, total: u32, inv: Option<u64>) -> u8 {
	let total = u64::from(total);
	let sum = acc + total / 2;
	match inv {
		Some(inv) => ((sum * inv) >> 40).min(255) as u8,
		None => (sum / total).min(255) as u8,
	}
}

fn downscale_horizontal(src_w: u32, src_h: u32, src: &[u8], dst_w: u32, dst: &mut [u8]) {
	let spans = axis_spans(src_w, dst_w);
	let inv = (src_w <= 16_384).then(|| (1u64 << 40).div_ceil(u64::from(src_w)));
	let dst_w = u64::from(dst_w);
	let src_stride = src_w as usize * 4;
	let dst_stride = dst_w as usize * 4;
	for (src_row, dst_row) in src
		.chunks_exact(src_stride)
		.zip(dst.chunks_exact_mut(dst_stride))
		.take(src_h as usize)
	{
		for (&(start, end, first_w, last_w), dst_px) in
			spans.iter().zip(dst_row.as_chunks_mut::<4>().0)
		{
			let p0 = &src_row[start * 4..][..4];
			let mut acc = [
				u64::from(p0[0]) * first_w,
				u64::from(p0[1]) * first_w,
				u64::from(p0[2]) * first_w,
				u64::from(p0[3]) * first_w,
			];
			if end > start + 1 {
				let mut mid = [0u64; 4];
				for px in src_row[(start + 1) * 4..end * 4].as_chunks::<4>().0 {
					mid[0] += u64::from(px[0]);
					mid[1] += u64::from(px[1]);
					mid[2] += u64::from(px[2]);
					mid[3] += u64::from(px[3]);
				}
				for c in 0..4 {
					acc[c] += mid[c] * dst_w;
				}
			}
			if end > start {
				let pn = &src_row[end * 4..][..4];
				for c in 0..4 {
					acc[c] += u64::from(pn[c]) * last_w;
				}
			}
			for c in 0..4 {
				dst_px[c] = div_round_u8(acc[c], src_w, inv);
			}
		}
	}
}

fn downscale_vertical(width: u32, src_h: u32, src: &[u8], dst_h: u32, dst: &mut [u8]) {
	let spans = axis_spans(src_h, dst_h);
	let inv = (src_h <= 16_384).then(|| (1u64 << 40).div_ceil(u64::from(src_h)));
	let dst_h = u64::from(dst_h);
	let row_bytes = width as usize * 4;
	let mut row_acc = vec![0u64; row_bytes];
	let mut mid_acc = vec![0u64; row_bytes];
	for (&(start, end, first_w, last_w), dst_row) in
		spans.iter().zip(dst.chunks_exact_mut(row_bytes))
	{
		let first_row = &src[start * row_bytes..][..row_bytes];
		for (acc, &px) in row_acc.iter_mut().zip(first_row) {
			*acc = u64::from(px) * first_w;
		}
		if end > start + 1 {
			mid_acc.fill(0);
			for sy in (start + 1)..end {
				let row = &src[sy * row_bytes..][..row_bytes];
				for (mid, &px) in mid_acc.iter_mut().zip(row) {
					*mid += u64::from(px);
				}
			}
			for (acc, &mid) in row_acc.iter_mut().zip(&mid_acc) {
				*acc += mid * dst_h;
			}
		}
		if end > start {
			let last_row = &src[end * row_bytes..][..row_bytes];
			for (acc, &px) in row_acc.iter_mut().zip(last_row) {
				*acc += u64::from(px) * last_w;
			}
		}
		for (out, &acc) in dst_row.iter_mut().zip(&row_acc) {
			*out = div_round_u8(acc, src_h, inv);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn downscale_rgba_averages_and_rejects_invalid_inputs() {
		assert!(downscale_rgba(0, 4, &[], 1, 1).is_none());
		assert!(downscale_rgba(2, 2, &[0; 15], 1, 1).is_none());
		assert!(downscale_rgba(2, 2, &[0; 16], 3, 2).is_none());

		let mut checker = Vec::with_capacity(4 * 4 * 4);
		for y in 0..4 {
			for x in 0..4 {
				let v = if (x + y) % 2 == 0 { 200 } else { 100 };
				checker.extend_from_slice(&[v, 40, 80, 255]);
			}
		}
		let exact = downscale_rgba(4, 4, &checker, 2, 2).unwrap();
		assert_eq!(exact.len(), 2 * 2 * 4);
		for px in exact.as_chunks::<4>().0 {
			assert_eq!(*px, [150, 40, 80, 255]);
		}

		// Non-integer ratio and 1D-only downscales preserve uniform color without rounding drift.
		let solid = [173u8, 91, 212, 255].repeat(13 * 11);
		for (dw, dh) in [(13, 5), (5, 11), (7, 5)] {
			let out = downscale_rgba(13, 11, &solid, dw, dh).unwrap();
			assert!(
				out.as_chunks::<4>()
					.0
					.iter()
					.all(|px| *px == [173, 91, 212, 255])
			);
		}
	}
}
