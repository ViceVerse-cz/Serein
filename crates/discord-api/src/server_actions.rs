use crate::{DiscordApi, Failure};
use client_core::server_actions::Action;
use reqwest::Method;

impl DiscordApi {
	pub(super) async fn send_server_invite(
		&self,
		user: model::Id,
		code: &str,
		nonce: &str,
	) -> Result<(model::Channel, model::Message), Failure> {
		if user.0 == 0
			|| !client_core::invites::valid_code(code)
			|| nonce.is_empty()
			|| nonce.len() > 64
		{
			return Err(Failure::Protocol);
		}
		let channel = self.open_dm(user).await?;
		let content = format!("https://discord.gg/{code}");
		let message = self
			.send_message(channel.id, &content, nonce, None, None, None)
			.await
			.map_err(write_failure)?;
		if message.content != content || message.nonce.as_deref() != Some(nonce) {
			return Err(Failure::Ambiguous);
		}
		Ok((channel, message))
	}
	// Documented developer API routes; normal-user interoperability remains unverified.
	pub(super) async fn server_action(&self, action: Action) -> Result<Option<String>, Failure> {
		if action.guild().0 == 0 {
			return Err(Failure::Protocol);
		}
		match action {
			Action::Notifications { guild, options } => {
				if !options.valid() {
					return Err(Failure::Protocol);
				}
				let mut body = serde_json::Map::new();
				if let Some(level) = options.level {
					body.insert("message_notifications".into(), level.into());
				}
				for (name, value) in [
					("muted", options.muted),
					("suppress_everyone", options.suppress_everyone),
					("suppress_roles", options.suppress_roles),
				] {
					if let Some(value) = value {
						body.insert(name.into(), value.into());
					}
				}
				if options.muted.is_some() {
					body.insert(
						"mute_config".into(),
						serde_json::json!({"end_time":null,"selected_time_window":-1}),
					);
				}
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/users/@me/guilds/{guild}/settings"),
						Some(body.into()),
						512 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let setting: discord_protocol::notifications::Setting =
					discord_protocol::decode(&bytes).map_err(|_| Failure::Ambiguous)?;
				if setting.guild_id != Some(guild)
					|| options
						.level
						.is_some_and(|value| setting.message_notifications != Some(value))
					|| options
						.muted
						.is_some_and(|value| setting.muted != Some(value))
					|| options
						.suppress_everyone
						.is_some_and(|value| setting.suppress_everyone != Some(value))
					|| options
						.suppress_roles
						.is_some_and(|value| setting.suppress_roles != Some(value))
					|| (options.muted.is_some()
						&& setting
							.mute_config
							.as_ref()
							.and_then(|config| config.until())
							.is_some())
				{
					return Err(Failure::Ambiguous);
				}
				Ok(None)
			}
			// Unofficial normal-user route; deletion is owner-only and attempted once.
			Action::Delete(guild) => self
				.request_limited(Method::DELETE, &format!("/guilds/{guild}"), None, 64 * 1024)
				.await
				.map_err(write_failure)
				.and_then(|body| {
					if body.is_empty() {
						Ok(None)
					} else {
						Err(Failure::Ambiguous)
					}
				}),
			Action::Leave(guild) => self
				.request_limited(
					Method::DELETE,
					&format!("/users/@me/guilds/{guild}"),
					None,
					64 * 1024,
				)
				.await
				.map_err(write_failure)
				.and_then(|body| {
					if body.is_empty() {
						Ok(None)
					} else {
						Err(Failure::Ambiguous)
					}
				}),
			Action::CreateInvite {
				guild,
				channel,
				options,
			} => {
				if channel.0 == 0 || !options.valid() {
					return Err(Failure::Protocol);
				}
				let bytes = self
					.request_limited(
						Method::POST,
						&format!("/channels/{channel}/invites"),
						Some(
							serde_json::json!({"max_age":options.max_age,"max_uses":options.max_uses,"temporary":options.temporary,"unique":true}),
						),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				discord_protocol::invites::created_code(
					&bytes,
					guild,
					channel,
					options.max_age,
					options.max_uses,
					options.temporary,
				)
				.map(Some)
				.map_err(|_| Failure::Ambiguous)
			}
		}
	}
}
fn write_failure(failure: Failure) -> Failure {
	// A response exceeding admission bounds can still belong to a completed write.
	if failure == Failure::Capacity {
		Failure::Ambiguous
	} else {
		failure
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Command, Event, auth::SessionSecret};
	use model::Id;
	use std::sync::Arc;
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};
	#[tokio::test]
	async fn friend_invites_scope_dm_and_confirm_message_without_retry() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_INVITE_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		for (recipient, nonce, status, expected) in [
			("8", "invite-nonce", 200, true),
			("9", "invite-nonce", 200, false),
			("8", "wrong", 200, false),
			("8", "invite-nonce", 500, false),
		] {
			let server = async {
				for step in 0..if recipient == "8" { 2 } else { 1 } {
					let (mut socket, _) = listener.accept().await.unwrap();
					let mut bytes = Vec::new();
					loop {
						let mut chunk = [0; 1024];
						let n = socket.read(&mut chunk).await.unwrap();
						assert!(n > 0);
						bytes.extend_from_slice(&chunk[..n]);
						assert!(bytes.len() <= 4096);
						if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
							let headers = std::str::from_utf8(&bytes[..end]).unwrap();
							let length: usize = headers
								.lines()
								.find_map(|h| {
									h.to_ascii_lowercase()
										.strip_prefix("content-length: ")
										.map(str::to_owned)
								})
								.unwrap()
								.parse()
								.unwrap();
							if bytes.len() < end + 4 + length {
								continue;
							}
							let body: serde_json::Value =
								serde_json::from_slice(&bytes[end + 4..]).unwrap();
							if step == 0 {
								assert!(headers.starts_with("POST /users/@me/channels HTTP/1.1"));
								assert_eq!(body, serde_json::json!({"recipient_id":"8"}));
							} else {
								assert!(headers.starts_with("POST /channels/10/messages HTTP/1.1"));
								assert_eq!(body["content"], "https://discord.gg/safe_link");
								assert_eq!(body["nonce"], "invite-nonce");
								assert_eq!(
									body["allowed_mentions"]["parse"],
									serde_json::json!([])
								);
							}
							break;
						}
					}
					let body=if step==0 {serde_json::json!({"id":"10","type":1,"recipients":[{"id":recipient,"username":"Friend"}]})} else {serde_json::json!({"id":"100","channel_id":"10","author":{"id":"1","username":"Owner"},"content":"https://discord.gg/safe_link","nonce":nonce,"type":0})}.to_string();
					let status = if step == 0 { 200 } else { status };
					socket
						.write_all(
							format!(
								"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
								body.len()
							)
							.as_bytes(),
						)
						.await
						.unwrap();
				}
			};
			let (result, ()) = tokio::join!(
				api.send_server_invite(Id(8), "safe_link", "invite-nonce"),
				server
			);
			assert_eq!(result.is_ok(), expected);
			if !expected {
				assert!(matches!(result, Err(Failure::Ambiguous)));
			}
		}
		assert!(
			api.send_server_invite(Id(0), "safe_link", "x")
				.await
				.is_err()
		);
		assert!(
			api.send_server_invite(Id(8), "../wrong", "x")
				.await
				.is_err()
		);
		assert!(
			tokio::time::timeout(std::time::Duration::from_millis(10), listener.accept())
				.await
				.is_err()
		);
	}
	#[tokio::test]
	async fn server_actions_routes_scope_and_uncertain_writes() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_SERVER_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let invite = Action::CreateInvite {
			guild: Id(2),
			channel: Id(3),
			options: client_core::server_actions::InviteOptions {
				max_age: 3600,
				max_uses: 10,
				temporary: true,
			},
		};
		let notifications = Action::Notifications {
			guild: Id(2),
			options: client_core::server_actions::NotificationOptions {
				level: Some(1),
				suppress_roles: Some(true),
				..Default::default()
			},
		};
		let defaults = Action::Notifications {
			guild: Id(2),
			options: client_core::server_actions::NotificationOptions {
				level: Some(3),
				muted: Some(false),
				..Default::default()
			},
		};
		for (action, status, body, expected) in [
			(
				defaults,
				200,
				r#"{"guild_id":"2"}"#,
				Err(Failure::Ambiguous),
			),
			(
				defaults,
				200,
				r#"{"guild_id":"2","message_notifications":3}"#,
				Err(Failure::Ambiguous),
			),
			(
				defaults,
				200,
				r#"{"guild_id":"2","message_notifications":3,"muted":false}"#,
				Ok(None),
			),
			(
				defaults,
				200,
				r#"{"guild_id":"2","message_notifications":3,"muted":false,"mute_config":{"end_time":"2030-01-01T00:00:00Z"}}"#,
				Err(Failure::Ambiguous),
			),
			(
				notifications,
				200,
				r#"{"guild_id":"2","message_notifications":1,"suppress_roles":true}"#,
				Ok(None),
			),
			(
				notifications,
				200,
				r#"{"guild_id":"8","message_notifications":1,"suppress_roles":true}"#,
				Err(Failure::Ambiguous),
			),
			(
				notifications,
				200,
				r#"{"guild_id":"2","message_notifications":0,"suppress_roles":true}"#,
				Err(Failure::Ambiguous),
			),
			(
				notifications,
				200,
				r#"{"guild_id":"2","message_notifications":1}"#,
				Err(Failure::Ambiguous),
			),
			(notifications, 500, "{}", Err(Failure::Ambiguous)),
			(
				notifications,
				429,
				r#"{"retry_after":0.01}"#,
				Err(Failure::RateLimited),
			),
			(
				invite,
				200,
				r#"{"code":"safe_1","guild":{"id":"2"},"channel":{"id":"3"},"max_age":3600,"max_uses":10,"temporary":true}"#,
				Ok(Some("safe_1".to_owned())),
			),
			(
				invite,
				200,
				r#"{"code":"safe_1","guild":{"id":"8"},"channel":{"id":"3"},"max_age":3600,"max_uses":10,"temporary":true}"#,
				Err(Failure::Ambiguous),
			),
			(
				invite,
				200,
				r#"{"code":"../bad","guild":{"id":"2"},"channel":{"id":"3"},"max_age":3600,"max_uses":10,"temporary":true}"#,
				Err(Failure::Ambiguous),
			),
			(invite, 403, "{}", Err(Failure::Forbidden)),
			(invite, 500, "{}", Err(Failure::Ambiguous)),
			(Action::Leave(Id(2)), 204, "", Ok(None)),
			(Action::Delete(Id(2)), 204, "", Ok(None)),
			(Action::Leave(Id(2)), 200, "{}", Err(Failure::Ambiguous)),
			(
				Action::Leave(Id(2)),
				429,
				r#"{"retry_after":0.01}"#,
				Err(Failure::RateLimited),
			),
		] {
			let server = async {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut bytes = Vec::new();
				loop {
					let mut chunk = [0; 1024];
					let n = socket.read(&mut chunk).await.unwrap();
					assert!(n > 0);
					bytes.extend_from_slice(&chunk[..n]);
					assert!(bytes.len() <= 4096);
					if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
						let headers = std::str::from_utf8(&bytes[..end]).unwrap();
						let length: usize = headers
							.lines()
							.find_map(|h| {
								h.to_ascii_lowercase()
									.strip_prefix("content-length: ")
									.map(str::to_owned)
							})
							.map_or(0, |n| n.parse().unwrap());
						if bytes.len() < end + 4 + length {
							continue;
						}
						assert!(headers.contains("SYNTHETIC_SERVER_TOKEN"));
						match action {
							Action::Notifications { options, .. } => {
								assert!(
									headers
										.starts_with("PATCH /users/@me/guilds/2/settings HTTP/1.1")
								);
								assert_eq!(
									serde_json::from_slice::<serde_json::Value>(&bytes[end + 4..])
										.unwrap(),
									if options.muted.is_some() {
										serde_json::json!({"message_notifications":3,"muted":false,"mute_config":{"end_time":null,"selected_time_window":-1}})
									} else {
										serde_json::json!({"message_notifications":1,"suppress_roles":true})
									}
								);
							}
							Action::CreateInvite { .. } => {
								assert!(headers.starts_with("POST /channels/3/invites HTTP/1.1"));
								assert_eq!(
									serde_json::from_slice::<serde_json::Value>(&bytes[end + 4..])
										.unwrap(),
									serde_json::json!({"max_age":3600,"max_uses":10,"temporary":true,"unique":true})
								);
							}
							Action::Leave(_) | Action::Delete(_) => {
								let path = if matches!(action, Action::Delete(_)) {
									"DELETE /guilds/2 HTTP/1.1"
								} else {
									"DELETE /users/@me/guilds/2 HTTP/1.1"
								};
								assert!(headers.starts_with(path));
								assert_eq!(length, 0);
							}
						}
						break;
					}
				}
				socket
					.write_all(
						format!(
							"HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
							body.len()
						)
						.as_bytes(),
					)
					.await
					.unwrap();
			};
			let (event, ()) = tokio::join!(
				api.execute(Command::ServerAction { action, request: 1 }),
				server
			);
			let Event::ServerAction(client_core::server_actions::Event::Written { result, .. }) =
				event
			else {
				panic!("wrong response");
			};
			assert_eq!(result, expected);
		}
		assert_eq!(
			api.server_action(Action::Leave(Id(0))).await,
			Err(Failure::Protocol)
		);
		assert!(
			tokio::time::timeout(std::time::Duration::from_millis(10), listener.accept())
				.await
				.is_err(),
			"writes must never retry automatically"
		);
	}
}
