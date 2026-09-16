//! Native global shortcut registration. Wayland intentionally falls back to focused input
//! because the compositor, not applications, owns global keyboard observation there.
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use model::KeyChord;

const READY: &str = "Global Push to Talk is enabled.";
const WAYLAND: &str = "Global shortcuts are unavailable on Wayland; Push to Talk works while Serein is focused.";
const UNAVAILABLE: &str = "Global shortcuts are unavailable on this system; Push to Talk works while Serein is focused.";
const INVALID: &str = "This Push to Talk binding cannot be registered globally; it still works while Serein is focused.";
const MODIFIER_REQUIRED: &str = "Add Ctrl, Alt, Shift, or Command to Push to Talk for global use; it works focused without one.";

pub struct Hotkeys {
	manager: Option<GlobalHotKeyManager>,
	registered: Option<HotKey>,
	ptt_down: bool,
	status: &'static str,
}

impl Hotkeys {
	pub fn new() -> Self {
		if cfg!(target_os = "linux")
			&& std::env::var_os("WAYLAND_DISPLAY").is_some()
			&& std::env::var_os("DISPLAY").is_none()
		{
			return Self {
				manager: None,
				registered: None,
				ptt_down: false,
				status: WAYLAND,
			};
		}
		match GlobalHotKeyManager::new() {
			Ok(manager) => Self {
				manager: Some(manager),
				registered: None,
				ptt_down: false,
				status: READY,
			},
			Err(_) => Self {
				manager: None,
				registered: None,
				ptt_down: false,
				status: if cfg!(target_os = "linux")
					&& std::env::var_os("WAYLAND_DISPLAY").is_some()
				{
					WAYLAND
				} else {
					UNAVAILABLE
				},
			},
		}
	}

	pub fn sync(&mut self, chord: &KeyChord) {
		let Some(manager) = &self.manager else { return };
		let next = native_hotkey(chord);
		if next.is_none() {
			if let Some(previous) = self.registered.take() {
				let _ = manager.unregister(previous);
			}
			self.status = if chord.modifiers == 0 { MODIFIER_REQUIRED } else { INVALID };
			return;
		}
		let Some(next) = next else {
			self.status = INVALID;
			return;
		};
		if self.registered == Some(next) {
			return;
		}
		if let Some(previous) = self.registered.take() {
			let _ = manager.unregister(previous);
		}
		match manager.register(next) {
			Ok(()) => {
				self.registered = Some(next);
				self.status = READY;
			}
			Err(_) => self.status = UNAVAILABLE,
		}
	}

	pub fn poll(&mut self) {
		let Some(hotkey) = self.registered else {
			self.ptt_down = false;
			return;
		};
		while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
			if event.id() == hotkey.id() {
				self.ptt_down = event.state() == HotKeyState::Pressed;
			}
		}
	}

	pub fn push_to_talk_down(&self) -> bool {
		self.ptt_down
	}

	pub fn status(&self) -> &'static str {
		self.status
	}
}

fn native_hotkey(chord: &KeyChord) -> Option<HotKey> {
	if !chord.is_valid() {
		return None;
	}
	let mut value = String::new();
	if chord.modifiers & model::keybinds::PRIMARY != 0 {
		value.push_str(if cfg!(target_os = "macos") { "super+" } else { "control+" });
	}
	if chord.modifiers & model::keybinds::CTRL != 0 { value.push_str("control+"); }
	if chord.modifiers & model::keybinds::ALT != 0 { value.push_str("alt+"); }
	if chord.modifiers & model::keybinds::SHIFT != 0 { value.push_str("shift+"); }
	value.push_str(code_name(&chord.key)?);
	value.parse().ok()
}

fn code_name(name: &str) -> Option<&'static str> {
	match name {
		"ArrowDown" => Some("ArrowDown"), "ArrowLeft" => Some("ArrowLeft"), "ArrowRight" => Some("ArrowRight"), "ArrowUp" => Some("ArrowUp"),
		"Escape" => Some("Escape"), "Tab" => Some("Tab"), "Backspace" => Some("Backspace"), "Enter" => Some("Enter"), "Space" => Some("Space"),
		"Delete" => Some("Delete"), "Home" => Some("Home"), "End" => Some("End"), "Slash" => Some("Slash"), "Backtick" => Some("Backquote"), "Minus" => Some("Minus"), "Equals" => Some("Equal"), "Comma" => Some("Comma"), "Period" => Some("Period"),
		"Num0" => Some("Digit0"), "Num1" => Some("Digit1"), "Num2" => Some("Digit2"), "Num3" => Some("Digit3"), "Num4" => Some("Digit4"), "Num5" => Some("Digit5"), "Num6" => Some("Digit6"), "Num7" => Some("Digit7"), "Num8" => Some("Digit8"), "Num9" => Some("Digit9"),
		"A" => Some("KeyA"), "B" => Some("KeyB"), "C" => Some("KeyC"), "D" => Some("KeyD"), "E" => Some("KeyE"), "F" => Some("KeyF"), "G" => Some("KeyG"), "H" => Some("KeyH"), "I" => Some("KeyI"), "J" => Some("KeyJ"), "K" => Some("KeyK"), "L" => Some("KeyL"), "M" => Some("KeyM"), "N" => Some("KeyN"), "O" => Some("KeyO"), "P" => Some("KeyP"), "Q" => Some("KeyQ"), "R" => Some("KeyR"), "S" => Some("KeyS"), "T" => Some("KeyT"), "U" => Some("KeyU"), "V" => Some("KeyV"), "W" => Some("KeyW"), "X" => Some("KeyX"), "Y" => Some("KeyY"), "Z" => Some("KeyZ"),
		"F1" => Some("F1"), "F2" => Some("F2"), "F3" => Some("F3"), "F4" => Some("F4"), "F5" => Some("F5"), "F6" => Some("F6"), "F7" => Some("F7"), "F8" => Some("F8"), "F9" => Some("F9"), "F10" => Some("F10"), "F11" => Some("F11"), "F12" => Some("F12"),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn default_push_to_talk_has_a_native_code() {
		assert!(native_hotkey(&KeyChord::default()).is_some());
		assert!(native_hotkey(&KeyChord::new("unknown", 0)).is_none());
	}
}
