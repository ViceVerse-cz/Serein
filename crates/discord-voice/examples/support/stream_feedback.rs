//! Synthetic authenticated feedback, bounded repair and rate-control checks; no sockets/devices.
use crate::{crypto::Encryption, stream_playback::video_receive::Receivers, video};
use std::time::Duration;
use tokio::time::Instant;

fn rtcp(kind: u8, count: u8, body: &[u8]) -> Vec<u8> {
	let mut clear = vec![0x80 | count, kind];
	clear.extend_from_slice(&((body.len() / 4) as u16).to_be_bytes());
	clear.extend_from_slice(body);
	clear
}

fn packet(sequence: u16, capacity: usize) -> video::Packet {
	let mut header = [0x80, 0xe5, 0, 0, 0, 1, 0x5f, 0x90, 0, 0, 0, 42];
	header[2..4].copy_from_slice(&sequence.to_be_bytes());
	let mut payload = Vec::with_capacity(capacity.max(2));
	payload.extend([0x65, 1]);
	video::Packet { header, payload }
}

pub(super) fn run() {
	let now = Instant::now();
	let mut crypto = Encryption::new(&[7; 32]);
	let mut report = vec![0; 28];
	report[4..8].copy_from_slice(&42u32.to_be_bytes());
	report[8] = 32; // 12.5% receiver-reported loss.
	let mut clear = rtcp(201, 1, &report);
	clear.extend(rtcp(205, 1, &[0, 0, 0, 9, 0, 0, 0, 42, 255, 255, 0, 3]));
	// REMB: 125000 * 2^4 = 2 Mbit/s, scoped to our media SSRC.
	clear.extend(rtcp(
		206,
		15,
		&[
			0, 0, 0, 9, 0, 0, 0, 0, b'R', b'E', b'M', b'B', 1, 17, 232, 72, 0, 0, 0, 42,
		],
	));
	let wire = crypto
		.seal_rtcp(clear[..8].try_into().unwrap(), &clear[8..])
		.unwrap();
	let feedback = crypto.feedback(&wire, 42).unwrap();
	assert_eq!(feedback.nacks, [u16::MAX, 0, 1]);
	assert_eq!(
		(feedback.loss, feedback.bitrate),
		(Some(32), Some(2_000_000))
	);
	let unrelated = crypto.feedback(&wire, 43).unwrap();
	assert!(unrelated.nacks.is_empty() && unrelated.loss.is_none() && unrelated.bitrate.is_none());
	let mut corrupt = wire.clone();
	corrupt[8] ^= 1;
	assert!(crypto.feedback(&corrupt, 42).is_none());
	assert!(Encryption::new(&[8; 32]).feedback(&wire, 42).is_none());
	clear.extend(rtcp(201, 1, &[0; 4]));
	let malformed = crypto
		.seal_rtcp(clear[..8].try_into().unwrap(), &clear[8..])
		.unwrap();
	assert!(crypto.feedback(&malformed, 42).is_none());

	let mut rate = video::Rate::new(4_000_000, now);
	rate.observe(feedback.loss, feedback.bitrate);
	assert_eq!(rate.tick(now + Duration::from_secs(1)), Some(1_280_000));
	assert_eq!(rate.tick(now + Duration::from_secs(2)), None); // No feedback, no growth.
	rate.observe(Some(0), None);
	assert_eq!(rate.tick(now + Duration::from_secs(3)), Some(1_344_000));
	for second in 4..40 {
		rate.observe(Some(0), Some(2_000_000));
		rate.tick(now + Duration::from_secs(second));
		assert!(rate.target <= 2_000_000);
	}
	assert_eq!(rate.target, 1_600_000); // REMB reserves 25% wire headroom above encoder target.
	for second in 40..80 {
		rate.observe(Some(0), None);
		rate.tick(now + Duration::from_secs(second));
	}
	assert_eq!(rate.target, 4_000_000); // Expired REMB permits recovery to the preset.
	for second in 80..110 {
		rate.observe(Some(255), None);
		rate.tick(now + Duration::from_secs(second));
	}
	assert_eq!(rate.target, 250_000);
	rate.observe(None, Some(1));
	rate.tick(now + Duration::from_secs(110));
	assert_eq!(rate.target, 250_000);
	let mut low_preset = video::Rate::new(100_000, now);
	low_preset.observe(Some(255), Some(0));
	low_preset.tick(now + Duration::from_secs(1));
	assert_eq!(low_preset.target, 100_000);

	let mut frame = vec![0, 0, 0, 1, 0x65];
	frame.resize(2500, 7);
	let mut sequence = u16::MAX;
	let packets = video::packetize(&frame, &mut sequence, 90_000, 42).unwrap();
	assert_eq!(packets.len(), 3);
	let mut receiver = Receivers::default();
	receiver.announce(1, 42).unwrap();
	receiver.announce_rtx(42, 43).unwrap();
	let mut history = video::History::default();
	for packet in packets {
		let original = crypto.seal(&packet.header, &packet.payload).unwrap();
		let rtp = crypto.open(&original).unwrap();
		if rtp.sequence != 0 {
			// Lose the middle fragment, retaining the later marker.
			assert!(
				receiver
					.push(42, rtp.sequence, rtp.timestamp, rtp.marker, &rtp.payload)
					.is_none()
			);
		}
		history.remember(packet, now);
	}
	assert!(!history.request(&feedback.nacks, now));
	assert!(!history.request(&feedback.nacks, now)); // Pending repair is deduplicated.
	let mut rtx_sequence = u16::MAX;
	let mut repaired = None;
	for (index, original) in feedback.nacks.iter().enumerate() {
		let packet = history.repair(43, &mut rtx_sequence, now).unwrap();
		let wire = crypto.seal(&packet.header, &packet.payload).unwrap();
		assert!(wire.len() <= 1200);
		let mut rtp = crypto.open(&wire).unwrap();
		assert_eq!(
			(rtp.payload_type, rtp.ssrc, rtp.timestamp),
			(102, 43, 90_000)
		);
		assert_eq!(rtp.sequence, u16::MAX.wrapping_add(index as u16));
		assert_eq!(rtp.marker, *original == 1);
		let (ssrc, sequence) = receiver.restore_rtx(rtp.ssrc, &mut rtp.payload).unwrap();
		assert_eq!((ssrc, sequence), (42, *original));
		if let Some((_, frame)) =
			receiver.push(ssrc, sequence, rtp.timestamp, rtp.marker, &rtp.payload)
		{
			repaired = Some(frame);
		}
	}
	assert_eq!(repaired.unwrap(), frame);
	assert!(!history.has_pending());
	history.request(&[0], now + Duration::from_millis(49));
	assert!(!history.has_pending());
	history.request(&[0], now + Duration::from_millis(50));
	assert!(
		history
			.repair(43, &mut rtx_sequence, now + Duration::from_millis(50))
			.is_some()
	);
	assert!(history.request(&[0], now + Duration::from_millis(100))); // Exhausted retries need IDR.
	assert!(!history.has_pending());
	assert!(history.request(&[1], now + Duration::from_secs(1))); // Expired packets need IDR.
	history.remember(packet(8, 2), now);
	history.request(&[8], now);
	history.clear();
	assert!(!history.has_pending() && history.repair(43, &mut rtx_sequence, now).is_none());
	assert!(history.request(&[8], now));

	for sequence in 0..2050 {
		history.remember(packet(sequence, 2), now);
	}
	assert!(history.request(&[0], now)); // Item budget evicts oldest packets.
	assert!(!history.request(&[2049], now));
	history.clear();
	for sequence in 0..5 {
		history.remember(packet(sequence, 512 * 1024), now);
	}
	assert!(history.request(&[0], now)); // Capacity, not just length, counts toward bytes.
	assert!(!history.request(&[4], now));
	history.clear();
	for sequence in 0..200 {
		history.remember(packet(sequence, 2), now);
	}
	history.request(&(0..200).collect::<Vec<_>>(), now);
	let mut repairs = 0;
	while history.repair(43, &mut rtx_sequence, now).is_some() {
		repairs += 1;
	}
	assert_eq!(repairs, 128);
	history.clear();
	history.remember(packet(1, 2), now);
	history.request(&[1], now);
	history.remember(packet(2, 2), now + Duration::from_millis(900));
	history.request(&[2], now + Duration::from_millis(900));
	let fresh = history
		.repair(43, &mut rtx_sequence, now + Duration::from_millis(1100))
		.unwrap();
	assert_eq!(&fresh.payload[..2], &2u16.to_be_bytes()); // Expired head cannot block fresh repairs.

	let mut pacer = video::Pacer::new();
	let now = Instant::now();
	frame.resize(120_000, 7);
	pacer.queue(
		video::packetize(&frame, &mut sequence, 93_000, 42).unwrap(),
		now,
	);
	let (mut sent, mut repairs) = (0, 0);
	for tick in 0..=500 {
		let due = now + Duration::from_millis(tick * 2);
		if pacer.allow_repair(due, 250_000) {
			repairs += 1;
		}
		for packet in pacer.next_batch(due, 250_000) {
			sent += packet.payload.len() + 32;
		}
	}
	assert!(sent > 60_000 && sent + repairs * 1200 <= 84_000); // 2.5x of 250 kbps.
	assert!((4..=5).contains(&repairs)); // Repairs cannot consume the whole wire budget.
	assert!(!pacer.stale(now + Duration::from_secs(1))); // Slow progress must not loop on IDRs.
	assert!(pacer.stale(now + Duration::from_millis(1500)));
	pacer.clear();
	assert!(pacer.is_empty() && !pacer.stale(now + Duration::from_secs(1)));
	println!(
		"PASS: authenticated RR/REMB/NACK, adaptive rate, bounded RTX recovery and wire pacing."
	);
}
