use crate::AppInvocation;
use serde::{Deserialize, Serialize};

/// The message a `message` action was chosen on. Granted with `selected_message`,
/// like the text in `Invocation::selected_message`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedMessageRef {
	pub channel_id: String,
	pub message_id: String,
	pub author_id: String,
	/// Absent in direct and group messages.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub guild_id: Option<String>,
}

/// Opt-in message-menu context, preserving the `Invocation` and `AppInvocation`
/// struct-literal APIs. Older hosts omit `selected_message_ref`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageActionInvocation {
	#[serde(flatten)]
	pub invocation: AppInvocation,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub selected_message_ref: Option<SelectedMessageRef>,
}
