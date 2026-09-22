use crate::timeline::TimelineView;
use client_core::{Envelope, Event, State};
use egui::{Context, Rect};
use model::{Delivery, Freshness, Id};

fn labels(shape: &egui::Shape, output: &mut Vec<(String, Rect)>) {
	match shape {
		egui::Shape::Text(text) => output.push((
			text.galley.job.text.clone(),
			text.galley.rect.translate(text.pos.to_vec2()),
		)),
		egui::Shape::Vec(shapes) => {
			for shape in shapes {
				labels(shape, output);
			}
		}
		_ => {}
	}
}

fn frame(
	ctx: &Context,
	view: &mut TimelineView,
	state: &mut State,
	width: f32,
) -> Vec<(String, Rect)> {
	let mut painted = vec![];
	for _ in 0..3 {
		let output = ctx.run_ui(
			egui::RawInput {
				focused: true,
				screen_rect: Some(Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(width, 600.0),
				)),
				..Default::default()
			},
			|ui| {
				view.show(
					ui,
					state,
					&mut None,
					&mut None,
					(
						&mut crate::avatars::Avatars::default(),
						&mut crate::profiles::ProfileSession::default(),
					),
					None,
				);
				assert!(ui.min_rect().right() <= ui.max_rect().right() + 1.0);
			},
		);
		assert!(output.platform_output.commands.is_empty());
		painted.clear();
		for shape in &output.shapes {
			labels(&shape.shape, &mut painted);
		}
		output.drop_without_applying_deltas();
	}
	painted
}

fn welcomes(painted: &[(String, Rect)]) -> bool {
	painted
		.iter()
		.any(|(text, _)| text.starts_with("Welcome to #"))
}

#[test]
fn welcome_tracks_confirmed_empty_history_messages_and_pending_delivery() {
	let mut state = test_support::empty_channel_demo_state(false);
	state.set_preserve_deleted_messages(true);
	let channel = state.selected.unwrap();
	let ctx = Context::default();
	let mut view = TimelineView::default();
	assert_eq!(state.freshness, Freshness::Fresh);
	assert!(state.older_exhausted && !state.history_pending);
	assert!(welcomes(&frame(&ctx, &mut view, &mut state, 700.0)));
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Message(test_support::message(600, channel)),
	});
	assert!(!welcomes(&frame(&ctx, &mut view, &mut state, 700.0)));
	state.apply(Envelope {
		generation: state.generation,
		event: Event::Delete {
			channel,
			id: Id(600),
		},
	});
	assert!(state.timeline.get(Id(600)).is_none());
	assert!(
		state
			.timeline
			.display_iter()
			.any(|message| message.id == Id(600))
	);
	assert_ne!(
		state.timeline.row_count(),
		0,
		"Deletion retains a tombstone"
	);
	assert!(!welcomes(&frame(&ctx, &mut view, &mut state, 700.0)));
	state
		.drafts
		.insert(channel, "Synthetic first message".into());
	assert!(state.prepare_send().is_some()); // No command is executed by this test.
	for delivery in [Delivery::Sending, Delivery::Rejected, Delivery::Ambiguous] {
		state.pending[0].delivery = delivery;
		assert!(!welcomes(&frame(&ctx, &mut view, &mut state, 700.0)));
	}
	state.pending[0].channel = Id(999);
	assert!(!welcomes(&frame(&ctx, &mut view, &mut state, 700.0)));
}

#[test]
fn welcome_requires_readable_complete_latest_guild_history() {
	for scenario in 0..11 {
		let mut state = test_support::empty_channel_demo_state(false);
		match scenario {
			0 => state.freshness = Freshness::Loading,
			1 => state.freshness = Freshness::Stale,
			2 => state.freshness = Freshness::Unavailable,
			3 => state.permissions = Default::default(),
			4 => state.history_targeted = true,
			5 => state.history_before = Some(Id(600)),
			6 => state.history_after = Some(Id(600)),
			7 => state.history_pending = true,
			8 => state.older_exhausted = false,
			9 => {
				let selected = state.selected.unwrap();
				let channel = state
					.channels
					.iter_mut()
					.find(|c| c.id == selected)
					.unwrap();
				channel.guild = None;
				channel.kind = 1;
			}
			_ => state.selected = None,
		}
		assert!(
			!welcomes(&frame(
				&Context::default(),
				&mut TimelineView::default(),
				&mut state,
				700.0
			)),
			"Unavailable or incomplete scenario {scenario} showed a channel welcome"
		);
	}
	for kind in [0, 5, 11] {
		let mut state = test_support::empty_channel_demo_state(false);
		let selected = state.selected.unwrap();
		if kind == 11 {
			let mut thread = state.channel(selected).unwrap().clone();
			thread.id = Id(999);
			thread.kind = kind;
			thread.parent_id = Some(selected);
			state.selected = Some(thread.id);
			state.channels.push(thread);
		} else {
			state
				.channels
				.iter_mut()
				.find(|c| c.id == selected)
				.unwrap()
				.kind = kind;
		}
		assert!(welcomes(&frame(
			&Context::default(),
			&mut TimelineView::default(),
			&mut state,
			700.0
		)));
	}
}

#[test]
fn long_channel_welcome_wraps_within_narrow_light_and_dark_viewports() {
	for dark in [true, false] {
		let ctx = Context::default();
		ctx.set_visuals(if dark {
			egui::Visuals::dark()
		} else {
			egui::Visuals::light()
		});
		let mut state = test_support::empty_channel_demo_state(true);
		let mut view = TimelineView::default();
		for width in [700.0, 240.0] {
			let painted = frame(&ctx, &mut view, &mut state, width);
			let (_, heading) = painted
				.iter()
				.find(|(text, _)| text.starts_with("Welcome to #"))
				.unwrap();
			let (_, description) = painted
				.iter()
				.find(|(text, _)| text == "This is the beginning of the conversation.")
				.unwrap();
			for rect in [heading, description] {
				assert!(rect.left() >= 0.0 && rect.right() <= width + 1.0);
				assert!(rect.top() >= 0.0 && rect.bottom() <= 601.0);
			}
			assert!(description.top() >= heading.bottom());
			if width < 300.0 {
				assert!(heading.height() > 60.0, "The long name must wrap");
			}
		}
	}
}
