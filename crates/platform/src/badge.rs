//! Native taskbar/Dock unread indicator; no message text or account identity leaves the app.
pub const fn supported() -> bool {
	cfg!(any(target_os = "windows", target_os = "macos"))
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn set(_window: &winit::window::Window, _count: u32) -> Result<(), &'static str> {
	Err("App icon badges are currently available on Windows and macOS.")
}
#[cfg(target_os = "macos")]
pub fn set(_window: &winit::window::Window, count: u32) -> Result<(), &'static str> {
	use objc2_app_kit::NSApplication;
	use objc2_foundation::{MainThreadMarker, NSString};
	let mtm = MainThreadMarker::new().ok_or("The Dock badge requires the main thread.")?;
	let label = match count {
		0 => None,
		1..=99 => Some(NSString::from_str(&count.to_string())),
		_ => Some(NSString::from_str("99+")),
	};
	NSApplication::sharedApplication(mtm)
		.dockTile()
		.setBadgeLabel(label.as_deref());
	Ok(())
}
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub fn set(window: &winit::window::Window, count: u32) -> Result<(), &'static str> {
	use windows::{
		Win32::{
			Foundation::HWND,
			System::Com::{
				CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
				CoUninitialize,
			},
			UI::{
				Shell::{ITaskbarList3, TaskbarList},
				WindowsAndMessaging::{CreateIcon, DestroyIcon, HICON},
			},
		},
		core::w,
	};
	use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
	const ERROR: &str = "Taskbar badge unavailable. Check your Windows taskbar settings.";
	let RawWindowHandle::Win32(handle) = window.window_handle().map_err(|_| ERROR)?.as_raw() else {
		return Err(ERROR);
	};
	// SAFETY: all COM operations and the retained winit handle stay on the UI thread.
	let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
	let result = (|| {
		let taskbar: ITaskbarList3 =
			unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) }
				.map_err(|_| ERROR)?;
		unsafe { taskbar.HrInit() }.map_err(|_| ERROR)?;
		let icon = if count > 0 {
			let (mask, pixels) = count_icon(count);
			// SAFETY: buffers contain exactly a 16x16 1-bit AND mask and 32-bit BGRA pixels.
			unsafe { CreateIcon(None, 16, 16, 1, 32, mask.as_ptr(), pixels.as_ptr()) }
				.map_err(|_| ERROR)?
		} else {
			HICON::default()
		};
		// SAFETY: SetOverlayIcon copies the icon; its owned handle is destroyed immediately after.
		let result = unsafe {
			taskbar.SetOverlayIcon(HWND(handle.hwnd.get() as *mut _), icon, w!("Direct pings"))
		}
		.map_err(|_| ERROR);
		if count > 0 {
			let _ = unsafe { DestroyIcon(icon) };
		}
		result
	})();
	// Balance only successful apartment initialization; an existing different apartment is untouched.
	if initialized {
		unsafe { CoUninitialize() };
	}
	result
}
#[cfg(any(target_os = "windows", test))]
fn count_icon(count: u32) -> ([u8; 32], [u8; 1024]) {
	const DIGITS: [[u8; 7]; 10] = [
		[14, 17, 19, 21, 25, 17, 14],
		[4, 12, 4, 4, 4, 4, 14],
		[14, 17, 1, 2, 4, 8, 31],
		[30, 1, 1, 14, 1, 1, 30],
		[2, 6, 10, 18, 31, 2, 2],
		[31, 16, 16, 30, 1, 1, 30],
		[14, 16, 16, 30, 17, 17, 14],
		[31, 1, 2, 4, 8, 8, 8],
		[14, 17, 17, 14, 17, 17, 14],
		[14, 17, 17, 15, 1, 1, 14],
	];
	let mut mask = [255; 32];
	let mut pixels = [0; 1024];
	let mut paint = |x: usize, y: usize, color: [u8; 4]| {
		mask[y * 2 + x / 8] &= !(1 << (7 - x % 8));
		pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4].copy_from_slice(&color);
	};
	// Sample around the actual 16-pixel center; the previous offset clipped the right edge.
	// HICON expects premultiplied BGRA for partially transparent edge pixels.
	for y in 0..16 {
		for x in 0..16 {
			let coverage = (0..4)
				.flat_map(|sy| (0..4).map(move |sx| (sx, sy)))
				.filter(|(sx, sy)| {
					let dx = (2 * (x * 4 + sx) + 1) as i32 - 64;
					let dy = (2 * (y * 4 + sy) + 1) as i32 - 64;
					dx * dx + dy * dy <= 58 * 58
				})
				.count();
			if coverage > 0 {
				let alpha = (coverage * 255 / 16) as u8;
				paint(
					x,
					y,
					[
						(67 * u32::from(alpha) / 255) as u8,
						(63 * u32::from(alpha) / 255) as u8,
						(242 * u32::from(alpha) / 255) as u8,
						alpha,
					],
				);
			}
		}
	}
	let mut digit = |value: usize, start_x: usize| {
		for (y, row) in DIGITS[value].iter().enumerate() {
			for x in 0..5 {
				if row & (1 << (4 - x)) != 0 {
					paint(start_x + x, y + 4, [255, 255, 255, 255]);
				}
			}
		}
	};
	if count < 10 {
		digit(count as usize, 5);
	} else if count < 100 {
		digit((count / 10) as usize, 2);
		digit((count % 10) as usize, 9);
	} else {
		// Three 3x5 glyphs fit inside the same circle for 99+.
		for start_x in [2, 6] {
			for (y, row) in [7u8, 5, 7, 1, 7].iter().enumerate() {
				for x in 0..3 {
					if row & (1 << (2 - x)) != 0 {
						paint(start_x + x, y + 5, [255, 255, 255, 255]);
					}
				}
			}
		}
		for (y, row) in [2u8, 2, 7, 2, 2].iter().enumerate() {
			for x in 0..3 {
				if row & (1 << (2 - x)) != 0 {
					paint(10 + x, y + 5, [255, 255, 255, 255]);
				}
			}
		}
	}
	(mask, pixels)
}
#[cfg(test)]
mod tests {
	#[test]
	fn badge_has_transparent_corners_and_round_red_background() {
		let (mask, pixels) = super::count_icon(4);
		assert_eq!(pixels[..4], [0; 4]);
		assert_eq!(
			pixels[(2 * 16 + 7) * 4..(2 * 16 + 7) * 4 + 4],
			[67, 63, 242, 255]
		);
		assert_ne!(mask[0] & 128, 0);
		assert_eq!(pixels[(8 * 16 + 7) * 4..(8 * 16 + 7) * 4 + 4], [255; 4]);
	}
}
