//! Device-local voice and video settings. Profiles preserve custom microphone settings.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
	#[default]
	H264,
	H265,
	Av1,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoBandwidth {
	Low,
	#[default]
	Medium,
	High,
}

impl VideoBandwidth {
	pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

	pub fn rates(self, width: u32, height: u32, fps: u32) -> VideoBitrate {
		let medium = VideoBitrate::for_dimensions(width, height);
		match self {
			Self::Medium => medium,
			Self::Low => VideoBitrate {
				average: (medium.average / 2).max(VideoBitrate::MINIMUM),
				maximum: (medium.maximum / 2).max(VideoBitrate::MINIMUM),
				..medium
			},
			Self::High => {
				let pixels = (u64::from(width) * u64::from(height)).min(7680 * 4320);
				let points = [
					(854u64 * 480, 2_000_000u64),
					(1280 * 720, 4_000_000),
					(1920 * 1080, 8_000_000),
					(2560 * 1440, 12_000_000),
					(3840 * 2160, 20_000_000),
					(7680 * 4320, 40_000_000),
				];
				let mut average = points[0].1;
				for [(low, rate), (high, high_rate)] in points.windows(2).map(|p| [p[0], p[1]]) {
					if pixels <= low {
						break;
					}
					average = rate + (pixels.min(high) - low) * (high_rate - rate) / (high - low);
					if pixels <= high {
						break;
					}
				}
				let average = (average * if fps == 60 { 2 } else { 1 }).min(50_000_000) as u32;
				let average = average.max(medium.average);
				VideoBitrate {
					average,
					maximum: average.max(medium.maximum),
					..medium
				}
			}
		}
	}
}

/// Stable IDs shared with the native encoder/query shim. Automatic retains the
/// vendor-specific defaults; explicit choices are filtered by driver and codec.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum EncoderPreset {
	#[default]
	Automatic = 0,
	NvencP1,
	NvencP2,
	NvencP3,
	NvencP4,
	NvencP5,
	NvencP6,
	NvencP7,
	AmfSpeed,
	AmfBalanced,
	AmfQuality,
	AmfHighQuality,
	QsvVerySlow,
	QsvSlower,
	QsvSlow,
	QsvMedium,
	QsvFast,
	QsvFaster,
	QsvVeryFast,
}

impl EncoderPreset {
	pub const ALL: [Self; 19] = [
		Self::Automatic,
		Self::NvencP1,
		Self::NvencP2,
		Self::NvencP3,
		Self::NvencP4,
		Self::NvencP5,
		Self::NvencP6,
		Self::NvencP7,
		Self::AmfSpeed,
		Self::AmfBalanced,
		Self::AmfQuality,
		Self::AmfHighQuality,
		Self::QsvVerySlow,
		Self::QsvSlower,
		Self::QsvSlow,
		Self::QsvMedium,
		Self::QsvFast,
		Self::QsvFaster,
		Self::QsvVeryFast,
	];

	pub fn backend(self) -> Option<HardwareBackend> {
		match self as i32 {
			1..=7 => Some(HardwareBackend::Nvenc),
			8..=11 => Some(HardwareBackend::Amf),
			12..=18 => Some(HardwareBackend::Qsv),
			_ => None,
		}
	}

	pub fn label(self) -> &'static str {
		match self {
			Self::Automatic => "Automatic",
			Self::NvencP1 => "P1 — fastest",
			Self::NvencP2 => "P2",
			Self::NvencP3 => "P3",
			Self::NvencP4 => "P4",
			Self::NvencP5 => "P5",
			Self::NvencP6 => "P6",
			Self::NvencP7 => "P7 — highest quality",
			Self::AmfSpeed => "Speed",
			Self::AmfBalanced => "Balanced",
			Self::AmfQuality => "Quality",
			Self::AmfHighQuality => "High quality",
			Self::QsvVerySlow => "Very slow",
			Self::QsvSlower => "Slower",
			Self::QsvSlow => "Slow",
			Self::QsvMedium => "Medium",
			Self::QsvFast => "Fast",
			Self::QsvFaster => "Faster",
			Self::QsvVeryFast => "Very fast",
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncoderPresets(u32);
impl EncoderPresets {
	pub fn from_bits(bits: u32) -> Self {
		Self(bits & 0x7fffe)
	}
	pub fn bits(self) -> u32 {
		self.0
	}
	pub fn contains(self, preset: EncoderPreset) -> bool {
		preset == EncoderPreset::Automatic || self.0 & (1 << preset as u32) != 0
	}
	pub fn for_backend(self, backend: HardwareBackend) -> Self {
		Self(Self::all_for_backend(backend).0 & self.0)
	}
	fn all_for_backend(backend: HardwareBackend) -> Self {
		Self(
			EncoderPreset::ALL
				.into_iter()
				.filter(|preset| preset.backend() == Some(backend))
				.fold(0, |bits, preset| bits | 1 << preset as u32),
		)
	}
}

/// Shared bandwidth budget for every codec and frame rate, in bits per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoBitrate {
	pub minimum: u32,
	pub average: u32,
	pub maximum: u32,
}

impl VideoBitrate {
	pub const MINIMUM: u32 = 200_000;

	/// The short edge follows screen presets in either orientation. Custom Source
	/// sizes interpolate between those budgets without increasing ultrawide rates.
	pub fn for_dimensions(width: u32, height: u32) -> Self {
		let edge = width.min(height).min(4320);
		let points = [
			(480, 600_000, 1_000_000),
			(720, 2_000_000, 3_000_000),
			(1080, 6_000_000, 8_000_000),
			(1440, 10_000_000, 14_000_000),
			(2160, 14_000_000, 16_000_000),
			(4320, 24_000_000, 28_000_000),
		];
		let mut rates = Self {
			minimum: Self::MINIMUM,
			average: points[0].1,
			maximum: points[0].2,
		};
		for [(low, average, maximum), (high, high_average, high_maximum)] in
			points.windows(2).map(|points| [points[0], points[1]])
		{
			if edge <= low {
				break;
			}
			let portion = u64::from(edge.min(high) - low);
			let span = u64::from(high - low);
			rates.average = average + (u64::from(high_average - average) * portion / span) as u32;
			rates.maximum = maximum + (u64::from(high_maximum - maximum) * portion / span) as u32;
			if edge <= high {
				break;
			}
		}
		rates
	}

	/// Feedback is a peak ceiling; do not exceed it to retain the average target.
	pub fn limited_to(self, ceiling: u32) -> Self {
		let maximum = ceiling.clamp(self.minimum, self.maximum);
		Self {
			average: self.average.min(maximum),
			maximum,
			..self
		}
	}

	pub const fn fixed(rate: u32) -> Self {
		Self {
			minimum: rate,
			average: rate,
			maximum: rate,
		}
	}

	pub fn is_valid(self) -> bool {
		self.minimum >= 1_000
			&& self.minimum <= self.average
			&& self.average <= self.maximum
			&& self.maximum <= 50_000_000
	}
}

/// Output presets shared by screen sharing and the device-local camera preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoResolution {
	#[default]
	P480,
	P720,
	P1080,
	P1440,
	P2160,
	P4320,
}

impl VideoResolution {
	pub const ALL: [Self; 6] = [
		Self::P480,
		Self::P720,
		Self::P1080,
		Self::P1440,
		Self::P2160,
		Self::P4320,
	];
	pub const MAX_WIDTH: u32 = 7680;
	pub const MAX_HEIGHT: u32 = 4320;

	pub const fn dimensions(self) -> (u32, u32) {
		match self {
			Self::P480 => (854, 480),
			Self::P720 => (1280, 720),
			Self::P1080 => (1920, 1080),
			Self::P1440 => (2560, 1440),
			Self::P2160 => (3840, 2160),
			Self::P4320 => (7680, 4320),
		}
	}

	pub const fn camera_dimensions(self) -> (u32, u32) {
		match self {
			Self::P480 => (640, 480),
			_ => self.dimensions(),
		}
	}

	pub const fn label(self) -> &'static str {
		match self {
			Self::P480 => "480p",
			Self::P720 => "720p",
			Self::P1080 => "1080p",
			Self::P1440 => "1440p",
			Self::P2160 => "4K (2160p)",
			Self::P4320 => "8K (4320p)",
		}
	}
}

/// Camera output frame-rate presets; native capture negotiates the closest supported rate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoFrameRate {
	#[default]
	Fps15,
	Fps30,
	Fps60,
}

impl VideoFrameRate {
	pub const ALL: [Self; 3] = [Self::Fps15, Self::Fps30, Self::Fps60];

	pub const fn fps(self) -> u32 {
		match self {
			Self::Fps15 => 15,
			Self::Fps30 => 30,
			Self::Fps60 => 60,
		}
	}

	pub const fn label(self) -> &'static str {
		match self {
			Self::Fps15 => "15 fps",
			Self::Fps30 => "30 fps",
			Self::Fps60 => "60 fps",
		}
	}
}

impl VideoCodec {
	pub const ALL: [Self; 3] = [Self::H264, Self::H265, Self::Av1];

	pub const fn index(self) -> usize {
		match self {
			Self::H264 => 0,
			Self::H265 => 1,
			Self::Av1 => 2,
		}
	}

	pub const fn key(self) -> &'static str {
		match self {
			Self::H264 => "h264",
			Self::H265 => "h265",
			Self::Av1 => "av1",
		}
	}
}

/// Hardware-only FFmpeg engines. Capability results are session-local, never preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareBackend {
	Nvenc,
	Amf,
	Qsv,
	VideoToolbox,
}

impl HardwareBackend {
	pub const ALL: [Self; 4] = [Self::Nvenc, Self::Amf, Self::Qsv, Self::VideoToolbox];

	pub const fn index(self) -> usize {
		match self {
			Self::Nvenc => 0,
			Self::Amf => 1,
			Self::Qsv => 2,
			Self::VideoToolbox => 3,
		}
	}

	pub const fn key(self) -> &'static str {
		match self {
			Self::Nvenc => "nvenc",
			Self::Amf => "amf",
			Self::Qsv => "qsv",
			Self::VideoToolbox => "videotoolbox",
		}
	}

	pub const fn name(self) -> &'static str {
		match self {
			Self::Nvenc => "NVIDIA NVENC",
			Self::Amf => "AMD AMF",
			Self::Qsv => "Intel Quick Sync",
			Self::VideoToolbox => "Apple VideoToolbox",
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProbeResult {
	#[default]
	Pending,
	Available,
	Unavailable,
	RuntimeUnavailable,
	Failed,
	TimedOut,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HardwareSupport {
	pub camera: ProbeResult,
	pub screen: ProbeResult,
}

/// Driver-advertised codec support, queried without submitting frames to an encoder.
/// A fixed codec/backend matrix bounds session-local state independently of driver output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DriverCapabilities {
	pub support: [[ProbeResult; 4]; 3],
	/// Ten-bit profile support, independently queried; never inferred from codec presence.
	pub ten_bit: [[ProbeResult; 4]; 3],
	/// None is an incomplete/failed query; Some(empty) reports no named presets.
	pub quality_presets: [[Option<EncoderPresets>; 4]; 3],
}

impl DriverCapabilities {
	pub fn available(&self, codec: VideoCodec) -> bool {
		self.codec(codec).contains(&ProbeResult::Available)
	}

	pub fn presets(&self, codec: VideoCodec) -> EncoderPresets {
		let bits = [
			HardwareBackend::Nvenc,
			HardwareBackend::Amf,
			HardwareBackend::Qsv,
			HardwareBackend::VideoToolbox,
		]
		.into_iter()
		.filter(|backend| self.codec(codec)[backend.index()] == ProbeResult::Available)
		.fold(0, |bits, backend| {
			bits | self.quality_presets[codec.index()][backend.index()]
				.unwrap_or_default()
				.for_backend(backend)
				.bits()
		});
		EncoderPresets::from_bits(bits)
	}

	pub fn presets_ready(&self, codec: VideoCodec) -> bool {
		self.available(codec)
			&& self
				.codec(codec)
				.iter()
				.enumerate()
				.all(|(index, support)| {
					*support != ProbeResult::Available
						|| self.quality_presets[codec.index()][index].is_some()
				})
	}
	pub fn codec(&self, codec: VideoCodec) -> &[ProbeResult; 4] {
		&self.support[codec.index()]
	}

	pub fn set(&mut self, backend: HardwareBackend, codec: VideoCodec, value: ProbeResult) {
		self.support[codec.index()][backend.index()] = value;
	}

	/// A negative report requires every backend to confirm it is unavailable.
	pub fn confirmed_unavailable(&self, codec: VideoCodec) -> bool {
		self.codec(codec)
			.iter()
			.all(|result| *result == ProbeResult::Unavailable)
	}
}

/// Results of explicit synthetic encoder tests; distinct from driver-advertised support.
/// The fixed codec/backend matrix bounds state independently of driver output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoCapabilities {
	pub support: [[HardwareSupport; 4]; 3],
}

impl VideoCapabilities {
	pub fn codec(&self, codec: VideoCodec) -> &[HardwareSupport; 4] {
		&self.support[codec.index()]
	}

	pub fn set(&mut self, backend: HardwareBackend, codec: VideoCodec, value: HardwareSupport) {
		self.support[codec.index()][backend.index()] = value;
	}

	/// A complete negative test matrix; this does not override driver-reported support.
	pub fn confirmed_unavailable(&self, codec: VideoCodec) -> bool {
		self.codec(codec).iter().all(|support| {
			support.camera == ProbeResult::Unavailable && support.screen == ProbeResult::Unavailable
		})
	}
}

/// Controls additional local webcam processing, explicitly selected by the user.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraDenoising {
	#[default]
	Off,
	On,
}

/// Whether the selected rendering device can run the webcam filter on a GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraDenoisingAvailability {
	#[default]
	Unavailable,
	Checking,
	Available,
}

/// Current processing state for a running camera, distinct from the saved choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CameraDenoisingStatus {
	#[default]
	Starting,
	Off,
	Active,
	Unavailable,
	ResolutionUnsupported,
	TooSlow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct VideoSettings {
	pub codec: VideoCodec,
	pub bandwidth: VideoBandwidth,
	pub quality: EncoderPreset,
	pub camera_resolution: VideoResolution,
	pub camera_frame_rate: VideoFrameRate,
	pub camera_denoising: CameraDenoising,
}
impl<'de> Deserialize<'de> for VideoSettings {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		// Only unknown enum strings are forward-compatible; malformed types still fail.
		fn known<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
			deserializer: D,
		) -> Result<Option<T>, D::Error> {
			let value = String::deserialize(deserializer)?;
			Ok(T::deserialize(serde::de::value::StringDeserializer::<D::Error>::new(value)).ok())
		}
		#[derive(Default, Deserialize)]
		#[serde(default)]
		struct Saved {
			#[serde(deserialize_with = "known")]
			#[serde(rename = "backend")]
			_legacy_backend: Option<String>,
			#[serde(deserialize_with = "known")]
			codec: Option<VideoCodec>,
			#[serde(deserialize_with = "known")]
			bandwidth: Option<VideoBandwidth>,
			#[serde(deserialize_with = "known")]
			quality: Option<EncoderPreset>,
			#[serde(deserialize_with = "known")]
			camera_resolution: Option<VideoResolution>,
			#[serde(deserialize_with = "known")]
			camera_frame_rate: Option<VideoFrameRate>,
			#[serde(deserialize_with = "known")]
			camera_denoising: Option<CameraDenoising>,
		}
		let saved = Saved::deserialize(deserializer)?;
		Ok(Self {
			codec: saved.codec.unwrap_or_default(),
			bandwidth: saved.bandwidth.unwrap_or_default(),
			quality: saved.quality.unwrap_or_default(),
			camera_resolution: saved.camera_resolution.unwrap_or_default(),
			camera_frame_rate: saved.camera_frame_rate.unwrap_or_default(),
			camera_denoising: saved.camera_denoising.unwrap_or_default(),
		})
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
	#[default]
	RnNoise,
	WebRtc,
	LavaSrV2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Processing {
	pub suppression: NoiseSuppression,
	/// WebRTC suppression strength, from low (0) through very high (3).
	pub suppression_level: u8,
	pub echo_cancellation: bool,
	pub automatic_gain: bool,
	/// None is an open microphone; otherwise a dBFS threshold with a short release hold.
	pub sensitivity_db: Option<i16>,
}
impl Default for Processing {
	fn default() -> Self {
		Self {
			suppression: NoiseSuppression::RnNoise,
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
mod video_policy_tests {
	use super::*;

	#[test]
	fn bandwidth_profiles_match_the_shared_table_and_high_keeps_previous_fps_scaling() {
		for (width, height, average, maximum) in [
			(1280, 720, 2_000_000, 3_000_000),
			(1920, 1080, 6_000_000, 8_000_000),
			(2560, 1440, 10_000_000, 14_000_000),
			(3840, 2160, 14_000_000, 16_000_000),
			(7680, 4320, 24_000_000, 28_000_000),
		] {
			for fps in [15, 30, 60] {
				for dimensions in [(width, height), (height, width)] {
					let medium = VideoBandwidth::Medium.rates(dimensions.0, dimensions.1, fps);
					assert_eq!(
						medium,
						VideoBitrate {
							minimum: 200_000,
							average,
							maximum
						}
					);
					let low = VideoBandwidth::Low.rates(dimensions.0, dimensions.1, fps);
					assert_eq!(low.average, average / 2);
					assert_eq!(low.maximum, maximum / 2);
					assert_eq!(low.minimum, 200_000);
				}
			}
		}
		assert_eq!(
			VideoBandwidth::High.rates(2560, 1440, 30).average,
			12_000_000
		);
		assert_eq!(
			VideoBandwidth::High.rates(2560, 1440, 60).average,
			24_000_000
		);
		assert_eq!(
			VideoBandwidth::High.rates(7680, 4320, 60).maximum,
			50_000_000
		);
		assert_eq!(
			VideoBandwidth::Medium.rates(5120, 1440, 60).average,
			10_000_000
		);
		let native = VideoBandwidth::Medium.rates(6144, 2560, 60);
		assert!(native.average > 14_000_000 && native.average < 24_000_000);
		let limited = native.limited_to(400_000);
		assert_eq!(
			(limited.minimum, limited.average, limited.maximum),
			(200_000, 400_000, 400_000)
		);
		assert_eq!(native.limited_to(0).maximum, 200_000);
	}

	#[test]
	fn preset_reports_cannot_advertise_another_vendor_or_an_unavailable_codec() {
		let mut caps = DriverCapabilities::default();
		caps.quality_presets[VideoCodec::H265.index()][HardwareBackend::Amf.index()] =
			Some(EncoderPresets::from_bits(u32::MAX));
		assert!(!caps.available(VideoCodec::H265));
		assert!(
			!caps
				.presets(VideoCodec::H265)
				.contains(EncoderPreset::AmfQuality)
		);
		caps.set(
			HardwareBackend::Amf,
			VideoCodec::H265,
			ProbeResult::Available,
		);
		assert!(caps.presets_ready(VideoCodec::H265));
		assert!(
			caps.presets(VideoCodec::H265)
				.contains(EncoderPreset::AmfQuality)
		);
		assert!(
			!caps
				.presets(VideoCodec::H265)
				.contains(EncoderPreset::NvencP5)
		);
		assert!(!caps.presets_ready(VideoCodec::Av1));
	}
}
