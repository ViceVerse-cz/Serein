//! Explicit, memory-only MP3/WAV/Ogg playback. One lazy worker, one replaceable request.
#[path = "audio/source.rs"]
mod source;
#[path = "audio/streaming.rs"]
mod streaming;
use model::Attachment;
#[cfg(any(test, all(debug_assertions, feature = "demo")))]
use std::io::Cursor;
use std::{
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
	},
	time::Duration,
};
use symphonia::core::{
	codecs::audio::{AudioDecoderOptions, well_known::CODEC_ID_OPUS},
	common::Limit,
	formats::{FormatOptions, TrackType, probe::Hint},
	io::{MediaSource, MediaSourceStream},
	meta::MetadataOptions,
};
use tokio::sync::{Notify, watch};

const MAX_ENCODED: usize = 20 * 1024 * 1024;
const MAX_SAMPLES: usize = 64 * 1024 * 1024 / size_of::<f32>();
const MAX_SECONDS: u64 = 600;
const NO_SEEK: u64 = u64::MAX;
const INVALID: &str = "Unsupported or damaged audio; download to play externally";
const TOO_LARGE: &str = "Audio preview limit: 20 MiB file, 64 MiB decoded, 10 minutes";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum State {
	#[default]
	Idle,
	Loading,
	Playing,
	Paused,
	Ended,
	Failed(&'static str),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
	pub state: State,
	pub position: Duration,
	pub duration: Duration,
}

struct Gate {
	generation: AtomicU64,
	paused: AtomicBool,
	volume: AtomicU32,
	seek_millis: AtomicU64,
	position_frames: AtomicU64,
	failed: AtomicBool,
	buffering: AtomicBool,
	sample_rate: AtomicU32,
}
impl Default for Gate {
	fn default() -> Self {
		Self {
			generation: AtomicU64::new(0),
			paused: AtomicBool::new(false),
			volume: AtomicU32::new(1.0f32.to_bits()),
			seek_millis: AtomicU64::new(NO_SEEK),
			position_frames: AtomicU64::new(0),
			failed: AtomicBool::new(false),
			buffering: AtomicBool::new(true),
			sample_rate: AtomicU32::new(0),
		}
	}
}
impl Gate {
	fn current(&self, generation: u64) -> bool {
		self.generation.load(Ordering::Acquire) == generation
	}
}
#[derive(Clone)]
struct Request {
	generation: u64,
	url: Option<url::Url>,
	expected: usize,
	#[cfg(feature = "demo")]
	voice_message: bool,
	duration: Duration,
}
struct Worker {
	requests: watch::Sender<Option<Request>>,
	status: watch::Receiver<(u64, Status)>,
	wake: Arc<Notify>,
}
#[derive(Default)]
pub struct Audio {
	gate: Arc<Gate>,
	worker: Option<Worker>,
	status: Status,
}
impl Audio {
	pub fn start(
		&mut self,
		attachment: Attachment,
		runtime: &tokio::runtime::Handle,
		context: &eframe::egui::Context,
		demo: bool,
	) -> Result<(), &'static str> {
		self.stop();
		let result = self.start_inner(attachment, runtime, context, demo);
		if let Err(error) = result {
			self.status.state = State::Failed(error);
		}
		result
	}
	fn start_inner(
		&mut self,
		attachment: Attachment,
		runtime: &tokio::runtime::Handle,
		context: &eframe::egui::Context,
		demo: bool,
	) -> Result<(), &'static str> {
		if attachment.size == 0 || attachment.size > MAX_ENCODED as u64 {
			return Err(TOO_LARGE);
		}
		let url = if demo {
			None
		} else {
			Some(
				crate::downloads::original_url(&attachment)
					.ok_or("Audio attachment unavailable")?,
			)
		};
		if self.worker.is_none() {
			let (requests, receiver) = watch::channel(None);
			let (status, updates) = watch::channel((0, Status::default()));
			let wake = Arc::new(Notify::new());
			let gate = self.gate.clone();
			let worker_wake = wake.clone();
			let runtime = runtime.clone();
			let context = context.clone();
			std::thread::Builder::new()
				.name("serein-attachment-audio".into())
				.spawn(move || {
					worker(receiver, status, gate, worker_wake, runtime, context);
				})
				.map_err(|_| "Could not start audio worker")?;
			self.worker = Some(Worker {
				requests,
				status: updates,
				wake,
			});
		}
		let generation = self.gate.generation.fetch_add(1, Ordering::AcqRel) + 1;
		self.gate.paused.store(false, Ordering::Release);
		self.gate.seek_millis.store(NO_SEEK, Ordering::Release);
		self.gate.sample_rate.store(0, Ordering::Release);
		self.status = Status {
			state: State::Loading,
			..Default::default()
		};
		let worker = self.worker.as_ref().expect("worker created");
		if worker.requests.is_closed() {
			self.status.state = State::Failed("Audio worker stopped; restart Serein");
			return Err("Audio worker stopped; restart Serein");
		}
		worker.requests.send_replace(Some(Request {
			generation,
			url,
			expected: attachment.size as usize,
			#[cfg(feature = "demo")]
			voice_message: attachment.is_voice_message(),
			duration: Duration::from_millis(u64::from(attachment.duration_ms.unwrap_or(0))),
		}));
		worker.wake.notify_one();
		Ok(())
	}
	pub fn poll(&mut self) -> Status {
		if let Some(worker) = &mut self.worker {
			let (generation, status) = worker.status.borrow_and_update().clone();
			if self.gate.current(generation) {
				self.status = status;
			}
		}
		let mut status = self.status.clone();
		let rate = self.gate.sample_rate.load(Ordering::Acquire);
		if rate > 0
			&& matches!(
				status.state,
				State::Playing | State::Paused | State::Loading
			) {
			status.position = Duration::from_secs_f64(
				self.gate.position_frames.load(Ordering::Acquire) as f64 / f64::from(rate),
			);
			status.state = if self.gate.paused.load(Ordering::Acquire) {
				State::Paused
			} else if self.gate.buffering.load(Ordering::Acquire) {
				State::Loading
			} else {
				State::Playing
			};
		}
		status
	}
	pub fn pause(&mut self, paused: bool) {
		self.gate.paused.store(paused, Ordering::Release);
		if let Some(worker) = &self.worker {
			worker.wake.notify_one();
		}
	}
	pub fn seek(&mut self, position: Duration) {
		self.gate.seek_millis.store(
			position.min(Duration::from_secs(MAX_SECONDS)).as_millis() as u64,
			Ordering::Release,
		);
		if let Some(worker) = &self.worker {
			worker.wake.notify_one();
		}
	}
	pub fn volume(&mut self, volume: f32) {
		if volume.is_finite() {
			self.gate
				.volume
				.store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Release);
		}
	}
	pub fn stop(&mut self) {
		if self.status.state == State::Idle {
			return;
		}
		self.gate.generation.fetch_add(1, Ordering::AcqRel);
		if let Some(worker) = &self.worker {
			worker.requests.send_replace(None);
			worker.wake.notify_one();
		}
		self.status = Status::default();
	}
}
impl Drop for Audio {
	fn drop(&mut self) {
		self.stop();
	}
}

fn worker(
	mut requests: watch::Receiver<Option<Request>>,
	status: watch::Sender<(u64, Status)>,
	gate: Arc<Gate>,
	wake: Arc<Notify>,
	runtime: tokio::runtime::Handle,
	context: eframe::egui::Context,
) {
	while runtime.block_on(requests.changed()).is_ok() {
		let request = requests.borrow_and_update().clone();
		let Some(request) = request else {
			continue;
		};
		let publish = |value| {
			if gate.current(request.generation)
				&& status.send_if_modified(|current| {
					if current.0 == request.generation && current.1 == value {
						return false;
					}
					*current = (request.generation, value);
					true
				}) {
				context.request_repaint();
			}
		};
		let result = streaming::play(&request, &gate, &wake, &runtime, &publish);
		if let Err(error) = result {
			publish(Status {
				state: State::Failed(error),
				..Default::default()
			});
		}
	}
}

#[cfg(test)]
async fn fetch(
	url: url::Url,
	expected: usize,
	gate: &Gate,
	generation: u64,
	wake: &Notify,
) -> Result<Vec<u8>, &'static str> {
	if expected == 0 || expected > MAX_ENCODED {
		return Err(TOO_LARGE);
	}
	let client = reqwest::Client::builder()
		.no_proxy()
		.redirect(reqwest::redirect::Policy::none())
		.timeout(Duration::from_secs(60))
		.build()
		.map_err(|_| "Audio download unavailable")?;
	let transfer = async {
		let mut response = client
			.get(url)
			.header(reqwest::header::ACCEPT_ENCODING, "identity")
			.send()
			.await
			.map_err(|_| "Audio download failed")?;
		if response.status() != reqwest::StatusCode::OK {
			return Err("Audio unavailable; reload the conversation");
		}
		if response
			.headers()
			.get(reqwest::header::CONTENT_ENCODING)
			.is_some_and(|v| v != "identity")
			|| response
				.content_length()
				.is_some_and(|size| size != expected as u64)
		{
			return Err("Audio size or encoding changed; reload the conversation");
		}
		let mut bytes = Vec::with_capacity(expected);
		while let Some(chunk) = response
			.chunk()
			.await
			.map_err(|_| "Audio download interrupted")?
		{
			if !gate.current(generation) {
				return Err("Cancelled");
			}
			if chunk.len() > expected - bytes.len() {
				return Err(TOO_LARGE);
			}
			bytes.extend_from_slice(&chunk);
		}
		if bytes.len() != expected {
			return Err("Audio download incomplete");
		}
		Ok(bytes)
	};
	let cancelled = async {
		while gate.current(generation) {
			wake.notified().await;
		}
	};
	tokio::select! { biased;
		_ = cancelled => Err("Cancelled"),
		result = transfer => result,
	}
}

#[cfg(any(test, all(debug_assertions, feature = "demo")))]
struct Pcm {
	samples: Vec<f32>,
	channels: usize,
	rate: u32,
}

#[cfg(any(test, all(debug_assertions, feature = "demo")))]
fn decode(mut bytes: Vec<u8>, current: &impl Fn() -> bool) -> Result<Pcm, &'static str> {
	if !current() {
		return Err("Cancelled");
	}
	if bytes.is_empty() || bytes.len() > MAX_ENCODED {
		return Err(TOO_LARGE);
	}
	if bytes.starts_with(b"OggS") {
		check_ogg_headers(&bytes, current)?;
	}
	prepare_media(&mut bytes)?;
	let mut pcm = Pcm {
		samples: Vec::new(),
		channels: 0,
		rate: 0,
	};
	decode_stream(
		Box::new(Cursor::new(bytes)),
		current,
		&mut |samples, channels, rate, _| {
			pcm.channels = channels;
			pcm.rate = rate;
			pcm.samples.extend(samples.iter().map(|sample| {
				if sample.is_finite() {
					sample.clamp(-1.0, 1.0)
				} else {
					0.0
				}
			}));
			Ok(())
		},
	)?;
	Ok(pcm)
}

pub(super) fn decode_stream(
	source: Box<dyn MediaSource>,
	current: &impl Fn() -> bool,
	emit: &mut impl FnMut(&[f32], usize, u32, Option<Duration>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
	let source = MediaSourceStream::new(source, Default::default());
	let metadata = MetadataOptions::default()
		.limit_tag_bytes(Limit::Maximum(0))
		.limit_visual_bytes(Limit::Maximum(0));
	let mut format = symphonia::default::get_probe()
		.probe(&Hint::new(), source, FormatOptions::default(), metadata)
		.map_err(|_| INVALID)?;
	let track = format.default_track(TrackType::Audio).ok_or(INVALID)?;
	let parameters = track
		.codec_params
		.as_ref()
		.and_then(|p| p.audio())
		.ok_or(INVALID)?;
	let rate = parameters.sample_rate.ok_or(INVALID)?;
	let channels = parameters.channels.as_ref().ok_or(INVALID)?.count();
	if !(8000..=96000).contains(&rate)
		|| !(1..=2).contains(&channels)
		|| parameters
			.max_frames_per_packet
			.is_some_and(|frames| frames > 65536)
		|| parameters
			.frames_per_block
			.is_some_and(|frames| frames > 65536)
	{
		return Err(INVALID);
	}
	let max_samples = MAX_SAMPLES.min(rate as usize * channels * MAX_SECONDS as usize);
	if track
		.num_frames
		.is_some_and(|frames| frames > (max_samples / channels) as u64)
	{
		return Err(TOO_LARGE);
	}
	let duration = track
		.num_frames
		.map(|frames| Duration::from_secs_f64(frames as f64 / f64::from(rate)));
	let track_id = track.id;
	let mut opus = if parameters.codec == CODEC_ID_OPUS {
		let header = parameters.extra_data.as_deref().ok_or(INVALID)?;
		// Family 0 is the mono/stereo voice-message mapping. Reject multistream mappings.
		if header.len() < 19
			|| &header[..8] != b"OpusHead"
			|| header[8] > 15
			|| usize::from(header[9]) != channels
			|| header[18] != 0
			|| rate != 48000
		{
			return Err(INVALID);
		}
		let mut decoder = opus2::Decoder::new(
			rate,
			if channels == 1 {
				opus2::Channels::Mono
			} else {
				opus2::Channels::Stereo
			},
		)
		.map_err(|_| INVALID)?;
		decoder
			.set_gain(i32::from(i16::from_le_bytes([header[16], header[17]])))
			.map_err(|_| INVALID)?;
		Some((
			decoder,
			usize::from(u16::from_le_bytes([header[10], header[11]])),
		))
	} else {
		None
	};
	let mut decoder = if opus.is_none() {
		Some(
			symphonia::default::get_codecs()
				.make_audio_decoder(parameters, &AudioDecoderOptions::default())
				.map_err(|_| INVALID)?,
		)
	} else {
		None
	};
	let mut buffer = Vec::new();
	let mut total_samples = 0;
	let mut packets = 0;
	loop {
		if !current() {
			return Err("Cancelled");
		}
		let Some(packet) = format.next_packet().map_err(|_| INVALID)? else {
			break;
		};
		packets += 1;
		if packets > 100_000 || packet.data.len() > 1024 * 1024 {
			return Err(TOO_LARGE);
		}
		if packet.track_id != track_id {
			return Err(INVALID);
		}
		let decoded = if let Some((decoder, pre_skip)) = &mut opus {
			// Opus packets decode to at most 120 ms at 48 kHz.
			buffer.resize(5760 * channels, 0.0);
			if packet.data.is_empty() {
				return Err(INVALID);
			}
			let frames = decoder
				.decode_float(&packet.data, &mut buffer, false)
				.map_err(|_| INVALID)?;
			// Symphonia 0.6.1 exposes Opus pre-skip in OpusHead, but only applies end trim.
			let skip = (*pre_skip).min(frames);
			*pre_skip -= skip;
			let start = usize::try_from(packet.trim_start.get())
				.map_err(|_| INVALID)?
				.max(skip);
			let end = frames
				.checked_sub(usize::try_from(packet.trim_end.get()).map_err(|_| INVALID)?)
				.filter(|end| start <= *end)
				.ok_or(INVALID)?;
			&buffer[start * channels..end * channels]
		} else {
			let decoded = decoder
				.as_mut()
				.ok_or(INVALID)?
				.decode(&packet)
				.map_err(|_| INVALID)?;
			if decoded.spec().rate() != rate
				|| decoded.spec().channels().count() != channels
				|| decoded.frames() > 65536
			{
				return Err(INVALID);
			}
			buffer.resize(decoded.samples_interleaved(), 0.0);
			decoded.copy_to_slice_interleaved(&mut buffer);
			&buffer[..]
		};
		let count = decoded.len();
		if count > max_samples - total_samples {
			return Err(TOO_LARGE);
		}
		total_samples += count;
		if !decoded.is_empty() {
			emit(decoded, channels, rate, duration)?;
		}
	}
	if total_samples == 0 {
		return Err(INVALID);
	}
	Ok(())
}

// Skip metadata without decoding attacker-provided tag lengths/artwork. Compact in-place:
// only bounded PCM fmt/data chunks reach the WAV demuxer; ID3 parsing is disabled entirely.
fn prepare_media(bytes: &mut Vec<u8>) -> Result<(), &'static str> {
	if bytes.starts_with(b"OggS") {
		// Preflight bounded the headers; Symphonia handles demuxing and CRC verification.
		return Ok(());
	} else if bytes.starts_with(b"RIFF") {
		if bytes.get(8..12) != Some(b"WAVE") {
			return Err(INVALID);
		}
		let size = u32::from_le_bytes(
			bytes
				.get(4..8)
				.ok_or(INVALID)?
				.try_into()
				.map_err(|_| INVALID)?,
		) as usize;
		let end = size
			.checked_add(8)
			.filter(|end| *end <= bytes.len())
			.ok_or(INVALID)?;
		let mut read = 12;
		let mut write = 12;
		let mut have_format = false;
		let mut have_data = false;
		let mut chunks = 0;
		while read < end {
			chunks += 1;
			if chunks > 1024 || end - read < 8 {
				return Err(INVALID);
			}
			let len = u32::from_le_bytes(bytes[read + 4..read + 8].try_into().map_err(|_| INVALID)?)
				as usize;
			let next = read
				.checked_add(8 + len + len % 2)
				.filter(|next| *next <= end)
				.ok_or(INVALID)?;
			let kind = &bytes[read..read + 4];
			let keep = if kind == b"fmt " {
				if have_format || have_data || !(16..=40).contains(&len) {
					return Err(INVALID);
				}
				let encoding = u16::from_le_bytes([bytes[read + 8], bytes[read + 9]]);
				if !matches!(encoding, 1 | 3 | 0xfffe) {
					return Err(INVALID);
				}
				let channels = u16::from_le_bytes([bytes[read + 10], bytes[read + 11]]);
				let rate = u32::from_le_bytes(
					bytes[read + 12..read + 16]
						.try_into()
						.map_err(|_| INVALID)?,
				);
				let byte_rate = u32::from_le_bytes(
					bytes[read + 16..read + 20]
						.try_into()
						.map_err(|_| INVALID)?,
				);
				let alignment = u16::from_le_bytes([bytes[read + 20], bytes[read + 21]]);
				let bits = u16::from_le_bytes([bytes[read + 22], bytes[read + 23]]);
				// Validate before the demuxer: its rate/alignment arithmetic assumes sane headers.
				if !(1..=2).contains(&channels)
					|| !(8000..=96000).contains(&rate)
					|| !matches!(bits, 8 | 16 | 24 | 32 | 64)
					|| alignment != channels * (bits / 8)
					|| byte_rate != rate * u32::from(alignment)
					|| (encoding == 1 && bits > 32)
					|| (encoding == 3 && !matches!(bits, 32 | 64))
				{
					return Err(INVALID);
				}
				if encoding == 0xfffe {
					if len != 40 || bytes[read + 24..read + 26] != [22, 0] {
						return Err(INVALID);
					}
					let valid_bits = u16::from_le_bytes([bytes[read + 26], bytes[read + 27]]);
					let mask = u32::from_le_bytes(
						bytes[read + 28..read + 32]
							.try_into()
							.map_err(|_| INVALID)?,
					);
					let subtype = u32::from_le_bytes(
						bytes[read + 32..read + 36]
							.try_into()
							.map_err(|_| INVALID)?,
					);
					if valid_bits == 0
						|| valid_bits > bits
						|| (mask != 0 && mask.count_ones() != u32::from(channels))
						|| !matches!(subtype, 1 | 3)
						|| (subtype == 1 && bits > 32)
						|| (subtype == 3 && !matches!(bits, 32 | 64))
						|| bytes[read + 36..read + 48]
							!= [0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]
					{
						return Err(INVALID);
					}
				}
				have_format = true;
				true
			} else if kind == b"data" {
				if !have_format || have_data || len == 0 {
					return Err(INVALID);
				}
				have_data = true;
				true
			} else {
				false
			};
			if keep {
				bytes.copy_within(read..next, write);
				write += next - read;
			}
			read = next;
		}
		if !have_data {
			return Err(INVALID);
		}
		bytes.truncate(write);
		bytes[4..8].copy_from_slice(&((write - 8) as u32).to_le_bytes());
	} else {
		let mut offset = 0;
		for _ in 0..16 {
			if bytes.get(offset..offset + 3) != Some(b"ID3") {
				break;
			}
			let header = bytes.get(offset..offset + 10).ok_or(INVALID)?;
			if header[6..10].iter().any(|byte| byte & 0x80 != 0) {
				return Err(INVALID);
			}
			let len = header[6..10]
				.iter()
				.fold(0usize, |size, byte| (size << 7) | *byte as usize);
			let footer = usize::from(header[3] == 4 && header[5] & 0x10 != 0) * 10;
			offset = offset
				.checked_add(10 + len + footer)
				.filter(|end| *end <= bytes.len())
				.ok_or(INVALID)?;
		}
		if bytes.get(offset).copied() != Some(0xff)
			|| bytes.get(offset + 1).is_none_or(|byte| byte & 0xe0 != 0xe0)
		{
			return Err(INVALID);
		}
		bytes.copy_within(offset.., 0);
		bytes.truncate(bytes.len() - offset);
	}
	Ok(())
}

// Symphonia's Ogg comments do not honor MetadataOptions. Bound header allocation before probing;
// this only checks page framing/header sizes, leaving CRCs and stream decoding to Symphonia.
#[cfg(any(test, all(debug_assertions, feature = "demo")))]
fn check_ogg_headers(bytes: &[u8], current: &impl Fn() -> bool) -> Result<(), &'static str> {
	let mut offset = 0;
	let mut serial = None;
	let mut packet = Vec::new();
	let mut packet_len = 0usize;
	let mut packets = 0;
	let mut headers = 3;
	let mut ended = false;
	while offset < bytes.len() {
		if !current() {
			return Err("Cancelled");
		}
		let page = bytes.get(offset..offset + 27).ok_or(INVALID)?;
		if &page[..4] != b"OggS" || page[4] != 0 || ended {
			return Err(INVALID);
		}
		let page_serial = u32::from_le_bytes(page[14..18].try_into().map_err(|_| INVALID)?);
		if serial.is_some_and(|serial| serial != page_serial)
			|| (offset > 0 && page[5] & 2 != 0)
			|| (offset == 0 && page[5] & 2 == 0)
			|| (page[5] & 1 != 0) != (packet_len > 0)
		{
			return Err(INVALID);
		}
		serial = Some(page_serial);
		ended = page[5] & 4 != 0;
		let lacing = bytes
			.get(offset + 27..offset + 27 + usize::from(page[26]))
			.ok_or(INVALID)?;
		offset += 27 + lacing.len();
		for &lace in lacing {
			let len = usize::from(lace);
			let data = bytes.get(offset..offset + len).ok_or(INVALID)?;
			offset += len;
			packet_len += len;
			if packet_len
				> if packets < headers {
					64 * 1024
				} else {
					1024 * 1024
				} {
				return Err(TOO_LARGE);
			}
			if packets < headers {
				packet.extend_from_slice(data);
			}
			if lace < 255 {
				if packets == 0 {
					headers = if packet.starts_with(b"OpusHead") {
						2
					} else if packet.starts_with(b"\x01vorbis") {
						3
					} else {
						return Err(INVALID);
					};
				} else if packets == 1 {
					let comments = packet
						.strip_prefix(b"OpusTags")
						.or_else(|| packet.strip_prefix(b"\x03vorbis"))
						.ok_or(INVALID)?;
					let read_len = |offset: usize| -> Result<usize, &'static str> {
						Ok(u32::from_le_bytes(
							comments
								.get(offset..offset + 4)
								.ok_or(INVALID)?
								.try_into()
								.map_err(|_| INVALID)?,
						) as usize)
					};
					let vendor_len = read_len(0)?;
					if vendor_len > comments.len().saturating_sub(4) {
						return Err(INVALID);
					}
					let count = read_len(4 + vendor_len)?;
					if count > 128 {
						return Err(TOO_LARGE);
					}
					let mut cursor = 8 + vendor_len;
					for _ in 0..count {
						let len = read_len(cursor)?;
						cursor = cursor
							.checked_add(4 + len)
							.filter(|end| *end <= comments.len())
							.ok_or(INVALID)?;
					}
				}
				packets += 1;
				if packets > 100_000 {
					return Err(TOO_LARGE);
				}
				packet.clear();
				packet_len = 0;
			}
		}
	}
	if !ended || packet_len != 0 || packets <= headers {
		return Err(INVALID);
	}
	Ok(())
}

#[cfg(any(test, feature = "demo"))]
fn demo_wav() -> Vec<u8> {
	let rate = 24000u32;
	let frames = rate * 12;
	let len = frames * 2;
	let mut bytes = Vec::with_capacity(44 + len as usize);
	bytes.extend_from_slice(b"RIFF");
	bytes.extend_from_slice(&(36 + len).to_le_bytes());
	bytes.extend_from_slice(b"WAVEfmt ");
	bytes.extend_from_slice(&16u32.to_le_bytes());
	bytes.extend_from_slice(&1u16.to_le_bytes());
	bytes.extend_from_slice(&1u16.to_le_bytes());
	bytes.extend_from_slice(&rate.to_le_bytes());
	bytes.extend_from_slice(&(rate * 2).to_le_bytes());
	bytes.extend_from_slice(&2u16.to_le_bytes());
	bytes.extend_from_slice(&16u16.to_le_bytes());
	bytes.extend_from_slice(b"data");
	bytes.extend_from_slice(&len.to_le_bytes());
	for frame in 0..frames {
		let sample =
			((frame as f32 * std::f32::consts::TAU * 440.0 / rate as f32).sin() * 1600.0) as i16;
		bytes.extend_from_slice(&sample.to_le_bytes());
	}
	bytes
}

#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_voice_message_check() {
	source::debug_check();
	streaming::debug_check();
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.unwrap();
	for voice_message in [false, true] {
		let request = Request {
			generation: 0,
			url: None,
			expected: 1,
			voice_message,
			duration: Duration::ZERO,
		};
		let make_source = || {
			source::source(
				&request,
				Arc::new(Gate::default()),
				Arc::new(Notify::new()),
				runtime.handle().clone(),
			)
			.unwrap()
		};
		let mut frames = 0;
		decode_stream(make_source(), &|| true, &mut |samples, channels, _, _| {
			frames += samples.len() / channels;
			Ok(())
		})
		.expect("forward-only source decodes");
		assert_eq!(frames, if voice_message { 144000 } else { 288000 });
		let mut first_frames = 0;
		assert_eq!(
			decode_stream(make_source(), &|| true, &mut |samples, channels, _, _| {
				first_frames = samples.len() / channels;
				Err("stop after first packet")
			}),
			Err("stop after first packet")
		);
		assert!(
			first_frames > 0 && first_frames < frames,
			"playback receives PCM before full decoding"
		);
	}
	let bytes = include_bytes!("../tests/fixtures/voice-message.ogg");
	let pcm = decode(bytes.to_vec(), &|| true).expect("synthetic Ogg/Opus decodes");
	assert_eq!(
		(pcm.rate, pcm.channels, pcm.samples.len()),
		(48000, 1, 144000)
	);
	assert!(pcm.samples.iter().any(|sample| sample.abs() > 0.01));
	assert!(decode(bytes.to_vec(), &|| false).is_err());
	assert!(decode(b"OggS\0".to_vec(), &|| true).is_err());
	let mut oversized_comments = bytes.to_vec();
	let tags = bytes
		.windows(8)
		.position(|window| window == b"OpusTags")
		.unwrap();
	let vendor = u32::from_le_bytes(bytes[tags + 8..tags + 12].try_into().unwrap()) as usize;
	oversized_comments[tags + 12 + vendor..tags + 16 + vendor]
		.copy_from_slice(&129u32.to_le_bytes());
	assert_eq!(
		check_ogg_headers(&oversized_comments, &|| true),
		Err(TOO_LARGE)
	);
	assert!(check_ogg_headers(&[bytes.as_slice(), bytes.as_slice()].concat(), &|| true).is_err());
}

#[cfg(test)]
struct Playback {
	pcm: Pcm,
	frame: f64,
	output_rate: u32,
}
#[cfg(test)]
impl Playback {
	fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(
		&mut self,
		data: &mut [T],
		channels: usize,
		gate: &Gate,
		generation: u64,
	) {
		data.fill(T::from_sample(0.0));
		if !gate.current(generation) {
			return;
		}
		let frames = self.pcm.samples.len() / self.pcm.channels;
		let seek = gate.seek_millis.swap(NO_SEEK, Ordering::AcqRel);
		if seek != NO_SEEK {
			self.frame = (seek as f64 * self.pcm.rate as f64 / 1000.0).min(frames as f64);
		}
		if !gate.paused.load(Ordering::Acquire) {
			let volume = f32::from_bits(gate.volume.load(Ordering::Relaxed));
			// ponytail: linear rate conversion; use a band-limited resampler if quality measurements require it.
			for output in data.chunks_exact_mut(channels) {
				if self.frame >= frames as f64 {
					break;
				}
				let frame = self.frame as usize;
				let next = (frame + 1).min(frames - 1);
				let fraction = (self.frame - frame as f64) as f32;
				let source = |channel: usize| {
					let a = self.pcm.samples[frame * self.pcm.channels + channel];
					let b = self.pcm.samples[next * self.pcm.channels + channel];
					a + (b - a) * fraction
				};
				for (channel, target) in output.iter_mut().enumerate() {
					let sample = if channels == 1 && self.pcm.channels == 2 {
						(source(0) + source(1)) * 0.5
					} else if channel < 2 {
						source(channel.min(self.pcm.channels - 1))
					} else {
						0.0
					};
					*target = T::from_sample(sample * volume);
				}
				self.frame += self.pcm.rate as f64 / self.output_rate as f64;
			}
		}
		gate.position_frames
			.store((self.frame as u64).min(frames as u64), Ordering::Release);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn decode_bounded_audio_and_reject_malformed_headers() {
		let pcm = decode(demo_wav(), &|| true).unwrap();
		assert_eq!(
			(pcm.rate, pcm.channels, pcm.samples.len()),
			(24000, 1, 288000)
		);
		assert!(pcm.samples.iter().any(|sample| sample.abs() > 0.01));
		let pcm = decode(
			include_bytes!("../tests/fixtures/audio-tone.mp3").to_vec(),
			&|| true,
		)
		.unwrap();
		assert_eq!((pcm.rate, pcm.channels), (24000, 1));
		assert!(!pcm.samples.is_empty());
		assert!(pcm.samples.iter().any(|sample| sample.abs() > 0.01));
		assert!(decode(demo_wav(), &|| false).is_err());
		assert!(decode(vec![0; MAX_ENCODED + 1], &|| true).is_err());
		for field in [22, 40] {
			let mut bytes = demo_wav();
			bytes[field..field + 2].copy_from_slice(&u16::MAX.to_le_bytes());
			assert!(decode(bytes, &|| true).is_err());
		}
		let mut bad_rate = demo_wav();
		bad_rate[24..28].copy_from_slice(&96001u32.to_le_bytes());
		assert!(decode(bad_rate, &|| true).is_err());
		let mut truncated = demo_wav();
		truncated.pop();
		assert!(decode(truncated, &|| true).is_err());
		assert!(decode(b"ID3\x04\0\0\x7f\x7f\x7f\x7f".to_vec(), &|| true).is_err());
		let mut metadata = demo_wav();
		metadata.splice(12..12, b"LIST\x04\0\0\0INFO".iter().copied());
		let len = (metadata.len() - 8) as u32;
		metadata[4..8].copy_from_slice(&len.to_le_bytes());
		assert_eq!(decode(metadata, &|| true).unwrap().samples.len(), 288000);
	}
	#[test]
	fn callback_pause_seek_volume_end_and_generation_cancellation() {
		let mut playback = Playback {
			pcm: Pcm {
				samples: vec![0.5; 200],
				channels: 2,
				rate: 1000,
			},
			frame: 0.0,
			output_rate: 1000,
		};
		let gate = Gate::default();
		let mut output = [0.0f32; 20];
		playback.render(&mut output, 2, &gate, 0);
		assert_eq!(output, [0.5; 20]);
		gate.paused.store(true, Ordering::Release);
		gate.seek_millis.store(40, Ordering::Release);
		playback.render(&mut output, 2, &gate, 0);
		assert_eq!(output, [0.0; 20]);
		assert_eq!(gate.position_frames.load(Ordering::Acquire), 40);
		gate.paused.store(false, Ordering::Release);
		gate.volume.store(0.5f32.to_bits(), Ordering::Release);
		playback.render(&mut output, 2, &gate, 0);
		assert_eq!(output, [0.25; 20]);
		gate.seek_millis.store(99, Ordering::Release);
		playback.render(&mut output, 2, &gate, 0);
		assert_eq!(&output[2..], &[0.0; 18]);
		assert_eq!(gate.position_frames.load(Ordering::Acquire), 100);
		gate.generation.store(1, Ordering::Release);
		playback.render(&mut output, 2, &gate, 0);
		assert_eq!(output, [0.0; 20]);
		let mut audio = Audio::default();
		audio.stop();
		audio.stop();
		assert_eq!(audio.gate.generation.load(Ordering::Acquire), 0);
		audio.volume(f32::NAN);
		assert_eq!(
			f32::from_bits(audio.gate.volume.load(Ordering::Acquire)),
			1.0
		);
	}
	#[tokio::test]
	async fn transfer_limits_and_cancel_stalled_response() {
		use tokio::io::{AsyncReadExt, AsyncWriteExt};
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/synthetic",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let server = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			let mut request = [0; 8192];
			let len = socket.read(&mut request).await.unwrap();
			let request = String::from_utf8_lossy(&request[..len]).to_ascii_lowercase();
			assert!(!request.contains("authorization:"));
			socket
				.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\n\r\n")
				.await
				.unwrap();
			std::future::pending::<()>().await;
		});
		let gate = Arc::new(Gate::default());
		let wake = Arc::new(Notify::new());
		let cancel_gate = gate.clone();
		let cancel_wake = wake.clone();
		let cancel = tokio::spawn(async move {
			tokio::time::sleep(Duration::from_millis(50)).await;
			cancel_gate.generation.store(1, Ordering::Release);
			cancel_wake.notify_one();
		});
		assert_eq!(
			tokio::time::timeout(
				Duration::from_secs(2),
				fetch(url.clone(), 1024, &gate, 0, &wake)
			)
			.await
			.unwrap(),
			Err("Cancelled")
		);
		cancel.await.unwrap();
		server.abort();
		assert_eq!(
			fetch(url, MAX_ENCODED + 1, &gate, 1, &wake).await,
			Err(TOO_LARGE)
		);
	}
}
