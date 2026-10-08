//! Offline check: cargo run --locked -p serein --example focused_keybinds
use eframe::egui;

fn main() {
	let legacy: model::Keybinds = serde_json::from_str("{}").unwrap();
	assert!(legacy.global_enabled);
	let store = local_store::LocalStore::open(std::path::Path::new(":memory:")).unwrap();
	let mut preferences = local_store::AppPreferences::default();
	preferences.keybinds.global_enabled = false;
	store.save_app_preferences(&preferences).unwrap();
	let mut view = ui::MessagingUi::default();
	view.keybinds = store.app_preferences().unwrap().keybinds;
	assert!(!view.keybinds.global_enabled);

	let runtime = tokio::runtime::Builder::new_current_thread()
		.build()
		.unwrap();
	let mut hotkeys = platform::hotkeys::Hotkeys::new(|| {});
	// Plain letters cannot register globally, so enabling this synthetic configuration
	// exercises switching back to focused input without capturing OS shortcuts.
	let mut plain = model::Keybinds::default();
	plain.toggle_mute.modifiers = 0;
	plain.toggle_deafen.modifiers = 0;
	hotkeys.sync(&plain, &runtime);
	hotkeys.sync(&view.keybinds, &runtime);
	assert_eq!(hotkeys.global_toggle_mask(), 0);
	assert_eq!(hotkeys.take_toggle_pending(), 0);
	assert!(!hotkeys.push_to_talk_down());
	assert!(hotkeys.status().contains("off"));

	for focused in [true, false] {
		let ctx = egui::Context::default();
		let modifiers = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
		ctx.run_ui(
			egui::RawInput {
				focused,
				events: [egui::Key::M, egui::Key::D, egui::Key::V]
					.into_iter()
					.map(|key| egui::Event::Key {
						key,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers,
					})
					.chain(std::iter::once(egui::Event::ModifiersChanged(modifiers)))
					.collect(),
				..Default::default()
			},
			|ui| {
				assert_eq!(
					view.voice_toggle_pressed(ui.ctx(), hotkeys.global_toggle_mask()),
					if focused { 3 } else { 0 }
				);
				view.keybinds.push_to_talk.modifiers =
					model::keybinds::PRIMARY | model::keybinds::SHIFT;
				assert_eq!(view.push_to_talk_down(ui.ctx()), focused);
			},
		)
		.drop_without_applying_deltas();
	}
	for key in [
		egui::Key::BrowserBack,
		egui::Key::BrowserForward,
		egui::Key::BrowserRefresh,
		egui::Key::BrowserSearch,
		egui::Key::BrowserHome,
		egui::Key::BrowserFavorites,
		egui::Key::BrowserStop,
		egui::Key::MediaPlayPause,
		egui::Key::MediaTrackNext,
		egui::Key::MediaTrackPrevious,
		egui::Key::MediaStop,
		egui::Key::AudioVolumeMute,
		egui::Key::AudioVolumeDown,
		egui::Key::AudioVolumeUp,
		egui::Key::LaunchMail,
		egui::Key::LaunchApp1,
		egui::Key::LaunchApp2,
		egui::Key::CapsLock,
		egui::Key::NumLock,
		egui::Key::ScrollLock,
		egui::Key::PrintScreen,
		egui::Key::Pause,
		egui::Key::Menu,
		egui::Key::Fn,
		egui::Key::Eject,
		egui::Key::Help,
		egui::Key::Power,
		egui::Key::Sleep,
		egui::Key::Clear,
	] {
		view.keybinds.toggle_mute = model::KeyChord::new(key.name(), 0);
		preferences.keybinds = view.keybinds.clone();
		store.save_app_preferences(&preferences).unwrap();
		view.keybinds = store.app_preferences().unwrap().keybinds;
		for focused in [true, false] {
			let ctx = egui::Context::default();
			ctx.run_ui(
				egui::RawInput {
					focused,
					events: vec![egui::Event::Key {
						key,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
					..Default::default()
				},
				|ui| {
					assert_eq!(
						view.voice_toggle_pressed(ui.ctx(), 0),
						u8::from(focused),
						"{}",
						key.name()
					);
				},
			)
			.drop_without_applying_deltas();
		}
	}
	println!(
		"Focused keybind check passed: saved preference, disabled global input, focused mute/deafen/PTT, media/browser/system keys and unfocused suppression."
	);
}
