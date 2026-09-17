//! Native global voice bindings, using the desktop portal on Wayland.
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use model::{KeyChord, KeybindAction, Keybinds};
#[cfg(target_os = "linux")]
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU8, Ordering},
};

const READY: &str = "Global voice keybinds are enabled.";
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

pub struct Hotkeys {
	manager: Option<GlobalHotKeyManager>,
	registered: [Option<HotKey>; 3],
	bindings: Option<[KeyChord; 3]>,
	ptt_down: bool,
	pending_toggles: u8,
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
		#[cfg(not(target_os = "linux"))]
		let _ = wake;
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
			ptt_down: false,
			pending_toggles: 0,
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
			wake: Arc::new(wake),
		}
	}

	pub fn sync(&mut self, keybinds: &Keybinds, _runtime: &tokio::runtime::Runtime) {
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

		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some() {
			if let Some(task) = self.portal.take() {
				task.abort();
			}
			self.portal_pending.store(0, Ordering::Relaxed);
			self.portal_registered.store(0, Ordering::Relaxed);
			self.portal_ptt_down.store(false, Ordering::Relaxed);
			self.portal_status.store(1, Ordering::Relaxed);
			let pending = self.portal_pending.clone();
			let registered = self.portal_registered.clone();
			let ptt_down = self.portal_ptt_down.clone();
			let status = self.portal_status.clone();
			let wake = self.wake.clone();
			self.portal = Some(_runtime.spawn(async move {
				let _ = portal(
					next,
					pending,
					registered.clone(),
					ptt_down.clone(),
					status.clone(),
					wake.clone(),
				)
				.await;
				registered.store(0, Ordering::Relaxed);
				ptt_down.store(false, Ordering::Relaxed);
				status.store(3, Ordering::Relaxed);
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
		} else if modifier_required {
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
						(TOGGLE_MUTE, HotKeyState::Pressed) => self.pending_toggles ^= 1,
						(TOGGLE_DEAFEN, HotKeyState::Pressed) => self.pending_toggles ^= 2,
						_ => {}
					}
				}
			}
		}
	}

	pub fn take_toggle_pending(&mut self) -> u8 {
		let pending = std::mem::take(&mut self.pending_toggles);
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
		#[cfg(target_os = "linux")]
		return self.ptt_down || self.portal_ptt_down.load(Ordering::Relaxed);
		#[cfg(not(target_os = "linux"))]
		self.ptt_down
	}

	pub fn status(&self) -> &'static str {
		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some() {
			return match self.portal_status.load(Ordering::Relaxed) {
				1 => WAYLAND_PENDING,
				2 => READY,
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
) -> Result<(), ashpd::Error> {
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
	if shortcuts.is_empty() {
		status.store(2, Ordering::Relaxed);
		wake();
		return Ok(());
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
	status.store(if mask == 0 { 3 } else { 2 }, Ordering::Relaxed);
	wake();
	loop {
		tokio::select! {
			_ = closed.next() => return Ok(()),
			event = activated.next() => {
				let Some(event) = event else { return Ok(()); };
				match event.shortcut_id() {
					"push-to-talk" => ptt_down.store(true, Ordering::Relaxed),
					"mute" => { pending.fetch_or(1, Ordering::Relaxed); }
					"deafen" => { pending.fetch_or(2, Ordering::Relaxed); }
					_ => continue,
				}
				wake();
			}
			event = deactivated.next() => {
				let Some(event) = event else { return Ok(()); };
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
	if !chord.is_valid() || chord.modifiers == 0 {
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
