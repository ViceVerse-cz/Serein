//! Small Annex-B H.264 packetizer for a single Go Live video stream.
use std::{collections::VecDeque, time::Duration};
use tokio::time::Instant;

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

/// One bounded access unit, spread across its frame interval instead of a UDP burst.
pub(crate) struct Pacer {
	packets: std::vec::IntoIter<Packet>,
	batch: usize,
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
			batch: 1,
			credit: MTU as f64,
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
	pub fn queue(&mut self, packets: Vec<Packet>, fps: u32, now: Instant) {
		let slots = (500 / fps.clamp(1, 60)).max(1) as usize;
		self.batch = packets.len().div_ceil(slots).max(1);
		self.packets = packets.into_iter();
		self.deadline = now;
		self.progress = now;
	}
	fn refill(&mut self, now: Instant, bitrate: u32) {
		let bytes = now.duration_since(self.updated).as_secs_f64() * f64::from(bitrate) / 8.0;
		self.updated = now;
		// 25% wire headroom includes RTP/AEAD and repairs; no large catch-up bursts.
		self.credit = (self.credit + bytes * 1.25).min((8 * MTU) as f64);
		self.repair_credit = (self.repair_credit + bytes * 0.20).min((2 * MTU) as f64);
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
		self.deadline = (self.deadline + Duration::from_millis(2)).max(now);
		let mut count = 0;
		for packet in self.packets.as_slice().iter().take(self.batch) {
			let bytes = (RTP_HEADER + packet.payload.len() + TRANSPORT_OVERHEAD) as f64;
			if bytes > self.credit {
				break;
			}
			self.credit -= bytes;
			count += 1;
		}
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
			header[1] = (header[1] & 0x80) | 102;
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
			push(&mut packets, sequence, timestamp, ssrc, payload, last)?;
			offset += take;
		}
	}
	if packets.is_empty() {
		return Err("H264 frame has no NAL units");
	}
	Ok(packets)
}

pub(crate) fn validate_source(frame: &[u8]) -> Result<(), &'static str> {
	if frame.len() > 2 * 1024 * 1024 {
		return Err("H264 frame exceeds the sharing limit");
	}
	nalus(frame).map(|_| ())
}

fn push(
	packets: &mut Vec<Packet>,
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
	payload: Vec<u8>,
	marker: bool,
) -> Result<(), &'static str> {
	if packets.len() == MAX_FRAGMENTS {
		return Err("H264 frame exceeds fragment limit");
	}
	let mut header = [0; RTP_HEADER];
	header[0] = 0x80;
	header[1] = 101 | u8::from(marker) << 7;
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
}
