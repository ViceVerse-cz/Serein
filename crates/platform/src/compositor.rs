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
		if std::env::var_os("WAYLAND_DISPLAY").is_none() {
			return None;
		}
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
		self.run(move |socket| {
			request(
				socket,
				&format!(
					"dispatch movetoworkspacesilent {WORKSPACE},pid:{}",
					std::process::id()
				),
			)
			.map(drop)
		});
	}

	/// Brings this process's windows back to the workspace the user is looking at.
	pub fn show(&self) {
		self.run(move |socket| {
			let active = request(socket, "j/activeworkspace")?;
			let id = serde_json::from_slice::<serde_json::Value>(&active)
				.ok()
				.and_then(|value| value.get("id")?.as_i64())
				.ok_or("active workspace has no id")?;
			request(
				socket,
				&format!("dispatch movetoworkspace {id},pid:{}", std::process::id()),
			)
			.map(drop)
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
fn request(socket: &std::path::Path, command: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
	use std::io::{Read, Write};
	let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
	stream.set_read_timeout(Some(TIMEOUT))?;
	stream.set_write_timeout(Some(TIMEOUT))?;
	stream.write_all(command.as_bytes())?;
	stream.flush()?;
	let mut reply = Vec::with_capacity(256);
	stream.take(64 * 1024).read_to_end(&mut reply)?;
	if command.starts_with("dispatch ") && reply.trim_ascii() != b"ok" {
		return Err(format!("{command}: {}", String::from_utf8_lossy(&reply).trim()).into());
	}
	Ok(reply)
}
