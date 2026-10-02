//! Native capture selection follows the encoder within each adapter's bounded input ceiling.
use super::{HEIGHT, WIDTH};

pub(super) const FPS: u32 = 15;
pub(super) const MAX_WIDTH: usize = 1280;
pub(super) const MAX_HEIGHT: usize = 720;

pub(super) fn nearest_fps(min: f64, max: f64) -> Option<f64> {
	(min.is_finite() && max.is_finite() && min > 0.0 && min <= max)
		.then(|| f64::from(FPS).clamp(min, max))
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

#[cfg(test)]
mod tests {
	use super::*;

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
