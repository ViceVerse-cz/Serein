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
	Hyprland(std::path::PathBuf),
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
impl Hider {
	/// Detects a compositor that needs and supports IPC hiding. `None` means winit's own
	/// visibility or minimize commands are the only option. KWin needs a Tokio runtime context.
	pub fn detect() -> Option<Self> {
		std::env::var_os("WAYLAND_DISPLAY")?;
		let backend = hyprland_socket()
			.map(Backend::Hyprland)
			.or_else(|| kwin::KWin::detect().map(Backend::KWin))?;
		Some(Self { backend })
	}

	/// KWin's scripting interface is probed in the background and may be denied (Flatpak).
	pub fn available(&self) -> bool {
		match &self.backend {
			Backend::Hyprland(_) => true,
			Backend::KWin(kwin) => kwin.available(),
		}
	}

	/// Moves this process's windows out of sight without focusing the hidden workspace.
	pub fn hide(&self) {
		match &self.backend {
			Backend::Hyprland(socket) => run(socket, move |socket| {
				move_window(socket, WORKSPACE, Some(WORKSPACE))
			}),
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
		let socket = match &self.backend {
			Backend::Hyprland(socket) => socket,
			Backend::KWin(kwin) => return kwin.set(kwin::State::Shown),
		};
		run(socket, move |socket| {
			let active = request(socket, "j/activeworkspace")?;
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
			move_window(socket, workspace, id.as_deref())
		});
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
fn run(
	socket: &std::path::Path,
	command: impl FnOnce(&std::path::Path) -> Result<(), Box<dyn std::error::Error>> + Send + 'static,
) {
	let socket = socket.to_owned();
	// Local socket traffic stays off the render thread; a stalled compositor must not stall the UI.
	std::thread::spawn(move || {
		if let Err(error) = command(&socket) {
			eprintln!("Hyprland window hiding failed: {error}");
		}
	});
}

#[cfg(target_os = "linux")]
fn move_window(
	socket: &std::path::Path,
	workspace: &str,
	legacy_workspace: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
	if workspace.is_empty() || workspace.len() > 1024 || workspace.chars().any(char::is_control) {
		return Err("invalid workspace address".into());
	}
	// Workspace names are data, never Lua source. Control characters are rejected above.
	let pid = std::process::id();
	let native_workspace = legacy_workspace.unwrap_or(workspace);
	let reply = request(
		socket,
		&format!("dispatch movetoworkspacesilent {native_workspace},pid:{pid}"),
	)?;
	if reply.trim_ascii() == b"ok" {
		return Ok(());
	}
	let quoted = workspace.replace('\\', "\\\\").replace('"', "\\\"");
	let command = format!(
		"dispatch hl.dsp.window.move({{ window = \"pid:{pid}\", workspace = \"{quoted}\", follow = false }})"
	);
	let reply = request(socket, &command)?;
	if reply.trim_ascii() == b"ok" {
		return Ok(());
	}
	Err("Hyprland rejected window move".into())
}

#[cfg(target_os = "linux")]
fn request(socket: &std::path::Path, command: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
	use std::io::{Read, Write};
	let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
	stream.set_read_timeout(Some(TIMEOUT))?;
	stream.set_write_timeout(Some(TIMEOUT))?;
	stream.write_all(command.as_bytes())?;
	stream.flush()?;
	let mut reply = Vec::with_capacity(256);
	stream.take(64 * 1024).read_to_end(&mut reply)?;
	Ok(reply)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
	use super::*;
	use std::io::{Read, Write};
	use std::os::unix::net::UnixListener;

	#[test]
	fn native_move_uses_id_and_lua_fallback_keeps_address() {
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
			let result = move_window(&path, workspace, legacy);
			server.join().unwrap();
			std::fs::remove_file(path).unwrap();
			result.unwrap();
		}
	}
}
