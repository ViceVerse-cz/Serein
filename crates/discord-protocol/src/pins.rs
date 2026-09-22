use crate::{
	Timestamp,
	search::{Hit, list},
};
use model::{Id, SearchPage};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Reply {
	#[serde(deserialize_with = "list::<_,_,25>")]
	items: Vec<Pin>,
	has_more: bool,
}
#[derive(Deserialize)]
struct Pin {
	pinned_at: Timestamp,
	message: Hit,
}
impl Reply {
	pub fn into_page(self, channel: Id, before: Option<i128>) -> Result<SearchPage, &'static str> {
		if self.has_more && self.items.is_empty()
			|| self
				.items
				.windows(2)
				.any(|pair| pair[0].pinned_at.0 < pair[1].pinned_at.0)
			|| before.is_some_and(|cursor| self.items.iter().any(|pin| pin.pinned_at.0 >= cursor))
		{
			return Err("Pinned-message page did not advance in pin order");
		}
		let pin_cursor = self
			.has_more
			.then(|| self.items.last().expect("nonempty pin page").pinned_at.0);
		let page = SearchPage {
			hits: self
				.items
				.into_iter()
				.map(|pin| pin.message.into_hit())
				.collect(),
			// Pins have no service-provided total. The UI uses the page length only.
			total: 0,
			partial: self.has_more,
			pin_cursor,
		};
		if !page.valid_pins(channel) {
			return Err("Invalid pinned messages");
		}
		Ok(page)
	}
}

pub fn format_cursor(nanoseconds: i128) -> Result<String, &'static str> {
	time::OffsetDateTime::from_unix_timestamp_nanos(nanoseconds)
		.map_err(|_| "Invalid pin cursor")?
		.format(&time::format_description::well_known::Rfc3339)
		.map_err(|_| "Invalid pin cursor")
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;
	#[test]
	fn pins_preserve_pin_order_and_reject_wrong_scope_duplicates_and_capacity() {
		let pin = |id: u64, channel: u64| json!({"pinned_at":"2026-09-10T12:00:00Z","message":{"id":id.to_string(),"channel_id":channel.to_string(),"author":{"id":"7","username":"Synthetic"},"content":"hidden ||synthetic secret||"}});
		let decode =
			|value: serde_json::Value| crate::decode::<Reply>(&serde_json::to_vec(&value).unwrap());
		let mut older = pin(9, 1);
		older["pinned_at"] = "2026-09-10T11:00:00.123456789Z".into();
		let page = decode(json!({"items":[pin(2,1),older.clone()],"has_more":true}))
			.unwrap()
			.into_page(Id(1), None)
			.unwrap();
		assert_eq!(
			page.hits.iter().map(|h| h.id).collect::<Vec<_>>(),
			vec![Id(2), Id(9)]
		);
		assert!(page.partial);
		let cursor = page.pin_cursor.unwrap();
		assert_eq!(
			format_cursor(cursor).unwrap(),
			"2026-09-10T11:00:00.123456789Z"
		);
		assert!(format_cursor(i128::MAX).is_err());
		assert!(format_cursor(i128::MIN).is_err());
		assert!(
			decode(json!({"items":[older.clone(),pin(2,1)],"has_more":true}))
				.unwrap()
				.into_page(Id(1), None)
				.is_err()
		);
		assert!(
			decode(json!({"items":[older.clone()],"has_more":true}))
				.unwrap()
				.into_page(Id(1), Some(cursor))
				.is_err()
		);
		assert!(
			decode(json!({"items":[],"has_more":true}))
				.unwrap()
				.into_page(Id(1), Some(cursor))
				.is_err()
		);
		older["pinned_at"] = "2026-09-10T10:00:00Z".into();
		let exhausted = decode(json!({"items":[older.clone()],"has_more":false}))
			.unwrap()
			.into_page(Id(1), Some(cursor))
			.unwrap();
		assert!(!exhausted.partial && exhausted.pin_cursor.is_none());
		older["pinned_at"] = "not a timestamp".into();
		assert!(decode(json!({"items":[older],"has_more":true})).is_err());
		assert_eq!(page.hits[0].excerpt, "hidden ||synthetic secret||");
		for items in [vec![pin(2, 2)], vec![pin(2, 1), pin(2, 1)]] {
			assert!(
				decode(json!({"items":items,"has_more":false}))
					.unwrap()
					.into_page(Id(1), None)
					.is_err()
			);
		}
		assert!(decode(json!({"items":vec![pin(2,1);26],"has_more":true})).is_err());
		assert!(
			decode(json!({"items":[],"has_more":false}))
				.unwrap()
				.into_page(Id(1), Some(cursor))
				.unwrap()
				.hits
				.is_empty()
		);
		assert!(decode(json!({"items":[]})).is_err());
	}
}
