use crate::design::Palette;
use crate::icons::{self, Icon};
use client_core::ChannelAccess;
use egui::{Color32, Pos2, Rect, Vec2};

const DIM: f32 = 0.6;
const EYE: f32 = 22.0;
const SCROLL: f32 = 16.0;
const LOCK: f32 = 8.0;
const LOCK_HALO: f32 = 3.5;
const LOCK_U: f32 = 19.5 / 24.0;
const LOCK_V: f32 = 6.25 / 24.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Emphasis {
	Unavailable,
	Idle,
	Unread,
	Focused,
	Connected,
}

pub(crate) fn tint(colors: &Palette, access: ChannelAccess, emphasis: Emphasis) -> Color32 {
	if access.dim() {
		return colors.muted.gamma_multiply(DIM);
	}
	match emphasis {
		Emphasis::Unavailable => colors.muted.gamma_multiply(DIM),
		Emphasis::Connected => colors.accent,
		Emphasis::Focused | Emphasis::Unread => colors.text_strong,
		Emphasis::Idle => colors.muted,
	}
}

pub(crate) fn trailing(access: ChannelAccess) -> f32 {
	if access.hidden() { EYE + SCROLL } else { 0.0 }
}

pub(crate) fn paint(
	painter: &egui::Painter,
	access: ChannelAccess,
	row: Rect,
	glyph: Rect,
	color: Color32,
	background: Color32,
) {
	if access.limited() {
		let badge = lock_badge(glyph);
		painter.circle_filled(badge.center(), LOCK * 0.5 + LOCK_HALO, background);
		icons::paint(painter, Icon::Lock, badge, color);
	}
	if access.hidden() {
		let center = Pos2::new(row.right() - SCROLL - EYE * 0.5, row.center().y);
		icons::paint(
			painter,
			Icon::EyeSlash,
			Rect::from_center_size(center, Vec2::splat(EYE)),
			color,
		);
	}
}

fn lock_badge(glyph: Rect) -> Rect {
	Rect::from_center_size(
		glyph.min + Vec2::new(glyph.width() * LOCK_U, glyph.height() * LOCK_V),
		Vec2::splat(LOCK),
	)
}

pub(crate) fn label(access: ChannelAccess) -> &'static str {
	match (access.muted(), access.hidden(), access.limited()) {
		(false, false, false) => "",
		(true, false, false) => " · Muted",
		(false, true, false) => " · Hidden",
		(false, false, true) => " · Limited",
		(true, true, false) => " · Muted · Hidden",
		(true, false, true) => " · Muted · Limited",
		(false, true, true) => " · Hidden · Limited",
		(true, true, true) => " · Muted · Hidden · Limited",
	}
}
