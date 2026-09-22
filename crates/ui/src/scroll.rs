use crate::design;
use egui::{AsIdSalt, IdSalt, Pos2, Rect, ScrollArea, Shape, Stroke, pos2};

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
		let next = (state.offset.y - delta).max(0.0);
		if (next - state.offset.y).abs() < f32::EPSILON {
			return builder;
		}
		if let Some((id, last)) = self.last_offset
			&& id == target
			&& clamped_away(last, state.offset.y, next)
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
