//! Bounded RTP ordering before H.264 reassembly. Only encrypted payloads are retained.
use std::{
	collections::BTreeMap,
	time::{Duration, Instant},
};

const MAX_PACKETS: u64 = 512;
const MAX_BYTES: usize = 512 * 1024;
const MAX_PAYLOAD: usize = 4096;
const REPAIR_AGE: Duration = Duration::from_millis(120);
const REORDER_GRACE: Duration = Duration::from_millis(10);
const RETRY: Duration = Duration::from_millis(40);
const FEEDBACK_INTERVAL: Duration = Duration::from_millis(20);
const MAX_REQUESTS: usize = 64;

pub(super) struct Packet {
	pub sequence: u16,
	pub timestamp: u32,
	pub marker: bool,
	pub payload: Vec<u8>,
}

struct Missing {
	since: Instant,
	last_sent: Option<Instant>,
	attempts: u8,
}

#[derive(Default)]
pub(super) struct Window {
	next: Option<u64>,
	packets: BTreeMap<u64, Packet>,
	missing: BTreeMap<u64, Missing>,
	bytes: usize,
	next_feedback: Option<Instant>,
	pub repaired: u64,
}

impl Window {
	/// Returns true when a resource/discontinuity bound forces reference-chain recovery.
	pub fn push(
		&mut self,
		sequence: u16,
		timestamp: u32,
		marker: bool,
		payload: &[u8],
		now: Instant,
	) -> bool {
		let next = *self.next.get_or_insert(u64::from(sequence));
		let distance = sequence.wrapping_sub(next as u16);
		if distance >= 32768 {
			return false;
		} // Old or duplicate, including expired RTX.
		let extended = next + u64::from(distance);
		if self.packets.contains_key(&extended) {
			return false;
		}
		if self
			.missing
			.get(&extended)
			.is_some_and(|missing| now.saturating_duration_since(missing.since) >= REPAIR_AGE)
		{
			return false;
		}
		let reset = u64::from(distance) >= MAX_PACKETS
			|| self.bytes + payload.len() > MAX_BYTES
			|| payload.len() > MAX_PAYLOAD;
		if reset {
			self.packets.clear();
			self.missing.clear();
			self.bytes = 0;
			self.next = Some(extended);
			self.next_feedback = None;
		}
		if payload.len() > MAX_PAYLOAD {
			self.next = Some(extended + 1);
			return true;
		}
		for missing in self.next.unwrap()..extended {
			if !self.packets.contains_key(&missing) {
				self.missing.entry(missing).or_insert(Missing {
					since: now,
					last_sent: None,
					attempts: 0,
				});
			}
		}
		if self.missing.remove(&extended).is_some() {
			self.repaired += 1;
		}
		self.bytes += payload.len();
		self.packets.insert(
			extended,
			Packet {
				sequence,
				timestamp,
				marker,
				payload: payload.to_vec(),
			},
		);
		reset
	}

	/// Ordered packets have no added delay. A gap gets at most 120 ms to be repaired,
	/// across picture boundaries, and expires on transport ticks even without new RTP.
	pub fn pop(&mut self, now: Instant) -> (Option<Packet>, bool) {
		let Some(mut next) = self.next else {
			return (None, false);
		};
		let mut lost = false;
		if !self.packets.contains_key(&next) {
			let expired = self
				.missing
				.get(&next)
				.is_some_and(|missing| now.saturating_duration_since(missing.since) >= REPAIR_AGE);
			if !expired {
				return (None, false);
			}
			let Some((&first, _)) = self.packets.first_key_value() else {
				return (None, false);
			};
			next = first;
			self.missing.retain(|sequence, _| *sequence >= next);
			lost = true;
		}
		let packet = self.packets.remove(&next).expect("ordered packet exists");
		self.bytes -= packet.payload.len();
		self.next = Some(next + 1);
		(Some(packet), lost)
	}

	/// RFC 4585 Generic NACK (FMT=1/PT=205). Ask only for observed holes, never
	/// advertise bandwidth. Three attempts per hole; at most 64 numbers per 20 ms.
	pub fn nack(&mut self, sender: u32, media: u32, now: Instant) -> Option<([u8; 8], Vec<u8>)> {
		if self.next_feedback.is_some_and(|at| now < at) {
			return None;
		}
		let mut requests = Vec::new();
		for (&sequence, missing) in &mut self.missing {
			let age = now.saturating_duration_since(missing.since);
			if age < REORDER_GRACE
				|| age >= REPAIR_AGE
				|| missing.attempts >= 3
				|| missing
					.last_sent
					.is_some_and(|at| now.saturating_duration_since(at) < RETRY)
			{
				continue;
			}
			requests.push(sequence as u16);
			missing.last_sent = Some(now);
			missing.attempts += 1;
			if requests.len() == MAX_REQUESTS {
				break;
			}
		}
		if requests.is_empty() {
			return None;
		}
		self.next_feedback = Some(now + FEEDBACK_INTERVAL);
		let mut body = media.to_be_bytes().to_vec();
		let mut at = 0;
		while at < requests.len() {
			let pid = requests[at];
			at += 1;
			let mut mask = 0u16;
			while at < requests.len() {
				let distance = requests[at].wrapping_sub(pid);
				if distance > 16 {
					break;
				}
				mask |= 1 << (distance - 1);
				at += 1;
			}
			body.extend_from_slice(&pid.to_be_bytes());
			body.extend_from_slice(&mask.to_be_bytes());
		}
		let mut header = [0x81, 205, 0, 0, 0, 0, 0, 0];
		header[2..4].copy_from_slice(&((body.len() / 4 + 1) as u16).to_be_bytes());
		header[4..].copy_from_slice(&sender.to_be_bytes());
		Some((header, body))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn ordering_crosses_timestamps_and_sequence_wrap_without_delaying_intact_packets() {
		let now = Instant::now();
		let mut window = Window::default();
		assert!(!window.push(65534, 1, true, &[1], now));
		assert_eq!(window.pop(now).0.unwrap().sequence, 65534);
		window.push(1, 3, true, &[4], now);
		window.push(0, 2, true, &[3], now);
		assert!(window.pop(now).0.is_none());
		assert!(window.nack(7, 9, now).is_none());
		let (header, body) = window.nack(7, 9, now + REORDER_GRACE).unwrap();
		assert_eq!(header, [0x81, 205, 0, 3, 0, 0, 0, 7]);
		assert_eq!(body, [0, 0, 0, 9, 255, 255, 0, 0]);
		window.push(65535, 1, true, &[2], now + RETRY);
		for sequence in [65535, 0, 1] {
			let (packet, lost) = window.pop(now + RETRY);
			assert_eq!(packet.unwrap().sequence, sequence);
			assert!(!lost);
		}
		assert_eq!(window.repaired, 2);
		assert!(window.nack(7, 9, now + RETRY).is_none());
		assert!(!window.push(65535, 1, true, &[9], now + RETRY));
		assert!(window.pop(now + RETRY).0.is_none());
	}
	#[test]
	fn timer_expires_unrepairable_holes_and_retries_stop() {
		let now = Instant::now();
		let mut window = Window::default();
		window.push(1, 1, true, &[1], now);
		window.pop(now);
		window.push(4, 2, true, &[4], now);
		for ms in [10, 50, 90] {
			let (header, body) = window.nack(7, 9, now + Duration::from_millis(ms)).unwrap();
			assert_eq!(&header[..4], &[0x81, 205, 0, 3]);
			assert_eq!(body, [0, 0, 0, 9, 0, 2, 0, 1]); // PID 2, BLP requests 3.
			assert!(
				window
					.nack(7, 9, now + Duration::from_millis(ms + 1))
					.is_none()
			);
		}
		assert!(
			window
				.pop(now + REPAIR_AGE - Duration::from_millis(1))
				.0
				.is_none()
		);
		let (packet, lost) = window.pop(now + REPAIR_AGE);
		assert!(lost);
		assert_eq!(packet.unwrap().sequence, 4);
		assert!(window.nack(7, 9, now + REPAIR_AGE).is_none());
		window.push(2, 1, true, &[2], now + REPAIR_AGE);
		assert!(window.pop(now + REPAIR_AGE).0.is_none());
	}
	#[test]
	fn memory_discontinuity_and_feedback_are_bounded() {
		let now = Instant::now();
		let mut window = Window::default();
		window.push(0, 1, false, &[1], now);
		window.pop(now);
		let payload = vec![1; MAX_PAYLOAD];
		for sequence in 2..=129 {
			assert!(!window.push(sequence, 1, false, &payload, now));
		}
		assert_eq!(window.bytes, MAX_BYTES);
		assert!(window.push(130, 1, true, &payload, now));
		assert_eq!(window.bytes, MAX_PAYLOAD);
		assert_eq!(window.pop(now).0.unwrap().sequence, 130);
		assert!(window.push(900, 2, true, &[1], now));
		window.pop(now);
		window.push(1412, 3, true, &[1], now);
		let (_, body) = window.nack(7, 9, now + REORDER_GRACE).unwrap();
		assert!(body.len() <= 4 + MAX_REQUESTS * 4);
		assert!(window.missing.len() < MAX_PACKETS as usize);
		assert!(window.push(1413, 3, true, &vec![1; MAX_PAYLOAD + 1], now));
		assert_eq!(window.bytes, 0);
		assert!(window.missing.is_empty());
	}
}
