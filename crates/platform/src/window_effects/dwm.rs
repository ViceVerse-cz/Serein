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
// Modal move loops dispatch other windows' messages on this same thread.
const TEMPORARY_DRAG: PCWSTR = w!("Serein.CustomCaption.TemporaryDragDepth");

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
			let _ = RemovePropW(hwnd, TEMPORARY_DRAG);
		}
		if (message == WM_NCLBUTTONDOWN && wparam.0 == HTCAPTION as usize)
			|| (message == WM_SYSCOMMAND && (wparam.0 & 0xFFF0) == SC_MOVE as usize)
		{
			let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
			let depth = GetPropW(hwnd, TEMPORARY_DRAG).0 as usize;
			if let Some(next) = depth.checked_add(1)
				&& SetPropW(hwnd, TEMPORARY_DRAG, Some(HANDLE(next as _))).is_ok()
			{
				let added_menu = style & WS_SYSMENU.0 as isize == 0;
				if added_menu {
					SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_SYSMENU.0 as isize);
				}
				let result = DefSubclassProc(hwnd, message, wparam, lparam);
				// Suppression may have been disabled while the modal move loop dispatched events.
				if added_menu && !GetPropW(hwnd, REQUESTED_MENU).0.is_null() {
					let current_style = GetWindowLongPtrW(hwnd, GWL_STYLE);
					SetWindowLongPtrW(hwnd, GWL_STYLE, current_style & !(WS_SYSMENU.0 as isize));
				}
				if !GetPropW(hwnd, TEMPORARY_DRAG).0.is_null() {
					if depth == 0 {
						let _ = RemovePropW(hwnd, TEMPORARY_DRAG);
					} else {
						let _ = SetPropW(hwnd, TEMPORARY_DRAG, Some(HANDLE(depth as _)));
					}
				}
				return result;
			}
		}
		let result = DefSubclassProc(hwnd, message, wparam, lparam);
		if message == WM_STYLECHANGING
			&& wparam.0 as i32 == GWL_STYLE.0
			&& GetPropW(hwnd, TEMPORARY_DRAG).0.is_null()
		{
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
	use std::cell::Cell;
	use windows::Win32::UI::WindowsAndMessaging::{
		CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_MAXIMIZE, WS_OVERLAPPEDWINDOW,
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

	thread_local! {
		static PROBE_SEEN_SYSMENU: Cell<u32> = const { Cell::new(0) };
		static PROBE_OTHER_WINDOW: Cell<usize> = const { Cell::new(0) };
		static PROBE_DISABLE: Cell<bool> = const { Cell::new(false) };
		static PROBE_DISABLED: Cell<bool> = const { Cell::new(false) };
		static PROBE_NESTED: Cell<bool> = const { Cell::new(false) };
		static PROBE_DEPTH: Cell<usize> = const { Cell::new(0) };
	}

	unsafe extern "system" fn probe_subclass(
		hwnd: HWND,
		message: u32,
		wparam: WPARAM,
		lparam: LPARAM,
		_id: usize,
		_data: usize,
	) -> LRESULT {
		unsafe {
			if (message == WM_NCLBUTTONDOWN && wparam.0 == HTCAPTION as usize)
				|| (message == WM_SYSCOMMAND && (wparam.0 & 0xFFF0) == SC_MOVE as usize)
			{
				let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
				if style & WS_SYSMENU.0 as isize != 0 {
					PROBE_SEEN_SYSMENU.set(PROBE_SEEN_SYSMENU.get() + 1);
				}
				if PROBE_OTHER_WINDOW.get() != 0 {
					let other = HWND(PROBE_OTHER_WINDOW.get() as _);
					let style = GetWindowLongPtrW(other, GWL_STYLE);
					SetWindowLongPtrW(other, GWL_STYLE, style | WS_SYSMENU.0 as isize);
				}
				if PROBE_NESTED.replace(false) {
					use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
					let _ = SendMessageW(
						hwnd,
						WM_SYSCOMMAND,
						Some(WPARAM(SC_MOVE as usize)),
						Some(LPARAM(0)),
					);
					PROBE_DEPTH.set(GetPropW(hwnd, TEMPORARY_DRAG).0 as usize);
				}
				if PROBE_DISABLE.replace(false) {
					PROBE_DISABLED.set(set_custom_caption(hwnd, false).is_ok());
					return LRESULT(0);
				}
				if message == WM_SYSCOMMAND {
					let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_MAXIMIZE.0 as isize);
					return LRESULT(0);
				}
			}
			DefSubclassProc(hwnd, message, wparam, lparam)
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
			assert!(SetWindowSubclass(hwnd, Some(probe_subclass), 1, 0).as_bool());
			set_custom_caption(hwnd, true).unwrap();
			assert_eq!(
				GetWindowLongPtrW(hwnd, GWL_STYLE),
				style & !(WS_SYSMENU.0 as isize)
			);
			let initial_menu = GetPropW(hwnd, REQUESTED_MENU);
			PROBE_SEEN_SYSMENU.set(0);

			use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
			let _ = SendMessageW(
				hwnd,
				WM_NCLBUTTONDOWN,
				Some(WPARAM(HTCAPTION as usize)),
				Some(LPARAM(0)),
			);
			assert_eq!(PROBE_SEEN_SYSMENU.get(), 1);
			assert_eq!(GetPropW(hwnd, REQUESTED_MENU), initial_menu);
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
			assert_eq!(PROBE_SEEN_SYSMENU.get(), 2);
			assert_eq!(GetPropW(hwnd, REQUESTED_MENU), initial_menu);
			let expected_moved_style = (style | WS_MAXIMIZE.0 as isize) & !(WS_SYSMENU.0 as isize);
			assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), expected_moved_style);

			set_custom_caption(hwnd, false).unwrap();
			assert_eq!(
				GetWindowLongPtrW(hwnd, GWL_STYLE),
				style | WS_MAXIMIZE.0 as isize
			);

			let _ = RemoveWindowSubclass(hwnd, Some(probe_subclass), 1);
			DestroyWindow(hwnd).unwrap();
		}
	}

	#[test]
	fn drag_guard_is_window_local_and_survives_nested_dispatch() {
		// SAFETY: both hidden windows and the probe are owned by this test thread.
		unsafe {
			let window = || {
				CreateWindowExW(
					WINDOW_EX_STYLE::default(),
					w!("STATIC"),
					w!("reentrant drag test"),
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
				.unwrap()
			};
			let hwnd = window();
			let other = window();
			assert!(SetWindowSubclass(hwnd, Some(probe_subclass), 1, 0).as_bool());
			set_custom_caption(hwnd, true).unwrap();
			set_custom_caption(other, true).unwrap();
			PROBE_OTHER_WINDOW.set(other.0 as usize);
			PROBE_NESTED.set(true);
			use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
			let _ = SendMessageW(
				hwnd,
				WM_SYSCOMMAND,
				Some(WPARAM(SC_MOVE as usize)),
				Some(LPARAM(0)),
			);
			assert_eq!(PROBE_DEPTH.get(), 1);
			assert!(GetPropW(hwnd, TEMPORARY_DRAG).0.is_null());
			assert_eq!(
				GetWindowLongPtrW(other, GWL_STYLE) & WS_SYSMENU.0 as isize,
				0
			);
			assert_eq!(
				GetPropW(other, REQUESTED_MENU).0 as usize,
				WS_SYSMENU.0 as usize | 1
			);
			PROBE_OTHER_WINDOW.set(0);
			DestroyWindow(other).unwrap();
			DestroyWindow(hwnd).unwrap();
		}
	}

	#[test]
	fn disabling_suppression_during_drag_preserves_restored_native_controls() {
		// SAFETY: this hidden window and its probe are owned by this test thread.
		unsafe {
			let hwnd = CreateWindowExW(
				WINDOW_EX_STYLE::default(),
				w!("STATIC"),
				w!("disable during drag test"),
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
			let original = GetWindowLongPtrW(hwnd, GWL_STYLE);
			assert!(SetWindowSubclass(hwnd, Some(probe_subclass), 1, 0).as_bool());
			set_custom_caption(hwnd, true).unwrap();
			PROBE_DISABLE.set(true);
			use windows::Win32::UI::WindowsAndMessaging::SendMessageW;
			let _ = SendMessageW(
				hwnd,
				WM_SYSCOMMAND,
				Some(WPARAM(SC_MOVE as usize)),
				Some(LPARAM(0)),
			);
			assert!(PROBE_DISABLED.get());
			assert!(GetPropW(hwnd, REQUESTED_MENU).0.is_null());
			assert!(GetPropW(hwnd, TEMPORARY_DRAG).0.is_null());
			assert_eq!(GetWindowLongPtrW(hwnd, GWL_STYLE), original);
			DestroyWindow(hwnd).unwrap();
		}
	}
}
