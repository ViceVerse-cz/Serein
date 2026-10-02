use crate::{DiscordApi, Failure};
use client_core::soundboard::{
	Action, MAX_BYTES, MAX_SOUNDS, Outcome, Request, Sound, catalog_bytes, valid_catalog,
};
use reqwest::Method;

impl DiscordApi {
	pub(super) async fn soundboard(&self, request: Request) -> Result<Outcome, Failure> {
		let scope = request.scope;
		if scope.channel.0 == 0 || scope.guild.0 == 0 {
			return Err(Failure::Protocol);
		}
		match request.action {
			Action::Load => {
				let bytes = self
					.request_limited(Method::GET, "/soundboard-default-sounds", None, MAX_BYTES)
					.await?;
				let mut sounds = discord_protocol::soundboard::sounds(&bytes, None)
					.map_err(|_| Failure::Protocol)?;
				drop(bytes);
				let bytes = self
					.request_limited(
						Method::GET,
						&format!("/guilds/{}/soundboard-sounds", scope.guild),
						None,
						MAX_BYTES,
					)
					.await?;
				let guild = discord_protocol::soundboard::sounds(&bytes, Some(scope.guild))
					.map_err(|_| Failure::Protocol)?;
				drop(bytes);
				if sounds.len() + guild.len() > MAX_SOUNDS {
					return Err(Failure::Capacity);
				}
				sounds.extend(guild);
				sounds.shrink_to_fit();
				if !valid_catalog(&sounds, scope.guild)
					|| catalog_bytes(&sounds)
						+ (sounds.capacity() - sounds.len()) * size_of::<Sound>()
						> MAX_BYTES
				{
					return Err(Failure::Protocol);
				}
				Ok(Outcome::Loaded(sounds))
			}
			Action::Play(sound) => {
				if sound.0 == 0 {
					return Err(Failure::Protocol);
				}
				let bytes = self
					.request_with_content(
						Method::POST,
						&format!("/channels/{}/send-soundboard-sound", scope.channel),
						Some(crate::RequestContent::JsonNoContent(
							serde_json::json!({"sound_id":sound.to_string()}),
						)),
						MAX_BYTES,
						None,
						None,
					)
					.await?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
				Ok(Outcome::Played)
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Command, Event, auth::SessionSecret, soundboard::Scope};
	use model::Id;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	async fn respond(
		listener: &TcpListener,
		route: &str,
		body: Option<serde_json::Value>,
		status: u16,
		response: &str,
	) {
		let (mut socket, _) = listener.accept().await.unwrap();
		let mut bytes = Vec::new();
		loop {
			let mut chunk = [0; 1024];
			let count = socket.read(&mut chunk).await.unwrap();
			assert!(count > 0);
			bytes.extend_from_slice(&chunk[..count]);
			assert!(bytes.len() <= 8192);
			if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
				let headers = std::str::from_utf8(&bytes[..end]).unwrap();
				let length = headers
					.lines()
					.find_map(|line| {
						line.to_ascii_lowercase()
							.strip_prefix("content-length: ")
							.and_then(|n| n.parse::<usize>().ok())
					})
					.unwrap_or_default();
				if bytes.len() < end + 4 + length {
					continue;
				}
				assert!(headers.starts_with(&format!("{route} HTTP/1.1\r\n")));
				assert_eq!(
					if length == 0 {
						None
					} else {
						Some(serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap())
					},
					body
				);
				break;
			}
		}
		socket
			.write_all(
				format!(
					"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
					response.len()
				)
				.as_bytes(),
			)
			.await
			.unwrap();
	}
	fn request(action: Action) -> Request {
		Request {
			scope: Scope {
				generation: 1,
				channel: Id(20),
				guild: Id(10),
				call_request: 7,
			},
			request: 1,
			action,
		}
	}
	fn api(listener: &TcpListener) -> DiscordApi {
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_SOUNDBOARD_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		api
	}
	#[tokio::test]
	async fn soundboard_reads_default_and_current_guild_and_writes_only_selected_id() {
		tokio::time::timeout(Duration::from_secs(5), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let api = api(&listener);
			let server = async {
				respond(
					&listener,
					"GET /soundboard-default-sounds",
					None,
					200,
					r#"[{"sound_id":"1","name":"Quack","volume":1.0,"available":true}]"#,
				)
				.await;
				respond(
					&listener,
					"GET /guilds/10/soundboard-sounds",
					None,
					200,
					r#"{"items":[{"sound_id":"99","name":"Yay","volume":0.5,"guild_id":"10","available":true}]}"#,
				)
				.await;
			};
			let (event, ()) = tokio::join!(
				api.execute(Command::Soundboard(request(Action::Load))),
				server
			);
			let Event::Soundboard(event) = event else {
				panic!("wrong event")
			};
			let Outcome::Loaded(sounds) = event.result.unwrap() else {
				panic!("wrong result")
			};
			assert_eq!(sounds.len(), 2);
			let server = respond(
				&listener,
				"POST /channels/20/send-soundboard-sound",
				Some(serde_json::json!({"sound_id":"99"})),
				204,
				"",
			);
			let (event, ()) = tokio::join!(
				api.execute(Command::Soundboard(request(Action::Play(Id(99))))),
				server
			);
			assert!(matches!(
				event,
				Event::Soundboard(client_core::soundboard::Event {
					result: Ok(Outcome::Played),
					..
				})
			));
		})
		.await
		.unwrap();
	}
	#[tokio::test]
	async fn soundboard_rejection_and_unexpected_ack_are_not_retried() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let api = api(&listener);
		for (status, response, failure) in [
			(403, "{}", Failure::Forbidden),
			(200, "", Failure::Ambiguous),
		] {
			let (event, ()) = tokio::join!(
				api.execute(Command::Soundboard(request(Action::Play(Id(99))))),
				respond(
					&listener,
					"POST /channels/20/send-soundboard-sound",
					Some(serde_json::json!({"sound_id":"99"})),
					status,
					response
				)
			);
			let Event::Soundboard(event) = event else {
				panic!("wrong event")
			};
			assert!(matches!(event.result,Err(actual) if actual==failure));
			assert!(!api.stopped());
			assert!(
				tokio::time::timeout(Duration::from_millis(50), listener.accept())
					.await
					.is_err()
			);
		}
	}
}
