//! Minimal Ogg Opus writer for one trimmed mono soundboard clip.
//! References: RFC 3533 (Ogg pages) and RFC 7845 (Opus in Ogg).
use opus2::{Application, Bitrate, Channels, Encoder};

const FRAME: usize = 960;
/// Ten seconds of 48 kHz mono; callers pass at most 5.2 seconds.
const MAX_SAMPLES: usize = 48_000 * 10;
const BITRATE: i32 = 96_000;
const SERIAL: u32 = 0x5e_7e_14;

/// The Ogg checksum: CRC-32 with polynomial 0x04c11db7, no reflection, zero initial value.
fn crc(bytes: &[u8]) -> u32 {
	let mut crc = 0u32;
	for byte in bytes {
		crc ^= u32::from(*byte) << 24;
		for _ in 0..8 {
			crc = if crc & 0x8000_0000 != 0 {
				(crc << 1) ^ 0x04c1_1db7
			} else {
				crc << 1
			};
		}
	}
	crc
}

struct Pages {
	bytes: Vec<u8>,
	sequence: u32,
}
impl Pages {
	/// Write one page holding whole packets; at most 255 lacing segments fit.
	fn page(&mut self, kind: u8, granule: u64, packets: &[Vec<u8>]) -> Result<(), &'static str> {
		let mut lacing = Vec::new();
		for packet in packets {
			lacing.extend(std::iter::repeat_n(255u8, packet.len() / 255));
			lacing.push((packet.len() % 255) as u8);
		}
		if lacing.len() > 255 {
			return Err("Sound could not be encoded");
		}
		let start = self.bytes.len();
		self.bytes.extend_from_slice(b"OggS");
		self.bytes.extend_from_slice(&[0, kind]);
		self.bytes.extend_from_slice(&granule.to_le_bytes());
		self.bytes.extend_from_slice(&SERIAL.to_le_bytes());
		self.bytes.extend_from_slice(&self.sequence.to_le_bytes());
		self.bytes.extend_from_slice(&[0; 4]);
		self.bytes.push(lacing.len() as u8);
		self.bytes.extend_from_slice(&lacing);
		for packet in packets {
			self.bytes.extend_from_slice(packet);
		}
		let checksum = crc(&self.bytes[start..]);
		self.bytes[start + 22..start + 26].copy_from_slice(&checksum.to_le_bytes());
		self.sequence += 1;
		Ok(())
	}
}

/// Encode 48 kHz mono PCM as a single-stream Ogg Opus file.
pub fn encode(pcm: &[f32]) -> Result<Vec<u8>, &'static str> {
	const FAILED: &str = "Sound could not be encoded";
	if pcm.is_empty() || pcm.len() > MAX_SAMPLES {
		return Err(FAILED);
	}
	let mut encoder =
		Encoder::new(48_000, Channels::Mono, Application::Audio).map_err(|_| FAILED)?;
	encoder
		.set_bitrate(Bitrate::Bits(BITRATE))
		.map_err(|_| FAILED)?;
	let pre_skip = encoder
		.get_lookahead()
		.ok()
		.and_then(|samples| u16::try_from(samples).ok())
		.ok_or(FAILED)?;
	let mut pages = Pages {
		bytes: Vec::new(),
		sequence: 0,
	};
	let mut head = b"OpusHead".to_vec();
	head.extend_from_slice(&[1, 1]);
	head.extend_from_slice(&pre_skip.to_le_bytes());
	head.extend_from_slice(&48_000u32.to_le_bytes());
	head.extend_from_slice(&[0, 0, 0]);
	pages.page(2, 0, &[head])?;
	let mut tags = b"OpusTags".to_vec();
	tags.extend_from_slice(&6u32.to_le_bytes());
	tags.extend_from_slice(b"serein");
	tags.extend_from_slice(&0u32.to_le_bytes());
	pages.page(0, 0, &[tags])?;

	// The decoder discards `pre_skip` samples, so that much padding follows the clip.
	let total = pcm.len() + usize::from(pre_skip);
	let frames = total.div_ceil(FRAME);
	let mut packets = Vec::new();
	let mut frame = [0.0f32; FRAME];
	let mut encoded = [0u8; 1500];
	for index in 0..frames {
		let start = index * FRAME;
		frame.fill(0.0);
		if start < pcm.len() {
			let end = (start + FRAME).min(pcm.len());
			for (out, sample) in frame.iter_mut().zip(&pcm[start..end]) {
				*out = if sample.is_finite() {
					sample.clamp(-1.0, 1.0)
				} else {
					0.0
				};
			}
		}
		let length = encoder
			.encode_float(&frame, &mut encoded)
			.map_err(|_| FAILED)?;
		packets.push(encoded[..length].to_vec());
		let last = index + 1 == frames;
		// One second per page; the final granule position trims the padding exactly.
		if packets.len() == 50 || last {
			let granule = if last { total } else { (index + 1) * FRAME };
			pages.page(if last { 4 } else { 0 }, granule as u64, &packets)?;
			packets.clear();
		}
	}
	Ok(pages.bytes)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn encoded_clip_is_a_bounded_ogg_opus_stream_that_decodes_to_the_same_length() {
		// 2.5 seconds of a 440 Hz tone: not a whole number of frames or pages.
		let pcm: Vec<f32> = (0..120_123)
			.map(|n| (n as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5)
			.collect();
		let file = encode(&pcm).unwrap();
		assert!(file.starts_with(b"OggS") && file.len() < 64 * 1024);
		assert!(discord_protocol::soundboard::valid_sound_file(
			"audio/ogg",
			&file
		));
		// The published Ogg test vector for the checksum algorithm's parameters.
		assert_eq!(crc(b"123456789"), 0x89a1_897f);
		let decoded = crate::audio::decode_clip(file, MAX_SAMPLES).unwrap();
		assert_eq!(decoded.len(), pcm.len());
		let energy = |samples: &[f32]| samples.iter().map(|s| s * s).sum::<f32>();
		let (input, output) = (energy(&pcm[4800..]), energy(&decoded[4800..]));
		assert!((output / input - 1.0).abs() < 0.2, "{input} {output}");

		assert!(encode(&[]).is_err());
		assert!(encode(&vec![0.0; MAX_SAMPLES + 1]).is_err());
		assert!(encode(&[f32::NAN; 480]).is_ok());
	}
}
