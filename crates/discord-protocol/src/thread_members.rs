//! Unofficial user-gateway thread-member snapshots, bounded to the first 100 members.
use crate::{DecodeError, MemberDto, MemberItem, PresenceDto};
use model::{Freshness, Id, MemberList, MemberSlot};
use serde::{
	Deserialize, Deserializer,
	de::{IgnoredAny, SeqAccess, Visitor},
};
use std::collections::BTreeSet;

pub const MAX_WIRE: usize = 512 * 1024;
const MAX_BYTES: usize = 128 * 1024;

#[derive(Deserialize)]
struct ThreadMember {
	user_id: Id,
	member: MemberDto,
	#[serde(default, deserialize_with = "crate::lenient_presence")]
	presence: model::Patch<PresenceDto>,
}

#[derive(Deserialize)]
struct Snapshot {
	guild_id: Id,
	thread_id: Id,
	members: Rows,
}

struct Rows {
	slots: Vec<Option<MemberSlot>>,
	total: u64,
}

impl<'de> Deserialize<'de> for Rows {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct Bounded;
		impl<'de> Visitor<'de> for Bounded {
			type Value = Rows;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("a thread member list")
			}
			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Rows, A::Error> {
				let mut slots = Vec::with_capacity(100);
				let mut seen = BTreeSet::new();
				let mut retained = slots.capacity() * size_of::<Option<MemberSlot>>();
				while slots.len() < 100 {
					let Some(mut entry) = seq.next_element::<ThreadMember>()? else {
						break;
					};
					if entry.user_id != entry.member.user.id || !seen.insert(entry.user_id) {
						return Err(serde::de::Error::custom("Invalid thread member identity"));
					}
					if let model::Patch::Value(presence) = entry.presence {
						entry.member.presence = model::Patch::Value(presence);
					}
					let member = MemberItem::Member {
						presence: model::Patch::Absent,
						member: Box::new(entry.member),
					}
					.into_model()
					.ok_or_else(|| serde::de::Error::custom("Invalid thread member"))?;
					retained += member.bytes() - size_of::<model::Member>();
					if !member.valid() || retained > MAX_BYTES {
						return Err(serde::de::Error::custom("Thread member capacity exceeded"));
					}
					slots.push(Some(MemberSlot::Person(member)));
				}
				let mut total = slots.len() as u64;
				while seq.next_element::<IgnoredAny>()?.is_some() {
					total += 1;
				}
				Ok(Rows { slots, total })
			}
		}
		d.deserialize_seq(Bounded)
	}
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
	let snapshot: Snapshot = crate::decode(bytes)?;
	if snapshot.guild_id != guild || snapshot.thread_id != channel {
		return Err(DecodeError);
	}
	Ok(MemberList {
		guild: Some(guild),
		channel,
		request,
		start: 0,
		total: snapshot.members.total,
		slots: snapshot.members.slots,
		lazy: false,
		freshness: Freshness::Fresh,
		groups: vec![],
		ranges: vec![],
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn thread_members_validate_identity_scope_and_bounds() {
		let entry = json!({"user_id":"3","member":{"user":{"id":"3","username":"Synthetic"},"nick":"Thread participant","roles":["9","8"]},"presence":{"status":"online"}});
		let snapshot = |value| json!({"guild_id":"1","thread_id":"2","members":value});
		let parse = |value| members(&serde_json::to_vec(&value).unwrap(), Id(1), Id(2), 7);
		let list = parse(snapshot(json!([entry]))).unwrap();
		assert_eq!(
			(list.guild, list.channel, list.request, list.total),
			(Some(Id(1)), Id(2), 7, 1)
		);
		let model::MemberSlot::Person(member) = list.slots[0].as_ref().unwrap() else {
			panic!("expected person slot");
		};
		assert_eq!(member.roles, vec![Id(8), Id(9)]);
		assert_eq!(member.nick.as_deref(), Some("Thread participant"));
		assert_eq!(member.status.as_deref(), Some("online"));
		let mut unusual = entry.clone();
		unusual["presence"] = json!({"status":"idle","activities":[{"type":0,"name":"Game","timestamps":{"start":1.5}}]});
		let list = parse(snapshot(json!([unusual]))).unwrap();
		let model::MemberSlot::Person(member) = list.slots[0].as_ref().unwrap() else {
			panic!("an unusual activity keeps the participant");
		};
		assert_eq!(member.status.as_deref(), Some("idle"));
		assert!(member.activities.is_empty());
		assert!(parse(snapshot(json!([]))).unwrap().slots.is_empty());
		for field in ["guild_id", "thread_id"] {
			let mut invalid = snapshot(json!([entry]));
			invalid[field] = json!("4");
			assert!(parse(invalid).is_err());
		}
		for (field, value) in [("user_id", json!("4")), ("member", json!(null))] {
			let mut invalid = entry.clone();
			invalid[field] = value;
			assert!(parse(snapshot(json!([invalid]))).is_err());
		}
		assert!(parse(snapshot(json!([entry, entry]))).is_err());
		assert!(
			parse(snapshot(
				json!([{"user_id":"0","member":{"user":{"id":"0","username":"Invalid"}}}])
			))
			.is_err()
		);
		let full: Vec<_> = (1..=101).map(|id| json!({"user_id":id.to_string(),"member":{"user":{"id":id.to_string(),"username":"Synthetic"}}})).collect();
		let list = parse(snapshot(json!(full))).unwrap();
		assert_eq!((list.slots.len(), list.total), (100, 101));
		assert!(members(&vec![b' '; MAX_WIRE + 1], Id(1), Id(2), 7).is_err());
		assert!(members(b"{}", Id(0), Id(2), 7).is_err());
		let large: Vec<_> = (1..=100).map(|id| json!({"user_id":id.to_string(),"member":{"user":{"id":id.to_string(),"username":"Synthetic"},"roles":(1..=200).map(|role| role.to_string()).collect::<Vec<_>>()}})).collect();
		let bytes = serde_json::to_vec(&snapshot(json!(large))).unwrap();
		assert!(bytes.len() < MAX_WIRE);
		assert!(
			members(&bytes, Id(1), Id(2), 7).is_err(),
			"retained role vectors exceed the byte cap"
		);
	}
}
