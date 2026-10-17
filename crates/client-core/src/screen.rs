//! Bounded screen-share settings and ephemeral Gateway negotiation events.
use crate::voice;
use model::{Id, voice_settings::VideoResolution};

pub const MAX_SOURCES: usize = 64;
pub const MAX_SOURCE_NAME_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceId {
	/// macOS chooses a display/window only after an explicit Share action.
	SystemPicker,
	/// The Linux desktop chooses the source after an explicit Share action.
	Portal,
	/// Explicit whole-desktop capture on a native X11 session, without a portal.
	X11Desktop,
	Display(u64),
	Window(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
	pub id: SourceId,
	pub name: String,
	/// Physical pixels; system pickers resolve these only after explicit selection.
	pub dimensions: Option<(u32, u32)>,
}

/// Screen quality is independent of the camera's saved resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
	Source,
	Preset(VideoResolution),
	/// Concrete output for existing offline callers, never offered by the picker.
	Fixed(u32, u32),
}

/// The same 8K pixel/allocation envelope in either orientation.
pub const fn valid_dimensions(width: u32, height: u32) -> bool {
	width > 0
		&& height > 0
		&& width <= VideoResolution::MAX_WIDTH
		&& height <= VideoResolution::MAX_WIDTH
		&& (width <= VideoResolution::MAX_HEIGHT || height <= VideoResolution::MAX_HEIGHT)
}

fn even(value: u64, limit: u32) -> Option<u32> {
	let rounded = value.checked_add(1)? / 2 * 2;
	let value = u32::try_from(rounded).ok()?.min(limit / 2 * 2);
	(value >= 2).then_some(value)
}

impl Resolution {
	pub fn dimensions(self, (width, height): (u32, u32)) -> Option<(u32, u32)> {
		if !valid_dimensions(width, height) {
			return None;
		}
		let size = match self {
			Self::Source => (width / 2 * 2, height / 2 * 2),
			Self::Fixed(w, h) if w <= width && h <= height => (w, h),
			Self::Fixed(_, _) => return None,
			Self::Preset(preset) => {
				let target = preset.dimensions().1;
				let short = width.min(height);
				if target > short {
					return None;
				}
				let long = u64::from(width.max(height)).checked_mul(u64::from(target))?;
				let long = long.checked_add(u64::from(short))? / (u64::from(short) * 2) * 2;
				let long = even(long, width.max(height))?;
				if width >= height {
					(long, target)
				} else {
					(target, long)
				}
			}
		};
		(valid_dimensions(size.0, size.1)
			&& size.0 >= 2
			&& size.1 >= 2
			&& size.0.is_multiple_of(2)
			&& size.1.is_multiple_of(2))
		.then_some(size)
	}

	pub fn fitting(self, source: (u32, u32)) -> Self {
		if self.dimensions(source).is_some() {
			return self;
		}
		VideoResolution::ALL
			.iter()
			.rev()
			.copied()
			.map(Self::Preset)
			.find(|mode| mode.dimensions(source).is_some())
			.unwrap_or(Self::Source)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestSettings {
	pub source: SourceId,
	pub resolution: Resolution,
	pub fps: u32,
	pub cursor: bool,
	pub audio: bool,
}
impl RequestSettings {
	pub fn valid(self) -> bool {
		!matches!(self.source, SourceId::Display(0) | SourceId::Window(0))
			&& matches!(self.fps, 15 | 30 | 60)
			&& match self.resolution {
				Resolution::Fixed(width, height) => {
					valid_dimensions(width, height)
						&& width >= 2 && height >= 2
						&& width.is_multiple_of(2)
						&& height.is_multiple_of(2)
				}
				_ => true,
			}
	}

	pub fn resolve(self, source: (u32, u32)) -> Option<Settings> {
		if !self.valid() {
			return None;
		}
		let (width, height) = self.resolution.fitting(source).dimensions(source)?;
		let settings = Settings {
			source: self.source,
			width,
			height,
			fps: self.fps,
			cursor: self.cursor,
			audio: self.audio,
		};
		settings.valid().then_some(settings)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
	pub source: SourceId,
	pub width: u32,
	pub height: u32,
	pub fps: u32,
	pub cursor: bool,
	/// Share system audio with the screen; call microphone settings are independent.
	pub audio: bool,
}
impl Settings {
	pub fn valid(self) -> bool {
		!matches!(self.source, SourceId::Display(0) | SourceId::Window(0))
			&& valid_dimensions(self.width, self.height)
			&& self.width >= 2
			&& self.height >= 2
			&& self.width.is_multiple_of(2)
			&& self.height.is_multiple_of(2)
			&& matches!(self.fps, 15 | 30 | 60)
	}

	pub fn bit_rate(self) -> u32 {
		let pixels = (u64::from(self.width) * u64::from(self.height)).min(7680 * 4320);
		let points = [
			(854u64 * 480, 2_000_000u64),
			(1280 * 720, 4_000_000),
			(1920 * 1080, 8_000_000),
			(2560 * 1440, 12_000_000),
			(3840 * 2160, 20_000_000),
			(7680 * 4320, 40_000_000),
		];
		let mut base = points[0].1;
		for [(low, low_rate), (high, high_rate)] in points.windows(2).map(|p| [p[0], p[1]]) {
			if pixels >= high {
				base = high_rate;
			} else if pixels > low {
				base = low_rate + (pixels - low) * (high_rate - low_rate) / (high - low);
				break;
			}
		}
		(base * if self.fps == 60 { 2 } else { 1 }).min(50_000_000) as u32
	}
}

impl From<Settings> for RequestSettings {
	fn from(settings: Settings) -> Self {
		Self {
			source: settings.source,
			resolution: Resolution::Fixed(settings.width, settings.height),
			fps: settings.fps,
			cursor: settings.cursor,
			audio: settings.audio,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn presets_follow_all_source_ratios_and_physical_pixel_sizes() {
		let mode = Resolution::Preset(VideoResolution::P1080);
		for (source, expected) in [
			((3840, 2160), (1920, 1080)),
			((2560, 1600), (1728, 1080)),
			((2560, 1920), (1440, 1080)),
			((2560, 2048), (1350, 1080)),
			((3000, 2000), (1620, 1080)),
			((3440, 1440), (2580, 1080)),
			((5120, 1440), (3840, 1080)),
			((6144, 2560), (2592, 1080)),
			((2160, 3840), (1080, 1920)),
			((2160, 2160), (1080, 1080)),
		] {
			assert_eq!(mode.dimensions(source), Some(expected), "source={source:?}");
			assert_eq!(Resolution::Source.dimensions(source), Some(source));
			assert_eq!(
				mode.dimensions((source.1, source.0)),
				Some((expected.1, expected.0))
			);
		}
		// Logical Retina/DPI dimensions cannot replace the physical capture extent.
		assert_eq!(
			Resolution::Source.dimensions((2880, 1800)),
			Some((2880, 1800))
		);
		assert_eq!(
			Resolution::Source.dimensions((3840, 2160)),
			Some((3840, 2160))
		);
	}
	#[test]
	fn fitting_presets_alignment_and_8k_envelope_do_not_upscale() {
		for source in [
			(1920, 1080),
			(2560, 1440),
			(3840, 2160),
			(7680, 4320),
			(4320, 7680),
			(6144, 2560),
			(6145, 2561),
			(321, 241),
		] {
			let native = Resolution::Source.dimensions(source).unwrap();
			assert!(native.0 <= source.0 && native.1 <= source.1);
			assert!(native.0.is_multiple_of(2) && native.1.is_multiple_of(2));
			for preset in VideoResolution::ALL {
				if let Some(size) = Resolution::Preset(preset).dimensions(source) {
					assert!(size.0 <= source.0 && size.1 <= source.1);
					assert!(valid_dimensions(size.0, size.1));
					// Alignment changes the ideal long edge by at most one pixel.
					let error = (u64::from(size.0) * u64::from(source.1))
						.abs_diff(u64::from(size.1) * u64::from(source.0));
					assert!(error <= u64::from(source.0.max(source.1)));
				}
			}
		}
		let oversized = Resolution::Preset(VideoResolution::P4320);
		assert_eq!(
			oversized.fitting((1920, 1080)),
			Resolution::Preset(VideoResolution::P1080)
		);
		assert_eq!(
			oversized.fitting((6144, 2560)),
			Resolution::Preset(VideoResolution::P2160)
		);
		assert_eq!(oversized.fitting((320, 240)), Resolution::Source);
		for size in [
			(0, 1080),
			(1080, 0),
			(7682, 4320),
			(4320, 7682),
			(7680, 7680),
			(u32::MAX, u32::MAX),
			(1, 1),
		] {
			assert!(Resolution::Source.dimensions(size).is_none());
		}
	}
	#[test]
	fn bitrate_preserves_preset_rates_and_scales_native_pixels_monotonically() {
		let mut settings = Settings {
			source: SourceId::Display(1),
			width: 854,
			height: 480,
			fps: 30,
			cursor: true,
			audio: false,
		};
		for (preset, rate) in VideoResolution::ALL.into_iter().zip([
			2_000_000, 4_000_000, 8_000_000, 12_000_000, 20_000_000, 40_000_000,
		]) {
			(settings.width, settings.height) = preset.dimensions();
			assert_eq!(settings.bit_rate(), rate);
			settings.fps = 60;
			assert_eq!(settings.bit_rate(), (rate * 2).min(50_000_000));
			settings.fps = 30;
		}
		(settings.width, settings.height) = (6144, 2560);
		assert!((20_000_000..40_000_000).contains(&settings.bit_rate()));
		settings.fps = 60;
		assert_eq!(settings.bit_rate(), 50_000_000);
		(settings.width, settings.height) = (2592, 1080);
		assert!((16_000_000..24_000_000).contains(&settings.bit_rate()));
		(settings.width, settings.height) = (1080, 2592);
		assert!((16_000_000..24_000_000).contains(&settings.bit_rate()));
	}

	#[test]
	fn output_presets_and_frame_rates_keep_bitrate_bounded_through_8k() {
		for resolution in VideoResolution::ALL {
			let (width, height) = resolution.dimensions();
			for fps in [15, 30, 60] {
				let settings = Settings {
					source: SourceId::Display(1),
					width,
					height,
					fps,
					cursor: true,
					audio: false,
				};
				assert!(settings.valid());
				assert!((2_000_000..=50_000_000).contains(&settings.bit_rate()));
			}
		}
		let settings = Settings {
			source: SourceId::Display(1),
			width: 7680,
			height: 4320,
			fps: 60,
			cursor: true,
			audio: false,
		};
		assert_eq!(settings.bit_rate(), 50_000_000);
		for invalid in [
			Settings {
				width: 7682,
				..settings
			},
			Settings {
				height: 4322,
				..settings
			},
			Settings {
				fps: 120,
				..settings
			},
			Settings {
				source: SourceId::Display(0),
				..settings
			},
		] {
			assert!(!invalid.valid());
		}
	}
}

#[derive(Debug)]
pub enum Event {
	Created {
		rtc_server: Id,
		rtc_channel: Id,
	},
	Server {
		token: Option<voice::Secret>,
		endpoint: Option<String>,
	},
	/// The stream is gone. `reason` names Discord's cause when it sent one we recognise.
	Deleted {
		reason: Option<&'static str>,
	},
	Failed(&'static str),
}
impl Event {
	pub(crate) fn bytes(&self) -> usize {
		match self {
			Self::Server { token, endpoint } => {
				token.as_ref().map_or(0, voice::Secret::bytes)
					+ endpoint.as_ref().map_or(0, String::capacity)
			}
			_ => 0,
		}
	}
}
