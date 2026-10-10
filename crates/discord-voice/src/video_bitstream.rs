//! Bounded inspection of outgoing encoder access units, before DAVE sees their bytes.
use model::voice_settings::VideoCodec;
use std::borrow::Cow;

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_UNITS: usize = 2048;

pub(super) fn nalus(frame: &[u8], codec: VideoCodec) -> Result<Vec<&[u8]>, &'static str> {
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
		if starts.len() == MAX_UNITS {
			return Err("Video frame exceeds its unit limit");
		}
		starts.push((at, size));
		at += size;
	}
	if starts.first().is_none_or(|start| start.0 != 0) {
		return Err("Video frame is not Annex B");
	}
	let mut units = Vec::with_capacity(starts.len());
	for (index, &(start, size)) in starts.iter().enumerate() {
		let end = starts.get(index + 1).map_or(frame.len(), |next| next.0);
		if start + size >= end {
			return Err("Video frame contains an empty NAL unit");
		}
		let unit = &frame[start + size..end];
		let valid = match codec {
			VideoCodec::H264 => {
				unit.len() >= 2 && unit[0] & 0x80 == 0 && (1..=23).contains(&(unit[0] & 31))
			}
			VideoCodec::H265 => {
				unit.len() >= 3
					&& unit[0] & 0x80 == 0
					&& (unit[0] >> 1) & 63 < 48
					&& unit[1] & 7 != 0
			}
			VideoCodec::Av1 => false,
		};
		if !valid {
			return Err("Invalid video NAL header");
		}
		units.push(unit);
	}
	Ok(units)
}

pub(super) struct Obu<'a> {
	pub header: u8,
	pub extension: Option<u8>,
	pub payload: &'a [u8],
}
impl Obu<'_> {
	pub fn kind(&self) -> u8 {
		(self.header >> 3) & 15
	}
	pub fn discarded(&self) -> bool {
		matches!(self.kind(), 2 | 8 | 15)
	}
}

/// Encoder input has explicit sizes. DAVE's final OBU has no size because its
/// authentication trailer follows the ciphertext in that same OBU.
pub(super) fn obus(frame: &[u8], explicit_sizes: bool) -> Result<Vec<Obu<'_>>, &'static str> {
	let mut units = Vec::new();
	let mut at = 0;
	while at < frame.len() {
		if units.len() == MAX_UNITS {
			return Err("AV1 frame exceeds its OBU limit");
		}
		let header = frame[at];
		at += 1;
		let kind = (header >> 3) & 15;
		if header & 0x81 != 0 || !(1..=8).contains(&kind) && kind != 15 {
			return Err("Invalid AV1 OBU header");
		}
		let extension = if header & 4 != 0 {
			let value = *frame.get(at).ok_or("Truncated AV1 OBU extension")?;
			if value & 7 != 0 {
				return Err("Invalid AV1 OBU extension");
			}
			at += 1;
			Some(value)
		} else {
			None
		};
		let size = if header & 2 != 0 {
			let (size, consumed) = read_size(&frame[at..])?;
			at += consumed;
			size
		} else if explicit_sizes {
			return Err("AV1 encoder output requires explicit OBU sizes");
		} else {
			frame.len() - at
		};
		if size > frame.len() - at {
			return Err("AV1 OBU exceeds its frame");
		}
		if size == 0 && !matches!(kind, 2 | 15) {
			return Err("Empty AV1 OBU payload");
		}
		units.push(Obu {
			header,
			extension,
			payload: &frame[at..at + size],
		});
		at += size;
	}
	if units.is_empty() {
		return Err("AV1 frame has no OBUs");
	}
	Ok(units)
}

fn read_size(bytes: &[u8]) -> Result<(usize, usize), &'static str> {
	let mut size = 0u64;
	for (index, &byte) in bytes.iter().take(8).enumerate() {
		size |= u64::from(byte & 127) << (index * 7);
		if byte & 128 == 0 {
			return usize::try_from(size)
				.map(|size| (size, index + 1))
				.map_err(|_| "AV1 OBU size overflow");
		}
	}
	Err("Invalid AV1 OBU size")
}

fn write_size(mut size: usize, bytes: &mut Vec<u8>) {
	while size >= 128 {
		bytes.push((size as u8 & 127) | 128);
		size >>= 7;
	}
	bytes.push(size as u8);
}

pub(super) fn validate(frame: &[u8], codec: VideoCodec) -> Result<(), &'static str> {
	if frame.len() > MAX_BYTES {
		return Err("Encoded video frame exceeds the sharing limit");
	}
	match codec {
		VideoCodec::H264 | VideoCodec::H265 => nalus(frame, codec).map(|_| ()),
		VideoCodec::Av1 => {
			let units = obus(frame, true)?;
			if !units.iter().any(|unit| matches!(unit.kind(), 3 | 6)) {
				return Err("AV1 access unit has no frame header");
			}
			Ok(())
		}
	}
}

pub(super) fn has_parameters(frame: &[u8], codec: VideoCodec) -> bool {
	if validate(frame, codec).is_err() {
		return false;
	}
	match codec {
		VideoCodec::H264 | VideoCodec::H265 => {
			let units = nalus(frame, codec).unwrap_or_default();
			let mut found = 0;
			for unit in units {
				found |= match (
					codec,
					if codec == VideoCodec::H264 {
						unit[0] & 31
					} else {
						unit[0] >> 1 & 63
					},
				) {
					(VideoCodec::H264, 7) | (VideoCodec::H265, 33) => 1,
					(VideoCodec::H264, 8) | (VideoCodec::H265, 34) => 2,
					(VideoCodec::H265, 32) => 4,
					_ => 0,
				};
			}
			found == if codec == VideoCodec::H264 { 3 } else { 7 }
		}
		VideoCodec::Av1 => {
			obus(frame, true).is_ok_and(|units| units.iter().any(|unit| unit.kind() == 1))
		}
	}
}

pub(super) fn is_keyframe(frame: &[u8], codec: VideoCodec) -> bool {
	if validate(frame, codec).is_err() {
		return false;
	}
	match codec {
		VideoCodec::H264 | VideoCodec::H265 => nalus(frame, codec).is_ok_and(|units| {
			units.iter().any(|unit| {
				if codec == VideoCodec::H264 {
					unit[0] & 31 == 5
				} else {
					(16..=21).contains(&(unit[0] >> 1 & 63))
				}
			})
		}),
		VideoCodec::Av1 => {
			let mut reduced = None;
			for unit in obus(frame, true).unwrap_or_default() {
				if unit.kind() == 1 {
					let first = unit.payload[0];
					if first >> 5 > 2 || first & 8 != 0 && first & 16 == 0 {
						return false;
					}
					reduced = Some(first & 8 != 0);
				} else if matches!(unit.kind(), 3 | 6) {
					// For regular sequences, show_existing_frame must be zero and
					// the following two frame_type bits must name KEY_FRAME.
					return reduced.is_some_and(|reduced| reduced || unit.payload[0] & 0xe0 == 0);
				}
			}
			false
		}
	}
}

pub(super) fn prepare(frame: &[u8], codec: VideoCodec) -> Result<Cow<'_, [u8]>, &'static str> {
	validate(frame, codec)?;
	match codec {
		VideoCodec::H264 => crate::video_sps::normalize(frame),
		VideoCodec::H265 => {
			let mut output = Vec::with_capacity(frame.len());
			for unit in nalus(frame, codec)? {
				output.extend([0, 0, 0, 1]);
				output.extend_from_slice(unit);
			}
			Ok(Cow::Owned(output))
		}
		VideoCodec::Av1 => {
			let mut output = Vec::with_capacity(frame.len());
			for unit in obus(frame, true)? {
				if unit.discarded() {
					continue;
				}
				output.push(unit.header | 2);
				if let Some(extension) = unit.extension {
					output.push(extension);
				}
				write_size(unit.payload.len(), &mut output);
				output.extend_from_slice(unit.payload);
			}
			Ok(Cow::Owned(output))
		}
	}
}
