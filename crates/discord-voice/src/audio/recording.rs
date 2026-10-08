//! Explicit, microphone-only voice-message capture. Nothing is written to disk or sent.
//! The native callback shares call input conversion; processing and Opus/Ogg run here.
use super::{Devices, Gate, amplify, echo, open_input_stream};
use crate::Frame;
use model::voice_settings::Processing;
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
	mpsc,
};
use std::time::{Duration, Instant};

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
const FRAME_TIME: Duration = Duration::from_millis(20);
const WAVEFORM_BINS: usize = 64;
static WORKER_ACTIVE: AtomicBool = AtomicBool::new(false);

struct WorkerSlot;
impl WorkerSlot {
	fn acquire() -> Result<Self, &'static str> {
		WORKER_ACTIVE
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.map(|_| Self)
			.map_err(|_| "Previous voice-message recorder is still closing")
	}
}
impl Drop for WorkerSlot {
	fn drop(&mut self) {
		WORKER_ACTIVE.store(false, Ordering::Release);
	}
}

pub struct Config {
	pub devices: Devices,
	pub processing: Processing,
	pub input_gain: u16,
	pub max_duration_seconds: u16,
	/// Mute/deafen/PTT state at the explicit start gesture, before any callback.
	pub initial_muted: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
	Starting,
	Recording,
	Stopped,
	Failed,
}
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
	pub elapsed_ms: u32,
	pub waveform: [u8; WAVEFORM_BINS],
	pub state: State,
}
impl Default for Snapshot {
	fn default() -> Self {
		Self {
			elapsed_ms: 0,
			waveform: [0; WAVEFORM_BINS],
			state: State::Starting,
		}
	}
}
/// One bounded Ogg/Opus message, with at most 64 amplitude values. Session-only.
pub struct Recording {
	pub bytes: Vec<u8>,
	pub duration_secs: f32,
	pub waveform: Vec<u8>,
}

pub struct Recorder {
	gate: Arc<Gate>,
	cancelled: Arc<AtomicBool>,
	thread: std::thread::Thread,
	snapshot: tokio::sync::watch::Receiver<Snapshot>,
	result: mpsc::Receiver<Result<Recording, &'static str>>,
	done: Option<mpsc::Receiver<()>>,
}
impl Recorder {
	/// Call only after an explicit recording gesture; never from startup/demo/tests.
	pub fn start(mut config: Config) -> Result<Self, &'static str> {
		if !(5..=120).contains(&config.max_duration_seconds)
			|| config.input_gain > 200
			|| !config.processing.is_valid()
			|| config
				.devices
				.input
				.as_ref()
				.is_some_and(|id| id.len() > 512)
		{
			return Err("Invalid voice-message recording settings");
		}
		// A microphone-only recording has no playback echo reference. Suppression,
		// gain and sensitivity still use exactly the existing microphone DSP.
		config.processing.echo_cancellation = false;
		let slot = WorkerSlot::acquire()?;
		let gate = initial_gate(config.initial_muted);
		let cancelled = Arc::new(AtomicBool::new(false));
		let (publish, snapshot) = tokio::sync::watch::channel(Snapshot::default());
		let (completed, result) = mpsc::sync_channel(1);
		let (finished, done) = mpsc::sync_channel(1);
		let worker_gate = gate.clone();
		let worker_cancelled = cancelled.clone();
		let handle = std::thread::Builder::new()
			.name("voice-message".into())
			.spawn(move || {
				let outcome = capture(config, &worker_gate, &worker_cancelled, &publish);
				worker_gate.stopped.store(true, Ordering::Release);
				if !worker_cancelled.load(Ordering::Acquire) {
					publish.send_modify(|s| {
						s.state = if outcome.is_ok() {
							State::Stopped
						} else {
							State::Failed
						}
					});
					let _ = completed.try_send(outcome);
				}
				drop(slot);
				let _ = finished.try_send(());
			})
			.map_err(|_| "Could not start voice-message recorder")?;
		Ok(Self {
			gate,
			cancelled,
			thread: handle.thread().clone(),
			snapshot,
			result,
			done: Some(done),
		})
	}
	pub fn poll(&self) -> Snapshot {
		*self.snapshot.borrow()
	}
	pub fn take_result(&self) -> Option<Result<Recording, &'static str>> {
		self.result.try_recv().ok()
	}
	/// Mute/deafen/PTT take precedence; rejected input never reaches the encoder.
	pub fn set_controls(&self, muted: bool, enabled: bool) {
		let mute_changed = self.gate.muted.swap(muted, Ordering::AcqRel) != muted;
		let enabled_changed = self.gate.input_enabled.swap(enabled, Ordering::AcqRel) != enabled;
		if mute_changed || enabled_changed {
			self.gate.capture_generation.fetch_add(1, Ordering::AcqRel);
			self.thread.unpark();
		}
	}
	/// Immediately gates callbacks. The worker closes input before finalizing Opus.
	pub fn stop(&self) {
		self.gate.stopped.store(true, Ordering::Release);
		self.thread.unpark();
	}
	pub fn cancel(&self) {
		self.cancelled.store(true, Ordering::Release);
		self.stop();
	}
	/// Retire without blocking rendering; await the fence before another mic owner.
	pub fn shutdown(mut self) -> mpsc::Receiver<()> {
		self.cancel();
		self.done.take().expect("recorder retirement fence exists")
	}
}
impl Drop for Recorder {
	fn drop(&mut self) {
		self.cancel();
	}
}

fn initial_gate(muted: bool) -> Arc<Gate> {
	let gate = Arc::new(Gate::default());
	gate.muted.store(muted, Ordering::Release);
	gate.ready.store(true, Ordering::Release);
	gate.acknowledged_revision.store(1, Ordering::Release);
	gate
}

fn capture(
	config: Config,
	gate: &Arc<Gate>,
	cancelled: &AtomicBool,
	publish: &tokio::sync::watch::Sender<Snapshot>,
) -> Result<Recording, &'static str> {
	let mut encoded = Encoder::new(config.max_duration_seconds)?;
	let mut processing = echo::Echo::new();
	processing.configure(config.processing)?;
	if gate.stopped.load(Ordering::Acquire) {
		return Err("Recording cancelled");
	}
	let host = cpal::default_host();
	let (stream, mut input, _) = open_input_stream(&host, &config.devices, gate, 1)?;
	// A driver can finish opening after cancellation. Never allow its callbacks then.
	if gate.stopped.load(Ordering::Acquire) {
		drop(stream);
		return Err("Recording cancelled");
	}
	let started = Instant::now();
	let mut next_frame = started + FRAME_TIME;
	let mut callbacks = gate.input_callbacks.load(Ordering::Acquire);
	let mut last_callback = started;
	let mut generation = gate.capture_generation.load(Ordering::Acquire);
	let mut sensitivity = crate::activity::InputGate::default();
	let outcome = (|| {
		publish.send_modify(|s| s.state = State::Recording);
		while !gate.stopped.load(Ordering::Acquire) && !encoded.full() {
			let now = Instant::now();
			if now.duration_since(started)
				>= Duration::from_secs(u64::from(config.max_duration_seconds))
			{
				break;
			}
			let current_callbacks = gate.input_callbacks.load(Ordering::Acquire);
			if current_callbacks != callbacks {
				callbacks = current_callbacks;
				last_callback = now;
			}
			if gate.input_failed_revision.load(Ordering::Acquire) == 1
				|| now.duration_since(last_callback) >= Duration::from_secs(5)
			{
				return Err("Microphone stopped; select a working microphone and record again");
			}
			let current_generation = gate.capture_generation.load(Ordering::Acquire);
			if generation != current_generation || gate.echo_reset.swap(false, Ordering::AcqRel) {
				generation = current_generation;
				for _ in 0..8 {
					if input.pop().is_err() {
						break;
					}
				}
				processing.reset();
				sensitivity = crate::activity::InputGate::default();
			}
			if now < next_frame {
				std::thread::park_timeout(next_frame - now);
				continue;
			}
			let mut frame = input.pop().unwrap_or([0.0; 960]);
			if gate.capture() {
				processing.capture(&mut frame, false)?;
				let gain = f32::from(config.input_gain) / 100.0;
				for sample in &mut frame {
					*sample = amplify(*sample, gain);
				}
				sensitivity.apply(&mut frame, config.processing.sensitivity_db);
			} else {
				frame.fill(0.0);
			}
			if !gate.capture() || gate.capture_generation.load(Ordering::Acquire) != generation {
				frame.fill(0.0);
			}
			encoded.push(&frame)?;
			publish.send_modify(|s| {
				s.elapsed_ms = encoded.frames * 20;
				s.waveform = encoded.live_waveform();
			});
			next_frame += FRAME_TIME;
		}
		Ok(())
	})();
	// Device release is deliberately before flush, CRC, result transfer or error delivery.
	gate.stopped.store(true, Ordering::Release);
	drop(stream);
	drop(input);
	outcome?;
	if cancelled.load(Ordering::Acquire) {
		return Err("Recording cancelled");
	}
	encoded.finish()
}

/// Minimal single-stream Ogg mux, one complete bounded Opus packet per page.
struct Encoder {
	opus: opus2::Encoder,
	bytes: Vec<u8>,
	peaks: Vec<u8>,
	frames: u32,
	max_frames: u32,
	sequence: u32,
	preskip: u16,
}
impl Encoder {
	fn new(seconds: u16) -> Result<Self, &'static str> {
		if !(5..=120).contains(&seconds) {
			return Err("Invalid recording duration");
		}
		let mut opus = opus2::Encoder::new(48_000, opus2::Channels::Mono, opus2::Application::Voip)
			.map_err(|_| "Could not initialize voice-message encoder")?;
		opus.set_bitrate(opus2::Bitrate::Bits(32_000))
			.map_err(|_| "Could not set recording bitrate")?;
		let preskip = u16::try_from(
			opus.get_lookahead()
				.map_err(|_| "Could not read encoder delay")?,
		)
		.map_err(|_| "Unsupported recording encoder delay")?;
		if preskip > 960 {
			return Err("Unsupported recording encoder delay");
		}
		let mut result = Self {
			opus,
			bytes: Vec::new(),
			peaks: Vec::with_capacity(usize::from(seconds) * 50),
			frames: 0,
			max_frames: u32::from(seconds) * 50,
			sequence: 0,
			preskip,
		};
		let mut head = Vec::from(&b"OpusHead\x01\x01"[..]);
		head.extend_from_slice(&preskip.to_le_bytes());
		head.extend_from_slice(&48_000u32.to_le_bytes());
		head.extend_from_slice(&[0, 0, 0]);
		result.page(&head, 0, 2)?;
		result.page(b"OpusTags\x06\x00\x00\x00Serein\x00\x00\x00\x00", 0, 0)?;
		Ok(result)
	}
	fn full(&self) -> bool {
		self.frames >= self.max_frames
	}
	fn push(&mut self, frame: &Frame) -> Result<(), &'static str> {
		if self.full() {
			return Err("Recording duration limit reached");
		}
		let frame = frame.map(|v| amplify(v, 1.0));
		self.packet(&frame, u64::from(self.frames + 1) * 960, 0)?;
		self.frames += 1;
		let rms = (frame.iter().map(|v| v * v).sum::<f32>() / 960.0).sqrt();
		self.peaks
			.push((rms.sqrt() * 255.0).round().clamp(0.0, 255.0) as u8);
		Ok(())
	}
	fn live_waveform(&self) -> [u8; WAVEFORM_BINS] {
		let mut levels = [0; WAVEFORM_BINS];
		let recent = &self.peaks[self.peaks.len().saturating_sub(WAVEFORM_BINS)..];
		levels[WAVEFORM_BINS - recent.len()..].copy_from_slice(recent);
		levels
	}
	fn finish(mut self) -> Result<Recording, &'static str> {
		if self.frames == 0 {
			return Err("Record some audio before stopping");
		}
		// Flush the encoder delay; EOS granule trims the zero padding precisely.
		self.packet(
			&[0.0; 960],
			u64::from(self.frames) * 960 + u64::from(self.preskip),
			4,
		)?;
		let bins = self.peaks.len().min(WAVEFORM_BINS);
		let waveform = (0..bins)
			.map(|i| {
				let from = i * self.peaks.len() / bins;
				let to = (i + 1) * self.peaks.len() / bins;
				*self.peaks[from..to].iter().max().unwrap_or(&0)
			})
			.collect();
		Ok(Recording {
			bytes: self.bytes,
			duration_secs: self.frames as f32 / 50.0,
			waveform,
		})
	}
	fn packet(&mut self, frame: &Frame, granule: u64, flags: u8) -> Result<(), &'static str> {
		let mut packet = [0; 1275];
		let length = self
			.opus
			.encode_float(frame, &mut packet)
			.map_err(|_| "Voice-message encoding failed")?;
		self.page(&packet[..length], granule, flags)
	}
	fn page(&mut self, packet: &[u8], granule: u64, flags: u8) -> Result<(), &'static str> {
		let segments = packet.len() / 255 + 1;
		let length = 27 + segments + packet.len();
		if segments > 255 || self.bytes.len().saturating_add(length) > MAX_BYTES {
			return Err("Voice-message byte limit reached");
		}
		// Allocate only within the byte ceiling, including Vec capacity.
		if self.bytes.capacity() < self.bytes.len() + length {
			let growth = (self.bytes.capacity().max(4096) * 2).min(MAX_BYTES);
			self.bytes
				.reserve_exact(growth.saturating_sub(self.bytes.len()));
		}
		let from = self.bytes.len();
		self.bytes.extend_from_slice(b"OggS\x00");
		self.bytes.push(flags);
		self.bytes.extend_from_slice(&granule.to_le_bytes());
		self.bytes.extend_from_slice(&1u32.to_le_bytes());
		self.bytes.extend_from_slice(&self.sequence.to_le_bytes());
		self.bytes.extend_from_slice(&[0; 4]);
		self.bytes.push(segments as u8);
		for _ in 1..segments {
			self.bytes.push(255);
		}
		self.bytes.push((packet.len() % 255) as u8);
		self.bytes.extend_from_slice(packet);
		let checksum = crc(&self.bytes[from..]);
		self.bytes[from + 22..from + 26].copy_from_slice(&checksum.to_le_bytes());
		self.sequence += 1;
		Ok(())
	}
}
fn crc(bytes: &[u8]) -> u32 {
	let mut crc = 0u32;
	for byte in bytes {
		crc ^= u32::from(*byte) << 24;
		for _ in 0..8 {
			crc = if crc & 0x8000_0000 != 0 {
				(crc << 1) ^ 0x04c1_1db7
			} else {
				crc << 1
			};
		}
	}
	crc
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn control_stop_drop_and_retirement_fence_do_not_open_devices() {
		let gate = initial_gate(true);
		assert!(
			!gate.capture(),
			"initial mute must precede the first callback"
		);
		let cancelled = Arc::new(AtomicBool::new(false));
		let (_, snapshot) = tokio::sync::watch::channel(Snapshot::default());
		let (_, result) = mpsc::sync_channel(1);
		let (finished, done) = mpsc::sync_channel(1);
		let recorder = Recorder {
			gate: gate.clone(),
			cancelled: cancelled.clone(),
			thread: std::thread::current(),
			snapshot,
			result,
			done: Some(done),
		};
		recorder.set_controls(false, true);
		assert!(gate.capture());
		let (send, mut received) = rtrb::RingBuffer::new(8);
		let mut capture = super::super::Capture::new(48_000, send);
		for (muted, enabled) in [(true, true), (false, false)] {
			capture.process(&[0.75_f32; 100], 1, &gate);
			assert!(received.pop().is_err());
			let generation = gate.capture_generation.load(Ordering::Acquire);
			recorder.set_controls(muted, enabled);
			recorder.set_controls(false, true);
			assert!(gate.capture());
			assert_ne!(gate.capture_generation.load(Ordering::Acquire), generation);
			// A complete mute/PTT transition between callbacks must erase old PCM.
			capture.process(&[0.25_f32; 961], 1, &gate);
			assert_eq!(received.pop().unwrap(), [0.25; 960]);
			assert!(received.pop().is_err());
		}
		assert_eq!(gate.playback_generation.load(Ordering::Acquire), 0);
		recorder.set_controls(true, false);
		assert!(gate.muted.load(Ordering::Acquire));
		assert!(!gate.input_enabled.load(Ordering::Acquire));
		assert!(!gate.capture());
		recorder.set_controls(false, true);
		assert!(gate.capture());
		recorder.stop();
		assert!(!gate.capture());
		assert!(!cancelled.load(Ordering::Acquire));
		let fence = recorder.shutdown();
		assert!(cancelled.load(Ordering::Acquire));
		assert!(matches!(fence.try_recv(), Err(mpsc::TryRecvError::Empty)));
		finished.send(()).unwrap();
		assert_eq!(fence.try_recv(), Ok(()));
	}
	#[test]
	fn synthetic_ogg_opus_is_decodable_with_valid_pages_and_trimmed_duration() {
		let mut encoder = Encoder::new(5).unwrap();
		let preskip = encoder.preskip;
		for n in 0..50 {
			encoder
				.push(&std::array::from_fn(|i| {
					((n * 960 + i) as f32 * 0.05).sin() * 0.25
				}))
				.unwrap();
		}
		let recording = encoder.finish().unwrap();
		assert_eq!(recording.duration_secs, 1.0);
		assert_eq!(recording.waveform.len(), 50);
		assert!(recording.waveform.iter().any(|v| *v > 0));
		let mut decoder = opus2::Decoder::new(48_000, opus2::Channels::Mono).unwrap();
		let mut at = 0;
		let mut sequence = 0;
		let mut samples = 0;
		let mut last_granule = 0;
		while at < recording.bytes.len() {
			let page = &recording.bytes[at..];
			assert_eq!(&page[..5], b"OggS\x00");
			assert_eq!(
				u32::from_le_bytes(page[18..22].try_into().unwrap()),
				sequence
			);
			let segments = usize::from(page[26]);
			let size: usize = page[27..27 + segments]
				.iter()
				.map(|n| usize::from(*n))
				.sum();
			let length = 27 + segments + size;
			let mut checked = page[..length].to_vec();
			let expected = u32::from_le_bytes(checked[22..26].try_into().unwrap());
			checked[22..26].fill(0);
			assert_eq!(crc(&checked), expected);
			let packet = &page[27 + segments..length];
			if sequence == 0 {
				assert!(packet.starts_with(b"OpusHead"));
				assert_eq!(page[5], 2);
			} else if sequence == 1 {
				assert!(packet.starts_with(b"OpusTags"));
			} else {
				samples += decoder
					.decode_float(packet, &mut [0.0; 960], false)
					.unwrap();
			}
			last_granule = u64::from_le_bytes(page[6..14].try_into().unwrap());
			at += length;
			if at == recording.bytes.len() {
				assert_eq!(page[5], 4);
			}
			sequence += 1;
		}
		assert_eq!(last_granule - u64::from(preskip), 48_000);
		assert_eq!(samples, 48_960);
	}
	#[test]
	fn duration_waveform_byte_and_empty_limits_are_device_free() {
		assert!(Encoder::new(4).is_err());
		assert!(Encoder::new(121).is_err());
		assert!(Encoder::new(5).unwrap().finish().is_err());
		let maximum = Encoder::new(120).unwrap();
		assert_eq!(maximum.max_frames, 6000);
		assert_eq!(maximum.peaks.capacity(), 6000);
		let mut encoder = Encoder::new(5).unwrap();
		for _ in 0..250 {
			encoder.push(&[0.0; 960]).unwrap();
		}
		assert!(encoder.full());
		assert!(encoder.push(&[0.0; 960]).is_err());
		assert_eq!(encoder.peaks.len(), 250);
		let clip = encoder.finish().unwrap();
		assert_eq!(clip.duration_secs, 5.0);
		assert_eq!(clip.waveform, vec![0; 64]);
		assert!(clip.bytes.capacity() <= MAX_BYTES);
		let mut encoder = Encoder::new(5).unwrap();
		encoder.bytes.resize(MAX_BYTES - 20, 0);
		assert!(encoder.page(b"packet", 0, 0).is_err());
	}
}
