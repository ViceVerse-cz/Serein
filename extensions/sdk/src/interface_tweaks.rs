use serde::{Deserialize, Serialize};

/// Host-defined interface behavior an activation or panel action may switch on for the whole
/// client. Requires the `interface_tweaks` capability; check `HostInfo::supports_tweak` first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceTweak {
	/// Typing indicators show still dots instead of animating.
	StaticTypingIndicator,
	/// Profiles use the client's colors instead of the member's profile theme.
	PlainProfiles,
	/// A member's roles show in full instead of collapsing into a `+N` chip.
	ExpandedRoles,
	/// Pending friend and message request counts stay off the Home button, the Pending tab
	/// and the message-request list.
	HiddenRequestCounts,
	/// A direct or group conversation alerts once until it is read, instead of for every
	/// message.
	OnePingPerDirectMessage,
	/// Messages from users you blocked are left out of conversations.
	HiddenBlockedMessages,
	/// Profiles leave out your private note about the member; the user menu still edits it.
	HiddenProfileNotes,
	/// The stream preview shown on hovering a live participant is twice as large, up to the
	/// preview image's own resolution.
	LargeStreamPreview,
	/// Double-clicking a message's blank space edits your own message or replies to someone
	/// else's, in place of the double-click reaction.
	DoubleClickReplyEdit,
	/// Message authors and member-list names show the account's username beside a differing
	/// display name or server nickname.
	UsernameBesideName,
	/// The server owner carries a crown beside their name in messages and the member list.
	OwnerCrown,
	/// Message authors and member-list names show which kinds of client (desktop, web,
	/// mobile) the person is online from.
	PlatformIndicators,
	/// A user mention takes the member's top colored role, as a role mention does.
	RoleColoredMentions,
	/// Forums open in list view on this client, whatever view the server set as default.
	/// Takes precedence over `ForumGalleryView`.
	ForumListView,
	/// Forums open in gallery view on this client, whatever view the server set as default.
	ForumGalleryView,
	/// Forums list posts by recent activity on this client, whatever order the server set.
	/// Takes precedence over `ForumSortByCreation`.
	ForumSortByActivity,
	/// Forums list the newest posts first on this client, whatever order the server set.
	ForumSortByCreation,
	/// Opening a server folder closes the other open folders, so one is open at a time.
	SingleOpenFolder,
	/// Profiles also count and list the group conversations you share with the person.
	MutualGroups,
}

/// Activation or panel output that sets the plugin's interface tweaks, preserving `Output`
/// struct literals. `None` keeps the current set; `Some` replaces it, and an empty list turns
/// the plugin's tweaks off. List each tweak once; the host keeps a tweak on while any enabled
/// plugin requests it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceTweaksOutput {
	#[serde(flatten)]
	pub output: crate::Output,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub interface_tweaks: Option<Vec<InterfaceTweak>>,
}

/// Message- or app-event output that applies one allowed notification or mute change without
/// an Apply button, preserving `Output` struct literals. Requires the `background_effects`
/// capability and the action's own capability; only `HostEffect::AppAction` with
/// `SetServerNotifications`, `SetChannelMute`, `SetChannelNotifications`, `SetGuildHideMuted`
/// or `SetConversationMuted` is accepted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundEffectsOutput {
	#[serde(flatten)]
	pub output: crate::Output,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub background_effects: Vec<crate::HostEffect>,
}
