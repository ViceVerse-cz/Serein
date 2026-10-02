//! One native font picker and one bounded decode; persistence uses the cache worker.
use std::{
	io::Read,
	path::Path,
	sync::{Arc, mpsc},
};
use ui::fonts::{CustomFont, MAX_CUSTOM_FONT_BYTES};

pub type Selected = Result<Option<CustomFont>, &'static str>;

pub fn choose(
	runtime: &tokio::runtime::Runtime,
	ctx: &eframe::egui::Context,
	parent: Arc<winit::window::Window>,
) -> mpsc::Receiver<Selected> {
	let dialog = platform::save::font_source(parent);
	let (send, receive) = mpsc::sync_channel(1);
	let ctx = ctx.clone();
	runtime.spawn(async move {
		let result = match dialog.await {
			Some(path) => tokio::task::spawn_blocking(move || read(&path).map(Some))
				.await
				.unwrap_or(Err("Font import interrupted. Try again.")),
			None => Ok(None),
		};
		let _ = send.send(result);
		ctx.request_repaint();
	});
	receive
}

fn read(path: &Path) -> Result<CustomFont, &'static str> {
	let metadata = std::fs::symlink_metadata(path).map_err(|_| "Could not open the font.")?;
	if !metadata.is_file() {
		return Err("Choose a regular TTF or OTF file.");
	}
	if metadata.len() == 0 || metadata.len() > MAX_CUSTOM_FONT_BYTES as u64 {
		return Err("Choose a font up to 8 MiB.");
	}
	let file = std::fs::File::open(path).map_err(|_| "Could not open the font.")?;
	let mut bytes = Vec::with_capacity(metadata.len() as usize);
	file.take(MAX_CUSTOM_FONT_BYTES as u64 + 1)
		.read_to_end(&mut bytes)
		.map_err(|_| "Could not read the font.")?;
	let name: String = path
		.file_stem()
		.unwrap_or_default()
		.to_string_lossy()
		.chars()
		.filter(|c| !c.is_control())
		.take(32)
		.collect();
	CustomFont::new(
		if name.is_empty() {
			"Custom font".into()
		} else {
			name
		},
		bytes,
	)
}

#[cfg(all(debug_assertions, feature = "demo"))]
pub fn debug_check() {
	use eframe::egui::{self, FontFamily};
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
	std::fs::remove_dir_all(directory).unwrap();

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
		if ctx.fonts(|fonts| {
			fonts
				.definitions()
				.font_data
				.contains_key("Noto Sans CJK JP")
		}) {
			break;
		}
		std::thread::sleep(std::time::Duration::from_millis(10));
	}
	assert_ne!(original, ui::fonts::revision(&ctx));
	ctx.fonts(|fonts| {
		let definitions = fonts.definitions();
		assert!(definitions.font_data.contains_key("Noto Sans CJK JP"));
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
		assert!(
			fonts
				.definitions()
				.font_data
				.contains_key("Noto Sans CJK JP")
		);
		assert!(!fonts.definitions().font_data.contains_key("Serein Custom"));
	});
	println!(
		"Font debug check passed: bounded import, invalid input, saved copy, replacement during CJK loading, reset, and saved decoration preference."
	);
}
