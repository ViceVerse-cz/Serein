//! Bounded outgoing video packetization, pacing and encrypted repair history.
//! H264 follows RFC 6184, H265 RFC 7798, and AV1 the AV1 RTP payload specification.
//! Discord's codec negotiation and AV1 DAVE framing remain live-unverified.
use model::voice_settings::VideoCodec;
use std::{collections::VecDeque, time::Duration};
use tokio::time::Instant;

#[path = "video_bitstream.rs"]
mod bitstream;

const MTU: usize = 1200;
const RTP_HEADER: usize = 12;
// XChaCha20-Poly1305 tag plus the rtpsize nonce trailer.
const TRANSPORT_OVERHEAD: usize = 20;
// Reserve the original sequence number added by RFC 4588 retransmission.
const MAX_PAYLOAD: usize = MTU - RTP_HEADER - TRANSPORT_OVERHEAD - 2;
pub(crate) const MAX_FRAGMENTS: usize = 2048;
const MAX_DAVE_FRAME: usize = 2 * 1024 * 1024 + 64 * 1024;

pub(crate) struct Packet {
	pub header: [u8; RTP_HEADER],
	pub payload: Vec<u8>,
}

/// libwebrtc's default: the pacer drains at 2.5x the encoder target, so an average frame
/// leaves in well under its frame interval while keyframes still avoid a line-rate burst.
const PACING_FACTOR: f64 = 2.5;
/// Longest sleep the pacer accounts for. A coarse OS timer (Windows' 15.6 ms default tick)
/// or a delayed task sends proportionally more on its next wake instead of losing frames.
const MAX_CATCH_UP: Duration = Duration::from_millis(100);
/// Budget a fresh access unit may send before the first pacing sleep.
const INITIAL_BURST: f64 = (4 * MTU) as f64;

/// One bounded access unit, paced by elapsed time against the encoder target.
pub(crate) struct Pacer {
	packets: std::vec::IntoIter<Packet>,
	credit: f64,
	repair_credit: f64,
	updated: Instant,
	progress: Instant,
	pub deadline: Instant,
}
impl Pacer {
	pub fn new() -> Self {
		Self {
			packets: Vec::new().into_iter(),
			credit: INITIAL_BURST,
			repair_credit: 0.0,
			updated: Instant::now(),
			progress: Instant::now(),
			deadline: Instant::now(),
		}
	}
	pub fn is_empty(&self) -> bool {
		self.packets.len() == 0
	}
	pub fn clear(&mut self) {
		self.packets = Vec::new().into_iter();
	}
	pub fn stale(&self, now: Instant) -> bool {
		!self.is_empty() && now.duration_since(self.progress) >= Duration::from_millis(500)
	}
	pub fn queue(&mut self, packets: Vec<Packet>, now: Instant) {
		// Idle time is not banked into a burst; only the small initial allowance carries over.
		self.credit = self.credit.min(INITIAL_BURST);
		self.updated = now;
		self.packets = packets.into_iter();
		self.deadline = now;
		self.progress = now;
	}
	fn refill(&mut self, now: Instant, bitrate: u32) {
		let elapsed = now
			.saturating_duration_since(self.updated)
			.min(MAX_CATCH_UP)
			.as_secs_f64();
		self.updated = now;
		let target = f64::from(bitrate) / 8.0;
		let added = elapsed * target * PACING_FACTOR;
		// Keep a 5 ms bucket on precise timers; a longer wake may spend what it accrued.
		let limit = (target * PACING_FACTOR * 0.005)
			.max(INITIAL_BURST)
			.max(added);
		self.credit = (self.credit + added).min(limit);
		self.repair_credit = (self.repair_credit + elapsed * target * 0.20).min((2 * MTU) as f64);
	}
	pub fn allow_repair(&mut self, now: Instant, bitrate: u32) -> bool {
		self.refill(now, bitrate);
		if self.credit < MTU as f64 || self.repair_credit < MTU as f64 {
			return false;
		}
		self.credit -= MTU as f64;
		self.repair_credit -= MTU as f64;
		true
	}
	pub fn next_batch(&mut self, now: Instant, bitrate: u32) -> impl Iterator<Item = Packet> + '_ {
		self.refill(now, bitrate);
		let mut count = 0;
		let mut shortfall = 0.0;
		for packet in self.packets.as_slice() {
			let bytes = (RTP_HEADER + packet.payload.len() + TRANSPORT_OVERHEAD) as f64;
			if bytes > self.credit {
				shortfall = bytes - self.credit;
				break;
			}
			self.credit -= bytes;
			count += 1;
		}
		// Sleep until the next media packet, or else a pending repair, is affordable.
		let target = f64::from(bitrate.max(1)) / 8.0;
		let wait = if count < self.packets.len() {
			shortfall / (target * PACING_FACTOR)
		} else {
			((MTU as f64 - self.repair_credit) / (target * 0.20))
				.max((MTU as f64 - self.credit) / (target * PACING_FACTOR))
				.max(0.0)
		};
		self.deadline = now + Duration::from_secs_f64(wait.min(0.1)).max(Duration::from_millis(1));
		// A large IDR at a reduced bitrate needs time to drain. Only a lack of
		// progress expires it, otherwise every replacement IDR could be cut off too.
		if count > 0 {
			self.progress = now;
		}
		self.packets.by_ref().take(count)
	}
}

/// Conservative loss-based control, with REMB as a ceiling and no growth without feedback.
pub(crate) struct Rate {
	pub target: u32,
	maximum: u32,
	at: Instant,
	last_congestion: Instant,
	loss: Option<u8>,
	limit: Option<(u32, Instant)>,
	estimate: Option<u32>,
}
impl Rate {
	pub fn new(maximum: u32, now: Instant) -> Self {
		Self {
			target: maximum,
			maximum,
			at: now,
			last_congestion: now,
			loss: None,
			limit: None,
			estimate: None,
		}
	}
	pub fn observe(&mut self, loss: Option<u8>, estimate: Option<u32>) {
		if let Some(loss) = loss {
			self.loss = Some(self.loss.map_or(loss, |old| old.max(loss)));
		}
		if let Some(estimate) = estimate {
			// REMB describes the wire rate; leave room for RTP/AEAD and bounded repairs.
			let estimate = (u64::from(estimate) * 4 / 5) as u32;
			self.estimate = Some(self.estimate.map_or(estimate, |old| old.min(estimate)));
		}
	}
	pub fn tick(&mut self, now: Instant) -> Option<u32> {
		if now.duration_since(self.at) < Duration::from_secs(1) {
			return None;
		}
		self.at = now;
		let loss = self.loss.take();
		let estimate = self.estimate.take();
		if let Some(estimate) = estimate {
			self.limit = Some((estimate, now));
		}
		let ceiling = self
			.limit
			.filter(|(_, at)| now.duration_since(*at) < Duration::from_secs(5))
			.map_or(self.maximum, |(limit, _)| limit.min(self.maximum))
			.max(250_000.min(self.maximum));
		let mut target = self.target.min(ceiling);
		// Fraction lost is /256. Reduce at >=5%, recover only below 2% for two seconds.
		if loss.is_some_and(|loss| loss >= 13) {
			target = target.saturating_mul(4) / 5;
			self.last_congestion = now;
		} else if target < self.target {
			self.last_congestion = now;
		} else if (loss.is_some_and(|loss| loss <= 5) || (loss.is_none() && estimate.is_some()))
			&& now.duration_since(self.last_congestion) >= Duration::from_secs(2)
		{
			target = target
				.saturating_add((target / 20).max(25_000))
				.min(ceiling);
		}
		target = target.clamp(250_000.min(self.maximum), self.maximum);
		if target == self.target {
			return None;
		}
		self.target = target;
		Some(target)
	}
}

const HISTORY_PACKETS: usize = 2048;
const HISTORY_BYTES: usize = 2 * 1024 * 1024;
const HISTORY_AGE: Duration = Duration::from_secs(1);
struct Sent {
	packet: Packet,
	at: Instant,
	retries: u8,
	last_retry: Option<Instant>,
}
impl Sent {
	fn bytes(&self) -> usize {
		std::mem::size_of::<Self>() + self.packet.payload.capacity()
	}
}

/// DAVE ciphertext only, cleared across every encryption transition.
#[derive(Default)]
pub(crate) struct History {
	packets: VecDeque<Sent>,
	bytes: usize,
	pending: VecDeque<u16>,
}
impl History {
	pub fn clear(&mut self) {
		self.packets.clear();
		self.pending.clear();
		self.bytes = 0;
	}
	pub fn expire(&mut self, now: Instant) {
		while self
			.packets
			.front()
			.is_some_and(|sent| now.duration_since(sent.at) >= HISTORY_AGE)
		{
			self.bytes -= self.packets.pop_front().unwrap().bytes();
		}
		if self.packets.is_empty() {
			self.pending.clear();
		}
	}
	pub fn remember(&mut self, packet: Packet, now: Instant) {
		self.expire(now);
		let sent = Sent {
			packet,
			at: now,
			retries: 0,
			last_retry: None,
		};
		let bytes = sent.bytes();
		if bytes > HISTORY_BYTES {
			return;
		}
		while self.packets.len() >= HISTORY_PACKETS || self.bytes + bytes > HISTORY_BYTES {
			self.bytes -= self.packets.pop_front().unwrap().bytes();
		}
		self.bytes += bytes;
		self.packets.push_back(sent);
	}
	/// A cache miss needs an IDR instead of resending media outside the repair window.
	pub fn request(&mut self, sequences: &[u16], now: Instant) -> bool {
		self.expire(now);
		let mut missing = false;
		for &seq in sequences.iter().take(128) {
			let Some(sent) = self
				.packets
				.iter()
				.find(|sent| sent.packet.header[2..4] == seq.to_be_bytes())
			else {
				missing = true;
				continue;
			};
			if sent.retries >= 2 {
				missing = true;
				continue;
			}
			if self.pending.len() < 128
				&& !self.pending.contains(&seq)
				&& sent
					.last_retry
					.is_none_or(|at| now.duration_since(at) >= Duration::from_millis(50))
			{
				self.pending.push_back(seq);
			}
		}
		missing
	}
	pub fn has_pending(&self) -> bool {
		!self.pending.is_empty()
	}
	pub fn repair(&mut self, ssrc: u32, sequence: &mut u16, now: Instant) -> Option<Packet> {
		self.expire(now);
		if ssrc == 0 {
			return None;
		}
		// Drain stale requests in this bounded pass, rather than charging an MTU
		// of repair budget for each one and blocking newer useful retransmissions.
		while let Some(seq) = self.pending.pop_front() {
			let Some(sent) = self
				.packets
				.iter_mut()
				.find(|sent| sent.packet.header[2..4] == seq.to_be_bytes())
			else {
				continue;
			};
			if sent.retries >= 2 {
				continue;
			}
			sent.retries += 1;
			sent.last_retry = Some(now);
			let mut header = sent.packet.header;
			let repair_type = match header[1] & 127 {
				101 => 102,
				103 => 104,
				109 => 110,
				_ => continue,
			};
			header[1] = (header[1] & 0x80) | repair_type;
			header[2..4].copy_from_slice(&sequence.to_be_bytes());
			header[8..12].copy_from_slice(&ssrc.to_be_bytes());
			*sequence = sequence.wrapping_add(1);
			let mut payload = Vec::with_capacity(sent.packet.payload.len() + 2);
			payload.extend_from_slice(&seq.to_be_bytes());
			payload.extend_from_slice(&sent.packet.payload);
			return Some(Packet { header, payload });
		}
		None
	}
}

/// Packetize an already DAVE-encrypted Annex-B H.264 frame as RFC 6184 NAL/FU-A RTP.
pub(crate) fn packetize(
	frame: &[u8],
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
) -> Result<Vec<Packet>, &'static str> {
	if frame.len() > MAX_DAVE_FRAME {
		return Err("DAVE H264 frame exceeds the sharing limit");
	}
	let nalus = nalus(frame)?;
	let mut packets = Vec::new();
	for (index, nalu) in nalus.iter().enumerate() {
		if nalu.is_empty() {
			continue;
		}
		let last_nalu = index + 1 == nalus.len();
		if nalu.len() <= MAX_PAYLOAD {
			push(
				&mut packets,
				sequence,
				timestamp,
				ssrc,
				nalu.to_vec(),
				last_nalu,
				VideoCodec::H264,
			)?;
			continue;
		}
		if nalu.len() < 2 {
			return Err("Invalid H264 NAL unit");
		}
		let indicator = (nalu[0] & 0xe0) | 28;
		let kind = nalu[0] & 0x1f;
		let mut offset = 1;
		while offset < nalu.len() {
			let take = (nalu.len() - offset).min(MAX_PAYLOAD - 2);
			let mut payload = Vec::with_capacity(take + 2);
			payload.push(indicator);
			payload.push(
				kind | if offset == 1 { 0x80 } else { 0 }
					| if offset + take == nalu.len() { 0x40 } else { 0 },
			);
			payload.extend_from_slice(&nalu[offset..offset + take]);
			let last = last_nalu && offset + take == nalu.len();
			push(
				&mut packets,
				sequence,
				timestamp,
				ssrc,
				payload,
				last,
				VideoCodec::H264,
			)?;
			offset += take;
		}
	}
	if packets.is_empty() {
		return Err("H264 frame has no NAL units");
	}
	Ok(packets)
}

pub(crate) fn validate_source(frame: &[u8]) -> Result<(), &'static str> {
	validate_source_for_codec(frame, VideoCodec::H264)
}

pub(crate) fn validate_source_for_codec(
	frame: &[u8],
	codec: VideoCodec,
) -> Result<(), &'static str> {
	bitstream::validate(frame, codec)
}
pub(crate) fn has_parameter_sets_for_codec(frame: &[u8], codec: VideoCodec) -> bool {
	bitstream::has_parameters(frame, codec)
}
pub(crate) fn is_keyframe_for_codec(frame: &[u8], codec: VideoCodec) -> bool {
	bitstream::is_keyframe(frame, codec)
}
pub(crate) fn prepare_source(
	frame: &[u8],
	codec: VideoCodec,
) -> Result<std::borrow::Cow<'_, [u8]>, &'static str> {
	bitstream::prepare(frame, codec)
}
pub(crate) fn codec_name(codec: VideoCodec) -> &'static str {
	match codec {
		VideoCodec::H264 => "H264",
		VideoCodec::H265 => "H265",
		VideoCodec::Av1 => "AV1",
	}
}
pub(crate) fn payload_type(codec: VideoCodec) -> u8 {
	match codec {
		VideoCodec::H264 => 101,
		VideoCodec::H265 => 103,
		VideoCodec::Av1 => 109,
	}
}
pub(crate) fn dave_codec(codec: VideoCodec) -> davey::Codec {
	match codec {
		VideoCodec::H264 => davey::Codec::H264,
		VideoCodec::H265 => davey::Codec::H265,
		VideoCodec::Av1 => davey::Codec::AV1,
	}
}

/// Packetize a DAVE encrypted access unit. `keyframe` comes from the validated
/// encoder result; AV1's N bit additionally requires an initial sequence header.
pub(crate) fn packetize_for_codec(
	frame: &[u8],
	codec: VideoCodec,
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
	keyframe: bool,
) -> Result<Vec<Packet>, &'static str> {
	if frame.len() > MAX_DAVE_FRAME {
		return Err("DAVE video frame exceeds the sharing limit");
	}
	match codec {
		VideoCodec::H264 => packetize(frame, sequence, timestamp, ssrc),
		VideoCodec::H265 => packetize_h265(frame, sequence, timestamp, ssrc),
		VideoCodec::Av1 => packetize_av1(frame, sequence, timestamp, ssrc, keyframe),
	}
}

fn packetize_h265(
	frame: &[u8],
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
) -> Result<Vec<Packet>, &'static str> {
	let units = bitstream::nalus(frame, VideoCodec::H265)?;
	let mut packets = Vec::new();
	for (index, unit) in units.iter().enumerate() {
		let last = index + 1 == units.len();
		if unit.len() <= MAX_PAYLOAD {
			push(
				&mut packets,
				sequence,
				timestamp,
				ssrc,
				unit.to_vec(),
				last,
				VideoCodec::H265,
			)?;
			continue;
		}
		let chunks = unit[2..].chunks(MAX_PAYLOAD - 3);
		let count = chunks.len();
		for (index, chunk) in chunks.enumerate() {
			let mut payload = Vec::with_capacity(chunk.len() + 3);
			// FU payload header retains F, layer_id and temporal_id_plus1.
			payload.extend([(unit[0] & 0x81) | (49 << 1), unit[1]]);
			payload.push(
				(unit[0] >> 1 & 63)
					| if index == 0 { 128 } else { 0 }
					| if index + 1 == count { 64 } else { 0 },
			);
			payload.extend_from_slice(chunk);
			push(
				&mut packets,
				sequence,
				timestamp,
				ssrc,
				payload,
				last && index + 1 == count,
				VideoCodec::H265,
			)?;
		}
	}
	Ok(packets)
}

fn packetize_av1(
	frame: &[u8],
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
	keyframe: bool,
) -> Result<Vec<Packet>, &'static str> {
	let units = bitstream::obus(frame, false)?;
	if units.iter().any(bitstream::Obu::discarded) {
		return Err("DAVE AV1 frame contains discarded OBUs");
	}
	let mut packets = Vec::new();
	for (unit_index, unit) in units.iter().enumerate() {
		// One OBU element per packet: W=1 omits its redundant element length.
		// RTP removes OBU size fields; the authenticated final DAVE trailer stays
		// inside the final size-less OBU, including when it is fragmented.
		let mut element = Vec::with_capacity(2 + unit.payload.len());
		element.push(unit.header & !2);
		if let Some(extension) = unit.extension {
			element.push(extension);
		}
		element.extend_from_slice(unit.payload);
		let chunks = element.chunks(MAX_PAYLOAD - 1);
		let count = chunks.len();
		for (index, chunk) in chunks.enumerate() {
			let mut payload = Vec::with_capacity(1 + chunk.len());
			let continues = index > 0;
			let remaining = index + 1 < count;
			let new_sequence = packets.is_empty() && unit.kind() == 1 && keyframe;
			payload.push(
				0x10 | if continues { 0x80 } else { 0 }
					| if remaining { 0x40 } else { 0 }
					| if new_sequence { 8 } else { 0 },
			);
			payload.extend_from_slice(chunk);
			push(
				&mut packets,
				sequence,
				timestamp,
				ssrc,
				payload,
				unit_index + 1 == units.len() && !remaining,
				VideoCodec::Av1,
			)?;
		}
	}
	Ok(packets)
}

fn push(
	packets: &mut Vec<Packet>,
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
	payload: Vec<u8>,
	marker: bool,
	codec: VideoCodec,
) -> Result<(), &'static str> {
	if packets.len() == MAX_FRAGMENTS {
		return Err("Video frame exceeds fragment limit");
	}
	let mut header = [0; RTP_HEADER];
	header[0] = 0x80;
	header[1] = payload_type(codec) | u8::from(marker) << 7;
	header[2..4].copy_from_slice(&sequence.to_be_bytes());
	header[4..8].copy_from_slice(&timestamp.to_be_bytes());
	header[8..12].copy_from_slice(&ssrc.to_be_bytes());
	*sequence = sequence.wrapping_add(1);
	packets.push(Packet { header, payload });
	Ok(())
}

fn nalus(frame: &[u8]) -> Result<Vec<&[u8]>, &'static str> {
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
		if starts.len() == MAX_FRAGMENTS {
			return Err("H264 frame exceeds fragment limit");
		}
		starts.push((at, size));
		at += size;
	}
	if starts.is_empty() || starts[0].0 != 0 {
		return Err("H264 frame is not Annex-B");
	}
	let mut out = Vec::with_capacity(starts.len());
	for (index, (start, size)) in starts.iter().enumerate() {
		let end = starts.get(index + 1).map_or(frame.len(), |next| next.0);
		if start + size >= end {
			return Err("H264 frame contains an empty NAL unit");
		}
		out.push(&frame[start + size..end]);
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{crypto::Dave, test_mls::Delivery};

	fn depacketize(packets: &[Packet]) -> Vec<u8> {
		let mut frame = Vec::new();
		let mut fragmented = false;
		for packet in packets {
			if packet.payload[0] & 0x1f == 28 {
				if packet.payload[1] & 0x80 != 0 {
					assert!(!fragmented);
					frame.extend([0, 0, 0, 1]);
					frame.push((packet.payload[0] & 0xe0) | (packet.payload[1] & 0x1f));
					fragmented = true;
				} else {
					assert!(fragmented);
				}
				frame.extend_from_slice(&packet.payload[2..]);
				if packet.payload[1] & 0x40 != 0 {
					fragmented = false;
				}
			} else {
				assert!(!fragmented);
				frame.extend([0, 0, 0, 1]);
				frame.extend_from_slice(&packet.payload);
			}
		}
		assert!(!fragmented);
		frame
	}
	#[test]
	fn fragments_annex_b_h264_with_a_final_marker() {
		assert!(validate_source(&[0, 0, 0, 1, 0x65, 1, 0, 0, 1]).is_err());
		let mut frame = vec![0, 0, 0, 1, 0x65];
		frame.resize(frame.len() + MAX_PAYLOAD * 2, 7);
		frame.extend([0, 0, 1, 0x41, 9]);
		let mut sequence = 7;
		let packets = packetize(&frame, &mut sequence, 90_000, 11).unwrap();
		assert!(packets.len() > 2 && packets.len() <= MAX_FRAGMENTS);
		assert!(
			packets[..packets.len() - 1]
				.iter()
				.all(|packet| packet.header[1] & 0x80 == 0)
		);
		assert_eq!(packets.last().unwrap().header[1], 101 | 0x80);
		assert!(
			packets
				.iter()
				.all(
					|packet| packet.header.len() + packet.payload.len() + TRANSPORT_OVERHEAD <= MTU
				)
		);
		assert_eq!(packets[0].payload[0] & 0x1f, 28);
		assert_ne!(packets[0].payload[1] & 0x80, 0);
		let mut rebuilt = Vec::new();
		for packet in packets
			.iter()
			.take_while(|packet| packet.payload[0] & 0x1f == 28)
		{
			if packet.payload[1] & 0x80 != 0 {
				rebuilt.push((packet.payload[0] & 0xe0) | (packet.payload[1] & 0x1f));
			}
			rebuilt.extend_from_slice(&packet.payload[2..]);
		}
		assert_eq!(rebuilt, frame[4..5 + MAX_PAYLOAD * 2]);
	}
	#[test]
	fn dave_h264_ciphertext_packetizes_without_extra_start_codes() {
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
		assert_eq!(alice.group_changed(29, &committed).unwrap(), 5);
		assert_eq!(bob.group_changed(30, &welcomed).unwrap(), 5);
		alice.execute(5).unwrap();
		bob.execute(5).unwrap();

		let mut original = vec![0, 0, 0, 1, 0x67, 0x64, 0, 0x1f, 0xac, 0xd9, 0x40];
		original.extend([0, 0, 0, 1, 0x65, 0x88]);
		original.resize(original.len() + MAX_PAYLOAD * 2, 7);
		let encrypted = alice
			.session
			.encrypt(davey::MediaType::VIDEO, davey::Codec::H264, &original)
			.unwrap()
			.into_owned();
		assert_eq!(nalus(&encrypted).unwrap().len(), 2);
		let mut sequence = 0;
		let packets = packetize(&encrypted, &mut sequence, 90_000, 11).unwrap();
		let restored = depacketize(&packets);
		assert_eq!(restored, encrypted);
		assert_eq!(
			bob.session
				.decrypt(1, davey::MediaType::VIDEO, &restored)
				.unwrap(),
			original
		);
	}

	fn peers() -> (Dave, Dave) {
		let server = Delivery::new();
		let mut alice = Dave::new(1, Some(2), 3).unwrap();
		let mut bob = Dave::new(2, Some(1), 3).unwrap();
		alice.session.set_external_sender(&server.external).unwrap();
		bob.session.set_external_sender(&server.external).unwrap();
		let (commit, welcome) = server.add(&mut alice, &bob.key_package().unwrap());
		alice
			.group_changed(29, &[&[0, 5], commit.as_slice()].concat())
			.unwrap();
		bob.group_changed(30, &[&[0, 5], welcome.as_slice()].concat())
			.unwrap();
		alice.execute(5).unwrap();
		bob.execute(5).unwrap();
		(alice, bob)
	}

	fn av1_unit(kind: u8, payload: &[u8], bytes: &mut Vec<u8>) {
		bytes.push(kind << 3 | 2);
		let mut size = payload.len();
		while size >= 128 {
			bytes.push((size as u8 & 127) | 128);
			size >>= 7;
		}
		bytes.push(size as u8);
		bytes.extend_from_slice(payload);
	}

	/// Synthetic wire reconstruction, including DAVE's size-less final AV1 OBU.
	/// This fixture does not represent an incoming H265/AV1 decoder or live Discord.
	fn rebuild(packets: &[Packet], codec: VideoCodec) -> Vec<u8> {
		if codec == VideoCodec::H264 {
			return depacketize(packets);
		}
		let mut output = Vec::new();
		if codec == VideoCodec::H265 {
			let mut fragmented = false;
			for packet in packets {
				let payload = &packet.payload;
				if payload[0] >> 1 & 63 == 49 {
					if payload[2] & 128 != 0 {
						assert!(!fragmented);
						output.extend([
							0,
							0,
							0,
							1,
							(payload[0] & 0x81) | (payload[2] & 63) << 1,
							payload[1],
						]);
						fragmented = true;
					} else {
						assert!(fragmented);
					}
					output.extend_from_slice(&payload[3..]);
					if payload[2] & 64 != 0 {
						fragmented = false;
					}
				} else {
					assert!(!fragmented);
					output.extend([0, 0, 0, 1]);
					output.extend_from_slice(payload);
				}
			}
			assert!(!fragmented);
			return output;
		}
		let mut elements: Vec<Vec<u8>> = Vec::new();
		let mut continuation = false;
		for packet in packets {
			let flags = packet.payload[0];
			assert_eq!(flags & 0x30, 0x10); // W=1: one element without length.
			assert_eq!(flags & 0x80 != 0, continuation);
			if !continuation {
				elements.push(Vec::new());
			}
			elements
				.last_mut()
				.unwrap()
				.extend_from_slice(&packet.payload[1..]);
			continuation = flags & 0x40 != 0;
		}
		assert!(!continuation);
		for (index, element) in elements.iter().enumerate() {
			let prefix = 1 + usize::from(element[0] & 4 != 0);
			if index + 1 == elements.len() {
				output.extend_from_slice(element);
			} else {
				let start = output.len();
				av1_unit(element[0] >> 3 & 15, &element[prefix..], &mut output);
				if prefix == 2 {
					output[start] |= 4;
					output.insert(start + 1, element[1]);
				}
			}
		}
		output
	}

	#[test]
	fn h265_and_av1_survive_fragmentation_dave_and_transport_encryption() {
		for codec in [VideoCodec::H265, VideoCodec::Av1] {
			let (mut alice, mut bob) = peers();
			let mut original = Vec::new();
			if codec == VideoCodec::H265 {
				for kind in [32, 33, 34] {
					original.extend([0, 0, 1, kind << 1, 1, 0x80]);
				}
				original.extend([0, 0, 1, 19 << 1, 1, 0x88]);
				original.resize(original.len() + MAX_PAYLOAD * 3, 7);
			} else {
				av1_unit(2, &[], &mut original);
				// Padded size is canonicalized before it becomes authenticated.
				original.extend([0x0e, 0x20, 0x81, 0, 0]);
				let mut picture = vec![7; MAX_PAYLOAD * 3];
				picture[0] = 0x10; // show_existing=0, KEY_FRAME, show_frame=1.
				av1_unit(6, &picture, &mut original);
				av1_unit(15, &[0; 8], &mut original);
			}
			validate_source_for_codec(&original, codec).unwrap();
			assert!(has_parameter_sets_for_codec(&original, codec));
			assert!(is_keyframe_for_codec(&original, codec));
			let prepared = prepare_source(&original, codec).unwrap();
			let encrypted = alice
				.session
				.encrypt(davey::MediaType::VIDEO, dave_codec(codec), &prepared)
				.unwrap();
			let mut sequence = u16::MAX;
			let packets =
				packetize_for_codec(&encrypted, codec, &mut sequence, 9000, 11, true).unwrap();
			assert!(packets.len() > 3 && packets.len() < MAX_FRAGMENTS);
			let mut encryption = crate::crypto::Encryption::new(&[7; 32]);
			for (index, packet) in packets.iter().enumerate() {
				assert_eq!(packet.header[1] & 127, payload_type(codec));
				assert_eq!(packet.header[1] & 128 != 0, index + 1 == packets.len());
				assert_eq!(
					u16::from_be_bytes(packet.header[2..4].try_into().unwrap()),
					u16::MAX.wrapping_add(index as u16)
				);
				let wire = encryption.seal(&packet.header, &packet.payload).unwrap();
				assert!(wire.len() <= MTU);
				let opened = encryption.open(&wire).unwrap();
				assert_eq!(opened.payload_type, payload_type(codec));
				assert_eq!(opened.payload, packet.payload);
			}
			if codec == VideoCodec::Av1 {
				assert_eq!(packets[0].payload[0] & 8, 8);
				assert!(packets[1..].iter().all(|packet| packet.payload[0] & 8 == 0));
			}
			let restored = rebuild(&packets, codec);
			assert_eq!(restored, encrypted.as_ref());
			let decrypted = bob
				.session
				.decrypt(1, davey::MediaType::VIDEO, &restored)
				.unwrap();
			if codec == VideoCodec::H265 {
				assert_eq!(decrypted.as_slice(), prepared.as_ref());
			} else {
				let units = bitstream::obus(&decrypted, false).unwrap();
				assert_eq!(units.len(), 2);
				assert_eq!(units[0].payload, &[0]);
				assert_eq!(
					units[1].payload,
					&prepared[prepared.len() - MAX_PAYLOAD * 3..]
				);
			}
		}
	}

	#[test]
	fn malformed_headers_sizes_and_unit_budgets_are_rejected() {
		for bytes in [
			vec![0, 0, 1, 38, 0, 7],
			vec![0, 0, 1, 38],
			vec![0, 0, 1, 0x80, 1, 7],
		] {
			assert!(validate_source_for_codec(&bytes, VideoCodec::H265).is_err());
		}
		for bytes in [
			vec![0x32, 9, 1],
			vec![0x34],
			vec![0xb2, 1, 7],
			vec![0x32, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80],
			vec![0x30, 7],
		] {
			assert!(validate_source_for_codec(&bytes, VideoCodec::Av1).is_err());
		}
		let mut headers = Vec::new();
		for _ in 0..2049 {
			av1_unit(6, &[7], &mut headers);
		}
		assert!(validate_source_for_codec(&headers, VideoCodec::Av1).is_err());
		let mut frame = vec![0, 0, 0, 1, 0x65];
		frame.resize(MAX_PAYLOAD + 10, 7);
		for _ in 0..MAX_FRAGMENTS - 1 {
			frame.extend([0, 0, 1, 0x41, 7]);
		}
		assert!(packetize(&frame, &mut 0, 0, 1).is_err());
		let oversized = vec![0; MAX_DAVE_FRAME + 1];
		for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1] {
			assert!(packetize_for_codec(&oversized, codec, &mut 0, 0, 1, false).is_err());
		}
	}

	#[test]
	fn av1_sequence_headers_alone_do_not_claim_a_keyframe() {
		let sequence_only = [0x0a, 1, 0];
		assert!(!is_keyframe_for_codec(&sequence_only, VideoCodec::Av1));
		for header in [0x20, 0x80] {
			let bytes = [0x0a, 1, 0, 0x32, 1, header];
			assert!(has_parameter_sets_for_codec(&bytes, VideoCodec::Av1));
			assert!(!is_keyframe_for_codec(&bytes, VideoCodec::Av1));
		}
		let reduced = [0x0a, 1, 0x18, 0x32, 1, 0x80];
		assert!(is_keyframe_for_codec(&reduced, VideoCodec::Av1));
		let regular = [0x0a, 1, 0, 0x32, 1, 0x10];
		let encrypted_style = [0x0a, 1, 7, 0x30, 8, 9];
		let packets =
			packetize_for_codec(&encrypted_style, VideoCodec::Av1, &mut 0, 0, 1, false).unwrap();
		assert!(packets.iter().all(|packet| packet.payload[0] & 8 == 0));
		assert!(is_keyframe_for_codec(&regular, VideoCodec::Av1));
	}

	#[test]
	fn repairs_use_each_negotiated_codecs_rtx_payload_type() {
		for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1] {
			let now = Instant::now();
			let mut history = History::default();
			let mut packets = Vec::new();
			push(&mut packets, &mut 9, 3000, 7, vec![42, 43], true, codec).unwrap();
			history.remember(packets.pop().unwrap(), now);
			assert!(!history.request(&[9], now));
			let mut repair_sequence = u16::MAX;
			let repair = history.repair(8, &mut repair_sequence, now).unwrap();
			assert_eq!(repair.header[1], (payload_type(codec) + 1) | 128);
			assert_eq!(&repair.header[8..], &8u32.to_be_bytes());
			assert_eq!(repair_sequence, 0);
			assert_eq!(repair.payload, [0, 9, 42, 43]);
			let mut encryption = crate::crypto::Encryption::new(&[7; 32]);
			let wire = encryption.seal(&repair.header, &repair.payload).unwrap();
			assert_eq!(
				encryption.open(&wire).unwrap().payload_type,
				payload_type(codec) + 1
			);
		}
	}
}
