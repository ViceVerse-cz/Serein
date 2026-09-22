//! Serein's egui design tokens, bundled fonts and Phosphor icons, adapted to GPUI.
use gpui::{prelude::*, *};
use std::borrow::Cow;

pub fn color(value: egui::Color32) -> Rgba {
	let [r, g, b, a] = value.to_srgba_unmultiplied();
	rgba((u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | u32::from(a))
}
pub fn palette() -> ui::design::Palette {
	ui::design::colors(true, ui::design::Variant::Standard)
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
	CaretDown,
	CaretRight,
	Chats,
	Copy,
	Download,
	File,
	FileImage,
	FileText,
	Forum,
	Hash,
	Megaphone,
	Send,
	Reply,
	Serein,
	Speaker,
	Users,
	Close,
}
impl Icon {
	fn path(self) -> &'static str {
		match self {
			Self::CaretDown => "icons/caret-down.svg",
			Self::CaretRight => "icons/caret-right.svg",
			Self::Chats => "icons/chat-centered-text.svg",
			Self::Copy => "icons/copy.svg",
			Self::Download => "icons/download-simple.svg",
			Self::File => "icons/file.svg",
			Self::FileImage => "icons/file-image.svg",
			Self::FileText => "icons/file-text.svg",
			Self::Forum => "icons/chats.svg",
			Self::Hash => "icons/hash.svg",
			Self::Megaphone => "icons/megaphone-simple.svg",
			Self::Send => "icons/paper-plane-right.svg",
			Self::Reply => "icons/arrow-bend-up-left.svg",
			Self::Serein => "icons/serein-mark.svg",
			Self::Speaker => "icons/speaker-high.svg",
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
			"caret-down",
			"caret-right",
			"chat-centered-text",
			"chats",
			"copy",
			"download-simple",
			"file",
			"file-image",
			"file-text",
			"hash",
			"megaphone-simple",
			"paper-plane-right",
			"speaker-high",
			"users",
			"x",
		)
		.map(Cow::Borrowed))
	}
	fn list(&self, _: &str) -> Result<Vec<SharedString>> {
		Ok(Vec::new())
	}
}
