use discord_voice::camera::{Camera, Frame};
use model::voice_settings::CameraFrameRate;
use std::{
	sync::{Arc, mpsc},
	time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
	match std::env::args().nth(1).as_deref() {
		Some("15") => probe(CameraFrameRate::Fps15),
		Some("30") | None => probe(CameraFrameRate::Fps30),
		Some("cycle") => {
			probe(CameraFrameRate::Fps15)?;
			probe(CameraFrameRate::Fps30)
		}
		_ => Err("Choose 15, 30, or cycle".into()),
	}
}

fn probe(rate: CameraFrameRate) -> Result<(), Box<dyn std::error::Error>> {
	let (send, receive) = mpsc::sync_channel(256);
	let on_frame = Arc::new(move |frame: Frame| {
		let _ = send.try_send(!frame.h264.is_empty());
	});
	let camera =
		Camera::start_with_frame_rate(Some("/dev/video0".into()), rate, on_frame, Arc::new(|| {}))
			.map_err(std::io::Error::other)?;
	let start = Instant::now();
	let deadline = start + Duration::from_secs(8);
	let mut first = None;
	let mut last = None;
	let mut frames = 0;
	let mut encoded = 0;
	while Instant::now() < deadline {
		match receive.recv_timeout(Duration::from_millis(250)) {
			Ok(has_encoded_frame) if start.elapsed() >= Duration::from_secs(1) => {
				let now = Instant::now();
				first.get_or_insert(now);
				last = Some(now);
				frames += 1;
				if has_encoded_frame {
					encoded += 1;
				}
			}
			Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) if camera.error().is_none() => {}
			_ => break,
		}
	}
	let error = camera.error();
	camera.stop();
	let shutdown_deadline = Instant::now() + Duration::from_secs(5);
	while !camera.stopped() && Instant::now() < shutdown_deadline {
		let _ = receive.recv_timeout(Duration::from_millis(50));
	}
	if !camera.stopped() {
		return Err("Camera did not stop cleanly".into());
	}
	if let Some(error) = error {
		return Err(error.into());
	}
	let elapsed = last
		.zip(first)
		.map(|(last, first): (Instant, Instant)| last.duration_since(first))
		.ok_or("Camera did not deliver enough frames")?;
	if frames < 2 || elapsed.is_zero() {
		return Err("Camera did not deliver enough frames".into());
	}
	let fps = (frames - 1) as f64 / elapsed.as_secs_f64();
	println!(
		"Camera target: {} fps; preview: {fps:.2} fps; encoded frames: {encoded}/{frames}",
		rate.fps()
	);
	if !(f64::from(rate.fps()) * 0.9..=f64::from(rate.fps()) * 1.1).contains(&fps) {
		return Err("Camera did not sustain the selected frame-rate target".into());
	}
	Ok(())
}
