//! Offline check of SPS normalization before DAVE; no capture, devices or network.
#![cfg(not(test))]
#![allow(dead_code)]

#[path = "../src/crypto.rs"]
mod crypto;
#[path = "../src/test_mls.rs"]
mod test_mls;
#[path = "../src/video_sps.rs"]
mod video_sps;

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
	for invalid in [&[][..], &[0, 0, 1, 0x67], &[0, 0, 1, 0x67, 0xff]] {
		assert!(video_sps::normalize(invalid).is_err());
	}
	assert!(video_sps::normalize(&vec![0; 2 * 1024 * 1024 + 1]).is_err());
	println!(
		"SPS rewrite authentication failure reproduced; normalized DAVE video decrypts and decodes."
	);
}
