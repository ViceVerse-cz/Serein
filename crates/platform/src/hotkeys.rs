//! Native global voice bindings, using the desktop portal on Wayland.
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
#[cfg(any(target_os = "windows", target_os = "macos"))]
use model::keybinds::{POLLED_KEYS, is_pad_button};
use model::{
	KeyChord, KeybindAction, Keybinds,
	keybinds::{is_mouse_button, polled_key_bit},
};
#[cfg(any(target_os = "windows", target_os = "macos"))]
use std::sync::atomic::Ordering;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU8, Ordering};
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

#[cfg(target_os = "windows")]
/// Mouse poller identities, kept out of the range `global-hotkey` assigns to real hotkeys.
const MOUSE_ID: u32 = 1 << 31;
#[cfg(any(target_os = "windows", target_os = "macos"))]
/// Polled-key identities, one per [`POLLED_KEYS`](model::keybinds::POLLED_KEYS) bit.
const POLLED_ID: u32 = 1 << 30;

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

	/// Whether any key currently held triggers `action`, keeping only holds `keep` accepts.
	fn push_down_where(&self, action: KeybindAction, keep: impl Fn(u32) -> bool) -> bool {
		self.state
			.lock()
			.expect("native hotkey state poisoned")
			.held
			.iter()
			.any(|(id, held)| *held == action && keep(*id))
	}

	/// Whether any key currently held triggers `action`.
	#[cfg(test)]
	fn push_down(&self, action: KeybindAction) -> bool {
		self.push_down_where(action, |_| true)
	}
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
/// Whether this held identity is a polled keyboard key, which focused typing may suppress.
/// Pad buttons are physical, not typing.
fn polled_keyboard_hold(id: u32) -> bool {
	id.checked_sub(POLLED_ID)
		.and_then(|index| POLLED_KEYS.get(index as usize))
		.is_some_and(|name| !is_pad_button(name))
}

/// Fans a global-hotkey event out to every live [`NativeInput`]; dead handles are skipped.
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
	/// Voice chords that only the native key poller can report, kept while global bindings
	/// are off.
	polled: Vec<(KeybindAction, KeyChord)>,
	native: Arc<NativeInput>,
	status: &'static str,
	#[cfg(target_os = "windows")]
	poller: Option<poller::Poller>,
	#[cfg(target_os = "macos")]
	poller: Option<mac_poller::Poller>,
	#[cfg(target_os = "windows")]
	shared: poller::Shared,
	#[cfg(target_os = "macos")]
	shared: mac_poller::Shared,
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
	/// Creates the native manager, unless Wayland owns registration, and installs the
	/// process-wide event handler that forwards to this and later inputs.
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
			polled: Vec::new(),
			native,
			status,
			#[cfg(target_os = "windows")]
			poller: None,
			#[cfg(target_os = "macos")]
			poller: None,
			#[cfg(target_os = "windows")]
			shared: poller::Shared::default(),
			#[cfg(target_os = "macos")]
			shared: mac_poller::Shared::default(),
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

	/// Brings OS registrations in line with `keybinds`. Polled keys stay watched even
	/// with global bindings off; publishing is what the gate suppresses.
	pub fn sync(&mut self, keybinds: &Keybinds, _runtime: &tokio::runtime::Runtime) {
		// Polled keys need no registration, so they stay watched even while global bindings
		// are off; only publishing is gated, on focus or on the global flag.
		let polled = KeybindAction::ALL
			.into_iter()
			.filter(|action| action.is_global())
			.flat_map(|action| {
				keybinds
					.chords(action)
					.filter(|chord| polled_key_bit(&chord.key).is_some())
					.cloned()
					.map(move |chord| (action, chord))
			})
			.collect::<Vec<_>>();
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
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		self.shared
			.global
			.store(keybinds.global_enabled, Ordering::Relaxed);
		if self.bindings == next && self.polled == polled {
			return;
		}
		self.bindings = next.clone();
		self.polled = polled;
		self.unregister_all();
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		self.restart_poller();

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
			// Polled keys never reach the OS registrar, so they cannot fail or need a modifier.
			if polled_key_bit(&chord.key).is_some() {
				continue;
			}
			if is_mouse_button(&chord.key) {
				// Windows polls mouse bindings below; elsewhere they stay focused-only.
				failed |= cfg!(not(target_os = "windows"));
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
		if failed {
			self.status = INVALID;
		} else if modifier_required {
			self.status = MODIFIER_REQUIRED;
		} else {
			self.status = READY;
		}
	}

	/// Stops the poller, clears native state and drops every OS registration, so no stale
	/// press survives a failed reconfiguration.
	fn unregister_all(&mut self) {
		// Stop the poller first so it cannot publish a stale press after the clear.
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		drop(self.poller.take());
		self.native.clear();
		if let Some(manager) = &self.manager {
			for (hotkey, ..) in self.registered.drain(..) {
				let _ = manager.unregister(hotkey);
			}
		} else {
			self.registered.clear();
		}
	}

	/// Restarts the native key poller after the watched bindings or the capture state
	/// changed. The old reader is joined here, so the caller can block until an in-flight
	/// HID read returns; a device open or enumeration is not interruptible and can take
	/// longer than the pad read timeout.
	#[cfg(any(target_os = "windows", target_os = "macos"))]
	fn restart_poller(&mut self) {
		// Join the old thread first so both configurations never publish at once.
		self.poller = None;
		self.shared.down.store(0, Ordering::Relaxed);
		#[cfg(target_os = "windows")]
		{
			self.poller = poller::Poller::start(
				self.bindings.as_deref().unwrap_or(&[]),
				&self.polled,
				&self.shared,
				self.native.clone(),
			);
		}
		#[cfg(target_os = "macos")]
		{
			self.poller =
				mac_poller::Poller::start(&self.polled, &self.shared, self.native.clone());
		}
	}

	/// Whether Serein is focused. Polled keys publish only while focused or globally enabled.
	pub fn set_focused(&mut self, focused: bool) {
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		self.shared.focused.store(focused, Ordering::Relaxed);
		#[cfg(not(any(target_os = "windows", target_os = "macos")))]
		let _ = focused;
	}

	/// Whether a chord is being recorded. The poller then runs even with no polled binding,
	/// so [`Self::polled_down`] stays live.
	pub fn set_capturing(&mut self, capturing: bool) {
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		if self.shared.capturing.load(Ordering::Relaxed) != capturing {
			// Stop the old poller under the old flag, so it cannot publish a held key as a
			// fresh press once recording ends; the new poller then seeds held toggles.
			self.poller = None;
			self.shared.capturing.store(capturing, Ordering::Relaxed);
			self.restart_poller();
		}
		#[cfg(not(any(target_os = "windows", target_os = "macos")))]
		let _ = capturing;
	}

	/// Polled keys currently down, for chord capture.
	pub fn polled_down(&self) -> u32 {
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		return self.shared.down.load(Ordering::Relaxed);
		#[cfg(not(any(target_os = "windows", target_os = "macos")))]
		0
	}

	/// Takes the mute/deafen bits the native handler toggled since the previous call.
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
		#[cfg(target_os = "windows")]
		if let Some(poller) = &self.poller
			&& poller.registers(action, chord)
		{
			return true;
		}
		// Polled chords are published by the poller, so focused input must skip them.
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		if self
			.polled
			.iter()
			.any(|(polled, current)| *polled == action && current == chord)
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
		#[cfg_attr(not(target_os = "windows"), allow(unused_mut))]
		let mut mask = self.registered.iter().fold(0, |mask, (_, action, _)| {
			mask | match action {
				KeybindAction::ToggleMute => 1,
				KeybindAction::ToggleDeafen => 2,
				_ => 0,
			}
		});
		#[cfg(target_os = "windows")]
		if let Some(poller) = &self.poller {
			mask |= poller.toggle_mask();
		}
		#[cfg(target_os = "linux")]
		return mask | ((self.portal_registered.load(Ordering::Relaxed) >> 1) & 3);
		#[cfg(not(target_os = "linux"))]
		mask
	}

	/// Whether push-to-talk is held. `typing` drops polled lone keyboard keys while global
	/// bindings are off, so holding one to type capitals cannot open the microphone.
	pub fn push_to_talk_down(&self, typing: bool) -> bool {
		self.voice_hold_down(KeybindAction::PushToTalk, typing)
	}

	pub fn push_to_mute_down(&self, typing: bool) -> bool {
		self.voice_hold_down(KeybindAction::PushToMute, typing)
	}

	/// A polled keyboard key never counts while a text field has the keyboard and global
	/// bindings are off; a registered global hotkey or a pad hold still does.
	fn voice_hold_down(&self, action: KeybindAction, typing: bool) -> bool {
		let native = self
			.native
			.push_down_where(action, |id| !self.typing_suppresses(id, typing));
		#[cfg(target_os = "linux")]
		return native
			|| match action {
				KeybindAction::PushToTalk => self.portal_ptt_held.load(Ordering::Relaxed) != 0,
				_ => self.portal_ptm_held.load(Ordering::Relaxed) != 0,
			};
		#[cfg(not(target_os = "linux"))]
		native
	}

	fn typing_suppresses(&self, id: u32, typing: bool) -> bool {
		#[cfg(any(target_os = "windows", target_os = "macos"))]
		return typing && !self.shared.global.load(Ordering::Relaxed) && polled_keyboard_hold(id);
		#[cfg(not(any(target_os = "windows", target_os = "macos")))]
		{
			let _ = (id, typing);
			false
		}
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

/// Portable key name `global-hotkey` accepts, or `None` for keys it cannot register.
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

/// The polled-key fold shared by the Windows and macOS pollers.
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod polled_fold {
	use super::{KeybindAction, POLLED_ID};

	/// The polled-key fold across ticks: the watched keys and the mask seen last tick.
	pub(super) struct Polled {
		watched: Vec<(KeybindAction, u32)>,
		was: u32,
		toggles: u32,
	}

	impl Polled {
		/// A fresh poller seeded with the keys held right now. A toggle already down must
		/// not press again across a restart, while a hold does: the old poller released it
		/// on exit, so its owner is no longer listening.
		pub(super) fn new(watched: Vec<(KeybindAction, u32)>, held_now: u32) -> Self {
			let toggles = watched.iter().fold(0, |mask, (action, bit)| {
				mask | if matches!(
					action,
					KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
				) {
					*bit
				} else {
					0
				}
			});
			Self {
				watched,
				was: held_now & toggles,
				toggles,
			}
		}

		/// Converts one poll into press/release edges, releasing rather than pressing while
		/// the gate is closed.
		pub(super) fn tick(&mut self, now: u32, live: bool) -> Vec<(u32, KeybindAction, bool)> {
			let events = transitions(&self.watched, self.was, now, live);
			// A gated tick releases holds only; toggles keep their physical level, so one
			// still held when the gate lifts does not press again.
			self.was = if live { now } else { now & self.toggles };
			events
		}

		/// Releases everything still held, for a poller that is being stopped or restarted.
		pub(super) fn release(&mut self) -> Vec<(u32, KeybindAction, bool)> {
			self.tick(0, true)
		}
	}

	/// Press/release edges for the watched polled keys, carrying the identity
	/// [`NativeInput`](super::NativeInput) tracks each hold under.
	fn transitions(
		watched: &[(KeybindAction, u32)],
		was: u32,
		now: u32,
		live: bool,
	) -> Vec<(u32, KeybindAction, bool)> {
		// Losing focus or the pad releases every held bit; toggles act on press only.
		let now = if live { now } else { 0 };
		watched
			.iter()
			.filter_map(|(action, bit)| {
				let pressed = now & bit != 0;
				if pressed == (was & bit != 0)
					|| (!pressed
						&& matches!(
							action,
							KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
						)) {
					return None;
				}
				Some((POLLED_ID + bit.trailing_zeros(), *action, pressed))
			})
			.collect()
	}

	/// The tick fold on its own, so the state machine is testable without input devices.
	#[cfg(test)]
	fn step(
		watched: &[(KeybindAction, u32)],
		was: u32,
		now: u32,
		live: bool,
	) -> Vec<(KeybindAction, bool)> {
		transitions(watched, was, now, live)
			.into_iter()
			.map(|(_, action, pressed)| (action, pressed))
			.collect()
	}

	#[cfg(test)]
	mod tests {
		use super::*;
		use model::keybinds::polled_key_bit;

		fn bit(name: &str) -> u32 {
			polled_key_bit(name).expect("polled key")
		}

		#[test]
		fn step_reports_press_and_release_edges() {
			let watched = [(KeybindAction::PushToTalk, bit("ControlRight"))];
			assert_eq!(
				step(&watched, 0, bit("ControlRight"), true),
				vec![(KeybindAction::PushToTalk, true)]
			);
			assert_eq!(
				step(&watched, bit("ControlRight"), 0, true),
				vec![(KeybindAction::PushToTalk, false)]
			);
			assert_eq!(step(&watched, 0, 0, true), vec![]);
		}

		#[test]
		fn focus_loss_releases_held_ptt() {
			let held = bit("ControlRight");
			assert_eq!(
				step(&[(KeybindAction::PushToTalk, held)], held, held, false),
				vec![(KeybindAction::PushToTalk, false)]
			);
		}

		/// Gating releases a held action once, and refocusing presses it again without
		/// duplicating the release.
		#[test]
		fn gated_ticks_release_once_then_press_on_refocus() {
			let held = bit("ControlRight");
			let mut polled = Polled::new(vec![(KeybindAction::PushToTalk, held)], 0);
			let actions = |events: Vec<(u32, KeybindAction, bool)>| {
				events
					.into_iter()
					.map(|(_, action, pressed)| (action, pressed))
					.collect::<Vec<_>>()
			};
			assert_eq!(
				actions(polled.tick(held, true)),
				vec![(KeybindAction::PushToTalk, true)]
			);
			// Unfocused with the key still held: one release, then silence on later ticks.
			assert_eq!(
				actions(polled.tick(held, false)),
				vec![(KeybindAction::PushToTalk, false)]
			);
			for _ in 0..3 {
				assert_eq!(actions(polled.tick(held, false)), vec![]);
			}
			// Refocusing while the key is still down presses once more.
			assert_eq!(
				actions(polled.tick(held, true)),
				vec![(KeybindAction::PushToTalk, true)]
			);
		}

		/// Losing the pad releases every button it still reported as held.
		#[test]
		fn pad_disconnect_releases() {
			// The pad reader cleared its bits while the button was still down.
			assert_eq!(
				step(
					&[(KeybindAction::PushToTalk, bit("PadSouth"))],
					bit("PadSouth") | bit("PadL2"),
					bit("PadL2"),
					true
				),
				vec![(KeybindAction::PushToTalk, false)]
			);
		}

		#[test]
		fn toggles_fire_once_per_press() {
			let watched = [(KeybindAction::ToggleMute, bit("PadEast"))];
			assert_eq!(
				step(&watched, 0, bit("PadEast"), true),
				vec![(KeybindAction::ToggleMute, true)]
			);
			assert_eq!(step(&watched, bit("PadEast"), 0, true), vec![]);
		}

		/// A restart keeps a held toggle held, so the pad's silence cannot fire it twice.
		#[test]
		fn restart_does_not_repress_a_held_toggle() {
			// Recording another row restarts the poller while the toggle key is down.
			let held = bit("ControlRight");
			let mut polled = Polled::new(vec![(KeybindAction::ToggleMute, held)], held);
			assert_eq!(polled.tick(held, true), vec![]);
			assert_eq!(polled.tick(0, true), vec![]);
		}

		#[test]
		fn refocus_does_not_repress_a_held_toggle() {
			let held = bit("ControlRight");
			let mut polled = Polled::new(vec![(KeybindAction::ToggleMute, held)], 0);
			assert_eq!(polled.tick(held, true).len(), 1);
			assert_eq!(polled.tick(held, false), vec![]);
			assert_eq!(polled.tick(held, true), vec![]);
		}

		#[test]
		fn restart_represses_a_held_hold_action() {
			// The old poller released the hold as it stopped, so the new one presses it.
			let held = bit("ControlRight");
			let mut polled = Polled::new(vec![(KeybindAction::PushToTalk, held)], held);
			assert_eq!(
				polled.tick(held, true),
				vec![(
					POLLED_ID + held.trailing_zeros(),
					KeybindAction::PushToTalk,
					true
				)]
			);
		}
	}
}

/// Windows has no global mouse-button hotkeys and no gamepad hotkeys, so their physical
/// state is polled instead.
#[cfg(target_os = "windows")]
mod poller {
	use super::{
		KeyChord, KeybindAction, MOUSE_ID, NativeInput, is_pad_button, polled_fold::Polled,
		polled_key_bit,
	};
	use hidapi::{HidApi, HidDevice};
	use model::keybinds::{ALT, CTRL, PRIMARY, SHIFT};
	use std::sync::{
		Arc,
		atomic::{AtomicBool, AtomicU32, Ordering},
	};
	use std::time::{Duration, Instant};
	use windows::Win32::UI::Input::KeyboardAndMouse::{
		GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_MBUTTON,
		VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_SHIFT, VK_XBUTTON1, VK_XBUTTON2,
	};

	/// Well under an ordinary click, which stays down for tens of milliseconds.
	const PERIOD: Duration = Duration::from_millis(10);
	/// Bounds the blocking HID read, so a stopped reader exits within about this long.
	/// Enumeration and device open are not interruptible and see the flag only after
	/// they return.
	const PAD_TIMEOUT: i32 = 50;
	/// The largest DualSense report, including the report id.
	const PAD_REPORT: usize = 78;
	/// A pad that streams nothing for about this many empty reads is treated as gone, so a
	/// stalled device cannot leave push-to-talk held.
	const PAD_SILENT_READS: u8 = 20;
	/// How long an unplugged pad waits before the reader enumerates again.
	const PAD_RETRY: Duration = Duration::from_secs(2);
	/// The analog triggers read as pressed from here.
	const TRIGGER_THRESHOLD: u8 = 128;

	/// Whether a mouse hold edge reaches [`NativeInput`]. A press needs a live poller,
	/// while a release always goes through: a press published before the gate closed
	/// would otherwise stay held and leave push-to-talk open.
	fn mouse_hold_edge(was_down: bool, down_now: bool, toggling: bool, live: bool) -> bool {
		!toggling && was_down != down_now && (live || !down_now)
	}

	/// State the poller threads share with `Hotkeys` and the focused input path.
	#[derive(Clone, Default)]
	pub(super) struct Shared {
		/// Whether global bindings are enabled.
		pub(super) global: Arc<AtomicBool>,
		/// Whether the Serein window is focused.
		pub(super) focused: Arc<AtomicBool>,
		/// Whether the settings UI is recording a chord, so the mask must stay live.
		pub(super) capturing: Arc<AtomicBool>,
		/// Pad buttons down, written by the HID reader thread.
		pub(super) pad: Arc<AtomicU32>,
		/// Polled keys down, published for chord capture.
		pub(super) down: Arc<AtomicU32>,
	}

	/// Runs while a voice action is bound to a polled input, or while capturing a chord.
	pub(super) struct Poller {
		stop: Arc<AtomicBool>,
		thread: Option<std::thread::JoinHandle<()>>,
		pad: Option<Pad>,
		toggles: u8,
		chords: Vec<(KeybindAction, KeyChord)>,
	}

	impl Poller {
		/// Starts the key poller, and the pad reader too, when a voice action is bound to a
		/// polled input or a chord is being captured.
		pub(super) fn start(
			bindings: &[(KeybindAction, KeyChord)],
			polled: &[(KeybindAction, KeyChord)],
			shared: &Shared,
			native: Arc<NativeInput>,
		) -> Option<Self> {
			let mouse: Vec<(KeybindAction, VIRTUAL_KEY, u8)> = bindings
				.iter()
				.filter_map(|(action, chord)| Some((*action, button(&chord.key)?, chord.modifiers)))
				.collect();
			// A bit doubles as the polled identity, so a second action on the same key folds
			// into the first and both transitions keep one hold each.
			let mut watched: Vec<(KeybindAction, u32)> = Vec::new();
			for (action, chord) in polled {
				if let Some(bit) = polled_key_bit(&chord.key)
					&& !watched.iter().any(|(_, known)| *known == bit)
				{
					watched.push((*action, bit));
				}
			}
			let capturing = shared.capturing.load(Ordering::Relaxed);
			if mouse.is_empty() && watched.is_empty() && !capturing {
				return None;
			}
			let toggles = mouse.iter().fold(0, |mask, (action, ..)| {
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
			for (action, button, _) in &mouse {
				if matches!(
					action,
					KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
				) && !toggle_groups.iter().any(|(group_action, group_button)| {
					group_action == action && group_button.0 == button.0
				}) {
					toggle_groups.push((*action, *button));
				}
			}
			let pad = (capturing || polled.iter().any(|(_, chord)| is_pad_button(&chord.key)))
				.then(|| Pad::start(shared.pad.clone()));
			if pad.is_none() {
				shared.pad.store(0, Ordering::Relaxed);
			}
			let stop = Arc::new(AtomicBool::new(false));
			let stopped = stop.clone();
			let global = shared.global.clone();
			let focused = shared.focused.clone();
			let capture_flag = shared.capturing.clone();
			let pad_down = shared.pad.clone();
			let down = shared.down.clone();
			let thread = std::thread::Builder::new()
				.name("serein-input-keybinds".into())
				.spawn(move || {
					let keyboard: [(u32, VIRTUAL_KEY); 6] = [
						("ControlLeft", VK_LCONTROL),
						("ControlRight", VK_RCONTROL),
						("ShiftLeft", VK_LSHIFT),
						("ShiftRight", VK_RSHIFT),
						("AltLeft", VK_LMENU),
						("AltRight", VK_RMENU),
					]
					.map(|(name, key)| (polled_key_bit(name).unwrap_or(0), key));
					let sample_keys = || {
						let mut now = pad_down.load(Ordering::Relaxed);
						for (bit, key) in keyboard {
							if key_down(key) {
								now |= bit;
							}
						}
						altgr_filter(now)
					};
					// A restart must not re-fire a held toggle, so start from what is down.
					let mut polled = Polled::new(watched, sample_keys());
					let mut held = vec![false; mouse.len()];
					let mut held_toggles: Vec<bool> = toggle_groups
						.iter()
						.map(|(action, group_button)| {
							mouse.iter().any(|(watched, button, modifiers)| {
								watched == action
									&& button.0 == group_button.0
									&& key_down(*button) && modifiers_match(*modifiers)
							})
						})
						.collect();
					while !stopped.load(Ordering::Relaxed) {
						let live = (global.load(Ordering::Relaxed)
							|| focused.load(Ordering::Relaxed))
							&& !capture_flag.load(Ordering::Relaxed);
						let next: Vec<bool> = mouse
							.iter()
							.map(|(_, button, modifiers)| {
								key_down(*button) && modifiers_match(*modifiers)
							})
							.collect();
						for (index, ((action, ..), (was_down, down_now))) in
							mouse.iter().zip(held.iter().zip(&next)).enumerate()
						{
							let toggling = matches!(
								action,
								KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
							);
							// Mouse chords stay global-only, never focus-gated, but a
							// release must always land so a gated press cannot stick.
							if mouse_hold_edge(*was_down, *down_now, toggling, live) {
								native.apply(MOUSE_ID + index as u32, *action, *down_now);
							}
						}
						for (group, (action, button)) in toggle_groups.iter().enumerate() {
							let down = mouse.iter().zip(&next).any(
								|((watched, watched_button, _), down)| {
									watched == action && watched_button.0 == button.0 && *down
								},
							);
							// Tracking still moves while capturing, so nothing is queued.
							if down != held_toggles[group] {
								held_toggles[group] = down;
								if live {
									native.apply(MOUSE_ID + 64 + group as u32, *action, down);
								}
							}
						}
						held = next;

						let now = sample_keys();
						// Recording reads the raw mask, so publishing alone is gated.
						down.store(now, Ordering::Relaxed);
						for (id, action, pressed) in polled.tick(now, live) {
							native.apply(id, action, pressed);
						}
						std::thread::sleep(PERIOD);
					}
					// A restart must not inherit a press, so release everything still held.
					// Toggle releases are ignored by `apply`, matching the running loop.
					for (id, action, _) in polled.release() {
						native.apply(id, action, false);
					}
					for (index, (action, ..)) in mouse.iter().enumerate() {
						if held.get(index).copied().unwrap_or_default()
							&& !matches!(
								action,
								KeybindAction::ToggleMute | KeybindAction::ToggleDeafen
							) {
							native.apply(MOUSE_ID + index as u32, *action, false);
						}
					}
				})
				.ok()?;
			Some(Self {
				stop,
				thread: Some(thread),
				pad,
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
		/// Stops and joins the reader before dropping the pad, so no report outlives the
		/// poller. The join waits out any in-flight HID read or open.
		fn drop(&mut self) {
			self.stop.store(true, Ordering::Relaxed);
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
			// The reader stops after the key poll, so no report outlives this poller.
			self.pad.take();
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

	/// Whether the OS reports this virtual key as down, focused or not.
	#[allow(unsafe_code)]
	fn key_down(key: VIRTUAL_KEY) -> bool {
		// SAFETY: GetAsyncKeyState only reads global input state for a virtual-key code.
		unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 }
	}

	/// Windows synthesises Left Ctrl while AltGr (Right Alt) is held, so a voice key
	/// bound to Left Ctrl must not fire for it.
	fn altgr_filter(mask: u32) -> u32 {
		let right_alt = polled_key_bit("AltRight").unwrap_or(0);
		let left_ctrl = polled_key_bit("ControlLeft").unwrap_or(0);
		if mask & right_alt != 0 {
			mask & !left_ctrl
		} else {
			mask
		}
	}

	/// Raw DualSense reports arrive independently of the key poll, so they get a thread.
	struct Pad {
		stop: Arc<AtomicBool>,
		thread: Option<std::thread::JoinHandle<()>>,
	}

	impl Pad {
		/// Spawns the pad reader, keeping the flag and handle needed to stop it.
		fn start(mask: Arc<AtomicU32>) -> Self {
			let stop = Arc::new(AtomicBool::new(false));
			let stopped = stop.clone();
			let thread = std::thread::Builder::new()
				.name("serein-pad-keybinds".into())
				.spawn(move || read_pad(&stopped, &mask))
				.ok();
			Self { stop, thread }
		}
	}

	impl Drop for Pad {
		/// Signals the reader and joins it; the join waits out an in-flight read or open.
		fn drop(&mut self) {
			self.stop.store(true, Ordering::Relaxed);
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
		}
	}

	/// Keeps the pad button mask current, reopening the pad whenever it goes away.
	fn read_pad(stop: &AtomicBool, mask: &AtomicU32) {
		let mut buffer = [0u8; PAD_REPORT];
		// A missing HID stack leaves the keyboard bindings working, and it can arrive
		// late, so keep retrying until the poller stops.
		let mut api = loop {
			if stop.load(Ordering::Relaxed) {
				return;
			}
			if let Ok(api) = HidApi::new() {
				break api;
			}
			mask.store(0, Ordering::Relaxed);
			wait(stop, PAD_RETRY);
		};
		let mut device: Option<HidDevice> = None;
		let mut quiet = Quiet::default();
		while !stop.load(Ordering::Relaxed) {
			if device.is_none() {
				let _ = api.refresh_devices();
				device = open_pad(&api);
				// Buttons carried over from a stopped reader are unconfirmed until this
				// device reports, so they expire on silence even on a change-only pad.
				quiet = Quiet::reopened(mask.load(Ordering::Relaxed) != 0);
				if device.is_none() {
					// No pad to read: release anything the last reader left held.
					mask.store(0, Ordering::Relaxed);
					wait(stop, PAD_RETRY);
					continue;
				}
			}
			let Some(pad) = device.as_ref() else {
				continue;
			};
			match pad.read_timeout(&mut buffer, PAD_TIMEOUT) {
				// A failed read is a disconnect; clearing releases any button still held.
				Err(_) => {
					mask.store(0, Ordering::Relaxed);
					device = None;
				}
				Ok(read) => {
					let report = &buffer[..read];
					if let Some(bits) = pad_bits(report) {
						quiet.report(report);
						mask.store(bits, Ordering::Relaxed);
					} else if quiet.gone() {
						// A stall cannot hold a key down.
						mask.store(0, Ordering::Relaxed);
						device = None;
					}
				}
			}
		}
		// The mask outlives this reader, so a restarted poller seeds from the buttons
		// still held and does not see them as a fresh press.
	}

	/// Silence tracking for one opened pad. A pad that streams reports and stops is
	/// gone; a change-only pad keeps its last mask, except buttons carried over from a
	/// stopped reader, which only a report from this device confirms.
	#[derive(Default)]
	struct Quiet {
		streaming: bool,
		carried: bool,
		silent: u8,
	}

	impl Quiet {
		/// Seeds the tracker from the mask a previous reader left behind, which stays
		/// unconfirmed until this device reports.
		fn reopened(carried: bool) -> Self {
			Self {
				carried,
				..Self::default()
			}
		}

		/// Records a recognized report: it clears silence, confirms carried buttons and
		/// marks pads that stream on their own.
		fn report(&mut self, report: &[u8]) {
			self.silent = 0;
			self.carried = false;
			self.streaming |= matches!(report.first(), Some(0x01) if report.len() >= 11)
				|| report.first() == Some(&0x31);
		}

		/// Counts a read with nothing new; true once the pad should be treated as gone.
		fn gone(&mut self) -> bool {
			if !(self.streaming || self.carried) {
				return false;
			}
			if self.silent < PAD_SILENT_READS {
				self.silent += 1;
				return false;
			}
			true
		}
	}

	/// Sleeps in short slices, so a stopped reader returns without waiting out a retry.
	fn wait(stop: &AtomicBool, delay: Duration) {
		let until = Instant::now() + delay;
		while !stop.load(Ordering::Relaxed) && Instant::now() < until {
			std::thread::sleep(PERIOD);
		}
	}

	/// The first DualSense that opens, falling back to its Edge variant.
	fn open_pad(api: &HidApi) -> Option<HidDevice> {
		api.device_list()
			.filter(|device| {
				device.vendor_id() == 0x054C
					&& matches!(device.product_id(), 0x0CE6 | 0x0DF2)
					&& device.usage_page() == 1
					&& device.usage() == 5
			})
			.find_map(|device| device.open_device(api).ok())
	}

	/// Polled-key mask for a DualSense input report, or `None` for a report this does not
	/// recognize.
	fn pad_bits(report: &[u8]) -> Option<u32> {
		// The report id and length place the button block and the analog triggers.
		let (offset, l2, r2) = match report.first() {
			Some(0x01) if report.len() >= 11 => (8, 5, 6),
			Some(0x31) if report.len() >= 12 => (9, 6, 7),
			Some(0x01) if report.len() >= 8 => (5, 8, 9),
			_ => return None,
		};
		let buttons = report[offset];
		let extended = report[offset + 1];
		let analog = |index: usize| report.get(index).copied().unwrap_or_default();
		let hat = buttons & 0x0F;
		let mut mask = 0;
		for (name, down) in [
			("PadWest", buttons & 0x10 != 0),
			("PadSouth", buttons & 0x20 != 0),
			("PadEast", buttons & 0x40 != 0),
			("PadNorth", buttons & 0x80 != 0),
			// 8 and above is the centred-and-released state.
			("PadUp", matches!(hat, 0 | 1 | 7)),
			("PadRight", matches!(hat, 1..=3)),
			("PadDown", matches!(hat, 3..=5)),
			("PadLeft", matches!(hat, 5..=7)),
			("PadL1", extended & 0x01 != 0),
			("PadR1", extended & 0x02 != 0),
			("PadL2", analog(l2) >= TRIGGER_THRESHOLD),
			("PadR2", analog(r2) >= TRIGGER_THRESHOLD),
			("PadL3", extended & 0x40 != 0),
			("PadR3", extended & 0x80 != 0),
		] {
			if down {
				mask |= polled_key_bit(name).unwrap_or(0);
			}
		}
		Some(mask)
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		fn bit(name: &str) -> u32 {
			polled_key_bit(name).expect("polled key")
		}

		#[test]
		fn gated_mouse_hold_still_releases() {
			// Live press and release.
			assert!(mouse_hold_edge(false, true, false, true));
			assert!(mouse_hold_edge(true, false, false, true));
			// Gated: the press is dropped, the release still lands.
			assert!(!mouse_hold_edge(false, true, false, false));
			assert!(mouse_hold_edge(true, false, false, false));
			// No edge, and toggles keep their press-only behaviour.
			assert!(!mouse_hold_edge(true, true, false, true));
			assert!(!mouse_hold_edge(false, true, true, true));
			assert!(!mouse_hold_edge(true, false, true, true));
		}

		#[test]
		fn altgr_drops_synthetic_left_ctrl() {
			let left_ctrl = bit("ControlLeft");
			let right_alt = bit("AltRight");
			assert_eq!(altgr_filter(right_alt | left_ctrl), right_alt);
			assert_eq!(altgr_filter(left_ctrl), left_ctrl);
		}

		/// Bluetooth reports carry the button block at the extended offset, and the
		/// trigger bytes at their matching positions.
		#[test]
		fn pad_bits_reads_bluetooth_layouts() {
			// Full Bluetooth report: buttons at 9, triggers at 6 and 7.
			let full = [0x31, 0x80, 0x80, 0x80, 0x80, 0, 200, 0, 0, 0x28, 0, 0];
			// Simple Bluetooth report: buttons at 5, triggers at 8 and 9.
			let simple = [0x01, 0x80, 0x80, 0x80, 0x80, 0x28, 0, 0, 200, 0];
			for report in [&full[..], &simple[..]] {
				assert_eq!(pad_bits(report), Some(bit("PadSouth") | bit("PadL2")));
			}
		}

		#[test]
		fn pad_bits_usb_cross_and_l2() {
			let report = [0x01, 0x80, 0x80, 0x80, 0x80, 200, 0, 0, 0x28, 0, 0, 0];
			assert_eq!(pad_bits(&report), Some(bit("PadSouth") | bit("PadL2")));
		}

		#[test]
		fn pad_bits_hat_sets_both_directions() {
			let mut report = [0u8; 12];
			report[0] = 0x01;
			report[8] = 0x01;
			assert_eq!(pad_bits(&report), Some(bit("PadUp") | bit("PadRight")));
			// Centred is still a report this pad sent, so the mask is known to be empty.
			report[8] = 0x08;
			assert_eq!(pad_bits(&report), Some(0));
		}

		/// A short report or an unknown report id yields no mask instead of reading past
		/// the buffer.
		#[test]
		fn pad_bits_rejects_short_or_unknown_reports() {
			let bt_full = [0x31, 0, 0, 0];
			let unknown = [0x02, 0, 0, 0, 0, 0, 0, 0, 0x28, 0, 0, 0];
			let short = [0x01];
			for report in [&[][..], &short[..], &unknown[..], &bt_full[..]] {
				assert_eq!(pad_bits(report), None);
			}
		}

		#[test]
		fn carried_buttons_expire_until_confirmed() {
			let silence = |quiet: &mut Quiet| (0..=PAD_SILENT_READS).any(|_| quiet.gone());
			// A reopened pad that never reports releases what the old reader held.
			assert!(silence(&mut Quiet::reopened(true)));
			// One change-only report confirms them; silence then keeps the hold.
			let mut quiet = Quiet::reopened(true);
			quiet.report(&[0x01, 0x80, 0x80, 0x80, 0x80, 0x28, 0, 0, 0, 0]);
			assert!(!silence(&mut quiet));
			// A streaming pad that stops is gone.
			quiet.report(&[0x01; 12]);
			assert!(silence(&mut quiet));
		}
	}
}

/// macOS has no global registration for a lone modifier, so its key state is polled with
/// CoreGraphics instead. There is no controller support here.
#[cfg(target_os = "macos")]
mod mac_poller {
	use super::{KeyChord, KeybindAction, NativeInput, polled_fold::Polled, polled_key_bit};
	use std::sync::{
		Arc,
		atomic::{AtomicBool, AtomicU32, Ordering},
	};
	use std::time::Duration;

	/// Well under an ordinary click, which stays down for tens of milliseconds.
	const PERIOD: Duration = Duration::from_millis(10);

	/// State the poller thread shares with `Hotkeys` and the focused input path.
	#[derive(Clone, Default)]
	pub(super) struct Shared {
		/// Whether global bindings are enabled.
		pub(super) global: Arc<AtomicBool>,
		/// Whether the Serein window is focused.
		pub(super) focused: Arc<AtomicBool>,
		/// Whether the settings UI is recording a chord, so the mask must stay live.
		pub(super) capturing: Arc<AtomicBool>,
		/// Polled keys down, also published for chord capture.
		pub(super) down: Arc<AtomicU32>,
	}

	/// Runs while a voice action is bound to a polled key, or while capturing a chord.
	pub(super) struct Poller {
		stop: Arc<AtomicBool>,
		thread: Option<std::thread::JoinHandle<()>>,
	}

	impl Poller {
		/// Starts the key poller when a polled key is bound or a chord is being captured;
		/// a bit doubles as the polled identity so duplicate keys fold together.
		pub(super) fn start(
			polled: &[(KeybindAction, KeyChord)],
			shared: &Shared,
			native: Arc<NativeInput>,
		) -> Option<Self> {
			// A bit doubles as the polled identity, so a second action on the same key folds
			// into the first and both transitions keep one hold each.
			let mut watched: Vec<(KeybindAction, u32)> = Vec::new();
			for (action, chord) in polled {
				if let Some(bit) = polled_key_bit(&chord.key)
					&& !watched.iter().any(|(_, known)| *known == bit)
				{
					watched.push((*action, bit));
				}
			}
			if watched.is_empty() && !shared.capturing.load(Ordering::Relaxed) {
				return None;
			}
			let stop = Arc::new(AtomicBool::new(false));
			let stopped = stop.clone();
			let global = shared.global.clone();
			let focused = shared.focused.clone();
			let capture_flag = shared.capturing.clone();
			let down = shared.down.clone();
			let thread = std::thread::Builder::new()
				.name("serein-input-keybinds".into())
				.spawn(move || {
					// CGKeyCode values for the side modifiers `POLLED_KEYS` can report.
					let keyboard: [(u32, u16); 6] = [
						("ControlLeft", 0x3B),
						("ControlRight", 0x3E),
						("ShiftLeft", 0x38),
						("ShiftRight", 0x3C),
						("AltLeft", 0x3A),
						("AltRight", 0x3D),
					]
					.map(|(name, code)| (polled_key_bit(name).unwrap_or(0), code));
					let sample_keys = || {
						let mut now = 0;
						for (bit, code) in keyboard {
							if key_down(code) {
								now |= bit;
							}
						}
						now
					};
					// A restart must not re-fire a held toggle, so start from what is down.
					let mut polled = Polled::new(watched, sample_keys());
					while !stopped.load(Ordering::Relaxed) {
						// Recording reads the raw mask, so publishing alone is gated.
						let now = sample_keys();
						down.store(now, Ordering::Relaxed);
						let live = (global.load(Ordering::Relaxed)
							|| focused.load(Ordering::Relaxed))
							&& !capture_flag.load(Ordering::Relaxed);
						for (id, action, pressed) in polled.tick(now, live) {
							native.apply(id, action, pressed);
						}
						std::thread::sleep(PERIOD);
					}
					// A restart must not inherit a press, so release everything still held.
					// Toggle releases are ignored by `apply`, matching the running loop.
					for (id, action, _) in polled.release() {
						native.apply(id, action, false);
					}
				})
				.ok()?;
			Some(Self {
				stop,
				thread: Some(thread),
			})
		}
	}

	impl Drop for Poller {
		/// Stops and joins the CGEvent poller before the next one can start.
		fn drop(&mut self) {
			self.stop.store(true, Ordering::Relaxed);
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
		}
	}

	/// Whether the physical key with this CGKeyCode is down, focused or not.
	#[allow(unsafe_code)]
	fn key_down(code: u16) -> bool {
		#[link(name = "CoreGraphics", kind = "framework")]
		unsafe extern "C" {
			fn CGEventSourceKeyState(state: i32, keycode: u16) -> bool;
		}
		// SAFETY: reads the combined session's key state for one key code; no pointers.
		unsafe { CGEventSourceKeyState(0, code) }
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::atomic::{AtomicBool, Ordering};

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

	/// A focused text field may suppress polled keyboard keys, but never a pad hold.
	#[cfg(any(target_os = "windows", target_os = "macos"))]
	#[test]
	fn polled_keyboard_holds_exclude_pad_buttons() {
		let id =
			|name: &str| POLLED_ID + polled_key_bit(name).expect("polled key").trailing_zeros();
		assert!(polled_keyboard_hold(id("ShiftLeft")));
		assert!(polled_keyboard_hold(id("ControlRight")));
		assert!(!polled_keyboard_hold(id("PadSouth")));
		// Registered native hotkeys use unrelated small identities.
		assert!(!polled_keyboard_hold(3));
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
