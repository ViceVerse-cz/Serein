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

/// The item a `user`, `channel`, `server` or `role` menu action was chosen on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuTarget {
	User {
		user_id: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		guild_id: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		channel_id: Option<String>,
	},
	Channel {
		channel_id: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		guild_id: Option<String>,
	},
	Server {
		guild_id: String,
	},
	/// A role in a member's profile, with the name and `0xRRGGBB` color shown on its chip.
	Role {
		guild_id: String,
		role_id: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		user_id: Option<String>,
		name: String,
		color: u32,
	},
}

/// Opt-in context-menu input: an `AppInvocation` plus the menu's target.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenuInvocation {
	#[serde(flatten)]
	pub invocation: AppInvocation,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub menu_target: Option<MenuTarget>,
}
