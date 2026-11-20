//! Session end notice from the desktop portal's Inhibit monitor (unverified across desktops:
//! GNOME, and Plasma since xdg-desktop-portal-kde added session monitoring).
//!
//! Plasma closes every Wayland window once at logout and waits until each is destroyed. Discord
//! (Electron) passes because hiding to the tray destroys its toplevel; winit cannot unmap a
//! Wayland surface, so a window hidden to the tray still exists there. Knowing the session is
//! ending lets that close exit instead of hiding again.

use std::sync::{
	Arc,
	atomic::{AtomicU8, Ordering},
};

const QUERY_END: u8 = 2;
const ENDING: u8 = 3;

#[derive(Clone, Debug)]
pub struct Monitor {
	state: Arc<AtomicU8>,
}

impl Monitor {
	/// Watches on the current Tokio runtime; `wake` runs after every state change.
	#[cfg(target_os = "linux")]
	pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
		let runtime = tokio::runtime::Handle::try_current().ok()?;
		let state = Arc::new(AtomicU8::new(1));
		let watched = state.clone();
		runtime.spawn(async move {
			if let Err(error) = watch(&watched, &wake).await {
				eprintln!("Session end monitor unavailable ({error}); logout may wait for Serein.");
			}
		});
		Some(Self { state })
	}

	/// Other platforms end the session through their own window or app events.
	#[cfg(not(target_os = "linux"))]
	pub fn start(_wake: impl Fn() + Send + Sync + 'static) -> Option<Self> {
		None
	}

	/// The user asked to end the session (it may still be cancelled), or it is ending.
	pub fn ending(&self) -> bool {
		self.state.load(Ordering::Acquire) >= QUERY_END
	}

	/// The session is ending and will not be cancelled.
	pub fn ended(&self) -> bool {
		self.state.load(Ordering::Acquire) == ENDING
	}
}

#[cfg(target_os = "linux")]
async fn watch(state: &AtomicU8, wake: &impl Fn()) -> ashpd::Result<()> {
	use ashpd::desktop::inhibit::{InhibitProxy, SessionState};
	use futures_util::StreamExt;
	let proxy = InhibitProxy::new().await?;
	// Subscribe first: the portal sends the current state right after creating the monitor.
	let mut changes = std::pin::pin!(proxy.receive_state_changed().await?);
	let session = proxy.create_monitor(None, Default::default()).await?;
	while let Some(change) = changes.next().await {
		let value = change.session_state();
		state.store(value as u8, Ordering::Release);
		wake();
		if value == SessionState::QueryEnd {
			// Serein never blocks logout, so answer at once instead of making the session wait.
			proxy.query_end_response(&session).await?;
		}
	}
	Ok(())
}
