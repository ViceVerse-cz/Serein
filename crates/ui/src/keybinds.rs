//! Shortcut reference; keycaps are labels, not buttons that execute the action.
use crate::design;

pub(super) fn show(ui: &mut egui::Ui) {
	let colors = design::palette(ui);
	let command = if cfg!(target_os = "macos") {
		"CMD"
	} else {
		"CTRL"
	};
	ui.heading("Custom Keybinds");
	ui.label("Keyboard shortcuts let you perform actions without clicking through menus.");
	design::card(ui, |ui| {
		ui.label("Custom keybinds are not available yet. The default shortcuts below work while Serein is focused.");
	});
	ui.add_space(16.0);
	ui.separator();
	ui.add_space(16.0);
	ui.heading("Default Keybinds");
	group(
		ui,
		"Navigation",
		"Use these shortcuts while browsing conversations.",
		&[
			("Show Keyboard Shortcuts List", &[command, "/"]),
			("Switch Conversation", &[command, "K"]),
			("Close Settings or Dialog", &["ESC"]),
		],
	);
	group(
		ui,
		"Messages",
		"These shortcuts work in the message composer.",
		&[
			("Send Message", &["ENTER"]),
			("Insert New Line", &["SHIFT", "ENTER"]),
			("Edit Last Editable Message (empty composer)", &["↑"]),
		],
	);
	group(
		ui,
		"Text Formatting",
		"Apply or remove formatting in the composer.",
		&[
			("Bold", &[command, "B"]),
			("Italic", &[command, "I"]),
			("Underline", &[command, "U"]),
			("Strikethrough", &[command, "SHIFT", "X"]),
			("Inline Code", &[command, "E"]),
			("Code Block", &[command, "SHIFT", "C"]),
			("Spoiler", &[command, "SHIFT", "P"]),
		],
	);
	group(
		ui,
		"Voice",
		"Enable Push to Talk in Voice & Audio. Hold the key during a connected call, with Serein focused and no text field active.",
		&[("Push to Talk", &["V"])],
	);
	ui.label(
		egui::RichText::new(
			"Shortcuts are local to Serein; global hotkeys and custom remapping are not supported.",
		)
		.color(colors.muted),
	);
}

fn group(ui: &mut egui::Ui, title: &str, description: &str, rows: &[(&str, &[&str])]) {
	ui.add_space(16.0);
	ui.label(design::semibold(ui, title, 18.0));
	ui.weak(description);
	design::card(ui, |ui| {
		for (index, (label, keys)) in rows.iter().enumerate() {
			if index > 0 {
				ui.separator();
			}
			if ui.available_width() < 360.0 {
				ui.label(*label);
				ui.horizontal_wrapped(|ui| {
					for key in *keys {
						keycap(ui, key);
					}
				});
			} else {
				ui.horizontal(|ui| {
					ui.set_min_height(42.0);
					ui.allocate_ui_with_layout(
						egui::vec2(ui.available_width() - 190.0, 42.0),
						egui::Layout::left_to_right(egui::Align::Center),
						|ui| {
							ui.add(egui::Label::new(*label).wrap());
						},
					);
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						for key in keys.iter().rev() {
							keycap(ui, key);
						}
					});
				});
			}
		}
	});
}

fn keycap(ui: &mut egui::Ui, label: &str) -> egui::Response {
	let colors = design::palette(ui);
	let font = egui::FontId::new(12.0, design::medium_family(ui.ctx()));
	let galley = ui
		.painter()
		.layout_no_wrap(label.to_owned(), font, colors.text_strong);
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2((galley.size().x + 16.0).max(26.0), 29.0),
		egui::Sense::hover(),
	);
	response
		.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), label));
	ui.painter().rect_filled(rect, 4, colors.border);
	let face = rect.with_max_y(rect.bottom() - 4.0);
	ui.painter().rect(
		face,
		4,
		colors.hover,
		egui::Stroke::new(1.0, colors.border),
		egui::StrokeKind::Inside,
	);
	ui.painter().galley(
		face.center() - galley.size() * 0.5,
		galley,
		colors.text_strong,
	);
	response
}

#[cfg(test)]
mod tests {
	#[test]
	fn keybinds_fit_narrow_and_wide_in_both_themes() {
		for width in [240.0, 720.0] {
			for visuals in [egui::Visuals::dark(), egui::Visuals::light()] {
				let ctx = egui::Context::default();
				ctx.set_visuals(visuals);
				let mut output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 2200.0),
						)),
						..Default::default()
					},
					|ui| {
						egui::CentralPanel::default().show(ui, |ui| {
							let right = ui.max_rect().right();
							super::show(ui);
							assert!(ui.min_rect().right() <= right + 1.0);
						});
					},
				);
				output.textures_delta.clear();
			}
		}
	}
}
