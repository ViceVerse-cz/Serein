use super::{
	MAX_FRAME_HEIGHT, MAX_FRAME_WIDTH, MAX_SOURCE_BYTES, MAX_SOURCE_HEIGHT, MAX_SOURCE_WIDTH,
	MAX_SOURCES, bounded_name,
};
use crate::screen::convert::Picture;
use crate::screen::{AudioChunk, RawFrame, Settings as CaptureSettings, Source, SourceId};
use std::borrow::Cow;
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU64, Ordering},
	mpsc::SyncSender,
};
use std::time::{Duration, Instant};
use windows_capture::{
	capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
	frame::Frame,
	graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl},
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
			width,
			height,
			refresh_hz: monitor.refresh_rate().unwrap_or(0),
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
			width,
			height,
			refresh_hz: window
				.monitor()
				.and_then(|monitor| monitor.refresh_rate().ok())
				.unwrap_or(0),
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
	/// The selected quality: a source over the sharing budget is fitted into it.
	settings: CaptureSettings,
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
		if width == 0 || height == 0 || width > MAX_SOURCE_WIDTH || height > MAX_SOURCE_HEIGHT {
			self.0.stop.store(true, Ordering::Release);
			control.stop();
			return Err("Captured picture exceeds the capture limit");
		}
		let Some(row_bytes) = (width as usize).checked_mul(4) else {
			self.0.stop.store(true, Ordering::Release);
			control.stop();
			return Err("Captured frame dimensions are invalid");
		};
		let mut buffer = frame.buffer().map_err(|_| {
			self.0.stop.store(true, Ordering::Release);
			"Captured frame could not be read"
		})?;
		let stride = buffer.row_pitch() as usize;
		let Some(source_len) = stride.checked_mul(height as usize) else {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame stride is invalid");
		};
		if stride < row_bytes || source_len > MAX_SOURCE_BYTES {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame stride exceeds the capture limit");
		}
		let source = buffer.as_raw_buffer();
		if source.len() < source_len {
			self.0.stop.store(true, Ordering::Release);
			return Err("Captured frame buffer is incomplete");
		}
		let source = &source[..source_len];
		// A picture over the sharing budget — a display above 4K, a window spanning two
		// displays, or a 4K window whose border and title bar push its rectangle past the
		// budget — is fitted into the selected quality as it is copied out, so the queue, the
		// local preview and the encoder keep that bound instead of the share failing.
		let (picture_width, picture_height) = shared_picture(self.0.settings, width, height);
		let data = if (picture_width, picture_height) == (width, height) {
			let Some(data_len) = row_bytes.checked_mul(height as usize) else {
				self.0.stop.store(true, Ordering::Release);
				return Err("Captured frame dimensions are invalid");
			};
			let mut data = Vec::with_capacity(data_len);
			for row in source.chunks_exact(stride) {
				data.extend_from_slice(&row[..row_bytes]);
			}
			data
		} else {
			let picture = Picture {
				data: source,
				width,
				height,
				stride,
			};
			match picture.scaled(picture_width, picture_height) {
				Ok(data) => data,
				Err(reason) => {
					self.0.stop.store(true, Ordering::Release);
					control.stop();
					return Err(reason);
				}
			}
		};
		if self
			.0
			.frames
			.try_send(RawFrame {
				width: picture_width,
				height: picture_height,
				stride: picture_width as usize * 4,
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
	/// Starts capturing the selected source and, when asked, system audio. A failure carries the
	/// reason shown to the person, including the Windows error code when audio reported one.
	#[allow(clippy::too_many_arguments)] // One capture's output, stop, readiness, audio and consent flags.
	pub(crate) fn start(
		settings: CaptureSettings,
		frames: SyncSender<RawFrame>,
		audio: Option<tokio::sync::mpsc::Sender<AudioChunk>>,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		audio_epoch: Arc<AtomicU64>,
		pending: Arc<AtomicBool>,
		// The worker's cancellation: a stopped share gives up a pending consent wait.
		cancel: Arc<AtomicBool>,
	) -> Result<Self, Cow<'static, str>> {
		if settings.width == 0
			|| settings.height == 0
			|| settings.width > MAX_FRAME_WIDTH
			|| settings.height > MAX_FRAME_HEIGHT
			|| settings.fps == 0
		{
			return Err("Invalid screen capture settings".into());
		}
		// The shared window's process, so a window share captures only that application's audio.
		let mut window_process = None;
		let mut capture = match settings.source {
			SourceId::Display(id) => {
				let monitor = Monitor::enumerate()
					.map_err(|_| "Displays could not be enumerated")?
					.into_iter()
					.find(|monitor| monitor_id(monitor) == id)
					.ok_or("Selected display is no longer available")?;
				start_item(settings, monitor, frames, stop.clone(), pending, cancel)
			}
			SourceId::Window(id) => {
				let window = Window::enumerate()
					.map_err(|_| "Windows could not be enumerated")?
					.into_iter()
					.find(|window| window_id(window) == id)
					.ok_or("Selected window is no longer available")?;
				window_process = window.process_id().ok();
				start_item(settings, window, frames, stop.clone(), pending, cancel)
			}
			#[allow(unreachable_patterns)] // Portal may be absent from platform-scoped models.
			_ => return Err("The desktop screen picker is available only on Linux".into()),
		}?;
		if let Some(send) = audio {
			capture.audio = Some(audio::Audio::start(
				send,
				stop,
				ready,
				audio_epoch,
				window_process,
			)?);
		}
		Ok(capture)
	}

	/// Whether system audio capture has failed or its worker has ended.
	pub(crate) fn failed(&self) -> bool {
		self.audio.as_ref().is_some_and(audio::Audio::failed)
	}

	/// The status text for a stopped audio capture, with the Windows error code when known.
	pub(crate) fn failure(&self) -> Cow<'static, str> {
		self.audio.as_ref().map_or(
			Cow::Borrowed(
				"System audio capture stopped; check your output device or share without audio",
			),
			audio::Audio::failure,
		)
	}
}

trait NativeSource:
	TryInto<windows_capture::settings::GraphicsCaptureItemType> + Send + Copy + 'static
{
	fn dimensions(&self) -> Option<(u32, u32)>;
	/// Converts the source once, so Windows can answer with its permission decision.
	fn probe(&self) -> Result<(), &'static str>;
}

impl NativeSource for Monitor {
	fn dimensions(&self) -> Option<(u32, u32)> {
		self.width().ok().zip(self.height().ok())
	}
	fn probe(&self) -> Result<(), &'static str> {
		<Monitor as TryInto<windows_capture::settings::GraphicsCaptureItemType>>::try_into(*self)
			.map(|_| ())
			.map_err(|error| capture_refusal(error.code().0 as u32))
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
	fn probe(&self) -> Result<(), &'static str> {
		<Window as TryInto<windows_capture::settings::GraphicsCaptureItemType>>::try_into(*self)
			.map(|_| ())
			.map_err(|error| capture_refusal(error.code().0 as u32))
	}
}

/// The picture handed downstream: the captured size, or the selected quality that an
/// over-budget source is fitted into with its aspect ratio kept.
fn shared_picture(settings: CaptureSettings, width: u32, height: u32) -> (u32, u32) {
	if width <= MAX_FRAME_WIDTH && height <= MAX_FRAME_HEIGHT {
		return (width, height);
	}
	let fitted = settings.fitted_to(width, height);
	(fitted.width, fitted.height)
}

#[allow(clippy::too_many_arguments)] // One capture's stop, readiness, audio and cancellation flags.
fn start_item<T>(
	settings: CaptureSettings,
	item: T,
	frames: SyncSender<RawFrame>,
	stop: Arc<AtomicBool>,
	pending: Arc<AtomicBool>,
	cancel: Arc<AtomicBool>,
) -> Result<Capture, &'static str>
where
	T: NativeSource,
{
	let Some((width, height)) = item.dimensions() else {
		return Err("Selected source dimensions are unavailable");
	};
	// A window rectangle includes its border and title bar, so a 4K window measures more than
	// 4K. Only the source bound applies here; the captured picture is fitted into the selected
	// quality when it arrives.
	if width == 0 || height == 0 || width > MAX_SOURCE_WIDTH || height > MAX_SOURCE_HEIGHT {
		return Err("Selected screen or window is too large to share");
	}
	// Windows 11 keeps a per-application consent; an application that never asks is denied and
	// the Settings page lists no request. Asking here shows the system prompt once.
	request_access(&cancel)?;
	// The item conversion can still refuse the source (a protected window, for example).
	item.probe()?;
	let flags = Flags {
		frames,
		stop,
		pending,
		next_frame: Instant::now(),
		interval: Duration::from_secs_f64(1.0 / f64::from(settings.fps)),
		settings,
	};
	let cursor =
		if settings.cursor && GraphicsCaptureApi::is_cursor_settings_supported().unwrap_or(false) {
			CursorCaptureSettings::WithCursor
		} else if settings.cursor {
			CursorCaptureSettings::Default
		} else {
			CursorCaptureSettings::WithoutCursor
		};
	// An unset interval lets Windows batch pointer-only updates away from the frame rate.
	let minimum_update =
		if GraphicsCaptureApi::is_minimum_update_interval_supported().unwrap_or(false) {
			MinimumUpdateIntervalSettings::Custom(Duration::from_millis(1))
		} else {
			MinimumUpdateIntervalSettings::Default
		};
	let native = Settings::new(
		item,
		cursor,
		// Hide the border where supported; older Windows must keep the system default.
		if GraphicsCaptureApi::is_border_settings_supported().unwrap_or(false) {
			DrawBorderSettings::WithoutBorder
		} else {
			DrawBorderSettings::Default
		},
		SecondaryWindowSettings::Default,
		minimum_update,
		DirtyRegionSettings::Default,
		ColorFormat::Bgra8,
		flags,
	);
	let control = Handler::start_free_threaded(native).map_err(|error| start_failure(&error))?;
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

/// `E_ACCESSDENIED`; Windows 11's "Screenshots and screen recording" privacy setting returns it.
const ACCESS_DENIED: u32 = 0x8007_0005;
/// Windows 11 blocks programmatic capture for applications the owner has not allowed.
const DENIED: &str = "Windows blocked screen capture for this app; allow it in Settings, Privacy & security, Screenshots and screen recording";
/// How often a pending consent wait checks whether the share was cancelled.
const CONSENT_POLL: Duration = Duration::from_millis(50);

/// Asks Windows for the programmatic capture consent this application needs.
///
/// The consent is per application: an application that never asks is denied outright and the
/// Settings page lists no request. The call shows the system prompt once; the wait stays
/// cancellable so a share stopped while the prompt is open releases the worker at once.
fn request_access(stop: &AtomicBool) -> Result<(), &'static str> {
	use windows::Graphics::Capture::{GraphicsCaptureAccess, GraphicsCaptureAccessKind};
	use windows::Security::Authorization::AppCapabilityAccess::AppCapabilityAccessStatus;
	use windows_future::AsyncStatus;
	// SAFETY: WinRT activation needs an initialized apartment on this worker thread. Only the
	// consent call below decides the outcome, so an already-initialized mode is harmless.
	#[allow(unsafe_code)]
	unsafe {
		let _ = windows::Win32::System::WinRT::RoInitialize(
			windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
		);
	}
	let Ok(operation) =
		GraphicsCaptureAccess::RequestAccessAsync(GraphicsCaptureAccessKind::Programmatic)
	else {
		// Builds without the consent API report their decision through the capture item instead.
		return Ok(());
	};
	while operation.Status() == Ok(AsyncStatus::Started) {
		if stop.load(Ordering::Acquire) {
			return Err("Screen sharing was cancelled");
		}
		std::thread::sleep(CONSENT_POLL);
	}
	match operation.GetResults() {
		Ok(status) if status == AppCapabilityAccessStatus::Allowed => Ok(()),
		// A decision the owner made is the only answer this call can be trusted with: an
		// unpackaged executable reports NotDeclaredByApp even where capture is already allowed,
		// and that decision still belongs to the capture item.
		Ok(status)
			if status == AppCapabilityAccessStatus::DeniedByUser
				|| status == AppCapabilityAccessStatus::DeniedBySystem =>
		{
			Err(DENIED)
		}
		_ => Ok(()),
	}
}

/// The item conversion is the first place Windows reports the privacy decision.
fn capture_refusal(code: u32) -> &'static str {
	if code == ACCESS_DENIED {
		DENIED
	} else {
		"Screen capture could not be started"
	}
}

fn start_failure(
	error: &windows_capture::capture::GraphicsCaptureApiError<&'static str>,
) -> &'static str {
	use windows_capture::{capture::GraphicsCaptureApiError as Error, graphics_capture_api};
	match error {
		Error::GraphicsCaptureApiError(graphics_capture_api::Error::WindowsError(windows))
			if windows.code().0 as u32 == ACCESS_DENIED =>
		{
			DENIED
		}
		Error::DirectXError(_) => "The graphics driver could not start screen capture",
		Error::FailedToInitWinRT
		| Error::FailedToCreateDispatcherQueueController
		| Error::FailedToShutdownDispatcherQueue
		| Error::FailedToSetDispatcherQueueCompletedHandler
		| Error::FailedToRunMessageLoop
		| Error::FailedToStartCaptureThread
		| Error::FailedToJoinThread => "Windows could not start its screen-capture service",
		Error::ItemConvertFailed => "The selected screen or window cannot be captured",
		Error::GraphicsCaptureApiError(graphics_capture_api::Error::Unsupported) => {
			"Windows Graphics Capture is unavailable on this Windows build"
		}
		Error::GraphicsCaptureApiError(_) => "Windows refused to start screen capture",
		Error::NewHandlerError(_) | Error::FrameHandlerError(_) => {
			"Screen capture could not be started"
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quality(width: u32, height: u32) -> CaptureSettings {
		CaptureSettings {
			source: SourceId::Window(1),
			width,
			height,
			fps: 30,
			cursor: true,
			audio: false,
		}
	}

	#[test]
	fn a_source_over_the_budget_shares_at_the_selected_quality() {
		let settings = quality(1280, 720);
		// A maximized window on a 4K display also measures its border and title bar, so its
		// rectangle is over the budget while its picture is not: it is fitted, not refused.
		assert_eq!(shared_picture(settings, 3856, 2176), (1276, 720));
		// A 4K window plus its title bar keeps its full width and loses the chrome.
		assert_eq!(
			shared_picture(quality(3840, 2160), 3840, 2191),
			(3786, 2160)
		);
		// A larger display scales into the same bound.
		assert_eq!(shared_picture(settings, 5120, 2880), (1280, 720));
		// A picture inside the budget keeps its captured size, chrome or not.
		assert_eq!(shared_picture(settings, 1920, 1080), (1920, 1080));
		assert_eq!(shared_picture(settings, 3840, 2160), (3840, 2160));
	}
}
