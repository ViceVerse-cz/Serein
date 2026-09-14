//! Synthetic egui workload, not native CPU, RSS, presentation time, or Discord interoperability.
//! cargo run --release --locked -p ui --example friends_idle
use client_core::{Envelope, Event, user_actions};
use model::Id;
use std::time::Instant;

fn main() {
	let mut state = test_support::friends_demo_state();
	let mut friends: Vec<_> = state
		.friends()
		.map(|user| {
			(
				user.clone(),
				state.friend_username(user.id).unwrap().to_owned(),
			)
		})
		.collect();
	let template = friends[0].0.clone();
	for index in friends.len()..user_actions::MAX_RELATIONSHIPS {
		let mut user = template.clone();
		user.id = Id(100_000 + index as u64);
		user.name = format!("Synthetic friend {index:04}");
		user.avatar = None;
		friends.push((user, format!("synthetic{index:04}")));
	}
	state.apply(Envelope {
		generation: state.generation,
		event: Event::UserAction(user_actions::Event::Friends(Some(friends))),
	});
	assert_eq!(state.friends().count(), 4000);
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut view = ui::MessagingUi::default();
	let mut frame = || {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 760.0),
				)),
				focused: true,
				..Default::default()
			},
			|ui| {
				assert!(
					view.show(ui, &mut state).is_empty(),
					"no actions during idle"
				);
			},
		);
		assert!(
			output.shapes.iter().any(|shape| online_count(&shape.shape)),
			"Friends Online fixture must be visible"
		);
		output.drop_without_applying_deltas();
	};
	for _ in 0..5 {
		frame();
	}
	let started = Instant::now();
	for _ in 0..200 {
		frame();
	}
	println!(
		"Synthetic Friends Online: 4000 friends, 7 online, 200 egui frames after 5 warmup frames: {:.3} ms. Excludes native event loop, tessellation and GPU presentation.",
		started.elapsed().as_secs_f64() * 1000.0
	);
}

fn online_count(shape: &egui::Shape) -> bool {
	match shape {
		egui::Shape::Text(text) => text.galley.job.text == "Online — 7",
		egui::Shape::Vec(shapes) => shapes.iter().any(online_count),
		_ => false,
	}
}
