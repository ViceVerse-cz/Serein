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
/// Maximum items in the bottom control row's dock.
pub const MAX_INTERFACE_DOCK_ITEMS: usize = 64;

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
	/// A native action the host runs itself on a double click or Enter, without an Apply
	/// step. The host labels such a row with the target's real name (the plugin's `text` is
	/// ignored), checks access and runs the client's own path. The click still reaches the
	/// plugin's `interface_click` so it can restate its interface.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub native: Option<InterfaceNative>,
}

/// Native actions a row can carry; each needs its own capability grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InterfaceNative {
	/// Join this voice channel with the current mute and deafen state, through the client's
	/// own join path including its call-switch confirmation. Requires `voice_connect`.
	JoinVoice { channel_id: String },
}

/// The window size an interface layout asks for, in logical pixels. The host centers the
/// window on its monitor (clamped to the monitor) and restores the previous size, position
/// and maximized state when the layout ends or stops asking. It never saves this size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceSize {
	pub width: u16,
	pub height: u16,
}

/// Smallest window side an interface layout may ask for, in logical pixels.
pub const MIN_INTERFACE_WINDOW_SIDE: u16 = 200;
/// Largest window width an interface layout may ask for, in logical pixels.
pub const MAX_INTERFACE_WINDOW_WIDTH: u16 = 1600;
/// Largest window height an interface layout may ask for, in logical pixels.
pub const MAX_INTERFACE_WINDOW_HEIGHT: u16 = 1200;

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

/// One small clickable item in the dock at the left of the tree pane's bottom control row.
/// `id` routes clicks exactly like a row id (unique across rows and dock items). With
/// `guild_id` the host paints that loaded server's icon (or its initials); `label` is the
/// hover text and the initials source. `selected` marks the item as current.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceDockItem {
	pub id: String,
	pub label: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub guild_id: Option<String>,
	#[serde(default, skip_serializing_if = "is_false")]
	pub selected: bool,
}

/// A native control the plugin places at the right of the bottom control row. The host
/// draws and handles each one exactly as its account card does; nothing reaches Wasm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterfaceControl {
	/// Leave the current call; shown only during a call.
	Disconnect,
	/// Toggle your microphone.
	Mute,
	/// Toggle your headphones.
	Deafen,
	/// Open the Friends / Direct Messages list in place of the window.
	Friends,
	/// Open the client's settings.
	Settings,
}

/// A complete interface restatement. Every activation, panel or event result of a granted
/// plugin restates its interface: `Some` replaces the shown tree, `None` restores the normal
/// layout. `window` asks the host to show one native window beside the tree; `dock` adds
/// small items to the left of the bottom control row, and `add_server` appends the
/// client's own add-a-server button after them (handled natively, never routed to Wasm).
/// `controls` lists the native buttons at the right of that row, left to right; the host
/// always adds `settings` when it is missing, so the user can still reach the extension
/// settings to turn the plugin off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceUi {
	pub nodes: Vec<InterfaceNode>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub window: Option<InterfaceWindow>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub dock: Vec<InterfaceDockItem>,
	#[serde(default, skip_serializing_if = "is_false")]
	pub add_server: bool,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub controls: Vec<InterfaceControl>,
	/// Resize the window while this layout shows; omitted keeps the current window.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub size: Option<InterfaceSize>,
}

fn snowflake(id: &str) -> bool {
	(1..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

/// Row text, badges and dock labels must carry visible characters and no control codes.
fn printable(value: &str) -> bool {
	!value.chars().any(char::is_control) && !value.trim().is_empty()
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
			if !printable(&node.text) {
				return Err("interface row text must be printable and not blank");
			}
			if node.badge.as_ref().is_some_and(|badge| {
				badge.len() > MAX_INTERFACE_BADGE_BYTES || badge.is_empty() || !printable(badge)
			}) {
				return Err("interface row badge must be 1 through 32 printable bytes");
			}
			if node.indent > MAX_INTERFACE_INDENT {
				return Err("interface row indent exceeds 8");
			}
			if let Some(InterfaceNative::JoinVoice { channel_id }) = &node.native
				&& !snowflake(channel_id)
			{
				return Err("interface native join needs a snowflake channel id");
			}
		}
		for (index, control) in self.controls.iter().enumerate() {
			if self.controls[..index].contains(control) {
				return Err("interface controls must not repeat");
			}
		}
		if self.size.is_some_and(|size| {
			!(MIN_INTERFACE_WINDOW_SIDE..=MAX_INTERFACE_WINDOW_WIDTH).contains(&size.width)
				|| !(MIN_INTERFACE_WINDOW_SIDE..=MAX_INTERFACE_WINDOW_HEIGHT).contains(&size.height)
		}) {
			return Err("interface window size must be 200 through 1600 by 200 through 1200");
		}
		if self.dock.len() > MAX_INTERFACE_DOCK_ITEMS {
			return Err("interface dock exceeds 64 items");
		}
		for item in &self.dock {
			if item.id.is_empty() || item.id.len() > MAX_INTERFACE_ID_BYTES {
				return Err("interface dock id must be 1 through 64 bytes");
			}
			if !ids.insert(item.id.as_str()) {
				return Err("interface row ids must be unique");
			}
			if item.label.is_empty()
				|| item.label.len() > MAX_INTERFACE_TEXT_BYTES
				|| !printable(&item.label)
			{
				return Err("interface dock label must be 1 through 256 printable bytes");
			}
			if item.guild_id.as_deref().is_some_and(|id| !snowflake(id)) {
				return Err("interface dock guild id must be a snowflake");
			}
		}
		if let Some(InterfaceWindow::Chat { channel_id }) = &self.window
			&& !snowflake(channel_id)
		{
			return Err("interface chat window needs a snowflake channel id");
		}
		Ok(())
	}
}
