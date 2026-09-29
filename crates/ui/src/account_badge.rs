//! One compact, theme-aware account name/badge row for chat and people.
use crate::{design, icons};
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

#[cfg(test)]
mod tests {
	use super::*;

	fn textured_shapes(shape: &egui::Shape) -> usize {
		match shape {
			egui::Shape::Mesh(mesh) => usize::from(mesh.texture_id != egui::TextureId::default()),
			egui::Shape::Rect(rect) => usize::from(
				rect.brush
					.as_ref()
					.is_some_and(|brush| brush.fill_texture_id != egui::TextureId::default()),
			),
			egui::Shape::Vec(shapes) => shapes.iter().map(textured_shapes).sum(),
			_ => 0,
		}
	}

	fn icon_meshes(kind: model::AccountKind) -> usize {
		let context = egui::Context::default();
		crate::design::apply(&context);
		let user = User {
			kind,
			webhook: false,
			id: model::Id(1),
			name: "Synthetic app".into(),
			avatar: None,
			discriminator: 0,
			primary_guild: None,
		};
		let output = context.run_ui(Default::default(), |ui| {
			ui.set_width(300.0);
			name(
				ui,
				&user,
				"Synthetic app",
				15.0,
				egui::Color32::WHITE,
				Sense::hover(),
				0.0,
			);
		});
		let count = output
			.shapes
			.iter()
			.map(|shape| textured_shapes(&shape.shape))
			.sum();
		output.drop_without_applying_deltas();
		count
	}

	#[test]
	fn verified_bot_adds_check_icon_to_app_badge() {
		assert_eq!(icon_meshes(model::AccountKind::App), 0);
		assert_eq!(icon_meshes(model::AccountKind::VerifiedBot), 1);
	}
}
