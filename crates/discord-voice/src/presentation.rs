//! Session-only media timing. The audio callback publishes device playback timing
//! with atomics; video rendering reads it without touching codecs or audio devices.
use std::{
	sync::{
		Arc,
		atomic::{AtomicI64, AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};

pub(crate) const INITIAL: Duration = Duration::from_millis(100);
pub(crate) const MAX_AHEAD: Duration = Duration::from_millis(500);
pub(crate) const MAX_LATE: Duration = Duration::from_millis(250);
pub(crate) const DECODE_LEAD: Duration = Duration::from_millis(40);

#[derive(Debug)]
pub(crate) struct Clock {
	epoch: Instant,
	generation: AtomicU64,
	// Device time minus logical media time, in microseconds; bounded to one second.
	offset: AtomicI64,
	seen: AtomicU64,
	played_generation: AtomicU64,
	audio_wait_until: AtomicU64,
}
impl Default for Clock {
	fn default() -> Self {
		Self {
			epoch: Instant::now(),
			generation: AtomicU64::new(1),
			offset: AtomicI64::new(0),
			seen: AtomicU64::new(0),
			played_generation: AtomicU64::new(0),
			audio_wait_until: AtomicU64::new(0),
		}
	}
}
impl Clock {
	pub fn reset(&self) {
		self.generation.fetch_add(1, Ordering::AcqRel);
		self.seen.store(0, Ordering::Release);
		self.audio_wait_until.store(0, Ordering::Release);
	}
	pub fn expect_audio(&self, now: Instant) {
		let until = (now + Duration::from_millis(200))
			.saturating_duration_since(self.epoch)
			.as_micros() as u64;
		let _ =
			self.audio_wait_until
				.compare_exchange(0, until, Ordering::Relaxed, Ordering::Relaxed);
	}
	pub fn stamp(self: &Arc<Self>, at: Instant) -> Presentation {
		Presentation {
			at,
			clock: self.clone(),
			generation: self.generation.load(Ordering::Acquire),
		}
	}
}

/// Intended presentation time, retained through decoded pictures and PCM queues.
/// Device timing is an OS estimate, not a measurement of physical speaker latency.
#[derive(Clone, Debug)]
pub struct Presentation {
	pub(crate) at: Instant,
	clock: Arc<Clock>,
	generation: u64,
}
impl Presentation {
	/// An independent presentation target, for sources without an audio master.
	pub fn new(at: Instant) -> Self {
		Arc::new(Clock::default()).stamp(at)
	}
	pub fn is_current(&self) -> bool {
		self.generation == self.clock.generation.load(Ordering::Acquire)
	}
	pub(crate) fn decode_deadline(&self, now: Instant, lead: Duration) -> Instant {
		let due = self.deadline(now) - lead;
		let wait = self.clock.audio_wait_until.load(Ordering::Relaxed);
		if self.clock.seen.load(Ordering::Acquire) == 0 && wait != 0 {
			due.max(self.clock.epoch + Duration::from_micros(wait))
		} else {
			due
		}
	}
	pub fn deadline(&self, now: Instant) -> Instant {
		let seen = self.clock.seen.load(Ordering::Acquire);
		let age = now.saturating_duration_since(self.clock.epoch).as_micros() as u64;
		let offset = if self.is_current()
			&& self.clock.played_generation.load(Ordering::Acquire) == self.generation
			&& seen != 0
			&& age.saturating_sub(seen) < 2_000_000
		{
			self.clock.offset.load(Ordering::Relaxed)
		} else {
			0
		};
		shift(self.at, i128::from(offset) * 1000).unwrap_or(self.at)
	}
	/// Called only as the first sample reaches the output callback. No locks,
	/// allocations, media retention or channels are involved.
	pub(crate) fn played(&self, at: Instant) {
		if !self.is_current() {
			return;
		}
		let offset = difference(at, self.at) / 1000;
		if offset.abs() > 1_000_000 {
			return;
		}
		let filtered = if self.clock.played_generation.load(Ordering::Acquire) == self.generation
			&& self.clock.seen.load(Ordering::Acquire) != 0
		{
			let old = self.clock.offset.load(Ordering::Relaxed);
			old + (offset as i64 - old) / 16
		} else {
			offset as i64
		};
		self.clock.offset.store(filtered, Ordering::Relaxed);
		self.clock.seen.store(
			at.saturating_duration_since(self.clock.epoch).as_micros() as u64 + 1,
			Ordering::Release,
		);
		self.clock
			.played_generation
			.store(self.generation, Ordering::Release);
	}
}

/// A normal call frame has no stream timestamp. A watched frame keeps its timestamp
/// while mixed with the call, queued to the device and resampled for its output rate.
#[derive(Clone, Debug)]
pub struct PlaybackFrame {
	pub samples: crate::Frame,
	pub presentation: Option<Presentation>,
}
impl From<crate::Frame> for PlaybackFrame {
	fn from(samples: crate::Frame) -> Self {
		Self {
			samples,
			presentation: None,
		}
	}
}

pub(crate) fn difference(a: Instant, b: Instant) -> i128 {
	if a >= b {
		a.duration_since(b).as_nanos() as i128
	} else {
		-(b.duration_since(a).as_nanos() as i128)
	}
}
pub(crate) fn shift(at: Instant, nanos: i128) -> Option<Instant> {
	let duration = Duration::from_nanos(u64::try_from(nanos.unsigned_abs()).ok()?);
	if nanos < 0 {
		at.checked_sub(duration)
	} else {
		at.checked_add(duration)
	}
}

/// A bounded jitter envelope: quick attack, slow release. RTP timestamps, not
/// packet count, determine how much media is being buffered. Reordered arrivals
/// do not move the estimator backwards and discontinuities start a new epoch.
pub(crate) struct Delay {
	last: Option<(u32, Instant)>,
	envelope: f64,
	target: f64,
}
impl Default for Delay {
	fn default() -> Self {
		Self {
			last: None,
			envelope: 0.0,
			target: 100.0,
		}
	}
}
impl Delay {
	pub fn observe(&mut self, timestamp: u32, rate: u32, now: Instant) {
		if let Some((last, at)) = self.last {
			let ticks = timestamp.wrapping_sub(last);
			if ticks == 0 || ticks >= 0x8000_0000 {
				return;
			}
			let elapsed = now.saturating_duration_since(at).as_secs_f64();
			let media = f64::from(ticks) / f64::from(rate);
			if elapsed > 1.0 || media > 1.0 {
				self.last = Some((timestamp, now));
				return;
			}
			let variation = ((elapsed - media) * 1000.0).abs();
			self.envelope = variation.max(self.envelope * (-elapsed / 10.0).exp());
			let desired = (60.0 + self.envelope * 2.0).clamp(60.0, 240.0);
			// At most 100 ms/s increase, 2 ms/s decrease; isolated packet bursts
			// cannot instantly move either playback clock or fill the whole queue.
			self.target += (desired - self.target).clamp(-elapsed * 2.0, elapsed * 100.0);
		}
		self.last = Some((timestamp, now));
	}
	pub fn target(&self) -> Duration {
		Duration::from_secs_f64(self.target / 1000.0)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn device_timing_moves_matching_video_and_expires_across_epochs() {
		let clock = Arc::new(Clock::default());
		let at = clock.epoch + Duration::from_millis(100);
		let audio = clock.stamp(at);
		let video = clock.stamp(at + Duration::from_millis(20));
		audio.played(at + Duration::from_millis(73));
		assert_eq!(
			video.deadline(at + Duration::from_millis(74)),
			at + Duration::from_millis(93)
		);
		clock.reset();
		audio.played(at + Duration::from_millis(200));
		assert!(!video.is_current());
		assert_eq!(clock.stamp(at).deadline(at), at);
	}
	#[test]
	fn jitter_attack_is_bounded_and_release_is_slow() {
		let start = Instant::now();
		let mut delay = Delay::default();
		for n in 0..1000u32 {
			// Four correctly ordered packets arrive together every 80 ms.
			let arrival = if n < 100 {
				u64::from(n) * 20
			} else {
				u64::from(n / 4) * 80 + 60
			};
			delay.observe(n * 960, 48_000, start + Duration::from_millis(arrival));
		}
		assert!(delay.target() > INITIAL);
		let high = delay.target();
		for n in 1000..1100u32 {
			delay.observe(
				n * 960,
				48_000,
				start + Duration::from_millis(u64::from(n) * 20),
			);
		}
		assert!(delay.target() >= high - Duration::from_millis(5));
		assert!(delay.target() <= Duration::from_millis(240));
	}
}
