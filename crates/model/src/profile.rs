//! One on-demand profile. Profile metadata is never persisted by the client.
use crate::{Id, User, valid_avatar_hash};

pub const MAX_PROFILE_BYTES: usize = 64 * 1024;
pub const MAX_PROFILE_NAME_CHARS: usize = 32;
pub const MAX_PROFILE_BIO_CHARS: usize = 190;
pub const MAX_PROFILE_PRONOUNS_CHARS: usize = 40;
pub const MAX_PROFILE_EDIT_BYTES: usize = 4096;

/// Only explicitly changed fields are sent. Nested `None` clears a nullable field.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProfileEdit {
	pub global_name: Option<Option<String>>,
	pub bio: Option<String>,
	pub pronouns: Option<String>,
	pub accent_color: Option<Option<u32>>,
}
impl ProfileEdit {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.global_name.as_ref().map_or(0, bytes)
			+ bytes(&self.bio)
			+ bytes(&self.pronouns)
	}
	pub fn valid(&self) -> bool {
		fn text(value: &str, max: usize, multiline: bool) -> bool {
			value.len() <= max * 4
				&& value.chars().count() <= max
				&& value
					.chars()
					.all(|c| !c.is_control() || (multiline && matches!(c, '\n' | '\r' | '\t')))
		}
		self.bytes() <= MAX_PROFILE_EDIT_BYTES
			&& self.global_name.as_ref().is_none_or(|name| {
				name.as_ref().is_none_or(|name| {
					!name.trim().is_empty() && text(name, MAX_PROFILE_NAME_CHARS, false)
				})
			}) && self
			.bio
			.as_ref()
			.is_none_or(|bio| text(bio, MAX_PROFILE_BIO_CHARS, true))
			&& self
				.pronouns
				.as_ref()
				.is_none_or(|pronouns| text(pronouns, MAX_PROFILE_PRONOUNS_CHARS, false))
			&& self
				.accent_color
				.flatten()
				.is_none_or(|color| color <= 0xff_ffff)
	}
}

#[cfg(test)]
mod edit_tests {
	use super::*;
	#[test]
	fn profile_edit_bounds_unicode_clear_values_and_retained_bytes() {
		let mut edit = ProfileEdit {
			global_name: Some(Some("🦀".repeat(MAX_PROFILE_NAME_CHARS))),
			bio: Some("🦀".repeat(MAX_PROFILE_BIO_CHARS)),
			pronouns: Some("🦀".repeat(MAX_PROFILE_PRONOUNS_CHARS)),
			accent_color: Some(Some(0xff_ffff)),
		};
		assert!(edit.valid());
		edit.bio.as_mut().unwrap().push('x');
		assert!(!edit.valid());
		edit.bio = Some("First line\nSecond line".into());
		edit.global_name = Some(None);
		edit.pronouns = Some(String::new());
		edit.accent_color = Some(None);
		assert!(edit.valid());
		edit.global_name = Some(Some(" ".into()));
		assert!(!edit.valid());
		edit.global_name = None;
		edit.accent_color = Some(Some(0x100_0000));
		assert!(!edit.valid());
		edit.accent_color = None;
		edit.pronouns = Some("they\0them".into());
		assert!(!edit.valid());
		edit.pronouns = Some(String::with_capacity(MAX_PROFILE_EDIT_BYTES));
		assert!(!edit.valid());
	}
}
#[derive(Clone)]
pub struct UserProfile {
	pub user: User,
	pub username: String,
	pub global_name: Option<String>,
	pub banner: Option<String>,
	pub accent_color: Option<u32>,
	pub bio: String,
	pub pronouns: String,
	pub badges: Vec<ProfileBadge>,
	pub connections: Vec<ProfileConnection>,
	pub mutual_guilds: Vec<ProfileGuild>,
	pub guild: Option<GuildProfile>,
	/// Two profile theme colors (top, bottom) when the account configured them.
	pub theme_colors: Option<[u32; 2]>,
	/// Displayed server tag when the account enabled one.
	pub clan: Option<ClanTag>,
	pub limited: bool,
}
#[derive(Clone)]
pub struct ProfileBadge {
	pub id: String,
	pub description: String,
	pub icon: Option<String>,
}
impl ProfileBadge {
	pub fn icon_key(&self) -> Option<String> {
		self.icon
			.as_deref()
			.filter(|hash| valid_avatar_hash(hash))
			.map(|hash| format!("badge-{hash}"))
	}
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClanTag {
	pub guild: Id,
	pub tag: String,
	pub badge: Option<String>,
}
impl ClanTag {
	pub fn badge_key(&self) -> Option<String> {
		self.badge
			.as_deref()
			.filter(|hash| valid_avatar_hash(hash))
			.map(|hash| format!("clan-{}-{hash}", self.guild))
	}
}
#[derive(Clone)]
pub struct ProfileConnection {
	pub kind: String,
	pub name: String,
	pub verified: bool,
}
#[derive(Clone)]
pub struct ProfileGuild {
	pub id: Id,
	pub nick: Option<String>,
}
#[derive(Clone)]
pub struct GuildProfile {
	pub guild: Id,
	/// Role IDs assigned to this member in the selected server.
	pub roles: Vec<Id>,
	pub nick: Option<String>,
	pub avatar: Option<String>,
	pub banner: Option<String>,
	pub bio: String,
	pub pronouns: String,
	pub joined_at: Option<String>,
}
fn bytes(value: &Option<String>) -> usize {
	value.as_ref().map_or(0, String::capacity)
}
impl UserProfile {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.user.heap_bytes()
			+ self.username.capacity()
			+ bytes(&self.global_name)
			+ bytes(&self.banner)
			+ self.bio.capacity()
			+ self.pronouns.capacity()
			+ self.badges.capacity() * size_of::<ProfileBadge>()
			+ self
				.badges
				.iter()
				.map(|b| b.id.capacity() + b.description.capacity() + bytes(&b.icon))
				.sum::<usize>()
			+ self
				.clan
				.as_ref()
				.map_or(0, |c| c.tag.capacity() + bytes(&c.badge))
			+ self.connections.capacity() * size_of::<ProfileConnection>()
			+ self
				.connections
				.iter()
				.map(|c| c.kind.capacity() + c.name.capacity())
				.sum::<usize>()
			+ self.mutual_guilds.capacity() * size_of::<ProfileGuild>()
			+ self
				.mutual_guilds
				.iter()
				.map(|g| bytes(&g.nick))
				.sum::<usize>()
			+ self.guild.as_ref().map_or(0, |g| {
				size_of::<GuildProfile>()
					+ g.roles.capacity() * size_of::<Id>()
					+ bytes(&g.nick)
					+ bytes(&g.avatar)
					+ bytes(&g.banner)
					+ g.bio.capacity()
					+ g.pronouns.capacity()
					+ bytes(&g.joined_at)
			})
	}
	pub fn valid(&self) -> bool {
		self.bytes() <= MAX_PROFILE_BYTES
			&& self.user.id.0 != 0
			&& self.user.name.len() <= 512
			&& self.username.len() <= 512
			&& self.global_name.as_ref().is_none_or(|s| s.len() <= 512)
			&& self.bio.len() <= 4096
			&& self.pronouns.len() <= 256
			&& self.accent_color.is_none_or(|c| c <= 0xff_ffff)
			&& self
				.theme_colors
				.is_none_or(|colors| colors.iter().all(|c| *c <= 0xff_ffff))
			&& self.clan.as_ref().is_none_or(|c| {
				c.guild.0 != 0
					&& !c.tag.is_empty()
					&& c.tag.len() <= 32
					&& c.badge.as_deref().is_none_or(valid_avatar_hash)
			}) && [&self.banner, &self.user.avatar]
			.into_iter()
			.all(|h| h.as_deref().is_none_or(valid_avatar_hash))
			&& self.badges.len() <= 16
			&& self.connections.len() <= 16
			&& self.mutual_guilds.len() <= 50
			&& self.badges.iter().all(|b| {
				b.id.len() <= 64
					&& b.description.len() <= 1024
					&& b.icon.as_deref().is_none_or(valid_avatar_hash)
			}) && self
			.connections
			.iter()
			.all(|c| c.kind.len() <= 64 && c.name.len() <= 512)
			&& self
				.mutual_guilds
				.iter()
				.all(|g| g.id.0 != 0 && g.nick.as_ref().is_none_or(|n| n.len() <= 512))
			&& self.guild.as_ref().is_none_or(|g| {
				g.guild.0 != 0
					&& g.roles.len() <= crate::permissions::MAX_MEMBER_ROLES
					&& g.roles.iter().all(|id| id.0 != 0)
					&& g.roles.windows(2).all(|ids| ids[0] < ids[1])
					&& g.nick.as_ref().is_none_or(|n| n.len() <= 512)
					&& g.bio.len() <= 4096
					&& g.pronouns.len() <= 256
					&& g.joined_at.as_ref().is_none_or(|s| s.len() <= 64)
					&& [&g.avatar, &g.banner]
						.into_iter()
						.all(|h| h.as_deref().is_none_or(valid_avatar_hash))
			})
	}
	pub fn banner_key(&self) -> Option<String> {
		if let Some(guild) = &self.guild
			&& let Some(hash) = guild.banner.as_deref().filter(|h| valid_avatar_hash(h))
		{
			return Some(format!(
				"member-banner-{}-{}-{hash}",
				guild.guild, self.user.id
			));
		}
		self.banner
			.as_deref()
			.filter(|h| valid_avatar_hash(h))
			.map(|h| format!("banner-{}-{h}", self.user.id))
	}
	pub fn avatar_key(&self) -> String {
		if let Some(guild) = &self.guild
			&& let Some(hash) = guild.avatar.as_deref().filter(|h| valid_avatar_hash(h))
		{
			return format!("member-avatar-{}-{}-{hash}", guild.guild, self.user.id);
		}
		self.user.avatar_key()
	}
}
