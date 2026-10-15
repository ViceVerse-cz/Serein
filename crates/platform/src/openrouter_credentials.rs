//! Account-scoped OpenRouter settings and key; never exposed to Wasm or local files.
use crate::{ACCOUNT, CredentialError, entry, forget_entry};
use client_core::auto_reply::Config;
use model::Id;
use zeroize::Zeroizing;

pub struct Connection {
	pub config: Config,
	pub key: Zeroizing<String>,
}
impl std::fmt::Debug for Connection {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("OpenRouterConnection([REDACTED])")
	}
}
fn name(account: Id) -> Result<String, CredentialError> {
	if account.0 == 0 {
		return Err(CredentialError::Invalid);
	}
	Ok(format!("{ACCOUNT}.openrouter.{}", account.0))
}
pub fn valid_key(key: &str) -> bool {
	!key.is_empty() && key.len() <= 256 && key.bytes().all(|c| c.is_ascii_graphic())
}
fn encode(value: &Connection) -> Result<Zeroizing<String>, CredentialError> {
	value
		.config
		.validate()
		.map_err(|_| CredentialError::Invalid)?;
	if !valid_key(&value.key) {
		return Err(CredentialError::Invalid);
	}
	let raw = Zeroizing::new(
		serde_json::to_string(&(&value.config, value.key.as_str()))
			.map_err(|_| CredentialError::Invalid)?,
	);
	// Fits Windows' credential blob bound as well as other supported stores.
	if raw.len() > 2400 {
		return Err(CredentialError::Invalid);
	}
	Ok(raw)
}
pub fn load(account: Id) -> Result<Option<Connection>, CredentialError> {
	let raw = match entry(&name(account)?)?.get_password() {
		Ok(value) => Zeroizing::new(value),
		Err(keyring_core::Error::NoEntry) => return Ok(None),
		Err(_) => return Err(CredentialError::Unavailable),
	};
	if raw.len() > 2400 {
		return Err(CredentialError::Invalid);
	}
	let (config, key): (Config, String) =
		serde_json::from_str(&raw).map_err(|_| CredentialError::Invalid)?;
	let value = Connection {
		config,
		key: Zeroizing::new(key),
	};
	encode(&value)?;
	Ok(Some(value))
}
pub fn save(account: Id, value: &Connection) -> Result<(), CredentialError> {
	entry(&name(account)?)?
		.set_password(&encode(value)?)
		.map_err(|_| CredentialError::Unavailable)
}
pub fn forget(account: Id) -> Result<(), CredentialError> {
	forget_entry(&name(account)?)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn bounded_record_and_redacted_debug() {
		let mut value = Connection {
			config: Config {
				model: "provider/model".into(),
				instructions: String::new(),
				channels: vec![Id(20)],
			},
			key: Zeroizing::new("synthetic-key".into()),
		};
		assert!(encode(&value).is_ok());
		assert!(!format!("{value:?}").contains("synthetic-key"));
		value.key.push('\n');
		assert!(encode(&value).is_err());
		assert!(name(Id(0)).is_err());
		assert_ne!(name(Id(1)).unwrap(), name(Id(2)).unwrap());
	}
}
