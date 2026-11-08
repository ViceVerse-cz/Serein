//! User-started, ephemeral camera capture. No capture stream starts before `start`.

use openh264::{
	OpenH264API,
	encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Profile},
	formats::{RgbSliceU8, YUVBuffer},
};
use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, Ordering},
};
use std::{thread, time::Duration};

#[cfg(target_os = "linux")]
#[path = "camera/encode_linux.rs"]
mod encode_linux;
mod format;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

pub const WIDTH: usize = 1280;
pub const HEIGHT: usize = 720;
pub const FPS: u32 = format::FPS;
pub const BIT_RATE: u32 = 2_500_000;
pub const MAX_ENCODED_BYTES: usize = 256 * 1024;
/// Pictures between periodic IDR refreshes; feedback and sender drops request one sooner.
const KEYFRAME_INTERVAL: u32 = FPS * 4;
pub const SUPPORTED: bool = cfg!(any(
	target_os = "macos",
	target_os = "windows",
	target_os = "linux"
));
/// Consecutive empty hardware outputs (two seconds) before falling back to openh264, so a
/// hardware encoder that accepts pictures but never returns them cannot leave the call
/// without video.
const HARDWARE_STALL: u8 = (FPS * 2) as u8;
const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / format::FPS as u64);
/// How early a picture may arrive for its slot. Without it, a device running at exactly
/// `FPS` loses every other picture to delivery jitter.
const FRAME_SLACK: Duration = Duration::from_nanos(FRAME_INTERVAL.as_nanos() as u64 / 4);

/// Admits at most `FPS` pictures per second on average. Each admission advances a schedule by
/// one interval rather than restarting it from the arrival, so early pictures cannot
/// accumulate into a higher rate. A schedule more than one interval behind restarts from the
/// arrival, so a pause never banks credit for a burst.
#[derive(Default)]
struct FrameGate {
	next: Option<std::time::Instant>,
}

impl FrameGate {
	/// Whether a picture arriving at `now` fits the schedule; nothing is recorded.
	fn ready(&self, now: std::time::Instant) -> bool {
		self.next.is_none_or(|next| now + FRAME_SLACK >= next)
	}

	/// Records a picture that was actually handed on. Callers whose hand-off can fail call
	/// this only on success, so a discarded picture does not use up a slot.
	fn commit(&mut self, now: std::time::Instant) {
		let base = match self.next {
			Some(next) if now < next + FRAME_INTERVAL => next,
			_ => now,
		};
		self.next = Some(base + FRAME_INTERVAL);
	}

	/// `ready` and `commit` together, for callers whose hand-off cannot fail.
	#[cfg_attr(not(any(test, target_os = "linux")), allow(dead_code))]
	fn admit(&mut self, now: std::time::Instant) -> bool {
		let ready = self.ready(now);
		if ready {
			self.commit(now);
		}
		ready
	}
}
// Includes asynchronous teardown: rapid toggles cannot accumulate camera workers.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Set when a receiver, the network or the sender lost a picture of the predicted chain. At
/// most one camera runs at a time (`RUNNING`), so one process-wide flag suffices.
static KEYFRAME: AtomicBool = AtomicBool::new(true);

/// Makes the next encoded camera picture an IDR with parameter sets.
pub fn request_keyframe() {
	KEYFRAME.store(true, Ordering::Release);
}

/// One captured picture. `h264` is empty when the encoder held or dropped this picture;
/// the local preview still shows `rgb`, but nothing is sent. Pictures other than keyframes
/// predict from the previous one, so a sender that drops any of them must wait for a
/// keyframe and call [`request_keyframe`].
pub struct Frame {
	pub rgb: Vec<u8>,
	pub h264: Vec<u8>,
	pub keyframe: bool,
}

#[derive(Default)]
struct Shared {
	stopped: AtomicBool,
	active: AtomicBool,
	finished: AtomicBool,
	error: Mutex<Option<&'static str>>,
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
		on_frame: Arc<dyn Fn(Frame) + Send + Sync>,
		wake: Arc<dyn Fn() + Send + Sync>,
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
		request_keyframe();
		let shared = Arc::new(Shared::default());
		let worker = shared.clone();
		let teardown = crate::teardown::Thread::start();
		if thread::Builder::new()
			.name("serein-camera".into())
			.spawn(move || {
				// Declared first so it is released last, after the camera encoder.
				let _teardown = teardown;
				let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
					#[cfg(target_os = "macos")]
					{
						objc2::rc::autoreleasepool(|_| {
							macos::run(&worker, device.as_deref(), &on_frame, &wake)
						})
					}
					#[cfg(any(target_os = "windows", target_os = "linux"))]
					{
						run(&worker, device.as_deref(), &on_frame, &wake)
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

/// Camera encoder preferring the platform hardware H.264 encoder (VideoToolbox on macOS,
/// Media Foundation on Windows, VA-API or NVENC through GStreamer on Linux) and falling back
/// to openh264 when it is unavailable or fails mid-stream. Baseline profile either way, with
/// predicted pictures between IDRs that are periodic or requested through [`request_keyframe`].
struct CameraEncoder {
	diagnostics: crate::diagnostics::EncoderRegistration,
	software: Option<Encoder>,
	yuv: YUVBuffer,
	#[cfg(any(target_os = "macos", target_os = "windows"))]
	hardware: Option<crate::video_encode::hardware::Encoder>,
	#[cfg(target_os = "linux")]
	hardware: Option<encode_linux::Encoder>,
	#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
	empty_run: u8,
}

impl CameraEncoder {
	fn new() -> Result<Self, &'static str> {
		#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
		let config = crate::video_encode::Config {
			width: WIDTH as u32,
			height: HEIGHT as u32,
			fps: FPS,
			bit_rate: BIT_RATE,
			max_bytes: MAX_ENCODED_BYTES,
			profile: crate::video_encode::Profile::Baseline,
		};
		#[cfg(target_os = "macos")]
		let hardware = crate::video_encode::hardware::Encoder::new(
			config,
			crate::video_encode::SourceFormat::Rgb,
		)
		.ok();
		#[cfg(target_os = "windows")]
		let hardware = crate::video_encode::hardware::Encoder::new(config).ok();
		#[cfg(target_os = "linux")]
		let hardware = encode_linux::Encoder::new(config).ok();
		#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
		let software = if hardware.is_some() {
			None
		} else {
			Some(encoder()?)
		};
		#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
		let software = Some(encoder()?);
		Ok(Self {
			diagnostics: crate::diagnostics::EncoderRegistration::new(false, software.is_none()),
			software,
			yuv: YUVBuffer::new(WIDTH, HEIGHT),
			#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
			hardware,
			#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
			empty_run: 0,
		})
	}

	/// Encodes one packed RGB picture, keeping the frame's pixels for local preview.
	fn encode(&mut self, rgb: Vec<u8>) -> Result<Frame, &'static str> {
		if rgb.len() != WIDTH * HEIGHT * 3 {
			return Err(INVALID);
		}
		// Taken once per picture; a request racing this picture applies to the next one.
		let keyframe = KEYFRAME.swap(false, Ordering::AcqRel);
		#[cfg(target_os = "linux")]
		if let Some(hardware) = self.hardware.as_mut() {
			match hardware.encode(&rgb, keyframe) {
				Ok(Some(h264)) => {
					self.empty_run = 0;
					return frame(rgb, h264);
				}
				// The encoder is still holding this picture; the next one returns it. A forced
				// keyframe stays attached to the held picture inside the encoder.
				Ok(None) if self.note_empty() => {
					return Ok(Frame {
						rgb,
						h264: Vec::new(),
						keyframe: false,
					});
				}
				_ => self.use_software()?,
			}
		}
		#[cfg(any(target_os = "macos", target_os = "windows"))]
		if let Some(hardware) = self.hardware.as_mut() {
			#[cfg(target_os = "macos")]
			let encoded = hardware.encode(&rgb, (WIDTH, HEIGHT), keyframe);
			#[cfg(target_os = "windows")]
			let encoded = {
				use openh264::formats::YUVSource;
				self.yuv.read_rgb8(RgbSliceU8::new(&rgb, (WIDTH, HEIGHT)));
				hardware.encode(self.yuv.y(), self.yuv.u(), self.yuv.v(), keyframe)
			};
			match encoded {
				Ok((h264, _)) if !h264.is_empty() => {
					self.empty_run = 0;
					return frame(rgb, h264);
				}
				// Dropped to hold the rate; a run of these means the encoder is stuck. The
				// request is retried, because a dropped picture cannot carry the keyframe.
				Ok(_) if self.note_empty() => {
					if keyframe {
						request_keyframe();
					}
					return Ok(Frame {
						rgb,
						h264: Vec::new(),
						keyframe: false,
					});
				}
				_ => self.use_software()?,
			}
		}
		let software = self
			.software
			.as_mut()
			.ok_or("Camera H264 encoder could not start")?;
		encode_rgb(software, &mut self.yuv, rgb, keyframe)
	}

	/// Counts one empty hardware output; false once the run says the encoder is stuck.
	#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
	fn note_empty(&mut self) -> bool {
		self.empty_run = self.empty_run.saturating_add(1);
		self.empty_run < HARDWARE_STALL
	}

	#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
	fn use_software(&mut self) -> Result<(), &'static str> {
		// openh264 starts with an IDR; receivers still hold the hardware encoder's references.
		request_keyframe();
		self.hardware = None;
		self.empty_run = 0;
		self.diagnostics.set(None);
		self.software = Some(encoder()?);
		self.diagnostics.set(Some(false));
		Ok(())
	}
}

fn encoder() -> Result<Encoder, &'static str> {
	Encoder::with_api_config(
		OpenH264API::from_source(),
		EncoderConfig::new()
			.bitrate(BitRate::from_bps(BIT_RATE))
			.max_frame_rate(FrameRate::from_hz(FPS as f32))
			.intra_frame_period(IntraFramePeriod::from_num_frames(KEYFRAME_INTERVAL))
			.profile(Profile::Baseline)
			.num_threads(1)
			.debug(false),
	)
	.map_err(|_| "Camera H264 encoder could not start")
}

fn encode_rgb(
	encoder: &mut Encoder,
	yuv: &mut YUVBuffer,
	rgb: Vec<u8>,
	keyframe: bool,
) -> Result<Frame, &'static str> {
	if rgb.len() != WIDTH * HEIGHT * 3 {
		return Err(INVALID);
	}
	yuv.read_rgb8(RgbSliceU8::new(&rgb, (WIDTH, HEIGHT)));
	if keyframe {
		encoder.force_intra_frame();
	}
	let bits = encoder
		.encode(yuv)
		.map_err(|_| "Camera frame could not be encoded")?;
	if bits.raw_info().iFrameSizeInBytes < 0 {
		return Err("Camera frame could not be encoded");
	}
	let h264 = bits.to_vec();
	if h264.is_empty() {
		// Rate control skipped this picture; skipped pictures are never referenced.
		if keyframe {
			request_keyframe();
		}
		return Ok(Frame {
			rgb,
			h264,
			keyframe: false,
		});
	}
	frame(rgb, h264)
}

const INVALID: &str = "Camera did not provide a bounded RGB frame";

/// Checks one encoded access unit and records whether it restarts the prediction chain.
fn frame(rgb: Vec<u8>, h264: Vec<u8>) -> Result<Frame, &'static str> {
	if h264.len() > MAX_ENCODED_BYTES {
		return Err("Camera encoded frame exceeded its 256 KiB limit");
	}
	let keyframe = crate::video_receive::is_keyframe(&h264);
	if keyframe && !crate::video_receive::has_parameter_sets(&h264) {
		return Err("Camera keyframe has no parameter sets");
	}
	Ok(Frame {
		rgb,
		h264,
		keyframe,
	})
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn run(
	shared: &Shared,
	device: Option<&str>,
	on_frame: &Arc<dyn Fn(Frame) + Send + Sync>,
	wake: &Arc<dyn Fn() + Send + Sync>,
) -> Result<(), &'static str> {
	let mut encoder = CameraEncoder::new()?;
	let mut emit = |rgb| {
		if shared.stopped.load(Ordering::Acquire) {
			return Ok(());
		}
		let frame = encoder.encode(rgb)?;
		if !shared.stopped.load(Ordering::Acquire) {
			on_frame(frame);
			shared.active.store(true, Ordering::Release);
			wake();
		}
		Ok(())
	};
	#[cfg(target_os = "windows")]
	{
		windows::run(shared, device, &mut emit)
	}
	#[cfg(target_os = "linux")]
	{
		linux::run(shared, device, &mut emit)
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
		gate: Mutex<FrameGate>,
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
				let Ok(mut gate) = state.gate.try_lock() else {
					return;
				};
				let now = Instant::now();
				if !gate.ready(now) {
					return;
				}
				// A picture the busy worker cannot take does not use up its slot.
				if state.send.try_send(copy_bgra(sample)).is_ok() {
					gate.commit(now);
				}
			}
		}
	);

	fn copy_bgra(sample: &CMSampleBuffer) -> Result<Vec<u8>, &'static str> {
		// SAFETY: The sample is valid for this delegate invocation. Retain its image,
		// verify packed BGRA dimensions/stride before reading, and unlock every path.
		unsafe {
			let pixels = sample.image_buffer().ok_or("Camera returned no image")?;
			let stride = CVPixelBufferGetBytesPerRow(&pixels);
			let bytes = stride
				.checked_mul(HEIGHT)
				.ok_or("Camera frame exceeds bounds")?;
			if CVPixelBufferGetWidth(&pixels) != WIDTH
				|| CVPixelBufferGetHeight(&pixels) != HEIGHT
				|| CVPixelBufferGetPixelFormatType(&pixels) != kCVPixelFormatType_32BGRA
				|| !(WIDTH * 4..=WIDTH * 4 + 4096).contains(&stride)
				|| bytes > CVPixelBufferGetDataSize(&pixels)
			{
				return Err("Camera did not provide a bounded BGRA frame");
			}
			if CVPixelBufferLockBaseAddress(&pixels, CVPixelBufferLockFlags::ReadOnly) != 0 {
				return Err("Camera image could not be read");
			}
			let base = CVPixelBufferGetBaseAddress(&pixels);
			let result = if base.is_null() {
				Err("Camera returned an empty image")
			} else {
				let source = std::slice::from_raw_parts(base.cast::<u8>(), bytes);
				let mut bgra = vec![0; WIDTH * HEIGHT * 4];
				for (row, dest) in source
					.chunks_exact(stride)
					.zip(bgra.as_chunks_mut::<{ WIDTH * 4 }>().0)
				{
					dest.copy_from_slice(&row[..WIDTH * 4]);
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

	/// Largest native mode accepted when a camera offers nothing at or below 1280×720.
	const FALLBACK: (usize, usize) = (3840, 2160);

	fn configure(device: &AVCaptureDevice) -> Result<(), &'static str> {
		// SAFETY: Formats/ranges come from this device. Only this worker changes it,
		// while locked, before capture starts. Endpoint durations stay exact.
		unsafe {
			let mut selected = None;
			for native in device.formats().iter().take(256) {
				let size = CMVideoFormatDescriptionGetDimensions(&native.formatDescription());
				for range in native.videoSupportedFrameRateRanges().iter().take(256) {
					let (min, max) = (range.minFrameRate(), range.maxFrameRate());
					let Some(fps) = format::nearest_fps(min, max) else {
						continue;
					};
					let (width, height) = (size.width as usize, size.height as usize);
					// Prefer modes within the usual ceiling; cameras that only offer larger
					// ones still work, since AVFoundation scales to the encoder size before delivery.
					let Some(rank) =
						format::rank_with_ceiling(width, height, fps, FALLBACK.0, FALLBACK.1)
					else {
						continue;
					};
					let rank = (
						width > format::MAX_WIDTH || height > format::MAX_HEIGHT,
						rank,
					);
					let duration = if fps == min {
						range.maxFrameDuration()
					} else if fps == max {
						range.minFrameDuration()
					} else {
						CMTime::new(1, format::FPS as i32)
					};
					if selected.as_ref().is_none_or(|(best, _, _)| rank < *best) {
						selected = Some((rank, native.clone(), duration));
					}
				}
			}
			let (_, native, duration) =
				selected.ok_or("Camera does not offer a capture mode at or below 4K")?;
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
		on_frame: &Arc<dyn Fn(Frame) + Send + Sync>,
		wake: &Arc<dyn Fn() + Send + Sync>,
	) -> Result<(), &'static str> {
		authorize(shared)?;
		if shared.stopped.load(Ordering::Acquire) {
			return Ok(());
		}
		let mut encoder = CameraEncoder::new()?;
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
			let configured = configure(&device);
			session.commitConfiguration();
			configured?;
			let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
			let width = NSNumber::new_usize(WIDTH);
			let height = NSNumber::new_usize(HEIGHT);
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
				gate: Mutex::new(FrameGate::default()),
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
			let mut rgb = vec![0; WIDTH * HEIGHT * 3];
			for (bgra, rgb) in bgra
				.as_chunks::<4>()
				.0
				.iter()
				.zip(rgb.as_chunks_mut::<3>().0)
			{
				rgb.copy_from_slice(&[bgra[2], bgra[1], bgra[0]]);
			}
			let frame = encoder.encode(rgb)?;
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
	fn camera_rejects_unbounded_or_nul_device_ids_before_starting_worker() {
		for id in ["x".repeat(4097), "dshow:bad\0id".into()] {
			let error = Camera::start(
				Some(id),
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
	fn camera_frames_are_bounded_predicted_and_stop_is_immediate() {
		let mut encoder = encoder().unwrap();
		let mut yuv = YUVBuffer::new(WIDTH, HEIGHT);
		for length in [0, WIDTH * HEIGHT * 3 - 1, WIDTH * HEIGHT * 3 + 1] {
			assert!(encode_rgb(&mut encoder, &mut yuv, vec![0; length], false).is_err());
		}
		// The first picture and every requested one restart the chain; the rest predict
		// from their predecessor and decode only in order.
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		let mut kinds = Vec::new();
		for (index, keyframe) in [true, false, false, true, false].into_iter().enumerate() {
			let frame =
				encode_rgb(&mut encoder, &mut yuv, textured(index as u8 * 40), keyframe).unwrap();
			if frame.h264.is_empty() {
				continue;
			}
			assert!(frame.h264.len() <= MAX_ENCODED_BYTES);
			assert_eq!(
				frame.keyframe,
				crate::video_receive::is_keyframe(&frame.h264)
			);
			if frame.keyframe {
				assert!(crate::video_receive::has_parameter_sets(&frame.h264));
			}
			kinds.push((keyframe, frame.keyframe));
			let decoded = decoder.decode(&frame.h264).unwrap().unwrap();
			assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
		}
		assert_eq!(kinds.first(), Some(&(true, true)));
		assert!(
			kinds
				.iter()
				.all(|(requested, keyframe)| !requested || *keyframe)
		);
		assert!(kinds.iter().any(|(_, keyframe)| !keyframe));
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
			let mut encoder = CameraEncoder::new().unwrap();
			// Exactly one encoder is live: hardware when the machine offers it, openh264 otherwise.
			#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
			assert_eq!(encoder.hardware.is_some(), encoder.software.is_none());
			for length in [0, WIDTH * HEIGHT * 3 - 1, WIDTH * HEIGHT * 3 + 1] {
				assert!(encoder.encode(vec![0; length]).is_err());
			}
			let mut decoder = openh264::decoder::Decoder::new().unwrap();
			let mut started = false;
			request_keyframe();
			for value in 0..12u8 {
				let frame = encoder.encode(textured(value * 20)).unwrap();
				if frame.h264.is_empty() {
					continue;
				}
				assert_eq!(frame.rgb.len(), WIDTH * HEIGHT * 3);
				assert!(frame.h264.len() <= MAX_ENCODED_BYTES);
				// Decoding starts at a keyframe; later pictures may predict from it.
				started |= frame.keyframe;
				assert!(started);
				if frame.keyframe {
					assert!(crate::video_receive::has_parameter_sets(&frame.h264));
				}
				if let Some(decoded) = decoder.decode(&frame.h264).unwrap() {
					assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
				}
			}
			assert!(started);
		}
	}

	#[test]
	fn frame_gate_holds_the_average_rate_and_tolerates_jitter() {
		let start = std::time::Instant::now();
		let admitted = |period_us: u64, jitter_us: u64| {
			let mut gate = FrameGate::default();
			(0..300u64)
				.filter(|index| {
					// Alternate early and late deliveries around the nominal period.
					let offset = if index % 2 == 0 { 0 } else { jitter_us };
					gate.admit(start + Duration::from_micros(index * period_us + offset))
				})
				.count()
		};
		let interval = FRAME_INTERVAL.as_micros() as u64;
		// A device at exactly `FPS` keeps every picture despite a few milliseconds of jitter.
		assert_eq!(admitted(interval, 4_000), 300);
		// Faster devices are held to `FPS`: 40 fps keeps three in four, 60 fps one in two.
		assert!(
			(224..=226).contains(&admitted(25_000, 0)),
			"{}",
			admitted(25_000, 0)
		);
		assert!((150..=151).contains(&admitted(interval / 2, 0)));
		// A slot is used only by a committed picture.
		let mut gate = FrameGate::default();
		assert!(gate.ready(start));
		assert!(gate.ready(start));
		gate.commit(start);
		assert!(!gate.ready(start + Duration::from_millis(5)));
		// A pause does not bank credit for a burst afterwards.
		let mut gate = FrameGate::default();
		assert!(gate.admit(start));
		let resumed = start + Duration::from_secs(1);
		assert!(gate.admit(resumed));
		assert!(!gate.admit(resumed + Duration::from_millis(5)));
	}

	/// Flat pictures compress to almost nothing; vary one row so size checks bite.
	fn textured(value: u8) -> Vec<u8> {
		let mut rgb = vec![value; WIDTH * HEIGHT * 3];
		for (index, pixel) in rgb
			.as_chunks_mut::<3>()
			.0
			.iter_mut()
			.take(WIDTH)
			.enumerate()
		{
			*pixel = [(index % 251) as u8, value, (index % 97) as u8];
		}
		rgb
	}
}
