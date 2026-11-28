//! Platform hardware H.264 encoders shared by screen sharing and the camera.
//!
//! Windows prefers the driver's NVENC session and falls back to the Media Foundation transform;
//! macOS uses VideoToolbox; Linux encodes inside its GStreamer pipelines instead. Every backend is
//! optional: callers keep an openh264 encoder for the machines and formats the hardware refuses,
//! and fall back mid-stream when one fails.

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

/// Why an AV1 share could not start; the share dialog and diagnostics show it verbatim.
/// AV1 sending exists only where a backend implements it (see [`Codec`]), so the message and the
/// stop reason below are only reachable off Linux.
#[cfg(not(target_os = "linux"))]
pub(crate) const AV1_UNAVAILABLE: &str = "NVENC AV1 encoding is unavailable on this machine";

/// Why an AV1 share stopped: the transport has already labelled the stream AV1, so there is no
/// H.264 substitute to hand over to mid-share.
#[cfg(not(target_os = "linux"))]
pub(crate) const AV1_ENCODER_FAILED: &str = "the AV1 encoder stopped";

/// Codec a hardware backend is asked to produce. Only the Windows NVENC backend implements AV1;
/// every other backend refuses it instead of silently encoding H.264. Linux has no AV1 sender, so
/// the variant does not exist there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Codec {
	#[default]
	H264,
	#[cfg(not(target_os = "linux"))]
	Av1,
}

/// Whether this build and machine can encode AV1 at all.
#[cfg(target_os = "windows")]
pub(crate) fn av1_supported() -> bool {
	nvenc::support().av1
}
#[cfg(not(target_os = "windows"))]
pub(crate) fn av1_supported() -> bool {
	false
}

/// Windows hardware encoders in preference order: the driver's own NVENC session first, then the
/// Media Foundation transform. Both keep the caller's openh264 fallback, and a machine that offers
/// neither keeps the software encoder alone.
#[cfg(target_os = "windows")]
pub(crate) mod hardware {
	use super::{AV1_UNAVAILABLE, Codec, Config, nvenc, windows};

	pub(crate) struct Encoder(Backend);

	enum Backend {
		Nvenc(Box<nvenc::Encoder>),
		MediaFoundation(Box<windows::Encoder>),
	}

	impl Encoder {
		pub(crate) fn new(config: Config) -> Result<Self, &'static str> {
			match nvenc::Encoder::new(config) {
				Ok(encoder) => Ok(Self(Backend::Nvenc(Box::new(encoder)))),
				// Only NVENC produces AV1; the Media Foundation transform stays H.264,
				// so an AV1 share fails visibly instead of sending a different codec.
				Err(_) if config.codec == Codec::Av1 => Err(AV1_UNAVAILABLE),
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
	/// Codec to produce. A backend that cannot produce it fails rather than substituting another.
	pub codec: Codec,
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
