//! Compositor-level window hiding for Wayland sessions where winit cannot hide or minimize.
//!
//! Hyprland has no minimize concept and ignores `xdg_toplevel.set_minimized`, and winit's Wayland
//! backend cannot unmap a surface. Its IPC socket can still park this process's windows on a
//! named special workspace, which is the established Hyprland way to hide a window to the tray.
//! KWin minimizes, but keeps the taskbar entry; its scripting interface can skip the taskbar.

#[cfg(target_os = "linux")]
#[path = "compositor/kwin.rs"]
mod kwin;

/// Commands the running compositor to hide or restore this process's windows.
#[derive(Clone, Debug)]
pub struct Hider {
	#[cfg(target_os = "linux")]
	backend: Backend,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
enum Backend {
	Hyprland(Hyprland),
	KWin(kwin::KWin),
}

#[cfg(not(target_os = "linux"))]
impl Hider {
	/// Other platforms hide windows through winit, so nothing is detected here.
	pub fn detect() -> Option<Self> {
		None
	}
	pub fn available(&self) -> bool {
		false
	}
	pub fn hide(&self) {}
	pub fn show(&self) {}
	pub fn minimize(&self) -> bool {
		false
	}
}

#[cfg(target_os = "linux")]
const WORKSPACE: &str = "special:serein-tray";
#[cfg(target_os = "linux")]
const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

#[cfg(target_os = "linux")]
type Error = Box<dyn std::error::Error + Send + Sync>;

/// A single worker orders moves, while the watch slot retains only the latest visibility.
#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct Hyprland {
	desired: tokio::sync::watch::Sender<bool>,
}

#[cfg(target_os = "linux")]
impl Hyprland {
	fn new(socket: std::path::PathBuf) -> Option<Self> {
		let runtime = tokio::runtime::Handle::try_current().ok()?;
		let (desired, mut changes) = tokio::sync::watch::channel(true);
		runtime.spawn(async move {
			while changes.changed().await.is_ok() {
				let shown = *changes.borrow_and_update();
				match tokio::time::timeout(TIMEOUT, visibility(&socket, shown)).await {
					Ok(Ok(())) => {}
					Ok(Err(error)) => eprintln!("Hyprland window hiding failed: {error}"),
					Err(_) => eprintln!("Hyprland window hiding failed: timed out"),
				}
			}
		});
		Some(Self { desired })
	}

	fn set(&self, shown: bool) {
		self.desired.send_replace(shown);
	}
}

#[cfg(target_os = "linux")]
impl Hider {
	/// Detects a compositor that needs and supports IPC hiding. `None` means winit's own
	/// visibility or minimize commands are the only option. Both workers need a Tokio runtime.
	pub fn detect() -> Option<Self> {
		std::env::var_os("WAYLAND_DISPLAY")?;
		let backend = hyprland_socket()
			.and_then(Hyprland::new)
			.map(Backend::Hyprland)
			.or_else(|| kwin::KWin::detect().map(Backend::KWin))?;
		Some(Self { backend })
	}

	/// KWin's scripting interface is probed in the background and may be denied (Flatpak).
	pub fn available(&self) -> bool {
		match &self.backend {
			Backend::Hyprland(hyprland) => !hyprland.desired.is_closed(),
			Backend::KWin(kwin) => kwin.available(),
		}
	}

	/// Moves this process's windows out of sight without focusing the hidden workspace.
	pub fn hide(&self) {
		match &self.backend {
			Backend::Hyprland(hyprland) => hyprland.set(false),
			Backend::KWin(kwin) => kwin.set(kwin::State::Hidden),
		}
	}

	/// Tray Minimize; returns whether the window also left the taskbar. Hyprland has no
	/// minimize, so it parks the window instead.
	pub fn minimize(&self) -> bool {
		match &self.backend {
			Backend::Hyprland(_) => {
				self.hide();
				true
			}
			Backend::KWin(kwin) => {
				kwin.set(kwin::State::Minimized);
				false
			}
		}
	}

	/// Brings this process's windows back to the workspace the user is looking at.
	pub fn show(&self) {
		match &self.backend {
			Backend::Hyprland(hyprland) => hyprland.set(true),
			Backend::KWin(kwin) => kwin.set(kwin::State::Shown),
		}
	}
}

#[cfg(target_os = "linux")]
fn hyprland_socket() -> Option<std::path::PathBuf> {
	let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
	if signature.is_empty() || signature.contains(['/', '\0']) {
		return None;
	}
	let runtime = std::env::var_os("XDG_RUNTIME_DIR")
		.map(std::path::PathBuf::from)
		.map(|dir| dir.join("hypr"));
	runtime
		.into_iter()
		.chain(std::iter::once(std::path::PathBuf::from("/tmp/hypr")))
		.map(|dir| dir.join(&signature).join(".socket.sock"))
		.find(|path| path.exists())
}

#[cfg(target_os = "linux")]
async fn visibility(socket: &std::path::Path, shown: bool) -> Result<(), Error> {
	if !shown {
		return move_window(socket, WORKSPACE, Some(WORKSPACE)).await;
	}
	let active = request(socket, "j/activeworkspace").await?;
	let active: serde_json::Value = serde_json::from_slice(&active)?;
	let id = active
		.get("id")
		.and_then(|value| value.as_i64())
		.map(|id| id.to_string());
	let workspace = active
		.get("address")
		.and_then(|value| value.as_str())
		.or(id.as_deref())
		.ok_or("active workspace has no address or id")?;
	move_window(socket, workspace, id.as_deref()).await
}

#[cfg(target_os = "linux")]
async fn move_window(
	socket: &std::path::Path,
	workspace: &str,
	legacy_workspace: Option<&str>,
) -> Result<(), Error> {
	if workspace.is_empty() || workspace.len() > 1024 || workspace.chars().any(char::is_control) {
		return Err("invalid workspace address".into());
	}
	// Workspace names are data, never Lua source. Control characters are rejected above.
	let pid = std::process::id();
	let native_workspace = legacy_workspace.unwrap_or(workspace);
	let reply = request(
		socket,
		&format!("dispatch movetoworkspacesilent {native_workspace},pid:{pid}"),
	)
	.await?;
	if reply.trim_ascii() == b"ok" {
		return Ok(());
	}
	let quoted = workspace.replace('\\', "\\\\").replace('"', "\\\"");
	let command = format!(
		"dispatch hl.dsp.window.move({{ window = \"pid:{pid}\", workspace = \"{quoted}\", follow = false }})"
	);
	let reply = request(socket, &command).await?;
	if reply.trim_ascii() == b"ok" {
		return Ok(());
	}
	Err("Hyprland rejected window move".into())
}

#[cfg(target_os = "linux")]
async fn request(socket: &std::path::Path, command: &str) -> Result<Vec<u8>, Error> {
	use tokio::io::{AsyncReadExt, AsyncWriteExt};
	let mut stream = tokio::net::UnixStream::connect(socket).await?;
	stream.write_all(command.as_bytes()).await?;
	let mut reply = Vec::with_capacity(256);
	stream.take(64 * 1024 + 1).read_to_end(&mut reply).await?;
	if reply.len() > 64 * 1024 {
		return Err("Hyprland reply is too large".into());
	}
	Ok(reply)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
	use super::*;
	use std::io::{Read, Write};
	use std::os::unix::net::UnixListener;

	#[tokio::test]
	async fn stalled_socket_times_out_and_latest_visibility_can_resume() {
		use tokio::io::{AsyncReadExt, AsyncWriteExt};
		let pid = std::process::id();
		let path = std::env::temp_dir().join(format!("serein-move-timeout-{pid}.sock"));
		let listener = tokio::net::UnixListener::bind(&path).unwrap();
		let hider = Hider {
			backend: Backend::Hyprland(Hyprland::new(path.clone()).unwrap()),
		};
		hider.hide();
		let (mut stalled, _) = tokio::time::timeout(TIMEOUT, listener.accept())
			.await
			.unwrap()
			.unwrap();
		let hidden = format!("dispatch movetoworkspacesilent {WORKSPACE},pid:{pid}");
		let mut command = vec![0; hidden.len()];
		stalled.read_exact(&mut command).await.unwrap();
		hider.show();
		// There is no reply on the original connection. It must retire before the next move.
		let (mut query, _) = tokio::time::timeout(TIMEOUT * 3, listener.accept())
			.await
			.unwrap()
			.unwrap();
		let mut command = vec![0; "j/activeworkspace".len()];
		query.read_exact(&mut command).await.unwrap();
		assert_eq!(command, b"j/activeworkspace");
		assert_eq!(stalled.read(&mut [0_u8; 1]).await.unwrap(), 0);
		query.write_all(br#"{"id":4}"#).await.unwrap();
		drop(query);
		let (mut restored, _) = tokio::time::timeout(TIMEOUT, listener.accept())
			.await
			.unwrap()
			.unwrap();
		let shown = format!("dispatch movetoworkspacesilent 4,pid:{pid}");
		let mut command = vec![0; shown.len()];
		restored.read_exact(&mut command).await.unwrap();
		assert_eq!(command, shown.as_bytes());
		restored.write_all(b"ok").await.unwrap();
		drop(restored);
		drop(hider);
		drop(listener);
		std::fs::remove_file(path).unwrap();
	}

	#[tokio::test]
	async fn pending_move_serializes_and_coalesces_later_visibility() {
		use tokio::io::{AsyncReadExt, AsyncWriteExt};
		let pid = std::process::id();
		let path = std::env::temp_dir().join(format!("serein-move-order-{pid}.sock"));
		let listener = tokio::net::UnixListener::bind(&path).unwrap();
		let hider = Hider {
			backend: Backend::Hyprland(Hyprland::new(path.clone()).unwrap()),
		};
		let hidden = format!("dispatch movetoworkspacesilent {WORKSPACE},pid:{pid}");
		hider.hide();
		let (mut first, _) = tokio::time::timeout(TIMEOUT, listener.accept())
			.await
			.unwrap()
			.unwrap();
		let mut command = vec![0; hidden.len()];
		first.read_exact(&mut command).await.unwrap();
		assert_eq!(command, hidden.as_bytes());
		// The first reply is deliberately pending while the user changes their mind.
		hider.show();
		hider.hide();
		hider.show();
		let overlapped =
			tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
				.await
				.is_ok();
		first.write_all(b"ok").await.unwrap();
		drop(first);
		if !overlapped {
			let (mut query, _) = tokio::time::timeout(TIMEOUT, listener.accept())
				.await
				.unwrap()
				.unwrap();
			let mut command = vec![0; "j/activeworkspace".len()];
			query.read_exact(&mut command).await.unwrap();
			assert_eq!(command, b"j/activeworkspace");
			query
				.write_all(br#"{"id":3,"address":"0xabc"}"#)
				.await
				.unwrap();
			drop(query);
			let (mut restored, _) = tokio::time::timeout(TIMEOUT, listener.accept())
				.await
				.unwrap()
				.unwrap();
			let shown = format!("dispatch movetoworkspacesilent 3,pid:{pid}");
			let mut command = vec![0; shown.len()];
			restored.read_exact(&mut command).await.unwrap();
			assert_eq!(command, shown.as_bytes());
			restored.write_all(b"ok").await.unwrap();
			drop(restored);
			assert!(
				tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept())
					.await
					.is_err()
			);
		}
		drop(hider);
		drop(listener);
		std::fs::remove_file(path).unwrap();
		assert!(
			!overlapped,
			"only one move may be in flight while later visibility changes coalesce"
		);
	}

	#[tokio::test]
	async fn native_move_uses_id_and_lua_fallback_keeps_address() {
		for (index, (workspace, legacy, reject)) in [
			(WORKSPACE, Some(WORKSPACE), false),
			("0xabc", Some("3"), false),
			("0xabc", Some("3"), true),
			("3", None, false),
		]
		.into_iter()
		.enumerate()
		{
			let pid = std::process::id();
			let path = std::env::temp_dir().join(format!("serein-move-{pid}-{index}.sock"));
			let listener = UnixListener::bind(&path).unwrap();
			let server = std::thread::spawn(move || {
				let mut commands = vec![format!(
					"dispatch movetoworkspacesilent {},pid:{pid}",
					legacy.unwrap_or(workspace)
				)];
				if reject {
					commands.push(format!(
						"dispatch hl.dsp.window.move({{ window = \"pid:{pid}\", workspace = \"{workspace}\", follow = false }})"
					));
				}
				for (i, command) in commands.iter().enumerate() {
					let (mut stream, _) = listener.accept().unwrap();
					stream.set_read_timeout(Some(TIMEOUT)).unwrap();
					let mut received = vec![0; command.len()];
					stream.read_exact(&mut received).unwrap();
					assert_eq!(received, command.as_bytes());
					stream
						.write_all(if reject && i == 0 {
							b"invalid dispatcher"
						} else {
							b"ok"
						})
						.unwrap();
				}
			});
			let result = tokio::time::timeout(TIMEOUT, move_window(&path, workspace, legacy))
				.await
				.unwrap();
			server.join().unwrap();
			std::fs::remove_file(path).unwrap();
			result.unwrap();
		}
	}
}
