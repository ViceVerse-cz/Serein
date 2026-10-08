//! Offline debug check: cargo run --locked -p ui --features demo --example profile_board
fn main() {
	// The full card keeps identity beside the board, and stacks it on narrow windows.
	for size in [egui::vec2(1120.0, 800.0), egui::vec2(600.0, 760.0)] {
		let mut state = test_support::demo_state();
		let mut view = ui::MessagingUi::default();
		let ctx = egui::Context::default();
		ui::design::apply(&ctx);
		view.preview_profile(test_support::message(1, model::Id(20)).author);
		for _ in 0..2 {
			ctx.run_ui(Default::default(), |ui| {
				view.show(ui, &mut state);
			})
			.drop_without_applying_deltas();
		}
		let mut compact = ctx.run_ui(Default::default(), |ui| {
			view.show(ui, &mut state);
		});
		compact.textures_delta.clear();
		let user = &state.profile.as_ref().unwrap().data.as_ref().unwrap().user;
		let input = ctx
			.read_response(egui::Id::unique((
				"profile-message-input",
				state.generation,
				user.id,
			)))
			.unwrap();
		let placeholder =
			ui::i18n::translate_args("profiles-message-placeholder", &[("user", &user.name)]);
		let text = compact
			.shapes
			.iter()
			.find_map(|shape| match &shape.shape {
				egui::Shape::Text(text) if text.galley.text() == placeholder => Some(text),
				_ => None,
			})
			.expect("profile message placeholder");
		assert!(
			(text.pos.y + text.galley.size().y / 2.0 - input.rect.center().y).abs() <= 1.0,
			"profile message text is vertically centered"
		);
		view.preview_full_profile(
			state
				.profile
				.as_ref()
				.unwrap()
				.data
				.as_ref()
				.unwrap()
				.user
				.clone(),
		);
		for _ in 0..4 {
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
					..Default::default()
				},
				|ui| {
					view.show(ui, &mut state);
				},
			);
			output.textures_delta.clear();
		}
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
				..Default::default()
			},
			|ui| {
				view.show(ui, &mut state);
			},
		);
		output.textures_delta.clear();
		let rect = ctx
			.memory(|memory| memory.area_rect(egui::Id::unique("user-profile-full")))
			.unwrap();
		assert!(
			rect.width() <= size.x && rect.height() <= size.y,
			"full profile fits {size:?}: {rect:?}"
		);
		let text = |label: &str| {
			output.shapes.iter().find_map(|shape| match &shape.shape {
				egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
				_ => None,
			})
		};
		if size.x > 700.0 {
			let board = text("Favorite Game").expect("favorite board card");
			let name = text("Synthetic favorite game").expect("game name");
			assert!(
				board.x > rect.left() + rect.width() * 0.45 && name.x > board.x,
				"board is in the right column: {rect:?} {board:?} {name:?}"
			);
		}
	}
	ui::debug_pr565(
		test_support::demo_state(),
		test_support::message(1, model::Id(20)).author,
	);
	println!("Full profile Board layout fits wide and narrow windows.");
}
