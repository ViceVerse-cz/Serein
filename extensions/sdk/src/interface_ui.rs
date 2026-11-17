//! Declarative, capability-gated replacement for the client's main panes.
//!
//! A plugin with the `interface_ui` grant returns a bounded flat tree of rows; the host
//! renders it in place of the navigation, conversation and member panes and routes clicks
//! on rows back to the plugin as actions. The schema knows nothing about servers, voice or
//! any particular product: indent, icon, tint, badge and emphasis are generic primitives.

use serde::{Deserialize, Serialize};

/// Maximum rows in one interface tree.
pub const MAX_INTERFACE_NODES: usize = 512;
/// Maximum UTF-8 bytes of one row label.
pub const MAX_INTERFACE_TEXT_BYTES: usize = 256;
/// The declared manifest action the host invokes for interface row and dock clicks; the
/// clicked item's id arrives in `Invocation.values["id"]`. `Invocation.double_clicked` is
/// set for a double click and for Enter on a focused item, the primary open action (as in
/// file managers); a single click or Space on a focused item leaves it false. Plugins
/// without this declared action get inert rows.
pub const INTERFACE_CLICK_ACTION: &str = "interface_click";
/// Maximum UTF-8 bytes of one row badge.
pub const MAX_INTERFACE_BADGE_BYTES: usize = 32;
/// Maximum UTF-8 bytes of one row action id.
pub const MAX_INTERFACE_ID_BYTES: usize = 64;
/// Maximum indent step of a row.
pub const MAX_INTERFACE_INDENT: u8 = 8;
/// Maximum items in the bottom control row's dock.
pub const MAX_INTERFACE_DOCK_ITEMS: usize = 64;

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

fn is_false(value: &bool) -> bool {
	!value
}

/// One row. `id` is the action the host invokes when the row is clicked; an empty id makes
/// the row inert. `double_clicked` on the resulting invocation marks a double click or Enter
/// on the focused row (the open action); Space counts as a single click.
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

/// A complete interface restatement. Every activation, panel or event result of a granted
/// plugin restates its interface: `Some` replaces the shown tree, `None` restores the normal
/// layout. `window` asks the host to show one native window beside the tree; `dock` adds
/// small items to the left of the bottom control row, and `add_server` appends the
/// client's own add-a-server button after them (handled natively, never routed to Wasm).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceUi {
	pub nodes: Vec<InterfaceNode>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub window: Option<InterfaceWindow>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub dock: Vec<InterfaceDockItem>,
	#[serde(default, skip_serializing_if = "is_false")]
	pub add_server: bool,
}

fn snowflake(id: &str) -> bool {
	(1..=20).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
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
			if item.label.is_empty() || item.label.len() > MAX_INTERFACE_TEXT_BYTES {
				return Err("interface dock label must be 1 through 256 bytes");
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

/// Opt-in wrapper preserving original SDK `Output` struct literals and compiled plugins.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceUiOutput {
	#[serde(flatten)]
	pub output: crate::Output,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub interface_ui: Option<InterfaceUi>,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn node(id: &str, text: &str) -> InterfaceNode {
		InterfaceNode {
			id: id.into(),
			indent: 0,
			icon: None,
			text: text.into(),
			badge: None,
			emphasis: false,
			selected: false,
			tint: None,
		}
	}

	fn dock(id: &str, label: &str, guild_id: Option<&str>) -> InterfaceDockItem {
		InterfaceDockItem {
			id: id.into(),
			label: label.into(),
			guild_id: guild_id.map(str::to_owned),
			selected: false,
		}
	}

	#[test]
	fn bounds_and_unique_ids_are_enforced() {
		let mut ui = InterfaceUi {
			nodes: vec![node("a", "one"), node("a", "two")],
			window: None,
			dock: Vec::new(),
			add_server: false,
		};
		assert!(ui.validate().is_err(), "duplicate ids rejected");
		ui.nodes[1].id = "b".into();
		assert!(ui.validate().is_ok());
		ui.nodes[1].text = "x".repeat(MAX_INTERFACE_TEXT_BYTES + 1);
		assert!(ui.validate().is_err(), "oversized text rejected");
		ui.nodes[1].text = "ok".into();
		ui.nodes[1].indent = MAX_INTERFACE_INDENT + 1;
		assert!(ui.validate().is_err(), "oversized indent rejected");
		ui.nodes[1].indent = 2;
		ui.nodes
			.extend((0..MAX_INTERFACE_NODES).map(|i| node(&format!("n{i}"), "row")));
		assert!(ui.validate().is_err(), "oversized tree rejected");
		ui.nodes.truncate(2);
		ui.window = Some(InterfaceWindow::Chat {
			channel_id: "../x".into(),
		});
		assert!(ui.validate().is_err(), "non-snowflake chat id rejected");
		ui.window = Some(InterfaceWindow::Chat {
			channel_id: "20".into(),
		});
		assert!(ui.validate().is_ok());
		ui.dock = vec![dock("dock:a", "Alpha", Some("10"))];
		assert!(ui.validate().is_ok(), "valid dock item accepted");
		ui.dock.push(dock("", "Empty", None));
		assert!(ui.validate().is_err(), "empty dock id rejected");
		ui.dock[1].id = "a".into();
		assert!(
			ui.validate().is_err(),
			"dock id duplicating a row id rejected"
		);
		ui.dock[1].id = "dock:b".into();
		ui.dock.push(dock("dock:b", "Duplicate", None));
		assert!(ui.validate().is_err(), "duplicate dock id rejected");
		ui.dock.pop();
		ui.dock[1].label = String::new();
		assert!(ui.validate().is_err(), "empty dock label rejected");
		ui.dock[1].label = "Beta".into();
		ui.dock[1].guild_id = Some("../guild".into());
		assert!(
			ui.validate().is_err(),
			"non-snowflake dock guild id rejected"
		);
		ui.dock[1].guild_id = None;
		assert!(ui.validate().is_ok());
		ui.dock
			.extend((0..MAX_INTERFACE_DOCK_ITEMS).map(|i| dock(&format!("dock:{i}"), "Row", None)));
		assert!(ui.validate().is_err(), "oversized dock rejected");
		ui.dock.clear();
		assert!(ui.validate().is_ok());
	}

	#[test]
	fn wire_shape_is_snake_case_and_optional_fields_absent() {
		let ui = InterfaceUi {
			nodes: vec![InterfaceNode {
				icon: Some(InterfaceIcon::MicrophoneSlash),
				tint: Some(InterfaceTint::Danger),
				badge: Some("You".into()),
				..node("channel:1", "Lobby")
			}],
			window: Some(InterfaceWindow::Chat {
				channel_id: "20".into(),
			}),
			dock: vec![dock("dock:1", "Voice club", Some("10"))],
			add_server: true,
		};
		let json = serde_json::to_value(&ui).unwrap();
		assert_eq!(json["nodes"][0]["icon"], "microphone_slash");
		assert_eq!(json["nodes"][0]["tint"], "danger");
		assert_eq!(json["window"]["kind"], "chat");
		assert_eq!(json["window"]["channel_id"], "20");
		assert_eq!(json["dock"][0]["id"], "dock:1");
		assert_eq!(json["dock"][0]["guild_id"], "10");
		assert!(json["dock"][0].get("selected").is_none());
		assert_eq!(json["add_server"], true);
		let call = serde_json::to_value(InterfaceWindow::Call).unwrap();
		assert_eq!(call, serde_json::json!({"kind": "call"}));
		let plain = serde_json::to_value(InterfaceUi::default()).unwrap();
		assert!(plain.get("window").is_none());
		assert!(plain.get("dock").is_none());
		assert!(plain.get("add_server").is_none());
		let plain_node = serde_json::to_value(node("", "row")).unwrap();
		assert!(plain_node.get("icon").is_none());
		assert!(plain_node.get("badge").is_none());
		assert!(plain_node.get("emphasis").is_none());
		assert_eq!(plain_node["id"], "");
		let wrapper = InterfaceUiOutput {
			interface_ui: Some(ui),
			..Default::default()
		};
		let json = serde_json::to_value(&wrapper).unwrap();
		assert!(json.get("interface_ui").is_some());
		assert!(json.get("panel").is_some(), "Output fields stay flattened");
	}
}
