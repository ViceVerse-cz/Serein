//! Offline debug check: cargo run --locked -p ui --example thread_member_groups
use model::{Freshness, Id, Member, MemberSlot, permissions as p};

fn text(shape: &egui::Shape, out: &mut Vec<String>) {
	match shape {
		egui::Shape::Text(shape) => out.push(shape.galley.job.text.clone()),
		egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text(shape, out)),
		_ => {}
	}
}

fn main() {
	let mut state = test_support::demo_state();
	let channel = state.selected.unwrap();
	let guild = state.channel(channel).unwrap().guild.unwrap();
	let parent = state
		.channels
		.iter()
		.find(|entry| entry.guild == Some(guild) && entry.id != channel && entry.kind == 0)
		.unwrap()
		.id;
	let thread = state
		.channels
		.iter_mut()
		.find(|entry| entry.id == channel)
		.unwrap();
	thread.kind = 11;
	thread.parent_id = Some(parent);
	state.request_members();
	state.permissions.guilds.insert(
		guild,
		p::Guild {
			id: guild,
			owner: None,
			member: None,
			roles: Some(vec![p::Role {
				id: Id(999),
				name: "Thread moderators".into(),
				bits: 0,
				position: 10,
				color: 0,
				hoist: true,
			}]),
		},
	);
	let list = state.members.as_mut().unwrap();
	list.lazy = false;
	list.freshness = Freshness::Fresh;
	list.groups.clear();
	list.slots = (1..=3)
		.map(|id| {
			Some(MemberSlot::Person(Member {
				user: test_support::message(id, channel).author,
				nick: Some(format!("Participant {id}")),
				roles: if id == 2 { vec![] } else { vec![Id(999)] },
				status: Some(if id == 3 { "offline" } else { "online" }.into()),
				custom_status: None,
				activities: if id == 2 {
					vec![model::RichActivity {
						kind: 2,
						name: "Spotify".into(),
						details: Some("Synthetic track".into()),
						state: Some("Synthetic artist".into()),
						image: None,
						small_image: None,
						started_at: None,
						ends_at: None,
					}]
				} else {
					vec![]
				},
			}))
		})
		.collect();
	list.total = 3;
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut view = ui::MessagingUi::default();
	view.reading_preferences.show_members = true;
	let mut painted = Vec::new();
	let mut name_rect = egui::Rect::NOTHING;
	let mut subtitle_rect = egui::Rect::NOTHING;
	for _ in 0..3 {
		painted.clear();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1200.0, 900.0),
				)),
				..Default::default()
			},
			|ui| {
				view.show(ui, &mut state);
			},
		);
		for shape in &output.shapes {
			text(&shape.shape, &mut painted);
			if let egui::Shape::Text(text) = &shape.shape {
				let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
				match text.galley.job.text.as_str() {
					"Participant 2" => name_rect = rect,
					"Synthetic artist" => subtitle_rect = rect,
					_ => {}
				}
			}
		}
		output.drop_without_applying_deltas();
	}
	let headers: Vec<_> = painted
		.iter()
		.filter(|text| {
			matches!(
				text.as_str(),
				"Thread moderators - 1" | "Online - 1" | "Offline - 1"
			)
		})
		.map(String::as_str)
		.collect();
	assert_eq!(
		headers,
		["Thread moderators - 1", "Online - 1", "Offline - 1"]
	);
	for id in 1..=3 {
		assert!(painted.contains(&format!("Participant {id}")));
	}
	assert!(painted.contains(&"Synthetic artist".into()));
	assert!(name_rect.is_finite() && subtitle_rect.is_finite());
	assert!(subtitle_rect.top() >= name_rect.bottom());
	assert!(
		subtitle_rect.bottom() - name_rect.top() <= 34.0,
		"member text must fit beside the avatar: {name_rect:?} {subtitle_rect:?}"
	);
	assert!(!painted.contains(&"Listening to Spotify".into()));
	println!("Thread role groups, online fallback and offline members rendered correctly.");
}
