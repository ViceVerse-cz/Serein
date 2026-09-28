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
	view.language = ui::i18n::Language::English;
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
		let painted = render(&ctx, &mut view, &mut state);
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
	// Render a synthetic connected account without a host/transport or open profile menu.
	let mut view = ui::MessagingUi::default();
	state.demo = false;
	state.gateway_connected = true;
	for language in [ui::i18n::Language::English, ui::i18n::Language::Czech] {
		view.language = language;
		for (status, key) in [
			(model::PresenceStatus::Online, "status-online"),
			(model::PresenceStatus::Idle, "status-idle"),
			(model::PresenceStatus::DoNotDisturb, "status-dnd"),
			(
				model::PresenceStatus::Invisible,
				"account-menu-status-invisible",
			),
		] {
			view.own_presence.status = status;
			let painted = render(&ctx, &mut view, &mut state);
			assert!(
				painted.lines().any(|line| line == language.text(key)),
				"{painted}"
			);
		}
		for custom in [true, false] {
			view.own_presence.custom_status = if custom {
				"search".into()
			} else {
				String::new()
			};
			view.own_game = (!custom).then(|| "search".into());
			view.share_game_activity = true;
			let painted = render(&ctx, &mut view, &mut state);
			assert!(
				painted.lines().any(|line| line == "search"),
				"literal user content: {painted}"
			);
		}
		view.own_game = None;
	}
	println!("Account presence labels and literal status/game text pass (synthetic debug check).");
}

fn render(
	ctx: &egui::Context,
	view: &mut ui::MessagingUi,
	state: &mut client_core::State,
) -> String {
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
				view.show(ui, state);
			},
		);
		painted.clear();
		for shape in &output.shapes {
			text(&shape.shape, &mut painted);
		}
		assert!(output.platform_output.commands.is_empty());
		output.drop_without_applying_deltas();
	}
	painted
}
