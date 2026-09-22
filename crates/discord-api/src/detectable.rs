//! Discord's public, credential-free list of detectable games, reduced to the executable
//! names local process detection compares against. No account data is sent or received.
use crate::rpc::download;
use model::Id;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::time::Instant;

/// The published list is a few megabytes of JSON; refuse anything beyond a generous cap.
const MAX_LIST: usize = 24 * 1024 * 1024;
/// The published list holds roughly 24,000 applications, of which under half are detectable.
const MAX_GAMES: usize = 65_536;
const MAX_EXECUTABLES: usize = 16;
const MAX_NAME: usize = 128;
const MAX_EXECUTABLE: usize = 256;

/// One detectable game reduced to what matching needs: never store URLs, hashes or SKUs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Game {
	pub id: Id,
	pub name: String,
	/// Lowercase, forward-slashed executable names, launchers excluded.
	pub executables: Vec<String>,
}

/// Every field is optional so one unexpected entry skips itself instead of the whole list.
#[derive(Deserialize)]
struct WireGame {
	#[serde(default)]
	id: Option<String>,
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	executables: Option<Vec<WireExecutable>>,
}
#[derive(Deserialize)]
struct WireExecutable {
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	is_launcher: bool,
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

/// Returns the raw list; decoding several megabytes of JSON belongs off the async runtime.
pub async fn download_list(
	client: &Client,
	cooldown: &mut Instant,
) -> Result<Vec<u8>, &'static str> {
	download(
		client,
		cooldown,
		"https://discord.com/api/v10/applications/detectable",
		MAX_LIST,
	)
	.await
}

pub fn decode(bytes: &[u8]) -> Result<Vec<Game>, &'static str> {
	let failure = "The detectable game list is invalid.";
	if bytes.len() > MAX_LIST {
		return Err(failure);
	}
	let wire: Vec<WireGame> = serde_json::from_slice(bytes).map_err(|_| failure)?;
	let games: Vec<Game> = wire
		.into_iter()
		.filter_map(|game| {
			let id: Id = game.id?.parse().ok()?;
			let name = game.name?.trim().to_owned();
			if name.is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control) {
				return None;
			}
			let mut executables: Vec<String> = game
				.executables
				.unwrap_or_default()
				.into_iter()
				// Launchers keep reporting after the game exits; Discord excludes them too.
				.filter(|executable| !executable.is_launcher)
				.filter_map(|executable| normalize(executable.name.as_deref()?))
				.take(MAX_EXECUTABLES)
				.collect();
			executables.sort();
			executables.dedup();
			(!executables.is_empty()).then_some(Game {
				id,
				name,
				executables,
			})
		})
		.collect();
	bound(games)
}

/// The reduced form Serein caches on disk. It is re-checked exactly like a fresh download.
pub fn decode_cached(bytes: &[u8]) -> Result<Vec<Game>, &'static str> {
	let failure = "The cached game list is invalid.";
	if bytes.len() > MAX_LIST {
		return Err(failure);
	}
	let games: Vec<Game> = serde_json::from_slice(bytes).map_err(|_| failure)?;
	bound(games).map_err(|_| failure)
}

fn bound(mut games: Vec<Game>) -> Result<Vec<Game>, &'static str> {
	let failure = "The detectable game list is invalid.";
	if games.len() > MAX_GAMES {
		return Err(failure);
	}
	games.retain(|game| {
		game.id.0 != 0
			&& !game.name.is_empty()
			&& game.name.len() <= MAX_NAME
			&& !game.name.chars().any(char::is_control)
			&& !game.executables.is_empty()
			&& game.executables.len() <= MAX_EXECUTABLES
			&& game
				.executables
				.iter()
				.all(|value| normalize(value).as_deref() == Some(value.as_str()))
	});
	if games.is_empty() {
		return Err(failure);
	}
	Ok(games)
}

/// Executable-name lookup built once per sharing session. Comparing every running process
/// against every entry is quadratic and far too slow to repeat on a timer.
pub struct Index(HashMap<String, (Id, String)>);

impl Index {
	pub fn new(games: &[Game]) -> Self {
		let mut names = HashMap::with_capacity(games.len());
		for game in games {
			for executable in &game.executables {
				// The first claim wins, so a later entry cannot shadow an earlier game.
				names
					.entry(executable.clone())
					.or_insert_with(|| (game.id, game.name.clone()));
			}
		}
		Self(names)
	}
	pub fn len(&self) -> usize {
		self.0.len()
	}
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}
	/// Match a running executable path. Longer suffixes win, so
	/// `steamapps/common/game/game.exe` beats a bare `game.exe` from another title.
	pub fn find(&self, path: &str) -> Option<(Id, String)> {
		let path = path.to_lowercase();
		let mut parts = [""; 8];
		let mut count = 0;
		for (index, part) in path
			.rsplit(['/', '\\'])
			.filter(|part| !part.is_empty())
			.take(parts.len())
			.enumerate()
		{
			parts[index] = part;
			count = index + 1;
		}
		let parts = &mut parts[..count];
		parts.reverse();
		let suffix = parts.join("/");
		let mut candidate = suffix.as_str();
		while !candidate.is_empty() {
			if let Some(found) = self.0.get(candidate) {
				return Some(found.clone());
			}
			candidate = candidate.split_once('/').map_or("", |(_, rest)| rest);
		}
		// macOS entries name the bundle, which sits above the executable inside it.
		parts
			.iter()
			.rev()
			.filter(|part| part.ends_with(".app"))
			.find_map(|part| self.0.get(*part).cloned())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	#[ignore = "release process matcher benchmark; one warmup and five measured batches"]
	fn process_matcher_benchmark() {
		const PROCESSES: usize = 4096;
		const SCANS: usize = 100;
		let games: Vec<Game> = (0..PROCESSES / 4)
			.map(|game| Game {
				id: Id(game as u64 + 1),
				name: format!("Synthetic game {game}"),
				executables: vec![
					format!("game-{game}.exe"),
					format!("steamapps/common/game-{game}/bin/play.exe"),
					format!("synthetic {game}.app"),
				],
			})
			.collect();
		let index = Index::new(&games);
		let paths: Vec<String> = (0..PROCESSES)
			.map(|process| {
				let game = process / 4;
				match process % 4 {
					0 => format!("/opt/synthetic/tools/{game}/unmatched.exe"),
					1 => format!(r"C:\Games\Slot{game}\game-{game}.exe"),
					2 => format!(r"C:\Steam\steamapps\common\game-{game}\bin\play.exe"),
					_ => format!("/Applications/Synthetic {game}.app/Contents/MacOS/main"),
				}
			})
			.collect();
		let mut samples = [0.0; 5];
		for batch in 0..=samples.len() {
			let started = std::time::Instant::now();
			let mut hits = 0;
			for _ in 0..SCANS {
				for path in &paths {
					hits += usize::from(
						std::hint::black_box(index.find(std::hint::black_box(path))).is_some(),
					);
				}
			}
			let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
			assert_eq!(hits, PROCESSES / 4 * 3 * SCANS);
			if batch > 0 {
				samples[batch - 1] = elapsed_ms;
			}
		}
		eprintln!(
			"Synthetic process matcher: {SCANS} scans of {PROCESSES} paths; batch ms {samples:?}"
		);
		samples.sort_by(f64::total_cmp);
		eprintln!(
			"Median {:.3} ms per batch, {:.3} ms per scan; matching only, excluding process enumeration.",
			samples[2],
			samples[2] / SCANS as f64,
		);
	}

	#[test]
	fn suffix_matching_preserves_component_limits_and_bundle_priority() {
		let games: Vec<Game> = [
			(1, "game.exe"),
			(2, "a/b/game.exe"),
			(3, "outer.app"),
			(4, "inner.app"),
			(5, "b/c/d/e/f/g/h/game.exe"),
			(6, "a/b/c/d/e/f/g/h/game.exe"),
			(7, "ος"),
		]
		.into_iter()
		.map(|(id, executable)| Game {
			id: Id(id),
			name: executable.into(),
			executables: vec![executable.into()],
		})
		.collect();
		let index = Index::new(&games);
		for (path, expected) in [
			(r"C:\\A//B\\GAME.EXE/", Some(Id(2))),
			("/a/b/c/d/e/f/g/h/game.exe", Some(Id(5))),
			("/outer.app/inner.app/Contents/MacOS/main", Some(Id(4))),
			("/outer.app/Contents/MacOS/game.exe", Some(Id(1))),
			("/outer.app/a/b/c/d/e/f/g/main", None),
			("/usr/bin/mygame.exe", None),
			("/usr/bin/ΟΣ", Some(Id(7))),
			("////", None),
			("", None),
		] {
			assert_eq!(index.find(path).map(|(id, _)| id), expected, "{path}");
		}
	}

	#[test]
	fn list_drops_launchers_and_matches_the_longest_path_suffix() {
		let games = decode(
			br#"[
			{"id":"7","name":"A game outside the old list","executables":[
				{"name":"Game.exe","is_launcher":false},
				{"name":"launcher.exe","is_launcher":true},
				{"name":">steamapps/common/game/game.exe","is_launcher":false}]},
			{"id":"8","name":"Another game","executables":[{"name":"game.exe","is_launcher":false}]},
			{"id":"9","name":"Launcher only","executables":[{"name":"only.exe","is_launcher":true}]},
			{"id":"0","name":"Invalid id","executables":[{"name":"zero.exe"}]},
			{"name":"No id","executables":[{"name":"missing.exe"}]},
			{"id":"11"}
		]"#,
		)
		.unwrap();
		assert_eq!(games.len(), 2);
		assert_eq!(
			games[0].executables,
			["game.exe", "steamapps/common/game/game.exe"]
		);
		// Two games claim `game.exe`; the first keeps it, so the index holds two names.
		let index = Index::new(&games);
		assert_eq!(index.len(), 2);
		assert_eq!(index.find("/opt/game.exe").unwrap().0, Id(7));
		assert_eq!(
			index.find(r"C:\Steam\steamapps\common\Game\Game.exe"),
			Some((Id(7), "A game outside the old list".into()))
		);
		assert_eq!(index.find("/usr/bin/other"), None);
		assert!(index.find("/opt/game.exe").is_some());
		// A macOS bundle is named by the list but never the process path's last component.
		let bundles = Index::new(
			&decode(
				br#"[{"id":"7","name":"Bundled game","executables":[{"name":"Bundled Game.app"}]}]"#,
			)
			.unwrap(),
		);
		assert_eq!(
			bundles.find("/Applications/Bundled Game.app/Contents/MacOS/bundled"),
			Some((Id(7), "Bundled game".into()))
		);
	}

	#[test]
	fn oversize_empty_and_malformed_lists_are_refused() {
		assert!(decode(b"[]").is_err());
		assert!(decode(b"{}").is_err());
		assert!(decode(&vec![b' '; MAX_LIST + 1]).is_err());
		assert!(decode(br#"[{"id":"7","name":"No executables"}]"#).is_err());
		assert!(
			decode(br#"[{"id":"7","name":"Bad\u0007name","executables":[{"name":"a"}]}]"#).is_err()
		);
		assert!(decode(br#"[{"id":7,"name":"Numeric id","executables":[{"name":"a"}]}]"#).is_err());
		// The on-disk cache is a different shape and is re-checked against the same bounds.
		let games =
			decode(br#"[{"id":"7","name":"Cached","executables":[{"name":"Game.exe"}]}]"#).unwrap();
		let cached = serde_json::to_vec(&games).unwrap();
		assert_eq!(decode_cached(&cached).unwrap(), games);
		assert!(decode_cached(&cached[..cached.len() - 1]).is_err());
		assert!(decode(&cached).is_err());
		assert!(
			decode_cached(br#"[{"id":"7","name":"Cached","executables":["Not/Normal.EXE"]}]"#)
				.is_err()
		);
		assert!(normalize("  ").is_none());
		assert!(normalize(&"x".repeat(MAX_EXECUTABLE + 1)).is_none());
		assert_eq!(
			normalize(r"\Game\Game.EXE").as_deref(),
			Some("game/game.exe")
		);
	}
}
