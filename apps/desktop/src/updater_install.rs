use super::Staged;
use std::{
	collections::HashSet,
	fs,
	io::{Read, Seek, SeekFrom, Write},
	path::{Component, Path, PathBuf},
	process::{Child, Command, Stdio},
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering},
	},
};
const MAX_FILES: usize = 8192;
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;
const WINDOWS_FILES: &[&str] = &[
	"serein.exe",
	"README.md",
	"LICENSE-MIT",
	"LICENSE-APACHE",
	"THIRD_PARTY_NOTICES.md",
	"docs",
	"licenses",
	"source",
	"install-notifications.ps1",
	"setup.ps1",
];

pub(super) fn flatpak_session() -> bool {
	static FLATPAK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*FLATPAK.get_or_init(|| {
		cfg!(target_os = "linux")
			&& (Path::new("/.flatpak-info").is_file() || std::env::var_os("FLATPAK_ID").is_some())
	})
}

pub(super) fn appimage_session() -> bool {
	static APPIMAGE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*APPIMAGE.get_or_init(|| {
		cfg!(all(target_os = "linux", target_arch = "x86_64"))
			&& std::env::var_os("APPIMAGE").is_some_and(|p| Path::new(&p).is_absolute())
			&& std::env::var_os("APPDIR").is_some_and(|p| Path::new(&p).is_absolute())
	})
}

fn appimage_header(header: &[u8]) -> bool {
	header.len() >= 20
		&& header[..7] == *b"\x7fELF\x02\x01\x01"
		&& header[8..11] == *b"AI\x02"
		&& matches!(u16::from_le_bytes([header[16], header[17]]), 2 | 3)
		&& header[18..20] == [62, 0]
}

fn verify_appimage(path: &Path) -> Result<(), String> {
	let mut header = [0; 20];
	let mut file = fs::File::open(path).map_err(|_| "Cannot read the AppImage.")?;
	let metadata = file
		.metadata()
		.map_err(|_| "Cannot inspect the AppImage.")?;
	if !metadata.is_file()
		|| metadata.len() > super::MAX_DOWNLOAD
		|| file.read_exact(&mut header).is_err()
		|| !appimage_header(&header)
	{
		return Err("The update is not a supported x86-64 Type 2 AppImage.".into());
	}
	Ok(())
}

fn installation() -> Result<PathBuf, String> {
	let exe = std::env::current_exe()
		.and_then(fs::canonicalize)
		.map_err(|_| "Cannot locate the installed application.".to_owned())?;
	if cfg!(target_os = "linux") {
		if !appimage_session() {
			if flatpak_session() {
				return Err(
					"Flatpak updates are managed through its repository or `flatpak update`."
						.into(),
				);
			}
			return Err(
				"Use your package manager to update Serein, or run a release AppImage.".into(),
			);
		}
		let appdir = PathBuf::from(std::env::var_os("APPDIR").ok_or("Missing AppImage mount.")?);
		if fs::canonicalize(appdir.join("usr/bin/serein"))
			.ok()
			.as_ref() != Some(&exe)
		{
			return Err("Run Serein from its AppImage to install updates.".into());
		}
		let image = PathBuf::from(std::env::var_os("APPIMAGE").ok_or("Missing AppImage path.")?);
		let image = fs::canonicalize(image).map_err(|_| "Cannot locate the installed AppImage.")?;
		verify_appimage(&image)?;
		return Ok(image);
	}
	// canonicalize returns extended Windows paths; PowerShell 5.1 expects ordinary drive/UNC paths.
	#[cfg(windows)]
	let exe = {
		let path = exe
			.to_str()
			.ok_or("The installation path cannot be represented by the update helper.")?;
		if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
			PathBuf::from(format!(r"\\{path}"))
		} else if let Some(path) = path.strip_prefix(r"\\?\") {
			PathBuf::from(path)
		} else {
			exe
		}
	};
	if cfg!(target_os = "macos") {
		let app = exe
			.parent()
			.and_then(Path::parent)
			.and_then(Path::parent)
			.ok_or("Run the installed Serein.app to install updates.")?;
		if exe.file_name().is_none_or(|name| name != "serein")
			|| exe
				.parent()
				.and_then(Path::file_name)
				.is_none_or(|name| name != "MacOS")
			|| app.extension().is_none_or(|extension| extension != "app")
		{
			return Err("Run the installed Serein.app to install updates; source builds cannot replace themselves.".into());
		}
		if app.starts_with("/Volumes") || app.to_string_lossy().contains("/AppTranslocation/") {
			return Err(
				"Move Serein to Applications or another writable folder before updating.".into(),
			);
		}
		Ok(app.to_owned())
	} else if cfg!(windows) {
		let root = exe
			.parent()
			.ok_or("Cannot locate the installed application folder.")?;
		if exe.file_name().is_none_or(|name| name != "serein.exe")
			|| !root.join("THIRD_PARTY_NOTICES.md").is_file()
			|| !root.join("licenses").is_dir()
		{
			return Err("Run Serein from an extracted release package to install updates; source builds cannot replace themselves.".into());
		}
		Ok(root.to_owned())
	} else {
		Err("Use your package manager to update Serein on Linux.".into())
	}
}

pub(super) fn create_stage() -> Result<Staged, String> {
	let installation = installation()?;
	let parent = if cfg!(any(target_os = "macos", target_os = "linux")) {
		installation
			.parent()
			.ok_or("Cannot locate the application folder.")?
	} else {
		installation.as_path()
	};
	let lock_path = parent.join(".serein-update.lock");
	if fs::symlink_metadata(&lock_path).is_ok_and(|metadata| !metadata.is_file()) {
		return Err("Unexpected update lock file.".into());
	}
	let lock = fs::OpenOptions::new()
		.read(true)
		.write(true)
		.create(true)
		.truncate(false)
		.open(&lock_path)
		.map_err(|_| "The installation folder is not writable.".to_owned())?;
	lock.try_lock()
		.map_err(|_| "Another Serein instance is preparing update storage.".to_owned())?;
	// One bounded staging directory per installation; discard leftovers only after their owner exits.
	let mut count = 0;
	for entry in
		fs::read_dir(parent).map_err(|_| "Cannot read the installation folder.".to_owned())?
	{
		count += 1;
		if count > 16_384 {
			return Err("The installation folder contains too many entries. Move Serein into its own folder.".into());
		}
		let entry = entry.map_err(|_| "Cannot inspect update storage.".to_owned())?;
		let name = entry.file_name();
		let name = name.to_string_lossy();
		let Some(pid) = name
			.strip_prefix(".serein-update-")
			.and_then(|pid| pid.parse::<u32>().ok())
			.filter(|pid| *pid > 0)
		else {
			continue;
		};
		if !entry
			.file_type()
			.map_err(|_| "Cannot inspect update storage.".to_owned())?
			.is_dir()
		{
			return Err("An unexpected file is occupying update storage.".into());
		}
		let helper_alive = fs::read_to_string(entry.path().join("helper-ready"))
			.ok()
			.filter(|s| s.len() <= 16)
			.and_then(|s| s.trim().parse::<u32>().ok())
			.is_some_and(process_alive);
		if helper_alive || (pid != std::process::id() && process_alive(pid)) {
			return Err(
				"Another Serein instance is preparing an update. Close it and try again.".into(),
			);
		}
		if fs::read(entry.path().join("owner")).ok().as_deref() != Some(b"serein-updater-v1") {
			return Err(
				"An unrecognized directory occupies update storage; move it before trying again."
					.into(),
			);
		}
		if entry.path().join("previous.app").exists()
			|| entry.path().join("previous").exists()
			|| entry.path().join("failed.app").exists()
			|| entry.path().join("previous.AppImage").exists()
		{
			return Err(format!(
				"An interrupted update needs recovery before continuing. Its backup is in {}.",
				entry.path().display()
			));
		}
		cleanup(&entry.path());
		if entry.path().exists() {
			return Err(
				"Cannot remove an unfinished update. Check installation folder permissions.".into(),
			);
		}
	}
	let directory = parent.join(format!(".serein-update-{}", std::process::id()));
	let builder = fs::DirBuilder::new();
	#[cfg(unix)]
	let mut builder = builder;
	#[cfg(unix)]
	{
		use std::os::unix::fs::DirBuilderExt;
		builder.mode(0o700);
	}
	builder.create(&directory).map_err(|_| "The installation folder is not writable. Move Serein to a writable folder and try again.".to_owned())?;
	fs::write(directory.join("owner"), b"serein-updater-v1")
		.map_err(|_| "Cannot mark update storage ownership.".to_owned())?;
	Ok(Staged {
		directory,
		installation,
	})
}
fn process_alive(pid: u32) -> bool {
	#[cfg(unix)]
	{
		Command::new("/bin/kill")
			.args(["-0", &pid.to_string()])
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()
			.is_ok_and(|status| status.success())
	}
	#[cfg(windows)]
	{
		powershell()
			.args([
				"-NoProfile",
				"-NonInteractive",
				"-Command",
				&format!(
					"if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
				),
			])
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()
			.is_ok_and(|status| status.success())
	}
	#[cfg(not(any(unix, windows)))]
	{
		let _ = pid;
		true
	}
}
pub(super) fn cleanup(directory: &Path) {
	let _ = fs::remove_dir_all(directory);
}

fn safe_path(name: &str) -> Result<PathBuf, String> {
	if name.is_empty()
		|| name.len() > 1024
		|| name.contains('\\')
		|| name.chars().any(|c| c.is_control())
	{
		return Err("The update archive contains an invalid path.".into());
	}
	let path = Path::new(name);
	for part in path.components() {
		let Component::Normal(part) = part else {
			return Err("The update archive contains a path outside its package.".into());
		};
		let part = part
			.to_str()
			.ok_or("The update archive contains a non-Unicode path.")?;
		let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
		if part.ends_with([' ', '.'])
			|| part.chars().any(|c| ":<>\"|?*".contains(c))
			|| matches!(
				stem.as_str(),
				"CON"
					| "CONIN$" | "CONOUT$"
					| "PRN" | "AUX" | "NUL"
					| "COM¹" | "COM²"
					| "COM³" | "LPT¹"
					| "LPT²" | "LPT³"
					| "COM1" | "COM2"
					| "COM3" | "COM4"
					| "COM5" | "COM6"
					| "COM7" | "COM8"
					| "COM9" | "LPT1"
					| "LPT2" | "LPT3"
					| "LPT4" | "LPT5"
					| "LPT6" | "LPT7"
					| "LPT8" | "LPT9"
			) {
			return Err("The update archive contains an unsafe filename.".into());
		}
	}
	Ok(path.to_owned())
}

// Bound the ZIP directory before ZipArchive allocates its entry table. Release archives fit ZIP32.
fn preflight_zip(file: &mut (impl Read + Seek)) -> Result<u64, String> {
	let invalid = || "The update ZIP directory is invalid or exceeds its limits.".to_owned();
	let length = file.seek(SeekFrom::End(0)).map_err(|_| invalid())?;
	let tail_len = length.min(65_557) as usize;
	if tail_len < 22 {
		return Err(invalid());
	}
	file.seek(SeekFrom::End(-(tail_len as i64)))
		.map_err(|_| invalid())?;
	let mut tail = vec![0; tail_len];
	file.read_exact(&mut tail).map_err(|_| invalid())?;
	let offset = (0..=tail_len - 22)
		.rev()
		.find(|&offset| {
			tail[offset..].starts_with(b"PK\x05\x06")
				&& offset + 22 + u16::from_le_bytes([tail[offset + 20], tail[offset + 21]]) as usize
					== tail_len
		})
		.ok_or_else(invalid)?;
	let end = &tail[offset..];
	let u16_at = |i| u16::from_le_bytes([end[i], end[i + 1]]);
	let u32_at = |i| u32::from_le_bytes([end[i], end[i + 1], end[i + 2], end[i + 3]]);
	let count = u16_at(10) as usize;
	let size = u32_at(12) as u64;
	let start = u32_at(16) as u64;
	if u16_at(4) != 0
		|| u16_at(6) != 0
		|| u16_at(8) as usize != count
		|| count == 0
		|| count > MAX_FILES
		|| size > 4 * 1024 * 1024
		|| start + size != length - tail_len as u64 + offset as u64
	{
		return Err(invalid());
	}
	file.seek(SeekFrom::Start(start)).map_err(|_| invalid())?;
	let mut directory = vec![0; size as usize];
	file.read_exact(&mut directory).map_err(|_| invalid())?;
	if directory
		.windows(4)
		.chain(end[22..].windows(4))
		.any(|bytes| bytes == b"PK\x05\x06")
	{
		return Err(invalid());
	}
	let mut position = 0_usize;
	for _ in 0..count {
		let header = directory.get(position..position + 46).ok_or_else(invalid)?;
		if !header.starts_with(b"PK\x01\x02") {
			return Err(invalid());
		}
		let name = u16::from_le_bytes([header[28], header[29]]) as usize;
		let extra = u16::from_le_bytes([header[30], header[31]]) as usize;
		let comment = u16::from_le_bytes([header[32], header[33]]) as usize;
		if name > 1024
			|| header[34..36] != [0, 0]
			|| header[20..24] == [255; 4]
			|| header[24..28] == [255; 4]
			|| header[42..46] == [255; 4]
		{
			return Err(invalid());
		}
		let extras = directory
			.get(position + 46 + name..position + 46 + name + extra)
			.ok_or_else(invalid)?;
		let mut remaining = extras;
		while !remaining.is_empty() {
			if remaining.len() < 4 {
				return Err(invalid());
			}
			if remaining[..2] == [1, 0] {
				return Err(invalid());
			}
			let bytes = u16::from_le_bytes([remaining[2], remaining[3]]) as usize;
			remaining = remaining.get(4 + bytes..).ok_or_else(invalid)?;
		}
		position = position
			.checked_add(46 + name + extra + comment)
			.ok_or_else(invalid)?;
		if position > directory.len() {
			return Err(invalid());
		}
	}
	if position != directory.len() {
		return Err(invalid());
	}
	file.seek(SeekFrom::Start(0)).map_err(|_| invalid())?;
	Ok(start)
}

// ZipArchive retries older footer signatures on malformed metadata. Hide payload bytes while
// it discovers metadata so a forged footer inside compressed data cannot bypass preflight.
struct MetadataReader<R> {
	file: R,
	directory_start: u64,
	reading_metadata: Arc<AtomicBool>,
}
impl<R: Read + Seek> Read for MetadataReader<R> {
	fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
		if self.reading_metadata.load(Ordering::Relaxed) {
			let position = self.file.stream_position()?;
			if position < self.directory_start {
				let count = buffer.len().min((self.directory_start - position) as usize);
				buffer[..count].fill(0);
				self.file.seek(SeekFrom::Current(count as i64))?;
				return Ok(count);
			}
		}
		self.file.read(buffer)
	}
}
impl<R: Seek> Seek for MetadataReader<R> {
	fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
		self.file.seek(position)
	}
}

pub(super) fn unpack(
	directory: &Path,
	installed: &Path,
	cancel: &AtomicBool,
) -> Result<(), String> {
	if cfg!(target_os = "linux") {
		if cancel.load(Ordering::Relaxed) {
			return Err("Update cancelled.".into());
		}
		let download = directory.join("package.zip");
		verify_appimage(&download)?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			fs::set_permissions(&download, fs::Permissions::from_mode(0o755))
				.map_err(|_| "Cannot make the AppImage executable.")?;
		}
		fs::rename(download, directory.join("package.AppImage"))
			.map_err(|_| "Cannot stage the AppImage.")?;
		return Ok(());
	}
	let mut archive = fs::File::open(directory.join("package.zip"))
		.map_err(|_| "Cannot read the downloaded update.".to_owned())?;
	let directory_start = preflight_zip(&mut archive)?;
	let reading_metadata = Arc::new(AtomicBool::new(true));
	let guarded = MetadataReader {
		file: archive,
		directory_start,
		reading_metadata: Arc::clone(&reading_metadata),
	};
	let mut archive = zip::ZipArchive::with_config(
		zip::read::Config {
			archive_offset: zip::read::ArchiveOffset::Known(0),
		},
		guarded,
	)
	.map_err(|_| "The update is not a valid ZIP package.".to_owned())?;
	reading_metadata.store(false, Ordering::Relaxed);
	if archive.len() > MAX_FILES {
		return Err("The update archive contains too many files.".into());
	}
	let destination = directory.join("package");
	fs::create_dir(&destination).map_err(|_| "Cannot create update staging storage.".to_owned())?;
	let mut paths = HashSet::new();
	let mut total = 0_u64;
	for index in 0..archive.len() {
		if cancel.load(Ordering::Relaxed) {
			return Err("Update cancelled.".into());
		}
		let mut entry = archive
			.by_index(index)
			.map_err(|_| "Cannot read an update archive entry.".to_owned())?;
		let path = safe_path(entry.name())?;
		if entry.encrypted()
			|| entry.is_symlink()
			|| entry
				.unix_mode()
				.is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000))
		{
			return Err(
				"Encrypted files, links and special files are not allowed in updates.".into(),
			);
		}
		if !paths.insert(path.to_string_lossy().to_lowercase()) {
			return Err("The update archive contains duplicate filenames.".into());
		}
		total = total
			.checked_add(entry.size())
			.ok_or("Update size overflow.")?;
		if total > MAX_UNPACKED {
			return Err("The update archive exceeds its extracted size limit.".into());
		}
		// ditto resource-fork metadata is not executable bundle content.
		if path.starts_with("__MACOSX") {
			continue;
		}
		let top = path
			.components()
			.next()
			.and_then(|p| p.as_os_str().to_str())
			.ok_or("Invalid package path.")?;
		if cfg!(target_os = "macos") {
			if top != "Serein.app" && !WINDOWS_FILES.contains(&top) {
				return Err("The update contains unexpected package content.".into());
			}
		} else if !WINDOWS_FILES.contains(&top) {
			return Err("The update contains unexpected package content.".into());
		}
		let target = destination.join(&path);
		if entry.is_dir() {
			fs::create_dir_all(&target)
				.map_err(|_| "Cannot create update directories.".to_owned())?;
			continue;
		}
		if let Some(parent) = target.parent() {
			fs::create_dir_all(parent)
				.map_err(|_| "Cannot create update directories.".to_owned())?;
		}
		let expected = entry.size();
		let mode = entry.unix_mode();
		let mut file = fs::OpenOptions::new()
			.write(true)
			.create_new(true)
			.open(&target)
			.map_err(|_| "Cannot create an update file.".to_owned())?;
		let mut copied = 0_u64;
		let mut buffer = [0_u8; 64 * 1024];
		loop {
			if cancel.load(Ordering::Relaxed) {
				return Err("Update cancelled.".into());
			}
			let read = entry
				.read(&mut buffer)
				.map_err(|_| "The update archive is corrupt.".to_owned())?;
			if read == 0 {
				break;
			}
			copied = copied
				.checked_add(read as u64)
				.ok_or("Update size overflow.")?;
			if copied > expected {
				return Err("An update file exceeds its declared size.".into());
			}
			file.write_all(&buffer[..read])
				.map_err(|_| "Cannot extract the update. Check available disk space.".to_owned())?;
		}
		if copied != expected {
			return Err("An update file is incomplete.".into());
		}
		file.sync_all()
			.map_err(|_| "Cannot save the extracted update.".to_owned())?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt;
			fs::set_permissions(
				&target,
				fs::Permissions::from_mode(if mode.is_some_and(|m| m & 0o111 != 0) {
					0o755
				} else {
					0o644
				}),
			)
			.map_err(|_| "Cannot set update file permissions.".to_owned())?;
		}
		#[cfg(not(unix))]
		{
			let _ = mode;
		}
	}
	if cfg!(target_os = "macos") {
		if !destination
			.join("Serein.app/Contents/MacOS/serein")
			.is_file()
		{
			return Err("The update does not contain Serein.app.".into());
		}
		verify_mac(&destination.join("Serein.app"), installed)?;
	} else if !destination.join("serein.exe").is_file()
		|| !destination.join("licenses").is_dir()
		|| !destination.join("THIRD_PARTY_NOTICES.md").is_file()
	{
		return Err("The update is missing its executable or bundled notices.".into());
	}
	fs::remove_file(directory.join("package.zip"))
		.map_err(|_| "Cannot clean the verified update archive.".to_owned())?;
	Ok(())
}
fn verify_mac(candidate: &Path, installed: &Path) -> Result<(), String> {
	if !cfg!(target_os = "macos") {
		return Ok(());
	}
	let status = Command::new("/usr/bin/codesign")
		.args(["--verify", "--deep", "--strict"])
		.arg(candidate)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status()
		.map_err(|_| "Cannot verify the update's code signature.".to_owned())?;
	if !status.success() {
		return Err("The update's macOS signature is invalid.".into());
	}
	fn identity(app: &Path) -> Result<(String, String), String> {
		let output = Command::new("/usr/bin/codesign")
			.args(["-d", "--verbose=4"])
			.arg(app)
			.output()
			.map_err(|_| "Cannot inspect the application signing identity.".to_owned())?;
		if !output.status.success() || output.stderr.len() > 64 * 1024 {
			return Err("Cannot inspect the application signing identity.".into());
		}
		let text = String::from_utf8_lossy(&output.stderr);
		let team = text
			.lines()
			.find_map(|line| line.strip_prefix("TeamIdentifier="))
			.filter(|team| team.len() == 10 && team.bytes().all(|b| b.is_ascii_alphanumeric()))
			.ok_or("In-app installation requires a Developer ID signed release of Serein.")?;
		let identifier = text
			.lines()
			.find_map(|line| line.strip_prefix("Identifier="))
			.ok_or("The update has no bundle signing identifier.")?;
		Ok((team.into(), identifier.into()))
	}
	let old = identity(installed)?;
	let new = identity(candidate)?;
	if old != new || new.1 != "org.serein.desktop" {
		return Err("The update was not signed by this Serein publisher.".into());
	}
	let status = Command::new("/usr/sbin/spctl")
		.args(["--assess", "--type", "execute"])
		.arg(candidate)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.status()
		.map_err(|_| "Cannot assess the update with macOS Gatekeeper.".to_owned())?;
	if !status.success() {
		return Err("macOS Gatekeeper did not accept the update.".into());
	}
	Ok(())
}

pub(super) struct Prepared {
	pub(super) marker: PathBuf,
	child: Child,
}
impl Prepared {
	pub(super) fn stop(mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}
pub(super) fn prepare_restart(
	directory: &Path,
	installation: &Path,
	version: Option<&str>,
) -> Result<Prepared, String> {
	let mut nonce = [0_u8; 8];
	getrandom::fill(&mut nonce)
		.map_err(|_| "Cannot create a unique restart handoff.".to_owned())?;
	let marker = directory.join(format!("commit-{:016x}", u64::from_ne_bytes(nonce)));
	let ready = directory.join("helper-ready");
	match fs::remove_file(&ready) {
		Ok(()) => {}
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
		Err(_) => return Err("Cannot reset the update handoff.".into()),
	}
	#[cfg(target_os = "macos")]
	let _ = version;
	#[cfg(target_os = "macos")]
	let mut child = {
		verify_mac(&directory.join("package/Serein.app"), installation)?;
		let script = directory.join("install.sh");
		fs::write(&script, MAC_HELPER)
			.map_err(|_| "Cannot prepare the update helper.".to_owned())?;
		Command::new("/bin/sh")
			.arg(&script)
			.arg(directory)
			.arg(installation)
			.arg(std::process::id().to_string())
			.arg(&marker)
			.current_dir("/")
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|_| "Cannot start the update helper.".to_owned())?
	};
	#[cfg(target_os = "linux")]
	let mut child = {
		let _ = version;
		if self::installation()?.as_path() != installation {
			return Err("The installed AppImage moved; restart before updating.".into());
		}
		verify_appimage(&directory.join("package.AppImage"))?;
		let script = directory.join("install.sh");
		fs::write(&script, LINUX_HELPER).map_err(|_| "Cannot prepare the update helper.")?;
		Command::new("/bin/sh")
			.env_remove("LD_LIBRARY_PATH")
			.env_remove("LD_PRELOAD")
			.arg(&script)
			.arg(directory)
			.arg(installation)
			.arg(std::process::id().to_string())
			.arg(&marker)
			.current_dir("/")
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|_| "Cannot start the update helper.")?
	};
	#[cfg(windows)]
	let mut child = {
		let script = directory.join("install.ps1");
		fs::write(&script, WINDOWS_HELPER)
			.map_err(|_| "Cannot prepare the update helper.".to_owned())?;
		let plan = serde_json::json!({
			"installation": installation,
			"parent": std::process::id(),
			"files": WINDOWS_FILES,
			"marker": marker,
			"version": version,
		});
		fs::write(
			directory.join("plan.json"),
			serde_json::to_vec(&plan).map_err(|_| "Cannot encode the update plan.".to_owned())?,
		)
		.map_err(|_| "Cannot save the update plan.".to_owned())?;
		powershell()
			.args([
				"-NoProfile",
				"-NonInteractive",
				"-ExecutionPolicy",
				"Bypass",
				"-File",
			])
			.arg(&script)
			.current_dir(std::env::temp_dir())
			.stdin(Stdio::null())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|_| "Cannot start the update helper.".to_owned())?
	};
	#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
	{
		let _ = (installation, marker, version);
		Err("In-app installation is unsupported on this platform.".into())
	}
	#[cfg(any(target_os = "macos", target_os = "linux", windows))]
	{
		for _ in 0..200 {
			if ready.is_file() {
				return Ok(Prepared { marker, child });
			}
			std::thread::sleep(std::time::Duration::from_millis(10));
		}
		let _ = child.kill();
		let _ = child.wait();
		Err("The update helper did not start. Serein will remain open.".into())
	}
}

#[cfg(windows)]
fn powershell() -> Command {
	use std::os::windows::process::CommandExt;
	let root = std::env::var_os("SystemRoot")
		.map(PathBuf::from)
		.unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
	let mut command = Command::new(root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
	command.creation_flags(0x0800_0000);
	command
}

#[cfg(target_os = "macos")]
const MAC_HELPER: &str = r#"#!/bin/sh
set -eu
stage=$1
installed=$2
parent=$3
commit=$4
printf '%s\n' "$$" > "$stage/helper-ready"
count=0
while [ ! -f "$commit" ]; do
  if ! /bin/kill -0 "$parent" 2>/dev/null; then
    [ -f "$commit" ] && break
    exit 0
  fi
  /bin/sleep 1
done
count=0
while /bin/kill -0 "$parent" 2>/dev/null; do
  count=$((count + 1)); [ "$count" -lt 120 ] || exit 0
  /bin/sleep 1
done
backup="$stage/previous.app"
if ! /bin/mv "$installed" "$backup"; then
  /usr/bin/open "$installed" || true
  exit 1
fi
if ! /bin/mv "$stage/package/Serein.app" "$installed"; then
  /bin/mv "$backup" "$installed"
  /usr/bin/open "$installed" || true
  exit 1
fi
if ! /usr/bin/open "$installed"; then
  /bin/mv "$installed" "$stage/failed.app"
  /bin/mv "$backup" "$installed"
  /usr/bin/open "$installed" || true
  exit 1
fi
/bin/rm -rf "$stage"
"#;

#[cfg(target_os = "linux")]
const LINUX_HELPER: &str = include_str!("updater_linux.sh");

#[cfg(windows)]
const WINDOWS_HELPER: &str = r#"$ErrorActionPreference = 'Stop'
$stage = $PSScriptRoot
$plan = Get-Content -LiteralPath (Join-Path $stage 'plan.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$installation = [string]$plan.installation
[IO.File]::WriteAllText((Join-Path $stage 'helper-ready'), [string]$PID)
while (!(Test-Path -LiteralPath $plan.marker)) {
  if (!(Get-Process -Id $plan.parent -ErrorAction SilentlyContinue)) {
    if (Test-Path -LiteralPath $plan.marker) { break }
    exit
  }
  Start-Sleep -Milliseconds 250
}
$process = Get-Process -Id $plan.parent -ErrorAction SilentlyContinue
if ($process -and !$process.WaitForExit(120000)) { exit }
$backup = Join-Path $stage 'previous'
[IO.Directory]::CreateDirectory($backup) | Out-Null
$moved = [Collections.Generic.List[string]]::new()
$replaced = [Collections.Generic.List[string]]::new()
try {
  foreach ($name in $plan.files) {
    $source = Join-Path (Join-Path $stage 'package') $name
    if (!(Test-Path -LiteralPath $source)) { continue }
    $target = Join-Path $installation $name
    if (Test-Path -LiteralPath $target) {
      Move-Item -LiteralPath $target -Destination (Join-Path $backup $name)
      $moved.Add($name)
    }
    Move-Item -LiteralPath $source -Destination $target
    $replaced.Add($name)
  }
  $uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Serein'
  if ($plan.version -and (Test-Path -LiteralPath $uninstallKey)) {
    Set-ItemProperty -LiteralPath $uninstallKey -Name 'DisplayVersion' -Value ([string]$plan.version) -ErrorAction SilentlyContinue
  }
  Start-Process -FilePath (Join-Path $installation 'serein.exe') -WorkingDirectory $installation
} catch {
  foreach ($name in $replaced) {
    $target = Join-Path $installation $name
    if (Test-Path -LiteralPath $target) { Move-Item -LiteralPath $target -Destination (Join-Path (Join-Path $stage 'package') $name) -ErrorAction SilentlyContinue }
  }
  foreach ($name in $moved) {
    Move-Item -LiteralPath (Join-Path $backup $name) -Destination (Join-Path $installation $name) -ErrorAction SilentlyContinue
  }
  Start-Process -FilePath (Join-Path $installation 'serein.exe') -WorkingDirectory $installation -ErrorAction SilentlyContinue
  exit 1
}
Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
"#;

#[cfg(feature = "demo")]
pub(super) fn debug_check() -> Result<(), String> {
	let mut header = [0; 20];
	header[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
	header[8..11].copy_from_slice(b"AI\x02");
	header[16] = 2;
	header[18] = 62;
	if !appimage_header(&header) || appimage_header(&header[..19]) {
		return Err("AppImage header validation failed.".into());
	}
	header[18] = 183;
	if appimage_header(&header) {
		return Err("Wrong AppImage architecture accepted.".into());
	}

	for path in [
		"../escape",
		"/absolute",
		"C:/absolute",
		"a/../../b",
		"a\\b",
		"a/CON.txt",
		"a/file:stream",
		"a/file. ",
	] {
		if safe_path(path).is_ok() {
			return Err(format!("Archive path validation accepted {path}"));
		}
	}
	if safe_path("Serein.app/Contents/MacOS/serein").is_err() {
		return Err("Valid archive path rejected.".into());
	}
	let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
	writer
		.start_file(
			"Serein.app/Contents/MacOS/serein",
			zip::write::SimpleFileOptions::default(),
		)
		.map_err(|_| "Cannot create synthetic ZIP.")?;
	writer
		.write_all(b"synthetic")
		.map_err(|_| "Cannot create synthetic ZIP.")?;
	let mut archive = writer
		.finish()
		.map_err(|_| "Cannot finish synthetic ZIP.")?;
	let directory_start = preflight_zip(&mut archive)?;
	let reading_metadata = Arc::new(AtomicBool::new(true));
	let mut parsed = zip::ZipArchive::with_config(
		zip::read::Config {
			archive_offset: zip::read::ArchiveOffset::Known(0),
		},
		MetadataReader {
			file: archive.clone(),
			directory_start,
			reading_metadata: Arc::clone(&reading_metadata),
		},
	)
	.map_err(|_| "Guarded ZIP metadata failed.")?;
	reading_metadata.store(false, Ordering::Relaxed);
	let mut decoded = Vec::new();
	parsed
		.by_index(0)
		.map_err(|_| "Synthetic ZIP entry missing.")?
		.take(32)
		.read_to_end(&mut decoded)
		.map_err(|_| "Synthetic ZIP decoding failed.")?;
	if decoded != b"synthetic" {
		return Err("Guarded ZIP payload decoding failed.".into());
	}
	let mut bytes = archive.into_inner();
	let length = bytes.len();
	bytes[length - 12] = 0xff;
	bytes[length - 11] = 0xff;
	if preflight_zip(&mut std::io::Cursor::new(bytes)).is_ok() {
		return Err("Oversized ZIP directory accepted.".into());
	}
	Ok(())
}
