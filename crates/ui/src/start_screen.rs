//! Host rendering of a plugin's start (connecting) screen: its title and subtitle, the logo's
//! visibility, ASCII pixel text and a spinning model, both printed through one 4x4 ordered
//! (Bayer) dither. The model is software rasterized on its own thread at a capped rate into a
//! bounded cell grid; the UI thread only uploads the latest finished frame to one texture.

use egui::emath::GuiRounding;
use extensions::{StartModel, StartScreen, StartTextStyle};
use std::{
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, Ordering},
	},
	time::{Duration, Instant},
};

/// Logical pixels per title text cell. Lit cells are filled pixel by pixel.
const TITLE_CELL: f32 = 4.0;
/// Logical pixels per subtitle text cell.
const SUBTITLE_CELL: f32 = 2.0;
/// Logical pixels per model cell.
const MODEL_CELL: f32 = 3.0;
/// Largest model grid edge in cells, bounding each frame's raster and upload.
const MAX_MODEL_CELLS: usize = 160;
/// Smallest model grid edge worth drawing; a shorter window drops the model, never a control.
const MIN_MODEL_CELLS: usize = 16;
/// Largest model edge on the start screen, in logical pixels.
const MAX_MODEL_SIDE: f32 = 240.0;
/// Least space the restore screen leaves above its header, in logical pixels.
const MIN_TOP: f32 = 12.0;
/// Gap under the model, in logical pixels.
const MODEL_GAP: f32 = 16.0;
/// Space kept under the screen's last control, at least the gaps around the progress bar.
const BOTTOM_PADDING: f32 = 32.0;
/// Height of the copy, progress bar and button assumed before the first frame measures it.
const BELOW_ESTIMATE: f32 = 200.0;
/// Model frame interval: at most 30 frames per second.
const FRAME: Duration = Duration::from_millis(33);
/// Opacity (0.18) of covered model cells and glyph pixels the dither leaves unlit.
const SHADE_ALPHA: u8 = 46;
/// Brightness lost from the top to the bottom row of each pixel-text line.
const TEXT_RAMP: f32 = 0.45;
/// Approximate height of the default logo, title and stage copy, in logical pixels.
const DEFAULT_HEADER: f32 = 124.0;
/// Glyph rows: seven down to the baseline and two for descenders.
const GLYPH_ROWS: usize = 9;
/// Text line pitch in cells: one glyph and a two-cell gap.
const LINE_ROWS: usize = GLYPH_ROWS + 2;
/// Camera distance from the unit sphere's center; it keeps the sphere inside the field of view.
const CAMERA: f32 = 3.1;
/// Focal length of the 40-degree field of view: 1 / tan(20°).
const FOCAL: f32 = 2.747_477;
/// Unit direction toward the light, from the upper left in front of the model.
const LIGHT: [f32; 3] = [-0.453_4, 0.654_9, 0.604_5];
/// Light reaching faces turned away from the light source.
const AMBIENT: f32 = 0.22;

/// 4x4 ordered-dither thresholds, in sixteenths.
const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// 5x9 glyphs for ASCII 0x20 through 0x7E: seven rows down to the baseline and two descender
/// rows; each row's bit 4 is the leftmost column.
const GLYPHS: [[u8; 9]; 95] = [
	[0, 0, 0, 0, 0, 0, 0, 0, 0],                         // space
	[0x04, 0x04, 0x04, 0x04, 0x04, 0, 0x04, 0, 0],       // !
	[0x0A, 0x0A, 0, 0, 0, 0, 0, 0, 0],                   // "
	[0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A, 0, 0],    // #
	[0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04, 0, 0],    // $
	[0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03, 0, 0],    // %
	[0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D, 0, 0],    // &
	[0x04, 0x04, 0x08, 0, 0, 0, 0, 0, 0],                // '
	[0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02, 0, 0],    // (
	[0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08, 0, 0],    // )
	[0, 0x04, 0x15, 0x0E, 0x15, 0x04, 0, 0, 0],          // *
	[0, 0x04, 0x04, 0x1F, 0x04, 0x04, 0, 0, 0],          // +
	[0, 0, 0, 0, 0, 0x0C, 0x0C, 0x04, 0x08],             // ,
	[0, 0, 0, 0x1F, 0, 0, 0, 0, 0],                      // -
	[0, 0, 0, 0, 0, 0x0C, 0x0C, 0, 0],                   // .
	[0, 0x01, 0x02, 0x04, 0x08, 0x10, 0, 0, 0],          // /
	[0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E, 0, 0],    // 0
	[0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E, 0, 0],    // 1
	[0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F, 0, 0],    // 2
	[0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E, 0, 0],    // 3
	[0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02, 0, 0],    // 4
	[0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E, 0, 0],    // 5
	[0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E, 0, 0],    // 6
	[0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08, 0, 0],    // 7
	[0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E, 0, 0],    // 8
	[0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C, 0, 0],    // 9
	[0, 0x0C, 0x0C, 0, 0x0C, 0x0C, 0, 0, 0],             // :
	[0, 0x0C, 0x0C, 0, 0, 0x0C, 0x0C, 0x04, 0x08],       // ;
	[0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02, 0, 0],    // <
	[0, 0, 0x1F, 0, 0x1F, 0, 0, 0, 0],                   // =
	[0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08, 0, 0],    // >
	[0x0E, 0x11, 0x01, 0x02, 0x04, 0, 0x04, 0, 0],       // ?
	[0x0E, 0x11, 0x01, 0x0D, 0x15, 0x15, 0x0E, 0, 0],    // @
	[0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11, 0, 0],    // A
	[0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E, 0, 0],    // B
	[0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E, 0, 0],    // C
	[0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C, 0, 0],    // D
	[0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F, 0, 0],    // E
	[0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10, 0, 0],    // F
	[0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F, 0, 0],    // G
	[0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11, 0, 0],    // H
	[0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E, 0, 0],    // I
	[0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C, 0, 0],    // J
	[0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11, 0, 0],    // K
	[0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F, 0, 0],    // L
	[0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11, 0, 0],    // M
	[0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0, 0],    // N
	[0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E, 0, 0],    // O
	[0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10, 0, 0],    // P
	[0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D, 0, 0],    // Q
	[0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11, 0, 0],    // R
	[0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E, 0, 0],    // S
	[0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0, 0],    // T
	[0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E, 0, 0],    // U
	[0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04, 0, 0],    // V
	[0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A, 0, 0],    // W
	[0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11, 0, 0],    // X
	[0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0, 0],    // Y
	[0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F, 0, 0],    // Z
	[0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E, 0, 0],    // [
	[0, 0x10, 0x08, 0x04, 0x02, 0x01, 0, 0, 0],          // backslash
	[0x0E, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0E, 0, 0],    // ]
	[0x04, 0x0A, 0x11, 0, 0, 0, 0, 0, 0],                // ^
	[0, 0, 0, 0, 0, 0, 0, 0x1F, 0],                      // _
	[0x08, 0x04, 0x02, 0, 0, 0, 0, 0, 0],                // `
	[0, 0, 0x0E, 0x01, 0x0F, 0x11, 0x0F, 0, 0],          // a
	[0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x1E, 0, 0],    // b
	[0, 0, 0x0E, 0x10, 0x10, 0x11, 0x0E, 0, 0],          // c
	[0x01, 0x01, 0x0D, 0x13, 0x11, 0x11, 0x0F, 0, 0],    // d
	[0, 0, 0x0E, 0x11, 0x1F, 0x10, 0x0E, 0, 0],          // e
	[0x06, 0x09, 0x08, 0x1C, 0x08, 0x08, 0x08, 0, 0],    // f
	[0, 0, 0x0F, 0x11, 0x11, 0x11, 0x0F, 0x01, 0x0E],    // g
	[0x10, 0x10, 0x16, 0x19, 0x11, 0x11, 0x11, 0, 0],    // h
	[0x04, 0, 0x0C, 0x04, 0x04, 0x04, 0x0E, 0, 0],       // i
	[0x02, 0, 0x06, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C], // j
	[0x10, 0x10, 0x12, 0x14, 0x18, 0x14, 0x12, 0, 0],    // k
	[0x0C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E, 0, 0],    // l
	[0, 0, 0x1A, 0x15, 0x15, 0x11, 0x11, 0, 0],          // m
	[0, 0, 0x16, 0x19, 0x11, 0x11, 0x11, 0, 0],          // n
	[0, 0, 0x0E, 0x11, 0x11, 0x11, 0x0E, 0, 0],          // o
	[0, 0, 0x1E, 0x11, 0x11, 0x11, 0x1E, 0x10, 0x10],    // p
	[0, 0, 0x0F, 0x11, 0x11, 0x11, 0x0F, 0x01, 0x01],    // q
	[0, 0, 0x16, 0x19, 0x10, 0x10, 0x10, 0, 0],          // r
	[0, 0, 0x0F, 0x10, 0x0E, 0x01, 0x1E, 0, 0],          // s
	[0x08, 0x08, 0x1C, 0x08, 0x08, 0x09, 0x06, 0, 0],    // t
	[0, 0, 0x11, 0x11, 0x11, 0x13, 0x0D, 0, 0],          // u
	[0, 0, 0x11, 0x11, 0x11, 0x0A, 0x04, 0, 0],          // v
	[0, 0, 0x11, 0x11, 0x15, 0x15, 0x0A, 0, 0],          // w
	[0, 0, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0, 0],          // x
	[0, 0, 0x11, 0x11, 0x11, 0x11, 0x0F, 0x01, 0x0E],    // y
	[0, 0, 0x1F, 0x02, 0x04, 0x08, 0x1F, 0, 0],          // z
	[0x02, 0x04, 0x04, 0x08, 0x04, 0x04, 0x02, 0, 0],    // {
	[0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0, 0],    // |
	[0x08, 0x04, 0x04, 0x02, 0x04, 0x04, 0x08, 0, 0],    // }
	[0, 0, 0x08, 0x15, 0x02, 0, 0, 0, 0],                // ~
];
/// Outline drawn for every character outside printable ASCII.
const MISSING: [u8; 9] = [0x1F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1F, 0, 0];

/// Start-screen rendering state: cached text rasters and, while a model shows, its renderer.
/// Draw with [`StartScreenView::header`] and call [`StartScreenView::retire`] once per frame
/// afterwards, so the model thread and textures exist only while the start screen is visible.
#[derive(Default)]
pub struct StartScreenView {
	title: TextRaster,
	subtitle: TextRaster,
	model: Option<ModelView>,
	lift: f32,
	/// Where this frame's copy starts, and how far the copy, bar and button reached below it
	/// on the previous frame; the model is sized so all of it fits above the bottom padding.
	copy_top: f32,
	below: f32,
	compact: bool,
	shown: bool,
}

impl StartScreenView {
	/// How far to raise the column so a taller custom header stays centred. It is measured on
	/// the previous frame, which the restore screen's continuous repaint keeps current.
	pub fn lift(&self) -> f32 {
		self.lift
	}

	/// Whether the start screen drawn this frame asked for the compact splash window.
	pub fn compact(&self) -> bool {
		self.shown && self.compact
	}

	/// Records where the screen's last control ended, after the caller drew everything below
	/// the header.
	pub fn end(&mut self, ui: &egui::Ui) {
		self.below = ui.cursor().min.y - self.copy_top;
	}

	/// Draws the logo, the plugin's model and the title and subtitle into the current top-down
	/// column. Without a plugin screen this is the client's own logo and copy. A plugin title
	/// replaces both default lines and a plugin subtitle is then the only second line; without
	/// a title, a plugin subtitle replaces just the stage line. The ASCII pixel font applies only
	/// to plugin text, so localized client copy keeps the normal typography.
	pub fn header(
		&mut self,
		ui: &mut egui::Ui,
		screen: Option<Arc<StartScreen>>,
		logo: impl FnOnce(&mut egui::Ui),
		title: &str,
		subtitle: &str,
	) {
		self.shown = true;
		let p = crate::design::palette(ui);
		let top = ui.cursor().min.y;
		let custom = screen.as_deref();
		self.compact = custom.is_some_and(|screen| screen.compact);
		if !custom.is_some_and(|screen| screen.hide_logo) {
			logo(ui);
			ui.add_space(20.0);
		}
		let tint = custom
			.and_then(|screen| screen.model.as_ref())
			.and_then(|model| model.color.as_deref())
			.and_then(rgb);
		let below = if self.below > 0.0 {
			self.below
		} else {
			BELOW_ESTIMATE
		};
		// Everything but the model, so a short window shrinks or drops the model, never a control.
		let reserve = MIN_TOP
			+ (ui.cursor().min.y - top)
			+ MODEL_GAP
			+ ui.spacing().item_spacing.y
			+ below + BOTTOM_PADDING;
		self.show_model(ui, screen.as_ref(), tint.unwrap_or(p.accent), reserve);
		self.copy_top = ui.cursor().min.y;
		let dither = custom.is_some_and(|screen| screen.text_style == StartTextStyle::Dither);
		let plugin_title = custom.and_then(|screen| screen.title.as_deref());
		let plugin_subtitle = custom.and_then(|screen| screen.subtitle.as_deref());
		match plugin_title {
			Some(text) if dither => {
				self.title
					.show(ui, text, TITLE_CELL, tint.unwrap_or(p.text_strong));
			}
			text => {
				ui.label(
					crate::design::semibold(ui, text.unwrap_or(title), 24.0).color(p.text_strong),
				);
			}
		}
		match (plugin_title, plugin_subtitle) {
			(_, Some(text)) => {
				ui.add_space(8.0);
				if dither {
					self.subtitle
						.show(ui, text, SUBTITLE_CELL, tint.unwrap_or(p.muted));
				} else {
					ui.label(egui::RichText::new(text).size(15.0).color(p.muted));
				}
			}
			(None, None) => {
				ui.add_space(8.0);
				ui.label(egui::RichText::new(subtitle).size(15.0).color(p.muted));
			}
			(Some(_), None) => {}
		}
		self.lift = if custom.is_some() {
			((ui.cursor().min.y - top - DEFAULT_HEADER) * 0.5).max(0.0)
		} else {
			0.0
		};
	}

	/// Keeps one renderer for the shown model, sized to what the column leaves after `reserve`,
	/// and draws its latest frame.
	fn show_model(
		&mut self,
		ui: &mut egui::Ui,
		screen: Option<&Arc<StartScreen>>,
		tint: egui::Color32,
		reserve: f32,
	) {
		let Some(screen) = screen.filter(|screen| screen.model.is_some()) else {
			self.model = None;
			return;
		};
		let cell = physical_cell(ui, MODEL_CELL);
		let side = (ui.max_rect().height() - reserve)
			.min(ui.available_width() * 0.6)
			.min(MAX_MODEL_SIDE);
		let cells = ((side / cell).max(0.0) as usize).min(MAX_MODEL_CELLS);
		if cells < MIN_MODEL_CELLS {
			self.model = None;
			return;
		}
		// The app follows egui's animation time; zero means reduced motion, so the model holds.
		let still = ui.style().animation_time <= 0.0;
		if !self
			.model
			.as_ref()
			.is_some_and(|view| view.matches(screen, cells, still))
		{
			// A resized model keeps showing its last frame until the new renderer's lands.
			let texture = self
				.model
				.as_mut()
				.filter(|view| Arc::ptr_eq(&view.screen, screen))
				.and_then(|view| view.texture.take());
			let mut view = ModelView::start(ui.ctx(), screen.clone(), cells, still);
			view.texture = texture;
			self.model = Some(view);
		}
		if let Some(view) = &mut self.model {
			view.show(ui, cells as f32 * cell, tint);
		}
		ui.add_space(16.0);
	}

	/// Releases the model thread and textures when the start screen was not drawn since the
	/// previous call.
	pub fn retire(&mut self) {
		if !std::mem::take(&mut self.shown) {
			*self = Self::default();
		}
	}
}

/// Parses a validated `#RRGGBB` color.
fn rgb(value: &str) -> Option<egui::Color32> {
	let [_, r, g, b] = u32::from_str_radix(value.strip_prefix('#')?, 16)
		.ok()?
		.to_be_bytes();
	Some(egui::Color32::from_rgb(r, g, b))
}

/// A cell edge rounded to whole physical pixels, so every cell of a raster prints the same size.
fn physical_cell(ui: &egui::Ui, logical: f32) -> f32 {
	let pixels_per_point = ui.pixels_per_point();
	(logical * pixels_per_point).round().max(1.0) / pixels_per_point
}

/// Draws a cell texture tinted by `tint`, aligned to the physical pixel grid.
fn paint(ui: &egui::Ui, texture: &egui::TextureHandle, rect: egui::Rect, tint: egui::Color32) {
	let rect =
		egui::Rect::from_min_size(rect.min.round_to_pixels(ui.pixels_per_point()), rect.size());
	let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
	ui.painter().image(texture.id(), rect, uv, tint);
}

/// The ordered-dither threshold of a cell or pixel, between 0 and 1.
fn threshold(x: usize, y: usize) -> f32 {
	(f32::from(BAYER[y % 4][x % 4]) + 0.5) / 16.0
}

/// A cell or pixel the dither lights fully or leaves as a faint shade, as white for tinting.
fn dithered(brightness: f32, x: usize, y: usize) -> egui::Color32 {
	if brightness > threshold(x, y) {
		egui::Color32::WHITE
	} else {
		egui::Color32::from_white_alpha(SHADE_ALPHA)
	}
}

fn glyph(character: char) -> &'static [u8; 9] {
	match character {
		' '..='~' => &GLYPHS[character as usize - 0x20],
		_ => &MISSING,
	}
}

/// Greedy word wrap into lines of at most `columns` characters; longer words break.
fn wrap(text: &str, columns: usize) -> Vec<Vec<char>> {
	let mut lines = Vec::new();
	let mut line = Vec::new();
	for word in text.split(' ') {
		let word: Vec<char> = word.chars().collect();
		if line.len() + usize::from(!line.is_empty()) + word.len() <= columns {
			if !line.is_empty() {
				line.push(' ');
			}
			line.extend(word);
			continue;
		}
		if !line.is_empty() {
			lines.push(std::mem::take(&mut line));
		}
		let mut chunks = word.chunks(columns.max(1)).peekable();
		while let Some(chunk) = chunks.next() {
			if chunks.peek().is_some() {
				lines.push(chunk.to_vec());
			} else {
				line.extend_from_slice(chunk);
			}
		}
	}
	if !line.is_empty() || lines.is_empty() {
		lines.push(line);
	}
	lines
}

/// Prints wrapped lines in the pixel font with `cell` physical pixels per glyph cell. Each lit
/// cell is filled pixel by pixel with the 4x4 Bayer pattern over a brightness ramp down the
/// line, so glyph shapes stay whole while their fill dithers; unlit pixels keep a faint shade.
fn text_image(lines: &[Vec<char>], cell: usize) -> egui::ColorImage {
	let columns = lines.iter().map(Vec::len).max().unwrap_or(0).max(1) * 6 - 1;
	let rows = lines.len() * LINE_ROWS - 2;
	let (width, height) = (columns * cell, rows * cell);
	let mut pixels = vec![egui::Color32::TRANSPARENT; width * height];
	for (number, line) in lines.iter().enumerate() {
		for (index, character) in line.iter().enumerate() {
			for (row, bits) in glyph(*character).iter().enumerate() {
				let brightness = 1.0 - TEXT_RAMP * row as f32 / (GLYPH_ROWS - 1) as f32;
				let top = (number * LINE_ROWS + row) * cell;
				for column in (0..5).filter(|column| bits & (0x10 >> column) != 0) {
					let left = (index * 6 + column) * cell;
					for y in top..top + cell {
						for x in left..left + cell {
							pixels[y * width + x] = dithered(brightness, x, y);
						}
					}
				}
			}
		}
	}
	egui::ColorImage::new([width, height], pixels)
}

/// A pixel-text texture at physical resolution, rebuilt only when its text, wrap width or
/// display scale changes.
#[derive(Default)]
struct TextRaster {
	key: Option<(String, usize, usize)>,
	texture: Option<egui::TextureHandle>,
}

impl TextRaster {
	fn show(&mut self, ui: &mut egui::Ui, text: &str, cell: f32, tint: egui::Color32) {
		let pixels_per_point = ui.pixels_per_point();
		let cell_pixels = (cell * pixels_per_point).round().max(1.0) as usize;
		let cell = cell_pixels as f32 / pixels_per_point;
		let columns = ((ui.available_width() + cell) / (6.0 * cell)).max(1.0) as usize;
		if self.key.as_ref().is_none_or(|(shown, wrapped, pixels)| {
			shown != text || *wrapped != columns || *pixels != cell_pixels
		}) {
			let image = text_image(&wrap(text, columns), cell_pixels);
			match &mut self.texture {
				Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
				None => {
					self.texture = Some(ui.ctx().load_texture(
						"start-screen-text",
						image,
						egui::TextureOptions::NEAREST,
					));
				}
			}
			self.key = Some((text.to_owned(), columns, cell_pixels));
		}
		let Some(texture) = &self.texture else {
			return;
		};
		let (rect, response) = ui.allocate_exact_size(
			texture.resolution_vec2() / pixels_per_point,
			egui::Sense::hover(),
		);
		response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
		paint(ui, texture, rect, tint);
	}
}

/// The newest finished model frame, replaced unread when the UI falls behind.
#[derive(Default)]
struct Frames {
	latest: Mutex<Option<egui::ColorImage>>,
	stop: AtomicBool,
}

/// One model's renderer thread and the texture its frames upload into.
struct ModelView {
	screen: Arc<StartScreen>,
	cells: usize,
	still: bool,
	frames: Arc<Frames>,
	worker: Option<std::thread::Thread>,
	texture: Option<egui::TextureHandle>,
}

impl ModelView {
	fn start(ctx: &egui::Context, screen: Arc<StartScreen>, cells: usize, still: bool) -> Self {
		let frames = Arc::new(Frames::default());
		let worker = {
			let (screen, frames, ctx) = (screen.clone(), frames.clone(), ctx.clone());
			std::thread::Builder::new()
				.name("serein-start-screen".into())
				.spawn(move || render(&screen, cells, still, &frames, &ctx))
				.ok()
				.map(|handle| handle.thread().clone())
		};
		Self {
			screen,
			cells,
			still,
			frames,
			worker,
			texture: None,
		}
	}

	fn matches(&self, screen: &Arc<StartScreen>, cells: usize, still: bool) -> bool {
		Arc::ptr_eq(&self.screen, screen) && self.cells == cells && self.still == still
	}

	fn show(&mut self, ui: &mut egui::Ui, side: f32, tint: egui::Color32) {
		let frame = self
			.frames
			.latest
			.lock()
			.ok()
			.and_then(|mut latest| latest.take());
		if let Some(image) = frame {
			match &mut self.texture {
				Some(texture) => texture.set(image, egui::TextureOptions::NEAREST),
				None => {
					self.texture = Some(ui.ctx().load_texture(
						"start-screen-model",
						image,
						egui::TextureOptions::NEAREST,
					));
				}
			}
		}
		let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
		if let Some(texture) = &self.texture {
			paint(ui, texture, rect, tint);
		}
	}
}

impl Drop for ModelView {
	fn drop(&mut self) {
		self.frames.stop.store(true, Ordering::Release);
		if let Some(worker) = &self.worker {
			worker.unpark();
		}
	}
}

/// Renders frames until stopped: one for a still model, otherwise at most 30 per second, and
/// none while the previous frame is still unread.
fn render(screen: &StartScreen, cells: usize, still: bool, frames: &Frames, ctx: &egui::Context) {
	let Some(model) = &screen.model else {
		return;
	};
	let mesh = Mesh::fit(model);
	let mut raster = Raster::new(cells, cells);
	let spin = if still { 0.0 } else { f64::from(model.spin) };
	let pitch = model.pitch.to_radians();
	let start = Instant::now();
	loop {
		let next = Instant::now() + FRAME;
		if frames.latest.lock().is_ok_and(|latest| latest.is_none()) {
			let yaw = (start.elapsed().as_secs_f64() * spin).rem_euclid(360.0);
			raster.draw(&mesh, yaw.to_radians() as f32, pitch);
			let image = raster.image();
			if let Ok(mut latest) = frames.latest.lock() {
				*latest = Some(image);
			}
			ctx.request_repaint();
			if spin == 0.0 {
				return;
			}
		}
		loop {
			if frames.stop.load(Ordering::Acquire) {
				return;
			}
			let now = Instant::now();
			if now >= next {
				break;
			}
			std::thread::park_timeout(next - now);
		}
	}
}

/// A model centered on its bounds and scaled into the unit sphere, so every rotation fits the
/// frame without clipping.
struct Mesh<'a> {
	vertices: Vec<[f32; 3]>,
	triangles: &'a [[u16; 3]],
}

impl<'a> Mesh<'a> {
	fn fit(model: &'a StartModel) -> Self {
		let (low, high) = model.vertices.iter().fold(
			([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
			|(low, high), vertex| {
				(
					std::array::from_fn(|axis| low[axis].min(vertex[axis])),
					std::array::from_fn(|axis| high[axis].max(vertex[axis])),
				)
			},
		);
		let center: [f32; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) * 0.5);
		let radius = model
			.vertices
			.iter()
			.map(|vertex| length(sub(*vertex, center)))
			.fold(0.0, f32::max);
		let scale = if radius > 0.0 { radius.recip() } else { 1.0 };
		Self {
			vertices: model
				.vertices
				.iter()
				.map(|vertex| sub(*vertex, center).map(|value| value * scale))
				.collect(),
			triangles: &model.triangles,
		}
	}
}

/// A bounded cell grid holding the nearest surface's lighting per cell, reused across frames.
struct Raster {
	columns: usize,
	rows: usize,
	/// Inverse view depth of the nearest surface; zero where the model leaves a cell empty.
	depth: Vec<f32>,
	light: Vec<f32>,
	view: Vec<[f32; 3]>,
}

impl Raster {
	fn new(columns: usize, rows: usize) -> Self {
		Self {
			columns,
			rows,
			depth: vec![0.0; columns * rows],
			light: vec![0.0; columns * rows],
			view: Vec::new(),
		}
	}

	/// Draws the mesh turned by `yaw` about its vertical axis and tilted by `pitch` toward the
	/// viewer, with perspective, a depth buffer and two-sided Lambert plus ambient lighting.
	fn draw(&mut self, mesh: &Mesh, yaw: f32, pitch: f32) {
		self.depth.fill(0.0);
		let (yaw_sin, yaw_cos) = yaw.sin_cos();
		let (pitch_sin, pitch_cos) = pitch.sin_cos();
		self.view.clear();
		self.view.extend(mesh.vertices.iter().map(|&[x, y, z]| {
			let (x, z) = (x * yaw_cos + z * yaw_sin, z * yaw_cos - x * yaw_sin);
			[
				x,
				y * pitch_cos - z * pitch_sin,
				y * pitch_sin + z * pitch_cos,
			]
		}));
		let scale = self.columns.min(self.rows) as f32 * 0.5 * FOCAL;
		let (center_x, center_y) = (self.columns as f32 * 0.5, self.rows as f32 * 0.5);
		for triangle in mesh.triangles {
			let [Some(&a), Some(&b), Some(&c)] =
				triangle.map(|index| self.view.get(usize::from(index)))
			else {
				continue;
			};
			let normal = cross(sub(b, a), sub(c, a));
			let size = length(normal);
			if size <= f32::EPSILON {
				continue;
			}
			// Light the side facing the camera, whatever the winding.
			let facing = dot(normal, [-a[0], -a[1], CAMERA - a[2]]).signum();
			let light = AMBIENT + (1.0 - AMBIENT) * (dot(normal, LIGHT) * facing / size).max(0.0);
			let project = |vertex: [f32; 3]| {
				let inverse = (CAMERA - vertex[2]).recip();
				[
					center_x + vertex[0] * inverse * scale,
					center_y - vertex[1] * inverse * scale,
					inverse,
				]
			};
			let points = [project(a), project(b), project(c)];
			let area = edge(points[0], points[1], [points[2][0], points[2][1]]);
			if area.abs() <= f32::EPSILON {
				continue;
			}
			let (left, right) = span(points.map(|point| point[0]), self.columns);
			let (top, bottom) = span(points.map(|point| point[1]), self.rows);
			for y in top..bottom {
				for x in left..right {
					let cell = [x as f32 + 0.5, y as f32 + 0.5];
					let weights = [
						edge(points[1], points[2], cell) / area,
						edge(points[2], points[0], cell) / area,
						edge(points[0], points[1], cell) / area,
					];
					if weights.iter().any(|weight| *weight < 0.0) {
						continue;
					}
					let depth = weights[0] * points[0][2]
						+ weights[1] * points[1][2]
						+ weights[2] * points[2][2];
					let index = y * self.columns + x;
					if depth > self.depth[index] {
						self.depth[index] = depth;
						self.light[index] = light;
					}
				}
			}
		}
	}

	/// Thresholds each covered cell's lighting against the Bayer pattern: lit cells are opaque,
	/// the rest of the silhouette a faint shade and uncovered cells transparent.
	fn image(&self) -> egui::ColorImage {
		let pixels = self
			.depth
			.iter()
			.zip(&self.light)
			.enumerate()
			.map(|(index, (depth, light))| {
				if *depth > 0.0 {
					dithered(*light, index % self.columns, index / self.columns)
				} else {
					egui::Color32::TRANSPARENT
				}
			})
			.collect();
		egui::ColorImage::new([self.columns, self.rows], pixels)
	}
}

/// The cell range a projected coordinate span touches, clamped to the grid.
fn span(values: [f32; 3], limit: usize) -> (usize, usize) {
	let low = values.into_iter().fold(f32::INFINITY, f32::min);
	let high = values.into_iter().fold(f32::NEG_INFINITY, f32::max);
	(
		(low.floor().max(0.0) as usize).min(limit),
		(high.ceil().max(0.0) as usize).min(limit),
	)
}

/// Twice the signed area of the triangle `a`, `b`, `p` in screen space.
fn edge(a: [f32; 3], b: [f32; 3], p: [f32; 2]) -> f32 {
	(b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[
		a[1] * b[2] - a[2] * b[1],
		a[2] * b[0] - a[0] * b[2],
		a[0] * b[1] - a[1] * b[0],
	]
}

fn length(a: [f32; 3]) -> f32 {
	dot(a, a).sqrt()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Checks the software rasterizer against a known pose: a cube fitted into the frame,
	/// turned and tilted, covers the center cell, leaves every corner empty, and shows more
	/// than one lighting level across its visible faces.
	#[test]
	fn cube_at_a_fixed_pose_covers_the_center_and_leaves_corners_empty() {
		let model = StartModel {
			vertices: vec![
				[-1.0, -1.0, -1.0],
				[1.0, -1.0, -1.0],
				[1.0, 1.0, -1.0],
				[-1.0, 1.0, -1.0],
				[-1.0, -1.0, 1.0],
				[1.0, -1.0, 1.0],
				[1.0, 1.0, 1.0],
				[-1.0, 1.0, 1.0],
			],
			triangles: vec![
				[0, 2, 1],
				[0, 3, 2],
				[4, 5, 6],
				[4, 6, 7],
				[0, 1, 5],
				[0, 5, 4],
				[3, 6, 2],
				[3, 7, 6],
				[0, 4, 7],
				[0, 7, 3],
				[1, 2, 6],
				[1, 6, 5],
			],
			color: None,
			spin: 12.0,
			pitch: 20.0,
		};
		let mesh = Mesh::fit(&model);
		let mut raster = Raster::new(32, 32);
		raster.draw(&mesh, 30f32.to_radians(), 20f32.to_radians());
		let covered = |x: usize, y: usize| raster.depth[y * raster.columns + x] > 0.0;
		assert!(covered(16, 16), "center cell covered");
		for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31)] {
			assert!(!covered(x, y), "corner ({x}, {y}) empty");
		}
		let mut levels: Vec<f32> = (0..32 * 32)
			.filter(|index| raster.depth[*index] > 0.0)
			.map(|index| raster.light[index])
			.collect();
		levels.sort_by(f32::total_cmp);
		levels.dedup();
		assert!(levels.len() > 1, "visible faces are lit differently");
	}
}
