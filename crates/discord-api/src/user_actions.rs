use crate::{DiscordApi, Failure};
use client_core::user_actions::Action;
use reqwest::Method;
use serde_json::json;

impl DiscordApi {
	pub(super) async fn open_dm(&self, user: model::Id) -> Result<model::Channel, Failure> {
		if user.0 == 0 {
			return Err(Failure::Protocol);
		}
		let bytes = self
			.request_limited(
				Method::POST,
				"/users/@me/channels",
				Some(json!({"recipient_id": user})),
				64 * 1024,
			)
			.await
			.map_err(|failure| {
				if failure == Failure::Capacity {
					Failure::Ambiguous
				} else {
					failure
				}
			})?;
		let channel: discord_protocol::ChannelDto =
			discord_protocol::decode(&bytes).map_err(|_| Failure::Ambiguous)?;
		let channel = channel.into_model();
		if channel.id.0 == 0
			|| channel.guild.is_some()
			|| channel.kind != 1
			|| channel.recipients.len() != 1
			|| channel.recipients[0].id != user
		{
			return Err(Failure::Ambiguous);
		}
		Ok(channel)
	}
	/// Reads one user's private note, treating a missing note as empty.
	pub(super) async fn user_note(&self, user: model::Id) -> Result<String, Failure> {
		if user.0 == 0 {
			return Err(Failure::Protocol);
		}
		#[derive(serde::Deserialize)]
		struct Note {
			note: Option<String>,
		}
		let bytes = self
			.request_limited(Method::GET, &format!("/users/@me/notes/{user}"), None, 2048)
			.await?;
		let note: Note = discord_protocol::decode(&bytes).map_err(|_| Failure::Protocol)?;
		let text = note.note.unwrap_or_default();
		if !client_core::user_actions::valid_personal_text(&text, false) {
			return Err(Failure::Protocol);
		}
		Ok(text)
	}
	/// Runs one account write, using the captcha slot for friendship actions.
	pub(super) async fn user_action(
		&self,
		action: &Action,
		captcha: Option<&client_core::captcha::Retry>,
		challenge: Option<&mut Option<client_core::captcha::Challenge>>,
	) -> Result<(), Failure> {
		// Unofficial normal-user routes: discord.py-self/http.py, checked 2026-09-12.
		// A solved challenge may only resume the friendship write it was issued for.
		if client_core::user_actions::establishes_friendship(action) {
			let target =
				client_core::user_actions::challenge_target(action).ok_or(Failure::Protocol)?;
			if captcha.is_some_and(|retry| !retry.matches_target(&target)) {
				return Err(Failure::Protocol);
			}
		} else if captcha.is_some() {
			return Err(Failure::Protocol);
		}
		if let Action::AddFriend { username } = action {
			if !client_core::user_actions::valid_username(username) {
				return Err(Failure::Protocol);
			}
			return self
				.request_with_captcha(
					Method::POST,
					"/users/@me/relationships",
					Some(json!({"username":username,"discriminator":null})),
					crate::MAX_WIRE,
					captcha,
					challenge,
				)
				.await
				.map(|_| ())
				.map_err(|f| {
					f.protocol_at(
						"Friend request rejected · check the username and recipient's privacy settings",
					)
				});
		}
		let id = match action {
			Action::LoadNote(id)
			| Action::Note { user: id, .. }
			| Action::Nickname { user: id, .. } => id,
			Action::AddFriend { .. } => unreachable!(),
			Action::ResolveFriend { user, .. } | Action::ProfileFriend { user, .. } => user,
			Action::OpenDm(id)
			| Action::CloseDm(id)
			| Action::Block { user: id, .. }
			| Action::Mute { channel: id, .. } => id,
		};
		if id.0 == 0 {
			return Err(Failure::Protocol);
		}
		match action {
			Action::LoadNote(_) | Action::OpenDm(_) => Err(Failure::Protocol),
			Action::Note { user, text } | Action::Nickname { user, text } => {
				let nickname = matches!(action, Action::Nickname { .. });
				if !client_core::user_actions::valid_personal_text(text, nickname) {
					return Err(Failure::Protocol);
				}
				let (method, path, body) = if nickname {
					(
						Method::PATCH,
						format!("/users/@me/relationships/{user}"),
						json!({"nickname": if text.is_empty() { None } else { Some(text) }}),
					)
				} else {
					(
						Method::PUT,
						format!("/users/@me/notes/{user}"),
						json!({"note": text}),
					)
				};
				self.request(method, &path, Some(body)).await.map(|_| ())
			}
			Action::AddFriend { .. } => unreachable!(),
			Action::ResolveFriend { user, accept } => {
				let (method, body) = if *accept {
					(Method::PUT, Some(json!({})))
				} else {
					(Method::DELETE, None)
				};
				let path = format!("/users/@me/relationships/{user}");
				if *accept {
					self.request_with_captcha(
						method,
						&path,
						body,
						crate::MAX_WIRE,
						captcha,
						challenge,
					)
					.await
					.map(|_| ())
				} else {
					self.request(method, &path, body).await.map(|_| ())
				}
			}
			Action::ProfileFriend { user, friend } => {
				let (method, body) = if *friend {
					(Method::PUT, Some(json!({})))
				} else {
					(Method::DELETE, None)
				};
				let path = format!("/users/@me/relationships/{user}");
				if *friend {
					self.request_with_captcha(
						method,
						&path,
						body,
						crate::MAX_WIRE,
						captcha,
						challenge,
					)
					.await
					.map(|_| ())
					.map_err(|failure| {
						failure.protocol_at(
							"Friend request rejected · check the recipient's privacy settings",
						)
					})
				} else {
					self.request(method, &path, body).await.map(|_| ())
				}
			}
			Action::CloseDm(channel) => self
				.request(Method::DELETE, &format!("/channels/{channel}"), None)
				.await
				.map(|_| ()),
			Action::Block { user, blocked } => self
				.request(
					if *blocked {
						Method::PUT
					} else {
						Method::DELETE
					},
					&format!("/users/@me/relationships/{user}"),
					blocked.then(|| json!({"type": 2})),
				)
				.await
				.map(|_| ()),
			Action::Mute { channel, muted } => {
				let bytes = self.request(Method::PATCH, "/users/@me/guilds/@me/settings",
					Some(json!({"channel_overrides": {channel.to_string(): {"muted": muted, "mute_config": {"end_time": null, "selected_time_window": -1}}}}))).await?;
				let setting: discord_protocol::notifications::Setting =
					discord_protocol::decode(&bytes).map_err(|_| Failure::Protocol)?;
				if setting.guild_id.is_some()
					|| !setting.channel_overrides.is_some_and(|o| {
						o.0.iter()
							.any(|c| c.channel_id == *channel && c.muted == Some(*muted))
					}) {
					return Err(Failure::ProtocolAt(
						"DM mute response did not confirm the requested setting",
					));
				}
				Ok(())
			}
		}
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
	/// Regression: note reads handle empty, missing and forbidden responses without writes.
	#[tokio::test]
	async fn notes_read_empty_missing_existing_and_forbidden_without_writes() {
		for (status, body, expected) in [
			(
				200,
				r#"{"note":"Synthetic note"}"#,
				Ok("Synthetic note".to_owned()),
			),
			(404, r#"{"code":10013}"#, Ok(String::new())),
			(403, "{}", Err(Failure::Forbidden)),
		] {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				SessionSecret::from_owner_input("SYNTHETIC_NOTE_TOKEN".into()).unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = async {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut bytes = Vec::new();
				while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
					let mut chunk = [0; 1024];
					let count = socket.read(&mut chunk).await.unwrap();
					assert!(count > 0 && bytes.len() + count <= 4096);
					bytes.extend_from_slice(&chunk[..count]);
				}
				assert!(bytes.starts_with(b"GET /users/@me/notes/2 HTTP/1.1\r\n"));
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
				api.execute(Command::UserAction {
					action: Action::LoadNote(Id(2)),
					request: 7,
					captcha: None,
				}),
				server
			);
			let Event::UserAction(client_core::user_actions::Event::NoteLoaded {
				user,
				request,
				result,
			}) = event
			else {
				panic!()
			};
			assert_eq!((user, request, result), (Id(2), 7, expected));
		}
	}
	/// Regression: account and friend actions use scoped routes and confirm outcomes.
	#[tokio::test]
	async fn account_and_friend_request_actions_use_scoped_routes_and_confirm_remote_outcomes() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_USER_ACTION_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let cases = [
			(
				Action::Note {
					user: Id(2),
					text: "Remember 🌙".into(),
				},
				"PUT /users/@me/notes/2",
				Some(json!({"note":"Remember 🌙"})),
				204,
				"",
				Ok(()),
			),
			(
				Action::Note {
					user: Id(2),
					text: String::new(),
				},
				"PUT /users/@me/notes/2",
				Some(json!({"note":""})),
				204,
				"",
				Ok(()),
			),
			(
				Action::Nickname {
					user: Id(2),
					text: "Bestie".into(),
				},
				"PATCH /users/@me/relationships/2",
				Some(json!({"nickname":"Bestie"})),
				204,
				"",
				Ok(()),
			),
			(
				Action::Nickname {
					user: Id(2),
					text: String::new(),
				},
				"PATCH /users/@me/relationships/2",
				Some(json!({"nickname":null})),
				204,
				"",
				Ok(()),
			),
			(
				Action::AddFriend {
					username: "synthetic_friend".into(),
				},
				"POST /users/@me/relationships",
				Some(json!({"username":"synthetic_friend","discriminator":null})),
				204,
				"",
				Ok(()),
			),
			(
				Action::ResolveFriend {
					user: Id(2),
					accept: true,
				},
				"PUT /users/@me/relationships/2",
				Some(json!({})),
				204,
				"",
				Ok(()),
			),
			(
				Action::ResolveFriend {
					user: Id(2),
					accept: false,
				},
				"DELETE /users/@me/relationships/2",
				None,
				204,
				"",
				Ok(()),
			),
			(
				Action::ProfileFriend {
					user: Id(2),
					friend: true,
				},
				"PUT /users/@me/relationships/2",
				Some(json!({})),
				204,
				"",
				Ok(()),
			),
			(
				Action::ProfileFriend {
					user: Id(2),
					friend: false,
				},
				"DELETE /users/@me/relationships/2",
				None,
				204,
				"",
				Ok(()),
			),
			(
				Action::ProfileFriend {
					user: Id(2),
					friend: true,
				},
				"PUT /users/@me/relationships/2",
				Some(json!({})),
				400,
				"{}",
				Err(Failure::ProtocolAt(
					"Friend request rejected · check the recipient's privacy settings",
				)),
			),
			(
				Action::CloseDm(Id(10)),
				"DELETE /channels/10",
				None,
				204,
				"",
				Ok(()),
			),
			(
				Action::Block {
					user: Id(2),
					blocked: true,
				},
				"PUT /users/@me/relationships/2",
				Some(json!({"type":2})),
				204,
				"",
				Ok(()),
			),
			(
				// A write with no challenge solver keeps the session and reports locally.
				Action::Block {
					user: Id(2),
					blocked: true,
				},
				"PUT /users/@me/relationships/2",
				Some(json!({"type":2})),
				400,
				r#"{"captcha_key":["required"],"captcha_service":"hcaptcha","captcha_sitekey":"synthetic-sitekey"}"#,
				Err(Failure::ProtocolAt(
					"Discord requires verification for this action; complete it in the official client",
				)),
			),
			(
				Action::Block {
					user: Id(2),
					blocked: false,
				},
				"DELETE /users/@me/relationships/2",
				None,
				204,
				"",
				Ok(()),
			),
			(
				Action::Mute {
					channel: Id(10),
					muted: true,
				},
				"PATCH /users/@me/guilds/@me/settings",
				Some(
					json!({"channel_overrides":{"10":{"muted":true,"mute_config":{"end_time":null,"selected_time_window":-1}}}}),
				),
				200,
				r#"{"guild_id":null,"channel_overrides":[{"channel_id":"10","muted":true}]}"#,
				Ok(()),
			),
			(
				Action::Mute {
					channel: Id(10),
					muted: false,
				},
				"PATCH /users/@me/guilds/@me/settings",
				Some(
					json!({"channel_overrides":{"10":{"muted":false,"mute_config":{"end_time":null,"selected_time_window":-1}}}}),
				),
				200,
				r#"{"guild_id":null,"channel_overrides":[{"channel_id":"11","muted":false}]}"#,
				Err(Failure::ProtocolAt(
					"DM mute response did not confirm the requested setting",
				)),
			),
			(
				Action::CloseDm(Id(10)),
				"DELETE /channels/10",
				None,
				403,
				"{}",
				Err(Failure::Forbidden),
			),
			(
				Action::Block {
					user: Id(2),
					blocked: true,
				},
				"PUT /users/@me/relationships/2",
				Some(json!({"type":2})),
				500,
				"{}",
				Err(Failure::Ambiguous),
			),
			(
				Action::Block {
					user: Id(2),
					blocked: true,
				},
				"PUT /users/@me/relationships/2",
				Some(json!({"type":2})),
				429,
				r#"{"retry_after":0.01}"#,
				Err(Failure::RateLimited),
			),
		];
		for (action, route, payload, status, body, expected) in cases {
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
						let len: usize = headers
							.lines()
							.find_map(|h| {
								h.to_ascii_lowercase()
									.strip_prefix("content-length: ")
									.map(str::to_owned)
							})
							.map_or(0, |n| n.parse().unwrap());
						if bytes.len() < end + 4 + len {
							continue;
						}
						assert!(headers.starts_with(&format!("{route} HTTP/1.1\r\n")));
						assert!(headers.contains("SYNTHETIC_USER_ACTION_TOKEN"));
						assert_eq!(
							if len == 0 {
								None
							} else {
								Some(
									serde_json::from_slice::<serde_json::Value>(&bytes[end + 4..])
										.unwrap(),
								)
							},
							payload
						);
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
				api.execute(Command::UserAction {
					action,
					request: 1,
					captcha: None,
				}),
				server
			);
			let Event::UserAction(client_core::user_actions::Event::Written { result, .. }) = event
			else {
				panic!("wrong result")
			};
			assert_eq!(result, expected);
		}
		assert_eq!(
			api.user_action(&Action::CloseDm(Id(0)), None, None).await,
			Err(Failure::Protocol)
		);
		assert!(
			tokio::time::timeout(std::time::Duration::from_millis(10), listener.accept())
				.await
				.is_err(),
			"writes must not retry automatically"
		);
	}
	/// Regression: a challenged friend request surfaces and resumes once with the solution.
	#[tokio::test]
	async fn friend_request_captcha_surfaces_and_resumes_once_with_the_solution() {
		// Discord can answer a friend request with a per-action captcha. The session must
		// stay usable, the challenge must surface to the user, and the solved token must be
		// replayed on that same write only.
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_FRIEND_CAPTCHA_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let server = tokio::spawn(async move {
			for attempt in 0..2 {
				let (mut socket, _) = listener.accept().await.unwrap();
				let mut bytes = Vec::new();
				while !bytes.windows(4).any(|b| b == b"\r\n\r\n") {
					let mut chunk = [0; 1024];
					let count = socket.read(&mut chunk).await.unwrap();
					assert!(count > 0);
					bytes.extend_from_slice(&chunk[..count]);
				}
				let request = std::str::from_utf8(&bytes).unwrap();
				assert!(request.starts_with("POST /users/@me/relationships HTTP/1.1"));
				assert_eq!(
					request.contains("x-captcha-key: synthetic-solution"),
					attempt == 1
				);
				assert_eq!(
					request.contains("x-captcha-session-id: synthetic-session"),
					attempt == 1
				);
				let (status, body) = if attempt == 0 {
					(
						"400 Bad Request",
						r#"{"code":0,"captcha_key":["required"],"captcha_service":"hcaptcha","captcha_sitekey":"synthetic-sitekey","captcha_session_id":"synthetic-session"}"#,
					)
				} else {
					("204 No Content", "")
				};
				socket
					.write_all(
						format!(
							"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
							body.len()
						)
						.as_bytes(),
					)
					.await
					.unwrap();
			}
		});
		let mut state = client_core::State {
			auth: client_core::auth::AuthState::Authenticated,
			gateway_connected: true,
			..Default::default()
		};
		let first = state.add_friend("synthetic_friend").unwrap();
		let event = api.execute(first).await;
		assert!(matches!(
			event,
			Event::UserAction(client_core::user_actions::Event::Challenge { .. })
		));
		assert!(
			!api.stopped(),
			"an action-level captcha must not stop the session"
		);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event,
		});
		let request = state.friend_challenge().unwrap().0;
		let command = state
			.resume_friend_challenge(
				request,
				client_core::captcha::Solution::new("synthetic-solution".into()).unwrap(),
			)
			.unwrap();
		assert!(matches!(
			api.execute(command).await,
			Event::UserAction(client_core::user_actions::Event::Written { result: Ok(()), .. })
		));
		assert!(!api.stopped());
		server.await.unwrap();
	}
}
