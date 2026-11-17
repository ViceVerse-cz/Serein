//! Declarative, capability-gated customization of the client's start (connecting) screen.
//! Host mirror of extensions/sdk/src/start_screen.rs, the contract authority.
//!
//! A plugin with the `start_screen` grant returns a [`StartScreen`] from an activation or
//! panel result. The host keeps the last accepted value on the device and shows it on the
//! next start screen, before any account loads. The host renders nothing of its own design:
//! it shows the plugin's pre-rendered cell images (each a looping animation or a single
//! frame) and optional plain text, above the unchanged progress bar and account button.
//! Disabling the plugin, or returning `start_screen: null`, restores the default screen.

use serde::{Deserialize, Serialize};

/// Maximum UTF-8 bytes of the title.
pub const MAX_START_TITLE_BYTES: usize = 64;
/// Maximum UTF-8 bytes of the subtitle.
pub const MAX_START_SUBTITLE_BYTES: usize = 128;
/// Maximum images stacked on one start screen.
pub const MAX_START_IMAGES: usize = 4;
/// Maximum width or height of one image, in cells.
pub const MAX_START_IMAGE_CELLS: u16 = 160;
/// Maximum frames of one image.
pub const MAX_START_FRAMES: usize = 120;
/// Maximum frame rate of an animated image.
pub const MAX_START_FPS: u8 = 30;
/// Maximum palette entries; cell value 0 is always transparent.
pub const MAX_START_PALETTE: usize = 3;
/// Maximum logical pixels per cell.
pub const MAX_START_CELL_PIXELS: u8 = 8;
/// Maximum packed frame bytes across all images of one screen (before base64).
pub const MAX_START_FRAME_BYTES: usize = 160 * 1024;
/// Smallest window side a start screen may ask for, in logical pixels.
pub const MIN_START_WINDOW_SIDE: u16 = 240;
/// Largest window width a start screen may ask for, in logical pixels.
pub const MAX_START_WINDOW_WIDTH: u16 = 1600;
/// Largest window height a start screen may ask for, in logical pixels.
pub const MAX_START_WINDOW_HEIGHT: u16 = 1200;

/// One pre-rendered cell image. Every cell holds a 2-bit value: 0 is transparent and
/// 1..=3 select `palette[value - 1]`. A frame packs its cells row-major, four per byte, the
/// first cell in the two most significant bits, the last byte zero-padded; `frames` holds
/// each frame's bytes as standard base64. One frame is a still image; more loop at `fps`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartImage {
	pub width: u16,
	pub height: u16,
	/// Colors for cell values 1, 2 and 3: `#RRGGBB`, `#RRGGBBAA`, or `accent` / `accent:AA`
	/// for the client's current accent color (the one its progress bar uses) at full or `AA`
	/// hex opacity, so a screen follows the user's theme.
	pub palette: Vec<String>,
	pub frames: Vec<String>,
	/// Frames per second for an animation; ignored for a single frame.
	#[serde(default = "default_fps")]
	pub fps: u8,
	/// Logical pixels per cell; the host may scale an image down to fit the window.
	#[serde(default = "default_cell")]
	pub cell: u8,
}

fn default_fps() -> u8 {
	12
}

fn default_cell() -> u8 {
	3
}

impl StartImage {
	/// Packed bytes of one frame for this image's size.
	pub fn frame_bytes(&self) -> usize {
		(usize::from(self.width) * usize::from(self.height)).div_ceil(4)
	}
}

/// The window size a start screen asks for, in logical pixels. The host centers the window
/// on its monitor (clamped to the monitor), and restores the previous size, position and
/// maximized state when the start screen ends. It never saves this size as the window size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartWindow {
	pub width: u16,
	pub height: u16,
}

/// The start screen a plugin asks for. When set, it replaces the default logo and copy:
/// `images` stack top to bottom, then `title` and `subtitle` in the client's normal font.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartScreen {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub images: Vec<StartImage>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub subtitle: Option<String>,
	/// Resize the window while the start screen shows; omitted keeps the current window.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub window: Option<StartWindow>,
}

fn color(value: &str) -> bool {
	let hex = |text: &str| text.bytes().all(|b| b.is_ascii_hexdigit());
	if let Some(rest) = value.strip_prefix("accent") {
		return rest.is_empty()
			|| rest
				.strip_prefix(':')
				.is_some_and(|a| a.len() == 2 && hex(a));
	}
	matches!(value.len(), 7 | 9) && value.starts_with('#') && hex(&value[1..])
}

fn text(value: &Option<String>, limit: usize) -> bool {
	value.as_ref().is_none_or(|text| {
		!text.is_empty() && text.len() <= limit && !text.chars().any(char::is_control)
	})
}

/// Decoded length of standard padded base64, or `None` when it is not valid base64.
fn base64_len(value: &str) -> Option<usize> {
	let bytes = value.as_bytes();
	if !bytes.len().is_multiple_of(4) {
		return None;
	}
	let padding = bytes.iter().rev().take_while(|b| **b == b'=').count();
	if padding > 2 {
		return None;
	}
	let body = &bytes[..bytes.len() - padding];
	if !body
		.iter()
		.all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/')
	{
		return None;
	}
	Some(bytes.len() / 4 * 3 - padding)
}

impl StartScreen {
	/// Bound every field so the host can render the screen without further checks.
	pub fn validate(&self) -> Result<(), &'static str> {
		if !text(&self.title, MAX_START_TITLE_BYTES) {
			return Err("start screen title must be 1 through 64 bytes without controls");
		}
		if !text(&self.subtitle, MAX_START_SUBTITLE_BYTES) {
			return Err("start screen subtitle must be 1 through 128 bytes without controls");
		}
		if self.window.is_some_and(|window| {
			!(MIN_START_WINDOW_SIDE..=MAX_START_WINDOW_WIDTH).contains(&window.width)
				|| !(MIN_START_WINDOW_SIDE..=MAX_START_WINDOW_HEIGHT).contains(&window.height)
		}) {
			return Err("start screen window must be 240 through 1600 by 240 through 1200");
		}
		if self.images.len() > MAX_START_IMAGES {
			return Err("start screen allows at most 4 images");
		}
		let mut total = 0usize;
		for image in &self.images {
			if !(1..=MAX_START_IMAGE_CELLS).contains(&image.width)
				|| !(1..=MAX_START_IMAGE_CELLS).contains(&image.height)
			{
				return Err("start screen image must be 1 through 160 cells per side");
			}
			if image.palette.is_empty()
				|| image.palette.len() > MAX_START_PALETTE
				|| !image.palette.iter().all(|value| color(value))
			{
				return Err(
					"start screen image palette needs 1 through 3 colors or accent entries",
				);
			}
			if image.frames.is_empty() || image.frames.len() > MAX_START_FRAMES {
				return Err("start screen image needs 1 through 120 frames");
			}
			if image.frames.len() > 1 && !(1..=MAX_START_FPS).contains(&image.fps) {
				return Err("start screen animation fps must be 1 through 30");
			}
			if !(1..=MAX_START_CELL_PIXELS).contains(&image.cell) {
				return Err("start screen image cell must be 1 through 8 pixels");
			}
			let expected = image.frame_bytes();
			for frame in &image.frames {
				if base64_len(frame) != Some(expected) {
					return Err("start screen frame must be base64 of exactly the packed cells");
				}
				total += expected;
			}
		}
		if total > MAX_START_FRAME_BYTES {
			return Err("start screen frames exceed 160 KiB");
		}
		Ok(())
	}
}

/// Distinguish an explicit `null` (`Some(None)`, clearing the plugin's start screen) from an
/// omitted field (`None`, leaving it unchanged) in the host output mirror.
pub(crate) fn present<'de, D>(deserializer: D) -> Result<Option<Option<StartScreen>>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	Option::<StartScreen>::deserialize(deserializer).map(Some)
}
