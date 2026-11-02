//! Opt-in, bounded diagnostics. Only reviewed event/field/tag vocabulary crosses this boundary.
//! Memory collection and private disk history are independent. No strings from errors or payloads.
use std::{
	collections::VecDeque,
	fs::File,
	io::Write,
	path::{Path, PathBuf},
	sync::{
		Condvar, Mutex, MutexGuard,
		atomic::{AtomicU8, AtomicU64, Ordering},
	},
	time::{Duration, Instant},
};

mod vocabulary;
const MAX_FIELDS: usize = 5;
const MAX_RING_LINES: usize = 2048;
const MAX_RING_BYTES: usize = 512 * 1024;
const MAX_QUEUE_LINES: usize = 128;
const MAX_QUEUE_BYTES: usize = 64 * 1024;
const MAX_LINE_BYTES: usize = 512;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const LOG_FILE: &str = "serein.log";
const LOG_FILES: [&str; 3] = [LOG_FILE, "serein.1.log", "serein.2.log"];

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
	#[default]
	Off,
	Error,
	Warn,
	Info,
	Debug,
}
impl Level {
	pub const ALL: [Self; 5] = [Self::Off, Self::Error, Self::Warn, Self::Info, Self::Debug];
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
		self as u8
	}
	fn from_code(code: u8) -> Self {
		Self::ALL
			.get(usize::from(code))
			.copied()
			.unwrap_or_default()
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
	App,
	Session,
	Call,
	Voice,
	Gateway,
	Cache,
	Media,
	Composer,
	Messages,
	Api,
	Storage,
	Extensions,
	Platform,
	Updater,
	Activity,
	Runtime,
}
impl Category {
	fn tag(self) -> &'static str {
		match self {
			Self::App => "app",
			Self::Session => "session",
			Self::Call => "call",
			Self::Voice => "voice",
			Self::Gateway => "gateway",
			Self::Cache => "cache",
			Self::Media => "media",
			Self::Composer => "composer",
			Self::Messages => "messages",
			Self::Api => "api",
			Self::Storage => "storage",
			Self::Extensions => "extensions",
			Self::Platform => "platform",
			Self::Updater => "updater",
			Self::Activity => "activity",
			Self::Runtime => "runtime",
		}
	}
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
	Unsigned(u64),
	Signed(i64),
	Text(&'static str),
}
macro_rules! unsigned { ($($t:ty),*) => { $(impl From<$t> for Value { fn from(v:$t)->Self {Self::Unsigned(v as u64)} })* }; }
unsigned!(u8, u16, u32, u64, usize);
impl From<i64> for Value {
	fn from(v: i64) -> Self {
		Self::Signed(v)
	}
}
impl From<bool> for Value {
	fn from(v: bool) -> Self {
		Self::Unsigned(u64::from(v))
	}
}
impl From<&'static str> for Value {
	fn from(v: &'static str) -> Self {
		Self::Text(v)
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InstallStatus {
	#[default]
	Ready,
	Pending,
	Failed,
}
static RECORDING_SESSION: AtomicU64 = AtomicU64::new(0);
static LEVEL: AtomicU8 = AtomicU8::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static SINK: Sink = Sink {
	shared: Mutex::new(Shared {
		recording: None,
		generation: 0,
		disk: None,
		cleanup: VecDeque::new(),
		worker: false,
		status: InstallStatus::Ready,
		pending: 0,
	}),
	wake: Condvar::new(),
};
struct Recording {
	started: Instant,
	ring: VecDeque<Record>,
	bytes: usize,
	queue: VecDeque<Record>,
	queue_bytes: usize,
	summaries: Vec<Record>,
}
struct Shared {
	recording: Option<Box<Recording>>,
	generation: u64,
	disk: Option<PathBuf>,
	cleanup: VecDeque<PathBuf>,
	worker: bool,
	status: InstallStatus,
	pending: usize,
}
struct Sink {
	shared: Mutex<Shared>,
	wake: Condvar,
}
// Canonical typed indices ensure the retained schema never contains caller string references.
#[derive(Clone, Copy, PartialEq, Eq)]
struct EventCode(u16);
#[derive(Clone, Copy, PartialEq, Eq)]
struct FieldCode(u8);
#[derive(Clone, Copy, PartialEq, Eq)]
struct TagCode(u16);
#[derive(Clone, Copy, PartialEq, Eq)]
enum RetainedValue {
	Unsigned(u64),
	Signed(i64),
	Tag(TagCode),
	Unclassified,
}
const _: () = assert!(
	vocabulary::EVENTS.len() <= u16::MAX as usize
		&& vocabulary::FIELDS.len() <= u8::MAX as usize
		&& vocabulary::TAGS.len() <= u16::MAX as usize
);
#[derive(Clone)]
struct Record {
	millis: u64,
	category: Category,
	level: Level,
	code: EventCode,
	fields: [Option<(FieldCode, RetainedValue)>; MAX_FIELDS],
}
impl Record {
	fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
	}
	fn format(&self) -> String {
		use std::fmt::Write;
		let mut line = format!(
			"+{}ms {} {} {}",
			self.millis,
			self.level.tag(),
			self.category.tag(),
			vocabulary::EVENTS[usize::from(self.code.0)]
		);
		for (name, value) in self.fields.iter().flatten() {
			let _ = write!(line, " {}=", vocabulary::FIELDS[usize::from(name.0)]);
			match value {
				RetainedValue::Unsigned(n) => {
					let _ = write!(line, "{n}");
				}
				RetainedValue::Signed(n) => {
					let _ = write!(line, "{n}");
				}
				RetainedValue::Tag(t) => line.push_str(vocabulary::TAGS[usize::from(t.0)]),
				RetainedValue::Unclassified => line.push_str("unclassified"),
			}
		}
		line.truncate(line.floor_char_boundary(MAX_LINE_BYTES.min(line.len())));
		line.push('\n');
		line
	}
}
impl Sink {
	fn lock(&self) -> MutexGuard<'_, Shared> {
		self.shared.lock().unwrap_or_else(|p| p.into_inner())
	}
}
/// Changes when recording is started again or its history is cleared.
pub fn recording_session() -> u64 {
	RECORDING_SESSION.load(Ordering::Acquire)
}
pub fn level() -> Level {
	Level::from_code(LEVEL.load(Ordering::Acquire))
}
pub fn is_enabled(at: Level) -> bool {
	at != Level::Off && level() >= at
}
pub fn disk_enabled() -> bool {
	SINK.lock().disk.is_some()
}
pub fn install_status() -> InstallStatus {
	SINK.lock().status
}
pub fn path() -> Option<PathBuf> {
	SINK.lock().disk.as_ref().map(|d| d.join(LOG_FILE))
}
/// Compatibility destination, used only after explicit disk opt-in; no fallback writes occur.
pub fn fallback_directory() -> PathBuf {
	std::env::temp_dir().join("serein-logs")
}
/// Compatibility entry point for existing explicit file-logging callers.
pub fn install(at: Level, directory: PathBuf) {
	install_mode(at, directory, true);
}
pub fn install_mode(at: Level, directory: PathBuf, disk: bool) {
	if at != Level::Off {
		install_panic_hook();
	}
	LEVEL.store(Level::Off.code(), Ordering::Release);
	let mut shared = SINK.lock();
	let mut invalid = at != Level::Off
		&& disk
		&& (directory.as_os_str().is_empty() || directory.as_os_str().len() > 4096);
	if shared.cleanup.len() >= 4 && !shared.cleanup.contains(&directory) {
		invalid |= disk;
	}
	let wanted = (at != Level::Off && disk && !invalid).then_some(directory);
	if at == Level::Off {
		shared.recording = None;
		DROPPED.store(0, Ordering::Relaxed);
	} else if shared.recording.is_none() {
		RECORDING_SESSION.fetch_add(1, Ordering::AcqRel);
		shared.recording = Some(Box::new(Recording {
			started: Instant::now(),
			ring: VecDeque::new(),
			bytes: 0,
			queue: VecDeque::new(),
			queue_bytes: 0,
			summaries: Vec::new(),
		}));
	}
	if shared.disk != wanted {
		shared.generation = shared.generation.wrapping_add(1);
		if let Some(old) = shared.disk.take()
			&& !shared.cleanup.contains(&old)
		{
			shared.cleanup.push_back(old);
		}
		shared.disk = wanted;
		shared.status = if shared.disk.is_some() {
			InstallStatus::Pending
		} else {
			InstallStatus::Ready
		};
		if let Some(r) = shared.recording.as_mut() {
			r.queue = VecDeque::new();
			r.queue_bytes = 0;
		}
	}
	if shared.disk.is_some() && shared.status == InstallStatus::Failed && !invalid {
		shared.generation = shared.generation.wrapping_add(1);
		shared.status = InstallStatus::Pending;
	}
	LEVEL.store(at.code(), Ordering::Release);
	if invalid {
		shared.status = InstallStatus::Failed;
	}
	if !shared.cleanup.is_empty() {
		shared.status = InstallStatus::Pending;
	}
	start_writer(&mut shared);
	SINK.wake.notify_all();
}
pub fn set_level(at: Level) {
	let directory = SINK.lock().disk.clone();
	install_mode(
		at,
		directory.clone().unwrap_or_default(),
		directory.is_some(),
	);
}
/// Clears memory immediately; the disk worker removes managed history before reopening.
pub fn clear() {
	clear_at(None);
}
/// Explicit owner action can also remove residual managed files while recording is off.
pub fn clear_at(directory: Option<PathBuf>) {
	RECORDING_SESSION.fetch_add(1, Ordering::AcqRel);
	let mut s = SINK.lock();
	if let Some(r) = s.recording.as_mut() {
		r.ring = VecDeque::new();
		r.queue = VecDeque::new();
		r.summaries = Vec::new();
		r.bytes = 0;
		r.queue_bytes = 0;
		r.started = Instant::now();
	}
	s.generation = s.generation.wrapping_add(1);
	if let Some(dir) = s
		.disk
		.clone()
		.or(directory.filter(|d| !d.as_os_str().is_empty() && d.as_os_str().len() <= 4096))
		&& !s.cleanup.contains(&dir)
	{
		if s.cleanup.len() < 4 {
			s.cleanup.push_back(dir);
		} else {
			s.status = InstallStatus::Failed;
		}
	}
	if !s.cleanup.is_empty() {
		s.status = InstallStatus::Pending;
	}
	start_writer(&mut s);
	DROPPED.store(0, Ordering::Relaxed);
	SINK.wake.notify_all();
}
pub fn record(category: Category, at: Level, code: &'static str) {
	record_fields(category, at, code, &[]);
}
pub fn record_fields(
	category: Category,
	at: Level,
	code: &'static str,
	fields: &[(&'static str, Value)],
) {
	emit(category, at, code, fields, false);
}
/// Latest numeric aggregate per approved scope; at most 64 slots, independent of event eviction.
pub fn summary(category: Category, code: &'static str, fields: &[(&'static str, Value)]) {
	emit(category, Level::Debug, code, fields, true);
}
fn emit(
	category: Category,
	at: Level,
	code: &'static str,
	fields: &[(&'static str, Value)],
	summary: bool,
) {
	if !is_enabled(at) {
		return;
	}
	// A static lifetime alone is not a privacy boundary: reject anything outside the reviewed
	// vocabulary, including leaked strings or arbitrary error labels supplied by a new caller.
	let Some(code) = vocabulary::EVENTS.iter().position(|known| *known == code) else {
		DROPPED.fetch_add(1, Ordering::Relaxed);
		return;
	};
	if fields.len() > MAX_FIELDS
		|| fields
			.iter()
			.any(|(key, _)| !vocabulary::FIELDS.contains(key))
	{
		DROPPED.fetch_add(1, Ordering::Relaxed);
		return;
	}
	let code = EventCode(code as u16);
	let Ok(mut s) = SINK.shared.try_lock() else {
		DROPPED.fetch_add(1, Ordering::Relaxed);
		return;
	};
	let disk = s.disk.is_some();
	if !is_enabled(at) {
		return;
	}
	let Some(r) = s.recording.as_mut() else {
		return;
	};
	let mut slots = [None; MAX_FIELDS];
	for (slot, (key, value)) in slots.iter_mut().zip(fields.iter()) {
		let field = FieldCode(
			vocabulary::FIELDS
				.iter()
				.position(|known| known == key)
				.expect("validated field") as u8,
		);
		let value = match value {
			Value::Unsigned(n) => RetainedValue::Unsigned(*n),
			Value::Signed(n) => RetainedValue::Signed(*n),
			Value::Text(t) => vocabulary::TAGS
				.iter()
				.position(|known| known == t)
				.map(|at| RetainedValue::Tag(TagCode(at as u16)))
				.unwrap_or(RetainedValue::Unclassified),
		};
		*slot = Some((field, value));
	}
	let item = Record {
		millis: r.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
		category,
		level: at,
		code,
		fields: slots,
	};
	if summary {
		if let Some(index) = r.summaries.iter().position(|old| {
			old.category == category
				&& old.code == code
				&& old
					.fields
					.iter()
					.zip(slots.iter())
					.all(|(a, b)| match (a, b) {
						(Some((_, RetainedValue::Tag(x))), Some((_, RetainedValue::Tag(y)))) => {
							x == y
						}
						(Some((_, RetainedValue::Tag(_))), _)
						| (_, Some((_, RetainedValue::Tag(_)))) => false,
						_ => true,
					})
		}) {
			r.summaries.remove(index);
		} else if r.summaries.len() == 64 {
			r.summaries.remove(0);
			DROPPED.fetch_add(1, Ordering::Relaxed);
		}
		r.summaries.push(item.clone());
	}
	while r.ring.len() >= MAX_RING_LINES || r.bytes + item.bytes() > MAX_RING_BYTES {
		if let Some(old) = r.ring.pop_front() {
			r.bytes -= old.bytes();
			DROPPED.fetch_add(1, Ordering::Relaxed);
		} else {
			break;
		}
	}
	r.bytes += item.bytes();
	r.ring.push_back(item.clone());
	if disk {
		while r.queue.len() >= MAX_QUEUE_LINES || r.queue_bytes + item.bytes() > MAX_QUEUE_BYTES {
			if let Some(old) = r.queue.pop_front() {
				r.queue_bytes -= old.bytes();
				DROPPED.fetch_add(1, Ordering::Relaxed);
			} else {
				break;
			}
		}
		r.queue_bytes += item.bytes();
		r.queue.push_back(item);
		SINK.wake.notify_all();
	}
}
/// Bounded report text, formatted from a snapshot after releasing the producer lock.
pub fn recent(max_lines: usize) -> String {
	let (records, summaries) = {
		let s = SINK.lock();
		let Some(r) = &s.recording else {
			return "Recording disabled.\n".into();
		};
		(
			r.ring
				.iter()
				.rev()
				.take(max_lines.min(MAX_RING_LINES))
				.cloned()
				.collect::<Vec<_>>(),
			r.summaries.clone(),
		)
	};
	let mut out = format!(
		"Dropped/evicted records: {}\n",
		DROPPED.load(Ordering::Relaxed)
	);
	for r in records.iter().rev() {
		out.push_str(&r.format());
	}
	if !summaries.is_empty() {
		out.push_str("\nLatest subsystem summaries:\n");
		for r in summaries {
			out.push_str(&r.format());
		}
	}
	if out.len() > 1024 * 1024 {
		out.truncate(out.floor_char_boundary(1024 * 1024 - 32));
		out.push_str("\nReport truncated.\n");
	}
	out
}
pub fn flush(timeout: Duration) {
	let start = Instant::now();
	let mut s = SINK.lock();
	while s.worker
		&& (s.pending > 0
			|| !s.cleanup.is_empty()
			|| (s.disk.is_none() && s.worker)
			|| s.status == InstallStatus::Pending
			|| s.recording.as_ref().is_some_and(|r| !r.queue.is_empty()))
	{
		let Some(left) = timeout.checked_sub(start.elapsed()) else {
			return;
		};
		s = SINK
			.wake
			.wait_timeout(s, left)
			.unwrap_or_else(|p| p.into_inner())
			.0;
	}
}
fn start_writer(shared: &mut Shared) {
	if (shared.disk.is_some() || !shared.cleanup.is_empty()) && !shared.worker {
		shared.worker = true;
		if std::thread::Builder::new()
			.name("serein-diagnostics".into())
			.spawn(writer)
			.is_err()
		{
			shared.worker = false;
			shared.status = InstallStatus::Failed;
		}
	}
}
fn writer() {
	let mut current: Option<(u64, PathBuf, FileSink)> = None;
	loop {
		let mut s = SINK.lock();
		if let Some(directory) = s.cleanup.front().cloned() {
			current = None;
			drop(s);
			let failed = remove_history(&directory).is_err();
			s = SINK.lock();
			if failed {
				// Keep the destination pending for the next explicit setup/clear action.
				// Stop this worker instead of spinning or silently opening retained history.
				s.status = InstallStatus::Failed;
				s.worker = false;
				SINK.wake.notify_all();
				return;
			}
			s.cleanup.pop_front();
		}
		if !s.cleanup.is_empty() {
			continue;
		}
		if current
			.as_ref()
			.is_some_and(|(generation, _, _)| *generation != s.generation)
		{
			current = None;
		}

		let Some(directory) = s.disk.clone() else {
			s.worker = false;
			if s.status == InstallStatus::Pending {
				s.status = InstallStatus::Ready;
			}
			s.cleanup = VecDeque::new();
			SINK.wake.notify_all();
			return;
		};
		if current.is_none() {
			let generation = s.generation;
			drop(s);
			let ready = FileSink::new(directory.clone());
			s = SINK.lock();
			if generation != s.generation {
				drop(s);
				if ready.is_ok() {
					let _ = remove_history(&directory);
				}
				continue;
			}
			match ready {
				Ok(file) => {
					current = Some((generation, directory, file));
					if s.status != InstallStatus::Failed {
						s.status = InstallStatus::Ready;
					}
				}
				Err(_) => {
					s.status = InstallStatus::Failed;
					s.worker = false;
					SINK.wake.notify_all();
					return;
				}
			}
			SINK.wake.notify_all();
		}
		let Some(r) = s.recording.as_mut() else {
			continue;
		};
		if r.queue.is_empty() {
			drop(SINK.wake.wait(s).unwrap_or_else(|p| p.into_inner()));
			continue;
		}
		let batch = std::mem::take(&mut r.queue);
		r.queue_bytes = 0;
		s.pending = batch.len();
		let generation = s.generation;
		drop(s);
		let mut failed = false;
		if let Some((_, _, file)) = current.as_mut() {
			for item in batch {
				// Disabling/reconfiguring drops queued work rather than writing it later.
				if !disk_enabled() || SINK.lock().generation != generation {
					break;
				}
				if file.write(&item.format()).is_err() {
					failed = true;
					break;
				}
			}
		}
		let mut s = SINK.lock();
		s.pending = 0;
		if failed {
			s.status = InstallStatus::Failed;
		}
		SINK.wake.notify_all();
	}
}
struct FileSink {
	directory: PathBuf,
	file: Option<File>,
	bytes: u64,
}
impl FileSink {
	fn new(directory: PathBuf) -> std::io::Result<Self> {
		let file = open_log(&directory)?;
		for name in &LOG_FILES[1..] {
			let path = directory.join(name);
			if let Ok(meta) = std::fs::symlink_metadata(&path)
				&& (!meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_FILE_BYTES)
			{
				remove_managed(&directory, name)?;
			}
		}
		let bytes = file.metadata()?.len();
		let mut sink = Self {
			directory,
			file: Some(file),
			bytes,
		};
		if bytes > MAX_FILE_BYTES {
			sink.file = None;
			remove_managed(&sink.directory, LOG_FILE)?;
			sink.file = Some(open_log(&sink.directory)?);
			sink.bytes = 0;
		}
		Ok(sink)
	}
	fn rotate(&mut self) -> std::io::Result<()> {
		self.file = None;
		remove_managed(&self.directory, LOG_FILES[2])?;
		for index in (0..2).rev() {
			rename_managed(&self.directory, LOG_FILES[index], LOG_FILES[index + 1])?;
		}
		self.file = Some(open_log(&self.directory)?);
		self.bytes = 0;
		Ok(())
	}
	fn write(&mut self, line: &str) -> std::io::Result<()> {
		if self.bytes + line.len() as u64 > MAX_FILE_BYTES {
			self.rotate()?;
		}
		// Reserve attempts before I/O: partial failures must still consume the cap.
		self.bytes += line.len() as u64;
		self.file
			.as_mut()
			.ok_or_else(refused)?
			.write_all(line.as_bytes())?;
		Ok(())
	}
}
#[cfg(unix)]
fn private_dir(directory: &Path) -> std::io::Result<std::os::fd::OwnedFd> {
	use rustix::fs::{self, Mode, OFlags};
	let fd = fs::open(
		directory,
		OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
		Mode::empty(),
	)?;
	if fs::fstat(&fd)?.st_uid != rustix::process::geteuid().as_raw() {
		return Err(refused());
	}
	Ok(fd)
}
fn remove_managed(directory: &Path, name: &str) -> std::io::Result<()> {
	#[cfg(unix)]
	{
		let dir = private_dir(directory)?;
		match rustix::fs::unlinkat(&dir, name, rustix::fs::AtFlags::empty()) {
			Ok(()) => Ok(()),
			Err(e) if e == rustix::io::Errno::NOENT => Ok(()),
			Err(e) => Err(e.into()),
		}
	}
	#[cfg(not(unix))]
	{
		if shared(directory) {
			return Err(refused());
		}
		match std::fs::remove_file(directory.join(name)) {
			Ok(()) => Ok(()),
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(e) => Err(e),
		}
	}
}
fn rename_managed(directory: &Path, from: &str, to: &str) -> std::io::Result<()> {
	#[cfg(unix)]
	{
		let dir = private_dir(directory)?;
		match rustix::fs::renameat(&dir, from, &dir, to) {
			Ok(()) => Ok(()),
			Err(e) if e == rustix::io::Errno::NOENT => Ok(()),
			Err(e) => Err(e.into()),
		}
	}
	#[cfg(not(unix))]
	{
		if shared(directory) {
			return Err(refused());
		}
		match std::fs::rename(directory.join(from), directory.join(to)) {
			Ok(()) => Ok(()),
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(e) => Err(e),
		}
	}
}
fn remove_history(directory: &Path) -> std::io::Result<()> {
	if !directory.exists() {
		return Ok(());
	}
	for name in LOG_FILES {
		remove_managed(directory, name)?;
	}
	Ok(())
}
/// A manually selected report destination is never included in the report itself.
/// Refuse symlinks and special files, restrict new Unix files before writing.
pub fn save_report(path: &Path, report: &str) -> std::io::Result<()> {
	if report.len() > 1024 * 1024 {
		return Err(refused());
	}
	#[cfg(unix)]
	{
		use rustix::fs::{self, FileType, Mode, OFlags};
		let fd = fs::open(
			path,
			OFlags::CREATE | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
			Mode::from_raw_mode(0o600),
		)?;
		let stat = fs::fstat(&fd)?;
		if stat.st_uid != rustix::process::geteuid().as_raw()
			|| stat.st_nlink != 1
			|| FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
		{
			return Err(refused());
		}
		fs::fchmod(&fd, Mode::from_raw_mode(0o600))?;
		fs::ftruncate(&fd, 0)?;
		File::from(fd).write_all(report.as_bytes())
	}
	#[cfg(not(unix))]
	{
		if shared(path) {
			return Err(refused());
		}
		let mut options = std::fs::OpenOptions::new();
		options.create(true).write(true);
		#[cfg(windows)]
		{
			use std::os::windows::fs::OpenOptionsExt;
			options.custom_flags(0x0020_0000);
		}
		let mut file = options.open(path)?;
		if !file.metadata()?.is_file() || reparse(&file.metadata()?) {
			return Err(refused());
		}
		file.set_len(0)?;
		file.write_all(report.as_bytes())
	}
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

/// Optional safe panic marker. Install only once collection is explicitly enabled. The hook
/// never formats PanicHookInfo, location, payload, thread name or a backtrace.
fn install_panic_hook() {
	static HOOK: std::sync::Once = std::sync::Once::new();
	HOOK.call_once(|| {
		let previous = std::panic::take_hook();
		std::panic::set_hook(Box::new(move |info| {
			if is_enabled(Level::Error) {
				record(Category::Runtime, Level::Error, "worker panicked");
			}
			previous(info);
		}));
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	// One global lifecycle scenario avoids parallel tests reconfiguring the process singleton.
	#[test]
	fn opt_in_privacy_bounds_and_disk_lifecycle() {
		let root = tempfile::tempdir().unwrap();
		let logs = root.path().join("logs");
		install_mode(Level::Off, logs.clone(), false);
		record(Category::App, Level::Error, "started");
		assert!(SINK.lock().recording.is_none());
		assert!(!SINK.lock().worker);
		assert!(!logs.exists());
		install_mode(Level::Debug, logs.clone(), false);
		assert!(!SINK.lock().worker);
		assert!(!logs.exists());
		let first_session = recording_session();
		install_mode(Level::Debug, logs.clone(), false);
		assert_eq!(
			recording_session(),
			first_session,
			"unchanged settings keep the session"
		);
		record(Category::App, Level::Info, "started");
		record(
			Category::App,
			Level::Error,
			"private token must not be retained",
		);
		record_fields(
			Category::App,
			Level::Error,
			"started",
			&[("reason", Value::Text("/home/private/name"))],
		);
		assert!(!recent(100).contains("private"));
		assert!(recent(100).contains("started"));
		for i in 0u64..6000 {
			record_fields(
				Category::App,
				Level::Info,
				"started",
				&[("callbacks", i.into())],
			);
		}
		{
			let s = SINK.lock();
			let r = s.recording.as_ref().unwrap();
			assert!(r.ring.len() <= MAX_RING_LINES);
			assert!(r.bytes <= MAX_RING_BYTES);
			assert!(r.queue.is_empty());
			assert!(r.ring.capacity() * std::mem::size_of::<Record>() <= MAX_RING_BYTES);
		}
		// New subsystem summaries displace old samples instead of being silently excluded.
		for scope in ["audio", "transport", "stream_send"] {
			for code in vocabulary::EVENTS {
				summary(Category::Voice, code, &[("scope", scope.into())]);
			}
		}
		summary(Category::Runtime, "worker panicked", &[]);
		{
			let s = SINK.lock();
			let r = s.recording.as_ref().unwrap();
			assert_eq!(r.summaries.len(), 64);
			assert_eq!(r.summaries.last().unwrap().category, Category::Runtime);
		}
		clear();
		assert_ne!(recording_session(), first_session);
		for scope in ["audio", "transport"] {
			summary(
				Category::Voice,
				"media stage",
				&[
					("scope", scope.into()),
					("stage", "encode".into()),
					("calls", 1u64.into()),
					("total_us", 10u64.into()),
					("max_us", 10u64.into()),
				],
			);
		}
		assert_eq!(
			SINK.lock().recording.as_ref().unwrap().summaries.len(),
			2,
			"five-field stages retain both worker scopes"
		);
		// Producers never wait on an exporter/writer holding the collector lock.
		{
			let guard = SINK.lock();
			record(Category::App, Level::Error, "started");
			drop(guard);
		}
		assert!(DROPPED.load(Ordering::Relaxed) > 0);
		install_mode(Level::Debug, logs.clone(), true);
		record(Category::App, Level::Info, "started");
		flush(Duration::from_secs(3));
		assert_eq!(install_status(), InstallStatus::Ready);
		assert!(logs.join(LOG_FILE).exists());
		install_mode(Level::Debug, logs.clone(), false);
		flush(Duration::from_secs(3));
		assert!(!logs.join(LOG_FILE).exists());
		assert!(SINK.lock().recording.is_some());
		assert!(!SINK.lock().worker);
		clear();
		assert!(!recent(100).contains("started"));
		// Reconfiguration cannot discard deletion requests for earlier destinations.
		for index in 0..3 {
			let next = root.path().join(format!("next-{index}"));
			install_mode(Level::Debug, next.clone(), true);
			record(Category::App, Level::Info, "started");
			install_mode(Level::Off, next, false);
		}
		flush(Duration::from_secs(3));
		for index in 0..3 {
			assert!(
				!root
					.path()
					.join(format!("next-{index}"))
					.join(LOG_FILE)
					.exists()
			);
		}
		install_mode(Level::Off, logs, false);
		assert!(SINK.lock().recording.is_none());
		install_mode(Level::Debug, PathBuf::new(), true);
		assert_eq!(install_status(), InstallStatus::Failed);
		assert!(!SINK.lock().worker);
		install_mode(Level::Off, PathBuf::new(), false);
		let blocked = root.path().join("blocked");
		std::fs::create_dir_all(blocked.join(LOG_FILE)).unwrap();
		clear_at(Some(blocked.clone()));
		flush(Duration::from_secs(3));
		assert_eq!(install_status(), InstallStatus::Failed);
		assert!(SINK.lock().cleanup.contains(&blocked));
		install_mode(Level::Debug, blocked.clone(), true);
		flush(Duration::from_secs(3));
		assert_eq!(install_status(), InstallStatus::Failed);
		assert!(
			SINK.lock().cleanup.contains(&blocked),
			"setup cannot forget failed deletion"
		);
		std::fs::remove_dir(blocked.join(LOG_FILE)).unwrap();
		install_mode(Level::Debug, blocked.clone(), true);
		flush(Duration::from_secs(3));
		assert_eq!(install_status(), InstallStatus::Ready);
		assert!(SINK.lock().cleanup.is_empty());
		let previous = recording_session();
		install_mode(Level::Off, blocked.clone(), false);
		flush(Duration::from_secs(3));
		install_mode(Level::Debug, blocked, false);
		assert_ne!(recording_session(), previous);
		install_mode(Level::Off, PathBuf::new(), false);
	}
	#[test]
	fn rotation_reopen_and_unsafe_destinations() {
		let root = tempfile::tempdir().unwrap();
		let logs = root.path().join("logs");
		let mut file = FileSink::new(logs.clone()).unwrap();
		let line = "x".repeat(512);
		for _ in 0..(MAX_FILE_BYTES as usize * 4 / line.len() + 1) {
			file.write(&line).unwrap();
		}
		drop(file);
		// A failed write (including a partial write) consumes the full reserved attempt.
		let mut failed = FileSink::new(logs.clone()).unwrap();
		failed.file = Some(File::open(logs.join(LOG_FILE)).unwrap());
		let before = failed.bytes;
		assert!(failed.write("failed write").is_err());
		assert_eq!(failed.bytes, before + 12);
		drop(failed);
		let total: u64 = LOG_FILES
			.iter()
			.map(|name| std::fs::metadata(logs.join(name)).unwrap().len())
			.sum();
		assert!(total <= MAX_FILE_BYTES * 3);
		std::fs::write(logs.join(LOG_FILE), vec![0; MAX_FILE_BYTES as usize + 1]).unwrap();
		drop(FileSink::new(logs.clone()).unwrap());
		assert!(std::fs::metadata(logs.join(LOG_FILE)).unwrap().len() <= MAX_FILE_BYTES);
		#[cfg(unix)]
		{
			let target = root.path().join("target");
			std::fs::write(&target, b"untouched").unwrap();
			std::fs::remove_file(logs.join(LOG_FILE)).unwrap();
			std::os::unix::fs::symlink(&target, logs.join(LOG_FILE)).unwrap();
			assert!(FileSink::new(logs.clone()).is_err());
			assert!(save_report(&logs.join(LOG_FILE), "safe report").is_err());
			assert_eq!(std::fs::read(target).unwrap(), b"untouched");
			std::fs::remove_file(logs.join(LOG_FILE)).unwrap();
			#[cfg(not(target_os = "macos"))]
			{
				rustix::fs::mkfifoat(
					rustix::fs::CWD,
					logs.join(LOG_FILE),
					rustix::fs::Mode::from_raw_mode(0o600),
				)
				.unwrap();
				assert!(FileSink::new(logs).is_err());
			}
		}
	}
}
