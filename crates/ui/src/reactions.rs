use model::{Reaction, ReactionEmoji};

#[derive(Debug, PartialEq)]
pub enum Action {
	Reload,
	Toggle(ReactionEmoji),
	Inspect(ReactionEmoji, bool),
}

fn reacting_names<'a>(names: impl Iterator<Item = &'a str>, count: u32) -> String {
	let names = names.take(3).collect::<Vec<_>>();
	let remaining = count.saturating_sub(names.len() as u32);
	let names = names.join(", ");
	if remaining == 0 {
		names
	} else {
		format!(
			"{names}, and {remaining} other{}",
			if remaining == 1 { "" } else { "s" }
		)
	}
}

fn reaction_button(
	ctx: &egui::Context,
	avatars: &mut crate::avatars::Avatars,
	emoji: &ReactionEmoji,
	count: u32,
	demo: bool,
) -> egui::Button<'static> {
	let label = emoji.label();
	let count = count.to_string();
	match emoji.id {
		None => crate::emoji::button(ctx, &label, count),
		Some(id) => {
			let image = avatars
				.custom_image(ctx, id, 18.0, demo)
				.unwrap_or_else(|| crate::emoji::blank(ctx, 18.0));
			egui::Button::image_and_text(image.alt_text(label), count)
				.image_tint_follows_text_color(false)
		}
	}
}

/// Height of the pill strip, using the same row size as [`show`].
/// Empty and still-loading counts reserve nothing.
pub fn estimated_height(ui: &egui::Ui, reactions: Option<&[Reaction]>, width: f32) -> f32 {
	let Some(reactions) = reactions.filter(|reactions| !reactions.is_empty()) else {
		return 0.0;
	};
	let width = width.max(40.0);
	let font = egui::TextStyle::Button.resolve(ui.style());
	let mut rows = 1u32;
	let mut used = 0.0_f32;
	for reaction in reactions {
		let text_width = ui
			.painter()
			.layout_no_wrap(
				reaction.count.to_string(),
				font.clone(),
				egui::Color32::WHITE,
			)
			.size()
			.x;
		let pill = 34.0 + text_width;
		if used > 0.0 && used + 4.0 + pill > width {
			rows += 1;
			used = pill;
		} else {
			used = if used == 0.0 { pill } else { used + 4.0 + pill };
		}
	}
	6.0 + rows as f32 * 30.0
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
		if writing || refreshing {
			ui.visuals_mut().disabled_alpha = 1.0;
		}
		for reaction in reactions {
			let label = format!("{} {}", reaction.emoji.label(), reaction.count);
			let button =
				reaction_button(ui.ctx(), media.0, &reaction.emoji, reaction.count, media.1);
			let response = ui.add_enabled(
				!writing
					&& !refreshing && reaction.emoji.name.is_some()
					&& can_react(&reaction.emoji, !reaction.me),
				button
					.gap(4.0)
					.min_size(egui::vec2(0.0, 26.0))
					.corner_radius(6)
					.selected(reaction.me),
			);
			response.widget_info(|| {
				egui::WidgetInfo::selected(
					egui::Role::Button,
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
			let colors = crate::design::palette(ui);
			let width = 320.0_f32.min((ui.ctx().content_rect().width() - 40.0).max(180.0));
			egui::Popup::from_response(&response)
				.kind(egui::PopupKind::Tooltip)
				.open(response.enabled() && egui::Tooltip::should_show_tooltip(&response, false))
				.gap(4.0)
				.width(width)
				.interactable(false)
				.frame(
					egui::Frame::popup(ui.style())
						.fill(colors.base)
						.stroke(egui::Stroke::new(1.0, colors.border))
						.inner_margin(12)
						.corner_radius(6),
				)
				.show(|ui| {
					ui.set_width(width - 24.0);
					ui.horizontal_centered(|ui| {
						if let Some(image) = match reaction.emoji.id {
							Some(id) => Some(
								media
									.0
									.custom_image(ui.ctx(), id, 52.0, media.1)
									.unwrap_or_else(|| crate::emoji::blank(ui.ctx(), 52.0)),
							),
							None => crate::emoji::image(ui.ctx(), &reaction.emoji.label(), 52.0),
						} {
							ui.add(image);
						} else {
							ui.label(
								egui::RichText::new(reaction.emoji.label())
									.size(26.0)
									.color(colors.text_strong),
							);
						}
						ui.add_space(8.0);
						let summary = if let Some(value) = matching
							&& !value.users.is_empty()
						{
							let reactors = reacting_names(
								value.users.iter().map(|user| user.name.as_str()),
								reaction.count,
							);
							format!("{} reacted by {reactors}", reaction.emoji.label())
						} else if matching.is_some_and(|value| value.error.is_some()) {
							"Reaction details unavailable".into()
						} else {
							"Loading reactions…".into()
						};
						ui.add(
							egui::Label::new(
								egui::RichText::new(summary).size(15.0).color(colors.text),
							)
							.wrap(),
						);
					});
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

pub fn show_frozen(
	ui: &mut egui::Ui,
	reactions: Option<&[Reaction]>,
	media: (&mut crate::avatars::Avatars, bool),
) {
	if reactions.is_none_or(<[Reaction]>::is_empty) {
		return;
	}
	ui.horizontal_wrapped(|ui| {
		ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
		ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
		ui.spacing_mut().interact_size.y = 26.0;
		ui.visuals_mut().disabled_alpha = 1.0;
		for reaction in reactions.unwrap_or_default() {
			let label = format!("{} {}", reaction.emoji.label(), reaction.count);
			let button =
				reaction_button(ui.ctx(), media.0, &reaction.emoji, reaction.count, media.1);
			let response = ui.add_enabled(
				false,
				button
					.gap(4.0)
					.min_size(egui::vec2(0.0, 26.0))
					.corner_radius(6)
					.selected(reaction.me),
			);
			response.widget_info(|| {
				egui::WidgetInfo::selected(egui::Role::Button, false, reaction.me, &label)
			});
		}
	});
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
								reaction_button(
									ui.ctx(),
									avatars,
									&reaction.emoji,
									reaction.count,
									state.demo,
								)
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
	fn truncated_reaction_names_include_the_hidden_count() {
		assert_eq!(
			reacting_names(["A", "B", "C", "D"].into_iter(), 9),
			"A, B, C, and 6 others"
		);
		assert_eq!(reacting_names(["A", "B", "C"].into_iter(), 3), "A, B, C");
		assert_eq!(
			reacting_names(["A", "B", "C"].into_iter(), 4),
			"A, B, C, and 1 other"
		);
	}

	#[test]
	fn custom_reaction_button_loads_its_image() {
		let ctx = egui::Context::default();
		let mut avatars = crate::avatars::Avatars::default();
		let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
			ui.add(reaction_button(
				ui.ctx(),
				&mut avatars,
				&ReactionEmoji {
					id: Some(model::Id(9001)),
					name: Some("catgirlvibe".into()),
				},
				2,
				true,
			));
		});
		assert!(!output.textures_delta.set.is_empty());
		output.textures_delta.clear();
	}

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
