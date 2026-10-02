//! Bounded metadata for default and current-guild soundboard sounds.
use crate::Id;
pub const MAX_SOUNDS: usize = 128;
pub const MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
	pub id: Id,
	pub name: String,
	pub volume: f64,
	pub emoji: Option<String>,
	pub emoji_id: Option<Id>,
	pub guild: Option<Id>,
	pub available: bool,
}
impl Sound {
	pub fn bytes(&self) -> usize {
		size_of::<Self>() + self.name.capacity() + self.emoji.as_ref().map_or(0, String::capacity)
	}
	pub fn valid(&self, guild: Id) -> bool {
		self.id.0 != 0
			&& (2..=32).contains(&self.name.chars().count())
			&& self.name.len() <= 128
			&& !self.name.chars().any(char::is_control)
			&& self.volume.is_finite()
			&& (0.0..=1.0).contains(&self.volume)
			&& self.guild.is_none_or(|id| id == guild && id.0 != 0)
			&& self.emoji_id.is_none_or(|id| id.0 != 0)
			&& self.emoji.as_ref().is_none_or(|emoji| {
				!emoji.is_empty()
					&& emoji.len() <= 128
					&& emoji.chars().count() <= 32
					&& !emoji.chars().any(char::is_control)
			}) && !(self.emoji_id.is_some() && self.emoji.is_some())
	}
}
pub fn catalog_bytes(sounds: &[Sound]) -> usize {
	sounds.iter().map(Sound::bytes).sum()
}
pub fn valid_catalog(sounds: &[Sound], guild: Id) -> bool {
	sounds.len() <= MAX_SOUNDS
		&& catalog_bytes(sounds) <= MAX_BYTES
		&& sounds.iter().enumerate().all(|(index, sound)| {
			sound.valid(guild) && !sounds[..index].iter().any(|other| other.id == sound.id)
		})
}
