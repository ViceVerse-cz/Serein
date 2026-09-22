//! Offline check that chat rows keep their places on the first presented frame.
use client_core::{Envelope, Event, State};
use model::{AccountKind, Freshness, Id, Reaction, ReactionEmoji};

fn text(shape: &egui::Shape, out: &mut Vec<(String, f32)>) {
	match shape {
		egui::Shape::Text(shape) if shape.galley.job.text.contains("Row ") => {
			out.push((shape.galley.job.text.clone(), shape.pos.y));
		}
		egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text(shape, out)),
		_ => {}
	}
}

fn frame(
	ctx: &egui::Context,
	view: &mut ui::MessagingUi,
	state: &mut State,
) -> (usize, Vec<(String, f32)>) {
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(1100.0, 720.0),
			)),
			focused: true,
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
	let passes = output.platform_output.num_completed_passes;
	output.drop_without_applying_deltas();
	(passes, painted)
}

fn worst(before: &[(String, f32)], after: &[(String, f32)]) -> f32 {
	if before.is_empty() || before.len() != after.len() {
		return f32::INFINITY;
	}
	before
		.iter()
		.map(|(label, y)| {
			after
				.iter()
				.find(|(next, _)| next == label)
				.map(|(_, next)| (y - next).abs())
				.unwrap_or(f32::INFINITY)
		})
		.fold(0.0, f32::max)
}

fn thumb(count: u32) -> Reaction {
	Reaction {
		emoji: ReactionEmoji {
			id: None,
			name: Some("👍".into()),
		},
		count,
		me: false,
		me_burst: false,
	}
}

fn posted(id: u64, channel: Id, reactions: Option<Vec<Reaction>>) -> model::Message {
	let mut message = test_support::message(id, channel);
	message.author.id = Id(9);
	message.author.name = "Deploy".into();
	message.author.webhook = true;
	message.author.kind = AccountKind::Bot;
	message.content = format!("Row {id} https://example.com/build/{id}");
	message.attachments.clear();
	message.embeds = vec![model::Embed {
		kind: "rich".into(),
		title: Some("Build status".into()),
		description: Some("The job finished.".into()),
		url: Some(format!("https://example.com/build/{id}")),
		color: Some(0x5865F2),
		..Default::default()
	}];
	message.reactions = reactions;
	message
}

fn fill(
	state: &mut State,
	channel: Id,
	reactions: Option<Vec<Reaction>>,
	kind: AccountKind,
	webhook: bool,
) {
	state.timeline.clear();
	state.selected = Some(channel);
	state.freshness = Freshness::Fresh;
	state.gateway_connected = true;
	state.older_exhausted = true;
	state.history_pending = reactions.is_none();
	for id in 1..=16 {
		let mut message = posted(1000 + id, channel, reactions.clone());
		message.author.webhook = webhook;
		message.author.kind = kind;
		if !webhook {
			message.author.name = "Helper".into();
		}
		state.timeline.insert(message, false, false).unwrap();
	}
	state.revision += 1;
}

fn main() {
	let channel = Id(21);
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	let mut view = ui::MessagingUi::default();
	view.reading_preferences.show_members = false;
	view.reading_preferences.smooth_scrolling = true;

	fill(
		&mut state,
		channel,
		Some(vec![thumb(3)]),
		AccountKind::Bot,
		true,
	);
	let (passes, first) = frame(&ctx, &mut view, &mut state);
	let mut open_worst = 0.0_f32;
	for _ in 0..4 {
		let (_, next) = frame(&ctx, &mut view, &mut state);
		open_worst = open_worst.max(worst(&first, &next));
	}
	println!(
		"open webhook passes={passes} rows={} worst={open_worst}",
		first.len()
	);
	assert!(open_worst <= 1.0, "open webhook moved {open_worst}");

	fill(&mut state, channel, None, AccountKind::Bot, true);
	let mut view = ui::MessagingUi::default();
	view.reading_preferences.show_members = false;
	view.reading_preferences.smooth_scrolling = true;
	for _ in 0..3 {
		frame(&ctx, &mut view, &mut state);
	}
	for id in 1..=16 {
		let message = posted(1000 + id, channel, Some(vec![thumb(3)]));
		state.timeline.insert(message, false, false).unwrap();
	}
	state.history_pending = false;
	state.revision += 1;
	let mut arrived = Vec::new();
	for _ in 0..4 {
		let (passes, painted) = frame(&ctx, &mut view, &mut state);
		let ys: Vec<_> = painted.iter().map(|(_, y)| *y).collect();
		println!("reactions passes={passes} rows={} y={ys:?}", painted.len());
		arrived.push(painted);
	}
	let settled = arrived.last().unwrap();
	let reaction_worst = arrived
		.iter()
		.map(|painted| worst(painted, settled))
		.fold(0.0, f32::max);
	println!("reactions worst={reaction_worst}");
	assert!(
		reaction_worst <= 1.0,
		"reaction arrival moved {reaction_worst}"
	);

	fill(
		&mut state,
		channel,
		Some(vec![thumb(2)]),
		AccountKind::Bot,
		true,
	);
	for id in 1..=16 {
		let mut message = posted(1000 + id, channel, Some(vec![thumb(2)]));
		message.embeds.clear();
		state.timeline.insert(message, false, false).unwrap();
	}
	state.revision += 1;
	let mut view = ui::MessagingUi::default();
	view.reading_preferences.show_members = false;
	view.reading_preferences.smooth_scrolling = true;
	for _ in 0..3 {
		frame(&ctx, &mut view, &mut state);
	}
	for id in 1..=16 {
		let message = posted(1000 + id, channel, Some(vec![thumb(2)]));
		state.timeline.insert(message, false, false).unwrap();
	}
	state.revision += 1;
	let mut embeds = Vec::new();
	for _ in 0..3 {
		let (passes, painted) = frame(&ctx, &mut view, &mut state);
		let ys: Vec<_> = painted.iter().map(|(_, y)| *y).collect();
		println!("embeds passes={passes} rows={} y={ys:?}", painted.len());
		embeds.push(painted);
	}
	let embed_settled = embeds.last().unwrap();
	let embed_worst = embeds
		.iter()
		.map(|painted| worst(painted, embed_settled))
		.fold(0.0, f32::max);
	println!("embeds worst={embed_worst}");
	assert!(embed_worst <= 1.0, "embed arrival moved {embed_worst}");

	fill(
		&mut state,
		channel,
		Some(vec![thumb(2)]),
		AccountKind::Bot,
		false,
	);
	let mut view = ui::MessagingUi::default();
	view.reading_preferences.show_members = false;
	view.reading_preferences.smooth_scrolling = true;
	for _ in 0..3 {
		frame(&ctx, &mut view, &mut state);
	}
	let (_, before_roles) = frame(&ctx, &mut view, &mut state);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Permissions(client_core::permissions::Event::Role {
			guild: Id(10),
			role: model::permissions::Role {
				id: Id(50),
				name: "Build alerts".into(),
				color: 0xE91E63,
				position: 1,
				hoist: false,
				bits: 0,
			},
		}),
	});
	for id in 1..=16 {
		let mut message = posted(1000 + id, channel, Some(vec![thumb(2)]));
		message.author.webhook = false;
		message.author.kind = AccountKind::Bot;
		message.author.name = "Helper".into();
		message.author_nick = Some("Build bot".into());
		message.author_roles = vec![Id(50)];
		state.timeline.insert(message, false, false).unwrap();
	}
	if let Some(channel) = state.channels.iter_mut().find(|item| item.id == Id(20)) {
		channel.name = "renamed-room".into();
	}
	state.revision += 1;
	let mut role_frames = Vec::new();
	for _ in 0..3 {
		let (passes, painted) = frame(&ctx, &mut view, &mut state);
		let ys: Vec<_> = painted.iter().map(|(_, y)| *y).collect();
		println!("roles passes={passes} rows={} y={ys:?}", painted.len());
		role_frames.push(painted);
	}
	let role_worst = role_frames
		.iter()
		.map(|painted| worst(&before_roles, painted))
		.fold(0.0, f32::max);
	println!("roles rows={} worst={role_worst}", role_frames[0].len());
	assert!(
		role_frames[0].len() == before_roles.len(),
		"role update cleared the timeline"
	);
	assert!(
		role_worst <= 1.0,
		"role and nick arrival moved {role_worst}"
	);
}
