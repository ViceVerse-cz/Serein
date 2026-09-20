//! Phosphor Icons (MIT), Simple Icons brand marks (CC0) and the Serein mark rasterized once
//! into one bundled atlas and tinted at draw time.
//!
//! `assets/icons/atlas.png` holds white glyphs on transparency in fixed 64px cells;
//! `index.tsv` maps upstream icon names to cells. See `assets/icons/README.md` for provenance.
use crate::design;
use egui::emath::GuiRounding;
use egui::{Color32, Rect, Response, Sense, TextureHandle, Vec2};
use std::sync::OnceLock;

const ATLAS: &[u8] = include_bytes!("../../../assets/icons/atlas.png");
const INDEX: &str = include_str!("../../../assets/icons/index.tsv");
const COLUMNS: usize = 8;
const CELL: f32 = 64.0;
const TEXTURE_KEY: &str = "phosphor-icons";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
	ChevronDown,
	ChevronRight,
	Gear,
	Microphone,
	MicrophoneSlash,
	Headphones,
	HeadphonesSlash,
	Pin,
	People,
	AddPeople,
	Profile,
	Search,
	Plus,
	Smile,
	Bell,
	Phone,
	InCall,
	HangUp,
	Video,
	VideoSlash,
	ScreenShare,
	Activities,
	Soundboard,
	Reply,
	Pencil,
	More,
	Inbox,
	Help,
	Reload,
	Threads,
	Speaker,
	Hash,
	Forum,
	Send,
	Attach,
	Close,
	External,
	GitHub,
	Twitch,
	Steam,
	Spotify,
	YouTube,
	XLogo,
	Reddit,
	Facebook,
	Instagram,
	TikTok,
	PayPal,
	Amazon,
	Bluesky,
	Mastodon,
	Skype,
	GameController,
	Television,
	Globe,
	Link,
	Copy,
	Verified,
	Calendar,
	Serein,
	File,
	FileImage,
	FilePdf,
	FileZip,
	FileText,
	FileCode,
	FileAudio,
	FileVideo,
	Trash,
	ArrowDown,
	ArrowUp,
	Check,
	Gif,
	Star,
	StarFill,
	Fire,
	ArrowLeft,
	PlayStation,
	BattleNet,
	EpicGames,
	LeagueOfLegends,
	RiotGames,
	Bungie,
	Roblox,
	Crunchyroll,
	Ebay,
	Folder,
	FolderOpen,
	CaretLeft,
	Download,
	ArrowRight,
	Image,
	Sparkle,
	Compass,
	Megaphone,
	ShieldWarning,
	Crown,
	ChartBar,
	ShoppingCart,
	Lock,
	EyeSlash,
	Sliders,
	SortArrows,
	Thread,
}

impl Icon {
	pub const ALL: [Icon; 104] = [
		Icon::ChevronDown,
		Icon::ChevronRight,
		Icon::Gear,
		Icon::Microphone,
		Icon::MicrophoneSlash,
		Icon::Headphones,
		Icon::HeadphonesSlash,
		Icon::Pin,
		Icon::People,
		Icon::AddPeople,
		Icon::Profile,
		Icon::Search,
		Icon::Plus,
		Icon::Smile,
		Icon::Bell,
		Icon::Phone,
		Icon::InCall,
		Icon::HangUp,
		Icon::Video,
		Icon::VideoSlash,
		Icon::ScreenShare,
		Icon::Activities,
		Icon::Soundboard,
		Icon::Reply,
		Icon::Pencil,
		Icon::More,
		Icon::Inbox,
		Icon::Help,
		Icon::Reload,
		Icon::Threads,
		Icon::Speaker,
		Icon::Hash,
		Icon::Forum,
		Icon::Send,
		Icon::Attach,
		Icon::Close,
		Icon::External,
		Icon::GitHub,
		Icon::Twitch,
		Icon::Steam,
		Icon::Spotify,
		Icon::YouTube,
		Icon::XLogo,
		Icon::Reddit,
		Icon::Facebook,
		Icon::Instagram,
		Icon::TikTok,
		Icon::PayPal,
		Icon::Amazon,
		Icon::Bluesky,
		Icon::Mastodon,
		Icon::Skype,
		Icon::GameController,
		Icon::Television,
		Icon::Globe,
		Icon::Link,
		Icon::Copy,
		Icon::Verified,
		Icon::Calendar,
		Icon::Serein,
		Icon::File,
		Icon::FileImage,
		Icon::FilePdf,
		Icon::FileZip,
		Icon::FileText,
		Icon::FileCode,
		Icon::FileAudio,
		Icon::FileVideo,
		Icon::Trash,
		Icon::ArrowDown,
		Icon::ArrowUp,
		Icon::Check,
		Icon::Gif,
		Icon::Star,
		Icon::StarFill,
		Icon::Fire,
		Icon::ArrowLeft,
		Icon::PlayStation,
		Icon::BattleNet,
		Icon::EpicGames,
		Icon::LeagueOfLegends,
		Icon::RiotGames,
		Icon::Bungie,
		Icon::Roblox,
		Icon::Crunchyroll,
		Icon::Ebay,
		Icon::Folder,
		Icon::FolderOpen,
		Icon::CaretLeft,
		Icon::Download,
		Icon::ArrowRight,
		Icon::Image,
		Icon::Sparkle,
		Icon::Compass,
		Icon::Megaphone,
		Icon::ShieldWarning,
		Icon::Crown,
		Icon::ChartBar,
		Icon::ShoppingCart,
		Icon::Lock,
		Icon::EyeSlash,
		Icon::Sliders,
		Icon::SortArrows,
		Icon::Thread,
	];
	/// Upstream icon name recorded in `index.tsv`.
	fn asset(self) -> &'static str {
		match self {
			Icon::Folder => "folder",
			Icon::FolderOpen => "folder-open",
			Icon::ChevronDown => "caret-down",
			Icon::ChevronRight => "caret-right",
			Icon::Gear => "gear",
			Icon::Microphone => "microphone",
			Icon::MicrophoneSlash => "microphone-slash",
			Icon::Headphones => "headphones",
			Icon::HeadphonesSlash => "headphones-slash",
			Icon::Pin => "push-pin",
			Icon::People => "users",
			Icon::AddPeople => "user-plus",
			Icon::Profile => "user-circle",
			Icon::Search => "magnifying-glass",
			Icon::Plus => "plus",
			Icon::Smile => "smiley",
			Icon::Bell => "bell",
			Icon::Phone => "phone",
			Icon::InCall => "phone-call",
			Icon::HangUp => "phone-disconnect",
			Icon::Video => "video-camera",
			Icon::VideoSlash => "video-camera-slash",
			Icon::ScreenShare => "monitor-arrow-up",
			Icon::Activities => "rocket-launch",
			Icon::Soundboard => "waveform",
			Icon::Reply => "arrow-bend-up-left",
			Icon::Pencil => "pencil-simple",
			Icon::More => "dots-three",
			Icon::Inbox => "tray",
			Icon::Help => "question",
			Icon::Reload => "arrow-clockwise",
			Icon::Threads => "chats",
			Icon::Thread => "thread",
			Icon::Speaker => "speaker-high",
			Icon::Hash => "hash",
			Icon::Forum => "chat-centered-text",
			Icon::Send => "paper-plane-right",
			Icon::Attach => "plus-circle",
			Icon::Close => "x",
			Icon::External => "arrow-square-out",
			Icon::GitHub => "github-logo",
			Icon::Twitch => "twitch-logo",
			Icon::Steam => "steam-logo",
			Icon::Spotify => "spotify-logo",
			Icon::YouTube => "youtube-logo",
			Icon::XLogo => "x-logo",
			Icon::Reddit => "reddit-logo",
			Icon::Facebook => "facebook-logo",
			Icon::Instagram => "instagram-logo",
			Icon::TikTok => "tiktok-logo",
			Icon::PayPal => "paypal-logo",
			Icon::Amazon => "amazon-logo",
			Icon::Bluesky => "bluesky",
			Icon::Mastodon => "mastodon-logo",
			Icon::Skype => "skype-logo",
			Icon::GameController => "game-controller",
			Icon::Television => "television",
			Icon::Globe => "globe",
			Icon::Link => "link",
			Icon::Copy => "copy",
			Icon::Verified => "seal-check",
			Icon::Calendar => "calendar-blank",
			Icon::Serein => "serein-mark",
			Icon::File => "file",
			Icon::FileImage => "file-image",
			Icon::FilePdf => "file-pdf",
			Icon::FileZip => "file-zip",
			Icon::FileText => "file-text",
			Icon::FileCode => "file-code",
			Icon::FileAudio => "file-audio",
			Icon::FileVideo => "file-video",
			Icon::Trash => "trash",
			Icon::ArrowDown => "arrow-down",
			Icon::ArrowUp => "arrow-up",
			Icon::Check => "check",
			Icon::Gif => "gif",
			Icon::Star => "star",
			Icon::StarFill => "star-fill",
			Icon::Fire => "fire",
			Icon::ArrowLeft => "arrow-left",
			Icon::PlayStation => "playstation",
			Icon::BattleNet => "battle-net",
			Icon::EpicGames => "epic-games",
			Icon::LeagueOfLegends => "league-of-legends",
			Icon::RiotGames => "riot-games",
			Icon::Bungie => "bungie",
			Icon::Roblox => "roblox",
			Icon::Crunchyroll => "crunchyroll",
			Icon::Ebay => "ebay",
			Icon::CaretLeft => "caret-left",
			Icon::Download => "download-simple",
			Icon::ArrowRight => "arrow-right",
			Icon::Image => "image",
			Icon::Sparkle => "sparkle",
			Icon::Compass => "compass",
			Icon::Megaphone => "megaphone-simple",
			Icon::ShieldWarning => "shield-warning",
			Icon::Crown => "crown",
			Icon::ChartBar => "chart-bar",
			Icon::ShoppingCart => "shopping-cart-simple",
			Icon::Lock => "lock-simple",
			Icon::EyeSlash => "eye-slash",
			Icon::Sliders => "sliders-horizontal",
			Icon::SortArrows => "arrows-down-up",
		}
	}
	fn cell(self) -> usize {
		// Resolved once from the bundled index, then a plain array lookup per paint.
		static CELLS: OnceLock<[usize; Icon::ALL.len()]> = OnceLock::new();
		CELLS.get_or_init(|| {
			let mut cells = [0; Icon::ALL.len()];
			for icon in Icon::ALL {
				let asset = icon.asset();
				cells[icon as usize] = INDEX
					.lines()
					.find_map(|line| {
						let (name, cell) = line.split_once('\t').expect("bundled icon index");
						(name == asset).then(|| cell.parse().expect("bundled icon cell"))
					})
					.expect("every icon is in the bundled atlas");
			}
			cells
		})[self as usize]
	}
}

fn decoded() -> &'static egui::ColorImage {
	static IMAGE: OnceLock<egui::ColorImage> = OnceLock::new();
	IMAGE.get_or_init(|| {
		let image = image::load_from_memory_with_format(ATLAS, image::ImageFormat::Png)
			.expect("bundled icon atlas")
			.into_rgba8();
		let size = [image.width() as usize, image.height() as usize];
		egui::ColorImage::from_rgba_unmultiplied(size, &image)
	})
}

/// Upload the atlas for `ctx` during application creation, outside the render callback.
pub fn install(ctx: &egui::Context) {
	let _ = texture(ctx);
}

fn texture(ctx: &egui::Context) -> TextureHandle {
	let id = egui::Id::unique(TEXTURE_KEY);
	if let Some(texture) = ctx.data(|data| data.get_temp::<TextureHandle>(id)) {
		return texture;
	}
	let texture = ctx.load_texture(
		"Phosphor Icons 2.1.1",
		decoded().clone(),
		egui::TextureOptions {
			mipmap_mode: Some(egui::TextureFilter::Linear),
			..egui::TextureOptions::LINEAR
		},
	);
	ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
	texture
}

/// Paint `icon` centred in `rect` with `color`.
pub fn paint(painter: &egui::Painter, icon: Icon, rect: Rect, color: Color32) {
	let size = rect.width().min(rect.height());
	let rect = Rect::from_center_size(rect.center(), Vec2::splat(size))
		.round_to_pixels(painter.pixels_per_point());
	let texture = texture(painter.ctx());
	let [width, height] = texture.size();
	let cell = icon.cell();
	let x = (cell % COLUMNS) as f32 * CELL;
	let y = (cell / COLUMNS) as f32 * CELL;
	let uv = Rect::from_min_max(
		egui::pos2(x / width as f32, y / height as f32),
		egui::pos2((x + CELL) / width as f32, (y + CELL) / height as f32),
	);
	// Glyphs occupy 56 of every 64 cell pixels; draw the cell slightly larger so the visible
	// glyph fills `rect` like the previous painted icons did.
	painter.image(texture.id(), rect.expand(size * 4.0 / 56.0), uv, color);
}

/// Square icon button that highlights on hover and exposes `label` to accessibility.
pub fn button(ui: &mut egui::Ui, icon: Icon, size: f32, label: &str) -> Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
	if response.hovered() || response.has_focus() {
		ui.painter().rect_filled(rect, 6, colors.hover);
	}
	let color = if !ui.is_enabled() {
		colors.muted.gamma_multiply(0.5)
	} else if response.hovered() || response.has_focus() {
		colors.text_strong
	} else {
		colors.muted
	};
	paint(ui.painter(), icon, rect.shrink(size * 0.2), color);
	response.widget_info(|| {
		egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
	});
	response.on_hover_text(label)
}

/// Toggleable variant: `active` keeps the icon in the strong text colour.
pub fn toggle(ui: &mut egui::Ui, icon: Icon, size: f32, active: bool, label: &str) -> Response {
	let colors = design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
	if response.hovered() || response.has_focus() {
		ui.painter().rect_filled(rect, 6, colors.hover);
	}
	let color = if active || response.hovered() || response.has_focus() {
		colors.text_strong
	} else {
		colors.muted
	};
	paint(ui.painter(), icon, rect.shrink(size * 0.2), color);
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), active, label)
	});
	response.on_hover_text(label)
}

/// Inline glyph used beside labels (channel kinds, section headers).
pub fn inline(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> Rect {
	let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
	paint(ui.painter(), icon, rect, color);
	rect
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn atlas_covers_every_icon_once_and_paints_inside_its_rect() {
		let image = decoded();
		let rows = Icon::ALL.len().div_ceil(COLUMNS);
		assert_eq!(image.size, [COLUMNS * CELL as usize, rows * CELL as usize]);
		assert!(
			ATLAS.len() < 256 * 1024,
			"atlas stays a small bundled asset"
		);
		let mut cells: Vec<usize> = Icon::ALL.iter().map(|icon| icon.cell()).collect();
		cells.sort_unstable();
		cells.dedup();
		assert_eq!(cells.len(), Icon::ALL.len(), "icons map to distinct cells");
		assert_eq!(
			INDEX.lines().count(),
			Icon::ALL.len(),
			"index has no unused cells"
		);
		for icon in Icon::ALL {
			// Every cell holds visible glyph pixels.
			let cell = icon.cell();
			let (x0, y0) = (
				(cell % COLUMNS) * CELL as usize,
				(cell / COLUMNS) * CELL as usize,
			);
			let opaque = (0..CELL as usize)
				.flat_map(|dy| (0..CELL as usize).map(move |dx| (dx, dy)))
				.filter(|(dx, dy)| image[(x0 + dx, y0 + dy)].a() > 128)
				.count();
			assert!(opaque > 40, "{icon:?} cell is blank");
		}
		let ctx = egui::Context::default();
		let output = ctx.run_ui(Default::default(), |ui| {
			for icon in Icon::ALL {
				let rect = Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::splat(24.0));
				paint(ui.painter(), icon, rect, Color32::WHITE);
				assert!(button(ui, icon, 32.0, "icon").rect.width() == 32.0);
			}
		});
		assert!(
			output.textures_delta.set.len() <= 2,
			"fonts plus one atlas upload"
		);
		for shape in &output.shapes {
			let bounds = shape.shape.visual_bounding_rect();
			assert!(bounds.is_negative() || bounds.min.x >= -1.0, "{:?}", bounds);
		}
		output.drop_without_applying_deltas();
	}
}
