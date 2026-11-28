use super::{
	MAX_FRAME_HEIGHT, MAX_FRAME_WIDTH, MAX_RAW_BYTES, MAX_SOURCE_HEIGHT, MAX_SOURCE_WIDTH,
	MAX_SOURCES, bounded_name,
};
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
	dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat},
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
	duplication: Option<Duplication>,
	audio: Option<audio::Audio>,
}

/// One desktop-duplication capture: the thread that reads frames and the flag that ends it.
struct Duplication {
	quit: Arc<AtomicBool>,
	thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Duplication {
	fn drop(&mut self) {
		self.quit.store(true, Ordering::Release);
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
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
				match start_item(
					settings,
					monitor,
					frames.clone(),
					stop.clone(),
					pending.clone(),
					cancel,
				) {
					Ok(capture) => Ok(capture),
					// Windows 11 26H2 refuses Windows Graphics Capture to an application the
					// privacy page has not listed, while the programmatic path it does allow is
					// desktop duplication. Read a display that way instead of failing the share.
					Err(reason) if reason == DENIED => {
						start_duplication(settings, monitor, frames, stop.clone(), pending)
					}
					Err(reason) => Err(reason),
				}
			}
			SourceId::Window(id) => {
				let window = Window::enumerate()
					.map_err(|_| "Windows could not be enumerated")?
					.into_iter()
					.find(|window| window_id(window) == id)
					.ok_or("Selected window is no longer available")?;
				window_process = window.process_id().ok();
				start_item(settings, window, frames, stop.clone(), pending, cancel).map_err(
					|reason| {
						if reason == DENIED {
							DENIED_WINDOW
						} else {
							reason
						}
					},
				)
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
	if width == 0 || height == 0 || width > MAX_FRAME_WIDTH || height > MAX_FRAME_HEIGHT {
		return Err("Selected source exceeds the 4K capture limit");
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
		duplication: None,
		audio: None,
	})
}

/// Starts desktop duplication for a display Windows Graphics Capture refused.
///
/// Duplication is the "programmatic" capture path, which the same privacy page keeps available,
/// and it is the only one Windows 11 26H2 offers an application that is not in its list. It draws
/// no cursor and copies each frame through the CPU, so it stays the fallback rather than the
/// default.
fn start_duplication(
	settings: CaptureSettings,
	monitor: Monitor,
	frames: SyncSender<RawFrame>,
	stop: Arc<AtomicBool>,
	pending: Arc<AtomicBool>,
) -> Result<Capture, &'static str> {
	let quit = Arc::new(AtomicBool::new(false));
	let thread = {
		let quit = Arc::clone(&quit);
		std::thread::Builder::new()
			.name("serein-screen-duplication".into())
			.spawn(move || duplication_loop(settings, monitor, &frames, &stop, &pending, &quit))
			.map_err(|_| "Screen capture could not be started")?
	};
	Ok(Capture {
		control: None,
		duplication: Some(Duplication {
			quit,
			thread: Some(thread),
		}),
		audio: None,
	})
}

/// Reads display frames until the share stops, paced like the Windows Graphics Capture handler and
/// dropping a frame while the encoder still holds the previous one.
fn duplication_loop(
	settings: CaptureSettings,
	monitor: Monitor,
	frames: &SyncSender<RawFrame>,
	stop: &AtomicBool,
	pending: &AtomicBool,
	quit: &AtomicBool,
) {
	let Ok(mut api) = DxgiDuplicationApi::new_options(monitor, &[DxgiDuplicationFormat::Bgra8])
	else {
		stop.store(true, Ordering::Release);
		return;
	};
	let interval = Duration::from_secs_f64(1.0 / f64::from(settings.fps));
	let mut next_frame = Instant::now();
	let mut packed = Vec::new();
	while !quit.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
		let mut frame = match api.acquire_next_frame(100) {
			Ok(frame) => frame,
			// A static display is the normal case between updates, not an error.
			Err(windows_capture::dxgi_duplication_api::Error::Timeout) => continue,
			Err(_) => match api.recreate() {
				Ok(recreated) => {
					api = recreated;
					continue;
				}
				Err(_) => break,
			},
		};
		let (width, height) = (frame.width(), frame.height());
		let Some(stride) = usize::try_from(width).ok().and_then(|w| w.checked_mul(4)) else {
			stop.store(true, Ordering::Release);
			break;
		};
		let Some(length) = stride.checked_mul(height as usize) else {
			stop.store(true, Ordering::Release);
			break;
		};
		if width == 0
			|| height == 0
			|| width > MAX_FRAME_WIDTH
			|| height > MAX_FRAME_HEIGHT
			|| length > MAX_RAW_BYTES
		{
			stop.store(true, Ordering::Release);
			break;
		}
		let Ok(buffer) = frame.buffer() else {
			continue;
		};
		let format = buffer.format();
		let source = buffer.as_nopadding_buffer(&mut packed);
		let Some(data) = packed_bgra(format, source, width, height) else {
			continue;
		};
		let now = Instant::now();
		if now < next_frame {
			std::thread::sleep(next_frame - now);
		}
		if stop.load(Ordering::Acquire) || quit.load(Ordering::Acquire) {
			break;
		}
		// The worker clears `pending` once it takes a frame, exactly as it does for the handler.
		if pending.swap(true, Ordering::AcqRel) {
			continue;
		}
		next_frame = (next_frame + interval).max(Instant::now());
		let frame = RawFrame {
			width,
			height,
			stride,
			data,
		};
		if frames.try_send(frame).is_err() {
			pending.store(false, Ordering::Release);
		}
	}
}

/// Converts one duplicated frame into the packed BGRA rows the encoder expects.
///
/// Duplication hands over the display's own format, and only sometimes converts: an HDR display
/// produces scRGB half floats, and a colour-space change switches between the RGBA and BGRA byte
/// orders. Reading those bytes as BGRA is what made a share flash the wrong colours, so the format
/// is honoured here instead.
fn packed_bgra(
	format: DxgiDuplicationFormat,
	source: &[u8],
	width: u32,
	height: u32,
) -> Option<Vec<u8>> {
	let pixels = usize::try_from(width)
		.ok()?
		.checked_mul(usize::try_from(height).ok()?)?;
	let length = pixels.checked_mul(4)?;
	match format {
		DxgiDuplicationFormat::Bgra8 => source.get(..length).map(<[u8]>::to_vec),
		DxgiDuplicationFormat::Rgba8 => {
			let source = source.get(..length)?;
			let mut data = Vec::with_capacity(length);
			for pixel in source.chunks_exact(4) {
				data.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
			}
			Some(data)
		}
		DxgiDuplicationFormat::Rgba16F => {
			let source = source.get(..pixels.checked_mul(8)?)?;
			let table = sc_rgb_half_table();
			let mut data = Vec::with_capacity(length);
			for pixel in source.chunks_exact(8) {
				let red = table[usize::from(u16::from_le_bytes([pixel[0], pixel[1]]))];
				let green = table[usize::from(u16::from_le_bytes([pixel[2], pixel[3]]))];
				let blue = table[usize::from(u16::from_le_bytes([pixel[4], pixel[5]]))];
				data.extend_from_slice(&[blue, green, red, 255]);
			}
			Some(data)
		}
	}
}

/// Half float scRGB to 8-bit sRGB, tabulated once: an HDR display's 1.0 is SDR white, and the
/// stream is 8-bit sRGB, so the table folds the clamp and the transfer function together.
fn sc_rgb_half_table() -> &'static [u8; 65536] {
	static TABLE: std::sync::OnceLock<[u8; 65536]> = std::sync::OnceLock::new();
	TABLE.get_or_init(|| {
		let mut table = [0u8; 65536];
		for (bits, entry) in table.iter_mut().enumerate() {
			let value = half_to_f32(bits as u16);
			let linear = if value.is_finite() {
				value.clamp(0.0, 1.0)
			} else {
				0.0
			};
			let encoded = if linear <= 0.003_130_8 {
				linear * 12.92
			} else {
				1.055 * linear.powf(1.0 / 2.4) - 0.055
			};
			*entry = (encoded * 255.0 + 0.5) as u8;
		}
		table
	})
}

/// One IEEE half decoded without a dependency on an `f16` type.
fn half_to_f32(bits: u16) -> f32 {
	let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
	let exponent = (bits >> 10) & 0x1f;
	let mantissa = f32::from(bits & 0x03ff);
	match exponent {
		0 => sign * mantissa * 2f32.powi(-24),
		31 if mantissa == 0.0 => sign * f32::INFINITY,
		31 => f32::NAN,
		_ => sign * (1.0 + mantissa / 1024.0) * 2f32.powi(i32::from(exponent) - 15),
	}
}

impl Drop for Capture {
	fn drop(&mut self) {
		self.audio.take();
		self.duplication.take();
		if let Some(control) = self.control.take() {
			let _ = control.stop();
		}
	}
}

/// `E_ACCESSDENIED`; Windows 11's "Screenshots and screen recording" privacy setting returns it.
const ACCESS_DENIED: u32 = 0x8007_0005;
/// Windows 11 blocks programmatic capture for applications the owner has not allowed.
const DENIED: &str = "Windows blocked screen capture for this app; allow it in Settings, Privacy & security, Screenshots and screen recording";
/// A window share has no fallback: desktop duplication captures displays only.
const DENIED_WINDOW: &str = "Windows blocked screen capture for this app; allow it in Settings, Privacy & security, Screenshots and screen recording, or share a whole display instead";
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

	/// A display share has to reach the encoder on a Windows build that refuses Windows Graphics
	/// Capture to this application, which is what the duplication fallback is for. Where capture is
	/// allowed the same call exercises the primary path, and a headless runner produces no frame at
	/// all, which this test then does not judge.
	#[test]
	fn a_display_share_produces_frames_through_whichever_path_windows_allows() {
		let Some(monitor) = Monitor::enumerate()
			.ok()
			.and_then(|mut all| all.drain(..).next())
		else {
			return;
		};
		let (Ok(width), Ok(height)) = (monitor.width(), monitor.height()) else {
			return;
		};
		if width < 2 || height < 2 || width > MAX_FRAME_WIDTH || height > MAX_FRAME_HEIGHT {
			return;
		}
		let settings = CaptureSettings {
			source: SourceId::Display(monitor_id(&monitor)),
			width,
			height,
			fps: 30,
			cursor: true,
			audio: false,
		};
		let (send, receive) = std::sync::mpsc::sync_channel(1);
		let Ok(capture) = Capture::start(
			settings,
			send,
			None,
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicU64::new(0)),
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicBool::new(false)),
		) else {
			return;
		};
		let frame = receive.recv_timeout(Duration::from_secs(5)).ok();
		drop(capture);
		let Some(frame) = frame else {
			return;
		};
		assert_eq!((frame.width, frame.height), (width, height));
		assert_eq!(frame.stride, width as usize * 4, "packed BGRA rows");
		assert_eq!(frame.data.len(), frame.stride * height as usize);
	}

	/// Duplication does not promise one byte order: an HDR display hands over scRGB half floats and
	/// a colour-space change switches between RGBA and BGRA. Treating any of them as BGRA is what
	/// made a share flash wrong colours.
	#[test]
	fn duplicated_frames_arrive_as_packed_bgra_whatever_the_display_hands_over() {
		let rgba = [10, 20, 30, 255, 40, 50, 60, 255];
		assert_eq!(
			packed_bgra(DxgiDuplicationFormat::Rgba8, &rgba, 2, 1),
			Some(vec![30, 20, 10, 255, 60, 50, 40, 255])
		);
		assert_eq!(
			packed_bgra(DxgiDuplicationFormat::Bgra8, &rgba, 2, 1),
			Some(rgba.to_vec())
		);
		// scRGB 1.0 in every channel is SDR white.
		let mut white = Vec::new();
		for _ in 0..3 {
			white.extend_from_slice(&0x3c00u16.to_le_bytes());
		}
		white.extend_from_slice(&0u16.to_le_bytes());
		assert_eq!(
			packed_bgra(DxgiDuplicationFormat::Rgba16F, &white, 1, 1),
			Some(vec![255, 255, 255, 255])
		);
		// A buffer too short for the picture is refused rather than read as pixels.
		assert_eq!(
			packed_bgra(DxgiDuplicationFormat::Bgra8, &[0; 3], 1, 1),
			None
		);
	}
}
