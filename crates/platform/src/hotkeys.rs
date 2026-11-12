//! Native global voice bindings, using the desktop portal on Wayland and evdev for mouse buttons on Linux.
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use model::{KeyChord, KeybindAction, Keybinds, keybinds::is_mouse_button};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU8, Ordering};
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
#[cfg(target_os = "linux")]
const MOUSE_MODIFIER_UNREADABLE: &str = "Global mouse-button keybinds need readable keyboard modifier state; they still work while Serein is focused.";
const UNAVAILABLE: &str =
	"Global voice keybinds are unavailable on this system; they work while Serein is focused.";
const INVALID: &str = "One or more voice bindings cannot be registered globally; they still work while Serein is focused.";
const MODIFIER_REQUIRED: &str = "Add Ctrl, Alt, Shift, or Command to use a voice binding globally; it still works while Serein is focused.";

#[cfg(any(target_os = "windows", target_os = "linux"))]
/// Mouse poller identities, kept out of the range `global-hotkey` assigns to real hotkeys.
const MOUSE_ID: u32 = 1 << 31;

static NATIVE_INPUTS: Mutex<Vec<Weak<NativeInput>>> = Mutex::new(Vec::new());

#[derive(Default)]
struct NativeState {
	/// Registered native hotkeys and the action each one triggers.
	registered: Vec<(u32, KeybindAction)>,
	/// Push-to-talk/mute keys currently held, so releasing one key keeps the others down.
	held: Vec<(u32, KeybindAction)>,
	pending_toggles: u8,
}

struct NativeInput {
	state: Mutex<NativeState>,
	wake: Arc<dyn Fn() + Send + Sync>,
}

impl NativeInput {
	fn handle(&self, event: GlobalHotKeyEvent) {
		let action = self
			.state
			.lock()
			.expect("native hotkey state poisoned")
			.registered
			.iter()
			.find(|(id, _)| *id == event.id())
			.map(|(_, action)| *action);
		if let Some(action) = action {
			self.apply(event.id(), action, event.state() == HotKeyState::Pressed);
		}
	}

	/// Folds one press or release of key `id` and wakes the host.
	fn apply(&self, id: u32, action: KeybindAction, pressed: bool) {
		let mut state = self.state.lock().expect("native hotkey state poisoned");
		let handled = match (action, pressed) {
			(KeybindAction::PushToTalk | KeybindAction::PushToMute, _) => {
				state.held.retain(|(held, _)| *held != id);
				if pressed {
					state.held.push((id, action));
				}
				true
			}
			(KeybindAction::ToggleMute, true) => {
				state.pending_toggles ^= 1;
				true
			}
			(KeybindAction::ToggleDeafen, true) => {
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

	fn register(&self, id: u32, action: KeybindAction) {
		self.state
			.lock()
			.expect("native hotkey state poisoned")
			.registered
			.push((id, action));
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

	/// Whether any key currently held triggers `action`.
	fn push_down(&self, action: KeybindAction) -> bool {
		self.state
			.lock()
			.expect("native hotkey state poisoned")
			.held
			.iter()
			.any(|(_, held)| *held == action)
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
	/// Registered native hotkeys and the chord/action each one owns.
	registered: Vec<(HotKey, KeybindAction, KeyChord)>,
	bindings: Option<Vec<(KeybindAction, KeyChord)>>,
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
	/// Chords the Wayland portal actually bound, so focused input skips only those.
	#[cfg(target_os = "linux")]
	portal_chords: Arc<Mutex<Vec<(KeybindAction, KeyChord)>>>,
	/// Shortcut indices currently held down for push-to-talk and push-to-mute.
	#[cfg(target_os = "linux")]
	portal_ptt_held: Arc<std::sync::atomic::AtomicU64>,
	#[cfg(target_os = "linux")]
	portal_ptm_held: Arc<std::sync::atomic::AtomicU64>,
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
			registered: Vec::new(),
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
			portal_chords: Arc::new(Mutex::new(Vec::new())),
			#[cfg(target_os = "linux")]
			portal_ptt_held: Arc::new(std::sync::atomic::AtomicU64::new(0)),
			#[cfg(target_os = "linux")]
			portal_ptm_held: Arc::new(std::sync::atomic::AtomicU64::new(0)),
			#[cfg(target_os = "linux")]
			portal_status: Arc::new(AtomicU8::new(0)),
			#[cfg(target_os = "linux")]
			wake,
		}
	}

	pub fn sync(&mut self, keybinds: &Keybinds, _runtime: &tokio::runtime::Runtime) {
		let next = keybinds.global_enabled.then(|| {
			KeybindAction::ALL
				.into_iter()
				.filter(|action| action.is_global())
				.flat_map(|action| {
					keybinds
						.chords(action)
						.cloned()
						.map(move |chord| (action, chord))
				})
				.collect::<Vec<_>>()
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
			self.portal_chords = Arc::new(Mutex::new(Vec::new()));
			self.portal_ptt_held = Arc::new(std::sync::atomic::AtomicU64::new(0));
			self.portal_ptm_held = Arc::new(std::sync::atomic::AtomicU64::new(0));
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
			let chords = self.portal_chords.clone();
			let ptt_held = self.portal_ptt_held.clone();
			let ptm_held = self.portal_ptm_held.clone();
			let status = self.portal_status.clone();
			let wake = self.wake.clone();
			self.portal = Some(_runtime.spawn(async move {
				let no_shortcuts = matches!(
					portal(
						next,
						pending,
						registered.clone(),
						chords.clone(),
						(ptt_held.clone(), ptm_held.clone()),
						status.clone(),
						wake.clone(),
					)
					.await,
					Ok(true)
				);
				registered.store(0, Ordering::Relaxed);
				chords.lock().expect("portal chords poisoned").clear();
				ptt_held.store(0, Ordering::Relaxed);
				ptm_held.store(0, Ordering::Relaxed);
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
		for (action, chord) in &next {
			// An unassigned binding, such as the default push-to-mute row, is simply absent.
			if chord.key.is_empty() {
				continue;
			}
			if !chord.is_valid() {
				failed = true;
				continue;
			}
			if is_mouse_button(&chord.key) {
				// Windows polls mouse bindings below and Linux reads them from evdev above;
				// elsewhere they stay focused-only.
				#[cfg(not(target_os = "linux"))]
				{
					failed |= cfg!(not(target_os = "windows"));
				}
				continue;
			}
			if chord.modifiers == 0 && !is_standalone_global_key(&chord.key) {
				modifier_required |= !matches!(
					action,
					KeybindAction::PushToTalk | KeybindAction::PushToMute
				);
				continue;
			}
			let Some(hotkey) = native_hotkey(chord) else {
				failed = true;
				continue;
			};
			// Cmd/Ctrl variants can translate to the same native hotkey. A same-action alias
			// is redundant, while a collision with another action leaves that action's
			// chord focused-only and reports the configuration as invalid.
			if let Some((_, registered, _)) = self
				.registered
				.iter()
				.find(|(existing, ..)| existing == &hotkey)
			{
				failed |= *registered != *action;
				continue;
			}
			match manager.register(hotkey) {
				Ok(()) => {
					self.native.register(hotkey.id(), *action);
					self.registered.push((hotkey, *action, chord.clone()));
				}
				Err(_) => failed = true,
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
			for (hotkey, ..) in self.registered.drain(..) {
				let _ = manager.unregister(hotkey);
			}
		} else {
			self.registered.clear();
		}
	}

	pub fn take_toggle_pending(&mut self) -> u8 {
		let pending = self.native.take_toggles();
		#[cfg(target_os = "linux")]
		return pending | self.portal_pending.swap(0, Ordering::Relaxed);
		#[cfg(not(target_os = "linux"))]
		pending
	}

	/// Whether this exact chord is owned by a native global registration.
	pub fn is_global(&self, action: KeybindAction, chord: &KeyChord) -> bool {
		if self
			.registered
			.iter()
			.any(|(_, registered, current)| *registered == action && current == chord)
		{
			return true;
		}
		#[cfg(any(target_os = "windows", target_os = "linux"))]
		if let Some(poller) = &self.mouse
			&& poller.registers(action, chord)
		{
			return true;
		}
		#[cfg(target_os = "linux")]
		if self
			.portal_chords
			.lock()
			.expect("portal chords poisoned")
			.iter()
			.any(|(registered, current)| *registered == action && current == chord)
		{
			return true;
		}
		false
	}

	/// Bits for mute/deafen bindings currently owned by the native global registrar.
	pub fn global_toggle_mask(&self) -> u8 {
		#[cfg_attr(
			not(any(target_os = "windows", target_os = "linux")),
			allow(unused_mut)
		)]
		let mut mask = self.registered.iter().fold(0, |mask, (_, action, _)| {
			mask | match action {
				KeybindAction::ToggleMute => 1,
				KeybindAction::ToggleDeafen => 2,
				_ => 0,
			}
		});
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
		let native = self.native.push_down(KeybindAction::PushToTalk);
		#[cfg(target_os = "linux")]
		return native || self.portal_ptt_held.load(Ordering::Relaxed) != 0;
		#[cfg(not(target_os = "linux"))]
		native
	}

	pub fn push_to_mute_down(&self) -> bool {
		let native = self.native.push_down(KeybindAction::PushToMute);
		#[cfg(target_os = "linux")]
		return native || self.portal_ptm_held.load(Ordering::Relaxed) != 0;
		#[cfg(not(target_os = "linux"))]
		native
	}

	/// Assigned bindings that are keys rather than mouse buttons.
	#[cfg(target_os = "linux")]
	fn keyboard_bindings(&self) -> impl Iterator<Item = &KeyChord> {
		self.bindings
			.iter()
			.flatten()
			.filter(|(_, chord)| !chord.key.is_empty() && !is_mouse_button(&chord.key))
			.map(|(_, chord)| chord)
	}

	pub fn status(&self) -> &'static str {
		if self.bindings.is_none() {
			return DISABLED;
		}
		#[cfg(target_os = "linux")]
		if let Some(problem) = self.mouse.as_ref().and_then(evdev::Poller::problem) {
			return problem;
		}
		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some() {
			let mouse_readable = self.mouse.as_ref().map(evdev::Poller::readable);
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
	bindings: Vec<(KeybindAction, KeyChord)>,
	pending: Arc<AtomicU8>,
	registered: Arc<AtomicU8>,
	chords: Arc<Mutex<Vec<(KeybindAction, KeyChord)>>>,
	(ptt_held, ptm_held): (
		Arc<std::sync::atomic::AtomicU64>,
		Arc<std::sync::atomic::AtomicU64>,
	),
	status: Arc<AtomicU8>,
	wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<bool, ashpd::Error> {
	use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
	use futures_util::StreamExt;
	// One action can own several chords, so group chords that translate to the same trigger.
	// A same-action alias shares the bound shortcut; another action's collision stays focused.
	let mut triggers: Vec<(String, KeybindAction, Vec<KeyChord>)> = Vec::new();
	for (action, chord) in &bindings {
		let Some(trigger) = portal_trigger(chord) else {
			continue;
		};
		match triggers
			.iter_mut()
			.find(|(existing, ..)| existing == &trigger)
		{
			Some((_, owner, group)) => {
				if *owner == *action {
					group.push(chord.clone());
				}
			}
			None => triggers.push((trigger, *action, vec![chord.clone()])),
		}
	}
	let mut ids = Vec::with_capacity(triggers.len());
	let mut shortcuts = Vec::with_capacity(triggers.len());
	for (index, (trigger, action, group_chords)) in triggers.iter().enumerate() {
		// Portals remember triggers by ID, so an ID must never move to another action when
		// bindings change. Only an action's own extra triggers are numbered.
		let earlier = triggers[..index]
			.iter()
			.filter(|(_, owner, _)| owner == action)
			.count();
		let id = match earlier {
			0 => portal_id(*action).to_owned(),
			n => format!("{}-{}", portal_id(*action), n + 1),
		};
		ids.push((id.clone(), *action, index as u32, group_chords.clone()));
		shortcuts.push(
			NewShortcut::new(id, portal_description(*action)).preferred_trigger(trigger.as_str()),
		);
	}
	let modifier_required = bindings.iter().any(|(action, chord)| {
		matches!(
			action,
			KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
		) && chord.is_valid()
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
	// A host install has no portal app ID until this connection registers one; Flatpak skips
	// it. Portals without the Registry interface, or without an installed
	// cz.viceverse.serein.desktop entry, reject it, so the session request still proceeds.
	if let Ok(app_id) = "cz.viceverse.serein".parse() {
		match tokio::time::timeout(
			std::time::Duration::from_secs(5),
			ashpd::register_host_app_with_connection(connection.clone(), app_id),
		)
		.await
		{
			Ok(Ok(())) => {}
			Ok(Err(error)) => {
				eprintln!("Global shortcuts: portal app ID registration failed: {error}")
			}
			Err(_) => eprintln!("Global shortcuts: portal app ID registration timed out"),
		}
	}
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
		mask | ids
			.iter()
			.find(|(id, ..)| id.as_str() == shortcut.id())
			.map_or(0, |(_, action, ..)| match action {
				KeybindAction::PushToTalk => 1,
				KeybindAction::ToggleMute => 2,
				KeybindAction::ToggleDeafen => 4,
				KeybindAction::PushToMute => 8,
				_ => 0,
			})
	});
	registered.store(mask, Ordering::Relaxed);
	*chords.lock().expect("portal chords poisoned") = response
		.shortcuts()
		.iter()
		.filter_map(|shortcut| ids.iter().find(|(id, ..)| id.as_str() == shortcut.id()))
		.flat_map(|(_, action, _, group_chords)| {
			group_chords
				.iter()
				.map(move |chord| (*action, chord.clone()))
		})
		.collect();
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
				let Some((_, action, bit, _)) =
					ids.iter().find(|(id, ..)| event.shortcut_id() == id.as_str())
				else {
					continue;
				};
				match action {
					KeybindAction::PushToTalk => {
						ptt_held.fetch_or(1u64 << *bit, Ordering::Relaxed);
					}
					KeybindAction::PushToMute => {
						ptm_held.fetch_or(1u64 << *bit, Ordering::Relaxed);
					}
					KeybindAction::ToggleMute => {
						pending.fetch_xor(1, Ordering::Relaxed);
					}
					KeybindAction::ToggleDeafen => {
						pending.fetch_xor(2, Ordering::Relaxed);
					}
					_ => continue,
				}
				wake();
			}
			event = deactivated.next() => {
				let Some(event) = event else { return Ok(false); };
				let Some((_, action, bit, _)) =
					ids.iter().find(|(id, ..)| event.shortcut_id() == id.as_str())
				else {
					continue;
				};
				match action {
					KeybindAction::PushToTalk => {
						ptt_held.fetch_and(!(1u64 << *bit), Ordering::Relaxed);
					}
					KeybindAction::PushToMute => {
						ptm_held.fetch_and(!(1u64 << *bit), Ordering::Relaxed);
					}
					_ => continue,
				}
				wake();
			}
		}
	}
}

#[cfg(target_os = "linux")]
fn portal_id(action: KeybindAction) -> &'static str {
	match action {
		KeybindAction::PushToTalk => "push-to-talk",
		KeybindAction::PushToMute => "push-to-mute",
		KeybindAction::ToggleMute => "toggle-mute",
		KeybindAction::ToggleDeafen => "toggle-deafen",
		_ => "voice-shortcut",
	}
}

#[cfg(target_os = "linux")]
fn portal_description(action: KeybindAction) -> &'static str {
	match action {
		KeybindAction::PushToTalk => "Serein push to talk",
		KeybindAction::PushToMute => "Serein push to mute",
		KeybindAction::ToggleMute => "Toggle Serein microphone mute",
		KeybindAction::ToggleDeafen => "Toggle Serein deafen",
		_ => "Serein voice shortcut",
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
	// The portal expects XKB keysyms, not global-hotkey's physical key codes.
	let key = match chord.key.as_str() {
		"ArrowDown" => "Down",
		"ArrowLeft" => "Left",
		"ArrowRight" => "Right",
		"ArrowUp" => "Up",
		"Backspace" => "BackSpace",
		"Enter" => "Return",
		"Space" => "space",
		"PageDown" => "Next",
		"PageUp" => "Prior",
		"Slash" => "slash",
		"Backtick" => "grave",
		"Minus" => "minus",
		"Equals" => "equal",
		"Comma" => "comma",
		"Period" => "period",
		"OpenBracket" => "bracketleft",
		"CloseBracket" => "bracketright",
		"OpenCurlyBracket" => "braceleft",
		"CloseCurlyBracket" => "braceright",
		"Backslash" | "IntlBackslash" => "backslash",
		"Semicolon" => "semicolon",
		"Colon" => "colon",
		"Quote" => "apostrophe",
		"Plus" => "plus",
		"Pipe" => "bar",
		"Questionmark" => "question",
		"Exclamationmark" => "exclam",
		name if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() => {
			value.push(name.as_bytes()[0].to_ascii_lowercase() as char);
			return Some(value);
		}
		name if name.starts_with("Num") => name
			.strip_prefix("Num")
			.filter(|digit| digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit())?,
		name => {
			code_name(name)?;
			name
		}
	};
	value.push_str(key);
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
		"OpenBracket" | "OpenCurlyBracket" => Some("BracketLeft"),
		"CloseBracket" | "CloseCurlyBracket" => Some("BracketRight"),
		"Backslash" | "IntlBackslash" => Some("Backslash"),
		"Semicolon" | "Colon" => Some("Semicolon"),
		"Quote" => Some("Quote"),
		"Plus" => Some("Equal"),
		"Pipe" => Some("Backslash"),
		"Questionmark" => Some("Slash"),
		"Exclamationmark" => Some("Digit1"),
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
	use super::{KeyChord, KeybindAction, MOUSE_ID, NativeInput};
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
		chords: Vec<(KeybindAction, KeyChord)>,
	}

	impl Poller {
		pub(super) fn start(
			bindings: &[(KeybindAction, KeyChord)],
			native: Arc<NativeInput>,
		) -> Option<Self> {
			let watched: Vec<_> = bindings
				.iter()
				.filter_map(|(action, chord)| Some((*action, button(&chord.key)?, chord.modifiers)))
				.collect();
			if watched.is_empty() {
				return None;
			}
			let toggles = watched.iter().fold(0, |mask, (action, ..)| {
				mask | match action {
					KeybindAction::ToggleMute => 1,
					KeybindAction::ToggleDeafen => 2,
					_ => 0,
				}
			});
			// Only mouse chords are polled, so keyboard chords never count as global here.
			let chords: Vec<_> = bindings
				.iter()
				.filter(|(_, chord)| button(&chord.key).is_some())
				.cloned()
				.collect();
			// One group per action and physical button; modifier variants within a group OR.
			let mut toggle_groups: Vec<(KeybindAction, VIRTUAL_KEY)> = Vec::new();
			for (action, button, _) in &watched {
				if matches!(
					action,
					KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
				) && !toggle_groups.iter().any(|(group_action, group_button)| {
					group_action == action && group_button.0 == button.0
				}) {
					toggle_groups.push((*action, *button));
				}
			}
			let stop = Arc::new(AtomicBool::new(false));
			let stopped = stop.clone();
			let thread = std::thread::Builder::new()
				.name("serein-mouse-keybinds".into())
				.spawn(move || {
					let mut held = vec![false; watched.len()];
					let mut held_toggles = vec![false; toggle_groups.len()];
					while !stopped.load(Ordering::Relaxed) {
						let next: Vec<bool> = watched
							.iter()
							.map(|(_, button, modifiers)| {
								key_down(*button) && modifiers_match(*modifiers)
							})
							.collect();
						for (index, ((action, ..), (was, now))) in
							watched.iter().zip(held.iter().zip(&next)).enumerate()
						{
							// Toggle chords fold into one transition per button below.
							if was != now
								&& !matches!(
									action,
									KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
								) {
								native.apply(MOUSE_ID + index as u32, *action, *now);
							}
						}
						for (group, (action, button)) in toggle_groups.iter().enumerate() {
							let down = watched.iter().zip(&next).any(
								|((watched, watched_button, _), down)| {
									watched == action && watched_button.0 == button.0 && *down
								},
							);
							if down != held_toggles[group] {
								held_toggles[group] = down;
								native.apply(MOUSE_ID + 64 + group as u32, *action, down);
							}
						}
						held = next;
						std::thread::sleep(PERIOD);
					}
				})
				.ok()?;
			Some(Self {
				stop,
				thread: Some(thread),
				toggles,
				chords,
			})
		}

		/// Mute/deafen bits this poller owns, so focused input does not toggle them twice.
		pub(super) fn toggle_mask(&self) -> u8 {
			self.toggles
		}

		/// Whether this poller watches the exact chord.
		pub(super) fn registers(&self, action: KeybindAction, chord: &KeyChord) -> bool {
			self.chords
				.iter()
				.any(|(watched, current)| *watched == action && current == chord)
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
	use super::{KeyChord, KeybindAction, MOUSE_ID, NativeInput};
	use model::keybinds::{ALT, CTRL, PRIMARY, SHIFT};
	use std::fs::File;
	use std::io::{self, ErrorKind, Read};
	use std::os::fd::AsRawFd;
	use std::os::unix::fs::OpenOptionsExt;
	use std::path::{Path, PathBuf};
	use std::sync::{
		Arc,
		atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
	};
	use std::time::{Duration, Instant};

	const PERIOD: Duration = Duration::from_millis(10);
	const RESCAN: Duration = Duration::from_secs(2);
	/// Linux input_event contains a timeval followed by type, code and value.
	const EVENT_SIZE: usize = size_of::<libc::input_event>();
	const EV_KEY: u16 = 1;
	const SYN_DROPPED: u16 = 3;
	const MAX_DEVICES: usize = 64;
	const MAX_EVENTS: usize = 64;
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
	const MODIFIER_GROUPS: [(u8, [u16; 2]); 3] = [
		(PRIMARY | CTRL, [KEY_LEFTCTRL, KEY_RIGHTCTRL]),
		(SHIFT, [KEY_LEFTSHIFT, KEY_RIGHTSHIFT]),
		(ALT, [KEY_LEFTALT, KEY_RIGHTALT]),
	];

	struct Device {
		path: PathBuf,
		file: File,
		held: Vec<u16>,
		keys: Vec<u16>,
	}

	type Watched = (KeybindAction, &'static [u16], u8);

	/// Shared availability is indexed by exact chord, including custom bindings.
	pub(super) struct Poller {
		stop: Arc<AtomicBool>,
		thread: Option<std::thread::JoinHandle<()>>,
		available: Arc<AtomicU64>,
		problem: Arc<AtomicU8>,
		chords: Vec<(KeybindAction, KeyChord)>,
	}

	impl Poller {
		pub(super) fn start(
			bindings: &[(KeybindAction, KeyChord)],
			native: Arc<NativeInput>,
		) -> Option<Self> {
			// At most four built-in and 32 custom voice chords; bound the bitset defensively too.
			let chords: Vec<_> = bindings
				.iter()
				.filter(|(_, chord)| chord.is_valid() && buttons(&chord.key).is_some())
				.take(64)
				.cloned()
				.collect();
			let watched: Vec<_> = chords
				.iter()
				.map(|(action, chord)| (*action, buttons(&chord.key).unwrap(), chord.modifiers))
				.collect();
			if watched.is_empty() {
				return None;
			}
			let mut wanted: Vec<u16> = watched
				.iter()
				.flat_map(|(_, codes, _)| codes.iter().copied())
				.collect();
			// Ctrl must agree even for an unmodified chord. Other modifier groups are read
			// only when required; no ordinary keyboard key is retained.
			wanted.extend([KEY_LEFTCTRL, KEY_RIGHTCTRL]);
			for (mask, codes) in MODIFIER_GROUPS.iter().skip(1) {
				if watched
					.iter()
					.any(|(_, _, modifiers)| modifiers & mask != 0)
				{
					wanted.extend(codes);
				}
			}
			wanted.sort_unstable();
			wanted.dedup();
			let mut devices = Vec::new();
			scan(&wanted, &mut devices);
			let available = Arc::new(AtomicU64::new(availability(&devices, &watched)));
			let status = Arc::new(AtomicU8::new(problem(&devices, &watched)));
			let worker_available = available.clone();
			let worker_problem = status.clone();
			let stop = Arc::new(AtomicBool::new(false));
			let stopped = stop.clone();
			let thread = std::thread::Builder::new()
				.name("serein-mouse-keybinds".into())
				.spawn(move || {
					let mut held = vec![false; watched.len()];
					let mut scanned = Instant::now();
					let mut buffer = [0; EVENT_SIZE];
					while !stopped.load(Ordering::Relaxed) {
						if scanned.elapsed() >= RESCAN {
							scanned = Instant::now();
							scan(&wanted, &mut devices);
						}
						let publish = |devices: &[Device]| {
							let next = availability(devices, &watched);
							let changed = worker_available.swap(next, Ordering::Relaxed) != next;
							let next_problem = problem(devices, &watched);
							let changed = worker_problem.swap(next_problem, Ordering::Relaxed)
								!= next_problem || changed;
							if changed {
								(native.wake)();
							}
						};
						publish(&devices);
						let mut device = 0;
						while device < devices.len() {
							if poll_device(
								&mut devices,
								device,
								&wanted,
								&watched,
								&mut held,
								&native,
								&mut buffer,
							) {
								device += 1;
							}
							publish(&devices);
						}
						std::thread::sleep(PERIOD);
					}
				})
				.ok()?;
			Some(Self {
				stop,
				thread: Some(thread),
				available,
				problem: status,
				chords,
			})
		}

		/// Only available chords suppress their focused counterparts.
		pub(super) fn registers(&self, action: KeybindAction, chord: &KeyChord) -> bool {
			let available = self.available.load(Ordering::Relaxed);
			self.chords
				.iter()
				.enumerate()
				.any(|(i, (registered, current))| {
					available & (1 << i) != 0 && *registered == action && current == chord
				})
		}

		pub(super) fn toggle_mask(&self) -> u8 {
			let available = self.available.load(Ordering::Relaxed);
			self.chords
				.iter()
				.enumerate()
				.fold(0, |mask, (i, (action, _))| {
					if available & (1 << i) != 0 {
						mask | toggle_bit(*action)
					} else {
						mask
					}
				})
		}

		pub(super) fn readable(&self) -> bool {
			self.problem.load(Ordering::Relaxed) == 0
		}

		/// A live status shared by X11 and Wayland, independent of keyboard registration.
		pub(super) fn problem(&self) -> Option<&'static str> {
			match self.problem.load(Ordering::Relaxed) {
				1 => Some(super::MOUSE_UNREADABLE),
				2 => Some(super::MOUSE_MODIFIER_UNREADABLE),
				_ => None,
			}
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

	fn toggle_bit(action: KeybindAction) -> u8 {
		match action {
			KeybindAction::ToggleMute => 1,
			KeybindAction::ToggleDeafen => 2,
			_ => 0,
		}
	}

	/// Side-button names vary between Linux mouse drivers.
	fn buttons(name: &str) -> Option<&'static [u16]> {
		match name {
			"MouseMiddle" => Some(&[BTN_MIDDLE]),
			"MouseExtra1" => Some(&[BTN_SIDE, BTN_BACK]),
			"MouseExtra2" => Some(&[BTN_EXTRA, BTN_FORWARD]),
			_ => None,
		}
	}

	/// Extra Shift/Alt match focused input, while Ctrl must agree even when absent.
	fn modifiers_match(modifiers: u8, down: &impl Fn(u16) -> bool) -> bool {
		let any = |codes: [u16; 2]| codes.into_iter().any(down);
		any([KEY_LEFTCTRL, KEY_RIGHTCTRL]) == (modifiers & (PRIMARY | CTRL) != 0)
			&& (modifiers & SHIFT == 0 || any([KEY_LEFTSHIFT, KEY_RIGHTSHIFT]))
			&& (modifiers & ALT == 0 || any([KEY_LEFTALT, KEY_RIGHTALT]))
	}

	/// Stream the whole directory twice, prioritizing button devices over modifier-only
	/// nodes. Payloads and open devices remain bounded; no directory listing is retained.
	fn scan(wanted: &[u16], devices: &mut Vec<Device>) {
		scan_at(
			wanted,
			devices,
			Path::new("/sys/class/input"),
			Path::new("/dev/input"),
			modifier_state,
		);
	}

	fn scan_at(
		wanted: &[u16],
		devices: &mut Vec<Device>,
		sys: &Path,
		dev: &Path,
		seed: fn(&File) -> io::Result<Vec<u16>>,
	) {
		for button_pass in [true, false] {
			let Ok(entries) = std::fs::read_dir(sys) else {
				return;
			};
			for entry in entries.flatten() {
				let name = entry.file_name();
				let Some(name) = name.to_str().filter(|name| {
					name.strip_prefix("event").is_some_and(|suffix| {
						!suffix.is_empty() && suffix.bytes().all(|c| c.is_ascii_digit())
					})
				}) else {
					continue;
				};
				let path = dev.join(name);
				if devices.iter().any(|device| device.path == path) {
					continue;
				}
				let mut capabilities = String::new();
				let Ok(file) = File::open(entry.path().join("device/capabilities/key")) else {
					continue;
				};
				if file.take(1025).read_to_string(&mut capabilities).is_err()
					|| capabilities.len() > 1024
				{
					continue;
				}
				let keys: Vec<_> = wanted
					.iter()
					.copied()
					.filter(|code| has_key(&capabilities, *code))
					.collect();
				if keys.is_empty()
					|| keys.iter().any(|code| !MODIFIERS.contains(code)) != button_pass
				{
					continue;
				}
				// A newly attached mouse may replace a modifier-only node at the limit.
				let replace = if devices.len() >= MAX_DEVICES {
					if !button_pass {
						continue;
					}
					let Some(index) = devices.iter().rposition(|device| {
						device.keys.iter().all(|code| MODIFIERS.contains(code))
					}) else {
						continue;
					};
					Some(index)
				} else {
					None
				};
				let Ok(file) = std::fs::OpenOptions::new()
					.read(true)
					.custom_flags(libc::O_NONBLOCK)
					.open(&path)
				else {
					continue;
				};
				let held = if keys.iter().any(|code| MODIFIERS.contains(code)) {
					let Ok(held) = seed(&file) else {
						continue;
					};
					held.into_iter()
						.filter(|code| keys.contains(code))
						.collect()
				} else {
					Vec::new()
				};
				if let Some(index) = replace {
					devices.remove(index);
				}
				devices.push(Device {
					path,
					file,
					held,
					keys,
				});
			}
		}
	}

	/// EVIOCGKEY queries current kernel state. Seed only modifiers: reopening a held
	/// mouse button must never manufacture a voice press after an overflow.
	#[allow(unsafe_code)]
	fn modifier_state(file: &File) -> io::Result<Vec<u16>> {
		let mut bitmap = [0u8; 96]; // KEY_CNT (0x300) bits, Linux input-event-codes.h.
		// SAFETY: this read-only ioctl writes at most the encoded 96 bytes into a
		// live buffer, using an owned open fd. libc supplies the target's ioctl ABI.
		let result = unsafe {
			libc::ioctl(
				file.as_raw_fd(),
				libc::_IOR::<[u8; 96]>(u32::from(b'E'), 0x18),
				bitmap.as_mut_ptr(),
			)
		};
		if result < 0 {
			return Err(io::Error::last_os_error());
		}
		Ok(seeded_modifiers(&bitmap))
	}

	fn seeded_modifiers(bitmap: &[u8; 96]) -> Vec<u16> {
		MODIFIERS
			.into_iter()
			.filter(|code| bitmap[usize::from(*code) / 8] & (1 << (*code % 8)) != 0)
			.collect()
	}

	/// sysfs hex words are native unsigned-long width, most significant first.
	pub(super) fn has_key(capabilities: &str, code: u16) -> bool {
		let code = usize::from(code);
		capabilities
			.split_whitespace()
			.rev()
			.nth(code / usize::BITS as usize)
			.and_then(|word| usize::from_str_radix(word, 16).ok())
			.is_some_and(|word| word & (1 << (code % usize::BITS as usize)) != 0)
	}

	/// 1: button unavailable; 2: required modifier (including Ctrl-up) unobservable.
	fn binding_problem(devices: &[Device], codes: &[u16], modifiers: u8) -> u8 {
		let supports = |code: &u16| devices.iter().any(|device| device.keys.contains(code));
		if !codes.iter().any(supports) {
			return 1;
		}
		if MODIFIER_GROUPS.iter().any(|(mask, codes)| {
			(*mask == (PRIMARY | CTRL) || modifiers & mask != 0) && !codes.iter().any(supports)
		}) {
			return 2;
		}
		0
	}

	fn availability(devices: &[Device], watched: &[Watched]) -> u64 {
		watched
			.iter()
			.enumerate()
			.fold(0, |mask, (i, (_, codes, modifiers))| {
				mask | (u64::from(binding_problem(devices, codes, *modifiers) == 0) << i)
			})
	}

	fn problem(devices: &[Device], watched: &[Watched]) -> u8 {
		watched
			.iter()
			.map(|(_, codes, modifiers)| binding_problem(devices, codes, *modifiers))
			.find(|problem| *problem != 0)
			.unwrap_or(0)
	}

	/// Held actions follow the chord state; toggles require a real button press with
	/// modifiers already satisfied. Modifier-only events can never toggle voice.
	fn apply_bindings(
		devices: &[Device],
		watched: &[Watched],
		held: &mut [bool],
		native: &NativeInput,
		pressed: Option<u16>,
	) {
		let down = |code: u16| devices.iter().any(|device| device.held.contains(&code));
		let mut toggled = 0;
		for (i, ((action, codes, modifiers), held)) in watched.iter().zip(held).enumerate() {
			let now = binding_problem(devices, codes, *modifiers) == 0
				&& codes.iter().any(|code| down(*code))
				&& modifiers_match(*modifiers, &down);
			let bit = toggle_bit(*action);
			if bit != 0 {
				if now && pressed.is_some_and(|code| codes.contains(&code)) && toggled & bit == 0 {
					native.apply(MOUSE_ID + i as u32, *action, true);
					toggled |= bit;
				}
			} else if now != *held {
				native.apply(MOUSE_ID + i as u32, *action, now);
			}
			*held = now;
		}
	}

	/// Process each queued event before the next; on loss release held actions and
	/// reopen on a later scan instead of trusting potentially missing releases.
	fn poll_device(
		devices: &mut Vec<Device>,
		index: usize,
		wanted: &[u16],
		watched: &[Watched],
		held: &mut [bool],
		native: &NativeInput,
		buffer: &mut [u8; EVENT_SIZE],
	) -> bool {
		for _ in 0..MAX_EVENTS {
			match read_event(&mut devices[index], buffer, wanted) {
				Ok(Some(pressed)) => apply_bindings(devices, watched, held, native, pressed),
				Ok(None) => return true,
				Err(()) => {
					devices.remove(index);
					apply_bindings(devices, watched, held, native, None);
					return false;
				}
			}
		}
		true
	}

	fn fold_event(
		held: &mut Vec<u16>,
		event: &[u8; EVENT_SIZE],
		wanted: &[u16],
	) -> Result<Option<u16>, ()> {
		let at = EVENT_SIZE - 8;
		let kind = u16::from_ne_bytes([event[at], event[at + 1]]);
		let code = u16::from_ne_bytes([event[at + 2], event[at + 3]]);
		let value = i32::from_ne_bytes(event[at + 4..].try_into().unwrap());
		if kind == 0 && code == SYN_DROPPED {
			return Err(());
		}
		if kind == EV_KEY && wanted.contains(&code) {
			match value {
				0 => held.retain(|held| *held != code),
				1 if !held.contains(&code) => {
					held.push(code);
					return Ok(Some(code));
				}
				_ => {}
			}
		}
		Ok(None)
	}

	/// Outer None means no record; inner None means a release or other non-press.
	fn read_event(
		device: &mut Device,
		buffer: &mut [u8; EVENT_SIZE],
		wanted: &[u16],
	) -> Result<Option<Option<u16>>, ()> {
		match device.file.read(buffer) {
			Ok(EVENT_SIZE) => fold_event(&mut device.held, buffer, wanted).map(Some),
			Ok(_) => Err(()),
			Err(error)
				if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
			{
				Ok(None)
			}
			Err(_) => Err(()),
		}
	}
	#[cfg(test)]
	mod tests {
		use super::*;
		use std::io::{Seek, Write};

		fn native() -> NativeInput {
			NativeInput {
				state: Default::default(),
				wake: Arc::new(|| {}),
			}
		}

		fn device(keys: &[u16]) -> Device {
			Device {
				path: PathBuf::new(),
				file: tempfile::tempfile().unwrap(),
				held: Vec::new(),
				keys: keys.to_vec(),
			}
		}

		fn event(kind: u16, code: u16, value: i32) -> [u8; EVENT_SIZE] {
			let mut bytes = [0; EVENT_SIZE];
			let at = EVENT_SIZE - 8;
			bytes[at..at + 2].copy_from_slice(&kind.to_ne_bytes());
			bytes[at + 2..at + 4].copy_from_slice(&code.to_ne_bytes());
			bytes[at + 4..].copy_from_slice(&value.to_ne_bytes());
			bytes
		}

		fn queue(device: &mut Device, events: &[(u16, u16, i32)]) {
			device.file.set_len(0).unwrap();
			device.file.rewind().unwrap();
			for &(kind, code, value) in events {
				device.file.write_all(&event(kind, code, value)).unwrap();
			}
			device.file.rewind().unwrap();
		}

		#[test]
		fn queued_clicks_survive_and_lost_events_release_push_to_talk() {
			let native = native();
			let watched: &[Watched] = &[
				(KeybindAction::ToggleMute, &[BTN_SIDE], 0),
				(KeybindAction::PushToTalk, &[BTN_SIDE], 0),
			];
			let mut devices = vec![device(&[BTN_SIDE, KEY_LEFTCTRL])];
			queue(
				&mut devices[0],
				&[(EV_KEY, BTN_SIDE, 1), (EV_KEY, BTN_SIDE, 0)],
			);
			let mut held = [false; 2];
			assert!(!poll_device(
				&mut devices,
				0,
				&[BTN_SIDE],
				watched,
				&mut held,
				&native,
				&mut [0; EVENT_SIZE]
			));
			assert!(devices.is_empty());
			assert_eq!(native.take_toggles(), 1);
			assert!(!native.push_down(KeybindAction::PushToTalk));

			// Separate worker step: a missed release must not leave voice held.
			devices.push(device(&[BTN_SIDE, KEY_LEFTCTRL]));
			devices[0].held.push(BTN_SIDE);
			apply_bindings(&devices, watched, &mut held, &native, None);
			assert!(native.push_down(KeybindAction::PushToTalk));
			queue(
				&mut devices[0],
				&[(0, SYN_DROPPED, 0), (EV_KEY, BTN_SIDE, 1)],
			);
			assert!(!poll_device(
				&mut devices,
				0,
				&[BTN_SIDE],
				watched,
				&mut held,
				&native,
				&mut [0; EVENT_SIZE]
			));
			assert!(!native.push_down(KeybindAction::PushToTalk));
			assert_eq!(native.take_toggles(), 0);
		}

		#[test]
		fn modifier_changes_affect_held_actions_but_never_toggle_a_held_button() {
			let native = native();
			let watched: &[Watched] = &[
				(KeybindAction::ToggleMute, &[BTN_SIDE], CTRL),
				(KeybindAction::PushToTalk, &[BTN_SIDE], CTRL),
				(KeybindAction::PushToMute, &[BTN_SIDE], 0),
			];
			let wanted = &[BTN_SIDE, KEY_LEFTCTRL];
			let mut devices = vec![device(wanted)];
			let mut held = [false; 3];
			// Plain press matches only the unmodified held binding.
			for (code, value, ptt, ptm) in [
				(BTN_SIDE, 1, false, true),
				(KEY_LEFTCTRL, 1, true, false),
				(KEY_LEFTCTRL, 0, false, true),
				(KEY_LEFTCTRL, 1, true, false),
				(BTN_SIDE, 0, false, false),
			] {
				let pressed =
					fold_event(&mut devices[0].held, &event(EV_KEY, code, value), wanted).unwrap();
				apply_bindings(&devices, watched, &mut held, &native, pressed);
				assert_eq!(native.take_toggles(), 0);
				assert_eq!(native.push_down(KeybindAction::PushToTalk), ptt);
				assert_eq!(native.push_down(KeybindAction::PushToMute), ptm);
			}
			let pressed =
				fold_event(&mut devices[0].held, &event(EV_KEY, BTN_SIDE, 1), wanted).unwrap();
			apply_bindings(&devices, watched, &mut held, &native, pressed);
			assert_eq!(native.take_toggles(), 1);
			assert!(native.push_down(KeybindAction::PushToTalk));
			// Kernel autorepeat is not another press.
			let pressed =
				fold_event(&mut devices[0].held, &event(EV_KEY, BTN_SIDE, 2), wanted).unwrap();
			apply_bindings(&devices, watched, &mut held, &native, pressed);
			assert_eq!(native.take_toggles(), 0);
		}

		#[test]
		fn availability_tracks_exact_chords_and_modifier_device_loss() {
			let native = native();
			let watched: &[Watched] = &[
				(KeybindAction::ToggleMute, &[BTN_SIDE], 0),
				(KeybindAction::ToggleMute, &[BTN_EXTRA], SHIFT),
			];
			let chords = vec![
				(
					KeybindAction::ToggleMute,
					KeyChord {
						key: "MouseExtra1".into(),
						modifiers: 0,
					},
				),
				(
					KeybindAction::ToggleMute,
					KeyChord {
						key: "MouseExtra2".into(),
						modifiers: SHIFT,
					},
				),
			];
			let mut devices = vec![device(&[BTN_SIDE, BTN_EXTRA])];
			assert_eq!(problem(&devices, watched), 2);
			assert_eq!(availability(&devices, watched), 0);
			devices.push(device(&[KEY_LEFTCTRL]));
			let available = Arc::new(AtomicU64::new(availability(&devices, watched)));
			let status = Arc::new(AtomicU8::new(problem(&devices, watched)));
			let poller = Poller {
				stop: Arc::new(AtomicBool::new(false)),
				thread: None,
				available: available.clone(),
				problem: status.clone(),
				chords,
			};
			assert!(poller.registers(KeybindAction::ToggleMute, &poller.chords[0].1));
			assert!(!poller.registers(KeybindAction::ToggleMute, &poller.chords[1].1));
			assert_eq!(poller.toggle_mask(), 1);
			assert_eq!(
				poller.problem(),
				Some(super::super::MOUSE_MODIFIER_UNREADABLE)
			);
			devices.push(device(&[KEY_LEFTSHIFT]));
			available.store(availability(&devices, watched), Ordering::Relaxed);
			status.store(problem(&devices, watched), Ordering::Relaxed);
			assert!(poller.readable());
			assert!(poller.registers(KeybindAction::ToggleMute, &poller.chords[1].1));
			devices[0].held.push(BTN_EXTRA);
			devices[2].held.push(KEY_LEFTSHIFT);
			let mut held = [false; 2];
			apply_bindings(&devices, watched, &mut held, &native, Some(BTN_EXTRA));
			assert_eq!(native.take_toggles(), 1);
			devices.remove(1); // Ctrl state is now unknown, not "released".
			available.store(availability(&devices, watched), Ordering::Relaxed);
			assert_eq!(poller.toggle_mask(), 0);
			apply_bindings(&devices, watched, &mut held, &native, Some(BTN_EXTRA));
			assert_eq!(native.take_toggles(), 0);
		}

		#[test]
		fn modifier_snapshot_never_seeds_buttons_and_rejects_non_evdev_files() {
			let mut bitmap = [0; 96];
			for code in [KEY_LEFTCTRL, KEY_RIGHTSHIFT, BTN_SIDE] {
				bitmap[usize::from(code) / 8] |= 1 << (code % 8);
			}
			assert_eq!(
				seeded_modifiers(&bitmap),
				vec![KEY_LEFTCTRL, KEY_RIGHTSHIFT]
			);
			assert!(modifier_state(&tempfile::tempfile().unwrap()).is_err());
		}

		#[test]
		fn scan_prioritizes_buttons_beyond_old_entry_limit_and_replaces_modifier_nodes() {
			let root = tempfile::tempdir().unwrap();
			let sys = root.path().join("sys");
			let dev = root.path().join("dev");
			std::fs::create_dir_all(&dev).unwrap();
			for i in 0..300 {
				let path = sys.join(format!("event{i}/device/capabilities"));
				std::fs::create_dir_all(&path).unwrap();
				std::fs::write(
					path.join("key"),
					if i == 299 {
						"80000 0 0 0 0"
					} else {
						"20000000"
					},
				)
				.unwrap();
				File::create(dev.join(format!("event{i}"))).unwrap();
			}
			let seed: fn(&File) -> io::Result<Vec<u16>> = |_| Ok(vec![KEY_LEFTCTRL]);
			let mut devices = Vec::new();
			scan_at(&[BTN_SIDE, KEY_LEFTCTRL], &mut devices, &sys, &dev, seed);
			assert_eq!(devices.len(), MAX_DEVICES);
			assert!(devices.iter().any(|d| d.keys.contains(&BTN_SIDE)));
			assert!(devices.iter().all(|d| !d.held.contains(&BTN_SIDE)));
			// A new button device replaces a modifier-only device in a full poller.
			let path = sys.join("event300/device/capabilities");
			std::fs::create_dir_all(&path).unwrap();
			std::fs::write(path.join("key"), "100000 0 0 0 0").unwrap();
			File::create(dev.join("event300")).unwrap();
			scan_at(
				&[BTN_SIDE, BTN_EXTRA, KEY_LEFTCTRL],
				&mut devices,
				&sys,
				&dev,
				seed,
			);
			assert_eq!(devices.len(), MAX_DEVICES);
			assert!(devices.iter().any(|d| d.keys.contains(&BTN_EXTRA)));
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

	#[cfg(target_os = "linux")]
	#[test]
	fn portal_triggers_use_xkb_keysyms() {
		use model::keybinds::{ALT, CTRL, PRIMARY, SHIFT};
		for (key, keysym) in [
			("M", "m"),
			("D", "d"),
			("Num1", "1"),
			("ArrowDown", "Down"),
			("Backspace", "BackSpace"),
			("Enter", "Return"),
			("Space", "space"),
			("PageDown", "Next"),
			("PageUp", "Prior"),
			("Slash", "slash"),
			("Backtick", "grave"),
			("Minus", "minus"),
			("Equals", "equal"),
			("Comma", "comma"),
			("Period", "period"),
			("OpenBracket", "bracketleft"),
			("CloseBracket", "bracketright"),
			("Backslash", "backslash"),
			("Semicolon", "semicolon"),
			("Quote", "apostrophe"),
			("F12", "F12"),
			("Tab", "Tab"),
		] {
			assert_eq!(
				portal_trigger(&KeyChord::new(key, PRIMARY | SHIFT)),
				Some(format!("CTRL+SHIFT+{keysym}"))
			);
		}
		assert_eq!(
			portal_trigger(&KeyChord::new("D", CTRL | ALT)),
			Some("CTRL+ALT+d".into())
		);
		assert_eq!(
			portal_trigger(&KeyChord::new("PageDown", 0)),
			Some("Next".into())
		);
		for chord in [
			KeyChord::default(),
			KeyChord::new("MouseExtra1", ALT),
			KeyChord::new("Num10", ALT),
			KeyChord::new("unknown", ALT),
		] {
			assert_eq!(portal_trigger(&chord), None);
		}
	}

	#[test]
	fn modifier_bindings_have_native_codes_and_plain_keys_stay_focused() {
		for key in [
			"OpenBracket",
			"CloseBracket",
			"OpenCurlyBracket",
			"CloseCurlyBracket",
			"Backslash",
			"IntlBackslash",
			"Semicolon",
			"Colon",
			"Quote",
			"Plus",
			"Pipe",
			"Questionmark",
			"Exclamationmark",
		] {
			assert!(
				native_hotkey(&KeyChord::new(key, model::keybinds::PRIMARY)).is_some(),
				"native hotkey mapping missing for {key}"
			);
		}
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
		input.register(7, KeybindAction::PushToMute);
		input.handle(GlobalHotKeyEvent {
			id: 7,
			state: HotKeyState::Pressed,
		});
		assert!(input.push_down(KeybindAction::PushToMute));
		assert!(!input.push_down(KeybindAction::PushToTalk));
		input.handle(GlobalHotKeyEvent {
			id: 7,
			state: HotKeyState::Released,
		});
		assert!(!input.push_down(KeybindAction::PushToMute));
		assert_eq!(input.take_toggles(), 0);
	}

	#[test]
	fn each_push_key_releases_without_clearing_the_other() {
		let input = NativeInput {
			state: Mutex::new(NativeState::default()),
			wake: Arc::new(|| {}),
		};
		input.register(3, KeybindAction::PushToTalk);
		input.register(5, KeybindAction::PushToTalk);
		for id in [3, 5] {
			input.handle(GlobalHotKeyEvent {
				id,
				state: HotKeyState::Pressed,
			});
		}
		assert!(input.push_down(KeybindAction::PushToTalk));
		input.handle(GlobalHotKeyEvent {
			id: 3,
			state: HotKeyState::Released,
		});
		assert!(input.push_down(KeybindAction::PushToTalk));
		input.handle(GlobalHotKeyEvent {
			id: 5,
			state: HotKeyState::Released,
		});
		assert!(!input.push_down(KeybindAction::PushToTalk));
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
		input.register(42, KeybindAction::ToggleMute);
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
		first.register(42, KeybindAction::ToggleMute);
		second.register(84, KeybindAction::ToggleDeafen);
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
