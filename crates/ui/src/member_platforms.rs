use crate::icons::{self, Icon};
use egui::{Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use model::ClientPlatforms;

pub(crate) const SIZE: f32 = 13.0;

#[derive(Clone, Copy)]
pub(crate) enum Platform {
	Desktop,
	Mobile,
	Web,
	Vr,
}

pub(crate) fn active(
	clients: ClientPlatforms,
	visible: bool,
) -> impl Iterator<Item = Platform> + Clone {
	[
		(clients.desktop, Platform::Desktop),
		(clients.mobile, Platform::Mobile),
		(clients.web, Platform::Web),
		(clients.vr, Platform::Vr),
	]
	.into_iter()
	.filter_map(move |(active, platform)| (visible && active).then_some(platform))
}

impl Platform {
	pub(crate) fn label(self) -> &'static str {
		match self {
			Self::Desktop => "member-platform-desktop",
			Self::Mobile => "member-platform-mobile",
			Self::Web => "member-platform-web",
			Self::Vr => "member-platform-vr",
		}
	}

	pub(crate) fn show(self, ui: &mut Ui, language: crate::i18n::Language, color: Color32) {
		let (rect, response) = ui.allocate_exact_size(vec2(SIZE, SIZE), Sense::hover());
		let painter = ui.painter();
		match self {
			Self::Desktop => {
				let screen = rect.shrink2(vec2(0.75, 1.0));
				let screen = egui::Rect::from_min_max(screen.min, screen.max - vec2(0.0, 3.0));
				let stroke = Stroke::new(1.25, color);
				painter.rect_stroke(screen, 1, stroke, StrokeKind::Inside);
				let base = screen.center_bottom() + vec2(0.0, 2.5);
				painter.line_segment([screen.center_bottom(), base], stroke);
				painter.hline(base.x - 3.0..=base.x + 3.0, base.y, stroke);
			}
			Self::Mobile => icons::paint(painter, Icon::DeviceMobile, rect, color),
			Self::Web => icons::paint(painter, Icon::Globe, rect, color),
			Self::Vr => {
				let lenses = rect.shrink2(vec2(0.75, 3.0));
				painter.rect_stroke(lenses, 2, Stroke::new(1.25, color), StrokeKind::Inside);
				for x in [-2.5, 2.5] {
					painter.circle_filled(lenses.center() + vec2(x, 0.0), 0.9, color);
				}
			}
		}
		response.on_hover_ui(|ui| {
			ui.label(language.text(self.label()));
		});
	}
}
