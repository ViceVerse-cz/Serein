//! Offline capture-selection debug check; never opens a camera.
use discord_voice::camera::{HEIGHT, WIDTH};
use std::time::{Duration, Instant};
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
	for resolution in model::voice_settings::VideoResolution::ALL {
		let (width, height) = resolution.camera_dimensions();
		let dimensions = (width as usize, height as usize);
		assert!(format::rank_for_output(dimensions.0, dimensions.1, 15.0, dimensions).is_some());
		assert!(format::raw_budget(dimensions.0, dimensions.1).unwrap() <= format::MAX_RAW_BYTES);
		if dimensions != (WIDTH, HEIGHT) {
			assert!(format::rank_for_output(WIDTH, HEIGHT, 15.0, dimensions).is_none());
		}
		for rate in model::voice_settings::VideoFrameRate::ALL {
			let fps = rate.fps();
			assert_eq!(
				format::nearest_fps_for(1.0, 60.0, fps),
				Some(f64::from(fps))
			);
			assert!(
				format::rank_for_output_at_rate(
					dimensions.0,
					dimensions.1,
					f64::from(fps),
					dimensions,
					fps
				)
				.is_some()
			);
			assert!(
				format::rank_for_output_with_ceiling_at_rate(
					dimensions.0,
					dimensions.1,
					f64::from(fps),
					dimensions,
					(7680, 4320),
					fps
				)
				.is_some()
			);
			let start = Instant::now();
			let mut cadence = format::Cadence::new(fps, start).unwrap();
			assert!(cadence.accept(start));
			assert!(!cadence.accept(start + Duration::from_millis(1)));
			assert!(cadence.accept(start + format::frame_interval(fps).unwrap()));
		}
	}
	for (min, max) in [
		(0.0, 30.0),
		(30.0, 15.0),
		(f64::NAN, 30.0),
		(15.0, f64::INFINITY),
	] {
		assert!(format::nearest_fps(min, max).is_none());
	}
	println!(
		"Camera selection: default compatibility, native modes through 8K at up to 60 fps, bounded buffers and cadence, invalid modes rejected."
	);
}
