//! Device-local GPU selection; independent of Discord accounts.

/// The physical adapter used by the running renderer. This value is session-only:
/// the saved preference orders adapters on the next launch, not during an active call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoAdapter {
	pub vendor_id: u32,
	pub device_id: u32,
	pub identity: VideoAdapterIdentity,
}

/// Native identities remain distinct even for two cards with the same vendor/model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VideoAdapterIdentity {
	/// Hardware encoders must not guess another adapter when identification fails.
	#[default]
	Unidentified,
	WindowsLuid(u64),
	Pci {
		domain: u32,
		bus: u8,
		device: u8,
		function: u8,
	},
	MetalRegistry(u64),
}

impl VideoAdapter {
	/// Bounded process argument for a discovery/test helper; never a saved preference.
	pub fn helper_key(self) -> String {
		let prefix = format!("{:08x}:{:08x}", self.vendor_id, self.device_id);
		match self.identity {
			VideoAdapterIdentity::Unidentified => format!("{prefix}:unknown"),
			VideoAdapterIdentity::WindowsLuid(id) => format!("{prefix}:luid:{id:016x}"),
			VideoAdapterIdentity::MetalRegistry(id) => format!("{prefix}:metal:{id:016x}"),
			VideoAdapterIdentity::Pci {
				domain,
				bus,
				device,
				function,
			} => format!("{prefix}:pci:{domain:08x}:{bus:02x}:{device:02x}:{function:x}"),
		}
	}

	pub fn from_helper_key(key: &str) -> Option<Self> {
		if key.len() > 64 || !key.is_ascii() {
			return None;
		}
		let mut fields = key.split(':');
		let hex = |field: &str, digits: usize| {
			(field.len() == digits && field.bytes().all(|byte| byte.is_ascii_hexdigit()))
				.then(|| u64::from_str_radix(field, 16).ok())
				.flatten()
		};
		let vendor_id = hex(fields.next()?, 8)? as u32;
		let device_id = hex(fields.next()?, 8)? as u32;
		let identity = match fields.next()? {
			"unknown" => VideoAdapterIdentity::Unidentified,
			"luid" => VideoAdapterIdentity::WindowsLuid(hex(fields.next()?, 16)?),
			"metal" => VideoAdapterIdentity::MetalRegistry(hex(fields.next()?, 16)?),
			"pci" => {
				let domain = hex(fields.next()?, 8)? as u32;
				let bus = hex(fields.next()?, 2)? as u8;
				let device = hex(fields.next()?, 2)? as u8;
				let function = hex(fields.next()?, 1)? as u8;
				if device > 31 || function > 7 {
					return None;
				}
				VideoAdapterIdentity::Pci {
					domain,
					bus,
					device,
					function,
				}
			}
			_ => return None,
		};
		fields.next().is_none().then_some(Self {
			vendor_id,
			device_id,
			identity,
		})
	}
}

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
	fn helper_adapter_keys_preserve_the_exact_device_and_reject_invalid_identity() {
		for identity in [
			VideoAdapterIdentity::Unidentified,
			VideoAdapterIdentity::WindowsLuid(0x8000_0000_0123_4567),
			VideoAdapterIdentity::MetalRegistry(0x1_0000_0321),
			VideoAdapterIdentity::Pci {
				domain: 0x10000,
				bus: 0xff,
				device: 31,
				function: 7,
			},
		] {
			let adapter = VideoAdapter {
				vendor_id: 0x10de,
				device_id: 0x2684,
				identity,
			};
			assert_eq!(
				VideoAdapter::from_helper_key(&adapter.helper_key()),
				Some(adapter)
			);
		}
		for key in [
			"000010de:00002684:unknown:extra",
			"000010de:00002684:pci:00000000:01:20:0",
			"000010de:00002684:pci:00000000:01:00:8",
			"000010de:00002684:luid:123",
			"000010de:00002684:other:0000000000000001",
			"000010de:00002684:luid:0000000000000é1",
		] {
			assert_eq!(VideoAdapter::from_helper_key(key), None);
		}
		assert_eq!(VideoAdapter::from_helper_key(&"x".repeat(65)), None);
	}

	#[test]
	fn round_trips_through_storage_keys() {
		{
			for preference in GpuPreference::ALL {
				let key = String::from(preference);
				assert_eq!(GpuPreference::from(key), preference);
			}
		}
		{
			assert_eq!(GpuPreference::default(), GpuPreference::Automatic);
			assert_eq!(
				GpuPreference::from("quantum-gpu".to_owned()),
				GpuPreference::Automatic
			);
			assert_eq!(GpuPreference::from(String::new()), GpuPreference::Automatic);
		}
	}
}
