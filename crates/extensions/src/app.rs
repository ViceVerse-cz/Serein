use crate::{
	ArchiveQueryKind, ChannelMetadataSnapshot, ConversationActivitySnapshot, ForumDataSnapshot,
	GuildFolderInput, MemberDetailsSnapshot, MessageContentSnapshot, MessagingSettingsChange,
	ServerAdminPage,
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
#[serde(default, deny_unknown_fields)]
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
	#[serde(skip_serializing_if = "Option::is_none")]
	pub audio_settings: Option<AudioSettingsSnapshot>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub own_presence: Option<OwnPresenceSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageDetailsSnapshot {
	pub channel_id: String,
	pub items: Vec<MessageDetailSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct AttachmentSnapshot {
	pub id: String,
	pub filename: String,
	pub size: u64,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub content_type: Option<String>,
	pub spoiler: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct RelationshipsSnapshot {
	pub items: Vec<RelationshipSnapshot>,
	pub truncated: bool,
	pub friends_known: bool,
	pub requests_known: bool,
	pub restricted_known: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct AccountProfileSnapshot {
	pub user: UserSnapshot,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub avatar: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub profile: Option<OwnProfileSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnProfileSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub display_name: Option<String>,
	pub bio: String,
	pub pronouns: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuildSnapshot {
	pub id: String,
	pub name: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuildDirectorySnapshot {
	pub items: Vec<GuildSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct AppContextSnapshot {
	pub connected: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub user: Option<UserSnapshot>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel: Option<ChannelSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserSnapshot {
	pub id: String,
	pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelSnapshot {
	pub id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub guild_id: Option<String>,
	pub name: String,
	pub kind: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelDirectorySnapshot {
	pub items: Vec<ChannelSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineSnapshot {
	pub channel_id: String,
	pub messages: Vec<MessageSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSnapshot {
	pub id: String,
	pub author: UserSnapshot,
	pub content: String,
	pub attachment_count: u16,
	pub edited: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MembersSnapshot {
	pub channel_id: String,
	pub items: Vec<UserSnapshot>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceSnapshot {
	pub items: Vec<PresenceEntry>,
	pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceEntry {
	pub user_id: String,
	pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ReadSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub channel_id: Option<String>,
	pub unread: Option<bool>,
	pub mentions: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
pub enum ActionResultStatus {
	Accepted,
	Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionResultCode {
	Accepted,
	ContextChanged,
	Unavailable,
	Invalid,
	Denied,
	Failed,
}

/// Bounded result of applying a tracked proposal. This reports host acceptance, not network completion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionResult {
	pub request_id: String,
	pub status: ActionResultStatus,
	pub code: ActionResultCode,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioSettingsSnapshot {
	pub input_percent: u16,
	pub output_percent: u16,
	pub push_to_talk: bool,
	pub input_profile: String,
	pub suppression: String,
	pub suppression_level: u8,
	pub echo_cancellation: bool,
	pub automatic_gain: bool,
	pub sensitivity_db: Option<i16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnPresenceSnapshot {
	pub status: String,
	pub custom_status: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub expires_at_ms: Option<u64>,
	pub share_game_activity: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OwnProfilePatch {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub global_name: Option<String>,
	pub clear_global_name: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub bio: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub pronouns: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub accent_color: Option<u32>,
	pub clear_accent_color: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OwnPresencePatch {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub status: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub custom_status: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub clear_after_seconds: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioSettingsPatch {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_percent: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub output_percent: Option<u16>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub push_to_talk: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub input_profile: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub suppression: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub suppression_level: Option<u8>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub echo_cancellation: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub automatic_gain: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub sensitivity_db: Option<i16>,
	pub open_microphone: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionOverwriteInput {
	pub id: String,
	pub kind: u8,
	pub allow: String,
	pub deny: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelEditInput {
	pub name: String,
	pub topic: String,
	pub slowmode: u32,
	pub nsfw: bool,
	pub overwrites: Vec<PermissionOverwriteInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelPositionInput {
	pub channel_id: String,
	pub position: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerTraitInput {
	pub label: String,
	pub emoji: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerSettingsPatch {
	pub name: Option<String>,
	pub banner_color: Option<u32>,
	pub traits: Option<Vec<ServerTraitInput>>,
	pub description: Option<String>,
	pub system_channel_id: Option<String>,
	pub clear_system_channel: bool,
	pub system_channel_flags: Option<u64>,
	pub activity_feed: Option<bool>,
	pub default_message_notifications: Option<u8>,
	pub afk_channel_id: Option<String>,
	pub clear_afk_channel: bool,
	pub afk_timeout: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RolePatch {
	pub name: Option<String>,
	pub primary_color: Option<u32>,
	pub secondary_color: Option<u32>,
	pub tertiary_color: Option<u32>,
	pub permissions: Option<String>,
	pub permission_mask: Option<String>,
	pub hoist: Option<bool>,
	pub mentionable: Option<bool>,
	pub unicode_emoji: Option<String>,
	pub clear_unicode_emoji: bool,
}

/// App operation proposed by a foreground action; the host revalidates it at Apply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AppAction {
	OpenFriendDm {
		user_id: String,
	},
	SetFriendNickname {
		user_id: String,
		text: String,
	},
	SetUserNote {
		user_id: String,
		text: String,
	},
	AddFriend {
		username: String,
	},
	RemoveFriend {
		user_id: String,
	},
	ResolveFriendRequest {
		user_id: String,
		accept: bool,
	},
	SetUserBlocked {
		user_id: String,
		blocked: bool,
	},
	SetOwnProfile {
		profile: OwnProfilePatch,
	},
	SetOwnPresence {
		presence: OwnPresencePatch,
	},
	SetActivitySharing {
		enabled: bool,
	},
	SetAudioSettings {
		settings: AudioSettingsPatch,
	},
	SetParticipantAudio {
		user_id: String,
		volume_percent: Option<u16>,
		muted: Option<bool>,
	},
	SetStreamAudio {
		volume_percent: Option<u16>,
		muted: Option<bool>,
	},
	WatchStream {
		user_id: String,
	},
	StopWatching,
	DeclineCall {
		channel_id: String,
	},
	JoinVoice {
		channel_id: String,
		ring: bool,
		muted: bool,
		deafened: bool,
	},
	SetCamera {
		enabled: bool,
	},
	OpenAttachmentPicker {
		channel_id: String,
	},
	SelectAudioDevices {
		input_id: Option<String>,
		output_id: Option<String>,
	},
	RefreshMediaDevices,
	SelectCameraDevice {
		device_id: Option<String>,
	},
	OpenScreenSharePicker,
	StopScreenShare,
	RequestMessageSearch {
		query: String,
		before_id: Option<String>,
	},
	RequestPins {
		before: Option<String>,
	},
	RequestArchives {
		parent_id: String,
		kind: ArchiveQueryKind,
		before: Option<String>,
	},
	RequestMemberSearch {
		channel_id: String,
		query: String,
	},
	RequestProfile {
		user_id: String,
		guild_id: Option<String>,
	},
	RequestGifs {
		query: Option<String>,
	},
	SetMessagingSettings {
		change: MessagingSettingsChange,
	},
	SetGuildFolders {
		base_version: u64,
		folders: Vec<GuildFolderInput>,
	},
	OpenJoinServer {
		invite: String,
	},
	SendServerInvite {
		guild_id: String,
		user_id: String,
	},
	OpenServerAdmin {
		guild_id: String,
		page: ServerAdminPage,
	},
	OpenGroupEditor {
		channel_id: String,
	},

	SendMessage {
		channel_id: String,
		content: String,
	},
	SendReply {
		channel_id: String,
		message_id: String,
		content: String,
		mention: bool,
	},
	SendSticker {
		channel_id: String,
		sticker_id: String,
	},
	ForwardMessage {
		channel_id: String,
		message_id: String,
		target_channel_ids: Vec<String>,
		note: String,
	},
	EditMessage {
		channel_id: String,
		message_id: String,
		content: String,
	},
	DeleteMessage {
		channel_id: String,
		message_id: String,
	},
	SetReaction {
		channel_id: String,
		message_id: String,
		emoji: String,
		add: bool,
	},
	SetMessagePinned {
		channel_id: String,
		message_id: String,
		pinned: bool,
	},
	MarkRead {
		channel_id: String,
		message_id: String,
	},
	MarkChannelRead {
		channel_id: String,
	},
	MarkUnread {
		channel_id: String,
		message_id: String,
	},
	MarkGuildRead {
		guild_id: String,
	},
	JumpToUnread,
	CreateThread {
		channel_id: String,
		name: String,
		message_id: Option<String>,
	},
	CreateForumPost {
		parent_id: String,
		title: String,
		content: String,
	},
	SetThreadArchived {
		channel_id: String,
		archived: bool,
	},
	SetThreadLocked {
		channel_id: String,
		locked: bool,
	},
	SetThreadFollowed {
		channel_id: String,
		followed: bool,
	},
	SetThreadPinned {
		channel_id: String,
		pinned: bool,
	},
	RenameThread {
		channel_id: String,
		name: String,
	},
	SetChannelMute {
		channel_id: String,
		duration_seconds: Option<u32>,
	},
	SetChannelNotifications {
		channel_id: String,
		level: u8,
	},
	SetGuildHideMuted {
		guild_id: String,
		hide: bool,
	},
	CreateChannel {
		guild_id: String,
		name: String,
		kind: String,
	},
	CreateCategory {
		guild_id: String,
		name: String,
	},
	DuplicateChannel {
		channel_id: String,
		name: String,
	},
	EditChannel {
		channel_id: String,
		before: ChannelEditInput,
		after: ChannelEditInput,
	},
	DeleteChannel {
		channel_id: String,
	},
	MoveChannel {
		channel_id: String,
		parent_id: Option<String>,
		position: i32,
		lock_permissions: bool,
		shifts: Vec<ChannelPositionInput>,
	},
	CreateServerInvite {
		guild_id: String,
		channel_id: Option<String>,
		max_age: u32,
		max_uses: u16,
		temporary: bool,
	},
	LeaveServer {
		guild_id: String,
	},
	UpdateServerSettings {
		guild_id: String,
		settings: ServerSettingsPatch,
	},
	CreateRole {
		guild_id: String,
		role: RolePatch,
	},
	EditRole {
		guild_id: String,
		role_id: String,
		role: RolePatch,
	},
	DeleteRole {
		guild_id: String,
		role_id: String,
	},
	MoveRole {
		guild_id: String,
		role_id: String,
		position: i32,
	},
	SetMemberRole {
		guild_id: String,
		user_id: String,
		role_id: String,
		assigned: bool,
	},
	SetMemberNickname {
		guild_id: String,
		user_id: String,
		nickname: String,
	},
	KickMember {
		guild_id: String,
		user_id: String,
	},
	PruneMembers {
		guild_id: String,
		days: u8,
		execute: bool,
	},
	SetMemberListVisible {
		guild_id: String,
		enabled: bool,
	},
	RenameServerEmoji {
		guild_id: String,
		emoji_id: String,
		name: String,
	},
	DeleteServerEmoji {
		guild_id: String,
		emoji_id: String,
	},
	LeaveGroup {
		channel_id: String,
	},
	RenameGroup {
		channel_id: String,
		name: String,
	},
	CloseDm {
		channel_id: String,
	},
	SetConversationMuted {
		channel_id: String,
		muted: bool,
	},
}

/// One host proposal per response, applied only after explicit user confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostEffect {
	AppAction {
		action: AppAction,
	},
	TrackedAppAction {
		request_id: String,
		action: AppAction,
	},
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
use crate::{Capability, Error, MAX_EVENT_CONTENT_BYTES, Manifest};
use std::{collections::BTreeSet, io};

fn grant(manifest: &Manifest, capability: Capability) -> Result<(), Error> {
	manifest
		.capabilities
		.contains(&capability)
		.then_some(())
		.ok_or(Error::Capability)
}

pub(crate) fn entity_id(id: &str) -> Result<(), Error> {
	if id.len() > 20
		|| !id.bytes().all(|byte| byte.is_ascii_digit())
		|| !id.parse::<u64>().is_ok_and(|value| value != 0)
	{
		return Err(Error::Invalid);
	}
	Ok(())
}

pub(crate) fn label(value: &str, limit: usize) -> Result<(), Error> {
	if value.len() > limit {
		return Err(Error::Limit);
	}
	if value.is_empty() || value.chars().any(char::is_control) {
		return Err(Error::Invalid);
	}
	Ok(())
}

pub(crate) fn user(user: &UserSnapshot) -> Result<(), Error> {
	entity_id(&user.id)?;
	label(&user.name, 256)
}

pub(crate) fn channel(channel: &ChannelSnapshot) -> Result<(), Error> {
	entity_id(&channel.id)?;
	if let Some(guild) = &channel.guild_id {
		entity_id(guild)?;
	}
	label(&channel.name, 256)
}

pub(crate) fn ids<'a>(items: impl Iterator<Item = &'a str>, limit: usize) -> Result<(), Error> {
	let mut seen = BTreeSet::new();
	for id in items {
		if seen.len() == limit {
			return Err(Error::Limit);
		}
		entity_id(id)?;
		if !seen.insert(id) {
			return Err(Error::Invalid);
		}
	}
	Ok(())
}

/// Counts serialized bytes without allocating an oversized JSON buffer.
pub(crate) fn bounded_bytes(
	value: &(impl Serialize + ?Sized),
	limit: usize,
) -> Result<usize, Error> {
	struct Counter {
		used: usize,
		limit: usize,
	}
	impl io::Write for Counter {
		fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
			if bytes.len() > self.limit - self.used {
				return Err(io::ErrorKind::WriteZero.into());
			}
			self.used += bytes.len();
			Ok(bytes.len())
		}
		fn flush(&mut self) -> io::Result<()> {
			Ok(())
		}
	}
	let mut counter = Counter { used: 0, limit };
	serde_json::to_writer(&mut counter, value).map_err(|error| {
		if error.io_error_kind() == Some(io::ErrorKind::WriteZero) {
			Error::Limit
		} else {
			Error::Invalid
		}
	})?;
	Ok(counter.used)
}

impl AppEventKind {
	/// Whether the manifest can read the data represented by this notification.
	pub fn data_granted(self, capabilities: &[Capability]) -> bool {
		let required: &[Capability] = match self {
			Self::Reactions | Self::Pins | Self::Typing => &[Capability::ConversationActivity],
			Self::Polls => &[Capability::MessageContent],
			Self::Threads => &[Capability::ChannelMetadata, Capability::ForumData],
			Self::Roles => &[Capability::MemberDetails],
			Self::Permissions | Self::Recovered => &[
				Capability::ChannelMetadata,
				Capability::MemberDetails,
				Capability::ForumData,
				Capability::ConversationActivity,
				Capability::MessageContent,
			],
			Self::MessageDetails => &[Capability::MessageDetails, Capability::MessageContent],
			Self::Relationships => &[Capability::Relationships],
			Self::Account => &[Capability::AccountProfile],
			Self::Channels => &[
				Capability::ChannelDirectory,
				Capability::GuildDirectory,
				Capability::ChannelDetails,
				Capability::ChannelMetadata,
				Capability::ForumData,
			],
			Self::Members => &[Capability::Members, Capability::MemberDetails],
			Self::Presence => &[Capability::Presence],
			Self::ReadState => &[Capability::ReadState],
			_ => return false,
		};
		required
			.iter()
			.any(|capability| capabilities.contains(capability))
	}

	pub(crate) fn validate(self, manifest: &Manifest) -> Result<(), Error> {
		grant(manifest, Capability::AppEvents)?;
		if matches!(
			self,
			Self::Account
				| Self::Channels
				| Self::Members
				| Self::Presence
				| Self::ReadState
				| Self::MessageDetails
				| Self::Relationships
				| Self::Threads
				| Self::Roles
				| Self::Permissions
				| Self::Recovered
				| Self::Reactions
				| Self::Pins | Self::Typing
				| Self::Polls
		) {
			grant(manifest, Capability::DataEvents)?;
			if !self.data_granted(&manifest.capabilities) {
				return Err(Error::Capability);
			}
		}
		Ok(())
	}
}

pub(crate) fn image_hash(value: &str) -> Result<(), Error> {
	if value.len() > 128 {
		return Err(Error::Limit);
	}
	if value.is_empty()
		|| !value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
	{
		return Err(Error::Invalid);
	}
	Ok(())
}

pub(crate) fn profile_text(value: &str, limit: usize, multiline: bool) -> Result<(), Error> {
	if value.len() > limit {
		return Err(Error::Limit);
	}
	if value.contains('\0') || (!multiline && value.chars().any(char::is_control)) {
		return Err(Error::Invalid);
	}
	Ok(())
}

impl AppSnapshot {
	/// Exact serialized size, rejecting snapshots above the 64 KiB wire budget.
	pub fn bytes(&self) -> Result<usize, Error> {
		bounded_bytes(self, MAX_APP_SNAPSHOT_BYTES)
	}

	pub fn validate(&self, manifest: &Manifest) -> Result<(), Error> {
		for (present, capability) in [
			(self.message_content.is_some(), Capability::MessageContent),
			(self.forum_data.is_some(), Capability::ForumData),
			(
				self.conversation_activity.is_some(),
				Capability::ConversationActivity,
			),
			(self.channel_metadata.is_some(), Capability::ChannelMetadata),
			(self.member_details.is_some(), Capability::MemberDetails),
			(self.message_details.is_some(), Capability::MessageDetails),
			(self.relationships.is_some(), Capability::Relationships),
			(self.account_profile.is_some(), Capability::AccountProfile),
			(self.guilds.is_some(), Capability::GuildDirectory),
			(self.channel_details.is_some(), Capability::ChannelDetails),
			(self.context.is_some(), Capability::AppContext),
			(self.channels.is_some(), Capability::ChannelDirectory),
			(self.timeline.is_some(), Capability::Timeline),
			(self.members.is_some(), Capability::Members),
			(self.presence.is_some(), Capability::Presence),
			(self.voice.is_some(), Capability::VoiceState),
			(self.read_state.is_some(), Capability::ReadState),
			(self.settings.is_some(), Capability::LocalSettings),
			(self.audio_settings.is_some(), Capability::AudioSettings),
			(self.own_presence.is_some(), Capability::AccountControl),
			(
				self.notification_settings.is_some(),
				Capability::NotificationSettings,
			),
		] {
			if present {
				grant(manifest, capability)?;
			}
		}
		if let Some(group) = &self.message_content {
			group.validate()?;
		}
		if let Some(group) = &self.forum_data {
			group.validate()?;
		}
		if let Some(group) = &self.conversation_activity {
			group.validate()?;
		}
		if let Some(group) = &self.channel_metadata {
			group.validate()?;
		}
		if let Some(group) = &self.member_details {
			group.validate()?;
		}
		if let Some(details) = &self.message_details {
			entity_id(&details.channel_id)?;
			ids(
				details.items.iter().map(|item| item.id.as_str()),
				MAX_MESSAGE_DETAILS,
			)?;
			for item in &details.items {
				if let Some(reply) = &item.reply_to {
					entity_id(reply)?;
				}
				ids(
					item.mention_ids.iter().map(String::as_str),
					MAX_MESSAGE_MENTIONS,
				)?;
				ids(
					item.attachments
						.iter()
						.map(|attachment| attachment.id.as_str()),
					MAX_MESSAGE_ATTACHMENTS,
				)?;
				for attachment in &item.attachments {
					label(&attachment.filename, 256)?;
					if let Some(content_type) = &attachment.content_type {
						label(content_type, 128)?;
					}
				}
				if let Some(reactions) = &item.reactions {
					if reactions.len() > MAX_MESSAGE_REACTIONS {
						return Err(Error::Limit);
					}
					for reaction in reactions {
						if reaction.emoji_id.is_none() && reaction.emoji_name.is_none() {
							return Err(Error::Invalid);
						}
						if let Some(id) = &reaction.emoji_id {
							entity_id(id)?;
						}
						if let Some(name) = &reaction.emoji_name {
							label(name, 128)?;
						}
					}
				}
			}
		}
		if let Some(relationships) = &self.relationships {
			ids(
				relationships.items.iter().map(|item| item.user.id.as_str()),
				MAX_RELATIONSHIPS,
			)?;
			for item in &relationships.items {
				user(&item.user)?;
			}
		}
		if let Some(account) = &self.account_profile {
			user(&account.user)?;
			if let Some(avatar) = &account.avatar {
				image_hash(avatar)?;
			}
			if let Some(profile) = &account.profile {
				if let Some(name) = &profile.display_name {
					profile_text(name, 256, false)?;
				}
				profile_text(&profile.bio, 2048, true)?;
				profile_text(&profile.pronouns, 256, false)?;
			}
		}
		if let Some(guilds) = &self.guilds {
			ids(
				guilds.items.iter().map(|guild| guild.id.as_str()),
				MAX_APP_GUILDS,
			)?;
			for guild in &guilds.items {
				label(&guild.name, 256)?;
				if let Some(icon) = &guild.icon {
					image_hash(icon)?;
				}
			}
		}
		if let Some(details) = &self.channel_details {
			channel(&details.channel)?;
			for id in [&details.parent_id, &details.last_message_id]
				.into_iter()
				.flatten()
			{
				entity_id(id)?;
			}
			ids(
				details
					.recipients
					.iter()
					.map(|recipient| recipient.id.as_str()),
				MAX_CHANNEL_RECIPIENTS,
			)?;
			for recipient in &details.recipients {
				user(recipient)?;
			}
		}
		if let Some(context) = &self.context {
			if let Some(value) = &context.user {
				user(value)?;
			}
			if let Some(value) = &context.channel {
				channel(value)?;
			}
		}
		if let Some(directory) = &self.channels {
			ids(
				directory.items.iter().map(|item| item.id.as_str()),
				MAX_APP_CHANNELS,
			)?;
			for item in &directory.items {
				channel(item)?;
			}
		}
		if let Some(timeline) = &self.timeline {
			entity_id(&timeline.channel_id)?;
			ids(
				timeline.messages.iter().map(|item| item.id.as_str()),
				MAX_APP_MESSAGES,
			)?;
			for message in &timeline.messages {
				user(&message.author)?;
				if message.content.len() > MAX_EVENT_CONTENT_BYTES {
					return Err(Error::Limit);
				}
			}
		}
		if let Some(members) = &self.members {
			entity_id(&members.channel_id)?;
			ids(
				members.items.iter().map(|item| item.id.as_str()),
				MAX_APP_MEMBERS,
			)?;
			for member in &members.items {
				user(member)?;
			}
		}
		if let Some(presence) = &self.presence {
			ids(
				presence.items.iter().map(|item| item.user_id.as_str()),
				MAX_APP_PRESENCES,
			)?;
			for item in &presence.items {
				label(&item.status, 32)?;
			}
		}
		if let Some(voice) = &self.voice {
			if let Some(channel) = &voice.channel_id {
				entity_id(channel)?;
			}
			label(&voice.phase, 64)?;
			ids(
				voice.participants.iter().map(String::as_str),
				MAX_VOICE_PARTICIPANTS,
			)?;
		}
		if let Some(read_state) = &self.read_state
			&& let Some(channel) = &read_state.channel_id
		{
			entity_id(channel)?;
		}
		if let Some(settings) = &self.settings {
			settings.validate()?;
		}
		if let Some(settings) = &self.notification_settings {
			settings.validate()?;
		}
		if let Some(settings) = &self.audio_settings {
			settings.validate()?;
		}
		if let Some(presence) = &self.own_presence {
			presence.validate()?;
		}
		self.bytes().map(|_| ())
	}
}

impl LocalSettingsSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		if !(80..=150).contains(&self.zoom_percent)
			|| !(190..=360).contains(&self.sidebar_width)
			|| self
				.scroll_speed_percent
				.is_some_and(|value| !(25..=300).contains(&value))
		{
			return Err(Error::Invalid);
		}
		Ok(())
	}
}

impl LocalSettingsPatch {
	pub fn validate(&self) -> Result<(), Error> {
		if self == &Self::default()
			|| self
				.zoom_percent
				.is_some_and(|value| !(80..=150).contains(&value))
			|| self
				.sidebar_width
				.is_some_and(|value| !(190..=360).contains(&value))
			|| self
				.scroll_speed_percent
				.is_some_and(|value| !(25..=300).contains(&value))
		{
			return Err(Error::Invalid);
		}
		Ok(())
	}
}

impl NotificationSettingsSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		if self.volume > 100 {
			return Err(Error::Invalid);
		}
		Ok(())
	}
}

impl NotificationSettingsPatch {
	pub fn validate(&self) -> Result<(), Error> {
		if self == &Self::default() || self.volume.is_some_and(|value| value > 100) {
			return Err(Error::Invalid);
		}
		Ok(())
	}
}

fn action_text(value: &str, chars: usize, multiline: bool, empty: bool) -> Result<(), Error> {
	if value.len() > chars * 4 || value.chars().count() > chars {
		return Err(Error::Limit);
	}
	if (!empty && value.trim().is_empty())
		|| value
			.chars()
			.any(|ch| ch.is_control() && !(multiline && matches!(ch, '\n' | '\r' | '\t')))
	{
		return Err(Error::Invalid);
	}
	Ok(())
}

impl OwnProfilePatch {
	pub fn validate(&self) -> Result<(), Error> {
		if self == &Self::default()
			|| (self.clear_global_name && self.global_name.is_some())
			|| (self.clear_accent_color && self.accent_color.is_some())
			|| self.accent_color.is_some_and(|color| color > 0xffffff)
		{
			return Err(Error::Invalid);
		}
		if let Some(value) = &self.global_name {
			action_text(value, 32, false, false)?;
		}
		if let Some(value) = &self.bio {
			action_text(value, 190, true, true)?;
		}
		if let Some(value) = &self.pronouns {
			action_text(value, 40, false, true)?;
		}
		Ok(())
	}
}
impl OwnPresencePatch {
	pub fn validate(&self) -> Result<(), Error> {
		if self == &Self::default()
			|| self
				.status
				.as_deref()
				.is_some_and(|value| !matches!(value, "online" | "idle" | "dnd" | "invisible"))
			|| self.clear_after_seconds.is_some_and(|value| value > 86400)
		{
			return Err(Error::Invalid);
		}
		if let Some(value) = &self.custom_status {
			if value.trim() != value {
				return Err(Error::Invalid);
			}
			action_text(value, 128, false, true)?;
		}
		Ok(())
	}
}
impl AudioSettingsPatch {
	pub fn validate(&self) -> Result<(), Error> {
		if self == &Self::default()
			|| self.input_percent.is_some_and(|value| value > 200)
			|| self.output_percent.is_some_and(|value| value > 200)
			|| self
				.input_profile
				.as_deref()
				.is_some_and(|value| !matches!(value, "voice_isolation" | "studio" | "custom"))
			|| self
				.suppression
				.as_deref()
				.is_some_and(|value| !matches!(value, "off" | "rnnoise" | "webrtc"))
			|| self.suppression_level.is_some_and(|value| value > 3)
			|| self
				.sensitivity_db
				.is_some_and(|value| !(-80..=0).contains(&value))
			|| (self.open_microphone && self.sensitivity_db.is_some())
		{
			return Err(Error::Invalid);
		}
		Ok(())
	}
}
fn audio_patch(volume: Option<u16>, muted: Option<bool>) -> Result<(), Error> {
	if (volume.is_none() && muted.is_none()) || volume.is_some_and(|value| value > 200) {
		return Err(Error::Invalid);
	}
	Ok(())
}
fn server_settings_patch(value: &ServerSettingsPatch) -> Result<(), Error> {
	if value.name.is_none()
		&& value.banner_color.is_none()
		&& value.traits.is_none()
		&& value.description.is_none()
		&& value.system_channel_id.is_none()
		&& !value.clear_system_channel
		&& value.system_channel_flags.is_none()
		&& value.activity_feed.is_none()
		&& value.default_message_notifications.is_none()
		&& value.afk_channel_id.is_none()
		&& !value.clear_afk_channel
		&& value.afk_timeout.is_none()
	{
		return Err(Error::Invalid);
	}
	if value.clear_system_channel && value.system_channel_id.is_some()
		|| value.clear_afk_channel && value.afk_channel_id.is_some()
		|| value.banner_color.is_some_and(|color| color > 0xff_ffff)
		|| value
			.default_message_notifications
			.is_some_and(|level| level > 1)
		|| value
			.afk_timeout
			.is_some_and(|seconds| !matches!(seconds, 60 | 300 | 900 | 1800 | 3600))
	{
		return Err(Error::Invalid);
	}
	if let Some(name) = &value.name {
		action_text(name, 100, false, false)?;
	}
	if let Some(description) = &value.description {
		action_text(description, 300, true, true)?;
	}
	for id in [&value.system_channel_id, &value.afk_channel_id]
		.into_iter()
		.flatten()
	{
		entity_id(id)?;
	}
	if let Some(traits) = &value.traits {
		if traits.len() > 5 {
			return Err(Error::Invalid);
		}
		for item in traits {
			action_text(&item.label, 100, false, false)?;
			if let Some(emoji) = &item.emoji {
				action_text(emoji, 32, false, false)?;
			}
		}
	}
	Ok(())
}
fn role_patch(value: &RolePatch) -> Result<(), Error> {
	if value.name.is_none()
		&& value.primary_color.is_none()
		&& value.secondary_color.is_none()
		&& value.tertiary_color.is_none()
		&& value.permissions.is_none()
		&& value.permission_mask.is_none()
		&& value.hoist.is_none()
		&& value.mentionable.is_none()
		&& value.unicode_emoji.is_none()
		&& !value.clear_unicode_emoji
	{
		return Err(Error::Invalid);
	}
	if value.clear_unicode_emoji && value.unicode_emoji.is_some()
		|| value.primary_color.is_none()
			&& (value.secondary_color.is_some() || value.tertiary_color.is_some())
		|| value.permissions.is_none() && value.permission_mask.is_some()
		|| [
			value.primary_color,
			value.secondary_color,
			value.tertiary_color,
		]
		.into_iter()
		.flatten()
		.any(|color| color > 0xff_ffff)
	{
		return Err(Error::Invalid);
	}
	if let Some(name) = &value.name {
		action_text(name, 100, false, false)?;
	}
	if let Some(emoji) = &value.unicode_emoji {
		action_text(emoji, 32, false, false)?;
	}
	for bits in [&value.permissions, &value.permission_mask]
		.into_iter()
		.flatten()
	{
		bits.parse::<u128>().map_err(|_| Error::Invalid)?;
	}
	Ok(())
}
impl AppAction {
	pub fn required_capability(&self) -> Capability {
		match self {
			Self::SendMessage { .. }
			| Self::SendReply { .. }
			| Self::SendSticker { .. }
			| Self::ForwardMessage { .. } => Capability::MessageSend,
			Self::EditMessage { .. }
			| Self::DeleteMessage { .. }
			| Self::SetMessagePinned { .. } => Capability::MessageManage,
			Self::SetReaction { .. } => Capability::ReactionsControl,
			Self::MarkRead { .. }
			| Self::MarkChannelRead { .. }
			| Self::MarkUnread { .. }
			| Self::MarkGuildRead { .. } => Capability::ReadStateControl,
			Self::JumpToUnread => Capability::Navigation,
			Self::CreateThread { .. }
			| Self::CreateForumPost { .. }
			| Self::SetThreadArchived { .. }
			| Self::SetThreadLocked { .. }
			| Self::SetThreadFollowed { .. }
			| Self::SetThreadPinned { .. }
			| Self::RenameThread { .. } => Capability::ThreadsControl,
			Self::OpenFriendDm { .. }
			| Self::SetFriendNickname { .. }
			| Self::SetUserNote { .. }
			| Self::AddFriend { .. }
			| Self::RemoveFriend { .. }
			| Self::ResolveFriendRequest { .. }
			| Self::SetUserBlocked { .. } => Capability::RelationshipControl,
			Self::SetOwnProfile { .. }
			| Self::SetOwnPresence { .. }
			| Self::SetActivitySharing { .. } => Capability::AccountControl,
			Self::SetAudioSettings { .. }
			| Self::SetParticipantAudio { .. }
			| Self::SetStreamAudio { .. } => Capability::AudioSettings,
			Self::WatchStream { .. } | Self::StopWatching => Capability::VoiceControl,
			Self::JoinVoice { .. } | Self::DeclineCall { .. } => Capability::VoiceConnect,
			Self::SetCamera { .. } => Capability::CameraControl,
			Self::OpenAttachmentPicker { .. } => Capability::MessageSend,
			Self::SelectAudioDevices { .. } | Self::RefreshMediaDevices => {
				Capability::AudioSettings
			}
			Self::SelectCameraDevice { .. } => Capability::CameraControl,
			Self::OpenScreenSharePicker | Self::StopScreenShare => Capability::MediaControl,
			Self::RequestMessageSearch { .. }
			| Self::RequestPins { .. }
			| Self::RequestArchives { .. }
			| Self::RequestMemberSearch { .. }
			| Self::RequestProfile { .. }
			| Self::RequestGifs { .. } => Capability::DataQueries,
			Self::SetMessagingSettings { .. } => Capability::MessagingSettings,
			Self::SetGuildFolders { .. } => Capability::GuildFolders,
			Self::OpenJoinServer { .. }
			| Self::SendServerInvite { .. }
			| Self::OpenServerAdmin { .. } => Capability::ServerControl,
			Self::OpenGroupEditor { .. } => Capability::ChannelControl,
			Self::SetChannelMute { .. }
			| Self::SetChannelNotifications { .. }
			| Self::SetGuildHideMuted { .. }
			| Self::CreateChannel { .. }
			| Self::CreateCategory { .. }
			| Self::DuplicateChannel { .. }
			| Self::EditChannel { .. }
			| Self::DeleteChannel { .. }
			| Self::MoveChannel { .. }
			| Self::LeaveGroup { .. }
			| Self::RenameGroup { .. }
			| Self::CloseDm { .. }
			| Self::SetConversationMuted { .. } => Capability::ChannelControl,
			Self::CreateServerInvite { .. }
			| Self::LeaveServer { .. }
			| Self::UpdateServerSettings { .. }
			| Self::RenameServerEmoji { .. }
			| Self::DeleteServerEmoji { .. } => Capability::ServerControl,
			Self::CreateRole { .. }
			| Self::EditRole { .. }
			| Self::DeleteRole { .. }
			| Self::MoveRole { .. } => Capability::RoleControl,
			Self::SetMemberRole { .. }
			| Self::SetMemberNickname { .. }
			| Self::KickMember { .. }
			| Self::PruneMembers { .. }
			| Self::SetMemberListVisible { .. } => Capability::ModerationControl,
		}
	}
	pub fn validate(&self) -> Result<(), Error> {
		match self {
			Self::SendMessage {
				channel_id,
				content,
			} => {
				entity_id(channel_id)?;
				action_text(content, 2000, true, false)?;
			}
			Self::SendReply {
				channel_id,
				message_id,
				content,
				..
			} => {
				entity_id(channel_id)?;
				entity_id(message_id)?;
				action_text(content, 2000, true, false)?;
			}
			Self::SendSticker {
				channel_id,
				sticker_id,
			} => {
				entity_id(channel_id)?;
				entity_id(sticker_id)?;
			}
			Self::ForwardMessage {
				channel_id,
				message_id,
				target_channel_ids,
				note,
			} => {
				entity_id(channel_id)?;
				entity_id(message_id)?;
				if target_channel_ids.is_empty() || target_channel_ids.len() > 5 {
					return Err(Error::Invalid);
				}
				let mut targets = std::collections::BTreeSet::new();
				for target in target_channel_ids {
					entity_id(target)?;
					if !targets.insert(target) {
						return Err(Error::Invalid);
					}
				}
				action_text(note, 2000, true, true)?;
			}
			Self::EditMessage {
				channel_id,
				message_id,
				content,
			} => {
				entity_id(channel_id)?;
				entity_id(message_id)?;
				action_text(content, 2000, true, false)?;
			}
			Self::DeleteMessage {
				channel_id,
				message_id,
			}
			| Self::SetMessagePinned {
				channel_id,
				message_id,
				..
			}
			| Self::MarkRead {
				channel_id,
				message_id,
			}
			| Self::MarkUnread {
				channel_id,
				message_id,
			} => {
				entity_id(channel_id)?;
				entity_id(message_id)?;
			}
			Self::SetReaction {
				channel_id,
				message_id,
				emoji,
				..
			} => {
				entity_id(channel_id)?;
				entity_id(message_id)?;
				label(emoji, 128)?;
				if let Some((name, id)) = emoji.split_once(':') {
					if !(2..=32).contains(&name.len())
						|| !name
							.bytes()
							.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
					{
						return Err(Error::Invalid);
					}
					entity_id(id)?;
				}
			}
			Self::MarkChannelRead { channel_id }
			| Self::SetThreadArchived { channel_id, .. }
			| Self::SetThreadLocked { channel_id, .. }
			| Self::SetThreadFollowed { channel_id, .. }
			| Self::SetThreadPinned { channel_id, .. }
			| Self::DeclineCall { channel_id }
			| Self::JoinVoice { channel_id, .. } => entity_id(channel_id)?,
			Self::MarkGuildRead { guild_id } => entity_id(guild_id)?,
			Self::CreateThread {
				channel_id,
				name,
				message_id,
			} => {
				entity_id(channel_id)?;
				if let Some(message) = message_id {
					entity_id(message)?;
				}
				action_text(name, 100, false, false)?;
			}
			Self::RenameThread { channel_id, name } => {
				entity_id(channel_id)?;
				action_text(name, 100, false, false)?;
			}
			Self::CreateForumPost {
				parent_id,
				title,
				content,
			} => {
				entity_id(parent_id)?;
				action_text(title, 100, false, false)?;
				action_text(content, 2000, true, false)?;
			}
			Self::OpenFriendDm { user_id }
			| Self::RemoveFriend { user_id }
			| Self::ResolveFriendRequest { user_id, .. }
			| Self::SetUserBlocked { user_id, .. }
			| Self::WatchStream { user_id } => entity_id(user_id)?,
			Self::SetFriendNickname { user_id, text } => {
				entity_id(user_id)?;
				action_text(text, 32, false, true)?;
			}
			Self::SetUserNote { user_id, text } => {
				entity_id(user_id)?;
				action_text(text, 256, true, true)?;
				if text.contains('\r') {
					return Err(Error::Invalid);
				}
			}
			Self::AddFriend { username } => {
				if !(2..=32).contains(&username.len())
					|| username.contains("..")
					|| !username.bytes().all(|byte| {
						byte.is_ascii_lowercase()
							|| byte.is_ascii_digit()
							|| matches!(byte, b'_' | b'.')
					}) {
					return Err(Error::Invalid);
				}
			}
			Self::SetOwnProfile { profile } => profile.validate()?,
			Self::SetOwnPresence { presence } => presence.validate()?,
			Self::SetAudioSettings { settings } => settings.validate()?,
			Self::SetParticipantAudio {
				user_id,
				volume_percent,
				muted,
			} => {
				entity_id(user_id)?;
				audio_patch(*volume_percent, *muted)?;
			}
			Self::SetStreamAudio {
				volume_percent,
				muted,
			} => audio_patch(*volume_percent, *muted)?,
			Self::OpenAttachmentPicker { channel_id } => entity_id(channel_id)?,
			Self::SelectAudioDevices {
				input_id,
				output_id,
			} => {
				if input_id.is_none() && output_id.is_none() {
					return Err(Error::Invalid);
				}
				for value in [input_id, output_id].into_iter().flatten() {
					label(value, 256)?;
				}
			}
			Self::SelectCameraDevice { device_id } => {
				if let Some(value) = device_id {
					label(value, 256)?;
				}
			}
			Self::RequestMessageSearch { query, before_id } => {
				profile_text(query, 1024, false)?;
				if query.trim().is_empty() {
					return Err(Error::Invalid);
				}
				if let Some(id) = before_id {
					entity_id(id)?;
				}
			}
			Self::RequestPins { before } => {
				if let Some(before) = before {
					before.parse::<i128>().map_err(|_| Error::Invalid)?;
				}
			}
			Self::RequestArchives {
				parent_id, before, ..
			} => {
				entity_id(parent_id)?;
				if let Some(before) = before {
					before.parse::<i128>().map_err(|_| Error::Invalid)?;
				}
			}
			Self::RequestMemberSearch { channel_id, query } => {
				entity_id(channel_id)?;
				label(query, 256)?;
			}
			Self::RequestProfile { user_id, guild_id } => {
				entity_id(user_id)?;
				if let Some(guild_id) = guild_id {
					entity_id(guild_id)?;
				}
			}
			Self::RequestGifs { query } => {
				if let Some(query) = query {
					label(query, 1024)?;
				}
			}
			Self::SetMessagingSettings { change } => change.validate()?,
			Self::SetGuildFolders {
				base_version,
				folders,
			} => {
				if *base_version > u32::MAX.into() {
					return Err(Error::Invalid);
				}
				if folders.len() > 200 {
					return Err(Error::Limit);
				}
				for folder in folders {
					folder.validate()?;
				}
			}
			Self::OpenJoinServer { invite } => label(invite, 512)?,
			Self::SendServerInvite { guild_id, user_id } => {
				entity_id(guild_id)?;
				entity_id(user_id)?;
			}
			Self::OpenServerAdmin { guild_id, .. } => entity_id(guild_id)?,
			Self::OpenGroupEditor { channel_id } => entity_id(channel_id)?,
			Self::SetChannelMute {
				channel_id,
				duration_seconds,
			} => {
				entity_id(channel_id)?;
				if duration_seconds
					.is_some_and(|value| !matches!(value, 0 | 900 | 3600 | 10800 | 28800 | 86400))
				{
					return Err(Error::Invalid);
				}
			}
			Self::SetChannelNotifications { channel_id, level } => {
				entity_id(channel_id)?;
				if *level > 3 {
					return Err(Error::Invalid);
				}
			}
			Self::SetGuildHideMuted { guild_id, .. } | Self::LeaveServer { guild_id } => {
				entity_id(guild_id)?
			}
			Self::CreateChannel {
				guild_id,
				name,
				kind,
			} => {
				entity_id(guild_id)?;
				action_text(name, 100, false, false)?;
				if !matches!(kind.as_str(), "text" | "voice" | "forum") {
					return Err(Error::Invalid);
				}
			}
			Self::CreateCategory { guild_id, name } => {
				entity_id(guild_id)?;
				action_text(name, 100, false, false)?;
			}
			Self::DuplicateChannel { channel_id, name }
			| Self::RenameGroup { channel_id, name } => {
				entity_id(channel_id)?;
				action_text(name, 100, false, false)?;
			}
			Self::EditChannel {
				channel_id,
				before,
				after,
			} => {
				entity_id(channel_id)?;
				for edit in [before, after] {
					action_text(&edit.name, 100, false, false)?;
					action_text(&edit.topic, 4096, true, true)?;
					if edit.slowmode > 21_600 || edit.overwrites.len() > 100 {
						return Err(Error::Invalid);
					}
					let mut ids = std::collections::BTreeSet::new();
					for overwrite in &edit.overwrites {
						entity_id(&overwrite.id)?;
						if overwrite.kind > 1
							|| overwrite.allow.parse::<u128>().is_err()
							|| overwrite.deny.parse::<u128>().is_err()
							|| !ids.insert((&overwrite.id, overwrite.kind))
						{
							return Err(Error::Invalid);
						}
					}
				}
			}
			Self::DeleteChannel { channel_id }
			| Self::LeaveGroup { channel_id }
			| Self::CloseDm { channel_id }
			| Self::SetConversationMuted { channel_id, .. } => entity_id(channel_id)?,
			Self::MoveChannel {
				channel_id,
				parent_id,
				position,
				shifts,
				..
			} => {
				entity_id(channel_id)?;
				if let Some(parent) = parent_id {
					entity_id(parent)?;
				}
				if *position < 0 || shifts.len() > 100 {
					return Err(Error::Invalid);
				}
				let mut ids = std::collections::BTreeSet::new();
				for shift in shifts {
					entity_id(&shift.channel_id)?;
					if shift.position < 0 || !ids.insert(&shift.channel_id) {
						return Err(Error::Invalid);
					}
				}
			}
			Self::CreateServerInvite {
				guild_id,
				channel_id,
				max_age,
				max_uses,
				..
			} => {
				entity_id(guild_id)?;
				if let Some(channel) = channel_id {
					entity_id(channel)?;
				}
				if *max_age > 2_592_000 || *max_uses > 100 {
					return Err(Error::Invalid);
				}
			}
			Self::UpdateServerSettings { guild_id, settings } => {
				entity_id(guild_id)?;
				server_settings_patch(settings)?;
			}
			Self::CreateRole { guild_id, role } => {
				entity_id(guild_id)?;
				role_patch(role)?;
			}
			Self::EditRole {
				guild_id,
				role_id,
				role,
			} => {
				entity_id(guild_id)?;
				entity_id(role_id)?;
				role_patch(role)?;
			}
			Self::DeleteRole { guild_id, role_id }
			| Self::MoveRole {
				guild_id, role_id, ..
			} => {
				entity_id(guild_id)?;
				entity_id(role_id)?;
				if matches!(self, Self::MoveRole { position, .. } if !(1..=4096).contains(position))
				{
					return Err(Error::Invalid);
				}
			}
			Self::SetMemberRole {
				guild_id,
				user_id,
				role_id,
				..
			} => {
				entity_id(guild_id)?;
				entity_id(user_id)?;
				entity_id(role_id)?;
			}
			Self::SetMemberNickname {
				guild_id,
				user_id,
				nickname,
			} => {
				entity_id(guild_id)?;
				entity_id(user_id)?;
				action_text(nickname, 32, false, true)?;
			}
			Self::KickMember { guild_id, user_id } => {
				entity_id(guild_id)?;
				entity_id(user_id)?;
			}
			Self::PruneMembers { guild_id, days, .. } => {
				entity_id(guild_id)?;
				if !matches!(days, 1 | 7 | 30) {
					return Err(Error::Invalid);
				}
			}
			Self::SetMemberListVisible { guild_id, .. } => entity_id(guild_id)?,
			Self::RenameServerEmoji {
				guild_id,
				emoji_id,
				name,
			} => {
				entity_id(guild_id)?;
				entity_id(emoji_id)?;
				if !(2..=32).contains(&name.len())
					|| !name
						.bytes()
						.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
				{
					return Err(Error::Invalid);
				}
			}
			Self::DeleteServerEmoji { guild_id, emoji_id } => {
				entity_id(guild_id)?;
				entity_id(emoji_id)?;
			}
			Self::JumpToUnread
			| Self::StopWatching
			| Self::RefreshMediaDevices
			| Self::OpenScreenSharePicker
			| Self::StopScreenShare
			| Self::SetActivitySharing { .. }
			| Self::SetCamera { .. } => {}
		}
		bounded_bytes(self, MAX_HOST_EFFECT_BYTES).map(|_| ())
	}
}

impl ActionResult {
	pub fn validate(&self) -> Result<(), Error> {
		label(&self.request_id, 64)?;
		if matches!(self.status, ActionResultStatus::Accepted)
			!= matches!(self.code, ActionResultCode::Accepted)
		{
			return Err(Error::Invalid);
		}
		Ok(())
	}
}

impl HostEffect {
	pub fn required_capability(&self) -> Capability {
		match self {
			Self::AppAction { action } => action.required_capability(),
			Self::TrackedAppAction { .. } => Capability::ActionFeedback,
			Self::Navigate { .. }
			| Self::Home
			| Self::OpenView { .. }
			| Self::OpenProfile { .. }
			| Self::JumpToMessage { .. }
			| Self::Search { .. } => Capability::Navigation,
			Self::Notice { .. } => Capability::LocalNotices,
			Self::CopyText { .. } => Capability::ClipboardWrite,
			Self::SetVoice { .. } | Self::LeaveVoice => Capability::VoiceControl,
			Self::SetLocalSettings { .. } => Capability::LocalSettings,
			Self::SetNotificationSettings { .. } => Capability::NotificationSettings,
		}
	}

	pub fn validate(&self, manifest: &Manifest) -> Result<(), Error> {
		grant(manifest, self.required_capability())?;
		match self {
			Self::AppAction { action } => action.validate()?,
			Self::TrackedAppAction { request_id, action } => {
				label(request_id, 64)?;
				action.validate()?;
				grant(manifest, action.required_capability())?;
			}
			Self::Navigate { channel_id } => entity_id(channel_id)?,
			Self::OpenProfile { user_id } => entity_id(user_id)?,
			Self::JumpToMessage {
				channel_id,
				message_id,
			} => {
				entity_id(channel_id)?;
				entity_id(message_id)?;
			}
			Self::Search { query } => label(query, 256)?,
			Self::Notice { text } => {
				if text.len() > 1024 {
					return Err(Error::Limit);
				}
				if text.trim().is_empty() {
					return Err(Error::Invalid);
				}
			}
			Self::CopyText { text } => {
				if text.len() > 4096 {
					return Err(Error::Limit);
				}
			}
			Self::SetLocalSettings { settings } => settings.validate()?,
			Self::SetNotificationSettings { settings } => settings.validate()?,
			Self::Home | Self::OpenView { .. } | Self::SetVoice { .. } | Self::LeaveVoice => {}
		}
		bounded_bytes(self, MAX_HOST_EFFECT_BYTES).map(|_| ())
	}
}

pub(crate) fn validate_effects(effects: &[HostEffect], manifest: &Manifest) -> Result<(), Error> {
	if effects.len() > MAX_HOST_EFFECTS {
		return Err(Error::Limit);
	}
	for effect in effects {
		effect.validate(manifest)?;
	}
	bounded_bytes(effects, MAX_HOST_EFFECT_BYTES).map(|_| ())
}

impl AudioSettingsSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		AudioSettingsPatch {
			input_percent: Some(self.input_percent),
			output_percent: Some(self.output_percent),
			push_to_talk: Some(self.push_to_talk),
			input_profile: Some(self.input_profile.clone()),
			suppression: Some(self.suppression.clone()),
			suppression_level: Some(self.suppression_level),
			echo_cancellation: Some(self.echo_cancellation),
			automatic_gain: Some(self.automatic_gain),
			sensitivity_db: self.sensitivity_db,
			open_microphone: self.sensitivity_db.is_none(),
		}
		.validate()
	}
}
impl OwnPresenceSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		OwnPresencePatch {
			status: Some(self.status.clone()),
			custom_status: Some(self.custom_status.clone()),
			clear_after_seconds: None,
		}
		.validate()
	}
}
