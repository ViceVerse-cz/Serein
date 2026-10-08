use crate::Error;
use serde::{Deserialize, Serialize};

/// Host-owned voice recorder preferences. This grants no capture API to Wasm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceMessagesConfig {
	pub max_duration_seconds: u16,
	pub waveform_style: WaveformStyle,
	pub noise_suppression: bool,
}

impl Default for VoiceMessagesConfig {
	fn default() -> Self {
		Self {
			max_duration_seconds: 120,
			waveform_style: WaveformStyle::Bars,
			noise_suppression: true,
		}
	}
}

impl VoiceMessagesConfig {
	pub fn validate(&self) -> Result<(), Error> {
		if !(5..=120).contains(&self.max_duration_seconds) {
			return Err(Error::Invalid);
		}
		Ok(())
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveformStyle {
	#[default]
	Bars,
	Line,
}
