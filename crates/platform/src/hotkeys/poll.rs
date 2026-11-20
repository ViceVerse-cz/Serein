//! Global voice keys read from the physical key state instead of an OS hotkey registration.
//! Registering a plain key or mouse button would take it from every other app, so like
//! Discord's input hook this only watches: the key still reaches the focused app.
use super::NativeInput;
use model::keybinds::{ALT, CTRL, PRIMARY, SHIFT, is_mouse_button};
use model::{KeyChord, KeybindAction};
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

/// Poller identities, kept out of the range `global-hotkey` assigns to real hotkeys.
const ID: u32 = 1 << 31;
/// Well under an ordinary key press or click, which stays down for tens of milliseconds.
const CALL_PERIOD: Duration = Duration::from_millis(10);
/// Outside a call only mute and deafen toggles matter, and a tap still spans a few polls.
const IDLE_PERIOD: Duration = Duration::from_millis(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Modifiers {
	pub ctrl: bool,
	pub shift: bool,
	pub alt: bool,
	/// Command on macOS, Windows or Super elsewhere.
	pub meta: bool,
}

/// Whether this platform can watch `name` at all.
pub(super) fn supported(name: &str) -> bool {
	os::supported(name)
}

/// Whether macOS lets Serein read keys typed into other apps (Input Monitoring). With `ask`,
/// the first call in a process shows the system prompt, which also lists Serein in System
/// Settings; Discord asks when a push-to-talk call starts too.
#[cfg(target_os = "macos")]
pub(super) fn listen_access(ask: bool) -> bool {
	static ASKED: AtomicBool = AtomicBool::new(false);
	os::listen_access(ask && !ASKED.swap(true, Ordering::Relaxed))
}

/// Shared with the host so the poll thread follows focus and call state without a resync.
#[derive(Clone, Default)]
pub(super) struct Flags {
	pub focused: Arc<AtomicBool>,
	pub in_call: Arc<AtomicBool>,
}

/// One chord's physical keys, grouped with every other chord of the same action and key so
/// modifier variants fold into one press, and a toggle fires once.
struct Watch<C> {
	group: usize,
	codes: Vec<C>,
	modifiers: u8,
	mouse: bool,
}

pub(super) struct Poller {
	stop: Arc<AtomicBool>,
	thread: Option<std::thread::JoinHandle<()>>,
	native: Arc<NativeInput>,
	chords: Vec<(KeybindAction, KeyChord)>,
}

impl Poller {
	pub(super) fn start(
		bindings: Vec<(KeybindAction, KeyChord)>,
		native: Arc<NativeInput>,
		flags: Flags,
	) -> Option<Self> {
		if bindings.is_empty() {
			return None;
		}
		let mut groups: Vec<(KeybindAction, String)> = Vec::new();
		let mut chords = Vec::with_capacity(bindings.len());
		for (action, chord) in &bindings {
			if !groups
				.iter()
				.any(|(group, key)| group == action && *key == chord.key)
			{
				groups.push((*action, chord.key.clone()));
			}
			chords.push((*action, chord.clone()));
		}
		let stop = Arc::new(AtomicBool::new(false));
		let stopped = stop.clone();
		let target = native.clone();
		let thread = std::thread::Builder::new()
			.name("serein-voice-keys".into())
			.spawn(move || {
				// X11 needs its own display connection, which must stay on this thread.
				let Some(mut keys) = os::Keys::open() else {
					return;
				};
				let watched: Vec<_> = bindings
					.iter()
					.filter_map(|(action, chord)| {
						let codes = keys.codes(&chord.key);
						(!codes.is_empty()).then(|| Watch {
							group: groups
								.iter()
								.position(|(group, key)| group == action && *key == chord.key)
								.expect("every chord has a group"),
							codes,
							modifiers: chord.modifiers,
							mouse: is_mouse_button(&chord.key),
						})
					})
					.collect();
				let toggle = |action: KeybindAction| {
					matches!(
						action,
						KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
					)
				};
				let mut tracker =
					Tracker::new(groups.iter().map(|(action, _)| toggle(*action)).collect());
				let mut first = true;
				while !stopped.load(Ordering::Relaxed) {
					keys.refresh();
					let modifiers = keys.modifiers();
					let focused = flags.focused.load(Ordering::Relaxed);
					let inputs = step(
						&watched,
						groups.len(),
						focused,
						|code| keys.down(code),
						modifiers,
					);
					// A key already down when polling starts (a resync mid-press) keeps a hold
					// going, but must not replay a toggle.
					let seeded: Vec<_> = inputs
						.iter()
						.enumerate()
						.map(|(group, input)| Input {
							eligible: input.eligible && !(first && toggle(groups[group].0)),
							..*input
						})
						.collect();
					first = false;
					for (group, now) in tracker.advance(&seeded) {
						target.apply(ID + group as u32, groups[group].0, now);
					}
					std::thread::sleep(if flags.in_call.load(Ordering::Relaxed) {
						CALL_PERIOD
					} else {
						IDLE_PERIOD
					});
				}
			})
			.ok()?;
		Some(Self {
			stop,
			thread: Some(thread),
			native,
			chords,
		})
	}

	/// Whether this poller owns the chord even while Serein is focused. Keyboard keys are left
	/// to focused input there, which knows when the user is typing.
	pub(super) fn owns_focused(&self, action: KeybindAction, chord: &KeyChord) -> bool {
		is_mouse_button(&chord.key)
			&& self
				.chords
				.iter()
				.any(|(watched, current)| *watched == action && current == chord)
	}

	/// Mute/deafen bits for mouse toggles, so focused input does not toggle them twice.
	pub(super) fn toggle_mask(&self) -> u8 {
		self.chords
			.iter()
			.filter(|(_, chord)| is_mouse_button(&chord.key))
			.fold(0, |mask, (action, _)| {
				mask | match action {
					KeybindAction::ToggleMute => 1,
					KeybindAction::ToggleDeafen => 2,
					_ => 0,
				}
			})
	}
}

impl Drop for Poller {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::Relaxed);
		if let Some(thread) = self.thread.take() {
			let _ = thread.join();
		}
		// A key held while the bindings changed must not keep push-to-talk open.
		self.native.release_from(ID);
	}
}

/// One group's physical state this poll.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct Input {
	/// A key or button of the group is down, whatever the modifiers.
	down: bool,
	/// The group's chord wins for the held modifiers.
	matching: bool,
	/// This poller may start a press now. Keyboard keys are left to focused input while Serein
	/// is focused; mouse buttons keep working because the window may not see them when the
	/// pointer is elsewhere.
	eligible: bool,
}

/// Physical state per group. The most specific chord wins when several match one key, so
/// Shift+button and plain button can be different actions.
fn step<C: Copy + PartialEq>(
	watched: &[Watch<C>],
	groups: usize,
	focused: bool,
	down: impl Fn(C) -> bool,
	modifiers: Modifiers,
) -> Vec<Input> {
	let mut result = vec![Input::default(); groups];
	let held: Vec<bool> = watched
		.iter()
		.map(|watch| watch.codes.iter().any(|code| down(*code)))
		.collect();
	for (watch, held) in watched.iter().zip(&held) {
		let input = &mut result[watch.group];
		input.down |= held;
		input.eligible |= watch.mouse || !focused;
	}
	let matching: Vec<&Watch<C>> = watched
		.iter()
		.zip(&held)
		.filter(|(watch, held)| **held && modifiers_match(watch.modifiers, modifiers))
		.map(|(watch, _)| watch)
		.collect();
	for watch in &matching {
		let shadowed = matching.iter().any(|other| {
			other.modifiers.count_ones() > watch.modifiers.count_ones()
				&& other.codes.iter().any(|code| watch.codes.contains(code))
		});
		if !shadowed {
			result[watch.group].matching = true;
		}
	}
	result
}

/// Turns physical state into press and release edges. A hold follows its key. A toggle starts
/// only on a physical edge the poller may own, so handing a held key between Serein and
/// another app, or changing modifiers under it, never replays it.
struct Tracker {
	toggle: Vec<bool>,
	previous: Vec<bool>,
	active: Vec<bool>,
}

impl Tracker {
	fn new(toggle: Vec<bool>) -> Self {
		let groups = toggle.len();
		Self {
			toggle,
			previous: vec![false; groups],
			active: vec![false; groups],
		}
	}

	/// Groups whose pressed state changed, with the new state.
	fn advance(&mut self, inputs: &[Input]) -> Vec<(usize, bool)> {
		let mut changed = Vec::new();
		for (group, input) in inputs.iter().enumerate() {
			let allowed = input.down && input.matching && input.eligible;
			let next = if self.toggle[group] {
				allowed && (self.active[group] || !self.previous[group])
			} else {
				allowed
			};
			self.previous[group] = input.down;
			if next != self.active[group] {
				self.active[group] = next;
				changed.push((group, next));
			}
		}
		changed
	}
}

/// Mirrors egui's `matches_logically` for focused input: extra Shift still matches, so a
/// push-to-talk key keeps working while running with Shift, while Ctrl or Command must agree so
/// Ctrl+V does not count as V.
fn modifiers_match(bits: u8, held: Modifiers) -> bool {
	let command_matches = if cfg!(target_os = "macos") {
		held.meta == (bits & PRIMARY != 0) && held.ctrl == (bits & CTRL != 0)
	} else {
		held.ctrl == (bits & (PRIMARY | CTRL) != 0)
	};
	// Extra Alt never matches, like focused input: Windows reports AltGr as Ctrl+Alt.
	command_matches && (bits & SHIFT == 0 || held.shift) && held.alt == (bits & ALT != 0)
}

#[cfg(target_os = "windows")]
mod os {
	use super::Modifiers;
	use windows::Win32::UI::Input::KeyboardAndMouse::*;

	pub(super) type Code = VIRTUAL_KEY;

	pub(super) struct Keys;

	pub(super) fn supported(name: &str) -> bool {
		!codes(name).is_empty()
	}

	impl Keys {
		pub(super) fn open() -> Option<Self> {
			Some(Self)
		}

		pub(super) fn codes(&self, name: &str) -> Vec<Code> {
			codes(name)
		}

		pub(super) fn refresh(&mut self) {}

		pub(super) fn down(&self, code: Code) -> bool {
			key_down(code)
		}

		pub(super) fn modifiers(&self) -> Modifiers {
			Modifiers {
				ctrl: key_down(VK_CONTROL),
				shift: key_down(VK_SHIFT),
				alt: key_down(VK_MENU),
				meta: key_down(VK_LWIN) || key_down(VK_RWIN),
			}
		}
	}

	#[allow(unsafe_code)]
	fn key_down(key: VIRTUAL_KEY) -> bool {
		// SAFETY: GetAsyncKeyState only reads global input state for a virtual-key code.
		unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 }
	}

	/// Virtual keys for an egui key name. Digits and the operators egui folds together also
	/// match their keypad key, as focused input does.
	fn codes(name: &str) -> Vec<Code> {
		let bytes = name.as_bytes();
		if let [letter] = bytes
			&& letter.is_ascii_uppercase()
		{
			return vec![VIRTUAL_KEY(u16::from(*letter))];
		}
		if let Some(digit) = name.strip_prefix("Num").and_then(|d| d.parse::<u16>().ok())
			&& digit < 10
		{
			return vec![VIRTUAL_KEY(0x30 + digit), VIRTUAL_KEY(VK_NUMPAD0.0 + digit)];
		}
		if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<u16>().ok())
			&& (1..=24).contains(&number)
		{
			return vec![VIRTUAL_KEY(VK_F1.0 + number - 1)];
		}
		let codes: &[VIRTUAL_KEY] = match name {
			"MouseMiddle" => &[VK_MBUTTON],
			"MouseExtra1" => &[VK_XBUTTON1],
			"MouseExtra2" => &[VK_XBUTTON2],
			"ArrowDown" => &[VK_DOWN],
			"ArrowLeft" => &[VK_LEFT],
			"ArrowRight" => &[VK_RIGHT],
			"ArrowUp" => &[VK_UP],
			"Escape" => &[VK_ESCAPE],
			"Tab" => &[VK_TAB],
			"Backspace" => &[VK_BACK],
			// Keypad Enter shares the virtual key.
			"Enter" => &[VK_RETURN],
			"Space" => &[VK_SPACE],
			"Insert" => &[VK_INSERT],
			"Delete" => &[VK_DELETE],
			"Home" => &[VK_HOME],
			"End" => &[VK_END],
			"PageUp" => &[VK_PRIOR],
			"PageDown" => &[VK_NEXT],
			"Slash" | "Questionmark" => &[VK_OEM_2, VK_DIVIDE],
			"Backtick" => &[VK_OEM_3],
			"Minus" => &[VK_OEM_MINUS, VK_SUBTRACT],
			"Equals" => &[VK_OEM_PLUS],
			"Plus" => &[VK_OEM_PLUS, VK_ADD],
			"Comma" => &[VK_OEM_COMMA],
			"Period" => &[VK_OEM_PERIOD, VK_DECIMAL],
			"Semicolon" | "Colon" => &[VK_OEM_1],
			"Quote" => &[VK_OEM_7],
			"OpenBracket" | "OpenCurlyBracket" => &[VK_OEM_4],
			"CloseBracket" | "CloseCurlyBracket" => &[VK_OEM_6],
			"Backslash" | "Pipe" => &[VK_OEM_5],
			"IntlBackslash" => &[VK_OEM_102],
			"Exclamationmark" => &[VK_1],
			"CapsLock" => &[VK_CAPITAL],
			"NumLock" => &[VK_NUMLOCK],
			"ScrollLock" => &[VK_SCROLL],
			"PrintScreen" => &[VK_SNAPSHOT],
			"Pause" => &[VK_PAUSE],
			"Menu" => &[VK_APPS],
			"MediaPlayPause" => &[VK_MEDIA_PLAY_PAUSE],
			"MediaTrackNext" => &[VK_MEDIA_NEXT_TRACK],
			"MediaTrackPrevious" => &[VK_MEDIA_PREV_TRACK],
			"MediaStop" => &[VK_MEDIA_STOP],
			"AudioVolumeMute" => &[VK_VOLUME_MUTE],
			"AudioVolumeDown" => &[VK_VOLUME_DOWN],
			"AudioVolumeUp" => &[VK_VOLUME_UP],
			"BrowserBack" => &[VK_BROWSER_BACK],
			"BrowserForward" => &[VK_BROWSER_FORWARD],
			"BrowserRefresh" => &[VK_BROWSER_REFRESH],
			"BrowserSearch" => &[VK_BROWSER_SEARCH],
			"BrowserHome" => &[VK_BROWSER_HOME],
			"BrowserFavorites" => &[VK_BROWSER_FAVORITES],
			"BrowserStop" => &[VK_BROWSER_STOP],
			"LaunchMail" => &[VK_LAUNCH_MAIL],
			"LaunchApp1" => &[VK_LAUNCH_APP1],
			"LaunchApp2" => &[VK_LAUNCH_APP2],
			"Help" => &[VK_HELP],
			"Sleep" => &[VK_SLEEP],
			"Clear" => &[VK_CLEAR],
			_ => &[],
		};
		codes.to_vec()
	}
}

#[cfg(target_os = "macos")]
mod os {
	use super::Modifiers;

	/// A `CGKeyCode`, or a `CGMouseButton` tagged with [`MOUSE`].
	pub(super) type Code = u32;
	const MOUSE: Code = 1 << 16;
	/// `kCGEventSourceStateHIDSystemState`: the hardware state, whichever app is focused.
	const HID_SYSTEM: i32 = 1;

	#[link(name = "CoreGraphics", kind = "framework")]
	#[allow(unsafe_code)]
	unsafe extern "C" {
		fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
		fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
		fn CGPreflightListenEventAccess() -> bool;
		fn CGRequestListenEventAccess() -> bool;
	}

	#[allow(unsafe_code)]
	pub(super) fn listen_access(ask: bool) -> bool {
		// SAFETY: both take no arguments; the request only shows the system prompt once.
		unsafe {
			if ask {
				CGRequestListenEventAccess()
			} else {
				CGPreflightListenEventAccess()
			}
		}
	}

	pub(super) struct Keys;

	pub(super) fn supported(name: &str) -> bool {
		!codes(name).is_empty()
	}

	impl Keys {
		pub(super) fn open() -> Option<Self> {
			Some(Self)
		}

		pub(super) fn codes(&self, name: &str) -> Vec<Code> {
			codes(name)
		}

		pub(super) fn refresh(&mut self) {}

		#[allow(unsafe_code)]
		pub(super) fn down(&self, code: Code) -> bool {
			// SAFETY: both are pure queries of the HID system state taking plain integers.
			unsafe {
				if code & MOUSE != 0 {
					CGEventSourceButtonState(HID_SYSTEM, code & !MOUSE)
				} else {
					CGEventSourceKeyState(HID_SYSTEM, code as u16)
				}
			}
		}

		pub(super) fn modifiers(&self) -> Modifiers {
			let either = |left, right| self.down(left) || self.down(right);
			Modifiers {
				ctrl: either(0x3B, 0x3E),
				shift: either(0x38, 0x3C),
				alt: either(0x3A, 0x3D),
				meta: either(0x37, 0x36),
			}
		}
	}

	/// ANSI key codes from `HIToolbox/Events.h`. They name physical positions, so a letter on
	/// another layout is the key at its US position, which differs from what the layout types.
	fn codes(name: &str) -> Vec<Code> {
		const LETTERS: [Code; 26] = [
			0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D,
			0x1F, 0x23, 0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06,
		];
		const DIGITS: [Code; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
		const KEYPAD: [Code; 10] = [0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C];
		const FUNCTION: [Code; 20] = [
			0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B,
			0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A,
		];
		let bytes = name.as_bytes();
		if let [letter] = bytes
			&& letter.is_ascii_uppercase()
		{
			return vec![LETTERS[usize::from(letter - b'A')]];
		}
		if let Some(digit) = name
			.strip_prefix("Num")
			.and_then(|d| d.parse::<usize>().ok())
			&& digit < 10
		{
			return vec![DIGITS[digit], KEYPAD[digit]];
		}
		if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<usize>().ok())
			&& (1..=FUNCTION.len()).contains(&number)
		{
			return vec![FUNCTION[number - 1]];
		}
		let codes: &[Code] = match name {
			"MouseMiddle" => &[MOUSE | 2],
			"MouseExtra1" => &[MOUSE | 3],
			"MouseExtra2" => &[MOUSE | 4],
			"ArrowLeft" => &[0x7B],
			"ArrowRight" => &[0x7C],
			"ArrowDown" => &[0x7D],
			"ArrowUp" => &[0x7E],
			"Escape" => &[0x35],
			"Tab" => &[0x30],
			"Backspace" => &[0x33],
			"Enter" => &[0x24, 0x4C],
			"Space" => &[0x31],
			"Delete" => &[0x75],
			"Home" => &[0x73],
			"End" => &[0x77],
			"PageUp" => &[0x74],
			"PageDown" => &[0x79],
			"Help" | "Insert" => &[0x72],
			"Slash" | "Questionmark" => &[0x2C, 0x4B],
			"Backtick" => &[0x32],
			"Minus" => &[0x1B, 0x4E],
			"Equals" => &[0x18, 0x51],
			"Plus" => &[0x18, 0x45],
			"Comma" => &[0x2B],
			"Period" => &[0x2F, 0x41],
			"Semicolon" | "Colon" => &[0x29],
			"Quote" => &[0x27],
			"OpenBracket" | "OpenCurlyBracket" => &[0x21],
			"CloseBracket" | "CloseCurlyBracket" => &[0x1E],
			"Backslash" | "Pipe" => &[0x2A],
			"IntlBackslash" => &[0x0A],
			"Exclamationmark" => &[0x12],
			"CapsLock" => &[0x39],
			"Clear" => &[0x47],
			"AudioVolumeUp" => &[0x48],
			"AudioVolumeDown" => &[0x49],
			"AudioVolumeMute" => &[0x4A],
			"Fn" => &[0x3F],
			_ => &[],
		};
		codes.to_vec()
	}
}

/// X11 only: Wayland clients cannot read input outside their own windows, so Wayland keeps
/// the GlobalShortcuts portal.
#[cfg(target_os = "linux")]
mod os {
	use super::Modifiers;
	use x11_dl::{keysym::*, xlib};

	/// An X keycode, or [`MIDDLE`] for the one extra mouse button the core pointer reports.
	pub(super) type Code = u16;
	const MIDDLE: Code = 1 << 8;

	pub(super) struct Keys {
		xlib: xlib::Xlib,
		display: *mut xlib::Display,
		keymap: [u8; 32],
		buttons: u32,
	}

	pub(super) fn supported(name: &str) -> bool {
		name == "MouseMiddle" || !keysyms(name).is_empty()
	}

	#[allow(unsafe_code)]
	impl Keys {
		pub(super) fn open() -> Option<Self> {
			std::env::var_os("DISPLAY")?;
			let xlib = xlib::Xlib::open().ok()?;
			// SAFETY: a null name opens $DISPLAY; the display is closed in Drop.
			let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
			(!display.is_null()).then_some(Self {
				xlib,
				display,
				keymap: [0; 32],
				buttons: 0,
			})
		}

		pub(super) fn codes(&self, name: &str) -> Vec<Code> {
			if name == "MouseMiddle" {
				return vec![MIDDLE];
			}
			keysyms(name)
				.iter()
				// SAFETY: the display is open; an unknown keysym returns 0.
				.map(|sym| unsafe { (self.xlib.XKeysymToKeycode)(self.display, (*sym).into()) })
				.filter(|code| *code != 0)
				.map(Code::from)
				.collect()
		}

		pub(super) fn refresh(&mut self) {
			// SAFETY: the display is open and the keymap buffer is the 32 bytes Xlib writes.
			unsafe {
				(self.xlib.XQueryKeymap)(self.display, self.keymap.as_mut_ptr().cast());
				let root = (self.xlib.XDefaultRootWindow)(self.display);
				let (mut root_return, mut child) = (0, 0);
				let (mut root_x, mut root_y, mut x, mut y) = (0, 0, 0, 0);
				let mut mask = 0;
				(self.xlib.XQueryPointer)(
					self.display,
					root,
					&mut root_return,
					&mut child,
					&mut root_x,
					&mut root_y,
					&mut x,
					&mut y,
					&mut mask,
				);
				self.buttons = mask;
			}
		}

		pub(super) fn down(&self, code: Code) -> bool {
			if code == MIDDLE {
				return self.buttons & xlib::Button2Mask != 0;
			}
			self.keymap[usize::from(code / 8)] & (1 << (code % 8)) != 0
		}

		pub(super) fn modifiers(&self) -> Modifiers {
			let any = |syms: &[u32]| {
				syms.iter()
					// SAFETY: the display is open; an unknown keysym returns 0.
					.map(|sym| unsafe { (self.xlib.XKeysymToKeycode)(self.display, (*sym).into()) })
					.any(|code| code != 0 && self.down(Code::from(code)))
			};
			Modifiers {
				ctrl: any(&[XK_Control_L, XK_Control_R]),
				shift: any(&[XK_Shift_L, XK_Shift_R]),
				alt: any(&[XK_Alt_L, XK_Alt_R, XK_Meta_L, XK_Meta_R]),
				meta: any(&[XK_Super_L, XK_Super_R]),
			}
		}
	}

	impl Drop for Keys {
		#[allow(unsafe_code)]
		fn drop(&mut self) {
			// SAFETY: opened in `open` and closed only here.
			unsafe { (self.xlib.XCloseDisplay)(self.display) };
		}
	}

	/// Keysyms for an egui key name; digits and folded operators also match the keypad.
	fn keysyms(name: &str) -> Vec<u32> {
		let bytes = name.as_bytes();
		if let [letter] = bytes
			&& letter.is_ascii_uppercase()
		{
			return vec![u32::from(letter.to_ascii_lowercase())];
		}
		if let Some(digit) = name.strip_prefix("Num").and_then(|d| d.parse::<u32>().ok())
			&& digit < 10
		{
			return vec![XK_0 + digit, XK_KP_0 + digit];
		}
		if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<u32>().ok())
			&& (1..=24).contains(&number)
		{
			return vec![XK_F1 + number - 1];
		}
		let syms: &[u32] = match name {
			"ArrowDown" => &[XK_Down],
			"ArrowLeft" => &[XK_Left],
			"ArrowRight" => &[XK_Right],
			"ArrowUp" => &[XK_Up],
			"Escape" => &[XK_Escape],
			"Tab" => &[XK_Tab],
			"Backspace" => &[XK_BackSpace],
			"Enter" => &[XK_Return, XK_KP_Enter],
			"Space" => &[XK_space],
			"Insert" => &[XK_Insert],
			"Delete" => &[XK_Delete],
			"Home" => &[XK_Home],
			"End" => &[XK_End],
			"PageUp" => &[XK_Prior],
			"PageDown" => &[XK_Next],
			"Slash" | "Questionmark" => &[XK_slash, XK_KP_Divide],
			"Backtick" => &[XK_grave],
			"Minus" => &[XK_minus, XK_KP_Subtract],
			"Equals" => &[XK_equal],
			"Plus" => &[XK_equal, XK_KP_Add],
			"Comma" => &[XK_comma],
			"Period" => &[XK_period, XK_KP_Decimal],
			"Semicolon" | "Colon" => &[XK_semicolon],
			"Quote" => &[XK_apostrophe],
			"OpenBracket" | "OpenCurlyBracket" => &[XK_bracketleft],
			"CloseBracket" | "CloseCurlyBracket" => &[XK_bracketright],
			"Backslash" | "Pipe" | "IntlBackslash" => &[XK_backslash],
			"Exclamationmark" => &[XK_1],
			"CapsLock" => &[XK_Caps_Lock],
			"NumLock" => &[XK_Num_Lock],
			"ScrollLock" => &[XK_Scroll_Lock],
			"PrintScreen" => &[XK_Print],
			"Pause" => &[XK_Pause],
			"Menu" => &[XK_Menu],
			"Help" => &[XK_Help],
			"Clear" => &[XK_Clear],
			_ => &[],
		};
		syms.to_vec()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn watch(group: usize, code: u8, modifiers: u8, mouse: bool) -> Watch<u8> {
		Watch {
			group,
			codes: vec![code],
			modifiers,
			mouse,
		}
	}

	fn input(down: bool, matching: bool, eligible: bool) -> Input {
		Input {
			down,
			matching,
			eligible,
		}
	}

	#[test]
	fn plain_keys_follow_focus_and_command_modifiers() {
		let watched = [watch(0, 1, 0, false), watch(1, 2, 0, true)];
		let down = |code| code == 1 || code == 2;
		let none = Modifiers::default();
		// Another app is focused: the plain key and the mouse button both count.
		assert_eq!(
			step(&watched, 2, false, down, none),
			vec![input(true, true, true), input(true, true, true)]
		);
		// Serein is focused: focused input owns the keyboard key, the button keeps working.
		assert_eq!(
			step(&watched, 2, true, down, none),
			vec![input(true, true, false), input(true, true, true)]
		);
		// Running with Shift keeps push-to-talk; Ctrl+key or AltGr+key is a different shortcut.
		let shift = Modifiers {
			shift: true,
			..none
		};
		assert!(step(&watched, 2, false, down, shift)[0].matching);
		for other in [
			Modifiers { ctrl: true, ..none },
			Modifiers { alt: true, ..none },
		] {
			assert!(
				step(&watched, 2, false, down, other)
					.iter()
					.all(|input| !input.matching)
			);
		}
	}

	#[test]
	fn most_specific_chord_wins_and_variants_share_a_group() {
		let down = |code| code == 1;
		let shift = Modifiers {
			shift: true,
			..Modifiers::default()
		};
		let plain_and_shift = [watch(0, 1, 0, true), watch(1, 1, SHIFT, true)];
		assert_eq!(
			step(&plain_and_shift, 2, false, down, shift),
			vec![input(true, false, true), input(true, true, true)]
		);
		let variants = [watch(0, 1, 0, false), watch(0, 1, ALT, false)];
		let alt = Modifiers {
			alt: true,
			..Modifiers::default()
		};
		assert_eq!(
			step(&variants, 1, false, down, alt),
			vec![input(true, true, true)]
		);
	}

	#[test]
	fn toggles_start_on_owned_edges_and_holds_follow_the_key() {
		let mut tracker = Tracker::new(vec![true, false]);
		let focused = [input(true, true, false), input(true, true, false)];
		let away = [input(true, true, true), input(true, true, true)];
		// Pressed while Serein is focused: focused input handled both.
		assert!(tracker.advance(&focused).is_empty());
		// Focus moves away with the keys still down: the hold continues, the toggle is not replayed.
		assert_eq!(tracker.advance(&away), vec![(1, true)]);
		assert_eq!(
			tracker.advance(&[Input::default(), Input::default()]),
			vec![(1, false)]
		);
		// A fresh press elsewhere starts the toggle once, and releasing a modifier under the held
		// key does not start it again.
		assert_eq!(tracker.advance(&away), vec![(0, true), (1, true)]);
		assert_eq!(
			tracker.advance(&[input(true, false, true), input(true, true, true)]),
			vec![(0, false)]
		);
		assert!(
			tracker
				.advance(&[input(true, true, true), input(true, true, true)])
				.is_empty()
		);
		// Serein takes focus: the hold ends.
		assert_eq!(
			tracker.advance(&[input(true, true, false), input(true, true, false)]),
			vec![(1, false)]
		);
	}
}
