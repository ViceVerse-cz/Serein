//! Session-only, bounded soundboard catalogs.
//!
//! Playback is local: triggering a sound only asks the server to fan out an effect, and every
//! client in the channel fetches and mixes the audio itself. Nothing here retains audio bytes;
//! the decoded-sound cache lives with the voice mixer.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Id, SoundboardSound};

/// Guild catalogs retained at once. A session can visit many guilds; only the recently used
/// catalogs stay resident.
pub const MAX_GUILD_CATALOGS: usize = 16;
/// Retained catalog text across every resident guild plus the defaults.
pub const MAX_RESIDENT_BYTES: usize = 256 * 1024;
/// Sounds kept in the recently-played row.
pub const MAX_RECENT: usize = 16;

/// A pending upload, validated before it leaves the client.
#[derive(Clone, Debug, PartialEq)]
pub struct Upload {
	pub name: String,
	/// Raw file bytes; MP3 and OGG are the documented formats.
	pub audio: Vec<u8>,
	pub content_type: &'static str,
	pub volume: f32,
	pub emoji_id: Option<Id>,
	pub emoji_name: Option<String>,
}

impl Upload {
	/// Recognises the documented upload formats from their magic bytes.
	///
	/// The extension is not trusted: the server rejects a mislabelled file, and sending one
	/// wastes an upload round trip.
	pub fn content_type_of(audio: &[u8]) -> Option<&'static str> {
		if audio.starts_with(b"OggS") {
			return Some("audio/ogg");
		}
		// An MP3 is either ID3-tagged or starts at a frame sync.
		if audio.starts_with(b"ID3")
			|| (audio.len() >= 2 && audio[0] == 0xFF && (audio[1] & 0xE0) == 0xE0)
		{
			return Some("audio/mpeg");
		}
		None
	}

	pub fn valid(&self) -> bool {
		model::valid_sound_name(&self.name)
			&& !self.audio.is_empty()
			&& self.audio.len() <= model::MAX_SOUND_UPLOAD_BYTES
			&& matches!(self.content_type, "audio/ogg" | "audio/mpeg")
			&& self.volume.is_finite()
			&& (0.0..=1.0).contains(&self.volume)
			&& self.emoji_id.is_none_or(|id| id.0 != 0)
			&& self
				.emoji_name
				.as_ref()
				.is_none_or(|n| !n.is_empty() && n.len() <= model::MAX_SOUND_EMOJI_NAME)
	}

	pub fn heap_bytes(&self) -> usize {
		self.name.capacity()
			+ self.audio.capacity()
			+ self
				.emoji_name
				.as_ref()
				.map_or(0, std::string::String::capacity)
	}
}

/// One guild's resident catalog.
pub struct GuildCatalog {
	pub guild: Id,
	pub sounds: Vec<SoundboardSound>,
	pub loading: bool,
	pub error: Option<&'static str>,
}

#[derive(Default)]
pub struct Soundboard {
	/// Sounds available in every channel, including DM calls.
	pub defaults: Vec<SoundboardSound>,
	pub defaults_loading: bool,
	pub defaults_loaded: bool,
	pub defaults_error: Option<&'static str>,
	/// Most recently used catalog first.
	pub guilds: Vec<GuildCatalog>,
	/// Sounds this session played, most recent first.
	pub recent: Vec<SoundboardSound>,
	/// Set while a trigger is in flight, so a double click does not fan out twice.
	pub sending: Option<Id>,
	pub send_error: Option<&'static str>,
	pub upload_in_flight: bool,
	pub upload_error: Option<&'static str>,
	/// Reports a failed catalog change, such as a delete the server refused.
	pub manage_error: Option<&'static str>,
}

pub fn resident_bytes(board: &Soundboard) -> usize {
	model::soundboard_bytes(&board.defaults)
		+ model::soundboard_bytes(&board.recent)
		+ board.guilds.capacity() * size_of::<GuildCatalog>()
		+ board
			.guilds
			.iter()
			.map(|c| model::soundboard_bytes(&c.sounds))
			.sum::<usize>()
}

impl Soundboard {
	pub fn catalog(&self, guild: Id) -> Option<&GuildCatalog> {
		self.guilds.iter().find(|c| c.guild == guild)
	}

	fn entry(&mut self, guild: Id) -> &mut GuildCatalog {
		if let Some(index) = self.guilds.iter().position(|c| c.guild == guild) {
			// Keep the most recently touched catalog first so eviction drops a cold one.
			let catalog = self.guilds.remove(index);
			self.guilds.insert(0, catalog);
		} else {
			self.guilds.insert(
				0,
				GuildCatalog {
					guild,
					sounds: Vec::new(),
					loading: false,
					error: None,
				},
			);
		}
		self.trim();
		&mut self.guilds[0]
	}

	/// Drops cold catalogs until both the count and the byte budget hold.
	fn trim(&mut self) {
		self.guilds.truncate(MAX_GUILD_CATALOGS);
		while self.guilds.len() > 1 && resident_bytes(self) > MAX_RESIDENT_BYTES {
			self.guilds.pop();
		}
	}

	/// All sounds playable in a channel: the guild's own catalog, then the defaults.
	pub fn playable(&self, guild: Option<Id>) -> impl Iterator<Item = &SoundboardSound> {
		guild
			.and_then(|id| self.catalog(id))
			.into_iter()
			.flat_map(|c| c.sounds.iter())
			.chain(self.defaults.iter())
			.filter(|sound| sound.available)
	}

	/// Resolves a sound id to the guild that must be named as its source, if any.
	pub fn source_guild(&self, sound: Id) -> Option<Id> {
		self.guilds
			.iter()
			.find(|c| c.sounds.iter().any(|s| s.id == sound))
			.map(|c| c.guild)
	}
}

impl State {
	pub(crate) fn interrupt_soundboard(&mut self) {
		if self.soundboard.defaults_loading {
			self.soundboard.defaults_loading = false;
			self.soundboard.defaults_error = Some("Soundboard loading interrupted; try again");
		}
		for catalog in &mut self.soundboard.guilds {
			if catalog.loading {
				catalog.loading = false;
				catalog.error = Some("Soundboard loading interrupted; try again");
			}
		}
		if self.soundboard.sending.take().is_some() {
			self.soundboard.send_error = Some("Sound interrupted; try again");
		}
		if self.soundboard.upload_in_flight {
			self.soundboard.upload_in_flight = false;
			self.soundboard.upload_error = Some("Upload interrupted; try again");
		}
	}

	fn ready(&self) -> bool {
		self.auth == AuthState::Authenticated && self.gateway_connected
	}

	pub fn request_default_sounds(&mut self) -> Option<Command> {
		if !self.ready() || self.soundboard.defaults_loading || self.soundboard.defaults_loaded {
			return None;
		}
		self.soundboard.defaults_loading = true;
		self.soundboard.defaults_error = None;
		Some(Command::DefaultSoundboardSounds)
	}

	pub fn request_guild_sounds(&mut self, guild: Id) -> Option<Command> {
		if guild.0 == 0 || !self.ready() {
			return None;
		}
		let catalog = self.soundboard.entry(guild);
		// A loaded catalog stays until a gateway dispatch replaces it.
		if catalog.loading || !catalog.sounds.is_empty() {
			return None;
		}
		catalog.loading = true;
		catalog.error = None;
		Some(Command::GuildSoundboardSounds(guild))
	}

	/// Asks the server to play `sound` for everyone in `channel`.
	pub fn play_soundboard_sound(&mut self, channel: Id, sound: Id) -> Option<Command> {
		if channel.0 == 0 || sound.0 == 0 || !self.ready() || self.soundboard.sending.is_some() {
			return None;
		}
		let source_guild = self.soundboard.source_guild(sound);
		self.soundboard.sending = Some(sound);
		self.soundboard.send_error = None;
		Some(Command::PlaySoundboardSound {
			channel,
			sound,
			source_guild,
		})
	}

	pub fn upload_soundboard_sound(&mut self, guild: Id, upload: Upload) -> Option<Command> {
		if guild.0 == 0 || !self.ready() || self.soundboard.upload_in_flight || !upload.valid() {
			return None;
		}
		self.soundboard.upload_in_flight = true;
		self.soundboard.upload_error = None;
		Some(Command::CreateSoundboardSound {
			guild,
			upload: Box::new(upload),
		})
	}

	pub fn delete_soundboard_sound(&mut self, guild: Id, sound: Id) -> Option<Command> {
		if guild.0 == 0 || sound.0 == 0 || !self.ready() {
			return None;
		}
		Some(Command::DeleteSoundboardSound { guild, sound })
	}

	pub(crate) fn apply_default_sounds(&mut self, result: Result<Vec<SoundboardSound>, Failure>) {
		self.soundboard.defaults_loading = false;
		match result {
			Ok(sounds) => {
				self.soundboard.defaults = sounds;
				self.soundboard.defaults_loaded = true;
				self.soundboard.defaults_error = None;
				self.soundboard.trim();
			}
			Err(failure) => self.soundboard.defaults_error = Some(failure.label()),
		}
	}

	pub(crate) fn apply_guild_sounds(
		&mut self,
		guild: Id,
		result: Result<Vec<SoundboardSound>, Failure>,
	) {
		if guild.0 == 0 {
			return;
		}
		let catalog = self.soundboard.entry(guild);
		catalog.loading = false;
		match result {
			Ok(sounds) => {
				catalog.sounds = sounds;
				catalog.error = None;
			}
			Err(failure) => catalog.error = Some(failure.label()),
		}
		self.soundboard.trim();
	}

	/// Replaces a guild catalog from a gateway dispatch.
	pub(crate) fn replace_guild_sounds(&mut self, guild: Id, sounds: Vec<SoundboardSound>) {
		if guild.0 == 0 {
			return;
		}
		let catalog = self.soundboard.entry(guild);
		catalog.sounds = sounds;
		catalog.loading = false;
		catalog.error = None;
		self.soundboard.trim();
	}

	/// Applies a single-sound create or update dispatch.
	pub(crate) fn upsert_guild_sound(&mut self, guild: Id, sound: SoundboardSound) {
		if guild.0 == 0 || !sound.valid() {
			return;
		}
		let catalog = self.soundboard.entry(guild);
		match catalog.sounds.iter().position(|s| s.id == sound.id) {
			Some(index) => catalog.sounds[index] = sound,
			// A create beyond the catalog bound is dropped rather than growing it unbounded.
			None if catalog.sounds.len() < model::MAX_GUILD_SOUNDS => catalog.sounds.push(sound),
			None => {}
		}
		self.soundboard.trim();
	}

	pub(crate) fn remove_guild_sound(&mut self, guild: Id, sound: Id) {
		if let Some(catalog) = self.soundboard.guilds.iter_mut().find(|c| c.guild == guild) {
			catalog.sounds.retain(|s| s.id != sound);
		}
		self.soundboard.recent.retain(|s| s.id != sound);
	}

	pub(crate) fn remember_played_sound(&mut self, sound: Id) {
		let Some(played) = self
			.soundboard
			.playable(None)
			.chain(self.soundboard.guilds.iter().flat_map(|c| c.sounds.iter()))
			.find(|s| s.id == sound)
			.cloned()
		else {
			return;
		};
		self.soundboard.recent.retain(|s| s.id != sound);
		self.soundboard.recent.insert(0, played);
		self.soundboard.recent.truncate(MAX_RECENT);
		self.soundboard.trim();
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sound(id: u64, guild: Option<u64>) -> SoundboardSound {
		SoundboardSound {
			id: Id(id),
			name: format!("sound-{id}"),
			volume: 1.0,
			emoji_id: None,
			emoji_name: None,
			guild_id: guild.map(Id),
			available: true,
			user_id: None,
		}
	}

	#[test]
	fn resident_catalogs_are_bounded_and_evict_the_coldest_guild() {
		let mut board = Soundboard::default();
		for guild in 1..=u64::try_from(MAX_GUILD_CATALOGS).expect("a small bound") + 4 {
			board.entry(Id(guild)).sounds = vec![sound(guild, Some(guild))];
		}
		assert_eq!(board.guilds.len(), MAX_GUILD_CATALOGS);
		// The most recently touched guild is retained and the first-touched is evicted.
		assert!(board.catalog(Id(20)).is_some());
		assert!(board.catalog(Id(1)).is_none());
		assert!(resident_bytes(&board) <= MAX_RESIDENT_BYTES);

		// Touching a cold catalog promotes it rather than duplicating it.
		let before = board.guilds.len();
		board.entry(Id(10));
		assert_eq!(board.guilds.len(), before);
		assert_eq!(board.guilds[0].guild, Id(10));
	}

	#[test]
	fn playable_prefers_the_guild_catalog_and_hides_unavailable_sounds() {
		let mut board = Soundboard {
			defaults: vec![sound(99, None)],
			..Default::default()
		};
		let mut unavailable = sound(2, Some(7));
		unavailable.available = false;
		board.entry(Id(7)).sounds = vec![sound(1, Some(7)), unavailable];

		let ids: Vec<_> = board.playable(Some(Id(7))).map(|s| s.id).collect();
		assert_eq!(ids, vec![Id(1), Id(99)]);
		// Outside a guild only the defaults play.
		assert_eq!(
			board.playable(None).map(|s| s.id).collect::<Vec<_>>(),
			vec![Id(99)]
		);
		// A default sound must not be attributed to a guild when sent.
		assert_eq!(board.source_guild(Id(99)), None);
		assert_eq!(board.source_guild(Id(1)), Some(Id(7)));
	}

	#[test]
	fn a_create_dispatch_cannot_grow_a_catalog_past_its_bound() {
		let mut board = Soundboard::default();
		let full: Vec<_> = (1..=u64::try_from(model::MAX_GUILD_SOUNDS).expect("a small bound"))
			.map(|id| sound(id, Some(7)))
			.collect();
		board.entry(Id(7)).sounds = full;
		let mut state = State {
			soundboard: board,
			..Default::default()
		};

		state.upsert_guild_sound(Id(7), sound(9999, Some(7)));
		assert_eq!(
			state
				.soundboard
				.catalog(Id(7))
				.expect("the catalog")
				.sounds
				.len(),
			model::MAX_GUILD_SOUNDS
		);

		// An update of a resident sound still applies at the bound.
		let mut renamed = sound(1, Some(7));
		renamed.name = "renamed".to_owned();
		state.upsert_guild_sound(Id(7), renamed);
		assert_eq!(
			state.soundboard.catalog(Id(7)).expect("the catalog").sounds[0].name,
			"renamed"
		);
	}

	#[test]
	fn upload_detects_its_format_from_content_and_bounds_the_file() {
		assert_eq!(Upload::content_type_of(b"OggS\0\0"), Some("audio/ogg"));
		assert_eq!(Upload::content_type_of(b"ID3\x04\0\0"), Some("audio/mpeg"));
		assert_eq!(
			Upload::content_type_of(&[0xFF, 0xFB, 0x90]),
			Some("audio/mpeg")
		);
		// A mislabelled file the server would reject never leaves the client.
		assert_eq!(Upload::content_type_of(b"RIFF....WAVE"), None);
		assert_eq!(Upload::content_type_of(b""), None);

		let upload = |audio: Vec<u8>, name: &str| Upload {
			name: name.to_owned(),
			audio,
			content_type: "audio/ogg",
			volume: 1.0,
			emoji_id: None,
			emoji_name: None,
		};
		assert!(upload(b"OggS".to_vec(), "quack").valid());
		assert!(!upload(vec![b'x'; model::MAX_SOUND_UPLOAD_BYTES + 1], "quack").valid());
		assert!(!upload(Vec::new(), "quack").valid());
		// Discord rejects a one-character name.
		assert!(!upload(b"OggS".to_vec(), "q").valid());
		assert!(!upload(b"OggS".to_vec(), &"q".repeat(model::MAX_SOUND_NAME + 1)).valid());
	}
}
