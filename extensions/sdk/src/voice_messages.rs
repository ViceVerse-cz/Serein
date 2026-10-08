use crate::Output;
use serde::{Deserialize, Serialize};

/// Native recorder preferences; the sandbox receives no microphone or audio access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceMessagesConfig {
	/// Recording ceiling in seconds, inclusive range 5 through 120.
	pub max_duration_seconds: u16,
	pub waveform_style: WaveformStyle,
	/// Apply the host's noise suppression to this recorder only.
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
	pub fn validate(&self) -> Result<(), &'static str> {
		if !(5..=120).contains(&self.max_duration_seconds) {
			return Err("Maximum recording duration must be between 5 and 120 seconds.");
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

/// Opt-in result wrapper preserving the original `Output` struct literal API.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceMessagesOutput {
	#[serde(flatten)]
	pub output: Output,
	/// Accepted only from granted panel/activation actions; omission preserves configuration.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub voice_messages: Option<VoiceMessagesConfig>,
}
