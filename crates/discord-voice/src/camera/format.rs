//! Native capture selection follows the encoder within each adapter's bounded input ceiling.
use super::{HEIGHT, WIDTH};
use std::time::{Duration, Instant};

pub(super) const FPS: u32 = 15;
pub(super) const MAX_WIDTH: usize = 1280;
pub(super) const MAX_HEIGHT: usize = 720;
pub(super) const MAX_CAPTURE_WIDTH: usize = 7680;
pub(super) const MAX_CAPTURE_HEIGHT: usize = 4320;
pub(super) const MAX_STRIDE_PADDING: usize = 4096;
pub(super) const MAX_RAW_BYTES: usize =
	(MAX_CAPTURE_WIDTH * 4 + MAX_STRIDE_PADDING) * MAX_CAPTURE_HEIGHT;

/// A native picture earns its own dimensions plus bounded row padding, never the
/// full 8K allocation allowance merely because larger presets are available.
pub(super) fn raw_budget(width: usize, height: usize) -> Option<usize> {
	if !(1..=MAX_CAPTURE_WIDTH).contains(&width) || !(1..=MAX_CAPTURE_HEIGHT).contains(&height) {
		return None;
	}
	width
		.checked_mul(4)?
		.checked_add(MAX_STRIDE_PADDING)?
		.checked_mul(height)
		.filter(|bytes| *bytes <= MAX_RAW_BYTES)
}

pub(super) fn nearest_fps(min: f64, max: f64) -> Option<f64> {
	nearest_fps_for(min, max, FPS)
}

/// Native rates may be lower than the selected delivery ceiling.
pub(super) fn nearest_fps_for(min: f64, max: f64, target: u32) -> Option<f64> {
	frame_interval(target)?;
	(min.is_finite() && max.is_finite() && min > 0.0 && min <= max)
		.then(|| f64::from(target).clamp(min, max))
}

pub(super) fn frame_interval(fps: u32) -> Option<Duration> {
	matches!(fps, 15 | 30 | 60)
		.then(|| Duration::from_nanos(1_000_000_000_u64.div_ceil(u64::from(fps))))
}

/// Carry the nominal deadline forward so tiny driver jitter does not halve the
/// delivered rate. Tolerance is at most 1 ms; missed periods never trigger bursts.
pub(super) struct Cadence {
	next: Instant,
	interval: Duration,
	tolerance: Duration,
}

impl Cadence {
	pub(super) fn new(fps: u32, now: Instant) -> Option<Self> {
		let interval = frame_interval(fps)?;
		Some(Self {
			next: now,
			interval,
			tolerance: Duration::from_millis(1).min(interval / 8),
		})
	}

	pub(super) fn accept(&mut self, now: Instant) -> bool {
		if self.next.saturating_duration_since(now) > self.tolerance {
			return false;
		}
		self.next = (self.next + self.interval).max(now + self.interval - self.tolerance);
		true
	}
}

/// Resolution first, then frame rate, then smaller input on ties.
pub(super) fn rank(width: usize, height: usize, fps: f64) -> Option<(usize, u64, usize)> {
	rank_with_ceiling(width, height, fps, MAX_WIDTH, MAX_HEIGHT)
}

/// Preserve adapters' existing bounded fallback modes when their input budgets permit it.
pub(super) fn rank_with_ceiling(
	width: usize,
	height: usize,
	fps: f64,
	max_width: usize,
	max_height: usize,
) -> Option<(usize, u64, usize)> {
	if !(1..=max_width).contains(&width) || !(1..=max_height).contains(&height) {
		return None;
	}
	nearest_fps(fps, fps)?;
	Some((
		width.abs_diff(WIDTH).pow(2) + height.abs_diff(HEIGHT).pow(2),
		((fps - f64::from(FPS)).abs() * 1_000_000.0) as u64,
		width * height,
	))
}

/// Higher presets require enough native pixels; a small camera is never enlarged to 8K.
/// The default retains its existing bounded compatibility modes.
pub(super) fn rank_for_output(
	width: usize,
	height: usize,
	fps: f64,
	output: (usize, usize),
) -> Option<(usize, u64, usize)> {
	if output == (WIDTH, HEIGHT) {
		return rank(width, height, fps);
	}
	rank_for_output_with_ceiling(
		width,
		height,
		fps,
		output,
		(MAX_CAPTURE_WIDTH, MAX_CAPTURE_HEIGHT),
	)
}

pub(super) fn rank_for_output_at_rate(
	width: usize,
	height: usize,
	fps: f64,
	output: (usize, usize),
	target_fps: u32,
) -> Option<(usize, u64, usize)> {
	if target_fps == FPS {
		return rank_for_output(width, height, fps, output);
	}
	if output == (WIDTH, HEIGHT) {
		return rank_for_output_with_ceiling_at_rate(
			width,
			height,
			fps,
			output,
			(MAX_WIDTH, MAX_HEIGHT),
			target_fps,
		);
	}
	rank_for_output_with_ceiling_at_rate(
		width,
		height,
		fps,
		output,
		(MAX_CAPTURE_WIDTH, MAX_CAPTURE_HEIGHT),
		target_fps,
	)
}

pub(super) fn rank_for_output_with_ceiling(
	width: usize,
	height: usize,
	fps: f64,
	output: (usize, usize),
	ceiling: (usize, usize),
) -> Option<(usize, u64, usize)> {
	rank_for_output_with_ceiling_at_rate(width, height, fps, output, ceiling, FPS)
}

pub(super) fn rank_for_output_with_ceiling_at_rate(
	width: usize,
	height: usize,
	fps: f64,
	output: (usize, usize),
	ceiling: (usize, usize),
	target_fps: u32,
) -> Option<(usize, u64, usize)> {
	let (target_width, target_height) = output;
	let (max_width, max_height) = ceiling;
	if !(1..=MAX_CAPTURE_WIDTH).contains(&target_width)
		|| !(1..=MAX_CAPTURE_HEIGHT).contains(&target_height)
		|| !(1..=max_width.min(MAX_CAPTURE_WIDTH)).contains(&width)
		|| !(1..=max_height.min(MAX_CAPTURE_HEIGHT)).contains(&height)
		|| (output != (WIDTH, HEIGHT) && (width < target_width || height < target_height))
	{
		return None;
	}
	nearest_fps_for(fps, fps, target_fps)?;
	Some((
		width.abs_diff(target_width).pow(2) + height.abs_diff(target_height).pow(2),
		((fps - f64::from(target_fps)).abs() * 1_000_000.0) as u64,
		width * height,
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn selected_rates_follow_native_ranges_without_promising_unsupported_fps() {
		for target in [15, 30, 60] {
			assert_eq!(nearest_fps_for(5.0, 60.0, target), Some(f64::from(target)));
			assert_eq!(
				nearest_fps_for(24.0, 30.0, target),
				Some(f64::from(target).clamp(24.0, 30.0))
			);
			let selected = [15.0, 30.0, 60.0]
				.into_iter()
				.min_by_key(|fps| {
					rank_for_output_at_rate(WIDTH, HEIGHT, *fps, (WIDTH, HEIGHT), target)
				})
				.unwrap();
			assert_eq!(selected, f64::from(target));
			assert_eq!(
				frame_interval(target).unwrap().as_nanos(),
				1_000_000_000_u128.div_ceil(u128::from(target))
			);
		}
		for target in [0, 1, 14, 16, 29, 31, 59, 61, u32::MAX] {
			assert!(frame_interval(target).is_none());
			assert!(nearest_fps_for(1.0, 60.0, target).is_none());
			assert!(
				rank_for_output_at_rate(WIDTH, HEIGHT, 30.0, (WIDTH, HEIGHT), target).is_none()
			);
		}
		for (min, max) in [
			(0.0, 60.0),
			(60.0, 30.0),
			(f64::NAN, 60.0),
			(15.0, f64::INFINITY),
		] {
			assert!(nearest_fps_for(min, max, 60).is_none());
		}
		assert_eq!(nearest_fps_for(30.0, 30.0, 60), Some(30.0));
		assert_eq!(nearest_fps_for(60.0, 60.0, 30), Some(60.0));
		assert!(rank_for_output_at_rate(640, 480, 60.0, (7680, 4320), 60).is_none());
	}

	#[test]
	fn cadence_keeps_native_fps_under_jitter_and_drops_faster_sources_without_bursts() {
		let start = Instant::now();
		for fps in [15, 30, 60] {
			let mut cadence = Cadence::new(fps, start).unwrap();
			for index in 0..u64::from(fps) * 10 {
				let nominal = start + Duration::from_nanos(index * 1_000_000_000 / u64::from(fps));
				let actual = if index.is_multiple_of(2) {
					nominal
				} else {
					nominal - Duration::from_micros(300)
				};
				assert!(
					cadence.accept(actual),
					"native {fps} fps frame {index} was dropped"
				);
				assert!(
					!cadence.accept(actual),
					"duplicate timestamps must not emit twice"
				);
			}
		}
		let mut cadence = Cadence::new(60, start).unwrap();
		let emitted = (0..120)
			.filter(|index| {
				cadence.accept(start + Duration::from_nanos(index * 1_000_000_000 / 120))
			})
			.count();
		assert_eq!(emitted, 60);
		let stalled = start + Duration::from_secs(5);
		assert!(cadence.accept(stalled));
		assert!(!cadence.accept(stalled));
		assert!(!cadence.accept(stalled + Duration::from_millis(5)));
		assert!(cadence.accept(stalled + frame_interval(60).unwrap()));
		let mut cadence = Cadence::new(60, start).unwrap();
		let late = start + frame_interval(60).unwrap() - Duration::from_micros(500);
		assert!(cadence.accept(late));
		assert!(!cadence.accept(late));
		assert!(!cadence.accept(late + Duration::from_millis(5)));
		assert!(Cadence::new(0, start).is_none());
	}

	#[test]
	fn native_raw_budget_follows_geometry_and_rejects_overflow() {
		assert_eq!(raw_budget(640, 480), Some((640 * 4 + 4096) * 480));
		assert_eq!(raw_budget(7680, 4320), Some(MAX_RAW_BYTES));
		assert!(raw_budget(640, 480).unwrap() < MAX_RAW_BYTES / 40);
		assert_eq!(raw_budget(0, 1), None);
		assert_eq!(raw_budget(7681, 4320), None);
		assert_eq!(raw_budget(7680, 4321), None);
		assert_eq!(raw_budget(usize::MAX, usize::MAX), None);
	}

	#[test]
	fn high_resolution_requires_sufficient_native_pixels() {
		for target in [
			(1280, 720),
			(1920, 1080),
			(2560, 1440),
			(3840, 2160),
			(7680, 4320),
		] {
			assert!(rank_for_output(target.0, target.1, 15.0, target).is_some());
			assert!(rank_for_output(target.0 - 1, target.1, 15.0, target).is_none());
			assert!(rank_for_output(target.0, target.1 - 1, 15.0, target).is_none());
			assert!(rank_for_output(640, 480, 15.0, target).is_none());
		}
		assert!(rank_for_output(3840, 2160, 15.0, (1920, 1080)).is_some());
		assert!(rank_for_output(7681, 4320, 15.0, (7680, 4320)).is_none());
		assert!(rank_for_output(7680, 4321, 15.0, (7680, 4320)).is_none());
		assert!(rank_for_output(usize::MAX, 4320, 15.0, (7680, 4320)).is_none());
		assert!(rank_for_output(320, 240, 15.0, (WIDTH, HEIGHT)).is_some());
	}

	#[test]
	fn native_modes_follow_stream_dimensions_then_actual_frame_rate() {
		let modes = [
			(3840, 2048, 15.0),
			(1280, 720, 30.0),
			(640, 480, 30.0),
			(640, 480, 15.0),
		];
		let selected = modes
			.into_iter()
			.filter_map(|mode| rank(mode.0, mode.1, mode.2).map(|rank| (rank, mode)))
			.min_by_key(|(rank, _)| *rank)
			.unwrap()
			.1;
		assert_eq!(selected, (WIDTH, HEIGHT, f64::from(FPS)));
		assert!(rank(640, 480, 15.0) < rank(640, 480, 30.0));
		assert!(rank(800, 600, 15.0) < rank(320, 240, 15.0));
		assert!(rank_with_ceiling(1920, 1080, 15.0, 1920, 1080).is_some());
		assert!(rank_with_ceiling(3840, 2160, 15.0, 1920, 1080).is_none());
		assert_eq!(nearest_fps(5.0, 30.0), Some(15.0));
		assert_eq!(nearest_fps(24.0, 60.0), Some(24.0));
		assert_eq!(nearest_fps(1.0, 10.0), Some(10.0));
		for (width, height) in [
			(0, HEIGHT),
			(WIDTH, 0),
			(MAX_WIDTH + 1, HEIGHT),
			(WIDTH, MAX_HEIGHT + 1),
			(usize::MAX, HEIGHT),
		] {
			assert!(rank(width, height, 15.0).is_none());
		}
		for (min, max) in [
			(0.0, 30.0),
			(30.0, 15.0),
			(f64::NAN, 30.0),
			(15.0, f64::INFINITY),
		] {
			assert!(nearest_fps(min, max).is_none());
		}
	}
}
