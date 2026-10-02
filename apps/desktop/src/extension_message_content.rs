//! Bounded summaries of already-loaded rich messages; no media fetches or raw service payloads.
use crate::extension_app::{name, push, text};
use client_core::{State, auth::AuthState};
use extensions::*;
use model::Freshness;

fn embed_summary(source: &model::Embed) -> EmbedSummarySnapshot {
	let mut result = EmbedSummarySnapshot {
		kind: text(&source.kind, 32),
		title: source.title.as_deref().map(|s| text(s, 256)),
		description: source.description.as_deref().map(|s| text(s, 512)),
		author: source.author.as_ref().map(|a| text(&a.name, 128)),
		footer: source.footer.as_ref().map(|f| text(&f.text, 256)),
		color: source.color.map(|c| c & 0xffffff),
		fields: Vec::new(),
		fields_truncated: false,
		has_image: source.image.is_some(),
		has_thumbnail: source.thumbnail.is_some(),
		has_video: source.video.is_some(),
		limited: source.limited,
	};
	result.limited |= source.kind.len() > 32
		|| source.title.as_ref().is_some_and(|s| s.len() > 256)
		|| source.description.as_ref().is_some_and(|s| s.len() > 512)
		|| source.author.as_ref().is_some_and(|a| a.name.len() > 128)
		|| source.footer.as_ref().is_some_and(|f| f.text.len() > 256);
	let mut budget = 768;
	for field in &source.fields {
		result.limited |= field.name.len() > 128 || field.value.len() > 256;
		if !push(
			&mut result.fields,
			EmbedFieldSnapshot {
				name: text(&field.name, 128),
				value: text(&field.value, 256),
				inline: field.inline,
			},
			&mut budget,
			MAX_MESSAGE_EMBED_FIELDS,
		) {
			result.fields_truncated = true;
			break;
		}
	}
	result
}
fn message_summary(message: &model::Message) -> RichMessageSnapshot {
	let mut result = RichMessageSnapshot {
		id: message.id.0.to_string(),
		embeds: Vec::new(),
		embeds_truncated: false,
		embeds_suppressed: message.embeds_suppressed,
		stickers: Vec::new(),
		stickers_truncated: false,
		reference: (message.reply_to.is_some_and(|id| id.0 != 0)
			|| message.reply_deleted
			|| message.forwarded)
			.then(|| MessageReferenceSnapshot {
				message_id: message
					.reply_to
					.filter(|id| id.0 != 0)
					.map(|id| id.0.to_string()),
				deleted: message.reply_deleted,
				forwarded: message.forwarded,
			}),
		poll: if message.extra_content.poll {
			PollAvailability::Unsupported
		} else {
			PollAvailability::Absent
		},
	};
	let mut budget = 2 * 1024;
	for sticker in &message.sticker_items {
		if sticker.id.0 == 0 {
			result.stickers_truncated = true;
			continue;
		}
		let id = sticker.id.0.to_string();
		if result.stickers.iter().any(|s| s.id == id) {
			continue;
		}
		if !push(
			&mut result.stickers,
			MessageStickerSnapshot {
				id,
				name: name(&sticker.name),
				format_type: sticker.format_type,
			},
			&mut budget,
			MAX_CONTENT_STICKERS,
		) {
			result.stickers_truncated = true;
			break;
		}
	}
	result.stickers_truncated |= message.extra_content.stickers
		|| (message.extra_content.sticker_items && message.sticker_items.is_empty());
	for embed in &message.embeds {
		let value = embed_summary(embed);
		let storage = value.fields.capacity() * std::mem::size_of::<EmbedFieldSnapshot>();
		if storage > budget {
			result.embeds_truncated = true;
			break;
		}
		budget -= storage;
		if !push(&mut result.embeds, value, &mut budget, MAX_MESSAGE_EMBEDS) {
			result.embeds_truncated = true;
			break;
		}
	}
	result
}
pub fn snapshot(state: &State) -> Option<MessageContentSnapshot> {
	if !(state.demo || state.auth == AuthState::Authenticated)
		|| !state.gateway_connected
		|| state.freshness != Freshness::Fresh
	{
		return None;
	}
	let channel = state.selected?;
	if !state.can_view(channel)
		|| !state.can_read_history(channel)
		|| !state.channel(channel)?.supports_text()
	{
		return None;
	}
	let mut result = MessageContentSnapshot {
		channel_id: channel.0.to_string(),
		items: Vec::new(),
		truncated: state.history_before.is_some()
			|| state.history_after.is_some()
			|| !state.older_exhausted,
	};
	let mut budget = MAX_MESSAGE_CONTENT_BYTES - 512;
	for message in state.timeline.iter().rev().filter(|m| {
		m.channel == channel
			&& m.id.0 != 0
			&& m.author.id.0 != 0
			&& !m.ephemeral
			&& m.flags & 64 == 0
	}) {
		let item = message_summary(message);
		let storage = item.embeds.capacity() * std::mem::size_of::<EmbedSummarySnapshot>()
			+ item.stickers.capacity() * std::mem::size_of::<MessageStickerSnapshot>()
			+ item
				.embeds
				.iter()
				.map(|e| e.fields.capacity() * std::mem::size_of::<EmbedFieldSnapshot>())
				.sum::<usize>();
		if storage > budget {
			result.truncated = true;
			break;
		}
		budget -= storage;
		if !push(&mut result.items, item, &mut budget, MAX_RICH_MESSAGES) {
			result.truncated = true;
			break;
		}
	}
	result.items.reverse();
	result.validate().ok()?;
	Some(result)
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn rich_summaries_are_bounded_and_omit_media_payloads() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let mut message = test_support::message(90000, channel);
		message.reply_to = Some(model::Id(89999));
		message.extra_content.poll = true;
		message.embeds = vec![
			model::Embed {
				kind: "rich".into(),
				title: Some("Title".into()),
				description: Some("description".repeat(200)),
				url: Some("PRIVATE_URL".into()),
				image: Some(model::EmbedMedia {
					url: Some("PRIVATE_IMAGE".into()),
					..Default::default()
				}),
				fields: vec![
					model::EmbedField {
						name: "Field".into(),
						value: "Value".into(),
						inline: false
					};
					6
				],
				..Default::default()
			};
			5
		];
		message.sticker_items = vec![model::Sticker {
			id: model::Id(77),
			name: "Wave".into(),
			description: "PRIVATE_STICKER_DESCRIPTION".into(),
			tags: "PRIVATE_TAG".into(),
			format_type: 1,
			guild_id: None,
			pack_id: None,
			available: true,
		}];
		let item = message_summary(&message);
		assert_eq!(item.poll, PollAvailability::Unsupported);
		assert!(item.embeds_truncated);
		assert!(item.embeds.iter().all(|e| e.limited && e.fields_truncated));
		assert_eq!(
			item.reference.as_ref().unwrap().message_id.as_deref(),
			Some("89999")
		);
		assert!(!serde_json::to_string(&item).unwrap().contains("PRIVATE_"));
		state
			.timeline
			.insert(message.clone(), false, false)
			.unwrap();
		message.id = model::Id(90001);
		message.ephemeral = true;
		state.timeline.insert(message, false, false).unwrap();
		let value = snapshot(&state).unwrap();
		assert!(value.items.iter().any(|m| m.id == "90000"));
		assert!(!value.items.iter().any(|m| m.id == "90001"));
		assert!(serde_json::to_vec(&value).unwrap().len() <= MAX_MESSAGE_CONTENT_BYTES);
		value.validate().unwrap();
	}
}
