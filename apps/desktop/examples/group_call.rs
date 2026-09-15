//! Offline group-call check: cargo run --locked -p serein --example group_call
use client_core::{Command, Envelope, Event, State, voice};
use eframe::egui;
use model::Id;

fn participant(user: Id) -> voice::Participant {
	voice::Participant {
		user,
		muted: false,
		deafened: false,
		server_muted: false,
		server_deafened: false,
		video: false,
		streaming: false,
	}
}

fn render(state: &mut State, width: f32, required: &[&str]) {
	let ctx = egui::Context::default();
	ctx.enable_accesskit();
	ui::design::apply(&ctx);
	let mut view = ui::MessagingUi::default();
	view.voice_available = true;
	for _ in 0..3 {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(width, 760.0),
				)),
				..Default::default()
			},
			|ui| {
				// No desktop/network adapter consumes any command or media request.
				assert!(
					!view
						.show(ui, state)
						.iter()
						.any(|c| matches!(c, Command::Voice(_)))
				);
			},
		);
		let nodes = &output
			.platform_output
			.accesskit_update
			.as_ref()
			.unwrap()
			.nodes;
		for label in required {
			assert!(
				nodes.iter().any(|(_, node)| node.label() == Some(label)),
				"Missing {label} at width {width}"
			);
		}
		output.drop_without_applying_deltas();
	}
}

fn main() {
	let mut state = test_support::demo_state();
	let channel = Id(29);
	let own = state.user.as_ref().unwrap().id;
	state.select(channel);
	assert!(!state.can_call(channel));
	state.demo = false; // Synthetic state only; no connection, runtime or devices exist here.
	assert!(state.can_call(channel) && state.can_camera(channel) && state.can_stream(channel));
	for width in [1400.0, 800.0] {
		render(&mut state, width, &["Start voice call"]);
	}
	assert!(state.voice.active.is_none());
	assert!(matches!(
		state.start_call(channel, true),
		Some(Command::Voice(voice::Command::Join { ring: true, .. }))
	));
	state.leave_call().unwrap();

	let mut peers: Vec<_> = state
		.channel(channel)
		.unwrap()
		.recipients
		.iter()
		.map(|u| participant(u.id))
		.collect();
	peers.push(participant(own));
	let call = |participants| voice::Event::Call {
		channel,
		ringing: Some(vec![own]),
		participants: Some(participants),
		unavailable: false,
	};
	state.apply_voice(call(peers.clone()));
	assert!(state.voice.has_dm_call(channel) && state.voice.active.is_none());
	assert_eq!(state.voice.incoming, Some(channel));
	render(&mut state, 800.0, &["Answer", "Decline"]);
	assert!(matches!(
		state.decline_call(),
		Some(Command::Voice(voice::Command::Decline { .. }))
	));
	assert!(matches!(
		state.start_call(channel, true),
		Some(Command::Voice(voice::Command::Join { ring: false, .. }))
	));
	let request = state.voice.active.as_ref().unwrap().request;
	assert_eq!(state.voice.active.as_ref().unwrap().participants, peers);
	state.apply_voice(voice::Event::Progress {
		channel,
		request,
		phase: voice::Phase::Connected,
	});
	assert!(state.set_call_mute(true, true).is_some());
	assert!(state.set_call_camera(true).is_some());
	state.set_call_camera(false).unwrap();
	state.set_call_mute(false, false).unwrap();

	let mut newcomer = state.channel(channel).unwrap().recipients[0].clone();
	newcomer.id = Id(900);
	newcomer.name = "New group participant (synthetic)".into();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::RecipientAdded {
			channel,
			user: newcomer,
		},
	});
	let mut sharing = participant(Id(900));
	sharing.streaming = true;
	peers.push(sharing);
	state.apply_voice(call(peers.clone()));
	state.watch_stream(Id(900)).unwrap();
	let mut invalid = peers.clone();
	invalid.push(participant(Id(9999)));
	state.apply_voice(call(invalid));
	assert_eq!(state.voice.active.as_ref().unwrap().participants, peers);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::RecipientRemoved {
			channel,
			user: Id(900),
		},
	});
	assert!(state.voice.active.as_ref().unwrap().watching.is_none());
	assert!(
		!state
			.voice
			.active
			.as_ref()
			.unwrap()
			.participants
			.iter()
			.any(|p| p.user == Id(900))
	);

	// A ten-person group uses the shared bounded grid and all ordinary call controls.
	let group = state.channels.iter_mut().find(|c| c.id == channel).unwrap();
	let template = group.recipients[0].clone();
	for id in 100..107 {
		let mut user = template.clone();
		user.id = Id(id);
		user.name = format!("Synthetic member {id}");
		group.recipients.push(user);
	}
	peers = group.recipients.iter().map(|u| participant(u.id)).collect();
	peers.push(participant(own));
	assert_eq!(peers.len(), 10);
	state.apply_voice(call(peers));
	for width in [1400.0, 800.0] {
		render(
			&mut state,
			width,
			&[
				"Mute",
				"Deafen",
				"Turn on camera",
				"Share your screen",
				"Disconnect",
			],
		);
	}
	state.leave_call().unwrap();
	assert!(state.voice.has_dm_call(channel));
	render(&mut state, 800.0, &["Join call"]);
	state.start_call(channel, false).unwrap();
	assert_eq!(state.voice.active.as_ref().unwrap().participants.len(), 10);
	state.apply(Envelope {
		generation: state.generation,
		event: Event::RecipientRemoved { channel, user: own },
	});
	assert!(
		!state.can_call(channel)
			&& state.voice.active.is_none()
			&& !state.voice.has_dm_call(channel)
	);
	println!(
		"PASS: offline group start/ring, answer/decline/join, membership, media controls, removal and wide/narrow UI. No Discord or devices accessed."
	);
}
