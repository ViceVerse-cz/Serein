//! One bounded attachment batch: selection, validation and upload hand-off, ported from the
//! egui app's `uploads.rs`. Paths never enter UI state or diagnostics; files are inspected on a
//! worker thread and uploaded by the connection worker through `discord_api::upload`.
use crate::Serein;
use crate::theme::{Icon, color, icon, palette};
use crate::tooltip;
use client_core::{Command, Envelope, Event, auth::Failure};
use discord_api::upload::{MAX_FILES, MAX_TOTAL_BYTES, Source, Status};
use gpui::{
	AnyElement, Context, ElementId, FontWeight, PathPromptOptions, div, prelude::*, px, relative,
};
use model::Id;
use std::{
	path::PathBuf,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
		mpsc,
	},
};
use tokio::sync::watch;

const TOO_MANY: &str = "Attach up to 10 files per message";
const TOO_LARGE: &str = "Attachments must total at most 500 MB; account limits may be lower";
const BUSY: &str = "Wait for the current attachment operation to finish";
/// Longest local path accepted from the picker or a drop, as `Source::inspect`.
const MAX_PATH_BYTES: usize = 4096;

/// One upload for the connection worker; it owns the only progress sender.
pub struct UploadRequest {
	pub command: Command,
	pub source: Vec<Source>,
	pub progress: watch::Sender<Status>,
	pub cancel: watch::Sender<bool>,
}

/// A chosen file. `source` is `None` only for the offline preview's synthetic files.
struct Chosen {
	filename: String,
	size: u64,
	source: Option<Source>,
}
struct Choosing {
	result: mpsc::Receiver<Result<Vec<Source>, &'static str>>,
	cancelled: Arc<AtomicBool>,
}
struct Uploading {
	progress: watch::Receiver<Status>,
	cancel: watch::Sender<bool>,
	cancelling: bool,
	files: usize,
	/// Offline preview only: stands in for the connection worker's progress sender.
	demo_hold: Option<watch::Sender<Status>>,
}

/// What the composer shows while an operation runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
	Checking,
	Uploading { files: usize, sent: u64, total: u64 },
	Sending,
	Cancelling,
}

#[derive(Default)]
pub struct Uploads {
	scope: Option<(u64, Id)>,
	selected: Vec<Chosen>,
	choosing: Option<Choosing>,
	uploading: Option<Uploading>,
	/// A native file panel is open; one at a time.
	picking: bool,
	/// One problem, announced once; drained into a transient notice.
	notice: Option<&'static str>,
	/// Offline preview: synthetic attachments for the next faked `SendResult`.
	demo_sent: Vec<model::Attachment>,
}

/// Picker and drop admission before any file is touched.
fn admit_paths(paths: &[PathBuf], already: usize) -> Result<(), &'static str> {
	if paths.is_empty() || paths.len() + already > MAX_FILES {
		return Err(TOO_MANY);
	}
	if paths.iter().any(|path| {
		!path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES
	}) {
		return Err("Drop a local file with a supported path");
	}
	Ok(())
}
/// The per-message file count and total size limits, over the kept and new sizes.
fn admit_sizes(sizes: impl IntoIterator<Item = u64>) -> Result<(), &'static str> {
	let (mut count, mut total) = (0usize, 0u64);
	for size in sizes {
		count += 1;
		total = total.saturating_add(size);
	}
	if count == 0 || count > MAX_FILES {
		return Err(TOO_MANY);
	}
	if total > MAX_TOTAL_BYTES {
		return Err(TOO_LARGE);
	}
	Ok(())
}
/// Inspects metadata only (never whole-file bytes), with the same checks as the egui app.
async fn inspect(paths: Vec<PathBuf>, cancelled: &AtomicBool) -> Result<Vec<Source>, &'static str> {
	let mut selected = Vec::with_capacity(paths.len());
	let mut total = 0u64;
	for path in paths {
		if cancelled.load(Ordering::Acquire) {
			return Ok(Vec::new());
		}
		let source = Source::inspect(path).await?;
		total = total.saturating_add(source.size());
		if total > MAX_TOTAL_BYTES {
			return Err(TOO_LARGE);
		}
		selected.push(source);
	}
	Ok(selected)
}
fn content_type(filename: &str) -> Option<&'static str> {
	let extension = filename.rsplit_once('.')?.1.to_ascii_lowercase();
	Some(match extension.as_str() {
		"png" => "image/png",
		"jpg" | "jpeg" => "image/jpeg",
		"gif" => "image/gif",
		"webp" => "image/webp",
		"mp4" => "video/mp4",
		"pdf" => "application/pdf",
		"zip" => "application/zip",
		"txt" | "md" | "log" => "text/plain",
		_ => return None,
	})
}
/// Synthetic attachment metadata for the offline preview; no URL, so nothing is fetched.
fn demo_attachments(files: &[Chosen], seed: u64) -> Vec<model::Attachment> {
	files
		.iter()
		.enumerate()
		.map(|(ix, file)| model::Attachment {
			id: Id(900_000 + seed * 16 + ix as u64),
			filename: file.filename.clone(),
			description: None,
			content_type: content_type(&file.filename).map(str::to_owned),
			size: file.size,
			media: model::EmbedMedia::default(),
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		})
		.collect()
}

impl Uploads {
	pub fn busy(&self) -> bool {
		self.choosing.is_some() || self.uploading.is_some()
	}
	pub fn has_files(&self) -> bool {
		!self.selected.is_empty()
	}
	/// Chosen files for the composer chips, only in the conversation they were chosen for.
	pub fn files(&self, generation: u64, channel: Option<Id>) -> Vec<(String, u64)> {
		if self.scope != channel.map(|id| (generation, id)) {
			return Vec::new();
		}
		self.selected
			.iter()
			.map(|c| (c.filename.clone(), c.size))
			.collect()
	}
	pub fn activity(&self) -> Option<Activity> {
		if self.choosing.is_some() {
			return Some(Activity::Checking);
		}
		let uploading = self.uploading.as_ref()?;
		if uploading.cancelling {
			return Some(Activity::Cancelling);
		}
		Some(match *uploading.progress.borrow() {
			Status::Uploading { sent, total } => Activity::Uploading {
				files: uploading.files,
				sent,
				total,
			},
			Status::Sending | Status::Finished => Activity::Sending,
			_ => Activity::Uploading {
				files: uploading.files,
				sent: 0,
				total: 0,
			},
		})
	}
	pub fn take_notice(&mut self) -> Option<&'static str> {
		self.notice.take()
	}

	/// Starts metadata inspection on a worker thread; the result arrives through `poll`.
	fn start_selection(
		&mut self,
		generation: u64,
		channel: Id,
		paths: Vec<PathBuf>,
	) -> Result<(), &'static str> {
		if self.busy() {
			return Err(BUSY);
		}
		if self.scope != Some((generation, channel)) {
			self.selected.clear();
		}
		admit_paths(&paths, self.selected.len())?;
		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let (send, result) = mpsc::sync_channel(1);
		std::thread::Builder::new()
			.name("serein-attach".into())
			.spawn(move || {
				let result = match tokio::runtime::Builder::new_current_thread().build() {
					Ok(runtime) => runtime.block_on(inspect(paths, &flag)),
					Err(_) => Err("Could not inspect the selected files"),
				};
				let _ = send.send(result);
				crate::backend::WAKE.notify_one();
			})
			.map_err(|_| "Could not inspect the selected files")?;
		self.scope = Some((generation, channel));
		self.choosing = Some(Choosing { result, cancelled });
		Ok(())
	}

	/// Applies worker results and progress; returns whether anything visible changed.
	pub fn poll(&mut self, generation: u64, channel: Option<Id>, allowed: bool) -> bool {
		let mut changed = false;
		if self
			.scope
			.is_some_and(|scope| !allowed || Some(scope) != channel.map(|id| (generation, id)))
		{
			changed = !self.selected.is_empty() || self.busy();
			self.remove();
		}
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
				changed = true;
				match result {
					_ if cancelled => {}
					Ok(sources) if sources.is_empty() => {}
					Ok(sources) => match admit_sizes(
						self.selected
							.iter()
							.map(|c| c.size)
							.chain(sources.iter().map(Source::size)),
					) {
						Ok(()) => self
							.selected
							.extend(sources.into_iter().map(|source| Chosen {
								filename: source.filename().to_owned(),
								size: source.size(),
								source: Some(source),
							})),
						Err(error) => self.notice = Some(error),
					},
					Err(error) => self.notice = Some(error),
				}
			}
		}
		if let Some(uploading) = &mut self.uploading {
			changed |= uploading.progress.has_changed().unwrap_or(true);
			let status = uploading.progress.borrow_and_update().clone();
			// The worker drops its sender when it is done; hold the slot until then.
			if uploading.progress.has_changed().is_err() {
				let problem = match status {
					Status::Sending => {
						Some("Message outcome unknown; check the conversation before retrying")
					}
					Status::Preparing | Status::Uploading { .. } if !uploading.cancelling => {
						Some("Attachment upload interrupted")
					}
					Status::Failed(error) => Some(error),
					_ => None,
				};
				if problem.is_some() {
					self.notice = problem;
				}
				self.uploading = None;
				changed = true;
			}
		}
		changed
	}
	pub fn remove_at(&mut self, index: usize) {
		if !self.busy() && index < self.selected.len() {
			self.selected.remove(index);
		}
	}
	pub fn remove(&mut self) {
		self.selected.clear();
		self.cancel();
	}
	pub fn cancel(&mut self) {
		if let Some(choosing) = &self.choosing {
			choosing.cancelled.store(true, Ordering::Release);
		}
		if let Some(uploading) = &mut self.uploading {
			uploading.cancelling = true;
			uploading.cancel.send_replace(true);
			// Offline preview: nothing else would ever release the slot.
			uploading.demo_hold = None;
		}
	}
	fn take(&mut self, generation: u64, channel: Id) -> Option<Vec<Chosen>> {
		if self.scope != Some((generation, channel)) || self.busy() || self.selected.is_empty() {
			return None;
		}
		Some(std::mem::take(&mut self.selected))
	}
	fn begin_upload(
		&mut self,
		files: usize,
		progress: watch::Receiver<Status>,
		cancel: watch::Sender<bool>,
	) -> Result<(), &'static str> {
		if self.busy() || !self.selected.is_empty() || self.scope.is_none() {
			cancel.send_replace(true);
			return Err(BUSY);
		}
		self.uploading = Some(Uploading {
			progress,
			cancel,
			cancelling: false,
			files,
			demo_hold: None,
		});
		Ok(())
	}
	/// Offline preview: synthetic chosen files, never read from disk.
	pub fn demo_select(&mut self, generation: u64, channel: Id, files: &[(&str, u64)]) {
		self.remove();
		self.scope = Some((generation, channel));
		self.selected = files
			.iter()
			.map(|(name, size)| Chosen {
				filename: (*name).to_owned(),
				size: *size,
				source: None,
			})
			.collect();
	}
	/// Offline preview: an upload frozen part-way, for the progress line screenshot.
	pub fn demo_progress(
		&mut self,
		generation: u64,
		channel: Id,
		files: usize,
		sent: u64,
		total: u64,
	) {
		self.remove();
		self.scope = Some((generation, channel));
		let (hold, progress) = watch::channel(Status::Uploading { sent, total });
		let (cancel, _) = watch::channel(false);
		if self.begin_upload(files, progress, cancel).is_ok()
			&& let Some(uploading) = &mut self.uploading
		{
			uploading.demo_hold = Some(hold);
		}
	}
	/// Offline preview: attachments for the demo `Command::Send` arm in `dispatch`.
	pub fn take_demo_sent(&mut self) -> Vec<model::Attachment> {
		std::mem::take(&mut self.demo_sent)
	}
}
impl Drop for Uploads {
	fn drop(&mut self) {
		self.cancel();
	}
}

impl Serein {
	fn attach_allowed(&self, channel: Id) -> bool {
		self.state.demo || self.state.can_attach(channel)
	}

	/// The composer "+" button: a native multi-file panel, awaited outside rendering.
	pub(crate) fn choose_files(&mut self, cx: &mut Context<Self>) {
		if self.uploads.busy() {
			self.notify_user(BUSY);
			cx.notify();
			return;
		}
		if self.uploads.picking {
			return;
		}
		self.uploads.picking = true;
		let chosen = cx.prompt_for_paths(PathPromptOptions {
			files: true,
			directories: false,
			multiple: true,
			prompt: Some("Attach".into()),
		});
		cx.spawn(async move |this, cx| {
			let chosen = chosen.await;
			let _ = this.update(cx, |this, cx| {
				this.uploads.picking = false;
				match chosen {
					Ok(Ok(Some(paths))) => this.attach_paths(paths, cx),
					Ok(Err(_)) => {
						this.notify_user("Could not open the file picker");
						cx.notify();
					}
					_ => {}
				}
			});
		})
		.detach();
	}

	/// Files from the picker or dropped on the conversation.
	pub(crate) fn attach_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
		let Some(channel) = self.state.selected else {
			return;
		};
		if !self.attach_allowed(channel) {
			self.notify_user("You do not have permission to attach files here.");
		} else if let Err(error) =
			self.uploads
				.start_selection(self.state.generation, channel, paths)
		{
			self.notify_user(error);
		}
		cx.notify();
	}

	/// Sends the chosen files with the draft; returns whether the draft was consumed.
	pub(crate) fn send_with_attachments(&mut self) -> bool {
		let Some(channel) = self.state.selected else {
			return false;
		};
		if self.uploads.busy() {
			self.notify_user(BUSY);
			return false;
		}
		let generation = self.state.generation;
		let names = self.uploads.files(generation, Some(channel));
		let names = names
			.iter()
			.map(|(name, _)| name.as_str())
			.collect::<Vec<_>>();
		if names.is_empty() {
			return false;
		}
		// Refusals surface as `State::status`, which `poll` turns into a notice.
		let Some(command) = self.state.prepare_send_with_attachments(&names) else {
			return false;
		};
		let Some(chosen) = self.uploads.take(generation, channel) else {
			return false;
		};
		if self.state.demo {
			// Never touches the network: the demo `Command::Send` arm fakes the result.
			self.uploads.demo_sent = demo_attachments(&chosen, self.state.send_sequence);
			self.dispatch(Some(command));
			return true;
		}
		let Command::Send { nonce, .. } = &command else {
			return false;
		};
		let nonce = nonce.clone();
		let source = chosen
			.into_iter()
			.map(|c| c.source)
			.collect::<Option<Vec<_>>>();
		let (progress, receive) = watch::channel(Status::Preparing);
		let (cancel, _) = watch::channel(false);
		let available = self.state.can_attach(channel) && self.state.gateway_connected;
		if let Some(source) = source.filter(|_| available)
			&& self
				.uploads
				.begin_upload(source.len(), receive, cancel.clone())
				.is_ok()
		{
			let request = UploadRequest {
				command,
				source,
				progress,
				cancel,
			};
			if let Err(error) = self.backend.uploads.try_send(request) {
				let request = error.into_inner();
				request
					.progress
					.send_replace(Status::Failed("Upload queue full; reselect the file"));
				self.state.command_rejected(request.command);
			}
			return true;
		}
		self.state.apply(Envelope {
			generation,
			event: Event::SendResult {
				nonce,
				result: Err(Failure::ProtocolAt(
					"File not sent; reconnect and reselect the attachment",
				)),
			},
		});
		true
	}
}

fn size_label(bytes: u64) -> String {
	match bytes {
		0..1024 => format!("{bytes} bytes"),
		1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.),
		_ => format!("{:.1} MB", bytes as f64 / 1_048_576.),
	}
}
fn activity_label(activity: Activity) -> String {
	match activity {
		Activity::Checking => "Checking files…".into(),
		Activity::Uploading {
			files, total: 0, ..
		} => {
			format!(
				"Preparing {files} file{}…",
				if files == 1 { "" } else { "s" }
			)
		}
		Activity::Uploading { files, sent, total } => format!(
			"Uploading {files} file{} · {} of {}",
			if files == 1 { "" } else { "s" },
			size_label(sent),
			size_label(total)
		),
		Activity::Sending => "Sending message…".into(),
		Activity::Cancelling => "Cancelling upload…".into(),
	}
}

impl Serein {
	/// Whether the composer shows the attach button for the open conversation.
	pub(crate) fn can_attach_here(&self) -> bool {
		self.state
			.selected
			.is_some_and(|channel| self.attach_allowed(channel))
	}

	/// Chosen-file cards, or the progress line while files are checked or uploaded.
	pub(crate) fn upload_tray(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
		let p = palette();
		let tray = div()
			.px(px(10.))
			.pt(px(10.))
			.pb(px(6.))
			.bg(color(p.raised))
			.border_b_1()
			.border_color(color(p.border));
		if let Some(activity) = self.uploads.activity() {
			let fraction = match activity {
				Activity::Uploading { sent, total, .. } if total > 0 => {
					(sent as f32 / total as f32).clamp(0., 1.)
				}
				Activity::Sending => 1.,
				_ => 0.,
			};
			let cancel = matches!(activity, Activity::Checking | Activity::Uploading { .. });
			return Some(
				tray.flex()
					.items_center()
					.gap(px(10.))
					.child(icon(Icon::File, px(24.), color(p.accent)))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.flex()
							.flex_col()
							.gap(px(6.))
							.child(
								div()
									.text_size(px(13.))
									.text_color(color(p.text_strong))
									.overflow_hidden()
									.whitespace_nowrap()
									.text_ellipsis()
									.child(activity_label(activity)),
							)
							.child(
								div()
									.h(px(4.))
									.w_full()
									.rounded_full()
									.bg(color(p.base))
									.child(
										div()
											.h_full()
											.w(relative(fraction))
											.rounded_full()
											.bg(color(p.accent)),
									),
							),
					)
					.when(cancel, |d| {
						d.child(
							self.icon_button("cancel-upload", Icon::Close, false, "Cancel upload")
								.size(px(28.))
								.on_click(cx.listener(|this, _, _, cx| {
									this.uploads.cancel();
									cx.notify();
								})),
						)
					})
					.into_any_element(),
			);
		}
		let files = self
			.uploads
			.files(self.state.generation, self.state.selected);
		if files.is_empty() {
			return None;
		}
		let cards = files
			.into_iter()
			.enumerate()
			.map(|(ix, (name, size))| {
				let image = content_type(&name).is_some_and(|kind| kind.starts_with("image/"));
				div()
					.id(ElementId::NamedInteger("upload-card".into(), ix as u64))
					.w(px(224.))
					.px(px(10.))
					.py(px(8.))
					.rounded(px(8.))
					.bg(color(p.base))
					.border_1()
					.border_color(color(p.border))
					.flex()
					.items_center()
					.gap(px(8.))
					.tooltip(tooltip(name.clone()))
					.child(icon(
						if image { Icon::FileImage } else { Icon::File },
						px(28.),
						color(if image { p.accent } else { p.muted }),
					))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.flex()
							.flex_col()
							.child(
								div()
									.overflow_hidden()
									.whitespace_nowrap()
									.text_ellipsis()
									.text_size(px(13.))
									.font_weight(FontWeight::MEDIUM)
									.text_color(color(p.text_strong))
									.child(name),
							)
							.child(
								div()
									.text_size(px(12.))
									.text_color(color(p.muted))
									.child(size_label(size)),
							),
					)
					.child(
						self.icon_button(
							ElementId::NamedInteger("remove-upload".into(), ix as u64),
							Icon::Close,
							false,
							"Remove attachment",
						)
						.size(px(22.))
						.on_click(cx.listener(move |this, _, _, cx| {
							this.uploads.remove_at(ix);
							cx.notify();
						})),
					)
					.into_any_element()
			})
			.collect::<Vec<_>>();
		Some(
			tray.flex()
				.flex_wrap()
				.gap(px(8.))
				.children(cards)
				.into_any_element(),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::{
		Activity, Choosing, TOO_LARGE, TOO_MANY, Uploads, admit_paths, admit_sizes, content_type,
	};
	use discord_api::upload::{MAX_FILES, MAX_TOTAL_BYTES, Source, Status};
	use model::Id;
	use std::{
		path::PathBuf,
		sync::{
			Arc,
			atomic::{AtomicBool, Ordering},
			mpsc,
		},
	};
	use tokio::sync::watch;

	fn settle(uploads: &mut Uploads, channel: Id) {
		let started = std::time::Instant::now();
		while uploads.busy() && started.elapsed() < std::time::Duration::from_secs(5) {
			uploads.poll(1, Some(channel), true);
			std::thread::yield_now();
		}
		assert!(!uploads.busy());
	}

	#[test]
	fn paths_and_sizes_keep_the_desktop_limits() {
		let file = PathBuf::from("/tmp/file.txt");
		assert_eq!(admit_paths(&[], 0), Err(TOO_MANY));
		assert!(admit_paths(std::slice::from_ref(&file), 0).is_ok());
		assert_eq!(
			admit_paths(std::slice::from_ref(&file), MAX_FILES),
			Err(TOO_MANY)
		);
		assert_eq!(admit_paths(&vec![file; MAX_FILES + 1], 0), Err(TOO_MANY));
		assert!(admit_paths(&["relative.txt".into()], 0).is_err());
		assert!(admit_paths(&[PathBuf::from("/").join("x".repeat(4096))], 0).is_err());

		assert_eq!(admit_sizes([]), Err(TOO_MANY));
		assert!(admit_sizes([1; MAX_FILES]).is_ok());
		assert_eq!(admit_sizes([1; MAX_FILES + 1]), Err(TOO_MANY));
		assert!(admit_sizes([MAX_TOTAL_BYTES]).is_ok());
		assert_eq!(admit_sizes([MAX_TOTAL_BYTES, 1]), Err(TOO_LARGE));
		assert_eq!(admit_sizes([u64::MAX, u64::MAX]), Err(TOO_LARGE));
		assert_eq!(content_type("Photo.PNG"), Some("image/png"));
		assert_eq!(content_type("archive"), None);
	}

	#[test]
	fn selection_inspects_metadata_off_thread_and_stays_scoped() {
		let path = std::env::temp_dir().join(format!(
			"serein-gpui-attach-{}-{}.txt",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::write(&path, b"synthetic attachment").unwrap();
		let mut uploads = Uploads::default();
		uploads
			.start_selection(1, Id(2), vec![path.clone()])
			.unwrap();
		assert_eq!(uploads.activity(), Some(Activity::Checking));
		assert!(
			uploads
				.start_selection(1, Id(2), vec![path.clone()])
				.is_err()
		);
		settle(&mut uploads, Id(2));
		let name = path.file_name().unwrap().to_str().unwrap().to_owned();
		assert_eq!(uploads.files(1, Some(Id(2))), vec![(name, 20)]);
		assert!(uploads.files(1, Some(Id(3))).is_empty());
		assert!(uploads.take(1, Id(3)).is_none());

		// Unsupported names and missing files are rejected by `Source::inspect`.
		uploads
			.start_selection(1, Id(2), vec![path.with_extension("missing")])
			.unwrap();
		settle(&mut uploads, Id(2));
		assert!(uploads.take_notice().is_some());
		assert_eq!(uploads.files(1, Some(Id(2))).len(), 1);

		uploads.remove_at(0);
		assert!(!uploads.has_files());
		// Navigating away before the worker answers drops its result.
		uploads
			.start_selection(1, Id(2), vec![path.clone()])
			.unwrap();
		uploads.poll(1, Some(Id(3)), true);
		settle(&mut uploads, Id(3));
		assert!(!uploads.has_files());
		std::fs::remove_file(path).unwrap();
	}

	#[test]
	fn cancelled_upload_keeps_the_slot_until_the_worker_releases_it() {
		let mut uploads = Uploads::default();
		uploads.demo_select(1, Id(2), &[("notes.pdf", 10), ("photo.png", 20)]);
		assert_eq!(uploads.files(1, Some(Id(2))).len(), 2);
		let chosen = uploads.take(1, Id(2)).unwrap();
		assert_eq!(chosen.len(), 2);
		let (progress, receive) = watch::channel(Status::Preparing);
		let (cancel, cancelled) = watch::channel(false);
		uploads.begin_upload(2, receive, cancel).unwrap();
		progress.send_replace(Status::Uploading { sent: 5, total: 30 });
		assert!(uploads.poll(1, Some(Id(2)), true));
		assert_eq!(
			uploads.activity(),
			Some(Activity::Uploading {
				files: 2,
				sent: 5,
				total: 30
			})
		);
		uploads.cancel();
		assert!(*cancelled.borrow());
		assert_eq!(uploads.activity(), Some(Activity::Cancelling));
		uploads.poll(1, Some(Id(2)), true);
		assert!(uploads.busy());
		drop(progress);
		uploads.poll(1, Some(Id(2)), true);
		assert!(!uploads.busy());
		assert_eq!(uploads.take_notice(), None);

		// A worker that stops mid-send is announced once, never kept as state.
		uploads.demo_select(1, Id(2), &[("a.txt", 1)]);
		uploads.take(1, Id(2)).unwrap();
		let (progress, receive) = watch::channel(Status::Sending);
		let (cancel, _) = watch::channel(false);
		uploads.begin_upload(1, receive, cancel).unwrap();
		drop(progress);
		uploads.poll(1, Some(Id(2)), true);
		assert!(uploads.take_notice().unwrap().contains("outcome unknown"));
		assert_eq!(uploads.take_notice(), None);

		// The demo progress line releases itself on cancel.
		uploads.demo_progress(1, Id(2), 2, 1, 4);
		assert!(uploads.busy());
		uploads.cancel();
		uploads.poll(1, Some(Id(2)), true);
		assert!(!uploads.busy());
	}

	#[test]
	fn cancelled_chooser_result_never_enters_the_selection() {
		let (send, result) = mpsc::sync_channel(1);
		let cancelled = Arc::new(AtomicBool::new(false));
		let mut uploads = Uploads::default();
		uploads.scope = Some((1, Id(2)));
		uploads.choosing = Some(Choosing {
			result,
			cancelled: cancelled.clone(),
		});
		uploads.poll(1, Some(Id(2)), false);
		assert!(cancelled.load(Ordering::Acquire));
		let source = Source::pasted_png(vec![1, 2, 3]).unwrap();
		assert!(send.send(Ok(vec![source])).is_ok());
		uploads.poll(1, Some(Id(2)), true);
		assert!(!uploads.busy());
		assert!(!uploads.has_files());
	}

	#[test]
	fn demo_state_allows_attachments_and_prepares_one_send() {
		let mut state = test_support::chat_demo_state();
		let channel = state.selected.unwrap();
		assert!(state.can_attach(channel));
		let command = state.prepare_send_with_attachments(&["notes.pdf", "photo.png"]);
		assert!(matches!(command, Some(client_core::Command::Send { .. })));
		assert_eq!(state.pending.last().unwrap().attachments.len(), 2);
	}
}
