use eframe::egui;
use model::Id;

fn labels(shape: &egui::Shape, output: &mut Vec<String>) {
	match shape {
		egui::Shape::Text(text) => output.push(text.galley.job.text.clone()),
		egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, output)),
		_ => {}
	}
}

pub fn check() {
	let ctx = egui::Context::default();
	ui::fonts::install(&ctx);
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	test_support::seed_access_marks(&mut state);
	let mut view = ui::MessagingUi::default();
	view.show_hidden_channels = true;
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(1120.0, 1600.0),
			)),
			focused: true,
			..Default::default()
		},
		|ui| {
			let _ = view.show(ui, &mut state);
		},
	);
	let mut text = Vec::new();
	for shape in &output.shapes {
		labels(&shape.shape, &mut text);
	}
	output.drop_without_applying_deltas();
	for name in [
		"staff-notes",
		"secret",
		"locked-hangout",
		"vault",
		"unknown-room",
		"long-form",
		"hangout",
	] {
		assert!(
			text.iter().any(|label| label == name),
			"missing {name}: {text:?}"
		);
	}
	assert!(
		!text.iter().any(|label| label.contains("hidden")),
		"visible name still says hidden: {text:?}"
	);
	let staff = state.channel_access(Id(61));
	assert!(!staff.hidden() && !staff.muted() && staff.limited());
	let secret = state.channel_access(Id(62));
	assert!(secret.hidden() && secret.limited());
	let locked = state.channel_access(Id(63));
	assert!(!locked.hidden() && locked.limited());
	let vault = state.channel_access(Id(64));
	assert!(vault.hidden() && !vault.limited());
	let unknown = state.channel_access(Id(65));
	assert!(unknown.hidden() && !unknown.limited());
	let muted_text = state.channel_access(Id(21));
	assert!(!muted_text.hidden() && muted_text.muted() && !muted_text.limited());
	let muted_voice = state.channel_access(Id(25));
	assert!(!muted_voice.hidden() && muted_voice.muted() && !muted_voice.limited());
	println!("Access marks paint names without a hidden suffix, and the fixture matrix holds.");
}
