//! Native global voice bindings, using the desktop portal on Wayland.
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use model::{KeyChord, KeybindAction, Keybinds};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};

const READY: &str = "Global voice keybinds are enabled.";
const DISABLED: &str = "Global keybinds are off. Shortcuts work while Serein is focused.";
#[cfg(target_os = "linux")]
const WAYLAND_PENDING: &str = "Approve the global voice keybinds in your desktop's dialog.";
#[cfg(target_os = "linux")]
const WAYLAND_UNAVAILABLE: &str = "Global voice keybinds were denied or the desktop GlobalShortcuts portal is unavailable; they still work while Serein is focused.";
const UNAVAILABLE: &str =
	"Global voice keybinds are unavailable on this system; they work while Serein is focused.";
const INVALID: &str = "One or more voice bindings cannot be registered globally; they still work while Serein is focused.";
const MODIFIER_REQUIRED: &str = "Add Ctrl, Alt, Shift, or Command to use a voice binding globally; it still works while Serein is focused.";

const PUSH_TO_TALK: usize = 0;
const TOGGLE_MUTE: usize = 1;
const TOGGLE_DEAFEN: usize = 2;

static NATIVE_INPUT: Mutex<Option<Weak<NativeInput>>> = Mutex::new(None);

#[derive(Default)]
struct NativeState {
	registered: [Option<u32>; 3],
	ptt_down: bool,
	pending_toggles: u8,
}

struct NativeInput {
	state: Mutex<NativeState>,
	wake: Arc<dyn Fn() + Send + Sync>,
}

impl NativeInput {
	fn handle(&self, event: GlobalHotKeyEvent) {
		let mut state = self.state.lock().expect("native hotkey state poisoned");
		let index = state
			.registered
			.iter()
			.position(|id| *id == Some(event.id()));
		let handled = match (index, event.state()) {
			(Some(PUSH_TO_TALK), HotKeyState::Pressed) => {
				state.ptt_down = true;
				true
			}
			(Some(PUSH_TO_TALK), HotKeyState::Released) => {
				state.ptt_down = false;
				true
			}
			(Some(TOGGLE_MUTE), HotKeyState::Pressed) => {
				state.pending_toggles ^= 1;
				true
			}
			(Some(TOGGLE_DEAFEN), HotKeyState::Pressed) => {
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
}

fn dispatch_native_event(event: GlobalHotKeyEvent) {
	let input = NATIVE_INPUT
		.lock()
		.expect("native hotkey target poisoned")
		.as_ref()
		.and_then(Weak::upgrade);
	if let Some(input) = input {
		input.handle(event);
	}
}

pub struct Hotkeys {
	manager: Option<GlobalHotKeyManager>,
	registered: [Option<HotKey>; 3],
	bindings: Option<[KeyChord; 3]>,
	native: Arc<NativeInput>,
	status: &'static str,
	#[cfg(target_os = "linux")]
	portal: Option<tokio::task::JoinHandle<()>>,
	#[cfg(target_os = "linux")]
	portal_pending: Arc<AtomicU8>,
	#[cfg(target_os = "linux")]
	portal_registered: Arc<AtomicU8>,
	#[cfg(target_os = "linux")]
	portal_ptt_down: Arc<AtomicBool>,
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
		*NATIVE_INPUT.lock().expect("native hotkey target poisoned") =
			Some(Arc::downgrade(&native));
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
			registered: [None; 3],
			bindings: None,
			native,
			status,
			#[cfg(target_os = "linux")]
			portal: None,
			#[cfg(target_os = "linux")]
			portal_pending: Arc::new(AtomicU8::new(0)),
			#[cfg(target_os = "linux")]
			portal_registered: Arc::new(AtomicU8::new(0)),
			#[cfg(target_os = "linux")]
			portal_ptt_down: Arc::new(AtomicBool::new(false)),
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
			self.portal_status = Arc::new(AtomicU8::new(1));
		}
		let Some(next) = next else {
			return;
		};

		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some() {
			let pending = self.portal_pending.clone();
			let registered = self.portal_registered.clone();
			let ptt_down = self.portal_ptt_down.clone();
			let status = self.portal_status.clone();
			let wake = self.wake.clone();
			self.portal = Some(_runtime.spawn(async move {
				let no_shortcuts = matches!(
					portal(
						next,
						pending,
						registered.clone(),
						ptt_down.clone(),
						status.clone(),
						wake.clone(),
					)
					.await,
					Ok(true)
				);
				registered.store(0, Ordering::Relaxed);
				ptt_down.store(false, Ordering::Relaxed);
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
			if !chord.is_valid() {
				failed = true;
				continue;
			}
			if chord.modifiers == 0 && !is_standalone_global_key(&chord.key) {
				modifier_required |= index != PUSH_TO_TALK;
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
		if failed {
			self.status = INVALID;
		} else if modifier_required {
			self.status = MODIFIER_REQUIRED;
		} else {
			self.status = READY;
		}
	}

	fn unregister_all(&mut self) {
		self.native.clear();
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

	pub fn take_toggle_pending(&mut self) -> u8 {
		let pending = self.native.take_toggles();
		#[cfg(target_os = "linux")]
		return pending | self.portal_pending.swap(0, Ordering::Relaxed);
		#[cfg(not(target_os = "linux"))]
		pending
	}

	/// Bits for mute/deafen bindings currently owned by the native global registrar.
	pub fn global_toggle_mask(&self) -> u8 {
		let mask = (self.registered[TOGGLE_MUTE].is_some() as u8)
			| ((self.registered[TOGGLE_DEAFEN].is_some() as u8) << 1);
		#[cfg(target_os = "linux")]
		return mask | (self.portal_registered.load(Ordering::Relaxed) >> 1);
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

	pub fn status(&self) -> &'static str {
		if self.bindings.is_none() {
			return DISABLED;
		}
		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some() {
			return match self.portal_status.load(Ordering::Relaxed) {
				1 => WAYLAND_PENDING,
				2 => READY,
				4 => MODIFIER_REQUIRED,
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
		let mut target = NATIVE_INPUT.lock().expect("native hotkey target poisoned");
		if target
			.as_ref()
			.is_some_and(|current| current.ptr_eq(&Arc::downgrade(&self.native)))
		{
			*target = None;
		}
	}
}

#[cfg(target_os = "linux")]
async fn portal(
	bindings: [KeyChord; 3],
	pending: Arc<AtomicU8>,
	registered: Arc<AtomicU8>,
	ptt_down: Arc<AtomicBool>,
	status: Arc<AtomicU8>,
	wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<bool, ashpd::Error> {
	use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
	use futures_util::StreamExt;
	let shortcuts: Vec<_> = [
		("push-to-talk", "Serein push to talk"),
		("mute", "Toggle Serein microphone mute"),
		("deafen", "Toggle Serein deafen"),
	]
	.into_iter()
	.zip(bindings.iter())
	.filter_map(|((id, description), chord)| {
		portal_trigger(chord)
			.map(|trigger| NewShortcut::new(id, description).preferred_trigger(trigger.as_str()))
	})
	.collect();
	let modifier_required = bindings[TOGGLE_MUTE..].iter().any(|chord| {
		chord.is_valid() && chord.modifiers == 0 && !is_standalone_global_key(&chord.key)
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
					"mute" => { pending.fetch_xor(1, Ordering::Relaxed); }
					"deafen" => { pending.fetch_xor(2, Ordering::Relaxed); }
					_ => continue,
				}
				wake();
			}
			event = deactivated.next() => {
				let Some(event) = event else { return Ok(false); };
				if event.shortcut_id() == "push-to-talk" {
					ptt_down.store(false, Ordering::Relaxed);
					wake();
				}
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
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicBool, Ordering};

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
}
