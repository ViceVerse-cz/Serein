//! Results that appear in the conversation itself rather than in a result window: text shown
//! in place of a message (a translation) and a message only you can see (a preview).
use serde::{Deserialize, Serialize};

/// Most alternatives one output may set.
pub const MAX_ALTERNATIVES: usize = 32;
/// Longest alternative or local message text, in UTF-8 bytes.
pub const MAX_CHAT_TEXT_BYTES: usize = 8 * 1024;

/// Text shown in place of a loaded message's content. Serein draws the plugin's icon beside
/// the author's name; clicking it switches between this text and the original.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageAlternative {
	pub channel_id: String,
	pub message_id: String,
	pub text: String,
	/// The icon's tooltip, such as "Translated from Romanian".
	pub note: String,
	/// Show the alternative now; false keeps the original with the switch beside the name.
	#[serde(default)]
	pub show: bool,
}

/// A message only you can see, added to the open conversation, authored by you and marked
/// "Only you can see this · Dismiss".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalMessage {
	/// The conversation; empty means the open one.
	#[serde(default)]
	pub channel_id: String,
	pub content: String,
}

/// Output with in-conversation results, preserving `NetworkOutput`. Each field needs its
/// capability: `message_alternatives`, `local_messages`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatOutput {
	#[serde(flatten)]
	pub output: crate::NetworkOutput,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub alternatives: Vec<MessageAlternative>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub local_message: Option<LocalMessage>,
	/// Interface tweaks, from activation, panel and composer toggle actions.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub interface_tweaks: Option<Vec<crate::InterfaceTweak>>,
}
