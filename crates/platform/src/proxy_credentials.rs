//! Proxy passwords live only in the OS credential store, never plugin storage.
use crate::{ACCOUNT, CredentialError, entry, forget_entry};
use zeroize::Zeroizing;

pub struct Credentials {
	pub endpoint: String,
	pub username: Zeroizing<String>,
	pub password: Zeroizing<String>,
}
impl std::fmt::Debug for Credentials {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("ProxyCredentials([redacted])")
	}
}
impl Credentials {
	pub fn validate(&self) -> Result<(), CredentialError> {
		let url = url::Url::parse(&self.endpoint).map_err(|_| CredentialError::Invalid)?;
		if self.endpoint.len() > 2048
			|| !matches!(url.scheme(), "http" | "https")
			|| url.host_str().is_none()
			|| !url.username().is_empty()
			|| url.password().is_some()
			|| url.path() != "/"
			|| url.query().is_some()
			|| url.fragment().is_some()
			|| self.username.is_empty()
			|| self.username.len() > 256
			|| self.password.len() > 1024
			|| self.username.contains(':')
			|| self
				.username
				.chars()
				.chain(self.password.chars())
				.any(char::is_control)
		{
			return Err(CredentialError::Invalid);
		}
		Ok(())
	}
}
fn name() -> String {
	format!("{ACCOUNT}.api-proxy")
}
pub fn load() -> Result<Option<Credentials>, CredentialError> {
	let raw = match entry(&name())?.get_password() {
		Ok(value) => Zeroizing::new(value),
		Err(keyring_core::Error::NoEntry) => return Ok(None),
		Err(_) => return Err(CredentialError::Unavailable),
	};
	if raw.len() > 8192 {
		return Err(CredentialError::Invalid);
	}
	let (endpoint, username, password): (String, String, String) =
		serde_json::from_str(&raw).map_err(|_| CredentialError::Invalid)?;
	let credentials = Credentials {
		endpoint,
		username: Zeroizing::new(username),
		password: Zeroizing::new(password),
	};
	credentials.validate()?;
	Ok(Some(credentials))
}
pub fn save(credentials: &Credentials) -> Result<(), CredentialError> {
	credentials.validate()?;
	let raw = Zeroizing::new(
		serde_json::to_string(&(
			&credentials.endpoint,
			credentials.username.as_str(),
			credentials.password.as_str(),
		))
		.map_err(|_| CredentialError::Invalid)?,
	);
	entry(&name())?
		.set_password(&raw)
		.map_err(|_| CredentialError::Unavailable)
}
pub fn forget() -> Result<(), CredentialError> {
	forget_entry(&name())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn credentials_are_bounded_and_debug_is_redacted() {
		let mut value = Credentials {
			endpoint: "http://localhost:8080/".into(),
			username: Zeroizing::new("owner".into()),
			password: Zeroizing::new("synthetic-password".into()),
		};
		assert!(value.validate().is_ok());
		assert!(!format!("{value:?}").contains("synthetic-password"));
		value.username = Zeroizing::new("bad:name".into());
		assert_eq!(value.validate(), Err(CredentialError::Invalid));
		value.username = Zeroizing::new("owner".into());
		value.password = Zeroizing::new("x".repeat(1025));
		assert_eq!(value.validate(), Err(CredentialError::Invalid));
	}
}
