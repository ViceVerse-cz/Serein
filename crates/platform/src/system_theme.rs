//! Desktop light/dark preference for windowing systems that do not report one.
//!
//! Windows and macOS deliver the preference through winit, so egui follows them already.
//! Wayland and X11 report nothing, so Linux reads the freedesktop Settings portal, keeps
//! listening for changes, and falls back to a bounded one-shot `gsettings` query.

/// Latest desktop color-scheme preference, updated off the UI thread.
pub struct SystemTheme {
	#[cfg(target_os = "linux")]
	state: std::sync::Arc<std::sync::atomic::AtomicU8>,
	#[cfg(target_os = "linux")]
	task: tokio::task::JoinHandle<()>,
}

#[cfg(not(target_os = "linux"))]
impl SystemTheme {
	/// Other platforms report the system theme through winit, so nothing is watched here.
	pub fn watch(
		_runtime: &tokio::runtime::Runtime,
		_wake: impl Fn() + Send + Sync + 'static,
	) -> Self {
		Self {}
	}
	/// `Some(true)` when the desktop prefers dark; `None` while unknown or undetectable.
	pub fn dark(&self) -> Option<bool> {
		None
	}
}

#[cfg(target_os = "linux")]
const UNKNOWN: u8 = 0;
#[cfg(target_os = "linux")]
const DARK: u8 = 1;
#[cfg(target_os = "linux")]
const LIGHT: u8 = 2;
#[cfg(target_os = "linux")]
const PORTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
#[cfg(target_os = "linux")]
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// `gsettings get` prints one short quoted value; anything longer is not a theme name.
#[cfg(any(target_os = "linux", test))]
const MAX_OUTPUT: u64 = 256;

#[cfg(target_os = "linux")]
impl SystemTheme {
	/// Starts watching the desktop preference and calls `wake` whenever it changes.
	pub fn watch(
		runtime: &tokio::runtime::Runtime,
		wake: impl Fn() + Send + Sync + 'static,
	) -> Self {
		use std::sync::{
			Arc,
			atomic::{AtomicU8, Ordering},
		};
		let state = Arc::new(AtomicU8::new(UNKNOWN));
		let task = runtime.spawn({
			let state = state.clone();
			async move {
				let publish = |dark: bool| {
					let value = if dark { DARK } else { LIGHT };
					if state.swap(value, Ordering::Relaxed) != value {
						wake();
					}
				};
				if !portal(&publish).await
					&& let Ok(Some(dark)) = tokio::task::spawn_blocking(gsettings_dark).await
				{
					publish(dark);
				}
			}
		});
		Self { state, task }
	}
	/// `Some(true)` when the desktop prefers dark; `None` while unknown or undetectable.
	pub fn dark(&self) -> Option<bool> {
		match self.state.load(std::sync::atomic::Ordering::Relaxed) {
			DARK => Some(true),
			LIGHT => Some(false),
			_ => None,
		}
	}
}

#[cfg(target_os = "linux")]
impl Drop for SystemTheme {
	fn drop(&mut self) {
		self.task.abort();
	}
}

/// Follows `org.freedesktop.appearance color-scheme`. Returns `false` when no portal answered,
/// so the caller can try another source.
#[cfg(target_os = "linux")]
async fn portal(publish: &impl Fn(bool)) -> bool {
	use ashpd::desktop::settings::Settings;
	use futures_util::StreamExt;
	use tokio::time::timeout;
	let Ok(Ok(settings)) = timeout(PORTAL_TIMEOUT, async {
		let connection = ashpd::zbus::connection::Builder::session()?
			.max_queued(8)
			.build()
			.await?;
		Settings::with_connection(connection).await
	})
	.await
	else {
		return false;
	};
	// Subscribe before reading so a change between the two is not lost.
	let Ok(Ok(changes)) = timeout(PORTAL_TIMEOUT, settings.receive_color_scheme_changed()).await
	else {
		return false;
	};
	let Ok(Ok(scheme)) = timeout(PORTAL_TIMEOUT, settings.color_scheme()).await else {
		return false;
	};
	publish(portal_dark(scheme));
	let mut changes = std::pin::pin!(changes);
	while let Some(scheme) = changes.next().await {
		publish(portal_dark(scheme));
	}
	true
}

/// GNOME reports its Light style as "no preference", and the Adwaita window frame, GTK and
/// browsers all render that light, so only an explicit dark preference selects dark.
#[cfg(target_os = "linux")]
fn portal_dark(scheme: ashpd::desktop::settings::ColorScheme) -> bool {
	scheme == ashpd::desktop::settings::ColorScheme::PreferDark
}

/// One-shot GNOME settings query for sessions without a Settings portal.
#[cfg(target_os = "linux")]
fn gsettings_dark() -> Option<bool> {
	let get = |key| {
		bounded_output(
			std::process::Command::new("gsettings").args([
				"get",
				"org.gnome.desktop.interface",
				key,
			]),
			COMMAND_TIMEOUT,
		)
	};
	let scheme = get("color-scheme");
	let scheme = scheme.as_deref().map(unquote);
	if matches!(scheme, Some("prefer-dark" | "prefer-light")) {
		return gnome_dark(scheme, None);
	}
	let gtk_theme = get("gtk-theme");
	gnome_dark(scheme, gtk_theme.as_deref().map(unquote))
}

/// Maps GNOME's `color-scheme` and, where that is unset or absent, the `gtk-theme` name.
#[cfg(any(target_os = "linux", test))]
fn gnome_dark(scheme: Option<&str>, gtk_theme: Option<&str>) -> Option<bool> {
	match scheme {
		Some("prefer-dark") => Some(true),
		Some("prefer-light") => Some(false),
		_ => gtk_theme
			.map(|name| name.to_ascii_lowercase().contains("dark"))
			.or(scheme.map(|_| false)),
	}
}

#[cfg(any(target_os = "linux", test))]
fn unquote(output: &str) -> &str {
	output.trim().trim_matches('\'')
}

/// Runs `command` and returns its stdout, or `None` on failure, excess output or `timeout`.
#[cfg(any(target_os = "linux", test))]
fn bounded_output(
	command: &mut std::process::Command,
	timeout: std::time::Duration,
) -> Option<String> {
	use std::{io::Read, process::Stdio};
	let mut child = command
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::null())
		.spawn()
		.ok()?;
	let deadline = std::time::Instant::now() + timeout;
	loop {
		match child.try_wait() {
			Ok(Some(status)) if status.success() => {
				let mut output = String::new();
				child
					.stdout
					.take()?
					.take(MAX_OUTPUT + 1)
					.read_to_string(&mut output)
					.ok()?;
				return (output.len() as u64 <= MAX_OUTPUT).then_some(output);
			}
			Ok(None) if std::time::Instant::now() < deadline => {
				std::thread::sleep(std::time::Duration::from_millis(10));
			}
			Ok(Some(_)) => return None,
			_ => {
				let _ = child.kill();
				let _ = child.wait();
				return None;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn gnome_values_map_to_the_desktop_appearance() {
		assert_eq!(gnome_dark(Some("prefer-dark"), None), Some(true));
		assert_eq!(
			gnome_dark(Some("prefer-light"), Some("Adwaita-dark")),
			Some(false)
		);
		// GNOME's Light style is `default`; legacy dark GTK themes still count as dark.
		assert_eq!(gnome_dark(Some("default"), Some("Adwaita")), Some(false));
		assert_eq!(
			gnome_dark(Some("default"), Some("Adwaita-dark")),
			Some(true)
		);
		assert_eq!(gnome_dark(Some("default"), None), Some(false));
		// Releases without `color-scheme` only have the theme name.
		assert_eq!(gnome_dark(None, Some("Yaru-Dark")), Some(true));
		assert_eq!(gnome_dark(None, Some("Yaru")), Some(false));
		assert_eq!(gnome_dark(None, None), None);
		assert_eq!(unquote("'prefer-dark'\n"), "prefer-dark");
	}

	#[cfg(unix)]
	#[test]
	fn commands_are_bounded_by_time_and_output() {
		let run = |script: &str, timeout| {
			bounded_output(
				std::process::Command::new("sh").args(["-c", script]),
				timeout,
			)
		};
		let quick = std::time::Duration::from_secs(5);
		assert_eq!(
			run("printf \"'default'\"", quick).as_deref().map(unquote),
			Some("default")
		);
		assert_eq!(run("exit 1", quick), None);
		assert_eq!(run("head -c 300 /dev/zero", quick), None);
		let started = std::time::Instant::now();
		assert_eq!(run("sleep 5", std::time::Duration::from_millis(50)), None);
		assert!(started.elapsed() < std::time::Duration::from_secs(4));
	}
}
