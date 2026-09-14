//! Offline debug check: cargo run --locked -p ui --example reaction_loading
use client_core::{Envelope, Event, State};
use model::Id;

fn text(shape: &egui::Shape, output: &mut String) {
	match shape {
		egui::Shape::Text(shape) => output.push_str(&shape.galley.job.text),
		egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text(shape, output)),
		_ => {}
	}
}

fn frame(state: &mut State) -> String {
	let ctx = egui::Context::default();
	let mut view = ui::MessagingUi::default();
	let mut painted = String::new();
	for _ in 0..3 {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1000.0, 800.0),
				)),
				..Default::default()
			},
			|ui| {
				let _ = view.show(ui, state);
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
		painted.contains("Synthetic reaction loading probe"),
		"the message must be visible"
	);
	painted
}

fn main() {
	let mut state = test_support::empty_channel_demo_state(false);
	let channel = state.selected.unwrap();
	let mut cached = test_support::message(600, channel);
	cached.content = "Synthetic reaction loading probe".into();
	cached.reactions = None; // SQLite does not persist reaction counts.
	state.timeline.insert(cached.clone(), false, false).unwrap();
	let _ = state.history(None);
	let loading = frame(&mut state);
	assert!(!loading.contains("Reactions unavailable") && !loading.contains("Reload reactions"));
	assert!(!loading.contains("Updating reactions"));
	state.apply(Envelope {
		generation: state.generation,
		event: Event::HistoryFailed {
			channel,
			request: state.request,
			failure: client_core::auth::Failure::Network,
		},
	});
	let failed = frame(&mut state);
	assert!(failed.contains("Reactions unavailable") && failed.contains("Reload reactions"));
	let _ = state.history(None);
	cached.reactions = Some(vec![]);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::History {
			channel,
			request: state.request,
			older: false,
			messages: vec![cached],
		},
	});
	assert!(!frame(&mut state).contains("Reactions unavailable"));
	state.refresh_reactions(Id(600));
	assert!(!frame(&mut state).contains("Reload reactions"));
	state.reactions.reset();
	state
		.timeline
		.set_reactions(
			Id(600),
			Some(vec![model::Reaction {
				emoji: model::ReactionEmoji {
					id: None,
					name: Some("👍".into()),
				},
				count: 3141,
				me: false,
				me_burst: false,
			}]),
		)
		.unwrap();
	let _ = state.history(None);
	assert!(
		frame(&mut state).contains("3141"),
		"known reactions stay visible while history reloads"
	);
	println!(
		"PASS: cached history loading stays quiet, failures retain retry, empty success and reaction refresh stay quiet, known counts remain visible (synthetic egui frames)."
	);
}
