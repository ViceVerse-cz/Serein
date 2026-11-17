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
/// The declared manifest action the host invokes for interface row clicks; the clicked row
/// id arrives in `Invocation.values["id"]`. Plugins without this declared action get inert rows.
pub const INTERFACE_CLICK_ACTION: &str = "interface_click";
/// Maximum UTF-8 bytes of one row badge.
pub const MAX_INTERFACE_BADGE_BYTES: usize = 32;
/// Maximum UTF-8 bytes of one row action id.
pub const MAX_INTERFACE_ID_BYTES: usize = 64;
/// Maximum indent step of a row.
pub const MAX_INTERFACE_INDENT: u8 = 8;

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

	#[test]
	fn bounds_and_unique_ids_are_enforced() {
		let mut ui = InterfaceUi {
			nodes: vec![node("a", "one"), node("a", "two")],
			window: None,
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
		};
		let json = serde_json::to_value(&ui).unwrap();
		assert_eq!(json["nodes"][0]["icon"], "microphone_slash");
		assert_eq!(json["nodes"][0]["tint"], "danger");
		assert_eq!(json["window"]["kind"], "chat");
		assert_eq!(json["window"]["channel_id"], "20");
		let call = serde_json::to_value(InterfaceWindow::Call).unwrap();
		assert_eq!(call, serde_json::json!({"kind": "call"}));
		let plain = serde_json::to_value(InterfaceUi::default()).unwrap();
		assert!(plain.get("window").is_none());
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
