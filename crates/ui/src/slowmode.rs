//! Slowmode notice at the end of the strip above the composer: a clock beside the notice, or
//! beside the wait once this account has sent a message.
use client_core::State;
use model::Id;
use std::time::{Duration, Instant};

const TEXT_SIZE: f32 = 12.5;
const CLOCK: f32 = 13.0;
const GAP: f32 = 6.0;

/// Whole seconds still to wait, as `m:ss` or `h:mm:ss`.
fn countdown(left: Duration) -> String {
	let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
	let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
	if hours > 0 {
		format!("{hours}:{minutes:02}:{seconds:02}")
	} else {
		format!("{minutes}:{seconds:02}")
	}
}

/// Paints the notice right-aligned in `rect` and returns the width it took, zero when this
/// account is not held to a slowmode here.
pub(super) fn overlay(
	ui: &mut egui::Ui,
	rect: egui::Rect,
	state: &State,
	channel: Id,
	now: Instant,
) -> f32 {
	if state.slowmode(channel).is_none() || !state.can_compose(channel) {
		return 0.0;
	}
	let colors = crate::design::palette(ui);
	let remaining = state.slowmode_remaining(channel, now);
	let notice = crate::i18n::translate("slowmode-enabled");
	let text = remaining.map_or_else(|| notice.clone(), countdown);
	let galley = ui.painter().layout_no_wrap(
		text.clone(),
		egui::FontId::proportional(TEXT_SIZE),
		colors.muted,
	);
	let width = (galley.size().x + GAP + CLOCK).min(rect.width());
	let area = egui::Rect::from_min_max(egui::pos2(rect.right() - width, rect.top()), rect.max);
	let response = ui.interact(
		area,
		ui.make_persistent_id("slowmode-indicator"),
		egui::Sense::hover(),
	);
	let painter = ui.painter_at(area);
	painter.galley(
		egui::pos2(area.left(), area.center().y - galley.size().y / 2.0),
		galley,
		colors.muted,
	);
	let center = egui::pos2(area.right() - CLOCK / 2.0, area.center().y);
	let stroke = egui::Stroke::new(1.3, colors.muted);
	painter.circle_stroke(center, CLOCK / 2.0 - 0.75, stroke);
	painter.line_segment([center, center - egui::vec2(0.0, 3.5)], stroke);
	painter.line_segment([center, center + egui::vec2(2.6, 0.0)], stroke);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
	if let Some(left) = remaining {
		crate::icons::immediate_tooltip(response, &notice);
		// One frame per shown second, and one as the wait ends.
		ui.ctx().request_repaint_after(Duration::from_nanos(
			u64::from(left.subsec_nanos()) + 1_000_000,
		));
	}
	width
}
