use serde::{Deserialize, Serialize};

pub const MAX_APP_SNAPSHOT_BYTES: usize = 64 * 1024;
pub const MAX_APP_CHANNELS: usize = 100;
pub const MAX_APP_MESSAGES: usize = 50;
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEventKind {
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
}

use crate::{Invocation, MessageEvent, Output};

/// Optional app data is capability-scoped; the original invocation types stay unchanged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppInvocation {
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
