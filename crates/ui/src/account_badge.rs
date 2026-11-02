//! One compact, theme-aware account name/badge row for chat and people.
use crate::{design, icons};
use egui::{Color32, Response, Sense, Ui, vec2};
use model::User;

pub(super) fn name(
	ui: &mut Ui,
	user: &User,
	name: &str,
	size: f32,
	color: (Color32, Option<model::server_roles::Colors>, Color32),
	sense: Sense,
	trailing: f32,
) -> Response {
	let colors = design::palette(ui);
	let account_label = user.account_label();
	let verified = user.kind == model::AccountKind::VerifiedBot;
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
		text.size().x + 8.0 + if verified { 10.0 } else { 0.0 } + ui.spacing().item_spacing.x
	});
	let width = (ui.available_width() - reserve - trailing).max(0.0);
	// A Nitro name style, when switched on, replaces the role colour like Discord's.
	let response = ui
		.scope(|ui| {
			ui.set_max_width(width);
			crate::name_styles::label(
				ui,
				crate::name_styles::style(user),
				name,
				egui::FontId::new(size, design::medium_family(ui.ctx())),
				color.1,
				color.2,
				color.0,
				sense,
				false,
			)
		})
		.inner;
	if let Some(text) = badge {
		let verified_width = if verified { 10.0 } else { 0.0 };
		let (rect, badge) = ui.allocate_exact_size(
			vec2(text.size().x + 8.0 + verified_width, 16.0),
			Sense::hover(),
		);
		ui.painter().rect_filled(rect, 3, colors.accent);
		if verified {
			icons::paint(
				ui.painter(),
				icons::Icon::Check,
				egui::Rect::from_center_size(
					egui::pos2(rect.left() + 8.5, rect.center().y),
					vec2(9.0, 9.0),
				),
				colors.accent_text,
			);
		}
		ui.painter().galley(
			egui::pos2(
				rect.left() + 4.0 + verified_width,
				rect.center().y - text.size().y * 0.5,
			),
			text,
			colors.accent_text,
		);
		let description_key = match (account_label, verified) {
			(_, true) => "account-badge-verified-bot-description",
			(Some("BOT"), _) => "account-badge-bot-description",
			(Some("APP"), _) => "account-badge-app-description",
			_ => "account-badge-webhook-description",
		};
		let description = crate::i18n::translate_if_key(description_key);
		badge.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &description));
		badge.on_hover_text(description);
	}
	response
}
