//! Opt-out native tray icon. Minimizing keeps its normal window behavior; the application
//! decides what closing does: hide when supported, otherwise ask the compositor to minimize.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Event {
	Show = 1,
	Unavailable = 2,
	Quit = 4,
	#[cfg(target_os = "linux")]
	Minimize = 8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum VoiceState {
	#[default]
	Unmuted = 0,
	Speaking = 1,
	Muted = 2,
	Deafened = 3,
}

// Keep the desired state for Explorer recovery, and only remember a successful
// presentation. A rejected native update is retried on the next background tick.
#[cfg(any(target_os = "windows", test))]
fn update_voice_state(
	desired: &std::cell::Cell<VoiceState>,
	applied: &std::cell::Cell<Option<VoiceState>>,
	state: VoiceState,
	apply: impl FnOnce(VoiceState) -> bool,
) {
	desired.set(state);
	if applied.get() != Some(state) && apply(state) {
		applied.set(Some(state));
	}
}

pub const fn supported() -> bool {
	cfg!(any(
		target_os = "windows",
		target_os = "macos",
		target_os = "linux"
	))
}

#[cfg(any(target_os = "windows", target_os = "macos", test))]
#[derive(Default)]
struct Events(std::cell::Cell<u8>);

#[cfg(any(target_os = "windows", target_os = "macos", test))]
impl Events {
	fn push(&self, event: Event) {
		self.0.set(self.0.get() | event as u8);
	}
	fn take(&self) -> Option<Event> {
		let event = [Event::Quit, Event::Unavailable, Event::Show]
			.into_iter()
			.find(|event| self.0.get() & *event as u8 != 0)?;
		self.0.set(self.0.get() & !(event as u8));
		Some(event)
	}
}

#[cfg(target_os = "linux")]
#[path = "tray/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Tray;

#[cfg(target_os = "windows")]
pub use native::Tray;

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
#[path = "tray/macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Tray;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub struct Tray;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
impl Tray {
	pub fn new(
		_window: std::sync::Arc<winit::window::Window>,
		_wake: impl Fn() + 'static,
	) -> Result<Self, &'static str> {
		Err("The tray icon is unavailable on this platform.")
	}
	pub fn take_event(&self) -> Option<Event> {
		None
	}
	pub fn set_voice_state(&self, _state: VoiceState) {}
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
	use super::{Event, Events};
	use std::{cell::Cell, rc::Rc, sync::Arc};
	use windows::{
		Win32::{
			Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, WPARAM},
			System::Threading::GetCurrentThreadId,
			UI::{Shell::*, WindowsAndMessaging::*},
		},
		core::w,
	};
	use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

	const ID: usize = 0x5352;
	const SHOW: usize = 1;
	const QUIT: usize = 2;
	const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
	const UNAVAILABLE: &str = "The Windows tray is unavailable. The window will remain accessible.";
	const ATLAS: &[u8] = include_bytes!("../../../assets/icons/atlas.png");
	const ICON_SIZE: usize = 32;

	/// UI-thread-owned registration: no worker, timer or allocating event queue.
	/// The retained window and Rc prevent cross-thread drop or a dangling subclass callback.
	pub struct Tray {
		state: Rc<State>,
		_window: Arc<winit::window::Window>,
	}

	struct State {
		icon: NOTIFYICONDATAW,
		menu: HMENU,
		previous: WNDPROC,
		restart: u32,
		hooked: Cell<bool>,
		enabled: Cell<bool>,
		present: Cell<bool>,
		events: Events,
		wake: Box<dyn Fn()>,
		voice_icons: [Option<HICON>; 4],
		current_voice_state: Cell<super::VoiceState>,
		applied_voice_state: Cell<Option<super::VoiceState>>,
	}

	impl Tray {
		pub fn new(
			window: Arc<winit::window::Window>,
			wake: impl Fn() + 'static,
		) -> Result<Self, &'static str> {
			let RawWindowHandle::Win32(handle) =
				window.window_handle().map_err(|_| UNAVAILABLE)?.as_raw()
			else {
				return Err(UNAVAILABLE);
			};
			let hwnd = HWND(handle.hwnd.get() as *mut _);
			// SAFETY: the retained winit window owns hwnd. Hooks must run on its UI thread.
			let previous = unsafe {
				if GetWindowThreadProcessId(hwnd, None) != GetCurrentThreadId()
					|| !GetPropW(hwnd, w!("Serein.TrayState")).is_invalid()
				{
					return Err(UNAVAILABLE);
				}
				let previous = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
				if previous == 0 {
					return Err(UNAVAILABLE);
				}
				std::mem::transmute::<isize, WNDPROC>(previous)
			};
			// SAFETY: these fixed names register messages, without taking ownership of pointers.
			let (notification, restart) = unsafe {
				(
					RegisterWindowMessageW(w!("Serein.TrayCallback")),
					RegisterWindowMessageW(w!("TaskbarCreated")),
				)
			};
			if notification == 0 || restart == 0 {
				return Err(UNAVAILABLE);
			}
			// SAFETY: request a borrowed window icon; the fallback is a shared system icon.
			let fallback_icon = unsafe {
				let handle = HICON(
					SendMessageW(hwnd, WM_GETICON, Some(WPARAM(ICON_SMALL2 as usize)), None).0
						as *mut _,
				);
				if handle.is_invalid() {
					LoadIconW(None, IDI_APPLICATION).map_err(|_| UNAVAILABLE)?
				} else {
					handle
				}
			};
			// SAFETY: creates a menu owned by State, released on every success/error path.
			let menu = unsafe { CreatePopupMenu() }.map_err(|_| UNAVAILABLE)?;
			// Create owned icons only after fallible borrowed-icon/menu initialization.
			let voice_icons = create_voice_icons();
			let icon = voice_icons[super::VoiceState::Unmuted as usize].unwrap_or(fallback_icon);
			let mut data = NOTIFYICONDATAW {
				cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
				hWnd: hwnd,
				uID: ID as u32,
				uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
				uCallbackMessage: notification,
				hIcon: icon,
				Anonymous: NOTIFYICONDATAW_0 {
					uVersion: NOTIFYICON_VERSION_4,
				},
				..Default::default()
			};
			for (slot, unit) in data.szTip.iter_mut().zip("Serein".encode_utf16()) {
				*slot = unit;
			}
			let tray = Self {
				state: Rc::new(State {
					icon: data,
					menu,
					previous,
					restart,
					hooked: Cell::new(false),
					enabled: Cell::new(true),
					present: Cell::new(false),
					events: Events::default(),
					wake: Box::new(wake),
					voice_icons,
					current_voice_state: Cell::new(super::VoiceState::Unmuted),
					applied_voice_state: Cell::new(None),
				}),
				_window: window,
			};
			// SAFETY: menu and hwnd are live UI-thread handles; Rc keeps callback state stable.
			unsafe {
				AppendMenuW(menu, MF_STRING, SHOW, w!("Show Serein")).map_err(|_| UNAVAILABLE)?;
				AppendMenuW(menu, MF_STRING, QUIT, w!("Quit")).map_err(|_| UNAVAILABLE)?;
				SetMenuDefaultItem(menu, SHOW as u32, 0).map_err(|_| UNAVAILABLE)?;
				let reference = Rc::into_raw(tray.state.clone());
				if SetPropW(
					hwnd,
					w!("Serein.TrayState"),
					Some(HANDLE(reference.cast_mut().cast())),
				)
				.is_err()
				{
					drop(Rc::from_raw(reference));
					return Err(UNAVAILABLE);
				}
				if SetWindowLongPtrW(hwnd, GWLP_WNDPROC, callback as *const () as isize) == 0 {
					let _ = RemovePropW(hwnd, w!("Serein.TrayState"));
					drop(Rc::from_raw(reference));
					return Err(UNAVAILABLE);
				}
			}
			tray.state.hooked.set(true);
			if !tray.state.add_icon() {
				return Err(UNAVAILABLE);
			}
			Ok(tray)
		}

		pub fn take_event(&self) -> Option<Event> {
			self.state.events.take()
		}

		pub fn set_voice_state(&self, state: super::VoiceState) {
			super::update_voice_state(
				&self.state.current_voice_state,
				&self.state.applied_voice_state,
				state,
				|state| {
					let hicon =
						self.state.voice_icons[state as usize].unwrap_or(self.state.icon.hIcon);
					let mut data = NOTIFYICONDATAW {
						cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
						hWnd: self.state.icon.hWnd,
						uID: self.state.icon.uID,
						uFlags: NIF_ICON | NIF_TIP | NIF_SHOWTIP,
						hIcon: hicon,
						Anonymous: NOTIFYICONDATAW_0 {
							uVersion: NOTIFYICON_VERSION_4,
						},
						..Default::default()
					};
					let tip = voice_state_tip(state);
					for (slot, unit) in data.szTip.iter_mut().zip(tip.encode_utf16()) {
						*slot = unit;
					}
					// SAFETY: the descriptor uses retained UI-thread handles and fixed tooltip text.
					unsafe { Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() }
				},
			);
		}
	}

	fn voice_state_tip(state: super::VoiceState) -> &'static str {
		match state {
			super::VoiceState::Unmuted => "Serein",
			super::VoiceState::Speaking => "Serein (Speaking)",
			super::VoiceState::Muted => "Serein (Muted)",
			super::VoiceState::Deafened => "Serein (Deafened)",
		}
	}

	impl State {
		fn add_icon(&self) -> bool {
			let mut icon_data = self.icon;
			let state = self.current_voice_state.get();
			if let Some(hicon) = self.voice_icons[state as usize] {
				icon_data.hIcon = hicon;
			}
			icon_data.szTip = [0; 128];
			for (slot, unit) in icon_data
				.szTip
				.iter_mut()
				.zip(voice_state_tip(state).encode_utf16())
			{
				*slot = unit;
			}
			// SAFETY: this initialized descriptor contains only live borrowed handles and fixed text.
			unsafe {
				if !Shell_NotifyIconW(NIM_ADD, &icon_data).as_bool() {
					return false;
				}
				if !Shell_NotifyIconW(NIM_SETVERSION, &icon_data).as_bool() {
					let _ = Shell_NotifyIconW(NIM_DELETE, &icon_data);
					return false;
				}
			}
			self.present.set(true);
			self.applied_voice_state.set(Some(state));
			true
		}
		fn remove_icon(&self) {
			self.applied_voice_state.set(None);
			if self.present.replace(false) {
				// SAFETY: hwnd/uID identify only this application's owned notification icon.
				let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &self.icon) };
			}
		}
		fn restore(&self) {
			// SAFETY: called while the retained window/subclass is live on its owning thread.
			unsafe {
				let _ = ShowWindow(self.icon.hWnd, SW_RESTORE);
				let _ = SetForegroundWindow(self.icon.hWnd);
			}
		}
		fn emit(&self, event: Event) {
			self.events.push(event);
			(self.wake)();
		}
		fn menu(&self) {
			let mut cursor = POINT::default();
			// SAFETY: live owned menu/window and stack cursor; TrackPopupMenu runs a nested UI loop.
			let command = unsafe {
				if GetCursorPos(&mut cursor).is_err() {
					return;
				}
				let _ = SetForegroundWindow(self.icon.hWnd);
				let command = TrackPopupMenu(
					self.menu,
					TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
					cursor.x,
					cursor.y,
					None,
					self.icon.hWnd,
					None,
				)
				.0 as usize;
				let _ = PostMessageW(Some(self.icon.hWnd), WM_NULL, WPARAM(0), LPARAM(0));
				command
			};
			self.activate(command);
		}
		fn activate(&self, command: usize) {
			if let Some(event) = match command {
				SHOW => Some(Event::Show),
				QUIT => Some(Event::Quit),
				_ => None,
			} {
				self.restore();
				self.emit(event);
			}
		}
	}

	impl Drop for Tray {
		fn drop(&mut self) {
			self.state.enabled.set(false);
			if self.state.hooked.get() {
				// SAFETY: Rc makes Tray !Send; only restore our hook if it is still atop the chain.
				unsafe {
					let hwnd = self.state.icon.hWnd;
					if GetWindowLongPtrW(hwnd, GWLP_WNDPROC) == callback as *const () as isize
						&& SetWindowLongPtrW(
							hwnd,
							GWLP_WNDPROC,
							self.state.previous.unwrap() as *const () as isize,
						) != 0
					{
						self.state.hooked.set(false);
						let _ = RemovePropW(hwnd, w!("Serein.TrayState"));
						drop(Rc::from_raw(Rc::as_ptr(&self.state)));
					}
				}
				// A newer hook may still call ours: retain disabled state until WM_NCDESTROY.
			}
			self.state.remove_icon();
		}
	}
	impl Drop for State {
		fn drop(&mut self) {
			// SAFETY: this state owns the menu; callback Rc copies keep it alive during nested menus.
			let _ = unsafe { DestroyMenu(self.menu) };
			for hicon in self.voice_icons.into_iter().flatten() {
				let _ = unsafe { DestroyIcon(hicon) };
			}
		}
	}

	fn create_voice_icons() -> [Option<HICON>; 4] {
		let Ok(atlas_image) = image::load_from_memory_with_format(ATLAS, image::ImageFormat::Png)
		else {
			return [None, None, None, None];
		};
		let atlas = atlas_image.into_rgba8();

		let extract = |cell: usize, rgb: [u8; 3]| -> Option<HICON> {
			let col = cell % 8;
			let row = cell / 8;
			let base_x = (col * 64) as u32;
			let base_y = (row * 64) as u32;
			let mut bgra = [0u8; ICON_SIZE * ICON_SIZE * 4];
			let [cr, cg, cb] = rgb;

			for dy in 0..ICON_SIZE {
				for dx in 0..ICON_SIZE {
					let sx = base_x + (dx as u32) * 2;
					let sy = base_y + (dy as u32) * 2;
					let a00 = atlas.get_pixel(sx, sy)[3] as u32;
					let a01 = atlas.get_pixel(sx + 1, sy)[3] as u32;
					let a10 = atlas.get_pixel(sx, sy + 1)[3] as u32;
					let a11 = atlas.get_pixel(sx + 1, sy + 1)[3] as u32;
					let alpha = ((a00 + a01 + a10 + a11 + 2) / 4) as u8;

					let idx = (dy * ICON_SIZE + dx) * 4;
					let a = alpha as u16;
					bgra[idx] = ((cb as u16 * a) / 255) as u8;
					bgra[idx + 1] = ((cg as u16 * a) / 255) as u8;
					bgra[idx + 2] = ((cr as u16 * a) / 255) as u8;
					bgra[idx + 3] = alpha;
				}
			}
			unsafe { create_hicon_from_bgra(ICON_SIZE as u32, ICON_SIZE as u32, &bgra) }
		};

		let extract_gradient =
			|cell: usize, top_left: [u8; 3], bottom_right: [u8; 3]| -> Option<HICON> {
				let col = cell % 8;
				let row = cell / 8;
				let base_x = (col * 64) as u32;
				let base_y = (row * 64) as u32;
				let mut bgra = [0u8; ICON_SIZE * ICON_SIZE * 4];

				for dy in 0..ICON_SIZE {
					for dx in 0..ICON_SIZE {
						let sx = base_x + (dx as u32) * 2;
						let sy = base_y + (dy as u32) * 2;
						let a00 = atlas.get_pixel(sx, sy)[3] as u32;
						let a01 = atlas.get_pixel(sx + 1, sy)[3] as u32;
						let a10 = atlas.get_pixel(sx, sy + 1)[3] as u32;
						let a11 = atlas.get_pixel(sx + 1, sy + 1)[3] as u32;
						let alpha = ((a00 + a01 + a10 + a11 + 2) / 4) as u8;

						// Diagonal interpolation factor across 32x32: dx + dy in 0..=62
						let t = (dx + dy) as u32;
						let cr = ((top_left[0] as u32 * (62 - t) + bottom_right[0] as u32 * t) / 62)
							as u16;
						let cg = ((top_left[1] as u32 * (62 - t) + bottom_right[1] as u32 * t) / 62)
							as u16;
						let cb = ((top_left[2] as u32 * (62 - t) + bottom_right[2] as u32 * t) / 62)
							as u16;

						let idx = (dy * ICON_SIZE + dx) * 4;
						let a = alpha as u16;
						bgra[idx] = ((cb * a) / 255) as u8;
						bgra[idx + 1] = ((cg * a) / 255) as u8;
						bgra[idx + 2] = ((cr * a) / 255) as u8;
						bgra[idx + 3] = alpha;
					}
				}
				unsafe { create_hicon_from_bgra(ICON_SIZE as u32, ICON_SIZE as u32, &bgra) }
			};

		[
			extract(58, [215, 218, 224]),
			extract_gradient(58, [100, 165, 255], [120, 50, 230]),
			extract(4, [242, 63, 67]),
			extract(6, [242, 63, 67]),
		]
	}

	unsafe fn create_hicon_from_bgra(width: u32, height: u32, bgra: &[u8]) -> Option<HICON> {
		use windows::Win32::Graphics::Gdi::{
			BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
			DeleteObject,
		};
		use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

		let mask_stride = width.div_ceil(32) * 4;
		let mask_bytes = vec![0u8; (mask_stride * height) as usize];
		// SAFETY: mask_bytes has valid stride and length for width * height monochrome bitmap.
		let mask = unsafe {
			CreateBitmap(
				width as i32,
				height as i32,
				1,
				1,
				Some(mask_bytes.as_ptr().cast()),
			)
		};
		if mask.is_invalid() {
			return None;
		}

		let bi = BITMAPINFOHEADER {
			biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
			biWidth: width as i32,
			biHeight: -(height as i32),
			biPlanes: 1,
			biBitCount: 32,
			biCompression: BI_RGB.0,
			..Default::default()
		};
		let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
		let info = BITMAPINFO {
			bmiHeader: bi,
			bmiColors: [Default::default()],
		};
		// SAFETY: creates a 32-bit top-down DIB section for icon color data.
		let color =
			match unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) } {
				Ok(c) => c,
				Err(_) => {
					let _ = unsafe { DeleteObject(mask.into()) };
					return None;
				}
			};

		if !bits.is_null() {
			// SAFETY: bits points to a valid DIB buffer of width * height * 4 bytes.
			unsafe {
				std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits.cast(), bgra.len());
			}
		}

		let icon_info = ICONINFO {
			fIcon: true.into(),
			xHotspot: 0,
			yHotspot: 0,
			hbmMask: mask,
			hbmColor: color,
		};
		// SAFETY: mask and color are valid owned HBITMAPs; icon is created and bitmap handles are released.
		let hicon = unsafe { CreateIconIndirect(&icon_info).ok() };
		let _ = unsafe { DeleteObject(mask.into()) };
		let _ = unsafe { DeleteObject(color.into()) };
		hicon
	}

	unsafe extern "system" fn callback(
		hwnd: HWND,
		message: u32,
		wparam: WPARAM,
		lparam: LPARAM,
	) -> LRESULT {
		// SAFETY: the UI-thread registration installed this property before replacing WNDPROC.
		let pointer = unsafe { GetPropW(hwnd, w!("Serein.TrayState")).0.cast::<State>() };
		if pointer.is_null() {
			// SAFETY: no owned state is accessible; use the OS default rather than a stale pointer.
			return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
		}
		// SAFETY: Tray retains this Rc until the subclass is removed. Retain a temporary Rc so
		// nested menu dispatch may disable/drop Tray without invalidating the current callback.
		let state = unsafe {
			Rc::increment_strong_count(pointer);
			Rc::from_raw(pointer)
		};
		if state.enabled.get() && message == state.icon.uCallbackMessage {
			match lparam.0 as u32 & 0xffff {
				NIN_SELECT | NIN_KEYSELECT => {
					state.restore();
					state.emit(Event::Show);
				}
				WM_CONTEXTMENU => state.menu(),
				_ => {}
			}
			return LRESULT(0);
		}
		if state.enabled.get() && message == state.restart {
			state.present.set(false);
			if !state.add_icon() {
				state.restore();
				state.emit(Event::Unavailable);
			}
		}
		if message == WM_NCDESTROY {
			let hooked = state.hooked.replace(false);
			state.remove_icon();
			// SAFETY: remove only our property as the window is destroyed.
			let _ = unsafe { RemovePropW(hwnd, w!("Serein.TrayState")) };
			if hooked {
				// SAFETY: the destroyed window cannot dispatch again; release its registration Rc.
				unsafe {
					drop(Rc::from_raw(pointer));
				}
			}
		}
		// SAFETY: every unhandled message follows the original winit subclass chain.
		unsafe { CallWindowProcW(state.previous, hwnd, message, wparam, lparam) }
	}

	#[cfg(test)]
	mod tests {
		use super::*;
		use winit::{event_loop::EventLoop, platform::windows::EventLoopBuilderExtWindows};

		#[test]
		#[ignore = "Requires an interactive Windows shell; creates only a synthetic test window/icon"]
		#[allow(deprecated)]
		fn native_minimize_restore_restart_quit_and_cleanup() {
			let mut builder = EventLoop::builder();
			builder.with_any_thread(true);
			let event_loop = builder.build().unwrap();
			let window = Arc::new(
				event_loop
					.create_window(
						winit::window::Window::default_attributes()
							.with_title("Serein synthetic tray test")
							.with_inner_size(winit::dpi::LogicalSize::new(320., 200.))
							.with_visible(false),
					)
					.unwrap(),
			);
			let wakes = Rc::new(Cell::new(0));
			let wake = wakes.clone();
			let tray = Tray::new(window.clone(), move || wake.set(wake.get() + 1)).unwrap();
			let hwnd = tray.state.icon.hWnd;
			let icon = tray.state.icon;
			assert!(Tray::new(window.clone(), || {}).is_err());
			// SAFETY: every API here targets only this test-owned synthetic window/menu/icon.
			unsafe {
				let _ = ShowWindow(hwnd, SW_MINIMIZE);
				assert!(IsWindowVisible(hwnd).as_bool());
				assert!(IsIconic(hwnd).as_bool());
				let _ = SendMessageW(
					hwnd,
					icon.uCallbackMessage,
					None,
					Some(LPARAM(((ID as u32) << 16 | NIN_KEYSELECT) as isize)),
				);
				assert!(IsWindowVisible(hwnd).as_bool());
				assert!(!IsIconic(hwnd).as_bool());
				assert_eq!(tray.take_event(), Some(Event::Show));
				tray.state.remove_icon();
				let _ = SendMessageW(hwnd, tray.state.restart, None, None);
				assert!(tray.state.present.get());
				assert_eq!(GetMenuItemID(tray.state.menu, 1), QUIT as u32);
				tray.state.activate(QUIT);
				assert_eq!(tray.take_event(), Some(Event::Quit));
				assert!(IsWindow(Some(hwnd)).as_bool()); // Quit is an app event, never forced destruction.
				let _ = ShowWindow(hwnd, SW_MINIMIZE);
				assert!(IsWindowVisible(hwnd).as_bool());
				drop(tray);
				assert!(IsWindowVisible(hwnd).as_bool());
				assert!(GetPropW(hwnd, w!("Serein.TrayState")).is_invalid());
				assert_ne!(
					GetWindowLongPtrW(hwnd, GWLP_WNDPROC),
					callback as *const () as isize
				);
				assert!(!Shell_NotifyIconW(NIM_MODIFY, &icon).as_bool());
				// Startup can minimize before the asynchronous preference enables the tray.
				let _ = ShowWindow(hwnd, SW_MINIMIZE);
				let late_tray = Tray::new(window.clone(), || {}).unwrap();
				assert!(IsIconic(hwnd).as_bool());
				assert!(IsWindowVisible(hwnd).as_bool());
				drop(late_tray);
				assert!(IsWindowVisible(hwnd).as_bool());
				assert!(IsIconic(hwnd).as_bool());
			}
			assert_eq!(wakes.get(), 2);
			window.set_visible(false);
		}
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn failed_voice_icon_updates_retry_without_losing_the_desired_state() {
		use super::{VoiceState, update_voice_state};
		use std::cell::Cell;
		let desired = Cell::new(VoiceState::Unmuted);
		let applied = Cell::new(Some(VoiceState::Unmuted));
		let attempts = Cell::new(0);
		let update = |success| {
			update_voice_state(&desired, &applied, VoiceState::Muted, |_| {
				attempts.set(attempts.get() + 1);
				success
			});
		};
		update(false);
		assert_eq!(desired.get(), VoiceState::Muted);
		assert_eq!(applied.get(), Some(VoiceState::Unmuted));
		update(true);
		update(true);
		assert_eq!(attempts.get(), 2);
		assert_eq!(applied.get(), Some(VoiceState::Muted));
		applied.set(None); // Explorer lost the registration.
		update(true);
		assert_eq!(attempts.get(), 3);
		for state in [VoiceState::Speaking, VoiceState::Deafened] {
			update_voice_state(&desired, &applied, state, |_| true);
			assert_eq!(applied.get(), Some(state));
		}
	}

	use super::*;
	#[test]
	fn clicks_coalesce_without_losing_quit_or_failure() {
		let events = Events::default();
		events.push(Event::Quit);
		for _ in 0..1000 {
			events.push(Event::Show);
		}
		events.push(Event::Unavailable);
		assert_eq!(events.take(), Some(Event::Quit));
		assert_eq!(events.take(), Some(Event::Unavailable));
		assert_eq!(events.take(), Some(Event::Show));
		assert_eq!(events.take(), None);
	}
}
