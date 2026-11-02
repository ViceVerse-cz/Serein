use super::*;
use crate::avatars::Avatars;
use egui::{Pos2, Rect, Vec2};

fn fixture() -> (State, Message) {
	let mut state = test_support::demo_state();
	state.auth = client_core::auth::AuthState::Authenticated;
	state.gateway_connected = true;
	state.freshness = model::Freshness::Fresh;
	let mut message = test_support::message(99101, state.selected.unwrap());
	message.author = state.user.clone().unwrap();
	message.content.clear();
	message.poll = Some(Box::new(
		Create {
			question: "What should we play tonight?".into(),
			answers: ["Minecraft", "Stardew Valley", "Terraria"]
				.into_iter()
				.map(|text| Media {
					text: text.into(),
					emoji: None,
				})
				.collect(),
			duration: 24,
			multiselect: false,
		}
		.fixture(now_ms()),
	));
	message.extra_content.poll = true;
	state.timeline.clear();
	state.timeline.insert(message.clone(), true, false).unwrap();
	assert!(state.can_vote_poll(&message));
	(state, message)
}

fn context(light: bool) -> egui::Context {
	crate::i18n::set_current(crate::i18n::Language::English);
	let ctx = egui::Context::default();
	design::apply(&ctx);
	ctx.set_visuals(if light {
		egui::Visuals::light()
	} else {
		egui::Visuals::dark()
	});
	ctx
}

fn input(size: Vec2, events: Vec<egui::Event>) -> egui::RawInput {
	egui::RawInput {
		screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
		events,
		..Default::default()
	}
}

fn click(pos: Pos2) -> Vec<egui::Event> {
	vec![
		egui::Event::PointerMoved(pos),
		egui::Event::PointerButton {
			pos,
			button: egui::PointerButton::Primary,
			pressed: true,
			modifiers: egui::Modifiers::NONE,
		},
		egui::Event::PointerButton {
			pos,
			button: egui::PointerButton::Primary,
			pressed: false,
			modifiers: egui::Modifiers::NONE,
		},
	]
}

fn labels(shape: &egui::Shape, clip: Rect, output: &mut Vec<(String, Rect)>) {
	match shape {
		egui::Shape::Text(text) => {
			let rect = text.visual_bounding_rect();
			if clip.intersects(rect) {
				output.push((text.galley.job.text.clone(), rect));
			}
		}
		egui::Shape::Vec(shapes) => {
			for shape in shapes {
				labels(shape, clip, output);
			}
		}
		_ => {}
	}
}

fn card_frame(
	ctx: &egui::Context,
	cards: &mut Cards,
	state: &State,
	message: &Message,
	width: f32,
	events: Vec<egui::Event>,
) -> (Option<Action>, Rect, Vec<(String, Rect)>) {
	let mut action = None;
	let mut bounds = Rect::NOTHING;
	let output = ctx.run_ui(input(Vec2::new(width, 3000.0), events), |ui| {
		ui.set_width(width - 16.0);
		let mut surface = crate::select::Surface::new(ui, "poll-row");
		let shown = ui.scope(|ui| cards.show(ui, state, message, &mut Avatars::default()));
		surface.exclude(shown.response.rect);
		action = shown.inner;
		bounds = ui.min_rect();
		surface.cover(bounds);
		surface.finish(ui);
	});
	let mut text = Vec::new();
	for shape in &output.shapes {
		labels(&shape.shape, shape.clip_rect, &mut text);
	}
	output.drop_without_applying_deltas();
	(action, bounds, text)
}

fn position(text: &[(String, Rect)], target: &str) -> Pos2 {
	text.iter()
		.find(|(label, _)| label == target)
		.unwrap_or_else(|| panic!("missing rendered label {target}"))
		.1
		.center()
}

#[test]
fn selection_preview_and_pending_cards_preserve_vote_actions() {
	let (mut state, mut message) = fixture();
	let ctx = context(false);
	let mut cards = Cards::default();
	let (_, _, text) = card_frame(&ctx, &mut cards, &state, &message, 500.0, vec![]);
	let (action, _, _) = card_frame(
		&ctx,
		&mut cards,
		&state,
		&message,
		500.0,
		click(position(&text, "Minecraft")),
	);
	assert!(action.is_none());
	assert_eq!(cards.selection.as_ref().unwrap().2, vec![1]);
	let (_, _, text) = card_frame(&ctx, &mut cards, &state, &message, 500.0, vec![]);
	card_frame(
		&ctx,
		&mut cards,
		&state,
		&message,
		500.0,
		click(position(&text, "Stardew Valley")),
	);
	assert_eq!(cards.selection.as_ref().unwrap().2, vec![2]);
	let (_, _, text) = card_frame(&ctx, &mut cards, &state, &message, 500.0, vec![]);
	let (action, _, _) = card_frame(
		&ctx,
		&mut cards,
		&state,
		&message,
		500.0,
		click(position(&text, "Show results")),
	);
	assert!(matches!(action, Some(Action::Read)));
	let (_, _, text) = card_frame(&ctx, &mut cards, &state, &message, 500.0, vec![]);
	let (action, _, _) = card_frame(
		&ctx,
		&mut cards,
		&state,
		&message,
		500.0,
		click(position(&text, "Back to voting")),
	);
	assert!(action.is_none());
	assert_eq!(cards.selection.as_ref().unwrap().2, vec![2]);
	state.polls.pending = Some((message.channel, Some(message.id), 1));
	let (_, _, text) = card_frame(&ctx, &mut cards, &state, &message, 500.0, vec![]);
	let (action, _, _) = card_frame(
		&ctx,
		&mut cards,
		&state,
		&message,
		500.0,
		click(position(&text, "Vote")),
	);
	assert!(action.is_none());
	state.polls.pending = None;
	let (_, _, text) = card_frame(&ctx, &mut cards, &state, &message, 500.0, vec![]);
	let (action, _, _) = card_frame(
		&ctx,
		&mut cards,
		&state,
		&message,
		500.0,
		click(position(&text, "Vote")),
	);
	assert!(matches!(action, Some(Action::Vote(ids)) if ids == vec![2]));
	message.poll.as_mut().unwrap().answers[1].me = true;
	let (_, _, text) = card_frame(&ctx, &mut cards, &state, &message, 500.0, vec![]);
	let (action, _, _) = card_frame(
		&ctx,
		&mut cards,
		&state,
		&message,
		500.0,
		click(position(&text, "Remove Vote")),
	);
	assert!(matches!(action, Some(Action::Vote(ids)) if ids.is_empty()));
	let selection = cards.selection.as_ref().unwrap();
	assert!(selection.2.is_empty() && !selection.3);
}
