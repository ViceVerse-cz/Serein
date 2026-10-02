//! Linked-account Spotify polling. No playback control, credential extraction or persisted token.
//! Discord's token route is unofficial (discord.py-self HTTPClient.get_connection_token).
//! /v1/me/player requires user-read-playback-state; linked-token scope support is live-unverified.
use crate::{DiscordApi, Failure};
use discord_protocol::spotify::{Activity, MAX_PLAYBACK_BYTES, decode_playback};
use model::Id;
use reqwest::{
	Client, Method, StatusCode,
	header::{AUTHORIZATION, HeaderValue},
};
use serde::Deserialize;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

const MAX_CONNECTION_BYTES: usize = 64 * 1024;
const MAX_TOKEN_BYTES: usize = 16 * 1024;

pub struct Playback {
	client: Client,
	account: Option<(Id, String)>,
	token: Option<Zeroizing<String>>,
	token_until: Instant,
	retry_at: Option<Instant>,
}

impl Playback {
	/// Invisible drops credentials without resetting the service's Retry-After deadline.
	pub fn clear_token(&mut self) {
		self.token = None;
	}

	pub fn new() -> Result<Self, Failure> {
		Ok(Self {
			client: Client::builder()
				.https_only(true)
				.no_proxy()
				.redirect(reqwest::redirect::Policy::none())
				.retry(reqwest::retry::never())
				.timeout(Duration::from_secs(10))
				.connect_timeout(Duration::from_secs(5))
				.build()
				.map_err(|_| Failure::Network)?,
			account: None,
			token: None,
			token_until: Instant::now(),
			retry_at: Some(Instant::now()),
		})
	}

	/// Caller polls every 15 seconds and clears published activity on any error.
	/// The connection preference is re-read each time so disabling sharing stops publication.
	pub async fn poll(&mut self, api: &DiscordApi, user: Id) -> Result<Option<Activity>, Failure> {
		if api.stopped() || user.0 == 0 {
			self.token = None;
			return Err(Failure::Expired);
		}
		let bytes = Zeroizing::new(
			api.request_limited(
				Method::GET,
				"/users/@me/connections",
				None,
				MAX_CONNECTION_BYTES,
			)
			.await?,
		);
		let connection = sharing_connection(&bytes)?;
		let Some(connection) = connection else {
			self.token = None;
			self.account = None;
			return Ok(None);
		};
		if self
			.account
			.as_ref()
			.is_none_or(|(owner, id)| *owner != user || id != &connection)
		{
			self.token = None;
			self.account = Some((user, connection));
		}
		if self.retry_at.is_none_or(|at| Instant::now() < at) {
			return Err(Failure::RateLimited);
		}
		if Instant::now() >= self.token_until {
			self.token = None;
		}
		if self.token.is_none() {
			let connection = &self.account.as_ref().ok_or(Failure::Protocol)?.1;
			let bytes = Zeroizing::new(
				api.request_limited(
					Method::GET,
					&format!("/users/@me/connections/spotify/{connection}/access-token"),
					None,
					MAX_TOKEN_BYTES,
				)
				.await?,
			);
			#[derive(Deserialize)]
			struct Token<'a> {
				access_token: &'a str,
			}
			let token: Token<'_> = serde_json::from_slice(&bytes).map_err(|_| Failure::Protocol)?;
			if token.access_token.is_empty()
				|| token.access_token.len() > 8192
				|| !token.access_token.bytes().all(|b| b.is_ascii_graphic())
			{
				return Err(Failure::Protocol);
			}
			self.token = Some(Zeroizing::new(token.access_token.to_owned()));
			self.token_until = Instant::now() + Duration::from_secs(50 * 60);
		}
		let bearer = Zeroizing::new(format!(
			"Bearer {}",
			self.token.as_ref().ok_or(Failure::Protocol)?.as_str()
		));
		let mut authorization = HeaderValue::from_str(&bearer).map_err(|_| Failure::Protocol)?;
		authorization.set_sensitive(true);
		let mut response = self
			.client
			.get("https://api.spotify.com/v1/me/player")
			.header(AUTHORIZATION, authorization)
			.send()
			.await
			.map_err(|_| Failure::Network)?;
		match response.status() {
			StatusCode::NO_CONTENT => return Ok(None),
			StatusCode::UNAUTHORIZED => {
				self.token = None;
				return Err(Failure::ProtocolAt(
					"Spotify access expired; retrying shortly",
				));
			}
			StatusCode::FORBIDDEN => {
				self.token = None;
				self.retry_at = Some(Instant::now() + Duration::from_secs(300));
				return Err(Failure::ProtocolAt(
					"Spotify playback access is unavailable for this connection",
				));
			}
			StatusCode::TOO_MANY_REQUESTS => {
				// Invalid or unrepresentable Retry-After halts this poller instead of retrying early.
				self.retry_at = response
					.headers()
					.get("retry-after")
					.and_then(|v| v.to_str().ok())
					.and_then(|v| v.parse::<u64>().ok())
					.and_then(|seconds| {
						Instant::now().checked_add(Duration::from_secs(seconds.max(15)))
					});
				return Err(Failure::RateLimited);
			}
			StatusCode::OK => {}
			_ => return Err(Failure::Network),
		}
		if response
			.content_length()
			.is_some_and(|n| n > MAX_PLAYBACK_BYTES as u64)
		{
			return Err(Failure::Capacity);
		}
		let mut bytes = Vec::new();
		while let Some(chunk) = response.chunk().await.map_err(|_| Failure::Network)? {
			if bytes.len() + chunk.len() > MAX_PLAYBACK_BYTES {
				return Err(Failure::Capacity);
			}
			bytes.extend_from_slice(&chunk);
		}
		let now_ms = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map_err(|_| Failure::Protocol)?
			.as_millis();
		decode_playback(
			&bytes,
			user,
			u64::try_from(now_ms).map_err(|_| Failure::Protocol)?,
		)
		.map_err(|_| Failure::ProtocolAt("Spotify playback response is unsupported"))
	}
}

fn sharing_connection(bytes: &[u8]) -> Result<Option<String>, Failure> {
	#[derive(Deserialize)]
	struct Connection {
		id: String,
		#[serde(rename = "type")]
		kind: String,
		show_activity: Option<bool>,
		revoked: Option<bool>,
	}
	let connections: Vec<Connection> =
		serde_json::from_slice(bytes).map_err(|_| Failure::Protocol)?;
	if connections.len() > 64 {
		return Err(Failure::Capacity);
	}
	let Some(connection) = connections
		.into_iter()
		.find(|c| c.kind == "spotify" && c.show_activity == Some(true) && c.revoked != Some(true))
	else {
		return Ok(None);
	};
	if connection.id.is_empty()
		|| connection.id.len() > 128
		|| !connection
			.id
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
	{
		return Err(Failure::Protocol);
	}
	Ok(Some(connection.id))
}

/// Offline assertions for the desktop debug command; never performs an HTTP request.
#[cfg(debug_assertions)]
pub fn debug_check() {
	assert_eq!(
		sharing_connection(br#"[{"id":"synthetic_123","type":"spotify","show_activity":true}]"#)
			.unwrap(),
		Some("synthetic_123".into())
	);
	for bytes in [
		br#"[{"id":"synthetic","type":"spotify","show_activity":true,"revoked":true}]"#.as_slice(),
		br#"[{"id":"synthetic","type":"spotify","show_activity":false}]"#.as_slice(),
		br#"[{"id":"synthetic","type":"spotify"}]"#.as_slice(),
		br#"[{"id":"synthetic","type":"twitch","show_activity":true}]"#.as_slice(),
	] {
		assert_eq!(sharing_connection(bytes).unwrap(), None);
	}
	for id in [
		"",
		"../other",
		"a/b",
		"a?query",
		"a#fragment",
		"a%2fb",
		"a b",
	] {
		let bytes =
			serde_json::json!([{"id":id,"type":"spotify","show_activity":true}]).to_string();
		assert!(sharing_connection(bytes.as_bytes()).is_err());
	}
}
