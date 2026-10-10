//! User-started, ephemeral camera capture. No capture stream starts before `start`.

use model::voice_settings::{VideoCodec, VideoFrameRate, VideoResolution, VideoSettings};
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

/// Camera bitrate, bounded by the selected preset and rate.
pub fn bit_rate(resolution: VideoResolution, frame_rate: VideoFrameRate) -> u32 {
	let base = match resolution {
		VideoResolution::P480 => 600_000,
		VideoResolution::P720 => 2_000_000,
		VideoResolution::P1080 => 4_000_000,
		VideoResolution::P1440 => 8_000_000,
		VideoResolution::P2160 => 16_000_000,
		VideoResolution::P4320 => 40_000_000,
	};
	(base * (frame_rate.fps() / 15)).min(50_000_000)
}
pub const SUPPORTED: bool = cfg!(any(
	target_os = "macos",
	target_os = "windows",
	target_os = "linux"
));
// Includes asynchronous teardown: rapid toggles cannot accumulate camera workers.
static RUNNING: AtomicBool = AtomicBool::new(false);

pub struct Frame {
	/// Actual encoded picture dimensions; the preview has its own smaller geometry.
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
	capacity: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
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
		if !video.is_valid() {
			return Err("Stable video encoding supports H.264 only");
		}
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
		let shared = Arc::new(Shared {
			capacity: Some(capacity),
			..Shared::default()
		});
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

	pub fn active(&self) -> bool {
		!self.shared.stopped.load(Ordering::Acquire) && self.shared.active.load(Ordering::Acquire)
	}
}

/// Stable cameras keep independent pictures. Experimental cameras preserve prediction order.
struct CameraEncoder {
	diagnostics: crate::diagnostics::EncoderRegistration,
	encoder: crate::video_backend::Encoder,
	codec: VideoCodec,
	yuv: YUVBuffer,
	i420: Vec<u8>,
	dimensions: (usize, usize),
	origin: std::time::Instant,
	keyframe_request: Arc<AtomicBool>,
	awaiting_keyframe: bool,
	reset: Arc<AtomicU64>,
	reset_generation: u64,
}

impl CameraEncoder {
	fn config(video: VideoSettings) -> crate::video_encode::Config {
		let (width, height) = video.camera_resolution.camera_dimensions();
		crate::video_encode::Config {
			width,
			height,
			fps: video.camera_frame_rate.fps(),
			bit_rate: bit_rate(video.camera_resolution, video.camera_frame_rate),
			max_bytes: encoded_limit(video.camera_resolution),
			profile: if video.backend == model::voice_settings::VideoBackend::Stable {
				crate::video_encode::Profile::Baseline
			} else {
				crate::video_encode::Profile::Main
			},
			codec: video.codec,
			adapter: None,
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

	fn new_on_adapter(
		video: VideoSettings,
		adapter: Option<model::VideoAdapter>,
		keyframe_request: Arc<AtomicBool>,
		reset: Arc<AtomicU64>,
	) -> Result<Self, &'static str> {
		let mut config = Self::config(video);
		config.adapter = adapter;
		let (width, height) = (config.width as usize, config.height as usize);
		let encoder = crate::video_backend::Encoder::new(config, video.backend)?;
		Ok(Self {
			diagnostics: crate::diagnostics::EncoderRegistration::new(
				false,
				encoder.hardware(),
				encoder.amf_split(),
			),
			encoder,
			codec: video.codec,
			yuv: YUVBuffer::new(width, height),
			i420: Vec::with_capacity(width * height * 3 / 2),
			dimensions: (width, height),
			origin: std::time::Instant::now(),
			keyframe_request,
			awaiting_keyframe: true,
			reset,
			reset_generation: 0,
		})
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
			self.encoder.restart()?;
			self.reset_generation = reset;
			self.awaiting_keyframe = true;
			// Capture can span the reset. The following input must be newly captured.
			return Ok(None);
		}
		self.yuv.read_rgb8(RgbSliceU8::new(&rgb, (width, height)));
		self.i420.clear();
		self.i420.extend_from_slice(self.yuv.y());
		self.i420.extend_from_slice(self.yuv.u());
		self.i420.extend_from_slice(self.yuv.v());
		// Coalesce repeated feedback while lookahead is still producing the first IDR.
		if self.keyframe_request.swap(false, Ordering::AcqRel) && !self.awaiting_keyframe {
			self.encoder.restart()?;
			self.awaiting_keyframe = true;
		}
		let timestamp = (self.origin.elapsed().as_micros() * 90 / 1000) as u32;
		let packet = self.encoder.encode_at(&self.i420, false, timestamp)?;
		self.diagnostics
			.set(Some((self.encoder.hardware(), self.encoder.amf_split())));
		self.finish_packet(rgb, packet).map(Some)
	}

	fn finish_packet(
		&mut self,
		rgb: Vec<u8>,
		packet: crate::video_encode::EncodedPacket,
	) -> Result<Frame, &'static str> {
		if !packet.data.is_empty() {
			if self.awaiting_keyframe && !packet.keyframe {
				return Err("Camera encoder did not restart at an independently decodable picture");
			}
			self.awaiting_keyframe = false;
		}
		// Lookahead delays media output, but the current capture is ready for local preview.
		let mut frame = self.preview(rgb)?;
		frame.keyframe = !packet.data.is_empty() && packet.keyframe;
		frame.data = packet.data;
		frame.timestamp = packet.timestamp;
		frame.epoch = packet.epoch;
		Ok(frame)
	}

	fn preview(&self, rgb: Vec<u8>) -> Result<Frame, &'static str> {
		let (width, height) = self.dimensions;
		let (rgb, preview_width, preview_height) = preview_rgb(rgb, width, height)?;
		Ok(Frame {
			width: width as u32,
			height: height as u32,
			preview_width,
			preview_height,
			rgb,
			data: Vec::new(),
			codec: self.codec,
			timestamp: (self.origin.elapsed().as_micros() * 90 / 1000) as u32,
			keyframe: false,
			epoch: self.encoder.epoch(),
			keyframe_request: self.keyframe_request.clone(),
			reset: self.reset.clone(),
			reset_generation: self.reset_generation,
		})
	}
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

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn run(
	shared: &Shared,
	device: Option<&str>,
	video: VideoSettings,
	adapter: Option<model::VideoAdapter>,
	on_frame: &Arc<dyn Fn(Frame) + Send + Sync>,
	wake: &Arc<dyn Fn() + Send + Sync>,
) -> Result<(), &'static str> {
	let mut encoder = CameraEncoder::new_on_adapter(
		video,
		adapter,
		shared.keyframe_request.clone(),
		shared.reset.clone(),
	)?;
	let mut emit = |rgb| {
		if shared.stopped.load(Ordering::Acquire) {
			return Ok(());
		}
		let frame = if shared.capacity.as_ref().is_none_or(|capacity| capacity()) {
			encoder.encode(rgb)?
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
	};
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

	const DENIED: &str = "Camera access denied. Allow Serein (or your terminal) in System Settings > Privacy & Security > Camera, then try again.";

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
		let mut encoder = CameraEncoder::new_on_adapter(
			video,
			adapter,
			shared.keyframe_request.clone(),
			shared.reset.clone(),
		)?;
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
			let frame = if shared.capacity.as_ref().is_none_or(|capacity| capacity()) {
				encoder.encode(rgb)?
			} else {
				Some(encoder.preview(rgb)?)
			};
			let Some(frame) = frame else {
				continue;
			};
			if shared.stopped.load(Ordering::Acquire) {
				break;
			}
			on_frame(frame);
			shared.active.store(true, Ordering::Release);
			wake();
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn camera_preview_does_not_advance_encoding_and_recovery_starts_a_fresh_epoch() {
		let request = Arc::new(AtomicBool::new(false));
		let video = VideoSettings {
			backend: model::voice_settings::VideoBackend::Experimental,
			..VideoSettings::default()
		};
		assert!(
			CameraEncoder::config(VideoSettings::default()).profile
				== crate::video_encode::Profile::Baseline
		);
		assert!(CameraEncoder::config(video).profile == crate::video_encode::Profile::Main);
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
				backend: model::voice_settings::VideoBackend::Experimental,
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
			VideoSettings {
				backend: model::voice_settings::VideoBackend::Experimental,
				..VideoSettings::default()
			},
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
		assert_eq!(encoder.encoder.epoch(), 1);
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
	fn selected_camera_rate_configures_encoding_and_scales_bitrate_with_a_ceiling() {
		let default = CameraEncoder::config(VideoSettings::default());
		assert_eq!(default.fps, 15);
		assert_eq!(default.bit_rate, 600_000);
		for rate in VideoFrameRate::ALL {
			for resolution in VideoResolution::ALL {
				let config = CameraEncoder::config(VideoSettings {
					camera_resolution: resolution,
					camera_frame_rate: rate,
					..VideoSettings::default()
				});
				assert_eq!(config.fps, rate.fps());
				assert_eq!(config.bit_rate, bit_rate(resolution, rate));
				assert!(config.bit_rate <= 50_000_000);
				assert_eq!(config.max_bytes, encoded_limit(resolution));
			}
		}
		assert_eq!(
			bit_rate(VideoResolution::P480, VideoFrameRate::Fps30),
			1_200_000
		);
		assert_eq!(
			bit_rate(VideoResolution::P480, VideoFrameRate::Fps60),
			2_400_000
		);
		assert_eq!(
			bit_rate(VideoResolution::P2160, VideoFrameRate::Fps30),
			32_000_000
		);
		assert_eq!(
			bit_rate(VideoResolution::P2160, VideoFrameRate::Fps60),
			50_000_000
		);
		assert_eq!(
			bit_rate(VideoResolution::P4320, VideoFrameRate::Fps60),
			50_000_000
		);
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
			backend: model::voice_settings::VideoBackend::Experimental,
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
			backend: model::voice_settings::VideoBackend::Experimental,
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
				backend: model::voice_settings::VideoBackend::Experimental,
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
		backend: model::voice_settings::VideoBackend::Experimental,
		codec: VideoCodec::H264,
		..VideoSettings::default()
	});
}

#[cfg(test)]
#[test]
#[ignore = "synthetic release workload; no capture devices or network"]
fn synthetic_stable_camera_encode_workload() {
	synthetic_camera_encode(VideoSettings::default());
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
