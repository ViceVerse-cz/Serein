use ui::MessagingUi;

#[test]
fn session_clear_keeps_window_preference_but_removes_account_sharing_actions() {
	let mut view = MessagingUi::default();
	view.language = ui::i18n::Language::English;
	view.minimize_to_tray = true;
	view.tray_available = true;
	view.discord_activity_sharing = Some(false);
	view.discord_activity_sharing_request = Some(true);
	view.clear();
	assert!(view.minimize_to_tray && view.tray_available);
	assert_eq!(view.discord_activity_sharing, None);
	assert_eq!(view.discord_activity_sharing_request, None);
}

#[test]
fn account_sharing_requires_an_explicit_action_and_disables_it_while_pending() {
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
			// Synthetic renderer only: no desktop connection or network worker exists.
			state.demo = false;
			state.gateway_connected = true;
			let mut view = MessagingUi::default();
			view.language = ui::i18n::Language::English;
			view.share_game_activity = true;
			view.discord_activity_sharing = Some(false);
			view.preview_settings("activity");
			let mut frame = |view: &mut MessagingUi, events: Vec<egui::Event>| {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 760.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						view.show(ui, &mut state);
					},
				);
				let position = output.shapes.iter().find_map(|shape| match &shape.shape {
					egui::Shape::Text(text) if text.galley.job.text == "Enable on Discord" => {
						Some(text.pos + text.galley.size() / 2.0)
					}
					_ => None,
				});
				assert!(output.platform_output.commands.is_empty());
				output.drop_without_applying_deltas();
				position
			};
			let mut position = None;
			for _ in 0..3 {
				position = frame(&mut view, vec![]);
			}
			assert_eq!(view.discord_activity_sharing_request, None);
			let position = position.expect("Account sharing action is visible at both widths");
			for busy in [true, false] {
				view.discord_activity_sharing_busy = busy;
				frame(&mut view, vec![]);
				for pressed in [true, false] {
					frame(
						&mut view,
						vec![
							egui::Event::PointerMoved(position),
							egui::Event::PointerButton {
								pos: position,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: Default::default(),
							},
						],
					);
				}
				assert_eq!(
					view.discord_activity_sharing_request.take(),
					(!busy).then_some(true)
				);
			}
		}
	}
}
