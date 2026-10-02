//! Bounded single-layer AV1 RTP packetization, after codec-aware DAVE encryption.
//!
//! AOM AV1 RTP payload format v1.0.0 sections 4.4–5:
//! https://aomediacodec.github.io/av1-rtp-spec/v1.0.0.html
//! DAVE's AV1 transform removes the final OBU size; Discord's patched receiver
//! preserves that omission so the authenticated frame, including its footer, survives:
//! https://github.com/discord/dave-protocol/blob/main/protocol.md#av1
use crate::video::{MAX_FRAGMENTS, Packet};

const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_DAVE_BYTES: usize = MAX_SOURCE_BYTES + 64 * 1024;
const MTU: usize = 1200;
// RTP header, transport tag/nonce, RTX original sequence and AV1 aggregation header.
const FRAGMENT_BYTES: usize = MTU - 12 - 20 - 2 - 1;
const INVALID: &str = "Invalid AV1 OBU frame";

struct Obu<'a> {
	header: u8,
	extension: Option<u8>,
	payload: &'a [u8],
	size_bytes: usize,
}
impl Obu<'_> {
	fn kind(&self) -> u8 {
		(self.header >> 3) & 15
	}
	fn discarded(&self) -> bool {
		matches!(self.kind(), 2 | 8 | 15)
	}
	fn wire_len(&self) -> usize {
		1 + usize::from(self.extension.is_some()) + self.payload.len()
	}
	/// Copy part of the RTP OBU representation without allocating a full-frame copy.
	fn fragment(&self, start: usize, end: usize, output: &mut Vec<u8>) {
		if start == 0 {
			output.push(self.header & !2);
		}
		if let Some(extension) = self.extension
			&& start <= 1
			&& end > 1
		{
			output.push(extension);
		}
		let header_len = 1 + usize::from(self.extension.is_some());
		if end > header_len {
			output.extend_from_slice(
				&self.payload[start.saturating_sub(header_len)..end - header_len],
			);
		}
	}
}

fn read_size(bytes: &[u8]) -> Result<(usize, usize), &'static str> {
	let mut size = 0u64;
	// AV1 leb128() is limited to eight bytes, including padded encoder output.
	for (index, byte) in bytes.iter().copied().take(8).enumerate() {
		size |= u64::from(byte & 127) << (index * 7);
		if byte & 128 == 0 {
			let size = usize::try_from(size).map_err(|_| INVALID)?;
			return Ok((size, index + 1));
		}
	}
	Err(INVALID)
}

fn write_size(mut size: usize, output: &mut Vec<u8>) {
	loop {
		let mut byte = (size & 127) as u8;
		size >>= 7;
		if size != 0 {
			byte |= 128;
		}
		output.push(byte);
		if size == 0 {
			break;
		}
	}
}

fn size_len(mut size: usize) -> usize {
	let mut length = 1;
	while size >= 128 {
		size >>= 7;
		length += 1;
	}
	length
}

fn obus(frame: &[u8], limit: usize) -> Result<Vec<Obu<'_>>, &'static str> {
	if frame.is_empty() || frame.len() > limit {
		return Err(INVALID);
	}
	let mut output = Vec::new();
	let mut at = 0;
	while at < frame.len() {
		if output.len() == MAX_FRAGMENTS {
			return Err("AV1 frame exceeds OBU limit");
		}
		let header = frame[at];
		at += 1;
		// Reject forbidden/reserved header bits and reserved OBU types.
		if header & 0x81 != 0 || !matches!((header >> 3) & 15, 1..=8 | 15) {
			return Err(INVALID);
		}
		let extension = if header & 4 != 0 {
			let extension = *frame.get(at).ok_or(INVALID)?;
			at += 1;
			// Low three extension bits are reserved by AV1.
			if extension & 7 != 0 {
				return Err(INVALID);
			}
			Some(extension)
		} else {
			None
		};
		let (length, size_bytes) = if header & 2 != 0 {
			let (length, size_bytes) = read_size(&frame[at..])?;
			at += size_bytes;
			(length, size_bytes)
		} else {
			(frame.len() - at, 0)
		};
		let end = at.checked_add(length).ok_or(INVALID)?;
		let payload = frame.get(at..end).ok_or(INVALID)?;
		if payload.is_empty() && !matches!((header >> 3) & 15, 2 | 8 | 15) {
			return Err(INVALID);
		}
		output.push(Obu {
			header,
			extension,
			payload,
			size_bytes,
		});
		at = end;
	}
	Ok(output)
}

/// Normalize bounded encoder OBU-stream output before `Codec::AV1` encryption.
/// In particular, trailing padding/temporal delimiters cannot prevent davey from
/// omitting the size of the last retained OBU. Canonical sizes survive RTP rewriting.
pub(crate) fn normalize_source(frame: &[u8]) -> Result<Vec<u8>, &'static str> {
	let retained: Vec<_> = obus(frame, MAX_SOURCE_BYTES)?
		.into_iter()
		.filter(|obu| !obu.discarded())
		.collect();
	if retained.is_empty() {
		return Err(INVALID);
	}
	let mut output = Vec::with_capacity(frame.len());
	for (index, obu) in retained.iter().enumerate() {
		let last = index + 1 == retained.len();
		output.push((obu.header & !2) | if last { 0 } else { 2 });
		if let Some(extension) = obu.extension {
			output.push(extension);
		}
		if !last {
			write_size(obu.payload.len(), &mut output);
		}
		output.extend_from_slice(obu.payload);
	}
	if output.len() > MAX_SOURCE_BYTES {
		return Err("AV1 frame exceeds the sharing limit");
	}
	Ok(output)
}

/// Admit encoder samples before they enter the bounded screen transport queue.
pub(crate) fn validate_source(frame: &[u8]) -> Result<(), &'static str> {
	let obus = obus(frame, MAX_SOURCE_BYTES)?;
	if !obus.iter().any(|obu| matches!(obu.kind(), 3 | 6)) {
		return Err(INVALID);
	}
	Ok(())
}

/// Packetize the transformed DAVE frame. One OBU or OBU fragment per packet uses
/// W=1, avoiding extra length fields and keeping the final DAVE footer in that OBU.
pub(crate) fn packetize(
	frame: &[u8],
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
	payload_type: u8,
	new_sequence: bool,
) -> Result<Vec<Packet>, &'static str> {
	if payload_type > 127 || ssrc == 0 {
		return Err("Invalid AV1 RTP configuration");
	}
	let obus = obus(frame, MAX_DAVE_BYTES)?;
	// DAVE authenticates the OBU headers and sizes. A receiver must reconstruct
	// canonical sizes except on the final OBU, which includes non-AV1 footer bytes.
	if obus.last().is_none_or(|obu| obu.header & 2 != 0)
		|| obus.iter().any(|obu| {
			obu.discarded()
				|| (obu.size_bytes != 0 && obu.size_bytes != size_len(obu.payload.len()))
		}) {
		return Err(INVALID);
	}
	let count: usize = obus
		.iter()
		.map(|obu| obu.wire_len().div_ceil(FRAGMENT_BYTES))
		.sum();
	if count > MAX_FRAGMENTS {
		return Err("AV1 frame exceeds fragment limit");
	}
	let new_sequence = new_sequence && obus.first().is_some_and(|obu| obu.kind() == 1);
	let mut packets = Vec::with_capacity(count);
	for obu in obus {
		let mut at = 0;
		while at < obu.wire_len() {
			let end = (at + FRAGMENT_BYTES).min(obu.wire_len());
			let first = packets.is_empty();
			let last = packets.len() + 1 == count;
			let mut payload = Vec::with_capacity(end - at + 1);
			payload.push(
				0x10 // W=1: the sole element has no aggregation length field.
					| if at != 0 { 0x80 } else { 0 } // Z: continuation from previous packet.
					| if end != obu.wire_len() { 0x40 } else { 0 } // Y: continues in next.
					| if first && new_sequence { 0x08 } else { 0 },
			);
			obu.fragment(at, end, &mut payload);
			let mut header = [0; 12];
			header[0] = 0x80;
			header[1] = payload_type | if last { 0x80 } else { 0 };
			header[2..4].copy_from_slice(&sequence.to_be_bytes());
			header[4..8].copy_from_slice(&timestamp.to_be_bytes());
			header[8..12].copy_from_slice(&ssrc.to_be_bytes());
			*sequence = sequence.wrapping_add(1);
			packets.push(Packet { header, payload });
			at = end;
		}
	}
	Ok(packets)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{crypto::Dave, test_mls::Delivery};
	// One 64×64 black keyframe produced with libaom/GStreamer av1enc, then decoded
	// with av1dec offline. It includes a temporal delimiter, sequence header and frame.
	const BLACK_KEYFRAME: &[u8] = &[
		0x12, 0x00, 0x0a, 0x0b, 0x00, 0x00, 0x00, 0x02, 0xaf, 0xff, 0xf0, 0x36, 0xbe, 0x40, 0x10,
		0x32, 0x10, 0x10, 0x80, 0x80, 0x01, 0x00, 0x00, 0xb4, 0x51, 0xb4, 0xf0, 0xa1, 0xf1, 0x97,
		0xe0, 0x13, 0x24,
	];

	/// Discord's AV1 receiver restores sizes except for the last reconstructed OBU.
	fn depacketize(packets: &[Packet]) -> Vec<u8> {
		let mut obus = Vec::<Vec<u8>>::new();
		let mut fragmented = false;
		for packet in packets {
			let header = packet.payload[0];
			assert_eq!(header & 0x30, 0x10);
			assert_eq!(header & 0x80 != 0, fragmented);
			if !fragmented {
				obus.push(Vec::new());
			}
			obus.last_mut()
				.unwrap()
				.extend_from_slice(&packet.payload[1..]);
			fragmented = header & 0x40 != 0;
		}
		assert!(!fragmented);
		let mut frame = Vec::new();
		for (index, obu) in obus.iter().enumerate() {
			let last = index + 1 == obus.len();
			let header_len = 1 + usize::from(obu[0] & 4 != 0);
			frame.push(obu[0] | if last { 0 } else { 2 });
			frame.extend_from_slice(&obu[1..header_len]);
			if !last {
				write_size(obu.len() - header_len, &mut frame);
			}
			frame.extend_from_slice(&obu[header_len..]);
		}
		frame
	}

	#[test]
	fn source_normalization_drops_trailing_padding_and_canonicalizes_sizes() {
		let frame = [0x12, 0, 0x0a, 0x81, 0, 7, 0x36, 0, 2, 8, 9, 0x7a, 1, 0];
		assert_eq!(
			normalize_source(&frame).unwrap(),
			[0x0a, 1, 7, 0x34, 0, 8, 9]
		);
		assert!(normalize_source(&[0x12, 0, 0x7a, 0]).is_err());
	}

	#[test]
	fn rtp_fragmentation_preserves_headers_footer_and_sequence_wrap() {
		let mut frame = vec![0x0a, 1, 7, 0x34, 0];
		frame.resize(FRAGMENT_BYTES * 2 + 90, 5);
		let mut sequence = u16::MAX;
		let packets = packetize(&frame, &mut sequence, 90_000, 42, 103, true).unwrap();
		assert_eq!(packets.len(), 4);
		assert_eq!(sequence, 3);
		assert_eq!(packets[0].payload[0], 0x18);
		assert_eq!(packets[1].payload[0], 0x50);
		assert_eq!(packets[2].payload[0], 0xd0);
		assert_eq!(packets[3].payload[0], 0x90);
		assert_eq!(packets[3].header[1], 103 | 0x80);
		assert!(packets[..3].iter().all(|packet| packet.header[1] == 103));
		assert!(
			packets
				.iter()
				.all(|packet| packet.payload.len() + 12 + 20 + 2 <= MTU)
		);
		assert_eq!(depacketize(&packets), frame);
	}

	#[test]
	fn malformed_and_oversized_obus_fail_before_changing_sequence() {
		for frame in [
			vec![],
			vec![0xb0, 1],
			vec![0x35, 0, 1],
			vec![0x34],
			vec![0x34, 7, 1],
			vec![0x32, 0xff, 0xff],
			vec![0x32, 9, 1],
			vec![0x32, 0x81, 0, 1],
			vec![0x32, 1, 1],
			vec![0x78, 1],
			vec![0x30; MAX_DAVE_BYTES + 1],
		] {
			let mut sequence = 77;
			assert!(packetize(&frame, &mut sequence, 0, 42, 103, true).is_err());
			assert_eq!(sequence, 77);
		}
		let mut over_obus = [0x32, 0].repeat(MAX_FRAGMENTS);
		over_obus.push(0x30);
		assert!(normalize_source(&over_obus).is_err());
	}

	#[test]
	fn dave_av1_rtp_round_trip_preserves_authenticated_frame() {
		let server = Delivery::new();
		let mut alice = Dave::new(1, Some(2), 3).unwrap();
		let mut bob = Dave::new(2, Some(1), 3).unwrap();
		alice.session.set_external_sender(&server.external).unwrap();
		bob.session.set_external_sender(&server.external).unwrap();
		let (commit, welcome) = server.add(&mut alice, &bob.key_package().unwrap());
		let mut committed = vec![0, 5];
		committed.extend(commit);
		let mut welcomed = vec![0, 5];
		welcomed.extend(welcome);
		alice.group_changed(29, &committed).unwrap();
		bob.group_changed(30, &welcomed).unwrap();
		alice.execute(5).unwrap();
		bob.execute(5).unwrap();

		let mut original = vec![0x0a, 1, 7, 0x30];
		original.resize(FRAGMENT_BYTES * 2 + 30, 9);
		for source in [BLACK_KEYFRAME, original.as_slice()] {
			validate_source(source).unwrap();
			let original = normalize_source(source).unwrap();
			let encrypted = alice
				.session
				.encrypt(davey::MediaType::VIDEO, davey::Codec::AV1, &original)
				.unwrap()
				.into_owned();
			let packets = packetize(&encrypted, &mut 0, 90_000, 11, 103, true).unwrap();
			let restored = depacketize(&packets);
			assert_eq!(restored, encrypted);
			assert_eq!(
				bob.session
					.decrypt(1, davey::MediaType::VIDEO, &restored)
					.unwrap(),
				original
			);
		}
	}
}
