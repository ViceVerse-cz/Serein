use serde::{Deserialize, Serialize};
pub const MAX_MESSAGE_CONTENT_BYTES: usize = 8 * 1024;
pub const MAX_RICH_MESSAGES: usize = 10;
pub const MAX_MESSAGE_EMBEDS: usize = 3;
pub const MAX_MESSAGE_EMBED_FIELDS: usize = 4;
pub const MAX_CONTENT_STICKERS: usize = 3;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageContentSnapshot {
	pub channel_id: String,
	pub items: Vec<RichMessageSnapshot>,
	pub truncated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RichMessageSnapshot {
	pub id: String,
	pub embeds: Vec<EmbedSummarySnapshot>,
	pub embeds_truncated: bool,
	pub embeds_suppressed: bool,
	pub stickers: Vec<MessageStickerSnapshot>,
	pub stickers_truncated: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub reference: Option<MessageReferenceSnapshot>,
	pub poll: PollAvailability,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedSummarySnapshot {
	pub kind: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub author: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub footer: Option<String>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub color: Option<u32>,
	pub fields: Vec<EmbedFieldSnapshot>,
	pub fields_truncated: bool,
	pub has_image: bool,
	pub has_thumbnail: bool,
	pub has_video: bool,
	pub limited: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbedFieldSnapshot {
	pub name: String,
	pub value: String,
	pub inline: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageStickerSnapshot {
	pub id: String,
	pub name: String,
	pub format_type: u8,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageReferenceSnapshot {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub message_id: Option<String>,
	pub deleted: bool,
	pub forwarded: bool,
}
/// Poll questions/options/results are not retained by the client protocol decoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PollAvailability {
	Absent,
	Unsupported,
}

use crate::{
	Error,
	app::{bounded_bytes, entity_id, ids, label},
};
fn text(value: &str, limit: usize) -> Result<(), Error> {
	if value.len() > limit {
		return Err(Error::Limit);
	}
	if value
		.chars()
		.any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
	{
		return Err(Error::Invalid);
	}
	Ok(())
}
impl MessageContentSnapshot {
	pub fn validate(&self) -> Result<(), Error> {
		entity_id(&self.channel_id)?;
		ids(self.items.iter().map(|m| m.id.as_str()), MAX_RICH_MESSAGES)?;
		for message in &self.items {
			if message.embeds.len() > MAX_MESSAGE_EMBEDS {
				return Err(Error::Limit);
			}
			for embed in &message.embeds {
				text(&embed.kind, 32)?;
				for (value, limit) in [
					(&embed.title, 256),
					(&embed.description, 512),
					(&embed.author, 128),
					(&embed.footer, 256),
				] {
					if let Some(value) = value {
						text(value, limit)?;
					}
				}
				if embed.color.is_some_and(|color| color > 0xffffff) {
					return Err(Error::Invalid);
				}
				if embed.fields.len() > MAX_MESSAGE_EMBED_FIELDS {
					return Err(Error::Limit);
				}
				for field in &embed.fields {
					text(&field.name, 128)?;
					text(&field.value, 256)?;
				}
			}
			ids(
				message.stickers.iter().map(|s| s.id.as_str()),
				MAX_CONTENT_STICKERS,
			)?;
			for sticker in &message.stickers {
				label(&sticker.name, 128)?;
			}
			if let Some(reference) = &message.reference {
				if let Some(id) = &reference.message_id {
					entity_id(id)?;
				}
				if reference.message_id.is_none() && !reference.deleted && !reference.forwarded {
					return Err(Error::Invalid);
				}
			}
		}
		bounded_bytes(self, MAX_MESSAGE_CONTENT_BYTES)?;
		Ok(())
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn rich_message_contract_bounds_and_sdk_parity() {
		let mut snapshot = MessageContentSnapshot {
			channel_id: "1".into(),
			items: vec![RichMessageSnapshot {
				id: "2".into(),
				embeds: vec![EmbedSummarySnapshot {
					kind: "rich".into(),
					title: Some("Title".into()),
					description: None,
					author: None,
					footer: None,
					color: Some(0xffffff),
					fields: vec![],
					fields_truncated: false,
					has_image: true,
					has_thumbnail: false,
					has_video: false,
					limited: false,
				}],
				embeds_truncated: false,
				embeds_suppressed: false,
				stickers: vec![MessageStickerSnapshot {
					id: "3".into(),
					name: "Wave".into(),
					format_type: 1,
				}],
				stickers_truncated: false,
				reference: Some(MessageReferenceSnapshot {
					message_id: Some("4".into()),
					deleted: false,
					forwarded: false,
				}),
				poll: PollAvailability::Unsupported,
			}],
			truncated: false,
		};
		snapshot.validate().unwrap();
		let wire = serde_json::to_value(&snapshot).unwrap();
		let sdk: serein_extension_sdk::MessageContentSnapshot =
			serde_json::from_value(wire.clone()).unwrap();
		assert_eq!(serde_json::to_value(sdk).unwrap(), wire);
		snapshot.items[0].embeds[0].description = Some("x".repeat(513));
		assert!(matches!(snapshot.validate(), Err(Error::Limit)));
		snapshot.items[0].embeds[0].description = None;
		let embed = snapshot.items[0].embeds[0].clone();
		snapshot.items[0].embeds = vec![embed; MAX_MESSAGE_EMBEDS + 1];
		assert!(matches!(snapshot.validate(), Err(Error::Limit)));
	}
}
