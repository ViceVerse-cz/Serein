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
	#[serde(default)]
	pub is_spam_request: bool,
	#[serde(default)]
	pub user_ignored: bool,
}
#[derive(Deserialize)]
pub struct Snapshot(
	#[serde(deserialize_with = "crate::read_state::entries")] pub Vec<Relationship>,
);
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
	pub fn requests(
		&self,
		users: &[crate::UserDto],
	) -> Result<Vec<(model::User, String, bool)>, crate::DecodeError> {
		let users: std::collections::BTreeMap<_, _> = users.iter().map(|u| (u.id, u)).collect();
		self.0
			.iter()
			.filter(|r| matches!(r.kind, 3 | 4))
			.map(|r| {
				let profile = r.user.as_ref().or_else(|| users.get(&r.id).copied());
				let (user, name) = if let Some(user) = profile {
					if user.id != r.id {
						return Err(crate::DecodeError);
					}
					friend(user.clone())?
				} else {
					if r.id.0 == 0 {
						return Err(crate::DecodeError);
					}
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
				r.user
					.as_ref()
					.or_else(|| users.get(&r.id).copied())
					.map(|user| (r, user))
			})
			.map(|(relationship, user)| {
				if user.id != relationship.id {
					return Err(crate::DecodeError);
				}
				let (user, username) = friend(user.clone())?;
				Ok((
					user,
					username,
					relationship.user_ignored && relationship.kind != 2,
				))
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
			if let Some(user) = relationship
				.user
				.as_ref()
				.or_else(|| users.get(&relationship.id).copied())
			{
				if user.id != relationship.id {
					return Err(crate::DecodeError);
				}
				friends.push(friend(user.clone())?);
			}
		}
		Ok(friends)
	}
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
		assert!(crate::decode::<Snapshot>(br#"[{"id":"1"}]"#).is_err());
		let large = format!("[{}]", vec![r#"{"id":"1","type":2}"#; 4001].join(","));
		assert!(crate::decode::<Snapshot>(large.as_bytes()).is_err());
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
		assert!(invalid.friends(&[]).is_err());
	}
}
