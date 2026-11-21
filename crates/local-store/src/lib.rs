//! Account-isolated bounded SQLite cache. This is not Discord's authoritative state.
mod accounts;
mod cache_cleanup;
mod expression_catalog;
mod history;
mod sync_history;
pub use sync_history::{
	HistoryCacheStats, HistoryCoverage, HistoryPageQuery, MAX_CACHED_MESSAGES, SyncCheckpoint,
	SyncPageResult,
};
pub mod cache_budget;
mod member_displays;
mod server_displays;
pub use server_displays::ServerDisplay;
mod preferences;
mod schema;
pub use preferences::{AppPreferences, Appearance, WindowGeometry};
mod account_presence;
mod channel_preferences;
use model::{Id, Message, ReadingPreferences, User};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
	collections::{BTreeMap, BTreeSet},
	ffi::OsString,
	path::{Path, PathBuf},
};

const MAX_MEDIA_JSON: usize = 256 * 1024;
const MAX_WINDOW_BYTES: usize = 4 * 1024 * 1024;
const NATIVE_SCHEMA: u32 = 32;
const READABLE_SCHEMA: u32 = 32;
#[derive(serde::Deserialize)]
struct CachedMentions(#[serde(deserialize_with = "model::deserialize_mentions")] Vec<User>);
fn parse_author_roles(raw: &str) -> std::result::Result<Vec<Id>, StoreError> {
	let values: Vec<String> = serde_json::from_str(raw).map_err(|_| StoreError::Incompatible)?;
	if values.len() > model::permissions::MAX_MEMBER_ROLES {
		return Err(StoreError::Capacity);
	}
	let mut roles = Vec::with_capacity(values.len());
	let mut seen = BTreeSet::new();
	for value in values {
		let id = value.parse::<Id>().map_err(|_| StoreError::Incompatible)?;
		if id.0 == 0 || !seen.insert(id) {
			return Err(StoreError::Incompatible);
		}
		roles.push(id);
	}
	Ok(roles)
}
pub struct LocalStore(Connection);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
	Unavailable,
	Capacity,
	Incompatible,
}
type Result<T> = std::result::Result<T, StoreError>;
impl From<rusqlite::Error> for StoreError {
	fn from(_: rusqlite::Error) -> Self {
		Self::Unavailable
	}
}

/// Resolves an explicit absolute root without falling back when it is invalid.
fn resolve_data_dir(configured: Option<OsString>, default: Option<PathBuf>) -> Result<PathBuf> {
	match configured {
		Some(root) => {
			let root = PathBuf::from(root);
			if root.as_os_str().is_empty() || !root.is_absolute() {
				return Err(StoreError::Unavailable);
			}
			Ok(root)
		}
		None => default.ok_or(StoreError::Unavailable),
	}
}

/// Keeps source-build data persistent but separate from installed Serein data.
fn default_data_dir() -> Option<PathBuf> {
	#[cfg(feature = "development-data")]
	{
		dirs::data_local_dir().map(|root| root.join("serein-development"))
	}
	#[cfg(not(feature = "development-data"))]
	{
		dirs::data_local_dir().map(|root| root.join("serein"))
	}
}

/// Creates only missing directories privately on Unix and preserves existing permissions.
fn create_data_dir(root: &Path) -> Result<()> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::DirBuilderExt;
		std::fs::DirBuilder::new()
			.recursive(true)
			.mode(0o700)
			.create(root)
			.map_err(|_| StoreError::Unavailable)
	}
	#[cfg(not(unix))]
	{
		std::fs::create_dir_all(root).map_err(|_| StoreError::Unavailable)
	}
}

/// Shared root for local application data. Developers may select an isolated absolute path.
pub fn data_dir() -> std::result::Result<PathBuf, StoreError> {
	resolve_data_dir(std::env::var_os("SEREIN_DATA_DIR"), default_data_dir())
}

/// Reject excess entries during parsing, before allocating a whole malformed array.
struct CachedEmbeds(Vec<model::Embed>);
impl<'de> serde::Deserialize<'de> for CachedEmbeds {
	fn deserialize<D: serde::Deserializer<'de>>(
		deserializer: D,
	) -> std::result::Result<Self, D::Error> {
		struct Visitor;
		impl<'de> serde::de::Visitor<'de> for Visitor {
			type Value = CachedEmbeds;
			fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				f.write_str("at most ten cached embeds")
			}
			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut sequence: A,
			) -> std::result::Result<Self::Value, A::Error> {
				let mut embeds = Vec::new();
				for _ in 0..model::MAX_EMBEDS {
					match sequence.next_element()? {
						Some(embed) => embeds.push(embed),
						None => return Ok(CachedEmbeds(embeds)),
					}
				}
				if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
					return Err(serde::de::Error::custom("cached embed limit"));
				}
				Ok(CachedEmbeds(embeds))
			}
		}
		deserializer.deserialize_seq(Visitor)
	}
}
impl LocalStore {
	/// Opens the SQLite store beneath the selected application-data root.
	pub fn open_default() -> Result<Self> {
		let root = data_dir()?;
		create_data_dir(&root)?;
		Self::open(&root.join("client.sqlite3"))
	}
	pub fn open(path: &Path) -> Result<Self> {
		Self::initialize(Connection::open(path)?)
	}
}
#[cfg(test)]
mod tests {
	#[test]
	fn custom_font_migration_preserves_saved_data_and_accepts_cjk_sizes() {
		let connection = rusqlite::Connection::open_in_memory().unwrap();
		connection.execute_batch("PRAGMA user_version=26;
            CREATE TABLE custom_font(
                singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                name TEXT NOT NULL CHECK(typeof(name)='text' AND length(CAST(name AS BLOB)) BETWEEN 1 AND 128),
                data BLOB NOT NULL CHECK(typeof(data)='blob' AND length(data) BETWEEN 1 AND 8388608));
            INSERT INTO custom_font VALUES(1,'Saved',X'010203');").unwrap();
		let store = super::LocalStore::initialize(connection).unwrap();
		assert_eq!(
			store.custom_font().unwrap(),
			Some(("Saved".into(), vec![1, 2, 3]))
		);
		let bytes = vec![0; 20 * 1024 * 1024];
		store.save_custom_font(Some(("CJK", &bytes))).unwrap();
		assert_eq!(store.custom_font().unwrap().unwrap().1, bytes);
		assert_eq!(
			store.save_custom_font(Some(("Oversized", &vec![0; 32 * 1024 * 1024 + 1]))),
			Err(super::StoreError::Capacity)
		);
		assert_eq!(store.custom_font().unwrap().unwrap().0, "CJK");
	}

	#[test]
	fn data_directory_override_is_absolute_and_never_falls_back() {
		let default = std::env::temp_dir();
		assert!(default.is_absolute());
		assert_eq!(
			resolve_data_dir(None, Some(default.clone())),
			Ok(default.clone())
		);
		let configured = default.join("serein-development");
		assert_eq!(
			resolve_data_dir(Some(configured.clone().into_os_string()), None),
			Ok(configured)
		);
		assert_eq!(
			resolve_data_dir(Some(OsString::new()), Some(default.clone())),
			Err(StoreError::Unavailable)
		);
		assert_eq!(
			resolve_data_dir(Some(OsString::from("relative")), Some(default)),
			Err(StoreError::Unavailable)
		);
	}

	#[cfg(feature = "development-data")]
	#[cfg(unix)]
	#[test]
	fn existing_data_directory_permissions_are_preserved() {
		use std::os::unix::fs::PermissionsExt;
		let root = std::env::temp_dir().join(format!(
			"serein-existing-permissions-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::create_dir(&root).unwrap();
		std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
		create_data_dir(&root).unwrap();
		assert_eq!(
			std::fs::metadata(&root).unwrap().permissions().mode() & 0o777,
			0o755
		);
		std::fs::remove_dir(root).unwrap();
	}

	#[cfg(not(feature = "development-data"))]
	#[test]
	fn changed_rows_preserve_retained_data_and_rollback_invalid_updates() {
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		for (account, channel, id) in [(1, 2, 10), (1, 2, 20), (1, 2, 30), (9, 2, 10), (1, 9, 10)] {
			store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES(?1,?2,?3,'4','Synthetic','original',0,0)", params![account.to_string(), channel.to_string(), id.to_string()]).unwrap();
		}
		let original = store.load_channel(Id(1), Id(2)).unwrap();
		let mut edited = original[1].clone();
		edited.content = "edited".into();
		edited.edited = true;
		let mut added = original[0].clone();
		added.id = Id(40);
		let mut superseded = edited.clone();
		superseded.content = "superseded".into();
		let retained = [Id(10), Id(20), Id(40)];
		store
			.save_changes(
				Id(1),
				Id(2),
				&[superseded, edited.clone(), added.clone()],
				&retained,
				false,
			)
			.unwrap();
		let expected = vec![
			original[0].clone(),
			edited.clone(),
			original[2].clone(),
			added,
		];
		assert!(store.load_channel(Id(1), Id(2)).unwrap() == expected);
		assert_eq!(
			store.load_channel(Id(9), Id(2)).unwrap()[0].content,
			"original"
		);
		assert_eq!(
			store.load_channel(Id(1), Id(9)).unwrap()[0].content,
			"original"
		);
		// Invalid author metadata is discovered after deletions begin: roll them back too.
		edited.author_roles = vec![Id(0)];
		assert_eq!(
			store.save_changes(
				Id(1),
				Id(2),
				std::slice::from_ref(&edited),
				&[Id(20)],
				false
			),
			Err(StoreError::Capacity)
		);
		assert!(store.load_channel(Id(1), Id(2)).unwrap() == expected);
		edited.author_roles.clear();
		assert_eq!(
			store.save_changes(
				Id(1),
				Id(2),
				std::slice::from_ref(&edited),
				&[Id(10)],
				false
			),
			Err(StoreError::Capacity)
		);
		edited.ephemeral = true;
		assert_eq!(
			store.save_changes(
				Id(1),
				Id(2),
				std::slice::from_ref(&edited),
				&retained,
				false
			),
			Err(StoreError::Capacity)
		);
		edited.ephemeral = false;
		// The changed batch fits by itself, but its retained neighbors exceed the window budget.
		edited.content = "x".repeat(MAX_WINDOW_BYTES - edited.bytes() + edited.content.capacity());
		assert_eq!(edited.bytes(), MAX_WINDOW_BYTES);
		assert_eq!(
			store.save_changes(
				Id(1),
				Id(2),
				std::slice::from_ref(&edited),
				&retained,
				false
			),
			Err(StoreError::Capacity)
		);
		assert!(store.load_channel(Id(1), Id(2)).unwrap() == expected);
		store.save_changes(Id(1), Id(2), &[], &[], false).unwrap();
		assert!(store.load_channel(Id(1), Id(2)).unwrap() == expected);
	}

	#[test]
	#[ignore = "manual release benchmark; synthetic in-memory SQLite, not disk or UI latency"]
	fn benchmark_changed_row_save() {
		use std::{hint::black_box, time::Instant};
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		let content = "x".repeat(7000);
		for id in 1..=500 {
			store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2',?1,'4','Synthetic',?2,0,0)", params![id.to_string(), content]).unwrap();
		}
		let messages = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(messages.len(), 500);
		let window_bytes = messages.iter().map(Message::bytes).sum::<usize>();
		assert!(window_bytes <= MAX_WINDOW_BYTES);
		let retained: Vec<_> = messages.iter().map(|message| message.id).collect();
		let mut changed = messages[250].clone();
		drop(messages);
		let mut samples = Vec::new();
		for run in 0..6 {
			let start = Instant::now();
			for _ in 0..200 {
				changed.edited = !changed.edited;
				store
					.save_changes(
						Id(1),
						Id(2),
						black_box(std::slice::from_ref(&changed)),
						black_box(&retained),
						false,
					)
					.unwrap();
			}
			if run != 0 {
				samples.push(start.elapsed());
			}
		}
		samples.sort_unstable();
		println!(
			"200 changed-row saves, 500 rows / {window_bytes} estimated bytes: median {:?}, samples {:?}",
			samples[2], samples
		);
		let loaded = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(loaded.len(), 500);
		assert!(loaded[250] == changed);
	}

	#[test]
	fn channel_order_index_upgrades_existing_cache_and_preserves_unsigned_ids() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		let ids = [9, 10, 99, 100, i64::MAX as u64 + 1, u64::MAX];
		for id in ids.into_iter().rev() {
			store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2',?1,'4','Synthetic','body',0,0)", [id.to_string()]).unwrap();
		}
		store
			.0
			.execute_batch("DROP INDEX messages_channel_order")
			.unwrap();
		let store = LocalStore::initialize(store.0).unwrap();
		for _ in 0..2 {
			assert_eq!(
				store
					.load_channel(Id(1), Id(2))
					.unwrap()
					.iter()
					.map(|m| m.id.0)
					.collect::<Vec<_>>(),
				ids
			);
			assert!(store.load_channel(Id(2), Id(2)).unwrap().is_empty());
		}
		let mut query = store.0.prepare("EXPLAIN QUERY PLAN SELECT id,content FROM messages WHERE account=?1 AND channel=?2 ORDER BY length(id),id LIMIT 500").unwrap();
		let plan = query
			.query_map(["1", "2"], |row| row.get::<_, String>(3))
			.unwrap()
			.collect::<rusqlite::Result<Vec<_>>>()
			.unwrap();
		assert!(
			plan.iter()
				.any(|step| step.contains("messages_channel_order")),
			"{plan:?}"
		);
		assert!(
			!plan.iter().any(|step| step.contains("TEMP B-TREE")),
			"{plan:?}"
		);
	}

	#[test]
	#[ignore = "manual release benchmark; synthetic in-memory SQLite, not disk or UI latency"]
	fn benchmark_channel_load() {
		use std::{hint::black_box, time::Instant};
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		for count in [1, 50, 500] {
			for id in 1..=count {
				store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1',?1,?2,'4','Synthetic','synthetic benchmark message',0,0)", rusqlite::params![count.to_string(), id.to_string()]).unwrap();
			}
			let mut samples = Vec::new();
			for run in 0..6 {
				let start = Instant::now();
				for _ in 0..200 {
					let loaded = store
						.load_channel(black_box(Id(1)), black_box(Id(count)))
						.unwrap();
					assert_eq!(loaded.len(), count as usize);
					black_box(loaded);
				}
				if run != 0 {
					samples.push(start.elapsed());
				}
			}
			samples.sort_unstable();
			println!(
				"200 channel loads, {count} rows: median {:?}, samples {:?}",
				samples[2], samples
			);
		}
	}

	#[test]
	fn projected_unicode_accounts_remain_switchable_without_a_schema_change() {
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		let name = "界".repeat(22);
		let display = "🦀".repeat(model::MAX_PROFILE_NAME_CHARS);
		let account = model::SavedAccount {
			id: Id(9),
			name: model::SavedAccount::label_projection(&name).to_owned(),
			display: Some(model::SavedAccount::label_projection(&display).to_owned()),
			avatar: None,
			discriminator: 0,
			has_token: false,
		};
		assert!(store.save_account(&account).unwrap().is_empty());
		store.set_account_token(account.id, true).unwrap();
		let restored = store.accounts().unwrap();
		assert_eq!(restored.len(), 1);
		assert_eq!(restored[0].id, account.id);
		assert_eq!(restored[0].name, "界".repeat(21));
		assert_eq!(restored[0].label(), "🦀".repeat(16));
		assert!(restored[0].has_token);
	}

	#[test]
	fn switcher_roster_orders_by_last_use_prunes_and_clears_with_the_account() {
		use super::{Id, LocalStore};
		let path =
			std::env::temp_dir().join(format!("serein-accounts-{}.sqlite", std::process::id()));
		let _ = std::fs::remove_file(&path);
		let mut store = LocalStore::open(&path).unwrap();
		let entry = |id: u64| model::SavedAccount {
			id: Id(id),
			name: format!("synthetic{id}"),
			display: Some(format!("Synthetic {id}")),
			avatar: None,
			discriminator: 0,
			has_token: false,
		};
		assert!(store.accounts().unwrap().is_empty());
		// One extra account beyond the bound: the least recently used entry is pruned.
		let mut pruned = Vec::new();
		for id in 1..=(model::MAX_SAVED_ACCOUNTS as u64 + 1) {
			pruned.extend(store.save_account(&entry(id)).unwrap());
		}
		assert_eq!(pruned, vec![Id(1)]);
		let accounts = store.accounts().unwrap();
		assert_eq!(accounts.len(), model::MAX_SAVED_ACCOUNTS);
		assert_eq!(accounts[0].id, Id(model::MAX_SAVED_ACCOUNTS as u64 + 1));
		assert!(!accounts.iter().any(|account| account.id == Id(1)));
		// Re-saving moves an account back to the front and updates its identity.
		let mut renamed = entry(2);
		renamed.display = Some("Renamed".into());
		assert!(store.save_account(&renamed).unwrap().is_empty());
		let accounts = store.accounts().unwrap();
		assert_eq!(accounts[0], renamed);
		// The token flag is owned by set_account_token: an identity refresh never claims one,
		// so the client cannot be tricked into skipping the write that backs the switcher.
		assert!(!store.accounts().unwrap()[0].has_token);
		store.set_account_token(Id(2), true).unwrap();
		assert!(
			store
				.accounts()
				.unwrap()
				.iter()
				.find(|account| account.id == Id(2))
				.unwrap()
				.has_token
		);
		let mut renamed_again = renamed.clone();
		renamed_again.display = Some("Renamed twice".into());
		assert!(store.save_account(&renamed_again).unwrap().is_empty());
		let refreshed = store.accounts().unwrap();
		let refreshed = refreshed
			.iter()
			.find(|account| account.id == Id(2))
			.unwrap();
		assert_eq!(refreshed.display.as_deref(), Some("Renamed twice"));
		assert!(refreshed.has_token);
		store.set_account_token(Id(2), false).unwrap();
		assert!(
			!store
				.accounts()
				.unwrap()
				.iter()
				.find(|account| account.id == Id(2))
				.unwrap()
				.has_token
		);
		store.set_account_token(Id(2), true).unwrap();
		// A row that could never be offered — written outside this client — neither occupies a
		// slot in the bounded roster nor survives the next write.
		store
			.0
			.execute(
				"INSERT INTO accounts(account,name,display,avatar,discriminator,touched,has_token)
                 VALUES('0','damaged',NULL,NULL,0,unixepoch('subsec')*1000+5000,0)",
				[],
			)
			.unwrap();
		let listed = store.accounts().unwrap();
		// The damaged row is newest, so an unfiltered LIMIT would have dropped a real account.
		assert_eq!(listed.len(), model::MAX_SAVED_ACCOUNTS);
		assert!(listed.iter().all(|account| account.id != Id(0)));
		for id in 2..=(model::MAX_SAVED_ACCOUNTS as u64 + 1) {
			assert!(listed.iter().any(|account| account.id == Id(id)), "{id}");
		}
		assert!(store.save_account(&entry(2)).unwrap().is_empty());
		let damaged: i64 = store
			.0
			.query_row(
				"SELECT COUNT(*) FROM accounts WHERE account='0'",
				[],
				|row| row.get(0),
			)
			.unwrap();
		assert_eq!(damaged, 0, "a damaged row is dropped, not counted");
		// Oversized identities never reach the table, and forgetting an account drops its row.
		let mut invalid = entry(3);
		invalid.name = "n".repeat(65);
		assert!(store.save_account(&invalid).is_err());
		store.forget_account(Id(2)).unwrap();
		let accounts = store.accounts().unwrap();
		assert!(!accounts.iter().any(|account| account.id == Id(2)));
		assert_eq!(accounts.len(), model::MAX_SAVED_ACCOUNTS - 1);
		drop(store);
		let _ = std::fs::remove_file(&path);
	}
	#[test]
	fn roster_upgrade_keeps_existing_accounts_switchable_without_rewriting_their_entries() {
		use super::{Id, LocalStore};
		let path = std::env::temp_dir().join(format!(
			"serein-roster-upgrade-{}.sqlite",
			std::process::id()
		));
		let _ = std::fs::remove_file(&path);
		let mut store = LocalStore::open(&path).unwrap();
		store
			.save_account(&model::SavedAccount {
				id: Id(7),
				name: "synthetic".into(),
				display: None,
				avatar: None,
				discriminator: 0,
				has_token: false,
			})
			.unwrap();
		// Reopen as a build that predates the flag, then upgrade again.
		store
			.0
			.execute_batch("ALTER TABLE accounts DROP COLUMN has_token;")
			.unwrap();
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		let accounts = store.accounts().unwrap();
		assert_eq!(accounts.len(), 1);
		assert!(accounts[0].has_token, "upgraded rows keep their entry");
		drop(store);
		let _ = std::fs::remove_file(&path);
	}

	#[test]
	fn forwarded_snapshot_survives_cache_reopen_and_upgrade() {
		let path =
			std::env::temp_dir().join(format!("serein-forwarded-{}.sqlite", std::process::id()));
		let _ = std::fs::remove_file(&path);
		let store = LocalStore::open(&path).unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','100','4','Synthetic','snapshot text',0,0)", []).unwrap();
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN forwarded; PRAGMA user_version=15;")
			.unwrap();
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		let mut messages = store.load_channel(Id(1), Id(2)).unwrap();
		assert!(!messages[0].forwarded);
		messages[0].forwarded = true;
		store.save_channel(Id(1), Id(2), &messages, false).unwrap();
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		let restored = store.load_channel(Id(1), Id(2)).unwrap();
		assert!(restored[0].forwarded);
		assert_eq!(restored[0].content, "snapshot text");
		assert!(
			store
				.0
				.execute("UPDATE messages SET forwarded=2", [])
				.is_err()
		);
		drop(store);
		std::fs::remove_file(path).unwrap();
	}
	#[test]
	fn webhook_author_survives_cache_and_legacy_migration() {
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','100','3','Synthetic webhook','body',0,0)", []).unwrap();
		let mut message = store.load_channel(Id(1), Id(2)).unwrap().remove(0);
		message.author.webhook = true;
		store.save_channel(Id(1), Id(2), &[message], false).unwrap();
		assert!(store.load_channel(Id(1), Id(2)).unwrap()[0].author.webhook);
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN webhook; PRAGMA user_version=12;")
			.unwrap();
		let store = LocalStore::initialize(store.0).unwrap();
		assert!(!store.load_channel(Id(1), Id(2)).unwrap()[0].author.webhook);
		assert!(
			store
				.0
				.execute("UPDATE messages SET webhook=2", [])
				.is_err()
		);
		store
			.0
			.execute_batch("PRAGMA ignore_check_constraints=ON; UPDATE messages SET webhook=2;")
			.unwrap();
		assert!(matches!(
			store.load_channel(Id(1), Id(2)),
			Err(StoreError::Incompatible)
		));
	}
	#[test]
	fn account_kinds_survive_cache_and_migrate_legacy_without_guessing() {
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','100','3','Synthetic','body',0,0)", []).unwrap();
		let mut message = store.load_channel(Id(1), Id(2)).unwrap().remove(0);
		for kind in [
			model::AccountKind::Human,
			model::AccountKind::Bot,
			model::AccountKind::App,
			model::AccountKind::VerifiedBot,
		] {
			message.author.kind = kind;
			store
				.save_channel(Id(1), Id(2), &[message.clone()], false)
				.unwrap();
			assert_eq!(
				store.load_channel(Id(1), Id(2)).unwrap()[0].author.kind,
				kind
			);
		}
		message.author.kind = model::AccountKind::Human;
		store
			.save_channel(Id(1), Id(2), &[message.clone()], false)
			.unwrap();
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN account_kind; ALTER TABLE messages DROP COLUMN verified_bot; PRAGMA user_version=13;")
			.unwrap();
		let store = LocalStore::initialize(store.0).unwrap();
		assert_eq!(
			store.load_channel(Id(1), Id(2)).unwrap()[0].author.kind,
			model::AccountKind::Human
		);
		assert!(
			store
				.0
				.execute("UPDATE messages SET account_kind=3", [])
				.is_err()
		);
		assert!(
			store
				.0
				.execute("UPDATE messages SET verified_bot=2", [])
				.is_err()
		);
		store
			.0
			.execute("UPDATE messages SET verified_bot=1", [])
			.unwrap();
		assert!(matches!(
			store.load_channel(Id(1), Id(2)),
			Err(StoreError::Incompatible)
		));
		store
			.0
			.execute("UPDATE messages SET verified_bot=0", [])
			.unwrap();
		store
			.0
			.execute_batch(
				"PRAGMA ignore_check_constraints=ON; UPDATE messages SET account_kind=3;",
			)
			.unwrap();
		assert!(matches!(
			store.load_channel(Id(1), Id(2)),
			Err(StoreError::Incompatible)
		));
	}
	#[test]
	fn legacy_backend_migration_and_future_choices_preserve_other_preferences() {
		use model::voice_settings::{
			EncoderPreset, VideoBandwidth, VideoCodec, VideoFrameRate, VideoResolution,
			VideoSettings,
		};
		for backend in ["Stable", "Experimental", "FutureBackend"] {
			let value = serde_json::json!({"backend":backend,"codec":"H265","camera_resolution":"P2160","camera_frame_rate":"Fps60"});
			let settings: VideoSettings = serde_json::from_value(value).unwrap();
			assert_eq!(settings.codec, VideoCodec::H265);
			assert_eq!(settings.camera_resolution, VideoResolution::P2160);
			assert_eq!(settings.camera_frame_rate, VideoFrameRate::Fps60);
			assert_eq!(settings.bandwidth, VideoBandwidth::Medium);
			assert_eq!(settings.quality, EncoderPreset::Automatic);
			assert!(
				serde_json::to_value(settings)
					.unwrap()
					.get("backend")
					.is_none()
			);
		}
		let future: VideoSettings = serde_json::from_str(
			r#"{"codec":"Av1","bandwidth":"Future","quality":"Future","camera_frame_rate":"Fps30"}"#,
		)
		.unwrap();
		assert_eq!(future.bandwidth, VideoBandwidth::Medium);
		assert_eq!(future.quality, EncoderPreset::Automatic);
		assert_eq!(future.codec, VideoCodec::Av1);
		assert_eq!(future.camera_frame_rate, VideoFrameRate::Fps30);
	}

	#[test]
	fn camera_denoising_preferences_round_trip_and_unknown_values_default_safely() {
		use model::voice_settings::{CameraDenoising, VideoCodec, VideoFrameRate};
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		for (mode, saved) in [(CameraDenoising::Off, "Off"), (CameraDenoising::On, "On")] {
			let mut value = AppPreferences {
				input_percent: 75,
				..Default::default()
			};
			value.video_settings.camera_denoising = mode;
			assert_eq!(serde_json::to_value(mode).unwrap(), saved);
			store.save_app_preferences(&value).unwrap();
			assert_eq!(store.app_preferences().unwrap(), value);
		}
		let legacy: AppPreferences = serde_json::from_str("{}").unwrap();
		assert_eq!(legacy.video_settings.camera_denoising, CameraDenoising::Off);
		for saved in ["Auto", "Future"] {
			let value: AppPreferences = serde_json::from_value(serde_json::json!({
				"input_percent": 75,
				"video_settings": {
					"camera_denoising": saved,
					"codec": "Av1",
					"camera_frame_rate": "Fps30"
				}
			}))
			.unwrap();
			assert_eq!(value.video_settings.camera_denoising, CameraDenoising::Off);
			assert_eq!(value.input_percent, 75);
			assert_eq!(value.video_settings.codec, VideoCodec::Av1);
			assert_eq!(
				value.video_settings.camera_frame_rate,
				VideoFrameRate::Fps30
			);
			store.save_app_preferences(&value).unwrap();
			assert_eq!(store.app_preferences().unwrap(), value);
		}
		assert!(
			serde_json::from_str::<AppPreferences>(
				r#"{"video_settings":{"camera_denoising":true}}"#,
			)
			.is_err()
		);
	}

	#[test]
	fn bandwidth_and_vendor_quality_choices_round_trip_without_resetting_preferences() {
		use model::voice_settings::{EncoderPreset, VideoBandwidth};
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		for bandwidth in VideoBandwidth::ALL {
			for quality in EncoderPreset::ALL {
				let mut value = AppPreferences {
					input_percent: 75,
					..Default::default()
				};
				value.video_settings.bandwidth = bandwidth;
				value.video_settings.quality = quality;
				store.save_app_preferences(&value).unwrap();
				assert_eq!(store.app_preferences().unwrap(), value);
			}
		}
	}

	#[test]
	fn video_preferences_migrate_and_round_trip_all_ffmpeg_codecs() {
		use model::voice_settings::{VideoCodec, VideoFrameRate, VideoResolution, VideoSettings};
		let legacy: AppPreferences = serde_json::from_str("{}").unwrap();
		assert_eq!(legacy.video_settings, VideoSettings::default());
		let existing: AppPreferences =
			serde_json::from_str(r#"{"video_settings":{"backend":"Experimental","codec":"H265"}}"#)
				.unwrap();
		assert_eq!(
			existing.video_settings.camera_resolution,
			VideoResolution::P480
		);
		assert_eq!(
			serde_json::from_str::<AppPreferences>(
				r#"{"video_settings":{"camera_resolution":"P8640"}}"#,
			)
			.unwrap()
			.video_settings
			.camera_resolution,
			VideoResolution::P480
		);
		assert_eq!(
			existing.video_settings.camera_frame_rate,
			VideoFrameRate::Fps15
		);
		assert_eq!(
			serde_json::from_str::<AppPreferences>(
				r#"{"video_settings":{"camera_frame_rate":"Fps120"}}"#,
			)
			.unwrap()
			.video_settings
			.camera_frame_rate,
			VideoFrameRate::Fps15
		);
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1] {
			for camera_resolution in VideoResolution::ALL {
				for camera_frame_rate in VideoFrameRate::ALL {
					let value = AppPreferences {
						video_settings: VideoSettings {
							codec,
							camera_resolution,
							camera_frame_rate,
							..Default::default()
						},
						..Default::default()
					};
					store.save_app_preferences(&value).unwrap();
					assert_eq!(store.app_preferences().unwrap(), value);
				}
			}
		}

		assert_eq!(
			store.app_preferences().unwrap().video_settings.codec,
			VideoCodec::Av1
		);
	}

	#[test]
	fn future_video_preferences_preserve_known_choices_and_unrelated_settings() {
		use model::voice_settings::{VideoCodec, VideoFrameRate, VideoResolution};
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		let original = AppPreferences {
			language: Some("en-US".into()),
			input_percent: 75,
			video_settings: model::voice_settings::VideoSettings {
				codec: VideoCodec::H265,
				camera_resolution: VideoResolution::P2160,
				camera_frame_rate: VideoFrameRate::Fps60,
				..Default::default()
			},
			..Default::default()
		};
		for (field, future) in [
			("backend", "FutureBackend"),
			("codec", "FutureCodec"),
			("camera_resolution", "P8640"),
			("camera_frame_rate", "Fps120"),
		] {
			let mut serialized = serde_json::to_value(&original).unwrap();
			serialized["video_settings"][field] = future.into();
			store.0.execute(
				"INSERT INTO app_preferences VALUES(1,?1) ON CONFLICT(singleton) DO UPDATE SET value=excluded.value",
				[serialized.to_string()],
			).unwrap();
			let mut expected = original.clone();
			match field {
				"backend" => {}
				"codec" => expected.video_settings.codec = VideoCodec::H264,
				"camera_resolution" => {
					expected.video_settings.camera_resolution = VideoResolution::P480
				}
				_ => expected.video_settings.camera_frame_rate = VideoFrameRate::Fps15,
			}
			assert_eq!(store.app_preferences().unwrap(), expected, "{field}");
			store.save_app_preferences(&expected).unwrap();
			assert_eq!(store.app_preferences().unwrap(), expected);
			for malformed in [
				serde_json::Value::Null,
				120.into(),
				true.into(),
				serde_json::json!({}),
			] {
				serialized["video_settings"][field] = malformed;
				assert!(
					serde_json::from_value::<AppPreferences>(serialized.clone()).is_err(),
					"{field}"
				);
			}
		}
	}

	#[test]
	fn emoticon_conversion_defaults_off_and_persists_across_reopens() {
		let legacy: AppPreferences = serde_json::from_str("{}").unwrap();
		assert!(!legacy.convert_emoticons);
		let root = std::env::temp_dir().join(format!("serein-emoticons-{}", std::process::id()));
		std::fs::create_dir(&root).unwrap();
		let path = root.join("preferences.sqlite3");
		for enabled in [true, false] {
			{
				let store = LocalStore::open(&path).unwrap();
				let mut preferences = store.app_preferences().unwrap();
				assert_eq!(preferences.convert_emoticons, !enabled);
				preferences.convert_emoticons = enabled;
				store.save_app_preferences(&preferences).unwrap();
			}
			let store = LocalStore::open(&path).unwrap();
			assert_eq!(store.app_preferences().unwrap().convert_emoticons, enabled);
		}
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn old_device_settings_gain_unread_bindings_without_replacing_existing_choices() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		store
			.0
			.execute(
				"INSERT INTO app_preferences VALUES(1,?1)",
				[r#"{"keybinds":{"search_conversation":{"key":"G","modifiers":1}}}"#],
			)
			.unwrap();
		let preferences = store.app_preferences().unwrap();
		assert_eq!(
			preferences.keybinds.search_conversation,
			model::KeyChord::new("G", model::keybinds::PRIMARY)
		);
		assert_eq!(
			preferences.keybinds.next_unread_conversation,
			model::KeyChord::new("ArrowDown", model::keybinds::ALT | model::keybinds::SHIFT)
		);
		assert_eq!(
			preferences.keybinds.previous_unread_conversation,
			model::KeyChord::new("ArrowUp", model::keybinds::ALT | model::keybinds::SHIFT)
		);
		assert_eq!(
			preferences.keybinds.jump_to_unread,
			model::KeyChord::new("U", model::keybinds::PRIMARY | model::keybinds::SHIFT)
		);
		assert!(preferences.keybinds.is_valid());
		for action in [
			model::KeybindAction::NextUnreadConversation,
			model::KeybindAction::PreviousUnreadConversation,
			model::KeybindAction::JumpToUnread,
		] {
			assert!(!action.is_global());
		}
		store.save_app_preferences(&preferences).unwrap();
		assert_eq!(store.app_preferences().unwrap(), preferences);
	}
	#[test]
	fn timezone_preferences_round_trip_and_invalid_values_recover_independently() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		for zone in [
			model::TimeZonePreference::System,
			model::TimeZonePreference::Named("Europe/London".into()),
			model::TimeZonePreference::FixedOffset(345),
			model::TimeZonePreference::FixedOffset(-210),
		] {
			let preferences = AppPreferences {
				time_zone: zone.clone(),
				time_format: model::TimeFormat::TwelveHour,
				..Default::default()
			};
			store.save_app_preferences(&preferences).unwrap();
			let reopened = store.app_preferences().unwrap();
			assert_eq!(reopened.time_zone, zone);
			assert_eq!(reopened.time_format, model::TimeFormat::TwelveHour);
		}
		for invalid in [
			r#""offset:841""#,
			r#""offset:broken""#,
			r#""iana:../secret""#,
			r#""future:unknown""#,
			"null",
			"42",
			r#"{"future":true}"#,
		] {
			let preferences: AppPreferences = serde_json::from_str(&format!(
				r#"{{"time_zone":{invalid},"time_format":"24-hour","transparency":37}}"#
			))
			.unwrap();
			assert_eq!(
				preferences.time_zone,
				model::TimeZonePreference::System,
				"{invalid}"
			);
			assert_eq!(preferences.time_format, model::TimeFormat::TwentyFourHour);
			assert_eq!(preferences.transparency, 37);
		}
		let legacy: AppPreferences = serde_json::from_str("{}").unwrap();
		assert_eq!(legacy.time_zone, model::TimeZonePreference::System);
		let invalid = AppPreferences {
			time_zone: model::TimeZonePreference::FixedOffset(841),
			..Default::default()
		};
		assert!(store.save_app_preferences(&invalid).is_err());
	}

	#[test]
	fn app_preferences_round_trip_and_reject_invalid_replacement() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		assert_eq!(store.app_preferences().unwrap(), AppPreferences::default());
		let legacy: AppPreferences =
			serde_json::from_str(r#"{"voice_noise_suppression":true}"#).unwrap();
		assert!(legacy.voice_processing.is_none());
		assert!(legacy.media_output.is_none());
		assert!(legacy.voice_noise_suppression);
		// Existing serialized bindings omit the newly optional diagnostics shortcut.
		let mut old_bindings = serde_json::to_value(model::Keybinds::default()).unwrap();
		old_bindings
			.as_object_mut()
			.unwrap()
			.remove("copy_issue_diagnostics");
		let restored: model::Keybinds = serde_json::from_value(old_bindings).unwrap();
		assert!(restored.is_valid());
		assert_eq!(restored.copy_issue_diagnostics, model::KeyChord::new("", 0));
		assert_eq!(
			restored.send_message,
			model::Keybinds::default().send_message
		);
		let mut value = AppPreferences {
			language: Some("cs".into()),
			notifications_enabled: true,
			hide_title_bar: true,
			primary_color: Some([80, 120, 220]),
			reduce_interface_motion: true,
			transparency_blur: true,
			transparency: 30,
			blur: 60,
			notification_options: model::notification_preferences::Device {
				current_channel: true,
				disable_sounds: true,
				unread_badge: false,
				..Default::default()
			},
			voice_noise_suppression: true,
			voice_processing: Some(model::voice_settings::VoiceProcessing::from_legacy(true)),
			voice_muted: true,
			voice_deafened: true,
			voice_input: Some("synthetic microphone".into()),
			media_output: Some("synthetic media speakers".into()),
			output_percent: 75,
			gpu_preference: model::GpuPreference::PowerSaving,
			..Default::default()
		};
		store.save_app_preferences(&value).unwrap();
		assert_eq!(store.app_preferences().unwrap(), value);
		value
			.voice_processing
			.as_mut()
			.unwrap()
			.custom
			.sensitivity_db = Some(-81);
		assert!(store.save_app_preferences(&value).is_err());
		value
			.voice_processing
			.as_mut()
			.unwrap()
			.custom
			.sensitivity_db = None;
		value
			.voice_processing
			.as_mut()
			.unwrap()
			.custom
			.suppression_level = 4;
		assert!(store.save_app_preferences(&value).is_err());
		value
			.voice_processing
			.as_mut()
			.unwrap()
			.custom
			.suppression_level = 0;
		value.transparency = 101;
		assert!(store.save_app_preferences(&value).is_err());
		value.transparency = 30;
		value.input_percent = 201;
		assert!(store.save_app_preferences(&value).is_err());
		assert_eq!(store.app_preferences().unwrap().input_percent, 100);
		value.input_percent = 100;
		value.media_output = Some("x".repeat(1025));
		assert!(store.save_app_preferences(&value).is_err());
		value.media_output = Some("synthetic media speakers".into());
		value.voice_input = Some("x".repeat(1025));
		assert!(store.save_app_preferences(&value).is_err());
		assert_eq!(
			store.app_preferences().unwrap().voice_input.as_deref(),
			Some("synthetic microphone")
		);
		assert_eq!(
			store.app_preferences().unwrap().gpu_preference,
			model::GpuPreference::PowerSaving
		);
		// Older rows show no cosmetics (they are opt-in) and keep the historical budgets.
		let legacy: AppPreferences = serde_json::from_str("{}").unwrap();
		assert_eq!(legacy.cosmetics, model::CosmeticPreferences::default());
		assert_eq!(legacy.image_cache, model::CachePreferences::default());
		assert_eq!(legacy.time_format, model::TimeFormat::Auto);
		for (key, format) in [
			("12-hour", model::TimeFormat::TwelveHour),
			("24-hour", model::TimeFormat::TwentyFourHour),
			("36-hour", model::TimeFormat::Auto),
		] {
			let saved: AppPreferences =
				serde_json::from_str(&format!(r#"{{"time_format":"{key}"}}"#)).unwrap();
			assert_eq!(saved.time_format, format);
		}
		let partial: AppPreferences = serde_json::from_str(
			r#"{"cosmetics":{"nameplates":true},"image_cache":{"media_mib":512}}"#,
		)
		.unwrap();
		assert!(partial.cosmetics.nameplates && !partial.cosmetics.decorations);
		assert_eq!(partial.image_cache.artwork_mib, 128);
		let mut cache = AppPreferences {
			cosmetics: model::CosmeticPreferences::SHOWN,
			..Default::default()
		};
		cache.image_cache.media_mib = 2048;
		store.save_app_preferences(&cache).unwrap();
		assert_eq!(store.app_preferences().unwrap(), cache);
		cache.image_cache.media_mib = 3000;
		assert!(store.save_app_preferences(&cache).is_err());
		assert_eq!(store.app_preferences().unwrap().image_cache.media_mib, 2048);
		value.voice_input = None;
		value.language = Some("../cs".into());
		assert!(store.save_app_preferences(&value).is_err());

		{
			let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
			store
				.0
				.execute(
					"INSERT INTO app_preferences VALUES(1,?1)",
					[r#"{"gpu_preference":"quantum-gpu"}"#],
				)
				.unwrap();
			assert_eq!(
				store.app_preferences().unwrap().gpu_preference,
				model::GpuPreference::Automatic
			);
		}
	}
	use super::*;
	#[test]
	fn reply_deletion_schema_migrates_reopens_and_rejects_invalid_markers() {
		let root = std::env::temp_dir().join(format!(
			"serein-synthetic-reply-schema-{}",
			std::process::id()
		));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let mut store = LocalStore::open(&path).unwrap();
		store.save_draft(Id(1), Id(2), "preserved draft").unwrap();
		let prefs = ReadingPreferences {
			zoom_percent: 125,
			..Default::default()
		};
		store.save_reading_preferences(prefs).unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported,message_kind,extra_content,reply) VALUES('1','2','100','4','Synthetic','reply body',0,0,19,31,'50')", []).unwrap();
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN reply_deleted; PRAGMA user_version=9;")
			.unwrap();
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		let mut messages = store.load_channel(Id(1), Id(2)).unwrap();
		assert!(!messages[0].reply_deleted);
		assert_eq!(messages[0].reply_to, Some(Id(50)));
		assert_eq!(messages[0].kind, 19);
		assert_eq!(messages[0].extra_content.bits(), 31);
		assert_eq!(store.reading_preferences().unwrap(), prefs);
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "preserved draft");
		messages[0].reply_deleted = true;
		store.save_channel(Id(1), Id(2), &messages, false).unwrap();
		for target in [None, Some(Id(0)), Some(Id(100)), Some(Id(101))] {
			messages[0].reply_to = target;
			assert_eq!(
				store.save_channel(Id(1), Id(2), &messages, false),
				Err(StoreError::Capacity)
			);
		}
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		let messages = store.load_channel(Id(1), Id(2)).unwrap();
		assert!(messages[0].reply_deleted);
		assert_eq!(messages[0].reply_to, Some(Id(50)));
		assert_eq!(messages[0].content, "reply body");
		let version: u32 = store
			.0
			.pragma_query_value(None, "user_version", |row| row.get(0))
			.unwrap();
		assert_eq!(version, NATIVE_SCHEMA);
		for invalid in ["-1", "2", "1.5", "'bad'"] {
			assert!(
				store
					.0
					.execute(&format!("UPDATE messages SET reply_deleted={invalid}"), [])
					.is_err()
			);
		}
		// SQLite files remain untrusted even if constraints were deliberately disabled.
		store
			.0
			.execute_batch(
				"PRAGMA ignore_check_constraints=ON; UPDATE messages SET reply_deleted=2;",
			)
			.unwrap();
		assert!(matches!(
			store.load_channel(Id(1), Id(2)),
			Err(StoreError::Incompatible)
		));
		store
			.0
			.execute_batch("UPDATE messages SET reply_deleted=1,reply='100';")
			.unwrap();
		assert!(matches!(
			store.load_channel(Id(1), Id(2)),
			Err(StoreError::Incompatible)
		));
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}
	#[test]
	fn divergent_schema_seven_and_eight_preserve_union_after_reopen() {
		let root = std::env::temp_dir().join(format!(
			"serein-synthetic-union-schema-{}",
			std::process::id()
		));
		std::fs::create_dir_all(&root).unwrap();
		let preferences = ReadingPreferences {
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
		for (name, legacy, expected_kind, expected_markers, expected_preferences) in [
			(
				"system7",
				"ALTER TABLE messages DROP COLUMN extra_content; DROP TABLE reading_preferences; PRAGMA user_version=7;",
				7,
				0,
				ReadingPreferences::default(),
			),
			(
				"markers7",
				"ALTER TABLE messages DROP COLUMN message_kind; DROP TABLE reading_preferences; PRAGMA user_version=7;",
				255,
				31,
				ReadingPreferences::default(),
			),
			(
				"reading8",
				"ALTER TABLE messages DROP COLUMN message_kind; PRAGMA user_version=8;",
				255,
				31,
				preferences,
			),
		] {
			let path = root.join(format!("{name}.sqlite3"));
			let mut store = LocalStore::open(&path).unwrap();
			store.save_draft(Id(1), Id(2), "preserved draft").unwrap();
			store.save_appearance(Appearance::Dark).unwrap();
			store.save_reading_preferences(preferences).unwrap();
			store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported,message_kind,extra_content) VALUES('1','2','3','4','Synthetic','preserved body',0,1,7,31)", []).unwrap();
			store.0.execute_batch(legacy).unwrap();
			drop(store);

			let mut store = LocalStore::open(&path).unwrap();
			let version: u32 = store
				.0
				.pragma_query_value(None, "user_version", |row| row.get(0))
				.unwrap();
			assert_eq!(version, NATIVE_SCHEMA);
			let mut messages = store.load_channel(Id(1), Id(2)).unwrap();
			assert_eq!(messages[0].kind, expected_kind);
			assert_eq!(messages[0].extra_content.bits(), expected_markers);
			assert_eq!(messages[0].content, "preserved body");
			assert_eq!(store.reading_preferences().unwrap(), expected_preferences);
			assert_eq!(store.appearance().unwrap(), Appearance::Dark);
			assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "preserved draft");
			messages[0].kind = 9;
			messages[0].extra_content = model::ExtraContent::from_bits(31).unwrap();
			store.save_channel(Id(1), Id(2), &messages, false).unwrap();
			store
				.save_reading_preferences(ReadingPreferences::default())
				.unwrap();
			drop(store);

			let store = LocalStore::open(&path).unwrap();
			let messages = store.load_channel(Id(1), Id(2)).unwrap();
			assert_eq!(messages[0].kind, 9);
			assert_eq!(messages[0].extra_content.bits(), 31);
			assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "preserved draft");
			assert_eq!(store.appearance().unwrap(), Appearance::Dark);
			assert_eq!(
				store.reading_preferences().unwrap(),
				ReadingPreferences::default()
			);
		}
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn union_migration_failure_rolls_back_columns_and_schema_version() {
		let root = std::env::temp_dir().join(format!(
			"serein-synthetic-union-rollback-{}",
			std::process::id()
		));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let store = LocalStore::open(&path).unwrap();
		store.0.execute_batch("ALTER TABLE messages DROP COLUMN extra_content; ALTER TABLE messages DROP COLUMN message_kind; PRAGMA user_version=7;").unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','3','4','Synthetic','preserved body',0,1)", []).unwrap();
		store.0.execute_batch("CREATE TRIGGER reject_kind_migration BEFORE UPDATE ON messages BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
		drop(store);
		assert!(matches!(
			LocalStore::open(&path),
			Err(StoreError::Unavailable)
		));
		let connection = Connection::open(&path).unwrap();
		let version: u32 = connection
			.pragma_query_value(None, "user_version", |row| row.get(0))
			.unwrap();
		assert_eq!(version, 7);
		let columns: u32 = connection.query_row("SELECT count(*) FROM pragma_table_info('messages') WHERE name IN ('extra_content','message_kind')", [], |row| row.get(0)).unwrap();
		assert_eq!(columns, 0);
		connection
			.execute_batch("DROP TRIGGER reject_kind_migration;")
			.unwrap();
		let store = LocalStore::initialize(connection).unwrap();
		let messages = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(messages[0].content, "preserved body");
		assert_eq!(messages[0].kind, 255);
		assert_eq!(messages[0].extra_content.bits(), 0);
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn message_kind_migration_round_trip_and_bounds() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN message_kind; PRAGMA user_version=6;")
			.unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','3','4','Synthetic','',0,1),('1','2','4','4','Synthetic','body',0,0)", []).unwrap();
		let mut store = LocalStore::initialize(store.0).unwrap();
		let mut messages = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(messages[0].kind, 255);
		assert_eq!(messages[1].kind, 0);
		messages[0].kind = 7;
		store.save_channel(Id(1), Id(2), &messages, false).unwrap();
		let store = LocalStore::initialize(store.0).unwrap();
		assert_eq!(store.load_channel(Id(1), Id(2)).unwrap()[0].kind, 7);
		for value in ["-1", "256", "1.5", "'invalid'"] {
			assert!(
				store
					.0
					.execute(&format!("UPDATE messages SET message_kind={value}"), [])
					.is_err()
			);
		}
		// Column detection also handles another feature using this schema version.
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN message_kind; PRAGMA user_version=7;")
			.unwrap();
		let store = LocalStore::initialize(store.0).unwrap();
		assert_eq!(store.load_channel(Id(1), Id(2)).unwrap()[0].kind, 255);
	}
	#[test]
	fn minimize_to_tray_is_bounded_opt_out_surviving_restart_and_logout() {
		let root = std::env::temp_dir().join(format!(
			"serein-synthetic-minimize-to-tray-{}",
			std::process::id()
		));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let store = LocalStore::open(&path).unwrap();
		assert!(store.minimize_to_tray().unwrap());
		store
			.0
			.execute_batch("DROP TABLE minimize_to_tray;")
			.unwrap();
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		assert!(store.minimize_to_tray().unwrap());
		store.save_minimize_to_tray(false).unwrap();
		store.save_minimize_to_tray(false).unwrap();
		assert!(
			store
				.0
				.execute("INSERT INTO minimize_to_tray VALUES(2,0)", [])
				.is_err()
		);
		assert!(
			store
				.0
				.execute("UPDATE minimize_to_tray SET enabled=2", [])
				.is_err()
		);
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		assert!(!store.minimize_to_tray().unwrap());
		store.forget_account(Id(1)).unwrap();
		assert!(!store.minimize_to_tray().unwrap());
		store.0.execute_batch("PRAGMA query_only=ON;").unwrap();
		assert_eq!(
			store.save_minimize_to_tray(true),
			Err(StoreError::Unavailable)
		);
		assert!(!store.minimize_to_tray().unwrap());
		store
			.0
			.execute_batch("PRAGMA query_only=OFF; PRAGMA ignore_check_constraints=ON;")
			.unwrap();
		for invalid in ["2", "-1", "0.5", "'invalid'", "x'01'"] {
			store
				.0
				.execute(
					&format!("UPDATE minimize_to_tray SET enabled={invalid}"),
					[],
				)
				.unwrap();
			assert_eq!(store.minimize_to_tray(), Err(StoreError::Incompatible));
		}
		store.save_minimize_to_tray(true).unwrap();
		let count: u32 = store
			.0
			.query_row("SELECT count(*) FROM minimize_to_tray", [], |row| {
				row.get(0)
			})
			.unwrap();
		assert_eq!(count, 0);
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		assert!(store.minimize_to_tray().unwrap());
		store
			.0
			.execute_batch("DROP TABLE minimize_to_tray;")
			.unwrap();
		assert_eq!(store.minimize_to_tray(), Err(StoreError::Unavailable));
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn game_activity_defaults_migrates_reopens_and_survives_logout() {
		let root = std::env::temp_dir().join(format!(
			"serein-synthetic-game-activity-{}",
			std::process::id()
		));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let mut store = LocalStore::open(&path).unwrap();
		assert!(!store.game_activity_enabled().unwrap());
		store.save_draft(Id(1), Id(2), "Synthetic draft").unwrap();
		store.0.execute_batch("DROP TABLE game_activity;").unwrap();
		drop(store);

		let store = LocalStore::open(&path).unwrap();
		assert!(!store.game_activity_enabled().unwrap());
		store.save_game_activity_enabled(true).unwrap();
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		assert!(store.game_activity_enabled().unwrap());
		store.forget_account(Id(1)).unwrap();
		assert!(store.game_activity_enabled().unwrap());
		assert!(store.load_drafts(Id(1)).unwrap().is_empty());
		store.0.execute_batch("PRAGMA query_only=ON;").unwrap();
		assert_eq!(
			store.save_game_activity_enabled(false),
			Err(StoreError::Unavailable)
		);
		assert!(store.game_activity_enabled().unwrap());
		store.0.execute_batch("PRAGMA query_only=OFF;").unwrap();
		store.save_game_activity_enabled(false).unwrap();
		let count: u32 = store
			.0
			.query_row("SELECT count(*) FROM game_activity", [], |row| row.get(0))
			.unwrap();
		assert_eq!(count, 0);
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		assert!(!store.game_activity_enabled().unwrap());
		drop(store);
		std::fs::remove_dir_all(root).unwrap();

		{
			let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
			store.save_game_activity_enabled(true).unwrap();
			assert!(
				store
					.0
					.execute("INSERT INTO game_activity VALUES(2,1)", [])
					.is_err()
			);
			assert!(
				store
					.0
					.execute("UPDATE game_activity SET enabled=2", [])
					.is_err()
			);
			store
				.0
				.execute_batch("PRAGMA ignore_check_constraints=ON;")
				.unwrap();
			for invalid in ["2", "-1", "0.5", "'invalid'", "x'01'"] {
				store
					.0
					.execute(&format!("UPDATE game_activity SET enabled={invalid}"), [])
					.unwrap();
				assert_eq!(store.game_activity_enabled(), Err(StoreError::Incompatible));
			}
			store.save_game_activity_enabled(false).unwrap();
			assert!(!store.game_activity_enabled().unwrap());
			store.0.execute_batch("DROP TABLE game_activity;").unwrap();
			assert_eq!(store.game_activity_enabled(), Err(StoreError::Unavailable));
		}
	}

	#[test]
	fn schema_seven_reading_preferences_migrate_reopen_reset_and_survive_logout() {
		let root = std::env::temp_dir().join(format!(
			"serein-synthetic-reading-preferences-{}",
			std::process::id()
		));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let mut store = LocalStore::open(&path).unwrap();
		store.save_draft(Id(1), Id(2), "preserved draft").unwrap();
		store.save_appearance(Appearance::Dark).unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','3','4','Synthetic','preserved body',0,0)", []).unwrap();
		store
			.0
			.execute_batch("DROP TABLE reading_preferences; PRAGMA user_version=7;")
			.unwrap();
		drop(store);

		let store = LocalStore::open(&path).unwrap();
		let version: u32 = store
			.0
			.pragma_query_value(None, "user_version", |row| row.get(0))
			.unwrap();
		assert_eq!(version, NATIVE_SCHEMA);
		assert_eq!(
			store.reading_preferences().unwrap(),
			ReadingPreferences::default()
		);
		let preferences = ReadingPreferences {
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
		store.save_reading_preferences(preferences).unwrap();
		drop(store);

		let store = LocalStore::open(&path).unwrap();
		assert_eq!(store.reading_preferences().unwrap(), preferences);
		store.0.execute_batch("PRAGMA query_only=ON;").unwrap();
		for replacement in [
			ReadingPreferences::default(),
			ReadingPreferences {
				zoom_percent: 150,
				sidebar_width: 360,
				show_members: true,
				show_members_dms: true,
				compact_messages: false,
				double_click_reaction_enabled: false,
				double_click_reaction: 0,
				animate_gifs: false,
				smooth_scrolling: true,
				scroll_speed_percent: 100,
				hide_media_links: true,
				confirm_external_links: true,
			},
		] {
			assert_eq!(
				store.save_reading_preferences(replacement),
				Err(StoreError::Unavailable)
			);
			assert_eq!(store.reading_preferences().unwrap(), preferences);
		}
		drop(store);

		let store = LocalStore::open(&path).unwrap();
		assert_eq!(store.reading_preferences().unwrap(), preferences);
		store
			.save_reading_preferences(ReadingPreferences::default())
			.unwrap();
		let count: u32 = store
			.0
			.query_row("SELECT count(*) FROM reading_preferences", [], |row| {
				row.get(0)
			})
			.unwrap();
		assert_eq!(count, 0);
		drop(store);

		let mut store = LocalStore::open(&path).unwrap();
		assert_eq!(
			store.reading_preferences().unwrap(),
			ReadingPreferences::default()
		);
		assert_eq!(store.appearance().unwrap(), Appearance::Dark);
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "preserved draft");
		assert_eq!(
			store.load_channel(Id(1), Id(2)).unwrap()[0].content,
			"preserved body"
		);
		store.save_reading_preferences(preferences).unwrap();
		store.forget_account(Id(1)).unwrap();
		drop(store);

		let store = LocalStore::open(&path).unwrap();
		assert_eq!(store.reading_preferences().unwrap(), preferences);
		assert_eq!(store.appearance().unwrap(), Appearance::Dark);
		assert!(store.load_drafts(Id(1)).unwrap().is_empty());
		assert!(store.load_channel(Id(1), Id(2)).unwrap().is_empty());
		drop(store);
		std::fs::remove_dir_all(root).unwrap();

		{
			let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
			store
				.save_reading_preferences(ReadingPreferences {
					animate_gifs: true,
					..Default::default()
				})
				.unwrap();
			store
				.0
				.execute_batch(
					"ALTER TABLE reading_preferences DROP COLUMN hide_media_links; PRAGMA user_version=11;",
				)
				.unwrap();
			let store = LocalStore::initialize(store.0).unwrap();
			let mut preferences = store.reading_preferences().unwrap();
			assert!(preferences.animate_gifs && preferences.hide_media_links);
			preferences.hide_media_links = false;
			store.save_reading_preferences(preferences).unwrap();
			let store = LocalStore::initialize(store.0).unwrap();
			assert_eq!(store.reading_preferences().unwrap(), preferences);
		}

		{
			let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
			store
				.save_reading_preferences(ReadingPreferences {
					smooth_scrolling: false,
					scroll_speed_percent: 100,
					..Default::default()
				})
				.unwrap();
			store
				.0
				.execute_batch(
					"ALTER TABLE reading_preferences DROP COLUMN smooth_scrolling; PRAGMA user_version=18;",
				)
				.unwrap();
			let store = LocalStore::initialize(store.0).unwrap();
			let mut preferences = store.reading_preferences().unwrap();
			assert!(preferences.smooth_scrolling);
			preferences.smooth_scrolling = false;
			store.save_reading_preferences(preferences).unwrap();
			let store = LocalStore::initialize(store.0).unwrap();
			assert_eq!(store.reading_preferences().unwrap(), preferences);
		}

		{
			let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
			store
				.save_reading_preferences(ReadingPreferences {
					zoom_percent: 125,
					..Default::default()
				})
				.unwrap();
			store
				.0
				.execute_batch(
					"ALTER TABLE reading_preferences DROP COLUMN animate_gifs; PRAGMA user_version=10;",
				)
				.unwrap();
			let store = LocalStore::initialize(store.0).unwrap();
			let mut preferences = store.reading_preferences().unwrap();
			assert_eq!(preferences.zoom_percent, 125);
			assert!(!preferences.animate_gifs);
			preferences.animate_gifs = true;
			store.save_reading_preferences(preferences).unwrap();
			let store = LocalStore::initialize(store.0).unwrap();
			assert_eq!(store.reading_preferences().unwrap(), preferences);
		}
	}

	#[test]
	fn gif_favorites_round_trip_and_remain_account_isolated() {
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		let gif = model::Gif {
			id: "chat-synthetic".into(),
			title: "Synthetic GIF".into(),
			url: "https://static.klipy.com/synthetic/wave.gif".into(),
			preview: "https://static.klipy.com/synthetic/wave.png".into(),
			width: 320,
			height: 180,
		};
		store
			.save_gif_favorites(Id(1), std::slice::from_ref(&gif))
			.unwrap();
		let mut store = LocalStore::initialize(store.0).unwrap();
		assert_eq!(store.gif_favorites(Id(1)).unwrap(), vec![gif]);
		let video = model::Gif {
			id: "discord-video".into(),
			title: String::new(),
			url: "https://tenor.com/view/synthetic-video".into(),
			preview: "https://media.tenor.com/synthetic/video.mp4".into(),
			width: 300,
			height: 200,
		};
		store
			.save_gif_favorites(Id(1), std::slice::from_ref(&video))
			.unwrap();
		assert_eq!(store.gif_favorites(Id(1)).unwrap(), vec![video]);
		assert!(store.gif_favorites(Id(2)).unwrap().is_empty());
		store.save_gif_favorites(Id(1), &[]).unwrap();
		assert!(store.gif_favorites(Id(1)).unwrap().is_empty());
	}

	#[test]
	fn older_zoom_constraint_migrates_without_losing_reading_settings() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		let preferences = ReadingPreferences {
			zoom_percent: 80,
			sidebar_width: 310,
			show_members: false,
			show_members_dms: false,
			animate_gifs: true,
			hide_media_links: false,
			confirm_external_links: false,
			smooth_scrolling: false,
			compact_messages: true,
			double_click_reaction_enabled: false,
			double_click_reaction: 0,
			scroll_speed_percent: 140,
		};
		store.save_reading_preferences(preferences).unwrap();
		let sql: String = store
			.0
			.query_row(
				"SELECT sql FROM sqlite_schema WHERE name='reading_preferences'",
				[],
				|row| row.get(0),
			)
			.unwrap();
		let old = sql
			.replace("reading_preferences", "old_reading_preferences")
			.replace("BETWEEN 50 AND 150", "BETWEEN 80 AND 150");
		store.0.execute_batch(&format!("{old}; INSERT INTO old_reading_preferences SELECT * FROM reading_preferences; DROP TABLE reading_preferences; ALTER TABLE old_reading_preferences RENAME TO reading_preferences; PRAGMA user_version=25;")).unwrap();
		assert!(
			store
				.0
				.execute("UPDATE reading_preferences SET zoom_percent=50", [])
				.is_err()
		);
		store.0.execute_batch("DELETE FROM reading_preferences; INSERT INTO reading_preferences(singleton,zoom_percent,sidebar_width,show_members) VALUES(1,80,236,1);").unwrap();
		assert_eq!(
			store.reading_preferences().unwrap(),
			ReadingPreferences {
				zoom_percent: 80,
				animate_gifs: false,
				..ReadingPreferences::default()
			}
		);
		store.save_reading_preferences(preferences).unwrap();
		let upgraded = LocalStore::initialize(store.0).unwrap();
		assert_eq!(upgraded.reading_preferences().unwrap(), preferences);
		let smaller = ReadingPreferences {
			zoom_percent: 50,
			..preferences
		};
		upgraded.save_reading_preferences(smaller).unwrap();
		let reopened = LocalStore::initialize(upgraded.0).unwrap();
		assert_eq!(reopened.reading_preferences().unwrap(), smaller);
		reopened.0.execute_batch("DELETE FROM reading_preferences; INSERT INTO reading_preferences(singleton,zoom_percent,sidebar_width,show_members) VALUES(1,50,236,1);").unwrap();
		assert_eq!(
			reopened.reading_preferences().unwrap(),
			ReadingPreferences {
				zoom_percent: 50,
				animate_gifs: false,
				..ReadingPreferences::default()
			}
		);
		assert!(
			reopened
				.0
				.execute("UPDATE reading_preferences SET zoom_percent=49", [])
				.is_err()
		);
	}

	#[test]
	fn reading_preferences_validate_storage_types_bounds_and_atomic_replacement() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		for (zoom_percent, sidebar_width) in [(50, 190), (80, 190), (150, 360)] {
			for show_members in [false, true] {
				let preferences = ReadingPreferences {
					zoom_percent,
					sidebar_width,
					show_members,
					show_members_dms: show_members,
					compact_messages: false,
					double_click_reaction_enabled: false,
					double_click_reaction: 0,
					animate_gifs: false,
					smooth_scrolling: true,
					scroll_speed_percent: 100,
					hide_media_links: true,
					confirm_external_links: true,
				};
				store.save_reading_preferences(preferences).unwrap();
				assert_eq!(store.reading_preferences().unwrap(), preferences);
			}
		}
		let previous = store.reading_preferences().unwrap();
		for (zoom_percent, sidebar_width) in [
			(0, 236),
			(49, 236),
			(151, 236),
			(u16::MAX, 236),
			(100, 189),
			(100, 361),
			(100, u16::MAX),
		] {
			assert_eq!(
				store.save_reading_preferences(ReadingPreferences {
					zoom_percent,
					sidebar_width,
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
				}),
				Err(StoreError::Capacity)
			);
			assert_eq!(store.reading_preferences().unwrap(), previous);
		}
		store.0.execute_batch("CREATE TRIGGER reject_reading_update AFTER UPDATE ON reading_preferences BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
		assert_eq!(
			store.save_reading_preferences(ReadingPreferences {
				zoom_percent: 90,
				sidebar_width: 200,
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
			}),
			Err(StoreError::Unavailable)
		);
		assert_eq!(store.reading_preferences().unwrap(), previous);
		store
			.0
			.execute_batch("DROP TRIGGER reject_reading_update;")
			.unwrap();
		assert!(
			store
				.0
				.execute("INSERT INTO reading_preferences(singleton,zoom_percent,sidebar_width,show_members) VALUES(2,100,236,1)", [])
				.is_err()
		);
		assert!(
			store
				.0
				.execute("UPDATE reading_preferences SET show_members=2", [])
				.is_err()
		);
		store
			.0
			.execute_batch("PRAGMA ignore_check_constraints=ON;")
			.unwrap();
		for invalid in [
			"zoom_percent=49",
			"zoom_percent=151",
			"zoom_percent=-1",
			"zoom_percent=65536",
			"zoom_percent=80.5",
			"zoom_percent='invalid'",
			"sidebar_width=189",
			"sidebar_width=361",
			"sidebar_width=x'00'",
			"show_members=2",
			"show_members=-1",
			"show_members='invalid'",
		] {
			store.save_reading_preferences(previous).unwrap();
			store
				.0
				.execute(&format!("UPDATE reading_preferences SET {invalid}"), [])
				.unwrap();
			assert_eq!(store.reading_preferences(), Err(StoreError::Incompatible));
		}
		store
			.save_reading_preferences(ReadingPreferences::default())
			.unwrap();
		assert_eq!(
			store.reading_preferences().unwrap(),
			ReadingPreferences::default()
		);
	}

	#[test]
	fn known_deletions_survive_reopen_and_preserve_other_channels_accounts_and_drafts() {
		let root =
			std::env::temp_dir().join(format!("serein-synthetic-deletions-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let mut store = LocalStore::open(&path).unwrap();
		for (account, channel, id) in [(1, 2, 10), (1, 2, 11), (1, 3, 10), (9, 2, 10)] {
			store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES(?1,?2,?3,'4','Synthetic','deleted synthetic body',0,0)", params![account.to_string(), channel.to_string(), id.to_string()]).unwrap();
		}
		store.save_draft(Id(1), Id(2), "preserved draft").unwrap();
		store.0.execute_batch("CREATE TRIGGER reject_second_deletion BEFORE DELETE ON messages WHEN old.id='11' BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
		assert_eq!(
			store.delete_messages(Id(1), Id(2), &[Id(10), Id(11)]),
			Err(StoreError::Unavailable)
		);
		assert_eq!(store.load_channel(Id(1), Id(2)).unwrap().len(), 2);
		store
			.0
			.execute_batch("DROP TRIGGER reject_second_deletion;")
			.unwrap();
		// Disk deletion is independent of whether a channel is selected, stale, or loading.
		store.delete_messages(Id(1), Id(2), &[Id(10)]).unwrap();
		store
			.delete_messages(Id(1), Id(2), &[Id(11), Id(11), Id(999)])
			.unwrap();
		assert_eq!(
			store.delete_messages(Id(9), Id(2), &[Id(10); 101]),
			Err(StoreError::Capacity)
		);
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		assert!(store.load_channel(Id(1), Id(2)).unwrap().is_empty());
		assert_eq!(store.load_channel(Id(1), Id(3)).unwrap().len(), 1);
		assert_eq!(store.load_channel(Id(9), Id(2)).unwrap().len(), 1);
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "preserved draft");
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn schema_six_marker_migration_preserves_rows_and_rejects_invalid_bits() {
		let root = std::env::temp_dir().join(format!(
			"serein-synthetic-content-markers-{}",
			std::process::id()
		));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let mut store = LocalStore::open(&path).unwrap();
		store.save_draft(Id(1), Id(2), "preserved draft").unwrap();
		store.save_appearance(Appearance::Dark).unwrap();
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN extra_content; PRAGMA user_version=6;")
			.unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','3','4','Synthetic','preserved body',1,1)", []).unwrap();
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		let legacy = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(legacy.len(), 1);
		assert_eq!(legacy[0].extra_content, model::ExtraContent::default());
		assert_eq!(legacy[0].content, "preserved body");
		assert!(legacy[0].edited && legacy[0].unsupported);
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "preserved draft");
		assert_eq!(store.appearance().unwrap(), Appearance::Dark);
		let version: u32 = store
			.0
			.pragma_query_value(None, "user_version", |row| row.get(0))
			.unwrap();
		assert_eq!(version, NATIVE_SCHEMA);
		let messages: Vec<_> = (0..32_u8)
			.map(|bits| {
				let mut message = legacy[0].clone();
				message.id = Id(100 + u64::from(bits));
				message.extra_content = model::ExtraContent::from_bits(bits).unwrap();
				message
			})
			.collect();
		store.save_channel(Id(1), Id(2), &messages, false).unwrap();
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		let reopened = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(reopened.len(), 33);
		assert_eq!(reopened[0].id, legacy[0].id);
		assert_eq!(reopened[0].content, "preserved body");
		for (actual, expected) in reopened[1..].iter().zip(&messages) {
			assert_eq!(actual.id, expected.id);
			assert_eq!(actual.extra_content, expected.extra_content);
			assert_eq!(actual.content, "preserved body");
			assert!(actual.edited && actual.unsupported);
		}
		assert!(store.load_channel(Id(9), Id(2)).unwrap().is_empty());
		for value in [
			rusqlite::types::Value::Integer(-1),
			rusqlite::types::Value::Integer(32),
			rusqlite::types::Value::Integer(256),
			rusqlite::types::Value::Real(1.5),
			rusqlite::types::Value::Text("invalid".into()),
		] {
			assert!(
				store
					.0
					.execute(
						"UPDATE messages SET extra_content=?1 WHERE id='131'",
						[&value]
					)
					.is_err()
			);
			// A tampered local database still cannot introduce unknown marker bits.
			store
				.0
				.execute_batch("PRAGMA ignore_check_constraints=ON;")
				.unwrap();
			store
				.0
				.execute(
					"UPDATE messages SET extra_content=?1 WHERE id='131'",
					[&value],
				)
				.unwrap();
			assert!(matches!(
				store.load_channel(Id(1), Id(2)),
				Err(StoreError::Incompatible)
			));
			store
				.0
				.execute("UPDATE messages SET extra_content=31 WHERE id='131'", [])
				.unwrap();
			store
				.0
				.execute_batch("PRAGMA ignore_check_constraints=OFF;")
				.unwrap();
		}
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "preserved draft");
		assert_eq!(store.appearance().unwrap(), Appearance::Dark);
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}
	#[test]
	fn schema_five_mentions_round_trip_and_reject_oversized_metadata() {
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		store.save_draft(Id(1), Id(2), "kept draft").unwrap();
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN mentions; PRAGMA user_version=5;")
			.unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','3','4','Synthetic','<@5>',0,0)",[]).unwrap();
		let mut store = LocalStore::initialize(store.0).unwrap();
		let mut messages = store.load_channel(Id(1), Id(2)).unwrap();
		assert!(messages[0].reactions.is_none());
		assert!(messages[0].mentions.is_empty());
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "kept draft");
		messages[0].mentions = vec![User {
			member_avatar: None,
			username: None,
			primary_guild: None,
			cosmetics: None,
			id: Id(5),
			name: "Mentioned user".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		}];
		store.save_channel(Id(1), Id(2), &messages, false).unwrap();
		assert_eq!(
			store.load_channel(Id(1), Id(2)).unwrap()[0].mentions[0].name,
			"Mentioned user"
		);
		let excessive = serde_json::to_string(&vec![messages[0].mentions[0].clone(); 101]).unwrap();
		store
			.0
			.execute("UPDATE messages SET mentions=?1", [excessive])
			.unwrap();
		assert!(matches!(
			store.load_channel(Id(1), Id(2)),
			Err(StoreError::Incompatible)
		));
		store
			.0
			.execute(
				"UPDATE messages SET mentions=?1",
				[" ".repeat(128 * 1024 + 1)],
			)
			.unwrap();
		assert!(matches!(
			store.load_channel(Id(1), Id(2)),
			Err(StoreError::Capacity)
		));
	}
	#[test]
	fn schema_four_attachment_migration_reopen_and_limits() {
		let root = std::env::temp_dir().join(format!(
			"serein-synthetic-attachments-{}",
			std::process::id()
		));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let mut store = LocalStore::open(&path).unwrap();
		store.save_draft(Id(1), Id(2), "synthetic draft").unwrap();
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN attachments; PRAGMA user_version=4;")
			.unwrap();
		store.0.execute(r#"INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported,embeds,embeds_suppressed) VALUES('1','2','3','4','Synthetic','body',0,0,'[{"title":"kept embed"}]',1)"#, []).unwrap();
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		let mut loaded = store.load_channel(Id(1), Id(2)).unwrap();
		assert!(loaded[0].attachments.is_empty());
		assert_eq!(loaded[0].embeds[0].title.as_deref(), Some("kept embed"));
		assert!(loaded[0].embeds_suppressed);
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "synthetic draft");
		loaded[0].attachments = vec![model::Attachment {
			duration_ms: None,
			waveform: Vec::new(),
			id: Id(5),
			filename: "SPOILER_synthetic.png".into(),
			description: Some("Synthetic alt text".into()),
			content_type: Some("image/png".into()),
			size: 2048,
			spoiler: true,
			media: model::EmbedMedia {
				url: Some("https://cdn.discordapp.com/attachments/2/5/synthetic.png".into()),
				width: 640,
				height: 480,
				..Default::default()
			},
		}];
		store.save_channel(Id(1), Id(2), &loaded, false).unwrap();
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		assert_eq!(
			store.load_channel(Id(1), Id(2)).unwrap()[0].attachments,
			loaded[0].attachments
		);
		assert!(store.load_channel(Id(9), Id(2)).unwrap().is_empty());
		for (json, error) in [
			("invalid JSON".to_owned(), StoreError::Incompatible),
			(
				serde_json::to_string(&vec![loaded[0].attachments[0].clone(); 11]).unwrap(),
				StoreError::Incompatible,
			),
			(" ".repeat(MAX_MEDIA_JSON + 1), StoreError::Capacity),
		] {
			store
				.0
				.execute("UPDATE messages SET attachments=?1", [json])
				.unwrap();
			assert!(matches!(store.load_channel(Id(1), Id(2)), Err(actual) if actual == error));
		}
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}
	#[test]
	fn schema_three_and_untrusted_cached_embeds_remain_bounded() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		store.0.execute_batch("ALTER TABLE messages DROP COLUMN embeds; ALTER TABLE messages DROP COLUMN embeds_suppressed; PRAGMA user_version=3;").unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','3','4','Synthetic','body',0,0)", []).unwrap();
		let store = LocalStore::initialize(store.0).unwrap();
		let loaded = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(loaded[0].content, "body");
		assert!(loaded[0].embeds.is_empty());
		assert!(!loaded[0].embeds_suppressed);
		let version: u32 = store
			.0
			.pragma_query_value(None, "user_version", |r| r.get(0))
			.unwrap();
		assert_eq!(version, NATIVE_SCHEMA);
		for (json, error) in [
			("broken JSON".to_owned(), StoreError::Incompatible),
			(
				serde_json::to_string(&vec![model::Embed::default(); 11]).unwrap(),
				StoreError::Incompatible,
			),
			(
				serde_json::json!([{"fields": vec![model::EmbedField::default(); 26]}]).to_string(),
				StoreError::Incompatible,
			),
			(" ".repeat(MAX_MEDIA_JSON + 1), StoreError::Capacity),
		] {
			store
				.0
				.execute("UPDATE messages SET embeds=?1", [json])
				.unwrap();
			assert!(matches!(store.load_channel(Id(1), Id(2)), Err(actual) if actual == error));
		}
		store
			.0
			.execute(
				"UPDATE messages SET embeds='[]',content=?1",
				["x".repeat(64 * 1024 + 1)],
			)
			.unwrap();
		assert!(matches!(
			store.load_channel(Id(1), Id(2)),
			Err(StoreError::Capacity)
		));
	}
	#[test]
	fn premium_unicode_drafts_survive_reopen_and_rejected_replacements() {
		use model::message_options::{MAX_DRAFT_CONTENT_BYTES, MAX_PREMIUM_CONTENT};
		let root = std::env::temp_dir().join(format!(
			"serein-premium-drafts-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		std::fs::create_dir(&root).unwrap();
		let path = root.join("client.sqlite3");
		let drafts = [
			"界".repeat(3000),
			format!("@silent\u{2003}{}", "🦀".repeat(MAX_PREMIUM_CONTENT)),
		];
		let mut store = LocalStore::open(&path).unwrap();
		for (index, draft) in drafts.iter().enumerate() {
			assert!(model::message_options::valid(
				draft,
				MAX_PREMIUM_CONTENT,
				false
			));
			store
				.save_draft(Id(1), Id(index as u64 + 2), draft)
				.unwrap();
		}
		store.save_draft(Id(2), Id(2), "Other account").unwrap();
		assert_eq!(
			store.save_draft(Id(1), Id(2), &"x".repeat(MAX_DRAFT_CONTENT_BYTES + 1)),
			Err(StoreError::Capacity)
		);
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		let restored = store.load_drafts(Id(1)).unwrap();
		assert_eq!(restored.len(), drafts.len());
		assert_eq!(restored[&Id(2)], drafts[0]);
		assert_eq!(restored[&Id(3)], drafts[1]);
		assert_eq!(store.load_drafts(Id(2)).unwrap()[&Id(2)], "Other account");
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn account_isolation_draft_reopen_eviction_and_logout() {
		let root =
			std::env::temp_dir().join(format!("serein-synthetic-store-{}", std::process::id()));
		std::fs::create_dir_all(&root).unwrap();
		let path = root.join("test.sqlite3");
		let legacy = Connection::open(&path).unwrap();
		legacy.execute_batch("CREATE TABLE IF NOT EXISTS messages(account TEXT NOT NULL,channel TEXT NOT NULL,id TEXT NOT NULL,author TEXT NOT NULL,name TEXT NOT NULL,content TEXT NOT NULL,edited INTEGER NOT NULL,reply TEXT,unsupported INTEGER NOT NULL,PRIMARY KEY(account,channel,id)); PRAGMA user_version=1;").unwrap();
		legacy.execute("INSERT OR REPLACE INTO messages(account,channel,id,author,name,content,edited,reply,unsupported) VALUES('9','90','900','9','Legacy','Synthetic',0,NULL,0)", []).unwrap();
		drop(legacy);
		let mut store = LocalStore::open(&path).unwrap();
		let upgraded = store.load_channel(Id(9), Id(90)).unwrap();
		assert!(upgraded[0].author.avatar.is_none());
		assert_eq!(upgraded[0].author.discriminator, 0);
		assert!(upgraded[0].embeds.is_empty());
		assert!(!upgraded[0].embeds_suppressed);
		assert_eq!(store.appearance().unwrap(), Appearance::System);
		store.save_appearance(Appearance::Light).unwrap();
		store.save_draft(Id(1), Id(20), "synthetic draft").unwrap();
		// The prior schema remains readable and is upgraded without losing drafts.
		store.0.pragma_update(None, "user_version", 1).unwrap();
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		assert_eq!(store.appearance().unwrap(), Appearance::Light);
		store.save_appearance(Appearance::Dark).unwrap();
		store.0.execute_batch("PRAGMA query_only=ON;").unwrap();
		assert_eq!(
			store.save_appearance(Appearance::System),
			Err(StoreError::Unavailable)
		);
		assert_eq!(store.appearance().unwrap(), Appearance::Dark);
		store.0.execute_batch("PRAGMA query_only=OFF;").unwrap();
		assert_eq!(
			store.load_drafts(Id(1)).unwrap()[&Id(20)],
			"synthetic draft"
		);
		assert!(store.load_drafts(Id(2)).unwrap().is_empty());
		for channel in 1..=63 {
			store
				.save_draft(Id(3), Id(channel), "other synthetic draft")
				.unwrap();
		}
		assert_eq!(
			store.save_draft(Id(1), Id(21), "over capacity"),
			Err(StoreError::Capacity)
		);
		assert_eq!(store.load_drafts(Id(1)).unwrap().len(), 1);
		assert_eq!(
			store.load_drafts(Id(1)).unwrap()[&Id(20)],
			"synthetic draft"
		);
		for channel in 1..=30 {
			let mut message = Message {
				reply_preview: None,
				poll: None,
				flags: 0,
				sticker_items: vec![],
				components: vec![],
				application_id: None,
				ephemeral: false,
				reactions: Some(vec![]),
				id: Id(100),
				channel: Id(channel),
				author: User {
					member_avatar: None,
					username: None,
					primary_guild: None,
					cosmetics: None,
					id: Id(1),
					name: "Synthetic".into(),
					avatar: Some("0123456789abcdef0123456789abcdef".into()),
					webhook: false,
					kind: Default::default(),
					discriminator: 1234,
				},
				content: "synthetic".into(),
				edited: false,
				edited_at: None,
				reply_to: None,
				unsupported: false,
				extra_content: model::ExtraContent::default(),
				kind: 0,
				reply_deleted: false,
				interaction: None,
				forwarded: false,
				embeds: vec![model::Embed {
					title: Some("Cached synthetic embed".into()),
					..Default::default()
				}],
				author_nick: None,
				author_roles: vec![],
				mention_roles: vec![],
				mention_everyone: false,
				suppress_notifications: false,
				mentions: Vec::new(),
				embeds_suppressed: true,
				attachments: Vec::new(),
				nonce: None,
				revision: 0,
			};
			// Notification targeting is session-only; disk hydration never replays it.
			message.mention_roles = vec![Id(77)];
			message.mention_everyone = true;
			message.suppress_notifications = true;
			store
				.save_channel(Id(1), Id(channel), &[message], false)
				.unwrap();
		}
		let count: i64 = store
			.0
			.query_row("SELECT count(*) FROM channels", [], |r| r.get(0))
			.unwrap();
		assert_eq!(count, 30);
		assert!(store.load_channel(Id(2), Id(30)).unwrap().is_empty());
		drop(store);
		let mut store = LocalStore::open(&path).unwrap();
		let cached = store.load_channel(Id(1), Id(30)).unwrap();
		assert_eq!(
			cached[0].embeds[0].title.as_deref(),
			Some("Cached synthetic embed")
		);
		assert!(cached[0].embeds_suppressed);
		assert!(cached[0].mention_roles.is_empty());
		assert!(!cached[0].mention_everyone);
		assert!(!cached[0].suppress_notifications);
		let cached_author = &cached[0].author;
		assert_eq!(
			cached_author.avatar.as_deref(),
			Some("0123456789abcdef0123456789abcdef")
		);
		assert_eq!(cached_author.discriminator, 1234);
		// Large cache cleanup commits bounded batches. A failed personal-data step
		// retains a durable forget intent; startup must finish it before exposing data.
		store.0.execute_batch("CREATE TRIGGER reject_draft_delete BEFORE DELETE ON drafts BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
		assert_eq!(store.forget_account(Id(1)), Err(StoreError::Unavailable));
		assert!(store.load_channel(Id(1), Id(30)).unwrap().is_empty());
		assert!(!store.load_drafts(Id(1)).unwrap().is_empty());
		let pending: i64 = store
			.0
			.query_row(
				"SELECT count(*) FROM cache_cleanup WHERE account='1' AND forget=1",
				[],
				|row| row.get(0),
			)
			.unwrap();
		assert_eq!(pending, 1);
		store
			.0
			.execute_batch("DROP TRIGGER reject_draft_delete;")
			.unwrap();
		store.forget_account(Id(1)).unwrap();
		assert!(store.load_drafts(Id(1)).unwrap().is_empty());
		assert!(store.load_channel(Id(1), Id(30)).unwrap().is_empty());
		assert_eq!(store.appearance().unwrap(), Appearance::Dark);
		assert_eq!(store.theme_variant().unwrap(), None);
		store.save_theme_variant(Some("onyx")).unwrap();
		assert_eq!(store.theme_variant().unwrap().as_deref(), Some("onyx"));
		store.save_theme_variant(Some("sunset")).unwrap();
		assert_eq!(store.theme_variant().unwrap().as_deref(), Some("sunset"));
		assert_eq!(
			store.save_theme_variant(Some(&"x".repeat(40))),
			Err(StoreError::Capacity)
		);
		store.save_theme_variant(None).unwrap();
		assert_eq!(store.theme_variant().unwrap(), None);
		store.save_appearance(Appearance::System).unwrap();
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		assert_eq!(store.appearance().unwrap(), Appearance::System);
		drop(store);
		std::fs::remove_dir_all(root).unwrap();
	}
}

#[cfg(test)]
mod component_storage_tests {
	use super::*;

	#[test]
	fn components_migrate_round_trip_and_never_persist_private_replies() {
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','3','4','Synthetic','kept',0,0)", []).unwrap();
		for column in ["components", "application_id", "original_flags"] {
			store
				.0
				.execute_batch(&format!(
					"ALTER TABLE messages DROP COLUMN {column}; PRAGMA user_version=18;"
				))
				.unwrap();
			store = LocalStore::initialize(store.0).unwrap();
		}
		let mut messages = store.load_channel(Id(1), Id(2)).unwrap();
		messages[0].flags = !64;
		messages[0].application_id = Some(Id(4));
		messages[0].components = vec![model::Component {
			kind: 2,
			custom_id: Some("synthetic".into()),
			label: Some("Press".into()),
			style: Some(1),
			..Default::default()
		}];
		store.save_channel(Id(1), Id(2), &messages, false).unwrap();
		store = LocalStore::initialize(store.0).unwrap();
		let loaded = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(loaded[0].components, messages[0].components);
		assert_eq!(loaded[0].application_id, Some(Id(4)));
		assert_eq!(loaded[0].flags, !64);
		messages[0].ephemeral = true;
		messages[0].content = "private synthetic reply".into();
		assert_eq!(
			store.save_channel(Id(1), Id(2), &messages, false),
			Err(StoreError::Capacity)
		);
		assert_eq!(store.load_channel(Id(1), Id(2)).unwrap()[0].content, "kept");
	}
	#[test]
	fn stickers_migrate_round_trip_and_reject_oversized_metadata() {
		let mut store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		store.0.execute("INSERT INTO messages(account,channel,id,author,name,content,edited,unsupported) VALUES('1','2','3','4','Synthetic','kept',0,0)", []).unwrap();
		store
			.0
			.execute_batch(
				"ALTER TABLE messages DROP COLUMN sticker_items; PRAGMA user_version=20;",
			)
			.unwrap();
		store = LocalStore::initialize(store.0).unwrap();
		let mut messages = store.load_channel(Id(1), Id(2)).unwrap();
		assert!(messages[0].sticker_items.is_empty());
		messages[0].sticker_items.push(model::Sticker {
			id: Id(99),
			name: "Wave".into(),
			description: String::new(),
			tags: String::new(),
			format_type: 1,
			guild_id: None,
			pack_id: None,
			available: true,
		});
		store.save_channel(Id(1), Id(2), &messages, false).unwrap();
		store = LocalStore::initialize(store.0).unwrap();
		assert_eq!(
			store.load_channel(Id(1), Id(2)).unwrap()[0].sticker_items,
			messages[0].sticker_items
		);
		messages[0].sticker_items[0].name = "x".repeat(121);
		assert_eq!(
			store.save_channel(Id(1), Id(2), &messages, false),
			Err(StoreError::Capacity)
		);
	}
}

#[cfg(debug_assertions)]
impl LocalStore {
	/// In-memory migration and persistence check; never opens the account database.
	pub fn debug_double_click_reaction_check() {
		let store = Self::initialize(Connection::open_in_memory().unwrap()).unwrap();
		let mut preferences = ReadingPreferences {
			zoom_percent: 125,
			..Default::default()
		};
		store.save_reading_preferences(preferences).unwrap();
		store
			.0
			.execute_batch(
				"ALTER TABLE reading_preferences DROP COLUMN double_click_reaction; ALTER TABLE reading_preferences DROP COLUMN double_click_reaction_enabled; PRAGMA user_version=27;",
			)
			.unwrap();
		let store = Self::initialize(store.0).unwrap();
		assert_eq!(store.reading_preferences().unwrap(), preferences);
		assert!(
			!store
				.reading_preferences()
				.unwrap()
				.double_click_reaction_enabled
		);
		assert_eq!(preferences.double_click_emoji(), "❤️");
		preferences.double_click_reaction = 3;
		store.save_reading_preferences(preferences).unwrap();
		store.0.execute_batch("ALTER TABLE reading_preferences DROP COLUMN double_click_reaction_enabled; PRAGMA user_version=28;").unwrap();
		let store = Self::initialize(store.0).unwrap();
		assert_eq!(
			store.reading_preferences().unwrap(),
			preferences,
			"migration preserves the chosen emoji and defaults to disabled"
		);
		preferences.double_click_reaction_enabled = true;
		store.save_reading_preferences(preferences).unwrap();
		let store = Self::initialize(store.0).unwrap();
		assert_eq!(store.reading_preferences().unwrap(), preferences);
		assert_eq!(
			store.reading_preferences().unwrap().double_click_emoji(),
			"🎉"
		);
		let invalid = ReadingPreferences {
			double_click_reaction_enabled: false,
			double_click_reaction: u8::MAX,
			..preferences
		};
		assert_eq!(
			store.save_reading_preferences(invalid),
			Err(StoreError::Capacity)
		);
		assert_eq!(store.reading_preferences().unwrap(), preferences);
		println!("Double-click reaction migration, roundtrip and bounds passed in memory");
	}
}
