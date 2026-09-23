//! Offline check of SPS normalization before DAVE; no capture, devices or network.
#![cfg(not(test))]
#![allow(dead_code)]

#[path = "../src/crypto.rs"]
mod crypto;
#[path = "support/stream_feedback.rs"]
mod stream_feedback;
mod stream_playback;
#[path = "../src/test_mls.rs"]
mod test_mls;
#[path = "../src/video.rs"]
mod video;
#[path = "../src/video_sps.rs"]
mod video_sps;
type Frame = [f32; 960];

fn main() {
	// Synthetic Baseline SPS: 320x240, one reference picture, POC type 2.
	// Expected VUI: motion vectors allowed, denominators 2/1, MV lengths 16/16,
	// zero reordered pictures and one buffered picture (H.264 Annex E).
	let expected = [
		0, 0, 0, 1, 0x67, 0x42, 0, 0x1f, 0xda, 0x05, 0x07, 0xe8, 0x06, 0xd0, 0x44, 0x23, 0x50,
	];
	for tail in [
		&[0xe4][..],
		&[0xe8, 0x02],
		&[0xe8, 0x06, 0xd0, 0x44, 0x22, 0x42, 0xc0],
	] {
		let frame = [
			&[0, 0, 0, 1, 0x67, 0x42, 0, 0x1f, 0xda, 0x05, 0x07][..],
			tail,
		]
		.concat();
		assert_eq!(video_sps::normalize(&frame).unwrap().as_ref(), expected);
	}
	assert_eq!(video_sps::normalize(&expected).unwrap().as_ref(), expected);
	let server = test_mls::Delivery::new();
	let mut alice =
		crypto::Dave::with_identity(1, Some(2), 3, crypto::Identity::generate()).unwrap();
	let mut bob = crypto::Dave::with_identity(2, Some(1), 3, crypto::Identity::generate()).unwrap();
	alice.session.set_external_sender(&server.external).unwrap();
	bob.session.set_external_sender(&server.external).unwrap();
	let (commit, welcome) = server.add(&mut alice, &bob.key_package().unwrap());
	alice
		.group_changed(29, &[&[0, 0], commit.as_slice()].concat())
		.unwrap();
	bob.group_changed(30, &[&[0, 0], welcome.as_slice()].concat())
		.unwrap();
	let mut encoder = openh264::encoder::Encoder::with_api_config(
		openh264::OpenH264API::from_source(),
		openh264::encoder::EncoderConfig::new(),
	)
	.unwrap();
	let source = openh264::formats::YUVBuffer::new(320, 240);
	let mut frame = Vec::new();
	encoder.encode(&source).unwrap().write_vec(&mut frame);
	let normalized = video_sps::normalize(&frame).unwrap();
	assert_eq!(video_sps::normalize(&normalized).unwrap(), normalized);

	// Rewriting authenticated SPS after encryption reproduces the receiver failure.
	let unnormalized = [
		0, 0, 0, 1, 0x67, 0x42, 0, 0x1f, 0xda, 0x05, 0x07, 0xe4, 0, 0, 0, 1, 0x65, 0xb8, 0x04,
		0x17, 0xff, 0xff,
	];
	let old = alice
		.session
		.encrypt(davey::MediaType::VIDEO, davey::Codec::H264, &unnormalized)
		.unwrap();
	let rewritten = video_sps::normalize(&old).unwrap();
	assert!(
		bob.session
			.decrypt(1, davey::MediaType::VIDEO, &rewritten)
			.is_err()
	);
	let corrected = video_sps::normalize(&unnormalized).unwrap();
	let encrypted = alice
		.session
		.encrypt(davey::MediaType::VIDEO, davey::Codec::H264, &corrected)
		.unwrap();
	let received = video_sps::normalize(&encrypted).unwrap();
	assert_eq!(received.as_ref(), encrypted.as_ref());
	assert_eq!(
		bob.session
			.decrypt(1, davey::MediaType::VIDEO, &received)
			.unwrap()
			.as_slice(),
		corrected.as_ref()
	);
	// Normalizing first leaves receiver-visible metadata stable and authenticated.
	let encrypted = alice
		.session
		.encrypt(davey::MediaType::VIDEO, davey::Codec::H264, &normalized)
		.unwrap();
	assert_eq!(
		video_sps::normalize(&encrypted).unwrap().as_ref(),
		encrypted.as_ref()
	);
	let decrypted = bob
		.session
		.decrypt(1, davey::MediaType::VIDEO, &encrypted)
		.unwrap();
	assert_eq!(decrypted.as_slice(), normalized.as_ref());
	let mut decoder = openh264::decoder::Decoder::new().unwrap();
	assert!(decoder.decode(&decrypted).unwrap().is_some());
	// Exercise paced RTP, transport authentication and reassembly.
	// A tiny payload would hide a whole-frame burst; use a bounded, noisy keyframe too.
	let mut noisy = vec![0, 0, 0, 1, 0x65, 0xb8];
	noisy.resize(120_000, 7);
	// Windows' default 15.6 ms tick must cost latency, never throughput.
	for tick in [1, 2, 16] {
		let tick = std::time::Duration::from_millis(tick);
		let mut sequence = u16::MAX - 2;
		let mut pacer = video::Pacer::new();
		let now = tokio::time::Instant::now();
		let packets = video::packetize(&noisy, &mut sequence, 90_000, 42).unwrap();
		let total = packets.len();
		pacer.queue(packets, now);
		let mut sent = 0;
		let mut receiver = stream_playback::video_receive::Receivers::default();
		receiver.announce(1, 42).unwrap();
		let mut transport = crypto::Encryption::new(&[9; 32]);
		let mut restored = None;
		let mut due = now;
		while !pacer.is_empty() {
			assert!(due < now + std::time::Duration::from_millis(60));
			let batch: Vec<_> = pacer.next_batch(due, 16_000_000).collect();
			assert!(batch.len() < total);
			for packet in batch {
				sent += 1;
				let wire = transport.seal(&packet.header, &packet.payload).unwrap();
				assert!(wire.len() <= 1200);
				let rtp = transport.open(&wire).unwrap();
				if let Some((_, frame)) = receiver.push(
					rtp.ssrc,
					rtp.sequence,
					rtp.timestamp,
					rtp.marker,
					&rtp.payload,
				) {
					restored = Some(frame);
				}
			}
			let wait = pacer.deadline.duration_since(now).as_micros();
			due = now + tick * wait.div_ceil(tick.as_micros()).max(1) as u32;
		}
		assert_eq!(sent, total);
		assert_eq!(restored.unwrap(), noisy);
		pacer.queue(
			video::packetize(&encrypted, &mut sequence, 93_000, 42).unwrap(),
			now,
		);
		pacer.clear(); // Rekey/cancellation must discard every pending encrypted fragment.
		assert!(pacer.is_empty());
		assert_eq!(pacer.next_batch(pacer.deadline, 16_000_000).count(), 0);
	}
	let encrypted = alice
		.session
		.encrypt(davey::MediaType::VIDEO, davey::Codec::H264, &normalized)
		.unwrap();
	let mut receiver = stream_playback::video_receive::Receivers::default();
	receiver.announce(1, 42).unwrap();
	let mut transport = crypto::Encryption::new(&[8; 32]);
	let mut sequence = 0;
	let mut received = None;
	for packet in video::packetize(&encrypted, &mut sequence, 90_000, 42).unwrap() {
		let wire = transport.seal(&packet.header, &packet.payload).unwrap();
		let rtp = transport.open(&wire).unwrap();
		if let Some((user, frame)) = receiver.push(
			rtp.ssrc,
			rtp.sequence,
			rtp.timestamp,
			rtp.marker,
			&rtp.payload,
		) {
			received = Some(
				bob.session
					.decrypt(user, davey::MediaType::VIDEO, &frame)
					.unwrap(),
			);
		}
	}
	assert!(decoder.decode(&received.unwrap()).unwrap().is_some());
	stream_playback::main();
	stream_feedback::run();
	for invalid in [&[][..], &[0, 0, 1, 0x67], &[0, 0, 1, 0x67, 0xff]] {
		assert!(video_sps::normalize(invalid).is_err());
	}
	assert!(video_sps::normalize(&vec![0; 2 * 1024 * 1024 + 1]).is_err());
	println!(
		"PASS: SPS authentication, paced bounded RTP, cancellation, DAVE round trip and H264 decode."
	);
}
