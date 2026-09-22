//! Compositor-level window hiding for Wayland sessions where winit cannot hide or minimize.
//!
//! Hyprland has no minimize concept and ignores `xdg_toplevel.set_minimized`, and winit's Wayland
//! backend cannot unmap a surface. Its IPC socket can still park this process's windows on a
//! named special workspace, which is the established Hyprland way to hide a window to the tray.

/// Commands the running compositor to hide or restore this process's windows.
#[derive(Clone, Debug)]
pub struct Hider {
	#[cfg(target_os = "linux")]
	socket: std::path::PathBuf,
}

#[cfg(not(target_os = "linux"))]
impl Hider {
	/// Other platforms hide windows through winit, so nothing is detected here.
	pub fn detect() -> Option<Self> {
		None
	}
	pub fn hide(&self) {}
	pub fn show(&self) {}
}

#[cfg(target_os = "linux")]
const WORKSPACE: &str = "special:serein-tray";
#[cfg(target_os = "linux")]
const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

#[cfg(target_os = "linux")]
impl Hider {
	/// Detects a compositor that needs and supports IPC hiding. `None` means winit's own
	/// visibility or minimize commands are the only option.
	pub fn detect() -> Option<Self> {
		std::env::var_os("WAYLAND_DISPLAY")?;
		let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
		if signature.is_empty() || signature.contains(['/', '\0']) {
			return None;
		}
		let runtime = std::env::var_os("XDG_RUNTIME_DIR")
			.map(std::path::PathBuf::from)
			.map(|dir| dir.join("hypr"));
		let socket = runtime
			.into_iter()
			.chain(std::iter::once(std::path::PathBuf::from("/tmp/hypr")))
			.map(|dir| dir.join(&signature).join(".socket.sock"))
			.find(|path| path.exists())?;
		Some(Self { socket })
	}

	/// Moves this process's windows out of sight without focusing the hidden workspace.
	pub fn hide(&self) {
		self.run(move |socket| move_window(socket, WORKSPACE, Some(WORKSPACE)));
	}

	/// Brings this process's windows back to the workspace the user is looking at.
	pub fn show(&self) {
		self.run(move |socket| {
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

	fn run(
		&self,
		command: impl FnOnce(&std::path::Path) -> Result<(), Box<dyn std::error::Error>>
		+ Send
		+ 'static,
	) {
		let socket = self.socket.clone();
		// Local socket traffic stays off the render thread; a stalled compositor must not stall the UI.
		std::thread::spawn(move || {
			if let Err(error) = command(&socket) {
				eprintln!("Hyprland window hiding failed: {error}");
			}
		});
	}
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
	let quoted = workspace.replace('\\', "\\\\").replace('"', "\\\"");
	let pid = std::process::id();
	let command = format!(
		"dispatch hl.dsp.window.move({{ window = \"pid:{pid}\", workspace = \"{quoted}\", follow = false }})"
	);
	// Transport failures are not retried: only a compositor rejection allows the old syntax.
	let reply = request(socket, &command)?;
	if reply.trim_ascii() == b"ok" {
		return Ok(());
	}
	if let Some(workspace) = legacy_workspace {
		let reply = request(
			socket,
			&format!("dispatch movetoworkspacesilent {workspace},pid:{pid}"),
		)?;
		if reply.trim_ascii() == b"ok" {
			return Ok(());
		}
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
