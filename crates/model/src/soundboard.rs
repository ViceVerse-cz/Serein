//! Discord soundboard sounds and the voice channel effects that play them.
use crate::Id;

/// Discord's documented name limit is 32 characters; the wire bound leaves room for UTF-8.
pub const MAX_SOUND_NAME_BYTES: usize = 128;
/// Boosted servers hold at most 48 sounds; anything beyond this is rejected as malformed.
pub const MAX_GUILD_SOUNDS: usize = 64;
/// Default sounds are a short fixed list.
pub const MAX_DEFAULT_SOUNDS: usize = 64;
/// Discord accepts uploads up to 512 KiB; the CDN response is bounded a little above that.
pub const MAX_SOUND_FILE_BYTES: usize = 1024 * 1024;
/// Uploads are limited to 5.2 seconds; decoding stops at this bound.
pub const MAX_SOUND_SECONDS: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
	pub id: Id,
	pub name: String,
	/// Discord's per-sound playback volume, from 0 to 1.
	pub volume: f32,
	pub emoji_id: Option<Id>,
	pub emoji_name: Option<String>,
	/// None for Discord's default sounds.
	pub guild: Option<Id>,
	/// False when the server lost the boost level that allowed the sound.
	pub available: bool,
}
impl Sound {
	/// Heap and inline bytes this sound keeps, for the catalog budget.
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.name.capacity()
			+ self.emoji_name.as_ref().map_or(0, String::capacity)
	}
	/// Lower-case name match for the picker search.
	pub fn matches(&self, query: &str) -> bool {
		query.is_empty() || self.name.to_lowercase().contains(&query.to_lowercase())
	}
}

/// A soundboard sound someone played in a voice channel (`VOICE_CHANNEL_EFFECT_SEND`).
/// Discord only signals the sound; each client downloads and plays it locally.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundEffect {
	pub channel: Id,
	pub guild: Option<Id>,
	pub user: Id,
	pub sound: Id,
	/// Sender-side volume, from 0 to 1.
	pub volume: f32,
}

/// The CDN location of a sound's original MP3 or Ogg file.
pub fn sound_url(sound: Id) -> String {
	format!("https://cdn.discordapp.com/soundboard-sounds/{sound}")
}
