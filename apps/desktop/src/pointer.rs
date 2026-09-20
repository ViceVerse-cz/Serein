use eframe::egui::{Event, PointerButton, Pos2, RawInput, pos2};
use ui::scroll::Middle;
use winit::window::Window;

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
	/// Remove the middle button from `events` and return what the autoscroll session needs.
	/// When `track`, append the OS cursor as a `PointerMoved` so a cursor that has left the
	/// window keeps reporting its distance from the drive origin.
	pub fn intercept(
		&mut self,
		raw: &mut RawInput,
		window: &Window,
		pixels_per_point: f32,
		track: bool,
	) -> Middle {
		let mut middle = Middle::default();
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
		middle
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
