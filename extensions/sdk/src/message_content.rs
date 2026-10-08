use serde::{Deserialize, Serialize};
pub const MAX_MESSAGE_CONTENT_BYTES: usize = 8 * 1024;
pub const MAX_RICH_MESSAGES: usize = 10;
pub const MAX_MESSAGE_EMBEDS: usize = 3;
pub const MAX_MESSAGE_EMBED_FIELDS: usize = 4;
pub const MAX_CONTENT_STICKERS: usize = 3;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContentSnapshot {
	pub channel_id: String,
	pub items: Vec<RichMessageSnapshot>,
	pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RichMessageSnapshot {
	pub id: String,
	pub embeds: Vec<EmbedSummarySnapshot>,
	pub embeds_truncated: bool,
	pub embeds_suppressed: bool,
	pub stickers: Vec<MessageStickerSnapshot>,
	pub stickers_truncated: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub reference: Option<MessageReferenceSnapshot>,
	pub poll: PollAvailability,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedSummarySnapshot {
	pub kind: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub author: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub footer: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub color: Option<u32>,
	pub fields: Vec<EmbedFieldSnapshot>,
	pub fields_truncated: bool,
	pub has_image: bool,
	pub has_thumbnail: bool,
	pub has_video: bool,
	pub limited: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedFieldSnapshot {
	pub name: String,
	pub value: String,
	pub inline: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageStickerSnapshot {
	pub id: String,
	pub name: String,
	pub format_type: u8,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReferenceSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub message_id: Option<String>,
	pub deleted: bool,
	pub forwarded: bool,
}
/// Poll questions/options/results are not retained by the client protocol decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollAvailability {
	Absent,
	Unsupported,
}
