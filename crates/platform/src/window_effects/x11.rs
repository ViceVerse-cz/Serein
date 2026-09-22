//! Requests compositor blur on X11 desktops supporting KDE's blur property.
#![allow(unsafe_code)]

use winit::raw_window_handle::{
	HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use x11_dl::xlib;

pub(super) fn set_blur(window: &winit::window::Window, enabled: bool) -> Result<(), &'static str> {
	let window_handle = window
		.window_handle()
		.map_err(|_| "Native window unavailable.")?;
	let display_handle = window
		.display_handle()
		.map_err(|_| "Native display unavailable.")?;
	let (RawWindowHandle::Xlib(handle), RawDisplayHandle::Xlib(display)) =
		(window_handle.as_raw(), display_handle.as_raw())
	else {
		return Err("Expected an X11 window.");
	};
	let display = display.display.ok_or("Native display unavailable.")?;
	let xlib = xlib::Xlib::open().map_err(|_| "X11 window effects unavailable.")?;
	// SAFETY: both handles belong to the borrowed winit window. Xlib manages its
	// connection locking; the property contains no data and retains no pointers.
	unsafe {
		let display = display.as_ptr().cast();
		let atom = (xlib.XInternAtom)(display, c"_KDE_NET_WM_BLUR_BEHIND_REGION".as_ptr(), 0);
		if atom == 0 {
			return Err("X11 blur property unavailable.");
		}
		if enabled {
			// An empty CARDINAL region requests blur behind the entire window.
			(xlib.XChangeProperty)(
				display,
				handle.window,
				atom,
				xlib::XA_CARDINAL,
				32,
				xlib::PropModeReplace,
				std::ptr::null(),
				0,
			);
		} else {
			(xlib.XDeleteProperty)(display, handle.window, atom);
		}
		(xlib.XFlush)(display);
	}
	Ok(())
}
