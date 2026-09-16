//! Explicitly selected, memory-only screen capture and H.264 encoding.
pub use client_core::screen::{Settings, Source, SourceId};
#[cfg(any(target_os = "linux", all(test, target_os = "macos")))]
#[cfg_attr(all(test, target_os = "macos"), allow(dead_code))]
#[path = "screen/audio_linux.rs"]
mod audio_linux;
#[cfg(all(test, not(target_os = "windows")))]
#[path = "screen/audio_windows.rs"]
mod audio_windows;
#[cfg(not(target_os = "linux"))]
#[path = "screen/capture.rs"]
mod capture;
#[cfg(target_os = "linux")]
#[path = "screen/gstreamer.rs"]
mod gstreamer;
#[cfg(target_os = "linux")]
#[path = "screen/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
#[path = "screen/portal_linux.rs"]
mod portal_linux;

use openh264::{
	OpenH264API,
	encoder::{
		BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod,
		RateControlMode, UsageType,
	},
	formats::{BgraSliceU8, YUVBuffer},
};
use std::sync::{
	Arc, Mutex,
	atomic::{AtomicBool, AtomicU64, Ordering},
	mpsc,
};

#[cfg(not(target_os = "linux"))]
use std::time::{Duration, Instant};

pub const MAX_RAW_BYTES: usize = 3840 * 2160 * 4;
pub const MAX_ENCODED_BYTES: usize = 2 * 1024 * 1024;

pub struct RawFrame {
	pub width: u32,
	pub height: u32,
	pub stride: usize,
	pub data: Vec<u8>,
}

pub struct EncodedFrame {
	pub data: Vec<u8>,
	pub timestamp: u32,
	pub keyframe: bool,
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
	pub settings: Settings,
	pub frames: tokio::sync::mpsc::Receiver<EncodedFrame>,
	pub ready: Arc<AtomicBool>,
	pub keyframe: Arc<AtomicBool>,
	/// Interleaved 48 kHz stereo system audio, present only when the share requested it.
	pub audio: Option<tokio::sync::mpsc::Receiver<AudioChunk>>,
	pub audio_epoch: Arc<AtomicU64>,
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
	return Ok(vec![Source {
		id: SourceId::Portal,
		name: "Choose in the system picker".into(),
	}]);
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
		settings: Settings,
		wake: impl Fn() + Send + 'static,
	) -> Result<(Self, Video), &'static str> {
		if !settings.valid() || !supported() {
			return Err("Screen sharing is unavailable for these settings or this platform");
		}
		let stop = Arc::new(AtomicBool::new(false));
		let ready = Arc::new(AtomicBool::new(false));
		let audio_epoch = Arc::new(AtomicU64::new(0));
		let worker_audio_epoch = audio_epoch.clone();
		let keyframe = Arc::new(AtomicBool::new(true));
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
				let finished_ready = worker_ready.clone();
				#[cfg(target_os = "linux")]
				let result = linux::run(
					settings,
					worker_stop,
					worker_ready,
					worker_keyframe,
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
					worker_stop,
					worker_ready,
					worker_keyframe,
					send,
					audio_send,
					worker_audio_epoch,
					worker_preview,
					&wake,
				);
				finished_ready.store(false, Ordering::Release);
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
				settings,
				frames,
				ready,
				keyframe,
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
	settings: Settings,
	stop: Arc<AtomicBool>,
	ready: Arc<AtomicBool>,
	keyframe: Arc<AtomicBool>,
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
	let (raw_send, raw) = mpsc::sync_channel(1);
	let capture_stop = Arc::new(AtomicBool::new(false));
	#[cfg(target_os = "windows")]
	let raw_pending = Arc::new(AtomicBool::new(false));
	#[cfg(target_os = "windows")]
	let _native = capture::Capture::start(
		settings,
		raw_send,
		audio,
		capture_stop.clone(),
		ready.clone(),
		audio_epoch,
		raw_pending.clone(),
	)?;
	#[cfg(not(target_os = "windows"))]
	let _native = capture::Capture::start(
		settings,
		raw_send,
		audio,
		capture_stop.clone(),
		ready.clone(),
		audio_epoch,
	)?;
	let mut encoding = None;
	let mut first_frame_deadline = Some(Instant::now() + Duration::from_secs(15));
	let mut next_frame = Instant::now();
	let mut next_preview = Instant::now();
	let mut latest_frame = None;

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
		let frame = match raw.recv_timeout(Duration::from_millis(100)) {
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
		let now = Instant::now();
		if let Some(frame) = &frame {
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
			keyframe.load(Ordering::Acquire),
		)?;
		// Local capture remains available while alone; only secure media is encoded or queued.
		if !ready.load(Ordering::Acquire) {
			encoding = None;
			keyframe.store(true, Ordering::Release);
			continue;
		}
		if !encode {
			continue;
		}
		// Pace on an accumulating schedule with a little tolerance: capture timing jitter must
		// not skip every other frame, and a stalled encoder resumes from now instead of bursting.
		let interval = Duration::from_secs_f64(1.0 / f64::from(settings.fps));
		if now + Duration::from_millis(2) < next_frame {
			continue;
		}
		next_frame = (next_frame + interval).max(now + interval / 2);
		// Drop before encoding while the transport is behind, so the encoder never references
		// a picture the receiver did not get.
		if send.capacity() == 0 {
			continue;
		}
		if encoding.is_none() {
			encoding = Some(ScreenEncoder::new(settings)?);
		}
		let pixels = fit_frame(
			latest_frame.take().expect("latest screen frame"),
			settings.width,
			settings.height,
		)?;
		// Retain one current source snapshot, never encoded media, for a viewer's keyframe
		// request on an unchanged desktop. Replacing it with fitted pixels avoids a copy.
		latest_frame = Some(RawFrame {
			width: settings.width,
			height: settings.height,
			stride: settings.width as usize * 4,
			data: pixels,
		});
		let force_keyframe = keyframe.swap(false, Ordering::AcqRel);
		let (data, is_keyframe) = encoding.as_mut().expect("secure screen encoder").encode(
			&latest_frame.as_ref().expect("fitted screen frame").data,
			(settings.width as usize, settings.height as usize),
			force_keyframe,
		)?;
		if force_keyframe && (data.is_empty() || !is_keyframe) {
			keyframe.store(true, Ordering::Release);
		}
		if data.is_empty() {
			continue;
		}
		if !ready.load(Ordering::Acquire) || stop.load(Ordering::Acquire) {
			continue;
		}
		let frame = EncodedFrame {
			data,
			timestamp: (origin.elapsed().as_micros() * 90 / 1000) as u32,
			keyframe: is_keyframe,
		};
		if send.try_send(frame).is_err() {
			keyframe.store(true, Ordering::Release);
		}
	}
	capture_stop.store(true, Ordering::Release);
	Ok(())
}

#[cfg(any(test, not(target_os = "linux")))]
fn retain_screen_frame(
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

/// Screen encoder preferring the platform hardware H.264 encoder (Media Foundation on
/// Windows, VideoToolbox on macOS) and falling back to openh264 when it is unavailable or
/// fails mid-stream.
#[cfg(not(target_os = "linux"))]
struct ScreenEncoder {
	software: Option<Encoder>,
	yuv: YUVBuffer,
	hardware: Option<crate::video_encode::hardware::Encoder>,
	settings: Settings,
}

#[cfg(not(target_os = "linux"))]
impl ScreenEncoder {
	fn new(settings: Settings) -> Result<Self, &'static str> {
		let config = crate::video_encode::Config {
			width: settings.width,
			height: settings.height,
			fps: settings.fps,
			bit_rate: settings.bit_rate(),
			max_bytes: MAX_ENCODED_BYTES,
			profile: crate::video_encode::Profile::Main,
		};
		#[cfg(target_os = "macos")]
		let hardware = crate::video_encode::hardware::Encoder::new(
			config,
			crate::video_encode::SourceFormat::Bgra,
		)
		.ok();
		#[cfg(target_os = "windows")]
		let hardware = crate::video_encode::hardware::Encoder::new(config).ok();
		let software = if hardware.is_none() {
			Some(encoder(settings)?)
		} else {
			None
		};
		Ok(Self {
			software,
			yuv: YUVBuffer::new(settings.width as usize, settings.height as usize),
			hardware,
			settings,
		})
	}

	fn encode(
		&mut self,
		pixels: &[u8],
		dimensions: (usize, usize),
		force_keyframe: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let mut software_force = force_keyframe;
		if let Some(hardware) = self.hardware.as_mut() {
			#[cfg(target_os = "macos")]
			let encoded = hardware.encode(pixels, dimensions, force_keyframe);
			#[cfg(target_os = "windows")]
			let encoded = {
				use openh264::formats::YUVSource;
				self.yuv.read_bgra8(BgraSliceU8::new(pixels, dimensions));
				hardware.encode(self.yuv.y(), self.yuv.u(), self.yuv.v(), force_keyframe)
			};
			if let Ok(encoded) = encoded {
				return Ok(encoded);
			}
			// The viewer must restart from a keyframe once the software encoder takes over.
			self.hardware = None;
			self.software = Some(encoder(self.settings)?);
			software_force = true;
		}
		self.yuv.read_bgra8(BgraSliceU8::new(pixels, dimensions));
		encode_yuv(
			self.software.as_mut().expect("software screen encoder"),
			&self.yuv,
			software_force,
		)
	}
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn i420_to_nv12(
	y: &[u8],
	u: &[u8],
	v: &[u8],
	output: &mut [u8],
) -> Result<(), &'static str> {
	if u.len() != v.len() || y.len() != u.len() * 4 || output.len() != y.len() + u.len() + v.len() {
		return Err("Invalid screen encoder color planes");
	}
	output[..y.len()].copy_from_slice(y);
	for (pair, (&u, &v)) in output[y.len()..]
		.as_chunks_mut::<2>()
		.0
		.iter_mut()
		.zip(u.iter().zip(v))
	{
		pair.copy_from_slice(&[u, v]);
	}
	Ok(())
}

pub(super) fn encoder(settings: Settings) -> Result<Encoder, &'static str> {
	let config = EncoderConfig::new()
		.bitrate(BitRate::from_bps(settings.bit_rate()))
		.max_frame_rate(FrameRate::from_hz(settings.fps as f32))
		.usage_type(UsageType::ScreenContentRealTime)
		.rate_control_mode(RateControlMode::Bitrate)
		.complexity(if cfg!(target_os = "windows") {
			Complexity::Low
		} else {
			Complexity::Medium
		})
		.num_threads(encoder_threads())
		.intra_frame_period(IntraFramePeriod::from_num_frames(settings.fps * 2));
	Encoder::with_api_config(OpenH264API::from_source(), config)
		.map_err(|_| "Screen video encoder is unavailable")
}

fn encoder_threads() -> u16 {
	std::thread::available_parallelism().map_or(2, |count| count.get().clamp(2, 8) as u16)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(super) fn encode_pixels(
	encoder: &mut Encoder,
	yuv: &mut YUVBuffer,
	pixels: &[u8],
	dimensions: (usize, usize),
	force_keyframe: bool,
) -> Result<(Vec<u8>, bool), &'static str> {
	yuv.read_bgra8(BgraSliceU8::new(pixels, dimensions));
	encode_yuv(encoder, yuv, force_keyframe)
}

fn encode_yuv(
	encoder: &mut Encoder,
	yuv: &YUVBuffer,
	force_keyframe: bool,
) -> Result<(Vec<u8>, bool), &'static str> {
	if force_keyframe {
		encoder.force_intra_frame();
	}
	let encoded = encoder
		.encode(yuv)
		.map_err(|_| "Screen video encoding failed")?;
	let is_keyframe = matches!(encoded.frame_type(), FrameType::IDR);
	let mut encoded_len = 0usize;
	for layer_index in 0..encoded.num_layers() {
		let layer = encoded
			.layer(layer_index)
			.ok_or("Screen video encoder returned an invalid layer")?;
		for nal_index in 0..layer.nal_count() {
			encoded_len = encoded_len
				.checked_add(
					layer
						.nal_unit(nal_index)
						.ok_or("Screen video encoder returned an invalid NAL")?
						.len(),
				)
				.filter(|length| *length <= MAX_ENCODED_BYTES)
				.ok_or("Encoded screen frame exceeds the sharing limit; choose a lower quality")?;
		}
	}
	let mut data = Vec::with_capacity(encoded_len);
	encoded.write_vec(&mut data);
	Ok((data, is_keyframe))
}

fn validate_frame(frame: &RawFrame) -> Result<(usize, usize), &'static str> {
	let row_bytes = (frame.width as usize)
		.checked_mul(4)
		.ok_or("Screen capture returned an unsupported frame size")?;
	let required = frame
		.stride
		.checked_mul(frame.height as usize)
		.ok_or("Screen capture returned an unsupported frame size")?;
	if frame.width == 0
		|| frame.height == 0
		|| frame.width > 3840
		|| frame.height > 2160
		|| frame.data.len() > MAX_RAW_BYTES
		|| frame.stride < row_bytes
		|| required > frame.data.len()
	{
		return Err("Screen capture returned an unsupported frame size");
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
	// ponytail: nearest sampling keeps the ten-fps preview cheap; use filtered scaling if needed.
	Ok(image::RgbaImage::from_fn(width, height, |x, y| {
		let source_x = (x * frame.width / width) as usize;
		let source_y = (y * frame.height / height) as usize;
		let offset = source_y * frame.stride + source_x * 4;
		image::Rgba([
			frame.data[offset + 2],
			frame.data[offset + 1],
			frame.data[offset],
			255,
		])
	}))
}

#[cfg(any(test, not(target_os = "linux")))]
fn fit_frame(frame: RawFrame, width: u32, height: u32) -> Result<Vec<u8>, &'static str> {
	let (row_bytes, required) = validate_frame(&frame)?;

	let mut packed = if frame.stride == row_bytes {
		frame.data
	} else {
		let mut packed = Vec::with_capacity(row_bytes * frame.height as usize);
		for row in frame.data[..required].chunks_exact(frame.stride) {
			packed.extend_from_slice(&row[..row_bytes]);
		}
		packed
	};
	packed.truncate(row_bytes * frame.height as usize);
	if frame.width == width && frame.height == height {
		return Ok(packed);
	}
	let image = image::RgbaImage::from_raw(frame.width, frame.height, packed)
		.ok_or("Invalid screen frame")?;
	let scale = (f64::from(width) / f64::from(frame.width))
		.min(f64::from(height) / f64::from(frame.height));
	let (scaled_width, scaled_height) = (
		(f64::from(frame.width) * scale).round().max(1.0) as u32,
		(f64::from(frame.height) * scale).round().max(1.0) as u32,
	);
	let scaled = image::imageops::resize(
		&image,
		scaled_width,
		scaled_height,
		image::imageops::FilterType::Triangle,
	);
	let mut output = image::RgbaImage::new(width, height);
	image::imageops::replace(
		&mut output,
		&scaled,
		i64::from((width - scaled_width) / 2),
		i64::from((height - scaled_height) / 2),
	);
	Ok(output.into_raw())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn idle_screen_keyframe_uses_latest_snapshot_only_when_ready() {
		let frame = |value| RawFrame {
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
		oversized.width = 3841;
		assert!(retain_screen_frame(&mut latest, Some(oversized), true, true).is_err());
		assert_eq!(latest.unwrap().data, vec![2; 16]);
	}

	#[test]
	fn local_preview_is_bounded_and_converts_padded_bgra_without_media_readiness() {
		let preview = preview_frame(&RawFrame {
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
		for (width, height) in [(3840, 2160), (2, 2160), (3840, 2)] {
			let frame = RawFrame {
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
			width: 2,
			height: 2,
			stride: 12,
			data: vec![255; 24],
		};
		let pixels = fit_frame(frame, 1280, 720).unwrap();
		assert_eq!(pixels.len(), 1280 * 720 * 4);

		let settings = Settings {
			source: SourceId::Display(1),
			width: 1280,
			height: 720,
			fps: 30,
			cursor: true,
			audio: false,
		};
		let mut encoder = encoder(settings).unwrap();
		let mut yuv = YUVBuffer::new(1280, 720);
		let (encoded, keyframe) =
			encode_pixels(&mut encoder, &mut yuv, &pixels, (1280, 720), true).unwrap();
		assert!(keyframe);
		assert!(!encoded.is_empty() && encoded.len() <= MAX_ENCODED_BYTES);
		assert!(encoded.windows(5).any(|nal| nal == [0, 0, 0, 1, 0x65]));

		assert!(
			fit_frame(
				RawFrame {
					width: 2,
					height: 2,
					stride: 4,
					data: vec![0; 8],
				},
				1280,
				720,
			)
			.is_err()
		);
	}

	#[test]
	fn interleaves_i420_chroma_for_windows_nv12() {
		let mut nv12 = [0; 12];
		i420_to_nv12(&[1, 2, 3, 4, 5, 6, 7, 8], &[9, 10], &[11, 12], &mut nv12).unwrap();
		assert_eq!(nv12, [1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 10, 12]);
		assert!(i420_to_nv12(&[0; 4], &[0; 2], &[0; 2], &mut [0; 8]).is_err());
	}
}
