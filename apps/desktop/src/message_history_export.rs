//! Bounded, database-free message-history export.
//!
//! The host supplies one snapshot iterator of already-loaded rows. This module streams them to a
//! user-selected path through a private temporary file and only publishes a complete file. It has
//! no store, UI or transport dependency. The export view deliberately carries no tokens, signed
//! media URLs or local paths; attachment rows keep only a display name, content type and size.

use serde::Serialize;
use std::{
	fs::{self, File, OpenOptions},
	io::{self, Write},
	path::{Path, PathBuf},
	sync::atomic::{AtomicBool, Ordering},
};

/// Hard row cap for one export.
pub const MAX_ROWS: u64 = 50_000;
/// Hard serialized-byte cap for one export.
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Matches the bounded message body retained by the logger.
pub const MAX_CONTENT_BYTES: usize = 16 * 1024;
const MAX_AUTHOR_BYTES: usize = 256;
const MAX_ATTACHMENTS: usize = 10;
const MAX_CHANGES: usize = 8;
const MAX_ATTACHMENT_NAME_BYTES: usize = 1024;
const MAX_CONTENT_TYPE_BYTES: usize = 128;

/// One row from a single-account snapshot. The host maps its local-store record into this view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportChangeKind {
	Edit,
	Delete,
}

/// One retained before/after transition. Signed URLs and reply/gateway payloads are excluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportChange {
	pub kind: ExportChangeKind,
	pub revision: u64,
	pub before: String,
	pub after: Option<String>,
	pub observed_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportRecord {
	/// Must equal [`ExportScope::account`]; prevents a mixed-account iterator.
	pub account: u64,
	pub channel: u64,
	pub message: u64,
	pub author: u64,
	pub author_name: String,
	pub content: String,
	pub edited: bool,
	pub deleted: bool,
	pub timestamp: Option<i64>,
	pub changes: Vec<ExportChange>,
	pub attachments: Vec<ExportAttachment>,
}

/// Attachment metadata safe to export. There is intentionally no URL field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportAttachment {
	pub filename: String,
	pub content_type: Option<String>,
	pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportScope {
	pub account: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
	/// The host must set this only after the native overwrite confirmation.
	pub overwrite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportSummary {
	pub path: PathBuf,
	pub rows: u64,
	pub bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportError {
	InvalidDestination,
	DestinationExists,
	Cancelled,
	TooManyRows,
	ByteLimit,
	InvalidRecord(&'static str),
	Io(&'static str),
	Serialization,
}

impl ExportError {
	pub fn message(self) -> &'static str {
		match self {
			Self::InvalidDestination => "Choose an existing folder for the export",
			Self::DestinationExists => "A file already exists at the export destination",
			Self::Cancelled => "Export cancelled",
			Self::TooManyRows => "Export exceeds the 50,000 row limit",
			Self::ByteLimit => "Export exceeds the 256 MiB limit",
			Self::InvalidRecord(_) => "Export data contains an invalid row",
			Self::Io(_) => "Could not write the export file",
			Self::Serialization => "Could not serialize the export",
		}
	}
}

impl std::fmt::Display for ExportError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(self.message())
	}
}

impl std::error::Error for ExportError {}

/// Stream human-readable text. `rows` must be a bounded single-account snapshot cursor.
#[cfg(test)]
pub fn export_text<I>(
	destination: &Path,
	scope: ExportScope,
	rows: I,
	options: ExportOptions,
	cancelled: &AtomicBool,
) -> Result<ExportSummary, ExportError>
where
	I: Iterator<Item = ExportRecord>,
{
	export_text_results(destination, scope, rows.map(Ok), options, cancelled)
}

/// Fallible rows stop before the temporary export can be published.
pub fn export_text_results<I>(
	destination: &Path,
	scope: ExportScope,
	rows: I,
	options: ExportOptions,
	cancelled: &AtomicBool,
) -> Result<ExportSummary, ExportError>
where
	I: Iterator<Item = Result<ExportRecord, ExportError>>,
{
	export(
		destination,
		scope,
		rows,
		options,
		cancelled,
		Format::Text,
		Limits::default(),
	)
}

/// Stream a JSON document with one object per row. No media URL is serialized.
#[cfg(test)]
pub fn export_json<I>(
	destination: &Path,
	scope: ExportScope,
	rows: I,
	options: ExportOptions,
	cancelled: &AtomicBool,
) -> Result<ExportSummary, ExportError>
where
	I: Iterator<Item = ExportRecord>,
{
	export_json_results(destination, scope, rows.map(Ok), options, cancelled)
}

/// Fallible rows stop before the temporary export can be published.
pub fn export_json_results<I>(
	destination: &Path,
	scope: ExportScope,
	rows: I,
	options: ExportOptions,
	cancelled: &AtomicBool,
) -> Result<ExportSummary, ExportError>
where
	I: Iterator<Item = Result<ExportRecord, ExportError>>,
{
	export(
		destination,
		scope,
		rows,
		options,
		cancelled,
		Format::Json,
		Limits::default(),
	)
}

#[derive(Clone, Copy)]
enum Format {
	Text,
	Json,
}

#[derive(Clone, Copy)]
struct Limits {
	rows: u64,
	bytes: u64,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			rows: MAX_ROWS,
			bytes: MAX_BYTES,
		}
	}
}

fn export<I>(
	destination: &Path,
	scope: ExportScope,
	rows: I,
	options: ExportOptions,
	cancelled: &AtomicBool,
	format: Format,
	limits: Limits,
) -> Result<ExportSummary, ExportError>
where
	I: Iterator<Item = Result<ExportRecord, ExportError>>,
{
	if scope.account == 0 {
		return Err(ExportError::InvalidRecord("account"));
	}
	if cancelled.load(Ordering::Acquire) {
		return Err(ExportError::Cancelled);
	}
	validate_destination(destination, options.overwrite)?;
	let mut partial = PartialExport::new(destination)?;
	let (result, exceeded) = {
		let file = partial.file.as_mut().expect("partial export file");
		let mut writer = LimitedWriter::new(file, limits.bytes);
		let result = match format {
			Format::Text => write_text(&mut writer, scope, rows, cancelled, limits),
			Format::Json => write_json(&mut writer, scope, rows, cancelled, limits),
		};
		let bytes = writer.written();
		let exceeded = writer.exceeded;
		(result.map(|rows| (rows, bytes)), exceeded)
	};
	let (rows, bytes) = match result {
		Ok(result) => result,
		Err(_) if exceeded => return Err(ExportError::ByteLimit),
		Err(error) => return Err(error),
	};
	partial.commit(destination, options.overwrite, cancelled)?;
	Ok(ExportSummary {
		path: destination.to_path_buf(),
		rows,
		bytes,
	})
}

fn write_text<I>(
	writer: &mut LimitedWriter<'_>,
	scope: ExportScope,
	rows: I,
	cancelled: &AtomicBool,
	limits: Limits,
) -> Result<u64, ExportError>
where
	I: Iterator<Item = Result<ExportRecord, ExportError>>,
{
	write!(
		writer,
		"Serein message history export\nAccount: {}\n\n",
		scope.account
	)
	.map_err(|_| ExportError::Io("Could not write the export file"))?;
	let mut count = 0u64;
	for row in rows {
		let row = row?;
		if cancelled.load(Ordering::Acquire) {
			return Err(ExportError::Cancelled);
		}
		if count >= limits.rows {
			return Err(ExportError::TooManyRows);
		}
		validate_record(&row, scope)?;
		if count > 0 {
			writer
				.write_all(b"---\n")
				.map_err(|_| ExportError::Io("Could not write the export file"))?;
		}
		writeln!(
			writer,
			"Channel: {}\nMessage: {}\nAuthor: {} ({})\nEdited: {}\nDeleted: {}\nTimestamp: {}\nContent:\n{}\n",
			row.channel,
			row.message,
			row.author_name,
			row.author,
			if row.edited { "yes" } else { "no" },
			if row.deleted { "yes" } else { "no" },
			row.timestamp
				.map(|timestamp| timestamp.to_string())
				.unwrap_or_else(|| "-".into()),
			row.content
		)
		.map_err(|_| ExportError::Io("Could not write the export file"))?;
		for change in &row.changes {
			writeln!(
				writer,
				"History: {}\nRevision: {}\nObserved: {}\nBefore:\n{}\nAfter:\n{}",
				match change.kind {
					ExportChangeKind::Edit => "edit",
					ExportChangeKind::Delete => "delete",
				},
				change.revision,
				change.observed_at,
				change.before,
				change.after.as_deref().unwrap_or("-")
			)
			.map_err(|_| ExportError::Io("Could not write the export file"))?;
		}
		if !row.attachments.is_empty() {
			writer
				.write_all(b"Attachments:\n")
				.map_err(|_| ExportError::Io("Could not write the export file"))?;
			for attachment in &row.attachments {
				writeln!(
					writer,
					"- {} ({}, {} bytes)",
					attachment.filename,
					attachment.content_type.as_deref().unwrap_or("-"),
					attachment.size
				)
				.map_err(|_| ExportError::Io("Could not write the export file"))?;
			}
		}
		count += 1;
	}
	Ok(count)
}

#[derive(Serialize)]
struct JsonRow<'a> {
	channel: u64,
	message: u64,
	author: u64,
	author_name: &'a str,
	content: &'a str,
	edited: bool,
	deleted: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	timestamp: Option<i64>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	changes: Vec<JsonChange<'a>>,
	#[serde(skip_serializing_if = "Vec::is_empty")]
	attachments: Vec<JsonAttachment<'a>>,
}

#[derive(Serialize)]
struct JsonChange<'a> {
	kind: &'static str,
	revision: u64,
	before: &'a str,
	#[serde(skip_serializing_if = "Option::is_none")]
	after: Option<&'a str>,
	observed_at: i64,
}

#[derive(Serialize)]
struct JsonAttachment<'a> {
	filename: &'a str,
	#[serde(skip_serializing_if = "Option::is_none")]
	content_type: Option<&'a str>,
	size: u64,
}

fn write_json<I>(
	writer: &mut LimitedWriter<'_>,
	scope: ExportScope,
	rows: I,
	cancelled: &AtomicBool,
	limits: Limits,
) -> Result<u64, ExportError>
where
	I: Iterator<Item = Result<ExportRecord, ExportError>>,
{
	write!(
		writer,
		"{{\"format\":\"serein-message-history\",\"version\":1,\"account\":{},\"rows\":[",
		scope.account
	)
	.map_err(|_| ExportError::Io("Could not write the export file"))?;
	let mut count = 0u64;
	for row in rows {
		let row = row?;
		if cancelled.load(Ordering::Acquire) {
			return Err(ExportError::Cancelled);
		}
		if count >= limits.rows {
			return Err(ExportError::TooManyRows);
		}
		validate_record(&row, scope)?;
		if count > 0 {
			writer
				.write_all(b",")
				.map_err(|_| ExportError::Io("Could not write the export file"))?;
		}
		let changes = row
			.changes
			.iter()
			.map(|change| JsonChange {
				kind: match change.kind {
					ExportChangeKind::Edit => "edit",
					ExportChangeKind::Delete => "delete",
				},
				revision: change.revision,
				before: &change.before,
				after: change.after.as_deref(),
				observed_at: change.observed_at,
			})
			.collect();
		let attachments = row
			.attachments
			.iter()
			.map(|attachment| JsonAttachment {
				filename: &attachment.filename,
				content_type: attachment.content_type.as_deref(),
				size: attachment.size,
			})
			.collect();
		let json = JsonRow {
			channel: row.channel,
			message: row.message,
			author: row.author,
			author_name: &row.author_name,
			content: &row.content,
			edited: row.edited,
			deleted: row.deleted,
			timestamp: row.timestamp,
			changes,
			attachments,
		};
		if serde_json::to_writer(&mut *writer, &json).is_err() {
			return Err(if writer.exceeded {
				ExportError::ByteLimit
			} else {
				ExportError::Serialization
			});
		}
		count += 1;
	}
	writer
		.write_all(b"]}\n")
		.map_err(|_| ExportError::Io("Could not write the export file"))?;
	Ok(count)
}

fn validate_record(row: &ExportRecord, scope: ExportScope) -> Result<(), ExportError> {
	if row.account == 0 || row.account != scope.account {
		return Err(ExportError::InvalidRecord("account"));
	}
	if row.channel == 0 || row.message == 0 || row.author == 0 {
		return Err(ExportError::InvalidRecord("message identity"));
	}
	if row.author_name.len() > MAX_AUTHOR_BYTES
		|| row.author_name.contains('\0')
		|| row.author_name.chars().any(char::is_control)
	{
		return Err(ExportError::InvalidRecord("author"));
	}
	if row.content.len() > MAX_CONTENT_BYTES || row.content.contains('\0') {
		return Err(ExportError::InvalidRecord("content"));
	}
	if row.changes.len() > MAX_CHANGES {
		return Err(ExportError::InvalidRecord("changes"));
	}
	for change in &row.changes {
		let bounded = |text: &str| text.len() <= MAX_CONTENT_BYTES && !text.contains('\0');
		if change.observed_at < 0
			|| !bounded(&change.before)
			|| !change.after.as_deref().is_none_or(bounded)
			|| match change.kind {
				ExportChangeKind::Edit => change.after.is_none(),
				ExportChangeKind::Delete => change.after.is_some(),
			} {
			return Err(ExportError::InvalidRecord("change"));
		}
	}
	if row.attachments.len() > MAX_ATTACHMENTS {
		return Err(ExportError::InvalidRecord("attachments"));
	}
	for attachment in &row.attachments {
		if attachment.filename.is_empty()
			|| attachment.filename.len() > MAX_ATTACHMENT_NAME_BYTES
			|| attachment.filename.contains('\0')
			|| attachment
				.filename
				.chars()
				.any(|character| character == '/' || character == '\\' || character.is_control())
		{
			return Err(ExportError::InvalidRecord("attachment"));
		}
		if attachment
			.content_type
			.as_ref()
			.is_some_and(|content_type| {
				content_type.len() > MAX_CONTENT_TYPE_BYTES
					|| content_type.contains('\0')
					|| content_type.chars().any(char::is_control)
			}) {
			return Err(ExportError::InvalidRecord("attachment content type"));
		}
	}
	Ok(())
}

fn validate_destination(destination: &Path, overwrite: bool) -> Result<(), ExportError> {
	if destination.as_os_str().is_empty() {
		return Err(ExportError::InvalidDestination);
	}
	let parent = destination
		.parent()
		.filter(|parent| !parent.as_os_str().is_empty())
		.unwrap_or_else(|| Path::new("."));
	if !parent.is_dir() {
		return Err(ExportError::InvalidDestination);
	}
	match fs::symlink_metadata(destination) {
		Ok(metadata) => {
			if metadata.is_dir() {
				return Err(ExportError::InvalidDestination);
			}
			if !overwrite {
				return Err(ExportError::DestinationExists);
			}
		}
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
		Err(_) => return Err(ExportError::InvalidDestination),
	}
	Ok(())
}

struct PartialExport {
	path: PathBuf,
	file: Option<File>,
	committed: bool,
}

impl PartialExport {
	fn new(destination: &Path) -> Result<Self, ExportError> {
		let parent = destination
			.parent()
			.filter(|parent| !parent.as_os_str().is_empty())
			.unwrap_or_else(|| Path::new("."));
		let mut random = [0u8; 16];
		getrandom::fill(&mut random)
			.map_err(|_| ExportError::Io("Export temporary file unavailable"))?;
		let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
		let path = parent.join(format!(".serein-export-{suffix}.partial"));
		let mut options = OpenOptions::new();
		options.write(true).create_new(true);
		#[cfg(unix)]
		{
			use std::os::unix::fs::OpenOptionsExt;
			options.mode(0o600);
		}
		let file = options
			.open(&path)
			.map_err(|_| ExportError::Io("Cannot create the export file"))?;
		Ok(Self {
			path,
			file: Some(file),
			committed: false,
		})
	}

	fn commit(
		mut self,
		destination: &Path,
		overwrite: bool,
		cancelled: &AtomicBool,
	) -> Result<(), ExportError> {
		if cancelled.load(Ordering::Acquire) {
			return Err(ExportError::Cancelled);
		}
		let file = self.file.take().expect("partial export file");
		file.sync_all()
			.map_err(|_| ExportError::Io("Could not finish the export file"))?;
		drop(file);
		if overwrite {
			fs::rename(&self.path, destination)
				.map_err(|_| ExportError::Io("Could not replace the export file"))?;
		} else {
			platform::save::publish_new(&self.path, destination).map_err(|error| {
				if error.kind() == std::io::ErrorKind::AlreadyExists {
					ExportError::DestinationExists
				} else {
					ExportError::Io("Could not publish the export file")
				}
			})?;
		}
		self.committed = true;
		Ok(())
	}
}

impl Drop for PartialExport {
	fn drop(&mut self) {
		self.file.take();
		if !self.committed {
			let _ = fs::remove_file(&self.path);
		}
	}
}

struct LimitedWriter<'a> {
	inner: &'a mut File,
	remaining: u64,
	written: u64,
	exceeded: bool,
}

impl<'a> LimitedWriter<'a> {
	fn new(inner: &'a mut File, limit: u64) -> Self {
		Self {
			inner,
			remaining: limit,
			written: 0,
			exceeded: false,
		}
	}

	fn written(&self) -> u64 {
		self.written
	}
}

impl Write for LimitedWriter<'_> {
	fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
		if buffer.len() as u64 > self.remaining {
			self.exceeded = true;
			return Err(io::Error::new(
				io::ErrorKind::WriteZero,
				"export byte limit",
			));
		}
		let written = self.inner.write(buffer)?;
		self.remaining = self.remaining.saturating_sub(written as u64);
		self.written = self.written.saturating_add(written as u64);
		Ok(written)
	}

	fn flush(&mut self) -> io::Result<()> {
		self.inner.flush()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::{
		Arc,
		atomic::{AtomicU64, Ordering},
	};

	fn directory() -> PathBuf {
		static NEXT: AtomicU64 = AtomicU64::new(0);
		let directory = std::env::temp_dir().join(format!(
			"serein-export-tests-{}-{}",
			std::process::id(),
			NEXT.fetch_add(1, Ordering::Relaxed)
		));
		let _ = fs::remove_dir_all(&directory);
		fs::create_dir_all(&directory).unwrap();
		directory
	}

	fn record(account: u64, channel: u64, message: u64) -> ExportRecord {
		ExportRecord {
			account,
			channel,
			message,
			author: 9,
			author_name: "Synthetic".into(),
			content: "synthetic body".into(),
			edited: false,
			deleted: false,
			timestamp: Some(1_700_000_000),
			changes: vec![ExportChange {
				kind: ExportChangeKind::Edit,
				revision: 4,
				before: "synthetic old body".into(),
				after: Some("synthetic body".into()),
				observed_at: 1_700_000_000,
			}],
			attachments: vec![ExportAttachment {
				filename: "synthetic image.png".into(),
				content_type: Some("image/png".into()),
				size: 1024,
			}],
		}
	}

	#[test]
	fn json_export_is_account_scoped_and_contains_no_urls_or_paths() {
		let directory = directory();
		let destination = directory.join("history.json");
		let scope = ExportScope { account: 7 };
		let rows = [record(7, 2, 10), record(7, 2, 11)];
		let summary = export_json(
			&destination,
			scope,
			rows.into_iter(),
			ExportOptions::default(),
			&AtomicBool::new(false),
		)
		.unwrap();
		assert_eq!(summary.rows, 2);
		assert_eq!(summary.bytes, fs::metadata(&destination).unwrap().len());
		let text = fs::read_to_string(&destination).unwrap();
		assert!(!text.contains("https://"));
		assert!(!text.contains("cdn.discordapp"));
		assert!(!text.contains("http://"));
		assert!(!text.contains(&directory.to_string_lossy().to_string()));
		let json: serde_json::Value = serde_json::from_str(&text).unwrap();
		assert_eq!(json["account"].as_u64(), Some(7));
		assert_eq!(json["rows"].as_array().unwrap().len(), 2);
		assert_eq!(json["rows"][0]["message"].as_u64(), Some(10));
		assert_eq!(
			json["rows"][0]["changes"][0]["before"].as_str(),
			Some("synthetic old body")
		);
		assert_eq!(json["rows"][0]["changes"][0]["kind"].as_str(), Some("edit"));
		assert_eq!(
			json["rows"][0]["attachments"][0]["filename"].as_str(),
			Some("synthetic image.png")
		);
		assert!(!text.contains("\"url\""));
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			assert_eq!(
				fs::metadata(&destination).unwrap().permissions().mode() & 0o077,
				0
			);
		}
		let _ = fs::remove_dir_all(directory);
	}

	#[test]
	fn text_export_and_account_mismatch_leave_no_output() {
		let directory = directory();
		let destination = directory.join("history.txt");
		let scope = ExportScope { account: 7 };
		let summary = export_text(
			&destination,
			scope,
			[record(7, 2, 10)].into_iter(),
			ExportOptions::default(),
			&AtomicBool::new(false),
		)
		.unwrap();
		assert_eq!(summary.rows, 1);
		let text = fs::read_to_string(&destination).unwrap();
		assert!(text.contains("Account: 7"));
		assert!(text.contains("synthetic body"));
		assert!(text.contains("synthetic old body"));
		assert!(!text.contains("https://"));
		assert!(!text.contains("cdn.discordapp"));
		fs::remove_file(&destination).unwrap();

		let mismatch = destination.clone();
		let result = export_text(
			&mismatch,
			scope,
			[record(8, 2, 10)].into_iter(),
			ExportOptions::default(),
			&AtomicBool::new(false),
		);
		assert_eq!(result, Err(ExportError::InvalidRecord("account")));
		assert!(!mismatch.exists());
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
		let _ = fs::remove_dir_all(directory);
	}

	#[test]
	fn row_byte_and_cancellation_limits_remove_partial_files() {
		let directory = directory();
		let scope = ExportScope { account: 7 };
		let destination = directory.join("history.json");
		let result = export(
			&destination,
			scope,
			[record(7, 2, 10), record(7, 2, 11)].into_iter().map(Ok),
			ExportOptions::default(),
			&AtomicBool::new(false),
			Format::Json,
			Limits {
				rows: 1,
				bytes: 64 * 1024,
			},
		);
		assert_eq!(result, Err(ExportError::TooManyRows));
		assert!(!destination.exists());
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);

		let mut large = record(7, 2, 10);
		large.content = "x".repeat(4096);
		let result = export(
			&destination,
			scope,
			[large].into_iter().map(Ok),
			ExportOptions::default(),
			&AtomicBool::new(false),
			Format::Text,
			Limits {
				rows: 10,
				bytes: 512,
			},
		);
		assert_eq!(result, Err(ExportError::ByteLimit));
		assert!(!destination.exists());
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);

		let cancelled = Arc::new(AtomicBool::new(false));
		let flag = cancelled.clone();
		let rows = (0..4).map(move |message| {
			if message == 2 {
				flag.store(true, Ordering::Release);
			}
			Ok(record(7, 2, 10 + message))
		});
		let result = export(
			&destination,
			scope,
			rows,
			ExportOptions::default(),
			&cancelled,
			Format::Json,
			Limits {
				rows: 10,
				bytes: 64 * 1024,
			},
		);
		assert_eq!(result, Err(ExportError::Cancelled));
		assert!(!destination.exists());
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
		let _ = fs::remove_dir_all(directory);
	}

	#[test]
	fn existing_destination_requires_explicit_overwrite() {
		let directory = directory();
		let destination = directory.join("history.txt");
		fs::write(&destination, b"keep").unwrap();
		let result = export_text(
			&destination,
			ExportScope { account: 7 },
			[record(7, 2, 10)].into_iter(),
			ExportOptions::default(),
			&AtomicBool::new(false),
		);
		assert_eq!(result, Err(ExportError::DestinationExists));
		assert_eq!(fs::read(&destination).unwrap(), b"keep".to_vec());
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
		export_text(
			&destination,
			ExportScope { account: 7 },
			[record(7, 2, 10)].into_iter(),
			ExportOptions { overwrite: true },
			&AtomicBool::new(false),
		)
		.unwrap();
		assert!(
			fs::read_to_string(&destination)
				.unwrap()
				.contains("synthetic body")
		);
		let _ = fs::remove_dir_all(directory);
	}

	#[test]
	fn invalid_attachment_paths_and_oversized_content_are_rejected() {
		let directory = directory();
		let destination = directory.join("history.json");
		let mut row = record(7, 2, 10);
		row.attachments[0].filename = "../escape.png".into();
		assert_eq!(
			export_json(
				&destination,
				ExportScope { account: 7 },
				[row].into_iter(),
				ExportOptions::default(),
				&AtomicBool::new(false),
			),
			Err(ExportError::InvalidRecord("attachment"))
		);
		assert!(!destination.exists());
		let mut row = record(7, 2, 10);
		row.content = "x".repeat(MAX_CONTENT_BYTES + 1);
		assert_eq!(
			export_text(
				&destination,
				ExportScope { account: 7 },
				[row].into_iter(),
				ExportOptions::default(),
				&AtomicBool::new(false),
			),
			Err(ExportError::InvalidRecord("content"))
		);
		assert!(!destination.exists());
		assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
		let _ = fs::remove_dir_all(directory);
	}
}
