use crate::{MessagingUi, design, dialog, i18n};
use client_core::{Command, State};

impl MessagingUi {
	pub(super) fn mentions_inbox(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		if !self.inbox_open {
			return;
		}
		let mut load = None;
		let mut open = None;
		let response = dialog::Dialog::new("mentions-inbox", i18n::translate("inbox-title"))
			.subtitle(i18n::translate("inbox-description"))
			.width(620.0)
			.show(ctx, |body| {
				body.scroll(210.0, |ui| {
					let colors = design::palette(ui);
					if state.inbox.loading {
						ui.spinner();
						ui.label(i18n::translate("inbox-loading"));
					} else if let Some(error) = state.inbox.error {
						ui.colored_label(colors.danger, error);
					} else if state.inbox.messages.is_empty() {
						ui.label(i18n::translate(if state.inbox.loaded {
							"inbox-empty"
						} else {
							"inbox-refresh-needed"
						}));
					}
					for message in &state.inbox.messages {
						if !state.can_read_history(message.channel) {
							continue;
						}
						let Some(channel) = state.channel(message.channel) else {
							continue;
						};
						let place = channel
							.guild
							.and_then(|guild| state.guild(guild))
							.map_or_else(
								|| channel.name.clone(),
								|guild| format!("{} · #{}", guild.name, channel.name),
							);
						ui.label(design::semibold(ui, &place, 14.0).color(colors.text_strong));
						ui.weak(&message.author.name);
						let concealed = crate::embeds::has_spoilers(message);
						let mut preview: String = if concealed {
							"Spoiler".into()
						} else {
							message.content.chars().take(240).collect()
						};
						if !concealed && preview.len() < message.content.len() {
							preview.push('…');
						}
						ui.label(preview);
						if ui.button(i18n::translate("inbox-open-message")).clicked() {
							open = Some((channel.guild, channel.id, message.id));
						}
						ui.separator();
					}
				});
				body.footer(|ui| {
					let enabled = state.can_load_mentions() && !state.inbox.loading;
					if ui
						.add_enabled(
							enabled,
							egui::Button::new(i18n::translate(if state.inbox.error.is_some() {
								"inbox-retry"
							} else {
								"inbox-refresh"
							})),
						)
						.clicked()
					{
						load = Some(if state.inbox.error.is_some() {
							state.inbox.before
						} else {
							None
						});
					}
					if ui
						.add_enabled(
							enabled && state.inbox.next.is_some(),
							egui::Button::new(i18n::translate("inbox-older")),
						)
						.clicked()
					{
						load = Some(state.inbox.next);
					}
				});
			});
		if let Some((guild, channel, message)) = open {
			match state.open_chat_link(guild, channel, Some(message)) {
				Ok(command) => {
					commands.extend(command);
					self.guild = guild;
					self.search.open = false;
					self.inbox_open = false;
				}
				Err(error) => state.status = error,
			}
		}
		if response.close || !self.inbox_open {
			self.inbox_open = false;
			commands.push(state.clear_mentions());
		} else if let Some(before) = load
			&& let Some(command) = state.request_mentions(before)
		{
			commands.push(command);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::Id;

	#[test]
	fn mentions_pages_bound_cursors_reject_stale_results_and_clear_on_close() {
		let mut state = test_support::demo_state();
		let Command::Mentions {
			request,
			before: None,
		} = state.request_mentions(None).unwrap()
		else {
			panic!()
		};
		let page = || {
			(76..=100)
				.rev()
				.map(|id| test_support::message(id, Id(20)))
				.collect()
		};
		state.apply_mentions(request + 1, Ok(page()));
		assert!(state.inbox.loading && state.inbox.messages.is_empty());
		state.apply_mentions(request, Ok(page()));
		assert_eq!(state.inbox.messages.len(), 25);
		assert_eq!(state.inbox.next, Some(Id(76)));
		assert!(state.request_mentions(Some(Id(999))).is_none());
		let Command::Mentions {
			request,
			before: Some(Id(76)),
		} = state.request_mentions(Some(Id(76))).unwrap()
		else {
			panic!()
		};
		state.apply_mentions(request, Ok(vec![test_support::message(100, Id(20))]));
		assert!(state.inbox.error.is_some());
		let Command::Mentions { request, .. } = state.request_mentions(Some(Id(76))).unwrap()
		else {
			panic!()
		};
		state.apply_mentions(request, Err(client_core::auth::Failure::Network));
		assert!(!state.inbox.loading && state.inbox.error.is_some());
		let Command::Mentions { request, .. } = state.request_mentions(None).unwrap() else {
			panic!()
		};
		state.apply_mentions(
			request,
			Ok((1..=26)
				.rev()
				.map(|id| test_support::message(id, Id(20)))
				.collect()),
		);
		assert!(state.inbox.error.is_some() && state.inbox.messages.is_empty());
		let Command::Mentions { request, .. } = state.request_mentions(None).unwrap() else {
			panic!()
		};
		state.clear_mentions();
		state.apply_mentions(request, Ok(page()));
		assert!(state.inbox.messages.is_empty() && !state.inbox.loading);
		let Command::Mentions { request, .. } = state.request_mentions(None).unwrap() else {
			panic!()
		};
		state.apply_mentions(request, Ok(page()));
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Delete {
				channel: Id(20),
				id: Id(999),
			},
		});
		assert_eq!(
			state.inbox.messages.len(),
			25,
			"unrelated messages must not erase the page"
		);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Delete {
				channel: Id(20),
				id: Id(100),
			},
		});
		assert!(state.inbox.messages.is_empty() && !state.inbox.loaded);
	}

	#[test]
	fn mentions_dialog_opens_target_message_and_escape_clears_its_page() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut state = test_support::demo_state();
		let mut message = test_support::message(80, Id(20));
		message.content = "Visible ||concealed private spoiler||".into();
		state.inbox.messages = vec![message];
		state.inbox.loaded = true;
		let mut view = MessagingUi {
			inbox_open: true,
			..Default::default()
		};
		let frame = |view: &mut MessagingUi, state: &mut State, events| {
			let mut commands = vec![];
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|_| view.mentions_inbox(&ctx, state, &mut commands),
			);
			assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains("concealed private spoiler"))));
			let open = output.shapes.iter().find_map(|shape| match &shape.shape {
				egui::Shape::Text(text) if text.galley.job.text == "Open message" => {
					Some(text.galley.rect.translate(text.pos.to_vec2()).center())
				}
				_ => None,
			});
			output.drop_without_applying_deltas();
			(open, commands)
		};
		frame(&mut view, &mut state, vec![]);
		let point = frame(&mut view, &mut state, vec![]).0.unwrap();
		for pressed in [true, false] {
			frame(
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(point),
					egui::Event::PointerButton {
						pos: point,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
		assert!(!view.inbox_open && state.inbox.messages.is_empty());
		assert_eq!(state.selected, Some(Id(20)));
		assert_eq!(state.search_target, Some(Id(80)));
		view.inbox_open = true;
		state.inbox.loaded = true;
		frame(&mut view, &mut state, vec![]);
		frame(
			&mut view,
			&mut state,
			vec![egui::Event::Key {
				key: egui::Key::Escape,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			}],
		);
		assert!(!view.inbox_open && !state.inbox.loaded);
	}
}
