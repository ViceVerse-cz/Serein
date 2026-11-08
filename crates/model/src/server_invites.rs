//! Bounded guild invite administration; absent service metadata stays unknown.
use crate::{Id, User};
use serde::{Deserialize, Serialize};
pub const MAX_ITEMS: usize = 1000;
pub const MAX_BYTES: usize = 1024 * 1024;
pub const PAUSED_FEATURE: &str = "INVITES_DISABLED";
/// Device-local defaults for newly created invites. The service still enforces the
/// account's per-guild permissions and its own limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InviteDefaults {
	pub max_age: u32,
	pub max_uses: u16,
	pub temporary: bool,
}
impl Default for InviteDefaults {
	fn default() -> Self {
		Self {
			max_age: 30 * 86400,
			max_uses: 0,
			temporary: false,
		}
	}
}
impl InviteDefaults {
	pub fn valid(self) -> bool {
		// Normal-user 30-day invites exceed the public developer API's documented 7 days.
		self.max_age <= 30 * 86400 && self.max_uses <= 100
	}
}
#[derive(Clone)]
pub struct Invite {
	pub code: String,
	pub inviter: Option<User>,
	pub channel: Option<Id>,
	pub channel_name: Option<String>,
	pub uses: Option<u64>,
	pub max_uses: Option<u64>,
	pub max_age: Option<u64>,
	pub created_at: Option<i128>,
	pub expires_at: Option<i128>,
	pub temporary: Option<bool>,
	pub roles: Option<Vec<Id>>,
}
impl Invite {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.code.capacity()
			+ self.inviter.as_ref().map_or(0, User::heap_bytes)
			+ self.channel_name.as_ref().map_or(0, String::capacity)
			+ self
				.roles
				.as_ref()
				.map_or(0, |roles| roles.capacity() * size_of::<Id>())
	}
	pub fn valid(&self) -> bool {
		valid_code(&self.code)
			&& self.bytes() <= 2048
			&& self.channel.is_none_or(|id| id.0 != 0)
			&& self
				.roles
				.as_ref()
				.is_none_or(|roles| roles.len() <= 100 && roles.iter().all(|role| role.0 != 0))
			&& self
				.inviter
				.as_ref()
				.is_none_or(|user| user.id.0 != 0 && user.heap_bytes() <= 1024)
			&& self
				.channel_name
				.as_ref()
				.is_none_or(|value| value.len() <= 400 && !value.chars().any(char::is_control))
			&& [self.created_at, self.expires_at].into_iter().all(|value| {
				value.is_none_or(|value| (0..=253_402_300_799_999_999_999_i128).contains(&value))
			})
	}
}
#[derive(Clone)]
pub struct Snapshot {
	pub guild: Id,
	pub items: Vec<Invite>,
	pub features: Vec<String>,
}
impl Snapshot {
	pub fn paused(&self) -> bool {
		self.features
			.iter()
			.any(|feature| feature == PAUSED_FEATURE)
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.items.iter().map(Invite::bytes).sum::<usize>()
			+ self.items.capacity().saturating_sub(self.items.len()) * size_of::<Invite>()
			+ self.features.capacity() * size_of::<String>()
			+ self.features.iter().map(String::capacity).sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		let mut seen = std::collections::BTreeSet::new();
		self.guild.0 != 0
			&& self.items.len() <= MAX_ITEMS
			&& self.bytes() <= MAX_BYTES
			&& self
				.items
				.iter()
				.all(|invite| invite.valid() && seen.insert(&invite.code))
			&& valid_features(&self.features)
	}
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
	Load,
	Revoke { code: String },
	SetPaused { paused: bool },
}
impl Action {
	pub fn write(&self) -> bool {
		!matches!(self, Self::Load)
	}
	pub fn valid(&self) -> bool {
		match self {
			Self::Revoke { code } => code.capacity() <= 100 && valid_code(code),
			_ => true,
		}
	}
	pub fn normalize(&mut self) {
		if let Self::Revoke { code } = self {
			code.shrink_to_fit();
		}
	}
}
pub fn valid_code(code: &str) -> bool {
	!code.is_empty()
		&& code.len() <= 100
		&& code
			.bytes()
			.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}
pub fn valid_features(features: &[String]) -> bool {
	features.len() <= 256
		&& features.iter().all(|feature| {
			!feature.is_empty()
				&& feature.len() <= 128
				&& feature
					.bytes()
					.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
		})
}
