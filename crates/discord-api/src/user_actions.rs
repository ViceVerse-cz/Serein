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
	pub(super) async fn user_action(&self, action: &Action) -> Result<(), Failure> {
		// Unofficial normal-user routes: discord.py-self/http.py, checked 2026-09-12.
		if let Action::AddFriend { username } = action {
			if !client_core::user_actions::valid_username(username) {
				return Err(Failure::Protocol);
			}
			return self
				.request(
					Method::POST,
					"/users/@me/relationships",
					Some(json!({"username":username,"discriminator":null})),
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
			Action::ResolveFriend { user, accept } => self
				.request(
					if *accept { Method::PUT } else { Method::DELETE },
					&format!("/users/@me/relationships/{user}"),
					accept.then(|| json!({})),
				)
				.await
				.map(|_| ()),
			Action::ProfileFriend { user, friend } => self
				.request(
					if *friend { Method::PUT } else { Method::DELETE },
					&format!("/users/@me/relationships/{user}"),
					friend.then(|| json!({})),
				)
				.await
				.map(|_| ())
				.map_err(|failure| {
					if *friend {
						failure.protocol_at(
							"Friend request rejected · check the recipient's privacy settings",
						)
					} else {
						failure
					}
				}),
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
					request: 7
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
				api.execute(Command::UserAction { action, request: 1 }),
				server
			);
			let Event::UserAction(client_core::user_actions::Event::Written { result, .. }) = event
			else {
				panic!("wrong result")
			};
			assert_eq!(result, expected);
		}
		assert_eq!(
			api.user_action(&Action::CloseDm(Id(0))).await,
			Err(Failure::Protocol)
		);
		assert!(
			tokio::time::timeout(std::time::Duration::from_millis(10), listener.accept())
				.await
				.is_err(),
			"writes must not retry automatically"
		);
	}
}
