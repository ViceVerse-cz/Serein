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
	splash: Option<String>,
	description: Option<String>,
}
#[derive(Deserialize)]
struct Channel {
	id: model::Id,
	name: Option<String>,
	icon: Option<String>,
	#[serde(rename = "type")]
	kind: Option<u8>,
}
/// Optional display text: an oversized or control-laden value is dropped, not fatal.
fn text(value: Option<String>, max: usize) -> Option<String> {
	value.map(|value| value.trim().to_owned()).filter(|value| {
		!value.is_empty()
			&& value.len() <= max
			&& !value.chars().any(|c| c.is_control() && c != '\n')
	})
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
		// Voice (2) and stage (13) invites name the channel they drop the user into.
		let voice_channel = invite
			.channel
			.as_ref()
			.filter(|channel| matches!(channel.kind, Some(2 | 13)))
			.and_then(|channel| text(channel.name.clone(), 400));
		let guild_icon = guild
			.icon
			.clone()
			.filter(|hash| model::valid_avatar_hash(hash));
		// The card's header art: the banner, else the invite splash.
		let header = media(guild.banner, "banners").or_else(|| media(guild.splash, "splashes"));
		Ok(model::InvitePreview {
			guild: Some(guild.id),
			channel: invite.channel.map(|c| c.id),
			about: text(guild.description, model::MAX_INVITE_ABOUT),
			guild_icon,
			voice_channel,
			embed: model::Embed {
				kind: "discord-invite".into(),
				title: Some(guild.name),
				description: counts,
				thumbnail: media(guild.icon, "icons"),
				image: header,
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
			about: None,
			guild_icon: None,
			voice_channel: None,
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

	#[test]
	fn voice_and_stage_invites_name_their_channel_and_bound_the_description() {
		let invite = |kind: u8, description: &str| {
			let json = serde_json::json!({
				"guild": {
					"id": "2",
					"name": "Synthetic",
					"description": description,
					"icon": "0123456789abcdef0123456789abcdef",
					"splash": "fedcba9876543210fedcba9876543210"
				},
				"channel": {"id": "3", "name": "Lounge", "type": kind},
				"approximate_presence_count": 4,
				"approximate_member_count": 9
			});
			decode(&serde_json::to_vec(&json).unwrap()).unwrap()
		};
		let voice = invite(2, "A synthetic server");
		assert_eq!(voice.voice_channel.as_deref(), Some("Lounge"));
		assert_eq!(voice.about.as_deref(), Some("A synthetic server"));
		// Without a banner the card's header falls back to the invite splash.
		assert!(
			voice
				.embed
				.image
				.as_ref()
				.and_then(|m| m.url.as_deref())
				.is_some_and(|url| {
					url.starts_with("https://cdn.discordapp.com/splashes/2/fedcba")
				})
		);
		assert_eq!(
			voice.guild_icon.as_deref(),
			Some("0123456789abcdef0123456789abcdef")
		);
		assert_eq!(invite(13, "").voice_channel.as_deref(), Some("Lounge"));
		let text = invite(0, &"x".repeat(model::MAX_INVITE_ABOUT + 1));
		// A text channel invite is a server card, and an oversized description is dropped
		// rather than failing the whole preview.
		assert!(text.voice_channel.is_none() && text.about.is_none());
		assert_eq!(text.embed.title.as_deref(), Some("Synthetic"));
	}
}
