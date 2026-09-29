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
	let account_label = user.account_label();
	let badge = account_label.map(|text| {
		let key = match text {
			"BOT" => "account-badge-bot",
			"APP" => "account-badge-app",
			_ => "account-badge-webhook",
		};
		ui.painter().layout_no_wrap(
			crate::i18n::translate_if_key(key),
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
		let description_key = match account_label {
			Some("BOT") => "account-badge-bot-description",
			Some("APP") => "account-badge-app-description",
			_ => "account-badge-webhook-description",
		};
		let description = crate::i18n::translate_if_key(description_key);
		badge.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &description));
		badge.on_hover_text(description);
	}
	response
}
