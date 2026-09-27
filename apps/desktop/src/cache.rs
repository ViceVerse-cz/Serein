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
const QUEUE_BYTES: usize = 16 * 1024 * 1024;
const WINDOW_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct Budget {
	used: Mutex<usize>,
	available: Condvar,
}
pub struct Reservation {
	budget: Arc<Budget>,
	bytes: usize,
}
impl Budget {
	fn reserve(self: &Arc<Self>, bytes: usize, wait: bool) -> Option<Reservation> {
		if bytes > QUEUE_BYTES {
			return None;
		}
		let mut used = self.used.lock().unwrap_or_else(|e| e.into_inner());
		while *used + bytes > QUEUE_BYTES {
			if !wait {
				return None;
			}
			used = self.available.wait(used).unwrap_or_else(|e| e.into_inner());
		}
		*used += bytes;
		Some(Reservation {
			budget: self.clone(),
			bytes,
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
	LoadAppPreferences,
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
	SaveDraft {
		channel: Id,
		content: String,
	},
	SaveChannel {
		channel: Id,
		messages: Vec<Message>,
	},
	SaveChanges {
		channel: Id,
		messages: Vec<Message>,
		retained: Vec<Id>,
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
	CustomFont(Result<Option<ui::fonts::CustomFont>, &'static str>),
	AppPreferences(Result<Box<local_store::AppPreferences>, StoreError>),
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
		epoch: u64,
	},
	Saved,
	HistoryCleared,
	Failed {
		error: StoreError,
		message: &'static str,
		draft_restore: bool,
		history_cleanup: bool,
	},
}
pub struct Cache {
	send: SyncSender<(u64, Id, u64, Operation, Reservation)>,
	pub receive: Receiver<(u64, Outcome, Reservation)>,
	budget: Arc<Budget>,
	pub history: Arc<HistorySafety>,
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
impl Cache {
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
		let payload = match &operation {
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
				if content.len() > 8192 {
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
		let Some(reservation) = self
			.budget
			.reserve(payload.saturating_add(ids).saturating_add(8192), false)
		else {
			return false;
		};
		let epoch = self.history.epoch();
		if matches!(
			operation,
			Operation::LoadChannel { .. }
				| Operation::SaveChannel { .. }
				| Operation::SaveChanges { .. }
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
		std::thread::spawn(move || {
			let mut store = LocalStore::open_default();
			let results = Arc::new(Budget::default());
			while let Ok((generation, account, epoch, operation, reservation)) = commands.recv() {
				let outcome = execute(&mut store, &worker_history, account, epoch, operation);
				drop(reservation);
				let bytes = match &outcome {
					Outcome::CustomFont(Ok(Some(font))) => {
						font.bytes().len() + font.name.capacity()
					}
					Outcome::Channel { messages, .. } => message_bytes(messages),
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
				};
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
			history,
		}
	}
}

fn execute(
	store: &mut Result<LocalStore, StoreError>,
	history: &HistorySafety,
	account: Id,
	epoch: u64,
	operation: Operation,
) -> Outcome {
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
		Operation::LoadAppPreferences => {
			return Outcome::AppPreferences(match store {
				Ok(store) => store.app_preferences().map(Box::new),
				Err(error) => Err(*error),
			});
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
	if matches!(
		operation,
		Operation::LoadChannel { .. }
			| Operation::SaveChannel { .. }
			| Operation::SaveChanges { .. }
	) && !history.allows(epoch)
	{
		return Outcome::Saved;
	}
	let draft_restore = matches!(operation, Operation::LoadDrafts);
	let history_cleanup = matches!(operation, Operation::ClearHistory);
	let history_unsafe = matches!(
		operation,
		Operation::DeleteMessages { .. } | Operation::ClearHistory | Operation::Forget
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
		Operation::LoadCustomFont
		| Operation::SaveCustomFont(_)
		| Operation::LoadAppPreferences
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
			Operation::LoadCustomFont
			| Operation::SaveCustomFont(_)
			| Operation::LoadAppPreferences
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
			Operation::LoadChannel { channel, request } => store
				.load_channel(account, channel)
				.map(|messages| Outcome::Channel {
					channel,
					request,
					messages,
					epoch,
				}),
			Operation::SaveDraft { channel, content } => store
				.save_draft(account, channel, &content)
				.map(|_| Outcome::Saved),
			Operation::SaveChanges {
				channel,
				messages,
				retained,
			} => store
				.save_changes(account, channel, &messages, &retained)
				.map(|_| Outcome::Saved),
			Operation::SaveChannel { channel, messages } => store
				.save_channel(account, channel, &messages)
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
		Outcome::Failed {
			error,
			message: failure_message,
			draft_restore,
			history_cleanup,
		}
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shortcut_restore_waits_for_queue_space_without_losing_or_duplicating_the_request() {
		let (send, commands) = mpsc::sync_channel(16);
		let (_, receive) = mpsc::sync_channel(16);
		let cache = Cache {
			send,
			receive,
			budget: Arc::new(Budget::default()),
			history: Arc::new(HistorySafety::default()),
		};
		let mut view = ui::MessagingUi::default();
		view.channel_preferences_reload = true;
		for _ in 0..16 {
			assert!(cache.queue(7, Id(0), Operation::LoadAppPreferences));
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
				Operation::LoadAppPreferences
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
				..Default::default()
			};
			let mut view = ui::MessagingUi::default();
			settings.apply(&mut view);
			view.notifications_enabled = enabled;
			settings.observe(&view);
			for _ in 0..16 {
				assert!(cache.queue(1, Id(0), Operation::LoadAppPreferences));
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
	}

	#[test]
	fn game_activity_operations_keep_their_own_results_even_when_history_is_blocked() {
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

	#[test]
	fn reading_operations_report_their_own_results_without_touching_account_history() {
		let safety = HistorySafety::default();
		let mut store = Ok(LocalStore::open(std::path::Path::new(":memory:")).unwrap());
		let value = model::ReadingPreferences {
			zoom_percent: 125,
			sidebar_width: 300,
			show_members: false,
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
				messages: vec![]
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
				.save_channel(owner, channel, &[test_support::message(10, channel)])
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
