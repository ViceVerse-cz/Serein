use crate::{DiscordApi, Failure};
use discord_protocol::gif_favorites::{self, Decoded, MAX_RESPONSE};
use model::Gif;
use reqwest::Method;

impl DiscordApi {
	async fn read_gif_favorites(&self) -> Result<Decoded, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				"/users/@me/settings-proto/2",
				None,
				MAX_RESPONSE,
			)
			.await?;
		gif_favorites::decode_response(&bytes)
			.map_err(|_| Failure::ProtocolAt("gif-favorites-sync-unsupported"))
	}
	pub async fn gif_favorites(&self, change: Option<(Gif, bool)>) -> Result<Vec<Gif>, Failure> {
		if change.as_ref().is_some_and(|(gif, favorite)| {
			!gif.valid()
				|| (*favorite
					&& !model::valid_gif_preview(&gif.preview)
					&& !model::valid_gif_video_source(&gif.preview))
		}) {
			return Err(Failure::Protocol);
		}
		let current = self.read_gif_favorites().await?;
		let Some((gif, favorite)) = change else {
			return current.favorites().map_err(|_| Failure::Protocol);
		};
		if current.contains(&gif.url).map_err(|_| Failure::Protocol)? == favorite {
			return current
				.favorites_for(favorite.then_some(gif.url.as_str()))
				.map_err(|_| Failure::Protocol);
		}
		let patch = gif_favorites::encode_patch(&current, &gif, favorite)
			.map_err(|_| Failure::ProtocolAt("gif-favorites-sync-unsupported"))?;
		let bytes = self
			.request_limited(
				Method::PATCH,
				"/users/@me/settings-proto/2",
				Some(serde_json::json!({"settings":patch,"required_data_version":current.version})),
				MAX_RESPONSE,
			)
			.await?;
		let saved = gif_favorites::decode_response(&bytes)
			.map_err(|_| Failure::ProtocolAt("gif-favorites-sync-unconfirmed"))?;
		if saved.version <= current.version
			|| !saved
				.matches(&gif, favorite)
				.map_err(|_| Failure::Protocol)?
			|| !current
				.unchanged_except(&saved, &gif.url)
				.map_err(|_| Failure::Protocol)?
		{
			return Err(Failure::ProtocolAt("gif-favorites-sync-unconfirmed"));
		}
		saved
			.favorites_for(favorite.then_some(gif.url.as_str()))
			.map_err(|_| Failure::Protocol)
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
	const BASE: &str = include_str!("../tests/fixtures/gif-favorites-base.json");
	const SAVED: &str = include_str!("../tests/fixtures/gif-favorites-saved.json");
	const REMOVED: &str = include_str!("../tests/fixtures/gif-favorites-removed.json");
	const PARTIAL: &str = include_str!("../tests/fixtures/gif-favorites-partial.json");
	const ADD_PATCH: &str = include_str!("../tests/fixtures/gif-favorites-add-patch.txt");
	const REMOVE_PATCH: &str = include_str!("../tests/fixtures/gif-favorites-remove-patch.txt");
	fn gif(index: usize) -> Gif {
		Gif {
			id: format!("test-{index}"),
			title: "Synthetic".into(),
			url: format!("https://tenor.com/view/synthetic-{index}"),
			preview: format!("https://media.tenor.com/synthetic/{index}.gif"),
			width: 300,
			height: 200,
		}
	}
	#[tokio::test]
	async fn favorite_writes_read_fresh_preserve_hidden_entries_and_never_retry_ambiguous_results()
	{
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_GIF_SETTINGS".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(async move {
				let mut outdated: serde_json::Value = serde_json::from_str(SAVED).unwrap();
				outdated["out_of_date"] = true.into();
				let outdated = outdated.to_string();
				for (method, patch, version, response) in [
					("GET", None, 0, BASE),
					("GET", None, 0, BASE),
					("PATCH", Some(ADD_PATCH), 7, SAVED),
					("GET", None, 0, SAVED),
					("PATCH", Some(REMOVE_PATCH), 8, REMOVED),
					("GET", None, 0, REMOVED),
					("PATCH", Some(ADD_PATCH), 9, outdated.as_str()),
					("GET", None, 0, BASE),
					("GET", None, 0, BASE),
					("PATCH", Some(ADD_PATCH), 7, PARTIAL),
				] {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut request = Vec::new();
					let header_end = loop {
						let mut chunk = [0; 4096];
						let count = socket.read(&mut chunk).await.unwrap();
						assert!(count > 0);
						request.extend_from_slice(&chunk[..count]);
						assert!(request.len() <= 256 * 1024);
						if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
							break end + 4;
						}
					};
					let headers = std::str::from_utf8(&request[..header_end]).unwrap();
					assert!(headers.starts_with(&format!(
						"{method} /users/@me/settings-proto/2 HTTP/1.1\r\n"
					)));
					let length = headers
						.lines()
						.find_map(|line| {
							line.to_ascii_lowercase()
								.strip_prefix("content-length: ")
								.and_then(|value| value.parse::<usize>().ok())
						})
						.unwrap_or(0);
					assert!(length <= 256 * 1024);
					while request.len() < header_end + length {
						let mut chunk = [0; 4096];
						let count = socket.read(&mut chunk).await.unwrap();
						assert!(count > 0);
						request.extend_from_slice(&chunk[..count]);
						assert!(request.len() <= 256 * 1024);
					}
					if let Some(patch) = patch {
						let body: serde_json::Value =
							serde_json::from_slice(&request[header_end..header_end + length])
								.unwrap();
						assert_eq!(
							body,
							serde_json::json!({"settings":patch,"required_data_version":version})
						);
					}
					socket
						.write_all(
							format!(
								"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
								response.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
				}
			});
			assert_eq!(api.gif_favorites(None).await.unwrap().len(), 100);
			let added = gif(1000);
			let saved = api
				.gif_favorites(Some((added.clone(), true)))
				.await
				.unwrap();
			assert_eq!(saved[0].url, added.url);
			assert_eq!(saved.len(), 100);
			assert_eq!(
				api.gif_favorites(Some((added.clone(), false)))
					.await
					.unwrap()
					.len(),
				100
			);
			assert!(
				api.gif_favorites(Some((added.clone(), true)))
					.await
					.is_err()
			);
			assert_eq!(
				api.gif_favorites(Some((gif(0), true))).await.unwrap().len(),
				100
			);
			assert!(api.gif_favorites(Some((added, true))).await.is_err());
			let mut invalid = gif(2000);
			invalid.preview = "https://example.invalid/file.gif".into();
			assert!(api.gif_favorites(Some((invalid, true))).await.is_err());
			server.await.unwrap();
		})
		.await
		.unwrap();
	}
}
