//! Documented channel administration routes and isolated unofficial user settings writes.
use crate::{DiscordApi, Failure};
use client_core::channel_actions::{Action, Edit, Mute, Outcome, PostDetails};
use model::{Id, permissions};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CHANNEL_BYTES: usize = 64 * 1024;
fn write_failure(failure: Failure) -> Failure {
	if matches!(failure, Failure::Capacity | Failure::Protocol) {
		Failure::Ambiguous
	} else {
		failure
	}
}
fn channel_value(bytes: &[u8], guild: Id, channel: Option<Id>) -> Result<Value, Failure> {
	let value: Value = discord_protocol::decode(bytes).map_err(|_| Failure::Protocol)?;
	let dto: discord_protocol::ChannelDto =
		discord_protocol::decode(bytes).map_err(|_| Failure::Protocol)?;
	if dto.id.0 == 0
		|| dto.guild_id != Some(guild)
		|| channel.is_some_and(|id| dto.id != id)
		|| !matches!(dto.kind, 0 | 2 | 4 | 5 | 13 | 15 | 16)
		|| dto.is_obfuscated()
		|| !dto
			.name
			.as_deref()
			.is_some_and(client_core::channel_actions::valid_name)
		|| dto.parent_id.is_some_and(|id| id.0 == 0)
		|| !dto.recipients.is_empty()
	{
		return Err(Failure::Protocol);
	}
	Ok(value)
}
fn channel_result(bytes: &[u8], guild: Id, channel: Option<Id>) -> Result<Outcome, Failure> {
	let value = channel_value(bytes, guild, channel)?;
	let dto: discord_protocol::ChannelDto =
		discord_protocol::decode(bytes).map_err(|_| Failure::Protocol)?;
	let overwrites = overwrites_from_value(&value)?;
	let metadata = overwrites.map(|overwrites| permissions::Channel {
		id: dto.id,
		guild,
		overwrites: Some(overwrites),
	});
	Ok(Outcome::Channel {
		channel: Box::new(dto.into_model()),
		permissions: metadata,
	})
}
fn thread_reference_result(bytes: &[u8], guild: Id, channel: Id) -> Result<Outcome, Failure> {
	let dto: discord_protocol::ChannelDto =
		discord_protocol::decode(bytes).map_err(|_| Failure::Protocol)?;
	if dto.id != channel
		|| dto.guild_id != Some(guild)
		|| dto.is_obfuscated()
		|| !dto
			.name
			.as_deref()
			.is_some_and(client_core::channel_actions::valid_name)
	{
		return Err(Failure::Protocol);
	}
	let channel =
		discord_protocol::threads::into_thread(dto, guild).map_err(|_| Failure::Protocol)?;
	Ok(Outcome::Channel {
		channel: Box::new(channel),
		permissions: None,
	})
}
fn created_thread_result(bytes: &[u8], guild: Id, parent: Id) -> Result<Outcome, Failure> {
	let dto: discord_protocol::ChannelDto =
		discord_protocol::decode(bytes).map_err(|_| Failure::Protocol)?;
	if dto.parent_id != Some(parent)
		|| dto.id == parent
		|| dto.is_obfuscated()
		|| !dto
			.name
			.as_deref()
			.is_some_and(client_core::channel_actions::valid_name)
	{
		return Err(Failure::Protocol);
	}
	let channel =
		discord_protocol::threads::into_thread(dto, guild).map_err(|_| Failure::Protocol)?;
	Ok(Outcome::Channel {
		channel: Box::new(channel),
		permissions: None,
	})
}
// GET channel includes the current user's member object for joined threads.
fn post_result(bytes: &[u8], guild: Id, channel: Id) -> Result<Outcome, Failure> {
	let Outcome::Channel {
		channel: target, ..
	} = thread_reference_result(bytes, guild, channel)?
	else {
		unreachable!()
	};
	if !matches!(target.kind, 10..=12) {
		return Err(Failure::Protocol);
	}
	let value: Value = discord_protocol::decode(bytes).map_err(|_| Failure::Protocol)?;
	let owner = match value.get("owner_id") {
		None | Some(Value::Null) => None,
		Some(value) => {
			Some(serde_json::from_value::<Id>(value.clone()).map_err(|_| Failure::Protocol)?)
		}
	};
	if owner.is_some_and(|id| id.0 == 0) {
		return Err(Failure::Protocol);
	}
	let flags = value
		.get("flags")
		.map_or(Some(0), Value::as_u64)
		.ok_or(Failure::Protocol)?;
	let member = match value.get("member") {
		None | Some(Value::Null) => None,
		Some(member) if member.is_object() => Some(member),
		_ => return Err(Failure::Protocol),
	};
	if let Some(member) = member
		&& let Some(id) = member.get("id")
	{
		let id: Id = serde_json::from_value(id.clone()).map_err(|_| Failure::Protocol)?;
		if id != channel {
			return Err(Failure::Protocol);
		}
	}
	let member_flags = member
		.map_or(Some(0), |m| m["flags"].as_u64())
		.ok_or(Failure::Protocol)?;
	let level = match member_flags & 14 {
		0 => 3,
		2 => 0,
		4 => 1,
		8 => 2,
		_ => return Err(Failure::Protocol),
	};
	let muted = member
		.and_then(|m| m.get("muted"))
		.map_or(Some(false), Value::as_bool)
		.ok_or(Failure::Protocol)?;
	let mute_until = member
		.and_then(|m| m.get("mute_config"))
		.filter(|v| !v.is_null())
		.map(|v| {
			serde_json::from_value::<discord_protocol::notifications::MuteConfig>(v.clone())
				.map_err(|_| Failure::Protocol)
		})
		.transpose()?
		.and_then(|m| m.until());
	Ok(Outcome::Post {
		channel: target,
		details: PostDetails {
			owner,
			archived: value["thread_metadata"]["archived"]
				.as_bool()
				.ok_or(Failure::Protocol)?,
			locked: value["thread_metadata"]["locked"]
				.as_bool()
				.ok_or(Failure::Protocol)?,
			pinned: flags & 2 != 0,
			followed: member.is_some(),
			muted,
			level,
			mute_until,
		},
	})
}
fn overwrites_from_value(value: &Value) -> Result<Option<Vec<permissions::Overwrite>>, Failure> {
	value
		.get("permission_overwrites")
		.map(|rows| {
			let rows = rows
				.as_array()
				.filter(|r| r.len() <= permissions::MAX_OVERWRITES)
				.ok_or(Failure::Protocol)?;
			let mut result = Vec::with_capacity(rows.len());
			for row in rows {
				let id: Id =
					serde_json::from_value(row["id"].clone()).map_err(|_| Failure::Protocol)?;
				let kind = row["type"]
					.as_u64()
					.filter(|n| *n <= 1)
					.ok_or(Failure::Protocol)? as u8;
				let bits = |field: &str| {
					row[field]
						.as_str()
						.filter(|s| {
							!s.is_empty() && s.len() <= 39 && s.bytes().all(|b| b.is_ascii_digit())
						})
						.and_then(|s| s.parse::<u128>().ok())
						.ok_or(Failure::Protocol)
				};
				if id.0 == 0 || result.iter().any(|o: &permissions::Overwrite| o.id == id) {
					return Err(Failure::Protocol);
				}
				result.push(permissions::Overwrite {
					id,
					kind,
					allow: bits("allow")?,
					deny: bits("deny")?,
				});
			}
			Ok(result)
		})
		.transpose()
}
fn edit_from_value(value: &Value) -> Result<Edit, Failure> {
	let edit = Edit {
		overwrites: overwrites_from_value(value)?.ok_or(Failure::Protocol)?,
		name: value["name"].as_str().ok_or(Failure::Protocol)?.to_owned(),
		topic: match value.get("topic") {
			None | Some(Value::Null) => String::new(),
			Some(Value::String(s)) => s.clone(),
			_ => return Err(Failure::Protocol),
		},
		slowmode: value
			.get("rate_limit_per_user")
			.map_or(Some(0), Value::as_u64)
			.filter(|n| *n <= 21600)
			.ok_or(Failure::Protocol)? as u32,
		nsfw: value
			.get("nsfw")
			.map_or(Some(false), Value::as_bool)
			.ok_or(Failure::Protocol)?,
	};
	if !edit.valid() {
		return Err(Failure::Protocol);
	}
	Ok(edit)
}
fn duplicate_body(value: &Value, name: &str) -> Result<Value, Failure> {
	// Copy all documented create-channel settings, including every member/role overwrite.
	// Never infer a private channel's permissions from a truncated navigation model.
	if !value["permission_overwrites"].is_array() {
		return Err(Failure::Protocol);
	}
	let mut body = json!({"name":name});
	for field in [
		"type",
		"topic",
		"bitrate",
		"user_limit",
		"rate_limit_per_user",
		"position",
		"permission_overwrites",
		"parent_id",
		"nsfw",
		"rtc_region",
		"video_quality_mode",
		"default_auto_archive_duration",
		"default_reaction_emoji",
		"available_tags",
		"default_sort_order",
		"default_forum_layout",
		"default_thread_rate_limit_per_user",
		"flags",
	] {
		if let Some(value) = value.get(field) {
			body[field] = value.clone();
		}
	}
	// Existing forum tag identifiers belong to the source channel; new tags get new IDs.
	if let Some(tags) = body.get_mut("available_tags").and_then(Value::as_array_mut) {
		for tag in tags {
			if let Some(tag) = tag.as_object_mut() {
				tag.remove("id");
			}
		}
	}
	Ok(body)
}
impl DiscordApi {
	pub(super) async fn channel_action(
		&self,
		guild: Id,
		channel: Id,
		action: &Action,
	) -> Result<Outcome, Failure> {
		if guild.0 == 0 || channel.0 == 0 || !action.valid() {
			return Err(Failure::Protocol);
		}
		if matches!(
			action,
			Action::Mute(_) | Action::Notifications(_) | Action::HideMuted(_)
		) {
			return self
				.channel_notification_action(guild, channel, action)
				.await;
		}
		if let Action::Move {
			parent,
			position,
			lock_permissions,
			shifts,
		} = action
		{
			let mut entries = Vec::with_capacity(1 + shifts.len());
			let mut main_entry = json!({
				"id": channel.to_string(),
				"position": position,
			});
			if let Some(parent) = parent {
				main_entry["parent_id"] = parent.to_string().into();
				main_entry["lock_permissions"] = (*lock_permissions).into();
			} else {
				main_entry["parent_id"] = Value::Null;
			}
			entries.push(main_entry);
			for (shift_id, shift_pos) in shifts {
				entries.push(json!({
					"id": shift_id.to_string(),
					"position": shift_pos,
				}));
			}
			let bytes = self
				.request_limited(
					Method::PATCH,
					&format!("/guilds/{guild}/channels"),
					Some(Value::Array(entries)),
					64,
				)
				.await
				.map_err(write_failure)?;
			return if bytes.is_empty() {
				Ok(Outcome::Moved)
			} else {
				Err(Failure::Ambiguous)
			};
		}
		let path = format!("/channels/{channel}");
		let bytes = self
			.request_limited(Method::GET, &path, None, MAX_CHANNEL_BYTES)
			.await?;
		if *action == Action::Reference {
			return thread_reference_result(&bytes, guild, channel);
		}
		if matches!(
			action,
			Action::PostLoad
				| Action::PostFollow(_)
				| Action::PostArchive(_)
				| Action::PostLock(_)
				| Action::PostRename(_)
				| Action::PostPin(_)
				| Action::PostMute(_)
				| Action::PostNotifications(_)
		) || (matches!(action, Action::Delete)
			&& serde_json::from_slice::<Value>(&bytes)
				.ok()
				.is_some_and(|v| matches!(v["type"].as_u64(), Some(10..=12))))
		{
			return self.post_action(guild, channel, action, &bytes).await;
		}
		let source = channel_value(&bytes, guild, Some(channel))?;
		// Validate full overwrite metadata before copying it to a creation request.
		channel_result(&bytes, guild, Some(channel))?;
		if let Action::CreateThread { name, message } = action {
			if !matches!(source["type"].as_u64(), Some(0 | 5)) {
				return Err(Failure::Protocol);
			}
			// Documented thread creation: from one message, or standalone in the channel.
			let path = match message {
				Some(message) => format!("/channels/{channel}/messages/{message}/threads"),
				None => format!("/channels/{channel}/threads"),
			};
			let mut body = json!({"name": name, "auto_archive_duration": 4320});
			if message.is_none() {
				body["type"] = json!(11);
			}
			let bytes = self
				.request_limited(Method::POST, &path, Some(body), MAX_CHANNEL_BYTES)
				.await
				.map_err(write_failure)?;
			return created_thread_result(&bytes, guild, channel).map_err(write_failure);
		}
		let (method, path, body) = match action {
			Action::Reference => unreachable!("handled before channel settings validation"),
			Action::Load => {
				return edit_from_value(&source).map(Outcome::Details);
			}
			Action::Edit { before, after } => {
				let current = edit_from_value(&source)?;
				let mut body = json!({});
				if before.topic != after.topic && after.topic.chars().count() > 1024 {
					return Err(Failure::Protocol);
				}
				if !matches!(source["type"].as_u64(), Some(0 | 5))
					&& (before.topic != after.topic
						|| before.slowmode != after.slowmode
						|| before.nsfw != after.nsfw)
				{
					return Err(Failure::Protocol);
				}
				if (before.overwrites != after.overwrites
					&& current.overwrites != before.overwrites)
					|| (before.name != after.name && current.name != before.name)
					|| (before.topic != after.topic && current.topic != before.topic)
					|| (before.slowmode != after.slowmode && current.slowmode != before.slowmode)
					|| (before.nsfw != after.nsfw && current.nsfw != before.nsfw)
				{
					return Err(Failure::ProtocolAt(
						"Channel settings changed; reopen the editor before saving",
					));
				}
				if before.overwrites != after.overwrites {
					body["permission_overwrites"] = Value::Array(after.overwrites.iter().map(|row| {
						json!({"id":row.id.to_string(),"type":row.kind,"allow":row.allow.to_string(),"deny":row.deny.to_string()})
					}).collect());
				}
				if before.name != after.name {
					body["name"] = after.name.clone().into();
				}
				if before.topic != after.topic {
					body["topic"] = after.topic.clone().into();
				}
				if before.slowmode != after.slowmode {
					body["rate_limit_per_user"] = after.slowmode.into();
				}
				if before.nsfw != after.nsfw {
					body["nsfw"] = after.nsfw.into();
				}
				if body.as_object().is_some_and(|o| o.is_empty()) {
					return channel_result(&bytes, guild, Some(channel));
				}
				(Method::PATCH, path, Some(body))
			}
			Action::Delete => (Method::DELETE, path, None),
			Action::Duplicate { name } => (
				Method::POST,
				format!("/guilds/{guild}/channels"),
				Some(duplicate_body(&source, name)?),
			),
			Action::Create { name, kind } => {
				let parent = if source["type"] == 4 {
					json!(channel.to_string())
				} else {
					source["parent_id"].clone()
				};
				let mut body = json!({"name":name,"type":kind.wire_kind(),"parent_id":parent});
				if !parent.is_null() {
					let parent_id: Id =
						serde_json::from_value(parent).map_err(|_| Failure::Protocol)?;
					let category = if parent_id == channel {
						source.clone()
					} else {
						let bytes = self
							.request_limited(
								Method::GET,
								&format!("/channels/{parent_id}"),
								None,
								MAX_CHANNEL_BYTES,
							)
							.await?;
						channel_result(&bytes, guild, Some(parent_id))?;
						channel_value(&bytes, guild, Some(parent_id))?
					};
					if category["type"] != 4 || !category["permission_overwrites"].is_array() {
						return Err(Failure::Protocol);
					}
					body["permission_overwrites"] = category["permission_overwrites"].clone();
				}
				(
					Method::POST,
					format!("/guilds/{guild}/channels"),
					Some(body),
				)
			}
			Action::CreateCategory { name } => (
				Method::POST,
				format!("/guilds/{guild}/channels"),
				Some(json!({"name":name,"type":4})),
			),
			_ => return Err(Failure::Protocol),
		};
		let bytes = self
			.request_limited(method, &path, body, MAX_CHANNEL_BYTES)
			.await
			.map_err(write_failure)?;
		if matches!(action, Action::Delete) {
			channel_value(&bytes, guild, Some(channel)).map_err(write_failure)?;
			return Ok(Outcome::Deleted);
		}
		let expected = matches!(action, Action::Edit { .. }).then_some(channel);
		let outcome = channel_result(&bytes, guild, expected).map_err(write_failure)?;
		if let Action::Edit { before, after } = action
			&& before.overwrites != after.overwrites
		{
			let confirmed = match &outcome {
				Outcome::Channel { permissions, .. } => permissions
					.as_ref()
					.and_then(|p| p.overwrites.as_ref())
					.is_some_and(|rows| {
						rows.len() == after.overwrites.len()
							&& after.overwrites.iter().all(|row| rows.contains(row))
					}),
				_ => false,
			};
			if !confirmed {
				return Err(Failure::Ambiguous);
			}
		}
		if let Outcome::Channel {
			channel: created, ..
		} = &outcome
		{
			if expected.is_none() && created.id == channel {
				return Err(Failure::Ambiguous);
			}
			if let Action::Create { kind, .. } = action
				&& created.kind != kind.wire_kind()
			{
				return Err(Failure::Ambiguous);
			}
			if matches!(action, Action::CreateCategory { .. }) && created.kind != 4 {
				return Err(Failure::Ambiguous);
			}
			if matches!(action, Action::Duplicate { .. })
				&& Some(u64::from(created.kind)) != source["type"].as_u64()
			{
				return Err(Failure::Ambiguous);
			}
		}
		Ok(outcome)
	}
	async fn post_action(
		&self,
		guild: Id,
		channel: Id,
		action: &Action,
		bytes: &[u8],
	) -> Result<Outcome, Failure> {
		let initial = post_result(bytes, guild, channel)?;
		let Outcome::Post {
			channel: target,
			details,
		} = &initial
		else {
			unreachable!()
		};
		let parent = target.parent_id.ok_or(Failure::Protocol)?;
		let parent_bytes = self
			.request_limited(
				Method::GET,
				&format!("/channels/{parent}"),
				None,
				MAX_CHANNEL_BYTES,
			)
			.await?;
		let parent_value = channel_value(&parent_bytes, guild, Some(parent))?;
		if !matches!(parent_value["type"].as_u64(), Some(0 | 5 | 15 | 16)) {
			return Err(Failure::Protocol);
		}
		if matches!(action, Action::PostLoad) {
			return Ok(initial);
		}
		let source: Value = discord_protocol::decode(bytes).map_err(|_| Failure::Protocol)?;
		let path = format!("/channels/{channel}");
		let (method, write_path, body) = match action {
			Action::PostFollow(follow) => {
				if details.archived {
					return Err(Failure::ProtocolAt(
						"Reopen the post before changing follow settings",
					));
				}
				(
					if *follow { Method::PUT } else { Method::DELETE },
					format!("{path}/thread-members/@me"),
					None,
				)
			}
			Action::PostArchive(archived) => (
				Method::PATCH,
				path.clone(),
				Some(json!({"archived":archived})),
			),
			Action::PostLock(locked) => {
				(Method::PATCH, path.clone(), Some(json!({"locked":locked})))
			}
			Action::PostRename(name) => (Method::PATCH, path.clone(), Some(json!({"name":name}))),
			Action::PostPin(pinned) => {
				let flags = source
					.get("flags")
					.map_or(Some(0), Value::as_u64)
					.ok_or(Failure::Protocol)?;
				(
					Method::PATCH,
					path.clone(),
					Some(json!({"flags":if *pinned { flags | 2 } else { flags & !2 }})),
				)
			}
			Action::Delete => (Method::DELETE, path.clone(), None),
			Action::PostMute(_) | Action::PostNotifications(_) => {
				if !details.followed {
					return Err(Failure::ProtocolAt(
						"Follow the post before changing its notifications",
					));
				}
				// Unofficial normal-user route: https://docs.discord.food/resources/channel#modify-thread-settings
				let body = match action {
					Action::PostNotifications(level) => {
						let flags = source["member"]["flags"]
							.as_u64()
							.ok_or(Failure::Protocol)?;
						json!({"flags": (flags & !15) | match level {0 => 2, 1 => 4, 2 => 8, _ => 0}})
					}
					Action::PostMute(mute) => {
						let seconds = if let Mute::For(seconds) = mute {
							Some(*seconds)
						} else {
							None
						};
						let end = seconds
							.map(|seconds| {
								let until = SystemTime::now()
									.duration_since(UNIX_EPOCH)
									.unwrap_or_default()
									.as_secs() + u64::from(seconds);
								discord_protocol::pins::format_cursor(
									i128::from(until) * 1_000_000_000,
								)
							})
							.transpose()
							.map_err(|_| Failure::Protocol)?;
						json!({"muted": *mute != Mute::Unmute, "mute_config":{"end_time":end,"selected_time_window":seconds.map_or(-1,i64::from)}})
					}
					_ => unreachable!(),
				};
				(
					Method::PATCH,
					format!("{path}/thread-members/@me/settings"),
					Some(body),
				)
			}
			_ => return Err(Failure::Protocol),
		};
		let requested_until = body
			.as_ref()
			.and_then(|body| body.get("mute_config"))
			.map(|config| {
				serde_json::from_value::<discord_protocol::notifications::MuteConfig>(
					config.clone(),
				)
				.map_err(|_| Failure::Protocol)
			})
			.transpose()?
			.and_then(|config| config.until());
		let response = self
			.request_limited(method, &write_path, body, MAX_CHANNEL_BYTES)
			.await
			.map_err(write_failure)?;
		if matches!(action, Action::Delete) {
			thread_reference_result(&response, guild, channel).map_err(write_failure)?;
			return Ok(Outcome::Deleted);
		}
		let refreshed = self
			.request_limited(Method::GET, &path, None, MAX_CHANNEL_BYTES)
			.await
			.map_err(write_failure)?;
		let outcome = post_result(&refreshed, guild, channel).map_err(write_failure)?;
		if let (Action::PostMute(mute), Outcome::Post { details, .. }) = (action, &outcome)
			&& *mute != Mute::Unmute
			&& details.mute_until != requested_until
		{
			return Err(Failure::Ambiguous);
		}
		Ok(outcome)
	}
	async fn channel_notification_action(
		&self,
		guild: Id,
		channel: Id,
		action: &Action,
	) -> Result<Outcome, Failure> {
		if let Action::HideMuted(hide) = action {
			let bytes = self
				.request_limited(
					Method::PATCH,
					&format!("/users/@me/guilds/{guild}/settings"),
					Some(json!({"hide_muted_channels": hide})),
					512 * 1024,
				)
				.await
				.map_err(write_failure)?;
			let setting: discord_protocol::notifications::Setting =
				discord_protocol::decode(&bytes).map_err(|_| Failure::Ambiguous)?;
			if setting.guild_id != Some(guild) || setting.hide_muted_channels != Some(*hide) {
				return Err(Failure::Ambiguous);
			}
			return Ok(Outcome::HideMuted(*hide));
		}
		let mut requested_until = None;
		let override_body = match action {
			Action::Notifications(level) => json!({"message_notifications":level}),
			Action::Mute(mute) => {
				let seconds = if let Mute::For(seconds) = mute {
					Some(*seconds)
				} else {
					None
				};
				requested_until = seconds.map(|seconds| {
					SystemTime::now()
						.duration_since(UNIX_EPOCH)
						.unwrap_or_default()
						.as_secs() as i64 + i64::from(seconds)
				});
				let end = requested_until
					.map(|until| {
						discord_protocol::pins::format_cursor(i128::from(until) * 1_000_000_000)
					})
					.transpose()
					.map_err(|_| Failure::Protocol)?;
				json!({"muted": *mute != Mute::Unmute, "mute_config":{"end_time":end,"selected_time_window":seconds.map_or(-1,i64::from)}})
			}
			_ => return Err(Failure::Protocol),
		};
		let bytes = self
			.request_limited(
				Method::PATCH,
				&format!("/users/@me/guilds/{guild}/settings"),
				Some(json!({"channel_overrides":{channel.to_string():override_body}})),
				512 * 1024,
			)
			.await
			.map_err(write_failure)?;
		let setting: discord_protocol::notifications::Setting =
			discord_protocol::decode(&bytes).map_err(|_| Failure::Ambiguous)?;
		if setting.guild_id != Some(guild) {
			return Err(Failure::Ambiguous);
		}
		let overrides = setting.channel_overrides.ok_or(Failure::Ambiguous)?.0;
		let mut matching = overrides.iter().filter(|o| o.channel_id == channel);
		let row = matching.next().ok_or(Failure::Ambiguous)?;
		if matching.next().is_some() || row.message_notifications.is_some_and(|l| l > 3) {
			return Err(Failure::Ambiguous);
		}
		if match action {
			Action::Mute(mute) => row.muted != Some(*mute != Mute::Unmute),
			Action::Notifications(level) => row.message_notifications != Some(*level),
			_ => true,
		} {
			return Err(Failure::Ambiguous);
		}
		if let Action::Mute(mute) = action
			&& *mute != Mute::Unmute
			&& row.mute_config.as_ref().and_then(|m| m.until()) != requested_until
		{
			return Err(Failure::Ambiguous);
		}
		Ok(Outcome::Preferences {
			muted: row.muted,
			level: row.message_notifications,
			mute_until: row.mute_config.as_ref().and_then(|m| m.until()),
		})
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	use client_core::auth::SessionSecret;
	use client_core::channel_actions::CreateKind;
	use std::sync::Arc;
	use tokio::{
		io::{AsyncReadExt, AsyncWriteExt},
		net::TcpListener,
	};
	fn source() -> Value {
		json!({"id":"3","guild_id":"2","name":"private","type":0,"parent_id":"4","topic":"Keep topic","nsfw":true,"rate_limit_per_user":30,"flags":0,"permission_overwrites":[{"id":"2","type":0,"allow":"0","deny":"1024"},{"id":"8","type":1,"allow":"1024","deny":"0"}]})
	}
	async fn reply(listener: &TcpListener, path: &str, status: u16, body: Value) -> Value {
		let (mut socket, _) = listener.accept().await.unwrap();
		let mut bytes = Vec::new();
		let payload;
		loop {
			let mut chunk = [0; 2048];
			let n = socket.read(&mut chunk).await.unwrap();
			assert!(n > 0);
			bytes.extend_from_slice(&chunk[..n]);
			assert!(bytes.len() < 64 * 1024);
			if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
				let headers = std::str::from_utf8(&bytes[..end]).unwrap();
				let length: usize = headers
					.lines()
					.find_map(|line| {
						line.to_ascii_lowercase()
							.strip_prefix("content-length: ")
							.map(str::to_owned)
					})
					.map_or(0, |s| s.parse().unwrap());
				if bytes.len() < end + 4 + length {
					continue;
				}
				assert!(headers.starts_with(path), "{headers}");
				payload = if length == 0 {
					Value::Null
				} else {
					serde_json::from_slice(&bytes[end + 4..]).unwrap()
				};
				break;
			}
		}
		let body = body.to_string();
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
		payload
	}
	#[test]
	fn post_details_validate_thread_scope_and_member_flags() {
		let mut value = json!({"id":"4","guild_id":"2","parent_id":"3","type":11,"name":"Post","owner_id":"7","flags":2,"thread_metadata":{"archived":false,"locked":true},"member":{"id":"4","flags":5,"muted":true}});
		let result = post_result(&serde_json::to_vec(&value).unwrap(), Id(2), Id(4)).unwrap();
		assert!(matches!(
			result,
			Outcome::Post {
				details: PostDetails {
					owner: Some(Id(7)),
					followed: true,
					pinned: true,
					locked: true,
					muted: true,
					level: 1,
					..
				},
				..
			}
		));
		value["member"]["flags"] = 6.into();
		assert!(post_result(&serde_json::to_vec(&value).unwrap(), Id(2), Id(4)).is_err());
		value.as_object_mut().unwrap().remove("member");
		assert!(matches!(
			post_result(&serde_json::to_vec(&value).unwrap(), Id(2), Id(4)),
			Ok(Outcome::Post {
				details: PostDetails {
					followed: false,
					level: 3,
					..
				},
				..
			})
		));
		assert!(post_result(&serde_json::to_vec(&value).unwrap(), Id(9), Id(4)).is_err());
	}
	#[test]
	fn thread_references_require_the_requested_guild_id_and_parent() {
		let valid =
			br#"{"id":"4","guild_id":"2","parent_id":"3","type":11,"name":"Synthetic thread"}"#;
		let Outcome::Channel {
			channel,
			permissions,
		} = thread_reference_result(valid, Id(2), Id(4)).unwrap()
		else {
			panic!()
		};
		assert_eq!(channel.name, "Synthetic thread");
		assert_eq!(channel.parent_id, Some(Id(3)));
		assert!(permissions.is_none());
		for invalid in [
			br#"{"id":"4","guild_id":"9","parent_id":"3","type":11,"name":"Foreign"}"#.as_slice(),
			br#"{"id":"4","guild_id":"2","type":11,"name":"No parent"}"#.as_slice(),
			br#"{"id":"4","guild_id":"2","parent_id":"3","type":0,"name":"Not a thread"}"#
				.as_slice(),
		] {
			assert!(thread_reference_result(invalid, Id(2), Id(4)).is_err());
		}
	}

	#[tokio::test]
	async fn channel_routes_preserve_permissions_scope_partial_edits_and_never_retry() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_CHANNEL_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let mut created = source();
		created["id"] = "9".into();
		let server = async {
			reply(&listener, "GET /channels/3 HTTP/1.1", 200, source()).await;
			let body = reply(
				&listener,
				"POST /guilds/2/channels HTTP/1.1",
				200,
				created.clone(),
			)
			.await;
			assert_eq!(
				body["permission_overwrites"],
				source()["permission_overwrites"]
			);
			assert_eq!(body["topic"], "Keep topic");
			assert_eq!(body["nsfw"], true);
			assert_eq!(body["rate_limit_per_user"], 30);
			assert_eq!(body["parent_id"], "4");
		};
		let duplicate = Action::Duplicate {
			name: "copy".into(),
		};
		let (result, ()) = tokio::join!(api.channel_action(Id(2), Id(3), &duplicate), server);
		assert!(matches!(result, Ok(Outcome::Channel { .. })));
		for kind in [CreateKind::Text, CreateKind::Voice, CreateKind::Forum] {
			for direct_category in [false, true] {
				for response_kind in [kind.wire_kind(), 4] {
					let server = async {
						let mut category = source();
						category["id"] = "4".into();
						category["type"] = 4.into();
						category["permission_overwrites"][0]["allow"] =
							(1_u128 << 100).to_string().into();
						if !direct_category {
							reply(&listener, "GET /channels/3 HTTP/1.1", 200, source()).await;
						}
						reply(&listener, "GET /channels/4 HTTP/1.1", 200, category.clone()).await;
						let mut response = created.clone();
						response["type"] = response_kind.into();
						let body =
							reply(&listener, "POST /guilds/2/channels HTTP/1.1", 200, response)
								.await;
						assert_eq!(
							body,
							json!({"name":"new","type":kind.wire_kind(),"parent_id":"4","permission_overwrites":category["permission_overwrites"]})
						);
					};
					let create = Action::Create {
						name: "new".into(),
						kind,
					};
					let anchor = if direct_category { Id(4) } else { Id(3) };
					let (result, ()) =
						tokio::join!(api.channel_action(Id(2), anchor, &create), server);
					if response_kind == kind.wire_kind() {
						assert!(
							matches!(result, Ok(Outcome::Channel { channel, .. }) if channel.kind == kind.wire_kind())
						);
					} else {
						assert!(matches!(result, Err(Failure::Ambiguous)));
					}
				}
			}
		}
		let server = async {
			reply(&listener, "GET /channels/3 HTTP/1.1", 200, source()).await;
			let mut category = source();
			category["id"] = "10".into();
			category["name"] = "Projects".into();
			category["type"] = 4.into();
			let body = reply(&listener, "POST /guilds/2/channels HTTP/1.1", 200, category).await;
			assert_eq!(body, json!({"name":"Projects","type":4}));
		};
		let create = Action::CreateCategory {
			name: "Projects".into(),
		};
		let (result, ()) = tokio::join!(api.channel_action(Id(2), Id(3), &create), server);
		assert!(matches!(result, Ok(Outcome::Channel { channel, .. }) if channel.kind == 4));
		let server = async {
			reply(&listener, "GET /channels/3 HTTP/1.1", 200, source()).await;
			let body = reply(&listener, "PATCH /channels/3 HTTP/1.1", 500, json!({})).await;
			assert_eq!(body, json!({"name":"rename"}));
		};
		let edit = Action::Edit {
			before: Edit {
				name: "private".into(),
				topic: "stale topic".into(),
				slowmode: 30,
				nsfw: true,
				overwrites: edit_from_value(&source()).unwrap().overwrites,
			},
			after: Edit {
				name: "rename".into(),
				topic: "stale topic".into(),
				slowmode: 30,
				nsfw: true,
				overwrites: edit_from_value(&source()).unwrap().overwrites,
			},
		};
		let (result, ()) = tokio::join!(api.channel_action(Id(2), Id(3), &edit), server);
		assert!(matches!(result, Err(Failure::Ambiguous)));
		let mut category = source();
		category["type"] = 4.into();
		category["permission_overwrites"][0]["allow"] = (1_u128 << 100).to_string().into();
		let before = edit_from_value(&category).unwrap();
		let mut after = before.clone();
		after.overwrites[0].deny |= permissions::SEND_MESSAGES;
		let edit = Action::Edit {
			before: before.clone(),
			after: after.clone(),
		};
		let server = async {
			reply(&listener, "GET /channels/3 HTTP/1.1", 200, category.clone()).await;
			let mut saved = category.clone();
			saved["permission_overwrites"][0]["deny"] = after.overwrites[0].deny.to_string().into();
			saved["permission_overwrites"]
				.as_array_mut()
				.unwrap()
				.reverse();
			let body = reply(&listener, "PATCH /channels/3 HTTP/1.1", 200, saved).await;
			assert_eq!(body.as_object().unwrap().len(), 1);
			assert_eq!(
				body["permission_overwrites"][0]["allow"],
				(1_u128 << 100).to_string()
			);
			assert_eq!(
				body["permission_overwrites"][1],
				category["permission_overwrites"][1]
			);
		};
		let (result, ()) = tokio::join!(api.channel_action(Id(2), Id(3), &edit), server);
		assert!(result.is_ok());
		for missing in [false, true] {
			let server = async {
				reply(&listener, "GET /channels/3 HTTP/1.1", 200, category.clone()).await;
				let mut response = category.clone();
				if missing {
					response
						.as_object_mut()
						.unwrap()
						.remove("permission_overwrites");
				}
				reply(&listener, "PATCH /channels/3 HTTP/1.1", 200, response).await;
			};
			let (result, ()) = tokio::join!(api.channel_action(Id(2), Id(3), &edit), server);
			assert!(matches!(result, Err(Failure::Ambiguous)));
		}
		let server = async {
			let mut changed = category.clone();
			changed["permission_overwrites"][1]["deny"] = "64".into();
			reply(&listener, "GET /channels/3 HTTP/1.1", 200, changed).await;
		};
		let (result, ()) = tokio::join!(api.channel_action(Id(2), Id(3), &edit), server);
		assert!(matches!(result, Err(Failure::ProtocolAt(_))));
		let server = async {
			let mut wrong = source();
			wrong["guild_id"] = "99".into();
			reply(&listener, "GET /channels/3 HTTP/1.1", 200, wrong).await;
		};
		let (result, ()) = tokio::join!(api.channel_action(Id(2), Id(3), &Action::Delete), server);
		assert!(matches!(result, Err(Failure::Protocol)));
		let server = async {
			let body = reply(&listener,"PATCH /users/@me/guilds/2/settings HTTP/1.1",200,json!({"guild_id":"2","channel_overrides":[{"channel_id":"3","muted":false,"message_notifications":2}]})).await;
			assert_eq!(
				body,
				json!({"channel_overrides":{"3":{"message_notifications":2}}})
			);
		};
		let (result, ()) = tokio::join!(
			api.channel_action(Id(2), Id(3), &Action::Notifications(2)),
			server
		);
		assert!(matches!(
			result,
			Ok(Outcome::Preferences { level: Some(2), .. })
		));
		assert!(
			tokio::time::timeout(std::time::Duration::from_millis(10), listener.accept())
				.await
				.is_err()
		);
	}
	#[tokio::test]
	async fn channel_move_uses_the_guild_position_route() {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let mut api = DiscordApi::new(Arc::new(
			SessionSecret::from_owner_input("SYNTHETIC_CHANNEL_TOKEN".into()).unwrap(),
		))
		.unwrap();
		api.base = format!("http://{}", listener.local_addr().unwrap());
		let server = async {
			let body = reply(
				&listener,
				"PATCH /guilds/2/channels HTTP/1.1",
				204,
				Value::Null,
			)
			.await;
			assert_eq!(
				body,
				json!([
					{"id":"3","position":2,"parent_id":"4","lock_permissions":true},
					{"id":"5","position":1}
				])
			);
		};
		let action = Action::Move {
			parent: Some(Id(4)),
			position: 2,
			lock_permissions: true,
			shifts: vec![(Id(5), 1)],
		};
		let (result, ()) = tokio::join!(api.channel_action(Id(2), Id(3), &action), server);
		assert!(matches!(result, Ok(Outcome::Moved)));
	}
	#[test]
	fn duplication_keeps_voice_and_forum_fields_and_rejects_missing_overwrites() {
		let value = json!({"type":15,"permission_overwrites":[],"available_tags":[{"id":"7","name":"Help","moderated":true}],"default_reaction_emoji":{"emoji_name":"ok"},"default_forum_layout":2,"default_thread_rate_limit_per_user":12,"default_sort_order":1,"flags":16,"bitrate":96000,"rtc_region":null,"video_quality_mode":2,"user_limit":5});
		let body = duplicate_body(&value, "copy").unwrap();
		assert!(body["available_tags"][0].get("id").is_none());
		for field in [
			"default_reaction_emoji",
			"default_forum_layout",
			"default_thread_rate_limit_per_user",
			"default_sort_order",
			"flags",
			"bitrate",
			"rtc_region",
			"video_quality_mode",
			"user_limit",
		] {
			assert_eq!(body[field], value[field]);
		}
		assert!(duplicate_body(&json!({"type":0}), "copy").is_err());
		assert!(channel_result(&serde_json::to_vec(&json!({"id":"3","guild_id":"2","type":0,"name":"private","permission_overwrites":[{"id":"2","type":0,"allow":"oops","deny":"0"}]})).unwrap(),Id(2),Some(Id(3))).is_err());
	}
}
