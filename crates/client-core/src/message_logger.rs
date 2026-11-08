//! Bounded volatile observations for the native persistent message log.
//!
//! This module deliberately does not read a `State` or any transport. The integrator observes
//! validated history/live events and calls these helpers before or while applying them. The store
//! never invents a baseline: edits and deletions for a message that was never loaded are queued
//! only as store operations (the local store applies them to an existing durable row or ignores
//! them), never as placeholder bodies.
//!
//! Volatile rows are scoped to one account + session generation. Changing either scope releases
//! RAM but does not delete durable rows. Explicit opt-out queues [`Mutation::ClearAccount`], so a
//! reconnect must never be mistaken for a consent revocation. No value here is serialized; the
//! host maps the owned [`Mutation`] queue into its native local store.

use model::{
	Attachment, Id,
	message_logger::{
		AttachmentMeta, Change, DownloadRequest, Entry, GhostAlert, GhostMode, Kind, MAX_BYTES,
		MAX_CONTENT_BYTES, MAX_EDITS_PER_MESSAGE, MAX_EVENTS, MAX_GHOST_ALERTS, MAX_GHOST_BYTES,
		MAX_IGNORED, MAX_MESSAGES, MAX_MUTATION_BYTES, MAX_MUTATIONS, MAX_TOMBSTONES, Mutation,
	},
};
use std::{
	collections::{BTreeMap, BTreeSet, VecDeque},
	mem::size_of,
	time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Optional metadata for a loaded baseline. Kept separate from [`LoadedMessage`] so existing
/// callers and extension SDKs keep their exact construction shape.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadedDetails<'a> {
	pub attachments: &'a [Attachment],
	pub mention_everyone: bool,
	pub mentions_self: bool,
	pub edited_at: Option<i128>,
}

/// A message body that the host has just loaded from a page, a live create or a full update.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadedMessage<'a> {
	pub channel: Id,
	pub message: Id,
	pub author: Id,
	pub content: &'a str,
	/// Monotonic host ordering revision for this observation. Use one domain consistently (for
	/// example the state revision that applied the event), not a mix of different revisions.
	pub revision: u64,
}

/// A content change observed on an already loaded message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditedMessage<'a> {
	pub channel: Id,
	pub message: Id,
	pub revision: u64,
	/// Old body when the host has it. `None` falls back to the currently retained body.
	pub before: Option<&'a str>,
	/// New body. `None` is a partial update: keep the retained body and history, advance revision.
	pub after: Option<&'a str>,
}

struct Slot {
	entry: Entry,
	/// Insertion/refresh order for capacity eviction; larger is newer.
	seen: u64,
}

fn slot_bytes(slot: &Slot) -> usize {
	size_of::<Slot>() + slot.entry.bytes() + 64
}

fn unix_ms() -> i64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map_or(0, |elapsed| {
			elapsed.as_millis().min(i64::MAX as u128) as i64
		})
}

fn baseline_mutation(account: Id, epoch: u64, entry: &Entry) -> Mutation {
	Mutation::Baseline {
		account,
		epoch,
		observed_at_ms: entry.observed_at_ms,
		channel: entry.channel,
		message: entry.message,
		author: entry.author,
		revision: entry.revision,
		content: entry.content.clone(),
		attachments: entry.attachments.clone(),
		mention_everyone: entry.mention_everyone,
		mentions_self: entry.mentions_self,
		edited_at: entry.edited_at,
	}
}

fn ghost_alert(
	mode: GhostMode,
	account: Option<Id>,
	entry: &Entry,
	kind: Kind,
) -> Option<GhostAlert> {
	let enabled = match mode {
		GhostMode::Off => false,
		GhostMode::Mentions => entry.mentions_self || entry.mention_everyone,
		GhostMode::All => true,
	};
	let account = account.filter(|_| enabled)?;
	Some(GhostAlert {
		account,
		channel: entry.channel,
		message: entry.message,
		kind,
	})
}

const MAX_DOWNLOAD_URLS: usize = 256;
const MAX_DOWNLOAD_URL_BYTES: usize = 1024 * 1024;
const MAX_PENDING_DOWNLOADS: usize = 16;

#[derive(Default)]
pub struct MessageLogger {
	account: Option<Id>,
	session: u64,
	/// Invalidates queued mutations when the account/session scope changes.
	epoch: u64,
	/// The selected conversation hint used by the existing viewer. It never gates observation.
	channel: Option<Id>,
	enabled: bool,
	downloads: bool,
	entries: BTreeMap<(Id, Id), Slot>,
	/// Deleted-before-load keys, retained so a late history page cannot create a tombstoned body.
	tombstones: BTreeMap<(Id, Id), u64>,
	events: usize,
	bytes: usize,
	next_seen: u64,
	ghost_mode: GhostMode,
	ghosts: VecDeque<GhostAlert>,
	ghost_bytes: usize,
	ignored_users: BTreeSet<Id>,
	ignored_channels: BTreeSet<Id>,
	pending: VecDeque<Mutation>,
	pending_bytes: usize,
	/// Signed URLs live only in this account/session until an explicit deletion-time download.
	download_urls: BTreeMap<(Id, Id, Id), String>,
	download_url_bytes: usize,
	pending_downloads: VecDeque<DownloadRequest>,
}

impl MessageLogger {
	/// An empty, disabled store for `account` and `session`.
	pub fn new(account: Option<Id>, session: u64) -> Self {
		Self {
			account,
			session,
			..Self::default()
		}
	}

	pub fn account(&self) -> Option<Id> {
		self.account
	}

	pub fn session(&self) -> u64 {
		self.session
	}

	pub fn epoch(&self) -> u64 {
		self.epoch
	}

	/// Selection hint for the existing viewer. It never gates observation or clears retained rows.
	pub fn channel(&self) -> Option<Id> {
		self.channel
	}

	pub fn enabled(&self) -> bool {
		self.enabled
	}

	pub fn downloads_enabled(&self) -> bool {
		self.downloads
	}

	pub fn entry_count(&self) -> usize {
		self.entries.len()
	}

	pub fn event_count(&self) -> usize {
		self.events
	}

	pub fn byte_len(&self) -> usize {
		self.bytes
	}

	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}

	pub fn ghost_mode(&self) -> GhostMode {
		self.ghost_mode
	}

	pub fn ignored_users(&self) -> &BTreeSet<Id> {
		&self.ignored_users
	}

	pub fn ignored_channels(&self) -> &BTreeSet<Id> {
		&self.ignored_channels
	}

	/// Update the owning account/session. A real change releases volatile RAM and advances the
	/// mutation epoch; durable rows are untouched.
	pub fn set_scope(&mut self, account: Option<Id>, session: u64) -> bool {
		if self.account == account && self.session == session {
			return false;
		}
		self.account = account;
		self.session = session;
		self.epoch = self.epoch.wrapping_add(1);
		self.channel = None;
		self.clear();
		true
	}

	/// Update the viewer's selected channel. Unlike the old session-only store this never clears
	/// another channel's retained rows.
	pub fn set_channel(&mut self, channel: Option<Id>) -> bool {
		if self.channel == channel {
			return false;
		}
		self.channel = channel;
		true
	}

	/// Explicit session opt-in. Disabling releases volatile RAM and queues an account clear because
	/// the existing consent semantics treat opt-out as deleting the log. Reconnecting alone must
	/// not call this with `false`.
	pub fn set_enabled(&mut self, enabled: bool) -> bool {
		if self.enabled == enabled {
			return false;
		}
		self.enabled = enabled;
		if !enabled {
			self.channel = None;
			self.clear();
			if let Some(account) = self.account {
				let epoch = self.epoch;
				self.push_mutation(Mutation::ClearAccount { account, epoch });
			}
		}
		true
	}

	pub fn set_attachment_downloads(&mut self, enabled: bool) -> bool {
		if self.downloads == enabled {
			return false;
		}
		self.downloads = enabled;
		true
	}

	/// Change only the transient session grant. The durable native setting is unchanged: a Wasm
	/// extension unloading or a reconnect does not revoke persistent consent.
	pub fn set_session_enabled(&mut self, enabled: bool) -> bool {
		if self.enabled == enabled {
			return false;
		}
		self.enabled = enabled;
		if !enabled {
			self.channel = None;
			self.clear();
		}
		true
	}

	/// Explicit owner opt-out: release volatile RAM and delete this account's durable log.
	pub fn revoke_persistent(&mut self) -> bool {
		self.enabled = false;
		self.channel = None;
		self.clear();
		if let Some(account) = self.account {
			let epoch = self.epoch;
			self.push_mutation(Mutation::ClearAccount { account, epoch });
		}
		true
	}

	pub fn set_ghost_mode(&mut self, mode: GhostMode) -> bool {
		if self.ghost_mode == mode {
			return false;
		}
		self.ghost_mode = mode;
		true
	}

	/// Replace the ignored-user filter. Newly ignored authors are purged from volatile RAM and a
	/// durable author purge is queued. Removing a filter never restores deleted rows.
	pub fn set_ignored_users(&mut self, users: impl IntoIterator<Item = Id>) -> bool {
		let users: BTreeSet<_> = users
			.into_iter()
			.filter(|id| id.0 != 0)
			.take(MAX_IGNORED)
			.collect();
		if users == self.ignored_users {
			return false;
		}
		let added: Vec<_> = users.difference(&self.ignored_users).copied().collect();
		self.ignored_users = users;
		for author in added {
			let keys: Vec<_> = self
				.entries
				.iter()
				.filter(|(_, slot)| slot.entry.author == author)
				.map(|(key, _)| *key)
				.collect();
			for key in keys {
				self.remove_entry(key);
			}
			if let Some(account) = self.account {
				let epoch = self.epoch;
				self.push_mutation(Mutation::PurgeAuthor {
					account,
					epoch,
					author,
				});
			}
		}
		true
	}

	/// Replace the ignored-channel filter. Newly ignored channels are purged from volatile RAM and
	/// a durable channel purge is queued.
	pub fn set_ignored_channels(&mut self, channels: impl IntoIterator<Item = Id>) -> bool {
		let channels: BTreeSet<_> = channels
			.into_iter()
			.filter(|id| id.0 != 0)
			.take(MAX_IGNORED)
			.collect();
		if channels == self.ignored_channels {
			return false;
		}
		let added: Vec<_> = channels
			.difference(&self.ignored_channels)
			.copied()
			.collect();
		self.ignored_channels = channels;
		for channel in added {
			self.remove_channel_volatile(channel);
			if let Some(account) = self.account {
				let epoch = self.epoch;
				self.push_mutation(Mutation::PurgeChannel {
					account,
					epoch,
					channel,
				});
			}
		}
		true
	}

	/// Record a loaded body as the trusted baseline. A deleted row is never resurrected.
	pub fn observe_loaded(&mut self, now: Instant, loaded: LoadedMessage<'_>) -> bool {
		self.observe_loaded_with(now, loaded, LoadedDetails::default())
	}

	/// Record a loaded body plus attachment/mention/edited metadata for native persistence.
	pub fn observe_loaded_with(
		&mut self,
		_now: Instant,
		loaded: LoadedMessage<'_>,
		details: LoadedDetails<'_>,
	) -> bool {
		if !self.enabled
			|| loaded.channel.0 == 0
			|| loaded.message.0 == 0
			|| loaded.author.0 == 0
			|| loaded.content.len() > MAX_CONTENT_BYTES
			|| self.ignored_channels.contains(&loaded.channel)
			|| self.ignored_users.contains(&loaded.author)
		{
			return false;
		}
		let key = (loaded.channel, loaded.message);
		if self.tombstones.contains_key(&key) {
			return false;
		}
		let attachments = self.attachment_metadata(details.attachments);
		if self
			.entries
			.get(&key)
			.is_some_and(|slot| slot.entry.deleted || loaded.revision < slot.entry.revision)
		{
			return false;
		}
		self.remember_download_urls(loaded.channel, loaded.message, details.attachments);
		if let Some(slot) = self.entries.get_mut(&key) {
			if slot.entry.deleted || loaded.revision < slot.entry.revision {
				return false;
			}
			let old_bytes = slot_bytes(slot);
			slot.entry.author = loaded.author;
			slot.entry.content.clear();
			slot.entry.content.push_str(loaded.content);
			slot.entry.attachments = attachments;
			slot.entry.mention_everyone = details.mention_everyone;
			slot.entry.mentions_self = details.mentions_self;
			slot.entry.edited_at = details.edited_at.or(slot.entry.edited_at);
			slot.entry.revision = loaded.revision;
			slot.entry.observed_at_ms = unix_ms();
			slot.seen = self.next_seen;
			self.next_seen = self.next_seen.wrapping_add(1);
			let new_bytes = slot_bytes(slot);
			self.bytes = self
				.bytes
				.saturating_sub(old_bytes)
				.saturating_add(new_bytes);
			let baseline = match self.account {
				Some(account) => Some(baseline_mutation(account, self.epoch, &slot.entry)),
				None => None,
			};
			if let Some(mutation) = baseline {
				self.push_mutation(mutation);
			}
			self.enforce();
			return self.entries.contains_key(&key);
		}
		let slot = Slot {
			entry: Entry {
				channel: loaded.channel,
				message: loaded.message,
				author: loaded.author,
				revision: loaded.revision,
				content: loaded.content.to_owned(),
				deleted: false,
				events: Vec::new(),
				attachments,
				mention_everyone: details.mention_everyone,
				mentions_self: details.mentions_self,
				edited_at: details.edited_at,
				observed_at_ms: unix_ms(),
			},
			seen: self.take_seen(),
		};
		let baseline = match self.account {
			Some(account) => Some(baseline_mutation(account, self.epoch, &slot.entry)),
			None => None,
		};
		if let Some(mutation) = baseline {
			self.push_mutation(mutation);
		}
		self.insert_slot(key, slot);
		self.entries.contains_key(&key)
	}

	/// Record an edit of an already loaded message.
	pub fn observe_edited(&mut self, now: Instant, edited: EditedMessage<'_>) -> bool {
		self.observe_edited_at(now, edited, None)
	}

	/// Record an edit with the service `edited_at` ordering value. A missing volatile row still
	/// queues an edit mutation because the durable store may already know the message.
	pub fn observe_edited_at(
		&mut self,
		_now: Instant,
		edited: EditedMessage<'_>,
		edited_at: Option<i128>,
	) -> bool {
		if !self.enabled
			|| edited.channel.0 == 0
			|| edited.message.0 == 0
			|| self.ignored_channels.contains(&edited.channel)
		{
			return false;
		}
		if edited
			.before
			.is_some_and(|before| before.len() > MAX_CONTENT_BYTES)
			|| edited
				.after
				.is_some_and(|after| after.len() > MAX_CONTENT_BYTES)
		{
			return false;
		}
		let key = (edited.channel, edited.message);
		if self.tombstones.contains_key(&key) {
			return false;
		}
		let Some(slot) = self.entries.get_mut(&key) else {
			let Some(after) = edited.after else {
				return false;
			};
			if let Some(account) = self.account {
				let epoch = self.epoch;
				self.push_mutation(Mutation::Edit {
					account,
					epoch,
					observed_at_ms: unix_ms(),
					channel: edited.channel,
					message: edited.message,
					revision: edited.revision,
					before: edited.before.map(str::to_owned),
					after: after.to_owned(),
				});
			}
			return true;
		};
		if slot.entry.deleted
			|| edited.revision < slot.entry.revision
			|| self.ignored_users.contains(&slot.entry.author)
			|| edited_at.is_some_and(|at| slot.entry.edited_at.is_some_and(|old| at < old))
		{
			return false;
		}
		slot.seen = self.next_seen;
		self.next_seen = self.next_seen.wrapping_add(1);
		if let Some(at) = edited_at
			&& slot.entry.edited_at.is_none_or(|old| at >= old)
		{
			slot.entry.edited_at = Some(at);
		}
		let Some(after) = edited.after else {
			if edited.revision > slot.entry.revision {
				slot.entry.revision = edited.revision;
			}
			return true;
		};
		if after == slot.entry.content {
			if edited.revision > slot.entry.revision {
				slot.entry.revision = edited.revision;
			}
			return true;
		}
		let before = edited
			.before
			.map(str::to_owned)
			.unwrap_or_else(|| slot.entry.content.clone());
		if before == after {
			if edited.revision > slot.entry.revision {
				slot.entry.revision = edited.revision;
			}
			return true;
		}
		let old_bytes = slot_bytes(&*slot);
		let old_events = slot.entry.events.len();
		if old_events >= MAX_EDITS_PER_MESSAGE {
			slot.entry.events.remove(0);
		}
		slot.entry.events.push(Change {
			kind: Kind::Edit,
			revision: edited.revision,
			before: Some(before.clone()),
			after: Some(after.to_owned()),
		});
		slot.entry.content.clear();
		slot.entry.content.push_str(after);
		slot.entry.revision = edited.revision;
		slot.entry.observed_at_ms = unix_ms();
		let new_bytes = slot_bytes(&*slot);
		let new_events = slot.entry.events.len();
		self.bytes = self
			.bytes
			.saturating_sub(old_bytes)
			.saturating_add(new_bytes);
		self.events = self
			.events
			.saturating_sub(old_events)
			.saturating_add(new_events);
		let edit = match self.account {
			Some(account) => Some(Mutation::Edit {
				account,
				epoch: self.epoch,
				observed_at_ms: slot.entry.observed_at_ms,
				channel: edited.channel,
				message: edited.message,
				revision: edited.revision,
				before: Some(before),
				after: after.to_owned(),
			}),
			None => None,
		};
		let ghost = ghost_alert(self.ghost_mode, self.account, &slot.entry, Kind::Edit);
		if let Some(mutation) = edit {
			self.push_mutation(mutation);
		}
		self.push_ghost(ghost);
		self.enforce();
		self.entries.contains_key(&key)
	}

	/// Record a deletion. A known row keeps its last body and is tombstoned. An unknown key only
	/// queues a durable tombstone: it never creates a body and never emits an alert.
	pub fn observe_deleted(
		&mut self,
		_now: Instant,
		channel: Id,
		message: Id,
		revision: u64,
	) -> bool {
		if !self.enabled
			|| channel.0 == 0
			|| message.0 == 0
			|| self.ignored_channels.contains(&channel)
		{
			return false;
		}
		let key = (channel, message);
		if self.entries.get(&key).is_some_and(|slot| {
			slot.entry.deleted
				|| revision < slot.entry.revision
				|| self.ignored_users.contains(&slot.entry.author)
		}) {
			return false;
		}
		self.queue_deletion_downloads(channel, message);
		if let Some(slot) = self.entries.get_mut(&key) {
			if slot.entry.deleted
				|| revision < slot.entry.revision
				|| self.ignored_users.contains(&slot.entry.author)
			{
				return false;
			}
			let old_bytes = slot_bytes(&*slot);
			let old_events = slot.entry.events.len();
			if old_events >= MAX_EDITS_PER_MESSAGE {
				slot.entry.events.remove(0);
			}
			slot.entry.events.push(Change {
				kind: Kind::Delete,
				revision,
				before: Some(slot.entry.content.clone()),
				after: None,
			});
			slot.entry.deleted = true;
			slot.entry.revision = revision;
			slot.entry.observed_at_ms = unix_ms();
			for attachment in &mut slot.entry.attachments {
				attachment.deleted = true;
			}
			slot.seen = self.next_seen;
			self.next_seen = self.next_seen.wrapping_add(1);
			let new_bytes = slot_bytes(&*slot);
			let new_events = slot.entry.events.len();
			self.bytes = self
				.bytes
				.saturating_sub(old_bytes)
				.saturating_add(new_bytes);
			self.events = self
				.events
				.saturating_sub(old_events)
				.saturating_add(new_events);
			let deletion = match self.account {
				Some(account) => Some(Mutation::Delete {
					account,
					epoch: self.epoch,
					observed_at_ms: slot.entry.observed_at_ms,
					channel,
					message,
					revision,
				}),
				None => None,
			};
			let ghost = ghost_alert(self.ghost_mode, self.account, &slot.entry, Kind::Delete);
			if let Some(mutation) = deletion {
				self.push_mutation(mutation);
			}
			self.push_ghost(ghost);
			self.remember_tombstone(key, revision);
			self.enforce();
			return self.entries.contains_key(&key);
		}
		if self
			.tombstones
			.get(&key)
			.is_some_and(|known| *known >= revision)
		{
			return false;
		}
		self.remember_tombstone(key, revision);
		if let Some(account) = self.account {
			let epoch = self.epoch;
			self.push_mutation(Mutation::Delete {
				account,
				epoch,
				observed_at_ms: unix_ms(),
				channel,
				message,
				revision,
			});
		}
		true
	}

	/// Drop one volatile message immediately. Timeline eviction deliberately does not call this.
	pub fn forget(&mut self, channel: Id, message: Id) -> bool {
		self.remove_entry((channel, message))
	}

	/// Revoke one channel after access loss. Volatile rows are released and a durable purge is
	/// queued; other channels are untouched.
	pub fn revoke_channel(&mut self, channel: Id) -> usize {
		let removed = self.remove_channel_volatile(channel);
		if self.account.is_some_and(|_| channel.0 != 0) {
			let account = self.account.unwrap();
			let epoch = self.epoch;
			self.push_mutation(Mutation::PurgeChannel {
				account,
				epoch,
				channel,
			});
		}
		removed
	}

	/// Release volatile rows for one channel without touching durable rows.
	pub fn forget_channel(&mut self, channel: Id) -> usize {
		self.remove_channel_volatile(channel)
	}

	/// Every channel currently held in volatile memory.
	pub fn retained_channels(&self) -> BTreeSet<Id> {
		self.entries.keys().map(|(channel, _)| *channel).collect()
	}

	/// Evict observations cancelled by a newer host revision.
	pub fn cancel_through(&mut self, revision: u64) -> usize {
		let mut cancelled = 0;
		let mut remove = Vec::new();
		for (key, slot) in &mut self.entries {
			if slot.entry.revision <= revision {
				cancelled += slot.entry.events.len();
				remove.push(*key);
				continue;
			}
			let before = slot.entry.events.len();
			slot.entry
				.events
				.retain(|change| change.revision > revision);
			cancelled += before.saturating_sub(slot.entry.events.len());
		}
		for key in remove {
			self.remove_entry(key);
		}
		self.recount();
		cancelled
	}

	pub fn get(&self, channel: Id, message: Id) -> Option<&Entry> {
		self.entries
			.get(&(channel, message))
			.map(|slot| &slot.entry)
	}

	pub fn entries(&self) -> impl Iterator<Item = &Entry> {
		self.entries.values().map(|slot| &slot.entry)
	}

	pub fn channel_entries(&self, channel: Id) -> impl Iterator<Item = &Entry> {
		self.entries
			.values()
			.map(|slot| &slot.entry)
			.filter(move |entry| entry.channel == channel)
	}

	pub fn ghost_alerts(&self) -> impl Iterator<Item = &GhostAlert> {
		self.ghosts.iter()
	}

	pub fn pending_mutation_bytes(&self) -> usize {
		self.pending_bytes
	}

	pub fn pending_mutations(&self) -> impl Iterator<Item = &Mutation> {
		self.pending.iter()
	}

	/// Drain queued native mutations for the parent's local-store worker. The returned values are
	/// owned and bounded by [`MAX_MUTATIONS`] / [`MAX_MUTATION_BYTES`].
	pub fn drain_mutations(&mut self) -> Vec<Mutation> {
		self.pending_bytes = 0;
		self.pending.drain(..).collect()
	}

	/// Drain queued ghost alerts. They contain no message bodies.
	pub fn drain_ghost_alerts(&mut self) -> Vec<GhostAlert> {
		self.ghost_bytes = 0;
		self.ghosts.drain(..).collect()
	}

	/// Drain explicitly opted-in deletion-time work. URLs never enter the persistent queue.
	pub fn drain_download_requests(&mut self) -> Vec<DownloadRequest> {
		self.pending_downloads.drain(..).collect()
	}

	/// Build a transient download request from retained metadata. The URL is supplied by the live
	/// message and is never added to a persistence mutation.
	pub fn download_request(
		&self,
		channel: Id,
		message: Id,
		attachment: Id,
		url: &str,
	) -> Option<DownloadRequest> {
		if !self.downloads {
			return None;
		}
		self.get(channel, message)?
			.attachments
			.iter()
			.find(|meta| meta.id == attachment)?
			.download_request(channel, message, url)
	}

	pub fn clear(&mut self) {
		self.entries.clear();
		self.tombstones.clear();
		self.ghosts.clear();
		self.events = 0;
		self.bytes = 0;
		self.ghost_bytes = 0;
		self.next_seen = 0;
		self.download_urls.clear();
		self.download_url_bytes = 0;
		self.pending_downloads.clear();
	}

	fn remember_download_urls(&mut self, channel: Id, message: Id, attachments: &[Attachment]) {
		self.download_urls
			.retain(|(entry_channel, entry_message, attachment), url| {
				if *entry_channel != channel || *entry_message != message {
					return true;
				}
				let keep = attachments
					.iter()
					.any(|candidate| candidate.id == *attachment);
				if !keep {
					self.download_url_bytes = self.download_url_bytes.saturating_sub(url.len());
				}
				keep
			});
		for attachment in attachments
			.iter()
			.take(model::message_logger::MAX_ATTACHMENT_META)
		{
			let Some(url) = attachment
				.media
				.url
				.as_deref()
				.filter(|url| url.len() <= 4096)
			else {
				continue;
			};
			let key = (channel, message, attachment.id);
			if self.download_urls.contains_key(&key) {
				continue;
			}
			let Some(meta) = AttachmentMeta::from_attachment(attachment) else {
				continue;
			};
			if meta.download_request(channel, message, url).is_none() {
				continue;
			}
			if self.download_urls.len() >= MAX_DOWNLOAD_URLS
				|| self.download_url_bytes.saturating_add(url.len()) > MAX_DOWNLOAD_URL_BYTES
			{
				break;
			}
			self.download_url_bytes = self.download_url_bytes.saturating_add(url.len());
			self.download_urls.insert(key, url.to_owned());
		}
	}

	fn queue_deletion_downloads(&mut self, channel: Id, message: Id) {
		if !self.downloads {
			return;
		}
		let Some(entry) = self
			.entries
			.get(&(channel, message))
			.map(|slot| &slot.entry)
		else {
			return;
		};
		let candidates: Vec<_> = entry
			.attachments
			.iter()
			.filter_map(|attachment| {
				let url = self.download_urls.get(&(channel, message, attachment.id))?;
				attachment.download_request(channel, message, url)
			})
			.collect();
		for request in candidates {
			if self.pending_downloads.len() >= MAX_PENDING_DOWNLOADS {
				break;
			}
			self.pending_downloads.push_back(request);
		}
	}

	fn attachment_metadata(&self, attachments: &[Attachment]) -> Vec<AttachmentMeta> {
		attachments
			.iter()
			.take(model::message_logger::MAX_ATTACHMENT_META)
			.filter_map(AttachmentMeta::from_attachment)
			.collect()
	}

	fn take_seen(&mut self) -> u64 {
		let seen = self.next_seen;
		self.next_seen = self.next_seen.wrapping_add(1);
		seen
	}

	fn push_mutation(&mut self, mutation: Mutation) {
		if !mutation.valid() {
			return;
		}
		let bytes = mutation.bytes();
		self.pending.push_back(mutation);
		self.pending_bytes = self.pending_bytes.saturating_add(bytes);
		while self.pending.len() > MAX_MUTATIONS || self.pending_bytes > MAX_MUTATION_BYTES {
			let Some(old) = self.pending.pop_front() else {
				break;
			};
			self.pending_bytes = self.pending_bytes.saturating_sub(old.bytes());
		}
	}

	fn push_ghost(&mut self, alert: Option<GhostAlert>) {
		let Some(alert) = alert else {
			return;
		};
		let bytes = size_of::<GhostAlert>();
		self.ghosts.push_back(alert);
		self.ghost_bytes = self.ghost_bytes.saturating_add(bytes);
		while self.ghosts.len() > MAX_GHOST_ALERTS || self.ghost_bytes > MAX_GHOST_BYTES {
			let Some(old) = self.ghosts.pop_front() else {
				break;
			};
			self.ghost_bytes = self.ghost_bytes.saturating_sub(size_of::<GhostAlert>());
			let _ = old;
		}
	}

	fn remember_tombstone(&mut self, key: (Id, Id), revision: u64) {
		self.tombstones
			.entry(key)
			.and_modify(|known| *known = (*known).max(revision))
			.or_insert(revision);
		while self.tombstones.len() > MAX_TOMBSTONES {
			let Some(oldest) = self.tombstones.keys().next().copied() else {
				break;
			};
			self.tombstones.remove(&oldest);
		}
	}

	fn remove_entry(&mut self, key: (Id, Id)) -> bool {
		self.download_urls.retain(|(channel, message, _), url| {
			let keep = (*channel, *message) != key;
			if !keep {
				self.download_url_bytes = self.download_url_bytes.saturating_sub(url.len());
			}
			keep
		});
		self.pending_downloads
			.retain(|request| (request.channel, request.message) != key);
		let Some(slot) = self.entries.remove(&key) else {
			return false;
		};
		self.bytes = self.bytes.saturating_sub(slot_bytes(&slot));
		self.events = self.events.saturating_sub(slot.entry.events.len());
		true
	}

	fn remove_channel_volatile(&mut self, channel: Id) -> usize {
		if self.channel == Some(channel) {
			self.channel = None;
		}
		let before = self.entries.len();
		self.entries
			.retain(|&(entry_channel, _), _| entry_channel != channel);
		self.tombstones
			.retain(|&(entry_channel, _), _| entry_channel != channel);
		self.download_urls.retain(|(entry_channel, _, _), url| {
			let keep = *entry_channel != channel;
			if !keep {
				self.download_url_bytes = self.download_url_bytes.saturating_sub(url.len());
			}
			keep
		});
		self.pending_downloads
			.retain(|request| request.channel != channel);
		let original_ghosts = self.ghosts.len();
		self.ghosts.retain(|alert| alert.channel != channel);
		if self.ghosts.len() != original_ghosts {
			self.ghost_bytes = self.ghosts.len().saturating_mul(size_of::<GhostAlert>());
		}
		let removed = before.saturating_sub(self.entries.len());
		if removed != 0 {
			self.recount();
		}
		removed
	}

	fn insert_slot(&mut self, key: (Id, Id), slot: Slot) {
		let bytes = slot_bytes(&slot);
		let events = slot.entry.events.len();
		if let Some(old) = self.entries.insert(key, slot) {
			self.bytes = self.bytes.saturating_sub(slot_bytes(&old));
			self.events = self.events.saturating_sub(old.entry.events.len());
		}
		self.bytes = self.bytes.saturating_add(bytes);
		self.events = self.events.saturating_add(events);
		self.enforce();
	}

	/// Evict the least recently observed rows until every count and byte budget holds.
	fn enforce(&mut self) {
		while self.entries.len() > MAX_MESSAGES
			|| self.events > MAX_EVENTS
			|| self.bytes > MAX_BYTES
		{
			let Some(key) = self
				.entries
				.iter()
				.min_by_key(|(_, slot)| slot.seen)
				.map(|(key, _)| *key)
			else {
				break;
			};
			self.remove_entry(key);
		}
	}

	fn recount(&mut self) {
		self.events = self
			.entries
			.values()
			.map(|slot| slot.entry.events.len())
			.sum();
		self.bytes = self.entries.values().map(slot_bytes).sum();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::message_logger::MAX_ATTACHMENT_META;

	fn loaded<'a>(message: u64, content: &'a str, revision: u64) -> LoadedMessage<'a> {
		LoadedMessage {
			channel: Id(1),
			message: Id(message),
			author: Id(2),
			content,
			revision,
		}
	}

	fn edit<'a>(
		message: u64,
		revision: u64,
		before: Option<&'a str>,
		after: Option<&'a str>,
	) -> EditedMessage<'a> {
		EditedMessage {
			channel: Id(1),
			message: Id(message),
			revision,
			before,
			after,
		}
	}

	fn attachment(id: u64) -> Attachment {
		Attachment {
			id: Id(id),
			filename: format!("file-{id}.png"),
			description: None,
			content_type: Some("image/png".into()),
			size: 64,
			media: Default::default(),
			spoiler: false,
			duration_ms: None,
			waveform: Vec::new(),
		}
	}

	#[test]
	fn capacity_evicts_oldest_rows_and_background_channels_are_observed() {
		let now = Instant::now();
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		assert!(!log.enabled());
		assert!(!log.observe_loaded(now, loaded(1, "opt-in required", 1)));
		assert!(log.set_enabled(true));
		assert!(log.set_channel(Some(Id(1))));
		let mut other_channel = loaded(1, "background chat", 1);
		other_channel.channel = Id(2);
		assert!(
			log.observe_loaded(now, other_channel),
			"a readable background chat is observed without being selected"
		);
		assert!(log.get(Id(2), Id(1)).is_some());
		for id in 1..=(MAX_MESSAGES as u64 + 32) {
			assert!(log.observe_loaded(now, loaded(id, "body", 1)));
		}
		assert_eq!(log.entry_count(), MAX_MESSAGES);
		assert!(log.get(Id(1), Id(1)).is_none());
		assert!(log.byte_len() <= MAX_BYTES);
		assert!(log.event_count() <= MAX_EVENTS);
		assert!(
			log.observe_edited(now, edit(1, 2, None, Some("edited after eviction"))),
			"the durable store may still apply an edit for a RAM-evicted row"
		);
		assert!(
			log.observe_deleted(now, Id(1), Id(1), 2),
			"the durable store may still tombstone a RAM-evicted row"
		);
		assert!(log.get(Id(1), Id(1)).is_none());
	}

	#[test]
	fn edits_keep_before_and_after_and_partial_or_stale_updates_do_not_duplicate() {
		let now = Instant::now();
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		log.set_enabled(true);
		assert!(log.observe_loaded(now, loaded(1, "first", 1)));
		assert!(log.observe_edited(now, edit(1, 2, Some("first"), Some("second"))));
		let entry = log.get(Id(1), Id(1)).unwrap();
		assert_eq!(entry.content, "second");
		assert_eq!(entry.events.len(), 1);
		assert_eq!(entry.events[0].kind, Kind::Edit);
		assert!(log.observe_edited(now, edit(1, 3, None, None)));
		let entry = log.get(Id(1), Id(1)).unwrap();
		assert_eq!(entry.content, "second");
		assert_eq!(entry.events.len(), 1);
		assert_eq!(entry.revision, 3);
		assert!(!log.observe_edited(now, edit(1, 2, Some("second"), Some("stale"))));
		assert_eq!(log.get(Id(1), Id(1)).unwrap().content, "second");
		assert!(log.observe_edited(now, edit(1, 4, None, Some("third"))));
		let entry = log.get(Id(1), Id(1)).unwrap();
		assert_eq!(entry.events[1].before.as_deref(), Some("second"));
		assert_eq!(entry.events[1].after.as_deref(), Some("third"));
		for revision in 5..=20 {
			let content = format!("edit-{revision}");
			assert!(log.observe_edited(now, edit(1, revision, None, Some(content.as_str()))));
		}
		let entry = log.get(Id(1), Id(1)).unwrap();
		assert_eq!(entry.events.len(), MAX_EDITS_PER_MESSAGE);
		assert_eq!(
			entry.events.last().unwrap().after.as_deref(),
			Some("edit-20")
		);
		assert_eq!(entry.content, "edit-20");
	}

	#[test]
	fn deleted_before_load_is_tombstoned_without_body_or_alert_and_late_baseline_is_rejected() {
		let now = Instant::now();
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		log.set_enabled(true);
		log.set_ghost_mode(GhostMode::All);
		assert!(log.observe_deleted(now, Id(1), Id(7), 1));
		assert!(log.get(Id(1), Id(7)).is_none());
		assert_eq!(log.drain_ghost_alerts().len(), 0);
		assert!(!log.observe_loaded(now, loaded(7, "must not appear", 1)));
		assert!(
			log.get(Id(1), Id(7)).is_none(),
			"a delete event cannot be followed by a deleted-before-load body"
		);

		assert!(log.observe_loaded(now, loaded(8, "last body", 1)));
		assert!(log.observe_deleted(now, Id(1), Id(8), 2));
		let entry = log.get(Id(1), Id(8)).unwrap();
		assert!(entry.deleted);
		assert_eq!(entry.content, "last body");
		assert_eq!(entry.events.len(), 1);
		assert_eq!(entry.events[0].kind, Kind::Delete);
		assert!(!log.observe_edited(now, edit(8, 3, Some("last body"), Some("revived"))));
		assert!(!log.observe_loaded(now, loaded(8, "revived", 3)));
		assert_eq!(log.get(Id(1), Id(8)).unwrap().content, "last body");
	}

	#[test]
	fn account_scope_changes_bump_epoch_and_queue_mutations_with_the_owning_account() {
		let now = Instant::now();
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		log.set_enabled(true);
		log.observe_loaded(now, loaded(1, "one", 1));
		let first: Vec<_> = log.drain_mutations();
		assert!(first.iter().all(|mutation| mutation.account() == Id(9)));
		let epoch = log.epoch();

		assert!(log.set_scope(Some(Id(10)), 1));
		assert!(log.is_empty(), "account change releases volatile RAM");
		assert!(log.epoch() > epoch);
		assert!(
			log.observe_loaded(now, loaded(1, "other account", 1)),
			"the same tweet ID may be observed under another account"
		);
		let second: Vec<_> = log.drain_mutations();
		assert!(second.iter().all(|mutation| mutation.account() == Id(10)));

		assert!(log.set_scope(Some(Id(10)), 2));
		assert!(
			log.is_empty(),
			"a new session generation releases volatile RAM"
		);
		assert!(
			!log.drain_mutations()
				.iter()
				.any(|mutation| matches!(mutation, Mutation::ClearAccount { .. })),
			"scope changes never delete durable rows"
		);
	}

	#[test]
	fn disabling_opt_in_queues_a_durable_account_clear() {
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		log.set_enabled(true);
		assert!(log.set_enabled(false));
		assert!(log.is_empty());
		let mutations = log.drain_mutations();
		assert!(matches!(
			mutations.as_slice(),
			[Mutation::ClearAccount { account: Id(9), .. }]
		));
	}

	#[test]
	fn ignore_filters_purge_volatile_rows_and_queue_channel_or_author_purges() {
		let now = Instant::now();
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		log.set_enabled(true);
		log.observe_loaded(now, loaded(1, "keep", 1));
		log.observe_loaded(now, loaded(2, "ignore channel", 1));
		let mut channel_two = loaded(3, "other channel", 1);
		channel_two.channel = Id(2);
		log.observe_loaded(now, channel_two);
		log.observe_loaded(now, loaded(4, "ignored author", 1));
		log.drain_mutations();

		assert!(log.set_ignored_channels([Id(2)]));
		assert!(log.get(Id(2), Id(3)).is_none());
		assert!(
			log.pending_mutations()
				.any(|mutation| matches!(mutation, Mutation::PurgeChannel { channel: Id(2), .. }))
		);

		// Messages 1..4 all share the synthetic author Id(2).
		assert!(log.set_ignored_users([Id(2)]));
		assert!(log.is_empty(), "ignored authors are purged from RAM");
		assert!(
			log.pending_mutations()
				.any(|mutation| matches!(mutation, Mutation::PurgeAuthor { author: Id(2), .. }))
		);
		assert!(
			!log.observe_loaded(now, loaded(5, "new ignored author body", 1)),
			"filters are checked before a baseline is admitted"
		);
	}

	#[test]
	fn attachment_metadata_is_copied_and_a_delete_marks_it() {
		let now = Instant::now();
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		log.set_enabled(true);
		let attachments = [attachment(5), attachment(6)].map(|mut attachment| {
			attachment.media.url = Some(format!(
				"https://cdn.discordapp.com/attachments/1/1/{}/{}",
				attachment.id, attachment.filename
			));
			attachment
		});
		assert!(log.observe_loaded_with(
			now,
			loaded(1, "with files", 1),
			LoadedDetails {
				attachments: &attachments,
				mention_everyone: true,
				mentions_self: true,
				edited_at: Some(4),
			},
		));
		let entry = log.get(Id(1), Id(1)).unwrap();
		assert_eq!(entry.attachments.len(), 2);
		assert_eq!(entry.attachments.len(), MAX_ATTACHMENT_META.min(2));
		assert!(entry.mention_everyone);
		assert!(entry.mentions_self);
		assert_eq!(entry.edited_at, Some(4));
		assert!(log.drain_download_requests().is_empty());
		log.set_attachment_downloads(true);
		assert!(log.observe_deleted(now, Id(1), Id(1), 2));
		assert!(
			log.get(Id(1), Id(1))
				.unwrap()
				.attachments
				.iter()
				.all(|attachment| attachment.deleted)
		);
		let downloads = log.drain_download_requests();
		assert_eq!(downloads.len(), 2);
		assert!(downloads.iter().all(|request| {
			request
				.url
				.starts_with("https://cdn.discordapp.com/attachments/1/1/")
		}));
		assert!(log.drain_download_requests().is_empty());
	}

	#[test]
	fn ghost_alerts_follow_mode_caps_and_known_live_transitions_only() {
		let now = Instant::now();
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		log.set_enabled(true);
		log.observe_deleted(now, Id(1), Id(99), 1);
		assert!(log.get(Id(1), Id(99)).is_none());
		assert_eq!(
			log.drain_ghost_alerts().len(),
			0,
			"unknown deletes never alert"
		);

		log.set_ghost_mode(GhostMode::All);
		for id in 1..=(MAX_GHOST_ALERTS as u64 + 4) {
			assert!(log.observe_loaded(now, loaded(id, "body", 1)));
			assert!(log.observe_deleted(now, Id(1), Id(id), 2));
		}
		let alerts = log.drain_ghost_alerts();
		assert_eq!(alerts.len(), MAX_GHOST_ALERTS);
		assert!(alerts.iter().all(|alert| alert.channel == Id(1)));
	}

	#[test]
	fn unknown_edits_still_queue_a_durable_mutation_but_known_tombstones_reject_them() {
		let now = Instant::now();
		let mut log = MessageLogger::new(Some(Id(9)), 1);
		log.set_enabled(true);
		assert!(log.observe_edited(now, edit(7, 2, None, Some("late edit"))));
		assert!(log.get(Id(1), Id(7)).is_none());
		assert!(matches!(
			log.pending_mutations().next(),
			Some(Mutation::Edit { message: Id(7), .. })
		));
		assert!(log.observe_deleted(now, Id(1), Id(7), 3));
		assert!(!log.observe_edited(now, edit(7, 4, None, Some("after delete"))));
	}
}
