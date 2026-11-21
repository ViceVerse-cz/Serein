//! Device-local preference types and their existing validation/serialization.
use super::{LocalStore, OptionalExtension, ReadingPreferences, Result, StoreError, params};

/// Device-local controls, bounded independently of account caches.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppPreferences {
	pub window_geometry: Option<WindowGeometry>,
	/// `None` follows the operating-system locale; otherwise this is a bounded BCP 47 tag.
	pub language: Option<String>,
	pub notifications_enabled: bool,
	pub auto_update: bool,
	pub update_nightly: bool,
	pub notification_options: model::notification_preferences::Device,
	pub show_hidden_channels: bool,
	/// Keep custom emoji that need Nitro out of suggestions and lock them in the picker.
	pub hide_nitro_emojis: bool,
	/// Device-local opt-in; older saved preferences deserialize with conversion off.
	pub convert_emoticons: bool,
	/// Request a conversation's page while the pointer rests on it; older preferences enable it.
	pub preload_on_hover: bool,
	/// Hide the in-app title strip. macOS and Windows then show the system title bar.
	pub hide_title_bar: bool,
	pub hide_window_decorations: bool,
	pub primary_color: Option<[u8; 3]>,
	/// Disable app chrome transitions independently of artwork and scrolling.
	pub reduce_interface_motion: bool,
	pub transparency_blur: bool,
	pub transparency: u8,
	pub blur: u8,
	#[serde(default)]
	pub voice_noise_suppression: bool,
	/// Absent in older preferences; migrate using the legacy suppression setting.
	#[serde(default)]
	pub voice_processing: Option<model::voice_settings::VoiceProcessing>,
	pub video_settings: model::voice_settings::VideoSettings,
	pub voice_push_to_talk: bool,
	pub voice_muted: bool,
	pub voice_deafened: bool,
	pub voice_input: Option<String>,
	pub voice_output: Option<String>,
	/// Independent media output; None preserves each player’s existing default routing.
	pub media_output: Option<String>,
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
	/// Profile cosmetics shown on this device; older preferences show all of them.
	pub cosmetics: model::CosmeticPreferences,
	/// Disk budgets and retention for downloaded images, from bounded presets.
	pub image_cache: model::CachePreferences,
	/// Clock style for message times; older preferences follow the language.
	pub time_format: model::TimeFormat,
	/// Device-local timezone; missing or malformed values recover to System.
	pub time_zone: model::TimeZonePreference,
	/// Owner opt-in: keep saved logins in private plain files when no OS keyring works.
	pub plaintext_login: bool,
}

impl Default for AppPreferences {
	fn default() -> Self {
		Self {
			window_geometry: None,
			language: None,
			notifications_enabled: true,
			auto_update: false,
			update_nightly: true,
			notification_options: Default::default(),
			show_hidden_channels: false,
			hide_nitro_emojis: false,
			convert_emoticons: false,
			preload_on_hover: true,
			hide_title_bar: false,
			hide_window_decorations: false,
			primary_color: None,
			reduce_interface_motion: false,
			transparency_blur: false,
			transparency: 15,
			blur: 50,
			voice_noise_suppression: true,
			voice_processing: Some(model::voice_settings::VoiceProcessing::default()),
			video_settings: Default::default(),
			voice_push_to_talk: false,
			voice_muted: false,
			voice_deafened: false,
			voice_input: None,
			voice_output: None,
			media_output: None,
			input_percent: 100,
			output_percent: 100,
			gpu_preference: Default::default(),
			keybinds: Default::default(),
			expanded_folders: Vec::new(),
			user_volumes: Vec::new(),
			muted_users: Vec::new(),
			cosmetics: Default::default(),
			image_cache: Default::default(),
			time_format: Default::default(),
			time_zone: Default::default(),
			plaintext_login: false,
		}
	}
}

impl AppPreferences {
	pub fn is_valid(&self) -> bool {
		self.window_geometry.is_none_or(WindowGeometry::is_valid)
			&& self.language.as_ref().is_none_or(|language| {
				!language.is_empty()
					&& language.len() <= 35
					&& language
						.bytes()
						.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
			}) && self.transparency <= 100
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
			&& self.image_cache.is_valid()
			&& self.time_zone.is_valid()
			&& [&self.voice_input, &self.voice_output, &self.media_output]
				.into_iter()
				.all(|value| value.as_ref().is_none_or(|value| value.len() <= 1024))
	}
}

/// Normal client size in logical pixels; outer position in physical desktop pixels.
/// Position is absent on Wayland, where the compositor owns window placement.
/// While maximized, size and position keep the normal geometry the window restores to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WindowGeometry {
	pub size: [u32; 2],
	pub position: Option<[i32; 2]>,
	/// Absent in older preferences, which were only saved while not maximized.
	#[serde(default)]
	pub maximized: bool,
}

impl WindowGeometry {
	pub fn is_valid(self) -> bool {
		self.size
			.into_iter()
			.all(|value| (1..=16384).contains(&value))
			&& self.position.is_none_or(|position| {
				position
					.into_iter()
					.all(|value| (-131072..=131072).contains(&value))
			})
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
	#[default]
	System,
	Light,
	Dark,
}

impl LocalStore {
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
	/// One device-local font, atomically replaced; account logout leaves it intact.
	pub fn custom_font(&self) -> Result<Option<(String, Vec<u8>)>> {
		self.0.query_row(
			"SELECT
             CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB)) BETWEEN 1 AND 128 THEN name ELSE NULL END,
             CASE WHEN typeof(data)='blob' AND length(data) BETWEEN 1 AND 33554432 THEN data ELSE NULL END
             FROM custom_font WHERE singleton=1",
			[], |row| Ok((row.get(0)?, row.get(1)?)),
		).optional().map_err(Into::into)
	}
	pub fn save_custom_font(&self, font: Option<(&str, &[u8])>) -> Result<()> {
		if let Some((name, bytes)) = font {
			if name.is_empty()
				|| name.len() > 128
				|| bytes.is_empty()
				|| bytes.len() > 32 * 1024 * 1024
			{
				return Err(StoreError::Capacity);
			}
			self.0.execute(
				"INSERT INTO custom_font VALUES(1,?1,?2)
                 ON CONFLICT(singleton) DO UPDATE SET name=excluded.name,data=excluded.data",
				params![name, bytes],
			)?;
		} else {
			self.0.execute("DELETE FROM custom_font", [])?;
		}
		Ok(())
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
				"SELECT zoom_percent,sidebar_width,show_members,animate_gifs,hide_media_links,confirm_external_links,smooth_scrolling,scroll_speed_percent,show_members_dms,compact_messages,double_click_reaction,double_click_reaction_enabled FROM reading_preferences WHERE singleton=1",
				[],
				|row| {
					Ok(match (
						row.get_ref(0)?,
						row.get_ref(1)?,
						row.get_ref(2)?,
						row.get_ref(3)?,
						row.get_ref(4)?,
						row.get_ref(5)?,
						row.get_ref(6)?,
						row.get_ref(7)?,
						row.get_ref(8)?,
						row.get_ref(9)?,
						row.get_ref(10)?,
						row.get_ref(11)?,
					) {
						(
							ValueRef::Integer(zoom @ 50..=150),
							ValueRef::Integer(width @ 190..=360),
							ValueRef::Integer(members @ 0..=1),
							ValueRef::Integer(animate_gifs @ 0..=1),
							ValueRef::Integer(hide_media_links @ 0..=1),
							ValueRef::Integer(confirm_external_links @ 0..=1),
							ValueRef::Integer(smooth_scrolling @ 0..=1),
							ValueRef::Integer(scroll_speed_percent @ 25..=300),
							ValueRef::Integer(members_dms @ 0..=1),
							ValueRef::Integer(compact_messages @ 0..=1),
							ValueRef::Integer(double_click_reaction @ 0..=5),
							ValueRef::Integer(double_click_enabled @ 0..=1),
						) => Some(ReadingPreferences {
							zoom_percent: zoom as u16,
							sidebar_width: width as u16,
							show_members: members == 1,
							show_members_dms: members_dms == 1,
							compact_messages: compact_messages == 1,
							double_click_reaction: double_click_reaction as u8,
							double_click_reaction_enabled: double_click_enabled == 1,
							animate_gifs: animate_gifs == 1,
							hide_media_links: hide_media_links == 1,
							confirm_external_links: confirm_external_links == 1,
							smooth_scrolling: smooth_scrolling == 1,
							scroll_speed_percent: scroll_speed_percent as u16,
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
			self.0.execute("INSERT INTO reading_preferences(singleton,zoom_percent,sidebar_width,show_members,animate_gifs,hide_media_links,confirm_external_links,smooth_scrolling,scroll_speed_percent,show_members_dms,compact_messages,double_click_reaction,double_click_reaction_enabled)
				VALUES(1,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(singleton) DO UPDATE SET
				zoom_percent=excluded.zoom_percent,sidebar_width=excluded.sidebar_width,show_members=excluded.show_members,animate_gifs=excluded.animate_gifs,hide_media_links=excluded.hide_media_links,confirm_external_links=excluded.confirm_external_links,smooth_scrolling=excluded.smooth_scrolling,scroll_speed_percent=excluded.scroll_speed_percent,show_members_dms=excluded.show_members_dms,compact_messages=excluded.compact_messages,double_click_reaction=excluded.double_click_reaction,double_click_reaction_enabled=excluded.double_click_reaction_enabled",
				params![preferences.zoom_percent, preferences.sidebar_width, preferences.show_members, preferences.animate_gifs, preferences.hide_media_links, preferences.confirm_external_links, preferences.smooth_scrolling, preferences.scroll_speed_percent, preferences.show_members_dms, preferences.compact_messages, preferences.double_click_reaction, preferences.double_click_reaction_enabled])?;
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
}
