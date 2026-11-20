use crate::{MentionList, UserDto};
use model::{Id, SearchHit, SearchPage};
use serde::{
	Deserialize, Deserializer,
	de::{SeqAccess, Visitor},
};

pub const MAX_WIRE: usize = 512 * 1024;
#[derive(Deserialize)]
pub struct Reply {
	#[serde(default)]
	pub code: Option<u32>,
	#[serde(default)]
	pub retry_after: Option<f64>,
	#[serde(default)]
	pub total_results: Option<u64>,
	#[serde(default)]
	pub doing_deep_historical_index: bool,
	#[serde(default)]
	messages: Option<Groups>,
}
#[derive(Deserialize)]
struct Groups(#[serde(deserialize_with = "list::<_,_,25>")] Vec<Group>);
#[derive(Deserialize)]
struct Group(#[serde(deserialize_with = "list::<_,_,5>")] Vec<Hit>);
#[derive(Deserialize)]
pub(crate) struct Hit {
	id: Id,
	channel_id: Id,
	author: UserDto,
	#[serde(default)]
	mentions: MentionList,
	#[serde(default)]
	content: String,
	#[serde(default)]
	attachments: crate::attachments::AttachmentList,
	#[serde(default)]
	embeds: crate::embeds::EmbedList,
	#[serde(default)]
	hit: Option<bool>,
	#[serde(default, deserialize_with = "crate::threads::lenient")]
	flags: Option<u64>,
	#[serde(rename = "type", default, deserialize_with = "crate::threads::lenient")]
	kind: Option<u8>,
	#[serde(default, deserialize_with = "crate::threads::lenient")]
	message_reference: Option<crate::Reference>,
	/// Decoded only for the chosen hit, so a malformed snapshot on a context message, or
	/// one the hit cannot use, never rejects the page.
	#[serde(default)]
	message_snapshots: Option<Box<serde_json::value::RawValue>>,
}
pub(crate) fn list<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
	d: D,
) -> Result<Vec<T>, D::Error> {
	struct Bounded<T, const N: usize>(std::marker::PhantomData<T>);
	impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Bounded<T, N> {
		type Value = Vec<T>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			write!(f, "at most {N} entries")
		}
		fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
			let mut items = Vec::new();
			while let Some(item) = seq.next_element()? {
				if items.len() == N {
					return Err(serde::de::Error::custom("Search capacity exceeded"));
				}
				items.push(item);
			}
			Ok(items)
		}
	}
	d.deserialize_seq(Bounded::<T, N>(std::marker::PhantomData))
}
impl Reply {
	pub fn into_page(
		self,
		channel: Option<Id>,
		cursor: Option<Id>,
		oldest: bool,
	) -> Result<SearchPage, &'static str> {
		let total = self.total_results.ok_or("Search result unavailable")?;
		let mut hits = Vec::new();
		for Group(group) in self.messages.ok_or("Missing search messages")?.0 {
			let single = group.len() == 1;
			let mut matches = group
				.into_iter()
				.filter(|m| m.hit == Some(true) || (single && m.hit.is_none()));
			let hit = matches.next().ok_or("Missing search hit")?;
			if matches.next().is_some() {
				return Err("Ambiguous search hit");
			}
			hits.push(hit.into_hit());
		}
		// Keep the order Discord sorted the whole result set in: the first match leads an
		// oldest-first page, the latest a newest-first one.
		if oldest {
			hits.sort_unstable_by_key(|h| h.id);
		} else {
			hits.sort_unstable_by_key(|h| std::cmp::Reverse(h.id));
		}
		let mut page = SearchPage {
			hits,
			total,
			partial: self.doing_deep_historical_index,
			pin_cursor: None,
		};
		page.fit_media();
		if !page.valid(channel, cursor, oldest) {
			return Err("Invalid search results");
		}
		Ok(page)
	}
}

impl Hit {
	pub(crate) fn into_hit(mut self) -> SearchHit {
		// A forward's own content is empty; its text and media are the snapshot's.
		let forward = self
			.message_snapshots
			.and_then(|raw| serde_json::from_str::<crate::Snapshots>(raw.get()).ok())
			.and_then(|mut snapshots| {
				snapshots.forward(self.kind.unwrap_or(0), self.message_reference.as_ref())
			});
		let forwarded = forward.is_some();
		let mut forward_origin = None;
		let mut flags = self.flags.unwrap_or(0);
		if let Some((body, origin)) = forward {
			self.content = body.content;
			self.attachments = body.attachments;
			self.embeds = body.embeds;
			flags = body.flags;
			forward_origin = origin;
		}
		// Suppressed embeds stay hidden here as in the conversation.
		if flags & 4 != 0 {
			self.embeds.0.clear();
		}
		let excerpt = if self.content.len() > 8192 {
			model::PREVIEW_LIMIT_NOTICE.into()
		} else {
			self.content
		};
		SearchHit {
			id: self.id,
			channel: self.channel_id,
			author: self.author.into_model(),
			mentions: self
				.mentions
				.0
				.into_iter()
				.map(UserDto::into_model)
				.collect(),
			excerpt,
			attachments: self.attachments.0,
			embeds: crate::embeds::bounded(self.embeds.0),
			forwarded,
			forward_origin,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;
	#[test]
	fn search_hits_are_scoped_bounded_and_preserve_rich_text() {
		let hit = |id: u64, channel: u64, content: &str| json!({"id":id.to_string(),"channel_id":channel.to_string(),"author":{"id":"7","username":"Synthetic"},"content":content});
		let decode =
			|value: serde_json::Value| crate::decode::<Reply>(&serde_json::to_vec(&value).unwrap());
		let mut context = hit(2, 1, "context");
		context["hit"] = json!(false);
		let mut matched = hit(3, 1, "hidden ||synthetic spoiler|| <@42>");
		matched["hit"] = json!(true);
		matched["mentions"] = json!([{"id":"42","username":"Mentioned"}]);
		let page = decode(
			json!({"total_results":8,"messages":[[context,matched]],"doing_deep_historical_index":true}),
		)
		.unwrap()
		.into_page(Some(Id(1)), Some(Id(4)), false)
		.unwrap();
		assert_eq!(page.hits.len(), 1);
		assert_eq!(page.hits[0].id, Id(3));
		assert!(page.partial);
		assert_eq!(page.hits[0].excerpt, "hidden ||synthetic spoiler|| <@42>");
		assert_eq!(page.hits[0].mentions[0].name, "Mentioned");
		// A forward's outer content is empty; the hit shows its snapshot and source.
		// An omitted outer type is a default message, as in history.
		let mut forward = hit(3, 1, "");
		forward["message_reference"] = json!({"type":1,"channel_id":"9","message_id":"2"});
		forward["message_snapshots"] = json!([{"message":{"type":0,"content":"forwarded words",
			"embeds":[{"type":"rich","title":"Snapshot embed"}]}}]);
		forward["hit"] = json!(true);
		// Malformed optional data on a context entry, or a forward's source, loses nothing else.
		let mut context = hit(2, 1, "context");
		context["hit"] = json!(false);
		context["message_snapshots"] = json!(null);
		let page = decode(json!({"total_results":1,"messages":[[context, forward.clone()]]}))
			.unwrap()
			.into_page(Some(Id(1)), None, false)
			.unwrap();
		let hit_page = &page.hits[0];
		assert!(hit_page.forwarded);
		assert_eq!(hit_page.excerpt, "forwarded words");
		assert_eq!(hit_page.embeds[0].title.as_deref(), Some("Snapshot embed"));
		assert_eq!(
			hit_page.forward_origin,
			Some(model::ForwardOrigin {
				guild: None,
				channel: Id(9),
				message: Id(2),
			})
		);
		forward["message_reference"]["guild_id"] = json!("0");
		forward["message_snapshots"][0]["message"]["flags"] = json!(4);
		let page = decode(json!({"total_results":1,"messages":[[forward]]}))
			.unwrap()
			.into_page(Some(Id(1)), None, false)
			.unwrap();
		assert_eq!(page.hits[0].excerpt, "forwarded words");
		assert_eq!(page.hits[0].forward_origin.unwrap().guild, None);
		// An unusable source ID drops only the source link.
		let mut sourceless = forward.clone();
		sourceless["message_reference"]["message_id"] = json!("0");
		let page = decode(json!({"total_results":1,"messages":[[sourceless]]}))
			.unwrap()
			.into_page(Some(Id(1)), None, false)
			.unwrap();
		assert!(page.hits[0].forwarded && page.hits[0].forward_origin.is_none());
		assert_eq!(page.hits[0].excerpt, "forwarded words");
		// Suppressed snapshot embeds stay hidden, as in the conversation.
		assert!(page.hits[0].embeds.is_empty());
		// Media that overflows the retained budget is shed from the last hits; every
		// excerpt survives.
		let heavy = |id: u64| {
			let mut value = hit(id, 1, "heavy");
			let url = |kind: &str, n: u64| {
				format!("https://{kind}/attachments/1/{n}/{}.png", "x".repeat(560))
			};
			value["attachments"] = (0..10)
				.map(|n| {
					json!({"id":(id * 100 + n).to_string(),"filename":"a.png",
					"url":url("cdn.discordapp.com", n),"proxy_url":url("media.discordapp.net", n)})
				})
				.collect();
			json!([value])
		};
		let page =
			decode(json!({"total_results":25,"messages":(1..=25).map(heavy).collect::<Vec<_>>()}))
				.unwrap()
				.into_page(Some(Id(1)), None, true)
				.unwrap();
		assert_eq!(page.hits.len(), 25);
		assert!(page.hits.iter().all(|hit| hit.excerpt == "heavy"));
		assert!(page.bytes() <= model::MAX_SEARCH_BYTES);
		assert_eq!(page.hits[0].attachments.len(), 10);
		assert!(page.hits[24].attachments.is_empty());
		for value in [
			json!({"total_results":1}),
			json!({"total_results":1,"messages":[[hit(3,2,"wrong channel")]]}),
			json!({"total_results":2,"messages":[[hit(3,1,"a")],[hit(3,1,"duplicate")]]}),
			json!({"total_results":1,"messages":[[hit(4,1,"outside cursor")]]}),
		] {
			assert!(
				decode(value)
					.unwrap()
					.into_page(Some(Id(1)), Some(Id(4)), false)
					.is_err()
			);
		}
		assert!(
			decode(json!({"total_results":26,"messages":vec![vec![hit(3,1,"a")];26]})).is_err()
		);
		assert!(decode(json!({"total_results":1,"messages":[vec![hit(3,1,"a");6]]})).is_err());
		let page = decode(json!({"total_results":1,"messages":[[hit(3,1,&"x".repeat(10000))]]}))
			.unwrap()
			.into_page(Some(Id(1)), None, false)
			.unwrap();
		assert!(page.hits[0].excerpt.contains("exceeds preview limit"));
		assert!(page.bytes() < model::MAX_SEARCH_BYTES);
		assert!(model::valid_search_query("synthetic & ? 日本語"));
		assert!(!model::valid_search_query("\n"));
		assert!(!model::valid_search_query(&"a".repeat(257)));
	}

	/// An oldest-first search keeps Discord's order: the very first message leads page one.
	#[test]
	fn oldest_first_pages_keep_the_earliest_message_on_top() {
		let hit = |id: u64| json!({"id":id.to_string(),"channel_id":"1","author":{"id":"7","username":"Synthetic"},"content":"m"});
		let reply = || {
			crate::decode::<Reply>(
				&serde_json::to_vec(
					&json!({"total_results":3,"messages":[[hit(5)],[hit(6)],[hit(9)]]}),
				)
				.unwrap(),
			)
			.unwrap()
		};
		let ids = |page: model::SearchPage| page.hits.iter().map(|h| h.id.0).collect::<Vec<_>>();
		assert_eq!(
			ids(reply().into_page(Some(Id(1)), None, true).unwrap()),
			[5, 6, 9]
		);
		assert_eq!(
			ids(reply().into_page(Some(Id(1)), None, false).unwrap()),
			[9, 6, 5]
		);
	}
}
