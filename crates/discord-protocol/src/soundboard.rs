//! Soundboard catalogs and voice channel effects.
//! References: https://docs.discord.com/developers/resources/soundboard and
//! https://docs.discord.com/developers/events/gateway-events#voice-channel-effect-send
use model::{Id, soundboard::Sound};
use serde::Deserialize;

/// Sound identifiers arrive as a snowflake string or an integer; zero means "no guild".
struct Loose(Option<Id>);
impl<'de> Deserialize<'de> for Loose {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct Visitor;
		impl serde::de::Visitor<'_> for Visitor {
			type Value = Loose;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("a Discord ID string or integer")
			}
			fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Loose, E> {
				if value.is_empty()
					|| value.len() > 20
					|| !value.bytes().all(|b| b.is_ascii_digit())
				{
					return Err(E::custom("Invalid Discord ID"));
				}
				value
					.parse::<u64>()
					.map(|id| Loose((id != 0).then_some(Id(id))))
					.map_err(E::custom)
			}
			fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Loose, E> {
				Ok(Loose((value != 0).then_some(Id(value))))
			}
			fn visit_unit<E: serde::de::Error>(self) -> Result<Loose, E> {
				Ok(Loose(None))
			}
		}
		d.deserialize_any(Visitor)
	}
}
fn available() -> bool {
	true
}
#[derive(Deserialize)]
struct SoundDto {
	sound_id: Loose,
	name: String,
	#[serde(default)]
	volume: Option<f64>,
	#[serde(default)]
	emoji_id: Option<Loose>,
	#[serde(default)]
	emoji_name: Option<String>,
	#[serde(default)]
	guild_id: Option<Loose>,
	#[serde(default = "available")]
	available: bool,
	/// Present only for members allowed to manage the server's expressions.
	#[serde(default)]
	user: Option<crate::UserDto>,
}
impl SoundDto {
	/// Bind the sound to its authoritative owner: `None` for the default set.
	fn checked(
		self,
		guild: Option<Id>,
	) -> Result<(Sound, Option<model::User>), crate::DecodeError> {
		let owner = self.guild_id.and_then(|id| id.0);
		if owner.is_some() && owner != guild {
			return Err(crate::DecodeError);
		}
		let volume = self.volume.unwrap_or(1.0);
		let sound = Sound {
			id: self.sound_id.0.ok_or(crate::DecodeError)?,
			name: self.name,
			volume: if volume.is_finite() {
				volume.clamp(0.0, 1.0) as f32
			} else {
				1.0
			},
			emoji_id: self.emoji_id.and_then(|id| id.0),
			emoji_name: self.emoji_name.filter(|name| !name.is_empty()),
			guild,
			available: self.available,
		};
		if !sound.valid() {
			return Err(crate::DecodeError);
		}
		Ok((sound, self.user.map(crate::UserDto::into_model)))
	}
}
fn catalog(items: Vec<SoundDto>, guild: Option<Id>) -> Result<Vec<Sound>, crate::DecodeError> {
	if items.len() > model::soundboard::MAX_SOUNDS {
		return Err(crate::DecodeError);
	}
	let sounds = items
		.into_iter()
		.map(|dto| dto.checked(guild).map(|(sound, _)| sound))
		.collect::<Result<Vec<_>, _>>()?;
	if !model::soundboard::valid_sounds(&sounds) {
		return Err(crate::DecodeError);
	}
	Ok(sounds)
}
/// `GET /soundboard-default-sounds`: a bare array.
pub fn default_sounds(bytes: &[u8]) -> Result<Vec<Sound>, crate::DecodeError> {
	catalog(crate::decode(bytes)?, None)
}
/// `GET /guilds/{guild.id}/soundboard-sounds`: `{"items": [...]}`.
pub fn guild_sounds(bytes: &[u8], guild: Id) -> Result<Vec<Sound>, crate::DecodeError> {
	#[derive(Deserialize)]
	struct Page {
		items: Vec<SoundDto>,
	}
	if guild.0 == 0 {
		return Err(crate::DecodeError);
	}
	catalog(crate::decode::<Page>(bytes)?.items, Some(guild))
}
/// The management view of a server's sounds, with uploaders where the service includes them.
pub fn admin_sounds(
	bytes: &[u8],
	guild: Id,
) -> Result<model::server_admin::Sounds, crate::DecodeError> {
	#[derive(Deserialize)]
	struct Page {
		items: Vec<SoundDto>,
	}
	let items = crate::decode::<Page>(bytes)?.items;
	if guild.0 == 0 || items.len() > model::soundboard::MAX_SOUNDS {
		return Err(crate::DecodeError);
	}
	let page = model::server_admin::Sounds {
		items: items
			.into_iter()
			.map(|dto| {
				dto.checked(Some(guild))
					.map(|(sound, uploader)| model::server_admin::Sound { sound, uploader })
			})
			.collect::<Result<_, _>>()?,
		limit: None,
	};
	if !model::server_admin::Result::Sounds(page.clone()).valid() {
		return Err(crate::DecodeError);
	}
	Ok(page)
}
/// One created or edited server sound.
pub fn admin_sound(bytes: &[u8], guild: Id) -> Result<Sound, crate::DecodeError> {
	if guild.0 == 0 {
		return Err(crate::DecodeError);
	}
	crate::decode::<SoundDto>(bytes)?
		.checked(Some(guild))
		.map(|(sound, _)| sound)
}
/// Slots by boost level, per Discord's published server perks; `MORE_SOUNDBOARD` raises it.
pub fn sound_limit(premium_tier: u8, features: &[String]) -> usize {
	if features.iter().any(|feature| feature == "MORE_SOUNDBOARD") {
		return 96;
	}
	match premium_tier {
		0 => 8,
		1 => 24,
		2 => 36,
		_ => 48,
	}
}
/// The `sound` field of Create Guild Soundboard Sound: a base64 data URI.
pub fn sound_data_uri(content_type: &str, bytes: &[u8]) -> String {
	use base64::{Engine as _, engine::general_purpose::STANDARD};
	format!("data:{content_type};base64,{}", STANDARD.encode(bytes))
}
/// A declared MP3 or Ogg upload within the service's size limit, checked by container magic.
pub fn valid_sound_file(content_type: &str, bytes: &[u8]) -> bool {
	!bytes.is_empty()
		&& bytes.len() <= model::server_admin::MAX_SOUND_FILE_BYTES
		&& match content_type {
			"audio/ogg" => bytes.starts_with(b"OggS"),
			"audio/mpeg" => {
				bytes.starts_with(b"ID3")
					|| (bytes.len() > 1 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
			}
			_ => false,
		}
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Effect {
	pub channel: Id,
	pub user: Id,
	pub sound: Id,
	pub volume: f32,
}
/// Voice Channel Effect Send. Emoji-only effects carry no sound and decode to `None`.
pub fn effect(bytes: &[u8]) -> Result<Option<Effect>, crate::DecodeError> {
	#[derive(Deserialize)]
	struct Dto {
		channel_id: Id,
		user_id: Id,
		#[serde(default)]
		sound_id: Option<Loose>,
		#[serde(default)]
		sound_volume: Option<f64>,
	}
	let dto: Dto = crate::decode(bytes)?;
	Ok(dto.sound_id.and_then(|id| id.0).map(|sound| Effect {
		channel: dto.channel_id,
		user: dto.user_id,
		sound,
		volume: match dto.sound_volume {
			Some(volume) if volume.is_finite() => volume.clamp(0.0, 1.0) as f32,
			_ => 1.0,
		},
	}))
}
/// Guild Soundboard Sound Create/Update/Delete and Sounds Update all name their guild.
pub fn changed_guild(bytes: &[u8]) -> Result<Id, crate::DecodeError> {
	#[derive(Deserialize)]
	struct Dto {
		guild_id: Id,
	}
	Ok(crate::decode::<Dto>(bytes)?.guild_id)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn catalogs_bind_owner_clamp_volume_and_reject_foreign_or_duplicate_sounds() {
		let defaults = default_sounds(
			br#"[{"name":"quack","sound_id":"1","volume":1.0,"emoji_id":null,"emoji_name":"duck","guild_id":"0","available":true},
			{"name":"airhorn","sound_id":2,"volume":7.5,"emoji_name":""}]"#,
		)
		.unwrap();
		assert_eq!(defaults.len(), 2);
		assert_eq!((defaults[0].id, defaults[0].guild), (Id(1), None));
		assert_eq!(defaults[0].emoji_name.as_deref(), Some("duck"));
		assert_eq!((defaults[1].id, defaults[1].volume), (Id(2), 1.0));
		assert!(defaults[1].available && defaults[1].emoji_name.is_none());

		let guild = guild_sounds(
			br#"{"items":[{"name":"Yay","sound_id":"30","volume":0.25,"emoji_id":"40","emoji_name":"yay","guild_id":"9","available":false,"user":{"id":"5","username":"Uploader"}}]}"#,
			Id(9),
		)
		.unwrap();
		assert_eq!(
			guild[0],
			Sound {
				id: Id(30),
				name: "Yay".into(),
				volume: 0.25,
				emoji_id: Some(Id(40)),
				emoji_name: Some("yay".into()),
				guild: Some(Id(9)),
				available: false,
			}
		);
		assert!(
			guild_sounds(
				br#"{"items":[{"name":"Yay","sound_id":"30","guild_id":"8"}]}"#,
				Id(9)
			)
			.is_err()
		);
		assert!(
			default_sounds(br#"[{"name":"a","sound_id":"1"},{"name":"b","sound_id":"1"}]"#)
				.is_err()
		);
		assert!(default_sounds(br#"[{"name":"","sound_id":"1"}]"#).is_err());
		assert!(default_sounds(br#"[{"name":"a","sound_id":"0"}]"#).is_err());
		let many = format!(
			"[{}]",
			(1..=model::soundboard::MAX_SOUNDS + 1)
				.map(|id| format!(r#"{{"name":"s","sound_id":"{id}"}}"#))
				.collect::<Vec<_>>()
				.join(",")
		);
		assert!(default_sounds(many.as_bytes()).is_err());
	}
	#[test]
	fn effects_keep_sounds_and_ignore_emoji_only_reactions() {
		assert_eq!(
			effect(br#"{"channel_id":"2","guild_id":"9","user_id":"3","sound_id":"30","sound_volume":0.5,"emoji":{"id":null,"name":"x"},"animation_type":1,"animation_id":4}"#)
				.unwrap(),
			Some(Effect {
				channel: Id(2),
				user: Id(3),
				sound: Id(30),
				volume: 0.5
			})
		);
		assert_eq!(
			effect(br#"{"channel_id":"2","user_id":"3","sound_id":4,"sound_volume":9}"#)
				.unwrap()
				.map(|effect| (effect.sound, effect.volume)),
			Some((Id(4), 1.0))
		);
		assert_eq!(
			effect(br#"{"channel_id":"2","guild_id":"9","user_id":"3","emoji":{"id":null,"name":"x"}}"#)
				.unwrap(),
			None
		);
		assert!(effect(br#"{"channel_id":"2","sound_id":"30"}"#).is_err());
		assert_eq!(
			changed_guild(br#"{"guild_id":"9","sound_id":"30"}"#).unwrap(),
			Id(9)
		);
	}
	#[test]
	fn management_rows_keep_uploaders_and_uploads_need_a_declared_container() {
		let page = admin_sounds(
			br#"{"items":[{"name":"Yay","sound_id":"30","volume":0.5,"emoji_name":"x","guild_id":"9","user":{"id":"5","username":"Uploader"}},
			{"name":"Boo","sound_id":"31","guild_id":"9"}]}"#,
			Id(9),
		)
		.unwrap();
		assert_eq!(page.items.len(), 2);
		assert_eq!(page.items[0].uploader.as_ref().unwrap().id, Id(5));
		assert!(page.items[1].uploader.is_none() && page.limit.is_none());
		assert!(
			admin_sounds(
				br#"{"items":[{"name":"Yay","sound_id":"30","guild_id":"8"}]}"#,
				Id(9)
			)
			.is_err()
		);
		assert!(
			admin_sounds(
				br#"{"items":[{"name":"a1","sound_id":"30"},{"name":"b1","sound_id":"30"}]}"#,
				Id(9)
			)
			.is_err()
		);
		assert_eq!(
			admin_sound(br#"{"name":"Yay","sound_id":"30","guild_id":"9"}"#, Id(9))
				.unwrap()
				.guild,
			Some(Id(9))
		);
		assert!(admin_sound(br#"{"name":"Yay","sound_id":"30","guild_id":"9"}"#, Id(0)).is_err());

		assert_eq!(
			[0, 1, 2, 3].map(|tier| sound_limit(tier, &[])),
			[8, 24, 36, 48]
		);
		assert_eq!(sound_limit(0, &["MORE_SOUNDBOARD".into()]), 96);
		assert_eq!(
			sound_data_uri("audio/ogg", b"OggS"),
			"data:audio/ogg;base64,T2dnUw=="
		);
		assert!(valid_sound_file("audio/ogg", b"OggS...."));
		assert!(valid_sound_file("audio/mpeg", b"ID3...."));
		assert!(valid_sound_file("audio/mpeg", &[0xff, 0xfb, 0x90]));
		assert!(!valid_sound_file("audio/mpeg", b"OggS...."));
		assert!(!valid_sound_file("audio/wav", b"RIFF...."));
		assert!(!valid_sound_file("audio/ogg", b""));
		assert!(!valid_sound_file(
			"audio/ogg",
			&[
				b"OggS".as_slice(),
				&vec![0; model::server_admin::MAX_SOUND_FILE_BYTES]
			]
			.concat()
		));
	}
}
