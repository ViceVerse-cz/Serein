//! ScreenCaptureKit adapter. Frame access is the only unsafe operation.
#![allow(unsafe_code)]

use super::{
	MAX_FRAME_HEIGHT, MAX_FRAME_WIDTH, MAX_RAW_BYTES, MAX_SOURCE_HEIGHT, MAX_SOURCE_WIDTH,
	MAX_SOURCES, bounded_name,
};
use crate::screen::{RawFrame, Settings, Source, SourceId};
use screencapturekit::{
	cm::{CMSampleBufferExt, CMSampleBufferSCExt},
	cv::CVPixelBufferLockFlags,
	prelude::*,
	stream::delegate_trait::SCStreamDelegateTrait,
};
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
	mpsc::SyncSender,
};

fn initialize() {
	static ONCE: std::sync::Once = std::sync::Once::new();
	ONCE.call_once(|| {
		// SAFETY: the no-argument bridge initializes CoreGraphics via CGMainDisplayID.
		unsafe { screencapturekit::ffi::sc_initialize_core_graphics() };
	});
}

pub(crate) fn sources() -> Result<Vec<Source>, &'static str> {
	initialize();
	let content = SCShareableContent::get().map_err(
		|_| "Screen sources are unavailable. Allow screen recording in System Settings, then refresh.",
	)?;
	let snapshot = content.snapshot().ok_or("Screen sources are unavailable")?;
	let mut sources = Vec::with_capacity(MAX_SOURCES);

	for display in snapshot.displays {
		let (Ok(width), Ok(height)) = (u32::try_from(display.width), u32::try_from(display.height))
		else {
			continue;
		};
		if width == 0 || height == 0 || width > MAX_SOURCE_WIDTH || height > MAX_SOURCE_HEIGHT {
			continue;
		}
		sources.push(Source {
			id: SourceId::Display(u64::from(display.display_id)),
			name: format!("Display {}", display.display_id),
		});
		if sources.len() == MAX_SOURCES {
			return Ok(sources);
		}
	}

	for window in snapshot.windows {
		let Some(title) = window.title.filter(|title| !title.trim().is_empty()) else {
			continue;
		};
		if !window.is_on_screen
			|| window.window_layer != 0
			|| !window.frame.size.width.is_finite()
			|| !window.frame.size.height.is_finite()
			|| window.frame.size.width <= 0.0
			|| window.frame.size.height <= 0.0
			|| window.frame.size.width > f64::from(MAX_SOURCE_WIDTH)
			|| window.frame.size.height > f64::from(MAX_SOURCE_HEIGHT)
		{
			continue;
		}
		let app = window
			.owning_app_index
			.and_then(|index| snapshot.applications.get(index))
			.map(|app| app.application_name.trim())
			.filter(|name| !name.is_empty());
		let name = app.map_or(title.clone(), |app| format!("{app} — {title}"));
		sources.push(Source {
			id: SourceId::Window(u64::from(window.window_id)),
			name: bounded_name(name),
		});
		if sources.len() == MAX_SOURCES {
			break;
		}
	}
	Ok(sources)
}

struct Handler {
	frames: SyncSender<RawFrame>,
	stop: Arc<AtomicBool>,
}

/// System audio as interleaved 48 kHz stereo `f32`, bounded per buffer and never stored.
struct AudioHandler {
	audio: tokio::sync::mpsc::Sender<crate::screen::AudioChunk>,
	stop: Arc<AtomicBool>,
	ready: Arc<AtomicBool>,
	audio_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl SCStreamOutputTrait for AudioHandler {
	fn did_output_sample_buffer(&self, sample: CMSampleBuffer, kind: SCStreamOutputType) {
		let epoch = self.audio_epoch.load(Ordering::Acquire);
		if kind != SCStreamOutputType::Audio
			|| self.stop.load(Ordering::Acquire)
			|| !self.ready.load(Ordering::Acquire)
			|| self.audio.capacity() == 0
		{
			return;
		}
		let Some(list) = sample.audio_buffer_list() else {
			return;
		};
		fn samples(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
			bytes
				.as_chunks::<4>()
				.0
				.iter()
				.map(|b| f32::from_ne_bytes(*b))
				.map(|s| {
					if s.is_finite() {
						s.clamp(-1.0, 1.0)
					} else {
						0.0
					}
				})
		}

		let interleaved: Vec<f32> = match list.num_buffers() {
			// Planar: one buffer per channel.
			2 => {
				let (Some(left), Some(right)) = (list.buffer(0), list.buffer(1)) else {
					return;
				};
				samples(left.data())
					.zip(samples(right.data()))
					.take(crate::screen::MAX_AUDIO_SAMPLES / 2)
					.flat_map(|(l, r)| [l, r])
					.collect()
			}
			1 => {
				let Some(buffer) = list.buffer(0) else {
					return;
				};
				if buffer.number_channels() == 2 {
					samples(buffer.data())
						.take(crate::screen::MAX_AUDIO_SAMPLES)
						.collect()
				} else {
					samples(buffer.data())
						.take(crate::screen::MAX_AUDIO_SAMPLES / 2)
						.flat_map(|s| [s, s])
						.collect()
				}
			}
			_ => return,
		};
		if interleaved.is_empty() || !interleaved.len().is_multiple_of(2) {
			return;
		}
		// A full queue drops audio rather than blocking the capture callback.
		if self.ready.load(Ordering::Acquire) && !self.stop.load(Ordering::Acquire) {
			let _ = self.audio.try_send(crate::screen::AudioChunk {
				samples: interleaved,
				epoch,
			});
		}
	}
}

impl SCStreamOutputTrait for Handler {
	fn did_output_sample_buffer(&self, sample: CMSampleBuffer, kind: SCStreamOutputType) {
		if kind != SCStreamOutputType::Screen
			|| self.stop.load(Ordering::Acquire)
			|| sample
				.frame_status()
				.is_some_and(|status| !status.has_content())
		{
			return;
		}
		let Some(pixel_buffer) = sample.pixel_buffer() else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		let Ok(guard) = pixel_buffer.lock(CVPixelBufferLockFlags::READ_ONLY) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		let (width, height) = (guard.width(), guard.height());
		let Some(row_bytes) = width.checked_mul(4) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		let stride = guard.bytes_per_row();
		let Some(source_len) = stride.checked_mul(height) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		let Some(data_len) = row_bytes.checked_mul(height) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		if width == 0
			|| height == 0
			|| width > MAX_FRAME_WIDTH as usize
			|| height > MAX_FRAME_HEIGHT as usize
			|| stride < row_bytes
			|| source_len > MAX_RAW_BYTES
			|| data_len > MAX_RAW_BYTES
		{
			self.stop.store(true, Ordering::Release);
			return;
		}

		// SAFETY: the read-only lock guard owns the pixel-buffer lock for this scope;
		// the crate returns a slice spanning its declared row stride and height.
		let Some(source) = (unsafe { guard.as_slice() }) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		if source.len() < source_len {
			self.stop.store(true, Ordering::Release);
			return;
		}
		let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		let mut data = Vec::with_capacity(data_len);
		for row in source[..source_len].chunks_exact(stride) {
			data.extend_from_slice(&row[..row_bytes]);
		}
		let _ = self.frames.try_send(RawFrame {
			width,
			height,
			stride: row_bytes,
			data,
		});
	}
}

struct Delegate(Arc<AtomicBool>);

impl SCStreamDelegateTrait for Delegate {
	fn did_stop_with_error(&self, _error: screencapturekit::error::SCError) {
		self.0.store(true, Ordering::Release);
	}
}

pub(crate) struct Capture {
	stream: SCStream,
}

impl Capture {
	pub(crate) fn start(
		settings: Settings,
		frames: SyncSender<RawFrame>,
		audio: Option<tokio::sync::mpsc::Sender<crate::screen::AudioChunk>>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		audio_epoch: Arc<std::sync::atomic::AtomicU64>,
	) -> Result<Self, &'static str> {
		initialize();
		if settings.width == 0
			|| settings.height == 0
			|| settings.width > MAX_FRAME_WIDTH
			|| settings.height > MAX_FRAME_HEIGHT
			|| settings.fps == 0
		{
			return Err("Invalid screen capture settings");
		}
		let content =
			SCShareableContent::get().map_err(|_| "Screen recording permission denied")?;
		let filter = match settings.source {
			SourceId::Portal => return Err("The desktop screen picker is available only on Linux"),
			SourceId::Display(id) => {
				let display = content
					.displays()
					.into_iter()
					.find(|display| u64::from(display.display_id()) == id)
					.ok_or("Selected display is no longer available")?;
				if display.width() > MAX_SOURCE_WIDTH || display.height() > MAX_SOURCE_HEIGHT {
					return Err("Selected display is too large to capture");
				}
				SCContentFilter::create()
					.with_display(&display)
					.with_excluding_windows(&[])
					.build()
			}
			SourceId::Window(id) => {
				let window = content
					.windows()
					.into_iter()
					.find(|window| u64::from(window.window_id()) == id)
					.ok_or("Selected window is no longer available")?;
				let size = window.frame().size;
				if !window.is_on_screen()
					|| window.window_layer() != 0
					|| !size.width.is_finite()
					|| !size.height.is_finite()
					|| size.width <= 0.0
					|| size.height <= 0.0
					|| size.width > f64::from(MAX_SOURCE_WIDTH)
					|| size.height > f64::from(MAX_SOURCE_HEIGHT)
				{
					return Err("Selected window is unavailable or too large");
				}
				SCContentFilter::create().with_window(&window).build()
			}
		};
		let mut config = SCStreamConfiguration::new()
			.with_width(settings.width)
			.with_height(settings.height)
			.with_pixel_format(PixelFormat::BGRA)
			.with_preserves_aspect_ratio(true)
			.with_shows_cursor(settings.cursor)
			.with_fps(settings.fps)
			.with_queue_depth(3);
		if audio.is_some() {
			// Serein's own playback is excluded so the call is not echoed into the stream.
			config = config
				.with_captures_audio(true)
				.with_sample_rate(48_000)
				.with_channel_count(2)
				.with_excludes_current_process_audio(true);
		}
		if !config.preserves_aspect_ratio() {
			return Err("Screen sharing requires macOS 14 or newer");
		}
		let mut stream = SCStream::new_with_delegate(&filter, &config, Delegate(stop.clone()));
		if stream
			.add_output_handler(
				Handler {
					frames,
					stop: stop.clone(),
				},
				SCStreamOutputType::Screen,
			)
			.is_none()
		{
			return Err("Screen capture frame callback could not be registered");
		}
		if let Some(audio) = audio
			&& stream
				.add_output_handler(
					AudioHandler {
						audio,
						stop: stop.clone(),
						ready,
						audio_epoch,
					},
					SCStreamOutputType::Audio,
				)
				.is_none()
		{
			return Err("System audio callback could not be registered");
		}
		stream
			.start_capture()
			.map_err(|_| "Screen capture could not be started")?;
		Ok(Self { stream })
	}
}

impl Drop for Capture {
	fn drop(&mut self) {
		let _ = self.stream.stop_capture();
	}
}
