use crate::design;
use egui::{AsIdSalt, IdSalt, Pos2, Rect, ScrollArea, Shape, Stroke, pos2};

/// A wheel notch finishes within this interval after the last event, regardless of frame rate.
const WHEEL_SMOOTHING_SECONDS: f64 = 0.150;

#[derive(Clone, Copy)]
struct WheelOwner {
	target: egui::Id,
	area: Rect,
	layer: egui::LayerId,
	frame: u64,
}

/// Fixed-size, viewport-local state. Distances are accumulated, never queued per event.
#[derive(Clone, Default)]
struct WheelMotion {
	pending: egui::Vec2,
	deadline: f64,
	last_time: Option<f64>,
	preferences: Option<(bool, u16)>,
	scope: Option<egui::Id>,
	owner: Option<WheelOwner>,
	hovered_layer: Option<egui::LayerId>,
	focused: Option<bool>,
	pass: Option<u64>,
}

impl WheelMotion {
	fn cancel(&mut self) {
		self.pending = egui::Vec2::ZERO;
		self.owner = None;
	}

	fn update(
		&mut self,
		input: &egui::InputState,
		options: egui::InputOptions,
		preferences: model::ReadingPreferences,
	) -> egui::Vec2 {
		let now = input.time;
		let settings = (
			preferences.smooth_scrolling,
			preferences.scroll_speed_percent,
		);
		if self.preferences != Some(settings) {
			self.cancel();
			self.preferences = Some(settings);
		}
		if !preferences.is_valid() || !now.is_finite() {
			self.cancel();
			self.last_time = None;
			return egui::Vec2::ZERO;
		}
		// Desktop systems can route new wheel events to the hovered, inactive window.
		// A focus transition cancels its old gesture without discarding those new events.
		if self.focused == Some(true) && !input.focused {
			self.cancel();
		}
		self.focused = Some(input.focused);
		let events = after_wheel_replacement(&input.raw.events);
		if self.last_time.is_some_and(|last| now < last) || events.len() != input.raw.events.len() {
			self.cancel();
		}
		if !preferences.smooth_scrolling {
			self.cancel();
			self.last_time = Some(now);
			let output = instant_wheel_delta(events, options, input.viewport_rect().height())
				* (f32::from(preferences.scroll_speed_percent) / 100.0);
			return if output.is_finite() {
				output
			} else {
				egui::Vec2::ZERO
			};
		}
		let mut due = if now >= self.deadline {
			std::mem::take(&mut self.pending)
		} else {
			egui::Vec2::ZERO
		};
		let was_pending = self.pending != egui::Vec2::ZERO;
		let mut direct = egui::Vec2::ZERO;
		let mut fresh = false;
		for event in events {
			let egui::Event::MouseWheel {
				unit,
				delta,
				phase,
				source,
				modifiers,
			} = event
			else {
				continue;
			};
			if modifiers.matches_any(options.zoom_modifier) {
				// egui has already extracted zoom from the event; do not also scroll.
				self.cancel();
				due = egui::Vec2::ZERO;
				continue;
			}
			if *phase == egui::TouchPhase::Start {
				self.cancel();
				due = egui::Vec2::ZERO;
			}
			if !matches!(phase, egui::TouchPhase::Move | egui::TouchPhase::Start) {
				continue;
			}
			let delta = normalized_wheel_delta(
				*unit,
				*delta,
				*modifiers,
				options,
				input.viewport_rect().height(),
			);
			if delta == egui::Vec2::ZERO {
				continue;
			}
			let precision = matches!(
				source,
				egui::MouseWheelSource::Trackpad | egui::MouseWheelSource::Momentum
			) || (*unit == egui::MouseWheelUnit::Point
				&& *source != egui::MouseWheelSource::Wheel);
			if precision {
				self.cancel();
				due = egui::Vec2::ZERO;
				direct += delta;
			} else {
				for axis in 0..2 {
					if delta[axis] != 0.0 && self.pending[axis].signum() != delta[axis].signum() {
						self.pending[axis] = 0.0;
					}
					if delta[axis] != 0.0 && due[axis].signum() != delta[axis].signum() {
						due[axis] = 0.0;
					}
				}
				self.pending += delta;
				self.deadline = now + WHEEL_SMOOTHING_SECONDS;
				fresh = true;
			}
		}
		let elapsed = self
			.last_time
			.map_or(f64::from(input.stable_dt), |last| now - last)
			.max(0.0);
		// A new gesture should respond on its first frame, even after an idle layout pass.
		let elapsed = if fresh && (!was_pending || elapsed == 0.0) {
			f64::from(input.stable_dt)
		} else {
			elapsed
		};
		let fraction = if now >= self.deadline {
			1.0
		} else {
			(elapsed / (self.deadline - now + elapsed)).clamp(0.0, 1.0)
		};
		let smoothed = self.pending * fraction as f32;
		self.pending -= smoothed;
		self.last_time = Some(now);
		if !self.pending.is_finite() || !direct.is_finite() {
			self.cancel();
			return egui::Vec2::ZERO;
		}
		let output =
			(due + direct + smoothed) * (f32::from(preferences.scroll_speed_percent) / 100.0);
		if output.is_finite() {
			output
		} else {
			egui::Vec2::ZERO
		}
	}
}

fn replaces_wheel_motion(event: &egui::Event) -> bool {
	matches!(
		event,
		egui::Event::PointerButton { pressed: true, .. }
			| egui::Event::PointerGone
			| egui::Event::Key {
				pressed: true,
				key: egui::Key::Escape
					| egui::Key::ArrowUp
					| egui::Key::ArrowDown
					| egui::Key::ArrowLeft
					| egui::Key::ArrowRight
					| egui::Key::PageUp
					| egui::Key::PageDown
					| egui::Key::Home
					| egui::Key::End,
				..
			}
	)
}

fn after_wheel_replacement(events: &[egui::Event]) -> &[egui::Event] {
	let start = events
		.iter()
		.rposition(replaces_wheel_motion)
		.map_or(0, |index| index + 1);
	&events[start..]
}

fn wheel_id(ctx: &egui::Context) -> egui::Id {
	egui::Id::unique(("serein-wheel-motion", ctx.viewport_id()))
}

/// Apply once before rendering the app's scroll areas. Zoom gestures remain unchanged.
pub fn apply_preferences(ctx: &egui::Context, preferences: model::ReadingPreferences) {
	apply_preferences_in(
		ctx,
		preferences,
		egui::Id::unique("serein-default-scroll-scope"),
	);
}

/// `scope` identifies the conversation/overlay receiving input. Changing it retires old motion.
pub fn apply_preferences_in(
	ctx: &egui::Context,
	preferences: model::ReadingPreferences,
	scope: egui::Id,
) {
	let id = wheel_id(ctx);
	let mut motion = ctx.data_mut(|data| data.remove_temp::<WheelMotion>(id).unwrap_or_default());
	let pass = ctx.cumulative_pass_nr();
	if motion.pass == Some(pass) {
		ctx.data_mut(|data| data.insert_temp(id, motion));
		return;
	}
	motion.pass = Some(pass);
	if motion.scope != Some(scope) {
		motion.cancel();
		motion.scope = Some(scope);
	}
	let pointer = ctx.input(|input| input.pointer.hover_pos());
	let hovered_layer = pointer.and_then(|pos| ctx.layer_id_at(pos));
	let frame = ctx.cumulative_frame_nr();
	let outside_owner = motion.owner.is_some_and(|owner| {
		pointer.is_none_or(|pos| {
			!owner.area.contains(pos) || ctx.layer_id_at(pos) != Some(owner.layer)
		}) || owner.frame.saturating_add(1) < frame
	});
	if outside_owner || (motion.owner.is_none() && motion.hovered_layer != hovered_layer) {
		motion.cancel();
	}
	motion.hovered_layer = hovered_layer;
	let options = ctx.options(|options| options.input_options);
	let delta = ctx.input(|input| motion.update(input, options, preferences));
	let pending = motion.pending != egui::Vec2::ZERO;
	ctx.input_mut(|input| input.smooth_scroll_delta = delta);
	ctx.data_mut(|data| data.insert_temp(id, motion));
	if pending {
		ctx.request_repaint();
	}
}

/// Retire a wheel gesture when another interaction, such as middle-click or navigation, replaces it.
pub fn reset_wheel(ctx: &egui::Context) {
	let id = wheel_id(ctx);
	ctx.data_mut(|data| data.remove::<WheelMotion>(id));
	ctx.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
}

/// Bind a pending wheel tail to the smallest hovered scroll area, including its interaction layer.
pub fn claim_wheel_area(ui: &egui::Ui, target: egui::Id, area: Rect) {
	if !ui.rect_contains_pointer(area) {
		return;
	}
	let frame = ui.ctx().cumulative_frame_nr();
	let layer = ui.layer_id();
	let id = wheel_id(ui.ctx());
	ui.ctx().data_mut(|data| {
		let Some(mut motion) = data.get_temp::<WheelMotion>(id) else {
			return;
		};
		if motion.owner.is_none_or(|owner| {
			owner.frame != frame || owner.target == target || area.area() <= owner.area.area()
		}) {
			motion.owner = Some(WheelOwner {
				target,
				area,
				layer,
				frame,
			});
			data.insert_temp(id, motion);
		}
	});
}

/// Register a direct egui scroll area's content viewport, without the middle-click adapter.
pub fn claim_wheel_content(ui: &egui::Ui) {
	claim_wheel_area(ui, ui.scope_id(), ui.clip_rect());
}

fn normalized_wheel_delta(
	unit: egui::MouseWheelUnit,
	delta: egui::Vec2,
	modifiers: egui::Modifiers,
	options: egui::InputOptions,
	page_height: f32,
) -> egui::Vec2 {
	if modifiers.matches_any(options.zoom_modifier) || !delta.is_finite() {
		return egui::Vec2::ZERO;
	}
	let mut delta = match unit {
		egui::MouseWheelUnit::Point => delta,
		egui::MouseWheelUnit::Line => options.line_scroll_speed * delta,
		egui::MouseWheelUnit::Page => page_height * delta,
	};
	let horizontal = modifiers.matches_any(options.horizontal_scroll_modifier);
	let vertical = modifiers.matches_any(options.vertical_scroll_modifier);
	if horizontal && !vertical {
		delta = egui::vec2(delta.x + delta.y, 0.0);
	}
	if !horizontal && vertical {
		delta = egui::vec2(0.0, delta.x + delta.y);
	}
	if delta.is_finite() {
		delta
	} else {
		egui::Vec2::ZERO
	}
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
			if !matches!(phase, egui::TouchPhase::Start | egui::TouchPhase::Move)
				|| modifiers.matches_any(options.zoom_modifier)
			{
				return None;
			}
			Some(normalized_wheel_delta(
				*unit,
				*delta,
				*modifiers,
				options,
				page_height,
			))
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
		claim_wheel_area(ui, target, area);
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

	fn wheel(unit: egui::MouseWheelUnit, delta: egui::Vec2) -> egui::Event {
		egui::Event::MouseWheel {
			unit,
			delta,
			phase: egui::TouchPhase::Move,
			source: egui::MouseWheelSource::Unknown,
			modifiers: egui::Modifiers::NONE,
		}
	}

	#[test]
	fn unsmoothed_scrolling_keeps_the_initial_touchpad_delta() {
		let mut event = wheel(egui::MouseWheelUnit::Point, egui::vec2(0.0, -24.0));
		if let egui::Event::MouseWheel { phase, source, .. } = &mut event {
			*phase = egui::TouchPhase::Start;
			*source = egui::MouseWheelSource::Trackpad;
		}
		let preferences = model::ReadingPreferences {
			smooth_scrolling: false,
			..Default::default()
		};
		assert_eq!(
			step(
				&mut WheelMotion::default(),
				1.0,
				1.0 / 60.0,
				vec![event],
				preferences
			),
			egui::vec2(0.0, -24.0)
		);
	}

	fn step(
		motion: &mut WheelMotion,
		time: f64,
		dt: f32,
		events: Vec<egui::Event>,
		preferences: model::ReadingPreferences,
	) -> egui::Vec2 {
		let mut input = egui::InputState::default();
		input.time = time;
		input.stable_dt = dt;
		input.raw.events = events;
		input.focused = true;
		motion.update(
			&input,
			egui::InputOptions {
				line_scroll_speed: DISCORD_LINE_SCROLL_SPEED,
				..Default::default()
			},
			preferences,
		)
	}

	#[test]
	fn rapid_wheel_bursts_deliver_the_distance_and_finish_within_the_deadline() {
		for fps in [30_u32, 60, 144, 240] {
			let dt = 1.0 / f64::from(fps);
			let mut motion = WheelMotion::default();
			let mut total = 0.0;
			let mut expected = 0.0;
			let mut last = 0.0;
			for frame in 0..fps {
				let time = f64::from(frame) * dt;
				let events = if time < 0.3 {
					last = time;
					expected -= 12.0 * DISCORD_LINE_SCROLL_SPEED;
					vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -12.0))]
				} else {
					vec![]
				};
				total += step(&mut motion, time, dt as f32, events, Default::default()).y;
				if time >= last + WHEEL_SMOOTHING_SECONDS {
					assert_eq!(motion.pending, egui::Vec2::ZERO, "{fps} Hz at {time}");
				}
			}
			assert!(
				(total - expected).abs() < expected.abs() * 0.00001,
				"{fps} Hz: {total}/{expected}"
			);
		}
	}

	#[test]
	fn delayed_frame_drains_the_tail_instead_of_entering_a_slow_scroll_mode() {
		let mut motion = WheelMotion::default();
		let initial = step(
			&mut motion,
			1.0,
			1.0 / 60.0,
			vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -30.0))],
			Default::default(),
		);
		let remaining = step(&mut motion, 1.3, 0.3, vec![], Default::default());
		assert!((initial.y + remaining.y + 30.0 * DISCORD_LINE_SCROLL_SPEED).abs() < 0.001);
		assert_eq!(motion.pending, egui::Vec2::ZERO);
		assert_eq!(
			step(&mut motion, 1.4, 0.1, vec![], Default::default()),
			egui::Vec2::ZERO
		);
	}

	#[test]
	fn new_notch_cannot_extend_an_expired_tail() {
		let mut motion = WheelMotion::default();
		let notch = |lines| vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, lines))];
		let initial = step(
			&mut motion,
			1.0,
			1.0 / 60.0,
			notch(-30.0),
			Default::default(),
		);
		let after_delay = step(
			&mut motion,
			1.3,
			1.0 / 60.0,
			notch(-1.0),
			Default::default(),
		);
		assert!(initial.y + after_delay.y <= -30.0 * DISCORD_LINE_SCROLL_SPEED);
		assert!(motion.pending.y.abs() < DISCORD_LINE_SCROLL_SPEED);
		let final_tail = step(&mut motion, 1.5, 1.0 / 60.0, vec![], Default::default());
		assert!(
			(initial.y + after_delay.y + final_tail.y + 31.0 * DISCORD_LINE_SCROLL_SPEED).abs()
				< 0.001
		);
	}

	#[test]
	fn reversal_immediately_discards_the_opposite_tail() {
		let mut motion = WheelMotion::default();
		assert!(
			step(
				&mut motion,
				0.0,
				1.0 / 60.0,
				vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -50.0))],
				Default::default(),
			)
			.y < 0.0
		);
		assert!(
			step(
				&mut motion,
				0.01,
				0.01,
				vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, 1.0))],
				Default::default(),
			)
			.y > 0.0
		);
		assert!(motion.pending.y > 0.0);
	}

	#[test]
	fn precision_scrolls_are_immediate_and_do_not_gain_an_application_tail() {
		for source in [
			egui::MouseWheelSource::Unknown,
			egui::MouseWheelSource::Trackpad,
			egui::MouseWheelSource::Momentum,
		] {
			let mut motion = WheelMotion::default();
			let mut event = wheel(egui::MouseWheelUnit::Point, egui::vec2(2.5, -600.25));
			if let egui::Event::MouseWheel { source: value, .. } = &mut event {
				*value = source;
			}
			assert_eq!(
				step(
					&mut motion,
					0.0,
					1.0 / 60.0,
					vec![event],
					Default::default()
				),
				egui::vec2(2.5, -600.25)
			);
			assert_eq!(motion.pending, egui::Vec2::ZERO);
			assert_eq!(
				step(&mut motion, 0.02, 0.02, vec![], Default::default()),
				egui::Vec2::ZERO
			);
		}
	}

	#[test]
	fn smooth_off_speed_and_modifiers_apply_once() {
		let mut motion = WheelMotion::default();
		let mut event = wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -3.0));
		if let egui::Event::MouseWheel { modifiers, .. } = &mut event {
			*modifiers = egui::Modifiers::SHIFT;
		}
		assert_eq!(
			step(
				&mut motion,
				0.0,
				1.0 / 60.0,
				vec![event],
				model::ReadingPreferences {
					smooth_scrolling: false,
					scroll_speed_percent: 200,
					..Default::default()
				}
			),
			egui::vec2(-720.0, 0.0)
		);
		assert_eq!(motion.pending, egui::Vec2::ZERO);
		let options = egui::InputOptions::default();
		assert_eq!(
			instant_wheel_delta(
				&[wheel(egui::MouseWheelUnit::Page, egui::vec2(0.0, 2.0))],
				options,
				640.0,
			),
			egui::vec2(0.0, 1280.0)
		);
		assert_eq!(
			normalized_wheel_delta(
				egui::MouseWheelUnit::Line,
				egui::vec2(0.0, 10.0),
				options.zoom_modifier,
				options,
				640.0,
			),
			egui::Vec2::ZERO
		);
	}

	#[test]
	fn later_interaction_retires_wheel_input_from_the_same_frame() {
		let click = egui::Event::PointerButton {
			pos: egui::pos2(10.0, 10.0),
			button: egui::PointerButton::Primary,
			pressed: true,
			modifiers: egui::Modifiers::NONE,
		};
		let line = wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0));
		let mut touchpad = wheel(egui::MouseWheelUnit::Point, egui::vec2(0.0, -24.0));
		if let egui::Event::MouseWheel { source, .. } = &mut touchpad {
			*source = egui::MouseWheelSource::Trackpad;
		}
		for wheel in [line, touchpad] {
			for smooth_scrolling in [false, true] {
				let preferences = model::ReadingPreferences {
					smooth_scrolling,
					..Default::default()
				};
				let mut motion = WheelMotion::default();
				assert_eq!(
					step(
						&mut motion,
						0.0,
						1.0 / 60.0,
						vec![wheel.clone(), click.clone()],
						preferences
					),
					egui::Vec2::ZERO,
				);
				assert_eq!(motion.pending, egui::Vec2::ZERO);
				assert!(
					step(
						&mut motion,
						0.1,
						1.0 / 60.0,
						vec![click.clone(), wheel.clone()],
						preferences
					)
					.y < 0.0
				);
			}
		}
	}

	#[test]
	fn replacing_interactions_and_preference_changes_retire_the_tail() {
		for event in [
			egui::Event::PointerGone,
			egui::Event::PointerButton {
				pos: egui::pos2(10.0, 10.0),
				button: egui::PointerButton::Primary,
				pressed: true,
				modifiers: egui::Modifiers::NONE,
			},
			egui::Event::Key {
				key: egui::Key::PageDown,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			},
		] {
			let mut motion = WheelMotion::default();
			step(
				&mut motion,
				0.0,
				1.0 / 60.0,
				vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -10.0))],
				Default::default(),
			);
			assert_eq!(
				step(&mut motion, 0.01, 0.01, vec![event], Default::default()),
				egui::Vec2::ZERO
			);
			assert_eq!(motion.pending, egui::Vec2::ZERO);
		}
		let mut motion = WheelMotion::default();
		step(
			&mut motion,
			0.0,
			1.0 / 60.0,
			vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -10.0))],
			Default::default(),
		);
		assert_eq!(
			step(
				&mut motion,
				0.01,
				0.01,
				vec![],
				model::ReadingPreferences {
					scroll_speed_percent: 150,
					..Default::default()
				}
			),
			egui::Vec2::ZERO
		);
	}

	fn adapter_frame(
		ctx: &egui::Context,
		time: f64,
		events: Vec<egui::Event>,
		scope: egui::Id,
		focused: bool,
	) -> egui::Vec2 {
		adapter_frame_config(ctx, time, events, scope, focused, Default::default(), true)
	}

	fn adapter_frame_config(
		ctx: &egui::Context,
		time: f64,
		events: Vec<egui::Event>,
		scope: egui::Id,
		focused: bool,
		preferences: model::ReadingPreferences,
		claim: bool,
	) -> egui::Vec2 {
		let mut result = None;
		let output = ctx.run_ui(
			egui::RawInput {
				time: Some(time),
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0))),
				focused,
				events,
				..Default::default()
			},
			|ui| {
				apply_preferences_in(ctx, preferences, scope);
				if result.is_none() {
					result = Some(ui.input(|input| input.smooth_scroll_delta()));
				}
				if claim {
					claim_wheel_area(
						ui,
						egui::Id::unique("left-pane"),
						Rect::from_min_max(Pos2::ZERO, egui::pos2(400.0, 600.0)),
					);
					claim_wheel_area(
						ui,
						egui::Id::unique("right-pane"),
						Rect::from_min_max(egui::pos2(400.0, 0.0), egui::pos2(800.0, 600.0)),
					);
				}
			},
		);
		output.drop_without_applying_deltas();
		result.unwrap()
	}

	#[test]
	fn inactive_windows_accept_new_wheel_events_with_smoothing_on_or_off() {
		for smooth_scrolling in [true, false] {
			let ctx = egui::Context::default();
			ctx.options_mut(|options| {
				options.input_options.line_scroll_speed = DISCORD_LINE_SCROLL_SPEED
			});
			let scope = egui::Id::unique("inactive-window");
			let preferences = model::ReadingPreferences {
				smooth_scrolling,
				..Default::default()
			};
			adapter_frame_config(
				&ctx,
				0.0,
				vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))],
				scope,
				false,
				preferences,
				true,
			);
			let mut total = adapter_frame_config(
				&ctx,
				0.01,
				vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0))],
				scope,
				false,
				preferences,
				true,
			)
			.y;
			assert!(
				total < 0.0,
				"new inactive-window input must be applied immediately"
			);
			for frame in 2..=20 {
				total += adapter_frame_config(
					&ctx,
					f64::from(frame) * 0.01,
					vec![],
					scope,
					false,
					preferences,
					true,
				)
				.y;
			}
			assert!(
				(total + DISCORD_LINE_SCROLL_SPEED).abs() < 0.001,
				"inactive input lost distance: {total}"
			);
		}
	}

	#[test]
	fn focus_loss_retires_old_motion_without_rejecting_a_new_inactive_gesture() {
		let ctx = egui::Context::default();
		ctx.options_mut(|options| {
			options.input_options.line_scroll_speed = DISCORD_LINE_SCROLL_SPEED
		});
		let scope = egui::Id::unique("focus-change");
		adapter_frame(
			&ctx,
			0.0,
			vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))],
			scope,
			true,
		);
		adapter_frame(
			&ctx,
			0.01,
			vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -20.0))],
			scope,
			true,
		);
		assert_eq!(
			adapter_frame(&ctx, 0.02, vec![], scope, false),
			egui::Vec2::ZERO
		);
		let mut total = adapter_frame(
			&ctx,
			0.03,
			vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0))],
			scope,
			false,
		)
		.y;
		for frame in 4..=22 {
			total += adapter_frame(&ctx, f64::from(frame) * 0.01, vec![], scope, false).y;
		}
		assert!(
			(total + DISCORD_LINE_SCROLL_SPEED).abs() < 0.001,
			"old tail contaminated inactive gesture: {total}"
		);
	}

	#[test]
	fn unclaimed_same_layer_pointer_movement_does_not_cancel_the_wheel_tail() {
		let ctx = egui::Context::default();
		ctx.options_mut(|options| {
			options.input_options.line_scroll_speed = DISCORD_LINE_SCROLL_SPEED
		});
		let scope = egui::Id::unique("unclaimed-scroll-area");
		adapter_frame_config(
			&ctx,
			0.0,
			vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))],
			scope,
			true,
			Default::default(),
			false,
		);
		let mut total = adapter_frame_config(
			&ctx,
			0.01,
			vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0))],
			scope,
			true,
			Default::default(),
			false,
		)
		.y;
		let moved = adapter_frame_config(
			&ctx,
			0.02,
			vec![egui::Event::PointerMoved(egui::pos2(110.0, 120.0))],
			scope,
			true,
			Default::default(),
			false,
		)
		.y;
		assert!(
			moved < 0.0,
			"small movement inside the same layer must retain smoothing"
		);
		total += moved;
		for frame in 3..=20 {
			total += adapter_frame_config(
				&ctx,
				f64::from(frame) * 0.01,
				vec![],
				scope,
				true,
				Default::default(),
				false,
			)
			.y;
		}
		assert!((total + DISCORD_LINE_SCROLL_SPEED).abs() < 0.001);
	}

	#[test]
	fn adapter_does_not_replay_egui_tail_after_navigation_focus_loss_or_pane_changes() {
		for replacement in 0..4 {
			let ctx = egui::Context::default();
			let original_scope = egui::Id::unique("original-channel");
			adapter_frame(
				&ctx,
				0.0,
				vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))],
				original_scope,
				true,
			);
			assert!(
				adapter_frame(
					&ctx,
					0.01,
					vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -20.0))],
					original_scope,
					true
				)
				.y < 0.0
			);
			let (events, scope, focused) = match replacement {
				0 => (vec![], egui::Id::unique("new-channel"), true),
				1 => (vec![], original_scope, false),
				2 => (
					vec![egui::Event::PointerMoved(egui::pos2(650.0, 100.0))],
					original_scope,
					true,
				),
				_ => {
					reset_wheel(&ctx);
					(vec![], original_scope, true)
				}
			};
			assert_eq!(
				adapter_frame(&ctx, 0.03, events, scope, focused),
				egui::Vec2::ZERO,
				"replacement {replacement}"
			);
			assert_eq!(
				adapter_frame(&ctx, 0.05, vec![], scope, true),
				egui::Vec2::ZERO,
				"egui tail leaked after replacement {replacement}"
			);
		}
	}

	#[test]
	fn a_fresh_wheel_in_another_pane_does_not_inherit_the_previous_distance() {
		let ctx = egui::Context::default();
		ctx.options_mut(|options| {
			options.input_options.line_scroll_speed = DISCORD_LINE_SCROLL_SPEED
		});
		let scope = egui::Id::unique("conversation");
		adapter_frame(
			&ctx,
			0.0,
			vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))],
			scope,
			true,
		);
		adapter_frame(
			&ctx,
			0.01,
			vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -20.0))],
			scope,
			true,
		);
		let mut total = adapter_frame(
			&ctx,
			0.02,
			vec![
				egui::Event::PointerMoved(egui::pos2(650.0, 100.0)),
				wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0)),
			],
			scope,
			true,
		)
		.y;
		for frame in 3..=20 {
			total += adapter_frame(&ctx, f64::from(frame) * 0.01, vec![], scope, true).y;
		}
		assert!(
			(total + DISCORD_LINE_SCROLL_SPEED).abs() < 0.001,
			"new pane received {total}"
		);
	}

	#[test]
	fn a_foreground_overlay_retires_motion_in_the_covered_pane() {
		for claim in [true, false] {
			let ctx = egui::Context::default();
			let scope = egui::Id::unique("conversation");
			adapter_frame_config(
				&ctx,
				0.0,
				vec![egui::Event::PointerMoved(egui::pos2(100.0, 100.0))],
				scope,
				true,
				Default::default(),
				claim,
			);
			adapter_frame_config(
				&ctx,
				0.01,
				vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, -20.0))],
				scope,
				true,
				Default::default(),
				claim,
			);
			// A newly created Area has a sizing pass before its final interaction rectangle.
			for time in [0.02, 0.03] {
				let output = ctx.run_ui(
					egui::RawInput {
						time: Some(time),
						screen_rect: Some(Rect::from_min_size(
							Pos2::ZERO,
							egui::vec2(800.0, 600.0),
						)),
						..Default::default()
					},
					|_ui| {
						egui::Area::new(egui::Id::unique("covering-menu"))
							.order(egui::Order::Foreground)
							.fixed_pos(egui::pos2(20.0, 20.0))
							.show(&ctx, |ui| {
								ui.allocate_space(egui::vec2(250.0, 250.0));
							});
					},
				);
				output.drop_without_applying_deltas();
			}
			assert_eq!(
				adapter_frame_config(&ctx, 0.04, vec![], scope, true, Default::default(), claim),
				egui::Vec2::ZERO,
				"claimed={claim}"
			);
		}
	}

	struct TimelineScenario {
		ctx: egui::Context,
		state: client_core::State,
		view: crate::timeline::TimelineView,
		avatars: crate::avatars::Avatars,
		profile: crate::profiles::ProfileSession,
		session: Session,
		width: f32,
		time: f64,
	}

	struct TimelineFrame {
		labels: std::collections::BTreeMap<String, f32>,
		applied: egui::Vec2,
	}

	impl TimelineScenario {
		fn new(width: f32) -> Self {
			let ctx = egui::Context::default();
			crate::design::apply(&ctx);
			ctx.options_mut(|options| {
				options.input_options.line_scroll_speed = DISCORD_LINE_SCROLL_SPEED
			});
			let mut state = client_core::State {
				selected: Some(model::Id(20)),
				revision: 1,
				demo: true,
				freshness: model::Freshness::Fresh,
				..Default::default()
			};
			for id in 1..=500 {
				let mut message = test_support::message(id, model::Id(20));
				message.author.cosmetics = None;
				message.author.primary_guild = None;
				message.reactions = Some(vec![]);
				message.content = format!("Scroll row {id}: synthetic scrolling content.");
				if id % 7 == 0 {
					message
						.content
						.push_str(&" Long wrapping content.".repeat(24));
				}
				state.timeline.insert(message, false, false).unwrap();
			}
			// Restore the middle of a conversation through the shipping reading-position interface.
			state.remember_reading(
				model::Id(20),
				client_core::ReadingCursor {
					message: Some(model::Id(200)),
					inset: 5.0,
				},
			);
			Self {
				ctx,
				state,
				view: Default::default(),
				avatars: Default::default(),
				profile: Default::default(),
				session: Default::default(),
				width,
				time: 0.0,
			}
		}

		fn frame(&mut self, lines: f32) -> TimelineFrame {
			fn texts(shape: &egui::Shape, labels: &mut std::collections::BTreeMap<String, f32>) {
				match shape {
					egui::Shape::Text(text) if text.galley.job.text.starts_with("Scroll row ") => {
						labels.insert(text.galley.job.text.clone(), text.pos.y);
					}
					egui::Shape::Vec(shapes) => {
						for shape in shapes {
							texts(shape, labels);
						}
					}
					_ => {}
				}
			}
			self.time += 1.0 / 60.0;
			let Self {
				ctx,
				state,
				view,
				avatars,
				profile,
				session,
				width,
				time,
			} = self;
			let mut events = vec![egui::Event::PointerMoved(egui::pos2(150.0, 200.0))];
			if lines != 0.0 {
				events.push(wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, lines)));
			}
			let mut applied = None;
			let output = ctx.run_ui(
				egui::RawInput {
					time: Some(*time),
					screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(*width, 600.0))),
					events,
					..Default::default()
				},
				|ui| {
					apply_preferences_in(
						ctx,
						Default::default(),
						egui::Id::unique((state.generation, state.selected)),
					);
					if applied.is_none() {
						applied = Some(ui.input(|input| input.smooth_scroll_delta()));
					}
					view.show_with_scroll(
						ui,
						state,
						&mut None,
						&mut None,
						(&mut *avatars, &mut *profile),
						None,
						session,
					);
				},
			);
			let mut labels = std::collections::BTreeMap::new();
			for shape in &output.shapes {
				if shape.clip_rect.is_positive() {
					texts(&shape.shape, &mut labels);
				}
			}
			output.drop_without_applying_deltas();
			TimelineFrame {
				labels,
				applied: applied.unwrap(),
			}
		}
	}

	#[test]
	fn smooth_line_bursts_render_without_timeline_bounce_and_interrupt_a_jump() {
		for width in [900.0, 360.0] {
			let mut scenario = TimelineScenario::new(width);
			for _ in 0..12 {
				scenario.frame(0.0);
			}
			assert!(
				!scenario.view.instant_scrolling,
				"exercise default animated scrolling"
			);
			let mut previous = scenario.frame(0.0);
			let mut comparisons = 0;
			let mut worst_error = 0.0_f32;
			let mut last_input = scenario.time;
			for (lines, count) in [(-3.0, 12), (0.0, 12), (2.0, 8), (-1.0, 6), (0.0, 20)] {
				for _ in 0..count {
					let current = scenario.frame(lines);
					if lines != 0.0 {
						last_input = scenario.time;
					}
					// egui applies wheel movement after painting; the next layout presents that delta.
					for (text, y) in &current.labels {
						if (50.0..500.0).contains(y)
							&& let Some(before) = previous.labels.get(text)
						{
							worst_error = worst_error.max((y - before - previous.applied.y).abs());
							comparisons += 1;
						}
					}
					if scenario.time > last_input + WHEEL_SMOOTHING_SECONDS + 1.0 / 60.0 {
						assert_eq!(
							current.applied,
							egui::Vec2::ZERO,
							"wheel tail outlived deadline"
						);
					}
					previous = current;
				}
			}
			assert!(
				comparisons >= 58,
				"need shared visible rows throughout the burst workload"
			);
			assert!(
				worst_error < 1.0,
				"width={width}: timeline bounced by {worst_error:.3}pt"
			);

			reset_wheel(&scenario.ctx);
			scenario.state.search_target = Some(model::Id(25));
			scenario.frame(0.0); // Starts the production target-reveal animation.
			assert!(scenario.state.search_target.is_none());
			scenario.frame(-1.0); // Manual input must interrupt the reveal on this frame.
			for _ in 0..30 {
				scenario.frame(0.0);
			}
			let stopped = scenario.frame(0.0);
			let stable = scenario.frame(0.0);
			assert_eq!(stopped.applied, egui::Vec2::ZERO);
			assert_eq!(
				stopped.labels, stable.labels,
				"cancelled jump continued moving the viewport"
			);
			assert!(
				!stable.labels.iter().any(|(text, y)| {
					text.starts_with("Scroll row 25:") && (0.0..600.0).contains(y)
				}),
				"the cancelled animation still reached its target"
			);
		}
	}

	#[test]
	fn invalid_or_overflowing_input_cannot_create_nonfinite_movement() {
		let mut motion = WheelMotion::default();
		for delta in [f32::NAN, f32::INFINITY, f32::MAX] {
			assert_eq!(
				step(
					&mut motion,
					0.0,
					1.0 / 60.0,
					vec![wheel(egui::MouseWheelUnit::Line, egui::vec2(0.0, delta))],
					Default::default()
				),
				egui::Vec2::ZERO
			);
		}
		assert_eq!(
			step(
				&mut motion,
				0.0,
				1.0 / 60.0,
				vec![wheel(
					egui::MouseWheelUnit::Point,
					egui::vec2(f32::MAX, f32::MAX)
				)],
				model::ReadingPreferences {
					smooth_scrolling: false,
					scroll_speed_percent: 300,
					..Default::default()
				}
			),
			egui::Vec2::ZERO
		);
	}
}
