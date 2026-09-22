//! Device-local GPU selection; independent of Discord accounts.

/// Which GPU Serein renders on, mirroring the three choices desktop platforms already offer.
///
/// A preference only orders the adapters that can actually present to the window, so it can
/// never select a device the compositor refuses to read from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuPreference {
	/// Serein picks: presentable adapters first, then the discrete GPU that usually drives
	/// the display. Honors `WGPU_POWER_PREF` for diagnostics.
	#[default]
	Automatic,
	/// Prefer the discrete GPU even when an integrated one could present.
	HighPerformance,
	/// Prefer the integrated GPU to save battery and fan noise on laptops.
	PowerSaving,
}
impl GpuPreference {
	pub const ALL: [Self; 3] = [Self::Automatic, Self::HighPerformance, Self::PowerSaving];
	pub fn label(self) -> &'static str {
		match self {
			Self::Automatic => "Automatic",
			Self::HighPerformance => "High performance",
			Self::PowerSaving => "Power saving",
		}
	}
	pub fn description(self) -> &'static str {
		match self {
			Self::Automatic => "Let Serein choose the GPU that can draw this window.",
			Self::HighPerformance => "Use the discrete graphics card when one is available.",
			Self::PowerSaving => "Use integrated graphics to save battery.",
		}
	}
	/// Stable storage key; unknown keys from a newer build fall back to [`Self::Automatic`].
	fn key(self) -> &'static str {
		match self {
			Self::Automatic => "automatic",
			Self::HighPerformance => "high-performance",
			Self::PowerSaving => "power-saving",
		}
	}
}
impl From<String> for GpuPreference {
	fn from(value: String) -> Self {
		Self::ALL
			.into_iter()
			.find(|candidate| candidate.key() == value)
			.unwrap_or_default()
	}
}
impl From<GpuPreference> for String {
	fn from(value: GpuPreference) -> Self {
		value.key().to_owned()
	}
}
impl serde::Serialize for GpuPreference {
	fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.serialize_str(self.key())
	}
}
impl<'de> serde::Deserialize<'de> for GpuPreference {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		// Tolerate a value written by a newer build rather than rejecting every preference.
		Ok(Self::from(String::deserialize(deserializer)?))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn round_trips_through_storage_keys() {
		for preference in GpuPreference::ALL {
			let key = String::from(preference);
			assert_eq!(GpuPreference::from(key), preference);
		}
	}

	#[test]
	fn unknown_and_default_fall_back_to_automatic() {
		assert_eq!(GpuPreference::default(), GpuPreference::Automatic);
		assert_eq!(
			GpuPreference::from("quantum-gpu".to_owned()),
			GpuPreference::Automatic
		);
		assert_eq!(GpuPreference::from(String::new()), GpuPreference::Automatic);
	}
}
