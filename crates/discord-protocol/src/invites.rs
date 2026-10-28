//! Bounded public invite metadata; normal-user compatibility remains unverified.
use serde::Deserialize;
#[derive(Deserialize)]
struct Invite {
	guild: Option<Guild>,
	channel: Option<Channel>,
	approximate_presence_count: Option<u64>,
	approximate_member_count: Option<u64>,
}
#[derive(Deserialize)]
struct Guild {
	id: model::Id,
	name: String,
	icon: Option<String>,
	banner: Option<String>,
}
#[derive(Deserialize)]
struct Channel {
	id: model::Id,
	name: Option<String>,
	icon: Option<String>,
}
pub fn decode(bytes: &[u8]) -> Result<model::InvitePreview, crate::DecodeError> {
	if bytes.len() > 64 * 1024 {
		return Err(crate::DecodeError);
	}
	let invite: Invite = serde_json::from_slice(bytes).map_err(|_| crate::DecodeError)?;
	let counts = match (
		invite.approximate_presence_count,
		invite.approximate_member_count,
	) {
		(Some(online), Some(members)) => Some(format!("● {online} online · {members} members")),
		(_, Some(members)) => Some(format!("{members} members")),
		_ => None,
	};
	if let Some(guild) = invite.guild {
		if guild.id.0 == 0 || guild.name.len() > 400 {
			return Err(crate::DecodeError);
		}
		let media = |hash: Option<String>, folder: &str| {
			hash.filter(|h| model::valid_avatar_hash(h))
				.map(|hash| model::EmbedMedia {
					url: Some(format!(
						"https://cdn.discordapp.com/{folder}/{}/{hash}.png?size=512",
						guild.id
					)),
					width: if folder == "icons" { 128 } else { 512 },
					height: if folder == "icons" { 128 } else { 288 },
					..Default::default()
				})
		};
		Ok(model::InvitePreview {
			guild: Some(guild.id),
			channel: invite.channel.map(|c| c.id),
			embed: model::Embed {
				kind: "discord-invite".into(),
				title: Some(guild.name),
				description: counts,
				thumbnail: media(guild.icon, "icons"),
				image: media(guild.banner, "banners"),
				..Default::default()
			},
		})
	} else if let Some(channel) = invite.channel {
		if channel.id.0 == 0 {
			return Err(crate::DecodeError);
		}
		let title = channel.name.unwrap_or_else(|| "Group DM".into());
		if title.len() > 400 {
			return Err(crate::DecodeError);
		}
		let thumbnail = channel
			.icon
			.filter(|h| model::valid_avatar_hash(h))
			.map(|hash| model::EmbedMedia {
				url: Some(format!(
					"https://cdn.discordapp.com/channel-icons/{}/{hash}.png?size=128",
					channel.id
				)),
				width: 128,
				height: 128,
				..Default::default()
			});
		Ok(model::InvitePreview {
			guild: None,
			channel: Some(channel.id),
			embed: model::Embed {
				kind: "discord-invite".into(),
				title: Some(title),
				description: counts,
				thumbnail,
				image: None,
				..Default::default()
			},
		})
	} else {
		Err(crate::DecodeError)
	}
}

/// A write response must confirm its scope before the invite is exposed to the user.
pub fn created_code(
	bytes: &[u8],
	guild: model::Id,
	channel: model::Id,
	max_age: u32,
	max_uses: u16,
	temporary: bool,
) -> Result<String, crate::DecodeError> {
	#[derive(Deserialize)]
	struct Scope {
		id: model::Id,
	}
	#[derive(Deserialize)]
	struct Created {
		code: String,
		guild: Scope,
		channel: Scope,
		max_age: u32,
		max_uses: u16,
		temporary: bool,
	}
	if bytes.len() > 64 * 1024 {
		return Err(crate::DecodeError);
	}
	let created: Created = serde_json::from_slice(bytes).map_err(|_| crate::DecodeError)?;
	if created.guild.id != guild
		|| created.max_age != max_age
		|| created.max_uses != max_uses
		|| created.temporary != temporary
		|| created.channel.id != channel
		|| guild.0 == 0
		|| channel.0 == 0
		|| created.code.is_empty()
		|| created.code.len() > 100
		|| !created
			.code
			.bytes()
			.all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
	{
		return Err(crate::DecodeError);
	}
	Ok(created.code)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn decodes_group_dm_invite_without_guild() {
		let json = serde_json::json!({
			"code": "groupcode",
			"type": 1,
			"channel": {
				"id": "123456789012345678",
				"name": "Cool Group",
				"icon": "85a643534b4c6b5b5bfbc92b9b7bf611",
				"type": 3
			},
			"approximate_member_count": 5
		});
		let bytes = serde_json::to_vec(&json).unwrap();
		let preview = decode(&bytes).unwrap();
		assert_eq!(preview.guild, None);
		assert_eq!(preview.channel, Some(model::Id(123456789012345678)));
		assert_eq!(preview.embed.title.as_deref(), Some("Cool Group"));
		assert_eq!(preview.embed.description.as_deref(), Some("5 members"));
		assert!(preview.embed.thumbnail.is_some());
	}
}
