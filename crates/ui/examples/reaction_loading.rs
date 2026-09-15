//! Offline debug check: cargo run --locked -p ui --example reaction_loading
use client_core::{Envelope, Event, State, reactions};
use model::{Id, ReactionEmoji};

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

fn known_counts(state: &mut State, expected: &[(&str, u32)]) {
	let painted = frame(state);
	for unavailable in [
		"Reactions unavailable",
		"Reload reactions",
		"Updating reactions",
	] {
		assert!(
			!painted.contains(unavailable),
			"known counts must stay visible"
		);
	}
	let values = state
		.timeline
		.get(Id(600))
		.unwrap()
		.reactions
		.as_ref()
		.unwrap();
	assert_eq!(values.len(), expected.len());
	for (emoji, count) in expected {
		assert!(
			values
				.iter()
				.any(|reaction| reaction.emoji.name.as_deref() == Some(*emoji)
					&& reaction.count == *count)
		);
		// This context deliberately uses the text fallback, so each pill has an exact label.
		let label = format!("{emoji} {count}");
		assert!(
			painted.lines().any(|line| line == label),
			"missing rendered pill: {label}"
		);
	}
	assert_eq!(
		painted
			.lines()
			.filter(|line| line.starts_with("👍 ") || line.starts_with("🎉 "))
			.count(),
		expected.len(),
		"removed reaction pills must disappear"
	);
}

fn reaction_event(state: &mut State, event: reactions::Event) {
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Reactions(event),
	});
}

fn read_request(state: &mut State) -> u64 {
	let Some(client_core::Command::Reactions(reactions::Command::Read {
		channel,
		message,
		request,
	})) = state.next_reaction_read()
	else {
		panic!("live deltas retain their coalesced verification read");
	};
	assert_eq!(Some(channel), state.selected);
	assert_eq!(message, Id(600));
	assert!(
		state.next_reaction_read().is_none(),
		"only one verification may be in flight"
	);
	request
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
			Some(vec![
				model::Reaction {
					emoji: model::ReactionEmoji {
						id: None,
						name: Some("👍".into()),
					},
					count: 3141,
					me: false,
					me_burst: false,
				},
				model::Reaction {
					emoji: ReactionEmoji {
						id: None,
						name: Some("🎉".into()),
					},
					count: 2718,
					me: false,
					me_burst: false,
				},
			]),
		)
		.unwrap();
	let thumbs = ReactionEmoji {
		id: None,
		name: Some("👍".into()),
	};
	let delta = |emoji: ReactionEmoji, add, burst| reactions::Event::Delta {
		channel,
		message: Id(600),
		user: Id(9007),
		emoji,
		add,
		burst,
	};
	known_counts(&mut state, &[("👍", 3141), ("🎉", 2718)]);
	reaction_event(&mut state, delta(thumbs.clone(), true, false));
	known_counts(&mut state, &[("👍", 3142), ("🎉", 2718)]);
	let mut stale_history = state.timeline.get(Id(600)).unwrap().clone();
	stale_history.content = "Synthetic reaction loading probe (history body refreshed)".into();
	let _ = state.history(None);
	known_counts(&mut state, &[("👍", 3142), ("🎉", 2718)]);
	reaction_event(&mut state, delta(thumbs.clone(), true, true));
	known_counts(&mut state, &[("👍", 3143), ("🎉", 2718)]);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::History {
			channel,
			request: state.request,
			older: false,
			messages: vec![stale_history],
		},
	});
	assert!(!state.history_pending);
	assert!(
		state
			.timeline
			.get(Id(600))
			.unwrap()
			.content
			.ends_with("(history body refreshed)")
	);
	known_counts(&mut state, &[("👍", 3143), ("🎉", 2718)]);
	let stale_reactions = state
		.timeline
		.get(Id(600))
		.unwrap()
		.reactions
		.clone()
		.unwrap();
	let stale_request = read_request(&mut state);
	known_counts(&mut state, &[("👍", 3143), ("🎉", 2718)]);
	reaction_event(&mut state, delta(thumbs.clone(), false, false));
	known_counts(&mut state, &[("👍", 3142), ("🎉", 2718)]);
	reaction_event(
		&mut state,
		reactions::Event::Read {
			channel,
			message: Id(600),
			request: stale_request,
			result: Ok(stale_reactions),
		},
	);
	known_counts(&mut state, &[("👍", 3142), ("🎉", 2718)]);
	let current_request = read_request(&mut state);
	assert_ne!(current_request, stale_request);
	let current_reactions = state
		.timeline
		.get(Id(600))
		.unwrap()
		.reactions
		.clone()
		.unwrap();
	reaction_event(
		&mut state,
		reactions::Event::Read {
			channel,
			message: Id(600),
			request: current_request,
			result: Ok(current_reactions),
		},
	);
	known_counts(&mut state, &[("👍", 3142), ("🎉", 2718)]);
	assert!(state.next_reaction_read().is_none());
	reaction_event(
		&mut state,
		reactions::Event::Cleared {
			channel,
			message: Id(600),
			emoji: Some(thumbs.clone()),
		},
	);
	known_counts(&mut state, &[("🎉", 2718)]);
	reaction_event(
		&mut state,
		reactions::Event::Cleared {
			channel,
			message: Id(600),
			emoji: None,
		},
	);
	known_counts(&mut state, &[]);
	reaction_event(&mut state, delta(thumbs.clone(), true, false));
	known_counts(&mut state, &[("👍", 1)]);
	reaction_event(&mut state, delta(thumbs, false, false));
	known_counts(&mut state, &[]);
	let request = read_request(&mut state);
	known_counts(&mut state, &[]);
	reaction_event(
		&mut state,
		reactions::Event::Read {
			channel,
			message: Id(600),
			request,
			result: Ok(vec![]),
		},
	);
	known_counts(&mut state, &[]);
	assert!(state.next_reaction_read().is_none());
	println!(
		"PASS: loading/failure/retry, live add/remove/burst counts, independent emoji, stale history/HTTP reads and last-pill/clear events retain visible counts during coalesced verification (synthetic egui frames)."
	);
}
