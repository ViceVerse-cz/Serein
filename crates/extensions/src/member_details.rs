use crate::UserSnapshot;
use serde::{Deserialize, Serialize};

pub const MAX_MEMBER_DETAILS: usize = 20;
pub const MAX_MEMBER_DETAIL_ROLES: usize = 32;
pub const MAX_MEMBER_ROLE_CATALOG: usize = 32;
pub const MAX_MEMBER_DETAILS_BYTES: usize = 6 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct MemberRoleSnapshot {
	pub id: String,
	pub name: String,
	pub color: u32,
	pub position: i32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

use crate::{
	Error,
	app::{bounded_bytes, ids, image_hash, label, user},
};
fn text(value: &str, limit: usize, multiline: bool) -> Result<(), Error> {
	if value.len() > limit {
		return Err(Error::Limit);
	}
	if value
		.chars()
		.any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
	{
		return Err(Error::Invalid);
	}
	Ok(())
}
impl MemberDetailsSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		crate::app::entity_id(&self.channel_id)?;
		crate::app::entity_id(&self.guild_id)?;
		ids(
			self.items.iter().map(|m| m.user.id.as_str()),
			MAX_MEMBER_DETAILS,
		)?;
		for member in &self.items {
			user(&member.user)?;
			label(&member.display_name, 256)?;
			if let Some(nick) = &member.nick {
				text(nick, 256, false)?;
			}
			ids(
				member.role_ids.iter().map(String::as_str),
				MAX_MEMBER_DETAIL_ROLES,
			)?;
			if let Some(profile) = &member.profile {
				if let Some(nick) = &profile.nick {
					text(nick, 256, false)?;
				}
				text(&profile.bio, 1024, true)?;
				text(&profile.pronouns, 256, false)?;
				if let Some(joined) = &profile.joined_at {
					text(joined, 64, false)?;
				}
				if let Some(avatar) = &profile.avatar {
					image_hash(avatar)?;
				}
			}
		}
		if let Some(roles) = &self.roles {
			ids(roles.iter().map(|r| r.id.as_str()), MAX_MEMBER_ROLE_CATALOG)?;
			for role in roles {
				if role.color > 0xffffff {
					return Err(Error::Invalid);
				}
				label(&role.name, 256)?;
			}
		}
		bounded_bytes(self, MAX_MEMBER_DETAILS_BYTES)?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn member_details_validate_bounds_and_sdk_wire() {
		let mut value = MemberDetailsSnapshot {
			channel_id: "1".into(),
			guild_id: "2".into(),
			items: vec![MemberDetailSnapshot {
				user: UserSnapshot {
					id: "3".into(),
					name: "User".into(),
				},
				nick: Some("Nick".into()),
				display_name: "Nick".into(),
				role_ids: vec!["4".into()],
				roles_truncated: false,
				profile: Some(MemberProfileSnapshot {
					nick: None,
					avatar: Some("a_abc".into()),
					bio: "bio\ntext".into(),
					pronouns: "they/them".into(),
					joined_at: Some("2026-01-01T00:00:00Z".into()),
				}),
			}],
			truncated: false,
			roles: Some(vec![MemberRoleSnapshot {
				id: "4".into(),
				name: "Role".into(),
				color: 0x123456,
				position: 1,
			}]),
			roles_truncated: false,
		};
		value.validate().unwrap();
		let wire = serde_json::to_value(&value).unwrap();
		let sdk: serein_extension_sdk::MemberDetailsSnapshot =
			serde_json::from_value(wire.clone()).unwrap();
		assert_eq!(serde_json::to_value(sdk).unwrap(), wire);
		value.items[0].role_ids = vec!["4".into(); 2];
		assert!(matches!(value.validate(), Err(Error::Invalid)));
		value.items[0].role_ids = (1..=MAX_MEMBER_DETAIL_ROLES + 1)
			.map(|id| id.to_string())
			.collect();
		assert!(matches!(value.validate(), Err(Error::Limit)));
		value.items[0].role_ids.clear();
		value.items[0].profile.as_mut().unwrap().bio = "x".repeat(1025);
		assert!(matches!(value.validate(), Err(Error::Limit)));
	}
}
