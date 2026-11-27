//! Platform hardware H.264 encoders shared by screen sharing and the camera.
//!
//! Media Foundation on Windows and VideoToolbox on macOS; Linux encodes inside its GStreamer
//! pipelines instead. Every backend is optional: callers keep an openh264 encoder for the
//! machines and formats the hardware refuses, and fall back mid-stream when one fails.

#[cfg(target_os = "macos")]
#[path = "video_encode_macos.rs"]
pub(crate) mod macos;
#[cfg(target_os = "windows")]
#[path = "video_encode_nvenc.rs"]
pub(crate) mod nvenc;
#[cfg(target_os = "windows")]
#[path = "video_encode_windows.rs"]
pub(crate) mod windows;

#[cfg(target_os = "macos")]
pub(crate) use macos as hardware;

/// Windows hardware encoders in preference order: the driver's own NVENC session first, then the
/// Media Foundation transform. Both keep the caller's openh264 fallback, and a machine that offers
/// neither keeps the software encoder alone.
#[cfg(target_os = "windows")]
pub(crate) mod hardware {
	use super::{Config, nvenc, windows};

	pub(crate) struct Encoder(Backend);

	enum Backend {
		Nvenc(Box<nvenc::Encoder>),
		MediaFoundation(Box<windows::Encoder>),
	}

	impl Encoder {
		pub(crate) fn new(config: Config) -> Result<Self, &'static str> {
			match nvenc::Encoder::new(config) {
				Ok(encoder) => Ok(Self(Backend::Nvenc(Box::new(encoder)))),
				Err(_) => windows::Encoder::new(config)
					.map(|encoder| Self(Backend::MediaFoundation(Box::new(encoder)))),
			}
		}

		pub(crate) fn set_bitrate(&mut self, bit_rate: u32) -> Result<(), &'static str> {
			match &mut self.0 {
				Backend::Nvenc(encoder) => encoder.set_bitrate(bit_rate),
				Backend::MediaFoundation(encoder) => encoder.set_bitrate(bit_rate),
			}
		}

		pub(crate) fn encode(
			&mut self,
			y: &[u8],
			u: &[u8],
			v: &[u8],
			force_keyframe: bool,
		) -> Result<(Vec<u8>, bool), &'static str> {
			match &mut self.0 {
				Backend::Nvenc(encoder) => encoder.encode(y, u, v, force_keyframe),
				Backend::MediaFoundation(encoder) => encoder.encode(y, u, v, force_keyframe),
			}
		}

		pub(crate) fn encode_with(
			&mut self,
			length: usize,
			force_keyframe: bool,
			fill: impl FnOnce(&mut [u8]) -> Result<(), &'static str>,
		) -> Result<(Vec<u8>, bool), &'static str> {
			match &mut self.0 {
				Backend::Nvenc(encoder) => encoder.encode_with(length, force_keyframe, fill),
				Backend::MediaFoundation(encoder) => {
					encoder.encode_with(length, force_keyframe, fill)
				}
			}
		}
	}
}

/// One hardware encoder's output format. Bounds come from the caller because a screen frame
/// and a camera frame have very different limits.
#[derive(Clone, Copy)]
pub(crate) struct Config {
	pub width: u32,
	pub height: u32,
	pub fps: u32,
	pub bit_rate: u32,
	/// Largest encoded access unit the caller's transport accepts.
	pub max_bytes: usize,
	/// Screen sharing already ships Main; the camera keeps the Baseline profile its
	/// software encoder has always sent, so its wire format is unchanged.
	pub profile: Profile,
}

/// H.264 profile requested from the hardware encoder.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
	Baseline,
	#[cfg_attr(target_os = "linux", allow(dead_code))]
	Main,
}

/// Packed layout of the pictures handed to a hardware encoder.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceFormat {
	/// Screen capture delivers 32-bit BGRA.
	Bgra,
	/// The camera delivers 24-bit RGB.
	Rgb,
}

#[cfg(target_os = "macos")]
impl SourceFormat {
	pub(crate) fn bytes_per_pixel(self) -> usize {
		match self {
			Self::Bgra => 4,
			Self::Rgb => 3,
		}
	}
}
