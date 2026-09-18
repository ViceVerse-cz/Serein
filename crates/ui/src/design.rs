//! Serein theme tokens, presets and typography shared by every native view.
//!
//! The palette is resolved from egui's light/dark mode plus a process-wide [`Variant`]
//! (the cool Serein neutrals, deep black, blue-grey, or a gradient recolour). Gradient
//! variants paint a backdrop under translucent surfaces; see [`paint_backdrop`].
use egui::{Color32, FontFamily, FontId, RichText, Stroke, epaint::FontColorTransferFunction};
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// Tooltip text that is only formatted while the tooltip is actually shown.
///
/// `Response::on_hover_text(format!(..))` evaluates its argument every frame for every widget;
/// lists of messages, reactions and channels otherwise allocate a string per row per frame.
pub trait LazyHover {
	fn on_hover_text_with(self, text: impl FnOnce() -> String) -> Self;
}
impl LazyHover for egui::Response {
	fn on_hover_text_with(self, text: impl FnOnce() -> String) -> Self {
		self.on_hover_ui(|ui| {
			// Same layout as `on_hover_text`: keep dynamic tooltips from shrinking (egui #5167).
			ui.set_max_width(ui.spacing().tooltip_width);
			ui.label(text());
		})
	}
}

pub fn rail_name(response: &egui::Response, name: impl AsRef<str>) {
	let name = name.as_ref();
	if name.is_empty() {
		return;
	}
	let dragging = response
		.ctx
		.input(|input| input.pointer.is_decidedly_dragging());
	if dragging {
		return;
	}
	if !response.contains_pointer() && !response.hovered() && !response.has_focus() {
		return;
	}
	let ctx = &response.ctx;
	let style = ctx.style_of(ctx.theme());
	let painter = ctx.layer_painter(egui::LayerId::new(
		egui::Order::Tooltip,
		response.id.with("rail-name"),
	));
	let font_id = egui::TextStyle::Body.resolve(style.as_ref());
	let text_color = style.visuals.widgets.noninteractive.fg_stroke.color;
	let galley = painter.layout(
		name.to_owned(),
		font_id,
		text_color,
		style.spacing.tooltip_width,
	);
	let margin = style.spacing.menu_margin;
	let size = galley.size() + margin.sum();
	let screen = ctx.content_rect();
	let mut min = egui::pos2(
		response.rect.right() + 8.0,
		response.rect.center().y - size.y * 0.5,
	);
	if min.x + size.x > screen.right() {
		min.x = (response.rect.left() - 8.0 - size.x).max(screen.left());
	}
	min.y = min
		.y
		.clamp(screen.top(), (screen.bottom() - size.y).max(screen.top()));
	let rect = egui::Rect::from_min_size(min, size);
	painter.rect(
		rect,
		style.visuals.menu_corner_radius,
		style.visuals.window_fill(),
		style.visuals.window_stroke(),
		egui::StrokeKind::Inside,
	);
	painter.galley(rect.min + margin.left_top(), galley, text_color);
}

/// Recolour preset layered over the light/dark preference.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum Variant {
	/// The house palette: cool blue-grey surfaces, following the light/dark preference.
	#[default]
	Standard = 0,
	/// Deep black surfaces for OLED displays.
	Eclipse = 1,
	/// Lighter blue-grey surfaces.
	Slate = 2,
	Nightfall = 3,
	Ember = 4,
	Verdant = 5,
	Afterglow = 6,
}
impl Variant {
	pub const ALL: [Variant; 7] = [
		Variant::Standard,
		Variant::Eclipse,
		Variant::Slate,
		Variant::Nightfall,
		Variant::Ember,
		Variant::Verdant,
		Variant::Afterglow,
	];
	pub fn label(self) -> &'static str {
		match self {
			Variant::Standard => "Serein",
			Variant::Eclipse => "Eclipse",
			Variant::Slate => "Slate",
			Variant::Nightfall => "Nightfall",
			Variant::Ember => "Ember",
			Variant::Verdant => "Verdant",
			Variant::Afterglow => "Afterglow",
		}
	}
	/// Stable identifier for persistence.
	pub fn key(self) -> &'static str {
		match self {
			Variant::Standard => "standard",
			Variant::Eclipse => "eclipse",
			Variant::Slate => "slate",
			Variant::Nightfall => "nightfall",
			Variant::Ember => "ember",
			Variant::Verdant => "verdant",
			Variant::Afterglow => "afterglow",
		}
	}
	/// Keys written by builds that shipped the previous theme names.
	fn legacy_key(self) -> &'static str {
		match self {
			Variant::Standard => "standard",
			Variant::Eclipse => "onyx",
			Variant::Slate => "ash",
			Variant::Nightfall => "midnight-blurple",
			Variant::Ember => "crimson-moon",
			Variant::Verdant => "forest",
			Variant::Afterglow => "sunset",
		}
	}
	pub fn from_key(key: &str) -> Option<Variant> {
		Variant::ALL
			.into_iter()
			.find(|v| v.key() == key || v.legacy_key() == key)
	}
	/// Gradient variants ignore the light/dark preference and always use dark text.
	pub fn is_gradient(self) -> bool {
		matches!(
			self,
			Variant::Nightfall | Variant::Ember | Variant::Verdant | Variant::Afterglow
		)
	}
	fn from_u8(value: u8) -> Variant {
		Variant::ALL
			.into_iter()
			.find(|v| *v as u8 == value)
			.unwrap_or_default()
	}
}
static VARIANT: AtomicU8 = AtomicU8::new(0);
static PRIMARY_COLOR: AtomicU32 = AtomicU32::new(0);

pub fn primary_color() -> Option<[u8; 3]> {
	let value = PRIMARY_COLOR.load(Ordering::Relaxed);
	(value != 0).then_some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}
pub fn set_primary_color(primary: Option<[u8; 3]>) {
	// The high byte distinguishes custom black from the default accent.
	PRIMARY_COLOR.store(
		primary.map_or(0, |[r, g, b]| u32::from_be_bytes([1, r, g, b])),
		Ordering::Relaxed,
	);
}
/// Shared swatch and direct #RRGGBB input for app, profile and folder colors.
pub fn color_edit(ui: &mut egui::Ui, color: &mut [u8; 3]) -> egui::Response {
	let swatch = ui.color_edit_button_srgb(color);
	let mut value = u32::from_be_bytes([0, color[0], color[1], color[2]]);
	let hex = ui
		.add(
			egui::DragValue::new(&mut value)
				.range(0..=0xFFFFFF)
				.hexadecimal(6, false, true)
				.prefix("#")
				.custom_parser(|text| parse_hex_color(text).map(f64::from))
				.update_while_editing(false),
		)
		.on_hover_text("Hex color: #RRGGBB. Click to type or paste.");
	if hex.changed() {
		let [_, r, g, b] = value.to_be_bytes();
		*color = [r, g, b];
	}
	swatch | hex
}
fn parse_hex_color(text: &str) -> Option<u32> {
	let text = text.trim();
	let text = text.strip_prefix('#').unwrap_or(text);
	if text.len() != 6 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
		return None;
	}
	u32::from_str_radix(text, 16).ok()
}
pub fn variant() -> Variant {
	Variant::from_u8(VARIANT.load(Ordering::Relaxed))
}
/// Select a variant; call [`apply`] afterwards so egui's own widgets follow it.
pub fn set_variant(variant: Variant) {
	VARIANT.store(variant as u8, Ordering::Relaxed);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Palette {
	/// Window frame: title bar and server rail.
	pub base: Color32,
	/// Channel and member lists.
	pub sidebar: Color32,
	/// Conversation area.
	pub chat: Color32,
	/// Composer, cards, inputs and popovers.
	pub raised: Color32,
	pub hover: Color32,
	pub selected: Color32,
	pub border: Color32,
	/// Headings and author names.
	pub text_strong: Color32,
	pub text: Color32,
	pub muted: Color32,
	pub link: Color32,
	pub accent: Color32,
	pub accent_text: Color32,
	pub positive: Color32,
	pub warning: Color32,
	pub danger: Color32,
	pub mention_bg: Color32,
	pub mention_text: Color32,
	/// Two-stop backdrop gradient (top-left to bottom-right) under translucent surfaces.
	pub backdrop: Option<[Color32; 2]>,
	/// Alias of `chat`, kept for older call sites.
	pub canvas: Color32,
	/// Alias of `sidebar`, kept for older call sites.
	pub surface: Color32,
}
const fn rgb(value: u32) -> Color32 {
	Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}
const fn rgba(value: u32, alpha: u8) -> Color32 {
	Color32::from_rgba_premultiplied(
		((value >> 16) as u8 as u32 * alpha as u32 / 255) as u8,
		((value >> 8) as u8 as u32 * alpha as u32 / 255) as u8,
		(value as u8 as u32 * alpha as u32 / 255) as u8,
		alpha,
	)
}
/// Serein azure: the house accent, packed for call sites that speak in integer colours.
pub const DEFAULT_PRIMARY_RGB: u32 = 0x1a72e8;
pub const DEFAULT_PRIMARY_COLOR: [u8; 3] = [
	(DEFAULT_PRIMARY_RGB >> 16) as u8,
	(DEFAULT_PRIMARY_RGB >> 8) as u8,
	DEFAULT_PRIMARY_RGB as u8,
];
const PRIMARY: Color32 = Color32::from_rgb(
	DEFAULT_PRIMARY_COLOR[0],
	DEFAULT_PRIMARY_COLOR[1],
	DEFAULT_PRIMARY_COLOR[2],
);
const MENTION_BG: Color32 = rgba(DEFAULT_PRIMARY_RGB, 76);
fn dark_common(
	base: Color32,
	sidebar: Color32,
	chat: Color32,
	raised: Color32,
	hover: Color32,
	selected: Color32,
	border: Color32,
) -> Palette {
	Palette {
		base,
		sidebar,
		chat,
		raised,
		hover,
		selected,
		border,
		text_strong: rgb(0xeef1f6),
		text: rgb(0xc9cfdb),
		muted: rgb(0x8b93a5),
		link: rgb(0x54abff),
		accent: PRIMARY,
		accent_text: Color32::WHITE,
		positive: rgb(0x2fb87a),
		warning: rgb(0xe8a33d),
		danger: rgb(0xef5561),
		mention_bg: MENTION_BG,
		mention_text: rgb(0xbcd9ff),
		backdrop: None,
		canvas: chat,
		surface: sidebar,
	}
}
fn gradient(stops: [u32; 2]) -> Palette {
	let mut p = dark_common(
		Color32::from_black_alpha(140),
		Color32::from_black_alpha(90),
		Color32::from_black_alpha(90),
		Color32::from_black_alpha(120),
		Color32::from_white_alpha(18),
		Color32::from_white_alpha(34),
		Color32::from_white_alpha(28),
	);
	p.text = rgb(0xe6eaf2);
	p.muted = rgb(0xafb6c4);
	p.backdrop = Some([rgb(stops[0]), rgb(stops[1])]);
	p
}
pub fn builtin_colors(dark: bool, variant: Variant) -> Palette {
	let palette = match variant {
		Variant::Standard if dark => dark_common(
			rgb(0x0d1016),
			rgb(0x12161f),
			rgb(0x161b25),
			rgb(0x1d2431),
			rgb(0x222a39),
			rgb(0x2b3547),
			rgb(0x212836),
		),
		Variant::Standard => Palette {
			base: rgb(0xdde3ec),
			sidebar: rgb(0xeef1f7),
			chat: Color32::WHITE,
			raised: rgb(0xe6ebf3),
			hover: rgb(0xe3e9f2),
			selected: rgb(0xd1d9e6),
			border: rgb(0xd9e0ea),
			text_strong: rgb(0x0b0f16),
			text: rgb(0x2c3340),
			muted: rgb(0x5b6473),
			link: rgb(0x0b63d6),
			accent: PRIMARY,
			accent_text: Color32::WHITE,
			positive: rgb(0x1c9c63),
			warning: rgb(0xc8860f),
			danger: rgb(0xd23742),
			mention_bg: MENTION_BG,
			mention_text: rgb(0x14508f),
			backdrop: None,
			canvas: Color32::WHITE,
			surface: rgb(0xeef1f7),
		},
		Variant::Eclipse => dark_common(
			Color32::BLACK,
			rgb(0x070708),
			rgb(0x070708),
			rgb(0x141416),
			rgb(0x17171a),
			rgb(0x222226),
			rgb(0x1c1c20),
		),
		Variant::Slate => dark_common(
			rgb(0x1b1f2a),
			rgb(0x262b38),
			rgb(0x2c3140),
			rgb(0x343a4b),
			rgb(0x313747),
			rgb(0x3d4456),
			rgb(0x3a4152),
		),
		Variant::Nightfall => gradient([0x35519f, 0x1b1440]),
		Variant::Ember => gradient([0x8c2340, 0x140609]),
		Variant::Verdant => gradient([0x2b6350, 0x081a15]),
		Variant::Afterglow => gradient([0xd9663c, 0x35205e]),
	};
	customize(palette, primary_color())
}
#[derive(Clone, Copy)]
struct ExtensionPalette {
	colors: [Option<Color32>; 18],
	backdrop: Option<[Color32; 2]>,
	background: Option<extensions::Background>,
}
thread_local! {
	static EXTENSION_THEME: std::cell::Cell<Option<[ExtensionPalette; 2]>> = const { std::cell::Cell::new(None) };
	static EXTENSION_STYLE: std::cell::Cell<extensions::ThemeStyle> = std::cell::Cell::new(extensions::ThemeStyle::default());
	static WINDOW_EFFECTS: std::cell::Cell<(bool, u8, u8, bool)> = const { std::cell::Cell::new((false, 15, 50, false)) };
	static BACKGROUND_IMAGE: std::cell::RefCell<Option<(std::sync::Arc<egui::ColorImage>, egui::TextureHandle)>> = const { std::cell::RefCell::new(None) };
}

pub fn set_window_effects(enabled: bool, transparency: u8, blur: u8, all: bool) {
	WINDOW_EFFECTS.set((enabled, transparency.min(100), blur.min(100), all));
}

pub fn default_window_effects() -> (bool, u8, u8, bool) {
	WINDOW_EFFECTS.get()
}

pub fn window_effects() -> (bool, u8, u8, bool) {
	let defaults = default_window_effects();
	let style = EXTENSION_STYLE.get();
	(
		style.transparency_blur.unwrap_or(defaults.0),
		style.transparency.unwrap_or(defaults.1),
		style.blur.unwrap_or(defaults.2),
		style.transparent_all.unwrap_or(defaults.3),
	)
}
const THEME_FIELDS: [&str; 18] = [
	"base",
	"sidebar",
	"chat",
	"raised",
	"hover",
	"selected",
	"border",
	"text_strong",
	"text",
	"muted",
	"link",
	"accent",
	"accent_text",
	"positive",
	"warning",
	"danger",
	"mention_bg",
	"mention_text",
];
fn extension_palette(theme: &extensions::ThemePalette) -> Option<ExtensionPalette> {
	let color = |text: &str| {
		extensions::parse_color(text)
			.ok()
			.map(|[r, g, b, a]| Color32::from_rgba_unmultiplied(r, g, b, a))
	};
	let mut colors = [None; 18];
	for (name, value) in &theme.colors {
		let index = THEME_FIELDS.iter().position(|field| *field == name)?;
		colors[index] = Some(color(value)?);
	}
	let backdrop = match &theme.backdrop {
		Some([a, b]) => Some([color(a)?, color(b)?]),
		None => None,
	};
	Some(ExtensionPalette {
		colors,
		backdrop,
		background: theme.background,
	})
}
/// Install color and native control overrides; malformed themes reset to built-in appearance.
/// Call [`apply`] after changing this value. No parsing or allocation runs while drawing.
pub fn set_extension_theme(theme: Option<&extensions::Theme>) {
	let theme = theme.filter(|theme| theme.validate().is_ok());
	let palettes = theme.and_then(|theme| {
		Some([
			extension_palette(&theme.light)?,
			extension_palette(&theme.dark)?,
		])
	});
	EXTENSION_THEME.set(palettes);
	EXTENSION_STYLE.set(theme.map_or_else(extensions::ThemeStyle::default, |theme| theme.style));
	if theme.is_none() {
		BACKGROUND_IMAGE.with(|image| *image.borrow_mut() = None);
	}
}

/// Upload only a changed worker-decoded image; slider changes reuse the texture.
pub fn set_background_image(ctx: &egui::Context, image: Option<std::sync::Arc<egui::ColorImage>>) {
	BACKGROUND_IMAGE.with(|current| {
		let mut current = current.borrow_mut();
		if let Some(image) = image {
			if image
				.size
				.iter()
				.any(|side| *side > ctx.input(|input| input.max_texture_side))
			{
				*current = None;
				return;
			}
			if current
				.as_ref()
				.is_some_and(|(old, _)| std::sync::Arc::ptr_eq(old, &image))
			{
				return;
			}
			let texture = ctx.load_texture(
				"theme-background",
				image.clone(),
				egui::TextureOptions::LINEAR,
			);
			*current = Some((image, texture));
		} else {
			*current = None;
		}
	});
}

pub fn has_window_background(ui: &egui::Ui) -> bool {
	BACKGROUND_IMAGE.with(|image| image.borrow().is_some())
		&& EXTENSION_THEME.get().is_some_and(|palettes| {
			palettes[usize::from(ui.visuals().dark_mode)]
				.background
				.unwrap_or_default()
				.target == extensions::BackgroundTarget::Window
		})
}

#[derive(Clone, Copy)]
pub enum ImageSection {
	TopBar,
	ServerList,
	ChannelList,
	MessageList,
	MemberList,
	Composer,
}

/// Cover one window image with a section surface. Only the surface changes opacity.
pub fn section_surface(ui: &egui::Ui, mut color: Color32, section: ImageSection) -> Color32 {
	let (enabled, _, _, all) = window_effects();
	if enabled && !all && !matches!(section, ImageSection::MessageList) {
		color = color.to_opaque();
	}
	if !has_window_background(ui) {
		return color;
	}
	let Some(sections) = EXTENSION_THEME.get().and_then(|palettes| {
		palettes[usize::from(ui.visuals().dark_mode)]
			.background?
			.sections
	}) else {
		return color;
	};
	let opacity = match section {
		ImageSection::TopBar => sections.top_bar,
		ImageSection::ServerList => sections.server_list,
		ImageSection::ChannelList => sections.channel_list,
		ImageSection::MessageList => sections.message_list,
		ImageSection::MemberList => sections.member_list,
		ImageSection::Composer => sections.composer,
	};
	let [r, g, b, _] = color.to_srgba_unmultiplied();
	Color32::from_rgba_unmultiplied(r, g, b, (u16::from(opacity) * 255 / 100) as u8)
}

pub fn has_section_background(ui: &egui::Ui) -> bool {
	has_window_background(ui)
		&& EXTENSION_THEME.get().is_some_and(|palettes| {
			palettes[usize::from(ui.visuals().dark_mode)]
				.background
				.is_some_and(|background| background.sections.is_some())
		})
}

/// Keep large hover rows translucent over a shared background image.
pub fn row_highlight(ui: &egui::Ui, color: Color32, strength: f32) -> Color32 {
	if has_section_background(ui) {
		let [r, g, b, _] = color.to_srgba_unmultiplied();
		Color32::from_rgba_unmultiplied(r, g, b, 48)
	} else {
		color.gamma_multiply(strength)
	}
}

/// A message-area image sits above the chat surface and below message content.
pub fn paint_chat_background(ui: &egui::Ui, rect: egui::Rect) {
	let background = EXTENSION_THEME
		.get()
		.and_then(|palettes| palettes[usize::from(ui.visuals().dark_mode)].background)
		.unwrap_or_default();
	if background.target != extensions::BackgroundTarget::Chat {
		return;
	}
	BACKGROUND_IMAGE.with(|image| {
		if let Some((_, texture)) = image.borrow().as_ref() {
			paint_background_image(ui.painter(), rect, texture, background);
		}
	});
}

/// Draw a centered static image without changing its aspect ratio.
pub fn paint_background_image(
	painter: &egui::Painter,
	rect: egui::Rect,
	texture: &egui::TextureHandle,
	background: extensions::Background,
) {
	if rect.width() <= 0.0 || rect.height() <= 0.0 || background.opacity == 0 {
		return;
	}
	let size = texture.size_vec2();
	let ratio = rect.size() / size;
	let scale = match background.fit {
		extensions::BackgroundFit::Cover => ratio.x.max(ratio.y),
		extensions::BackgroundFit::Contain => ratio.x.min(ratio.y),
	};
	let image_rect = egui::Rect::from_center_size(rect.center(), size * scale);
	painter
		.with_clip_rect(rect.intersect(painter.clip_rect()))
		.image(
			texture.id(),
			image_rect,
			egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
			Color32::from_white_alpha((u16::from(background.opacity) * 255 / 100) as u8),
		);
}
fn recolor(mut palette: Palette, theme: ExtensionPalette) -> Palette {
	for (destination, color) in [
		&mut palette.base,
		&mut palette.sidebar,
		&mut palette.chat,
		&mut palette.raised,
		&mut palette.hover,
		&mut palette.selected,
		&mut palette.border,
		&mut palette.text_strong,
		&mut palette.text,
		&mut palette.muted,
		&mut palette.link,
		&mut palette.accent,
		&mut palette.accent_text,
		&mut palette.positive,
		&mut palette.warning,
		&mut palette.danger,
		&mut palette.mention_bg,
		&mut palette.mention_text,
	]
	.into_iter()
	.zip(theme.colors)
	{
		if let Some(color) = color {
			*destination = color;
		}
	}
	palette.backdrop = theme.backdrop;
	palette.canvas = palette.chat;
	palette.surface = palette.sidebar;
	palette
}
pub fn colors(dark: bool, variant: Variant) -> Palette {
	let mut palette = builtin_colors(dark, variant);
	if let Some(palettes) = EXTENSION_THEME.get() {
		palette = recolor(palette, palettes[usize::from(dark)]);
	}
	let mut palette = customize(palette, primary_color());
	let (enabled, transparency, _, all) = window_effects();
	if enabled && transparency > 0 {
		let alpha = 100 - u16::from(transparency);
		for (surface, chrome) in [
			(&mut palette.base, true),
			(&mut palette.sidebar, true),
			(&mut palette.chat, false),
			(&mut palette.raised, true),
			(&mut palette.canvas, false),
			(&mut palette.surface, true),
		] {
			if chrome && !all {
				continue;
			}
			let [r, g, b, a] = surface.to_srgba_unmultiplied();
			*surface = Color32::from_rgba_unmultiplied(r, g, b, (u16::from(a) * alpha / 100) as u8);
		}
		palette.backdrop = palette.backdrop.map(|stops| {
			stops.map(|color| {
				let [r, g, b, a] = color.to_srgba_unmultiplied();
				Color32::from_rgba_unmultiplied(r, g, b, (u16::from(a) * alpha / 100) as u8)
			})
		});
	}
	palette
}

fn customize(mut palette: Palette, primary: Option<[u8; 3]>) -> Palette {
	if let Some([r, g, b]) = primary {
		palette.accent = Color32::from_rgb(r, g, b);
		palette.accent_text = if contrast(Color32::WHITE, palette.accent) >= 4.5 {
			Color32::WHITE
		} else {
			Color32::BLACK
		};
		// Mention pills tint with the user's accent too, not just the default house colour.
		palette.mention_bg = palette.accent.gamma_multiply(76.0 / 255.0);
		palette.mention_text = readable_tint(palette.accent, palette.chat);
	}
	palette
}
/// Whichever of a light or dark tint of `base` reads better against `background`.
fn readable_tint(base: Color32, background: Color32) -> Color32 {
	let light = base.lerp_to_gamma(Color32::WHITE, 0.55);
	let dark = base.lerp_to_gamma(Color32::BLACK, 0.45);
	if contrast(light, background) >= contrast(dark, background) {
		light
	} else {
		dark
	}
}
pub(crate) fn theme_preview_palette(ui: &egui::Ui, theme: &extensions::Theme) -> Palette {
	let base = builtin_colors(ui.visuals().dark_mode, variant());
	let theme = if ui.visuals().dark_mode {
		&theme.dark
	} else {
		&theme.light
	};
	let colors = extension_palette(theme).map_or(base, |overrides| recolor(base, overrides));
	opaque_surfaces(customize(colors, primary_color()))
}
pub fn palette(ui: &egui::Ui) -> Palette {
	opaque_surfaces(colors(ui.visuals().dark_mode, variant()))
}
pub fn palette_for(ctx: &egui::Context) -> Palette {
	opaque_surfaces(colors(ctx.theme() == egui::Theme::Dark, variant()))
}
/// Main surfaces preserve gradient presets; widgets and popouts use opaque surfaces.
pub fn window_palette(ui: &egui::Ui) -> Palette {
	colors(ui.visuals().dark_mode, variant())
}
fn opaque_surfaces(mut palette: Palette) -> Palette {
	let backdrop = palette
		.backdrop
		.map_or(palette.chat.to_opaque(), |[top, bottom]| {
			mix(top, bottom, 0.5).to_opaque()
		});
	for surface in [
		&mut palette.base,
		&mut palette.sidebar,
		&mut palette.chat,
		&mut palette.raised,
		&mut palette.canvas,
		&mut palette.surface,
	] {
		*surface = backdrop.blend(*surface);
	}
	palette
}
/// Paint the gradient backdrop behind every panel; a no-op for opaque variants.
pub fn paint_backdrop(ctx: &egui::Context) {
	let dark = ctx.theme() == egui::Theme::Dark;
	let palette = colors(dark, variant());
	let has_image = BACKGROUND_IMAGE.with(|image| image.borrow().is_some());
	if palette.backdrop.is_none() && !has_image {
		return;
	}
	let [top, bottom] = palette.backdrop.unwrap_or([palette.base.to_opaque(); 2]);
	let rect = ctx.content_rect();
	let mut mesh = egui::Mesh::default();
	let mid = Color32::from_rgba_premultiplied(
		((top.r() as u16 + bottom.r() as u16) / 2) as u8,
		((top.g() as u16 + bottom.g() as u16) / 2) as u8,
		((top.b() as u16 + bottom.b() as u16) / 2) as u8,
		((top.a() as u16 + bottom.a() as u16) / 2) as u8,
	);
	mesh.colored_vertex(rect.left_top(), top);
	mesh.colored_vertex(rect.right_top(), mid);
	mesh.colored_vertex(rect.right_bottom(), bottom);
	mesh.colored_vertex(rect.left_bottom(), mid);
	mesh.add_triangle(0, 1, 2);
	mesh.add_triangle(0, 2, 3);
	ctx.layer_painter(egui::LayerId::background())
		.add(egui::Shape::mesh(mesh));
	BACKGROUND_IMAGE.with(|image| {
		if let Some((_, texture)) = image.borrow().as_ref() {
			let background = EXTENSION_THEME
				.get()
				.and_then(|palettes| palettes[usize::from(dark)].background)
				.unwrap_or_default();
			if background.target != extensions::BackgroundTarget::Window {
				return;
			}
			paint_background_image(
				&ctx.layer_painter(egui::LayerId::background()),
				rect,
				texture,
				background,
			);
		}
	});
}

pub const SEMIBOLD: &str = "semibold";
pub const MEDIUM: &str = "medium";
const WEIGHTS_KEY: &str = "serein-font-weights";
// Called by `fonts::install` for one context; until then the weight families resolve to
// the default face so headless contexts (tests) never reference an unknown family.
thread_local! {
	// One context per UI thread, compared by egui's Arc identity; headless contexts stay isolated.
	static WEIGHT_CONTEXT: std::cell::RefCell<Option<(egui::Context, bool)>> = const { std::cell::RefCell::new(None) };
}
pub fn weights_installed(ctx: &egui::Context) {
	ctx.data_mut(|d| d.insert_temp(egui::Id::unique(WEIGHTS_KEY), true));
	WEIGHT_CONTEXT.with(|cache| *cache.borrow_mut() = Some((ctx.clone(), true)));
}
fn weight(ctx: &egui::Context, name: &str) -> FontFamily {
	let installed = WEIGHT_CONTEXT.with(|cache| {
		let mut cache = cache.borrow_mut();
		if let Some((cached, installed)) = &*cache
			&& cached == ctx
		{
			return *installed;
		}
		let installed =
			ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(WEIGHTS_KEY))) == Some(true);
		*cache = Some((ctx.clone(), installed));
		installed
	});
	if installed {
		{
			static MEDIUM_NAME: std::sync::OnceLock<std::sync::Arc<str>> =
				std::sync::OnceLock::new();
			static SEMIBOLD_NAME: std::sync::OnceLock<std::sync::Arc<str>> =
				std::sync::OnceLock::new();
			let cached = if name == MEDIUM {
				&MEDIUM_NAME
			} else {
				&SEMIBOLD_NAME
			};
			FontFamily::Name(cached.get_or_init(|| name.into()).clone())
		}
	} else {
		FontFamily::Proportional
	}
}
pub fn semibold_family(ctx: &egui::Context) -> FontFamily {
	weight(ctx, SEMIBOLD)
}
pub fn medium_family(ctx: &egui::Context) -> FontFamily {
	weight(ctx, MEDIUM)
}
pub fn semibold(ui: &egui::Ui, text: impl Into<String>, size: f32) -> RichText {
	RichText::new(text).font(FontId::new(size, semibold_family(ui.ctx())))
}
pub fn medium(ui: &egui::Ui, text: impl Into<String>, size: f32) -> RichText {
	RichText::new(text).font(FontId::new(size, medium_family(ui.ctx())))
}
/// Uppercase section heading used above channel categories and member groups.
pub fn eyebrow(ui: &egui::Ui, text: impl Into<String>, color: Color32) -> RichText {
	semibold(ui, text.into().to_uppercase(), 12.0).color(color)
}

fn is_activate_target(sense: egui::Sense) -> bool {
	sense.senses_click() && (!sense.senses_drag() || sense.is_focusable())
}

pub(crate) fn menu_anchor_sense() -> egui::Sense {
	egui::Sense::focusable_noninteractive()
}

// Registered once per context, including when appearance settings reapply the theme.
struct ClickableCursor;
impl egui::Plugin for ClickableCursor {
	fn debug_name(&self) -> &'static str {
		"Clickable cursor"
	}
	fn on_end_pass(&mut self, ui: &mut egui::Ui) {
		let ctx = ui.ctx();
		if ctx.output(|output| output.cursor_icon) != egui::CursorIcon::Default {
			return;
		}
		let hovered = ctx.interaction_snapshot(|snapshot| snapshot.hovered.clone());
		if hovered.into_iter().any(|id| {
			ctx.read_response(id).is_some_and(|response| {
				response.enabled() && response.hovered() && is_activate_target(response.sense)
			})
		}) {
			ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
		}
	}
}

pub fn apply(ctx: &egui::Context) {
	ctx.add_plugin(ClickableCursor);
	crate::select::install(ctx);
	let variant = variant();
	let metrics = EXTENSION_STYLE.get();
	let item_spacing = metrics.item_spacing.unwrap_or([8, 8]);
	let button_padding = metrics.button_padding.unwrap_or([12, 6]);
	for theme in [egui::Theme::Dark, egui::Theme::Light] {
		let p = opaque_surfaces(colors(theme == egui::Theme::Dark, variant));
		let mut style = (*ctx.style_of(theme)).clone();
		style.text_styles.insert(
			egui::TextStyle::Heading,
			FontId::new(
				f32::from(metrics.heading_size.unwrap_or(20)),
				semibold_family(ctx),
			),
		);
		style.text_styles.insert(
			egui::TextStyle::Body,
			FontId::proportional(f32::from(metrics.body_size.unwrap_or(15))),
		);
		style.text_styles.insert(
			egui::TextStyle::Button,
			FontId::new(
				f32::from(metrics.button_size.unwrap_or(14)),
				medium_family(ctx),
			),
		);
		style.text_styles.insert(
			egui::TextStyle::Small,
			FontId::proportional(f32::from(metrics.small_size.unwrap_or(12))),
		);
		style.text_styles.insert(
			egui::TextStyle::Monospace,
			FontId::monospace(f32::from(metrics.monospace_size.unwrap_or(14))),
		);
		style.spacing.item_spacing =
			egui::vec2(f32::from(item_spacing[0]), f32::from(item_spacing[1]));
		style.spacing.button_padding =
			egui::vec2(f32::from(button_padding[0]), f32::from(button_padding[1]));
		style.spacing.interact_size.y = f32::from(metrics.control_height.unwrap_or(32));
		style.spacing.menu_margin = egui::Margin::same(8);
		style.visuals.panel_fill = p.chat;
		style.visuals.text_options.font_hinting = false;
		style.visuals.text_options.subpixel_binning = true;
		style.visuals.text_options.color_transfer_function = if theme == egui::Theme::Dark {
			FontColorTransferFunction::Gamma(0.5)
		} else {
			FontColorTransferFunction::Off
		};
		style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
		style.visuals.window_fill = p.raised.to_opaque();
		style.visuals.window_corner_radius = metrics.window_radius.unwrap_or(12).into();
		style.visuals.menu_corner_radius = metrics.menu_radius.unwrap_or(12).into();
		style.visuals.window_stroke = Stroke::new(1.0, p.border);
		style.visuals.window_shadow = egui::epaint::Shadow {
			offset: [0, 8],
			blur: 24,
			spread: 0,
			color: Color32::from_black_alpha(
				if p.backdrop.is_some() || theme == egui::Theme::Dark {
					120
				} else {
					40
				},
			),
		};
		style.visuals.popup_shadow = style.visuals.window_shadow;
		style.visuals.override_text_color = Some(p.text);
		style.visuals.weak_text_color = Some(p.muted);
		style.visuals.hyperlink_color = p.link;
		style.visuals.extreme_bg_color = p.raised;
		style.visuals.text_edit_bg_color = Some(p.raised);
		style.visuals.code_bg_color = p.raised;
		style.visuals.faint_bg_color = p.hover;
		style.visuals.selection.bg_fill = p.accent.gamma_multiply(0.35);
		style.visuals.selection.stroke = Stroke::new(1.0, p.accent);
		style.visuals.text_cursor.stroke = Stroke::new(2.0, p.text);
		for widget in [
			&mut style.visuals.widgets.noninteractive,
			&mut style.visuals.widgets.inactive,
			&mut style.visuals.widgets.hovered,
			&mut style.visuals.widgets.active,
			&mut style.visuals.widgets.open,
		] {
			widget.corner_radius = metrics.widget_radius.unwrap_or(8).into();
			widget.fg_stroke = Stroke::new(1.0, p.text);
			widget.bg_stroke = Stroke::NONE;
			widget.expansion = 0.0;
		}
		style.visuals.widgets.noninteractive.bg_fill = p.sidebar;
		style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
		style.visuals.widgets.inactive.bg_fill = p.raised;
		style.visuals.widgets.inactive.weak_bg_fill = p.raised;
		style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
		style.visuals.widgets.hovered.bg_fill = p.selected;
		style.visuals.widgets.hovered.weak_bg_fill = p.hover;
		style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text_strong);
		style.visuals.widgets.active.bg_fill = p.selected;
		style.visuals.widgets.active.weak_bg_fill = p.selected;
		style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.text_strong);
		style.visuals.widgets.open.bg_fill = p.selected;
		style.visuals.widgets.open.weak_bg_fill = p.selected;
		style.visuals.widgets.open.fg_stroke = Stroke::new(1.0, p.text_strong);
		ctx.set_style_of(theme, style);
	}
	ctx.options_mut(|options| {
		options.input_options.line_scroll_speed = crate::scroll::DISCORD_LINE_SCROLL_SPEED;
	});
}
/// Space reserved at the left of window strips for macOS traffic lights.
pub const TRAFFIC_LIGHT_INSET: f32 = if cfg!(target_os = "macos") { 72.0 } else { 0.0 };
/// Width of the Windows caption buttons drawn by [`window_controls`]; zero elsewhere.
pub const WINDOW_CONTROLS_WIDTH: f32 = if cfg!(target_os = "windows") {
	3.0 * 46.0
} else {
	0.0
};

/// Make `rect` behave like a native title bar: drag moves the window and, where the app
/// draws its own frame (Windows), a double click toggles maximize.
pub fn window_drag(ui: &mut egui::Ui, rect: egui::Rect) {
	// The OS owns dragging. Sensing only clicks lets child caption buttons win hit testing.
	let response = ui.interact(
		rect,
		ui.scope_id().with("window-drag"),
		egui::Sense::click(),
	);
	// StartDrag must reach the window backend on the press, before egui's drag threshold.
	if response.is_pointer_button_down_on()
		&& ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
	{
		ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
	}
	if cfg!(target_os = "windows") && response.double_clicked() {
		let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
		ui.ctx()
			.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
	}
}

/// Native resize handles for the undecorated Windows viewport, including sign-in.
pub fn window_resize(ctx: &egui::Context) {
	if !cfg!(target_os = "windows")
		|| ctx.input(|i| {
			i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
		}) {
		return;
	}
	use egui::{CursorIcon as C, ResizeDirection as D};
	let rect = ctx.viewport_rect();
	let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
	let edge = 5.0;
	let corner = 12.0;
	for (index, (min, max, direction, cursor)) in [
		(
			[l, t],
			[l + corner, t + corner],
			D::NorthWest,
			C::ResizeNwSe,
		),
		(
			[r - corner, t],
			[r, t + corner],
			D::NorthEast,
			C::ResizeNeSw,
		),
		(
			[l, b - corner],
			[l + corner, b],
			D::SouthWest,
			C::ResizeNeSw,
		),
		(
			[r - corner, b - corner],
			[r, b],
			D::SouthEast,
			C::ResizeNwSe,
		),
		(
			[l + corner, t],
			[r - corner, t + edge],
			D::North,
			C::ResizeVertical,
		),
		(
			[l + corner, b - edge],
			[r - corner, b],
			D::South,
			C::ResizeVertical,
		),
		(
			[l, t + corner],
			[l + edge, b - corner],
			D::West,
			C::ResizeHorizontal,
		),
		(
			[r - edge, t + corner],
			[r, b - corner],
			D::East,
			C::ResizeHorizontal,
		),
	]
	.into_iter()
	.enumerate()
	{
		let handle = egui::Rect::from_min_max(min.into(), max.into());
		egui::Area::new(egui::Id::unique(("window-resize", index)))
			.order(egui::Order::Foreground)
			.fixed_pos(handle.min)
			.constrain(false)
			.default_size(handle.size())
			.movable(false)
			.show(ctx, |ui| {
				let (_, response) = ui.allocate_exact_size(handle.size(), egui::Sense::click());
				let response = response.on_hover_cursor(cursor);
				if response.is_pointer_button_down_on()
					&& ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
				{
					ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
				}
			});
	}
}

/// Discord-style caption buttons (minimize, maximize/restore, close) for the undecorated
/// Windows frame. Lay out in a right-to-left `ui`; a no-op on other platforms.
pub fn window_controls(ui: &mut egui::Ui) {
	if !cfg!(target_os = "windows") {
		return;
	}
	let p = palette(ui);
	let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
	let height = ui.available_height().clamp(28.0, 36.0);
	ui.spacing_mut().item_spacing.x = 0.0;
	let caption =
		|ui: &mut egui::Ui, label: &str, danger: bool| -> (egui::Response, egui::Rect, Color32) {
			let (rect, response) =
				ui.allocate_exact_size(egui::vec2(46.0, height), egui::Sense::click());
			response
				.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
			let hovered = response.hovered() || response.has_focus();
			if hovered {
				ui.painter()
					.rect_filled(rect, 0, if danger { rgb(0xc42b1c) } else { p.hover });
			}
			let color = if hovered && danger {
				Color32::WHITE
			} else if hovered {
				p.text_strong
			} else {
				p.text
			};
			(response, rect, color)
		};
	let (close, rect, color) = caption(ui, "Close", true);
	let c = rect.center();
	let stroke = Stroke::new(1.0, color);
	ui.painter().line_segment(
		[c + egui::vec2(-5.0, -5.0), c + egui::vec2(5.0, 5.0)],
		stroke,
	);
	ui.painter().line_segment(
		[c + egui::vec2(-5.0, 5.0), c + egui::vec2(5.0, -5.0)],
		stroke,
	);
	if close.clicked() {
		ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
	}
	let (toggle, rect, color) = caption(ui, if maximized { "Restore" } else { "Maximize" }, false);
	let c = rect.center();
	let stroke = Stroke::new(1.0, color);
	if maximized {
		let back = egui::Rect::from_center_size(c + egui::vec2(1.5, -1.5), egui::Vec2::splat(9.0));
		let front = egui::Rect::from_center_size(c + egui::vec2(-1.5, 1.5), egui::Vec2::splat(9.0));
		ui.painter().line_segment(
			[back.left_top() + egui::vec2(0.0, 0.0), back.right_top()],
			stroke,
		);
		ui.painter()
			.line_segment([back.right_top(), back.right_bottom()], stroke);
		ui.painter().line_segment(
			[back.left_top(), back.left_top() + egui::vec2(0.0, 3.0)],
			stroke,
		);
		ui.painter().line_segment(
			[
				back.right_bottom(),
				back.right_bottom() - egui::vec2(3.0, 0.0),
			],
			stroke,
		);
		ui.painter()
			.rect_stroke(front, 1, stroke, egui::StrokeKind::Middle);
	} else {
		let square = egui::Rect::from_center_size(c, egui::Vec2::splat(10.0));
		ui.painter()
			.rect_stroke(square, 1, stroke, egui::StrokeKind::Middle);
	}
	if toggle.clicked() {
		ui.ctx()
			.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
	}
	let (minimize, rect, color) = caption(ui, "Minimize", false);
	let c = rect.center();
	ui.painter().line_segment(
		[c + egui::vec2(-5.0, 0.5), c + egui::vec2(5.0, 0.5)],
		Stroke::new(1.0, color),
	);
	if minimize.clicked() {
		ui.ctx()
			.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
	}
	ui.add_space(6.0);
}

/// Full-width accent call to action with centred text and an optional leading icon.
pub fn primary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
	let p = palette(ui);
	wide_button(ui, label, None, p.accent, Stroke::NONE, p.accent_text)
}
pub fn primary_icon_button(
	ui: &mut egui::Ui,
	icon: crate::icons::Icon,
	label: &str,
) -> egui::Response {
	let p = palette(ui);
	wide_button(ui, label, Some(icon), p.accent, Stroke::NONE, p.accent_text)
}
/// Full-width neutral companion to [`primary_button`].
pub fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
	let p = palette(ui);
	wide_button(
		ui,
		label,
		None,
		p.raised,
		Stroke::new(1.0, p.border),
		p.text_strong,
	)
}
fn wide_button(
	ui: &mut egui::Ui,
	label: &str,
	icon: Option<crate::icons::Icon>,
	fill: Color32,
	stroke: Stroke,
	text: Color32,
) -> egui::Response {
	let p = palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 44.0), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
	});
	let enabled = ui.is_enabled();
	let fill = if !enabled {
		fill.gamma_multiply(0.5)
	} else if response.is_pointer_button_down_on() {
		fill.gamma_multiply(0.85)
	} else if response.hovered() {
		fill.linear_multiply(1.12)
	} else {
		fill
	};
	let text = if enabled {
		text
	} else {
		text.gamma_multiply(0.6)
	};
	let painter = ui.painter();
	painter.rect(rect, 8, fill, stroke, egui::StrokeKind::Inside);
	if response.has_focus() {
		painter.rect_stroke(
			rect.expand(2.0),
			10,
			Stroke::new(2.0, p.accent),
			egui::StrokeKind::Outside,
		);
	}
	let galley = painter.layout_no_wrap(
		label.to_owned(),
		FontId::new(15.0, medium_family(ui.ctx())),
		text,
	);
	let icon_size = if icon.is_some() { 20.0 } else { 0.0 };
	let gap = if icon.is_some() { 10.0 } else { 0.0 };
	let total = icon_size + gap + galley.size().x;
	let mut x = rect.center().x - total * 0.5;
	if let Some(icon) = icon {
		let icon_rect = egui::Rect::from_center_size(
			egui::pos2(x + icon_size * 0.5, rect.center().y),
			egui::Vec2::splat(icon_size),
		);
		crate::icons::paint(painter, icon, icon_rect, text);
		x += icon_size + gap;
	}
	painter.galley(
		egui::pos2(x, rect.center().y - galley.size().y * 0.5),
		galley,
		text,
	);
	if enabled && response.hovered() {
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
	}
	response
}
/// Deterministic fallback avatar colours drawn from the Serein palette, keyed by the display name.
fn fallback_avatar_color(name: &str) -> Color32 {
	const COLORS: [u32; 5] = [DEFAULT_PRIMARY_RGB, 0x6b7a94, 0x2fb87a, 0xe8a33d, 0xef5561];
	let hash = name
		.bytes()
		.fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
	rgb(COLORS[(hash % COLORS.len() as u32) as usize])
}
pub fn avatar(ui: &mut egui::Ui, name: &str, size: f32) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
	paint_avatar(ui, name, size, rect);
	response.on_hover_text(name)
}
pub(crate) fn paint_avatar(ui: &egui::Ui, name: &str, size: f32, rect: egui::Rect) {
	let initials: String = name
		.split_whitespace()
		.take(2)
		.filter_map(|part| part.chars().find(|c| c.is_alphanumeric()))
		.flat_map(char::to_uppercase)
		.take(2)
		.collect();
	ui.painter()
		.circle_filled(rect.center(), size * 0.5, fallback_avatar_color(name));
	ui.painter().text(
		rect.center(),
		egui::Align2::CENTER_CENTER,
		initials,
		FontId::new(size * 0.36, semibold_family(ui.ctx())),
		Color32::WHITE,
	);
}
/// Presence dot with a surface-coloured ring, bottom-right of an avatar `rect`.
pub fn presence_dot(ui: &egui::Ui, rect: egui::Rect, color: Color32, ring: Color32) {
	let radius = (rect.width() * 0.16).clamp(4.0, 8.0);
	let center = rect.right_bottom() - egui::vec2(radius + 0.5, radius + 0.5);
	ui.painter().circle_filled(center, radius + 2.0, ring);
	ui.painter().circle_filled(center, radius, color);
}

/// Preserve role hue where readable, otherwise move toward the theme's text color.
pub fn role_name_color(rgb: u32, background: Color32, fallback: Color32) -> Color32 {
	let role = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
	for step in 0..=16 {
		let mix = |a: u8, b: u8| ((u32::from(a) * (16 - step) + u32::from(b) * step) / 16) as u8;
		let color = Color32::from_rgb(
			mix(role.r(), fallback.r()),
			mix(role.g(), fallback.g()),
			mix(role.b(), fallback.b()),
		);
		if contrast(color, background) >= 4.5 {
			return color;
		}
	}
	fallback
}
fn luminance(c: Color32) -> f32 {
	let channel = |v: u8| {
		let v = v as f32 / 255.0;
		if v <= 0.03928 {
			v / 12.92
		} else {
			((v + 0.055) / 1.055).powf(2.4)
		}
	};
	0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}
fn contrast(a: Color32, b: Color32) -> f32 {
	let (l1, l2) = (luminance(a) + 0.05, luminance(b) + 0.05);
	l1.max(l2) / l1.min(l2)
}

#[cfg(test)]
mod tests {
	#[test]
	fn action_button_text_uses_the_current_palette() {
		use super::*;
		for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
			for kind in [
				ButtonKind::Neutral,
				ButtonKind::Outline,
				ButtonKind::Primary,
			] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				apply(&ctx);
				let mut expected = Color32::TRANSPARENT;
				let output = ctx.run_ui(egui::RawInput::default(), |ui| {
					let palette = palette(ui);
					expected = match kind {
						ButtonKind::Neutral => palette.text,
						ButtonKind::Outline => palette.text_strong,
						_ => palette.accent_text,
					};
					button(ui, "Readable action", kind);
				});
				let text = output
					.shapes
					.iter()
					.find_map(|shape| match &shape.shape {
						egui::Shape::Text(text) if text.galley.job.text == "Readable action" => {
							Some(text)
						}
						_ => None,
					})
					.expect("button label is painted");
				let color = text
					.override_text_color
					.unwrap_or(text.galley.job.sections[0].format.color);
				assert_eq!(
					if color == Color32::PLACEHOLDER {
						text.fallback_color
					} else {
						color
					},
					expected
				);
				output.drop_without_applying_deltas();
			}
		}
	}
	#[test]
	fn theme_card_preview_inherits_builtin_colors_not_the_active_theme() {
		use super::*;
		set_variant(Variant::Standard);
		set_primary_color(None);
		let ctx = egui::Context::default();
		ctx.set_theme(egui::ThemePreference::Dark);
		let mut active = extensions::Theme::default();
		active
			.dark
			.colors
			.insert("sidebar".into(), "#FF0000".into());
		set_extension_theme(Some(&active));
		apply(&ctx);
		let mut candidate = extensions::Theme::default();
		candidate
			.dark
			.colors
			.insert("accent".into(), "#00FF00".into());
		let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
			assert_eq!(palette(ui).sidebar, rgb(0xff0000));
			let preview = theme_preview_palette(ui, &candidate);
			assert_eq!(
				preview.sidebar,
				builtin_colors(true, Variant::Standard).sidebar
			);
			assert_eq!(preview.accent, rgb(0x00ff00));
		});
		output.textures_delta.clear();
		set_extension_theme(None);
	}

	#[test]
	fn clickable_cursor_preserves_disabled_text_and_specialized_controls() {
		use egui::{CursorIcon, Sense};
		for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
			for (kind, expected) in [
				("button", CursorIcon::PointingHand),
				("checkbox", CursorIcon::PointingHand),
				("custom", CursorIcon::PointingHand),
				("click-drag", CursorIcon::PointingHand),
				("chrome", CursorIcon::Default),
				("menu-anchor", CursorIcon::Default),
				("disabled", CursorIcon::Default),
				("disabled-custom", CursorIcon::Default),
				("hover", CursorIcon::Default),
				("text", CursorIcon::Text),
				("resize", CursorIcon::ResizeHorizontal),
				("drag", CursorIcon::Grab),
			] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				super::apply(&ctx);
				super::apply(&ctx);
				let mut center = egui::Pos2::ZERO;
				let mut text = String::from("Editable text");
				let mut actual = CursorIcon::Default;
				for _ in 0..3 {
					let output = ctx.run_ui(
						egui::RawInput {
							events: vec![egui::Event::PointerMoved(center)],
							..Default::default()
						},
						|ui| {
							let response = match kind {
								"button" => ui.button("Action"),
								"checkbox" => ui.checkbox(&mut false, "Toggle"),
								"disabled" => ui.add_enabled(false, egui::Button::new("Disabled")),
								"text" => ui.text_edit_singleline(&mut text),
								_ => {
									ui.add_enabled_ui(kind != "disabled-custom", |ui| {
										ui.allocate_exact_size(
											egui::vec2(100.0, 32.0),
											match kind {
												"hover" => Sense::hover(),
												"click-drag" => Sense::click_and_drag(),
												"chrome" => Sense::CLICK | Sense::DRAG,
												"menu-anchor" => super::menu_anchor_sense(),
												_ => Sense::click(),
											},
										)
										.1
									})
									.inner
								}
							};
							center = response.rect.center();
							if matches!(kind, "resize" | "drag") {
								response.on_hover_cursor(expected);
							}
						},
					);
					actual = output.platform_output.cursor_icon;
					output.drop_without_applying_deltas();
				}
				assert_eq!(actual, expected, "{kind}");
			}
		}
	}
	use super::*;
	#[test]
	fn hex_colors_accept_pasted_values_without_applying_partial_or_invalid_input() {
		for text in ["#FF4000", "ff4000", " #Ff4000 "] {
			assert_eq!(parse_hex_color(text), Some(0xFF4000));
		}
		assert_eq!(parse_hex_color("#000000"), Some(0));
		assert_eq!(parse_hex_color("FFFFFF"), Some(0xFFFFFF));
		for text in [
			"",
			"#FF4",
			"#FF40000",
			"#FF4000FF",
			"#GG4000",
			"+FF400",
			"éFF400",
		] {
			assert_eq!(parse_hex_color(text), None);
		}
	}
	#[test]
	fn primary_color_preserves_readable_controls() {
		for dark in [false, true] {
			let base = colors(dark, Variant::Standard);
			for rgb in [[0, 0, 0], [255, 255, 255], [255, 220, 0], [90, 40, 200]] {
				let p = customize(base, Some(rgb));
				assert_eq!(p.accent.to_array()[..3], rgb);
				assert!(contrast(p.accent_text, p.accent) >= 4.5);
				assert_eq!((p.text, p.raised), (base.text, base.raised));
			}
			assert_eq!(customize(base, None), base);
			assert_eq!(base.accent, rgb(DEFAULT_PRIMARY_RGB));
		}
	}
	#[test]
	fn popup_surfaces_stay_opaque_for_every_preset() {
		for variant in Variant::ALL {
			for dark in [false, true] {
				let base = colors(dark, variant);
				let popup = opaque_surfaces(base);
				for surface in [
					popup.base,
					popup.sidebar,
					popup.chat,
					popup.raised,
					popup.canvas,
					popup.surface,
				] {
					assert_eq!(surface.a(), 255);
				}
				assert_eq!(popup.text, base.text);
			}
		}
	}
	#[test]
	fn role_colors_remain_readable_in_light_and_dark_palettes() {
		for variant in Variant::ALL {
			for dark in [false, true] {
				let p = colors(dark, variant);
				for rgb in [0, 0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0xe78284] {
					for background in [p.sidebar, p.hover] {
						assert!(
							contrast(role_name_color(rgb, background, p.text), background) >= 4.5
						);
					}
				}
			}
		}
	}
	#[test]
	fn opaque_presets_keep_readable_text_and_keys_round_trip() {
		for variant in Variant::ALL {
			assert_eq!(Variant::from_key(variant.key()), Some(variant));
			for dark in [true, false] {
				let p = colors(dark, variant);
				assert_eq!(p.canvas, p.chat);
				assert_eq!(p.surface, p.sidebar);
				if p.backdrop.is_none() {
					assert!(contrast(p.text, p.chat) >= 7.0, "{variant:?} body text");
					assert!(
						contrast(p.muted, p.sidebar) >= 4.5,
						"{variant:?} muted text"
					);
					assert!(
						contrast(p.accent_text, p.accent) >= 4.5,
						"{variant:?} accent text"
					);
				} else {
					assert!(variant.is_gradient());
				}
			}
		}
		assert_eq!(Variant::from_key("nonsense"), None);
		assert_eq!(Variant::from_u8(200), Variant::Standard);
	}
}

/// Release channel shown in the title bar; stable builds show nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Channel {
	#[default]
	Stable,
	Nightly,
	Dev,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Build {
	pub channel: Channel,
	pub version: &'static str,
}
/// Gradient release pill with a glow, a channel glyph and the version. None for stable builds.
pub fn build_badge(ui: &mut egui::Ui, build: Build) -> Option<egui::Response> {
	let (label, hint, stops) = match build.channel {
		Channel::Stable => return None,
		Channel::Nightly => (
			"NIGHTLY",
			"Nightly build from the latest main. Unofficial client; live compatibility is unverified.",
			[rgb(DEFAULT_PRIMARY_RGB), rgb(0x7bd0ff)],
		),
		Channel::Dev => (
			"DEV",
			"Local development build. Unofficial client; live compatibility is unverified.",
			[rgb(0xe8a33d), rgb(0xe8590c)],
		),
	};
	let font = FontId::new(10.0, semibold_family(ui.ctx()));
	let title = ui
		.painter()
		.layout_no_wrap(label.to_owned(), font, Color32::WHITE);
	let version = (!build.version.is_empty()).then(|| {
		ui.painter().layout_no_wrap(
			format!("v{}", build.version),
			FontId::monospace(10.0),
			Color32::from_white_alpha(210),
		)
	});
	const HEIGHT: f32 = 20.0;
	const PAD: f32 = 8.0;
	const GLYPH: f32 = 12.0;
	let width = PAD
		+ GLYPH
		+ 5.0 + title.size().x
		+ version.as_ref().map_or(0.0, |v| 13.0 + v.size().x)
		+ PAD;
	let (rect, response) = ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::hover());
	let painter = ui.painter();
	let radius = HEIGHT / 2.0;
	// Soft glow, then a pill whose caps carry the gradient end colours.
	painter.rect_filled(
		rect.expand(3.0),
		radius + 3.0,
		stops[0].gamma_multiply(0.14),
	);
	painter.rect_filled(
		rect.expand(1.0),
		radius + 1.0,
		stops[0].gamma_multiply(0.22),
	);
	let left = egui::pos2(rect.left() + radius, rect.center().y);
	let right = egui::pos2(rect.right() - radius, rect.center().y);
	painter.circle_filled(left, radius, stops[0]);
	painter.circle_filled(right, radius, stops[1]);
	let mut mesh = egui::Mesh::default();
	mesh.colored_vertex(egui::pos2(left.x, rect.top()), stops[0]);
	mesh.colored_vertex(egui::pos2(right.x, rect.top()), stops[1]);
	mesh.colored_vertex(egui::pos2(right.x, rect.bottom()), stops[1]);
	mesh.colored_vertex(egui::pos2(left.x, rect.bottom()), stops[0]);
	mesh.add_triangle(0, 1, 2);
	mesh.add_triangle(0, 2, 3);
	painter.add(egui::Shape::mesh(mesh));
	// Top highlight line for a glassy edge.
	painter.line_segment(
		[
			egui::pos2(rect.left() + radius, rect.top() + 1.0),
			egui::pos2(rect.right() - radius, rect.top() + 1.0),
		],
		Stroke::new(1.0, Color32::from_white_alpha(56)),
	);
	let glyph = egui::Rect::from_center_size(
		egui::pos2(rect.left() + PAD + GLYPH / 2.0, rect.center().y),
		egui::Vec2::splat(GLYPH),
	);
	match build.channel {
		Channel::Nightly => {
			// Crescent moon: a white disc with a pill-coloured bite.
			painter.circle_filled(glyph.center(), 4.6, Color32::WHITE);
			painter.circle_filled(glyph.center() + egui::vec2(2.4, -1.8), 4.0, stops[0]);
		}
		Channel::Dev | Channel::Stable => {
			// Code chevrons: < >
			let s = Stroke::new(1.5, Color32::WHITE);
			let c = glyph.center();
			painter.line_segment([c + egui::vec2(-1.5, -3.5), c + egui::vec2(-4.5, 0.0)], s);
			painter.line_segment([c + egui::vec2(-4.5, 0.0), c + egui::vec2(-1.5, 3.5)], s);
			painter.line_segment([c + egui::vec2(1.5, -3.5), c + egui::vec2(4.5, 0.0)], s);
			painter.line_segment([c + egui::vec2(4.5, 0.0), c + egui::vec2(1.5, 3.5)], s);
		}
	}
	let mut x = glyph.right() + 5.0;
	painter.galley(
		egui::pos2(x, rect.center().y - title.size().y / 2.0),
		title.clone(),
		Color32::WHITE,
	);
	x += title.size().x;
	if let Some(version) = version {
		x += 6.0;
		painter.line_segment(
			[
				egui::pos2(x, rect.top() + 5.0),
				egui::pos2(x, rect.bottom() - 5.0),
			],
			Stroke::new(1.0, Color32::from_white_alpha(90)),
		);
		x += 7.0;
		painter.galley(
			egui::pos2(x, rect.center().y - version.size().y / 2.0),
			version,
			Color32::WHITE,
		);
	}
	Some(response.on_hover_text(hint))
}

/// Discord-style settings row with a pill switch on the right. Clicking anywhere on the row
/// toggles `enabled`; the accessible label is `label`.
pub fn switch(
	ui: &mut egui::Ui,
	label: &str,
	description: Option<&str>,
	enabled: &mut bool,
) -> egui::Response {
	let p = palette(ui);
	let width = ui.available_width();
	let text_width = (width - 64.0).max(80.0);
	let title = ui.painter().layout(
		label.to_owned(),
		FontId::new(16.0, medium_family(ui.ctx())),
		p.text_strong,
		text_width,
	);
	let detail = description.map(|text| {
		ui.painter().layout(
			text.to_owned(),
			FontId::proportional(13.0),
			p.muted,
			text_width,
		)
	});
	let text_height = title.size().y + detail.as_ref().map_or(0.0, |d| d.size().y + 4.0);
	let (rect, mut response) = ui.allocate_exact_size(
		egui::vec2(width, text_height.max(24.0) + 16.0),
		egui::Sense::click(),
	);
	if response.clicked() {
		*enabled = !*enabled;
		response.mark_changed();
	}
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *enabled, label)
	});
	let painter = ui.painter();
	let mut y = rect.top() + 8.0;
	painter.galley(egui::pos2(rect.left(), y), title.clone(), p.text_strong);
	y += title.size().y + 4.0;
	if let Some(detail) = detail {
		painter.galley(egui::pos2(rect.left(), y), detail, p.muted);
	}
	let pill = egui::Rect::from_center_size(
		egui::pos2(rect.right() - 20.0, rect.top() + 8.0 + title.size().y / 2.0),
		egui::vec2(40.0, 24.0),
	);
	let mut fill = if *enabled { p.accent } else { p.base };
	if !ui.is_enabled() {
		fill = fill.gamma_multiply(0.4);
	}
	painter.rect_filled(pill, 12, fill);
	painter.rect_stroke(
		pill,
		12,
		Stroke::new(1.0, if *enabled { fill } else { p.border }),
		egui::StrokeKind::Inside,
	);
	let knob = egui::pos2(
		if *enabled {
			pill.right() - 12.0
		} else {
			pill.left() + 12.0
		},
		pill.center().y,
	);
	painter.circle_filled(knob, 9.0, Color32::WHITE);
	if response.has_focus() {
		painter.rect_stroke(
			pill.expand(3.0),
			15,
			Stroke::new(2.0, p.accent),
			egui::StrokeKind::Outside,
		);
	}
	response
}

/// Height of every inline [`button`].
const BUTTON_HEIGHT: f32 = 38.0;

/// Visual weight of an inline [`button`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ButtonKind {
	/// Accent-filled confirming action. One per surface.
	Primary,
	/// Destructive confirming action.
	Danger,
	/// Borderless companion, used for "Cancel".
	Neutral,
	/// Bordered companion for a secondary but non-dismissing action.
	Outline,
}

/// Compact inline button. Sizes, radius, focus ring and disabled styling are identical
/// everywhere: dialog footers, settings toolbars and page headers all use this.
pub fn button(ui: &mut egui::Ui, label: &str, kind: ButtonKind) -> egui::Response {
	let p = palette(ui);
	let (fill, stroke, text) = match kind {
		ButtonKind::Primary => (p.accent, Stroke::NONE, p.accent_text),
		ButtonKind::Danger => (p.danger, Stroke::NONE, Color32::WHITE),
		ButtonKind::Neutral => (Color32::TRANSPARENT, Stroke::NONE, p.text),
		ButtonKind::Outline => (
			Color32::TRANSPARENT,
			Stroke::new(1.0, p.border),
			p.text_strong,
		),
	};
	let font = FontId::new(14.0, medium_family(ui.ctx()));
	let galley = ui
		.painter()
		.layout_no_wrap(label.to_owned(), font, Color32::WHITE);
	let width = (galley.size().x + 32.0).max(if kind == ButtonKind::Neutral {
		72.0
	} else {
		92.0
	});
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(width, BUTTON_HEIGHT), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
	});
	let enabled = ui.is_enabled();
	let hot = response.hovered() || response.has_focus();
	let fill = if !enabled {
		fill.gamma_multiply(0.4)
	} else if response.is_pointer_button_down_on() {
		fill.gamma_multiply(0.82)
	} else if hot {
		match kind {
			ButtonKind::Neutral | ButtonKind::Outline => p.hover,
			_ => fill.linear_multiply(1.1),
		}
	} else {
		fill
	};
	let text = if enabled {
		text
	} else {
		text.gamma_multiply(0.5)
	};
	let painter = ui.painter();
	painter.rect(rect, 8, fill, stroke, egui::StrokeKind::Inside);
	if response.has_focus() {
		painter.rect_stroke(
			rect.expand(2.0),
			10,
			Stroke::new(2.0, p.accent),
			egui::StrokeKind::Outside,
		);
	}
	painter.galley_with_override_text_color(
		rect.center() - galley.size() * 0.5,
		galley.clone(),
		text,
	);
	response
}

/// Uppercase label above a form control.
pub fn label(ui: &mut egui::Ui, text: &str) -> egui::Response {
	let colors = palette(ui);
	let response = ui.label(eyebrow(ui, text, colors.muted));
	ui.add_space(6.0);
	response
}

/// Small muted explanation under a form control.
pub fn hint(ui: &mut egui::Ui, text: &str) {
	let colors = palette(ui);
	ui.add_space(4.0);
	ui.add(egui::Label::new(RichText::new(text).size(12.0).color(colors.muted)).wrap());
}

/// Text input with the dialog's inset fill and an accent focus ring.
pub fn input(ui: &mut egui::Ui, edit: egui::TextEdit<'_>) -> egui::Response {
	let colors = palette(ui);
	let response = ui.add(
		edit.desired_width(f32::INFINITY)
			.background_color(colors.base)
			.frame(
				egui::Frame::new()
					.fill(colors.base)
					.corner_radius(8)
					.inner_margin(egui::Margin::symmetric(12, 9)),
			),
	);
	let stroke = if response.has_focus() {
		Stroke::new(2.0, colors.accent)
	} else {
		Stroke::new(1.0, colors.border)
	};
	ui.painter()
		.rect_stroke(response.rect, 8, stroke, egui::StrokeKind::Inside);
	response
}

/// Severity of a [`notice`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
	Info,
	Warning,
	Error,
}

/// Tinted callout used for dialog status, permission and failure messages. Replaces the bare
/// coloured labels these dialogs used to print.
pub fn notice(ui: &mut egui::Ui, level: Level, text: &str) {
	let colors = palette(ui);
	let (tint, icon) = match level {
		Level::Info => (colors.accent, crate::icons::Icon::Help),
		Level::Warning => (colors.warning, crate::icons::Icon::ShieldWarning),
		Level::Error => (colors.danger, crate::icons::Icon::ShieldWarning),
	};
	egui::Frame::new()
		.fill(tint.gamma_multiply(0.13))
		.stroke(Stroke::new(1.0, tint.gamma_multiply(0.45)))
		.corner_radius(8)
		.inner_margin(egui::Margin::symmetric(12, 10))
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			ui.horizontal_top(|ui| {
				ui.spacing_mut().item_spacing.x = 8.0;
				let (rect, _) =
					ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
				crate::icons::paint(ui.painter(), icon, rect, tint);
				ui.add(egui::Label::new(RichText::new(text).size(13.0).color(colors.text)).wrap());
			});
		});
}

/// Horizontal rule between settings sections. One spacing rhythm everywhere.
pub fn divider(ui: &mut egui::Ui) {
	ui.add_space(24.0);
	ui.separator();
	ui.add_space(24.0);
}

/// Title of a settings group, with an optional supporting line under it.
pub fn section(ui: &mut egui::Ui, title: &str, help: Option<&str>) {
	let p = palette(ui);
	ui.label(medium(ui, title, 16.0).color(p.text_strong));
	if let Some(help) = help {
		ui.add(egui::Label::new(RichText::new(help).size(13.0).color(p.muted)).wrap());
	}
	ui.add_space(6.0);
}

/// Height for a virtualized list that fills the rest of a settings page, leaving `reserved`
/// pixels for the controls below it. Keeps every list scrolling against the page instead of
/// guessing a height from the window size.
pub fn list_height(ui: &egui::Ui, reserved: f32) -> f32 {
	(ui.available_height() - reserved).max(160.0)
}

/// Rounded settings card that groups related rows on the raised surface.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
	let p = palette(ui);
	egui::Frame::new()
		.fill(p.raised)
		.stroke(Stroke::new(1.0, p.border))
		.corner_radius(8)
		.inner_margin(egui::Margin::symmetric(16, 12))
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			add(ui)
		})
		.inner
}

/// Syntax colours for fenced code blocks: one dark and one light set, tuned to stay legible
/// on the `raised` surface every preset uses as its code background.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CodeColors {
	pub keyword: Color32,
	pub type_name: Color32,
	pub function: Color32,
	pub string: Color32,
	pub comment: Color32,
	pub number: Color32,
	pub constant: Color32,
	pub attribute: Color32,
	pub tag: Color32,
	pub punctuation: Color32,
	pub added: Color32,
	pub removed: Color32,
}
impl CodeColors {
	pub fn color(&self, token: crate::highlight::Token, plain: Color32) -> Color32 {
		use crate::highlight::Token;
		match token {
			Token::Plain => plain,
			Token::Keyword => self.keyword,
			Token::Type => self.type_name,
			Token::Function => self.function,
			Token::String => self.string,
			Token::Comment => self.comment,
			Token::Number => self.number,
			Token::Constant => self.constant,
			Token::Attribute => self.attribute,
			Token::Tag => self.tag,
			Token::Punctuation => self.punctuation,
			Token::Added => self.added,
			Token::Removed => self.removed,
		}
	}
}
pub fn code_colors(ui: &egui::Ui) -> CodeColors {
	let p = palette(ui);
	if ui.visuals().dark_mode {
		CodeColors {
			keyword: rgb(0xc792ea),
			type_name: rgb(0xffcb6b),
			function: rgb(0x82aaff),
			string: rgb(0xa5d97a),
			comment: p.muted,
			number: rgb(0xf78c6c),
			constant: rgb(0xf07178),
			attribute: rgb(0x89ddff),
			tag: rgb(0xf07178),
			punctuation: mix(p.text, p.muted, 0.5),
			added: p.positive,
			removed: p.danger,
		}
	} else {
		CodeColors {
			keyword: rgb(0x7c3aed),
			type_name: rgb(0xb45309),
			function: rgb(0x1d4ed8),
			string: rgb(0x15803d),
			comment: p.muted,
			number: rgb(0xc2410c),
			constant: rgb(0xbe185d),
			attribute: rgb(0x0e7490),
			tag: rgb(0xbe123c),
			punctuation: mix(p.text, p.muted, 0.5),
			added: p.positive,
			removed: p.danger,
		}
	}
}

/// Linear blend of two colours in premultiplied space; `t` = 0 keeps `a`, 1 gives `b`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
	let lerp = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
	Color32::from_rgba_premultiplied(
		lerp(a.r(), b.r()),
		lerp(a.g(), b.g()),
		lerp(a.b(), b.b()),
		lerp(a.a(), b.a()),
	)
}

#[cfg(test)]
mod extension_theme_tests {
	use super::*;
	#[test]
	fn extension_colors_keep_aliases_and_user_accent() {
		let theme = extensions::ThemePalette {
			colors: [
				("chat".into(), "#112233".into()),
				("sidebar".into(), "#445566".into()),
				("accent".into(), "#ff0000".into()),
			]
			.into(),
			backdrop: Some(["#010203".into(), "#040506".into()]),
			background: None,
		};
		let palette = recolor(
			builtin_colors(true, Variant::Standard),
			extension_palette(&theme).unwrap(),
		);
		assert_eq!(palette.chat, rgb(0x112233));
		assert_eq!(palette.canvas, palette.chat);
		assert_eq!(palette.surface, palette.sidebar);
		assert_eq!(palette.backdrop, Some([rgb(0x010203), rgb(0x040506)]));
		assert_eq!(customize(palette, Some([3, 4, 5])).accent, rgb(0x030405));
		let malformed = extensions::ThemePalette {
			colors: [("chat".into(), "invalid".into())].into(),
			..Default::default()
		};
		assert!(extension_palette(&malformed).is_none());
	}
}
