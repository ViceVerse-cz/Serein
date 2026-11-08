use serde::{Deserialize, Serialize};

/// Package metadata for authoring tools and tests, not an invocation or a permission grant.
/// Serde checks the document shape; the host remains responsible for semantic validation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
	pub api_version: u32,
	pub id: String,
	pub name: String,
	pub version: String,
	pub author: String,
	pub license: String,
	pub source: String,
	pub kind: ExtensionKind,
	#[serde(default)]
	pub capabilities: Vec<Capability>,
	#[serde(default)]
	pub actions: Vec<Action>,
	/// Exact HTTPS host names a `network` plugin may reach; no other host is reachable.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub network_hosts: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionKind {
	Plugin,
	Theme,
}

/// Explicit permissions requested by a manifest. Each still requires user consent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
	RichPresence,
	RelationshipControl,
	AccountControl,
	AudioSettings,
	VoiceConnect,
	CameraControl,

	MessageSend,
	MessageManage,
	ReactionsControl,
	ReadStateControl,
	ThreadsControl,
	ChannelControl,
	ServerControl,
	RoleControl,
	ModerationControl,
	MediaControl,
	ActionFeedback,
	DataQueries,
	MessagingSettings,
	GuildFolders,

	MessageContent,
	ForumData,
	ConversationActivity,
	ChannelMetadata,
	MemberDetails,
	SelectedMessage,
	Composer,
	Storage,
	DeletedMessages,
	ImageSharing,
	Appearance,
	MessageEvents,
	AppContext,
	ChannelDirectory,
	Timeline,
	Members,
	Presence,
	VoiceState,
	ReadState,
	LocalSettings,
	NotificationSettings,
	Navigation,
	LocalNotices,
	ClipboardWrite,
	VoiceControl,
	AppEvents,
	AccountProfile,
	GuildDirectory,
	ChannelDetails,
	DataEvents,
	MessageDetails,
	Relationships,
	ContextMenus,
	InterfaceTweaks,
	/// Apply allowed notification and mute changes for your own account from message- and
	/// app-event handlers, without an Apply button.
	BackgroundEffects,
	/// Send HTTPS requests to the host names listed in the manifest's `network_hosts`;
	/// each response re-runs the requesting action.
	Network,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
	Message,
	Composer,
	Panel,
	Activation,
	MessageEvent,
	AppEvent,
	Tick,
	User,
	Channel,
	Server,
	Role,
}

/// Declared entry point; its ID arrives in `Invocation::action`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
	pub id: String,
	pub label: String,
	pub surface: Surface,
}
