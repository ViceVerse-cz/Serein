//! Unofficial normal-user relationship payloads; bounded friend and block projections.
use model::Id;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Relationship {
	pub id: Id,
	#[serde(rename = "type")]
	pub kind: u8,
	#[serde(default)]
	pub nickname: model::Patch<String>,
	#[serde(default)]
	pub user: Option<crate::UserDto>,
	#[serde(default, deserialize_with = "crate::lossy::null_default")]
	pub is_spam_request: bool,
	#[serde(default, deserialize_with = "crate::lossy::null_default")]
	pub user_ignored: bool,
	/// Unofficial: when the relationship began, as Unix seconds. A missing, oversized or
	/// malformed timestamp is dropped without rejecting the row.
	#[serde(default, deserialize_with = "since")]
	pub since: Option<i64>,
}
fn since<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
	use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
	struct Since;
	impl<'de> Visitor<'de> for Since {
		type Value = Option<i64>;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("an optional timestamp")
		}
		fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
			Ok((text.len() <= 64)
				.then(|| {
					time::OffsetDateTime::parse(
						text,
						&time::format_description::well_known::Rfc3339,
					)
					.ok()
				})
				.flatten()
				.map(time::OffsetDateTime::unix_timestamp))
		}
		fn visit_unit<E>(self) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
			Ok(None)
		}
		fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
			while seq.next_element::<IgnoredAny>()?.is_some() {}
			Ok(None)
		}
		fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
			while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
			Ok(None)
		}
	}
	d.deserialize_any(Since)
}
/// Busy accounts can hold thousands of requests and blocks: past the bound, or on a malformed
/// row, entries are dropped and `.1` reports it instead of rejecting the login.
pub struct Snapshot(pub Vec<Relationship>, pub bool, pub(crate) Option<String>);
pub const MAX_RELATIONSHIPS: usize = 10_000;
impl<'de> Deserialize<'de> for Snapshot {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		let rows = crate::lossy::Lossy::<Relationship, MAX_RELATIONSHIPS, true>::deserialize(d)?;
		Ok(Self(rows.items, rows.skipped, rows.cause))
	}
}
impl Snapshot {
	pub fn nicknames(&self) -> Vec<(Id, String)> {
		self.0
			.iter()
			.filter(|r| r.kind == 1)
			.filter_map(|r| match &r.nickname {
				model::Patch::Value(text) => Some((r.id, text.clone())),
				_ => None,
			})
			.collect()
	}
	/// Start of each friendship that reported one.
	pub fn friends_since(&self) -> Vec<(Id, i64)> {
		self.0
			.iter()
			.filter(|r| r.kind == 1 && r.id.0 != 0)
			.filter_map(|r| Some((r.id, r.since?)))
			.collect()
	}
	pub fn requests(
		&self,
		users: &[crate::UserDto],
	) -> Result<Vec<(model::User, String, bool)>, crate::DecodeError> {
		let users: std::collections::BTreeMap<_, _> = users.iter().map(|u| (u.id, u)).collect();
		self.0
			.iter()
			.filter(|r| matches!(r.kind, 3 | 4) && r.id.0 != 0)
			.map(|r| {
				// A mismatched or malformed profile is shown like a missing one.
				let profile = r.user.as_ref().or_else(|| users.get(&r.id).copied());
				let (user, name) = if let Some(profile) = valid_profile(r, profile) {
					profile
				} else {
					(
						model::User {
							id: r.id,
							name: "Unknown user".into(),
							avatar: None,
							discriminator: 0,
							primary_guild: None,
							webhook: false,
							kind: Default::default(),
						},
						format!("User ID: {}", r.id),
					)
				};
				Ok((user, name, r.kind == 3))
			})
			.collect()
	}
	pub fn entries(&self) -> Vec<(Id, bool)> {
		self.0.iter().map(|r| (r.id, r.kind == 2)).collect()
	}
	pub fn restricted(
		&self,
		users: &[crate::UserDto],
	) -> Result<Vec<(model::User, String, bool)>, crate::DecodeError> {
		let users: std::collections::BTreeMap<_, _> = users.iter().map(|u| (u.id, u)).collect();
		self.0
			.iter()
			.filter(|r| r.kind == 2 || r.user_ignored)
			.filter_map(|r| {
				let profile = r.user.as_ref().or_else(|| users.get(&r.id).copied());
				valid_profile(r, profile)
					.map(|(user, username)| Ok((user, username, r.user_ignored && r.kind != 2)))
			})
			.collect()
	}
	pub fn spam_incoming_ids(&self) -> Vec<Id> {
		self.0
			.iter()
			.filter(|row| row.kind == 3 && row.is_spam_request)
			.map(|row| row.id)
			.collect()
	}
	pub fn friends(
		&self,
		users: &[crate::UserDto],
	) -> Result<Vec<(model::User, String)>, crate::DecodeError> {
		let users: std::collections::BTreeMap<_, _> = users.iter().map(|u| (u.id, u)).collect();
		let mut friends = Vec::new();
		for relationship in self.0.iter().filter(|r| r.kind == 1) {
			let profile = relationship
				.user
				.as_ref()
				.or_else(|| users.get(&relationship.id).copied());
			friends.extend(valid_profile(relationship, profile));
		}
		Ok(friends)
	}
}
fn valid_profile(
	relationship: &Relationship,
	profile: Option<&crate::UserDto>,
) -> Option<(model::User, String)> {
	profile
		.filter(|user| user.id == relationship.id)
		.and_then(|user| friend(user.clone()).ok())
}
pub fn friend(user: crate::UserDto) -> Result<(model::User, String), crate::DecodeError> {
	if user.id.0 == 0
		|| user.username.is_empty()
		|| user.username.len() > 128
		|| user.username.chars().any(char::is_control)
		|| user
			.global_name
			.as_ref()
			.is_some_and(|n| n.is_empty() || n.len() > 512 || n.chars().any(char::is_control))
	{
		return Err(crate::DecodeError);
	}
	let username = user.username.clone();
	Ok((user.into_model(), username))
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn relationships_decode_only_bounded_typed_account_state() {
		for (json, expected) in [
			(r#"{"id":"2","type":1}"#, model::Patch::Absent),
			(r#"{"id":"2","type":1,"nickname":null}"#, model::Patch::Null),
			(
				r#"{"id":"2","type":1,"nickname":"Bestie"}"#,
				model::Patch::Value("Bestie".into()),
			),
		] {
			let row: Relationship = crate::decode(json.as_bytes()).unwrap();
			assert_eq!(row.nickname, expected);
		}
		let rows: Snapshot = crate::decode(
			br#"[{"id":"1","type":2,"user":{"id":"1","username":"ignored"}},{"id":"2","type":1}]"#,
		)
		.unwrap();
		assert_eq!(rows.entries(), vec![(Id(1), true), (Id(2), false)]);
		let rows: Snapshot = crate::decode(
			br#"[{"id":"2","type":1,"since":"2021-03-04T05:06:07.000000+00:00"},{"id":"3","type":1,"since":"not a date"},{"id":"4","type":1,"since":7},{"id":"5","type":3,"since":"2021-03-04T05:06:07+00:00"}]"#,
		)
		.unwrap();
		assert_eq!(rows.0.len(), 4, "A malformed timestamp never drops the row");
		assert_eq!(rows.friends_since(), vec![(Id(2), 1_614_834_367)]);
		let malformed = crate::decode::<Snapshot>(br#"[{"id":"1"},{"id":"2","type":1}]"#).unwrap();
		assert!(malformed.1 && malformed.0.len() == 1);
		let large = format!(
			"[{}]",
			vec![r#"{"id":"1","type":2}"#; MAX_RELATIONSHIPS + 1].join(",")
		);
		let large = crate::decode::<Snapshot>(large.as_bytes()).unwrap();
		assert!(large.1 && large.0.len() == MAX_RELATIONSHIPS);
		let rows:Snapshot=crate::decode(br#"[{"id":"2","type":1},{"id":"3","type":2,"user":{"id":"3","username":"blocked"}},{"id":"4","type":3},{"id":"5","type":1,"user_ignored":true,"user":{"id":"5","username":"ignored"}}]"#).unwrap();
		let users=crate::decode::<Vec<crate::UserDto>>(br#"[{"id":"2","username":"friend_name","global_name":"Friend display"},{"id":"4","username":"pending"}]"#).unwrap();
		let friends = rows.friends(&users).unwrap();
		assert_eq!(friends.len(), 2);
		assert_eq!(friends[0].0.name, "Friend display");
		assert_eq!(friends[0].1, "friend_name");
		let restricted = rows.restricted(&users).unwrap();
		assert_eq!(restricted.len(), 2);
		assert_eq!((restricted[0].0.id, restricted[0].2), (Id(3), false));
		assert_eq!((restricted[1].0.id, restricted[1].2), (Id(5), true));
		let requests = rows.requests(&users).unwrap();
		assert_eq!(requests.len(), 1);
		assert!(requests[0].2);
		assert_eq!(requests[0].1, "pending");
		let outgoing: Snapshot = crate::decode(br#"[{"id":"8","type":4}]"#).unwrap();
		let requests = outgoing.requests(&[]).unwrap();
		assert!(!requests[0].2);
		assert_eq!(requests[0].0.id, Id(8));
		let spam: Snapshot =
			crate::decode(br#"[{"id":"9","type":3,"is_spam_request":true}]"#).unwrap();
		assert_eq!(spam.spam_incoming_ids(), vec![Id(9)]);
		assert!(spam.requests(&[]).unwrap()[0].2);
		let invalid: Snapshot =
			crate::decode(br#"[{"id":"2","type":1,"user":{"id":"3","username":"wrong"}}]"#)
				.unwrap();
		assert!(
			invalid.friends(&[]).unwrap().is_empty(),
			"A mismatched profile is never attributed to the relationship"
		);
	}
}
