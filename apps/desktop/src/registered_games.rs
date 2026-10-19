//! Registered Games persistence and the running-program list behind "Add it!". File work and
//! process enumeration run on background threads, never during rendering.
use model::registered_games::{self as games, RegisteredGame};
use std::{
	io::Read,
	path::PathBuf,
	sync::{Arc, Mutex, mpsc},
};

/// Fits all 128 valid entries even when every path/name byte needs JSON escaping.
const MAX_FILE: u64 = 128 * 1024;
/// The picker filters by name, so a very busy machine need not list every helper process.
const MAX_PROGRAMS: usize = 512;

#[derive(Default)]
pub struct Registered {
	saved: Vec<RegisteredGame>,
	/// Nothing is written before the stored list is read, so it cannot be replaced by an empty one.
	loading: Option<mpsc::Receiver<Vec<RegisteredGame>>>,
	loaded: bool,
	writer: Option<Writer>,
	programs: Option<mpsc::Receiver<Vec<String>>>,
	/// Executable the scan reported last, so a play session writes the file only as it
	/// starts and stops.
	playing: Option<String>,
}

impl Registered {
	pub fn load() -> Self {
		let (send, receive) = mpsc::channel();
		std::thread::spawn(move || {
			let _ = send.send(file().and_then(read).unwrap_or_default());
		});
		Self {
			loading: Some(receive),
			..Self::default()
		}
	}

	/// Applies the stored list once, then persists edits made on the settings page.
	pub fn sync(&mut self, messaging: &mut ui::MessagingUi, ctx: &eframe::egui::Context) {
		if let Some(loading) = &self.loading {
			match loading.try_recv() {
				Ok(stored) => {
					messaging.registered_games = stored.clone();
					self.saved = stored;
					self.loaded = true;
					self.loading = None;
					ctx.request_repaint();
				}
				Err(mpsc::TryRecvError::Empty) => {}
				Err(mpsc::TryRecvError::Disconnected) => {
					self.loaded = true;
					self.loading = None;
				}
			}
		}
		if self.loaded && messaging.registered_games != self.saved {
			games::sanitize(&mut messaging.registered_games);
			self.saved = messaging.registered_games.clone();
			let writer = self.writer.get_or_insert_with(spawn_writer);
			writer.send(self.saved.clone());
		}
		if std::mem::take(&mut messaging.running_processes_request) && self.programs.is_none() {
			let (send, receive) = mpsc::channel();
			let wake = ctx.clone();
			std::thread::spawn(move || {
				let programs = platform::processes::running()
					.map(|paths| programs(paths, std::env::current_exe().ok()))
					.unwrap_or_default();
				let _ = send.send(programs);
				wake.request_repaint();
			});
			self.programs = Some(receive);
		}
		if let Some(programs) = &self.programs {
			match programs.try_recv() {
				Ok(list) => {
					messaging.running_processes = Some(list);
					self.programs = None;
				}
				Err(mpsc::TryRecvError::Disconnected) => {
					messaging.running_processes = Some(Vec::new());
					self.programs = None;
				}
				Err(mpsc::TryRecvError::Empty) => {}
			}
		}
	}

	pub fn games(&self) -> &[RegisteredGame] {
		&self.saved
	}

	/// Adds detected games to the Added Games list and keeps their last-played time, as
	/// Discord does. Edits land in `messaging` and are written by the next [`Self::sync`].
	pub fn record(&mut self, messaging: &mut ui::MessagingUi, now_ms: u64) {
		// The stored list replaces the page's list when it loads; recording waits for it.
		if !self.loaded {
			return;
		}
		let running = messaging.running_game.as_ref();
		if running.map(|game| &game.executable) == self.playing.as_ref() {
			return;
		}
		if let Some(previous) = self.playing.take() {
			games::stopped(&mut messaging.registered_games, &previous, now_ms);
		}
		if let Some(game) = running {
			games::record(&mut messaging.registered_games, game, now_ms);
			self.playing = Some(game.executable.clone());
		}
	}
}

/// One replaceable catalog and one wake token remain pending while disk work is blocked.
struct Writer {
	latest: Arc<Mutex<Option<Vec<RegisteredGame>>>>,
	wake: mpsc::SyncSender<()>,
}

impl Writer {
	fn send(&self, list: Vec<RegisteredGame>) {
		*self.latest.lock().expect("catalog writer lock") = Some(list);
		let _ = self.wake.try_send(());
	}
}

fn spawn_writer() -> Writer {
	spawn_writer_with(|latest| {
		if let Some(path) = file() {
			let _ = write(&path, latest);
		}
	})
}

fn spawn_writer_with(mut write: impl FnMut(&[RegisteredGame]) + Send + 'static) -> Writer {
	let (wake, receive) = mpsc::sync_channel(1);
	let latest = Arc::new(Mutex::new(None::<Vec<RegisteredGame>>));
	let pending = latest.clone();
	std::thread::spawn(move || {
		while receive.recv().is_ok() {
			let latest = pending.lock().expect("catalog writer lock").take();
			if let Some(latest) = latest {
				write(&latest);
			}
		}
	});
	Writer { latest, wake }
}

fn file() -> Option<PathBuf> {
	local_store::data_dir()
		.ok()
		.map(|root| root.join("registered_games.json"))
}

fn read(path: PathBuf) -> Option<Vec<RegisteredGame>> {
	let file = std::fs::File::open(path).ok()?;
	(file.metadata().ok()?.len() <= MAX_FILE).then_some(())?;
	let mut bytes = Vec::new();
	file.take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
	(bytes.len() as u64 <= MAX_FILE).then_some(())?;
	let mut stored: Vec<RegisteredGame> = serde_json::from_slice(&bytes).ok()?;
	games::sanitize(&mut stored);
	Some(stored)
}

fn write(path: &std::path::Path, list: &[RegisteredGame]) -> Option<()> {
	std::fs::create_dir_all(path.parent()?).ok()?;
	let bytes = serde_json::to_vec(list).ok()?;
	(bytes.len() as u64 <= MAX_FILE).then_some(())?;
	let partial = path.with_extension("json.partial");
	std::fs::write(&partial, bytes).ok()?;
	std::fs::rename(partial, path).ok()
}

/// Operating-system helpers can never be the game, and listing them buries the real choices.
const SYSTEM: [&str; 9] = [
	"/system/",
	"/library/apple/",
	"/usr/libexec/",
	"/usr/sbin/",
	"/sbin/",
	"/usr/lib/",
	"/lib/",
	"/lib64/",
	"/c:/windows/",
];

/// Running programs the user may pick, one per executable, sorted by their display name.
fn programs(paths: Vec<String>, own: Option<PathBuf>) -> Vec<String> {
	let own = own.and_then(|own| games::normalize(own.to_str()?));
	let mut seen = std::collections::HashSet::new();
	let mut list: Vec<String> = paths
		.into_iter()
		.filter(|path| {
			let Some(key) = games::normalize(path) else {
				return false;
			};
			// Unix lists bare argv[0] words next to the real image path; Windows has only names.
			(cfg!(windows) || path.contains(['/', '\\']))
				&& !SYSTEM
					.iter()
					.any(|prefix| format!("/{key}").starts_with(prefix))
				&& Some(&key) != own.as_ref()
				&& seen.insert(key)
		})
		.collect();
	list.sort_by_cached_key(|path| games::default_name(path).to_lowercase());
	list.truncate(MAX_PROGRAMS);
	list
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::time::Duration;

	#[test]
	fn stalled_writer_retains_only_the_latest_catalog_and_flushes_on_drop() {
		let (started, ready) = mpsc::sync_channel(1);
		let (release, blocked) = mpsc::sync_channel(1);
		let (written, results) = mpsc::channel();
		let mut first = true;
		let writer = spawn_writer_with(move |list| {
			if first {
				first = false;
				started.send(()).unwrap();
				blocked.recv_timeout(Duration::from_secs(3)).unwrap();
			}
			written.send(list.to_vec()).unwrap();
		});
		let game = |name: String| {
			vec![RegisteredGame {
				executable: "opt/game".into(),
				name,
				application: None,
				hidden: false,
				last_played: None,
			}]
		};
		writer.send(game("First".into()));
		ready.recv_timeout(Duration::from_secs(3)).unwrap();
		for index in 0..1000 {
			writer.send(game(format!("Edit {index}")));
		}
		assert_eq!(
			writer.latest.lock().unwrap().as_ref().unwrap(),
			&game("Edit 999".into())
		);
		release.send(()).unwrap();
		drop(writer);
		assert_eq!(
			results.recv_timeout(Duration::from_secs(3)).unwrap(),
			game("First".into())
		);
		assert_eq!(
			results.recv_timeout(Duration::from_secs(3)).unwrap(),
			game("Edit 999".into())
		);
		assert_eq!(
			results.recv_timeout(Duration::from_secs(3)),
			Err(mpsc::RecvTimeoutError::Disconnected)
		);
	}

	#[test]
	fn pending_process_scan_is_reused_and_disconnected_scan_finishes_loading() {
		let ctx = eframe::egui::Context::default();
		let (send, receive) = mpsc::channel();
		let mut registered = Registered {
			programs: Some(receive),
			..Registered::default()
		};
		let mut messaging = ui::MessagingUi::default();
		messaging.running_processes_request = true;
		registered.sync(&mut messaging, &ctx);
		send.send(vec!["/opt/game".into()]).unwrap();
		registered.sync(&mut messaging, &ctx);
		assert_eq!(messaging.running_processes, Some(vec!["/opt/game".into()]));
		assert!(registered.programs.is_none());
		let (send, receive) = mpsc::channel();
		registered.programs = Some(receive);
		messaging.running_processes = None;
		drop(send);
		registered.sync(&mut messaging, &ctx);
		assert_eq!(messaging.running_processes, Some(Vec::new()));
		assert!(registered.programs.is_none());
	}

	#[test]
	fn picker_lists_each_user_program_once() {
		let paths = [
			"/opt/Game/game.x86_64",
			"/opt/game/GAME.x86_64",
			"/usr/lib/systemd/systemd",
			"/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder",
			"/opt/serein/serein",
			"bash",
			"/usr/bin/alpha",
		]
		.map(str::to_owned)
		.to_vec();
		let list = programs(paths, Some("/opt/serein/serein".into()));
		if cfg!(windows) {
			assert!(list.contains(&"bash".to_owned()));
		} else {
			assert_eq!(list, ["/usr/bin/alpha", "/opt/Game/game.x86_64"]);
		}
	}

	#[test]
	fn detected_games_are_recorded_once_per_play_session() {
		let mut registered = Registered {
			loaded: true,
			..Registered::default()
		};
		let mut messaging = ui::MessagingUi::default();
		messaging.running_game = Some(games::RunningGame {
			executable: "opt/scanned".into(),
			name: "Scanned game".into(),
			application: Some(model::Id(7)),
			renamed: false,
		});
		registered.record(&mut messaging, 10);
		registered.record(&mut messaging, 11);
		assert_eq!(messaging.registered_games.len(), 1);
		assert_eq!(messaging.registered_games[0].last_played, Some(10));
		assert!(messaging.registered_games[0].detected());
		let running = messaging.running_game.take();
		registered.record(&mut messaging, 20);
		assert_eq!(messaging.registered_games[0].last_played, Some(20));
		// Nothing is recorded before the stored list has loaded.
		let mut loading = Registered::default();
		let mut fresh = ui::MessagingUi::default();
		fresh.running_game = running;
		loading.record(&mut fresh, 30);
		assert!(fresh.registered_games.is_empty());
	}

	#[test]
	fn maximum_valid_escaped_catalog_round_trips() {
		let directory =
			std::env::temp_dir().join(format!("serein-games-max-{}", std::process::id()));
		let path = directory.join("registered_games.json");
		let list: Vec<_> = (0..games::MAX_GAMES)
			.map(|index| RegisteredGame {
				executable: format!("{index:03}/{}", "\"".repeat(games::MAX_EXECUTABLE - 4)),
				name: "\"".repeat(games::MAX_NAME),
				application: Some(model::Id(u64::MAX)),
				hidden: false,
				last_played: Some(u64::MAX),
			})
			.collect();
		assert!(list.iter().all(RegisteredGame::valid));
		assert!(serde_json::to_vec(&list).unwrap().len() > 64 * 1024);
		assert_eq!(write(&path, &list), Some(()));
		assert_eq!(read(path).unwrap(), list);
		std::fs::remove_dir_all(directory).unwrap();
	}

	#[test]
	fn stored_list_round_trips_and_rejects_oversize_files() {
		let directory = std::env::temp_dir().join(format!("serein-games-{}", std::process::id()));
		let path = directory.join("registered_games.json");
		let list = vec![RegisteredGame {
			executable: "opt/game/game.x86_64".into(),
			name: "My game".into(),
			application: None,
			hidden: false,
			last_played: Some(1_700_000_000_000),
		}];
		write(&path, &list).unwrap();
		assert_eq!(read(path.clone()).unwrap(), list);
		// Lists written before detections were recorded still load.
		std::fs::write(
			&path,
			br#"[{"executable":"a.exe","name":"A","application":"7","hidden":true}]"#,
		)
		.unwrap();
		let legacy = read(path.clone()).unwrap();
		assert_eq!(legacy[0].last_played, None);
		assert!(legacy[0].detected() && legacy[0].hidden);
		std::fs::write(&path, vec![b' '; MAX_FILE as usize + 1]).unwrap();
		assert!(read(path).is_none());
		let _ = std::fs::remove_dir_all(directory);
	}
}
