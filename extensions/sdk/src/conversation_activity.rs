use serde::{Deserialize, Serialize};
/// Loaded selected-channel activity; never a complete pin inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]

pub struct ConversationActivitySnapshot {
	pub channel_id: String,
	pub typing_user_ids: Vec<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub pinned_message_ids: Option<Vec<String>>,
	pub pins_truncated: bool,
}
