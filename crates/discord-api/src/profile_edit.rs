use crate::{DiscordApi, Failure};
use discord_protocol::{UserDto, profile};
use model::{Id, ProfileEdit, UserProfile};
use reqwest::Method;

impl DiscordApi {
	pub(super) async fn edit_profile(
		&self,
		user: Id,
		changes: Option<ProfileEdit>,
	) -> Result<Box<UserProfile>, Failure> {
		if user.0 == 0 {
			return Err(Failure::Protocol);
		}
		if let Some(changes) = &changes {
			let body = profile::encode_edit(changes).map_err(|_| Failure::Protocol)?;
			if changes != &ProfileEdit::default() {
				// One explicitly requested write, without retries. Account responses may
				// contain tokens; ignore those fields and clear the owned response buffer.
				let bytes = zeroize::Zeroizing::new(
					self.request_limited(
						Method::PATCH,
						"/users/@me",
						Some(body),
						profile::MAX_PROFILE_WIRE,
					)
					.await?,
				);
				let saved: UserDto = discord_protocol::decode(&bytes).map_err(|_| {
					Failure::ProtocolAt(
						"Profile save could not be confirmed; reload before retrying",
					)
				})?;
				if saved.id != user || saved.bot {
					return Err(Failure::Protocol);
				}
			}
		}
		let path = format!(
			"/users/{user}/profile?with_mutual_guilds=false&with_mutual_friends=false&with_mutual_friends_count=false"
		);
		let bytes = self
			.request_limited(Method::GET, &path, None, profile::MAX_PROFILE_WIRE)
			.await?;
		let saved = profile::decode_profile(&bytes, None).map_err(|_| Failure::Protocol)?;
		if saved.user.id != user || saved.limited || saved.guild.is_some() {
			return Err(Failure::ProtocolAt(
				"Full account profile is unavailable; reload before editing",
			));
		}
		if let Some(changes) = changes
			&& (changes
				.global_name
				.as_ref()
				.is_some_and(|name| *name != saved.global_name)
				|| changes.bio.as_ref().is_some_and(|bio| *bio != saved.bio)
				|| changes
					.pronouns
					.as_ref()
					.is_some_and(|pronouns| *pronouns != saved.pronouns)
				|| changes
					.accent_color
					.is_some_and(|color| color != saved.accent_color)
				|| changes
					.avatar
					.as_ref()
					.is_some_and(|avatar| avatar.is_some() != saved.user.avatar.is_some()))
		{
			return Err(Failure::ProtocolAt(
				"Profile changes were not confirmed; reload before retrying",
			));
		}
		Ok(Box::new(saved))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Command, Event, auth::SessionSecret};
	use serde_json::{Value, json};
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	const GET: &str = "GET /users/1/profile?with_mutual_guilds=false&with_mutual_friends=false&with_mutual_friends_count=false";
	const PATCH: &str = "PATCH /users/@me";
	const USER: &str =
		r#"{"id":"1","username":"synthetic","global_name":null,"avatar":null,"discriminator":"0"}"#;
	const PROFILE: &str = r#"{"user":{"id":"1","username":"synthetic","global_name":null,"avatar":null,"discriminator":"0"},"user_profile":{"bio":"hello","pronouns":"","accent_color":null}}"#;

	async fn respond(
		listener: &TcpListener,
		route: &str,
		body: Option<Value>,
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
							.and_then(|length| length.parse::<usize>().ok())
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
						Some(
							serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
								.unwrap(),
						)
					},
					body
				);
				break;
			}
		}
		socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nRetry-After: 0.001\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await.unwrap();
	}

	#[tokio::test]
	async fn profile_edits_send_only_changes_confirm_readback_and_do_not_retry() {
		tokio::time::timeout(Duration::from_secs(10), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_PROFILE_EDIT_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let edit = ProfileEdit {
				global_name: Some(None),
				bio: Some("hello".into()),
				pronouns: Some(String::new()),
				accent_color: Some(None),
				avatar: None,
			};
			let server = async {
				respond(
					&listener,
					PATCH,
					Some(
						json!({"global_name":null,"bio":"hello","pronouns":"","accent_color":null}),
					),
					200,
					USER,
				)
				.await;
				respond(&listener, GET, None, 200, PROFILE).await;
			};
			let (event, ()) = tokio::join!(
				api.execute(Command::EditProfile {
					user: Id(1),
					request: 42,
					changes: Some(edit)
				}),
				server
			);
			let Event::ProfileEdited {
				user,
				request,
				result,
			} = event
			else {
				panic!("wrong event")
			};
			assert_eq!((user, request), (Id(1), 42));
			assert_eq!(result.unwrap().bio, "hello");
			let set = ProfileEdit {
				global_name: Some(Some("New name".into())),
				bio: Some(String::new()),
				pronouns: Some("they/them".into()),
				accent_color: Some(Some(0x123456)),
				avatar: None,
			};
			let server = async {
				respond(&listener, PATCH, Some(json!({"global_name":"New name","bio":"","pronouns":"they/them","accent_color":0x123456})), 200, USER).await;
				let saved = json!({"user":{"id":"1","username":"synthetic","global_name":"New name"},"user_profile":{"bio":"","pronouns":"they/them","accent_color":0x123456}}).to_string();
				respond(&listener, GET, None, 200, &saved).await;
			};
			let (result, ()) = tokio::join!(api.edit_profile(Id(1), Some(set)), server);
			assert_eq!(result.unwrap().user.name, "New name");

			let only_bio = ProfileEdit {
				bio: Some("hello".into()),
				..Default::default()
			};
			for changes in [None, Some(ProfileEdit::default()), Some(only_bio.clone())] {
				let server = async {
					if changes.as_ref().is_some_and(|edit| edit.bio.is_some()) {
						respond(&listener, PATCH, Some(json!({"bio":"hello"})), 200, USER).await;
					}
					respond(&listener, GET, None, 200, PROFILE).await;
				};
				let (result, ()) = tokio::join!(api.edit_profile(Id(1), changes.clone()), server);
				assert_eq!(result.unwrap().bio, "hello");
			}

			for (status, body, expected) in [
				(401, "{}", Failure::Expired),
				(403, "{}", Failure::Forbidden),
				(429, r#"{"retry_after":0.001}"#, Failure::RateLimited),
				(500, "{}", Failure::Ambiguous),
				(200, r#"{"id":"2","username":"other"}"#, Failure::Protocol),
			] {
				let mut api = DiscordApi::new(Arc::new(
					SessionSecret::from_owner_input("SYNTHETIC_PROFILE_EDIT_TOKEN".into()).unwrap(),
				))
				.unwrap();
				api.base = format!("http://{}", listener.local_addr().unwrap());
				let (result, ()) = tokio::join!(
					api.edit_profile(Id(1), Some(only_bio.clone())),
					respond(&listener, PATCH, Some(json!({"bio":"hello"})), status, body)
				);
				assert_eq!(result.err(), Some(expected));
			}
			let server = async {
				respond(&listener, PATCH, Some(json!({"bio":"hello"})), 200, USER).await;
				respond(
					&listener,
					GET,
					None,
					200,
					&PROFILE.replace("hello", "old value"),
				)
				.await;
			};
			let (result, ()) = tokio::join!(api.edit_profile(Id(1), Some(only_bio)), server);
			assert!(matches!(result, Err(Failure::ProtocolAt(_))));
			for body in [format!(r#"{{"user":{USER}}}"#), PROFILE.replace(r#""id":"1""#, r#""id":"2""#)] {
				let (result, ()) = tokio::join!(api.edit_profile(Id(1), None), respond(&listener, GET, None, 200, &body));
				assert!(matches!(result, Err(Failure::ProtocolAt(_))));
			}

			let too_long = ProfileEdit {
				bio: Some("x".repeat(191)),
				..Default::default()
			};
			assert_eq!(
				api.edit_profile(Id(1), Some(too_long)).await.err(),
				Some(Failure::Protocol)
			);
			assert_eq!(
				api.edit_profile(Id(0), None).await.err(),
				Some(Failure::Protocol)
			);
			assert!(
				tokio::time::timeout(Duration::from_millis(10), listener.accept())
					.await
					.is_err(),
				"invalid edits and retry attempts must not reach the transport"
			);
		})
		.await
		.unwrap();
	}
}
