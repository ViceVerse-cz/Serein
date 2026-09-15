//! Incoming H.264 video: RFC 6184 depacketizing per remote SSRC and software decoding on a
//! dedicated thread. Frames stay bounded (1080p RGBA) and no video is ever written to disk.
//! Discord's video signaling is unofficial; live interoperability is unverified.
use std::{
	collections::HashMap,
	sync::{
		Arc, Mutex,
		mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
	},
};

/// Users whose decoder hit undecodable data; the transport turns these into keyframe requests.
pub(crate) type Lost = Arc<Mutex<Vec<u64>>>;

/// Largest reassembled (still DAVE-encrypted) access unit accepted from one remote sender.
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024 + 64 * 1024;
/// Remote video senders tracked per transport; Discord forwards at most a few at once.
pub const MAX_SOURCES: usize = 16;
/// Decoders kept alive at once; each holds reference pictures for one remote user.
const MAX_DECODERS: usize = 8;
const QUEUE_BYTES: usize = 16 * 1024 * 1024;
const MAX_WIDTH: u32 = 1920;
const MAX_PIXELS: u64 = 1920 * 1080;
const START_CODE: [u8; 4] = [0, 0, 0, 1];

/// One decoded remote picture, packed RGBA.
pub struct RemoteFrame<'a> {
	pub user: u64,
	pub width: u32,
	pub height: u32,
	pub rgba: &'a [u8],
}
pub type VideoSink = Arc<dyn Fn(RemoteFrame<'_>) + Send + Sync>;

pub(crate) struct DecoderQueue {
	send: SyncSender<(Encoded, tokio::sync::OwnedSemaphorePermit)>,
	bytes: Arc<tokio::sync::Semaphore>,
}

/// A cleartext Annex-B access unit handed to the decoder thread.
pub(crate) struct Encoded {
	pub user: u64,
	pub data: Vec<u8>,
	pub keyframe: bool,
}

/// True when the cleartext Annex-B access unit carries an IDR slice.
pub(crate) fn is_keyframe(frame: &[u8]) -> bool {
	let mut at = 0;
	while at + 4 <= frame.len() {
		let size = if frame[at..].starts_with(&[0, 0, 0, 1]) {
			4
		} else if frame[at..].starts_with(&[0, 0, 1]) {
			3
		} else {
			at += 1;
			continue;
		};
		if frame.get(at + size).is_some_and(|nal| nal & 0x1f == 5) {
			return true;
		}
		at += size;
	}
	false
}

/// Reassembles RTP payloads of one SSRC into Annex-B access units (single NAL, STAP-A, FU-A).
#[derive(Default)]
pub(crate) struct Assembler {
	timestamp: u32,
	next_sequence: Option<u16>,
	frame: Vec<u8>,
	fragmenting: bool,
	broken: bool,
	started: bool,
	// Preserve loss across frame resets until Receivers schedules recovery.
	lost: bool,
}
impl Assembler {
	/// Feed one packet; returns a complete access unit when the marker closes an intact frame.
	pub fn push(
		&mut self,
		sequence: u16,
		timestamp: u32,
		marker: bool,
		payload: &[u8],
	) -> Option<Vec<u8>> {
		if self.started && timestamp != self.timestamp {
			// A new picture started before the previous marker arrived: drop the partial one.
			self.lost = true;
			self.reset_frame();
			self.timestamp = timestamp;
		}
		if !self.started {
			self.started = true;
			self.timestamp = timestamp;
		}
		if self
			.next_sequence
			.is_some_and(|expected| expected != sequence)
		{
			self.broken = true;
		}
		self.next_sequence = Some(sequence.wrapping_add(1));
		if !self.broken && self.append(payload).is_err() {
			self.broken = true;
		}
		self.lost |= self.broken;
		if !marker {
			return None;
		}
		let complete = (!self.broken && !self.fragmenting && !self.frame.is_empty())
			.then(|| std::mem::take(&mut self.frame));
		self.lost |= complete.is_none();
		self.reset_frame();
		self.started = false;
		complete
	}
	fn reset_frame(&mut self) {
		self.frame.clear();
		if self.frame.capacity() > 256 * 1024 {
			self.frame = Vec::new();
		}
		self.fragmenting = false;
		self.broken = false;
	}
	fn extend(&mut self, parts: &[&[u8]]) -> Result<(), ()> {
		let total: usize = parts.iter().map(|part| part.len()).sum();
		if self.frame.len() + total > MAX_FRAME_BYTES {
			return Err(());
		}
		if self.frame.len() + total > self.frame.capacity()
			&& self.frame.capacity().saturating_mul(2) > MAX_FRAME_BYTES
		{
			self.frame.reserve_exact(MAX_FRAME_BYTES - self.frame.len());
		}
		for part in parts {
			self.frame.extend_from_slice(part);
		}
		Ok(())
	}
	fn append(&mut self, payload: &[u8]) -> Result<(), ()> {
		let Some(&indicator) = payload.first() else {
			return Err(());
		};
		match indicator & 0x1f {
			1..=23 => {
				if self.fragmenting {
					return Err(());
				}
				self.extend(&[&START_CODE, payload])
			}
			24 => {
				if self.fragmenting {
					return Err(());
				}
				let mut at = 1;
				while at < payload.len() {
					if at + 2 > payload.len() {
						return Err(());
					}
					let size = usize::from(u16::from_be_bytes([payload[at], payload[at + 1]]));
					at += 2;
					if size == 0 || at + size > payload.len() {
						return Err(());
					}
					self.extend(&[&START_CODE, &payload[at..at + size]])?;
					at += size;
				}
				Ok(())
			}
			28 => {
				if payload.len() < 2 {
					return Err(());
				}
				let header = payload[1];
				let start = header & 0x80 != 0;
				let end = header & 0x40 != 0;
				if start == self.fragmenting {
					return Err(());
				}
				if start {
					let nal_header = (indicator & 0xe0) | (header & 0x1f);
					self.extend(&[&START_CODE, &[nal_header], &payload[2..]])?;
				} else {
					self.extend(&[&payload[2..]])?;
				}
				self.fragmenting = !end;
				Ok(())
			}
			_ => Err(()),
		}
	}
}

/// Remote video SSRC ownership and per-source reassembly for one media transport.
#[derive(Default)]
pub(crate) struct Receivers {
	sources: Vec<(u32, u64, Assembler)>,
	/// Users whose decoder lost a reference picture; only a keyframe restarts their video.
	awaiting_keyframe: Vec<u64>,
}
impl Receivers {
	/// Bind an announced video SSRC to a user; zero clears that user's sources.
	pub fn announce(&mut self, user: u64, ssrc: u32) -> Result<(), &'static str> {
		if ssrc == 0 {
			self.remove(user);
			return Ok(());
		}
		if let Some(entry) = self.sources.iter_mut().find(|(s, _, _)| *s == ssrc) {
			if entry.1 != user {
				entry.1 = user;
				entry.2 = Assembler::default();
			}
			return Ok(());
		}
		if self.sources.len() >= MAX_SOURCES {
			return Err("Voice channel announces more video sources than supported");
		}
		self.sources.push((ssrc, user, Assembler::default()));
		self.require_keyframe(user);
		Ok(())
	}
	pub fn remove(&mut self, user: u64) {
		self.sources.retain(|(_, u, _)| *u != user);
		self.awaiting_keyframe.retain(|u| *u != user);
	}
	/// A dropped or undecodable picture invalidates every later prediction until an IDR.
	pub fn require_keyframe(&mut self, user: u64) {
		if !self.awaiting_keyframe.contains(&user) {
			self.awaiting_keyframe.push(user);
		}
	}
	/// Adopt decoder-side losses so a keyframe gets requested for them too.
	pub fn absorb(&mut self, lost: &Lost) {
		if let Ok(mut lost) = lost.try_lock() {
			for user in lost.drain(..) {
				self.require_keyframe(user);
			}
		}
	}
	/// One video SSRC per user still waiting for a keyframe, for Picture Loss Indications.
	pub fn keyframe_requests(&self) -> impl Iterator<Item = u32> + '_ {
		self.awaiting_keyframe.iter().filter_map(|user| {
			self.sources
				.iter()
				.find(|(_, u, _)| u == user)
				.map(|(ssrc, _, _)| *ssrc)
		})
	}
	/// Whether a decoded access unit may be decoded: keyframes always, predictions only
	/// while the reference chain is intact.
	pub fn accept(&mut self, user: u64, keyframe: bool) -> bool {
		if keyframe {
			self.awaiting_keyframe.retain(|u| *u != user);
			return true;
		}
		!self.awaiting_keyframe.contains(&user)
	}
	/// Returns the owning user and a complete encrypted access unit when one closes.
	pub fn push(
		&mut self,
		ssrc: u32,
		sequence: u16,
		timestamp: u32,
		marker: bool,
		payload: &[u8],
	) -> Option<(u64, Vec<u8>)> {
		let (_, user, assembler) = self.sources.iter_mut().find(|(s, _, _)| *s == ssrc)?;
		let user = *user;
		let frame = assembler.push(sequence, timestamp, marker, payload);
		if std::mem::take(&mut assembler.lost) {
			self.require_keyframe(user);
		}
		frame.map(|frame| (user, frame))
	}
}

/// Decoder thread: cleartext access units in, bounded RGBA frames out through the sink.
/// Dropping the returned sender ends the thread and releases every decoder.
pub(crate) fn spawn_decoder(sink: VideoSink) -> Result<(DecoderQueue, Lost), &'static str> {
	// Predictions are small; queueing a second of them beats dropping and waiting for an IDR.
	let (send, receive) = sync_channel(64);
	let lost: Lost = Arc::new(Mutex::new(Vec::new()));
	let report = lost.clone();
	std::thread::Builder::new()
		.name("remote-video".into())
		.spawn(move || decode_loop(receive, sink, report))
		.map_err(|_| "Could not start the video decoder thread")?;
	Ok((
		DecoderQueue {
			send,
			bytes: Arc::new(tokio::sync::Semaphore::new(QUEUE_BYTES)),
		},
		lost,
	))
}

/// RFC 4585 Picture Loss Indication asking the media server for a fresh keyframe.
pub(crate) fn pli(sender: u32, media: u32) -> ([u8; 8], [u8; 4]) {
	let mut header = [0x81, 206, 0, 2, 0, 0, 0, 0];
	header[4..].copy_from_slice(&sender.to_be_bytes());
	(header, media.to_be_bytes())
}

/// Queue a frame without blocking the transport. `Ok(false)` means the queue was full and
/// the frame dropped, so the caller must wait for the next keyframe.
pub(crate) fn offer(sender: &DecoderQueue, frame: Encoded) -> Result<bool, &'static str> {
	if frame.data.len() > MAX_FRAME_BYTES || frame.data.capacity() > QUEUE_BYTES {
		return Ok(false);
	}
	let Ok(permit) = sender
		.bytes
		.clone()
		.try_acquire_many_owned(frame.data.capacity() as u32)
	else {
		return Ok(false);
	};
	match sender.send.try_send((frame, permit)) {
		Ok(()) => Ok(true),
		Err(TrySendError::Full(_)) => Ok(false),
		Err(TrySendError::Disconnected(_)) => Err("Video decoder stopped"),
	}
}

/// Hardware decoding where the OS offers it; the software decoder is the fallback and the
/// only option on the other platforms. Hardware pictures reach the sink asynchronously.
enum Backend {
	Hardware(platform::video::live::H264Decoder),
	Software(openh264::decoder::Decoder),
}
impl Backend {
	fn new(prefer_hardware: bool, user: u64, sink: &VideoSink) -> Option<Self> {
		if prefer_hardware {
			let sink = sink.clone();
			let deliver: platform::video::LiveSink = Box::new(move |frame| {
				if bounded(frame.width as usize, frame.height as usize).is_ok() {
					sink(RemoteFrame {
						user,
						width: frame.width,
						height: frame.height,
						rgba: &frame.rgba,
					});
				}
			});
			if let Ok(decoder) = platform::video::live::H264Decoder::new(deliver) {
				return Some(Self::Hardware(decoder));
			}
		}
		openh264::decoder::Decoder::new().ok().map(Self::Software)
	}
	/// Feed one access unit. Software pictures are returned; hardware ones were already
	/// delivered to the sink. `scratch` is reused so no frame-sized buffer is zeroed per frame.
	fn decode(&mut self, data: &[u8], scratch: &mut Vec<u8>) -> Result<Option<(u32, u32)>, ()> {
		match self {
			Self::Hardware(decoder) => decoder.decode(data).map(|()| None).map_err(|_| ()),
			Self::Software(decoder) => {
				let decoded = match decoder.decode(data) {
					Ok(Some(yuv)) => yuv,
					Ok(None) => return Ok(None),
					Err(_) => return Err(()),
				};
				let (width, height) = openh264::formats::YUVSource::dimensions(&decoded);
				let (width, height) = bounded(width, height)?;
				let bytes = width as usize * height as usize * 4;
				if scratch.len() != bytes {
					*scratch = vec![0; bytes];
				}
				decoded.write_rgba8(scratch);
				Ok(Some((width, height)))
			}
		}
	}
	#[cfg(all(test, target_os = "macos"))]
	fn flush(&self) {
		if let Self::Hardware(decoder) = self {
			decoder.flush();
		}
	}
}

fn decode_loop(
	receive: Receiver<(Encoded, tokio::sync::OwnedSemaphorePermit)>,
	sink: VideoSink,
	lost: Lost,
) {
	let mut decoders: HashMap<u64, Backend> = HashMap::new();
	// Users whose hardware decoder rejected the stream fall back to software.
	let mut software_only: Vec<u64> = Vec::new();
	// After a decode error, predictions are skipped until a keyframe rebuilds the references.
	let mut broken: Vec<u64> = Vec::new();
	let mut scratch = Vec::new();
	while let Ok((frame, _permit)) = receive.recv() {
		if frame.data.len() > MAX_FRAME_BYTES {
			continue;
		}
		if frame.keyframe {
			broken.retain(|user| *user != frame.user);
		} else if broken.contains(&frame.user) {
			continue;
		}
		if !decoders.contains_key(&frame.user) {
			if decoders.len() >= MAX_DECODERS {
				continue;
			}
			let Some(decoder) =
				Backend::new(!software_only.contains(&frame.user), frame.user, &sink)
			else {
				continue;
			};
			decoders.insert(frame.user, decoder);
		}
		let decoder = decoders.get_mut(&frame.user).expect("decoder inserted");
		let hardware = !matches!(decoder, Backend::Software(_));
		let decoded = match decoder.decode(&frame.data, &mut scratch) {
			Ok(Some(picture)) => picture,
			Ok(None) => continue,
			Err(()) => {
				// Corrupt or lost data: a fresh decoder waits for the next keyframe. A
				// hardware decoder that fails on a keyframe is replaced by software.
				decoders.remove(&frame.user);
				if hardware && frame.keyframe && !software_only.contains(&frame.user) {
					software_only.push(frame.user);
				}
				broken.push(frame.user);
				if let Ok(mut lost) = lost.lock()
					&& lost.len() < MAX_DECODERS
				{
					lost.push(frame.user);
				}
				continue;
			}
		};
		let (width, height) = decoded;
		sink(RemoteFrame {
			user: frame.user,
			width,
			height,
			rgba: &scratch,
		});
	}
	// Hardware pictures still in flight must land before the sink goes away.
	for decoder in decoders.values() {
		if let Backend::Hardware(decoder) = decoder {
			decoder.flush();
		}
	}
}

fn bounded(width: usize, height: usize) -> Result<(u32, u32), ()> {
	let (w, h) = (
		u32::try_from(width).map_err(|_| ())?,
		u32::try_from(height).map_err(|_| ())?,
	);
	if w == 0
		|| h == 0
		|| w > MAX_WIDTH
		|| h > MAX_WIDTH
		|| u64::from(w) * u64::from(h) > MAX_PIXELS
	{
		return Err(());
	}
	Ok((w, h))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn single_stap_and_fragmented_nal_units_rebuild_annex_b() {
		let mut assembler = Assembler::default();
		// SPS + PPS aggregated, then a fragmented IDR slice, all in one picture.
		let stap = [24, 0, 2, 0x67, 0xaa, 0, 1, 0x68];
		assert!(assembler.push(10, 900, false, &stap).is_none());
		let start = [0x7c, 0x85, 1, 2, 3];
		let middle = [0x7c, 0x05, 4, 5];
		let end = [0x7c, 0x45, 6];
		assert!(assembler.push(11, 900, false, &start).is_none());
		assert!(assembler.push(12, 900, false, &middle).is_none());
		let frame = assembler.push(13, 900, true, &end).unwrap();
		assert_eq!(
			frame,
			vec![
				0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68, 0, 0, 0, 1, 0x65, 1, 2, 3, 4, 5, 6
			]
		);
		// A single NAL picture after a lost packet is dropped, the next intact one is kept.
		assert!(assembler.push(15, 1800, true, &[0x41, 9]).is_none());
		assert_eq!(
			assembler.push(16, 2700, true, &[0x41, 9]).unwrap(),
			vec![0, 0, 0, 1, 0x41, 9]
		);
		// Unknown NAL types and oversized frames never complete.
		assert!(assembler.push(17, 3600, true, &[29, 1]).is_none());
		let mut huge = Assembler::default();
		let chunk = vec![7u8; 1200];
		let mut sequence = 0u16;
		for _ in 0..(MAX_FRAME_BYTES / 1200 + 2) {
			assert!(huge.push(sequence, 1, false, &chunk).is_none());
			sequence = sequence.wrapping_add(1);
		}
		assert!(huge.push(sequence, 1, true, &chunk).is_none());
	}
	#[test]
	fn packet_loss_requests_a_keyframe_before_accepting_predictions() {
		for missing_marker in [false, true] {
			let mut receivers = Receivers::default();
			receivers.announce(7, 700).unwrap();
			assert!(receivers.push(700, 1, 900, true, &[0x65, 1]).is_some());
			assert!(receivers.accept(7, true));
			assert!(receivers.keyframe_requests().next().is_none());
			if missing_marker {
				assert!(receivers.push(700, 2, 1800, false, &[0x41, 1]).is_none());
				// A new timestamp exposes an unfinished prior picture even without a sequence gap.
				assert!(receivers.push(700, 3, 2700, true, &[0x41, 2]).is_some());
			} else {
				assert!(receivers.push(700, 3, 1800, true, &[0x41, 1]).is_none());
			}
			assert_eq!(receivers.keyframe_requests().collect::<Vec<_>>(), vec![700]);
			assert!(!receivers.accept(7, false));
			assert!(receivers.push(700, 4, 3600, true, &[0x41, 3]).is_some());
			assert!(!receivers.accept(7, false));
			assert!(receivers.push(700, 5, 4500, true, &[0x65, 4]).is_some());
			assert!(receivers.accept(7, true));
			assert!(receivers.keyframe_requests().next().is_none());
			assert!(receivers.accept(7, false));
		}
	}
	#[test]
	fn receivers_bind_ssrcs_to_users_within_the_source_limit() {
		let mut receivers = Receivers::default();
		receivers.announce(5, 100).unwrap();
		receivers.announce(5, 101).unwrap();
		assert_eq!(
			receivers.push(100, 1, 1, true, &[0x41, 1]).unwrap(),
			(5, vec![0, 0, 0, 1, 0x41, 1])
		);
		assert!(receivers.push(999, 1, 1, true, &[0x41, 1]).is_none());
		receivers.announce(5, 0).unwrap();
		assert!(receivers.push(100, 2, 2, true, &[0x41, 1]).is_none());
		for ssrc in 1..=MAX_SOURCES as u32 {
			receivers.announce(u64::from(ssrc), ssrc).unwrap();
		}
		assert!(receivers.announce(99, 4242).is_err());
		// Predictions are gated until the first keyframe, and again after a drop.
		let mut gated = Receivers::default();
		gated.announce(7, 700).unwrap();
		assert!(!gated.accept(7, false));
		assert!(gated.accept(7, true));
		assert!(gated.accept(7, false));
		gated.require_keyframe(7);
		assert!(!gated.accept(7, false));
		assert!(is_keyframe(&[0, 0, 0, 1, 0x67, 1, 0, 0, 1, 0x65, 2]));
		assert!(!is_keyframe(&[0, 0, 0, 1, 0x41, 9]));
		assert!(bounded(1920, 1080).is_ok());
		assert!(bounded(1920, 1081).is_err());
		assert!(bounded(0, 4).is_err());
	}
	#[cfg(target_os = "macos")]
	#[test]
	fn hardware_decoder_round_trips_an_openh264_keyframe() {
		use openh264::{
			OpenH264API,
			encoder::{Encoder, EncoderConfig},
			formats::YUVBuffer,
		};
		let mut encoder =
			Encoder::with_api_config(OpenH264API::from_source(), EncoderConfig::new()).unwrap();
		let yuv = YUVBuffer::new(320, 240);
		let pictures = Arc::new(std::sync::Mutex::new(Vec::new()));
		let seen = pictures.clone();
		let sink: VideoSink = Arc::new(move |frame: RemoteFrame| {
			seen.lock()
				.unwrap()
				.push((frame.user, frame.width, frame.height, frame.rgba.to_vec()));
		});
		let mut decoder = Backend::new(true, 9, &sink).expect("hardware backend");
		assert!(matches!(decoder, Backend::Hardware(_)));
		let mut scratch = Vec::new();
		for _ in 0..3 {
			let encoded = encoder.encode(&yuv).unwrap();
			let mut data = Vec::new();
			encoded.write_vec(&mut data);
			assert!(!data.is_empty());
			assert!(decoder.decode(&data, &mut scratch).unwrap().is_none());
		}
		decoder.flush();
		let pictures = pictures.lock().unwrap();
		assert!(!pictures.is_empty(), "VideoToolbox produced pictures");
		let frame = &pictures[0];
		assert_eq!((frame.0, frame.1, frame.2), (9, 320, 240));
		assert_eq!(frame.3.len(), 320 * 240 * 4);
		assert!(frame.3.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
	}
	/// `cargo test -p discord-voice compare_decoder_backends -- --ignored --nocapture`
	#[cfg(target_os = "macos")]
	#[test]
	#[ignore]
	fn compare_decoder_backends() {
		use openh264::{
			OpenH264API,
			encoder::{BitRate, Encoder, EncoderConfig},
			formats::YUVBuffer,
		};
		let mut encoder = Encoder::with_api_config(
			OpenH264API::from_source(),
			EncoderConfig::new().bitrate(BitRate::from_bps(6_000_000)),
		)
		.unwrap();
		let (width, height) = (1280usize, 720usize);
		let frames: Vec<Vec<u8>> = (0..60u8)
			.map(|i| {
				let mut yuv = vec![0u8; width * height * 3 / 2];
				for (n, byte) in yuv[..width * height].iter_mut().enumerate() {
					*byte = ((n / width) as u8).wrapping_add(i.wrapping_mul(3));
				}
				let mut data = Vec::new();
				encoder
					.encode(&YUVBuffer::from_vec(yuv, width, height))
					.unwrap()
					.write_vec(&mut data);
				data
			})
			.collect();
		for hardware in [true, false] {
			let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
			let seen = count.clone();
			let sink: VideoSink = Arc::new(move |_| {
				seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			});
			let mut decoder = Backend::new(hardware, 1, &sink).unwrap();
			let mut scratch = Vec::new();
			// Session start-up (IOSurface, Metal) is a one-time cost; time steady state only.
			let _ = decoder.decode(&frames[0], &mut scratch);
			decoder.flush();
			count.store(0, std::sync::atomic::Ordering::Relaxed);
			let start = std::time::Instant::now();
			let mut pictures = 0;
			for frame in &frames[1..] {
				if let Ok(Some(_)) = decoder.decode(frame, &mut scratch) {
					pictures += 1;
				}
			}
			decoder.flush();
			let pictures = pictures + count.load(std::sync::atomic::Ordering::Relaxed);
			println!(
				"{} decoder: {pictures} pictures of 720p in {:?} ({:.2} ms per frame)",
				if hardware { "VideoToolbox" } else { "openh264" },
				start.elapsed(),
				start.elapsed().as_secs_f64() * 1000.0 / pictures.max(1) as f64
			);
		}
	}
	#[test]
	fn decoder_thread_rejects_garbage_and_ends_with_its_sender() {
		let frames = Arc::new(std::sync::Mutex::new(0usize));
		let seen = frames.clone();
		let (sender, lost) = spawn_decoder(Arc::new(move |_| *seen.lock().unwrap() += 1)).unwrap();
		assert!(
			offer(
				&sender,
				Encoded {
					user: 1,
					data: vec![0, 0, 0, 1, 0x65, 1, 2, 3],
					keyframe: true,
				},
			)
			.unwrap()
		);
		drop(sender);
		std::thread::sleep(std::time::Duration::from_millis(50));
		assert_eq!(*frames.lock().unwrap(), 0);
		// Garbage after a keyframe header is a decode failure the transport must learn about.
		let mut receivers = Receivers::default();
		receivers.announce(1, 100).unwrap();
		receivers.accept(1, true);
		receivers.absorb(&lost);
		assert_eq!(
			receivers.keyframe_requests().collect::<Vec<_>>(),
			if lost.lock().unwrap().is_empty() && !receivers.accept(1, false) {
				vec![100]
			} else {
				vec![]
			}
		);
		let (header, body) = pli(7, 100);
		assert_eq!(header, [0x81, 206, 0, 2, 0, 0, 0, 7]);
		assert_eq!(body, 100u32.to_be_bytes());
	}
}
