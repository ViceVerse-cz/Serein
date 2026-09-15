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
		let settings = Settings { source: SourceId::Portal, width: 1280, height: 720, fps: 30, cursor: true, audio: false };
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
		let (send, mut receive) = tokio::sync::mpsc::channel(4);
		let epoch = Arc::new(AtomicU64::new(0));
		let source = gst::ElementFactory::make("audiotestsrc").property("is-live", true).property("samplesperbuffer", 480i32).build().unwrap();
		let audio = audio_linux::Audio::new(source, stop.clone(), ready.clone(), epoch.clone(), false).unwrap();
		tokio::time::sleep(Duration::from_millis(50)).await;
		audio.pump(&send, &ready, &stop).unwrap();
		assert!(receive.try_recv().is_err(), "no audio before secure readiness");
		ready.store(true, Ordering::Release);
		let deadline = Instant::now() + Duration::from_secs(3);
		let mut heard = false;
		while Instant::now() < deadline && !heard {
			let _ = tokio::time::timeout(Duration::from_millis(100), audio.changed.notified()).await;
			audio.pump(&send, &ready, &stop).unwrap();
			if let Ok(chunk) = receive.try_recv() {
				let samples = chunk.samples;
				assert_eq!(chunk.epoch, 0);
				assert_eq!(samples.len(), 960);
				assert!(samples.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
				assert!(samples.iter().any(|v| v.abs() > 0.1));
				heard = true;
			}
		}
		assert!(heard, "synthetic stereo audio did not arrive");
		tokio::time::sleep(Duration::from_millis(100)).await;
		audio.pump(&send, &ready, &stop).unwrap();
		assert!(receive.len() <= 4, "slow consumers must not grow the audio queue");
		while receive.try_recv().is_ok() {}
		tokio::time::sleep(Duration::from_millis(50)).await;
		ready.store(false, Ordering::Release);
		epoch.fetch_add(1, Ordering::AcqRel);
		ready.store(true, Ordering::Release);
		audio.pump(&send, &ready, &stop).unwrap();
		assert_eq!(receive.try_recv().unwrap().epoch, 0, "queued samples retain their old generation so transport rejects them");
		ready.store(false, Ordering::Release);
		while receive.try_recv().is_ok() {}
		audio.pump(&send, &ready, &stop).unwrap();
		assert!(receive.try_recv().is_err(), "queued audio must be discarded during a rekey");
		stop.store(true, Ordering::Release);
		drop(audio);
		drop(pipeline);
		let stop = Arc::new(AtomicBool::new(false));
		ready.store(true, Ordering::Release);
		let source = gst::ElementFactory::make("audiotestsrc").property("is-live", true).property("samplesperbuffer", 4801i32).build().unwrap();
		let oversized = audio_linux::Audio::new(source, stop.clone(), ready.clone(), epoch.clone(), false).unwrap();
		let deadline = Instant::now() + Duration::from_secs(3);
		let mut rejected = false;
		while Instant::now() < deadline && !rejected {
			tokio::time::sleep(Duration::from_millis(10)).await;
			rejected = oversized.pump(&send, &ready, &stop).is_err();
		}
		assert!(rejected, "oversized native audio must fail before admission to the sink queue");
		drop(oversized);
		stop.store(true, Ordering::Release);
		let mut cancelled = audio_linux::Worker::start(send, stop.clone(), ready.clone(), epoch).unwrap();
		let deadline = Instant::now() + Duration::from_secs(3);
		loop {
			if let Some(result) = cancelled.result() { result.unwrap(); break; }
			assert!(Instant::now() < deadline, "cancelled audio worker must retire without opening a device");
			tokio::time::sleep(Duration::from_millis(10)).await;
		}
		println!("Linux screen pipeline: synthetic preview, secure-readiness gates, stereo audio, software H.264 and portal pre-cancellation passed. Native Linux capture/GPU encoding remains unverified.");
	});
}
#[cfg(not(unix))]
fn main() {
	eprintln!("This debug check requires Linux or macOS with GStreamer.");
}
