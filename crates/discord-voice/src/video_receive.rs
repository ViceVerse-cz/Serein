//! Incoming H.264/H.265 video: bounded RTP reassembly per remote SSRC and decoding on a
//! dedicated thread. Frames stay bounded (1080p RGBA) and no video is ever written to disk.
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

use crate::video_codec::Codec;

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
const MAX_DECODE_AGE: Duration = Duration::from_millis(150);

#[path = "video_reorder.rs"]
mod reorder;

/// One decoded remote picture in the decoder's native layout (NV12 or I420 on every
/// current backend); the renderer converts colour.
pub struct RemoteFrame<'a> {
	pub presentation: Option<crate::Presentation>,
	pub user: u64,
	pub picture: platform::video::Picture<&'a [u8]>,
}
pub type VideoSink = Arc<dyn Fn(RemoteFrame<'_>) + Send + Sync>;

pub(crate) struct DecoderQueue {
	send: SyncSender<Decode>,
	#[cfg(test)]
	worker: Option<(std::thread::JoinHandle<()>, Receiver<()>)>,
	bytes: Arc<tokio::sync::Semaphore>,
	// One cancellable lifetime per user; queued frames keep the old lifetime on restart.
	// This table has at most MAX_SOURCES entries. Old tokens survive only in the
	// bounded frame queue/worker and decoder callbacks, never an accumulating tombstone list.
	active: Mutex<HashMap<u64, Arc<AtomicBool>>>,
	/// Decoded pictures delivered to the sink and decoder failures, for diagnostics only.
	pub counters: Arc<DecoderCounters>,
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
	pub sizes: crate::diagnostics::VideoSizeCounters,
	pub pictures: AtomicU64,
	pub errors: AtomicU64,
	pub hardware: AtomicU64,
	pub software: AtomicU64,
	pub queue_ms: AtomicU64,
	pub stale: AtomicU64,
	pub decode_us: AtomicU64,
}

/// A cleartext Annex-B access unit handed to the decoder thread.
pub(crate) struct Encoded {
	pub codec: Codec,
	pub user: u64,
	pub data: Vec<u8>,
	pub keyframe: bool,
}

enum Decode {
	Frame(
		Encoded,
		tokio::sync::OwnedSemaphorePermit,
		Instant,
		Arc<AtomicBool>,
		Option<crate::Presentation>,
	),
	Wake,
	#[cfg(test)]
	Barrier(SyncSender<usize>),
}

/// True when the cleartext Annex-B access unit carries both an SPS and a PPS, so a freshly
/// created decoder can start from it.
pub(crate) fn has_parameter_sets(frame: &[u8]) -> bool {
	Codec::H264.has_parameter_sets(frame)
}

pub(crate) fn is_keyframe(frame: &[u8]) -> bool {
	Codec::H264.is_keyframe(frame)
}

/// Reassembles RTP payloads of one SSRC into Annex-B access units (single NAL, STAP-A, FU-A).
#[derive(Default)]
pub(crate) struct Assembler {
	codec: Codec,
	fragment_header: Option<[u8; 2]>,
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
	fn push(
		&mut self,
		sequence: u16,
		timestamp: u32,
		marker: bool,
		payload: &[u8],
	) -> Option<Vec<u8>> {
		// Empty RTP payloads are padding/keepalives, not H.264 access units. They
		// consume a sequence number but have no picture timestamp or marker semantics.
		if payload.is_empty() {
			if self
				.next_sequence
				.is_some_and(|expected| expected != sequence)
			{
				self.broken = true;
				self.lost = true;
			}
			self.next_sequence = Some(sequence.wrapping_add(1));
			return None;
		}
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
		self.fragment_header = None;
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
		if self.codec == Codec::H265 {
			return self.append_h265(payload);
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
	/// RFC 7798 single-layer SRST, without DONL (the WebRTC packetization mode).
	fn append_h265(&mut self, payload: &[u8]) -> Result<(), ()> {
		let header = payload.get(..2).ok_or(())?;
		// Reject forbidden bits, layered streams and invalid temporal IDs.
		if header[0] & 0x81 != 0 || header[1] >> 3 != 0 || header[1] & 7 == 0 {
			return Err(());
		}
		match (header[0] >> 1) & 63 {
			0..=47 if !self.fragmenting => self.extend(&[&START_CODE, payload]),
			48 if !self.fragmenting => {
				let mut at = 2;
				let mut count = 0;
				while at < payload.len() {
					let length = payload.get(at..at + 2).ok_or(())?;
					let size = usize::from(u16::from_be_bytes([length[0], length[1]]));
					at += 2;
					let nal = payload
						.get(at..at + size)
						.filter(|nal| nal.len() >= 2)
						.ok_or(())?;
					if nal[0] & 0x81 != 0
						|| nal[1] >> 3 != 0
						|| nal[1] & 7 == 0 || (nal[0] >> 1) & 63 >= 48
					{
						return Err(());
					}
					self.extend(&[&START_CODE, nal])?;
					at += size;
					count += 1;
				}
				(count >= 2).then_some(()).ok_or(())
			}
			49 if payload.len() >= 4 => {
				let fu = payload[2];
				let start = fu & 0x80 != 0;
				let end = fu & 0x40 != 0;
				let nal = [(header[0] & 0x81) | ((fu & 63) << 1), header[1]];
				if (start && end) || start == self.fragmenting || fu & 63 >= 48 {
					return Err(());
				}
				if start {
					self.extend(&[&START_CODE, &nal, &payload[3..]])?;
					self.fragment_header = Some(nal);
				} else {
					if self.fragment_header != Some(nal) {
						return Err(());
					}
					self.extend(&[&payload[3..]])?;
				}
				self.fragmenting = !end;
				if end {
					self.fragment_header = None;
				}
				Ok(())
			}
			_ => Err(()),
		}
	}
}

/// Remote video SSRC ownership and per-source reassembly for one media transport.
#[derive(Default)]
struct Source {
	codec: Option<Codec>,
	assembler: Assembler,
	window: reorder::Window,
	/// Forward original-sequence progress; duplicates/old RTX must not steal recovery.
	activity: Option<(u16, Instant)>,
	last_pli: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct Receivers {
	sources: Vec<(u32, u64, Source)>,
	rtx: Vec<(u32, u32)>,
	/// Users whose decoder lost a reference picture; only a keyframe restarts their video.
	awaiting_keyframe: Vec<u64>,
	/// Depacketizer outcomes since the last `take_stats`, for diagnostics.
	stats: ReceiveStats,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ReceiveStats {
	pub unknown_ssrc: u64,
	pub incomplete: u64,
	pub complete: u64,
	pub repaired: u64,
	pub padding_packets: u64,
}
/// One complete encrypted access unit, retaining its source's 90 kHz RTP clock.
pub(crate) struct Received {
	pub codec: Codec,
	pub user: u64,
	pub source: u32,
	pub timestamp: u32,
	pub data: Vec<u8>,
}
impl Receivers {
	/// Incoming codec is identified by an authenticated, advertised RTP payload type,
	/// independently per source. Session video_codec describes our outgoing selection.
	/// Old primary packets and RTX from a retired codec cannot reset the current epoch.
	pub fn select_codec(
		&mut self,
		ssrc: u32,
		sequence: u16,
		codec: Codec,
		rtx: bool,
	) -> Result<Option<u64>, ()> {
		let Some((_, user, source)) = self.sources.iter_mut().find(|(s, _, _)| *s == ssrc) else {
			self.stats.unknown_ssrc += 1;
			return Err(());
		};
		if source.codec == Some(codec) {
			return Ok(None);
		}
		if source.codec.is_none() {
			source.codec = Some(codec);
			return Ok(None);
		}
		if rtx
			|| source.activity.is_some_and(|(last, _)| {
				let distance = sequence.wrapping_sub(last);
				distance == 0 || distance >= 32768
			}) {
			return Err(());
		}
		let user = *user;
		*source = Source {
			codec: Some(codec),
			..Source::default()
		};
		self.require_keyframe(user);
		Ok(Some(user))
	}
	/// Bind an announced video SSRC to a user; zero clears that user's sources.
	pub fn announce(&mut self, user: u64, ssrc: u32) -> Result<(), &'static str> {
		if ssrc == 0 {
			self.remove(user);
			return Ok(());
		}
		if let Some(entry) = self.sources.iter_mut().find(|(s, _, _)| *s == ssrc) {
			if entry.1 != user {
				entry.1 = user;
				entry.2 = Source::default();
				self.rtx.retain(|(_, media)| *media != ssrc);
			}
			return Ok(());
		}
		if self.sources.len() >= MAX_SOURCES {
			return Err("Voice channel announces more video sources than supported");
		}
		self.sources.push((ssrc, user, Source::default()));
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
		}
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
	/// Recover the source actually receiving media, falling back to announcement order
	/// before the first packet. Announced but idle simulcast layers need no refresh.
	pub fn keyframe_requests(&self) -> impl Iterator<Item = u32> + '_ {
		self.awaiting_keyframe.iter().filter_map(|user| {
			self.sources
				.iter()
				.filter(|(_, u, _)| u == user)
				.min_by_key(|(_, _, source)| std::cmp::Reverse(source.activity.map(|(_, at)| at)))
				.map(|(ssrc, _, _)| *ssrc)
		})
	}
	/// Cooldowns start only when a request is issued, independently per source. Idle
	/// ticks cannot postpone a fresh loss; retries remain capped at two per second.
	pub fn take_keyframe_requests(&mut self, now: Instant) -> Vec<u32> {
		let pending: Vec<_> = self.keyframe_requests().collect();
		pending
			.into_iter()
			.filter(|ssrc| {
				let Some((_, _, source)) =
					self.sources.iter_mut().find(|(known, _, _)| known == ssrc)
				else {
					return false;
				};
				if source.last_pli.is_some_and(|at| {
					now.saturating_duration_since(at) < Duration::from_millis(500)
				}) {
					return false;
				}
				source.last_pli = Some(now);
				true
			})
			.collect()
	}
	/// Whether any sender still owes this receiver a keyframe.
	pub fn awaiting(&self) -> bool {
		!self.awaiting_keyframe.is_empty()
	}
	/// Whether any video source has been announced for this connection.
	pub fn has_sources(&self) -> bool {
		!self.sources.is_empty()
	}
	pub fn has_source(&self, ssrc: u32) -> bool {
		self.sources.iter().any(|(known, _, _)| *known == ssrc)
	}
	pub fn owner(&self, ssrc: u32) -> Option<u64> {
		self.sources
			.iter()
			.find(|(known, _, _)| *known == ssrc)
			.map(|(_, user, _)| *user)
	}
	/// Ask every announced sender for a keyframe. Used when video stops without the
	/// depacketizer seeing loss, which no per-picture signal would ever reveal.
	pub fn require_all_keyframes(&mut self) {
		for index in 0..self.sources.len() {
			self.require_keyframe(self.sources[index].1);
		}
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
	/// Queue RTP and return the first ready access unit. Call `pop` afterwards: filling
	/// one hole can release several pictures, including later RTP timestamps.
	pub fn push(
		&mut self,
		ssrc: u32,
		sequence: u16,
		timestamp: u32,
		marker: bool,
		payload: &[u8],
	) -> Option<(u64, Vec<u8>)> {
		self.push_timed(ssrc, sequence, timestamp, marker, payload)
			.map(|frame| (frame.user, frame.data))
	}
	pub fn push_timed(
		&mut self,
		ssrc: u32,
		sequence: u16,
		timestamp: u32,
		marker: bool,
		payload: &[u8],
	) -> Option<Received> {
		let Some((_, _, source)) = self.sources.iter_mut().find(|(s, _, _)| *s == ssrc) else {
			self.stats.unknown_ssrc += 1;
			return None;
		};
		if !payload.is_empty() && source.codec.is_none() {
			// Cameras and the legacy helpers accept only H.264. Stream packets select
			// their advertised codec explicitly before entering this common queue.
			source.codec = Some(Codec::H264);
		}
		let now = Instant::now();
		self.stats.padding_packets += u64::from(payload.is_empty());
		if source.activity.is_none_or(|(last, _)| {
			let distance = sequence.wrapping_sub(last);
			distance != 0 && distance < 32768
		}) {
			if payload.is_empty() {
				// Advance the sequence watermark through long padding runs without
				// making an otherwise idle media source the recovery target.
				if let Some((last, _)) = &mut source.activity {
					*last = sequence;
				}
			} else {
				source.activity = Some((sequence, now));
			}
		}
		if source
			.window
			.push(sequence, timestamp, marker, payload, now)
		{
			source.assembler = Assembler {
				lost: true,
				..Assembler::default()
			};
		}
		self.pop_timed_at(now)
	}
	/// Also called on transport ticks so a missing packet cannot hold video forever.
	pub fn pop(&mut self) -> Option<(u64, Vec<u8>)> {
		self.pop_at(Instant::now())
	}
	fn pop_at(&mut self, now: Instant) -> Option<(u64, Vec<u8>)> {
		self.pop_timed_at(now).map(|frame| (frame.user, frame.data))
	}
	pub fn pop_timed(&mut self) -> Option<Received> {
		self.pop_timed_at(Instant::now())
	}
	fn pop_timed_at(&mut self, now: Instant) -> Option<Received> {
		for index in 0..self.sources.len() {
			let (ssrc, user, source) = &mut self.sources[index];
			let ssrc = *ssrc;
			let user = *user;
			let mut frame = None;
			loop {
				let (packet, expired) = source.window.pop(now);
				if expired {
					source.assembler = Assembler {
						lost: true,
						..Assembler::default()
					};
				}
				let Some(packet) = packet else {
					break;
				};
				source.assembler.codec = source.codec.unwrap_or_default();
				frame = source.assembler.push(
					packet.sequence,
					packet.timestamp,
					packet.marker,
					&packet.payload,
				);
				if frame.is_some() {
					break;
				}
			}
			let lost = std::mem::take(&mut source.assembler.lost);
			let timestamp = source.assembler.timestamp;
			let codec = source.codec.unwrap_or_default();
			self.stats.repaired += std::mem::take(&mut source.window.repaired);
			self.stats.incomplete += u64::from(lost);
			self.stats.complete += u64::from(frame.is_some());
			if lost {
				self.require_keyframe(user);
			}
			if let Some(frame) = frame {
				return Some(Received {
					codec,
					user,
					source: ssrc,
					timestamp,
					data: frame,
				});
			}
		}
		None
	}
	/// Only request repair for announced RTX pairs. Source removal drops all pending
	/// packets and feedback; source ownership changes cannot inherit another user's holes.
	pub fn nacks(&mut self, sender: u32, now: Instant) -> Vec<([u8; 8], Vec<u8>)> {
		self.sources
			.iter_mut()
			.filter_map(|(ssrc, _, source)| {
				self.rtx
					.iter()
					.any(|(_, media)| media == ssrc)
					.then(|| source.window.nack(sender, *ssrc, now))
					.flatten()
			})
			.collect()
	}
}

/// Decoder thread: cleartext access units in, bounded RGBA frames out through the sink.
/// Dropping the returned sender ends the thread and releases every decoder.
pub(crate) fn spawn_decoder(sink: VideoSink) -> Result<(DecoderQueue, Lost), &'static str> {
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
			decode_loop(receive, sink, report, thread_counters, true);
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

/// Queue a frame without blocking the transport. `Ok(false)` means the queue was full and
/// the frame dropped, so the caller must wait for the next keyframe.
pub(crate) fn offer(sender: &DecoderQueue, frame: Encoded) -> Result<bool, &'static str> {
	offer_timed(sender, frame, None)
}
pub(crate) fn offer_timed(
	sender: &DecoderQueue,
	frame: Encoded,
	presentation: Option<crate::Presentation>,
) -> Result<bool, &'static str> {
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
	let active = {
		let mut users = sender
			.active
			.lock()
			.map_err(|_| "Video decoder state unavailable")?;
		if !users.contains_key(&frame.user) && users.len() >= MAX_SOURCES {
			return Ok(false);
		}
		users
			.entry(frame.user)
			.or_insert_with(|| Arc::new(AtomicBool::new(true)))
			.clone()
	};
	match sender.send.try_send(Decode::Frame(
		frame,
		permit,
		Instant::now(),
		active,
		presentation,
	)) {
		Ok(()) => Ok(true),
		Err(TrySendError::Full(_)) => Ok(false),
		Err(TrySendError::Disconnected(_)) => Err("Video decoder stopped"),
	}
}

/// Release one participant's decoder without blocking the media transport.
pub(crate) fn remove(sender: &DecoderQueue, user: u64) {
	if let Ok(mut users) = sender.active.lock()
		&& let Some(active) = users.remove(&user)
	{
		active.store(false, Ordering::Release);
	}
	// A full queue already wakes the worker; cancellation lives outside that queue.
	let _ = sender.send.try_send(Decode::Wake);
}

/// Drop decoders whose final SSRC disappeared, including SSRCs reassigned to another user.
pub(crate) fn retain_sources(sender: &DecoderQueue, receivers: &Receivers) {
	let mut cancelled = false;
	if let Ok(mut users) = sender.active.lock() {
		users.retain(|user, active| {
			let retained = receivers.sources.iter().any(|(_, owner, _)| owner == user);
			if !retained {
				active.store(false, Ordering::Release);
				cancelled = true;
			}
			retained
		});
	}
	if cancelled {
		let _ = sender.send.try_send(Decode::Wake);
	}
}

#[derive(Debug, PartialEq, Eq)]
enum DecodeError {
	/// The stream or decoder is unusable; the decoder is rebuilt at the next keyframe.
	Failed,
	/// The native queue is full; the caller retains this access unit and the decoder is healthy.
	Busy,
}

/// Native queues can briefly fill when intact pictures arrive in a network burst. Keep
/// the reference picture until capacity returns, within its existing decode-age budget.
/// Only this dedicated decoder worker waits; transport and rendering remain nonblocking.
fn decode_when_ready<T>(
	deadline: Instant,
	lifetime: &AtomicBool,
	mut decode: impl FnMut() -> Result<T, DecodeError>,
	mut wait: impl FnMut(Duration),
) -> Option<Result<T, DecodeError>> {
	loop {
		if !lifetime.load(Ordering::Acquire) {
			return None;
		}
		if Instant::now() >= deadline {
			return Some(Err(DecodeError::Busy));
		}
		match decode() {
			Err(DecodeError::Busy) => {
				let remaining = deadline.saturating_duration_since(Instant::now());
				if remaining.is_zero() {
					return Some(Err(DecodeError::Busy));
				}
				wait(remaining.min(Duration::from_millis(1)));
			}
			result => return Some(result),
		}
	}
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
fn record_decode_time(counters: &DecoderCounters, elapsed: std::time::Duration) {
	let value = elapsed.as_micros().min(100_000) as u64;
	let _ = counters
		.decode_us
		.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
			Some(value.max(old.saturating_mul(99) / 100))
		});
}
#[derive(Default)]
struct Timings {
	active: bool,
	pending: std::collections::VecDeque<(u64, Instant, Option<crate::Presentation>)>,
}
impl Timings {
	fn insert(&mut self, tag: u64, presentation: Option<crate::Presentation>) -> bool {
		let now = Instant::now();
		self.active = true;
		self.pending
			.retain(|(_, at, _)| now.saturating_duration_since(*at) < MAX_DECODE_AGE);
		if self.pending.len() == 16 {
			return false;
		}
		self.pending.push_back((tag, now, presentation));
		true
	}
	fn take(&mut self, tag: u64) -> Option<(Option<crate::Presentation>, std::time::Duration)> {
		if !self.active {
			return Some((None, std::time::Duration::ZERO));
		}
		let index = self.pending.iter().position(|(id, _, _)| *id == tag)?;
		let (_, at, stamp) = self.pending.remove(index)?;
		Some((stamp, at.elapsed()))
	}
}
enum Backend {
	Hardware(platform::video::live::H264Decoder),
	Software(openh264::decoder::Decoder),
}
impl Backend {
	#[cfg(all(test, any(target_os = "windows", target_os = "macos")))]
	fn new(
		prefer_hardware: bool,
		user: u64,
		sink: &VideoSink,
		counters: &Arc<DecoderCounters>,
		lifetime: &Arc<AtomicBool>,
	) -> Option<Self> {
		Self::new_timed(
			Codec::H264,
			prefer_hardware,
			user,
			sink,
			counters,
			lifetime,
			Arc::default(),
		)
	}
	fn new_timed(
		codec: Codec,
		prefer_hardware: bool,
		user: u64,
		sink: &VideoSink,
		counters: &Arc<DecoderCounters>,
		lifetime: &Arc<AtomicBool>,
		timings: Arc<Mutex<Timings>>,
	) -> Option<Self> {
		if prefer_hardware || codec == Codec::H265 {
			let sink = sink.clone();
			let counters = counters.clone();
			let lifetime = lifetime.clone();
			let deliver: platform::video::TimedLiveSink = Box::new(move |frame, tag| {
				let Some((presentation, elapsed)) = timings
					.lock()
					.ok()
					.and_then(|mut timings| timings.take(tag))
				else {
					return;
				};
				record_decode_time(&counters, elapsed);
				if lifetime.load(Ordering::Acquire)
					&& bounded(frame.width as usize, frame.height as usize).is_ok()
				{
					counters.sizes.record(frame.width, frame.height);
					counters.pictures.fetch_add(1, Ordering::Relaxed);
					sink(RemoteFrame {
						presentation,
						user,
						picture: frame.borrow(),
					});
				}
			});
			if codec == Codec::H265 {
				#[cfg(target_os = "linux")]
				return platform::video::live::H264Decoder::new_h265_timed(deliver)
					.ok()
					.map(Self::Hardware);
				#[cfg(not(target_os = "linux"))]
				return None;
			}
			if let Ok(decoder) = platform::video::live::H264Decoder::new_timed(deliver) {
				return Some(Self::Hardware(decoder));
			}
		}
		openh264::decoder::Decoder::new().ok().map(Self::Software)
	}
	/// Feed one access unit. Software pictures are returned; hardware ones were already
	/// delivered to the sink. `scratch` is reused so no frame-sized buffer is zeroed per frame.
	#[cfg(test)]
	fn decode(
		&mut self,
		data: &[u8],
		scratch: &mut Vec<u8>,
	) -> Result<Option<(u32, u32)>, DecodeError> {
		if let Self::Hardware(decoder) = self {
			return decoder.decode(data).map(|()| None).map_err(|error| {
				if error == platform::video::BUSY {
					DecodeError::Busy
				} else {
					DecodeError::Failed
				}
			});
		}
		self.decode_timed(data, scratch, 0)
	}
	fn decode_timed(
		&mut self,
		data: &[u8],
		scratch: &mut Vec<u8>,
		tag: u64,
	) -> Result<Option<(u32, u32)>, DecodeError> {
		match self {
			Self::Hardware(decoder) => {
				decoder
					.decode_timed(data, tag)
					.map(|()| None)
					.map_err(|error| {
						if error == platform::video::BUSY {
							DecodeError::Busy
						} else {
							DecodeError::Failed
						}
					})
			}
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

fn decode_loop(
	receive: Receiver<Decode>,
	sink: VideoSink,
	lost: Lost,
	counters: Arc<DecoderCounters>,
	prefer_hardware: bool,
) {
	let mut decoders: HashMap<u64, Backend> = HashMap::new();
	let mut codecs: HashMap<u64, Codec> = HashMap::new();
	let mut timings: HashMap<u64, Arc<Mutex<Timings>>> = HashMap::new();
	let mut serial = 0u64;
	let mut active: HashMap<u64, Arc<AtomicBool>> = HashMap::new();
	// Users whose hardware decoder rejected the stream fall back to software.
	let mut software_only: Vec<u64> = Vec::new();
	// After a decode error, predictions are skipped until a keyframe rebuilds the references.
	let mut broken: Vec<u64> = Vec::new();
	let mut scratch = Vec::new();
	while let Ok(message) = receive.recv() {
		let had_active = !active.is_empty();
		active.retain(|user, lifetime| {
			let retained = lifetime.load(Ordering::Acquire);
			if !retained {
				decoders.remove(user);
				codecs.remove(user);
				timings.remove(user);
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
		let (frame, _permit, queued, lifetime, presentation) = match message {
			Decode::Frame(frame, permit, queued, lifetime, presentation) => {
				(frame, permit, queued, lifetime, presentation)
			}
			Decode::Wake => continue,
			#[cfg(test)]
			Decode::Barrier(done) => {
				let _ = done.send(scratch.capacity());
				continue;
			}
		};
		if !lifetime.load(Ordering::Acquire) || frame.data.len() > MAX_FRAME_BYTES {
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
		if codecs
			.get(&frame.user)
			.is_some_and(|codec| *codec != frame.codec)
		{
			decoders.remove(&frame.user);
			timings.remove(&frame.user);
			software_only.retain(|user| *user != frame.user);
			mark_broken(frame.user, &mut broken, &lost);
		}
		if frame.keyframe {
			broken.retain(|user| *user != frame.user);
		} else if broken.contains(&frame.user) {
			continue;
		}
		if !frame.codec.bounded_parameters(&frame.data) {
			counters.errors.fetch_add(1, Ordering::Relaxed);
			decoders.remove(&frame.user);
			mark_broken(frame.user, &mut broken, &lost);
			continue;
		}
		if !decoders.contains_key(&frame.user) {
			if frame.codec == Codec::H265
				&& (!frame.keyframe || !frame.codec.has_parameter_sets(&frame.data))
			{
				mark_broken(frame.user, &mut broken, &lost);
				continue;
			}
			if decoders.len() >= MAX_DECODERS {
				continue;
			}
			let timing = Arc::new(Mutex::new(Timings::default()));
			let Some(decoder) = Backend::new_timed(
				frame.codec,
				prefer_hardware && !software_only.contains(&frame.user),
				frame.user,
				&sink,
				&counters,
				&lifetime,
				timing.clone(),
			) else {
				counters.errors.fetch_add(1, Ordering::Relaxed);
				continue;
			};
			decoders.insert(frame.user, decoder);
			codecs.insert(frame.user, frame.codec);
			timings.insert(frame.user, timing);
		}
		serial = serial.wrapping_add(1);
		let timing = timings.get(&frame.user).expect("decoder timing inserted");
		if !timing
			.lock()
			.is_ok_and(|mut timing| timing.insert(serial, presentation))
		{
			mark_broken(frame.user, &mut broken, &lost);
			continue;
		}
		let decoder = decoders.get_mut(&frame.user).expect("decoder inserted");
		let hardware = !matches!(decoder, Backend::Software(_));
		let Some(result) = decode_when_ready(
			queued + MAX_DECODE_AGE,
			&lifetime,
			|| decoder.decode_timed(&frame.data, &mut scratch, serial),
			std::thread::sleep,
		) else {
			continue;
		};
		let decoded = match result {
			Ok(Some(picture)) => picture,
			Ok(None) => {
				decoder_counts(&decoders, &counters);
				continue;
			}
			Err(DecodeError::Busy) => {
				// The retry budget expired: keep the decoder and its hardware path,
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
				if hardware
					&& frame.codec == Codec::H264
					&& frame.keyframe
					&& !software_only.contains(&frame.user)
				{
					software_only.push(frame.user);
				}
				mark_broken(frame.user, &mut broken, &lost);
				continue;
			}
		};
		decoder_counts(&decoders, &counters);
		let (width, height) = decoded;
		if !lifetime.load(Ordering::Acquire) {
			continue;
		}
		counters.sizes.record(width, height);
		counters.pictures.fetch_add(1, Ordering::Relaxed);
		let (presentation, elapsed) = timing
			.lock()
			.ok()
			.and_then(|mut timing| {
				let tag = timing.pending.front()?.0;
				timing.take(tag)
			})
			.unwrap_or((None, std::time::Duration::ZERO));
		record_decode_time(&counters, elapsed);
		sink(RemoteFrame {
			presentation,
			user: frame.user,
			picture: platform::video::Picture {
				width,
				height,
				layout: platform::video::Layout::I420,
				color: platform::video::Color::WEBRTC,
				rotation: 0,
				data: &scratch,
			},
		});
	}
	// Hardware pictures still in flight must land before the sink goes away.
	for decoder in decoders.values() {
		match decoder {
			Backend::Hardware(decoder) => decoder.flush(),
			Backend::Software(_) => {}
		}
	}
}

fn decoder_counts(decoders: &HashMap<u64, Backend>, counters: &DecoderCounters) {
	let hardware = decoders
		.values()
		.filter(|decoder| !matches!(decoder, Backend::Software(_)))
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
	fn native_backpressure_preserves_predictions_until_capacity_or_cancellation() {
		let live = AtomicBool::new(true);
		let mut calls = 0;
		let value = decode_when_ready(
			Instant::now() + MAX_DECODE_AGE,
			&live,
			|| {
				calls += 1;
				if calls < 3 {
					Err(DecodeError::Busy)
				} else {
					Ok(42)
				}
			},
			|_| {},
		);
		assert!(matches!(value, Some(Ok(42))));
		assert_eq!(calls, 3);
		assert!(matches!(
			decode_when_ready::<()>(
				Instant::now(),
				&live,
				|| panic!("expired frame submitted"),
				|_| {}
			),
			Some(Err(DecodeError::Busy))
		));
		let mut calls = 0;
		assert!(matches!(
			decode_when_ready(
				Instant::now() + MAX_DECODE_AGE,
				&live,
				|| {
					calls += 1;
					Err::<(), _>(DecodeError::Failed)
				},
				|_| {}
			),
			Some(Err(DecodeError::Failed))
		));
		assert_eq!(calls, 1, "decoder faults are not retried");
		assert!(
			decode_when_ready(
				Instant::now() + MAX_DECODE_AGE,
				&live,
				|| {
					live.store(false, Ordering::Release);
					Err::<(), _>(DecodeError::Busy)
				},
				|_| {}
			)
			.is_none()
		);
		assert!(
			decode_when_ready::<()>(
				Instant::now() + MAX_DECODE_AGE,
				&live,
				|| panic!("cancelled decode"),
				|_| {}
			)
			.is_none()
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
						codec: crate::video_codec::Codec::H264,
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
	fn decoder_cleanup_invalidates_full_queue_and_allows_restart() {
		let data = decoder_cleanup_keyframe(32, 32);
		for full in [true, false] {
			let (queue, receive) = decoder_cleanup_queue(3);
			for _ in 0..if full { 3 } else { 1 } {
				assert!(
					offer(
						&queue,
						Encoded {
							codec: crate::video_codec::Codec::H264,
							user: 7,
							data: data.clone(),
							keyframe: true
						}
					)
					.unwrap()
				);
			}
			remove(&queue, 7);
			if !full {
				assert!(
					offer(
						&queue,
						Encoded {
							codec: crate::video_codec::Codec::H264,
							user: 7,
							data: data.clone(),
							keyframe: true
						}
					)
					.unwrap()
				);
			}
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
					codec: crate::video_codec::Codec::H264,
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
						codec: crate::video_codec::Codec::H264,
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
					codec: crate::video_codec::Codec::H264,
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
				codec: crate::video_codec::Codec::H264,
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
					codec: crate::video_codec::Codec::H264,
					user: 7,
					data: vec![0],
					keyframe: true
				}
			)
			.unwrap()
		);
		let original = queue.active.lock().unwrap()[&7].clone();
		assert!(matches!(receive.try_recv(), Ok(Decode::Frame(..))));
		retain_sources(&queue, &receivers);
		assert!(matches!(
			receive.try_recv(),
			Err(std::sync::mpsc::TryRecvError::Empty)
		));
		// Reassigning one of a user's sources must not cancel their remaining source.
		receivers.announce(8, 700).unwrap();
		retain_sources(&queue, &receivers);
		assert!(original.load(Ordering::Acquire));
		assert!(matches!(
			receive.try_recv(),
			Err(std::sync::mpsc::TryRecvError::Empty)
		));
		// Reassigning the final SSRC must also release its previous owner's decoder.
		receivers.announce(8, 710).unwrap();
		retain_sources(&queue, &receivers);
		assert!(!original.load(Ordering::Acquire));
		assert!(!queue.active.lock().unwrap().contains_key(&7));
		assert!(matches!(receive.try_recv(), Ok(Decode::Wake)));
		assert!(
			offer(
				&queue,
				Encoded {
					codec: crate::video_codec::Codec::H264,
					user: 8,
					data: vec![0],
					keyframe: true
				}
			)
			.unwrap()
		);
		let current = queue.active.lock().unwrap()[&8].clone();
		receivers.announce(8, 0).unwrap();
		retain_sources(&queue, &receivers);
		assert!(!current.load(Ordering::Acquire));
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
	fn hevc_reassembly_rejects_invalid_fragment_headers_aggregation_and_pressure() {
		for malformed in [
			vec![],
			vec![38],
			vec![0x80, 1, 7],
			vec![38, 0, 7],
			vec![39, 1, 7],
			vec![96, 1, 0, 3, 38],
			vec![96, 1, 0, 2, 96, 1, 0, 2, 38, 1],
			vec![96, 1, 0, 2, 38, 1],
			vec![98, 1, 0xd3, 7],
			vec![98, 1, 0x93],
			vec![100, 1, 7],
		] {
			let mut assembler = Assembler {
				codec: Codec::H265,
				..Assembler::default()
			};
			assert!(assembler.push(1, 900, true, &malformed).is_none());
		}
		let mut assembler = Assembler {
			codec: Codec::H265,
			..Assembler::default()
		};
		assert!(assembler.push(1, 900, false, &[98, 1, 0x93, 7]).is_none());
		// Continuation type/temporal ID must agree with the starting fragment.
		assert!(assembler.push(2, 900, true, &[98, 2, 0x53, 7]).is_none());
		let mut assembler = Assembler {
			codec: Codec::H265,
			..Assembler::default()
		};
		let mut nal = vec![7; MAX_FRAME_BYTES];
		nal[..2].copy_from_slice(&[38, 1]);
		assert!(assembler.push(1, 900, true, &nal).is_none());
		assert!(assembler.frame.capacity() <= MAX_FRAME_BYTES);
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
				// Give RTX a bounded chance before breaking the reference chain.
				assert!(!receivers.awaiting());
				assert!(
					receivers
						.pop_at(Instant::now() + Duration::from_millis(121))
						.is_some()
				);
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
			assert!(
				receivers
					.pop_at(Instant::now() + Duration::from_millis(121))
					.is_some()
			);
			let stats = receivers.take_stats();
			assert_eq!(
				(stats.unknown_ssrc, stats.incomplete, stats.complete),
				(1, 1, 2)
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
	fn rtx_repairs_an_earlier_picture_and_releases_successors_without_a_keyframe() {
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		receivers.announce_rtx(700, 701).unwrap();
		assert!(receivers.push(700, 1, 900, true, &[0x65, 1]).is_some());
		assert!(receivers.accept(7, true));
		assert!(
			receivers
				.push(700, 2, 1800, false, &[0x7c, 0x81, 2])
				.is_none()
		);
		assert!(receivers.push(700, 4, 2700, true, &[0x41, 4]).is_none());
		assert!(!receivers.awaiting());
		let requests = receivers.nacks(99, Instant::now() + Duration::from_millis(20));
		assert_eq!(requests.len(), 1);
		assert_eq!(requests[0].1, [0, 0, 2, 188, 0, 3, 0, 0]);
		let mut rtx = vec![0, 3, 0x7c, 0x41, 3];
		let (ssrc, sequence) = receivers.restore_rtx(701, &mut rtx).unwrap();
		assert_eq!(
			receivers.push(ssrc, sequence, 1800, true, &rtx),
			Some((7, vec![0, 0, 0, 1, 0x61, 2, 3]))
		);
		assert!(receivers.accept(7, false));
		assert_eq!(receivers.pop(), Some((7, vec![0, 0, 0, 1, 0x41, 4])));
		assert!(receivers.accept(7, false));
		assert!(receivers.pop().is_none());
		assert!(
			receivers
				.nacks(99, Instant::now() + Duration::from_millis(60))
				.is_empty()
		);
		let stats = receivers.take_stats();
		assert_eq!(
			(stats.complete, stats.incomplete, stats.repaired),
			(3, 0, 1)
		);
		// Source retirement must cancel feedback and discard withheld media.
		assert!(receivers.push(700, 6, 3600, true, &[0x41, 6]).is_none());
		receivers.remove(7);
		assert!(receivers.pop().is_none());
		assert!(
			receivers
				.nacks(99, Instant::now() + Duration::from_millis(20))
				.is_empty()
		);
	}
	#[test]
	fn completed_pictures_retain_their_source_timestamp_after_cross_picture_repair() {
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		assert!(
			receivers
				.push_timed(700, 1, u32::MAX - 2999, false, &[0x7c, 0x85, 1])
				.is_none()
		);
		assert!(receivers.push_timed(700, 3, 0, true, &[0x41, 3]).is_none());
		let repaired = receivers
			.push_timed(700, 2, u32::MAX - 2999, true, &[0x7c, 0x45, 2])
			.unwrap();
		assert_eq!(
			(repaired.user, repaired.source, repaired.timestamp),
			(7, 700, u32::MAX - 2999)
		);
		assert_eq!(repaired.data, [0, 0, 0, 1, 0x65, 1, 2]);
		let next = receivers.pop_timed().unwrap();
		assert_eq!((next.user, next.source, next.timestamp), (7, 700, 0));
	}
	#[test]
	fn empty_keepalives_between_fragments_preserve_the_picture_and_reference_chain() {
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		assert!(
			receivers
				.push(700, 1, 900, false, &[0x7c, 0x85, 1])
				.is_none()
		);
		// Padding timestamps/markers must not close or replace the unfinished picture.
		assert!(receivers.push(700, 2, 0, true, &[]).is_none());
		assert_eq!(
			receivers.push(700, 3, 900, true, &[0x7c, 0x45, 2]),
			Some((7, vec![0, 0, 0, 1, 0x65, 1, 2]))
		);
		receivers.accept(7, true);
		assert!(receivers.push(700, 4, 0, false, &[]).is_none());
		assert!(receivers.push(700, 5, 1800, true, &[0x41, 3]).is_some());
		assert!(receivers.accept(7, false));
		let stats = receivers.take_stats();
		assert_eq!(
			(stats.incomplete, stats.complete, stats.padding_packets),
			(0, 2, 2)
		);
	}
	#[test]
	fn reordered_padding_repairs_a_sequence_hole_but_real_missing_media_still_needs_recovery() {
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		assert!(receivers.push(700, 65534, 900, true, &[0x65, 1]).is_some());
		receivers.accept(7, true);
		assert!(receivers.push(700, 0, 1800, true, &[0x41, 2]).is_none());
		assert!(receivers.push(700, 65535, 0, true, &[]).is_some());
		assert!(receivers.accept(7, false));
		let stats = receivers.take_stats();
		assert_eq!(
			(stats.incomplete, stats.repaired, stats.padding_packets),
			(0, 1, 1)
		);
		assert!(receivers.push(700, 2, 2700, true, &[0x41, 3]).is_none());
		assert!(
			receivers
				.pop_at(Instant::now() + Duration::from_millis(121))
				.is_some()
		);
		assert!(!receivers.accept(7, false));
		assert_eq!(receivers.take_stats().incomplete, 1);
	}
	#[test]
	fn long_padding_runs_do_not_hide_or_invent_media_activity() {
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		receivers.announce(7, 710).unwrap();
		assert!(receivers.push(700, 1, 900, true, &[0x65, 1]).is_some());
		assert!(receivers.push(710, 1, 900, true, &[0x65, 2]).is_some());
		for sequence in 2..=32770 {
			assert!(receivers.push(700, sequence, 0, false, &[]).is_none());
		}
		assert_eq!(receivers.keyframe_requests().collect::<Vec<_>>(), [710]);
		assert!(receivers.push(700, 32771, 1800, true, &[0x65, 3]).is_some());
		assert_eq!(receivers.keyframe_requests().collect::<Vec<_>>(), [700]);
		assert_eq!(receivers.take_stats().incomplete, 0);
	}
	#[test]
	fn keyframe_cooldowns_begin_with_real_requests_and_are_independent_per_source() {
		let now = Instant::now();
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		assert!(receivers.accept(7, true));
		for ms in [0, 200, 400] {
			assert!(
				receivers
					.take_keyframe_requests(now + Duration::from_millis(ms))
					.is_empty()
			);
		}
		receivers.require_keyframe(7);
		let first = now + Duration::from_millis(490);
		assert_eq!(receivers.take_keyframe_requests(first), [700]);
		assert!(
			receivers
				.take_keyframe_requests(first + Duration::from_millis(499))
				.is_empty()
		);
		// A second sender's first request does not inherit the first sender's cooldown.
		receivers.announce(8, 800).unwrap();
		assert_eq!(
			receivers.take_keyframe_requests(first + Duration::from_millis(10)),
			[800]
		);
		assert_eq!(
			receivers.take_keyframe_requests(first + Duration::from_millis(500)),
			[700]
		);
		receivers.accept(7, true);
		assert_eq!(
			receivers.take_keyframe_requests(first + Duration::from_millis(510)),
			[800]
		);
		receivers.remove(8);
		assert!(
			receivers
				.take_keyframe_requests(first + Duration::from_secs(1))
				.is_empty()
		);
	}
	#[test]
	fn recovery_targets_forward_media_activity_instead_of_idle_layers_or_old_rtx() {
		let mut receivers = Receivers::default();
		receivers.announce(7, 700).unwrap();
		receivers.announce(7, 710).unwrap();
		assert_eq!(receivers.keyframe_requests().collect::<Vec<_>>(), [700]);
		assert!(receivers.push(700, 1, 900, true, &[0x65, 1]).is_some());
		receivers.accept(7, true);
		assert!(receivers.push(710, 5, 900, true, &[0x65, 2]).is_some());
		assert!(receivers.push(700, 1, 900, true, &[0x65, 1]).is_none()); // Old duplicate.
		let lost: Lost = Arc::new(Mutex::new(vec![7]));
		receivers.absorb(&lost);
		assert!(lost.lock().unwrap().is_empty());
		assert_eq!(receivers.keyframe_requests().collect::<Vec<_>>(), [710]);
		let now = Instant::now();
		assert_eq!(receivers.take_keyframe_requests(now), [710]);
		// Pruning the active layer also retires its feedback/cooldown state.
		receivers.retain_user_sources(7, &[700]);
		assert_eq!(receivers.take_keyframe_requests(now), [700]);
		receivers.remove(7);
		assert!(!receivers.awaiting());
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
			assert!(decoder.decode(data, &mut grown).is_ok());
		}
		let bytes = platform::video::Layout::I420.len(64, 64).unwrap();
		assert!(grown.capacity() >= bytes);
		let mut scratch = Vec::new();
		let mut allocation = None;
		for _ in 0..4 {
			for (decoder, data, dimensions) in &mut streams {
				assert_eq!(decoder.decode(data, &mut scratch), Ok(Some(*dimensions)));
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
					let (width, height) = decoder.decode(data, &mut scratch).unwrap().unwrap();
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
			seen.lock().unwrap().push((
				frame.user,
				frame.picture.width,
				frame.picture.height,
				frame.picture.to_rgba().unwrap().2,
			));
		});
		let counters = Arc::new(DecoderCounters::default());
		let lifetime = Arc::new(AtomicBool::new(true));
		let mut decoder =
			Backend::new(true, 9, &sink, &counters, &lifetime).expect("hardware backend");
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
		let delivered = {
			let pictures = pictures.lock().unwrap();
			assert!(!pictures.is_empty(), "VideoToolbox produced pictures");
			let frame = &pictures[0];
			assert_eq!((frame.0, frame.1, frame.2), (9, 320, 240));
			assert_eq!(frame.3.len(), 320 * 240 * 4);
			assert!(frame.3.as_chunks::<4>().0.iter().all(|px| px[3] == 255));
			pictures.len() as u64
		};
		assert_eq!(counters.pictures.load(Ordering::Relaxed), delivered);
		lifetime.store(false, Ordering::Release);
		let mut data = Vec::new();
		encoder.encode(&yuv).unwrap().write_vec(&mut data);
		decoder.decode(&data, &mut scratch).unwrap();
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
				&Arc::new(AtomicBool::new(true)),
			)
			.unwrap();
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
					codec: crate::video_codec::Codec::H264,
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
