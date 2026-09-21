//! User-started, ephemeral camera capture. No device is opened before `start`.

use openh264::{
	OpenH264API,
	encoder::{BitRate, Encoder, EncoderConfig, FrameRate, Profile},
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
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;
pub const MAX_ENCODED_BYTES: usize = 128 * 1024;
pub const SUPPORTED: bool = cfg!(any(
	target_os = "macos",
	target_os = "windows",
	target_os = "linux"
));
const FRAME_INTERVAL: Duration = Duration::from_millis(67);
// Includes asynchronous teardown: rapid toggles cannot accumulate camera workers.
static RUNNING: AtomicBool = AtomicBool::new(false);

pub struct Frame {
	pub rgb: Vec<u8>,
	pub h264: Vec<u8>,
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
	#[cfg(not(target_os = "windows"))]
	{
		Err("Camera selection is currently available on Windows only")
	}
}

impl Camera {
	/// Call only after an explicit camera-on gesture in a connected call.
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
		#[cfg(not(target_os = "windows"))]
		if device.is_some() {
			return Err("Camera selection is currently available on Windows only");
		}
		if RUNNING
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.is_err()
		{
			return Err("Previous camera session is still closing; try again shortly");
		}
		let shared = Arc::new(Shared::default());
		let worker = shared.clone();
		if thread::Builder::new()
			.name("serein-camera".into())
			.spawn(move || {
				let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
					#[cfg(target_os = "macos")]
					{
						objc2::rc::autoreleasepool(|_| macos::run(&worker, &on_frame, &wake))
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
/// to openh264 when it is unavailable or fails mid-stream. Baseline profile and one IDR per
/// picture either way, so the wire format does not change.
struct CameraEncoder {
	diagnostics: crate::diagnostics::EncoderRegistration,
	software: Option<Encoder>,
	yuv: YUVBuffer,
	#[cfg(any(target_os = "macos", target_os = "windows"))]
	hardware: Option<crate::video_encode::hardware::Encoder>,
	#[cfg(target_os = "linux")]
	hardware: Option<encode_linux::Encoder>,
}

impl CameraEncoder {
	fn new() -> Result<Self, &'static str> {
		#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
		let config = crate::video_encode::Config {
			width: WIDTH as u32,
			height: HEIGHT as u32,
			fps: 15,
			bit_rate: 600_000,
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
		})
	}

	/// Encodes one 640×480 packed RGB picture, keeping the frame's pixels for local preview.
	fn encode(&mut self, rgb: Vec<u8>) -> Result<Option<Frame>, &'static str> {
		if rgb.len() != WIDTH * HEIGHT * 3 {
			return Err("Camera did not provide a bounded 640×480 RGB frame");
		}
		#[cfg(target_os = "linux")]
		if let Some(hardware) = self.hardware.as_mut() {
			// ponytail: one IDR per picture, matching the software encoder, because the sender
			// drops to the latest frame; inter prediction would freeze a receiver instead.
			match hardware.encode(&rgb) {
				Ok(Some(h264)) => {
					if h264.len() > MAX_ENCODED_BYTES {
						return Err("Camera encoded frame exceeded its 128 KiB limit");
					}
					return Ok(Some(Frame { rgb, h264 }));
				}
				// The encoder is still holding this picture; the next one returns it.
				Ok(None) => return Ok(None),
				Err(_) => {
					self.hardware = None;
					self.diagnostics.set(None);
					self.software = Some(encoder()?);
					self.diagnostics.set(Some(false));
				}
			}
		}
		#[cfg(any(target_os = "macos", target_os = "windows"))]
		if let Some(hardware) = self.hardware.as_mut() {
			// ponytail: every picture stays independently decodable, matching the software
			// encoder, because the sender drops to the latest frame; inter prediction would
			// freeze a receiver until the next refresh.
			#[cfg(target_os = "macos")]
			let encoded = hardware.encode(&rgb, (WIDTH, HEIGHT), true);
			#[cfg(target_os = "windows")]
			let encoded = {
				use openh264::formats::YUVSource;
				self.yuv.read_rgb8(RgbSliceU8::new(&rgb, (WIDTH, HEIGHT)));
				hardware.encode(self.yuv.y(), self.yuv.u(), self.yuv.v(), true)
			};
			match encoded {
				Ok((h264, _)) => {
					if h264.len() > MAX_ENCODED_BYTES {
						return Err("Camera encoded frame exceeded its 128 KiB limit");
					}
					return Ok((!h264.is_empty()).then_some(Frame { rgb, h264 }));
				}
				Err(_) => {
					self.hardware = None;
					self.diagnostics.set(None);
					self.software = Some(encoder()?);
					self.diagnostics.set(Some(false));
				}
			}
		}
		let software = self
			.software
			.as_mut()
			.ok_or("Camera H264 encoder could not start")?;
		encode_rgb(software, &mut self.yuv, rgb)
	}
}

fn encoder() -> Result<Encoder, &'static str> {
	Encoder::with_api_config(
		OpenH264API::from_source(),
		EncoderConfig::new()
			.bitrate(BitRate::from_bps(600_000))
			.max_frame_rate(FrameRate::from_hz(15.0))
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
) -> Result<Option<Frame>, &'static str> {
	if rgb.len() != WIDTH * HEIGHT * 3 {
		return Err("Camera did not provide a bounded 640×480 RGB frame");
	}
	yuv.read_rgb8(RgbSliceU8::new(&rgb, (WIDTH, HEIGHT)));
	// ponytail: independently decodable frames tolerate latest-slot drops;
	// add feedback-aware inter frames when bandwidth adaptation is implemented.
	encoder.force_intra_frame();
	let bits = encoder
		.encode(yuv)
		.map_err(|_| "Camera frame could not be encoded")?;
	if bits.raw_info().iFrameSizeInBytes < 0
		|| bits.raw_info().iFrameSizeInBytes as usize > MAX_ENCODED_BYTES
	{
		return Err("Camera encoded frame exceeded its 128 KiB limit");
	}
	let h264 = bits.to_vec();
	Ok((!h264.is_empty()).then_some(Frame { rgb, h264 }))
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
		if !shared.stopped.load(Ordering::Acquire)
			&& let Some(frame) = encoder.encode(rgb)?
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
		windows::run(shared, device, &mut emit)
	}
	#[cfg(target_os = "linux")]
	{
		let _ = device;
		linux::run(shared, &mut emit)
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
	use objc2_core_media::CMSampleBuffer;
	use objc2_core_video::*;
	use objc2_foundation::{NSDictionary, NSNumber, NSString};
	use std::{
		sync::mpsc::{self, Receiver, SyncSender},
		time::{Duration, Instant},
	};

	const DENIED: &str = "Camera access denied. Allow Serein (or your terminal) in System Settings > Privacy & Security > Camera, then try again.";

	struct DelegateState {
		send: SyncSender<Result<Vec<u8>, &'static str>>,
		shared: Arc<Shared>,
		last: Mutex<Instant>,
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
				let Ok(mut last) = state.last.try_lock() else {
					return;
				};
				if last.elapsed() < FRAME_INTERVAL {
					return;
				}
				*last = Instant::now();
				let _ = state.send.try_send(copy_bgra(sample));
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
				return Err("Camera did not provide a bounded 640×480 BGRA frame");
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
			let device = AVCaptureDevice::defaultDeviceWithMediaType(
				AVMediaTypeVideo.ok_or("Camera media type unavailable")?,
			)
			.ok_or("No camera is available")?;
			let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device)
				.map_err(|_| "Camera is busy or unavailable")?;
			let session = AVCaptureSession::new();
			let output = AVCaptureVideoDataOutput::new();
			if !session.canAddInput(&input) || !session.canAddOutput(&output) {
				return Err("Camera cannot join capture session");
			}
			session.addInput(&input);
			session.addOutput(&output);
			if !session.canSetSessionPreset(AVCaptureSessionPreset640x480) {
				return Err("Camera does not support 640×480 capture");
			}
			session.setSessionPreset(AVCaptureSessionPreset640x480);
			let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
			let width = NSNumber::new_usize(WIDTH);
			let height = NSNumber::new_usize(HEIGHT);
			let format_key = NSString::from_str(&kCVPixelBufferPixelFormatTypeKey.to_string());
			let width_key = NSString::from_str(&kCVPixelBufferWidthKey.to_string());
			let height_key = NSString::from_str(&kCVPixelBufferHeightKey.to_string());
			// The session preset alone does not fix the video data output dimensions.
			let settings = NSDictionary::from_slices(
				&[&*format_key, &*width_key, &*height_key],
				&[&*format as &AnyObject, &*width, &*height],
			);
			output.setVideoSettings(Some(&settings));
			output.setAlwaysDiscardsLateVideoFrames(true);
			let allocated = SereinCameraDelegate::alloc().set_ivars(DelegateState {
				send,
				shared: shared.clone(),
				last: Mutex::new(Instant::now() - FRAME_INTERVAL),
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
			let Some(frame) = encoder.encode(rgb)? else {
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
	fn hardware_camera_frames_decode_and_stay_independently_decodable() {
		let mut encoder = CameraEncoder::new().unwrap();
		// Exactly one encoder is live: hardware when the machine offers it, openh264 otherwise.
		#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
		assert_eq!(encoder.hardware.is_some(), encoder.software.is_none());
		for length in [0, WIDTH * HEIGHT * 3 - 1, WIDTH * HEIGHT * 3 + 1] {
			assert!(encoder.encode(vec![0; length]).is_err());
		}
		let mut decoder = openh264::decoder::Decoder::new().unwrap();
		for value in [0, 96, 255] {
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
			assert!(frame.h264.len() <= MAX_ENCODED_BYTES);
			// The sender drops to the latest frame, so each picture must stand alone.
			assert!(crate::video_receive::is_keyframe(&frame.h264));
			assert!(crate::video_receive::has_parameter_sets(&frame.h264));
			let decoded = decoder.decode(&frame.h264).unwrap().unwrap();
			assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
		}
	}

	#[test]
	fn camera_frames_are_bounded_independently_decodable_and_stop_is_immediate() {
		let mut encoder = encoder().unwrap();
		let mut yuv = YUVBuffer::new(WIDTH, HEIGHT);
		for length in [0, WIDTH * HEIGHT * 3 - 1, WIDTH * HEIGHT * 3 + 1] {
			assert!(encode_rgb(&mut encoder, &mut yuv, vec![0; length]).is_err());
		}
		for value in [0, 127, 255] {
			let frame = encode_rgb(&mut encoder, &mut yuv, vec![value; WIDTH * HEIGHT * 3])
				.unwrap()
				.unwrap();
			assert!(frame.h264.len() <= MAX_ENCODED_BYTES);
			let mut decoder = openh264::decoder::Decoder::new().unwrap();
			let decoded = decoder.decode(&frame.h264).unwrap().unwrap();
			assert_eq!(decoded.dimensions(), (WIDTH, HEIGHT));
		}
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
	}
}
