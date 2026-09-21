//! Offline debug check: cargo run --locked -p ui --example voice_chat
use client_core::{Command, Envelope, Event, State};

fn frame(
	ctx: &egui::Context,
	view: &mut ui::MessagingUi,
	state: &mut State,
	width: f32,
	events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
	let output = ctx.run_ui(
		egui::RawInput {
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(width, 760.0),
			)),
			events,
			..Default::default()
		},
		|ui| {
			// Commands are inspected locally, never dispatched to a service.
			let commands = view.show(ui, state);
			assert!(!commands.iter().any(|c| matches!(c, Command::Voice(_))));
		},
	);
	let mut labels: Vec<_> = output
		.shapes
		.iter()
		.filter_map(|shape| {
			if let egui::Shape::Text(text) = &shape.shape {
				Some((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				))
			} else {
				None
			}
		})
		.collect();
	if let Some(response) = ctx
		.memory(|memory| memory.focused())
		.and_then(|id| ctx.read_response(id))
	{
		for event in &output.platform_output.events {
			if let Some(label) = &event.widget_info().label {
				labels.push((label.clone(), response.rect));
			}
		}
	}
	output.drop_without_applying_deltas();
	labels
}

fn main() {
	for (width, right_click) in [
		(1400.0, false),
		(800.0, false),
		(1400.0, true),
		(800.0, true),
	] {
		let mut state = test_support::voice_demo_state();
		let channel = state.selected.unwrap();
		assert!(state.channel(channel).unwrap().supports_text());
		assert!(
			matches!(state.history(None), Command::History { channel: id, .. } if id == channel)
		);
		let mut message = test_support::message(600, channel);
		message.content = "Synthetic voice chat message".into();
		message.attachments.clear();
		message.embeds.clear();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::History {
				channel,
				request: state.request,
				older: false,
				messages: vec![message],
			},
		});
		assert!(state.can_compose(channel));
		let ctx = egui::Context::default();
		ui::design::apply(&ctx);
		let mut view = ui::MessagingUi::default();
		for (label, open) in [("Show chat", true), ("Hide chat", false)] {
			let mut labels = vec![];
			for _ in 0..100 {
				labels = frame(
					&ctx,
					&mut view,
					&mut state,
					width,
					vec![egui::Event::Key {
						key: egui::Key::Tab,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
				);
				if labels.iter().any(|(text, _)| text == label) {
					break;
				}
			}
			let pos = labels
				.iter()
				.find(|(text, _)| text == label)
				.unwrap()
				.1
				.center();
			for pressed in [true, false] {
				frame(
					&ctx,
					&mut view,
					&mut state,
					width,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(view.voice_chat_open, open);
			for _ in 0..3 {
				labels = frame(&ctx, &mut view, &mut state, width, vec![]);
			}
			assert_eq!(
				labels
					.iter()
					.any(|(text, _)| text == "Synthetic voice chat message"),
				open
			);
			assert_eq!(state.voice.active.as_ref().unwrap().channel, channel);
		}
		state.voice.active.as_mut().unwrap().watching = Some(model::Id(7));
		if right_click {
			view.voice_stream_view = Some(ctx.load_texture(
				"synthetic stream",
				egui::ColorImage::filled([320, 180], egui::Color32::GRAY),
				egui::TextureOptions::LINEAR,
			));
		}
		assert_eq!(view.voice_stream_volume(), 100);
		for expanded in [true, false] {
			let mut labels = vec![];
			for _ in 0..3 {
				labels = frame(&ctx, &mut view, &mut state, width, vec![]);
			}
			let pos = labels
				.iter()
				.find(|(text, _)| text == "Stream audio")
				.unwrap()
				.1
				.center() + egui::vec2(0.0, 40.0);
			for pressed in [true, false] {
				frame(
					&ctx,
					&mut view,
					&mut state,
					width,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(
				view.voice_focus.is_some(),
				expanded,
				"left-click must toggle stream expansion"
			);
		}
		for (label, expected_volume) in [
			("Stream audio", 100),
			("Mute stream audio", 0),
			("Mute stream audio", 100),
		] {
			let mut labels = vec![];
			for _ in 0..3 {
				labels = frame(&ctx, &mut view, &mut state, width, vec![]);
			}
			let mut pos = labels
				.iter()
				.find(|(text, _)| text == label)
				.unwrap()
				.1
				.center();
			let button = if right_click && label == "Stream audio" {
				pos.y += 40.0;
				egui::PointerButton::Secondary
			} else {
				egui::PointerButton::Primary
			};
			for pressed in [true, false] {
				frame(
					&ctx,
					&mut view,
					&mut state,
					width,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(view.voice_stream_volume(), expected_volume);
			assert!(
				view.voice_focus.is_none(),
				"audio controls must not expand the stream"
			);
			assert!(
				view.voice_user_volumes()
					.iter()
					.all(|(_, gain)| *gain == 100)
			);
		}
		state.voice.active = None;
		assert!(
			state.can_compose(channel),
			"chat must not require joining voice"
		);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Unavailable(channel),
		});
		assert!(!state.can_compose(channel) && !state.can_read_history(channel));
	}
	println!(
		"PASS: voice chat history, wide/narrow stream audio button and right-click menus, stream mute, independent user volume, permission gates (offline egui)."
	);
}
