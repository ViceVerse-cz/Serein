//! One real client per local account store, independent of optional desktop IPC.
use client_core::auth::Failure;
use std::{fs::OpenOptions, path::Path};

/// The OS releases this claim on close or abnormal process termination; no stale PID file exists.
#[must_use]
pub struct SessionLock {
	_file: std::fs::File,
}

pub fn acquire() -> Result<SessionLock, Failure> {
	let root = local_store::data_dir().map_err(|_| {
		Failure::ProtocolAt(
			"The local account store is unavailable; no account session was started",
		)
	})?;
	acquire_at(&root)
}

pub fn acquire_at(root: &Path) -> Result<SessionLock, Failure> {
	if !root.is_absolute() {
		return Err(Failure::ProtocolAt(
			"The local account store must be an absolute path; no account session was started",
		));
	}
	let path = root.join("session-store.lock");
	let file = OpenOptions::new()
		.read(true)
		.write(true)
		.create(true)
		.truncate(false)
		.open(&path)
		.map_err(|_| {
			Failure::ProtocolAt(
				"The account lock file is unavailable; no account session was started",
			)
		})?;
	file.try_lock().map_err(|error| match error {
		std::fs::TryLockError::WouldBlock => Failure::CapacityAt(
			"Another Serein process is using this account store; close it before signing in again",
		),
		_ => Failure::ProtocolAt(
			"The account lock could not be acquired; no account session was started",
		),
	})?;
	Ok(SessionLock { _file: file })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn second_client_waits_for_the_first_lock_to_close() {
		let random = {
			let mut bytes = [0_u8; 8];
			getrandom::fill(&mut bytes).unwrap();
			u64::from_le_bytes(bytes)
		};
		let root = std::env::temp_dir().join(format!("serein-store-lock-{random:016x}"));
		std::fs::create_dir(&root).unwrap();
		let first = acquire_at(&root).unwrap();
		assert!(matches!(acquire_at(&root), Err(Failure::CapacityAt(_))));
		drop(first);
		let second = acquire_at(&root).unwrap();
		drop(second);
		// An unlocked file is not a claim; a crashed process cannot leave a stale one.
		assert!(root.join("session-store.lock").exists());
		std::fs::remove_dir_all(root).unwrap();
	}
}
