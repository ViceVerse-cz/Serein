//! Native global voice bindings, using the desktop portal on Wayland and evdev for mouse buttons on Linux.
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use model::{KeyChord, KeybindAction, Keybinds, keybinds::is_mouse_button};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};

const READY: &str = "Global voice keybinds are enabled.";
const DISABLED: &str = "Global keybinds are off. Shortcuts work while Serein is focused.";
#[cfg(target_os = "linux")]
const WAYLAND_PENDING: &str = "Approve the global voice keybinds in your desktop's dialog.";
#[cfg(target_os = "linux")]
const WAYLAND_UNAVAILABLE: &str = "Global voice keybinds were denied or the desktop GlobalShortcuts portal is unavailable; they still work while Serein is focused.";
#[cfg(target_os = "linux")]
const WAYLAND_KEYBOARD_UNAVAILABLE: &str = "Mouse-button voice keybinds work globally. Keyboard ones need the desktop GlobalShortcuts portal, which was denied or is unavailable; they still work while Serein is focused.";
#[cfg(target_os = "linux")]
const MOUSE_UNREADABLE: &str = "Global mouse-button keybinds need read access to /dev/input (join the input group and sign in again); they still work while Serein is focused.";
const UNAVAILABLE: &str =
	"Global voice keybinds are unavailable on this system; they work while Serein is focused.";
const INVALID: &str = "One or more voice bindings cannot be registered globally; they still work while Serein is focused.";
const MODIFIER_REQUIRED: &str = "Add Ctrl, Alt, Shift, or Command to use a voice binding globally; it still works while Serein is focused.";

const PUSH_TO_TALK: usize = 0;
const TOGGLE_MUTE: usize = 1;
const TOGGLE_DEAFEN: usize = 2;
const PUSH_TO_MUTE: usize = 3;

static NATIVE_INPUTS: Mutex<Vec<Weak<NativeInput>>> = Mutex::new(Vec::new());

#[derive(Default)]
struct NativeState {
	registered: [Option<u32>; 4],
	ptt_down: bool,
	ptm_down: bool,
	pending_toggles: u8,
}

struct NativeInput {
	state: Mutex<NativeState>,
	wake: Arc<dyn Fn() + Send + Sync>,
}

impl NativeInput {
	fn handle(&self, event: GlobalHotKeyEvent) {
		let index = self
			.state
			.lock()
			.expect("native hotkey state poisoned")
			.registered
			.iter()
			.position(|id| *id == Some(event.id()));
		if let Some(index) = index {
			self.apply(index, event.state() == HotKeyState::Pressed);
		}
	}

	/// Folds one press or release of the binding in slot `index` and wakes the host.
	fn apply(&self, index: usize, pressed: bool) {
		let mut state = self.state.lock().expect("native hotkey state poisoned");
		let handled = match (index, pressed) {
			(PUSH_TO_TALK, _) => {
				state.ptt_down = pressed;
				true
			}
			(PUSH_TO_MUTE, _) => {
				state.ptm_down = pressed;
				true
			}
			(TOGGLE_MUTE, true) => {
				state.pending_toggles ^= 1;
				true
			}
			(TOGGLE_DEAFEN, true) => {
				state.pending_toggles ^= 2;
				true
			}
			_ => false,
		};
		drop(state);
		if handled {
			(self.wake)();
		}
	}

	fn clear(&self) {
		*self.state.lock().expect("native hotkey state poisoned") = NativeState::default();
	}

	fn set_registered(&self, index: usize, id: Option<u32>) {
		self.state
			.lock()
			.expect("native hotkey state poisoned")
			.registered[index] = id;
	}

	fn take_toggles(&self) -> u8 {
		std::mem::take(
			&mut self
				.state
				.lock()
				.expect("native hotkey state poisoned")
				.pending_toggles,
		)
	}

	fn ptt_down(&self) -> bool {
		self.state
			.lock()
			.expect("native hotkey state poisoned")
			.ptt_down
	}

	fn ptm_down(&self) -> bool {
		self.state
			.lock()
			.expect("native hotkey state poisoned")
			.ptm_down
	}
}

fn dispatch_native_event(event: GlobalHotKeyEvent) {
	let inputs = NATIVE_INPUTS
		.lock()
		.expect("native hotkey targets poisoned")
		.iter()
		.filter_map(Weak::upgrade)
		.collect::<Vec<_>>();
	for input in inputs {
		input.handle(event);
	}
}

pub struct Hotkeys {
	manager: Option<GlobalHotKeyManager>,
	registered: [Option<HotKey>; 4],
	bindings: Option<[KeyChord; 4]>,
	native: Arc<NativeInput>,
	status: &'static str,
	#[cfg(target_os = "windows")]
	mouse: Option<mouse::Poller>,
	#[cfg(target_os = "linux")]
	mouse: Option<evdev::Poller>,
	#[cfg(target_os = "linux")]
	portal: Option<tokio::task::JoinHandle<()>>,
	#[cfg(target_os = "linux")]
	portal_pending: Arc<AtomicU8>,
	#[cfg(target_os = "linux")]
	portal_registered: Arc<AtomicU8>,
	#[cfg(target_os = "linux")]
	portal_ptt_down: Arc<AtomicBool>,
	#[cfg(target_os = "linux")]
	portal_ptm_down: Arc<AtomicBool>,
	#[cfg(target_os = "linux")]
	portal_status: Arc<AtomicU8>,
	#[cfg(target_os = "linux")]
	wake: Arc<dyn Fn() + Send + Sync>,
}

impl Hotkeys {
	pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
		let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
		let native = Arc::new(NativeInput {
			state: Mutex::new(NativeState::default()),
			wake: wake.clone(),
		});
		NATIVE_INPUTS
			.lock()
			.expect("native hotkey targets poisoned")
			.push(Arc::downgrade(&native));
		GlobalHotKeyEvent::set_event_handler(Some(dispatch_native_event));
		let manager = if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
		{
			None
		} else {
			GlobalHotKeyManager::new().ok()
		};
		let status = if manager.is_some() {
			READY
		} else {
			UNAVAILABLE
		};
		Self {
			manager,
			registered: [None; 4],
			bindings: None,
			native,
			status,
			#[cfg(target_os = "windows")]
			mouse: None,
			#[cfg(target_os = "linux")]
			mouse: None,
			#[cfg(target_os = "linux")]
			portal: None,
			#[cfg(target_os = "linux")]
			portal_pending: Arc::new(AtomicU8::new(0)),
			#[cfg(target_os = "linux")]
			portal_registered: Arc::new(AtomicU8::new(0)),
			#[cfg(target_os = "linux")]
			portal_ptt_down: Arc::new(AtomicBool::new(false)),
			#[cfg(target_os = "linux")]
			portal_ptm_down: Arc::new(AtomicBool::new(false)),
			#[cfg(target_os = "linux")]
			portal_status: Arc::new(AtomicU8::new(0)),
			#[cfg(target_os = "linux")]
			wake,
		}
	}

	pub fn sync(&mut self, keybinds: &Keybinds, _runtime: &tokio::runtime::Runtime) {
		let next = keybinds.global_enabled.then(|| {
			[
				keybinds.chord(KeybindAction::PushToTalk).clone(),
				keybinds.chord(KeybindAction::ToggleMute).clone(),
				keybinds.chord(KeybindAction::ToggleDeafen).clone(),
				keybinds.chord(KeybindAction::PushToMute).clone(),
			]
		});
		if self.bindings == next {
			return;
		}
		self.bindings = next.clone();
		self.unregister_all();

		#[cfg(target_os = "linux")]
		{
			if let Some(task) = self.portal.take() {
				task.abort();
			}
			// A cancelled portal task must not publish late input into the new configuration.
			self.portal_pending = Arc::new(AtomicU8::new(0));
			self.portal_registered = Arc::new(AtomicU8::new(0));
			self.portal_ptt_down = Arc::new(AtomicBool::new(false));
			self.portal_ptm_down = Arc::new(AtomicBool::new(false));
			self.portal_status = Arc::new(AtomicU8::new(1));
		}
		let Some(next) = next else {
			return;
		};

		#[cfg(target_os = "linux")]
		{
			self.mouse = evdev::Poller::start(&next, self.native.clone());
		}
		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some() {
			let pending = self.portal_pending.clone();
			let registered = self.portal_registered.clone();
			let ptt_down = self.portal_ptt_down.clone();
			let ptm_down = self.portal_ptm_down.clone();
			let status = self.portal_status.clone();
			let wake = self.wake.clone();
			self.portal = Some(_runtime.spawn(async move {
				let no_shortcuts = matches!(
					portal(
						next,
						pending,
						registered.clone(),
						ptt_down.clone(),
						ptm_down.clone(),
						status.clone(),
						wake.clone(),
					)
					.await,
					Ok(true)
				);
				registered.store(0, Ordering::Relaxed);
				ptt_down.store(false, Ordering::Relaxed);
				ptm_down.store(false, Ordering::Relaxed);
				if !no_shortcuts {
					status.store(3, Ordering::Relaxed);
				}
				wake();
			}));
			return;
		}

		let Some(manager) = &self.manager else {
			return;
		};
		let mut failed = false;
		let mut modifier_required = false;
		for (index, chord) in next.iter().enumerate() {
			// An unassigned push-to-mute binding is simply absent.
			if chord.key.is_empty() && index == PUSH_TO_MUTE {
				continue;
			}
			if !chord.is_valid() {
				failed = true;
				continue;
			}
			if is_mouse_button(&chord.key) {
				// Windows polls mouse bindings below and Linux reads them from evdev above;
				// elsewhere they stay focused-only.
				#[cfg(target_os = "linux")]
				{
					failed |= !self.mouse.as_ref().is_some_and(evdev::Poller::readable);
				}
				#[cfg(not(target_os = "linux"))]
				{
					failed |= cfg!(not(target_os = "windows"));
				}
				continue;
			}
			if chord.modifiers == 0 && !is_standalone_global_key(&chord.key) {
				modifier_required |= !matches!(index, PUSH_TO_TALK | PUSH_TO_MUTE);
				continue;
			}
			let Some(hotkey) = native_hotkey(chord) else {
				failed = true;
				continue;
			};
			self.native.set_registered(index, Some(hotkey.id()));
			match manager.register(hotkey) {
				Ok(()) => {
					self.registered[index] = Some(hotkey);
				}
				Err(_) => {
					self.native.set_registered(index, None);
					failed = true;
				}
			}
		}
		#[cfg(target_os = "windows")]
		{
			self.mouse = mouse::Poller::start(&next, self.native.clone());
		}
		if failed {
			self.status = INVALID;
		} else if modifier_required {
			self.status = MODIFIER_REQUIRED;
		} else {
			self.status = READY;
		}
	}

	fn unregister_all(&mut self) {
		// Stop the poller first so it cannot publish a stale press after the clear.
		#[cfg(any(target_os = "windows", target_os = "linux"))]
		drop(self.mouse.take());
		self.native.clear();
		if let Some(manager) = &self.manager {
			for hotkey in &mut self.registered {
				if let Some(hotkey) = hotkey.take() {
					let _ = manager.unregister(hotkey);
				}
			}
		} else {
			self.registered = [None; 4];
		}
	}

	pub fn take_toggle_pending(&mut self) -> u8 {
		let pending = self.native.take_toggles();
		#[cfg(target_os = "linux")]
		return pending | self.portal_pending.swap(0, Ordering::Relaxed);
		#[cfg(not(target_os = "linux"))]
		pending
	}

	/// Bits for mute/deafen bindings currently owned by the native global registrar.
	pub fn global_toggle_mask(&self) -> u8 {
		#[cfg_attr(
			not(any(target_os = "windows", target_os = "linux")),
			allow(unused_mut)
		)]
		let mut mask = (self.registered[TOGGLE_MUTE].is_some() as u8)
			| ((self.registered[TOGGLE_DEAFEN].is_some() as u8) << 1);
		#[cfg(any(target_os = "windows", target_os = "linux"))]
		if let Some(poller) = &self.mouse {
			mask |= poller.toggle_mask();
		}
		#[cfg(target_os = "linux")]
		return mask | ((self.portal_registered.load(Ordering::Relaxed) >> 1) & 3);
		#[cfg(not(target_os = "linux"))]
		mask
	}

	pub fn push_to_talk_down(&self) -> bool {
		let native = self.native.ptt_down();
		#[cfg(target_os = "linux")]
		return native || self.portal_ptt_down.load(Ordering::Relaxed);
		#[cfg(not(target_os = "linux"))]
		native
	}

	pub fn push_to_mute_down(&self) -> bool {
		let native = self.native.ptm_down();
		#[cfg(target_os = "linux")]
		return native || self.portal_ptm_down.load(Ordering::Relaxed);
		#[cfg(not(target_os = "linux"))]
		native
	}

	/// Assigned bindings that are keys rather than mouse buttons.
	#[cfg(target_os = "linux")]
	fn keyboard_bindings(&self) -> impl Iterator<Item = &KeyChord> {
		self.bindings
			.iter()
			.flatten()
			.filter(|chord| !chord.key.is_empty() && !is_mouse_button(&chord.key))
	}

	pub fn status(&self) -> &'static str {
		if self.bindings.is_none() {
			return DISABLED;
		}
		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some() {
			let mouse_readable = self.mouse.as_ref().map(evdev::Poller::readable);
			if mouse_readable == Some(false) {
				return MOUSE_UNREADABLE;
			}
			return match self.portal_status.load(Ordering::Relaxed) {
				1 => WAYLAND_PENDING,
				2 => READY,
				4 => MODIFIER_REQUIRED,
				// The portal had nothing to bind because every binding is a mouse button.
				_ if mouse_readable == Some(true) && self.keyboard_bindings().next().is_none() => {
					READY
				}
				_ if mouse_readable == Some(true) => WAYLAND_KEYBOARD_UNAVAILABLE,
				_ => WAYLAND_UNAVAILABLE,
			};
		}
		self.status
	}
}

impl Drop for Hotkeys {
	fn drop(&mut self) {
		#[cfg(target_os = "linux")]
		if let Some(task) = self.portal.take() {
			task.abort();
		}
		self.unregister_all();
		let native = Arc::downgrade(&self.native);
		NATIVE_INPUTS
			.lock()
			.expect("native hotkey targets poisoned")
			.retain(|current| !current.ptr_eq(&native));
	}
}

#[cfg(target_os = "linux")]
async fn portal(
	bindings: [KeyChord; 4],
	pending: Arc<AtomicU8>,
	registered: Arc<AtomicU8>,
	ptt_down: Arc<AtomicBool>,
	ptm_down: Arc<AtomicBool>,
	status: Arc<AtomicU8>,
	wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<bool, ashpd::Error> {
	use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
	use futures_util::StreamExt;
	let shortcuts: Vec<_> = [
		("push-to-talk", "Serein push to talk"),
		("mute", "Toggle Serein microphone mute"),
		("deafen", "Toggle Serein deafen"),
		("push-to-mute", "Serein push to mute"),
	]
	.into_iter()
	.zip(bindings.iter())
	.filter_map(|((id, description), chord)| {
		portal_trigger(chord)
			.map(|trigger| NewShortcut::new(id, description).preferred_trigger(trigger.as_str()))
	})
	.collect();
	let modifier_required = bindings[TOGGLE_MUTE..=TOGGLE_DEAFEN].iter().any(|chord| {
		chord.is_valid()
			&& chord.modifiers == 0
			&& !is_mouse_button(&chord.key)
			&& !is_standalone_global_key(&chord.key)
	});
	if shortcuts.is_empty() {
		status.store(if modifier_required { 4 } else { 3 }, Ordering::Relaxed);
		wake();
		return Ok(true);
	}
	let connection = ashpd::zbus::connection::Builder::session()?
		.max_queued(16)
		.build()
		.await?;
	let proxy = GlobalShortcuts::with_connection(connection).await?;
	let session = proxy.create_session(Default::default()).await?;
	let mut activated = proxy.receive_activated().await?;
	let mut deactivated = proxy.receive_deactivated().await?;
	let mut closed = session.receive_closed().await?;
	let response = proxy
		.bind_shortcuts(&session, &shortcuts, None, Default::default())
		.await?
		.response()?;
	let mask = response.shortcuts().iter().fold(0, |mask, shortcut| {
		mask | match shortcut.id() {
			"push-to-talk" => 1,
			"mute" => 2,
			"deafen" => 4,
			"push-to-mute" => 8,
			_ => 0,
		}
	});
	registered.store(mask, Ordering::Relaxed);
	status.store(
		if modifier_required {
			4
		} else if mask == 0 {
			3
		} else {
			2
		},
		Ordering::Relaxed,
	);
	wake();
	loop {
		tokio::select! {
			_ = closed.next() => return Ok(false),
			event = activated.next() => {
				let Some(event) = event else { return Ok(false); };
				match event.shortcut_id() {
					"push-to-talk" => ptt_down.store(true, Ordering::Relaxed),
					"push-to-mute" => ptm_down.store(true, Ordering::Relaxed),
					"mute" => { pending.fetch_xor(1, Ordering::Relaxed); }
					"deafen" => { pending.fetch_xor(2, Ordering::Relaxed); }
					_ => continue,
				}
				wake();
			}
			event = deactivated.next() => {
				let Some(event) = event else { return Ok(false); };
				match event.shortcut_id() {
					"push-to-talk" => ptt_down.store(false, Ordering::Relaxed),
					"push-to-mute" => ptm_down.store(false, Ordering::Relaxed),
					_ => continue,
				}
				wake();
			}
		}
	}
}

#[cfg(target_os = "linux")]
fn portal_trigger(chord: &KeyChord) -> Option<String> {
	if !chord.is_valid() || (chord.modifiers == 0 && !is_standalone_global_key(&chord.key)) {
		return None;
	}
	let mut value = String::new();
	if chord.modifiers & (model::keybinds::PRIMARY | model::keybinds::CTRL) != 0 {
		value.push_str("CTRL+");
	}
	if chord.modifiers & model::keybinds::ALT != 0 {
		value.push_str("ALT+");
	}
	if chord.modifiers & model::keybinds::SHIFT != 0 {
		value.push_str("SHIFT+");
	}
	value.push_str(code_name(&chord.key)?);
	Some(value)
}

fn is_standalone_global_key(name: &str) -> bool {
	matches!(
		name,
		"PageDown"
			| "PageUp"
			| "Insert"
			| "F1" | "F2"
			| "F3" | "F4"
			| "F5" | "F6"
			| "F7" | "F8"
			| "F9" | "F10"
			| "F11" | "F12"
			| "F13" | "F14"
			| "F15" | "F16"
			| "F17" | "F18"
			| "F19" | "F20"
			| "F21" | "F22"
			| "F23" | "F24"
	)
}

fn native_hotkey(chord: &KeyChord) -> Option<HotKey> {
	if !chord.is_valid() || (chord.modifiers == 0 && !is_standalone_global_key(&chord.key)) {
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
		"PageDown" => Some("PageDown"),
		"PageUp" => Some("PageUp"),
		"Insert" => Some("Insert"),
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
		"F13" => Some("F13"),
		"F14" => Some("F14"),
		"F15" => Some("F15"),
		"F16" => Some("F16"),
		"F17" => Some("F17"),
		"F18" => Some("F18"),
		"F19" => Some("F19"),
		"F20" => Some("F20"),
		"F21" => Some("F21"),
		"F22" => Some("F22"),
		"F23" => Some("F23"),
		"F24" => Some("F24"),
		_ => None,
	}
}

/// Windows has no global mouse-button hotkeys, so the physical button state is polled instead.
#[cfg(target_os = "windows")]
mod mouse {
	use super::{KeyChord, NativeInput, TOGGLE_DEAFEN, TOGGLE_MUTE};
	use model::keybinds::{ALT, CTRL, PRIMARY, SHIFT};
	use std::sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	};
	use std::time::Duration;
	use windows::Win32::UI::Input::KeyboardAndMouse::{
		GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_MBUTTON, VK_MENU, VK_SHIFT, VK_XBUTTON1,
		VK_XBUTTON2,
	};

	/// Well under an ordinary click, which stays down for tens of milliseconds.
	const PERIOD: Duration = Duration::from_millis(10);

	/// Runs only while a global voice action is bound to a mouse button.
	pub(super) struct Poller {
		stop: Arc<AtomicBool>,
		thread: Option<std::thread::JoinHandle<()>>,
		toggles: u8,
	}

	impl Poller {
		pub(super) fn start(bindings: &[KeyChord; 4], native: Arc<NativeInput>) -> Option<Self> {
			let watched: Vec<_> = bindings
				.iter()
				.enumerate()
				.filter_map(|(index, chord)| Some((index, button(&chord.key)?, chord.modifiers)))
				.collect();
			if watched.is_empty() {
				return None;
			}
			let toggles = watched.iter().fold(0, |mask, (index, ..)| {
				mask | match *index {
					TOGGLE_MUTE => 1,
					TOGGLE_DEAFEN => 2,
					_ => 0,
				}
			});
			let stop = Arc::new(AtomicBool::new(false));
			let stopped = stop.clone();
			let thread = std::thread::Builder::new()
				.name("serein-mouse-keybinds".into())
				.spawn(move || {
					let mut held = vec![false; watched.len()];
					while !stopped.load(Ordering::Relaxed) {
						for ((index, button, modifiers), held) in watched.iter().zip(&mut held) {
							let down = key_down(*button) && modifiers_match(*modifiers);
							if down != *held {
								*held = down;
								native.apply(*index, down);
							}
						}
						std::thread::sleep(PERIOD);
					}
				})
				.ok()?;
			Some(Self {
				stop,
				thread: Some(thread),
				toggles,
			})
		}

		/// Mute/deafen bits this poller owns, so focused input does not toggle them twice.
		pub(super) fn toggle_mask(&self) -> u8 {
			self.toggles
		}
	}

	impl Drop for Poller {
		fn drop(&mut self) {
			self.stop.store(true, Ordering::Relaxed);
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
		}
	}

	fn button(name: &str) -> Option<VIRTUAL_KEY> {
		match name {
			"MouseMiddle" => Some(VK_MBUTTON),
			"MouseExtra1" => Some(VK_XBUTTON1),
			"MouseExtra2" => Some(VK_XBUTTON2),
			_ => None,
		}
	}

	/// Mirrors egui's `matches_logically` for focused input: extra Shift or Alt still
	/// match, while Ctrl must agree so Ctrl+button stays a distinct binding.
	fn modifiers_match(modifiers: u8) -> bool {
		key_down(VK_CONTROL) == (modifiers & (PRIMARY | CTRL) != 0)
			&& (modifiers & SHIFT == 0 || key_down(VK_SHIFT))
			&& (modifiers & ALT == 0 || key_down(VK_MENU))
	}

	#[allow(unsafe_code)]
	fn key_down(key: VIRTUAL_KEY) -> bool {
		// SAFETY: GetAsyncKeyState only reads global input state for a virtual-key code.
		unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 }
	}
}

/// Neither X11 hotkeys nor the GlobalShortcuts portal cover mouse buttons, and compositors such
/// as niri have no such portal, so bound buttons are read straight from evdev. The devices are
/// only read, never grabbed: every button still reaches the focused application.
#[cfg(target_os = "linux")]
mod evdev {
	use super::{KeyChord, NativeInput, TOGGLE_DEAFEN, TOGGLE_MUTE};
	use model::keybinds::{ALT, CTRL, PRIMARY, SHIFT};
	use std::fs::File;
	use std::io::{ErrorKind, Read};
	use std::os::unix::fs::OpenOptionsExt;
	use std::path::{Path, PathBuf};
	use std::sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	};
	use std::time::{Duration, Instant};

	/// Well under an ordinary click, which stays down for tens of milliseconds.
	const PERIOD: Duration = Duration::from_millis(10);
	/// How often unplugged or newly attached devices (wireless mice, docks) are picked up.
	const RESCAN: Duration = Duration::from_secs(2);
	#[cfg(any(
		target_arch = "x86_64",
		target_arch = "x86",
		target_arch = "aarch64",
		target_arch = "arm",
		target_arch = "riscv64",
		target_arch = "loongarch64",
		target_arch = "powerpc64",
		target_arch = "s390x"
	))]
	const O_NONBLOCK: i32 = 0o4000;
	/// `struct input_event`: a `timeval`, then type, code and value.
	const EVENT_SIZE: usize = 2 * size_of::<usize>() + 8;
	const EV_KEY: u16 = 1;
	const BTN_MIDDLE: u16 = 0x112;
	const BTN_SIDE: u16 = 0x113;
	const BTN_EXTRA: u16 = 0x114;
	const BTN_FORWARD: u16 = 0x115;
	const BTN_BACK: u16 = 0x116;
	const KEY_LEFTCTRL: u16 = 29;
	const KEY_LEFTSHIFT: u16 = 42;
	const KEY_RIGHTSHIFT: u16 = 54;
	const KEY_LEFTALT: u16 = 56;
	const KEY_RIGHTCTRL: u16 = 97;
	const KEY_RIGHTALT: u16 = 100;
	const MODIFIERS: [u16; 6] = [
		KEY_LEFTCTRL,
		KEY_RIGHTCTRL,
		KEY_LEFTSHIFT,
		KEY_RIGHTSHIFT,
		KEY_LEFTALT,
		KEY_RIGHTALT,
	];

	struct Device {
		path: PathBuf,
		file: File,
		held: Vec<u16>,
	}

	/// Runs only while a global voice action is bound to a mouse button.
	pub(super) struct Poller {
		stop: Arc<AtomicBool>,
		thread: Option<std::thread::JoinHandle<()>>,
		toggles: u8,
		readable: bool,
	}

	impl Poller {
		pub(super) fn start(bindings: &[KeyChord; 4], native: Arc<NativeInput>) -> Option<Self> {
			let watched: Vec<_> = bindings
				.iter()
				.enumerate()
				.filter_map(|(index, chord)| Some((index, buttons(&chord.key)?, chord.modifiers)))
				.collect();
			if watched.is_empty() {
				return None;
			}
			let toggles = watched.iter().fold(0, |mask, (index, ..)| {
				mask | match *index {
					TOGGLE_MUTE => 1,
					TOGGLE_DEAFEN => 2,
					_ => 0,
				}
			});
			let mut wanted: Vec<u16> = watched
				.iter()
				.flat_map(|(_, codes, _)| codes.iter().copied())
				.collect();
			// Keyboards are opened only to read modifier state for chords that need it.
			if watched.iter().any(|(_, _, modifiers)| *modifiers != 0) {
				wanted.extend(MODIFIERS);
			}
			let mut devices = Vec::new();
			scan(&wanted, &mut devices);
			let readable = !devices.is_empty();
			let stop = Arc::new(AtomicBool::new(false));
			let stopped = stop.clone();
			let thread = std::thread::Builder::new()
				.name("serein-mouse-keybinds".into())
				.spawn(move || {
					let mut held = vec![false; watched.len()];
					let mut scanned = Instant::now();
					let mut buffer = [0u8; EVENT_SIZE * 64];
					while !stopped.load(Ordering::Relaxed) {
						if scanned.elapsed() >= RESCAN {
							scanned = Instant::now();
							scan(&wanted, &mut devices);
						}
						// A device that fails to read was unplugged; its held buttons are released.
						devices.retain_mut(|device| drain(device, &mut buffer));
						let down = |code: u16| devices.iter().any(|d| d.held.contains(&code));
						for ((index, codes, modifiers), held) in watched.iter().zip(&mut held) {
							let now = codes.iter().any(|code| down(*code))
								&& modifiers_match(*modifiers, &down);
							if now != *held {
								*held = now;
								native.apply(*index, now);
							}
						}
						std::thread::sleep(PERIOD);
					}
				})
				.ok()?;
			Some(Self {
				stop,
				thread: Some(thread),
				toggles,
				readable,
			})
		}

		/// Mute/deafen bits this poller owns, so focused input does not toggle them twice.
		pub(super) fn toggle_mask(&self) -> u8 {
			if self.readable { self.toggles } else { 0 }
		}

		/// Whether any input device with a bound button could be opened.
		pub(super) fn readable(&self) -> bool {
			self.readable
		}
	}

	impl Drop for Poller {
		fn drop(&mut self) {
			self.stop.store(true, Ordering::Relaxed);
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
		}
	}

	/// Linux reports side buttons as SIDE/EXTRA or, on some mice, BACK/FORWARD.
	fn buttons(name: &str) -> Option<&'static [u16]> {
		match name {
			"MouseMiddle" => Some(&[BTN_MIDDLE]),
			"MouseExtra1" => Some(&[BTN_SIDE, BTN_BACK]),
			"MouseExtra2" => Some(&[BTN_EXTRA, BTN_FORWARD]),
			_ => None,
		}
	}

	/// Mirrors egui's `matches_logically` for focused input: extra Shift or Alt still
	/// match, while Ctrl must agree so Ctrl+button stays a distinct binding.
	fn modifiers_match(modifiers: u8, down: &impl Fn(u16) -> bool) -> bool {
		let any = |codes: [u16; 2]| codes.into_iter().any(down);
		any([KEY_LEFTCTRL, KEY_RIGHTCTRL]) == (modifiers & (PRIMARY | CTRL) != 0)
			&& (modifiers & SHIFT == 0 || any([KEY_LEFTSHIFT, KEY_RIGHTSHIFT]))
			&& (modifiers & ALT == 0 || any([KEY_LEFTALT, KEY_RIGHTALT]))
	}

	/// Opens every not-yet-open event device that reports one of the `wanted` key codes.
	fn scan(wanted: &[u16], devices: &mut Vec<Device>) {
		let Ok(entries) = std::fs::read_dir("/sys/class/input") else {
			return;
		};
		for entry in entries.flatten() {
			let name = entry.file_name();
			let Some(name) = name.to_str().filter(|name| name.starts_with("event")) else {
				continue;
			};
			let path = Path::new("/dev/input").join(name);
			if devices.iter().any(|device| device.path == path) {
				continue;
			}
			let Ok(capabilities) =
				std::fs::read_to_string(entry.path().join("device/capabilities/key"))
			else {
				continue;
			};
			if !wanted.iter().any(|code| has_key(&capabilities, *code)) {
				continue;
			}
			if let Ok(file) = std::fs::OpenOptions::new()
				.read(true)
				.custom_flags(O_NONBLOCK)
				.open(&path)
			{
				devices.push(Device {
					path,
					file,
					held: Vec::new(),
				});
			}
		}
	}

	/// sysfs lists the key bitmap as hex words of `long` width, most significant first.
	pub(super) fn has_key(capabilities: &str, code: u16) -> bool {
		let bits = usize::BITS as usize;
		let code = usize::from(code);
		capabilities
			.split_whitespace()
			.rev()
			.nth(code / bits)
			.and_then(|word| usize::from_str_radix(word, 16).ok())
			.is_some_and(|word| word & (1 << (code % bits)) != 0)
	}

	/// Folds every queued event into the device's held keys; false once the device is gone.
	fn drain(device: &mut Device, buffer: &mut [u8]) -> bool {
		loop {
			match device.file.read(buffer) {
				Ok(0) => return false,
				Ok(read) => {
					for event in buffer[..read].as_chunks::<EVENT_SIZE>().0 {
						let at = EVENT_SIZE - 8;
						let kind = u16::from_ne_bytes([event[at], event[at + 1]]);
						let code = u16::from_ne_bytes([event[at + 2], event[at + 3]]);
						let value = i32::from_ne_bytes([
							event[at + 4],
							event[at + 5],
							event[at + 6],
							event[at + 7],
						]);
						if kind != EV_KEY {
							continue;
						}
						match value {
							0 => device.held.retain(|held| *held != code),
							1 if !device.held.contains(&code) => device.held.push(code),
							_ => {}
						}
					}
				}
				Err(error) if error.kind() == ErrorKind::WouldBlock => return true,
				Err(error) if error.kind() == ErrorKind::Interrupted => {}
				Err(_) => return false,
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicBool, Ordering};

	#[cfg(target_os = "linux")]
	#[test]
	fn evdev_capability_bitmaps_find_side_buttons_and_modifiers() {
		// A real mouse: words are most significant first, so BTN_LEFT..BTN_TASK sit in word 4.
		let mouse = "1f0000 0 0 0 0";
		assert!(evdev::has_key(mouse, 0x113));
		assert!(evdev::has_key(mouse, 0x114));
		assert!(!evdev::has_key(mouse, 29));
		let keyboard = "1000000000007 ff980000000007ff febeffdfffefffff fffffffffffffffe";
		assert!(!evdev::has_key(keyboard, 0x113));
		assert!(evdev::has_key(keyboard, 29));
		assert!(evdev::has_key(keyboard, 42));
		assert!(!evdev::has_key("", 0x113));
	}

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
		// Plain letter key stays focused-only so typing is not swallowed
		assert!(native_hotkey(&KeyChord::new("M", 0)).is_none());
		// Standalone navigation and function keys can be registered globally
		assert!(native_hotkey(&KeyChord::new("PageDown", 0)).is_some());
		assert!(native_hotkey(&KeyChord::new("PageUp", 0)).is_some());
		assert!(native_hotkey(&KeyChord::new("Insert", 0)).is_some());
		assert!(native_hotkey(&KeyChord::new("F12", 0)).is_some());
		assert!(native_hotkey(&KeyChord::new("F13", 0)).is_some());
		assert!(native_hotkey(&KeyChord::new("F24", 0)).is_some());
		// Mouse buttons have no native hotkey code.
		assert!(native_hotkey(&KeyChord::new("MouseExtra1", 0)).is_none());
	}

	#[test]
	fn push_to_mute_follows_press_and_release() {
		let input = NativeInput {
			state: Mutex::new(NativeState::default()),
			wake: Arc::new(|| {}),
		};
		input.set_registered(PUSH_TO_MUTE, Some(7));
		input.handle(GlobalHotKeyEvent {
			id: 7,
			state: HotKeyState::Pressed,
		});
		assert!(input.ptm_down());
		assert!(!input.ptt_down());
		input.handle(GlobalHotKeyEvent {
			id: 7,
			state: HotKeyState::Released,
		});
		assert!(!input.ptm_down());
		assert_eq!(input.take_toggles(), 0);
	}

	#[test]
	fn native_events_wake_and_fold_without_a_queue() {
		let woke = Arc::new(AtomicBool::new(false));
		let wake_flag = woke.clone();
		let input = NativeInput {
			state: Mutex::new(NativeState::default()),
			wake: Arc::new(move || {
				wake_flag.store(true, Ordering::Relaxed);
			}),
		};
		input.set_registered(TOGGLE_MUTE, Some(42));
		input.handle(GlobalHotKeyEvent {
			id: 42,
			state: HotKeyState::Pressed,
		});
		assert_eq!(input.take_toggles(), 1);
		assert!(woke.load(Ordering::Relaxed));
	}

	#[test]
	fn native_events_reach_each_live_matching_input() {
		let first = Arc::new(NativeInput {
			state: Mutex::new(NativeState::default()),
			wake: Arc::new(|| {}),
		});
		let second = Arc::new(NativeInput {
			state: Mutex::new(NativeState::default()),
			wake: Arc::new(|| {}),
		});
		first.set_registered(TOGGLE_MUTE, Some(42));
		second.set_registered(TOGGLE_DEAFEN, Some(84));
		{
			let mut inputs = NATIVE_INPUTS
				.lock()
				.expect("native hotkey targets poisoned");
			inputs.clear();
			inputs.extend([Arc::downgrade(&first), Arc::downgrade(&second)]);
		}

		dispatch_native_event(GlobalHotKeyEvent {
			id: 42,
			state: HotKeyState::Pressed,
		});
		dispatch_native_event(GlobalHotKeyEvent {
			id: 84,
			state: HotKeyState::Pressed,
		});

		assert_eq!(first.take_toggles(), 1);
		assert_eq!(second.take_toggles(), 2);
		NATIVE_INPUTS
			.lock()
			.expect("native hotkey targets poisoned")
			.clear();
	}
}
