//! Bounded soundboard metadata. Sound files are fetched separately and never stored here.
//! Reference: https://docs.discord.com/developers/resources/soundboard
use crate::Id;
/// Guilds hold at most 96 sounds; the default set is shared by every account.
pub const MAX_SOUNDS: usize = 256;
pub const MAX_SOUND_BYTES: usize = 128 * 1024;
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
	pub id: Id,
	pub name: String,
	/// Service-configured playback gain, 0 to 1.
	pub volume: f32,
	pub emoji_id: Option<Id>,
	pub emoji_name: Option<String>,
	/// `None` for Discord's default sounds.
	pub guild: Option<Id>,
	pub available: bool,
}
impl Sound {
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity() + self.emoji_name.as_ref().map_or(0, String::capacity)
	}
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& !self.name.is_empty()
			&& self.name.len() <= 128
			&& (0.0..=1.0).contains(&self.volume)
			&& self.emoji_id.is_none_or(|id| id.0 != 0)
			&& self
				.emoji_name
				.as_ref()
				.is_none_or(|name| name.len() <= 128)
			&& self.guild.is_none_or(|id| id.0 != 0)
	}
}
pub fn sound_bytes(items: &Vec<Sound>) -> usize {
	items.capacity() * size_of::<Sound>() + items.iter().map(Sound::heap_bytes).sum::<usize>()
}
pub fn valid_sounds(items: &Vec<Sound>) -> bool {
	items.len() <= MAX_SOUNDS
		&& sound_bytes(items) <= MAX_SOUND_BYTES
		&& items
			.iter()
			.enumerate()
			.all(|(i, s)| s.valid() && !items[..i].iter().any(|other| other.id == s.id))
}
