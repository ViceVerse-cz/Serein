//! Bounded RFC 3550 reception reports for watched video. Only authenticated RTP from
//! announced sources enters these counters; no synthetic bandwidth estimate is advertised.
use std::time::Instant;

const MAX_SOURCES: usize = super::video_receive::MAX_SOURCES;

struct Source {
	ssrc: u32,
	base: u16,
	max: u16,
	cycles: u64,
	bad_sequence: u32,
	received: u64,
	expected_prior: u64,
	received_prior: u64,
	transit: Option<u32>,
	jitter: f64,
	sender_report: Option<(u32, Instant)>,
}

impl Source {
	fn new(ssrc: u32, sequence: u16) -> Self {
		Self {
			ssrc,
			base: sequence,
			max: sequence,
			cycles: 0,
			bad_sequence: 65_537,
			received: 0,
			expected_prior: 0,
			received_prior: 0,
			transit: None,
			jitter: 0.0,
			sender_report: None,
		}
	}

	fn receive(&mut self, sequence: u16, timestamp: u32, arrival: u32) {
		// RFC 3550 Appendix A.1: accept wrap/reordering, but require confirmation of a
		// large sequence jump before restarting the accounting epoch.
		let delta = sequence.wrapping_sub(self.max);
		if delta < 3000 {
			if sequence < self.max {
				self.cycles += 65_536;
			}
			self.max = sequence;
		} else if u32::from(delta) <= 65_536 - 100 {
			if u32::from(sequence) != self.bad_sequence {
				self.bad_sequence = u32::from(sequence.wrapping_add(1));
				return;
			}
			*self = Self::new(self.ssrc, sequence);
		}
		self.received += 1;
		let transit = arrival.wrapping_sub(timestamp);
		if let Some(previous) = self.transit {
			let delta = (transit.wrapping_sub(previous) as i32).unsigned_abs();
			self.jitter += (f64::from(delta) - self.jitter) / 16.0;
		}
		self.transit = Some(transit);
	}

	fn report(&mut self, now: Instant, body: &mut Vec<u8>) {
		let highest = self.cycles + u64::from(self.max);
		let expected = highest - u64::from(self.base) + 1;
		let lost = i128::from(expected) - i128::from(self.received);
		let interval = expected - self.expected_prior;
		let interval_lost = i128::from(interval) - i128::from(self.received - self.received_prior);
		let fraction = if interval == 0 || interval_lost <= 0 {
			0
		} else {
			((interval_lost * 256 / i128::from(interval)).min(255)) as u32
		};
		self.expected_prior = expected;
		self.received_prior = self.received;
		let loss =
			(fraction << 24) | ((lost.clamp(-8_388_608, 8_388_607) as i32 as u32) & 0x00ff_ffff);
		let (lsr, delay) = self.sender_report.map_or((0, 0), |(lsr, at)| {
			let delay = now
				.saturating_duration_since(at)
				.as_micros()
				.saturating_mul(65_536)
				/ 1_000_000;
			(lsr, delay.min(u128::from(u32::MAX)) as u32)
		});
		for word in [
			self.ssrc,
			loss,
			highest as u32,
			self.jitter as u32,
			lsr,
			delay,
		] {
			body.extend_from_slice(&word.to_be_bytes());
		}
	}
}

pub(crate) struct ReceiveReports {
	started: Instant,
	sources: Vec<Source>,
}

impl ReceiveReports {
	pub fn new(now: Instant) -> Self {
		Self {
			started: now,
			sources: Vec::new(),
		}
	}

	pub fn retain(&mut self, mut keep: impl FnMut(u32) -> bool) {
		self.sources.retain(|source| keep(source.ssrc));
	}

	pub fn receive(&mut self, ssrc: u32, sequence: u16, timestamp: u32, now: Instant) {
		let index = match self.sources.iter().position(|source| source.ssrc == ssrc) {
			Some(index) => index,
			None if ssrc != 0 && self.sources.len() < MAX_SOURCES => {
				self.sources.push(Source::new(ssrc, sequence));
				self.sources.len() - 1
			}
			None => return,
		};
		let arrival = (now.saturating_duration_since(self.started).as_nanos() * 90_000
			/ 1_000_000_000) as u32;
		self.sources[index].receive(sequence, timestamp, arrival);
	}

	/// The caller authenticates/decrypts the whole datagram first. Validate the complete
	/// compound packet before applying any sender clocks, including padding/truncation.
	pub fn sender_reports(
		&mut self,
		mut packet: &[u8],
		now: Instant,
	) -> Vec<crate::stream_sync::SenderReport> {
		let mut clocks = Vec::with_capacity(MAX_SOURCES * 2);
		while !packet.is_empty() {
			if packet.len() < 4 || packet[0] >> 6 != 2 || !(192..=223).contains(&packet[1]) {
				return Vec::new();
			}
			let size = (usize::from(u16::from_be_bytes([packet[2], packet[3]])) + 1) * 4;
			if size > packet.len() {
				return Vec::new();
			}
			let mut contents = &packet[..size];
			if contents[0] & 0x20 != 0 {
				let padding = usize::from(contents[size - 1]);
				if size != packet.len() || padding == 0 || padding > size - 4 {
					return Vec::new();
				}
				contents = &contents[..size - padding];
			}
			if contents.len() < 8 {
				return Vec::new();
			}
			if contents[1] == 201 && contents.len() < 8 + usize::from(contents[0] & 31) * 24 {
				return Vec::new();
			}
			if contents[1] == 200 {
				if contents.len() < 28 + usize::from(contents[0] & 31) * 24 {
					return Vec::new();
				}
				let ssrc = u32::from_be_bytes(contents[4..8].try_into().unwrap());
				if clocks.len() < MAX_SOURCES * 2 {
					clocks.push(crate::stream_sync::SenderReport {
						ssrc,
						ntp: u64::from_be_bytes(contents[8..16].try_into().unwrap()),
						rtp: u32::from_be_bytes(contents[16..20].try_into().unwrap()),
					});
				}
			}
			packet = &packet[size..];
		}
		for clock in &clocks {
			if let Some(source) = self
				.sources
				.iter_mut()
				.find(|source| source.ssrc == clock.ssrc)
			{
				source.sender_report = Some(((clock.ntp >> 16) as u32, now));
			}
		}
		clocks
	}

	pub fn report(&mut self, sender: u32, now: Instant) -> ([u8; 8], Vec<u8>) {
		let mut header = [0x80 | self.sources.len() as u8, 201, 0, 0, 0, 0, 0, 0];
		let words = 1 + self.sources.len() as u16 * 6;
		header[2..4].copy_from_slice(&words.to_be_bytes());
		header[4..].copy_from_slice(&sender.to_be_bytes());
		let mut body = Vec::with_capacity(self.sources.len() * 24);
		for source in &mut self.sources {
			source.report(now, &mut body);
		}
		(header, body)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;
	fn words(body: &[u8]) -> Vec<u32> {
		body.as_chunks::<4>()
			.0
			.iter()
			.map(|b| u32::from_be_bytes(*b))
			.collect()
	}

	#[test]
	fn reports_actual_loss_reordering_wrap_and_recovered_intervals() {
		let now = Instant::now();
		let mut reports = ReceiveReports::new(now);
		for seq in [65_534, 65_535, 1] {
			reports.receive(42, seq, 100, now);
		}
		let (header, body) = reports.report(9, now);
		assert_eq!(header, [0x81, 201, 0, 7, 0, 0, 0, 9]);
		assert_eq!(words(&body), vec![42, (64 << 24) | 1, 65_537, 0, 0, 0]);
		reports.receive(42, 0, 100, now);
		let (_, body) = reports.report(9, now);
		assert_eq!(words(&body)[1], 0);
		reports.receive(42, 1, 100, now);
		let (_, body) = reports.report(9, now);
		assert_eq!(words(&body)[1], 0x00ff_ffff);
		reports.receive(42, 30_000, 100, now);
		let (_, body) = reports.report(9, now);
		assert_eq!(words(&body)[2], 65_537);
		reports.receive(42, 30_001, 100, now);
		let (_, body) = reports.report(9, now);
		assert_eq!(words(&body)[1..3], [0, 30_001]);
	}

	#[test]
	fn sender_clocks_require_a_valid_compound_and_track_elapsed_delay() {
		let now = Instant::now();
		let mut reports = ReceiveReports::new(now);
		reports.receive(42, 100, 0, now);
		reports.receive(42, 101, 900, now + Duration::from_millis(20));
		let mut sr = vec![0x80, 200, 0, 6, 0, 0, 0, 42];
		sr.extend_from_slice(&[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]);
		sr.resize(28, 0);
		sr[16..20].copy_from_slice(&0x12345678u32.to_be_bytes());
		let mut malformed = sr.clone();
		malformed.push(0);
		assert!(reports.sender_reports(&malformed, now).is_empty());
		assert!(reports.sources[0].sender_report.is_none());
		let clocks = reports.sender_reports(&sr, now);
		assert_eq!(clocks.len(), 1);
		assert_eq!(
			(clocks[0].ssrc, clocks[0].ntp, clocks[0].rtp),
			(42, 0x123456789abcdef0, 0x12345678)
		);
		let (_, body) = reports.report(9, now + Duration::from_millis(500));
		assert_eq!(words(&body)[3..], [56, 0x5678_9abc, 32_768]);
		for end in 0..sr.len() {
			assert!(reports.sender_reports(&sr[..end], now).is_empty());
		}
	}

	#[test]
	fn report_sources_and_packet_storage_are_bounded_and_retired() {
		let now = Instant::now();
		let mut reports = ReceiveReports::new(now);
		for ssrc in 0..100 {
			reports.receive(ssrc, 1, 0, now);
		}
		assert_eq!(reports.sources.len(), MAX_SOURCES);
		let (header, body) = reports.report(9, now);
		assert_eq!(header[0] & 31, 16);
		assert_eq!(body.len(), 16 * 24);
		reports.retain(|ssrc| ssrc == 1);
		assert_eq!(reports.sources.len(), 1);
		reports.retain(|_| false);
		assert_eq!(
			reports.report(9, now),
			(
				vec![0x80, 201, 0, 1, 0, 0, 0, 9].try_into().unwrap(),
				vec![]
			)
		);
	}

	#[test]
	fn reception_reports_round_trip_through_authenticated_transport_feedback() {
		use crate::crypto::Encryption;
		let now = Instant::now();
		let mut reports = ReceiveReports::new(now);
		for seq in [1, 2, 4] {
			reports.receive(42, seq, 0, now);
		}
		let (header, body) = reports.report(9, now);
		let mut cipher = Encryption::new(&[7; 32]);
		let packet = cipher.seal_rtcp(&header, &body).unwrap();
		assert_eq!(
			cipher.open_rtcp(&packet).unwrap(),
			[&header[..], &body].concat()
		);
		assert_eq!(cipher.feedback(&packet, 42).unwrap().loss, Some(64));
		assert!(Encryption::new(&[8; 32]).open_rtcp(&packet).is_none());
		for index in 0..packet.len() {
			let mut changed = packet.clone();
			changed[index] ^= 0x40;
			assert!(cipher.open_rtcp(&changed).is_none());
			assert!(cipher.open_rtcp(&packet[..index]).is_none());
		}
	}
}
