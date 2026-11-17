//! Host playback of a plugin's start (connecting) screen. The host draws nothing of its own
//! design: a screen's frames are decoded once into palette indices when it is loaded or
//! accepted, and each shown frame is mapped through its palette into one reused texture per
//! image. The optional title and subtitle are normal labels.

use egui::emath::GuiRounding;
use extensions::{StartImage, StartScreen, StartWindow};
use std::{sync::Arc, time::Duration};

/// Space kept under the screen's last control, at least the gaps around the progress bar.
const BOTTOM_PADDING: f32 = 32.0;
/// Height of the progress bar and the way-out button assumed before the first frame
/// measures them.
const BELOW_ESTIMATE: f32 = 80.0;
/// Height of the title and subtitle assumed before the first frame measures them.
const TEXT_ESTIMATE: f32 = 80.0;
/// Approximate height of the default logo, title and stage copy, in logical pixels.
const DEFAULT_HEADER: f32 = 124.0;
/// Space between the logo or images and the title, in logical pixels.
const HEADER_GAP: f32 = 20.0;

/// A validated plugin start screen with every frame decoded once into palette indices, so
/// playback only maps the shown frame through its palette.
pub struct DecodedStartScreen {
	screen: StartScreen,
	images: Vec<Image>,
}

impl DecodedStartScreen {
	/// Validates and decodes a screen; `None` when it is invalid.
	pub fn new(screen: StartScreen) -> Option<Self> {
		screen.validate().ok()?;
		let images = screen.images.iter().map(Image::decode).collect();
		Some(Self { screen, images })
	}

	/// The screen as the plugin set it.
	pub fn screen(&self) -> &StartScreen {
		&self.screen
	}
}

/// One image's decoded frames.
struct Image {
	/// Cells across and down.
	size: [usize; 2],
	/// Colors for cell values 0 through 3; 0 and values past the plugin's palette are transparent.
	palette: [Paint; 4],
	/// One palette index per cell, row-major, for each frame.
	frames: Vec<Vec<u8>>,
	fps: f64,
	/// Logical pixels per cell.
	cell: f32,
}

impl Image {
	fn decode(image: &StartImage) -> Self {
		let size = [usize::from(image.width), usize::from(image.height)];
		let mut palette = [Paint::Fixed(egui::Color32::TRANSPARENT); 4];
		for (slot, value) in palette[1..].iter_mut().zip(&image.palette) {
			*slot = paint(value);
		}
		Self {
			size,
			palette,
			frames: image
				.frames
				.iter()
				.map(|frame| indices(frame, size[0] * size[1]))
				.collect(),
			fps: f64::from(image.fps),
			cell: f32::from(image.cell),
		}
	}

	/// The frame shown `elapsed` seconds into playback.
	fn frame(&self, elapsed: f64) -> usize {
		if self.frames.len() < 2 {
			return 0;
		}
		(elapsed * self.fps) as usize % self.frames.len()
	}

	/// Time from `elapsed` until an animation shows its next frame.
	fn next_frame(&self, elapsed: f64) -> Option<Duration> {
		(self.frames.len() > 1).then(|| {
			let next = ((elapsed * self.fps).floor() + 1.0) / self.fps;
			Duration::from_secs_f64((next - elapsed).max(0.0))
		})
	}

	/// One frame mapped through the palette, with `accent` entries in the current accent.
	fn pixels(&self, frame: usize, accent: egui::Color32) -> egui::ColorImage {
		let colors = self.palette.map(|paint| match paint {
			Paint::Fixed(color) => color,
			Paint::Accent(alpha) => {
				let [r, g, b, _] = accent.to_srgba_unmultiplied();
				egui::Color32::from_rgba_unmultiplied(r, g, b, alpha)
			}
		});
		egui::ColorImage::new(
			self.size,
			self.frames[frame]
				.iter()
				.map(|index| colors[usize::from(*index)])
				.collect(),
		)
	}

	/// The image's size at `cell` logical pixels per cell.
	fn extent(&self, cell: f32) -> egui::Vec2 {
		egui::vec2(self.size[0] as f32 * cell, self.size[1] as f32 * cell)
	}
}

/// The size of images stacked flush, top to bottom, at `cell` logical pixels per cell each.
fn stack(images: &[Image], cell: impl Fn(&Image) -> f32) -> egui::Vec2 {
	images.iter().fold(egui::Vec2::ZERO, |stack, image| {
		let size = image.extent(cell(image));
		egui::vec2(stack.x.max(size.x), stack.y + size.y)
	})
}

/// Unpacks one frame's base64 into a palette index per cell: four cells per byte, the first
/// in the two most significant bits. Validation guarantees the packed length; the zero padding
/// of the last byte is dropped.
fn indices(frame: &str, cells: usize) -> Vec<u8> {
	let mut indices = Vec::with_capacity(cells.next_multiple_of(4));
	let (mut bits, mut held) = (0u32, 0u32);
	for value in frame.bytes().map_while(sextet) {
		bits = (bits << 6) | u32::from(value);
		held += 6;
		if held >= 8 {
			held -= 8;
			let byte = (bits >> held) as u8;
			indices.extend([byte >> 6, (byte >> 4) & 3, (byte >> 2) & 3, byte & 3]);
		}
	}
	indices.resize(cells, 0);
	indices
}

/// The value of a standard base64 symbol; `None` for padding.
fn sextet(symbol: u8) -> Option<u8> {
	Some(match symbol {
		b'A'..=b'Z' => symbol - b'A',
		b'a'..=b'z' => symbol - b'a' + 26,
		b'0'..=b'9' => symbol - b'0' + 52,
		b'+' => 62,
		b'/' => 63,
		_ => return None,
	})
}

/// A palette entry: a fixed color, or the client's current accent at some opacity.
#[derive(Clone, Copy)]
enum Paint {
	Fixed(egui::Color32),
	Accent(u8),
}

/// Parses a validated `#RRGGBB`, `#RRGGBBAA`, `accent` or `accent:AA` palette entry.
fn paint(value: &str) -> Paint {
	let channel = |text: &str, at: usize| {
		text.get(at..at + 2)
			.and_then(|hex| u8::from_str_radix(hex, 16).ok())
	};
	if let Some(rest) = value.strip_prefix("accent") {
		return Paint::Accent(
			rest.strip_prefix(':')
				.and_then(|alpha| channel(alpha, 0))
				.unwrap_or(u8::MAX),
		);
	}
	match (channel(value, 1), channel(value, 3), channel(value, 5)) {
		(Some(r), Some(g), Some(b)) => Paint::Fixed(egui::Color32::from_rgba_unmultiplied(
			r,
			g,
			b,
			channel(value, 7).unwrap_or(u8::MAX),
		)),
		_ => Paint::Fixed(egui::Color32::TRANSPARENT),
	}
}

/// Start-screen playback state. Draw with [`StartScreenView::header`] and call
/// [`StartScreenView::retire`] once per frame afterwards, so the image textures exist only
/// while the start screen is visible.
#[derive(Default)]
pub struct StartScreenView {
	playing: Option<Playing>,
	lift: f32,
	/// Where this frame's copy starts; the title and subtitle measured below it, and how far
	/// the bar and button reached below the text on the previous frame, size the images so
	/// all of it fits above the bottom padding.
	copy_top: f32,
	text: f32,
	below: f32,
	window: Option<StartWindow>,
	shown: bool,
}

/// The screen being played: when it started, in egui time, and one texture per image with
/// the frame it holds.
struct Playing {
	screen: Arc<DecodedStartScreen>,
	start: f64,
	/// The accent the textures were colored with; a theme change re-colors them.
	accent: egui::Color32,
	textures: Vec<(egui::TextureHandle, usize)>,
}

impl StartScreenView {
	/// How far to raise the column so a taller custom header stays centred. It is measured on
	/// the previous frame, which the restore screen's continuous repaint keeps current.
	pub fn lift(&self) -> f32 {
		self.lift
	}

	/// The window size the start screen drawn this frame asked for.
	pub fn window(&self) -> Option<StartWindow> {
		self.window.filter(|_| self.shown)
	}

	/// Records where the screen's last control ended, after the caller drew everything below
	/// the header; `below` excludes the header's own text, which `text` already measures.
	pub fn end(&mut self, ui: &egui::Ui) {
		self.below = (ui.cursor().min.y - self.copy_top - self.text).max(0.0);
	}

	/// Draws the header into the current top-down column. Without a plugin screen this is the
	/// client's own logo, title and stage line. A plugin screen replaces all three with its
	/// images, stacked top to bottom, then its optional title and subtitle. Both lines
	/// truncate instead of wrapping, so a short window always leaves room for the bar and
	/// the way out below.
	pub fn header(
		&mut self,
		ui: &mut egui::Ui,
		screen: Option<Arc<DecodedStartScreen>>,
		logo: impl FnOnce(&mut egui::Ui),
		title: &str,
		subtitle: &str,
	) {
		self.shown = true;
		let p = crate::design::palette(ui);
		let top = ui.cursor().min.y;
		self.window = screen.as_ref().and_then(|screen| screen.screen.window);
		let Some(screen) = screen else {
			self.playing = None;
			self.lift = 0.0;
			logo(ui);
			ui.add_space(HEADER_GAP);
			self.copy_top = ui.cursor().min.y;
			ui.add(
				egui::Label::new(crate::design::semibold(ui, title, 24.0).color(p.text_strong))
					.truncate(),
			);
			ui.add_space(8.0);
			ui.add(
				egui::Label::new(egui::RichText::new(subtitle).size(15.0).color(p.muted))
					.truncate(),
			);
			self.text = ui.cursor().min.y - self.copy_top;
			return;
		};
		let mut gap = 0.0;
		if screen.images.is_empty() {
			self.playing = None;
		} else {
			self.images(ui, &screen);
			gap = HEADER_GAP;
		}
		self.copy_top = ui.cursor().min.y;
		if let Some(text) = &screen.screen.title {
			ui.add_space(gap);
			ui.add(
				egui::Label::new(crate::design::semibold(ui, text, 24.0).color(p.text_strong))
					.truncate(),
			);
			gap = 8.0;
		}
		if let Some(text) = &screen.screen.subtitle {
			ui.add_space(gap);
			ui.add(
				egui::Label::new(egui::RichText::new(text).size(15.0).color(p.muted)).truncate(),
			);
		}
		self.text = ui.cursor().min.y - self.copy_top;
		self.lift = ((ui.cursor().min.y - top - DEFAULT_HEADER) * 0.5).max(0.0);
	}

	/// Draws the images flush, top to bottom, at their own cell size or scaled down to what the
	/// column leaves above the copy, bar and button, and shows each image's current frame.
	fn images(&mut self, ui: &mut egui::Ui, screen: &Arc<DecodedStartScreen>) {
		let now = ui.input(|input| input.time);
		let accent = crate::design::palette(ui).accent;
		if self
			.playing
			.as_ref()
			.is_none_or(|playing| !Arc::ptr_eq(&playing.screen, screen) || playing.accent != accent)
		{
			// An accent change re-colors the textures without restarting the animation.
			let start = self
				.playing
				.as_ref()
				.filter(|playing| Arc::ptr_eq(&playing.screen, screen))
				.map_or(now, |playing| playing.start);
			let textures = screen
				.images
				.iter()
				.map(|image| {
					let texture = ui.ctx().load_texture(
						"start-screen-image",
						image.pixels(0, accent),
						egui::TextureOptions::NEAREST,
					);
					(texture, 0)
				})
				.collect();
			self.playing = Some(Playing {
				screen: screen.clone(),
				start,
				accent,
				textures,
			});
		}
		let Some(playing) = &mut self.playing else {
			return;
		};
		let below = if self.below > 0.0 {
			self.below
		} else {
			BELOW_ESTIMATE
		};
		let text = if self.text > 0.0 {
			self.text
		} else {
			TEXT_ESTIMATE
		};
		// Everything but the images, measured from where they start, so a short window
		// shrinks them — to nothing if it must — never a control below.
		let room = ui.max_rect().bottom() - ui.cursor().min.y - text - below - BOTTOM_PADDING;
		let pixels_per_point = ui.pixels_per_point();
		// A cell of whole physical pixels prints every cell of an image the same size.
		let crisp = |image: &Image| (image.cell * pixels_per_point).round().max(1.0);
		let natural = stack(&screen.images, |image| crisp(image) / pixels_per_point);
		let scale = (ui.available_width() / natural.x)
			.min(room / natural.y)
			.min(1.0);
		if scale <= 0.0 {
			return;
		}
		// Scaled cells stay whole physical pixels until they would fall under one.
		let scaled = |image: &Image| {
			let pixels = crisp(image) * scale;
			if pixels >= 1.0 {
				pixels.floor() / pixels_per_point
			} else {
				pixels / pixels_per_point
			}
		};
		let (rect, _) = ui.allocate_exact_size(stack(&screen.images, scaled), egui::Sense::hover());
		let mut top = rect.min.round_to_pixels(pixels_per_point).y;
		let elapsed = now - playing.start;
		let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
		for (image, (texture, shown)) in screen.images.iter().zip(&mut playing.textures) {
			let frame = image.frame(elapsed);
			if *shown != frame {
				texture.set(image.pixels(frame, accent), egui::TextureOptions::NEAREST);
				*shown = frame;
			}
			if let Some(wait) = image.next_frame(elapsed) {
				ui.ctx().request_repaint_after(wait);
			}
			let size = image.extent(scaled(image));
			let min =
				egui::pos2(rect.center().x - size.x * 0.5, top).round_to_pixels(pixels_per_point);
			top += size.y;
			ui.painter().image(
				texture.id(),
				egui::Rect::from_min_size(min, size),
				uv,
				egui::Color32::WHITE,
			);
		}
	}

	/// Releases the image textures when the start screen was not drawn since the previous call.
	pub fn retire(&mut self) {
		if !std::mem::take(&mut self.shown) {
			*self = Self::default();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Checks that frames unpack MSB first into one palette index per cell, stopping at base64
	/// padding and dropping the zero padding of a frame's last byte.
	#[test]
	fn frames_decode_msb_first_into_palette_indices() {
		let image = |width, height, frame: &str| StartImage {
			width,
			height,
			palette: vec!["#ffffff".into()],
			frames: vec![frame.into()],
			fps: 12,
			cell: 3,
		};
		let screen = DecodedStartScreen::new(StartScreen {
			// 0b01_10_11_00: the first cell sits in the two most significant bits.
			// 0b01_01_01_10, 0b11_00_00_00: the second byte holds one cell, then padding.
			images: vec![image(2, 2, "bA=="), image(5, 1, "VsA=")],
			..Default::default()
		})
		.expect("valid screen");
		assert_eq!(screen.images[0].frames, [vec![1, 2, 3, 0]]);
		assert_eq!(screen.images[1].frames, [vec![1, 1, 1, 2, 3]]);
	}
}
