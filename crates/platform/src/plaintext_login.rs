//! Explicit opt-in saved-login fallback. Secret files are private and bounded; empty
//! authority markers prevent older keyring entries reappearing after fallback deletion.
use crate::CredentialError;
use client_core::auth::SessionSecret;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use zeroize::Zeroizing;

const MAX_BYTES: u64 = 2048;
const MAX_ENTRIES: usize = 10; // Launch, eight remembered accounts and one incoming account.
const MAX_MARKERS: usize = 64; // Bounded tombstones for native stores awaiting deletion.
const MAX_FILES: usize = MAX_ENTRIES + MAX_MARKERS + 1;
static CONFIG: Mutex<(Option<PathBuf>, bool)> = Mutex::new((None, false));

/// Keep the directory while disabled so Forget and startup cleanup can still remove secrets.
pub fn configure(directory: Option<PathBuf>, enabled: bool) {
	*CONFIG.lock().unwrap_or_else(PoisonError::into_inner) = (directory, enabled);
}
pub fn configuration() -> (Option<PathBuf>, bool) {
	CONFIG
		.lock()
		.unwrap_or_else(PoisonError::into_inner)
		.clone()
}
fn valid_name(name: &str) -> bool {
	name == crate::ACCOUNT
		|| name.strip_prefix(crate::ACCOUNT).is_some_and(|suffix| {
			suffix.strip_prefix('.').is_some_and(|id| {
				!id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
			})
		})
}
fn marker(name: &str) -> String {
	format!(".stale-{name}")
}
fn entries(directory: &Path) -> Result<Vec<std::fs::DirEntry>, CredentialError> {
	if !private_directory(directory, false)? {
		return Ok(Vec::new());
	}
	let mut files = Vec::new();
	for file in std::fs::read_dir(directory).map_err(|_| CredentialError::Unavailable)? {
		if files.len() == MAX_FILES {
			return Err(CredentialError::Unavailable);
		}
		files.push(file.map_err(|_| CredentialError::Unavailable)?);
	}
	Ok(files)
}
/// Removes secret files and interrupted writes, retaining only credential-free authority markers.
pub fn forget_all(directory: &Path) -> Result<(), CredentialError> {
	for file in entries(directory)? {
		let name = file.file_name();
		let name = name.to_str().ok_or(CredentialError::Unavailable)?;
		if valid_name(name) || name.starts_with(".pending-") || name.ends_with(".tmp") {
			std::fs::remove_file(file.path()).map_err(|_| CredentialError::Unavailable)?;
		}
	}
	sync_directory(directory)
}
/// Interrupted temporary files never participate in restore and are removed before new IO.
pub fn cleanup_temporary(directory: &Path) -> Result<(), CredentialError> {
	for file in entries(directory)? {
		let name = file.file_name();
		let name = name.to_str().ok_or(CredentialError::Unavailable)?;
		if name.starts_with(".pending-") || name.ends_with(".tmp") {
			std::fs::remove_file(file.path()).map_err(|_| CredentialError::Unavailable)?;
		}
	}
	sync_directory(directory)
}
/// The committed roster bounds account authority state. Missing roster accounts
/// cannot be restored without a new owner login, so their secret and marker can go.
pub fn retain_accounts(directory: &Path, accounts: &[model::Id]) -> Result<(), CredentialError> {
	if accounts.len() > 19 {
		return Err(CredentialError::Invalid);
	}
	for file in entries(directory)? {
		let filename = file.file_name();
		let filename = filename.to_str().ok_or(CredentialError::Unavailable)?;
		let name = filename.strip_prefix(".stale-").unwrap_or(filename);
		if let Some(id) = name
			.strip_prefix(crate::ACCOUNT)
			.and_then(|suffix| suffix.strip_prefix('.'))
			&& let Ok(id) = id.parse::<u64>()
			&& !accounts.contains(&model::Id(id))
		{
			std::fs::remove_file(file.path()).map_err(|_| CredentialError::Unavailable)?;
		}
	}
	sync_directory(directory)
}
pub(crate) fn keyring_stale(directory: &Path, name: &str) -> Result<bool, CredentialError> {
	if !private_directory(directory, false)? {
		return Ok(false);
	}
	match std::fs::symlink_metadata(directory.join(marker(name))) {
		Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
		Ok(_) => Err(CredentialError::Unavailable),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
		Err(_) => Err(CredentialError::Unavailable),
	}
}
pub(crate) fn mark_stale(directory: &Path, name: &str) -> Result<(), CredentialError> {
	private_directory(directory, true)?;
	let mut names = std::collections::BTreeSet::new();
	for file in entries(directory)? {
		let filename = file.file_name();
		let filename = filename.to_str().ok_or(CredentialError::Unavailable)?;
		let entry = filename.strip_prefix(".stale-").unwrap_or(filename);
		if valid_name(entry) {
			names.insert(entry.to_owned());
		}
	}
	if !names.contains(name) && names.len() >= MAX_MARKERS {
		return Err(CredentialError::Unavailable);
	}
	if !keyring_stale(directory, name)? {
		atomic_write(directory, &marker(name), b"")?;
	}
	Ok(())
}
pub(crate) fn load(directory: &Path, name: &str) -> Result<Option<SessionSecret>, CredentialError> {
	if !valid_name(name) {
		return Err(CredentialError::Invalid);
	}
	if !private_directory(directory, false)? {
		return Ok(None);
	}
	let path = directory.join(name);
	let metadata = match std::fs::symlink_metadata(&path) {
		Ok(metadata) => metadata,
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(_) => return Err(CredentialError::Unavailable),
	};
	// In particular, do not open a FIFO or follow a preexisting symlink.
	if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
		return Err(CredentialError::Invalid);
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::MetadataExt;
		if metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
			return Err(CredentialError::Unavailable);
		}
	}
	let file = std::fs::File::open(path).map_err(|_| CredentialError::Unavailable)?;
	let mut value = Zeroizing::new(String::new());
	file.take(MAX_BYTES + 1)
		.read_to_string(&mut value)
		.map_err(|_| CredentialError::Invalid)?;
	if value.len() as u64 > MAX_BYTES {
		return Err(CredentialError::Invalid);
	}
	SessionSecret::from_owner_input(value.trim().to_owned())
		.map(Some)
		.map_err(|_| CredentialError::Invalid)
}
pub(crate) fn save(
	directory: &Path,
	name: &str,
	secret: &SessionSecret,
) -> Result<(), CredentialError> {
	if !valid_name(name) || secret.expose().len() as u64 > MAX_BYTES {
		return Err(CredentialError::Invalid);
	}
	cleanup_temporary(directory)?;
	let secrets = entries(directory)?
		.into_iter()
		.filter(|file| file.file_name().to_str().is_some_and(valid_name))
		.count();
	if !directory.join(name).exists() && secrets >= MAX_ENTRIES {
		return Err(CredentialError::Unavailable);
	}
	// Commit authority before the secret: an interrupted write must not restore an older account.
	mark_stale(directory, name)?;
	atomic_write(directory, name, secret.expose().as_bytes())
}
fn atomic_write(directory: &Path, name: &str, bytes: &[u8]) -> Result<(), CredentialError> {
	let mut file = tempfile::Builder::new()
		.prefix(".pending-")
		.tempfile_in(directory)
		.map_err(|_| CredentialError::Unavailable)?;
	file.write_all(bytes)
		.and_then(|()| file.as_file().sync_all())
		.map_err(|_| CredentialError::Unavailable)?;
	file.persist(directory.join(name))
		.map_err(|_| CredentialError::Unavailable)?;
	sync_directory(directory)
}
/// Forget keeps the marker until the keyring confirms deletion, so unavailable OS stores
/// cannot later resurrect a login. The marker contains no token.
pub(crate) fn forget(directory: &Path, name: &str) -> Result<(), CredentialError> {
	if !valid_name(name) {
		return Err(CredentialError::Invalid);
	}
	if !private_directory(directory, false)? {
		return Ok(());
	}
	cleanup_temporary(directory)?;
	match std::fs::remove_file(directory.join(name)) {
		Ok(()) => sync_directory(directory),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(_) => Err(CredentialError::Unavailable),
	}
}
pub(crate) fn keyring_current(directory: &Path, name: &str) -> Result<(), CredentialError> {
	forget(directory, name)?;
	match std::fs::remove_file(directory.join(marker(name))) {
		Ok(()) => sync_directory(directory),
		Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
		Err(_) => Err(CredentialError::Unavailable),
	}
}
fn sync_directory(directory: &Path) -> Result<(), CredentialError> {
	#[cfg(unix)]
	{
		match std::fs::File::open(directory).and_then(|file| file.sync_all()) {
			Ok(()) => Ok(()),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
			Err(_) => Err(CredentialError::Unavailable),
		}
	}
	#[cfg(not(unix))]
	{
		let _ = directory;
		Ok(())
	}
}
fn private_directory(directory: &Path, create: bool) -> Result<bool, CredentialError> {
	if create {
		#[cfg(unix)]
		{
			use std::os::unix::fs::DirBuilderExt;
			std::fs::DirBuilder::new()
				.recursive(true)
				.mode(0o700)
				.create(directory)
				.map_err(|_| CredentialError::Unavailable)?;
		}
		#[cfg(not(unix))]
		std::fs::create_dir_all(directory).map_err(|_| CredentialError::Unavailable)?;
	}
	let metadata = match std::fs::symlink_metadata(directory) {
		Ok(metadata) => metadata,
		Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
		Err(_) => return Err(CredentialError::Unavailable),
	};
	if !metadata.is_dir() || metadata.file_type().is_symlink() {
		return Err(CredentialError::Unavailable);
	}
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt;
		if metadata.permissions().mode() & 0o077 != 0 {
			return Err(CredentialError::Unavailable);
		}
	}
	Ok(true)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn round_trips_privately_and_forgets_without_restoring_stale_keyring() {
		let root = tempfile::tempdir().unwrap();
		let directory = root.path().join("login");
		let secret = SessionSecret::from_owner_input("SYNTHETIC_SAVED_LOGIN".into()).unwrap();
		assert!(load(&directory, crate::ACCOUNT).unwrap().is_none());
		for _ in 0..2 {
			save(&directory, crate::ACCOUNT, &secret).unwrap();
		}
		assert_eq!(
			load(&directory, crate::ACCOUNT).unwrap().unwrap().expose(),
			secret.expose()
		);
		assert!(keyring_stale(&directory, crate::ACCOUNT).unwrap());
		std::fs::write(
			directory.join(crate::ACCOUNT),
			vec![b'a'; MAX_BYTES as usize + 1],
		)
		.unwrap();
		assert_eq!(
			load(&directory, crate::ACCOUNT).err(),
			Some(CredentialError::Invalid)
		);
		std::fs::write(directory.join(".pending-crash"), b"SYNTHETIC").unwrap();
		forget(&directory, crate::ACCOUNT).unwrap();
		assert!(!directory.join(".pending-crash").exists());
		assert!(load(&directory, crate::ACCOUNT).unwrap().is_none());
		assert!(keyring_stale(&directory, crate::ACCOUNT).unwrap());
		save(&directory, crate::ACCOUNT, &secret).unwrap();
		for id in 1..MAX_ENTRIES {
			save(&directory, &format!("{}.{id}", crate::ACCOUNT), &secret).unwrap();
		}
		assert_eq!(
			save(&directory, &format!("{}.99", crate::ACCOUNT), &secret),
			Err(CredentialError::Unavailable)
		);
		retain_accounts(&directory, &[model::Id(1)]).unwrap();
		assert!(!keyring_stale(&directory, &format!("{}.2", crate::ACCOUNT)).unwrap());
		// Orphan cleanup makes room for the incoming account before its roster is committed.
		save(&directory, &format!("{}.99", crate::ACCOUNT), &secret).unwrap();
		forget_all(&directory).unwrap();
		assert!(
			load(&directory, &format!("{}.1", crate::ACCOUNT))
				.unwrap()
				.is_none()
		);
		keyring_current(&directory, crate::ACCOUNT).unwrap();
		assert!(!keyring_stale(&directory, crate::ACCOUNT).unwrap());
	}
	#[cfg(unix)]
	#[test]
	fn rejects_insecure_paths_and_ignores_predictable_temporary_symlinks() {
		use std::os::unix::fs::{PermissionsExt, symlink};
		let root = tempfile::tempdir().unwrap();
		let directory = root.path().join("login");
		let secret = SessionSecret::from_owner_input("SYNTHETIC_SAVED_LOGIN".into()).unwrap();
		save(&directory, crate::ACCOUNT, &secret).unwrap();
		assert_eq!(
			std::fs::metadata(directory.join(crate::ACCOUNT))
				.unwrap()
				.permissions()
				.mode() & 0o077,
			0
		);
		let outside = root.path().join("outside");
		std::fs::write(&outside, b"unchanged").unwrap();
		symlink(&outside, directory.join(format!(".{}.tmp", crate::ACCOUNT))).unwrap();
		save(&directory, crate::ACCOUNT, &secret).unwrap();
		assert_eq!(std::fs::read(&outside).unwrap(), b"unchanged");
		forget(&directory, crate::ACCOUNT).unwrap();
		symlink(&outside, directory.join(crate::ACCOUNT)).unwrap();
		assert_eq!(
			load(&directory, crate::ACCOUNT).err(),
			Some(CredentialError::Invalid)
		);
		std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
		assert_eq!(
			save(&directory, crate::ACCOUNT, &secret),
			Err(CredentialError::Unavailable)
		);
	}
}
