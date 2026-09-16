//! Platform hardware H.264 encoders shared by screen sharing and the camera.
//!
//! Media Foundation on Windows and VideoToolbox on macOS; Linux encodes inside its GStreamer
//! pipelines instead. Every backend is optional: callers keep an openh264 encoder for the
//! machines and formats the hardware refuses, and fall back mid-stream when one fails.

#[cfg(target_os = "macos")]
#[path = "video_encode_macos.rs"]
pub(crate) mod macos;
#[cfg(target_os = "windows")]
#[path = "video_encode_windows.rs"]
pub(crate) mod windows;

#[cfg(target_os = "macos")]
pub(crate) use macos as hardware;
#[cfg(target_os = "windows")]
pub(crate) use windows as hardware;

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
