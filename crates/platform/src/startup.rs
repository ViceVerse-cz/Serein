//! Explicit opt-in launch at Windows or macOS sign-in. The OS entry is the only saved setting.
//! Call from a worker; demo mode must keep changes in memory instead.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
	pub enabled: bool,
	pub minimized: bool,
}

pub const fn available() -> bool {
	cfg!(any(target_os = "windows", target_os = "macos"))
}

#[cfg(target_os = "windows")]
pub use native::{load, save};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{load, save};

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn load() -> Result<Settings, &'static str> {
	Err("Automatic startup is currently available on Windows and macOS only.")
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn save(_settings: Settings) -> Result<(), &'static str> {
	Err("Automatic startup is currently available on Windows and macOS only.")
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod native {
	use super::Settings;
	use std::path::Path;
	use windows::{
		Win32::{
			Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
			System::Registry::{
				HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
				RegSetKeyValueW,
			},
		},
		core::{PCWSTR, w},
	};

	const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
	const VALUE: PCWSTR = w!("Serein");
	// Windows documents a 260-character limit for Run commands, excluding the terminator.
	const MAX_COMMAND: usize = 260;
	const READ_ERROR: &str = "Could not read the Windows startup setting.";
	const WRITE_ERROR: &str = "Could not update the Windows startup setting.";
	const PATH_ERROR: &str = "This executable path cannot be registered for Windows startup.";
	const INVALID_ENTRY: &str =
		"The Serein startup entry does not match this application. Turn startup on to replace it.";

	pub fn load() -> Result<Settings, &'static str> {
		let executable = std::env::current_exe().map_err(|_| PATH_ERROR)?;
		load_at(RUN, &executable)
	}

	pub fn save(settings: Settings) -> Result<(), &'static str> {
		let command = if settings.enabled {
			let executable = std::env::current_exe().map_err(|_| PATH_ERROR)?;
			Some(startup_command(&executable, settings.minimized)?)
		} else {
			None
		};
		save_at(RUN, command.as_deref())
	}

	fn startup_command(executable: &Path, minimized: bool) -> Result<String, &'static str> {
		let path = executable.to_str().ok_or(PATH_ERROR)?;
		if !executable.is_absolute()
			|| path.ends_with(['\\', '/'])
			|| path.chars().any(|ch| ch == '"' || ch.is_control())
			|| path.encode_utf16().count() > MAX_COMMAND
		{
			return Err(PATH_ERROR);
		}
		let suffix = if minimized { " --start-minimized" } else { "" };
		let command = format!("\"{path}\" --autostart{suffix}");
		if command.encode_utf16().count() > MAX_COMMAND {
			return Err(
				"The executable path is too long for Windows startup. Move Serein to a shorter path.",
			);
		}
		Ok(command)
	}

	fn parse_command(command: &str, executable: &Path) -> Result<Settings, &'static str> {
		for minimized in [false, true] {
			if command == startup_command(executable, minimized)? {
				return Ok(Settings {
					enabled: true,
					minimized,
				});
			}
		}
		Err(INVALID_ENTRY)
	}

	fn load_at(subkey: PCWSTR, executable: &Path) -> Result<Settings, &'static str> {
		let mut data = [0_u16; MAX_COMMAND + 1];
		let mut bytes = std::mem::size_of_val(&data) as u32;
		// SAFETY: all names are terminated and live for the call; the writable buffer's
		// byte size is exact. RegGetValueW rejects oversized values without allocating.
		let status = unsafe {
			RegGetValueW(
				HKEY_CURRENT_USER,
				subkey,
				VALUE,
				RRF_RT_REG_SZ,
				None,
				Some(data.as_mut_ptr().cast()),
				Some(&mut bytes),
			)
		};
		if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
			return Ok(Settings::default());
		}
		if status != ERROR_SUCCESS {
			return Err(READ_ERROR);
		}
		let units = bytes as usize / 2;
		if !bytes.is_multiple_of(2) || units == 0 || units > data.len() || data[units - 1] != 0 {
			return Err(INVALID_ENTRY);
		}
		let command = String::from_utf16(&data[..units - 1]).map_err(|_| INVALID_ENTRY)?;
		parse_command(&command, executable)
	}

	fn save_at(subkey: PCWSTR, command: Option<&str>) -> Result<(), &'static str> {
		let status = if let Some(command) = command {
			let data: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
			// SAFETY: names and bounded REG_SZ data are terminated and live for the call.
			// This writes only our named value, creating the subkey if needed.
			unsafe {
				RegSetKeyValueW(
					HKEY_CURRENT_USER,
					subkey,
					VALUE,
					REG_SZ.0,
					Some(data.as_ptr().cast()),
					(data.len() * 2) as u32,
				)
			}
		} else {
			// SAFETY: names are terminated and live for the call. Other values are untouched.
			unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey, VALUE) }
		};
		if status == ERROR_SUCCESS
			|| (command.is_none() && matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND))
		{
			Ok(())
		} else {
			Err(WRITE_ERROR)
		}
	}

	#[cfg(test)]
	mod tests {
		use super::*;
		use windows::Win32::System::Registry::RegDeleteKeyW;

		#[test]
		fn startup_commands_are_quoted_bounded_and_exact() {
			let executable = Path::new(r"C:\Program Files\Serein\serein.exe");
			for minimized in [false, true] {
				let command = startup_command(executable, minimized).unwrap();
				assert!(command.starts_with(r#""C:\Program Files\Serein\serein.exe" --autostart"#));
				assert_eq!(
					parse_command(&command, executable),
					Ok(Settings {
						enabled: true,
						minimized
					})
				);
				assert!(parse_command(&(command + " --other"), executable).is_err());
			}
			for path in [
				"serein.exe",
				"C:\\bad\"path.exe",
				"C:\\bad\0path.exe",
				"C:\\folder\\",
			] {
				assert!(startup_command(Path::new(path), false).is_err());
			}
			assert!(
				startup_command(Path::new(&format!("C:\\{}.exe", "x".repeat(260))), false).is_err()
			);
			let boundary = format!("C:\\{}.exe", "x".repeat(239));
			assert_eq!(
				startup_command(Path::new(&boundary), false).unwrap().len(),
				260
			);
			assert!(startup_command(Path::new(&boundary), true).is_err());
			assert!(startup_command(Path::new(&boundary.replace('x', "😀")), false).is_err());
			assert!(parse_command(r#""C:\Other\serein.exe" --autostart"#, executable).is_err());
		}

		#[test]
		fn isolated_registry_round_trip_and_removal() {
			// This key is outside every Windows startup location, including during test failures.
			struct TestKey(Vec<u16>);
			impl Drop for TestKey {
				fn drop(&mut self) {
					// SAFETY: owned terminated name identifies only this random synthetic test key.
					unsafe {
						let _ = RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(self.0.as_ptr()));
					}
				}
			}
			let mut nonce = [0_u8; 8];
			getrandom::fill(&mut nonce).unwrap();
			let name = format!(
				"Software\\SereinStartupTest-{:016x}",
				u64::from_le_bytes(nonce)
			);
			let key = TestKey(name.encode_utf16().chain(Some(0)).collect());
			let subkey = PCWSTR(key.0.as_ptr());
			let executable = Path::new(r"C:\Synthetic Folder\serein.exe");
			assert_eq!(load_at(subkey, executable), Ok(Settings::default()));
			save_at(subkey, None).unwrap();
			for minimized in [false, true, false] {
				let command = startup_command(executable, minimized).unwrap();
				save_at(subkey, Some(&command)).unwrap();
				assert_eq!(
					load_at(subkey, executable),
					Ok(Settings {
						enabled: true,
						minimized
					})
				);
			}
			save_at(subkey, Some(&"x".repeat(1024))).unwrap();
			assert!(load_at(subkey, executable).is_err());
			save_at(subkey, None).unwrap();
			assert_eq!(load_at(subkey, executable), Ok(Settings::default()));
			save_at(subkey, None).unwrap();
		}
	}
}
