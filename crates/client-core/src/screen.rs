//! Bounded screen-share settings and ephemeral Gateway negotiation events.
use crate::voice;
use model::Id;

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
	/// Native pixel size. Zero means the platform has not reported one yet.
	pub width: u32,
	pub height: u32,
	/// Monitor refresh in hertz. Zero means unknown, which hides 90 and 120 fps.
	pub refresh_hz: u32,
}

/// A share picture the picker can offer. `Source` is the monitor or window's own size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
	Source,
	Preset { width: u32, height: u32 },
}

pub const MAX_SHARE_WIDTH: u32 = 3840;
pub const MAX_SHARE_HEIGHT: u32 = 2160;

const PRESETS: [Resolution; 4] = [
	Resolution::Preset {
		width: 2560,
		height: 1440,
	},
	Resolution::Preset {
		width: 1920,
		height: 1080,
	},
	Resolution::Preset {
		width: 1280,
		height: 720,
	},
	Resolution::Preset {
		width: 854,
		height: 480,
	},
];

/// Resolutions that fit inside a source. Unknown size keeps 1080p, 720p, and 480p.
pub fn offered_resolutions(width: u32, height: u32) -> impl Iterator<Item = Resolution> {
	let known = width > 0 && height > 0;
	let source = fitted_source(width, height).map(|_| Resolution::Source);
	let presets = PRESETS.into_iter().filter(move |preset| {
		let Resolution::Preset {
			width: preset_width,
			height: preset_height,
		} = *preset
		else {
			return false;
		};
		if known {
			width >= preset_width && height >= preset_height
		} else {
			preset_height <= 1080
		}
	});
	source.into_iter().chain(presets)
}

/// 15, 30, and 60 always. 90 and 120 only when the monitor refreshes at least that fast.
pub fn offered_frame_rates(refresh_hz: u32) -> impl Iterator<Item = u32> {
	[15, 30, 60, 90, 120]
		.into_iter()
		.filter(move |fps| *fps <= 60 || refresh_hz >= *fps)
}

/// Output size for a choice. `None` when that choice does not fit the source.
pub fn picture(resolution: Resolution, width: u32, height: u32) -> Option<(u32, u32)> {
	match resolution {
		Resolution::Source => fitted_source(width, height),
		Resolution::Preset {
			width: preset_width,
			height: preset_height,
		} => {
			let known = width > 0 && height > 0;
			if known && (width < preset_width || height < preset_height) {
				return None;
			}
			if !known && preset_height > 1080 {
				return None;
			}
			Some((preset_width, preset_height))
		}
	}
}

fn fitted_source(width: u32, height: u32) -> Option<(u32, u32)> {
	if width < 2 || height < 2 {
		return None;
	}
	if width <= MAX_SHARE_WIDTH && height <= MAX_SHARE_HEIGHT {
		let width = width & !1;
		let height = height & !1;
		return (width >= 2 && height >= 2).then_some((width, height));
	}
	let width = u64::from(width);
	let height = u64::from(height);
	let scale = ((u64::from(MAX_SHARE_WIDTH) << 16) / width)
		.min((u64::from(MAX_SHARE_HEIGHT) << 16) / height);
	let fitted_width = u32::try_from((width * scale) >> 16).unwrap_or(0) & !1;
	let fitted_height = u32::try_from((height * scale) >> 16).unwrap_or(0) & !1;
	(fitted_width >= 2 && fitted_height >= 2).then_some((fitted_width, fitted_height))
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
			&& self.width >= 2
			&& self.height >= 2
			&& self.width <= MAX_SHARE_WIDTH
			&& self.height <= MAX_SHARE_HEIGHT
			&& self.width.is_multiple_of(2)
			&& self.height.is_multiple_of(2)
			&& matches!(self.fps, 15 | 30 | 60 | 90 | 120)
	}

	pub fn bit_rate(self) -> u32 {
		let pixels = u64::from(self.width) * u64::from(self.height);
		let base = if pixels <= 854 * 480 {
			2_000_000
		} else if pixels <= 1280 * 720 {
			4_000_000
		} else if pixels <= 1920 * 1080 {
			8_000_000
		} else {
			let scaled = 8_000_000u64 * pixels / (1920 * 1080);
			u32::try_from(scaled.min(12_000_000)).unwrap_or(12_000_000)
		};
		let scale = match self.fps {
			120 => 4,
			90 => 3,
			60 => 2,
			_ => 1,
		};
		base.saturating_mul(scale).min(16_000_000)
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
