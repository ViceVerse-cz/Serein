//! Offline debug check: cargo run --locked -p ui --example member_paging
use client_core::{Envelope, Event, State};
use model::{Freshness, Member, MemberSlot};

fn person(list_channel: model::Id, index: u64) -> MemberSlot {
	MemberSlot::Person(Member {
		user: test_support::message(index + 1, list_channel).author,
		nick: Some(format!("Synthetic member {index}")),
		roles: vec![],
		status: None,
		custom_status: None,
		activities: vec![],
	})
}

fn reply(state: &mut State, start: usize, total: u64) {
	let mut list = state.members.clone().unwrap();
	list.start = start;
	list.total = total;
	list.freshness = Freshness::Fresh;
	list.groups.clear();
	list.slots = (start..(start + 100).min(total as usize))
		.map(|index| Some(person(list.channel, index as u64)))
		.collect();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Members(list),
	});
}

fn groups(state: &mut State, total: u64, groups: Vec<(String, u64)>) {
	let mut list = state.members.clone().unwrap();
	list.start = 0;
	list.total = total;
	list.freshness = Freshness::Fresh;
	list.groups = groups;
	list.slots = vec![Some(person(list.channel, 0))];
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

fn ranges(state: &State) -> Vec<[usize; 2]> {
	state.members.as_ref().unwrap().ranges.clone()
}

fn main() {
	let ctx = egui::Context::default();
	let mut view = ui::MessagingUi::default();
	view.reading_preferences.show_members = true;
	view.reading_preferences.smooth_scrolling = false;
	let mut state = test_support::demo_state();
	state.gateway_connected = true;
	state.request_members().unwrap();
	let first = state.members.as_ref().unwrap();
	assert!(
		first.slots.iter().all(|slot| slot.is_none()),
		"a server opened for the first time has no cached people"
	);
	assert_eq!(first.freshness, Freshness::Loading);
	assert_eq!(state.member_scroll_rows(), 0);

	reply(&mut state, 0, 250_000);
	assert_eq!(
		state.member_scroll_rows(),
		250_000,
		"without groups the scrollbar follows the reported total"
	);
	frames(&ctx, &mut view, &mut state, 0.0);
	assert_eq!(ranges(&state), [[0, 99]]);
	frames(&ctx, &mut view, &mut state, -500.0);
	assert_eq!(
		ranges(&state),
		[[0, 99]],
		"a short wheel stays inside the open chunk"
	);
	frames(&ctx, &mut view, &mut state, -20_000.0);
	assert_eq!(
		ranges(&state),
		[[400, 499], [500, 599]],
		"a 20_000px wheel requests the visible chunks"
	);

	groups(
		&mut state,
		500,
		vec![("online".into(), 40), ("offline".into(), 460)],
	);
	assert_eq!(
		state.member_scroll_rows(),
		502,
		"under 1,000 the scrollbar includes offline people and both headers"
	);
	groups(
		&mut state,
		2_000,
		vec![("online".into(), 30), ("offline".into(), 1_970)],
	);
	assert_eq!(
		state.member_scroll_rows(),
		32,
		"at 1,000+ only online people, their header, and the offline header remain"
	);
	let origin = state.selected.unwrap();
	let origin_guild = state.members.as_ref().unwrap().guild;
	let other = state
		.channels
		.iter()
		.find(|channel| channel.guild == origin_guild && channel.id != origin && channel.kind == 0)
		.map(|channel| channel.id)
		.expect("demo has another channel in the same server");
	state.select(other).expect("other channel opens");
	state.request_members().expect("other channel lists people");
	assert_eq!(
		state.member_scroll_rows(),
		32,
		"switching channels keeps the offline list hidden"
	);
	state.select(origin).expect("return to the first channel");
	state.request_members().expect("first channel lists people");
	groups(
		&mut state,
		800,
		vec![("online".into(), 30), ("offline".into(), 770)],
	);
	assert_eq!(
		state.member_scroll_rows(),
		32,
		"once hidden, offline rows stay hidden at 800 members"
	);
	groups(
		&mut state,
		799,
		vec![("online".into(), 30), ("offline".into(), 769)],
	);
	assert_eq!(
		state.member_scroll_rows(),
		801,
		"offline rows return under 800 members"
	);

	let cached = state.members.as_ref().unwrap().total;
	state.close_members();
	state.request_members().unwrap();
	let restored = state.members.as_ref().unwrap();
	assert!(
		restored.slots.iter().any(|slot| slot.is_some()),
		"reopening paints the cached top page"
	);
	assert_eq!(restored.total, cached);
	assert_eq!(
		restored.groups,
		vec![("online".to_owned(), 30), ("offline".to_owned(), 769)]
	);
	assert_eq!(restored.freshness, Freshness::Fresh);
	assert_eq!(state.member_scroll_rows(), 801);

	reply(&mut state, 0, 250_000);
	frames(&ctx, &mut view, &mut state, 0.0);
	assert_eq!(
		ranges(&state),
		[[0, 99]],
		"a new member request resets the pane to the top"
	);
	state.freshness = Freshness::Unavailable;
	state.request_members().unwrap();
	assert_eq!(
		state.members.as_ref().unwrap().freshness,
		Freshness::Unavailable,
		"a cached page stays unavailable with the session"
	);
	println!(
		"Member paging follows the viewport, sizes the scrollbar to Discord's online cap, and reopens on the cached top page (offline)."
	);
}
