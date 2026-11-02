//! Read only the SPS prefix needed to enforce live picture bounds before native
//! allocation (HEVC 7.3.2.2.1). No parameter sets are rewritten before decryption.
pub(super) fn bounded_sps(nal: &[u8]) -> bool {
	parse(nal).is_some()
}
fn parse(nal: &[u8]) -> Option<()> {
	let mut rbsp = Vec::with_capacity(nal.len().min(4096));
	let mut zeros = 0;
	for &byte in nal.get(2..)?.iter().take(4096) {
		if zeros >= 2 && byte == 3 {
			zeros = 0;
			continue;
		}
		rbsp.push(byte);
		zeros = if byte == 0 { zeros + 1 } else { 0 };
	}
	let mut bits = Bits { data: &rbsp, at: 0 };
	bits.read(4)?;
	let layers = bits.read(3)? as usize;
	if layers > 6 {
		return None;
	}
	bits.read(1)?;
	bits.skip(96)?; // general_profile_tier_level
	let mut profile = [false; 7];
	let mut level = [false; 7];
	for i in 0..layers {
		profile[i] = bits.read(1)? != 0;
		level[i] = bits.read(1)? != 0;
	}
	if layers > 0 {
		bits.skip((8 - layers) * 2)?;
	}
	for i in 0..layers {
		if profile[i] {
			bits.skip(88)?;
		}
		if level[i] {
			bits.skip(8)?;
		}
	}
	if bits.ue()? > 15 {
		return None;
	}
	let chroma = bits.ue()?;
	if chroma > 3 {
		return None;
	}
	let separate = chroma == 3 && bits.read(1)? != 0;
	let width = bits.ue()?;
	let height = bits.ue()?;
	// Allow up to one 64-pixel coding-block alignment around the largest visible image.
	let max_side = platform::video::MAX_LIVE_DIMENSION.div_ceil(64) * 64;
	let max_pixels = u64::from(max_side) * (2160_u64.div_ceil(64) * 64);
	if width == 0
		|| height == 0
		|| width > max_side
		|| height > max_side
		|| u64::from(width) * u64::from(height) > max_pixels
	{
		return None;
	}
	let (mut visible_width, mut visible_height) = (width, height);
	if bits.read(1)? != 0 {
		let horizontal = bits.ue()?.checked_add(bits.ue()?)?;
		let vertical = bits.ue()?.checked_add(bits.ue()?)?;
		let unit_x = if !separate && (chroma == 1 || chroma == 2) {
			2
		} else {
			1
		};
		let unit_y = if !separate && chroma == 1 { 2 } else { 1 };
		visible_width = width.checked_sub(horizontal.checked_mul(unit_x)?)?;
		visible_height = height.checked_sub(vertical.checked_mul(unit_y)?)?;
	}
	platform::video::check_live_dimensions(visible_width, visible_height).ok()?;
	// Only 8/10-bit 4:2:0 surfaces are supported by the live pipeline.
	if chroma != 1 || bits.ue()? > 2 || bits.ue()? > 2 {
		return None;
	}
	Some(())
}
struct Bits<'a> {
	data: &'a [u8],
	at: usize,
}
impl Bits<'_> {
	fn skip(&mut self, count: usize) -> Option<()> {
		if count > self.data.len().checked_mul(8)?.checked_sub(self.at)? {
			return None;
		}
		self.at += count;
		Some(())
	}
	fn read(&mut self, count: usize) -> Option<u32> {
		if count > 32 {
			return None;
		}
		let mut result = 0;
		for _ in 0..count {
			result =
				(result << 1) | u32::from((*self.data.get(self.at / 8)? >> (7 - self.at % 8)) & 1);
			self.at += 1;
		}
		Some(result)
	}
	fn ue(&mut self) -> Option<u32> {
		let mut zeros = 0;
		while self.read(1)? == 0 {
			zeros += 1;
			if zeros > 31 {
				return None;
			}
		}
		((1u32 << zeros) - 1).checked_add(self.read(zeros)?)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn sps(width: u32, height: u32, crop_bottom: u32, chroma: u32, depth: u32) -> Vec<u8> {
		let mut bits = vec![false; 8 + 96]; // one temporal layer, profile_tier_level
		let mut ue = |value: u32| {
			let value = u64::from(value) + 1;
			let count = 64 - value.leading_zeros() as usize;
			bits.extend(std::iter::repeat_n(false, count - 1));
			bits.extend((0..count).rev().map(|i| value & (1 << i) != 0));
		};
		ue(0);
		ue(chroma);
		ue(width);
		ue(height);
		// Replace the helper after the conformance-window flag to retain bit alignment.
		bits.push(crop_bottom != 0);
		let mut ue = |value: u32| {
			let value = u64::from(value) + 1;
			let count = 64 - value.leading_zeros() as usize;
			bits.extend(std::iter::repeat_n(false, count - 1));
			bits.extend((0..count).rev().map(|i| value & (1 << i) != 0));
		};
		if crop_bottom != 0 {
			ue(0);
			ue(0);
			ue(0);
			ue(crop_bottom);
		}
		ue(depth);
		ue(depth);
		bits.push(true);
		let mut nal = vec![66, 1];
		for chunk in bits.chunks(8) {
			nal.push(
				chunk
					.iter()
					.enumerate()
					.fold(0, |v, (i, b)| v | (u8::from(*b) << (7 - i))),
			);
		}
		nal
	}
	#[test]
	fn hevc_sps_bounds_reject_oversized_surfaces_and_invalid_crops_before_decode() {
		assert!(bounded_sps(&sps(1280, 534, 0, 1, 0)));
		assert!(bounded_sps(&sps(1920, 1088, 4, 1, 2)));
		assert!(bounded_sps(&sps(2560, 1440, 0, 1, 0)));
		assert!(bounded_sps(&sps(3840, 2160, 0, 1, 0)));
		assert!(bounded_sps(&sps(3840, 2176, 8, 1, 2)));
		for nal in [
			sps(3840, 2176, 0, 1, 0),
			sps(4096, 2160, 0, 1, 0),
			sps(3840, 3840, 840, 1, 0),
			sps(1280, 534, 300, 1, 0),
			sps(1280, 534, 0, 1, 3),
			sps(1280, 534, 0, 2, 0),
			sps(u32::MAX, 534, 0, 1, 0),
		] {
			assert!(!bounded_sps(&nal));
		}
		let valid = sps(1280, 534, 0, 1, 0);
		for length in 0..valid.len() - 1 {
			assert!(!bounded_sps(&valid[..length]));
		}
	}
}
