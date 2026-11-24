//! One user-initiated paste job; clipboard contents never enter logs or disk caches.
use discord_api::upload::Source;
use eframe::egui;
use image::ImageEncoder;
use model::Id;
use std::sync::mpsc;

pub enum Content {
	File(Vec<Source>),
	Text(String),
}

pub struct Paste {
	pub generation: u64,
	pub channel: Id,
	pub target: egui::Id,
	result: mpsc::Receiver<Result<Content, &'static str>>,
}

impl Paste {
	pub fn start(
		generation: u64,
		channel: Id,
		request: ui::AttachmentPaste,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
	) -> Self {
		let target = request.target;
		let (send, result) = mpsc::sync_channel(1);
		let context = context.clone();
		runtime.spawn(async move {
			let result = match tokio::task::spawn_blocking(move || read(request)).await {
				Ok(Ok(Read::Paths(paths))) => {
					let mut sources = Vec::with_capacity(paths.len());
					let mut total = 0;
					let mut error = None;
					for path in paths {
						match Source::inspect(path).await {
							Ok(source) => {
								total += source.size();
								if total > discord_api::upload::MAX_TOTAL_BYTES {
									error = Some(
										"Attachments must total at most 500 MB; account limits may be lower",
									);
									break;
								}
								sources.push(source);
							}
							Err(failure) => {
								error = Some(failure);
								break;
							}
						}
					}
					match error {
						Some(error) => Err(error),
						None => Ok(Content::File(sources)),
					}
				}
				Ok(Ok(Read::Content(content))) => Ok(content),
				Ok(Err(error)) => Err(error),
				Err(_) => Err("Clipboard reading interrupted; paste again"),
			};
			let _ = send.send(result);
			context.request_repaint();
		});
		Self {
			generation,
			channel,
			target,
			result,
		}
	}

	pub fn poll(&self) -> Option<Result<Content, &'static str>> {
		match self.result.try_recv() {
			Ok(result) => Some(result),
			Err(mpsc::TryRecvError::Empty) => None,
			Err(mpsc::TryRecvError::Disconnected) => Some(Err("Clipboard reading interrupted")),
		}
	}
}

enum Read {
	Paths(Vec<std::path::PathBuf>),
	Content(Content),
}

fn read(request: ui::AttachmentPaste) -> Result<Read, &'static str> {
	let ui::AttachmentPaste {
		target: _,
		text,
		image,
	} = request;
	// A copied file also offers its path as text, so the file list outranks delivered
	// text; the image re-read comes after it, see `resolve`.
	let mut clipboard = match arboard::Clipboard::new() {
		Ok(clipboard) => clipboard,
		Err(_) => {
			// egui already transferred text/image clipboard data through the focused
			// Wayland surface. Do not make that data depend on arboard's headless
			// data-control backend, which may be unavailable in a sandbox.
			return supplied_content(text, image).ok_or("Clipboard unavailable")?;
		}
	};
	match clipboard.get().file_list() {
		Ok(paths) if !paths.is_empty() => {
			if paths.len() > discord_api::upload::MAX_FILES {
				return Err("Attach up to 10 files per message");
			}
			let paths: Vec<_> = paths.into_iter().map(normalize).collect();
			if paths
				.iter()
				.any(|path| !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > 4096)
			{
				return Err("Paste a local file with a supported path");
			}
			return Ok(Read::Paths(paths));
		}
		Err(arboard::Error::ContentNotAvailable) | Ok(_) => {}
		Err(_) => {
			return supplied_content(text, image)
				.unwrap_or(Err("Could not read copied files; paste again"));
		}
	}
	resolve(text, image, || {
		if let Ok(image) = clipboard.get_image() {
			return Ok(Some(png(&image.bytes, image.width, image.height)));
		}
		Ok(clipboard.get_text().ok().map(supplied_text))
	})
}

/// Resolves a paste from what the triggering key delivered and one read of the system
/// clipboard. Delivered text wins over a system image: the key read the selection through
/// the focused surface, the freshest view, while arboard re-reads through its own backend.
/// On Wayland without data-control that is the XWayland bridge, which keeps serving an
/// image copied in this client after newer text took the Wayland selection; reading the
/// image first pasted that stale image instead of a link copied afterwards.
fn resolve(
	text: Option<String>,
	image: Option<std::sync::Arc<egui::ColorImage>>,
	system: impl FnOnce() -> Result<Option<Result<Read, &'static str>>, &'static str>,
) -> Result<Read, &'static str> {
	if let Some(content) = supplied_content(text, image) {
		return content;
	}
	system()?.unwrap_or(Err("Copy a file, image, or text before pasting"))
}

/// The key delivers an image beside text only when the text is a lone file path (a
/// screenshot tool's saved path, vendored egui patch 0030), so the image is the paste.
fn supplied_content(
	text: Option<String>,
	image: Option<std::sync::Arc<egui::ColorImage>>,
) -> Option<Result<Read, &'static str>> {
	if let Some(image) = image {
		return Some(supplied_image(image));
	}
	text.map(supplied_text)
}

fn supplied_image(image: std::sync::Arc<egui::ColorImage>) -> Result<Read, &'static str> {
	let pixels = image
		.width()
		.checked_mul(image.height())
		.ok_or("Image is too large")?;
	if pixels == 0 || pixels > ui::MAX_PASTE_PIXELS || pixels != image.pixels().len() {
		return Err(ui::PASTE_PIXELS_EXCEEDED);
	}
	let bytes: Vec<u8> = image
		.pixels()
		.iter()
		.flat_map(|pixel| pixel.to_srgba_unmultiplied())
		.collect();
	png(&bytes, image.width(), image.height())
}

fn supplied_text(text: String) -> Result<Read, &'static str> {
	if text.len() > client_core::MAX_DRAFT_BYTES {
		return Err("Pasted text exceeds the draft limit");
	}
	Ok(Read::Content(Content::Text(text.replace("\r\n", "\n"))))
}

/// Linux file managers publish `text/uri-list` with CRLF line endings and may name a
/// `localhost` authority; arboard leaves both in the path, which then fails as a filename.
#[cfg(unix)]
fn normalize(path: std::path::PathBuf) -> std::path::PathBuf {
	use std::os::unix::ffi::OsStrExt;
	let bytes = path.as_os_str().as_bytes();
	let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
	let bytes = match bytes.strip_prefix(b"localhost/") {
		Some(_) => &bytes[b"localhost".len()..],
		None => bytes,
	};
	std::path::PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn normalize(path: std::path::PathBuf) -> std::path::PathBuf {
	path
}

fn png(bytes: &[u8], width: usize, height: usize) -> Result<Read, &'static str> {
	let pixels = width.checked_mul(height).ok_or("Image is too large")?;
	if pixels == 0 || pixels > ui::MAX_PASTE_PIXELS || bytes.len() != pixels * 4 {
		return Err(ui::PASTE_PIXELS_EXCEEDED);
	}
	let mut encoded = Vec::new();
	image::codecs::png::PngEncoder::new(&mut encoded)
		.write_image(
			bytes,
			width as u32,
			height as u32,
			image::ExtendedColorType::Rgba8,
		)
		.map_err(|_| "Could not prepare pasted image")?;
	Source::pasted_png(encoded).map(|source| Read::Content(Content::File(vec![source])))
}

#[cfg(test)]
mod tests {
	#[test]
	#[cfg(unix)]
	fn uri_list_paths_lose_line_endings_and_localhost() {
		use std::path::PathBuf;
		let normalize = |s: &str| super::normalize(PathBuf::from(s));
		assert_eq!(normalize("/home/a/b.png\r"), PathBuf::from("/home/a/b.png"));
		assert_eq!(normalize("localhost/tmp/x"), PathBuf::from("/tmp/x"));
		assert_eq!(normalize("/plain"), PathBuf::from("/plain"));
		assert_eq!(normalize("localhost"), PathBuf::from("localhost"));
	}

	#[test]
	#[ignore = "Explicit native check: replaces the system clipboard with a temp file"]
	fn native_copied_file_reads_as_paste_paths() {
		let dir = std::env::temp_dir().join(format!("serein-paste-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let file = dir.join("copied file.txt");
		std::fs::write(&file, b"hello").unwrap();
		let mut clipboard = arboard::Clipboard::new().unwrap();
		clipboard.set().file_list(&[&file]).unwrap();
		let request = ui::AttachmentPaste {
			target: eframe::egui::Id::unique("paste"),
			text: None,
			image: None,
		};
		let super::Read::Paths(paths) = super::read(request).unwrap() else {
			panic!("Expected copied file paths")
		};
		assert_eq!(paths, vec![file.canonicalize().unwrap()]);
		let _ = std::fs::remove_dir_all(dir);
	}

	#[test]
	fn pasted_images_are_bounded_png_upload_sources() {
		let super::Read::Content(super::Content::File(source)) =
			super::png(&[255, 0, 0, 255], 1, 1).unwrap()
		else {
			panic!("Expected image source")
		};
		assert_eq!(source[0].filename(), "pasted-image.png");
		assert!(source[0].size() > 0 && source[0].size() <= discord_api::upload::MAX_BYTES);
		assert!(super::png(&[], usize::MAX, 2).is_err());
		assert!(super::png(&[], 4096, 4096).is_err());
		assert!(super::png(&[], 1, 1).is_err());
		assert!(super::png(&[], 0, 0).is_err());
	}

	/// Regression: a link copied after an image was copied in this client pasted the image.
	/// On Wayland without data-control, arboard's XWayland-bridge view kept serving the
	/// older image while the key delivered the newer text through the focused surface.
	#[test]
	fn delivered_text_outranks_a_stale_system_image() {
		let stale_image = || Ok(Some(super::png(&[255, 0, 0, 255], 1, 1)));
		let super::Read::Content(super::Content::Text(text)) =
			super::resolve(Some("https://example.com/link".into()), None, stale_image).unwrap()
		else {
			panic!("Expected the delivered link")
		};
		assert_eq!(text, "https://example.com/link");

		// A screenshot path delivered beside its image pastes the image, not the path.
		let shot = std::sync::Arc::new(eframe::egui::ColorImage::filled(
			[1, 1],
			eframe::egui::Color32::RED,
		));
		let super::Read::Content(super::Content::File(files)) =
			super::resolve(Some("/tmp/shot.png".into()), Some(shot), stale_image).unwrap()
		else {
			panic!("Expected the delivered screenshot")
		};
		assert_eq!(files[0].filename(), "pasted-image.png");

		// Without delivered text the system image is the paste, as before.
		let super::Read::Content(super::Content::File(files)) =
			super::resolve(None, None, stale_image).unwrap()
		else {
			panic!("Expected the system image")
		};
		assert_eq!(files[0].filename(), "pasted-image.png");
	}

	#[test]
	fn nothing_to_paste_is_reported_instead_of_a_stale_image() {
		let empty = || Ok(None);
		assert!(matches!(
			super::resolve(None, None, empty),
			Err("Copy a file, image, or text before pasting")
		));
	}
}
