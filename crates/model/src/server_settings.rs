//! One bounded, service-authoritative server settings snapshot.
use crate::{Id, Patch};

pub const MAX_SETTINGS_BYTES: usize = 32 * 1024;
pub const MAX_ICON_DATA_URI: usize = 22 + 4 * (256_usize * 1024).div_ceil(3);
pub const SYSTEM_MESSAGE_MASK: u64 = 0b1111;
pub const ACTIVITY_ENABLED: &str = "ACTIVITY_FEED_ENABLED_BY_USER";
pub const ACTIVITY_DISABLED: &str = "ACTIVITY_FEED_DISABLED_BY_USER";
/// Community servers must keep a verification level of at least low and scan every member.
pub const COMMUNITY: &str = "COMMUNITY";
/// Highest documented guild verification level ("very high": verified phone).
pub const MAX_VERIFICATION_LEVEL: u8 = 4;
/// Highest documented explicit content filter level ("all members").
pub const MAX_CONTENT_FILTER: u8 = 2;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Trait {
	pub label: String,
	pub emoji: Option<String>,
}
impl Trait {
	pub fn valid(&self) -> bool {
		!self.label.trim().is_empty()
			&& text(&self.label, 100, false)
			&& self
				.emoji
				.as_ref()
				.is_none_or(|emoji| !emoji.is_empty() && text(emoji, 32, false))
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
	pub guild: Id,
	pub name: String,
	pub icon: Option<String>,
	pub banner_color: Option<u32>,
	pub traits: Vec<Trait>,
	pub description: String,
	pub online_count: Option<u64>,
	pub member_count: Option<u64>,
	pub system_channel_id: Option<Id>,
	pub system_channel_flags: u64,
	pub activity_feed: Option<bool>,
	pub default_message_notifications: u8,
	pub afk_channel_id: Option<Id>,
	pub afk_timeout: u32,
	pub verification_level: u8,
	pub explicit_content_filter: u8,
	pub features: Vec<String>,
}
impl Default for Settings {
	fn default() -> Self {
		Self {
			guild: Id(0),
			name: String::new(),
			icon: None,
			banner_color: None,
			traits: vec![],
			description: String::new(),
			online_count: None,
			member_count: None,
			system_channel_id: None,
			system_channel_flags: 0,
			activity_feed: None,
			default_message_notifications: 0,
			afk_channel_id: None,
			afk_timeout: 300,
			verification_level: 0,
			explicit_content_filter: 0,
			features: vec![],
		}
	}
}
impl Settings {
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity()
			+ self.icon.as_ref().map_or(0, String::capacity)
			+ self.description.capacity()
			+ trait_bytes(&self.traits)
			+ self.features.capacity() * size_of::<String>()
			+ self.features.iter().map(String::capacity).sum::<usize>()
	}
	pub fn community(&self) -> bool {
		self.features.iter().any(|feature| feature == COMMUNITY)
	}
	pub fn valid(&self) -> bool {
		self.guild.0 != 0
			&& self.heap_bytes() <= MAX_SETTINGS_BYTES
			&& valid_name(&self.name)
			&& text(&self.description, 300, true)
			&& self
				.icon
				.as_ref()
				.is_none_or(|hash| crate::valid_avatar_hash(hash))
			&& self.banner_color.is_none_or(|color| color <= 0xff_ffff)
			&& self.traits.len() <= 5
			&& self.traits.iter().all(Trait::valid)
			&& self.default_message_notifications <= 1
			&& valid_timeout(self.afk_timeout)
			&& self.verification_level <= MAX_VERIFICATION_LEVEL
			&& self.explicit_content_filter <= MAX_CONTENT_FILTER
			&& self.system_channel_id.is_none_or(|id| id.0 != 0)
			&& self.afk_channel_id.is_none_or(|id| id.0 != 0)
			&& self.features.len() <= 256
			&& self.features.iter().all(|feature| {
				!feature.is_empty()
					&& feature.len() <= 128
					&& feature
						.bytes()
						.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
			})
	}
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
	pub name: Option<String>,
	pub icon: Patch<String>,
	pub banner_color: Option<u32>,
	pub traits: Option<Vec<Trait>>,
	pub description: Option<String>,
	pub system_channel_id: Patch<Id>,
	pub system_channel_flags: Option<u64>,
	pub activity_feed: Option<bool>,
	pub default_message_notifications: Option<u8>,
	pub afk_channel_id: Patch<Id>,
	pub afk_timeout: Option<u32>,
	pub verification_level: Option<u8>,
	pub explicit_content_filter: Option<u8>,
}
impl Edit {
	pub fn between(before: &Settings, after: &Settings) -> Self {
		Self {
			name: (before.name != after.name).then(|| after.name.clone()),
			icon: if before.icon.is_some() && after.icon.is_none() {
				Patch::Null
			} else {
				Patch::Absent
			},
			banner_color: (before.banner_color != after.banner_color)
				.then_some(after.banner_color)
				.flatten(),
			traits: (before.traits != after.traits).then(|| after.traits.clone()),
			description: (before.description != after.description)
				.then(|| after.description.clone()),
			system_channel_id: changed_id(before.system_channel_id, after.system_channel_id),
			system_channel_flags: (before.system_channel_flags != after.system_channel_flags)
				.then_some(after.system_channel_flags),
			activity_feed: (before.activity_feed != after.activity_feed)
				.then_some(after.activity_feed)
				.flatten(),
			default_message_notifications: (before.default_message_notifications
				!= after.default_message_notifications)
				.then_some(after.default_message_notifications),
			afk_channel_id: changed_id(before.afk_channel_id, after.afk_channel_id),
			afk_timeout: (before.afk_timeout != after.afk_timeout).then_some(after.afk_timeout),
			verification_level: (before.verification_level != after.verification_level)
				.then_some(after.verification_level),
			explicit_content_filter: (before.explicit_content_filter
				!= after.explicit_content_filter)
				.then_some(after.explicit_content_filter),
		}
	}
	pub fn is_empty(&self) -> bool {
		self == &Self::default()
	}
	pub fn valid(&self) -> bool {
		self.name
			.as_ref()
			.is_none_or(|name| name.capacity() <= 400 && valid_name(name))
			&& self
				.description
				.as_ref()
				.is_none_or(|value| value.capacity() <= 1200 && text(value, 300, true))
			&& self.banner_color.is_none_or(|value| value <= 0xff_ffff)
			&& self.traits.as_ref().is_none_or(|values| {
				values.len() <= 5 && trait_bytes(values) <= 4096 && values.iter().all(Trait::valid)
			}) && self
			.default_message_notifications
			.is_none_or(|value| value <= 1)
			&& self.afk_timeout.is_none_or(valid_timeout)
			&& self
				.verification_level
				.is_none_or(|value| value <= MAX_VERIFICATION_LEVEL)
			&& self
				.explicit_content_filter
				.is_none_or(|value| value <= MAX_CONTENT_FILTER)
			&& !matches!(self.system_channel_id, Patch::Value(Id(0)))
			&& !matches!(self.afk_channel_id, Patch::Value(Id(0)))
			&& match &self.icon {
				Patch::Value(uri) => {
					uri.capacity() <= MAX_ICON_DATA_URI
						&& uri
							.strip_prefix("data:image/png;base64,")
							.is_some_and(|data| {
								!data.is_empty()
									&& data
										.bytes()
										.all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
							})
				}
				_ => true,
			}
	}
	/// Apply metadata for synthetic previews only; icon upload data never becomes an icon hash.
	pub fn apply(&self, settings: &mut Settings) {
		if let Some(value) = &self.name {
			settings.name.clone_from(value);
		}
		if matches!(self.icon, Patch::Null) {
			settings.icon = None;
		}
		if let Some(value) = self.banner_color {
			settings.banner_color = Some(value);
		}
		if let Some(value) = &self.traits {
			settings.traits.clone_from(value);
		}
		if let Some(value) = &self.description {
			settings.description.clone_from(value);
		}
		apply_id(&self.system_channel_id, &mut settings.system_channel_id);
		if let Some(value) = self.system_channel_flags {
			settings.system_channel_flags = (settings.system_channel_flags & !SYSTEM_MESSAGE_MASK)
				| (value & SYSTEM_MESSAGE_MASK);
		}
		if let Some(value) = self.activity_feed {
			settings.activity_feed = Some(value);
			settings
				.features
				.retain(|feature| feature != ACTIVITY_ENABLED && feature != ACTIVITY_DISABLED);
			settings.features.push(
				if value {
					ACTIVITY_ENABLED
				} else {
					ACTIVITY_DISABLED
				}
				.into(),
			);
		}
		if let Some(value) = self.default_message_notifications {
			settings.default_message_notifications = value;
		}
		apply_id(&self.afk_channel_id, &mut settings.afk_channel_id);
		if let Some(value) = self.afk_timeout {
			settings.afk_timeout = value;
		}
		if let Some(value) = self.verification_level {
			settings.verification_level = value;
		}
		if let Some(value) = self.explicit_content_filter {
			settings.explicit_content_filter = value;
		}
	}
	/// Community servers reject a verification level of none and partial content scanning.
	pub fn keeps_community_requirements(&self, latest: &Settings) -> bool {
		!latest.community()
			|| self.verification_level.is_none_or(|value| value >= 1)
				&& self
					.explicit_content_filter
					.is_none_or(|value| value == MAX_CONTENT_FILTER)
	}
}
fn changed_id(before: Option<Id>, after: Option<Id>) -> Patch<Id> {
	if before == after {
		Patch::Absent
	} else {
		after.map_or(Patch::Null, Patch::Value)
	}
}
fn apply_id(patch: &Patch<Id>, value: &mut Option<Id>) {
	match patch {
		Patch::Value(id) => *value = Some(*id),
		Patch::Null => *value = None,
		Patch::Absent => {}
	}
}
fn trait_bytes(traits: &Vec<Trait>) -> usize {
	traits.capacity() * size_of::<Trait>()
		+ traits
			.iter()
			.map(|value| value.label.capacity() + value.emoji.as_ref().map_or(0, String::capacity))
			.sum::<usize>()
}
fn text(value: &str, max: usize, multiline: bool) -> bool {
	value.len() <= max * 4
		&& value.chars().count() <= max
		&& value
			.chars()
			.all(|c| !c.is_control() || (multiline && matches!(c, '\n' | '\r' | '\t')))
}
fn valid_name(value: &str) -> bool {
	value.trim().chars().count() >= 2 && text(value, 100, false)
}
fn valid_timeout(value: u32) -> bool {
	matches!(value, 60 | 300 | 900 | 1800 | 3600)
}
