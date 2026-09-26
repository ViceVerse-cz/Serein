//! Active forum posts from the thread search route; archived rows belong to the archive page.
use crate::ChannelDto;
use model::{
	Id, Patch,
	forum::{Layout, MAX_APPLIED_TAGS, MAX_TAG_NAME, MAX_TAGS, Page, Sort, Starter, Tag, Tags},
};
use serde::{Deserialize, Deserializer, de::Visitor};

/// Forum channel flag: every new post must carry at least one tag.
const REQUIRE_TAG: u64 = 1 << 4;

/// Keeps the first `N` entries of a list and skips the rest, so an oversized tag list never
/// rejects the snapshot carrying it; null reads as empty.
fn capped<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
	d: D,
) -> Result<Vec<T>, D::Error> {
	struct Capped<T, const N: usize>(std::marker::PhantomData<T>);
	impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Capped<T, N> {
		type Value = Vec<T>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("a list of forum tags")
		}
		fn visit_unit<E>(self) -> Result<Self::Value, E> {
			Ok(Vec::new())
		}
		fn visit_none<E>(self) -> Result<Self::Value, E> {
			Ok(Vec::new())
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(
			self,
			mut seq: A,
		) -> Result<Self::Value, A::Error> {
			let mut items = Vec::new();
			while items.len() < N {
				match seq.next_element()? {
					Some(item) => items.push(item),
					None => return Ok(items),
				}
			}
			while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
			Ok(items)
		}
	}
	d.deserialize_any(Capped::<T, N>(std::marker::PhantomData))
}

#[derive(Deserialize)]
struct TagDto {
	id: Id,
	#[serde(default)]
	name: String,
	#[serde(default)]
	moderated: bool,
	#[serde(default)]
	emoji_id: Option<Id>,
	#[serde(default)]
	emoji_name: Option<String>,
}

/// A forum's `available_tags`, bounded while decoding.
pub struct TagList(Vec<TagDto>);
impl<'de> Deserialize<'de> for TagList {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		capped::<_, _, MAX_TAGS>(d).map(Self)
	}
}

/// A post's `applied_tags`, bounded while decoding.
pub struct AppliedTags(Vec<Id>);
impl<'de> Deserialize<'de> for AppliedTags {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		capped::<_, _, MAX_APPLIED_TAGS>(d).map(Self)
	}
}

/// A forum's `default_reaction_emoji`.
#[derive(Deserialize)]
pub struct DefaultReaction {
	#[serde(default)]
	emoji_id: Option<Id>,
	#[serde(default)]
	emoji_name: Option<String>,
}
impl DefaultReaction {
	fn into_model(self) -> Option<model::ReactionEmoji> {
		let emoji = model::ReactionEmoji {
			id: self.emoji_id.filter(|id| id.0 > 0),
			name: self
				.emoji_name
				.map(|name| name.chars().take(32).collect::<String>())
				.filter(|name| !name.is_empty()),
		};
		emoji.valid().then_some(emoji)
	}
}

/// A forum's post defaults as they arrive on the wire.
#[derive(Default)]
pub(crate) struct Defaults {
	pub reaction: Option<DefaultReaction>,
	pub layout: Option<u8>,
	pub sort: Option<u8>,
	pub tag_setting: Option<String>,
}

/// Reduce wire tags to the model: containers keep what they offer, posts what they apply.
pub(crate) fn tags(
	kind: u8,
	available: Option<TagList>,
	applied: Option<AppliedTags>,
	flags: u64,
	defaults: Defaults,
) -> Option<Box<Tags>> {
	let mut tags = Tags::default();
	if matches!(kind, 15 | 16) {
		tags.reaction = defaults.reaction.and_then(DefaultReaction::into_model);
		tags.layout = if defaults.layout == Some(2) {
			Layout::Gallery
		} else {
			Layout::List
		};
		tags.sort = if defaults.sort == Some(1) {
			Sort::Created
		} else {
			Sort::Activity
		};
		tags.match_all = defaults.tag_setting.as_deref() == Some("match_all");
		for tag in available.map(|list| list.0).unwrap_or_default() {
			if tag.id.0 == 0 || tags.available.iter().any(|known| known.id == tag.id) {
				continue;
			}
			tags.available.push(Tag {
				id: tag.id,
				name: tag.name.trim().chars().take(MAX_TAG_NAME).collect(),
				moderated: tag.moderated,
				emoji_id: tag.emoji_id.filter(|id| id.0 > 0),
				emoji_name: tag
					.emoji_name
					.map(|name| name.chars().take(32).collect::<String>())
					.filter(|name| !name.is_empty()),
			});
		}
		tags.required = flags & REQUIRE_TAG != 0;
	} else if matches!(kind, 10..=12) {
		for id in applied.map(|list| list.0).unwrap_or_default() {
			if id.0 > 0 && !tags.applied.contains(&id) {
				tags.applied.push(id);
			}
		}
	}
	(!tags.is_empty()).then(|| Box::new(tags))
}

/// Channel updates carry whole objects; any tag-bearing field replaces the known tags.
pub(crate) fn patched_tags(
	kind: Patch<u8>,
	available: Patch<TagList>,
	applied: Patch<AppliedTags>,
	flags: &Patch<u64>,
	defaults: Defaults,
) -> Patch<Box<Tags>> {
	let Patch::Value(kind) = kind else {
		return Patch::Absent;
	};
	if matches!(available, Patch::Absent)
		&& matches!(applied, Patch::Absent)
		&& !matches!(flags, Patch::Value(_))
	{
		return Patch::Absent;
	}
	fn value<T>(patch: Patch<T>) -> Option<T> {
		match patch {
			Patch::Value(value) => Some(value),
			_ => None,
		}
	}
	let flags = match flags {
		Patch::Value(flags) => *flags,
		_ => 0,
	};
	tags(kind, value(available), value(applied), flags, defaults).map_or(Patch::Null, Patch::Value)
}

/// What a card shows of a starter: its first image (an attachment first, then an embed's
/// artwork) and its reactions.
fn preview(message: crate::MessageDto) -> Option<(Id, Starter)> {
	let message = message.into_model();
	let image = message
		.attachments
		.into_iter()
		.find(|attachment| attachment.is_image() && !attachment.spoiler)
		.map(|attachment| attachment.media)
		.or_else(|| {
			message
				.embeds
				.into_iter()
				.find_map(|embed| embed.image.or(embed.thumbnail))
		})
		.filter(|media| media.valid() && (media.url.is_some() || media.proxy_url.is_some()));
	let mut reactions = message.reactions.unwrap_or_default();
	reactions.sort_by_key(|reaction| std::cmp::Reverse(reaction.count));
	reactions.truncate(20);
	let starter = Starter { image, reactions };
	(!starter.is_empty() && starter.valid()).then_some((message.channel, starter))
}
pub const MAX_WIRE: usize = 512 * 1024;
/// The guild-wide fallback lists every visible thread, so it needs the snapshot budget.
pub const GUILD_MAX_WIRE: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
pub struct Reply {
	#[serde(deserialize_with = "crate::search::list::<_,_,25>")]
	threads: Vec<Thread>,
	#[serde(default)]
	has_more: bool,
	/// Unofficial: the starter message of each listed post, used for its card preview.
	#[serde(default, deserialize_with = "capped::<_,_,25>")]
	first_messages: Vec<crate::MessageDto>,
}
#[derive(Deserialize)]
struct Thread {
	#[serde(flatten)]
	channel: ChannelDto,
	#[serde(default)]
	thread_metadata: Option<Metadata>,
}
#[derive(Deserialize)]
struct Metadata {
	#[serde(default)]
	archived: bool,
}

impl Reply {
	pub fn into_page(self, parent: Id, guild: Id) -> Result<Page, &'static str> {
		let invalid = "Invalid forum post page";
		let mut threads = Vec::with_capacity(self.threads.len());
		for thread in self.threads {
			// An archived row here would duplicate the archive page and mislead the post list.
			if thread
				.thread_metadata
				.is_some_and(|metadata| metadata.archived)
			{
				return Err(invalid);
			}
			threads.push(crate::threads::into_thread(thread.channel, guild).map_err(|_| invalid)?);
		}
		let mut previews: Vec<(Id, Starter)> = Vec::new();
		for (id, starter) in self.first_messages.into_iter().filter_map(preview) {
			if threads.iter().any(|thread| thread.id == id)
				&& !previews.iter().any(|(known, _)| *known == id)
			{
				previews.push((id, starter));
			}
		}
		let page = Page {
			threads,
			more: self.has_more,
			previews,
		};
		if !page.valid(parent, guild) {
			return Err(invalid);
		}
		Ok(page)
	}
}

/// The documented guild-wide active list, used when the per-forum search route is unavailable.
#[derive(Deserialize)]
pub struct GuildActive {
	#[serde(deserialize_with = "crate::threads::list")]
	threads: Vec<ChannelDto>,
}

impl GuildActive {
	pub fn into_page(self, parent: Id, guild: Id) -> Result<Page, &'static str> {
		let invalid = "Invalid forum post page";
		let mut threads = Vec::new();
		for thread in self.threads {
			if thread.parent_id != Some(parent) {
				continue;
			}
			threads.push(crate::threads::into_thread(thread, guild).map_err(|_| invalid)?);
		}
		threads.sort_by_key(|thread| std::cmp::Reverse(thread.last_message.unwrap_or(thread.id)));
		threads.truncate(model::forum::PAGE_SIZE);
		threads.shrink_to_fit();
		let page = Page {
			threads,
			more: false,
			previews: Vec::new(),
		};
		if !page.valid(parent, guild) {
			return Err(invalid);
		}
		Ok(page)
	}
}

/// The existing history route, reduced before entering the UI event queue.
#[derive(Deserialize)]
pub struct Recent(
	#[serde(deserialize_with = "crate::search::list::<_,_,50>")] Vec<crate::MessageDto>,
);
impl Recent {
	pub fn into_summary(self, channel: Id) -> Result<model::forum::Summary, &'static str> {
		let mut messages: Vec<_> = self
			.0
			.into_iter()
			.map(crate::MessageDto::into_model)
			.collect();
		if messages.iter().any(|message| message.channel != channel) {
			return Err("Invalid forum preview");
		}
		messages.sort_unstable_by_key(|message| std::cmp::Reverse(message.id));
		let latest = messages.first().map(|message| model::forum::Latest {
			id: message.id,
			channel: message.channel,
			author_id: message.author.id,
			author: message
				.author_nick
				.clone()
				.unwrap_or_else(|| message.author.name.clone()),
			roles: message.author_roles.clone(),
			webhook: message.author.webhook,
			excerpt: if message.content.contains("||") {
				"Spoiler content - open message to reveal".into()
			} else {
				message.content.chars().take(256).collect()
			},
		});
		let summary = model::forum::Summary {
			complete: messages.len() < 50,
			messages: messages.iter().map(|message| message.id).collect(),
			latest,
		};
		if !summary.valid(channel) {
			return Err("Invalid forum preview");
		}
		Ok(summary)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;
	#[test]
	fn forum_pages_validate_scope_state_and_capacity() {
		let thread = |id: u64, kind: u8, archived: bool| json!({"id":id.to_string(),"guild_id":"1","parent_id":"2","type":kind,"name":"Synthetic","thread_metadata":{"archived":archived}});
		let decode = |threads, more| {
			crate::decode::<Reply>(
				&serde_json::to_vec(
					&json!({"threads":threads,"has_more":more,"members":[{"private":"ignored"}],"total_results":2}),
				)
				.unwrap(),
			)
		};
		let page = decode(vec![thread(3, 11, false), thread(9, 11, false)], true)
			.unwrap()
			.into_page(Id(2), Id(1))
			.unwrap();
		assert_eq!(
			page.threads.iter().map(|t| t.id).collect::<Vec<_>>(),
			vec![Id(3), Id(9)]
		);
		assert!(page.more);
		// A missing metadata block still lists; only an archived row is rejected.
		let mut bare = thread(3, 11, false);
		bare["thread_metadata"] = json!(null);
		assert!(
			decode(vec![bare], false)
				.unwrap()
				.into_page(Id(2), Id(1))
				.is_ok()
		);
		assert!(
			decode(vec![thread(3, 11, true)], false)
				.unwrap()
				.into_page(Id(2), Id(1))
				.is_err()
		);
		assert!(
			decode(Vec::<serde_json::Value>::new(), true)
				.unwrap()
				.into_page(Id(2), Id(1))
				.is_err()
		);
		for (key, value) in [
			("guild_id", json!("7")),
			("parent_id", json!("7")),
			("type", json!(12)),
		] {
			let mut invalid = thread(3, 11, false);
			invalid[key] = value;
			assert!(
				decode(vec![invalid], false)
					.unwrap()
					.into_page(Id(2), Id(1))
					.is_err()
			);
		}
		// The guild-wide fallback keeps only this forum's newest rows.
		let guild_active = |threads| {
			crate::decode::<GuildActive>(
				&serde_json::to_vec(&json!({ "threads": threads, "members": [] })).unwrap(),
			)
			.unwrap()
		};
		let mut elsewhere = thread(4, 11, false);
		elsewhere["parent_id"] = json!("5");
		let page = guild_active(vec![thread(3, 11, false), elsewhere])
			.into_page(Id(2), Id(1))
			.unwrap();
		assert_eq!(
			page.threads.iter().map(|t| t.id).collect::<Vec<_>>(),
			vec![Id(3)]
		);
		assert!(!page.more);
		assert!(
			guild_active(vec![thread(3, 12, false)])
				.into_page(Id(2), Id(1))
				.is_err()
		);
		let crowded: Vec<_> = (3..=32).map(|id| thread(id, 11, false)).collect();
		assert_eq!(
			guild_active(crowded)
				.into_page(Id(2), Id(1))
				.unwrap()
				.threads
				.len(),
			model::forum::PAGE_SIZE
		);
		assert!(decode(vec![thread(3, 11, false); 26], false).is_err());
		assert!(
			decode(vec![thread(3, 11, false); 2], false)
				.unwrap()
				.into_page(Id(2), Id(1))
				.is_err()
		);
	}
}
