use super::*;
use egui::{Event as InputEvent, PointerButton, ViewportCommand};

#[cfg(not(target_os = "windows"))]
fn frame(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	width: f32,
	events: Vec<InputEvent>,
) -> Vec<ViewportCommand> {
	frame_with_maximized(ctx, view, width, events, None)
}

fn frame_with_maximized(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	width: f32,
	events: Vec<InputEvent>,
	maximized: Option<bool>,
) -> Vec<ViewportCommand> {
	let mut raw = egui::RawInput {
		screen_rect: Some(egui::Rect::from_min_size(
			egui::Pos2::ZERO,
			egui::vec2(width, 600.0),
		)),
		focused: true,
		events,
		..Default::default()
	};
	if let Some(maximized) = maximized {
		raw.viewports
			.entry(egui::ViewportId::ROOT)
			.or_default()
			.maximized = Some(maximized);
	}
	let output = ctx.run_ui(raw, |ui| {
		view.title_bar(
			ui,
			&mut State::default(),
			&mut Vec::new(),
			"Synthetic context title",
		);

		// The desktop shows resize handles alongside the strip; include them
		// so corner clicks are tested against the real hit-testing stack.
		crate::design::window_resize(ui.ctx());
	});
	let commands = output.viewport_output[&egui::ViewportId::ROOT]
		.commands
		.clone();
	output.drop_without_applying_deltas();
	commands
}

fn pointer(pos: egui::Pos2, button: PointerButton, pressed: bool) -> Vec<InputEvent> {
	vec![
		InputEvent::PointerMoved(pos),
		InputEvent::PointerButton {
			pos,
			button,
			pressed,
			modifiers: egui::Modifiers::NONE,
		},
	]
}

#[cfg(target_os = "windows")]
#[test]
fn hidden_title_strip_does_not_start_window_drag() {
	let ctx = egui::Context::default();
	let mut view = MessagingUi::default();
	let mut state = test_support::demo_state();
	let run = |view: &mut MessagingUi, state: &mut State, events| {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(1200.0, 760.0),
				)),
				focused: true,
				events,
				..Default::default()
			},
			|ui| {
				view.show(ui, state);
			},
		);
		let commands = output.viewport_output[&egui::ViewportId::ROOT]
			.commands
			.clone();
		output.drop_without_applying_deltas();
		commands
	};
	let pos = egui::pos2(600.0, 18.0);
	run(&mut view, &mut state, vec![]);
	assert_eq!(
		run(
			&mut view,
			&mut state,
			pointer(pos, PointerButton::Primary, true)
		),
		[ViewportCommand::StartDrag]
	);
	run(
		&mut view,
		&mut state,
		pointer(pos, PointerButton::Primary, false),
	);
	view.hide_title_bar = true;
	run(&mut view, &mut state, vec![]);
	assert!(
		!run(
			&mut view,
			&mut state,
			pointer(pos, PointerButton::Primary, true)
		)
		.contains(&ViewportCommand::StartDrag)
	);
}

#[cfg(target_os = "windows")]
#[test]
fn restore_caption_button_emits_unmaximize_when_maximized() {
	let ctx = egui::Context::default();
	let mut view = MessagingUi::default();
	let width = 760.0;
	frame_with_maximized(&ctx, &mut view, width, vec![], Some(true));
	let pos = egui::pos2(width - 69.0, 18.0);
	assert!(
		frame_with_maximized(
			&ctx,
			&mut view,
			width,
			pointer(pos, PointerButton::Primary, true),
			Some(true)
		)
		.is_empty()
	);
	assert_eq!(
		frame_with_maximized(
			&ctx,
			&mut view,
			width,
			pointer(pos, PointerButton::Primary, false),
			Some(true)
		),
		[ViewportCommand::Maximized(false)]
	);
}

#[cfg(not(target_os = "windows"))]
#[test]
fn title_strip_does_not_emit_caption_commands() {
	for width in [760.0, 1200.0] {
		for (x, y) in [
			(width - 1.0, 1.0),
			(width - 8.0, 18.0),
			(width - 35.0, 18.0),
			(width - 81.0, 18.0),
			(width - 127.0, 18.0),
		] {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			frame(&ctx, &mut view, width, vec![]);
			let pos = egui::pos2(x, y);
			for pressed in [true, false] {
				let commands = frame(
					&ctx,
					&mut view,
					width,
					pointer(pos, PointerButton::Primary, pressed),
				);
				assert!(
					!commands.iter().any(|command| matches!(
						command,
						ViewportCommand::Close
							| ViewportCommand::Minimized(_)
							| ViewportCommand::Maximized(_)
					)),
					"width={width} pos={pos:?} pressed={pressed} commands={commands:?}"
				);
			}
		}
	}
	let ctx = egui::Context::default();
	let mut view = MessagingUi::default();
	frame(&ctx, &mut view, 760.0, vec![]);
	let pos = egui::pos2(380.0, 18.0);
	for _ in 0..2 {
		assert_eq!(
			frame(
				&ctx,
				&mut view,
				760.0,
				pointer(pos, PointerButton::Primary, true)
			),
			[ViewportCommand::StartDrag]
		);
		assert!(
			frame(
				&ctx,
				&mut view,
				760.0,
				pointer(pos, PointerButton::Primary, false)
			)
			.is_empty()
		);
	}
}
