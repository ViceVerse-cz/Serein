use super::*;
use client_core::{Command, Envelope, Event, auth::Failure};
use model::Delivery;

fn send(state: &mut State, text: &str) -> String {
	state.drafts.insert(state.selected.unwrap(), text.into());
	let Command::Send { nonce, .. } = state.prepare_send().unwrap() else {
		panic!("expected send command")
	};
	nonce
}

fn apply(state: &mut State, event: Event) {
	state.apply(Envelope {
		generation: state.generation,
		event,
	});
}

fn render(
	ctx: &egui::Context,
	view: &mut TimelineView,
	state: &mut State,
	width: f32,
	mut events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect, egui::Color32)> {
	fn collect(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect, egui::Color32)>) {
		match shape {
			egui::Shape::Text(text) => out.push((
				text.galley.job.text.clone(),
				text.visual_bounding_rect(),
				text.galley.job.sections[0].format.color,
			)),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
			_ => {}
		}
	}
	let mut painted = Vec::new();
	let mut avatars = crate::avatars::Avatars::default();
	for _ in 0..6 {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(width, 600.0),
				)),
				events: std::mem::take(&mut events),
				..Default::default()
			},
			|ui| {
				view.show(
					ui,
					state,
					&mut None,
					&mut None,
					(
						&mut avatars,
						&mut crate::profiles::ProfileSession::default(),
					),
					None,
				)
			},
		);
		painted.clear();
		for shape in &output.shapes {
			collect(&shape.shape, &mut painted);
		}
		output.drop_without_applying_deltas();
	}
	painted
}

#[test]
fn pending_full_body_turns_into_one_confirmed_row_in_either_arrival_order() {
	for (width, theme) in [(900.0, egui::Theme::Dark), (300.0, egui::Theme::Light)] {
		for gateway_first in [false, true] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.set_theme(theme);
			let mut state = test_support::demo_state();
			state.timeline.clear();
			state.read_state.reset();
			let body = "This synthetic outgoing message is deliberately longer than eighty characters so its complete body must be visible while sending.";
			let nonce = send(&mut state, body);
			let mut view = TimelineView::default();
			let pending = render(&ctx, &mut view, &mut state, width, vec![]);
			let rows: Vec<_> = pending.iter().filter(|(s, _, _)| s == body).collect();
			assert_eq!(rows.len(), 1, "pending body must appear without truncation");
			let pending_color = rows[0].2;
			if width < 400.0 {
				assert!(rows[0].1.height() > 30.0, "long body must wrap");
			}
			let mut message = test_support::message(900, state.selected.unwrap());
			message.author = state.user.clone().unwrap();
			message.content = body.into();
			message.nonce = Some(nonce.clone());
			let gateway = Event::Message(message.clone());
			let rest = Event::SendResult {
				nonce,
				result: Ok(message),
			};
			let events = if gateway_first {
				[gateway, rest]
			} else {
				[rest, gateway]
			};
			for event in events {
				apply(&mut state, event);
				let confirmed = render(&ctx, &mut view, &mut state, width, vec![]);
				let rows: Vec<_> = confirmed.iter().filter(|(s, _, _)| s == body).collect();
				assert_eq!(
					rows.len(),
					1,
					"confirmation must replace, not duplicate, pending text"
				);
				assert_ne!(
					rows[0].2, pending_color,
					"confirmed text must lose pending gray"
				);
				assert!(state.pending.is_empty());
			}
		}
	}
}

#[test]
fn pending_failures_keep_restore_action_and_other_channels_stay_hidden() {
	for (failure, label) in [
		(Failure::Forbidden, "Not sent"),
		(Failure::Ambiguous, "Delivery unknown"),
	] {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		state.timeline.clear();
		state.read_state.reset();
		let nonce = send(&mut state, "Failed synthetic body");
		let other = send(&mut state, "Other channel body");
		state
			.pending
			.iter_mut()
			.find(|p| p.nonce == other)
			.unwrap()
			.channel = Id(21);
		apply(
			&mut state,
			Event::SendResult {
				nonce: nonce.clone(),
				result: Err(failure),
			},
		);
		let mut view = TimelineView::default();
		let painted = render(&ctx, &mut view, &mut state, 500.0, vec![]);
		assert!(painted.iter().any(|(s, _, _)| s == "Failed synthetic body"));
		assert!(painted.iter().any(|(s, _, _)| s == label));
		assert!(!painted.iter().any(|(s, _, _)| s == "Other channel body"));
		let position = painted
			.iter()
			.find(|(s, _, _)| s == "Restore to composer")
			.unwrap()
			.1
			.center();
		let click = [true, false].map(|pressed| egui::Event::PointerButton {
			pos: position,
			button: egui::PointerButton::Primary,
			pressed,
			modifiers: egui::Modifiers::NONE,
		});
		render(
			&ctx,
			&mut view,
			&mut state,
			500.0,
			vec![
				egui::Event::PointerMoved(position),
				click[0].clone(),
				click[1].clone(),
			],
		);
		assert_eq!(view.restore_pending.as_deref(), Some(nonce.as_str()));
		state.selected = Some(Id(21));
		let painted = render(&ctx, &mut view, &mut state, 500.0, vec![]);
		assert!(painted.iter().any(|(s, _, _)| s == "Other channel body"));
		assert!(!painted.iter().any(|(s, _, _)| s == "Failed synthetic body"));
	}
}

#[test]
fn many_pending_rows_follow_the_last_row_and_prune_on_channel_switch() {
	let ctx = egui::Context::default();
	crate::design::apply(&ctx);
	let mut state = test_support::demo_state();
	state.timeline.clear();
	state.read_state.reset();
	for index in 0..40 {
		send(&mut state, &format!("Pending synthetic row {index}"));
	}
	let mut view = TimelineView::default();
	let painted = render(&ctx, &mut view, &mut state, 320.0, vec![]);
	let last = painted
		.iter()
		.find(|(s, _, _)| s == "Pending synthetic row 39")
		.unwrap();
	assert!(
		last.1.top() >= 0.0 && last.1.bottom() <= 600.0,
		"last pending row must be visible"
	);
	assert!(view.following);
	state.selected = Some(Id(21));
	render(&ctx, &mut view, &mut state, 320.0, vec![]);
	assert!(view.pending_heights.is_empty());
}

#[test]
fn restore_processing_preserves_existing_drafts_and_ignores_late_confirmations() {
	for scenario in 0..4 {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let nonce = send(&mut state, "Text to recover");
		state.pending[0].delivery = Delivery::Ambiguous;
		state.pending[0].attachments = vec!["example.txt".into()];
		if scenario == 1 {
			state.drafts.insert(channel, "Newer draft".into());
		} else if scenario == 2 {
			state.pending.clear(); // Confirmation can arrive after the restore click.
		} else if scenario == 3 {
			for id in 1000..1064 {
				state.drafts.insert(Id(id), "Other draft".into());
			}
		}
		let mut messaging = crate::MessagingUi::default();
		messaging.restore_pending(&mut state, channel, &nonce);
		match scenario {
			0 => {
				assert_eq!(state.drafts[&channel], "Text to recover");
				assert!(state.pending.is_empty());
				assert!(state.status.contains("reselect the attachment"));
				assert!(messaging.draft_changes.contains(&channel));
			}
			1 => {
				assert_eq!(state.drafts[&channel], "Newer draft");
				assert_eq!(state.pending.len(), 1);
			}
			2 => assert!(!state.drafts.contains_key(&channel)),
			_ => {
				assert!(!state.drafts.contains_key(&channel));
				assert_eq!(state.pending.len(), 1);
				assert_eq!(state.drafts.len(), 64);
			}
		}
	}
}

#[test]
fn sending_from_scrolled_history_returns_to_latest_without_resending() {
	fn composer_rect(shape: &egui::Shape) -> Option<egui::Rect> {
		match shape {
			egui::Shape::Text(text) if text.galley.job.text == "Send from historical view" => {
				Some(text.visual_bounding_rect())
			}
			egui::Shape::Vec(shapes) => shapes.iter().find_map(composer_rect),
			_ => None,
		}
	}
	for targeted in [false, true] {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		state.read_state.reset();
		let channel = state.selected.unwrap();
		state
			.drafts
			.insert(channel, "Send from historical view".into());
		let mut messaging = crate::MessagingUi::default();
		let frame = |messaging: &mut crate::MessagingUi, state: &mut State, events| {
			let mut commands = Vec::new();
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(1000.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| commands.extend(messaging.show(ui, state)),
			);
			let rect = output
				.shapes
				.iter()
				.find_map(|shape| composer_rect(&shape.shape));
			output.drop_without_applying_deltas();
			(commands, rect)
		};
		for _ in 0..6 {
			frame(&mut messaging, &mut state, vec![]);
		}
		let position = frame(&mut messaging, &mut state, vec![])
			.1
			.expect("draft is rendered")
			.center();
		let click = [true, false].map(|pressed| egui::Event::PointerButton {
			pos: position,
			button: egui::PointerButton::Primary,
			pressed,
			modifiers: egui::Modifiers::NONE,
		});
		frame(
			&mut messaging,
			&mut state,
			vec![
				egui::Event::PointerMoved(position),
				click[0].clone(),
				click[1].clone(),
			],
		);
		let editor = ctx
			.memory(|memory| memory.focused())
			.expect("click focuses composer");
		messaging.timeline.browse_away();
		messaging.timeline.anchor = Some((state.timeline.row_ids().next().unwrap(), 0.0));
		messaging.timeline.revision = u64::MAX;
		state.history_targeted = targeted;
		frame(&mut messaging, &mut state, vec![]);
		assert!(!messaging.timeline.following);
		ctx.memory_mut(|memory| memory.request_focus(editor));
		let (commands, _) = frame(
			&mut messaging,
			&mut state,
			vec![egui::Event::Key {
				key: egui::Key::Enter,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			}],
		);
		assert_eq!(
			commands
				.iter()
				.filter(|c| matches!(c, Command::Send { .. }))
				.count(),
			1
		);
		assert_eq!(
			commands
				.iter()
				.filter(|c| matches!(c, Command::History { .. }))
				.count(),
			usize::from(targeted)
		);
		assert!(!state.history_targeted);
		assert_eq!(state.pending.len(), 1);
		assert_eq!(state.pending[0].content, "Send from historical view");
		for _ in 0..6 {
			assert!(
				!frame(&mut messaging, &mut state, vec![])
					.0
					.iter()
					.any(|c| matches!(c, Command::Send { .. }))
			);
		}
		assert!(messaging.timeline.following);
	}
}
