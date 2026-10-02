use serde::{Deserialize, Serialize};

pub const MAX_FORUM_POSTS: usize = 10;
pub const MAX_FORUM_DATA_BYTES: usize = 6 * 1024;

/// Already-loaded, accessible threads under the selected parent. Never a complete service listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumDataSnapshot {
	pub channel_id: String,
	pub guild_id: String,
	pub parent_id: String,
	pub posts: Vec<ForumPostSnapshot>,
	pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForumPostSnapshot {
	pub id: String,
	pub name: String,
	pub kind: u8,
	pub message_count: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub owner_id: Option<String>,
	pub archived: Option<bool>,
	pub locked: Option<bool>,
	pub pinned: Option<bool>,
	/// Loaded follow state, not a claim about private-thread membership.
	pub followed: Option<bool>,
}
