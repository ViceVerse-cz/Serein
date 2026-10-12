//! Offline debug check: cargo run --locked -p serein --features demo --example double_click_settings
fn main() {
	local_store::LocalStore::debug_double_click_reaction_check();
	ui::MessagingUi::debug_double_click_reaction_check(
		test_support::demo_state(),
		test_support::message(60_000 << 22, model::Id(20)),
	);
	for width in [360.0, 720.0] {
		let ctx = eframe::egui::Context::default();
		ui::design::apply(&ctx);
		let mut view = ui::MessagingUi::default();
		let mut state = test_support::demo_state();
		for enabled in [false, true] {
			view.reading_preferences.double_click_reaction_enabled = enabled;
			for _ in 0..2 {
				ctx.run_ui(
					eframe::egui::RawInput {
						screen_rect: Some(eframe::egui::Rect::from_min_size(
							eframe::egui::Pos2::ZERO,
							eframe::egui::vec2(width, 1000.0),
						)),
						..Default::default()
					},
					|ui| {
						view.chat_reading_settings(ui, &mut state);
						assert!(
							ui.min_rect().right() <= width,
							"settings fit narrow and wide windows"
						);
					},
				)
				.drop_without_applying_deltas();
			}
		}
	}
	println!("Double-click settings fit narrow/wide layouts enabled and disabled.");
}
