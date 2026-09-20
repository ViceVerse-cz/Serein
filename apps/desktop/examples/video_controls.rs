//! Offline UI check, without a window, account, decoder, or audio device.
//! cargo run --locked -p serein --features demo --example video_controls
use eframe::egui;
use ui::DownloadUi;

fn frame(
	ctx: &egui::Context,
	view: &mut ui::MessagingUi,
	state: &mut client_core::State,
	events: Vec<egui::Event>,
	fullscreen: bool,
) -> egui::FullOutput {
	let mut input = egui::RawInput {
		screen_rect: Some(egui::Rect::from_min_size(
			egui::Pos2::ZERO,
			if fullscreen {
				egui::vec2(1600.0, 900.0)
			} else {
				egui::vec2(1000.0, 720.0)
			},
		)),
		focused: true,
		events,
		..Default::default()
	};
	input
		.viewports
		.get_mut(&egui::ViewportId::ROOT)
		.unwrap()
		.fullscreen = Some(fullscreen);
	ctx.run_ui(input, |ui| {
		let _ = view.show(ui, state);
		// Match the desktop's visibility-based player cleanup.
		let player = view.video();
		if player.active.is_some() && !player.seen {
			player.stop();
		}
	})
}

fn button(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
	let node = &output
		.platform_output
		.accesskit_update
		.as_ref()
		.unwrap()
		.nodes
		.iter()
		.find(|(_, node)| node.label() == Some(label))
		.unwrap_or_else(|| panic!("Missing {label}"))
		.1;
	let rect = node.bounds().unwrap();
	egui::pos2(
		((rect.x0 + rect.x1) * 0.5) as f32,
		((rect.y0 + rect.y1) * 0.5) as f32,
	)
}

fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
	vec![
		egui::Event::PointerMoved(pos),
		egui::Event::PointerButton {
			pos,
			button: egui::PointerButton::Primary,
			pressed,
			modifiers: egui::Modifiers::NONE,
		},
	]
}

fn main() {
	let ctx = egui::Context::default();
	ctx.enable_accesskit();
	ui::design::apply(&ctx);
	let mut view = ui::MessagingUi::default();
	let mut state = test_support::video_demo_state();
	let mut message = state.timeline.get(model::Id(601)).unwrap().clone();
	message.attachments[0].media.url = Some("https://example.com/synthetic-video.mp4".into());
	state.timeline.insert(message, false, false).unwrap();
	frame(&ctx, &mut view, &mut state, vec![], false).drop_without_applying_deltas();
	let message = state.timeline.get(model::Id(601)).unwrap();
	let video = view.video();
	video.active = Some((message.channel, message.id, message.attachments[0].clone()));
	video.position = 6.0;
	video.duration = 12.0;
	video.state = ui::VideoState::Loading;
	assert!(video.accept_frame(&ctx, 1, 1, &[10, 20, 30, 255]));
	for _ in 0..3 {
		let output = frame(&ctx, &mut view, &mut state, vec![], false);
		let seek = &output
			.platform_output
			.accesskit_update
			.as_ref()
			.unwrap()
			.nodes
			.iter()
			.find(|(_, node)| node.label() == Some("Seek video"))
			.expect("Seek slider")
			.1;
		assert_eq!(seek.numeric_value(), Some(6.0));
		assert_eq!(seek.max_numeric_value(), Some(12.0));
		output.drop_without_applying_deltas();
	}
	view.video().state = ui::VideoState::Paused;
	let output = frame(&ctx, &mut view, &mut state, vec![], false);
	let enter = button(&output, "Fullscreen");
	output.drop_without_applying_deltas();
	view.video().state = ui::VideoState::Playing;
	frame(&ctx, &mut view, &mut state, pointer(enter, true), false).drop_without_applying_deltas();
	let output = frame(&ctx, &mut view, &mut state, pointer(enter, false), false);
	output.drop_without_applying_deltas();
	assert_eq!(view.video().take_fullscreen_request(), Some(true));
	assert_eq!(view.video().state, ui::VideoState::Playing);
	assert!(view.video().command.is_none());
	// Native fullscreen changes asynchronously and can resize through intermediate sizes.
	for fullscreen in [false, true, true, false, true] {
		let output = frame(&ctx, &mut view, &mut state, vec![], fullscreen);
		let exit = button(&output, "Exit fullscreen (Esc)");
		assert!(
			exit.x > if fullscreen { 1400.0 } else { 800.0 }
				&& exit.y > if fullscreen { 800.0 } else { 600.0 },
			"Fullscreen controls fill the viewport: {exit:?}"
		);
		assert!(
			!output
				.platform_output
				.accesskit_update
				.as_ref()
				.unwrap()
				.nodes
				.iter()
				.any(|(_, node)| node.label() == Some("Send message")),
			"Fullscreen must render only the player, without the chat composer"
		);
		assert!(view.video().seen);
		assert_eq!(view.video().position, 6.0);
		assert!(view.video().command.is_none());
		output.drop_without_applying_deltas();
	}
	let output = frame(
		&ctx,
		&mut view,
		&mut state,
		vec![egui::Event::Key {
			key: egui::Key::Escape,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		}],
		true,
	);
	output.drop_without_applying_deltas();
	assert_eq!(view.video().take_fullscreen_request(), Some(false));
	assert_eq!(view.video().state, ui::VideoState::Playing);
	assert_eq!(view.video().position, 6.0);
	view.video().state = ui::VideoState::Paused;
	let output = frame(&ctx, &mut view, &mut state, vec![], false);
	let enter = button(&output, "Fullscreen");
	output.drop_without_applying_deltas();
	for pressed in [true, false] {
		frame(&ctx, &mut view, &mut state, pointer(enter, pressed), false)
			.drop_without_applying_deltas();
	}
	frame(&ctx, &mut view, &mut state, vec![], true).drop_without_applying_deltas();
	for pressed in [true, false] {
		let mut events = pointer(egui::pos2(800.0, 450.0), pressed);
		for event in &mut events {
			if let egui::Event::PointerButton { button, .. } = event {
				*button = egui::PointerButton::Secondary;
			}
		}
		frame(&ctx, &mut view, &mut state, events, true).drop_without_applying_deltas();
	}
	let output = frame(&ctx, &mut view, &mut state, vec![], true);
	let open = button(&output, "Open original\u{2026}");
	output.drop_without_applying_deltas();
	frame(&ctx, &mut view, &mut state, pointer(open, true), true).drop_without_applying_deltas();
	let output = frame(&ctx, &mut view, &mut state, pointer(open, false), true);
	assert!(
		!output
			.platform_output
			.commands
			.iter()
			.any(|command| matches!(command, egui::OutputCommand::OpenUrl(_)))
	);
	output.drop_without_applying_deltas();
	assert_eq!(
		view.video().take_fullscreen_request(),
		Some(false),
		"Open original must restore the window before confirmation"
	);
	let output = frame(&ctx, &mut view, &mut state, vec![], false);
	let _ = button(&output, "Open in Browser");
	assert!(
		!output
			.platform_output
			.commands
			.iter()
			.any(|command| matches!(command, egui::OutputCommand::OpenUrl(_))),
		"The synthetic external URL must await confirmation"
	);
	output.drop_without_applying_deltas();
	let message = state.timeline.get(model::Id(601)).unwrap();
	for width in [220.0, 420.0] {
		let compact = egui::Context::default();
		compact
			.run_ui(egui::RawInput::default(), |ui| {
				ui.set_width(width);
				view.video().show(
					ui,
					message,
					&message.attachments[0],
					&mut DownloadUi::default(),
					&mut None,
					false,
				);
				assert!(
					ui.min_rect().width() <= width + 2.0,
					"Compact video controls overflow"
				);
			})
			.drop_without_applying_deltas();
	}
	println!(
		"Offline video UI passed: seek range stays stable during loading; fullscreen fills the viewport; Escape restores playback; Open original exits fullscreen and awaits confirmation."
	);
}
