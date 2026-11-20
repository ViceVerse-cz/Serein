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

impl SourceId {
	/// The Linux portal reports the picked display's size before capture starts, so the
	/// capture worker sizes the picture then ([`Settings::fitted_to`]). Other sources of
	/// unknown size never learn it in time.
	pub fn sized_after_pick(self) -> bool {
		matches!(self, Self::Portal)
	}
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

/// Smallest to largest, so the quality row reads 480p … 2160p, then Source.
const PRESETS: [Resolution; 5] = [
	Resolution::Preset {
		width: 854,
		height: 480,
	},
	Resolution::Preset {
		width: 1280,
		height: 720,
	},
	Resolution::Preset {
		width: 1920,
		height: 1080,
	},
	Resolution::Preset {
		width: 2560,
		height: 1440,
	},
	Resolution::Preset {
		width: 3840,
		height: 2160,
	},
];

/// Resolutions offered for a source, smallest first and Source last. Like Discord, a
/// preset is an upper bound: a smaller source keeps its native size, never upscaled. A
/// source of unknown size offers them all only when it is sized after the pick
/// (`sized_after_pick`); otherwise it keeps 1080p, 720p, and 480p, without Source.
pub fn offered_resolutions(
	width: u32,
	height: u32,
	sized_after_pick: bool,
) -> impl Iterator<Item = Resolution> {
	let all = (width > 0 && height > 0) || sized_after_pick;
	let source = all.then_some(Resolution::Source);
	let presets = PRESETS.into_iter().filter(move |preset| {
		all || matches!(preset, Resolution::Preset { height, .. } if *height <= 1080)
	});
	presets.chain(source)
}

/// 15, 30, and 60 always. 90 and 120 only when the monitor refreshes at least that fast.
pub fn offered_frame_rates(refresh_hz: u32) -> impl Iterator<Item = u32> {
	[15, 30, 60, 90, 120]
		.into_iter()
		.filter(move |fps| *fps <= 60 || refresh_hz >= *fps)
}

/// Output size for a choice, `None` when the source cannot use it. For a source sized
/// after the pick this is only the bound (Source: the largest share size), which the
/// capture worker narrows with [`Settings::fitted_to`].
pub fn picture(
	resolution: Resolution,
	width: u32,
	height: u32,
	sized_after_pick: bool,
) -> Option<(u32, u32)> {
	let bound = match resolution {
		Resolution::Source => (MAX_SHARE_WIDTH, MAX_SHARE_HEIGHT),
		Resolution::Preset { width, height } => (width, height),
	};
	if width > 0 && height > 0 {
		return fit_within(bound, width, height);
	}
	(sized_after_pick || matches!(resolution, Resolution::Preset { height, .. } if height <= 1080))
		.then_some(bound)
}

/// The shortest edge scaling may produce: OpenH264, the software fallback, rejects less.
const MIN_SCALED_EDGE: u32 = 16;

/// A source that fits inside the (even) `bound` keeps its native even size; a larger one
/// is scaled down onto one edge of `bound` at its own aspect ratio, so an ultrawide share
/// is not letterboxed into a 16:9 picture. A sliver keeps an encodable minimum edge.
fn fit_within(bound: (u32, u32), width: u32, height: u32) -> Option<(u32, u32)> {
	let (fitted_width, fitted_height) = if width <= bound.0 && height <= bound.1 {
		(width, height)
	} else {
		// The scaled edge rounds to the nearest even pixel.
		let scaled = |edge: u32, to: u32, from: u32| {
			let (edge, to, from) = (u128::from(edge), u128::from(to), u128::from(from));
			u32::try_from((edge * to + from) / (2 * from) * 2)
				.unwrap_or(0)
				.max(MIN_SCALED_EDGE)
		};
		if u64::from(width) * u64::from(bound.1) >= u64::from(height) * u64::from(bound.0) {
			(bound.0, scaled(height, bound.0, width))
		} else {
			(scaled(width, bound.1, height), bound.1)
		}
	};
	let (fitted_width, fitted_height) = (fitted_width & !1, fitted_height & !1);
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
	/// Narrows a bound chosen before the pick to the picked source's size, never upscaling.
	pub fn fitted_to(self, width: u32, height: u32) -> Self {
		if width == 0 || height == 0 {
			return self;
		}
		let Some((width, height)) = fit_within((self.width, self.height), width, height) else {
			return self;
		};
		let fitted = Self {
			width,
			height,
			..self
		};
		if fitted.valid() { fitted } else { self }
	}

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
		} else if pixels >= 3840 * 2160 {
			16_000_000
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
	/// Discord's latest viewers of this device's own or watched stream (unofficial
	/// `STREAM_UPDATE`).
	Viewers(Viewers),
}
/// Who is watching a stream: the full count and at most 64 of their IDs, in Discord's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Viewers {
	pub count: u16,
	pub ids: Vec<Id>,
}
impl Event {
	pub(crate) fn bytes(&self) -> usize {
		match self {
			Self::Server { token, endpoint } => {
				token.as_ref().map_or(0, voice::Secret::bytes)
					+ endpoint.as_ref().map_or(0, String::capacity)
			}
			Self::Viewers(viewers) => viewers.ids.capacity() * size_of::<Id>(),
			_ => 0,
		}
	}
}
