use super::*;
use egui::{Event as InputEvent, PointerButton, ViewportCommand};

fn frame(
	ctx: &egui::Context,
	view: &mut MessagingUi,
	width: f32,
	events: Vec<InputEvent>,
) -> Vec<ViewportCommand> {
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(width, 600.0),
			)),
			focused: true,
			events,
			..Default::default()
		},
		|ui| view.title_bar(ui, &State::default(), "Synthetic context title"),
	);
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

#[test]
fn title_strip_primary_press_starts_drag_immediately_and_only_once() {
	for width in [760.0, 1200.0] {
		for dark in [true, false] {
			for x in [24.0, width * 0.22, width * 0.5] {
				let ctx = egui::Context::default();
				ctx.set_visuals(if dark {
					egui::Visuals::dark()
				} else {
					egui::Visuals::light()
				});
				let mut view = MessagingUi::default();
				frame(&ctx, &mut view, width, vec![]);
				let pos = egui::pos2(x, 18.0);
				assert_eq!(
					frame(
						&ctx,
						&mut view,
						width,
						pointer(pos, PointerButton::Primary, true)
					),
					[ViewportCommand::StartDrag],
					"width={width}, dark={dark}, x={x}"
				);
				let moved = pos + egui::vec2(12.0, 2.0);
				assert!(
					frame(
						&ctx,
						&mut view,
						width,
						vec![InputEvent::PointerMoved(moved)]
					)
					.is_empty()
				);
				assert!(
					frame(
						&ctx,
						&mut view,
						width,
						pointer(moved, PointerButton::Primary, false)
					)
					.is_empty()
				);
			}
		}
	}
}

#[test]
fn secondary_press_and_content_press_do_not_drag() {
	for (pos, button) in [
		(egui::pos2(24.0, 18.0), PointerButton::Secondary),
		(egui::pos2(24.0, 90.0), PointerButton::Primary),
	] {
		let ctx = egui::Context::default();
		let mut view = MessagingUi::default();
		frame(&ctx, &mut view, 760.0, vec![]);
		for pressed in [true, false] {
			assert!(frame(&ctx, &mut view, 760.0, pointer(pos, button, pressed)).is_empty());
		}
	}
}

#[cfg(target_os = "windows")]
#[test]
fn caption_buttons_act_without_dragging_and_title_double_click_maximizes() {
	for width in [760.0, 1200.0] {
		for (offset, expected) in [
			(35.0, ViewportCommand::Close),
			(81.0, ViewportCommand::Maximized(true)),
			(127.0, ViewportCommand::Minimized(true)),
		] {
			let ctx = egui::Context::default();
			let mut view = MessagingUi::default();
			frame(&ctx, &mut view, width, vec![]);
			let pos = egui::pos2(width - offset, 18.0);
			assert!(
				frame(
					&ctx,
					&mut view,
					width,
					pointer(pos, PointerButton::Primary, true)
				)
				.is_empty()
			);
			assert_eq!(
				frame(
					&ctx,
					&mut view,
					width,
					pointer(pos, PointerButton::Primary, false)
				),
				[expected]
			);
		}
	}
	let ctx = egui::Context::default();
	let mut view = MessagingUi::default();
	frame(&ctx, &mut view, 760.0, vec![]);
	let pos = egui::pos2(380.0, 18.0);
	for click in 0..2 {
		assert_eq!(
			frame(
				&ctx,
				&mut view,
				760.0,
				pointer(pos, PointerButton::Primary, true)
			),
			[ViewportCommand::StartDrag]
		);
		let released = frame(
			&ctx,
			&mut view,
			760.0,
			pointer(pos, PointerButton::Primary, false),
		);
		if click == 0 {
			assert!(released.is_empty());
		} else {
			assert_eq!(released, [ViewportCommand::Maximized(true)]);
		}
	}
}
