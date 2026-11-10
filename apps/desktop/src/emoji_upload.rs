//! Bounded local emoji preparation. Only the settings confirmation sends an upload.
use eframe::egui;
use image::{AnimationDecoder, ImageDecoder};
use model::Id;
use std::{
	io::{Cursor, Read},
	path::{Path, PathBuf},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
};

type Scope = (u64, Id, u64);
type Prepared = (String, String, bool, egui::ColorImage);
type Selected = Result<Vec<Prepared>, &'static str>;
struct Choosing {
	scope: Scope,
	result: mpsc::Receiver<Selected>,
	cancelled: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct EmojiUpload {
	choosing: Option<Choosing>,
}
impl EmojiUpload {
	/// Prepares up to 10 emoji images in the background, opening a file picker when no paths are given.
	/// Fails immediately if a previous selection is still pending or too many paths are supplied.
	pub fn start(
		&mut self,
		scope: Scope,
		paths: Vec<PathBuf>,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
		parent: Arc<winit::window::Window>,
	) -> Result<(), &'static str> {
		if self.choosing.is_some() {
			return Err("Close the previous emoji picker first");
		}
		if paths.len() > 10 {
			return Err("Choose at most 10 emoji at a time");
		}
		let dialog = paths
			.is_empty()
			.then(|| platform::save::emoji_sources(parent));
		let (send, result) = mpsc::sync_channel(1);
		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let context = context.clone();
		runtime.spawn(async move {
			let paths = match dialog {
				Some(dialog) => dialog.await.unwrap_or_default(),
				None => paths,
			};
			let decode_flag = flag.clone();
			let result = tokio::task::spawn_blocking(move || {
				if paths.len() > 10 {
					return Err("Choose at most 10 emoji at a time");
				}
				let mut prepared = Vec::new();
				for path in paths {
					if decode_flag.load(Ordering::Acquire) {
						return Ok(Vec::new());
					}
					prepared.push(read(&path, &decode_flag)?);
				}
				Ok(prepared)
			})
			.await
			.unwrap_or(Err(
				"Emoji preparation interrupted; choose the images again",
			));
			let _ = send.send(if flag.load(Ordering::Acquire) {
				Ok(Vec::new())
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
	/// Downloads another server's custom emoji from Discord's CDN and prepares it like a
	/// chosen file, keeping its name.
	pub fn start_clone(
		&mut self,
		scope: Scope,
		emoji: ui::EmojiClone,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
	) -> Result<(), &'static str> {
		if self.choosing.is_some() {
			return Err("Wait for the current emoji images to finish preparing");
		}
		let (send, result) = mpsc::sync_channel(1);
		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let context = context.clone();
		runtime.spawn(async move {
			let result = match download(&emoji).await {
				Ok(bytes) => {
					let decode_flag = flag.clone();
					tokio::task::spawn_blocking(move || {
						prepare(bytes, &emoji.name, &decode_flag).map(|prepared| vec![prepared])
					})
					.await
					.unwrap_or(Err("Emoji preparation interrupted; try again"))
				}
				Err(error) => Err(error),
			};
			let _ = send.send(if flag.load(Ordering::Acquire) {
				Ok(Vec::new())
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
				Err("Emoji preparation interrupted; choose the images again")
			}
		};
		let job = self.choosing.take()?;
		(!job.cancelled.load(Ordering::Acquire)).then_some((job.scope, result))
	}
}
impl Drop for EmojiUpload {
	fn drop(&mut self) {
		self.cancel();
	}
}

/// Largest emoji image accepted from the CDN; Discord's own emoji limit is 256 KB.
const MAX_CLONE_BYTES: usize = 256 * 1024;

/// Downloads a message emoji's image from Discord's CDN, refusing redirects and anything over [`MAX_CLONE_BYTES`].
async fn download(emoji: &ui::EmojiClone) -> Result<Vec<u8>, &'static str> {
	let url = format!(
		"https://cdn.discordapp.com/emojis/{}.{}?size=128&quality=lossless",
		emoji.id,
		if emoji.animated { "gif" } else { "png" }
	);
	let client = reqwest::Client::builder()
		.no_proxy()
		.redirect(reqwest::redirect::Policy::none())
		.timeout(std::time::Duration::from_secs(20))
		.build()
		.map_err(|_| "Emoji download unavailable")?;
	let mut response = client
		.get(url)
		.send()
		.await
		.map_err(|_| "Could not download the emoji")?;
	if !response.status().is_success()
		|| response
			.content_length()
			.is_some_and(|size| size > MAX_CLONE_BYTES as u64)
	{
		return Err("Could not download the emoji");
	}
	let mut bytes = Vec::new();
	while let Some(chunk) = response
		.chunk()
		.await
		.map_err(|_| "Could not download the emoji")?
	{
		if bytes.len() + chunk.len() > MAX_CLONE_BYTES {
			return Err("The emoji image is larger than Discord allows");
		}
		bytes.extend_from_slice(&chunk);
	}
	Ok(bytes)
}

/// Reads a local emoji image, rejecting relative or overlong paths, symlinks, and files over 8 MB.
/// Uses the file stem as the emoji name and returns a user-facing message on any failure.
fn read(path: &Path, cancelled: &AtomicBool) -> Result<Prepared, &'static str> {
	const MAX_INPUT: u64 = 8 * 1024 * 1024;
	if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > 4096 {
		return Err("Choose a local image with a supported path");
	}
	let metadata =
		std::fs::symlink_metadata(path).map_err(|_| "Could not open the chosen emoji")?;
	if !metadata.is_file()
		|| metadata.file_type().is_symlink()
		|| metadata.len() == 0
		|| metadata.len() > MAX_INPUT
	{
		return Err("Choose regular image files up to 8 MB");
	}
	let mut bytes = Vec::with_capacity(metadata.len() as usize);
	std::fs::File::open(path)
		.map_err(|_| "Could not open the chosen emoji")?
		.take(MAX_INPUT + 1)
		.read_to_end(&mut bytes)
		.map_err(|_| "Could not read the chosen emoji")?;
	if bytes.len() as u64 > MAX_INPUT {
		return Err("Choose images up to 8 MB");
	}
	let name = path
		.file_stem()
		.and_then(|name| name.to_str())
		.unwrap_or("emoji")
		.to_owned();
	prepare(bytes, &name, cancelled)
}

/// Checks and encodes emoji image bytes; `name` is reduced to Discord's emoji name rules.
fn prepare(bytes: Vec<u8>, name: &str, cancelled: &AtomicBool) -> Result<Prepared, &'static str> {
	let gif = bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a");
	let (uri, preview) = if gif {
		if bytes.len() > 256 * 1024 {
			return Err("Animated emoji must be at most 256 KB");
		}
		let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(&bytes))
			.map_err(|_| "Invalid GIF image")?;
		let mut limits = image::Limits::default();
		limits.max_image_width = Some(4096);
		limits.max_image_height = Some(4096);
		limits.max_alloc = Some(64 * 1024 * 1024);
		decoder
			.set_limits(limits)
			.map_err(|_| "GIF image is too large")?;
		let mut preview = None;
		let mut total = 0usize;
		for (index, frame) in decoder.into_frames().enumerate() {
			if cancelled.load(Ordering::Acquire) {
				return Err("Emoji preparation cancelled");
			}
			if index >= 120 {
				return Err("Choose a GIF with at most 120 frames");
			}
			let frame = frame.map_err(|_| "Invalid GIF animation")?.into_buffer();
			total = total.saturating_add(frame.as_raw().len());
			if total > 64 * 1024 * 1024 {
				return Err("Choose a smaller GIF animation");
			}
			if preview.is_none() {
				let pixels = image::imageops::thumbnail(&frame, 128, 128);
				preview = Some(egui::ColorImage::from_rgba_unmultiplied(
					[pixels.width() as usize, pixels.height() as usize],
					pixels.as_raw(),
				));
			}
		}
		(
			discord_protocol::server_admin::emoji_data_uri(&bytes, true)
				.ok_or("Invalid emoji image")?,
			preview.ok_or("GIF has no frames")?,
		)
	} else {
		crate::group_icon::decode_image(&bytes, 128, false)?
	};
	let mut name: String = name
		.chars()
		.filter(|c| c.is_ascii_alphanumeric() || *c == '_')
		.take(32)
		.collect();
	if name.len() < 2 {
		name = "emoji".into();
	}
	Ok((name, uri, gif, preview))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Checks that downloaded emoji bytes become a PNG data URI under the emoji's own name,
	/// and that names are reduced to Discord's emoji name rules.
	#[test]
	fn cloned_emoji_bytes_prepare_like_chosen_files() {
		let mut png = Cursor::new(Vec::new());
		image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
			32,
			32,
			image::Rgba([200, 40, 60, 255]),
		))
		.write_to(&mut png, image::ImageFormat::Png)
		.unwrap();
		let cancelled = AtomicBool::new(false);
		let (name, uri, gif, preview) =
			prepare(png.into_inner(), "party_blob", &cancelled).unwrap();
		assert_eq!(name, "party_blob");
		assert!(uri.starts_with("data:image/png;base64,") && !gif);
		assert_eq!(preview.resolution(), [32, 32]);
		let (name, ..) = prepare(
			{
				let mut png = Cursor::new(Vec::new());
				image::DynamicImage::ImageRgba8(image::RgbaImage::new(8, 8))
					.write_to(&mut png, image::ImageFormat::Png)
					.unwrap();
				png.into_inner()
			},
			"a-b",
			&cancelled,
		)
		.unwrap();
		assert_eq!(name, "ab");
		assert!(prepare(b"not an image".to_vec(), "x", &cancelled).is_err());
	}
}
