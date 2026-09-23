//! Local persistence for the experiment: appearance, theme, accent, the notification opt-in,
//! and per-account drafts and collapsed categories.
//!
//! The experiment keeps its own SQLite file beside, never inside, the main app's store, so it
//! cannot change the egui app's data. One worker thread owns the database behind bounded
//! queues; the UI only enqueues writes and drains results in `poll`. Drafts are written at
//! most once per second per channel, and on channel switch and quit. The offline preview
//! never starts the worker, so it never touches disk. Tokens are never stored here.
use crate::theme::{self, Appearance};
use gpui::{App, Context, Window};
use local_store::{AppPreferences, LocalStore, StoreError};
use model::{ChannelPreferences, Id};
use std::{
	collections::{BTreeMap, BTreeSet, HashMap},
	hash::{DefaultHasher, Hash, Hasher},
	path::{Path, PathBuf},
	sync::mpsc,
	time::{Duration, Instant},
};
use ui::design::Variant;

const COMMANDS: usize = 16;
const REPLIES: usize = 16;
/// Drafts are written at most this often per channel, plus on channel switch and quit.
const DRAFT_INTERVAL: Duration = Duration::from_secs(1);
/// Longest wait at exit for queued writes; GPUI itself stops waiting on quit after 200 ms.
pub const EXIT_WAIT: Duration = Duration::from_millis(500);
/// Per-draft limit of the shared store schema.
const MAX_STORED_DRAFT: usize = 8192;
const UNAVAILABLE: &str = "Local settings are unavailable; changes last until you quit.";

/// Device-wide choices restored at startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
	pub appearance: Appearance,
	pub variant: Variant,
	pub accent: Option<[u8; 3]>,
	pub notifications: bool,
}
impl Default for Settings {
	fn default() -> Self {
		Self {
			appearance: Appearance::System,
			variant: Variant::Standard,
			accent: None,
			notifications: false,
		}
	}
}
impl Settings {
	fn current(notifications: bool) -> Self {
		Self {
			appearance: theme::appearance(),
			variant: ui::design::variant(),
			accent: ui::design::primary_color(),
			notifications,
		}
	}
	fn apply(self) {
		theme::set_appearance(self.appearance);
		ui::design::set_variant(self.variant);
		ui::design::set_primary_color(self.accent);
	}
}

/// The experiment's own database: `serein-gpui/store.sqlite3` in the platform data directory,
/// a sibling of the main app's `serein/client.sqlite3`.
pub fn store_path(data_dir: &Path) -> PathBuf {
	data_dir.join("serein-gpui").join("store.sqlite3")
}

/// At most one draft write per second for the same channel; another channel is ready at once.
#[derive(Default)]
pub struct Debounce(Option<(Id, Instant)>);
impl Debounce {
	pub fn ready(&mut self, channel: Id, now: Instant) -> bool {
		if let Some((last, at)) = self.0
			&& last == channel
			&& now.saturating_duration_since(at) < DRAFT_INTERVAL
		{
			return false;
		}
		self.0 = Some((channel, now));
		true
	}
}

enum Command {
	Settings(Settings),
	Load(Id),
	Draft {
		account: Id,
		channel: Id,
		content: String,
	},
	Collapsed {
		account: Id,
		categories: Vec<Id>,
	},
	Forget(Id),
}

enum Reply {
	Settings(Settings),
	Account {
		account: Id,
		drafts: BTreeMap<Id, String>,
		collapsed: Vec<Id>,
		problem: Option<&'static str>,
	},
	Problem(&'static str),
}

struct Link {
	commands: mpsc::SyncSender<Command>,
	replies: mpsc::Receiver<Reply>,
	/// Disconnects when the worker exits.
	done: mpsc::Receiver<()>,
}

pub struct Persist {
	link: Option<Link>,
	/// Settings last loaded or queued; `None` until the startup load arrives.
	settings: Option<Settings>,
	/// Account whose drafts and categories were requested, and whether they arrived.
	account: Option<Id>,
	loaded: bool,
	/// Collapsed categories last loaded or queued for `account`.
	collapsed: BTreeSet<Id>,
	/// Hash of each channel's stored draft, so unchanged text is not rewritten.
	drafts: HashMap<Id, u64>,
	debounce: Debounce,
	notice: Option<&'static str>,
	long_draft_noticed: bool,
}

impl Persist {
	/// Starts the worker; the offline preview gets an inert, in-memory instance.
	pub fn start(demo: bool) -> Self {
		let mut this = Self {
			link: None,
			settings: None,
			account: None,
			loaded: false,
			collapsed: BTreeSet::new(),
			drafts: HashMap::new(),
			debounce: Debounce::default(),
			notice: None,
			long_draft_noticed: false,
		};
		if demo {
			return this;
		}
		let Some(path) = dirs::data_local_dir().map(|dir| store_path(&dir)) else {
			this.notice = Some(UNAVAILABLE);
			return this;
		};
		let (commands, receive) = mpsc::sync_channel(COMMANDS);
		let (send, replies) = mpsc::sync_channel(REPLIES);
		let (finished, done) = mpsc::channel();
		let spawned = std::thread::Builder::new()
			.name("serein-gpui-store".into())
			.spawn(move || run(&path, receive, send, finished));
		match spawned {
			Ok(_) => {
				this.link = Some(Link {
					commands,
					replies,
					done,
				})
			}
			Err(_) => this.notice = Some(UNAVAILABLE),
		}
		this
	}

	fn enabled(&self) -> bool {
		self.link.is_some()
	}

	/// Queues without blocking; a full queue is retried by the next poll.
	fn send(&mut self, command: Command) -> bool {
		let Some(link) = &self.link else {
			return false;
		};
		match link.commands.try_send(command) {
			Ok(()) => true,
			Err(mpsc::TrySendError::Full(_)) => false,
			Err(mpsc::TrySendError::Disconnected(_)) => {
				self.link = None;
				false
			}
		}
	}

	fn replies(&mut self) -> Vec<Reply> {
		let Some(link) = &self.link else {
			return Vec::new();
		};
		let mut replies = Vec::new();
		loop {
			match link.replies.try_recv() {
				Ok(reply) => replies.push(reply),
				Err(mpsc::TryRecvError::Empty) => break,
				Err(mpsc::TryRecvError::Disconnected) => {
					self.link = None;
					break;
				}
			}
			if replies.len() == REPLIES {
				crate::backend::WAKE.notify_one();
				break;
			}
		}
		replies
	}

	/// Requests the signed-in account's data; `false` while the queue is full.
	fn switch_account(&mut self, account: Option<Id>) -> bool {
		if let Some(id) = account
			&& !self.send(Command::Load(id))
		{
			return false;
		}
		self.account = account;
		self.loaded = false;
		self.collapsed.clear();
		self.drafts.clear();
		true
	}

	fn draft(&mut self, account: Id, channel: Id, value: &str) {
		let hash = (!value.is_empty()).then(|| hash(value));
		if self.drafts.get(&channel).copied() == hash {
			return;
		}
		if value.len() > MAX_STORED_DRAFT {
			if !self.long_draft_noticed {
				self.long_draft_noticed = true;
				self.notice = Some("This draft is too long to keep after you quit.");
			}
			return;
		}
		let content = value.to_owned();
		if self.send(Command::Draft {
			account,
			channel,
			content,
		}) {
			match hash {
				Some(hash) => self.drafts.insert(channel, hash),
				None => self.drafts.remove(&channel),
			};
		}
	}

	/// Removes the account's local drafts and categories, as logging out does in the main app.
	pub fn forget(&mut self, account: Id) {
		if self.enabled() && !self.send(Command::Forget(account)) {
			self.notice = Some("Could not remove this account's local drafts.");
		}
		// Nothing more is written for it; the next poll sees the signed-out state.
		self.loaded = false;
	}

	/// Stops accepting work; the worker exits after the queued writes, closing the receiver.
	pub fn finish(&mut self) -> Option<mpsc::Receiver<()>> {
		let Link {
			commands,
			replies,
			done,
		} = self.link.take()?;
		drop((commands, replies));
		Some(done)
	}
}

impl Drop for Persist {
	/// Lets queued writes finish, bounded so closing the window never hangs.
	fn drop(&mut self) {
		if let Some(done) = self.finish() {
			let _ = done.recv_timeout(EXIT_WAIT);
		}
	}
}

fn hash(value: &str) -> u64 {
	let mut hasher = DefaultHasher::new();
	value.hash(&mut hasher);
	hasher.finish()
}

fn run(
	path: &Path,
	commands: mpsc::Receiver<Command>,
	replies: mpsc::SyncSender<Reply>,
	_finished: mpsc::Sender<()>,
) {
	let reply = |reply| {
		// A dropped receiver means the app is exiting; queued writes still run.
		let _ = replies.send(reply);
		crate::backend::WAKE.notify_one();
	};
	let mut store = match open(path) {
		Ok((store, settings)) => {
			reply(Reply::Settings(settings));
			store
		}
		Err(problem) => return reply(Reply::Problem(problem)),
	};
	while let Ok(command) = commands.recv() {
		if let Some(answer) = execute(&mut store, command) {
			reply(answer);
		}
	}
}

fn open(path: &Path) -> Result<(LocalStore, Settings), &'static str> {
	let dir = path.parent().ok_or(UNAVAILABLE)?;
	std::fs::create_dir_all(dir).map_err(|_| UNAVAILABLE)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
			.map_err(|_| UNAVAILABLE)?;
	}
	let fresh = !path.exists();
	let store = LocalStore::open(path).map_err(|_| UNAVAILABLE)?;
	let mut settings = load_settings(&store);
	if fresh {
		// The store's default enables notifications; this frontend stays off until opted in.
		settings.notifications = false;
		let _ = save_settings(&store, settings);
	}
	Ok((store, settings))
}

fn load_settings(store: &LocalStore) -> Settings {
	let preferences = store.app_preferences().ok();
	Settings {
		appearance: match store.appearance() {
			Ok(local_store::Appearance::Light) => Appearance::Light,
			Ok(local_store::Appearance::Dark) => Appearance::Dark,
			_ => Appearance::System,
		},
		variant: store
			.theme_variant()
			.ok()
			.flatten()
			.as_deref()
			.and_then(Variant::from_key)
			.unwrap_or(Variant::Standard),
		accent: preferences.as_ref().and_then(|p| p.primary_color),
		notifications: preferences.is_some_and(|p| p.notifications_enabled),
	}
}

fn save_settings(store: &LocalStore, settings: Settings) -> Result<(), StoreError> {
	store.save_appearance(match settings.appearance {
		Appearance::Light => local_store::Appearance::Light,
		Appearance::Dark => local_store::Appearance::Dark,
		Appearance::System => local_store::Appearance::System,
	})?;
	store.save_theme_variant(
		(settings.variant != Variant::Standard).then(|| settings.variant.key()),
	)?;
	let mut preferences = store.app_preferences().unwrap_or_else(|_| AppPreferences {
		notifications_enabled: false,
		..AppPreferences::default()
	});
	preferences.primary_color = settings.accent;
	preferences.notifications_enabled = settings.notifications;
	store.save_app_preferences(&preferences)
}

fn execute(store: &mut LocalStore, command: Command) -> Option<Reply> {
	match command {
		Command::Settings(settings) => save_settings(store, settings)
			.err()
			.map(|_| Reply::Problem("Could not save appearance settings.")),
		Command::Load(account) => {
			let drafts = store.load_drafts(account);
			let preferences = store.channel_preferences(account);
			let problem = (drafts.is_err() || preferences.is_err())
				.then_some("Some saved drafts or categories could not be read.");
			Some(Reply::Account {
				account,
				drafts: drafts.unwrap_or_default(),
				collapsed: preferences
					.map(|p| p.collapsed_categories)
					.unwrap_or_default(),
				problem,
			})
		}
		Command::Draft {
			account,
			channel,
			content,
		} => store
			.save_draft(account, channel, &content)
			.err()
			.map(|error| {
				Reply::Problem(if error == StoreError::Capacity {
					"Too many saved drafts; this one lasts until you quit."
				} else {
					"Could not save the draft; it lasts until you quit."
				})
			}),
		Command::Collapsed {
			account,
			mut categories,
		} => {
			let mut preferences = store.channel_preferences(account).unwrap_or_default();
			let room = ChannelPreferences::MAX_ENTRIES
				.saturating_sub(preferences.favorites.len() + preferences.pinned.len());
			categories.retain(|id| id.0 != 0);
			categories.truncate(room);
			categories.shrink_to_fit();
			preferences.collapsed_categories = categories;
			store
				.save_channel_preferences(account, &preferences)
				.err()
				.map(|_| Reply::Problem("Could not save collapsed categories."))
		}
		Command::Forget(account) => store
			.forget_account(account)
			.err()
			.map(|_| Reply::Problem("Could not remove this account's local drafts.")),
	}
}

impl crate::Serein {
	/// Applies stored values as they arrive, then queues changed settings, categories and the
	/// open draft. Returns whether anything visible changed.
	pub(crate) fn poll_persist(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
		let mut changed = false;
		for reply in self.persist.replies() {
			match reply {
				Reply::Settings(settings) => {
					settings.apply();
					self.settings.notifications = settings.notifications;
					self.alerts.set_enabled(settings.notifications);
					self.persist.settings = Some(settings);
					window.refresh();
					changed = true;
				}
				Reply::Account {
					account,
					drafts,
					collapsed,
					problem,
				} => {
					if self.persist.account == Some(account) && !self.persist.loaded {
						self.restore_account(account, drafts, collapsed, cx);
						changed = true;
					}
					if let Some(problem) = problem {
						self.notify_user(problem);
						changed = true;
					}
				}
				Reply::Problem(problem) => {
					self.notify_user(problem);
					changed = true;
				}
			}
		}
		if let Some(notice) = self.persist.notice.take() {
			self.notify_user(notice);
			changed = true;
		}
		if !self.persist.enabled() {
			return changed;
		}
		let account = self.state.user.as_ref().map(|user| user.id);
		if account != self.persist.account {
			let previous = self.persist.account.is_some();
			if self.persist.switch_account(account) && previous && !self.collapsed.is_empty() {
				// Collapsed categories belong to the previous account.
				self.collapsed.clear();
				self.sync_channels();
				changed = true;
			}
		}
		self.queue_persist(false, cx);
		changed
	}

	/// Queues whatever differs from the store; `flush` skips the draft debounce (switch, quit).
	pub(crate) fn queue_persist(&mut self, flush: bool, cx: &App) {
		if !self.persist.enabled() {
			return;
		}
		if let Some(saved) = self.persist.settings {
			let current = Settings::current(self.settings.notifications);
			if current != saved && self.persist.send(Command::Settings(current)) {
				self.persist.settings = Some(current);
			}
		}
		let (Some(account), true) = (self.persist.account, self.persist.loaded) else {
			return;
		};
		if self.collapsed != self.persist.collapsed {
			let categories = self
				.collapsed
				.iter()
				.copied()
				.take(ChannelPreferences::MAX_ENTRIES)
				.collect();
			if self.persist.send(Command::Collapsed {
				account,
				categories,
			}) {
				self.persist.collapsed = self.collapsed.clone();
			}
		}
		if let Some(channel) = self.state.selected
			&& (flush || self.persist.debounce.ready(channel, Instant::now()))
		{
			let value = self.composer.read(cx).value();
			self.persist.draft(account, channel, value);
		}
	}

	fn restore_account(
		&mut self,
		account: Id,
		drafts: BTreeMap<Id, String>,
		collapsed: Vec<Id>,
		cx: &mut Context<Self>,
	) {
		self.persist.loaded = true;
		for (channel, content) in drafts {
			self.persist.drafts.insert(channel, hash(&content));
			if self.state.drafts.contains_key(&channel)
				|| self.state.draft_bytes() + content.len() > client_core::MAX_DRAFT_BYTES
			{
				continue;
			}
			if self.state.selected == Some(channel) {
				if !self.composer.read(cx).value().is_empty() {
					continue;
				}
				let value = content.clone();
				self.composer
					.update(cx, |input, cx| input.set_value(value, cx));
			}
			self.state.drafts.insert(channel, content);
		}
		// Drafts kept for other channels before the store answered.
		let pending = self
			.state
			.drafts
			.iter()
			.filter(|(channel, _)| Some(**channel) != self.state.selected)
			.map(|(channel, content)| (*channel, content.clone()))
			.collect::<Vec<_>>();
		for (channel, content) in pending {
			self.persist.draft(account, channel, &content);
		}
		self.persist.collapsed = collapsed.iter().copied().collect();
		self.collapsed.extend(collapsed);
		self.sync_channels();
	}
}

#[cfg(test)]
mod tests {
	use super::{Command, Debounce, Reply, Settings, execute, load_settings, open, store_path};
	use crate::theme::Appearance;
	use model::Id;
	use std::{
		path::Path,
		time::{Duration, Instant},
	};
	use ui::design::Variant;

	#[test]
	fn drafts_are_written_at_most_once_per_second_per_channel() {
		let mut debounce = Debounce::default();
		let start = Instant::now();
		assert!(debounce.ready(Id(1), start));
		assert!(!debounce.ready(Id(1), start + Duration::from_millis(500)));
		// Switching channels is never delayed by the previous channel's write.
		assert!(debounce.ready(Id(2), start + Duration::from_millis(600)));
		assert!(!debounce.ready(Id(2), start + Duration::from_millis(900)));
		assert!(debounce.ready(Id(2), start + Duration::from_millis(1600)));
	}

	#[test]
	fn the_store_is_a_sibling_of_the_main_app_database() {
		let data = Path::new("/data");
		let path = store_path(data);
		let main = data.join("serein").join("client.sqlite3");
		assert_ne!(path, main);
		assert!(!path.starts_with(data.join("serein")));
		assert_eq!(path.parent(), Some(data.join("serein-gpui").as_path()));
	}

	#[test]
	fn settings_drafts_and_categories_survive_a_reopen() {
		let root = std::env::temp_dir().join(format!(
			"serein-gpui-persist-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		let path = store_path(&root);
		let (mut store, first) = open(&path).unwrap();
		// A new store never opts in to notifications on its own.
		assert_eq!(first, Settings::default());
		let chosen = Settings {
			appearance: Appearance::Light,
			variant: Variant::Eclipse,
			accent: Some([0x8b, 0x5c, 0xf6]),
			notifications: true,
		};
		let account = Id(7);
		for command in [
			Command::Settings(chosen),
			Command::Draft {
				account,
				channel: Id(21),
				content: "half-written".into(),
			},
			Command::Draft {
				account: Id(8),
				channel: Id(21),
				content: "other account".into(),
			},
			Command::Collapsed {
				account,
				categories: vec![Id(30), Id(31)],
			},
		] {
			assert!(execute(&mut store, command).is_none());
		}
		drop(store);
		let (mut store, reopened) = open(&path).unwrap();
		assert_eq!(reopened, chosen);
		assert_eq!(load_settings(&store), chosen);
		let Some(Reply::Account {
			account: loaded,
			drafts,
			collapsed,
			problem: None,
		}) = execute(&mut store, Command::Load(account))
		else {
			panic!("account data did not load");
		};
		assert_eq!(loaded, account);
		assert_eq!(drafts.len(), 1);
		assert_eq!(drafts[&Id(21)], "half-written");
		assert_eq!(collapsed, vec![Id(30), Id(31)]);
		// Clearing a draft deletes it; forgetting removes only that account.
		assert!(
			execute(
				&mut store,
				Command::Draft {
					account,
					channel: Id(21),
					content: String::new(),
				}
			)
			.is_none()
		);
		assert!(execute(&mut store, Command::Forget(account)).is_none());
		let Some(Reply::Account {
			drafts, collapsed, ..
		}) = execute(&mut store, Command::Load(account))
		else {
			panic!("account data did not load");
		};
		assert!(drafts.is_empty() && collapsed.is_empty());
		let Some(Reply::Account { drafts, .. }) = execute(&mut store, Command::Load(Id(8))) else {
			panic!("account data did not load");
		};
		assert_eq!(drafts[&Id(21)], "other account");
		drop(store);
		std::fs::remove_dir_all(&root).unwrap();
	}
}
