//! Offline debug check: cargo run --locked -p ui --example composer_scroll
fn main() {
	let mut state = test_support::empty_channel_demo_state(false);
	let channel = state.selected.unwrap();
	let draft = "composer overflow line\n".repeat(70);
	state.drafts.insert(channel, draft.clone());
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut view = ui::MessagingUi::default();
	let mut positions = Vec::new();
	for frame in 0..12 {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 760.0),
				)),
				events: if frame >= 6 {
					vec![
						egui::Event::PointerMoved(egui::pos2(450.0, 690.0)),
						egui::Event::MouseWheel {
							unit: egui::MouseWheelUnit::Point,
							phase: egui::TouchPhase::Move,
							delta: egui::vec2(0.0, -100.0),
							modifiers: egui::Modifiers::NONE,
						},
					]
				} else {
					vec![]
				},
				..Default::default()
			},
			|ui| {
				let _ = view.show(ui, &mut state);
			},
		);
		for shape in &output.shapes {
			if let egui::Shape::Text(text) = &shape.shape
				&& text.galley.job.text == draft
			{
				assert!(shape.clip_rect.height() <= 0.5 * 760.0 + 2.0);
				assert!(shape.clip_rect.top() > 300.0);
				positions.push(text.pos.y);
			}
		}
		output.drop_without_applying_deltas();
	}
	assert!(positions.len() >= 6, "draft must render");
	assert!(
		positions.last().unwrap() < &positions[5],
		"draft must scroll"
	);
	assert_eq!(state.drafts[&channel], draft);
	println!("Composer stays bounded, scrolls, and preserves the draft.");
}
