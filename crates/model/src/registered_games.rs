//! Games the user registered on this device, matching Discord's "Registered Games" page:
//! manually added executables, renamed detections and detections the user removed.
use crate::Id;
use serde::{Deserialize, Serialize};

pub const MAX_GAMES: usize = 128;
pub const MAX_NAME: usize = 128;
pub const MAX_EXECUTABLE: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredGame {
	/// Normalized executable path as it was running when registered; see [`normalize`].
	pub executable: String,
	pub name: String,
	/// The detectable application this executable matched, when Discord knows the game.
	#[serde(default)]
	pub application: Option<Id>,
	/// Removed by the user: never detect this executable or application again.
	#[serde(default)]
	pub hidden: bool,
	/// Unix milliseconds when the game was last seen starting or stopping.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub last_played: Option<u64>,
}

impl RegisteredGame {
	pub fn valid(&self) -> bool {
		normalize(&self.executable).as_deref() == Some(self.executable.as_str())
			&& valid_name(&self.name)
	}
	/// Found through Discord's detectable list rather than added by the user.
	pub fn detected(&self) -> bool {
		self.application.is_some()
	}
}

/// Remembers a game the scan reports, like Discord's Added Games list: a known entry gets a
/// new last-played time, and a detection is added so the user can rename or hide it. When the
/// list is full, the detection played longest ago gives way; user choices are never evicted.
pub fn record(games: &mut Vec<RegisteredGame>, running: &RunningGame, now_ms: u64) {
	if let Some(game) = games
		.iter_mut()
		.find(|g| g.executable == running.executable)
	{
		game.last_played = Some(now_ms);
		return;
	}
	let Some(application) = running.application else {
		return;
	};
	if games.len() >= MAX_GAMES {
		let Some(oldest) = games
			.iter()
			.enumerate()
			.filter(|(_, game)| game.detected() && !game.hidden)
			.min_by_key(|(_, game)| game.last_played)
			.map(|(index, _)| index)
		else {
			return;
		};
		games.remove(oldest);
	}
	games.push(RegisteredGame {
		executable: running.executable.clone(),
		name: running.name.clone(),
		application: Some(application),
		hidden: false,
		last_played: Some(now_ms),
	});
}

/// The game stopped: its last-played time is now.
pub fn stopped(games: &mut [RegisteredGame], executable: &str, now_ms: u64) {
	if let Some(game) = games.iter_mut().find(|g| g.executable == executable) {
		game.last_played = Some(now_ms);
	}
}

/// The game a local process scan currently reports, shown as "Current Game".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningGame {
	pub executable: String,
	pub name: String,
	pub application: Option<Id>,
	/// Shown by its registered name rather than Discord's.
	pub renamed: bool,
}

pub fn valid_name(name: &str) -> bool {
	let trimmed = name.trim();
	!trimmed.is_empty()
		&& trimmed.len() == name.len()
		&& name.len() <= MAX_NAME
		&& !name.chars().any(char::is_control)
}

/// Executables are compared as path suffixes, so both sides use one normal form.
pub fn normalize(name: &str) -> Option<String> {
	let name = name.trim().trim_start_matches('>');
	if name.is_empty() || name.len() > MAX_EXECUTABLE || name.chars().any(char::is_control) {
		return None;
	}
	let name = name
		.to_lowercase()
		.replace('\\', "/")
		.trim_matches('/')
		.to_owned();
	(!name.is_empty()).then_some(name)
}

/// A readable default title for a newly added executable: its file name without extension.
pub fn default_name(executable: &str) -> String {
	let file = executable.rsplit(['/', '\\']).next().unwrap_or(executable);
	let file = file.strip_suffix(".app").unwrap_or(file);
	let stem = file
		.rsplit_once('.')
		.filter(|(stem, extension)| !stem.is_empty() && extension.len() <= 4)
		.map_or(file, |(stem, _)| stem);
	let mut name: String = stem.trim().chars().take(MAX_NAME).collect();
	while name.len() > MAX_NAME {
		name.pop();
	}
	if name.is_empty() {
		"Unknown game".into()
	} else {
		name
	}
}

/// Drop invalid and duplicate entries so a hand-edited file cannot grow detection work.
pub fn sanitize(games: &mut Vec<RegisteredGame>) {
	let mut seen = std::collections::HashSet::new();
	games.retain(|game| game.valid() && seen.insert(game.executable.clone()));
	games.truncate(MAX_GAMES);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_and_entries_are_bounded() {
		assert_eq!(
			default_name("/opt/my game/Game-Bin.x86_64"),
			"Game-Bin.x86_64"
		);
		assert_eq!(default_name("c:/games/lms.exe"), "lms");
		assert_eq!(default_name("/applications/foo.app"), "foo");
		assert_eq!(default_name("/usr/bin/.hidden"), ".hidden");
		let game = |executable: &str, name: &str| RegisteredGame {
			executable: executable.into(),
			name: name.into(),
			application: None,
			hidden: false,
			last_played: None,
		};
		let mut games = vec![
			game("a.exe", "A"),
			game("a.exe", "Duplicate"),
			game("B.EXE", "Not normalized"),
			game("c.exe", " padded "),
			game("d.exe", "D"),
		];
		sanitize(&mut games);
		assert_eq!(
			games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
			["A", "D"]
		);
	}

	#[test]
	fn detections_are_recorded_bounded_and_backward_compatible() {
		let running = |executable: &str, application| RunningGame {
			executable: executable.into(),
			name: "Synthetic".into(),
			application,
			renamed: false,
		};
		let mut games = vec![RegisteredGame {
			executable: "a.exe".into(),
			name: "A".into(),
			application: None,
			hidden: false,
			last_played: None,
		}];
		record(&mut games, &running("a.exe", None), 10);
		assert_eq!((games.len(), games[0].last_played), (1, Some(10)));
		// An unknown executable without an application was never added; nothing to record.
		record(&mut games, &running("b.exe", None), 11);
		assert_eq!(games.len(), 1);
		record(&mut games, &running("c.exe", Some(Id(7))), 12);
		assert!(games[1].detected() && !games[1].hidden);
		stopped(&mut games, "c.exe", 20);
		assert_eq!(games[1].last_played, Some(20));
		// A full list evicts the detection played longest ago, never manual or hidden entries.
		for n in games.len()..MAX_GAMES {
			games.push(RegisteredGame {
				executable: format!("full/{n}"),
				name: "Hidden".into(),
				application: Some(Id(100 + n as u64)),
				hidden: true,
				last_played: Some(1),
			});
		}
		record(&mut games, &running("d.exe", Some(Id(8))), 30);
		assert_eq!(games.len(), MAX_GAMES);
		assert!(games.iter().all(|g| g.executable != "c.exe"));
		assert_eq!(games.last().unwrap().executable, "d.exe");
		for game in &mut games {
			game.hidden = game.detected();
		}
		record(&mut games, &running("e.exe", Some(Id(9))), 40);
		assert!(games.iter().all(|g| g.executable != "e.exe"));
	}
}
