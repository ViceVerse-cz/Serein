//! Credential-free, bounded public application metadata for local Rich Presence.
use model::Id;
use reqwest::Client;
use serde::Deserialize;
use std::time::Duration;
use tokio::time::Instant;

const MAX_METADATA: usize = 256 * 1024;
const MAX_ASSETS: usize = 1024;

#[derive(Deserialize)]
struct Application {
	id: Id,
	name: String,
}
#[derive(Clone, Deserialize)]
pub struct Asset {
	pub id: Id,
	pub name: String,
}

#[derive(Clone)]
pub struct Metadata {
	pub name: String,
	pub assets: Vec<Asset>,
}
impl Metadata {
	/// Only registered application assets are forwarded. Never fetch game-supplied URLs.
	pub fn asset(&self, key: &str) -> Option<String> {
		self.assets
			.iter()
			.find(|asset| asset.name == key || asset.id.to_string() == key)
			.map(|asset| asset.id.to_string())
	}
}

/// One client per sharing session; no credentials, proxy, redirects, persistence or retries.
pub fn client() -> Result<Client, &'static str> {
	Client::builder()
		.https_only(true)
		.no_proxy()
		.redirect(reqwest::redirect::Policy::none())
		.timeout(Duration::from_secs(10))
		.connect_timeout(Duration::from_secs(5))
		.pool_max_idle_per_host(1)
		.build()
		.map_err(|_| "Game application lookup is unavailable.")
}

pub async fn metadata(
	client: &Client,
	cooldown: &mut Instant,
	id: Id,
) -> Result<Metadata, &'static str> {
	let bytes = download(
		client,
		cooldown,
		&format!("https://discord.com/api/v10/applications/{id}/rpc"),
		MAX_METADATA,
	)
	.await?;
	let mut metadata = decode_application(id, &bytes)?;
	// Artwork failure must not prevent the game's text presence.
	metadata.assets = assets(client, cooldown, id).await.unwrap_or_default();
	Ok(metadata)
}

/// Registered artwork is uploaded while a game runs, so this list is refetchable on a miss.
pub async fn assets(
	client: &Client,
	cooldown: &mut Instant,
	id: Id,
) -> Result<Vec<Asset>, &'static str> {
	let bytes = download(
		client,
		cooldown,
		&format!("https://discord.com/api/v10/oauth2/applications/{id}/assets"),
		MAX_METADATA,
	)
	.await?;
	decode_assets(&bytes).ok_or("Game artwork metadata is invalid.")
}

pub(crate) async fn download(
	client: &Client,
	cooldown: &mut Instant,
	url: &str,
	limit: usize,
) -> Result<Vec<u8>, &'static str> {
	let failure = "Game application lookup failed. Restart the game to retry.";
	if Instant::now() < *cooldown {
		return Err("Game application lookup is rate limited. Try again later.");
	}
	let mut response = client.get(url).send().await.map_err(|_| failure)?;
	if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
		let seconds = response
			.headers()
			.get(reqwest::header::RETRY_AFTER)
			.and_then(|v| v.to_str().ok())
			.and_then(|v| v.parse::<f64>().ok())
			.filter(|v| v.is_finite() && *v >= 0.0)
			.unwrap_or(60.0);
		*cooldown = Instant::now()
			.checked_add(Duration::try_from_secs_f64(seconds.max(1.0)).unwrap_or(Duration::MAX))
			.unwrap_or_else(|| Instant::now() + Duration::from_secs(100 * 365 * 24 * 60 * 60));
		return Err("Game application lookup is rate limited. Try again later.");
	}
	if !response.status().is_success() {
		return Err(failure);
	}
	if response
		.content_length()
		.is_some_and(|size| size > limit as u64)
	{
		return Err(failure);
	}
	let mut bytes = Vec::new();
	while let Some(chunk) = response.chunk().await.map_err(|_| failure)? {
		if bytes.len().saturating_add(chunk.len()) > limit {
			return Err(failure);
		}
		bytes.extend_from_slice(&chunk);
	}
	Ok(bytes)
}

fn decode_application(id: Id, bytes: &[u8]) -> Result<Metadata, &'static str> {
	let failure = "Game application metadata is invalid.";
	if bytes.len() > MAX_METADATA {
		return Err(failure);
	}
	let value: Application = serde_json::from_slice(bytes).map_err(|_| failure)?;
	if value.id != id
		|| value.name.trim().is_empty()
		|| value.name.len() > 128
		|| value.name.chars().any(char::is_control)
	{
		return Err(failure);
	}
	Ok(Metadata {
		name: value.name,
		assets: Vec::new(),
	})
}
fn decode_assets(bytes: &[u8]) -> Option<Vec<Asset>> {
	if bytes.len() > MAX_METADATA {
		return None;
	}
	let assets: Vec<Asset> = serde_json::from_slice(bytes).ok()?;
	(assets.len() <= MAX_ASSETS
		&& assets.iter().all(|asset| {
			!asset.name.is_empty()
				&& asset.name.len() <= 256
				&& !asset.name.chars().any(char::is_control)
		}))
	.then_some(assets)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[tokio::test]
	async fn lookup_rejects_redirects_oversize_and_honors_shared_cooldown() {
		use tokio::io::{AsyncReadExt, AsyncWriteExt};
		let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
		let url = format!("http://{}/synthetic", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			for response in [
				"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/private\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
				"HTTP/1.1 200 OK\r\nContent-Length: 262145\r\nConnection: close\r\n\r\n",
				"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 120\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
			] {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut bytes = [0; 2048];
				let length = socket.read(&mut bytes).await.unwrap();
				assert!(
					!String::from_utf8_lossy(&bytes[..length])
						.to_lowercase()
						.contains("authorization")
				);
				socket.write_all(response.as_bytes()).await.unwrap();
			}
		});
		let client = Client::builder()
			.no_proxy()
			.redirect(reqwest::redirect::Policy::none())
			.timeout(Duration::from_secs(2))
			.build()
			.unwrap();
		let mut cooldown = Instant::now();
		for _ in 0..3 {
			assert!(
				download(&client, &mut cooldown, &url, MAX_METADATA)
					.await
					.is_err()
			);
		}
		assert!(cooldown > Instant::now() + Duration::from_secs(110));
		server.await.unwrap();
		assert!(
			download(&client, &mut cooldown, &url, MAX_METADATA)
				.await
				.is_err()
		);
	}

	#[test]
	fn application_identity_and_registered_assets_are_bounded() {
		let mut metadata =
			decode_application(Id(7), br#"{"id":"7","name":"A game outside the old list"}"#)
				.unwrap();
		assert!(decode_application(Id(8), br#"{"id":"7","name":"Wrong application"}"#).is_err());
		assert!(decode_application(Id(7), &vec![b' '; MAX_METADATA + 1]).is_err());
		metadata.assets = decode_assets(br#"[{"id":"9","name":"map"}]"#).unwrap();
		assert_eq!(metadata.asset("map").as_deref(), Some("9"));
		assert_eq!(metadata.asset("9").as_deref(), Some("9"));
		assert!(metadata.asset("https://localhost/private").is_none());
		let assets = format!(
			"[{}]",
			vec![r#"{"id":"9","name":"map"}"#; MAX_ASSETS + 1].join(",")
		);
		assert!(decode_assets(assets.as_bytes()).is_none());
	}
}
