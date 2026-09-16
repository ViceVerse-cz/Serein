//! Native registrations only: no keyboard capture or unbounded application event queue.
use global_hotkey::{
	GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
	hotkey::{Code, HotKey, Modifiers},
};
use std::sync::{
	Arc,
	atomic::{AtomicU8, Ordering},
};

pub struct VoiceShortcuts {
	enabled: bool,
	manager: Option<GlobalHotKeyManager>,
	keys: [HotKey; 2],
	pending: Arc<AtomicU8>,
	pressed: Arc<AtomicU8>,
	status: Arc<AtomicU8>,
	#[cfg(target_os = "linux")]
	wake: Arc<dyn Fn() + Send + Sync>,
	#[cfg(target_os = "linux")]
	portal: Option<tokio::task::JoinHandle<()>>,
}

impl VoiceShortcuts {
	pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
		let modifier = if cfg!(target_os = "macos") {
			Modifiers::SUPER
		} else {
			Modifiers::CONTROL
		};
		let keys =
			[Code::KeyM, Code::KeyD].map(|key| HotKey::new(Some(modifier | Modifiers::SHIFT), key));
		let pending = Arc::new(AtomicU8::new(0));
		let pressed = Arc::new(AtomicU8::new(0));
		let wake = Arc::new(wake);
		GlobalHotKeyEvent::set_event_handler(Some({
			let pending = pending.clone();
			let pressed = pressed.clone();
			let wake = wake.clone();
			move |event: GlobalHotKeyEvent| {
				let Some(index) = keys.iter().position(|key| key.id() == event.id) else {
					return;
				};
				let mask = 1 << index;
				if record_key(&pending, &pressed, mask, event.state) {
					wake();
				}
			}
		}));
		Self {
			enabled: false,
			manager: None,
			keys,
			pending,
			pressed,
			status: Arc::new(AtomicU8::new(0)),
			#[cfg(target_os = "linux")]
			wake,
			#[cfg(target_os = "linux")]
			portal: None,
		}
	}

	pub fn sync(&mut self, enabled: bool, _runtime: &tokio::runtime::Runtime) {
		if self.enabled == enabled {
			return;
		}
		self.enabled = enabled;
		self.manager = None;
		#[cfg(target_os = "linux")]
		if let Some(task) = self.portal.take() {
			task.abort();
		}
		self.pending.store(0, Ordering::Relaxed);
		self.pressed.store(0, Ordering::Relaxed);
		self.status.store(0, Ordering::Relaxed);
		if !enabled {
			return;
		}
		#[cfg(target_os = "linux")]
		if std::env::var_os("WAYLAND_DISPLAY").is_some() {
			self.status.store(1, Ordering::Relaxed);
			let pending = self.pending.clone();
			let status = self.status.clone();
			let wake = self.wake.clone();
			self.portal = Some(_runtime.spawn(async move {
				let _ = portal(pending, status.clone(), wake.clone()).await;
				status.store(3, Ordering::Relaxed);
				wake();
			}));
			return;
		}
		match GlobalHotKeyManager::new().and_then(|manager| {
			// Drop the manager on either failure, releasing any partial registration.
			for key in self.keys {
				manager.register(key)?;
			}
			Ok(manager)
		}) {
			Ok(manager) => {
				self.manager = Some(manager);
				self.status.store(2, Ordering::Relaxed);
			}
			Err(_) => {
				self.status.store(3, Ordering::Relaxed);
			}
		}
	}

	pub fn take_pending(&self) -> u8 {
		let pending = self.pending.swap(0, Ordering::Relaxed);
		if self.enabled { pending } else { 0 }
	}

	pub fn status(&self) -> &'static str {
		match self.status.load(Ordering::Relaxed) {
			1 => "Approve the shortcuts in your desktop’s dialog.",
			2 => "Global voice shortcuts enabled.",
			3 => {
				"Shortcuts unavailable or denied. Free conflicting keys or enable your desktop’s GlobalShortcuts portal, then switch off and on to retry."
			}
			_ => "Global voice shortcuts disabled.",
		}
	}
}

impl Drop for VoiceShortcuts {
	fn drop(&mut self) {
		#[cfg(target_os = "linux")]
		if let Some(task) = self.portal.take() {
			task.abort();
		}
	}
}

#[cfg(target_os = "linux")]
async fn portal(
	pending: Arc<AtomicU8>,
	status: Arc<AtomicU8>,
	wake: Arc<dyn Fn() + Send + Sync>,
) -> Result<(), ashpd::Error> {
	use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
	use futures_util::StreamExt;
	// A dedicated connection makes cancellation/exit release the portal session,
	// including when the owner is still looking at the permission dialog.
	let connection = ashpd::zbus::connection::Builder::session()?
		.max_queued(16)
		.build()
		.await?;
	let proxy = GlobalShortcuts::with_connection(connection).await?;
	let session = proxy.create_session(Default::default()).await?;
	let mut activated = proxy.receive_activated().await?;
	let mut closed = session.receive_closed().await?;
	let response = proxy
		.bind_shortcuts(
			&session,
			&[
				NewShortcut::new("mute", "Toggle Serein microphone mute")
					.preferred_trigger("CTRL+SHIFT+m"),
				NewShortcut::new("deafen", "Toggle Serein deafen")
					.preferred_trigger("CTRL+SHIFT+d"),
			],
			None,
			Default::default(),
		)
		.await?
		.response()?;
	if !["mute", "deafen"]
		.iter()
		.all(|id| response.shortcuts().iter().any(|key| key.id() == *id))
	{
		session.close().await?;
		return Ok(());
	}
	status.store(2, Ordering::Relaxed);
	wake();
	loop {
		tokio::select! {
			_ = closed.next() => return Ok(()),
			event = activated.next() => {
				let Some(event) = event else { return Ok(()); };
				let mask = match event.shortcut_id() { "mute" => 1, "deafen" => 2, _ => continue };
				pending.fetch_or(mask, Ordering::Relaxed);
				wake();
			}
		}
	}
}

fn record_key(pending: &AtomicU8, pressed: &AtomicU8, mask: u8, state: HotKeyState) -> bool {
	if state == HotKeyState::Released {
		pressed.fetch_and(!mask, Ordering::Relaxed);
		false
	} else if pressed.fetch_or(mask, Ordering::Relaxed) & mask == 0 {
		// ponytail: coalesce bursts to one toggle per action; use a bounded FIFO if every tap must survive a stalled UI.
		pending.fetch_or(mask, Ordering::Relaxed);
		true
	} else {
		false
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn held_keys_toggle_once_and_release_rearms_each_action() {
		let pending = AtomicU8::new(0);
		let pressed = AtomicU8::new(0);
		assert!(record_key(&pending, &pressed, 1, HotKeyState::Pressed));
		assert_eq!(pending.swap(0, Ordering::Relaxed), 1);
		assert!(!record_key(&pending, &pressed, 1, HotKeyState::Pressed));
		assert_eq!(pending.load(Ordering::Relaxed), 0);
		assert!(record_key(&pending, &pressed, 2, HotKeyState::Pressed));
		assert!(!record_key(&pending, &pressed, 1, HotKeyState::Released));
		assert!(record_key(&pending, &pressed, 1, HotKeyState::Pressed));
		assert_eq!(pending.swap(0, Ordering::Relaxed), 3);
	}
}
