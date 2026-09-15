//! Native destination selection; never interprets an attachment name as a path.
use std::{path::PathBuf, sync::Arc};

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
