//! Explicit, cancellable background refresh. Never installs history into the visible timeline.
use super::{Desktop, Event, cache};
use client_core::{
	auth::{AuthState, Failure},
	force_sync_read::{HISTORY_MILLIS, Read, ReadResult, last_before},
};
use model::Id;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use ui::force_sync::{Action, Outcome, Progress, Scope, Section, Stage, Status};

const MAX_TARGETS: usize = 10_000;
const MAX_CHECKPOINT: usize = 1024 * 1024;
const LAST_STEP: u8 = 11;
const MAX_STALE_RETRIES: u8 = 3;
const MAX_READ_RETRIES: u8 = 2;
const MAX_ACTIVE_PAGES: usize = 400;
static NEXT_JOB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn next_job_id(now: u64) -> Option<u64> {
	use std::sync::atomic::Ordering::Relaxed;
	let previous = NEXT_JOB
		.fetch_update(Relaxed, Relaxed, |old| old.max(now).checked_add(1))
		.ok()?;
	previous.max(now).checked_add(1)
}
fn combine_outcomes(existing: u8, incoming: u8) -> u8 {
	match (existing, incoming) {
		(0, value) | (value, 0) => value,
		(1, 1) => 1,
		(3, 3) => 3,
		(3 | 4, 3 | 4) => 4,
		_ => 2,
	}
}

#[derive(Clone, Serialize, Deserialize)]
struct Target {
	id: Id,
	kind: u8,
	#[serde(default)]
	parent: Option<Id>,
}
impl Target {
	fn has_history(&self) -> bool {
		matches!(self.kind, 0..=3 | 5 | 10..=12)
	}
	fn readable(&self, state: &client_core::State, guild: Option<Id>) -> bool {
		if state.channel(self.id).is_some() {
			return state.can_read_history(self.id);
		}
		matches!(self.kind, 10..=12)
			&& self.parent.is_some_and(|parent| {
				state
					.channel(parent)
					.is_some_and(|channel| channel.guild == guild)
					&& state.can_read_history(parent)
			})
	}
}

#[derive(Clone, Serialize, Deserialize)]
struct Job {
	version: u8,
	id: u64,
	account: Id,
	generation: u64,
	scope: Option<Id>,
	end_ms: i64,
	cutoff_ms: i64,
	guilds: Vec<Id>,
	guild_index: usize,
	dms: Vec<Id>,
	targets: Vec<Target>,
	channel_index: usize,
	step: u8,
	archive_index: usize,
	archive_kind: u8,
	archive_time: Option<i128>,
	archive_id: Option<Id>,
	#[serde(default)]
	active_index: usize,
	#[serde(default)]
	active_before: Option<Id>,
	#[serde(default)]
	active_pages: usize,
	members: Vec<Id>,
	member_index: usize,
	before: Option<Id>,
	request: u64,
	channels_done: u64,
	channels_total: u64,
	messages_saved: u64,
	outcomes: [u8; 7],
	#[serde(default)]
	details: [u8; 7],
	status: u8,
	roster_started: bool,
	#[serde(default)]
	roster_complete: bool,
}
impl Job {
	fn new(state: &client_core::State, scope: Scope) -> Option<Self> {
		let account = state.user.as_ref()?.id;
		let now = i64::try_from(
			SystemTime::now()
				.duration_since(UNIX_EPOCH)
				.ok()?
				.as_millis(),
		)
		.ok()?;
		let guilds = match scope {
			Scope::Server(guild) => {
				state.guilds.iter().find(|g| g.id == guild)?;
				vec![guild]
			}
			Scope::Account => state.guilds.iter().map(|g| g.id).take(4096).collect(),
		};
		let dms = if scope == Scope::Account {
			state
				.channels
				.iter()
				.filter(|c| c.guild.is_none() && matches!(c.kind, 1 | 3) && state.can_view(c.id))
				.map(|c| c.id)
				.take(MAX_TARGETS)
				.collect()
		} else {
			Vec::new()
		};
		let mut job = Self {
			version: 2,
			id: next_job_id(now as u64)?,
			account,
			generation: state.generation,
			scope: match scope {
				Scope::Server(id) => Some(id),
				Scope::Account => None,
			},
			end_ms: now,
			cutoff_ms: now.saturating_sub(HISTORY_MILLIS),
			guilds,
			guild_index: 0,
			dms,
			targets: Vec::new(),
			channel_index: 0,
			step: 0,
			archive_index: 0,
			archive_kind: 0,
			archive_time: None,
			archive_id: None,
			active_index: 0,
			active_before: None,
			active_pages: 0,
			members: Vec::new(),
			member_index: 0,
			before: None,
			request: 0,
			channels_done: 0,
			channels_total: 0,
			messages_saved: 0,
			outcomes: [0; 7],
			details: [0; 7],
			status: 0,
			roster_started: false,
			roster_complete: false,
		};
		if job.guilds.is_empty() {
			job.begin_dms();
		}
		Some(job)
	}
	fn scope(&self) -> Scope {
		self.scope.map_or(Scope::Account, Scope::Server)
	}
	fn guild(&self) -> Option<Id> {
		self.guilds.get(self.guild_index).copied()
	}
	fn stage(&self) -> Stage {
		match self.step {
			0 => Stage::Server,
			1 | 2 | 8 => Stage::Channels,
			3..=5 => Stage::Expressions,
			6 => Stage::Preferences,
			7 | 10 => Stage::Members,
			_ => Stage::History,
		}
	}
	fn stage_index(stage: Stage) -> usize {
		Stage::ALL.iter().position(|s| *s == stage).unwrap_or(0)
	}
	fn record(&mut self, stage: Stage, outcome: Outcome) {
		let value = match outcome {
			Outcome::Updated => 1,
			Outcome::Partial => 2,
			Outcome::Unavailable => 3,
			Outcome::Failed => 4,
		};
		let slot = &mut self.outcomes[Self::stage_index(stage)];
		*slot = combine_outcomes(*slot, value);
	}
	fn migrate_checkpoint(&mut self) {
		if self.version == 1 {
			// The older format has no per-parent thread cursor. Revalidation will
			// combine its outcome with current results before resuming history.
			self.version = 2;
		}
	}

	fn record_failure(&mut self, stage: Stage, failure: Failure) {
		let (outcome, detail) = match failure {
			Failure::Forbidden => (Outcome::Unavailable, 1),
			Failure::Network
			| Failure::Expired
			| Failure::InvalidCredential
			| Failure::Challenged => (Outcome::Failed, 2),
			Failure::RateLimited => (Outcome::Partial, 3),
			Failure::Busy | Failure::ResponseLimit | Failure::Capacity | Failure::CapacityAt(_) => {
				(Outcome::Partial, 4)
			}
			_ => (Outcome::Failed, 5),
		};
		self.record(stage, outcome);
		self.details[Self::stage_index(stage)] = detail;
	}
	fn record_storage_failure(&mut self, stage: Stage, failure: local_store::StoreError) {
		self.record(stage, Outcome::Partial);
		self.details[Self::stage_index(stage)] = match failure {
			local_store::StoreError::Capacity => 4,
			local_store::StoreError::Unavailable => 6,
			local_store::StoreError::Incompatible => 5,
		};
	}

	fn valid(&self) -> bool {
		let distinct = |ids: &[Id]| {
			ids.iter().all(|id| id.0 != 0)
				&& ids.iter().collect::<std::collections::BTreeSet<_>>().len() == ids.len()
		};
		self.version == 2
			&& self.account.0 != 0
			&& self.id != 0
			&& self.guilds.len() <= 4096
			&& self.dms.len() <= MAX_TARGETS
			&& self.targets.len() <= MAX_TARGETS
			&& self.members.len() <= MAX_TARGETS
			&& self.guild_index <= self.guilds.len()
			&& self.channel_index <= self.targets.len()
			&& self.member_index <= self.members.len()
			&& self.archive_index <= self.targets.len()
			&& self.active_index <= self.targets.len()
			&& self.active_pages <= MAX_ACTIVE_PAGES
			&& self.active_before.is_none_or(|id| id.0 != 0)
			&& self.step <= LAST_STEP
			&& self.status <= 4
			&& self.archive_kind <= 1
			&& self.outcomes.iter().all(|v| *v <= 4)
			&& self.details.iter().all(|v| *v <= 7)
			&& self.channels_done <= self.channels_total
			&& self.end_ms.checked_sub(self.cutoff_ms) == Some(HISTORY_MILLIS)
			&& self.cutoff_ms >= client_core::force_sync_read::DISCORD_EPOCH_MS
			&& last_before(self.end_ms).is_some()
			&& self.before.is_none_or(|id| {
				id.0 != 0 && last_before(self.end_ms).is_some_and(|upper| id <= upper)
			}) && self.scope.is_none_or(|guild| {
			guild.0 != 0 && self.guilds.as_slice() == [guild] && self.dms.is_empty()
		}) && distinct(&self.guilds)
			&& distinct(&self.dms)
			&& distinct(&self.members)
			&& self.targets.iter().all(|target| {
				target.id.0 != 0
					&& matches!(target.kind,0..=5|10..=16)
					&& target
						.parent
						.is_none_or(|parent| parent.0 != 0 && parent != target.id)
			}) && self
			.targets
			.iter()
			.map(|t| t.id)
			.collect::<std::collections::BTreeSet<_>>()
			.len() == self.targets.len()
			&& self.archive_time.is_none_or(|time| {
				self.archive_kind == 0 && time >= 0 && time <= i128::from(self.end_ms) * 1_000_000
			}) && self
			.archive_id
			.is_none_or(|id| self.archive_kind == 1 && id.0 != 0)
	}
	fn skip_read(&mut self, read: &Read) {
		match read {
			Read::History { .. } => self.next_channel(),
			Read::Archives { .. } => self.advance_archive(),
			Read::ActiveThreads { .. } => self.advance_active(),
			Read::Members { .. } => {
				self.member_index = (self.member_index + 100).min(self.members.len())
			}
			_ => self.advance(),
		}
	}
	fn advance_active(&mut self) {
		self.active_index = self.active_index.saturating_add(1);
		self.active_before = None;
		self.active_pages = 0;
	}
	fn advance_archive(&mut self) {
		self.archive_time = None;
		self.archive_id = None;
		if self.archive_kind == 0 {
			self.archive_kind = 1;
		} else {
			self.archive_kind = 0;
			self.archive_index += 1;
		}
	}
	fn next_active_read(&mut self, state: &client_core::State, guild: Id) -> Option<Read> {
		while let Some(target) = self.targets.get(self.active_index) {
			let (parent, kind) = (target.id, target.kind);
			if matches!(kind, 0 | 5 | 15 | 16) {
				if state
					.channel(parent)
					.is_some_and(|channel| channel.guild == Some(guild))
					&& state.can_archive(parent, model::archives::Kind::Public)
				{
					return Some(Read::ActiveThreads {
						guild,
						parent,
						before: self.active_before,
					});
				}
				self.record_failure(Stage::Channels, Failure::Forbidden);
			}
			self.advance_active();
		}
		None
	}
	fn next_archive_read(&mut self, state: &client_core::State, guild: Id) -> Option<Read> {
		while let Some(target) = self.targets.get(self.archive_index) {
			let (parent, channel_kind) = (target.id, target.kind);
			if !matches!(channel_kind, 0 | 5 | 15 | 16) {
				self.archive_kind = 0;
				self.archive_index += 1;
				continue;
			}
			let kind = if self.archive_kind == 0 {
				model::archives::Kind::Public
			} else {
				model::archives::Kind::JoinedPrivate
			};
			if kind == model::archives::Kind::JoinedPrivate && channel_kind != 0 {
				self.advance_archive();
				continue;
			}
			if state
				.channel(parent)
				.is_some_and(|channel| channel.guild == Some(guild))
				&& state.can_archive(parent, kind)
			{
				return Some(Read::Archives {
					guild,
					parent,
					kind,
					before: if self.archive_kind == 0 {
						self.archive_time.map(model::archives::Cursor::Time)
					} else {
						self.archive_id.map(model::archives::Cursor::Id)
					},
				});
			}
			self.record_failure(Stage::Channels, Failure::Forbidden);
			self.advance_archive();
		}
		None
	}

	fn add_targets(&mut self, rows: impl IntoIterator<Item = Target>) -> bool {
		let mut seen: std::collections::BTreeSet<_> =
			self.targets.iter().map(|target| target.id).collect();
		for row in rows {
			if !seen.insert(row.id) {
				continue;
			}
			if self.targets.len() == MAX_TARGETS {
				self.record(Stage::History, Outcome::Partial);
				return false;
			}
			self.targets.push(row);
		}
		true
	}
	fn discover(&mut self, state: &client_core::State) {
		let guild = self.guild();
		if !self.add_targets(
			state
				.channels
				.iter()
				.filter(|c| c.guild == guild && state.can_view(c.id))
				.map(|c| Target {
					id: c.id,
					kind: c.kind,
					parent: c.parent_id,
				}),
		) {
			self.record_failure(Stage::Channels, Failure::Capacity);
		}
	}
	fn add_members(&mut self, rows: impl IntoIterator<Item = Id>) {
		let mut seen: std::collections::BTreeSet<_> = self.members.iter().copied().collect();
		for id in rows {
			if id.0 != 0 && seen.insert(id) {
				if self.members.len() == MAX_TARGETS {
					self.record(Stage::Members, Outcome::Partial);
					break;
				}
				self.members.push(id);
			}
		}
	}
	fn add_known_members(&mut self, state: &client_core::State, guild: Id) {
		self.add_members(state.force_sync_known_member_ids(guild));
	}
	fn begin_dms(&mut self) {
		self.targets = self
			.dms
			.iter()
			.map(|id| Target {
				id: *id,
				kind: 1,
				parent: None,
			})
			.collect();
		self.channels_total += self.targets.len() as u64;
		self.channel_index = 0;
		self.before = last_before(self.end_ms);
		self.step = 9;
	}
	fn next_guild(&mut self) {
		self.guild_index += 1;
		self.targets.clear();
		self.members.clear();
		self.member_index = 0;
		self.channel_index = 0;
		self.archive_index = 0;
		self.archive_kind = 0;
		self.archive_time = None;
		self.archive_id = None;
		self.active_index = 0;
		self.active_before = None;
		self.active_pages = 0;
		self.before = None;
		self.roster_started = false;
		self.roster_complete = false;
		self.step = 0;
		if self.guild_index >= self.guilds.len() {
			self.begin_dms();
		}
	}
	fn history_read(&self) -> Option<Read> {
		if self.step != 9 {
			return None;
		}
		Some(Read::History {
			channel: self.targets.get(self.channel_index)?.id,
			before: self.before,
			cutoff_ms: self.cutoff_ms,
			end_ms: self.end_ms,
		})
	}
	fn next_channel(&mut self) {
		self.channels_done += 1;
		self.channel_index += 1;
		self.before = last_before(self.end_ms);
	}
	fn skip_unreadable_targets(&mut self, state: &client_core::State) {
		while let Some(target) = self.targets.get(self.channel_index) {
			if !target.has_history() {
				self.channel_index += 1;
			} else if !target.readable(state, self.guild()) {
				self.record_failure(Stage::History, Failure::Forbidden);
				self.next_channel();
			} else {
				break;
			}
		}
	}
	fn advance(&mut self) {
		self.step += 1;
		if self.step == 9 {
			self.channels_total += self.targets.iter().filter(|t| t.has_history()).count() as u64;
			self.before = last_before(self.end_ms);
		}
	}
	fn progress(&self, cache_bytes: u64) -> Progress {
		Progress {
			scope: self.scope(),
			status: match self.status {
				1 => Status::Paused,
				2 => Status::Waiting,
				3 => Status::Complete,
				4 => Status::Cancelled,
				_ => Status::Running,
			},
			stage: self.stage(),
			channels_done: self.channels_done,
			channels_total: self.channels_total,
			messages_saved: self.messages_saved,
			cache_bytes,
			sections: Stage::ALL
				.iter()
				.enumerate()
				.filter_map(|(i, stage)| {
					let outcome = match self.outcomes[i] {
						1 => Outcome::Updated,
						2 => Outcome::Partial,
						3 => Outcome::Unavailable,
						4 => Outcome::Failed,
						_ => return None,
					};
					Some(Section {
						stage: *stage,
						outcome,
						detail: match self.details[i] {
							1 => Some("force-sync-error-permission"),
							2 => Some("force-sync-error-network"),
							3 => Some("force-sync-error-rate-limit"),
							4 => Some("force-sync-error-capacity"),
							5 => Some("force-sync-error-unsupported"),
							6 => Some("force-sync-error-storage"),
							7 => Some("force-sync-error-stale"),
							_ if *stage == Stage::Members && self.outcomes[i] == 2 => {
								Some("force-sync-members-partial")
							}
							_ => None,
						},
					})
				})
				.collect(),
			detail: if self.status == 2 {
				Some("force-sync-waiting-connection")
			} else {
				None
			},
			can_retry: self.status >= 3 && self.outcomes.iter().any(|x| *x >= 2),
		}
	}
}

struct Active {
	request: u64,
	revision: u64,
	read: Read,
	task: tokio::task::JoinHandle<Result<ReadResult, Failure>>,
}
impl Drop for Active {
	fn drop(&mut self) {
		self.task.abort();
	}
}
struct PageCommit {
	channel: Id,
	request: u64,
	next: Option<Id>,
	done: bool,
}
struct RosterChunk {
	guild: Id,
	members: Vec<model::Member>,
	nick_unknown: Vec<Id>,
	global_names: Vec<(Id, model::GlobalNameUpdate)>,
	index: usize,
}

fn complete_member_display(
	user: Id,
	nick_unknown: &[Id],
	global_names: &[(Id, model::GlobalNameUpdate)],
) -> bool {
	nick_unknown.binary_search(&user).is_err()
		&& client_core::force_sync_read::member_name_update(global_names, user)
			!= model::GlobalNameUpdate::Unchanged
}

fn member_display_rows<'a>(
	guild: Id,
	members: impl Iterator<Item = &'a model::Member>,
	nick_unknown: &[Id],
	global_names: &[(Id, model::GlobalNameUpdate)],
) -> Vec<model::MemberDisplay> {
	members
		.filter(|member| complete_member_display(member.user.id, nick_unknown, global_names))
		.map(|member| model::MemberDisplay {
			guild,
			user: member.user.id,
			name: member.user.name.clone(),
			username: member.user.username.clone(),
			nick: member.nick.clone(),
		})
		.collect()
}

#[derive(Default)]
pub(super) struct Sync {
	job: Option<Job>,
	active: Option<Active>,
	pending_page: Option<PageCommit>,
	revision_requested: bool,
	revision: Option<u64>,
	checkpoint_loaded: Option<(u64, Id)>,
	clear_checkpoint: bool,
	metadata_revision: u64,
	history_revision: u64,
	preference: Option<u64>,
	preference_due: Option<Instant>,
	retry_read: Option<Read>,
	stale_retries: u8,
	read_retries: u8,
	next_read: Option<Instant>,
	dirty: bool,
	usage: u64,
	usage_due: Option<Instant>,
	restore: Option<local_store::SyncCheckpoint>,
	resume: Option<Box<Job>>,
	needs_revalidate: bool,
	roster_chunk: Option<RosterChunk>,
	roster_changed: std::collections::BTreeSet<Id>,
	roster_write: Option<(u64, Id)>,
	roster_finished: bool,
	roster_users: Option<Vec<Id>>,
	roster_reconcile: Option<u64>,
	roster_attempt: Option<u64>,
	roster_sequence: u64,
}

impl Sync {
	fn history_change(&self, event: &Event) -> Option<(Id, Id)> {
		self.job
			.as_ref()?
			.history_read()
			.and_then(|read| history_event_change(&read, event))
	}
	fn prepare_read(&mut self, read: &Read) {
		if self.retry_read.as_ref() != Some(read) {
			self.retry_read = Some(read.clone());
			self.stale_retries = 0;
			self.read_retries = 0;
			self.next_read = None;
		}
	}
	fn reject_stale(&mut self, job: &mut Job, read: &Read) {
		self.revision = None;
		self.revision_requested = false;
		self.stale_retries = self.stale_retries.saturating_add(1);
		if self.stale_retries >= MAX_STALE_RETRIES {
			job.record(job.stage(), Outcome::Partial);
			job.details[Job::stage_index(job.stage())] = 7;
			job.skip_read(read);
			self.retry_read = None;
		}
		self.dirty = true;
	}
	fn defer_read(&mut self, failure: Failure, now: Instant) -> bool {
		if !matches!(failure, Failure::Busy | Failure::RateLimited)
			|| self.read_retries >= MAX_READ_RETRIES
		{
			return false;
		}
		self.read_retries += 1;
		let base = if failure == Failure::RateLimited {
			60
		} else {
			1
		};
		self.next_read = Some(now + Duration::from_secs(base * u64::from(self.read_retries)));
		true
	}
	fn defer_indexing(&mut self, delay: Duration, now: Instant) -> bool {
		if delay > Duration::from_secs(30) || self.read_retries >= MAX_READ_RETRIES {
			return false;
		}
		self.read_retries += 1;
		self.next_read = Some(now + delay.max(Duration::from_millis(50)));
		true
	}
	fn should_measure_usage(&self, visible: bool) -> bool {
		visible || self.job.as_ref().is_some_and(|job| job.status == 0)
	}
}

impl Desktop {
	pub(super) fn poll_force_sync(&mut self, ctx: &eframe::egui::Context) {
		let Some(account) = self.state.user.as_ref().map(|u| u.id) else {
			if let Some(cache) = &self.cache {
				cache.protect_history(Id(0), None);
				cache.protect_guild(Id(0), None);
				cache.protect_sync_history(Id(0), &[]);
			}
			self.cancel_force_sync_roster();
			self.force_sync = Sync::default();
			self.messaging.force_sync_progress = None;
			return;
		};
		if let Some(cache) = &self.cache {
			let syncing = self
				.force_sync
				.job
				.as_ref()
				.filter(|job| job.status == 0 && job.step == 9)
				.and_then(|job| job.targets.get(job.channel_index))
				.map(|target| target.id);
			cache.protect_sync_history(account, &syncing.into_iter().collect::<Vec<_>>());
			cache.protect_history(account, self.state.selected);
			cache.protect_guild(
				account,
				self.state
					.selected
					.and_then(|id| self.state.channel(id))
					.and_then(|channel| channel.guild),
			);
		}
		if self
			.force_sync
			.job
			.as_ref()
			.is_some_and(|j| j.account != account || j.generation != self.state.generation)
		{
			self.cancel_force_sync_roster();
			self.force_sync = Sync::default();
			self.messaging.force_sync_progress = None;
		}
		if self.state.auth != AuthState::Authenticated {
			return;
		}
		let key = (self.state.generation, account);
		if self.force_sync.checkpoint_loaded != Some(key)
			&& self.queue_cache(cache::Operation::LoadSyncCheckpoint)
		{
			self.force_sync.checkpoint_loaded = Some(key);
		}
		if let Some(checkpoint) = self.force_sync.restore.take()
			&& self.force_sync.job.is_none()
			&& checkpoint.payload.len() <= MAX_CHECKPOINT
			&& let Ok(mut job) = serde_json::from_slice::<Job>(&checkpoint.payload)
			&& {
				job.migrate_checkpoint();
				job.valid()
			} && job.account == account
			&& checkpoint.job == job.id
			&& checkpoint.scope == job.scope
			&& checkpoint.started_ms == job.end_ms
			&& checkpoint.cutoff_ms == job.cutoff_ms
			&& job.status < 3
		{
			job.generation = self.state.generation;
			job.status = 1;
			job.roster_started = false;
			self.force_sync.needs_revalidate = true;
			self.force_sync.job = Some(job);
		}
		if let Some(action) = self.messaging.force_sync_action.take() {
			self.force_sync_action(action);
		}
		if self.force_sync.clear_checkpoint
			&& self.queue_cache(cache::Operation::ClearSyncCheckpoint)
		{
			self.force_sync.clear_checkpoint = false;
		}
		if self
			.force_sync
			.should_measure_usage(self.messaging.force_sync_usage_visible())
			&& self
				.force_sync
				.usage_due
				.is_none_or(|at| Instant::now() >= at)
			&& self.queue_cache(cache::Operation::CacheUsage)
		{
			self.force_sync.usage_due = Some(Instant::now() + Duration::from_secs(5));
		}
		if self.force_sync.job.is_none() {
			return;
		}
		let mut job = self.force_sync.job.take().expect("job checked");
		if !self.state.gateway_connected && job.status == 0 {
			job.status = 2;
			job.roster_started = false;
			self.cancel_force_sync_roster();
			self.force_sync.active = None;
			self.force_sync.revision = None;
			self.force_sync.revision_requested = false;
			self.force_sync.dirty = true;
		}
		if self.state.gateway_connected && job.status == 2 {
			job.status = 0;
			self.force_sync.dirty = true;
		}
		if job.status == 0 {
			if let Some(active) = self.force_sync.active.take() {
				if active.task.is_finished() {
					let mut active = active;
					let finished = self
						.runtime
						.block_on(&mut active.task)
						.unwrap_or(Err(Failure::Network));
					let revision = if matches!(active.read, Read::History { .. }) {
						self.force_sync.history_revision
					} else {
						self.force_sync.metadata_revision
					};
					if active.request != job.request || active.revision != revision {
						self.force_sync.reject_stale(&mut job, &active.read);
					} else {
						self.force_sync_read_result(&mut job, active.read.clone(), finished);
					}
				} else {
					self.force_sync.active = Some(active);
				}
			}
			if self.force_sync.active.is_none() && self.force_sync.pending_page.is_none() {
				self.drive_force_sync(&mut job, ctx);
			}
		}
		if self.force_sync.dirty {
			let checkpoint_job = self.force_sync.resume.as_deref().unwrap_or(&job);
			if let Ok(payload) = serde_json::to_vec(checkpoint_job)
				&& payload.len() <= MAX_CHECKPOINT
			{
				let checkpoint = local_store::SyncCheckpoint {
					job: checkpoint_job.id,
					scope: checkpoint_job.scope,
					started_ms: checkpoint_job.end_ms,
					cutoff_ms: checkpoint_job.cutoff_ms,
					payload,
				};
				if self.queue_cache(cache::Operation::SaveSyncCheckpoint(checkpoint)) {
					self.force_sync.dirty = false;
				}
			}
		}
		let mut progress = job.progress(self.force_sync.usage);
		if job.status == 0
			&& self
				.force_sync
				.next_read
				.is_some_and(|at| at > Instant::now())
		{
			progress.status = Status::Waiting;
			progress.detail = Some("force-sync-waiting-rate-limit");
		}
		self.messaging.force_sync_progress = Some(progress);
		if job.status == 0 && self.force_sync.roster_chunk.is_some() {
			ctx.request_repaint_after(Duration::from_millis(16));
		}
		if job.status == 0 {
			ctx.request_repaint_after(
				self.force_sync
					.next_read
					.map_or(Duration::from_millis(250), |at| {
						at.saturating_duration_since(Instant::now())
					}),
			);
		}
		self.force_sync.job = Some(job);
	}

	fn force_sync_action(&mut self, action: Action) {
		match action {
			Action::Start(scope) => {
				if self.force_sync.job.as_ref().is_some_and(|j| j.status < 3) {
					return;
				}
				if self.state.demo {
					self.messaging.force_sync_progress =
						Job::new(&self.state, scope).map(|j| j.progress(0));
					return;
				}
				if let Some(job) = Job::new(&self.state, scope) {
					// Explicit sync retries presentation writes after storage pressure clears.
					self.server_cache_failed = false;
					self.cancel_force_sync_roster();
					self.force_sync.active = None;
					self.force_sync.pending_page = None;
					self.force_sync.revision = None;
					self.force_sync.revision_requested = false;
					self.force_sync.retry_read = None;
					self.force_sync.next_read = None;
					self.force_sync.preference_due = None;
					self.force_sync.job = Some(job);
					self.force_sync.dirty = true;
				}
			}
			Action::Pause => {
				if let Some(j) = &mut self.force_sync.job {
					j.status = 1;
				}
				self.force_sync.active = None;
				self.force_sync.revision = None;
				self.force_sync.revision_requested = false;
				self.force_sync.dirty = true;
				self.cancel_force_sync_roster();
			}
			Action::Resume => {
				if let Some(mut job) = self.force_sync.job.take() {
					if job.status < 3 {
						self.server_cache_failed = false;
						job.status = 0;
						if self.force_sync.needs_revalidate {
							self.force_sync.resume = Some(Box::new(job.clone()));
							self.force_sync.needs_revalidate = false;
							if job.guild().is_some() {
								job.step = 0;
								job.targets.clear();
								job.channel_index = 0;
								job.active_index = 0;
								job.active_before = None;
								job.active_pages = 0;
							} else {
								self.restore_force_sync_continuation(&mut job);
							}
						}
						self.force_sync.dirty = true;
					}
					self.force_sync.job = Some(job);
				}
			}
			Action::Cancel => {
				if let Some(j) = &mut self.force_sync.job {
					j.status = 4;
				}
				self.force_sync.active = None;
				self.force_sync.pending_page = None;
				self.force_sync.preference = None;
				self.force_sync.preference_due = None;
				self.force_sync.next_read = None;
				self.force_sync.dirty = false;
				self.force_sync.clear_checkpoint = true;
				self.cancel_force_sync_roster();
			}
			Action::Retry => {
				if let Some(j) = &self.force_sync.job {
					let scope = j.scope();
					self.force_sync.job = None;
					self.force_sync_action(Action::Start(scope));
				}
			}
		}
	}

	fn drive_force_sync(&mut self, job: &mut Job, ctx: &eframe::egui::Context) {
		if let Some(due) = self.force_sync.next_read
			&& due > Instant::now()
		{
			ctx.request_repaint_after(due.saturating_duration_since(Instant::now()));
			return;
		}
		if let Some(guild) = job.guild()
			&& self.state.guilds.iter().all(|g| g.id != guild)
		{
			job.record(job.stage(), Outcome::Unavailable);
			self.force_sync.resume = None;
			job.next_guild();
			self.force_sync.dirty = true;
			return;
		}
		let guild = job.guild();
		let read = match job.step {
			0 => guild.map(|guild| Read::Guild {
				guild,
				user: job.account,
			}),
			1 => guild.map(|guild| Read::Channels {
				guild,
				user: job.account,
			}),
			2 => {
				if job.active_index == 0 && job.targets.is_empty() {
					// A failed or repeatedly stale channel refresh must not discard the
					// currently authorized Gateway navigation snapshot.
					job.discover(&self.state);
				}
				let read = guild.and_then(|guild| job.next_active_read(&self.state, guild));
				if read.is_none() {
					job.active_index = 0;
					job.active_before = None;
					job.active_pages = 0;
					job.advance();
					self.restore_force_sync_continuation(job);
					self.force_sync.dirty = true;
					return;
				}
				read
			}
			3 => guild.map(|guild| Read::Emojis { guild }),
			4 => guild.map(|guild| Read::Stickers { guild }),
			5 => guild.map(|guild| Read::Sounds { guild }),
			6 => {
				let due = *self
					.force_sync
					.preference_due
					.get_or_insert_with(|| Instant::now() + Duration::from_secs(45));
				if Instant::now() >= due {
					job.record_failure(Stage::Preferences, Failure::Network);
					job.advance();
					self.force_sync.preference = None;
					self.force_sync.preference_due = None;
					self.force_sync.dirty = true;
					return;
				}
				if self.force_sync.preference.is_none()
					&& let Some(guild) = guild
					&& let Some(command) = self.state.refresh_channel_preferences(guild)
				{
					if let client_core::Command::ChannelPreferences(request) = &command {
						self.force_sync.preference = Some(request.request);
					}
					self.command(command);
				}
				return;
			}
			7 => {
				self.drive_force_sync_roster(job);
				return;
			}
			8 => {
				let read = guild.and_then(|guild| job.next_archive_read(&self.state, guild));
				if read.is_none() {
					job.advance();
					self.force_sync.dirty = true;
					return;
				}
				read
			}
			9 => {
				job.skip_unreadable_targets(&self.state);
				let Some(target) = job.targets.get(job.channel_index) else {
					if guild.is_some() {
						job.advance();
					} else {
						job.status = 3;
						self.force_sync.clear_checkpoint = true;
					}
					self.force_sync.dirty = job.status != 3;
					return;
				};
				if self.force_sync.revision.is_none() {
					if !self.force_sync.revision_requested
						&& self.queue_cache(cache::Operation::SyncReadRevision {
							job: job.id,
							request: job.request,
							channel: target.id,
						}) {
						self.force_sync.revision_requested = true;
					}
					return;
				}
				job.history_read()
			}
			10 => {
				if self.force_sync.roster_write.is_some() {
					return;
				}
				if job.roster_complete {
					job.advance();
					return;
				}
				let end = (job.member_index + 100).min(job.members.len());
				if end == job.member_index {
					job.advance();
					return;
				}
				let mut users = job.members[job.member_index..end].to_vec();
				users.sort_unstable();
				users.dedup();
				guild.map(|guild| Read::Members { guild, users })
			}
			_ => {
				job.next_guild();
				self.force_sync.dirty = true;
				return;
			}
		};
		let Some(read) = read else {
			return;
		};
		self.force_sync.prepare_read(&read);
		let api = self
			.connection
			.as_ref()
			.and_then(|c| c.sync_api.borrow().clone());
		let Some(api) = api else {
			return;
		};
		job.request = job.request.wrapping_add(1);
		let request = job.request;
		let revision = if matches!(read, Read::History { .. }) {
			self.force_sync.history_revision
		} else {
			self.force_sync.metadata_revision
		};
		let worker_read = read.clone();
		self.force_sync.next_read = None;
		let wake = ctx.clone();
		let task = self.runtime.spawn(async move {
			let result =
				tokio::time::timeout(Duration::from_secs(45), api.force_sync_read(worker_read))
					.await
					.unwrap_or(Err(Failure::Network));
			wake.request_repaint();
			result
		});
		self.force_sync.active = Some(Active {
			request,
			revision,
			read,
			task,
		});
	}

	fn force_sync_read_result(
		&mut self,
		job: &mut Job,
		read: Read,
		result: Result<ReadResult, Failure>,
	) {
		self.force_sync.dirty = true;
		let stage = job.stage();
		let result = match result {
			Ok(ReadResult::ActiveThreadsIndexing {
				guild,
				parent,
				retry_after,
			}) => {
				if !active_read_is_current(job, &self.state, &read, guild, parent) {
					job.record_failure(Stage::Channels, Failure::Protocol);
					job.skip_read(&read);
				} else if !self.force_sync.defer_indexing(retry_after, Instant::now()) {
					job.record_failure(Stage::Channels, Failure::RateLimited);
					job.advance_active();
					self.force_sync.retry_read = None;
					self.force_sync.next_read = None;
				}
				return;
			}
			Ok(result) => {
				self.force_sync.read_retries = 0;
				self.force_sync.next_read = None;
				if !matches!(read, Read::History { .. }) {
					self.force_sync.stale_retries = 0;
				}
				result
			}
			Err(failure) => {
				if failure.ends_session() {
					if let Some(connection) = &self.connection {
						if let Some(api) = connection.sync_api.borrow().as_ref() {
							api.stop();
						}
						connection.sync_failure.send_replace(Some(failure));
					}
					job.record_failure(stage, failure);
					job.status = 1;
					self.force_sync.next_read = None;
					self.force_sync.pending_page = None;
					self.cancel_force_sync_roster();
					return;
				}
				if self.force_sync.defer_read(failure, Instant::now()) {
					return;
				}
				if matches!(read, Read::Channels { .. }) {
					job.discover(&self.state);
					if !job.targets.is_empty() {
						job.record(stage, Outcome::Updated);
					}
				}
				job.record_failure(stage, failure);
				job.skip_read(&read);
				self.force_sync.revision = None;
				self.force_sync.revision_requested = false;
				self.force_sync.retry_read = None;
				self.force_sync.next_read = None;
				return;
			}
		};
		if result.bytes() > 8 * 1024 * 1024 {
			job.record_failure(stage, Failure::ResponseLimit);
			job.skip_read(&read);
			return;
		}
		match result {
			ReadResult::ActiveThreads {
				guild,
				parent,
				threads,
				next,
			} => {
				if !active_read_is_current(job, &self.state, &read, guild, parent)
					|| next.is_some_and(|id| {
						id.0 == 0 || job.active_before.is_some_and(|before| id >= before)
					}) {
					job.record_failure(Stage::Channels, Failure::Protocol);
					job.skip_read(&read);
					return;
				}
				let targets: Vec<_> = threads
					.iter()
					.map(|thread| Target {
						id: thread.id,
						kind: thread.kind,
						parent: thread.parent_id,
					})
					.collect();
				if !self
					.state
					.apply_force_sync_result(ReadResult::ActiveThreads {
						guild,
						parent,
						threads,
						next,
					}) {
					job.record_failure(Stage::Channels, Failure::Protocol);
					job.advance_active();
					return;
				}
				job.record(Stage::Channels, Outcome::Updated);
				if !job.add_targets(targets) {
					job.record_failure(Stage::Channels, Failure::Capacity);
				}
				job.active_pages += 1;
				if let Some(next) = next {
					if job.active_pages < MAX_ACTIVE_PAGES {
						job.active_before = Some(next);
					} else {
						job.record_failure(Stage::Channels, Failure::Capacity);
						job.advance_active();
					}
				} else {
					job.advance_active();
				}
			}
			ReadResult::ActiveThreadsIndexing { .. } => {
				job.record_failure(Stage::Channels, Failure::Protocol);
				job.skip_read(&read);
			}
			ReadResult::History(page) => {
				if page.channel != job.targets[job.channel_index].id
					|| !job.targets[job.channel_index].readable(&self.state, job.guild())
				{
					job.record(Stage::History, Outcome::Unavailable);
					job.next_channel();
					return;
				}
				job.add_members(page.messages.iter().map(|m| m.author.id));
				let before = match read {
					Read::History { before, .. } => before,
					_ => None,
				};
				if page
					.oldest
					.is_some_and(|oldest| before.is_some_and(|bound| oldest >= bound))
				{
					job.record_failure(Stage::History, Failure::Protocol);
					job.next_channel();
					self.force_sync.revision = None;
					self.force_sync.revision_requested = false;
					return;
				}
				let done = page.boundary_reached || page.oldest.is_none();
				let coverage = local_store::HistoryCoverage {
					job: job.id,
					request: job.request,
					cutoff: Id(
						((job.cutoff_ms - client_core::force_sync_read::DISCORD_EPOCH_MS) as u64)
							<< 22,
					),
					upper: last_before(job.end_ms).unwrap_or(Id(u64::MAX)),
					before,
					next_before: if done { None } else { page.oldest },
					complete: done,
					revision: self.force_sync.revision.unwrap_or(0),
				};
				let commit = PageCommit {
					channel: page.channel,
					request: job.request,
					next: page.oldest,
					done,
				};
				if self.queue_cache(cache::Operation::SyncSavePage {
					job: job.id,
					request: job.request,
					channel: page.channel,
					guild: job.guild(),
					parent: job.targets[job.channel_index].parent,
					messages: page.messages,
					coverage,
				}) {
					self.force_sync.pending_page = Some(commit);
				} else {
					job.record_failure(Stage::History, Failure::Busy);
					job.next_channel();
				}
				self.force_sync.revision = None;
				self.force_sync.revision_requested = false;
			}
			ReadResult::Archives(page) => {
				let Read::Archives {
					guild,
					parent,
					kind,
					before,
				} = read
				else {
					job.record_failure(Stage::Channels, Failure::Protocol);
					return;
				};
				if job.guild() != Some(guild)
					|| job
						.targets
						.get(job.archive_index)
						.is_none_or(|target| target.id != parent)
					|| !page.valid(parent, guild, kind, before)
				{
					job.record_failure(Stage::Channels, Failure::Protocol);
					job.advance_archive();
					return;
				}
				if !self.state.can_archive(parent, kind) {
					job.record_failure(Stage::Channels, Failure::Forbidden);
					job.advance_archive();
					return;
				}
				let next = page.next;
				if !job.add_targets(page.threads.iter().map(|c| Target {
					id: c.id,
					kind: c.kind,
					parent: c.parent_id,
				})) {
					job.record_failure(Stage::Channels, Failure::Capacity);
				}
				job.record(Stage::Channels, Outcome::Updated);

				match next {
					Some(model::archives::Cursor::Time(at))
						if at >= i128::from(job.cutoff_ms) * 1_000_000 =>
					{
						job.archive_time = Some(at)
					}
					Some(model::archives::Cursor::Id(id)) => job.archive_id = Some(id),
					_ => self.advance_sync_archive(job),
				}
			}
			ReadResult::Members {
				guild,
				items,
				missing,
				nick_unknown,
				global_names,
			} => {
				let missing_on_disk = missing.clone();
				let unknown_on_disk = nick_unknown.clone();
				let names_on_disk = global_names.clone();
				let completed_members = items.clone();
				let rows = member_display_rows(guild, items.iter(), &nick_unknown, &global_names);
				if !missing.is_empty()
					|| !nick_unknown.is_empty()
					|| global_names
						.iter()
						.any(|(_, name)| *name == model::GlobalNameUpdate::Unchanged)
				{
					job.record(Stage::Members, Outcome::Partial);
				}
				if !self.state.apply_force_sync_result(ReadResult::Members {
					guild,
					items,
					missing,
					nick_unknown,
					global_names,
				}) {
					job.record(Stage::Members, Outcome::Partial);
					job.skip_read(&read);
					return;
				}
				if self.queue_cache(cache::Operation::SyncSaveMembers {
					job: job.id,
					request: job.request,
					guild,
					rows,
					missing: missing_on_disk,
				}) {
					self.force_sync.roster_write = Some((job.request, guild));
					let count = completed_members.len();
					self.force_sync.roster_chunk = Some(RosterChunk {
						guild,
						members: completed_members,
						nick_unknown: unknown_on_disk,
						global_names: names_on_disk,
						index: count,
					});
				} else {
					job.record_failure(Stage::Members, Failure::Busy);
					job.member_index = (job.member_index + 100).min(job.members.len());
				}
			}
			other => {
				let previously_readable: Vec<_> =
					if matches!(read, Read::Guild { .. } | Read::Channels { .. }) {
						self.state
							.channels
							.iter()
							.filter(|channel| {
								channel.guild == read.guild()
									&& self.state.can_read_history(channel.id)
							})
							.map(|channel| channel.id)
							.collect()
					} else {
						Vec::new()
					};
				let roles = matches!(&other, ReadResult::Guild(_));
				let roles_complete =
					!matches!(&other, ReadResult::Guild(value) if value.self_member.is_err());
				if let ReadResult::Guild(value) = &other
					&& let Err(failure) = value.self_member.as_ref()
				{
					job.record_failure(Stage::Members, *failure);
				}
				if let ReadResult::Guild(value) = &other
					&& value.self_member.is_ok()
					&& (!value.self_member_nick_known
						|| value.self_member_global_name == model::GlobalNameUpdate::Unchanged)
				{
					job.record(Stage::Members, Outcome::Partial);
				}
				let applied = self.state.apply_force_sync_result(other);
				for channel in previously_readable {
					if !self.state.can_read_history(channel)
						&& let Some(cache) = &self.cache
					{
						if cache.invalidate_scope(
							self.state.generation,
							job.account,
							cache::Operation::ClearChannel { channel },
						) {
							self.cache_pending += 1;
						} else {
							cache.history.fail();
							job.record_failure(Stage::History, Failure::Busy);
							job.status = 1;
						}
					}
				}
				let outcome = if applied {
					Outcome::Updated
				} else {
					Outcome::Partial
				};
				if roles {
					job.record(
						Stage::Roles,
						if roles_complete {
							outcome
						} else {
							Outcome::Partial
						},
					);
				}
				if applied && matches!(read, Read::Channels { .. }) {
					job.discover(&self.state);
				}
				job.record(stage, outcome);
				if !applied {
					job.details[Job::stage_index(stage)] = 5;
				}
				job.advance();
			}
		}
	}
	fn advance_sync_archive(&mut self, job: &mut Job) {
		job.advance_archive();
	}

	pub(super) fn force_sync_cache_result(
		&mut self,
		generation: u64,
		outcome: &cache::Outcome,
	) -> bool {
		match outcome {
			cache::Outcome::SyncCheckpoint { epoch, result } => {
				if self.force_sync_epoch_valid(generation, *epoch) {
					self.force_sync.restore = result.as_ref().ok().and_then(Clone::clone);
				}
				true
			}
			cache::Outcome::SyncMembersReconciled {
				job,
				request,
				guild,
				epoch,
				result,
			} => {
				if generation == self.state.generation
					&& self.force_sync.roster_reconcile == Some(*request)
					&& self
						.force_sync
						.job
						.as_ref()
						.is_some_and(|j| j.id == *job && j.guild() == Some(*guild) && j.step == 7)
				{
					self.force_sync.roster_reconcile = None;
					let mut current = self.force_sync.job.take().expect("job checked");
					let failure = match result {
						_ if !self.force_sync_epoch_valid(generation, *epoch) => {
							Some(Failure::Busy)
						}
						Ok(_)
							if self.force_sync.roster_changed.is_empty()
								&& self.force_sync.roster_users.as_ref().is_some_and(|users| {
									self.state.reconcile_force_sync_roster(*guild, users)
								}) =>
						{
							None
						}
						Ok(_) => Some(Failure::Busy),
						Err(error) => {
							current.record_storage_failure(Stage::Members, *error);
							Some(Failure::Busy)
						}
					};
					self.finish_force_sync_roster(&mut current, failure);
					self.force_sync.job = Some(current);
				}
				true
			}
			cache::Outcome::SyncCheckpointSaved { epoch, result } => {
				if result.is_err()
					&& self.force_sync_epoch_valid(generation, *epoch)
					&& let Some(j) = &mut self.force_sync.job
				{
					j.record(Stage::History, Outcome::Partial);
					j.details[Job::stage_index(Stage::History)] = 6;
					if j.status >= 3 {
						self.force_sync.clear_checkpoint = true;
					}
				}
				true
			}
			cache::Outcome::SyncRevision {
				job,
				request,
				channel,
				epoch,
				result,
			} => {
				if generation == self.state.generation
					&& self.force_sync.job.as_ref().is_some_and(|j| {
						j.id == *job
							&& j.request == *request
							&& j.targets
								.get(j.channel_index)
								.is_some_and(|t| t.id == *channel)
					}) {
					match result {
						Ok(revision) if self.force_sync_epoch_valid(generation, *epoch) => {
							self.force_sync.revision = Some(*revision)
						}
						_ => {
							if let Some(j) = &mut self.force_sync.job {
								j.record(Stage::History, Outcome::Partial);
								j.details[Job::stage_index(Stage::History)] = 6;
								j.next_channel();
							}
							self.force_sync.revision_requested = false;
						}
					}
				}
				true
			}
			cache::Outcome::SyncPageSaved {
				job,
				request,
				channel,
				epoch,
				result,
			} => {
				if generation == self.state.generation
					&& self.force_sync.job.as_ref().is_some_and(|j| j.id == *job)
					&& self
						.force_sync
						.pending_page
						.as_ref()
						.is_some_and(|p| p.channel == *channel && p.request == *request)
				{
					let page = self.force_sync.pending_page.take().expect("page checked");
					let current_epoch = self.force_sync_epoch_valid(generation, *epoch);
					let j = self.force_sync.job.as_mut().expect("job checked");
					match result {
						_ if !current_epoch => {
							j.record(Stage::History, Outcome::Partial);
							j.details[Job::stage_index(Stage::History)] = 6;
							j.next_channel();
						}
						Ok(saved) if !saved.stale => {
							self.force_sync.stale_retries = 0;
							j.messages_saved += saved.saved as u64;
							if page.done {
								j.record(Stage::History, Outcome::Updated);
								j.next_channel();
							} else {
								j.before = page.next;
							}
						}
						Ok(_) => {
							self.force_sync.stale_retries =
								self.force_sync.stale_retries.saturating_add(1);
							if self.force_sync.stale_retries >= MAX_STALE_RETRIES {
								j.record(Stage::History, Outcome::Partial);
								j.details[Job::stage_index(Stage::History)] = 7;
								j.next_channel();
								self.force_sync.retry_read = None;
							}
						}
						Err(error) => {
							j.record_storage_failure(Stage::History, *error);
							j.next_channel();
						}
					}
					self.force_sync.dirty = true;
				}
				true
			}
			cache::Outcome::SyncMembersSaved {
				job,
				request,
				guild,
				epoch,
				result,
			} => {
				if generation == self.state.generation
					&& self.force_sync.job.as_ref().is_some_and(|current| {
						current.id == *job
							&& matches!(current.step, 7 | 10)
							&& current.guild() == Some(*guild)
					}) && self.force_sync.roster_write == Some((*request, *guild))
				{
					let current_epoch = self.force_sync_epoch_valid(generation, *epoch);
					self.force_sync.roster_write = None;
					self.force_sync.roster_chunk = None;
					if self
						.force_sync
						.job
						.as_ref()
						.is_some_and(|job| job.step == 10)
					{
						let current = self.force_sync.job.as_mut().expect("job checked");
						if result.is_err() || !current_epoch {
							current.record_storage_failure(
								Stage::Members,
								result
									.as_ref()
									.err()
									.copied()
									.unwrap_or(local_store::StoreError::Capacity),
							);
						}
						current.member_index =
							(current.member_index + 100).min(current.members.len());
						self.force_sync.dirty = true;
						return true;
					}
					if result.is_err() || !current_epoch {
						let mut current = self.force_sync.job.take().expect("job checked");
						self.finish_force_sync_roster(&mut current, Some(Failure::Busy));
						current.record_storage_failure(
							Stage::Members,
							result
								.as_ref()
								.err()
								.copied()
								.unwrap_or(local_store::StoreError::Capacity),
						);
						self.force_sync.job = Some(current);
					}
					self.force_sync.dirty = true;
				}
				true
			}
			cache::Outcome::CacheUsage(result) => {
				if let Ok(usage) = result {
					self.force_sync.usage = usage.total;
					self.messaging.force_sync_cache = Some(ui::force_sync::CacheSummary {
						total_bytes: usage.total,
						history_bytes: Some(usage.database),
						metadata_bytes: None,
						media_bytes: Some(usage.media),
						artwork_bytes: Some(usage.artwork),
						last_cleanup: (usage.last_cleanup_reclaimed > 0)
							.then_some("force-sync-cache-cleaned"),
					});
				}
				true
			}
			_ => false,
		}
	}

	pub(super) fn observe_force_sync_event(&mut self, event: &Event) {
		let Some(job) = self.force_sync.job.as_ref() else {
			return;
		};
		let account = job.account;
		let changed = self.force_sync.history_change(event);
		if let Some(active) = &self.force_sync.active
			&& sync_event_affects_read(&active.read, event, &self.state, account, job.guild())
		{
			if matches!(active.read, Read::History { .. }) {
				self.force_sync.history_revision = self.force_sync.history_revision.wrapping_add(1);
			} else {
				self.force_sync.metadata_revision =
					self.force_sync.metadata_revision.wrapping_add(1);
			}
		}
		self.observe_force_sync_roster_event(event);
		if let Some((channel, message)) = changed
			&& !self.queue_cache(cache::Operation::ObserveHistoryChange { channel, message })
		{
			// A missing disk fence cannot permit an old page to overwrite a live edit.
			self.force_sync.active = None;
			self.force_sync.revision = None;
			self.force_sync.revision_requested = false;
			if let Some(job) = &mut self.force_sync.job {
				job.record(Stage::History, Outcome::Partial);
				job.next_channel();
			}
			self.force_sync.dirty = true;
		}
		if let Event::ChannelPreferences(finished) = event
			&& self.force_sync.preference == Some(finished.request)
			&& self.force_sync.job.as_ref().is_some_and(|job| {
				finished.generation == job.generation && Some(finished.guild) == job.guild()
			}) {
			if let Some(job) = &mut self.force_sync.job {
				match &finished.result {
					Ok(_) => job.record(Stage::Preferences, Outcome::Updated),
					Err(failure) => job.record_failure(Stage::Preferences, *failure),
				};
				job.advance();
				self.force_sync.dirty = true;
			}
			self.force_sync.preference = None;
			self.force_sync.preference_due = None;
		}
	}

	fn observe_force_sync_roster_event(&mut self, event: &Event) {
		let Some(job) = self.force_sync.job.as_ref().filter(|job| {
			job.step == 7 && job.roster_started
				|| job.step == 10 && self.force_sync.roster_write.is_some()
		}) else {
			return;
		};
		let Some(guild) = job.guild() else {
			return;
		};
		if matches!(event, Event::Disconnected | Event::Resync)
			|| matches!(event,Event::GuildRemoved(id) if *id==guild)
		{
			self.cancel_force_sync_roster();
			return;
		}
		let changed: Vec<Id> = match event {
			Event::MemberIdentity(update) if update.guild == guild => vec![update.user],
			Event::Members(list) if list.guild == Some(guild) => list
				.slots
				.iter()
				.filter_map(|slot| match slot {
					Some(model::MemberSlot::Person(member)) => Some(member.user.id),
					_ => None,
				})
				.take(MAX_TARGETS + 1)
				.collect(),
			Event::MemberSearch {
				request,
				result: Ok(members),
			}
			| Event::MemberSearchPartial {
				request,
				rows: members,
				..
			} if request.guild == guild => members
				.iter()
				.map(|member| member.user.id)
				.take(MAX_TARGETS + 1)
				.collect(),
			_ => return,
		};
		for user in changed {
			if self.force_sync.roster_changed.len() == MAX_TARGETS
				&& !self.force_sync.roster_changed.contains(&user)
			{
				let mut job = self.force_sync.job.take().expect("job checked");
				self.finish_force_sync_roster(&mut job, Some(Failure::ResponseLimit));
				self.force_sync.job = Some(job);
				return;
			}
			self.force_sync.roster_changed.insert(user);
		}
		// If a complete disk batch was already queued, retire its epoch before a newer
		// nickname can be overwritten. A completed write is followed by scoped cleanup.
		let changed_disk = self.force_sync.roster_reconcile.is_some()
			|| self.force_sync.roster_write.is_some()
				&& self.force_sync.roster_chunk.as_ref().is_some_and(
					|RosterChunk {
					     members,
					     nick_unknown,
					     global_names,
					     ..
					 }| match event {
						Event::MemberIdentity(update) => members
							.iter()
							.find(|member| {
								member.user.id == update.user
									&& complete_member_display(
										member.user.id,
										nick_unknown,
										global_names,
									)
							})
							.is_some_and(|member| {
								let nick = match &update.nick {
									model::Patch::Value(nick) => {
										member.nick.as_deref() != Some(nick.as_str())
									}
									model::Patch::Null => member.nick.is_some(),
									model::Patch::Absent => false,
								};
								nick || update.departed
									|| update.profile.as_ref().is_some_and(|user| {
										(update.global_name != model::GlobalNameUpdate::Unchanged
											&& user.name != member.user.name) || user.username
											!= member.user.username
									})
							}),
						Event::Members(list) => list.slots.iter().any(|slot| match slot {
							Some(model::MemberSlot::Person(current)) => members
								.iter()
								.find(|member| {
									member.user.id == current.user.id
										&& complete_member_display(
											member.user.id,
											nick_unknown,
											global_names,
										)
								})
								.is_some_and(|old| {
									(list
										.missing_identity
										.iter()
										.find(|fields| fields.user == current.user.id)
										.is_none_or(|fields| !fields.nick)
										&& old.nick != current.nick) || (list
										.missing_identity
										.iter()
										.find(|fields| fields.user == current.user.id)
										.is_none_or(|fields| {
											fields.global_name != model::GlobalNameUpdate::Unchanged
										}) && old.user.name
										!= current.user.name) || old.user.username
										!= current.user.username
								}),
							_ => false,
						}),
						Event::MemberSearch {
							result: Ok(current),
							..
						} => current.iter().any(|current| {
							members
								.iter()
								.find(|member| {
									member.user.id == current.user.id
										&& complete_member_display(
											member.user.id,
											nick_unknown,
											global_names,
										)
								})
								.is_some_and(|old| {
									old.nick != current.nick
										|| old.user.name != current.user.name
										|| old.user.username != current.user.username
								})
						}),
						Event::MemberSearchPartial { rows, missing, .. } => {
							rows.iter().any(|current| {
								members
									.iter()
									.find(|member| {
										member.user.id == current.user.id
											&& complete_member_display(
												member.user.id,
												nick_unknown,
												global_names,
											)
									})
									.is_some_and(|old| {
										(missing
											.iter()
											.find(|fields| fields.user == current.user.id)
											.is_none_or(|fields| !fields.nick)
											&& old.nick != current.nick) || (missing
											.iter()
											.find(|fields| fields.user == current.user.id)
											.is_none_or(|fields| {
												fields.global_name
													!= model::GlobalNameUpdate::Unchanged
											}) && old.user.name
											!= current.user.name) || old.user.username
											!= current.user.username
									})
							})
						}
						_ => false,
					},
				);
		if changed_disk {
			let user = match event {
				Event::MemberIdentity(update) => Some(update.user),
				_ => None,
			};
			let account = self.state.user.as_ref().map(|user| user.id);
			if let (Some(cache), Some(account)) = (&self.cache, account) {
				if cache.invalidate_scope(
					self.state.generation,
					account,
					cache::Operation::RemoveMember { guild, user },
				) {
					self.cache_pending += 1;
				} else {
					self.cache_error = true;
					self.cache_status =
						"Could not retire a changed member cache entry; cache reads remain paused";
				}
			}
		}
	}
	fn force_sync_epoch_valid(&self, generation: u64, epoch: u64) -> bool {
		generation == self.state.generation
			&& self
				.cache
				.as_ref()
				.is_some_and(|cache| cache.history.allows(epoch))
	}
	pub(super) fn cancel_force_sync_roster(&mut self) {
		if let Some(connection) = &self.connection {
			connection.sync_roster.cancel();
		}
		self.force_sync.roster_chunk = None;
		self.force_sync.roster_changed.clear();
		self.force_sync.roster_write = None;
		self.force_sync.roster_finished = false;
		self.force_sync.roster_users = None;
		self.force_sync.roster_reconcile = None;
		self.force_sync.roster_attempt = None;
		if let Some(job) = &mut self.force_sync.job {
			job.roster_started = false;
		}
	}
	fn finish_force_sync_roster(&mut self, job: &mut Job, failure: Option<Failure>) {
		let changed = !self.force_sync.roster_changed.is_empty();
		if let Some(failure) = failure {
			job.record_failure(Stage::Members, failure);
			job.roster_complete = false;
		} else {
			job.roster_complete = !changed;
			job.record(
				Stage::Members,
				if changed {
					Outcome::Partial
				} else {
					Outcome::Updated
				},
			);
		}
		job.add_members(self.force_sync.roster_changed.iter().copied());
		if !job.roster_complete
			&& let Some(guild) = job.guild()
		{
			job.add_known_members(&self.state, guild);
		}
		self.cancel_force_sync_roster();
		job.roster_started = false;
		job.advance();
		self.force_sync.dirty = true;
	}
	fn drive_force_sync_roster(&mut self, job: &mut Job) {
		let Some(guild) = job.guild() else {
			job.advance();
			return;
		};
		let Some(permit) = self.state.force_sync_roster_permit(guild) else {
			self.cancel_force_sync_roster();
			job.roster_started = false;
			job.roster_complete = false;
			job.add_known_members(&self.state, guild);
			job.record(Stage::Members, Outcome::Partial);
			job.advance();
			self.force_sync.dirty = true;
			return;
		};
		if self.force_sync.roster_write.is_some() {
			return;
		}
		if self.force_sync.roster_reconcile.is_some() {
			return;
		}
		if let Some(RosterChunk {
			guild: chunk_guild,
			members,
			nick_unknown,
			global_names,
			index,
		}) = self.force_sync.roster_chunk.take()
		{
			if chunk_guild != guild {
				self.finish_force_sync_roster(job, Some(Failure::Protocol));
				return;
			}
			let end = (index + 100).min(members.len());
			let accepted: Vec<_> = members[index..end]
				.iter()
				.filter(|member| !self.force_sync.roster_changed.contains(&member.user.id))
				.cloned()
				.collect();
			let accepted_unknown: Vec<_> = nick_unknown
				.iter()
				.copied()
				.filter(|id| accepted.iter().any(|member| member.user.id == *id))
				.collect();
			let accepted_names: Vec<_> = global_names
				.iter()
				.copied()
				.filter(|(id, _)| accepted.iter().any(|member| member.user.id == *id))
				.collect();
			self.state.apply_force_sync_members_with_presence(
				guild,
				&accepted,
				&accepted_unknown,
				&accepted_names,
			);
			if end < members.len() {
				self.force_sync.roster_chunk = Some(RosterChunk {
					guild,
					members,
					nick_unknown,
					global_names,
					index: end,
				});
				return;
			}
			let rows = member_display_rows(
				guild,
				members
					.iter()
					.filter(|member| !self.force_sync.roster_changed.contains(&member.user.id)),
				&nick_unknown,
				&global_names,
			);
			job.request = job.request.wrapping_add(1);
			if self.queue_cache(cache::Operation::SyncSaveMembers {
				job: job.id,
				request: job.request,
				guild,
				rows,
				missing: Vec::new(),
			}) {
				self.force_sync.roster_write = Some((job.request, guild));
				self.force_sync.roster_chunk = Some(RosterChunk {
					guild,
					members,
					nick_unknown,
					global_names,
					index: end,
				});
			} else {
				self.finish_force_sync_roster(job, Some(Failure::Busy));
			}
			return;
		}
		if self.force_sync.roster_finished {
			if !self.force_sync.roster_changed.is_empty() {
				self.finish_force_sync_roster(job, None);
				return;
			}
			let Some(users) = self.force_sync.roster_users.as_ref() else {
				self.finish_force_sync_roster(job, Some(Failure::Protocol));
				return;
			};
			if users.len() > 250_000 || users.binary_search(&job.account).is_err() {
				self.finish_force_sync_roster(job, Some(Failure::Protocol));
				return;
			}
			job.request = job.request.wrapping_add(1);
			if self.queue_cache(cache::Operation::SyncReconcileMembers {
				job: job.id,
				request: job.request,
				guild,
				users: users.clone(),
			}) {
				self.force_sync.roster_reconcile = Some(job.request);
			} else {
				self.finish_force_sync_roster(job, Some(Failure::Busy));
			}
			return;
		}
		if !job.roster_started {
			let Some(connection) = &self.connection else {
				return;
			};
			self.force_sync.roster_sequence =
				self.force_sync.roster_sequence.wrapping_add(1).max(1);
			let attempt = self.force_sync.roster_sequence;
			match connection.sync_roster.start_roster(permit, attempt) {
				Ok(()) => {
					job.roster_started = true;
					self.force_sync.roster_attempt = Some(attempt);
					self.force_sync.roster_changed.clear();
					self.force_sync.dirty = true;
				}
				Err(failure) => self.finish_force_sync_roster(job, Some(failure)),
			}
			return;
		}
		let event = self
			.connection
			.as_ref()
			.and_then(|connection| connection.sync_roster.try_recv());
		let Some(event) = event else {
			return;
		};
		use discord_gateway::roster::Event as R;
		let request = match &event {
			R::Chunk { request, .. } | R::Failed { request, .. } => *request,
			R::Finished { request, .. } => *request,
		};
		if request.guild != guild || Some(request.job) != self.force_sync.roster_attempt {
			return;
		}
		match event {
			R::Chunk {
				members,
				nick_unknown,
				global_names,
				..
			} => {
				let users: std::collections::BTreeSet<_> =
					members.iter().map(|member| member.user.id).collect();
				if !nick_unknown.windows(2).all(|ids| ids[0] < ids[1])
					|| nick_unknown.iter().any(|id| !users.contains(id))
					|| !client_core::force_sync_read::valid_member_name_updates(
						&global_names,
						&members,
					) {
					self.finish_force_sync_roster(job, Some(Failure::Protocol));
					return;
				}
				if !nick_unknown.is_empty()
					|| global_names
						.iter()
						.any(|(_, name)| *name == model::GlobalNameUpdate::Unchanged)
				{
					job.record(Stage::Members, Outcome::Partial);
				}
				self.force_sync.roster_chunk = Some(RosterChunk {
					guild,
					members,
					nick_unknown,
					global_names,
					index: 0,
				});
			}
			R::Finished { users, .. } => {
				self.force_sync.roster_finished = true;
				self.force_sync.roster_users = Some(users);
			}
			R::Failed { failure, .. } => self.finish_force_sync_roster(job, Some(failure)),
		}
	}
	fn restore_force_sync_continuation(&mut self, job: &mut Job) {
		let Some(saved) = self.force_sync.resume.take() else {
			return;
		};
		if saved.id != job.id
			|| saved.account != job.account
			|| saved.generation != job.generation
			|| saved.guild() != job.guild()
			|| saved.guild_index != job.guild_index
			|| saved.step < 3
			|| !saved.valid()
		{
			return;
		}
		let mut saved = *saved;
		let archive_target = saved
			.targets
			.get(saved.archive_index)
			.map(|target| target.id);
		let target = saved
			.targets
			.get(saved.channel_index)
			.map(|target| target.id);
		let mut next_index = 0;
		let guild = saved.guild();
		let retained = saved
			.targets
			.iter()
			.enumerate()
			.filter(|(_, target)| target.readable(&self.state, guild))
			.map(|(index, target)| {
				if index < saved.channel_index {
					next_index += 1;
				}
				target.clone()
			})
			.collect();
		saved.targets = retained;
		saved.channel_index = next_index.min(saved.targets.len());
		if target
			!= saved
				.targets
				.get(saved.channel_index)
				.map(|target| target.id)
		{
			saved.before = last_before(saved.end_ms);
			saved.record(Stage::History, Outcome::Partial);
		}
		saved.archive_index = archive_target
			.and_then(|id| saved.targets.iter().position(|target| target.id == id))
			.unwrap_or_else(|| {
				saved.archive_time = None;
				saved.archive_id = None;
				saved.archive_kind = 0;
				saved.archive_index.min(saved.targets.len())
			});
		let prior_len = saved.targets.len();
		let previous_history = saved
			.targets
			.iter()
			.filter(|target| target.has_history())
			.count();
		let mut added_parent = false;
		for target in &job.targets {
			if target.readable(&self.state, guild)
				&& !saved.targets.iter().any(|old| old.id == target.id)
			{
				added_parent |= matches!(target.kind, 0 | 5 | 15 | 16);
				if !saved.add_targets(std::iter::once(target.clone())) {
					saved.record_failure(Stage::Channels, Failure::Capacity);
					break;
				}
			}
		}
		if saved.step >= 9 {
			let new_history = saved
				.targets
				.iter()
				.filter(|target| target.has_history())
				.count()
				.saturating_sub(previous_history);
			saved.channels_total = saved.channels_total.saturating_add(new_history as u64);
			if saved.step > 9 && new_history > 0 {
				saved.step = 9;
				saved.channel_index = prior_len;
				saved.before = last_before(saved.end_ms);
			}
			if added_parent {
				// The resumed job already passed archive discovery for this new parent.
				saved.record(Stage::Channels, Outcome::Partial);
			}
		}
		saved.roster_started = false;
		saved.request = saved.request.max(job.request);
		for index in 0..3 {
			let previous = saved.outcomes[index];
			saved.outcomes[index] = combine_outcomes(previous, job.outcomes[index]);
			if job.outcomes[index] > 1 {
				saved.details[index] = job.details[index];
			}
		}
		*job = saved;
		self.force_sync.dirty = true;
	}
}

fn active_read_is_current(
	job: &Job,
	state: &client_core::State,
	read: &Read,
	guild: Id,
	parent: Id,
) -> bool {
	job.step == 2
		&& job.guild() == Some(guild)
		&& job
			.targets
			.get(job.active_index)
			.is_some_and(|target| target.id == parent)
		&& state
			.channel(parent)
			.is_some_and(|channel| channel.guild == Some(guild))
		&& state.can_archive(parent, model::archives::Kind::Public)
		&& matches!(read, Read::ActiveThreads {
			guild: requested_guild, parent: requested_parent, before
		} if *requested_guild == guild && *requested_parent == parent && *before == job.active_before)
}

fn history_event_change(read: &Read, event: &Event) -> Option<(Id, Id)> {
	let Read::History {
		channel,
		before,
		cutoff_ms,
		end_ms,
	} = read
	else {
		return None;
	};
	let upper = last_before(*end_ms)?;
	let overlaps = |changed: Id, message: Id| {
		changed == *channel
			&& client_core::force_sync_read::timestamp_ms(message) >= *cutoff_ms
			&& message < upper
			&& before.is_none_or(|cursor| message < cursor)
	};
	match event {
		Event::Message(message) if overlaps(message.channel, message.id) => {
			Some((message.channel, message.id))
		}
		Event::Edited {
			channel,
			message,
			result: Ok(_),
			..
		}
		| Event::Delete {
			channel,
			id: message,
		} if overlaps(*channel, *message) => Some((*channel, *message)),
		Event::Patch(patch) if overlaps(patch.channel, patch.id) => Some((patch.channel, patch.id)),
		Event::DeleteBulk { channel, ids } => ids
			.iter()
			.copied()
			.find(|id| overlaps(*channel, *id))
			.map(|id| (*channel, id)),
		_ => None,
	}
}

/// Only changes to the resource being read can invalidate its response. Continuous
/// traffic in another channel must not prevent background history from making progress.
fn sync_event_affects_read(
	read: &Read,
	event: &Event,
	state: &client_core::State,
	account: Id,
	guild_hint: Option<Id>,
) -> bool {
	if history_event_change(read, event).is_some() {
		return true;
	}
	let channel = match read {
		Read::History { channel, .. } => Some(*channel),
		_ => None,
	};
	let parent = match read {
		Read::ActiveThreads { parent, .. } | Read::Archives { parent, .. } => Some(*parent),
		_ => None,
	};
	let guild = read
		.guild()
		.or_else(|| {
			channel
				.and_then(|id| state.channel(id))
				.and_then(|channel| channel.guild)
		})
		.or_else(|| {
			matches!(read, Read::History { .. })
				.then_some(guild_hint)
				.flatten()
		});
	match event {
		Event::GuildRemoved(changed) => guild == Some(*changed),
		Event::GuildChanged(patch) => matches!(read, Read::Guild { .. }) && guild == Some(patch.id),
		Event::GuildEmojis { guild: changed, .. } => {
			matches!(read, Read::Emojis { .. }) && guild == Some(*changed)
		}
		Event::GuildStickers { guild: changed, .. } => {
			matches!(read, Read::Stickers { .. }) && guild == Some(*changed)
		}
		Event::Soundboard(client_core::soundboard::Event::Changed(changed)) => {
			matches!(read, Read::Sounds { .. }) && guild == Some(*changed)
		}
		Event::MemberIdentity(update) => {
			guild == Some(update.guild)
				&& match read {
					Read::Guild { .. } => update.user == account,
					Read::Members { users, .. } => users.contains(&update.user),
					_ => false,
				}
		}
		Event::MemberSearch {
			request,
			result: Ok(rows),
		}
		| Event::MemberSearchPartial { request, rows, .. } => {
			guild == Some(request.guild)
				&& match read {
					Read::Guild { user, .. } => rows.iter().any(|row| row.user.id == *user),
					Read::Members { users, .. } => {
						rows.iter().any(|row| users.contains(&row.user.id))
					}
					_ => false,
				}
		}
		Event::Members(list) => {
			guild == list.guild
				&& list.slots.iter().flatten().any(|slot| {
					let model::MemberSlot::Person(member) = slot else {
						return false;
					};
					match read {
						Read::Guild { user, .. } => member.user.id == *user,
						Read::Members { users, .. } => users.contains(&member.user.id),
						_ => false,
					}
				})
		}
		Event::PermissionsChanged => matches!(
			read,
			Read::Guild { .. }
				| Read::Channels { .. }
				| Read::History { .. }
				| Read::ActiveThreads { .. }
				| Read::Archives { .. }
		),
		Event::Permissions(update)
			if matches!(
				read,
				Read::Guild { .. }
					| Read::Channels { .. }
					| Read::History { .. }
					| Read::ActiveThreads { .. }
					| Read::Archives { .. }
			) =>
		{
			use client_core::permissions::Event as P;
			match update {
				P::Snapshot(snapshot) => {
					snapshot.guilds.iter().any(|g| Some(g.id) == guild)
						|| snapshot.channels.iter().any(|c| {
							if let Some(scoped) = channel.or(parent) {
								c.id == scoped
							} else {
								Some(c.guild) == guild
							}
						})
				}
				P::Guild(value) => guild == Some(value.id),
				P::Role { guild: changed, .. }
				| P::RoleRemoved { guild: changed, .. }
				| P::Member { guild: changed, .. }
				| P::Owner { guild: changed, .. }
				| P::UnavailableGuild(changed) => guild == Some(*changed),
				P::Members(updates) => updates
					.iter()
					.any(|(changed, _, _)| guild == Some(*changed)),
				P::Channel {
					channel: changed,
					guild: changed_guild,
					..
				} => {
					if let Some(scoped) = channel.or(parent) {
						scoped == *changed
					} else {
						guild.is_some() && guild == *changed_guild
					}
				}
			}
		}
		Event::ChannelCreated(changed) | Event::ChannelRestored(changed) => {
			guild.is_some()
				&& guild == changed.guild
				&& (matches!(read, Read::Channels { .. })
					|| parent.is_some_and(|id| changed.id == id || changed.parent_id == Some(id)))
		}
		Event::ChannelChanged(changed) => {
			channel == Some(changed.id)
				|| (matches!(read, Read::Channels { .. })
					&& state
						.channel(changed.id)
						.is_some_and(|c| guild.is_some() && c.guild == guild))
				|| parent.is_some_and(|id| {
					changed.id == id
						|| state
							.channel(changed.id)
							.is_some_and(|c| c.parent_id == Some(id))
						|| matches!(&changed.parent_id, model::Patch::Value(value) if *value == id)
				})
		}
		Event::ThreadChanged {
			guild: changed,
			patch,
		} => {
			channel == Some(patch.id)
				|| guild == Some(*changed)
					&& parent.is_some_and(|id| {
						state
							.channel(patch.id)
							.is_none_or(|c| c.parent_id == Some(id))
							|| matches!(&patch.parent_id, model::Patch::Value(value) if *value == id)
					})
		}
		Event::ThreadRemoved { guild: changed, id } => {
			channel == Some(*id)
				|| guild == Some(*changed)
					&& parent.is_some_and(|parent| {
						state
							.channel(*id)
							.is_none_or(|c| c.parent_id == Some(parent))
					})
		}
		Event::ThreadsSync {
			guild: changed,
			parents,
			removed,
			..
		} => {
			channel.is_some_and(|id| removed.contains(&id))
				|| guild == Some(*changed)
					&& parent.is_some_and(|id| {
						parents.as_ref().is_none_or(|parents| parents.contains(&id))
					})
		}
		Event::Unavailable(changed) => {
			channel == Some(*changed)
				|| parent.is_some_and(|id| {
					*changed == id
						|| state
							.channel(*changed)
							.is_some_and(|c| c.parent_id == Some(id))
				}) || (matches!(read, Read::Channels { .. })
				&& state
					.channel(*changed)
					.is_some_and(|c| guild.is_some() && c.guild == guild))
		}
		_ => false,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn frozen_history() -> (client_core::State, Job, Read) {
		let mut state = test_support::demo_state();
		state.demo = false;
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		let channel = state.channel(state.selected.unwrap()).unwrap();
		assert!(state.can_read_history(channel.id));
		let mut job = Job::new(&state, Scope::Server(channel.guild.unwrap())).unwrap();
		job.cutoff_ms = client_core::force_sync_read::DISCORD_EPOCH_MS + 1_000;
		job.end_ms = job.cutoff_ms + HISTORY_MILLIS;
		job.targets = vec![Target {
			id: channel.id,
			kind: channel.kind,
			parent: channel.parent_id,
		}];
		job.step = 9;
		job.request = 1;
		job.channels_total = 1;
		job.before = last_before(job.end_ms);
		let read = Read::History {
			channel: channel.id,
			before: job.before,
			cutoff_ms: job.cutoff_ms,
			end_ms: job.end_ms,
		};
		assert!(job.valid());
		(state, job, read)
	}

	fn history_events(channel: Id, id: Id) -> [Event; 5] {
		[
			Event::Message(test_support::message(id.0, channel)),
			Event::Patch(model::MessagePatch {
				poll: model::Patch::Absent,
				sticker_items: model::Patch::Absent,
				flags: model::Patch::Absent,
				components: model::Patch::Absent,
				application_id: model::Patch::Absent,
				extra_content: Default::default(),
				reactions: model::Patch::Absent,
				id,
				channel,
				content: model::Patch::Value("Current content".into()),
				mentions: model::Patch::Absent,
				edited: model::Patch::Absent,
				embeds: model::Patch::Absent,
				embeds_suppressed: model::Patch::Absent,
				attachments: model::Patch::Absent,
			}),
			Event::Edited {
				request: 1,
				channel,
				message: id,
				result: Ok(test_support::message(id.0, channel)),
			},
			Event::Delete { channel, id },
			Event::DeleteBulk {
				channel,
				ids: vec![id],
			},
		]
	}

	#[test]
	fn frozen_history_read_ignores_post_freeze_traffic_in_its_channel() {
		let (state, mut job, read) = frozen_history();
		let channel = job.targets[0].id;
		let upper = last_before(job.end_ms).unwrap();
		let mut sync = Sync::default();
		sync.prepare_read(&read);
		for _ in 0..MAX_STALE_RETRIES + 1 {
			for event in history_events(channel, upper) {
				if sync_event_affects_read(&read, &event, &state, job.account, job.guild()) {
					sync.reject_stale(&mut job, &read);
				}
			}
		}
		assert_eq!(job.channel_index, 0);
		assert_eq!(job.channels_done, 0);
		assert_eq!(sync.stale_retries, 0);
	}

	#[test]
	fn frozen_history_read_fences_exact_cutoff_end_and_before_boundaries() {
		let (state, job, read) = frozen_history();
		let channel = job.targets[0].id;
		let cutoff = Id(1_000 << 22);
		let upper = last_before(job.end_ms).unwrap();
		let before = Id(upper.0 - 100);
		for (cursor, id, affects) in [
			(None, Id(cutoff.0 - 1), false),
			(None, cutoff, true),
			(None, Id(upper.0 - 1), true),
			(None, upper, false),
			(None, Id(upper.0 + 1), false),
			(Some(before), Id(before.0 - 1), true),
			(Some(before), before, false),
			(Some(before), Id(before.0 + 1), false),
		] {
			let mut read = read.clone();
			let Read::History { before, .. } = &mut read else {
				unreachable!();
			};
			*before = cursor;
			for event in history_events(channel, id) {
				assert_eq!(
					sync_event_affects_read(&read, &event, &state, job.account, job.guild()),
					affects,
					"id={id:?}, before={cursor:?}"
				);
			}
		}
	}

	#[test]
	fn frozen_history_read_checks_every_bulk_delete_id() {
		let (state, job, read) = frozen_history();
		let channel = job.targets[0].id;
		let upper = last_before(job.end_ms).unwrap();
		for (ids, affects) in [
			(vec![], false),
			(vec![upper, Id(upper.0 + 1)], false),
			(vec![upper, Id(upper.0 - 1)], true),
			(vec![Id(upper.0 - 1), upper], true),
		] {
			assert_eq!(
				sync_event_affects_read(
					&read,
					&Event::DeleteBulk { channel, ids },
					&state,
					job.account,
					job.guild()
				),
				affects
			);
		}
	}

	fn frozen_history_store(job: &Job) -> local_store::LocalStore {
		let mut store = local_store::LocalStore::open(std::path::Path::new(":memory:")).unwrap();
		store
			.save_sync_checkpoint(
				job.account,
				&local_store::SyncCheckpoint {
					job: job.id,
					scope: job.scope,
					started_ms: job.end_ms,
					cutoff_ms: job.cutoff_ms,
					payload: serde_json::to_vec(job).unwrap(),
				},
			)
			.unwrap();
		store
	}

	fn frozen_history_coverage(job: &Job, revision: u64) -> local_store::HistoryCoverage {
		local_store::HistoryCoverage {
			job: job.id,
			request: job.request,
			cutoff: Id(
				((job.cutoff_ms - client_core::force_sync_read::DISCORD_EPOCH_MS) as u64) << 22,
			),
			upper: last_before(job.end_ms).unwrap(),
			before: job.before,
			next_before: None,
			complete: true,
			revision,
		}
	}

	#[test]
	fn post_freeze_messages_preserve_disk_revision_and_allow_history_commit() {
		let (_, job, _) = frozen_history();
		let channel = job.targets[0].id;
		let upper = last_before(job.end_ms).unwrap();
		let mut store = frozen_history_store(&job);
		let revision = store.history_revision(job.account, channel).unwrap();
		let coverage = frozen_history_coverage(&job, revision);
		let sync = Sync {
			job: Some(job.clone()),
			pending_page: Some(PageCommit {
				channel,
				request: job.request,
				next: Some(Id(upper.0 - 1)),
				done: true,
			}),
			..Default::default()
		};
		for _ in 0..MAX_STALE_RETRIES + 1 {
			let [created, patched, edited, ..] = history_events(channel, upper);
			for event in [created, patched, edited] {
				let changed = sync.history_change(&event);
				if let Some((channel, message)) = changed {
					store
						.observe_history_change(job.account, channel, message, false)
						.unwrap();
				}
				assert!(changed.is_none());
			}
		}
		assert_eq!(
			store.history_revision(job.account, channel).unwrap(),
			revision
		);
		let row = test_support::message(upper.0 - 1, channel);
		let saved = store
			.save_sync_page(job.account, channel, &[row], &coverage)
			.unwrap();
		assert!(!saved.stale);
		assert_eq!(saved.saved, 1);
		assert!(
			store
				.history_coverage(job.account, channel)
				.unwrap()
				.unwrap()
				.complete
		);
	}

	#[test]
	fn overlapping_live_updates_preserve_fences_before_history_commit() {
		let (_, job, _) = frozen_history();
		let channel = job.targets[0].id;
		let upper = last_before(job.end_ms).unwrap();
		let id = Id(upper.0 - 1);
		let sync = Sync {
			job: Some(job.clone()),
			pending_page: Some(PageCommit {
				channel,
				request: job.request,
				next: Some(id),
				done: true,
			}),
			..Default::default()
		};
		let mut events = Vec::from(history_events(channel, id));
		events.push(Event::DeleteBulk {
			channel,
			ids: vec![upper, id],
		});
		for event in events {
			let mut store = frozen_history_store(&job);
			let row = test_support::message(id.0, channel);
			store
				.save_channel(job.account, channel, std::slice::from_ref(&row), false)
				.unwrap();
			let coverage = frozen_history_coverage(
				&job,
				store.history_revision(job.account, channel).unwrap(),
			);
			let changed = sync.history_change(&event);
			assert_eq!(changed, Some((channel, id)));
			store
				.observe_history_change(job.account, channel, changed.unwrap().1, false)
				.unwrap();
			let deleted = match &event {
				Event::Delete { id, .. } => {
					store.delete_messages(job.account, channel, &[*id]).unwrap();
					true
				}
				Event::DeleteBulk { ids, .. } => {
					store.delete_messages(job.account, channel, ids).unwrap();
					true
				}
				_ => {
					let mut current = row.clone();
					current.content = "Live edit".into();
					store
						.save_changes(job.account, channel, &[current], &[id], false)
						.unwrap();
					false
				}
			};
			assert!(
				store
					.save_sync_page(job.account, channel, &[row], &coverage)
					.unwrap()
					.stale
			);
			let cached = store.load_channel(job.account, channel).unwrap();
			if deleted {
				assert!(cached.is_empty());
			} else {
				assert_eq!(cached[0].content, "Live edit");
			}
		}
	}

	#[test]
	fn older_history_pages_ignore_newer_edits_and_keep_continuation_fences() {
		let (_, mut job, _) = frozen_history();
		let channel = job.targets[0].id;
		let upper = last_before(job.end_ms).unwrap();
		let mut store = frozen_history_store(&job);
		let rows: Vec<_> = (1..=50)
			.map(|offset| test_support::message(upper.0 - offset, channel))
			.collect();
		let before = rows.last().unwrap().id;
		let first = local_store::HistoryCoverage {
			next_before: Some(before),
			complete: false,
			..frozen_history_coverage(&job, 0)
		};
		assert!(
			!store
				.save_sync_page(job.account, channel, &rows, &first)
				.unwrap()
				.stale
		);
		job.before = Some(before);
		job.request += 1;
		let coverage = frozen_history_coverage(&job, 0);
		let mut edited = rows[0].clone();
		edited.content = "Newer live edit".into();
		let event = Event::Edited {
			request: 1,
			channel,
			message: edited.id,
			result: Ok(edited.clone()),
		};
		let sync = Sync {
			job: Some(job.clone()),
			..Default::default()
		};
		assert!(sync.history_change(&event).is_none());
		store
			.save_changes(
				job.account,
				channel,
				std::slice::from_ref(&edited),
				&[edited.id],
				false,
			)
			.unwrap();
		assert_eq!(
			store
				.load_channel(job.account, channel)
				.unwrap()
				.iter()
				.find(|message| message.id == edited.id)
				.unwrap()
				.content,
			"Newer live edit"
		);
		let row = test_support::message(before.0 - 2, channel);
		let older_id = row.id;
		let wrong_cursor = local_store::HistoryCoverage {
			before: Some(Id(before.0 - 1)),
			..coverage
		};
		assert!(
			store
				.save_sync_page(
					job.account,
					channel,
					std::slice::from_ref(&row),
					&wrong_cursor
				)
				.unwrap()
				.stale
		);
		assert!(
			!store
				.save_sync_page(job.account, channel, &[row], &coverage)
				.unwrap()
				.stale
		);
		let cached = store.load_channel(job.account, channel).unwrap();
		assert!(cached.iter().any(|message| message.id == older_id));
		assert_eq!(
			cached
				.iter()
				.find(|message| message.id == edited.id)
				.unwrap()
				.content,
			"Newer live edit"
		);
		assert!(
			store
				.save_sync_page(job.account, channel, &rows, &first)
				.unwrap()
				.stale
		);
	}

	#[test]
	fn incomplete_names_leave_existing_sqlite_display_intact() {
		let mut store = local_store::LocalStore::open(std::path::Path::new(":memory:")).unwrap();
		let account = Id(1);
		let guild = Id(10);
		let user = Id(7);
		let prior = model::MemberDisplay {
			guild,
			user,
			name: "Known display".into(),
			username: Some("account".into()),
			nick: Some("Nickname".into()),
		};
		store
			.save_member_displays(account, std::slice::from_ref(&prior))
			.unwrap();
		let mut profile = test_support::message(3, Id(20)).author;
		profile.id = user;
		profile.name = "account".into();
		profile.username = Some("account".into());
		let member = model::Member {
			user: profile,
			nick: Some("Nickname".into()),
			roles: vec![],
			status: None,
			custom_status: None,
			activities: vec![],
			clients: Default::default(),
		};
		let missing = [(user, model::GlobalNameUpdate::Unchanged)];
		let rows = member_display_rows(guild, std::iter::once(&member), &[], &missing);
		assert!(rows.is_empty());
		store.save_member_displays(account, &rows).unwrap();
		assert_eq!(
			store.load_member_displays(account, guild).unwrap(),
			vec![prior]
		);
		let cleared = [(user, model::GlobalNameUpdate::Cleared)];
		let rows = member_display_rows(guild, std::iter::once(&member), &[], &cleared);
		assert_eq!(rows.len(), 1);
		store.save_member_displays(account, &rows).unwrap();
		assert_eq!(
			store.load_member_displays(account, guild).unwrap()[0].name,
			"account"
		);
		assert!(member_display_rows(guild, std::iter::once(&member), &[user], &cleared).is_empty());
	}
	#[test]
	fn job_identity_does_not_repeat_within_one_millisecond_or_after_clock_reversal() {
		let first = next_job_id(42).unwrap();
		let second = next_job_id(42).unwrap();
		let reversed = next_job_id(1).unwrap();
		assert!(first < second && second < reversed);
	}
	#[test]
	fn checkpoint_is_bounded_and_does_not_serialize_live_permissions() {
		let state = test_support::demo_state();
		let job = Job::new(&state, Scope::Account).unwrap();
		assert!(job.valid());
		let bytes = serde_json::to_vec(&job).unwrap();
		assert!(bytes.len() < MAX_CHECKPOINT);
		assert!(!String::from_utf8_lossy(&bytes).contains("permissions"));
		assert!(serde_json::from_slice::<Job>(&bytes).unwrap().valid());
	}
	#[test]
	fn partial_outcomes_survive_later_success() {
		let state = test_support::demo_state();
		let mut job = Job::new(&state, Scope::Account).unwrap();
		job.record(Stage::Members, Outcome::Partial);
		job.record(Stage::Members, Outcome::Updated);
		assert_eq!(job.outcomes[Job::stage_index(Stage::Members)], 2);
	}
	#[test]
	fn successful_channels_and_failed_thread_reads_are_partial_in_either_order() {
		for (first, second) in [
			(Outcome::Updated, Outcome::Unavailable),
			(Outcome::Unavailable, Outcome::Updated),
			(Outcome::Failed, Outcome::Updated),
		] {
			let mut job = Job::new(&test_support::demo_state(), Scope::Account).unwrap();
			job.record(Stage::Channels, first);
			job.record(Stage::Channels, second);
			assert_eq!(job.outcomes[Job::stage_index(Stage::Channels)], 2);
		}
		assert_eq!(combine_outcomes(3, 3), 3);
		assert_eq!(combine_outcomes(4, 4), 4);
	}
	#[test]
	fn old_checkpoints_keep_their_history_cursor_and_channel_outcome() {
		let mut job = Job::new(&test_support::demo_state(), Scope::Account).unwrap();
		job.step = 9;
		job.before = Some(Id(123));
		job.record(Stage::Channels, Outcome::Unavailable);
		let mut value = serde_json::to_value(&job).unwrap();
		value["version"] = 1.into();
		let object = value.as_object_mut().unwrap();
		object.remove("active_index");
		object.remove("active_before");
		object.remove("active_pages");
		let mut restored: Job = serde_json::from_value(value).unwrap();
		restored.migrate_checkpoint();
		assert!(restored.valid());
		assert_eq!(restored.before, Some(Id(123)));
		assert_eq!(restored.step, 9);
		assert_eq!(restored.outcomes[Job::stage_index(Stage::Channels)], 3);
	}
	#[test]
	fn active_threads_are_scoped_to_accessible_parents_and_paginated() {
		let state = test_support::demo_state();
		let parent = state.selected.unwrap();
		let channel = state.channel(parent).unwrap();
		let guild = channel.guild.unwrap();
		let mut job = Job::new(&state, Scope::Server(guild)).unwrap();
		job.targets = vec![Target {
			id: parent,
			kind: channel.kind,
			parent: channel.parent_id,
		}];
		job.step = 2;
		let first = job.next_active_read(&state, guild).unwrap();
		assert!(
			matches!(first, Read::ActiveThreads { guild: g, parent: p, before: None } if g == guild && p == parent)
		);
		assert!(active_read_is_current(&job, &state, &first, guild, parent));
		job.active_before = Some(Id(123));
		assert!(!active_read_is_current(&job, &state, &first, guild, parent));
		assert!(matches!(
			job.next_active_read(&state, guild),
			Some(Read::ActiveThreads {
				before: Some(Id(123)),
				..
			})
		));
		job.skip_read(&first);
		assert!(job.next_active_read(&state, guild).is_none());
		assert_eq!(job.active_before, None);
	}
	#[test]
	fn indexing_retry_is_bounded_and_waits_for_the_supplied_delay() {
		let mut sync = Sync::default();
		let now = Instant::now();
		assert!(sync.defer_indexing(Duration::from_millis(1500), now));
		assert_eq!(sync.next_read, Some(now + Duration::from_millis(1500)));
		assert!(sync.defer_indexing(Duration::ZERO, now));
		assert_eq!(sync.next_read, Some(now + Duration::from_millis(50)));
		assert!(!sync.defer_indexing(Duration::from_secs(1), now));
		sync.read_retries = 0;
		assert!(!sync.defer_indexing(Duration::from_secs(31), now));
	}
	#[test]
	fn archived_thread_requests_follow_parent_type_and_permission() {
		let mut state = test_support::demo_state();
		let parent = state.selected.unwrap();
		let guild = state.channel(parent).unwrap().guild.unwrap();
		let mut job = Job::new(&state, Scope::Server(guild)).unwrap();
		job.targets = vec![Target {
			id: parent,
			kind: 0,
			parent: None,
		}];
		assert!(matches!(
			job.next_archive_read(&state, guild),
			Some(Read::Archives {
				kind: model::archives::Kind::Public,
				..
			})
		));
		job.advance_archive();
		assert!(matches!(
			job.next_archive_read(&state, guild),
			Some(Read::Archives {
				kind: model::archives::Kind::JoinedPrivate,
				..
			})
		));
		job.advance_archive();
		assert!(job.next_archive_read(&state, guild).is_none());
		state
			.channels
			.iter_mut()
			.find(|channel| channel.id == parent)
			.unwrap()
			.kind = 15;
		job.targets[0].kind = 15;
		job.archive_index = 0;
		assert!(matches!(
			job.next_archive_read(&state, guild),
			Some(Read::Archives {
				kind: model::archives::Kind::Public,
				..
			})
		));
		job.advance_archive();
		assert!(job.next_archive_read(&state, guild).is_none());
	}
	#[test]
	fn thread_discovery_is_invalidated_by_its_permissions_without_other_channel_noise() {
		let state = test_support::demo_state();
		let parent = state.selected.unwrap();
		let guild = state.channel(parent).unwrap().guild.unwrap();
		let account = state.user.as_ref().unwrap().id;
		for read in [
			Read::ActiveThreads {
				guild,
				parent,
				before: None,
			},
			Read::Archives {
				guild,
				parent,
				kind: model::archives::Kind::Public,
				before: None,
			},
		] {
			let event = |channel| {
				Event::Permissions(client_core::permissions::Event::Channel {
					channel,
					guild: Some(guild),
					overwrites: model::Patch::Absent,
				})
			};
			assert!(sync_event_affects_read(
				&read,
				&event(parent),
				&state,
				account,
				None
			));
			assert!(!sync_event_affects_read(
				&read,
				&event(Id(u64::MAX)),
				&state,
				account,
				None
			));
			assert!(sync_event_affects_read(
				&read,
				&Event::PermissionsChanged,
				&state,
				account,
				None
			));
		}
	}
	#[test]
	fn member_query_results_fence_matching_force_sync_reads() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let account = state.user.as_ref().unwrap().id;
		let Some(client_core::Command::MemberSearch(request)) =
			state.search_members(channel, "person", 0)
		else {
			panic!("member query");
		};
		let mut user = test_support::message(3, channel).author;
		user.id = Id(7);
		let row = model::Member {
			user,
			nick: None,
			roles: vec![],
			status: None,
			custom_status: None,
			activities: vec![],
			clients: Default::default(),
		};
		let event = Event::MemberSearchPartial {
			request,
			rows: vec![row.clone()],
			missing: vec![model::MissingMemberFields {
				global_name: model::GlobalNameUpdate::Set,
				user: Id(7),
				nick: true,
				roles: false,
			}],
		};
		let read = |id| Read::Members {
			guild,
			users: vec![Id(id)],
		};
		assert!(sync_event_affects_read(
			&read(7),
			&event,
			&state,
			account,
			None
		));
		assert!(!sync_event_affects_read(
			&read(8),
			&event,
			&state,
			account,
			None
		));
		state.request_members();
		let mut list = state.members.clone().expect("member list");
		list.slots = vec![Some(model::MemberSlot::Person(row))];
		let event = Event::Members(list);
		assert!(sync_event_affects_read(
			&read(7),
			&event,
			&state,
			account,
			None
		));
		assert!(!sync_event_affects_read(
			&read(8),
			&event,
			&state,
			account,
			None
		));
	}
	#[test]
	fn targets_are_deduplicated_without_replacing_the_time_range() {
		let state = test_support::demo_state();
		let mut job = Job::new(&state, Scope::Account).unwrap();
		let cutoff = job.cutoff_ms;
		job.add_targets([
			Target {
				id: Id(5),
				kind: 0,
				parent: None,
			},
			Target {
				id: Id(5),
				kind: 0,
				parent: None,
			},
		]);
		assert_eq!(job.targets.iter().filter(|t| t.id == Id(5)).count(), 1);
		assert_eq!(job.end_ms - cutoff, HISTORY_MILLIS);
	}
	#[test]
	fn ordinary_member_sync_includes_known_people_without_recent_messages() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let mut person = test_support::message(7, channel).author;
		person.id = Id(987654);
		state.apply_force_sync_members(
			guild,
			&[model::Member {
				user: person,
				nick: Some("Known nickname".into()),
				roles: vec![],
				status: None,
				custom_status: None,
				activities: vec![],
				clients: Default::default(),
			}],
		);
		let mut job = Job::new(&state, Scope::Server(guild)).unwrap();
		job.add_known_members(&state, guild);
		job.add_known_members(&state, guild);
		assert_eq!(
			job.members.iter().filter(|id| **id == Id(987654)).count(),
			1
		);
	}
	#[test]
	fn checkpoint_rejects_scope_cursor_duplicates_and_invalid_archive_position() {
		let state = test_support::demo_state();
		let first = state.guilds[0].id;
		let mut job = Job::new(&state, Scope::Server(first)).unwrap();
		assert!(job.valid());
		job.scope = Some(Id(u64::MAX));
		assert!(!job.valid());
		job.scope = Some(first);
		job.guilds.push(first);
		assert!(!job.valid());
		job.guilds.pop();
		job.before = Some(Id(u64::MAX));
		assert!(!job.valid());
		job.before = None;
		job.archive_index = 1;
		assert!(!job.valid());
		job.archive_index = 0;
		job.archive_kind = 2;
		assert!(!job.valid());
		job.archive_kind = 0;
		job.dms.push(Id(9));
		assert!(!job.valid());
		job.dms.clear();
		assert!(job.valid());
	}
	#[test]
	fn unrelated_live_traffic_cannot_starve_a_history_or_expression_read() {
		let state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let own = state.user.as_ref().unwrap().id;
		let read = Read::History {
			channel,
			before: None,
			cutoff_ms: client_core::force_sync_read::DISCORD_EPOCH_MS,
			end_ms: client_core::force_sync_read::DISCORD_EPOCH_MS + HISTORY_MILLIS,
		};
		assert!(!sync_event_affects_read(
			&read,
			&Event::Message(test_support::message(5, Id(987654321))),
			&state,
			own,
			None
		));
		assert!(sync_event_affects_read(
			&read,
			&Event::Message(test_support::message(5, channel)),
			&state,
			own,
			None
		));
		let update = Event::MemberIdentity(Box::new(model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: model::Patch::Absent,
			guild,
			user: Id(987654321),
			profile: None,
			nick: model::Patch::Value("changed".into()),
			roles: model::Patch::Absent,
			departed: false,
		}));
		assert!(!sync_event_affects_read(
			&Read::Emojis { guild },
			&update,
			&state,
			own,
			None
		));
		assert!(!sync_event_affects_read(
			&Read::Guild { guild, user: own },
			&update,
			&state,
			own,
			None
		));
		assert!(sync_event_affects_read(
			&Read::Members {
				guild,
				users: vec![Id(987654321)]
			},
			&update,
			&state,
			own,
			None
		));
	}
	#[test]
	fn stale_reads_and_transient_retries_are_bounded_and_hidden_completed_jobs_do_not_poll() {
		let state = test_support::demo_state();
		let mut job = Job::new(&state, Scope::Account).unwrap();
		let read = Read::Emojis {
			guild: job.guild().unwrap(),
		};
		job.step = 3;
		let mut sync = Sync::default();
		sync.prepare_read(&read);
		for _ in 0..MAX_STALE_RETRIES {
			sync.reject_stale(&mut job, &read);
		}
		assert_eq!(job.step, 4);
		assert_eq!(job.outcomes[Job::stage_index(Stage::Expressions)], 2);
		let now = Instant::now();
		sync.read_retries = 0;
		assert!(sync.defer_read(Failure::RateLimited, now));
		assert_eq!(sync.next_read, Some(now + Duration::from_secs(60)));
		assert!(sync.defer_read(Failure::RateLimited, now));
		assert_eq!(sync.next_read, Some(now + Duration::from_secs(120)));
		assert!(!sync.defer_read(Failure::RateLimited, now));
		job.status = 3;
		sync.job = Some(job);
		assert!(!sync.should_measure_usage(false));
		assert!(sync.should_measure_usage(true));
	}
	#[test]
	fn approved_thread_targets_use_parent_access_without_overriding_known_denials() {
		let mut state = test_support::demo_state();
		let parent = state.selected.unwrap();
		let guild = state.channel(parent).unwrap().guild;
		assert!(state.can_read_history(parent));
		let target = Target {
			id: Id(7654321),
			kind: 11,
			parent: Some(parent),
		};
		assert!(target.readable(&state, guild));
		assert!(!target.readable(&state, Some(Id(u64::MAX))));
		let mut thread = state.channel(parent).unwrap().clone();
		thread.id = target.id;
		thread.parent_id = Some(parent);
		thread.kind = 11;
		state.channels.push(thread);
		// A known channel's own current permissions take precedence over parent fallback.
		assert_eq!(
			target.readable(&state, guild),
			state.can_read_history(target.id)
		);
	}
	#[test]
	fn lost_history_access_reports_unavailable_and_completes_its_progress_slot() {
		let state = test_support::demo_state();
		let channel = state.channel(state.selected.unwrap()).unwrap();
		let mut job = Job::new(&state, Scope::Server(channel.guild.unwrap())).unwrap();
		job.targets = vec![
			Target {
				id: Id(87654321),
				kind: 0,
				parent: None,
			},
			Target {
				id: Id(87654322),
				kind: 4,
				parent: None,
			},
			Target {
				id: channel.id,
				kind: channel.kind,
				parent: channel.parent_id,
			},
		];
		job.step = 8;
		job.advance();
		assert_eq!(job.channels_total, 2);
		job.skip_unreadable_targets(&state);
		assert_eq!(job.channels_done, 1);
		assert_eq!(job.channel_index, 2);
		assert_eq!(job.outcomes[Job::stage_index(Stage::History)], 3);
		assert_eq!(job.details[Job::stage_index(Stage::History)], 1);
		assert!(job.valid());
	}
	#[test]
	fn unloaded_archived_history_observes_thread_removal_and_its_guild_permissions() {
		let state = client_core::State::default();
		let guild = Id(10);
		let own = Id(2);
		let thread = Id(7654321);
		assert!(state.channel(thread).is_none());
		let read = Read::History {
			channel: thread,
			before: None,
			cutoff_ms: client_core::force_sync_read::DISCORD_EPOCH_MS,
			end_ms: client_core::force_sync_read::DISCORD_EPOCH_MS + HISTORY_MILLIS,
		};
		assert!(sync_event_affects_read(
			&read,
			&Event::ThreadRemoved { guild, id: thread },
			&state,
			own,
			Some(guild)
		));
		let permission = |guild| {
			Event::Permissions(client_core::permissions::Event::RoleRemoved {
				guild,
				id: Id(9000),
			})
		};
		assert!(sync_event_affects_read(
			&read,
			&permission(guild),
			&state,
			own,
			Some(guild)
		));
		assert!(!sync_event_affects_read(
			&read,
			&permission(Id(987654321)),
			&state,
			own,
			Some(guild)
		));
	}
}
