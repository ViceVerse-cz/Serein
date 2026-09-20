/// Cursor position in physical pixels, in desktop coordinates. `None` where the platform has
/// no global cursor query (Wayland) or it is not implemented yet.
#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
pub fn cursor_position() -> Option<(f64, f64)> {
	use windows::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
	let mut cursor = POINT::default();
	// SAFETY: stack POINT; GetCursorPos fails while the input desktop is locked.
	unsafe {
		GetCursorPos(&mut cursor).ok()?;
	}
	Some((f64::from(cursor.x), f64::from(cursor.y)))
}

#[cfg(not(target_os = "windows"))]
pub fn cursor_position() -> Option<(f64, f64)> {
	None
}
