//! Offline debug check for the actual Linux pipeline and portal cancellation.
//! No screen picker, display capture, microphone or network connection is opened.
// This runnable debug example includes implementation modules, not their unit-test harnesses.
#![cfg(not(test))]
#![allow(dead_code)]
#![allow(clippy::duplicate_mod)] // Crate-root shims and screen.rs share production modules.
#[cfg(target_os = "linux")]
#[path = "../src/screen.rs"]
mod screen;
#[cfg(target_os = "linux")]
use screen::{
	AudioChunk, EncodedFrame, MAX_AUDIO_SAMPLES, MAX_ENCODED_BYTES, MAX_RAW_BYTES, RawFrame,
	Settings, SourceId, encode_pixels, encoder, preview_frame,
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
#[path = "../src/video_encode.rs"]
mod video_encode;
#[cfg(target_os = "linux")]
#[path = "../src/video_receive.rs"]
mod video_receive;
// The application-audio worker reports its capture counters through the shared reporter.
#[cfg(target_os = "linux")]
#[path = "../src/diagnostics.rs"]
mod diagnostics;
#[cfg(target_os = "linux")]
#[path = "../src/teardown.rs"]
mod teardown;
#[cfg(target_os = "linux")]
#[path = "../src/timer.rs"]
mod timer;

/// `--nvenc-exit`: quits while a synthetic NVENC share is still encoding, the way closing
/// Serein drops a live screen worker. `--no-wait` exits during the teardown instead of waiting
/// for it: on NVIDIA drivers libnvcuvid's exit handler then crashes or deadlocks the process.
#[cfg(target_os = "linux")]
fn nvenc_exit() {
	use ::gstreamer as gst;
	use gstreamer::{Capture, Mode};
	use std::{
		sync::{
			Arc,
			atomic::{AtomicBool, Ordering},
			mpsc,
		},
		time::Duration,
	};
	gst::init().unwrap();
	let settings = Settings {
		source: SourceId::Display(1),
		width: 1280,
		height: 720,
		fps: 30,
		cursor: true,
		audio: false,
	};
	let stop = Arc::new(AtomicBool::new(false));
	let worker_stop = stop.clone();
	let (encoding, encoded) = mpsc::sync_channel(1);
	let teardown = teardown::Thread::start();
	std::thread::Builder::new()
		.name("screen-encoder".into())
		.spawn(move || {
			let _teardown = teardown;
			let source = gst::ElementFactory::make("videotestsrc")
				.property("is-live", true)
				.build()
				.unwrap();
			let ready = Arc::new(AtomicBool::new(true));
			let Ok(pipeline) = Capture::new(
				settings,
				Mode::Nvidia,
				settings.bit_rate(),
				source,
				worker_stop.clone(),
				ready,
				Arc::new(AtomicBool::new(true)),
				|| true,
			) else {
				return;
			};
			while !worker_stop.load(Ordering::Acquire) && !pipeline.failed() {
				if pipeline
					.frames
					.try_pull_sample(gst::ClockTime::from_mseconds(10))
					.is_some()
				{
					let _ = encoding.try_send(());
				}
			}
		})
		.unwrap();
	if encoded.recv_timeout(Duration::from_secs(10)).is_err() {
		eprintln!(
			"NVENC did not encode a synthetic frame; this check needs an NVIDIA GPU and nvh264enc."
		);
		std::process::exit(2);
	}
	stop.store(true, Ordering::Release);
	if std::env::args().any(|arg| arg == "--no-wait") {
		// Exit inside the encoder's teardown, where an RTX 3070 crashed in 10 of 10 runs.
		std::thread::sleep(Duration::from_millis(5));
	} else {
		assert!(
			teardown::wait_for_media_threads(Duration::from_secs(5)),
			"NVENC teardown did not finish"
		);
	}
	println!("Exiting with the synthetic NVENC share stopped.");
}

#[cfg(target_os = "linux")]
fn main() {
	use ::gstreamer as gst;
	use gst::prelude::*;
	use gstreamer::{Capture, Mode};
	use std::{
		sync::{
			Arc,
			atomic::{AtomicBool, AtomicU64, Ordering},
		},
		time::{Duration, Instant},
	};
	if std::env::args().any(|arg| arg == "--nvenc-exit") {
		return nvenc_exit();
	}
	let mode = if std::env::args().any(|arg| arg == "--legacy-vaapi") {
		Mode::VaLegacy
	} else {
		Mode::Software
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
		let pipeline = Capture::new(settings, mode, settings.bit_rate(), source, stop.clone(), ready.clone(), keyframe.clone(), || true).unwrap();
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
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		while Instant::now() < deadline && encoded < 5 {
			assert!(!pipeline.failed());
			if let Some(sample) = pipeline.frames.try_pull_sample(gst::ClockTime::ZERO) {
				let pts = sample.buffer().unwrap().pts().expect("frame timestamp");
				assert!(last_pts.is_none_or(|last| pts > last), "frames must keep advancing");
				last_pts = Some(pts);
				if mode == Mode::VaLegacy {
					let buffer = sample.buffer().unwrap();
					let data = buffer.map_readable().unwrap();
					video::validate_source(&data).unwrap();
					if encoded == 0 {
						assert!(!buffer.flags().contains(gst::BufferFlags::DELTA_UNIT));
						assert!(video_receive::is_keyframe(&data));
						assert!(video_receive::has_parameter_sets(&data));
					}
					let picture = decoder.decode(&data).unwrap().expect("decodable legacy H.264");
					use openh264::formats::YUVSource;
					assert_eq!(picture.dimensions(), (1280, 720));
					encoded += 1;
					continue;
				}
				let raw = gstreamer::raw(&sample).unwrap();
				assert_eq!((raw.width, raw.height), (1280, 720));
				let mut encoder = encoder(settings, settings.bit_rate()).unwrap();
				let mut yuv = openh264::formats::YUVBuffer::new(1280, 720);
				let (data, keyframe) = encode_pixels(&mut encoder, &mut yuv, &raw.data, (1280, 720), true).unwrap();
				assert!(keyframe);
				video::validate_source(&data).unwrap();
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
		println!("Linux screen pipeline: synthetic preview, secure-readiness gates, application audio exclusion/bounded stereo mixing, {} and portal pre-cancellation passed. Native screen capture and Discord delivery remain unverified.", mode.label());
	});
}
#[cfg(not(target_os = "linux"))]
fn main() {
	eprintln!("This debug check requires Linux with GStreamer.");
}
