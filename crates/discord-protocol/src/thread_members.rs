//! Documented thread-member REST snapshots, bounded to the visible first 100 members.
use crate::{DecodeError, MemberDto, MemberItem, permissions::List};
use model::{Freshness, Id, MemberList};
use serde::Deserialize;
use std::collections::BTreeSet;

pub const MAX_WIRE: usize = 512 * 1024;
const MAX_BYTES: usize = 128 * 1024;

#[derive(Deserialize)]
struct ThreadMember {
	id: Option<Id>,
	user_id: Id,
	member: MemberDto,
}

pub fn members(
	bytes: &[u8],
	guild: Id,
	channel: Id,
	request: u64,
) -> Result<MemberList, DecodeError> {
	if bytes.len() > MAX_WIRE || guild.0 == 0 || channel.0 == 0 {
		return Err(DecodeError);
	}
	let members: List<ThreadMember, 100> = crate::decode(bytes)?;
	let mut rows = Vec::with_capacity(members.0.len());
	let mut seen = BTreeSet::new();
	let mut retained = 0;
	for entry in members.0 {
		if entry.id.is_some_and(|id| id != channel)
			|| entry.user_id != entry.member.user.id
			|| !seen.insert(entry.user_id)
		{
			return Err(DecodeError);
		}
		let member = MemberItem::Member {
			member: Box::new(entry.member),
		}
		.into_model()
		.ok_or(DecodeError)?;
		retained += member.bytes();
		if !member.valid() || retained > MAX_BYTES {
			return Err(DecodeError);
		}
		rows.push(Some(member));
	}
	Ok(MemberList {
		guild: Some(guild),
		channel,
		request,
		total: rows.len() as u64,
		rows,
		freshness: Freshness::Fresh,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn thread_members_validate_identity_scope_and_bounds() {
		let entry = json!({"id":"2","user_id":"3","member":{"user":{"id":"3","username":"Synthetic"},"nick":"Thread participant","roles":["9","8"]}});
		let parse = |value| members(&serde_json::to_vec(&value).unwrap(), Id(1), Id(2), 7);
		let list = parse(json!([entry])).unwrap();
		assert_eq!(
			(list.guild, list.channel, list.request, list.total),
			(Some(Id(1)), Id(2), 7, 1)
		);
		let member = list.rows[0].as_ref().unwrap();
		assert_eq!(member.roles, vec![Id(8), Id(9)]);
		assert_eq!(member.nick.as_deref(), Some("Thread participant"));
		assert_eq!(member.status, None);
		assert!(parse(json!([])).unwrap().rows.is_empty());
		let mut without_id = entry.clone();
		without_id.as_object_mut().unwrap().remove("id");
		assert!(parse(json!([without_id])).is_ok());
		for (field, value) in [
			("id", json!("4")),
			("user_id", json!("4")),
			("member", json!(null)),
		] {
			let mut invalid = entry.clone();
			invalid[field] = value;
			assert!(parse(json!([invalid])).is_err());
		}
		assert!(parse(json!([entry, entry])).is_err());
		assert!(
			parse(json!([{"user_id":"0","member":{"user":{"id":"0","username":"Invalid"}}}]))
				.is_err()
		);
		let full: Vec<_> = (1..=100).map(|id| json!({"user_id":id.to_string(),"member":{"user":{"id":id.to_string(),"username":"Synthetic"}}})).collect();
		assert_eq!(parse(json!(full)).unwrap().rows.len(), 100);
		assert!(parse(json!(vec![entry.clone(); 101])).is_err());
		assert!(members(&vec![b' '; MAX_WIRE + 1], Id(1), Id(2), 7).is_err());
		assert!(members(b"[]", Id(0), Id(2), 7).is_err());
		let large: Vec<_> = (1..=100).map(|id| json!({"user_id":id.to_string(),"member":{"user":{"id":id.to_string(),"username":"Synthetic"},"roles":(1..=200).map(|role| role.to_string()).collect::<Vec<_>>()}})).collect();
		let bytes = serde_json::to_vec(&large).unwrap();
		assert!(bytes.len() < MAX_WIRE);
		assert!(
			members(&bytes, Id(1), Id(2), 7).is_err(),
			"retained role vectors exceed the byte cap"
		);
	}
}
