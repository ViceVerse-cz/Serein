//! Opt-in, bounded media downloads for persistent message history.
//!
//! This module owns only the bytes for media that the owner explicitly asked to keep. It does not
//! touch the timeline, the audio callback or the message cache. A single dedicated worker streams
//! one file at a time through a private temporary file and publishes the complete file with an
//! atomic rename. Signed CDN URLs stay in memory for the request and are never copied into a
//! completion, a file name or a persistent row.
//!
//! Parent integration maps an attachment to [`MediaRequest`], calls [`MediaManager::enqueue`],
//! and drains [`MediaManager::poll`] from the existing frame loop. Rejected requests return a
//! static failure and never enter the queue.

use std::{
	collections::{HashMap, VecDeque},
	fs::{self, File, OpenOptions},
	io::Write,
	path::{Path, PathBuf},
	sync::{Arc, Condvar, Mutex, MutexGuard},
	thread::JoinHandle,
	time::{Duration, Instant},
};
use tokio::sync::Notify;

pub type MediaFailure = &'static str;

/// Total queued and active jobs accepted at once.
pub const MAX_JOBS: usize = 16;
/// Declared bytes allowed in the queue (the active job has its own reservation).
pub const MAX_QUEUED_BYTES: u64 = 32 * 1024 * 1024;
/// Declared or completed bytes allowed for one account.
pub const MAX_ACCOUNT_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_AUDIO_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: u64 = 32 * 1024 * 1024;
/// Disk and network chunks stay small so cancellation is observed promptly.
pub const CHUNK_BYTES: usize = 64 * 1024;
const MAX_COMPLETIONS: usize = 64;
const MAX_FILENAME_BYTES: usize = 1024;
const MAX_CONTENT_TYPE_BYTES: usize = 128;
const MAX_URL_BYTES: usize = 4096;
const MAX_QUERY_VALUE_BYTES: usize = 2048;
const CANCELLED: MediaFailure = "Cancelled";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
	Image,
	Audio,
	Video,
}

impl MediaKind {
	pub fn per_file_limit(self) -> u64 {
		match self {
			Self::Image => MAX_IMAGE_BYTES,
			Self::Audio => MAX_AUDIO_BYTES,
			Self::Video => MAX_VIDEO_BYTES,
		}
	}
}

struct MediaType {
	kind: MediaKind,
	content_type: &'static str,
	/// Extensions belonging to this pair, all normalized to `extension`.
	extensions: &'static [&'static str],
	extension: &'static str,
}

/// Only canonical Discord content types and their matching extensions are accepted.
const MEDIA_TYPES: [MediaType; 9] = [
	MediaType {
		kind: MediaKind::Image,
		content_type: "image/png",
		extensions: &["png"],
		extension: "png",
	},
	MediaType {
		kind: MediaKind::Image,
		content_type: "image/jpeg",
		extensions: &["jpg", "jpeg"],
		extension: "jpg",
	},
	MediaType {
		kind: MediaKind::Image,
		content_type: "image/gif",
		extensions: &["gif"],
		extension: "gif",
	},
	MediaType {
		kind: MediaKind::Image,
		content_type: "image/webp",
		extensions: &["webp"],
		extension: "webp",
	},
	MediaType {
		kind: MediaKind::Audio,
		content_type: "audio/mpeg",
		extensions: &["mp3"],
		extension: "mp3",
	},
	MediaType {
		kind: MediaKind::Audio,
		content_type: "audio/ogg",
		extensions: &["ogg"],
		extension: "ogg",
	},
	MediaType {
		kind: MediaKind::Audio,
		content_type: "audio/wav",
		extensions: &["wav"],
		extension: "wav",
	},
	MediaType {
		kind: MediaKind::Video,
		content_type: "video/mp4",
		extensions: &["mp4"],
		extension: "mp4",
	},
	MediaType {
		kind: MediaKind::Video,
		content_type: "video/webm",
		extensions: &["webm"],
		extension: "webm",
	},
];

/// One attachment the owner explicitly opted to keep. `url` is validated but is not retained in
/// any completion or output path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaRequest {
	pub account: u64,
	pub channel: u64,
	/// Set when the CDN path includes the forwarded message id.
	pub message: Option<u64>,
	pub attachment: u64,
	pub filename: String,
	pub content_type: Option<String>,
	/// Attachment metadata size. It is an upper bound for the transfer and the account budget.
	pub size: u64,
	pub url: String,
}

/// The app data directory; this module owns `message-history-media` beneath it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaConfig {
	pub data_dir: PathBuf,
}

impl MediaConfig {
	pub fn new(data_dir: impl Into<PathBuf>) -> Self {
		Self {
			data_dir: data_dir.into(),
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaProgress {
	pub job: u64,
	pub account: u64,
	pub attachment: u64,
	pub kind: MediaKind,
	pub received: u64,
	pub total: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MediaOutcome {
	Saved { path: PathBuf, bytes: u64 },
	Cancelled,
	Failed(MediaFailure),
}

/// A finished job. The signed URL is deliberately absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaCompletion {
	pub job: u64,
	pub account: u64,
	pub channel: u64,
	pub message: Option<u64>,
	pub attachment: u64,
	pub kind: MediaKind,
	pub outcome: MediaOutcome,
}

struct AdmittedMedia {
	kind: MediaKind,
	content_type: &'static str,
	extension: &'static str,
	declared: u64,
	url: url::Url,
}

struct Job {
	id: u64,
	account: u64,
	channel: u64,
	message: Option<u64>,
	attachment: u64,
	epoch: u64,
	notify: Arc<Notify>,
	admitted: AdmittedMedia,
}

struct Active {
	job: u64,
	account: u64,
	attachment: u64,
	notify: Arc<Notify>,
}

#[derive(Default)]
struct AccountState {
	epoch: u64,
	/// Reserved declared bytes plus completed file bytes.
	bytes: u64,
	scanned: bool,
}

struct State {
	queue: VecDeque<Job>,
	queued_bytes: u64,
	active: Option<Active>,
	accounts: HashMap<u64, AccountState>,
	completions: VecDeque<MediaCompletion>,
	progress: Option<MediaProgress>,
	next_job: u64,
	shutdown: bool,
	worker_finished: bool,
}

impl Default for State {
	fn default() -> Self {
		Self {
			queue: VecDeque::new(),
			queued_bytes: 0,
			active: None,
			accounts: HashMap::new(),
			completions: VecDeque::new(),
			progress: None,
			next_job: 1,
			shutdown: false,
			worker_finished: false,
		}
	}
}

struct Shared {
	root: PathBuf,
	state: Mutex<State>,
	wake: Condvar,
}

/// Owns the worker and the bounded queue. Dropping it cancels the active transfer and joins the
/// worker after its partial file is removed.
pub struct MediaManager {
	root: PathBuf,
	shared: Arc<Shared>,
	worker: Option<JoinHandle<()>>,
}

impl MediaManager {
	pub fn new(config: MediaConfig) -> Result<Self, MediaFailure> {
		if !config.data_dir.is_absolute() {
			return Err("Media storage requires an absolute application data directory");
		}
		let root = config.data_dir.join("message-history-media");
		create_private_dir_all(&root)?;
		harden_dir(&root)?;
		let shared = Arc::new(Shared {
			root: root.clone(),
			state: Mutex::new(State::default()),
			wake: Condvar::new(),
		});
		let worker_shared = shared.clone();
		let worker = std::thread::Builder::new()
			.name("serein-message-media".into())
			.spawn(move || worker(worker_shared))
			.map_err(|_| "Media worker unavailable")?;
		Ok(Self {
			root,
			shared,
			worker: Some(worker),
		})
	}

	/// Validate and queue one explicit download. The returned id can be matched with completions.
	pub fn enqueue(&self, request: MediaRequest) -> Result<u64, MediaFailure> {
		let admitted = admit(&request)?;
		let mut state = lock_state(&self.shared);
		if state.shutdown {
			return Err("Media downloads are stopping");
		}
		if state.queue.len() + usize::from(state.active.is_some()) >= MAX_JOBS {
			return Err("Too many queued media downloads");
		}
		if state
			.queue
			.iter()
			.any(|job| job.account == request.account && job.attachment == request.attachment)
			|| state.active.as_ref().is_some_and(|active| {
				active.account == request.account && active.attachment == request.attachment
			}) {
			return Err("Media download is already queued");
		}
		ensure_account_scanned(&self.root, &mut state, request.account)?;
		if state
			.queued_bytes
			.checked_add(admitted.declared)
			.is_none_or(|bytes| bytes > MAX_QUEUED_BYTES)
		{
			return Err("Queued media exceeds the byte limit");
		}
		let epoch = {
			let account = state
				.accounts
				.get_mut(&request.account)
				.expect("scanned account");
			if account
				.bytes
				.checked_add(admitted.declared)
				.is_none_or(|bytes| bytes > MAX_ACCOUNT_BYTES)
			{
				return Err("Account media exceeds the storage limit");
			}
			account.bytes += admitted.declared;
			account.epoch
		};
		let id = state.next_job;
		state.next_job = state.next_job.wrapping_add(1).max(1);
		state.queued_bytes += admitted.declared;
		state.queue.push_back(Job {
			id,
			account: request.account,
			channel: request.channel,
			message: request.message,
			attachment: request.attachment,
			epoch,
			notify: Arc::new(Notify::new()),
			admitted,
		});
		self.shared.wake.notify_one();
		Ok(id)
	}

	/// Drain completed jobs. The queue is bounded; the oldest completion is replaced if the host
	/// stops polling long enough to fill it.
	pub fn poll(&self) -> Vec<MediaCompletion> {
		let mut state = lock_state(&self.shared);
		state.completions.drain(..).collect()
	}

	pub fn progress(&self) -> Option<MediaProgress> {
		lock_state(&self.shared).progress.clone()
	}

	pub fn queued(&self) -> usize {
		lock_state(&self.shared).queue.len()
	}

	/// Cancel queued and active downloads for one account. Partial files are removed by the
	/// worker; an in-flight network read is interrupted by the cancellation notification.
	pub fn cancel_account(&self, account: u64) {
		let mut state = lock_state(&self.shared);
		cancel_account_locked(&mut state, account);
	}

	/// Cancel an account, wait for its active job to release its temporary file, then remove every
	/// owned media file for that account. Safe to call on logout or explicit history clearing.
	pub fn clear_account(&self, account: u64) -> Result<(), MediaFailure> {
		let mut state = lock_state(&self.shared);
		cancel_account_locked(&mut state, account);
		while state
			.active
			.as_ref()
			.is_some_and(|active| active.account == account)
			&& !state.worker_finished
		{
			state = self
				.shared
				.wake
				.wait(state)
				.unwrap_or_else(|poisoned| poisoned.into_inner());
		}
		if state
			.active
			.as_ref()
			.is_some_and(|active| active.account == account)
		{
			return Err("Media download is still stopping");
		}
		drop(state);
		let directory = self.root.join(account.to_string());
		match fs::remove_dir_all(&directory) {
			Ok(()) => {}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(_) => return Err("Could not clear account media"),
		}
		let mut state = lock_state(&self.shared);
		if let Some(account) = state.accounts.get_mut(&account) {
			account.bytes = 0;
			account.scanned = true;
			account.epoch = account.epoch.wrapping_add(1);
		}
		Ok(())
	}
}

impl Drop for MediaManager {
	fn drop(&mut self) {
		{
			let mut state = lock_state(&self.shared);
			state.shutdown = true;
			for account in state.accounts.values_mut() {
				account.epoch = account.epoch.wrapping_add(1);
			}
			state.queue.clear();
			state.queued_bytes = 0;
			if let Some(active) = &state.active {
				active.notify.notify_waiters();
			}
			self.shared.wake.notify_all();
		}
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

fn lock_state(shared: &Shared) -> MutexGuard<'_, State> {
	shared
		.state
		.lock()
		.unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn admit(request: &MediaRequest) -> Result<AdmittedMedia, MediaFailure> {
	if request.account == 0 || request.channel == 0 || request.attachment == 0 {
		return Err("Attachment identity is invalid");
	}
	if request.message.is_some_and(|message| message == 0) {
		return Err("Attachment message identity is invalid");
	}
	if request.size == 0 || request.size > MAX_VIDEO_BYTES {
		return Err("Attachment size is outside the media limit");
	}
	if request.filename.is_empty()
		|| request.filename.len() > MAX_FILENAME_BYTES
		|| request.filename.chars().any(|character| {
			character == '/' || character == '\\' || character == '\0' || character.is_control()
		}) {
		return Err("Attachment name is not a media file name");
	}
	if request.url.len() > MAX_URL_BYTES {
		return Err("Attachment URL is too long");
	}
	let (kind, content_type, extension) =
		classify(&request.filename, request.content_type.as_deref())?;
	let declared = request.size;
	if declared > kind.per_file_limit() {
		return Err(match kind {
			MediaKind::Image => "Image must be at most 8 MiB",
			MediaKind::Audio => "Audio must be at most 16 MiB",
			MediaKind::Video => "Video must be at most 32 MiB",
		});
	}
	let url = validate_url(request)?;
	Ok(AdmittedMedia {
		kind,
		content_type,
		extension,
		declared,
		url,
	})
}

fn classify(
	filename: &str,
	content_type: Option<&str>,
) -> Result<(MediaKind, &'static str, &'static str), MediaFailure> {
	let extension = filename
		.rsplit_once('.')
		.map(|(_, extension)| extension)
		.filter(|extension| !extension.is_empty())
		.ok_or("Attachment has no supported media extension")?;
	let content_type = content_type
		.ok_or("Attachment has no declared media content type")?
		.split(';')
		.next()
		.unwrap_or("")
		.trim();
	if content_type.is_empty() || content_type.len() > MAX_CONTENT_TYPE_BYTES {
		return Err("Attachment content type is invalid");
	}
	for media in &MEDIA_TYPES {
		if media.content_type.eq_ignore_ascii_case(content_type)
			&& media
				.extensions
				.iter()
				.any(|candidate| candidate.eq_ignore_ascii_case(extension))
		{
			return Ok((media.kind, media.content_type, media.extension));
		}
	}
	Err("Attachment content type and extension do not match a supported media pair")
}

fn validate_url(request: &MediaRequest) -> Result<url::Url, MediaFailure> {
	let mut url = url::Url::parse(&request.url).map_err(|_| "Attachment URL is invalid")?;
	if url.scheme() != "https"
		|| url.port_or_known_default() != Some(443)
		|| !url.username().is_empty()
		|| url.password().is_some()
		|| url.fragment().is_some()
	{
		return Err("Attachment URL is not an accepted Discord media origin");
	}
	match url.host_str() {
		Some("cdn.discordapp.com") => {}
		Some("media.discordapp.net") => {
			// The media host serves the same original object; normalize without changing the path
			// or signed query.
			url.set_host(Some("cdn.discordapp.com"))
				.map_err(|_| "Attachment URL host is invalid")?;
		}
		_ => return Err("Attachment URL is not an accepted Discord media origin"),
	}
	if url.query_pairs().any(|(name, value)| {
		value.len() > MAX_QUERY_VALUE_BYTES
			|| match name.as_ref() {
				"ex" | "is" | "hm" => false,
				// Storage selectors observed from the current CDN; unknown renditions stay out.
				"backend" => !matches!(value.as_ref(), "b2" | "b3"),
				_ => true,
			}
	}) {
		return Err("Attachment URL contains an unsupported query parameter");
	}
	let segments: Vec<_> = url
		.path_segments()
		.ok_or("Attachment URL path is invalid")?
		.collect();
	let (channel, message, id, encoded_name) = match segments.as_slice() {
		["attachments", channel, id, name] => (*channel, None, *id, *name),
		["attachments", channel, message, id, name] => (*channel, Some(*message), *id, *name),
		_ => return Err("Attachment URL path does not match Discord media"),
	};
	if !canonical_id(channel, request.channel) {
		return Err("Attachment URL channel does not match the attachment");
	}
	match (message, request.message) {
		(Some(segment), Some(expected)) if canonical_id(segment, expected) => {}
		(Some(_), None) | (None, None) => {}
		_ => return Err("Attachment URL message does not match the attachment"),
	}
	if !canonical_id(id, request.attachment) {
		return Err("Attachment URL id does not match the attachment");
	}
	let decoded = percent_decode(encoded_name).ok_or("Attachment URL name is invalid")?;
	if decoded != request.filename {
		return Err("Attachment URL name does not match the attachment");
	}
	Ok(url)
}

fn canonical_id(segment: &str, expected: u64) -> bool {
	expected != 0
		&& !segment.is_empty()
		&& segment.bytes().all(|byte| byte.is_ascii_digit())
		&& segment.parse::<u64>() == Ok(expected)
		&& segment == expected.to_string()
}

/// Bounded percent decoding for the one path segment. Rejects malformed escapes and any decoded
/// separator, control byte or NUL, so an encoded slash can never become a path.
fn percent_decode(encoded: &str) -> Option<String> {
	let bytes = encoded.as_bytes();
	let mut decoded = Vec::with_capacity(bytes.len());
	let mut index = 0;
	while index < bytes.len() {
		match bytes[index] {
			b'%' => {
				let high = hex(*bytes.get(index + 1)?)?;
				let low = hex(*bytes.get(index + 2)?)?;
				let byte = high << 4 | low;
				if byte == b'/' || byte == b'\\' || byte == 0 || byte.is_ascii_control() {
					return None;
				}
				decoded.push(byte);
				index += 3;
			}
			byte => {
				if byte == b'/' || byte == b'\\' || byte == 0 || byte.is_ascii_control() {
					return None;
				}
				decoded.push(byte);
				index += 1;
			}
		}
	}
	String::from_utf8(decoded).ok()
}

fn hex(byte: u8) -> Option<u8> {
	match byte {
		b'0'..=b'9' => Some(byte - b'0'),
		b'a'..=b'f' => Some(byte - b'a' + 10),
		b'A'..=b'F' => Some(byte - b'A' + 10),
		_ => None,
	}
}

fn ensure_account_scanned(
	root: &Path,
	state: &mut State,
	account: u64,
) -> Result<(), MediaFailure> {
	let entry = state.accounts.entry(account).or_default();
	if entry.scanned {
		return Ok(());
	}
	entry.bytes = account_owned_bytes(root, account)?;
	entry.scanned = true;
	Ok(())
}

fn account_owned_bytes(root: &Path, account: u64) -> Result<u64, MediaFailure> {
	let directory = root.join(account.to_string());
	let entries = match fs::read_dir(&directory) {
		Ok(entries) => entries,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
		Err(_) => return Err("Cannot inspect media storage"),
	};
	let mut total = 0u64;
	for entry in entries {
		let entry = entry.map_err(|_| "Cannot inspect media storage")?;
		let metadata =
			fs::symlink_metadata(entry.path()).map_err(|_| "Cannot inspect media storage")?;
		if metadata.is_file() {
			total = total.saturating_add(metadata.len());
			if total > MAX_ACCOUNT_BYTES {
				break;
			}
		}
	}
	Ok(total)
}

fn cancel_account_locked(state: &mut State, account: u64) {
	if let Some(entry) = state.accounts.get_mut(&account) {
		entry.epoch = entry.epoch.wrapping_add(1);
	}
	let mut remaining = VecDeque::with_capacity(state.queue.len());
	while let Some(job) = state.queue.pop_front() {
		if job.account == account {
			state.queued_bytes = state.queued_bytes.saturating_sub(job.admitted.declared);
			if let Some(entry) = state.accounts.get_mut(&account) {
				entry.bytes = entry.bytes.saturating_sub(job.admitted.declared);
			}
			push_completion(
				state,
				MediaCompletion {
					job: job.id,
					account: job.account,
					channel: job.channel,
					message: job.message,
					attachment: job.attachment,
					kind: job.admitted.kind,
					outcome: MediaOutcome::Cancelled,
				},
			);
		} else {
			remaining.push_back(job);
		}
	}
	state.queue = remaining;
	if let Some(active) = &state.active
		&& active.account == account
	{
		active.notify.notify_waiters();
	}
}

fn push_completion(state: &mut State, completion: MediaCompletion) {
	if state.completions.len() >= MAX_COMPLETIONS {
		state.completions.pop_front();
	}
	state.completions.push_back(completion);
}

fn worker(shared: Arc<Shared>) {
	let runtime = match tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
	{
		Ok(runtime) => runtime,
		Err(_) => {
			mark_worker_finished(&shared);
			return;
		}
	};
	let client = match reqwest::Client::builder()
		.no_proxy()
		.redirect(reqwest::redirect::Policy::none())
		.connect_timeout(Duration::from_secs(15))
		.read_timeout(Duration::from_secs(30))
		.timeout(Duration::from_secs(300))
		.build()
	{
		Ok(client) => client,
		Err(_) => {
			mark_worker_finished(&shared);
			return;
		}
	};
	loop {
		let Some(job) = next_job(&shared) else {
			mark_worker_finished(&shared);
			return;
		};
		let outcome = match runtime.block_on(download(&shared, &client, &job)) {
			Ok((path, bytes)) => MediaOutcome::Saved { path, bytes },
			Err(error) if error == CANCELLED => MediaOutcome::Cancelled,
			Err(error) => MediaOutcome::Failed(error),
		};
		finish_job(&shared, job, outcome);
	}
}

fn next_job(shared: &Shared) -> Option<Job> {
	let mut state = lock_state(shared);
	loop {
		if state.shutdown {
			return None;
		}
		if let Some(job) = state.queue.pop_front() {
			state.queued_bytes = state.queued_bytes.saturating_sub(job.admitted.declared);
			state.active = Some(Active {
				job: job.id,
				account: job.account,
				attachment: job.attachment,
				notify: job.notify.clone(),
			});
			state.progress = Some(MediaProgress {
				job: job.id,
				account: job.account,
				attachment: job.attachment,
				kind: job.admitted.kind,
				received: 0,
				total: job.admitted.declared,
			});
			return Some(job);
		}
		state = shared
			.wake
			.wait(state)
			.unwrap_or_else(|poisoned| poisoned.into_inner());
	}
}

fn finish_job(shared: &Shared, job: Job, outcome: MediaOutcome) {
	let mut state = lock_state(shared);
	let declared = job.admitted.declared;
	if let Some(account) = state.accounts.get_mut(&job.account) {
		account.bytes = account.bytes.saturating_sub(declared);
		if let MediaOutcome::Saved { bytes, .. } = &outcome {
			account.bytes = account.bytes.saturating_add(*bytes);
		}
	}
	if state
		.active
		.as_ref()
		.is_some_and(|active| active.job == job.id)
	{
		state.active = None;
	}
	if state
		.progress
		.as_ref()
		.is_some_and(|progress| progress.job == job.id)
	{
		state.progress = None;
	}
	push_completion(
		&mut state,
		MediaCompletion {
			job: job.id,
			account: job.account,
			channel: job.channel,
			message: job.message,
			attachment: job.attachment,
			kind: job.admitted.kind,
			outcome,
		},
	);
	shared.wake.notify_all();
}

fn mark_worker_finished(shared: &Shared) {
	let mut state = lock_state(shared);
	state.shutdown = true;
	state.worker_finished = true;
	state.active = None;
	state.progress = None;
	shared.wake.notify_all();
}

fn job_cancelled(shared: &Shared, job: &Job) -> bool {
	let state = lock_state(shared);
	state.shutdown
		|| state
			.accounts
			.get(&job.account)
			.is_none_or(|account| account.epoch != job.epoch)
}

fn set_progress(shared: &Shared, job: &Job, received: u64, total: u64) {
	let mut state = lock_state(shared);
	if state
		.active
		.as_ref()
		.is_some_and(|active| active.job == job.id)
	{
		state.progress = Some(MediaProgress {
			job: job.id,
			account: job.account,
			attachment: job.attachment,
			kind: job.admitted.kind,
			received,
			total,
		});
	}
}

async fn download(
	shared: &Shared,
	client: &reqwest::Client,
	job: &Job,
) -> Result<(PathBuf, u64), MediaFailure> {
	if job_cancelled(shared, job) {
		return Err(CANCELLED);
	}
	let directory = shared.root.join(job.account.to_string());
	let destination = directory.join(format!("{}.{}", job.attachment, job.admitted.extension));
	if let Ok(metadata) = fs::symlink_metadata(&destination) {
		if metadata.is_file()
			&& metadata.len() > 0
			&& metadata.len() <= job.admitted.declared
			&& metadata.len() <= job.admitted.kind.per_file_limit()
		{
			return Ok((destination, metadata.len()));
		}
		return Err("Existing media file does not match the attachment");
	}
	let notified = job.notify.notified();
	tokio::pin!(notified);
	notified.as_mut().enable();
	if job_cancelled(shared, job) {
		return Err(CANCELLED);
	}
	let response = tokio::select! {
		biased;
		_ = notified.as_mut() => return Err(CANCELLED),
		response = client
			.get(job.admitted.url.as_str())
			.header(reqwest::header::ACCEPT_ENCODING, "identity")
			.send() => response.map_err(|_| "Media download failed")?,
	};
	if response.status() != reqwest::StatusCode::OK {
		return Err("Media is unavailable; reload the conversation and try again");
	}
	if response
		.headers()
		.get(reqwest::header::CONTENT_ENCODING)
		.is_some_and(|encoding| encoding != "identity")
	{
		return Err("Media response used an unexpected encoding");
	}
	let response_type = response
		.headers()
		.get(reqwest::header::CONTENT_TYPE)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.split(';').next())
		.map(str::trim)
		.ok_or("Media response has no content type")?;
	if !response_type.eq_ignore_ascii_case(job.admitted.content_type) {
		return Err("Media response content type changed");
	}
	let content_length = response.content_length();
	if let Some(length) = content_length
		&& (length == 0
			|| length > job.admitted.declared
			|| length > job.admitted.kind.per_file_limit())
	{
		return Err("Media response exceeds the attachment limit");
	}
	create_private_dir_all(&directory)?;
	harden_dir(&directory)?;
	let mut partial = PartialFile::new(&directory, job.attachment, job.admitted.declared)?;
	let mut response = response;
	let mut received = 0u64;
	let mut last_progress = Instant::now();
	loop {
		if job_cancelled(shared, job) {
			return Err(CANCELLED);
		}
		let chunk = tokio::select! {
			biased;
			_ = notified.as_mut() => return Err(CANCELLED),
			chunk = response.chunk() => chunk.map_err(|_| "Media transfer interrupted")?,
		};
		let Some(chunk) = chunk else {
			break;
		};
		received = received
			.checked_add(chunk.len() as u64)
			.ok_or("Media transfer exceeds the limit")?;
		if received > job.admitted.declared || received > job.admitted.kind.per_file_limit() {
			return Err("Media transfer exceeds the attachment limit");
		}
		// reqwest may hand back a large transport chunk; split disk writes and cancellation
		// checks into the documented 64 KiB unit.
		for block in chunk.chunks(CHUNK_BYTES) {
			if job_cancelled(shared, job) {
				return Err(CANCELLED);
			}
			partial.write(block)?;
		}
		if last_progress.elapsed() >= Duration::from_millis(100) {
			set_progress(
				shared,
				job,
				received,
				content_length.unwrap_or(job.admitted.declared),
			);
			last_progress = Instant::now();
		}
	}
	if received == 0 {
		return Err("Media transfer was empty");
	}
	if let Some(length) = content_length
		&& received != length
	{
		return Err("Media transfer was incomplete");
	}
	let cancelled = job_cancelled(shared, job);
	let bytes = partial.commit(&destination, cancelled)?;
	Ok((destination, bytes))
}

struct PartialFile {
	path: PathBuf,
	file: Option<File>,
	written: u64,
	declared: u64,
	committed: bool,
}

impl PartialFile {
	fn new(directory: &Path, attachment: u64, declared: u64) -> Result<Self, MediaFailure> {
		let mut random = [0u8; 16];
		getrandom::fill(&mut random).map_err(|_| "Media temporary file unavailable")?;
		let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
		let path = directory.join(format!(".{attachment}-{suffix}.partial"));
		let mut options = OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}
		let file = options
			.open(&path)
			.map_err(|_| "Cannot create media temporary file")?;
		Ok(Self {
			path,
			file: Some(file),
			written: 0,
			declared,
			committed: false,
		})
	}

	fn write(&mut self, chunk: &[u8]) -> Result<(), MediaFailure> {
		let next = self
			.written
			.checked_add(chunk.len() as u64)
			.ok_or("Media transfer exceeds the attachment limit")?;
		if next > self.declared {
			return Err("Media transfer exceeds the attachment limit");
		}
		self.file
			.as_mut()
			.expect("partial media file")
			.write_all(chunk)
			.map_err(|_| "Cannot write media; check available disk space")?;
		self.written = next;
		Ok(())
	}

	fn commit(mut self, destination: &Path, cancelled: bool) -> Result<u64, MediaFailure> {
		if cancelled {
			return Err(CANCELLED);
		}
		let file = self.file.take().expect("partial media file");
		file.sync_all()
			.map_err(|_| "Could not finish media download")?;
		drop(file);
		fs::rename(&self.path, destination)
			.map_err(|_| "Could not publish the completed media file")?;
		self.committed = true;
		Ok(self.written)
	}
}

impl Drop for PartialFile {
	fn drop(&mut self) {
		self.file.take();
		if !self.committed {
			let _ = fs::remove_file(&self.path);
		}
	}
}

fn create_private_dir_all(directory: &Path) -> Result<(), MediaFailure> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::DirBuilderExt;
		let mut builder = fs::DirBuilder::new();
		builder.recursive(true).mode(0o700);
		builder
			.create(directory)
			.map_err(|_| "Cannot create media storage")?;
	}
	#[cfg(not(unix))]
	{
		fs::create_dir_all(directory).map_err(|_| "Cannot create media storage")?;
	}
	Ok(())
}

fn harden_dir(directory: &Path) -> Result<(), MediaFailure> {
	let metadata = fs::symlink_metadata(directory).map_err(|_| "Cannot inspect media storage")?;
	if !metadata.is_dir() || metadata.file_type().is_symlink() {
		return Err("Media storage path is not a private directory");
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
			.map_err(|_| "Cannot secure media storage")?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn request() -> MediaRequest {
		MediaRequest {
			account: 1,
			channel: 2,
			message: Some(3),
			attachment: 4,
			filename: "synthetic image.png".into(),
			content_type: Some("image/png".into()),
			size: 1024,
			url: "https://cdn.discordapp.com/attachments/2/3/4/synthetic%20image.png?ex=1&is=2&hm=abc"
				.into(),
		}
	}

	#[test]
	fn admission_rejects_host_query_path_name_and_type_mismatches() {
		assert!(admit(&request()).is_ok());
		for url in [
			"https://cdn.discordapp.com.evil.test/attachments/2/3/4/synthetic%20image.png",
			"http://cdn.discordapp.com/attachments/2/3/4/synthetic%20image.png",
			"https://user@cdn.discordapp.com/attachments/2/3/4/synthetic%20image.png",
			"https://cdn.discordapp.com:444/attachments/2/3/4/synthetic%20image.png",
			"https://cdn.discordapp.com/attachments/2/3/9/synthetic%20image.png",
			"https://cdn.discordapp.com/attachments/2/3/4/other.png",
			"https://cdn.discordapp.com/attachments/2/3/4/synthetic%20image.png?width=1",
			"https://cdn.discordapp.com/attachments/2/3/4/synthetic%20image.png#fragment",
			"https://cdn.discordapp.com/attachments/2/3/4/%2fimage.png",
			"https://cdn.discordapp.com/attachments/2/3/4/synthetic%20image.png?backend=b4",
			"https://cdn.discordapp.com/attachments/02/3/4/synthetic%20image.png",
		] {
			let mut request = request();
			request.url = url.into();
			assert!(admit(&request).is_err(), "{url}");
		}
		let mut media = request();
		media.url =
			"https://media.discordapp.net/attachments/2/3/4/synthetic%20image.png?backend=b2&ex=1&is=2&hm=abc"
				.into();
		let admitted = admit(&media).unwrap();
		assert_eq!(admitted.url.host_str(), Some("cdn.discordapp.com"));
		assert_eq!(admitted.extension, "png");
		for (filename, content_type) in [
			("synthetic.png", "image/avif"),
			("synthetic.svg", "image/svg+xml"),
			("synthetic.png", "image/jpeg"),
			("synthetic.png", "application/octet-stream"),
			("synthetic.mp3", "image/png"),
			("synthetic.exe", "application/pdf"),
		] {
			let mut request = request();
			request.filename = filename.into();
			request.content_type = Some(content_type.into());
			if filename == "synthetic.mp3" {
				request.url = "https://cdn.discordapp.com/attachments/2/3/4/synthetic.mp3".into();
			}
			assert!(admit(&request).is_err(), "{filename}/{content_type}");
		}
		for (filename, content_type, limit) in [
			("a.jpeg", "image/jpeg", MAX_IMAGE_BYTES),
			("a.gif", "image/gif", MAX_IMAGE_BYTES),
			("a.webp", "image/webp", MAX_IMAGE_BYTES),
			("a.mp3", "audio/mpeg", MAX_AUDIO_BYTES),
			("a.ogg", "audio/ogg", MAX_AUDIO_BYTES),
			("a.wav", "audio/wav", MAX_AUDIO_BYTES),
			("a.mp4", "video/mp4", MAX_VIDEO_BYTES),
			("a.webm", "video/webm", MAX_VIDEO_BYTES),
		] {
			let mut request = request();
			request.filename = filename.into();
			request.content_type = Some(content_type.into());
			request.size = limit;
			request.message = None;
			request.url = format!("https://cdn.discordapp.com/attachments/2/3/4/{filename}");
			assert!(admit(&request).is_ok(), "{filename}");
			request.size = limit + 1;
			assert!(admit(&request).is_err(), "{filename}");
		}
		let mut request = request();
		request.filename = "../../escape.png".into();
		assert!(admit(&request).is_err());
	}

	#[test]
	fn byte_limit_cancellation_and_failed_publish_remove_partial_files() {
		let directory =
			std::env::temp_dir().join(format!("serein-media-bounds-{}", std::process::id()));
		let _ = fs::remove_dir_all(&directory);
		create_private_dir_all(&directory).unwrap();
		let mut partial = PartialFile::new(&directory, 7, 8).unwrap();
		partial.write(&[1; 8]).unwrap();
		assert!(partial.write(&[2]).is_err());
		drop(partial);
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);

		let mut partial = PartialFile::new(&directory, 7, 8).unwrap();
		partial.write(&[1; 4]).unwrap();
		let destination = directory.join("7.png");
		assert_eq!(partial.commit(&destination, true), Err(CANCELLED));
		assert!(!destination.exists());
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);

		let mut partial = PartialFile::new(&directory, 8, 8).unwrap();
		partial.write(&[1; 8]).unwrap();
		let bytes = partial.commit(&destination, false).unwrap();
		assert_eq!(bytes, 8);
		assert_eq!(fs::read(&destination).unwrap(), vec![1; 8]);
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_eq!(
				fs::metadata(&destination).unwrap().permissions().mode() & 0o077,
				0
			);
			assert_eq!(
				fs::metadata(&directory).unwrap().permissions().mode() & 0o077,
				0
			);
		}
		let _ = fs::remove_dir_all(directory);
	}

	#[test]
	fn account_usage_scan_is_bounded_to_regular_files() {
		let root = std::env::temp_dir().join(format!("serein-media-scan-{}", std::process::id()));
		let _ = fs::remove_dir_all(&root);
		create_private_dir_all(&root).unwrap();
		assert_eq!(account_owned_bytes(&root, 5).unwrap(), 0);
		let directory = root.join("5");
		create_private_dir_all(&directory).unwrap();
		fs::write(directory.join("1.png"), vec![0; 10]).unwrap();
		fs::write(directory.join(".1-x.partial"), vec![0; 5]).unwrap();
		assert_eq!(account_owned_bytes(&root, 5).unwrap(), 15);
		let _ = fs::remove_dir_all(root);
	}
}
