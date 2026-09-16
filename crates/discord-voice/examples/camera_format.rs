//! Offline capture-selection debug check; never opens a camera.
use discord_voice::camera::{HEIGHT, WIDTH};
#[path = "../src/camera/format.rs"]
mod format;

fn main() {
	let modes = [
		(3840, 2048, 15.0),
		(1280, 720, 30.0),
		(640, 480, 30.0),
		(640, 480, 15.0),
	];
	let selected = modes
		.into_iter()
		.filter_map(|mode| format::rank(mode.0, mode.1, mode.2).map(|rank| (rank, mode)))
		.min_by_key(|(rank, _)| *rank)
		.unwrap()
		.1;
	assert_eq!(selected, (WIDTH, HEIGHT, 15.0));
	assert!(format::rank(800, 600, 15.0) < format::rank(320, 240, 15.0));
	for (width, height) in [
		(0, 480),
		(640, 0),
		(1281, 720),
		(1280, 721),
		(usize::MAX, 480),
	] {
		assert!(format::rank(width, height, 15.0).is_none());
	}
	assert!(format::rank(1280, 720, 15.0).is_some());
	assert_eq!(format::nearest_fps(5.0, 30.0), Some(15.0));
	assert_eq!(format::nearest_fps(24.0, 60.0), Some(24.0));
	assert_eq!(format::nearest_fps(1.0, 10.0), Some(10.0));
	for (min, max) in [
		(0.0, 30.0),
		(30.0, 15.0),
		(f64::NAN, 30.0),
		(15.0, f64::INFINITY),
	] {
		assert!(format::nearest_fps(min, max).is_none());
	}
	println!(
		"Camera selection: closest stream dimensions/rate, 720p ceiling, invalid modes rejected."
	);
}
