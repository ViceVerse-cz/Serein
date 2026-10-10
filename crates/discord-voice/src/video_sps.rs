//! Normalize SPS before DAVE so WebRTC's receive-side VUI rewrite preserves authentication.
//! H.264 sections 7.3.2.1 and E.1; WebRTC common_video/h264/sps_vui_rewriter.cc.
use std::borrow::Cow;
use std::ops::Range;

const INVALID: &str = "Invalid H264 sequence parameter set";
const MAX_FRAME: usize = 2 * 1024 * 1024;
const MAX_SPS: usize = 4096;

pub(crate) fn normalize(frame: &[u8]) -> Result<Cow<'_, [u8]>, &'static str> {
	if frame.len() > MAX_FRAME {
		return Err(INVALID);
	}
	let units = nal_ranges(frame)?;
	let mut output = Vec::new();
	let mut copied = 0;
	for range in units {
		let nal = &frame[range.clone()];
		if nal[0] & 31 == 7
			&& let Some(rewritten) = rewrite(&nal[1..])?
		{
			output.extend_from_slice(&frame[copied..range.start + 1]);
			output.extend_from_slice(&rewritten);
			copied = range.end;
			if output.len() > MAX_FRAME {
				return Err(INVALID);
			}
		}
	}
	if copied == 0 {
		Ok(Cow::Borrowed(frame))
	} else {
		output.extend_from_slice(&frame[copied..]);
		if output.len() > MAX_FRAME {
			return Err(INVALID);
		}
		Ok(Cow::Owned(output))
	}
}

fn nal_ranges(frame: &[u8]) -> Result<Vec<Range<usize>>, &'static str> {
	let mut starts = Vec::new();
	let mut at = 0;
	while at + 3 <= frame.len() {
		let size = if frame[at..].starts_with(&[0, 0, 0, 1]) {
			4
		} else if frame[at..].starts_with(&[0, 0, 1]) {
			3
		} else {
			at += 1;
			continue;
		};
		if starts.len() == 2048 {
			return Err(INVALID);
		}
		starts.push((at, at + size));
		at += size;
	}
	if starts.first().is_none_or(|&(start, _)| start != 0) {
		return Err(INVALID);
	}
	let mut units = Vec::with_capacity(starts.len());
	for (index, &(_, start)) in starts.iter().enumerate() {
		let end = starts
			.get(index + 1)
			.map_or(frame.len(), |&(start, _)| start);
		let nal = &frame[start..end];
		if nal.is_empty() || nal[0] & 0x80 != 0 || !(1..=23).contains(&(nal[0] & 31)) {
			return Err(INVALID);
		}
		units.push(start..end);
	}
	Ok(units)
}

pub(crate) struct Dimensions {
	/// Native reference surfaces use macroblock dimensions, before cropping.
	pub coded: (usize, usize),
	pub visible: (usize, usize),
}

/// Inspect every replacement SPS before native decoding can allocate its surfaces.
/// Subset SPS has the same prefix and is also understood by OpenH264's SVC decoder.
pub(crate) fn validate_dimensions(
	frame: &[u8],
	mut admitted: impl FnMut(Dimensions) -> bool,
) -> Result<(), &'static str> {
	for range in nal_ranges(frame)? {
		let nal = &frame[range];
		if matches!(nal[0] & 31, 7 | 15) {
			let (_, _, dimensions) = prefix(&nal[1..])?;
			if !admitted(dimensions) {
				return Err("H264 sequence parameter set exceeds the receive dimensions");
			}
		}
	}
	Ok(())
}

struct Bits {
	data: Vec<bool>,
	at: usize,
}
impl Bits {
	fn read(&mut self, count: usize) -> Result<u32, &'static str> {
		if count > 32 || count > self.data.len().saturating_sub(self.at) {
			return Err(INVALID);
		}
		let mut value = 0;
		for _ in 0..count {
			value = (value << 1) | u32::from(self.data[self.at]);
			self.at += 1;
		}
		Ok(value)
	}
	fn flag(&mut self) -> Result<bool, &'static str> {
		Ok(self.read(1)? != 0)
	}
	fn ue(&mut self) -> Result<u32, &'static str> {
		let mut zeros = 0;
		while !self.flag()? {
			zeros += 1;
			if zeros > 31 {
				return Err(INVALID);
			}
		}
		Ok(((1u32 << zeros) - 1) + self.read(zeros)?)
	}
	fn se(&mut self) -> Result<i64, &'static str> {
		let code = i64::from(self.ue()?);
		Ok(if code & 1 == 0 {
			-(code / 2)
		} else {
			(code + 1) / 2
		})
	}
	fn hrd(&mut self) -> Result<(), &'static str> {
		let count = self.ue()?;
		if count > 31 {
			return Err(INVALID);
		}
		self.read(8)?;
		for _ in 0..=count {
			self.ue()?;
			self.ue()?;
			self.read(1)?;
		}
		self.read(20)?;
		Ok(())
	}
}

fn ue(bits: &mut Vec<bool>, value: u32) {
	let code = u64::from(value) + 1;
	let width = 64 - code.leading_zeros();
	bits.extend(std::iter::repeat_n(false, width as usize - 1));
	bits.extend((0..width).rev().map(|shift| code & (1 << shift) != 0));
}

fn prefix(escaped: &[u8]) -> Result<(Bits, u32, Dimensions), &'static str> {
	if escaped.is_empty() || escaped.len() > MAX_SPS {
		return Err(INVALID);
	}
	let mut bytes = Vec::with_capacity(escaped.len());
	let mut zeros = 0;
	for (index, &byte) in escaped.iter().enumerate() {
		if zeros == 2 && byte == 3 {
			if escaped.get(index + 1).is_none_or(|&next| next > 3) {
				return Err(INVALID);
			}
			zeros = 0;
			continue;
		}
		bytes.push(byte);
		zeros = if byte == 0 { zeros + 1 } else { 0 };
	}
	let mut bits = Bits {
		data: bytes
			.iter()
			.flat_map(|byte| (0..8).rev().map(move |bit| byte & (1 << bit) != 0))
			.collect(),
		at: 0,
	};
	// Exclude rbsp_stop_one_bit and padding from the parser's readable input.
	let stop = bits.data.iter().rposition(|&bit| bit).ok_or(INVALID)?;
	bits.data.truncate(stop);
	let profile = bits.read(8)?;
	bits.read(16)?;
	if bits.ue()? > 31 {
		return Err(INVALID);
	}
	let mut chroma = 1;
	if matches!(
		profile,
		100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
	) {
		chroma = bits.ue()?;
		if chroma > 3 {
			return Err(INVALID);
		}
		let separate_planes = chroma == 3 && bits.flag()?;
		if bits.ue()? > 6 || bits.ue()? > 6 {
			return Err(INVALID);
		}
		bits.read(1)?;
		if bits.flag()? {
			for index in 0..if chroma == 3 { 12 } else { 8 } {
				if bits.flag()? {
					let mut last = 8i64;
					let mut next = 8i64;
					for _ in 0..if index < 6 { 16 } else { 64 } {
						if next != 0 {
							next = (last + bits.se()?).rem_euclid(256);
						}
						if next != 0 {
							last = next;
						}
					}
				}
			}
		}
		if separate_planes {
			chroma = 0;
		}
	} else if !matches!(profile, 66 | 77 | 88) {
		return Err(INVALID);
	}
	if bits.ue()? > 12 {
		return Err(INVALID);
	}
	match bits.ue()? {
		0 => {
			if bits.ue()? > 12 {
				return Err(INVALID);
			}
		}
		1 => {
			bits.read(1)?;
			bits.se()?;
			bits.se()?;
			let cycle = bits.ue()?;
			if cycle > 255 {
				return Err(INVALID);
			}
			for _ in 0..cycle {
				bits.se()?;
			}
		}
		2 => {}
		_ => return Err(INVALID),
	}
	let refs = bits.ue()?;
	if refs > 16 {
		return Err(INVALID);
	}
	bits.read(1)?;
	let width = (u64::from(bits.ue()?) + 1) * 16;
	let map_height = (u64::from(bits.ue()?) + 1) * 16;
	let frame_only = bits.flag()?;
	let vertical = if frame_only { 1 } else { 2 };
	let height = map_height * vertical;
	if !frame_only {
		bits.read(1)?;
	}
	bits.read(1)?;
	let (mut cropped_width, mut cropped_height) = (width, height);
	if bits.flag()? {
		let horizontal = if matches!(chroma, 1 | 2) { 2 } else { 1 };
		let crop_vertical = if chroma == 1 { 2 * vertical } else { vertical };
		let left = u64::from(bits.ue()?);
		let right = u64::from(bits.ue()?);
		let top = u64::from(bits.ue()?);
		let bottom = u64::from(bits.ue()?);
		cropped_width = width
			.checked_sub((left + right) * horizontal)
			.ok_or(INVALID)?;
		cropped_height = height
			.checked_sub((top + bottom) * crop_vertical)
			.ok_or(INVALID)?;
	}
	// At least vui_parameters_present_flag remains; truncation must fail admission.
	if bits.at == bits.data.len() || cropped_width == 0 || cropped_height == 0 {
		return Err(INVALID);
	}
	let dimensions = Dimensions {
		coded: (
			usize::try_from(width).map_err(|_| INVALID)?,
			usize::try_from(height).map_err(|_| INVALID)?,
		),
		visible: (
			usize::try_from(cropped_width).map_err(|_| INVALID)?,
			usize::try_from(cropped_height).map_err(|_| INVALID)?,
		),
	};
	Ok((bits, refs, dimensions))
}

fn rewrite(escaped: &[u8]) -> Result<Option<Vec<u8>>, &'static str> {
	let (mut bits, refs, _) = prefix(escaped)?;
	let mut start = bits.at;
	let mut replacement = vec![true];
	if bits.flag()? {
		if bits.flag()? && bits.read(8)? == 255 {
			bits.read(32)?;
		}
		if bits.flag()? {
			bits.read(1)?;
		}
		if bits.flag()? {
			bits.read(4)?;
			if bits.flag()? {
				bits.read(24)?;
			}
		}
		if bits.flag()? {
			bits.ue()?;
			bits.ue()?;
		}
		if bits.flag()? {
			bits.read(32)?;
			bits.read(32)?;
			bits.read(1)?;
		}
		let nal_hrd = bits.flag()?;
		if nal_hrd {
			bits.hrd()?;
		}
		let vcl_hrd = bits.flag()?;
		if vcl_hrd {
			bits.hrd()?;
		}
		if nal_hrd || vcl_hrd {
			bits.read(1)?;
		}
		bits.read(1)?;
		start = bits.at;
		if bits.flag()? {
			bits.read(1)?;
			for _ in 0..4 {
				bits.ue()?;
			}
			let values_start = bits.at;
			let reorder = bits.ue()?;
			let buffering = bits.ue()?;
			if bits.at != bits.data.len() {
				return Err(INVALID);
			}
			if reorder == 0 && buffering <= refs {
				return Ok(None);
			}
			start = values_start;
			replacement.clear();
			ue(&mut replacement, 0);
			ue(&mut replacement, refs);
		} else {
			restrictions(&mut replacement, refs);
		}
	} else {
		replacement.extend([false; 8]);
		replacement.push(true);
		restrictions(&mut replacement, refs);
	}
	if bits.at != bits.data.len() {
		return Err(INVALID);
	}
	bits.data.truncate(start);
	bits.data.extend(replacement);
	bits.data.push(true);
	let mut result = Vec::new();
	let mut zeros = 0;
	for chunk in bits.data.chunks(8) {
		let byte = chunk
			.iter()
			.enumerate()
			.fold(0, |value, (i, &bit)| value | (u8::from(bit) << (7 - i)));
		if zeros == 2 && byte <= 3 {
			result.push(3);
			zeros = 0;
		}
		result.push(byte);
		zeros = if byte == 0 { zeros + 1 } else { 0 };
	}
	Ok(Some(result))
}

fn restrictions(bits: &mut Vec<bool>, refs: u32) {
	bits.push(true); // motion_vectors_over_pic_boundaries_flag
	for value in [2, 1, 16, 16, 0, refs] {
		ue(bits, value);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sequence(
		coded: (u32, u32),
		crop: [u32; 4],
		chroma: u32,
		separate_planes: bool,
		frame_only: bool,
	) -> Vec<u8> {
		let mut bits = Vec::new();
		for byte in [100_u8, 0, 52] {
			bits.extend((0..8).rev().map(|shift| byte & (1 << shift) != 0));
		}
		ue(&mut bits, 0); // seq_parameter_set_id
		ue(&mut bits, chroma);
		if chroma == 3 {
			bits.push(separate_planes);
		}
		ue(&mut bits, 0); // bit_depth_luma_minus8
		ue(&mut bits, 0); // bit_depth_chroma_minus8
		bits.extend([false, false]); // transform bypass, scaling matrices
		ue(&mut bits, 0); // log2_max_frame_num_minus4
		ue(&mut bits, 2); // pic_order_cnt_type
		ue(&mut bits, 1); // max_num_ref_frames
		bits.push(false); // gaps_in_frame_num_value_allowed_flag
		ue(&mut bits, coded.0 / 16 - 1);
		ue(&mut bits, coded.1 / if frame_only { 16 } else { 32 } - 1);
		bits.push(frame_only);
		if !frame_only {
			bits.push(false); // mb_adaptive_frame_field_flag
		}
		bits.push(true); // direct_8x8_inference_flag
		bits.push(crop != [0; 4]);
		if crop != [0; 4] {
			for value in crop {
				ue(&mut bits, value);
			}
		}
		bits.extend([false, true]); // absent VUI, rbsp_stop_one_bit
		let mut frame = vec![0, 0, 0, 1, 0x67];
		let mut zeros = 0;
		for chunk in bits.chunks(8) {
			let byte = chunk
				.iter()
				.enumerate()
				.fold(0, |value, (i, &bit)| value | (u8::from(bit) << (7 - i)));
			if zeros == 2 && byte <= 3 {
				frame.push(3);
				zeros = 0;
			}
			frame.push(byte);
			zeros = if byte == 0 { zeros + 1 } else { 0 };
		}
		frame
	}

	#[test]
	fn dimension_prefix_preserves_chroma_and_field_crop_units() {
		for (chroma, separate_planes, frame_only, visible) in [
			(0, false, true, (15, 31)),
			(1, false, true, (14, 30)),
			(2, false, true, (14, 31)),
			(3, false, true, (15, 31)),
			(3, true, true, (15, 31)),
			(1, false, false, (14, 28)),
		] {
			let frame = sequence((16, 32), [0, 1, 0, 1], chroma, separate_planes, frame_only);
			validate_dimensions(&frame, |dimensions| {
				assert_eq!(dimensions.coded, (16, 32));
				assert_eq!(dimensions.visible, visible);
				true
			})
			.unwrap();
		}
	}

	#[test]
	fn dimension_gate_does_not_admit_large_surfaces_hidden_by_crop_or_subset_sps() {
		for nal_type in [7, 15] {
			let mut frame = sequence((4096, 2304), [0, 2032, 0, 1136], 1, false, true);
			frame[4] = 0x60 | nal_type;
			assert!(
				validate_dimensions(&frame, |dimensions| {
					assert_eq!(dimensions.visible, (32, 32));
					assert_eq!(dimensions.coded, (4096, 2304));
					false
				})
				.is_err()
			);
		}
		for (coded, crop, visible) in [
			((1920, 1088), [0, 0, 0, 4], (1920, 1080)),
			((1088, 1920), [0, 4, 0, 0], (1080, 1920)),
		] {
			let frame = sequence(coded, crop, 1, false, true);
			validate_dimensions(&frame, |dimensions| {
				assert_eq!(dimensions.visible, visible);
				assert_eq!(dimensions.coded, (coded.0 as usize, coded.1 as usize));
				true
			})
			.unwrap();
		}
	}

	#[test]
	fn dimension_gate_rejects_truncation_invalid_crop_and_oversized_sps() {
		let frame = sequence((32, 32), [0; 4], 1, false, true);
		for length in 0..frame.len() {
			assert!(validate_dimensions(&frame[..length], |_| true).is_err());
		}
		for crop in [[0, 17, 0, 0], [0, 0, 0, 17], [0, 16, 0, 0]] {
			assert!(
				validate_dimensions(&sequence((32, 32), crop, 1, false, true), |_| true).is_err()
			);
		}
		let mut large = vec![0, 0, 1, 0x67];
		large.extend(std::iter::repeat_n(0xff, MAX_SPS + 1));
		assert!(validate_dimensions(&large, |_| true).is_err());
	}
}
