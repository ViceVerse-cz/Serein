//! Device-local microphone processing. Profiles leave the user's custom settings intact.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputProfile {
	#[default]
	VoiceIsolation,
	Studio,
	Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseSuppression {
	/// Prefer DeepFilterNet when a bounded worker probe finds sufficient processing headroom.
	#[default]
	Auto,
	DeepFilterNet,
	Off,
	RnNoise,
	WebRtc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Processing {
	pub suppression: NoiseSuppression,
	/// Strength from low (0) through very high (3); DeepFilterNet limits attenuation
	/// to 6/12/24/100 dB, while WebRTC uses its four native levels.
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn voice_isolation_defaults_to_auto_and_preserves_custom_choices() {
		let mut settings = VoiceProcessing::default();
		assert_eq!(NoiseSuppression::default(), NoiseSuppression::Auto);
		assert_eq!(settings.effective().suppression, NoiseSuppression::Auto);
		assert_eq!(settings.effective().suppression_level, 2);
		settings.edit().suppression = NoiseSuppression::DeepFilterNet;
		settings.custom.suppression_level = 1;
		let custom = settings.custom;
		settings.profile = InputProfile::Studio;
		assert_eq!(settings.effective(), Processing::studio());
		settings.profile = InputProfile::VoiceIsolation;
		assert_eq!(settings.effective(), Processing::default());
		assert_eq!(settings.custom, custom);
		settings.profile = InputProfile::Custom;
		assert_eq!(settings.effective(), custom);
		settings.profile = InputProfile::VoiceIsolation;
		settings.edit().sensitivity_db = Some(-40);
		assert_eq!(settings.custom.suppression, NoiseSuppression::Auto);
		assert_eq!(settings.custom.suppression_level, 2);
	}

	#[test]
	fn voice_processing_legacy_migration_preserves_explicit_suppression() {
		for (enabled, expected) in [
			(true, NoiseSuppression::RnNoise),
			(false, NoiseSuppression::Off),
		] {
			let settings = VoiceProcessing::from_legacy(enabled);
			assert_eq!(settings.profile, InputProfile::Custom);
			assert_eq!(settings.effective().suppression, expected);
			assert_eq!(settings.effective().sensitivity_db, None);
			assert!(!settings.effective().automatic_gain);
			assert!(settings.effective().echo_cancellation);
		}
	}
}
