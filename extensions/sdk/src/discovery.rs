use serde::{Deserialize, Serialize};
/// Nonprivate host support metadata. Absent on hosts predating discovery.
/// Supported capability names do not imply user consent. Unknown future names are retained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
	pub api_version: u32,
	pub sdk_revision: u32,
	pub capabilities: Vec<String>,
	pub app_events: Vec<String>,
	/// Interface tweak names this host honors; empty on hosts predating tweaks.
	#[serde(default)]
	pub interface_tweaks: Vec<String>,
}
impl HostInfo {
	pub fn supports(&self, capability: &str) -> bool {
		self.capabilities.iter().any(|name| name == capability)
	}
	pub fn supports_event(&self, event: &str) -> bool {
		self.app_events.iter().any(|name| name == event)
	}
	/// Whether the host honors the named interface tweak, such as `"expanded_roles"`.
	pub fn supports_tweak(&self, tweak: &str) -> bool {
		self.interface_tweaks.iter().any(|name| name == tweak)
	}
}
