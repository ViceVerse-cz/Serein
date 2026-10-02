use serde::{Deserialize, Serialize};
/// Loaded selected-channel activity; never a complete pin inventory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationActivitySnapshot {
	pub channel_id: String,
	pub typing_user_ids: Vec<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub pinned_message_ids: Option<Vec<String>>,
	pub pins_truncated: bool,
}

impl ConversationActivitySnapshot {
	pub fn validate(&self) -> Result<(), crate::Error> {
		crate::app::entity_id(&self.channel_id)?;
		crate::app::ids(self.typing_user_ids.iter().map(String::as_str), 8)?;
		if let Some(pins) = &self.pinned_message_ids {
			crate::app::ids(pins.iter().map(String::as_str), 20)?;
		}
		crate::app::bounded_bytes(self, 2048)?;
		Ok(())
	}
}
