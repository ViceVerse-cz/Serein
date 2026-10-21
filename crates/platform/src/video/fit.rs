use super::{Color, INVALID, MAX_BYTES, SOURCE_TOO_LARGE, check_dimensions};

/// Longest attachment source or output edge: the desktop wgpu texture limit.
const EDGE: u32 = 8192;
const OUTPUT_PIXELS: u64 = 3840 * 2160;
const SOURCE_PIXELS: u64 = 7680 * 4320;
const STEP: u32 = 64;

const _: () = assert!(OUTPUT_PIXELS * 3 / 2 <= MAX_BYTES as u64);

/// The physical-pixel box a picture is shown in, in display orientation. Edges are multiples
/// of 64 between 64 and 8192.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
	width: u32,
	height: u32,
}

impl Viewport {
	pub const POSTER: Self = Self {
		width: 320,
		height: 320,
	};

	/// Non-finite or non-positive edges become one step.
	pub fn from_pixels(width: f32, height: f32) -> Self {
		let edge = |value: f32| {
			if !value.is_finite() || value <= 0.0 {
				return STEP;
			}
			(value.ceil().min(EDGE as f32) as u32)
				.div_ceil(STEP)
				.saturating_mul(STEP)
				.clamp(STEP, EDGE)
		};
		Self {
			width: edge(width),
			height: edge(height),
		}
	}

	pub fn width(self) -> u32 {
		self.width
	}

	pub fn height(self) -> u32 {
		self.height
	}

	pub fn pack(self) -> u64 {
		(u64::from(self.width) << 32) | u64::from(self.height)
	}

	pub fn unpack(word: u64) -> Option<Self> {
		let edge = |value: u32| {
			(value.is_multiple_of(STEP) && (STEP..=EDGE).contains(&value)).then_some(value)
		};
		Some(Self {
			width: edge((word >> 32) as u32)?,
			height: edge(word as u32)?,
		})
	}
}

/// Coded size handed to the OS scaler. `fit` contains the upright picture in the viewport,
/// never upscales, and keeps a scaled size even and within 4K.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputSize {
	width: u32,
	height: u32,
}

impl OutputSize {
	pub(crate) fn fit(coded: (u32, u32), rotation: u32, viewport: Viewport) -> Self {
		let (width, height) = (coded.0.max(1), coded.1.max(1));
		let (shown_width, shown_height) = if matches!(rotation, 90 | 270) {
			(height, width)
		} else {
			(width, height)
		};
		let area = f64::from(width) * f64::from(height);
		let scale = (f64::from(viewport.width) / f64::from(shown_width))
			.min(f64::from(viewport.height) / f64::from(shown_height))
			.min((OUTPUT_PIXELS as f64 / area).sqrt())
			.min(1.0);
		if scale >= 1.0 {
			return Self { width, height };
		}
		let even = |edge: u32| (((f64::from(edge) * scale) as u32) & !1).max(2);
		Self {
			width: even(width),
			height: even(height),
		}
	}

	pub(crate) fn width(self) -> u32 {
		self.width
	}

	pub(crate) fn height(self) -> u32 {
		self.height
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FramePolicy {
	#[cfg_attr(
		target_os = "windows",
		expect(
			dead_code,
			reason = "the Windows live decoder calls check_dimensions directly"
		)
	)]
	Live,
	Attachment {
		source_height: u32,
	},
}

impl FramePolicy {
	pub(crate) fn check(self, width: u32, height: u32) -> Result<(), &'static str> {
		match self {
			Self::Live => check_dimensions(width, height),
			Self::Attachment { .. } => (width > 0
				&& height > 0
				&& width <= EDGE
				&& height <= EDGE
				&& u64::from(width) * u64::from(height) <= OUTPUT_PIXELS)
				.then_some(())
				.ok_or(INVALID),
		}
	}

	pub(crate) fn untagged(self, frame_height: u32, full_range: bool) -> Color {
		match self {
			Self::Live => Color::untagged(frame_height, full_range),
			Self::Attachment { source_height } => Color::untagged(source_height, full_range),
		}
	}
}

pub(crate) fn check_source(width: u32, height: u32) -> Result<(), &'static str> {
	if width == 0 || height == 0 {
		return Err(INVALID);
	}
	if width > EDGE || height > EDGE || u64::from(width) * u64::from(height) > SOURCE_PIXELS {
		return Err(SOURCE_TOO_LARGE);
	}
	Ok(())
}
