//! Offline capture-selection debug check; never opens a camera.
use discord_voice::camera::{HEIGHT, WIDTH};
#[path = "../src/camera/format.rs"]
mod format;

fn main() {
	let target_fps = model::voice_settings::CameraFrameRate::default().fps();
	let rank = |width, height, fps| {
		format::rank_with_ceiling_for(
			width,
			height,
			fps,
			format::MAX_WIDTH,
			format::MAX_HEIGHT,
			target_fps,
		)
	};
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
	assert_eq!(selected, (WIDTH, HEIGHT, f64::from(target_fps)));
	assert!(rank(800, 600, 15.0) < rank(320, 240, 15.0));
	for (width, height) in [
		(0, 480),
		(640, 0),
		(1281, 720),
		(1280, 721),
		(usize::MAX, 480),
	] {
		assert!(rank(width, height, 15.0).is_none());
	}
	assert!(rank(1280, 720, 15.0).is_some());
	assert_eq!(
		format::nearest_fps_for(5.0, 30.0, target_fps),
		Some(f64::from(target_fps))
	);
	assert_eq!(
		format::nearest_fps_for(24.0, 60.0, target_fps),
		Some(f64::from(target_fps))
	);
	assert_eq!(format::nearest_fps_for(1.0, 10.0, target_fps), Some(10.0));
	for (min, max) in [
		(0.0, 30.0),
		(30.0, 15.0),
		(f64::NAN, 30.0),
		(15.0, f64::INFINITY),
	] {
		assert!(format::nearest_fps_for(min, max, target_fps).is_none());
	}
	println!(
		"Camera selection: closest stream dimensions/rate, 720p ceiling, invalid modes rejected."
	);
}
