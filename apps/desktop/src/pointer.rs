use eframe::egui::{Event, PointerButton, Pos2, RawInput, pos2};
use ui::scroll::{Middle, SidePress};
use winit::window::Window;

/// Middle and side-button edges stripped from egui input for one frame.
pub struct Intercepted {
	pub middle: Middle,
	pub side: SidePress,
}

/// The window's pointer, translated for Serein.
///
/// Holds the two facts `RawInput` cannot carry across frames: whether the middle button is
/// still down after a frame with no events, and the last position the cursor was seen at.
#[derive(Default)]
pub struct Pointer {
	down: bool,
	last: Option<Pos2>,
}

impl Pointer {
	/// Remove middle / Extra1 / Extra2 from `events` and return what scroll and side-nav need.
	/// When `track`, append the OS cursor as a `PointerMoved` so a cursor that has left the
	/// window keeps reporting its distance from the drive origin.
	pub fn intercept(
		&mut self,
		raw: &mut RawInput,
		window: &Window,
		pixels_per_point: f32,
		track: bool,
	) -> Intercepted {
		let mut middle = Middle::default();
		let mut side = SidePress::default();
		raw.events.retain(|event| match event {
			Event::PointerButton {
				pos,
				button: PointerButton::Middle,
				pressed,
				..
			} => {
				if *pressed {
					middle.pressed.get_or_insert(*pos);
				}
				self.down = *pressed;
				false
			}
			Event::PointerButton {
				button: PointerButton::Extra1,
				pressed,
				..
			} => {
				if *pressed {
					side.back = true;
				}
				false
			}
			Event::PointerButton {
				button: PointerButton::Extra2,
				pressed,
				..
			} => {
				if *pressed {
					side.forward = true;
				}
				false
			}
			Event::PointerMoved(pos) => {
				self.last = Some(*pos);
				true
			}
			_ => true,
		});
		middle.down = self.down;
		if track
			&& let Some(at) = Self::client_cursor(window, pixels_per_point)
			&& self.last != Some(at)
		{
			// Appended last so a same-batch `PointerGone` from `CursorLeft` is restored.
			self.last = Some(at);
			raw.events.push(Event::PointerMoved(at));
		}
		Intercepted { middle, side }
	}

	fn client_cursor(window: &Window, pixels_per_point: f32) -> Option<Pos2> {
		let (x, y) = platform::cursor_position()?;
		let origin = window.inner_position().ok()?;
		Some(pos2(
			(x - f64::from(origin.x)) as f32 / pixels_per_point,
			(y - f64::from(origin.y)) as f32 / pixels_per_point,
		))
	}
}
