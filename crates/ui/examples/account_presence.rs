//! Offline debug check: cargo run --locked -p ui --features demo --example account_presence
fn text(shape: &egui::Shape, output: &mut String) {
	match shape {
		egui::Shape::Text(shape) => {
			output.push_str(&shape.galley.job.text);
			output.push('\n');
		}
		egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text(shape, output)),
		_ => {}
	}
}

fn main() {
	let ctx = egui::Context::default();
	let mut state = test_support::demo_state();
	let mut view = ui::MessagingUi::default();
	view.reading_preferences.show_members = false;
	view.preview_account_menu(state.generation);
	let started_at = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.unwrap()
		.as_millis() as u64
		- 130_000;
	for details in [Some("First song"), Some("Next song"), None] {
		state.set_local_game_activity(details.map(|details| model::RichActivity {
			kind: 0,
			name: "Synthetic game".into(),
			details: Some(details.into()),
			state: Some("Solo".into()),
			image: Some(model::ActivityImage::Asset {
				application: model::Id(1),
				asset: model::Id(2),
			}),
			small_image: Some(model::ActivityImage::Application(model::Id(1))),
			ends_at: None,
			started_at: Some(started_at),
		}));
		let mut painted = String::new();
		for _ in 0..3 {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 760.0),
					)),
					..Default::default()
				},
				|ui| {
					view.show(ui, &mut state);
				},
			);
			painted.clear();
			for shape in &output.shapes {
				text(&shape.shape, &mut painted);
			}
			assert!(output.platform_output.commands.is_empty());
			output.drop_without_applying_deltas();
		}
		assert!(
			painted.contains("Set a custom status"),
			"account preview is open"
		);
		assert_eq!(painted.contains("Synthetic game"), details.is_some());
		assert_eq!(painted.contains("Playing"), details.is_some());
		if let Some(details) = details {
			assert!(painted.contains(details) && painted.contains("Solo"));
			assert!(
				painted.lines().any(|line| line.starts_with("2:")),
				"elapsed timer"
			);
		}
		assert!(view.take_avatar_requests().is_empty(), "offline artwork");
	}
	println!("Account preview Rich Presence updates and clears (synthetic debug check).");
}
