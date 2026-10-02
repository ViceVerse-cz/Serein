//! Offline check: cargo run --locked -p ui --features demo --example friend_message
use client_core::{Command, Envelope, Event, State, auth::Failure, user_actions};
use egui::{Event as Input, Pos2};
use model::Id;

fn apply(state: &mut State, event: user_actions::Event) {
	state.apply(Envelope {
		generation: state.generation,
		event: Event::UserAction(event),
	});
}

fn frame(
	ctx: &egui::Context,
	view: &mut ui::MessagingUi,
	state: &mut State,
	events: Vec<Input>,
) -> (Vec<Command>, Vec<(String, Pos2, bool)>) {
	let mut commands = vec![];
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				Pos2::ZERO,
				egui::vec2(1120.0, 800.0),
			)),
			focused: true,
			events,
			..Default::default()
		},
		|ui| commands = view.show(ui, state),
	);
	assert!(
		!commands
			.iter()
			.any(|c| matches!(c, Command::Send { .. } | Command::Voice(_)))
	);
	let buttons = output
		.platform_output
		.accesskit_update
		.as_ref()
		.unwrap()
		.nodes
		.iter()
		.filter_map(|(_, node)| {
			let label = node.label()?;
			let bounds = node.bounds()?;
			Some((
				label.to_owned(),
				egui::pos2(
					((bounds.x0 + bounds.x1) / 2.0) as f32,
					((bounds.y0 + bounds.y1) / 2.0) as f32,
				),
				node.is_disabled(),
			))
		})
		.collect();
	output.drop_without_applying_deltas();
	(commands, buttons)
}

fn main() {
	// Exercise both real hit targets, existing and missing DMs, and service rejection.
	for (row, existing, forbidden) in [
		(false, false, false),
		(true, false, false),
		(false, true, false),
		(true, true, false),
		(false, false, true),
	] {
		let mut state = test_support::friends_demo_state();
		state.selected = None;
		let friend = state.friend(Id(1003)).unwrap().clone();
		let username = state.friend_username(friend.id).unwrap().to_owned();
		apply(
			&mut state,
			user_actions::Event::Friends(Some(vec![(friend.clone(), username)])),
		);
		let mut dm = state.channels.iter().find(|c| c.kind == 1).unwrap().clone();
		dm.id = Id(991003);
		dm.recipients = vec![friend.clone()];
		dm.name = friend.name.clone();
		assert!(
			!state
				.channels
				.iter()
				.any(|c| c.kind == 1 && c.recipients.iter().any(|u| u.id == friend.id))
		);
		if existing {
			state.apply(Envelope {
				generation: state.generation,
				event: Event::ChannelCreated(dm.clone()),
			});
		}
		let drafts = state.drafts.clone();
		let ctx = egui::Context::default();
		ctx.enable_accesskit();
		ui::design::apply(&ctx);
		let mut view = ui::MessagingUi::default();
		view.preview_friends_tab("all");
		let mut buttons = vec![];
		for _ in 0..3 {
			let (commands, found) = frame(&ctx, &mut view, &mut state, vec![]);
			assert!(
				commands
					.iter()
					.all(|c| !matches!(c, Command::UserAction { .. }))
			);
			buttons = found;
		}
		let label = if row { friend.name.as_str() } else { "Message" };
		let (_, pos, disabled) = buttons
			.iter()
			.find(|(name, _, _)| name == label)
			.expect("friend action visible");
		assert!(
			!disabled,
			"friend action must be enabled without a loaded DM"
		);
		let pos = *pos;
		let mut commands = vec![];
		for pressed in [true, false] {
			commands.extend(
				frame(
					&ctx,
					&mut view,
					&mut state,
					vec![
						Input::PointerMoved(pos),
						Input::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				)
				.0,
			);
		}
		if existing {
			assert!(
				!commands
					.iter()
					.any(|c| matches!(c, Command::UserAction { .. }))
			);
		} else {
			let requests: Vec<_> = commands
				.iter()
				.filter_map(|c| match c {
					Command::UserAction {
						action: user_actions::Action::OpenDm(user),
						request,
						..
					} if *user == friend.id => Some(*request),
					_ => None,
				})
				.collect();
			assert_eq!(requests.len(), 1, "one explicit DM open request");
			assert_eq!(state.selected, None, "wait for service confirmation");
			assert!(
				state.open_friend_dm(friend.id).is_none(),
				"suppress duplicate requests"
			);
			apply(
				&mut state,
				user_actions::Event::DmOpened {
					user: friend.id,
					request: requests[0],
					result: if forbidden {
						Err(Failure::Forbidden)
					} else {
						Ok(Box::new(dm.clone()))
					},
				},
			);
			commands = frame(&ctx, &mut view, &mut state, vec![]).0;
		}
		if forbidden {
			assert_eq!(state.selected, None);
			assert!(state.channel(dm.id).is_none());
			assert!(
				state.user_action_status().is_some() || state.status != "Direct message opened"
			);
		} else {
			assert_eq!(state.selected, Some(dm.id));
			assert!(
				commands
					.iter()
					.any(|c| matches!(c, Command::History { channel, .. } if *channel == dm.id))
			);
		}
		assert_eq!(state.drafts, drafts);
		assert!(!state.user_action_pending());
	}
	println!(
		"Friends Message check passed: row/button, missing/existing DM, single request, confirmed navigation/history, rejected request, preserved drafts; synthetic only."
	);
}
