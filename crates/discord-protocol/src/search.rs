use crate::UserDto;
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
	content: String,
	#[serde(default)]
	attachments: crate::attachments::AttachmentList,
	#[serde(default)]
	embeds: crate::embeds::EmbedList,
	#[serde(default)]
	hit: Option<bool>,
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
	pub fn into_page(self, channel: Id, before: Option<Id>) -> Result<SearchPage, &'static str> {
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
		hits.sort_unstable_by_key(|h| std::cmp::Reverse(h.id));
		let page = SearchPage {
			hits,
			total,
			partial: self.doing_deep_historical_index,
			pin_cursor: None,
		};
		if !page.valid(channel, before) {
			return Err("Invalid search results");
		}
		Ok(page)
	}
}

impl Hit {
	pub(crate) fn into_hit(self) -> SearchHit {
		let excerpt = if self.content.len() > 8192 {
			"Message exceeds preview limit - open message to read".into()
		} else {
			self.content
		};
		SearchHit {
			id: self.id,
			channel: self.channel_id,
			author: self.author.into_model(),
			excerpt,
			attachments: self.attachments.0,
			embeds: crate::embeds::bounded(self.embeds.0),
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
		let mut matched = hit(3, 1, "hidden ||synthetic spoiler||");
		matched["hit"] = json!(true);
		let page = decode(
			json!({"total_results":8,"messages":[[context,matched]],"doing_deep_historical_index":true}),
		)
		.unwrap()
		.into_page(Id(1), Some(Id(4)))
		.unwrap();
		assert_eq!(page.hits.len(), 1);
		assert_eq!(page.hits[0].id, Id(3));
		assert!(page.partial);
		assert_eq!(page.hits[0].excerpt, "hidden ||synthetic spoiler||");
		for value in [
			json!({"total_results":1}),
			json!({"total_results":1,"messages":[[hit(3,2,"wrong channel")]]}),
			json!({"total_results":2,"messages":[[hit(3,1,"a")],[hit(3,1,"duplicate")]]}),
			json!({"total_results":1,"messages":[[hit(4,1,"outside cursor")]]}),
		] {
			assert!(
				decode(value)
					.unwrap()
					.into_page(Id(1), Some(Id(4)))
					.is_err()
			);
		}
		assert!(
			decode(json!({"total_results":26,"messages":vec![vec![hit(3,1,"a")];26]})).is_err()
		);
		assert!(decode(json!({"total_results":1,"messages":[vec![hit(3,1,"a");6]]})).is_err());
		let page = decode(json!({"total_results":1,"messages":[[hit(3,1,&"x".repeat(10000))]]}))
			.unwrap()
			.into_page(Id(1), None)
			.unwrap();
		assert!(page.hits[0].excerpt.contains("exceeds preview limit"));
		assert!(page.bytes() < model::MAX_SEARCH_BYTES);
		assert!(model::valid_search_query("synthetic & ? 日本語"));
		assert!(!model::valid_search_query("\n"));
		assert!(!model::valid_search_query(&"a".repeat(257)));
	}
}
