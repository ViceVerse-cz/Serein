//! One running client per user session. A second launch asks the running client to show its
//! window and exits, instead of starting another tray-resident instance. macOS bundles are
//! unique through LaunchServices, so there the Dock's reopen request shows the window.

use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
};

#[cfg(all(
	any(target_os = "windows", target_os = "linux"),
	not(feature = "development-data")
))]
const NAME: &str = "cz.viceverse.serein";
#[cfg(all(
	any(target_os = "windows", target_os = "linux"),
	feature = "development-data"
))]
const NAME: &str = "cz.viceverse.serein.development";

pub enum Launch {
	Primary(Instance),
	/// Another instance owns this session and was asked to show itself.
	Forwarded,
}

/// Owns this session's instance claim; pending show requests are read by the UI.
#[derive(Default)]
pub struct Instance {
	requested: Arc<AtomicBool>,
	#[cfg(target_os = "windows")]
	event: Option<win::Event>,
	#[cfg(target_os = "linux")]
	listener: Option<std::os::unix::net::UnixListener>,
	#[cfg(target_os = "macos")]
	reopen: Option<macos::Reopen>,
}

/// Claims this session. A failed claim never blocks startup; it only allows duplicates.
pub fn claim() -> Launch {
	#[cfg(target_os = "windows")]
	return match win::claim() {
		Some(event) => Launch::Primary(Instance {
			event,
			..Instance::default()
		}),
		None => Launch::Forwarded,
	};
	#[cfg(target_os = "linux")]
	return match linux::claim() {
		Some(listener) => Launch::Primary(Instance {
			listener,
			..Instance::default()
		}),
		None => Launch::Forwarded,
	};
	#[cfg(not(any(target_os = "windows", target_os = "linux")))]
	Launch::Primary(Instance::default())
}

impl Instance {
	/// Starts answering later launches. `restore` runs off the UI thread before `wake`, for
	/// compositors that send no frames to a hidden window.
	pub fn listen(
		&mut self,
		#[cfg_attr(not(target_os = "macos"), allow(unused_variables))] window: Arc<
			winit::window::Window,
		>,
		restore: impl Fn() + Send + Sync + 'static,
		wake: impl Fn() + Send + Sync + 'static,
	) {
		let requested = self.requested.clone();
		let request = move || {
			restore();
			requested.store(true, Ordering::Release);
			wake();
		};
		#[cfg(target_os = "windows")]
		if let Some(event) = self.event.take() {
			event.listen(request);
		}
		#[cfg(target_os = "linux")]
		if let Some(listener) = self.listener.take() {
			linux::listen(listener, request);
		}
		#[cfg(target_os = "macos")]
		{
			self.reopen = macos::Reopen::install(window, request);
		}
		#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
		let _ = request;
	}

	/// Whether another launch or a Dock reopen asked to show the window since the last call.
	pub fn take_show_request(&self) -> bool {
		self.requested.swap(false, Ordering::AcqRel)
	}
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod win {
	use windows::{
		Win32::{
			Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
			System::Threading::{
				CreateEventW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent,
				WaitForSingleObject,
			},
			UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow},
		},
		core::HSTRING,
	};

	/// A named auto-reset event in this logon session; it disappears with its last handle.
	pub struct Event(HANDLE);
	// SAFETY: kernel event handles may be waited on and closed from any thread.
	unsafe impl Send for Event {}

	fn name() -> HSTRING {
		HSTRING::from(format!("Local\\{}.show", super::NAME))
	}

	/// `None` when another instance owns the event and was signalled.
	pub fn claim() -> Option<Option<Event>> {
		let name = name();
		// SAFETY: the name outlives the call; default security keeps the event per-session.
		let Ok(handle) = (unsafe { CreateEventW(None, false, false, &name) }) else {
			return Some(None);
		};
		// SAFETY: read immediately after the CreateEventW call that set it.
		if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
			return Some(Some(Event(handle)));
		}
		// SAFETY: closes only the duplicate handle opened above.
		let _ = unsafe { CloseHandle(handle) };
		// SAFETY: the name outlives the call and the returned handle is closed below.
		let Ok(existing) = (unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &name) }) else {
			return Some(None);
		};
		// SAFETY: this launch was started by the user, so it may lend foreground rights to the
		// running instance before waking it; `existing` is a valid event handle.
		unsafe {
			let _ = AllowSetForegroundWindow(ASFW_ANY);
			let _ = SetEvent(existing);
			let _ = CloseHandle(existing);
		}
		None
	}

	impl Event {
		fn wait(&self) -> bool {
			// SAFETY: the handle stays open for the process lifetime of the waiting thread.
			unsafe { WaitForSingleObject(self.0, INFINITE) == WAIT_OBJECT_0 }
		}
		pub fn listen(self, request: impl Fn() + Send + 'static) {
			let _ = std::thread::Builder::new()
				.name("serein-instance".into())
				.spawn(move || {
					while self.wait() {
						request();
					}
				});
		}
	}
}

#[cfg(target_os = "linux")]
mod linux {
	use std::{
		io::ErrorKind,
		os::unix::net::{UnixListener, UnixStream},
		path::PathBuf,
	};

	/// The per-user runtime directory is private to this user, so only their launches connect.
	fn path() -> Option<PathBuf> {
		let directory = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
		directory
			.is_absolute()
			.then(|| directory.join(format!("{}.sock", super::NAME)))
	}

	/// `None` when a running instance accepted the connection, which is its show request.
	pub fn claim() -> Option<Option<UnixListener>> {
		let Some(path) = path() else {
			return Some(None);
		};
		match UnixListener::bind(&path) {
			Ok(listener) => Some(Some(listener)),
			Err(error) if error.kind() == ErrorKind::AddrInUse => {
				if UnixStream::connect(&path).is_ok() {
					return None;
				}
				// A crashed instance left its socket behind.
				let _ = std::fs::remove_file(&path);
				Some(UnixListener::bind(&path).ok())
			}
			Err(_) => Some(None),
		}
	}

	pub fn listen(listener: UnixListener, request: impl Fn() + Send + 'static) {
		let _ = std::thread::Builder::new()
			.name("serein-instance".into())
			.spawn(move || {
				// Connections carry no data; accepting one is the request.
				for stream in listener.incoming() {
					if stream.is_ok() {
						request();
					}
				}
			});
	}
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod macos {
	use objc2::{
		DefinedClass, MainThreadOnly, class, define_class, msg_send, rc::Retained,
		runtime::AnyObject, sel,
	};
	use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol};
	use std::sync::Arc;

	/// `kCoreEventClass` / `kAEReopenApplication`: Dock clicks and relaunches of a running app.
	const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
	const REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp");

	struct State {
		window: Arc<winit::window::Window>,
		request: Box<dyn Fn()>,
	}

	define_class!(
		// SAFETY: NSObject has no subclassing requirements. The handler stays on the main
		// thread and is retained until it is unregistered on drop.
		#[unsafe(super = NSObject)]
		#[name = "SereinReopenHandler"]
		#[thread_kind = MainThreadOnly]
		#[ivars = State]
		struct Handler;

		// SAFETY: NSObjectProtocol has no additional requirements.
		unsafe impl NSObjectProtocol for Handler {}

		impl Handler {
			#[unsafe(method(handleReopen:withReplyEvent:))]
			fn reopen(&self, _event: &AnyObject, _reply: &AnyObject) {
				let state = self.ivars();
				// A hidden window may receive no frames, so show it before waking the UI.
				state.window.set_visible(true);
				state.window.set_minimized(false);
				state.window.focus_window();
				(state.request)();
			}
		}
	);

	pub struct Reopen {
		_handler: Retained<Handler>,
	}

	impl Reopen {
		/// Replaces AppKit's reopen handler, which only restores minimized windows.
		pub fn install(
			window: Arc<winit::window::Window>,
			request: impl Fn() + 'static,
		) -> Option<Self> {
			let mtm = MainThreadMarker::new()?;
			let handler = Handler::alloc(mtm).set_ivars(State {
				window,
				request: Box::new(request),
			});
			// SAFETY: NSObject init initializes this allocated NSObject subclass.
			let handler: Retained<Handler> = unsafe { msg_send![super(handler), init] };
			// SAFETY: the selector is implemented above with the Apple event handler signature,
			// and Reopen keeps the handler alive until it is unregistered.
			unsafe {
				let manager: Retained<AnyObject> =
					msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
				let _: () = msg_send![
					&manager,
					setEventHandler: &*handler,
					andSelector: sel!(handleReopen:withReplyEvent:),
					forEventClass: CORE_EVENT_CLASS,
					andEventID: REOPEN_APPLICATION,
				];
			}
			Some(Self { _handler: handler })
		}
	}

	impl Drop for Reopen {
		fn drop(&mut self) {
			// SAFETY: unregisters the handler installed above before it is released.
			unsafe {
				let manager: Retained<AnyObject> =
					msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
				let _: () = msg_send![
					&manager,
					removeEventHandlerForEventClass: CORE_EVENT_CLASS,
					andEventID: REOPEN_APPLICATION,
				];
			}
		}
	}
}
