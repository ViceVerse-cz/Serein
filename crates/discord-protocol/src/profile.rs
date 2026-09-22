//! Unofficial normal-user profile response; see docs/profiles.md for source evidence.
use crate::{DecodeError, UserDto};
use model::{GuildProfile, Id, ProfileBadge, ProfileConnection, ProfileGuild, UserProfile};
use serde::{
	Deserialize, Deserializer,
	de::{SeqAccess, Visitor},
};

pub const MAX_PROFILE_WIRE: usize = 256 * 1024;

/// Unofficial current-user PATCH fields; omit untouched fields, retain explicit nulls.
pub fn encode_edit(changes: &model::ProfileEdit) -> Result<serde_json::Value, DecodeError> {
	if !changes.valid() {
		return Err(DecodeError);
	}
	let mut fields = serde_json::Map::new();
	if let Some(name) = &changes.global_name {
		fields.insert("global_name".into(), serde_json::json!(name));
	}
	if let Some(bio) = &changes.bio {
		fields.insert("bio".into(), serde_json::json!(bio));
	}
	if let Some(pronouns) = &changes.pronouns {
		fields.insert("pronouns".into(), serde_json::json!(pronouns));
	}
	if let Some(color) = changes.accent_color {
		fields.insert("accent_color".into(), serde_json::json!(color));
	}
	if let Some(avatar) = &changes.avatar {
		fields.insert("avatar".into(), serde_json::json!(avatar));
	}
	Ok(serde_json::Value::Object(fields))
}
#[derive(Deserialize)]
struct ProfileDto {
	user: ProfileUser,
	#[serde(default)]
	user_profile: Option<Metadata>,
	#[serde(default)]
	guild_member: Option<Member>,
	#[serde(default)]
	guild_member_profile: Option<Metadata>,
	#[serde(default)]
	badges: Small<Badge, 16>,
	#[serde(default)]
	guild_badges: Small<Badge, 16>,
	#[serde(default)]
	connected_accounts: Small<Connection, 16>,
	#[serde(default)]
	mutual_guilds: Small<MutualGuild, 50>,
}
#[derive(Deserialize)]
struct ProfileUser {
	#[serde(flatten)]
	user: UserDto,
	#[serde(default)]
	bio: Option<String>,
	#[serde(default)]
	banner: Option<String>,
	#[serde(default)]
	accent_color: Option<u32>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct Metadata {
	bio: Option<String>,
	pronouns: Option<String>,
	banner: Option<String>,
	accent_color: Option<u32>,
	theme_colors: Option<Small<u32, 2>>,
	guild_id: Option<GuildId>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum GuildId {
	Text(Id),
	Number(u64),
}
impl GuildId {
	fn id(&self) -> Id {
		match self {
			Self::Text(id) => *id,
			Self::Number(id) => Id(*id),
		}
	}
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct Member {
	#[serde(default)]
	roles: Small<Id, 512>,
	nick: Option<String>,
	avatar: Option<String>,
	banner: Option<String>,
	bio: Option<String>,
	joined_at: Option<String>,
	user: Option<UserDto>,
}
#[derive(Deserialize)]
struct Badge {
	id: String,
	description: String,
	#[serde(default)]
	icon: Option<String>,
}
#[derive(Deserialize)]
struct Connection {
	#[serde(rename = "type")]
	kind: String,
	name: String,
	#[serde(default)]
	verified: bool,
}
#[derive(Deserialize)]
struct MutualGuild {
	id: Id,
	#[serde(default)]
	nick: Option<String>,
}
struct Small<T, const N: usize> {
	items: Vec<T>,
	limited: bool,
}
impl<T, const N: usize> Default for Small<T, N> {
	fn default() -> Self {
		Self {
			items: vec![],
			limited: false,
		}
	}
}
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for Small<T, N> {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct Items<T, const N: usize>(std::marker::PhantomData<T>);
		impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Items<T, N> {
			type Value = Small<T, N>;
			fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				f.write_str("bounded profile list or null")
			}
			fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
				// Restricted profiles can retain identity while withholding these lists.
				Ok(Small {
					limited: true,
					..Small::default()
				})
			}
			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
				let mut list = Small::default();
				for _ in 0..N {
					match seq.next_element()? {
						Some(item) => list.items.push(item),
						None => {
							list.items.shrink_to_fit();
							return Ok(list);
						}
					}
				}
				while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
					list.limited = true;
				}
				list.items.shrink_to_fit();
				Ok(list)
			}
		}
		deserializer.deserialize_any(Items::<T, N>(std::marker::PhantomData))
	}
}
fn text(value: String, chars: usize, limited: &mut bool) -> String {
	let end = value
		.char_indices()
		.nth(chars)
		.map_or(value.len(), |(index, _)| index);
	*limited |= end < value.len();
	value[..end].to_owned()
}
fn hash(value: Option<String>) -> Option<String> {
	value.filter(|s| model::valid_avatar_hash(s))
}
fn timestamp(value: Option<String>) -> Option<String> {
	value.filter(|s| s.len() <= 64 && crate::Timestamp::try_from(s.clone()).is_ok())
}
pub fn decode_profile(bytes: &[u8], guild: Option<Id>) -> Result<UserProfile, DecodeError> {
	if bytes.len() > MAX_PROFILE_WIRE {
		return Err(DecodeError);
	}
	let dto: ProfileDto = crate::decode(bytes)?;
	let user_id = dto.user.user.id;
	if dto
		.guild_member
		.as_ref()
		.and_then(|m| m.user.as_ref())
		.is_some_and(|u| u.id != user_id)
		|| dto
			.guild_member_profile
			.as_ref()
			.and_then(|m| m.guild_id.as_ref())
			.is_some_and(|id| Some(id.id()) != guild)
	{
		return Err(DecodeError);
	}
	let mut limited = dto.badges.limited
		|| dto.guild_badges.limited
		|| dto.connected_accounts.limited
		|| dto.mutual_guilds.limited
		|| dto.user_profile.is_none();
	let username = text(dto.user.user.username.clone(), 128, &mut limited);
	let global_name = dto
		.user
		.user
		.global_name
		.as_ref()
		.map(|s| text(s.clone(), 128, &mut limited));
	let metadata = dto.user_profile.unwrap_or_default();
	let mut badges: Vec<_> = dto
		.badges
		.items
		.into_iter()
		.chain(dto.guild_badges.items)
		.map(|badge| ProfileBadge {
			id: text(badge.id, 64, &mut limited),
			description: text(badge.description, 256, &mut limited),
			icon: hash(badge.icon),
		})
		.collect();
	limited |= badges.len() > 16;
	badges.truncate(16);
	badges.shrink_to_fit();
	let guild = match (guild, dto.guild_member) {
		(Some(guild), Some(mut member)) => {
			let profile = dto.guild_member_profile.unwrap_or_default();
			limited |= member.roles.limited;
			member.roles.items.sort_unstable();
			if member.roles.items.iter().any(|id| id.0 == 0)
				|| member.roles.items.windows(2).any(|ids| ids[0] == ids[1])
			{
				return Err(DecodeError);
			}
			member.roles.items.shrink_to_fit();
			Some(GuildProfile {
				guild,
				roles: member.roles.items,
				nick: member.nick.map(|n| text(n, 128, &mut limited)),
				avatar: hash(member.avatar),
				banner: hash(profile.banner.or(member.banner)),
				bio: text(
					profile.bio.or(member.bio).unwrap_or_default(),
					1024,
					&mut limited,
				),
				pronouns: text(profile.pronouns.unwrap_or_default(), 64, &mut limited),
				joined_at: timestamp(member.joined_at),
			})
		}
		_ => None,
	};
	// Exactly two colors are documented by observation; anything else is not a theme.
	let theme_colors = metadata
		.theme_colors
		.as_ref()
		.filter(|colors| !colors.limited)
		.and_then(|colors| {
			let [top, bottom] = colors.items[..] else {
				return None;
			};
			(top <= 0xff_ffff && bottom <= 0xff_ffff).then_some([top, bottom])
		});
	let user = dto.user.user.into_model();
	let clan = user.primary_guild.as_deref().cloned();
	let mut profile = UserProfile {
		user,
		username,
		global_name,
		banner: hash(metadata.banner.or(dto.user.banner)),
		accent_color: metadata
			.accent_color
			.or(dto.user.accent_color)
			.filter(|c| *c <= 0xff_ffff),
		bio: text(
			metadata.bio.or(dto.user.bio).unwrap_or_default(),
			1024,
			&mut limited,
		),
		pronouns: text(metadata.pronouns.unwrap_or_default(), 64, &mut limited),
		badges,
		connections: dto
			.connected_accounts
			.items
			.into_iter()
			.map(|c| ProfileConnection {
				kind: text(c.kind, 16, &mut limited),
				name: text(c.name, 128, &mut limited),
				verified: c.verified,
			})
			.collect(),
		mutual_guilds: dto
			.mutual_guilds
			.items
			.into_iter()
			.map(|g| ProfileGuild {
				id: g.id,
				nick: g.nick.map(|n| text(n, 128, &mut limited)),
			})
			.collect(),
		guild,
		theme_colors,
		clan,
		limited,
	};
	// Keep identity/about fields; large returned mutual lists are the first expendable summaries.
	while profile.bytes() > model::MAX_PROFILE_BYTES {
		profile.limited = true;
		if profile.mutual_guilds.pop().is_some() {
			profile.mutual_guilds.shrink_to_fit();
		} else if profile.badges.pop().is_some() {
			profile.badges.shrink_to_fit();
		} else {
			return Err(DecodeError);
		}
	}
	if !profile.valid() {
		return Err(DecodeError);
	}
	Ok(profile)
}
#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;
	#[test]
	fn partial_profiles_preserve_identity_and_reject_malformed_lists() {
		let base = json!({"user":{"id":"1","username":"synthetic","global_name":"Display","avatar":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"user_profile":{},"badges":[],"guild_badges":[],"connected_accounts":[],"mutual_guilds":[]});
		assert!(
			!decode_profile(base.to_string().as_bytes(), None)
				.unwrap()
				.limited
		);
		for field in [
			"badges",
			"guild_badges",
			"connected_accounts",
			"mutual_guilds",
		] {
			let mut value = base.clone();
			value[field] = serde_json::Value::Null;
			let profile = decode_profile(value.to_string().as_bytes(), None).unwrap();
			assert_eq!(profile.username, "synthetic");
			assert_eq!(profile.user.name, "Display");
			assert_eq!(
				profile.user.avatar.as_deref(),
				Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
			);
			assert!(profile.limited);
			assert!(profile.valid());
			for invalid in [json!({}), json!(false), json!(1), json!(""), json!([null])] {
				value[field] = invalid;
				assert!(decode_profile(value.to_string().as_bytes(), None).is_err());
			}
		}
	}
	#[test]
	fn profile_metadata_is_bounded_and_guild_identity_is_checked() {
		let value = json!({"user":{"id":"1","username":"name","global_name":"Display","avatar":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","bio":"global bio","primary_guild":{"identity_guild_id":"2","identity_enabled":true,"tag":"SRN","badge":"ffffffffffffffffffffffffffffffff"}},
            "user_profile":{"bio":"About me","pronouns":"they/them","banner":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","accent_color":123,"theme_colors":[1193046,16777215]},
            "guild_member":{"roles":["8","7"],"nick":"Server name","avatar":"cccccccccccccccccccccccccccccccc","joined_at":"2026-01-01T00:00:00Z"},
            "guild_member_profile":{"guild_id":2,"banner":"dddddddddddddddddddddddddddddddd","bio":"Server bio"},
            "badges":[{"id":"badge","description":"Synthetic badge","icon":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}],"connected_accounts":[{"type":"github","name":"synthetic","verified":true}],"mutual_guilds":[{"id":"2","nick":"Server name"}]});
		let profile = decode_profile(value.to_string().as_bytes(), Some(Id(2))).unwrap();
		assert_eq!(profile.user.name, "Display");
		assert_eq!(profile.username, "name");
		assert_eq!(profile.bio, "About me");
		assert!(
			profile
				.banner_key()
				.unwrap()
				.starts_with("member-banner-2-1-")
		);
		assert!(profile.avatar_key().starts_with("member-avatar-2-1-"));
		assert_eq!(profile.guild.as_ref().unwrap().bio, "Server bio");
		assert_eq!(profile.guild.as_ref().unwrap().roles, [Id(7), Id(8)]);
		assert_eq!(profile.theme_colors, Some([0x123456, 0xffffff]));
		let clan = profile.clan.as_ref().unwrap();
		assert_eq!((clan.guild, clan.tag.as_str()), (Id(2), "SRN"));
		assert_eq!(profile.user.primary_guild.as_deref(), Some(clan));
		assert_eq!(
			clan.badge_key().as_deref(),
			Some("clan-2-ffffffffffffffffffffffffffffffff")
		);
		assert_eq!(
			profile.badges[0].icon_key().as_deref(),
			Some("badge-eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
		);
		assert!(profile.valid());
		let disabled = json!({"user":{"id":"1","username":"n","clan":{"identity_guild_id":"2","identity_enabled":false,"tag":"OFF"}},
            "user_profile":{"theme_colors":[1,2,3]},"badges":[{"id":"b","description":"d","icon":"../evil"}]});
		let profile = decode_profile(disabled.to_string().as_bytes(), None).unwrap();
		assert!(profile.clan.is_none());
		assert!(profile.theme_colors.is_none());
		assert!(profile.badges[0].icon.is_none());
		let too_bright =
			json!({"user":{"id":"1","username":"n"},"user_profile":{"theme_colors":[16777216,0]}});
		assert!(
			decode_profile(too_bright.to_string().as_bytes(), None)
				.unwrap()
				.theme_colors
				.is_none()
		);
		assert!(decode_profile(value.to_string().as_bytes(), Some(Id(3))).is_err());
		assert!(decode_profile(&vec![0; MAX_PROFILE_WIRE + 1], None).is_err());
		let huge = json!({"user":{"id":"1","username":"x".repeat(10000)},"user_profile":{"bio":"世".repeat(5000),"banner":"../invalid"},"mutual_guilds":vec![json!({"id":"2","nick":"文".repeat(300)});70]});
		let profile = decode_profile(huge.to_string().as_bytes(), None).unwrap();
		assert!(profile.valid());
		assert!(profile.limited);
		assert!(profile.banner.is_none());
		assert!(profile.mutual_guilds.len() <= 50);
		assert!(
			decode_profile(
				br#"{"user":{"id":"1","username":"User"},"user_profile":null}"#,
				None
			)
			.unwrap()
			.limited
		);
	}
}
