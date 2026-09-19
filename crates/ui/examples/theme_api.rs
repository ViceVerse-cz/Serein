//! Offline creator check: cargo run --locked -p ui --example theme_api

fn main() {
	let ctx = egui::Context::default();
	let package =
		extensions::parse_package(include_bytes!("../../../extensions/ocean.serein-extension"))
			.expect("existing color-only themes remain compatible");
	let mut theme = package.theme.unwrap();
	assert_eq!(theme.style, extensions::ThemeStyle::default());
	theme.style = extensions::ThemeStyle {
		transparency_blur: Some(true),
		transparency: Some(30),
		blur: Some(60),
		transparent_all: Some(true),
		body_size: Some(18),
		heading_size: Some(26),
		button_size: Some(17),
		small_size: Some(13),
		monospace_size: Some(16),
		item_spacing: Some([10, 6]),
		button_padding: Some([16, 8]),
		control_height: Some(40),
		widget_radius: Some(10),
		window_radius: Some(16),
		menu_radius: Some(12),
	};
	let overlay = extensions::Theme {
		style: extensions::ThemeStyle {
			button_size: Some(19),
			..Default::default()
		},
		..Default::default()
	};
	let mut merged = theme.clone();
	merged.overlay(&overlay);
	assert_eq!(merged.style.button_size, Some(19));
	assert_eq!(merged.style.body_size, Some(18));
	assert_eq!(merged.style.transparency_blur, Some(true));
	assert_eq!(merged.style.transparency, Some(30));
	assert_eq!(merged.dark, theme.dark);
	theme.validate().unwrap();
	ui::design::set_extension_theme(Some(&theme));
	assert!(!ui::design::window_effects().0);
	ui::design::set_window_effects(true, 15, 50, false);
	assert_eq!(ui::design::window_effects(), (true, 30, 60, true));
	ui::design::apply(&ctx);
	for appearance in [egui::Theme::Dark, egui::Theme::Light] {
		let style = ctx.style_of(appearance);
		for (kind, size) in [
			(egui::TextStyle::Body, 18.0),
			(egui::TextStyle::Heading, 26.0),
			(egui::TextStyle::Button, 17.0),
			(egui::TextStyle::Small, 13.0),
			(egui::TextStyle::Monospace, 16.0),
		] {
			assert_eq!(style.text_styles[&kind].size, size);
		}
		assert_eq!(style.spacing.item_spacing, egui::vec2(10.0, 6.0));
		assert_eq!(style.spacing.button_padding, egui::vec2(16.0, 8.0));
		assert_eq!(style.spacing.interact_size.y, 40.0);
		assert_eq!(style.visuals.widgets.inactive.corner_radius, 10.into());
		assert_eq!(style.visuals.window_corner_radius, 16.into());
		assert_eq!(style.visuals.menu_corner_radius, 12.into());
	}
	let mut invalid = theme.clone();
	invalid.style.transparency = Some(101);
	assert!(invalid.validate().is_err());
	invalid.style.transparency = Some(30);
	invalid.style.body_size = Some(0);
	assert!(invalid.validate().is_err());
	invalid = theme;
	invalid.style.widget_radius = Some(255);
	assert!(invalid.validate().is_err());
	// Invalid input must not leave the previously active theme partially applied.
	ui::design::set_extension_theme(Some(&invalid));
	ui::design::apply(&ctx);
	assert_eq!(
		ctx.style_of(egui::Theme::Dark).text_styles[&egui::TextStyle::Body].size,
		15.0
	);
	assert_eq!(
		ctx.style_of(egui::Theme::Dark)
			.visuals
			.widgets
			.inactive
			.corner_radius,
		8.into()
	);
	ui::design::set_extension_theme(None);
	assert_eq!(ui::design::window_effects(), (true, 15, 50, false));
	ui::design::set_window_effects(true, 30, 60, false);
	let focused = ui::design::colors(true, ui::design::Variant::Standard);
	assert!(focused.chat.a() < 255);
	assert_eq!(focused.sidebar.a(), 255);
	ui::design::set_window_effects(true, 30, 60, true);
	assert!(
		ui::design::colors(true, ui::design::Variant::Standard)
			.sidebar
			.a() < 255
	);
	ui::design::set_window_effects(false, 15, 50, false);
	ui::design::apply(&ctx);
	assert_eq!(
		ctx.style_of(egui::Theme::Dark).spacing.button_padding,
		egui::vec2(12.0, 6.0)
	);
	println!("Theme API debug check passed: legacy package, both appearances, bounds and reset.");
}
