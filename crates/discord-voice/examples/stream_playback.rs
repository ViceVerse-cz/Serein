//! Offline viewer check: reordered/RTX packets, audio gain/backlog, native decode.
#![cfg(not(test))]
#![allow(dead_code)]
#[path = "../src/stream_playout.rs"]
mod stream_playout;
#[path = "../src/video_receive.rs"]
pub(crate) mod video_receive;
type Frame = [f32; 960];

pub(crate) fn main() {
	for fps in [15, 30, 60] {
		let settings = client_core::screen::Settings {
			source: client_core::screen::SourceId::Display(1),
			width: 854,
			height: 480,
			fps,
			cursor: true,
			audio: false,
		};
		assert!(settings.valid());
		assert_eq!(
			settings.bit_rate(),
			if fps == 60 { 4_000_000 } else { 2_000_000 }
		);
	}
	let (send, receive) = std::sync::mpsc::sync_channel(8);
	let mut audio = stream_playout::Playout::default();
	for n in 1..=8 {
		send.try_send([n as f32 / 10.0; 960]).unwrap();
	}
	assert_eq!(audio.next(&receive, 50, true, false).unwrap(), [0.35; 960]);
	assert_eq!(audio.next(&receive, 200, true, false).unwrap(), [1.0; 960]);
	assert!(audio.next(&receive, 100, true, false).is_none());
	for (volume, enabled, stalled) in [(0, true, false), (100, false, false), (100, true, true)] {
		send.try_send([0.5; 960]).unwrap();
		assert!(audio.next(&receive, volume, enabled, stalled).is_none());
		assert!(audio.next(&receive, 100, true, false).is_none());
	}
	send.try_send([f32::NAN; 960]).unwrap();
	assert_eq!(audio.next(&receive, 100, true, false).unwrap(), [0.0; 960]);

	let mut video = video_receive::Receivers::default();
	video.announce(1, 100).unwrap();
	video.announce_rtx(100, 200).unwrap();
	assert!(
		video
			.push(100, 65534, 9000, false, &[0x7c, 0x85, 1])
			.is_none()
	);
	assert!(video.push(100, 0, 9000, true, &[0x7c, 0x45, 3]).is_none());
	let mut repaired = vec![0xff, 0xff, 0x7c, 0x05, 2];
	assert!(video.restore_rtx(201, &mut repaired).is_none());
	let (ssrc, seq) = video.restore_rtx(200, &mut repaired).unwrap();
	let (user, frame) = video.push(ssrc, seq, 9000, false, &repaired).unwrap();
	assert_eq!((user, frame), (1, vec![0, 0, 0, 1, 0x65, 1, 2, 3]));
	assert!(video.push(100, 65535, 9000, false, &repaired).is_none());
	assert_eq!(video.take_stats().incomplete, 0);
	video.remove(1);
	assert!(video.restore_rtx(200, &mut repaired).is_none());

	// Stream snapshots retire old SSRC/RTX state without resetting an unchanged picture.
	video.announce(1, 100).unwrap();
	video.announce_rtx(100, 200).unwrap();
	video.announce(1, 110).unwrap();
	video.announce_rtx(110, 210).unwrap();
	video.announce(2, 300).unwrap();
	assert!(video.push(110, 1, 90, false, &[0x7c, 0x85, 1]).is_none());
	video.retain_user_sources(1, &[110]);
	assert!(video.restore_rtx(200, &mut repaired).is_none());
	assert_eq!(
		video.keyframe_requests().collect::<Vec<_>>(),
		vec![110, 300]
	);
	assert_eq!(
		video.push(110, 2, 90, true, &[0x7c, 0x45, 2]),
		Some((1, vec![0, 0, 0, 1, 0x65, 1, 2]))
	);
	let mut previous = 110;
	for ssrc in 1000..1100 {
		video.retain_user_sources(1, &[ssrc]);
		video.announce(1, ssrc).unwrap();
		assert!(video.push(previous, 3, 180, true, &[0x65, 3]).is_none());
		assert_eq!(video.keyframe_requests().count(), 2);
		previous = ssrc;
	}
	assert!(video.push(300, 1, 90, true, &[0x65, 4]).is_some());
	video.retain_user_sources(1, &[]);
	assert_eq!(video.keyframe_requests().collect::<Vec<_>>(), vec![300]);

	#[cfg(target_os = "macos")]
	{
		use std::sync::{
			Arc,
			atomic::{AtomicUsize, Ordering},
		};
		let pictures = Arc::new(AtomicUsize::new(0));
		let sink = pictures.clone();
		let mut decoder = platform::video::live::H264Decoder::new(Box::new(move |frame| {
			assert_eq!((frame.width, frame.height), (854, 480));
			sink.fetch_add(1, Ordering::Relaxed);
		}))
		.unwrap();
		let mut encoder = openh264::encoder::Encoder::with_api_config(
			openh264::OpenH264API::from_source(),
			openh264::encoder::EncoderConfig::new(),
		)
		.unwrap();
		let image = openh264::formats::YUVBuffer::new(854, 480);
		for _ in 0..9 {
			let frame = encoder.encode(&image).unwrap().to_vec();
			decoder
				.decode(&frame)
				.expect("VideoToolbox hardware decoding");
		}
		decoder.flush();
		assert_eq!(pictures.load(Ordering::Relaxed), 9);
	}
	println!(
		"PASS: stream volume, mute, bounded backlog, packet reordering/RTX, SSRC replacement and native video decode."
	);
}
