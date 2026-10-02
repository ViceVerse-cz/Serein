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

fn artwork_bounds(shape: &egui::Shape, texture: egui::TextureId, output: &mut Vec<Rect>) {
	match shape {
		egui::Shape::Rect(rect)
			if rect
				.brush
				.as_ref()
				.is_some_and(|brush| brush.fill_texture_id == texture) =>
		{
			output.push(rect.rect)
		}
		egui::Shape::Mesh(mesh) if mesh.texture_id == texture => {
			output.push(shape.visual_bounding_rect())
		}
		egui::Shape::Vec(shapes) => {
			for shape in shapes {
				artwork_bounds(shape, texture, output);
			}
		}
		_ => {}
	}
}

fn position(text: &[(String, Rect)], target: &str) -> Pos2 {
	text.iter()
		.find(|(label, _)| label == target)
		.unwrap_or_else(|| panic!("missing rendered label {target}"))
		.1
		.center()
}

#[test]
fn maximum_poll_content_stays_inside_narrow_cards_in_each_state() {
	let (state, mut message) = fixture();
	let poll = message.poll.as_mut().unwrap();
	poll.question = "A long question to wrap safely "
		.chars()
		.cycle()
		.take(300)
		.collect();
	assert_eq!(poll.question.chars().count(), 300);
	poll.answers = (1..=10)
		.map(|id| model::polls::Answer {
			id,
			media: Media {
				text: "M".repeat(55),
				emoji: Some(model::ReactionEmoji {
					id: None,
					name: Some("🎮".into()),
				}),
			},
			votes: u32::MAX,
			me: false,
		})
		.collect();
	assert!(poll.valid());
	for width in [260.0, 900.0] {
		for light in [false, true] {
			for mode in 0..5 {
				let ctx = context(light);
				let mut cards = Cards::default();
				let poll = message.poll.as_mut().unwrap();
				poll.answers[0].me = mode == 2;
				poll.results_known = mode != 3;
				poll.finalized = mode == 4;
				if mode == 1 || mode == 3 {
					cards.selection = Some((message.channel, message.id, vec![1], mode == 3));
				}
				let (_, bounds, text) =
					card_frame(&ctx, &mut cards, &state, &message, width, vec![]);
				assert!(
					bounds.right() <= width + 1.0,
					"card exceeds {width}: {bounds:?}"
				);
				for (label, rect) in &text {
					assert!(
						rect.left() >= bounds.left() - 1.0 && rect.right() <= bounds.right() + 1.0,
						"{label:?} leaves card bounds: {rect:?}, {bounds:?}"
					);
				}
				if mode == 3 {
					assert!(!text.iter().any(|(label, _)| label.ends_with('%')));
					assert!(
						text.iter()
							.any(|(label, _)| label.starts_with("Results not loaded"))
					);
				} else if mode >= 2 {
					assert!(text.iter().any(|(label, _)| label == "10%"));
				}
			}
		}
	}
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

#[test]
fn answer_rows_accept_keyboard_activation_and_results_are_read_only() {
	let (_, message) = fixture();
	let answer = &message.poll.as_ref().unwrap().answers[0];
	let ctx = context(false);
	ctx.run_ui(input(Vec2::new(300.0, 200.0), vec![]), |ui| {
		answer_row(ui, answer, None, false, false, true, 0).request_focus();
	})
	.drop_without_applying_deltas();
	let enter = || {
		vec![egui::Event::Key {
			key: egui::Key::Enter,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		}]
	};
	ctx.run_ui(input(Vec2::new(300.0, 200.0), enter()), |ui| {
		assert!(answer_row(ui, answer, None, false, false, true, 0).clicked());
	})
	.drop_without_applying_deltas();
	ctx.run_ui(input(Vec2::new(300.0, 200.0), enter()), |ui| {
		assert!(!answer_row(ui, answer, None, false, true, true, 0).clicked());
	})
	.drop_without_applying_deltas();
}

#[test]
fn bundled_emoji_artwork_fits_wrapped_answer_rows() {
	let (_, message) = fixture();
	let mut answer = message.poll.as_ref().unwrap().answers[0].clone();
	answer.media.text = "M".repeat(55);
	answer.media.emoji = Some(model::ReactionEmoji {
		id: None,
		name: Some("🎮".into()),
	});
	let ctx = context(false);
	crate::emoji::install(&ctx).unwrap();
	assert!(crate::emoji::image(&ctx, "🎮", 24.0).is_some());
	let mut row = Rect::NOTHING;
	let output = ctx.run_ui(input(Vec2::new(260.0, 900.0), vec![]), |ui| {
		ui.set_width(220.0);
		let image = emoji_image(
			ui.ctx(),
			&mut Avatars::default(),
			answer.media.emoji.as_ref().unwrap(),
			24.0,
			true,
		);
		row = answer_row(ui, &answer, image, true, true, true, 0).rect;
	});
	let mut artwork = Vec::new();
	for shape in &output.shapes {
		artwork_bounds(
			&shape.shape,
			crate::emoji::atlas(&ctx).unwrap().id,
			&mut artwork,
		);
	}
	output.drop_without_applying_deltas();
	assert!(!artwork.is_empty(), "bundled emoji must paint an image");
	for rect in artwork {
		assert!(
			row.contains_rect(rect),
			"emoji leaves answer bounds: {rect:?}, {row:?}"
		);
		assert_eq!(rect.size(), Vec2::splat(24.0));
	}
}

#[test]
fn maximum_creator_draft_fits_short_narrow_windows_and_closes_on_navigation() {
	let (mut state, _) = fixture();
	for size in [Vec2::new(260.0, 360.0), Vec2::new(900.0, 900.0)] {
		for light in [false, true] {
			let ctx = context(light);
			let mut creator = Creator::default();
			creator.open(&state, state.selected.unwrap());
			let draft = &mut creator.draft.as_mut().unwrap().poll;
			draft.question = "Question ".repeat(33);
			draft.answers = vec![
				Media {
					text: "M".repeat(55),
					emoji: Some(model::ReactionEmoji {
						id: None,
						name: Some("🎮".into())
					})
				};
				10
			];
			assert!(draft.valid());
			// Modal areas settle their position after their first sizing frame.
			let mut text = Vec::new();
			for _ in 0..3 {
				let output = ctx.run_ui(input(size, vec![]), |ui| {
					assert!(
						creator
							.show(ui.ctx(), &mut state, &mut Avatars::default())
							.is_none()
					);
				});
				text.clear();
				for shape in &output.shapes {
					labels(&shape.shape, shape.clip_rect, &mut text);
				}
				output.drop_without_applying_deltas();
			}
			let rect = ctx
				.memory(|memory| memory.area_rect(egui::Id::unique("poll-create")))
				.unwrap();
			assert!(
				rect.left() >= -1.0
					&& rect.right() <= size.x + 1.0
					&& rect.top() >= -1.0
					&& rect.bottom() <= size.y + 1.0,
				"creator exceeds {size:?}: {rect:?}"
			);
			// The footer keeps both controls inside the dialog, even when the checkbox wraps.
			for label in ["Post", "Allow Multiple Answers"] {
				let (_, bounds) = text
					.iter()
					.find(|(text, _)| text == label)
					.unwrap_or_else(|| panic!("missing footer label {label}"));
				assert!(
					rect.expand(1.0).contains_rect(*bounds),
					"{label} leaves the creator at {size:?}: {bounds:?}, {rect:?}"
				);
			}
		}
	}
	let ctx = context(false);
	let mut creator = Creator::default();
	creator.open(&state, state.selected.unwrap());
	state.selected = None;
	ctx.run_ui(input(Vec2::new(500.0, 500.0), vec![]), |ui| {
		assert!(
			creator
				.show(ui.ctx(), &mut state, &mut Avatars::default())
				.is_none()
		);
	})
	.drop_without_applying_deltas();
	assert!(creator.draft.is_none());
}

#[test]
fn answer_emoji_comes_from_the_picker_as_twemoji_and_can_be_cleared() {
	let (mut state, _) = fixture();
	let ctx = context(false);
	crate::emoji::install(&ctx).unwrap();
	let atlas = crate::emoji::atlas(&ctx).unwrap().id;
	let mut creator = Creator::default();
	let mut avatars = Avatars::default();
	creator.open(&state, state.selected.unwrap());
	let key = |key| egui::Event::Key {
		key,
		physical_key: None,
		pressed: true,
		repeat: false,
		modifiers: egui::Modifiers::NONE,
	};
	let mut frame = |creator: &mut Creator, state: &mut State, events| {
		let output = ctx.run_ui(input(Vec2::new(900.0, 900.0), events), |ui| {
			assert!(creator.show(ui.ctx(), state, &mut avatars).is_none());
		});
		let (mut text, mut art) = (Vec::new(), Vec::new());
		for shape in &output.shapes {
			labels(&shape.shape, shape.clip_rect, &mut text);
			artwork_bounds(&shape.shape, atlas, &mut art);
		}
		output.drop_without_applying_deltas();
		let focused = ctx
			.memory(|m| m.focused())
			.and_then(|id| ctx.read_response(id))
			.map(|r| r.rect.size());
		(text, art, focused)
	};
	// Tab through the popout and activate the first focused control of `size`.
	macro_rules! choose {
		($size:expr) => {{
			let mut chosen = false;
			for _ in 0..16 {
				let (_, _, focused) = frame(&mut creator, &mut state, vec![key(egui::Key::Tab)]);
				if focused == Some(Vec2::splat($size)) {
					frame(&mut creator, &mut state, vec![key(egui::Key::Enter)]);
					chosen = true;
					break;
				}
			}
			assert!(chosen, "no focusable {}px picker control", $size);
		}};
	}
	for _ in 0..3 {
		frame(&mut creator, &mut state, vec![]);
	}
	let (text, art, _) = frame(&mut creator, &mut state, vec![]);
	assert!(art.is_empty(), "empty answers show the smiley glyph");
	// The emoji button sits inside the field, just left of the answer text.
	let hint = text
		.iter()
		// Fluent wraps placeables in invisible bidi isolation marks.
		.find(|(label, _)| label.replace(['\u{2068}', '\u{2069}'], "") == "Answer 1")
		.unwrap()
		.1;
	let button = egui::pos2(hint.left() - 30.0, hint.center().y);
	frame(&mut creator, &mut state, click(button));
	assert!(creator.picker.choosing());
	assert_eq!(creator.draft.as_ref().unwrap().picking, Some(0));
	// Above the dialog the popout becomes focusable a frame after opening; typing then
	// lands in its search field.
	let (_, _, focused) = frame(&mut creator, &mut state, vec![]);
	assert!(focused.is_some(), "the picker search takes focus");
	frame(
		&mut creator,
		&mut state,
		vec![egui::Event::Text("rocket".into())],
	);
	choose!(40.0);
	let draft = creator.draft.as_ref().unwrap();
	assert_eq!(
		draft.poll.answers[0].emoji,
		Some(model::ReactionEmoji {
			id: None,
			name: Some("🚀".into())
		})
	);
	assert!(draft.poll.answers[1].emoji.is_none());
	assert!(!creator.picker.choosing() && draft.picking.is_none());
	let (_, art, _) = frame(&mut creator, &mut state, vec![]);
	assert_eq!(art.len(), 1, "the chosen emoji paints bundled Twemoji");
	assert_eq!(art[0].size(), Vec2::splat(22.0));
	assert!((art[0].center().x - button.x).abs() < 16.0);

	// Reopening offers removal, which clears only this answer's emoji.
	frame(&mut creator, &mut state, click(button));
	assert!(creator.picker.choosing());
	choose!(30.0);
	assert!(
		creator.draft.as_ref().unwrap().poll.answers[0]
			.emoji
			.is_none()
	);
	assert!(!creator.picker.choosing());
	let (_, art, _) = frame(&mut creator, &mut state, vec![]);
	assert!(art.is_empty());
}

#[test]
fn poll_check_restores_window_effects_after_unwind() {
	let _restore = WindowEffectsRestore(design::default_window_effects());
	let original = (true, 37, 63);
	design::set_window_effects(original.0, original.1, original.2);
	let result = std::panic::catch_unwind(|| {
		let _restore = WindowEffectsRestore(design::default_window_effects());
		design::set_window_effects(true, 100, 0);
		panic!("synthetic failed check");
	});
	assert!(result.is_err());
	assert_eq!(design::default_window_effects(), original);
}

#[cfg(feature = "demo")]
#[test]
fn poll_check_restores_callers_window_effects() {
	let _restore = WindowEffectsRestore(design::default_window_effects());
	let (state, _) = fixture();
	for original in [(true, 37, 63), (false, 15, 50)] {
		design::set_window_effects(original.0, original.1, original.2);
		debug_poll_check(&state);
		assert_eq!(design::default_window_effects(), original);
	}
}
