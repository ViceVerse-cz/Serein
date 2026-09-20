//! Read-only enumeration of this user's running executable paths for local game detection.
//! Never inspects another user's processes, memory, arguments, environment or open files.

/// Bounds both the syscall/parse work and the memory a hostile process table can force.
pub const MAX_PROCESSES: usize = 4096;
const MAX_PATH: usize = 512;

/// Executable paths of the processes visible to this user, in no particular order.
/// A partial list is normal: processes exit while the table is read.
pub fn running() -> std::io::Result<Vec<String>> {
	native::running()
}

fn accept(path: &str, into: &mut Vec<String>) {
	let path = path.trim();
	if path.is_empty()
		|| path.len() > MAX_PATH
		|| path.chars().any(char::is_control)
		|| into.len() >= MAX_PROCESSES
	{
		return;
	}
	into.push(path.to_owned());
}

#[cfg(target_os = "linux")]
mod native {
	use super::{MAX_PATH, MAX_PROCESSES, accept};
	use std::fs;

	/// `/proc/<pid>/exe` is the real image; `cmdline`'s first word covers interpreted
	/// launches (and Wine/Proton, where the Windows executable only appears there).
	pub fn running() -> std::io::Result<Vec<String>> {
		let mut paths = Vec::new();
		for entry in fs::read_dir("/proc")? {
			if paths.len() >= MAX_PROCESSES {
				break;
			}
			let Ok(entry) = entry else { continue };
			let name = entry.file_name();
			let Some(name) = name.to_str() else { continue };
			if name.parse::<u32>().is_err() {
				continue;
			}
			let directory = entry.path();
			if let Ok(target) = fs::read_link(directory.join("exe"))
				&& let Some(target) = target.to_str()
			{
				accept(target, &mut paths);
			}
			// Bounded read: a command line may be megabytes, but only argv[0] is used.
			if let Ok(cmdline) = fs::read(directory.join("cmdline"))
				&& let Some(first) = cmdline.split(|byte| *byte == 0).next()
				&& first.len() <= MAX_PATH
				&& let Ok(first) = std::str::from_utf8(first)
			{
				accept(first, &mut paths);
			}
		}
		Ok(paths)
	}
}

#[cfg(target_os = "macos")]
mod native {
	use super::{MAX_PROCESSES, accept};
	use std::process::Command;

	/// `ps` reports the executable path of every process this user may see, without
	/// arguments and without the private-API entitlements a direct sysctl walk needs.
	pub fn running() -> std::io::Result<Vec<String>> {
		let output = Command::new("/bin/ps").args(["-Axo", "comm="]).output()?;
		if !output.status.success() {
			return Err(std::io::Error::other("process list is unavailable"));
		}
		let mut paths = Vec::new();
		for line in String::from_utf8_lossy(&output.stdout).lines() {
			if paths.len() >= MAX_PROCESSES {
				break;
			}
			accept(line, &mut paths);
		}
		Ok(paths)
	}
}

#[cfg(target_os = "windows")]
mod native {
	use super::{MAX_PROCESSES, accept};
	use std::os::windows::process::CommandExt;
	use std::process::Command;

	const CREATE_NO_WINDOW: u32 = 0x0800_0000;

	/// `tasklist` lists image names without opening another process' handle. Paths are
	/// unavailable this way, which is fine: detectable entries are image names on Windows.
	pub fn running() -> std::io::Result<Vec<String>> {
		let output = Command::new("tasklist.exe")
			.args(["/nh", "/fo", "csv"])
			.creation_flags(CREATE_NO_WINDOW)
			.output()?;
		if !output.status.success() {
			return Err(std::io::Error::other("process list is unavailable"));
		}
		let mut paths = Vec::new();
		for line in String::from_utf8_lossy(&output.stdout).lines() {
			if paths.len() >= MAX_PROCESSES {
				break;
			}
			// `"image.exe","1234","Console","1","12,345 K"`; only the quoted image name is used.
			let Some(name) = line.strip_prefix('"').and_then(|l| l.split('"').next()) else {
				continue;
			};
			accept(name, &mut paths);
		}
		Ok(paths)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn own_process_is_listed_within_bounds() {
		let paths = running().expect("the current user's process list must be readable");
		assert!(paths.len() <= MAX_PROCESSES);
		assert!(paths.iter().all(|path| path.len() <= MAX_PATH));
		let current = std::env::current_exe().unwrap();
		let name = current
			.file_name()
			.unwrap()
			.to_string_lossy()
			.to_lowercase();
		assert!(
			paths
				.iter()
				.any(|path| path.to_lowercase().contains(name.trim_end_matches(".exe"))),
			"the test binary must appear in {paths:?}"
		);
	}

	#[test]
	fn unbounded_and_control_character_paths_are_dropped() {
		let mut paths = Vec::new();
		accept("", &mut paths);
		accept("  ", &mut paths);
		accept("/usr/bin/game\u{7}", &mut paths);
		accept(&"x".repeat(MAX_PATH + 1), &mut paths);
		assert!(paths.is_empty());
		accept("  /usr/bin/game  ", &mut paths);
		assert_eq!(paths, ["/usr/bin/game"]);
		let mut full = vec![String::new(); MAX_PROCESSES];
		accept("/usr/bin/game", &mut full);
		assert_eq!(full.len(), MAX_PROCESSES);
	}
}
