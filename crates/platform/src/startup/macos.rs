//! Per-user launchd entry, read/written only by the existing startup worker.
use super::Settings;
use std::{
	fs::{self, File, OpenOptions},
	io::{Read, Write},
	os::unix::fs::OpenOptionsExt,
	path::{Path, PathBuf},
};

const LABEL: &str = "cz.viceverse.serein.startup";
const MAX_BYTES: u64 = 16 * 1024;
const READ_ERROR: &str = "Could not read the macOS login setting.";
const WRITE_ERROR: &str = "Could not update the macOS login setting.";
const PATH_ERROR: &str = "This executable path cannot be registered for macOS login.";
const INVALID_ENTRY: &str =
	"The Serein login entry does not match this application. Turn startup on to replace it.";

fn entry_path() -> Result<PathBuf, &'static str> {
	let home = dirs::home_dir()
		.filter(|path| path.is_absolute())
		.ok_or(PATH_ERROR)?;
	Ok(home
		.join("Library/LaunchAgents")
		.join(format!("{LABEL}.plist")))
}

fn entry(executable: &Path, minimized: bool) -> Result<String, &'static str> {
	let path = executable.to_str().ok_or(PATH_ERROR)?;
	if !executable.is_absolute() || path.len() > 4096 || path.chars().any(char::is_control) {
		return Err(PATH_ERROR);
	}
	let path = path
		.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;");
	let minimized = if minimized {
		"<string>--start-minimized</string>"
	} else {
		""
	};
	Ok(format!(
		r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{path}</string><string>--autostart</string>{minimized}</array>
<key>RunAtLoad</key><true/>
<key>LimitLoadToSessionType</key><string>Aqua</string>
</dict></plist>
"#
	))
}

pub fn load() -> Result<Settings, &'static str> {
	let file = match File::open(entry_path()?) {
		Ok(file) => file,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
			return Ok(Settings::default());
		}
		Err(_) => return Err(READ_ERROR),
	};
	let mut data = String::new();
	file.take(MAX_BYTES + 1)
		.read_to_string(&mut data)
		.map_err(|_| READ_ERROR)?;
	if data.len() as u64 > MAX_BYTES {
		return Err(INVALID_ENTRY);
	}
	let executable = std::env::current_exe().map_err(|_| PATH_ERROR)?;
	for minimized in [false, true] {
		if data == entry(&executable, minimized)? {
			return Ok(Settings {
				enabled: true,
				minimized,
			});
		}
	}
	Err(INVALID_ENTRY)
}

pub fn save(settings: Settings) -> Result<(), &'static str> {
	let path = entry_path()?;
	if !settings.enabled {
		return match fs::remove_file(path) {
			Ok(()) => Ok(()),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(_) => Err(WRITE_ERROR),
		};
	}
	let data = entry(
		&std::env::current_exe().map_err(|_| PATH_ERROR)?,
		settings.minimized,
	)?;
	if data.len() as u64 > MAX_BYTES {
		return Err(PATH_ERROR);
	}
	fs::create_dir_all(path.parent().ok_or(PATH_ERROR)?).map_err(|_| WRITE_ERROR)?;
	let mut nonce = [0_u8; 8];
	getrandom::fill(&mut nonce).map_err(|_| WRITE_ERROR)?;
	let temporary = path.with_extension(format!("{:016x}.tmp", u64::from_le_bytes(nonce)));
	let mut file = OpenOptions::new()
		.write(true)
		.create_new(true)
		.mode(0o600)
		.open(&temporary)
		.map_err(|_| WRITE_ERROR)?;
	let result = file
		.write_all(data.as_bytes())
		.and_then(|()| file.sync_all())
		.and_then(|()| fs::rename(&temporary, &path));
	if result.is_err() {
		let _ = fs::remove_file(&temporary);
	}
	// launchd reads the entry on the next login. Do not bootstrap it now: doing so
	// would launch a second client, and KeepAlive would relaunch after Quit.
	result.map_err(|_| WRITE_ERROR)
}
