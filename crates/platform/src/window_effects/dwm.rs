//! DWM draws a system backdrop only inside the window frame. The app paints its own caption,
//! so the frame is extended over the whole client area while blur is on; the transparent
//! composition swapchain then lets the acrylic show through wherever surfaces are translucent.
#![allow(unsafe_code)]

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
	GWL_STYLE, GetPropW, GetWindowLongPtrW, HTCAPTION, RemovePropW, SC_MOVE, STYLESTRUCT,
	SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetPropW,
	SetWindowLongPtrW, SetWindowPos, WM_NCDESTROY, WM_NCLBUTTONDOWN, WM_STYLECHANGING,
	WM_SYSCOMMAND, WS_SYSMENU,
};
use windows::core::{PCWSTR, w};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

// A nonzero scalar, not a pointer: WS_SYSMENU plus a presence bit, even in fullscreen.
const REQUESTED_MENU: PCWSTR = w!("Serein.CustomCaption.RequestedMenu");

// winit retains WS_SYSMENU even for undecorated windows and restores it on style changes.
// With a full-client DWM frame this paints native buttons underneath our custom buttons.
unsafe extern "system" fn custom_caption(
	hwnd: HWND,
	message: u32,
	wparam: WPARAM,
	lparam: LPARAM,
	id: usize,
	_data: usize,
) -> LRESULT {
	// SAFETY: the subclass only receives messages for its live HWND on the window thread.
	unsafe {
		if message == WM_NCDESTROY {
			let _ = RemoveWindowSubclass(hwnd, Some(custom_caption), id);
			let _ = RemovePropW(hwnd, REQUESTED_MENU);
		}
		if (message == WM_NCLBUTTONDOWN && wparam.0 == HTCAPTION as usize)
			|| (message == WM_SYSCOMMAND && (wparam.0 & 0xFFF0) == SC_MOVE as usize)
		{
			let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
			if style & WS_SYSMENU.0 as isize == 0 {
				SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_SYSMENU.0 as isize);
				let result = DefSubclassProc(hwnd, message, wparam, lparam);
				SetWindowLongPtrW(hwnd, GWL_STYLE, style);
				return result;
			}
		}
		let result = DefSubclassProc(hwnd, message, wparam, lparam);
		if message == WM_STYLECHANGING && wparam.0 as i32 == GWL_STYLE.0 {
			// WM_STYLECHANGING supplies a writable STYLESTRUCT for this synchronous call.
			let style = &mut *(lparam.0 as *mut STYLESTRUCT);
			let menu = (style.styleNew & WS_SYSMENU.0) as usize | 1;
			// Store the latest window-mode request before masking it, including fullscreen.
			let _ = SetPropW(hwnd, REQUESTED_MENU, Some(HANDLE(menu as _)));
			style.styleNew &= !WS_SYSMENU.0;
		}
		result
	}
}

fn set_custom_caption(hwnd: HWND, enabled: bool) -> Result<(), String> {
	// SAFETY: called on the window thread with a live HWND. No borrowed callback data is stored.
	unsafe {
		let requested_menu = GetPropW(hwnd, REQUESTED_MENU);
		let installed = !requested_menu.0.is_null();
		if enabled == installed {
			return Ok(());
		}
		let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
		if style == 0 {
			return Err(windows::core::Error::from_win32().to_string());
		}
		let style = if enabled {
			let menu = (style as u32 & WS_SYSMENU.0) as usize | 1;
			SetPropW(hwnd, REQUESTED_MENU, Some(HANDLE(menu as _)))
				.map_err(|error| error.to_string())?;
			if !SetWindowSubclass(hwnd, Some(custom_caption), 0, 0).as_bool() {
				let _ = RemovePropW(hwnd, REQUESTED_MENU);
				return Err("Could not install the custom-caption subclass.".to_owned());
			}
			// Let the callback mask the initial request too, without recording our own mask.
			style
		} else {
			RemoveWindowSubclass(hwnd, Some(custom_caption), 0)
				.ok()
				.map_err(|error| error.to_string())?;
			let _ = RemovePropW(hwnd, REQUESTED_MENU);
			(style & !(WS_SYSMENU.0 as isize)) | (requested_menu.0 as isize & WS_SYSMENU.0 as isize)
		};
		if SetWindowLongPtrW(hwnd, GWL_STYLE, style) == 0 {
			return Err(windows::core::Error::from_win32().to_string());
		}
	}
	Ok(())
}

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
	set_custom_caption(hwnd, extended && !window.is_decorated())?;
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

#[cfg(test)]
mod tests {
	use super::*;
	use windows::Win32::UI::WindowsAndMessaging::{
		CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
	};
	use windows::core::w;

	#[test]
	fn custom_caption_survives_style_rewrites_and_restores_native_controls() {
		// SAFETY: this hidden test window is created, used and destroyed on this thread.
		unsafe {
			let hwnd = CreateWindowExW(
				WINDOW_EX_STYLE::default(),
				w!("STATIC"),
				w!("caption test"),
				WS_OVERLAPPEDWINDOW,
				0,
				0,
				100,
				100,
				None,
				None,
				None,
				None,
			)
			.unwrap();
			let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
			set_custom_caption(hwnd, true).unwrap();
			assert_eq!(
				GetWindowLongPtrW(hwnd, GWL_STYLE),
				style & !(WS_SYSMENU.0 as isize)
			);
			// winit rewrites all style bits during maximize, restore and transparency changes.
			SetWindowLongPtrW(hwnd, GWL_STYLE, style);
			assert_eq!(
				GetWindowLongPtrW(hwnd, GWL_STYLE),
				style & !(WS_SYSMENU.0 as isize)
			);
			set_custom_caption(hwnd, false).unwrap();
			assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
			set_custom_caption(hwnd, true).unwrap();
			// Borderless fullscreen removes the menu while suppression is active.
			let fullscreen_style = style & !(WS_OVERLAPPEDWINDOW.0 as isize);
			SetWindowLongPtrW(hwnd, GWL_STYLE, fullscreen_style);
			set_custom_caption(hwnd, false).unwrap();
			assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), fullscreen_style);
			// Disabling twice, or enabling in fullscreen, must not invent a menu.
			set_custom_caption(hwnd, false).unwrap();
			set_custom_caption(hwnd, true).unwrap();
			set_custom_caption(hwnd, false).unwrap();
			assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), fullscreen_style);
			// Returning to windowed mode must restore the latest request, not the
			// absent menu captured when the subclass was installed in fullscreen.
			set_custom_caption(hwnd, true).unwrap();
			SetWindowLongPtrW(hwnd, GWL_STYLE, style);
			set_custom_caption(hwnd, true).unwrap();
			assert_eq!(
				GetWindowLongPtrW(hwnd, GWL_STYLE),
				style & !(WS_SYSMENU.0 as isize)
			);
			set_custom_caption(hwnd, false).unwrap();
			assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), style);
			set_custom_caption(hwnd, true).unwrap();
			DestroyWindow(hwnd).unwrap();
		}
	}

	#[test]
	fn custom_caption_allows_drag_without_leaking_menu() {
		// SAFETY: this hidden test window is created, used and destroyed on this thread.
		unsafe {
			let hwnd = CreateWindowExW(
				WINDOW_EX_STYLE::default(),
				w!("STATIC"),
				w!("drag test"),
				WS_OVERLAPPEDWINDOW,
				0,
				0,
				100,
				100,
				None,
				None,
				None,
				None,
			)
			.unwrap();
			let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
			set_custom_caption(hwnd, true).unwrap();
			assert_eq!(
				GetWindowLongPtrW(hwnd, GWL_STYLE),
				style & !(WS_SYSMENU.0 as isize)
			);
			use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
			let _ = SendMessageW(
				hwnd,
				WM_NCLBUTTONDOWN,
				Some(WPARAM(HTCAPTION as usize)),
				Some(LPARAM(0)),
			);
			assert_eq!(
				GetWindowLongPtrW(hwnd, GWL_STYLE),
				style & !(WS_SYSMENU.0 as isize)
			);
			let _ = SendMessageW(
				hwnd,
				WM_SYSCOMMAND,
				Some(WPARAM(SC_MOVE as usize)),
				Some(LPARAM(0)),
			);
			assert_eq!(
				GetWindowLongPtrW(hwnd, GWL_STYLE),
				style & !(WS_SYSMENU.0 as isize)
			);
			DestroyWindow(hwnd).unwrap();
		}
	}
}
