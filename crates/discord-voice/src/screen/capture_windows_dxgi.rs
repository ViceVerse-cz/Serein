//! Borderless Windows 10 display capture via DXGI Desktop Duplication (`IDXGIOutputDuplication`).
//!
//! Windows Graphics Capture only allows hiding the yellow capture border when
//! `GraphicsCaptureSession.IsBorderRequired` is present (Windows 11 / build 20348+). On older
//! Windows 10 builds, display shares use DXGI Desktop Duplication to capture the selected monitor
//! without that border while honoring the cursor toggle, falling back to Windows Graphics Capture
//! if DXGI duplication is unavailable for that display.

use crate::screen::MAX_RAW_BYTES;
use std::time::{Duration, Instant};

const MAX_FRAME_WIDTH: u32 = 3840;
const MAX_FRAME_HEIGHT: u32 = 2160;
/// Bounded cap on a single cursor shape buffer returned by DXGI (4 MiB covers up to 1024x1024 BGRA).
const MAX_POINTER_SHAPE_BYTES: usize = 4 * 1024 * 1024;
const RECOVERY_LIMIT: Duration = Duration::from_secs(3);

const _: () = assert!(MAX_FRAME_WIDTH as usize * MAX_FRAME_HEIGHT as usize * 4 == MAX_RAW_BYTES);

/// Prefer DXGI Desktop Duplication for display capture only when Windows explicitly reports that
/// WGC border removal (`IsBorderRequired`) is unsupported. Capability probe failures (`None`)
/// keep the Windows Graphics Capture path instead of assuming Windows 10.
pub(super) const fn should_use_dxgi_for_display(border_settings_supported: Option<bool>) -> bool {
	matches!(border_settings_supported, Some(false))
}

/// Output rotation reported by `DXGI_OUTDUPL_DESC.Rotation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rotation {
	Identity,
	Rotate90,
	Rotate180,
	Rotate270,
}

impl Rotation {
	/// Output dimensions after transforming an acquired surface into upright desktop orientation.
	const fn output_dimensions(self, width: u32, height: u32) -> (u32, u32) {
		match self {
			Self::Identity | Self::Rotate180 => (width, height),
			Self::Rotate90 | Self::Rotate270 => (height, width),
		}
	}

	/// Avoids double-rotating if a display driver already exposes the surface in logical desktop
	/// dimensions instead of the raw unrotated scanout dimensions.
	const fn effective(self, texture: (u32, u32), mode: (u32, u32)) -> Self {
		match self {
			Self::Rotate90 | Self::Rotate270
				if mode.0 != mode.1 && texture.0 == mode.0 && texture.1 == mode.1 =>
			{
				Self::Identity
			}
			other => other,
		}
	}
}

/// Copies a mapped BGRA8 desktop surface into `dst`, un-rotating it into upright desktop
/// orientation when the monitor is rotated.
fn copy_bgra_rotated(
	src: &[u8],
	src_width: u32,
	src_height: u32,
	src_stride: usize,
	rotation: Rotation,
	dst: &mut Vec<u8>,
) -> Result<(u32, u32), &'static str> {
	let src_w = src_width as usize;
	let src_h = src_height as usize;
	let src_row_bytes = src_w
		.checked_mul(4)
		.ok_or("Captured frame dimensions are invalid")?;
	let src_required = src_stride
		.checked_mul(src_h)
		.ok_or("Captured frame stride is invalid")?;
	if src_width == 0 || src_height == 0 || src_stride < src_row_bytes || src.len() < src_required {
		return Err("Captured frame buffer is incomplete");
	}
	let (out_width, out_height) = rotation.output_dimensions(src_width, src_height);
	let dst_row_bytes = (out_width as usize)
		.checked_mul(4)
		.ok_or("Captured frame dimensions are invalid")?;
	let dst_len = dst_row_bytes
		.checked_mul(out_height as usize)
		.ok_or("Captured frame dimensions are invalid")?;
	if out_width == 0
		|| out_height == 0
		|| out_width > MAX_FRAME_WIDTH
		|| out_height > MAX_FRAME_HEIGHT
		|| dst_len > MAX_RAW_BYTES
	{
		return Err("Captured frame exceeds the 4K limit");
	}

	dst.resize(dst_len, 0);
	match rotation {
		Rotation::Identity => {
			for (dst_row, src_row) in dst
				.chunks_exact_mut(dst_row_bytes)
				.zip(src[..src_required].chunks_exact(src_stride))
			{
				dst_row.copy_from_slice(&src_row[..src_row_bytes]);
			}
		}
		Rotation::Rotate90 => {
			// Matches Microsoft's DisplayManager.cpp SetDirtyVert: DXGI_MODE_ROTATION_ROTATE90
			// maps the raw unrotated surface 90 degrees clockwise into upright desktop pixels.
			for y in 0..src_h {
				let src_row = &src[y * src_stride..y * src_stride + src_row_bytes];
				let dst_x = src_h - 1 - y;
				for x in 0..src_w {
					let dst_y = x;
					let dst_off = dst_y * dst_row_bytes + dst_x * 4;
					let src_off = x * 4;
					dst[dst_off..dst_off + 4].copy_from_slice(&src_row[src_off..src_off + 4]);
				}
			}
		}
		Rotation::Rotate180 => {
			for y in 0..src_h {
				let src_row = &src[y * src_stride..y * src_stride + src_row_bytes];
				let dst_y = src_h - 1 - y;
				let dst_row = &mut dst[dst_y * dst_row_bytes..(dst_y + 1) * dst_row_bytes];
				for x in 0..src_w {
					let dst_off = (src_w - 1 - x) * 4;
					let src_off = x * 4;
					dst_row[dst_off..dst_off + 4].copy_from_slice(&src_row[src_off..src_off + 4]);
				}
			}
		}
		Rotation::Rotate270 => {
			// Matches Microsoft's DisplayManager.cpp SetDirtyVert: DXGI_MODE_ROTATION_ROTATE270
			// maps the raw unrotated surface 90 degrees counter-clockwise into upright desktop pixels.
			for y in 0..src_h {
				let src_row = &src[y * src_stride..y * src_stride + src_row_bytes];
				for x in 0..src_w {
					let dst_x = y;
					let dst_y = src_w - 1 - x;
					let dst_off = dst_y * dst_row_bytes + dst_x * 4;
					let src_off = x * 4;
					dst[dst_off..dst_off + 4].copy_from_slice(&src_row[src_off..src_off + 4]);
				}
			}
		}
	}
	Ok((out_width, out_height))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerShapeKind {
	Monochrome,
	Color,
	MaskedColor,
}

#[derive(Clone, Debug)]
struct PointerShape {
	kind: PointerShapeKind,
	width: u32,
	/// Visible cursor height in pixels (for `Monochrome`, half of DXGI's combined AND+XOR height).
	height: u32,
	pitch: usize,
	data: Vec<u8>,
}

impl PointerShape {
	/// Validates cursor metadata and byte length returned by `GetFramePointerShape`.
	fn new(
		kind: PointerShapeKind,
		width: u32,
		raw_height: u32,
		pitch: u32,
		data: Vec<u8>,
	) -> Option<Self> {
		if width == 0 || raw_height == 0 || data.len() > MAX_POINTER_SHAPE_BYTES {
			return None;
		}
		let pitch = usize::try_from(pitch).ok()?;
		let (height, min_pitch) = match kind {
			PointerShapeKind::Monochrome => {
				if raw_height < 2 || !raw_height.is_multiple_of(2) {
					return None;
				}
				(raw_height / 2, (width as usize).div_ceil(8))
			}
			PointerShapeKind::Color | PointerShapeKind::MaskedColor => {
				(raw_height, (width as usize).checked_mul(4)?)
			}
		};
		if pitch < min_pitch {
			return None;
		}
		let required = pitch.checked_mul(raw_height as usize)?;
		if data.len() < required {
			return None;
		}
		Some(Self {
			kind,
			width,
			height,
			pitch,
			data,
		})
	}
}

/// Composites a DXGI hardware cursor bitmap onto a packed BGRA8 desktop frame.
fn composite_pointer(
	frame: &mut [u8],
	frame_width: u32,
	frame_height: u32,
	pos_x: i32,
	pos_y: i32,
	shape: &PointerShape,
) {
	let Some(frame_stride) = (frame_width as usize).checked_mul(4) else {
		return;
	};
	let Some(frame_bytes) = frame_stride.checked_mul(frame_height as usize) else {
		return;
	};
	if frame.len() < frame_bytes || shape.width == 0 || shape.height == 0 {
		return;
	}
	let src_x0 = if pos_x < 0 { pos_x.unsigned_abs() } else { 0 };
	let src_y0 = if pos_y < 0 { pos_y.unsigned_abs() } else { 0 };
	let dst_x0 = pos_x.max(0) as u32;
	let dst_y0 = pos_y.max(0) as u32;
	if dst_x0 >= frame_width
		|| dst_y0 >= frame_height
		|| src_x0 >= shape.width
		|| src_y0 >= shape.height
	{
		return;
	}
	let copy_w = (shape.width - src_x0).min(frame_width - dst_x0) as usize;
	let copy_h = (shape.height - src_y0).min(frame_height - dst_y0) as usize;
	let src_x0 = src_x0 as usize;
	let src_y0 = src_y0 as usize;
	let dst_x0 = dst_x0 as usize;
	let dst_y0 = dst_y0 as usize;

	match shape.kind {
		PointerShapeKind::Monochrome => {
			let xor_base = shape.height as usize * shape.pitch;
			for row in 0..copy_h {
				let sy = src_y0 + row;
				let dy = dst_y0 + row;
				let and_row = sy * shape.pitch;
				let xor_row = xor_base + and_row;
				let dst_row = dy * frame_stride;
				for col in 0..copy_w {
					let sx = src_x0 + col;
					let mask = 0x80u8 >> (sx & 7);
					let byte_idx = sx >> 3;
					let and_set = (shape.data[and_row + byte_idx] & mask) != 0;
					let xor_set = (shape.data[xor_row + byte_idx] & mask) != 0;
					let dst_off = dst_row + (dst_x0 + col) * 4;
					match (and_set, xor_set) {
						(false, false) => {
							frame[dst_off..dst_off + 4].copy_from_slice(&[0, 0, 0, 255])
						}
						(false, true) => {
							frame[dst_off..dst_off + 4].copy_from_slice(&[255, 255, 255, 255]);
						}
						(true, false) => {}
						(true, true) => {
							frame[dst_off] ^= 0xff;
							frame[dst_off + 1] ^= 0xff;
							frame[dst_off + 2] ^= 0xff;
							frame[dst_off + 3] = 255;
						}
					}
				}
			}
		}
		PointerShapeKind::Color => {
			for row in 0..copy_h {
				let src_row = (src_y0 + row) * shape.pitch;
				let dst_row = (dst_y0 + row) * frame_stride;
				for col in 0..copy_w {
					let src_off = src_row + (src_x0 + col) * 4;
					let alpha = u32::from(shape.data[src_off + 3]);
					if alpha == 0 {
						continue;
					}
					let dst_off = dst_row + (dst_x0 + col) * 4;
					if alpha == 255 {
						frame[dst_off] = shape.data[src_off];
						frame[dst_off + 1] = shape.data[src_off + 1];
						frame[dst_off + 2] = shape.data[src_off + 2];
						frame[dst_off + 3] = 255;
					} else {
						// Standard straight-alpha source-over matching Microsoft's DXGI
						// Desktop Duplication OutputManager blend state.
						let inv = 255 - alpha;
						for channel in 0..3 {
							let s = u32::from(shape.data[src_off + channel]);
							let d = u32::from(frame[dst_off + channel]);
							frame[dst_off + channel] = ((s * alpha + d * inv + 127) / 255) as u8;
						}
						frame[dst_off + 3] = 255;
					}
				}
			}
		}
		PointerShapeKind::MaskedColor => {
			for row in 0..copy_h {
				let src_row = (src_y0 + row) * shape.pitch;
				let dst_row = (dst_y0 + row) * frame_stride;
				for col in 0..copy_w {
					let src_off = src_row + (src_x0 + col) * 4;
					let mask = shape.data[src_off + 3];
					let dst_off = dst_row + (dst_x0 + col) * 4;
					if mask == 0 {
						frame[dst_off] = shape.data[src_off];
						frame[dst_off + 1] = shape.data[src_off + 1];
						frame[dst_off + 2] = shape.data[src_off + 2];
						frame[dst_off + 3] = 255;
					} else {
						frame[dst_off] ^= shape.data[src_off];
						frame[dst_off + 1] ^= shape.data[src_off + 1];
						frame[dst_off + 2] ^= shape.data[src_off + 2];
						frame[dst_off + 3] = 255;
					}
				}
			}
		}
	}
}

/// Tracks whether a pointer update visibly changes the captured monitor so off-monitor moves do
/// not emit redundant frames, while leaving the monitor still clears the previous cursor image.
#[derive(Default)]
struct CursorTracker {
	visible: bool,
	x: i32,
	y: i32,
	shape: Option<PointerShape>,
	last_drawn_visible: bool,
}

impl CursorTracker {
	fn update_position(&mut self, visible: bool, x: i32, y: i32, shape_updated: bool) -> bool {
		let moved = self.x != x || self.y != y;
		let visibility_changed = self.visible != visible;
		self.visible = visible;
		self.x = x;
		self.y = y;
		if visible {
			self.shape.is_some()
				&& (visibility_changed || moved || shape_updated || !self.last_drawn_visible)
		} else {
			self.last_drawn_visible
		}
	}

	fn set_shape(&mut self, shape: PointerShape) {
		self.shape = Some(shape);
	}

	fn clear_shape(&mut self) {
		self.shape = None;
		self.visible = false;
		self.last_drawn_visible = false;
	}

	fn composite_onto(&mut self, frame: &mut [u8], width: u32, height: u32, enabled: bool) {
		if enabled
			&& self.visible
			&& let Some(shape) = &self.shape
		{
			composite_pointer(frame, width, height, self.x, self.y, shape);
			self.last_drawn_visible = true;
		} else {
			self.last_drawn_visible = false;
		}
	}
}

/// Whether a mid-stream DXGI recovery failure should wait indefinitely (for example while the user
/// is on a UAC prompt or lock screen) or stop the share after a bounded grace window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecoveryDisposition {
	KeepWaiting,
	RetryBounded,
}

const E_ACCESSDENIED_CODE: u32 = 0x8007_0005;
const DXGI_ERROR_SESSION_DISCONNECTED_CODE: u32 = 0x887A_0028;

const fn classify_recovery_hresult(code: u32) -> RecoveryDisposition {
	match code {
		E_ACCESSDENIED_CODE | DXGI_ERROR_SESSION_DISCONNECTED_CODE => {
			RecoveryDisposition::KeepWaiting
		}
		_ => RecoveryDisposition::RetryBounded,
	}
}

/// Tracks one recovery episode across both failed session opens and reopened sessions that fail
/// before delivering a copied frame, clearing only after a frame copy succeeds.
#[derive(Default)]
struct RecoveryBudget {
	started: Option<Instant>,
}

impl RecoveryBudget {
	/// Records a recovery failure at `now`. Returns `true` while recovery may continue and `false`
	/// once bounded failures have exceeded `RECOVERY_LIMIT`.
	fn note_failure(&mut self, now: Instant, disposition: RecoveryDisposition) -> bool {
		match disposition {
			RecoveryDisposition::KeepWaiting => {
				self.started = None;
				true
			}
			RecoveryDisposition::RetryBounded => {
				let started = *self.started.get_or_insert(now);
				now.saturating_duration_since(started) < RECOVERY_LIMIT
			}
		}
	}

	fn clear(&mut self) {
		self.started = None;
	}
}

/// True when any pixel has a non-zero RGB channel, distinguishing a real static desktop surface
/// from an uninitialized all-black initial duplication frame.
fn has_visible_pixels(bgra: &[u8]) -> bool {
	bgra.as_chunks::<4>()
		.0
		.iter()
		.any(|pixel| (pixel[0] | pixel[1] | pixel[2]) != 0)
}

#[cfg(target_os = "windows")]
pub(super) use native::{DxgiCapture, StartError};

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
	use super::*;
	use crate::screen::{RawFrame, Settings as CaptureSettings};
	use std::{
		ffi::c_void,
		sync::{
			Arc,
			atomic::{AtomicBool, Ordering},
			mpsc::{SyncSender, sync_channel},
		},
		thread::JoinHandle,
	};
	use windows::{
		Win32::{
			Foundation::{
				E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, HMODULE, RPC_E_CHANGED_MODE, S_FALSE, S_OK,
			},
			Graphics::{
				Direct3D::{
					D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1,
					D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
				},
				Direct3D11::{
					D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
					D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
					D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
					ID3D11Texture2D,
				},
				Dxgi::{
					Common::{
						DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION,
						DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_ROTATE90,
						DXGI_MODE_ROTATION_ROTATE180, DXGI_MODE_ROTATION_ROTATE270,
						DXGI_MODE_ROTATION_UNSPECIFIED, DXGI_SAMPLE_DESC,
					},
					CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_DEVICE_REMOVED,
					DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_INVALID_CALL, DXGI_ERROR_NOT_FOUND,
					DXGI_ERROR_UNSUPPORTED, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_DESC,
					DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_POINTER_SHAPE_INFO,
					DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR,
					DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR,
					DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME, IDXGIAdapter, IDXGIFactory1,
					IDXGIOutput, IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication,
				},
			},
			System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
			UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext},
		},
		core::{HRESULT, Interface},
	};

	const MAX_ADAPTERS: u32 = 32;
	const MAX_OUTPUTS: u32 = 32;
	const ACQUIRE_TIMEOUT_MS: u32 = 50;
	const INITIAL_PRESENT_GRACE: Duration = Duration::from_millis(200);
	const STOP_POLL_SLICE: Duration = Duration::from_millis(25);
	const HIDDEN_CURSOR_BACKOFF: Duration = Duration::from_millis(4);

	pub(crate) enum StartError {
		Cancelled,
		Fallback(&'static str, Option<u32>),
	}

	pub(crate) struct DxgiCapture {
		local_stop: Arc<AtomicBool>,
		thread: Option<JoinHandle<()>>,
	}

	impl DxgiCapture {
		pub(crate) fn start(
			settings: CaptureSettings,
			target_hmonitor: usize,
			frames: SyncSender<RawFrame>,
			capture_stop: Arc<AtomicBool>,
			pending: Arc<AtomicBool>,
			cancel: &AtomicBool,
		) -> Result<Self, StartError> {
			if cancel.load(Ordering::Acquire) {
				return Err(StartError::Cancelled);
			}
			let local_stop = Arc::new(AtomicBool::new(false));
			let worker_local_stop = local_stop.clone();
			let (init_tx, init_rx) = sync_channel::<Result<(), (&'static str, Option<u32>)>>(1);
			let thread = std::thread::Builder::new()
				.name("screen-dxgi".into())
				.spawn(move || {
					run_worker(
						settings,
						target_hmonitor,
						frames,
						capture_stop,
						worker_local_stop,
						pending,
						init_tx,
					);
				})
				.map_err(|_| StartError::Fallback("DXGI worker thread could not start", None))?;

			match init_rx.recv() {
				Ok(Ok(())) => {
					if cancel.load(Ordering::Acquire) {
						local_stop.store(true, Ordering::Release);
						let _ = thread.join();
						return Err(StartError::Cancelled);
					}
					Ok(Self {
						local_stop,
						thread: Some(thread),
					})
				}
				Ok(Err((reason, code))) => {
					local_stop.store(true, Ordering::Release);
					let _ = thread.join();
					Err(StartError::Fallback(reason, code))
				}
				Err(_) => {
					local_stop.store(true, Ordering::Release);
					let _ = thread.join();
					Err(StartError::Fallback(
						"DXGI worker ended during startup",
						None,
					))
				}
			}
		}
	}

	impl Drop for DxgiCapture {
		fn drop(&mut self) {
			self.local_stop.store(true, Ordering::Release);
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
		}
	}

	struct ComGuard {
		uninitialize: bool,
	}

	impl ComGuard {
		fn init() -> Result<Self, &'static str> {
			// SAFETY: Worker thread owns its COM apartment and balances CoUninitialize in Drop
			// only when CoInitializeEx succeeded (S_OK or S_FALSE).
			let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
			if hr == S_OK || hr == S_FALSE {
				Ok(Self { uninitialize: true })
			} else if hr == RPC_E_CHANGED_MODE {
				Ok(Self {
					uninitialize: false,
				})
			} else {
				Err("COM initialization failed for DXGI capture")
			}
		}
	}

	impl Drop for ComGuard {
		fn drop(&mut self) {
			if self.uninitialize {
				// SAFETY: Balanced with CoInitializeEx on the same worker thread.
				unsafe { CoUninitialize() };
			}
		}
	}

	/// Signals `capture_stop` if the worker exits on its own so `encode_loop` never waits
	/// indefinitely on a finished DXGI worker.
	struct StopOnExit {
		capture_stop: Arc<AtomicBool>,
		local_stop: Arc<AtomicBool>,
		armed: bool,
	}

	impl Drop for StopOnExit {
		fn drop(&mut self) {
			if self.armed && !self.local_stop.load(Ordering::Acquire) {
				self.capture_stop.store(true, Ordering::Release);
			}
		}
	}

	struct FrameGuard(IDXGIOutputDuplication);

	impl Drop for FrameGuard {
		fn drop(&mut self) {
			// SAFETY: FrameGuard is only created after AcquireNextFrame succeeded on the same
			// worker thread that owns this duplication interface.
			let _ = unsafe { self.0.ReleaseFrame() };
		}
	}

	struct MappedGuard<'a> {
		context: &'a ID3D11DeviceContext,
		texture: &'a ID3D11Texture2D,
	}

	impl Drop for MappedGuard<'_> {
		fn drop(&mut self) {
			// SAFETY: Balanced with a successful ID3D11DeviceContext::Map on subresource 0.
			unsafe { self.context.Unmap(self.texture, 0) };
		}
	}

	enum OpenError {
		Hresult(HRESULT, &'static str),
		Other(&'static str),
	}

	impl OpenError {
		const fn reason(&self) -> &'static str {
			match self {
				Self::Hresult(_, reason) | Self::Other(reason) => reason,
			}
		}

		const fn code(&self) -> Option<u32> {
			match self {
				Self::Hresult(hr, _) => Some(hr.0 as u32),
				Self::Other(_) => None,
			}
		}

		const fn disposition(&self) -> RecoveryDisposition {
			match self {
				Self::Hresult(hr, _) => classify_recovery_hresult(hr.0 as u32),
				Self::Other(_) => RecoveryDisposition::RetryBounded,
			}
		}
	}

	struct DuplicationSession {
		device: ID3D11Device,
		context: ID3D11DeviceContext,
		duplication: IDXGIOutputDuplication,
		dupl_desc: DXGI_OUTDUPL_DESC,
		staging: Option<(D3D11_TEXTURE2D_DESC, ID3D11Texture2D)>,
	}

	impl DuplicationSession {
		fn open(target_hmonitor: usize) -> Result<Self, OpenError> {
			// SAFETY: CreateDXGIFactory1 returns a fresh COM factory bound to this worker thread.
			let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
				.map_err(|error| OpenError::Hresult(error.code(), "DXGI factory unavailable"))?;

			let (adapter, output) = find_adapter_and_output(&factory, target_hmonitor)?;

			let feature_levels = [
				D3D_FEATURE_LEVEL_11_1,
				D3D_FEATURE_LEVEL_11_0,
				D3D_FEATURE_LEVEL_10_1,
				D3D_FEATURE_LEVEL_10_0,
			];
			let adapter_base: IDXGIAdapter = adapter
				.cast()
				.map_err(|error| OpenError::Hresult(error.code(), "DXGI adapter cast failed"))?;
			let mut device = None;
			let mut context = None;
			// SAFETY: Passing an explicit IDXGIAdapter requires D3D_DRIVER_TYPE_UNKNOWN per MSDN.
			// Output pointers are valid local Option slots owned by this thread.
			unsafe {
				D3D11CreateDevice(
					&adapter_base,
					D3D_DRIVER_TYPE_UNKNOWN,
					HMODULE::default(),
					D3D11_CREATE_DEVICE_BGRA_SUPPORT,
					Some(&feature_levels),
					D3D11_SDK_VERSION,
					Some(&mut device),
					None,
					Some(&mut context),
				)
			}
			.map_err(|error| OpenError::Hresult(error.code(), "D3D11 device creation failed"))?;
			let device = device.ok_or(OpenError::Other("D3D11 returned no device"))?;
			let context = context.ok_or(OpenError::Other("D3D11 returned no device context"))?;

			let duplication = duplicate_output(&output, &device)?;
			// SAFETY: duplication is a live COM interface owned by this thread.
			let dupl_desc = unsafe { duplication.GetDesc() };
			if dupl_desc.ModeDesc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
				return Err(OpenError::Other("DXGI output format is not 8-bit BGRA"));
			}
			if dupl_desc.ModeDesc.Width == 0
				|| dupl_desc.ModeDesc.Height == 0
				|| dupl_desc.ModeDesc.Width > MAX_FRAME_WIDTH
				|| dupl_desc.ModeDesc.Height > MAX_FRAME_HEIGHT
			{
				return Err(OpenError::Other(
					"DXGI output dimensions exceed the 4K limit",
				));
			}

			let mut session = Self {
				device,
				context,
				duplication,
				dupl_desc,
				staging: None,
			};
			// Pre-allocate the initial staging texture during startup so allocation failures fall
			// back to Windows Graphics Capture immediately without waiting for a presented frame.
			let (staging_w, staging_h) = map_rotation(dupl_desc.Rotation)
				.output_dimensions(dupl_desc.ModeDesc.Width, dupl_desc.ModeDesc.Height);
			session
				.ensure_staging(staging_w, staging_h, dupl_desc.ModeDesc.Format)
				.map_err(|error| {
					OpenError::Hresult(error.code(), "Could not allocate DXGI staging texture")
				})?;
			Ok(session)
		}

		fn ensure_staging(
			&mut self,
			width: u32,
			height: u32,
			format: DXGI_FORMAT,
		) -> Result<&ID3D11Texture2D, windows::core::Error> {
			let needs_new = self.staging.as_ref().is_none_or(|(desc, _)| {
				desc.Width != width || desc.Height != height || desc.Format != format
			});
			if needs_new {
				let desc = D3D11_TEXTURE2D_DESC {
					Width: width,
					Height: height,
					MipLevels: 1,
					ArraySize: 1,
					Format: format,
					SampleDesc: DXGI_SAMPLE_DESC {
						Count: 1,
						Quality: 0,
					},
					Usage: D3D11_USAGE_STAGING,
					BindFlags: 0,
					CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
					MiscFlags: 0,
				};
				let mut texture = None;
				// SAFETY: `desc` describes a valid CPU-readable staging texture on `self.device`.
				unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut texture)) }?;
				let texture =
					texture.ok_or_else(|| windows::core::Error::from(DXGI_ERROR_INVALID_CALL))?;
				self.staging = Some((desc, texture));
			}
			Ok(&self.staging.as_ref().expect("staging texture").1)
		}

		fn copy_surface_to_bgra(
			&mut self,
			texture: &ID3D11Texture2D,
			dst: &mut Vec<u8>,
		) -> Result<(u32, u32), &'static str> {
			let mut tex_desc = D3D11_TEXTURE2D_DESC::default();
			// SAFETY: `texture` is a valid acquired ID3D11Texture2D.
			unsafe { texture.GetDesc(&mut tex_desc) };
			if tex_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
				return Err("DXGI frame format is not 8-bit BGRA");
			}
			let reported_rotation = map_rotation(self.dupl_desc.Rotation);
			let rotation = reported_rotation.effective(
				(tex_desc.Width, tex_desc.Height),
				(
					self.dupl_desc.ModeDesc.Width,
					self.dupl_desc.ModeDesc.Height,
				),
			);
			let (out_w, out_h) = rotation.output_dimensions(tex_desc.Width, tex_desc.Height);
			if out_w == 0 || out_h == 0 || out_w > MAX_FRAME_WIDTH || out_h > MAX_FRAME_HEIGHT {
				return Err("Captured frame exceeds the 4K limit");
			}

			self.ensure_staging(tex_desc.Width, tex_desc.Height, tex_desc.Format)
				.map_err(|_| "Could not allocate DXGI staging texture")?;
			let staging = &self.staging.as_ref().expect("staging texture").1;

			// SAFETY: Both textures belong to `self.device` and match dimensions/format.
			unsafe { self.context.CopyResource(staging, texture) };

			let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
			// SAFETY: `staging` is a D3D11_USAGE_STAGING texture with D3D11_CPU_ACCESS_READ.
			// MappedGuard unmaps subresource 0 before returning.
			unsafe {
				self.context
					.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
			}
			.map_err(|_| "Could not map DXGI staging texture")?;
			let _unmap = MappedGuard {
				context: &self.context,
				texture: staging,
			};

			if mapped.pData.is_null() {
				return Err("Mapped DXGI staging texture was empty");
			}
			let stride = mapped.RowPitch as usize;
			let src_len = stride
				.checked_mul(tex_desc.Height as usize)
				.filter(|len| *len <= MAX_RAW_BYTES * 2)
				.ok_or("Captured frame stride is invalid")?;
			// SAFETY: D3D11 guarantees `mapped.pData` points to at least `RowPitch * Height`
			// readable bytes until `Unmap` runs when `_unmap` drops.
			let mapped_bytes =
				unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), src_len) };
			copy_bgra_rotated(
				mapped_bytes,
				tex_desc.Width,
				tex_desc.Height,
				stride,
				rotation,
				dst,
			)
		}

		fn read_pointer_shape(
			&self,
			buffer_size: u32,
			scratch: &mut Vec<u8>,
		) -> Result<Option<PointerShape>, HRESULT> {
			let size = buffer_size as usize;
			if size == 0 || size > MAX_POINTER_SHAPE_BYTES {
				return Ok(None);
			}
			scratch.resize(size, 0);
			let mut required = 0u32;
			let mut info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
			// SAFETY: `scratch` has `size` writable bytes and `required`/`info` are valid stack
			// out-parameters; called while the current DXGI frame is acquired.
			let result = unsafe {
				self.duplication.GetFramePointerShape(
					buffer_size,
					scratch.as_mut_ptr().cast::<c_void>(),
					&mut required,
					&mut info,
				)
			};
			if let Err(error) = result {
				let code = error.code();
				if code == DXGI_ERROR_ACCESS_LOST
					|| code == DXGI_ERROR_DEVICE_REMOVED
					|| code == DXGI_ERROR_DEVICE_RESET
				{
					return Err(code);
				}
				return Ok(None);
			}
			let required = (required as usize).min(scratch.len());
			let Some(kind) = map_pointer_kind(info.Type) else {
				return Ok(None);
			};
			Ok(PointerShape::new(
				kind,
				info.Width,
				info.Height,
				info.Pitch,
				scratch[..required].to_vec(),
			))
		}
	}

	fn find_adapter_and_output(
		factory: &IDXGIFactory1,
		target_hmonitor: usize,
	) -> Result<(windows::Win32::Graphics::Dxgi::IDXGIAdapter1, IDXGIOutput), OpenError> {
		for adapter_index in 0..MAX_ADAPTERS {
			// SAFETY: `factory` is a valid IDXGIFactory1 on this thread.
			let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
				Ok(adapter) => adapter,
				Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
				Err(error) => {
					return Err(OpenError::Hresult(
						error.code(),
						"DXGI adapter enumeration failed",
					));
				}
			};
			for output_index in 0..MAX_OUTPUTS {
				// SAFETY: `adapter` is a valid IDXGIAdapter1 on this thread.
				let output = match unsafe { adapter.EnumOutputs(output_index) } {
					Ok(output) => output,
					Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
					Err(error) => {
						return Err(OpenError::Hresult(
							error.code(),
							"DXGI output enumeration failed",
						));
					}
				};
				// SAFETY: `output` is a valid IDXGIOutput on this thread.
				let Ok(desc) = (unsafe { output.GetDesc() }) else {
					continue;
				};
				if desc.AttachedToDesktop.as_bool() && desc.Monitor.0 as usize == target_hmonitor {
					return Ok((adapter, output));
				}
			}
		}
		Err(OpenError::Other(
			"Selected display was not found in DXGI outputs",
		))
	}

	fn duplicate_output(
		output: &IDXGIOutput,
		device: &ID3D11Device,
	) -> Result<IDXGIOutputDuplication, OpenError> {
		// Prefer IDXGIOutput5::DuplicateOutput1 requesting BGRA8 so DWM converts HDR/WCG desktops
		// into 8-bit SDR BGRA when supported (Windows 10 1607+), falling back to DuplicateOutput.
		if let Ok(output5) = output.cast::<IDXGIOutput5>() {
			// SAFETY: `device` was created on `output`'s parent adapter; formats slice is valid.
			match unsafe { output5.DuplicateOutput1(device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]) } {
				Ok(duplication) => return Ok(duplication),
				Err(error)
					if error.code() == E_NOTIMPL
						|| error.code() == E_NOINTERFACE
						|| error.code() == E_INVALIDARG
						|| error.code() == DXGI_ERROR_UNSUPPORTED => {}
				Err(error) => {
					return Err(OpenError::Hresult(
						error.code(),
						"DXGI DuplicateOutput1 failed",
					));
				}
			}
		}
		let output1: IDXGIOutput1 = output
			.cast()
			.map_err(|error| OpenError::Hresult(error.code(), "IDXGIOutput1 unavailable"))?;
		// SAFETY: `device` was created on `output`'s parent adapter.
		unsafe { output1.DuplicateOutput(device) }
			.map_err(|error| OpenError::Hresult(error.code(), "DXGI DuplicateOutput failed"))
	}

	const fn map_rotation(rotation: DXGI_MODE_ROTATION) -> Rotation {
		match rotation {
			DXGI_MODE_ROTATION_ROTATE90 => Rotation::Rotate90,
			DXGI_MODE_ROTATION_ROTATE180 => Rotation::Rotate180,
			DXGI_MODE_ROTATION_ROTATE270 => Rotation::Rotate270,
			DXGI_MODE_ROTATION_IDENTITY | DXGI_MODE_ROTATION_UNSPECIFIED => Rotation::Identity,
			_ => Rotation::Identity,
		}
	}

	const fn map_pointer_kind(raw: u32) -> Option<PointerShapeKind> {
		if raw == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 as u32 {
			Some(PointerShapeKind::Monochrome)
		} else if raw == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 as u32 {
			Some(PointerShapeKind::Color)
		} else if raw == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0 as u32 {
			Some(PointerShapeKind::MaskedColor)
		} else {
			None
		}
	}

	fn stopped(capture_stop: &AtomicBool, local_stop: &AtomicBool) -> bool {
		capture_stop.load(Ordering::Acquire) || local_stop.load(Ordering::Acquire)
	}

	fn sleep_interruptible(
		duration: Duration,
		capture_stop: &AtomicBool,
		local_stop: &AtomicBool,
	) -> bool {
		let deadline = Instant::now() + duration;
		while !stopped(capture_stop, local_stop) {
			let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
				return true;
			};
			if remaining.is_zero() {
				return true;
			}
			std::thread::sleep(remaining.min(STOP_POLL_SLICE));
		}
		false
	}

	fn diagnostics_enabled() -> bool {
		std::env::var_os("SEREIN_VOICE_DIAGNOSTICS").is_some_and(|value| value == "1")
	}

	fn recover_session(
		target_hmonitor: usize,
		capture_stop: &AtomicBool,
		local_stop: &AtomicBool,
		budget: &mut RecoveryBudget,
	) -> Option<DuplicationSession> {
		let mut backoff = Duration::from_millis(50);
		while !stopped(capture_stop, local_stop) {
			if !sleep_interruptible(backoff, capture_stop, local_stop) {
				return None;
			}
			match DuplicationSession::open(target_hmonitor) {
				Ok(session) => return Some(session),
				Err(error) => {
					if !budget.note_failure(Instant::now(), error.disposition()) {
						if diagnostics_enabled() {
							eprintln!(
								"[Serein voice Screen] dxgi_recovery_failed={}",
								error.reason()
							);
						}
						return None;
					}
					backoff = (backoff * 2).min(Duration::from_millis(250));
				}
			}
		}
		None
	}

	#[allow(clippy::too_many_arguments)]
	fn run_worker(
		settings: CaptureSettings,
		target_hmonitor: usize,
		frames: SyncSender<RawFrame>,
		capture_stop: Arc<AtomicBool>,
		local_stop: Arc<AtomicBool>,
		pending: Arc<AtomicBool>,
		init_tx: SyncSender<Result<(), (&'static str, Option<u32>)>>,
	) {
		let _com = match ComGuard::init() {
			Ok(guard) => guard,
			Err(reason) => {
				let _ = init_tx.send(Err((reason, None)));
				return;
			}
		};
		// SAFETY: Thread-scoped Per-Monitor V2 DPI awareness ensures physical monitor coordinates
		// for DXGI duplication without altering process-wide DPI state.
		unsafe {
			let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
		}

		let mut session = match DuplicationSession::open(target_hmonitor) {
			Ok(session) => session,
			Err(error) => {
				let _ = init_tx.send(Err((error.reason(), error.code())));
				return;
			}
		};
		if init_tx.send(Ok(())).is_err() {
			return;
		}
		let _exit_guard = StopOnExit {
			capture_stop: capture_stop.clone(),
			local_stop: local_stop.clone(),
			armed: true,
		};

		let interval = Duration::from_secs_f64(1.0 / f64::from(settings.fps));
		let mut next_frame = Instant::now();
		let mut desktop_bgra = Vec::new();
		let mut desktop_size = (0u32, 0u32);
		let mut have_desktop = false;
		let mut initial_fallback: Option<((u32, u32), Vec<u8>)> = None;
		let mut session_opened = Instant::now();
		let mut cursor = CursorTracker::default();
		let mut pointer_scratch = Vec::new();
		let mut dirty = false;
		let mut recovery_budget = RecoveryBudget::default();

		while !stopped(&capture_stop, &local_stop) {
			if have_desktop {
				let now = Instant::now();
				if pending.load(Ordering::Acquire) {
					std::thread::sleep(Duration::from_millis(2));
					continue;
				}
				if now + Duration::from_millis(2) < next_frame {
					let wait = next_frame
						.saturating_duration_since(now)
						.saturating_sub(Duration::from_millis(2))
						.clamp(Duration::from_millis(1), Duration::from_millis(16));
					std::thread::sleep(wait);
					continue;
				}
			}

			let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
			let mut resource = None;
			let timeout_ms = if dirty && have_desktop {
				0
			} else {
				ACQUIRE_TIMEOUT_MS
			};
			// SAFETY: `session.duplication` lives on this thread; out-pointers are valid locals.
			let acquire = unsafe {
				session
					.duplication
					.AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
			};

			let mut needs_recreate = false;
			let mut hidden_cursor_only = false;

			match acquire {
				Ok(()) => {
					let _release = FrameGuard(session.duplication.clone());

					let mut shape_updated = false;
					if frame_info.PointerShapeBufferSize > 0 {
						match session.read_pointer_shape(
							frame_info.PointerShapeBufferSize,
							&mut pointer_scratch,
						) {
							Ok(Some(shape)) => {
								cursor.set_shape(shape);
								shape_updated = true;
							}
							Ok(None) => {}
							Err(_) => needs_recreate = true,
						}
					}

					let mut cursor_changed = false;
					if frame_info.LastMouseUpdateTime != 0 {
						cursor_changed = cursor.update_position(
							frame_info.PointerPosition.Visible.as_bool(),
							frame_info.PointerPosition.Position.x,
							frame_info.PointerPosition.Position.y,
							shape_updated,
						);
					} else if shape_updated && cursor.visible {
						cursor_changed = true;
					}

					if !needs_recreate && frame_info.LastPresentTime != 0 {
						if let Some(resource) = resource
							&& let Ok(texture) = resource.cast::<ID3D11Texture2D>()
						{
							match session.copy_surface_to_bgra(&texture, &mut desktop_bgra) {
								Ok(size) => {
									desktop_size = size;
									have_desktop = true;
									initial_fallback = None;
									recovery_budget.clear();
									dirty = true;
								}
								Err(_) => needs_recreate = true,
							}
						}
					} else if !needs_recreate && !have_desktop {
						// Some drivers deliver the initial static desktop with LastPresentTime == 0
						// while others return an empty black surface before the first present.
						if initial_fallback.is_none()
							&& let Some(resource) = resource
							&& let Ok(texture) = resource.cast::<ID3D11Texture2D>()
						{
							let mut candidate = Vec::new();
							match session.copy_surface_to_bgra(&texture, &mut candidate) {
								Ok(size) => {
									if has_visible_pixels(&candidate) {
										desktop_bgra = candidate;
										desktop_size = size;
										have_desktop = true;
										recovery_budget.clear();
										dirty = true;
									} else {
										initial_fallback = Some((size, candidate));
									}
								}
								Err(_) => needs_recreate = true,
							}
						}
					} else if !needs_recreate {
						if settings.cursor && cursor_changed {
							dirty = true;
						} else if frame_info.LastMouseUpdateTime != 0 {
							hidden_cursor_only = true;
						}
					}
				}
				Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {}
				Err(error)
					if error.code() == DXGI_ERROR_ACCESS_LOST
						|| error.code() == DXGI_ERROR_DEVICE_REMOVED
						|| error.code() == DXGI_ERROR_DEVICE_RESET
						|| error.code() == DXGI_ERROR_INVALID_CALL =>
				{
					needs_recreate = true;
				}
				Err(_) => {
					needs_recreate = true;
				}
			}

			// Promote an all-black initial desktop candidate after the grace period even if
			// pointer-only updates keep AcquireNextFrame returning Ok(()) without a timeout.
			if !needs_recreate
				&& !have_desktop
				&& session_opened.elapsed() >= INITIAL_PRESENT_GRACE
				&& let Some((size, candidate)) = initial_fallback.take()
			{
				desktop_bgra = candidate;
				desktop_size = size;
				have_desktop = true;
				recovery_budget.clear();
				dirty = true;
			}

			if needs_recreate {
				// Release the old duplication and D3D11 device before requesting a replacement.
				drop(session);
				cursor.clear_shape();
				have_desktop = false;
				initial_fallback = None;
				dirty = false;
				if !recovery_budget.note_failure(Instant::now(), RecoveryDisposition::RetryBounded)
				{
					if diagnostics_enabled() {
						eprintln!(
							"[Serein voice Screen] dxgi_recovery_failed=repeated DXGI frame capture failures"
						);
					}
					return;
				}
				let Some(recovered) = recover_session(
					target_hmonitor,
					&capture_stop,
					&local_stop,
					&mut recovery_budget,
				) else {
					return;
				};
				session = recovered;
				session_opened = Instant::now();
				next_frame = Instant::now();
				continue;
			}

			if hidden_cursor_only && !dirty {
				std::thread::sleep(HIDDEN_CURSOR_BACKOFF.min(interval / 2));
				continue;
			}

			if have_desktop && dirty {
				let now = Instant::now();
				if now + Duration::from_millis(2) < next_frame
					|| pending.swap(true, Ordering::AcqRel)
				{
					continue;
				}
				next_frame = (next_frame + interval).max(now + interval / 2);
				let (width, height) = desktop_size;
				let mut data = desktop_bgra.clone();
				cursor.composite_onto(&mut data, width, height, settings.cursor);
				dirty = false;
				if frames
					.try_send(RawFrame {
						width,
						height,
						stride: width as usize * 4,
						data,
					})
					.is_err()
				{
					pending.store(false, Ordering::Release);
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn selects_dxgi_only_when_wgc_border_setting_is_unavailable() {
		assert!(should_use_dxgi_for_display(Some(false)));
		assert!(!should_use_dxgi_for_display(Some(true)));
		assert!(!should_use_dxgi_for_display(None));
	}

	#[test]
	fn rotates_and_unpacks_padded_bgra_frames_and_rejects_oversized_or_truncated() {
		// 2x3 unrotated surface with 4 bytes of row padding per row.
		// Pixels by (x, y):
		// (0,0)=A  (1,0)=B
		// (0,1)=C  (1,1)=D
		// (0,2)=E  (1,2)=F
		let px = |tag: u8| [tag, tag + 1, tag + 2, 255];
		let mut src = Vec::new();
		for row in [[px(10), px(20)], [px(30), px(40)], [px(50), px(60)]] {
			src.extend_from_slice(&row[0]);
			src.extend_from_slice(&row[1]);
			src.extend_from_slice(&[0xee; 4]);
		}
		let stride = 12;

		let mut out = Vec::new();
		assert_eq!(
			copy_bgra_rotated(&src, 2, 3, stride, Rotation::Identity, &mut out).unwrap(),
			(2, 3)
		);
		assert_eq!(
			out,
			[px(10), px(20), px(30), px(40), px(50), px(60)].concat()
		);

		// Rotate90 rotates 90 deg CW -> 3x2:
		// row 0: (0,2)=E  (0,1)=C  (0,0)=A
		// row 1: (1,2)=F  (1,1)=D  (1,0)=B
		assert_eq!(
			copy_bgra_rotated(&src, 2, 3, stride, Rotation::Rotate90, &mut out).unwrap(),
			(3, 2)
		);
		assert_eq!(
			out,
			[px(50), px(30), px(10), px(60), px(40), px(20)].concat()
		);

		// Rotate180 -> 2x3 reversed:
		// row 0: F E, row 1: D C, row 2: B A
		assert_eq!(
			copy_bgra_rotated(&src, 2, 3, stride, Rotation::Rotate180, &mut out).unwrap(),
			(2, 3)
		);
		assert_eq!(
			out,
			[px(60), px(50), px(40), px(30), px(20), px(10)].concat()
		);

		// Rotate270 rotates 90 deg CCW -> 3x2:
		// row 0: (1,0)=B  (1,1)=D  (1,2)=F
		// row 1: (0,0)=A  (0,1)=C  (0,2)=E
		assert_eq!(
			copy_bgra_rotated(&src, 2, 3, stride, Rotation::Rotate270, &mut out).unwrap(),
			(3, 2)
		);
		assert_eq!(
			out,
			[px(20), px(40), px(60), px(10), px(30), px(50)].concat()
		);

		// Pre-rotated texture matching portrait ModeDesc must not be rotated a second time.
		assert_eq!(
			Rotation::Rotate90.effective((1080, 1920), (1080, 1920)),
			Rotation::Identity
		);
		assert_eq!(
			Rotation::Rotate90.effective((1920, 1080), (1080, 1920)),
			Rotation::Rotate90
		);

		// Stride smaller than row bytes or truncated buffer is rejected.
		assert_eq!(
			copy_bgra_rotated(&src, 2, 3, 4, Rotation::Identity, &mut out),
			Err("Captured frame buffer is incomplete")
		);
		assert_eq!(
			copy_bgra_rotated(&src[..20], 2, 3, stride, Rotation::Identity, &mut out),
			Err("Captured frame buffer is incomplete")
		);
		// A complete source row wider than 4K must be rejected by the 4K limit check.
		let oversized = vec![0u8; (MAX_FRAME_WIDTH as usize + 1) * 4];
		assert_eq!(
			copy_bgra_rotated(
				&oversized,
				MAX_FRAME_WIDTH + 1,
				1,
				oversized.len(),
				Rotation::Identity,
				&mut out,
			),
			Err("Captured frame exceeds the 4K limit")
		);
	}

	#[test]
	fn composites_monochrome_color_and_masked_cursors_with_edge_clipping() {
		// 1. Monochrome: 4x1 cursor (raw height 2: 1 row AND mask, 1 row XOR mask).
		// Pixels: (0,0)=black, (0,1)=white, (1,0)=transparent, (1,1)=invert.
		// MSB-first: AND = 0b0011_0000 = 0x30, XOR = 0b0101_0000 = 0x50.
		let mono = PointerShape::new(PointerShapeKind::Monochrome, 4, 2, 1, vec![0x30, 0x50])
			.expect("valid monochrome shape");
		let mut frame = vec![
			10, 20, 30, 255, // x=0 -> black
			40, 50, 60, 255, // x=1 -> white
			70, 80, 90, 255, // x=2 -> unchanged
			100, 150, 200, 255, // x=3 -> inverted
		];
		composite_pointer(&mut frame, 4, 1, 0, 0, &mono);
		assert_eq!(
			frame,
			vec![
				0, 0, 0, 255, // black
				255, 255, 255, 255, // white
				70, 80, 90, 255, // transparent
				155, 105, 55, 255, // inverted
			]
		);

		// 2. Color (straight alpha) with negative origin clipping out the first column.
		let color = PointerShape::new(
			PointerShapeKind::Color,
			2,
			1,
			8,
			vec![
				0, 0, 255, 255, // clipped left
				200, 100, 50, 128, // ~50% blend onto [100, 100, 100, 255]
			],
		)
		.expect("valid color shape");
		let mut frame = vec![100, 100, 100, 255, 10, 20, 30, 255];
		composite_pointer(&mut frame, 2, 1, -1, 0, &color);
		assert_eq!(&frame[..4], &[150, 100, 75, 255]);
		assert_eq!(&frame[4..8], &[10, 20, 30, 255]);

		// 3. MaskedColor: mask=0 replaces, mask=0xFF XORs RGB.
		let masked = PointerShape::new(
			PointerShapeKind::MaskedColor,
			2,
			1,
			8,
			vec![
				12, 34, 56, 0x00, // opaque replace
				0xff, 0x00, 0xff, 0xff, // XOR B and R, keep G
			],
		)
		.expect("valid masked shape");
		let mut frame = vec![10, 20, 30, 255, 10, 20, 30, 255];
		composite_pointer(&mut frame, 2, 1, 0, 0, &masked);
		assert_eq!(frame, vec![12, 34, 56, 255, 245, 20, 225, 255]);

		// Invalid shapes must be rejected before compositing.
		assert!(PointerShape::new(PointerShapeKind::Monochrome, 4, 3, 1, vec![0; 3]).is_none());
		assert!(PointerShape::new(PointerShapeKind::Color, 4, 1, 8, vec![0; 8]).is_none());
		assert!(PointerShape::new(PointerShapeKind::Color, 2, 2, 8, vec![0; 12]).is_none());
	}

	#[test]
	fn cursor_tracker_suppresses_off_monitor_moves_and_clears_cursor_on_exit() {
		let shape = PointerShape::new(PointerShapeKind::Color, 1, 1, 4, vec![255, 255, 255, 255])
			.expect("valid shape");
		let mut tracker = CursorTracker::default();
		tracker.set_shape(shape);

		// Moving while hidden on another monitor should not emit frames.
		assert!(!tracker.update_position(false, 10, 10, false));
		assert!(!tracker.update_position(false, 20, 20, false));

		// Entering the monitor triggers a frame; compositing marks cursor drawn.
		assert!(tracker.update_position(true, 0, 0, false));
		let mut frame = vec![0, 0, 0, 255];
		tracker.composite_onto(&mut frame, 1, 1, true);
		assert_eq!(frame, vec![255, 255, 255, 255]);

		// Leaving the monitor triggers one cleanup frame, then goes quiet again.
		assert!(tracker.update_position(false, 0, 0, false));
		let mut clean = vec![10, 20, 30, 255];
		tracker.composite_onto(&mut clean, 1, 1, true);
		assert_eq!(clean, vec![10, 20, 30, 255]);
		assert!(!tracker.update_position(false, 5, 5, false));

		// Clearing cursor state after a DXGI session reset suppresses draws until a shape arrives.
		tracker.clear_shape();
		assert!(!tracker.update_position(true, 0, 0, false));
	}

	#[test]
	fn recovery_waits_across_secure_desktop_and_bounds_hard_errors() {
		assert_eq!(
			classify_recovery_hresult(E_ACCESSDENIED_CODE),
			RecoveryDisposition::KeepWaiting
		);
		assert_eq!(
			classify_recovery_hresult(DXGI_ERROR_SESSION_DISCONNECTED_CODE),
			RecoveryDisposition::KeepWaiting
		);
		assert_eq!(
			classify_recovery_hresult(0x887A_0004), // DXGI_ERROR_UNSUPPORTED
			RecoveryDisposition::RetryBounded
		);
		assert!(!has_visible_pixels(&[0, 0, 0, 255, 0, 0, 0, 0]));
		assert!(has_visible_pixels(&[0, 1, 0, 255]));

		let start = Instant::now();
		let mut budget = RecoveryBudget::default();
		assert!(budget.note_failure(start, RecoveryDisposition::RetryBounded));
		// Repeated failures without a delivered frame share the same 3-second deadline.
		assert!(budget.note_failure(
			start + RECOVERY_LIMIT - Duration::from_millis(1),
			RecoveryDisposition::RetryBounded,
		));
		assert!(!budget.note_failure(start + RECOVERY_LIMIT, RecoveryDisposition::RetryBounded));

		// Secure-desktop waits pause the hard-error clock; a successful copy clears it.
		assert!(budget.note_failure(start + RECOVERY_LIMIT, RecoveryDisposition::KeepWaiting));
		assert!(budget.note_failure(
			start + RECOVERY_LIMIT + Duration::from_secs(1),
			RecoveryDisposition::RetryBounded,
		));
		budget.clear();
		assert!(budget.note_failure(
			start + RECOVERY_LIMIT + Duration::from_secs(10),
			RecoveryDisposition::RetryBounded,
		));
	}
}
