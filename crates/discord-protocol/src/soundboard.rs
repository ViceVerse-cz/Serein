//! Documented soundboard payloads: sound objects, catalog events and voice channel effects.
//! <https://discord.com/developers/docs/resources/soundboard>
use model::{
	Id,
	soundboard::{MAX_DEFAULT_SOUNDS, MAX_GUILD_SOUNDS, MAX_SOUND_NAME_BYTES, Sound, SoundEffect},
};
use serde::Deserialize;

/// A snowflake that Discord may send as a string or, for default sounds in
/// `VOICE_CHANNEL_EFFECT_SEND`, as a JSON integer.
struct FlexibleId(Id);
impl<'de> Deserialize<'de> for FlexibleId {
	/// Accept a snowflake string or a positive integer.
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct Visitor;
		impl serde::de::Visitor<'_> for Visitor {
			type Value = FlexibleId;
			/// Describes the accepted forms in decode errors.
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("a Discord ID string or positive integer")
			}
			/// A snowflake string, as for server sounds.
			fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<FlexibleId, E> {
				value.parse().map(FlexibleId).map_err(E::custom)
			}
			/// A JSON integer, as Discord sends for default sounds; zero is not an ID.
			fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<FlexibleId, E> {
				if value == 0 {
					return Err(E::custom("Invalid Discord ID"));
				}
				Ok(FlexibleId(Id(value)))
			}
			/// A signed JSON integer; only positive values are IDs.
			fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<FlexibleId, E> {
				u64::try_from(value)
					.map_err(|_| E::custom("Invalid Discord ID"))
					.and_then(|value| self.visit_u64(value))
			}
		}
		d.deserialize_any(Visitor)
	}
}

#[derive(Deserialize)]
struct SoundDto {
	name: String,
	sound_id: FlexibleId,
	#[serde(default = "full_volume")]
	volume: f64,
	#[serde(default)]
	emoji_id: Option<Id>,
	#[serde(default)]
	emoji_name: Option<String>,
	#[serde(default)]
	guild_id: Option<Id>,
	#[serde(default = "available")]
	available: bool,
}
/// Discord omits `volume` for sounds at full volume.
fn full_volume() -> f64 {
	1.0
}
/// Discord omits `available` for sounds that can be played.
fn available() -> bool {
	true
}
impl SoundDto {
	/// Validate the name and emoji bounds and convert to the model sound.
	fn into_model(self) -> Result<Sound, crate::DecodeError> {
		let name = self.name.trim();
		if name.is_empty()
			|| name.len() > MAX_SOUND_NAME_BYTES
			|| self
				.emoji_name
				.as_ref()
				.is_some_and(|emoji| emoji.len() > MAX_SOUND_NAME_BYTES)
		{
			return Err(crate::DecodeError);
		}
		Ok(Sound {
			id: self.sound_id.0,
			name: name.to_owned(),
			volume: volume(self.volume),
			emoji_id: self.emoji_id,
			emoji_name: self.emoji_name.filter(|emoji| !emoji.is_empty()),
			guild: self.guild_id,
			available: self.available,
		})
	}
}
/// Clamp a wire volume to 0–1; a non-finite value means full volume.
fn volume(value: f64) -> f32 {
	if value.is_finite() {
		value.clamp(0.0, 1.0) as f32
	} else {
		1.0
	}
}
/// Convert a catalog of at most `limit` sounds, binding each to `guild` (`None` for default
/// sounds) and skipping repeated IDs. A sound claiming another server rejects the catalog.
fn list(
	sounds: Vec<SoundDto>,
	guild: Option<Id>,
	limit: usize,
) -> Result<Vec<Sound>, crate::DecodeError> {
	if sounds.len() > limit {
		return Err(crate::DecodeError);
	}
	let mut out = Vec::with_capacity(sounds.len());
	for sound in sounds {
		let mut sound = sound.into_model()?;
		// A listed sound always belongs to the guild the list is for; default sounds have none.
		if sound.guild.is_some_and(|id| Some(id) != guild) {
			return Err(crate::DecodeError);
		}
		sound.guild = guild;
		if out.iter().any(|known: &Sound| known.id == sound.id) {
			continue;
		}
		out.push(sound);
	}
	Ok(out)
}

/// `GET /soundboard-default-sounds`: a bare array of sound objects.
pub fn default_sounds(bytes: &[u8]) -> Result<Vec<Sound>, crate::DecodeError> {
	list(crate::decode(bytes)?, None, MAX_DEFAULT_SOUNDS)
}

/// `SOUNDBOARD_SOUNDS` (the reply to opcode 31) and `GUILD_SOUNDBOARD_SOUNDS_UPDATE`.
pub fn guild_sounds(bytes: &[u8]) -> Result<(Id, Vec<Sound>), crate::DecodeError> {
	#[derive(Deserialize)]
	struct Dto {
		guild_id: Id,
		soundboard_sounds: Vec<SoundDto>,
	}
	let dto: Dto = crate::decode(bytes)?;
	Ok((
		dto.guild_id,
		list(dto.soundboard_sounds, Some(dto.guild_id), MAX_GUILD_SOUNDS)?,
	))
}

/// `GUILD_SOUNDBOARD_SOUND_CREATE` / `_UPDATE`: one guild sound object.
pub fn guild_sound(bytes: &[u8]) -> Result<Sound, crate::DecodeError> {
	let sound = crate::decode::<SoundDto>(bytes)?.into_model()?;
	if sound.guild.is_none() {
		return Err(crate::DecodeError);
	}
	Ok(sound)
}

/// `GUILD_SOUNDBOARD_SOUND_DELETE`: `(guild, sound)`.
pub fn deleted_sound(bytes: &[u8]) -> Result<(Id, Id), crate::DecodeError> {
	#[derive(Deserialize)]
	struct Dto {
		guild_id: Id,
		sound_id: FlexibleId,
	}
	let dto: Dto = crate::decode(bytes)?;
	Ok((dto.guild_id, dto.sound_id.0))
}

/// `VOICE_CHANNEL_EFFECT_SEND`. Emoji reactions carry no `sound_id` and yield `None`.
pub fn effect(bytes: &[u8]) -> Result<Option<SoundEffect>, crate::DecodeError> {
	#[derive(Deserialize)]
	struct Dto {
		channel_id: Id,
		#[serde(default)]
		guild_id: Option<Id>,
		user_id: Id,
		#[serde(default)]
		sound_id: Option<FlexibleId>,
		#[serde(default)]
		sound_volume: Option<f64>,
	}
	let dto: Dto = crate::decode(bytes)?;
	Ok(dto.sound_id.map(|sound| SoundEffect {
		channel: dto.channel_id,
		guild: dto.guild_id,
		user: dto.user_id,
		sound: sound.0,
		volume: volume(dto.sound_volume.unwrap_or(1.0)),
	}))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Effect parsing: both ID forms, volume clamping, emoji-only reactions and malformed input.
	#[test]
	fn effects_accept_integer_default_ids_and_skip_emoji_reactions() {
		let played = effect(br#"{"channel_id":"10","guild_id":"20","user_id":"30","sound_id":3,"sound_volume":0.5,"emoji":{"name":"duck"}}"#)
			.unwrap()
			.unwrap();
		assert_eq!(
			(
				played.channel,
				played.guild,
				played.user,
				played.sound,
				played.volume
			),
			(Id(10), Some(Id(20)), Id(30), Id(3), 0.5)
		);
		let custom = effect(
			br#"{"channel_id":"10","user_id":"30","sound_id":"1234567890123","sound_volume":7}"#,
		)
		.unwrap()
		.unwrap();
		assert_eq!(
			(custom.sound, custom.volume, custom.guild),
			(Id(1234567890123), 1.0, None)
		);
		let reaction = br#"{"channel_id":"10","guild_id":"20","user_id":"30","emoji":{"name":"x"},"animation_type":1,"animation_id":4}"#;
		assert!(effect(reaction).unwrap().is_none());
		for malformed in [
			&br#"{"channel_id":"10","user_id":"30","sound_id":0}"#[..],
			br#"{"channel_id":"10","user_id":"30","sound_id":-4}"#,
			br#"{"channel_id":"10","user_id":"30","sound_id":"abc"}"#,
			br#"{"user_id":"30","sound_id":"1"}"#,
		] {
			assert!(effect(malformed).is_err());
		}
	}

	/// Catalog parsing binds sounds to their server and enforces name and count bounds.
	#[test]
	fn catalogs_bind_guild_provenance_and_bound_entries() {
		let defaults = default_sounds(br#"[{"name":"quack","sound_id":"1","volume":1.0,"emoji_id":null,"emoji_name":"duck","available":true},{"name":"quack","sound_id":"1","volume":1.0}]"#).unwrap();
		assert_eq!(defaults.len(), 1);
		assert_eq!(defaults[0].guild, None);
		let (guild, sounds) = guild_sounds(br#"{"guild_id":"5","soundboard_sounds":[{"name":" airhorn ","sound_id":"9","volume":0.25,"emoji_id":"77","emoji_name":null,"guild_id":"5","available":false,"user":{"id":"1"}}]}"#).unwrap();
		assert_eq!(guild, Id(5));
		assert_eq!(
			sounds[0],
			Sound {
				id: Id(9),
				name: "airhorn".into(),
				volume: 0.25,
				emoji_id: Some(Id(77)),
				emoji_name: None,
				guild: Some(Id(5)),
				available: false,
			}
		);
		// A sound claiming another guild, an empty name, or too many entries is rejected.
		assert!(guild_sounds(br#"{"guild_id":"5","soundboard_sounds":[{"name":"a","sound_id":"9","guild_id":"6"}]}"#).is_err());
		assert!(default_sounds(br#"[{"name":"  ","sound_id":"1"}]"#).is_err());
		let long = format!(
			r#"[{{"name":"{}","sound_id":"1"}}]"#,
			"a".repeat(MAX_SOUND_NAME_BYTES + 1)
		);
		assert!(default_sounds(long.as_bytes()).is_err());
		let many = format!(
			r#"{{"guild_id":"5","soundboard_sounds":[{}]}}"#,
			(1..=MAX_GUILD_SOUNDS + 1)
				.map(|id| format!(r#"{{"name":"s","sound_id":"{id}"}}"#))
				.collect::<Vec<_>>()
				.join(",")
		);
		assert!(guild_sounds(many.as_bytes()).is_err());
		assert!(guild_sound(br#"{"name":"a","sound_id":"9"}"#).is_err());
		assert_eq!(
			deleted_sound(br#"{"guild_id":"5","sound_id":"9"}"#).unwrap(),
			(Id(5), Id(9))
		);
	}
}
