//! Outgoing camera RTP. Unofficial Discord video signaling; live interoperability is unverified.
//! Signaling reference: https://github.com/dank074/Discord-video-stream/blob/master/src/client/voice/BaseMediaConnection.ts
//! Uses the same bounded codec packetization as Go Live.
use crate::crypto::Encryption;
use model::voice_settings::{VideoCodec, VideoSettings};
use serde_json::{Value, json};
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::time::Instant;

pub const MAX_FRAME_BYTES: usize = crate::camera::MAX_ENCODED_BYTES;
const MAX_PACKETS: usize = crate::video::MAX_FRAGMENTS;
const MAX_WIRE_BYTES: usize = MAX_PACKETS * 1200;

pub struct Frame {
	pub generation: u64,
	pub timestamp: u32,
	pub codec: VideoCodec,
	pub data: Vec<u8>,
	pub keyframe: bool,
	pub epoch: u64,
	pub keyframe_request: Arc<AtomicBool>,
	pub reset: Arc<AtomicU64>,
	pub reset_generation: u64,
}

pub(crate) struct Sender {
	codec: VideoCodec,
	ssrc: u32,
	rtx: u32,
	sequence: u16,
	pub negotiated: bool,
	pub generation: u64,
	pub announced: bool,
	pacer: crate::video::Pacer,
	dimensions: (u32, u32),
	bitrate: u32,
	frame_rate: u32,
	max_dave_bytes: usize,
	epoch: Option<u64>,
	minimum_epoch: Option<u64>,
	awaiting_keyframe: bool,
	keyframe_request: Option<Arc<AtomicBool>>,
	reset: Option<Arc<AtomicU64>>,
	reset_pending: bool,
	ready: bool,
}
impl Default for Sender {
	fn default() -> Self {
		Self::new(VideoSettings::default())
	}
}
impl Sender {
	pub fn new(settings: VideoSettings) -> Self {
		Self {
			codec: settings.codec,
			ssrc: 0,
			rtx: 0,
			sequence: 0,
			negotiated: false,
			generation: 0,
			announced: false,
			pacer: crate::video::Pacer::new(),
			dimensions: settings.camera_resolution.camera_dimensions(),
			bitrate: crate::camera::bit_rate(
				settings.camera_resolution,
				settings.camera_frame_rate,
			),
			frame_rate: settings.camera_frame_rate.fps(),
			max_dave_bytes: crate::camera::encoded_limit(settings.camera_resolution) + 64 * 1024,
			epoch: None,
			minimum_epoch: None,
			awaiting_keyframe: true,
			keyframe_request: None,
			reset: None,
			reset_pending: false,
			ready: true,
		}
	}
	pub fn configure(&mut self, data: &Value, audio: u32) {
		// Request one stream and accept only that exact assignment, never guessed SSRCs.
		let Some(stream) = data["streams"]
			.as_array()
			.filter(|s| s.len() <= 4)
			.and_then(|s| s.iter().find(|s| s["type"] == "video" && s["rid"] == "100"))
		else {
			return;
		};
		let ssrc = stream["ssrc"]
			.as_u64()
			.and_then(|s| u32::try_from(s).ok())
			.unwrap_or(0);
		let rtx = stream["rtx_ssrc"]
			.as_u64()
			.and_then(|s| u32::try_from(s).ok())
			.unwrap_or(0);
		if ssrc != 0 && rtx != 0 && ssrc != audio && rtx != audio && ssrc != rtx {
			self.ssrc = ssrc;
			self.rtx = rtx;
		}
	}
	pub fn available(&self) -> bool {
		self.negotiated && self.ssrc != 0
	}
	pub fn media_ssrc(&self) -> u32 {
		self.ssrc
	}
	pub fn announcement(&self, audio: u32, enabled: bool) -> Value {
		json!({"op":12,"d":{"audio_ssrc":audio,"video_ssrc":if enabled {self.ssrc} else {0},"rtx_ssrc":if enabled {self.rtx} else {0},"streams":if enabled {vec![json!({"type":"video","rid":"100","ssrc":self.ssrc,"rtx_ssrc":self.rtx,"active":true,"quality":100,"max_bitrate":self.bitrate,"max_framerate":self.frame_rate,"max_resolution":{"type":"fixed","width":self.dimensions.0,"height":self.dimensions.1}})]}else{vec![]}}})
	}
	pub fn clear(&mut self) {
		self.clear_media();
		self.ready = false;
		if let Some(reset) = &self.reset {
			advance_reset(reset);
		} else {
			self.reset_pending = self.generation != 0;
		}
	}

	/// Pause once per security boundary rather than reopening on every audio tick.
	pub fn set_ready(&mut self, ready: bool) {
		if self.ready && !ready {
			self.clear();
		}
		self.ready = ready;
	}

	/// Ordinary receiver feedback coalesces while the next IDR is pending.
	pub fn request_keyframe(&mut self) {
		self.clear_media();
	}

	fn clear_media(&mut self) {
		self.pacer = crate::video::Pacer::new();
		self.awaiting_keyframe = true;
		self.minimum_epoch = self
			.epoch
			.map(|epoch| epoch.saturating_add(1))
			.or_else(|| (self.generation != 0).then_some(1));
		if let Some(request) = &self.keyframe_request {
			request.store(true, Ordering::Release);
		}
	}

	pub fn set_generation(&mut self, generation: u64) {
		if self.generation != generation {
			self.clear();
			self.epoch = None;
			self.minimum_epoch = None;
			self.keyframe_request = None;
			self.reset = None;
			self.reset_pending = false;
			self.ready = true;
			self.generation = generation;
		}
	}

	/// Learn the reset signal even from a picture rejected during negotiation.
	pub fn observe(&mut self, frame: &Frame) -> bool {
		if self
			.reset
			.as_ref()
			.is_some_and(|reset| !Arc::ptr_eq(reset, &frame.reset))
		{
			if self.ready {
				frame.keyframe_request.store(true, Ordering::Release);
			}
			return false;
		}
		self.reset = Some(frame.reset.clone());
		if self.reset_pending {
			advance_reset(&frame.reset);
			self.reset_pending = false;
		}
		true
	}

	/// Do not resume a broken prediction chain with an older queued access unit.
	pub fn accept(&mut self, frame: &Frame) -> bool {
		if !self.observe(frame) {
			return false;
		}
		let reset = frame.reset.load(Ordering::Acquire);
		if reset == u64::MAX || frame.reset_generation != reset {
			frame.keyframe_request.store(true, Ordering::Release);
			return false;
		}
		if self.epoch.is_some_and(|epoch| frame.epoch < epoch)
			|| self.minimum_epoch.is_some_and(|epoch| frame.epoch < epoch)
		{
			frame.keyframe_request.store(true, Ordering::Release);
			return false;
		}
		self.keyframe_request = Some(frame.keyframe_request.clone());
		if self.epoch != Some(frame.epoch) {
			self.pacer = crate::video::Pacer::new();
			self.epoch = Some(frame.epoch);
			self.awaiting_keyframe = true;
		}
		if self.awaiting_keyframe && !frame.keyframe {
			frame.keyframe_request.store(true, Ordering::Release);
			return false;
		}
		self.awaiting_keyframe = false;
		true
	}
	pub fn is_empty(&self) -> bool {
		self.pacer.is_empty()
	}
	pub fn deadline(&self) -> Instant {
		self.pacer.deadline
	}
	pub fn stale(&self, now: Instant) -> bool {
		self.pacer.stale(now)
	}
	pub fn next_batch(
		&mut self,
		now: Instant,
		encryption: &mut Encryption,
	) -> Result<Vec<Vec<u8>>, &'static str> {
		self.pacer
			.next_batch(now, self.bitrate)
			.map(|packet| encryption.seal(&packet.header, &packet.payload))
			.collect()
	}

	pub fn packetize(
		&mut self,
		frame: &[u8],
		timestamp: u32,
		keyframe: bool,
		now: Instant,
	) -> Result<(), &'static str> {
		if frame.len() > self.max_dave_bytes || !self.pacer.is_empty() {
			return Err("Camera frame exceeds the media budget");
		}
		let mut sequence = self.sequence;
		let packets = crate::video::packetize_for_codec(
			frame,
			self.codec,
			&mut sequence,
			timestamp,
			self.ssrc,
			keyframe,
		)?;
		if packets.len() > MAX_PACKETS {
			return Err("Camera packet queue exceeds its budget");
		}
		let mut bytes = 0;
		for packet in &packets {
			// RTP header, XChaCha tag and nonce trailer; each datagram stays <=1200 bytes.
			bytes += packet.header.len() + packet.payload.len() + 20;
			if bytes > MAX_WIRE_BYTES {
				return Err("Camera packet bytes exceed their budget");
			}
		}
		self.sequence = sequence;
		self.pacer.queue(packets, now);
		Ok(())
	}
}

fn advance_reset(reset: &AtomicU64) {
	// MAX stays exhausted: wrapping could make a pre-transition picture current.
	let _ = reset.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
		value.checked_add(1)
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	fn synthetic_frame(
		epoch: u64,
		keyframe: bool,
		request: &Arc<AtomicBool>,
		reset: &Arc<AtomicU64>,
	) -> Frame {
		Frame {
			generation: 1,
			timestamp: 0,
			codec: VideoCodec::H264,
			data: vec![0, 0, 0, 1, if keyframe { 0x65 } else { 0x41 }, 7],
			keyframe,
			epoch,
			keyframe_request: request.clone(),
			reset: reset.clone(),
			reset_generation: reset.load(Ordering::Acquire),
		}
	}

	#[test]
	fn interrupted_camera_requires_a_fresh_epoch_and_keyframe() {
		let request = Arc::new(AtomicBool::new(false));
		let reset = Arc::new(AtomicU64::new(0));
		let mut sender = Sender::default();
		sender.set_generation(1);
		assert!(!sender.accept(&synthetic_frame(0, false, &request, &reset)));
		assert!(request.swap(false, Ordering::AcqRel));
		assert!(sender.accept(&synthetic_frame(0, true, &request, &reset)));
		assert!(sender.accept(&synthetic_frame(0, false, &request, &reset)));
		let old = synthetic_frame(0, true, &request, &reset);
		sender.clear();
		assert!(request.swap(false, Ordering::AcqRel));
		// Even an old queued IDR cannot repair access units lost after it.
		assert!(!sender.accept(&old));
		assert!(!sender.accept(&synthetic_frame(1, false, &request, &reset)));
		assert!(sender.accept(&synthetic_frame(1, true, &request, &reset)));
		assert!(sender.accept(&synthetic_frame(1, false, &request, &reset)));
		assert!(!sender.accept(&synthetic_frame(0, true, &request, &reset)));
		// A new camera worker has its own presentation clock and encoder epochs.
		sender.set_generation(2);
		assert!(sender.accept(&synthetic_frame(
			0,
			true,
			&request,
			&Arc::new(AtomicU64::new(0))
		)));
	}

	#[test]
	fn security_pause_before_first_output_rejects_the_pending_initial_picture() {
		let request = Arc::new(AtomicBool::new(false));
		let reset = Arc::new(AtomicU64::new(0));
		let mut sender = Sender::default();
		// Ordinary camera startup does not require an unnecessary encoder restart.
		sender.set_generation(1);
		assert_eq!(sender.minimum_epoch, None);
		// A prepare/execute transition can occur before lookahead emits any frame.
		sender.clear();
		assert_eq!(sender.minimum_epoch, Some(1));
		assert!(!sender.accept(&synthetic_frame(0, true, &request, &reset)));
		assert_eq!(reset.load(Ordering::Acquire), 1);
		assert!(request.swap(false, Ordering::AcqRel));
		assert!(sender.accept(&synthetic_frame(1, true, &request, &reset)));
		// A newly enabled worker starts its own epoch sequence.
		sender.set_generation(2);
		assert!(sender.accept(&synthetic_frame(
			0,
			true,
			&request,
			&Arc::new(AtomicU64::new(0))
		)));
		sender.set_generation(0);
		sender.clear();
		assert_eq!(sender.minimum_epoch, None);
	}

	#[test]
	fn security_reset_discards_a_keyframe_already_pending_for_receiver_feedback() {
		let request = Arc::new(AtomicBool::new(false));
		let reset = Arc::new(AtomicU64::new(0));
		let mut sender = Sender::default();
		sender.set_generation(1);
		assert!(sender.accept(&synthetic_frame(0, true, &request, &reset)));
		sender.request_keyframe();
		assert!(request.swap(false, Ordering::AcqRel));
		assert_eq!(reset.load(Ordering::Acquire), 0);
		let pending = synthetic_frame(1, true, &request, &reset);
		sender.clear();
		assert_eq!(reset.load(Ordering::Acquire), 1);
		// A new encoder epoch alone is insufficient if its input predates the reset.
		assert!(!sender.accept(&pending));
		assert!(sender.accept(&synthetic_frame(2, true, &request, &reset)));
		sender.set_ready(false);
		sender.set_ready(false);
		assert_eq!(reset.load(Ordering::Acquire), 1);
		sender.set_ready(true);
		sender.set_ready(false);
		assert_eq!(reset.load(Ordering::Acquire), 2);
	}

	#[test]
	fn security_pause_requests_recovery_once_even_without_sending() {
		let request = Arc::new(AtomicBool::new(false));
		let reset = Arc::new(AtomicU64::new(0));
		let mut sender = Sender::default();
		sender.set_generation(1);
		assert!(sender.accept(&synthetic_frame(0, true, &request, &reset)));
		sender.set_ready(false);
		assert!(request.swap(false, Ordering::AcqRel));
		assert_eq!(reset.load(Ordering::Acquire), 1);
		sender.set_ready(false);
		assert!(!request.load(Ordering::Acquire));
		assert_eq!(reset.load(Ordering::Acquire), 1);
	}

	#[test]
	fn exhausted_security_reset_cannot_wrap_into_an_accepted_generation() {
		let request = Arc::new(AtomicBool::new(false));
		let reset = Arc::new(AtomicU64::new(u64::MAX - 1));
		let mut sender = Sender::default();
		sender.set_generation(1);
		assert!(sender.accept(&synthetic_frame(0, true, &request, &reset)));
		sender.clear();
		sender.clear();
		assert_eq!(reset.load(Ordering::Acquire), u64::MAX);
		assert!(!sender.accept(&synthetic_frame(1, true, &request, &reset)));
	}

	#[test]
	fn camera_rtp_keeps_decode_order_and_original_presentation_timestamps() {
		let mut sender = Sender::default();
		let mut crypto = Encryption::new(&[7; 32]);
		let now = Instant::now();
		let mut sequence: Option<u16> = None;
		for timestamp in [3000_u32, 12000, 6000, 9000] {
			sender
				.packetize(&[0, 0, 0, 1, 0x41, 7], timestamp, false, now)
				.unwrap();
			let packets = sender.next_batch(now, &mut crypto).unwrap();
			assert_eq!(packets.len(), 1);
			let header = &packets[0];
			assert_eq!(&header[4..8], &timestamp.to_be_bytes());
			let actual = u16::from_be_bytes([header[2], header[3]]);
			if let Some(previous) = sequence {
				assert_eq!(actual, previous.wrapping_add(1));
			}
			sequence = Some(actual);
		}
	}

	#[test]
	fn av1_camera_does_not_mark_delta_frames_as_new_sequences() {
		let mut sender = Sender::new(VideoSettings {
			backend: model::voice_settings::VideoBackend::Experimental,
			codec: VideoCodec::Av1,
			..VideoSettings::default()
		});
		let now = Instant::now();
		sender
			.packetize(&[0x0a, 1, 7, 0x30, 8, 9], 0, false, now)
			.unwrap();
		assert!(
			sender
				.pacer
				.next_batch(now, sender.bitrate)
				.all(|packet| packet.payload[0] & 8 == 0)
		);
	}
	#[test]
	fn camera_packetization_is_bounded_and_marks_only_the_last_fragment() {
		let mut sender = Sender::default();
		let mut crypto = Encryption::new(&[7; 32]);
		let mut frame = vec![0, 0, 0, 1, 0x65];
		frame.extend(vec![9; 2400]);
		let now = Instant::now();
		sender.packetize(&frame, 6000, true, now).unwrap();
		let packets = sender.next_batch(now, &mut crypto).unwrap();
		assert_eq!(packets.len(), 3);
		for (i, packet) in packets.iter().enumerate() {
			assert!(packet.len() <= 1200);
			assert_eq!(packet[1] & 0x80 != 0, i == 2);
			assert_eq!(&packet[4..8], &6000u32.to_be_bytes());
		}
		sender.clear();
		let over_budget = sender.max_dave_bytes + 1;
		assert!(
			sender
				.packetize(&vec![0; over_budget], 0, true, now)
				.is_err()
		);
		assert!(sender.packetize(&[0, 0, 1], 0, true, now).is_err());
	}
	#[test]
	fn failed_packet_budget_does_not_queue_a_partial_camera_frame() {
		let mut sender = Sender::default();
		let mut frame = Vec::new();
		for _ in 0..MAX_PACKETS + 1 {
			frame.extend([0, 0, 0, 1, 0x65, 7]);
		}
		assert!(sender.packetize(&frame, 0, true, Instant::now()).is_err());
		assert!(sender.is_empty());
		assert_eq!(sender.sequence, 0);
	}
	#[test]
	fn camera_packetizer_enforces_the_selected_frame_budget() {
		use model::voice_settings::VideoResolution;
		for resolution in [
			VideoResolution::P480,
			VideoResolution::P720,
			VideoResolution::P4320,
		] {
			let mut sender = Sender::new(VideoSettings {
				camera_resolution: resolution,
				..VideoSettings::default()
			});
			let limit = crate::camera::encoded_limit(resolution) + 64 * 1024;
			let mut frame = vec![0, 0, 0, 1, 0x65];
			frame.resize(limit + 1, 9);
			assert_eq!(
				sender.packetize(&frame, 0, true, Instant::now()),
				Err("Camera frame exceeds the media budget")
			);
			assert!(sender.is_empty());
			frame.pop();
			assert!(sender.packetize(&frame, 0, true, Instant::now()).is_ok());
		}
	}
	#[test]
	fn camera_announces_the_selected_resolution_and_rate() {
		use model::voice_settings::{VideoFrameRate, VideoResolution};
		for resolution in VideoResolution::ALL {
			for frame_rate in VideoFrameRate::ALL {
				let sender = Sender::new(VideoSettings {
					camera_resolution: resolution,
					camera_frame_rate: frame_rate,
					..VideoSettings::default()
				});
				let announcement = sender.announcement(1, true);
				let stream = &announcement["d"]["streams"][0];
				let (width, height) = resolution.camera_dimensions();
				assert_eq!(stream["max_resolution"]["width"], width);
				assert_eq!(stream["max_resolution"]["height"], height);
				assert_eq!(
					stream["max_bitrate"],
					crate::camera::bit_rate(resolution, frame_rate)
				);
				assert_eq!(stream["max_framerate"], frame_rate.fps());
			}
		}
	}

	#[test]
	fn high_resolution_camera_is_paced_above_the_old_fixed_packet_rate() {
		use model::voice_settings::VideoResolution;
		let mut sender = Sender::new(VideoSettings {
			camera_resolution: VideoResolution::P4320,
			..VideoSettings::default()
		});
		let now = Instant::now();
		let mut crypto = Encryption::new(&[7; 32]);
		let mut frame = vec![0, 0, 0, 1, 0x65];
		frame.extend(vec![9; 300_000]);
		sender.packetize(&frame, 6000, true, now).unwrap();
		assert!(sender.next_batch(now, &mut crypto).unwrap().len() <= 4);
		let packets = sender
			.next_batch(now + std::time::Duration::from_millis(2), &mut crypto)
			.unwrap();
		assert!(packets.len() > 1);
		assert!(packets.iter().all(|packet| packet.len() <= 1200));
		sender.clear();
		assert!(sender.is_empty());
		assert!(
			sender
				.next_batch(now + std::time::Duration::from_secs(1), &mut crypto)
				.unwrap()
				.is_empty()
		);
	}
}
