use crate::UserSnapshot;
use serde::{Deserialize, Serialize};

pub const MAX_MEMBER_DETAILS: usize = 20;
pub const MAX_MEMBER_DETAIL_ROLES: usize = 32;
pub const MAX_MEMBER_ROLE_CATALOG: usize = 32;
pub const MAX_MEMBER_DETAILS_BYTES: usize = 6 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberDetailsSnapshot {
	pub channel_id: String,
	pub guild_id: String,
	pub items: Vec<MemberDetailSnapshot>,
	pub truncated: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub roles: Option<Vec<MemberRoleSnapshot>>,
	pub roles_truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberDetailSnapshot {
	pub user: UserSnapshot,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub nick: Option<String>,
	pub display_name: String,
	pub role_ids: Vec<String>,
	pub roles_truncated: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub profile: Option<MemberProfileSnapshot>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRoleSnapshot {
	pub id: String,
	pub name: String,
	pub color: u32,
	pub position: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberProfileSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub nick: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub avatar: Option<String>,
	pub bio: String,
	pub pronouns: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub joined_at: Option<String>,
}
