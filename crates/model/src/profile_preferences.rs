//! Device-local profile presentation choices. These never change what the service shares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProfilePreferences {
	/// Open the full profile instead of the anchored popout from existing profile surfaces.
	pub always_expand: bool,
	/// Hide other users' loaded profile theme colours; the account's own theme stays visible.
	pub hide_other_themes: bool,
	/// Show every loaded role chip in profiles instead of collapsing after two rows.
	pub expand_role_chips: bool,
	/// Show the cached desktop/mobile/web/VR client flags beside a user.
	pub platform_indicators: bool,
	/// Paint already-loaded profile banners behind member rows.
	pub row_banners: bool,
}
