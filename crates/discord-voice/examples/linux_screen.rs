//! Offline debug check for the actual Linux pipeline and portal cancellation.
//! No screen picker, display capture, microphone or network connection is opened.
// This runnable debug example includes implementation modules, not their unit-test harnesses.
#![cfg(not(test))]
#![allow(dead_code)]
#[cfg(unix)]
#[path = "../src/screen.rs"]
mod screen;
#[cfg(unix)]
use screen::{
	AudioChunk, EncodedFrame, MAX_AUDIO_SAMPLES, MAX_ENCODED_BYTES, MAX_RAW_BYTES, RawFrame,
	Settings, SourceId, encode_pixels, encoder, preview_frame,
};
#[cfg(unix)]
#[path = "../src/screen/audio_linux.rs"]
mod audio_linux;
#[cfg(unix)]
#[path = "../src/screen/gstreamer.rs"]
mod gstreamer;
#[cfg(unix)]
#[path = "../src/screen/linux.rs"]
mod linux;
#[cfg(unix)]
#[path = "../src/screen/portal_linux.rs"]
mod portal_linux;
#[cfg(unix)]
#[path = "../src/video.rs"]
mod video;
// The shared screen module reaches the platform encoders' keyframe check through this path.
#[cfg(unix)]
#[path = "../src/video_receive.rs"]
mod video_receive;
// Non-Linux builds of this offline example still compile the shared hardware encoder facade.
#[cfg(unix)]
#[path = "../src/video_encode.rs"]
mod video_encode;
// The application-audio worker reports its capture counters through the shared reporter.
#[cfg(unix)]
#[path = "../src/diagnostics.rs"]
mod diagnostics;

#[cfg(unix)]
fn main() {
	use ::gstreamer as gst;
	use gstreamer::{Capture, Mode};
	use std::{
		sync::{
			Arc,
			atomic::{AtomicBool, AtomicU64, Ordering},
		},
		time::{Duration, Instant},
	};
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap();
	runtime.block_on(async {
		assert_eq!(portal_linux::Portal::open(true, &AtomicBool::new(true)).await.err(), Some("Screen sharing was cancelled."));
		gst::init().unwrap();
		let settings = Settings { source: SourceId::Display(1), width: 1280, height: 720, fps: 30, cursor: true, audio: false };
		let stop = Arc::new(AtomicBool::new(false));
		let ready = Arc::new(AtomicBool::new(false));
		let keyframe = Arc::new(AtomicBool::new(true));
		let source = gst::ElementFactory::make("videotestsrc").property("is-live", true).build().unwrap();
		let pipeline = Capture::new(settings, Mode::Software, source, stop.clone(), ready.clone(), keyframe.clone(), || true).unwrap();
		let deadline = Instant::now() + Duration::from_secs(5);
		let mut saw_preview = false;
		while Instant::now() < deadline && !saw_preview {
			assert!(!pipeline.failed());
			assert!(pipeline.frames.try_pull_sample(gst::ClockTime::ZERO).is_none(), "must not encode before secure readiness");
			if let Some(sample) = pipeline.preview.try_pull_sample(gst::ClockTime::ZERO) {
				let raw = gstreamer::raw(&sample).unwrap();
				assert_eq!((raw.width, raw.height), (640, 360));
				assert_eq!(preview_frame(&raw).unwrap().as_raw().len(), 640 * 360 * 4);
				saw_preview = true;
			}
			pipeline.changed().await;
		}
		assert!(saw_preview, "synthetic preview did not arrive");
		ready.store(true, Ordering::Release);
		let deadline = Instant::now() + Duration::from_secs(5);
		let mut encoded = false;
		while Instant::now() < deadline && !encoded {
			assert!(!pipeline.failed());
			if let Some(sample) = pipeline.frames.try_pull_sample(gst::ClockTime::ZERO) {
				let raw = gstreamer::raw(&sample).unwrap();
				assert_eq!((raw.width, raw.height), (1280, 720));
				let mut encoder = encoder(settings).unwrap();
				let mut yuv = openh264::formats::YUVBuffer::new(1280, 720);
				let (data, keyframe) = encode_pixels(&mut encoder, &mut yuv, &raw.data, (1280, 720), true).unwrap();
				assert!(keyframe);
				video::validate_source(&data).unwrap();
				encoded = true;
			}
			pipeline.changed().await;
		}
		assert!(encoded, "synthetic frame did not encode");
		ready.store(false, Ordering::Release);
		let (send, _receive) = tokio::sync::mpsc::channel(4);
		let epoch = Arc::new(AtomicU64::new(0));
		audio_linux::check_isolation();
		stop.store(true, Ordering::Release);
		drop(pipeline);
		let mut cancelled = audio_linux::Worker::start(send, stop.clone(), ready.clone(), epoch).unwrap();
		let deadline = Instant::now() + Duration::from_secs(3);
		loop {
			if let Some(result) = cancelled.result() { result.unwrap(); break; }
			assert!(Instant::now() < deadline, "cancelled audio worker must retire without opening a device");
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
		println!("Linux screen pipeline: synthetic preview, secure-readiness gates, application audio exclusion/bounded stereo mixing, software H.264 and portal pre-cancellation passed. Native Linux capture/GPU encoding remains unverified.");
	});
}
#[cfg(not(unix))]
fn main() {
	eprintln!("This debug check requires Linux or macOS with GStreamer.");
}
