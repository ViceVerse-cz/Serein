//! ThumbHash decoder (Evan Wallace's format, as shipped in Discord's `placeholder` fields).
//! Decodes a few DCT coefficients into a ≤32×32 image; one decode per hash, then cached.
use egui::{Color32, ColorImage};

/// Decode a ThumbHash into a small RGBA image. Returns `None` for malformed or truncated input.
pub fn decode(hash: &[u8]) -> Option<ColorImage> {
	if hash.len() < 5 {
		return None;
	}
	let header24 = u32::from(hash[0]) | u32::from(hash[1]) << 8 | u32::from(hash[2]) << 16;
	let header16 = u32::from(hash[3]) | u32::from(hash[4]) << 8;
	let l_dc = (header24 & 63) as f32 / 63.0;
	let p_dc = ((header24 >> 6) & 63) as f32 / 31.5 - 1.0;
	let q_dc = ((header24 >> 12) & 63) as f32 / 31.5 - 1.0;
	let l_scale = ((header24 >> 18) & 31) as f32 / 31.0;
	let has_alpha = header24 >> 23 != 0;
	let p_scale = ((header16 >> 3) & 63) as f32 / 63.0;
	let q_scale = ((header16 >> 9) & 63) as f32 / 63.0;
	let landscape = header16 >> 15 != 0;
	let long = if has_alpha { 5 } else { 7 };
	let short = (header16 & 7) as usize;
	let (lx, ly) = if landscape {
		(long, short.max(3))
	} else {
		(short.max(3), long)
	};
	let (a_dc, a_scale) = if has_alpha {
		let byte = *hash.get(5)?;
		((byte & 15) as f32 / 15.0, (byte >> 4) as f32 / 15.0)
	} else {
		(1.0, 1.0)
	};
	let ac_start = if has_alpha { 6 } else { 5 };
	let mut ac_index = 0usize;
	let mut channel = |nx: usize, ny: usize, scale: f32| -> Option<Vec<f32>> {
		let mut ac = Vec::new();
		for cy in 0..ny {
			let mut cx = usize::from(cy == 0);
			while cx * ny < nx * (ny - cy) {
				let byte = *hash.get(ac_start + (ac_index >> 1))?;
				let nibble = (byte >> ((ac_index & 1) << 2)) & 15;
				ac.push((f32::from(nibble) / 7.5 - 1.0) * scale);
				ac_index += 1;
				cx += 1;
			}
		}
		Some(ac)
	};
	let l_ac = channel(lx, ly, l_scale)?;
	let p_ac = channel(3, 3, p_scale * 1.25)?;
	let q_ac = channel(3, 3, q_scale * 1.25)?;
	let a_ac = if has_alpha {
		channel(5, 5, a_scale)?
	} else {
		Vec::new()
	};

	let ratio = lx as f32 / ly as f32;
	let (w, h) = if ratio > 1.0 {
		(32, (32.0 / ratio).round() as usize)
	} else {
		((32.0 * ratio).round() as usize, 32)
	};
	let (w, h) = (w.max(1), h.max(1));
	let mut pixels = Vec::with_capacity(w * h);
	let cx_count = lx.max(if has_alpha { 5 } else { 3 });
	let cy_count = ly.max(if has_alpha { 5 } else { 3 });
	let mut fx = [0f32; 7];
	let mut fy = [0f32; 7];
	for y in 0..h {
		for (cy, value) in fy.iter_mut().enumerate().take(cy_count) {
			*value = (std::f32::consts::PI / h as f32 * (y as f32 + 0.5) * cy as f32).cos();
		}
		for x in 0..w {
			for (cx, value) in fx.iter_mut().enumerate().take(cx_count) {
				*value = (std::f32::consts::PI / w as f32 * (x as f32 + 0.5) * cx as f32).cos();
			}
			let (mut l, mut p, mut q, mut a) = (l_dc, p_dc, q_dc, a_dc);
			let mut j = 0;
			for (cy, fy) in fy.iter().enumerate().take(ly) {
				let fy2 = fy * 2.0;
				let mut cx = usize::from(cy == 0);
				while cx * ly < lx * (ly - cy) {
					l += l_ac[j] * fx[cx] * fy2;
					j += 1;
					cx += 1;
				}
			}
			let mut j = 0;
			for (cy, fy) in fy.iter().enumerate().take(3) {
				let fy2 = fy * 2.0;
				for fx in &fx[usize::from(cy == 0)..3 - cy] {
					let f = fx * fy2;
					p += p_ac[j] * f;
					q += q_ac[j] * f;
					j += 1;
				}
			}
			if has_alpha {
				let mut j = 0;
				for (cy, fy) in fy.iter().enumerate().take(5) {
					let fy2 = fy * 2.0;
					for fx in &fx[usize::from(cy == 0)..5 - cy] {
						a += a_ac[j] * fx * fy2;
						j += 1;
					}
				}
			}
			let b = l - 2.0 / 3.0 * p;
			let r = (3.0 * l - b + q) / 2.0;
			let g = r - q;
			let to_byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
			pixels.push(Color32::from_rgba_unmultiplied(
				to_byte(r),
				to_byte(g),
				to_byte(b),
				to_byte(a),
			));
		}
	}
	Some(ColorImage {
		size: [w, h],
		source_size: egui::vec2(w as f32, h as f32),
		pixels,
	})
}

#[cfg(test)]
mod tests {
	use super::decode;

	// `1QcSHQRnh493V4dIh4eXh1h4kJUI` from the reference implementation's README: a portrait
	// sunset, warm centre over a dark sea. Expected pixels come from an independent port.
	const SUNSET: [u8; 21] = [
		0xd5, 0x07, 0x12, 0x1d, 0x04, 0x67, 0x87, 0x8f, 0x77, 0x57, 0x87, 0x48, 0x87, 0x87, 0x97,
		0x87, 0x58, 0x78, 0x90, 0x95, 0x08,
	];

	#[test]
	fn decodes_reference_hash_to_portrait_image() {
		let image = decode(&SUNSET).expect("valid hash");
		assert_eq!(image.size, [23, 32]);
		assert_eq!(image.pixels.len(), 23 * 32);
		for ((x, y), expected) in [
			((0, 0), [64u8, 78, 113]),
			((22, 0), [85, 109, 139]),
			((0, 31), [0, 12, 43]),
			((22, 31), [0, 4, 40]),
			((11, 16), [140, 110, 89]),
		] {
			let pixel = image.pixels[y * 23 + x];
			assert_eq!(pixel.a(), 255);
			for (actual, expected) in [pixel.r(), pixel.g(), pixel.b()].into_iter().zip(expected) {
				assert!(
					(i16::from(actual) - i16::from(expected)).abs() <= 1,
					"pixel ({x},{y}) was {pixel:?}"
				);
			}
		}
	}

	#[test]
	fn rejects_truncated_hashes() {
		assert!(decode(&[]).is_none());
		assert!(decode(&SUNSET[..4]).is_none());
		assert!(decode(&SUNSET[..8]).is_none());
		assert!(decode(&SUNSET[..20]).is_none());

		{
			for seed in 0..512u32 {
				let bytes: Vec<u8> = (0..(seed % 40) as u8)
					.map(|i| (seed.wrapping_mul(2654435761) >> (i % 24)) as u8)
					.collect();
				let _ = decode(&bytes);
			}
		}
	}
}
