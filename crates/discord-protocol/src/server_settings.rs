//! Guild metadata uses the documented guild route; profile metadata is unofficial.
//! Observed profile schema: https://docs.discord.food/resources/discovery#guild-profile-object
use crate::DecodeError;
use model::{
	Id, Patch,
	server_settings::{ACTIVITY_DISABLED, ACTIVITY_ENABLED, Edit, Settings, Trait},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const MAX_SETTINGS_WIRE: usize = 1024 * 1024;

#[derive(Deserialize)]
struct Guild {
	id: Id,
	system_channel_id: Option<Id>,
	system_channel_flags: u64,
	features: Vec<String>,
	default_message_notifications: u8,
	afk_channel_id: Option<Id>,
	afk_timeout: u32,
	#[serde(default)]
	verification_level: u8,
	#[serde(default)]
	explicit_content_filter: u8,
}
#[derive(Deserialize)]
struct Profile {
	id: Id,
	name: String,
	icon_hash: Option<String>,
	description: Option<String>,
	brand_color_primary: Option<String>,
	traits: Vec<ProfileTrait>,
	online_count: Option<u64>,
	member_count: Option<u64>,
}
#[derive(Deserialize)]
struct ProfileTrait {
	label: String,
	emoji_name: Option<String>,
	emoji_id: Option<Id>,
	emoji_animated: bool,
	position: u8,
}
pub fn decode_settings(
	guild: Id,
	guild_bytes: &[u8],
	profile_bytes: &[u8],
) -> Result<Settings, DecodeError> {
	if guild_bytes.len() > MAX_SETTINGS_WIRE || profile_bytes.len() > MAX_SETTINGS_WIRE {
		return Err(DecodeError);
	}
	let metadata: Guild = crate::decode(guild_bytes)?;
	let mut profile: Profile = crate::decode(profile_bytes)?;
	if metadata.id != guild
		|| profile.id != guild
		|| profile.traits.len() > 5
		|| metadata.features.len() > 256
	{
		return Err(DecodeError);
	}
	profile.traits.sort_by_key(|value| value.position);
	if profile.traits.iter().enumerate().any(|(position, value)| {
		usize::from(value.position) != position || value.emoji_id.is_some() || value.emoji_animated
	}) {
		return Err(DecodeError);
	}
	let banner_color = profile
		.brand_color_primary
		.map(|color| {
			let hex = color.strip_prefix('#').ok_or(DecodeError)?;
			if hex.len() != 6 {
				return Err(DecodeError);
			}
			u32::from_str_radix(hex, 16).map_err(|_| DecodeError)
		})
		.transpose()?;
	let enabled = metadata
		.features
		.iter()
		.any(|value| value == ACTIVITY_ENABLED);
	let disabled = metadata
		.features
		.iter()
		.any(|value| value == ACTIVITY_DISABLED);
	if enabled && disabled {
		return Err(DecodeError);
	}
	let settings = Settings {
		guild,
		name: profile.name,
		icon: profile.icon_hash,
		banner_color,
		traits: profile
			.traits
			.into_iter()
			.map(|value| Trait {
				label: value.label,
				emoji: value.emoji_name,
			})
			.collect(),
		description: profile.description.unwrap_or_default(),
		online_count: profile.online_count,
		member_count: profile.member_count,
		system_channel_id: metadata.system_channel_id,
		system_channel_flags: metadata.system_channel_flags,
		activity_feed: if enabled {
			Some(true)
		} else if disabled {
			Some(false)
		} else {
			None
		},
		default_message_notifications: metadata.default_message_notifications,
		afk_channel_id: metadata.afk_channel_id,
		afk_timeout: metadata.afk_timeout,
		verification_level: metadata.verification_level,
		explicit_content_filter: metadata.explicit_content_filter,
		features: metadata.features,
	};
	if !settings.valid() {
		return Err(DecodeError);
	}
	Ok(settings)
}

/// Both bodies are partial. Unknown guild flags and features come from a fresh GET.
pub fn encode_edit(edit: &Edit, latest: &Settings) -> Result<(Value, Value), DecodeError> {
	if !edit.valid() || !latest.valid() || !edit.keeps_community_requirements(latest) {
		return Err(DecodeError);
	}
	let mut profile = Map::new();
	let mut guild = Map::new();
	if let Some(name) = &edit.name {
		profile.insert("name".into(), json!(name));
	}
	match &edit.icon {
		Patch::Null => {
			profile.insert("icon".into(), Value::Null);
		}
		Patch::Value(uri) => {
			if !crate::group_actions::valid_icon_data_uri(uri) {
				return Err(DecodeError);
			}
			profile.insert("icon".into(), json!(uri));
		}
		Patch::Absent => {}
	}
	if let Some(color) = edit.banner_color {
		profile.insert("brand_color_primary".into(), json!(format!("#{color:06x}")));
	}
	if let Some(description) = &edit.description {
		profile.insert("description".into(), json!(description));
	}
	if let Some(traits) = &edit.traits {
		profile.insert("traits".into(), Value::Array(traits.iter().enumerate().map(|(position, value)| json!({"label":value.label,"emoji_name":value.emoji,"emoji_id":null,"emoji_animated":false,"position":position})).collect()));
	}
	insert_id(&mut guild, "system_channel_id", &edit.system_channel_id);
	insert_id(&mut guild, "afk_channel_id", &edit.afk_channel_id);
	if let Some(value) = edit.system_channel_flags {
		let mask = model::server_settings::SYSTEM_MESSAGE_MASK;
		guild.insert(
			"system_channel_flags".into(),
			json!((latest.system_channel_flags & !mask) | (value & mask)),
		);
	}
	if let Some(value) = edit.default_message_notifications {
		guild.insert("default_message_notifications".into(), json!(value));
	}
	if let Some(value) = edit.afk_timeout {
		guild.insert("afk_timeout".into(), json!(value));
	}
	if let Some(value) = edit.verification_level {
		guild.insert("verification_level".into(), json!(value));
	}
	if let Some(value) = edit.explicit_content_filter {
		guild.insert("explicit_content_filter".into(), json!(value));
	}
	if let Some(value) = edit.activity_feed {
		let mut features: Vec<&str> = latest
			.features
			.iter()
			.map(String::as_str)
			.filter(|value| *value != ACTIVITY_ENABLED && *value != ACTIVITY_DISABLED)
			.collect();
		features.push(if value {
			ACTIVITY_ENABLED
		} else {
			ACTIVITY_DISABLED
		});
		guild.insert("features".into(), json!(features));
	}
	Ok((Value::Object(profile), Value::Object(guild)))
}
fn insert_id(body: &mut Map<String, Value>, key: &str, patch: &Patch<Id>) {
	match patch {
		Patch::Value(id) => {
			body.insert(key.into(), json!(id));
		}
		Patch::Null => {
			body.insert(key.into(), Value::Null);
		}
		Patch::Absent => {}
	}
}
pub fn confirm_guild(bytes: &[u8], guild: Id) -> Result<(), DecodeError> {
	#[derive(Deserialize)]
	struct Identity {
		id: Id,
	}
	let response: Identity = crate::decode(bytes)?;
	if response.id != guild {
		return Err(DecodeError);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	const GUILD: &[u8] = br#"{"id":"10","system_channel_id":null,"system_channel_flags":0,"features":["COMMUNITY"],"default_message_notifications":1,"afk_channel_id":null,"afk_timeout":300,"verification_level":2,"explicit_content_filter":2}"#;
	const PROFILE: &[u8] = br#"{"id":"10","name":"Synthetic","icon_hash":null,"description":null,"brand_color_primary":null,"traits":[],"online_count":1,"member_count":2}"#;

	#[test]
	fn safety_levels_round_trip_and_keep_community_requirements() {
		let settings = decode_settings(Id(10), GUILD, PROFILE).unwrap();
		assert_eq!(settings.verification_level, 2);
		assert_eq!(settings.explicit_content_filter, 2);
		assert!(settings.community());

		let mut draft = settings.clone();
		draft.verification_level = 4;
		let (profile, guild) = encode_edit(&Edit::between(&settings, &draft), &settings).unwrap();
		assert_eq!(profile, json!({}));
		assert_eq!(guild, json!({"verification_level": 4}));

		draft.verification_level = 0;
		assert!(encode_edit(&Edit::between(&settings, &draft), &settings).is_err());
		draft.verification_level = 1;
		draft.explicit_content_filter = 1;
		assert!(encode_edit(&Edit::between(&settings, &draft), &settings).is_err());

		let mut plain = settings.clone();
		plain.features.clear();
		let (_, guild) = encode_edit(&Edit::between(&plain, &draft), &plain).unwrap();
		assert_eq!(
			guild,
			json!({"verification_level": 1, "explicit_content_filter": 1})
		);
		draft.verification_level = 5;
		assert!(!Edit::between(&plain, &draft).valid());
	}
}
