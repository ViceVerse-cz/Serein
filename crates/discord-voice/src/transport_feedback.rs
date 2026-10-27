//! Transport-wide arrival feedback (WebRTC RTPFB/FMT=15). No bandwidth estimate:
//! report only authenticated extension-5 sequence numbers and observed arrival times.
use std::{collections::VecDeque, time::Instant};

const HISTORY: usize = 2048;
const REPORT: usize = 256;
const AGE_TICKS: i64 = 4000; // One second, in 250 us units.

pub(crate) struct TransportFeedback {
	started: Instant,
	base: Option<u16>,
	arrivals: VecDeque<Option<i64>>,
	count: u8,
	discarded: u64,
	last: Option<i64>,
}
pub(crate) struct Report {
	pub header: [u8; 8],
	pub body: Vec<u8>,
	pub received: u64,
	pub missing: u64,
}
impl TransportFeedback {
	pub fn new(started: Instant) -> Self {
		Self {
			started,
			base: None,
			arrivals: VecDeque::new(),
			count: 0,
			discarded: 0,
			last: None,
		}
	}
	fn ticks(&self, now: Instant) -> i64 {
		((now.saturating_duration_since(self.started).as_nanos() + 125_000) / 250_000)
			.min(i64::MAX as u128) as i64
	}
	pub fn receive(&mut self, sequence: u16, now: Instant) {
		let at = self.ticks(now);
		if self
			.last
			.is_some_and(|last| at.saturating_sub(last) > AGE_TICKS)
		{
			self.discarded += self.arrivals.len() as u64;
			self.arrivals.clear();
			self.base = None;
		}
		let base = *self.base.get_or_insert(sequence);
		let mut offset = i32::from(sequence.wrapping_sub(base) as i16);
		if offset < 0 {
			return;
		} // Already reported or outside the bounded sequence epoch.
		if offset as usize >= HISTORY {
			self.discarded += self.arrivals.len() as u64;
			self.arrivals.clear();
			self.base = Some(sequence);
			offset = 0;
		}
		self.arrivals
			.resize(self.arrivals.len().max(offset as usize + 1), None);
		if self.arrivals[offset as usize].is_none() {
			self.arrivals[offset as usize] = Some(at); // First arrival, never a duplicate.
			self.last = Some(at);
		}
	}
	fn advance(&mut self, count: usize) {
		self.arrivals.drain(..count);
		self.base = self.base.map(|base| base.wrapping_add(count as u16));
	}
	pub fn take_discarded(&mut self) -> u64 {
		std::mem::take(&mut self.discarded)
	}
	pub fn report(&mut self, sender: u32, now: Instant) -> Option<Report> {
		let oldest = self.ticks(now).saturating_sub(AGE_TICKS);
		while let Some(index) = self.arrivals.iter().position(Option::is_some) {
			if self.arrivals[index]? >= oldest {
				break;
			}
			self.discarded += (index + 1) as u64;
			self.advance(index + 1);
		}
		let anchor = self.arrivals.iter().flatten().next().copied()?;
		let reference = anchor / 256; // Reference time is in 64 ms units, modulo 24 bits.
		let mut previous = reference * 256;
		let count = self.arrivals.len().min(REPORT);
		let mut symbols = Vec::<u8>::with_capacity(count);
		let mut deltas = Vec::with_capacity(count * 2);
		let mut received = 0;
		for arrival in self.arrivals.iter().take(count) {
			let Some(at) = arrival else {
				symbols.push(0);
				continue;
			};
			let delta = i16::try_from(at - previous).ok()?;
			if let Ok(small) = u8::try_from(delta) {
				symbols.push(1);
				deltas.push(small);
			} else {
				symbols.push(2);
				deltas.extend_from_slice(&delta.to_be_bytes());
			}
			previous = *at;
			received += 1;
		}
		let mut body = Vec::with_capacity(608);
		body.extend_from_slice(&[0; 4]); // Transport-wide feedback has no single media SSRC.
		body.extend_from_slice(&self.base?.to_be_bytes());
		body.extend_from_slice(&(count as u16).to_be_bytes());
		let reference = (reference as u32).to_be_bytes();
		body.extend_from_slice(&reference[1..]);
		body.push(self.count);
		for chunk in symbols.chunks(7) {
			let mut word = 0xc000u16; // Two-bit status vector; no run-length ambiguity.
			for (i, symbol) in chunk.iter().enumerate() {
				word |= u16::from(*symbol) << (12 - i * 2);
			}
			body.extend_from_slice(&word.to_be_bytes());
		}
		body.extend_from_slice(&deltas);
		while body.len() % 4 != 0 {
			body.push(0);
		}
		let mut header = [0x8f, 205, 0, 0, 0, 0, 0, 0];
		header[2..4].copy_from_slice(&((body.len() / 4 + 1) as u16).to_be_bytes());
		header[4..].copy_from_slice(&sender.to_be_bytes());
		self.advance(count);
		self.count = self.count.wrapping_add(1);
		Some(Report {
			header,
			body,
			received,
			missing: count as u64 - received,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;
	#[test]
	fn wire_feedback_preserves_wrap_reordering_loss_and_signed_arrival_deltas() {
		let start = Instant::now();
		let mut feedback = TransportFeedback::new(start);
		for (seq, ms) in [(65534, 10), (0, 11), (65535, 12), (65535, 13), (1, 82)] {
			feedback.receive(seq, start + Duration::from_millis(ms));
		}
		let report = feedback
			.report(9, start + Duration::from_millis(100))
			.unwrap();
		assert_eq!(report.header, [0x8f, 205, 0, 6, 0, 0, 0, 9]);
		assert_eq!(
			report.body,
			[
				0, 0, 0, 0, 255, 254, 0, 4, 0, 0, 0, 0, 0xd6, 0x80, 40, 8, 255, 252, 1, 28
			]
		);
		assert_eq!((report.received, report.missing), (4, 0));
		feedback.receive(65535, start + Duration::from_millis(101));
		assert!(
			feedback
				.report(9, start + Duration::from_millis(102))
				.is_none()
		);
		feedback.receive(3, start + Duration::from_millis(110));
		let report = feedback
			.report(9, start + Duration::from_millis(120))
			.unwrap();
		assert_eq!(
			report.body,
			[0, 0, 0, 0, 0, 2, 0, 2, 0, 0, 1, 1, 0xc4, 0, 184, 0]
		);
		assert_eq!((report.received, report.missing), (1, 1));
		let mut cipher = crate::crypto::Encryption::new(&[7; 32]);
		let packet = cipher.seal_rtcp(&report.header, &report.body).unwrap();
		assert_eq!(
			cipher.open_rtcp(&packet).unwrap(),
			[&report.header[..], &report.body].concat()
		);
	}
	#[test]
	fn history_feedback_datagrams_and_expired_arrivals_are_bounded() {
		let start = Instant::now();
		let mut feedback = TransportFeedback::new(start);
		for seq in 0..2048 {
			feedback.receive(seq, start + Duration::from_micros(u64::from(seq) * 200));
		}
		assert!(feedback.arrivals.capacity() <= HISTORY);
		let mut received = 0;
		while let Some(report) = feedback.report(9, start + Duration::from_millis(500)) {
			assert!(report.body.len() + 8 + 20 <= 1200);
			received += report.received;
		}
		assert_eq!(received, 2048);
		feedback.receive(2048, start + Duration::from_secs(1));
		assert!(feedback.report(9, start + Duration::from_secs(3)).is_none());
		assert_eq!(feedback.take_discarded(), 1);
		feedback.receive(2050, start + Duration::from_secs(3));
		feedback.receive(10000, start + Duration::from_secs(3));
		let report = feedback.report(9, start + Duration::from_secs(3)).unwrap();
		assert_eq!((report.received, report.missing), (1, 0));
		assert!(feedback.take_discarded() > 0);
		// A restarted transport sequence after inactivity must not be mistaken for
		// permanently stale packets from the previous sequence epoch.
		feedback.receive(1, start + Duration::from_secs(5));
		let report = feedback.report(9, start + Duration::from_secs(5)).unwrap();
		assert_eq!((report.received, report.missing), (1, 0));
	}
}
