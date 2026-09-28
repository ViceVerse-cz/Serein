//! DWM draws a system backdrop only inside the window frame. The app paints its own caption,
//! so the frame is extended over the whole client area while blur is on; the transparent
//! composition swapchain then lets the acrylic show through wherever surfaces are translucent.
#![allow(unsafe_code)]

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
	SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

pub fn extend_frame(window: &Window, extended: bool) -> Result<(), String> {
	let Ok(RawWindowHandle::Win32(handle)) = window.window_handle().map(|h| h.as_raw()) else {
		return Err("Expected a Win32 window.".to_owned());
	};
	// -1 on every side extends the frame over the full client area ("sheet of glass").
	let inset = if extended { -1 } else { 0 };
	let margins = MARGINS {
		cxLeftWidth: inset,
		cxRightWidth: inset,
		cyTopHeight: inset,
		cyBottomHeight: inset,
	};
	// SAFETY: winit keeps this HWND alive for the borrowed window, and `margins` outlives the call.
	let hwnd = HWND(handle.hwnd.get() as _);
	// Re-run winit's undecorated WM_NCCALCSIZE handling after DWM changes the frame.
	unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) }.map_err(|error| error.to_string())?;
	unsafe {
		SetWindowPos(
			hwnd,
			None,
			0,
			0,
			0,
			0,
			SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
		)
	}
	.map_err(|error| error.to_string())
}
