//! Watches a user-managed live theme package outside Serein's installed-theme store.
//!
//! External theme tools atomically replace `<data_dir>/extensions/live-theme.serein-extension`.
//! Polling and decoding stay off the UI thread, bounded by the existing 16 MiB package and
//! static-image decode limits.

use crate::extensions::{MAX_PACKAGE, digest, package_background};
use extensions::{ExtensionKind, Theme};
use std::{
	fs::{self, File},
	io::{ErrorKind, Read},
	path::Path,
	sync::{Arc, Mutex, mpsc},
	thread::JoinHandle,
	time::{Duration, Instant, SystemTime},
};

pub(crate) const LIVE_THEME_FILE: &str = "live-theme.serein-extension";
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Retries for a deterministic content/validation failure whose file stamp has not changed.
/// Covers non-atomic writers on coarse-timestamp filesystems without spinning forever.
const MAX_CONTENT_RETRIES: u8 = 3;
/// Cap on exponential backoff for transient open/read errors (e.g. Windows sharing violations
/// or temporary permission changes).
const MAX_IO_BACKOFF: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
enum LiveThemeStamp {
	Missing,
	NotRegularFile,
	Unreadable,
	Present(FileStamp),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
	bytes: u64,
	modified: Option<SystemTime>,
	#[cfg(unix)]
	dev: u64,
	#[cfg(unix)]
	ino: u64,
	#[cfg(unix)]
	ctime: i64,
	#[cfg(unix)]
	ctime_nsec: i64,
	#[cfg(windows)]
	creation_time: u64,
	#[cfg(windows)]
	last_write_time: u64,
	#[cfg(windows)]
	attributes: u32,
	#[cfg(not(unix))]
	directory_modified: Option<SystemTime>,
}

fn live_theme_stamp(path: &Path) -> LiveThemeStamp {
	match fs::metadata(path) {
		Err(error) if error.kind() == ErrorKind::NotFound => LiveThemeStamp::Missing,
		Err(_) => LiveThemeStamp::Unreadable,
		Ok(metadata) if metadata.is_file() => LiveThemeStamp::Present(file_stamp(path, &metadata)),
		Ok(_) => LiveThemeStamp::NotRegularFile,
	}
}

#[cfg(unix)]
fn file_stamp(_path: &Path, metadata: &fs::Metadata) -> FileStamp {
	use std::os::unix::fs::MetadataExt;
	FileStamp {
		bytes: metadata.len(),
		modified: metadata.modified().ok(),
		dev: metadata.dev(),
		ino: metadata.ino(),
		ctime: metadata.ctime(),
		ctime_nsec: metadata.ctime_nsec(),
	}
}

#[cfg(windows)]
fn file_stamp(path: &Path, metadata: &fs::Metadata) -> FileStamp {
	use std::os::windows::fs::MetadataExt;
	FileStamp {
		bytes: metadata.len(),
		modified: metadata.modified().ok(),
		creation_time: metadata.creation_time(),
		last_write_time: metadata.last_write_time(),
		attributes: metadata.file_attributes(),
		directory_modified: parent_modified(path),
	}
}

#[cfg(all(not(unix), not(windows)))]
fn file_stamp(path: &Path, metadata: &fs::Metadata) -> FileStamp {
	FileStamp {
		bytes: metadata.len(),
		modified: metadata.modified().ok(),
		directory_modified: parent_modified(path),
	}
}

#[cfg(not(unix))]
fn parent_modified(path: &Path) -> Option<SystemTime> {
	path.parent().and_then(|parent| {
		fs::metadata(parent)
			.ok()
			.and_then(|metadata| metadata.modified().ok())
	})
}

pub(crate) enum LiveThemeUpdate {
	Loaded {
		theme: Box<Theme>,
		background_image: Option<Arc<eframe::egui::ColorImage>>,
	},
	Cleared,
}

#[derive(Default)]
pub(crate) struct LiveThemeChanges {
	pub update: Option<LiveThemeUpdate>,
	pub error: Option<String>,
}

enum LiveThemeLoad {
	Loaded {
		update: LiveThemeUpdate,
		digest: Option<String>,
	},
	Unchanged,
}

enum LoadFailure {
	/// File disappeared between `fs::metadata` and `File::open`.
	Vanished,
	/// Transient OS open/read failure (e.g. sharing violation or temporary permissions).
	TransientIo(String),
	/// Deterministic validation, size, image, or file-type error.
	Content(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FailureKind {
	TransientIo,
	Content,
}

struct FailureState {
	stamp: LiveThemeStamp,
	kind: FailureKind,
	message: String,
	attempts: u8,
	next_retry: Option<Instant>,
}

struct LiveThemeState {
	poll_interval: Duration,
	observed: LiveThemeStamp,
	observed_digest: Option<String>,
	failure: Option<FailureState>,
}

impl LiveThemeState {
	fn new(poll_interval: Duration) -> Self {
		Self {
			poll_interval,
			observed: LiveThemeStamp::Missing,
			observed_digest: None,
			failure: None,
		}
	}

	fn poll_path(&mut self, path: &Path, now: Instant, pending: &mut LiveThemeChanges) -> bool {
		self.poll_with(
			now,
			pending,
			|| live_theme_stamp(path),
			|previous_digest| load_live_theme(path, previous_digest),
		)
	}

	fn poll_with(
		&mut self,
		now: Instant,
		pending: &mut LiveThemeChanges,
		mut stamp_fn: impl FnMut() -> LiveThemeStamp,
		mut load_fn: impl FnMut(Option<&str>) -> Result<LiveThemeLoad, LoadFailure>,
	) -> bool {
		let stamp = stamp_fn();
		if stamp == self.observed {
			if self.failure.take().is_some() {
				pending.error = None;
			}
			return false;
		}
		if let Some(failure) = &self.failure {
			if failure.stamp == stamp {
				match failure.next_retry {
					None => return false,
					Some(retry_at) if now < retry_at => return false,
					Some(_) => {}
				}
			} else {
				self.failure = None;
			}
		}

		match stamp {
			LiveThemeStamp::Missing => {
				self.observed = LiveThemeStamp::Missing;
				self.observed_digest = None;
				self.failure = None;
				pending.update = Some(LiveThemeUpdate::Cleared);
				pending.error = None;
				true
			}
			LiveThemeStamp::NotRegularFile => self.record_failure(
				stamp,
				FailureKind::Content,
				"Live theme package must be a regular file".into(),
				None,
				pending,
			),
			LiveThemeStamp::Unreadable => self.record_transient_io(
				stamp,
				"Live theme package cannot be read".into(),
				now,
				pending,
			),
			LiveThemeStamp::Present(_) => match load_fn(self.observed_digest.as_deref()) {
				Ok(LiveThemeLoad::Loaded { update, digest }) => {
					self.observed = stamp;
					self.observed_digest = digest;
					self.failure = None;
					pending.update = Some(update);
					pending.error = None;
					true
				}
				Ok(LiveThemeLoad::Unchanged) => {
					self.observed = stamp;
					self.failure = None;
					pending.error = None;
					false
				}
				Err(LoadFailure::Vanished) => {
					let after = stamp_fn();
					if after == LiveThemeStamp::Missing {
						self.failure = None;
						pending.error = None;
						if self.observed != LiveThemeStamp::Missing {
							self.observed = LiveThemeStamp::Missing;
							self.observed_digest = None;
							pending.update = Some(LiveThemeUpdate::Cleared);
							return true;
						}
					}
					false
				}
				Err(LoadFailure::TransientIo(message)) => {
					if stamp_fn() != stamp {
						return false;
					}
					self.record_transient_io(stamp, message, now, pending)
				}
				Err(LoadFailure::Content(message)) => {
					if stamp_fn() != stamp {
						return false;
					}
					let attempts = self
						.failure
						.as_ref()
						.filter(|f| f.stamp == stamp && f.kind == FailureKind::Content)
						.map_or(1, |f| f.attempts.saturating_add(1));
					let next_retry = (attempts <= MAX_CONTENT_RETRIES)
						.then(|| now + retry_delay(self.poll_interval, attempts, MAX_IO_BACKOFF));
					self.record_failure(stamp, FailureKind::Content, message, next_retry, pending)
				}
			},
		}
	}

	fn record_transient_io(
		&mut self,
		stamp: LiveThemeStamp,
		message: String,
		now: Instant,
		pending: &mut LiveThemeChanges,
	) -> bool {
		let attempts = self
			.failure
			.as_ref()
			.filter(|f| f.stamp == stamp && f.kind == FailureKind::TransientIo)
			.map_or(1, |f| f.attempts.saturating_add(1));
		let next_retry = Some(now + retry_delay(self.poll_interval, attempts, MAX_IO_BACKOFF));
		self.record_failure(
			stamp,
			FailureKind::TransientIo,
			message,
			next_retry,
			pending,
		)
	}

	fn record_failure(
		&mut self,
		stamp: LiveThemeStamp,
		kind: FailureKind,
		message: String,
		next_retry: Option<Instant>,
		pending: &mut LiveThemeChanges,
	) -> bool {
		let already_reported = self
			.failure
			.as_ref()
			.is_some_and(|f| f.stamp == stamp && f.message == message);
		let attempts = self
			.failure
			.as_ref()
			.filter(|f| f.stamp == stamp && f.kind == kind)
			.map_or(1, |f| f.attempts.saturating_add(1));
		self.failure = Some(FailureState {
			stamp,
			kind,
			message: message.clone(),
			attempts,
			next_retry,
		});
		if already_reported {
			false
		} else {
			pending.error = Some(message);
			true
		}
	}
}

fn retry_delay(base: Duration, attempt: u8, cap: Duration) -> Duration {
	let shift = u32::from(attempt.saturating_sub(1)).min(16);
	base.saturating_mul(1u32 << shift).min(cap)
}

/// Watches the user-managed live theme package without exposing Serein's installed-package store.
pub(crate) struct LiveThemeWatcher {
	pending: Arc<Mutex<LiveThemeChanges>>,
	stop: Option<mpsc::Sender<()>>,
	thread: Option<JoinHandle<()>>,
}

impl LiveThemeWatcher {
	pub fn new(root: &Path, context: eframe::egui::Context) -> Result<Self, String> {
		Self::spawn(root, context, POLL_INTERVAL, None)
	}

	fn spawn(
		root: &Path,
		context: eframe::egui::Context,
		poll_interval: Duration,
		notify: Option<mpsc::Sender<()>>,
	) -> Result<Self, String> {
		let path = root.join(LIVE_THEME_FILE);
		let pending = Arc::new(Mutex::new(LiveThemeChanges::default()));
		let changes = pending.clone();
		let (stop, stopped) = mpsc::channel();
		let thread = std::thread::Builder::new()
			.name("serein-live-theme".into())
			.spawn(move || {
				let mut state = LiveThemeState::new(poll_interval);
				loop {
					let had_failure = state.failure.is_some();
					let mut batch = LiveThemeChanges::default();
					let repaint = state.poll_path(&path, Instant::now(), &mut batch);
					let clear_error =
						had_failure && state.failure.is_none() && batch.error.is_none();
					if (batch.update.is_some() || batch.error.is_some() || clear_error)
						&& let Ok(mut pending) = changes.lock()
					{
						if let Some(update) = batch.update {
							pending.update = Some(update);
							pending.error = None;
						}
						if let Some(error) = batch.error {
							pending.error = Some(error);
						} else if clear_error {
							pending.error = None;
						}
					}
					if repaint {
						context.request_repaint();
						if let Some(notify) = &notify {
							let _ = notify.send(());
						}
					}
					match stopped.recv_timeout(poll_interval) {
						Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
						Err(mpsc::RecvTimeoutError::Timeout) => {}
					}
				}
			})
			.map_err(|_| "Cannot start live theme watcher".to_owned())?;
		Ok(Self {
			pending,
			stop: Some(stop),
			thread: Some(thread),
		})
	}

	pub fn take_changes(&self) -> LiveThemeChanges {
		self.pending
			.lock()
			.map(|mut pending| std::mem::take(&mut *pending))
			.unwrap_or_default()
	}
}

impl Drop for LiveThemeWatcher {
	fn drop(&mut self) {
		self.stop.take();
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
	}
}

fn read_live_theme_bytes(path: &Path, limit: usize) -> Result<Vec<u8>, LoadFailure> {
	let file = File::open(path).map_err(|error| {
		if error.kind() == ErrorKind::NotFound {
			LoadFailure::Vanished
		} else {
			LoadFailure::TransientIo("Cannot read live theme package".into())
		}
	})?;
	let metadata = file
		.metadata()
		.map_err(|_| LoadFailure::TransientIo("Cannot inspect live theme package".into()))?;
	if !metadata.is_file() {
		return Err(LoadFailure::Content(
			"Live theme package must be a regular file".into(),
		));
	}
	if metadata.len() > limit as u64 {
		return Err(LoadFailure::Content(
			"Live theme package exceeds 16 MiB".into(),
		));
	}
	let mut bytes = Vec::new();
	file.take(limit as u64 + 1)
		.read_to_end(&mut bytes)
		.map_err(|_| LoadFailure::TransientIo("Cannot read live theme package".into()))?;
	if bytes.len() > limit {
		return Err(LoadFailure::Content(
			"Live theme package exceeds 16 MiB".into(),
		));
	}
	Ok(bytes)
}

fn load_live_theme(
	path: &Path,
	previous_digest: Option<&str>,
) -> Result<LiveThemeLoad, LoadFailure> {
	load_live_theme_with_limit(path, previous_digest, MAX_PACKAGE)
}

fn load_live_theme_with_limit(
	path: &Path,
	previous_digest: Option<&str>,
	limit: usize,
) -> Result<LiveThemeLoad, LoadFailure> {
	let bytes = read_live_theme_bytes(path, limit)?;
	let current_digest = digest(&bytes);
	if previous_digest == Some(current_digest.as_str()) {
		return Ok(LiveThemeLoad::Unchanged);
	}
	let package = extensions::parse_package(&bytes)
		.map_err(|_| LoadFailure::Content("Live theme package is invalid".into()))?;
	if package.manifest.kind != ExtensionKind::Theme {
		return Err(LoadFailure::Content(
			"Live theme file must contain a theme package".into(),
		));
	}
	let background_image = package_background(&package).map_err(LoadFailure::Content)?;
	let theme = package
		.theme
		.ok_or_else(|| LoadFailure::Content("Live theme package has no theme".into()))?;
	Ok(LiveThemeLoad::Loaded {
		update: LiveThemeUpdate::Loaded {
			theme: Box::new(theme),
			background_image,
		},
		digest: Some(current_digest),
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use extensions::{Manifest, Package};
	use std::{cell::Cell, path::PathBuf};

	struct TempDir(PathBuf);

	impl TempDir {
		fn new() -> Self {
			let mut nonce = [0; 16];
			getrandom::fill(&mut nonce).unwrap();
			let path =
				std::env::temp_dir().join(format!("serein-live-theme-test-{}", digest(&nonce)));
			fs::create_dir(&path).unwrap();
			Self(path)
		}
	}

	impl Drop for TempDir {
		fn drop(&mut self) {
			let _ = fs::remove_dir_all(&self.0);
		}
	}

	fn sample_theme(accent: &str) -> Package {
		let mut theme = Theme::default();
		theme.dark.colors.insert("accent".into(), accent.into());
		Package {
			manifest: Manifest {
				api_version: extensions::API_VERSION,
				id: "live-theme".into(),
				name: "Live Theme".into(),
				version: "1.0.0".into(),
				author: "Synthetic test".into(),
				license: "MIT".into(),
				source: "https://example.org/live-theme".into(),
				kind: ExtensionKind::Theme,
				capabilities: Vec::new(),
				actions: Vec::new(),
			},
			theme: Some(theme),
			background_image: Vec::new(),
			cover_image: Vec::new(),
			wasm: Vec::new(),
		}
	}

	fn fake_stamp(id: u64) -> LiveThemeStamp {
		LiveThemeStamp::Present(FileStamp {
			bytes: id,
			modified: None,
			#[cfg(unix)]
			dev: 1,
			#[cfg(unix)]
			ino: id,
			#[cfg(unix)]
			ctime: 0,
			#[cfg(unix)]
			ctime_nsec: 0,
			#[cfg(windows)]
			creation_time: id,
			#[cfg(windows)]
			last_write_time: id,
			#[cfg(windows)]
			attributes: 0,
			#[cfg(not(unix))]
			directory_modified: None,
		})
	}

	fn loaded_result(accent: &str, hash: &str) -> Result<LiveThemeLoad, LoadFailure> {
		let mut theme = Theme::default();
		theme.dark.colors.insert("accent".into(), accent.into());
		Ok(LiveThemeLoad::Loaded {
			update: LiveThemeUpdate::Loaded {
				theme: Box::new(theme),
				background_image: None,
			},
			digest: Some(hash.into()),
		})
	}

	#[test]
	fn state_machine_handles_unread_updates_torn_reads_toctou_and_bounded_retries() {
		let mut state = LiveThemeState::new(POLL_INTERVAL);
		let mut pending = LiveThemeChanges::default();
		let t0 = Instant::now();

		// Initial missing file stays quiet with no spurious Cleared or repaint.
		assert!(!state.poll_with(
			t0,
			&mut pending,
			|| LiveThemeStamp::Missing,
			|_| unreachable!()
		));
		assert!(pending.update.is_none());
		assert!(pending.error.is_none());

		// Valid load followed by an invalid write before UI drains preserves both slots.
		assert!(state.poll_with(
			t0,
			&mut pending,
			|| fake_stamp(1),
			|_| loaded_result("#112233", "hash-a"),
		));
		assert!(state.poll_with(
			t0,
			&mut pending,
			|| fake_stamp(2),
			|_| Err(LoadFailure::Content("bad json".into())),
		));
		assert!(matches!(
			pending.update,
			Some(LiveThemeUpdate::Loaded { .. })
		));
		assert_eq!(pending.error.as_deref(), Some("bad json"));
		let _ = std::mem::take(&mut pending);

		// Persistent content failure stops reading after MAX_CONTENT_RETRIES until stamp changes.
		let reads = Cell::new(0u32);
		let mut now = t0;
		for _ in 0..12 {
			now += Duration::from_secs(1);
			let _ = state.poll_with(
				now,
				&mut pending,
				|| fake_stamp(2),
				|_| {
					reads.set(reads.get() + 1);
					Err(LoadFailure::Content("bad json".into()))
				},
			);
		}
		assert_eq!(
			reads.get(),
			u32::from(MAX_CONTENT_RETRIES),
			"content retries must stop after the bounded limit"
		);
		assert!(
			pending.error.is_none(),
			"identical failure must not re-report"
		);

		// Torn read (stamp changes during failed load) suppresses bogus error.
		let stamp_calls = Cell::new(0u8);
		assert!(!state.poll_with(
			now,
			&mut pending,
			|| {
				let call = stamp_calls.get();
				stamp_calls.set(call + 1);
				if call == 0 {
					fake_stamp(3)
				} else {
					fake_stamp(4)
				}
			},
			|_| Err(LoadFailure::Content("partial write".into())),
		));
		assert!(pending.error.is_none());

		// Unchanged SHA-256 on a new stamp updates observed without waking UI.
		assert!(!state.poll_with(
			now,
			&mut pending,
			|| fake_stamp(4),
			|prev| {
				assert_eq!(prev, Some("hash-a"));
				Ok(LiveThemeLoad::Unchanged)
			},
		));
		assert!(pending.update.is_none());

		// Transient I/O failure (e.g. Windows sharing violation / permissions) retries with
		// capped backoff and recovers on the exact same stamp without a metadata change.
		let io_reads = Cell::new(0u32);
		assert!(state.poll_with(
			now,
			&mut pending,
			|| fake_stamp(5),
			|_| {
				io_reads.set(io_reads.get() + 1);
				Err(LoadFailure::TransientIo("locked".into()))
			},
		));
		assert_eq!(pending.error.as_deref(), Some("locked"));
		let _ = std::mem::take(&mut pending);
		// Before backoff elapses, no retry runs.
		assert!(!state.poll_with(
			now + Duration::from_millis(100),
			&mut pending,
			|| fake_stamp(5),
			|_| {
				io_reads.set(io_reads.get() + 1);
				loaded_result("#445566", "hash-b")
			},
		));
		assert_eq!(io_reads.get(), 1);
		// Once backoff elapses, same stamp retries and loads cleanly.
		now += POLL_INTERVAL;
		assert!(state.poll_with(
			now,
			&mut pending,
			|| fake_stamp(5),
			|_| {
				io_reads.set(io_reads.get() + 1);
				loaded_result("#445566", "hash-b")
			},
		));
		assert_eq!(io_reads.get(), 2);
		assert!(matches!(
			pending.update,
			Some(LiveThemeUpdate::Loaded { .. })
		));
		assert!(pending.error.is_none());
		let _ = std::mem::take(&mut pending);

		// TOCTOU deletion between stat and open clears cleanly without reporting an error.
		let vanished_calls = Cell::new(0u8);
		assert!(state.poll_with(
			now,
			&mut pending,
			|| {
				let call = vanished_calls.get();
				vanished_calls.set(call + 1);
				if call == 0 {
					fake_stamp(6)
				} else {
					LiveThemeStamp::Missing
				}
			},
			|_| Err(LoadFailure::Vanished),
		));
		assert!(matches!(pending.update, Some(LiveThemeUpdate::Cleared)));
		assert!(pending.error.is_none());

		// An error after an unread Cleared preserves Cleared, and removing the broken file
		// resets failure deduplication so a later identical invalid write reports again.
		assert!(state.poll_with(
			now,
			&mut pending,
			|| fake_stamp(7),
			|_| Err(LoadFailure::Content("broken".into())),
		));
		assert!(matches!(pending.update, Some(LiveThemeUpdate::Cleared)));
		assert_eq!(pending.error.as_deref(), Some("broken"));
		let _ = std::mem::take(&mut pending);

		assert!(!state.poll_with(
			now,
			&mut pending,
			|| LiveThemeStamp::Missing,
			|_| unreachable!(),
		));
		assert!(state.poll_with(
			now,
			&mut pending,
			|| fake_stamp(7),
			|_| Err(LoadFailure::Content("broken".into())),
		));
		assert_eq!(pending.error.as_deref(), Some("broken"));
	}

	#[test]
	fn filesystem_poll_loads_backgrounds_rejects_invalid_packages_and_handles_atomic_replace() {
		let dir = TempDir::new();
		let path = dir.0.join(LIVE_THEME_FILE);
		let mut state = LiveThemeState::new(POLL_INTERVAL);
		let mut pending = LiveThemeChanges::default();
		let now = Instant::now();

		// Valid theme with an embedded PNG background loads and decodes.
		let mut first = sample_theme("#112233");
		first.background_image = include_bytes!("../../../extensions/previews/ocean.png").to_vec();
		fs::write(&path, serde_json::to_vec(&first).unwrap()).unwrap();
		assert!(state.poll_path(&path, now, &mut pending));
		let Some(LiveThemeUpdate::Loaded {
			theme,
			background_image,
		}) = pending.update.take()
		else {
			panic!("expected initial live theme");
		};
		assert_eq!(theme.dark.colors["accent"], "#112233");
		assert!(background_image.is_some());
		assert!(pending.error.is_none());

		// Non-theme plugin package is rejected and preserves the previous theme.
		let plugin_bytes = include_bytes!(
			"../../../examples/extensions/packages/message-counter.serein-extension"
		);
		let tmp = dir.0.join("live-theme.tmp");
		fs::write(&tmp, plugin_bytes).unwrap();
		fs::rename(&tmp, &path).unwrap();
		assert!(state.poll_path(&path, now, &mut pending));
		assert!(pending.update.is_none());
		assert_eq!(
			pending.error.take().as_deref(),
			Some("Live theme file must contain a theme package")
		);

		// Oversized package fails closed before reading past the limit.
		let oversized = dir.0.join("oversized.json");
		fs::write(&oversized, b"12345").unwrap();
		assert!(matches!(
			load_live_theme_with_limit(&oversized, None, 4),
			Err(LoadFailure::Content(_))
		));

		// Atomic replacement with a valid same-size theme (different hex color) reloads.
		let base = sample_theme("#223344");
		let base_bytes = serde_json::to_vec(&base).unwrap();
		fs::write(&tmp, &base_bytes).unwrap();
		fs::rename(&tmp, &path).unwrap();
		assert!(state.poll_path(&path, now, &mut pending));
		let Some(LiveThemeUpdate::Loaded { theme, .. }) = pending.update.take() else {
			panic!("expected base live theme");
		};
		assert_eq!(theme.dark.colors["accent"], "#223344");

		let original_mtime = fs::metadata(&path).unwrap().modified().ok();
		let next = sample_theme("#556677");
		let next_bytes = serde_json::to_vec(&next).unwrap();
		assert_eq!(base_bytes.len(), next_bytes.len());
		{
			use std::io::Write;
			let mut file = File::create(&tmp).unwrap();
			file.write_all(&next_bytes).unwrap();
			if let Some(mtime) = original_mtime {
				let _ = file.set_modified(mtime);
			}
		}
		fs::rename(&tmp, &path).unwrap();
		#[cfg(not(unix))]
		if live_theme_stamp(&path) == state.observed {
			// Coarse Windows timer ticks + NTFS tunneling can match within <15 ms.
			let newer = SystemTime::now() + Duration::from_secs(2);
			let _ = File::options()
				.write(true)
				.open(&path)
				.and_then(|f| f.set_modified(newer));
		}
		assert!(state.poll_path(&path, now, &mut pending));
		let Some(LiveThemeUpdate::Loaded { theme, .. }) = pending.update.take() else {
			panic!("expected same-size atomic replacement to reload");
		};
		assert_eq!(theme.dark.colors["accent"], "#556677");

		// Directory at the live theme path is rejected without attempting File::open.
		fs::remove_file(&path).unwrap();
		assert!(state.poll_path(&path, now, &mut pending));
		assert!(matches!(
			pending.update.take(),
			Some(LiveThemeUpdate::Cleared)
		));
		fs::create_dir(&path).unwrap();
		assert!(state.poll_path(&path, now, &mut pending));
		assert_eq!(
			pending.error.take().as_deref(),
			Some("Live theme package must be a regular file")
		);
		// Second poll on unchanged directory stays silent (no retry spin).
		assert!(!state.poll_path(&path, now + Duration::from_secs(10), &mut pending));
		assert!(pending.error.is_none());
		fs::remove_dir(&path).unwrap();
	}

	#[cfg(unix)]
	#[test]
	fn unix_symlinks_and_neighboring_writes_behave_cleanly() {
		use std::os::unix::fs::symlink;

		let dir = TempDir::new();
		let target_a = dir.0.join("theme-a.serein-extension");
		let target_b = dir.0.join("theme-b.serein-extension");
		let link = dir.0.join(LIVE_THEME_FILE);
		let link_tmp = dir.0.join("live-theme.link.tmp");

		fs::write(
			&target_a,
			serde_json::to_vec(&sample_theme("#112233")).unwrap(),
		)
		.unwrap();
		fs::write(
			&target_b,
			serde_json::to_vec(&sample_theme("#445566")).unwrap(),
		)
		.unwrap();
		symlink(&target_a, &link).unwrap();

		let mut state = LiveThemeState::new(POLL_INTERVAL);
		let mut pending = LiveThemeChanges::default();
		let now = Instant::now();
		assert!(state.poll_path(&link, now, &mut pending));
		let Some(LiveThemeUpdate::Loaded { theme, .. }) = pending.update.take() else {
			panic!("expected symlinked live theme to load");
		};
		assert_eq!(theme.dark.colors["accent"], "#112233");

		// Neighboring writes in extensions/ do not alter the Unix file stamp.
		let before = live_theme_stamp(&link);
		fs::write(dir.0.join("catalog.json"), b"{}").unwrap();
		assert_eq!(live_theme_stamp(&link), before);
		assert!(!state.poll_path(&link, now, &mut pending));

		// Atomic symlink swap (e.g. Nix/Home Manager) changes target inode and reloads.
		symlink(&target_b, &link_tmp).unwrap();
		fs::rename(&link_tmp, &link).unwrap();
		assert!(state.poll_path(&link, now, &mut pending));
		let Some(LiveThemeUpdate::Loaded { theme, .. }) = pending.update.take() else {
			panic!("expected swapped symlink live theme to reload");
		};
		assert_eq!(theme.dark.colors["accent"], "#445566");
	}

	#[test]
	fn watcher_thread_loads_initial_file_and_stops_cleanly_on_drop() {
		let dir = TempDir::new();
		let path = dir.0.join(LIVE_THEME_FILE);
		fs::write(&path, serde_json::to_vec(&sample_theme("#abcdef")).unwrap()).unwrap();
		let (tx, rx) = mpsc::channel();
		let watcher = LiveThemeWatcher::spawn(
			&dir.0,
			eframe::egui::Context::default(),
			POLL_INTERVAL,
			Some(tx),
		)
		.unwrap();
		rx.recv_timeout(Duration::from_secs(5))
			.expect("watcher thread did not publish initial theme");
		let changes = watcher.take_changes();
		let Some(LiveThemeUpdate::Loaded { theme, .. }) = changes.update else {
			panic!("expected Loaded update from watcher thread");
		};
		assert_eq!(theme.dark.colors["accent"], "#abcdef");
		assert!(changes.error.is_none());
		drop(watcher);
	}
}
