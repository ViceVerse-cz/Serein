//! Opt-in native tray icon. Linux routes window close to an available tray; Quit exits.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Event {
	Show = 1,
	Unavailable = 2,
	Quit = 4,
}

pub const fn supported() -> bool {
	cfg!(any(target_os = "windows", target_os = "linux"))
}

#[cfg(any(target_os = "windows", test))]
#[derive(Default)]
struct Events(std::cell::Cell<u8>);

#[cfg(any(target_os = "windows", test))]
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

#[cfg(target_os = "windows")]
pub use native::Tray;

#[cfg(target_os = "linux")]
#[path = "tray/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Tray;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub struct Tray;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl Tray {
	pub fn new(
		_window: std::sync::Arc<winit::window::Window>,
		_wake: impl Fn() + 'static,
	) -> Result<Self, &'static str> {
		Err("The tray icon is currently available on Windows and Linux only.")
	}
	pub fn take_event(&self) -> Option<Event> {
		None
	}
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
			let icon = unsafe {
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
	}

	impl State {
		fn add_icon(&self) -> bool {
			// SAFETY: this initialized descriptor contains only live borrowed handles and fixed text.
			unsafe {
				if !Shell_NotifyIconW(NIM_ADD, &self.icon).as_bool() {
					return false;
				}
				if !Shell_NotifyIconW(NIM_SETVERSION, &self.icon).as_bool() {
					let _ = Shell_NotifyIconW(NIM_DELETE, &self.icon);
					return false;
				}
			}
			self.present.set(true);
			true
		}
		fn remove_icon(&self) {
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
		}
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
