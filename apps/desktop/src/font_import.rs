//! Installed-family enumeration and one bounded face load; persistence uses the cache worker.
use std::sync::mpsc;
use ui::fonts::CustomFont;

pub type Selected = Result<Option<CustomFont>, &'static str>;
/// Upper bound on listed families; real systems carry a few hundred to a couple thousand.
const MAX_FAMILIES: usize = 4096;

fn collection() -> fontique::Collection {
	fontique::Collection::new(fontique::CollectionOptions {
		shared: false,
		system_fonts: true,
	})
}

/// Sorted, user-visible installed family names, enumerated off the UI thread.
pub fn installed(
	runtime: &tokio::runtime::Runtime,
	ctx: &eframe::egui::Context,
) -> mpsc::Receiver<Vec<String>> {
	let (send, receive) = mpsc::sync_channel(1);
	let ctx = ctx.clone();
	runtime.spawn(async move {
		let families = tokio::task::spawn_blocking(|| {
			let mut names: Vec<String> = collection()
				.family_names()
				// Dot-prefixed families are private system UI faces.
				.filter(|name| {
					!name.is_empty()
						&& !name.starts_with('.')
						&& name.len() <= 128
						&& !name.chars().any(char::is_control)
				})
				.take(MAX_FAMILIES)
				.map(str::to_owned)
				.collect();
			names.sort_by_cached_key(|name| name.to_lowercase());
			names.dedup();
			names
		})
		.await
		.unwrap_or_default();
		let _ = send.send(families);
		ctx.request_repaint();
	});
	receive
}

/// Load the family's upright regular face as a standalone, validated font.
pub fn load(
	runtime: &tokio::runtime::Runtime,
	ctx: &eframe::egui::Context,
	family: String,
) -> mpsc::Receiver<Selected> {
	let (send, receive) = mpsc::sync_channel(1);
	let ctx = ctx.clone();
	runtime.spawn(async move {
		let result = tokio::task::spawn_blocking(move || read_family(family).map(Some))
			.await
			.unwrap_or(Err("Font loading interrupted. Try again."));
		let _ = send.send(result);
		ctx.request_repaint();
	});
	receive
}

fn read_family(name: String) -> Result<CustomFont, &'static str> {
	let mut collection = collection();
	let family = collection
		.family_by_name(&name)
		.ok_or("This font is no longer installed.")?;
	let font = family
		.fonts()
		.iter()
		.filter(|font| font.style() == fontique::FontStyle::Normal)
		.min_by_key(|font| {
			(
				(font.width().ratio() - 1.0).abs().to_bits(),
				(font.weight().value() - 400.0).abs().to_bits(),
			)
		})
		.or_else(|| family.default_font())
		.ok_or("This font has no usable style.")?;
	let blob = font.load(None).ok_or("Could not read the font.")?;
	let bytes = ui::fonts::standalone_face(blob.as_ref(), font.index())?;
	CustomFont::new(name, bytes)
}

#[cfg(all(debug_assertions, feature = "demo"))]
fn read(path: &std::path::Path) -> Result<CustomFont, &'static str> {
	let bytes = std::fs::read(path).map_err(|_| "Could not read the font.")?;
	CustomFont::new(
		path.file_stem()
			.unwrap_or_default()
			.to_string_lossy()
			.into_owned(),
		bytes,
	)
}

#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_check() {
	use eframe::egui::{self, FontFamily};
	use std::path::Path;
	use ui::fonts::MAX_CUSTOM_FONT_BYTES;
	let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts");
	let font = read(&assets.join("Inter-Regular.ttf")).unwrap();
	let replacement = read(&assets.join("Inter-SemiBold.ttf")).unwrap();
	assert!(CustomFont::new("Invalid".into(), b"not a font".to_vec()).is_err());
	let mut random = [0_u8; 8];
	getrandom::fill(&mut random).unwrap();
	let directory = std::env::temp_dir().join(format!("serein-font-debug-{random:02x?}"));
	std::fs::create_dir(&directory).unwrap();
	let oversized = directory.join("large.ttf");
	std::fs::File::create(&oversized)
		.unwrap()
		.set_len(MAX_CUSTOM_FONT_BYTES as u64 + 1)
		.unwrap();
	assert!(read(&oversized).is_err());
	let path = directory.join("preferences.sqlite3");
	let store = local_store::LocalStore::open(&path).unwrap();
	store
		.save_custom_font(Some((&font.name, font.bytes())))
		.unwrap();
	let preferences = local_store::AppPreferences {
		hide_window_decorations: true,
		..Default::default()
	};
	store.save_app_preferences(&preferences).unwrap();
	drop(store);
	let store = local_store::LocalStore::open(&path).unwrap();
	let (name, bytes) = store.custom_font().unwrap().unwrap();
	let restored = CustomFont::new(name, bytes).unwrap();
	assert_eq!(restored.bytes(), font.bytes());
	assert!(store.save_custom_font(Some(("", font.bytes()))).is_err());
	assert_eq!(store.custom_font().unwrap().unwrap().0, font.name);
	let mut view = ui::MessagingUi::default();
	crate::app_settings::Settings {
		current: store.app_preferences().unwrap(),
		..Default::default()
	}
	.apply(&mut view);
	assert!(view.hide_window_decorations);
	store.save_custom_font(None).unwrap();
	assert!(store.custom_font().unwrap().is_none());
	drop(store);

	let ctx = egui::Context::default();
	ui::fonts::install(&ctx);
	ui::fonts::apply_custom(&ctx, Some(&restored));
	let frame = |text: &str| {
		ctx.run_ui(egui::RawInput::default(), |ui| {
			ui.label(text);
		})
		.drop_without_applying_deltas();
	};
	frame("Custom font 日本語");
	let original = ui::fonts::revision(&ctx);
	ui::fonts::apply_custom(&ctx, Some(&replacement));
	for _ in 0..100 {
		frame("Replacement font 日本語");
		if ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")) {
			break;
		}
		std::thread::sleep(std::time::Duration::from_millis(10));
	}
	assert_ne!(original, ui::fonts::revision(&ctx));
	ctx.fonts(|fonts| {
		let definitions = fonts.definitions();
		assert!(definitions.font_data.contains_key("Noto Sans CJK"));
		assert_eq!(
			definitions.families[&FontFamily::Proportional][0],
			"Serein Custom"
		);
		assert_eq!(
			definitions.font_data["Serein Custom"].bytes(),
			replacement.bytes()
		);
		assert!(
			!definitions.families[&FontFamily::Monospace]
				.iter()
				.any(|name| name.starts_with("Serein Custom"))
		);
	});
	ui::fonts::apply_custom(&ctx, None);
	frame("Back to Inter");
	ctx.fonts(|fonts| {
		assert_eq!(
			fonts.definitions().families[&FontFamily::Proportional][0],
			"Inter"
		);
		assert!(fonts.definitions().font_data.contains_key("Noto Sans CJK"));
		assert!(!fonts.definitions().font_data.contains_key("Serein Custom"));
	});
	let collection = ctx.fonts(|fonts| {
		fonts.definitions().font_data["Noto Sans CJK"]
			.bytes()
			.to_vec()
	});
	for index in [0, 2, 3] {
		let bytes = ui::fonts::standalone_face(&collection, index).unwrap();
		assert!(bytes.len() > 8 * 1024 * 1024);
		let font = CustomFont::new("CJK".into(), bytes).unwrap();
		let store = local_store::LocalStore::open(&path).unwrap();
		store
			.save_custom_font(Some((&font.name, font.bytes())))
			.unwrap();
		drop(store);
		let store = local_store::LocalStore::open(&path).unwrap();
		assert_eq!(store.custom_font().unwrap().unwrap().1, font.bytes());
	}
	ui::fonts::apply_custom(&ctx, Some(&replacement));
	ui::i18n::set_current(ui::i18n::Language::English);
	frame("中文 日本語");
	frame("中文 日本語");
	for (language, index) in [
		(ui::i18n::Language::ChineseSimplified, 2),
		(ui::i18n::Language::ChineseTraditional, 3),
		(ui::i18n::Language::Japanese, 0),
	] {
		let before = ui::fonts::revision(&ctx);
		ui::i18n::set_current(language);
		frame("中文 日本語");
		frame("中文 日本語");
		assert_ne!(before, ui::fonts::revision(&ctx));
		ctx.fonts(|fonts| {
			let definitions = fonts.definitions();
			assert_eq!(definitions.font_data["Noto Sans CJK"].index, index);
			assert_eq!(
				definitions.families[&FontFamily::Proportional][0],
				"Serein Custom"
			);
		});
	}
	ui::i18n::set_current(ui::i18n::Language::System);
	std::fs::remove_dir_all(directory).unwrap();
	println!(
		"Font debug check passed: bounded CJK import and persistence, locale switching, invalid input, replacement during CJK loading, reset, and saved decoration preference."
	);
}
