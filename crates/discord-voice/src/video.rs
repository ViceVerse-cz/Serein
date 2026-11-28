//! Small Annex-B H.264 and AOM AV1 packetizers for a single Go Live video stream.
use std::{collections::VecDeque, time::Duration};
use tokio::time::Instant;

const MTU: usize = 1200;
pub(crate) const RTP_HEADER: usize = 12;
// XChaCha20-Poly1305 tag plus the rtpsize nonce trailer.
const TRANSPORT_OVERHEAD: usize = 20;
// Reserve the original sequence number added by RFC 4588 retransmission.
pub(crate) const MAX_PAYLOAD: usize = MTU - RTP_HEADER - TRANSPORT_OVERHEAD - 2;
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
	pub fn repair(
		&mut self,
		ssrc: u32,
		sequence: &mut u16,
		now: Instant,
		rtx_payload_type: u8,
	) -> Option<Packet> {
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
			header[1] = (header[1] & 0x80) | rtx_payload_type;
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
				101,
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
			push(&mut packets, sequence, timestamp, ssrc, payload, last, 101)?;
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

/// One AV1 OBU inside an encoded frame: its header, optional LEB128 size and payload.
fn obus_av1(data: &[u8]) -> Option<Vec<(usize, usize)>> {
	let mut spans = Vec::new();
	let mut at = 0;
	while at < data.len() {
		if spans.len() == 256 {
			return None;
		}
		let rest = &data[at..];
		let header = *rest.first()?;
		// Forbidden bit, reserved OBU types and non-zero extension bits are not encoder output.
		if header & 0x81 != 0 || matches!((header >> 3) & 15, 0 | 8) {
			return None;
		}
		let head = if header & 4 != 0 {
			if rest.get(1)? & 31 != 0 {
				return None;
			}
			2
		} else {
			1
		};
		let (len, prefix) = if header & 2 != 0 {
			let mut value = 0usize;
			let mut used = 0;
			loop {
				let byte = *rest.get(head + used)?;
				used += 1;
				if used > 4 {
					return None;
				}
				value = value.checked_add(usize::from(byte & 127) << (7 * (used - 1)))?;
				if byte & 128 == 0 {
					break;
				}
			}
			(value, used)
		} else {
			(rest.len() - head, 0)
		};
		let end = head.checked_add(prefix)?.checked_add(len)?;
		if end == 0 || end > rest.len() {
			return None;
		}
		spans.push((at, at + end));
		at += end;
	}
	(!spans.is_empty()).then_some(spans)
}

/// The framing the AV1 receiver rebuilds before decoding: every OBU keeps its size field except
/// the last one, whose size the frame boundary implies. DAVE encrypts exactly these bytes, so the
/// sender has to produce what the receiver reconstructs.
pub(crate) fn av1_sender_frame(data: &[u8]) -> Result<Vec<u8>, &'static str> {
	const INVALID: &str = "Invalid AV1 frame";
	// Temporal delimiters, tile lists and padding carry no coded data: DAVE drops them before it
	// encrypts, so they must not stand between the last real OBU and the frame end here either.
	// Left in, the last surviving OBU would keep a size field and the protocol data DAVE appends
	// would be parsed as one more OBU.
	const DROPPED: [u8; 3] = [2, 8, 15];
	let mut spans = obus_av1(data).ok_or(INVALID)?;
	spans.retain(|(start, _)| !DROPPED.contains(&((data[*start] >> 3) & 15)));
	let count = spans.len();
	if count == 0 {
		return Err(INVALID);
	}
	let mut frame = Vec::with_capacity(data.len());
	for (index, (start, end)) in spans.into_iter().enumerate() {
		let obu = &data[start..end];
		let header = obu[0];
		let head = if header & 4 != 0 { 2 } else { 1 };
		// Drop the size field of the final OBU and clear its "has size" bit.
		let (size_prefix, payload) = if index + 1 == count {
			let mut used = 0;
			while header & 2 != 0 && obu.get(head + used).is_some_and(|byte| byte & 128 != 0) {
				used += 1;
			}
			(
				0,
				if header & 2 != 0 {
					head + used + 1
				} else {
					head
				},
			)
		} else {
			(head, 0)
		};
		if frame.len() + obu.len() + 8 > MAX_DAVE_FRAME {
			return Err("AV1 frame exceeds the sharing limit");
		}
		frame.push(if index + 1 == count {
			header & !2
		} else {
			header
		});
		frame.extend_from_slice(&obu[1..head]);
		if index + 1 == count {
			frame.extend_from_slice(&obu[payload..]);
		} else {
			frame.extend_from_slice(&obu[size_prefix..]);
		}
	}
	Ok(frame)
}

/// Packetize an already DAVE-encrypted AV1 frame as AOM AV1 RTP v1.0.0 payloads: one aggregation
/// header per packet, then size-prefixed elements, `Z`/`Y` marking an element that continues
/// across packets, and the marker bit on the final packet.
pub(crate) fn packetize_av1(
	frame: &[u8],
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
) -> Result<Vec<Packet>, &'static str> {
	if frame.len() > MAX_DAVE_FRAME {
		return Err("DAVE AV1 frame exceeds the sharing limit");
	}
	let spans = obus_av1(frame).ok_or("Invalid AV1 frame")?;
	// `N` marks the packet carrying the start of a coded video sequence, which is the packet that
	// carries the sequence header first; the receiver resets its dependency state on it.
	let starts_sequence = spans
		.first()
		.is_some_and(|(start, _)| (frame[*start] >> 3) & 15 == 1);
	// The receiver refuses a payload with more than 64 elements, so the sender flushes before
	// packing the 65th even when the bytes still fit.
	const MAX_ELEMENTS: u8 = 64;
	let mut packets = Vec::new();
	let mut payload = vec![0u8; 1];
	let mut elements = 0u8;
	for (start, end) in spans {
		let element = &frame[start..end];
		let mut prefix = [0u8; 3];
		let prefix = size_bytes(element.len(), &mut prefix);
		if 1 + prefix.len() + element.len() <= MAX_PAYLOAD {
			if payload.len() + prefix.len() + element.len() > MAX_PAYLOAD
				|| elements == MAX_ELEMENTS
			{
				flush_av1(
					&mut payload,
					&mut elements,
					&mut packets,
					sequence,
					timestamp,
					ssrc,
				)?;
			}
			payload.extend_from_slice(prefix);
			payload.extend_from_slice(element);
			elements = elements.saturating_add(1);
			continue;
		}
		// A single OBU larger than one payload is fragmented across packets, each carrying one
		// element, with `Z` continuing the previous one and `Y` continuing into the next.
		let take = MAX_PAYLOAD - 1 - prefix.len();
		flush_av1(
			&mut payload,
			&mut elements,
			&mut packets,
			sequence,
			timestamp,
			ssrc,
		)?;
		let mut offset = 0;
		while offset < element.len() {
			let mut fragment = vec![0u8; 1];
			let chunk = (element.len() - offset).min(take);
			let last = offset + chunk == element.len();
			fragment[0] = u8::from(offset > 0) << 7 | u8::from(!last) << 6;
			let mut chunk_prefix = [0u8; 3];
			fragment.extend_from_slice(size_bytes(chunk, &mut chunk_prefix));
			fragment.extend_from_slice(&element[offset..offset + chunk]);
			push(
				&mut packets,
				sequence,
				timestamp,
				ssrc,
				fragment,
				false,
				105,
			)?;
			offset += chunk;
		}
	}
	flush_av1(
		&mut payload,
		&mut elements,
		&mut packets,
		sequence,
		timestamp,
		ssrc,
	)?;
	if packets.is_empty() {
		return Err("AV1 frame has no OBUs");
	}
	if starts_sequence {
		packets[0].payload[0] |= 0x08;
	}
	let last = packets.len() - 1;
	packets[last].header[1] |= 0x80;
	Ok(packets)
}

/// Emits the AV1 payload built so far and starts an empty one.
fn flush_av1(
	payload: &mut Vec<u8>,
	elements: &mut u8,
	packets: &mut Vec<Packet>,
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
) -> Result<(), &'static str> {
	if *elements == 0 {
		return Ok(());
	}
	// `W` stays zero: every element then carries its own size, so the receiver never has to
	// infer the last one's length from the element count.
	payload[0] = 0;
	let filled = std::mem::replace(payload, vec![0u8; 1]);
	*elements = 0;
	push(packets, sequence, timestamp, ssrc, filled, false, 105)
}

/// LEB128 element size, written into `out`; returns the written slice.
fn size_bytes(value: usize, out: &mut [u8; 3]) -> &[u8] {
	let mut length = 0;
	let mut value = value;
	loop {
		let byte = (value & 127) as u8;
		value >>= 7;
		out[length] = if value == 0 { byte } else { byte | 128 };
		length += 1;
		if value == 0 || length == out.len() {
			return &out[..length];
		}
	}
}

fn push(
	packets: &mut Vec<Packet>,
	sequence: &mut u16,
	timestamp: u32,
	ssrc: u32,
	payload: Vec<u8>,
	marker: bool,
	payload_type: u8,
) -> Result<(), &'static str> {
	if packets.len() == MAX_FRAGMENTS {
		return Err("Frame exceeds fragment limit");
	}
	let mut header = [0; RTP_HEADER];
	header[0] = 0x80;
	header[1] = payload_type | u8::from(marker) << 7;
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

	/// A sized sequence header followed by a sized frame header, as an encoder emits them.
	const AV1_ENCODER_FRAME: [u8; 9] = [0x0a, 2, 0x20, 0x10, 0x32, 3, 0x20, 0x30, 0x10];

	fn av1_packets(frame: &[u8]) -> Vec<Packet> {
		let mut sequence = 0u16;
		packetize_av1(frame, &mut sequence, 7, 9).expect("packetized")
	}

	/// The receiver this stream's own viewers run, used as the oracle for the wire format.
	fn av1_rebuild(packets: &[Packet]) -> Vec<u8> {
		let mut frame = Vec::new();
		let mut fragments = crate::video_av1::Fragments::default();
		for packet in packets {
			fragments
				.append(&packet.payload, packet.header[1] & 0x80 != 0, &mut frame)
				.expect("reassembled");
		}
		frame
	}

	#[test]
	fn av1_sender_framing_and_packetizer_round_trip_through_the_receiver() {
		let frame = av1_sender_frame(&AV1_ENCODER_FRAME).expect("a framed access unit");
		// The final OBU loses its size field; the first keeps it.
		assert_eq!(frame, [0x0a, 2, 0x20, 0x10, 0x30, 0x20, 0x30, 0x10]);
		let packets = av1_packets(&frame);
		assert_eq!(packets.len(), 1);
		assert_eq!(packets[0].header[1] & 0x7f, 105, "AV1 payload type");
		assert_ne!(packets[0].header[1] & 0x80, 0, "the last packet is marked");
		assert_eq!(
			(packets[0].payload[0] >> 4) & 3,
			0,
			"every element carries its size"
		);
		assert_eq!(av1_rebuild(&packets), frame);
	}

	#[test]
	fn an_av1_obu_larger_than_a_payload_fragments_and_rebuilds() {
		let mut encoder = vec![0x0a, 2, 0x20, 0x10];
		let body = vec![0x41u8; 3 * MAX_PAYLOAD];
		encoder.push(0x32);
		let mut size = [0u8; 3];
		encoder.extend_from_slice(size_bytes(body.len(), &mut size));
		encoder.extend_from_slice(&body);
		let frame = av1_sender_frame(&encoder).expect("a framed access unit");
		let packets = av1_packets(&frame);
		assert!(packets.len() >= 3, "the large OBU spans several packets");
		assert_eq!(
			packets.last().map(|packet| packet.header[1] & 0x80),
			Some(0x80)
		);
		assert_eq!(av1_rebuild(&packets), frame);
	}

	#[test]
	fn av1_frames_without_obus_are_rejected() {
		let mut sequence = 0u16;
		assert!(packetize_av1(&[0x0a], &mut sequence, 0, 0).is_err());
		assert!(av1_sender_frame(&[]).is_err());
		// A reserved OBU type is not encoder output.
		// The forbidden bit cannot appear in encoder output.
		assert!(av1_sender_frame(&[0x80, 1]).is_err());
	}

	#[test]
	fn av1_obus_dave_drops_never_stand_last() {
		// A keyframe as an encoder emits it: temporal delimiter, sequence header, frame, padding.
		let encoder = [
			0x12, 0x00, // temporal delimiter
			0x0a, 2, 0x20, 0x10, // sequence header
			0x32, 3, 0x20, 0x30, 0x10, // frame
			0x7a, 3, 0, 0, 0, // padding
		];
		let frame = av1_sender_frame(&encoder).expect("a framed access unit");
		// Padding must not keep the frame OBU's size field, which DAVE would then leave in place
		// while appending its protocol data behind it.
		assert_eq!(frame, [0x0a, 2, 0x20, 0x10, 0x30, 0x20, 0x30, 0x10]);
		let packets = av1_packets(&frame);
		assert_ne!(
			packets[0].payload[0] & 0x08,
			0,
			"the packet carrying the sequence header opens a coded video sequence"
		);
		assert_eq!(av1_rebuild(&packets), frame);
	}

	#[test]
	fn av1_payloads_stay_within_the_receivers_element_budget() {
		// Sixty-five OBUs fit one payload by bytes alone; the receiver stops at 64 elements.
		let encoder: Vec<u8> = (0..65).flat_map(|_| [0x32, 1, 0x00]).collect();
		let frame = av1_sender_frame(&encoder).expect("a framed access unit");
		let packets = av1_packets(&frame);
		assert_eq!(packets.len(), 2, "the 65th element opens a second payload");
		assert_eq!(av1_rebuild(&packets), frame);
	}

	/// libwebrtc's `video_rtp_depacketizer_av1.cc`, ported close enough to answer one question: does
	/// the code the official client runs accept the packets this sender writes? A rejection here is a
	/// rejection there, and this repository's own receiver cannot answer that — sender and receiver
	/// were written together.
	fn reference_assemble(payloads: &[&[u8]]) -> Result<Vec<u8>, &'static str> {
		fn leb(data: &[u8]) -> Option<(u64, usize)> {
			let mut value = 0u64;
			for (index, byte) in data.iter().copied().enumerate() {
				if index == 8 {
					return None;
				}
				value |= u64::from(byte & 0x7f) << (index * 7);
				if byte & 0x80 == 0 {
					return Some((value, index + 1));
				}
			}
			None
		}
		fn encode_leb(mut value: u64) -> Vec<u8> {
			let mut bytes = Vec::new();
			loop {
				let byte = (value & 0x7f) as u8;
				value >>= 7;
				if value == 0 {
					bytes.push(byte);
					return bytes;
				}
				bytes.push(byte | 0x80);
			}
		}
		let mut obus: Vec<Vec<u8>> = Vec::new();
		let mut expects_continuation = false;
		for payload in payloads {
			let Some((&header, body)) = payload.split_first() else {
				return Err("empty payload");
			};
			// The reference also rejects the reserved bits, and this sender leaves them clear.
			if header & 0x07 != 0 {
				return Err("reserved aggregation bits set");
			}
			let continues = header & 0x80 != 0;
			if continues != expects_continuation {
				return Err("Z bit disagrees with the previous packet");
			}
			let expected = usize::from((header >> 4) & 0x03);
			if body.is_empty() {
				if expected != 1 {
					return Err("packet carries only an aggregation header");
				}
				if !continues {
					obus.push(Vec::new());
				}
				expects_continuation = header & 0x40 != 0;
				continue;
			}
			let (mut at, mut index) = (0usize, 1usize);
			while at < body.len() {
				let has_size = index != expected;
				let (size, prefix) = if has_size {
					let (size, used) = leb(&body[at..]).ok_or("element size")?;
					(usize::try_from(size).map_err(|_| "element size")?, used)
				} else {
					(body.len() - at, 0)
				};
				if size > body.len() - at - prefix {
					return Err("element runs past the packet");
				}
				let element = &body[at + prefix..at + prefix + size];
				if index == 1 && continues {
					obus.last_mut()
						.ok_or("continuation without an OBU")?
						.extend_from_slice(element);
				} else {
					obus.push(element.to_vec());
				}
				at += prefix + size;
				index += 1;
			}
			expects_continuation = header & 0x40 != 0;
		}
		if expects_continuation {
			return Err("the last packet ends inside an OBU");
		}
		let mut frame = Vec::new();
		for obu in obus {
			let Some((&header, rest)) = obu.split_first() else {
				return Err("empty OBU");
			};
			let prefix = if header & 0x04 != 0 {
				if rest.is_empty() {
					return Err("OBU extension missing");
				}
				2
			} else {
				1
			};
			let (payload, size_field) = if header & 0x02 == 0 {
				(obu.len() - prefix, 0)
			} else {
				let (declared, used) = leb(&obu[prefix..]).ok_or("OBU size")?;
				let actual = obu.len() - prefix - used;
				if declared as usize != actual {
					return Err("declared OBU size disagrees with the segment");
				}
				(actual, used)
			};
			// The reference sets the size bit on the header it writes, whatever the wire carried.
			let mut head = obu[..prefix].to_vec();
			head[0] |= 0x02;
			frame.extend_from_slice(&head);
			frame.extend_from_slice(&encode_leb(payload as u64));
			frame.extend_from_slice(&obu[prefix + size_field..]);
		}
		Ok(frame)
	}

	/// The OBU sequence as `(type, payload)`, so frames assembled with different size fields still
	/// compare by what they carry.
	fn obu_payloads(data: &[u8]) -> Vec<(u8, Vec<u8>)> {
		obus_av1(data)
			.expect("an OBU walk")
			.into_iter()
			.map(|(start, end)| {
				let obu = &data[start..end];
				let header = obu[0];
				let mut at = if header & 0x04 != 0 { 2 } else { 1 };
				if header & 0x02 != 0 {
					while obu.get(at).is_some_and(|byte| byte & 0x80 != 0) {
						at += 1;
					}
					at += 1;
				}
				((header >> 3) & 15, obu[at..].to_vec())
			})
			.collect()
	}

	/// The official client runs libwebrtc's AV1 depacketizer, so a share only reaches it when that
	/// code accepts these packets. This checks a keyframe that carries a sequence header and a delta
	/// frame that does not.
	#[test]
	fn the_reference_depacketizer_accepts_the_av1_packets_we_send() {
		let keyframe = [
			0x12, 0x00, // temporal delimiter
			0x0a, 2, 0x20, 0x10, // sequence header
			0x32, 3, 0x20, 0x30, 0x10, // frame
		];
		let delta = [0x32, 3, 0x20, 0x30, 0x10];
		for encoder in [keyframe.to_vec(), delta.to_vec()] {
			let frame = av1_sender_frame(&encoder).expect("a framed access unit");
			let packets = av1_packets(&frame);
			let payloads: Vec<&[u8]> = packets
				.iter()
				.map(|packet| packet.payload.as_slice())
				.collect();
			let assembled = reference_assemble(&payloads).expect("libwebrtc accepts the packets");
			assert_eq!(
				obu_payloads(&assembled),
				obu_payloads(&frame),
				"the reference assembles the same OBUs the sender framed"
			);
		}
	}

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
