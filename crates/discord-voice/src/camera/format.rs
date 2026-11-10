//! Native capture selection follows the encoder within each adapter's bounded input ceiling.
use super::{HEIGHT, WIDTH};

pub(super) const MAX_WIDTH: usize = 1280;
pub(super) const MAX_HEIGHT: usize = 720;

pub(super) fn nearest_fps_for(min: f64, max: f64, target: u32) -> Option<f64> {
	(min.is_finite() && max.is_finite() && min > 0.0 && min <= max)
		.then(|| f64::from(target).clamp(min, max))
}

/// Rank bounded modes by dimensions, then target rate, then smaller input.
pub(super) fn rank_with_ceiling_for(
	width: usize,
	height: usize,
	fps: f64,
	max_width: usize,
	max_height: usize,
	target: u32,
) -> Option<(usize, u64, usize)> {
	if !(1..=max_width).contains(&width) || !(1..=max_height).contains(&height) {
		return None;
	}
	nearest_fps_for(fps, fps, target)?;
	Some((
		width.abs_diff(WIDTH).pow(2) + height.abs_diff(HEIGHT).pow(2),
		((fps - f64::from(target)).abs() * 1_000_000.0) as u64,
		width * height,
	))
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::voice_settings::CameraFrameRate;

	#[test]
	fn native_modes_follow_stream_dimensions_then_actual_frame_rate() {
		let target_fps = CameraFrameRate::default().fps();
		let rank = |width, height, fps| {
			rank_with_ceiling_for(width, height, fps, MAX_WIDTH, MAX_HEIGHT, target_fps)
		};
		let modes = [
			(3840, 2048, 15.0),
			(1280, 720, 15.0),
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
		assert_eq!(selected, (WIDTH, HEIGHT, f64::from(target_fps)));
		assert!(rank(640, 480, 30.0) < rank(640, 480, 15.0));
		assert!(rank(800, 600, 15.0) < rank(320, 240, 15.0));
		assert!(rank_with_ceiling_for(1920, 1080, 15.0, 1920, 1080, 15).is_some());
		assert!(rank_with_ceiling_for(3840, 2160, 15.0, 1920, 1080, 15).is_none());
		assert_eq!(nearest_fps_for(5.0, 30.0, target_fps), Some(30.0));
		assert_eq!(nearest_fps_for(24.0, 60.0, target_fps), Some(30.0));
		assert_eq!(nearest_fps_for(1.0, 10.0, target_fps), Some(10.0));
		assert_eq!(nearest_fps_for(5.0, 30.0, 15), Some(15.0));
		assert!(
			rank_with_ceiling_for(640, 480, 15.0, MAX_WIDTH, MAX_HEIGHT, 15)
				< rank_with_ceiling_for(640, 480, 30.0, MAX_WIDTH, MAX_HEIGHT, 15)
		);
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
			assert!(nearest_fps_for(min, max, target_fps).is_none());
		}
	}
}
