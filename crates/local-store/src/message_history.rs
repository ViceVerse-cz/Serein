//! Persistent, account-isolated MessageLogger storage.
//!
//! This module owns only validated SQLite records. It does not fetch messages, request downloads,
//! touch the filesystem or run a media worker. The host must check [`HistorySettings::enabled`]
//! before recording new observations; the storage primitives themselves stay usable so existing
//! rows can still be viewed, retained, cleared or repaired while capture is off.
//!
//! Attachment rows contain metadata and download accounting only. Signed/CDN URLs, local paths,
//! tokens, cookies and gateway payloads are deliberately not representable here. The host performs
//! downloads and records reservations/completions through the bounded helpers below.
//!
//! `serde::Serialize` is implemented only for the explicit user-export page/row types. Internal
//! settings and write results are not serialized, and none of these types belong in the Wasm or
//! extension SDK surface.

use super::{LocalStore, Result, StoreError};
use model::Id;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

/// Largest retained body per message. Matches the history model limit.
pub const HISTORY_MAX_CONTENT_BYTES: usize = 16 * 1024;
/// Newest retained changes per message. Older changes are evicted first.
pub const HISTORY_MAX_CHANGES: usize = 8;
/// Newest retained attachment metadata rows per message.
pub const HISTORY_MAX_ATTACHMENTS: usize = 10;
/// Viewer page size. Larger requests are clamped to this value.
pub const HISTORY_PAGE_MAX: u32 = 100;
/// Absolute bound for one export page.
pub const HISTORY_PAGE_HARD_MAX: u32 = 500;
/// Maximum ignore-list entries per user list and per channel list.
pub const HISTORY_FILTER_MAX: usize = 512;
/// Maximum number of message IDs accepted by one bulk tombstone operation.
pub const HISTORY_BULK_MAX: usize = 1000;
pub const HISTORY_SETTINGS_RETENTION_DAYS_MIN: u32 = 1;
pub const HISTORY_SETTINGS_RETENTION_DAYS_MAX: u32 = 365;
pub const HISTORY_SETTINGS_MESSAGES_MIN: u32 = 100;
pub const HISTORY_SETTINGS_MESSAGES_MAX: u32 = 50_000;
pub const HISTORY_SETTINGS_BYTES_MIN: u64 = 4 * 1024 * 1024;
pub const HISTORY_SETTINGS_BYTES_MAX: u64 = 64 * 1024 * 1024;
pub const HISTORY_SETTINGS_BYTES_DEFAULT: u64 = 32 * 1024 * 1024;
/// Total reservation/storage budget for downloaded attachment bytes per account.
pub const HISTORY_ATTACHMENT_ACCOUNT_BYTES: u64 = 256 * 1024 * 1024;
pub const HISTORY_ATTACHMENT_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
pub const HISTORY_ATTACHMENT_AUDIO_BYTES: u64 = 16 * 1024 * 1024;
pub const HISTORY_ATTACHMENT_VIDEO_BYTES: u64 = 32 * 1024 * 1024;
/// Generic files use the smallest per-file budget; they are not treated like video.
pub const HISTORY_ATTACHMENT_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const HISTORY_SCHEMA_VERSION: u32 = 31;

const HISTORY_DAY_MS: i64 = 86_400_000;
const STATE_NOT_REQUESTED: i64 = 0;
const STATE_RESERVED: i64 = 1;
const STATE_STORED: i64 = 2;
const STATE_FAILED: i64 = 3;

const HISTORY_SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS history_settings(
    account TEXT PRIMARY KEY NOT NULL CHECK(typeof(account)='text' AND length(account) BETWEEN 1 AND 20),
    enabled INTEGER NOT NULL CHECK(typeof(enabled)='integer' AND enabled IN (0,1)),
    retention_days INTEGER NOT NULL CHECK(typeof(retention_days)='integer' AND retention_days BETWEEN 1 AND 365),
    max_messages INTEGER NOT NULL CHECK(typeof(max_messages)='integer' AND max_messages BETWEEN 100 AND 50000),
    max_bytes INTEGER NOT NULL CHECK(typeof(max_bytes)='integer' AND max_bytes BETWEEN 4194304 AND 67108864),
    alert_mode INTEGER NOT NULL CHECK(typeof(alert_mode)='integer' AND alert_mode IN (0,1,2)),
    download_images INTEGER NOT NULL CHECK(typeof(download_images)='integer' AND download_images IN (0,1)),
    download_audio INTEGER NOT NULL CHECK(typeof(download_audio)='integer' AND download_audio IN (0,1)),
    download_video INTEGER NOT NULL CHECK(typeof(download_video)='integer' AND download_video IN (0,1)),
    download_files INTEGER NOT NULL CHECK(typeof(download_files)='integer' AND download_files IN (0,1))
);
CREATE TABLE IF NOT EXISTS history_filters(
    account TEXT NOT NULL CHECK(typeof(account)='text' AND length(account) BETWEEN 1 AND 20),
    kind INTEGER NOT NULL CHECK(typeof(kind)='integer' AND kind IN (0,1)),
    filter_id TEXT NOT NULL CHECK(typeof(filter_id)='text' AND length(filter_id) BETWEEN 1 AND 20),
    PRIMARY KEY(account,kind,filter_id)
);
CREATE TABLE IF NOT EXISTS history_messages(
    account TEXT NOT NULL CHECK(typeof(account)='text' AND length(account) BETWEEN 1 AND 20),
    channel TEXT NOT NULL CHECK(typeof(channel)='text' AND length(channel) BETWEEN 1 AND 20),
    id TEXT NOT NULL CHECK(typeof(id)='text' AND length(id) BETWEEN 1 AND 20),
    author TEXT NOT NULL CHECK(typeof(author)='text' AND length(author) BETWEEN 1 AND 20),
    content TEXT NOT NULL CHECK(typeof(content)='text' AND length(CAST(content AS BLOB))<=16384),
    revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0),
    deleted INTEGER NOT NULL CHECK(typeof(deleted)='integer' AND deleted IN (0,1)),
    updated_at INTEGER NOT NULL CHECK(typeof(updated_at)='integer' AND updated_at>=0),
    PRIMARY KEY(account,channel,id)
);
CREATE INDEX IF NOT EXISTS history_messages_order
    ON history_messages(account,updated_at,channel,id);
CREATE INDEX IF NOT EXISTS history_messages_channel_order
    ON history_messages(account,channel,updated_at,id);
CREATE TABLE IF NOT EXISTS history_tombstones(
    account TEXT NOT NULL CHECK(typeof(account)='text' AND length(account) BETWEEN 1 AND 20),
    channel TEXT NOT NULL CHECK(typeof(channel)='text' AND length(channel) BETWEEN 1 AND 20),
    message TEXT NOT NULL CHECK(typeof(message)='text' AND length(message) BETWEEN 1 AND 20),
    observed_at INTEGER NOT NULL CHECK(typeof(observed_at)='integer' AND observed_at>=0),
    PRIMARY KEY(account,channel,message)
);
CREATE INDEX IF NOT EXISTS history_tombstones_order
    ON history_tombstones(account,observed_at,channel,message);
CREATE TABLE IF NOT EXISTS history_changes(
    account TEXT NOT NULL,
    channel TEXT NOT NULL,
    message TEXT NOT NULL,
    seq INTEGER NOT NULL CHECK(typeof(seq)='integer' AND seq>=1),
    kind INTEGER NOT NULL CHECK(typeof(kind)='integer' AND kind IN (0,1)),
    revision INTEGER NOT NULL CHECK(typeof(revision)='integer' AND revision>=0),
    before TEXT NOT NULL CHECK(typeof(before)='text' AND length(CAST(before AS BLOB))<=16384),
    after TEXT CHECK(after IS NULL OR (typeof(after)='text' AND length(CAST(after AS BLOB))<=16384)),
    observed_at INTEGER NOT NULL CHECK(typeof(observed_at)='integer' AND observed_at>=0),
    PRIMARY KEY(account,channel,message,seq),
    CHECK((kind=0 AND after IS NOT NULL) OR (kind=1 AND after IS NULL))
);
CREATE TABLE IF NOT EXISTS history_attachments(
    account TEXT NOT NULL,
    channel TEXT NOT NULL,
    message TEXT NOT NULL,
    attachment_id TEXT NOT NULL CHECK(typeof(attachment_id)='text' AND length(attachment_id) BETWEEN 1 AND 20),
    filename TEXT NOT NULL CHECK(typeof(filename)='text' AND length(CAST(filename AS BLOB)) BETWEEN 1 AND 256),
    content_type TEXT CHECK(content_type IS NULL OR (typeof(content_type)='text' AND length(CAST(content_type AS BLOB))<=127)),
    size INTEGER NOT NULL CHECK(typeof(size)='integer' AND size>=0),
    kind INTEGER NOT NULL CHECK(typeof(kind)='integer' AND kind IN (0,1,2,3)),
    download_state INTEGER NOT NULL CHECK(typeof(download_state)='integer' AND download_state IN (0,1,2,3)),
    deleted INTEGER NOT NULL CHECK(typeof(deleted)='integer' AND deleted IN (0,1)),
    stored_bytes INTEGER NOT NULL CHECK(typeof(stored_bytes)='integer' AND stored_bytes BETWEEN 0 AND 33554432),
    PRIMARY KEY(account,channel,message,attachment_id)
);
";

/// The bounded expression used by both usage reporting and eviction ordering.
const HISTORY_MESSAGE_BYTES_SQL: &str = "(length(CAST(m.content AS BLOB)) + 96
    + COALESCE((SELECT SUM(8 + length(CAST(c.before AS BLOB)) + COALESCE(length(CAST(c.after AS BLOB)),0))
        FROM history_changes c
        WHERE c.account=m.account AND c.channel=m.channel AND c.message=m.id),0)
    + COALESCE((SELECT SUM(64 + length(CAST(a.filename AS BLOB)) + COALESCE(length(CAST(a.content_type AS BLOB)),0))
        FROM history_attachments a
        WHERE a.account=m.account AND a.channel=m.channel AND a.message=m.id),0))";

/// Creates the persistent history tables. Safe to call more than once.
pub(crate) fn create_tables(connection: &Connection) -> Result<()> {
	connection.execute_batch(HISTORY_SCHEMA_SQL)?;
	Ok(())
}

/// Schema-31 migration hook. Table creation is idempotent, so the version argument is advisory.
pub(crate) fn migrate(connection: &Connection, _old_version: u32) -> Result<()> {
	create_tables(connection)
}

/// Removes every persistent history row for one account from an existing purge transaction.
///
/// The parent account-purge path already owns a transaction; call this before its commit.
pub(crate) fn forget_account_in(connection: &Connection, account: Id) -> Result<()> {
	let account = account_key(account)?;
	for table in [
		"history_attachments",
		"history_tombstones",
		"history_changes",
		"history_messages",
		"history_filters",
		"history_settings",
	] {
		connection.execute(
			&format!("DELETE FROM {table} WHERE account=?1"),
			params![account.as_str()],
		)?;
	}
	Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryAlertMode {
	#[default]
	Off,
	Mentions,
	All,
}

impl HistoryAlertMode {
	fn as_i64(self) -> i64 {
		match self {
			Self::Off => 0,
			Self::Mentions => 1,
			Self::All => 2,
		}
	}

	fn from_i64(value: i64) -> Option<Self> {
		match value {
			0 => Some(Self::Off),
			1 => Some(Self::Mentions),
			2 => Some(Self::All),
			_ => None,
		}
	}
}

/// Per-type download opt-ins. Every type defaults to off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryDownloadSettings {
	pub images: bool,
	pub audio: bool,
	pub video: bool,
	pub files: bool,
}

impl HistoryDownloadSettings {
	fn allows(self, kind: HistoryAttachmentKind) -> bool {
		match kind {
			HistoryAttachmentKind::Image => self.images,
			HistoryAttachmentKind::Audio => self.audio,
			HistoryAttachmentKind::Video => self.video,
			HistoryAttachmentKind::File => self.files,
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistorySettings {
	pub enabled: bool,
	pub retention_days: u32,
	pub max_messages: u32,
	pub max_bytes: u64,
	pub alert_mode: HistoryAlertMode,
	pub ignore_users: Vec<Id>,
	pub ignore_channels: Vec<Id>,
	pub downloads: HistoryDownloadSettings,
}

impl Default for HistorySettings {
	fn default() -> Self {
		Self {
			enabled: false,
			retention_days: 30,
			max_messages: 10_000,
			max_bytes: HISTORY_SETTINGS_BYTES_DEFAULT,
			alert_mode: HistoryAlertMode::Off,
			ignore_users: Vec::new(),
			ignore_channels: Vec::new(),
			downloads: HistoryDownloadSettings::default(),
		}
	}
}

impl HistorySettings {
	pub fn is_valid(&self) -> bool {
		self.retention_days >= HISTORY_SETTINGS_RETENTION_DAYS_MIN
			&& self.retention_days <= HISTORY_SETTINGS_RETENTION_DAYS_MAX
			&& self.max_messages >= HISTORY_SETTINGS_MESSAGES_MIN
			&& self.max_messages <= HISTORY_SETTINGS_MESSAGES_MAX
			&& self.max_bytes >= HISTORY_SETTINGS_BYTES_MIN
			&& self.max_bytes <= HISTORY_SETTINGS_BYTES_MAX
			&& valid_filter(&self.ignore_users)
			&& valid_filter(&self.ignore_channels)
	}
}

fn valid_filter(ids: &[Id]) -> bool {
	ids.len() <= HISTORY_FILTER_MAX
		&& ids.iter().all(|id| id.0 != 0)
		&& ids.iter().map(|id| id.0).collect::<BTreeSet<_>>().len() == ids.len()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum HistoryAttachmentKind {
	Image,
	Audio,
	Video,
	File,
}

impl HistoryAttachmentKind {
	fn as_i64(self) -> i64 {
		match self {
			Self::Image => 0,
			Self::Audio => 1,
			Self::Video => 2,
			Self::File => 3,
		}
	}

	fn from_i64(value: i64) -> Option<Self> {
		match value {
			0 => Some(Self::Image),
			1 => Some(Self::Audio),
			2 => Some(Self::Video),
			3 => Some(Self::File),
			_ => None,
		}
	}

	fn download_cap(self) -> u64 {
		match self {
			Self::Image => HISTORY_ATTACHMENT_IMAGE_BYTES,
			Self::Audio => HISTORY_ATTACHMENT_AUDIO_BYTES,
			Self::Video => HISTORY_ATTACHMENT_VIDEO_BYTES,
			Self::File => HISTORY_ATTACHMENT_FILE_BYTES,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum HistoryAttachmentState {
	NotRequested,
	Reserved,
	Stored,
	Failed,
}

impl HistoryAttachmentState {
	fn as_i64(self) -> i64 {
		match self {
			Self::NotRequested => STATE_NOT_REQUESTED,
			Self::Reserved => STATE_RESERVED,
			Self::Stored => STATE_STORED,
			Self::Failed => STATE_FAILED,
		}
	}

	fn from_i64(value: i64) -> Option<Self> {
		match value {
			STATE_NOT_REQUESTED => Some(Self::NotRequested),
			STATE_RESERVED => Some(Self::Reserved),
			STATE_STORED => Some(Self::Stored),
			STATE_FAILED => Some(Self::Failed),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum HistoryChangeKind {
	Edit,
	Delete,
}

impl HistoryChangeKind {
	fn as_i64(self) -> i64 {
		match self {
			Self::Edit => 0,
			Self::Delete => 1,
		}
	}

	fn from_i64(value: i64) -> Option<Self> {
		match value {
			0 => Some(Self::Edit),
			1 => Some(Self::Delete),
			_ => None,
		}
	}
}

/// Result of a single observation write.
///
/// `Unknown` is distinct from `NoOp` so a delete handler can alert only for a known message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryWrite {
	Applied,
	NoOp,
	Unknown,
}

/// One loaded body. Attachment inputs contain metadata only, never a URL or local path.
#[derive(Clone, Copy, Debug)]
pub struct HistoryBaseline<'a> {
	pub channel: Id,
	pub message: Id,
	pub author: Id,
	pub content: &'a str,
	pub revision: u64,
	pub observed_at: i64,
	pub attachments: &'a [HistoryAttachmentInput<'a>],
}

#[derive(Clone, Copy, Debug)]
pub struct HistoryAttachmentInput<'a> {
	pub id: Id,
	pub filename: &'a str,
	pub content_type: Option<&'a str>,
	pub size: u64,
	pub kind: HistoryAttachmentKind,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HistoryChange {
	pub kind: HistoryChangeKind,
	pub revision: u64,
	pub before: Option<String>,
	pub after: Option<String>,
	pub observed_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HistoryAttachment {
	pub id: Id,
	pub filename: String,
	pub content_type: Option<String>,
	pub size: u64,
	pub kind: HistoryAttachmentKind,
	pub state: HistoryAttachmentState,
	pub deleted: bool,
	pub stored_bytes: u64,
}

/// One viewer row. Serialization is for explicit export only.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HistoryRecord {
	pub channel: Id,
	pub message: Id,
	pub author: Id,
	pub content: String,
	pub deleted: bool,
	pub revision: u64,
	pub observed_at: i64,
	pub changes: Vec<HistoryChange>,
	pub attachments: Vec<HistoryAttachment>,
}

/// Stable newest-first page cursor. Opaque to callers apart from passing it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HistoryCursor {
	pub observed_at: i64,
	pub channel: Id,
	pub message: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
	pub messages: Vec<HistoryRecord>,
	pub next: Option<HistoryCursor>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HistoryViewFilter {
	#[default]
	All,
	Edited,
	Deleted,
}

/// Native-viewer query. Cursors are identities; the store resolves their timestamps securely.
pub struct HistoryViewQuery {
	pub channel: Option<Id>,
	pub author: Option<Id>,
	pub message: Option<Id>,
	pub filter: HistoryViewFilter,
	pub before: Option<(Id, Id)>,
	pub after: Option<(Id, Id)>,
	pub limit: u32,
}

pub struct HistoryViewPage {
	pub messages: Vec<HistoryRecord>,
	pub has_older: bool,
	pub has_newer: bool,
}

/// Explicit export only. The desktop may stream one bounded page at a time as JSON or text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HistoryExportPage {
	pub messages: Vec<HistoryRecord>,
	pub next: Option<HistoryCursor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryUsage {
	pub messages: u64,
	pub bytes: u64,
	pub attachments: u64,
	/// Reserved plus stored download bytes, not the metadata size of every attachment.
	pub attachment_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryMaintenance {
	pub expired: usize,
	pub evicted: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryReservation {
	Reserved { bytes: u64 },
	AlreadyReserved,
	AlreadyStored,
	Disabled,
	TooLarge,
	AccountFull,
	Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryAttachmentUpdate {
	Stored,
	Released,
	TooLarge,
	AccountFull,
	NotReserved,
	Unknown,
}

impl LocalStore {
	pub fn history_settings(&self, account: Id) -> Result<HistorySettings> {
		let account = account_key(account)?;
		read_history_settings(&self.0, &account)
	}

	/// Distinguish first-time activation (no row) from an explicit saved opt-out.
	pub fn history_settings_configured(&self, account: Id) -> Result<bool> {
		Ok(self.0.query_row(
			"SELECT EXISTS(SELECT 1 FROM history_settings WHERE account=?1)",
			params![account_key(account)?],
			|row| row.get(0),
		)?)
	}

	pub fn save_history_settings(&mut self, account: Id, value: &HistorySettings) -> Result<()> {
		if !value.is_valid() {
			return Err(StoreError::Capacity);
		}
		let account = account_key(account)?;
		let transaction = self.0.transaction()?;
		transaction.execute(
			"INSERT INTO history_settings(account,enabled,retention_days,max_messages,max_bytes,alert_mode,download_images,download_audio,download_video,download_files)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(account) DO UPDATE SET
               enabled=excluded.enabled,retention_days=excluded.retention_days,
               max_messages=excluded.max_messages,max_bytes=excluded.max_bytes,
               alert_mode=excluded.alert_mode,download_images=excluded.download_images,
               download_audio=excluded.download_audio,download_video=excluded.download_video,
               download_files=excluded.download_files",
			params![
				account.as_str(),
				flag(value.enabled),
				i64::from(value.retention_days),
				i64::from(value.max_messages),
				i64::try_from(value.max_bytes).map_err(|_| StoreError::Capacity)?,
				value.alert_mode.as_i64(),
				flag(value.downloads.images),
				flag(value.downloads.audio),
				flag(value.downloads.video),
				flag(value.downloads.files),
			],
		)?;
		transaction.execute(
			"DELETE FROM history_filters WHERE account=?1",
			params![account.as_str()],
		)?;
		for id in &value.ignore_users {
			transaction.execute(
				"INSERT INTO history_filters(account,kind,filter_id) VALUES(?1,0,?2)",
				params![account.as_str(), id.to_string()],
			)?;
		}
		for id in &value.ignore_channels {
			transaction.execute(
				"INSERT INTO history_filters(account,kind,filter_id) VALUES(?1,1,?2)",
				params![account.as_str(), id.to_string()],
			)?;
		}
		enforce_history_limits_in(&transaction, &account, value)?;
		transaction.commit()?;
		Ok(())
	}

	/// Inserts a trusted baseline only when the message row does not exist yet.
	pub fn record_history_baseline(
		&mut self,
		account: Id,
		baseline: &HistoryBaseline<'_>,
	) -> Result<HistoryWrite> {
		let account_id = account;
		let account = account_key(account)?;
		let channel = id_key(baseline.channel)?;
		let message = id_key(baseline.message)?;
		let author = id_key(baseline.author)?;
		validate_content(baseline.content)?;
		let revision = revision_key(baseline.revision)?;
		let observed_at = timestamp(baseline.observed_at)?;
		if baseline.attachments.len() > HISTORY_MAX_ATTACHMENTS {
			return Err(StoreError::Capacity);
		}
		let mut seen = BTreeSet::new();
		for attachment in baseline.attachments {
			if attachment.id.0 == 0
				|| !seen.insert(attachment.id)
				|| attachment.filename.is_empty()
				|| attachment.filename.len() > 256
				|| attachment.filename.contains('/')
				|| attachment.filename.contains('\\')
				|| attachment.filename.contains('\0')
				|| attachment
					.content_type
					.is_some_and(|kind| kind.len() > 127 || kind.contains('\0'))
				|| attachment.size > i64::MAX as u64
			{
				return Err(StoreError::Capacity);
			}
		}
		let settings = self.history_settings(account_id)?;
		let transaction = self.0.transaction()?;
		let tombstoned: bool = transaction.query_row(
			"SELECT EXISTS(SELECT 1 FROM history_tombstones WHERE account=?1 AND channel=?2 AND message=?3)",
			params![account.as_str(), channel.as_str(), message.as_str()],
			|row| row.get(0),
		)?;
		if tombstoned {
			transaction.commit()?;
			return Ok(HistoryWrite::NoOp);
		}
		let inserted = transaction.execute(
			"INSERT INTO history_messages(account,channel,id,author,content,revision,deleted,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,0,?7)
             ON CONFLICT(account,channel,id) DO NOTHING",
			params![
				account.as_str(),
				channel.as_str(),
				message.as_str(),
				author.as_str(),
				baseline.content,
				revision,
				observed_at,
			],
		)?;
		if inserted == 0 {
			transaction.commit()?;
			return Ok(HistoryWrite::NoOp);
		}
		for attachment in baseline.attachments {
			transaction.execute(
				"INSERT INTO history_attachments(account,channel,message,attachment_id,filename,content_type,size,kind,download_state,deleted,stored_bytes)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,0,0)",
				params![
					account.as_str(),
					channel.as_str(),
					message.as_str(),
					attachment.id.to_string(),
					attachment.filename,
					attachment.content_type,
					i64::try_from(attachment.size).map_err(|_| StoreError::Capacity)?,
					attachment.kind.as_i64(),
					STATE_NOT_REQUESTED,
				],
			)?;
		}
		enforce_history_limits_in(&transaction, &account, &settings)?;
		transaction.commit()?;
		Ok(HistoryWrite::Applied)
	}

	/// Records an edit of an existing live row. Unknown, deleted, stale and unchanged edits are no-ops.
	pub fn record_history_edit(
		&mut self,
		account: Id,
		channel: Id,
		message: Id,
		revision: u64,
		after: &str,
		observed_at: i64,
	) -> Result<HistoryWrite> {
		let account_id = account;
		let account = account_key(account)?;
		let channel = id_key(channel)?;
		let message = id_key(message)?;
		validate_content(after)?;
		let revision = revision_key(revision)?;
		let observed_at = timestamp(observed_at)?;
		let settings = self.history_settings(account_id)?;
		let transaction = self.0.transaction()?;
		let existing: Option<(Option<String>, i64, i64, i64)> = transaction
			.query_row(
				"SELECT CASE WHEN typeof(content)='text' AND length(CAST(content AS BLOB))<=16384 THEN content ELSE NULL END,
                        deleted,revision,updated_at
                 FROM history_messages WHERE account=?1 AND channel=?2 AND id=?3",
				params![account.as_str(), channel.as_str(), message.as_str()],
				|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
			)
			.optional()?;
		let Some((content, deleted, existing_revision, existing_updated)) = existing else {
			return Ok(HistoryWrite::Unknown);
		};
		let Some(content) = content else {
			return Err(StoreError::Incompatible);
		};
		if !matches!(deleted, 0 | 1) || existing_revision < 0 || existing_updated < 0 {
			return Err(StoreError::Incompatible);
		}
		if deleted != 0 || existing_revision > revision || after == content {
			return Ok(HistoryWrite::NoOp);
		}
		let observed_at = observed_at.max(existing_updated);
		insert_history_change(
			&transaction,
			&account,
			&channel,
			&message,
			HistoryChangeKind::Edit,
			revision,
			&content,
			Some(after),
			observed_at,
		)?;
		transaction.execute(
			"UPDATE history_messages SET content=?4,revision=?5,updated_at=?6
             WHERE account=?1 AND channel=?2 AND id=?3 AND deleted=0",
			params![
				account.as_str(),
				channel.as_str(),
				message.as_str(),
				after,
				revision,
				observed_at,
			],
		)?;
		enforce_history_limits_in(&transaction, &account, &settings)?;
		transaction.commit()?;
		Ok(HistoryWrite::Applied)
	}

	/// Marks one existing message deleted. The tombstone keeps the last body and is never recreated
	/// by a later baseline while the row exists.
	pub fn record_history_delete(
		&mut self,
		account: Id,
		channel: Id,
		message: Id,
		observed_at: i64,
	) -> Result<HistoryWrite> {
		let account_id = account;
		let account = account_key(account)?;
		let channel = id_key(channel)?;
		let message = id_key(message)?;
		let observed_at = timestamp(observed_at)?;
		let settings = self.history_settings(account_id)?;
		let transaction = self.0.transaction()?;
		let result = mark_deleted_in(
			&transaction,
			&account,
			&channel,
			&message,
			observed_at,
			settings.max_messages,
		)?;
		if result == HistoryWrite::Applied {
			enforce_history_limits_in(&transaction, &account, &settings)?;
		}
		transaction.commit()?;
		Ok(result)
	}

	/// Marks existing messages deleted in one transaction. Returns the number newly tombstoned.
	pub fn record_history_bulk_delete(
		&mut self,
		account: Id,
		channel: Id,
		messages: &[Id],
		observed_at: i64,
	) -> Result<usize> {
		if messages.len() > HISTORY_BULK_MAX {
			return Err(StoreError::Capacity);
		}
		let account_id = account;
		let account = account_key(account)?;
		let channel = id_key(channel)?;
		let observed_at = timestamp(observed_at)?;
		if messages.is_empty() {
			return Ok(0);
		}
		let settings = self.history_settings(account_id)?;
		let transaction = self.0.transaction()?;
		let mut applied = 0;
		for message in messages {
			let message = id_key(*message)?;
			if mark_deleted_in(
				&transaction,
				&account,
				&channel,
				&message,
				observed_at,
				settings.max_messages,
			)? == HistoryWrite::Applied
			{
				applied += 1;
			}
		}
		if applied != 0 {
			enforce_history_limits_in(&transaction, &account, &settings)?;
		}
		transaction.commit()?;
		Ok(applied)
	}

	/// Removes rows older than the account's retention window. Tombstones are included.
	pub fn expire_history(&mut self, account: Id, now_ms: i64) -> Result<usize> {
		let account = account_key(account)?;
		let settings = read_history_settings(&self.0, &account)?;
		let now = timestamp(now_ms)?;
		let cutoff =
			now.saturating_sub(i64::from(settings.retention_days).saturating_mul(HISTORY_DAY_MS));
		let transaction = self.0.transaction()?;
		let mut keys = Vec::new();
		{
			let mut query = transaction.prepare(
				"SELECT channel,id FROM history_messages
                 WHERE account=?1 AND updated_at < ?2
                 ORDER BY updated_at ASC,length(id) ASC,id ASC
                 LIMIT 50000",
			)?;
			let mut rows = query.query(params![account.as_str(), cutoff])?;
			while let Some(row) = rows.next()? {
				keys.push((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
			}
		}
		for (channel, message) in &keys {
			delete_history_message(&transaction, &account, channel, message)?;
		}
		transaction.execute(
			"DELETE FROM history_tombstones WHERE account=?1 AND observed_at < ?2",
			params![account.as_str(), cutoff],
		)?;
		transaction.commit()?;
		Ok(keys.len())
	}

	/// Evicts oldest messages until the account is within both count and metadata-byte limits.
	pub fn enforce_history_limits(&mut self, account: Id) -> Result<usize> {
		let account = account_key(account)?;
		let settings = read_history_settings(&self.0, &account)?;
		let transaction = self.0.transaction()?;
		let evicted = enforce_history_limits_in(&transaction, &account, &settings)?;
		transaction.commit()?;
		Ok(evicted)
	}

	/// Applies retention and then count/byte eviction.
	pub fn maintain_history(&mut self, account: Id, now_ms: i64) -> Result<HistoryMaintenance> {
		let expired = self.expire_history(account, now_ms)?;
		let evicted = self.enforce_history_limits(account)?;
		Ok(HistoryMaintenance { expired, evicted })
	}

	pub fn history_usage(&self, account: Id) -> Result<HistoryUsage> {
		let account = account_key(account)?;
		let sql = format!(
			"SELECT COUNT(*),COALESCE(SUM({HISTORY_MESSAGE_BYTES_SQL}),0),
             (SELECT COUNT(*) FROM history_attachments WHERE account=?1),
             (SELECT COALESCE(SUM(stored_bytes),0) FROM history_attachments
              WHERE account=?1 AND download_state IN (?2,?3))
             FROM history_messages m WHERE m.account=?1"
		);
		let values: (i64, i64, i64, i64) = self.0.query_row(
			&sql,
			params![
				account.as_str(),
				HistoryAttachmentState::Reserved.as_i64(),
				HistoryAttachmentState::Stored.as_i64(),
			],
			|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
		)?;
		Ok(HistoryUsage {
			messages: u64::try_from(values.0).map_err(|_| StoreError::Incompatible)?,
			bytes: u64::try_from(values.1).map_err(|_| StoreError::Incompatible)?,
			attachments: u64::try_from(values.2).map_err(|_| StoreError::Incompatible)?,
			attachment_bytes: u64::try_from(values.3).map_err(|_| StoreError::Incompatible)?,
		})
	}

	pub fn clear_history_channel(&mut self, account: Id, channel: Id) -> Result<usize> {
		let account = account_key(account)?;
		let channel = id_key(channel)?;
		let transaction = self.0.transaction()?;
		let count: i64 = transaction.query_row(
			"SELECT COUNT(*) FROM history_messages WHERE account=?1 AND channel=?2",
			params![account.as_str(), channel.as_str()],
			|row| row.get(0),
		)?;
		transaction.execute(
			"DELETE FROM history_attachments WHERE account=?1 AND channel=?2",
			params![account.as_str(), channel.as_str()],
		)?;
		transaction.execute(
			"DELETE FROM history_changes WHERE account=?1 AND channel=?2",
			params![account.as_str(), channel.as_str()],
		)?;
		transaction.execute(
			"DELETE FROM history_messages WHERE account=?1 AND channel=?2",
			params![account.as_str(), channel.as_str()],
		)?;
		transaction.execute(
			"DELETE FROM history_tombstones WHERE account=?1 AND channel=?2",
			params![account.as_str(), channel.as_str()],
		)?;
		transaction.commit()?;
		usize::try_from(count).map_err(|_| StoreError::Incompatible)
	}

	pub fn clear_history_account(&mut self, account: Id) -> Result<usize> {
		let account = account_key(account)?;
		let settings = read_history_settings(&self.0, &account)?;
		let transaction = self.0.transaction()?;
		let count: i64 = transaction.query_row(
			"SELECT COUNT(*) FROM history_messages WHERE account=?1",
			params![account.as_str()],
			|row| row.get(0),
		)?;
		for table in [
			"history_attachments",
			"history_tombstones",
			"history_changes",
			"history_messages",
		] {
			transaction.execute(
				&format!("DELETE FROM {table} WHERE account=?1"),
				params![account.as_str()],
			)?;
		}
		if transaction.execute(
			"UPDATE history_settings SET enabled=0 WHERE account=?1",
			params![account.as_str()],
		)? == 0
		{
			transaction.execute(
				"INSERT INTO history_settings(account,enabled,retention_days,max_messages,max_bytes,alert_mode,download_images,download_audio,download_video,download_files)
				 VALUES(?1,0,?2,?3,?4,?5,?6,?7,?8,?9)",
				params![
					account.as_str(),
					settings.retention_days,
					settings.max_messages,
					i64::try_from(settings.max_bytes).map_err(|_| StoreError::Capacity)?,
					settings.alert_mode.as_i64(),
					flag(settings.downloads.images),
					flag(settings.downloads.audio),
					flag(settings.downloads.video),
					flag(settings.downloads.files),
				],
			)?;
		}
		transaction.commit()?;
		usize::try_from(count).map_err(|_| StoreError::Incompatible)
	}

	/// Purge retained bodies and versions for an author newly added to the ignore list.
	pub fn clear_history_author(&mut self, account: Id, author: Id) -> Result<usize> {
		let account = account_key(account)?;
		let author = id_key(author)?;
		let transaction = self.0.transaction()?;
		let mut keys = Vec::new();
		{
			let mut query = transaction.prepare(
				"SELECT channel,id FROM history_messages WHERE account=?1 AND author=?2 LIMIT 50000",
			)?;
			let mut rows = query.query(params![account.as_str(), author.as_str()])?;
			while let Some(row) = rows.next()? {
				keys.push((row.get::<_, String>(0)?, row.get::<_, String>(1)?));
			}
		}
		for (channel, message) in &keys {
			delete_history_message(&transaction, &account, channel, message)?;
		}
		transaction.commit()?;
		Ok(keys.len())
	}

	/// Explicit purge alias used by account-switch/logout paths.
	pub fn forget_history_channel(&mut self, account: Id, channel: Id) -> Result<usize> {
		self.clear_history_channel(account, channel)
	}

	/// Explicit purge alias used by account-switch/logout paths.
	pub fn forget_history_account(&mut self, account: Id) -> Result<usize> {
		self.clear_history_account(account)
	}

	pub fn history_page(
		&self,
		account: Id,
		channel: Option<Id>,
		cursor: Option<HistoryCursor>,
		limit: u32,
	) -> Result<HistoryPage> {
		if limit == 0 || limit > HISTORY_PAGE_HARD_MAX {
			return Err(StoreError::Capacity);
		}
		let limit = limit.min(HISTORY_PAGE_MAX);
		let (messages, next) = history_page_inner(&self.0, account, channel, cursor, limit)?;
		Ok(HistoryPage { messages, next })
	}

	/// One bounded, filtered native-viewer page. This reads only the requested account's rows.
	pub fn history_view_page(
		&self,
		account: Id,
		query: &HistoryViewQuery,
	) -> Result<HistoryViewPage> {
		history_view_inner(&self.0, account, query)
	}

	/// Bounded export page. The caller serializes it; this module performs no file IO.
	pub fn history_export_page(
		&self,
		account: Id,
		channel: Option<Id>,
		cursor: Option<HistoryCursor>,
		limit: u32,
	) -> Result<HistoryExportPage> {
		if limit == 0 || limit > HISTORY_PAGE_HARD_MAX {
			return Err(StoreError::Capacity);
		}
		let (messages, next) = history_page_inner(&self.0, account, channel, cursor, limit)?;
		Ok(HistoryExportPage { messages, next })
	}

	/// Reserves download bytes for one existing attachment after checking its type opt-in.
	pub fn reserve_history_attachment(
		&mut self,
		account: Id,
		channel: Id,
		message: Id,
		attachment: Id,
		expected_bytes: u64,
	) -> Result<HistoryReservation> {
		let account = account_key(account)?;
		let channel = id_key(channel)?;
		let message = id_key(message)?;
		let attachment = id_key(attachment)?;
		let settings = read_history_settings(&self.0, &account)?;
		let expected = i64::try_from(expected_bytes).map_err(|_| StoreError::Capacity)?;
		let transaction = self.0.transaction()?;
		let existing: Option<(i64, i64, i64)> = transaction
			.query_row(
				"SELECT kind,download_state,stored_bytes FROM history_attachments
                 WHERE account=?1 AND channel=?2 AND message=?3 AND attachment_id=?4",
				params![
					account.as_str(),
					channel.as_str(),
					message.as_str(),
					attachment.as_str()
				],
				|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
			)
			.optional()?;
		let Some((kind, state, _)) = existing else {
			return Ok(HistoryReservation::Unknown);
		};
		let kind = HistoryAttachmentKind::from_i64(kind).ok_or(StoreError::Incompatible)?;
		let state = HistoryAttachmentState::from_i64(state).ok_or(StoreError::Incompatible)?;
		match state {
			HistoryAttachmentState::Reserved => return Ok(HistoryReservation::AlreadyReserved),
			HistoryAttachmentState::Stored => return Ok(HistoryReservation::AlreadyStored),
			HistoryAttachmentState::NotRequested | HistoryAttachmentState::Failed => {}
		}
		if !settings.enabled || !settings.downloads.allows(kind) {
			return Ok(HistoryReservation::Disabled);
		}
		if expected_bytes > kind.download_cap() {
			return Ok(HistoryReservation::TooLarge);
		}
		let used: i64 = transaction.query_row(
			"SELECT COALESCE(SUM(stored_bytes),0) FROM history_attachments
             WHERE account=?1 AND download_state IN (?2,?3)",
			params![
				account.as_str(),
				HistoryAttachmentState::Reserved.as_i64(),
				HistoryAttachmentState::Stored.as_i64(),
			],
			|row| row.get(0),
		)?;
		let used = u64::try_from(used).map_err(|_| StoreError::Incompatible)?;
		if used.saturating_add(expected_bytes) > HISTORY_ATTACHMENT_ACCOUNT_BYTES {
			return Ok(HistoryReservation::AccountFull);
		}
		let updated = transaction.execute(
			"UPDATE history_attachments SET download_state=?5,stored_bytes=?6
             WHERE account=?1 AND channel=?2 AND message=?3 AND attachment_id=?4
               AND download_state IN (?7,?8)",
			params![
				account.as_str(),
				channel.as_str(),
				message.as_str(),
				attachment.as_str(),
				HistoryAttachmentState::Reserved.as_i64(),
				expected,
				HistoryAttachmentState::NotRequested.as_i64(),
				HistoryAttachmentState::Failed.as_i64(),
			],
		)?;
		if updated == 0 {
			return Ok(HistoryReservation::AlreadyReserved);
		}
		transaction.commit()?;
		Ok(HistoryReservation::Reserved {
			bytes: expected_bytes,
		})
	}

	/// Completes a reservation with the actual stored byte count.
	pub fn complete_history_attachment(
		&mut self,
		account: Id,
		channel: Id,
		message: Id,
		attachment: Id,
		actual_bytes: u64,
	) -> Result<HistoryAttachmentUpdate> {
		let account = account_key(account)?;
		let channel = id_key(channel)?;
		let message = id_key(message)?;
		let attachment = id_key(attachment)?;
		let transaction = self.0.transaction()?;
		let existing: Option<(i64, i64, i64)> = transaction
			.query_row(
				"SELECT kind,download_state,stored_bytes FROM history_attachments
                 WHERE account=?1 AND channel=?2 AND message=?3 AND attachment_id=?4",
				params![
					account.as_str(),
					channel.as_str(),
					message.as_str(),
					attachment.as_str()
				],
				|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
			)
			.optional()?;
		let Some((kind, state, reserved)) = existing else {
			return Ok(HistoryAttachmentUpdate::Unknown);
		};
		let kind = HistoryAttachmentKind::from_i64(kind).ok_or(StoreError::Incompatible)?;
		let state = HistoryAttachmentState::from_i64(state).ok_or(StoreError::Incompatible)?;
		if state != HistoryAttachmentState::Reserved {
			return Ok(HistoryAttachmentUpdate::NotReserved);
		}
		if actual_bytes > kind.download_cap() {
			return Ok(HistoryAttachmentUpdate::TooLarge);
		}
		let reserved = u64::try_from(reserved).map_err(|_| StoreError::Incompatible)?;
		let used: i64 = transaction.query_row(
			"SELECT COALESCE(SUM(stored_bytes),0) FROM history_attachments
             WHERE account=?1 AND download_state IN (?2,?3)",
			params![
				account.as_str(),
				HistoryAttachmentState::Reserved.as_i64(),
				HistoryAttachmentState::Stored.as_i64(),
			],
			|row| row.get(0),
		)?;
		let used = u64::try_from(used).map_err(|_| StoreError::Incompatible)?;
		let others = used.saturating_sub(reserved);
		if others.saturating_add(actual_bytes) > HISTORY_ATTACHMENT_ACCOUNT_BYTES {
			return Ok(HistoryAttachmentUpdate::AccountFull);
		}
		let actual = i64::try_from(actual_bytes).map_err(|_| StoreError::Capacity)?;
		transaction.execute(
			"UPDATE history_attachments SET download_state=?5,stored_bytes=?6
             WHERE account=?1 AND channel=?2 AND message=?3 AND attachment_id=?4
               AND download_state=?7",
			params![
				account.as_str(),
				channel.as_str(),
				message.as_str(),
				attachment.as_str(),
				HistoryAttachmentState::Stored.as_i64(),
				actual,
				HistoryAttachmentState::Reserved.as_i64(),
			],
		)?;
		transaction.commit()?;
		Ok(HistoryAttachmentUpdate::Stored)
	}

	/// Releases a reservation or stored download. `failed` records a failed attempt instead of
	/// returning the attachment to the not-requested state.
	pub fn release_history_attachment(
		&mut self,
		account: Id,
		channel: Id,
		message: Id,
		attachment: Id,
		failed: bool,
	) -> Result<bool> {
		let account = account_key(account)?;
		let channel = id_key(channel)?;
		let message = id_key(message)?;
		let attachment = id_key(attachment)?;
		let transaction = self.0.transaction()?;
		let state: Option<i64> = transaction
			.query_row(
				"SELECT download_state FROM history_attachments
                 WHERE account=?1 AND channel=?2 AND message=?3 AND attachment_id=?4",
				params![
					account.as_str(),
					channel.as_str(),
					message.as_str(),
					attachment.as_str()
				],
				|row| row.get(0),
			)
			.optional()?;
		let Some(state) = state else {
			return Ok(false);
		};
		let state = HistoryAttachmentState::from_i64(state).ok_or(StoreError::Incompatible)?;
		if matches!(
			state,
			HistoryAttachmentState::NotRequested | HistoryAttachmentState::Failed
		) {
			return Ok(false);
		}
		let next = if failed {
			HistoryAttachmentState::Failed
		} else {
			HistoryAttachmentState::NotRequested
		};
		transaction.execute(
			"UPDATE history_attachments SET download_state=?5,stored_bytes=0
             WHERE account=?1 AND channel=?2 AND message=?3 AND attachment_id=?4",
			params![
				account.as_str(),
				channel.as_str(),
				message.as_str(),
				attachment.as_str(),
				next.as_i64(),
			],
		)?;
		transaction.commit()?;
		Ok(true)
	}
}

fn account_key(account: Id) -> Result<String> {
	if account.0 == 0 {
		return Err(StoreError::Capacity);
	}
	Ok(account.to_string())
}

fn flag(value: bool) -> i64 {
	if value { 1 } else { 0 }
}

fn id_key(id: Id) -> Result<String> {
	if id.0 == 0 {
		return Err(StoreError::Capacity);
	}
	Ok(id.to_string())
}

fn timestamp(value: i64) -> Result<i64> {
	if value < 0 {
		return Err(StoreError::Capacity);
	}
	Ok(value)
}

fn revision_key(revision: u64) -> Result<i64> {
	i64::try_from(revision).map_err(|_| StoreError::Capacity)
}

fn validate_content(content: &str) -> Result<()> {
	if content.len() > HISTORY_MAX_CONTENT_BYTES {
		return Err(StoreError::Capacity);
	}
	Ok(())
}

fn parse_flag(value: i64) -> Result<bool> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(StoreError::Incompatible),
	}
}

type StoredSettingsRow = (i64, i64, i64, i64, i64, i64, i64, i64, i64);

fn read_history_settings(connection: &Connection, account: &str) -> Result<HistorySettings> {
	let values: Option<StoredSettingsRow> = connection
		.query_row(
			"SELECT enabled,retention_days,max_messages,max_bytes,alert_mode,
                    download_images,download_audio,download_video,download_files
             FROM history_settings WHERE account=?1",
			params![account],
			|row| {
				Ok((
					row.get(0)?,
					row.get(1)?,
					row.get(2)?,
					row.get(3)?,
					row.get(4)?,
					row.get(5)?,
					row.get(6)?,
					row.get(7)?,
					row.get(8)?,
				))
			},
		)
		.optional()?;
	let Some((enabled, retention, max_messages, max_bytes, alert, images, audio, video, files)) =
		values
	else {
		return Ok(HistorySettings::default());
	};
	let mut settings = HistorySettings {
		enabled: parse_flag(enabled)?,
		retention_days: u32::try_from(retention).map_err(|_| StoreError::Incompatible)?,
		max_messages: u32::try_from(max_messages).map_err(|_| StoreError::Incompatible)?,
		max_bytes: u64::try_from(max_bytes).map_err(|_| StoreError::Incompatible)?,
		alert_mode: HistoryAlertMode::from_i64(alert).ok_or(StoreError::Incompatible)?,
		ignore_users: Vec::new(),
		ignore_channels: Vec::new(),
		downloads: HistoryDownloadSettings {
			images: parse_flag(images)?,
			audio: parse_flag(audio)?,
			video: parse_flag(video)?,
			files: parse_flag(files)?,
		},
	};
	settings.ignore_users = read_history_filters(connection, account, 0)?;
	settings.ignore_channels = read_history_filters(connection, account, 1)?;
	if !settings.is_valid() {
		return Err(StoreError::Incompatible);
	}
	Ok(settings)
}

fn read_history_filters(connection: &Connection, account: &str, kind: i64) -> Result<Vec<Id>> {
	let mut query = connection.prepare(
		"SELECT filter_id FROM history_filters WHERE account=?1 AND kind=?2
         ORDER BY length(filter_id),filter_id",
	)?;
	let mut rows = query.query(params![account, kind])?;
	let mut values = Vec::new();
	let mut seen = BTreeSet::new();
	while let Some(row) = rows.next()? {
		if values.len() >= HISTORY_FILTER_MAX {
			return Err(StoreError::Incompatible);
		}
		let raw: String = row.get(0)?;
		let id = raw.parse::<Id>().map_err(|_| StoreError::Incompatible)?;
		if !seen.insert(id) {
			return Err(StoreError::Incompatible);
		}
		values.push(id);
	}
	values.sort_unstable();
	Ok(values)
}

#[allow(clippy::too_many_arguments)]
fn insert_history_change(
	connection: &Connection,
	account: &str,
	channel: &str,
	message: &str,
	kind: HistoryChangeKind,
	revision: i64,
	before: &str,
	after: Option<&str>,
	observed_at: i64,
) -> Result<()> {
	let seq: i64 = connection.query_row(
		"SELECT COALESCE(MAX(seq),0)+1 FROM history_changes
         WHERE account=?1 AND channel=?2 AND message=?3",
		params![account, channel, message],
		|row| row.get(0),
	)?;
	connection.execute(
		"INSERT INTO history_changes(account,channel,message,seq,kind,revision,before,after,observed_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
		params![
			account,
			channel,
			message,
			seq,
			kind.as_i64(),
			revision,
			before,
			after,
			observed_at,
		],
	)?;
	let count: i64 = connection.query_row(
		"SELECT COUNT(*) FROM history_changes
         WHERE account=?1 AND channel=?2 AND message=?3",
		params![account, channel, message],
		|row| row.get(0),
	)?;
	if count > HISTORY_MAX_CHANGES as i64 {
		connection.execute(
			"DELETE FROM history_changes WHERE rowid IN (
               SELECT rowid FROM history_changes
               WHERE account=?1 AND channel=?2 AND message=?3
               ORDER BY seq ASC LIMIT ?4)",
			params![
				account,
				channel,
				message,
				count - HISTORY_MAX_CHANGES as i64
			],
		)?;
	}
	Ok(())
}

fn mark_deleted_in(
	connection: &Connection,
	account: &str,
	channel: &str,
	message: &str,
	observed_at: i64,
	max_tombstones: u32,
) -> Result<HistoryWrite> {
	// An unknown deletion retains only identities, never an author or body. This also prevents a
	// late pre-delete page from turning the deletion into newly invented retained content.
	connection.execute(
		"INSERT INTO history_tombstones(account,channel,message,observed_at)
         VALUES(?1,?2,?3,?4)
         ON CONFLICT(account,channel,message) DO UPDATE SET observed_at=max(observed_at,?4)",
		params![account, channel, message, observed_at],
	)?;
	let tombstone_count: i64 = connection.query_row(
		"SELECT COUNT(*) FROM history_tombstones WHERE account=?1",
		params![account],
		|row| row.get(0),
	)?;
	if tombstone_count > i64::from(max_tombstones) {
		connection.execute(
			"DELETE FROM history_tombstones WHERE rowid IN (
               SELECT rowid FROM history_tombstones WHERE account=?1
               ORDER BY observed_at ASC,length(channel) ASC,channel ASC,length(message) ASC,message ASC
               LIMIT ?2)",
			params![account, tombstone_count - i64::from(max_tombstones)],
		)?;
	}
	let existing: Option<(Option<String>, i64, i64, i64)> = connection
		.query_row(
			"SELECT CASE WHEN typeof(content)='text' AND length(CAST(content AS BLOB))<=16384 THEN content ELSE NULL END,
                    deleted,revision,updated_at
             FROM history_messages WHERE account=?1 AND channel=?2 AND id=?3",
			params![account, channel, message],
			|row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
		)
		.optional()?;
	let Some((content, deleted, revision, updated)) = existing else {
		return Ok(HistoryWrite::Unknown);
	};
	let Some(content) = content else {
		return Err(StoreError::Incompatible);
	};
	if !matches!(deleted, 0 | 1) || revision < 0 || updated < 0 {
		return Err(StoreError::Incompatible);
	}
	connection.execute(
		"UPDATE history_attachments SET deleted=1
         WHERE account=?1 AND channel=?2 AND message=?3",
		params![account, channel, message],
	)?;
	if deleted != 0 {
		return Ok(HistoryWrite::NoOp);
	}
	let observed_at = observed_at.max(updated);
	insert_history_change(
		connection,
		account,
		channel,
		message,
		HistoryChangeKind::Delete,
		revision,
		&content,
		None,
		observed_at,
	)?;
	connection.execute(
		"UPDATE history_messages SET deleted=1,updated_at=?4
         WHERE account=?1 AND channel=?2 AND id=?3 AND deleted=0",
		params![account, channel, message, observed_at],
	)?;
	Ok(HistoryWrite::Applied)
}

fn delete_history_message(
	connection: &Connection,
	account: &str,
	channel: &str,
	message: &str,
) -> Result<()> {
	connection.execute(
		"DELETE FROM history_attachments WHERE account=?1 AND channel=?2 AND message=?3",
		params![account, channel, message],
	)?;
	connection.execute(
		"DELETE FROM history_changes WHERE account=?1 AND channel=?2 AND message=?3",
		params![account, channel, message],
	)?;
	connection.execute(
		"DELETE FROM history_messages WHERE account=?1 AND channel=?2 AND id=?3",
		params![account, channel, message],
	)?;
	connection.execute(
		"DELETE FROM history_tombstones WHERE account=?1 AND channel=?2 AND message=?3",
		params![account, channel, message],
	)?;
	Ok(())
}

fn enforce_history_limits_in(
	connection: &Connection,
	account: &str,
	settings: &HistorySettings,
) -> Result<usize> {
	let sql = format!(
		"SELECT m.channel,m.id,{HISTORY_MESSAGE_BYTES_SQL}
         FROM history_messages m WHERE m.account=?1
         ORDER BY m.updated_at ASC,length(m.id) ASC,m.id ASC,
                  length(m.channel) ASC,m.channel ASC"
	);
	let mut query = connection.prepare(&sql)?;
	let mut rows = query.query(params![account])?;
	let mut candidates = Vec::new();
	while let Some(row) = rows.next()? {
		candidates.push((
			row.get::<_, String>(0)?,
			row.get::<_, String>(1)?,
			row.get::<_, i64>(2)?,
		));
	}
	drop(rows);
	drop(query);
	let mut count = candidates.len() as u64;
	let mut total = candidates
		.iter()
		.map(|(_, _, bytes)| u64::try_from(*bytes).unwrap_or(0))
		.fold(0u64, |sum, bytes| sum.saturating_add(bytes));
	let mut evicted = 0;
	for (channel, message, bytes) in candidates {
		if count <= u64::from(settings.max_messages) && total <= settings.max_bytes {
			break;
		}
		delete_history_message(connection, account, &channel, &message)?;
		count = count.saturating_sub(1);
		total = total.saturating_sub(u64::try_from(bytes).unwrap_or(0));
		evicted += 1;
	}
	Ok(evicted)
}

fn history_view_inner(
	connection: &Connection,
	account: Id,
	query: &HistoryViewQuery,
) -> Result<HistoryViewPage> {
	if query.limit == 0
		|| query.limit > HISTORY_PAGE_MAX
		|| query.before.is_some() && query.after.is_some()
	{
		return Err(StoreError::Capacity);
	}
	let account = account_key(account)?;
	let mut values = vec![rusqlite::types::Value::from(account.clone())];
	let mut clauses = vec!["m.account=?1".to_owned()];
	let mut bind = |value: rusqlite::types::Value| {
		values.push(value);
		format!("?{}", values.len())
	};
	if let Some(channel) = query.channel {
		let slot = bind(id_key(channel)?.into());
		clauses.push(format!("m.channel={slot}"));
	}
	if let Some(author) = query.author {
		let slot = bind(id_key(author)?.into());
		clauses.push(format!("m.author={slot}"));
	}
	if let Some(message) = query.message {
		let slot = bind(id_key(message)?.into());
		clauses.push(format!("m.id={slot}"));
	}
	match query.filter {
		HistoryViewFilter::All => {}
		HistoryViewFilter::Edited => clauses.push(
			"EXISTS(SELECT 1 FROM history_changes c WHERE c.account=m.account AND c.channel=m.channel AND c.message=m.id AND c.kind=0)"
				.into(),
		),
		HistoryViewFilter::Deleted => clauses.push("m.deleted=1".into()),
	}
	let newer = query.after.is_some();
	if let Some((channel, message)) = query.before.or(query.after) {
		let channel = id_key(channel)?;
		let message = id_key(message)?;
		let observed_at: Option<i64> = connection
			.query_row(
				"SELECT updated_at FROM history_messages WHERE account=?1 AND channel=?2 AND id=?3",
				params![account.as_str(), channel.as_str(), message.as_str()],
				|row| row.get(0),
			)
			.optional()?;
		let Some(observed_at) = observed_at else {
			return Ok(HistoryViewPage {
				messages: Vec::new(),
				has_older: false,
				has_newer: false,
			});
		};
		let comparison = if newer { ">" } else { "<" };
		let at = bind(observed_at.into());
		let len = bind((message.len() as i64).into());
		let id = bind(message.into());
		let channel_len = bind((channel.len() as i64).into());
		let channel = bind(channel.into());
		clauses.push(format!(
			"(m.updated_at,length(m.id),m.id,length(m.channel),m.channel)
			 {comparison} ({at},{len},{id},{channel_len},{channel})"
		));
	}
	let direction = if newer { "ASC" } else { "DESC" };
	let limit = bind(i64::from(query.limit + 1).into());
	let sql = format!(
		"SELECT m.channel,m.id,m.author,m.content,m.deleted,m.revision,m.updated_at
		 FROM history_messages m WHERE {} ORDER BY m.updated_at {direction},
		 length(m.id) {direction},m.id {direction},length(m.channel) {direction},m.channel {direction}
		 LIMIT {limit}",
		clauses.join(" AND ")
	);
	let mut statement = connection.prepare(&sql)?;
	let mut rows = statement.query(rusqlite::params_from_iter(values))?;
	let mut records = Vec::new();
	while let Some(row) = rows.next()? {
		let channel: String = row.get(0)?;
		let message: String = row.get(1)?;
		let author: String = row.get(2)?;
		let content: String = row.get(3)?;
		let deleted = parse_flag(row.get(4)?)?;
		let revision: i64 = row.get(5)?;
		let observed_at: i64 = row.get(6)?;
		if content.len() > HISTORY_MAX_CONTENT_BYTES || revision < 0 || observed_at < 0 {
			return Err(StoreError::Incompatible);
		}
		records.push(HistoryRecord {
			channel: channel.parse().map_err(|_| StoreError::Incompatible)?,
			message: message.parse().map_err(|_| StoreError::Incompatible)?,
			author: author.parse().map_err(|_| StoreError::Incompatible)?,
			content,
			deleted,
			revision: u64::try_from(revision).map_err(|_| StoreError::Incompatible)?,
			observed_at,
			changes: Vec::new(),
			attachments: Vec::new(),
		});
	}
	drop(rows);
	drop(statement);
	let more = records.len() > query.limit as usize;
	records.truncate(query.limit as usize);
	if newer {
		records.reverse();
	}
	for record in &mut records {
		let channel = record.channel.to_string();
		let message = record.message.to_string();
		record.changes = load_history_changes(connection, &account, &channel, &message)?;
		record.attachments = load_history_attachments(connection, &account, &channel, &message)?;
	}
	Ok(HistoryViewPage {
		messages: records,
		has_older: query.message.is_none() && if newer { query.after.is_some() } else { more },
		has_newer: query.message.is_none() && if newer { more } else { query.before.is_some() },
	})
}

fn history_page_inner(
	connection: &Connection,
	account: Id,
	channel: Option<Id>,
	cursor: Option<HistoryCursor>,
	limit: u32,
) -> Result<(Vec<HistoryRecord>, Option<HistoryCursor>)> {
	let account = account_key(account)?;
	let channel = match channel {
		Some(channel) => Some(id_key(channel)?),
		None => None,
	};
	let (cursor_at, cursor_id_len, cursor_id, cursor_channel_len, cursor_channel) = match cursor {
		Some(cursor) => {
			if cursor.observed_at < 0 || cursor.channel.0 == 0 || cursor.message.0 == 0 {
				return Err(StoreError::Capacity);
			}
			let id = cursor.message.to_string();
			let channel = cursor.channel.to_string();
			(
				Some(cursor.observed_at),
				Some(id.len() as i64),
				Some(id),
				Some(channel.len() as i64),
				Some(channel),
			)
		}
		None => (None, None, None, None, None),
	};
	let mut query = connection.prepare(
		"SELECT m.channel,m.id,m.author,
                CASE WHEN typeof(m.content)='text' AND length(CAST(m.content AS BLOB))<=16384
                     THEN m.content ELSE NULL END,
                m.deleted,m.revision,m.updated_at
         FROM history_messages m
         WHERE m.account=?1
           AND (?2 IS NULL OR m.channel=?2)
           AND (
             ?3 IS NULL
             OR m.updated_at < ?3
             OR (m.updated_at = ?3 AND (
                  length(m.id) < ?4
                  OR (length(m.id) = ?4 AND (
                       m.id < ?5
                       OR (m.id = ?5 AND (
                            length(m.channel) < ?6
                            OR (length(m.channel) = ?6 AND m.channel < ?7)
                       ))
                  ))
             ))
           )
         ORDER BY m.updated_at DESC,length(m.id) DESC,m.id DESC,
                  length(m.channel) DESC,m.channel DESC
         LIMIT ?8",
	)?;
	let mut rows = query.query(params![
		account.as_str(),
		channel.as_deref(),
		cursor_at,
		cursor_id_len,
		cursor_id.as_deref(),
		cursor_channel_len,
		cursor_channel.as_deref(),
		i64::from(limit),
	])?;
	let mut messages = Vec::with_capacity(limit as usize);
	while let Some(row) = rows.next()? {
		let channel_text: String = row.get(0)?;
		let message_text: String = row.get(1)?;
		let author_text: String = row.get(2)?;
		let content: Option<String> = row.get(3)?;
		let deleted: i64 = row.get(4)?;
		let revision: i64 = row.get(5)?;
		let observed_at: i64 = row.get(6)?;
		let Some(content) = content else {
			return Err(StoreError::Incompatible);
		};
		if content.len() > HISTORY_MAX_CONTENT_BYTES
			|| !matches!(deleted, 0 | 1)
			|| revision < 0
			|| observed_at < 0
		{
			return Err(StoreError::Incompatible);
		}
		messages.push(HistoryRecord {
			channel: channel_text
				.parse::<Id>()
				.map_err(|_| StoreError::Incompatible)?,
			message: message_text
				.parse::<Id>()
				.map_err(|_| StoreError::Incompatible)?,
			author: author_text
				.parse::<Id>()
				.map_err(|_| StoreError::Incompatible)?,
			content,
			deleted: deleted != 0,
			revision: u64::try_from(revision).map_err(|_| StoreError::Incompatible)?,
			observed_at,
			changes: Vec::new(),
			attachments: Vec::new(),
		});
	}
	drop(rows);
	drop(query);
	for record in &mut messages {
		let channel = record.channel.to_string();
		let message = record.message.to_string();
		record.changes = load_history_changes(connection, &account, &channel, &message)?;
		record.attachments = load_history_attachments(connection, &account, &channel, &message)?;
	}
	let next = if messages.len() == limit as usize {
		messages.last().map(|record| HistoryCursor {
			observed_at: record.observed_at,
			channel: record.channel,
			message: record.message,
		})
	} else {
		None
	};
	Ok((messages, next))
}

fn load_history_changes(
	connection: &Connection,
	account: &str,
	channel: &str,
	message: &str,
) -> Result<Vec<HistoryChange>> {
	let mut query = connection.prepare(
		"SELECT kind,revision,before,after,observed_at FROM history_changes
         WHERE account=?1 AND channel=?2 AND message=?3 ORDER BY seq ASC",
	)?;
	let mut rows = query.query(params![account, channel, message])?;
	let mut changes = Vec::new();
	while let Some(row) = rows.next()? {
		if changes.len() >= HISTORY_MAX_CHANGES {
			return Err(StoreError::Incompatible);
		}
		let kind = HistoryChangeKind::from_i64(row.get(0)?).ok_or(StoreError::Incompatible)?;
		let revision: i64 = row.get(1)?;
		let before: Option<String> = row.get(2)?;
		let after: Option<String> = row.get(3)?;
		let observed_at: i64 = row.get(4)?;
		if revision < 0 || observed_at < 0 {
			return Err(StoreError::Incompatible);
		}
		if before
			.as_ref()
			.is_some_and(|text| text.len() > HISTORY_MAX_CONTENT_BYTES)
			|| after
				.as_ref()
				.is_some_and(|text| text.len() > HISTORY_MAX_CONTENT_BYTES)
		{
			return Err(StoreError::Incompatible);
		}
		let coherent = match kind {
			HistoryChangeKind::Edit => before.is_some() && after.is_some(),
			HistoryChangeKind::Delete => before.is_some() && after.is_none(),
		};
		if !coherent {
			return Err(StoreError::Incompatible);
		}
		changes.push(HistoryChange {
			kind,
			revision: u64::try_from(revision).map_err(|_| StoreError::Incompatible)?,
			before,
			after,
			observed_at,
		});
	}
	Ok(changes)
}

fn load_history_attachments(
	connection: &Connection,
	account: &str,
	channel: &str,
	message: &str,
) -> Result<Vec<HistoryAttachment>> {
	let mut query = connection.prepare(
		"SELECT attachment_id,filename,content_type,size,kind,download_state,deleted,stored_bytes
         FROM history_attachments
         WHERE account=?1 AND channel=?2 AND message=?3
         ORDER BY length(attachment_id),attachment_id",
	)?;
	let mut rows = query.query(params![account, channel, message])?;
	let mut attachments = Vec::new();
	while let Some(row) = rows.next()? {
		if attachments.len() >= HISTORY_MAX_ATTACHMENTS {
			return Err(StoreError::Incompatible);
		}
		let id: String = row.get(0)?;
		let filename: String = row.get(1)?;
		let content_type: Option<String> = row.get(2)?;
		let size: i64 = row.get(3)?;
		let kind = HistoryAttachmentKind::from_i64(row.get(4)?).ok_or(StoreError::Incompatible)?;
		let state =
			HistoryAttachmentState::from_i64(row.get(5)?).ok_or(StoreError::Incompatible)?;
		let deleted = parse_flag(row.get(6)?)?;
		let stored_bytes: i64 = row.get(7)?;
		if filename.is_empty()
			|| filename.len() > 256
			|| content_type.as_ref().is_some_and(|value| value.len() > 127)
			|| size < 0
			|| stored_bytes < 0
			|| stored_bytes > kind.download_cap() as i64
		{
			return Err(StoreError::Incompatible);
		}
		attachments.push(HistoryAttachment {
			id: id.parse::<Id>().map_err(|_| StoreError::Incompatible)?,
			filename,
			content_type,
			size: u64::try_from(size).map_err(|_| StoreError::Incompatible)?,
			kind,
			state,
			deleted,
			stored_bytes: u64::try_from(stored_bytes).map_err(|_| StoreError::Incompatible)?,
		});
	}
	Ok(attachments)
}

#[cfg(test)]
mod tests {
	use super::*;
	use rusqlite::Connection;
	use std::collections::BTreeSet;

	fn test_store() -> LocalStore {
		let connection = Connection::open_in_memory().unwrap();
		create_tables(&connection).unwrap();
		LocalStore(connection)
	}

	fn enabled_settings() -> HistorySettings {
		HistorySettings {
			enabled: true,
			..HistorySettings::default()
		}
	}

	fn baseline<'a>(
		channel: Id,
		message: Id,
		author: Id,
		content: &'a str,
		observed_at: i64,
	) -> HistoryBaseline<'a> {
		HistoryBaseline {
			channel,
			message,
			author,
			content,
			revision: 1,
			observed_at,
			attachments: &[],
		}
	}

	#[test]
	fn migration_is_idempotent_and_settings_default_to_explicitly_off() {
		let connection = Connection::open_in_memory().unwrap();
		migrate(&connection, 30).unwrap();
		create_tables(&connection).unwrap();
		let mut store = LocalStore(connection);
		assert_eq!(
			store.history_settings(Id(1)).unwrap(),
			HistorySettings::default()
		);
		let value = HistorySettings {
			enabled: true,
			ignore_users: vec![Id(2)],
			ignore_channels: vec![Id(3)],
			..HistorySettings::default()
		};
		store.save_history_settings(Id(1), &value).unwrap();
		assert_eq!(store.history_settings(Id(1)).unwrap(), value);
		assert!(!store.history_settings(Id(2)).unwrap().enabled);
	}

	#[test]
	fn settings_validation_rejects_zero_unbounded_values_and_duplicate_filters() {
		let mut store = test_store();
		let mut value = HistorySettings::default();
		store.save_history_settings(Id(1), &value).unwrap();

		value.retention_days = 0;
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.retention_days = 366;
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.retention_days = 1;
		value.max_messages = 99;
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.max_messages = 0;
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.max_messages = 50_001;
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.max_messages = 100;
		value.max_bytes = 0;
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.max_bytes = HISTORY_SETTINGS_BYTES_MIN - 1;
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.max_bytes = HISTORY_SETTINGS_BYTES_MAX + 1;
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.max_bytes = HISTORY_SETTINGS_BYTES_DEFAULT;
		value.ignore_users = vec![Id(0)];
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.ignore_users = vec![Id(9), Id(9)];
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.ignore_users = vec![Id(9); HISTORY_FILTER_MAX + 1];
		assert_eq!(
			store.save_history_settings(Id(1), &value),
			Err(StoreError::Capacity)
		);
		value.ignore_users = (1..=HISTORY_FILTER_MAX as u64).map(Id).collect();
		store.save_history_settings(Id(1), &value).unwrap();
		assert_eq!(
			store.history_settings(Id(1)).unwrap().ignore_users.len(),
			HISTORY_FILTER_MAX
		);
	}

	#[test]
	fn unknown_edits_and_deletes_create_nothing_and_tombstones_stay_deleted() {
		let mut store = test_store();
		store
			.save_history_settings(Id(1), &enabled_settings())
			.unwrap();
		let first = baseline(Id(10), Id(100), Id(7), "first", 1);
		assert_eq!(
			store.record_history_baseline(Id(1), &first).unwrap(),
			HistoryWrite::Applied
		);
		assert_eq!(
			store.record_history_baseline(Id(1), &first).unwrap(),
			HistoryWrite::NoOp
		);
		assert_eq!(
			store
				.record_history_edit(Id(1), Id(10), Id(100), 2, "second", 2)
				.unwrap(),
			HistoryWrite::Applied
		);
		assert_eq!(
			store
				.record_history_edit(Id(1), Id(10), Id(100), 3, "second", 3)
				.unwrap(),
			HistoryWrite::NoOp
		);
		assert_eq!(
			store
				.record_history_delete(Id(1), Id(10), Id(100), 4)
				.unwrap(),
			HistoryWrite::Applied
		);
		assert_eq!(
			store
				.record_history_delete(Id(1), Id(10), Id(100), 5)
				.unwrap(),
			HistoryWrite::NoOp
		);
		assert_eq!(
			store
				.record_history_edit(Id(1), Id(10), Id(100), 6, "third", 6)
				.unwrap(),
			HistoryWrite::NoOp
		);
		let resurrect = baseline(Id(10), Id(100), Id(7), "resurrect", 7);
		assert_eq!(
			store.record_history_baseline(Id(1), &resurrect).unwrap(),
			HistoryWrite::NoOp
		);
		assert_eq!(
			store
				.record_history_edit(Id(1), Id(11), Id(999), 1, "x", 1)
				.unwrap(),
			HistoryWrite::Unknown
		);
		assert_eq!(
			store
				.record_history_delete(Id(1), Id(11), Id(999), 1)
				.unwrap(),
			HistoryWrite::Unknown
		);
		let unknown_body = baseline(Id(11), Id(999), Id(7), "not reconstructable", 2);
		assert_eq!(
			store.record_history_baseline(Id(1), &unknown_body).unwrap(),
			HistoryWrite::NoOp,
			"a late page cannot recreate an ID-only deletion tombstone"
		);
		assert_eq!(store.history_usage(Id(1)).unwrap().messages, 1);
		let page = store.history_page(Id(1), None, None, 10).unwrap();
		assert_eq!(page.messages.len(), 1);
		let record = &page.messages[0];
		assert!(record.deleted);
		assert_eq!(record.content, "second");
		assert_eq!(record.changes.len(), 2);
		assert_eq!(record.changes[0].kind, HistoryChangeKind::Edit);
		assert_eq!(record.changes[0].before.as_deref(), Some("first"));
		assert_eq!(record.changes[0].after.as_deref(), Some("second"));
		assert_eq!(record.changes[1].kind, HistoryChangeKind::Delete);
		assert_eq!(record.changes[1].after, None);
	}

	#[test]
	fn viewer_filters_page_in_both_directions_and_author_purges_stay_scoped() {
		let mut store = test_store();
		store
			.save_history_settings(Id(1), &enabled_settings())
			.unwrap();
		for (channel, message, author) in [(10, 101, 7), (10, 102, 8), (11, 103, 7)] {
			store
				.record_history_baseline(
					Id(1),
					&baseline(Id(channel), Id(message), Id(author), "body", message as i64),
				)
				.unwrap();
		}
		store
			.record_history_delete(Id(1), Id(10), Id(101), 100)
			.unwrap();
		let mut query = HistoryViewQuery {
			filter: HistoryViewFilter::Deleted,
			limit: 10,
			channel: None,
			author: None,
			message: None,
			before: None,
			after: None,
		};
		let deleted = store.history_view_page(Id(1), &query).unwrap();
		assert_eq!(deleted.messages.len(), 1);
		assert!(deleted.messages[0].deleted);
		query.filter = HistoryViewFilter::All;
		query.author = Some(Id(7));
		query.limit = 1;
		let newest = store.history_view_page(Id(1), &query).unwrap();
		assert_eq!(newest.messages.len(), 1);
		assert_eq!(newest.messages[0].message, Id(103));
		assert!(newest.has_older);
		query.before = Some((Id(11), Id(103)));
		query.after = None;
		let older = store.history_view_page(Id(1), &query).unwrap();
		assert_eq!(older.messages[0].message, Id(101));
		query.before = None;
		query.after = Some((Id(10), Id(101)));
		let newer = store.history_view_page(Id(1), &query).unwrap();
		assert_eq!(newer.messages[0].message, Id(103));
		assert_eq!(store.clear_history_author(Id(1), Id(7)).unwrap(), 2);
		assert_eq!(
			store
				.history_page(Id(1), None, None, 10)
				.unwrap()
				.messages
				.len(),
			1
		);
		assert_eq!(
			store.history_page(Id(1), None, None, 10).unwrap().messages[0].author,
			Id(8)
		);
	}

	#[test]
	fn ninth_change_evicts_the_oldest_version() {
		let mut store = test_store();
		store
			.save_history_settings(Id(1), &enabled_settings())
			.unwrap();
		let initial = baseline(Id(10), Id(100), Id(7), "v0", 0);
		store.record_history_baseline(Id(1), &initial).unwrap();
		for revision in 1..=9 {
			let content = format!("v{revision}");
			assert_eq!(
				store
					.record_history_edit(
						Id(1),
						Id(10),
						Id(100),
						revision,
						&content,
						revision as i64,
					)
					.unwrap(),
				HistoryWrite::Applied
			);
		}
		let page = store.history_page(Id(1), None, None, 10).unwrap();
		let record = &page.messages[0];
		assert_eq!(record.content, "v9");
		assert_eq!(record.changes.len(), HISTORY_MAX_CHANGES);
		assert_eq!(record.changes[0].before.as_deref(), Some("v1"));
		assert_eq!(record.changes[0].after.as_deref(), Some("v2"));
		assert_eq!(record.changes[7].before.as_deref(), Some("v8"));
		assert_eq!(record.changes[7].after.as_deref(), Some("v9"));
	}

	#[test]
	fn count_and_byte_limits_evict_oldest_messages() {
		let mut store = test_store();
		let mut settings = enabled_settings();
		settings.max_messages = 100;
		store.save_history_settings(Id(1), &settings).unwrap();
		for message in 1..=130 {
			let content = format!("m{message}");
			store
				.record_history_baseline(
					Id(1),
					&baseline(Id(10), Id(message), Id(7), &content, message as i64),
				)
				.unwrap();
		}
		let usage = store.history_usage(Id(1)).unwrap();
		assert_eq!(usage.messages, 100);
		let page = store.history_page(Id(1), None, None, 100).unwrap();
		assert_eq!(page.messages.len(), 100);
		assert_eq!(page.messages[0].message, Id(130));
		assert_eq!(page.messages[99].message, Id(31));

		let mut settings = enabled_settings();
		settings.max_messages = HISTORY_SETTINGS_MESSAGES_MAX;
		settings.max_bytes = HISTORY_SETTINGS_BYTES_MIN;
		store.save_history_settings(Id(1), &settings).unwrap();
		let content = "x".repeat(HISTORY_MAX_CONTENT_BYTES);
		for message in 1..=260 {
			store
				.record_history_baseline(
					Id(1),
					&baseline(Id(20), Id(message), Id(7), &content, 1000 + message as i64),
				)
				.unwrap();
		}
		let usage = store.history_usage(Id(1)).unwrap();
		assert!(usage.messages > 0);
		assert!(usage.messages < 260, "byte budget must evict old rows");
		assert!(usage.bytes <= HISTORY_SETTINGS_BYTES_MIN);
	}

	#[test]
	fn page_cursor_has_no_duplicates_and_channel_filter_is_account_scoped() {
		let mut store = test_store();
		store
			.save_history_settings(Id(1), &enabled_settings())
			.unwrap();
		store
			.save_history_settings(Id(2), &enabled_settings())
			.unwrap();
		for message in 1..=5 {
			store
				.record_history_baseline(
					Id(1),
					&baseline(Id(20), Id(message), Id(7), "a", message as i64),
				)
				.unwrap();
		}
		for message in 10..=12 {
			store
				.record_history_baseline(
					Id(1),
					&baseline(Id(21), Id(message), Id(7), "b", message as i64),
				)
				.unwrap();
		}
		store
			.record_history_baseline(Id(2), &baseline(Id(20), Id(99), Id(8), "other", 99))
			.unwrap();

		let mut seen = BTreeSet::new();
		let mut cursor = None;
		loop {
			let page = store.history_page(Id(1), None, cursor, 2).unwrap();
			if page.messages.is_empty() {
				break;
			}
			for record in &page.messages {
				assert!(
					seen.insert((record.channel.0, record.message.0)),
					"cursor returned a duplicate row"
				);
			}
			cursor = page.next;
			if cursor.is_none() {
				break;
			}
		}
		assert_eq!(seen.len(), 8);

		let filtered = store.history_page(Id(1), Some(Id(20)), None, 100).unwrap();
		assert_eq!(filtered.messages.len(), 5);
		assert!(
			filtered
				.messages
				.iter()
				.all(|record| record.channel == Id(20))
		);
		assert_eq!(
			store
				.history_page(Id(1), None, None, 100)
				.unwrap()
				.messages
				.iter()
				.filter(|record| record.message == Id(99))
				.count(),
			0
		);
		assert_eq!(
			store
				.history_page(Id(1), None, None, HISTORY_PAGE_HARD_MAX + 1)
				.unwrap_err(),
			StoreError::Capacity
		);
		assert_eq!(
			store
				.history_export_page(Id(1), None, None, HISTORY_PAGE_HARD_MAX)
				.unwrap()
				.messages
				.len(),
			8
		);
	}

	#[test]
	fn retention_and_clear_only_touch_the_requested_account() {
		let mut store = test_store();
		let mut settings = enabled_settings();
		settings.retention_days = 1;
		store.save_history_settings(Id(1), &settings).unwrap();
		store.save_history_settings(Id(2), &settings).unwrap();
		store
			.record_history_baseline(Id(1), &baseline(Id(10), Id(1), Id(7), "old", 0))
			.unwrap();
		store
			.record_history_baseline(
				Id(1),
				&baseline(Id(10), Id(2), Id(7), "fresh", 10 * HISTORY_DAY_MS),
			)
			.unwrap();
		store
			.record_history_baseline(Id(2), &baseline(Id(10), Id(1), Id(7), "old", 0))
			.unwrap();

		assert_eq!(store.expire_history(Id(1), 10 * HISTORY_DAY_MS).unwrap(), 1);
		let remaining = store.history_page(Id(1), None, None, 100).unwrap();
		assert_eq!(remaining.messages.len(), 1);
		assert_eq!(remaining.messages[0].message, Id(2));
		assert_eq!(
			store
				.history_page(Id(2), None, None, 100)
				.unwrap()
				.messages
				.len(),
			1
		);

		assert_eq!(store.clear_history_channel(Id(1), Id(10)).unwrap(), 1);
		assert!(
			store
				.history_page(Id(1), None, None, 100)
				.unwrap()
				.messages
				.is_empty()
		);
		assert_eq!(
			store
				.history_page(Id(2), None, None, 100)
				.unwrap()
				.messages
				.len(),
			1
		);

		store
			.record_history_baseline(Id(1), &baseline(Id(10), Id(3), Id(7), "new", 20))
			.unwrap();
		assert_eq!(store.clear_history_account(Id(1)).unwrap(), 1);
		settings.enabled = false;
		assert_eq!(store.history_settings(Id(1)).unwrap(), settings);
		assert!(store.history_settings_configured(Id(1)).unwrap());
		assert_eq!(
			store
				.history_page(Id(1), None, None, 100)
				.unwrap()
				.messages
				.len(),
			0
		);
	}

	#[test]
	fn account_forget_helper_removes_history_inside_a_parent_transaction() {
		let mut store = test_store();
		store
			.save_history_settings(Id(1), &enabled_settings())
			.unwrap();
		store
			.record_history_baseline(Id(1), &baseline(Id(10), Id(1), Id(7), "body", 1))
			.unwrap();
		let transaction = store.0.transaction().unwrap();
		forget_account_in(&transaction, Id(1)).unwrap();
		transaction.commit().unwrap();
		assert_eq!(store.history_usage(Id(1)).unwrap().messages, 0);
		assert_eq!(
			store.history_settings(Id(1)).unwrap(),
			HistorySettings::default()
		);
		assert!(!store.history_settings_configured(Id(1)).unwrap());
	}

	#[test]
	fn attachment_reservations_enforce_type_opt_in_per_file_and_account_caps() {
		let mut store = test_store();
		let mut settings = enabled_settings();
		settings.downloads.images = true;
		settings.downloads.video = true;
		store.save_history_settings(Id(1), &settings).unwrap();
		let attachments = [
			HistoryAttachmentInput {
				id: Id(1000),
				filename: "a.png",
				content_type: Some("image/png"),
				size: 1024,
				kind: HistoryAttachmentKind::Image,
			},
			HistoryAttachmentInput {
				id: Id(1001),
				filename: "sound.bin",
				content_type: Some("audio/ogg"),
				size: 2048,
				kind: HistoryAttachmentKind::Audio,
			},
		];
		let record = HistoryBaseline {
			channel: Id(10),
			message: Id(100),
			author: Id(7),
			content: "body",
			revision: 1,
			observed_at: 1,
			attachments: &attachments,
		};
		store.record_history_baseline(Id(1), &record).unwrap();

		assert_eq!(
			store
				.reserve_history_attachment(Id(1), Id(10), Id(100), Id(1001), 2048)
				.unwrap(),
			HistoryReservation::Disabled
		);
		assert_eq!(
			store
				.reserve_history_attachment(
					Id(1),
					Id(10),
					Id(100),
					Id(1000),
					HISTORY_ATTACHMENT_IMAGE_BYTES + 1,
				)
				.unwrap(),
			HistoryReservation::TooLarge
		);
		assert_eq!(
			store
				.reserve_history_attachment(Id(1), Id(10), Id(100), Id(1000), 1024)
				.unwrap(),
			HistoryReservation::Reserved { bytes: 1024 }
		);
		assert_eq!(
			store
				.reserve_history_attachment(Id(1), Id(10), Id(100), Id(1000), 1024)
				.unwrap(),
			HistoryReservation::AlreadyReserved
		);
		assert_eq!(
			store
				.complete_history_attachment(Id(1), Id(10), Id(100), Id(1000), 1024)
				.unwrap(),
			HistoryAttachmentUpdate::Stored
		);
		assert_eq!(
			store
				.reserve_history_attachment(Id(1), Id(10), Id(100), Id(1000), 1024)
				.unwrap(),
			HistoryReservation::AlreadyStored
		);
		assert_eq!(store.history_usage(Id(1)).unwrap().attachment_bytes, 1024);
		let page = store.history_page(Id(1), None, None, 10).unwrap();
		assert_eq!(
			page.messages[0].attachments[0].state,
			HistoryAttachmentState::Stored
		);

		let video_bytes = 26 * 1024 * 1024;
		let videos: Vec<_> = (0..HISTORY_MAX_ATTACHMENTS)
			.map(|index| HistoryAttachmentInput {
				id: Id(2000 + index as u64),
				filename: "clip.mp4",
				content_type: Some("video/mp4"),
				size: video_bytes,
				kind: HistoryAttachmentKind::Video,
			})
			.collect();
		let bulk = HistoryBaseline {
			channel: Id(10),
			message: Id(200),
			author: Id(7),
			content: "videos",
			revision: 1,
			observed_at: 2,
			attachments: &videos,
		};
		store.record_history_baseline(Id(1), &bulk).unwrap();
		for index in 0..9 {
			assert_eq!(
				store
					.reserve_history_attachment(
						Id(1),
						Id(10),
						Id(200),
						Id(2000 + index),
						video_bytes,
					)
					.unwrap(),
				HistoryReservation::Reserved { bytes: video_bytes }
			);
		}
		assert_eq!(
			store
				.reserve_history_attachment(Id(1), Id(10), Id(200), Id(2009), video_bytes)
				.unwrap(),
			HistoryReservation::AccountFull
		);
		assert!(
			store
				.release_history_attachment(Id(1), Id(10), Id(200), Id(2000), true)
				.unwrap()
		);
		assert_eq!(
			store
				.reserve_history_attachment(Id(1), Id(10), Id(200), Id(2009), video_bytes)
				.unwrap(),
			HistoryReservation::Reserved { bytes: video_bytes }
		);
	}
}
