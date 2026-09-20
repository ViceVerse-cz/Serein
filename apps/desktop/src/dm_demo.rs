//! Offline debug check: cargo run --locked -p serein --features demo -- --demo --demo-check-switcher
use client_core::{Command, Envelope, Event, State, auth::Failure, user_actions};
use eframe::egui;
use model::{Channel, Id};

pub fn channel(state: &State, user: Id) -> Result<Channel, Failure> {
	let recipient = state.friend(user).ok_or(Failure::Forbidden)?;
	Ok(state
		.channels
		.iter()
		.find(|c| {
			c.guild.is_none()
				&& c.kind == 1
				&& c.recipients.len() == 1
				&& c.recipients[0].id == user
		})
		.cloned()
		.unwrap_or_else(|| Channel {
			id: Id(100_000 + user.0),
			guild: None,
			name: recipient.name.clone(),
			kind: 1,
			parent_id: None,
			position: 0,
			recipients: vec![recipient.clone()],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
		}))
}

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
	events: Vec<egui::Event>,
) -> Vec<Command> {
	let mut commands = vec![];
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(1120.0, 800.0),
			)),
			focused: true,
			events,
			..Default::default()
		},
		|ui| commands = view.show(ui, state),
	);
	output.drop_without_applying_deltas();
	assert!(!commands.iter().any(|c| matches!(
		c,
		Command::Send { .. } | Command::Edit { .. } | Command::Voice(_)
	)));
	commands
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
	egui::Event::Key {
		key,
		physical_key: None,
		pressed: true,
		repeat: false,
		modifiers,
	}
}

/// Offline fixture check for the demo friend and username-search flow.
pub fn check() {
	let ctx = egui::Context::default();
	let mut view = ui::MessagingUi::default();
	let mut state = test_support::demo_state();
	let friend = Id(1003);
	let dm = channel(&state, friend).unwrap();
	assert!(
		state.channel(dm.id).is_none(),
		"friend must have no open DM"
	);
	let original = state.selected;
	let drafts = state.drafts.clone();
	frame(&ctx, &mut view, &mut state, vec![]);
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![key(egui::Key::K, egui::Modifiers::COMMAND)],
	);
	frame(&ctx, &mut view, &mut state, vec![]);
	frame(
		&ctx,
		&mut view,
		&mut state,
		vec![egui::Event::Text("MORGAN.SYNTHETIC".into())],
	);
	let commands = frame(
		&ctx,
		&mut view,
		&mut state,
		vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
	);
	let request = commands
		.into_iter()
		.find_map(|command| match command {
			Command::UserAction {
				action: user_actions::Action::OpenDm(user),
				request,
				..
			} if user == friend => Some(request),
			_ => None,
		})
		.expect("username search must open the friend's missing DM");
	assert_eq!(state.selected, original);
	assert!(
		state.open_friend_dm(friend).is_none(),
		"only one pending request"
	);
	apply(
		&mut state,
		user_actions::Event::DmOpened {
			user: friend,
			request: request + 1,
			result: Ok(Box::new(dm.clone())),
		},
	);
	assert!(
		state.channel(dm.id).is_none(),
		"stale request must be ignored"
	);
	apply(
		&mut state,
		user_actions::Event::DmOpened {
			user: friend,
			request,
			result: Ok(Box::new(dm.clone())),
		},
	);
	let commands = frame(&ctx, &mut view, &mut state, vec![]);
	assert_eq!(state.selected, Some(dm.id));
	assert!(
		commands
			.iter()
			.any(|c| matches!(c, Command::History { channel, .. } if *channel == dm.id))
	);
	assert_eq!(state.drafts, drafts);
	assert!(!state.user_action_pending());
	state.select(original.unwrap());
	assert!(
		matches!(state.open_friend_dm(friend), Some(Command::History { channel, .. }) if channel == dm.id)
	);
	assert_eq!(state.channels.iter().filter(|c| c.id == dm.id).count(), 1);

	for outcome in 0..4 {
		let mut state = test_support::demo_state();
		let Command::UserAction { request, .. } = state.open_friend_dm(friend).unwrap() else {
			panic!("open command")
		};
		let mut returned = dm.clone();
		match outcome {
			0 => {
				state.select(Id(21));
			}
			1 => returned.recipients[0].id = Id(999),
			2 => apply(
				&mut state,
				user_actions::Event::Friend {
					user: friend,
					friend: false,
					profile: None,
				},
			),
			_ => {}
		}
		let selected = state.selected;
		apply(
			&mut state,
			user_actions::Event::DmOpened {
				user: friend,
				request,
				result: if outcome == 3 {
					Err(Failure::Forbidden)
				} else {
					Ok(Box::new(returned))
				},
			},
		);
		assert!(state.select_opened_dm().is_none());
		assert_eq!(
			state.selected, selected,
			"late or rejected results must not move navigation"
		);
		assert!(!state.user_action_pending());
		if outcome > 0 {
			assert!(state.channel(dm.id).is_none());
		}
	}
	println!(
		"Switcher debug check passed: friend username, missing/existing DM, history, drafts, stale replies and failures."
	);
}
