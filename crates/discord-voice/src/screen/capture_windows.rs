use super::{
	MAX_FRAME_HEIGHT, MAX_FRAME_WIDTH, MAX_RAW_BYTES, MAX_SOURCE_HEIGHT, MAX_SOURCE_WIDTH,
	MAX_SOURCES, bounded_name,
};
use crate::screen::{AudioChunk, RawFrame, Settings as CaptureSettings, Source, SourceId};
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU64, Ordering},
	mpsc::SyncSender,
};
use std::time::{Duration, Instant};
use windows_capture::{
	capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
	frame::Frame,
	graphics_capture_api::InternalCaptureControl,
	monitor::Monitor,
	settings::{
		ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
		MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
	},
	window::Window,
};

#[path = "audio_windows.rs"]
mod audio;

fn monitor_id(monitor: &Monitor) -> u64 {
	u64::try_from(monitor.as_raw_hmonitor() as usize).unwrap_or(0)
}

fn window_id(window: &Window) -> u64 {
	u64::try_from(window.as_raw_hwnd() as usize).unwrap_or(0)
}

pub(crate) fn sources() -> Result<Vec<Source>, &'static str> {
	let mut sources = Vec::with_capacity(MAX_SOURCES);
	for monitor in Monitor::enumerate().map_err(|_| "Displays could not be enumerated")? {
		let (Ok(width), Ok(height)) = (monitor.width(), monitor.height()) else {
			continue;
		};
		if width == 0 || height == 0 || width > MAX_SOURCE_WIDTH || height > MAX_SOURCE_HEIGHT {
			continue;
		}
		let name = monitor
			.name()
			.or_else(|_| monitor.device_string())
			.unwrap_or_else(|_| "Display".to_owned());
		sources.push(Source {
			id: SourceId::Display(monitor_id(&monitor)),
			name: bounded_name(name),
		});
		if sources.len() == MAX_SOURCES {
			return Ok(sources);
		}
	}

	for window in Window::enumerate().map_err(|_| "Windows could not be enumerated")? {
		let Ok(title) = window.title() else {
			continue;
		};
		let (Ok(width), Ok(height)) = (window.width(), window.height()) else {
			continue;
		};
		let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
			continue;
		};
		if title.trim().is_empty()
			|| width == 0
			|| height == 0
			|| width > MAX_SOURCE_WIDTH
			|| height > MAX_SOURCE_HEIGHT
		{
			continue;
		}
		sources.push(Source {
			id: SourceId::Window(window_id(&window)),
			name: bounded_name(title),
		});
		if sources.len() == MAX_SOURCES {
			break;
		}
	}
	Ok(sources)
}

#[derive(Clone)]
struct Flags {
	frames: SyncSender<RawFrame>,
	stop: Arc<AtomicBool>,
	pending: Arc<AtomicBool>,
	next_frame: Instant,
	interval: Duration,
}

struct Handler(Flags);

impl GraphicsCaptureApiHandler for Handler {
	type Flags = Flags;
	type Error = &'static str;

	fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
		Ok(Self(context.flags))
	}

	fn on_frame_arrived(
		&mut self,
		frame: &mut Frame,
		control: InternalCaptureControl,
	) -> Result<(), Self::Error> {
		if self.0.stop.load(Ordering::Acquire) {
			control.stop();
			return Ok(());
		}
		let now = Instant::now();
		if now + Duration::from_millis(2) < self.0.next_frame
			|| self.0.pending.swap(true, Ordering::AcqRel)
		{
			return Ok(());
		}
		self.0.next_frame = (self.0.next_frame + self.0.interval).max(now + self.0.interval / 2);
		let (width, height) = (frame.width(), frame.height());
		let Some(row_bytes) = (width as usize).checked_mul(4) else {
			self.0.stop.store(true, Ordering::Release);
			control.stop();
			return Err("Captured frame dimensions are invalid");
		};
		let Some(data_len) = row_bytes.checked_mul(height as usize) else {
			self.0.stop.store(true, Ordering::Release);
			control.stop();
			return Err("Captured frame dimensions are invalid");
		};
		if width == 0
			|| height == 0
			|| width > MAX_FRAME_WIDTH
			|| height > MAX_FRAME_HEIGHT
			|| data_len > MAX_RAW_BYTES
		{
			self.0.stop.store(true, Ordering::Release);
			control.stop();
			return Err("Captured frame exceeds the 4K limit");
		}
		let mut buffer = frame.buffer().map_err(|_| {
			self.0.stop.store(true, Ordering::Release);
			"Captured frame could not be read"
		})?;
		let stride = buffer.row_pitch() as usize;
		let Some(source_len) = stride.checked_mul(height as usize) else {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame stride is invalid");
		};
		if stride < row_bytes || source_len > MAX_RAW_BYTES {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame stride exceeds the 4K limit");
		}
		let source = buffer.as_raw_buffer();
		if source.len() < source_len {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame buffer is incomplete");
		}
		let mut data = Vec::with_capacity(data_len);
		for row in source[..source_len].chunks_exact(stride) {
			data.extend_from_slice(&row[..row_bytes]);
		}
		if self
			.0
			.frames
			.try_send(RawFrame {
				width,
				height,
				stride: row_bytes,
				data,
			})
			.is_err()
		{
			self.0.pending.store(false, Ordering::Release);
		}
		Ok(())
	}

	fn on_closed(&mut self) -> Result<(), Self::Error> {
		self.0.stop.store(true, Ordering::Release);
		Ok(())
	}
}

pub(crate) struct Capture {
	control: Option<CaptureControl<Handler, &'static str>>,
	audio: Option<audio::Audio>,
}

impl Capture {
	pub(crate) fn start(
		settings: CaptureSettings,
		frames: SyncSender<RawFrame>,
		audio: Option<tokio::sync::mpsc::Sender<AudioChunk>>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		audio_epoch: Arc<AtomicU64>,
		pending: Arc<AtomicBool>,
	) -> Result<Self, &'static str> {
		if settings.width == 0
			|| settings.height == 0
			|| settings.width > MAX_FRAME_WIDTH
			|| settings.height > MAX_FRAME_HEIGHT
			|| settings.fps == 0
		{
			return Err("Invalid screen capture settings");
		}
		let mut capture = match settings.source {
			SourceId::Display(id) => {
				let monitor = Monitor::enumerate()
					.map_err(|_| "Displays could not be enumerated")?
					.into_iter()
					.find(|monitor| monitor_id(monitor) == id)
					.ok_or("Selected display is no longer available")?;
				start_item(settings, monitor, frames, stop.clone(), pending)
			}
			SourceId::Window(id) => {
				let window = Window::enumerate()
					.map_err(|_| "Windows could not be enumerated")?
					.into_iter()
					.find(|window| window_id(window) == id)
					.ok_or("Selected window is no longer available")?;
				start_item(settings, window, frames, stop.clone(), pending)
			}
			#[allow(unreachable_patterns)] // Portal may be absent from platform-scoped models.
			_ => return Err("The desktop screen picker is available only on Linux"),
		}?;
		if let Some(send) = audio {
			capture.audio = Some(audio::Audio::start(send, stop, ready, audio_epoch)?);
		}
		Ok(capture)
	}

	pub(crate) fn failed(&self) -> bool {
		self.audio.as_ref().is_some_and(audio::Audio::failed)
	}
}

trait NativeSource:
	TryInto<windows_capture::settings::GraphicsCaptureItemType> + Send + 'static
{
	fn dimensions(&self) -> Option<(u32, u32)>;
}

impl NativeSource for Monitor {
	fn dimensions(&self) -> Option<(u32, u32)> {
		self.width().ok().zip(self.height().ok())
	}
}

impl NativeSource for Window {
	fn dimensions(&self) -> Option<(u32, u32)> {
		self.width()
			.ok()
			.and_then(|width| u32::try_from(width).ok())
			.zip(
				self.height()
					.ok()
					.and_then(|height| u32::try_from(height).ok()),
			)
	}
}

fn start_item<T>(
	settings: CaptureSettings,
	item: T,
	frames: SyncSender<RawFrame>,
	stop: Arc<AtomicBool>,
	pending: Arc<AtomicBool>,
) -> Result<Capture, &'static str>
where
	T: NativeSource,
{
	let Some((width, height)) = item.dimensions() else {
		return Err("Selected source dimensions are unavailable");
	};
	if width == 0 || height == 0 || width > MAX_FRAME_WIDTH || height > MAX_FRAME_HEIGHT {
		return Err("Selected source exceeds the 4K capture limit");
	}
	let flags = Flags {
		frames,
		stop,
		pending,
		next_frame: Instant::now(),
		interval: Duration::from_secs_f64(1.0 / f64::from(settings.fps)),
	};
	let native = Settings::new(
		item,
		if settings.cursor {
			CursorCaptureSettings::Default
		} else {
			CursorCaptureSettings::WithoutCursor
		},
		DrawBorderSettings::WithoutBorder,
		SecondaryWindowSettings::Default,
		MinimumUpdateIntervalSettings::Default,
		DirtyRegionSettings::Default,
		ColorFormat::Bgra8,
		flags,
	);
	let control =
		Handler::start_free_threaded(native).map_err(|_| "Screen capture could not be started")?;
	Ok(Capture {
		control: Some(control),
		audio: None,
	})
}

impl Drop for Capture {
	fn drop(&mut self) {
		self.audio.take();
		if let Some(control) = self.control.take() {
			let _ = control.stop();
		}
	}
}
