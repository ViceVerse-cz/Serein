//! One compact, theme-aware account name/badge row for chat and people.
use crate::design;
use egui::{Color32, Response, Sense, Ui, vec2};
use model::User;

pub(super) fn name(
	ui: &mut Ui,
	user: &User,
	name: &str,
	size: f32,
	color: Color32,
	sense: Sense,
	trailing: f32,
) -> Response {
	let colors = design::palette(ui);
	let badge = user.account_label().map(|text| {
		ui.painter().layout_no_wrap(
			text.into(),
			egui::FontId::proportional(10.0),
			colors.accent_text,
		)
	});
	let reserve = badge.as_ref().map_or(0.0, |text| {
		text.size().x + 8.0 + ui.spacing().item_spacing.x
	});
	let width = (ui.available_width() - reserve - trailing).max(0.0);
	let response = ui
		.scope(|ui| {
			ui.set_max_width(width);
			ui.add(
				egui::Label::new(design::medium(ui, name, size).color(color))
					.truncate()
					.selectable(false)
					.sense(sense),
			)
		})
		.inner;
	if let Some(text) = badge {
		let (rect, badge) = ui.allocate_exact_size(vec2(text.size().x + 8.0, 16.0), Sense::hover());
		ui.painter().rect_filled(rect, 3, colors.accent);
		ui.painter()
			.galley(rect.center() - text.size() * 0.5, text, colors.accent_text);
		let description = match user.account_label() {
			Some("BOT") => "Bot account",
			Some("APP") => "Application-generated message",
			_ => "Webhook author",
		};
		badge.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, description));
		badge.on_hover_text(description);
	}
	response
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn badges_fit_next_to_long_names_and_leave_humans_untagged() {
		for light in [false, true] {
			for width in [130.0, 240.0] {
				for (kind, webhook, expected) in [
					(model::AccountKind::Human, false, None),
					(model::AccountKind::Bot, false, Some("BOT")),
					(model::AccountKind::App, true, Some("APP")),
					(model::AccountKind::Human, true, Some("WEBHOOK")),
				] {
					let ctx = egui::Context::default();
					ctx.set_theme(if light {
						egui::ThemePreference::Light
					} else {
						egui::ThemePreference::Dark
					});
					design::apply(&ctx);
					let user = User {
						id: model::Id(1),
						name: "A very long synthetic nickname".repeat(4),
						avatar: None,
						discriminator: 0,
						primary_guild: None,
						kind,
						webhook,
					};
					let mut name_rect = egui::Rect::NOTHING;
					let output = ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								vec2(width, 100.0),
							)),
							..Default::default()
						},
						|ui| {
							ui.horizontal(|ui| {
								name_rect = name(
									ui,
									&user,
									&user.name,
									15.0,
									design::palette(ui).text,
									Sense::click(),
									0.0,
								)
								.rect;
							});
						},
					);
					let badge = output.shapes.iter().find_map(|shape| match &shape.shape {
						egui::Shape::Text(text)
							if Some(text.galley.job.text.as_str()) == expected =>
						{
							Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
						}
						_ => None,
					});
					assert_eq!(badge.is_some(), expected.is_some());
					if let Some(badge) = badge {
						assert!(badge.left() >= name_rect.right(), "{name_rect:?} {badge:?}");
						assert!(badge.right() <= width, "{width}: {badge:?}");
						assert!((badge.center().y - name_rect.center().y).abs() < 3.0);
					}
					output.drop_without_applying_deltas();
				}
			}
		}
	}
}
