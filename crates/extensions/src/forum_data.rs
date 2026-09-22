use serde::{Deserialize, Serialize};

pub const MAX_FORUM_POSTS: usize = 10;
pub const MAX_FORUM_DATA_BYTES: usize = 6 * 1024;

/// Already-loaded, accessible threads under the selected parent. Never a complete service listing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForumDataSnapshot {
	pub channel_id: String,
	pub guild_id: String,
	pub parent_id: String,
	pub posts: Vec<ForumPostSnapshot>,
	pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForumPostSnapshot {
	pub id: String,
	pub name: String,
	pub kind: u8,
	pub message_count: Option<u32>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub owner_id: Option<String>,
	pub archived: Option<bool>,
	pub locked: Option<bool>,
	pub pinned: Option<bool>,
	/// Loaded follow state, not a claim about private-thread membership.
	pub followed: Option<bool>,
}

impl ForumDataSnapshot {
	pub fn validate(&self) -> Result<(), crate::Error> {
		use crate::app::{bounded_bytes, entity_id, ids, label};
		entity_id(&self.channel_id)?;
		entity_id(&self.guild_id)?;
		entity_id(&self.parent_id)?;
		ids(
			self.posts.iter().map(|post| post.id.as_str()),
			MAX_FORUM_POSTS,
		)?;
		for post in &self.posts {
			if post.id == self.parent_id || !matches!(post.kind, 10..=12) {
				return Err(crate::Error::Invalid);
			}
			label(&post.name, 128)?;
			if let Some(owner) = &post.owner_id {
				entity_id(owner)?;
			}
		}
		bounded_bytes(self, MAX_FORUM_DATA_BYTES)?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn forum_wire_is_bounded_and_matches_sdk() {
		let mut value = ForumDataSnapshot {
			channel_id: "1".into(),
			guild_id: "2".into(),
			parent_id: "1".into(),
			truncated: false,
			posts: vec![ForumPostSnapshot {
				id: "3".into(),
				name: "Loaded post".into(),
				kind: 11,
				message_count: Some(5),
				owner_id: None,
				archived: None,
				locked: None,
				pinned: None,
				followed: None,
			}],
		};
		value.validate().unwrap();
		let wire = serde_json::to_value(&value).unwrap();
		let sdk: serein_extension_sdk::ForumDataSnapshot =
			serde_json::from_value(wire.clone()).unwrap();
		assert_eq!(serde_json::to_value(sdk).unwrap(), wire);
		value.posts[0].name = "x".repeat(129);
		assert!(value.validate().is_err());
		value.posts[0].name = "valid".into();
		value.posts.push(value.posts[0].clone());
		assert!(value.validate().is_err());
	}
}
