//! Incoming H.264 video: RFC 6184 depacketizing per remote SSRC and software decoding on a
//! dedicated thread. Access units, reference surfaces and display pictures are byte bounded.
//! Discord's video signaling is unofficial; live interoperability is unverified.
use std::{
	collections::HashMap,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, AtomicU64, Ordering},
		mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
	},
	time::{Duration, Instant},
};

/// Users whose decoder hit undecodable data; the transport turns these into keyframe requests.
pub(crate) type Lost = Arc<Mutex<Vec<u64>>>;

/// Largest reassembled (still DAVE-encrypted) access unit accepted from one remote sender.
pub const MAX_FRAME_BYTES: usize = platform::video::ffmpeg::MAX_UNIT;
/// Remote video senders tracked per transport; Discord forwards at most a few at once.
pub const MAX_SOURCES: usize = 16;
/// Decoders kept alive at once; each holds reference pictures for one remote user.
const MAX_DECODERS: usize = 8;
const QUEUE_BYTES: usize = 32 * 1024 * 1024;
const MAX_WIDTH: u32 = 7680;
const MAX_PIXELS: u64 = 7680 * 4320;
// A normal 1080p SPS stores 1088 coded rows and crops the final eight.
const MAX_CODED_PIXELS: u64 = 7680 * 4352;
const START_CODE: [u8; 4] = [0, 0, 0, 1];
const MAX_DECODE_AGE: Duration = Duration::from_millis(150);

/// One decoded remote picture. Native SDR planes retain their GPU conversion path;
/// shared FFmpeg pictures retain their explicit HDR metadata and high-bit-depth pixels.
pub struct RemoteFrame<'a> {
	pub user: u64,
	pub picture: RemotePicture<'a>,
}

pub enum RemotePicture<'a> {
	Native(platform::video::Picture<&'a [u8]>),
	Ffmpeg {
		picture: platform::video::ffmpeg::Picture,
		data: &'a [u8],
	},
}
impl<'a> RemotePicture<'a> {
	/// Borrow an SDR picture without converting or copying its planes.
	pub fn native(&self) -> Option<platform::video::Picture<&'a [u8]>> {
		match self {
			Self::Native(picture) => Some(picture.with_data(picture.data)),
			Self::Ffmpeg { picture, data }
				if !picture.hdr() && picture.validate().is_ok() && data.len() == picture.bytes =>
			{
				Some(platform::video::Picture {
					width: picture.width,
					height: picture.height,
					layout: platform::video::Layout::Rgba,
					color: platform::video::Color::WEBRTC,
					rotation: picture.rotation as u16,
					data,
				})
			}
			_ => None,
		}
	}

	pub fn hdr(&self) -> Option<(platform::video::ffmpeg::Picture, &'a [u8])> {
		match self {
			Self::Ffmpeg { picture, data } if picture.hdr() => Some((*picture, data)),
			_ => None,
		}
	}
}
pub type VideoSink = Arc<dyn Fn(RemoteFrame<'_>) + Send + Sync>;

pub(crate) struct SourceLifetime {
	active: AtomicBool,
	pub pictures: AtomicU64,
}

impl SourceLifetime {
	fn new() -> Self {
		Self {
			active: AtomicBool::new(true),
			pictures: AtomicU64::new(0),
		}
	}
}

struct DecodeSource {
	lifetime: Arc<SourceLifetime>,
	ssrcs: Vec<u32>,
}

pub(crate) struct DecoderQueue {
	send: SyncSender<Decode>,
	#[cfg(test)]
	worker: Option<(std::thread::JoinHandle<()>, Receiver<()>)>,
	bytes: Arc<tokio::sync::Semaphore>,
	// One cancellable lifetime per user; queued frames keep the old lifetime on restart.
	// This table has at most MAX_SOURCES entries. Old tokens survive only in the
	// bounded frame queue/worker and decoder callbacks, never an accumulating tombstone list.
	active: Mutex<HashMap<u64, DecodeSource>>,
	/// Aggregate delivery and decoder outcomes for diagnostics; lifetimes track each sender.
	pub counters: Arc<DecoderCounters>,
}

impl DecoderQueue {
	pub fn source_lifetime(&self, user: u64) -> Option<Arc<SourceLifetime>> {
		self.active
			.lock()
			.ok()?
			.get(&user)
			.map(|source| source.lifetime.clone())
	}
}

#[cfg(test)]
impl Drop for DecoderQueue {
	fn drop(&mut self) {
		let Some((worker, completed)) = self.worker.take() else {
			return;
		};
		// Close the queue before waiting: the native decoder is owned by this worker.
		drop(std::mem::replace(&mut self.send, sync_channel(0).0));
		assert!(
			!matches!(
				completed.recv_timeout(Duration::from_secs(5)),
				Err(std::sync::mpsc::RecvTimeoutError::Timeout)
			),
			"Video decoder did not terminate after its queue closed"
		);
		worker.join().expect("Video decoder worker panicked");
	}
}

#[derive(Default)]
pub(crate) struct DecoderCounters {
	pub failures: Mutex<Vec<(u64, &'static str)>>,
	pub pictures: AtomicU64,
	pub errors: AtomicU64,
	pub hardware: AtomicU64,
	pub software: AtomicU64,
	pub queue_ms: AtomicU64,
	pub stale: AtomicU64,
}

/// A cleartext Annex-B access unit handed to the decoder thread.
pub(crate) struct Encoded {
	pub timestamp: u32,
	pub codec: model::voice_settings::VideoCodec,
	pub user: u64,
	pub data: Vec<u8>,
	pub keyframe: bool,
}

enum Decode {
	Frame(
		Encoded,
		tokio::sync::OwnedSemaphorePermit,
		Instant,
		Arc<SourceLifetime>,
	),
	Wake,
	#[cfg(test)]
	Barrier(SyncSender<usize>),
}

/// True when the cleartext Annex-B access unit carries both an SPS and a PPS, so a freshly
/// created decoder can start from it.
pub(crate) fn has_parameter_sets(frame: &[u8]) -> bool {
	let (mut sps, mut pps) = (false, false);
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
		match frame.get(at + size).map(|nal| nal & 0x1f) {
			Some(7) => sps = true,
			Some(8) => pps = true,
			_ => {}
		}
		at += size;
	}
	sps && pps
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
	codec: model::voice_settings::VideoCodec,
	obu_fragment: Vec<u8>,
	obu_count: usize,
	timestamp: u32,
	next_sequence: Option<u16>,
	frame: Vec<u8>,
	fragmenting: bool,
	broken: bool,
	started: bool,
	// Preserve loss across frame resets until Receivers schedules recovery.
	lost: bool,
	// Only packets of the current picture; the next timestamp expires missing fragments.
	pending: Vec<(u16, bool, Vec<u8>)>,
	pending_bytes: usize,
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
		if let Some(expected) = self.next_sequence {
			let distance = sequence.wrapping_sub(expected);
			if distance >= 32768 {
				return None;
			} // Duplicate or late retransmission.
			if distance != 0 && self.started && timestamp == self.timestamp {
				if self.pending.iter().any(|(seq, _, _)| *seq == sequence) {
					return None;
				}
				if distance < 128
					&& self.pending.len() < 128
					&& payload.len() <= 4096
					&& self.pending_bytes + payload.len() <= 256 * 1024
				{
					self.pending_bytes += payload.len();
					self.pending.push((sequence, marker, payload.to_vec()));
					return None;
				}
				self.pending.clear();
				self.pending_bytes = 0;
			}
		}
		if timestamp != self.timestamp {
			self.pending.clear();
			self.pending_bytes = 0;
		}
		let mut complete = self.push_ordered(sequence, timestamp, marker, payload);
		while complete.is_none() {
			let Some(index) = self
				.pending
				.iter()
				.position(|(seq, _, _)| Some(*seq) == self.next_sequence)
			else {
				break;
			};
			let (seq, marker, payload) = self.pending.swap_remove(index);
			self.pending_bytes -= payload.len();
			complete = self.push_ordered(seq, timestamp, marker, &payload);
		}
		complete
	}
	fn push_ordered(
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
		if !self.broken && self.append(payload, marker).is_err() {
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
		self.obu_fragment = Vec::new();
		self.obu_count = 0;
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
	fn append(&mut self, payload: &[u8], marker: bool) -> Result<(), ()> {
		match self.codec {
			model::voice_settings::VideoCodec::H265 => return self.append_hevc(payload),
			model::voice_settings::VideoCodec::Av1 => return self.append_av1(payload, marker),
			_ => {}
		}
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
	fn append_hevc(&mut self, payload: &[u8]) -> Result<(), ()> {
		if payload.len() < 2 || payload[0] & 0x80 != 0 || payload[1] & 7 == 0 {
			return Err(());
		}
		let kind = (payload[0] >> 1) & 63;
		match kind {
			0..=47 => {
				if self.fragmenting {
					return Err(());
				}
				self.extend(&[&START_CODE, payload])
			}
			48 => {
				if self.fragmenting {
					return Err(());
				}
				let mut at = 2;
				while at < payload.len() {
					let size = payload.get(at..at + 2).ok_or(())?;
					let size = u16::from_be_bytes([size[0], size[1]]) as usize;
					at += 2;
					if size < 2 {
						return Err(());
					}
					let unit = payload.get(at..at + size).ok_or(())?;
					if unit[0] >> 1 & 63 >= 48 || unit[0] & 128 != 0 || unit[1] & 7 == 0 {
						return Err(());
					}
					self.extend(&[&START_CODE, unit])?;
					at += size;
				}
				Ok(())
			}
			49 => {
				if payload.len() < 4 {
					return Err(());
				}
				let start = payload[2] & 128 != 0;
				let end = payload[2] & 64 != 0;
				if start == self.fragmenting || start && end || payload[2] & 63 >= 48 {
					return Err(());
				}
				if start {
					self.extend(&[
						&START_CODE,
						&[(payload[0] & 0x81) | ((payload[2] & 63) << 1), payload[1]],
						&payload[3..],
					])?;
				} else {
					self.extend(&[&payload[3..]])?;
				}
				self.fragmenting = !end;
				Ok(())
			}
			_ => Err(()),
		}
	}
	fn append_av1(&mut self, payload: &[u8], marker: bool) -> Result<(), ()> {
		let flags = *payload.first().ok_or(())?;
		let continuation = flags & 128 != 0;
		let continues = flags & 64 != 0;
		let count = ((flags >> 4) & 3) as usize;
		if flags & 7 != 0
			|| continuation != self.fragmenting
			|| continuation && flags & 8 != 0
			|| marker && continues
		{
			return Err(());
		}
		let mut at = 1;
		let mut index = 0;
		while at < payload.len() {
			let size = if count != 0 && index + 1 == count {
				payload.len() - at
			} else {
				let mut size = 0usize;
				let mut closed = false;
				for shift in (0..=49).step_by(7) {
					let b = *payload.get(at).ok_or(())?;
					at += 1;
					size |= ((b & 127) as usize).checked_shl(shift).ok_or(())?;
					if b & 128 == 0 {
						closed = true;
						break;
					}
				}
				if !closed {
					return Err(());
				}
				size
			};
			if size == 0 || size > MAX_FRAME_BYTES {
				return Err(());
			}
			let element = payload.get(at..at + size).ok_or(())?;
			at += size;
			let final_element = at == payload.len();
			if index == 0 && continuation {
				self.obu_fragment.extend_from_slice(element);
			} else {
				if !self.obu_fragment.is_empty() {
					return Err(());
				}
				self.obu_fragment.extend_from_slice(element);
			}
			if self.obu_fragment.len() + self.frame.len() > MAX_FRAME_BYTES {
				return Err(());
			}
			if final_element && continues {
				self.fragmenting = true;
			} else {
				let unit = std::mem::take(&mut self.obu_fragment);
				let header = *unit.first().ok_or(())?;
				let prefix = 1 + usize::from(header & 4 != 0);
				if header & 0x81 != 0 || unit.len() < prefix || self.obu_count >= 2048 {
					return Err(());
				}
				let mut body = &unit[prefix..];
				if header & 2 != 0 {
					let (declared, consumed) = crate::video::read_av1_size(body).map_err(|_| ())?;
					body = &body[consumed..];
					if declared != body.len() {
						return Err(());
					}
				}
				self.obu_count += 1;
				if marker && final_element {
					self.extend(&[&unit])?;
				} else {
					let mut head = vec![header | 2];
					if prefix == 2 {
						head.push(unit[1]);
					}
					let mut size = body.len();
					while size >= 128 {
						head.push((size as u8 & 127) | 128);
						size >>= 7;
					}
					head.push(size as u8);
					self.extend(&[&head, body])?;
				}
				self.fragmenting = false;
			}
			index += 1;
			if count != 0 && index > count {
				return Err(());
			}
		}
		if index == 0 || count != 0 && index != count {
			return Err(());
		}
		Ok(())
	}
}

/// Remote video SSRC ownership and per-source reassembly for one media transport.
#[derive(Default)]
pub(crate) struct Receivers {
	sources: Vec<(u32, u64, Assembler)>,
	rtx: Vec<(u32, u32)>,
	/// Users whose decoder lost a reference picture; only a keyframe restarts their video.
	awaiting_keyframe: Vec<u64>,
	/// Consecutive complete, decrypted access units that failed syntax validation.
	malformed: Vec<(u64, u8)>,
	/// Depacketizer outcomes since the last `take_stats`, for diagnostics.
	stats: ReceiveStats,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ReceiveStats {
	pub unknown_ssrc: u64,
	pub incomplete: u64,
	pub complete: u64,
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
				let previous = entry.1;
				entry.1 = user;
				entry.2 = Assembler::default();
				self.rtx.retain(|(_, media)| *media != ssrc);
				if !self.sources.iter().any(|(_, owner, _)| *owner == previous) {
					self.awaiting_keyframe.retain(|owner| *owner != previous);
					self.admitted(previous);
				}
				self.admitted(user);
				self.require_keyframe(user);
			}
			return Ok(());
		}
		if self.sources.len() >= MAX_SOURCES {
			return Err("Voice channel announces more video sources than supported");
		}
		self.sources.push((ssrc, user, Assembler::default()));
		self.admitted(user);
		self.require_keyframe(user);
		Ok(())
	}
	pub fn remove(&mut self, user: u64) {
		self.retain_user_sources(user, &[]);
	}
	/// Reconcile a complete stream announcement without resetting unchanged assemblers.
	pub fn retain_user_sources(&mut self, user: u64, keep: &[u32]) {
		self.sources
			.retain(|(ssrc, owner, _)| *owner != user || keep.contains(ssrc));
		self.rtx
			.retain(|(_, media)| self.sources.iter().any(|(ssrc, _, _)| ssrc == media));
		if !self.sources.iter().any(|(_, owner, _)| *owner == user) {
			self.awaiting_keyframe.retain(|u| *u != user);
			self.admitted(user);
		}
	}
	/// Packet loss, reordering and DAVE failures never count toward a permanent data error.
	pub fn reject_malformed(&mut self, user: u64) -> Option<&'static str> {
		if !self.sources.iter().any(|(_, owner, _)| *owner == user) {
			return None;
		}
		let count = if let Some((_, count)) = self.malformed.iter_mut().find(|(u, _)| *u == user) {
			*count = count.saturating_add(1);
			*count
		} else {
			// One entry per announced owner, bounded by MAX_SOURCES.
			self.malformed.push((user, 1));
			1
		};
		if count >= 5 {
			self.remove(user);
			Some("This video repeatedly contains malformed frames; reopen it to try again")
		} else {
			self.require_keyframe(user);
			None
		}
	}
	pub fn admitted(&mut self, user: u64) {
		self.malformed.retain(|(u, _)| *u != user);
	}
	pub fn announce_rtx(&mut self, media: u32, rtx: u32) -> Result<(), &'static str> {
		if media == 0 || rtx == 0 {
			self.rtx.retain(|(_, source)| *source != media);
			return Ok(());
		}
		if !self.sources.iter().any(|(ssrc, _, _)| *ssrc == media)
			|| self.sources.iter().any(|(ssrc, _, _)| *ssrc == rtx)
			|| self
				.rtx
				.iter()
				.any(|(known, source)| *known == rtx && *source != media)
		{
			return Err("Invalid video retransmission SSRC");
		}
		self.rtx.retain(|(_, source)| *source != media);
		if self.rtx.len() >= MAX_SOURCES {
			return Err("Too many video retransmission sources");
		}
		self.rtx.push((rtx, media));
		Ok(())
	}
	/// RFC 4588: RTX payload starts with the original sequence number.
	pub fn restore_rtx(&self, ssrc: u32, payload: &mut Vec<u8>) -> Option<(u32, u16)> {
		let (_, media) = self.rtx.iter().find(|(rtx, _)| *rtx == ssrc)?;
		if payload.len() < 3 {
			return None;
		}
		let sequence = u16::from_be_bytes([payload[0], payload[1]]);
		payload.drain(..2);
		Some((*media, sequence))
	}
	/// A dropped or undecodable picture invalidates every later prediction until an IDR.
	pub fn require_keyframe(&mut self, user: u64) {
		if self.sources.iter().any(|(_, owner, _)| *owner == user)
			&& !self.awaiting_keyframe.contains(&user)
			&& self.awaiting_keyframe.len() < MAX_SOURCES
		{
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
	/// Whether any sender still owes this receiver a keyframe.
	pub fn awaiting(&self) -> bool {
		!self.awaiting_keyframe.is_empty()
	}
	/// Whether any video source has been announced for this connection.
	pub fn has_sources(&self) -> bool {
		!self.sources.is_empty()
	}
	pub fn users(&self) -> impl Iterator<Item = u64> + '_ {
		self.sources.iter().map(|(_, user, _)| *user)
	}
	/// Drains the depacketizer counters.
	pub fn take_stats(&mut self) -> ReceiveStats {
		std::mem::take(&mut self.stats)
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
	#[cfg(test)]
	pub fn push(
		&mut self,
		ssrc: u32,
		sequence: u16,
		timestamp: u32,
		marker: bool,
		payload: &[u8],
	) -> Option<(u64, Vec<u8>)> {
		self.push_codec(
			ssrc,
			sequence,
			timestamp,
			marker,
			payload,
			model::voice_settings::VideoCodec::H264,
		)
	}
	/// Returns the owning user and a complete encrypted access unit when one closes.
	pub fn push_codec(
		&mut self,
		ssrc: u32,
		sequence: u16,
		timestamp: u32,
		marker: bool,
		payload: &[u8],
		codec: model::voice_settings::VideoCodec,
	) -> Option<(u64, Vec<u8>)> {
		let Some((_, user, assembler)) = self.sources.iter_mut().find(|(s, _, _)| *s == ssrc)
		else {
			self.stats.unknown_ssrc += 1;
			return None;
		};
		let user = *user;
		if assembler.codec != codec {
			*assembler = Assembler {
				codec,
				..Default::default()
			};
			self.malformed.retain(|(owner, _)| *owner != user);
		}
		let frame = assembler.push(sequence, timestamp, marker, payload);
		let lost = std::mem::take(&mut assembler.lost);
		self.stats.incomplete += u64::from(lost);
		self.stats.complete += u64::from(frame.is_some());
		if lost {
			self.require_keyframe(user);
		}
		frame.map(|frame| (user, frame))
	}
}

/// Decoder thread: cleartext access units in, bounded RGBA frames out through the sink.
/// Dropping the returned sender ends the thread and releases every decoder.
#[cfg(test)]
pub(crate) fn spawn_decoder(sink: VideoSink) -> Result<(DecoderQueue, Lost), &'static str> {
	spawn_decoder_on_adapter(sink, None)
}
pub(crate) fn spawn_decoder_on_adapter(
	sink: VideoSink,
	adapter: Option<model::VideoAdapter>,
) -> Result<(DecoderQueue, Lost), &'static str> {
	// Keep short bursts, but recover from a fresh keyframe instead of replaying stale video.
	let (send, receive) = sync_channel(16);
	let lost: Lost = Arc::new(Mutex::new(Vec::new()));
	let report = lost.clone();
	let counters = Arc::new(DecoderCounters::default());
	let thread_counters = counters.clone();
	#[cfg(test)]
	let (completed, completion) = sync_channel(1);
	let worker = std::thread::Builder::new()
		.name("remote-video".into())
		.spawn(move || {
			decode_loop_on_adapter(receive, sink, report, thread_counters, true, adapter);
			// Signal only after every decoder and native runtime has been released.
			#[cfg(test)]
			let _ = completed.send(());
		})
		.map_err(|_| "Could not start the video decoder thread")?;
	#[cfg(not(test))]
	drop(worker);
	Ok((
		DecoderQueue {
			send,
			#[cfg(test)]
			worker: Some((worker, completion)),
			bytes: Arc::new(tokio::sync::Semaphore::new(QUEUE_BYTES)),
			active: Mutex::new(HashMap::new()),
			counters,
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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Admission {
	Queued,
	Congested,
	Malformed,
	Unsupported(&'static str),
}

#[cfg(test)]
pub(crate) fn offer(sender: &DecoderQueue, frame: Encoded) -> Result<bool, &'static str> {
	admit(sender, frame).map(|outcome| outcome == Admission::Queued)
}

/// Validate before allocating native surfaces; congestion is recoverable while a
/// rejected resolution/profile must be surfaced instead of repeated keyframe requests.
pub(crate) fn admit(sender: &DecoderQueue, frame: Encoded) -> Result<Admission, &'static str> {
	if frame.data.is_empty()
		|| frame.data.len() > MAX_FRAME_BYTES
		|| frame.data.capacity() > QUEUE_BYTES
	{
		return Ok(Admission::Malformed);
	}
	let mut supported = true;
	let validation = if frame.codec == model::voice_settings::VideoCodec::H264 {
		crate::video_sps::validate_dimensions(&frame.data, |dimensions| {
			let (width, height) = dimensions.coded;
			supported &= width <= MAX_WIDTH as usize
				&& height <= MAX_WIDTH as usize
				&& width as u64 * height as u64 <= MAX_CODED_PIXELS
				&& bounded(dimensions.visible.0, dimensions.visible.1).is_ok()
				&& dimensions.depth <= 10
				&& dimensions.chroma <= 1
				&& dimensions.references <= 16;
			supported
		})
	} else {
		crate::video::decoder_frame(&frame.data, frame.codec)
			.and_then(|data| crate::video::validate_received(&data, frame.codec))
	};
	if !supported {
		return Ok(Admission::Unsupported(
			"This video resolution or profile is not supported",
		));
	}
	if validation.is_err() {
		return Ok(Admission::Malformed);
	}
	let Ok(permit) = sender
		.bytes
		.clone()
		.try_acquire_many_owned(frame.data.capacity() as u32)
	else {
		return Ok(Admission::Congested);
	};
	let active = {
		let mut users = sender
			.active
			.lock()
			.map_err(|_| "Video decoder state unavailable")?;
		if !users.contains_key(&frame.user) && users.len() >= MAX_SOURCES {
			return Ok(Admission::Congested);
		}
		users
			.entry(frame.user)
			.or_insert_with(|| DecodeSource {
				lifetime: Arc::new(SourceLifetime::new()),
				ssrcs: Vec::new(),
			})
			.lifetime
			.clone()
	};
	match sender
		.send
		.try_send(Decode::Frame(frame, permit, Instant::now(), active))
	{
		Ok(()) => Ok(Admission::Queued),
		Err(TrySendError::Full(_)) => Ok(Admission::Congested),
		Err(TrySendError::Disconnected(_)) => Err("Video decoder stopped"),
	}
}

/// Release one participant's decoder without blocking the media transport.
pub(crate) fn remove(sender: &DecoderQueue, user: u64) {
	if let Ok(mut users) = sender.active.lock()
		&& let Some(active) = users.remove(&user)
	{
		active.lifetime.active.store(false, Ordering::Release);
	}
	// A full queue already wakes the worker; cancellation lives outside that queue.
	let _ = sender.send.try_send(Decode::Wake);
}

/// Drop decoders whose final SSRC disappeared, including SSRCs reassigned to another user.
pub(crate) fn retain_sources(sender: &DecoderQueue, receivers: &Receivers) {
	let mut cancelled = false;
	if let Ok(mut users) = sender.active.lock() {
		users.retain(|user, source| {
			let ssrcs: Vec<_> = receivers
				.sources
				.iter()
				.filter(|(_, owner, _)| owner == user)
				.map(|(ssrc, _, _)| *ssrc)
				.collect();
			if ssrcs.is_empty() {
				source.lifetime.active.store(false, Ordering::Release);
				cancelled = true;
				return false;
			}
			if !source.ssrcs.is_empty() && !source.ssrcs.iter().any(|ssrc| ssrcs.contains(ssrc)) {
				// A replacement SSRC cannot inherit old callbacks, predictions or progress.
				source.lifetime.active.store(false, Ordering::Release);
				source.lifetime = Arc::new(SourceLifetime::new());
				cancelled = true;
			}
			source.ssrcs = ssrcs;
			true
		});
		for user in receivers.users() {
			if users.len() >= MAX_SOURCES {
				break;
			}
			users.entry(user).or_insert_with(|| DecodeSource {
				lifetime: Arc::new(SourceLifetime::new()),
				ssrcs: receivers
					.sources
					.iter()
					.filter(|(_, owner, _)| *owner == user)
					.map(|(ssrc, _, _)| *ssrc)
					.collect(),
			});
		}
	}
	if cancelled {
		let _ = sender.send.try_send(Decode::Wake);
	}
}

#[derive(Debug, PartialEq, Eq)]
enum DecodeError {
	Permanent(&'static str),
	/// The stream or decoder is unusable; the decoder is rebuilt at the next keyframe.
	Failed,
	/// The native queue is full; this access unit was dropped but the decoder is healthy.
	Busy,
}

/// Skip a user's predictions until a keyframe and report the loss so a PLI gets sent.
fn mark_broken(user: u64, broken: &mut Vec<u64>, lost: &Lost) {
	if !broken.contains(&user) && broken.len() < MAX_SOURCES {
		broken.push(user);
	}
	if let Ok(mut lost) = lost.lock()
		&& !lost.contains(&user)
		&& lost.len() < MAX_SOURCES
	{
		lost.push(user);
	}
}

/// Hardware decoding where the OS offers it; the software decoder is the fallback and the
/// only option on the other platforms. Hardware pictures reach the sink asynchronously.
enum Backend {
	Ffmpeg(
		platform::video::ffmpeg::LiveDecoder,
		platform::video::ffmpeg::Picture,
	),
	#[cfg(all(test, target_os = "macos"))]
	Hardware(platform::video::live::H264Decoder),
	#[cfg(test)]
	Software(openh264::decoder::Decoder),
}
impl Backend {
	fn ffmpeg(
		codec: model::voice_settings::VideoCodec,
		hardware: bool,
		adapter: Option<model::VideoAdapter>,
	) -> Option<Self> {
		platform::video::ffmpeg::LiveDecoder::new(codec.index() as u32, adapter, hardware)
			.or_else(|_| {
				platform::video::ffmpeg::LiveDecoder::new(codec.index() as u32, None, false)
			})
			.ok()
			.map(|decoder| Self::Ffmpeg(decoder, Default::default()))
	}
	fn hardware(&self) -> bool {
		match self {
			#[cfg(all(test, target_os = "macos"))]
			Self::Hardware(_) => true,
			#[cfg(test)]
			Self::Software(_) => false,
			Self::Ffmpeg(decoder, _) => decoder.hardware(),
		}
	}
	fn picture<'a>(&self, _width: u32, _height: u32, data: &'a [u8]) -> RemotePicture<'a> {
		match self {
			Self::Ffmpeg(_, picture) => RemotePicture::Ffmpeg {
				picture: *picture,
				data: &data[..picture.bytes],
			},
			#[cfg(test)]
			_ => RemotePicture::Native(platform::video::Picture {
				width: _width,
				height: _height,
				layout: platform::video::Layout::I420,
				color: platform::video::Color::WEBRTC,
				rotation: 0,
				data,
			}),
		}
	}

	#[cfg(all(test, target_os = "macos"))]
	fn new(
		prefer_hardware: bool,
		user: u64,
		sink: &VideoSink,
		counters: &Arc<DecoderCounters>,
		lifetime: &Arc<SourceLifetime>,
	) -> Option<Self> {
		if prefer_hardware {
			let sink = sink.clone();
			let counters = counters.clone();
			let lifetime = lifetime.clone();
			let deliver: platform::video::LiveSink = Box::new(move |frame| {
				if lifetime.active.load(Ordering::Acquire)
					&& bounded(frame.width as usize, frame.height as usize).is_ok()
				{
					lifetime.pictures.fetch_add(1, Ordering::Relaxed);
					counters.pictures.fetch_add(1, Ordering::Relaxed);
					sink(RemoteFrame {
						user,
						picture: RemotePicture::Native(frame.borrow()),
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
	fn decode(
		&mut self,
		data: &[u8],
		pts: i64,
		scratch: &mut Vec<u8>,
	) -> Result<Option<(u32, u32)>, DecodeError> {
		match self {
			Self::Ffmpeg(decoder, last_picture) => {
				let mut admission_error = None;
				let validation = if decoder.codec() == 0 {
					crate::video_sps::validate_dimensions(data, |dimensions| {
						let result = decoder.reserve(
							dimensions.coded.0,
							dimensions.coded.1,
							dimensions.depth,
							dimensions.chroma,
							dimensions.references,
						);
						if let Err(error) = result {
							admission_error = Some(error);
							return false;
						}
						true
					})
				} else {
					match decoder.inspect(data) {
						Ok(Some(header)) => decoder
							.reserve(
								header.coded_width as usize,
								header.coded_height as usize,
								header.depth,
								header.chroma,
								header.references,
							)
							.map_err(|error| {
								admission_error = Some(error);
								"Unsafe video profile"
							}),
						Ok(None) => {
							if last_picture.width == 0 {
								Err("Missing video sequence header")
							} else {
								Ok(())
							}
						}
						Err(error) => {
							admission_error = Some(error);
							Err("Unsafe video profile")
						}
					}
				};
				if let Some(error) = admission_error {
					return Err(DecodeError::Permanent(error));
				}
				validation.map_err(|_| DecodeError::Failed)?;
				let classify = |error| {
					if error == platform::video::INVALID {
						DecodeError::Failed
					} else {
						DecodeError::Permanent(error)
					}
				};
				let mut latest = None;
				if !decoder.send(data, pts).map_err(classify)? {
					for _ in 0..32 {
						match decoder.receive(scratch).map_err(classify)? {
							Some(picture) => latest = Some(picture),
							None => break,
						}
					}
					if !decoder.send(data, pts).map_err(classify)? {
						return Err(DecodeError::Busy);
					}
				}
				// Drain ready output before awaiting another input. This also avoids
				// showing an older retained picture when a codec reorders its frames.
				for _ in 0..32 {
					match decoder.receive(scratch).map_err(classify)? {
						Some(picture) => latest = Some(picture),
						None => break,
					}
				}
				if let Some(picture) = latest {
					*last_picture = picture;
					Ok(Some((picture.width, picture.height)))
				} else {
					Ok(None)
				}
			}
			#[cfg(all(test, target_os = "macos"))]
			Self::Hardware(decoder) => decoder.decode(data).map(|()| None).map_err(|error| {
				if error == platform::video::BUSY {
					DecodeError::Busy
				} else {
					DecodeError::Failed
				}
			}),
			#[cfg(test)]
			Self::Software(decoder) => {
				let decoded = match decoder.decode(data) {
					Ok(Some(yuv)) => yuv,
					Ok(None) => return Ok(None),
					Err(_) => return Err(DecodeError::Failed),
				};
				use openh264::formats::YUVSource;
				let (width, height) = decoded.dimensions();
				let (w, h) = bounded(width, height).map_err(|()| DecodeError::Failed)?;
				let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
				let (luma_stride, cb_stride, cr_stride) = decoded.strides();
				// Shared across participants: clearing keeps the capacity when resolutions
				// alternate, and the planes are copied as-is (I420) for GPU colour conversion.
				scratch.clear();
				let bytes = platform::video::Layout::I420
					.len(w, h)
					.ok_or(DecodeError::Failed)?;
				// After `clear`, the exact reservation covers the whole picture.
				scratch.reserve_exact(bytes);
				platform::video::pack_plane(scratch, decoded.y(), luma_stride, width, height)
					.and_then(|()| {
						platform::video::pack_plane(
							scratch,
							decoded.u(),
							cb_stride,
							chroma_width,
							chroma_height,
						)
					})
					.and_then(|()| {
						platform::video::pack_plane(
							scratch,
							decoded.v(),
							cr_stride,
							chroma_width,
							chroma_height,
						)
					})
					.ok_or(DecodeError::Failed)?;
				Ok(Some((w, h)))
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

#[cfg(test)]
fn decode_loop(
	receive: Receiver<Decode>,
	sink: VideoSink,
	lost: Lost,
	counters: Arc<DecoderCounters>,
	prefer_hardware: bool,
) {
	decode_loop_on_adapter(receive, sink, lost, counters, prefer_hardware, None);
}
fn decode_loop_on_adapter(
	receive: Receiver<Decode>,
	sink: VideoSink,
	lost: Lost,
	counters: Arc<DecoderCounters>,
	prefer_hardware: bool,
	adapter: Option<model::VideoAdapter>,
) {
	let mut decoders: HashMap<u64, Backend> = HashMap::new();
	let mut active: HashMap<u64, Arc<SourceLifetime>> = HashMap::new();
	// Users whose hardware decoder rejected the stream fall back to software.
	let mut software_only: Vec<u64> = Vec::new();
	// After a decode error, predictions are skipped until a keyframe rebuilds the references.
	let mut broken: Vec<u64> = Vec::new();
	let mut failed_keyframes: HashMap<u64, u8> = HashMap::new();
	let mut scratch = Vec::new();
	while let Ok(message) = receive.recv() {
		failed_keyframes.retain(|user, _| {
			active
				.get(user)
				.is_some_and(|lifetime| lifetime.active.load(Ordering::Acquire))
		});
		let had_active = !active.is_empty();
		active.retain(|user, lifetime| {
			let retained = lifetime.active.load(Ordering::Acquire);
			if !retained {
				decoders.remove(user);
				software_only.retain(|known| known != user);
				broken.retain(|known| known != user);
			}
			retained
		});
		// Only cancellation of the final lifetime releases scratch; decode errors keep reuse.
		if had_active && active.is_empty() {
			scratch = Vec::new();
		}
		decoder_counts(&decoders, &counters);
		let (frame, _permit, queued, lifetime) = match message {
			Decode::Frame(frame, permit, queued, lifetime) => (frame, permit, queued, lifetime),
			Decode::Wake => continue,
			#[cfg(test)]
			Decode::Barrier(done) => {
				let _ = done.send(scratch.capacity());
				continue;
			}
		};
		if !lifetime.active.load(Ordering::Acquire) || frame.data.len() > MAX_FRAME_BYTES {
			continue;
		}
		active.insert(frame.user, lifetime.clone());
		let age = queued.elapsed();
		counters.queue_ms.fetch_max(
			age.as_millis().min(u128::from(u64::MAX)) as u64,
			Ordering::Relaxed,
		);
		if age > MAX_DECODE_AGE {
			counters.stale.fetch_add(1, Ordering::Relaxed);
			decoders.remove(&frame.user);
			decoder_counts(&decoders, &counters);
			mark_broken(frame.user, &mut broken, &lost);
			continue;
		}
		if frame.keyframe {
			broken.retain(|user| *user != frame.user);
		} else if broken.contains(&frame.user) {
			continue;
		}
		if decoders
			.get(&frame.user)
			.is_some_and(|decoder| match decoder {
				Backend::Ffmpeg(native, _) => native.codec() != frame.codec.index() as u32,
				#[cfg(test)]
				_ => frame.codec != model::voice_settings::VideoCodec::H264,
			}) {
			decoders.remove(&frame.user);
			software_only.retain(|user| *user != frame.user);
			if !frame.keyframe {
				mark_broken(frame.user, &mut broken, &lost);
				continue;
			}
		}
		if !decoders.contains_key(&frame.user) {
			if decoders.len() >= MAX_DECODERS {
				fail_source(
					frame.user,
					"Too many concurrent videos for the decoder budget",
					&lifetime,
					&counters,
				);
				continue;
			}
			let Some(decoder) = Backend::ffmpeg(
				frame.codec,
				prefer_hardware && !software_only.contains(&frame.user),
				adapter,
			) else {
				fail_source(
					frame.user,
					"No decoder is available for this video codec",
					&lifetime,
					&counters,
				);
				continue;
			};
			decoders.insert(frame.user, decoder);
		}
		let data = match crate::video::decoder_frame(&frame.data, frame.codec) {
			Ok(data) => data,
			Err(_) => {
				mark_broken(frame.user, &mut broken, &lost);
				continue;
			}
		};
		let decoder = decoders.get_mut(&frame.user).expect("decoder inserted");
		let hardware = decoder.hardware();
		let mut outcome = decoder.decode(&data, i64::from(frame.timestamp), &mut scratch);
		if hardware && frame.keyframe && outcome.is_err() && outcome != Err(DecodeError::Busy) {
			decoders.remove(&frame.user);
			if !software_only.contains(&frame.user) {
				software_only.push(frame.user);
			}
			if let Some(mut fallback) = Backend::ffmpeg(frame.codec, false, None) {
				outcome = fallback.decode(&data, i64::from(frame.timestamp), &mut scratch);
				decoders.insert(frame.user, fallback);
			}
		}
		let decoded = match outcome {
			Ok(Some(picture)) => picture,
			Ok(None) => {
				decoder_counts(&decoders, &counters);
				continue;
			}
			Err(DecodeError::Permanent(error)) => {
				fail_source(frame.user, error, &lifetime, &counters);
				decoders.remove(&frame.user);
				continue;
			}
			Err(DecodeError::Busy) => {
				// Backpressure, not a decoder fault: keep the decoder and its hardware path,
				// and skip predictions until the requested keyframe arrives.
				counters.stale.fetch_add(1, Ordering::Relaxed);
				decoder_counts(&decoders, &counters);
				mark_broken(frame.user, &mut broken, &lost);
				continue;
			}
			Err(DecodeError::Failed) => {
				// Corrupt or lost data: a fresh decoder waits for the next keyframe. A
				// hardware decoder that fails on a keyframe is replaced by software.
				counters.errors.fetch_add(1, Ordering::Relaxed);
				decoders.remove(&frame.user);
				decoder_counts(&decoders, &counters);
				if hardware && frame.keyframe && !software_only.contains(&frame.user) {
					software_only.push(frame.user);
				}
				if frame.keyframe {
					let failures = failed_keyframes.entry(frame.user).or_default();
					*failures = failures.saturating_add(1);
					if *failures >= 5 {
						fail_source(
							frame.user,
							"Video could not be decoded after repeated keyframes",
							&lifetime,
							&counters,
						);
						continue;
					}
				}
				mark_broken(frame.user, &mut broken, &lost);
				continue;
			}
		};
		decoder_counts(&decoders, &counters);
		failed_keyframes.remove(&frame.user);
		let (width, height) = decoded;
		let picture = decoders
			.get(&frame.user)
			.expect("decoded backend")
			.picture(width, height, &scratch);
		if !lifetime.active.load(Ordering::Acquire) {
			continue;
		}
		lifetime.pictures.fetch_add(1, Ordering::Relaxed);
		counters.pictures.fetch_add(1, Ordering::Relaxed);
		sink(RemoteFrame {
			user: frame.user,
			picture,
		});
	}
	// Hardware pictures still in flight must land before the sink goes away.
	#[cfg(all(test, target_os = "macos"))]
	for decoder in decoders.values() {
		if let Backend::Hardware(decoder) = decoder {
			decoder.flush();
		}
	}
}

fn fail_source(
	user: u64,
	error: &'static str,
	lifetime: &SourceLifetime,
	counters: &DecoderCounters,
) {
	counters.errors.fetch_add(1, Ordering::Relaxed);
	if lifetime.active.swap(false, Ordering::AcqRel)
		&& let Ok(mut failures) = counters.failures.lock()
		&& failures.len() < MAX_SOURCES
	{
		failures.push((user, error));
	}
}

fn decoder_counts(decoders: &HashMap<u64, Backend>, counters: &DecoderCounters) {
	let hardware = decoders
		.values()
		.filter(|decoder| decoder.hardware())
		.count();
	counters.hardware.store(hardware as u64, Ordering::Relaxed);
	counters
		.software
		.store((decoders.len() - hardware) as u64, Ordering::Relaxed);
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
	fn remote_pictures_preserve_native_planes_and_shared_hdr_metadata() {
		use platform::video::{Color, Layout, Matrix, Picture};
		for layout in [Layout::I420, Layout::Nv12, Layout::Rgba] {
			let bytes = vec![128; layout.len(4, 2).unwrap()];
			let expected = Picture {
				width: 4,
				height: 2,
				layout,
				color: Color {
					matrix: Matrix::Bt709,
					full_range: true,
				},
				rotation: 90,
				data: bytes.as_slice(),
			};
			let frame = RemotePicture::Native(expected.clone());
			let borrowed = frame.native().unwrap();
			assert_eq!(borrowed, expected);
			assert_eq!(borrowed.data.as_ptr(), bytes.as_ptr());
			assert!(frame.hdr().is_none());
		}
		let rgba = [0; 8];
		let sdr = RemotePicture::Ffmpeg {
			picture: platform::video::ffmpeg::Picture {
				rotation: 90,
				..platform::video::ffmpeg::Picture::sdr(2, 1)
			},
			data: &rgba,
		};
		assert_eq!(sdr.native().unwrap().data.as_ptr(), rgba.as_ptr());
		assert_eq!(sdr.native().unwrap().layout, Layout::Rgba);
		assert_eq!(sdr.native().unwrap().display_size(), (1, 2));
		assert!(
			RemotePicture::Ffmpeg {
				picture: platform::video::ffmpeg::Picture::sdr(2, 1),
				data: &rgba[..7],
			}
			.native()
			.is_none()
		);
		for transfer in [16, 18] {
			let data = [0; 12];
			let picture = platform::video::ffmpeg::Picture {
				width: 2,
				height: 2,
				format: 1,
				depth: 10,
				primaries: 9,
				matrix: 9,
				transfer,
				peak_nits: 1000.0,
				bytes: data.len(),
				..Default::default()
			};
			assert!(picture.validate().is_ok());
			let frame = RemotePicture::Ffmpeg {
				picture,
				data: &data,
			};
			assert!(
				frame.native().is_none(),
				"HDR must retain its high-precision renderer"
			);
			let (metadata, planes) = frame.hdr().unwrap();
			assert_eq!(
				(metadata.depth, metadata.transfer, metadata.matrix),
				(10, transfer, 9)
			);
			assert_eq!(planes.as_ptr(), data.as_ptr());
		}
	}

	#[test]
	fn av1_rtp_preserves_sized_payloads_without_duplicating_leb128_lengths() {
		use model::voice_settings::VideoCodec;
		// W=2: a sized sequence header followed by DAVE's size-less final OBU.
		// A nonminimal length makes accidental reuse of the old field visible.
		for unit in [
			vec![0x0a, 0x81, 0x00, 0x7e],
			vec![0x0e, 0x20, 0x81, 0x00, 0x7e],
		] {
			let prefix = 1 + usize::from(unit[0] & 4 != 0);
			let mut payload = vec![0x20, unit.len() as u8];
			payload.extend_from_slice(&unit);
			payload.extend_from_slice(&[0x30, 0x55]);
			let mut receiver = Assembler {
				codec: VideoCodec::Av1,
				..Default::default()
			};
			let frame = receiver.push(1, 9000, true, &payload).unwrap();
			let mut expected = unit[..prefix].to_vec();
			expected.extend_from_slice(&[1, 0x7e, 0x30, 0x55]);
			assert_eq!(frame, expected);
			assert!(!receiver.lost);
		}
		// A multi-byte OBU size can itself straddle RTP fragments.
		let mut receiver = Assembler {
			codec: VideoCodec::Av1,
			..Default::default()
		};
		assert!(receiver.push(1, 9000, false, &[0x50, 0x0a, 0xac]).is_none());
		let mut rest = vec![0xa0, 0xad, 0x02, 0x02]; // 301-byte continuation.
		rest.extend_from_slice(&[0x7e; 300]);
		rest.extend_from_slice(&[0x30, 0x55]);
		let frame = receiver.push(2, 9000, true, &rest).unwrap();
		let mut expected = vec![0x0a, 0xac, 0x02];
		expected.extend_from_slice(&[0x7e; 300]);
		expected.extend_from_slice(&[0x30, 0x55]);
		assert_eq!(frame, expected);
		// A sender may also retain the size on its final OBU.
		let mut receiver = Assembler {
			codec: VideoCodec::Av1,
			..Default::default()
		};
		assert_eq!(
			receiver.push(1, 9000, true, &[0x10, 0x32, 1, 0x55]),
			Some(vec![0x32, 1, 0x55])
		);
	}

	#[test]
	fn av1_rtp_rejects_truncated_overlong_and_mismatched_obu_lengths() {
		use model::voice_settings::VideoCodec;
		for unit in [
			vec![0x0a],
			vec![0x0a, 0x80],
			vec![0x0a, 2, 0x7e],
			vec![0x0a, 0, 0x7e],
			vec![0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f],
			vec![0x0a, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0],
		] {
			for final_element in [false, true] {
				let mut payload = if final_element {
					vec![0x10]
				} else {
					vec![0x20, unit.len() as u8]
				};
				payload.extend_from_slice(&unit);
				if !final_element {
					payload.extend_from_slice(&[0x30, 0x55]);
				}
				let mut receiver = Assembler {
					codec: VideoCodec::Av1,
					..Default::default()
				};
				assert!(
					receiver.push(1, 9000, true, &payload).is_none(),
					"{payload:?}"
				);
				assert!(receiver.lost);
				// A malformed frame cannot poison the next intact timestamp.
				assert_eq!(
					receiver.push(2, 12000, true, &[0x10, 0x30, 0x55]),
					Some(vec![0x30, 0x55])
				);
			}
		}
	}

	#[test]
	fn removed_source_ignores_late_decoder_losses() {
		let mut receivers = Receivers::default();
		let lost: Lost = Arc::default();
		for user in 1..=MAX_SOURCES as u64 + 4 {
			receivers.announce(user, 700).unwrap();
			receivers.remove(user);
			// A decode already running when cancellation occurred can report failure afterward.
			lost.lock().unwrap().push(user);
			receivers.absorb(&lost);
			assert!(!receivers.awaiting());
			assert_eq!(receivers.keyframe_requests().count(), 0);
		}
	}

	#[test]
	fn reassigned_ssrc_requests_new_owner_keyframe_and_forgets_departed_owner() {
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		receivers.announce(8, 800).unwrap();
		assert!(receivers.accept(8, true));
		receivers.announce(8, 700).unwrap();
		assert!(!receivers.accept(8, false));
		assert_eq!(receivers.awaiting_keyframe, vec![8]);
		assert_eq!(receivers.keyframe_requests().collect::<Vec<_>>(), vec![700]);
		// An owner who still has another SSRC keeps its independent recovery request.
		receivers.announce(9, 700).unwrap();
		assert_eq!(receivers.awaiting_keyframe, vec![8, 9]);
		assert_eq!(
			receivers.keyframe_requests().collect::<Vec<_>>(),
			vec![800, 700]
		);
	}

	fn decoder_cleanup_queue(capacity: usize) -> (DecoderQueue, Receiver<Decode>) {
		let (send, receive) = sync_channel(capacity);
		(
			DecoderQueue {
				send,
				worker: None,
				bytes: Arc::new(tokio::sync::Semaphore::new(QUEUE_BYTES)),
				active: Mutex::new(HashMap::new()),
				counters: Arc::default(),
			},
			receive,
		)
	}

	fn decoder_cleanup_keyframe(width: usize, height: usize) -> Vec<u8> {
		use openh264::{
			OpenH264API,
			encoder::{Encoder, EncoderConfig},
			formats::YUVBuffer,
		};
		let mut encoder =
			Encoder::with_api_config(OpenH264API::from_source(), EncoderConfig::new()).unwrap();
		let mut data = Vec::new();
		encoder
			.encode(&YUVBuffer::new(width, height))
			.unwrap()
			.write_vec(&mut data);
		data
	}

	fn decoder_cleanup_sync(queue: &DecoderQueue) -> usize {
		let (send, receive) = sync_channel(1);
		assert!(queue.send.send(Decode::Barrier(send)).is_ok());
		receive.recv_timeout(Duration::from_secs(10)).unwrap()
	}

	/// Decodes one keyframe, resending it when a loaded runner held it past `MAX_DECODE_AGE`.
	fn decoder_cleanup_decode(queue: &DecoderQueue, user: u64, data: &[u8]) {
		let deadline = Instant::now() + Duration::from_secs(10);
		loop {
			let stale = queue.counters.stale.load(Ordering::Relaxed);
			assert!(
				offer(
					queue,
					Encoded {
						timestamp: 0,
						codec: model::voice_settings::VideoCodec::H264,
						user,
						data: data.to_vec(),
						keyframe: true
					}
				)
				.unwrap()
			);
			decoder_cleanup_sync(queue);
			if queue.counters.stale.load(Ordering::Relaxed) == stale {
				return;
			}
			assert!(Instant::now() < deadline, "keyframe stayed stale");
		}
	}

	#[test]
	fn malformed_frames_fail_boundedly_and_recovery_resets_the_count() {
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		for _ in 0..4 {
			assert!(receivers.reject_malformed(7).is_none());
		}
		receivers.admitted(7);
		for _ in 0..4 {
			assert!(receivers.reject_malformed(7).is_none());
		}
		assert!(receivers.reject_malformed(7).is_some());
		assert!(!receivers.has_sources());
		assert!(!receivers.awaiting());
		assert!(receivers.malformed.is_empty());
		receivers.announce(7, 701).unwrap();
		// Packet loss only requests a keyframe; it cannot poison this counter.
		for _ in 0..100 {
			receivers.require_keyframe(7);
		}
		assert!(receivers.reject_malformed(7).is_none());
		receivers.announce(8, 701).unwrap();
		assert!(receivers.malformed.is_empty());
		assert!(receivers.reject_malformed(999).is_none());
		assert!(receivers.malformed.is_empty());
	}

	#[test]
	fn receiver_admission_rejects_separate_colour_planes_before_queueing() {
		for (chroma, separate_planes, supported) in [
			(0, false, true),
			(1, false, true),
			(2, false, false),
			(3, false, false),
			(3, true, false),
		] {
			let (queue, receive) = decoder_cleanup_queue(1);
			let data = crate::video_sps::test_sequence(
				(32, 32),
				[0; 4],
				chroma,
				[0; 2],
				separate_planes,
				true,
			);
			let outcome = admit(
				&queue,
				Encoded {
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
					user: 7,
					data,
					keyframe: true,
				},
			)
			.unwrap();
			if supported {
				assert_eq!(outcome, Admission::Queued);
				assert!(matches!(receive.try_recv(), Ok(Decode::Frame(..))));
				assert_eq!(queue.active.lock().unwrap().len(), 1);
			} else {
				assert!(matches!(outcome, Admission::Unsupported(_)));
				assert!(matches!(
					receive.try_recv(),
					Err(std::sync::mpsc::TryRecvError::Empty)
				));
				assert!(queue.active.lock().unwrap().is_empty());
			}
			assert_eq!(queue.bytes.available_permits(), QUEUE_BYTES);
		}
	}

	#[test]
	fn receiver_admission_accepts_the_former_1080p_limit() {
		let data = decoder_cleanup_keyframe(2048, 1152);
		assert!(data.len() < MAX_FRAME_BYTES);
		assert!(has_parameter_sets(&data) && is_keyframe(&data));
		// Verify this is a decodable frame, rather than malformed bytes rejected incidentally.
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		let decoded = decoder.decode(&data).unwrap().unwrap();
		assert_eq!(
			openh264::formats::YUVSource::dimensions(&decoded),
			(2048, 1152)
		);
		let (queue, receive) = decoder_cleanup_queue(1);
		assert!(
			offer(
				&queue,
				Encoded {
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
					user: 7,
					data,
					keyframe: true,
				},
			)
			.unwrap()
		);
		assert!(matches!(receive.try_recv(), Ok(Decode::Frame(..))));
		assert_eq!(queue.active.lock().unwrap().len(), 1);
		assert_eq!(queue.bytes.available_permits(), QUEUE_BYTES);
	}

	#[test]
	fn receiver_admission_accepts_1080p_and_rejects_parameter_replacement() {
		let (queue, receive) = decoder_cleanup_queue(1);
		for (width, height) in [(1920, 1080), (1080, 1920)] {
			let data = decoder_cleanup_keyframe(width, height);
			assert!(
				offer(
					&queue,
					Encoded {
						timestamp: 0,
						codec: model::voice_settings::VideoCodec::H264,
						user: 7,
						data,
						keyframe: true
					}
				)
				.unwrap()
			);
			assert!(matches!(receive.try_recv(), Ok(Decode::Frame(..))));
		}
		let lifetime = queue.source_lifetime(7).unwrap();
		let data = decoder_cleanup_keyframe(2048, 1152);
		assert!(
			offer(
				&queue,
				Encoded {
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
					user: 7,
					data,
					keyframe: true
				}
			)
			.unwrap()
		);
		assert!(lifetime.active.load(Ordering::Acquire));
		assert!(matches!(receive.try_recv(), Ok(Decode::Frame(..))));
		assert_eq!(queue.bytes.available_permits(), QUEUE_BYTES);
		// Malformed SPS and SPS without its body must also stay outside the queue.
		for data in [vec![0, 0, 1, 0x67], vec![0, 0, 1, 0x67, 66, 0, 52, 0x80]] {
			assert!(
				!offer(
					&queue,
					Encoded {
						timestamp: 0,
						codec: model::voice_settings::VideoCodec::H264,
						user: 7,
						data,
						keyframe: true
					}
				)
				.unwrap()
			);
		}
		let data = decoder_cleanup_keyframe(32, 32);
		assert!(
			offer(
				&queue,
				Encoded {
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
					user: 7,
					data,
					keyframe: true
				}
			)
			.unwrap()
		);
	}

	#[test]
	fn decoder_cleanup_invalidates_full_queue_and_allows_restart() {
		let data = decoder_cleanup_keyframe(32, 32);
		for full in [true, false] {
			let (queue, receive) = decoder_cleanup_queue(3);
			for _ in 0..if full { 3 } else { 1 } {
				assert!(
					offer(
						&queue,
						Encoded {
							timestamp: 0,
							codec: model::voice_settings::VideoCodec::H264,
							user: 7,
							data: data.clone(),
							keyframe: true
						}
					)
					.unwrap()
				);
			}
			let retired = queue.source_lifetime(7).unwrap();
			remove(&queue, 7);
			if !full {
				assert!(
					offer(
						&queue,
						Encoded {
							timestamp: 0,
							codec: model::voice_settings::VideoCodec::H264,
							user: 7,
							data: data.clone(),
							keyframe: true
						}
					)
					.unwrap()
				);
			}
			let current = queue.source_lifetime(7);
			let bytes = queue.bytes.clone();
			let counters = queue.counters.clone();
			drop(queue);
			let seen = Arc::new(Mutex::new(Vec::new()));
			let pictures = seen.clone();
			decode_loop(
				receive,
				Arc::new(move |frame| pictures.lock().unwrap().push(frame.user)),
				Arc::default(),
				counters.clone(),
				false,
			);
			assert_eq!(*seen.lock().unwrap(), if full { vec![] } else { vec![7] });
			assert_eq!(retired.pictures.load(Ordering::Relaxed), 0);
			if let Some(current) = current {
				assert_eq!(current.pictures.load(Ordering::Relaxed), 1);
			}
			assert_eq!(counters.errors.load(Ordering::Relaxed), 0);
			assert_eq!(counters.software.load(Ordering::Relaxed), u64::from(!full));
			assert_eq!(bytes.available_permits(), QUEUE_BYTES);
		}
	}

	#[test]
	fn decoder_cleanup_full_queue_rapid_off_on_keeps_only_the_new_lifetime() {
		let data = decoder_cleanup_keyframe(32, 32);
		let (queue, receive) = decoder_cleanup_queue(3);
		let (seen, pictures) = sync_channel(8);
		let (resume, paused) = sync_channel(1);
		let paused = Mutex::new(paused);
		let counters = queue.counters.clone();
		let worker = std::thread::spawn(move || {
			decode_loop(
				receive,
				Arc::new(move |frame| {
					seen.send(frame.user).unwrap();
					if frame.user == 1 {
						paused
							.lock()
							.unwrap()
							.recv_timeout(Duration::from_secs(10))
							.unwrap();
					}
				}),
				Arc::default(),
				counters,
				false,
			)
		});
		assert!(
			offer(
				&queue,
				Encoded {
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
					user: 1,
					data: data.clone(),
					keyframe: true
				}
			)
			.unwrap()
		);
		assert_eq!(pictures.recv_timeout(Duration::from_secs(10)).unwrap(), 1);
		// The first sink is paused, so all three old frames and the failed wake are deterministic.
		for _ in 0..3 {
			assert!(
				offer(
					&queue,
					Encoded {
						timestamp: 0,
						codec: model::voice_settings::VideoCodec::H264,
						user: 7,
						data: data.clone(),
						keyframe: true
					}
				)
				.unwrap()
			);
		}
		remove(&queue, 7);
		// Restart before the old queue drains; the first offer creates a fresh lifetime.
		assert!(
			!offer(
				&queue,
				Encoded {
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
					user: 7,
					data: data.clone(),
					keyframe: true
				}
			)
			.unwrap()
		);
		resume.send(()).unwrap();
		let deadline = Instant::now() + Duration::from_secs(10);
		while !offer(
			&queue,
			Encoded {
				timestamp: 0,
				codec: model::voice_settings::VideoCodec::H264,
				user: 7,
				data: data.clone(),
				keyframe: true,
			},
		)
		.unwrap()
		{
			assert!(Instant::now() < deadline, "decoder did not drain its queue");
			std::thread::yield_now();
		}
		decoder_cleanup_sync(&queue);
		assert_eq!(pictures.try_iter().collect::<Vec<_>>(), vec![7]);
		assert_eq!(queue.counters.software.load(Ordering::Relaxed), 2);
		drop(queue);
		worker.join().unwrap();
	}

	#[test]
	fn decoder_cleanup_tracks_all_sources_and_reassigned_owners() {
		let (queue, receive) = decoder_cleanup_queue(16);
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		receivers.announce(7, 710).unwrap();
		assert!(
			offer(
				&queue,
				Encoded {
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
					user: 7,
					data: vec![0, 0, 1, 0x41, 0x80],
					keyframe: true
				}
			)
			.unwrap()
		);
		let original = queue.source_lifetime(7).unwrap();
		assert!(matches!(receive.try_recv(), Ok(Decode::Frame(..))));
		retain_sources(&queue, &receivers);
		assert!(matches!(
			receive.try_recv(),
			Err(std::sync::mpsc::TryRecvError::Empty)
		));
		// Reassigning one of a user's sources must not cancel their remaining source.
		receivers.announce(8, 700).unwrap();
		retain_sources(&queue, &receivers);
		assert!(original.active.load(Ordering::Acquire));
		assert!(matches!(
			receive.try_recv(),
			Err(std::sync::mpsc::TryRecvError::Empty)
		));
		// Reassigning the final SSRC must also release its previous owner's decoder.
		receivers.announce(8, 710).unwrap();
		retain_sources(&queue, &receivers);
		assert!(!original.active.load(Ordering::Acquire));
		assert!(!queue.active.lock().unwrap().contains_key(&7));
		assert!(matches!(receive.try_recv(), Ok(Decode::Wake)));
		assert!(
			offer(
				&queue,
				Encoded {
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
					user: 8,
					data: vec![0, 0, 1, 0x41, 0x80],
					keyframe: true
				}
			)
			.unwrap()
		);
		let current = queue.source_lifetime(8).unwrap();
		receivers.announce(8, 0).unwrap();
		retain_sources(&queue, &receivers);
		assert!(!current.active.load(Ordering::Acquire));
	}

	#[test]
	fn announced_picture_lifetimes_are_bounded_and_replacement_starts_fresh() {
		let (queue, _receive) = decoder_cleanup_queue(1);
		let mut receivers = Receivers::default();
		for user in 1..=MAX_SOURCES as u64 {
			receivers.announce(user, user as u32).unwrap();
		}
		retain_sources(&queue, &receivers);
		assert_eq!(queue.active.lock().unwrap().len(), MAX_SOURCES);
		let mut original = queue.source_lifetime(1).unwrap();
		for ssrc in 100..132 {
			original.pictures.store(20, Ordering::Relaxed);
			receivers.retain_user_sources(1, &[]);
			receivers.announce(1, ssrc).unwrap();
			retain_sources(&queue, &receivers);
			let current = queue.source_lifetime(1).unwrap();
			assert!(!original.active.load(Ordering::Acquire));
			assert!(!Arc::ptr_eq(&original, &current));
			assert_eq!(current.pictures.load(Ordering::Relaxed), 0);
			assert_eq!(queue.active.lock().unwrap().len(), MAX_SOURCES);
			// Repeating the same SSRC announcement preserves its current progress.
			current.pictures.store(3, Ordering::Relaxed);
			retain_sources(&queue, &receivers);
			assert!(Arc::ptr_eq(&current, &queue.source_lifetime(1).unwrap()));
			assert_eq!(current.pictures.load(Ordering::Relaxed), 3);
			original = current;
		}
		receivers.remove(1);
		retain_sources(&queue, &receivers);
		assert!(!original.active.load(Ordering::Acquire));
		assert!(queue.source_lifetime(1).is_none());
		assert_eq!(queue.active.lock().unwrap().len(), MAX_SOURCES - 1);
	}

	#[test]
	fn decoder_cleanup_releases_slots_for_another_participant() {
		let data = decoder_cleanup_keyframe(32, 32);
		let (queue, receive) = decoder_cleanup_queue(16);
		let counters = queue.counters.clone();
		let seen = Arc::new(Mutex::new(Vec::new()));
		let pictures = seen.clone();
		let worker = std::thread::spawn(move || {
			decode_loop(
				receive,
				Arc::new(move |frame| pictures.lock().unwrap().push(frame.user)),
				Arc::default(),
				counters,
				false,
			)
		});
		let mut receivers = Receivers::default();
		for user in 1..=MAX_DECODERS as u64 {
			crate::transport::announce_video(
				&mut receivers,
				&queue,
				user,
				&serde_json::json!({"video_ssrc": user}),
			)
			.unwrap();
			decoder_cleanup_decode(&queue, user, &data);
		}
		assert_eq!(
			queue.counters.software.load(Ordering::Relaxed),
			MAX_DECODERS as u64
		);
		crate::transport::announce_video(
			&mut receivers,
			&queue,
			1,
			&serde_json::json!({"video_ssrc": 0, "streams": [{"type": "video", "ssrc": 1}]}),
		)
		.unwrap();
		decoder_cleanup_sync(&queue);
		assert_eq!(
			queue.counters.software.load(Ordering::Relaxed),
			MAX_DECODERS as u64
		);
		crate::transport::announce_video(
			&mut receivers,
			&queue,
			1,
			&serde_json::json!({"video_ssrc": 0, "streams": []}),
		)
		.unwrap();
		decoder_cleanup_sync(&queue);
		assert_eq!(
			queue.counters.software.load(Ordering::Relaxed),
			MAX_DECODERS as u64 - 1
		);
		crate::transport::announce_video(
			&mut receivers,
			&queue,
			9,
			&serde_json::json!({"video_ssrc": 9}),
		)
		.unwrap();
		decoder_cleanup_decode(&queue, 9, &data);
		assert_eq!(seen.lock().unwrap().last(), Some(&9));
		assert_eq!(
			queue.counters.software.load(Ordering::Relaxed),
			MAX_DECODERS as u64
		);
		assert!(decoder_cleanup_sync(&queue) > 0);
		for user in 2..=9 {
			crate::transport::announce_video(
				&mut receivers,
				&queue,
				user,
				&serde_json::json!({"video_ssrc": 0, "streams": []}),
			)
			.unwrap();
		}
		assert_eq!(
			decoder_cleanup_sync(&queue),
			0,
			"final camera-off releases scratch"
		);
		assert_eq!(queue.counters.software.load(Ordering::Relaxed), 0);
		drop(queue);
		worker.join().unwrap();
	}

	#[test]
	fn parameter_set_detection_needs_both_sps_and_pps() {
		assert!(has_parameter_sets(&[
			0, 0, 0, 1, 0x67, 1, 0, 0, 1, 0x68, 2, 0, 0, 0, 1, 0x65, 3
		]));
		assert!(!has_parameter_sets(&[
			0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x65, 3
		]));
		assert!(!has_parameter_sets(&[0, 0, 0, 1, 0x65, 3]));
		assert!(!has_parameter_sets(&[]));
	}

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

		{
			let mut receivers = Receivers::default();
			receivers.announce(7, 700).unwrap();
			assert!(receivers.push(999, 1, 900, true, &[0x65, 1]).is_none());
			assert!(receivers.push(700, 1, 900, true, &[0x65, 1]).is_some());
			assert!(receivers.push(700, 3, 1800, true, &[0x41, 1]).is_none());
			let stats = receivers.take_stats();
			assert_eq!(
				(stats.unknown_ssrc, stats.incomplete, stats.complete),
				(1, 1, 1)
			);
			assert!(receivers.awaiting());
			let stats = receivers.take_stats();
			assert_eq!(
				(stats.unknown_ssrc, stats.incomplete, stats.complete),
				(0, 0, 0)
			);
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
		assert!(bounded(1920, 1081).is_ok());
		assert!(bounded(6144, 2560).is_ok());
		assert!(bounded(7680, 4320).is_ok());
		assert!(bounded(7680, 4321).is_err());
		assert!(bounded(0, 4).is_err());
	}
	#[test]
	fn software_decoders_reuse_scratch_across_alternating_resolutions() {
		use openh264::{
			OpenH264API,
			encoder::{Encoder, EncoderConfig},
			formats::YUVBuffer,
		};
		let mut streams: Vec<_> = [(64, 64), (32, 32)]
			.into_iter()
			.map(|(width, height)| {
				let mut encoder =
					Encoder::with_api_config(OpenH264API::from_source(), EncoderConfig::new())
						.unwrap();
				let mut data = Vec::new();
				encoder
					.encode(&YUVBuffer::new(width, height))
					.unwrap()
					.write_vec(&mut data);
				let decoder = Backend::Software(openh264::decoder::Decoder::new().unwrap());
				(decoder, data, (width as u32, height as u32))
			})
			.collect();
		// Growing from the smaller picture still holds the whole larger one. `reserve_exact`
		// only promises a lower bound, so the exact capacity is left to the allocator.
		let mut grown = Vec::new();
		for index in [1, 0] {
			let (decoder, data, _) = &mut streams[index];
			assert!(decoder.decode(data, 0, &mut grown).is_ok());
		}
		let bytes = platform::video::Layout::I420.len(64, 64).unwrap();
		assert!(grown.capacity() >= bytes);
		let mut scratch = Vec::new();
		let mut allocation = None;
		for _ in 0..4 {
			for (decoder, data, dimensions) in &mut streams {
				assert_eq!(decoder.decode(data, 0, &mut scratch), Ok(Some(*dimensions)));
				// Exactly one packed I420 picture; the larger allocation is kept.
				let bytes = platform::video::Layout::I420
					.len(dimensions.0, dimensions.1)
					.unwrap();
				assert_eq!(scratch.len(), bytes);
				let current = (scratch.as_ptr(), scratch.capacity());
				assert_eq!(*allocation.get_or_insert(current), current);
			}
		}
	}

	/// `cargo test --release -p discord-voice compare_alternating_software_decode -- --ignored --nocapture`
	#[test]
	#[ignore]
	fn compare_alternating_software_decode() {
		use openh264::{
			OpenH264API,
			encoder::{Encoder, EncoderConfig},
			formats::YUVBuffer,
		};
		let streams: Vec<_> = [(1920, 1080), (1280, 720)]
			.into_iter()
			.map(|(width, height)| {
				let mut encoder =
					Encoder::with_api_config(OpenH264API::from_source(), EncoderConfig::new())
						.unwrap();
				let mut data = Vec::new();
				encoder
					.encode(&YUVBuffer::new(width, height))
					.unwrap()
					.write_vec(&mut data);
				data
			})
			.collect();
		for run in 0..6 {
			let mut decoders = streams
				.iter()
				.map(|_| Backend::Software(openh264::decoder::Decoder::new().unwrap()))
				.collect::<Vec<_>>();
			let mut scratch = Vec::new();
			let mut length_changes = 0;
			let mut peak_capacity = 0;
			let start = std::time::Instant::now();
			for _ in 0..60 {
				for (decoder, data) in decoders.iter_mut().zip(&streams) {
					let previous = scratch.len();
					let (width, height) = decoder.decode(data, 0, &mut scratch).unwrap().unwrap();
					std::hint::black_box((width, height, &scratch[..]));
					length_changes += usize::from(previous != scratch.len());
					peak_capacity = peak_capacity.max(scratch.capacity());
				}
			}
			println!(
				"alternating software run={run} (0=warmup) frames=120 ms={:.3} scratch_length_changes={length_changes} peak_scratch_capacity={peak_capacity}",
				start.elapsed().as_secs_f64() * 1000.0
			);
		}
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
			let picture = frame.picture.native().expect("native VideoToolbox picture");
			seen.lock().unwrap().push((
				frame.user,
				picture.width,
				picture.height,
				picture.to_rgba().unwrap().2,
			));
		});
		let counters = Arc::new(DecoderCounters::default());
		let lifetime = Arc::new(SourceLifetime::new());
		let mut decoder =
			Backend::new(true, 9, &sink, &counters, &lifetime).expect("hardware backend");
		assert!(matches!(decoder, Backend::Hardware(_)));
		let mut scratch = Vec::new();
		for _ in 0..3 {
			let encoded = encoder.encode(&yuv).unwrap();
			let mut data = Vec::new();
			encoded.write_vec(&mut data);
			assert!(!data.is_empty());
			assert!(decoder.decode(&data, 0, &mut scratch).unwrap().is_none());
		}
		decoder.flush();
		let delivered = {
			let pictures = pictures.lock().unwrap();
			assert!(!pictures.is_empty(), "VideoToolbox produced pictures");
			let frame = &pictures[0];
			assert_eq!((frame.0, frame.1, frame.2), (9, 320, 240));
			assert_eq!(frame.3.len(), 320 * 240 * 4);
			assert!(frame.3.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
			pictures.len() as u64
		};
		assert_eq!(lifetime.pictures.load(Ordering::Relaxed), delivered);
		assert_eq!(counters.pictures.load(Ordering::Relaxed), delivered);
		lifetime.active.store(false, Ordering::Release);
		let mut data = Vec::new();
		encoder.encode(&yuv).unwrap().write_vec(&mut data);
		decoder.decode(&data, 0, &mut scratch).unwrap();
		decoder.flush();
		assert_eq!(pictures.lock().unwrap().len() as u64, delivered);
		assert_eq!(
			counters.pictures.load(Ordering::Relaxed),
			delivered,
			"cancelled hardware callbacks must not reset the stall detector"
		);
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
			let mut decoder = Backend::new(
				hardware,
				1,
				&sink,
				&Arc::default(),
				&Arc::new(SourceLifetime::new()),
			)
			.unwrap();
			let mut scratch = Vec::new();
			// Session start-up (IOSurface, Metal) is a one-time cost; time steady state only.
			let _ = decoder.decode(&frames[0], 0, &mut scratch);
			decoder.flush();
			count.store(0, std::sync::atomic::Ordering::Relaxed);
			let start = std::time::Instant::now();
			let mut pictures = 0;
			for frame in &frames[1..] {
				if let Ok(Some(_)) = decoder.decode(frame, 0, &mut scratch) {
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
					timestamp: 0,
					codec: model::voice_settings::VideoCodec::H264,
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
