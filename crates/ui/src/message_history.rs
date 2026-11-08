//! Native retained message-history browser.
//!
//! This module owns only a bounded viewer cache and the explicit requests the host must service.
//! It never reads a local database, never writes files and never serializes a retained body, so
//! nothing here can leak message content into the Wasm/extension boundary. The native desktop
//! layer feeds owned [`Entry`] rows to [`HistoryBrowser::accept_page`] and handles
//! [`Request`] values from [`HistoryBrowser::take_requests`].

use model::{
	Id,
	message_logger::{Entry, GhostMode, Kind},
};
use std::collections::{BTreeMap, VecDeque};

/// Rows requested for one page. A page result is also truncated to this many rows.
pub const PAGE_ROWS: usize = 100;
/// Rows the viewer page cache may retain. This is deliberately much smaller than the durable log.
pub const MAX_CACHE_ROWS: usize = 500;
/// Bytes the viewer page cache may retain. Eviction here does not delete durable rows.
pub const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;
/// Longest filter input accepted for a channel or author decimal ID.
pub const MAX_FILTER_CHARS: usize = 32;
/// Longest host-injected operation/status text shown by the viewer.
pub const MAX_STATUS_CHARS: usize = 240;
/// Pending host requests retained before the oldest is dropped.
pub const MAX_REQUESTS: usize = 32;

/// Which retained rows the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
	#[default]
	All,
	Edited,
	Deleted,
}

impl Filter {
	pub fn label(self) -> &'static str {
		match self {
			Self::All => "message-history-filter-all",
			Self::Edited => "message-history-filter-edited",
			Self::Deleted => "message-history-filter-deleted",
		}
	}
}

/// Scope of an explicit clear request. The viewer only asks the host; it never deletes a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearScope {
	Channel(Id),
	Account,
}

/// Format of an explicit export request. No file IO happens in this crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
	Text,
	Json,
}

/// Stable cursor for one cached row. The host maps it to its durable row ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PageKey {
	pub channel: Id,
	pub message: Id,
}

impl PageKey {
	pub fn new(channel: Id, message: Id) -> Self {
		Self { channel, message }
	}
}

impl From<&Entry> for PageKey {
	fn from(entry: &Entry) -> Self {
		Self::new(entry.channel, entry.message)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageDirection {
	Older,
	Newer,
}

/// One owned page from the native store. Entries are expected newest first; only the first
/// [`PAGE_ROWS`] rows are accepted so a malformed/oversized page cannot fill the viewer at once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryPage {
	pub generation: u64,
	pub account: Option<Id>,
	/// Set for a channel-scoped page, or `None` for an all-chat page.
	pub channel: Option<Id>,
	/// Set for a one-message context page.
	pub message: Option<Id>,
	pub entries: Vec<Entry>,
	/// Whether the host has rows older than this page.
	pub has_older: bool,
	/// Whether the host has rows newer than this page.
	pub has_newer: bool,
}

impl HistoryPage {
	pub fn new(generation: u64, account: Option<Id>, entries: Vec<Entry>) -> Self {
		Self {
			generation,
			account,
			entries,
			..Self::default()
		}
	}
}

pub const MAX_PREFERENCE_LIST_CHARS: usize = 11_000;

/// Editable native history policy. String lists are parsed and bounded by the desktop, never
/// written into plugin storage or evaluated by the UI as trusted IDs without host validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
	pub enabled: bool,
	pub retention_days: u32,
	pub max_messages: u32,
	pub ghost: GhostMode,
	pub download_images: bool,
	pub download_audio: bool,
	pub download_video: bool,
	pub ignore_users: String,
	pub ignore_channels: String,
}

impl Default for Preferences {
	fn default() -> Self {
		Self {
			enabled: false,
			retention_days: 30,
			max_messages: 10_000,
			ghost: GhostMode::Off,
			download_images: false,
			download_audio: false,
			download_video: false,
			ignore_users: String::new(),
			ignore_channels: String::new(),
		}
	}
}

/// Explicit work for the native host. The UI never performs any of these itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
	/// Load one bounded page from the native retained store. `before`/`after` are viewer cursors,
	/// not Discord search cursors.
	LoadPage {
		generation: u64,
		account: Option<Id>,
		channel: Option<Id>,
		author: Option<Id>,
		message: Option<Id>,
		kind: Filter,
		before: Option<PageKey>,
		after: Option<PageKey>,
		limit: usize,
	},
	/// Apply reviewed native preferences after the host validates IDs and bounds.
	SavePreferences {
		generation: u64,
		account: Option<Id>,
		preferences: Preferences,
	},
	/// Navigate to a retained target. `deleted` tells the host to open the selected channel and
	/// this viewer instead of issuing a service message search.
	OpenTarget {
		generation: u64,
		channel: Id,
		message: Id,
		deleted: bool,
	},
	/// Apply an explicitly confirmed destructive operation to the native store.
	Clear {
		generation: u64,
		account: Option<Id>,
		scope: ClearScope,
	},
	/// Export retained rows through a native file path owned by the desktop.
	Export {
		generation: u64,
		account: Option<Id>,
		format: ExportFormat,
	},
}

/// Outcome of opening one message context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOpen {
	/// The in-memory entry was available and is shown without a host request.
	Retained,
	/// The host must supply a bounded native page. This is a local-store read, not a search.
	NeedsPage,
}

/// Bounded, host-injected progress/status. `complete` must be set only after the host finishes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationStatus {
	pub text: String,
	pub progress: Option<(u32, u32)>,
	pub complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
	All,
	Message { channel: Id, message: Id },
}

impl Scope {
	fn channel(self) -> Option<Id> {
		match self {
			Self::All => None,
			Self::Message { channel, .. } => Some(channel),
		}
	}

	fn message(self) -> Option<Id> {
		match self {
			Self::All => None,
			Self::Message { message, .. } => Some(message),
		}
	}
}

struct Cached {
	entry: Entry,
	seq: u64,
}

fn cached_bytes(entry: &Entry) -> usize {
	std::mem::size_of::<Cached>() + entry.bytes()
}

#[derive(Default)]
struct Filters {
	kind: Filter,
	channel: String,
	author: String,
}

#[derive(Clone, Copy)]
enum IdFilter {
	Any,
	Invalid,
	Id(Id),
}

impl IdFilter {
	fn matches(self, id: Id) -> bool {
		match self {
			Self::Any => true,
			Self::Invalid => false,
			Self::Id(expected) => expected == id,
		}
	}
}

fn parse_id_filter(value: &str) -> IdFilter {
	let value = value.trim();
	if value.is_empty() {
		return IdFilter::Any;
	}
	value
		.parse::<u64>()
		.ok()
		.filter(|id| *id != 0)
		.map_or(IdFilter::Invalid, |id| IdFilter::Id(Id(id)))
}

fn bounded_text(value: impl Into<String>) -> String {
	let value = value.into();
	let mut chars = value.chars();
	let text: String = chars.by_ref().take(MAX_STATUS_CHARS).collect();
	if chars.next().is_some() {
		format!("{text}…")
	} else {
		text
	}
}

fn preview(value: &str, max_chars: usize) -> String {
	let mut chars = value.chars();
	let mut text: String = chars.by_ref().take(max_chars).collect();
	if chars.next().is_some() {
		text.push('…');
	}
	text
}

fn full_timestamp(nanos: i128) -> Option<String> {
	let utc = time::OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?;
	Some(crate::local_time::full(utc))
}

/// Owned viewer state and bounded page cache. It is UI-only and does not depend on `local-store`.
#[derive(Default)]
pub struct HistoryBrowser {
	generation: u64,
	account: Option<Id>,
	open: bool,
	scope: Option<Scope>,
	selected: Option<PageKey>,
	filters: Filters,
	cache: BTreeMap<PageKey, Cached>,
	cache_bytes: usize,
	next_seq: u64,
	requests: VecDeque<Request>,
	loading: bool,
	error: Option<String>,
	has_older: bool,
	has_newer: bool,
	clear_confirm: Option<ClearScope>,
	status: Option<OperationStatus>,
	preferences: Preferences,
	capture_controls: bool,
}

impl HistoryBrowser {
	/// The account/generation fence currently held by the viewer.
	pub fn generation(&self) -> u64 {
		self.generation
	}

	pub fn account(&self) -> Option<Id> {
		self.account
	}

	pub fn is_open(&self) -> bool {
		self.open
	}

	pub fn set_capture_controls(&mut self, generation: u64, account: Option<Id>, allowed: bool) {
		if generation == self.generation && account == self.account {
			self.capture_controls = allowed;
		}
	}

	pub fn set_preferences(&mut self, generation: u64, account: Option<Id>, values: Preferences) {
		if generation == self.generation && account == self.account {
			self.preferences = values;
		}
	}

	pub fn preferences(&self) -> &Preferences {
		&self.preferences
	}

	/// Change the account/session fence. A different scope releases only the viewer cache; the
	/// durable native log is untouched. Returns whether the fence was already current.
	pub fn sync_scope(&mut self, generation: u64, account: Option<Id>) -> bool {
		if self.generation == generation && self.account == account {
			return true;
		}
		self.generation = generation;
		self.account = account;
		self.open = false;
		self.scope = None;
		self.selected = None;
		self.filters = Filters::default();
		self.cache.clear();
		self.cache_bytes = 0;
		self.next_seq = 0;
		self.requests.clear();
		self.loading = false;
		self.error = None;
		self.has_older = false;
		self.has_newer = false;
		self.clear_confirm = None;
		self.status = None;
		self.preferences = Preferences::default();
		self.capture_controls = false;
		false
	}

	/// Close the viewer without clearing its bounded cache.
	pub fn close(&mut self) {
		self.open = false;
		self.clear_confirm = None;
	}

	/// Rows currently held in the viewer page cache.
	pub fn row_count(&self) -> usize {
		self.cache.len()
	}

	/// Bytes currently held in the viewer page cache.
	pub fn byte_len(&self) -> usize {
		self.cache_bytes
	}

	pub fn has_older(&self) -> bool {
		self.has_older
	}

	pub fn has_newer(&self) -> bool {
		self.has_newer
	}

	pub fn focused_target(&self) -> Option<(Id, Id)> {
		self.selected.map(|key| (key.channel, key.message))
	}

	pub fn focused_entry(&self) -> Option<&Entry> {
		self.selected
			.and_then(|key| self.cache.get(&key))
			.map(|cached| &cached.entry)
	}

	/// Take one queued host request.
	pub fn take_request(&mut self) -> Option<Request> {
		self.requests.pop_front()
	}

	/// Take every queued host request in submission order.
	pub fn take_requests(&mut self) -> Vec<Request> {
		self.requests.drain(..).collect()
	}

	/// Open the all-chat view and ask the host for the first bounded page.
	pub fn open_all(&mut self, generation: u64, account: Option<Id>) {
		self.ensure_scope(generation, account);
		if self.scope != Some(Scope::All) || !self.open {
			self.clear_page_cache();
		}
		self.open = true;
		self.scope = Some(Scope::All);
		self.selected = None;
		self.filters = Filters::default();
		self.loading = true;
		self.error = None;
		self.has_older = true;
		self.has_newer = false;
		self.status = None;
		self.clear_confirm = None;
		self.queue(Request::LoadPage {
			generation: self.generation,
			account: self.account,
			channel: None,
			author: None,
			message: None,
			kind: Filter::All,
			before: None,
			after: None,
			limit: PAGE_ROWS,
		});
	}

	/// Open one message context. When the in-memory entry is available no host request is made;
	/// otherwise this asks for a local native page, never a service search.
	pub fn open_target(
		&mut self,
		generation: u64,
		account: Option<Id>,
		channel: Id,
		message: Id,
		retained: Option<Entry>,
	) -> TargetOpen {
		self.ensure_scope(generation, account);
		let scope = Scope::Message { channel, message };
		if self.scope != Some(scope) || !self.open {
			self.clear_page_cache();
		}
		self.open = true;
		self.scope = Some(scope);
		self.selected = Some(PageKey::new(channel, message));
		self.filters = Filters::default();
		self.loading = false;
		self.error = None;
		self.has_older = false;
		self.has_newer = false;
		self.status = None;
		self.clear_confirm = None;
		if let Some(entry) = retained
			.filter(|entry| entry.valid() && entry.channel == channel && entry.message == message)
		{
			self.insert(entry);
			return TargetOpen::Retained;
		}
		if self.cache.contains_key(&PageKey::new(channel, message)) {
			return TargetOpen::Retained;
		}
		self.loading = true;
		self.queue(Request::LoadPage {
			generation: self.generation,
			account: self.account,
			channel: Some(channel),
			author: None,
			message: Some(message),
			kind: Filter::All,
			before: None,
			after: None,
			limit: PAGE_ROWS,
		});
		TargetOpen::NeedsPage
	}

	/// Accept an asynchronously loaded page. Wrong generation/account values are rejected, and
	/// only [`PAGE_ROWS`] rows can enter the viewer from one call.
	pub fn accept_page(&mut self, mut page: HistoryPage) -> bool {
		if page.generation != self.generation || page.account != self.account {
			return false;
		}
		if let Some(scope) = self.scope {
			if scope.channel().is_some()
				&& page.channel.is_some_and(|id| Some(id) != scope.channel())
			{
				return false;
			}
			if scope.message().is_some()
				&& page.message.is_some_and(|id| Some(id) != scope.message())
			{
				return false;
			}
		}
		let overflow = page.entries.len() > PAGE_ROWS;
		page.entries.truncate(PAGE_ROWS);
		self.loading = false;
		self.error = None;
		self.has_older = page.has_older || overflow;
		self.has_newer = page.has_newer;
		let mut accepted = false;
		for entry in page.entries {
			if !entry.valid() {
				continue;
			}
			if let Some(scope) = self.scope {
				if let Some(channel) = scope.channel()
					&& entry.channel != channel
				{
					continue;
				}
				if let Some(message) = scope.message()
					&& entry.message != message
				{
					continue;
				}
			}
			self.insert(entry);
			accepted = true;
		}
		accepted
	}

	/// Report a page failure. The viewer keeps any already-cached rows.
	pub fn page_failed(&mut self, generation: u64, message: impl Into<String>) {
		if generation != self.generation {
			return;
		}
		self.loading = false;
		self.error = Some(bounded_text(message));
	}

	/// Inject bounded native operation progress. Never set `complete` before the host finishes.
	pub fn set_status(
		&mut self,
		generation: u64,
		text: impl Into<String>,
		progress: Option<(u32, u32)>,
		complete: bool,
	) {
		if generation != self.generation {
			return;
		}
		let progress = progress.map(|(done, total)| (done.min(total), total));
		self.status = Some(OperationStatus {
			text: bounded_text(text),
			progress,
			complete,
		});
	}

	pub fn clear_status(&mut self) {
		self.status = None;
	}

	/// Step one of a clear: show the confirmation. No request is queued yet.
	pub fn begin_clear(&mut self, scope: ClearScope) {
		self.clear_confirm = Some(scope);
	}

	/// Step two of a clear. Only a confirmed choice queues the host request.
	pub fn confirm_clear(&mut self, confirmed: bool) {
		let Some(scope) = self.clear_confirm.take() else {
			return;
		};
		if !confirmed {
			return;
		}
		self.queue(Request::Clear {
			generation: self.generation,
			account: self.account,
			scope,
		});
		self.status = Some(OperationStatus {
			text: crate::i18n::translate("message-history-status-clear-requested"),
			progress: None,
			complete: false,
		});
	}

	pub fn clear_confirmation(&self) -> Option<ClearScope> {
		self.clear_confirm
	}

	/// Queue an export request after the caller displayed the deleted-body/old-version warning.
	/// No file is opened here and success is never assumed.
	pub fn request_export(&mut self, format: ExportFormat) {
		self.queue(Request::Export {
			generation: self.generation,
			account: self.account,
			format,
		});
		self.status = Some(OperationStatus {
			text: crate::i18n::translate("message-history-status-export-requested"),
			progress: None,
			complete: false,
		});
	}

	/// Request the selected row's target. A deleted row carries `deleted: true` so the host opens
	/// its channel and retained viewer instead of running a service search.
	pub fn request_target(&mut self, key: PageKey) -> bool {
		let Some(cached) = self.cache.get(&key) else {
			return false;
		};
		let deleted = cached.entry.deleted;
		self.selected = Some(key);
		self.queue(Request::OpenTarget {
			generation: self.generation,
			channel: key.channel,
			message: key.message,
			deleted,
		});
		if !deleted {
			self.open = false;
		}
		true
	}

	/// Request one explicit page relative to the cached window.
	pub fn request_page(&mut self, direction: PageDirection) {
		if self.loading {
			return;
		}
		let channel = self.scope.and_then(Scope::channel).or_else(|| {
			match parse_id_filter(&self.filters.channel) {
				IdFilter::Id(id) => Some(id),
				_ => None,
			}
		});
		let author = match parse_id_filter(&self.filters.author) {
			IdFilter::Id(id) => Some(id),
			_ => None,
		};
		let message = self.scope.and_then(Scope::message);
		let (before, after) = match direction {
			PageDirection::Older => (self.page_cursor(true), None),
			PageDirection::Newer => (None, self.page_cursor(false)),
		};
		self.loading = true;
		self.error = None;
		self.queue(Request::LoadPage {
			generation: self.generation,
			account: self.account,
			channel,
			author,
			message,
			kind: self.filters.kind,
			before,
			after,
			limit: PAGE_ROWS,
		});
	}

	fn ensure_scope(&mut self, generation: u64, account: Option<Id>) {
		let _ = self.sync_scope(generation, account);
	}

	fn clear_page_cache(&mut self) {
		self.cache.clear();
		self.cache_bytes = 0;
		self.next_seq = 0;
		self.requests
			.retain(|request| !matches!(request, Request::LoadPage { .. }));
	}

	fn queue(&mut self, request: Request) {
		if self.requests.len() >= MAX_REQUESTS {
			self.requests.pop_front();
		}
		self.requests.push_back(request);
	}

	fn insert(&mut self, entry: Entry) {
		let key = PageKey::from(&entry);
		let bytes = cached_bytes(&entry);
		if bytes > MAX_CACHE_BYTES {
			return;
		}
		self.next_seq = self.next_seq.wrapping_add(1);
		if let Some(old) = self.cache.insert(
			key,
			Cached {
				entry,
				seq: self.next_seq,
			},
		) {
			self.cache_bytes = self.cache_bytes.saturating_sub(cached_bytes(&old.entry));
		}
		self.cache_bytes = self.cache_bytes.saturating_add(bytes);
		self.evict();
	}

	fn evict(&mut self) {
		while self.cache.len() > MAX_CACHE_ROWS || self.cache_bytes > MAX_CACHE_BYTES {
			let Some(key) = self
				.cache
				.iter()
				.min_by_key(|(key, cached)| (cached.seq, **key))
				.map(|(key, _)| *key)
			else {
				break;
			};
			if let Some(old) = self.cache.remove(&key) {
				self.cache_bytes = self.cache_bytes.saturating_sub(cached_bytes(&old.entry));
			}
		}
	}

	fn page_cursor(&self, oldest: bool) -> Option<PageKey> {
		let channel = match parse_id_filter(&self.filters.channel) {
			IdFilter::Id(id) => Some(id),
			IdFilter::Any => None,
			IdFilter::Invalid => return None,
		};
		let filter = |key: &&PageKey| channel.is_none_or(|id| key.channel == id);
		if oldest {
			self.cache.keys().find(filter).copied()
		} else {
			self.cache.keys().rfind(filter).copied()
		}
	}

	fn matches(&self, entry: &Entry) -> bool {
		let kind = match self.filters.kind {
			Filter::All => true,
			Filter::Edited => entry.events.iter().any(|change| change.kind == Kind::Edit),
			Filter::Deleted => entry.deleted,
		};
		kind && parse_id_filter(&self.filters.channel).matches(entry.channel)
			&& parse_id_filter(&self.filters.author).matches(entry.author)
	}

	fn visible_keys(&self) -> Vec<PageKey> {
		self.cache
			.iter()
			.rev()
			.filter(|(_, cached)| self.matches(&cached.entry))
			.map(|(key, _)| *key)
			.collect()
	}

	fn selected_channel(&self) -> Option<Id> {
		self.scope
			.and_then(Scope::channel)
			.or_else(|| match parse_id_filter(&self.filters.channel) {
				IdFilter::Id(id) => Some(id),
				_ => None,
			})
			.or_else(|| self.selected.map(|key| key.channel))
			.filter(|id| id.0 != 0)
	}

	/// Draw the native viewer. The desktop owns every request returned by
	/// [`Self::take_requests`]; this method performs no persistence or file work.
	pub fn show(&mut self, ctx: &egui::Context) {
		if !self.open {
			return;
		}
		let mut remain_open = true;
		let mut browse_all = false;
		let mut page_request = None;
		let mut export = None;
		let mut begin_clear = None;
		let mut target_request = None;
		let mut retry = false;
		egui::Window::new(crate::i18n::translate("message-history-title"))
			.id(egui::Id::unique(("message-history", self.generation)))
			.open(&mut remain_open)
			.default_width(680.0)
			.max_width((ctx.content_rect().width() - 24.0).clamp(280.0, 680.0))
			.default_height(560.0)
			.max_height((ctx.content_rect().height() - 48.0).clamp(280.0, 680.0))
			.resizable(true)
			.show(ctx, |ui| {
				egui::ScrollArea::vertical()
					.id_salt(("message-history-scroll", self.generation))
					.auto_shrink([false, false])
					.show(ui, |ui| {
						ui.label(crate::i18n::translate("message-history-notice"));
						ui.label(
							egui::RichText::new(crate::i18n::translate(
								"message-history-cache-notice",
							))
							.small(),
						);
						ui.separator();
						let mut preferences = self.preferences.clone();
						let mut save_preferences = false;
						egui::CollapsingHeader::new(crate::i18n::translate(
							"message-history-settings",
						))
						.default_open(false)
						.show(ui, |ui| {
							ui.small(crate::i18n::translate("message-history-settings-help"));
							if preferences.enabled {
								ui.label(crate::i18n::translate("message-history-capture-enabled"));
								if ui
									.button(crate::i18n::translate(
										"message-history-disable-capture",
									))
									.clicked()
								{
									begin_clear = Some(ClearScope::Account);
								}
							} else if ui
								.add_enabled(
									self.capture_controls,
									egui::Button::new(crate::i18n::translate(
										"message-history-enable-capture",
									)),
								)
								.clicked()
							{
								preferences.enabled = true;
								save_preferences = true;
							}
							if !self.capture_controls {
								ui.small(crate::i18n::translate(
									"message-history-package-required",
								));
							}
							ui.label(crate::i18n::translate("message-history-retention-days"));
							egui::ComboBox::from_id_salt((
								"message-history-retention",
								self.generation,
							))
							.selected_text(preferences.retention_days.to_string())
							.show_ui(ui, |ui| {
								for days in [1, 7, 30, 90, 365] {
									ui.selectable_value(
										&mut preferences.retention_days,
										days,
										format!("{days}"),
									);
								}
							});
							ui.label(crate::i18n::translate("message-history-max-messages"));
							egui::ComboBox::from_id_salt((
								"message-history-max-messages",
								self.generation,
							))
							.selected_text(preferences.max_messages.to_string())
							.show_ui(ui, |ui| {
								for count in [1_000, 10_000, 50_000] {
									ui.selectable_value(
										&mut preferences.max_messages,
										count,
										format!("{count}"),
									);
								}
							});
							ui.label(crate::i18n::translate("message-history-ghost-mode"));
							egui::ComboBox::from_id_salt((
								"message-history-ghost",
								self.generation,
							))
							.selected_text(crate::i18n::translate(match preferences.ghost {
								GhostMode::Off => "message-history-ghost-off",
								GhostMode::Mentions => "message-history-ghost-mentions",
								GhostMode::All => "message-history-ghost-all",
							}))
							.show_ui(ui, |ui| {
								for (mode, label) in [
									(GhostMode::Off, "message-history-ghost-off"),
									(GhostMode::Mentions, "message-history-ghost-mentions"),
									(GhostMode::All, "message-history-ghost-all"),
								] {
									ui.selectable_value(
										&mut preferences.ghost,
										mode,
										crate::i18n::translate(label),
									);
								}
							});
							ui.small(crate::i18n::translate("message-history-download-note"));
							ui.checkbox(
								&mut preferences.download_images,
								crate::i18n::translate("message-history-download-images"),
							);
							ui.checkbox(
								&mut preferences.download_audio,
								crate::i18n::translate("message-history-download-audio"),
							);
							ui.checkbox(
								&mut preferences.download_video,
								crate::i18n::translate("message-history-download-video"),
							);
							ui.label(crate::i18n::translate("message-history-ignore-users"));
							ui.add(
								egui::TextEdit::singleline(&mut preferences.ignore_users)
									.char_limit(MAX_PREFERENCE_LIST_CHARS),
							);
							ui.label(crate::i18n::translate("message-history-ignore-channels"));
							ui.add(
								egui::TextEdit::singleline(&mut preferences.ignore_channels)
									.char_limit(MAX_PREFERENCE_LIST_CHARS),
							);
							if ui
								.button(crate::i18n::translate("message-history-apply"))
								.clicked()
							{
								save_preferences = true;
							}
						});
						self.preferences = preferences;
						if save_preferences {
							self.queue(Request::SavePreferences {
								generation: self.generation,
								account: self.account,
								preferences: self.preferences.clone(),
							});
						}
						ui.separator();
						if ui
							.button(crate::i18n::translate("message-history-browse-all"))
							.clicked()
						{
							browse_all = true;
						}
						ui.horizontal_wrapped(|ui| {
							let mut kind = self.filters.kind;
							for filter in [Filter::All, Filter::Edited, Filter::Deleted] {
								ui.selectable_value(
									&mut kind,
									filter,
									crate::i18n::translate(filter.label()),
								);
							}
							self.filters.kind = kind;
							ui.separator();
							ui.label(crate::i18n::translate("message-history-filter-channel"));
							ui.add(
								egui::TextEdit::singleline(&mut self.filters.channel)
									.hint_text(crate::i18n::translate(
										"message-history-filter-id-hint",
									))
									.char_limit(MAX_FILTER_CHARS)
									.desired_width(110.0),
							);
							ui.label(crate::i18n::translate("message-history-filter-author"));
							ui.add(
								egui::TextEdit::singleline(&mut self.filters.author)
									.hint_text(crate::i18n::translate(
										"message-history-filter-id-hint",
									))
									.char_limit(MAX_FILTER_CHARS)
									.desired_width(110.0),
							);
						});
						let invalid_channel =
							matches!(parse_id_filter(&self.filters.channel), IdFilter::Invalid);
						let invalid_author =
							matches!(parse_id_filter(&self.filters.author), IdFilter::Invalid);
						if invalid_channel || invalid_author {
							crate::dialog::notice(
								ui,
								crate::dialog::Level::Warning,
								"message-history-filter-invalid-id",
							);
						}
						ui.separator();
						ui.horizontal_wrapped(|ui| {
							let channel = self.selected_channel();
							if ui
								.add_enabled(
									channel.is_some(),
									egui::Button::new(crate::i18n::translate(
										"message-history-clear-channel",
									)),
								)
								.clicked() && let Some(channel) = channel
							{
								begin_clear = Some(ClearScope::Channel(channel));
							}
							if ui
								.button(crate::i18n::translate("message-history-clear-account"))
								.clicked()
							{
								begin_clear = Some(ClearScope::Account);
							}
						});
						ui.label(crate::i18n::translate("message-history-export-warning"));
						ui.horizontal_wrapped(|ui| {
							if ui
								.button(crate::i18n::translate("message-history-export-text"))
								.clicked()
							{
								export = Some(ExportFormat::Text);
							}
							if ui
								.button(crate::i18n::translate("message-history-export-json"))
								.clicked()
							{
								export = Some(ExportFormat::Json);
							}
						});
						if let Some(status) = &self.status {
							ui.separator();
							ui.horizontal(|ui| {
								if !status.complete {
									ui.spinner();
								}
								ui.label(&status.text);
							});
							if let Some((done, total)) = status.progress
								&& total > 0
							{
								ui.add(
									egui::ProgressBar::new(done as f32 / total as f32)
										.text(format!("{done}/{total}")),
								);
							}
						}
						if let Some(error) = &self.error {
							crate::dialog::notice(ui, crate::dialog::Level::Error, error);
						}
						ui.separator();

						let keys = self.visible_keys();
						if self.loading && keys.is_empty() {
							ui.horizontal(|ui| {
								ui.spinner();
								ui.label(crate::i18n::translate("message-history-loading"));
							});
						} else if keys.is_empty() {
							ui.label(crate::i18n::translate("message-history-empty"));
						} else {
							egui::ScrollArea::vertical()
								.id_salt(("message-history-rows", self.generation))
								.max_height(300.0)
								.auto_shrink([false, false])
								.show(ui, |ui| {
									for key in keys {
										let Some(cached) = self.cache.get(&key) else {
											continue;
										};
										let entry = &cached.entry;
										let state = if entry.deleted {
											crate::i18n::translate("message-history-row-deleted")
										} else if entry
											.events
											.iter()
											.any(|change| change.kind == Kind::Edit)
										{
											crate::i18n::translate("message-history-row-edited")
										} else {
											crate::i18n::translate("message-history-row-retained")
										};
										let heading = format!(
											"{} {} · {} {} · {}",
											crate::i18n::translate(
												"message-history-column-channel"
											),
											entry.channel.0,
											crate::i18n::translate(
												"message-history-column-message"
											),
											entry.message.0,
											state
										);
										let body = preview(&entry.content, 180);
										if ui
											.add(
												egui::Button::new(
													egui::RichText::new(heading).strong(),
												)
												.selected(self.selected == Some(key))
												.wrap_mode(egui::TextWrapMode::Wrap),
											)
											.clicked()
										{
											self.selected = Some(key);
										}
										if !body.is_empty() {
											ui.label(egui::RichText::new(body).small());
										}
										ui.separator();
									}
								});
						}
						if self.scope == Some(Scope::All) {
							ui.horizontal(|ui| {
								if ui
									.add_enabled(
										!self.loading && self.has_older,
										egui::Button::new(crate::i18n::translate(
											"message-history-older",
										)),
									)
									.clicked()
								{
									page_request = Some(PageDirection::Older);
								}
								if ui
									.add_enabled(
										!self.loading && self.has_newer,
										egui::Button::new(crate::i18n::translate(
											"message-history-newer",
										)),
									)
									.clicked()
								{
									page_request = Some(PageDirection::Newer);
								}
								if self.loading {
									ui.spinner();
								}
							});
						}
						if self.error.is_some()
							&& !self.loading && ui
							.button(crate::i18n::translate("message-history-retry"))
							.clicked()
						{
							retry = true;
						}

						ui.separator();
						match self.selected {
							Some(key) => {
								let Some(cached) = self.cache.get(&key) else {
									crate::dialog::notice(
										ui,
										crate::dialog::Level::Warning,
										"message-history-cache-evicted",
									);
									return;
								};
								let entry = &cached.entry;
								ui.label(
									egui::RichText::new(crate::i18n::translate(
										"message-history-latest-content",
									))
									.strong(),
								);
								ui.add(egui::Label::new(&entry.content).wrap());
								ui.horizontal_wrapped(|ui| {
									ui.label(format!(
										"{} {}",
										crate::i18n::translate("message-history-column-channel"),
										entry.channel.0
									));
									ui.label(format!(
										"{} {}",
										crate::i18n::translate("message-history-column-message"),
										entry.message.0
									));
									ui.label(format!(
										"{} {}",
										crate::i18n::translate("message-history-column-author"),
										entry.author.0
									));
									ui.label(format!(
										"{} {}",
										crate::i18n::translate("message-history-column-revision"),
										entry.revision
									));
									if entry.deleted {
										ui.label(crate::i18n::translate("message-history-deleted"));
									}
								});
								if let Some(at) = entry.edited_at.and_then(full_timestamp) {
									ui.label(format!(
										"{} {at}",
										crate::i18n::translate("message-history-last-edited")
									));
								}
								if !entry.events.is_empty() {
									ui.separator();
									ui.label(
										egui::RichText::new(crate::i18n::translate(
											"message-history-versions",
										))
										.strong(),
									);
									for change in &entry.events {
										let kind = match change.kind {
											Kind::Edit => {
												crate::i18n::translate("message-history-edit")
											}
											Kind::Delete => {
												crate::i18n::translate("message-history-delete")
											}
										};
										let before = change.before.as_deref().unwrap_or("");
										let after = change.after.as_deref().unwrap_or("");
										let text = match change.kind {
											Kind::Edit => format!(
												"{kind} · {} {}: {before} → {after}",
												crate::i18n::translate(
													"message-history-column-revision"
												),
												change.revision
											),
											Kind::Delete => format!(
												"{kind} · {} {}: {before}",
												crate::i18n::translate(
													"message-history-column-revision"
												),
												change.revision
											),
										};
										ui.add(egui::Label::new(text).wrap());
									}
								}
								if !entry.attachments.is_empty() {
									ui.separator();
									ui.label(
										egui::RichText::new(crate::i18n::translate(
											"message-history-attachments",
										))
										.strong(),
									);
									for attachment in &entry.attachments {
										let mut metadata = attachment.filename.clone();
										metadata.push_str(&format!(" · {}", attachment.size));
										if let Some(content_type) = &attachment.content_type {
											metadata.push_str(" · ");
											metadata.push_str(content_type);
										}
										if attachment.deleted {
											metadata.push_str(" · ");
											metadata.push_str(&crate::i18n::translate(
												"message-history-deleted",
											));
										}
										ui.label(metadata);
									}
								}
								ui.separator();
								if ui
									.button(if entry.deleted {
										crate::i18n::translate("message-history-open-retained")
									} else {
										crate::i18n::translate("message-history-open-target")
									})
									.clicked()
								{
									target_request = Some(key);
								}
							}
							None => {
								ui.label(crate::i18n::translate("message-history-select-row"));
							}
						}
					});
			});
		if !remain_open {
			self.close();
			return;
		}
		if browse_all {
			self.open_all(self.generation, self.account);
		}
		if let Some(direction) = page_request {
			self.request_page(direction);
		}
		if retry {
			self.request_page(PageDirection::Older);
		}
		if let Some(format) = export {
			self.request_export(format);
		}
		if let Some(scope) = begin_clear {
			self.begin_clear(scope);
		}
		if let Some(key) = target_request {
			self.request_target(key);
		}
		if let Some(scope) = self.clear_confirm {
			let title = crate::i18n::translate("message-history-clear-confirm-title");
			let message = match scope {
				ClearScope::Channel(_) => {
					crate::i18n::translate("message-history-clear-confirm-channel")
				}
				ClearScope::Account => {
					crate::i18n::translate("message-history-clear-confirm-account")
				}
			};
			let choice = crate::dialog::Confirm::new(
				("message-history-clear", self.generation),
				title,
				message,
			)
			.danger()
			.confirm_label(crate::i18n::translate(
				"message-history-clear-confirm-action",
			))
			.note(
				crate::dialog::Level::Warning,
				crate::i18n::translate("message-history-clear-warning"),
			)
			.show(ctx);
			if let Some(choice) = choice {
				self.confirm_clear(choice == crate::dialog::Choice::Confirmed);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::message_logger::Change;

	fn entry(channel: u64, message: u64, content: &str) -> Entry {
		Entry {
			channel: Id(channel),
			message: Id(message),
			author: Id(7),
			revision: 1,
			content: content.into(),
			deleted: false,
			events: Vec::new(),
			attachments: Vec::new(),
			mention_everyone: false,
			mentions_self: false,
			edited_at: None,
			observed_at_ms: 1,
		}
	}

	fn page(generation: u64, account: Option<Id>, entries: Vec<Entry>) -> HistoryPage {
		HistoryPage {
			generation,
			account,
			entries,
			has_older: false,
			has_newer: false,
			channel: None,
			message: None,
		}
	}

	#[test]
	fn cache_deduplicates_rows_and_stays_within_row_and_byte_bounds() {
		let account = Some(Id(9));
		let mut browser = HistoryBrowser::default();
		browser.open_all(7, account);
		assert!(matches!(
			browser.take_request(),
			Some(Request::LoadPage { .. })
		));

		for page_index in 0..8 {
			let entries = (0..PAGE_ROWS)
				.map(|index| {
					let message = (page_index * PAGE_ROWS + index + 1) as u64;
					entry(1, message, &"x".repeat(4096))
				})
				.collect();
			assert!(browser.accept_page(page(7, account, entries)));
		}
		assert!(browser.row_count() <= MAX_CACHE_ROWS);
		assert!(browser.byte_len() <= MAX_CACHE_BYTES);

		let before = browser.row_count();
		let key = PageKey::new(Id(1), Id(800));
		assert!(browser.accept_page(page(7, account, vec![entry(1, 800, "replacement body")],)));
		assert_eq!(
			browser.row_count(),
			before,
			"an existing channel/message key must replace, not duplicate"
		);
		assert_eq!(
			browser
				.cache
				.get(&key)
				.map(|cached| cached.entry.content.as_str()),
			Some("replacement body")
		);
	}

	#[test]
	fn cache_eviction_removes_the_row_without_closing_the_viewer() {
		let mut browser = HistoryBrowser::default();
		assert_eq!(
			browser.open_target(7, Some(Id(9)), Id(1), Id(2), Some(entry(1, 2, "selected"))),
			TargetOpen::Retained
		);
		for index in 0..MAX_CACHE_ROWS + 1 {
			browser.insert(entry(2, 1_000 + index as u64, "paged"));
		}
		assert!(browser.is_open());
		assert_eq!(
			browser.focused_entry().map(|entry| entry.content.as_str()),
			None,
			"the evicted focus is absent from the page cache, not reported as retained data loss"
		);
		assert!(browser.row_count() <= MAX_CACHE_ROWS);
	}

	#[test]
	fn page_acceptance_is_fenced_by_generation_and_account() {
		let mut browser = HistoryBrowser::default();
		browser.open_all(7, Some(Id(9)));
		browser.take_requests();
		assert!(!browser.accept_page(page(6, Some(Id(9)), vec![entry(1, 1, "a")])));
		assert!(!browser.accept_page(page(7, Some(Id(8)), vec![entry(1, 1, "a")])));
		assert_eq!(browser.row_count(), 0);
		assert!(browser.accept_page(page(7, Some(Id(9)), vec![entry(1, 1, "a")])));
		assert_eq!(browser.row_count(), 1);
	}

	#[test]
	fn deleted_target_uses_the_retained_viewer_without_a_search_or_page_request() {
		let account = Some(Id(9));
		let mut row = entry(1, 2, "last retained body before deletion");
		row.deleted = true;
		let mut browser = HistoryBrowser::default();
		assert_eq!(
			browser.open_target(7, account, Id(1), Id(2), Some(row)),
			TargetOpen::Retained
		);
		assert!(browser.take_requests().is_empty());
		assert_eq!(browser.focused_target(), Some((Id(1), Id(2))));
		assert!(browser.focused_entry().is_some_and(|entry| entry.deleted));
		browser.request_target(PageKey::new(Id(1), Id(2)));
		assert_eq!(
			browser.take_requests(),
			vec![Request::OpenTarget {
				generation: 7,
				channel: Id(1),
				message: Id(2),
				deleted: true,
			}],
			"the host receives the deleted flag so it will not search the service"
		);
	}

	#[test]
	fn deleted_target_without_a_cached_row_requests_a_local_page_only() {
		let mut browser = HistoryBrowser::default();
		assert_eq!(
			browser.open_target(7, Some(Id(9)), Id(1), Id(2), None),
			TargetOpen::NeedsPage
		);
		assert_eq!(
			browser.take_requests(),
			vec![Request::LoadPage {
				generation: 7,
				account: Some(Id(9)),
				channel: Some(Id(1)),
				author: None,
				message: Some(Id(2)),
				kind: Filter::All,
				before: None,
				after: None,
				limit: PAGE_ROWS,
			}]
		);
	}

	#[test]
	fn live_target_requests_normal_navigation_without_the_deleted_flag() {
		let mut browser = HistoryBrowser::default();
		assert_eq!(
			browser.open_target(7, Some(Id(9)), Id(1), Id(2), Some(entry(1, 2, "live"))),
			TargetOpen::Retained
		);
		assert!(browser.request_target(PageKey::new(Id(1), Id(2))));
		assert!(
			!browser.is_open(),
			"the channel area resumes normal navigation"
		);
		assert_eq!(
			browser.take_requests(),
			vec![Request::OpenTarget {
				generation: 7,
				channel: Id(1),
				message: Id(2),
				deleted: false,
			}]
		);
	}

	#[test]
	fn per_message_fallback_uses_the_in_memory_entry_without_a_host_round_trip() {
		let mut browser = HistoryBrowser::default();
		assert_eq!(
			browser.open_target(
				3,
				Some(Id(4)),
				Id(5),
				Id(6),
				Some(entry(5, 6, "already observed")),
			),
			TargetOpen::Retained
		);
		assert!(browser.take_requests().is_empty());
		assert_eq!(
			browser.focused_entry().map(|entry| entry.content.as_str()),
			Some("already observed")
		);
	}

	#[test]
	fn filters_match_kind_and_decimal_ids_without_inventing_names() {
		let account = Some(Id(9));
		let mut browser = HistoryBrowser::default();
		browser.open_all(7, account);
		browser.take_requests();
		let mut edited = entry(10, 1, "edited");
		edited.events.push(Change {
			kind: Kind::Edit,
			revision: 2,
			before: Some("old".into()),
			after: Some("edited".into()),
		});
		let mut deleted = entry(10, 2, "deleted");
		deleted.deleted = true;
		assert!(browser.accept_page(page(
			7,
			account,
			vec![edited.clone(), deleted.clone(), entry(11, 3, "plain")],
		)));

		browser.filters.kind = Filter::Edited;
		assert_eq!(browser.visible_keys(), vec![PageKey::new(Id(10), Id(1))]);
		browser.filters.kind = Filter::Deleted;
		assert_eq!(browser.visible_keys(), vec![PageKey::new(Id(10), Id(2))]);
		browser.filters.kind = Filter::All;
		browser.filters.channel = "10".into();
		assert_eq!(
			browser.visible_keys(),
			vec![PageKey::new(Id(10), Id(2)), PageKey::new(Id(10), Id(1))]
		);
		browser.filters.channel = "not-a-decimal-id".into();
		assert!(browser.visible_keys().is_empty());
		browser.filters.channel.clear();
		browser.filters.author = "7".into();
		assert_eq!(browser.visible_keys().len(), 3);
		browser.filters.author = "8".into();
		assert!(browser.visible_keys().is_empty());
	}

	#[test]
	fn clear_needs_confirmation_and_export_is_an_explicit_host_request() {
		let account = Some(Id(9));
		let mut browser = HistoryBrowser::default();
		browser.open_all(7, account);
		browser.take_requests();
		browser.begin_clear(ClearScope::Channel(Id(4)));
		assert!(
			browser.take_requests().is_empty(),
			"no clear before confirmation"
		);
		browser.confirm_clear(true);
		assert_eq!(
			browser.take_requests(),
			vec![Request::Clear {
				generation: 7,
				account,
				scope: ClearScope::Channel(Id(4)),
			}]
		);
		assert!(
			browser
				.status
				.as_ref()
				.is_some_and(|status| !status.complete)
		);

		browser.begin_clear(ClearScope::Account);
		browser.confirm_clear(false);
		assert!(browser.take_requests().is_empty(), "cancel queues nothing");

		browser.request_export(ExportFormat::Json);
		assert_eq!(
			browser.take_requests(),
			vec![Request::Export {
				generation: 7,
				account,
				format: ExportFormat::Json,
			}]
		);
		assert!(
			browser
				.status
				.as_ref()
				.is_some_and(|status| !status.complete)
		);
	}

	#[test]
	fn operation_status_text_is_bounded_and_foreign_generations_are_ignored() {
		let mut browser = HistoryBrowser::default();
		browser.open_all(7, Some(Id(9)));
		browser.set_status(6, "stale", None, true);
		assert!(browser.status.is_none());
		browser.set_status(7, "z".repeat(MAX_STATUS_CHARS + 10), Some((9, 4)), false);
		let status = browser.status.as_ref().unwrap();
		assert_eq!(status.text.chars().count(), MAX_STATUS_CHARS + 1);
		assert_eq!(status.progress, Some((4, 4)));
		assert!(!status.complete);
	}
}
