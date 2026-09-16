//! Native global voice bindings. Wayland intentionally falls back to focused input because the
//! compositor, not applications, owns global keyboard observation there.
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use model::{KeyChord, KeybindAction, Keybinds};

const READY: &str = "Global voice keybinds are enabled.";
const WAYLAND: &str =
	"Global voice keybinds are unavailable on Wayland; they work while Serein is focused.";
const UNAVAILABLE: &str =
	"Global voice keybinds are unavailable on this system; they work while Serein is focused.";
const INVALID: &str = "One or more voice bindings cannot be registered globally; they still work while Serein is focused.";
const MODIFIER_REQUIRED: &str = "Add Ctrl, Alt, Shift, or Command to use a voice binding globally; it still works while Serein is focused.";

const PUSH_TO_TALK: usize = 0;
const TOGGLE_MUTE: usize = 1;
const TOGGLE_DEAFEN: usize = 2;

pub struct Hotkeys {
	manager: Option<GlobalHotKeyManager>,
	registered: [Option<HotKey>; 3],
	bindings: Option<[KeyChord; 3]>,
	ptt_down: bool,
	pending_toggles: u8,
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
				registered: [None; 3],
				bindings: None,
				ptt_down: false,
				pending_toggles: 0,
				status: WAYLAND,
			};
		}
		match GlobalHotKeyManager::new() {
			Ok(manager) => Self {
				manager: Some(manager),
				registered: [None; 3],
				bindings: None,
				ptt_down: false,
				pending_toggles: 0,
				status: READY,
			},
			Err(_) => Self {
				manager: None,
				registered: [None; 3],
				bindings: None,
				ptt_down: false,
				pending_toggles: 0,
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

	pub fn sync(&mut self, keybinds: &Keybinds) {
		let next = [
			keybinds.chord(KeybindAction::PushToTalk).clone(),
			keybinds.chord(KeybindAction::ToggleMute).clone(),
			keybinds.chord(KeybindAction::ToggleDeafen).clone(),
		];
		if self.bindings.as_ref() == Some(&next) {
			return;
		}
		self.bindings = Some(next.clone());
		self.unregister_all();
		self.ptt_down = false;
		self.pending_toggles = 0;

		let Some(manager) = &self.manager else {
			return;
		};
		let mut failed = false;
		let mut modifier_required = false;
		for (index, chord) in next.iter().enumerate() {
			if !chord.is_valid() {
				failed = true;
				continue;
			}
			if chord.modifiers == 0 {
				modifier_required |= index != PUSH_TO_TALK;
				continue;
			}
			let Some(hotkey) = native_hotkey(chord) else {
				failed = true;
				continue;
			};
			match manager.register(hotkey) {
				Ok(()) => self.registered[index] = Some(hotkey),
				Err(_) => failed = true,
			}
		}
		if failed {
			self.status = INVALID;
		} else if modifier_required && self.global_toggle_mask() == 0 {
			self.status = MODIFIER_REQUIRED;
		} else {
			self.status = READY;
		}
	}

	fn unregister_all(&mut self) {
		if let Some(manager) = &self.manager {
			for hotkey in &mut self.registered {
				if let Some(hotkey) = hotkey.take() {
					let _ = manager.unregister(hotkey);
				}
			}
		} else {
			self.registered = [None; 3];
		}
	}

	pub fn poll(&mut self) {
		while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
			for (index, hotkey) in self.registered.iter().enumerate() {
				if hotkey.is_some_and(|hotkey| hotkey.id() == event.id()) {
					match (index, event.state()) {
						(PUSH_TO_TALK, HotKeyState::Pressed) => self.ptt_down = true,
						(PUSH_TO_TALK, HotKeyState::Released) => self.ptt_down = false,
						(TOGGLE_MUTE, HotKeyState::Pressed) => self.pending_toggles |= 1,
						(TOGGLE_DEAFEN, HotKeyState::Pressed) => self.pending_toggles |= 2,
						_ => {}
					}
				}
			}
		}
	}

	pub fn take_toggle_pending(&mut self) -> u8 {
		std::mem::take(&mut self.pending_toggles)
	}

	/// Bits for mute/deafen bindings currently owned by the native global registrar.
	pub fn global_toggle_mask(&self) -> u8 {
		(self.registered[TOGGLE_MUTE].is_some() as u8)
			| ((self.registered[TOGGLE_DEAFEN].is_some() as u8) << 1)
	}

	pub fn push_to_talk_down(&self) -> bool {
		self.ptt_down
	}

	pub fn status(&self) -> &'static str {
		self.status
	}
}

impl Drop for Hotkeys {
	fn drop(&mut self) {
		self.unregister_all();
	}
}

fn native_hotkey(chord: &KeyChord) -> Option<HotKey> {
	if !chord.is_valid() || chord.modifiers == 0 {
		return None;
	}
	let mut value = String::new();
	if chord.modifiers & model::keybinds::PRIMARY != 0 {
		value.push_str(if cfg!(target_os = "macos") {
			"super+"
		} else {
			"control+"
		});
	}
	if chord.modifiers & model::keybinds::CTRL != 0 {
		value.push_str("control+");
	}
	if chord.modifiers & model::keybinds::ALT != 0 {
		value.push_str("alt+");
	}
	if chord.modifiers & model::keybinds::SHIFT != 0 {
		value.push_str("shift+");
	}
	value.push_str(code_name(&chord.key)?);
	value.parse().ok()
}

fn code_name(name: &str) -> Option<&'static str> {
	match name {
		"ArrowDown" => Some("ArrowDown"),
		"ArrowLeft" => Some("ArrowLeft"),
		"ArrowRight" => Some("ArrowRight"),
		"ArrowUp" => Some("ArrowUp"),
		"Escape" => Some("Escape"),
		"Tab" => Some("Tab"),
		"Backspace" => Some("Backspace"),
		"Enter" => Some("Enter"),
		"Space" => Some("Space"),
		"Delete" => Some("Delete"),
		"Home" => Some("Home"),
		"End" => Some("End"),
		"Slash" => Some("Slash"),
		"Backtick" => Some("Backquote"),
		"Minus" => Some("Minus"),
		"Equals" => Some("Equal"),
		"Comma" => Some("Comma"),
		"Period" => Some("Period"),
		"Num0" => Some("Digit0"),
		"Num1" => Some("Digit1"),
		"Num2" => Some("Digit2"),
		"Num3" => Some("Digit3"),
		"Num4" => Some("Digit4"),
		"Num5" => Some("Digit5"),
		"Num6" => Some("Digit6"),
		"Num7" => Some("Digit7"),
		"Num8" => Some("Digit8"),
		"Num9" => Some("Digit9"),
		"A" => Some("KeyA"),
		"B" => Some("KeyB"),
		"C" => Some("KeyC"),
		"D" => Some("KeyD"),
		"E" => Some("KeyE"),
		"F" => Some("KeyF"),
		"G" => Some("KeyG"),
		"H" => Some("KeyH"),
		"I" => Some("KeyI"),
		"J" => Some("KeyJ"),
		"K" => Some("KeyK"),
		"L" => Some("KeyL"),
		"M" => Some("KeyM"),
		"N" => Some("KeyN"),
		"O" => Some("KeyO"),
		"P" => Some("KeyP"),
		"Q" => Some("KeyQ"),
		"R" => Some("KeyR"),
		"S" => Some("KeyS"),
		"T" => Some("KeyT"),
		"U" => Some("KeyU"),
		"V" => Some("KeyV"),
		"W" => Some("KeyW"),
		"X" => Some("KeyX"),
		"Y" => Some("KeyY"),
		"Z" => Some("KeyZ"),
		"F1" => Some("F1"),
		"F2" => Some("F2"),
		"F3" => Some("F3"),
		"F4" => Some("F4"),
		"F5" => Some("F5"),
		"F6" => Some("F6"),
		"F7" => Some("F7"),
		"F8" => Some("F8"),
		"F9" => Some("F9"),
		"F10" => Some("F10"),
		"F11" => Some("F11"),
		"F12" => Some("F12"),
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn modifier_bindings_have_native_codes_and_plain_keys_stay_focused() {
		assert!(
			native_hotkey(&KeyChord::new(
				"M",
				model::keybinds::PRIMARY | model::keybinds::SHIFT
			))
			.is_some()
		);
		assert!(native_hotkey(&KeyChord::default()).is_none());
		assert!(native_hotkey(&KeyChord::new("unknown", model::keybinds::PRIMARY)).is_none());
	}
}
