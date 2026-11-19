//! Host OS details the desktop client reports in its super-properties, read once.
//!
//! Observed (unofficial) in Discord's desktop core and a recorded 0.0.423 session:
//! `os_version` is Electron's `process.getSystemVersion()` (the product version on macOS,
//! `10.0.<build>` on Windows, the kernel release on Linux), `os_sdk_version` is the first
//! part of the Darwin release on macOS and the build number on Windows, and Linux adds its
//! desktop, distribution, packaging and session type. A value that cannot be read is empty.
use std::sync::LazyLock;

pub(super) struct Os {
	pub version: String,
	pub sdk: Option<String>,
	/// Windows update build revision (UBR).
	pub build_revision: Option<String>,
	pub linux: Option<Linux>,
}

pub(super) struct Linux {
	pub window_manager: String,
	pub distro: Option<String>,
	pub runtime_environment: &'static str,
	pub display_server: String,
}

pub(super) static OS: LazyLock<Os> = LazyLock::new(read);

/// A version string made only of the characters version numbers use.
fn version(value: &str) -> Option<String> {
	let value = value.trim();
	((1..=64).contains(&value.len())
		&& value
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+' | b'_')))
	.then(|| value.to_owned())
}

/// Free text such as a distribution name, kept to printable characters and a bounded length.
#[cfg(any(test, not(any(target_os = "macos", target_os = "windows"))))]
fn label(value: &str) -> Option<String> {
	let value = value.trim();
	(!value.is_empty() && !value.chars().any(char::is_control))
		.then(|| value.chars().take(128).collect())
}

/// Output of a short system tool; nothing when it is missing, fails or says too much.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn tool(command: &mut std::process::Command) -> Option<String> {
	let output = command.stdin(std::process::Stdio::null()).output().ok()?;
	(output.status.success() && output.stdout.len() <= 512)
		.then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "macos")]
fn read() -> Os {
	// The plist names its value right after the key; no parser needed for one string.
	let product = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist")
		.ok()
		.and_then(|plist| {
			let after = plist.split("<key>ProductVersion</key>").nth(1)?;
			let start = after.find("<string>")? + "<string>".len();
			let end = after[start..].find("</string>")?;
			version(&after[start..start + end])
		});
	let darwin =
		tool(std::process::Command::new("/usr/sbin/sysctl").args(["-n", "kern.osrelease"]))
			.and_then(|release| version(&release));
	Os {
		version: product.unwrap_or_default(),
		sdk: darwin.and_then(|release| release.split('.').next().map(str::to_owned)),
		build_revision: None,
		linux: None,
	}
}

#[cfg(target_os = "windows")]
fn read() -> Os {
	use std::os::windows::process::CommandExt;
	/// No console window flashes up for the one `ver` call.
	const CREATE_NO_WINDOW: u32 = 0x0800_0000;
	// "Microsoft Windows [Version 10.0.26100.4652]"
	let full = tool(
		std::process::Command::new("cmd")
			.args(["/d", "/c", "ver"])
			.creation_flags(CREATE_NO_WINDOW),
	)
	.and_then(|text| {
		let start = text.find("Version ")? + "Version ".len();
		let end = text[start..].find(']')?;
		version(&text[start..start + end])
	})
	.unwrap_or_default();
	let parts: Vec<&str> = full.split('.').collect();
	Os {
		version: parts
			.get(..3)
			.map(|parts| parts.join("."))
			.unwrap_or_default(),
		sdk: parts.get(2).map(|build| (*build).to_owned()),
		build_revision: parts.get(3).map(|revision| (*revision).to_owned()),
		linux: None,
	}
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn read() -> Os {
	let env = |name: &str| std::env::var(name).ok().and_then(|value| label(&value));
	let has = |name: &str| std::env::var_os(name).is_some();
	let distro = std::fs::read_to_string("/etc/os-release")
		.ok()
		.and_then(|text| {
			let line = text
				.lines()
				.find_map(|line| line.strip_prefix("PRETTY_NAME="))?;
			label(line.trim_matches('"'))
		});
	Os {
		version: std::fs::read_to_string("/proc/sys/kernel/osrelease")
			.ok()
			.and_then(|release| version(&release))
			.unwrap_or_default(),
		sdk: None,
		build_revision: None,
		linux: Some(Linux {
			window_manager: format!(
				"{},{}",
				env("XDG_CURRENT_DESKTOP").unwrap_or_else(|| "unknown".into()),
				env("GDMSESSION").unwrap_or_else(|| "unknown".into())
			),
			distro,
			runtime_environment: if has("FLATPAK_ID") {
				"flatpak"
			} else if has("SNAP") || has("SNAP_NAME") {
				"snap"
			} else if has("APPIMAGE") || has("APPDIR") {
				"appimage"
			} else {
				"native"
			},
			display_server: env("XDG_SESSION_TYPE").unwrap_or_else(|| "unknown".into()),
		}),
	}
}

/// Electron's spelling of the processor architecture.
pub(super) fn arch() -> &'static str {
	match std::env::consts::ARCH {
		"aarch64" => "arm64",
		"x86_64" => "x64",
		"x86" => "ia32",
		other => other,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn reported_strings_keep_only_safe_characters() {
		assert_eq!(version(" 27.2.1\n").as_deref(), Some("27.2.1"));
		assert_eq!(
			version("10.0.26100.4652").as_deref(),
			Some("10.0.26100.4652")
		);
		for bad in ["", "1\"2", "a b", "x\\y", &"9".repeat(65)] {
			assert_eq!(version(bad), None, "{bad:?}");
		}
		assert_eq!(
			label("Ubuntu 24.04 LTS").as_deref(),
			Some("Ubuntu 24.04 LTS")
		);
		assert_eq!(label("bad\u{7}name"), None);
		assert_eq!(label(&"x".repeat(300)).map(|value| value.len()), Some(128));
	}
}
