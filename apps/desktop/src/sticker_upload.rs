//! One bounded local sticker preparation job; selecting a file never uploads it.
use eframe::egui;
use model::Id;
use std::{
	io::Read,
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
};

type Scope = (u64, Id, u64);
type Prepared = (String, String, Vec<u8>, egui::ColorImage);
type Selected = Result<Option<Prepared>, &'static str>;

struct Choosing {
	scope: Scope,
	result: mpsc::Receiver<Selected>,
	cancelled: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct StickerUpload {
	choosing: Option<Choosing>,
}

impl StickerUpload {
	pub fn start(
		&mut self,
		scope: Scope,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
		parent: Arc<winit::window::Window>,
	) -> Result<(), &'static str> {
		if self.choosing.is_some() {
			return Err("Close the previous sticker picker first");
		}
		let dialog = platform::save::icon_source(parent, "Choose sticker artwork");
		let (send, result) = mpsc::sync_channel(1);
		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let context = context.clone();
		runtime.spawn(async move {
			let result = match dialog.await {
				Some(path) if !flag.load(Ordering::Acquire) => {
					let flag = flag.clone();
					tokio::task::spawn_blocking(move || read(path, &flag).map(Some))
						.await
						.unwrap_or(Err("Sticker preparation interrupted; choose it again"))
				}
				_ => Ok(None),
			};
			let _ = send.send(if flag.load(Ordering::Acquire) {
				Ok(None)
			} else {
				result
			});
			context.request_repaint();
		});
		self.choosing = Some(Choosing {
			scope,
			result,
			cancelled,
		});
		Ok(())
	}

	pub fn cancel(&self) {
		if let Some(job) = &self.choosing {
			job.cancelled.store(true, Ordering::Release);
		}
	}

	pub fn poll(
		&mut self,
		generation: u64,
		valid: impl FnOnce(Id) -> bool,
	) -> Option<(Scope, Selected)> {
		let job = self.choosing.as_ref()?;
		if job.scope.0 != generation || !valid(job.scope.1) {
			self.cancel();
		}
		let result = match job.result.try_recv() {
			Ok(result) => result,
			Err(mpsc::TryRecvError::Empty) => return None,
			Err(mpsc::TryRecvError::Disconnected) => {
				Err("Sticker preparation interrupted; choose it again")
			}
		};
		let job = self.choosing.take()?;
		(!job.cancelled.load(Ordering::Acquire)).then_some((job.scope, result))
	}
}

impl Drop for StickerUpload {
	fn drop(&mut self) {
		self.cancel();
	}
}

fn read(path: PathBuf, cancelled: &AtomicBool) -> Result<Prepared, &'static str> {
	const MAX_INPUT: u64 = 8 * 1024 * 1024;
	if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > 4096 {
		return Err("Choose a local image with a supported path");
	}
	let metadata = std::fs::symlink_metadata(&path)
		.map_err(|_| "Could not open the chosen sticker artwork")?;
	if !metadata.is_file()
		|| metadata.file_type().is_symlink()
		|| metadata.len() == 0
		|| metadata.len() > MAX_INPUT
	{
		return Err("Choose a regular PNG, JPEG or WebP image up to 8 MB");
	}
	let mut input = Vec::with_capacity(metadata.len() as usize);
	std::fs::File::open(&path)
		.map_err(|_| "Could not open the chosen sticker artwork")?
		.take(MAX_INPUT + 1)
		.read_to_end(&mut input)
		.map_err(|_| "Could not read the chosen sticker artwork")?;
	if cancelled.load(Ordering::Acquire) {
		return Err("Sticker preparation cancelled");
	}
	let (file, preview) = crate::group_icon::decode_png(&input, 320, true, 512 * 1024)
		.map_err(|_| "Use a static PNG, JPEG or WebP image that can fit within 512 KB")?;
	let mut name: String = path
		.file_stem()
		.and_then(|name| name.to_str())
		.unwrap_or("sticker")
		.chars()
		.filter(|character| !character.is_control())
		.take(30)
		.collect();
	if name.trim().is_empty() {
		name = "sticker".into();
	}
	Ok((name, "sticker.png".into(), file, preview))
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::io::Cursor;

	#[test]
	fn prepared_sticker_is_bounded_square_png() {
		let root = std::env::temp_dir().join("serein-sticker-upload-test.png");
		let mut source = Cursor::new(Vec::new());
		image::DynamicImage::new_rgba8(640, 400)
			.write_to(&mut source, image::ImageFormat::Png)
			.unwrap();
		std::fs::write(&root, source.into_inner()).unwrap();
		let (_, filename, bytes, preview) = read(root.clone(), &AtomicBool::new(false)).unwrap();
		assert_eq!(filename, "sticker.png");
		assert_eq!(preview.size, [320, 320]);
		assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() <= 512 * 1024);
		std::fs::remove_file(root).unwrap();
	}
}
