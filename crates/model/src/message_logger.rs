//! UI-neutral, bounded message observations and native persistence mutations.
//!
//! [`Entry`] deliberately does not implement `Serialize`: message bodies retained for the native
//! history viewer must not become a Wasm/offline export. A native host persists them through the
//! owned [`Mutation`] values this module defines and drops the volatile entries on account/session
//! scope changes. Persistent rows survive ordinary restarts and disconnects; only an explicit
//! account clear (opt-in revocation or logout) removes them.

use crate::{Attachment, Id};
use std::mem::size_of;

/// Largest retained version of one message body. Service message content is normally much
/// smaller; this leaves room for long unicode text without admitting a multi-megabyte field.
pub const MAX_CONTENT_BYTES: usize = 16 * 1024;
/// Old versions retained for one message. The newest versions win; the oldest edit is dropped.
pub const MAX_EDITS_PER_MESSAGE: usize = 8;
/// Volatile rows retained in RAM for one account/session. This is intentionally conservative:
/// the native store keeps the much larger durable log.
pub const MAX_MESSAGES: usize = 512;
/// Total volatile change records for one account/session.
pub const MAX_EVENTS: usize = 2048;
/// Volatile retained-allocation budget for one account/session.
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
/// Durable default row budget used by the native local-store backend.
pub const PERSISTED_MAX_MESSAGES: usize = 10_000;
/// Durable default byte budget used by the native local-store backend.
pub const PERSISTED_MAX_BYTES: usize = 32 * 1024 * 1024;
/// Attachment metadata rows copied into a durable baseline.
pub const MAX_ATTACHMENT_META: usize = 10;
pub const MAX_ATTACHMENT_FILENAME_BYTES: usize = 1024;
pub const MAX_ATTACHMENT_DESCRIPTION_BYTES: usize = 4096;
pub const MAX_ATTACHMENT_CONTENT_TYPE_BYTES: usize = 128;
pub const MAX_ATTACHMENT_WAVEFORM_BYTES: usize = 256;
/// Bounded set of deleted-before-load keys retained only to reject a late baseline.
pub const MAX_TOMBSTONES: usize = 2048;
/// Queued native mutations before the oldest observation is dropped.
pub const MAX_MUTATIONS: usize = 4096;
pub const MAX_MUTATION_BYTES: usize = 8 * 1024 * 1024;
/// Number and byte budget of navigation-only ghost alerts.
pub const MAX_GHOST_ALERTS: usize = 8;
pub const MAX_GHOST_BYTES: usize = 16 * 1024;
/// Each ignore filter is capped independently.
pub const MAX_IGNORED: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	Edit,
	Delete,
}

/// Attachment metadata retained with a message row. CDN URLs are deliberately omitted: they are
/// signed view capabilities, not durable metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentMeta {
	pub id: Id,
	pub filename: String,
	pub description: Option<String>,
	pub content_type: Option<String>,
	pub size: u64,
	pub spoiler: bool,
	pub duration_ms: Option<u32>,
	pub waveform: Vec<u8>,
	pub deleted: bool,
}

impl AttachmentMeta {
	/// Copy metadata only. The attachment's transient CDN URL is never retained here.
	pub fn from_attachment(attachment: &Attachment) -> Option<Self> {
		let meta = Self {
			id: attachment.id,
			filename: attachment.filename.clone(),
			description: attachment.description.clone(),
			content_type: attachment.content_type.clone(),
			size: attachment.size,
			spoiler: attachment.spoiler,
			duration_ms: attachment.duration_ms,
			waveform: attachment.waveform.clone(),
			deleted: false,
		};
		meta.valid().then_some(meta)
	}

	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& (1..=MAX_ATTACHMENT_FILENAME_BYTES).contains(&self.filename.len())
			&& self
				.description
				.as_ref()
				.is_none_or(|value| value.len() <= MAX_ATTACHMENT_DESCRIPTION_BYTES)
			&& self
				.content_type
				.as_ref()
				.is_none_or(|value| value.len() <= MAX_ATTACHMENT_CONTENT_TYPE_BYTES)
			&& self.waveform.len() <= MAX_ATTACHMENT_WAVEFORM_BYTES
			&& self.duration_ms.is_none_or(|value| value <= 600_000)
	}

	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.filename.capacity()
			+ self.description.as_ref().map_or(0, String::capacity)
			+ self.content_type.as_ref().map_or(0, String::capacity)
			+ self.waveform.capacity()
	}

	/// A download may start only for an exact supported CDN attachment URL. The URL is carried by
	/// the transient request and is not part of any persistence mutation.
	pub fn download_request(&self, channel: Id, message: Id, url: &str) -> Option<DownloadRequest> {
		if channel.0 == 0 || message.0 == 0 || !exact_cdn_attachment_url(url, self.id) {
			return None;
		}
		Some(DownloadRequest {
			channel,
			message,
			attachment: self.clone(),
			url: url.to_owned(),
		})
	}
}

/// Transient request for the native download worker. Never persisted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadRequest {
	pub channel: Id,
	pub message: Id,
	pub attachment: AttachmentMeta,
	pub url: String,
}

fn exact_cdn_attachment_url(url: &str, attachment: Id) -> bool {
	if !(url.starts_with("https://cdn.discordapp.com/attachments/")
		|| url.starts_with("https://media.discordapp.net/attachments/"))
	{
		return false;
	}
	let path = url.split(['?', '#']).next().unwrap_or(url);
	let mut parts = path.rsplit('/');
	let Some(_filename) = parts.next().filter(|part| !part.is_empty()) else {
		return false;
	};
	parts
		.next()
		.and_then(|part| part.parse::<u64>().ok())
		.is_some_and(|id| id == attachment.0)
}

/// One bounded volatile observation. An edit keeps the content before and after the change; a
/// delete keeps the last loaded content and no replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
	pub kind: Kind,
	/// The loaded message revision this observation belongs to. Stale observations are ignored
	/// and can be cancelled when a history replacement invalidates an older snapshot.
	pub revision: u64,
	pub before: Option<String>,
	pub after: Option<String>,
}

impl Change {
	pub fn heap_bytes(&self) -> usize {
		self.before.as_ref().map_or(0, String::capacity)
			+ self.after.as_ref().map_or(0, String::capacity)
	}

	pub fn bytes(&self) -> usize {
		size_of::<Self>() + self.heap_bytes()
	}

	pub fn valid(&self) -> bool {
		let bounded = |text: &Option<String>| {
			text.as_ref()
				.is_none_or(|text| text.len() <= MAX_CONTENT_BYTES)
		};
		bounded(&self.before)
			&& bounded(&self.after)
			&& match self.kind {
				Kind::Edit => self.before.is_some() && self.after.is_some(),
				Kind::Delete => self.before.is_some() && self.after.is_none(),
			}
	}
}

/// One retained message's bounded history. `content` is the latest trusted body, including the
/// last body retained before a deletion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
	pub channel: Id,
	pub message: Id,
	pub author: Id,
	pub revision: u64,
	pub content: String,
	pub deleted: bool,
	pub events: Vec<Change>,
	pub attachments: Vec<AttachmentMeta>,
	pub mention_everyone: bool,
	pub mentions_self: bool,
	pub edited_at: Option<i128>,
	pub observed_at_ms: i64,
}

impl Entry {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.content.capacity()
			+ self.events.capacity() * size_of::<Change>()
			+ self.events.iter().map(Change::heap_bytes).sum::<usize>()
			+ self.attachments.capacity() * size_of::<AttachmentMeta>()
			+ self
				.attachments
				.iter()
				.map(|attachment| attachment.bytes() - size_of::<AttachmentMeta>())
				.sum::<usize>()
	}

	pub fn valid(&self) -> bool {
		self.channel.0 != 0
			&& self.message.0 != 0
			&& self.author.0 != 0
			&& self.content.len() <= MAX_CONTENT_BYTES
			&& self.events.len() <= MAX_EDITS_PER_MESSAGE
			&& self.events.iter().all(Change::valid)
			&& self.attachments.len() <= MAX_ATTACHMENT_META
			&& self.attachments.iter().all(AttachmentMeta::valid)
	}
}

/// Navigation-only alert for a known live edit/delete transition. No body is included.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GhostMode {
	#[default]
	Off,
	Mentions,
	All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GhostAlert {
	pub account: Id,
	pub channel: Id,
	pub message: Id,
	pub kind: Kind,
}

/// A bounded owned mutation queued for the native local-store backend. None of these values are
/// serialized by this crate; the host decides how to persist them inside its own native boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mutation {
	Baseline {
		account: Id,
		epoch: u64,
		observed_at_ms: i64,
		channel: Id,
		message: Id,
		author: Id,
		revision: u64,
		content: String,
		attachments: Vec<AttachmentMeta>,
		mention_everyone: bool,
		mentions_self: bool,
		edited_at: Option<i128>,
	},
	Edit {
		account: Id,
		epoch: u64,
		observed_at_ms: i64,
		channel: Id,
		message: Id,
		revision: u64,
		before: Option<String>,
		after: String,
	},
	Delete {
		account: Id,
		epoch: u64,
		observed_at_ms: i64,
		channel: Id,
		message: Id,
		revision: u64,
	},
	PurgeChannel {
		account: Id,
		epoch: u64,
		channel: Id,
	},
	PurgeAuthor {
		account: Id,
		epoch: u64,
		author: Id,
	},
	ClearAccount {
		account: Id,
		epoch: u64,
	},
}

impl Mutation {
	pub fn account(&self) -> Id {
		match self {
			Self::Baseline { account, .. }
			| Self::Edit { account, .. }
			| Self::Delete { account, .. }
			| Self::PurgeChannel { account, .. }
			| Self::PurgeAuthor { account, .. }
			| Self::ClearAccount { account, .. } => *account,
		}
	}

	pub fn epoch(&self) -> u64 {
		match self {
			Self::Baseline { epoch, .. }
			| Self::Edit { epoch, .. }
			| Self::Delete { epoch, .. }
			| Self::PurgeChannel { epoch, .. }
			| Self::PurgeAuthor { epoch, .. }
			| Self::ClearAccount { epoch, .. } => *epoch,
		}
	}

	pub fn valid(&self) -> bool {
		match self {
			Self::Baseline {
				account,
				channel,
				message,
				author,
				content,
				attachments,
				..
			} => {
				account.0 != 0
					&& channel.0 != 0
					&& message.0 != 0
					&& author.0 != 0
					&& content.len() <= MAX_CONTENT_BYTES
					&& attachments.len() <= MAX_ATTACHMENT_META
					&& attachments.iter().all(AttachmentMeta::valid)
			}
			Self::Edit {
				account,
				channel,
				message,
				before,
				after,
				..
			} => {
				account.0 != 0
					&& channel.0 != 0
					&& message.0 != 0
					&& before
						.as_ref()
						.is_none_or(|value| value.len() <= MAX_CONTENT_BYTES)
					&& after.len() <= MAX_CONTENT_BYTES
			}
			Self::Delete {
				account,
				channel,
				message,
				..
			} => account.0 != 0 && channel.0 != 0 && message.0 != 0,
			Self::PurgeChannel {
				account, channel, ..
			} => account.0 != 0 && channel.0 != 0,
			Self::PurgeAuthor {
				account, author, ..
			} => account.0 != 0 && author.0 != 0,
			Self::ClearAccount { account, .. } => account.0 != 0,
		}
	}

	pub fn bytes(&self) -> usize {
		let string = |value: &Option<String>| value.as_ref().map_or(0, String::capacity);
		size_of::<Self>()
			+ match self {
				Self::Baseline {
					content,
					attachments,
					..
				} => {
					content.capacity()
						+ attachments.capacity() * size_of::<AttachmentMeta>()
						+ attachments
							.iter()
							.map(|attachment| attachment.bytes() - size_of::<AttachmentMeta>())
							.sum::<usize>()
				}
				Self::Edit { before, after, .. } => string(before) + after.capacity(),
				_ => 0,
			}
	}
}

/// Native persistence defaults. The local-store backend owns retention; these values are shared
/// so the UI and store agree on the advertised bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistentPolicy {
	pub max_messages: usize,
	pub max_bytes: usize,
}

impl Default for PersistentPolicy {
	fn default() -> Self {
		Self {
			max_messages: PERSISTED_MAX_MESSAGES,
			max_bytes: PERSISTED_MAX_BYTES,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn attachment(id: u64) -> Attachment {
		Attachment {
			id: Id(id),
			filename: "image.png".into(),
			description: None,
			content_type: Some("image/png".into()),
			size: 128,
			media: Default::default(),
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		}
	}

	#[test]
	fn change_and_entry_validation_reject_unbounded_or_incoherent_rows() {
		let edit = Change {
			kind: Kind::Edit,
			revision: 1,
			before: Some("before".into()),
			after: Some("after".into()),
		};
		assert!(edit.valid());
		assert!(edit.bytes() >= size_of::<Change>() + "before".len() + "after".len());
		assert!(
			!Change {
				kind: Kind::Edit,
				revision: 1,
				before: None,
				after: Some("after".into()),
			}
			.valid()
		);
		assert!(
			!Change {
				kind: Kind::Delete,
				revision: 1,
				before: Some("before".into()),
				after: Some("after".into()),
			}
			.valid()
		);
		let mut entry = Entry {
			channel: Id(1),
			message: Id(2),
			author: Id(3),
			revision: 1,
			content: "body".into(),
			deleted: false,
			events: vec![edit],
			attachments: vec![AttachmentMeta::from_attachment(&attachment(4)).unwrap()],
			mention_everyone: false,
			mentions_self: true,
			edited_at: None,
			observed_at_ms: 1,
		};
		assert!(entry.valid());
		entry.events = vec![
			Change {
				kind: Kind::Edit,
				revision: 1,
				before: Some("a".into()),
				after: Some("b".into()),
			};
			MAX_EDITS_PER_MESSAGE + 1
		];
		assert!(!entry.valid());
		entry.events.truncate(1);
		entry.content = "x".repeat(MAX_CONTENT_BYTES + 1);
		assert!(!entry.valid());
		entry.content.clear();
		entry.author = Id(0);
		assert!(!entry.valid());
	}

	#[test]
	fn attachment_metadata_omits_cdn_url_and_download_requires_exact_attachment() {
		let mut source = attachment(7);
		source.media.url = Some("https://cdn.discordapp.com/attachments/1/7/image.png".into());
		let meta = AttachmentMeta::from_attachment(&source).unwrap();
		assert_eq!(meta.id, Id(7));
		assert_eq!(meta.filename, "image.png");
		assert!(
			meta.download_request(Id(1), Id(2), source.media.url.as_deref().unwrap())
				.is_some()
		);
		assert!(
			meta.download_request(Id(1), Id(2), "https://example.invalid/7/image.png")
				.is_none(),
			"only exact supported CDN attachment URLs may start a download"
		);
		assert!(
			meta.download_request(
				Id(1),
				Id(2),
				"https://cdn.discordapp.com/attachments/1/8/image.png"
			)
			.is_none(),
			"the URL must name this attachment"
		);
	}
}
