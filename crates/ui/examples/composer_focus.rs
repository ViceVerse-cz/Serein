//! Offline debug check: cargo run --locked -p ui --example composer_focus
use egui::{Event, Pos2};

fn frame(
	ctx: &egui::Context,
	view: &mut ui::MessagingUi,
	state: &mut client_core::State,
	events: Vec<Event>,
) -> egui::FullOutput {
	let mut output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				Pos2::ZERO,
				egui::vec2(1120.0, 800.0),
			)),
			focused: true,
			events,
			..Default::default()
		},
		|ui| {
			let _ = view.show(ui, state);
		},
	);
	output.textures_delta.clear();
	output
}

fn main() {
	let ctx = egui::Context::default();
	ctx.enable_accesskit();
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	let channel = state.selected.unwrap();
	let draft = "A saved draft 🙂\nLast line";
	state.drafts.insert(channel, draft.into());
	let mut view = ui::MessagingUi::default();
	for _ in 0..3 {
		frame(&ctx, &mut view, &mut state, vec![]).drop_without_applying_deltas();
	}
	let editor = ctx
		.memory(|memory| memory.focused())
		.expect("composer focus");
	assert_eq!(
		egui::text_edit::TextEditState::load(&ctx, editor)
			.unwrap()
			.cursor
			.char_range()
			.unwrap()
			.primary
			.index
			.0,
		draft.chars().count()
	);
	// Simulate returning from another channel with the old shared caret at the start.
	state.selected = None;
	frame(&ctx, &mut view, &mut state, vec![]).drop_without_applying_deltas();
	let mut edit = egui::text_edit::TextEditState::load(&ctx, editor).unwrap();
	edit.cursor
		.set_char_range(Some(egui::text::CCursorRange::one(
			egui::text::CCursor::new(0),
		)));
	edit.store(&ctx, editor);
	state.selected = Some(channel);
	frame(&ctx, &mut view, &mut state, vec![]).drop_without_applying_deltas();
	let output = frame(&ctx, &mut view, &mut state, vec![Event::Text("!".into())]);
	assert_eq!(state.drafts[&channel], format!("{draft}!"));
	let button = output
		.platform_output
		.accesskit_update
		.as_ref()
		.unwrap()
		.nodes
		.iter()
		.find_map(|(_, node)| {
			(node.label()
				== Some(ui::i18n::translate("emoji-picker-show-insert-an-emoji").as_str()))
			.then(|| node.bounds())
			.flatten()
		})
		.expect("emoji button");
	let pos = egui::pos2(
		((button.x0 + button.x1) / 2.0) as f32,
		((button.y0 + button.y1) / 2.0) as f32,
	);
	output.drop_without_applying_deltas();
	for pressed in [true, false] {
		frame(
			&ctx,
			&mut view,
			&mut state,
			vec![
				Event::PointerMoved(pos),
				Event::PointerButton {
					pos,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			],
		)
		.drop_without_applying_deltas();
	}
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![Event::Text("smile".into())],
	)
	.drop_without_applying_deltas();
	let output = frame(&ctx, &mut view, &mut state, vec![]);
	assert_eq!(
		state.drafts[&channel],
		format!("{draft}!"),
		"search must not type into the draft"
	);
	let update = output.platform_output.accesskit_update.as_ref().unwrap();
	let (id, search) = update
		.nodes
		.iter()
		.find(|(_, node)| {
			node.label() == Some(ui::i18n::translate("emoji-picker-search-emoji-label").as_str())
		})
		.expect("search field");
	assert_eq!(*id, update.focus, "picker search receives focus");
	assert_eq!(
		search.value(),
		Some("smile"),
		"first search characters are retained"
	);
	let bounds = search.bounds().unwrap();
	let text = output
		.shapes
		.iter()
		.find_map(|shape| match &shape.shape {
			egui::Shape::Text(text) if text.galley.text() == "smile" => Some(text),
			_ => None,
		})
		.expect("rendered query");
	assert!(
		(text.pos.y + text.galley.size().y / 2.0 - ((bounds.y0 + bounds.y1) / 2.0) as f32).abs()
			<= 1.0,
		"query is vertically centered"
	);
	output.drop_without_applying_deltas();
	println!(
		"Draft return appends after Unicode text; picker click focuses search and centers its text."
	);
}
