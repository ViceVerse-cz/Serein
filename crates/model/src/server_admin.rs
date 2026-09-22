//! Bounded on-demand server administration data; absent metadata stays unknown.
use crate::{CustomEmoji, Id, User, permissions};

pub const PAGE_SIZE: usize = 100;
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 256 * 1024;
pub const MAX_IMAGE_URI: usize = 24 + 4 * MAX_IMAGE_BYTES.div_ceil(3);
pub const MEMBER_CHANNEL_FEATURE: &str = "ENABLED_MODERATION_EXPERIENCE_FOR_NON_COMMUNITY";
#[derive(Clone)]
pub struct Emoji {
	pub emoji: CustomEmoji,
	pub uploader: Option<User>,
}
#[derive(Clone, Default)]
pub struct Emojis {
	pub items: Vec<Emoji>,
	pub static_limit: Option<usize>,
	pub animated_limit: Option<usize>,
}
#[derive(Clone)]
pub struct Member {
	pub user: User,
	pub nick: Option<String>,
	pub roles: Vec<Id>,
	pub joined_at: Option<i128>,
	pub join_source: Option<u8>,
	pub invite_code: Option<String>,
	pub flags: Option<u64>,
	pub unusual_dm_until: Option<i128>,
	pub timeout_until: Option<i128>,
}
#[derive(Clone)]
pub struct Role {
	pub role: permissions::Role,
	pub managed: bool,
}
#[derive(Clone, Default)]
pub struct Members {
	pub items: Vec<Member>,
	pub roles: Vec<Role>,
	pub total: u64,
	pub next: Option<Cursor>,
	pub features: Vec<String>,
	pub show_in_channel_list: Option<bool>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
	pub user: Id,
	pub joined_at: i64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
	pub search: String,
	pub sort: u8,
	pub recent: bool,
	pub after: Option<Cursor>,
}
impl Default for Query {
	fn default() -> Self {
		Self {
			search: String::new(),
			sort: 1,
			recent: false,
			after: None,
		}
	}
}
impl Query {
	pub fn valid(&self) -> bool {
		self.search.capacity() <= 400
			&& self.search.chars().count() <= 100
			&& !self.search.chars().any(char::is_control)
			&& (1..=4).contains(&self.sort)
			&& self
				.after
				.is_none_or(|cursor| cursor.user.0 != 0 && cursor.joined_at >= 0)
	}
}
#[derive(Clone)]
pub enum Action {
	AuditLog(crate::server_audit_log::Query),
	Integrations(crate::server_integrations::Action),
	Invites(crate::server_invites::Action),
	Roles(crate::server_roles::Action),
	LoadEmojis,
	CreateEmoji { name: String, image: String },
	RenameEmoji { id: Id, name: String },
	DeleteEmoji { id: Id },
	LoadMembers(Query),
	SetRole { user: Id, role: Id, assigned: bool },
	SetNickname { user: Id, nick: String },
	Kick { user: Id },
	Prune { days: u8, execute: bool },
	ShowMembers { enabled: bool },
}
impl Action {
	pub fn write(&self) -> bool {
		if let Self::Integrations(action) = self {
			return action.write();
		}
		if let Self::Invites(action) = self {
			return action.write();
		}
		if let Self::Roles(action) = self {
			return action.write();
		}
		!matches!(
			self,
			Self::AuditLog(_)
				| Self::LoadEmojis
				| Self::LoadMembers(_)
				| Self::Prune { execute: false, .. }
		)
	}
	pub fn emoji(&self) -> bool {
		matches!(
			self,
			Self::LoadEmojis
				| Self::CreateEmoji { .. }
				| Self::RenameEmoji { .. }
				| Self::DeleteEmoji { .. }
		)
	}
	pub fn valid(&self) -> bool {
		match self {
			Self::Roles(action) => action.valid(),
			Self::Invites(action) => action.valid(),
			Self::Integrations(action) => action.valid(),
			Self::AuditLog(query) => query.valid(),
			Self::CreateEmoji { name, image } => {
				name.capacity() <= 128
					&& valid_emoji_name(name)
					&& image.capacity() <= MAX_IMAGE_URI
					&& (image.starts_with("data:image/png;base64,")
						|| image.starts_with("data:image/gif;base64,"))
			}
			Self::RenameEmoji { id, name } => {
				id.0 != 0 && name.capacity() <= 128 && valid_emoji_name(name)
			}
			Self::DeleteEmoji { id } => id.0 != 0,
			Self::LoadMembers(query) => query.valid(),
			Self::SetRole { user, role, .. } => user.0 != 0 && role.0 != 0,
			Self::SetNickname { user, nick } => {
				user.0 != 0
					&& nick.capacity() <= 128
					&& nick.chars().count() <= 32
					&& !nick.chars().any(char::is_control)
			}
			Self::Kick { user } => user.0 != 0,
			Self::Prune { days, .. } => (1..=30).contains(days),
			_ => true,
		}
	}
}
pub enum Result {
	WebhookUrl(crate::server_integrations::WebhookUrl),
	AuditLog(crate::server_audit_log::Page),
	Integrations(crate::server_integrations::Snapshot),
	Invites(crate::server_invites::Snapshot),
	Roles(crate::server_roles::Result),
	Emojis(Emojis),
	Members(Members),
	Member(Member),
	Kicked(Id),
	Pruned(Option<u64>),
	ChannelList(bool),
}
pub fn valid_emoji_name(name: &str) -> bool {
	(2..=32).contains(&name.len())
		&& name
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
impl Member {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.user.heap_bytes()
			+ self.nick.as_ref().map_or(0, String::capacity)
			+ self.roles.capacity() * size_of::<Id>()
			+ self.invite_code.as_ref().map_or(0, String::capacity)
	}
	pub fn valid(&self) -> bool {
		self.user.id.0 != 0
			&& self.user.heap_bytes() <= 1024
			&& self.roles.len() <= permissions::MAX_MEMBER_ROLES
			&& self.roles.iter().all(|id| id.0 != 0)
			&& self.nick.as_ref().is_none_or(|nick| nick.len() <= 128)
			&& self
				.invite_code
				.as_ref()
				.is_none_or(|value| value.len() <= 128)
			&& [self.joined_at, self.unusual_dm_until, self.timeout_until]
				.into_iter()
				.all(|value| value.is_none_or(|value| (0..=i128::from(i64::MAX)).contains(&value)))
			&& self.bytes() <= 8192
	}
}
impl Result {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ match self {
				Self::AuditLog(page) => page.bytes(),
				Self::WebhookUrl(url) => url.bytes(),
				Self::Integrations(snapshot) => snapshot.bytes(),
				Self::Roles(result) => result.bytes(),
				Self::Invites(snapshot) => snapshot.bytes(),
				Self::Emojis(page) => {
					page.items.capacity() * size_of::<Emoji>()
						+ page
							.items
							.iter()
							.map(|row| {
								row.emoji.heap_bytes()
									+ row.uploader.as_ref().map_or(0, User::heap_bytes)
							})
							.sum::<usize>()
				}
				Self::Members(page) => page.bytes(),
				Self::Member(member) => member.bytes(),
				_ => 0,
			}
	}
	pub fn valid(&self) -> bool {
		self.bytes() <= MAX_BYTES
			&& match self {
				Self::AuditLog(page) => page.valid_response(),
				Self::Integrations(snapshot) => snapshot.valid(),
				Self::Roles(result) => result.valid(),
				Self::Invites(snapshot) => snapshot.valid(),
				Self::Emojis(page) => {
					page.items.len() <= crate::MAX_GUILD_EMOJIS
						&& page.items.capacity() * size_of::<CustomEmoji>()
							+ page
								.items
								.iter()
								.map(|row| row.emoji.heap_bytes())
								.sum::<usize>() <= crate::MAX_GUILD_EMOJI_BYTES
						&& page.items.iter().all(|row| {
							row.emoji.valid()
								&& row
									.uploader
									.as_ref()
									.is_none_or(|user| user.id.0 != 0 && user.heap_bytes() <= 1024)
						})
				}
				Self::Members(page) => page.valid(),
				Self::Member(member) => member.valid(),
				Self::Kicked(id) => id.0 != 0,
				_ => true,
			}
	}
}

impl Members {
	pub fn bytes(&self) -> usize {
		self.items.iter().map(Member::bytes).sum::<usize>()
			+ self.items.capacity().saturating_sub(self.items.len()) * size_of::<Member>()
			+ self.roles.capacity() * size_of::<Role>()
			+ self
				.roles
				.iter()
				.map(|role| role.role.name.capacity())
				.sum::<usize>()
			+ self.features.capacity() * size_of::<String>()
			+ self.features.iter().map(String::capacity).sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		self.bytes() <= MAX_BYTES
			&& self.items.len() <= PAGE_SIZE
			&& self.items.iter().all(Member::valid)
			&& self.roles.len() <= permissions::MAX_ROLES
			&& self
				.roles
				.iter()
				.all(|role| role.role.id.0 != 0 && role.role.name.len() <= 400)
			&& self.features.len() <= 256
			&& self.features.iter().all(|value| value.len() <= 128)
	}
}
