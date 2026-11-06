use ui::MessagingUi;

#[test]
fn own_activity_panel_and_setting_render_and_clear() {
	for dark in [true, false] {
		for width in [760.0, 1120.0] {
			let ctx = egui::Context::default();
			ctx.set_theme(if dark {
				egui::ThemePreference::Dark
			} else {
				egui::ThemePreference::Light
			});
			ui::design::apply(&ctx);
			let mut state = test_support::demo_state();
			let mut view = MessagingUi::default();
			view.language = ui::i18n::Language::English;
			view.own_game = Some("Playing osu!".into());
			for settings in [false, true] {
				if settings {
					view.preview_settings("activity");
				}
				for enabled in [true, false] {
					view.share_game_activity = enabled;
					// Let the immediate-mode panel finish its sizing pass.
					for frame in 0..3 {
						let output = ctx.run_ui(
							egui::RawInput {
								screen_rect: Some(egui::Rect::from_min_size(
									egui::Pos2::ZERO,
									egui::vec2(width, 760.0),
								)),
								..Default::default()
							},
							|ui| {
								view.show(ui, &mut state);
							},
						);
						let text = output
							.shapes
							.iter()
							.filter_map(|shape| match &shape.shape {
								egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
								_ => None,
							})
							.collect::<Vec<_>>()
							.join("\n");
						output.drop_without_applying_deltas();
						if frame == 2 {
							assert_eq!(text.contains("Playing osu!"), enabled, "{text}");
							if settings {
								assert!(text.contains("Share game activity"), "{text}");
							}
						}
					}
				}
			}
		}
	}
}
