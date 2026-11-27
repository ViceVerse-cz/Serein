//! Bounded soundboard entities.
//!
//! Discord documents soundboard sounds as a guild-scoped catalog plus a small set of default
//! sounds available everywhere. Playback is local: a `VOICE_CHANNEL_EFFECT_SEND` dispatch names a
//! sound, and every client in the channel fetches and mixes it itself. The sound payload is never
//! carried on the voice transport, so the bounds here cover catalog text only; the audio byte
//! budget lives with the fetcher.
use crate::Id;
use serde::{Deserialize, Serialize};

/// Discord rejects names outside this range when creating a sound.
pub const MIN_SOUND_NAME: usize = 2;
pub const MAX_SOUND_NAME: usize = 32;
/// Unicode emoji names are short; a custom emoji contributes an id instead.
pub const MAX_SOUND_EMOJI_NAME: usize = 64;
/// Documented ceiling for one guild's catalog at the highest boost tier.
pub const MAX_GUILD_SOUNDS: usize = 96;
/// Discord ships a fixed default set; keep headroom without allowing an unbounded list.
pub const MAX_DEFAULT_SOUNDS: usize = 64;
/// Total catalog text retained per guild.
pub const MAX_SOUNDBOARD_BYTES: usize = 64 * 1024;
/// Documented upload ceiling for a sound file.
pub const MAX_SOUND_UPLOAD_BYTES: usize = 512 * 1024;
/// Documented maximum duration of a sound, in seconds.
pub const MAX_SOUND_SECONDS: f32 = 5.2;

/// One soundboard sound.
///
/// Default sounds carry no `guild_id` and their `sound_id` is a small integer rather than a
/// snowflake, so nothing here may assume snowflake shape beyond being non-zero.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SoundboardSound {
	#[serde(rename = "sound_id")]
	pub id: Id,
	#[serde(deserialize_with = "name")]
	pub name: String,
	/// Author-set gain in `0.0..=1.0`; applied on top of the local soundboard volume.
	#[serde(default = "full_volume", deserialize_with = "volume")]
	pub volume: f32,
	#[serde(default)]
	pub emoji_id: Option<Id>,
	#[serde(default, deserialize_with = "emoji_name")]
	pub emoji_name: Option<String>,
	#[serde(default)]
	pub guild_id: Option<Id>,
	#[serde(default = "available")]
	pub available: bool,
	/// Uploader, when the server discloses one. Absent for default sounds.
	#[serde(default)]
	pub user_id: Option<Id>,
}

fn full_volume() -> f32 {
	1.0
}

fn available() -> bool {
	true
}

fn name<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
	struct Name;
	impl<'de> serde::de::Visitor<'de> for Name {
		type Value = String;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("a bounded soundboard sound name")
		}
		fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<String, E> {
			if value.len() > MAX_SOUND_NAME {
				return Err(E::custom("Soundboard name capacity"));
			}
			Ok(value.to_owned())
		}
	}
	d.deserialize_str(Name)
}

fn emoji_name<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
	struct EmojiName;
	impl<'de> serde::de::Visitor<'de> for EmojiName {
		type Value = Option<String>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("an optional bounded soundboard emoji name")
		}
		fn visit_none<E: serde::de::Error>(self) -> Result<Option<String>, E> {
			Ok(None)
		}
		fn visit_unit<E: serde::de::Error>(self) -> Result<Option<String>, E> {
			Ok(None)
		}
		fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Option<String>, D::Error> {
			struct Inner;
			impl<'de> serde::de::Visitor<'de> for Inner {
				type Value = Option<String>;
				fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
					f.write_str("a bounded soundboard emoji name")
				}
				fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Option<String>, E> {
					if value.len() > MAX_SOUND_EMOJI_NAME {
						return Err(E::custom("Soundboard emoji capacity"));
					}
					Ok((!value.is_empty()).then(|| value.to_owned()))
				}
			}
			d.deserialize_str(Inner)
		}
	}
	d.deserialize_option(EmojiName)
}

/// Clamps rather than rejects: an out-of-range gain is a server quirk, not a malformed catalog,
/// and dropping the whole list over it would lose every other sound in the guild.
fn volume<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
	let value = f64::deserialize(d)?;
	if !value.is_finite() {
		return Ok(1.0);
	}
	Ok(value.clamp(0.0, 1.0) as f32)
}

impl SoundboardSound {
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity()
			+ self
				.emoji_name
				.as_ref()
				.map_or(0, std::string::String::capacity)
	}

	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& !self.name.is_empty()
			&& self.name.len() <= MAX_SOUND_NAME
			&& self.volume.is_finite()
			&& (0.0..=1.0).contains(&self.volume)
			&& self
				.emoji_name
				.as_ref()
				.is_none_or(|n| !n.is_empty() && n.len() <= MAX_SOUND_EMOJI_NAME)
			&& self.emoji_id.is_none_or(|id| id.0 != 0)
			&& self.guild_id.is_none_or(|id| id.0 != 0)
			&& self.user_id.is_none_or(|id| id.0 != 0)
	}

	/// A default sound belongs to no guild and stays playable from any channel.
	pub fn is_default(&self) -> bool {
		self.guild_id.is_none()
	}
}

pub fn soundboard_bytes(items: &Vec<SoundboardSound>) -> usize {
	items.capacity() * std::mem::size_of::<SoundboardSound>()
		+ items.iter().map(SoundboardSound::heap_bytes).sum::<usize>()
}

pub fn valid_soundboard_sounds(items: &Vec<SoundboardSound>, max: usize) -> bool {
	items.len() <= max
		&& soundboard_bytes(items) <= MAX_SOUNDBOARD_BYTES
		&& items
			.iter()
			.enumerate()
			.all(|(i, s)| s.valid() && !items[..i].iter().any(|other| other.id == s.id))
}

/// Bounded wire decoder for a soundboard catalog.
pub struct SoundboardList<const MAX: usize>(pub Vec<SoundboardSound>);

impl<'de, const MAX: usize> Deserialize<'de> for SoundboardList<MAX> {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct List<const N: usize>;
		impl<'de, const N: usize> serde::de::Visitor<'de> for List<N> {
			type Value = SoundboardList<N>;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("a bounded soundboard sound list")
			}
			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut seq: A,
			) -> Result<Self::Value, A::Error> {
				let mut items: Vec<SoundboardSound> = Vec::new();
				while let Some(item) = seq.next_element::<SoundboardSound>()? {
					if items.len() >= N || !item.valid() || items.iter().any(|s| s.id == item.id) {
						return Err(serde::de::Error::custom("Invalid soundboard sounds"));
					}
					items.push(item);
					if soundboard_bytes(&items) > MAX_SOUNDBOARD_BYTES {
						return Err(serde::de::Error::custom("Soundboard capacity"));
					}
				}
				Ok(SoundboardList(items))
			}
		}
		d.deserialize_seq(List::<MAX>)
	}
}

/// Validates a candidate name before a create or rename request leaves the client.
pub fn valid_sound_name(name: &str) -> bool {
	let trimmed = name.trim();
	(MIN_SOUND_NAME..=MAX_SOUND_NAME).contains(&trimmed.len())
}

/// One member's soundboard playback in a voice channel, already clamped for mixing.
///
/// The audio is never carried on the voice transport: this names a sound that every client in
/// the channel fetches and mixes locally.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundEffect {
	pub channel: Id,
	pub user: Id,
	pub guild: Option<Id>,
	pub sound: Id,
	/// Sender-side gain in `0.0..=1.0`.
	pub volume: f32,
}
