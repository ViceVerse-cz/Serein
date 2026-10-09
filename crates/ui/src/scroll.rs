use crate::design;
use egui::{AsIdSalt, IdSalt, Pos2, Rect, ScrollArea, Shape, Stroke, pos2};

const TOUCHPAD_STATE_KEY: &str = "serein-touchpad-scroll-state";

#[derive(Clone, Copy, Default)]
struct TouchpadState {
	last_active: Option<f64>,
	accumulator: egui::Vec2,
}

/// Apply once before rendering the app's scroll areas. Zoom gestures remain unchanged.
pub fn apply_preferences(ctx: &egui::Context, preferences: model::ReadingPreferences) {
	if !preferences.is_valid() {
		return;
	}
	let options = ctx.options(|options| options.input_options);
	let key = egui::Id::unique(TOUCHPAD_STATE_KEY);
	let (time, page_height, dt) = ctx.input(|i| (i.time, i.viewport_rect().height(), i.stable_dt));
	let mut state: TouchpadState = ctx.data(|d| d.get_temp(key)).unwrap_or_default();

	let has_high_res_event = ctx.input(|input| {
		let wheel_events = input
			.raw
			.events
			.iter()
			.filter(|event| matches!(event, egui::Event::MouseWheel { .. }))
			.count();
		wheel_events > 1
			|| input.raw.events.iter().any(|event| match event {
				egui::Event::MouseWheel {
					unit,
					delta,
					source,
					..
				} => {
					*source == egui::MouseWheelSource::Trackpad
						|| *source == egui::MouseWheelSource::Momentum
						|| *unit == egui::MouseWheelUnit::Point
						|| delta.x.fract().abs() > 0.0001
						|| delta.y.fract().abs() > 0.0001
				}
				_ => false,
			})
	});

	if has_high_res_event {
		state.last_active = Some(time);
	} else if let Some(last) = state.last_active {
		if time - last < 0.35 {
			let has_any_wheel = ctx.input(|input| {
				input
					.raw
					.events
					.iter()
					.any(|event| matches!(event, egui::Event::MouseWheel { .. }))
			});
			if has_any_wheel {
				state.last_active = Some(time);
			}
		} else {
			state.last_active = None;
			state.accumulator = egui::Vec2::ZERO;
		}
	}

	let touchpad_active = state.last_active.is_some_and(|last| time - last < 0.35)
		|| state.accumulator.length() > 0.5;

	if ctx.input(|i| i.pointer.any_pressed()) {
		state.accumulator = egui::Vec2::ZERO;
	}

	let delta = if touchpad_active {
		let incoming =
			ctx.input(|input| instant_wheel_delta(&input.raw.events, options, page_height));
		if incoming.y != 0.0
			&& state.accumulator.y != 0.0
			&& incoming.y.signum() != state.accumulator.y.signum()
		{
			state.accumulator.y = 0.0;
		}
		if incoming.x != 0.0
			&& state.accumulator.x != 0.0
			&& incoming.x.signum() != state.accumulator.x.signum()
		{
			state.accumulator.x = 0.0;
		}
		state.accumulator += incoming;

		let dt_clamped = dt.clamp(1.0 / 240.0, 0.05);
		let step = if !preferences.smooth_scrolling {
			let d = state.accumulator;
			state.accumulator = egui::Vec2::ZERO;
			d
		} else {
			let frame_ratio = dt_clamped / (1.0 / 60.0);
			let max_step = (page_height * 0.25).clamp(120.0, 180.0) * frame_ratio;
			let len = state.accumulator.length();
			if len <= max_step {
				let d = state.accumulator;
				state.accumulator = egui::Vec2::ZERO;
				d
			} else {
				let d = state.accumulator.normalized() * max_step;
				state.accumulator -= d;
				d
			}
		};

		if state.accumulator.length() > 0.5 {
			ctx.request_repaint();
		}

		step
	} else {
		state.accumulator = egui::Vec2::ZERO;
		if preferences.smooth_scrolling {
			ctx.input(|i| i.smooth_scroll_delta())
		} else {
			ctx.input(|input| instant_wheel_delta(&input.raw.events, options, page_height))
		}
	};

	ctx.data_mut(|d| d.insert_temp(key, state));

	ctx.input_mut(|input| {
		input.smooth_scroll_delta = delta * (f32::from(preferences.scroll_speed_percent) / 100.0);
	});
}

pub(super) fn instant_wheel_delta(
	events: &[egui::Event],
	options: egui::InputOptions,
	page_height: f32,
) -> egui::Vec2 {
	events
		.iter()
		.filter_map(|event| {
			let egui::Event::MouseWheel {
				unit,
				delta,
				phase,
				modifiers,
				..
			} = event
			else {
				return None;
			};
			if *phase != egui::TouchPhase::Move || modifiers.matches_any(options.zoom_modifier) {
				return None;
			}
			let mut delta = match unit {
				egui::MouseWheelUnit::Point => *delta,
				egui::MouseWheelUnit::Line => options.line_scroll_speed * *delta,
				egui::MouseWheelUnit::Page => page_height * *delta,
			};
			let horizontal = modifiers.matches_any(options.horizontal_scroll_modifier);
			let vertical = modifiers.matches_any(options.vertical_scroll_modifier);
			if horizontal && !vertical {
				delta = egui::vec2(delta.x + delta.y, 0.0);
			}
			if !horizontal && vertical {
				delta = egui::vec2(0.0, delta.x + delta.y);
			}
			Some(delta)
		})
		.fold(egui::Vec2::ZERO, |total, delta| total + delta)
}
/// Chromium / Discord default: 3 wheel lines times 40 px. winit reports one notch as `LineDelta` 1.0.
pub const DISCORD_LINE_SCROLL_SPEED: f32 = 120.0;

/// Chromium's middle-click autoscroll shape (`autoscroll_controller.cc`): a dead zone, then
/// the full distance from the origin raised to a power. The dead zone is a gate, not subtracted.
pub const DEAD_ZONE: f32 = 15.0;
const CURVE: f32 = 2.2;
const GAIN: f32 = 0.11;
const CEILING: f32 = 48_000.0;

/// The middle button over one frame, delivered outside egui's pointer state.
///
/// egui starts a text selection on `any_pressed()` while a selectable label is hovered, and
/// keeps extending it while any button is down. Middle counts. The window layer never gives
/// egui the button; it arrives here instead.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Middle {
	/// Position of the frame's first press, if it pressed.
	pub pressed: Option<Pos2>,
	/// Down at the end of the frame.
	pub down: bool,
}

/// Mouse 4 / mouse 5 edge presses for this frame, delivered outside egui's pointer state.
///
/// Label selection uses `any_pressed()`, so these buttons must not enter egui.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct SidePress {
	pub back: bool,
	pub forward: bool,
}

#[derive(Clone, Copy, Default)]
enum Drive {
	#[default]
	Idle,
	Driving {
		aim: Aim,
		hold: Hold,
	},
}

#[derive(Clone, Copy)]
struct Aim {
	origin: Pos2,
	cursor: Pos2,
	target: egui::Id,
}

#[derive(Clone, Copy)]
enum Hold {
	Button { wandered: bool },
	Latched,
}

#[derive(Default)]
pub struct Session {
	drive: Drive,
	middle: Middle,
	frame: Option<u64>,
	bound: bool,
	last_offset: Option<(egui::Id, f32)>,
	ignore_press: bool,
}

/// Scroll speed in points per second for a cursor `offset` points from the drive origin.
/// Signed like `offset`. Dead zone is a gate, not subtracted.
pub fn speed(offset: f32) -> f32 {
	let distance = offset.abs();
	if distance <= DEAD_ZONE {
		return 0.0;
	}
	offset.signum() * (GAIN * distance.powf(CURVE)).min(CEILING)
}

impl Session {
	/// Feed the frame's middle button before any `bind`/`attach`. Never fed means never pressed.
	pub fn middle(&mut self, middle: Middle) {
		self.middle = middle;
	}

	/// True while a drive needs the cursor position, including outside the window.
	pub fn tracking(&self) -> bool {
		matches!(self.drive, Drive::Driving { .. })
	}

	pub fn holding(&self) -> bool {
		!matches!(self.drive, Drive::Idle)
	}

	pub fn bind(&mut self, ui: &egui::Ui, target: egui::Id, area: Rect) -> f32 {
		let frame = ui.ctx().cumulative_frame_nr();
		if self.frame != Some(frame) {
			self.frame = Some(frame);
			self.bound = false;
			self.ignore_press = self.step(ui);
		}
		if !self.ignore_press {
			self.try_start(ui, target, area);
		}
		match self.drive {
			Drive::Driving { aim, .. } if aim.target == target => {
				self.bound = true;
				let dt = ui.input(|input| input.stable_dt).min(0.05);
				-speed(aim.cursor.y - aim.origin.y) * dt
			}
			_ => 0.0,
		}
	}

	pub fn attach(
		&mut self,
		ui: &egui::Ui,
		salt: impl AsIdSalt + Copy,
		builder: ScrollArea,
	) -> ScrollArea {
		let builder = builder.id_salt(salt);
		let target = ui.make_persistent_id(IdSalt::new(salt));
		let area = ui.available_rect_before_wrap().intersect(ui.clip_rect());
		let delta = self.bind(ui, target, area);
		if delta == 0.0 {
			if !self.holding() {
				self.last_offset = None;
			}
			return builder;
		}
		let Some(state) = egui::scroll_area::State::load(ui.ctx(), target) else {
			return builder;
		};
		let next = (state.clamped_offset().y - delta).max(0.0);
		if (next - state.clamped_offset().y).abs() < f32::EPSILON {
			return builder;
		}
		if let Some((id, last)) = self.last_offset
			&& id == target
			&& clamped_away(last, state.clamped_offset().y, next)
		{
			return builder;
		}
		self.last_offset = Some((target, next));
		ui.ctx().request_repaint();
		builder.vertical_scroll_offset(next)
	}

	pub fn paint(&self, ctx: &egui::Context) {
		let origin = match self.drive {
			Drive::Driving { aim, .. } => aim.origin,
			Drive::Idle => return,
		};
		let colors = design::palette_for(ctx);
		let painter = ctx.layer_painter(egui::LayerId::new(
			egui::Order::Foreground,
			egui::Id::unique("serein-autoscroll"),
		));
		painter.circle_filled(origin, 12.0, colors.raised);
		painter.circle_stroke(origin, 12.0, Stroke::new(1.0, colors.muted));
		let tip = 3.6;
		let gap = 1.6;
		painter.add(Shape::convex_polygon(
			vec![
				pos2(origin.x, origin.y - gap - tip),
				pos2(origin.x - tip, origin.y - gap),
				pos2(origin.x + tip, origin.y - gap),
			],
			colors.text,
			Stroke::NONE,
		));
		painter.add(Shape::convex_polygon(
			vec![
				pos2(origin.x, origin.y + gap + tip),
				pos2(origin.x - tip, origin.y + gap),
				pos2(origin.x + tip, origin.y + gap),
			],
			colors.text,
			Stroke::NONE,
		));
		ctx.set_cursor_icon(egui::CursorIcon::ResizeVertical);
	}

	pub fn clear_if_unbound(&mut self, ctx: &egui::Context) {
		if self.frame != Some(ctx.cumulative_frame_nr()) || !self.bound {
			self.drive = Drive::Idle;
			self.last_offset = None;
		}
	}

	fn step(&mut self, ui: &egui::Ui) -> bool {
		let Drive::Driving { mut aim, hold } = self.drive else {
			return false;
		};
		let middle = std::mem::take(&mut self.middle);
		let (focused, egui_pressed, escape, wheel, hover) = ui.input(|input| {
			(
				input.focused,
				input.pointer.any_pressed(),
				input.key_pressed(egui::Key::Escape),
				input.smooth_scroll_delta() != egui::Vec2::ZERO,
				input.pointer.hover_pos(),
			)
		});
		aim.cursor = hover.unwrap_or(aim.cursor);

		if !focused || escape || wheel || egui_pressed {
			self.idle();
			return egui_pressed;
		}
		match hold {
			Hold::Button { wandered } => {
				let wandered = wandered || (aim.cursor.y - aim.origin.y).abs() > DEAD_ZONE;
				if middle.down {
					self.drive = Drive::Driving {
						aim,
						hold: Hold::Button { wandered },
					};
				} else if wandered {
					self.idle();
				} else {
					self.drive = Drive::Driving {
						aim,
						hold: Hold::Latched,
					};
				}
			}
			Hold::Latched => {
				if middle.pressed.is_some() {
					self.idle();
					return true;
				}
				self.drive = Drive::Driving {
					aim,
					hold: Hold::Latched,
				};
			}
		}
		false
	}

	fn idle(&mut self) {
		self.drive = Drive::Idle;
		self.last_offset = None;
	}

	fn try_start(&mut self, ui: &egui::Ui, target: egui::Id, area: Rect) {
		if !matches!(self.drive, Drive::Idle) {
			return;
		}
		let Some(pos) = self.middle.pressed.filter(|pos| area.contains(*pos)) else {
			return;
		};
		if ui.input(|input| input.pointer.any_down()) {
			return;
		}
		self.drive = Drive::Driving {
			aim: Aim {
				origin: pos,
				cursor: pos,
				target,
			},
			hold: Hold::Button { wandered: false },
		};
		self.bound = true;
	}
}

fn clamped_away(requested: f32, current: f32, next: f32) -> bool {
	(requested - current).abs() > 0.5 && (next - current).signum() == (requested - current).signum()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn notched_mouse_wheel_uses_smooth_scroll_when_enabled() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut raw = egui::RawInput {
			time: Some(0.1),
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(900.0, 600.0),
			)),
			events: vec![egui::Event::MouseWheel {
				unit: egui::MouseWheelUnit::Line,
				delta: egui::vec2(0.0, 1.0),
				modifiers: egui::Modifiers::NONE,
				phase: egui::TouchPhase::Move,
				source: egui::MouseWheelSource::Unknown,
			}],
			..Default::default()
		};
		let mut delta = egui::Vec2::ZERO;
		ctx.run_ui(raw.clone(), |ui| {
			apply_preferences(ui.ctx(), model::ReadingPreferences::default());
			delta = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		// For a notched wheel, egui smooths 120px over frames (t ≈ 0.32), so frame 1 delta is ~38px
		assert!(delta.y > 20.0 && delta.y < 60.0);

		// With smooth_scrolling = false, instant_wheel_delta applies all 120px in frame 1
		let prefs_instant = model::ReadingPreferences {
			smooth_scrolling: false,
			..Default::default()
		};
		raw.time = Some(0.2);
		ctx.run_ui(raw, |ui| {
			apply_preferences(ui.ctx(), prefs_instant);
			delta = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		assert_eq!(delta.y, 120.0);
	}

	#[test]
	fn touchpad_fractional_delta_bypasses_exponential_delay_queue() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let raw = egui::RawInput {
			time: Some(0.1),
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(900.0, 600.0),
			)),
			events: vec![egui::Event::MouseWheel {
				unit: egui::MouseWheelUnit::Line,
				// Fractional delta characteristic of Windows Precision Touchpad
				delta: egui::vec2(0.0, 0.5),
				modifiers: egui::Modifiers::NONE,
				phase: egui::TouchPhase::Move,
				source: egui::MouseWheelSource::Unknown,
			}],
			..Default::default()
		};
		let mut delta = egui::Vec2::ZERO;
		ctx.run_ui(raw, |ui| {
			apply_preferences(ui.ctx(), model::ReadingPreferences::default());
			delta = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		// 0.5 lines * 120px/line = 60px instant delta (not 0.32 * 60 ≈ 19px)
		assert_eq!(delta.y, 60.0);

		// Frame with no events during active touchpad gesture should be 0, not egui's queued lag
		let raw_idle = egui::RawInput {
			time: Some(0.15),
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(900.0, 600.0),
			)),
			events: vec![],
			..Default::default()
		};
		ctx.run_ui(raw_idle, |ui| {
			apply_preferences(ui.ctx(), model::ReadingPreferences::default());
			delta = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		assert_eq!(delta.y, 0.0);
	}

	#[test]
	fn touchpad_fast_swipe_is_soft_clamped_per_frame() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let raw = egui::RawInput {
			time: Some(0.1),
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(900.0, 600.0),
			)),
			// Rapid swipe delivering 10.25 lines = 1,230px in 1 frame
			events: vec![egui::Event::MouseWheel {
				unit: egui::MouseWheelUnit::Line,
				delta: egui::vec2(0.0, 10.25),
				modifiers: egui::Modifiers::NONE,
				phase: egui::TouchPhase::Move,
				source: egui::MouseWheelSource::Unknown,
			}],
			..Default::default()
		};
		let mut delta = egui::Vec2::ZERO;
		ctx.run_ui(raw, |ui| {
			apply_preferences(ui.ctx(), model::ReadingPreferences::default());
			delta = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		// Frame 1 is clamped to a smooth per-frame step (around 168px, not 1230px)
		assert!(delta.y >= 140.0 && delta.y <= 180.0);

		// Frame 2 with no new events continues smoothly from the accumulator, avoiding single-frame stalls
		let raw_frame2 = egui::RawInput {
			time: Some(0.116),
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(900.0, 600.0),
			)),
			events: vec![],
			..Default::default()
		};
		let mut delta2 = egui::Vec2::ZERO;
		ctx.run_ui(raw_frame2, |ui| {
			apply_preferences(ui.ctx(), model::ReadingPreferences::default());
			delta2 = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		assert!(delta2.y >= 140.0 && delta2.y <= 180.0);
	}
}
