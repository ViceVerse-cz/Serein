//! Read-only enumeration of this user's running executable paths for local game detection.
//! Never inspects another user's processes, memory, arguments, environment or open files.

/// Bounds both the syscall/parse work and the memory a hostile process table can force.
pub const MAX_PROCESSES: usize = 4096;
const MAX_PATH: usize = 512;

#[cfg(any(target_os = "macos", target_os = "windows", test))]
const MAX_OUTPUT: usize = MAX_PROCESSES * (MAX_PATH + 128);
#[cfg(any(target_os = "macos", target_os = "windows", test))]
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(unsafe_code)]
fn effective_uid() -> u32 {
	unsafe extern "C" {
		fn geteuid() -> u32;
	}
	// SAFETY: POSIX geteuid takes no arguments and returns the current effective user ID.
	unsafe { geteuid() }
}

/// Reads stdout incrementally. Failure kills and reaps the helper without a retained reader thread.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows", test))]
pub(crate) fn command_output(
	command: std::process::Command,
	max_bytes: usize,
	deadline: std::time::Duration,
) -> std::io::Result<Vec<u8>> {
	use std::io;
	use std::process::Stdio;
	use tokio::io::AsyncReadExt;
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()?;
	runtime.block_on(async move {
		let mut command = tokio::process::Command::from(command);
		let mut child = command
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::null())
			.kill_on_drop(true)
			.spawn()?;
		let mut stdout = child
			.stdout
			.take()
			.ok_or_else(|| io::Error::other("process list is unavailable"))?;
		let result = tokio::time::timeout(deadline, async {
			let mut output = Vec::with_capacity(max_bytes.min(4096));
			let mut buffer = [0_u8; 4096];
			loop {
				let count = stdout.read(&mut buffer).await?;
				if count == 0 {
					break;
				}
				if output.len().saturating_add(count) > max_bytes {
					return Err(io::Error::other("process list is too large"));
				}
				output.extend_from_slice(&buffer[..count]);
			}
			if !child.wait().await?.success() {
				return Err(io::Error::other("process list is unavailable"));
			}
			Ok(output)
		})
		.await;
		match result {
			Ok(Ok(output)) => Ok(output),
			failure => {
				let error = match failure {
					Ok(Err(error)) => error,
					Err(_) => io::Error::new(io::ErrorKind::TimedOut, "process list timed out"),
					Ok(Ok(_)) => unreachable!(),
				};
				let _ = child.start_kill();
				let _ =
					tokio::time::timeout(std::time::Duration::from_millis(500), child.wait()).await;
				Err(error)
			}
		}
	})
}

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

#[cfg(any(target_os = "linux", test))]
fn accept_cmdline(reader: impl std::io::Read, into: &mut Vec<String>) -> std::io::Result<()> {
	use std::io::Read;

	// One extra byte distinguishes an overlong argv[0] from an exactly bounded path.
	let mut bytes = Vec::with_capacity(MAX_PATH + 1);
	reader.take((MAX_PATH + 1) as u64).read_to_end(&mut bytes)?;
	if let Some(first) = bytes.split(|byte| *byte == 0).next()
		&& first.len() <= MAX_PATH
		&& let Ok(first) = std::str::from_utf8(first)
	{
		accept(first, into);
	}
	Ok(())
}

/// `"image.exe","1234","Console","1","12,345 K"`: the image name, unless it runs in the
/// non-interactive services session. The session name is localized; its number is not.
#[cfg(any(target_os = "windows", test))]
fn interactive_image(line: &str) -> Option<&str> {
	let mut fields = line.strip_prefix('"')?.split("\",\"");
	let name = fields.next()?;
	let session = fields.nth(2)?;
	(session.trim() != "0").then_some(name)
}

#[cfg(target_os = "linux")]
mod native {
	use super::{MAX_PROCESSES, accept, accept_cmdline, effective_uid};
	use std::fs;
	use std::io::Read;
	use std::os::fd::AsRawFd;
	use std::path::{Path, PathBuf};
	const MAX_STATUS: usize = 4096;

	/// `/proc/<pid>/exe` is the real image; `cmdline`'s first word covers interpreted
	/// launches (and Wine/Proton, where the Windows executable only appears there).
	pub fn running() -> std::io::Result<Vec<String>> {
		let owner = effective_uid();
		let entries = fs::read_dir("/proc")?
			.take(MAX_PROCESSES)
			.filter_map(Result::ok)
			.map(|entry| entry.path());
		Ok(scan(entries, owner))
	}

	pub(super) fn scan(entries: impl IntoIterator<Item = PathBuf>, owner: u32) -> Vec<String> {
		let mut paths = Vec::new();
		for directory in entries.into_iter().take(MAX_PROCESSES) {
			if paths.len() >= MAX_PROCESSES {
				break;
			}
			let Some(name) = directory.file_name().and_then(|name| name.to_str()) else {
				continue;
			};
			if name.parse::<u32>().is_err() {
				continue;
			}
			// Directory ownership changes for nondumpable processes. Read the actual effective
			// UID instead, keeping the directory alive so PID reuse cannot switch the image.
			let Ok(handle) = fs::File::open(&directory) else {
				continue;
			};
			let directory = PathBuf::from(format!("/proc/self/fd/{}", handle.as_raw_fd()));
			if owner_of(&directory) != Some(owner) {
				continue;
			}
			append(&directory, &mut paths);
		}
		paths
	}

	fn owner_of(directory: &Path) -> Option<u32> {
		let file = fs::File::open(directory.join("status")).ok()?;
		let mut status = Vec::with_capacity(1024);
		file.take(MAX_STATUS as u64).read_to_end(&mut status).ok()?;
		status_owner(&status)
	}

	pub(super) fn status_owner(status: &[u8]) -> Option<u32> {
		let line = status
			.split(|byte| *byte == b'\n')
			.find(|line| line.starts_with(b"Uid:"))?;
		std::str::from_utf8(&line[4..])
			.ok()?
			.split_ascii_whitespace()
			.nth(1)?
			.parse()
			.ok()
	}

	fn append(directory: &Path, paths: &mut Vec<String>) {
		if let Ok(target) = fs::read_link(directory.join("exe"))
			&& let Some(target) = target.to_str()
		{
			accept(target, paths);
		}
		// Bounded read: a command line may be megabytes, but only argv[0] is used.
		if let Ok(cmdline) = fs::File::open(directory.join("cmdline")) {
			let _ = accept_cmdline(cmdline, paths);
		}
	}
}

#[cfg(target_os = "macos")]
mod native {
	use super::{
		COMMAND_TIMEOUT, MAX_OUTPUT, MAX_PROCESSES, accept, command_output, effective_uid,
	};
	use std::process::Command;

	/// `ps` reports executable paths owned by this effective user, without
	/// arguments and without the private-API entitlements a direct sysctl walk needs.
	pub fn running() -> std::io::Result<Vec<String>> {
		let mut command = Command::new("/bin/ps");
		// macOS's legacy BSD -u is a formatting flag; POSIX mode makes it a UID filter.
		command.env("COMMAND_MODE", "unix2003").args([
			"-u",
			&effective_uid().to_string(),
			"-o",
			"comm=",
		]);
		let output = command_output(command, MAX_OUTPUT, COMMAND_TIMEOUT)?;
		let mut paths = Vec::new();
		for line in String::from_utf8_lossy(&output).lines().take(MAX_PROCESSES) {
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
	use super::{
		COMMAND_TIMEOUT, MAX_OUTPUT, MAX_PROCESSES, accept, command_output, interactive_image,
	};
	use std::os::windows::process::CommandExt;
	use std::process::Command;

	const CREATE_NO_WINDOW: u32 = 0x0800_0000;

	/// `tasklist` lists this account's images without opening another process' handle. Paths are
	/// unavailable this way, which is fine: detectable entries are image names on Windows.
	/// Session 0 holds only services, never a game the user is playing; Intel's `LMS.exe`
	/// service there otherwise matches "Last Man Standing".
	pub fn running() -> std::io::Result<Vec<String>> {
		let account = current_account()?;
		let filter = format!("USERNAME eq {account}");
		let mut command = Command::new("tasklist.exe");
		command
			.args(["/nh", "/fo", "csv", "/fi", &filter])
			.creation_flags(CREATE_NO_WINDOW);
		let output = command_output(command, MAX_OUTPUT, COMMAND_TIMEOUT)?;
		let mut paths = Vec::new();
		for line in String::from_utf8_lossy(&output).lines().take(MAX_PROCESSES) {
			if paths.len() >= MAX_PROCESSES {
				break;
			}
			if let Some(name) = interactive_image(line) {
				accept(name, &mut paths);
			}
		}
		Ok(paths)
	}

	#[allow(unsafe_code)]
	fn current_account() -> std::io::Result<String> {
		use std::io;
		use windows::{
			Win32::{
				Foundation::{CloseHandle, HANDLE},
				Security::{
					GetTokenInformation, LookupAccountSidW, SID_NAME_USE, TOKEN_QUERY, TOKEN_USER,
					TokenUser,
				},
				System::Threading::{GetCurrentProcess, OpenProcessToken},
			},
			core::{PCWSTR, PWSTR},
		};
		let mut token = HANDLE::default();
		// SAFETY: output is a valid slot; the current process token is queried read-only.
		unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }
			.map_err(io::Error::other)?;
		let mut storage = [0_usize; 128];
		let mut length = 0;
		// SAFETY: writable pointer-aligned storage bounds TOKEN_USER and its maximum-size SID.
		let result = unsafe {
			GetTokenInformation(
				token,
				TokenUser,
				Some(storage.as_mut_ptr().cast()),
				std::mem::size_of_val(&storage) as u32,
				&mut length,
			)
		};
		// SAFETY: this function owns the OpenProcessToken result, including on query failure.
		let _ = unsafe { CloseHandle(token) };
		result.map_err(io::Error::other)?;
		// SAFETY: the successful query initialized TOKEN_USER and its in-buffer SID.
		let user = unsafe { &*storage.as_ptr().cast::<TOKEN_USER>() };
		let mut name = [0_u16; 256];
		let mut domain = [0_u16; 256];
		let mut name_length = name.len() as u32;
		let mut domain_length = domain.len() as u32;
		let mut use_kind = SID_NAME_USE::default();
		// SAFETY: SID remains valid; both UTF-16 output buffers have their actual lengths.
		unsafe {
			LookupAccountSidW(
				PCWSTR::null(),
				user.User.Sid,
				Some(PWSTR(name.as_mut_ptr())),
				&mut name_length,
				Some(PWSTR(domain.as_mut_ptr())),
				&mut domain_length,
				&mut use_kind,
			)
		}
		.map_err(io::Error::other)?;
		let name = String::from_utf16(
			name.get(..name_length as usize)
				.ok_or_else(|| io::Error::other("account name is too large"))?,
		)
		.map_err(io::Error::other)?;
		let domain = String::from_utf16(
			domain
				.get(..domain_length as usize)
				.ok_or_else(|| io::Error::other("account domain is too large"))?,
		)
		.map_err(io::Error::other)?;
		if name.is_empty()
			|| domain.is_empty()
			|| name
				.chars()
				.chain(domain.chars())
				.any(|c| c.is_control() || c == '"')
		{
			return Err(io::Error::other("current account is unavailable"));
		}
		Ok(format!("{domain}\\{name}"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[cfg(target_os = "linux")]
	#[test]
	fn helper_output_and_runtime_are_bounded_and_children_are_reaped() {
		use std::{
			process::Command,
			time::{Duration, Instant},
		};
		for (index, (payload, expected_kind)) in [
			("x".repeat(4096), std::io::ErrorKind::Other),
			(String::new(), std::io::ErrorKind::TimedOut),
		]
		.into_iter()
		.enumerate()
		{
			let path = std::env::temp_dir().join(format!(
				"serein-process-child-{}-{index}",
				std::process::id()
			));
			let mut command = Command::new("/bin/sh");
			command
				.args([
					"-c",
					"printf '%s' \"$$\" > \"$1\"; printf '%s' \"$2\"; exec /bin/sleep 20",
					"synthetic-process-test",
				])
				.arg(&path)
				.arg(payload);
			let began = Instant::now();
			let result = command_output(command, 32, Duration::from_millis(200));
			let elapsed = began.elapsed();
			let child_pid: u32 = std::fs::read_to_string(&path).unwrap().parse().unwrap();
			std::fs::remove_file(path).unwrap();
			assert_eq!(result.unwrap_err().kind(), expected_kind);
			assert!(
				elapsed < Duration::from_secs(2),
				"a pending or oversized helper must retire promptly"
			);
			assert!(
				!std::path::Path::new("/proc")
					.join(child_pid.to_string())
					.exists(),
				"the synthetic helper must be killed and reaped"
			);
		}
		let mut command = Command::new("/bin/sh");
		command.args(["-c", "printf '/games/example\\n'"]);
		assert_eq!(
			command_output(command, MAX_OUTPUT, COMMAND_TIMEOUT).unwrap(),
			b"/games/example\n"
		);
		let mut command = Command::new("/bin/sh");
		command.args(["-c", "exit 3"]);
		assert!(command_output(command, MAX_OUTPUT, COMMAND_TIMEOUT).is_err());
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn process_scan_excludes_other_owners_before_reading_images() {
		use std::os::unix::fs::MetadataExt;
		let root =
			std::env::temp_dir().join(format!("serein-process-owner-{}", std::process::id()));
		let directory = root.join("123");
		std::fs::create_dir_all(&directory).unwrap();
		std::fs::write(
			directory.join("cmdline"),
			b"/games/example\0private arguments",
		)
		.unwrap();
		let owner = std::fs::metadata(&directory).unwrap().uid();
		std::fs::write(
			directory.join("status"),
			format!("Name:\texample\nUid:\t{owner}\t{owner}\t{owner}\t{owner}\n"),
		)
		.unwrap();
		let other = owner.checked_add(1).unwrap();
		let current = native::scan([directory.clone()], owner);
		let foreign = native::scan([directory], other);
		std::fs::remove_dir_all(root).unwrap();
		assert_eq!(current, ["/games/example"]);
		assert!(
			foreign.is_empty(),
			"foreign-owner argv[0] must never enter detection"
		);
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn process_uid_uses_effective_status_and_rejects_unbounded_or_invalid_status() {
		assert_eq!(
			native::status_owner(b"Name:\tgame\nUid:\t1000\t2000\t3000\t4000\n"),
			Some(2000)
		);
		for invalid in [
			b"Uid:\t1000\n".as_slice(),
			b"Uid:\t1000\twrong\n",
			b"Name:\tgame\n",
			b"Uid:\t0\t4294967296\t0\t0\n",
		] {
			assert_eq!(native::status_owner(invalid), None);
		}
		let root =
			std::env::temp_dir().join(format!("serein-process-status-{}", std::process::id()));
		let directory = root.join("123");
		std::fs::create_dir_all(&directory).unwrap();
		std::fs::write(directory.join("cmdline"), b"/games/example\0").unwrap();
		// Effective UID filtering must work even when the proc directory's owner differs.
		std::fs::write(directory.join("status"), b"Uid:\t1000\t2000\t1000\t1000\n").unwrap();
		assert_eq!(native::scan([directory.clone()], 2000), ["/games/example"]);
		assert!(native::scan([directory.clone()], 1000).is_empty());
		let oversized = format!(
			"Name:\t{}\nUid:\t1000\t2000\t1000\t1000\n",
			"x".repeat(8192)
		);
		std::fs::write(directory.join("status"), oversized).unwrap();
		let result = native::scan([directory], 2000);
		std::fs::remove_dir_all(root).unwrap();
		assert!(
			result.is_empty(),
			"UID fields beyond the status byte cap must not be accepted"
		);
	}

	#[cfg(target_os = "linux")]
	#[test]
	fn process_scan_bounds_attempts_even_when_no_paths_are_accepted() {
		use std::os::unix::fs::MetadataExt;
		let root =
			std::env::temp_dir().join(format!("serein-process-limit-{}", std::process::id()));
		let directory = root.join((MAX_PROCESSES + 1).to_string());
		std::fs::create_dir_all(&directory).unwrap();
		std::fs::write(directory.join("cmdline"), b"/games/example\0").unwrap();
		let owner = std::fs::metadata(&directory).unwrap().uid();
		std::fs::write(
			directory.join("status"),
			format!("Uid:\t{owner}\t{owner}\t{owner}\t{owner}\n"),
		)
		.unwrap();
		let entries = (1..=MAX_PROCESSES)
			.map(|pid| root.join(pid.to_string()))
			.chain([directory]);
		let paths = native::scan(entries, owner);
		std::fs::remove_dir_all(root).unwrap();
		assert!(
			paths.is_empty(),
			"failed PID inspections also consume the scan budget"
		);
	}

	#[test]
	fn windows_services_session_is_ignored() {
		assert_eq!(
			interactive_image(r#""LMS.exe","4321","Services","0","8,120 K""#),
			None
		);
		assert_eq!(
			interactive_image(r#""lms.exe","1234","Console","1","12,345 K""#),
			Some("lms.exe")
		);
		assert_eq!(interactive_image("INFO: No tasks are running"), None);
		assert_eq!(interactive_image(r#""short","1""#), None);
	}

	#[test]
	fn cmdline_reads_are_bounded_and_only_accept_complete_first_paths() {
		let mut command = b"/usr/bin/game\0".to_vec();
		command.extend(vec![b'x'; 1024 * 1024]);
		let mut reader = std::io::Cursor::new(command);
		let mut paths = Vec::new();
		accept_cmdline(&mut reader, &mut paths).unwrap();
		assert_eq!(reader.position(), (MAX_PATH + 1) as u64);
		assert_eq!(paths, ["/usr/bin/game"]);

		let exact = vec![b'x'; MAX_PATH];
		accept_cmdline(exact.as_slice(), &mut paths).unwrap();
		let mut terminated = exact.clone();
		terminated.push(0);
		accept_cmdline(terminated.as_slice(), &mut paths).unwrap();
		assert_eq!(paths.len(), 3);
		assert_eq!(paths[1].len(), MAX_PATH);
		assert_eq!(paths[1], paths[2]);

		let overlong = vec![b'x'; MAX_PATH + 1];
		for invalid in [overlong.as_slice(), b"\xff\0", b"\0", b""] {
			accept_cmdline(invalid, &mut paths).unwrap();
		}
		assert_eq!(paths.len(), 3);

		{
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
}
