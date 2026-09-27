//! Native destination selection; never interprets an attachment name as a path.
use std::{path::PathBuf, sync::Arc};

pub fn font_source(
	parent: Arc<winit::window::Window>,
) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title("Import interface font")
		.add_filter("TrueType and OpenType fonts", &["ttf", "otf"])
		.pick_file();
	async move {
		let file = dialog.await?;
		drop(parent);
		Some(file.path().to_owned())
	}
}

/// Explicit local extension import; selection grants no plugin capabilities.
pub fn extension_source(
	parent: Arc<winit::window::Window>,
) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title("Import Serein extension")
		.add_filter("Serein extensions", &["serein-extension", "json"])
		.pick_file();
	async move {
		let file = dialog.await?;
		drop(parent);
		Some(file.path().to_owned())
	}
}

pub fn icon_source(
	parent: Arc<winit::window::Window>,
	title: &'static str,
) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title(title)
		.add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
		.pick_file();
	async move {
		let file = dialog.await?;
		drop(parent);
		Some(file.path().to_owned())
	}
}

pub fn theme_background_source(
	parent: Arc<winit::window::Window>,
) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title("Choose theme background")
		.add_filter("Static images", &["png", "jpg", "jpeg"])
		.pick_file();
	async move {
		let file = dialog.await?;
		drop(parent);
		Some(file.path().to_owned())
	}
}

pub fn theme_cover_source(
	parent: Arc<winit::window::Window>,
) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title("Choose theme card cover")
		.add_filter("Static images", &["png", "jpg", "jpeg"])
		.pick_file();
	async move {
		let file = dialog.await?;
		drop(parent);
		Some(file.path().to_owned())
	}
}

pub fn theme_destination(
	parent: Arc<winit::window::Window>,
	filename: &str,
) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title("Export Serein theme")
		.set_file_name(safe_filename(filename))
		.add_filter("Serein theme", &["serein-extension"])
		.save_file();
	async move {
		let file = dialog.await?;
		drop(parent);
		Some(file.path().to_owned())
	}
}

pub fn emoji_sources(
	parent: Arc<winit::window::Window>,
) -> impl std::future::Future<Output = Option<Vec<PathBuf>>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title("Choose emoji images")
		.add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
		.pick_files();
	async move {
		let files = dialog.await?;
		drop(parent);
		// Keep one excess entry so the caller can report the selection limit.
		Some(
			files
				.into_iter()
				.take(11)
				.map(|file| file.path().to_owned())
				.collect(),
		)
	}
}

pub fn attachment_source(
	parent: Arc<winit::window::Window>,
) -> impl std::future::Future<Output = Option<Vec<PathBuf>>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title("Choose attachments")
		.pick_files();
	async move {
		let file = dialog.await?;
		drop(parent);
		Some(
			file.into_iter()
				.map(|file| file.path().to_owned())
				.collect(),
		)
	}
}

pub fn attachment_destination(
	parent: Arc<winit::window::Window>,
	filename: &str,
) -> impl std::future::Future<Output = Option<PathBuf>> + Send + 'static {
	let dialog = rfd::AsyncFileDialog::new()
		.set_parent(parent.as_ref())
		.set_title("Save attachment")
		.set_file_name(safe_filename(filename))
		.save_file();
	async move {
		let file = dialog.await?;
		drop(parent);
		Some(file.path().to_owned())
	}
}

/// Publish a completed sibling file without replacing a concurrently created destination.
/// Native exclusive rename also works on volumes that do not support hard links.
#[allow(unsafe_code)]
pub fn publish_new(source: &std::path::Path, destination: &std::path::Path) -> std::io::Result<()> {
	#[cfg(any(target_os = "macos", target_os = "linux"))]
	{
		use std::{ffi::CString, os::unix::ffi::OsStrExt};
		let source = CString::new(source.as_os_str().as_bytes())?;
		let destination = CString::new(destination.as_os_str().as_bytes())?;
		#[cfg(target_os = "macos")]
		unsafe extern "C" {
			fn renamex_np(
				from: *const std::ffi::c_char,
				to: *const std::ffi::c_char,
				flags: u32,
			) -> i32;
		}
		#[cfg(target_os = "linux")]
		unsafe extern "C" {
			fn renameat2(
				from_fd: i32,
				from: *const std::ffi::c_char,
				to_fd: i32,
				to: *const std::ffi::c_char,
				flags: u32,
			) -> i32;
		}
		// SAFETY: both pointers are valid NUL-terminated paths for the call.
		#[cfg(target_os = "macos")]
		let result = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), 4) }; // RENAME_EXCL
		#[cfg(target_os = "linux")]
		let result = unsafe { renameat2(-100, source.as_ptr(), -100, destination.as_ptr(), 1) }; // AT_FDCWD, RENAME_NOREPLACE
		if result == 0 {
			Ok(())
		} else {
			Err(std::io::Error::last_os_error())
		}
	}
	#[cfg(target_os = "windows")]
	{
		use std::os::windows::ffi::OsStrExt;
		let path = |path: &std::path::Path| -> std::io::Result<Vec<u16>> {
			let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
			if value.contains(&0) {
				return Err(std::io::ErrorKind::InvalidInput.into());
			}
			value.push(0);
			Ok(value)
		};
		let source = path(source)?;
		let destination = path(destination)?;
		#[link(name = "kernel32")]
		unsafe extern "system" {
			fn MoveFileW(from: *const u16, to: *const u16) -> i32;
		}
		// SAFETY: both paths are valid NUL-terminated UTF-16 buffers. MoveFileW refuses replacement.
		if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } != 0 {
			Ok(())
		} else {
			Err(std::io::Error::last_os_error())
		}
	}
	#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
	{
		std::fs::hard_link(source, destination)
	}
}

pub fn safe_filename(filename: &str) -> String {
	let name: String = filename
		.chars()
		.filter(|c| {
			!c.is_control() && !matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
		})
		.take(120)
		.collect();
	let name = name.trim_matches(['.', ' ']);
	let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
	if name.is_empty() {
		"attachment".into()
	} else if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
		|| stem
			.strip_prefix("COM")
			.or_else(|| stem.strip_prefix("LPT"))
			.is_some_and(|suffix| {
				matches!(suffix.as_bytes(), [b'0'..=b'9']) || matches!(suffix, "¹" | "²" | "³")
			}) {
		format!("_{name}")
	} else {
		name.into()
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn completed_files_publish_without_clobbering() {
		let mut random = [0_u8; 16];
		getrandom::fill(&mut random).unwrap();
		let root = std::env::temp_dir().join(format!("serein-save-{random:02x?}"));
		std::fs::create_dir(&root).unwrap();
		let source = root.join("partial");
		let destination = root.join("attachment");
		std::fs::write(&source, b"complete").unwrap();
		super::publish_new(&source, &destination).unwrap();
		assert_eq!(std::fs::read(&destination).unwrap(), b"complete");
		std::fs::write(&source, b"replacement").unwrap();
		assert!(super::publish_new(&source, &destination).is_err());
		assert_eq!(std::fs::read(&destination).unwrap(), b"complete");
		std::fs::remove_dir_all(root).unwrap();
	}
	#[test]
	fn suggested_names_are_single_safe_components() {
		for name in [
			"../../",
			"C:\\private\\file.png",
			"CON.png",
			"../NUL",
			"\0\n",
			"...",
		] {
			let safe = super::safe_filename(name);
			assert!(!safe.is_empty() && !safe.contains(['/', '\\', ':', '\0', '\n']));
			assert!(!matches!(safe.as_str(), "." | ".." | "CON.png" | "NUL"));
		}
		assert_eq!(super::safe_filename("photo.png"), "photo.png");
		assert_eq!(super::safe_filename("report.pdf"), "report.pdf");
		assert_eq!(super::safe_filename("archive.bin"), "archive.bin");
		assert_eq!(super::safe_filename("..."), "attachment");
		for prefix in ["COM", "LPT", "com", "lpt"] {
			for digit in ["1", "9", "¹", "²", "³"] {
				let name = format!("{prefix}{digit}.png");
				assert_eq!(super::safe_filename(&name), format!("_{name}"));
			}
		}
		for name in ["COM10.png", "LPT12.png", "COM¹photo.png", "COM⁴.png"] {
			assert_eq!(super::safe_filename(name), name);
		}
	}
}
