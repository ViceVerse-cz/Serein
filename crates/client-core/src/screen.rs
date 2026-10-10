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
}

#[derive(Clone, Copy, Debug)]
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
			&& VideoResolution::ALL
				.iter()
				.any(|resolution| resolution.dimensions() == (self.width, self.height))
			&& matches!(self.fps, 15 | 30 | 60)
	}

	pub fn bit_rate(self) -> u32 {
		let base = match (self.width, self.height) {
			(854, 480) => 2_000_000,
			(1920, 1080) => 8_000_000,
			(2560, 1440) => 12_000_000,
			(3840, 2160) => 20_000_000,
			(7680, 4320) => 40_000_000,
			_ => 4_000_000,
		};
		(base * if self.fps == 60 { 2 } else { 1 }).min(50_000_000)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

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
