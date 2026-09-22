use ui::MessagingUi;

#[test]
fn session_clear_preserves_device_startup_preferences_and_pending_status() {
	let mut view = MessagingUi::default();
	view.startup_available = true;
	view.startup_enabled = true;
	view.startup_minimized = true;
	view.startup_busy = true;
	view.startup_status = "Synthetic pending startup write";
	view.clear();
	assert!(view.startup_available && view.startup_enabled && view.startup_minimized);
	assert!(view.startup_busy);
	assert_eq!(view.startup_status, "Synthetic pending startup write");
}

#[test]
fn startup_switches_require_availability_and_dependency_and_disable_while_pending() {
	const LABELS: [&str; 2] = ["Open Serein when your computer starts", "Start minimized"];
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
			view.startup_available = true;
			view.preview_settings("general");
			// Only the renderer exists: clicks cannot register OS startup or contact Discord.
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
				let positions = LABELS.map(|label| {
					output.shapes.iter().find_map(|shape| match &shape.shape {
						egui::Shape::Text(text) if text.galley.job.text == label => {
							Some(text.pos + text.galley.size() / 2.0)
						}
						_ => None,
					})
				});
				assert!(output.platform_output.commands.is_empty());
				output.drop_without_applying_deltas();
				positions
			};
			for _ in 0..3 {
				frame(&mut view, vec![]);
			}
			assert!(!view.startup_enabled && !view.startup_minimized);
			let mut click = |view: &mut MessagingUi, row: usize| {
				let position = frame(view, vec![])[row]
					.expect("Startup switches remain visible at both widths and themes");
				for pressed in [true, false] {
					frame(
						view,
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
			};
			click(&mut view, 1);
			assert!(
				!view.startup_minimized,
				"Minimized requires automatic startup"
			);
			view.startup_available = false;
			click(&mut view, 0);
			assert!(
				!view.startup_enabled,
				"Unavailable platforms cannot enable startup"
			);
			view.startup_available = true;
			view.startup_busy = true;
			click(&mut view, 0);
			assert!(!view.startup_enabled, "Loading disables startup edits");
			view.startup_busy = false;
			click(&mut view, 0);
			assert!(view.startup_enabled);
			click(&mut view, 1);
			assert!(view.startup_minimized);
			view.startup_busy = true;
			click(&mut view, 0);
			click(&mut view, 1);
			assert!(
				view.startup_enabled && view.startup_minimized,
				"Saving disables both switches"
			);
			view.startup_busy = false;
			click(&mut view, 0);
			assert!(!view.startup_enabled);
			click(&mut view, 1);
			assert!(
				view.startup_minimized,
				"Disabled dependent switch cannot change"
			);
		}
	}
}
