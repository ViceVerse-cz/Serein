//! Background installer for Microsoft Edge WebView2 Runtime on Windows.
//!
//! Tries multiple automated installation paths without displaying console windows:
//! 1. `winget install --id Microsoft.EdgeWebView2Runtime --silent ...`
//! 2. Downloading Microsoft Evergreen Bootstrapper and executing `/silent /install`
//! 3. PowerShell WebClient download and silent install pipeline

use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallEvent {
	Progress(&'static str),
	Success,
	Failed(&'static str),
}

pub struct Webview2Installer {
	rx: Receiver<InstallEvent>,
}

impl Webview2Installer {
	pub fn start() -> Self {
		let (tx, rx) = channel();
		std::thread::Builder::new()
			.name("webview2-installer".to_owned())
			.spawn(move || {
				run_installation(tx);
			})
			.expect("spawn webview2 installer thread");
		Self { rx }
	}

	pub fn try_recv(&self) -> Result<InstallEvent, std::sync::mpsc::TryRecvError> {
		self.rx.try_recv()
	}
}

#[cfg(windows)]
fn run_installation(tx: Sender<InstallEvent>) {
	if platform::is_webview2_installed() {
		let _ = tx.send(InstallEvent::Success);
		return;
	}

	// Method 1: Winget
	let _ = tx.send(InstallEvent::Progress(
		"Attempting installation via winget…",
	));
	if try_winget() && platform::is_webview2_installed() {
		let _ = tx.send(InstallEvent::Success);
		return;
	}

	// Method 2: Microsoft Evergreen Bootstrapper
	let _ = tx.send(InstallEvent::Progress(
		"Downloading Microsoft Edge WebView2 installer…",
	));
	if try_evergreen_bootstrapper(&tx) && platform::is_webview2_installed() {
		let _ = tx.send(InstallEvent::Success);
		return;
	}

	// Method 3: PowerShell WebClient fallback
	let _ = tx.send(InstallEvent::Progress(
		"Installing Edge WebView2 Runtime via PowerShell fallback…",
	));
	if try_powershell_pipeline() && platform::is_webview2_installed() {
		let _ = tx.send(InstallEvent::Success);
		return;
	}

	// If all methods exhausted and still not installed:
	let _ = tx.send(InstallEvent::Failed(
		"Automatic installation of Edge WebView2 Runtime was unsuccessful.",
	));
}

#[cfg(not(windows))]
fn run_installation(tx: Sender<InstallEvent>) {
	let _ = tx.send(InstallEvent::Failed(
		"Edge WebView2 Runtime is only available on Windows.",
	));
}

#[cfg(windows)]
fn try_winget() -> bool {
	let program = find_winget();
	let mut cmd = silent_command(program);
	cmd.args([
		"install",
		"--id",
		"Microsoft.EdgeWebView2Runtime",
		"--silent",
		"--accept-source-agreements",
		"--accept-package-agreements",
	]);
	wait_command(cmd, std::time::Duration::from_secs(180))
}

#[cfg(windows)]
fn try_evergreen_bootstrapper(tx: &Sender<InstallEvent>) -> bool {
	const BOOTSTRAPPER_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
	let dest = std::env::temp_dir().join("MicrosoftEdgeWebview2Setup.exe");

	// Clean up any stale download first
	let _ = std::fs::remove_file(&dest);

	// Try downloading via curl.exe first
	let downloaded = download_with_curl(BOOTSTRAPPER_URL, &dest)
		|| download_with_powershell(BOOTSTRAPPER_URL, &dest);

	if !downloaded || !is_valid_executable(&dest) {
		let _ = std::fs::remove_file(&dest);
		return false;
	}

	let _ = tx.send(InstallEvent::Progress(
		"Running Microsoft Edge WebView2 installer in background…",
	));

	let mut cmd = silent_command(&dest);
	cmd.args(["/silent", "/install"]);
	let success = wait_command(cmd, std::time::Duration::from_secs(180));

	let _ = std::fs::remove_file(&dest);
	success
}

#[cfg(windows)]
fn download_with_curl(url: &str, dest: &std::path::Path) -> bool {
	let root = std::env::var_os("SystemRoot")
		.map(std::path::PathBuf::from)
		.unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
	let curl_path = root.join(r"System32\curl.exe");
	if !curl_path.is_file() {
		return false;
	}
	let mut cmd = silent_command(curl_path);
	cmd.args(["-L", "-s", "-S", "-f", "-o"]).arg(dest).arg(url);
	wait_command(cmd, std::time::Duration::from_secs(60))
}

#[cfg(windows)]
fn download_with_powershell(url: &str, dest: &std::path::Path) -> bool {
	let script = format!(
		"[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
		(New-Object Net.WebClient).DownloadFile('{url}', '{}')",
		dest.display().to_string().replace('\'', "''")
	);
	let mut cmd = powershell();
	cmd.args([
		"-NoProfile",
		"-NonInteractive",
		"-ExecutionPolicy",
		"Bypass",
		"-Command",
		&script,
	]);
	wait_command(cmd, std::time::Duration::from_secs(60))
}

#[cfg(windows)]
fn try_powershell_pipeline() -> bool {
	let script = "\
		$ErrorActionPreference = 'Stop'; \
		[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
		$tmp = Join-Path $env:TEMP 'MicrosoftEdgeWebview2Setup.exe'; \
		(New-Object Net.WebClient).DownloadFile('https://go.microsoft.com/fwlink/p/?LinkId=2124703', $tmp); \
		Start-Process -FilePath $tmp -ArgumentList '/silent /install' -Wait; \
		Remove-Item -Force $tmp";
	let mut cmd = powershell();
	cmd.args([
		"-NoProfile",
		"-NonInteractive",
		"-ExecutionPolicy",
		"Bypass",
		"-Command",
		script,
	]);
	wait_command(cmd, std::time::Duration::from_secs(180))
}

#[cfg(windows)]
fn find_winget() -> std::path::PathBuf {
	if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
		let p = std::path::PathBuf::from(local_app_data).join(r"Microsoft\WindowsApps\winget.exe");
		if p.is_file() {
			return p;
		}
	}
	std::path::PathBuf::from("winget")
}

#[cfg(windows)]
fn is_valid_executable(path: &std::path::Path) -> bool {
	path.metadata()
		.map(|m| m.is_file() && m.len() > 100_000)
		.unwrap_or(false)
}

#[cfg(windows)]
fn powershell() -> std::process::Command {
	let root = std::env::var_os("SystemRoot")
		.map(std::path::PathBuf::from)
		.unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
	silent_command(root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
}

#[cfg(windows)]
fn silent_command(program: impl AsRef<std::path::Path>) -> std::process::Command {
	use std::os::windows::process::CommandExt;
	let mut cmd = std::process::Command::new(program.as_ref());
	cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
	cmd.stdin(std::process::Stdio::null());
	cmd.stdout(std::process::Stdio::null());
	cmd.stderr(std::process::Stdio::null());
	cmd
}

#[cfg(windows)]
fn wait_command(mut cmd: std::process::Command, timeout: std::time::Duration) -> bool {
	let Ok(mut child) = cmd.spawn() else {
		return false;
	};
	let start = std::time::Instant::now();
	while start.elapsed() < timeout {
		match child.try_wait() {
			Ok(Some(status)) => return status.success(),
			Ok(None) => std::thread::sleep(std::time::Duration::from_millis(500)),
			Err(_) => return false,
		}
	}
	let _ = child.kill();
	let _ = child.wait();
	false
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn installer_event_variants_are_distinct() {
		assert_ne!(InstallEvent::Success, InstallEvent::Failed(""));
		assert_eq!(
			InstallEvent::Progress("testing"),
			InstallEvent::Progress("testing")
		);
	}
}
