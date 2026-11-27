//! Discord's public, credential-free list of detectable games, reduced to the executable
//! names local process detection compares against. No account data is sent or received.
use crate::rpc::download;
use model::Id;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tokio::time::Instant;
use wreq::Client;

/// The published list is a few megabytes of JSON; refuse anything beyond a generous cap.
const MAX_LIST: usize = 24 * 1024 * 1024;
/// The published list holds roughly 24,000 applications.
const MAX_GAMES: usize = 65_536;
const MAX_EXECUTABLES: usize = 16;
const MAX_NAME: usize = 128;

/// One detectable game reduced to what matching needs: never store URLs, hashes or SKUs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Game {
	pub id: Id,
	pub name: String,
	/// Lowercase, forward-slashed executable names, launchers excluded. Empty for a game
	/// Discord detects by other means; its title still resolves a manually added game.
	#[serde(default)]
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

/// Game references need presentation metadata, independently of process detection.
#[derive(Deserialize)]
struct WireGameMention {
	#[serde(default)]
	id: Option<String>,
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	icon: Option<String>,
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
	model::registered_games::normalize(name)
}

/// Returns the raw list; decoding several megabytes of JSON belongs off the async runtime.
pub async fn download_list(
	client: &Client,
	cooldown: &mut Instant,
) -> Result<Vec<u8>, &'static str> {
	download(
		client,
		cooldown,
		&format!("{}/applications/detectable", crate::api_base()),
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
			Some(Game {
				id,
				name,
				executables,
			})
		})
		.collect();
	bound(games)
}

/// The same public list also contains games without local executable detection.
pub fn decode_mentions(bytes: &[u8]) -> Result<Vec<model::GameMention>, &'static str> {
	let failure = "The game catalog is unavailable.";
	if bytes.len() > MAX_LIST {
		return Err(failure);
	}
	let wire: Vec<WireGameMention> = serde_json::from_slice(bytes).map_err(|_| failure)?;
	if wire.len() > MAX_GAMES {
		return Err(failure);
	}
	let games: Vec<_> = wire
		.into_iter()
		.filter_map(|game| {
			let id = game.id?.parse().ok()?;
			let name = game.name?.trim().to_owned();
			if name.is_empty()
				|| name.len() > MAX_NAME
				|| name.chars().any(|c| {
					c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
				}) {
				return None;
			}
			Some(model::GameMention {
				id,
				name,
				icon: game.icon.filter(|hash| model::valid_avatar_hash(hash)),
			})
		})
		.collect();
	if games.is_empty() {
		Err(failure)
	} else {
		Ok(games)
	}
}

#[cfg(test)]
mod mention_tests {
	#[test]
	fn game_mentions_ignore_malformed_executable_metadata() {
		for executables in [
			r#"[{"is_launcher":null}]"#,
			r#"[{"name":42,"is_launcher":"yes"}]"#,
			r#"{"unexpected":true}"#,
		] {
			let list = format!(
				r#"[{{"id":"42","name":"Deadlock","icon":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","executables":{executables}}},{{"id":"43","name":"Dead Space"}}]"#
			);
			let games = super::decode_mentions(list.as_bytes()).unwrap();
			assert_eq!(games.len(), 2);
			assert_eq!(games[0].id, model::Id(42));
			assert_eq!(games[0].name, "Deadlock");
			assert_eq!(
				games[0].icon.as_deref(),
				Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
			);
		}
	}

	#[test]
	fn catalog_keeps_games_without_executables_and_validates_icons() {
		let games = super::decode_mentions(
			br#"[
			{"id":"42","name":"Deadlock","icon":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
			{"id":"43","name":"Dead Space","icon":"../private"},
			{"id":"44","name":"Bad\nName"},
			{"id":"0","name":"Zero"}
		]"#,
		)
		.unwrap();
		assert_eq!(games.len(), 2);
		assert_eq!(games[0].name, "Deadlock");
		assert_eq!(
			games[0].icon.as_deref(),
			Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
		);
		assert!(games[1].icon.is_none());
		assert!(super::decode_mentions(b"[]").is_err());
	}
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
pub struct Index {
	names: HashMap<String, (Id, String)>,
	/// Path suffixes two applications claim. A readable path that reaches one is a guess, so
	/// suffix lookup stops there instead of falling through to a shorter, unique name.
	ambiguous_names: HashSet<String>,
	/// A Windows process that cannot be queried exposes only its image name, while Discord's
	/// list often stores `win64/cs2.exe`. Only bare inputs use this, and only names a
	/// single application claims: `launcher.exe` must not select one arbitrary game.
	basenames: HashMap<String, (Id, String)>,
	/// Lowercase titles; `None` when several applications share one, so a guess never wins.
	titles: HashMap<String, Option<Id>>,
}

impl Index {
	pub fn new(games: &[Game]) -> Self {
		let mut names = HashMap::with_capacity(games.len());
		let mut basenames = HashMap::with_capacity(games.len());
		let mut titles = HashMap::with_capacity(games.len());
		let mut ambiguous_names = HashSet::new();
		let mut ambiguous_basenames = HashSet::new();
		for game in games {
			titles
				.entry(game.name.to_lowercase())
				.and_modify(|claim| {
					if *claim != Some(game.id) {
						*claim = None;
					}
				})
				.or_insert(Some(game.id));
			for executable in &game.executables {
				// A name two games claim stays unmatched: keeping either would report a game
				// the user is not playing.
				let found = (game.id, game.name.clone());
				claim(&mut names, &mut ambiguous_names, executable.clone(), &found);
				if let Some(base) = executable.rsplit('/').next() {
					claim(
						&mut basenames,
						&mut ambiguous_basenames,
						base.to_owned(),
						&found,
					);
				}
			}
		}
		Self {
			names,
			ambiguous_names,
			basenames,
			titles,
		}
	}
	pub fn len(&self) -> usize {
		self.names.len()
	}
	pub fn is_empty(&self) -> bool {
		self.names.is_empty()
	}
	/// The application a manually added game names exactly, when only one carries that title.
	pub fn find_title(&self, name: &str) -> Option<Id> {
		self.titles
			.get(&name.trim().to_lowercase())
			.copied()
			.flatten()
	}
	/// Match a running executable path. Longer suffixes win; a name two games claim is never
	/// matched, and neither is a shorter name behind it, so a generic image name cannot
	/// select an arbitrary game.
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
		// A process whose path could not be read supplies only its image name, while the
		// list often stores `win64/cs2.exe`. Only `basenames` knows whether several games
		// claim that image name, so a bare input never consults `names` first.
		if let [basename] = parts {
			return self.basenames.get(*basename).cloned();
		}
		let suffix = parts.join("/");
		let mut candidate = suffix.as_str();
		while !candidate.is_empty() {
			if let Some(found) = self.names.get(candidate) {
				return Some(found.clone());
			}
			// The path ends with a name two games claim. A shorter name below it belongs to
			// a different program, so it must not be reported for this path, and neither
			// must a bundle named elsewhere in the path.
			if self.ambiguous_names.contains(candidate) {
				return None;
			}
			candidate = candidate.split_once('/').map_or("", |(_, rest)| rest);
		}
		// macOS entries name the bundle, which sits above the executable inside it. The
		// innermost bundle wins; when two games claim it, an outer bundle is a different
		// application and cannot stand in for it.
		for part in parts.iter().rev().filter(|part| part.ends_with(".app")) {
			if let Some(found) = self.names.get(*part) {
				return Some(found.clone());
			}
			if self.ambiguous_names.contains(*part) {
				return None;
			}
		}
		// A known directory that does not match must not fall back either: `GitHub CLI/gh.exe`
		// is not `green hell/gh.exe`.
		None
	}
}

/// Remembers one executable claim. Two different games claiming one name make it
/// ambiguous, so neither can be reported from a bare image name or a shared path.
fn claim(
	map: &mut HashMap<String, (Id, String)>,
	ambiguous: &mut HashSet<String>,
	key: String,
	found: &(Id, String),
) {
	if ambiguous.contains(&key) {
		return;
	}
	match map.get(&key) {
		Some(existing) if existing.0 != found.0 => {
			map.remove(&key);
			ambiguous.insert(key);
		}
		Some(_) => {}
		None => {
			map.insert(key, found.clone());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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
	fn bare_windows_image_names_match_nested_detectable_paths() {
		let games = decode(
			br#"[{"id":"1158877933042143272","name":"Counter-Strike 2",
				"executables":[{"name":"win64/cs2.exe","is_launcher":false}]}]"#,
		)
		.unwrap();
		let index = Index::new(&games);
		assert_eq!(
			index
				.find("C:\\Games\\CS2\\game\\bin\\win64\\cs2.exe")
				.unwrap()
				.1,
			"Counter-Strike 2"
		);
		// A known path in another directory is a different program with the same file name.
		assert!(index.find("C:\\Tools\\cs2.exe").is_none());
		assert_eq!(index.find("cs2.exe").unwrap().1, "Counter-Strike 2");
		assert_eq!(index.find("CS2.EXE").unwrap().1, "Counter-Strike 2");
		assert!(index.find("not-cs2.exe").is_none());
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
			{"id":"11"},
			{"id":"12","name":"another GAME"}
		]"#,
		)
		.unwrap();
		// A launcher-only game stays as a title; it never matches a process.
		assert_eq!(games.len(), 4);
		assert!(games[2].executables.is_empty());
		assert_eq!(
			games[0].executables,
			["game.exe", "steamapps/common/game/game.exe"]
		);
		// Two games claim `game.exe`; a bare image name is ambiguous and stays unmatched,
		// while the longer exact path still resolves.
		let index = Index::new(&games);
		assert_eq!(index.len(), 1);
		assert_eq!(index.find("/opt/game.exe"), None);
		assert_eq!(
			index.find(r"C:\Steam\steamapps\common\Game\Game.exe"),
			Some((Id(7), "A game outside the old list".into()))
		);
		assert_eq!(index.find("/usr/bin/other"), None);
		assert_eq!(index.find("only.exe"), None);
		// Manually added games resolve by exact title, never by a title two applications share.
		assert_eq!(index.find_title(" launcher ONLY "), Some(Id(9)));
		assert_eq!(index.find_title("Another game"), None);
		assert_eq!(index.find_title("Launcher"), None);
		assert_eq!(index.find("/opt/game.exe"), None);
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
	fn shared_image_names_never_resolve_to_an_arbitrary_game() {
		let games = decode(
			br#"[
			{"id":"1","name":"First","executables":[{"name":"total war/launcher.exe"}]},
			{"id":"2","name":"Second","executables":[{"name":"other/launcher.exe"}]},
			{"id":"3","name":"Unique","executables":[{"name":"win64/unique.exe"}]},
			{"id":"4","name":"Two paths","executables":[
				{"name":"one/shared.exe"},{"name":"two/shared.exe"}]}
		]"#,
		)
		.unwrap();
		let index = Index::new(&games);
		// An unreadable process exposes only `launcher.exe`; two owners make it a guess.
		assert_eq!(index.find("launcher.exe"), None);
		// A readable path still matches the longest known suffix.
		assert_eq!(
			index
				.find(r"C:\Games\Total War\launcher.exe")
				.map(|(id, _)| id),
			Some(Id(1))
		);
		assert_eq!(
			index.find(r"C:\Games\Other\launcher.exe").map(|(id, _)| id),
			Some(Id(2))
		);
		// One game with two paths to the same image name is not ambiguous.
		assert_eq!(index.find("shared.exe").map(|(id, _)| id), Some(Id(4)));
		// A name only one game claims still resolves from an image name.
		assert_eq!(index.find("unique.exe").map(|(id, _)| id), Some(Id(3)));
	}

	#[test]
	fn bare_and_nested_claims_of_one_image_name_stay_unmatched() {
		let games = decode(
			br#"[
			{"id":"1","name":"Bare","executables":[{"name":"launcher.exe"}]},
			{"id":"2","name":"Nested","executables":[{"name":"other/launcher.exe"}]}
		]"#,
		)
		.unwrap();
		let index = Index::new(&games);
		// An unreadable process path supplies only the image name, which both games claim.
		assert_eq!(index.find("launcher.exe"), None);
		// A readable path still resolves the suffix it actually ends with.
		assert_eq!(
			index.find(r"C:\Games\Bare\launcher.exe").map(|(id, _)| id),
			Some(Id(1))
		);
		assert_eq!(
			index.find(r"C:\Games\Other\launcher.exe").map(|(id, _)| id),
			Some(Id(2))
		);
	}

	#[test]
	fn shared_path_suffix_does_not_fall_through_to_a_shorter_name() {
		let games = decode(
			br#"[
			{"id":"1","name":"First","executables":[{"name":"x/game.exe"}]},
			{"id":"2","name":"Second","executables":[{"name":"x/game.exe"}]},
			{"id":"3","name":"Third","executables":[{"name":"game.exe"}]}
		]"#,
		)
		.unwrap();
		let index = Index::new(&games);
		// `x/game.exe` belongs to two games, so reaching it must not return the game that
		// merely claims the shorter `game.exe`.
		assert_eq!(index.find(r"C:\Games\x\game.exe"), None);
		// A path that only ends at the third game's own claim still resolves it.
		assert_eq!(
			index.find(r"C:\Games\Third\game.exe").map(|(id, _)| id),
			Some(Id(3))
		);
	}

	#[test]
	fn ambiguous_suffix_does_not_fall_through_to_an_enclosing_bundle() {
		let games = decode(
			br#"[
			{"id":"1","name":"First","executables":[{"name":"x/game.exe"}]},
			{"id":"2","name":"Second","executables":[{"name":"x/game.exe"}]},
			{"id":"3","name":"Bundle","executables":[{"name":"outer.app"}]}
		]"#,
		)
		.unwrap();
		let index = Index::new(&games);
		// Two games claim the executable suffix, so the enclosing bundle must not decide
		// the match on their behalf.
		assert_eq!(index.find("/outer.app/x/game.exe"), None);
	}

	#[test]
	fn shared_inner_bundle_does_not_fall_through_to_the_outer_bundle() {
		let games = decode(
			br#"[
			{"id":"1","name":"First","executables":[{"name":"inner.app"}]},
			{"id":"2","name":"Second","executables":[{"name":"inner.app"}]},
			{"id":"3","name":"Outer","executables":[{"name":"outer.app"}]}
		]"#,
		)
		.unwrap();
		let index = Index::new(&games);
		// The nearest bundle belongs to two games; the enclosing one is a different app.
		assert_eq!(index.find("/outer.app/inner.app/Contents/MacOS/main"), None);
	}

	#[test]
	fn oversize_empty_and_malformed_lists_are_refused() {
		assert!(decode(b"[]").is_err());
		assert!(decode(b"{}").is_err());
		assert!(decode(&vec![b' '; MAX_LIST + 1]).is_err());
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
		assert!(normalize(&"x".repeat(model::registered_games::MAX_EXECUTABLE + 1)).is_none());
		assert_eq!(
			normalize(r"\Game\Game.EXE").as_deref(),
			Some("game/game.exe")
		);
	}
}
