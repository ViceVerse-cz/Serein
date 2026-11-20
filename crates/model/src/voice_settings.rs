//! Device-local microphone processing. Profiles leave the user's custom settings intact.
use serde::{Deserialize, Serialize};

/// Session-local frame-rate target for native camera capture and encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraFrameRate {
	Fps15,
	#[default]
	Fps30,
}

impl CameraFrameRate {
	/// Return the selected target in frames per second.
	pub const fn fps(self) -> u32 {
		match self {
			Self::Fps15 => 15,
			Self::Fps30 => 30,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputProfile {
	#[default]
	VoiceIsolation,
	Studio,
	Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseSuppression {
	Off,
	RnNoise,
	WebRtc,
	DeepFilterNet,
	/// DeepFilterNet while this device keeps up with it, RNNoise once it does not.
	#[default]
	Auto,
}
impl NoiseSuppression {
	/// How much longer than RNNoise's 10 ms this choice can hold audio back. Push-to-talk
	/// keeps transmitting this long past its release delay, so the end of a word gets out.
	pub const fn extra_delay(self) -> std::time::Duration {
		std::time::Duration::from_millis(match self {
			Self::DeepFilterNet | Self::Auto => 20,
			Self::Off | Self::RnNoise | Self::WebRtc => 0,
		})
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Processing {
	pub suppression: NoiseSuppression,
	/// WebRTC and DeepFilterNet suppression strength, from low (0) through very high (3).
	pub suppression_level: u8,
	pub echo_cancellation: bool,
	pub automatic_gain: bool,
	/// None is an open microphone; otherwise a dBFS threshold with a short release hold.
	pub sensitivity_db: Option<i16>,
}
impl Default for Processing {
	fn default() -> Self {
		Self {
			suppression: NoiseSuppression::Auto,
			suppression_level: 2,
			echo_cancellation: true,
			automatic_gain: true,
			sensitivity_db: Some(-55),
		}
	}
}
impl Processing {
	pub fn is_valid(self) -> bool {
		self.suppression_level <= 3 && self.sensitivity_db.is_none_or(|db| (-80..=0).contains(&db))
	}
	pub fn studio() -> Self {
		Self {
			suppression: NoiseSuppression::Off,
			suppression_level: 0,
			echo_cancellation: false,
			automatic_gain: false,
			sensitivity_db: None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceProcessing {
	pub profile: InputProfile,
	pub custom: Processing,
}
impl VoiceProcessing {
	pub fn effective(self) -> Processing {
		match self.profile {
			InputProfile::VoiceIsolation => Processing::default(),
			InputProfile::Studio => Processing::studio(),
			InputProfile::Custom => self.custom,
		}
	}
	pub fn from_legacy(noise_suppression: bool) -> Self {
		Self {
			profile: InputProfile::Custom,
			custom: Processing {
				suppression: if noise_suppression {
					NoiseSuppression::RnNoise
				} else {
					NoiseSuppression::Off
				},
				echo_cancellation: true,
				..Processing::studio()
			},
		}
	}
	/// Editing a preset starts from its visible values, rather than hidden custom values.
	pub fn edit(&mut self) -> &mut Processing {
		if self.profile != InputProfile::Custom {
			self.custom = self.effective();
		}
		self.profile = InputProfile::Custom;
		&mut self.custom
	}
}
