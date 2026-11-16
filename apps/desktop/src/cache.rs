use eframe::egui;
use local_store::{Appearance, LocalStore, StoreError};
use model::{Id, Message};
use std::{
	collections::{BTreeMap, BTreeSet},
	sync::{
		Arc, Condvar, Mutex,
		atomic::{AtomicBool, AtomicU64, Ordering},
		mpsc::{self, Receiver, SyncSender},
	},
};
// One full CJK font plus the existing 16 MiB of storage work and metadata.
const QUEUE_BYTES: usize = 48 * 1024 * 1024;
const WINDOW_BYTES: usize = 4 * 1024 * 1024;
const SYNC_QUEUE_BYTES: usize = 8 * 1024 * 1024;
/// Reliable events plus the bounded typing and confirmation lanes drained in one pass.
pub const MEMBER_DISPLAY_CLEANUP_LIMIT: usize = client_core::EVENT_SLOTS + 9;

struct Budget {
	limit: usize,
	used: Mutex<usize>,
	available: Condvar,
}
impl Default for Budget {
	fn default() -> Self {
		Self {
			limit: QUEUE_BYTES,
			used: Mutex::new(0),
			available: Condvar::new(),
		}
	}
}
pub struct Reservation {
	budget: Arc<Budget>,
	bytes: usize,
	_sync: Option<Box<Reservation>>,
}
impl Budget {
	fn reserve(self: &Arc<Self>, bytes: usize, wait: bool) -> Option<Reservation> {
		if bytes > self.limit {
			return None;
		}
		let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
		while *used + bytes > self.limit {
			if !wait {
				return None;
			}
			used = self.available.wait(used).unwrap_or_else(|e| e.into_inner());
		}
		*used += bytes;
		Some(Reservation {
			budget: self.clone(),
			bytes,
			_sync: None,
		})
	}
}
impl Drop for Reservation {
	fn drop(&mut self) {
		*self.budget.used.lock().unwrap_or_else(|e| e.into_inner()) -= self.bytes;
		self.budget.available.notify_one();
	}
}
fn message_bytes(messages: &Vec<Message>) -> usize {
	messages.iter().map(Message::bytes).sum::<usize>()
		+ messages.capacity().saturating_sub(messages.len()) * size_of::<Message>()
}
#[allow(clippy::large_enum_variant)]
pub enum Operation {
	LoadCustomFont,
	SaveCustomFont(Option<ui::fonts::CustomFont>),
	SaveAppPreferences(Box<local_store::AppPreferences>),
	LoadAppearance,
	SaveAppearance(Appearance),
	SaveThemeVariant(Option<String>),
	LoadReadingPreferences,
	SaveReadingPreferences(model::ReadingPreferences),
	LoadGameActivity,
	SaveGameActivity(bool),
	LoadMinimizeToTray,
	SaveMinimizeToTray(bool),
	LoadDrafts,
	LoadGifFavorites,
	SaveGifFavorites(Vec<model::Gif>),
	LoadChannelPreferences,
	SaveChannelPreferences(model::ChannelPreferences),
	LoadAccountPresences,
	SaveAccountPresence(model::OwnPresence),
	LoadAccounts,
	SaveAccount(model::SavedAccount),
	SetAccountToken {
		account: Id,
		has_token: bool,
	},
	LoadChannel {
		channel: Id,
		request: u64,
	},
	FindMemberDisplays {
		request: client_core::member_search::Request,
		revision: u64,
		visible: bool,
	},
	SyncSaveMembers {
		job: u64,
		request: u64,
		guild: Id,
		rows: Vec<model::MemberDisplay>,
		missing: Vec<Id>,
	},
	SyncReconcileMembers {
		job: u64,
		request: u64,
		guild: Id,
		users: Vec<Id>,
	},
	SyncReadRevision {
		job: u64,
		request: u64,
		channel: Id,
	},
	SyncSavePage {
		job: u64,
		request: u64,
		channel: Id,
		guild: Option<Id>,
		parent: Option<Id>,
		messages: Vec<Message>,
		coverage: local_store::HistoryCoverage,
	},
	LoadSyncCheckpoint,
	SaveSyncCheckpoint(local_store::SyncCheckpoint),
	ClearSyncCheckpoint,
	LoadHistoryPage {
		channel: Id,
		request: u64,
		page: local_store::HistoryPageQuery,
	},
	CacheUsage,
	ObserveHistoryChange {
		channel: Id,
		message: Id,
	},

	LoadMembers {
		guild: Id,
		revision: u64,
	},
	LoadExpressions {
		guild: Id,
		revision: u64,
	},
	SaveExpressions(Box<model::ExpressionCatalog>),
	LoadStickerPackHints {
		revision: u64,
	},
	SaveStickerPackHints(Box<model::StickerPackHints>),
	ClearExpressions {
		guild: Id,
		/// None clears the guild; Some clears only the affected catalog.
		emoji: Option<bool>,
	},
	SaveMembers(Vec<model::MemberDisplay>),
	LoadServers {
		revision: u64,
		guilds: Vec<Id>,
	},
	SaveServers {
		rows: Vec<local_store::ServerDisplay>,
		revision: u64,
	},
	RemoveServer {
		guild: Id,
	},
	RemoveMember {
		guild: Id,
		user: Option<Id>,
	},
	RemoveMemberDisplays(Vec<(Id, Id)>),
	ClearChannel {
		channel: Id,
	},
	SaveDraft {
		channel: Id,
		content: String,
	},
	/// `start`: the window reaches the channel's first message.
	SaveChannel {
		channel: Id,
		messages: Vec<Message>,
		start: bool,
	},
	SaveChanges {
		channel: Id,
		messages: Vec<Message>,
		retained: Vec<Id>,
		start: bool,
	},
	DeleteMessages {
		channel: Id,
		ids: Vec<Id>,
	},
	ClearHistory,
	Forget,
}
#[allow(clippy::large_enum_variant)]
pub enum Outcome {
	SyncMembersReconciled {
		job: u64,
		request: u64,
		guild: Id,
		epoch: u64,
		result: Result<usize, StoreError>,
	},
	MemberDisplaysFound {
		request: client_core::member_search::Request,
		revision: u64,
		visible: bool,
		epoch: u64,
		result: Result<Vec<model::MemberDisplay>, StoreError>,
	},
	SyncMembersSaved {
		job: u64,
		request: u64,
		guild: Id,
		epoch: u64,
		result: Result<(), StoreError>,
	},
	SyncRevision {
		job: u64,
		request: u64,
		channel: Id,
		epoch: u64,
		result: Result<u64, StoreError>,
	},
	SyncPageSaved {
		job: u64,
		request: u64,
		channel: Id,
		epoch: u64,
		result: Result<local_store::SyncPageResult, StoreError>,
	},
	SyncCheckpoint {
		epoch: u64,
		result: Result<Option<local_store::SyncCheckpoint>, StoreError>,
	},
	SyncCheckpointSaved {
		epoch: u64,
		result: Result<(), StoreError>,
	},
	HistoryPage {
		channel: Id,
		request: u64,
		epoch: u64,
		result: Result<Vec<Message>, StoreError>,
	},
	CacheUsage(Result<local_store::cache_budget::CacheUsage, StoreError>),
	CustomFont(Result<Option<ui::fonts::CustomFont>, &'static str>),
	AppPreferencesSaved(Result<(), StoreError>),
	/// Saved appearance plus the saved theme preset key, if any.
	Appearance(Appearance, Option<String>),
	ReadingPreferences(Result<model::ReadingPreferences, StoreError>),
	ReadingPreferencesSaved(Result<(), StoreError>),
	GameActivity(Result<bool, StoreError>),
	GameActivitySaved(Result<(), StoreError>),
	MinimizeToTray(Result<bool, StoreError>),
	MinimizeToTraySaved(Result<(), StoreError>),
	Drafts(BTreeMap<Id, String>),
	GifFavorites(Vec<model::Gif>),
	ChannelPreferences(Result<model::ChannelPreferences, StoreError>),
	ChannelPreferencesSaved(Result<(), StoreError>),
	AccountPresences(Result<std::collections::BTreeMap<model::Id, model::OwnPresence>, StoreError>),
	AccountPresenceSaved(Result<(), StoreError>),
	/// The whole switcher roster, plus any accounts pruned to keep it bounded. Pruning is
	/// already committed when this is produced, so the IDs travel outside the roster result:
	/// a failed re-read must not strand their saved secrets and cached data.
	Accounts {
		roster: Result<Vec<model::SavedAccount>, StoreError>,
		pruned: Vec<Id>,
	},
	Channel {
		channel: Id,
		request: u64,
		messages: Vec<Message>,
		/// The cached window reaches the channel's first message.
		start: bool,
		epoch: u64,
	},
	Saved,
	HistoryCleared,
	Members {
		guild: Id,
		revision: u64,
		epoch: u64,
		result: Result<Vec<model::MemberDisplay>, StoreError>,
	},
	Expressions {
		account: Id,
		guild: Id,
		revision: u64,
		epoch: u64,
		result: Result<Option<model::ExpressionCatalog>, StoreError>,
	},
	StickerPackHints {
		account: Id,
		revision: u64,
		epoch: u64,
		result: Result<Option<model::StickerPackHints>, StoreError>,
	},
	Servers {
		revision: u64,
		epoch: u64,
		result: Result<Vec<local_store::ServerDisplay>, StoreError>,
	},
	ServersSaved {
		revision: u64,
		epoch: u64,
		ids: Vec<Id>,
		result: Option<Result<(), StoreError>>,
	},
	ScopedCleanupFailed {
		account: Id,
	},
	Failed {
		error: StoreError,
		message: &'static str,
		draft_restore: bool,
		history_cleanup: bool,
	},
}
#[derive(Clone, Copy, Default)]
struct HistoryPins {
	channel: Option<(Id, Id)>,
	writing: Option<(Id, Id)>,
	sync: [Option<(Id, Id)>; 2],
	guild: Option<(Id, Id)>,
}

pub struct Cache {
	send: SyncSender<(u64, Id, u64, Operation, Reservation)>,
	pub receive: Receiver<(u64, Outcome, Reservation)>,
	budget: Arc<Budget>,
	sync_budget: Arc<Budget>,
	protected_history: Arc<Mutex<HistoryPins>>,
	pub history: Arc<HistorySafety>,
}
/// Saves in these lanes remain pending after queue backpressure and retry on progress.
pub fn coalesced_retry(operation: &Operation) -> bool {
	matches!(
		operation,
		Operation::SaveGifFavorites(_)
			| Operation::SaveServers { .. }
			| Operation::SaveExpressions(_)
			| Operation::SaveStickerPackHints(_)
	)
}
/// Coalesced work for up to the number of guild catalog rows the store can retain.
#[derive(Default)]
pub struct ExpressionCacheSchedule {
	scope: Option<(u64, Id, u64)>,
	core_epoch: Option<u64>,
	selected: Option<Id>,
	pending: std::collections::VecDeque<(Id, u8)>,
	waiting: bool,
}
impl ExpressionCacheSchedule {
	const LIMIT: usize = 128;
	pub const LOAD: u8 = 1;
	pub const SAVE: u8 = 2;
	pub fn scope(&mut self, scope: (u64, Id, u64), guilds: impl Iterator<Item = Id>) {
		if self.scope == Some(scope) {
			return;
		}
		*self = Self::default();
		self.scope = Some(scope);
		for guild in guilds.take(Self::LIMIT) {
			self.enqueue(guild, Self::LOAD | Self::SAVE);
		}
	}
	/// Reload only once when core invalidates all guild hint revisions, including
	/// rare revision-map overflow. Unchanged authoritative catalogs need no writes.
	pub fn observe_epoch(&mut self, epoch: u64, guilds: impl Iterator<Item = Id>) {
		let previous = self.core_epoch.replace(epoch);
		if previous.is_some_and(|previous| previous != epoch) {
			self.refresh(guilds);
		}
	}
	pub fn refresh(&mut self, guilds: impl Iterator<Item = Id>) {
		for guild in guilds.take(Self::LIMIT) {
			self.enqueue(guild, Self::LOAD);
		}
		if let Some(guild) = self.selected {
			self.prioritize(guild, Self::LOAD);
		}
	}
	pub fn enqueue(&mut self, guild: Id, operations: u8) {
		if guild.0 == 0 || operations == 0 {
			return;
		}
		if let Some((_, flags)) = self.pending.iter_mut().find(|(id, _)| *id == guild) {
			*flags |= operations;
			return;
		}
		if self.pending.len() == Self::LIMIT {
			let victim = self
				.pending
				.iter()
				.position(|(id, _)| Some(*id) != self.selected)
				.unwrap_or(0);
			self.pending.remove(victim);
		}
		self.pending.push_back((guild, operations));
	}
	fn prioritize(&mut self, guild: Id, operations: u8) {
		self.enqueue(guild, operations);
		if let Some(index) = self.pending.iter().position(|(id, _)| *id == guild)
			&& let Some(item) = self.pending.remove(index)
		{
			self.pending.push_front(item);
		}
	}
	pub fn selected(&mut self, guild: Option<Id>) {
		if self.selected != guild {
			self.selected = guild;
			if let Some(guild) = guild {
				self.prioritize(guild, Self::LOAD | Self::SAVE);
			}
		}
	}
	pub fn next(&self) -> Option<(Id, u8)> {
		if self.waiting {
			return None;
		}
		self.pending.front().map(|&(guild, flags)| {
			(
				guild,
				if flags & Self::LOAD != 0 {
					Self::LOAD
				} else {
					Self::SAVE
				},
			)
		})
	}
	pub fn completed(&mut self, guild: Id, operation: u8) {
		if let Some((id, flags)) = self.pending.front_mut()
			&& *id == guild
		{
			*flags &= !operation;
			if *flags == 0 {
				self.pending.pop_front();
			}
		}
	}
	pub fn blocked(&mut self) {
		self.waiting = true;
	}
	pub fn capacity_changed(&mut self) {
		self.waiting = false;
	}
	pub fn remove(&mut self, guild: Id) {
		self.pending.retain(|(id, _)| *id != guild);
	}
}
/// Retry a rejected read only after storage has made progress or its scope changes.
#[derive(Default)]
pub struct HistoryLoad {
	attempted: Option<(u64, Id, u64, u64)>,
	accepted: Option<(u64, Id, u64, u64)>,
}
impl HistoryLoad {
	pub fn attempt(&mut self, key: (u64, Id, u64, u64)) -> bool {
		if self.accepted == Some(key) || self.attempted == Some(key) {
			return false;
		}
		self.attempted = Some(key);
		true
	}
	pub fn accepted(&mut self, key: (u64, Id, u64, u64)) {
		self.accepted = Some(key);
	}
	pub fn was_accepted(&self, key: (u64, Id, u64, u64)) -> bool {
		self.accepted == Some(key)
	}
	pub fn capacity_changed(&mut self) {
		self.attempted = None;
	}
	/// A completed read discarded by its consumer may retry after that worker progress.
	pub fn retry(&mut self, key: (u64, Id, u64, u64)) {
		if self.accepted == Some(key) {
			*self = Self::default();
		}
	}
	/// Called only for a nonempty completed read whose account/epoch fence still holds.
	/// Navigation can make a result ineligible without changing its observation revision.
	pub fn member_result(
		&mut self,
		key: (u64, Id, u64, u64),
		revision: u64,
		state: &client_core::State,
	) {
		if key.0 == state.generation
			&& (revision != state.member_display_revision()
				|| !state.gateway_connected
				|| state.auth != client_core::auth::AuthState::Authenticated
				|| !state.selected.is_some_and(|channel| {
					state.can_view(channel)
						&& state
							.channel(channel)
							.is_some_and(|c| c.guild == Some(key.1))
				})) {
			self.retry(key);
		}
	}
	/// RAM identities are retired on these accepted session transitions even when the
	/// account generation and disk-safety epoch survive a successful RESUME.
	pub fn observe_session(&mut self, generation: u64, event: &client_core::Envelope) -> bool {
		if event.generation == generation
			&& (event.event.ready_navigation().is_some()
				|| matches!(
					event.event,
					client_core::Event::Disconnected
						| client_core::Event::Resync
						| client_core::Event::PermissionsChanged
				)) {
			*self = Self::default();
			true
		} else {
			false
		}
	}
}

/// A scoped cleanup can retire a previously accepted save without changing the
/// display revision, so the safety epoch is part of the save admission key.
#[derive(Default)]
pub struct MemberCacheClear {
	generation: u64,
	blocked: std::collections::BTreeSet<Id>,
}
impl MemberCacheClear {
	pub fn reset(&mut self, state: &client_core::State) {
		self.generation = state.generation;
		self.blocked = state.cached_member_guilds().into_iter().collect();
	}
	pub fn observe(&mut self, state: &client_core::State, guild: Option<Id>, before: u64) {
		if self.generation == state.generation
			&& before != state.member_display_revision()
			&& let Some(guild) = guild
		{
			self.blocked.remove(&guild);
		}
	}
}

pub fn member_save_key(
	state: &client_core::State,
	guild: Id,
	epoch: u64,
	cleared: &MemberCacheClear,
) -> Option<(u64, Id, u64, u64)> {
	let revision = state.member_display_cache_revision();
	(cleared.generation != state.generation || !cleared.blocked.contains(&guild)).then_some((
		state.generation,
		guild,
		revision,
		epoch,
	))
}

pub fn member_observation_guild(event: &client_core::Event) -> Option<Id> {
	match event {
		client_core::Event::MemberIdentity(update) => Some(update.guild),
		client_core::Event::Typing(signal) => signal.member.as_ref().map(|member| member.guild),
		client_core::Event::MemberSearch { request, .. }
		| client_core::Event::MemberSearchPartial { request, .. } => Some(request.guild),
		client_core::Event::Members(list) => list.guild,
		_ => None,
	}
}

/// Preserve only a bounded target while the reducer validates the event.
pub fn member_display_target(event: &client_core::Event) -> Option<(Id, Id)> {
	let update = match event {
		client_core::Event::MemberIdentity(update) => Some(update.as_ref()),
		client_core::Event::Typing(signal) => signal.member.as_deref(),
		_ => None,
	}?;
	(!update.departed
		&& update.valid()
		&& (update.profile.is_some() || !matches!(update.nick, model::Patch::Absent)))
	.then_some((update.guild, update.user))
}

pub fn departed_member_target(
	state: &client_core::State,
	envelope: &client_core::Envelope,
) -> Option<(Id, Id)> {
	match &envelope.event {
		client_core::Event::MemberIdentity(update)
			if envelope.generation == state.generation && update.departed && update.valid() =>
		{
			Some((update.guild, update.user))
		}
		_ => None,
	}
}

/// An accepted display change cannot leave its older disk row behind. A complete
/// observation can be evicted before the next coalesced save snapshots it.
pub fn changed_member_display_cleanup(
	state: &client_core::State,
	target: Option<(Id, Id)>,
	before: u64,
) -> Option<(Id, Id)> {
	let (guild, user) = target?;
	(before != state.member_display_cache_revision()).then_some((guild, user))
}

/// One bounded cleanup per event drain, with an explicit account for a scope handoff.
#[derive(Default)]
pub struct MemberDisplayCleanup {
	scope: Option<(u64, Id)>,
	targets: Vec<(Id, Id)>,
}
impl MemberDisplayCleanup {
	pub fn observe(
		&mut self,
		state: &client_core::State,
		target: Option<(Id, Id)>,
		before: u64,
	) -> Option<(u64, Id, Operation)> {
		let scope = state.user.as_ref().map(|user| (state.generation, user.id));
		let target = changed_member_display_cleanup(state, target, before);
		let distinct = target.is_some_and(|target| !self.targets.contains(&target));
		let ready = if self.scope != scope
			|| (distinct && self.targets.len() == MEMBER_DISPLAY_CLEANUP_LIMIT)
		{
			self.take()
		} else {
			None
		};
		self.scope = scope;
		if scope.is_some()
			&& let Some(target) = target
			&& !self.targets.contains(&target)
		{
			self.targets.push(target);
		}
		ready
	}
	pub fn take(&mut self) -> Option<(u64, Id, Operation)> {
		let (generation, account) = self.scope.take()?;
		(!self.targets.is_empty()).then(|| {
			(
				generation,
				account,
				Operation::RemoveMemberDisplays(std::mem::take(&mut self.targets)),
			)
		})
	}
}

fn valid_member_display_cleanup(targets: &[(Id, Id)]) -> bool {
	!targets.is_empty()
		&& targets.len() <= MEMBER_DISPLAY_CLEANUP_LIMIT
		&& targets
			.iter()
			.all(|(guild, user)| guild.0 != 0 && user.0 != 0)
}

fn remove_member_display_targets(
	store: &mut LocalStore,
	account: Id,
	targets: &[(Id, Id)],
) -> Result<(), StoreError> {
	if !valid_member_display_cleanup(targets) {
		return Err(StoreError::Capacity);
	}
	let mut guilds: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
	for &(guild, user) in targets {
		guilds.entry(guild).or_default().push(user);
	}
	for (guild, users) in guilds {
		for chunk in users.chunks(100) {
			store.remove_member_displays(account, guild, chunk)?;
		}
	}
	Ok(())
}

/// Admission state only: GIF contents remain in the bounded account-scoped state.
#[derive(Default)]
pub struct GifCache {
	pub load: HistoryLoad,
	blocked_save: Option<(u64, Id)>,
	loaded: Option<(u64, Id)>,
}
impl GifCache {
	pub fn loaded(&mut self, scope: (u64, Id)) {
		if self.load.was_accepted((scope.0, scope.1, 0, 0)) {
			self.loaded = Some(scope);
		}
	}
	pub fn may_save(&self, scope: (u64, Id)) -> bool {
		self.loaded == Some(scope) && self.blocked_save != Some(scope)
	}
	pub fn reject_save(&mut self, scope: (u64, Id)) {
		self.blocked_save = Some(scope);
	}
	pub fn capacity_changed(&mut self) {
		self.load.capacity_changed();
		self.blocked_save = None;
	}
}
#[derive(Default)]
pub struct HistorySafety {
	epoch: AtomicU64,
	blocked: AtomicBool,
	failed: AtomicBool,
}
impl HistorySafety {
	pub fn invalidate(&self) {
		self.epoch.fetch_add(1, Ordering::SeqCst);
	}
	pub fn block(&self) {
		self.blocked.store(true, Ordering::SeqCst);
	}
	pub fn fail(&self) {
		self.failed.store(true, Ordering::SeqCst);
		self.block();
	}
	pub fn cleared(&self) {
		if !self.failed.load(Ordering::SeqCst) {
			self.blocked.store(false, Ordering::SeqCst);
		}
	}
	pub fn epoch(&self) -> u64 {
		self.epoch.load(Ordering::SeqCst)
	}
	pub fn allows(&self, epoch: u64) -> bool {
		!self.failed.load(Ordering::SeqCst)
			&& !self.blocked.load(Ordering::SeqCst)
			&& self.epoch() == epoch
	}
}
/// At most sixteen distinct account cleanups await queue space; drafts are unaffected.
#[derive(Default)]
pub struct HistoryClears {
	accounts: BTreeSet<Id>,
	queued: usize,
}
impl HistoryClears {
	pub fn pending(&self) -> bool {
		!self.accounts.is_empty() || self.queued != 0
	}
	pub fn request(&mut self, account: Id) -> bool {
		if self.accounts.len() == 16 && !self.accounts.contains(&account) {
			return false;
		}
		self.accounts.insert(account);
		true
	}
	pub fn next(&self) -> Option<Id> {
		self.accounts.first().copied()
	}
	pub fn queued(&mut self, account: Id) {
		if self.accounts.remove(&account) {
			self.queued += 1;
		}
	}
	pub fn acknowledge(&mut self, safety: &HistorySafety) -> bool {
		self.queued = self.queued.saturating_sub(1);
		let complete = self.accounts.is_empty() && self.queued == 0;
		if complete {
			safety.cleared();
		}
		complete
	}
}

/// Successful credential removals waiting for storage queue space, bounded by the roster.
#[derive(Default)]
pub struct AccountForgets {
	accounts: BTreeMap<Id, u64>,
}
impl AccountForgets {
	pub fn pending(&self) -> bool {
		!self.accounts.is_empty()
	}
	pub fn contains(&self, account: Id) -> bool {
		self.accounts.contains_key(&account)
	}
	pub fn request(&mut self, generation: u64, account: Id) -> bool {
		if self.accounts.len() == model::MAX_SAVED_ACCOUNTS && !self.accounts.contains_key(&account)
		{
			return false;
		}
		self.accounts.insert(account, generation);
		true
	}
	pub fn next(&mut self, generation: u64, active_account: Option<Id>) -> Option<Id> {
		// An explicit sign-in to the same account supersedes its old deletion. Other
		// accounts' device-scoped cleanup survives ordinary session switches.
		self.accounts.retain(|account, requested| {
			crate::credentials::account_removal_applies(
				*requested,
				generation,
				*account,
				active_account,
			)
		});
		self.accounts.first_key_value().map(|(account, _)| *account)
	}
	pub fn queued(&mut self, account: Id) {
		self.accounts.remove(&account);
	}
}
impl Cache {
	/// Update a fixed-size reading pin. Filesystem work stays on the storage worker.
	pub fn protect_history(&self, account: Id, channel: Option<Id>) {
		let mut pins = self
			.protected_history
			.lock()
			.unwrap_or_else(|error| error.into_inner());
		pins.channel = channel.map(|channel| (account, channel));
		if channel.is_none() || pins.guild.is_some_and(|(old, _)| old != account) {
			pins.guild = None;
		}
	}
	pub fn protect_guild(&self, account: Id, guild: Option<Id>) {
		self.protected_history
			.lock()
			.unwrap_or_else(|error| error.into_inner())
			.guild = guild.map(|guild| (account, guild));
	}
	pub fn protect_sync_history(&self, account: Id, channels: &[Id]) {
		let mut pins = self
			.protected_history
			.lock()
			.unwrap_or_else(|error| error.into_inner());
		pins.sync = [None; 2];
		for (slot, channel) in pins
			.sync
			.iter_mut()
			.zip(channels.iter().copied().filter(|channel| channel.0 != 0))
		{
			*slot = Some((account, channel));
		}
	}
	/// FIFO cleanup plus the epoch fence prevents earlier reads/writes restoring removed data.
	pub fn invalidate_scope(&self, generation: u64, account: Id, operation: Operation) -> bool {
		let blocked = !self.history.allows(self.history.epoch());
		self.history.invalidate();
		if blocked || !self.queue(generation, account, operation) {
			self.history.block();
			false
		} else {
			true
		}
	}
	pub fn delete_messages(&self, generation: u64, account: Id, channel: Id, ids: Vec<Id>) -> bool {
		let blocked = !self.history.allows(self.history.epoch());
		self.history.invalidate();
		if blocked
			|| ids.len() > 100
			|| !self.queue(
				generation,
				account,
				Operation::DeleteMessages { channel, ids },
			) {
			self.history.block();
			false
		} else {
			true
		}
	}
	pub fn queue(&self, generation: u64, account: Id, operation: Operation) -> bool {
		if matches!(&operation, Operation::SaveChannelPreferences(value) if !value.is_valid()) {
			return false;
		}
		if matches!(&operation, Operation::SyncSavePage {job,request,channel,guild,parent,coverage,..} if coverage.job != *job || coverage.request != *request || !valid_history_scope(*channel,*guild,*parent))
			|| matches!(&operation, Operation::SyncSaveMembers {job,request,guild,rows,missing} if *job==0 || *request==0 || !valid_sync_members(*guild,rows,missing))
			|| matches!(&operation, Operation::SyncReconcileMembers {job,request,guild,users} if *job==0 || *request==0 || *guild==Id(0) || !valid_member_roster(users))
		{
			return false;
		}
		let payload = match &operation {
			Operation::FindMemberDisplays {
				request, visible, ..
			} => {
				if !request.valid() || (*visible && request.slot != 1) {
					return false;
				}
				request.query.capacity() + request.users.capacity() * size_of::<Id>() + 1024 * 1024
			}
			Operation::SyncSavePage { messages, .. } => {
				if messages.len() > 50 || message_bytes(messages) > WINDOW_BYTES {
					return false;
				}
				message_bytes(messages)
			}
			Operation::SaveSyncCheckpoint(checkpoint) => {
				if checkpoint.payload.len() > 1024 * 1024 {
					return false;
				}
				checkpoint.payload.capacity()
			}
			Operation::LoadHistoryPage { page, .. }
				if page.limit == 0
					|| page.limit > 500
					|| (page.before.is_some() && page.around.is_some()) =>
			{
				return false;
			}

			// The bounded JSON columns and decoded catalog can coexist during a read.
			Operation::LoadExpressions { .. } | Operation::LoadStickerPackHints { .. } => {
				8 * 1024 * 1024
			}
			Operation::SaveExpressions(catalog) => {
				if !catalog.valid() {
					return false;
				}
				catalog.bytes() + 4 * 1024 * 1024
			}
			Operation::SaveStickerPackHints(packs) => {
				if !packs.valid() {
					return false;
				}
				packs.bytes() + 2 * 1024 * 1024
			}
			Operation::SaveServers { rows, .. } => {
				if rows.len() > 4096
					|| rows.iter().any(|row| !row.valid())
					|| rows
						.iter()
						.map(local_store::ServerDisplay::bytes)
						.sum::<usize>() > 2 * 1024 * 1024
				{
					return false;
				}
				rows.iter()
					.map(local_store::ServerDisplay::bytes)
					.sum::<usize>() + rows.capacity() * size_of::<local_store::ServerDisplay>()
			}
			Operation::SaveMembers(rows) | Operation::SyncSaveMembers { rows, .. } => {
				if rows.len() > 4096
					|| rows.iter().any(|row| !row.valid())
					|| rows.iter().map(model::MemberDisplay::bytes).sum::<usize>() > 2 * 1024 * 1024
				{
					return false;
				}
				rows.iter().map(model::MemberDisplay::bytes).sum::<usize>()
					+ rows.capacity().saturating_sub(rows.len()) * size_of::<model::MemberDisplay>()
			}
			Operation::SaveCustomFont(font) => font
				.as_ref()
				.map_or(0, |font| font.bytes().len() + font.name.capacity()),
			Operation::SaveChannel { messages, .. } | Operation::SaveChanges { messages, .. } => {
				if messages.len() > 500
					|| messages.iter().map(Message::bytes).sum::<usize>() > WINDOW_BYTES
				{
					return false;
				}
				message_bytes(messages)
			}
			Operation::SaveDraft { content, .. } => {
				if content.len() > model::message_options::MAX_DRAFT_CONTENT_BYTES {
					return false;
				}
				content.capacity()
			}
			Operation::SaveGifFavorites(favorites) => {
				if favorites.len() > 100 {
					return false;
				}
				favorites.iter().map(model::Gif::bytes).sum::<usize>()
					+ favorites.capacity() * size_of::<model::Gif>()
			}
			Operation::SaveAppPreferences(value) => {
				if !value.is_valid() {
					return false;
				}
				value.voice_input.as_ref().map_or(0, String::capacity)
					+ value.voice_output.as_ref().map_or(0, String::capacity)
					+ value.media_output.as_ref().map_or(0, String::capacity)
					+ value.expanded_folders.capacity() * size_of::<u64>()
					+ model::KeybindAction::ALL
						.into_iter()
						.map(|action| value.keybinds.chord(action).key.capacity())
						.sum::<usize>()
			}
			Operation::SaveThemeVariant(value) => value.as_ref().map_or(0, String::capacity),
			Operation::SaveAccount(account) => {
				if !account.is_valid() {
					return false;
				}
				account.heap_bytes()
			}
			_ => 0,
		};
		let ids = match &operation {
			Operation::RemoveMemberDisplays(targets) => {
				if !valid_member_display_cleanup(targets) {
					return false;
				}
				targets.capacity() * size_of::<(Id, Id)>()
			}
			Operation::SyncSaveMembers { missing, .. } => missing.capacity() * size_of::<Id>(),
			Operation::SyncReconcileMembers { users, .. } => users.capacity() * size_of::<Id>(),
			Operation::LoadServers { guilds, .. } => {
				if guilds.len() > 4096 {
					return false;
				}
				guilds.capacity() * size_of::<Id>()
			}
			Operation::SaveChanges { retained, .. } => {
				if retained.len() > 500 {
					return false;
				}
				retained.capacity() * size_of::<Id>()
			}
			Operation::DeleteMessages { ids, .. } => {
				if ids.len() > 100 {
					return false;
				}
				ids.capacity() * size_of::<Id>()
			}
			_ => 0,
		};
		// Small metadata and channel preferences fit in the reserved 8 KiB overhead.
		let Some(mut reservation) = self
			.budget
			.reserve(payload.saturating_add(ids).saturating_add(8192), false)
		else {
			return false;
		};
		if is_sync_operation(&operation) {
			let Some(sync) = self
				.sync_budget
				.reserve(payload.saturating_add(ids).saturating_add(8192), false)
			else {
				return false;
			};
			reservation._sync = Some(Box::new(sync));
		}
		let epoch = self.history.epoch();
		if is_sync_operation(&operation) && !self.history.allows(epoch) {
			return false;
		}
		if matches!(
			operation,
			Operation::LoadChannel { .. }
				| Operation::ObserveHistoryChange { .. }
				| Operation::FindMemberDisplays { .. }
				| Operation::LoadHistoryPage { .. }
				| Operation::SaveChannel { .. }
				| Operation::SaveChanges { .. }
				| Operation::LoadMembers { .. }
				| Operation::SaveMembers(_)
				| Operation::LoadExpressions { .. }
				| Operation::SaveExpressions(_)
				| Operation::LoadStickerPackHints { .. }
				| Operation::SaveStickerPackHints(_)
				| Operation::LoadServers { .. }
				| Operation::SaveServers { .. }
		) && !self.history.allows(epoch)
		{
			return false;
		}
		self.send
			.try_send((generation, account, epoch, operation, reservation))
			.is_ok()
	}
	pub fn start(ctx: egui::Context) -> Self {
		let (send, commands) = mpsc::sync_channel::<(u64, Id, u64, Operation, Reservation)>(16);
		let (events, receive) = mpsc::sync_channel(16);
		let budget = Arc::new(Budget::default());
		let history = Arc::new(HistorySafety::default());
		let worker_history = history.clone();
		let protected_history = Arc::new(Mutex::new(HistoryPins::default()));
		let worker_protected = protected_history.clone();
		std::thread::spawn(move || {
			let mut store = LocalStore::open_default();
			let disk_budget = local_store::data_dir()
				.ok()
				.map(|root| local_store::cache_budget::CacheBudget::for_root(&root));
			if let Ok(store) = &mut store {
				// Schema initialization preserves legacy layouts. Reserve the bounded
				// one-time rebuild here, on the worker, before admitting cache growth.
				let _ = prepare_cache_layout(store, disk_budget.as_ref(), &HistoryPins::default());
				if let Some(budget) = &disk_budget {
					let _ = maintain_disk_cache(store, budget, &HistoryPins::default());
				}
			}
			let results = Arc::new(Budget::default());
			loop {
				let (generation, account, epoch, operation, reservation) =
					match commands.recv_timeout(std::time::Duration::from_secs(5)) {
						Ok(command) => command,
						Err(mpsc::RecvTimeoutError::Disconnected) => break,
						Err(mpsc::RecvTimeoutError::Timeout) => {
							let protected = *worker_protected
								.lock()
								.unwrap_or_else(|error| error.into_inner());
							if let (Some(budget), Ok(store)) = (&disk_budget, &mut store)
								&& budget.needs_maintenance()
							{
								let _ = maintain_disk_cache(store, budget, &protected);
								let _ = prepare_cache_layout(store, Some(budget), &protected);
							}
							continue;
						}
					};
				let mut protected = *worker_protected
					.lock()
					.unwrap_or_else(|error| error.into_inner());
				protected.writing =
					history_write(&operation).map(|(channel, _)| (account, channel));
				if let Some(budget) = &disk_budget {
					budget.set_active_account(protected.channel.map(|scope| scope.0));
				}

				let outcome = execute_with_disk_budget(
					&mut store,
					&worker_history,
					account,
					epoch,
					operation,
					disk_budget.as_ref(),
					&protected,
				);
				if let (Some(budget), Ok(store)) = (&disk_budget, &mut store)
					&& budget.needs_maintenance()
				{
					let _ = maintain_disk_cache(store, budget, &protected);
				}
				drop(reservation);
				let bytes = outcome_bytes(&outcome);
				let reservation = results
					.reserve(bytes + 64 * 1024, true)
					.expect("bounded storage outcome fits queue");
				if events.send((generation, outcome, reservation)).is_err() {
					break;
				}
				ctx.request_repaint();
			}
		});
		Self {
			send,
			receive,
			budget,
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history,
			history,
		}
	}
}

/// Heap retained by a completed result while the UI has not consumed it.
fn outcome_bytes(outcome: &Outcome) -> usize {
	match outcome {
		Outcome::HistoryPage {
			result: Ok(messages),
			..
		} => message_bytes(messages),
		Outcome::SyncCheckpoint {
			result: Ok(Some(checkpoint)),
			..
		} => checkpoint.payload.capacity(),
		Outcome::CustomFont(Ok(Some(font))) => font.bytes().len() + font.name.capacity(),
		Outcome::Channel { messages, .. } => message_bytes(messages),
		Outcome::MemberDisplaysFound {
			request, result, ..
		} => {
			request.query.capacity()
				+ request.users.capacity() * size_of::<Id>()
				+ result.as_ref().map_or(0, |rows| {
					rows.iter().map(model::MemberDisplay::bytes).sum::<usize>()
						+ rows.capacity().saturating_sub(rows.len())
							* size_of::<model::MemberDisplay>()
				})
		}
		Outcome::Members { result, .. } => result.as_ref().map_or(0, |rows| {
			rows.iter().map(model::MemberDisplay::bytes).sum::<usize>()
				+ rows.capacity().saturating_sub(rows.len()) * size_of::<model::MemberDisplay>()
		}),
		Outcome::Expressions { result, .. } => result
			.as_ref()
			.ok()
			.and_then(Option::as_ref)
			.map_or(0, model::ExpressionCatalog::bytes),
		Outcome::StickerPackHints { result, .. } => result
			.as_ref()
			.ok()
			.and_then(Option::as_ref)
			.map_or(0, model::StickerPackHints::bytes),
		Outcome::Servers { result, .. } => result.as_ref().map_or(0, |rows| {
			rows.iter()
				.map(local_store::ServerDisplay::bytes)
				.sum::<usize>()
				+ rows.capacity() * size_of::<local_store::ServerDisplay>()
		}),
		Outcome::ServersSaved { ids, .. } => ids.capacity() * size_of::<Id>(),
		Outcome::Drafts(drafts) => drafts.values().map(String::capacity).sum(),
		Outcome::GifFavorites(favorites) => {
			favorites.iter().map(model::Gif::bytes).sum::<usize>()
				+ favorites.capacity() * size_of::<model::Gif>()
		}
		Outcome::Accounts { roster, pruned } => {
			roster.as_ref().map_or(0, |accounts| {
				accounts
					.iter()
					.map(model::SavedAccount::heap_bytes)
					.sum::<usize>() + accounts.capacity() * size_of::<model::SavedAccount>()
			}) + pruned.capacity() * size_of::<Id>()
		}
		_ => 0,
	}
}

fn maintain_disk_cache(
	store: &mut LocalStore,
	budget: &Arc<local_store::cache_budget::CacheBudget>,
	protected: &HistoryPins,
) -> std::io::Result<local_store::cache_budget::CacheUsage> {
	budget.maintain(|stage, wanted| reclaim_disk_cache(store, stage, wanted, protected))
}

fn reclaim_disk_cache(
	store: &mut LocalStore,
	stage: local_store::cache_budget::ReclaimStage,
	wanted: u64,
	protected: &HistoryPins,
) -> std::io::Result<()> {
	use local_store::cache_budget::ReclaimStage;
	if wanted == 0 {
		return Ok(());
	}
	let limit = ((wanted / 256).max(1) as usize).min(4096);
	let cutoff = history_cutoff();
	let pins = history_pins(protected);
	let before = store
		.history_cache_stats()
		.map_err(|_| std::io::Error::other("Cache cleanup could not measure database pages"))?
		.database_bytes;
	for _ in 0..32 {
		let reclaimed = match stage {
			ReclaimStage::ExpiredHistory => {
				store.evict_history_before_protected(cutoff, limit, &pins)
			}
			ReclaimStage::RecentHistory => store.evict_oldest_history(limit, &pins),
			ReclaimStage::PresentationHints => store
				.evict_presentation_hints(limit, &protected.guild.into_iter().collect::<Vec<_>>()),
		}
		.map_err(|_| std::io::Error::other("Cache cleanup could not remove old records"))?;
		if reclaimed == 0 {
			break;
		}
		store
			.reclaim_history_space()
			.map_err(|_| std::io::Error::other("Cache cleanup could not reclaim database pages"))?;
		let remaining = store
			.history_cache_stats()
			.map_err(|_| std::io::Error::other("Cache cleanup could not measure database pages"))?
			.database_bytes;
		if before.saturating_sub(remaining) >= wanted {
			break;
		}
	}
	Ok(())
}

/// Reserve pessimistic database and journal growth before optional cache writes.
fn cache_write_reservation(operation: &Operation) -> Option<u64> {
	let bytes = match operation {
		Operation::SyncSavePage { messages, .. }
		| Operation::SaveChannel { messages, .. }
		| Operation::SaveChanges { messages, .. } => message_bytes(messages),
		Operation::SyncSaveMembers { rows, missing, .. } if !missing.is_empty() => {
			return Some(
				(rows.iter().map(model::MemberDisplay::bytes).sum::<usize>() as u64)
					.saturating_mul(16)
					.saturating_add(64 * 1024 * 1024),
			);
		}
		Operation::SyncSaveMembers { rows, .. } | Operation::SaveMembers(rows) => {
			rows.iter().map(model::MemberDisplay::bytes).sum()
		}
		Operation::SaveExpressions(rows) => rows.bytes(),
		Operation::SaveStickerPackHints(rows) => rows.bytes(),
		Operation::SaveServers { rows, .. } => {
			rows.iter().map(local_store::ServerDisplay::bytes).sum()
		}
		Operation::SaveSyncCheckpoint(checkpoint) => checkpoint.payload.len(),
		Operation::SaveDraft { content, .. } => content.len(),
		Operation::SaveGifFavorites(rows) => rows.iter().map(model::Gif::bytes).sum(),
		Operation::SaveCustomFont(font) => {
			return Some(
				font.as_ref()
					.map_or(0, |font| font.bytes().len() as u64)
					.saturating_mul(4)
					.saturating_add(64 * 1024 * 1024),
			);
		}
		Operation::SyncReconcileMembers { .. }
		| Operation::DeleteMessages { .. }
		| Operation::ClearHistory
		| Operation::ClearChannel { .. }
		| Operation::RemoveServer { .. }
		| Operation::RemoveMember { .. }
		| Operation::RemoveMemberDisplays(_)
		| Operation::ClearExpressions { .. }
		| Operation::Forget => return Some(64 * 1024 * 1024),
		Operation::SaveAppPreferences(_)
		| Operation::SaveAppearance(_)
		| Operation::SaveThemeVariant(_)
		| Operation::SaveReadingPreferences(_)
		| Operation::SaveGameActivity(_)
		| Operation::SaveMinimizeToTray(_)
		| Operation::SaveChannelPreferences(_)
		| Operation::SaveAccountPresence(_)
		| Operation::SaveAccount(_)
		| Operation::SetAccountToken { .. }
		| Operation::ObserveHistoryChange { .. }
		| Operation::ClearSyncCheckpoint => 64 * 1024,
		_ => return None,
	};
	// JSON escaping, indexes and WAL may coexist with final SQLite pages. Personal
	// records receive admission too; they remain intact if only their write must wait.
	Some(
		(bytes as u64)
			.saturating_mul(16)
			.saturating_add(1024 * 1024),
	)
}

fn history_cutoff() -> Id {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap_or_default()
		.as_millis();
	let relative = now
		.saturating_sub(21 * 86_400_000)
		.saturating_sub(1_420_070_400_000);
	Id((relative.min((u64::MAX >> 22) as u128) as u64) << 22)
}
fn history_pins(protected: &HistoryPins) -> Vec<(Id, Id)> {
	let mut pins: Vec<_> = [protected.channel, protected.writing]
		.into_iter()
		.chain(protected.sync)
		.flatten()
		.collect();
	pins.sort_unstable();
	pins.dedup();
	pins
}
fn history_write(operation: &Operation) -> Option<(Id, &[Message])> {
	match operation {
		Operation::SyncSavePage {
			channel, messages, ..
		}
		| Operation::SaveChannel {
			channel, messages, ..
		}
		| Operation::SaveChanges {
			channel, messages, ..
		} => Some((*channel, messages)),
		_ => None,
	}
}
/// At the item ceiling, real growth evicts old pages even when byte usage is low.
/// Exact-ID checks keep foreground edits and unchanged pages writable at capacity.
fn history_growth(
	store: &LocalStore,
	account: Id,
	channel: Id,
	messages: &[Message],
	limit: u64,
) -> Result<usize, StoreError> {
	let count = store.history_cache_stats()?.messages;
	if count.saturating_add(messages.len() as u64) <= limit {
		return Ok(0);
	}
	let ids: Vec<_> = messages.iter().map(|message| message.id).collect();
	let missing = store.missing_history_messages(account, channel, &ids)?;
	Ok(if count.saturating_add(missing as u64) > limit {
		missing
	} else {
		0
	})
}
fn ensure_history_items(
	store: &mut LocalStore,
	account: Id,
	channel: Id,
	messages: &[Message],
	limit: u64,
	protected: &HistoryPins,
) -> Result<(), StoreError> {
	let incoming = history_growth(store, account, channel, messages, limit)?;
	if incoming == 0 {
		return Ok(());
	}
	let cutoff = history_cutoff();
	let pins = history_pins(protected);
	let slack = (limit / 1000).clamp(1, 1000);
	let target = limit.saturating_sub(incoming as u64).saturating_sub(slack);
	for expired in [true, false] {
		for _ in 0..32 {
			let count = store.history_cache_stats()?.messages;
			if count <= target {
				return Ok(());
			}
			let victims = (count - target).min(4096) as usize;
			let removed = if expired {
				store.evict_history_before_protected(cutoff, victims, &pins)?
			} else {
				store.evict_oldest_history(victims, &pins)?
			};
			if removed == 0 {
				break;
			}
			store.reclaim_history_space()?;
		}
	}
	if store
		.history_cache_stats()?
		.messages
		.saturating_add(incoming as u64)
		<= limit
	{
		Ok(())
	} else {
		Err(StoreError::Capacity)
	}
}

/// Existing personal data remains readable if the optional cache layout rebuild
/// cannot reserve its temporary files. Cleanup never calls this helper recursively.
fn prepare_cache_layout(
	store: &mut LocalStore,
	budget: Option<&Arc<local_store::cache_budget::CacheBudget>>,
	protected: &HistoryPins,
) -> Result<(), StoreError> {
	let Some(bytes) = store.cache_layout_migration_bytes()? else {
		return Ok(());
	};
	let budget = budget.ok_or(StoreError::Capacity)?;
	let reservation = budget
		.reserve_with_reclaim(bytes, |stage, wanted| {
			reclaim_disk_cache(store, stage, wanted, protected)
		})
		.map_err(|_| StoreError::Capacity)?;
	let result = store.migrate_cache_layout();
	drop(reservation);
	let refreshed = budget
		.refresh_database()
		.map_err(|_| StoreError::Unavailable);
	result?;
	refreshed?;
	Ok(())
}
fn cache_growth_needs_layout(operation: &Operation) -> bool {
	matches!(
		operation,
		Operation::SyncSavePage { .. }
			| Operation::SaveChannel { .. }
			| Operation::SaveChanges { .. }
			| Operation::SyncSaveMembers { .. }
			| Operation::SaveMembers(_)
			| Operation::SaveExpressions(_)
			| Operation::SaveStickerPackHints(_)
			| Operation::SaveServers { .. }
			| Operation::SaveSyncCheckpoint(_)
	)
}

fn execute_with_disk_budget(
	store: &mut Result<LocalStore, StoreError>,
	history: &HistorySafety,
	account: Id,
	epoch: u64,
	operation: Operation,
	budget: Option<&Arc<local_store::cache_budget::CacheBudget>>,
	protected: &HistoryPins,
) -> Outcome {
	if !history.allows(epoch)
		&& (is_sync_operation(&operation)
			|| matches!(
				&operation,
				Operation::SaveChannel { .. }
					| Operation::SaveChanges { .. }
					| Operation::SaveMembers(_)
					| Operation::SaveExpressions(_)
					| Operation::SaveStickerPackHints(_)
					| Operation::SaveServers { .. }
					| Operation::ObserveHistoryChange { .. }
			)) {
		return execute(store, history, account, epoch, operation);
	}

	let mut protected = *protected;
	protected.writing = history_write(&operation).map(|(channel, _)| (account, channel));
	if cache_growth_needs_layout(&operation)
		&& let Ok(store) = &mut *store
		&& let Err(error) = prepare_cache_layout(store, budget, &protected)
	{
		return execute(&mut Err(error), history, account, epoch, operation);
	}
	let item_pressure = match (history_write(&operation), store.as_ref()) {
		(Some((channel, messages)), Ok(store)) => match history_growth(
			store,
			account,
			channel,
			messages,
			local_store::MAX_CACHED_MESSAGES as u64,
		) {
			Ok(growth) => growth > 0,
			Err(error) => return execute(&mut Err(error), history, account, epoch, operation),
		},
		_ => false,
	};
	let required = cache_write_reservation(&operation).map(|bytes| {
		if item_pressure {
			bytes.max(64 * 1024 * 1024)
		} else {
			bytes
		}
	});
	let reservation =
		if let (Some(bytes), Some(budget), Ok(store)) = (required, budget, &mut *store) {
			match budget.reserve_with_reclaim(bytes, |stage, wanted| {
				reclaim_disk_cache(store, stage, wanted, &protected)
			}) {
				Ok(reservation) => Some(reservation),
				Err(_) => {
					return execute(
						&mut Err(StoreError::Capacity),
						history,
						account,
						epoch,
						operation,
					);
				}
			}
		} else {
			None
		};
	if item_pressure
		&& let (Some((channel, messages)), Ok(store)) = (history_write(&operation), &mut *store)
		&& let Err(error) = ensure_history_items(
			store,
			account,
			channel,
			messages,
			local_store::MAX_CACHED_MESSAGES as u64,
			&protected,
		) {
		return execute(&mut Err(error), history, account, epoch, operation);
	}
	let outcome = execute(store, history, account, epoch, operation);
	drop(reservation);
	if let Some(budget) = budget {
		let _ = budget.refresh_database();
	}
	outcome
}

fn valid_history_scope(channel: Id, guild: Option<Id>, parent: Option<Id>) -> bool {
	channel.0 != 0
		&& guild.is_none_or(|guild| guild.0 != 0)
		&& parent.is_none_or(|parent| parent.0 != 0 && parent != channel && guild.is_some())
}

fn is_sync_operation(operation: &Operation) -> bool {
	matches!(
		operation,
		Operation::SyncReadRevision { .. }
			| Operation::SyncSaveMembers { .. }
			| Operation::SyncReconcileMembers { .. }
			| Operation::SyncSavePage { .. }
			| Operation::LoadSyncCheckpoint
			| Operation::SaveSyncCheckpoint(_)
			| Operation::ClearSyncCheckpoint
	)
}

fn valid_member_roster(users: &[Id]) -> bool {
	users.len() <= 250_000
		&& users.iter().all(|user| user.0 != 0)
		&& users.windows(2).all(|pair| pair[0] < pair[1])
}
fn valid_sync_members(guild: Id, rows: &[model::MemberDisplay], missing: &[Id]) -> bool {
	if guild.0 == 0
		|| rows.len() > 4096
		|| rows.iter().map(model::MemberDisplay::bytes).sum::<usize>() > 2 * 1024 * 1024
		|| missing.len() > 100
		|| missing.iter().any(|user| user.0 == 0)
		|| missing.windows(2).any(|pair| pair[0] >= pair[1])
	{
		return false;
	}
	let mut seen = BTreeSet::new();
	rows.iter().all(|row| {
		row.guild == guild
			&& row.valid()
			&& missing.binary_search(&row.user).is_err()
			&& seen.insert(row.user)
	})
}
fn sync_member_store<'a>(
	store: &'a mut Result<LocalStore, StoreError>,
	history: &HistorySafety,
	account: Id,
	epoch: u64,
	job: u64,
	guild: Id,
) -> Result<&'a mut LocalStore, StoreError> {
	let store = sync_store(store, history, epoch)?;
	let checkpoint = store
		.load_sync_checkpoint(account)?
		.ok_or(StoreError::Unavailable)?;
	if guild.0 == 0 || checkpoint.job != job || checkpoint.scope.is_some_and(|scope| scope != guild)
	{
		return Err(StoreError::Unavailable);
	}
	Ok(store)
}

fn sync_store<'a>(
	store: &'a mut Result<LocalStore, StoreError>,
	history: &HistorySafety,
	epoch: u64,
) -> Result<&'a mut LocalStore, StoreError> {
	if history.allows(epoch) {
		store.as_mut().map_err(|error| *error)
	} else {
		Err(StoreError::Unavailable)
	}
}

/// Background results retain their own correlation and failures; they never become timeline events.
fn execute_sync(
	store: &mut Result<LocalStore, StoreError>,
	history: &HistorySafety,
	account: Id,
	epoch: u64,
	operation: Operation,
) -> Result<Operation, Outcome> {
	let outcome = match operation {
		Operation::FindMemberDisplays {
			request,
			revision,
			visible,
		} => {
			let result = sync_store(store, history, epoch).and_then(|store| {
				if !request.valid() || (visible && request.slot != 1) {
					return Err(StoreError::Capacity);
				}
				if request.slot == 1 {
					return store.load_member_displays_for_users(
						account,
						request.guild,
						&request.users,
					);
				}
				if let Ok(user) = request.query.parse::<Id>() {
					return store.load_member_displays_for_users(account, request.guild, &[user]);
				}
				store.search_member_displays(account, request.guild, &request.query, 100)
			});
			Outcome::MemberDisplaysFound {
				request,
				revision,
				visible,
				epoch,
				result,
			}
		}
		Operation::SyncSaveMembers {
			job,
			request,
			guild,
			rows,
			missing,
		} => Outcome::SyncMembersSaved {
			job,
			request,
			guild,
			epoch,
			result: if request == 0 || !valid_sync_members(guild, &rows, &missing) {
				Err(StoreError::Capacity)
			} else {
				sync_member_store(store, history, account, epoch, job, guild).and_then(|store| {
					store.remove_member_displays(account, guild, &missing)?;
					store.save_member_displays(account, &rows)
				})
			},
		},
		Operation::SyncReconcileMembers {
			job,
			request,
			guild,
			users,
		} => Outcome::SyncMembersReconciled {
			job,
			request,
			guild,
			epoch,
			result: if request == 0 || !valid_member_roster(&users) {
				Err(StoreError::Capacity)
			} else {
				sync_member_store(store, history, account, epoch, job, guild)
					.and_then(|store| store.reconcile_member_displays(account, guild, &users))
			},
		},
		Operation::SyncReadRevision {
			job,
			request,
			channel,
		} => Outcome::SyncRevision {
			job,
			request,
			channel,
			epoch,
			result: sync_store(store, history, epoch)
				.and_then(|store| store.history_revision(account, channel)),
		},
		Operation::SyncSavePage {
			job,
			request,
			channel,
			guild,
			parent,
			messages,
			coverage,
		} => Outcome::SyncPageSaved {
			job,
			request,
			channel,
			epoch,
			result: sync_store(store, history, epoch).and_then(|store| {
				if coverage.job != job
					|| coverage.request != request
					|| !valid_history_scope(channel, guild, parent)
				{
					return Err(StoreError::Capacity);
				}
				store.save_sync_page_scoped(account, channel, &messages, &coverage, guild, parent)
			}),
		},
		Operation::LoadSyncCheckpoint => Outcome::SyncCheckpoint {
			epoch,
			result: sync_store(store, history, epoch)
				.and_then(|store| store.load_sync_checkpoint(account)),
		},
		Operation::SaveSyncCheckpoint(checkpoint) => Outcome::SyncCheckpointSaved {
			epoch,
			result: sync_store(store, history, epoch)
				.and_then(|store| store.save_sync_checkpoint(account, &checkpoint)),
		},
		Operation::ClearSyncCheckpoint => Outcome::SyncCheckpointSaved {
			epoch,
			result: store
				.as_mut()
				.map_err(|error| *error)
				.and_then(|store| store.clear_sync_checkpoint(account)),
		},
		Operation::LoadHistoryPage {
			channel,
			request,
			page,
		} => Outcome::HistoryPage {
			channel,
			request,
			epoch,
			result: sync_store(store, history, epoch)
				.and_then(|store| store.load_history_page(account, channel, page)),
		},
		Operation::ObserveHistoryChange { channel, message } => {
			match sync_store(store, history, epoch)
				.and_then(|store| store.observe_history_change(account, channel, message, false))
			{
				Ok(_) => Outcome::Saved,
				Err(error) => {
					history.fail();
					Outcome::Failed {
						error,
						message: "Could not invalidate changed cached messages; history cache disabled until restart",
						draft_restore: false,
						history_cleanup: true,
					}
				}
			}
		}
		Operation::CacheUsage => Outcome::CacheUsage(local_store::data_dir().and_then(|root| {
			local_store::cache_budget::CacheBudget::for_root(&root)
				.refresh_database()
				.map_err(|_| StoreError::Unavailable)
		})),
		operation => return Ok(operation),
	};
	Err(outcome)
}

fn execute(
	store: &mut Result<LocalStore, StoreError>,
	history: &HistorySafety,
	account: Id,
	epoch: u64,
	operation: Operation,
) -> Outcome {
	let operation = match execute_sync(store, history, account, epoch, operation) {
		Ok(operation) => operation,
		Err(outcome) => return outcome,
	};
	// Settings completions have their own pending/error state, independent of history.
	match &operation {
		Operation::LoadCustomFont => {
			return Outcome::CustomFont((|| {
				let store = store
					.as_ref()
					.map_err(|_| "Could not load the saved font.")?;
				store
					.custom_font()
					.map_err(|_| "Could not load the saved font.")?
					.map(|(name, bytes)| ui::fonts::CustomFont::new(name, bytes))
					.transpose()
			})());
		}
		Operation::SaveCustomFont(font) => {
			return Outcome::CustomFont((|| {
				let store = store
					.as_ref()
					.map_err(|_| "Could not save the font. Try importing it again.")?;
				store
					.save_custom_font(font.as_ref().map(|font| (font.name.as_str(), font.bytes())))
					.map_err(|_| "Could not save the font. Try again.")?;
				Ok(font.clone())
			})());
		}
		Operation::LoadChannelPreferences => {
			return Outcome::ChannelPreferences(match store {
				Ok(store) => store.channel_preferences(account),
				Err(error) => Err(*error),
			});
		}
		Operation::SaveChannelPreferences(value) => {
			return Outcome::ChannelPreferencesSaved(match store {
				Ok(store) => store.save_channel_preferences(account, value),
				Err(error) => Err(*error),
			});
		}
		Operation::LoadAccountPresences => {
			return Outcome::AccountPresences(match store {
				Ok(store) => store.account_presences(),
				Err(error) => Err(*error),
			});
		}
		Operation::SaveAccountPresence(presence) => {
			return Outcome::AccountPresenceSaved(match store {
				Ok(store) => store.save_account_presence(account, presence),
				Err(error) => Err(*error),
			});
		}
		Operation::LoadAccounts => {
			return Outcome::Accounts {
				roster: match store {
					Ok(store) => store.accounts(),
					Err(error) => Err(*error),
				},
				pruned: Vec::new(),
			};
		}
		Operation::SaveAccount(account) => {
			let (roster, pruned) = match store {
				Ok(store) => match store.save_account(account) {
					// Report the committed pruning even when the re-read fails.
					Ok(pruned) => (store.accounts(), pruned),
					Err(error) => (Err(error), Vec::new()),
				},
				Err(error) => (Err(*error), Vec::new()),
			};
			return Outcome::Accounts { roster, pruned };
		}
		Operation::SetAccountToken {
			account: id,
			has_token,
		} => {
			return Outcome::Accounts {
				roster: match store {
					Ok(store) => store
						.set_account_token(*id, *has_token)
						.and(store.accounts()),
					Err(error) => Err(*error),
				},
				pruned: Vec::new(),
			};
		}
		Operation::SaveAppPreferences(value) => {
			return Outcome::AppPreferencesSaved(match store {
				Ok(store) => store.save_app_preferences(value),
				Err(error) => Err(*error),
			});
		}
		Operation::LoadMinimizeToTray => {
			return Outcome::MinimizeToTray(match store {
				Ok(store) => store.minimize_to_tray(),
				Err(error) => Err(*error),
			});
		}
		Operation::SaveMinimizeToTray(value) => {
			return Outcome::MinimizeToTraySaved(match store {
				Ok(store) => store.save_minimize_to_tray(*value),
				Err(error) => Err(*error),
			});
		}
		Operation::LoadGameActivity => {
			return Outcome::GameActivity(match store {
				Ok(store) => store.game_activity_enabled(),
				Err(error) => Err(*error),
			});
		}
		Operation::SaveGameActivity(value) => {
			return Outcome::GameActivitySaved(match store {
				Ok(store) => store.save_game_activity_enabled(*value),
				Err(error) => Err(*error),
			});
		}
		Operation::LoadReadingPreferences => {
			return Outcome::ReadingPreferences(match store {
				Ok(store) => store.reading_preferences(),
				Err(error) => Err(*error),
			});
		}
		Operation::SaveReadingPreferences(value) => {
			return Outcome::ReadingPreferencesSaved(match store {
				Ok(store) => store.save_reading_preferences(*value),
				Err(error) => Err(*error),
			});
		}
		_ => {}
	}
	if let Operation::LoadServers { revision, guilds } = operation {
		return Outcome::Servers {
			revision,
			epoch,
			result: if history.allows(epoch) {
				match store {
					Ok(store) => store.load_server_displays_for(account, &guilds),
					Err(error) => Err(*error),
				}
			} else {
				Ok(Vec::new())
			},
		};
	}
	if let Operation::SaveServers { rows, revision } = operation {
		let ids = rows.iter().map(|row| row.guild).collect();
		return Outcome::ServersSaved {
			revision,
			epoch,
			ids,
			result: if history.allows(epoch) {
				Some(match store {
					Ok(store) => store.save_server_displays(account, &rows),
					Err(error) => Err(*error),
				})
			} else {
				None
			},
		};
	}
	if let Operation::LoadMembers { guild, revision } = operation {
		return Outcome::Members {
			guild,
			revision,
			epoch,
			result: if history.allows(epoch) {
				match store {
					Ok(store) => store.load_recent_member_displays(account, guild),
					Err(error) => Err(*error),
				}
			} else {
				Ok(Vec::new())
			},
		};
	}
	if let Operation::LoadExpressions { guild, revision } = operation {
		return Outcome::Expressions {
			account,
			guild,
			revision,
			epoch,
			result: if history.allows(epoch) {
				match store {
					Ok(store) => store.load_expression_catalog(account, guild),
					Err(error) => Err(*error),
				}
			} else {
				Ok(None)
			},
		};
	}
	if let Operation::LoadStickerPackHints { revision } = operation {
		return Outcome::StickerPackHints {
			account,
			revision,
			epoch,
			result: if history.allows(epoch) {
				match store {
					Ok(store) => store.load_sticker_pack_hints(account),
					Err(error) => Err(*error),
				}
			} else {
				Ok(None)
			},
		};
	}
	if matches!(
		operation,
		Operation::LoadChannel { .. }
			| Operation::SaveChannel { .. }
			| Operation::SaveChanges { .. }
			| Operation::SaveMembers(_)
			| Operation::SaveExpressions(_)
			| Operation::SaveStickerPackHints(_)
	) && !history.allows(epoch)
	{
		return Outcome::Saved;
	}
	let draft_restore = matches!(operation, Operation::LoadDrafts);
	let history_cleanup = matches!(operation, Operation::ClearHistory);
	let scoped_cleanup = matches!(
		operation,
		Operation::ClearChannel { .. }
			| Operation::RemoveMember { .. }
			| Operation::RemoveMemberDisplays(_)
			| Operation::ClearExpressions { .. }
			| Operation::RemoveServer { .. }
	);
	let history_unsafe = matches!(
		operation,
		Operation::DeleteMessages { .. }
			| Operation::ClearHistory
			| Operation::Forget
			| Operation::ClearChannel { .. }
			| Operation::RemoveMember { .. }
			| Operation::RemoveMemberDisplays(_)
			| Operation::ClearExpressions { .. }
			| Operation::RemoveServer { .. }
	);
	let failure_message = match &operation {
		Operation::LoadAppearance => "Could not load saved appearance; using system theme",
		Operation::SaveAppearance(_) | Operation::SaveThemeVariant(_) => {
			"Could not save appearance; change exists only in this session"
		}
		Operation::Forget => {
			"Could not remove local account data; history and drafts may remain on disk"
		}
		Operation::ClearHistory => {
			"Could not clear cached history; history cache disabled until restart; messages may remain on disk"
		}
		Operation::ClearChannel { .. }
		| Operation::RemoveMember { .. }
		| Operation::RemoveMemberDisplays(_)
		| Operation::ClearExpressions { .. } => {
			"Could not remove inaccessible cached data; history cache disabled until restart"
		}
		Operation::SaveMembers(_) => {
			"Could not save member names; current names remain available in this session"
		}
		Operation::SaveExpressions(_) => {
			"Could not save cached emoji and stickers; live catalogs remain available"
		}
		Operation::SaveStickerPackHints(_) => {
			"Could not save cached sticker packs; live packs remain available"
		}
		Operation::RemoveServer { .. } => "Could not remove cached server data",
		Operation::DeleteMessages { .. } => {
			"Could not remove deleted cached messages; history cache disabled until restart; messages may remain on disk"
		}
		Operation::SaveDraft { .. } => {
			"Could not save a draft; latest text may exist only in memory"
		}
		Operation::SaveChannel { .. } | Operation::SaveChanges { .. } => {
			"Could not save cached history"
		}
		Operation::LoadDrafts => "Could not restore drafts from local storage",
		Operation::LoadGifFavorites => "Could not restore GIF favorites from local storage",
		Operation::SaveGifFavorites(_) => {
			"Could not save GIF favorites; the change exists only in this session"
		}
		Operation::LoadChannel { .. } => "Could not read cached history",
		Operation::SyncReadRevision { .. }
		| Operation::FindMemberDisplays { .. }
		| Operation::SyncSaveMembers { .. }
		| Operation::SyncReconcileMembers { .. }
		| Operation::SyncSavePage { .. }
		| Operation::LoadSyncCheckpoint
		| Operation::SaveSyncCheckpoint(_)
		| Operation::ClearSyncCheckpoint
		| Operation::LoadHistoryPage { .. }
		| Operation::CacheUsage
		| Operation::ObserveHistoryChange { .. }
		| Operation::LoadCustomFont
		| Operation::LoadMembers { .. }
		| Operation::LoadExpressions { .. }
		| Operation::LoadStickerPackHints { .. }
		| Operation::LoadServers { .. }
		| Operation::SaveServers { .. }
		| Operation::SaveCustomFont(_)
		| Operation::LoadAccounts
		| Operation::SaveAccount(_)
		| Operation::SetAccountToken { .. }
		| Operation::LoadChannelPreferences
		| Operation::SaveChannelPreferences(_)
		| Operation::SaveAppPreferences(_)
		| Operation::LoadReadingPreferences
		| Operation::SaveReadingPreferences(_)
		| Operation::LoadGameActivity
		| Operation::SaveGameActivity(_)
		| Operation::LoadMinimizeToTray
		| Operation::SaveMinimizeToTray(_)
		| Operation::LoadAccountPresences
		| Operation::SaveAccountPresence(_) => unreachable!(),
	};
	let result = match store {
		Ok(store) => match operation {
			Operation::SyncReadRevision { .. }
			| Operation::FindMemberDisplays { .. }
			| Operation::SyncSaveMembers { .. }
			| Operation::SyncReconcileMembers { .. }
			| Operation::SyncSavePage { .. }
			| Operation::LoadSyncCheckpoint
			| Operation::SaveSyncCheckpoint(_)
			| Operation::ClearSyncCheckpoint
			| Operation::LoadHistoryPage { .. }
			| Operation::CacheUsage
			| Operation::ObserveHistoryChange { .. }
			| Operation::LoadCustomFont
			| Operation::LoadMembers { .. }
			| Operation::LoadExpressions { .. }
			| Operation::LoadStickerPackHints { .. }
			| Operation::LoadServers { .. }
			| Operation::SaveServers { .. }
			| Operation::SaveCustomFont(_)
			| Operation::LoadAccounts
			| Operation::SaveAccount(_)
			| Operation::SetAccountToken { .. }
			| Operation::LoadChannelPreferences
			| Operation::SaveChannelPreferences(_)
			| Operation::SaveAppPreferences(_)
			| Operation::LoadReadingPreferences
			| Operation::SaveReadingPreferences(_)
			| Operation::LoadGameActivity
			| Operation::SaveGameActivity(_)
			| Operation::LoadMinimizeToTray
			| Operation::SaveMinimizeToTray(_)
			| Operation::LoadAccountPresences
			| Operation::SaveAccountPresence(_) => {
				unreachable!()
			}
			Operation::LoadAppearance => store
				.appearance()
				.and_then(|appearance| Ok(Outcome::Appearance(appearance, store.theme_variant()?))),
			Operation::SaveAppearance(appearance) => {
				store.save_appearance(appearance).map(|_| Outcome::Saved)
			}
			Operation::SaveThemeVariant(variant) => store
				.save_theme_variant(variant.as_deref())
				.map(|_| Outcome::Saved),
			Operation::LoadDrafts => store.load_drafts(account).map(Outcome::Drafts),
			Operation::LoadGifFavorites => store.gif_favorites(account).map(Outcome::GifFavorites),
			Operation::SaveGifFavorites(favorites) => store
				.save_gif_favorites(account, &favorites)
				.map(|_| Outcome::Saved),
			Operation::LoadChannel { channel, request } => {
				store.load_channel(account, channel).and_then(|messages| {
					Ok(Outcome::Channel {
						channel,
						request,
						start: store.history_start(account, channel, &messages)?,
						messages,
						epoch,
					})
				})
			}
			Operation::SaveMembers(rows) => store
				.save_member_displays(account, &rows)
				.map(|_| Outcome::Saved),
			Operation::SaveExpressions(catalog) => store
				.save_expression_catalog(account, &catalog)
				.map(|_| Outcome::Saved),
			Operation::SaveStickerPackHints(packs) => store
				.save_sticker_pack_hints(account, &packs)
				.map(|_| Outcome::Saved),
			Operation::ClearExpressions { guild, emoji } => match emoji {
				Some(emoji) => store.clear_expression_field(account, guild, emoji),
				None => store.clear_expression_guild(account, guild),
			}
			.map(|_| Outcome::Saved),
			Operation::RemoveServer { guild } => store
				.clear_server_display(account, guild)
				.map(|_| Outcome::Saved),
			Operation::RemoveMember { guild, user } => match user {
				Some(user) => store.remove_member_display(account, guild, user),
				None => store
					.clear_member_guild(account, guild)
					.and_then(|()| store.clear_expression_guild(account, guild)),
			}
			.map(|_| Outcome::Saved),
			Operation::RemoveMemberDisplays(targets) => {
				remove_member_display_targets(store, account, &targets).map(|_| Outcome::Saved)
			}
			Operation::ClearChannel { channel } => store
				.clear_channel_history(account, channel)
				.map(|_| Outcome::Saved),
			Operation::SaveDraft { channel, content } => store
				.save_draft(account, channel, &content)
				.map(|_| Outcome::Saved),
			Operation::SaveChanges {
				channel,
				messages,
				retained,
				start,
			} => store
				.save_changes(account, channel, &messages, &retained, start)
				.map(|_| Outcome::Saved),
			Operation::SaveChannel {
				channel,
				messages,
				start,
			} => store
				.save_channel(account, channel, &messages, start)
				.map(|_| Outcome::Saved),
			Operation::DeleteMessages { channel, ids } => store
				.delete_messages(account, channel, &ids)
				.map(|_| Outcome::Saved),
			Operation::ClearHistory => store
				.clear_history(account)
				.map(|_| Outcome::HistoryCleared),
			Operation::Forget => store.forget_account(account).map(|_| Outcome::Saved),
		},
		Err(error) => Err(*error),
	};
	result.unwrap_or_else(|error| {
		if history_unsafe {
			history.fail();
		}
		if scoped_cleanup {
			return Outcome::ScopedCleanupFailed { account };
		}
		Outcome::Failed {
			error,
			message: failure_message,
			draft_restore,
			history_cleanup,
		}
	})
}

/// Offline restart check; uses only synthetic device preferences and opens no audio devices.
#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_voice_preferences_check() {
	let root = std::env::temp_dir().join(format!(
		"serein-voice-preferences-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_nanos()
	));
	std::fs::create_dir(&root).unwrap();
	let path = root.join("client.sqlite3");
	let (send, commands) = mpsc::sync_channel(16);
	let (_, receive) = mpsc::sync_channel(16);
	let cache = Cache {
		send,
		receive,
		budget: Arc::new(Budget::default()),
		sync_budget: Arc::new(Budget {
			limit: SYNC_QUEUE_BYTES,
			..Budget::default()
		}),
		protected_history: Arc::new(Mutex::new(HistoryPins::default())),
		history: Arc::new(HistorySafety::default()),
	};
	let mut settings =
		crate::app_settings::Settings::from_preferences(Ok(local_store::AppPreferences::default()));
	let mut view = ui::MessagingUi::default();
	settings.apply(&mut view);
	view.set_voice_user_volume_overrides(&[(7, 35), (9, 150)]);
	view.set_voice_user_mutes(&[9]);
	settings.observe(&view);
	{
		let mut store = Ok(local_store::LocalStore::open(&path).unwrap());
		assert!(settings.save(Some(&cache), 0));
		let (_, account, epoch, operation, _reservation) = commands.try_recv().unwrap();
		assert!(matches!(
			execute(&mut store, &cache.history, account, epoch, operation),
			Outcome::AppPreferencesSaved(Ok(()))
		));
	}
	let mut restored = crate::app_settings::Settings::from_preferences(
		local_store::LocalStore::open(&path)
			.unwrap()
			.app_preferences(),
	);
	let mut restarted = ui::MessagingUi::default();
	restored.apply(&mut restarted);
	assert!(restored.loaded);
	assert!(restarted.voice_user_volumes().contains(&(7, 35)));
	assert!(restarted.voice_user_volumes().contains(&(9, 0)));
	assert_eq!(
		restarted.voice_user_volume_overrides(),
		vec![(7, 35), (9, 150)]
	);
	restored.observe(&restarted);
	assert!(!restored.state.touched && !restored.state.dirty);
	restarted.notifications_enabled = !restarted.notifications_enabled;
	restored.observe(&restarted);
	assert_eq!(restored.current.user_volumes, vec![(7, 35), (9, 150)]);
	assert_eq!(restored.current.muted_users, vec![9]);
	let mut failed =
		crate::app_settings::Settings::from_preferences(Err(local_store::StoreError::Unavailable));
	failed.apply(&mut restarted);
	restarted.notifications_enabled = !restarted.notifications_enabled;
	failed.observe(&restarted);
	assert!(!failed.save(Some(&cache), 0));
	assert!(matches!(
		commands.try_recv(),
		Err(mpsc::TryRecvError::Empty)
	));
	assert!(failed.state.failed && !failed.state.saving);
	assert_eq!(
		local_store::LocalStore::open(&path)
			.unwrap()
			.app_preferences()
			.unwrap()
			.user_volumes,
		vec![(7, 35), (9, 150)]
	);
	std::fs::remove_file(path).unwrap();
	std::fs::remove_dir(root).unwrap();
	println!(
		"Voice preferences debug check passed: SQLite reopen, startup restore, independent local mute and subsequent preference edits. No Discord or audio devices accessed."
	);
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A valid two-page SQLite file with one empty table and auto_vacuum=NONE.
	/// Keeping the table prevents the fresh-database pragma from upgrading its layout.
	fn legacy_cache_file(root: &std::path::Path) -> LocalStore {
		let mut bytes = vec![0u8; 8192];
		bytes[..16].copy_from_slice(b"SQLite format 3\0");
		bytes[16..18].copy_from_slice(&4096u16.to_be_bytes());
		bytes[18..24].copy_from_slice(&[1, 1, 0, 64, 32, 32]);
		for (offset, value) in [
			(24, 1u32),
			(28, 2),
			(40, 1),
			(44, 4),
			(56, 1),
			(60, 1),
			(92, 1),
			(96, 3_046_001),
		] {
			bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
		}
		let values: [&[u8]; 4] = [
			b"table",
			b"legacy_marker",
			b"legacy_marker",
			b"CREATE TABLE legacy_marker(value)",
		];
		let mut record = vec![
			6,
			13 + 2 * values[0].len() as u8,
			13 + 2 * values[1].len() as u8,
			13 + 2 * values[2].len() as u8,
			1,
			13 + 2 * values[3].len() as u8,
		];
		for value in &values[..3] {
			record.extend_from_slice(value);
		}
		record.push(2);
		record.extend_from_slice(values[3]);
		assert!(record.len() < 128);
		let mut cell = vec![record.len() as u8, 1];
		cell.extend(record);
		let start = 4096 - cell.len();
		bytes[100] = 13;
		bytes[103..105].copy_from_slice(&1u16.to_be_bytes());
		bytes[105..107].copy_from_slice(&(start as u16).to_be_bytes());
		bytes[108..110].copy_from_slice(&(start as u16).to_be_bytes());
		bytes[start..4096].copy_from_slice(&cell);
		bytes[4096] = 13;
		bytes[4101..4103].copy_from_slice(&4096u16.to_be_bytes());
		let path = root.join("client.sqlite3");
		std::fs::write(&path, bytes).unwrap();
		let store = LocalStore::open(&path).unwrap();
		assert!(store.cache_layout_migration_bytes().unwrap().is_some());
		store
	}

	#[test]
	fn cache_layout_migration_requires_reserved_space_and_preserves_personal_data() {
		let root = std::env::temp_dir().join(format!(
			"serein-cache-layout-worker-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::create_dir(&root).unwrap();
		let mut store = legacy_cache_file(&root);
		store
			.save_draft(Id(1), Id(2), "Preserved legacy draft")
			.unwrap();
		store.save_game_activity_enabled(false).unwrap();
		let budget = local_store::cache_budget::CacheBudget::for_root(&root);
		let journal = root.join("client.sqlite3-journal");
		std::fs::File::create(&journal)
			.unwrap()
			.set_len(model::TOTAL_CACHE_BYTES)
			.unwrap();
		assert_eq!(
			prepare_cache_layout(&mut store, Some(&budget), &HistoryPins::default()),
			Err(StoreError::Capacity)
		);
		assert!(store.cache_layout_migration_bytes().unwrap().is_some());
		assert_eq!(
			store.load_drafts(Id(1)).unwrap()[&Id(2)],
			"Preserved legacy draft"
		);
		assert!(!store.game_activity_enabled().unwrap());
		assert_eq!(budget.usage().reserved, 0);
		std::fs::remove_file(journal).unwrap();
		budget.refresh_database().unwrap();
		prepare_cache_layout(&mut store, Some(&budget), &HistoryPins::default()).unwrap();
		assert_eq!(store.cache_layout_migration_bytes().unwrap(), None);
		assert_eq!(
			store.load_drafts(Id(1)).unwrap()[&Id(2)],
			"Preserved legacy draft"
		);
		assert!(!store.game_activity_enabled().unwrap());
		assert_eq!(budget.usage().reserved, 0);
		assert!(budget.usage().total <= model::TOTAL_CACHE_BYTES);
		drop(store);
		drop(budget);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn fresh_incremental_cache_needs_no_layout_rebuild_or_reservation() {
		let root = std::env::temp_dir().join(format!(
			"serein-cache-layout-fresh-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::create_dir(&root).unwrap();
		let mut store = LocalStore::open(&root.join("client.sqlite3")).unwrap();
		assert_eq!(store.cache_layout_migration_bytes().unwrap(), None);
		prepare_cache_layout(&mut store, None, &HistoryPins::default()).unwrap();
		assert_eq!(store.cache_layout_migration_bytes().unwrap(), None);
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn confirmed_missing_members_remove_only_the_current_job_account_and_server_hints() {
		let mut store = LocalStore::open(std::path::Path::new(":memory:"));
		let row = |guild, user| model::MemberDisplay {
			guild: Id(guild),
			user: Id(user),
			name: format!("Member {user}"),
			username: Some(format!("user{user}").into_boxed_str()),
			nick: None,
		};
		for (account, rows) in [
			(Id(1), vec![row(10, 1), row(10, 2), row(10, 3), row(11, 2)]),
			(Id(2), vec![row(10, 2)]),
		] {
			store
				.as_mut()
				.unwrap()
				.save_member_displays(account, &rows)
				.unwrap();
		}
		let checkpoint = local_store::SyncCheckpoint {
			job: 7,
			scope: Some(Id(10)),
			started_ms: 1_420_070_400_000,
			cutoff_ms: 1_420_070_400_000,
			payload: vec![],
		};
		store
			.as_mut()
			.unwrap()
			.save_sync_checkpoint(Id(1), &checkpoint)
			.unwrap();
		let safety = HistorySafety::default();
		let epoch = safety.epoch();
		let mut fresh = row(10, 1);
		fresh.nick = Some("New nickname".into());
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				epoch,
				Operation::SyncSaveMembers {
					job: 7,
					request: 1,
					guild: Id(10),
					rows: vec![fresh],
					missing: vec![Id(2)]
				}
			),
			Outcome::SyncMembersSaved {
				job: 7,
				request: 1,
				result: Ok(()),
				..
			}
		));
		let current = store
			.as_ref()
			.unwrap()
			.load_member_displays(Id(1), Id(10))
			.unwrap();
		assert_eq!(current.len(), 2);
		assert!(!current.iter().any(|row| row.user == Id(2)));
		assert!(
			current
				.iter()
				.any(|row| row.user == Id(1) && row.nick.as_deref() == Some("New nickname"))
		);
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(1), Id(11))
				.unwrap(),
			vec![row(11, 2)]
		);
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(2), Id(10))
				.unwrap(),
			vec![row(10, 2)]
		);
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(2),
				epoch,
				Operation::SyncSaveMembers {
					job: 7,
					request: 2,
					guild: Id(10),
					rows: vec![],
					missing: vec![Id(2)]
				}
			),
			Outcome::SyncMembersSaved {
				result: Err(StoreError::Unavailable),
				..
			}
		));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				epoch,
				Operation::SyncSaveMembers {
					job: 7,
					request: 2,
					guild: Id(11),
					rows: vec![],
					missing: vec![Id(2)]
				}
			),
			Outcome::SyncMembersSaved {
				result: Err(StoreError::Unavailable),
				..
			}
		));
		// A cancelled/replaced job cannot apply a queued confirmed-missing result.
		store
			.as_mut()
			.unwrap()
			.save_sync_checkpoint(
				Id(1),
				&local_store::SyncCheckpoint {
					job: 8,
					..checkpoint
				},
			)
			.unwrap();
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				epoch,
				Operation::SyncSaveMembers {
					job: 7,
					request: 2,
					guild: Id(10),
					rows: vec![],
					missing: vec![Id(3)]
				}
			),
			Outcome::SyncMembersSaved {
				result: Err(StoreError::Unavailable),
				..
			}
		));
		safety.invalidate();
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				epoch,
				Operation::SyncSaveMembers {
					job: 8,
					request: 3,
					guild: Id(10),
					rows: vec![],
					missing: vec![Id(3)]
				}
			),
			Outcome::SyncMembersSaved {
				result: Err(StoreError::Unavailable),
				..
			}
		));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.iter()
				.any(|row| row.user == Id(3))
		);
	}

	#[test]
	fn complete_member_snapshot_reconciliation_is_scoped_and_retired_epochs_cannot_prune() {
		let mut store = LocalStore::open(std::path::Path::new(":memory:"));
		let row = |guild, user| model::MemberDisplay {
			guild: Id(guild),
			user: Id(user),
			name: format!("Member {user}"),
			username: None,
			nick: None,
		};
		store
			.as_mut()
			.unwrap()
			.save_member_displays(Id(1), &[row(10, 1), row(10, 2), row(10, 3), row(11, 2)])
			.unwrap();
		store
			.as_mut()
			.unwrap()
			.save_member_displays(Id(2), &[row(10, 2)])
			.unwrap();
		store
			.as_mut()
			.unwrap()
			.save_sync_checkpoint(
				Id(1),
				&local_store::SyncCheckpoint {
					job: 4,
					scope: None,
					started_ms: 1_420_070_400_000,
					cutoff_ms: 1_420_070_400_000,
					payload: vec![],
				},
			)
			.unwrap();
		let safety = HistorySafety::default();
		let epoch = safety.epoch();
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				epoch,
				Operation::SyncReconcileMembers {
					job: 4,
					request: 1,
					guild: Id(10),
					users: vec![Id(1), Id(3)]
				}
			),
			Outcome::SyncMembersReconciled {
				job: 4,
				request: 1,
				result: Ok(1),
				..
			}
		));
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.len(),
			2
		);
		safety.invalidate();
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				epoch,
				Operation::SyncReconcileMembers {
					job: 4,
					request: 2,
					guild: Id(10),
					users: vec![]
				}
			),
			Outcome::SyncMembersReconciled {
				result: Err(StoreError::Unavailable),
				..
			}
		));
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.len(),
			2
		);
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				safety.epoch(),
				Operation::SyncReconcileMembers {
					job: 4,
					request: 3,
					guild: Id(10),
					users: vec![]
				}
			),
			Outcome::SyncMembersReconciled { result: Ok(2), .. }
		));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.is_empty()
		);
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(1), Id(11))
				.unwrap(),
			vec![row(11, 2)]
		);
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(2), Id(10))
				.unwrap(),
			vec![row(10, 2)]
		);
	}

	#[test]
	fn sync_member_queue_rejects_incomplete_or_conflicting_prune_shapes() {
		let (send, _commands) = mpsc::sync_channel(16);
		let (_, receive) = mpsc::sync_channel(16);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		for users in [
			vec![Id(2), Id(1)],
			vec![Id(1), Id(1)],
			vec![Id(0)],
			vec![Id(1); 250_001],
		] {
			assert!(!cache.queue(
				1,
				Id(1),
				Operation::SyncReconcileMembers {
					job: 1,
					request: 1,
					guild: Id(10),
					users
				}
			));
		}
		let row = model::MemberDisplay {
			guild: Id(10),
			user: Id(2),
			name: "Member".into(),
			username: None,
			nick: None,
		};
		assert!(!cache.queue(
			1,
			Id(1),
			Operation::SyncSaveMembers {
				job: 1,
				request: 1,
				guild: Id(10),
				rows: vec![row],
				missing: vec![Id(2)]
			}
		));
		assert!(!cache.queue(
			1,
			Id(1),
			Operation::SyncSaveMembers {
				job: 1,
				request: 1,
				guild: Id(10),
				rows: vec![],
				missing: vec![Id(1); 101]
			}
		));
		assert!(cache.queue(
			1,
			Id(1),
			Operation::SyncReconcileMembers {
				job: 1,
				request: 1,
				guild: Id(10),
				users: vec![Id(1), Id(2)]
			}
		));
	}

	#[test]
	fn item_pressure_evicts_inactive_history_and_keeps_current_pages_and_edits() {
		let mut store = LocalStore::open(std::path::Path::new(":memory:")).unwrap();
		let account = Id(1);
		let current = Id(2);
		let inactive = Id(3);
		store
			.save_channel(
				account,
				current,
				&[
					test_support::message(10, current),
					test_support::message(11, current),
				],
				false,
			)
			.unwrap();
		store
			.save_sync_checkpoint(
				account,
				&local_store::SyncCheckpoint {
					job: 1,
					scope: None,
					started_ms: 1_420_070_400_000,
					cutoff_ms: 1_420_070_400_000,
					payload: Vec::new(),
				},
			)
			.unwrap();
		let coverage = local_store::HistoryCoverage {
			job: 1,
			request: 1,
			cutoff: Id(1),
			upper: Id(100),
			before: None,
			next_before: None,
			complete: true,
			revision: 0,
		};
		store
			.save_sync_page(
				account,
				inactive,
				&[test_support::message(9, inactive)],
				&coverage,
			)
			.unwrap();
		let pins = HistoryPins {
			channel: Some((account, current)),
			writing: Some((account, current)),
			..HistoryPins::default()
		};
		// Inject a three-item ceiling instead of allocating a million messages.
		ensure_history_items(
			&mut store,
			account,
			current,
			&[test_support::message(12, current)],
			3,
			&pins,
		)
		.unwrap();
		assert_eq!(store.history_cache_stats().unwrap().messages, 2);
		assert_eq!(store.load_channel(account, current).unwrap().len(), 2);
		assert!(store.load_channel(account, inactive).unwrap().is_empty());
		// Evicted sync coverage cannot silently redownload the same job's old page.
		assert!(
			store
				.save_sync_page(
					account,
					inactive,
					&[test_support::message(9, inactive)],
					&local_store::HistoryCoverage {
						request: 2,
						..coverage
					}
				)
				.unwrap()
				.stale
		);
		store
			.save_changes(
				account,
				current,
				&[test_support::message(12, current)],
				&[Id(10), Id(11), Id(12)],
				false,
			)
			.unwrap();
		assert_eq!(store.history_cache_stats().unwrap().messages, 3);
		// Existing-row edits remain writable even when every row belongs to the pinned channel.
		ensure_history_items(
			&mut store,
			account,
			current,
			&[test_support::message(11, current)],
			3,
			&pins,
		)
		.unwrap();
		assert_eq!(store.history_cache_stats().unwrap().messages, 3);
		assert_eq!(
			ensure_history_items(
				&mut store,
				account,
				current,
				&[test_support::message(13, current)],
				3,
				&pins
			),
			Err(StoreError::Capacity)
		);
		assert_eq!(store.load_channel(account, current).unwrap().len(), 3);
	}

	#[test]
	fn full_disk_budget_rejects_only_the_new_write_and_preserves_drafts() {
		let root = std::env::temp_dir().join(format!(
			"serein-global-cache-write-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::create_dir(&root).unwrap();
		let path = root.join("client.sqlite3");
		// A sparse synthetic allocation exercises accounting without writing gigabytes.
		std::fs::File::create(&path)
			.unwrap()
			.set_len(model::TOTAL_CACHE_BYTES)
			.unwrap();
		let budget = local_store::cache_budget::CacheBudget::for_root(&root);
		let mut store = LocalStore::open(std::path::Path::new(":memory:"));
		store
			.as_mut()
			.unwrap()
			.save_draft(Id(1), Id(2), "Previous saved draft")
			.unwrap();
		let safety = HistorySafety::default();
		let result = execute_with_disk_budget(
			&mut store,
			&safety,
			Id(1),
			safety.epoch(),
			Operation::SaveDraft {
				channel: Id(2),
				content: "Latest draft remains in memory".into(),
			},
			Some(&budget),
			&HistoryPins::default(),
		);
		assert!(matches!(
			result,
			Outcome::Failed {
				error: StoreError::Capacity,
				history_cleanup: false,
				..
			}
		));
		assert!(safety.allows(safety.epoch()));
		assert_eq!(
			store.as_ref().unwrap().load_drafts(Id(1)).unwrap()[&Id(2)],
			"Previous saved draft"
		);
		assert!(matches!(
			execute_with_disk_budget(
				&mut store,
				&safety,
				Id(1),
				safety.epoch(),
				Operation::SaveCustomFont(None),
				Some(&budget),
				&HistoryPins::default()
			),
			Outcome::CustomFont(Err(_))
		));
		drop(budget);
		std::fs::remove_file(path).unwrap();
		std::fs::remove_dir(root).unwrap();
	}

	#[test]
	fn cached_older_page_remains_pending_and_service_reconciles_its_covered_rows() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.timeline.clear();
		state
			.timeline
			.insert(test_support::message(100, channel), false, false)
			.unwrap();
		state.history_pending = false;
		state.freshness = model::Freshness::Fresh;
		let Some(client_core::Command::History { request, .. }) = state.older_history() else {
			panic!("older request")
		};
		assert!(state.hydrate_cached_history_page(
			channel,
			request,
			vec![
				test_support::message(90, channel),
				test_support::message(91, channel)
			]
		));
		assert!(state.history_pending);
		assert!(state.timeline.get(Id(100)).is_some());
		assert!(state.timeline.get(Id(90)).is_some());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::History {
				channel,
				request,
				older: true,
				messages: vec![
					test_support::message(89, channel),
					test_support::message(91, channel),
				],
			},
		});
		assert!(!state.history_pending);
		assert!(state.timeline.get(Id(100)).is_some());
		assert!(state.timeline.get(Id(90)).is_none());
		assert!(state.timeline.get(Id(89)).is_some());
	}

	#[test]
	fn cached_reply_jump_requires_exact_target_and_preserves_back_navigation() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.timeline.clear();
		state
			.timeline
			.insert(test_support::message(100, channel), false, false)
			.unwrap();
		state.history_pending = false;
		state.freshness = model::Freshness::Fresh;
		state.reply = Some(client_core::Reply::to(Id(50)));
		let Some(client_core::Command::HistoryAround { request, .. }) =
			state.open_reply_target(Id(50))
		else {
			panic!("around request")
		};
		assert_eq!(state.pending_cached_history_target(), Some(Id(50)));
		assert!(!state.hydrate_cached_history_page(
			channel,
			request,
			vec![test_support::message(49, channel)]
		));
		assert!(state.timeline.get(Id(100)).is_some());
		assert!(!state.hydrate_cached_history_page(
			channel,
			request.wrapping_sub(1),
			vec![test_support::message(50, channel)]
		));
		assert!(state.hydrate_cached_history_page(
			channel,
			request,
			vec![test_support::message(50, channel)]
		));
		assert!(state.timeline.get(Id(50)).is_some());
		assert!(state.timeline.get(Id(100)).is_none());
		assert!(state.history_pending);
		assert!(state.can_return_from_reply());
		// A service page without the original must retire the provisional target.
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::History {
				channel,
				request,
				older: false,
				messages: vec![test_support::message(49, channel)],
			},
		});
		assert!(state.timeline.get(Id(50)).is_none());
		assert!(state.status.contains("not returned"));
		assert!(state.return_from_reply().is_none());
		assert!(state.timeline.get(Id(100)).is_some());
	}

	#[test]
	fn scoped_sync_pages_follow_parent_cleanup_and_preserve_other_scopes() {
		let mut store = LocalStore::open(std::path::Path::new(":memory:"));
		for account in [Id(1), Id(2)] {
			store
				.as_mut()
				.unwrap()
				.save_sync_checkpoint(
					account,
					&local_store::SyncCheckpoint {
						job: 9,
						scope: None,
						started_ms: 1_420_070_400_000,
						cutoff_ms: 1_420_070_400_000,
						payload: vec![],
					},
				)
				.unwrap();
		}
		let safety = HistorySafety::default();
		let coverage = local_store::HistoryCoverage {
			job: 9,
			request: 1,
			cutoff: Id(1),
			upper: Id(100),
			before: None,
			next_before: None,
			complete: true,
			revision: 0,
		};
		for (account, channel, guild, parent, request) in [
			(Id(1), Id(30), Id(10), Id(20), 1),
			(Id(1), Id(31), Id(10), Id(21), 2),
			(Id(1), Id(32), Id(11), Id(40), 3),
			(Id(2), Id(30), Id(10), Id(20), 1),
		] {
			assert!(matches!(
				execute(
					&mut store,
					&safety,
					account,
					safety.epoch(),
					Operation::SyncSavePage {
						job: 9,
						request,
						channel,
						guild: Some(guild),
						parent: Some(parent),
						messages: vec![test_support::message(10, channel)],
						coverage: local_store::HistoryCoverage {
							request,
							..coverage
						}
					}
				),
				Outcome::SyncPageSaved {
					result: Ok(local_store::SyncPageResult {
						saved: 1,
						stale: false
					}),
					..
				}
			));
		}
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				safety.epoch(),
				Operation::ClearChannel { channel: Id(20) }
			),
			Outcome::Saved
		));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_channel(Id(1), Id(30))
				.unwrap()
				.is_empty()
		);
		for (account, channel) in [(Id(1), Id(31)), (Id(1), Id(32)), (Id(2), Id(30))] {
			assert_eq!(
				store
					.as_ref()
					.unwrap()
					.load_channel(account, channel)
					.unwrap()
					.len(),
				1
			);
		}
		// A reply already in flight before revocation cannot recreate the removed child.
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				safety.epoch(),
				Operation::SyncSavePage {
					job: 9,
					request: 4,
					channel: Id(30),
					guild: Some(Id(10)),
					parent: Some(Id(20)),
					messages: vec![test_support::message(10, Id(30))],
					coverage: local_store::HistoryCoverage {
						request: 4,
						..coverage
					}
				}
			),
			Outcome::SyncPageSaved {
				result: Ok(local_store::SyncPageResult {
					saved: 0,
					stale: true
				}),
				..
			}
		));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_channel(Id(1), Id(30))
				.unwrap()
				.is_empty()
		);
	}

	#[test]
	fn invalid_sync_parent_scopes_fail_before_cache_mutation() {
		let mut store = LocalStore::open(std::path::Path::new(":memory:"));
		store
			.as_mut()
			.unwrap()
			.save_sync_checkpoint(
				Id(1),
				&local_store::SyncCheckpoint {
					job: 9,
					scope: None,
					started_ms: 1_420_070_400_000,
					cutoff_ms: 1_420_070_400_000,
					payload: vec![],
				},
			)
			.unwrap();
		let safety = HistorySafety::default();
		let coverage = local_store::HistoryCoverage {
			job: 9,
			request: 1,
			cutoff: Id(1),
			upper: Id(100),
			before: None,
			next_before: None,
			complete: true,
			revision: 0,
		};
		for (guild, parent) in [
			(None, Some(Id(20))),
			(Some(Id(0)), None),
			(Some(Id(10)), Some(Id(0))),
			(Some(Id(10)), Some(Id(30))),
		] {
			assert!(!valid_history_scope(Id(30), guild, parent));
			assert!(matches!(
				execute(
					&mut store,
					&safety,
					Id(1),
					safety.epoch(),
					Operation::SyncSavePage {
						job: 9,
						request: 1,
						channel: Id(30),
						guild,
						parent,
						messages: vec![test_support::message(10, Id(30))],
						coverage
					}
				),
				Outcome::SyncPageSaved {
					result: Err(StoreError::Capacity),
					..
				}
			));
		}
		assert!(valid_history_scope(Id(30), None, None));
		assert!(valid_history_scope(Id(30), Some(Id(10)), Some(Id(20))));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_channel(Id(1), Id(30))
				.unwrap()
				.is_empty()
		);
	}

	#[test]
	fn background_sync_pages_keep_correlation_and_reject_live_edit_races() {
		let mut store = LocalStore::open(std::path::Path::new(":memory:"));
		let safety = HistorySafety::default();
		let account = Id(1);
		let channel = Id(2);
		store
			.as_mut()
			.unwrap()
			.save_sync_checkpoint(
				account,
				&local_store::SyncCheckpoint {
					job: 7,
					scope: None,
					started_ms: 1_420_070_400_000,
					cutoff_ms: 1_420_070_400_000,
					payload: Vec::new(),
				},
			)
			.unwrap();
		let coverage = local_store::HistoryCoverage {
			job: 7,
			request: 8,
			cutoff: Id(1),
			upper: Id(100),
			before: None,
			next_before: None,
			complete: true,
			revision: 0,
		};
		let message = test_support::message(10, channel);
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				safety.epoch(),
				Operation::SyncSavePage {
					job: 7,
					request: 8,
					channel,
					guild: None,
					parent: None,
					messages: vec![message.clone()],
					coverage
				}
			),
			Outcome::SyncPageSaved {
				job: 7,
				request: 8,
				result: Ok(local_store::SyncPageResult {
					saved: 1,
					stale: false
				}),
				..
			}
		));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				safety.epoch(),
				Operation::ObserveHistoryChange {
					channel,
					message: Id(10)
				}
			),
			Outcome::Saved
		));
		let mut edited = message.clone();
		edited.content = "Current live body".into();
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				safety.epoch(),
				Operation::SaveChanges {
					channel,
					messages: vec![edited],
					retained: vec![Id(10)],
					start: false
				}
			),
			Outcome::Saved
		));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				safety.epoch(),
				Operation::SyncSavePage {
					job: 7,
					request: 9,
					channel,
					guild: None,
					parent: None,
					messages: vec![message],
					coverage: local_store::HistoryCoverage {
						request: 9,
						..coverage
					}
				}
			),
			Outcome::SyncPageSaved {
				result: Ok(local_store::SyncPageResult {
					saved: 0,
					stale: true
				}),
				..
			}
		));
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_channel(account, channel)
				.unwrap()[0]
				.content,
			"Current live body"
		);
		let epoch = safety.epoch();
		safety.invalidate();
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				epoch,
				Operation::SyncReadRevision {
					job: 7,
					request: 10,
					channel
				}
			),
			Outcome::SyncRevision {
				job: 7,
				request: 10,
				result: Err(StoreError::Unavailable),
				..
			}
		));
	}

	#[test]
	fn background_sync_has_an_independent_eight_mebibyte_queue_budget() {
		let (send, commands) = mpsc::sync_channel(16);
		let (_, receive) = mpsc::sync_channel(16);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let checkpoint = local_store::SyncCheckpoint {
			job: 1,
			scope: Some(Id(2)),
			started_ms: 100,
			cutoff_ms: 0,
			payload: vec![0; 1024 * 1024],
		};
		for _ in 0..7 {
			assert!(cache.queue(1, Id(1), Operation::SaveSyncCheckpoint(checkpoint.clone())));
		}
		assert!(!cache.queue(1, Id(1), Operation::SaveSyncCheckpoint(checkpoint)));
		assert!(cache.queue(
			1,
			Id(1),
			Operation::SaveDraft {
				channel: Id(2),
				content: "Foreground draft remains admitted".into()
			}
		));
		assert!(*cache.sync_budget.used.lock().unwrap() <= SYNC_QUEUE_BYTES);
		drop(commands);
		assert_eq!(*cache.sync_budget.used.lock().unwrap(), 0);
	}

	#[test]
	fn sync_checkpoints_remain_account_scoped_and_cleanup_removes_them() {
		let mut store = LocalStore::open(std::path::Path::new(":memory:"));
		let safety = HistorySafety::default();
		let checkpoint = local_store::SyncCheckpoint {
			job: 4,
			scope: None,
			started_ms: 100,
			cutoff_ms: 0,
			payload: vec![1, 2, 3],
		};
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				safety.epoch(),
				Operation::SaveSyncCheckpoint(checkpoint.clone())
			),
			Outcome::SyncCheckpointSaved { result: Ok(()), .. }
		));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(2),
				safety.epoch(),
				Operation::LoadSyncCheckpoint
			),
			Outcome::SyncCheckpoint {
				result: Ok(None),
				..
			}
		));
		assert!(
			matches!(execute(&mut store,&safety,Id(1),safety.epoch(),Operation::LoadSyncCheckpoint),Outcome::SyncCheckpoint {result:Ok(Some(value)),..} if value==checkpoint)
		);
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				safety.epoch(),
				Operation::ClearHistory
			),
			Outcome::HistoryCleared
		));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_sync_checkpoint(Id(1))
				.unwrap()
				.is_none()
		);
	}

	fn expression_hint(guild: Id, user: u64) -> model::ExpressionCatalog {
		model::ExpressionCatalog {
			guild,
			emojis: Some(vec![model::CustomEmoji {
				id: Id(user),
				name: "cached_emoji".into(),
				animated: true,
				available: false,
				managed: false,
				roles: None,
			}]),
			stickers: Some(vec![model::Sticker {
				id: Id(user + 1),
				name: "Cached sticker".into(),
				description: String::new(),
				tags: String::new(),
				format_type: 1,
				guild_id: Some(guild),
				pack_id: None,
				available: false,
			}]),
		}
	}
	#[test]
	fn expression_worker_restores_only_display_hints_and_fences_scoped_cleanup() {
		let mut state = test_support::demo_state();
		state.demo = false;
		let account = state.user.as_ref().unwrap().id;
		let guild = Id(10);
		let current = state.guilds.iter_mut().find(|g| g.id == guild).unwrap();
		current.emojis = None;
		current.stickers = None;
		let hint = expression_hint(guild, 90_000);
		assert!(hint.valid());
		let other = expression_hint(Id(11), 91_000);
		let safety = HistorySafety::default();
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		for (owner, value) in [
			(account, hint.clone()),
			(account, other.clone()),
			(Id(999_999), hint.clone()),
		] {
			assert!(matches!(
				execute(
					&mut store,
					&safety,
					owner,
					safety.epoch(),
					Operation::SaveExpressions(Box::new(value))
				),
				Outcome::Saved
			));
		}
		let revision = state.expression_guild_cache_revision(guild);
		let old_epoch = safety.epoch();
		let Outcome::Expressions {
			account: owner,
			result: Ok(Some(restored)),
			..
		} = execute(
			&mut store,
			&safety,
			account,
			old_epoch,
			Operation::LoadExpressions { guild, revision },
		)
		else {
			panic!("expected cached expression hints")
		};
		assert_eq!(owner, account);
		assert!(state.hydrate_expression_catalog(guild, revision, restored));
		assert_eq!(
			state.display_guild_emojis(guild).unwrap()[0].name,
			"cached_emoji"
		);
		assert!(state.guild(guild).unwrap().emojis.is_none());
		assert!(!state.can_send_sticker(&hint.stickers.as_ref().unwrap()[0]));
		safety.invalidate();
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				old_epoch,
				Operation::SaveExpressions(Box::new(hint.clone()))
			),
			Outcome::Saved
		));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				safety.epoch(),
				Operation::ClearExpressions {
					guild,
					emoji: Some(true)
				}
			),
			Outcome::Saved
		));
		let remaining = store
			.as_ref()
			.unwrap()
			.load_expression_catalog(account, guild)
			.unwrap()
			.unwrap();
		assert!(remaining.emojis.is_none());
		assert_eq!(remaining.stickers.as_ref(), hint.stickers.as_ref());
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				safety.epoch(),
				Operation::ClearExpressions { guild, emoji: None }
			),
			Outcome::Saved
		));
		let db = store.as_ref().unwrap();
		assert!(
			db.load_expression_catalog(account, guild)
				.unwrap()
				.is_none()
		);
		assert_eq!(
			db.load_expression_catalog(account, Id(11)).unwrap(),
			Some(other)
		);
		assert_eq!(
			db.load_expression_catalog(Id(999_999), guild).unwrap(),
			Some(hint)
		);
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				old_epoch,
				Operation::LoadExpressions { guild, revision }
			),
			Outcome::Expressions {
				result: Ok(None),
				..
			}
		));
	}
	#[test]
	fn expression_schedule_coalesces_bounds_and_retries_after_progress() {
		let mut work = ExpressionCacheSchedule::default();
		work.scope((1, Id(1), 0), (1..=180).map(Id));
		assert_eq!(work.pending.len(), ExpressionCacheSchedule::LIMIT);
		work.selected(Some(Id(180)));
		assert_eq!(work.pending.len(), ExpressionCacheSchedule::LIMIT);
		assert_eq!(work.next(), Some((Id(180), ExpressionCacheSchedule::LOAD)));
		work.enqueue(Id(180), ExpressionCacheSchedule::SAVE);
		assert_eq!(work.pending.len(), ExpressionCacheSchedule::LIMIT);
		let (send, commands) = mpsc::sync_channel(1);
		let (_, receive) = mpsc::sync_channel(1);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		assert!(cache.queue(1, Id(1), Operation::LoadDrafts));
		assert!(!cache.queue(
			1,
			Id(1),
			Operation::LoadExpressions {
				guild: Id(180),
				revision: 0
			}
		));
		work.blocked();
		assert!(work.next().is_none());
		drop(commands.try_recv().unwrap());
		work.capacity_changed();
		assert!(cache.queue(
			1,
			Id(1),
			Operation::LoadExpressions {
				guild: Id(180),
				revision: 0
			}
		));
		work.completed(Id(180), ExpressionCacheSchedule::LOAD);
		assert_eq!(work.next(), Some((Id(180), ExpressionCacheSchedule::SAVE)));
		work.remove(Id(180));
		assert_ne!(work.next().map(|(id, _)| id), Some(Id(180)));
		work.scope((2, Id(2), 0), std::iter::once(Id(300)));
		assert_eq!(work.next(), Some((Id(300), ExpressionCacheSchedule::LOAD)));
		work.completed(Id(300), ExpressionCacheSchedule::LOAD);
		work.completed(Id(300), ExpressionCacheSchedule::SAVE);
		work.refresh(std::iter::once(Id(300)));
		assert_eq!(work.next(), Some((Id(300), ExpressionCacheSchedule::LOAD)));
		work.completed(Id(300), ExpressionCacheSchedule::LOAD);
		assert!(
			work.next().is_none(),
			"permission refresh must not resave unchanged catalogs"
		);
		work.scope((3, Id(1), 0), (1..=128).map(Id));
		work.selected(Some(Id(1)));
		work.observe_epoch(7, std::iter::empty());
		work.observe_epoch(8, (129..=256).map(Id));
		assert_eq!(work.pending.len(), ExpressionCacheSchedule::LIMIT);
		assert_eq!(work.next(), Some((Id(1), ExpressionCacheSchedule::LOAD)));
		assert!(work.pending.iter().any(|(guild, _)| *guild == Id(256)));
		let snapshot = work.pending.clone();
		work.observe_epoch(8, std::iter::once(Id(999)));
		assert_eq!(
			work.pending, snapshot,
			"unchanged epoch must not rescan guilds"
		);
		assert!(coalesced_retry(&Operation::SaveGifFavorites(Vec::new())));
		assert!(coalesced_retry(&Operation::SaveExpressions(Box::new(
			expression_hint(Id(10), 93000)
		))));
		assert!(coalesced_retry(&Operation::SaveStickerPackHints(Box::new(
			model::StickerPackHints { packs: vec![] }
		))));
		assert!(!coalesced_retry(&Operation::SaveDraft {
			channel: Id(20),
			content: String::new()
		}));
	}
	#[test]
	fn expression_scheduler_restores_joined_guilds_after_sqlite_restart() {
		let root = std::env::temp_dir().join(format!(
			"serein-expressions-restart-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::create_dir(&root).unwrap();
		let path = root.join("cache.sqlite");
		let mut state = test_support::demo_state();
		state.demo = false;
		let account = state.user.as_ref().unwrap().id;
		let mut second = state.guilds[0].clone();
		second.id = Id(11);
		second.emojis = Some(vec![model::CustomEmoji {
			id: Id(55_001),
			name: "other_server".into(),
			animated: false,
			available: true,
			managed: false,
			roles: None,
		}]);
		state.guilds.push(second);
		let safety = HistorySafety::default();
		{
			let mut store = Ok(LocalStore::open(&path).unwrap());
			for guild in [Id(10), Id(11)] {
				let catalog = state.cached_expression_catalog(guild).unwrap();
				assert!(matches!(
					execute(
						&mut store,
						&safety,
						account,
						safety.epoch(),
						Operation::SaveExpressions(Box::new(catalog))
					),
					Outcome::Saved
				));
				let row = state.guilds.iter_mut().find(|g| g.id == guild).unwrap();
				row.emojis = None;
				row.stickers = None;
			}
		}
		let mut reopened = Ok(LocalStore::open(&path).unwrap());
		let mut schedule = ExpressionCacheSchedule::default();
		schedule.scope(
			(state.generation, account, safety.epoch()),
			state.guilds.iter().map(|guild| guild.id),
		);
		while let Some((guild, operation)) = schedule.next() {
			if operation == ExpressionCacheSchedule::LOAD {
				let revision = state.expression_guild_cache_revision(guild);
				let Outcome::Expressions {
					result: Ok(Some(hints)),
					..
				} = execute(
					&mut reopened,
					&safety,
					account,
					safety.epoch(),
					Operation::LoadExpressions { guild, revision },
				)
				else {
					panic!("missing cached guild")
				};
				assert!(state.hydrate_expression_catalog(guild, revision, hints));
			}
			schedule.completed(guild, operation);
		}
		assert_eq!(
			state.display_guild_emojis(Id(11)).unwrap()[0].name,
			"other_server"
		);
		assert!(!state.display_guild_emojis(Id(11)).unwrap()[0].available);
		assert!(state.guild(Id(10)).unwrap().emojis.is_none());
		assert!(state.guild(Id(11)).unwrap().emojis.is_none());
		assert!(
			reopened
				.as_ref()
				.unwrap()
				.load_expression_catalog(Id(2), Id(11))
				.unwrap()
				.is_none()
		);
		drop(reopened);
		std::fs::remove_dir_all(root).unwrap();
	}
	#[test]
	fn expression_queue_charges_and_releases_bounded_read_capacity() {
		let (send, commands) = mpsc::sync_channel(1);
		let (_, receive) = mpsc::sync_channel(1);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		assert!(cache.queue(
			1,
			Id(1),
			Operation::LoadExpressions {
				guild: Id(10),
				revision: 0
			}
		));
		assert!(*cache.budget.used.lock().unwrap() >= 8 * 1024 * 1024);
		assert!(!cache.queue(
			1,
			Id(1),
			Operation::LoadExpressions {
				guild: Id(11),
				revision: 0
			}
		));
		drop(commands.try_recv().unwrap());
		assert_eq!(*cache.budget.used.lock().unwrap(), 0);
		assert!(cache.queue(1, Id(1), Operation::LoadStickerPackHints { revision: 0 }));
		assert!(*cache.budget.used.lock().unwrap() >= 8 * 1024 * 1024);
		drop(commands.try_recv().unwrap());
		assert_eq!(*cache.budget.used.lock().unwrap(), 0);
	}
	#[test]
	fn expression_results_reserve_decoded_memory_until_consumed() {
		let catalog = expression_hint(Id(10), 93_000);
		let pack_id = Id(93_100);
		let packs = model::StickerPackHints {
			packs: vec![model::StickerPack {
				id: pack_id,
				name: "Cached pack".into(),
				stickers: vec![model::Sticker {
					id: Id(93_101),
					name: "Cached sticker".into(),
					description: String::new(),
					tags: String::new(),
					format_type: 1,
					guild_id: None,
					pack_id: Some(pack_id),
					available: false,
				}],
			}],
		};
		let expression_bytes = catalog.bytes();
		let pack_bytes = packs.bytes();
		let outcomes = [
			(
				Outcome::Expressions {
					account: Id(1),
					guild: Id(10),
					revision: 0,
					epoch: 0,
					result: Ok(Some(catalog)),
				},
				expression_bytes,
			),
			(
				Outcome::StickerPackHints {
					account: Id(1),
					revision: 0,
					epoch: 0,
					result: Ok(Some(packs)),
				},
				pack_bytes,
			),
		];
		let budget = Arc::new(Budget::default());
		for (outcome, expected) in outcomes {
			assert!(expected > 0);
			assert_eq!(outcome_bytes(&outcome), expected);
			let held = budget
				.reserve(outcome_bytes(&outcome) + 64 * 1024, false)
				.unwrap();
			assert!(
				budget
					.reserve(QUEUE_BYTES - held.bytes + 1, false)
					.is_none()
			);
			drop(held);
			assert!(budget.reserve(QUEUE_BYTES, false).is_some());
		}
		assert_eq!(
			outcome_bytes(&Outcome::StickerPackHints {
				account: Id(1),
				revision: 0,
				epoch: 0,
				result: Ok(None),
			}),
			0
		);
	}
	#[test]
	fn sticker_pack_worker_restores_disabled_hints_and_clears_only_the_account() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.stickers.loaded = false;
		let account = state.user.as_ref().unwrap().id;
		let pack_id = Id(92_000);
		let hints = model::StickerPackHints {
			packs: vec![model::StickerPack {
				id: pack_id,
				name: "Cached pack".into(),
				stickers: vec![model::Sticker {
					id: Id(92_001),
					name: "Cached sticker".into(),
					description: String::new(),
					tags: String::new(),
					format_type: 1,
					guild_id: None,
					pack_id: Some(pack_id),
					available: false,
				}],
			}],
		};
		assert!(hints.valid());
		let safety = HistorySafety::default();
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		for owner in [account, Id(999_999)] {
			assert!(matches!(
				execute(
					&mut store,
					&safety,
					owner,
					safety.epoch(),
					Operation::SaveStickerPackHints(Box::new(hints.clone()))
				),
				Outcome::Saved
			));
		}
		let revision = state.sticker_pack_cache_revision();
		let epoch = safety.epoch();
		let Outcome::StickerPackHints {
			account: owner,
			result: Ok(Some(restored)),
			..
		} = execute(
			&mut store,
			&safety,
			account,
			epoch,
			Operation::LoadStickerPackHints { revision },
		)
		else {
			panic!("expected cached sticker pack hints")
		};
		assert_eq!(owner, account);
		assert!(state.hydrate_sticker_pack_hints(revision, restored));
		assert!(
			!state.stickers.loaded,
			"a disk hint cannot suppress live refresh"
		);
		assert!(!state.can_send_sticker(&hints.packs[0].stickers[0]));
		safety.invalidate();
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				epoch,
				Operation::SaveStickerPackHints(Box::new(hints.clone()))
			),
			Outcome::Saved
		));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				safety.epoch(),
				Operation::ClearHistory
			),
			Outcome::HistoryCleared
		));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_sticker_pack_hints(account)
				.unwrap()
				.is_none()
		);
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_sticker_pack_hints(Id(999_999))
				.unwrap(),
			Some(hints)
		);
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				epoch,
				Operation::LoadStickerPackHints { revision }
			),
			Outcome::StickerPackHints {
				result: Ok(None),
				..
			}
		));
	}
	#[test]
	fn gif_cache_retries_only_unsent_work_after_worker_progress_and_scopes_accounts() {
		let (send, commands) = mpsc::sync_channel(1);
		let (_, receive) = mpsc::sync_channel(1);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let mut state = GifCache::default();
		let scope = (7, Id(1));
		let key = (scope.0, scope.1, 0, 0);
		assert!(cache.queue(7, Id(1), Operation::LoadDrafts));
		assert!(state.load.attempt(key));
		assert!(!cache.queue(7, Id(1), Operation::LoadGifFavorites));
		assert!(
			!state.load.attempt(key),
			"repaint cannot busy-loop on a full queue"
		);
		drop(commands.try_recv().unwrap());
		state.capacity_changed();
		assert!(state.load.attempt(key));
		assert!(cache.queue(7, Id(1), Operation::LoadGifFavorites));
		state.load.accepted(key);
		assert!(state.load.was_accepted(key));
		assert!(!state.may_save(scope), "accepted load is not yet applied");
		assert!(!state.load.attempt(key));
		let gif = model::Gif {
			id: "cached".into(),
			title: "cached".into(),
			url: "https://tenor.com/view/cached".into(),
			preview: "https://media.tenor.com/cached/tenor.gif".into(),
			width: 100,
			height: 100,
		};
		assert!(!cache.queue(7, Id(1), Operation::SaveGifFavorites(vec![gif.clone()])));
		state.reject_save(scope);
		assert!(!state.may_save(scope));
		assert!(
			!state.may_save((8, Id(2))),
			"another account must load its own cache"
		);
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		let (_, account, epoch, operation, reservation) = commands.try_recv().unwrap();
		assert!(matches!(
			execute(&mut store, &cache.history, account, epoch, operation),
			Outcome::GifFavorites(_)
		));
		drop(reservation);
		state.loaded(scope);
		state.capacity_changed();
		assert!(!state.load.attempt(key), "an accepted read is not repeated");
		assert!(state.may_save(scope));
		assert!(cache.queue(7, Id(1), Operation::SaveGifFavorites(vec![gif.clone()])));
		let (_, account, epoch, operation, _reservation) = commands.try_recv().unwrap();
		assert!(matches!(
			execute(&mut store, &cache.history, account, epoch, operation),
			Outcome::Saved
		));
		assert_eq!(
			store.as_ref().unwrap().gif_favorites(Id(1)).unwrap(),
			vec![gif]
		);
		assert!(
			store
				.as_ref()
				.unwrap()
				.gif_favorites(Id(2))
				.unwrap()
				.is_empty()
		);
	}
	#[test]
	fn server_display_worker_obeys_account_and_history_fences() {
		let mut store = LocalStore::open(std::path::Path::new(":memory:"));
		let safety = HistorySafety::default();
		let row = local_store::ServerDisplay {
			guild: Id(10),
			name: Some("cached server".into()),
			icon: Some(None),
		};
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				0,
				Operation::SaveServers {
					rows: vec![row.clone()],
					revision: 7
				}
			),
			Outcome::ServersSaved {
				result: Some(Ok(())),
				..
			}
		));
		let Outcome::Servers { result, .. } = execute(
			&mut store,
			&safety,
			Id(1),
			0,
			Operation::LoadServers {
				revision: 7,
				guilds: vec![Id(10)],
			},
		) else {
			panic!("server load")
		};
		assert_eq!(result.unwrap(), vec![row.clone()]);
		let Outcome::Servers { result, .. } = execute(
			&mut store,
			&safety,
			Id(2),
			0,
			Operation::LoadServers {
				revision: 7,
				guilds: vec![Id(10)],
			},
		) else {
			panic!("server load")
		};
		assert!(result.unwrap().is_empty());
		safety.invalidate();
		let updated = local_store::ServerDisplay {
			name: Some("renamed server".into()),
			..row.clone()
		};
		assert!(
			matches!(
				execute(
					&mut store,
					&safety,
					Id(1),
					0,
					Operation::SaveServers {
						rows: vec![updated.clone()],
						revision: 7
					}
				),
				Outcome::ServersSaved { result: None, .. }
			),
			"an invalidated write must not acknowledge dirty fields"
		);
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				safety.epoch(),
				Operation::SaveServers {
					rows: vec![updated.clone()],
					revision: 8
				}
			),
			Outcome::ServersSaved {
				result: Some(Ok(())),
				..
			}
		));
		let Outcome::Servers { result, .. } = execute(
			&mut store,
			&safety,
			Id(1),
			safety.epoch(),
			Operation::LoadServers {
				revision: 8,
				guilds: vec![Id(10)],
			},
		) else {
			panic!("server load")
		};
		assert_eq!(result.unwrap(), vec![updated]);
		let Outcome::Servers { result, .. } = execute(
			&mut store,
			&safety,
			Id(1),
			0,
			Operation::LoadServers {
				revision: 7,
				guilds: vec![Id(10)],
			},
		) else {
			panic!("server load")
		};
		assert!(result.unwrap().is_empty());
	}
	#[test]
	fn rejected_history_read_waits_for_capacity_then_retries_once() {
		let (send, commands) = mpsc::sync_channel(1);
		let (_, receive) = mpsc::sync_channel(1);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let mut load = HistoryLoad::default();
		let key = (1, Id(2), 7, cache.history.epoch());
		assert!(cache.queue(1, Id(1), Operation::LoadDrafts));
		assert!(load.attempt(key));
		assert!(!cache.queue(
			1,
			Id(1),
			Operation::LoadChannel {
				channel: Id(2),
				request: 7
			}
		));
		for _ in 0..100 {
			assert!(!load.attempt(key), "repainting cannot spin on a full queue");
		}
		drop(commands.try_recv().unwrap());
		load.capacity_changed();
		assert!(load.attempt(key));
		assert!(cache.queue(
			1,
			Id(1),
			Operation::LoadChannel {
				channel: Id(2),
				request: 7
			}
		));
		load.accepted(key);
		load.capacity_changed();
		assert!(!load.attempt(key), "accepted read is never duplicated");
		assert!(
			load.attempt((2, Id(2), 7, 1)),
			"account and safety changes retire the attempt"
		);
	}
	fn member_hint_store(account: Id) -> Result<LocalStore, StoreError> {
		let mut store = LocalStore::open(std::path::Path::new(":memory:")).unwrap();
		store
			.save_member_displays(
				account,
				&[model::MemberDisplay {
					guild: Id(10),
					user: Id(900000),
					name: "Cached account".into(),
					username: Some("cached_account".into()),
					nick: Some("Cached nickname".into()),
				}],
			)
			.unwrap();
		Ok(store)
	}
	fn member_hint_rows(
		store: &mut Result<LocalStore, StoreError>,
		safety: &HistorySafety,
		account: Id,
		revision: u64,
	) -> Vec<model::MemberDisplay> {
		let Outcome::Members { result, .. } = execute(
			store,
			safety,
			account,
			safety.epoch(),
			Operation::LoadMembers {
				guild: Id(10),
				revision,
			},
		) else {
			panic!("member load")
		};
		result.unwrap()
	}
	#[test]
	fn visible_author_cache_worker_does_not_complete_the_gateway_query() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let account = state.user.as_ref().unwrap().id;
		let Some(client_core::Command::MemberSearch(live)) =
			state.request_author_members(&[Id(900000)])
		else {
			panic!("author lookup");
		};
		let request = state.visible_member_cache_request().unwrap();
		let revision = state.member_display_revision();
		let safety = HistorySafety::default();
		let mut store = member_hint_store(account);
		let Outcome::MemberDisplaysFound {
			request,
			revision,
			visible: true,
			result: Ok(rows),
			..
		} = execute(
			&mut store,
			&safety,
			account,
			safety.epoch(),
			Operation::FindMemberDisplays {
				request,
				revision,
				visible: true,
			},
		)
		else {
			panic!("visible member result");
		};
		assert!(state.hydrate_visible_member_displays(&request, revision, rows));
		assert_eq!(
			state.member_nickname(Id(20), Id(900000)),
			Some(Some("Cached nickname"))
		);
		assert_eq!(state.member_search[1].request.as_ref(), Some(&live));
		assert!(!state.member_search[1].finished);
	}
	#[test]
	fn cleanup_epoch_retries_the_current_local_query_without_hydrating_the_old_read() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let account = state.user.as_ref().unwrap().id;
		let generation = state.generation;
		let (send, commands) = mpsc::sync_channel(16);
		let (_, receive) = mpsc::sync_channel(16);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let mut store = member_hint_store(account);
		let client_core::Command::MemberSearchLocal(request) =
			state.prepare_member_search(channel, "Cached", 0).unwrap()
		else {
			panic!("local query");
		};
		let revision = state.member_display_revision();
		let read_epoch = cache.history.epoch();
		let Outcome::MemberDisplaysFound {
			result: Ok(old_rows),
			..
		} = execute(
			&mut store,
			&cache.history,
			account,
			read_epoch,
			Operation::FindMemberDisplays {
				request: request.clone(),
				revision,
				visible: false,
			},
		)
		else {
			panic!("old local read");
		};
		assert_eq!(old_rows.len(), 1);
		let mut search = super::super::member_cache_search::Search::default();
		search.sync_scope((generation, account));
		let mut profile = state.user.as_ref().unwrap().clone();
		profile.id = Id(900001);
		let event = client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
			guild,
			user: profile.id,
			profile: Some(profile),
			global_name: model::GlobalNameUpdate::Set,
			avatar: model::Patch::Absent,
			nick: model::Patch::Value("Unrelated changed nickname".into()),
			roles: model::Patch::Absent,
			departed: false,
		}));
		let target = member_display_target(&event);
		let before = state.member_display_cache_revision();
		state.apply(client_core::Envelope { generation, event });
		let mut cleanup = MemberDisplayCleanup::default();
		assert!(cleanup.observe(&state, target, before).is_none());
		let (generation, owner, operation) = cleanup.take().unwrap();
		assert!(cache.invalidate_scope(generation, owner, operation));
		assert!(!cache.history.allows(read_epoch));
		assert!(search.retry_epoch(
			&state,
			generation,
			&request,
			read_epoch,
			cache.history.epoch()
		));
		drop(old_rows);
		assert_eq!(state.member_nickname(channel, Id(900000)), None);
		let (_, owner, epoch, operation, _reservation) = commands.try_recv().unwrap();
		assert!(matches!(
			execute(&mut store, &cache.history, owner, epoch, operation),
			Outcome::Saved
		));
		let revision = state.member_display_revision();
		let Outcome::MemberDisplaysFound {
			result: Ok(rows), ..
		} = execute(
			&mut store,
			&cache.history,
			account,
			cache.history.epoch(),
			Operation::FindMemberDisplays {
				request: request.clone(),
				revision,
				visible: false,
			},
		)
		else {
			panic!("retried local read");
		};
		assert!(state.hydrate_member_search_displays(&request, revision, rows));
		assert_eq!(
			state.member_nickname(channel, Id(900000)),
			Some(Some("Cached nickname"))
		);
		assert!(
			state.member_search[0].local_only,
			"the local retry must not dispatch the Gateway lookup"
		);
		state
			.prepare_member_search(channel, "Superseding", 0)
			.unwrap();
		assert!(!search.retry_epoch(
			&state,
			generation,
			&request,
			read_epoch,
			cache.history.epoch()
		));
	}

	#[test]
	fn targeted_cache_after_live_eviction_uses_fresh_persisted_names() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let account = state.user.as_ref().unwrap().id;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let target = Id(900000);
		let safety = HistorySafety::default();
		let mut store = member_hint_store(account);
		let Some(client_core::Command::MemberSearch(old_request)) =
			state.search_members(channel, "nickname", 0)
		else {
			panic!("initial member lookup");
		};
		let started = state.member_display_revision();
		let Outcome::MemberDisplaysFound {
			result: Ok(old_rows),
			..
		} = execute(
			&mut store,
			&safety,
			account,
			safety.epoch(),
			Operation::FindMemberDisplays {
				request: old_request.clone(),
				revision: started,
				visible: false,
			},
		)
		else {
			panic!("delayed cache lookup");
		};
		let mut profile = state.user.as_ref().unwrap().clone();
		profile.id = target;
		profile.name = "Fresh display".into();
		profile.username = Some("fresh_account".into());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
				guild,
				user: target,
				profile: Some(profile),
				global_name: model::GlobalNameUpdate::Set,
				avatar: model::Patch::Absent,
				nick: model::Patch::Value("Fresh nickname".into()),
				roles: model::Patch::Value(vec![]),
				departed: false,
			})),
		});
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				safety.epoch(),
				Operation::SaveMembers(state.cached_member_displays(guild))
			),
			Outcome::Saved
		));
		for user in 1000000..1000128 {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
					guild,
					user: Id(user),
					profile: None,
					global_name: model::GlobalNameUpdate::Unchanged,
					avatar: model::Patch::Absent,
					nick: model::Patch::Value("Unrelated observation".into()),
					roles: model::Patch::Absent,
					departed: false,
				})),
			});
		}
		assert!(state.member_identity(channel, target).is_none());
		assert!(state.hydrate_member_search_displays(&old_request, started, old_rows));
		assert_eq!(
			state.member_nickname(channel, target),
			None,
			"the delayed read must not restore the old nickname"
		);
		let Some(client_core::Command::MemberSearch(request)) =
			state.search_members(channel, "Fresh", 0)
		else {
			panic!("fresh member lookup");
		};
		let revision = state.member_display_revision();
		let Outcome::MemberDisplaysFound {
			result: Ok(rows), ..
		} = execute(
			&mut store,
			&safety,
			account,
			safety.epoch(),
			Operation::FindMemberDisplays {
				request: request.clone(),
				revision,
				visible: false,
			},
		)
		else {
			panic!("fresh cache lookup");
		};
		assert!(state.hydrate_member_search_displays(&request, revision, rows));
		assert_eq!(
			state.member_nickname(channel, target),
			Some(Some("Fresh nickname"))
		);
	}

	#[test]
	fn resumed_session_reloads_member_hints_without_changing_history_epoch() {
		let mut state = test_support::demo_state();
		state.demo = false;
		let account = state.user.as_ref().unwrap().id;
		let safety = HistorySafety::default();
		let epoch = safety.epoch();
		let mut store = member_hint_store(account);
		let mut load = HistoryLoad::default();
		let generation = state.generation;
		let key = (generation, Id(10), 0, epoch);
		assert!(load.attempt(key));
		load.accepted(key);
		let revision = state.member_display_revision();
		let rows = member_hint_rows(&mut store, &safety, account, revision);
		state.hydrate_member_displays(Id(10), revision, rows);
		assert_eq!(
			state.member_nickname(Id(20), Id(900000)),
			Some(Some("Cached nickname"))
		);
		let stale = client_core::Envelope {
			generation: generation.wrapping_add(1),
			event: client_core::Event::Disconnected,
		};
		assert!(!load.observe_session(generation, &stale));
		state.apply(stale);
		assert!(
			!load.attempt(key),
			"obsolete session events cannot duplicate accepted reads"
		);
		let disconnected = client_core::Envelope {
			generation,
			event: client_core::Event::Disconnected,
		};
		assert!(load.observe_session(generation, &disconnected));
		state.apply(disconnected);
		assert!(state.member_identity(Id(20), Id(900000)).is_none());
		let resumed = client_core::Envelope {
			generation,
			event: client_core::Event::Resumed,
		};
		assert!(!load.observe_session(generation, &resumed));
		state.apply(resumed);
		assert_eq!(state.generation, generation);
		assert_eq!(safety.epoch(), epoch);
		assert!(load.attempt(key));
		load.accepted(key);
		let revision = state.member_display_revision();
		let rows = member_hint_rows(&mut store, &safety, account, revision);
		state.hydrate_member_displays(Id(10), revision, rows);
		assert_eq!(
			state.member_nickname(Id(20), Id(900000)),
			Some(Some("Cached nickname"))
		);
		load.capacity_changed();
		assert!(!load.attempt(key));
	}
	#[test]
	fn discarded_member_hints_retry_after_worker_progress_and_keep_live_updates() {
		let mut state = test_support::demo_state();
		state.demo = false;
		let account = state.user.as_ref().unwrap().id;
		let safety = HistorySafety::default();
		let mut store = member_hint_store(account);
		let mut load = HistoryLoad::default();
		let key = (state.generation, Id(10), 0, safety.epoch());
		assert!(load.attempt(key));
		load.accepted(key);
		let revision = state.member_display_revision();
		let rows = member_hint_rows(&mut store, &safety, account, revision);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: model::Patch::Absent,
				guild: Id(10),
				user: Id(900001),
				profile: None,
				nick: model::Patch::Value("Live nickname".into()),
				roles: model::Patch::Absent,
				departed: false,
			})),
		});
		assert_ne!(revision, state.member_display_revision());
		state.hydrate_member_displays(Id(10), revision, rows);
		assert!(
			state.member_identity(Id(20), Id(900000)).is_none(),
			"stale completion remains rejected"
		);
		assert!(
			!load.attempt(key),
			"repaint does not retry an accepted read"
		);
		// The worker completion is the explicit retry signal; it is not a frame timer.
		load.member_result(key, revision, &state);
		assert!(load.attempt(key));
		load.accepted(key);
		let revision = state.member_display_revision();
		let rows = member_hint_rows(&mut store, &safety, account, revision);
		state.hydrate_member_displays(Id(10), revision, rows);
		assert_eq!(
			state.member_nickname(Id(20), Id(900000)),
			Some(Some("Cached nickname"))
		);
		assert_eq!(
			state.member_nickname(Id(20), Id(900001)),
			Some(Some("Live nickname"))
		);
		load.capacity_changed();
		assert!(!load.attempt(key));
		load.retry((key.0, Id(11), key.2, key.3));
		assert!(
			!load.attempt(key),
			"another guild's completion cannot restart this read"
		);
	}
	#[test]
	fn member_load_completed_away_from_server_retries_after_return() {
		for home in [false, true] {
			let mut state = test_support::demo_state();
			state.demo = false;
			let account = state.user.as_ref().unwrap().id;
			let safety = HistorySafety::default();
			let mut store = member_hint_store(account);
			let mut load = HistoryLoad::default();
			let key = (state.generation, Id(10), 0, safety.epoch());
			assert!(load.attempt(key));
			load.accepted(key);
			let revision = state.member_display_revision();
			let rows = member_hint_rows(&mut store, &safety, account, revision);
			if home {
				state.arrived_home();
			} else {
				state.select(Id(22));
			}
			assert_eq!(
				state.member_display_revision(),
				revision,
				"ordinary navigation is not a new identity observation"
			);
			load.member_result(key, revision, &state);
			state.hydrate_member_displays(Id(10), revision, rows);
			assert!(
				state.member_identity(Id(20), Id(900000)).is_none(),
				"a completion outside its server remains rejected"
			);
			state.select(Id(20));
			assert!(
				load.attempt(key),
				"worker progress released the old server admission marker"
			);
			load.accepted(key);
			let revision = state.member_display_revision();
			let rows = member_hint_rows(&mut store, &safety, account, revision);
			load.member_result(key, revision, &state);
			state.hydrate_member_displays(Id(10), revision, rows);
			assert_eq!(
				state.member_nickname(Id(20), Id(900000)),
				Some(Some("Cached nickname"))
			);
			load.capacity_changed();
			assert!(
				!load.attempt(key),
				"successful hydration is not repeated on repaint"
			);
		}
	}
	#[test]
	fn scoped_cleanup_fences_old_writes_and_preserves_other_windows() {
		let (send, commands) = mpsc::sync_channel(4);
		let (_, receive) = mpsc::sync_channel(1);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		for (account, channel) in [(Id(1), Id(2)), (Id(1), Id(3)), (Id(9), Id(2))] {
			store
				.as_mut()
				.unwrap()
				.save_channel(
					account,
					channel,
					&[test_support::message(10, channel)],
					false,
				)
				.unwrap();
		}
		let old = cache.history.epoch();
		assert!(cache.queue(
			1,
			Id(1),
			Operation::SaveChannel {
				channel: Id(2),
				messages: vec![test_support::message(11, Id(2))],
				start: false
			}
		));
		assert!(cache.invalidate_scope(1, Id(1), Operation::ClearChannel { channel: Id(2) }));
		assert!(!cache.history.allows(old));
		while let Ok((_, account, epoch, op, _reservation)) = commands.try_recv() {
			assert!(matches!(
				execute(&mut store, &cache.history, account, epoch, op),
				Outcome::Saved
			));
		}
		let db = store.as_ref().unwrap();
		assert!(db.load_channel(Id(1), Id(2)).unwrap().is_empty());
		assert_eq!(db.load_channel(Id(1), Id(3)).unwrap().len(), 1);
		assert_eq!(db.load_channel(Id(9), Id(2)).unwrap().len(), 1);
		assert!(cache.history.allows(cache.history.epoch()));
		let mut unavailable = Err(StoreError::Unavailable);
		assert!(matches!(
			execute(
				&mut unavailable,
				&cache.history,
				Id(1),
				cache.history.epoch(),
				Operation::ClearChannel { channel: Id(3) }
			),
			Outcome::ScopedCleanupFailed { account: Id(1) }
		));
		assert!(!cache.history.allows(cache.history.epoch()));
	}
	#[test]
	fn member_storage_worker_roundtrip_and_deletion_are_epoch_fenced() {
		let safety = HistorySafety::default();
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		let row = model::MemberDisplay {
			guild: Id(10),
			user: Id(2),
			name: "Synthetic".into(),
			username: Some("account".into()),
			nick: Some("Nickname".into()),
		};
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				0,
				Operation::SaveMembers(vec![row.clone()])
			),
			Outcome::Saved
		));
		let Outcome::Members {
			result: Ok(rows),
			epoch,
			..
		} = execute(
			&mut store,
			&safety,
			Id(1),
			0,
			Operation::LoadMembers {
				guild: Id(10),
				revision: 0,
			},
		)
		else {
			panic!("worker read");
		};
		assert_eq!(rows, vec![row.clone()]);
		safety.invalidate();
		assert!(!safety.allows(epoch));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				safety.epoch(),
				Operation::RemoveMember {
					guild: Id(10),
					user: Some(Id(2))
				}
			),
			Outcome::Saved
		));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(1),
				0,
				Operation::SaveMembers(vec![row])
			),
			Outcome::Saved
		));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(Id(1), Id(10))
				.unwrap()
				.is_empty()
		);
	}

	#[test]
	fn scoped_cleanup_retries_member_save_without_undoing_an_explicit_clear() {
		let (send, commands) = mpsc::sync_channel(4);
		let (_, receive) = mpsc::sync_channel(4);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let mut state = test_support::demo_state();
		state.demo = false;
		let account = state.user.as_ref().unwrap().id;
		let row = model::MemberDisplay {
			guild: Id(10),
			user: Id(900000),
			name: "Synthetic".into(),
			username: Some("account".into()),
			nick: Some("Current nickname".into()),
		};
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		let mut cleared = MemberCacheClear::default();
		let first = member_save_key(&state, Id(10), cache.history.epoch(), &cleared).unwrap();
		assert!(cache.queue(
			state.generation,
			account,
			Operation::SaveMembers(vec![row.clone()])
		));
		assert!(cache.invalidate_scope(
			state.generation,
			account,
			Operation::ClearChannel { channel: Id(20) },
		));
		while let Ok((_, owner, epoch, operation, _reservation)) = commands.try_recv() {
			assert!(matches!(
				execute(&mut store, &cache.history, owner, epoch, operation),
				Outcome::Saved
			));
		}
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(account, Id(10))
				.unwrap()
				.is_empty()
		);
		let retry = member_save_key(&state, Id(10), cache.history.epoch(), &cleared).unwrap();
		assert_ne!(
			first, retry,
			"scoped invalidation must retry the retired save"
		);
		assert!(cache.queue(
			state.generation,
			account,
			Operation::SaveMembers(vec![row.clone()])
		));
		let (_, owner, epoch, operation, _reservation) = commands.try_recv().unwrap();
		assert!(matches!(
			execute(&mut store, &cache.history, owner, epoch, operation),
			Outcome::Saved
		));
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_member_displays(account, Id(10))
				.unwrap(),
			vec![row.clone()]
		);

		// Full clearing suppresses retained hints for each guild independently. A
		// fresh observation in one guild must not restore another guild's old rows.
		state.hydrate_member_displays(Id(10), state.member_display_revision(), vec![row]);
		let identity = model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: model::Patch::Absent,
			guild: Id(10),
			user: Id(900001),
			profile: None,
			nick: model::Patch::Value("Fresh".into()),
			roles: model::Patch::Absent,
			departed: false,
		};
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberIdentity(Box::new(identity.clone())),
		});
		cleared.reset(&state);
		assert!(cleared.blocked.contains(&Id(10)));
		cleared.blocked.insert(Id(11));
		assert!(member_save_key(&state, Id(10), cache.history.epoch(), &cleared).is_none());
		let before = state.member_display_revision();
		let saved_revision = state.member_display_cache_revision();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberIdentity(Box::new(identity)),
		});
		assert_eq!(state.member_display_cache_revision(), saved_revision);
		assert_ne!(state.member_display_revision(), before);
		cleared.observe(&state, Some(Id(10)), before);
		assert!(member_save_key(&state, Id(10), cache.history.epoch(), &cleared).is_some());
		assert!(member_save_key(&state, Id(11), cache.history.epoch(), &cleared).is_none());
		state.generation = state.generation.wrapping_add(1);
		assert!(member_save_key(&state, Id(11), cache.history.epoch(), &cleared).is_some());
	}

	#[test]
	fn complete_display_cleanup_batches_evictions_before_the_next_snapshot() {
		for mode in ["nickname", "cleared", "profile"] {
			for updates in [1_u64, 129] {
				let mut state = test_support::demo_state();
				state.demo = false;
				state.auth = client_core::auth::AuthState::Authenticated;
				state.gateway_connected = true;
				let account = state.user.as_ref().unwrap().id;
				let channel = state.selected.unwrap();
				let guild = state.channel(channel).unwrap().guild.unwrap();
				let target = Id(900000);
				let (send, commands) = mpsc::sync_channel(16);
				let (_, receive) = mpsc::sync_channel(16);
				let cache = Cache {
					send,
					receive,
					budget: Arc::new(Budget::default()),
					sync_budget: Arc::new(Budget {
						limit: SYNC_QUEUE_BYTES,
						..Budget::default()
					}),
					protected_history: Arc::new(Mutex::new(HistoryPins::default())),
					history: Arc::new(HistorySafety::default()),
				};
				let mut store = member_hint_store(account);
				store
					.as_mut()
					.unwrap()
					.save_channel(
						account,
						channel,
						&[test_support::message(7000000, channel)],
						false,
					)
					.unwrap();
				let old_epoch = cache.history.epoch();
				let old_rows = store
					.as_ref()
					.unwrap()
					.load_member_displays_for_users(account, guild, &[target])
					.unwrap();
				assert_eq!(old_rows.len(), 1);
				assert!(cache.queue(state.generation, account, Operation::SaveMembers(old_rows)));
				let mut cleanups = 0;
				let identity_batch = client_core::EVENT_SLOTS + 8;
				// The worker stays paused, and no display snapshot is taken until all drains finish.
				for first in (0..updates).step_by(identity_batch) {
					let mut cleanup = MemberDisplayCleanup::default();
					for offset in first..updates.min(first + identity_batch as u64) {
						let mut profile = state.user.as_ref().unwrap().clone();
						profile.id = Id(target.0 + offset);
						profile.name = format!("Fresh display {offset}");
						profile.username = Some(format!("fresh_{offset}").into());
						let event =
							client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
								guild,
								user: profile.id,
								profile: Some(profile),
								global_name: model::GlobalNameUpdate::Set,
								avatar: model::Patch::Absent,
								nick: if offset == 0 && mode == "profile" {
									model::Patch::Absent
								} else if offset == 0 && mode == "cleared" {
									model::Patch::Null
								} else {
									model::Patch::Value(format!("Fresh nickname {offset}"))
								},
								roles: model::Patch::Absent,
								departed: false,
							}));
						let target = member_display_target(&event);
						let before = state.member_display_cache_revision();
						state.apply(client_core::Envelope {
							generation: state.generation,
							event,
						});
						assert!(cleanup.observe(&state, target, before).is_none());
					}
					let (generation, owner, operation) = cleanup.take().unwrap();
					assert!(
						matches!(&operation, Operation::RemoveMemberDisplays(rows) if rows.len() <= MEMBER_DISPLAY_CLEANUP_LIMIT)
					);
					assert!(cache.invalidate_scope(generation, owner, operation));
					cleanups += 1;
				}
				assert_eq!(cleanups, if updates == 129 { 9 } else { 1 });
				assert!(!cache.history.allows(old_epoch));
				assert!(
					cache.history.allows(cache.history.epoch()),
					"batched work must fit the paused 16-item queue"
				);
				let rows = state.cached_member_displays(guild);
				assert_eq!(
					rows.iter().any(|row| row.user == target),
					updates == 1 && mode != "profile"
				);
				assert!(cache.queue(state.generation, account, Operation::SaveMembers(rows)));
				let mut completed_cleanups = 0;
				while let Ok((_, owner, epoch, operation, _reservation)) = commands.try_recv() {
					if matches!(operation, Operation::RemoveMemberDisplays(_)) {
						completed_cleanups += 1;
					}
					assert!(!matches!(operation, Operation::ClearHistory));
					assert!(matches!(
						execute(&mut store, &cache.history, owner, epoch, operation),
						Outcome::Saved
					));
				}
				assert_eq!(completed_cleanups, cleanups);
				assert_eq!(
					store
						.as_ref()
						.unwrap()
						.load_channel(account, channel)
						.unwrap()
						.len(),
					1
				);
				if updates == 129 {
					let retained = store
						.as_ref()
						.unwrap()
						.load_member_displays_for_users(account, guild, &[Id(target.0 + 128)])
						.unwrap();
					assert_eq!(retained.len(), 1);
					assert_eq!(retained[0].nick.as_deref(), Some("Fresh nickname 128"));
				}
				let Some(client_core::Command::MemberSearch(request)) =
					state.search_members(channel, &target.to_string(), 0)
				else {
					panic!("current lookup");
				};
				let revision = state.member_display_revision();
				let Outcome::MemberDisplaysFound {
					result: Ok(rows), ..
				} = execute(
					&mut store,
					&cache.history,
					account,
					cache.history.epoch(),
					Operation::FindMemberDisplays {
						request: request.clone(),
						revision,
						visible: false,
					},
				)
				else {
					panic!("current cache read");
				};
				if updates == 1 && mode != "profile" {
					assert_eq!(rows.len(), 1);
					assert_eq!(rows[0].name, "Fresh display 0");
					assert_eq!(
						rows[0].nick.as_deref(),
						if mode == "cleared" {
							None
						} else {
							Some("Fresh nickname 0")
						}
					);
				} else {
					assert!(
						rows.is_empty(),
						"eviction before snapshot cannot preserve the stale disk row"
					);
				}
				assert!(state.hydrate_member_search_displays(&request, revision, rows));
				if updates == 129 {
					assert_eq!(state.member_nickname(channel, target), None);
				}
			}
		}
	}

	#[test]
	fn unchanged_display_observations_do_not_enqueue_cleanup_and_scope_handoff_keeps_owner() {
		let mut state = test_support::demo_state();
		state.demo = false;
		let old_scope = (state.generation, state.user.as_ref().unwrap().id);
		let guild = state
			.channel(state.selected.unwrap())
			.unwrap()
			.guild
			.unwrap();
		let mut profile = state.user.as_ref().unwrap().clone();
		profile.id = Id(900000);
		let update = model::MemberIdentity {
			guild,
			user: profile.id,
			profile: Some(profile),
			global_name: model::GlobalNameUpdate::Set,
			avatar: model::Patch::Absent,
			nick: model::Patch::Value("Current nickname".into()),
			roles: model::Patch::Absent,
			departed: false,
		};
		let mut cleanup = MemberDisplayCleanup::default();
		for index in 0..64 {
			let event = client_core::Event::MemberIdentity(Box::new(update.clone()));
			let target = member_display_target(&event);
			let before = state.member_display_cache_revision();
			state.apply(client_core::Envelope {
				generation: state.generation,
				event,
			});
			assert!(cleanup.observe(&state, target, before).is_none());
			if index != 0 {
				assert!(changed_member_display_cleanup(&state, target, before).is_none());
			}
		}
		state.generation = state.generation.wrapping_add(1);
		state.user.as_mut().unwrap().id = Id(old_scope.1.0 + 1);
		let (generation, account, operation) = cleanup
			.observe(&state, None, state.member_display_cache_revision())
			.unwrap();
		assert_eq!((generation, account), old_scope);
		assert!(
			matches!(operation, Operation::RemoveMemberDisplays(targets) if targets == [(guild, update.user)])
		);
		assert!(cleanup.take().is_none());
	}

	#[test]
	fn partial_live_nickname_evicts_old_disk_hint_across_reopen() {
		use std::time::{SystemTime, UNIX_EPOCH};
		let root = std::env::temp_dir().join(format!(
			"serein-partial-nickname-{}-{}",
			std::process::id(),
			SystemTime::now()
				.duration_since(UNIX_EPOCH)
				.unwrap()
				.as_nanos(),
		));
		std::fs::create_dir(&root).unwrap();
		for (case, nick, typing) in [
			(0, model::Patch::Null, false),
			(1, model::Patch::Value("New nickname".into()), false),
			(2, model::Patch::Value("Typing nickname".into()), true),
		] {
			let path = root.join(format!("{case}.sqlite3"));
			let mut state = test_support::demo_state();
			state.demo = false;
			let account = state.user.as_ref().unwrap().id;
			let mut store = Ok(LocalStore::open(&path).unwrap());
			let stale = model::MemberDisplay {
				guild: Id(10),
				user: Id(900000),
				name: "Cached".into(),
				username: Some("cached".into()),
				nick: Some("Old nickname".into()),
			};
			store
				.as_mut()
				.unwrap()
				.save_member_displays(account, &[stale])
				.unwrap();
			let safety = HistorySafety::default();
			let rows = member_hint_rows(
				&mut store,
				&safety,
				account,
				state.member_display_revision(),
			);
			state.hydrate_member_displays(Id(10), state.member_display_revision(), rows);
			let identity = model::MemberIdentity {
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: model::Patch::Absent,
				guild: Id(10),
				user: Id(900000),
				profile: None,
				nick,
				roles: model::Patch::Absent,
				departed: false,
			};
			let make_event = || {
				if typing {
					client_core::Event::Typing(client_core::typing::Signal {
						channel: Id(20),
						user: Id(900000),
						timestamp: SystemTime::now()
							.duration_since(UNIX_EPOCH)
							.unwrap()
							.as_secs(),
						member: Some(Box::new(identity.clone())),
					})
				} else {
					client_core::Event::MemberIdentity(Box::new(identity.clone()))
				}
			};
			let departed = model::MemberIdentity {
				avatar: model::Patch::Absent,
				departed: true,
				..identity.clone()
			};
			let stale_departure = client_core::Envelope {
				generation: state.generation.wrapping_sub(1),
				event: client_core::Event::MemberIdentity(Box::new(departed.clone())),
			};
			assert_eq!(departed_member_target(&state, &stale_departure), None);
			let current_departure = client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::MemberIdentity(Box::new(departed)),
			};
			assert_eq!(
				departed_member_target(&state, &current_departure),
				Some((Id(10), Id(900000)))
			);
			let target = member_display_target(&make_event());
			let before = state.member_display_cache_revision();
			state.apply(client_core::Envelope {
				generation: state.generation.wrapping_sub(1),
				event: make_event(),
			});
			assert_eq!(
				changed_member_display_cleanup(&state, target, before),
				None,
				"stale generation is ignored"
			);
			let expired = client_core::Event::Typing(client_core::typing::Signal {
				channel: Id(20),
				user: Id(900000),
				timestamp: 0,
				member: Some(Box::new(identity.clone())),
			});
			let target = member_display_target(&expired);
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: expired,
			});
			assert_eq!(
				changed_member_display_cleanup(&state, target, before),
				None,
				"expired typing is ignored"
			);
			let target = member_display_target(&make_event());
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: make_event(),
			});
			assert_eq!(
				changed_member_display_cleanup(&state, target, before),
				Some((Id(10), Id(900000)))
			);
			safety.invalidate();
			let epoch = safety.epoch();
			assert!(matches!(
				execute(
					&mut store,
					&safety,
					account,
					epoch,
					Operation::RemoveMember {
						guild: Id(10),
						user: Some(Id(900000))
					}
				),
				Outcome::Saved
			));
			drop(store);
			assert!(
				LocalStore::open(&path)
					.unwrap()
					.load_member_displays(account, Id(10))
					.unwrap()
					.is_empty()
			);
		}
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn credential_cleanup_retries_full_storage_without_repeating_keyring_deletion() {
		let (send, commands) = mpsc::sync_channel(1);
		let (_, receive) = mpsc::sync_channel(1);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let active = Id(1);
		let removed = Id(2);
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		let saved = model::SavedAccount {
			id: removed,
			name: "Synthetic".into(),
			display: None,
			avatar: None,
			discriminator: 0,
			has_token: true,
		};
		store.as_mut().unwrap().save_account(&saved).unwrap();
		for account in [active, removed] {
			store
				.as_mut()
				.unwrap()
				.save_draft(account, Id(7), "offline draft")
				.unwrap();
		}
		assert!(cache.queue(1, active, Operation::LoadDrafts));
		let mut pending = AccountForgets::default();
		// The OS deletion has already succeeded. A full storage queue retains only
		// this acknowledgment; retrying must not repeat a credential-store operation.
		assert!(pending.request(1, removed));
		assert_eq!(pending.next(2, Some(active)), Some(removed));
		assert!(!cache.queue(2, removed, Operation::Forget));
		assert!(pending.contains(removed));
		assert_eq!(store.as_ref().unwrap().accounts().unwrap()[0].id, removed);
		let (_, _, _, _, reservation) = commands.try_recv().unwrap();
		drop(reservation);
		assert!(cache.queue(2, pending.next(2, Some(active)).unwrap(), Operation::Forget));
		pending.queued(removed);
		let (generation, account, epoch, operation, reservation) = commands.try_recv().unwrap();
		assert_eq!((generation, account), (2, removed));
		assert!(matches!(
			execute(&mut store, &cache.history, account, epoch, operation),
			Outcome::Saved
		));
		drop(reservation);
		assert!(!pending.pending());
		assert!(store.as_ref().unwrap().accounts().unwrap().is_empty());
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_drafts(removed)
				.unwrap()
				.is_empty()
		);
		assert_eq!(
			store.as_ref().unwrap().load_drafts(active).unwrap()[&Id(7)],
			"offline draft"
		);
		assert!(commands.try_recv().is_err());
		assert_eq!(*cache.budget.used.lock().unwrap(), 0);
	}

	#[test]
	fn pending_account_cleanup_is_bounded_and_new_signin_supersedes_only_its_account() {
		let mut pending = AccountForgets::default();
		for id in 1..=model::MAX_SAVED_ACCOUNTS as u64 {
			assert!(pending.request(1, Id(id)));
			assert!(
				pending.request(1, Id(id)),
				"repeated acknowledgments coalesce"
			);
		}
		assert!(!pending.request(1, Id(9)));
		assert_eq!(pending.next(2, Some(Id(1))), Some(Id(2)));
		assert!(
			!pending.contains(Id(1)),
			"new sign-in preserves its account data"
		);
		assert!(pending.contains(Id(2)), "switching preserves other cleanup");
		for id in 2..=model::MAX_SAVED_ACCOUNTS as u64 {
			assert_eq!(pending.next(3, None), Some(Id(id)));
			pending.queued(Id(id));
		}
		assert!(!pending.pending());
		assert!(
			pending.request(3, Id(1)),
			"a later explicit deletion is independent"
		);
		assert_eq!(pending.next(3, Some(Id(1))), Some(Id(1)));
	}

	#[test]
	fn premium_unicode_drafts_enter_the_worker_and_reject_only_oversized_content() {
		use model::message_options::{MAX_DRAFT_CONTENT_BYTES, MAX_PREMIUM_CONTENT};
		let (send, commands) = mpsc::sync_channel(16);
		let (_, receive) = mpsc::sync_channel(16);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let draft = format!("@silent\u{2003}{}", "🦀".repeat(MAX_PREMIUM_CONTENT));
		assert!(cache.queue(
			7,
			Id(1),
			Operation::SaveDraft {
				channel: Id(2),
				content: draft.clone()
			}
		));
		let (_, _, epoch, operation, reservation) = commands.try_recv().unwrap();
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		assert!(matches!(
			execute(&mut store, &cache.history, Id(1), epoch, operation),
			Outcome::Saved
		));
		assert_eq!(
			store.as_ref().unwrap().load_drafts(Id(1)).unwrap()[&Id(2)],
			draft
		);
		drop(reservation);
		assert_eq!(*cache.budget.used.lock().unwrap(), 0);
		assert!(!cache.queue(
			7,
			Id(1),
			Operation::SaveDraft {
				channel: Id(2),
				content: "x".repeat(MAX_DRAFT_CONTENT_BYTES + 1)
			}
		));
		assert!(commands.try_recv().is_err());
	}

	#[test]
	fn shortcut_restore_waits_for_queue_space_without_losing_or_duplicating_the_request() {
		let (send, commands) = mpsc::sync_channel(16);
		let (_, receive) = mpsc::sync_channel(16);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let mut view = ui::MessagingUi::default();
		view.channel_preferences_reload = true;
		for _ in 0..16 {
			assert!(cache.queue(7, Id(0), Operation::LoadAppearance));
		}
		assert!(!crate::queue_channel_preferences(
			Some(&cache),
			&mut view,
			7,
			Id(42)
		));
		assert!(view.channel_preferences_reload);
		assert!(!view.channel_preferences_load_pending);
		assert!(view.channel_preferences_status.is_empty());
		commands.try_recv().unwrap();
		assert!(crate::queue_channel_preferences(
			Some(&cache),
			&mut view,
			7,
			Id(42)
		));
		assert!(!view.channel_preferences_reload);
		assert!(view.channel_preferences_load_pending);
		// A second READY or retry click while loading cannot enqueue another restore.
		view.channel_preferences_reload = true;
		assert!(!crate::queue_channel_preferences(
			Some(&cache),
			&mut view,
			7,
			Id(42)
		));
		for _ in 0..15 {
			assert!(matches!(
				commands.try_recv().unwrap().3,
				Operation::LoadAppearance
			));
		}
		let (generation, account, epoch, operation, reservation) = commands.try_recv().unwrap();
		assert_eq!((generation, account), (7, Id(42)));
		assert!(matches!(operation, Operation::LoadChannelPreferences));
		assert!(commands.try_recv().is_err());
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		let preferences = model::ChannelPreferences {
			favorites: vec![Id(19)],
			pinned: vec![Id(20)],
			collapsed_categories: vec![Id(21)],
			last_channels: vec![(Id(22), Id(23))],
		};
		store
			.as_ref()
			.unwrap()
			.save_channel_preferences(account, &preferences)
			.unwrap();
		let Outcome::ChannelPreferences(Ok(restored)) =
			execute(&mut store, &cache.history, account, epoch, operation)
		else {
			panic!("Expected restored shortcuts");
		};
		assert_eq!(restored, preferences);
		drop(reservation);
		assert_eq!(*cache.budget.used.lock().unwrap(), 0);
		view.clear();
		assert!(!view.channel_preferences_load_pending && !view.channel_preferences_reload);
		view.channel_preferences_reload = true;
		assert!(!crate::queue_channel_preferences(
			None,
			&mut view,
			8,
			Id(43)
		));
		assert!(!view.channel_preferences_reload);
		assert!(!view.channel_preferences_status.is_empty());
	}

	#[test]
	fn notification_choice_survives_a_full_queue_and_database_reopen() {
		let root = std::env::temp_dir().join(format!(
			"serein-notification-restart-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::create_dir(&root).unwrap();
		let path = root.join("client.sqlite3");
		let (send, commands) = mpsc::sync_channel(16);
		let (_, receive) = mpsc::sync_channel(16);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let initial = LocalStore::open(&path)
			.unwrap()
			.app_preferences()
			.unwrap()
			.notifications_enabled;
		for enabled in [!initial, initial] {
			let mut store = Ok(LocalStore::open(&path).unwrap());
			let mut settings = crate::app_settings::Settings {
				current: store.as_ref().unwrap().app_preferences().unwrap(),
				loaded: true,
				..Default::default()
			};
			let mut view = ui::MessagingUi::default();
			settings.apply(&mut view);
			view.notifications_enabled = enabled;
			settings.observe(&view);
			for _ in 0..16 {
				assert!(cache.queue(1, Id(0), Operation::LoadAppearance));
			}
			assert!(!settings.save(Some(&cache), 1));
			assert!(settings.state.needs_attention());
			for _ in 0..16 {
				commands.try_recv().unwrap();
			}
			// The next frame retries without another click, after cache work has drained.
			settings.observe(&view);
			assert!(settings.save(Some(&cache), 2));
			assert!(!settings.save(Some(&cache), 2));
			let (_, account, epoch, operation, _reservation) = commands.try_recv().unwrap();
			assert!(matches!(
				execute(&mut store, &cache.history, account, epoch, operation),
				Outcome::AppPreferencesSaved(Ok(()))
			));
			drop(store);
			let reopened = LocalStore::open(&path).unwrap();
			let restored = crate::app_settings::Settings {
				current: reopened.app_preferences().unwrap(),
				..Default::default()
			};
			let mut restarted_view = ui::MessagingUi::default();
			restored.apply(&mut restarted_view);
			assert_eq!(restarted_view.notifications_enabled, enabled);
		}
		std::fs::remove_file(path).unwrap();
		std::fs::remove_dir(root).unwrap();
	}

	#[test]
	fn minimize_to_tray_results_are_independent_of_account_history() {
		let safety = HistorySafety::default();
		safety.fail();
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		assert!(matches!(
			execute(&mut store, &safety, Id(0), 0, Operation::LoadMinimizeToTray),
			Outcome::MinimizeToTray(Ok(true))
		));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				Id(0),
				0,
				Operation::SaveMinimizeToTray(false)
			),
			Outcome::MinimizeToTraySaved(Ok(()))
		));
		assert!(matches!(
			execute(&mut store, &safety, Id(9), 0, Operation::LoadMinimizeToTray),
			Outcome::MinimizeToTray(Ok(false))
		));
		let mut unavailable = Err(StoreError::Unavailable);
		assert!(matches!(
			execute(
				&mut unavailable,
				&safety,
				Id(0),
				0,
				Operation::LoadMinimizeToTray
			),
			Outcome::MinimizeToTray(Err(StoreError::Unavailable))
		));
		assert!(matches!(
			execute(
				&mut unavailable,
				&safety,
				Id(0),
				0,
				Operation::SaveMinimizeToTray(false)
			),
			Outcome::MinimizeToTraySaved(Err(StoreError::Unavailable))
		));

		{
			let safety = HistorySafety::default();
			safety.block();
			let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
			assert!(matches!(
				execute(&mut store, &safety, Id(0), 0, Operation::LoadGameActivity),
				Outcome::GameActivity(Ok(false))
			));
			assert!(matches!(
				execute(
					&mut store,
					&safety,
					Id(0),
					0,
					Operation::SaveGameActivity(true)
				),
				Outcome::GameActivitySaved(Ok(()))
			));
			assert!(matches!(
				execute(&mut store, &safety, Id(9), 0, Operation::LoadGameActivity),
				Outcome::GameActivity(Ok(true))
			));
			let mut unavailable = Err(StoreError::Unavailable);
			assert!(matches!(
				execute(
					&mut unavailable,
					&safety,
					Id(0),
					0,
					Operation::LoadGameActivity
				),
				Outcome::GameActivity(Err(StoreError::Unavailable))
			));
			assert!(matches!(
				execute(
					&mut unavailable,
					&safety,
					Id(0),
					0,
					Operation::SaveGameActivity(false)
				),
				Outcome::GameActivitySaved(Err(StoreError::Unavailable))
			));
		}

		{
			let safety = HistorySafety::default();
			let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
			let value = model::ReadingPreferences {
				zoom_percent: 125,
				sidebar_width: 300,
				show_members: false,
				show_members_dms: false,
				compact_messages: false,
				double_click_reaction_enabled: false,
				double_click_reaction: 0,
				animate_gifs: false,
				smooth_scrolling: true,
				scroll_speed_percent: 100,
				hide_media_links: true,
				confirm_external_links: true,
			};
			store
				.as_mut()
				.unwrap()
				.save_draft(Id(1), Id(2), "Synthetic draft")
				.unwrap();
			safety.block(); // History cleanup does not prohibit application settings.
			assert!(matches!(
				execute(
					&mut store,
					&safety,
					Id(0),
					0,
					Operation::SaveReadingPreferences(value)
				),
				Outcome::ReadingPreferencesSaved(Ok(()))
			));
			assert!(matches!(execute(&mut store, &safety, Id(9), 0,
            Operation::LoadReadingPreferences), Outcome::ReadingPreferences(Ok(stored)) if stored == value));
			assert_eq!(
				store.as_ref().unwrap().load_drafts(Id(1)).unwrap()[&Id(2)],
				"Synthetic draft"
			);
			let mut unavailable = Err(StoreError::Unavailable);
			assert!(matches!(
				execute(
					&mut unavailable,
					&safety,
					Id(0),
					0,
					Operation::LoadReadingPreferences
				),
				Outcome::ReadingPreferences(Err(StoreError::Unavailable))
			));
			assert!(matches!(
				execute(
					&mut unavailable,
					&safety,
					Id(0),
					0,
					Operation::SaveReadingPreferences(value)
				),
				Outcome::ReadingPreferencesSaved(Err(StoreError::Unavailable))
			));
		}
	}

	#[test]
	fn cleanup_acknowledgements_cross_generations_without_reopening_early() {
		let safety = HistorySafety::default();
		let mut clears = HistoryClears::default();
		safety.block();
		assert!(clears.request(Id(1)));
		clears.queued(Id(1)); // Cleanup queued before logout.
		assert!(clears.request(Id(9))); // Another account needs cleanup after login.
		assert!(!clears.acknowledge(&safety)); // Old-generation completion still counts.
		assert!(!safety.allows(safety.epoch()));
		clears.queued(Id(9));
		assert!(clears.request(Id(9))); // A second deletion races that account's first clear.
		assert!(!clears.acknowledge(&safety));
		clears.queued(Id(9));
		assert!(clears.acknowledge(&safety));
		assert!(safety.allows(safety.epoch()));
		assert!(!clears.pending());
		safety.block();
		for account in 1..=16 {
			assert!(clears.request(Id(account)));
		}
		assert!(!clears.request(Id(17)));
		for account in 1..=16 {
			assert_eq!(clears.next(), Some(Id(account)));
			clears.queued(Id(account));
		}
		safety.fail();
		for _ in 0..16 {
			clears.acknowledge(&safety);
		}
		// Even a racing clear of the separate blocked atomic cannot override failure.
		safety.blocked.store(false, Ordering::SeqCst);
		assert!(!safety.allows(safety.epoch()));
	}

	#[test]
	fn deletion_epoch_rejects_queued_saves_and_returning_loads() {
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		let safety = HistorySafety::default();
		let account = Id(1);
		let channel = Id(2);
		let snapshot = vec![
			test_support::message(10, channel),
			test_support::message(11, channel),
		];
		for owner in [account, Id(9)] {
			assert!(matches!(
				execute(
					&mut store,
					&safety,
					owner,
					0,
					Operation::SaveChannel {
						channel,
						messages: snapshot.clone(),
						start: false,
					}
				),
				Outcome::Saved
			));
		}
		let loaded = execute(
			&mut store,
			&safety,
			account,
			0,
			Operation::LoadChannel {
				channel,
				request: 5,
			},
		);
		let Outcome::Channel {
			epoch, messages, ..
		} = loaded
		else {
			panic!()
		};
		assert_eq!(messages.len(), 2);
		let mut state = client_core::State {
			user: Some(test_support::message(1, channel).author),
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			selected: Some(channel),
			freshness: model::Freshness::Fresh,
			channels: vec![model::Channel {
				id: channel,
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic DM".into(),
				icon: None,
				position: 0,
				recipients: vec![],
				last_message: None,
				member_list_id: None,
				tags: None,
				message_count: None,
			}],
			..Default::default()
		};
		let mut reply = test_support::message(12, channel);
		reply.kind = 19;
		reply.reply_to = Some(Id(10));
		reply.reply_deleted = true;
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Message(reply),
		});
		let deletions = state.take_reply_deletions();
		assert_eq!(deletions, vec![(channel, Id(10))]);
		safety.invalidate();
		assert!(!safety.allows(epoch));
		assert!(matches!(
			execute(
				&mut store,
				&safety,
				account,
				0,
				Operation::LoadChannel {
					channel,
					request: 5
				}
			),
			Outcome::Saved
		));
		execute(
			&mut store,
			&safety,
			account,
			safety.epoch(),
			Operation::DeleteMessages {
				channel,
				ids: deletions.into_iter().map(|(_, id)| id).collect(),
			},
		);
		execute(
			&mut store,
			&safety,
			account,
			0,
			Operation::SaveChannel {
				channel,
				messages: snapshot,
				start: false,
			},
		);
		let current = store
			.as_ref()
			.unwrap()
			.load_channel(account, channel)
			.unwrap();
		assert_eq!(
			current.iter().map(|m| m.id).collect::<Vec<_>>(),
			vec![Id(11)]
		);
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_channel(Id(9), channel)
				.unwrap()
				.len(),
			2
		);
	}

	#[test]
	fn full_queue_blocks_history_until_scoped_cleanup_and_io_failure_stays_closed() {
		let (send, commands) = mpsc::sync_channel(16);
		let (_, receive) = mpsc::sync_channel(16);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			sync_budget: Arc::new(Budget {
				limit: SYNC_QUEUE_BYTES,
				..Budget::default()
			}),
			protected_history: Arc::new(Mutex::new(HistoryPins::default())),
			history: Arc::new(HistorySafety::default()),
		};
		let account = Id(1);
		let channel = Id(2);
		for _ in 0..16 {
			assert!(cache.queue(0, account, Operation::LoadDrafts));
		}
		assert!(!cache.delete_messages(0, account, channel, vec![Id(10)]));
		assert!(!cache.history.allows(cache.history.epoch()));
		assert!(!cache.queue(
			0,
			account,
			Operation::LoadChannel {
				channel,
				request: 1
			}
		));
		assert!(!cache.queue(
			0,
			account,
			Operation::SaveChannel {
				channel,
				messages: vec![],
				start: false,
			}
		));
		for _ in 0..16 {
			commands.try_recv().unwrap();
		}
		assert!(cache.queue(1, account, Operation::ClearHistory));
		let (generation, owner, epoch, operation, _reservation) = commands.try_recv().unwrap();
		assert_eq!((generation, owner), (1, account));
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		for owner in [account, Id(9)] {
			store
				.as_mut()
				.unwrap()
				.save_channel(owner, channel, &[test_support::message(10, channel)], false)
				.unwrap();
			store
				.as_mut()
				.unwrap()
				.save_draft(owner, channel, "preserved draft")
				.unwrap();
		}
		assert!(matches!(
			execute(&mut store, &cache.history, owner, epoch, operation),
			Outcome::HistoryCleared
		));
		cache.history.cleared();
		assert!(cache.history.allows(cache.history.epoch()));
		assert!(
			store
				.as_ref()
				.unwrap()
				.load_channel(account, channel)
				.unwrap()
				.is_empty()
		);
		assert_eq!(
			store
				.as_ref()
				.unwrap()
				.load_channel(Id(9), channel)
				.unwrap()
				.len(),
			1
		);
		assert_eq!(
			store.as_ref().unwrap().load_drafts(account).unwrap()[&channel],
			"preserved draft"
		);
		let mut failed = Err(StoreError::Unavailable);
		assert!(matches!(
			execute(
				&mut failed,
				&cache.history,
				account,
				epoch,
				Operation::DeleteMessages {
					channel,
					ids: vec![Id(10)]
				}
			),
			Outcome::Failed { .. }
		));
		cache.history.cleared();
		assert!(!cache.history.allows(cache.history.epoch()));
		assert!(cache.queue(
			2,
			account,
			Operation::SaveDraft {
				channel,
				content: "still usable".into()
			}
		));
	}
}
