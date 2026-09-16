//! Native capture selection follows the encoder, with a hard 720p ceiling.
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
	if !(1..=MAX_WIDTH).contains(&width) || !(1..=MAX_HEIGHT).contains(&height) {
		return None;
	}
	nearest_fps(fps, fps)?;
	Some((
		width.abs_diff(WIDTH).pow(2) + height.abs_diff(HEIGHT).pow(2),
		((fps - f64::from(FPS)).abs() * 1_000_000.0) as u64,
		width * height,
	))
}
