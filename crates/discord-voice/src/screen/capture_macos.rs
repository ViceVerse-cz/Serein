//! ScreenCaptureKit adapter. Frame access is the only unsafe operation.
#![allow(unsafe_code)]

use super::{
	MAX_FRAME_HEIGHT, MAX_FRAME_WIDTH, MAX_RAW_BYTES, MAX_SOURCE_HEIGHT, MAX_SOURCE_WIDTH,
	MAX_SOURCES, bounded_name,
};
use crate::screen::{RawFrame, RequestSettings, Source, SourceId};
use screencapturekit::{
	cm::{CMSampleBufferExt, CMSampleBufferSCExt},
	content_sharing_picker::{
		SCContentSharingPicker, SCContentSharingPickerConfiguration, SCContentSharingPickerMode,
		SCPickerOutcome,
	},
	cv::CVPixelBufferLockFlags,
	prelude::*,
	stream::delegate_trait::SCStreamDelegateTrait,
};
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
	mpsc::{self, SyncSender},
};

fn initialize() {
	static ONCE: std::sync::Once = std::sync::Once::new();
	ONCE.call_once(|| {
		// SAFETY: the no-argument bridge initializes CoreGraphics via CGMainDisplayID.
		unsafe { screencapturekit::ffi::sc_initialize_core_graphics() };
	});
}

pub(crate) fn sources() -> Result<Vec<Source>, &'static str> {
	if SCContentSharingPicker::is_available() {
		return Ok(vec![Source {
			id: SourceId::SystemPicker,
			name: "Choose with the macOS system picker".into(),
			dimensions: None,
		}]);
	}
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
		if !crate::screen::valid_dimensions(width, height)
			|| width > MAX_SOURCE_WIDTH
			|| height > MAX_SOURCE_HEIGHT
		{
			continue;
		}
		sources.push(Source {
			id: SourceId::Display(u64::from(display.display_id)),
			name: format!("Display {}", display.display_id),
			dimensions: content
				.displays()
				.into_iter()
				.find(|native| native.display_id() == display.display_id)
				.and_then(|native| {
					physical_dimensions(
						&SCContentFilter::create()
							.with_display(&native)
							.with_excluding_windows(&[])
							.build(),
					)
					.ok()
				}),
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
			dimensions: None,
		});
		if sources.len() == MAX_SOURCES {
			break;
		}
	}
	Ok(sources)
}

/// Wait for the picker until it answers, the deadline passes or any `stops` flag is set.
fn await_picker<T>(
	receiver: mpsc::Receiver<Result<T, &'static str>>,
	stops: &[&AtomicBool],
	deadline: std::time::Instant,
) -> Result<T, &'static str> {
	let stopped = || stops.iter().any(|stop| stop.load(Ordering::Acquire));
	loop {
		if stopped() {
			return Err("Screen sharing cancelled");
		}
		let remaining = deadline.saturating_duration_since(std::time::Instant::now());
		if remaining.is_zero() {
			return Err("Screen sharing picker timed out; share again to retry");
		}
		match receiver.recv_timeout(remaining.min(std::time::Duration::from_millis(20))) {
			Ok(result) => {
				return if stopped() {
					Err("Screen sharing cancelled")
				} else {
					result
				};
			}
			Err(mpsc::RecvTimeoutError::Timeout) => {}
			Err(mpsc::RecvTimeoutError::Disconnected) => {
				return Err("Screen sharing picker stopped");
			}
		}
	}
}

fn choose_with_system_picker(
	stop: &AtomicBool,
	teardown: &AtomicBool,
) -> Result<SCContentFilter, &'static str> {
	let mut config = SCContentSharingPickerConfiguration::try_new()
		.ok_or("The macOS sharing picker requires macOS 14 or newer")?;
	config.set_allowed_picker_modes(&[
		SCContentSharingPickerMode::SingleDisplay,
		SCContentSharingPickerMode::SingleWindow,
	]);
	config.set_allows_changing_selected_content(false);
	let (send, receive) = mpsc::sync_channel(1);
	SCContentSharingPicker::show(&config, move |outcome| {
		let result = match outcome {
			SCPickerOutcome::Picked(result) => Ok(result),
			SCPickerOutcome::Cancelled => Err("Screen sharing cancelled"),
			SCPickerOutcome::Error(_) => Err("Could not open the macOS sharing picker"),
		};
		let _ = send.try_send(result);
	});
	let result = await_picker(
		receive,
		&[stop, teardown],
		std::time::Instant::now() + std::time::Duration::from_secs(120),
	);
	if result.is_err() {
		SCContentSharingPicker::deactivate();
	}
	let picked = result?;
	let (width, height) = picked.pixel_size();
	let (points_width, points_height) = picked.size();
	if !crate::screen::valid_dimensions(width, height)
		|| height == 0
		|| width > MAX_SOURCE_WIDTH
		|| height > MAX_SOURCE_HEIGHT
		|| !points_width.is_finite()
		|| !points_height.is_finite()
		|| points_width <= 0.0
		|| points_height <= 0.0
	{
		SCContentSharingPicker::deactivate();
		return Err("Selected screen or window is unavailable or too large");
	}
	Ok(picked.filter())
}

#[derive(Clone)]
struct Handler {
	frames: SyncSender<RawFrame>,
	format: crate::screen::RawFormat,
	stop: Arc<AtomicBool>,
}

/// System audio as interleaved 48 kHz stereo `f32`, bounded per buffer and never stored.
#[derive(Clone)]
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
		let (buffer_width, buffer_height) = (guard.width(), guard.height());
		let expected = if matches!(self.format, crate::screen::RawFormat::LinearRgba16 { .. }) {
			PixelFormat::RGhA
		} else {
			PixelFormat::BGRA
		};
		if PixelFormat::from(guard.pixel_format()) != expected {
			self.stop.store(true, Ordering::Release);
			return;
		}
		// ScreenCaptureKit draws a window of another shape into the corner of the configured
		// picture; keep only that content so the encoder letterboxes it centered.
		let (left, top, width, height) =
			content_pixels(sample.content_rect(), buffer_width, buffer_height);
		let Some(row_bytes) = width.checked_mul(self.format.bytes()) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		let stride = guard.bytes_per_row();
		let Some(source_len) = stride.checked_mul(buffer_height) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		let Some(data_len) = row_bytes.checked_mul(height) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		if !crate::screen::valid_dimensions(width as u32, height as u32)
			|| !crate::screen::valid_dimensions(buffer_width as u32, buffer_height as u32)
			|| buffer_width > MAX_FRAME_WIDTH as usize
			|| buffer_height > MAX_FRAME_HEIGHT as usize
			|| stride < buffer_width * self.format.bytes()
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
		let mut data = Vec::with_capacity(data_len);
		for row in source[..source_len]
			.chunks_exact(stride)
			.skip(top)
			.take(height)
		{
			data.extend_from_slice(&row[left * self.format.bytes()..][..row_bytes]);
		}
		let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
			self.stop.store(true, Ordering::Release);
			return;
		};
		let _ = self.frames.try_send(RawFrame {
			format: self.format,
			width,
			height,
			stride: row_bytes,
			data,
		});
	}
}

/// The frame's content rectangle in buffer pixels: left, top, width, height. Its units are
/// points, but `scalesToFit` makes the content span the picture along one axis, which gives
/// the points-to-pixels factor without trusting a separate display-scale attachment.
fn content_pixels(
	rect: Option<screencapturekit::cg::CGRect>,
	width: usize,
	height: usize,
) -> (usize, usize, usize, usize) {
	let full = (0, 0, width, height);
	let Some(rect) = rect else {
		return full;
	};
	let (x, y, w, h) = (
		rect.origin.x,
		rect.origin.y,
		rect.size.width,
		rect.size.height,
	);
	if !crate::screen::valid_dimensions(width as u32, height as u32)
		|| height == 0
		|| ![x, y, w, h].iter().all(|value| value.is_finite())
		|| x < 0.0
		|| y < 0.0
		|| w <= 0.0
		|| h <= 0.0
	{
		return full;
	}
	let scale = (width as f64 / (x + w)).min(height as f64 / (y + h));
	let left = ((x * scale).round() as usize).min(width - 1);
	let top = ((y * scale).round() as usize).min(height - 1);
	(
		left,
		top,
		((w * scale).round() as usize).clamp(1, width - left),
		((h * scale).round() as usize).clamp(1, height - top),
	)
}

struct Delegate {
	stop: Arc<AtomicBool>,
	// Starting, active, failed. A rejected HDR start must not cancel the SDR retry.
	state: Arc<std::sync::atomic::AtomicU8>,
}

impl SCStreamDelegateTrait for Delegate {
	fn did_stop_with_error(&self, _error: screencapturekit::error::SCError) {
		if self.state.swap(2, Ordering::AcqRel) == 1 {
			self.stop.store(true, Ordering::Release);
		}
	}
}

struct PickerSession(bool);
impl Drop for PickerSession {
	fn drop(&mut self) {
		if self.0 {
			SCContentSharingPicker::deactivate();
		}
	}
}

fn physical_dimensions(filter: &SCContentFilter) -> Result<(u32, u32), &'static str> {
	let info = screencapturekit::shareable_content::SCShareableContentInfo::for_filter(filter)
		.ok_or("Selected screen geometry is unavailable")?;
	let size = info.content_rect().size;
	let scale = f64::from(info.point_pixel_scale());
	let (width, height) = (size.width * scale, size.height * scale);
	if !width.is_finite()
		|| !height.is_finite()
		|| width < 1.0
		|| height < 1.0
		|| width > f64::from(MAX_SOURCE_WIDTH)
		|| height > f64::from(MAX_SOURCE_HEIGHT)
	{
		return Err("Selected screen or window is unavailable or too large");
	}
	let dimensions = (width.round() as u32, height.round() as u32);
	if !crate::screen::valid_dimensions(dimensions.0, dimensions.1) {
		return Err("Selected screen or window exceeds the 8K limit");
	}
	Ok(dimensions)
}

pub(crate) struct Capture {
	next_geometry: std::time::Instant,
	stream: SCStream,
	filter: SCContentFilter,
	config: SCStreamConfiguration,
	handler: Handler,
	audio_handler: Option<AudioHandler>,
	_system_picker: PickerSession,
}

impl Capture {
	pub(crate) fn refresh_dimensions(&mut self) -> Result<(), &'static str> {
		if std::time::Instant::now() < self.next_geometry {
			return Ok(());
		}
		self.next_geometry = std::time::Instant::now() + std::time::Duration::from_secs(1);
		// Refresh information for the approved filter, without enumerating other
		// sources or requiring a separate screen-recording permission grant.
		// Selected content can be temporarily unavailable; retain the approved
		// configuration and retry at the next interval rather than ending the share.
		let Ok((width, height)) = physical_dimensions(&self.filter) else {
			return Ok(());
		};
		if self.config.width() != width || self.config.height() != height {
			self.config.set_width(width).set_height(height);
			self.stream
				.update_configuration(&self.config)
				.map_err(|_| "Could not resize screen capture")?;
		}
		Ok(())
	}

	pub(crate) fn request_frame(
		&mut self,
		frames: &std::sync::mpsc::Receiver<RawFrame>,
	) -> Result<(), &'static str> {
		self.stream
			.stop_capture()
			.map_err(|_| "Screen capture could not be refreshed")?;
		while frames.try_recv().is_ok() {}
		// Recreate the stream without reopening the picker or changing its approved source.
		self.stream = Self::start_stream(
			&self.filter,
			&self.config,
			&self.handler,
			self.audio_handler.as_ref(),
		)?;
		Ok(())
	}

	pub(crate) fn start(
		settings: RequestSettings,
		frames: SyncSender<RawFrame>,
		audio: Option<tokio::sync::mpsc::Sender<crate::screen::AudioChunk>>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		audio_epoch: Arc<std::sync::atomic::AtomicU64>,
		teardown: Arc<AtomicBool>,
	) -> Result<Self, &'static str> {
		initialize();
		if !settings.valid() {
			return Err("Invalid screen capture settings");
		}
		// Own the picker before selection so every later error/cancellation deactivates it.
		let system_picker = PickerSession(settings.source == SourceId::SystemPicker);
		let filter = match settings.source {
			SourceId::SystemPicker => choose_with_system_picker(&stop, &teardown)?,
			SourceId::Display(id) => {
				let content =
					SCShareableContent::get().map_err(|_| "Screen recording permission denied")?;
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
				let content =
					SCShareableContent::get().map_err(|_| "Screen recording permission denied")?;
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
			#[allow(unreachable_patterns)] // Portal may be absent from platform-scoped models.
			_ => return Err("The desktop screen picker is available only on Linux"),
		};
		let (native_width, native_height) = physical_dimensions(&filter)?;
		let luminance = filter
			.included_displays()
			.first()
			.and_then(|display| {
				platform::video::ffmpeg::capture_luminance(u64::from(display.display_id()), false)
			})
			.or_else(|| {
				filter.included_windows().first().and_then(|window| {
					platform::video::ffmpeg::capture_luminance(u64::from(window.window_id()), true)
				})
			});
		let hdr = luminance.is_some();
		let base = if hdr {
			screencapturekit::stream::configuration::SCStreamConfiguration::from_preset(screencapturekit::stream::configuration::SCStreamConfigurationPreset::CaptureHDRStreamCanonicalDisplay)
            .with_color_space_name("kCGColorSpaceExtendedLinearSRGB")
		} else {
			SCStreamConfiguration::new()
		};
		let mut config = base
			.with_width(native_width)
			.with_height(native_height)
			.with_pixel_format(if hdr {
				PixelFormat::RGhA
			} else {
				PixelFormat::BGRA
			})
			.with_preserves_aspect_ratio(true)
			.with_scales_to_fit(true)
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
		let mut handler = Handler {
			format: if hdr {
				{
					let light = luminance.ok_or("HDR capture metadata is unavailable")?;
					crate::screen::RawFormat::LinearRgba16 {
						unit_nits: light.unit_nits,
						white_nits: light.white_nits,
						peak_nits: light.peak_nits,
					}
				}
			} else {
				crate::screen::RawFormat::Bgra8
			},
			frames,
			stop: stop.clone(),
		};
		let audio_handler = audio.map(|audio| AudioHandler {
			audio,
			stop,
			ready,
			audio_epoch,
		});
		let stream = match Self::start_stream(&filter, &config, &handler, audio_handler.as_ref()) {
			Ok(stream) => stream,
			Err(_) if hdr && !handler.stop.load(Ordering::Acquire) => {
				// Keep the picker-approved filter and audio configuration on the SDR retry.
				config
					.set_capture_dynamic_range(
						screencapturekit::stream::configuration::SCCaptureDynamicRange::SDR,
					)
					.set_pixel_format(PixelFormat::BGRA)
					.set_color_space_name("kCGColorSpaceSRGB");
				handler.format = crate::screen::RawFormat::Bgra8;
				Self::start_stream(&filter, &config, &handler, audio_handler.as_ref())?
			}
			Err(error) => return Err(error),
		};
		Ok(Self {
			next_geometry: std::time::Instant::now(),
			stream,
			filter,
			config,
			handler,
			audio_handler,
			_system_picker: system_picker,
		})
	}

	fn start_stream(
		filter: &SCContentFilter,
		config: &SCStreamConfiguration,
		handler: &Handler,
		audio_handler: Option<&AudioHandler>,
	) -> Result<SCStream, &'static str> {
		if handler.stop.load(Ordering::Acquire) {
			return Err("Screen sharing cancelled");
		}
		let state = Arc::new(std::sync::atomic::AtomicU8::new(0));
		let mut stream = SCStream::new_with_delegate(
			filter,
			config,
			Delegate {
				stop: handler.stop.clone(),
				state: state.clone(),
			},
		);
		if stream
			.add_output_handler(handler.clone(), SCStreamOutputType::Screen)
			.is_none()
		{
			return Err("Screen capture frame callback could not be registered");
		}
		if let Some(audio_handler) = audio_handler
			&& stream
				.add_output_handler(audio_handler.clone(), SCStreamOutputType::Audio)
				.is_none()
		{
			return Err("System audio callback could not be registered");
		}
		if handler.stop.load(Ordering::Acquire) {
			return Err("Screen sharing cancelled");
		}
		stream
			.start_capture()
			.map_err(|_| "Screen capture could not be started")?;
		if state
			.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
			.is_err()
		{
			let _ = stream.stop_capture();
			return Err("Screen capture could not be started");
		}
		Ok(stream)
	}
}

impl Drop for Capture {
	fn drop(&mut self) {
		let _ = self.stream.stop_capture();
	}
}

#[cfg(test)]
mod tests {
	use super::{await_picker, content_pixels};
	use screencapturekit::cg::CGRect;
	use std::sync::{atomic::AtomicBool, mpsc};
	use std::time::{Duration, Instant};

	#[test]
	fn system_picker_wait_is_bounded_and_rejects_results_after_teardown() {
		let (send, receive) = mpsc::sync_channel(1);
		send.send(Ok(42u8)).unwrap();
		assert_eq!(
			await_picker(
				receive,
				&[&AtomicBool::new(false)],
				Instant::now() + Duration::from_secs(1)
			),
			Ok(42)
		);
		let (send, receive) = mpsc::sync_channel(1);
		send.send(Ok(42u8)).unwrap();
		assert_eq!(
			await_picker(
				receive,
				&[&AtomicBool::new(true)],
				Instant::now() + Duration::from_secs(1)
			),
			Err("Screen sharing cancelled")
		);
		// Call teardown cancels a pending picker even while capture itself is still running.
		let (_send, receive) = mpsc::sync_channel::<Result<(), &'static str>>(1);
		assert_eq!(
			await_picker(
				receive,
				&[&AtomicBool::new(false), &AtomicBool::new(true)],
				Instant::now() + Duration::from_secs(60)
			),
			Err("Screen sharing cancelled")
		);
		let (_send, receive) = mpsc::sync_channel::<Result<(), &'static str>>(1);
		assert_eq!(
			await_picker(receive, &[&AtomicBool::new(false)], Instant::now()),
			Err("Screen sharing picker timed out; share again to retry")
		);
		let (send, receive) = mpsc::sync_channel::<Result<(), &'static str>>(1);
		drop(send);
		assert_eq!(
			await_picker(
				receive,
				&[&AtomicBool::new(false)],
				Instant::now() + Duration::from_secs(1)
			),
			Err("Screen sharing picker stopped")
		);
	}

	#[test]
	fn content_rect_maps_to_buffer_pixels_in_any_unit() {
		// A portrait window fitted into 1920x1080 at the corner, in Retina points and pixels.
		for (rect, expected) in [
			(CGRect::new(0.0, 0.0, 270.0, 540.0), (0, 0, 540, 1080)),
			(CGRect::new(0.0, 0.0, 540.0, 1080.0), (0, 0, 540, 1080)),
			// Already centered content keeps its offset.
			(CGRect::new(345.0, 0.0, 270.0, 540.0), (690, 0, 540, 1080)),
			(CGRect::new(0.0, 0.0, 960.0, 300.0), (0, 0, 1920, 600)),
		] {
			assert_eq!(content_pixels(Some(rect), 1920, 1080), expected);
		}
		for rect in [
			None,
			Some(CGRect::new(0.0, 0.0, 0.0, 10.0)),
			Some(CGRect::new(f64::NAN, 0.0, 1.0, 1.0)),
		] {
			assert_eq!(content_pixels(rect, 1920, 1080), (0, 0, 1920, 1080));
		}
	}
}
