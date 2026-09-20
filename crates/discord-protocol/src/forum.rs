//! Active forum posts from the thread search route; archived rows belong to the archive page.
use crate::ChannelDto;
use model::{Id, forum::Page};
use serde::Deserialize;
pub const MAX_WIRE: usize = 512 * 1024;
/// The guild-wide fallback lists every visible thread, so it needs the snapshot budget.
pub const GUILD_MAX_WIRE: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
pub struct Reply {
	#[serde(deserialize_with = "crate::search::list::<_,_,25>")]
	threads: Vec<Thread>,
	#[serde(default)]
	has_more: bool,
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
		let page = Page {
			threads,
			more: self.has_more,
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
