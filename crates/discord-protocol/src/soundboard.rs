//! Documented soundboard metadata; user/creator fields are ignored without retention.
use crate::{DecodeError, permissions::List};
use model::{
	Id,
	soundboard::{MAX_BYTES, MAX_SOUNDS, Sound, valid_catalog},
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Wire {
	sound_id: Id,
	name: String,
	volume: f64,
	emoji_id: Option<Id>,
	emoji_name: Option<String>,
	guild_id: Option<Id>,
	available: bool,
}
impl Wire {
	fn into_model(self) -> Sound {
		Sound {
			id: self.sound_id,
			name: self.name,
			volume: self.volume,
			emoji: self.emoji_name,
			emoji_id: self.emoji_id,
			guild: self.guild_id,
			available: self.available,
		}
	}
}
pub fn sounds(bytes: &[u8], guild: Option<Id>) -> Result<Vec<Sound>, DecodeError> {
	#[derive(Deserialize)]
	struct Guild {
		items: List<Wire, MAX_SOUNDS>,
	}
	if bytes.len() > MAX_BYTES || guild.is_some_and(|id| id.0 == 0) {
		return Err(DecodeError);
	}
	let values: List<Wire, MAX_SOUNDS> = if guild.is_some() {
		crate::decode::<Guild>(bytes)?.items
	} else {
		crate::decode(bytes)?
	};
	let mut sounds = Vec::with_capacity(values.0.len());
	for mut sound in values.0 {
		if sound.guild_id.is_some_and(|id| Some(id) != guild) {
			return Err(DecodeError);
		}
		// guild_id is optional on the documented object; the fixed guild route supplies scope.
		sound.guild_id = guild;
		sounds.push(sound.into_model());
	}
	if !valid_catalog(&sounds, guild.unwrap_or(Id(1)))
		|| sounds.iter().any(|sound| sound.guild != guild)
	{
		return Err(DecodeError);
	}
	Ok(sounds)
}

#[cfg(test)]
mod tests {
	use super::*;
	const DEFAULT: &str = r#"[{"sound_id":"1","name":"quack","volume":1.0,"emoji_name":"🦆","emoji_id":null,"available":true}]"#;
	#[test]
	fn catalogs_have_distinct_shapes_and_reject_invalid_metadata() {
		let defaults = sounds(DEFAULT.as_bytes(), None).unwrap();
		assert_eq!(defaults[0].name, "quack");
		assert_eq!(defaults[0].emoji.as_deref(), Some("🦆"));
		assert!(sounds(DEFAULT.as_bytes(), Some(Id(2))).is_err());
		assert_eq!(
			sounds(format!("{{\"items\":{DEFAULT}}}").as_bytes(), Some(Id(2))).unwrap()[0].guild,
			Some(Id(2))
		);
		let guild = format!(
			"{{\"items\":{}}}",
			DEFAULT.replace(
				"\"available\":true",
				"\"guild_id\":\"2\",\"available\":false"
			)
		);
		assert!(!sounds(guild.as_bytes(), Some(Id(2))).unwrap()[0].available);
		assert!(sounds(guild.as_bytes(), Some(Id(3))).is_err());
		for invalid in [
			DEFAULT.replace("\"1\"", "\"0\""),
			DEFAULT.replace("quack", &"a".repeat(33)),
			DEFAULT.replace("1.0", "1.1"),
			DEFAULT.replace("🦆", &"x".repeat(129)),
			DEFAULT.replace("null", "\"0\""),
			DEFAULT.replace("quack", "bad\\nname"),
		] {
			assert!(sounds(invalid.as_bytes(), None).is_err());
		}
		let sound = DEFAULT.trim_start_matches('[').trim_end_matches(']');
		assert!(sounds(format!("[{},{}]", sound, sound).as_bytes(), None).is_err());
		assert!(
			sounds(
				format!(
					"[{}]",
					std::iter::repeat_n(sound, MAX_SOUNDS + 1)
						.collect::<Vec<_>>()
						.join(",")
				)
				.as_bytes(),
				None
			)
			.is_err()
		);
		assert!(sounds(&vec![b' '; MAX_BYTES + 1], None).is_err());
	}
}
