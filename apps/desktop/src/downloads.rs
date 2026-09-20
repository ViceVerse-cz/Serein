//! Explicit attachment downloads and media clipboard copies. One job; no credentials, URL logs, or automatic saves.
use model::Attachment;
use std::{
	fs::{self, File, OpenOptions},
	io::Write,
	path::{Path, PathBuf},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
	time::{Duration, Instant},
};
use tokio::sync::{Notify, watch};

const MAX_BYTES: u64 = 100 * 1024 * 1024;
const MAX_EMBED_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
	#[default]
	Idle,
	Choosing,
	Downloading {
		received: u64,
		total: u64,
	},
	Saved,
	Copied,
	Cancelled,
	Failed(&'static str),
}
struct Job {
	cancelled: Arc<AtomicBool>,
	wake_cancel: Arc<Notify>,
	status: watch::Receiver<Status>,
	done: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct Downloads {
	job: Option<Job>,
	status: Status,
	clipboard_owner: Option<std::sync::mpsc::SyncSender<()>>,
	clipboard_thread: Option<std::thread::JoinHandle<()>>,
	clipboard_done: Arc<AtomicBool>,
	copy_cleanup_failed: Arc<AtomicBool>,
}
impl Downloads {
	pub fn start(
		&mut self,
		attachment: Attachment,
		runtime: &tokio::runtime::Handle,
		context: &eframe::egui::Context,
		parent: Arc<winit::window::Window>,
	) -> Result<(), &'static str> {
		let url = original_url(&attachment)
			.ok_or("Attachment download unavailable")
			.inspect_err(|&error| self.status = Status::Failed(error))?;
		self.start_job(attachment, url, runtime, context, Some(parent))
	}
	pub fn start_copy(
		&mut self,
		attachment: Attachment,
		runtime: &tokio::runtime::Handle,
		context: &eframe::egui::Context,
	) -> Result<(), &'static str> {
		if !attachment.is_image() && !attachment.is_video() {
			self.status = Status::Failed("Copy supports images and videos");
			return Err("Copy supports images and videos");
		}
		let url = original_url(&attachment)
			.ok_or("Attachment download unavailable")
			.inspect_err(|&error| self.status = Status::Failed(error))?;
		self.start_job(attachment, url, runtime, context, None)
	}
	pub fn start_embed(
		&mut self,
		media: model::EmbedMedia,
		copy: bool,
		runtime: &tokio::runtime::Handle,
		context: &eframe::egui::Context,
		parent: Arc<winit::window::Window>,
	) -> Result<(), &'static str> {
		let url = [media.proxy_url.as_deref(), media.url.as_deref()]
			.into_iter()
			.flatten()
			.find_map(|source| {
				crate::avatars::embed_url(source, ui::LARGE_EDGE)
					.and_then(|url| url::Url::parse(&url).ok())
			})
			.ok_or("Embedded image download unavailable")
			.inspect_err(|&error| self.status = Status::Failed(error))?;
		// Private transfer metadata: zero means a bounded PNG length from the proxy.
		// Public attachment entry points still require valid nonzero original metadata.
		let image = Attachment {
			id: model::Id(0),
			filename: "image.png".into(),
			description: None,
			content_type: Some("image/png".into()),
			size: 0,
			media: model::EmbedMedia::default(),
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		};
		self.start_job(image, url, runtime, context, (!copy).then_some(parent))
	}
	fn start_job(
		&mut self,
		attachment: Attachment,
		url: url::Url,
		runtime: &tokio::runtime::Handle,
		context: &eframe::egui::Context,
		parent: Option<Arc<winit::window::Window>>,
	) -> Result<(), &'static str> {
		self.poll();
		if self.is_active() {
			return Err("A download or clipboard cleanup is already active");
		}
		// Construct on the native UI thread; Cocoa reads its application/window here.
		let copying = parent.is_none();
		let dialog = parent
			.map(|parent| platform::save::attachment_destination(parent, &attachment.filename));
		let (previous, owner) = if copying {
			// ponytail: one retained copy; stage two only if failed-copy clipboard continuity is needed.
			self.clipboard_owner.take();
			let previous = self.clipboard_thread.take();
			let (send, receive) = std::sync::mpsc::sync_channel(0);
			self.clipboard_owner = Some(send);
			(previous, Some(receive))
		} else {
			(None, None)
		};
		let clipboard_done = copying.then(|| {
			let done = Arc::new(AtomicBool::new(false));
			self.clipboard_done = done.clone();
			done
		});
		let copy_cleanup_failed = self.copy_cleanup_failed.clone();
		let cancelled = Arc::new(AtomicBool::new(false));
		let wake_cancel = Arc::new(Notify::new());
		let done = Arc::new(AtomicBool::new(false));
		let initial = if copying {
			Status::Downloading {
				received: 0,
				total: attachment.size,
			}
		} else {
			Status::Choosing
		};
		let (send, receive) = watch::channel(initial.clone());
		self.status = initial;
		self.job = Some(Job {
			cancelled: cancelled.clone(),
			wake_cancel: wake_cancel.clone(),
			status: receive,
			done: done.clone(),
		});
		let runtime = runtime.clone();
		let context = context.clone();
		// One dedicated owner keeps all file work and cleanup away from the render/runtime threads.
		let spawn = std::thread::Builder::new()
			.name("serein-download".into())
			.spawn(move || {
				if let Some(previous) = previous {
					let _ = previous.join();
				}
				let mut copied = None;
				let mut clipboard = None;
				let publish = |status| {
					send.send_replace(status);
					context.request_repaint();
				};
				let result = runtime.block_on(async {
					// rfd dispatches native work to Cocoa/Windows/the desktop portal. Keep the
					// single slot until the dialog closes even if cancelled: no accumulating dialogs.
					let path = if let Some(dialog) = dialog {
						dialog.await
					} else {
						copied = Some(CopyFile::create(
							&attachment.filename,
							copy_cleanup_failed.clone(),
						)?);
						copied.as_ref().map(|file| file.path.clone())
					};
					if cancelled.load(Ordering::Acquire) {
						return Err("Cancelled");
					}
					let Some(path) = path else {
						return Err("Cancelled");
					};
					let client = reqwest::Client::builder()
						.no_proxy()
						.redirect(reqwest::redirect::Policy::none())
						.connect_timeout(Duration::from_secs(15))
						.read_timeout(Duration::from_secs(30))
						.timeout(Duration::from_secs(300))
						.build()
						.map_err(|_| "Download unavailable")?;
					download(
						&client,
						url,
						&path,
						attachment.size,
						!copying,
						&cancelled,
						&wake_cancel,
						&publish,
					)
					.await?;
					if copying {
						clipboard = Some(copy_media(&path, attachment.is_image(), &cancelled)?);
						if attachment.is_image() {
							copied.take();
						}
					}
					Ok(())
				});
				let copied_ok = copying && result.is_ok();
				if !copied_ok {
					copied.take();
				}
				publish(match result {
					Ok(()) if copying => Status::Copied,
					Ok(()) => Status::Saved,
					Err("Cancelled") => Status::Cancelled,
					Err(error) => Status::Failed(error),
				});
				done.store(true, Ordering::Release);
				context.request_repaint();
				if copied_ok && let Some(owner) = owner {
					// Linux owns the selection while this clipboard object lives; video paste
					// needs its one bounded file until the next copy or session cancellation.
					let _ = owner.recv();
				}
				if let (Some(clipboard), Some(file)) = (clipboard.as_mut(), copied.as_ref())
					&& clear_copied_file(clipboard, &file.path).is_err()
				{
					copy_cleanup_failed.store(true, Ordering::Release);
				}
				drop(clipboard);
				drop(copied);
				if let Some(done) = clipboard_done {
					done.store(true, Ordering::Release);
				}
				context.request_repaint();
			});
		if spawn.is_err() {
			self.job = None;
			self.status = Status::Failed("Download worker unavailable");
			return Err("Download worker unavailable");
		}
		if copying {
			self.clipboard_thread = spawn.ok();
		}
		Ok(())
	}
	pub fn poll(&mut self) -> &Status {
		if let Some(job) = &self.job {
			let done = job.done.load(Ordering::Acquire);
			self.status = job.status.borrow().clone();
			if done {
				self.job = None;
			}
		}
		if self.status == Status::Copied && self.clipboard_owner.is_none() {
			self.status = Status::Cancelled;
		}
		if self.copy_cleanup_failed.swap(false, Ordering::AcqRel) {
			self.status = Status::Failed("Could not release clipboard temporary media");
		}
		&self.status
	}
	pub fn is_active(&self) -> bool {
		self.job.is_some()
			|| (self.clipboard_owner.is_none()
				&& self.clipboard_thread.is_some()
				&& !self.clipboard_done.load(Ordering::Acquire))
	}
	pub fn dismiss(&mut self) {
		self.poll();
		if !self.is_active() {
			self.status = Status::Idle;
		}
	}
	pub fn cancel(&mut self) {
		self.clipboard_owner.take();
		if let Some(job) = &self.job {
			job.cancelled.store(true, Ordering::Release);
			job.wake_cancel.notify_one();
		}
		self.poll();
	}
}
impl Drop for Downloads {
	fn drop(&mut self) {
		self.cancel();
	}
}

/// Temporary copy storage; one owner, one file up to MAX_BYTES. A forced termination
/// can leave this OS-temp directory behind, like interrupted explicit downloads.
struct CopyFile {
	root: PathBuf,
	path: PathBuf,
	cleanup_failed: Arc<AtomicBool>,
}
impl CopyFile {
	fn create(filename: &str, cleanup_failed: Arc<AtomicBool>) -> Result<Self, &'static str> {
		let mut random = [0u8; 16];
		getrandom::fill(&mut random).map_err(|_| "Temporary file unavailable")?;
		let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
		let root = std::env::temp_dir().join(format!("serein-clipboard-{suffix}"));
		#[allow(unused_mut)] // Unix permissions require the mutable builder.
		let mut directory = fs::DirBuilder::new();
		#[cfg(unix)]
		{
			use std::os::unix::fs::DirBuilderExt;
			directory.mode(0o700);
		}
		directory
			.create(&root)
			.map_err(|_| "Cannot create clipboard temporary directory")?;
		let path = root.join(platform::save::safe_filename(filename));
		Ok(Self {
			root,
			path,
			cleanup_failed,
		})
	}
}
impl Drop for CopyFile {
	fn drop(&mut self) {
		if let Err(error) = fs::remove_file(&self.path)
			&& error.kind() != std::io::ErrorKind::NotFound
		{
			self.cleanup_failed.store(true, Ordering::Release);
		}
		if fs::remove_dir(&self.root).is_err() {
			self.cleanup_failed.store(true, Ordering::Release);
		}
	}
}

/// Invalidate our file reference before deletion, without clearing a newer clipboard.
fn clear_copied_file(clipboard: &mut arboard::Clipboard, path: &Path) -> Result<(), &'static str> {
	match clipboard.get().file_list() {
		Ok(paths) if paths.len() == 1 && paths[0] == path => {
			clipboard.clear().map_err(|_| "Could not clear copied file")
		}
		Ok(_) | Err(arboard::Error::ContentNotAvailable) => Ok(()),
		Err(_) => Err("Could not check copied file"),
	}
}

fn clipboard_image(path: &Path) -> Result<image::RgbaImage, &'static str> {
	use image::ImageDecoder;
	let mut reader = image::ImageReader::open(path)
		.and_then(|reader| reader.with_guessed_format())
		.map_err(|_| "Could not read copied image")?;
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(4096);
	limits.max_image_height = Some(4096);
	limits.max_alloc = Some(32 * 1024 * 1024);
	reader.limits(limits);
	let decoder = reader
		.into_decoder()
		.map_err(|_| "Image cannot be copied within the decode limit")?;
	let (width, height) = decoder.dimensions();
	if u64::from(width) * u64::from(height) > 4 * 1024 * 1024 {
		return Err("Copy images with at most 4 million pixels; save larger images instead");
	}
	image::DynamicImage::from_decoder(decoder)
		.map(|image| image.into_rgba8())
		.map_err(|_| "Image could not be decoded")
}
fn copy_media(
	path: &Path,
	image: bool,
	cancelled: &AtomicBool,
) -> Result<arboard::Clipboard, &'static str> {
	let pixels = if image {
		Some(clipboard_image(path)?)
	} else {
		None
	};
	if cancelled.load(Ordering::Acquire) {
		return Err("Cancelled");
	}
	let mut clipboard = arboard::Clipboard::new().map_err(|_| "Clipboard unavailable")?;
	if let Some(pixels) = pixels {
		clipboard.set_image(arboard::ImageData {
			width: pixels.width() as usize,
			height: pixels.height() as usize,
			bytes: std::borrow::Cow::Borrowed(pixels.as_raw()),
		})
	} else {
		clipboard.set().file_list(&[path])
	}
	.map_err(|_| "Could not copy media to clipboard")?;
	Ok(clipboard)
}

pub(crate) fn original_url(attachment: &Attachment) -> Option<url::Url> {
	if !model::valid_attachments(std::slice::from_ref(attachment))
		|| attachment.size == 0
		|| attachment.size > MAX_BYTES
	{
		return None;
	}
	let url = url::Url::parse(attachment.media.url.as_deref()?).ok()?;
	let path: Vec<_> = url.path_segments()?.collect();
	(url.scheme() == "https"
		&& url.host_str() == Some("cdn.discordapp.com")
		&& url.port_or_known_default() == Some(443)
		&& url.username().is_empty()
		&& url.password().is_none()
		&& url.fragment().is_none()
		&& path.len() == 4
		&& path[0] == "attachments"
		&& path[1].parse::<model::Id>().is_ok()
		&& path[2] == attachment.id.to_string()
		&& !path[3].is_empty()
		&& !path[3].contains('\\')
		&& !["%2f", "%5c"]
			.iter()
			.any(|escape| path[3].to_ascii_lowercase().contains(escape))
		&& url
			.query_pairs()
			.all(|(name, _)| matches!(name.as_ref(), "ex" | "is" | "hm")))
	.then_some(url)
}

struct Partial<'a> {
	cleanup_failed: &'a AtomicBool,
	path: PathBuf,
	file: Option<File>,
}
impl<'a> Partial<'a> {
	fn create(destination: &Path, cleanup_failed: &'a AtomicBool) -> Result<Self, &'static str> {
		let parent = destination.parent().ok_or("Invalid save destination")?;
		let mut random = [0u8; 16];
		getrandom::fill(&mut random).map_err(|_| "Temporary file unavailable")?;
		let random: String = random.iter().map(|b| format!("{b:02x}")).collect();
		let path = parent.join(format!(".serein-{random}.partial"));
		let mut options = OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}
		let file = options
			.open(&path)
			.map_err(|_| "Cannot create download file")?;
		Ok(Self {
			path,
			file: Some(file),
			cleanup_failed,
		})
	}
	fn commit(
		mut self,
		destination: &Path,
		existed: bool,
		cancelled: &AtomicBool,
	) -> Result<(), &'static str> {
		self.file
			.take()
			.expect("partial file")
			.sync_all()
			.map_err(|_| "Could not finish download")?;
		if cancelled.load(Ordering::Acquire) {
			return Err("Cancelled");
		}
		if existed {
			// Native Save dialog explicitly confirms replacement. Rename never truncates
			// the old file on an interrupted network transfer.
			fs::rename(&self.path, destination).map_err(|_| "Could not replace selected file")?;
		} else {
			// Atomic no-clobber publication: a file created meanwhile must survive.
			platform::save::publish_new(&self.path, destination).map_err(|error| {
				if error.kind() == std::io::ErrorKind::AlreadyExists {
					"A file appeared at the destination; choose another name or confirm replacement"
				} else {
					"Could not finish saving attachment; check folder permissions and disk space"
				}
			})?;
		}
		Ok(())
	}
}
impl Drop for Partial<'_> {
	fn drop(&mut self) {
		self.file.take();
		if let Err(error) = fs::remove_file(&self.path)
			&& error.kind() != std::io::ErrorKind::NotFound
		{
			self.cleanup_failed.store(true, Ordering::Release);
		}
	}
}
#[allow(clippy::too_many_arguments)]
async fn download(
	client: &reqwest::Client,
	url: url::Url,
	destination: &Path,
	expected: u64,
	allow_replace: bool,
	cancelled: &AtomicBool,
	wake_cancel: &Notify,
	publish: &impl Fn(Status),
) -> Result<(), &'static str> {
	if expected > MAX_BYTES {
		return Err("Attachment must be nonempty and at most 100 MiB");
	}
	let metadata = fs::symlink_metadata(destination);
	let existed = match metadata {
		Ok(meta) if meta.is_file() && allow_replace => true,
		Ok(_) => return Err("Destination exists or is not a regular file"),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
		Err(_) => return Err("Cannot access save destination"),
	};
	if cancelled.load(Ordering::Acquire) {
		return Err("Cancelled");
	}
	let response = tokio::select! {
		biased;
		_ = wake_cancel.notified() => return Err("Cancelled"),
		response = client.get(url).header(reqwest::header::ACCEPT_ENCODING, "identity").send() => response.map_err(|_| "Attachment download failed")?,
	};
	if response.status() != reqwest::StatusCode::OK {
		return Err("Attachment unavailable; reload the conversation and try again");
	}
	if response
		.headers()
		.get(reqwest::header::CONTENT_ENCODING)
		.is_some_and(|encoding| encoding != "identity")
	{
		return Err("Unexpected attachment encoding; reload the conversation");
	}
	let expected = if expected == 0 {
		// Only the validated embedded-image entry point supplies an unknown size.
		if !response
			.headers()
			.get(reqwest::header::CONTENT_TYPE)
			.and_then(|value| value.to_str().ok())
			.is_some_and(|value| {
				value
					.split(';')
					.next()
					.unwrap_or("")
					.trim()
					.eq_ignore_ascii_case("image/png")
			}) {
			return Err("Embedded image proxy did not return PNG");
		}
		response
			.headers()
			.get(reqwest::header::CONTENT_LENGTH)
			.and_then(|value| value.to_str().ok())
			.and_then(|value| value.parse::<u64>().ok())
			.filter(|length| (1..=MAX_EMBED_BYTES).contains(length))
			.ok_or("Embedded image needs a nonempty Content-Length of at most 16 MiB")?
	} else {
		expected
	};
	if response
		.content_length()
		.is_some_and(|length| length != expected)
	{
		return Err("Attachment size changed; reload the conversation");
	}
	let mut response = response;
	let cleanup_failed = AtomicBool::new(false);
	let mut partial = Partial::create(destination, &cleanup_failed)?;
	let result = async move {
		let mut received = 0u64;
		let mut repaint = Instant::now();
		publish(Status::Downloading {
			received,
			total: expected,
		});
		loop {
			if cancelled.load(Ordering::Acquire) {
				return Err("Cancelled");
			}
			let chunk = tokio::select! {
				biased;
				_ = wake_cancel.notified() => return Err("Cancelled"),
				chunk = response.chunk() => chunk.map_err(|_| "Attachment transfer interrupted")?,
			};
			let Some(chunk) = chunk else {
				break;
			};
			received = received
				.checked_add(chunk.len() as u64)
				.ok_or("Attachment exceeds download limit")?;
			if received > expected || received > MAX_BYTES {
				return Err("Attachment exceeds download limit");
			}
			// reqwest supplies transport chunks; split disk writes without retaining a file buffer.
			for block in chunk.chunks(32 * 1024) {
				if cancelled.load(Ordering::Acquire) {
					return Err("Cancelled");
				}
				partial
					.file
					.as_mut()
					.expect("partial file")
					.write_all(block)
					.map_err(|_| "Could not write attachment; check available disk space")?;
			}
			if repaint.elapsed() >= Duration::from_millis(100) {
				publish(Status::Downloading {
					received,
					total: expected,
				});
				repaint = Instant::now();
			}
		}
		if received != expected {
			return Err("Attachment transfer incomplete");
		}
		if cancelled.load(Ordering::Acquire) {
			return Err("Cancelled");
		}
		partial.commit(destination, existed, cancelled)
	}
	.await;
	if cleanup_failed.load(Ordering::Acquire) {
		Err(if result.is_ok() {
			"Attachment saved, but its temporary file could not be removed"
		} else {
			"Download stopped, but its temporary file could not be removed"
		})
	} else {
		result
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tokio::io::{AsyncReadExt, AsyncWriteExt};
	#[test]
	fn cancelling_copy_cannot_restore_a_late_copied_status() {
		let (owner, _keep_alive) = std::sync::mpsc::sync_channel(0);
		let (status, receive) = watch::channel(Status::Downloading {
			received: 0,
			total: 1,
		});
		let done = Arc::new(AtomicBool::new(false));
		let mut downloads = Downloads {
			clipboard_owner: Some(owner),
			job: Some(Job {
				cancelled: Arc::new(AtomicBool::new(false)),
				wake_cancel: Arc::new(Notify::new()),
				status: receive,
				done: done.clone(),
			}),
			status: Status::Idle,
			clipboard_thread: None,
			clipboard_done: Arc::new(AtomicBool::new(false)),
			copy_cleanup_failed: Arc::new(AtomicBool::new(false)),
		};
		downloads.cancel();
		status.send_replace(Status::Copied);
		done.store(true, Ordering::Release);
		assert_eq!(downloads.poll(), &Status::Cancelled);
		assert!(!downloads.is_active());
		downloads.status = Status::Copied;
		downloads.cancel();
		assert_eq!(downloads.poll(), &Status::Cancelled);
	}

	#[test]
	fn copied_images_decode_with_limits_and_temporary_files_are_removed() {
		let failed = Arc::new(AtomicBool::new(false));
		let file = CopyFile::create("../../CON.png", failed.clone()).unwrap();
		assert_eq!(file.path.parent(), Some(file.root.as_path()));
		let root = file.root.clone();
		image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 255]))
			.save_with_format(&file.path, image::ImageFormat::Png)
			.unwrap();
		let pixels = clipboard_image(&file.path).unwrap();
		assert_eq!(pixels.dimensions(), (2, 1));
		assert_eq!(pixels.as_raw(), &[20, 40, 60, 255, 20, 40, 60, 255]);
		image::RgbaImage::new(4096, 1025)
			.save_with_format(&file.path, image::ImageFormat::Png)
			.unwrap();
		assert!(clipboard_image(&file.path).is_err());
		fs::write(&file.path, b"not an image").unwrap();
		assert!(clipboard_image(&file.path).is_err());
		assert!(copy_media(&file.path, false, &AtomicBool::new(true)).is_err());
		drop(file);
		assert!(!root.exists());
		assert!(!failed.load(Ordering::Acquire));
	}

	#[test]
	#[ignore = "Explicit native check: replaces the system clipboard with synthetic media"]
	fn native_media_clipboard_roundtrip() {
		let failed = Arc::new(AtomicBool::new(false));
		let file = CopyFile::create("synthetic.png", failed.clone()).unwrap();
		image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 255]))
			.save_with_format(&file.path, image::ImageFormat::Png)
			.unwrap();
		let cancelled = AtomicBool::new(false);
		let mut clipboard = copy_media(&file.path, true, &cancelled).unwrap();
		let pixels = clipboard.get_image().unwrap();
		assert_eq!((pixels.width, pixels.height), (2, 1));
		assert_eq!(pixels.bytes.as_ref(), &[20, 40, 60, 255, 20, 40, 60, 255]);
		drop(clipboard);
		let video = CopyFile::create("synthetic.mp4", failed.clone()).unwrap();
		fs::write(&video.path, b"synthetic video file clipboard payload").unwrap();
		let mut clipboard = copy_media(&video.path, false, &cancelled).unwrap();
		assert_eq!(
			clipboard.get().file_list().unwrap(),
			vec![video.path.clone()]
		);
		clear_copied_file(&mut clipboard, &video.path).unwrap();
		assert!(matches!(
			clipboard.get().file_list(),
			Err(arboard::Error::ContentNotAvailable)
		));
		clipboard.set_text("synthetic newer clipboard").unwrap();
		clear_copied_file(&mut clipboard, &video.path).unwrap();
		assert_eq!(clipboard.get_text().unwrap(), "synthetic newer clipboard");
		clipboard.clear().unwrap();
		drop(clipboard);
		drop(video);
		drop(file);
		assert!(!failed.load(Ordering::Acquire));
	}

	fn attachment() -> Attachment {
		Attachment { duration_ms: None, waveform: Vec::new(), id: model::Id(2), filename: "synthetic image.png".into(), description: None,
            content_type: Some("image/png".into()), size: 1024, spoiler: false,
            media: model::EmbedMedia { url: Some("https://cdn.discordapp.com/attachments/1/2/synthetic%20image.png?ex=123&is=123&hm=abc".into()), ..Default::default() } }
	}
	async fn endpoint(
		body: Vec<u8>,
		chunked: bool,
		truncated: bool,
	) -> (url::Url, tokio::task::JoinHandle<()>) {
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/attachment",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let task = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			let mut request = Vec::new();
			let mut byte = [0u8; 1];
			while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
				socket.read_exact(&mut byte).await.unwrap();
				request.push(byte[0]);
			}
			let headers = String::from_utf8(request).unwrap().to_ascii_lowercase();
			assert!(
				!headers.contains("authorization:") && !headers.contains("proxy-authorization:")
			);
			assert!(headers.contains("accept-encoding: identity"));
			let header = if chunked {
				"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".into()
			} else {
				format!(
					"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
					body.len() + usize::from(truncated)
				)
			};
			if socket.write_all(header.as_bytes()).await.is_err() {
				return;
			}
			for block in body.chunks(1024) {
				if chunked
					&& socket
						.write_all(format!("{:x}\r\n", block.len()).as_bytes())
						.await
						.is_err()
				{
					return;
				}
				if socket.write_all(block).await.is_err() {
					return;
				}
				if chunked && socket.write_all(b"\r\n").await.is_err() {
					return;
				}
			}
			if chunked {
				let _ = socket.write_all(b"0\r\n\r\n").await;
			}
		});
		(url, task)
	}
	#[tokio::test]
	async fn invalid_copy_admission_updates_visible_failure() {
		let mut downloads = Downloads::default();
		let mut image = attachment();
		image.size = 0;
		assert!(
			downloads
				.start_copy(
					image.clone(),
					&tokio::runtime::Handle::current(),
					&eframe::egui::Context::default()
				)
				.is_err()
		);
		assert_eq!(
			downloads.poll(),
			&Status::Failed("Attachment download unavailable")
		);
		image.size = 1024;
		image.media.url = Some("https://untrusted.example/image.png".into());
		assert!(
			downloads
				.start_copy(
					image,
					&tokio::runtime::Handle::current(),
					&eframe::egui::Context::default()
				)
				.is_err()
		);
		assert_eq!(
			downloads.poll(),
			&Status::Failed("Attachment download unavailable")
		);
		assert!(!downloads.is_active());
	}

	#[tokio::test]
	async fn embedded_image_download_requires_bounded_png_length() {
		let failed = Arc::new(AtomicBool::new(false));
		let file = CopyFile::create("image.png", failed.clone()).unwrap();
		let client = reqwest::Client::builder()
			.no_proxy()
			.redirect(reqwest::redirect::Policy::none())
			.timeout(Duration::from_secs(5))
			.build()
			.unwrap();
		let cancelled = AtomicBool::new(false);
		let wake = Notify::new();
		let mut body = std::io::Cursor::new(Vec::new());
		image::RgbaImage::from_pixel(2, 1, image::Rgba([20, 40, 60, 255]))
			.write_to(&mut body, image::ImageFormat::Png)
			.unwrap();
		let body = body.into_inner();
		let (url, server) = endpoint(body.clone(), false, false).await;
		download(
			&client,
			url,
			&file.path,
			0,
			false,
			&cancelled,
			&wake,
			&|_| {},
		)
		.await
		.unwrap();
		server.await.unwrap();
		assert_eq!(fs::read(&file.path).unwrap(), body);
		// A header is mandatory; a truncated declared body cannot replace a saved file.
		for (chunked, truncated) in [(true, false), (false, true)] {
			let (url, server) = endpoint(body.clone(), chunked, truncated).await;
			assert!(
				download(
					&client,
					url,
					&file.path,
					0,
					true,
					&cancelled,
					&wake,
					&|_| {}
				)
				.await
				.is_err()
			);
			server.await.unwrap();
			assert_eq!(fs::read(&file.path).unwrap(), body);
		}
		for headers in [
			"Content-Length: 0\r\nContent-Type: image/png\r\n",
			"Content-Length: 16777217\r\nContent-Type: image/png\r\n",
			"Content-Length: 10\r\nContent-Type: image/jpeg\r\n",
			"Content-Length: 10\r\n",
		] {
			let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
			let url = url::Url::parse(&format!("http://{}/image", listener.local_addr().unwrap()))
				.unwrap();
			let server = tokio::spawn(async move {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut request = [0; 8192];
				let _ = socket.read(&mut request).await.unwrap();
				socket
					.write_all(
						format!("HTTP/1.1 200 OK\r\n{headers}Connection: close\r\n\r\n").as_bytes(),
					)
					.await
					.unwrap();
			});
			assert!(
				download(
					&client,
					url,
					&file.path,
					0,
					true,
					&cancelled,
					&wake,
					&|_| {}
				)
				.await
				.is_err()
			);
			server.await.unwrap();
			assert_eq!(fs::read(&file.path).unwrap(), body);
			assert_eq!(fs::read_dir(&file.root).unwrap().count(), 1);
		}
		drop(file);
		assert!(!failed.load(Ordering::Acquire));
	}

	#[tokio::test]
	async fn explicit_download_stream_limits_cancel_and_atomic_replacement() {
		let mut image = attachment();
		assert!(original_url(&image).is_some());
		// Admission depends on a bounded original CDN target, not an image MIME type.
		image.filename = "synthetic file.bin".into();
		image.media.url = Some(
			"https://cdn.discordapp.com/attachments/1/2/synthetic%20file.bin?ex=123&is=123&hm=abc"
				.into(),
		);
		for kind in [
			Some("application/pdf"),
			Some("application/octet-stream"),
			None,
		] {
			image.content_type = kind.map(str::to_owned);
			assert!(!image.is_image());
			assert!(original_url(&image).is_some());
		}
		for size in [0, MAX_BYTES + 1] {
			image.size = size;
			assert!(original_url(&image).is_none());
		}
		image.size = MAX_BYTES;
		assert!(original_url(&image).is_some());
		image.size = 1024;
		for url in [
			"http://cdn.discordapp.com/attachments/1/2/a.png",
			"https://cdn.discordapp.com.evil.test/attachments/1/2/a.png",
			"https://user@cdn.discordapp.com/attachments/1/2/a.png",
			"https://cdn.discordapp.com:444/attachments/1/2/a.png",
			"https://cdn.discordapp.com/attachments/1/99/a.png",
			"https://cdn.discordapp.com/attachments/1/2/a.png?width=1",
			"https://cdn.discordapp.com/attachments/1/2/%2fapi",
			"https://cdn.discordapp.com/attachments/1/2/a.png#fragment",
			"https://127.0.0.1/attachments/1/2/a.png",
			"https://media.discordapp.net/attachments/1/2/a.png",
		] {
			image.media.url = Some(url.into());
			assert!(original_url(&image).is_none());
		}
		let root =
			std::env::temp_dir().join(format!("serein-download-tests-{}", std::process::id()));
		fs::create_dir_all(&root).unwrap();
		let destination = root.join("chosen.bin");
		let client = reqwest::Client::builder()
			.no_proxy()
			.redirect(reqwest::redirect::Policy::none())
			.timeout(Duration::from_secs(5))
			.build()
			.unwrap();
		let cancelled = AtomicBool::new(false);
		let wake = Notify::new();
		// Preserve arbitrary binary bytes (including NUL and invalid UTF-8) without decoding.
		let content: Vec<u8> = (0..=255).cycle().take(64 * 1024).collect();
		let (url, task) = endpoint(content.clone(), false, false).await;
		download(
			&client,
			url,
			&destination,
			content.len() as u64,
			false,
			&cancelled,
			&wake,
			&|_| {},
		)
		.await
		.unwrap();
		task.await.unwrap();
		assert_eq!(fs::read(&destination).unwrap(), content);
		// A failed or oversized stream cannot truncate the existing destination.
		for (chunked, truncated, expected) in
			[(true, false, 1024), (false, true, content.len() as u64 + 1)]
		{
			let (url, task) = endpoint(content.clone(), chunked, truncated).await;
			assert!(
				download(
					&client,
					url,
					&destination,
					expected,
					true,
					&cancelled,
					&wake,
					&|_| {}
				)
				.await
				.is_err()
			);
			task.await.unwrap();
			assert_eq!(fs::read(&destination).unwrap(), content);
			assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
		}
		// Explicit overwrite publishes only the complete new file.
		let replacement = vec![91; 8192];
		let (url, task) = endpoint(replacement.clone(), false, false).await;
		download(
			&client,
			url,
			&destination,
			replacement.len() as u64,
			true,
			&cancelled,
			&wake,
			&|_| {},
		)
		.await
		.unwrap();
		task.await.unwrap();
		assert_eq!(fs::read(&destination).unwrap(), replacement);
		// Cancellation after partial creation leaves the previous file and no partial sibling.
		let (url, task) = endpoint(content.clone(), false, false).await;
		assert_eq!(
			download(
				&client,
				url,
				&destination,
				content.len() as u64,
				true,
				&cancelled,
				&wake,
				&|_| {
					cancelled.store(true, Ordering::Release);
					wake.notify_one();
				}
			)
			.await,
			Err("Cancelled")
		);
		task.await.unwrap();
		assert_eq!(fs::read(&destination).unwrap(), replacement);
		assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
		cancelled.store(false, Ordering::Release);
		let wake = Notify::new();
		// An unrelated file appearing during a new save is never overwritten.
		fs::remove_file(&destination).unwrap();
		let (url, task) = endpoint(content.clone(), false, false).await;
		assert!(
			download(
				&client,
				url,
				&destination,
				content.len() as u64,
				false,
				&cancelled,
				&wake,
				&|_| {
					fs::write(&destination, b"unrelated").unwrap();
				}
			)
			.await
			.is_err()
		);
		task.await.unwrap();
		assert_eq!(fs::read(&destination).unwrap(), b"unrelated");
		assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
		// A stalled HTTP body is interrupted by cancellation, not the long request timeout.
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/stalled",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let server = tokio::spawn(async move {
			let (mut socket, _) = listener.accept().await.unwrap();
			let mut request = [0; 8192];
			let _ = socket.read(&mut request).await.unwrap();
			socket
				.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\n\r\n")
				.await
				.unwrap();
			std::future::pending::<()>().await;
		});
		let cancelled = Arc::new(AtomicBool::new(false));
		let wake = Arc::new(Notify::new());
		let flag = cancelled.clone();
		let notify = wake.clone();
		let cancel = tokio::spawn(async move {
			tokio::time::sleep(Duration::from_millis(50)).await;
			flag.store(true, Ordering::Release);
			notify.notify_one();
		});
		assert_eq!(
			tokio::time::timeout(
				Duration::from_secs(2),
				download(
					&client,
					url,
					&destination,
					1024,
					true,
					&cancelled,
					&wake,
					&|_| {}
				)
			)
			.await
			.unwrap(),
			Err("Cancelled")
		);
		cancel.await.unwrap();
		server.abort();
		assert_eq!(fs::read(&destination).unwrap(), b"unrelated");
		assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
		// Filesystem cleanup failures are signalled rather than silently reported as success.
		let cleanup_failed = AtomicBool::new(false);
		let mut partial = Partial::create(&destination, &cleanup_failed).unwrap();
		let partial_path = partial.path.clone();
		partial.file.take();
		fs::remove_file(&partial_path).unwrap();
		fs::create_dir(&partial_path).unwrap();
		drop(partial);
		assert!(cleanup_failed.load(Ordering::Acquire));
		fs::remove_dir(&partial_path).unwrap();
		fs::remove_dir_all(root).unwrap();
	}
}
