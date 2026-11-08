//! Native adapter between client-core observations and bounded SQLite history.
//!
//! This module runs on the cache worker, never in rendering or an audio callback. The observation
//! timestamp is the durable ordering revision: client-core revisions restart with each session.
use local_store::message_history::{
	HistoryAlertMode, HistoryAttachmentInput, HistoryAttachmentKind, HistoryBaseline,
	HistoryChangeKind, HistoryRecord, HistoryViewFilter, HistoryViewQuery,
};
use model::{
	Id,
	message_logger::{AttachmentMeta, Entry, Kind, Mutation},
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ViewPage {
	pub generation: u64,
	pub account: Id,
	pub channel: Option<Id>,
	pub author: Option<Id>,
	pub message: Option<Id>,
	pub kind: ui::message_history::Filter,
	pub before: Option<(Id, Id)>,
	pub after: Option<(Id, Id)>,
	pub limit: u32,
}

#[derive(Clone, Debug)]
pub struct ExportRequest {
	pub destination: PathBuf,
	pub format: ui::message_history::ExportFormat,
	pub cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

pub fn apply_mutations(
	store: &mut local_store::LocalStore,
	account: Id,
	mutations: &[Mutation],
) -> std::result::Result<usize, local_store::StoreError> {
	if account.0 == 0
		|| mutations.is_empty()
		|| mutations.len() > model::message_logger::MAX_MUTATIONS
		|| mutations.iter().map(Mutation::bytes).sum::<usize>()
			> model::message_logger::MAX_MUTATION_BYTES
		|| mutations
			.iter()
			.any(|mutation| mutation.account() != account || !mutation.valid())
	{
		return Err(local_store::StoreError::Capacity);
	}
	let capture = store.history_settings(account)?.enabled;
	let mut applied = 0;
	for mutation in mutations {
		match mutation {
			Mutation::Baseline {
				observed_at_ms,
				channel,
				message,
				author,
				content,
				attachments,
				..
			} if capture => {
				if *observed_at_ms < 0 {
					return Err(local_store::StoreError::Capacity);
				}
				let metadata: Vec<HistoryAttachmentInput<'_>> = attachments
					.iter()
					.map(|attachment| HistoryAttachmentInput {
						id: attachment.id,
						filename: &attachment.filename,
						content_type: attachment.content_type.as_deref(),
						size: attachment.size,
						kind: attachment_kind(attachment),
					})
					.collect();
				let baseline = HistoryBaseline {
					channel: *channel,
					message: *message,
					author: *author,
					content,
					revision: *observed_at_ms as u64,
					observed_at: *observed_at_ms,
					attachments: &metadata,
				};
				store.record_history_baseline(account, &baseline)?;
				applied += 1;
			}
			Mutation::Edit {
				observed_at_ms,
				channel,
				message,
				after,
				..
			} if capture => {
				if *observed_at_ms < 0 {
					return Err(local_store::StoreError::Capacity);
				}
				store.record_history_edit(
					account,
					*channel,
					*message,
					*observed_at_ms as u64,
					after,
					*observed_at_ms,
				)?;
				applied += 1;
			}
			Mutation::Delete {
				observed_at_ms,
				channel,
				message,
				..
			} if capture => {
				store.record_history_delete(account, *channel, *message, *observed_at_ms)?;
				applied += 1;
			}
			Mutation::PurgeChannel { channel, .. } => {
				store.clear_history_channel(account, *channel)?;
				applied += 1;
			}
			Mutation::PurgeAuthor { author, .. } => {
				store.clear_history_author(account, *author)?;
				applied += 1;
			}
			Mutation::ClearAccount { .. } => {
				store.clear_history_account(account)?;
				applied += 1;
			}
			// Capture can be off while a persisted row is still readable or being purged.
			Mutation::Baseline { .. } | Mutation::Edit { .. } | Mutation::Delete { .. } => {}
		}
	}
	Ok(applied)
}

fn attachment_kind(attachment: &AttachmentMeta) -> HistoryAttachmentKind {
	let kind = attachment.content_type.as_deref().unwrap_or("");
	if kind.starts_with("image/") {
		HistoryAttachmentKind::Image
	} else if kind.starts_with("audio/") {
		HistoryAttachmentKind::Audio
	} else if kind.starts_with("video/") {
		HistoryAttachmentKind::Video
	} else {
		HistoryAttachmentKind::File
	}
}

pub fn load_view_page(
	store: &local_store::LocalStore,
	page: ViewPage,
) -> std::result::Result<ui::message_history::HistoryPage, local_store::StoreError> {
	if page.account.0 == 0 || !(1..=100).contains(&page.limit) {
		return Err(local_store::StoreError::Capacity);
	}
	let result = store.history_view_page(
		page.account,
		&HistoryViewQuery {
			channel: page.channel,
			author: page.author,
			message: page.message,
			filter: match page.kind {
				ui::message_history::Filter::All => HistoryViewFilter::All,
				ui::message_history::Filter::Edited => HistoryViewFilter::Edited,
				ui::message_history::Filter::Deleted => HistoryViewFilter::Deleted,
			},
			before: page.before,
			after: page.after,
			limit: page.limit,
		},
	)?;
	Ok(ui::message_history::HistoryPage {
		generation: page.generation,
		account: Some(page.account),
		channel: page.channel,
		message: page.message,
		entries: result.messages.iter().map(record_entry).collect(),
		has_older: result.has_older,
		has_newer: result.has_newer,
	})
}

fn record_entry(record: &HistoryRecord) -> Entry {
	Entry {
		channel: record.channel,
		message: record.message,
		author: record.author,
		revision: record.revision,
		content: record.content.clone(),
		deleted: record.deleted,
		events: record
			.changes
			.iter()
			.map(|change| model::message_logger::Change {
				kind: match change.kind {
					HistoryChangeKind::Edit => Kind::Edit,
					HistoryChangeKind::Delete => Kind::Delete,
				},
				revision: change.revision,
				before: change.before.clone(),
				after: change.after.clone(),
			})
			.collect(),
		attachments: record
			.attachments
			.iter()
			.map(|attachment| AttachmentMeta {
				id: attachment.id,
				filename: attachment.filename.clone(),
				description: None,
				content_type: attachment.content_type.clone(),
				size: attachment.size,
				spoiler: false,
				duration_ms: None,
				waveform: Vec::new(),
				deleted: attachment.deleted,
			})
			.collect(),
		mention_everyone: false,
		mentions_self: false,
		edited_at: (!record.changes.is_empty()).then_some(
			i128::from(record.observed_at) * time::Duration::MILLISECOND.whole_nanoseconds(),
		),
		observed_at_ms: record.observed_at,
	}
}

pub fn export(
	store: &local_store::LocalStore,
	account: Id,
	request: ExportRequest,
) -> std::result::Result<
	super::message_history_export::ExportSummary,
	super::message_history_export::ExportError,
> {
	use super::message_history_export as output;
	if account.0 == 0 {
		return Err(output::ExportError::InvalidRecord("account"));
	}
	let mut cursor = None;
	let mut exhausted = false;
	let mut pending: std::collections::VecDeque<output::ExportRecord> =
		std::collections::VecDeque::new();
	let rows = std::iter::from_fn(|| {
		loop {
			if let Some(mut record) = pending.pop_front() {
				record.account = account.0;
				return Some(Ok(record));
			}
			if exhausted {
				return None;
			}
			let page = match store.history_export_page(account, None, cursor, 500) {
				Ok(page) => page,
				Err(_) => {
					exhausted = true;
					return Some(Err(output::ExportError::Io(
						"Could not read the history snapshot",
					)));
				}
			};
			cursor = page.next;
			exhausted = cursor.is_none();
			pending.extend(page.messages.into_iter().map(export_record));
		}
	});
	let next = export_path(&request.destination)?;
	let options = output::ExportOptions { overwrite: false };
	match request.format {
		ui::message_history::ExportFormat::Text => output::export_text_results(
			&next,
			output::ExportScope { account: account.0 },
			rows,
			options,
			request.cancelled.as_ref(),
		),
		ui::message_history::ExportFormat::Json => output::export_json_results(
			&next,
			output::ExportScope { account: account.0 },
			rows,
			options,
			request.cancelled.as_ref(),
		),
	}
}

fn export_path(
	destination: &Path,
) -> std::result::Result<PathBuf, super::message_history_export::ExportError> {
	if destination.as_os_str().len() > 1024 {
		return Err(super::message_history_export::ExportError::InvalidDestination);
	}
	Ok(destination.to_path_buf())
}

fn export_record(record: HistoryRecord) -> super::message_history_export::ExportRecord {
	super::message_history_export::ExportRecord {
		account: 0, // Filled by the single-account iterator below.
		channel: record.channel.0,
		message: record.message.0,
		author: record.author.0,
		author_name: record.author.to_string(),
		content: record.content,
		edited: !record.changes.is_empty(),
		deleted: record.deleted,
		timestamp: Some(record.observed_at),
		changes: record
			.changes
			.into_iter()
			.map(|change| super::message_history_export::ExportChange {
				kind: match change.kind {
					HistoryChangeKind::Edit => {
						super::message_history_export::ExportChangeKind::Edit
					}
					HistoryChangeKind::Delete => {
						super::message_history_export::ExportChangeKind::Delete
					}
				},
				revision: change.revision,
				before: change.before.unwrap_or_default(),
				after: change.after,
				observed_at: change.observed_at,
			})
			.collect(),
		attachments: record
			.attachments
			.into_iter()
			.map(
				|attachment| super::message_history_export::ExportAttachment {
					filename: attachment.filename,
					content_type: attachment.content_type,
					size: attachment.size,
				},
			)
			.collect(),
	}
}

pub fn ghost_mode(mode: HistoryAlertMode) -> model::message_logger::GhostMode {
	match mode {
		HistoryAlertMode::Off => model::message_logger::GhostMode::Off,
		HistoryAlertMode::Mentions => model::message_logger::GhostMode::Mentions,
		HistoryAlertMode::All => model::message_logger::GhostMode::All,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use local_store::message_history::HistorySettings;
	use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

	fn db_path(name: &str) -> PathBuf {
		static NEXT: AtomicU64 = AtomicU64::new(0);
		let path = std::env::temp_dir().join(format!(
			"serein-history-adapter-{name}-{}-{}.sqlite3",
			std::process::id(),
			NEXT.fetch_add(1, Ordering::Relaxed)
		));
		let _ = std::fs::remove_file(&path);
		path
	}

	fn cleanup(path: &Path) {
		for suffix in ["", "-wal", "-shm"] {
			let _ = std::fs::remove_file(path.with_extension(format!("sqlite3{suffix}")));
		}
	}

	fn baseline(message: u64, observed_at: i64) -> Mutation {
		Mutation::Baseline {
			account: Id(7),
			epoch: 1,
			observed_at_ms: observed_at,
			channel: Id(10),
			message: Id(message),
			author: Id(11),
			revision: 1,
			content: format!("body-{message}"),
			attachments: vec![],
			mention_everyone: false,
			mentions_self: false,
			edited_at: None,
		}
	}

	#[test]
	fn persistence_survives_reopen_without_accepting_stale_session_revisions() {
		let path = db_path("reopen");
		let mut store = local_store::LocalStore::open(&path).unwrap();
		store
			.save_history_settings(
				Id(7),
				&HistorySettings {
					enabled: true,
					..Default::default()
				},
			)
			.unwrap();
		apply_mutations(&mut store, Id(7), &[baseline(100, 1000)]).unwrap();
		apply_mutations(
			&mut store,
			Id(7),
			&[Mutation::Edit {
				account: Id(7),
				epoch: 1,
				observed_at_ms: 2000,
				channel: Id(10),
				message: Id(100),
				revision: 1,
				before: Some("body-100".into()),
				after: "accepted edit".into(),
			}],
		)
		.unwrap();
		apply_mutations(
			&mut store,
			Id(7),
			&[Mutation::Delete {
				account: Id(7),
				epoch: 1,
				observed_at_ms: 3000,
				channel: Id(10),
				message: Id(100),
				revision: 1,
			}],
		)
		.unwrap();
		drop(store);
		let store = local_store::LocalStore::open(&path).unwrap();
		let page = store.history_page(Id(7), None, None, 10).unwrap();
		assert_eq!(page.messages.len(), 1);
		assert!(page.messages[0].deleted);
		assert_eq!(page.messages[0].content, "accepted edit");
		assert_eq!(page.messages[0].changes.len(), 2);
		assert!(
			store
				.history_page(Id(8), None, None, 10)
				.unwrap()
				.messages
				.is_empty()
		);
		cleanup(&path);
	}

	#[test]
	fn export_streams_past_one_page_and_includes_old_versions() {
		let path = db_path("export-db");
		let mut store = local_store::LocalStore::open(&path).unwrap();
		store
			.save_history_settings(
				Id(7),
				&HistorySettings {
					enabled: true,
					..Default::default()
				},
			)
			.unwrap();
		let rows: Vec<Mutation> = (1..=501)
			.map(|message| baseline(message, message as i64))
			.collect();
		apply_mutations(&mut store, Id(7), &rows).unwrap();
		apply_mutations(
			&mut store,
			Id(7),
			&[Mutation::Edit {
				account: Id(7),
				epoch: 1,
				observed_at_ms: 1000,
				channel: Id(10),
				message: Id(501),
				revision: 1,
				before: Some("body-501".into()),
				after: "edited-501".into(),
			}],
		)
		.unwrap();
		let destination = db_path("export-json").with_extension("json");
		let summary = export(
			&store,
			Id(7),
			ExportRequest {
				destination: destination.clone(),
				format: ui::message_history::ExportFormat::Json,
				cancelled: std::sync::Arc::new(AtomicBool::new(false)),
			},
		)
		.unwrap();
		assert_eq!(summary.rows, 501);
		let text = std::fs::read_to_string(&destination).unwrap();
		let value: serde_json::Value = serde_json::from_str(&text).unwrap();
		assert_eq!(value["account"].as_u64(), Some(7));
		assert_eq!(value["rows"].as_array().unwrap().len(), 501);
		assert!(text.contains("body-501"));
		assert!(text.contains("edited-501"));
		assert!(!text.contains("https://"));
		assert!(!text.contains(path.to_string_lossy().as_ref()));
		cleanup(&path);
		let _ = std::fs::remove_file(destination);
	}

	#[test]
	fn export_read_failure_never_publishes_a_partial_destination() {
		let path = db_path("export-read-failure");
		let mut store = local_store::LocalStore::open(&path).unwrap();
		store
			.save_history_settings(
				Id(7),
				&HistorySettings {
					enabled: true,
					..Default::default()
				},
			)
			.unwrap();
		let rows: Vec<Mutation> = (1..=501)
			.map(|message| baseline(message, message as i64))
			.collect();
		apply_mutations(&mut store, Id(7), &rows).unwrap();
		// The first 500-row page is valid. Corrupt only the second page with a
		// type SQLite accepts when checks are explicitly disabled for this fixture.
		let connection = rusqlite::Connection::open(&path).unwrap();
		connection
			.execute_batch("PRAGMA ignore_check_constraints=ON")
			.unwrap();
		connection
			.execute(
				"UPDATE history_messages SET revision='invalid'
                 WHERE account='7' AND channel='10' AND id='501'",
				[],
			)
			.unwrap();
		drop(connection);
		for (format, extension) in [
			(ui::message_history::ExportFormat::Text, "txt"),
			(ui::message_history::ExportFormat::Json, "json"),
		] {
			let destination = db_path("export-failed").with_extension(extension);
			let result = export(
				&store,
				Id(7),
				ExportRequest {
					destination: destination.clone(),
					format,
					cancelled: std::sync::Arc::new(AtomicBool::new(false)),
				},
			);
			assert!(matches!(
				result,
				Err(crate::message_history_export::ExportError::Io(message))
					if message == "Could not read the history snapshot"
			));
			assert!(!destination.exists());
		}
		cleanup(&path);
	}
}
