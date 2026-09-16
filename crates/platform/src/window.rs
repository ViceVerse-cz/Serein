//! Native macOS title-bar appearance without recreating the window or its webviews.
#![allow(unsafe_code)]

pub fn set_native_title_bar(
	window: &winit::window::Window,
	native: bool,
) -> Result<(), &'static str> {
	use objc2::MainThreadMarker;
	use objc2_app_kit::{NSView, NSWindowStyleMask, NSWindowTitleVisibility};
	use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

	let _main = MainThreadMarker::new().ok_or("Window appearance requires the main thread.")?;
	let RawWindowHandle::AppKit(handle) = window
		.window_handle()
		.map_err(|_| "Native window unavailable.")?
		.as_raw()
	else {
		return Err("Expected a macOS window.");
	};
	// SAFETY: winit owns this NSView for the borrowed window's lifetime; AppKit access
	// stays on the main thread and the containing window is retained while in use.
	let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
	let window = view.window().ok_or("Native window unavailable.")?;
	let mut style = window.styleMask();
	if style.contains(NSWindowStyleMask::FullSizeContentView) == native {
		style.set(NSWindowStyleMask::FullSizeContentView, !native);
		window.setStyleMask(style);
		window.setTitlebarAppearsTransparent(!native);
		window.setTitleVisibility(if native {
			NSWindowTitleVisibility::Visible
		} else {
			NSWindowTitleVisibility::Hidden
		});
	}
	Ok(())
}
