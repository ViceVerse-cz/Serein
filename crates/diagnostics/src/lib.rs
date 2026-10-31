//! Serein's opt-in diagnostic log.
//!
//! A record carries a compile-time category and label, numbers, and at most four fields whose text
//! is a compile-time constant. Nothing in this API accepts an account, channel, message, path,
//! credential or URL value, so a written log can be attached to an issue without redaction.
//!
//! Every buffer is bounded. Ring overflow and queue overflow drop the oldest line rather than the
//! newest, the writer thread is the only one that touches the disk, and the file rotates once and
//! then stops instead of growing without end. Logging is off until the owner selects a level in
//! Settings; nothing is created until logging is enabled.

use std::{
	collections::VecDeque,
	fs::File,
	io::Write,
	path::{Path, PathBuf},
	sync::{
		Condvar, Mutex, MutexGuard, OnceLock,
		atomic::{AtomicU8, Ordering},
	},
	time::{Duration, Instant},
};

/// Fields one record may carry; a longer slice is truncated, never allocated past this.
const MAX_FIELDS: usize = 4;
/// Lines waiting for the writer, and their formatted bytes.
const MAX_QUEUE_LINES: usize = 256;
const MAX_QUEUE_BYTES: usize = 64 * 1024;
/// The copyable tail kept for the Settings button, independent of the file.
const MAX_RING_LINES: usize = 512;
const MAX_RING_BYTES: usize = 128 * 1024;
/// One formatted line, so a single record cannot bloat either buffer.
const MAX_LINE_BYTES: usize = 320;
/// The file rotates once; a run then stops appending instead of growing without end.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RUN_BYTES: u64 = 4 * 1024 * 1024;
const LOG_FILE: &str = "serein.log";
const PREVIOUS_LOG_FILE: &str = "serein.1.log";

/// How much detail the log keeps: a record is written when its level is at or above the
/// owner's choice, so `Error` keeps failures only and `Debug` keeps everything.
#[derive(
	Clone,
	Copy,
	Debug,
	Default,
	PartialEq,
	Eq,
	PartialOrd,
	Ord,
	serde::Serialize,
	serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Level {
	/// Nothing is recorded and no folder or file is created.
	#[default]
	Off,
	/// An operation the owner asked for failed.
	Error,
	/// A recoverable anomaly: a retry, a rejected event, an expiring deadline.
	Warn,
	/// Lifecycle milestones: a call joined, a gateway connected, a level chosen.
	Info,
	/// Internal decisions worth seeing next to a reproduction.
	Debug,
}

impl Level {
	/// Every level in menu order.
	pub const ALL: [Self; 5] = [Self::Off, Self::Error, Self::Warn, Self::Info, Self::Debug];

	/// The level's name in a log line.
	fn tag(self) -> &'static str {
		match self {
			Self::Off => "off",
			Self::Error => "ERROR",
			Self::Warn => "WARN",
			Self::Info => "INFO",
			Self::Debug => "DEBUG",
		}
	}

	fn code(self) -> u8 {
		match self {
			Self::Off => 0,
			Self::Error => 1,
			Self::Warn => 2,
			Self::Info => 3,
			Self::Debug => 4,
		}
	}

	fn from_code(code: u8) -> Self {
		Self::ALL
			.into_iter()
			.find(|level| level.code() == code)
			.unwrap_or(Self::Off)
	}
}

/// What a record is about, so a log can be read by scope before it is read in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
	/// Startup, shutdown, saved preferences and the log itself.
	App,
	/// Login, READY and account changes.
	Session,
	/// The call card: joining, rejoining and failing.
	Call,
	/// Media inside a call: devices, transport, capture and encoding.
	Voice,
	/// Gateway connection, session and dispatch handling.
	Gateway,
	/// Local cache, history and disk budgets.
	Cache,
	/// Attachments, images and video.
	Media,
}

impl Category {
	/// The category's name in a log line.
	fn tag(self) -> &'static str {
		match self {
			Self::App => "app",
			Self::Session => "session",
			Self::Call => "call",
			Self::Voice => "voice",
			Self::Gateway => "gateway",
			Self::Cache => "cache",
			Self::Media => "media",
		}
	}
}

/// One field value: a number, or a constant from the client's own vocabulary.
#[derive(Clone, Copy, Debug)]
pub enum Value {
	Unsigned(u64),
	Signed(i64),
	/// A `&'static str`, so record sites can only use their own constants.
	Text(&'static str),
}

impl From<u8> for Value {
	fn from(value: u8) -> Self {
		Self::Unsigned(u64::from(value))
	}
}

impl From<u16> for Value {
	fn from(value: u16) -> Self {
		Self::Unsigned(u64::from(value))
	}
}

impl From<u32> for Value {
	fn from(value: u32) -> Self {
		Self::Unsigned(u64::from(value))
	}
}

impl From<u64> for Value {
	fn from(value: u64) -> Self {
		Self::Unsigned(value)
	}
}

impl From<usize> for Value {
	fn from(value: usize) -> Self {
		Self::Unsigned(value as u64)
	}
}

impl From<i64> for Value {
	fn from(value: i64) -> Self {
		Self::Signed(value)
	}
}

impl From<bool> for Value {
	fn from(value: bool) -> Self {
		Self::Unsigned(u64::from(value))
	}
}

impl From<&'static str> for Value {
	fn from(value: &'static str) -> Self {
		Self::Text(value)
	}
}

/// Whether a record at `level` would be kept now, so callers can skip measuring a value.
pub fn is_enabled(level: Level) -> bool {
	sink().enabled(level)
}

/// The level the log is currently set to.
pub fn level() -> Level {
	Level::from_code(sink().level.load(Ordering::Relaxed))
}

/// The fallback folder used when Serein's own data folder cannot be written.
const FALLBACK_FOLDER: &str = "serein-logs";

/// The folder a log uses when the app has no writable data folder at all.
pub fn fallback_directory() -> PathBuf {
	std::env::temp_dir().join(FALLBACK_FOLDER)
}

/// The folder that holds the running binary; a sandboxed run often keeps write access there even
/// when the profile and the temporary folder are refused.
fn executable_directory() -> Option<PathBuf> {
	let executable = std::env::current_exe().ok()?;
	executable.parent().map(Path::to_path_buf)
}

/// Name the folder for the log file and apply `level`. Only an enabled level touches the disk, so
/// an owner who never turns the log on gets no folder and no file. When the data folder cannot be
/// written, the temporary folder and then the executable's own folder are tried, because a report
/// still needs its evidence; the caller can then name the file that ended up in use.
pub fn install(level: Level, directory: PathBuf) {
	let sink = sink();
	sink.configure(level, directory);
	set_level(level);
	sink.wake.notify_all();
}

/// Installation is performed by the single writer, never by a UI frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstallStatus {
	#[default]
	Ready,
	Pending,
	Failed,
}

pub fn install_status() -> InstallStatus {
	sink().lock().status
}

fn prepare_destination(level: Level, directory: PathBuf) -> Result<PathBuf, &'static str> {
	let mut path = directory.join(LOG_FILE);
	let mut moved = false;
	let mut reason = None;
	// Every attempt is reported by number: a fixed label alone cannot tell "denied" from
	// "sharing violation", and the whole point of this log is to answer that without guessing.
	let mut attempts: Vec<(&'static str, Option<std::io::Error>)> = Vec::with_capacity(3);
	if level != Level::Off {
		let primary = prepare(&directory);
		let refused = primary.is_some();
		attempts.push(("data", primary));
		if refused {
			let mut candidates = vec![("temporary", fallback_directory())];
			if let Some(executable) = executable_directory() {
				candidates.push(("executable", executable.join(FALLBACK_FOLDER)));
			}
			for (label, candidate) in candidates {
				let error = prepare(&candidate);
				let worked = error.is_none();
				attempts.push((label, error));
				if worked {
					path = candidate.join(LOG_FILE);
					moved = true;
					break;
				}
			}
			if !moved {
				reason = Some(attempt_failure(&attempts[0].1));
			}
		}
	}
	for (folder, error) in &attempts {
		let Some(error) = error else {
			record_fields(
				Category::App,
				Level::Info,
				"log folder ready",
				&[("folder", (*folder).into())],
			);
			continue;
		};
		record_fields(
			Category::App,
			Level::Warn,
			"log folder refused",
			&[
				("folder", (*folder).into()),
				("kind", error_kind(error).into()),
				(
					"os_error",
					i64::from(error.raw_os_error().unwrap_or(-1)).into(),
				),
			],
		);
	}
	if let Some(reason) = reason {
		record_fields(
			Category::App,
			Level::Error,
			"log file unavailable",
			&[("reason", reason.into())],
		);
		return Err(reason);
	}
	if moved {
		record_fields(
			Category::App,
			Level::Warn,
			"log file moved",
			&[("reason", "the data folder could not be written".into())],
		);
	}
	Ok(path)
}

/// The fixed reason a failed attempt reports to the page.
fn attempt_failure(error: &Option<std::io::Error>) -> &'static str {
	match error.as_ref().map(|error| error.kind()) {
		Some(std::io::ErrorKind::NotFound) => "the log folder does not exist",
		Some(std::io::ErrorKind::PermissionDenied) => "the log folder is not writable",
		Some(_) => "the log folder could not be created",
		None => "the log folder could not be created",
	}
}

/// A small stable number for the failure class, so a report needs no prose to name it.
fn error_kind(error: &std::io::Error) -> u64 {
	match error.kind() {
		std::io::ErrorKind::NotFound => 1,
		std::io::ErrorKind::PermissionDenied => 2,
		std::io::ErrorKind::AlreadyExists => 3,
		std::io::ErrorKind::InvalidInput => 4,
		std::io::ErrorKind::UnexpectedEof => 5,
		std::io::ErrorKind::WriteZero => 6,
		std::io::ErrorKind::ReadOnlyFilesystem => 7,
		std::io::ErrorKind::DirectoryNotEmpty => 8,
		std::io::ErrorKind::StorageFull => 9,
		std::io::ErrorKind::NotADirectory => 10,
		std::io::ErrorKind::IsADirectory => 11,
		_ => 0,
	}
}

/// Creates the folder and proves the file can be opened, without keeping it open: the writer owns
/// the handle. Returns the operating system's error when the folder is unusable.
fn prepare(directory: &Path) -> Option<std::io::Error> {
	open_log(directory).err()
}

fn refused() -> std::io::Error {
	std::io::Error::new(
		std::io::ErrorKind::PermissionDenied,
		"unsafe log destination",
	)
}

/// Check ownership and type on the opened descriptors. NOFOLLOW and NONBLOCK prevent a
/// swapped symlink or FIFO from redirecting a write or blocking startup. Permissions are
/// applied to the handles, never to a path that could have been replaced.
#[cfg(unix)]
fn open_log(directory: &Path) -> std::io::Result<File> {
	use rustix::fs::{self, FileType, Mode, OFlags};
	create_private(directory)?;
	let dir = fs::open(
		directory,
		OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
		Mode::empty(),
	)?;
	let uid = rustix::process::geteuid().as_raw();
	if fs::fstat(&dir)?.st_uid != uid {
		return Err(refused());
	}
	fs::fchmod(&dir, Mode::from_raw_mode(0o700))?;
	let fd = fs::openat(
		&dir,
		LOG_FILE,
		OFlags::CREATE
			| OFlags::APPEND
			| OFlags::WRONLY
			| OFlags::NOFOLLOW
			| OFlags::NONBLOCK
			| OFlags::CLOEXEC,
		Mode::from_raw_mode(0o600),
	)?;
	let stat = fs::fstat(&fd)?;
	if stat.st_uid != uid
		|| stat.st_nlink != 1
		|| FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
	{
		return Err(refused());
	}
	fs::fchmod(&fd, Mode::from_raw_mode(0o600))?;
	Ok(File::from(fd))
}

#[cfg(not(unix))]
fn open_log(directory: &Path) -> std::io::Result<File> {
	create_private(directory)?;
	let path = directory.join(LOG_FILE);
	if shared(directory) || shared(&path) {
		return Err(refused());
	}
	let mut options = std::fs::OpenOptions::new();
	options.create(true).append(true);
	#[cfg(windows)]
	{
		use std::os::windows::fs::OpenOptionsExt;
		// FILE_FLAG_OPEN_REPARSE_POINT: inspect the link itself instead of its target.
		options.custom_flags(0x0020_0000);
	}
	let file = options.open(path)?;
	if !file.metadata()?.is_file() || reparse(&file.metadata()?) {
		return Err(refused());
	}
	Ok(file)
}

#[cfg(not(unix))]
fn shared(path: &Path) -> bool {
	std::fs::symlink_metadata(path)
		.is_ok_and(|meta| meta.file_type().is_symlink() || reparse(&meta))
}

#[cfg(not(unix))]
fn reparse(meta: &std::fs::Metadata) -> bool {
	#[cfg(windows)]
	{
		use std::os::windows::fs::MetadataExt;
		meta.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT, including junctions.
	}
	#[cfg(not(windows))]
	{
		meta.file_type().is_symlink()
	}
}

/// Applies a level chosen in Settings; it takes effect for the next record.
pub fn set_level(level: Level) {
	let sink = sink();
	let previous = Level::from_code(sink.level.swap(level.code(), Ordering::Relaxed));
	if previous != level {
		record_fields(
			Category::App,
			Level::Info,
			"log level",
			&[("now", level.tag().into()), ("was", previous.tag().into())],
		);
	}
}

/// Where the log file is written, once a folder is installed.
pub fn path() -> Option<PathBuf> {
	sink()
		.lock()
		.directory
		.as_ref()
		.map(|dir| dir.join(LOG_FILE))
}

/// The most recent formatted lines, newest last, for the copy button.
pub fn recent(max_lines: usize) -> String {
	let shared = sink().lock();
	let mut lines: Vec<&str> = shared
		.ring
		.iter()
		.rev()
		.take(max_lines)
		.map(String::as_str)
		.collect();
	lines.reverse();
	lines.join("\n")
}

/// Waits for queued lines to reach the file, so an orderly exit does not lose the tail.
pub fn flush(timeout: Duration) {
	let sink = sink();
	let started = Instant::now();
	let mut shared = sink.lock();
	while !(shared.queue.is_empty()
		&& shared.pending == 0
		&& shared.status != InstallStatus::Pending)
	{
		let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
			return;
		};
		shared = sink.wait(shared, remaining);
	}
}

/// Records a labelled moment with no fields.
pub fn record(category: Category, level: Level, label: &'static str) {
	record_fields(category, level, label, &[]);
}

/// Records a labelled moment with bounded numeric or fixed-text fields.
pub fn record_fields(
	category: Category,
	level: Level,
	label: &'static str,
	fields: &[(&'static str, Value)],
) {
	let sink = sink();
	if !sink.enabled(level) {
		return;
	}
	sink.push(format_line(&clock(), category, level, label, fields));
}

fn sink() -> &'static Sink {
	static SINK: OnceLock<&'static Sink> = OnceLock::new();
	SINK.get_or_init(|| {
		let sink: &'static Sink = Box::leak(Box::new(Sink::new()));
		let _ = std::thread::Builder::new()
			.name("serein-log".into())
			.spawn(move || writer(sink));
		sink
	})
}

/// The one writer: it parks until lines arrive and is the only thread that opens the file. It
/// reopens whenever the destination changes, so Settings never names a file the writer ignores.
fn writer(sink: &'static Sink) {
	let mut file: Option<FileSink> = None;
	let mut spent = 0;
	loop {
		let (batch, install) = sink.take_work();
		if let Some((generation, level, directory)) = install {
			spent = file.as_ref().map_or(spent, FileSink::spent);
			file = None;
			let result = if level == Level::Off {
				None
			} else {
				Some(prepare_destination(level, directory))
			};
			let status = if result.as_ref().is_some_and(Result::is_err) {
				InstallStatus::Failed
			} else {
				InstallStatus::Ready
			};
			let directory = result
				.and_then(Result::ok)
				.and_then(|path| path.parent().map(Path::to_path_buf));
			// Even metadata reads stay outside the mutex used by UI producers.
			let prepared = directory
				.as_ref()
				.map(|directory| FileSink::new(directory.join(LOG_FILE), spent));
			let mut shared = sink.lock();
			if shared.destination == generation {
				shared.status = status;
				shared.directory = directory;
				file = prepared;
			}
			sink.wake.notify_all();
		}
		if sink.level.load(Ordering::Relaxed) != Level::Off.code()
			&& let Some(file) = &mut file
		{
			file.write(&batch);
		}
		sink.finished(batch.len());
	}
}

struct Shared {
	queue: VecDeque<String>,
	queue_bytes: usize,
	ring: VecDeque<String>,
	ring_bytes: usize,
	/// Lines taken by the writer and not yet on disk, so `flush` can wait for the file too.
	pending: usize,
	directory: Option<PathBuf>,
	/// Bumped by every install, so the writer notices a destination it has not opened yet.
	destination: u64,
	install: Option<(u64, Level, PathBuf)>,
	status: InstallStatus,
}

struct Sink {
	level: AtomicU8,
	shared: Mutex<Shared>,
	wake: Condvar,
}

impl Sink {
	fn configure(&self, level: Level, directory: PathBuf) {
		let mut shared = self.lock();
		shared.destination = shared.destination.wrapping_add(1);
		shared.install = Some((shared.destination, level, directory));
		shared.status = InstallStatus::Pending;
		shared.directory = None;
	}

	fn new() -> Self {
		Self {
			level: AtomicU8::new(Level::Off.code()),
			shared: Mutex::new(Shared {
				queue: VecDeque::new(),
				queue_bytes: 0,
				ring: VecDeque::new(),
				ring_bytes: 0,
				pending: 0,
				directory: None,
				destination: 0,
				install: None,
				status: InstallStatus::Ready,
			}),
			wake: Condvar::new(),
		}
	}

	/// A record is kept when it is at or above the chosen level.
	fn enabled(&self, level: Level) -> bool {
		level != Level::Off && Level::from_code(self.level.load(Ordering::Relaxed)) >= level
	}

	fn lock(&self) -> MutexGuard<'_, Shared> {
		self.shared
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
	}

	fn wait<'a>(
		&self,
		shared: MutexGuard<'a, Shared>,
		timeout: Duration,
	) -> MutexGuard<'a, Shared> {
		self.wake
			.wait_timeout(shared, timeout)
			.unwrap_or_else(|poisoned| poisoned.into_inner())
			.0
	}

	fn push(&self, line: String) {
		{
			let mut shared = self.lock();
			let Shared {
				queue,
				queue_bytes,
				ring,
				ring_bytes,
				pending,
				..
			} = &mut *shared;
			let dropped = push_bounded(
				queue,
				queue_bytes,
				line.clone(),
				MAX_QUEUE_LINES,
				MAX_QUEUE_BYTES,
			);
			// A line the queue dropped will never be written, so `flush` must not wait for it.
			*pending = (*pending).saturating_sub(dropped) + 1;
			push_bounded(ring, ring_bytes, line, MAX_RING_LINES, MAX_RING_BYTES);
		}
		self.wake.notify_all();
	}

	/// Blocks until a line or setup request is queued, then hands the writer bounded work.
	fn take_work(&self) -> (Vec<String>, Option<(u64, Level, PathBuf)>) {
		let mut shared = self.lock();
		while shared.queue.is_empty() && shared.install.is_none() {
			shared = self
				.wake
				.wait(shared)
				.unwrap_or_else(|poisoned| poisoned.into_inner());
		}
		let mut batch = Vec::with_capacity(MAX_QUEUE_LINES);
		let mut bytes = 0;
		while let Some(line) = shared.queue.front() {
			if !batch.is_empty() && bytes + line.len() > MAX_QUEUE_BYTES {
				break;
			}
			bytes += line.len();
			shared.queue_bytes -= line.len();
			batch.push(shared.queue.pop_front().expect("front line"));
		}
		(batch, shared.install.take())
	}

	fn finished(&self, lines: usize) {
		{
			let mut shared = self.lock();
			shared.pending = shared.pending.saturating_sub(lines);
		}
		self.wake.notify_all();
	}
}

/// Keeps the newest lines: an overflow drops the oldest, never the newest. Returns how many lines
/// the queue gave up, which keeps the writer's outstanding count honest.
fn push_bounded(
	queue: &mut VecDeque<String>,
	bytes: &mut usize,
	line: String,
	max_lines: usize,
	max_bytes: usize,
) -> usize {
	*bytes += line.len();
	queue.push_back(line);
	let mut dropped = 0;
	while queue.len() > max_lines || *bytes > max_bytes {
		let Some(given_up) = queue.pop_front() else {
			break;
		};
		*bytes -= given_up.len();
		dropped += 1;
	}
	dropped
}

/// Appends to one file, rotates it once, then stops instead of growing without end.
struct FileSink {
	path: PathBuf,
	file: Option<File>,
	/// Bytes in the current file, reset when it rotates.
	file_bytes: u64,
	/// Bytes this process has written across rotations, never reset.
	run_bytes: u64,
	stopped: bool,
}

impl FileSink {
	fn new(path: PathBuf, run_bytes: u64) -> Self {
		// The file is opened in append mode, so what earlier runs left counts against its budget.
		let file_bytes = std::fs::metadata(&path).map_or(0, |meta| meta.len());
		Self {
			path,
			file: None,
			file_bytes,
			run_bytes,
			stopped: false,
		}
	}

	/// Bytes this process has written for the whole run, across rotations and reopens.
	fn spent(&self) -> u64 {
		self.run_bytes
	}

	fn write(&mut self, lines: &[String]) -> usize {
		if self.stopped {
			return 0;
		}
		let mut written = 0;
		for line in lines {
			let bytes = line.len() as u64;
			if self.run_bytes.saturating_add(bytes) > MAX_RUN_BYTES {
				// The run's budget is spent; the tail stays in the copyable ring.
				self.stopped = true;
				self.file = None;
				break;
			}
			if self.handle().is_none() {
				self.stopped = true;
				break;
			}
			if self.file_bytes.saturating_add(bytes) > MAX_FILE_BYTES && !self.rotate() {
				// Another program holds the old file: stop rather than append past the cap.
				self.stopped = true;
				break;
			}
			if self.handle().is_none() {
				self.stopped = true;
				break;
			}
			let Some(file) = self.file.as_mut() else {
				self.stopped = true;
				break;
			};
			// Reserve even a failed/partial write against the process budget.
			self.run_bytes = self.run_bytes.saturating_add(bytes);
			if file.write_all(line.as_bytes()).is_err() {
				// A full or removed file must not stop the client; the ring stays usable.
				self.file = None;
				break;
			}
			self.file_bytes = self.file_bytes.saturating_add(bytes);
			written += 1;
		}
		written
	}

	fn handle(&mut self) -> Option<&mut File> {
		if self.file.is_none() {
			let directory = self.path.parent()?;
			let file = open_log(directory).ok()?;
			self.file_bytes = file.metadata().ok()?.len();
			self.file = Some(file);
		}
		self.file.as_mut()
	}

	/// Moves the full file aside. Returns whether the file was really replaced, because appending
	/// to a file another program refused to move would grow past the cap.
	fn rotate(&mut self) -> bool {
		self.file = None;
		let previous = self.path.with_file_name(PREVIOUS_LOG_FILE);
		let _ = std::fs::remove_file(&previous);
		if std::fs::rename(&self.path, &previous).is_err() {
			return false;
		}
		self.file_bytes = 0;
		true
	}
}

/// Creates only missing folders, privately, so a log stays as restricted as the data folder.
fn create_private(directory: &Path) -> std::io::Result<()> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::DirBuilderExt;
		std::fs::DirBuilder::new()
			.recursive(true)
			.mode(0o700)
			.create(directory)
	}
	#[cfg(not(unix))]
	{
		std::fs::create_dir_all(directory)
	}
}

/// The wall clock of the machine, read once per run; a log line correlates an action with a time.
fn clock() -> String {
	static OFFSET: OnceLock<Option<time::UtcOffset>> = OnceLock::new();
	let utc = time::OffsetDateTime::now_utc();
	let at = match OFFSET
		.get_or_init(|| time::UtcOffset::local_offset_at(time::OffsetDateTime::now_utc()).ok())
	{
		Some(offset) => utc.to_offset(*offset),
		None => utc,
	};
	format!(
		"{:02}:{:02}:{:02}.{:03}",
		at.hour(),
		at.minute(),
		at.second(),
		at.millisecond()
	)
}

fn format_line(
	stamp: &str,
	category: Category,
	level: Level,
	label: &str,
	fields: &[(&'static str, Value)],
) -> String {
	use std::fmt::Write;
	let mut line = String::with_capacity(MAX_LINE_BYTES.min(96 + fields.len() * 24));
	line.push_str(stamp);
	line.push(' ');
	push_padded(&mut line, level.tag(), 5);
	push_padded(&mut line, category.tag(), 8);
	line.push_str(label);
	for (name, value) in fields.iter().take(MAX_FIELDS) {
		line.push(' ');
		line.push_str(name);
		line.push('=');
		match value {
			Value::Unsigned(number) => {
				let _ = write!(line, "{number}");
			}
			Value::Signed(number) => {
				let _ = write!(line, "{number}");
			}
			Value::Text(text) => {
				line.push('"');
				line.push_str(text);
				line.push('"');
			}
		}
	}
	truncate(&mut line, MAX_LINE_BYTES);
	line.push('\n');
	line
}

fn push_padded(line: &mut String, text: &str, width: usize) {
	line.push_str(text);
	for _ in text.len()..width {
		line.push(' ');
	}
	line.push(' ');
}

/// Truncates on a character boundary, so a line stays valid UTF-8.
fn truncate(line: &mut String, cap: usize) {
	if line.len() <= cap {
		return;
	}
	let mut end = cap;
	while !line.is_char_boundary(end) {
		end -= 1;
	}
	line.truncate(end);
}

#[cfg(test)]
mod tests {
	use super::*;

	fn line(level: Level, label: &str, fields: &[(&'static str, Value)]) -> String {
		format_line("13:47:52.011", Category::Call, level, label, fields)
	}

	#[test]
	fn a_level_keeps_itself_and_above_and_off_keeps_nothing() {
		let sink = Sink::new();
		assert!(!sink.enabled(Level::Error), "off records nothing");
		sink.level.store(Level::Warn.code(), Ordering::Relaxed);
		assert!(sink.enabled(Level::Error), "a worse record is kept");
		assert!(sink.enabled(Level::Warn));
		assert!(!sink.enabled(Level::Info), "a finer record is filtered");
		assert!(!sink.enabled(Level::Off));
		sink.level.store(Level::Debug.code(), Ordering::Relaxed);
		assert!(sink.enabled(Level::Debug));
	}

	#[test]
	fn a_formatted_line_names_its_level_category_label_and_numbers() {
		assert_eq!(
			line(
				Level::Warn,
				"media failure",
				&[("attempt", 2u8.into()), ("reason", "disconnected".into())]
			),
			"13:47:52.011 WARN  call     media failure attempt=2 reason=\"disconnected\"\n"
		);
		assert_eq!(
			line(Level::Info, "joined", &[]),
			"13:47:52.011 INFO  call     joined\n"
		);
	}

	#[test]
	fn an_over_long_line_is_truncated_on_a_character_boundary() {
		let mut ascii = "a".repeat(MAX_LINE_BYTES + 64);
		truncate(&mut ascii, MAX_LINE_BYTES);
		assert_eq!(ascii.len(), MAX_LINE_BYTES);
		let mut mixed = "я".repeat(MAX_LINE_BYTES);
		truncate(&mut mixed, MAX_LINE_BYTES - 1);
		assert!(mixed.len() < MAX_LINE_BYTES);
		assert!(
			mixed.is_char_boundary(mixed.len()),
			"a cut never splits a character"
		);
	}

	#[test]
	fn both_buffers_keep_the_newest_lines_within_their_bounds() {
		let sink = Sink::new();
		for index in 0..(MAX_RING_LINES * 2) {
			sink.push(format!("line {index} {}\n", "x".repeat(48)));
		}
		let shared = sink.lock();
		assert!(shared.ring.len() <= MAX_RING_LINES);
		assert!(shared.queue.len() <= MAX_QUEUE_LINES);
		assert!(shared.ring_bytes <= MAX_RING_BYTES);
		assert!(shared.queue_bytes <= MAX_QUEUE_BYTES);
		assert_eq!(
			shared.pending,
			shared.queue.len(),
			"a dropped line is never awaited by flush"
		);
		assert!(
			shared
				.ring
				.back()
				.is_some_and(|last| last.starts_with("line 1023 ")),
			"the newest line survives an overflow"
		);
		assert!(
			shared
				.ring
				.front()
				.is_some_and(|first| !first.starts_with("line 1023 ")),
			"the oldest lines are the ones dropped"
		);
	}

	#[test]
	fn a_batch_drained_for_the_writer_stays_within_the_queue_budget() {
		let sink = Sink::new();
		for index in 0..MAX_QUEUE_LINES {
			sink.push(format!("{index:0>60}\n"));
		}
		let (batch, _) = sink.take_work();
		assert!(!batch.is_empty());
		assert!(batch.len() <= MAX_QUEUE_LINES);
		assert!(batch.iter().map(String::len).sum::<usize>() <= MAX_QUEUE_BYTES);
	}

	#[test]
	fn the_file_rotates_once_then_stops_at_the_run_budget() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("logs").join(LOG_FILE);
		let mut sink = FileSink::new(path.clone(), 0);
		let chunk = format!("{}\n", "y".repeat(1024));
		let per_file = MAX_FILE_BYTES as usize / chunk.len() + 4;
		sink.write(&vec![chunk.clone(); per_file]);
		assert!(path.exists(), "the first file is written");
		assert!(
			std::fs::metadata(&path).unwrap().len() <= MAX_FILE_BYTES,
			"a file never grows past its own budget"
		);
		assert!(
			path.with_file_name(PREVIOUS_LOG_FILE).exists(),
			"the previous file is kept once"
		);
		let per_run = MAX_RUN_BYTES as usize / chunk.len() + 4;
		sink.write(&vec![chunk; per_run]);
		assert!(
			sink.stopped,
			"the run budget closes the file for this process"
		);
		assert!(
			std::fs::metadata(path.with_file_name(PREVIOUS_LOG_FILE))
				.unwrap()
				.len() + std::fs::metadata(&path).unwrap().len()
				<= MAX_FILE_BYTES * 2,
			"two files bound what a run can leave on disk"
		);
	}

	#[test]
	fn a_folder_that_cannot_be_created_keeps_the_log_silent_instead_of_panicking() {
		let directory = tempfile::tempdir().unwrap();
		let blocker = directory.path().join("file");
		std::fs::write(&blocker, b"not a folder").unwrap();
		let mut sink = FileSink::new(blocker.join(LOG_FILE), 0);
		assert_eq!(sink.write(&["line\n".to_owned()]), 0);
		assert!(sink.stopped);
	}

	#[test]
	fn a_file_left_by_an_earlier_run_counts_toward_its_budget() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join(LOG_FILE);
		// A run that stopped just under the cap leaves the next launch to notice it.
		std::fs::write(&path, vec![b'x'; MAX_FILE_BYTES as usize - 8]).unwrap();
		let mut sink = FileSink::new(path.clone(), 0);
		sink.write(&["a short line\n".to_owned()]);
		assert!(
			path.with_file_name(PREVIOUS_LOG_FILE).exists(),
			"the full file is rotated before anything is appended"
		);
		assert!(
			std::fs::metadata(&path).unwrap().len() <= MAX_FILE_BYTES,
			"an appended file still honours its own cap across launches"
		);
	}

	#[test]
	fn a_shared_fallback_folder_is_refused_before_anything_is_written() {
		// A folder another account planted is not a log destination; the check is by owner on
		// Unix and by link type everywhere, and either way nothing may be appended.
		let directory = tempfile::tempdir().unwrap();
		let logs = directory.path().join("logs");
		std::fs::create_dir_all(&logs).unwrap();
		#[cfg(unix)]
		{
			let planted = directory.path().join("planted");
			std::fs::create_dir_all(&planted).unwrap();
			std::os::unix::fs::symlink(&planted, logs.join(LOG_FILE)).unwrap();
			assert!(prepare(&logs).is_some(), "a linked log file is refused");
			assert!(
				!planted.join(LOG_FILE).exists(),
				"nothing is written through the link"
			);
		}
		#[cfg(not(unix))]
		{
			assert!(prepare(&logs).is_none());
		}
	}
	#[test]
	fn setup_is_coalesced_without_touching_the_destination() {
		let directory = tempfile::tempdir().unwrap();
		let logs = directory.path().join("not-created-by-the-caller");
		let sink = Sink::new();
		sink.configure(Level::Info, logs.clone());
		sink.configure(Level::Off, logs.clone());
		assert!(!logs.exists());
		let (batch, request) = sink.take_work();
		assert!(batch.is_empty());
		let (_, level, path) = request.unwrap();
		assert_eq!(level, Level::Off);
		assert_eq!(path, logs);
		assert!(sink.lock().install.is_none());
	}

	#[test]
	fn a_repaired_destination_can_reopen_without_renewing_the_run_budget() {
		let directory = tempfile::tempdir().unwrap();
		let blocker = directory.path().join("logs");
		std::fs::write(&blocker, b"blocker").unwrap();
		let path = blocker.join(LOG_FILE);
		let spent = MAX_RUN_BYTES - 5;
		let mut broken = FileSink::new(path.clone(), spent);
		assert_eq!(broken.write(&["abc\n".into()]), 0);
		std::fs::remove_file(&blocker).unwrap();
		let mut repaired = FileSink::new(path.clone(), broken.spent());
		assert_eq!(repaired.write(&["abc\n".into(), "def\n".into()]), 1);
		assert!(repaired.stopped);
		assert_eq!(std::fs::read(path).unwrap(), b"abc\n");
	}

	#[cfg(unix)]
	#[test]
	fn every_reopen_refuses_symlinks_and_fifos() {
		use rustix::fs::{CWD, Mode, mkfifoat};
		let directory = tempfile::tempdir().unwrap();
		let target = directory.path().join("target");
		std::fs::write(&target, b"unchanged").unwrap();
		let logs = directory.path().join("logs");
		std::fs::create_dir(&logs).unwrap();
		let path = logs.join(LOG_FILE);
		std::os::unix::fs::symlink(&target, &path).unwrap();
		assert!(prepare(&logs).is_some());
		assert_eq!(
			FileSink::new(path.clone(), 0).write(&["private\n".into()]),
			0
		);
		assert_eq!(std::fs::read(&target).unwrap(), b"unchanged");
		std::fs::remove_file(&path).unwrap();
		mkfifoat(CWD, &path, Mode::from_raw_mode(0o600)).unwrap();
		assert!(prepare(&logs).is_some());
		assert_eq!(FileSink::new(path, 0).write(&["private\n".into()]), 0);
	}
	#[test]
	fn the_writer_installs_flushes_and_changes_destination_without_ui_io() {
		let directory = tempfile::tempdir().unwrap();
		let disabled = directory.path().join("disabled");
		install(Level::Off, disabled.clone());
		flush(Duration::from_secs(2));
		assert_eq!(install_status(), InstallStatus::Ready);
		assert!(!disabled.exists());
		assert!(path().is_none());
		let first = directory.path().join("first");
		install(Level::Info, first.clone());
		record(Category::App, Level::Info, "synthetic first destination");
		flush(Duration::from_secs(2));
		assert_eq!(install_status(), InstallStatus::Ready);
		assert_eq!(path(), Some(first.join(LOG_FILE)));
		assert!(
			std::fs::read_to_string(first.join(LOG_FILE))
				.unwrap()
				.contains("synthetic first destination")
		);
		let second = directory.path().join("second");
		install(Level::Info, second.clone());
		record(Category::App, Level::Info, "synthetic second destination");
		flush(Duration::from_secs(2));
		assert_eq!(path(), Some(second.join(LOG_FILE)));
		assert!(
			std::fs::read_to_string(second.join(LOG_FILE))
				.unwrap()
				.contains("synthetic second destination")
		);
		assert!(
			!std::fs::read_to_string(first.join(LOG_FILE))
				.unwrap()
				.contains("synthetic second destination")
		);
		install(Level::Off, disabled);
		flush(Duration::from_secs(2));
	}
}
