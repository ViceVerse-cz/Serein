//! On-demand server administration DTOs, including unofficial normal-user member search.
//! https://docs.discord.food/resources/guild#search-guild-members
use crate::{DecodeError, Timestamp, UserDto, permissions::List};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use model::{
	Id,
	server_admin::{self as m, Cursor, Emoji, Emojis, Member, Members, Query, Role},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub const MAX_WIRE: usize = 2 * 1024 * 1024;
#[derive(Deserialize)]
pub struct GuildMetadata {
	pub id: Id,
	pub owner_id: Id,
	pub(crate) roles: List<RoleWire, 512>,
	pub(crate) features: List<String, 256>,
}
#[derive(Deserialize)]
pub struct RoleWire {
	#[serde(flatten)]
	role: crate::permissions::Role,
	managed: bool,
}
impl GuildMetadata {
	pub fn checked_roles(self) -> Result<(Vec<Role>, Vec<String>), DecodeError> {
		let roles = self
			.roles
			.0
			.into_iter()
			.map(|role| {
				Ok(Role {
					role: role.role.checked()?,
					managed: role.managed,
				})
			})
			.collect::<Result<Vec<_>, DecodeError>>()?;
		let mut seen = std::collections::BTreeSet::new();
		if roles.iter().any(|role| !seen.insert(role.role.id))
			|| self.features.0.iter().any(|feature| {
				feature.len() > 128
					|| !feature
						.bytes()
						.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
			}) {
			return Err(DecodeError);
		}
		Ok((roles, self.features.0))
	}
}
pub fn guild_metadata(bytes: &[u8], guild: Id) -> Result<GuildMetadata, DecodeError> {
	let result: GuildMetadata = crate::decode(bytes)?;
	if result.id != guild {
		return Err(DecodeError);
	}
	Ok(result)
}
#[derive(Deserialize)]
struct EmojiWire {
	id: Id,
	name: String,
	#[serde(default)]
	animated: bool,
	available: Option<bool>,
	managed: bool,
	roles: Option<Vec<Id>>,
	user: Option<UserDto>,
}
fn emoji_model(value: EmojiWire) -> Emoji {
	Emoji {
		emoji: model::CustomEmoji {
			id: value.id,
			name: value.name,
			animated: value.animated,
			available: value.available.unwrap_or(true),
			managed: value.managed,
			roles: value.roles,
		},
		uploader: value.user.map(UserDto::into_model),
	}
}
pub fn emojis(bytes: &[u8]) -> Result<Emojis, DecodeError> {
	let rows: List<EmojiWire, 1000> = crate::decode(bytes)?;
	let result = Emojis {
		items: rows.0.into_iter().map(emoji_model).collect(),
		static_limit: None,
		animated_limit: None,
	};
	let mut ids = std::collections::BTreeSet::new();
	if result.items.iter().any(|row| !ids.insert(row.emoji.id)) {
		return Err(DecodeError);
	}
	if !m::Result::Emojis(result.clone()).valid() {
		return Err(DecodeError);
	}
	Ok(result)
}
pub fn emoji(bytes: &[u8]) -> Result<Emoji, DecodeError> {
	let row = emoji_model(crate::decode(bytes)?);
	if !row.emoji.valid() {
		return Err(DecodeError);
	}
	Ok(row)
}
#[derive(Deserialize)]
struct StickerWire {
	#[serde(flatten)]
	sticker: model::Sticker,
	#[serde(rename = "type")]
	kind: u8,
	user: Option<UserDto>,
}
impl StickerWire {
	fn checked(self, guild: Id) -> Result<m::Sticker, DecodeError> {
		if self.kind != 2 || !matches!(self.sticker.format_type, 1..=4) {
			return Err(DecodeError);
		}
		let sticker = crate::stickers::guild_catalog(vec![self.sticker], guild)?
			.pop()
			.ok_or(DecodeError)?;
		let row = m::Sticker {
			sticker,
			uploader: self.user.map(UserDto::into_model),
		};
		if !m::Result::Stickers(m::Stickers {
			items: vec![row.clone()],
			limit: None,
		})
		.valid()
		{
			return Err(DecodeError);
		}
		Ok(row)
	}
}
pub fn stickers(bytes: &[u8], guild: Id) -> Result<m::Stickers, DecodeError> {
	let rows: List<StickerWire, { model::MAX_GUILD_STICKERS }> = crate::decode(bytes)?;
	let result = m::Stickers {
		items: rows
			.0
			.into_iter()
			.map(|row| row.checked(guild))
			.collect::<Result<_, _>>()?,
		limit: None,
	};
	let mut ids = std::collections::BTreeSet::new();
	if result.items.iter().any(|row| !ids.insert(row.sticker.id))
		|| !m::Result::Stickers(result.clone()).valid()
	{
		return Err(DecodeError);
	}
	Ok(result)
}
pub fn sticker(bytes: &[u8], guild: Id) -> Result<m::Sticker, DecodeError> {
	crate::decode::<StickerWire>(bytes)?.checked(guild)
}
#[derive(Deserialize)]
struct MemberWire {
	user: UserDto,
	nick: Option<String>,
	#[serde(deserialize_with = "crate::permissions::member_roles")]
	roles: Vec<Id>,
	joined_at: Option<Timestamp>,
	flags: Option<u64>,
	unusual_dm_activity_until: Option<Timestamp>,
	communication_disabled_until: Option<Timestamp>,
}
impl MemberWire {
	fn checked(self) -> Result<Member, DecodeError> {
		let member = Member {
			user: self.user.into_model(),
			nick: self.nick,
			roles: self.roles,
			joined_at: self.joined_at.map(|value| value.0 / 1_000_000),
			join_source: None,
			invite_code: None,
			flags: self.flags,
			unusual_dm_until: self
				.unusual_dm_activity_until
				.map(|value| value.0 / 1_000_000),
			timeout_until: self
				.communication_disabled_until
				.map(|value| value.0 / 1_000_000),
		};
		if !member.valid() {
			return Err(DecodeError);
		}
		Ok(member)
	}
}
pub fn member(bytes: &[u8], user: Id) -> Result<Member, DecodeError> {
	let row = crate::decode::<MemberWire>(bytes)?.checked()?;
	if row.user.id != user {
		return Err(DecodeError);
	}
	Ok(row)
}
#[derive(Deserialize)]
struct Supplemental {
	member: MemberWire,
	join_source_type: Option<u8>,
	source_invite_code: Option<String>,
}
#[derive(Deserialize)]
struct Search {
	guild_id: Id,
	members: List<Supplemental, 100>,
	page_result_count: usize,
	total_result_count: u64,
}
pub fn members(bytes: &[u8], guild: Id, metadata: GuildMetadata) -> Result<Members, DecodeError> {
	let response: Search = crate::decode(bytes)?;
	if response.guild_id != guild || response.page_result_count != response.members.0.len() {
		return Err(DecodeError);
	}
	let items = response
		.members
		.0
		.into_iter()
		.map(|row| {
			let mut member = row.member.checked()?;
			member.join_source = row.join_source_type;
			member.invite_code = row.source_invite_code;
			Ok(member)
		})
		.collect::<Result<Vec<_>, DecodeError>>()?;
	let mut ids = std::collections::BTreeSet::new();
	if response.total_result_count < items.len() as u64
		|| items.iter().any(|row| !ids.insert(row.user.id))
	{
		return Err(DecodeError);
	}
	let next = if items.len() == m::PAGE_SIZE {
		items.last().and_then(|member| {
			Some(Cursor {
				user: member.user.id,
				joined_at: member.joined_at?.try_into().ok()?,
			})
		})
	} else {
		None
	};
	let (roles, features) = metadata.checked_roles()?;
	let show_in_channel_list = (!features.iter().any(|value| value == "COMMUNITY")).then(|| {
		features
			.iter()
			.any(|value| value == m::MEMBER_CHANNEL_FEATURE)
	});
	let result = Members {
		items,
		roles,
		total: response.total_result_count,
		next,
		features,
		show_in_channel_list,
	};
	if !m::Result::Members(result.clone()).valid() {
		return Err(DecodeError);
	}
	Ok(result)
}
pub fn query(query: &Query, now_millis: i64) -> Result<Value, DecodeError> {
	if !query.valid() {
		return Err(DecodeError);
	}
	let mut value = json!({"limit":m::PAGE_SIZE,"sort":query.sort});
	if !query.search.trim().is_empty() {
		value["or_query"] = if let Ok(user) = query.search.trim().parse::<Id>() {
			json!({"user_id":{"or_query":[user]}})
		} else {
			json!({"usernames":{"or_query":[query.search.trim()]}})
		};
	}
	if query.recent {
		value["and_query"] = json!({"guild_joined_at":{"range":{"gte":now_millis.saturating_sub(7 * 86400 * 1000)}}});
	}
	if let Some(cursor) = query.after {
		value["after"] = json!({"user_id":cursor.user,"guild_joined_at":cursor.joined_at});
	}
	Ok(value)
}
fn valid_image(bytes: &[u8], animated: bool) -> bool {
	if bytes.is_empty() || bytes.len() > m::MAX_IMAGE_BYTES {
		return false;
	}
	if animated {
		bytes.len() >= 14
			&& (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
			&& bytes.last() == Some(&0x3b)
			&& [6, 8].into_iter().all(|offset| {
				(1..=4096).contains(&u16::from_le_bytes(
					bytes[offset..offset + 2].try_into().unwrap(),
				))
			})
	} else {
		bytes.len() >= 33
			&& bytes.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR")
			&& [16, 20].into_iter().all(|offset| {
				(1..=128).contains(&u32::from_be_bytes(
					bytes[offset..offset + 4].try_into().unwrap(),
				))
			})
	}
}
pub fn emoji_data_uri(bytes: &[u8], animated: bool) -> Option<String> {
	valid_image(bytes, animated).then(|| {
		let mut value = format!(
			"data:image/{};base64,{}",
			if animated { "gif" } else { "png" },
			STANDARD.encode(bytes)
		);
		value.shrink_to_fit();
		value
	})
}
pub fn valid_emoji_data_uri(value: &str) -> bool {
	if value.len() > m::MAX_IMAGE_URI {
		return false;
	}
	let (data, animated) = if let Some(data) = value.strip_prefix("data:image/png;base64,") {
		(data, false)
	} else if let Some(data) = value.strip_prefix("data:image/gif;base64,") {
		(data, true)
	} else {
		return false;
	};
	STANDARD
		.decode(data)
		.is_ok_and(|bytes| valid_image(&bytes, animated))
}
pub fn valid_sticker_file(filename: &str, content_type: &str, bytes: &[u8]) -> bool {
	if bytes.is_empty()
		|| bytes.len() > m::MAX_STICKER_FILE_BYTES
		|| filename.len() > 128
		|| !filename
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
	{
		return false;
	}
	match (content_type, filename.rsplit('.').next()) {
		("image/png", Some("png")) => valid_sticker_png(bytes),
		("image/gif", Some("gif")) => {
			bytes.len() >= 14
				&& (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"))
				&& u16::from_le_bytes([bytes[6], bytes[7]]) == 320
				&& u16::from_le_bytes([bytes[8], bytes[9]]) == 320
				&& bytes.last() == Some(&0x3b)
		}
		("application/json", Some("json")) => {
			let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
				return false;
			};
			let number = |key| value.get(key).and_then(Value::as_f64);
			matches!(number("w"), Some(320.0))
				&& matches!(number("h"), Some(320.0))
				&& number("fr").is_some_and(|value| value.is_finite() && value > 0.0)
				&& number("ip").is_some_and(f64::is_finite)
				&& number("op").is_some_and(f64::is_finite)
				&& number("op")
					.zip(number("ip"))
					.zip(number("fr"))
					.is_some_and(|((end, start), rate)| end >= start && end - start <= rate * 5.0)
		}
		_ => false,
	}
}
fn valid_sticker_png(bytes: &[u8]) -> bool {
	if bytes.len() < 45
		|| !bytes.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR")
		|| u32::from_be_bytes(bytes[16..20].try_into().unwrap()) != 320
		|| u32::from_be_bytes(bytes[20..24].try_into().unwrap()) != 320
	{
		return false;
	}
	let mut offset = 8usize;
	while offset.checked_add(12).is_some_and(|end| end <= bytes.len()) {
		let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
		let Some(end) = offset
			.checked_add(12)
			.and_then(|value| value.checked_add(length))
		else {
			return false;
		};
		if end > bytes.len() {
			return false;
		}
		if &bytes[offset + 4..offset + 8] == b"IEND" {
			return length == 0 && end == bytes.len();
		}
		offset = end;
	}
	false
}
pub fn pruned(bytes: &[u8]) -> Result<Option<u64>, DecodeError> {
	#[derive(Deserialize)]
	struct Response {
		pruned: Option<u64>,
	}
	Ok(crate::decode::<Response>(bytes)?.pruned)
}

#[cfg(test)]
mod sticker_tests {
	use super::*;

	fn png(width: u32, height: u32) -> Vec<u8> {
		let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
		bytes.extend_from_slice(&width.to_be_bytes());
		bytes.extend_from_slice(&height.to_be_bytes());
		bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
		bytes.extend_from_slice(&[0; 4]);
		bytes.extend_from_slice(b"\0\0\0\0IEND\0\0\0\0");
		bytes
	}

	#[test]
	fn guild_stickers_bind_scope_and_preserve_uploader() {
		let bytes = br#"[{"id":"4","name":"Wave","description":"A wave","tags":"wave","type":2,"format_type":1,"user":{"id":"7","username":"Uploader"}}]"#;
		let page = stickers(bytes, Id(2)).unwrap();
		assert_eq!(page.items[0].sticker.guild_id, Some(Id(2)));
		assert_eq!(page.items[0].uploader.as_ref().unwrap().id, Id(7));

		let foreign = br#"[{"id":"4","name":"Wave","description":"A wave","tags":"wave","type":2,"format_type":1,"guild_id":"3"}]"#;
		assert!(stickers(foreign, Id(2)).is_err());
		let duplicate = format!(
			"[{},{}]",
			String::from_utf8_lossy(&bytes[1..bytes.len() - 1]),
			String::from_utf8_lossy(&bytes[1..bytes.len() - 1])
		);
		assert!(stickers(duplicate.as_bytes(), Id(2)).is_err());
	}

	#[test]
	fn sticker_files_require_declared_type_size_and_320_square_dimensions() {
		assert!(valid_sticker_file("wave.png", "image/png", &png(320, 320)));
		assert!(!valid_sticker_file("wave.png", "image/png", &png(319, 320)));
		assert!(!valid_sticker_file("wave.gif", "image/png", &png(320, 320)));

		let mut gif = b"GIF89a\x40\x01\x40\x01\0\0\0".to_vec();
		gif.push(0x3b);
		assert!(valid_sticker_file("wave.gif", "image/gif", &gif));
		let lottie = br#"{"w":320,"h":320,"fr":60,"ip":0,"op":300}"#;
		assert!(valid_sticker_file("wave.json", "application/json", lottie));
		assert!(!valid_sticker_file(
			"wave.json",
			"application/json",
			br#"{"w":320,"h":320,"fr":60,"ip":0,"op":301}"#
		));
	}
}
