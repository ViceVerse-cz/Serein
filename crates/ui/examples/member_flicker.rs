//! Offline debug check: cargo run --locked -p ui --example member_flicker
use client_core::{Envelope, Event, State};
use model::{Freshness, Id, Member};

fn text(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
	match shape {
		egui::Shape::Text(shape) => {
			out.push((shape.galley.job.text.clone(), shape.pos));
		}
		egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text(shape, out)),
		_ => {}
	}
}

fn frame(
	ctx: &egui::Context,
	view: &mut ui::MessagingUi,
	state: &mut State,
) -> Vec<(String, egui::Pos2)> {
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(1200.0, 760.0),
			)),
			..Default::default()
		},
		|ui| {
			view.show(ui, state);
		},
	);
	let mut painted = Vec::new();
	for shape in &output.shapes {
		if shape.clip_rect.is_positive() {
			text(&shape.shape, &mut painted);
		}
	}
	if state
		.members
		.as_ref()
		.is_some_and(|list| list.freshness == Freshness::Loading)
		&& state.members_cached()
	{
		assert!(
			painted
				.iter()
				.any(|(text, _)| text.contains("Synthetic game")),
			"cached activity must remain visible during member refresh"
		);
	}
	painted.retain(|(text, _)| text.contains("Stable message"));
	output.drop_without_applying_deltas();
	painted
}

fn main() {
	for count in [3, 100] {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		state.gateway_connected = true;
		state.timeline.clear();
		for id in 1..=count {
			let mut message = test_support::message(id, channel);
			message.content = format!(
				"Stable message {id} [link](https://example.com/{})",
				"a".repeat(3500)
			);
			message.attachments.clear();
			message.embeds.clear();
			state.timeline.insert(message, false, false).unwrap();
		}
		state.request_members();
		let mut list = state.members.clone().unwrap();
		list.freshness = Freshness::Fresh;
		list.slots = (1..=8)
			.map(|id| {
				Some(model::MemberSlot::Person(Member {
					user: model::User {
						id: Id(id),
						..test_support::message(id, channel).author
					},
					nick: Some(format!("Member {id}")),
					roles: vec![],
					status: Some("online".into()),
					custom_status: None,
					activities: vec![],
				}))
			})
			.collect();
		list.total = list.slots.len() as u64;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Members(list.clone()),
		});
		if count > 3 {
			state.remember_reading(
				channel,
				client_core::ReadingCursor {
					message: Some(Id(count / 2)),
					inset: 5.0,
				},
			);
		}
		let mut view = ui::MessagingUi::default();
		view.reading_preferences.show_members = true;
		for _ in 0..12 {
			frame(&ctx, &mut view, &mut state);
		}
		let settled = frame(&ctx, &mut view, &mut state);
		assert!(!settled.is_empty());
		assert!(state.members_cached());
		let revision = state.revision;
		for status in [
			Some("idle"),
			Some("dnd"),
			Some("offline"),
			None,
			Some("online"),
		] {
			let online = matches!(status, Some("online" | "idle" | "dnd"));
			let update = model::MemberPresence {
				user: Id(1),
				status: status.map(str::to_owned),
				custom_status: online.then(|| "Synthetic status".into()),
				activities: if online {
					vec![model::RichActivity {
						kind: 0,
						name: "Synthetic game".into(),
						details: None,
						state: None,
						image: None,
						small_image: None,
						ends_at: None,
						started_at: None,
					}]
				} else {
					vec![]
				},
			};
			state.apply(Envelope {
				generation: state.generation,
				event: Event::MemberPresence {
					guild: list.guild.unwrap(),
					channel,
					request: list.request,
					updates: vec![update.clone()],
				},
			});
			let model::MemberSlot::Person(cached) = state.member_slot(0).unwrap() else {
				panic!("expected cached person");
			};
			assert_eq!(cached.status, update.status);
			assert_eq!(cached.custom_status, update.custom_status);
			assert_eq!(cached.activities, update.activities);
			assert_eq!(state.revision, revision);
			assert_eq!(frame(&ctx, &mut view, &mut state), settled);
		}
		// Returning to a cached page must retain the latest presence while SYNC is pending.
		let mut pending = state.members.clone().unwrap();
		pending.start = 100;
		pending.slots = vec![None; 100];
		pending.freshness = Freshness::Loading;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Members(pending),
		});
		let model::MemberSlot::Person(cached) = state.member_slot(0).unwrap() else {
			panic!("expected cached person");
		};
		assert_eq!(cached.status.as_deref(), Some("online"));
		assert_eq!(cached.custom_status.as_deref(), Some("Synthetic status"));
		assert_eq!(cached.activities[0].name, "Synthetic game");
		frame(&ctx, &mut view, &mut state);
		for step in 0..3 {
			match step {
				0 => match list.slots[0].as_mut().unwrap() {
					model::MemberSlot::Person(member) => member.status = Some("offline".into()),
					model::MemberSlot::Group(_) => unreachable!(),
				},
				1 => {
					list.slots.remove(0);
					list.total -= 1;
				}
				_ => list.slots.reverse(),
			}
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Members(list.clone()),
			});
			for pass in 0..3 {
				assert_eq!(
					frame(&ctx, &mut view, &mut state),
					settled,
					"count={count}, step={step}, pass={pass}"
				);
			}
		}
		let id = settled[0]
			.0
			.split_whitespace()
			.nth(2)
			.unwrap()
			.parse::<u64>()
			.unwrap();
		let mut edited = state.timeline.get(Id(id)).unwrap().clone();
		edited.content = "Stable message edited\nA second line".into();
		state.timeline.insert(edited, false, false).unwrap();
		state.revision += 1;
		for _ in 0..3 {
			frame(&ctx, &mut view, &mut state);
		}
		assert!(
			frame(&ctx, &mut view, &mut state)
				.iter()
				.any(|(text, _)| text.contains("edited"))
		);
	}
	println!(
		"Cached status/activity stays current and clears; member updates keep chat stable (offline)."
	);
}
