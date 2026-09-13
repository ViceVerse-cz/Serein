//! Emoji information uses the already-loaded server catalog and artwork.
use crate::{design, emoji, emoji_picker};

fn title(text: &str) -> String {
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
		.find(|(value, _)| {
			value
				.chars()
				.filter(|c| *c != '\u{fe0f}')
				.eq(text.chars().filter(|c| *c != '\u{fe0f}'))
		})
		.map_or_else(
			|| text.to_owned(),
			|(_, name)| emoji_picker::shortcode(name),
		)
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
			let source = custom.and_then(|(id, _)| {
				guilds.iter().find_map(|guild| {
					guild
						.emojis
						.as_ref()?
						.iter()
						.find(|emoji| emoji.id == id)
						.map(|emoji| (guild, emoji))
				})
			});
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
					let name = source
						.map_or_else(|| title(text), |(_, emoji)| format!(":{}:", emoji.name));
					ui.add(
						egui::Label::new(
							design::semibold(ui, name, 16.0).color(colors.text_strong),
						)
						.wrap(),
					);
					if custom.is_some() {
						ui.label("A custom emoji.");
						if let Some((guild, _)) = source {
							ui.add(egui::Label::new(format!("From {}", guild.name)).wrap());
						} else {
							ui.add(
								egui::Label::new("Source server unavailable in this session.")
									.wrap(),
							);
						}
					} else {
						ui.add(
							egui::Label::new(
								"A default emoji. You can use this emoji everywhere on Discord.",
							)
							.wrap(),
						);
					}
				});
			});
		});
	response.context_menu(|ui| {
		if ui.button("Copy emoji").clicked() {
			ui.ctx().copy_text(text.to_owned());
			ui.close();
		}
	});
}
