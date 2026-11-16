//! User-started, ephemeral camera capture. No capture stream starts before `start`.

use model::voice_settings::{
	CameraDenoisingStatus, VideoCodec, VideoFrameRate, VideoResolution, VideoSettings,
};
use openh264::formats::{RgbSliceU8, YUVBuffer, YUVSource};
use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread;

mod format;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;
pub const MAX_ENCODED_BYTES: usize = 2 * 1024 * 1024;

/// Keep the default camera's existing allocation budget; larger selected presets
/// earn progressively larger access units within the shared transport ceiling.
pub fn encoded_limit(resolution: VideoResolution) -> usize {
	match resolution {
		VideoResolution::P480 => 128 * 1024,
		VideoResolution::P720 => 256 * 1024,
		VideoResolution::P1080 => 512 * 1024,
		VideoResolution::P1440 => 1024 * 1024,
		VideoResolution::P2160 | VideoResolution::P4320 => MAX_ENCODED_BYTES,
	}
}

/// Shared bandwidth-first policy; FPS does not multiply the selected budget.
pub fn bit_rate(resolution: VideoResolution, _frame_rate: VideoFrameRate) -> u32 {
	bit_rates(resolution).average
}

pub fn bit_rates(resolution: VideoResolution) -> model::voice_settings::VideoBitrate {
	let (width, height) = resolution.camera_dimensions();
	model::voice_settings::VideoBitrate::for_dimensions(width, height)
}
pub const SUPPORTED: bool = cfg!(any(
	target_os = "macos",
	target_os = "windows",
	target_os = "linux"
));
// Includes asynchronous teardown: rapid toggles cannot accumulate camera workers.
static RUNNING: AtomicBool = AtomicBool::new(false);

pub struct Frame {
	/// Selected capture dimensions; the preview has its own smaller geometry.
	pub width: u32,
	pub height: u32,
	pub preview_width: u32,
	pub preview_height: u32,
	pub rgb: Vec<u8>,
	pub data: Vec<u8>,
	pub codec: VideoCodec,
	/// Presentation timestamp of the submitted picture, preserved across encode delay.
	pub timestamp: u32,
	pub keyframe: bool,
	pub epoch: u64,
	pub keyframe_request: Arc<AtomicBool>,
	/// Security resets must discard even an encoder's pending initial picture.
	pub reset: Arc<AtomicU64>,
	pub reset_generation: u64,
}

#[derive(Default)]
struct Shared {
	stopped: AtomicBool,
	active: AtomicBool,
	finished: AtomicBool,
	error: Mutex<Option<&'static str>>,
	keyframe_request: Arc<AtomicBool>,
	reset: Arc<AtomicU64>,
	preview_only: bool,
	capacity: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
	denoising_status: Arc<Mutex<CameraDenoisingStatus>>,
}

pub struct Camera {
	shared: Arc<Shared>,
}

/// Device identities and friendly labels, limited to 32 entries and 136 KiB total.
pub type DeviceList = Vec<(String, String)>;

/// Enumeration never opens a capture stream.
pub fn devices() -> Result<DeviceList, &'static str> {
	#[cfg(target_os = "windows")]
	{
		windows::devices()
	}
	#[cfg(target_os = "macos")]
	{
		objc2::rc::autoreleasepool(|_| macos::devices())
	}
	#[cfg(target_os = "linux")]
	{
		linux::devices()
	}
	#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
	{
		Err("Camera selection is unavailable on this platform")
	}
}

impl Camera {
	/// Call only after an explicit camera-on gesture in a call or settings preview.
	pub fn start(
		device: Option<String>,
		video: VideoSettings,
		on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
		wake: Arc<dyn Fn() + Send + Sync>,
	) -> Result<Self, &'static str> {
		Self::start_on_adapter(device, video, None, on_frame, wake)
	}

	pub fn start_on_adapter(
		device: Option<String>,
		video: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
		wake: Arc<dyn Fn() + Send + Sync>,
	) -> Result<Self, &'static str> {
		Self::start_on_adapter_with_capacity(
			device,
			video,
			adapter,
			on_frame,
			wake,
			Arc::new(|| true),
		)
	}

	pub fn start_on_adapter_with_capacity(
		device: Option<String>,
		video: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
		wake: Arc<dyn Fn() + Send + Sync>,
		capacity: Arc<dyn Fn() -> bool + Send + Sync>,
	) -> Result<Self, &'static str> {
		Self::start_worker(
			device,
			video,
			adapter,
			on_frame,
			wake,
			Shared {
				capacity: Some(capacity),
				..Shared::default()
			},
		)
	}

	/// Explicit local preview with denoising, without a video encoder or transport.
	pub fn preview(
		device: Option<String>,
		video: VideoSettings,
		on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
		wake: Arc<dyn Fn() + Send + Sync>,
	) -> Result<Self, &'static str> {
		Self::start_worker(
			device,
			video,
			None,
			on_frame,
			wake,
			Shared {
				preview_only: true,
				..Shared::default()
			},
		)
	}

	fn start_worker(
		device: Option<String>,
		video: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
		wake: Arc<dyn Fn() + Send + Sync>,
		shared: Shared,
	) -> Result<Self, &'static str> {
		if !SUPPORTED {
			return Err("Camera capture is unavailable on this platform");
		}
		if device
			.as_ref()
			.is_some_and(|id| id.len() > 4096 || id.contains('\0'))
		{
			return Err("Invalid camera device selection");
		}
		if RUNNING
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.is_err()
		{
			return Err("Previous camera session is still closing; try again shortly");
		}
		let shared = Arc::new(shared);
		let worker = shared.clone();
		if thread::Builder::new()
			.name("serein-camera".into())
			.spawn(move || {
				let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
					#[cfg(target_os = "macos")]
					{
						objc2::rc::autoreleasepool(|_| {
							macos::run(&worker, device.as_deref(), video, adapter, &on_frame, &wake)
						})
					}
					#[cfg(any(target_os = "windows", target_os = "linux"))]
					{
						run(&worker, device.as_deref(), video, adapter, &on_frame, &wake)
					}
					#[cfg(not(any(
						target_os = "macos",
						target_os = "windows",
						target_os = "linux"
					)))]
					Err("Camera capture is unavailable on this platform")
				}))
				.unwrap_or(Err("Camera worker failed"));
				worker.active.store(false, Ordering::Release);
				if !worker.stopped.load(Ordering::Acquire)
					&& let Err(error) = result
					&& let Ok(mut slot) = worker.error.lock()
				{
					*slot = Some(error);
				}
				worker.finished.store(true, Ordering::Release);
				RUNNING.store(false, Ordering::Release);
				wake();
			})
			.is_err()
		{
			RUNNING.store(false, Ordering::Release);
			return Err("Camera worker could not start");
		}
		Ok(Self { shared })
	}

	pub fn stop(&self) {
		self.shared.stopped.store(true, Ordering::Release);
		self.shared.active.store(false, Ordering::Release);
	}

	pub fn request_keyframe(&self) {
		self.shared.keyframe_request.store(true, Ordering::Release);
	}

	pub fn stopped(&self) -> bool {
		self.shared.finished.load(Ordering::Acquire)
	}

	pub fn error(&self) -> Option<&'static str> {
		*self.shared.error.lock().ok()?
	}

	pub fn denoising_status(&self) -> CameraDenoisingStatus {
		self.shared
			.denoising_status
			.lock()
			.map_or(CameraDenoisingStatus::Unavailable, |status| *status)
	}

	pub fn active(&self) -> bool {
		!self.shared.stopped.load(Ordering::Acquire) && self.shared.active.load(Ordering::Acquire)
	}
}

fn new_camera_denoiser(
	width: usize,
	height: usize,
) -> (
	Option<platform::video::denoise::CameraDenoiser>,
	CameraDenoisingStatus,
) {
	if !platform::video::denoise::CameraDenoiser::supports_dimensions(width, height) {
		return (None, CameraDenoisingStatus::ResolutionUnsupported);
	}
	let filter = platform::video::denoise::CameraDenoiser::new(width, height);
	let status = if filter.is_some() {
		CameraDenoisingStatus::Active
	} else {
		CameraDenoisingStatus::Unavailable
	};
	(filter, status)
}

struct CameraEncoding {
	diagnostics: crate::diagnostics::EncoderRegistration,
	encoder: crate::video_encode::Encoder,
}

struct CameraEncoder {
	encoding: Option<CameraEncoding>,
	codec: VideoCodec,
	yuv: YUVBuffer,
	i420: Vec<u8>,
	dimensions: (usize, usize),
	origin: std::time::Instant,
	keyframe_request: Arc<AtomicBool>,
	awaiting_keyframe: bool,
	reset: Arc<AtomicU64>,
	reset_generation: u64,
	denoiser: Option<platform::video::denoise::CameraDenoiser>,
	denoising_status: Arc<Mutex<CameraDenoisingStatus>>,
	frame_budget: std::time::Duration,
	denoising_slow_frames: u8,
	denoising_paused: bool,
	wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl CameraEncoder {
	fn config(video: VideoSettings) -> crate::video_encode::Config {
		let (width, height) = video.camera_resolution.camera_dimensions();
		crate::video_encode::Config {
			width,
			height,
			fps: video.camera_frame_rate.fps(),
			rates: video
				.bandwidth
				.rates(width, height, video.camera_frame_rate.fps()),
			preset: video.quality,
			max_bytes: encoded_limit(video.camera_resolution),
			profile: crate::video_encode::Profile::Main,
			codec: video.codec,
			adapter: None,
			hdr: None,
		}
	}

	#[cfg(test)]
	fn new(video: VideoSettings) -> Result<Self, &'static str> {
		Self::new_on_adapter(
			video,
			None,
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicU64::new(0)),
		)
	}

	#[cfg(test)]
	fn new_on_adapter(
		video: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		keyframe_request: Arc<AtomicBool>,
		reset: Arc<AtomicU64>,
	) -> Result<Self, &'static str> {
		Self::new_processing(video, adapter, keyframe_request, reset, false)
	}

	fn for_capture(
		shared: &Shared,
		video: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		wake: Arc<dyn Fn() + Send + Sync>,
	) -> Result<Self, &'static str> {
		let mut processor = Self::new_processing(
			video,
			adapter,
			shared.keyframe_request.clone(),
			shared.reset.clone(),
			shared.preview_only,
		)?;
		processor.connect_status(shared.denoising_status.clone(), wake);
		Ok(processor)
	}

	fn new_processing(
		video: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		keyframe_request: Arc<AtomicBool>,
		reset: Arc<AtomicU64>,
		preview_only: bool,
	) -> Result<Self, &'static str> {
		let mut config = Self::config(video);
		config.adapter = adapter;
		let (width, height) = (config.width as usize, config.height as usize);
		let encoding = if preview_only {
			None
		} else {
			let encoder = crate::video_encode::Encoder::new(config)?;
			Some(CameraEncoding {
				diagnostics: crate::diagnostics::EncoderRegistration::new(
					false,
					encoder.hardware(),
					encoder.amf_split(),
				),
				encoder,
			})
		};
		let (denoiser, status) = match video.camera_denoising {
			model::voice_settings::CameraDenoising::Off => (None, CameraDenoisingStatus::Off),
			model::voice_settings::CameraDenoising::On => new_camera_denoiser(width, height),
		};
		Ok(Self {
			encoding,
			codec: video.codec,
			yuv: YUVBuffer::new(width, height),
			i420: Vec::with_capacity(width * height * 3 / 2),
			dimensions: (width, height),
			origin: std::time::Instant::now(),
			keyframe_request,
			awaiting_keyframe: true,
			reset,
			reset_generation: 0,
			denoiser,
			denoising_status: Arc::new(Mutex::new(status)),
			frame_budget: std::time::Duration::from_secs_f64(
				1.0 / video.camera_frame_rate.fps() as f64,
			),
			denoising_slow_frames: 0,
			denoising_paused: false,
			wake: None,
		})
	}

	fn connect_status(
		&mut self,
		target: Arc<Mutex<CameraDenoisingStatus>>,
		wake: Arc<dyn Fn() + Send + Sync>,
	) {
		let status = self
			.denoising_status
			.lock()
			.map_or(CameraDenoisingStatus::Unavailable, |status| *status);
		if let Ok(mut slot) = target.lock() {
			*slot = status;
		}
		self.denoising_status = target;
		wake();
		self.wake = Some(wake);
	}

	fn disable_denoising(&mut self, status: CameraDenoisingStatus) {
		self.denoiser = None;
		if let Ok(mut slot) = self.denoising_status.lock() {
			*slot = status;
		}
		if let Some(wake) = &self.wake {
			wake();
		}
	}

	// Do not let buffered temporal pictures survive arbitrarily long congestion.
	fn pause_denoising(&mut self) {
		if self.denoiser.take().is_some() {
			self.denoising_paused = true;
		}
	}

	fn encode(&mut self, rgb: Vec<u8>) -> Result<Option<Frame>, &'static str> {
		let (width, height) = self.dimensions;
		if rgb.len() != width * height * 3 {
			return Err("Camera did not provide the selected bounded RGB frame size");
		}
		let reset = self.reset.load(Ordering::Acquire);
		if reset == u64::MAX {
			return Err("Camera security reset generation exhausted");
		}
		if reset != self.reset_generation {
			// Destroy the temporal window before accepting a post-reset capture.
			if self.denoiser.take().is_some() {
				let (filter, status) = new_camera_denoiser(width, height);
				self.denoiser = filter;
				if self.denoiser.is_none() {
					self.disable_denoising(status);
				}
			}
			self.denoising_slow_frames = 0;
			if let Some(encoding) = &mut self.encoding {
				encoding.encoder.restart()?;
			}
			self.reset_generation = reset;
			self.awaiting_keyframe = true;
			// Capture can span the reset. The following input must be newly captured.
			return Ok(None);
		}
		if self.denoising_paused {
			self.denoising_paused = false;
			self.denoising_slow_frames = 0;
			// Retire encoder-delayed pictures, if any, along with filter history.
			if let Some(encoding) = &mut self.encoding
				&& encoding.encoder.pending()
			{
				encoding.encoder.restart()?;
				self.awaiting_keyframe = true;
			}
			let (filter, status) = new_camera_denoiser(width, height);
			self.denoiser = filter;
			if self.denoiser.is_none() {
				self.disable_denoising(status);
			}
		}
		if self.encoding.is_none() && self.denoiser.is_none() {
			return self.preview(rgb).map(Some);
		}
		self.yuv.read_rgb8(RgbSliceU8::new(&rgb, (width, height)));
		self.i420.clear();
		self.i420.extend_from_slice(self.yuv.y());
		self.i420.extend_from_slice(self.yuv.u());
		self.i420.extend_from_slice(self.yuv.v());
		let capture_timestamp = (self.origin.elapsed().as_micros() * 90 / 1000) as i64;
		let mut filtered = false;
		let mut timestamp = capture_timestamp;
		if let Some(denoiser) = &mut self.denoiser {
			let start = std::time::Instant::now();
			let result = denoiser.process(&mut self.i420, capture_timestamp);
			if denoising_too_slow(
				&mut self.denoising_slow_frames,
				start.elapsed(),
				self.frame_budget,
			) {
				self.disable_denoising(CameraDenoisingStatus::TooSlow);
			}
			match result {
				Ok(Some(pts)) => {
					filtered = true;
					timestamp = pts;
				}
				Ok(None) if self.denoiser.is_some() => return Ok(None),
				Ok(None) => {}
				Err(_) => self.disable_denoising(CameraDenoisingStatus::Unavailable),
			}
		}
		if let Some(encoding) = &mut self.encoding {
			// Coalesce feedback while lookahead is still producing the first IDR.
			if self.keyframe_request.swap(false, Ordering::AcqRel) && !self.awaiting_keyframe {
				encoding.encoder.restart()?;
				self.awaiting_keyframe = true;
			}
			let packet = encoding
				.encoder
				.encode_at(&self.i420, false, timestamp as u32)?;
			encoding.diagnostics.set(Some((
				encoding.encoder.hardware(),
				encoding.encoder.amf_split(),
			)));
			if filtered {
				let preview = self.filtered_preview()?;
				self.finish_packet_with_preview(preview, packet).map(Some)
			} else {
				self.finish_packet(rgb, packet).map(Some)
			}
		} else {
			let mut preview = if filtered {
				self.filtered_preview()?
			} else {
				self.preview(rgb)?
			};
			preview.timestamp = timestamp as u32;
			Ok(Some(preview))
		}
	}

	fn finish_packet(
		&mut self,
		rgb: Vec<u8>,
		packet: crate::video_encode::EncodedPacket,
	) -> Result<Frame, &'static str> {
		let frame = self.preview(rgb)?;
		self.finish_packet_with_preview(frame, packet)
	}

	fn finish_packet_with_preview(
		&mut self,
		mut frame: Frame,
		packet: crate::video_encode::EncodedPacket,
	) -> Result<Frame, &'static str> {
		if !packet.data.is_empty() {
			if self.awaiting_keyframe && !packet.keyframe {
				return Err("Camera encoder did not restart at an independently decodable picture");
			}
			self.awaiting_keyframe = false;
		}
		// Lookahead delays media output, but the current capture is ready for local preview.
		frame.keyframe = !packet.data.is_empty() && packet.keyframe;
		frame.data = packet.data;
		frame.timestamp = packet.timestamp;
		frame.epoch = packet.epoch;
		Ok(frame)
	}

	fn preview(&self, rgb: Vec<u8>) -> Result<Frame, &'static str> {
		let (width, height) = self.dimensions;
		let (rgb, preview_width, preview_height) = preview_rgb(rgb, width, height)?;
		Ok(self.preview_frame(rgb, preview_width, preview_height))
	}

	fn filtered_preview(&self) -> Result<Frame, &'static str> {
		let (width, height) = self.dimensions;
		let (rgb, preview_width, preview_height) = preview_i420(&self.i420, width, height)?;
		Ok(self.preview_frame(rgb, preview_width, preview_height))
	}

	fn preview_frame(&self, rgb: Vec<u8>, preview_width: u32, preview_height: u32) -> Frame {
		let (width, height) = self.dimensions;
		Frame {
			width: width as u32,
			height: height as u32,
			preview_width,
			preview_height,
			rgb,
			data: Vec::new(),
			codec: self.codec,
			timestamp: (self.origin.elapsed().as_micros() * 90 / 1000) as u32,
			keyframe: false,
			epoch: self
				.encoding
				.as_ref()
				.map_or(0, |encoding| encoding.encoder.epoch()),
			keyframe_request: self.keyframe_request.clone(),
			reset: self.reset.clone(),
			reset_generation: self.reset_generation,
		}
	}
}

// A single slow sample can be scheduler noise; sustained overload bypasses the filter.
fn denoising_too_slow(
	streak: &mut u8,
	elapsed: std::time::Duration,
	budget: std::time::Duration,
) -> bool {
	if elapsed > budget {
		*streak = streak.saturating_add(1);
	} else {
		*streak = 0;
	}
	*streak >= 3
}

/// Only the worker sees full-resolution RGB; UI queues retain a small preview.
fn preview_rgb(
	rgb: Vec<u8>,
	width: usize,
	height: usize,
) -> Result<(Vec<u8>, u32, u32), &'static str> {
	if width == 0
		|| height == 0
		|| width > format::MAX_CAPTURE_WIDTH
		|| height > format::MAX_CAPTURE_HEIGHT
		|| width
			.checked_mul(height)
			.and_then(|pixels| pixels.checked_mul(3))
			!= Some(rgb.len())
	{
		return Err("Camera preview dimensions exceed bounds");
	}
	let scale = (WIDTH as f64 / width as f64)
		.min(HEIGHT as f64 / height as f64)
		.min(1.0);
	let preview_width = ((width as f64 * scale).round() as usize).clamp(1, WIDTH);
	let preview_height = ((height as f64 * scale).round() as usize).clamp(1, HEIGHT);
	if (preview_width, preview_height) == (width, height) {
		return Ok((rgb, width as u32, height as u32));
	}
	let mut preview = vec![0; preview_width * preview_height * 3];
	for y in 0..preview_height {
		for x in 0..preview_width {
			let source = (y * height / preview_height * width + x * width / preview_width) * 3;
			let target = (y * preview_width + x) * 3;
			preview[target..target + 3].copy_from_slice(&rgb[source..source + 3]);
		}
	}
	Ok((preview, preview_width as u32, preview_height as u32))
}

// Convert only the small preview from the filtered picture, avoiding another
// full-resolution RGB buffer. Camera RGB conversion uses limited-range I420.
fn preview_i420(
	data: &[u8],
	width: usize,
	height: usize,
) -> Result<(Vec<u8>, u32, u32), &'static str> {
	let pixels = width
		.checked_mul(height)
		.ok_or("Camera preview dimensions exceed bounds")?;
	if width == 0
		|| height == 0
		|| !width.is_multiple_of(2)
		|| !height.is_multiple_of(2)
		|| pixels.checked_mul(3).map(|size| size / 2) != Some(data.len())
	{
		return Err("Camera preview dimensions exceed bounds");
	}
	let scale = (WIDTH as f64 / width as f64)
		.min(HEIGHT as f64 / height as f64)
		.min(1.0);
	let pw = ((width as f64 * scale).round() as usize).clamp(1, WIDTH);
	let ph = ((height as f64 * scale).round() as usize).clamp(1, HEIGHT);
	let mut preview = vec![0; pw * ph * 3];
	for y in 0..ph {
		for x in 0..pw {
			let sx = x * width / pw;
			let sy = y * height / ph;
			let yv = i32::from(data[sy * width + sx]) - 16;
			let chroma = (sy / 2) * (width / 2) + sx / 2;
			let u = i32::from(data[pixels + chroma]) - 128;
			let v = i32::from(data[pixels + pixels / 4 + chroma]) - 128;
			let pixel = &mut preview[(y * pw + x) * 3..][..3];
			pixel[0] = ((298 * yv + 409 * v + 128) >> 8).clamp(0, 255) as u8;
			pixel[1] = ((298 * yv - 100 * u - 208 * v + 128) >> 8).clamp(0, 255) as u8;
			pixel[2] = ((298 * yv + 516 * u + 128) >> 8).clamp(0, 255) as u8;
		}
	}
	Ok((preview, pw as u32, ph as u32))
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn run(
	shared: &Shared,
	device: Option<&str>,
	video: VideoSettings,
	adapter: Option<model::VideoAdapter>,
	on_frame: &Arc<dyn Fn(Frame) + Send + Sync>,
	wake: &Arc<dyn Fn() + Send + Sync>,
) -> Result<(), &'static str> {
	let mut encoder = CameraEncoder::for_capture(shared, video, adapter, wake.clone())?;
	let mut emit = |rgb| emit_frame(shared, &mut encoder, rgb, on_frame, wake);
	#[cfg(target_os = "windows")]
	{
		windows::run(
			shared,
			device,
			video.camera_resolution,
			video.camera_frame_rate,
			&mut emit,
		)
	}
	#[cfg(target_os = "linux")]
	{
		linux::run(
			shared,
			device,
			video.camera_resolution,
			video.camera_frame_rate,
			&mut emit,
		)
	}
}

fn emit_frame(
	shared: &Shared,
	encoder: &mut CameraEncoder,
	rgb: Vec<u8>,
	on_frame: &Arc<dyn Fn(Frame) + Send + Sync>,
	wake: &Arc<dyn Fn() + Send + Sync>,
) -> Result<(), &'static str> {
	if shared.stopped.load(Ordering::Acquire) {
		return Ok(());
	}
	let frame = if shared.preview_only || shared.capacity.as_ref().is_none_or(|capacity| capacity())
	{
		encoder.encode(rgb)?
	} else if encoder.denoiser.is_some() || encoder.denoising_paused {
		encoder.pause_denoising();
		None
	} else {
		Some(encoder.preview(rgb)?)
	};
	if let Some(frame) = frame
		&& !shared.stopped.load(Ordering::Acquire)
	{
		on_frame(frame);
		shared.active.store(true, Ordering::Release);
		wake();
	}
	Ok(())
}

impl Drop for Camera {
	fn drop(&mut self) {
		self.stop();
	}
}

#[cfg(target_os = "macos")]
mod macos {
	#![allow(unsafe_code)]

	use super::*;
	use block2::RcBlock;
	use dispatch2::{DispatchQueue, DispatchRetained};
	use objc2::{
		AnyThread, DefinedClass, define_class, msg_send,
		rc::Retained,
		runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject},
	};
	use objc2_av_foundation::*;
	use objc2_core_media::{CMSampleBuffer, CMTime, CMVideoFormatDescriptionGetDimensions};
	use objc2_core_video::*;
	use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
	use std::{
		sync::mpsc::{self, Receiver, SyncSender},
		time::{Duration, Instant},
	};

	const DENIED: &str = "Camera access denied. Allow NuSerein (or your terminal) in System Settings > Privacy & Security > Camera, then try again.";

	pub(super) fn devices() -> Result<DeviceList, &'static str> {
		// SAFETY: Framework-owned device types and discovery only; no stream or permission request.
		unsafe {
			let types = NSArray::from_slice(&[
				AVCaptureDeviceTypeBuiltInWideAngleCamera,
				AVCaptureDeviceTypeExternal,
				AVCaptureDeviceTypeContinuityCamera,
				AVCaptureDeviceTypeDeskViewCamera,
			]);
			let discovery =
				AVCaptureDeviceDiscoverySession::discoverySessionWithDeviceTypes_mediaType_position(
					&types,
					Some(AVMediaTypeVideo.ok_or("Camera media type unavailable")?),
					AVCaptureDevicePosition::Unspecified,
				);
			Ok(discovery
				.devices()
				.iter()
				.take(32)
				.filter_map(|device| {
					let id = device.uniqueID();
					let name = device.localizedName();
					if id.length() > 4096 || name.length() > 256 {
						return None;
					}
					let (id, name) = (id.to_string(), name.to_string());
					(id.len() <= 4096 && name.len() <= 256 && !id.contains('\0'))
						.then_some((id, name))
				})
				.collect())
		}
	}

	struct DelegateState {
		send: SyncSender<Result<Vec<u8>, &'static str>>,
		shared: Arc<Shared>,
		cadence: Mutex<format::Cadence>,
		dimensions: (usize, usize),
	}

	define_class!(
		// SAFETY: NSObject superclass, initialized Send + Sync ivars, and the
		// exact AVFoundation delegate signature. AVFoundation uses a serial queue.
		#[unsafe(super = NSObject)]
		#[ivars = DelegateState]
		struct SereinCameraDelegate;

		unsafe impl NSObjectProtocol for SereinCameraDelegate {}
		unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for SereinCameraDelegate {
			#[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
			fn capture(
				&self,
				_output: &AVCaptureOutput,
				sample: &CMSampleBuffer,
				_connection: &AVCaptureConnection,
			) {
				let state = self.ivars();
				if state.shared.stopped.load(Ordering::Acquire) {
					return;
				}
				let Ok(mut cadence) = state.cadence.try_lock() else {
					return;
				};
				if !cadence.accept(Instant::now()) {
					return;
				}
				let _ = state.send.try_send(copy_bgra(sample, state.dimensions));
			}
		}
	);

	fn copy_bgra(
		sample: &CMSampleBuffer,
		dimensions: (usize, usize),
	) -> Result<Vec<u8>, &'static str> {
		let (width, height) = dimensions;
		let budget = format::raw_budget(width, height).ok_or("Camera frame exceeds bounds")?;
		// SAFETY: The sample is valid for this delegate invocation. Retain its image,
		// verify packed BGRA dimensions/stride before reading, and unlock every path.
		unsafe {
			let pixels = sample.image_buffer().ok_or("Camera returned no image")?;
			let stride = CVPixelBufferGetBytesPerRow(&pixels);
			let bytes = stride
				.checked_mul(height)
				.ok_or("Camera frame exceeds bounds")?;
			if CVPixelBufferGetWidth(&pixels) != width
				|| CVPixelBufferGetHeight(&pixels) != height
				|| CVPixelBufferGetPixelFormatType(&pixels) != kCVPixelFormatType_32BGRA
				|| !(width * 4..=width * 4 + format::MAX_STRIDE_PADDING).contains(&stride)
				|| bytes > budget
				|| bytes > CVPixelBufferGetDataSize(&pixels)
			{
				return Err("Camera did not provide a selected bounded BGRA frame");
			}
			if CVPixelBufferLockBaseAddress(&pixels, CVPixelBufferLockFlags::ReadOnly) != 0 {
				return Err("Camera image could not be read");
			}
			let base = CVPixelBufferGetBaseAddress(&pixels);
			let result = if base.is_null() {
				Err("Camera returned an empty image")
			} else {
				let source = std::slice::from_raw_parts(base.cast::<u8>(), bytes);
				let mut bgra = vec![0; width * height * 4];
				for (row, dest) in source
					.chunks_exact(stride)
					.zip(bgra.chunks_exact_mut(width * 4))
				{
					dest.copy_from_slice(&row[..width * 4]);
				}
				Ok(bgra)
			};
			CVPixelBufferUnlockBaseAddress(&pixels, CVPixelBufferLockFlags::ReadOnly);
			result
		}
	}

	fn authorize(shared: &Shared) -> Result<(), &'static str> {
		// SAFETY: Framework-owned constant, class methods callable from this worker;
		// AVFoundation copies the block, which owns only a bounded result sender.
		let receive = unsafe {
			let media = AVMediaTypeVideo.ok_or("macOS camera authorization is unavailable")?;
			match AVCaptureDevice::authorizationStatusForMediaType(media) {
				AVAuthorizationStatus::Authorized => return Ok(()),
				AVAuthorizationStatus::NotDetermined => {
					let (send, receive) = mpsc::sync_channel(1);
					let block = RcBlock::new(move |granted: Bool| {
						let _ = send.try_send(granted.as_bool());
					});
					AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &block);
					receive
				}
				_ => return Err(DENIED),
			}
		};
		let deadline = Instant::now() + Duration::from_secs(20);
		while !shared.stopped.load(Ordering::Acquire) && Instant::now() < deadline {
			match receive.recv_timeout(Duration::from_millis(100)) {
				Ok(true) => return Ok(()),
				Ok(false) => return Err(DENIED),
				Err(mpsc::RecvTimeoutError::Disconnected) => {
					return Err("Camera permission request failed");
				}
				Err(mpsc::RecvTimeoutError::Timeout) => {}
			}
		}
		Err("Camera permission canceled or timed out; respond to the macOS prompt and try again")
	}

	struct CaptureSession {
		session: Retained<AVCaptureSession>,
		output: Retained<AVCaptureVideoDataOutput>,
		_delegate: Retained<SereinCameraDelegate>,
		queue: DispatchRetained<DispatchQueue>,
	}

	fn configure(
		device: &AVCaptureDevice,
		dimensions: (usize, usize),
		target_fps: u32,
	) -> Result<(), &'static str> {
		// SAFETY: Formats/ranges come from this device. Only this worker changes it,
		// while locked, before capture starts. Endpoint durations stay exact.
		unsafe {
			let mut selected = None;
			for native in device.formats().iter().take(256) {
				let size = CMVideoFormatDescriptionGetDimensions(&native.formatDescription());
				for range in native.videoSupportedFrameRateRanges().iter().take(256) {
					let (min, max) = (range.minFrameRate(), range.maxFrameRate());
					let Some(fps) = format::nearest_fps_for(min, max, target_fps) else {
						continue;
					};
					let Some(rank) = format::rank_for_output_at_rate(
						size.width as usize,
						size.height as usize,
						fps,
						dimensions,
						target_fps,
					) else {
						continue;
					};
					let duration = if fps == min {
						range.maxFrameDuration()
					} else if fps == max {
						range.minFrameDuration()
					} else {
						CMTime::new(1, target_fps as i32)
					};
					if selected.as_ref().is_none_or(|(best, _, _)| rank < *best) {
						selected = Some((rank, native.clone(), duration));
					}
				}
			}
			let (_, native, duration) =
				selected.ok_or("Camera does not support the selected resolution")?;
			device
				.lockForConfiguration()
				.map_err(|_| "Camera configuration is unavailable")?;
			device.setActiveFormat(&native);
			device.setActiveVideoMinFrameDuration(duration);
			device.setActiveVideoMaxFrameDuration(duration);
			device.unlockForConfiguration();
		}
		Ok(())
	}
	impl Drop for CaptureSession {
		fn drop(&mut self) {
			// SAFETY: Owned session is configured, and teardown runs on its worker.
			unsafe {
				self.output.setSampleBufferDelegate_queue(None, None);
				self.session.stopRunning();
			}
			self.queue.exec_sync(|| {});
		}
	}

	pub(super) fn run(
		shared: &Arc<Shared>,
		selected: Option<&str>,
		video: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		on_frame: &Arc<dyn Fn(Frame) + Send + Sync>,
		wake: &Arc<dyn Fn() + Send + Sync>,
	) -> Result<(), &'static str> {
		authorize(shared)?;
		if shared.stopped.load(Ordering::Acquire) {
			return Ok(());
		}
		let mut encoder = CameraEncoder::for_capture(shared, video, adapter, wake.clone())?;
		let target_fps = video.camera_frame_rate.fps();
		let cadence =
			format::Cadence::new(target_fps, Instant::now()).ok_or("Invalid camera frame rate")?;
		let (send, receive) = mpsc::sync_channel(1);
		let queue = DispatchQueue::new("serein.camera.frames", None);
		// SAFETY: Only this worker configures/owns the session. Delegate lives until
		// capture is stopped and the serial callback queue has drained.
		let capture = unsafe {
			let media = AVMediaTypeVideo.ok_or("Camera media type unavailable")?;
			let device = match selected {
				Some(id) => AVCaptureDevice::deviceWithUniqueID(&NSString::from_str(id))
					.filter(|device| device.hasMediaType(media))
					.ok_or(
						"Selected camera is disconnected or unavailable. Refresh cameras and choose another device.",
					)?,
				None => AVCaptureDevice::defaultDeviceWithMediaType(media)
					.ok_or("No camera is available")?,
			};
			let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
				.map_err(|_| "Camera is busy or unavailable")?;
			let session = AVCaptureSession::new();
			let output = AVCaptureVideoDataOutput::new();
			if !session.canAddInput(&input) || !session.canAddOutput(&output) {
				return Err("Camera cannot join capture session");
			}
			session.addInput(&input);
			session.addOutput(&output);
			if !session.canSetSessionPreset(AVCaptureSessionPresetInputPriority) {
				return Err("Camera does not support native format selection");
			}
			session.beginConfiguration();
			session.setSessionPreset(AVCaptureSessionPresetInputPriority);
			let configured = configure(&device, encoder.dimensions, target_fps);
			session.commitConfiguration();
			configured?;
			let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
			let width = NSNumber::new_usize(encoder.dimensions.0);
			let height = NSNumber::new_usize(encoder.dimensions.1);
			let format_key = NSString::from_str(&kCVPixelBufferPixelFormatTypeKey.to_string());
			let width_key = NSString::from_str(&kCVPixelBufferWidthKey.to_string());
			let height_key = NSString::from_str(&kCVPixelBufferHeightKey.to_string());
			let scaling_key =
				AVVideoScalingModeKey.ok_or("Camera aspect scaling is unavailable")?;
			let scaling =
				AVVideoScalingModeResizeAspect.ok_or("Camera aspect scaling is unavailable")?;
			// Fix output dimensions and preserve nonmatching native aspect ratios.
			// AVFoundation performs this conversion before the delegate callback.
			let settings = NSDictionary::from_slices(
				&[&*format_key, &*width_key, &*height_key, scaling_key],
				&[&*format as &AnyObject, &*width, &*height, scaling],
			);
			output.setVideoSettings(Some(&settings));
			output.setAlwaysDiscardsLateVideoFrames(true);
			let allocated = SereinCameraDelegate::alloc().set_ivars(DelegateState {
				send,
				shared: shared.clone(),
				cadence: Mutex::new(cadence),
				dimensions: encoder.dimensions,
			});
			let delegate: Retained<SereinCameraDelegate> = msg_send![super(allocated), init];
			output.setSampleBufferDelegate_queue(
				Some(ProtocolObject::from_ref(&*delegate)),
				Some(&queue),
			);
			let capture = CaptureSession {
				session,
				output,
				_delegate: delegate,
				queue,
			};
			if !shared.stopped.load(Ordering::Acquire) {
				capture.session.startRunning();
			}
			capture
		};
		let result = encode_loop(shared, on_frame, wake, &receive, &mut encoder);
		drop(capture);
		result
	}

	fn encode_loop(
		shared: &Shared,
		on_frame: &Arc<dyn Fn(Frame) + Send + Sync>,
		wake: &Arc<dyn Fn() + Send + Sync>,
		receive: &Receiver<Result<Vec<u8>, &'static str>>,
		encoder: &mut CameraEncoder,
	) -> Result<(), &'static str> {
		let mut last_frame = Instant::now();
		while !shared.stopped.load(Ordering::Acquire) {
			let bgra = match receive.recv_timeout(Duration::from_millis(100)) {
				Ok(frame) => frame?,
				Err(mpsc::RecvTimeoutError::Timeout)
					if last_frame.elapsed() < Duration::from_secs(5) =>
				{
					continue;
				}
				_ => {
					return Err("Camera stopped delivering frames; check the device and try again");
				}
			};
			last_frame = Instant::now();
			let mut rgb = vec![0; encoder.dimensions.0 * encoder.dimensions.1 * 3];
			for (bgra, rgb) in bgra
				.as_chunks::<4>()
				.0
				.iter()
				.zip(rgb.as_chunks_mut::<3>().0)
			{
				rgb.copy_from_slice(&[bgra[2], bgra[1], bgra[0]]);
			}
			emit_frame(shared, encoder, rgb, on_frame, wake)?;
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn local_preview(video: VideoSettings) -> (Shared, CameraEncoder) {
		let shared = Shared {
			preview_only: true,
			capacity: Some(Arc::new(|| {
				panic!("Local preview has no transport capacity gate")
			})),
			..Shared::default()
		};
		let processor = CameraEncoder::for_capture(
			&shared,
			video,
			Some(model::VideoAdapter::default()),
			Arc::new(|| {}),
		)
		.unwrap();
		(shared, processor)
	}

	#[test]
	fn local_preview_does_not_initialize_the_selected_hardware_encoder() {
		for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1] {
			let video = VideoSettings {
				codec,
				camera_resolution: VideoResolution::P720,
				..VideoSettings::default()
			};
			if codec != VideoCodec::H264 {
				assert!(
					CameraEncoder::new_on_adapter(
						video,
						Some(model::VideoAdapter::default()),
						Arc::new(AtomicBool::new(false)),
						Arc::new(AtomicU64::new(0)),
					)
					.is_err()
				);
			}
			let (shared, mut processor) = local_preview(video);
			assert!(processor.encoding.is_none());
			assert_eq!(
				*shared.denoising_status.lock().unwrap(),
				CameraDenoisingStatus::Off
			);
			processor.origin -= std::time::Duration::from_secs(1);
			shared.keyframe_request.store(true, Ordering::Release);
			let picture = Arc::new(Mutex::new(None));
			let target = picture.clone();
			let on_frame: Arc<dyn Fn(Frame) + Send + Sync> = Arc::new(move |frame| {
				*target.lock().unwrap() = Some(frame);
			});
			let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
			emit_frame(
				&shared,
				&mut processor,
				vec![127; 1280 * 720 * 3],
				&on_frame,
				&wake,
			)
			.unwrap();
			let frame = picture.lock().unwrap().take().unwrap();
			assert_eq!((frame.width, frame.height), (1280, 720));
			assert_eq!((frame.preview_width, frame.preview_height), (640, 360));
			assert_eq!(frame.rgb, vec![127; 640 * 360 * 3]);
			assert!(frame.data.is_empty() && !frame.keyframe);
			assert_eq!((frame.codec, frame.epoch), (codec, 0));
			assert!(frame.timestamp >= 90_000);
			assert!(processor.i420.is_empty());
			assert!(shared.active.load(Ordering::Acquire));
			assert!(shared.keyframe_request.load(Ordering::Acquire));
		}
	}

	#[test]
	fn local_preview_keeps_frame_bounds_cancellation_and_security_resets() {
		let (shared, mut processor) = local_preview(VideoSettings::default());
		let picture = Arc::new(Mutex::new(None));
		let target = picture.clone();
		let on_frame: Arc<dyn Fn(Frame) + Send + Sync> = Arc::new(move |frame| {
			*target.lock().unwrap() = Some(frame);
		});
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
		for length in [0, WIDTH * HEIGHT * 3 - 1, WIDTH * HEIGHT * 3 + 3] {
			assert!(
				emit_frame(&shared, &mut processor, vec![0; length], &on_frame, &wake).is_err()
			);
		}
		assert!(picture.lock().unwrap().is_none());
		assert!(!shared.active.load(Ordering::Acquire));
		shared.reset.store(1, Ordering::Release);
		emit_frame(
			&shared,
			&mut processor,
			vec![16; WIDTH * HEIGHT * 3],
			&on_frame,
			&wake,
		)
		.unwrap();
		assert!(picture.lock().unwrap().is_none());
		emit_frame(
			&shared,
			&mut processor,
			vec![240; WIDTH * HEIGHT * 3],
			&on_frame,
			&wake,
		)
		.unwrap();
		let fresh = picture.lock().unwrap().take().unwrap();
		assert_eq!(fresh.reset_generation, 1);
		assert!(fresh.rgb.iter().all(|value| *value == 240));
		assert!(fresh.data.is_empty());
		assert!(Arc::ptr_eq(&fresh.reset, &shared.reset));
		shared.reset.store(u64::MAX, Ordering::Release);
		assert!(
			emit_frame(
				&shared,
				&mut processor,
				vec![240; WIDTH * HEIGHT * 3],
				&on_frame,
				&wake,
			)
			.is_err()
		);
		shared.stopped.store(true, Ordering::Release);
		emit_frame(&shared, &mut processor, Vec::new(), &on_frame, &wake).unwrap();
		assert!(picture.lock().unwrap().is_none());
	}

	#[test]
	fn local_preview_keeps_denoising_fallback_status_without_an_encoder() {
		let (shared, mut processor) = local_preview(VideoSettings {
			codec: VideoCodec::H265,
			camera_resolution: VideoResolution::P1440,
			camera_denoising: model::voice_settings::CameraDenoising::On,
			..VideoSettings::default()
		});
		assert!(processor.encoding.is_none() && processor.denoiser.is_none());
		assert_eq!(
			*shared.denoising_status.lock().unwrap(),
			CameraDenoisingStatus::ResolutionUnsupported
		);
		let picture = Arc::new(Mutex::new(None));
		let target = picture.clone();
		let on_frame: Arc<dyn Fn(Frame) + Send + Sync> = Arc::new(move |frame| {
			*target.lock().unwrap() = Some(frame);
		});
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
		let (width, height) = processor.dimensions;
		emit_frame(
			&shared,
			&mut processor,
			vec![128; width * height * 3],
			&on_frame,
			&wake,
		)
		.unwrap();
		let frame = picture.lock().unwrap().take().unwrap();
		assert_eq!((frame.preview_width, frame.preview_height), (640, 360));
		assert_eq!(frame.rgb.len(), 640 * 360 * 3);
		assert!(frame.data.is_empty());
		assert!(processor.i420.is_empty());
	}

	#[test]
	fn local_preview_denoises_without_encoding_and_discards_old_temporal_frames() {
		if !offline_gpu() {
			return;
		}
		let (shared, mut processor) = local_preview(VideoSettings {
			codec: VideoCodec::Av1,
			camera_denoising: model::voice_settings::CameraDenoising::On,
			..VideoSettings::default()
		});
		assert!(processor.encoding.is_none());
		assert!(processor.denoiser.is_some());
		assert_eq!(
			*shared.denoising_status.lock().unwrap(),
			CameraDenoisingStatus::Active
		);
		processor.frame_budget = std::time::Duration::from_secs(60);
		processor.origin -= std::time::Duration::from_secs(1);
		let picture = Arc::new(Mutex::new(None));
		let target = picture.clone();
		let on_frame: Arc<dyn Fn(Frame) + Send + Sync> = Arc::new(move |frame| {
			*target.lock().unwrap() = Some(frame);
		});
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
		let mut newest_timestamp = 0;
		for _ in 0..10 {
			newest_timestamp = (processor.origin.elapsed().as_micros() * 90 / 1000) as u32;
			emit_frame(
				&shared,
				&mut processor,
				vec![16; WIDTH * HEIGHT * 3],
				&on_frame,
				&wake,
			)
			.unwrap();
			processor.origin -= std::time::Duration::from_millis(100);
		}
		let dark = picture.lock().unwrap().take().expect("filtered preview");
		assert!(dark.data.is_empty());
		assert!(dark.timestamp >= 90_000 && dark.timestamp < newest_timestamp);
		assert_eq!(
			dark.rgb,
			preview_i420(&processor.i420, WIDTH, HEIGHT).unwrap().0
		);
		shared.reset.store(1, Ordering::Release);
		emit_frame(
			&shared,
			&mut processor,
			vec![16; WIDTH * HEIGHT * 3],
			&on_frame,
			&wake,
		)
		.unwrap();
		assert!(picture.lock().unwrap().is_none());
		for _ in 0..10 {
			emit_frame(
				&shared,
				&mut processor,
				vec![240; WIDTH * HEIGHT * 3],
				&on_frame,
				&wake,
			)
			.unwrap();
			processor.origin -= std::time::Duration::from_millis(100);
		}
		let fresh = picture
			.lock()
			.unwrap()
			.take()
			.expect("fresh filtered preview");
		assert_eq!(fresh.reset_generation, 1);
		assert!(fresh.timestamp > dark.timestamp);
		assert!(fresh.data.is_empty() && !fresh.keyframe);
		assert!(fresh.rgb.iter().all(|value| *value > 190));
		assert_eq!(fresh.rgb.len(), WIDTH * HEIGHT * 3);
	}

	fn offline_gpu() -> bool {
		let available = platform::video::denoise::install_test_gpu();
		if !available {
			eprintln!("No offline GPU adapter is available for the camera fixture");
		}
		available
	}

	#[test]
	fn camera_preview_does_not_advance_encoding_and_recovery_starts_a_fresh_epoch() {
		let request = Arc::new(AtomicBool::new(false));
		let video = VideoSettings::default();
		assert!(
			CameraEncoder::config(VideoSettings::default()).profile
				== crate::video_encode::Profile::Main
		);
		// An unidentified renderer cannot select another GPU, so this is deterministic software.
		let mut encoder = CameraEncoder::new_on_adapter(
			video,
			Some(model::VideoAdapter::default()),
			request.clone(),
			Arc::new(AtomicU64::new(0)),
		)
		.unwrap();
		let preview = encoder.preview(vec![128; WIDTH * HEIGHT * 3]).unwrap();
		assert!(preview.data.is_empty());
		assert_eq!(preview.rgb.len(), WIDTH * HEIGHT * 3);
		assert!(!request.load(Ordering::Acquire));
		let first = encoder
			.encode(vec![128; WIDTH * HEIGHT * 3])
			.unwrap()
			.unwrap();
		assert!(first.keyframe);
		assert_eq!(first.epoch, preview.epoch);
		request.store(true, Ordering::Release);
		let recovered = encoder
			.encode(vec![64; WIDTH * HEIGHT * 3])
			.unwrap()
			.unwrap();
		assert!(recovered.keyframe);
		assert!(recovered.epoch > first.epoch);
		assert!(crate::video_receive::has_parameter_sets(&recovered.data));
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		assert_eq!(
			decoder
				.decode(&recovered.data)
				.unwrap()
				.unwrap()
				.dimensions(),
			(WIDTH, HEIGHT)
		);
	}

	#[test]
	fn pending_encoder_output_delivers_preview_without_completing_keyframe_recovery() {
		let request = Arc::new(AtomicBool::new(false));
		let reset = Arc::new(AtomicU64::new(0));
		let mut encoder = CameraEncoder::new_on_adapter(
			VideoSettings {
				camera_resolution: VideoResolution::P720,
				..VideoSettings::default()
			},
			Some(model::VideoAdapter::default()),
			request.clone(),
			reset.clone(),
		)
		.unwrap();
		// Model the native delayed-output result without requiring a physical GPU.
		let preview = encoder
			.finish_packet(
				vec![127; 1280 * 720 * 3],
				crate::video_encode::EncodedPacket {
					data: Vec::new(),
					keyframe: false,
					timestamp: 1234,
					epoch: 0,
					input_accepted: true,
				},
			)
			.unwrap();
		assert_eq!((preview.preview_width, preview.preview_height), (640, 360));
		assert_eq!(preview.rgb.len(), 640 * 360 * 3);
		assert!(preview.rgb.iter().all(|byte| *byte == 127));
		assert!(preview.data.is_empty() && !preview.keyframe);
		assert_eq!(
			(preview.timestamp, preview.epoch, preview.reset_generation),
			(1234, 0, 0)
		);
		assert!(Arc::ptr_eq(&preview.keyframe_request, &request));
		assert!(Arc::ptr_eq(&preview.reset, &reset));
		assert!(encoder.awaiting_keyframe);
		assert!(!request.load(Ordering::Acquire));
		let first = encoder.encode(vec![192; 1280 * 720 * 3]).unwrap().unwrap();
		assert!(!first.data.is_empty() && first.keyframe);
		assert!(!encoder.awaiting_keyframe);
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		assert_eq!(
			decoder.decode(&first.data).unwrap().unwrap().dimensions(),
			(1280, 720)
		);
	}

	#[test]
	fn security_reset_discards_capture_even_while_the_first_keyframe_is_pending() {
		let request = Arc::new(AtomicBool::new(true));
		let reset = Arc::new(AtomicU64::new(0));
		let mut encoder = CameraEncoder::new_on_adapter(
			VideoSettings::default(),
			Some(model::VideoAdapter::default()),
			request,
			reset.clone(),
		)
		.unwrap();
		assert!(encoder.awaiting_keyframe);
		reset.store(1, Ordering::Release);
		assert!(
			encoder
				.encode(vec![16; WIDTH * HEIGHT * 3])
				.unwrap()
				.is_none()
		);
		assert_eq!(encoder.reset_generation, 1);
		assert_eq!(encoder.encoding.as_ref().unwrap().encoder.epoch(), 1);
		let fresh = encoder
			.encode(vec![192; WIDTH * HEIGHT * 3])
			.unwrap()
			.unwrap();
		assert!(fresh.keyframe);
		assert_eq!((fresh.reset_generation, fresh.epoch), (1, 1));
		assert!(Arc::ptr_eq(&fresh.reset, &reset));
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		let image = decoder.decode(&fresh.data).unwrap().unwrap();
		assert_eq!(image.dimensions(), (WIDTH, HEIGHT));
		assert!(image.y().iter().all(|luma| *luma > 128));
		reset.store(u64::MAX, Ordering::Release);
		assert!(encoder.encode(vec![192; WIDTH * HEIGHT * 3]).is_err());
	}
	#[test]
	fn default_denoising_is_off_and_delivers_without_filter_warmup() {
		let mut encoder = CameraEncoder::new_on_adapter(
			VideoSettings::default(),
			Some(model::VideoAdapter::default()),
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicU64::new(0)),
		)
		.unwrap();
		assert!(encoder.denoiser.is_none());
		assert_eq!(
			*encoder.denoising_status.lock().unwrap(),
			CameraDenoisingStatus::Off
		);
		let frame = encoder
			.encode(vec![128; WIDTH * HEIGHT * 3])
			.unwrap()
			.unwrap();
		assert!(frame.keyframe && !frame.data.is_empty());
		assert!(frame.rgb.iter().all(|value| *value == 128));
	}

	#[test]
	fn filtered_camera_preview_and_encoded_video_use_the_same_picture() {
		if !offline_gpu() {
			return;
		}
		let mut encoder = CameraEncoder::new_on_adapter(
			VideoSettings {
				camera_denoising: model::voice_settings::CameraDenoising::On,
				..VideoSettings::default()
			},
			Some(model::VideoAdapter::default()),
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicU64::new(0)),
		)
		.unwrap();
		encoder.frame_budget = std::time::Duration::from_secs(60);
		let mut output = None;
		for input in 0..12 {
			let mut rgb = vec![0; WIDTH * HEIGHT * 3];
			for (index, pixel) in rgb.as_chunks_mut::<3>().0.iter_mut().enumerate() {
				let x = index % WIDTH;
				pixel.fill(if x < WIDTH / 2 {
					60 + input % 3
				} else {
					160 + input % 3
				});
			}
			if let Some(frame) = encoder.encode(rgb).unwrap()
				&& !frame.data.is_empty()
			{
				output = Some(frame);
				break;
			}
		}
		let frame = output.expect("filtered camera keyframe");
		assert!(frame.keyframe);
		let (expected, width, height) = preview_i420(&encoder.i420, WIDTH, HEIGHT).unwrap();
		assert_eq!((frame.preview_width, frame.preview_height), (width, height));
		assert_eq!(frame.rgb, expected);
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		let decoded = decoder.decode(&frame.data).unwrap().unwrap();
		assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
		let stride = decoded.strides().0;
		assert!(stride >= WIDTH && decoded.y().len() >= stride * HEIGHT);
		let error = decoded
			.y()
			.chunks_exact(stride)
			.take(HEIGHT)
			.zip(encoder.i420[..WIDTH * HEIGHT].as_chunks::<WIDTH>().0.iter())
			.flat_map(|(actual, expected)| actual[..WIDTH].iter().zip(expected))
			.map(|(actual, expected)| usize::from(actual.abs_diff(*expected)))
			.sum::<usize>();
		assert!(
			error / (WIDTH * HEIGHT) < 10,
			"encoder must receive the filtered picture (mean error {})",
			error / (WIDTH * HEIGHT)
		);
	}

	#[test]
	fn denoised_camera_discards_temporal_history_on_security_reset() {
		if !offline_gpu() {
			return;
		}
		let reset = Arc::new(AtomicU64::new(0));
		let mut encoder = CameraEncoder::new_on_adapter(
			VideoSettings {
				camera_denoising: model::voice_settings::CameraDenoising::On,
				..VideoSettings::default()
			},
			Some(model::VideoAdapter::default()),
			Arc::new(AtomicBool::new(false)),
			reset.clone(),
		)
		.unwrap();
		assert!(encoder.denoiser.is_some());
		encoder.frame_budget = std::time::Duration::from_secs(60);
		let mut saw_dark = false;
		for _ in 0..10 {
			if let Some(frame) = encoder.encode(vec![16; WIDTH * HEIGHT * 3]).unwrap()
				&& !frame.data.is_empty()
			{
				saw_dark = true;
			}
		}
		assert!(saw_dark);
		reset.store(1, Ordering::Release);
		assert!(
			encoder
				.encode(vec![16; WIDTH * HEIGHT * 3])
				.unwrap()
				.is_none()
		);
		let mut fresh = None;
		for _ in 0..10 {
			if let Some(frame) = encoder.encode(vec![240; WIDTH * HEIGHT * 3]).unwrap()
				&& !frame.data.is_empty()
			{
				fresh = Some(frame);
				break;
			}
		}
		let fresh = fresh.expect("post-reset denoised frame");
		assert!(fresh.keyframe);
		assert_eq!((fresh.reset_generation, fresh.epoch), (1, 1));
		assert!(fresh.rgb.iter().all(|v| *v > 190));
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		assert!(
			decoder
				.decode(&fresh.data)
				.unwrap()
				.unwrap()
				.y()
				.iter()
				.all(|v| *v > 190)
		);
	}

	#[test]
	fn denoiser_discards_buffered_frames_on_backpressure() {
		if !offline_gpu() {
			return;
		}
		for force_pending in [false, true] {
			let mut encoder = CameraEncoder::new_on_adapter(
				VideoSettings {
					camera_denoising: model::voice_settings::CameraDenoising::On,
					..VideoSettings::default()
				},
				Some(model::VideoAdapter::default()),
				Arc::new(AtomicBool::new(false)),
				Arc::new(AtomicU64::new(0)),
			)
			.unwrap();
			encoder.frame_budget = std::time::Duration::from_secs(60);
			let mut decoder = openh264::decoder::Decoder::new().unwrap();
			for _ in 0..8 {
				if let Some(frame) = encoder.encode(vec![16; WIDTH * HEIGHT * 3]).unwrap()
					&& !frame.data.is_empty()
				{
					let _ = decoder.decode(&frame.data).unwrap();
				}
			}
			assert!(encoder.denoiser.is_some());
			assert!(!encoder.encoding.as_ref().unwrap().encoder.pending());
			if force_pending {
				encoder
					.encoding
					.as_mut()
					.unwrap()
					.encoder
					.simulate_pending_for_test();
			}
			let old_epoch = encoder.encoding.as_ref().unwrap().encoder.epoch();
			encoder.pause_denoising();
			assert!(encoder.denoiser.is_none() && encoder.denoising_paused);
			encoder.pause_denoising();
			let mut received = false;
			for _ in 0..10 {
				if let Some(frame) = encoder.encode(vec![240; WIDTH * HEIGHT * 3]).unwrap() {
					assert!(frame.rgb.iter().all(|v| *v > 190));
					if !frame.data.is_empty() {
						assert_eq!(frame.epoch, old_epoch + u64::from(force_pending));
						if force_pending {
							assert!(frame.keyframe);
						}
						assert!(
							decoder
								.decode(&frame.data)
								.unwrap()
								.unwrap()
								.y()
								.iter()
								.all(|v| *v > 190)
						);
						received = true;
						break;
					}
				}
			}
			assert!(received, "no fresh frame after pressure subsided");
		}
	}

	#[test]
	fn timely_denoising_resets_the_overload_streak() {
		let mut streak = 0;
		let budget = std::time::Duration::from_millis(10);
		let slow = std::time::Duration::from_millis(11);
		assert!(!denoising_too_slow(&mut streak, slow, budget));
		assert!(!denoising_too_slow(&mut streak, slow, budget));
		assert!(!denoising_too_slow(&mut streak, budget, budget));
		assert_eq!(streak, 0);
		assert!(!denoising_too_slow(&mut streak, slow, budget));
		assert!(!denoising_too_slow(&mut streak, slow, budget));
		assert!(denoising_too_slow(&mut streak, slow, budget));
	}

	#[test]
	fn slow_denoiser_bypasses_without_stalling_video() {
		if !offline_gpu() {
			return;
		}
		let mut encoder = CameraEncoder::new_on_adapter(
			VideoSettings {
				camera_denoising: model::voice_settings::CameraDenoising::On,
				..VideoSettings::default()
			},
			Some(model::VideoAdapter::default()),
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicU64::new(0)),
		)
		.unwrap();
		// An impossible budget makes the adaptive bypass deterministic.
		encoder.frame_budget = std::time::Duration::ZERO;
		let mut timestamps = Vec::new();
		for _ in 0..8 {
			if let Some(frame) = encoder.encode(vec![128; WIDTH * HEIGHT * 3]).unwrap()
				&& !frame.data.is_empty()
			{
				timestamps.push(frame.timestamp);
			}
		}
		assert!(!timestamps.is_empty());
		assert!(timestamps.windows(2).all(|pair| pair[0] <= pair[1]));
		assert!(encoder.denoiser.is_none());
		assert_eq!(
			*encoder.denoising_status.lock().unwrap(),
			CameraDenoisingStatus::TooSlow
		);
	}

	#[test]
	fn oversized_camera_reports_the_resolution_limit() {
		for (width, height) in [(3840, 2160), (usize::MAX, usize::MAX)] {
			let (filter, status) = new_camera_denoiser(width, height);
			assert!(filter.is_none());
			assert_eq!(status, CameraDenoisingStatus::ResolutionUnsupported);
		}
	}

	#[test]
	fn unavailable_denoising_leaves_camera_operational() {
		let mut encoder = CameraEncoder::new_on_adapter(
			VideoSettings::default(),
			Some(model::VideoAdapter::default()),
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicU64::new(0)),
		)
		.unwrap();
		encoder.disable_denoising(CameraDenoisingStatus::Unavailable);
		assert_eq!(
			*encoder.denoising_status.lock().unwrap(),
			CameraDenoisingStatus::Unavailable
		);
		assert!(
			encoder
				.encode(vec![128; WIDTH * HEIGHT * 3])
				.unwrap()
				.is_some()
		);
	}

	#[test]
	fn selected_camera_rate_configures_the_selected_bandwidth_profile() {
		let default = CameraEncoder::config(VideoSettings::default());
		assert_eq!(default.fps, 15);
		assert_eq!(default.rates.average, 600_000);
		for rate in VideoFrameRate::ALL {
			for resolution in VideoResolution::ALL {
				let config = CameraEncoder::config(VideoSettings {
					camera_resolution: resolution,
					camera_frame_rate: rate,
					..VideoSettings::default()
				});
				assert_eq!(config.fps, rate.fps());
				assert_eq!(config.rates.average, bit_rate(resolution, rate));
				assert!(config.rates.average <= 50_000_000);
				assert_eq!(config.max_bytes, encoded_limit(resolution));
			}
		}
		for bandwidth in model::voice_settings::VideoBandwidth::ALL {
			let video = VideoSettings {
				bandwidth,
				camera_resolution: VideoResolution::P1440,
				camera_frame_rate: VideoFrameRate::Fps60,
				quality: model::voice_settings::EncoderPreset::AmfQuality,
				..VideoSettings::default()
			};
			let config = CameraEncoder::config(video);
			assert_eq!(config.rates, bandwidth.rates(2560, 1440, 60));
			assert_eq!(config.preset, video.quality);
		}
	}
	#[test]
	fn encoded_camera_allocations_follow_the_selected_preset() {
		let default = CameraEncoder::config(VideoSettings::default());
		assert_eq!((default.width, default.height), (640, 480));
		assert_eq!(default.max_bytes, 128 * 1024);
		for (resolution, limit) in [
			(VideoResolution::P480, 128 * 1024),
			(VideoResolution::P720, 256 * 1024),
			(VideoResolution::P1080, 512 * 1024),
			(VideoResolution::P1440, 1024 * 1024),
			(VideoResolution::P2160, 2 * 1024 * 1024),
			(VideoResolution::P4320, 2 * 1024 * 1024),
		] {
			let config = CameraEncoder::config(VideoSettings {
				camera_resolution: resolution,
				..VideoSettings::default()
			});
			assert_eq!(config.max_bytes, limit);
			assert!(config.max_bytes <= MAX_ENCODED_BYTES);
		}
	}
	#[test]
	fn selected_camera_resolution_is_encoded_separately_from_its_small_preview() {
		let mut encoder = CameraEncoder::new(VideoSettings {
			camera_resolution: VideoResolution::P720,
			..VideoSettings::default()
		})
		.unwrap();
		let mut produced = 0;
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		for value in [0, 127, 255].into_iter().cycle().take(48) {
			let Some(frame) = encoder.encode(vec![value; 1280 * 720 * 3]).unwrap() else {
				continue;
			};
			assert_eq!((frame.width, frame.height), (1280, 720));
			assert_eq!((frame.preview_width, frame.preview_height), (640, 360));
			assert_eq!(frame.rgb.len(), 640 * 360 * 3);
			if frame.data.is_empty() {
				continue;
			}
			produced += 1;
			let decoded = decoder.decode(&frame.data).unwrap().unwrap();
			assert_eq!(decoded.dimensions(), (1280, 720));
		}
		assert!(produced > 0);
	}

	#[test]
	fn high_resolution_preview_is_small_and_preserves_the_picture() {
		let rgb = vec![126; 3840 * 2160 * 3];
		let (preview, width, height) = preview_rgb(rgb, 3840, 2160).unwrap();
		assert_eq!((width, height), (640, 360));
		assert_eq!(preview.len(), 640 * 360 * 3);
		assert!(preview.iter().all(|byte| *byte == 126));
		assert!(preview_rgb(Vec::new(), 7681, 4320).is_err());
		assert!(preview_rgb(vec![0; 6], 2, 2).is_err());
		let original = vec![12; WIDTH * HEIGHT * 3];
		let pointer = original.as_ptr();
		let (preview, width, height) = preview_rgb(original, WIDTH, HEIGHT).unwrap();
		assert_eq!((width, height), (WIDTH as u32, HEIGHT as u32));
		assert_eq!(preview.as_ptr(), pointer);
	}

	#[test]
	fn camera_rejects_unbounded_or_nul_device_ids_before_starting_worker() {
		for id in ["x".repeat(4097), "dshow:bad\0id".into()] {
			let error = Camera::start(
				Some(id),
				VideoSettings::default(),
				Arc::new(|_| panic!("no capture")),
				Arc::new(|| {}),
			)
			.err();
			if SUPPORTED {
				assert_eq!(error, Some("Invalid camera device selection"));
			} else {
				assert!(error.is_some());
			}
		}
	}
	use openh264::formats::YUVSource;

	#[test]
	fn camera_frames_preserve_prediction_order_and_stop_is_immediate() {
		let mut encoder = CameraEncoder::new(VideoSettings {
			codec: VideoCodec::H264,
			..VideoSettings::default()
		})
		.unwrap();
		for length in [0, WIDTH * HEIGHT * 3 - 1, WIDTH * HEIGHT * 3 + 1] {
			assert!(encoder.encode(vec![0; length]).is_err());
		}
		let mut produced = 0;
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		for value in [0, 127, 255].into_iter().cycle().take(48) {
			if let Some(frame) = encoder.encode(vec![value; WIDTH * HEIGHT * 3]).unwrap() {
				assert_eq!(frame.rgb.len(), WIDTH * HEIGHT * 3);
				assert!(frame.data.len() <= MAX_ENCODED_BYTES);
				if frame.data.is_empty() {
					continue;
				}
				produced += 1;
				let decoded = decoder.decode(&frame.data).unwrap().unwrap();
				assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
			}
		}
		assert!(produced > 0, "camera encoder must produce bounded output");
		let camera = Camera {
			shared: Arc::new(Shared::default()),
		};
		camera.shared.active.store(true, Ordering::Release);
		assert!(camera.active());
		camera.stop();
		// A late native callback cannot turn a stopped camera back on.
		camera.shared.active.store(true, Ordering::Release);
		assert!(!camera.active());
		assert!(!camera.stopped());
		camera.shared.finished.store(true, Ordering::Release);
		assert!(camera.stopped());

		{
			let mut encoder = CameraEncoder::new(VideoSettings {
				codec: VideoCodec::H264,
				..VideoSettings::default()
			})
			.unwrap();
			for length in [0, WIDTH * HEIGHT * 3 - 1, WIDTH * HEIGHT * 3 + 1] {
				assert!(encoder.encode(vec![0; length]).is_err());
			}
			let mut decoder = openh264::decoder::Decoder::new().unwrap();
			let mut produced = 0;
			for value in [0, 96, 255].into_iter().cycle().take(48) {
				let mut rgb = vec![value; WIDTH * HEIGHT * 3];
				// Flat pictures compress to almost nothing; vary one row so the size check bites.
				for (index, pixel) in rgb
					.as_chunks_mut::<3>()
					.0
					.iter_mut()
					.take(WIDTH)
					.enumerate()
				{
					*pixel = [(index % 251) as u8, value, (index % 97) as u8];
				}
				let Some(frame) = encoder.encode(rgb).unwrap() else {
					continue;
				};
				assert_eq!(frame.rgb.len(), WIDTH * HEIGHT * 3);
				assert!(frame.data.len() <= MAX_ENCODED_BYTES);
				if frame.data.is_empty() {
					continue;
				}
				produced += 1;
				// Every restart starts from a keyframe; subsequent predictions stay in order.
				assert_eq!(
					frame.keyframe,
					crate::video_receive::is_keyframe(&frame.data)
				);
				if produced == 1 {
					assert!(
						frame.keyframe && crate::video_receive::has_parameter_sets(&frame.data)
					);
				}
				let decoded = decoder.decode(&frame.data).unwrap().unwrap();
				assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
			}
			assert!(produced > 0, "camera encoder must produce bounded output");
		}
	}
}

#[cfg(test)]
#[test]
#[ignore = "synthetic release workload; no capture devices or network"]
fn synthetic_camera_encode_workload() {
	synthetic_camera_encode(VideoSettings {
		codec: VideoCodec::H264,
		..VideoSettings::default()
	});
}

#[cfg(test)]
#[test]
#[ignore = "synthetic release workload; no capture devices or network"]
fn synthetic_high_bandwidth_camera_encode_workload() {
	synthetic_camera_encode(VideoSettings {
		bandwidth: model::voice_settings::VideoBandwidth::High,
		..Default::default()
	});
}

#[cfg(test)]
fn synthetic_camera_encode(video: VideoSettings) {
	let mut encoder = CameraEncoder::new(video).unwrap();
	let mut rgb = vec![0; WIDTH * HEIGHT * 3];
	for (index, pixel) in rgb.as_chunks_mut::<3>().0.iter_mut().enumerate() {
		let (x, y) = (index % WIDTH, index / WIDTH);
		*pixel = [(x % 251) as u8, (y * 7 % 251) as u8, ((x + y) % 251) as u8];
	}
	for _ in 0..30 {
		std::hint::black_box(encoder.encode(rgb.clone()).unwrap());
	}
	let started = std::time::Instant::now();
	let (mut packets, mut bytes) = (0, 0);
	for _ in 0..300 {
		if let Some(frame) = encoder.encode(rgb.clone()).unwrap() {
			if frame.data.is_empty() {
				continue;
			}
			packets += 1;
			bytes += frame.data.len();
			std::hint::black_box(frame);
		}
	}
	let elapsed = started.elapsed();
	assert_eq!(packets, 300);
	println!(
		"camera_encode_ms={:.3} packets={packets} bytes={bytes}",
		elapsed.as_secs_f64() * 1000.0
	);
}
