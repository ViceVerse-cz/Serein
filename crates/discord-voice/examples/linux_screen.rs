//! Offline debug check for the actual Linux pipeline and portal cancellation.
//! No screen picker, display capture, microphone or network connection is opened.
// This runnable debug example includes implementation modules, not their unit-test harnesses.
#![cfg(not(test))]
#![allow(dead_code)]
#![allow(clippy::duplicate_mod)] // Crate-root shims and screen.rs share production modules.
// Retain the package's native C linkage when this harness includes its Rust modules.
use discord_voice as _;
#[cfg(target_os = "linux")]
#[path = "../src/screen.rs"]
mod screen;
#[cfg(target_os = "linux")]
use screen::{
	AudioChunk, EncodedFrame, MAX_AUDIO_SAMPLES, MAX_RAW_BYTES, RawFrame, ScreenEncoder, Settings,
	SourceId, preview_frame, retain_screen_frame,
};
#[cfg(target_os = "linux")]
#[path = "../src/screen/audio_linux.rs"]
mod audio_linux;
#[cfg(target_os = "linux")]
#[path = "../src/screen/gstreamer.rs"]
mod gstreamer;
#[cfg(target_os = "linux")]
#[path = "../src/screen/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
#[path = "../src/screen/portal_linux.rs"]
mod portal_linux;
#[cfg(target_os = "linux")]
#[path = "../src/video.rs"]
mod video;
// The shared screen module reaches the platform encoders' keyframe check through this path.
#[cfg(target_os = "linux")]
#[path = "../src/video_backend.rs"]
mod video_backend;
#[cfg(target_os = "linux")]
#[path = "../src/video_encode.rs"]
mod video_encode;
#[cfg(target_os = "linux")]
#[path = "../src/video_gpu.rs"]
mod video_gpu;
#[cfg(target_os = "linux")]
#[path = "../src/video_receive.rs"]
mod video_receive;
#[cfg(target_os = "linux")]
#[path = "../src/video_sps.rs"]
mod video_sps;
// The application-audio worker reports its capture counters through the shared reporter.
#[cfg(target_os = "linux")]
#[path = "../src/diagnostics.rs"]
mod diagnostics;
#[cfg(target_os = "linux")]
#[path = "../src/timer.rs"]
mod timer;

#[cfg(target_os = "linux")]
fn main() {
	use ::gstreamer as gst;
	use gst::prelude::*;
	use gstreamer::Capture;
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
		// Construct only: never transition this source out of Null or capture a desktop.
		let x11 = linux::x11_source(false).expect("install GStreamer Good for X11 capture");
		assert!(!x11.property::<bool>("show-pointer"));
		assert!(!x11.property::<bool>("use-damage"));
		assert_eq!(x11.property::<u64>("xid"), 0, "explicit whole-desktop source");
		drop(x11);
		let settings = Settings { source: SourceId::Display(1), width: 1280, height: 720, fps: 30, cursor: true, audio: false };
		let stop = Arc::new(AtomicBool::new(false));
		let ready = Arc::new(AtomicBool::new(false));
		let keyframe = Arc::new(AtomicBool::new(true));
		let source = gst::ElementFactory::make("videotestsrc").property("is-live", true).build().unwrap();
		// Niri 26.04 leaves SPA header PTS at zero. GstBaseSrc adds a constant
		// startup offset but does not replace that valid (stuck) presentation time.
		if std::env::args().any(|arg| arg == "--niri-timestamps") {
			source.static_pad("src").unwrap().add_probe(gst::PadProbeType::BUFFER, |_, info| {
				if let Some(gst::PadProbeData::Buffer(buffer)) = &mut info.data {
					buffer.make_mut().set_pts(gst::ClockTime::ZERO);
				}
				gst::PadProbeReturn::Ok
			});
			linux::timestamp_niri_frames(&source).unwrap();
		}
		let pipeline = Capture::new(settings, source, stop.clone(), ready.clone(), keyframe.clone(), || true).unwrap();
		let deadline = Instant::now() + Duration::from_secs(5);
		let mut previews = 0;
		let mut last_preview_pts = None;
		while Instant::now() < deadline && previews < 3 {
			assert!(!pipeline.failed());
			assert!(pipeline.frames.try_pull_sample(gst::ClockTime::ZERO).is_none(), "must not encode before secure readiness");
			if let Some(sample) = pipeline.preview.try_pull_sample(gst::ClockTime::ZERO) {
				let raw = gstreamer::raw(&sample).unwrap();
				assert_eq!((raw.width, raw.height), (640, 360));
				assert_eq!(preview_frame(&raw).unwrap().as_raw().len(), 640 * 360 * 4);
				let pts = sample.buffer().unwrap().pts().expect("preview timestamp");
				assert!(last_preview_pts.is_none_or(|last| pts > last), "preview must keep advancing");
				last_preview_pts = Some(pts);
				previews += 1;
			}
			pipeline.changed().await;
		}
		assert_eq!(previews, 3, "synthetic preview did not keep advancing");
		ready.store(true, Ordering::Release);
		let deadline = Instant::now() + Duration::from_secs(5);
		let mut encoded = 0;
		let mut last_pts = None;
		let mut encoder = ScreenEncoder::new_on_adapter(settings, settings.bit_rate(), model::voice_settings::VideoSettings::default(), None, 0, 0).unwrap();
		while Instant::now() < deadline && encoded < 5 {
			assert!(!pipeline.failed());
			if let Some(sample) = pipeline.frames.try_pull_sample(gst::ClockTime::ZERO) {
				let pts = sample.buffer().unwrap().pts().expect("frame timestamp");
				assert!(last_pts.is_none_or(|last| pts > last), "frames must keep advancing");
				last_pts = Some(pts);
				let raw = gstreamer::raw(&sample).unwrap();
				assert_eq!((raw.width, raw.height), (1280, 720));
				let packet = encoder.encode_at(&raw, true, 0).unwrap();
				if packet.data.is_empty() { continue; }
				assert!(packet.keyframe);
				video::validate_source(&packet.data).unwrap();
				encoded += 1;
			}
			pipeline.changed().await;
		}
		assert_eq!(encoded, 5, "synthetic frames did not keep encoding");
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
		println!("Linux screen pipeline: synthetic preview, secure-readiness gates, application audio exclusion/bounded stereo mixing, Stable H.264 encoding and portal pre-cancellation passed. Native screen capture and Discord delivery remain unverified.");
	});
}
#[cfg(not(target_os = "linux"))]
fn main() {
	eprintln!("This debug check requires Linux with GStreamer.");
}
