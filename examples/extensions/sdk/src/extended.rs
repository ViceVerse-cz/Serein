use crate::{ChannelSnapshot, UserSnapshot};
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
