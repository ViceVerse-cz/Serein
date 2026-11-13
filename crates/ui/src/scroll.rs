use crate::design;
use egui::{AsIdSalt, IdSalt, Pos2, Rect, ScrollArea, Shape, Stroke, pos2};

const TOUCHPAD_STATE_KEY: &str = "serein-touchpad-scroll-state";
const WHEEL_LATCH_KEY: &str = "serein-wheel-modifier-latch";

#[derive(Clone, Copy, Default)]
struct TouchpadState {
	last_active: Option<f64>,
	accumulator: egui::Vec2,
	/// Where the pointer was when the queued movement arrived. egui gives wheel movement to the
	/// hovered scroll area, so the queue belongs to whatever sits under this point.
	origin: Option<Pos2>,
	/// The frame that last took a smoothed step. A discarded pass reruns the frame.
	stepped_frame: Option<u64>,
}

/// How far the pointer may drift before queued movement stops belonging to its scroll area.
const QUEUE_ORIGIN_SLOP: f32 = 4.0;

/// egui's wheel modifier latch. Between a gesture's `TouchPhase::Start` and `End`, a released
/// modifier still applies and a newly pressed one joins, so zoom and axis follow egui's choice.
#[derive(Clone, Copy, Default, PartialEq)]
struct WheelLatch {
	modifiers: egui::Modifiers,
	/// The open gesture's latest event time and source.
	touch: Option<(f64, egui::MouseWheelSource)>,
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
	// Every path reads the frame's wheel events once, so the latch sees each event exactly once.
	let latch_key = egui::Id::unique(WHEEL_LATCH_KEY);
	let before: WheelLatch = ctx.data(|d| d.get_temp(latch_key)).unwrap_or_default();
	let mut latch = before;
	let (incoming, zooming) = ctx.input(|input| {
		let incoming =
			instant_wheel_delta(&input.raw.events, options, page_height, time, &mut latch);
		let zooming = zoom_gesture(&latch, options, &input.raw.events);
		(incoming, zooming)
	});
	if latch != before {
		ctx.data_mut(|d| d.insert_temp(latch_key, latch));
	}

	let has_high_res_event = ctx.input(|input| {
		input.raw.events.iter().any(|event| match event {
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
	} else if let Some(last) = state.last_active
		&& time - last < 0.35
	{
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
	}

	let is_scrolling = ctx.input(|i| i.is_scrolling());
	// A zoom that starts while a swipe is still queued must not keep scrolling.
	if zooming {
		state.accumulator = egui::Vec2::ZERO;
	}
	let touchpad_active = state.last_active.is_some_and(|last| time - last < 0.35)
		|| state.accumulator.length() > 0.5;
	let queue_draining = state
		.last_active
		.is_some_and(|last| time - last >= 0.35 && is_scrolling);

	if state.last_active.is_some() && !touchpad_active && !queue_draining {
		state.last_active = None;
		state.accumulator = egui::Vec2::ZERO;
		ctx.data_mut(|d| d.remove_temp::<TouchpadState>(key));
	}

	if ctx.input(|i| i.pointer.any_pressed()) {
		state.accumulator = egui::Vec2::ZERO;
	}
	let hover = ctx.input(|i| i.pointer.hover_pos());
	if incoming != egui::Vec2::ZERO {
		state.origin = hover;
	} else if !match (hover, state.origin) {
		(Some(hover), Some(origin)) => hover.distance(origin) <= QUEUE_ORIGIN_SLOP,
		(hover, origin) => hover == origin,
	} {
		// The pointer moved to what may be another scroll area; it must not get this swipe.
		state.accumulator = egui::Vec2::ZERO;
	}

	let speed = f32::from(preferences.scroll_speed_percent) / 100.0;
	let delta = if zooming {
		egui::Vec2::ZERO
	} else if touchpad_active {
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
		// Smoothing spreads one gesture across frames, so the queue is bounded.
		// With smoothing off the whole event is delivered below and must not be clipped.
		if preferences.smooth_scrolling {
			let max_backlog = (page_height * 2.0).clamp(1200.0, 2400.0);
			if state.accumulator.length() > max_backlog {
				state.accumulator = state.accumulator.normalized() * max_backlog;
			}
		}

		let dt_clamped = dt.clamp(1.0 / 240.0, 0.05);
		let frame = ctx.cumulative_frame_nr();
		let step = if !preferences.smooth_scrolling {
			let d = state.accumulator;
			state.accumulator = egui::Vec2::ZERO;
			d
		} else {
			let len = state.accumulator.length();
			// For small or slow touchpad scrolls, deliver 100% immediately for crisp 1:1 tactile feel.
			// Divide the thresholds by the speed factor so the applied delta respects them at every
			// speed setting, and the per-frame cap still bounds the timeline overscan.
			let viewport_cap = page_height * 0.45 / speed;
			let immediate = (120.0 / speed).min(viewport_cap);
			if state.stepped_frame == Some(frame) {
				egui::Vec2::ZERO
			} else if len <= immediate {
				let d = state.accumulator;
				state.accumulator = egui::Vec2::ZERO;
				d
			} else {
				let frame_ratio = (dt_clamped / (1.0 / 60.0)).clamp(0.5, 1.5);
				let max_step = (page_height * 0.25).clamp(120.0, 160.0) * frame_ratio / speed;
				let decay = 1.0 - (-dt_clamped / 0.05).exp();
				let step_len = (len * decay)
					.max(120.0 / speed)
					.min(max_step)
					.min(viewport_cap)
					.min(len);
				let d = state.accumulator.normalized() * step_len;
				state.accumulator -= d;
				state.stepped_frame = Some(frame);
				d
			}
		};

		if state.accumulator.length() > 0.5 {
			ctx.request_repaint();
		}

		step
	} else if queue_draining {
		incoming
	} else {
		state.accumulator = egui::Vec2::ZERO;
		if preferences.smooth_scrolling {
			ctx.input(|i| i.smooth_scroll_delta())
		} else {
			incoming
		}
	};

	if touchpad_active || queue_draining {
		ctx.data_mut(|d| d.insert_temp(key, state));
	}

	ctx.input_mut(|input| {
		input.smooth_scroll_delta = delta * speed;
	});
}

fn instant_wheel_delta(
	events: &[egui::Event],
	options: egui::InputOptions,
	page_height: f32,
	time: f64,
	latch: &mut WheelLatch,
) -> egui::Vec2 {
	events
		.iter()
		.filter_map(|event| {
			let egui::Event::MouseWheel {
				unit,
				delta,
				phase,
				modifiers,
				source,
			} = event
			else {
				return None;
			};
			// A gesture whose `End` never arrives times out like egui's (resting finger or not).
			if let Some((last, touch_source)) = latch.touch {
				let timeout = if touch_source == egui::MouseWheelSource::Trackpad {
					1.0
				} else {
					0.15
				};
				if time - last > timeout {
					*latch = WheelLatch::default();
				}
			}
			match phase {
				egui::TouchPhase::Start => {
					*latch = WheelLatch {
						modifiers: *modifiers,
						touch: Some((time, *source)),
					};
					return None;
				}
				egui::TouchPhase::End | egui::TouchPhase::Cancel => {
					*latch = WheelLatch::default();
					return None;
				}
				egui::TouchPhase::Move => {}
			}
			if let Some(touch) = &mut latch.touch {
				*touch = (time, *source);
				latch.modifiers |= *modifiers;
			} else {
				latch.modifiers = *modifiers;
			}
			let modifiers = latch.modifiers;
			if modifiers.matches_any(options.zoom_modifier) {
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

fn zoom_gesture(latch: &WheelLatch, options: egui::InputOptions, events: &[egui::Event]) -> bool {
	let zoom = options.zoom_modifier;
	if latch.touch.is_some() && latch.modifiers.matches_any(zoom) {
		return true;
	}
	events.iter().any(|event| match event {
		egui::Event::MouseWheel { modifiers, .. } => modifiers.matches_any(zoom),
		_ => false,
	})
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
	fn multiple_notched_mouse_wheel_events_in_one_frame_preserve_smooth_scrolling() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let raw = egui::RawInput {
			time: Some(0.1),
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(900.0, 600.0),
			)),
			events: vec![
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Line,
					delta: egui::vec2(0.0, 1.0),
					modifiers: egui::Modifiers::NONE,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
				},
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Line,
					delta: egui::vec2(0.0, 1.0),
					modifiers: egui::Modifiers::NONE,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
				},
			],
			..Default::default()
		};
		let mut delta = egui::Vec2::ZERO;
		ctx.run_ui(raw, |ui| {
			apply_preferences(ui.ctx(), model::ReadingPreferences::default());
			delta = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		// Two integer 1.0 line notches delivered together must preserve smooth scrolling
		// (~76px, well under the 240px instant delta that would occur if misclassified as touchpad).
		assert!(delta.y > 40.0 && delta.y < 120.0);
		let key = egui::Id::unique(TOUCHPAD_STATE_KEY);
		assert!(ctx.data(|d| d.get_temp::<TouchpadState>(key)).is_none());
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
			events: vec![
				egui::Event::PointerMoved(egui::pos2(450.0, 300.0)),
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Line,
					delta: egui::vec2(0.0, 10.25),
					modifiers: egui::Modifiers::NONE,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
				},
			],
			..Default::default()
		};
		let mut delta = egui::Vec2::ZERO;
		ctx.run_ui(raw, |ui| {
			apply_preferences(ui.ctx(), model::ReadingPreferences::default());
			delta = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		// Frame 1 moves a bounded step, well short of the 1,230px swipe and half the viewport.
		assert!(
			delta.y > 0.0 && delta.y < 300.0,
			"frame 1 moved {}",
			delta.y
		);

		// Frame 2 with no new events continues smoothly from the accumulator, avoiding single-frame stalls.
		// A discarded pass reruns the frame, and the rerun must not take a second step.
		let frame_input = |time: f64, events: Vec<egui::Event>| egui::RawInput {
			time: Some(time),
			screen_rect: Some(egui::Rect::from_min_size(
				egui::Pos2::ZERO,
				egui::vec2(900.0, 600.0),
			)),
			events,
			..Default::default()
		};
		let mut passes = Vec::new();
		ctx.run_ui(frame_input(0.116, vec![]), |ui| {
			apply_preferences(ui.ctx(), model::ReadingPreferences::default());
			passes.push(ui.input(|i| i.smooth_scroll_delta()).y);
			if passes.len() == 1 {
				ui.ctx().request_discard("test rerun");
			}
		})
		.drop_without_applying_deltas();
		assert_eq!(passes.len(), 2);
		assert!(
			passes[0] > 0.0 && passes[0] < 300.0,
			"frame 2 moved {}",
			passes[0]
		);
		assert_eq!(passes[1], 0.0, "the rerun took another step");

		// The rest of the swipe belongs to the area under the pointer; moving away drops it.
		let mut delta3 = egui::Vec2::ZERO;
		ctx.run_ui(
			frame_input(
				0.132,
				vec![egui::Event::PointerMoved(egui::pos2(100.0, 300.0))],
			),
			|ui| {
				apply_preferences(ui.ctx(), model::ReadingPreferences::default());
				delta3 = ui.input(|i| i.smooth_scroll_delta());
			},
		)
		.drop_without_applying_deltas();
		assert_eq!(delta3, egui::Vec2::ZERO);
	}

	#[test]
	fn scroll_speed_percent_does_not_bypass_the_touchpad_frame_cap() {
		let step = |scroll_speed_percent| {
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
					delta: egui::vec2(0.0, 10.25),
					modifiers: egui::Modifiers::NONE,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Unknown,
				}],
				..Default::default()
			};
			let preferences = model::ReadingPreferences {
				scroll_speed_percent,
				..Default::default()
			};
			let mut delta = egui::Vec2::ZERO;
			ctx.run_ui(raw, |ui| {
				apply_preferences(ui.ctx(), preferences);
				delta = ui.input(|i| i.smooth_scroll_delta());
			})
			.drop_without_applying_deltas();
			delta.y
		};
		let (normal, fast) = (step(100), step(300));
		assert!(
			fast > 0.0 && fast <= normal + 0.5,
			"a 300% speed factor must not push one frame past the cap: {fast} vs {normal}"
		);
	}

	#[test]
	fn released_zoom_modifier_stays_latched_for_the_touchpad_gesture() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let wheel = |phase, modifiers| egui::Event::MouseWheel {
			unit: egui::MouseWheelUnit::Point,
			delta: egui::vec2(0.0, 30.0),
			modifiers,
			phase,
			source: egui::MouseWheelSource::Trackpad,
		};
		let frame = |time, events| {
			let raw = egui::RawInput {
				time: Some(time),
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 600.0),
				)),
				events,
				..Default::default()
			};
			let mut delta = egui::Vec2::ZERO;
			ctx.run_ui(raw, |ui| {
				apply_preferences(ui.ctx(), model::ReadingPreferences::default());
				delta = ui.input(|i| i.smooth_scroll_delta());
			})
			.drop_without_applying_deltas();
			delta
		};
		let zoom = egui::InputOptions::default().zoom_modifier;
		frame(
			0.1,
			vec![
				wheel(egui::TouchPhase::Start, zoom),
				wheel(egui::TouchPhase::Move, zoom),
			],
		);
		// egui keeps zooming after the modifier is released mid-gesture; nothing may scroll.
		let released = frame(
			0.116,
			vec![wheel(egui::TouchPhase::Move, egui::Modifiers::NONE)],
		);
		assert_eq!(released, egui::Vec2::ZERO);
		// The next gesture starts without the modifier and scrolls again.
		frame(
			0.2,
			vec![wheel(egui::TouchPhase::End, egui::Modifiers::NONE)],
		);
		let next = frame(
			0.216,
			vec![
				wheel(egui::TouchPhase::Start, egui::Modifiers::NONE),
				wheel(egui::TouchPhase::Move, egui::Modifiers::NONE),
			],
		);
		assert_eq!(next.y, 30.0);
	}

	#[test]
	fn touchpad_step_stays_within_a_short_viewport() {
		// 0.9 lines stays under the immediate threshold; 10.25 lines takes the capped path.
		for lines in [0.9, 10.25] {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			let raw = egui::RawInput {
				time: Some(0.1),
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 200.0),
				)),
				events: vec![egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Line,
					delta: egui::vec2(0.0, lines),
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
			assert!(
				delta.y > 0.0 && delta.y <= 90.0,
				"{lines} lines moved a 200pt viewport by {}",
				delta.y
			);
		}
	}

	#[test]
	fn disabled_smoothing_keeps_a_large_wheel_event() {
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
				delta: egui::vec2(0.0, 10.25),
				modifiers: egui::Modifiers::NONE,
				phase: egui::TouchPhase::Move,
				source: egui::MouseWheelSource::Unknown,
			}],
			..Default::default()
		};
		let mut delta = egui::Vec2::ZERO;
		ctx.run_ui(raw, |ui| {
			apply_preferences(
				ui.ctx(),
				model::ReadingPreferences {
					smooth_scrolling: false,
					..Default::default()
				},
			);
			delta = ui.input(|i| i.smooth_scroll_delta());
		})
		.drop_without_applying_deltas();
		assert_eq!(delta.y, 10.25 * 120.0);
	}

	#[test]
	fn queued_touchpad_swipe_does_not_scroll_during_zoom() {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let frame = |time: f64, events: Vec<egui::Event>| {
			let raw = egui::RawInput {
				time: Some(time),
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 600.0),
				)),
				events,
				..Default::default()
			};
			let mut delta = egui::Vec2::ZERO;
			ctx.run_ui(raw, |ui| {
				apply_preferences(ui.ctx(), model::ReadingPreferences::default());
				delta = ui.input(|i| i.smooth_scroll_delta());
			})
			.drop_without_applying_deltas();
			delta
		};
		let swipe = frame(
			0.1,
			vec![egui::Event::MouseWheel {
				unit: egui::MouseWheelUnit::Line,
				delta: egui::vec2(0.0, 10.25),
				modifiers: egui::Modifiers::NONE,
				phase: egui::TouchPhase::Move,
				source: egui::MouseWheelSource::Unknown,
			}],
		);
		assert!(
			swipe.y > 0.0 && swipe.y < 1_230.0,
			"the swipe must leave a queue, moved {}",
			swipe.y
		);
		let zoom = egui::InputOptions::default().zoom_modifier;
		let zoomed = frame(
			0.12,
			vec![
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Point,
					delta: egui::vec2(0.0, 40.0),
					modifiers: zoom,
					phase: egui::TouchPhase::Start,
					source: egui::MouseWheelSource::Trackpad,
				},
				egui::Event::MouseWheel {
					unit: egui::MouseWheelUnit::Point,
					delta: egui::vec2(0.0, 40.0),
					modifiers: zoom,
					phase: egui::TouchPhase::Move,
					source: egui::MouseWheelSource::Trackpad,
				},
			],
		);
		assert_eq!(zoomed, egui::Vec2::ZERO);
		let idle = frame(0.14, vec![]);
		assert_eq!(idle, egui::Vec2::ZERO);
	}
}
