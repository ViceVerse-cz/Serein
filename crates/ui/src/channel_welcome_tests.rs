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

/// Visible text without Fluent's isolation marks around inserted names.
fn plain(text: &str) -> String {
	text.replace(['\u{2068}', '\u{2069}'], "")
}

/// The empty channel's full-height welcome; the history introduction ends with "!".
fn welcomes(painted: &[(String, Rect)]) -> bool {
	painted.iter().any(|(text, _)| {
		let text = plain(text);
		text.starts_with("Welcome to #") && !text.ends_with('!')
	})
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

fn dm_history(state: &mut State, count: u64) {
	history(state, Id(22), count);
}

fn history(state: &mut State, channel: Id, count: u64) {
	state.gateway_connected = true;
	state.auth = client_core::auth::AuthState::Authenticated;
	let Some(client_core::Command::History { request, .. }) = state.select(channel) else {
		panic!("conversation history requested");
	};
	let messages = (0..count)
		.rev()
		.map(|n| {
			let mut message = test_support::message(1_000_000 + n, channel);
			message.attachments.clear();
			message.embeds.clear();
			message
		})
		.collect();
	state.apply(Envelope {
		generation: state.generation,
		event: Event::History {
			channel,
			request,
			older: false,
			messages,
		},
	});
}

fn introduces(painted: &[(String, Rect)]) -> bool {
	painted
		.iter()
		.any(|(text, _)| text.starts_with("This is the beginning of your direct message history"))
}

#[test]
fn direct_messages_open_with_an_introduction_once_history_is_complete() {
	let ctx = Context::default();
	// A short history is the whole conversation: the introduction leads it.
	let mut state = test_support::demo_state();
	dm_history(&mut state, 3);
	assert!(state.older_exhausted);
	let mut view = TimelineView::default();
	let painted = frame(&ctx, &mut view, &mut state, 900.0);
	assert!(introduces(&painted));
	assert!(!welcomes(&painted));
	// Reached by scrolling up a long history or from a search result, the start still
	// introduces the conversation even though the newest messages are not loaded.
	let mut state = test_support::demo_state();
	dm_history(&mut state, 3);
	state.history_targeted = true;
	state.history_after = Some(model::Id(1_000_000));
	let mut view = TimelineView::default();
	assert!(introduces(&frame(&ctx, &mut view, &mut state, 900.0)));
	// A full page may have older messages: no introduction until they are loaded.
	let mut state = test_support::demo_state();
	dm_history(&mut state, 50);
	assert!(!state.older_exhausted);
	let mut view = TimelineView::default();
	assert!(!introduces(&frame(&ctx, &mut view, &mut state, 900.0)));
}

#[test]
fn channels_and_forum_posts_open_with_their_introduction_once_history_is_complete() {
	let ctx = Context::default();
	let painted_text = |channel: Id, count: u64| {
		let mut state = test_support::demo_state();
		history(&mut state, channel, count);
		let mut view = TimelineView::default();
		frame(&ctx, &mut view, &mut state, 900.0)
			.into_iter()
			.map(|(text, _)| plain(&text))
			.collect::<Vec<_>>()
	};
	// #long-form (21) is a server text channel; 27 is a synthetic forum post.
	let channel = painted_text(Id(21), 3);
	assert!(channel.iter().any(|text| text == "Welcome to #long-form!"));
	assert!(
		channel
			.iter()
			.any(|text| text == "This is the start of the #long-form channel.")
	);
	let post = painted_text(Id(27), 3);
	assert!(post.iter().any(|text| text == "A synthetic forum post"));
	assert!(!post.iter().any(|text| text.starts_with("Welcome to #")));
	assert!(
		!post
			.iter()
			.any(|text| text.starts_with("This is the start of"))
	);
	// A full page may have older messages: no introduction yet.
	assert!(
		!painted_text(Id(21), 50)
			.iter()
			.any(|text| text == "Welcome to #long-form!")
	);
}

#[test]
fn direct_message_introduction_shows_a_known_username_only_when_it_differs() {
	let ctx = Context::default();
	let mut state = test_support::demo_state();
	dm_history(&mut state, 3);
	let recipient = state.channel(Id(22)).unwrap().recipients[0].clone();
	let mut view = TimelineView::default();
	let mut painted_with = |state: &mut State, username: &str| {
		state.apply(Envelope {
			generation: state.generation,
			event: Event::UserAction(client_core::user_actions::Event::Friends(Some(vec![(
				recipient.clone(),
				username.into(),
			)]))),
		});
		frame(&ctx, &mut view, state, 900.0)
			.into_iter()
			.map(|(text, _)| text)
			.collect::<Vec<_>>()
	};
	let name = recipient.name.clone();
	let named = |painted: &[String]| painted.iter().filter(|text| **text == name).count();
	let painted = painted_with(&mut state, "synthetic.handle");
	assert!(painted.iter().any(|text| text == "synthetic.handle"));
	let before = named(&painted);
	// A username equal to the heading would only repeat it.
	assert_eq!(named(&painted_with(&mut state, &name)), before);
}

/// Jumping to a conversation's first message starts the view at the top, like Discord:
/// the introduction is not pushed down by the padding that rests the live edge on the composer.
#[test]
fn a_window_opened_at_the_start_begins_at_the_top_instead_of_resting_on_the_composer() {
	let ctx = Context::default();
	let top = |targeted: bool| {
		let mut state = test_support::demo_state();
		dm_history(&mut state, 3);
		state.history_targeted = targeted;
		let mut view = TimelineView::default();
		frame(&ctx, &mut view, &mut state, 900.0)
			.into_iter()
			.find(|(text, _)| {
				text.starts_with("This is the beginning of your direct message history")
			})
			.map(|(_, rect)| rect.top())
			.expect("introduction painted")
	};
	let live = top(false);
	let jumped = top(true);
	assert!(jumped < 300.0, "the start begins at the top: {jumped}");
	assert!(
		live > jumped + 50.0,
		"the live edge still rests on the composer: {live}"
	);
}
