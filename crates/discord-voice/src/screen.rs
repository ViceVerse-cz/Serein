//! Explicitly selected, memory-only screen capture and video encoding.
pub use client_core::screen::{RequestSettings, Settings, Source, SourceId, valid_dimensions};
use model::voice_settings::{VideoCodec, VideoSettings};
#[cfg(target_os = "linux")]
#[path = "screen/audio_linux.rs"]
mod audio_linux;
#[cfg(all(test, not(target_os = "windows")))]
#[path = "screen/audio_windows.rs"]
mod audio_windows;
#[cfg(not(target_os = "linux"))]
#[path = "screen/capture.rs"]
mod capture;
#[path = "screen/convert.rs"]
mod convert;
#[cfg(target_os = "linux")]
#[path = "screen/gstreamer.rs"]
mod gstreamer;
#[cfg(target_os = "linux")]
#[path = "screen/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
#[path = "screen/portal_linux.rs"]
mod portal_linux;

use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
	mpsc,
};

#[cfg(not(target_os = "linux"))]
use std::time::{Duration, Instant};

pub const MAX_RAW_BYTES: usize = 7680 * 4320 * 8;
pub const MAX_ENCODED_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Default)]
pub enum RawFormat {
	#[default]
	Bgra8,
	/// Linear Rec.709 half floats. Windows scRGB uses 80 nits per unit;
	/// Apple EDR capture uses the canonical 203-nit reference white.
	LinearRgba16 {
		unit_nits: u32,
		white_nits: u32,
		peak_nits: u32,
	},
	P010 {
		transfer: u32,
		primaries: u32,
		matrix: u32,
		full_range: bool,
		peak_nits: u32,
	},
}
impl RawFormat {
	fn bytes(self) -> usize {
		match self {
			Self::Bgra8 => 4,
			Self::LinearRgba16 { .. } => 8,
			Self::P010 { .. } => 2,
		}
	}
}
pub struct RawFrame {
	pub format: RawFormat,
	pub width: u32,
	pub height: u32,
	pub stride: usize,
	pub data: Vec<u8>,
}

pub struct EncodedFrame {
	pub codec: VideoCodec,
	pub data: Vec<u8>,
	pub timestamp: u32,
	pub keyframe: bool,
	/// Prediction generation; a restart invalidates every older queued access unit.
	pub epoch: u64,
	/// Capture reset acknowledged before this encoder session accepted its input.
	pub reset_generation: u64,
	/// Output-size generation, independent of encryption and prediction resets.
	pub geometry_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
	pub settings: Settings,
	pub source_dimensions: (u32, u32),
	pub generation: u64,
}

/// Publish only validated capture geometry. A resize pauses sending until its
/// metadata is announced; old queued packets carry the previous generation.
pub(super) fn publish_output(
	slot: &Mutex<Option<Output>>,
	settings: Settings,
	source_dimensions: (u32, u32),
	ready: &AtomicBool,
	keyframe: &AtomicBool,
	bitrate: &AtomicU32,
) -> Result<Output, &'static str> {
	if !settings.valid()
		|| !valid_dimensions(source_dimensions.0, source_dimensions.1)
		|| settings.width > source_dimensions.0
		|| settings.height > source_dimensions.1
	{
		return Err("Invalid resolved screen dimensions");
	}
	let mut output = slot.lock().map_err(|_| "Screen dimensions unavailable")?;
	if let Some(current) = *output
		&& current.settings == settings
		&& current.source_dimensions == source_dimensions
	{
		return Ok(current);
	}
	let generation = match *output {
		Some(current) => current
			.generation
			.checked_add(1)
			.ok_or("Screen size generation exhausted")?,
		None => 0,
	};
	ready.store(false, Ordering::Release);
	keyframe.store(true, Ordering::Release);
	bitrate.store(settings.bit_rate(), Ordering::Release);
	let current = Output {
		settings,
		source_dimensions,
		generation,
	};
	*output = Some(current);
	Ok(current)
}

/// Longest system-audio chunk accepted from the OS: 100 ms of 48 kHz stereo.
pub const MAX_AUDIO_SAMPLES: usize = 4800 * 2;

#[derive(Debug)]
pub struct AudioChunk {
	pub samples: Vec<f32>,
	/// Capture generation; old buffers must not cross an encryption transition.
	pub epoch: u64,
}

pub struct Video {
	pub video_settings: VideoSettings,
	pub output: Arc<Mutex<Option<Output>>>,
	pub frames: tokio::sync::mpsc::Receiver<EncodedFrame>,
	pub ready: Arc<AtomicBool>,
	pub keyframe: Arc<AtomicBool>,
	/// Transport feedback target in bits per second, bounded by the selected quality.
	pub bitrate: Arc<AtomicU32>,
	/// Interleaved 48 kHz stereo system audio, present only when the share requested it.
	pub audio: Option<tokio::sync::mpsc::Receiver<AudioChunk>>,
	pub audio_epoch: Arc<AtomicU64>,
}

impl Video {
	pub(crate) fn output(&self) -> Option<Output> {
		self.output
			.lock()
			.ok()
			.and_then(|output| *output)
			.filter(|output| {
				output.settings.valid()
					&& valid_dimensions(output.source_dimensions.0, output.source_dimensions.1)
					&& output.settings.width <= output.source_dimensions.0
					&& output.settings.height <= output.source_dimensions.1
			})
	}
	pub(crate) fn enable_output(&self, announced: Output) -> bool {
		let Ok(output) = self.output.lock() else {
			return false;
		};
		if *output != Some(announced) {
			return false;
		}
		self.keyframe.store(true, Ordering::Release);
		self.ready.store(true, Ordering::Release);
		true
	}
}

/// Whether this platform can capture system audio with the screen.
pub fn audio_supported() -> bool {
	supported()
}

pub fn supported() -> bool {
	cfg!(any(
		target_os = "macos",
		target_os = "windows",
		target_os = "linux"
	))
}

pub fn sources() -> Result<Vec<Source>, &'static str> {
	#[cfg(target_os = "linux")]
	{
		let mut sources = vec![Source {
			id: SourceId::Portal,
			name: "Choose in the system picker".into(),
			dimensions: None,
		}];
		if linux::x11_session() {
			sources.push(Source {
				id: SourceId::X11Desktop,
				name: "Entire X11 desktop · all monitors · no portal".into(),
				dimensions: None,
			});
		}
		Ok(sources)
	}
	#[cfg(not(target_os = "linux"))]
	capture::sources()
}

pub struct Worker {
	stop: Arc<AtomicBool>,
	ready: Arc<AtomicBool>,
	preview: Arc<Mutex<Option<image::RgbaImage>>>,
	done: Option<mpsc::Receiver<Result<(), &'static str>>>,
	#[cfg(target_os = "linux")]
	status: Arc<Mutex<&'static str>>,
	#[cfg(target_os = "linux")]
	preview_visible: Arc<AtomicBool>,
}

impl Worker {
	pub fn start(
		settings: impl Into<RequestSettings>,
		video_settings: VideoSettings,
		wake: impl Fn() + Send + 'static,
	) -> Result<(Self, Video), &'static str> {
		Self::start_on_adapter(settings, video_settings, None, wake)
	}

	pub fn start_on_adapter(
		settings: impl Into<RequestSettings>,
		video_settings: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		wake: impl Fn() + Send + 'static,
	) -> Result<(Self, Video), &'static str> {
		let settings = settings.into();
		if !settings.valid() || !video_settings.is_valid() || !supported() {
			return Err("Screen sharing is unavailable for these settings or this platform");
		}
		let stop = Arc::new(AtomicBool::new(false));
		let ready = Arc::new(AtomicBool::new(false));
		let audio_epoch = Arc::new(AtomicU64::new(0));
		let worker_audio_epoch = audio_epoch.clone();
		let keyframe = Arc::new(AtomicBool::new(true));
		let bitrate = Arc::new(AtomicU32::new(250_000));
		let output = Arc::new(Mutex::new(None));
		let worker_output = output.clone();
		let worker_bitrate = bitrate.clone();
		let preview = Arc::new(Mutex::new(None));
		let worker_preview = preview.clone();
		#[cfg(target_os = "linux")]
		let status = Arc::new(Mutex::new("Choose a screen or window in the system picker"));
		#[cfg(target_os = "linux")]
		let worker_status = status.clone();
		#[cfg(target_os = "linux")]
		let preview_visible = Arc::new(AtomicBool::new(true));
		#[cfg(target_os = "linux")]
		let worker_preview_visible = preview_visible.clone();
		// A few frames of slack absorbs send jitter without forcing keyframes on every hiccup.
		let (send, frames) = tokio::sync::mpsc::channel(3);
		let (complete, done) = mpsc::sync_channel(1);
		let (audio_send, audio) = if settings.audio && audio_supported() {
			let (send, receive) = tokio::sync::mpsc::channel(4);
			(Some(send), Some(receive))
		} else {
			(None, None)
		};
		let (worker_stop, worker_ready, worker_keyframe) =
			(stop.clone(), ready.clone(), keyframe.clone());
		std::thread::Builder::new()
			.name("screen-encoder".into())
			.spawn(move || {
				// The MFT event poll and capture pacing sleep 1 ms at a time.
				let _resolution = crate::timer::Resolution::acquire();
				let finished_ready = worker_ready.clone();
				#[cfg(target_os = "linux")]
				let result = linux::run(
					settings,
					worker_output,
					video_settings,
					adapter,
					worker_stop,
					worker_ready,
					worker_keyframe,
					worker_bitrate,
					send,
					audio_send,
					worker_audio_epoch,
					worker_preview,
					worker_status,
					worker_preview_visible,
					&wake,
				);
				#[cfg(not(target_os = "linux"))]
				let result = encode_loop(
					settings,
					worker_output,
					video_settings,
					adapter,
					worker_stop,
					worker_ready,
					worker_keyframe,
					worker_bitrate,
					send,
					audio_send,
					worker_audio_epoch,
					worker_preview,
					&wake,
				);
				finished_ready.store(false, Ordering::Release);
				// The desktop only shows the latest status, which a later stop overwrites.
				// Name the cause once so a share that ends by itself is never a mystery.
				if std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|value| value == "1") {
					match &result {
						Ok(()) => eprintln!("[Serein voice Screen] capture_stopped=ok"),
						Err(reason) => eprintln!("[Serein voice Screen] capture_stopped={reason}"),
					}
				}
				let _ = complete.try_send(result);
				wake();
			})
			.map_err(|_| "Could not start screen capture worker")?;
		Ok((
			Self {
				stop,
				ready: ready.clone(),
				preview,
				done: Some(done),
				#[cfg(target_os = "linux")]
				status,
				#[cfg(target_os = "linux")]
				preview_visible,
			},
			Video {
				video_settings,
				output,
				frames,
				ready,
				keyframe,
				bitrate,
				audio,
				audio_epoch,
			},
		))
	}

	pub fn set_preview_visible(&self, _visible: bool) {
		#[cfg(target_os = "linux")]
		self.preview_visible.store(_visible, Ordering::Release);
	}

	pub fn capture_status(&self) -> Option<&'static str> {
		#[cfg(target_os = "linux")]
		return self.status.try_lock().ok().map(|status| *status);
		#[cfg(not(target_os = "linux"))]
		None
	}

	pub fn result(&self) -> Option<Result<(), &'static str>> {
		self.done.as_ref()?.try_recv().ok()
	}

	/// One local RGBA image, bounded to 640 by 360 pixels and replaced at most ten times a second.
	pub fn take_preview(&self) -> Option<image::RgbaImage> {
		self.preview.try_lock().ok()?.take()
	}

	pub fn shutdown(mut self) -> mpsc::Receiver<Result<(), &'static str>> {
		self.done.take().expect("screen worker completion")
	}
}

impl Drop for Worker {
	fn drop(&mut self) {
		self.ready.store(false, Ordering::Release);
		self.stop.store(true, Ordering::Release);
	}
}

#[cfg(not(target_os = "linux"))]
#[allow(clippy::too_many_arguments)] // Media outputs of one explicitly started capture.
fn encode_loop(
	settings: RequestSettings,
	output: Arc<Mutex<Option<Output>>>,
	video_settings: VideoSettings,
	adapter: Option<model::VideoAdapter>,
	stop: Arc<AtomicBool>,
	ready: Arc<AtomicBool>,
	keyframe: Arc<AtomicBool>,
	bitrate: Arc<AtomicU32>,
	send: tokio::sync::mpsc::Sender<EncodedFrame>,
	audio: Option<tokio::sync::mpsc::Sender<AudioChunk>>,
	audio_epoch: Arc<AtomicU64>,
	preview: Arc<Mutex<Option<image::RgbaImage>>>,
	wake: &impl Fn(),
) -> Result<(), &'static str> {
	if stop.load(Ordering::Acquire) || send.is_closed() {
		return Ok(());
	}
	let origin = Instant::now();
	let mut reset_generation = audio_epoch.load(Ordering::Acquire);
	let (raw_send, raw) = mpsc::sync_channel(1);
	let capture_stop = Arc::new(AtomicBool::new(false));
	#[cfg(target_os = "windows")]
	let raw_pending = Arc::new(AtomicBool::new(false));
	#[cfg(target_os = "windows")]
	let mut _native = capture::Capture::start(
		settings,
		raw_send,
		audio,
		capture_stop.clone(),
		ready.clone(),
		audio_epoch.clone(),
		raw_pending.clone(),
	)?;
	// The worker's stop also reaches a pending macOS picker, so call teardown closes it now.
	#[cfg(not(target_os = "windows"))]
	let mut _native = capture::Capture::start(
		settings,
		raw_send,
		audio,
		capture_stop.clone(),
		ready.clone(),
		audio_epoch.clone(),
		stop.clone(),
	)?;
	let mut encoding: Option<ScreenEncoder> = None;
	let mut first_frame_deadline = Some(Instant::now() + Duration::from_secs(15));
	let mut next_frame = Instant::now();
	let mut next_preview = Instant::now();
	let mut latest_frame = None;
	let mut next_epoch = 0;
	let mut current_output: Option<Output> = None;

	while !stop.load(Ordering::Acquire) && !send.is_closed() {
		#[cfg(target_os = "windows")]
		if _native.failed() {
			return Err(
				"System audio capture stopped; check your output device or share without audio",
			);
		}
		if capture_stop.load(Ordering::Acquire) {
			return Err("The selected screen or window stopped sharing");
		}
		#[cfg(target_os = "macos")]
		_native.refresh_dimensions()?;
		let interval = Duration::from_secs_f64(1.0 / f64::from(settings.fps));
		let frame = match raw.recv_timeout(interval.min(Duration::from_millis(100))) {
			Ok(frame) => {
				#[cfg(target_os = "windows")]
				raw_pending.store(false, Ordering::Release);
				Some(frame)
			}
			Err(_) if stop.load(Ordering::Acquire) || send.is_closed() => break,
			Err(mpsc::RecvTimeoutError::Timeout)
				if first_frame_deadline.is_none_or(|deadline| Instant::now() < deadline) =>
			{
				None
			}
			Err(_) => {
				return Err(
					"No screen frames received; check screen recording permission and the selected source",
				);
			}
		};
		if stop.load(Ordering::Acquire) || send.is_closed() {
			break;
		}
		let reset = audio_epoch.load(Ordering::Acquire);
		if reset == u64::MAX {
			return Err("Screen security reset generation exhausted");
		}
		if reset != reset_generation {
			reset_generation = reset;
			if let Some(encoder) = &mut encoding {
				encoder.reset_for_security(reset)?;
			}
			latest_frame = None;
			keyframe.store(true, Ordering::Release);
			// This input can span the transition. Restart the selected capture so an
			// idle desktop supplies a new snapshot instead of waiting for damage.
			_native.request_frame(&raw)?;
			first_frame_deadline = Some(Instant::now() + Duration::from_secs(15));
			next_frame = Instant::now();
			continue;
		}
		let now = Instant::now();
		if let Some(frame) = &frame {
			validate_frame(frame)?;
			let resolved = settings
				.resolve((frame.width, frame.height))
				.ok_or("Unsupported screen source dimensions")?;
			let current = publish_output(
				&output,
				resolved,
				(frame.width, frame.height),
				&ready,
				&keyframe,
				&bitrate,
			)?;
			if current_output != Some(current) {
				if let Some(encoder) = &encoding {
					next_epoch = encoder
						.epoch()
						.checked_add(1)
						.ok_or("Screen encoder epoch exhausted")?;
				}
				encoding = None;
				latest_frame = None;
				current_output = Some(current);
				next_frame = now;
			}
			first_frame_deadline = None;
			if now >= next_preview {
				let image = preview_frame(frame)?;
				if let Ok(mut slot) = preview.try_lock() {
					*slot = Some(image);
				}
				next_preview = now + Duration::from_millis(100);
				wake();
			}
		}
		let encode = retain_screen_frame(
			&mut latest_frame,
			frame,
			ready.load(Ordering::Acquire),
			keyframe.load(Ordering::Acquire)
				|| encoding.as_ref().is_some_and(ScreenEncoder::pending),
		)?;
		// Local capture remains available while alone; only secure media is encoded or queued.
		if !ready.load(Ordering::Acquire) {
			if let Some(encoder) = &encoding {
				next_epoch = encoder
					.epoch()
					.checked_add(1)
					.ok_or("Screen encoder epoch exhausted")?;
			}
			encoding = None;
			keyframe.store(true, Ordering::Release);
			continue;
		}
		if !encode {
			continue;
		}
		// Pace on an accumulating schedule with a little tolerance: capture timing jitter must
		// not skip every other frame, and a stalled encoder resumes from now instead of bursting.
		if now + Duration::from_millis(2) < next_frame {
			continue;
		}
		next_frame = (next_frame + interval).max(now + interval / 2);
		// Drop before encoding while the transport is behind, so the encoder never references
		// a picture the receiver did not get.
		if send.capacity() == 0 {
			continue;
		}
		let current = current_output.ok_or("Screen dimensions are unresolved")?;
		let resolved = current.settings;
		let target = bitrate
			.load(Ordering::Acquire)
			.clamp(250_000, resolved.bit_rate());
		if encoding.is_none() {
			encoding = Some(ScreenEncoder::new_on_adapter(
				resolved,
				target,
				video_settings,
				adapter,
				next_epoch,
				reset_generation,
			)?);
		}
		if encoding
			.as_mut()
			.expect("secure screen encoder")
			.set_bitrate(target)?
		{
			keyframe.store(true, Ordering::Release);
		}
		// Retain one source snapshot for keyframe requests on an unchanged desktop.
		let force_keyframe = keyframe.swap(false, Ordering::AcqRel);
		if audio_epoch.load(Ordering::Acquire) != reset_generation {
			continue;
		}
		let packet = encoding
			.as_mut()
			.expect("secure screen encoder")
			.encode_at(
				latest_frame.as_ref().expect("latest screen frame"),
				force_keyframe,
				(origin.elapsed().as_micros() * 90 / 1000) as u32,
			)?;
		if packet.data.is_empty() {
			continue;
		}
		if !ready.load(Ordering::Acquire)
			|| stop.load(Ordering::Acquire)
			|| audio_epoch.load(Ordering::Acquire) != reset_generation
		{
			continue;
		}
		let frame = EncodedFrame {
			geometry_generation: current.generation,
			codec: video_settings.codec,
			data: packet.data,
			timestamp: packet.timestamp,
			keyframe: packet.keyframe,
			epoch: packet.epoch,
			reset_generation: encoding
				.as_ref()
				.expect("secure screen encoder")
				.reset_generation,
		};
		if send.try_send(frame).is_err() {
			encoding
				.as_mut()
				.expect("secure screen encoder")
				.restart()?;
			keyframe.store(true, Ordering::Release);
		}
	}
	capture_stop.store(true, Ordering::Release);
	Ok(())
}

pub(super) fn retain_screen_frame(
	latest: &mut Option<RawFrame>,
	frame: Option<RawFrame>,
	ready: bool,
	keyframe: bool,
) -> Result<bool, &'static str> {
	let fresh = frame.is_some();
	if let Some(frame) = frame {
		validate_frame(&frame)?;
		*latest = Some(frame);
	}
	Ok(ready && latest.is_some() && (fresh || keyframe))
}

/// Selected encoder shared by every platform; native capture remains independent.
pub(super) struct ScreenEncoder {
	diagnostics: crate::diagnostics::EncoderRegistration,
	encoder: Option<crate::video_backend::Encoder>,
	video_settings: VideoSettings,
	i420: Vec<u8>,
	settings: Settings,
	bitrate: u32,
	adapter: Option<model::VideoAdapter>,
	epoch_base: u64,
	awaiting_keyframe: bool,
	reset_generation: u64,
}

impl ScreenEncoder {
	pub(super) fn new_on_adapter(
		settings: Settings,
		bitrate: u32,
		video_settings: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		epoch_base: u64,
		reset_generation: u64,
	) -> Result<Self, &'static str> {
		if !settings.valid() || !video_settings.is_valid() || reset_generation == u64::MAX {
			return Err("Invalid screen encoder settings");
		}
		let mut config = Self::config(settings, bitrate, video_settings.codec);
		config.adapter = adapter;
		let encoder = crate::video_backend::Encoder::new(config, video_settings.backend)?;
		Ok(Self {
			diagnostics: crate::diagnostics::EncoderRegistration::new(
				true,
				encoder.hardware(),
				encoder.amf_split(),
			),
			encoder: Some(encoder),
			video_settings,
			i420: Vec::new(),
			settings,
			bitrate,
			adapter,
			epoch_base,
			awaiting_keyframe: true,
			reset_generation,
		})
	}

	fn config(settings: Settings, bitrate: u32, codec: VideoCodec) -> crate::video_encode::Config {
		crate::video_encode::Config {
			width: settings.width,
			height: settings.height,
			fps: settings.fps,
			bit_rate: bitrate.clamp(250_000, settings.bit_rate()),
			max_bytes: MAX_ENCODED_BYTES,
			profile: crate::video_encode::Profile::Main,
			codec,
			adapter: None,
			hdr: None,
		}
	}

	#[cfg(target_os = "linux")]
	pub(super) fn label(&self) -> &'static str {
		self.encoder
			.as_ref()
			.map_or("FFmpeg encoder stopped", |encoder| encoder.label())
	}

	pub(super) fn set_bitrate(&mut self, bitrate: u32) -> Result<bool, &'static str> {
		let bitrate = bitrate.clamp(250_000, self.settings.bit_rate());
		// Rate changes restart at an IDR. Bound restart frequency with the sender's
		// existing 15% reduction / 25% recovery thresholds.
		if self.awaiting_keyframe || !software_rate_change(self.bitrate, bitrate) {
			return Ok(false);
		}
		self.diagnostics.set(None);
		let mut config = Self::config(self.settings, bitrate, self.video_settings.codec);
		config.adapter = self.adapter;
		let encoder = self.encoder.as_mut().ok_or("Screen encoder stopped")?;
		encoder.reconfigure(config)?;
		self.diagnostics
			.set(Some((encoder.hardware(), encoder.amf_split())));
		self.bitrate = bitrate;
		self.awaiting_keyframe = true;
		Ok(true)
	}

	#[cfg(test)]
	pub(super) fn encode(
		&mut self,
		frame: &RawFrame,
		force: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let packet = self.encode_at(frame, force, 0)?;
		Ok((packet.data, packet.keyframe))
	}

	pub(super) fn pending(&self) -> bool {
		self.encoder
			.as_ref()
			.is_some_and(|encoder| encoder.pending())
	}

	pub(super) fn epoch(&self) -> u64 {
		self.epoch_base
			.saturating_add(self.encoder.as_ref().map_or(0, |encoder| encoder.epoch()))
	}

	pub(super) fn restart(&mut self) -> Result<(), &'static str> {
		if !self.awaiting_keyframe {
			self.encoder
				.as_mut()
				.ok_or("Screen encoder stopped")?
				.restart()?;
			self.awaiting_keyframe = true;
		}
		Ok(())
	}

	#[cfg(target_os = "linux")]
	pub(super) fn reset_generation(&self) -> u64 {
		self.reset_generation
	}

	pub(super) fn reset_for_security(&mut self, generation: u64) -> Result<bool, &'static str> {
		if generation == u64::MAX {
			return Err("Screen security reset generation exhausted");
		}
		if generation == self.reset_generation {
			return Ok(false);
		}
		// Unlike ordinary keyframe feedback, this must discard an already pending IDR.
		self.encoder
			.as_mut()
			.ok_or("Screen encoder stopped")?
			.restart()?;
		self.reset_generation = generation;
		self.awaiting_keyframe = true;
		Ok(true)
	}

	pub(super) fn encode_at(
		&mut self,
		frame: &RawFrame,
		force: bool,
		timestamp: u32,
	) -> Result<crate::video_encode::EncodedPacket, &'static str> {
		if force {
			self.restart()?;
		}
		let (width, height) = (self.settings.width as usize, self.settings.height as usize);
		self.i420.resize(width * height * 3 / 2, 0);
		convert::bgra_to_i420(frame, width, height, &mut self.i420)?;
		let encoder = self
			.encoder
			.as_mut()
			.ok_or("FFmpeg screen encoder stopped")?;
		let mut result = encoder.encode_at(&self.i420, false, timestamp)?;
		self.diagnostics
			.set(Some((encoder.hardware(), encoder.amf_split())));
		result.epoch = result
			.epoch
			.checked_add(self.epoch_base)
			.ok_or("Screen encoder epoch exhausted")?;
		if !result.data.is_empty() {
			if self.awaiting_keyframe && !result.keyframe {
				return Err("Screen encoder did not restart at an independently decodable picture");
			}
			self.awaiting_keyframe = false;
		}
		Ok(result)
	}
}

/// Significant bitrate moves apply without restarting on every feedback tick.
pub(crate) fn software_rate_change(current: u32, target: u32) -> bool {
	u64::from(target) * 100 <= u64::from(current) * 85
		|| u64::from(target) * 100 >= u64::from(current) * 125
}

fn validate_frame(frame: &RawFrame) -> Result<(usize, usize), &'static str> {
	let row_bytes = (frame.width as usize)
		.checked_mul(frame.format.bytes())
		.ok_or("Screen capture returned an unsupported frame size")?;
	let required = frame
		.stride
		.checked_mul(frame.height as usize)
		.ok_or("Screen capture returned an unsupported frame size")?;
	if !valid_dimensions(frame.width, frame.height)
		|| frame.data.len() > MAX_RAW_BYTES
		|| frame.stride < row_bytes
		|| required > frame.data.len()
		|| matches!(frame.format, RawFormat::P010 { .. })
			&& (!frame.width.is_multiple_of(2)
				|| !frame.height.is_multiple_of(2)
				|| frame.stride != frame.width as usize * 2
				|| required + required / 2 != frame.data.len())
	{
		return Err("Screen capture returned an unsupported frame size");
	}

	let valid_color = match frame.format {
		RawFormat::Bgra8 => true,
		RawFormat::LinearRgba16 {
			unit_nits,
			white_nits,
			peak_nits,
		} => {
			(1..=1000).contains(&unit_nits)
				&& (80..=1000).contains(&white_nits)
				&& (white_nits..=10000).contains(&peak_nits)
		}
		RawFormat::P010 {
			transfer,
			primaries,
			matrix,
			peak_nits,
			..
		} => {
			matches!(transfer, 16 | 18)
				&& matches!(primaries, 1 | 9 | 12)
				&& matches!(matrix, 1 | 5 | 6 | 9)
				&& (100..=10000).contains(&peak_nits)
		}
	};
	if !valid_color {
		return Err("Screen capture returned unsupported color metadata");
	}

	Ok((row_bytes, required))
}

pub(super) fn preview_frame(frame: &RawFrame) -> Result<image::RgbaImage, &'static str> {
	validate_frame(frame)?;
	let scale = (640.0 / f64::from(frame.width))
		.min(360.0 / f64::from(frame.height))
		.min(1.0);
	let width = (f64::from(frame.width) * scale).round().max(1.0) as u32;
	let height = (f64::from(frame.height) * scale).round().max(1.0) as u32;
	// Nearest sampling keeps the ten-fps preview cheap; streaming uses filtered scaling.
	let conversion = frame.conversion();
	Ok(image::RgbaImage::from_fn(width, height, |x, y| {
		let source_x = (x * frame.width / width) as usize;
		let source_y = (y * frame.height / height) as usize;
		let rgb = frame.sdr(source_x, source_y, conversion.as_ref());
		image::Rgba([rgb[0], rgb[1], rgb[2], 255])
	}))
}

impl RawFrame {
	fn conversion(&self) -> Option<platform::video::ffmpeg::SdrConverter> {
		use platform::video::ffmpeg::{HdrTransfer, SdrConverter};
		match self.format {
			RawFormat::Bgra8 => None,
			RawFormat::LinearRgba16 {
				white_nits,
				peak_nits,
				..
			} => Some(SdrConverter::new(
				1,
				peak_nits as f32,
				white_nits as f32,
				None,
			)),
			RawFormat::P010 {
				primaries,
				peak_nits,
				transfer,
				..
			} => Some(SdrConverter::new(
				primaries,
				peak_nits as f32,
				203.0,
				HdrTransfer::from_native(transfer),
			)),
		}
	}
	fn sdr(
		&self,
		x: usize,
		y: usize,
		conversion: Option<&platform::video::ffmpeg::SdrConverter>,
	) -> [u8; 3] {
		let at = y * self.stride + x * self.format.bytes();
		if matches!(self.format, RawFormat::Bgra8) {
			return [self.data[at + 2], self.data[at + 1], self.data[at]];
		}
		let conversion = conversion.expect("validated HDR conversion");
		conversion.to_sdr(self.nits(x, y, conversion))
	}
	fn nits(
		&self,
		x: usize,
		y: usize,
		conversion: &platform::video::ffmpeg::SdrConverter,
	) -> [f32; 3] {
		let at = y * self.stride + x * self.format.bytes();
		match self.format {
			RawFormat::LinearRgba16 { unit_nits, .. } => std::array::from_fn(|channel| {
				platform::video::ffmpeg::half([
					self.data[at + channel * 2],
					self.data[at + channel * 2 + 1],
				]) * unit_nits as f32
			}),
			RawFormat::P010 {
				transfer,
				matrix,
				full_range,
				..
			} => {
				let word =
					|at| (u16::from_le_bytes([self.data[at], self.data[at + 1]]) >> 6) as f32;
				let yy = word(at);
				let c = self.stride * self.height as usize + y / 2 * self.stride + x / 2 * 4;
				let uv = [word(c), word(c + 2)];
				let yy = if full_range {
					yy / 1023.0
				} else {
					(yy - 64.0) / 876.0
				};
				let uv = uv.map(|v| (v - 512.0) / if full_range { 1023.0 } else { 896.0 });
				let (kr, kb) = match matrix {
					9 => (0.2627, 0.0593),
					1 => (0.2126, 0.0722),
					_ => (0.299, 0.114),
				};
				let kg = 1.0 - kr - kb;
				let rgb = [
					yy + 2.0 * (1.0 - kr) * uv[1],
					yy - 2.0 * kb * (1.0 - kb) / kg * uv[0] - 2.0 * kr * (1.0 - kr) / kg * uv[1],
					yy + 2.0 * (1.0 - kb) * uv[0],
				];
				conversion.source_nits(
					rgb,
					platform::video::ffmpeg::HdrTransfer::from_native(transfer)
						.expect("validated capture transfer"),
				)
			}
			RawFormat::Bgra8 => [0.0; 3],
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn source_aspect_ratios_encode_the_resolved_dimensions_and_fresh_keyframes() {
		use openh264::formats::YUVSource;
		for (source, expected) in [
			((1280, 720), (854, 480)),
			((1280, 800), (768, 480)),
			((1280, 960), (640, 480)),
			((1280, 1024), (600, 480)),
			((1500, 1000), (720, 480)),
			((1720, 720), (1146, 480)),
			((2560, 720), (1706, 480)),
			((6144, 2560), (1152, 480)),
			((720, 1280), (480, 854)),
			((720, 720), (480, 480)),
		] {
			let request = RequestSettings {
				source: SourceId::Display(1),
				resolution: client_core::screen::Resolution::Preset(
					model::voice_settings::VideoResolution::P480,
				),
				fps: 30,
				cursor: true,
				audio: false,
			};
			let settings = request.resolve(source).unwrap();
			assert_eq!((settings.width, settings.height), expected);
			let mut encoder = ScreenEncoder::new_on_adapter(
				settings,
				settings.bit_rate(),
				VideoSettings::default(),
				None,
				0,
				0,
			)
			.unwrap();
			let frame = RawFrame {
				format: RawFormat::Bgra8,
				width: source.0,
				height: source.1,
				stride: source.0 as usize * 4,
				data: vec![128; source.0 as usize * source.1 as usize * 4],
			};
			let (encoded, keyframe) = encoder.encode(&frame, true).unwrap();
			assert!(keyframe && crate::video_receive::has_parameter_sets(&encoded));
			let mut decoder = openh264::decoder::Decoder::new().unwrap();
			assert_eq!(
				decoder.decode(&encoded).unwrap().unwrap().dimensions(),
				(expected.0 as usize, expected.1 as usize)
			);
			assert!(encoder.reset_for_security(1).unwrap());
			let (encoded, keyframe) = encoder.encode(&frame, false).unwrap();
			assert!(keyframe && crate::video_receive::has_parameter_sets(&encoded));
		}
	}
	#[test]
	fn output_generation_retires_older_geometry_and_bounds_invalid_publication() {
		let output = Arc::new(Mutex::new(None));
		let ready = Arc::new(AtomicBool::new(true));
		let keyframe = Arc::new(AtomicBool::new(false));
		let bitrate = Arc::new(AtomicU32::new(0));
		let settings = Settings {
			source: SourceId::Display(1),
			width: 6144,
			height: 2560,
			fps: 60,
			cursor: true,
			audio: false,
		};
		let first = publish_output(
			&output,
			settings,
			(settings.width, settings.height),
			&ready,
			&keyframe,
			&bitrate,
		)
		.unwrap();
		assert_eq!(first.generation, 0);
		assert!(!ready.load(Ordering::Acquire));
		assert!(keyframe.load(Ordering::Acquire));
		assert_eq!(bitrate.load(Ordering::Acquire), 50_000_000);
		let (_, frames) = tokio::sync::mpsc::channel(1);
		let video = Video {
			video_settings: VideoSettings::default(),
			output: output.clone(),
			frames,
			ready: ready.clone(),
			keyframe: keyframe.clone(),
			bitrate: bitrate.clone(),
			audio: None,
			audio_epoch: Arc::new(AtomicU64::new(0)),
		};
		assert!(video.enable_output(first));
		assert_eq!(
			publish_output(
				&output,
				settings,
				(settings.width, settings.height),
				&ready,
				&keyframe,
				&bitrate
			)
			.unwrap(),
			first
		);
		assert!(ready.load(Ordering::Acquire));
		let resized = publish_output(
			&output,
			Settings {
				width: 2592,
				height: 1080,
				..settings
			},
			(6144, 2560),
			&ready,
			&keyframe,
			&bitrate,
		)
		.unwrap();
		assert_eq!(resized.generation, 1);
		assert!(!video.enable_output(first));
		assert!(!ready.load(Ordering::Acquire));
		assert!(video.enable_output(resized));
		assert!(
			publish_output(
				&output,
				Settings {
					width: 0,
					..settings
				},
				(6144, 2560),
				&ready,
				&keyframe,
				&bitrate
			)
			.is_err()
		);
		assert_eq!(video.output(), Some(resized));
		let same_output = publish_output(
			&output,
			resized.settings,
			(3072, 1280),
			&ready,
			&keyframe,
			&bitrate,
		)
		.unwrap();
		assert_eq!(same_output.settings, resized.settings);
		assert_eq!(same_output.generation, 2);
		assert!(!ready.load(Ordering::Acquire));
		assert!(!video.enable_output(resized));
		assert!(video.enable_output(same_output));

		*output.lock().unwrap() = Some(Output {
			generation: u64::MAX,
			..resized
		});
		assert!(
			publish_output(
				&output,
				settings,
				(settings.width, settings.height),
				&ready,
				&keyframe,
				&bitrate
			)
			.is_err()
		);
	}

	#[test]
	fn screen_recovery_coalesces_feedback_and_always_honors_security_resets() {
		let settings = Settings {
			source: SourceId::Display(1),
			width: 320,
			height: 240,
			fps: 30,
			cursor: false,
			audio: false,
		};
		let video_settings = VideoSettings::default();
		let software = crate::video_backend::Encoder::software(ScreenEncoder::config(
			settings,
			settings.bit_rate(),
			video_settings.codec,
		))
		.unwrap();
		let mut encoder = ScreenEncoder {
			diagnostics: crate::diagnostics::EncoderRegistration::new(
				true,
				false,
				crate::diagnostics::AmfSplit::Off,
			),
			encoder: Some(software),
			video_settings,
			i420: Vec::new(),
			settings,
			bitrate: settings.bit_rate(),
			adapter: None,
			epoch_base: 7,
			awaiting_keyframe: true,
			reset_generation: 0,
		};
		let raw = RawFrame {
			format: crate::screen::RawFormat::Bgra8,
			width: 2,
			height: 2,
			stride: 8,
			data: vec![128; 16],
		};
		encoder.restart().unwrap();
		encoder.restart().unwrap();
		assert_eq!(encoder.epoch(), 7);
		assert!(!encoder.set_bitrate(settings.bit_rate() / 2).unwrap());
		let first = encoder.encode_at(&raw, true, 6000).unwrap();
		assert!(first.keyframe);
		assert_eq!((first.epoch, first.timestamp), (7, 6000));
		encoder.restart().unwrap();
		encoder.restart().unwrap();
		assert_eq!(encoder.epoch(), 8);
		let recovered = encoder.encode_at(&raw, false, 12000).unwrap();
		assert!(recovered.keyframe);
		assert_eq!((recovered.epoch, recovered.timestamp), (8, 12000));
		assert!(crate::video_receive::has_parameter_sets(&recovered.data));
		encoder.restart().unwrap();
		assert!(encoder.awaiting_keyframe);
		assert_eq!(encoder.epoch(), 9);
		assert!(encoder.reset_for_security(1).unwrap());
		assert_eq!(encoder.epoch(), 10);
		assert_eq!(encoder.reset_generation, 1);
		assert!(!encoder.reset_for_security(1).unwrap());
		let fresh = encoder.encode_at(&raw, false, 18000).unwrap();
		assert_eq!((fresh.epoch, fresh.timestamp), (10, 18000));
		assert!(fresh.keyframe && crate::video_receive::has_parameter_sets(&fresh.data));
		use openh264::formats::YUVSource;
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		assert_eq!(
			decoder.decode(&fresh.data).unwrap().unwrap().dimensions(),
			(320, 240)
		);
		assert!(encoder.reset_for_security(u64::MAX).is_err());
	}

	#[test]
	fn idle_screen_keyframe_uses_latest_snapshot_only_when_ready() {
		let frame = |value| RawFrame {
			format: crate::screen::RawFormat::Bgra8,
			width: 2,
			height: 2,
			stride: 8,
			data: vec![value; 16],
		};
		let mut latest = None;
		assert!(!retain_screen_frame(&mut latest, None, true, true).unwrap());
		assert!(retain_screen_frame(&mut latest, Some(frame(1)), true, false).unwrap());
		assert!(!retain_screen_frame(&mut latest, None, true, false).unwrap());
		assert!(retain_screen_frame(&mut latest, None, true, true).unwrap());
		// A security pause keeps tracking the source without encoding. Its newest snapshot
		// can be freshly encoded after readiness, even if no further capture event arrives.
		assert!(!retain_screen_frame(&mut latest, Some(frame(2)), false, true).unwrap());
		assert!(!retain_screen_frame(&mut latest, None, false, true).unwrap());
		assert!(retain_screen_frame(&mut latest, None, true, true).unwrap());
		assert_eq!(latest.as_ref().unwrap().data, vec![2; 16]);
		let mut oversized = frame(3);
		oversized.width = model::voice_settings::VideoResolution::MAX_WIDTH + 1;
		assert!(retain_screen_frame(&mut latest, Some(oversized), true, true).is_err());
		assert_eq!(latest.unwrap().data, vec![2; 16]);
	}

	#[test]
	fn local_preview_is_bounded_and_converts_padded_bgra_without_media_readiness() {
		let preview = preview_frame(&RawFrame {
			format: crate::screen::RawFormat::Bgra8,
			width: 1,
			height: 2,
			stride: 8,
			data: vec![1, 2, 3, 0, 9, 9, 9, 9, 4, 5, 6, 0, 9, 9, 9, 9],
		})
		.unwrap();
		assert_eq!(preview.as_raw(), &[3, 2, 1, 255, 6, 5, 4, 255]);
		let worker = Worker {
			stop: Arc::new(AtomicBool::new(false)),
			ready: Arc::new(AtomicBool::new(false)),
			preview: Arc::new(Mutex::new(Some(preview))),
			done: None,
			#[cfg(target_os = "linux")]
			status: Arc::new(Mutex::new("")),
			#[cfg(target_os = "linux")]
			preview_visible: Arc::new(AtomicBool::new(true)),
		};
		assert!(worker.take_preview().is_some());
		assert!(worker.take_preview().is_none());
		assert!(!worker.ready.load(Ordering::Acquire));
		for (width, height) in [(3840, 2160), (2, 4320), (7680, 2)] {
			let frame = RawFrame {
				format: crate::screen::RawFormat::Bgra8,
				width,
				height,
				stride: width as usize * 4,
				data: vec![0; width as usize * height as usize * 4],
			};
			let preview = preview_frame(&frame).unwrap();
			assert!(preview.width() > 0 && preview.width() <= 640);
			assert!(preview.height() > 0 && preview.height() <= 360);
			assert!(preview.as_raw().len() <= 640 * 360 * 4);
		}
		assert!(
			preview_frame(&RawFrame {
				format: crate::screen::RawFormat::Bgra8,
				width: 2,
				height: 2,
				stride: 4,
				data: vec![0; 8],
			})
			.is_err()
		);
	}

	#[test]
	fn synthetic_frame_is_bounded_and_encodes_a_keyframe() {
		let frame = RawFrame {
			format: crate::screen::RawFormat::Bgra8,
			width: 2,
			height: 2,
			stride: 12,
			data: vec![255; 24],
		};
		let mut pixels = vec![0; 1280 * 720 * 3 / 2];
		convert::bgra_to_i420(&frame, 1280, 720, &mut pixels).unwrap();
		assert_eq!(pixels.len(), 1280 * 720 * 3 / 2);

		let settings = Settings {
			source: SourceId::Display(1),
			width: 1280,
			height: 720,
			fps: 30,
			cursor: true,
			audio: false,
		};
		let video_settings = VideoSettings::default();
		let encoder = crate::video_backend::Encoder::software(ScreenEncoder::config(
			settings,
			settings.bit_rate(),
			video_settings.codec,
		))
		.unwrap();
		let mut encoder = ScreenEncoder {
			diagnostics: crate::diagnostics::EncoderRegistration::new(
				true,
				false,
				crate::diagnostics::AmfSplit::Off,
			),
			encoder: Some(encoder),
			video_settings,
			i420: Vec::new(),
			settings,
			bitrate: settings.bit_rate(),
			adapter: None,
			epoch_base: 0,
			awaiting_keyframe: true,
			reset_generation: 0,
		};
		let raw = frame;
		let (encoded, keyframe) = encoder.encode(&raw, true).unwrap();
		assert!(keyframe);
		assert!(!encoded.is_empty() && encoded.len() <= MAX_ENCODED_BYTES);
		assert!(crate::video_receive::is_keyframe(&encoded));
		assert!(crate::video_receive::has_parameter_sets(&encoded));
		use openh264::formats::YUVSource;
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		assert_eq!(
			decoder.decode(&encoded).unwrap().unwrap().dimensions(),
			(1280, 720)
		);
		assert!(encoder.set_bitrate(settings.bit_rate() / 2).unwrap());
		let (encoded, keyframe) = encoder.encode(&raw, true).unwrap();
		assert!(keyframe && crate::video_receive::has_parameter_sets(&encoded));

		assert!(
			convert::bgra_to_i420(
				&RawFrame {
					format: crate::screen::RawFormat::Bgra8,
					width: 2,
					height: 2,
					stride: 4,
					data: vec![0; 8],
				},
				1280,
				720,
				&mut pixels,
			)
			.is_err()
		);
	}
}
