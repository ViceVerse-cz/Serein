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
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
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

impl HostEffect {
	pub fn required_capability(&self) -> Capability {
		match self {
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
