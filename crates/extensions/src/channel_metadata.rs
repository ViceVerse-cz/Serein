use crate::ChannelSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Loaded metadata and permission decisions for the selected readable guild channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

impl ChannelMetadataSnapshot {
	pub fn validate(&self) -> Result<(), crate::Error> {
		use crate::app::{bounded_bytes, channel, entity_id, profile_text};
		entity_id(&self.channel_id)?;
		entity_id(&self.guild_id)?;
		for related in [&self.parent, &self.category].into_iter().flatten() {
			channel(related)?;
			if related.guild_id.as_deref() != Some(&self.guild_id) {
				return Err(crate::Error::Invalid);
			}
		}
		if self.category.as_ref().is_some_and(|c| c.kind != 4) {
			return Err(crate::Error::Invalid);
		}
		if let Some(topic) = &self.topic {
			profile_text(topic, 2048, true)?;
		}
		if self.slowmode_seconds.is_some_and(|s| s > 21600) {
			return Err(crate::Error::Invalid);
		}
		if let Some(thread) = &self.thread
			&& let Some(owner) = &thread.owner_id
		{
			entity_id(owner)?;
		}
		bounded_bytes(self, 6 * 1024)?;
		Ok(())
	}
}
