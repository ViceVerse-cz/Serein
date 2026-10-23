//! AOM AV1 RTP v1.0.0, single spatial layer. Rebuild the final size-less OBU exactly
//! as DAVE expects; only authenticated plaintext is inspected or normalized for decode.
use super::video_receive::MAX_FRAME_BYTES;

pub(super) fn leb(data: &[u8]) -> Option<(usize, usize)> {
	let mut value = 0u32;
	for (i, &byte) in data.iter().take(8).enumerate() {
		let low = u32::from(byte & 127);
		if i >= 4 && low != 0 {
			return None;
		}
		if i < 4 {
			value |= low << (i * 7);
		}
		if byte & 128 == 0 {
			return Some((value as usize, i + 1));
		}
	}
	None
}
fn size(mut value: usize, out: &mut Vec<u8>) {
	while value >= 128 {
		out.push((value as u8 & 127) | 128);
		value >>= 7;
	}
	out.push(value as u8);
}
fn header(obu: &[u8]) -> Option<usize> {
	let &h = obu.first()?;
	if h & 0x81 != 0 || matches!((h >> 3) & 15, 0 | 8) {
		return None;
	}
	if h & 4 != 0 {
		// Temporal scalability is permitted; extra spatial surfaces are not.
		if obu.get(1)? & 31 != 0 {
			return None;
		}
		Some(2)
	} else {
		Some(1)
	}
}
fn body(obu: &[u8]) -> Option<(usize, &[u8])> {
	let h = header(obu)?;
	if obu[0] & 2 == 0 {
		return Some((h, &obu[h..]));
	}
	let (len, count) = leb(&obu[h..])?;
	if h + count + len != obu.len() {
		return None;
	}
	Some((h, &obu[h + count..]))
}
fn reserve(out: &mut Vec<u8>, required: usize) {
	if required > out.capacity() && out.capacity().saturating_mul(2) > MAX_FRAME_BYTES {
		out.reserve_exact(required - out.len());
	}
}
fn write_obu(obu: &[u8], omit_size: bool, out: &mut Vec<u8>) -> Option<()> {
	let (h, payload) = body(obu)?;
	if out.len().checked_add(h + 8 + payload.len())? > MAX_FRAME_BYTES {
		return None;
	}
	reserve(out, out.len() + h + 8 + payload.len());
	out.push(if omit_size { obu[0] & !2 } else { obu[0] | 2 });
	out.extend_from_slice(&obu[1..h]);
	if !omit_size {
		size(payload.len(), out);
	}
	out.extend_from_slice(payload);
	Some(())
}

#[derive(Default)]
pub(super) struct Fragments {
	partial: Vec<u8>,
	units: usize,
}
impl Fragments {
	pub fn clear(&mut self) {
		self.partial.clear();
		self.units = 0;
		if self.partial.capacity() > 256 * 1024 {
			self.partial = Vec::new();
		}
	}
	pub fn pending(&self) -> bool {
		!self.partial.is_empty()
	}
	pub fn append(&mut self, data: &[u8], marker: bool, frame: &mut Vec<u8>) -> Result<(), ()> {
		let &h = data.first().ok_or(())?;
		let z = h & 128 != 0;
		let y = h & 64 != 0;
		let w = usize::from((h >> 4) & 3);
		if h & 7 != 0 || (h & 8 != 0 && z) || (marker && y) || z != self.pending() {
			return Err(());
		}
		let mut at = 1;
		let mut count = 0;
		while at < data.len() {
			count += 1;
			if count > 64 {
				return Err(());
			}
			let len = if w == 0 || count != w {
				let (len, n) = leb(&data[at..]).ok_or(())?;
				at += n;
				len
			} else {
				data.len() - at
			};
			let element = data
				.get(at..at.checked_add(len).ok_or(())?)
				.filter(|s| !s.is_empty())
				.ok_or(())?;
			at += len;
			if frame.len() + self.partial.len() + element.len() + 8 > MAX_FRAME_BYTES {
				return Err(());
			}
			let continues = count == 1 && z;
			if !continues && self.pending() {
				return Err(());
			}
			let required = self.partial.len() + element.len();
			reserve(&mut self.partial, required);
			self.partial.extend_from_slice(element);
			if at == data.len() && y {
				continue;
			}
			self.units += 1;
			if self.units > 256 {
				return Err(());
			}
			write_obu(&self.partial, marker && at == data.len(), frame).ok_or(())?;
			self.partial.clear();
		}
		if count == 0 || (w != 0 && w != count) {
			return Err(());
		}
		Ok(())
	}
}

fn obus(mut data: &[u8], mut visit: impl FnMut(u8, &[u8]) -> Option<()>) -> Option<()> {
	let mut count = 0;
	while !data.is_empty() {
		count += 1;
		if count > 256 {
			return None;
		}
		let h = header(data)?;
		let (len, n) = if data[0] & 2 != 0 {
			leb(&data[h..])?
		} else {
			(data.len() - h, 0)
		};
		let end = h.checked_add(n)?.checked_add(len)?;
		visit((data[0] >> 3) & 15, data.get(h + n..end)?)?;
		data = data.get(end..)?;
	}
	(count > 0).then_some(())
}
#[derive(Default)]
pub(super) struct Info {
	pub keyframe: bool,
	pub sequence: bool,
}
pub(super) fn inspect(data: &[u8]) -> Option<Info> {
	let mut info = Info::default();
	let mut reduced = false;
	obus(data, |kind, payload| {
		if kind == 1 {
			reduced = sequence(payload)?;
			info.sequence = true;
		}
		if matches!(kind, 3 | 6) {
			info.keyframe |= reduced || payload.first().is_some_and(|b| b & 0xf0 == 0x10);
		}
		Some(())
	})?;
	Some(info)
}
/// GStreamer's obu-stream needs explicit sizes, including on the final plaintext OBU.
pub(super) fn normalize(mut data: &[u8]) -> Option<Vec<u8>> {
	let mut result = Vec::new();
	while !data.is_empty() {
		let h = header(data)?;
		let (len, n) = if data[0] & 2 != 0 {
			leb(&data[h..])?
		} else {
			(data.len() - h, 0)
		};
		let end = h.checked_add(n)?.checked_add(len)?;
		write_obu(data.get(..end)?, false, &mut result)?;
		data = data.get(end..)?;
	}
	Some(result)
}

// AV1 5.5 sequence_header_obu: validate profile, maximum surface and colour before
// native submission. Decode only Main profile's 8/10-bit, non-monochrome 4:2:0.
fn sequence(data: &[u8]) -> Option<bool> {
	let mut b = Bits {
		data: data.get(..data.len().min(4096))?,
		at: 0,
	};
	if b.read(3)? != 0 {
		return None;
	}
	let still = b.read(1)? != 0;
	let reduced = b.read(1)? != 0;
	if reduced {
		if !still {
			return None;
		}
		b.read(5)?;
	} else {
		let mut delay_bits = None;
		if b.read(1)? != 0 {
			b.skip(64)?;
			if b.read(1)? != 0 {
				b.uvlc()?;
			}
			if b.read(1)? != 0 {
				delay_bits = Some(b.read(5)? as usize + 1);
				b.skip(42)?;
			}
		}
		let display = b.read(1)? != 0;
		let ops = b.read(5)?;
		for _ in 0..=ops {
			b.read(12)?;
			if b.read(5)? > 7 {
				b.read(1)?;
			}
			if let Some(n) = delay_bits
				&& b.read(1)? != 0
			{
				b.skip(n * 2 + 1)?;
			}
			if display && b.read(1)? != 0 {
				b.read(4)?;
			}
		}
	}
	let width_bits = b.read(4)? as usize + 1;
	let height_bits = b.read(4)? as usize + 1;
	let width = b.read(width_bits)? + 1;
	let height = b.read(height_bits)? + 1;
	platform::video::check_live_dimensions(width, height).ok()?;
	if !reduced && b.read(1)? != 0 {
		b.skip(7)?;
	}
	b.skip(3)?;
	if !reduced {
		b.skip(4)?;
		let order = b.read(1)? != 0;
		if order {
			b.skip(2)?;
		}
		let screen = if b.read(1)? != 0 { 2 } else { b.read(1)? };
		if screen > 0 && b.read(1)? == 0 {
			b.read(1)?;
		}
		if order {
			b.read(3)?;
		}
	}
	b.skip(3)?;
	b.read(1)?; // high_bitdepth, Main supports 8 or 10 bits.
	if b.read(1)? != 0 {
		return None;
	} // monochrome
	if b.read(1)? != 0 {
		let colour = (b.read(8)?, b.read(8)?, b.read(8)?);
		if colour == (1, 13, 0) {
			return None;
		} // RGB / 4:4:4
	}
	b.skip(1 + 2 + 1 + 1)?; // range, chroma position, separate UV, film grain
	Some(reduced)
}
struct Bits<'a> {
	data: &'a [u8],
	at: usize,
}
impl Bits<'_> {
	fn skip(&mut self, n: usize) -> Option<()> {
		if n > self.data.len().checked_mul(8)?.checked_sub(self.at)? {
			return None;
		}
		self.at += n;
		Some(())
	}
	fn read(&mut self, n: usize) -> Option<u32> {
		if n > 32 {
			return None;
		}
		let mut value = 0;
		for _ in 0..n {
			value =
				(value << 1) | u32::from((self.data.get(self.at / 8)? >> (7 - self.at % 8)) & 1);
			self.at += 1;
		}
		Some(value)
	}
	fn uvlc(&mut self) -> Option<()> {
		let mut n = 0;
		while self.read(1)? == 0 {
			n += 1;
			if n >= 32 {
				return Some(());
			}
		}
		self.skip(n)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn aggregate_and_fragment_rebuild_dave_framing_before_plaintext_normalization() {
		let mut fragments = Fragments::default();
		let mut frame = Vec::new();
		fragments
			.append(&[0x60, 2, 0x28, 0x55, 0x30, 0x10], false, &mut frame)
			.unwrap();
		fragments
			.append(&[0x90, 0xaa, 0xbb], true, &mut frame)
			.unwrap();
		assert_eq!(frame, [0x2a, 1, 0x55, 0x30, 0x10, 0xaa, 0xbb]);
		assert!(inspect(&frame).unwrap().keyframe);
		assert_eq!(
			normalize(&frame).unwrap(),
			[0x2a, 1, 0x55, 0x32, 3, 0x10, 0xaa, 0xbb]
		);
		for payload in [
			&[0x90, 0x30, 7][..], // Continuation without a fragment.
			&[0x50, 0x30, 7],     // Marker on an unfinished fragment.
			&[0, 0x80],           // Truncated element length.
			&[0x30, 1, 0x28],     // Wrong element count.
			&[0x10, 0x34, 8, 7],  // Extra spatial layer.
			&[0x10, 0xb0, 7],     // Forbidden OBU bit.
			&[0x10, 0x40, 7],     // Tile lists are outside AV1 RTP reception.
			&[0x10, 0x32, 5, 7],  // Inconsistent internal size.
			&[0x10],              // No element.
		] {
			assert!(
				Fragments::default()
					.append(payload, true, &mut Vec::new())
					.is_err()
			);
		}
	}
	#[test]
	fn fragment_bytes_and_frame_obu_counts_are_bounded_before_copying() {
		let mut fragments = Fragments::default();
		let mut frame = Vec::new();
		fragments
			.append(&[0x50, 0x30, 7], false, &mut frame)
			.unwrap();
		let mut middle = vec![7; 4096];
		middle[0] = 0xd0;
		while fragments.append(&middle, false, &mut frame).is_ok() {}
		assert!(frame.len() + fragments.partial.len() <= MAX_FRAME_BYTES);
		assert!(fragments.partial.capacity() <= MAX_FRAME_BYTES);
		fragments.clear();
		let packet: Vec<_> = std::iter::once(0).chain([1, 0x28].repeat(64)).collect();
		for _ in 0..4 {
			fragments.append(&packet, false, &mut frame).unwrap();
		}
		assert!(fragments.append(&[0x10, 0x28], true, &mut frame).is_err());
	}
	fn reduced(width: u32, height: u32, mono: bool) -> Vec<u8> {
		let mut bits = Vec::new();
		let mut put = |v: u32, n: usize| bits.extend((0..n).rev().map(|i| v & (1 << i) != 0));
		put(0, 3);
		put(1, 1);
		put(1, 1);
		put(0, 5);
		put(15, 4);
		put(15, 4);
		put(width - 1, 16);
		put(height - 1, 16);
		put(0, 3 + 3);
		put(0, 1);
		put(u32::from(mono), 1);
		put(0, 1);
		put(0, 5);
		put(1, 1);
		bits.chunks(8)
			.map(|c| {
				c.iter()
					.enumerate()
					.fold(0, |v, (i, b)| v | (u8::from(*b) << (7 - i)))
			})
			.collect()
	}
	#[test]
	fn sequence_bounds_reject_unsupported_surfaces_and_truncated_colour_syntax() {
		assert_eq!(sequence(&reduced(2560, 1440, false)), Some(true));
		assert_eq!(sequence(&reduced(3840, 2160, false)), Some(true));
		assert_eq!(sequence(&reduced(2160, 3840, false)), Some(true));
		assert!(sequence(&reduced(3840, 3840, false)).is_none());
		assert!(sequence(&reduced(4096, 2160, false)).is_none());
		assert!(sequence(&reduced(640, 480, true)).is_none());
		let valid = reduced(2560, 1440, false);
		for end in 0..valid.len() - 1 {
			assert!(sequence(&valid[..end]).is_none());
		}
	}
}
