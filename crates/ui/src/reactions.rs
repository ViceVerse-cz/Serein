use crate::design::LazyHover;
use model::{Reaction, ReactionEmoji};

pub fn show(
	ui: &mut egui::Ui,
	reactions: Option<&[Reaction]>,
	enabled: bool,
	writing: bool,
	refreshing: bool,
	media: (&mut crate::avatars::Avatars, bool),
	can_react: impl Fn(&ReactionEmoji, bool) -> bool,
) -> Option<Option<ReactionEmoji>> {
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
				action = Some(None);
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
			let verb = if reaction.me {
				"Remove your reaction"
			} else {
				"Add your reaction"
			};
			let response = response.on_hover_text_with(|| {
				format!(
					"{verb}: {}. Count includes super reactions; only normal reactions can be toggled here.",
					reaction.emoji.label()
				)
			});
			if response.clicked() {
				action = Some(Some(reaction.emoji.clone()));
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
					|_, _| true,
				),
				None
			);
			assert_eq!(ui.min_rect(), before);
		});
		output.drop_without_applying_deltas();
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
						|_, add| {
							assert!(!add, "The owned reaction is removed");
							toggle
						},
					);
				});
				assert!(output.platform_output.commands.is_empty());
				output.textures_delta.clear();
			}
			assert_eq!(action, toggle.then(|| Some(values[0].emoji.clone())));
		}
	}
}
