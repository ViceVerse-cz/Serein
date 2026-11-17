//! Declarative, capability-gated replacement for the client's main panes. Host mirror of
//! extensions/sdk/src/interface_ui.rs, the contract authority.
//!
//! A plugin with the `interface_ui` grant returns a bounded flat tree of rows; the host
//! renders it in place of the navigation, conversation and member panes and routes clicks on
//! rows back to the plugin through its declared [`INTERFACE_CLICK_ACTION`] panel action. The
//! schema knows nothing about servers, voice or any particular product: indent, icon, tint,
//! badge and emphasis are generic primitives.

use serde::{Deserialize, Serialize};

/// Maximum rows in one interface tree.
pub const MAX_INTERFACE_NODES: usize = 512;
/// Maximum UTF-8 bytes of one row label.
pub const MAX_INTERFACE_TEXT_BYTES: usize = 256;
/// Maximum UTF-8 bytes of one row badge.
pub const MAX_INTERFACE_BADGE_BYTES: usize = 32;
/// Maximum UTF-8 bytes of one row action id.
pub const MAX_INTERFACE_ID_BYTES: usize = 64;
/// Maximum indent step of a row.
pub const MAX_INTERFACE_INDENT: u8 = 8;

/// The declared panel action the host invokes when an interface row is clicked, with the
/// row's id in `Invocation::values["id"]` and `double_clicked` set for double clicks. Rows
/// of a plugin that does not declare this action are inert.
pub const INTERFACE_CLICK_ACTION: &str = "interface_click";

/// Generic glyph names the host already paints; unknown names are rejected at validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceIcon {
	Speaker,
	MicrophoneSlash,
	HeadphonesSlash,
	ChevronDown,
	ChevronRight,
	Server,
	Hash,
	Gear,
	HangUp,
}

/// Semantic tint of a row's glyph, mapped to the host palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceTint {
	Positive,
	Danger,
	Muted,
}

pub(crate) fn is_false(value: &bool) -> bool {
	!value
}

/// One row. `id` is the action the host invokes when the row is clicked; an empty id makes
/// the row inert. `double_clicked` on the resulting invocation distinguishes a double click.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceNode {
	#[serde(default)]
	pub id: String,
	#[serde(default)]
	pub indent: u8,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub icon: Option<InterfaceIcon>,
	pub text: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub badge: Option<String>,
	#[serde(default, skip_serializing_if = "is_false")]
	pub emphasis: bool,
	#[serde(default, skip_serializing_if = "is_false")]
	pub selected: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub tint: Option<InterfaceTint>,
}

/// A native host window shown beside the tree. The plugin names what to show; the host
/// renders its own existing call stage or conversation view and rechecks access.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InterfaceWindow {
	/// The active call's stage; nothing shows when there is no active call.
	Call,
	/// One accessible channel's conversation (timeline and composer). `channel_id` is a
	/// snowflake string; an unknown or inaccessible channel shows nothing.
	Chat { channel_id: String },
}

/// A complete interface restatement. Every activation, panel or event result of a granted
/// plugin restates its interface: `Some` replaces the shown tree, `None` restores the normal
/// layout. `window` asks the host to show one native window beside the tree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceUi {
	pub nodes: Vec<InterfaceNode>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub window: Option<InterfaceWindow>,
}

impl InterfaceUi {
	/// Bound every row and keep action ids unique so clicks route to one row.
	pub fn validate(&self) -> Result<(), &'static str> {
		if self.nodes.len() > MAX_INTERFACE_NODES {
			return Err("interface tree exceeds 512 rows");
		}
		let mut ids = std::collections::BTreeSet::new();
		for node in &self.nodes {
			if node.id.len() > MAX_INTERFACE_ID_BYTES {
				return Err("interface row id exceeds 64 bytes");
			}
			if !node.id.is_empty() && !ids.insert(node.id.as_str()) {
				return Err("interface row ids must be unique");
			}
			if node.text.len() > MAX_INTERFACE_TEXT_BYTES {
				return Err("interface row text exceeds 256 bytes");
			}
			if node.text.is_empty() {
				return Err("interface row text must not be empty");
			}
			if node
				.badge
				.as_ref()
				.is_some_and(|badge| badge.len() > MAX_INTERFACE_BADGE_BYTES || badge.is_empty())
			{
				return Err("interface row badge must be 1 through 32 bytes");
			}
			if node.indent > MAX_INTERFACE_INDENT {
				return Err("interface row indent exceeds 8");
			}
		}
		if let Some(InterfaceWindow::Chat { channel_id }) = &self.window
			&& (channel_id.is_empty()
				|| channel_id.len() > 20
				|| !channel_id.bytes().all(|b| b.is_ascii_digit()))
		{
			return Err("interface chat window needs a snowflake channel id");
		}
		Ok(())
	}
}
