//! Account-isolated bounded SQLite cache. This is not Discord's authoritative state.
mod channel_preferences;
use model::{Id, Message, ReadingPreferences, User};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
	collections::{BTreeMap, BTreeSet},
	path::Path,
};

const MAX_MEDIA_JSON: usize = 256 * 1024;
const MAX_WINDOW_BYTES: usize = 4 * 1024 * 1024;
const NATIVE_SCHEMA: u32 = 21;
const READABLE_SCHEMA: u32 = 21;
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
/// Device-local controls, bounded independently of account caches.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppPreferences {
	pub notifications_enabled: bool,
	pub auto_update: bool,
	pub update_nightly: bool,
	pub notification_options: model::notification_preferences::Device,
	pub show_hidden_channels: bool,
	pub hide_title_bar: bool,
	pub primary_color: Option<[u8; 3]>,
	pub transparency_blur: bool,
	pub transparency: u8,
	pub blur: u8,
	pub transparent_all: bool,
	pub voice_noise_suppression: bool,
	/// Absent in older preferences; migrate using the legacy suppression setting.
	#[serde(default)]
	pub voice_processing: Option<model::voice_settings::VoiceProcessing>,
	pub voice_push_to_talk: bool,
	pub voice_muted: bool,
	pub voice_deafened: bool,
	pub voice_input: Option<String>,
	pub voice_output: Option<String>,
	pub input_percent: u16,
	pub output_percent: u16,
	/// Which GPU renders the window; applied on the next start.
	pub gpu_preference: model::GpuPreference,
	/// Device-local, account-independent keyboard bindings.
	pub keybinds: model::Keybinds,
	/// Expanded server folders, bounded so one device preference stays small.
	pub expanded_folders: Vec<u64>,
	/// Per-user voice volume overrides, bounded so one device preference stays small.
	pub user_volumes: Vec<(u64, u16)>,
	/// Voice participants silenced on this device only, bounded like the volume overrides.
	pub muted_users: Vec<u64>,
}
impl Default for AppPreferences {
	fn default() -> Self {
		Self {
			notifications_enabled: true,
			auto_update: false,
			update_nightly: true,
			notification_options: Default::default(),
			show_hidden_channels: false,
			hide_title_bar: false,
			primary_color: None,
			transparency_blur: false,
			transparency: 15,
			blur: 50,
			transparent_all: false,
			voice_noise_suppression: false,
			voice_processing: Some(model::voice_settings::VoiceProcessing::default()),
			voice_push_to_talk: false,
			voice_muted: false,
			voice_deafened: false,
			voice_input: None,
			voice_output: None,
			input_percent: 100,
			output_percent: 100,
			gpu_preference: Default::default(),
			keybinds: Default::default(),
			expanded_folders: Vec::new(),
			user_volumes: Vec::new(),
			muted_users: Vec::new(),
		}
	}
}
impl AppPreferences {
	pub fn is_valid(&self) -> bool {
		self.transparency <= 100
			&& self.blur <= 100
			&& self.input_percent <= 200
			&& self.output_percent <= 200
			&& self
				.voice_processing
				.is_none_or(|value| value.custom.is_valid())
			&& self.expanded_folders.len() <= 256
			&& self.user_volumes.len() <= 64
			&& self.user_volumes.iter().all(|(_, volume)| *volume <= 200)
			&& self.muted_users.len() <= 64
			&& self.keybinds.is_valid()
			&& [&self.voice_input, &self.voice_output]
				.into_iter()
				.all(|value| value.as_ref().is_none_or(|value| value.len() <= 1024))
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
	#[default]
	System,
	Light,
	Dark,
}
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
	pub fn open_default() -> Result<Self> {
		let root = dirs::data_local_dir()
			.ok_or(StoreError::Unavailable)?
			.join("serein");
		std::fs::create_dir_all(&root).map_err(|_| StoreError::Unavailable)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
				.map_err(|_| StoreError::Unavailable)?;
		}
		Self::open(&root.join("client.sqlite3"))
	}
	pub fn open(path: &Path) -> Result<Self> {
		Self::initialize(Connection::open(path)?)
	}
	fn initialize(mut connection: Connection) -> Result<Self> {
		connection.busy_timeout(std::time::Duration::from_secs(2))?;
		let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
		if version > READABLE_SCHEMA {
			return Err(StoreError::Incompatible);
		}
		connection.execute_batch("PRAGMA page_size=4096; PRAGMA max_page_count=16384; PRAGMA cache_size=-2048; PRAGMA temp_store=MEMORY; PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA wal_autocheckpoint=256; PRAGMA journal_size_limit=8388608; PRAGMA secure_delete=ON; PRAGMA auto_vacuum=INCREMENTAL;
            CREATE TABLE IF NOT EXISTS messages(account TEXT NOT NULL,channel TEXT NOT NULL,id TEXT NOT NULL,author TEXT NOT NULL,name TEXT NOT NULL,content TEXT NOT NULL,edited INTEGER NOT NULL,reply TEXT,unsupported INTEGER NOT NULL,PRIMARY KEY(account,channel,id));
            CREATE INDEX IF NOT EXISTS messages_channel_order ON messages(account,channel,length(id),id);
            CREATE TABLE IF NOT EXISTS channels(account TEXT NOT NULL,channel TEXT NOT NULL,touched INTEGER NOT NULL,PRIMARY KEY(account,channel));
            CREATE TABLE IF NOT EXISTS drafts(account TEXT NOT NULL,channel TEXT NOT NULL,content TEXT NOT NULL,PRIMARY KEY(account,channel));
            CREATE TABLE IF NOT EXISTS appearance(singleton INTEGER PRIMARY KEY CHECK(singleton=1),theme TEXT NOT NULL CHECK(theme IN ('light','dark')));
            CREATE TABLE IF NOT EXISTS theme_variant(singleton INTEGER PRIMARY KEY CHECK(singleton=1),variant TEXT NOT NULL CHECK(length(variant) BETWEEN 1 AND 32));
            CREATE TABLE IF NOT EXISTS gif_favorites(account TEXT NOT NULL,position INTEGER NOT NULL CHECK(typeof(position)='integer' AND position BETWEEN 0 AND 99),id TEXT NOT NULL CHECK(length(id) BETWEEN 1 AND 64),title TEXT NOT NULL CHECK(length(title) <= 256),url TEXT NOT NULL CHECK(length(url) BETWEEN 1 AND 512),preview TEXT NOT NULL CHECK(length(preview) BETWEEN 1 AND 512),width INTEGER NOT NULL CHECK(typeof(width)='integer' AND width BETWEEN 1 AND 4096),height INTEGER NOT NULL CHECK(typeof(height)='integer' AND height BETWEEN 1 AND 4096),PRIMARY KEY(account,position));
            ")?;
		let has_avatar: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='avatar')",
			[],
			|row| row.get(0),
		)?;
		if !has_avatar {
			connection.execute_batch("BEGIN; ALTER TABLE messages ADD COLUMN avatar TEXT; ALTER TABLE messages ADD COLUMN discriminator INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=3; COMMIT;")?;
		}
		let has_embeds: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='embeds')",
			[],
			|row| row.get(0),
		)?;
		if !has_embeds {
			connection.execute_batch("BEGIN; ALTER TABLE messages ADD COLUMN embeds TEXT NOT NULL DEFAULT '[]'; ALTER TABLE messages ADD COLUMN embeds_suppressed INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=4; COMMIT;")?;
		}
		let has_attachments: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='attachments')",
			[],
			|row| row.get(0),
		)?;
		if !has_attachments {
			connection.execute_batch("BEGIN; ALTER TABLE messages ADD COLUMN attachments TEXT NOT NULL DEFAULT '[]'; PRAGMA user_version=5; COMMIT;")?;
		} else if version < 5 {
			connection.pragma_update(None, "user_version", 5)?;
		}
		let has_mentions: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='mentions')",
			[],
			|row| row.get(0),
		)?;
		if !has_mentions {
			connection.execute_batch("BEGIN; ALTER TABLE messages ADD COLUMN mentions TEXT NOT NULL DEFAULT '[]'; PRAGMA user_version=6; COMMIT;")?;
		} else if version < 6 {
			connection.pragma_update(None, "user_version", 6)?;
		}
		// Schema 7 was used independently for markers and system-message kinds.
		// Detect both columns and commit their union with reading preferences atomically.
		let has_extra_content: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='extra_content')",
			[],
			|row| row.get(0),
		)?;
		let has_message_kind: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='message_kind')",
			[],
			|row| row.get(0),
		)?;
		let has_reply_deleted: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='reply_deleted')",
			[],
			|row| row.get(0),
		)?;
		let has_account_kind: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='account_kind')",
			[],
			|row| row.get(0),
		)?;
		let has_forwarded: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='forwarded')",
			[],
			|row| row.get(0),
		)?;
		let has_webhook: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='webhook')",
			[],
			|row| row.get(0),
		)?;
		let has_stickers: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='sticker_items')",
			[],
			|row| row.get(0),
		)?;
		let has_components: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='components')",
			[],
			|row| row.get(0),
		)?;
		let has_application_id: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='application_id')",
			[],
			|row| row.get(0),
		)?;
		let has_original_flags: bool = connection.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='original_flags')",
			[],
			|row| row.get(0),
		)?;
		let transaction = connection.transaction()?;
		if !has_original_flags {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN original_flags TEXT NOT NULL DEFAULT '0' CHECK(typeof(original_flags)='text' AND length(CAST(original_flags AS BLOB)) BETWEEN 1 AND 20);")?;
		}

		if !has_application_id {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN application_id TEXT;")?;
		}
		if !has_stickers {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN sticker_items TEXT NOT NULL DEFAULT '[]' CHECK(length(CAST(sticker_items AS BLOB))<=32768);")?;
		}
		if !has_components {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN components TEXT NOT NULL DEFAULT '[]' CHECK(length(CAST(components AS BLOB))<=262144);")?;
		}

		if !has_forwarded {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN forwarded INTEGER NOT NULL DEFAULT 0 CHECK(typeof(forwarded)='integer' AND forwarded IN (0,1));")?;
		}
		if !has_account_kind {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN account_kind INTEGER NOT NULL DEFAULT 0 CHECK(typeof(account_kind)='integer' AND account_kind BETWEEN 0 AND 2);")?;
		}
		if !has_webhook {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN webhook INTEGER NOT NULL DEFAULT 0 CHECK(typeof(webhook)='integer' AND webhook IN (0,1));")?;
		}
		transaction.execute_batch("CREATE TABLE IF NOT EXISTS app_preferences(
            singleton INTEGER PRIMARY KEY CHECK(singleton=1),
            value TEXT NOT NULL CHECK(typeof(value)='text' AND length(CAST(value AS BLOB))<=16384));")?;
		if !has_reply_deleted {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN reply_deleted INTEGER NOT NULL DEFAULT 0 CHECK(typeof(reply_deleted)='integer' AND reply_deleted IN (0,1));")?;
		}
		if !has_extra_content {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN extra_content INTEGER NOT NULL DEFAULT 0 CHECK(typeof(extra_content)='integer' AND extra_content BETWEEN 0 AND 31);")?;
		}
		if !has_message_kind {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN message_kind INTEGER NOT NULL DEFAULT 0 CHECK(typeof(message_kind)='integer' AND message_kind BETWEEN 0 AND 255); UPDATE messages SET message_kind=255 WHERE unsupported<>0;")?;
		}
		transaction.execute_batch("CREATE TABLE IF NOT EXISTS reading_preferences(
                singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                zoom_percent INTEGER NOT NULL CHECK(typeof(zoom_percent)='integer' AND zoom_percent BETWEEN 80 AND 150),
                sidebar_width INTEGER NOT NULL CHECK(typeof(sidebar_width)='integer' AND sidebar_width BETWEEN 190 AND 360),
                show_members INTEGER NOT NULL CHECK(typeof(show_members)='integer' AND show_members IN (0,1))
            );
            CREATE TABLE IF NOT EXISTS game_activity(
                singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                enabled INTEGER NOT NULL CHECK(typeof(enabled)='integer' AND enabled IN (0,1))
            );
            CREATE TABLE IF NOT EXISTS minimize_to_tray(
                singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                enabled INTEGER NOT NULL CHECK(typeof(enabled)='integer' AND enabled IN (0,1))
            );
            CREATE TABLE IF NOT EXISTS channel_preferences(
                account TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL CHECK(typeof(value)='text' AND length(CAST(value AS BLOB))<=8192)
            );
            CREATE TABLE IF NOT EXISTS accounts(
                account TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL CHECK(typeof(name)='text' AND length(CAST(name AS BLOB)) BETWEEN 1 AND 64),
                display TEXT CHECK(display IS NULL OR (typeof(display)='text' AND length(CAST(display AS BLOB)) BETWEEN 1 AND 64)),
                avatar TEXT CHECK(avatar IS NULL OR (typeof(avatar)='text' AND length(avatar) BETWEEN 1 AND 34)),
                discriminator INTEGER NOT NULL CHECK(typeof(discriminator)='integer' AND discriminator BETWEEN 0 AND 9999),
                touched INTEGER NOT NULL CHECK(typeof(touched)='integer'),
                has_token INTEGER NOT NULL DEFAULT 0 CHECK(typeof(has_token)='integer' AND has_token IN (0,1))
            );")?;
		let has_token_column: bool = transaction.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('accounts') WHERE name='has_token')",
			[],
			|row| row.get(0),
		)?;
		if !has_token_column {
			// Rows predating the flag came from a build that wrote a per-account entry on every
			// connect, so their entries exist. Claiming otherwise would rewrite each one, which
			// on macOS is an access-controlled keychain operation; a wrong claim self-heals on
			// the next switch instead.
			transaction.execute_batch("ALTER TABLE accounts ADD COLUMN has_token INTEGER NOT NULL DEFAULT 0 CHECK(typeof(has_token)='integer' AND has_token IN (0,1)); UPDATE accounts SET has_token=1;")?;
		}
		transaction.pragma_update(None, "user_version", version.max(NATIVE_SCHEMA))?;
		let has_animate_gifs: bool = transaction.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('reading_preferences') WHERE name='animate_gifs')",
			[],
			|row| row.get(0),
		)?;
		if !has_animate_gifs {
			transaction.execute_batch("ALTER TABLE reading_preferences ADD COLUMN animate_gifs INTEGER NOT NULL DEFAULT 0 CHECK(typeof(animate_gifs)='integer' AND animate_gifs IN (0,1));")?;
		}
		let has_hide_media_links: bool = transaction.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('reading_preferences') WHERE name='hide_media_links')", [], |row| row.get(0),
		)?;
		if !has_hide_media_links {
			transaction.execute_batch("ALTER TABLE reading_preferences ADD COLUMN hide_media_links INTEGER NOT NULL DEFAULT 1 CHECK(typeof(hide_media_links)='integer' AND hide_media_links IN (0,1));")?;
		}
		let has_confirm_external_links: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('reading_preferences') WHERE name='confirm_external_links')", [], |row| row.get(0),
        )?;
		if !has_confirm_external_links {
			transaction.execute_batch("ALTER TABLE reading_preferences ADD COLUMN confirm_external_links INTEGER NOT NULL DEFAULT 1 CHECK(typeof(confirm_external_links)='integer' AND confirm_external_links IN (0,1));")?;
		}
		let has_author_roles: bool = transaction.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='author_roles')",
			[],
			|row| row.get(0),
		)?;
		if !has_author_roles {
			transaction.execute_batch(
				"ALTER TABLE messages ADD COLUMN author_roles TEXT NOT NULL DEFAULT '[]';",
			)?;
		}
		let has_author_nick: bool = transaction.query_row(
			"SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='author_nick')",
			[],
			|row| row.get(0),
		)?;
		if !has_author_nick {
			transaction.execute_batch("ALTER TABLE messages ADD COLUMN author_nick TEXT;")?;
		}
		transaction.commit()?;
		Ok(Self(connection))
	}
	pub fn app_preferences(&self) -> Result<AppPreferences> {
		let value: Option<String> = self.0.query_row(
            "SELECT CASE WHEN length(CAST(value AS BLOB))<=16384 THEN value ELSE NULL END FROM app_preferences WHERE singleton=1",
            [], |row| row.get(0)).optional()?;
		let value: AppPreferences = match value {
			Some(value) => serde_json::from_str(&value).map_err(|_| StoreError::Incompatible)?,
			None => AppPreferences::default(),
		};
		if !value.is_valid() {
			return Err(StoreError::Incompatible);
		}
		Ok(value)
	}
	pub fn save_app_preferences(&self, value: &AppPreferences) -> Result<()> {
		if !value.is_valid() {
			return Err(StoreError::Incompatible);
		}
		let value = serde_json::to_string(value).map_err(|_| StoreError::Incompatible)?;
		self.0.execute(
			"INSERT INTO app_preferences VALUES(1,?1)
            ON CONFLICT(singleton) DO UPDATE SET value=excluded.value",
			[value],
		)?;
		Ok(())
	}
	/// Application-wide opt-in; an absent override never enables activity sharing.
	pub fn game_activity_enabled(&self) -> Result<bool> {
		let stored = self
			.0
			.query_row(
				"SELECT enabled FROM game_activity WHERE singleton=1",
				[],
				|row| {
					Ok(match row.get_ref(0)? {
						rusqlite::types::ValueRef::Integer(enabled @ 0..=1) => Some(enabled == 1),
						_ => None,
					})
				},
			)
			.optional()?;
		match stored {
			None => Ok(false),
			Some(Some(enabled)) => Ok(enabled),
			Some(None) => Err(StoreError::Incompatible),
		}
	}
	pub fn save_game_activity_enabled(&self, enabled: bool) -> Result<()> {
		if enabled {
			self.0.execute(
				"INSERT INTO game_activity(singleton,enabled) VALUES(1,1)
                ON CONFLICT(singleton) DO UPDATE SET enabled=1",
				[],
			)?;
		} else {
			self.0
				.execute("DELETE FROM game_activity WHERE singleton=1", [])?;
		}
		Ok(())
	}
	/// Application-wide opt-out; an absent override keeps the tray icon enabled.
	pub fn minimize_to_tray(&self) -> Result<bool> {
		let stored = self
			.0
			.query_row(
				"SELECT enabled FROM minimize_to_tray WHERE singleton=1",
				[],
				|row| {
					Ok(match row.get_ref(0)? {
						rusqlite::types::ValueRef::Integer(enabled @ 0..=1) => Some(enabled == 1),
						_ => None,
					})
				},
			)
			.optional()?;
		match stored {
			None => Ok(true),
			Some(Some(enabled)) => Ok(enabled),
			Some(None) => Err(StoreError::Incompatible),
		}
	}
	pub fn save_minimize_to_tray(&self, enabled: bool) -> Result<()> {
		if enabled {
			self.0
				.execute("DELETE FROM minimize_to_tray WHERE singleton=1", [])?;
		} else {
			self.0.execute(
				"INSERT INTO minimize_to_tray(singleton,enabled) VALUES(1,0)
                ON CONFLICT(singleton) DO UPDATE SET enabled=0",
				[],
			)?;
		}
		Ok(())
	}
	/// Application-wide settings survive account logout; missing override means defaults.
	pub fn reading_preferences(&self) -> Result<ReadingPreferences> {
		use rusqlite::types::ValueRef;
		let stored = self
			.0
			.query_row(
				"SELECT zoom_percent,sidebar_width,show_members,animate_gifs,hide_media_links,confirm_external_links FROM reading_preferences WHERE singleton=1",
				[],
				|row| {
					Ok(match (
						row.get_ref(0)?,
						row.get_ref(1)?,
						row.get_ref(2)?,
						row.get_ref(3)?,
						row.get_ref(4)?,
						row.get_ref(5)?,
					) {
						(
							ValueRef::Integer(zoom @ 80..=150),
							ValueRef::Integer(width @ 190..=360),
							ValueRef::Integer(members @ 0..=1),
							ValueRef::Integer(animate_gifs @ 0..=1),
							ValueRef::Integer(hide_media_links @ 0..=1),
							ValueRef::Integer(confirm_external_links @ 0..=1),
						) => Some(ReadingPreferences {
							zoom_percent: zoom as u16,
							sidebar_width: width as u16,
							show_members: members == 1,
							animate_gifs: animate_gifs == 1,
							hide_media_links: hide_media_links == 1,
							confirm_external_links: confirm_external_links == 1,
						}),
						_ => None,
					})
				},
			)
			.optional()?;
		match stored {
			None => Ok(ReadingPreferences::default()),
			Some(Some(preferences)) => Ok(preferences),
			Some(None) => Err(StoreError::Incompatible),
		}
	}
	pub fn save_reading_preferences(&self, preferences: ReadingPreferences) -> Result<()> {
		if !preferences.is_valid() {
			return Err(StoreError::Capacity);
		}
		// Each statement is one SQLite transaction; reset only removes this override.
		if preferences == ReadingPreferences::default() {
			self.0
				.execute("DELETE FROM reading_preferences WHERE singleton=1", [])?;
		} else {
			self.0.execute("INSERT INTO reading_preferences(singleton,zoom_percent,sidebar_width,show_members,animate_gifs,hide_media_links,confirm_external_links)
                VALUES(1,?1,?2,?3,?4,?5,?6) ON CONFLICT(singleton) DO UPDATE SET
                zoom_percent=excluded.zoom_percent,sidebar_width=excluded.sidebar_width,show_members=excluded.show_members,animate_gifs=excluded.animate_gifs,hide_media_links=excluded.hide_media_links,confirm_external_links=excluded.confirm_external_links",
                params![preferences.zoom_percent, preferences.sidebar_width, preferences.show_members, preferences.animate_gifs, preferences.hide_media_links, preferences.confirm_external_links])?;
		}
		Ok(())
	}
	/// One account-independent application preference. System deletes the override.
	pub fn appearance(&self) -> Result<Appearance> {
		match self
			.0
			.query_row("SELECT theme FROM appearance WHERE singleton=1", [], |r| {
				r.get::<_, String>(0)
			})
			.optional()?
			.as_deref()
		{
			None => Ok(Appearance::System),
			Some("light") => Ok(Appearance::Light),
			Some("dark") => Ok(Appearance::Dark),
			Some(_) => Err(StoreError::Incompatible),
		}
	}
	pub fn save_appearance(&self, appearance: Appearance) -> Result<()> {
		match appearance {
			Appearance::System => {
				self.0.execute("DELETE FROM appearance", [])?;
			}
			Appearance::Light | Appearance::Dark => {
				let theme = if appearance == Appearance::Light {
					"light"
				} else {
					"dark"
				};
				self.0.execute("INSERT INTO appearance VALUES(1,?1) ON CONFLICT(singleton) DO UPDATE SET theme=excluded.theme", [theme])?;
			}
		}
		Ok(())
	}
	/// Recolour preset key (see the UI crate's theme variants); `None` means the default.
	pub fn theme_variant(&self) -> Result<Option<String>> {
		Ok(self
			.0
			.query_row(
				"SELECT variant FROM theme_variant WHERE singleton=1",
				[],
				|r| r.get::<_, String>(0),
			)
			.optional()?)
	}
	pub fn save_theme_variant(&self, variant: Option<&str>) -> Result<()> {
		match variant {
			None => {
				self.0.execute("DELETE FROM theme_variant", [])?;
			}
			Some(variant) if (1..=32).contains(&variant.len()) => {
				self.0.execute("INSERT INTO theme_variant VALUES(1,?1) ON CONFLICT(singleton) DO UPDATE SET variant=excluded.variant", [variant])?;
			}
			Some(_) => return Err(StoreError::Capacity),
		}
		Ok(())
	}
	/// Apply a bounded changed-row batch while retaining exactly the current window IDs.
	pub fn save_changes(
		&mut self,
		account: Id,
		channel: Id,
		messages: &[Message],
		retained: &[Id],
	) -> Result<()> {
		if retained.len() > 500
			|| messages.len() > 500
			|| messages.iter().map(Message::bytes).sum::<usize>() > MAX_WINDOW_BYTES
		{
			return Err(StoreError::Capacity);
		}
		let retained: std::collections::BTreeSet<_> = retained.iter().copied().collect();
		let existing = self.load_channel(account, channel)?;
		let mut window: BTreeMap<_, _> = existing
			.iter()
			.filter(|m| retained.contains(&m.id))
			.map(|m| (m.id, m.clone()))
			.collect();
		for message in messages {
			if !retained.contains(&message.id) {
				return Err(StoreError::Capacity);
			}
			window.insert(message.id, message.clone());
		}
		self.save_channel_loaded(
			account,
			channel,
			&window.into_values().collect::<Vec<_>>(),
			&existing,
		)
	}
	pub fn save_channel(&mut self, account: Id, channel: Id, messages: &[Message]) -> Result<()> {
		if messages.len() > 500 {
			return Err(StoreError::Capacity);
		}
		let existing = self.load_channel(account, channel)?;
		self.save_channel_loaded(account, channel, messages, &existing)
	}
	fn save_channel_loaded(
		&mut self,
		account: Id,
		channel: Id,
		messages: &[Message],
		existing: &[Message],
	) -> Result<()> {
		if messages.len() > 500
			|| messages.iter().map(Message::bytes).sum::<usize>() > MAX_WINDOW_BYTES
			|| messages.iter().any(|m| {
				m.channel != channel
					|| (m.reply_deleted
						&& (!matches!(m.kind, 19 | 23)
							|| !m.reply_to.is_some_and(|id| id.0 > 0 && id < m.id)))
					|| !model::valid_mentions(&m.mentions)
					|| m.ephemeral || m.flags & 64 != 0
					|| m.application_id.is_some_and(|id| id.0 == 0)
					|| !model::valid_stickers(&m.sticker_items, model::MAX_MESSAGE_STICKERS)
					|| !model::valid_components(&m.components)
					|| !model::valid_embeds(&m.embeds)
					|| !model::valid_attachments(&m.attachments)
			}) {
			return Err(StoreError::Capacity);
		}
		// Compare on the storage worker; unchanged rows need no serialization or write.
		let transaction = self.0.transaction()?;
		let account = account.to_string();
		let channel = channel.to_string();
		let retained: std::collections::BTreeSet<_> = messages.iter().map(|m| m.id).collect();
		let previous: BTreeMap<_, _> = existing.iter().map(|m| (m.id, m)).collect();
		let mut deleted = false;
		{
			let mut delete = transaction
				.prepare_cached("DELETE FROM messages WHERE account=?1 AND channel=?2 AND id=?3")?;
			for message in existing {
				if !retained.contains(&message.id) {
					deleted |=
						delete.execute(params![account, channel, message.id.to_string()])? > 0;
				}
			}
		}
		let mut insert = transaction.prepare_cached(
            "INSERT OR REPLACE INTO messages(account,channel,id,author,name,content,edited,reply,unsupported,avatar,discriminator,embeds,embeds_suppressed,attachments,mentions,extra_content,message_kind,reply_deleted,webhook,account_kind,forwarded,author_roles,author_nick,components,application_id,original_flags,sticker_items) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27)")?;
		for message in messages {
			if previous
				.get(&message.id)
				.is_some_and(|old| **old == *message)
			{
				continue;
			}
			let mentions =
				serde_json::to_string(&message.mentions).map_err(|_| StoreError::Incompatible)?;
			if mentions.len() > 128 * 1024 {
				return Err(StoreError::Capacity);
			}
			if message.author_roles.len() > model::permissions::MAX_MEMBER_ROLES
				|| message.author_roles.iter().any(|role| role.0 == 0)
			{
				return Err(StoreError::Capacity);
			}
			{
				let mut seen = BTreeSet::new();
				if message.author_roles.iter().any(|role| !seen.insert(*role)) {
					return Err(StoreError::Capacity);
				}
			}
			let author_roles = serde_json::to_string(
				&message
					.author_roles
					.iter()
					.map(|role| role.to_string())
					.collect::<Vec<_>>(),
			)
			.map_err(|_| StoreError::Incompatible)?;
			if author_roles.len() > 16 * 1024 {
				return Err(StoreError::Capacity);
			}
			if message
				.author_nick
				.as_ref()
				.is_some_and(|nick| nick.len() > 512)
			{
				return Err(StoreError::Capacity);
			}
			let sticker_items = serde_json::to_string(&message.sticker_items)
				.map_err(|_| StoreError::Incompatible)?;
			if sticker_items.len() > 32768 {
				return Err(StoreError::Capacity);
			}
			let components =
				serde_json::to_string(&message.components).map_err(|_| StoreError::Incompatible)?;
			if components.len() > MAX_MEDIA_JSON {
				return Err(StoreError::Capacity);
			}
			let embeds =
				serde_json::to_string(&message.embeds).map_err(|_| StoreError::Incompatible)?;
			let attachments = serde_json::to_string(&message.attachments)
				.map_err(|_| StoreError::Incompatible)?;
			if embeds.len() > MAX_MEDIA_JSON || attachments.len() > MAX_MEDIA_JSON {
				return Err(StoreError::Capacity);
			}
			insert.execute(params![
				account,
				channel,
				message.id.to_string(),
				message.author.id.to_string(),
				message.author.name,
				message.content,
				message.edited,
				message.reply_to.map(|id| id.to_string()),
				message.unsupported,
				message.author.avatar,
				message.author.discriminator,
				embeds,
				message.embeds_suppressed,
				attachments,
				mentions,
				message.extra_content.bits(),
				message.kind,
				message.reply_deleted,
				message.author.webhook,
				message.author.kind as u8,
				message.forwarded,
				author_roles,
				message.author_nick.as_deref(),
				components,
				message.application_id.map(|id| id.to_string()),
				message.flags.to_string(),
				sticker_items,
			])?;
		}
		drop(insert);
		transaction.execute("INSERT INTO channels VALUES(?1,?2,unixepoch('subsec')*1000) ON CONFLICT(account,channel) DO UPDATE SET touched=excluded.touched",params![account,channel])?;
		// Global limit: 20 channel windows, 10000 messages AND 48 MiB content, below the 64 MiB database page ceiling.
		loop {
			let channels: i64 =
				transaction.query_row("SELECT count(*) FROM channels", [], |row| row.get(0))?;
			let page_count: i64 =
				transaction.pragma_query_value(None, "page_count", |row| row.get(0))?;
			let free_pages: i64 =
				transaction.pragma_query_value(None, "freelist_count", |row| row.get(0))?;
			let page_size: i64 =
				transaction.pragma_query_value(None, "page_size", |row| row.get(0))?;
			let bytes = if (page_count - free_pages) * page_size <= 48 * 1024 * 1024 {
				0
			} else {
				transaction.query_row("SELECT coalesce(sum(length(CAST(content AS BLOB))+length(CAST(name AS BLOB))+length(CAST(original_flags AS BLOB))+length(CAST(components AS BLOB))+length(CAST(sticker_items AS BLOB))+coalesce(length(CAST(application_id AS BLOB)),0)+length(CAST(embeds AS BLOB))+length(CAST(attachments AS BLOB))+length(CAST(mentions AS BLOB))+length(CAST(author_roles AS BLOB))+coalesce(length(CAST(author_nick AS BLOB)),0)+256),0) FROM messages",[],|row|row.get(0))?
			};
			if channels <= 20 && bytes <= 48 * 1024 * 1024 {
				break;
			}
			let (a, c): (String, String) = transaction.query_row(
				"SELECT account,channel FROM channels ORDER BY touched,account,channel LIMIT 1",
				[],
				|r| Ok((r.get(0)?, r.get(1)?)),
			)?;
			transaction.execute(
				"DELETE FROM messages WHERE account=?1 AND channel=?2",
				params![a, c],
			)?;
			transaction.execute(
				"DELETE FROM channels WHERE account=?1 AND channel=?2",
				params![a, c],
			)?;
			deleted = true;
		}
		transaction.commit()?;
		if deleted {
			self.0.execute_batch("PRAGMA incremental_vacuum(64);")?;
		}
		Ok(())
	}
	pub fn load_channel(&self, account: Id, channel: Id) -> Result<Vec<Message>> {
		let mut query = self.0.prepare_cached("SELECT id,author,name,content,edited,reply,unsupported,avatar,discriminator,embeds,embeds_suppressed,attachments,mentions,extra_content,message_kind,reply_deleted,webhook,account_kind,forwarded,author_roles,author_nick,components,application_id,original_flags,sticker_items FROM messages WHERE account=?1 AND channel=?2 ORDER BY length(id),id LIMIT 500")?;
		let mut rows = query.query(params![account.to_string(), channel.to_string()])?;
		let mut messages = Vec::new();
		let mut bytes = 0;
		while let Some(row) = rows.next()? {
			let account_kind = match row.get_ref(17)? {
				rusqlite::types::ValueRef::Integer(0) => model::AccountKind::Human,
				rusqlite::types::ValueRef::Integer(1) => model::AccountKind::Bot,
				rusqlite::types::ValueRef::Integer(2) => model::AccountKind::App,
				_ => return Err(StoreError::Incompatible),
			};
			let webhook = match row.get_ref(16)? {
				rusqlite::types::ValueRef::Integer(0) => false,
				rusqlite::types::ValueRef::Integer(1) => true,
				_ => return Err(StoreError::Incompatible),
			};
			let reply_deleted = match row.get_ref(15)? {
				rusqlite::types::ValueRef::Integer(0) => false,
				rusqlite::types::ValueRef::Integer(1) => true,
				_ => return Err(StoreError::Incompatible),
			};
			let extra_content = row
				.get_ref(13)?
				.as_i64()
				.ok()
				.and_then(|bits| u8::try_from(bits).ok())
				.and_then(model::ExtraContent::from_bits)
				.ok_or(StoreError::Incompatible)?;
			// Inspect borrowed SQLite fields before allocating attacker-controlled cache strings.
			for (column, maximum) in [
				(0, 20),
				(1, 20),
				(2, 512),
				(3, 64 * 1024),
				(9, MAX_MEDIA_JSON),
				(11, MAX_MEDIA_JSON),
				(12, 128 * 1024),
				(19, 16 * 1024),
				(21, MAX_MEDIA_JSON),
				(23, 20),
				(24, 32768),
			] {
				if row
					.get_ref(column)?
					.as_str()
					.map_err(|_| StoreError::Incompatible)?
					.len() > maximum
				{
					return Err(StoreError::Capacity);
				}
			}
			for (column, maximum) in [(5, 20), (7, 34), (20, 512), (22, 20)] {
				if !matches!(row.get_ref(column)?, rusqlite::types::ValueRef::Null)
					&& row
						.get_ref(column)?
						.as_str()
						.map_err(|_| StoreError::Incompatible)?
						.len() > maximum
				{
					return Err(StoreError::Capacity);
				}
			}
			let embeds = serde_json::from_str::<CachedEmbeds>(
				row.get_ref(9)?
					.as_str()
					.map_err(|_| StoreError::Incompatible)?,
			)
			.map_err(|_| StoreError::Incompatible)?
			.0;
			let attachments = serde_json::from_str::<model::AttachmentList>(
				row.get_ref(11)?
					.as_str()
					.map_err(|_| StoreError::Incompatible)?,
			)
			.map_err(|_| StoreError::Incompatible)?
			.0;
			let mentions = serde_json::from_str::<CachedMentions>(
				row.get_ref(12)?
					.as_str()
					.map_err(|_| StoreError::Incompatible)?,
			)
			.map_err(|_| StoreError::Incompatible)?
			.0;
			if !model::valid_mentions(&mentions)
				|| !model::valid_embeds(&embeds)
				|| !model::valid_attachments(&attachments)
			{
				return Err(StoreError::Capacity);
			}
			let parse = |value: String| value.parse::<Id>().map_err(|_| StoreError::Incompatible);
			let author_roles = parse_author_roles(
				row.get_ref(19)?
					.as_str()
					.map_err(|_| StoreError::Incompatible)?,
			)?;
			let author_nick = match row.get_ref(20)? {
				rusqlite::types::ValueRef::Null => None,
				rusqlite::types::ValueRef::Text(bytes) => {
					let nick = std::str::from_utf8(bytes).map_err(|_| StoreError::Incompatible)?;
					if nick.is_empty() {
						None
					} else {
						Some(nick.chars().take(128).collect())
					}
				}
				_ => return Err(StoreError::Incompatible),
			};
			let flags_text = row
				.get_ref(23)?
				.as_str()
				.map_err(|_| StoreError::Incompatible)?;
			if flags_text.is_empty() || !flags_text.bytes().all(|b| b.is_ascii_digit()) {
				return Err(StoreError::Incompatible);
			}
			let flags = flags_text
				.parse::<u64>()
				.map_err(|_| StoreError::Incompatible)?;
			if flags & 64 != 0 {
				return Err(StoreError::Incompatible);
			}
			let message = Message {
				sticker_items: serde_json::from_str::<
					model::StickerList<{ model::MAX_MESSAGE_STICKERS }>,
				>(
					row.get_ref(24)?
						.as_str()
						.map_err(|_| StoreError::Incompatible)?,
				)
				.map_err(|_| StoreError::Incompatible)?
				.0,
				flags,
				ephemeral: false,
				application_id: row.get::<_, Option<String>>(22)?.map(parse).transpose()?,
				components: serde_json::from_str::<model::ComponentList>(
					row.get_ref(21)?
						.as_str()
						.map_err(|_| StoreError::Incompatible)?,
				)
				.map_err(|_| StoreError::Incompatible)?
				.0,
				reactions: None,
				id: parse(row.get(0)?)?,
				channel,
				author: User {
					primary_guild: None,
					id: parse(row.get(1)?)?,
					name: row.get(2)?,
					avatar: row.get(7)?,
					webhook,
					kind: account_kind,
					discriminator: row.get(8)?,
				},
				content: row.get(3)?,
				edited: row.get(4)?,
				edited_at: None,
				reply_to: row.get::<_, Option<String>>(5)?.map(parse).transpose()?,
				unsupported: row.get(6)?,
				extra_content,
				kind: row.get(14)?,
				reply_deleted,
				forwarded: row.get(18)?,
				nonce: None,
				revision: 0,
				embeds,
				author_nick,
				author_roles,
				mention_roles: vec![],
				mention_everyone: false,
				suppress_notifications: false,
				mentions,
				embeds_suppressed: row.get(10)?,
				attachments,
			};
			if message.reply_deleted
				&& (!matches!(message.kind, 19 | 23)
					|| !message
						.reply_to
						.is_some_and(|id| id.0 > 0 && id < message.id))
			{
				return Err(StoreError::Incompatible);
			}
			bytes += message.bytes();
			if bytes > MAX_WINDOW_BYTES {
				return Err(StoreError::Capacity);
			}
			messages.push(message);
		}
		Ok(messages)
	}
	pub fn save_draft(&mut self, account: Id, channel: Id, content: &str) -> Result<()> {
		if content.len() > 8192 {
			return Err(StoreError::Capacity);
		}
		let transaction = self.0.transaction()?;
		transaction.execute(
			"DELETE FROM drafts WHERE account=?1 AND channel=?2",
			params![account.to_string(), channel.to_string()],
		)?;
		if !content.is_empty() {
			transaction.execute(
				"INSERT INTO drafts VALUES(?1,?2,?3)",
				params![account.to_string(), channel.to_string(), content],
			)?;
		}
		let (count, bytes): (i64, i64) = transaction.query_row(
			"SELECT count(*),coalesce(sum(length(CAST(content AS BLOB))),0) FROM drafts",
			[],
			|r| Ok((r.get(0)?, r.get(1)?)),
		)?;
		if count > 64 || bytes > 2 * 1024 * 1024 {
			return Err(StoreError::Capacity);
		}
		transaction.commit()?;
		Ok(())
	}
	pub fn load_drafts(&self, account: Id) -> Result<BTreeMap<Id, String>> {
		let mut query = self
			.0
			.prepare("SELECT channel,content FROM drafts WHERE account=?1 LIMIT 64")?;
		let rows = query.query_map([account.to_string()], |r| {
			Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
		})?;
		let mut drafts = BTreeMap::new();
		let mut bytes = 0;
		for row in rows {
			let (id, content) = row?;
			bytes += content.len();
			if bytes > 2 * 1024 * 1024 || content.len() > 8192 {
				return Err(StoreError::Capacity);
			}
			drafts.insert(id.parse().map_err(|_| StoreError::Incompatible)?, content);
		}
		Ok(drafts)
	}
	pub fn clear_history(&mut self, account: Id) -> Result<()> {
		let transaction = self.0.transaction()?;
		transaction.execute(
			"DELETE FROM messages WHERE account=?1",
			[account.to_string()],
		)?;
		transaction.execute(
			"DELETE FROM channels WHERE account=?1",
			[account.to_string()],
		)?;
		transaction.commit()?;
		self.0
			.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA incremental_vacuum(64);")?;
		Ok(())
	}
	pub fn delete_messages(&mut self, account: Id, channel: Id, ids: &[Id]) -> Result<()> {
		if ids.len() > 100 || account.0 == 0 || channel.0 == 0 || ids.iter().any(|id| id.0 == 0) {
			return Err(StoreError::Capacity);
		}
		let transaction = self.0.transaction()?;
		{
			let mut statement = transaction
				.prepare("DELETE FROM messages WHERE account=?1 AND channel=?2 AND id=?3")?;
			for id in ids {
				statement.execute(params![
					account.to_string(),
					channel.to_string(),
					id.to_string()
				])?;
			}
		}
		transaction.commit()?;
		self.0
			.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA incremental_vacuum(64);")?;
		Ok(())
	}
	/// Saved GIF favorites for one account, newest first. Rejected rows are skipped.
	pub fn gif_favorites(&self, account: Id) -> Result<Vec<model::Gif>> {
		let mut statement = self.0.prepare(
			"SELECT id,title,url,preview,width,height FROM gif_favorites WHERE account=?1 ORDER BY position LIMIT 100",
		)?;
		let rows = statement.query_map([account.to_string()], |row| {
			Ok(model::Gif {
				id: row.get(0)?,
				title: row.get(1)?,
				url: row.get(2)?,
				preview: row.get(3)?,
				width: row.get::<_, u32>(4)?,
				height: row.get::<_, u32>(5)?,
			})
		})?;
		let mut favorites = Vec::new();
		for gif in rows {
			let gif = gif?;
			if gif.valid()
				&& !favorites
					.iter()
					.any(|known: &model::Gif| known.id == gif.id)
			{
				favorites.push(gif);
			}
		}
		Ok(favorites)
	}
	/// Replaces the account's favorites atomically; the list is bounded like the in-memory one.
	pub fn save_gif_favorites(&mut self, account: Id, favorites: &[model::Gif]) -> Result<()> {
		if favorites.len() > model::MAX_GIF_FAVORITES || !favorites.iter().all(model::Gif::valid) {
			return Err(StoreError::Capacity);
		}
		let transaction = self.0.transaction()?;
		transaction.execute(
			"DELETE FROM gif_favorites WHERE account=?1",
			[account.to_string()],
		)?;
		for (position, gif) in favorites.iter().enumerate() {
			transaction.execute(
				"INSERT INTO gif_favorites VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
				rusqlite::params![
					account.to_string(),
					position as i64,
					gif.id,
					gif.title,
					gif.url,
					gif.preview,
					gif.width,
					gif.height
				],
			)?;
		}
		transaction.commit()?;
		Ok(())
	}
	/// Switcher roster, most recently used first. Damaged rows are skipped, never fatal.
	pub fn accounts(&self) -> Result<Vec<model::SavedAccount>> {
		Ok(self.ordered_accounts()?.0)
	}
	/// Valid rows newest first, bounded, plus the IDs of every row that cannot produce a
	/// usable account. Filtering happens before the bound, so a row the switcher could never
	/// offer — a damaged ID or avatar written outside this client — cannot hide a real one.
	fn ordered_accounts(&self) -> Result<(Vec<model::SavedAccount>, Vec<String>)> {
		let mut query = self.0.prepare(
			"SELECT account,name,display,avatar,discriminator,has_token FROM accounts ORDER BY touched DESC,account",
		)?;
		let mut rows = query.query([])?;
		let mut accounts = Vec::new();
		let mut damaged = Vec::new();
		while let Some(row) = rows.next()? {
			let stored: String = row.get(0)?;
			let account = row.get::<_, String>(0)?.parse::<Id>().ok().map(|id| {
				Ok::<_, rusqlite::Error>(model::SavedAccount {
					id,
					name: row.get(1)?,
					display: row.get(2)?,
					avatar: row.get(3)?,
					discriminator: row.get::<_, i64>(4)?.clamp(0, 9999) as u16,
					has_token: row.get::<_, i64>(5)? == 1,
				})
			});
			match account {
				Some(account) if account.as_ref().is_ok_and(model::SavedAccount::is_valid) => {
					accounts.push(account?);
				}
				_ => damaged.push(stored),
			}
		}
		accounts.truncate(model::MAX_SAVED_ACCOUNTS);
		Ok((accounts, damaged))
	}
	/// Records whether the credential store holds this account's own entry. Separate from
	/// `save_account` so an identity refresh can never claim a token that was never written.
	pub fn set_account_token(&self, account: Id, has_token: bool) -> Result<()> {
		self.0.execute(
			"UPDATE accounts SET has_token=?2 WHERE account=?1",
			params![account.to_string(), i64::from(has_token)],
		)?;
		Ok(())
	}
	/// Remembers one account and returns the IDs pruned to keep the roster bounded.
	/// Callers own removing the pruned accounts' credential-store entries.
	pub fn save_account(&mut self, account: &model::SavedAccount) -> Result<Vec<Id>> {
		if !account.is_valid() {
			return Err(StoreError::Capacity);
		}
		let transaction = self.0.transaction()?;
		transaction.execute(
			"INSERT INTO accounts(account,name,display,avatar,discriminator,touched) VALUES(?1,?2,?3,?4,?5,
             MAX(unixepoch('subsec')*1000,(SELECT IFNULL(MAX(touched),0)+1 FROM accounts)))
             ON CONFLICT(account) DO UPDATE SET name=excluded.name,display=excluded.display,avatar=excluded.avatar,discriminator=excluded.discriminator,touched=excluded.touched",
			params![
				account.id.to_string(),
				account.name,
				account.display,
				account.avatar,
				account.discriminator
			],
		)?;
		transaction.commit()?;
		// Keep the newest valid rows and drop everything else, damaged rows included, so a row
		// that cannot be offered never costs a real account its place.
		let (keep, damaged) = self.ordered_accounts()?;
		let keep: std::collections::BTreeSet<Id> = keep.into_iter().map(|a| a.id).collect();
		let transaction = self.0.transaction()?;
		let mut pruned = Vec::new();
		{
			let mut query = transaction.prepare("SELECT account FROM accounts")?;
			let mut rows = query.query([])?;
			while let Some(row) = rows.next()? {
				let stored: String = row.get(0)?;
				match stored.parse::<Id>() {
					Ok(id) if keep.contains(&id) => {}
					Ok(id) => pruned.push(id),
					Err(_) => {}
				}
			}
		}
		for id in pruned.iter().map(Id::to_string).chain(damaged) {
			transaction.execute("DELETE FROM accounts WHERE account=?1", [id])?;
		}
		transaction.commit()?;
		Ok(pruned)
	}
	pub fn forget_account(&mut self, account: Id) -> Result<()> {
		let transaction = self.0.transaction()?;
		for table in [
			"messages",
			"channels",
			"drafts",
			"gif_favorites",
			"channel_preferences",
			"accounts",
		] {
			transaction.execute(
				&format!("DELETE FROM {table} WHERE account=?1"),
				[account.to_string()],
			)?;
		}
		transaction.commit()?;
		self.0
			.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA incremental_vacuum(64);")?;
		Ok(())
	}
}
#[cfg(test)]
mod tests {
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
		store.save_channel(Id(1), Id(2), &messages).unwrap();
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
		store.save_channel(Id(1), Id(2), &[message]).unwrap();
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
		] {
			message.author.kind = kind;
			store
				.save_channel(Id(1), Id(2), &[message.clone()])
				.unwrap();
			assert_eq!(
				store.load_channel(Id(1), Id(2)).unwrap()[0].author.kind,
				kind
			);
		}
		store
			.0
			.execute_batch("ALTER TABLE messages DROP COLUMN account_kind; PRAGMA user_version=13;")
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
	fn app_preferences_round_trip_and_reject_invalid_replacement() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		assert_eq!(store.app_preferences().unwrap(), AppPreferences::default());
		let legacy: AppPreferences =
			serde_json::from_str(r#"{"voice_noise_suppression":true}"#).unwrap();
		assert!(legacy.voice_processing.is_none());
		assert!(legacy.voice_noise_suppression);
		let mut value = AppPreferences {
			notifications_enabled: true,
			hide_title_bar: true,
			primary_color: Some([80, 120, 220]),
			transparency_blur: true,
			transparency: 30,
			blur: 60,
			transparent_all: true,
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
	}
	#[test]
	fn app_preferences_tolerate_an_unknown_gpu_preference() {
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
		store.save_channel(Id(1), Id(2), &messages).unwrap();
		for target in [None, Some(Id(0)), Some(Id(100)), Some(Id(101))] {
			messages[0].reply_to = target;
			assert_eq!(
				store.save_channel(Id(1), Id(2), &messages),
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
			animate_gifs: false,
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
			store.save_channel(Id(1), Id(2), &messages).unwrap();
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
		store.save_channel(Id(1), Id(2), &messages).unwrap();
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
	}

	#[test]
	fn game_activity_rejects_corrupt_values_and_storage_failure() {
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
			animate_gifs: false,
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
				animate_gifs: false,
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
		assert!(store.gif_favorites(Id(2)).unwrap().is_empty());
		store.save_gif_favorites(Id(1), &[]).unwrap();
		assert!(store.gif_favorites(Id(1)).unwrap().is_empty());
	}

	#[test]
	fn hide_media_links_migrates_enabled_and_round_trips_disabled() {
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

	#[test]
	fn gif_animation_migrates_off_and_round_trips() {
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
	#[test]
	fn reading_preferences_validate_storage_types_bounds_and_atomic_replacement() {
		let store = LocalStore::initialize(Connection::open_in_memory().unwrap()).unwrap();
		for (zoom_percent, sidebar_width) in [(80, 190), (150, 360)] {
			for show_members in [false, true] {
				let preferences = ReadingPreferences {
					zoom_percent,
					sidebar_width,
					show_members,
					animate_gifs: false,
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
			(79, 236),
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
					animate_gifs: false,
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
				animate_gifs: false,
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
			"zoom_percent=79",
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
		store.save_channel(Id(1), Id(2), &messages).unwrap();
		drop(store);
		let store = LocalStore::open(&path).unwrap();
		let reopened = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(reopened.len(), 32);
		for (actual, expected) in reopened.iter().zip(&messages) {
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
		assert!(messages[0].reactions.is_none()); // Session-only counts must be revalidated.
		assert!(messages[0].mentions.is_empty());
		assert_eq!(store.load_drafts(Id(1)).unwrap()[&Id(2)], "kept draft");
		messages[0].mentions = vec![User {
			primary_guild: None,
			id: Id(5),
			name: "Mentioned user".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		}];
		store.save_channel(Id(1), Id(2), &messages).unwrap();
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
		store.save_channel(Id(1), Id(2), &loaded).unwrap();
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
				flags: 0,
				sticker_items: vec![],
				components: vec![],
				application_id: None,
				ephemeral: false,
				reactions: Some(vec![]),
				id: Id(100),
				channel: Id(channel),
				author: User {
					primary_guild: None,
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
			store.save_channel(Id(1), Id(channel), &[message]).unwrap();
		}
		let count: i64 = store
			.0
			.query_row("SELECT count(*) FROM channels", [], |r| r.get(0))
			.unwrap();
		assert_eq!(count, 20);
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
		// A failed logout must leave the whole account transaction intact.
		store.0.execute_batch("CREATE TRIGGER reject_draft_delete BEFORE DELETE ON drafts BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;").unwrap();
		assert_eq!(store.forget_account(Id(1)), Err(StoreError::Unavailable));
		assert!(!store.load_channel(Id(1), Id(30)).unwrap().is_empty());
		assert!(!store.load_drafts(Id(1)).unwrap().is_empty());
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
		store.save_channel(Id(1), Id(2), &messages).unwrap();
		store = LocalStore::initialize(store.0).unwrap();
		let loaded = store.load_channel(Id(1), Id(2)).unwrap();
		assert_eq!(loaded[0].components, messages[0].components);
		assert_eq!(loaded[0].application_id, Some(Id(4)));
		assert_eq!(loaded[0].flags, !64);
		messages[0].ephemeral = true;
		messages[0].content = "private synthetic reply".into();
		assert_eq!(
			store.save_channel(Id(1), Id(2), &messages),
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
		store.save_channel(Id(1), Id(2), &messages).unwrap();
		store = LocalStore::initialize(store.0).unwrap();
		assert_eq!(
			store.load_channel(Id(1), Id(2)).unwrap()[0].sticker_items,
			messages[0].sticker_items
		);
		messages[0].sticker_items[0].name = "x".repeat(121);
		assert_eq!(
			store.save_channel(Id(1), Id(2), &messages),
			Err(StoreError::Capacity)
		);
	}
}
