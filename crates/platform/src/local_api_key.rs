//! The local API's access key lives only in the OS credential store, never in preferences.
use crate::{ACCOUNT, CredentialError, entry, forget_entry};
use zeroize::Zeroizing;

/// 32 random bytes as lowercase hex.
pub const KEY_CHARS: usize = 64;

pub fn valid(key: &str) -> bool {
	key.len() == KEY_CHARS
		&& key
			.bytes()
			.all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
fn name() -> String {
	format!("{ACCOUNT}.local-api")
}
/// The saved key. An entry that is not a key reads as absent, so the caller replaces it.
pub fn load() -> Result<Option<Zeroizing<String>>, CredentialError> {
	match entry(&name())?.get_password() {
		Ok(value) => Ok(Some(Zeroizing::new(value)).filter(|key| valid(key))),
		Err(keyring_core::Error::NoEntry) => Ok(None),
		Err(_) => Err(CredentialError::Unavailable),
	}
}
pub fn save(key: &str) -> Result<(), CredentialError> {
	if !valid(key) {
		return Err(CredentialError::Invalid);
	}
	entry(&name())?
		.set_password(key)
		.map_err(|_| CredentialError::Unavailable)
}
pub fn forget() -> Result<(), CredentialError> {
	forget_entry(&name())
}
