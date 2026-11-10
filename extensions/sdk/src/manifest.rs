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
	ExternalNavigation,
	ContextMenus,
	InterfaceTweaks,
	/// Apply allowed notification and mute changes for your own account from message- and
	/// app-event handlers, without an Apply button.
	BackgroundEffects,
	/// Send HTTPS requests to the host names listed in the manifest's `network_hosts`;
	/// each response re-runs the requesting action.
	Network,
	/// Show text in place of loaded messages, with the plugin's icon beside the name to
	/// switch back (a translation).
	MessageAlternatives,
	/// Add a message only you can see to the open conversation (a message preview).
	LocalMessages,
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

/// Where an action sits, unless the user keeps its plugin grouped under its name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
	/// Menus: first entries.
	Top,
	/// Message menu: right after the copy entries (the default for message actions).
	#[default]
	AfterCopy,
	/// Menus: last entries.
	Bottom,
	/// Message actions: a toolbar button while hovering a message. Needs an icon.
	Toolbar,
	/// Composer actions: a composer bar button. Needs an icon.
	Bar,
	/// Composer actions: an entry in the composer's "Tools" menu.
	Tools,
}

/// Declared entry point; its ID arrives in `Invocation::action`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
	pub id: String,
	pub label: String,
	pub surface: Surface,
	/// The plugin's own SVG glyph, at most 8 KiB; only its shape is drawn.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub icon: Option<String>,
	/// Where the action sits, unless the user keeps its plugin grouped under its name.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub placement: Option<Placement>,
	/// The plugin's own submenu for this action, e.g. "Privacy Pruning".
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub menu: Option<String>,
	/// A composer button that stays on or off; its state arrives as the `on` value.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub toggle: bool,
	/// A toggle's icon while it is on.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub icon_on: Option<String>,
	/// A composer bar button shown only while the draft has text.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub needs_draft: bool,
}
