//! One explicit read-only Safety Hub request for the signed-in account. The request carries
//! only the normal session token; response bodies are never logged.
use crate::{DiscordApi, Failure};
use discord_protocol::safety_hub;
use model::safety_hub::Snapshot;
use wreq::Method;

impl DiscordApi {
	pub(super) async fn safety_hub(&self) -> Result<Snapshot, Failure> {
		let bytes = self
			.request_once(Method::GET, "/safety-hub/@me", safety_hub::MAX_WIRE)
			.await?;
		let snapshot = safety_hub::decode_hub(&bytes).map_err(|_| Failure::Protocol)?;
		if snapshot.valid() {
			Ok(snapshot)
		} else {
			Err(Failure::Capacity)
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::SessionSecret;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	#[tokio::test]
	async fn safety_hub_http_is_a_normal_get_with_bounded_parse() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_SAFETY_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let body = serde_json::json!({
				"account_standing": {"state": 200},
				"classifications": [{
					"id": "1411751868690075770",
					"description": "Synthetic limit",
					"flagged_content": [{"content": "must not be retained"}]
				}]
			})
			.to_string();
			let server = tokio::spawn(async move {
				let (mut stream, _) = listener.accept().await.unwrap();
				let mut bytes = Vec::new();
				let header_end = loop {
					let mut chunk = [0; 4096];
					let n = stream.read(&mut chunk).await.unwrap();
					assert!(n > 0);
					bytes.extend_from_slice(&chunk[..n]);
					assert!(bytes.len() <= 16384);
					if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
						break end + 4;
					}
				};
				let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
				assert!(headers.starts_with("GET /safety-hub/@me HTTP/1.1\r\n"));
				assert!(headers.lines().any(|line| {
					line.split_once(':')
						.is_some_and(|(name, _)| name.eq_ignore_ascii_case("authorization"))
				}));
				stream
					.write_all(
						format!(
							"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
							body.len()
						)
						.as_bytes(),
					)
					.await
					.unwrap();
			});
			let snapshot = api.safety_hub().await.unwrap();
			assert_eq!(
				snapshot.standing,
				Some(model::safety_hub::AccountStanding::Limited)
			);
			assert_eq!(snapshot.classifications.len(), 1);
			assert!(!format!("{snapshot:?}").contains("must not be retained"));
			server.await.unwrap();
		})
		.await
		.unwrap();
	}

	#[tokio::test]
	async fn a_short_rate_limit_is_reported_without_a_second_request() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_SAFETY_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				let (mut stream, _) = listener.accept().await.unwrap();
				let mut bytes = Vec::new();
				while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
					let mut chunk = [0; 4096];
					let n = stream.read(&mut chunk).await.unwrap();
					assert!(n > 0, "the client stopped after one request");
					bytes.extend_from_slice(&chunk[..n]);
				}
				let body = r#"{"retry_after":0.1,"global":false,"message":"slow down"}"#;
				stream
					.write_all(
						format!(
							"HTTP/1.1 429 Too Many Requests\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
							body.len()
						)
						.as_bytes(),
					)
					.await
					.unwrap();
				// A retry would arrive here; the test then fails on the second read above.
				let mut extra = [0; 1];
				assert_eq!(stream.read(&mut extra).await.unwrap(), 0);
			});
			// One click reaches the service once, even though the wait is short enough to resend.
			assert_eq!(api.safety_hub().await, Err(Failure::RateLimited));
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
