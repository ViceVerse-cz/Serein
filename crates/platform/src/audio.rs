//! Bounded output-format selection; callers own their devices, streams and workers.
use cpal::traits::DeviceTrait;
use cpal::{SampleFormat, SupportedStreamConfig, SupportedStreamConfigRange};

const UNSUPPORTED: &str = "Audio device format is unsupported; choose another device";

/// Selection policy for the caller's existing playback and channel conversion.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OutputConfigPreference {
	/// Prefer 48 kHz and the fewest channels, as voice playback historically did.
	Voice,
	/// Keep a usable system default; otherwise prefer stereo over surround or mono.
	Notification,
}

/// Select only F32/I16/I32/U16, 1–8 channels and 8–192 kHz. Never opens a stream.
/// Windows always retains the native shared-mode default, avoiding converted WASAPI
/// formats that some drivers advertise but reject when opening a stream.
pub fn output_config(
	device: &cpal::Device,
	preference: OutputConfigPreference,
) -> Result<SupportedStreamConfig, &'static str> {
	select_output_config(
		preference,
		cfg!(target_os = "windows"),
		|| {
			device
				.default_output_config()
				.map_err(|_| "Default audio format is unavailable")
		},
		|| {
			device
				.supported_output_configs()
				.map_err(|_| "Speaker formats are unavailable")
		},
	)
}

fn select_output_config<I: Iterator<Item = SupportedStreamConfigRange>>(
	preference: OutputConfigPreference,
	native_mix: bool,
	default: impl FnOnce() -> Result<SupportedStreamConfig, &'static str>,
	supported: impl FnOnce() -> Result<I, &'static str>,
) -> Result<SupportedStreamConfig, &'static str> {
	if native_mix {
		return checked(default()?);
	}
	match preference {
		OutputConfigPreference::Voice => advertised(supported()?, preference)
			.map(Ok)
			.unwrap_or_else(|| checked(default()?)),
		OutputConfigPreference::Notification => {
			let default = default();
			if let Ok(config) = &default
				&& usable(config)
			{
				return Ok(*config);
			}
			advertised(supported()?, preference)
				.map(Ok)
				.unwrap_or_else(|| checked(default?))
		}
	}
}

fn advertised(
	supported: impl Iterator<Item = SupportedStreamConfigRange>,
	preference: OutputConfigPreference,
) -> Option<SupportedStreamConfig> {
	supported
		.take(64)
		.filter(|config| {
			(1..=8).contains(&config.channels()) && supported_format(config.sample_format())
		})
		.filter_map(|config| {
			let rate = match preference {
				OutputConfigPreference::Voice => 48_000,
				OutputConfigPreference::Notification => {
					let min = config.min_sample_rate().max(8_000);
					let max = config.max_sample_rate().min(192_000);
					if min > max {
						return None;
					}
					48_000_u32.clamp(min, max)
				}
			};
			config.try_with_sample_rate(rate)
		})
		.min_by_key(|config| match preference {
			OutputConfigPreference::Voice => (0, config.channels(), 0),
			OutputConfigPreference::Notification => (
				u16::from(config.channels() == 1),
				config.channels().abs_diff(2),
				config.sample_rate().abs_diff(48_000),
			),
		})
}

fn checked(config: SupportedStreamConfig) -> Result<SupportedStreamConfig, &'static str> {
	if usable(&config) {
		Ok(config)
	} else {
		Err(UNSUPPORTED)
	}
}
fn usable(config: &SupportedStreamConfig) -> bool {
	(1..=8).contains(&config.channels())
		&& (8_000..=192_000).contains(&config.sample_rate())
		&& supported_format(config.sample_format())
}
fn supported_format(format: SampleFormat) -> bool {
	matches!(
		format,
		SampleFormat::F32 | SampleFormat::I16 | SampleFormat::I32 | SampleFormat::U16
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use OutputConfigPreference::{Notification, Voice};
	use cpal::SupportedBufferSize;

	fn range(
		channels: u16,
		min: u32,
		max: u32,
		format: SampleFormat,
	) -> SupportedStreamConfigRange {
		SupportedStreamConfigRange::new(channels, min, max, SupportedBufferSize::Unknown, format)
	}
	fn default_config(channels: u16, rate: u32, format: SampleFormat) -> SupportedStreamConfig {
		SupportedStreamConfig::new(channels, rate, SupportedBufferSize::Unknown, format)
	}
	fn select(
		preference: OutputConfigPreference,
		default: SupportedStreamConfig,
		ranges: Vec<SupportedStreamConfigRange>,
	) -> Result<SupportedStreamConfig, &'static str> {
		select_output_config(preference, false, || Ok(default), || Ok(ranges.into_iter()))
	}

	#[test]
	fn unsupported_default_recovers_without_downmixing_stereo_notifications() {
		let ranges = vec![
			range(2, 8_000, 192_000, SampleFormat::I24),
			range(1, 48_000, 48_000, SampleFormat::F32),
			range(6, 48_000, 48_000, SampleFormat::I16),
			range(2, 44_100, 44_100, SampleFormat::F32),
		];
		let default = default_config(2, 44_100, SampleFormat::I24);
		let notification = select(Notification, default, ranges.clone()).unwrap();
		assert_eq!(
			(
				notification.channels(),
				notification.sample_rate(),
				notification.sample_format()
			),
			(2, 44_100, SampleFormat::F32)
		);
		let voice = select(Voice, default, ranges).unwrap();
		assert_eq!((voice.channels(), voice.sample_rate()), (1, 48_000));
		// A failed default query can also recover from advertised configurations.
		let recovered = select_output_config(
			Notification,
			false,
			|| Err("no default"),
			|| Ok([range(2, 8_000, 192_000, SampleFormat::I32)].into_iter()),
		)
		.unwrap();
		assert_eq!(recovered.sample_rate(), 48_000);
	}

	#[test]
	fn notification_keeps_default_channels_and_can_fall_back_to_surround_or_mono() {
		let selected = select_output_config(
			Notification,
			false,
			|| Ok(default_config(6, 96_000, SampleFormat::U16)),
			|| -> Result<std::iter::Empty<SupportedStreamConfigRange>, &'static str> {
				panic!("usable notification default must not enumerate")
			},
		)
		.unwrap();
		assert_eq!((selected.channels(), selected.sample_rate()), (6, 96_000));
		let unsupported = default_config(2, 48_000, SampleFormat::I24);
		let surround = select(
			Notification,
			unsupported,
			vec![
				range(1, 48_000, 48_000, SampleFormat::F32),
				range(6, 48_000, 48_000, SampleFormat::F32),
			],
		)
		.unwrap();
		assert_eq!(surround.channels(), 6);
		let mono = select(
			Notification,
			unsupported,
			vec![range(1, 48_000, 48_000, SampleFormat::I16)],
		)
		.unwrap();
		assert_eq!(mono.channels(), 1);
	}

	#[test]
	fn windows_native_mix_never_uses_advertised_converted_formats() {
		for preference in [Voice, Notification] {
			let choose = |default| {
				select_output_config(
					preference,
					true,
					|| default,
					|| -> Result<std::iter::Empty<SupportedStreamConfigRange>, &'static str> {
						panic!("WASAPI enumeration must not run")
					},
				)
			};
			let native = choose(Ok(default_config(6, 44_100, SampleFormat::F32))).unwrap();
			assert_eq!(
				(
					native.channels(),
					native.sample_rate(),
					native.sample_format()
				),
				(6, 44_100, SampleFormat::F32)
			);
			assert!(choose(Ok(default_config(2, 48_000, SampleFormat::I24))).is_err());
			assert_eq!(choose(Err("no native mix")).unwrap_err(), "no native mix");
		}
	}

	#[test]
	fn selection_bounds_ranges_channels_and_rates_and_preserves_voice_fallback() {
		let invalid = default_config(2, 48_000, SampleFormat::I24);
		let mut ranges = vec![range(2, 48_000, 48_000, SampleFormat::I24); 64];
		ranges.push(range(2, 48_000, 48_000, SampleFormat::F32));
		assert!(select(Notification, invalid, ranges).is_err());
		let ranges = vec![
			range(0, 48_000, 48_000, SampleFormat::F32),
			range(9, 48_000, 48_000, SampleFormat::F32),
			range(2, 1, 7_999, SampleFormat::F32),
			range(2, 192_001, 384_000, SampleFormat::F32),
			range(2, 44_100, 44_100, SampleFormat::I16),
		];
		// Invalid candidates must be rejected even without a valid stereo range
		// outranking them. The unsupported default cannot mask an invalid selection.
		assert!(select(Notification, invalid, ranges[..4].to_vec()).is_err());
		assert!(select(Voice, invalid, ranges[..4].to_vec()).is_err());
		let selected = select(Notification, invalid, ranges.clone()).unwrap();
		assert_eq!((selected.channels(), selected.sample_rate()), (2, 44_100));
		let voice = select(Voice, default_config(6, 96_000, SampleFormat::U16), ranges).unwrap();
		assert_eq!(
			(voice.channels(), voice.sample_rate(), voice.sample_format()),
			(6, 96_000, SampleFormat::U16)
		);
		for default in [
			default_config(0, 48_000, SampleFormat::F32),
			default_config(9, 48_000, SampleFormat::F32),
			default_config(2, 7_999, SampleFormat::F32),
			default_config(2, 192_001, SampleFormat::F32),
		] {
			assert!(select(Voice, default, vec![]).is_err());
			assert!(select(Notification, default, vec![]).is_err());
		}
	}
}
