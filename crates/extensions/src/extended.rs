use crate::{ChannelSnapshot, Error, UserSnapshot, app};
use serde::{Deserialize, Serialize};

pub const MAX_QUERY_ITEMS: usize = 25;
pub const MAX_QUERY_SNAPSHOT_BYTES: usize = 48 * 1024;
pub const MAX_MESSAGING_SETTINGS_IDS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveQueryKind {
	Public,
	Private,
	JoinedPrivate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuerySnapshot {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub messages: Option<MessageQuerySnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub archives: Option<ArchiveQuerySnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub members: Option<MemberQuerySnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub profile: Option<ProfileQuerySnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub gifs: Option<GifQuerySnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageQuerySnapshot {
	pub channel_id: String,
	pub pins: bool,
	pub query: String,
	pub loading: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
	pub total: u64,
	pub partial: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next: Option<String>,
	pub items: Vec<MessageQueryItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageQueryItem {
	pub id: String,
	pub author: UserSnapshot,
	pub excerpt: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveQuerySnapshot {
	pub parent_id: String,
	pub kind: ArchiveQueryKind,
	pub loading: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
	pub items: Vec<ChannelSnapshot>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub next: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberQuerySnapshot {
	pub channel_id: String,
	pub query: String,
	pub loading: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
	pub items: Vec<MemberQueryItem>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberQueryItem {
	pub user: UserSnapshot,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub nickname: Option<String>,
	pub role_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileQuerySnapshot {
	pub user_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub guild_id: Option<String>,
	pub loading: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub data: Option<ProfileQueryData>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileQueryData {
	pub user: UserSnapshot,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub display_name: Option<String>,
	pub bio: String,
	pub pronouns: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub nickname: Option<String>,
	pub role_ids: Vec<String>,
	pub limited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GifQuerySnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub query: Option<String>,
	pub loading: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub error: Option<String>,
	pub items: Vec<GifQueryItem>,
	pub categories: Vec<String>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GifQueryItem {
	pub id: String,
	pub title: String,
	pub url: String,
	pub preview: String,
	pub width: u32,
	pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagingSettingsSnapshot {
	pub spam_filter: u8,
	pub default_allow_dms: bool,
	pub restricted_guild_ids: Vec<String>,
	pub default_filter_requests: bool,
	pub unfiltered_guild_ids: Vec<String>,
	pub friend_source_flags: u32,
	pub personalized_requests: bool,
	pub game_friend_dms: bool,
	pub game_dms: u8,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuildFoldersSnapshot {
	pub folders: Vec<GuildFolderInput>,
	pub version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuildFolderInput {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub id: Option<u64>,
	pub guild_ids: Vec<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub color: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessagingSettingsChange {
	SpamFilter { level: u8 },
	DefaultAllowDms { enabled: bool },
	AllowGuildDms { guild_id: String, enabled: bool },
	DefaultFilterRequests { enabled: bool },
	FilterGuildRequests { guild_id: String, enabled: bool },
	Everyone { enabled: bool },
	FriendsOfFriends { enabled: bool },
	ServerMembers { enabled: bool },
	PersonalizedRequests { enabled: bool },
	GameFriendDms { enabled: bool },
	GameDms { level: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerAdminPage {
	Emoji,
	Members,
	Roles,
	Invites,
	AuditLog,
}

fn text(value: &str, limit: usize) -> Result<(), Error> {
	app::profile_text(value, limit, true)
}

fn error(value: &Option<String>) -> Result<(), Error> {
	if let Some(value) = value {
		text(value, 256)?;
	}
	Ok(())
}

impl QuerySnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		app::bounded_bytes(self, MAX_QUERY_SNAPSHOT_BYTES)?;
		if let Some(value) = &self.messages {
			app::entity_id(&value.channel_id)?;
			text(&value.query, 1024)?;
			if let Some(next) = &value.next {
				text(next, 64)?;
			}
			error(&value.error)?;
			if value.items.len() > MAX_QUERY_ITEMS {
				return Err(Error::Limit);
			}
			for item in &value.items {
				app::entity_id(&item.id)?;
				app::user(&item.author)?;
				text(&item.excerpt, 4096)?;
			}
		}
		if let Some(value) = &self.archives {
			app::entity_id(&value.parent_id)?;
			error(&value.error)?;
			if let Some(next) = &value.next {
				text(next, 64)?;
			}
			if value.items.len() > MAX_QUERY_ITEMS {
				return Err(Error::Limit);
			}
			for item in &value.items {
				app::channel(item)?;
			}
		}
		if let Some(value) = &self.members {
			app::entity_id(&value.channel_id)?;
			text(&value.query, 256)?;
			error(&value.error)?;
			if value.items.len() > MAX_QUERY_ITEMS {
				return Err(Error::Limit);
			}
			for item in &value.items {
				app::user(&item.user)?;
				if let Some(nick) = &item.nickname {
					text(nick, 256)?;
				}
				app::ids(item.role_ids.iter().map(String::as_str), 32)?;
			}
		}
		if let Some(value) = &self.profile {
			app::entity_id(&value.user_id)?;
			if let Some(id) = &value.guild_id {
				app::entity_id(id)?;
			}
			error(&value.error)?;
			if let Some(data) = &value.data {
				app::user(&data.user)?;
				if let Some(name) = &data.display_name {
					text(name, 256)?;
				}
				text(&data.bio, 4096)?;
				text(&data.pronouns, 256)?;
				if let Some(nick) = &data.nickname {
					text(nick, 256)?;
				}
				app::ids(data.role_ids.iter().map(String::as_str), 32)?;
			}
		}
		if let Some(value) = &self.gifs {
			if let Some(query) = &value.query {
				text(query, 1024)?;
			}
			error(&value.error)?;
			if value.items.len() > MAX_QUERY_ITEMS || value.categories.len() > MAX_QUERY_ITEMS {
				return Err(Error::Limit);
			}
			for item in &value.items {
				text(&item.id, 64)?;
				text(&item.title, 256)?;
				for url in [&item.url, &item.preview] {
					if url.len() > 512 || !url.starts_with("https://") || !url.is_ascii() {
						return Err(Error::Invalid);
					}
				}
				if !(1..=4096).contains(&item.width) || !(1..=4096).contains(&item.height) {
					return Err(Error::Invalid);
				}
			}
			for category in &value.categories {
				text(category, 64)?;
			}
		}
		Ok(())
	}
}

impl MessagingSettingsSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		if self.spam_filter > 3 || self.game_dms > 3 {
			return Err(Error::Invalid);
		}
		app::ids(
			self.restricted_guild_ids.iter().map(String::as_str),
			MAX_MESSAGING_SETTINGS_IDS,
		)?;
		app::ids(
			self.unfiltered_guild_ids.iter().map(String::as_str),
			MAX_MESSAGING_SETTINGS_IDS,
		)
	}
}

impl GuildFolderInput {
	pub fn validate(&self) -> Result<(), Error> {
		if self.id == Some(0) || self.color.is_some_and(|color| color > 0xffffff) {
			return Err(Error::Invalid);
		}
		app::ids(self.guild_ids.iter().map(String::as_str), 200)?;
		if self.id.is_none() && self.guild_ids.len() != 1 {
			return Err(Error::Invalid);
		}
		if let Some(name) = &self.name {
			text(name, 400)?;
		}
		Ok(())
	}
}

impl GuildFoldersSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		if self.folders.len() > 200 {
			return Err(Error::Limit);
		}
		for folder in &self.folders {
			folder.validate()?;
		}
		Ok(())
	}
}

impl MessagingSettingsChange {
	pub fn validate(&self) -> Result<(), Error> {
		match self {
			Self::SpamFilter { level } | Self::GameDms { level } if !(1..=3).contains(level) => {
				Err(Error::Invalid)
			}
			Self::AllowGuildDms { guild_id, .. } | Self::FilterGuildRequests { guild_id, .. } => {
				app::entity_id(guild_id)
			}
			_ => Ok(()),
		}
	}
}
