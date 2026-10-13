//! Documented routes; user-account interoperability remains unverified.
use super::*;
use client_core::soundboard::{self, Event, Request, Sound};
use serde::Deserialize;

#[derive(Deserialize)]
struct WireSound {
	name: String,
	sound_id: Id,
	emoji_name: Option<String>,
	guild_id: Option<Id>,
	available: bool,
}
fn defaults(bytes: &[u8]) -> Result<Vec<Sound>, Failure> {
	if bytes.len() > soundboard::MAX_BYTES {
		return Err(Failure::Capacity);
	}
	let wire: Vec<WireSound> = decode(bytes).map_err(|_| Failure::Protocol)?;
	if wire.len() > soundboard::MAX_SOUNDS || wire.iter().any(|s| s.guild_id.is_some()) {
		return Err(Failure::Protocol);
	}
	let sounds = wire
		.into_iter()
		.map(|s| Sound {
			id: s.sound_id,
			name: s.name,
			emoji: s.emoji_name,
			available: s.available,
		})
		.collect();
	if !soundboard::valid(&sounds) {
		return Err(Failure::Protocol);
	}
	Ok(sounds)
}
impl DiscordApi {
	pub(crate) async fn soundboard(&self, request: Request) -> Event {
		match request {
			Request::List { request } => Event::Listed {
				request,
				result: match self
					.request_limited(
						Method::GET,
						"/soundboard-default-sounds",
						None,
						soundboard::MAX_BYTES,
					)
					.await
				{
					Ok(bytes) => defaults(&bytes),
					Err(e) => Err(e),
				},
			},
			Request::Play {
				request,
				channel,
				call,
				sound,
			} => Event::Played {
				request,
				channel,
				call,
				result: if channel.0 == 0 || sound.0 == 0 {
					Err(Failure::Protocol)
				} else {
					self.request_limited(
						Method::POST,
						&format!("/channels/{channel}/send-soundboard-sound"),
						Some(serde_json::json!({"sound_id":sound.to_string()})),
						4096,
					)
					.await
					.map(|_| ())
				},
			},
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[tokio::test]
	async fn loopback_catalog_and_explicit_send_keep_original_call_identity() {
		use tokio::{
			io::{AsyncReadExt, AsyncWriteExt},
			net::TcpListener,
			time::timeout,
		};
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_SOUNDBOARD_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			for send in [false, true] {
				let (mut socket, _) = timeout(Duration::from_secs(5), listener.accept())
					.await
					.unwrap()
					.unwrap();
				let mut header = Vec::new();
				while !header.ends_with(b"\r\n\r\n") {
					assert!(header.len() < 4096);
					header.push(socket.read_u8().await.unwrap());
				}
				let header = String::from_utf8(header).unwrap();
				if send {
					assert!(
						header.starts_with("POST /channels/20/send-soundboard-sound HTTP/1.1\r\n")
					);
					let length = header
						.lines()
						.find_map(|line| {
							line.to_ascii_lowercase()
								.strip_prefix("content-length: ")
								.and_then(|value| value.parse::<usize>().ok())
						})
						.unwrap();
					assert!(length <= 128);
					let mut body = vec![0; length];
					socket.read_exact(&mut body).await.unwrap();
					assert_eq!(
						serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
						serde_json::json!({"sound_id":"1"})
					);
					socket
						.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
						.await
						.unwrap();
				} else {
					assert!(header.starts_with("GET /soundboard-default-sounds HTTP/1.1\r\n"));
					let body = r#"[{"name":"synthetic","sound_id":"1","available":true}]"#;
					let response = format!(
						"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
						body.len()
					);
					socket.write_all(response.as_bytes()).await.unwrap();
				}
			}
		});
		assert!(
			matches!(api.soundboard(Request::List { request: 7 }).await, Event::Listed { request: 7, result: Ok(sounds) } if sounds.len() == 1 && sounds[0].id == Id(1))
		);
		assert!(matches!(
			api.soundboard(Request::Play {
				request: 8,
				channel: Id(20),
				call: 9,
				sound: Id(1)
			})
			.await,
			Event::Played {
				request: 8,
				channel: Id(20),
				call: 9,
				result: Ok(())
			}
		));
		server.await.unwrap();
	}
	#[test]
	fn default_catalog_rejects_custom_duplicate_and_oversized_sounds() {
		let json = br#"[{"name":"quack","sound_id":"1","emoji_name":"Q","available":true}]"#;
		let sounds = defaults(json).unwrap();
		assert_eq!(sounds[0].id, Id(1));
		assert!(
			defaults(br#"[{"name":"custom","sound_id":"1","guild_id":"7","available":true}]"#)
				.is_err()
		);
		assert!(defaults(br#"[{"name":"a","sound_id":"1","available":true},{"name":"b","sound_id":"1","available":true}]"#).is_err());
		assert!(defaults(&vec![b' '; soundboard::MAX_BYTES + 1]).is_err());
	}
}
