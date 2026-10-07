use crate::{DiscordApi, Failure};
use client_core::server_admin::Event;
use discord_protocol::server_admin::{self as wire, MAX_WIRE};
use model::{
	Id,
	server_admin::{Action, Result as Outcome},
};
use reqwest::Method;
use serde_json::json;

impl DiscordApi {
	pub(super) async fn server_admin(&self, guild: Id, request: u64, action: &Action) -> Event {
		let result = self.server_admin_action(guild, action).await;
		Event {
			guild,
			request,
			result,
		}
	}
	pub(super) async fn admin_metadata(&self, guild: Id) -> Result<wire::GuildMetadata, Failure> {
		let bytes = self
			.request_limited(Method::GET, &format!("/guilds/{guild}"), None, MAX_WIRE)
			.await?;
		wire::guild_metadata(&bytes, guild).map_err(|_| Failure::Protocol)
	}
	async fn admin_emojis(&self, guild: Id) -> Result<Outcome, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/emojis"),
				None,
				MAX_WIRE,
			)
			.await?;
		wire::emojis(&bytes)
			.map(Outcome::Emojis)
			.map_err(|_| Failure::Protocol)
	}
	async fn admin_stickers(&self, guild: Id) -> Result<Outcome, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/stickers"),
				None,
				MAX_WIRE,
			)
			.await?;
		wire::stickers(&bytes, guild)
			.map(Outcome::Stickers)
			.map_err(|_| Failure::Protocol)
	}
	/// The server's sounds plus its slot limit; missing boost metadata only hides the limit.
	async fn admin_sounds(&self, guild: Id) -> Result<Outcome, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/soundboard-sounds"),
				None,
				client_core::soundboard::MAX_WIRE_BYTES,
			)
			.await?;
		let mut page = discord_protocol::soundboard::admin_sounds(&bytes, guild)
			.map_err(|_| Failure::Protocol)?;
		match self.admin_metadata(guild).await {
			Ok(metadata) => {
				let tier = metadata.premium_tier;
				let features = metadata.checked_roles().map(|(_, features)| features);
				page.limit = features
					.ok()
					.map(|features| discord_protocol::soundboard::sound_limit(tier, &features));
			}
			Err(failure) if failure.ends_session() => return Err(failure),
			Err(_) => {}
		}
		Ok(Outcome::Sounds(page))
	}
	async fn admin_member(
		&self,
		guild: Id,
		user: Id,
	) -> Result<model::server_admin::Member, Failure> {
		let bytes = self
			.request_limited(
				Method::GET,
				&format!("/guilds/{guild}/members/{user}"),
				None,
				64 * 1024,
			)
			.await?;
		wire::member(&bytes, user).map_err(|_| Failure::Protocol)
	}
	async fn server_admin_action(&self, guild: Id, action: &Action) -> Result<Outcome, Failure> {
		if guild.0 == 0 || !action.valid() {
			return Err(Failure::Protocol);
		}
		match action {
			Action::AuditLog(query) => self
				.server_audit_log(guild, query)
				.await
				.map(Outcome::AuditLog),
			Action::Integrations(model::server_integrations::Action::CopyWebhookUrl {
				webhook,
				channel,
				..
			}) => self
				.copy_webhook_url(guild, *webhook, *channel)
				.await
				.map(Outcome::WebhookUrl),
			Action::Integrations(action) => self
				.server_integration_action(guild, action)
				.await
				.map(Outcome::Integrations),
			Action::Invites(action) => self
				.server_invite_action(guild, action)
				.await
				.map(Outcome::Invites),
			Action::Roles(action) => self
				.server_role_action(guild, action)
				.await
				.map(Outcome::Roles),
			Action::LoadEmojis => self.admin_emojis(guild).await,
			Action::CreateEmoji { name, image } => {
				if !wire::valid_emoji_data_uri(image) {
					return Err(Failure::Protocol);
				}
				let bytes = self
					.request_limited(
						Method::POST,
						&format!("/guilds/{guild}/emojis"),
						Some(json!({"name":name,"image":image})),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let created = wire::emoji(&bytes).map_err(|_| Failure::Ambiguous)?;
				if created.emoji.name != *name {
					return Err(Failure::Ambiguous);
				}
				self.admin_emojis(guild).await.map_err(reconcile_failure)
			}
			Action::RenameEmoji { id, name } => {
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/guilds/{guild}/emojis/{id}"),
						Some(json!({"name":name})),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let renamed = wire::emoji(&bytes).map_err(|_| Failure::Ambiguous)?;
				if renamed.emoji.id != *id || renamed.emoji.name != *name {
					return Err(Failure::Ambiguous);
				}
				self.admin_emojis(guild).await.map_err(reconcile_failure)
			}
			Action::DeleteEmoji { id } => {
				let bytes = self
					.request_limited(
						Method::DELETE,
						&format!("/guilds/{guild}/emojis/{id}"),
						None,
						4096,
					)
					.await
					.map_err(write_failure)?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
				self.admin_emojis(guild).await.map_err(reconcile_failure)
			}
			Action::LoadStickers => self.admin_stickers(guild).await,
			Action::CreateSticker {
				name,
				description,
				tags,
				filename,
				content_type,
				file,
			} => {
				if !wire::valid_sticker_file(filename, content_type, file) {
					return Err(Failure::Protocol);
				}
				let (content_type_header, body) =
					sticker_multipart(name, description, tags, filename, content_type, file)?;
				let bytes = self
					.request_multipart_limited(
						&format!("/guilds/{guild}/stickers"),
						content_type_header,
						body,
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let created = wire::sticker(&bytes, guild).map_err(|_| Failure::Ambiguous)?;
				if created.sticker.name != *name
					|| created.sticker.description != *description
					|| created.sticker.tags != *tags
				{
					return Err(Failure::Ambiguous);
				}
				self.admin_stickers(guild).await.map_err(reconcile_failure)
			}
			Action::EditSticker {
				id,
				name,
				description,
				tags,
			} => {
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/guilds/{guild}/stickers/{id}"),
						Some(json!({"name":name,"description":description,"tags":tags})),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let edited = wire::sticker(&bytes, guild).map_err(|_| Failure::Ambiguous)?;
				if edited.sticker.id != *id
					|| edited.sticker.name != *name
					|| edited.sticker.description != *description
					|| edited.sticker.tags != *tags
				{
					return Err(Failure::Ambiguous);
				}
				self.admin_stickers(guild).await.map_err(reconcile_failure)
			}
			Action::DeleteSticker { id } => {
				let bytes = self
					.request_limited(
						Method::DELETE,
						&format!("/guilds/{guild}/stickers/{id}"),
						None,
						4096,
					)
					.await
					.map_err(write_failure)?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
				self.admin_stickers(guild).await.map_err(reconcile_failure)
			}
			Action::LoadSounds => self.admin_sounds(guild).await,
			Action::CreateSound {
				name,
				emoji,
				volume,
				content_type,
				file,
			} => {
				if !discord_protocol::soundboard::valid_sound_file(content_type, file) {
					return Err(Failure::Protocol);
				}
				let mut body = json!({
					"name": name,
					"sound": discord_protocol::soundboard::sound_data_uri(content_type, file),
					"volume": f64::from(*volume) / 100.0,
				});
				if !emoji.is_empty() {
					body["emoji_name"] = json!(emoji);
				}
				let bytes = self
					.request_limited(
						Method::POST,
						&format!("/guilds/{guild}/soundboard-sounds"),
						Some(body),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let created = discord_protocol::soundboard::admin_sound(&bytes, guild)
					.map_err(|_| Failure::Ambiguous)?;
				if created.name != *name {
					return Err(Failure::Ambiguous);
				}
				self.admin_sounds(guild).await.map_err(reconcile_failure)
			}
			Action::EditSound {
				id,
				name,
				emoji,
				volume,
			} => {
				let mut body = json!({"name": name, "volume": f64::from(*volume) / 100.0});
				match emoji {
					model::Patch::Absent => {}
					model::Patch::Null => {
						body["emoji_id"] = serde_json::Value::Null;
						body["emoji_name"] = serde_json::Value::Null;
					}
					model::Patch::Value(emoji) => {
						body["emoji_id"] = serde_json::Value::Null;
						body["emoji_name"] = json!(emoji);
					}
				}
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/guilds/{guild}/soundboard-sounds/{id}"),
						Some(body),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let edited = discord_protocol::soundboard::admin_sound(&bytes, guild)
					.map_err(|_| Failure::Ambiguous)?;
				if edited.id != *id || edited.name != *name {
					return Err(Failure::Ambiguous);
				}
				self.admin_sounds(guild).await.map_err(reconcile_failure)
			}
			Action::DeleteSound { id } => {
				let bytes = self
					.request_limited(
						Method::DELETE,
						&format!("/guilds/{guild}/soundboard-sounds/{id}"),
						None,
						4096,
					)
					.await
					.map_err(write_failure)?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
				self.admin_sounds(guild).await.map_err(reconcile_failure)
			}
			Action::LoadMembers(query) => {
				let now = std::time::SystemTime::now()
					.duration_since(std::time::UNIX_EPOCH)
					.map_or(0, |value| value.as_millis().min(i64::MAX as u128) as i64);
				let body = wire::query(query, now).map_err(|_| Failure::Protocol)?;
				// This POST is an on-demand search, not a write and not the bot-only GET member route.
				let bytes = self
					.request_limited(
						Method::POST,
						&format!("/guilds/{guild}/members-search"),
						Some(body),
						MAX_WIRE,
					)
					.await?;
				let metadata = self.admin_metadata(guild).await?;
				wire::members(&bytes, guild, metadata)
					.map(Outcome::Members)
					.map_err(|_| {
						Failure::ProtocolAt(
							"Member search is unavailable or still indexing; reload later",
						)
					})
			}
			Action::SetRole {
				user,
				role,
				assigned,
			} => {
				let bytes = self
					.request_limited(
						if *assigned {
							Method::PUT
						} else {
							Method::DELETE
						},
						&format!("/guilds/{guild}/members/{user}/roles/{role}"),
						None,
						4096,
					)
					.await
					.map_err(write_failure)?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
				let member = self
					.admin_member(guild, *user)
					.await
					.map_err(reconcile_failure)?;
				if member.roles.contains(role) != *assigned {
					return Err(Failure::Ambiguous);
				}
				Ok(Outcome::Member(member))
			}
			Action::SetNickname { user, nick } => {
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/guilds/{guild}/members/{user}"),
						Some(json!({"nick":if nick.is_empty() { None } else { Some(nick) }})),
						64 * 1024,
					)
					.await
					.map_err(write_failure)?;
				let member = wire::member(&bytes, *user).map_err(|_| Failure::Ambiguous)?;
				if member.nick.as_deref().unwrap_or("") != nick {
					return Err(Failure::Ambiguous);
				}
				Ok(Outcome::Member(member))
			}
			Action::Kick { user } => {
				let bytes = self
					.request_limited(
						Method::DELETE,
						&format!("/guilds/{guild}/members/{user}"),
						None,
						4096,
					)
					.await
					.map_err(write_failure)?;
				if !bytes.is_empty() {
					return Err(Failure::Ambiguous);
				}
				Ok(Outcome::Kicked(*user))
			}
			Action::Prune { days, execute } => {
				let (method, path, body) = if *execute {
					(
						Method::POST,
						format!("/guilds/{guild}/prune"),
						Some(json!({"days":days,"compute_prune_count":false})),
					)
				} else {
					(
						Method::GET,
						format!("/guilds/{guild}/prune?days={days}"),
						None,
					)
				};
				let bytes = self
					.request_limited(method, &path, body, 4096)
					.await
					.map_err(|failure| {
						if *execute {
							write_failure(failure)
						} else {
							failure
						}
					})?;
				let count = wire::pruned(&bytes).map_err(|_| {
					if *execute {
						Failure::Ambiguous
					} else {
						Failure::Protocol
					}
				})?;
				if !execute && count.is_none() {
					return Err(Failure::Protocol);
				}
				Ok(Outcome::Pruned(count))
			}
			Action::ShowMembers { enabled } => {
				let (_, mut features) = self
					.admin_metadata(guild)
					.await?
					.checked_roles()
					.map_err(|_| Failure::Protocol)?;
				if features.iter().any(|value| value == "COMMUNITY") {
					return Err(Failure::Forbidden);
				}
				features.retain(|value| value != model::server_admin::MEMBER_CHANNEL_FEATURE);
				if *enabled {
					features.push(model::server_admin::MEMBER_CHANNEL_FEATURE.into());
				}
				let bytes = self
					.request_limited(
						Method::PATCH,
						&format!("/guilds/{guild}"),
						Some(json!({"features":features})),
						MAX_WIRE,
					)
					.await
					.map_err(write_failure)?;
				let (_, saved) = wire::guild_metadata(&bytes, guild)
					.and_then(wire::GuildMetadata::checked_roles)
					.map_err(|_| Failure::Ambiguous)?;
				if saved
					.iter()
					.any(|value| value == model::server_admin::MEMBER_CHANNEL_FEATURE)
					!= *enabled
				{
					return Err(Failure::Ambiguous);
				}
				Ok(Outcome::ChannelList(*enabled))
			}
		}
	}
}
fn sticker_multipart(
	name: &str,
	description: &str,
	tags: &str,
	filename: &str,
	file_content_type: &str,
	file: &[u8],
) -> Result<(String, Vec<u8>), Failure> {
	let mut suffix = 0u32;
	let boundary = loop {
		let candidate = format!("----------------serein-sticker-{suffix:x}");
		if !file
			.windows(candidate.len())
			.any(|window| window == candidate.as_bytes())
			&& [name, description, tags]
				.into_iter()
				.all(|value| !value.contains(&candidate))
		{
			break candidate;
		}
		suffix = suffix.checked_add(1).ok_or(Failure::Protocol)?;
	};
	let mut body = Vec::with_capacity(file.len().saturating_add(2048));
	for (field, value) in [("name", name), ("description", description), ("tags", tags)] {
		body.extend_from_slice(
			format!(
				"--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n"
			)
			.as_bytes(),
		);
	}
	body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {file_content_type}\r\n\r\n").as_bytes());
	body.extend_from_slice(file);
	body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
	if body.len() > model::server_admin::MAX_STICKER_FILE_BYTES + 4096 {
		return Err(Failure::Capacity);
	}
	Ok((format!("multipart/form-data; boundary={boundary}"), body))
}
fn write_failure(failure: Failure) -> Failure {
	if failure == Failure::Capacity {
		Failure::Ambiguous
	} else {
		failure
	}
}
fn reconcile_failure(failure: Failure) -> Failure {
	if failure.ends_session() {
		failure
	} else {
		Failure::Ambiguous
	}
}

#[cfg(test)]
mod sticker_tests {
	use super::*;

	#[test]
	fn sticker_multipart_keeps_fields_file_and_collision_free_boundary() {
		let file = b"----------------serein-sticker-0 image";
		let (content_type, body) = sticker_multipart(
			"Wave",
			"A friendly wave",
			"wave",
			"wave.png",
			"image/png",
			file,
		)
		.unwrap();
		assert!(content_type.ends_with("serein-sticker-1"));
		let body = String::from_utf8_lossy(&body);
		for expected in [
			"name=\"name\"\r\n\r\nWave",
			"name=\"description\"\r\n\r\nA friendly wave",
			"name=\"tags\"\r\n\r\nwave",
			"name=\"file\"; filename=\"wave.png\"",
			"Content-Type: image/png",
		] {
			assert!(body.contains(expected));
		}
		assert!(body.ends_with("--\r\n"));
	}
}

#[cfg(test)]
mod sound_tests {
	use super::*;
	use std::{sync::Arc, time::Duration};
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};

	/// Serve one scripted exchange per connection, checking each request line and JSON body.
	async fn serve(
		listener: TcpListener,
		script: Vec<(&'static str, Option<serde_json::Value>, &'static str)>,
	) {
		for (line, expected, reply) in script {
			let (mut socket, _) = listener.accept().await.unwrap();
			let mut request = Vec::new();
			loop {
				let mut bytes = [0; 4096];
				let n = socket.read(&mut bytes).await.unwrap();
				assert!(n > 0);
				request.extend_from_slice(&bytes[..n]);
				assert!(request.len() < 64 * 1024);
				let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
					continue;
				};
				let headers = String::from_utf8_lossy(&request[..end]).into_owned();
				assert!(headers.starts_with(line), "{headers}");
				let Some(expected) = &expected else {
					break;
				};
				let length: usize = headers
					.lines()
					.find_map(|line| {
						line.to_ascii_lowercase()
							.strip_prefix("content-length: ")
							.map(str::to_owned)
					})
					.unwrap()
					.parse()
					.unwrap();
				if request.len() >= end + 4 + length {
					let body: serde_json::Value =
						serde_json::from_slice(&request[end + 4..]).unwrap();
					assert_eq!(&body, expected);
					break;
				}
			}
			let status = if reply.is_empty() {
				"204 No Content"
			} else {
				"200 OK"
			};
			socket
				.write_all(
					format!(
						"HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
						reply.len()
					)
					.as_bytes(),
				)
				.await
				.unwrap();
		}
	}
	const LIST: &str = r#"{"items":[{"name":"Air horn","sound_id":"30","volume":0.8,"emoji_name":"x","guild_id":"9","user":{"id":"5","username":"Uploader"}}]}"#;
	const GUILD: &str =
		r#"{"id":"9","owner_id":"5","roles":[],"features":["COMMUNITY"],"premium_tier":1}"#;

	#[tokio::test]
	async fn sound_writes_use_documented_routes_and_reload_the_catalog_with_its_slot_limit() {
		tokio::time::timeout(Duration::from_secs(5), async {
			let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
			let mut api = DiscordApi::new(Arc::new(
				crate::SessionSecret::from_owner_input("SYNTHETIC_SOUND_ADMIN_TOKEN".into())
					.unwrap(),
			))
			.unwrap();
			api.base = format!("http://{}", listener.local_addr().unwrap());
			let server = tokio::spawn(serve(
				listener,
				vec![
					(
						"POST /guilds/9/soundboard-sounds HTTP/1.1",
						Some(json!({
							"name": "Air horn",
							"sound": "data:audio/ogg;base64,T2dnUw==",
							"volume": 0.8,
							"emoji_name": "x",
						})),
						r#"{"name":"Air horn","sound_id":"30","volume":0.8,"guild_id":"9"}"#,
					),
					("GET /guilds/9/soundboard-sounds HTTP/1.1", None, LIST),
					("GET /guilds/9 HTTP/1.1", None, GUILD),
					(
						"PATCH /guilds/9/soundboard-sounds/30 HTTP/1.1",
						Some(json!({
							"name": "Air horn",
							"volume": 0.5,
							"emoji_id": null,
							"emoji_name": null,
						})),
						r#"{"name":"Air horn","sound_id":"30","volume":0.5,"guild_id":"9"}"#,
					),
					("GET /guilds/9/soundboard-sounds HTTP/1.1", None, LIST),
					// Missing boost metadata only hides the slot limit.
					("GET /guilds/9 HTTP/1.1", None, r#"{"id":"8"}"#),
					("DELETE /guilds/9/soundboard-sounds/30 HTTP/1.1", None, ""),
					(
						"GET /guilds/9/soundboard-sounds HTTP/1.1",
						None,
						r#"{"items":[]}"#,
					),
					("GET /guilds/9 HTTP/1.1", None, GUILD),
				],
			));
			let created = api
				.server_admin_action(
					Id(9),
					&Action::CreateSound {
						name: "Air horn".into(),
						emoji: "x".into(),
						volume: 80,
						content_type: "audio/ogg".into(),
						file: b"OggS".to_vec(),
					},
				)
				.await;
			let Ok(Outcome::Sounds(page)) = created else {
				panic!("created sound must reload the catalog");
			};
			assert_eq!(page.limit, Some(24));
			assert_eq!(page.items[0].sound.id, Id(30));
			assert_eq!(page.items[0].uploader.as_ref().unwrap().id, Id(5));

			let edited = api
				.server_admin_action(
					Id(9),
					&Action::EditSound {
						id: Id(30),
						name: "Air horn".into(),
						emoji: model::Patch::Null,
						volume: 50,
					},
				)
				.await;
			assert!(matches!(edited, Ok(Outcome::Sounds(page)) if page.limit.is_none()));

			let deleted = api
				.server_admin_action(Id(9), &Action::DeleteSound { id: Id(30) })
				.await;
			assert!(
				matches!(deleted, Ok(Outcome::Sounds(page)) if page.items.is_empty() && page.limit == Some(24))
			);
			server.await.unwrap();

			// A file that is not the declared container never reaches the network.
			assert!(matches!(
				api.server_admin_action(
					Id(9),
					&Action::CreateSound {
						name: "Air horn".into(),
						emoji: String::new(),
						volume: 80,
						content_type: "audio/mpeg".into(),
						file: b"OggS".to_vec(),
					},
				)
				.await,
				Err(Failure::Protocol)
			));
		})
		.await
		.unwrap();
	}
}
