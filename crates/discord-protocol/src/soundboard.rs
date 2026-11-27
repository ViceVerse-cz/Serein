//! Bounded wire decoding for soundboard catalogs and voice channel effects.
//!
//! Protocol status: documented. `GET /soundboard-default-sounds`, the guild catalog routes and
//! `VOICE_CHANNEL_EFFECT_SEND` appear in Discord's published API documentation. The gateway
//! `REQUEST_SOUNDBOARD_SOUNDS` opcode and the `SOUNDBOARD_SOUNDS` dispatch are documented for
//! applications; their use from a user session is unverified.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use model::{Id, SoundEffect, SoundboardSound};
use serde::Deserialize;

/// The documented create route takes the audio file as a base64 data URI.
pub fn sound_data_uri(content_type: &str, audio: &[u8]) -> String {
	format!("data:{content_type};base64,{}", STANDARD.encode(audio))
}

pub type GuildCatalog = model::SoundboardList<{ model::MAX_GUILD_SOUNDS }>;
pub type DefaultCatalog = model::SoundboardList<{ model::MAX_DEFAULT_SOUNDS }>;

/// Binds a decoded catalog to the guild that actually contains it.
///
/// The wire `guild_id` is provenance the server may omit; the containing guild is authoritative,
/// so a sound that claims a different guild is rejected rather than silently reassigned.
pub fn guild_catalog(
	mut sounds: Vec<SoundboardSound>,
	guild: Id,
) -> Result<Vec<SoundboardSound>, crate::DecodeError> {
	if guild.0 == 0 || !model::valid_soundboard_sounds(&sounds, model::MAX_GUILD_SOUNDS) {
		return Err(crate::DecodeError);
	}
	for sound in &mut sounds {
		if sound.guild_id.is_some_and(|id| id != guild) {
			return Err(crate::DecodeError);
		}
		sound.guild_id = Some(guild);
	}
	Ok(sounds)
}

/// Default sounds are global; one that claims a guild is not a default sound.
pub fn default_catalog(
	sounds: Vec<SoundboardSound>,
) -> Result<Vec<SoundboardSound>, crate::DecodeError> {
	if !model::valid_soundboard_sounds(&sounds, model::MAX_DEFAULT_SOUNDS)
		|| sounds.iter().any(|sound| sound.guild_id.is_some())
	{
		return Err(crate::DecodeError);
	}
	Ok(sounds)
}

/// `GET /soundboard-default-sounds` responds with a bare array.
pub fn default_sounds(bytes: &[u8]) -> Result<Vec<SoundboardSound>, crate::DecodeError> {
	let list: DefaultCatalog = crate::decode(bytes)?;
	default_catalog(list.0)
}

/// `GET /guilds/{guild_id}/soundboard-sounds` wraps its array in `items`.
#[derive(Deserialize)]
pub struct GuildSoundsResponse {
	pub items: GuildCatalog,
}

pub fn guild_sounds(bytes: &[u8], guild: Id) -> Result<Vec<SoundboardSound>, crate::DecodeError> {
	let response: GuildSoundsResponse = crate::decode(bytes)?;
	guild_catalog(response.items.0, guild)
}

/// The `SOUNDBOARD_SOUNDS` dispatch answering a `REQUEST_SOUNDBOARD_SOUNDS` opcode.
#[derive(Deserialize)]
pub struct SoundboardSounds {
	pub guild_id: Id,
	pub soundboard_sounds: GuildCatalog,
}

/// `GUILD_SOUNDBOARD_SOUNDS_UPDATE` carries a whole replacement catalog.
#[derive(Deserialize)]
pub struct GuildSoundboardSoundsUpdate {
	pub guild_id: Id,
	pub soundboard_sounds: GuildCatalog,
}

/// `GUILD_SOUNDBOARD_SOUND_CREATE` and `..._UPDATE` carry a single sound inline.
pub fn guild_sound(bytes: &[u8], guild: Id) -> Result<SoundboardSound, crate::DecodeError> {
	let mut sound: SoundboardSound = crate::decode(bytes)?;
	if !sound.valid() || guild.0 == 0 || sound.guild_id.is_some_and(|id| id != guild) {
		return Err(crate::DecodeError);
	}
	sound.guild_id = Some(guild);
	Ok(sound)
}

/// `GUILD_SOUNDBOARD_SOUND_DELETE`.
#[derive(Deserialize)]
pub struct GuildSoundboardSoundDelete {
	pub sound_id: Id,
	pub guild_id: Id,
}

/// Identifies the guild of a soundboard dispatch without decoding its catalog, so an event for an
/// unknown guild can be dropped before the bounded list work happens.
#[derive(Deserialize)]
pub struct SoundboardGuild {
	pub guild_id: Id,
}

/// `VOICE_CHANNEL_EFFECT_SEND`: another member triggered an effect in a voice channel.
///
/// Playback is local. The dispatch names a sound, and each client fetches and mixes it itself;
/// no audio travels on the voice transport. Fields unrelated to sound playback (emoji reactions
/// and animation ids) are accepted and ignored so an emoji-only effect does not fail the decode.
#[derive(Deserialize)]
pub struct VoiceChannelEffect {
	pub channel_id: Id,
	pub user_id: Id,
	#[serde(default)]
	pub guild_id: Option<Id>,
	#[serde(default)]
	pub sound_id: Option<Id>,
	#[serde(default)]
	pub sound_volume: Option<f64>,
}

/// Decodes a voice channel effect, yielding `None` for effects that play no sound.
pub fn voice_channel_effect(bytes: &[u8]) -> Result<Option<SoundEffect>, crate::DecodeError> {
	// `Id` rejects a zero or malformed snowflake while decoding, so a decoded id is already usable.
	let effect: VoiceChannelEffect = crate::decode(bytes)?;
	// An emoji-only effect carries no sound; it is well formed and simply plays nothing.
	let Some(sound) = effect.sound_id else {
		return Ok(None);
	};
	let volume = effect
		.sound_volume
		.filter(|v| v.is_finite())
		.map_or(1.0, |v| v.clamp(0.0, 1.0) as f32);
	Ok(Some(SoundEffect {
		channel: effect.channel_id,
		user: effect.user_id,
		guild: effect.guild_id,
		sound,
		volume,
	}))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn guild_catalog_binds_provenance_and_rejects_a_foreign_guild() {
		let sound = |guild: &str| SoundboardSound {
			id: Id(5),
			name: "quack".to_owned(),
			volume: 1.0,
			emoji_id: None,
			emoji_name: None,
			guild_id: guild.parse().ok().map(Id),
			available: true,
			user_id: None,
		};

		// An omitted wire guild is filled in from the containing guild.
		let bound = guild_catalog(vec![sound("")], Id(7)).expect("an unlabelled sound binds");
		assert_eq!(bound[0].guild_id, Some(Id(7)));
		// A matching label is accepted.
		assert!(guild_catalog(vec![sound("7")], Id(7)).is_ok());
		// A sound claiming another guild must not be reassigned into this one.
		assert!(guild_catalog(vec![sound("9")], Id(7)).is_err());
		assert!(guild_catalog(vec![sound("7")], Id(0)).is_err());
	}

	#[test]
	fn catalog_decoding_bounds_count_identity_and_text() {
		let one =
			|id: u64, name: &str| format!(r#"{{"sound_id":"{id}","name":"{name}","volume":1.0}}"#);
		let parse = |body: String| default_sounds(body.as_bytes());

		let sounds = parse(format!("[{},{}]", one(1, "quack"), one(2, "honk")))
			.expect("a well-formed catalog decodes");
		assert_eq!(sounds.len(), 2);
		assert_eq!(sounds[1].name, "honk");

		// A duplicate id would let one sound shadow another in the catalog.
		assert!(parse(format!("[{},{}]", one(1, "quack"), one(1, "honk"))).is_err());
		// A zero id is unusable as a send target.
		assert!(parse(format!("[{}]", one(0, "quack"))).is_err());
		// Over the name bound.
		assert!(
			parse(format!(
				"[{}]",
				one(1, &"n".repeat(model::MAX_SOUND_NAME + 1))
			))
			.is_err()
		);
		// Over the list bound.
		let many = (1..=u64::try_from(model::MAX_DEFAULT_SOUNDS).expect("a small bound") + 1)
			.map(|i| one(i, "a"))
			.collect::<Vec<_>>()
			.join(",");
		assert!(parse(format!("[{many}]")).is_err());
	}

	#[test]
	fn odd_and_absent_volumes_are_clamped_rather_than_rejected() {
		let sounds = default_sounds(
			br#"[
				{"sound_id":"1","name":"quack","volume":1.0},
				{"sound_id":"2","name":"honk","volume":17.5},
				{"sound_id":"3","name":"boop","volume":-3.0},
				{"sound_id":"4","name":"beep"}
			]"#,
		)
		.expect("one odd gain must not discard the rest of the catalog");
		assert_eq!(sounds[1].volume, 1.0);
		assert_eq!(sounds[2].volume, 0.0);
		// An absent volume defaults to unity rather than silence.
		assert_eq!(sounds[3].volume, 1.0);
	}

	#[test]
	fn emoji_name_is_optional_bounded_and_never_empty() {
		let parse = |field: &str| {
			default_sounds(format!(r#"[{{"sound_id":"1","name":"quack",{field}}}]"#).as_bytes())
				.map(|mut sounds| sounds.remove(0).emoji_name)
		};
		assert_eq!(
			parse(r#""emoji_name":"wave""#).expect("a named emoji"),
			Some("wave".to_owned())
		);
		// Discord sends null for a sound with no emoji, and "" is not a usable emoji.
		assert_eq!(parse(r#""emoji_name":null"#).expect("a null emoji"), None);
		assert_eq!(parse(r#""emoji_name":"""#).expect("an empty emoji"), None);
		assert!(
			parse(&format!(
				r#""emoji_name":"{}""#,
				"e".repeat(model::MAX_SOUND_EMOJI_NAME + 1)
			))
			.is_err()
		);
	}

	#[test]
	fn guild_sounds_decodes_the_items_envelope() {
		let body = br#"{"items":[{"sound_id":"3","name":"honk","volume":0.5}]}"#;
		let sounds = guild_sounds(body, Id(7)).expect("the envelope decodes");
		assert_eq!(sounds.len(), 1);
		assert_eq!(sounds[0].volume, 0.5);
		assert_eq!(sounds[0].guild_id, Some(Id(7)));
		// A bare array is the default-sounds shape, not this route's.
		assert!(guild_sounds(br#"[{"sound_id":"3","name":"honk"}]"#, Id(7)).is_err());
	}

	#[test]
	fn default_sounds_reject_a_guild_labelled_entry() {
		let sounds = default_sounds(br#"[{"sound_id":"1","name":"quack"}]"#)
			.expect("a bare default array decodes");
		assert!(sounds[0].is_default());
		assert!(default_sounds(br#"[{"sound_id":"1","name":"q","guild_id":"7"}]"#).is_err());
	}

	#[test]
	fn voice_channel_effect_separates_sound_from_emoji_only_effects() {
		let effect = voice_channel_effect(
			br#"{"channel_id":"2","user_id":"3","guild_id":"7","sound_id":"9","sound_volume":0.25}"#,
		)
		.expect("a sound effect decodes")
		.expect("it carries a sound");
		assert_eq!(effect.sound, Id(9));
		assert_eq!(effect.volume, 0.25);

		// An emoji reaction in a voice channel is well formed and plays nothing.
		assert_eq!(
			voice_channel_effect(br#"{"channel_id":"2","user_id":"3","emoji":{"name":"wave"}}"#)
				.expect("an emoji effect decodes"),
			None
		);
		// A zero sound id is not a usable fetch target, and `Id` rejects it while decoding.
		assert!(
			voice_channel_effect(br#"{"channel_id":"2","user_id":"3","sound_id":"0"}"#).is_err()
		);
		// An out-of-range gain is clamped rather than dropping the playback.
		assert_eq!(
			voice_channel_effect(
				br#"{"channel_id":"2","user_id":"3","sound_id":"9","sound_volume":44.0}"#
			)
			.expect("a loud effect decodes")
			.expect("it carries a sound")
			.volume,
			1.0
		);
		// A missing channel or user leaves the effect unattributable.
		// A missing or zero channel leaves the effect unattributable.
		assert!(voice_channel_effect(br#"{"user_id":"3","sound_id":"9"}"#).is_err());
	}
}
