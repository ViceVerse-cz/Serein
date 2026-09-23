//! Serein's egui design tokens, bundled fonts and Phosphor icons, adapted to GPUI.
use gpui::{prelude::*, *};
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

pub fn color(value: egui::Color32) -> Rgba {
	let [r, g, b, a] = value.to_srgba_unmultiplied();
	rgba((u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | u32::from(a))
}
/// Light/dark preference; `System` follows the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Appearance {
	Dark,
	Light,
	System,
}
impl Appearance {
	pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::System];
	fn is_dark(self, system_dark: bool) -> bool {
		match self {
			Self::Dark => true,
			Self::Light => false,
			Self::System => system_dark,
		}
	}
}

// Process-wide statics (saved by `persist`), because `palette()` is called from render code
// without a context. The colour variant and accent live in `ui::design`'s own statics, as in egui.
static APPEARANCE: AtomicU8 = AtomicU8::new(Appearance::System as u8);
static SYSTEM_DARK: AtomicBool = AtomicBool::new(true);

pub fn appearance() -> Appearance {
	Appearance::ALL
		.into_iter()
		.find(|mode| *mode as u8 == APPEARANCE.load(Ordering::Relaxed))
		.unwrap_or(Appearance::System)
}
pub fn set_appearance(mode: Appearance) {
	APPEARANCE.store(mode as u8, Ordering::Relaxed);
}
/// Records the window's appearance for [`Appearance::System`].
pub fn set_system_appearance(appearance: WindowAppearance) {
	let dark = matches!(
		appearance,
		WindowAppearance::Dark | WindowAppearance::VibrantDark
	);
	SYSTEM_DARK.store(dark, Ordering::Relaxed);
}
pub fn dark() -> bool {
	appearance().is_dark(SYSTEM_DARK.load(Ordering::Relaxed))
}
pub fn palette() -> ui::design::Palette {
	ui::design::colors(dark(), ui::design::variant())
}
/// Premultiplied `top` composited over `bottom`.
fn over(top: egui::Color32, bottom: egui::Color32) -> egui::Color32 {
	let keep = 255 - u32::from(top.a());
	let channel = |t: u8, b: u8| (u32::from(t) + u32::from(b) * keep / 255).min(255) as u8;
	egui::Color32::from_rgba_premultiplied(
		channel(top.r(), bottom.r()),
		channel(top.g(), bottom.g()),
		channel(top.b(), bottom.b()),
		channel(top.a(), bottom.a()),
	)
}
/// Window fill: the frame colour, over the gradient backdrop for gradient variants.
pub fn window_background() -> Background {
	let p = palette();
	match p.backdrop {
		Some([top, bottom]) => linear_gradient(
			135.,
			linear_color_stop(color(over(p.base, top)), 0.),
			linear_color_stop(color(over(p.base, bottom)), 1.),
		),
		None => color(p.base).into(),
	}
}
/// A palette surface made opaque for floating popovers; gradient variants use translucent
/// surfaces that would otherwise show the conversation through them.
pub fn solid(value: egui::Color32) -> Rgba {
	let p = palette();
	let under = p.backdrop.map_or(p.base, |[top, bottom]| {
		over(p.base, top.lerp_to_gamma(bottom, 0.5))
	});
	color(over(value, over(under, egui::Color32::BLACK)))
}
/// A palette colour at a reduced opacity, for tints over the chat surface.
pub fn tint(value: egui::Color32, alpha: f32) -> Rgba {
	let mut value = color(value);
	value.a *= alpha;
	value
}

pub const FONT: &str = "Inter";
const FONTS: [&[u8]; 3] = [
	include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
	include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
	include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
];
/// Registers the same bundled OFL Inter faces as the egui app; no system font scan.
pub fn install_fonts(cx: &App) {
	let fonts = FONTS.iter().map(|font| Cow::Borrowed(*font)).collect();
	if cx.text_system().add_fonts(fonts).is_err() {
		eprintln!("Bundled Inter fonts could not be registered; using the system font.");
	}
}

#[derive(Clone, Copy)]
pub enum Icon {
	ArrowDown,
	CaretDown,
	CaretRight,
	Chats,
	Check,
	Copy,
	Download,
	File,
	FileImage,
	FileText,
	FolderOpen,
	Forum,
	Gear,
	Hash,
	Link,
	Megaphone,
	Pencil,
	Pin,
	PlusCircle,
	Send,
	Reply,
	Search,
	Serein,
	Smiley,
	Speaker,
	Trash,
	Users,
	Close,
}
impl Icon {
	fn path(self) -> &'static str {
		match self {
			Self::ArrowDown => "icons/arrow-down.svg",
			Self::CaretDown => "icons/caret-down.svg",
			Self::CaretRight => "icons/caret-right.svg",
			Self::Chats => "icons/chat-centered-text.svg",
			Self::Check => "icons/check.svg",
			Self::Copy => "icons/copy.svg",
			Self::Download => "icons/download-simple.svg",
			Self::File => "icons/file.svg",
			Self::FileImage => "icons/file-image.svg",
			Self::FileText => "icons/file-text.svg",
			Self::Forum => "icons/chats.svg",
			Self::Gear => "icons/gear.svg",
			Self::Hash => "icons/hash.svg",
			Self::FolderOpen => "icons/folder-open.svg",
			Self::Link => "icons/link.svg",
			Self::Megaphone => "icons/megaphone-simple.svg",
			Self::Pencil => "icons/pencil-simple.svg",
			Self::Pin => "icons/push-pin.svg",
			Self::PlusCircle => "icons/plus-circle.svg",
			Self::Send => "icons/paper-plane-right.svg",
			Self::Reply => "icons/arrow-bend-up-left.svg",
			Self::Search => "icons/magnifying-glass.svg",
			Self::Serein => "icons/serein-mark.svg",
			Self::Smiley => "icons/smiley.svg",
			Self::Speaker => "icons/speaker-high.svg",
			Self::Trash => "icons/trash.svg",
			Self::Users => "icons/users.svg",
			Self::Close => "icons/x.svg",
		}
	}
}
pub fn icon(icon: Icon, size: Pixels, tint: impl Into<Hsla>) -> Svg {
	svg()
		.path(icon.path())
		.flex_none()
		.size(size)
		.text_color(tint)
}

/// Compiled-in icons only: nothing is read from disk or the network.
pub struct Assets;
impl AssetSource for Assets {
	fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
		macro_rules! icons {
			($($name:literal),* $(,)?) => {
				match path {
					"icons/serein-mark.svg" => Some(include_bytes!("../../../assets/brand/serein-mark.svg").as_slice()),
					$(concat!("icons/", $name, ".svg") => Some(include_bytes!(concat!("../assets/icons/", $name, ".svg")).as_slice()),)*
					_ => None,
				}
			};
		}
		Ok(icons!(
			"arrow-bend-up-left",
			"arrow-down",
			"caret-down",
			"caret-right",
			"chat-centered-text",
			"chats",
			"check",
			"copy",
			"download-simple",
			"file",
			"file-image",
			"file-text",
			"folder-open",
			"gear",
			"hash",
			"link",
			"magnifying-glass",
			"megaphone-simple",
			"paper-plane-right",
			"pencil-simple",
			"plus-circle",
			"push-pin",
			"smiley",
			"speaker-high",
			"trash",
			"users",
			"x",
		)
		.map(Cow::Borrowed))
	}
	fn list(&self, _: &str) -> Result<Vec<SharedString>> {
		Ok(Vec::new())
	}
}
