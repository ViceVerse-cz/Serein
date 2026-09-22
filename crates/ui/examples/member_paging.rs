//! Offline debug check: cargo run --locked -p ui --example member_paging
use client_core::{Envelope, Event, State};
use model::{Freshness, Member, MemberSlot};

fn reply(state: &mut State, start: usize, total: u64) {
	let mut list = state.members.clone().unwrap();
	list.start = start;
	list.total = total;
	list.freshness = Freshness::Fresh;
	list.slots = (start..(start + 100).min(total as usize))
		.map(|index| {
			Some(MemberSlot::Person(Member {
				user: test_support::message(index as u64 + 1, list.channel).author,
				nick: Some(format!("Synthetic member {index}")),
				roles: vec![],
				status: None,
				custom_status: None,
				activities: vec![],
			}))
		})
		.collect();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Members(list),
	});
}

fn frames(ctx: &egui::Context, view: &mut ui::MessagingUi, state: &mut State, wheel: f32) {
	for frame in 0..20 {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1200.0, 760.0),
				)),
				events: if frame == 0 && wheel != 0.0 {
					vec![
						egui::Event::PointerMoved(egui::pos2(1100.0, 400.0)),
						egui::Event::MouseWheel {
							unit: egui::MouseWheelUnit::Point,
							phase: egui::TouchPhase::Move,
							delta: egui::vec2(0.0, wheel),
							modifiers: egui::Modifiers::NONE,
						},
					]
				} else {
					vec![]
				},
				..Default::default()
			},
			|ui| {
				let _ = view.show(ui, state);
			},
		);
		output.drop_without_applying_deltas();
		assert!(view.take_avatar_requests().is_empty());
	}
}

fn main() {
	let ctx = egui::Context::default();
	let mut view = ui::MessagingUi::default();
	view.reading_preferences.show_members = true;
	view.reading_preferences.smooth_scrolling = false;
	let mut state = test_support::demo_state();
	state.gateway_connected = true;
	state.request_members().unwrap();
	reply(&mut state, 0, 250_000);
	frames(&ctx, &mut view, &mut state, 0.0);
	assert_eq!(state.members.as_ref().unwrap().ranges, [[0, 99]]);
	frames(&ctx, &mut view, &mut state, -500.0);
	assert_eq!(state.members.as_ref().unwrap().ranges, [[0, 99]]);
	frames(&ctx, &mut view, &mut state, -20_000.0);
	assert_eq!(
		state.members.as_ref().unwrap().ranges,
		[[0, 99], [100, 199]]
	);
	// Waiting at the bottom must neither cancel the request nor fetch further pages.
	frames(&ctx, &mut view, &mut state, 0.0);
	assert_eq!(
		state.members.as_ref().unwrap().ranges,
		[[0, 99], [100, 199]]
	);
	reply(&mut state, 100, 250_000);
	frames(&ctx, &mut view, &mut state, 0.0);
	frames(&ctx, &mut view, &mut state, -20_000.0);
	assert_eq!(
		state.members.as_ref().unwrap().ranges,
		[[100, 199], [200, 299]]
	);
	// A shorter final page stops pagination at the reported total.
	reply(&mut state, 200, 237);
	frames(&ctx, &mut view, &mut state, 0.0);
	frames(&ctx, &mut view, &mut state, -20_000.0);
	assert_eq!(state.members.as_ref().unwrap().ranges, [[200, 299]]);
	state.close_members();
	state.request_members().unwrap();
	reply(&mut state, 0, 250_000);
	frames(&ctx, &mut view, &mut state, 0.0);
	assert_eq!(state.members.as_ref().unwrap().ranges, [[0, 99]]);
	println!(
		"Member paging waits for bottom scroll and replies, stops at the total, and resets on reopen (offline)."
	);
}
