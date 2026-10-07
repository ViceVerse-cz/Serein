//! KWin hiding through its D-Bus scripting interface. On native Wayland winit can only minimize,
//! which leaves the taskbar entry; a KWin script can also skip the taskbar, pager and switcher.

use std::path::{Path, PathBuf};
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::watch;

type Error = Box<dyn std::error::Error + Send + Sync>;

const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
	Shown = 0,
	Hidden = 1,
	Minimized = 2,
}

/// Commands one background worker; the latest requested state wins.
#[derive(Clone, Debug)]
pub struct KWin {
	desired: watch::Sender<State>,
	available: Arc<AtomicBool>,
}

impl KWin {
	/// Starts the worker on the current Tokio runtime for a KDE Plasma session.
	pub fn detect() -> Option<Self> {
		let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok()?;
		if !desktop
			.split(':')
			.any(|name| name.eq_ignore_ascii_case("KDE"))
		{
			return None;
		}
		let runtime = tokio::runtime::Handle::try_current().ok()?;
		let mut dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
		// A Flatpak's process IDs are namespaced, so match its Wayland app ID instead. Its
		// per-app runtime directory is the only one KWin can read at the same path.
		let target = match std::env::var("FLATPAK_ID") {
			Ok(id) if valid_app_id(&id) => {
				dir = dir.join("app").join(&id);
				Target::App(id)
			}
			_ => Target::Pid(std::process::id()),
		};
		let (desired, changes) = watch::channel(State::Shown);
		let available = Arc::new(AtomicBool::new(false));
		runtime.spawn(worker(changes, available.clone(), dir, target));
		Some(Self { desired, available })
	}

	/// False until KWin's scripting interface answered; callers then fall back to minimizing.
	pub fn available(&self) -> bool {
		self.available.load(Ordering::Acquire)
	}

	pub fn set(&self, state: State) {
		self.desired.send_replace(state);
	}
}

enum Target {
	Pid(u32),
	App(String),
}

fn valid_app_id(id: &str) -> bool {
	id.len() <= 255
		&& id.starts_with(|first: char| first.is_ascii_alphanumeric())
		&& id
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// Supports both the KWin 6 (`windowList`, `activeWindow`, `desktops`) and KWin 5 APIs.
fn script(state: State, target: &Target) -> String {
	let matches = match target {
		Target::Pid(pid) => format!("w.pid === {pid}"),
		Target::App(id) => {
			let id = serde_json::to_string(id).unwrap_or_default();
			format!("w.desktopFileName === {id} || String(w.resourceClass) === {id}")
		}
	};
	let action = match state {
		State::Hidden => "skip(w, true); w.minimized = true;",
		State::Minimized => "skip(w, false); w.minimized = true;",
		State::Shown => {
			"skip(w, false); w.minimized = false;
		if (!w.onAllDesktops) {
			if (kwin6) w.desktops = [workspace.currentDesktop];
			else w.desktop = workspace.currentDesktop;
		}
		if (kwin6) workspace.activeWindow = w;
		else workspace.activeClient = w;"
		}
	};
	format!(
		"const kwin6 = typeof workspace.windowList === \"function\";
function skip(w, value) {{
	w.skipTaskbar = value;
	w.skipPager = value;
	w.skipSwitcher = value;
}}
for (const w of kwin6 ? workspace.windowList() : workspace.clientList()) {{
	if ({matches}) {{
		{action}
	}}
}}
"
	)
}

async fn worker(
	mut changes: watch::Receiver<State>,
	available: Arc<AtomicBool>,
	dir: PathBuf,
	target: Target,
) {
	let name = format!("serein-tray-{}", std::process::id());
	let paths = [State::Shown, State::Hidden, State::Minimized]
		.map(|state| dir.join(format!("{name}-{}.js", state as u8)));
	let connected = tokio::time::timeout(TIMEOUT, async {
		let bus = zbus::Connection::session().await?;
		let scripting =
			zbus::Proxy::new(&bus, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting").await?;
		let _: bool = scripting.call("isScriptLoaded", &(name.as_str(),)).await?;
		for (state, path) in [State::Shown, State::Hidden, State::Minimized]
			.into_iter()
			.zip(&paths)
		{
			std::fs::write(path, script(state, &target))?;
		}
		Ok::<_, Error>((bus, scripting))
	})
	.await;
	let (bus, scripting) = match connected {
		Ok(Ok(connected)) => connected,
		failure => {
			let reason = match failure {
				Ok(Err(error)) => error.to_string(),
				_ => "timed out".into(),
			};
			eprintln!("KWin scripting unavailable ({reason}); Close will minimize instead.");
			return;
		}
	};
	available.store(true, Ordering::Release);
	while changes.changed().await.is_ok() {
		let state = *changes.borrow_and_update();
		let path = &paths[state as usize];
		match tokio::time::timeout(TIMEOUT, run(&bus, &scripting, &name, path)).await {
			Ok(Ok(())) => {}
			Ok(Err(error)) => eprintln!("KWin window hiding failed: {error}"),
			Err(_) => eprintln!("KWin window hiding failed: timed out"),
		}
	}
	let _ = tokio::time::timeout(
		TIMEOUT,
		scripting.call::<_, _, bool>("unloadScript", &(name.as_str(),)),
	)
	.await;
	for path in &paths {
		let _ = std::fs::remove_file(path);
	}
}

async fn run(
	bus: &zbus::Connection,
	scripting: &zbus::Proxy<'_>,
	name: &str,
	path: &Path,
) -> Result<(), Error> {
	// KWin reads a script asynchronously after `run`, so the previous one is unloaded only now.
	let _: bool = scripting.call("unloadScript", &(name,)).await?;
	let path = path.to_str().ok_or("script path is not UTF-8")?;
	let id: i32 = scripting.call("loadScript", &(path, name)).await?;
	if id < 0 {
		return Err("KWin rejected the script".into());
	}
	// KWin 6 exports loaded scripts below /Scripting, KWin 5 at the root.
	let mut last = None;
	for object in [format!("/Scripting/Script{id}"), format!("/{id}")] {
		let script =
			zbus::Proxy::new(bus, "org.kde.KWin", object.as_str(), "org.kde.kwin.Script").await?;
		match script.call::<_, _, ()>("run", &()).await {
			Ok(()) => return Ok(()),
			Err(error) => last = Some(error),
		}
	}
	Err(last.map_or_else(|| "script not found".into(), Into::into))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn scripts_match_this_window_only() {
		let pid = script(State::Hidden, &Target::Pid(42));
		assert!(pid.contains("w.pid === 42"));
		assert!(pid.contains("skip(w, true); w.minimized = true;"));
		let app = script(State::Shown, &Target::App("cz.viceverse.serein".into()));
		assert!(app.contains("w.desktopFileName === \"cz.viceverse.serein\""));
		assert!(app.contains("workspace.activeWindow = w"));
		assert!(valid_app_id("cz.viceverse.serein"));
		assert!(!valid_app_id("../x"));
		assert!(!valid_app_id(".."));
		assert!(!valid_app_id("a\"b"));
	}
}
