use crate::{
	ChannelMetadataSnapshot, ConversationActivitySnapshot, ForumDataSnapshot,
	MemberDetailsSnapshot, MessageContentSnapshot,
};
use serde::{Deserialize, Serialize};

pub const MAX_APP_SNAPSHOT_BYTES: usize = 64 * 1024;
pub const MAX_APP_CHANNELS: usize = 100;
pub const MAX_APP_GUILDS: usize = 100;
pub const MAX_CHANNEL_RECIPIENTS: usize = 32;
pub const MAX_APP_MESSAGES: usize = 50;
pub const MAX_MESSAGE_DETAILS: usize = 20;
pub const MAX_MESSAGE_MENTIONS: usize = 32;
pub const MAX_MESSAGE_ATTACHMENTS: usize = 10;
pub const MAX_MESSAGE_REACTIONS: usize = 16;
pub const MAX_RELATIONSHIPS: usize = 100;
pub const MAX_APP_MEMBERS: usize = 100;
pub const MAX_APP_PRESENCES: usize = 100;
pub const MAX_VOICE_PARTICIPANTS: usize = 64;
pub const MAX_HOST_EFFECTS: usize = 1;
pub const MAX_HOST_EFFECT_BYTES: usize = 8 * 1024;

/// Each group is present only when granted and available; partial lists say so explicitly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSnapshot {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub message_content: Option<MessageContentSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub forum_data: Option<ForumDataSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub conversation_activity: Option<ConversationActivitySnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub channel_metadata: Option<ChannelMetadataSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub member_details: Option<MemberDetailsSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub message_details: Option<MessageDetailsSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub relationships: Option<RelationshipsSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub account_profile: Option<AccountProfileSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub guilds: Option<GuildDirectorySnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub channel_details: Option<ChannelDetailsSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub context: Option<AppContextSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub channels: Option<ChannelDirectorySnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub timeline: Option<TimelineSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub members: Option<MembersSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub presence: Option<PresenceSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub voice: Option<VoiceSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub read_state: Option<ReadSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub settings: Option<LocalSettingsSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub notification_settings: Option<NotificationSettingsSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDetailsSnapshot {
	pub channel_id: String,
	pub items: Vec<MessageDetailSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDetailSnapshot {
	pub id: String,
	pub kind: u8,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub reply_to: Option<String>,
	pub mention_ids: Vec<String>,
	pub mentions_truncated: bool,
	pub mention_everyone: bool,
	pub attachments: Vec<AttachmentSnapshot>,
	pub attachments_truncated: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub reactions: Option<Vec<ReactionSnapshot>>,
	pub reactions_truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentSnapshot {
	pub id: String,
	pub filename: String,
	pub size: u64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub content_type: Option<String>,
	pub spoiler: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub emoji_id: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub emoji_name: Option<String>,
	pub count: u32,
	pub me: bool,
	pub me_burst: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipsSnapshot {
	pub items: Vec<RelationshipSnapshot>,
	pub truncated: bool,
	pub friends_known: bool,
	pub requests_known: bool,
	pub restricted_known: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipSnapshot {
	pub user: UserSnapshot,
	pub kind: RelationshipKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationshipKind {
	Friend,
	IncomingRequest,
	OutgoingRequest,
	Blocked,
	Ignored,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProfileSnapshot {
	pub user: UserSnapshot,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub avatar: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub profile: Option<OwnProfileSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnProfileSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub display_name: Option<String>,
	pub bio: String,
	pub pronouns: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildSnapshot {
	pub id: String,
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuildDirectorySnapshot {
	pub items: Vec<GuildSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDetailsSnapshot {
	pub channel: ChannelSnapshot,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub parent_id: Option<String>,
	pub position: i32,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub last_message_id: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub message_count: Option<u32>,
	pub recipients: Vec<UserSnapshot>,
	pub recipients_truncated: bool,
	pub can_send: bool,
	pub can_read_history: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppContextSnapshot {
	pub connected: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user: Option<UserSnapshot>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel: Option<ChannelSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSnapshot {
	pub id: String,
	pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSnapshot {
	pub id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub guild_id: Option<String>,
	pub name: String,
	pub kind: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelDirectorySnapshot {
	pub items: Vec<ChannelSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineSnapshot {
	pub channel_id: String,
	pub messages: Vec<MessageSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSnapshot {
	pub id: String,
	pub author: UserSnapshot,
	pub content: String,
	pub attachment_count: u16,
	pub edited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MembersSnapshot {
	pub channel_id: String,
	pub items: Vec<UserSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceSnapshot {
	pub items: Vec<PresenceEntry>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceEntry {
	pub user_id: String,
	pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel_id: Option<String>,
	pub phase: String,
	pub muted: bool,
	pub deafened: bool,
	pub camera: bool,
	pub streaming: bool,
	pub participants: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel_id: Option<String>,
	pub unread: Option<bool>,
	pub mentions: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSettingsSnapshot {
	pub zoom_percent: u16,
	pub sidebar_width: u16,
	pub show_members: bool,
	pub animate_gifs: bool,
	pub hide_media_links: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub smooth_scrolling: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub scroll_speed_percent: Option<u16>,
}

/// Omitted preferences keep their current values when the user approves the proposal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalSettingsPatch {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub zoom_percent: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sidebar_width: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub show_members: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub animate_gifs: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub hide_media_links: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub smooth_scrolling: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub scroll_speed_percent: Option<u16>,
}

/// Device-local notification preferences, available only with an explicit grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettingsSnapshot {
	pub new_message: bool,
	pub current_channel: bool,
	pub incoming_ring: bool,
	pub outgoing_ring: bool,
	pub disable_sounds: bool,
	pub unread_badge: bool,
	pub mute: bool,
	pub unmute: bool,
	pub deafen: bool,
	pub undeafen: bool,
	pub camera_on: bool,
	pub screen_share_on: bool,
	pub user_join: bool,
	pub user_leave: bool,
	pub volume: u8,
}

/// Omitted preferences keep their current values when the user approves the proposal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotificationSettingsPatch {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub new_message: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub current_channel: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub incoming_ring: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub outgoing_ring: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub disable_sounds: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub unread_badge: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub mute: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub unmute: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub deafen: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub undeafen: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub camera_on: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub screen_share_on: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub user_join: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub user_leave: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub volume: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEventKind {
	Reactions,
	Pins,
	Typing,
	Polls,
	Threads,
	Roles,
	Permissions,
	Recovered,
	MessageDetails,
	Relationships,
	Account,
	Channels,
	Members,
	Presence,
	ReadState,
	Ready,
	Navigation,
	Connection,
	Context,
	Voice,
	Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppView {
	Friends,
	Search,
	Pins,
	Members,
	Threads,
	Settings,
	Account,
	ProfileSettings,
	Appearance,
	MessagingPermissions,
	Notifications,
	Activity,
	Keybinds,
	Storage,
	Updates,
	Extensions,
	Themes,
	VoiceSettings,
}

/// One local host proposal per response, applied only after explicit user confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostEffect {
	Navigate {
		channel_id: String,
	},
	Home,
	OpenView {
		view: AppView,
	},
	OpenProfile {
		user_id: String,
	},
	JumpToMessage {
		channel_id: String,
		message_id: String,
	},
	Search {
		query: String,
	},
	Notice {
		text: String,
	},
	CopyText {
		text: String,
	},
	SetVoice {
		muted: bool,
		deafened: bool,
	},
	LeaveVoice,
	SetLocalSettings {
		settings: LocalSettingsPatch,
	},
	SetNotificationSettings {
		settings: NotificationSettingsPatch,
	},
}

use crate::{Invocation, MessageEvent, Output};

/// Optional app data is capability-scoped; the original invocation types stay unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInvocation {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub host: Option<crate::HostInfo>,
	#[serde(flatten)]
	pub invocation: Invocation,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub message_event: Option<MessageEvent>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub app: Option<AppSnapshot>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub app_event: Option<AppEventKind>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppOutput {
	#[serde(flatten)]
	pub output: Output,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub effects: Vec<HostEffect>,
}
