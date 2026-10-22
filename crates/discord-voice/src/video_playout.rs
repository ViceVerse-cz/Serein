//! Bounded decode scheduling. RTP clock ownership lives only in stream_sync.
use crate::{
	Presentation,
	presentation::{DECODE_LEAD, MAX_AHEAD, MAX_LATE},
	video_receive::Encoded,
};
use std::{collections::VecDeque, time::Instant};
const MAX_FRAMES: usize = 64;
const MAX_BYTES: usize = 8 * 1024 * 1024;
pub(crate) struct Timed {
	pub frame: Encoded,
	pub source: u32,
}
struct Pending {
	frame: Encoded,
	source: u32,
	deadline: Instant,
	presentation: Option<Presentation>,
}
#[derive(Default)]
pub(crate) struct Stats {
	pub late: u64,
	pub resets: u64,
	pub dropped: u64,
	pub depth: u64,
	pub rewinds: u64,
	pub source_changes: u64,
	pub expired: u64,
	pub pressure: u64,
	pub ahead: u64,
}
#[derive(Default)]
pub(crate) struct Playout {
	queue: VecDeque<Pending>,
	bytes: usize,
	stats: Stats,
	lead: std::time::Duration,
}
impl Playout {
	pub fn set_decode_lead(&mut self, lead: std::time::Duration) {
		self.lead = lead.clamp(
			std::time::Duration::from_millis(8),
			std::time::Duration::from_millis(100),
		);
	}
	#[cfg(test)]
	pub fn push_at(
		&mut self,
		timed: Timed,
		now: Instant,
		deadline: Option<Instant>,
	) -> Result<(), u64> {
		self.insert(timed, now, deadline.unwrap_or(now), None)
	}
	pub fn push_presented(
		&mut self,
		timed: Timed,
		now: Instant,
		presentation: Presentation,
	) -> Result<(), u64> {
		self.insert(timed, now, presentation.at, Some(presentation))
	}
	fn insert(
		&mut self,
		timed: Timed,
		now: Instant,
		deadline: Instant,
		presentation: Option<Presentation>,
	) -> Result<(), u64> {
		let user = timed.frame.user;
		let rewind = self
			.queue
			.back()
			.is_some_and(|pending| deadline < pending.deadline);
		let source_change = self
			.queue
			.back()
			.is_some_and(|pending| pending.frame.user != user || pending.source != timed.source);
		let ahead = deadline > now + MAX_AHEAD;
		let expired = now.saturating_duration_since(deadline) > MAX_LATE;
		if rewind || source_change || ahead || expired {
			self.stats.rewinds += u64::from(rewind);
			self.stats.source_changes += u64::from(source_change);
			self.stats.ahead += u64::from(ahead);
			self.stats.expired += u64::from(expired);
			self.clear();
			if !timed.frame.keyframe {
				return Err(user);
			}
		}
		if self.queue.len() >= MAX_FRAMES || timed.frame.data.capacity() > MAX_BYTES - self.bytes {
			self.stats.pressure += 1;
			self.clear();
			return Err(user);
		}
		self.stats.late += u64::from(deadline < now);
		self.bytes += timed.frame.data.capacity();
		self.queue.push_back(Pending {
			frame: timed.frame,
			source: timed.source,
			deadline,
			presentation,
		});
		self.stats.depth = self.stats.depth.max(self.queue.len() as u64);
		Ok(())
	}
	pub fn deadline(&self) -> Option<Instant> {
		let lead = if self.lead.is_zero() {
			DECODE_LEAD
		} else {
			self.lead
		};
		self.queue.front().map(|pending| {
			pending
				.presentation
				.as_ref()
				.map_or(pending.deadline, |stamp| {
					stamp.decode_deadline(Instant::now(), lead)
				})
		})
	}
	pub fn pop_timed(
		&mut self,
		now: Instant,
	) -> Result<Option<(Encoded, Option<Presentation>)>, u64> {
		let Some(due) = self.deadline() else {
			return Ok(None);
		};
		if now < due {
			return Ok(None);
		}
		let pending = self.queue.front().expect("deadline has a picture");
		let target = pending
			.presentation
			.as_ref()
			.map_or(pending.deadline, |stamp| stamp.deadline(now));
		if now.saturating_duration_since(target) > MAX_LATE {
			self.stats.expired += 1;
			let user = pending.frame.user;
			self.clear();
			return Err(user);
		}
		let pending = self.queue.pop_front().expect("front is ready");
		self.bytes -= pending.frame.data.capacity();
		Ok(Some((pending.frame, pending.presentation)))
	}
	#[cfg(test)]
	pub fn pop(&mut self, now: Instant) -> Result<Option<Encoded>, u64> {
		self.pop_timed(now)
			.map(|frame| frame.map(|(frame, _)| frame))
	}
	pub fn clear(&mut self) -> bool {
		let discarded = !self.queue.is_empty();
		self.stats.resets += u64::from(discarded);
		self.stats.dropped += self.queue.len() as u64;
		self.queue.clear();
		self.bytes = 0;
		discarded
	}
	pub fn take_stats(&mut self) -> Stats {
		std::mem::take(&mut self.stats)
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;
	fn frame(_timestamp: u32) -> Timed {
		Timed {
			frame: Encoded {
				codec: crate::video_codec::Codec::H264,
				user: 7,
				data: vec![1],
				keyframe: true,
			},
			source: 70,
		}
	}
	#[test]
	fn decode_starts_before_presentation_without_reordering_reference_pictures() {
		let start = Instant::now();
		let clock = std::sync::Arc::new(crate::presentation::Clock::default());
		let mut queue = Playout::default();
		for n in 0..8 {
			let at = start + Duration::from_millis(100 + n * 16);
			queue
				.push_presented(frame(n as u32 * 1500), start, clock.stamp(at))
				.unwrap();
		}
		for n in 0..8 {
			let due = start + Duration::from_millis(60 + n * 16);
			assert_eq!(queue.deadline(), Some(due));
			assert!(
				queue
					.pop_timed(due - Duration::from_nanos(1))
					.unwrap()
					.is_none()
			);
			let (_, tag) = queue.pop_timed(due).unwrap().unwrap();
			assert_eq!(tag.unwrap().at, due + DECODE_LEAD);
		}
	}
	#[test]
	fn queued_decode_deadlines_follow_device_playback_feedback() {
		let now = Instant::now();
		let clock = std::sync::Arc::new(crate::presentation::Clock::default());
		let mut queue = Playout::default();
		queue
			.push_presented(frame(0), now, clock.stamp(now + Duration::from_millis(100)))
			.unwrap();
		clock.stamp(now).played(now + Duration::from_millis(73));
		assert_eq!(queue.deadline(), Some(now + Duration::from_millis(133)));
		assert!(
			queue
				.pop_timed(now + Duration::from_millis(132))
				.unwrap()
				.is_none()
		);
		assert!(
			queue
				.pop_timed(now + Duration::from_millis(133))
				.unwrap()
				.is_some()
		);
	}

	#[test]
	fn pressure_and_discontinuities_require_reference_chain_recovery() {
		let now = Instant::now();
		for large in [false, true] {
			let mut queue = Playout::default();
			for _ in 0..if large { 8 } else { MAX_FRAMES } {
				let mut frame = frame(0);
				if large {
					frame.frame.data = vec![0; 1024 * 1024];
				}
				queue.push_at(frame, now, Some(now)).unwrap();
			}
			let mut extra = frame(0);
			if large {
				extra.frame.data = vec![0; 1024 * 1024];
			}
			assert_eq!(queue.push_at(extra, now, Some(now)), Err(7));
			assert!(queue.deadline().is_none());
		}
		let mut queue = Playout::default();
		queue
			.push_at(frame(0), now, Some(now + Duration::from_millis(100)))
			.unwrap();
		let mut prediction = frame(1500);
		prediction.frame.keyframe = false;
		assert_eq!(
			queue.push_at(prediction, now, Some(now + Duration::from_millis(80))),
			Err(7)
		);
		assert!(queue.deadline().is_none());
		queue.push_at(frame(0), now, Some(now)).unwrap();
		assert!(
			queue
				.pop(now + MAX_LATE + Duration::from_millis(1))
				.is_err()
		);
		assert!(queue.deadline().is_none());
	}
}
