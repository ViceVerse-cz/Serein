//! Documented soundboard REST: the default catalog and playing a sound in a voice channel.
use crate::{DiscordApi, Failure};
use model::Id;
use wreq::Method;

/// The default catalog is a few kilobytes; this leaves generous room without unbounded reads.
const MAX_DEFAULTS_RESPONSE: usize = 256 * 1024;
/// A successful send is `204 No Content`; error bodies are short JSON.
const MAX_SEND_RESPONSE: usize = 16 * 1024;

/// An oversized optional response is a local failure, never a session-ending capacity error.
fn optional(failure: Failure) -> Failure {
	match failure {
		Failure::Capacity | Failure::CapacityAt(_) => {
			Failure::ProtocolAt("Discord's soundboard response was too large")
		}
		failure => failure,
	}
}

impl DiscordApi {
	/// `GET /soundboard-default-sounds`, bounded in size; a malformed catalog is local to the
	/// picker and never ends the session.
	pub(crate) async fn default_soundboard_sounds(
		&self,
	) -> Result<Vec<model::soundboard::Sound>, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				"/soundboard-default-sounds",
				None,
				MAX_DEFAULTS_RESPONSE,
			)
			.await
			.map_err(optional)?;
		discord_protocol::soundboard::default_sounds(&bytes)
			.map_err(|_| Failure::ProtocolAt("Default sounds were in an unsupported format"))
	}
	/// `POST /channels/{channel}/send-soundboard-sound`. Discord plays the sound for everyone,
	/// including this client, through `VOICE_CHANNEL_EFFECT_SEND`; no audio is sent by RTP.
	pub(crate) async fn send_soundboard_sound(
		&self,
		channel: Id,
		sound: Id,
		source_guild: Option<Id>,
	) -> Result<(), Failure> {
		let mut body = serde_json::json!({ "sound_id": sound.to_string() });
		if let Some(guild) = source_guild {
			body["source_guild_id"] = guild.to_string().into();
		}
		self.request_limited(
			Method::POST,
			&format!("/channels/{channel}/send-soundboard-sound"),
			Some(body),
			MAX_SEND_RESPONSE,
		)
		.await
		.map(drop)
		.map_err(optional)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{
		Event,
		auth::SessionSecret,
		soundboard::{Command, Event as SoundEvent},
	};
	use std::sync::Arc;
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	/// Accept one request and answer it; returns the request line and JSON body.
	async fn answer(listener: TcpListener, status: &str, body: &'static str) -> (String, String) {
		let (mut stream, _) = listener.accept().await.unwrap();
		let mut bytes = Vec::new();
		let (head, body_start, length) = loop {
			let mut chunk = [0; 1024];
			let count = stream.read(&mut chunk).await.unwrap();
			assert!(count > 0 && bytes.len() < 64 * 1024);
			bytes.extend_from_slice(&chunk[..count]);
			let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
				continue;
			};
			let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
			let length = head
				.lines()
				.find_map(|line| {
					line.to_ascii_lowercase()
						.strip_prefix("content-length: ")
						.and_then(|value| value.parse::<usize>().ok())
				})
				.unwrap_or(0);
			if bytes.len() >= end + 4 + length {
				break (head, end + 4, length);
			}
		};
		let request_body =
			String::from_utf8(bytes[body_start..body_start + length].to_vec()).unwrap();
		stream
			.write_all(
				format!(
					"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				)
				.as_bytes(),
			)
			.await
			.unwrap();
		(head.lines().next().unwrap().to_owned(), request_body)
	}
	/// A client whose REST base points at a fresh local listener.
	async fn local_api() -> (DiscordApi, TcpListener) {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_SOUNDBOARD_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		(api, listener)
	}

	/// The send payload carries `source_guild_id` only for another server's sound, and refusals
	/// map to local failures.
	#[tokio::test]
	async fn sends_external_sounds_with_their_source_and_maps_refusals() {
		let (api, listener) = local_api().await;
		let server = tokio::spawn(answer(listener, "204 No Content", ""));
		let event = api
			.execute(client_core::Command::Soundboard(Command::Play {
				request: 4,
				channel: Id(20),
				sound: Id(9),
				source_guild: Some(Id(11)),
			}))
			.await;
		let (line, body) = server.await.unwrap();
		assert_eq!(line, "POST /channels/20/send-soundboard-sound HTTP/1.1");
		assert_eq!(
			serde_json::from_str::<serde_json::Value>(&body).unwrap(),
			serde_json::json!({"sound_id":"9","source_guild_id":"11"})
		);
		assert!(matches!(
			event,
			Event::Soundboard(SoundEvent::Played {
				request: 4,
				result: Ok(())
			})
		));
		// A local sound omits the source; a missing permission is a plain refusal.
		let (api, listener) = local_api().await;
		let server = tokio::spawn(answer(
			listener,
			"403 Forbidden",
			r#"{"message":"Missing Permissions","code":50013}"#,
		));
		let result = api.send_soundboard_sound(Id(20), Id(1), None).await;
		let (_, body) = server.await.unwrap();
		assert_eq!(body, r#"{"sound_id":"1"}"#);
		assert_eq!(result, Err(Failure::Forbidden));
	}

	/// An oversized or malformed default catalog fails the picker, not the session.
	#[tokio::test]
	async fn default_catalog_is_bounded_and_never_ends_the_session() {
		let (api, listener) = local_api().await;
		let server = tokio::spawn(answer(
			listener,
			"200 OK",
			r#"[{"name":"quack","sound_id":"1","volume":1.0,"emoji_id":null,"emoji_name":"d","available":true}]"#,
		));
		let event = api
			.execute(client_core::Command::Soundboard(Command::Defaults))
			.await;
		let (line, _) = server.await.unwrap();
		assert_eq!(line, "GET /soundboard-default-sounds HTTP/1.1");
		assert!(
			matches!(event, Event::Soundboard(SoundEvent::Defaults(Ok(sounds))) if sounds[0].name == "quack")
		);
		let (api, listener) = local_api().await;
		let huge: &'static str =
			Box::leak(format!("[{}]", " ".repeat(MAX_DEFAULTS_RESPONSE)).into_boxed_str());
		let server = tokio::spawn(answer(listener, "200 OK", huge));
		let result = api.default_soundboard_sounds().await;
		let _ = server.await;
		assert!(result.is_err_and(|failure| !failure.ends_session()));
	}
}
