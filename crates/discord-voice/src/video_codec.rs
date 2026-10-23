//! Go Live codec selection. Payload numbers are our advertised assignments, not
//! fixed Discord constants. H.265 and AV1 are receive-only on capable Linux systems.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Codec {
	#[default]
	H264,
	H265,
	Av1,
}

impl Codec {
	pub fn from_name(name: &str) -> Option<Self> {
		if name.eq_ignore_ascii_case("H264") {
			Some(Self::H264)
		} else if name.eq_ignore_ascii_case("H265") {
			Some(Self::H265)
		} else if name.eq_ignore_ascii_case("AV1") || name.eq_ignore_ascii_case("AV1X") {
			Some(Self::Av1)
		} else {
			None
		}
	}
	pub fn payload(self) -> u8 {
		match self {
			Self::H264 => 101,
			Self::H265 => 103,
			Self::Av1 => 105,
		}
	}
	pub fn available_h265() -> bool {
		#[cfg(target_os = "linux")]
		return platform::video::live::H264Decoder::new_h265_timed(Box::new(|_, _| {})).is_ok();
		#[cfg(not(target_os = "linux"))]
		false
	}
	pub fn available_av1() -> bool {
		#[cfg(target_os = "linux")]
		return platform::video::live::H264Decoder::new_av1_timed(Box::new(|_, _| {})).is_ok();
		#[cfg(not(target_os = "linux"))]
		false
	}
	pub fn is_keyframe(self, frame: &[u8]) -> bool {
		if self == Self::Av1 {
			return crate::video_av1::inspect(frame).is_some_and(|info| info.keyframe);
		}
		self.nals(frame).any(|nal| match self {
			Self::H264 => nal[0] & 31 == 5,
			// BLA/IDR and CRA pictures are independently decodable recovery points.
			Self::H265 => (16..=21).contains(&((nal[0] >> 1) & 63)),
			Self::Av1 => false,
		})
	}
	pub fn has_parameter_sets(self, frame: &[u8]) -> bool {
		if self == Self::Av1 {
			return crate::video_av1::inspect(frame).is_some_and(|info| info.sequence);
		}
		let mut found = 0u8;
		for nal in self.nals(frame) {
			match (self, nal[0]) {
				(Self::H264, h) if h & 31 == 7 => found |= 1,
				(Self::H264, h) if h & 31 == 8 => found |= 2,
				(Self::H265, h) => match (h >> 1) & 63 {
					32 => found |= 4,
					33 => found |= 1,
					34 => found |= 2,
					_ => {}
				},
				_ => {}
			}
		}
		found == if self == Self::H265 { 7 } else { 3 }
	}
	pub fn bounded_parameters(self, frame: &[u8]) -> bool {
		if self == Self::Av1 {
			return crate::video_av1::inspect(frame).is_some();
		}
		self != Self::H265
			|| self
				.nals(frame)
				.filter(|nal| (nal[0] >> 1) & 63 == 33)
				.all(crate::video_hevc::bounded_sps)
	}
	fn nals(self, frame: &[u8]) -> impl Iterator<Item = &[u8]> {
		let mut at = 0;
		std::iter::from_fn(move || {
			while let Some((start, size)) = start_code(frame, at) {
				let nal = start + size;
				at = start_code(frame, nal).map_or(frame.len(), |(end, _)| end);
				if at - nal >= if self == Self::H265 { 2 } else { 1 } {
					return Some(&frame[nal..at]);
				}
			}
			None
		})
	}
}

fn start_code(frame: &[u8], mut at: usize) -> Option<(usize, usize)> {
	while at + 3 <= frame.len() {
		if frame[at..].starts_with(&[0, 0, 0, 1]) {
			return Some((at, 4));
		}
		if frame[at..].starts_with(&[0, 0, 1]) {
			return Some((at, 3));
		}
		at += 1;
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn truncated_hevc_parameter_sets_cannot_read_into_following_nals() {
		let frame = [0, 0, 0, 1, 66, 1, 0, 0, 0, 1, 38, 1, 7, 7];
		let nals: Vec<_> = Codec::H265.nals(&frame).collect();
		assert_eq!(nals, [&[66, 1][..], &[38, 1, 7, 7]]);
		assert!(!Codec::H265.bounded_parameters(&frame));
		assert!(!Codec::H265.has_parameter_sets(&frame));
		assert!(Codec::H265.is_keyframe(&frame));
	}
}
