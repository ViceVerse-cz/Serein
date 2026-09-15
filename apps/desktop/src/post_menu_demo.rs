//! Small offline debug run for the forum card -> menu -> command path.
use client_core::{Command, Envelope};
use eframe::egui;
use model::Id;

fn labels(shape: &egui::Shape, found: &mut Vec<(String, egui::Rect)>) {
	match shape {
		egui::Shape::Text(text) => found.push((
			text.galley.job.text.clone(),
			text.galley.rect.translate(text.pos.to_vec2()),
		)),
		egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, found)),
		_ => {}
	}
}

pub fn check() {
	let ctx = egui::Context::default();
	ui::design::apply(&ctx);
	let mut state = test_support::demo_state();
	let mut permissions = test_support::permission_snapshot(&state);
	for guild in &mut permissions.guilds {
		guild.owner = state.user.as_ref().map(|user| user.id);
	}
	state.permissions.replace(permissions).unwrap();
	state.select(Id(26));
	let post = state.forum_posts(Id(26))[0].clone();
	let mut view = ui::MessagingUi::default();
	let mut followed = false;
	let mut frame = |events: Vec<egui::Event>| {
		let mut commands = vec![];
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1120.0, 900.0),
				)),
				events,
				..Default::default()
			},
			|ui| commands = view.show(ui, &mut state),
		);
		let mut text = vec![];
		for shape in &output.shapes {
			labels(&shape.shape, &mut text);
		}
		let copied = output.platform_output.commands.iter().any(
			|c| matches!(c, egui::OutputCommand::CopyText(value) if value == &post.id.to_string()),
		);
		output.drop_without_applying_deltas();
		for command in commands {
			if let Command::ChannelAction {
				guild,
				channel,
				request,
				action,
			} = command
			{
				assert!(
					!matches!(action, client_core::channel_actions::Action::Delete),
					"opening delete confirmation must not delete"
				);
				followed |= matches!(
					action,
					client_core::channel_actions::Action::PostFollow(true)
				);
				let event = crate::channel_demo::execute(
					&state, guild, channel, request, action, &mut 9_000,
				);
				state.apply(Envelope {
					generation: state.generation,
					event,
				});
			}
		}
		assert_eq!(
			state.selected,
			Some(Id(26)),
			"right click must not navigate"
		);
		(text, copied)
	};
	frame(vec![]);
	let (text, _) = frame(vec![]);
	let pos = text
		.iter()
		.find(|(label, rect)| label == &post.name && rect.left() > 300.0)
		.expect("forum card")
		.1
		.center();
	let pointer = |pos, button, pressed| {
		vec![
			egui::Event::PointerMoved(pos),
			egui::Event::PointerButton {
				pos,
				button,
				pressed,
				modifiers: egui::Modifiers::NONE,
			},
		]
	};
	for pressed in [true, false] {
		frame(pointer(pos, egui::PointerButton::Secondary, pressed));
	}
	frame(vec![]);
	let (text, _) = frame(vec![]);
	for label in [
		"Mark As Read",
		"Add To Favorites",
		"Follow Post",
		"Close Post",
		"Lock Post",
		"Edit Post",
		"Pin Post",
		"Delete Post",
		"Copy Link",
		"Mute Post",
		"Notification Settings",
		"Copy Thread ID",
	] {
		assert!(
			text.iter().any(|(text, _)| text == label),
			"missing {label}"
		);
	}
	let pos = text
		.iter()
		.find(|(label, _)| label == "Copy Thread ID")
		.unwrap()
		.1
		.center();
	frame(pointer(pos, egui::PointerButton::Primary, true));
	assert!(frame(pointer(pos, egui::PointerButton::Primary, false)).1);
	let (text, _) = frame(vec![]);
	let card = text
		.iter()
		.find(|(label, rect)| label == &post.name && rect.left() > 300.0)
		.unwrap()
		.1
		.center();
	for pressed in [true, false] {
		frame(pointer(card, egui::PointerButton::Secondary, pressed));
	}
	frame(vec![]);
	let (text, _) = frame(vec![]);
	let follow = text
		.iter()
		.find(|(label, _)| label == "Follow Post")
		.unwrap()
		.1
		.center();
	for pressed in [true, false] {
		frame(pointer(follow, egui::PointerButton::Primary, pressed));
	}
	frame(vec![]);
	for pressed in [true, false] {
		frame(pointer(card, egui::PointerButton::Secondary, pressed));
	}
	frame(vec![]);
	let (text, _) = frame(vec![]);
	assert!(text.iter().any(|(label, _)| label == "Unfollow Post"));
	let delete = text
		.iter()
		.find(|(label, _)| label == "Delete Post")
		.unwrap()
		.1
		.center();
	for pressed in [true, false] {
		frame(pointer(delete, egui::PointerButton::Primary, pressed));
	}
	let (text, _) = frame(vec![]);
	assert!(text.iter().any(|(label, _)| label == "Delete Post?"));
	assert!(followed);
	println!(
		"Post menu debug check passed: right-click, all controls, follow, copy ID, delete confirmation, and unchanged selection."
	);
}
