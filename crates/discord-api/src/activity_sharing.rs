use crate::{DiscordApi, Failure};
use discord_protocol::activity_sharing::{self, MAX_SETTINGS_RESPONSE, Settings};
use reqwest::Method;

impl DiscordApi {
	async fn read_activity_sharing(&self) -> Result<Settings, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				"/users/@me/settings-proto/1",
				None,
				MAX_SETTINGS_RESPONSE,
			)
			.await?;
		activity_sharing::decode_response(&bytes).map_err(|_| {
			Failure::ProtocolAt("Discord activity sharing settings are unavailable or unsupported")
		})
	}

	pub async fn activity_sharing(&self) -> Result<bool, Failure> {
		self.read_activity_sharing()
			.await
			.map(|settings| settings.enabled)
	}

	/// Explicit account preference update; never automatically called for a local preview.
	pub async fn set_activity_sharing(&self, enabled: bool) -> Result<bool, Failure> {
		let current = self.read_activity_sharing().await?;
		if current.enabled == enabled {
			return Ok(enabled);
		}
		let patch =
			activity_sharing::encode_patch(&current, enabled).map_err(|_| Failure::Protocol)?;
		let bytes = self
			.request_limited(
				Method::PATCH,
				"/users/@me/settings-proto/1",
				Some(serde_json::json!({"settings":patch,"required_data_version":current.version})),
				MAX_SETTINGS_RESPONSE,
			)
			.await?;
		let saved = activity_sharing::decode_response(&bytes).map_err(|_| {
			Failure::ProtocolAt(
				"Discord activity sharing was not confirmed; refresh before retrying",
			)
		})?;
		if saved.enabled != enabled {
			return Err(Failure::ProtocolAt(
				"Discord activity sharing was not confirmed; refresh before retrying",
			));
		}
		Ok(saved.enabled)
	}

	pub async fn account_presence(&self) -> Result<model::OwnPresence, Failure> {
		let settings = self.read_activity_sharing().await?;
		activity_sharing::account_presence(&settings).map_err(|_| {
			Failure::ProtocolAt("Discord status settings are unavailable or unsupported")
		})
	}

	/// Writes the account status Discord restores on the next sign-in. Same value is not patched again.
	pub async fn set_account_presence(&self, presence: &model::OwnPresence) -> Result<(), Failure> {
		if !presence.valid() {
			return Err(Failure::Protocol);
		}
		let current = self.read_activity_sharing().await?;
		let existing =
			activity_sharing::account_presence(&current).map_err(|_| Failure::Protocol)?;
		if existing.status == presence.status
			&& existing.custom_status == presence.custom_status
			&& existing.expires_at_ms == presence.expires_at_ms
		{
			return Ok(());
		}
		let patch = activity_sharing::encode_account_presence(&current, presence)
			.map_err(|_| Failure::Protocol)?;
		let bytes = self
			.request_limited(
				Method::PATCH,
				"/users/@me/settings-proto/1",
				Some(serde_json::json!({"settings":patch,"required_data_version":current.version})),
				MAX_SETTINGS_RESPONSE,
			)
			.await?;
		let saved = activity_sharing::decode_response(&bytes).map_err(|_| {
			Failure::ProtocolAt("Discord status was not confirmed; refresh before retrying")
		})?;
		let confirmed = activity_sharing::account_presence(&saved).map_err(|_| {
			Failure::ProtocolAt("Discord status was not confirmed; refresh before retrying")
		})?;
		if confirmed.status != presence.status || confirmed.custom_status != presence.custom_status
		{
			return Err(Failure::ProtocolAt(
				"Discord status was not confirmed; refresh before retrying",
			));
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::auth::SessionSecret;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	#[tokio::test]
	async fn sharing_reads_fresh_preserves_status_and_never_retries_unconfirmed_writes() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_ACTIVITY_SETTINGS".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let disabled = r#"{"settings":"CgIYB1oLCgUKA2RuZBoCCAA="}"#;
			let enabled = r#"{"settings":"CgIYCFoLCgUKA2RuZBoCCAE="}"#;
			let server = tokio::spawn(async move {
				for (method, expected_patch, body) in [
					("GET", None, disabled),
					("GET", None, disabled),
					("PATCH", Some((7, "WgsKBQoDZG5kGgIIAQ==")), enabled),
					("GET", None, enabled),
					("GET", None, enabled),
					(
						"PATCH",
						Some((8, "WgsKBQoDZG5kGgIIAA==")),
						r#"{"settings":"CgIYCFoLCgUKA2RuZBoCCAE=","out_of_date":true}"#,
					),
					("GET", None, enabled),
					("PATCH", Some((8, "WgsKBQoDZG5kGgIIAA==")), enabled),
					("GET", None, enabled),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = Vec::new();
					let header_end = loop {
						let mut bytes = [0; 1024];
						let count = socket.read(&mut bytes).await.unwrap();
						assert!(count > 0);
						request.extend_from_slice(&bytes[..count]);
						assert!(request.len() < 4096);
						if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
							break end + 4;
						}
					};
					let headers = std::str::from_utf8(&request[..header_end]).unwrap();
					assert!(headers.starts_with(&format!(
						"{method} /users/@me/settings-proto/1 HTTP/1.1\r\n"
					)));
					let length: usize = headers
						.lines()
						.find_map(|line| {
							line.to_ascii_lowercase()
								.strip_prefix("content-length: ")
								.and_then(|v| v.parse().ok())
						})
						.unwrap_or_default();
					assert!(length < 1024);
					while request.len() < header_end + length {
						let mut bytes = [0; 1024];
						let count = socket.read(&mut bytes).await.unwrap();
						assert!(count > 0);
						request.extend_from_slice(&bytes[..count]);
					}
					if let Some((version, patch)) = expected_patch {
						let sent: serde_json::Value =
							serde_json::from_slice(&request[header_end..header_end + length])
								.unwrap();
						assert_eq!(
							sent,
							serde_json::json!({"settings":patch,"required_data_version":version})
						);
					}
					socket
						.write_all(
							format!(
								"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
								body.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
				}
			});
			assert!(!api.activity_sharing().await.unwrap());
			assert!(api.set_activity_sharing(true).await.unwrap());
			assert!(api.set_activity_sharing(true).await.unwrap());
			assert!(api.set_activity_sharing(false).await.is_err());
			assert!(api.set_activity_sharing(false).await.is_err());
			assert!(api.activity_sharing().await.unwrap());
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
