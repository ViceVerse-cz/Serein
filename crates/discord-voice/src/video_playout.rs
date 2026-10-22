//! Watched-video input pacing. Decode intact access units on the RTP clock instead of
//! dumping repaired/network bursts into a latest-picture display slot. No media is saved.
use crate::video_receive::Encoded;
use std::{
	collections::VecDeque,
	time::{Duration, Instant},
};

const BUFFER: Duration = crate::stream_sync::BUFFER;
const MAX_AHEAD: Duration = Duration::from_millis(500);
const MAX_LATE: Duration = Duration::from_millis(250);
// A/V alignment can legitimately hold up to 500 ms: 30 pictures at 60 fps,
// before packet-repair/scheduling bursts. Keep that within the item budget too;
// the independent 8 MiB allocated-payload cap still bounds retained media.
const MAX_FRAMES: usize = 64;
const MAX_BYTES: usize = 8 * 1024 * 1024;

pub(crate) struct Timed {
	pub frame: Encoded,
	pub source: u32,
	pub timestamp: u32,
}
struct Clock {
	user: u64,
	source: u32,
	timestamp: u32,
	deadline: Instant,
}
struct Pending {
	frame: Encoded,
	deadline: Instant,
}
#[derive(Default)]
pub(crate) struct Stats {
	pub late: u64,
	pub resets: u64,
	pub dropped: u64,
	pub depth: u64,
}
#[derive(Default)]
pub(crate) struct Playout {
	clock: Option<Clock>,
	queue: VecDeque<Pending>,
	bytes: usize,
	stats: Stats,
}
impl Playout {
	pub fn push(&mut self, timed: Timed, now: Instant) -> Result<(), u64> {
		self.push_at(timed, now, None)
	}
	pub fn push_at(
		&mut self,
		timed: Timed,
		now: Instant,
		synced: Option<Instant>,
	) -> Result<(), u64> {
		let Timed {
			frame,
			source,
			timestamp,
		} = timed;
		let user = frame.user;
		let target = synced
			.or_else(|| {
				self.clock.as_ref().and_then(|clock| {
					let delta = timestamp.wrapping_sub(clock.timestamp);
					(clock.user == user && clock.source == source && delta < 0x8000_0000).then(
						|| {
							clock.deadline
								+ Duration::from_nanos(u64::from(delta) * 1_000_000_000 / 90_000)
						},
					)
				})
			})
			.filter(|target| {
				*target <= now + MAX_AHEAD && now.saturating_duration_since(*target) <= MAX_LATE
			});
		let deadline = match target {
			Some(target) => target,
			None => {
				// Re-anchoring an empty queue preserves all decoder references. Replacing
				// queued pictures needs an IDR, since those references would be skipped.
				if !self.queue.is_empty() && !frame.keyframe {
					self.clear();
					return Err(user);
				}
				self.clear();
				now + BUFFER
			}
		};
		if self
			.queue
			.back()
			.is_some_and(|pending| deadline < pending.deadline)
			|| (!self.queue.is_empty()
				&& self
					.clock
					.as_ref()
					.is_some_and(|clock| clock.user != user || clock.source != source))
		{
			// A genuine clock/layer discontinuity cannot put predictions behind a
			// later deadline and silently invalidate their decoder references.
			self.clear();
			if !frame.keyframe {
				return Err(user);
			}
		}
		if self.queue.len() >= MAX_FRAMES || frame.data.capacity() > MAX_BYTES - self.bytes {
			self.clear();
			return Err(user);
		}
		self.stats.late += u64::from(deadline < now);
		self.clock = Some(Clock {
			user,
			source,
			timestamp,
			deadline,
		});
		self.bytes += frame.data.capacity();
		self.queue.push_back(Pending { frame, deadline });
		self.stats.depth = self.stats.depth.max(self.queue.len() as u64);
		Ok(())
	}
	pub fn deadline(&self) -> Option<Instant> {
		self.queue.front().map(|pending| pending.deadline)
	}
	pub fn pop(&mut self, now: Instant) -> Result<Option<Encoded>, u64> {
		let Some(pending) = self.queue.front() else {
			return Ok(None);
		};
		if now < pending.deadline {
			return Ok(None);
		}
		if now.saturating_duration_since(pending.deadline) > MAX_LATE {
			let user = pending.frame.user;
			self.clear();
			return Err(user);
		}
		let pending = self.queue.pop_front().expect("front was ready");
		self.bytes -= pending.frame.data.capacity();
		Ok(Some(pending.frame))
	}
	/// Rekeys, teardown and decoder rejection discard queued plaintext immediately.
	pub fn clear(&mut self) -> bool {
		let discarded = !self.queue.is_empty();
		self.stats.resets += u64::from(self.clock.take().is_some());
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
	fn frame(timestamp: u32) -> Timed {
		Timed {
			frame: Encoded {
				user: 7,
				data: vec![1],
				keyframe: true,
			},
			source: 700,
			timestamp,
		}
	}
	#[test]
	fn burst_pictures_leave_on_the_source_clock_at_30_and_60_fps() {
		for step in [1500, 3000] {
			let now = Instant::now();
			let mut playout = Playout::default();
			let first = u32::MAX - step;
			for n in 0u32..4 {
				playout
					.push(frame(first.wrapping_add(n * step)), now)
					.unwrap();
			}
			for n in 0u32..4 {
				let due = now
					+ BUFFER + Duration::from_nanos(
					u64::from(n) * u64::from(step) * 1_000_000_000 / 90_000,
				);
				// Nanosecond rounding accumulates by at most 3 ns over these submissions.
				let actual = playout.deadline().unwrap();
				assert!(due.saturating_duration_since(actual) <= Duration::from_nanos(3));
				assert!(
					playout
						.pop(actual - Duration::from_nanos(1))
						.unwrap()
						.is_none()
				);
				assert!(playout.pop(actual).unwrap().is_some());
			}
			assert!(playout.deadline().is_none());
			assert_eq!(playout.take_stats().dropped, 0);
		}
	}
	#[test]
	fn late_repairs_can_join_before_their_deadline_without_resetting_the_clock() {
		let now = Instant::now();
		let mut playout = Playout::default();
		playout.push(frame(0), now).unwrap();
		playout
			.push(frame(3000), now + Duration::from_millis(80))
			.unwrap();
		playout
			.push(frame(6000), now + Duration::from_millis(80))
			.unwrap();
		assert_eq!(playout.deadline(), Some(now + BUFFER));
		for _ in 0..3 {
			let due = playout.deadline().unwrap();
			assert!(playout.pop(due).unwrap().is_some());
		}
		let stats = playout.take_stats();
		assert_eq!((stats.late, stats.resets, stats.dropped), (0, 0, 0));
	}
	#[test]
	fn pressure_discontinuity_and_long_stalls_require_reference_recovery() {
		let now = Instant::now();
		for large in [false, true] {
			let mut playout = Playout::default();
			let limit = if large { 8 } else { MAX_FRAMES };
			for _ in 0..limit {
				let mut timed = frame(0);
				if large {
					timed.frame.data = vec![1; 1024 * 1024];
				}
				playout.push(timed, now).unwrap();
			}
			let mut extra = frame(0);
			if large {
				extra.frame.data = vec![1; 1024 * 1024];
			}
			assert_eq!(playout.push(extra, now), Err(7));
			assert!(playout.deadline().is_none());
			assert_eq!(playout.take_stats().dropped, limit as u64);
		}
		let mut playout = Playout::default();
		playout.push(frame(0), now).unwrap();
		assert!(matches!(
			playout.pop(now + BUFFER + MAX_LATE + Duration::from_millis(1)),
			Err(7)
		));
		assert!(playout.deadline().is_none());
		playout.push(frame(0), now).unwrap();
		let mut changed = frame(1000);
		changed.source = 800;
		changed.frame.keyframe = false;
		assert_eq!(playout.push(changed, now), Err(7));
		assert!(playout.deadline().is_none());
		playout.push(frame(0), now).unwrap();
		let mut backwards = frame(1500);
		backwards.frame.keyframe = false;
		assert_eq!(
			playout.push_at(backwards, now, Some(now + BUFFER / 2)),
			Err(7)
		);
		assert!(playout.deadline().is_none());
	}
	#[test]
	fn rekeys_clear_plaintext_and_an_idle_clock_can_reanchor_without_dropping_references() {
		let now = Instant::now();
		let mut playout = Playout::default();
		playout.push(frame(0), now).unwrap();
		assert!(playout.clear());
		assert_eq!(playout.bytes, 0);
		assert!(playout.queue.is_empty());
		playout.push(frame(0), now).unwrap();
		playout.pop(now + BUFFER).unwrap();
		let mut changed = frame(1);
		changed.frame.keyframe = false;
		playout.push(changed, now + Duration::from_secs(2)).unwrap();
		assert_eq!(
			playout.deadline(),
			Some(now + Duration::from_secs(2) + BUFFER)
		);
	}
}
