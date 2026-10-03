//! Synthetic recovery check; no transport, storage, window or audio device is opened.
use crate::MessagingUi;
use client_core::{Command, Envelope, Event, State, auth::Failure};
use egui::{Event as Input, Pos2};
use model::{Delivery, Freshness};

fn apply(state: &mut State, event: Event) {
	state.apply(Envelope {
		generation: state.generation,
		event,
	});
}

fn frame(
	ctx: &egui::Context,
	view: &mut MessagingUi,
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
		|ui| commands.extend(view.show(ui, state)),
	);
	let nodes = output
		.platform_output
		.accesskit_update
		.as_ref()
		.unwrap()
		.nodes
		.iter()
		.filter_map(|(_, node)| {
			let bounds = node.bounds()?;
			Some((
				node.label()?.to_owned(),
				egui::pos2(
					((bounds.x0 + bounds.x1) / 2.0) as f32,
					((bounds.y0 + bounds.y1) / 2.0) as f32,
				),
				node.is_disabled(),
			))
		})
		.collect();
	output.drop_without_applying_deltas();
	(commands, nodes)
}

fn click(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	state: &mut State,
	label: &str,
) -> Vec<Command> {
	for _ in 0..3 {
		frame(ctx, view, state, vec![]);
	}
	let (_, nodes) = frame(ctx, view, state, vec![]);
	let (_, pos, disabled) = nodes
		.iter()
		.find(|(name, _, _)| name == label)
		.unwrap_or_else(|| panic!("Missing {label}"));
	assert!(!disabled, "{label} must remain enabled during recovery");
	let mut commands = vec![];
	for pressed in [true, false] {
		commands.extend(
			frame(
				ctx,
				view,
				state,
				vec![
					Input::PointerMoved(*pos),
					Input::PointerButton {
						pos: *pos,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			)
			.0,
		);
	}
	commands
}

pub fn check(mut state: State) {
	let channel = state
		.channels
		.iter()
		.find(|channel| channel.kind == 1)
		.unwrap()
		.id;
	state.demo = false;
	state.select(channel);
	let request = state.request;
	apply(
		&mut state,
		Event::History {
			channel,
			request,
			older: false,
			messages: vec![],
		},
	);
	assert!(state.can_send(channel));
	apply(&mut state, Event::Disconnected);
	assert!(!state.gateway_connected && state.can_send(channel));
	assert!(!state.can_attach(channel));
	let ctx = egui::Context::default();
	ctx.enable_accesskit();
	let mut view = MessagingUi {
		language: crate::i18n::Language::English,
		hide_title_bar: true,
		..Default::default()
	};
	for _ in 0..3 {
		frame(&ctx, &mut view, &mut state, vec![]);
	}
	// Recovery remains reachable even on platforms without a custom title bar.
	let commands = click(&ctx, &mut view, &mut state, "Reconnect now");
	assert!(view.reconnect_requested && commands.is_empty());
	view.reconnect_requested = false;
	// Refresh wakes Gateway recovery and independently reloads readable REST history.
	let commands = click(&ctx, &mut view, &mut state, "Reload history");
	assert!(view.reconnect_requested);
	let histories: Vec<_> = commands
		.iter()
		.filter(|command| matches!(command, Command::History { .. }))
		.collect();
	assert_eq!(histories.len(), 1);
	assert!(matches!(histories[0], Command::History { channel: id, .. } if *id == channel));
	let request = state.request;
	assert_eq!(state.freshness, Freshness::Loading);
	let commands = click(&ctx, &mut view, &mut state, "Reload history");
	assert!(
		!commands
			.iter()
			.any(|command| matches!(command, Command::History { .. }))
	);
	assert_eq!(
		state.request, request,
		"Refresh must not replace an in-flight REST request"
	);
	apply(&mut state, Event::Disconnected);
	assert!(state.history_pending);
	assert_eq!(
		state.request, request,
		"Gateway retries must preserve the REST reload"
	);
	assert_eq!(state.freshness, Freshness::Loading);
	view.reconnect_requested = false;
	state
		.drafts
		.insert(channel, "Synthetic message after wake".into());
	let commands = click(&ctx, &mut view, &mut state, "Send message");
	let sends: Vec<_> = commands
		.iter()
		.filter(|command| matches!(command, Command::Send { .. }))
		.collect();
	assert_eq!(
		sends.len(),
		1,
		"status={}, can_send={}, draft={:?}",
		state.status,
		state.can_send(channel),
		state.drafts.get(&channel)
	);
	let Command::Send { nonce, content, .. } = sends[0] else {
		unreachable!()
	};
	assert_eq!(content, "Synthetic message after wake");
	let nonce = nonce.clone();
	apply(
		&mut state,
		Event::SendResult {
			nonce,
			result: Err(Failure::Ambiguous),
		},
	);
	assert_eq!(state.pending[0].delivery, Delivery::Ambiguous);
	// The Refresh REST response finishes as Stale, never stuck Loading during an outage.
	apply(
		&mut state,
		Event::History {
			channel,
			request,
			older: false,
			messages: vec![],
		},
	);
	assert_eq!(state.freshness, Freshness::Stale);
	apply(&mut state, Event::Resumed);
	let commands = frame(&ctx, &mut view, &mut state, vec![]).0;
	assert!(
		!commands
			.iter()
			.any(|command| matches!(command, Command::Send { .. }))
	);
	assert_eq!(state.pending[0].delivery, Delivery::Ambiguous);
	assert!(state.can_send(channel));
	// Lost permissions, unavailable channels and terminal authentication still block sends.
	state.freshness = Freshness::Unavailable;
	assert!(!state.can_send(channel));
	state.freshness = Freshness::Stale;
	state
		.channels
		.iter_mut()
		.find(|c| c.id == channel)
		.unwrap()
		.kind = 2;
	assert!(!state.can_send(channel));
	state
		.channels
		.iter_mut()
		.find(|c| c.id == channel)
		.unwrap()
		.kind = 1;
	apply(&mut state, Event::Failure(Failure::Expired));
	assert!(!state.can_send(channel));
	println!(
		"Offline recovery passed: cross-platform reconnection, independent Refresh history, explicit send during Gateway outage, stale history completion, auth/access gates and no ambiguous-write replay."
	);
}

#[test]
fn disconnected_refresh_and_send_preserve_independent_rest_work() {
	check(test_support::demo_state());
}
