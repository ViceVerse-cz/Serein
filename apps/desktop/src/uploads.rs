//! One bounded attachment batch selection or upload; paths never enter UI state or diagnostics.
use client_core::Command;
use discord_api::upload::{Source, Status};
use eframe::egui;
use model::Id;
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
	mpsc,
};
use tokio::sync::watch;

pub struct UploadRequest {
	pub command: Command,
	pub source: Vec<Source>,
	pub progress: watch::Sender<Status>,
	pub cancel: watch::Sender<bool>,
}

type Selected = (Source, Option<egui::ColorImage>);
struct Choosing {
	result: mpsc::Receiver<Result<Option<Vec<Selected>>, &'static str>>,
	cancelled: Arc<AtomicBool>,
}
/// One chosen file with its composer thumbnail; `key` survives removals while a thumbnail
/// is still decoding for it.
struct Chosen {
	key: u64,
	source: Source,
	preview: Option<Arc<egui::ColorImage>>,
}
/// Longest edge of the composer thumbnail; the full decode stays bounded by `image::Limits`.
const PREVIEW_EDGE: u32 = 320;
const PREVIEW_ALLOC: u64 = 64 * 1024 * 1024;
const SHARE_BYTES: usize = 8 * 1024 * 1024;

fn image_share_source(asset: model::ImageShare) -> Option<(String, String, image::ImageFormat)> {
	use model::ImageShare;
	let (id, kind, host, extension, query) = match asset {
		ImageShare::Emoji { id, animated } => (
			id,
			"emoji",
			"cdn.discordapp.com",
			if animated { "gif" } else { "png" },
			"",
		),
		ImageShare::Sticker {
			id,
			format_type: 1 | 2,
		} => (id, "sticker", "cdn.discordapp.com", "png", ""),
		ImageShare::Sticker { id, format_type: 3 } => (
			id,
			"sticker",
			"media.discordapp.net",
			"png",
			"?passthrough=false",
		),
		ImageShare::Sticker { id, format_type: 4 } => {
			(id, "sticker", "media.discordapp.net", "gif", "")
		}
		_ => return None,
	};
	(id.0 != 0).then(|| {
		(
			format!("https://{host}/{kind}s/{id}.{extension}{query}"),
			format!("{kind}-{id}.{extension}"),
			if extension == "gif" {
				image::ImageFormat::Gif
			} else {
				image::ImageFormat::Png
			},
		)
	})
}

async fn download_image_share(url: &str, cancelled: &AtomicBool) -> Result<Vec<u8>, &'static str> {
	let client = reqwest::Client::builder()
		.https_only(true)
		.no_proxy()
		.redirect(reqwest::redirect::Policy::none())
		.timeout(std::time::Duration::from_secs(15))
		.connect_timeout(std::time::Duration::from_secs(5))
		.build()
		.map_err(|_| "Could not prepare image download")?;
	if cancelled.load(Ordering::Acquire) {
		return Err("Image selection cancelled");
	}
	let mut response = client
		.get(url)
		.send()
		.await
		.map_err(|_| "Could not download this image")?;
	if !response.status().is_success()
		|| response
			.content_length()
			.is_some_and(|size| size > SHARE_BYTES as u64)
	{
		return Err("Image unavailable or larger than 8 MiB");
	}
	let mut bytes = Vec::with_capacity(
		response
			.content_length()
			.unwrap_or(4096)
			.min(SHARE_BYTES as u64) as usize,
	);
	while let Some(chunk) = response
		.chunk()
		.await
		.map_err(|_| "Image download interrupted")?
	{
		if cancelled.load(Ordering::Acquire) {
			return Err("Image selection cancelled");
		}
		if chunk.len() > SHARE_BYTES - bytes.len() {
			return Err("Image is larger than 8 MiB");
		}
		bytes.extend_from_slice(&chunk);
	}
	Ok(bytes)
}

fn synthetic_share(format: image::ImageFormat) -> Result<Vec<u8>, &'static str> {
	#[cfg(any(test, feature = "demo"))]
	{
		let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
			32,
			32,
			image::Rgba([103, 192, 177, 255]),
		));
		let mut bytes = std::io::Cursor::new(Vec::new());
		image
			.write_to(&mut bytes, format)
			.map_err(|_| "Could not prepare synthetic image")?;
		Ok(bytes.into_inner())
	}
	#[cfg(not(any(test, feature = "demo")))]
	{
		let _ = format;
		Err("Synthetic image sharing requires a demo build")
	}
}
fn previewable(filename: &str) -> bool {
	filename.rsplit_once('.').is_some_and(|(_, extension)| {
		matches!(
			extension.to_ascii_lowercase().as_str(),
			"png" | "jpg" | "jpeg" | "gif" | "webp"
		)
	})
}
/// Downscaled pixels for the composer card, decoded on a blocking worker, never in a frame.
async fn preview(source: &Source) -> Option<egui::ColorImage> {
	if !previewable(source.filename()) {
		return None;
	}
	let bytes = source.preview_bytes(discord_api::upload::MAX_BYTES).await?;
	tokio::task::spawn_blocking(move || decode_preview(&bytes))
		.await
		.ok()
		.flatten()
}
fn decode_preview(bytes: &[u8]) -> Option<egui::ColorImage> {
	let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
		.with_guessed_format()
		.ok()?;
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(8192);
	limits.max_image_height = Some(8192);
	limits.max_alloc = Some(PREVIEW_ALLOC);
	reader.limits(limits);
	let image = reader
		.decode()
		.ok()?
		.thumbnail(PREVIEW_EDGE, PREVIEW_EDGE)
		.into_rgba8();
	Some(egui::ColorImage::from_rgba_unmultiplied(
		[image.width() as usize, image.height() as usize],
		image.as_raw(),
	))
}
struct Uploading {
	progress: watch::Receiver<Status>,
	cancel: watch::Sender<bool>,
	cancelling: bool,
}
#[derive(Default)]
pub struct Uploads {
	auto_image: bool,
	scope: Option<(u64, Id)>,
	selected: Vec<Chosen>,
	next_key: u64,
	/// Thumbnails still decoding for pasted files, by `Chosen::key`; at most `MAX_FILES`.
	previewing: Vec<(u64, mpsc::Receiver<Option<egui::ColorImage>>)>,
	choosing: Option<Choosing>,
	uploading: Option<Uploading>,
	/// Progress of the batch in flight, and only ever a running state: terminal failures
	/// leave through `notice`, so nothing here outlives the work it describes.
	last: Option<Status>,
	/// One problem, announced once. Drained by the caller into a toast.
	notice: Option<&'static str>,
}
impl Uploads {
	/// A picker click authorizes one image send after preparation.
	#[allow(clippy::too_many_arguments)]
	pub fn start_image_share(
		&mut self,
		generation: u64,
		channel: Id,
		asset: model::ImageShare,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
		demo: bool,
	) -> Result<(), &'static str> {
		if self.busy() {
			return Err("Wait for the current attachment operation to finish");
		}
		if !self.selected.is_empty() {
			return Err("Send or remove existing attachments before selecting an image");
		}
		let (url, filename, format) =
			image_share_source(asset).ok_or("Unsupported emoji or sticker artwork")?;
		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let (send, result) = mpsc::sync_channel(1);
		let context = context.clone();
		runtime.spawn(async move {
			let result = async {
				let bytes = if demo {
					synthetic_share(format)?
				} else {
					download_image_share(&url, &flag).await?
				};
				if flag.load(Ordering::Acquire) {
					return Ok(None);
				}
				let selected = tokio::task::spawn_blocking(move || {
					if bytes.is_empty()
						|| bytes.len() > SHARE_BYTES
						|| image::guess_format(&bytes).ok() != Some(format)
					{
						return Err("Unsupported or invalid image data");
					}
					let thumbnail =
						decode_preview(&bytes).ok_or("Could not decode this image safely")?;
					let source = Source::image_bytes(filename, bytes)?;
					Ok((source, Some(thumbnail)))
				})
				.await
				.map_err(|_| "Image preparation interrupted")??;
				Ok((!flag.load(Ordering::Acquire)).then(|| vec![selected]))
			}
			.await;
			let _ = send.send(result);
			context.request_repaint();
		});
		self.scope = Some((generation, channel));
		self.last = None;
		self.choosing = Some(Choosing { result, cancelled });
		self.auto_image = true;
		Ok(())
	}
	pub fn image_send(
		&mut self,
		state: &mut client_core::State,
		enabled: bool,
	) -> Option<client_core::Command> {
		if !enabled {
			self.auto_image = false;
		}
		if !self.auto_image || self.busy() {
			return None;
		}
		self.auto_image = false;
		if self.scope != state.selected.map(|channel| (state.generation, channel))
			|| self.selected.len() != 1
		{
			return None;
		}
		state.prepare_image_send(self.selected[0].source.filename())
	}
	pub fn select_pasted(
		&mut self,
		generation: u64,
		channel: Id,
		source: Vec<Source>,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
	) -> Result<(), &'static str> {
		if self.busy() {
			return Err("Wait for the current attachment operation to finish");
		}
		self.admit(&source)?;
		self.scope = Some((generation, channel));
		self.last = None;
		for source in source {
			let key = self.push(source, None);
			if previewable(self.selected.last().map_or("", |c| c.source.filename())) {
				let (send, receive) = mpsc::sync_channel(1);
				let context = context.clone();
				let copy = self.selected.last().map(|c| c.source.clone());
				runtime.spawn(async move {
					let thumbnail = match copy {
						Some(source) => preview(&source).await,
						None => None,
					};
					let _ = send.send(thumbnail);
					context.request_repaint();
				});
				self.previewing.push((key, receive));
			}
		}
		Ok(())
	}
	pub fn start_choose(
		&mut self,
		generation: u64,
		channel: Id,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
		parent: Arc<winit::window::Window>,
	) -> Result<(), &'static str> {
		if self.busy() {
			return Err("Wait for the current attachment operation to finish");
		}
		// Construct on the native UI thread; await and inspect outside rendering.
		let dialog = platform::save::attachment_source(parent);
		self.start_selection(generation, channel, runtime, context, dialog);
		Ok(())
	}
	pub fn start_drop(
		&mut self,
		generation: u64,
		channel: Id,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
		files: Vec<egui::DroppedFileHandle>,
	) -> Result<(), &'static str> {
		if self.busy() {
			return Err("Wait for the current attachment operation to finish");
		}
		if files.is_empty() || files.len() + self.selected.len() > discord_api::upload::MAX_FILES {
			return Err("Attach up to 10 files per message");
		}
		let mut paths = Vec::with_capacity(files.len());
		for file in files {
			let path = file.path();
			if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > 4096 {
				return Err("Drop a local file with a supported path");
			}
			paths.push(path.to_owned());
		}
		self.start_selection(
			generation,
			channel,
			runtime,
			context,
			async move { Some(paths) },
		);
		Ok(())
	}
	fn start_selection(
		&mut self,
		generation: u64,
		channel: Id,
		runtime: &tokio::runtime::Handle,
		context: &egui::Context,
		selection: impl std::future::Future<Output = Option<Vec<std::path::PathBuf>>> + Send + 'static,
	) {
		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let (send, result) = mpsc::sync_channel(1);
		let context = context.clone();
		runtime.spawn(async move {
			let result = async {
				let path = selection.await;
				if flag.load(Ordering::Acquire) {
					return Ok(None);
				}
				let Some(paths) = path else {
					return Ok(None);
				};
				if paths.is_empty() || paths.len() > discord_api::upload::MAX_FILES {
					return Err("Attach up to 10 files per message");
				}
				let mut selected = Vec::with_capacity(paths.len());
				let mut total = 0;
				for path in paths {
					let source = Source::inspect(path).await?;
					total += source.size();
					if total > discord_api::upload::MAX_TOTAL_BYTES {
						return Err(
							"Attachments must total at most 500 MB; account limits may be lower",
						);
					}
					let thumbnail = preview(&source).await;
					selected.push((source, thumbnail));
				}
				Ok(Some(selected))
			}
			.await;
			let _ = send.send(result);
			context.request_repaint();
		});
		self.scope = Some((generation, channel));
		self.last = None;
		self.choosing = Some(Choosing { result, cancelled });
	}
	pub fn revalidate_scope(&mut self, generation: u64, channel: Option<Id>, allowed: bool) {
		if self
			.scope
			.is_some_and(|scope| !allowed || Some(scope) != channel.map(|id| (generation, id)))
		{
			self.remove();
		}
	}
	pub fn poll(
		&mut self,
		generation: u64,
		channel: Option<Id>,
		allowed: bool,
		context: &egui::Context,
	) {
		self.revalidate_scope(generation, channel, allowed);
		if let Some(choosing) = &self.choosing {
			let result = match choosing.result.try_recv() {
				Ok(result) => Some(result),
				Err(mpsc::TryRecvError::Disconnected) => {
					Some(Err("Attachment selection interrupted"))
				}
				Err(mpsc::TryRecvError::Empty) => None,
			};
			if let Some(result) = result {
				let cancelled = choosing.cancelled.load(Ordering::Acquire);
				self.choosing = None;
				self.last = None;
				if !cancelled {
					match result {
						Ok(Some(selected)) => {
							let sources: Vec<_> =
								selected.iter().map(|(source, _)| source.clone()).collect();
							match self.admit(&sources) {
								Ok(()) => {
									for (source, thumbnail) in selected {
										self.push(source, thumbnail);
									}
								}
								Err(error) => self.notice = Some(error),
							}
						}
						Ok(None) => {}
						Err(error) => self.notice = Some(error),
					}
				}
			}
		}
		self.previewing
			.retain(|(key, receive)| match receive.try_recv() {
				Ok(thumbnail) => {
					if let Some(chosen) = self.selected.iter_mut().find(|c| c.key == *key) {
						chosen.preview = thumbnail.map(Arc::new);
					}
					false
				}
				Err(mpsc::TryRecvError::Disconnected) => false,
				Err(mpsc::TryRecvError::Empty) => true,
			});
		if let Some(uploading) = &mut self.uploading {
			let status = uploading.progress.borrow_and_update().clone();
			let closed = uploading.progress.has_changed().is_err();
			if closed {
				// A stream that ends mid-flight is a failure to announce, not a state to hold.
				let (reached, problem) = match status {
					Status::Sending => (
						None,
						Some("Message outcome unknown; check the conversation before retrying"),
					),
					Status::Preparing | Status::Uploading { .. } if uploading.cancelling => {
						(Some(Status::Cancelled), None)
					}
					Status::Preparing | Status::Uploading { .. } => {
						(None, Some("Attachment upload interrupted"))
					}
					Status::Failed(error) => (None, Some(error)),
					terminal => (Some(terminal), None),
				};
				self.last = reached;
				if problem.is_some() {
					self.notice = problem;
				}
			} else {
				self.last = Some(status);
			}
			// Cancellation retains this slot until the actual network worker releases its sender.
			if closed {
				self.uploading = None;
			}
		}
		if self.busy() {
			context.request_repaint_after(std::time::Duration::from_millis(100));
		}
	}
	fn push(&mut self, source: Source, preview: Option<egui::ColorImage>) -> u64 {
		let key = self.next_key;
		self.next_key += 1;
		self.selected.push(Chosen {
			key,
			source,
			preview: preview.map(Arc::new),
		});
		key
	}
	fn admit(&self, sources: &[Source]) -> Result<(), &'static str> {
		if sources.is_empty()
			|| self.selected.len() + sources.len() > discord_api::upload::MAX_FILES
		{
			return Err("Attach up to 10 files per message");
		}
		if self
			.selected
			.iter()
			.map(|chosen| &chosen.source)
			.chain(sources)
			.map(Source::size)
			.sum::<u64>()
			> discord_api::upload::MAX_TOTAL_BYTES
		{
			return Err("Attachments must total at most 500 MB; account limits may be lower");
		}
		Ok(())
	}
	pub fn files(&self) -> Vec<(String, u64)> {
		self.selected
			.iter()
			.map(|c| (c.source.filename().to_owned(), c.source.size()))
			.collect()
	}
	pub fn remove_at(&mut self, index: usize) {
		if !self.busy() && index < self.selected.len() {
			let removed = self.selected.remove(index);
			self.previewing.retain(|(key, _)| *key != removed.key);
		}
	}

	pub fn selection(&self) -> Option<(&str, u64)> {
		self.selected
			.first()
			.map(|c| (c.source.filename(), c.source.size()))
	}
	/// One thumbnail slot per file in `files()` order; non-images and pending decodes are `None`.
	pub fn previews(&self) -> Vec<Option<Arc<egui::ColorImage>>> {
		self.selected.iter().map(|c| c.preview.clone()).collect()
	}
	pub fn busy(&self) -> bool {
		self.choosing.is_some() || self.uploading.is_some()
	}
	pub fn has_unsent(&self) -> bool {
		!self.selected.is_empty() || self.busy()
	}
	pub fn transfer_progress(&self) -> (Option<(u64, u64)>, bool) {
		match self.last {
			Some(Status::Uploading { sent, total }) => (Some((sent, total)), false),
			Some(Status::Sending | Status::Finished) => (None, true),
			_ => (None, false),
		}
	}
	/// Takes the pending problem, if any. Progress is not reported here: the timeline's
	/// own pending row carries it, and a toast is for what went wrong, not what is going.
	pub fn take_notice(&mut self) -> Option<&'static str> {
		self.notice.take()
	}
	pub fn remove(&mut self) {
		self.selected.clear();
		self.previewing.clear();
		self.cancel();
		self.last = None;
	}
	pub fn cancel(&mut self) {
		self.auto_image = false;
		if let Some(choosing) = &self.choosing {
			choosing.cancelled.store(true, Ordering::Release);
		}
		if let Some(uploading) = &mut self.uploading {
			uploading.cancelling = true;
			uploading.cancel.send_replace(true);
		}
	}
	pub fn take_source(&mut self, generation: u64, channel: Id) -> Option<Vec<Source>> {
		if self.scope != Some((generation, channel)) || self.busy() {
			return None;
		}
		self.previewing.clear();
		(!self.selected.is_empty()).then(|| {
			std::mem::take(&mut self.selected)
				.into_iter()
				.map(|c| c.source)
				.collect()
		})
	}
	pub fn begin_upload(
		&mut self,
		progress: watch::Receiver<Status>,
		cancel: watch::Sender<bool>,
	) -> Result<(), &'static str> {
		if self.busy() || !self.selected.is_empty() || self.scope.is_none() {
			cancel.send_replace(true);
			return Err("Attachment operation already active or no selection scope");
		}
		self.last = Some(progress.borrow().clone());
		self.uploading = Some(Uploading {
			progress,
			cancel,
			cancelling: false,
		});
		Ok(())
	}
}
impl Drop for Uploads {
	fn drop(&mut self) {
		self.cancel();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[tokio::test]
	async fn image_sharing_stages_bounded_artwork_and_cancels_on_navigation() {
		let context = egui::Context::default();
		let runtime = tokio::runtime::Handle::current();
		let mut uploads = Uploads::default();
		let asset = model::ImageShare::Sticker {
			id: Id(7),
			format_type: 2,
		};
		assert_eq!(
			image_share_source(asset).unwrap().0,
			"https://cdn.discordapp.com/stickers/7.png"
		);
		assert_eq!(
			image_share_source(model::ImageShare::Sticker {
				id: Id(7),
				format_type: 3
			})
			.unwrap()
			.0,
			"https://media.discordapp.net/stickers/7.png?passthrough=false"
		);
		assert_eq!(
			image_share_source(model::ImageShare::Sticker {
				id: Id(7),
				format_type: 4
			})
			.unwrap()
			.0,
			"https://media.discordapp.net/stickers/7.gif"
		);
		assert_eq!(
			image_share_source(model::ImageShare::Emoji {
				id: Id(7),
				animated: true
			})
			.unwrap()
			.0,
			"https://cdn.discordapp.com/emojis/7.gif"
		);
		for invalid in [
			model::ImageShare::Emoji {
				id: Id(0),
				animated: false,
			},
			model::ImageShare::Sticker {
				id: Id(7),
				format_type: 5,
			},
		] {
			assert!(
				uploads
					.start_image_share(1, Id(2), invalid, &runtime, &context, true)
					.is_err()
			);
		}
		async fn settle(uploads: &mut Uploads, context: &egui::Context, channel: Id) {
			tokio::time::timeout(std::time::Duration::from_secs(5), async {
				while uploads.busy() {
					uploads.poll(1, Some(channel), true, context);
					tokio::task::yield_now().await;
				}
			})
			.await
			.unwrap();
		}
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.generation = 1;
		state.drafts.insert(channel, "Keep typing".into());
		for asset in [
			asset,
			model::ImageShare::Emoji {
				id: Id(9),
				animated: true,
			},
		] {
			uploads
				.start_image_share(1, channel, asset, &runtime, &context, true)
				.unwrap();
			assert!(uploads.image_send(&mut state, true).is_none());
			settle(&mut uploads, &context, channel).await;
			assert!(uploads.take_notice().is_none());
			assert!(
				matches!(uploads.image_send(&mut state, true), Some(client_core::Command::Send { content, .. }) if content.is_empty())
			);
			assert!(uploads.image_send(&mut state, true).is_none());
			assert_eq!(state.drafts[&channel], "Keep typing");
			assert!(uploads.previews().iter().all(Option::is_some));
			assert_eq!(uploads.take_source(1, channel).unwrap().len(), 1);
		}
		uploads
			.start_image_share(1, channel, asset, &runtime, &context, true)
			.unwrap();
		settle(&mut uploads, &context, channel).await;
		assert!(uploads.image_send(&mut state, false).is_none());
		assert!(uploads.image_send(&mut state, true).is_none());
		uploads.remove();

		uploads
			.start_image_share(1, Id(2), asset, &runtime, &context, true)
			.unwrap();
		uploads.revalidate_scope(1, Some(Id(3)), true);
		settle(&mut uploads, &context, Id(3)).await;
		assert!(uploads.selection().is_none());
		for _ in 0..discord_api::upload::MAX_FILES {
			uploads.push(
				Source::image_bytes("emoji-7.png".into(), vec![1]).unwrap(),
				None,
			);
		}
		assert!(
			uploads
				.start_image_share(1, Id(3), asset, &runtime, &context, true)
				.is_err()
		);
	}
	#[test]
	fn progress_distinguishes_streamed_bytes_from_message_confirmation() {
		let mut uploads = Uploads::default();
		assert_eq!(uploads.transfer_progress(), (None, false));
		uploads.last = Some(Status::Uploading {
			sent: 42,
			total: 100,
		});
		assert_eq!(uploads.transfer_progress(), (Some((42, 100)), false));
		uploads.last = Some(Status::Sending);
		assert_eq!(uploads.transfer_progress(), (None, true));
		uploads.last = Some(Status::Failed("rejected"));
		assert_eq!(uploads.transfer_progress(), (None, false));
	}
	#[tokio::test]
	async fn file_drop_is_bounded_scoped_selection_and_never_reads_handle_bytes() {
		struct SyntheticDrop(std::path::PathBuf);
		impl std::fmt::Debug for SyntheticDrop {
			fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				f.write_str("SyntheticDrop([REDACTED])")
			}
		}
		impl egui::DroppedFile for SyntheticDrop {
			fn path(&self) -> &std::path::Path {
				&self.0
			}
			fn bytes(&self) -> Result<Vec<u8>, String> {
				panic!("Drop admission must never read whole-file bytes")
			}
		}
		async fn settle(uploads: &mut Uploads, context: &egui::Context, channel: Id) {
			tokio::time::timeout(std::time::Duration::from_secs(5), async {
				while uploads.busy() {
					uploads.poll(1, Some(channel), true, context);
					tokio::task::yield_now().await;
				}
			})
			.await
			.unwrap();
		}
		let context = egui::Context::default();
		let runtime = tokio::runtime::Handle::current();
		let mut uploads = Uploads::default();
		let path = std::env::temp_dir().join(format!(
			"serein-drop-{}-{}.txt",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		let handle =
			|path: std::path::PathBuf| -> egui::DroppedFileHandle { Arc::new(SyntheticDrop(path)) };
		for files in [
			vec![],
			(0..11).map(|_| handle(path.clone())).collect(),
			vec![handle(std::path::PathBuf::new())],
			vec![handle("memory-only.txt".into())],
			vec![handle(std::env::temp_dir().join("x".repeat(4097)))],
		] {
			assert!(
				uploads
					.start_drop(1, Id(2), &runtime, &context, files)
					.is_err()
			);
			assert!(!uploads.busy());
		}
		let mut file = tokio::fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&path)
			.await
			.unwrap();
		tokio::io::AsyncWriteExt::write_all(&mut file, b"synthetic drop")
			.await
			.unwrap();
		tokio::io::AsyncWriteExt::flush(&mut file).await.unwrap();
		drop(file);
		assert!(
			uploads
				.start_drop(1, Id(2), &runtime, &context, vec![handle(path.clone())])
				.is_ok()
		);
		assert!(
			uploads
				.start_drop(1, Id(2), &runtime, &context, vec![handle(path.clone())])
				.is_err()
		);
		settle(&mut uploads, &context, Id(2)).await;
		assert_eq!(
			uploads.selection(),
			Some((path.file_name().unwrap().to_str().unwrap(), 14))
		);
		assert!(
			uploads
				.start_drop(1, Id(2), &runtime, &context, vec![handle(path.clone())])
				.is_ok()
		);
		settle(&mut uploads, &context, Id(2)).await;
		assert_eq!(uploads.files().len(), 2);
		uploads.remove_at(1);
		assert_eq!(uploads.files().len(), 1);
		assert!(uploads.selection().is_some());
		assert!(uploads.take_source(1, Id(3)).is_none());
		uploads.remove();
		assert!(
			uploads
				.start_drop(1, Id(2), &runtime, &context, vec![handle(path.clone())])
				.is_ok()
		);
		// A result inspected before or after navigation must never enter the new conversation.
		uploads.poll(1, Some(Id(3)), true, &context);
		settle(&mut uploads, &context, Id(3)).await;
		assert!(uploads.selection().is_none());
		tokio::fs::remove_file(path).await.unwrap();
	}
	#[test]
	fn cancelled_chooser_and_upload_keep_the_single_slot_until_the_worker_finishes() {
		let context = egui::Context::default();
		let (send, result) = mpsc::sync_channel(1);
		let cancelled = Arc::new(AtomicBool::new(false));
		let mut uploads = Uploads {
			auto_image: false,
			scope: Some((1, Id(2))),
			choosing: Some(Choosing {
				result,
				cancelled: cancelled.clone(),
			}),
			selected: vec![],
			next_key: 0,
			previewing: vec![],
			uploading: None,
			last: None,
			notice: None,
		};
		uploads.poll(2, Some(Id(2)), true, &context);
		assert!(cancelled.load(Ordering::Acquire));
		assert!(uploads.busy());
		assert!(send.send(Ok(None)).is_ok());
		uploads.poll(2, Some(Id(2)), true, &context);
		assert!(!uploads.busy());
		assert!(uploads.selection().is_none());

		uploads.scope = Some((2, Id(2)));
		let (progress, receive) = watch::channel(Status::Sending);
		let (cancel, cancellation) = watch::channel(false);
		assert!(uploads.begin_upload(receive, cancel).is_ok());
		uploads.poll(2, Some(Id(3)), true, &context);
		assert!(*cancellation.borrow());
		assert!(uploads.busy());
		drop(progress);
		uploads.poll(2, Some(Id(3)), true, &context);
		assert!(!uploads.busy());
		assert!(uploads.take_notice().unwrap().contains("outcome unknown"));

		let (progress, receive) = watch::channel(Status::Preparing);
		let (cancel, _) = watch::channel(false);
		assert!(uploads.begin_upload(receive, cancel).is_ok());
		progress.send_replace(Status::Finished);
		uploads.poll(2, Some(Id(2)), true, &context);
		assert!(uploads.busy());
		drop(progress);
		uploads.poll(2, Some(Id(2)), true, &context);
		assert!(!uploads.busy());
		assert_eq!(uploads.take_notice(), None);
	}
}
