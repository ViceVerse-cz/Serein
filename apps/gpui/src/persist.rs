//! Local persistence for the experiment: appearance, theme, accent, the notification opt-in,
//! reading and channel-list choices, and per-account drafts and collapsed categories.
//!
//! The experiment keeps its own SQLite file beside, never inside, the main app's store, so it
//! cannot change the egui app's data. One worker thread owns the database behind bounded
//! queues; the UI only enqueues writes and drains results in `poll`. Drafts are written at
//! most once per second per channel, and on channel switch and quit. The offline preview
//! never starts the worker, so it never touches disk. Tokens are never stored here.
use crate::theme::{self, Appearance};
use gpui::{App, Context, Window};
use local_store::{AppPreferences, LocalStore, StoreError};
use model::{ChannelPreferences, Id, ReadingPreferences};
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
	pub reading: ReadingPreferences,
	pub show_hidden_channels: bool,
}
impl Default for Settings {
	fn default() -> Self {
		Self {
			appearance: Appearance::System,
			variant: Variant::Standard,
			accent: None,
			notifications: false,
			reading: ReadingPreferences::default(),
			show_hidden_channels: false,
		}
	}
}
impl Settings {
	fn current(view: &crate::Serein) -> Self {
		let mut reading = view.settings.reading;
		// The header's People button toggles the member list directly.
		reading.show_members = view.members_open;
		Self {
			appearance: theme::appearance(),
			variant: ui::design::variant(),
			accent: ui::design::primary_color(),
			notifications: view.settings.notifications,
			reading,
			show_hidden_channels: view.settings.show_hidden_channels,
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
	/// Deletes the account's stored drafts, keeping its other local data.
	ClearDrafts(Id),
	/// Written to the window file beside the store, not to SQLite.
	Window(WindowMemory),
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
	DraftsCleared(usize),
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
	window: WindowState,
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
			window: WindowState::default(),
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
	let window_file = path.with_file_name(WINDOW_FILE);
	while let Ok(command) = commands.recv() {
		let answer = match command {
			Command::Window(memory) => save_window(&window_file, &memory)
				.err()
				.map(|_| Reply::Problem("Could not save the window size.")),
			command => execute(&mut store, command),
		};
		if let Some(answer) = answer {
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
		notifications: preferences
			.as_ref()
			.is_some_and(|p| p.notifications_enabled),
		reading: store.reading_preferences().unwrap_or_default(),
		show_hidden_channels: preferences.is_some_and(|p| p.show_hidden_channels),
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
	preferences.show_hidden_channels = settings.show_hidden_channels;
	store.save_app_preferences(&preferences)?;
	store.save_reading_preferences(settings.reading)
}

fn execute(store: &mut LocalStore, command: Command) -> Option<Reply> {
	match command {
		Command::Settings(settings) => save_settings(store, settings)
			.err()
			.map(|_| Reply::Problem("Could not save your settings.")),
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
		Command::ClearDrafts(account) => Some(clear_drafts(store, account)),
		// `run` writes the window file; the database never holds it.
		Command::Window(_) => None,
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
					self.apply_reading(settings.reading);
					self.settings.show_hidden_channels = settings.show_hidden_channels;
					self.sync_channels();
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
				Reply::DraftsCleared(count) => {
					self.notify_user(match count {
						0 => "No saved drafts to clear.".to_owned(),
						1 => "Cleared 1 saved draft.".to_owned(),
						count => format!("Cleared {count} saved drafts."),
					});
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
		if flush {
			self.persist.flush_window();
		}
		if !self.persist.enabled() {
			return;
		}
		if let Some(saved) = self.persist.settings {
			let current = Settings::current(self);
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

/// Deletes every stored draft of `account`; its categories and the settings stay.
fn clear_drafts(store: &mut LocalStore, account: Id) -> Reply {
	const PROBLEM: &str = "Could not clear the saved drafts.";
	let Ok(drafts) = store.load_drafts(account) else {
		return Reply::Problem(PROBLEM);
	};
	for channel in drafts.keys() {
		if store.save_draft(account, *channel, "").is_err() {
			return Reply::Problem(PROBLEM);
		}
	}
	Reply::DraftsCleared(drafts.len())
}

// Window size memory. The bounds live in a small text file beside the store, written by the
// store worker; `main` reads it once before opening the window. The offline preview never
// reads or writes it.

/// File beside `store.sqlite3` in the experiment's own data directory.
const WINDOW_FILE: &str = "window.txt";
const WINDOW_HEADER: &str = "serein-gpui window 1";
/// Longest window file read at startup.
const MAX_WINDOW_FILE: u64 = 512;
/// Bounds are written once the window has rested this long, and when it closes.
const WINDOW_DEBOUNCE: Duration = Duration::from_secs(1);
/// The window's minimum size, as `main` passes it to GPUI.
pub const MIN_WINDOW: (f32, f32) = (760., 480.);
/// Coordinates beyond this are corrupt, not a real display arrangement.
const MAX_COORDINATE: f32 = 100_000.;

/// Whether to reopen at the last size, and where that was.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowMemory {
	pub remember: bool,
	pub placement: Option<Placement>,
}
impl Default for WindowMemory {
	fn default() -> Self {
		Self {
			remember: true,
			placement: None,
		}
	}
}

/// Restore bounds in GPUI's display-relative logical pixels, and the display's UUID.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
	pub display: Option<String>,
	pub bounds: gpui::Bounds<gpui::Pixels>,
	pub maximized: bool,
}

#[derive(Default)]
struct WindowState {
	memory: WindowMemory,
	/// Latest bounds not yet queued, and when they last changed.
	pending: Option<(Placement, Instant)>,
	/// A debounce task is waiting to write `pending`.
	timer: bool,
}

/// `serein-gpui/window.txt` in the platform data directory.
pub fn window_path(data_dir: &Path) -> PathBuf {
	store_path(data_dir).with_file_name(WINDOW_FILE)
}

/// The remembered window, read once at startup; defaults when absent or unreadable.
pub fn load_window(path: &Path) -> WindowMemory {
	use std::io::Read;
	let mut text = String::new();
	let read = std::fs::File::open(path)
		.and_then(|file| file.take(MAX_WINDOW_FILE).read_to_string(&mut text));
	match read {
		Ok(_) => parse_window(&text),
		Err(_) => WindowMemory::default(),
	}
}

fn valid_display(id: &str) -> bool {
	(1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn parse_window(text: &str) -> WindowMemory {
	let mut lines = text.lines();
	if lines.next() != Some(WINDOW_HEADER) {
		return WindowMemory::default();
	}
	let mut memory = WindowMemory::default();
	let mut display = None;
	let mut maximized = false;
	let mut bounds = None;
	for line in lines {
		let mut words = line.split_ascii_whitespace();
		match (words.next(), words.next()) {
			(Some("remember"), Some(value)) => memory.remember = value != "0",
			(Some("display"), Some(id)) if valid_display(id) => display = Some(id.to_owned()),
			(Some("maximized"), Some(value)) => maximized = value == "1",
			(Some("bounds"), Some(x)) => {
				let values = std::iter::once(x)
					.chain(words.by_ref())
					.map(str::parse::<f32>)
					.collect::<Result<Vec<_>, _>>();
				if let Ok(&[x, y, width, height]) = values.as_deref()
					&& [x, y, width, height]
						.iter()
						.all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
					&& width >= 1. && height >= 1.
				{
					bounds = Some(gpui::Bounds::new(
						gpui::point(gpui::px(x), gpui::px(y)),
						gpui::size(gpui::px(width), gpui::px(height)),
					));
				}
			}
			_ => {}
		}
	}
	memory.placement = bounds.map(|bounds| Placement {
		display,
		bounds,
		maximized,
	});
	memory
}

fn format_window(memory: &WindowMemory) -> String {
	let mut text = format!("{WINDOW_HEADER}\nremember {}\n", u8::from(memory.remember));
	if let Some(placement) = &memory.placement {
		let b = placement.bounds;
		if let Some(display) = placement.display.as_deref().filter(|id| valid_display(id)) {
			text.push_str(&format!("display {display}\n"));
		}
		text.push_str(&format!(
			"maximized {}\nbounds {} {} {} {}\n",
			u8::from(placement.maximized),
			f32::from(b.origin.x).round(),
			f32::from(b.origin.y).round(),
			f32::from(b.size.width).round(),
			f32::from(b.size.height).round(),
		));
	}
	text
}

/// Replaces the file atomically so a crash mid-write keeps the previous bounds.
fn save_window(path: &Path, memory: &WindowMemory) -> std::io::Result<()> {
	let staging = path.with_extension("txt.tmp");
	std::fs::write(&staging, format_window(memory))?;
	std::fs::rename(&staging, path)
}

/// Where to reopen: on the remembered display when it is still connected, otherwise centred
/// on the first display (the primary). The size keeps the window minimum and fits the
/// display's visible area; the window is moved fully onto it. `None` without displays.
pub fn place_window(
	saved: &Placement,
	displays: &[(Option<String>, gpui::Bounds<gpui::Pixels>)],
) -> Option<(usize, gpui::Bounds<gpui::Pixels>)> {
	use gpui::{Bounds, point, px, size};
	let found = saved.display.as_ref().and_then(|id| {
		displays
			.iter()
			.position(|(display, _)| display.as_ref() == Some(id))
	});
	let index = found.or((!displays.is_empty()).then_some(0))?;
	let area = displays[index].1;
	let fit = |value: f32, minimum: f32, room: f32| value.max(minimum).min(room.max(minimum));
	let width = fit(
		f32::from(saved.bounds.size.width),
		MIN_WINDOW.0,
		f32::from(area.size.width),
	);
	let height = fit(
		f32::from(saved.bounds.size.height),
		MIN_WINDOW.1,
		f32::from(area.size.height),
	);
	let (left, top) = (f32::from(area.origin.x), f32::from(area.origin.y));
	let (right, bottom) = (
		left + f32::from(area.size.width),
		top + f32::from(area.size.height),
	);
	let (x, y) = if found.is_some() {
		(
			f32::from(saved.bounds.origin.x),
			f32::from(saved.bounds.origin.y),
		)
	} else {
		(
			left + (f32::from(area.size.width) - width) / 2.,
			top + (f32::from(area.size.height) - height) / 2.,
		)
	};
	let x = x.min(right - width).max(left);
	let y = y.min(bottom - height).max(top);
	Some((
		index,
		Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
	))
}

impl Persist {
	/// Queues the latest window bounds, if any changed since the last write.
	fn flush_window(&mut self) {
		let Some((placement, changed)) = self.window.pending.take() else {
			return;
		};
		if !self.enabled() || !self.window.memory.remember {
			return;
		}
		let memory = WindowMemory {
			remember: true,
			placement: Some(placement),
		};
		if self.send(Command::Window(memory.clone())) {
			self.window.memory = memory;
		} else if self.enabled() {
			// A full queue: keep it for the next change or the close.
			self.window.pending = memory.placement.map(|placement| (placement, changed));
		}
	}

	/// Whether "Remember window size" is on.
	pub fn remember_window(&self) -> bool {
		self.window.memory.remember
	}

	/// Whether the window size can be saved at all (never in the offline preview).
	pub fn saves_window(&self) -> bool {
		self.enabled()
	}
}

fn placement(window: &Window, cx: &App) -> Placement {
	let (bounds, maximized) = match window.window_bounds() {
		gpui::WindowBounds::Maximized(bounds) => (bounds, true),
		other => (other.get_bounds(), false),
	};
	Placement {
		display: window
			.display(cx)
			.and_then(|display| display.uuid().ok())
			.map(|uuid| uuid.to_string()),
		bounds,
		maximized,
	}
}

impl crate::Serein {
	/// The window file as `main` read it at startup.
	pub(crate) fn restore_window_memory(&mut self, memory: WindowMemory) {
		self.persist.window.memory = memory;
	}

	/// Records moved or resized bounds and writes them once the window rests.
	pub(crate) fn window_bounds_changed(&mut self, window: &Window, cx: &mut Context<Self>) {
		if !self.persist.enabled() || !self.persist.window.memory.remember {
			return;
		}
		let current = placement(window, cx);
		if self.persist.window.pending.is_none()
			&& self.persist.window.memory.placement.as_ref() == Some(&current)
		{
			return;
		}
		self.persist.window.pending = Some((current, Instant::now()));
		if self.persist.window.timer {
			return;
		}
		self.persist.window.timer = true;
		cx.spawn(async move |this, cx| {
			loop {
				cx.background_executor().timer(WINDOW_DEBOUNCE).await;
				let rested = this.update(cx, |this, _| {
					let state = &mut this.persist.window;
					let rested = state
						.pending
						.as_ref()
						.is_none_or(|(_, at)| at.elapsed() >= WINDOW_DEBOUNCE);
					if rested {
						state.timer = false;
						this.persist.flush_window();
					}
					rested
				});
				if !matches!(rested, Ok(false)) {
					break;
				}
			}
		})
		.detach();
	}

	/// Turns "Remember window size" on (saving the current bounds) or off (forgetting them).
	pub(crate) fn set_remember_window(&mut self, on: bool, window: &Window, cx: &App) {
		let state = &mut self.persist.window;
		state.memory.remember = on;
		state.pending = None;
		if !self.persist.enabled() {
			return;
		}
		if on {
			self.persist.window.pending = Some((placement(window, cx), Instant::now()));
			self.persist.flush_window();
		} else {
			let memory = WindowMemory {
				remember: false,
				placement: None,
			};
			if self.persist.send(Command::Window(memory.clone())) {
				self.persist.window.memory = memory;
			} else {
				self.notify_user("Could not save the window setting; try again.");
			}
		}
	}

	/// Whether this account's drafts can be cleared from the store now.
	pub(crate) fn can_clear_drafts(&self) -> bool {
		self.persist.enabled() && self.persist.account.is_some() && self.persist.loaded
	}

	/// Clears the signed-in account's drafts, in memory and in the experiment's store.
	pub(crate) fn clear_saved_drafts(&mut self, cx: &mut Context<Self>) {
		let Some(account) = self.persist.account.filter(|_| self.can_clear_drafts()) else {
			return;
		};
		if !self.persist.send(Command::ClearDrafts(account)) {
			self.notify_user("Could not clear the saved drafts; try again.");
			return;
		}
		self.state.drafts.clear();
		self.persist.drafts.clear();
		self.composer
			.update(cx, |input, cx| input.set_value(String::new(), cx));
	}
}

#[cfg(test)]
mod tests {
	use super::{Command, Debounce, Reply, Settings, execute, load_settings, open, store_path};
	use crate::theme::Appearance;
	use model::{Id, ReadingPreferences};
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
			reading: ReadingPreferences {
				sidebar_width: 300,
				show_members: false,
				hide_media_links: false,
				confirm_external_links: false,
				..ReadingPreferences::default()
			},
			show_hidden_channels: true,
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

#[cfg(test)]
mod window_tests {
	use super::{
		Command, MIN_WINDOW, Placement, Reply, WindowMemory, execute, format_window, load_window,
		open, parse_window, place_window, save_window, store_path, window_path,
	};
	use gpui::{Bounds, Pixels, point, px, size};
	use model::Id;

	fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
		Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
	}

	fn temp_root(name: &str) -> std::path::PathBuf {
		std::env::temp_dir().join(format!(
			"serein-gpui-{name}-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		))
	}

	const MAIN: &str = "37D8832A-2D66-02CA-B9F7-8F30A301B230";
	const SIDE: &str = "1C9A-44";

	#[test]
	fn saved_bounds_stay_on_a_connected_display_and_above_the_minimum() {
		let displays = [
			(Some(MAIN.to_owned()), rect(0., 25., 1512., 920.)),
			(Some(SIDE.to_owned()), rect(0., 0., 1920., 1080.)),
		];
		let saved = |display: Option<&str>, bounds| Placement {
			display: display.map(str::to_owned),
			bounds,
			maximized: false,
		};
		// Fits already: unchanged, on its own display.
		let fits = saved(Some(SIDE), rect(100., 80., 1200., 800.));
		assert_eq!(
			place_window(&fits, &displays),
			Some((1, rect(100., 80., 1200., 800.)))
		);
		// Too small and partly off-screen: grown to the minimum and pulled back in.
		let (index, bounds) =
			place_window(&saved(Some(MAIN), rect(1400., -50., 300., 200.)), &displays).unwrap();
		assert_eq!(index, 0);
		assert_eq!(bounds.size, size(px(MIN_WINDOW.0), px(MIN_WINDOW.1)));
		assert_eq!(bounds.origin, point(px(1512. - MIN_WINDOW.0), px(25.)));
		// Larger than the display: shrunk to its visible area.
		let (_, bounds) =
			place_window(&saved(Some(MAIN), rect(0., 0., 4000., 3000.)), &displays).unwrap();
		assert_eq!(bounds, rect(0., 25., 1512., 920.));
		// A disconnected display: centred on the primary with the saved size.
		let (index, bounds) = place_window(
			&saved(Some("FFFF"), rect(3000., 200., 1000., 700.)),
			&displays,
		)
		.unwrap();
		assert_eq!(index, 0);
		assert_eq!(bounds, rect(256., 135., 1000., 700.));
		// A display smaller than the minimum still gets the minimum size.
		let tiny = [(None, rect(0., 0., 640., 400.))];
		let (_, bounds) = place_window(&saved(None, rect(0., 0., 900., 600.)), &tiny).unwrap();
		assert_eq!(bounds, rect(0., 0., MIN_WINDOW.0, MIN_WINDOW.1));
		assert_eq!(place_window(&fits, &[]), None);
	}

	#[test]
	fn window_memory_round_trips_and_rejects_corrupt_files() {
		let memory = WindowMemory {
			remember: true,
			placement: Some(Placement {
				display: Some(MAIN.to_owned()),
				bounds: rect(40., 60., 1180., 780.),
				maximized: true,
			}),
		};
		assert_eq!(parse_window(&format_window(&memory)), memory);
		let off = WindowMemory {
			remember: false,
			placement: None,
		};
		assert_eq!(parse_window(&format_window(&off)), off);
		for corrupt in [
			"",
			"something else\nremember 0\n",
			"serein-gpui window 1\nbounds 1 2 NaN 4\n",
			"serein-gpui window 1\nbounds 1 2 3\n",
			"serein-gpui window 1\nbounds 0 0 0 480\n",
			"serein-gpui window 1\nbounds 0 0 1e9 480\n",
		] {
			assert_eq!(
				parse_window(corrupt),
				WindowMemory::default(),
				"{corrupt:?}"
			);
		}
		// A display id with odd characters is dropped, not trusted.
		let parsed = parse_window("serein-gpui window 1\ndisplay ../x\nbounds 0 0 800 600\n");
		assert_eq!(parsed.placement.unwrap().display, None);
	}

	#[test]
	fn window_file_survives_a_restart_beside_the_store() {
		let root = temp_root("window");
		let path = window_path(&root);
		assert_eq!(path.parent(), store_path(&root).parent());
		// Nothing saved yet: remember is on, with no bounds.
		assert_eq!(load_window(&path), WindowMemory::default());
		let (_store, _) = open(&store_path(&root)).unwrap();
		let memory = WindowMemory {
			remember: true,
			placement: Some(Placement {
				display: None,
				bounds: rect(12., 34., 1000., 700.),
				maximized: false,
			}),
		};
		save_window(&path, &memory).unwrap();
		assert_eq!(load_window(&path), memory);
		let off = WindowMemory {
			remember: false,
			placement: None,
		};
		save_window(&path, &off).unwrap();
		assert_eq!(load_window(&path), off);
		std::fs::remove_dir_all(&root).unwrap();
	}

	#[test]
	fn clearing_drafts_keeps_other_accounts_and_categories() {
		let root = temp_root("clear-drafts");
		let (mut store, _) = open(&store_path(&root)).unwrap();
		let account = Id(7);
		for command in [
			Command::Draft {
				account,
				channel: Id(21),
				content: "one".into(),
			},
			Command::Draft {
				account,
				channel: Id(22),
				content: "two".into(),
			},
			Command::Draft {
				account: Id(8),
				channel: Id(21),
				content: "other account".into(),
			},
			Command::Collapsed {
				account,
				categories: vec![Id(30)],
			},
		] {
			assert!(execute(&mut store, command).is_none());
		}
		assert!(matches!(
			execute(&mut store, Command::ClearDrafts(account)),
			Some(Reply::DraftsCleared(2))
		));
		let Some(Reply::Account {
			drafts, collapsed, ..
		}) = execute(&mut store, Command::Load(account))
		else {
			panic!("account data did not load");
		};
		assert!(drafts.is_empty());
		assert_eq!(collapsed, vec![Id(30)]);
		let Some(Reply::Account { drafts, .. }) = execute(&mut store, Command::Load(Id(8))) else {
			panic!("account data did not load");
		};
		assert_eq!(drafts.len(), 1);
		drop(store);
		std::fs::remove_dir_all(&root).unwrap();
	}
}
