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
