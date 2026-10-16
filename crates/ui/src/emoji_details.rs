//! Emoji information uses the already-loaded server catalog and artwork.
use crate::{design, emoji, emoji_picker};

/// Edge of the artwork in emoji and sticker hover cards.
pub(crate) const HOVER_ART: f32 = 40.0;

/// Discord-style `:name:` for a default emoji or a `<:name:id>` custom emoji. Text that is
/// neither is returned unchanged.
pub(crate) fn shortcode(text: &str) -> String {
	if let Some((_, len)) = emoji::custom_prefix(text)
		&& len == text.len()
	{
		return format!(
			":{}:",
			text.trim_end_matches('>')
				.split(':')
				.nth(1)
				.unwrap_or("emoji")
		);
	}
	emoji_picker::standard()
		.iter()
		.position(|(value, _)| {
			value
				.chars()
				.filter(|c| *c != '\u{fe0f}')
				.eq(text.chars().filter(|c| *c != '\u{fe0f}'))
		})
		.map_or_else(
			|| text.to_owned(),
			|index| emoji_picker::shortcodes()[index].clone(),
		)
}

/// The joined server whose catalog holds a custom emoji, with its current catalog entry.
fn source<'a>(
	text: &str,
	guilds: &'a [model::Guild],
) -> Option<(&'a model::Guild, &'a model::CustomEmoji)> {
	let (id, _) = emoji::custom_prefix(text)?;
	guilds.iter().find_map(|guild| {
		guild
			.emojis
			.as_ref()?
			.iter()
			.find(|emoji| emoji.id == id)
			.map(|emoji| (guild, emoji))
	})
}

/// Name shown for message emoji: the catalog's current name when its server is joined.
pub(crate) fn name(text: &str, guilds: &[model::Guild]) -> String {
	source(text, guilds).map_or_else(|| shortcode(text), |(_, emoji)| format!(":{}:", emoji.name))
}

/// Hover card shared by message emoji and stickers: artwork beside its name, and a hint when
/// clicking opens details.
pub(crate) fn hover_card(
	response: egui::Response,
	image: Option<egui::Image<'static>>,
	name: &str,
	hint: bool,
) -> egui::Response {
	response.on_hover_ui(|ui| {
		let colors = design::palette(ui);
		ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 10.0;
			if let Some(image) = image {
				ui.add(image.fit_to_exact_size(egui::Vec2::splat(HOVER_ART)));
			}
			ui.vertical(|ui| {
				ui.spacing_mut().item_spacing.y = 2.0;
				ui.label(design::semibold(ui, name, 14.0).color(colors.text_strong));
				if hint {
					ui.label(
						egui::RichText::new(crate::i18n::translate(
							"emoji-details-hover-card-click-to-learn-more",
						))
						.size(12.0)
						.color(colors.muted),
					);
				}
			});
		});
	})
}

pub(crate) fn show(
	ui: &mut egui::Ui,
	response: &egui::Response,
	text: &str,
	image: Option<egui::Image<'static>>,
	guilds: &[model::Guild],
) {
	if !ui.is_enabled() {
		egui::Popup::close_id(ui.ctx(), response.id.with("emoji-details"));
		return;
	}
	let colors = design::palette(ui);
	let width = 340.0_f32.min((ui.ctx().content_rect().width() - 40.0).max(160.0));
	egui::Popup::from_toggle_button_response(response)
		.id(response.id.with("emoji-details"))
		.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
		.gap(6.0)
		.width(width)
		.frame(
			egui::Frame::popup(ui.style())
				.fill(colors.raised)
				.inner_margin(16)
				.corner_radius(10),
		)
		.show(|ui| {
			let custom = emoji::custom_prefix(text);
			let source = source(text, guilds);
			ui.set_width(width - 32.0);
			ui.horizontal_top(|ui| {
				let (rect, _) =
					ui.allocate_exact_size(egui::Vec2::splat(64.0), egui::Sense::hover());
				if let Some(image) = image {
					let size = image.calc_size(rect.size(), image.size());
					image.paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
				} else {
					ui.painter().text(
						rect.center(),
						egui::Align2::CENTER_CENTER,
						"?",
						egui::FontId::proportional(32.0),
						colors.muted,
					);
				}
				ui.add_space(12.0);
				ui.vertical(|ui| {
					ui.set_width(ui.available_width().max(52.0));
					let name = name(text, guilds);
					ui.add(
						egui::Label::new(
							design::semibold(ui, name, 16.0).color(colors.text_strong),
						)
						.wrap(),
					);
					if custom.is_some() {
						ui.label(crate::i18n::translate("emoji-details-show-a-custom-emoji"));
						if let Some((guild, _)) = source {
							ui.add(
								egui::Label::new(format!(
									"{} {}",
									crate::i18n::translate("emoji-details-show-from"),
									guild.name
								))
								.wrap(),
							);
						} else {
							ui.add(
								egui::Label::new(crate::i18n::translate(
									"emoji-details-show-source-server-unavailable-in-this-session",
								))
								.wrap(),
							);
						}
					} else {
						ui.add(
							egui::Label::new(crate::i18n::translate(
								"emoji-details-show-a-default-emoji-you-can-use-this-emoji-everywhere-on",
							))
							.wrap(),
						);
					}
				});
			});
		});
	response.context_menu(|ui| {
		if ui
			.button(crate::i18n::translate("emoji-details-show-copy-emoji"))
			.clicked()
		{
			ui.ctx().copy_text(text.to_owned());
			ui.close();
		}
	});
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_use_shortcodes_and_the_joined_catalog() {
		assert_eq!(shortcode("😂"), ":joy:");
		assert_eq!(shortcode("🔥"), ":fire:");
		assert_eq!(shortcode("👍🏽"), ":thumbsup_tone3:");
		assert_eq!(shortcode("❤"), ":heart:");
		assert_eq!(shortcode("<:wave:9001>"), ":wave:");
		assert_eq!(shortcode("<a:wave_animated:9001>"), ":wave_animated:");
		// Trailing text and unknown sequences are not emoji; they stay as received.
		assert_eq!(shortcode("<:wave:9001> hi"), "<:wave:9001> hi");
		assert_eq!(shortcode("sticker"), "sticker");
		let guilds = test_support::demo_state().guilds;
		let (guild, emoji) = source("<a:old_name:9001>", &guilds).expect("synthetic catalog");
		assert_eq!(
			name("<a:old_name:9001>", &guilds),
			format!(":{}:", emoji.name)
		);
		assert!(!guild.name.is_empty());
		assert_eq!(name("<a:wave_animated:999999>", &guilds), ":wave_animated:");
		assert_eq!(name("😂", &guilds), ":joy:");
	}
}
