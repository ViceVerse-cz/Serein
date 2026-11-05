use serde::{Deserialize, Serialize};

/// Device-local chat-media masking controlled by a media-privacy plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaPrivacyPreferences {
	pub enabled: bool,
	pub reveal_on_hover: bool,
	pub style: MediaPrivacyStyle,
	/// Blur radius for `blur`, or mask opacity for `solid`, from 0 to 100.
	pub strength: u8,
	/// Opacity of the mask, from 0 to 100.
	pub opacity: u8,
	/// RGB tint used by `solid`.
	pub color: [u8; 3],
}

impl Default for MediaPrivacyPreferences {
	fn default() -> Self {
		Self {
			enabled: true,
			reveal_on_hover: true,
			style: MediaPrivacyStyle::Blur,
			strength: 55,
			opacity: 100,
			color: [0, 0, 0],
		}
	}
}

impl MediaPrivacyPreferences {
	pub fn is_valid(self) -> bool {
		self.strength <= 100 && self.opacity <= 100
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaPrivacyStyle {
	#[default]
	Blur,
	Solid,
}
