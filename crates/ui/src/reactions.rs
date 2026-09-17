use model::{Reaction, ReactionEmoji};

#[derive(Debug, PartialEq)]
pub enum Action {
	Reload,
	Toggle(ReactionEmoji),
	Inspect(ReactionEmoji, bool),
}

#[allow(clippy::too_many_arguments)]
pub fn show(
	ui: &mut egui::Ui,
	reactions: Option<&[Reaction]>,
	enabled: bool,
	writing: bool,
	refreshing: bool,
	media: (&mut crate::avatars::Avatars, bool),
	message: model::Id,
	details: Option<&client_core::reactions::ReactionUsers>,
	can_react: impl Fn(&ReactionEmoji, bool) -> bool,
) -> Option<Action> {
	// Unknown cached counts are not a failure while history/reactions are loading.
	// Allocate no placeholder row, so reaction-free messages do not jump in height.
	if reactions.is_some_and(<[Reaction]>::is_empty) || (reactions.is_none() && refreshing) {
		return None;
	}
	let mut action = None;
	ui.horizontal_wrapped(|ui| {
		ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
		ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
		ui.spacing_mut().interact_size.y = 26.0;
		let Some(reactions) = reactions else {
			ui.weak("Reactions unavailable");
			if ui
				.add_enabled(enabled, egui::Button::new("Reload reactions").small())
				.clicked()
			{
				action = Some(Action::Reload);
			}
			return;
		};
		if writing {
			ui.visuals_mut().disabled_alpha = 1.0;
		}
		for reaction in reactions {
			let label = format!("{} {}", reaction.emoji.label(), reaction.count);
			let button = if reaction.emoji.id.is_none() {
				crate::emoji::button(
					ui.ctx(),
					&reaction.emoji.label(),
					reaction.count.to_string(),
				)
			} else if let Some(image) = reaction
				.emoji
				.id
				.and_then(|id| media.0.custom_image(ui.ctx(), id, 18.0, media.1))
			{
				egui::Button::image_and_text(
					image.alt_text(reaction.emoji.label()),
					reaction.count.to_string(),
				)
				.image_tint_follows_text_color(false)
			} else {
				egui::Button::new(label.clone())
			};
			let response = ui.add_enabled(
				!writing
					&& reaction.emoji.name.is_some()
					&& can_react(&reaction.emoji, !reaction.me),
				button
					.gap(4.0)
					.min_size(egui::vec2(0.0, 26.0))
					.corner_radius(6)
					.selected(reaction.me),
			);
			response.widget_info(|| {
				egui::WidgetInfo::selected(
					egui::WidgetType::Button,
					response.enabled(),
					reaction.me,
					&label,
				)
			});
			let matching = details
				.filter(|value| value.message == message && value.emoji.same(&reaction.emoji));
			if response.hovered() && matching.is_none() && action.is_none() {
				action = Some(Action::Inspect(reaction.emoji.clone(), false));
			}
			let clicked = response.clicked();
			let secondary_clicked = response.secondary_clicked();
			response.on_hover_ui(|ui| {
				if let Some(value) = matching
					&& !value.users.is_empty()
				{
					let names = value
						.users
						.iter()
						.take(3)
						.map(|user| user.name.as_str())
						.collect::<Vec<_>>()
						.join(", ");
					let remaining = reaction.count.saturating_sub(value.users.len() as u32);
					ui.label(if remaining == 0 {
						names
					} else {
						format!(
							"{names}, and {remaining} other{}",
							if remaining == 1 { "" } else { "s" }
						)
					});
				} else if matching.is_some_and(|value| value.error.is_some()) {
					ui.label("Reaction details unavailable");
				} else {
					ui.label("Loading reactions…");
				}
			});
			if secondary_clicked {
				action = Some(Action::Inspect(reaction.emoji.clone(), true));
			} else if clicked {
				action = Some(Action::Toggle(reaction.emoji.clone()));
			}
		}
	});
	action
}

pub fn add_button(
	ui: &mut egui::Ui,
	enabled: bool,
	writing: bool,
) -> Option<(egui::Rect, egui::Id)> {
	let response = ui
		.add_enabled_ui(enabled && !writing, |ui| {
			crate::icons::button(ui, crate::icons::Icon::Smile, 28.0, "Add reaction")
		})
		.inner;
	response.clicked().then_some((response.rect, response.id))
}

pub fn show_users(
	ctx: &egui::Context,
	state: &mut client_core::State,
	avatars: &mut crate::avatars::Avatars,
	commands: &mut Vec<client_core::Command>,
) {
	let Some(details) = state
		.reactions
		.users
		.as_ref()
		.filter(|details| details.open)
	else {
		return;
	};
	let reactions = state
		.timeline
		.get(details.message)
		.and_then(|message| message.reactions.clone())
		.unwrap_or_default();
	let mut select = None;
	let mut more = false;
	let mut close = false;
	let response = crate::dialog::Dialog::new("reaction-users", "Reactions")
		.width(560.0)
		.show(ctx, |dialog| {
			dialog.content(|ui| {
				ui.horizontal_wrapped(|ui| {
					for reaction in &reactions {
						let selected = details.emoji.same(&reaction.emoji);
						if ui
							.add(
								egui::Button::new(format!(
									"{}  {}",
									reaction.emoji.label(),
									reaction.count
								))
								.selected(selected),
							)
							.clicked() && !selected
						{
							select = Some(reaction.emoji.clone());
						}
					}
				});
			});
			dialog.scroll(190.0, |ui| {
				if details.users.is_empty() && details.loading {
					ui.horizontal(|ui| {
						ui.spinner();
						ui.label("Loading reactions…");
					});
				} else if details.users.is_empty() {
					crate::dialog::hint(
						ui,
						details
							.error
							.unwrap_or("Nobody currently has this reaction."),
					);
				}
				for user in &details.users {
					ui.horizontal(|ui| {
						avatars.show_plain(ui, user, 36.0, state.demo);
						ui.label(crate::design::medium(ui, &user.name, 15.0));
					});
				}
				if details.users.len() >= client_core::reactions::MAX_REACTION_USERS {
					crate::dialog::hint(ui, "Showing the first 1,000 reactions.");
				} else if let Some(error) = details.error {
					crate::dialog::notice(ui, crate::dialog::Level::Warning, error);
					if crate::dialog::action(ui, "Retry", crate::dialog::Action::Neutral).clicked()
					{
						more = true;
					}
				} else if !details.exhausted {
					if details.loading {
						ui.spinner();
					} else if crate::dialog::action(ui, "Load more", crate::dialog::Action::Neutral)
						.clicked()
					{
						more = true;
					}
				}
			});
			dialog.footer(|ui| {
				close =
					crate::dialog::action(ui, "Close", crate::dialog::Action::Primary).clicked();
			});
		});
	close |= response.close;
	if close {
		state.close_reaction_users();
	} else if let Some(emoji) = select {
		if let Some(command) = state.request_reaction_users(details.message, emoji, true) {
			commands.push(command);
		}
	} else if more && let Some(command) = state.next_reaction_users_page() {
		commands.push(command);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn empty_reactions_do_not_allocate_a_row() {
		let ctx = egui::Context::default();
		let output = ctx.run_ui(egui::RawInput::default(), |ui| {
			let before = ui.min_rect();
			assert_eq!(
				show(
					ui,
					Some(&[]),
					true,
					false,
					false,
					(&mut crate::avatars::Avatars::default(), true),
					model::Id(1),
					None,
					|_, _| true,
				),
				None
			);
			assert_eq!(ui.min_rect(), before);
		});
		output.drop_without_applying_deltas();
	}

	#[test]
	fn hovering_starts_reaction_user_load_before_the_tooltip_opens() {
		let ctx = egui::Context::default();
		crate::emoji::install(&ctx).unwrap();
		let emoji = ReactionEmoji {
			id: None,
			name: Some("👍".into()),
		};
		let mut action = None;
		for frame in 0..2 {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(240.0, 180.0),
					)),
					events: (frame == 0)
						.then(|| egui::Event::PointerMoved(egui::pos2(20.0, 15.0)))
						.into_iter()
						.collect(),
					..Default::default()
				},
				|ui| {
					action = show(
						ui,
						Some(&[Reaction {
							emoji: emoji.clone(),
							count: 3,
							me: false,
							me_burst: false,
						}]),
						true,
						false,
						false,
						(&mut crate::avatars::Avatars::default(), true),
						model::Id(1),
						None,
						|_, _| true,
					);
				},
			);
			output.drop_without_applying_deltas();
		}
		assert_eq!(action, Some(Action::Inspect(emoji, false)));
	}

	#[test]
	fn keyboard_reaction_toggle_and_disabled_refresh_emit_only_local_actions() {
		let values = vec![Reaction {
			emoji: ReactionEmoji {
				id: None,
				name: Some("👍".into()),
			},
			count: 3,
			me: true,
			me_burst: false,
		}];
		for (enabled, toggle) in [(true, true), (false, true), (true, false), (false, false)] {
			let ctx = egui::Context::default();
			crate::emoji::install(&ctx).unwrap();
			let mut action = None;
			for key in [None, Some(egui::Key::Tab), Some(egui::Key::Enter)] {
				let input = egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(240.0, 180.0),
					)),
					events: key
						.map(|key| {
							vec![egui::Event::Key {
								key,
								physical_key: None,
								pressed: true,
								repeat: false,
								modifiers: egui::Modifiers::NONE,
							}]
						})
						.unwrap_or_default(),
					..Default::default()
				};
				let mut output = ctx.run_ui(input, |ui| {
					action = show(
						ui,
						Some(&values),
						enabled,
						false,
						false,
						(&mut crate::avatars::Avatars::default(), true),
						model::Id(1),
						None,
						|_, add| {
							assert!(!add, "The owned reaction is removed");
							toggle
						},
					);
				});
				assert!(output.platform_output.commands.is_empty());
				output.textures_delta.clear();
			}
			assert!(
				matches!(action, Some(Action::Toggle(ref emoji)) if toggle && emoji.same(&values[0].emoji))
					|| (!toggle && action.is_none())
			);
		}
	}
}
