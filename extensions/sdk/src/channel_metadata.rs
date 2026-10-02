use crate::ChannelSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Loaded metadata and permission decisions for the selected readable guild channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMetadataSnapshot {
	pub channel_id: String,
	pub guild_id: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub parent: Option<ChannelSnapshot>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub category: Option<ChannelSnapshot>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub topic: Option<String>,
	pub topic_truncated: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub slowmode_seconds: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub nsfw: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub thread: Option<ThreadMetadataSnapshot>,
	/// Null means unknown. These are permission decisions, not guarantees an action can run.
	pub permissions: BTreeMap<ChannelPermission, Option<bool>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMetadataSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub owner_id: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub message_count: Option<u32>,
	pub archived: Option<bool>,
	pub locked: Option<bool>,
	pub pinned: Option<bool>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelPermission {
	ViewChannel,
	ReadMessageHistory,
	SendMessages,
	SendMessagesInThreads,
	AttachFiles,
	EmbedLinks,
	AddReactions,
	MentionEveryone,
	UseExternalEmojis,
	UseExternalStickers,
	UseApplicationCommands,
	ManageChannels,
	ManageMessages,
	ManageRoles,
	ManageThreads,
	CreatePublicThreads,
	CreatePrivateThreads,
	ManageWebhooks,
	Connect,
	Speak,
	Stream,
	MuteMembers,
	DeafenMembers,
	MoveMembers,
	UseVad,
	PinMessages,
}
