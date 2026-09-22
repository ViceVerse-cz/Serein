//! Discord's explicit role/overwrite calculation. Action prerequisites remain with callers.
//! Reference: https://docs.discord.com/developers/topics/permissions
use crate::Id;
use std::collections::{BTreeMap, BTreeSet};

pub const ADMINISTRATOR: u128 = 1 << 3;
pub const KICK_MEMBERS: u128 = 1 << 1;
pub const MANAGE_GUILD_EXPRESSIONS: u128 = 1 << 30;
pub const USE_APPLICATION_COMMANDS: u128 = 1 << 31;
pub const CREATE_GUILD_EXPRESSIONS: u128 = 1 << 43;
pub const CHANGE_NICKNAME: u128 = 1 << 26;
pub const MANAGE_NICKNAMES: u128 = 1 << 27;
pub const MANAGE_CHANNELS: u128 = 1 << 4;
pub const MANAGE_GUILD: u128 = 1 << 5;
pub const ADD_REACTIONS: u128 = 1 << 6;
pub const VIEW_AUDIT_LOG: u128 = 1 << 7;
pub const STREAM: u128 = 1 << 9;
pub const VIEW_CHANNEL: u128 = 1 << 10;
pub const SEND_MESSAGES: u128 = 1 << 11;
pub const SEND_TTS_MESSAGES: u128 = 1 << 12;
pub const MANAGE_MESSAGES: u128 = 1 << 13;
pub const EMBED_LINKS: u128 = 1 << 14;
pub const ATTACH_FILES: u128 = 1 << 15;
pub const READ_MESSAGE_HISTORY: u128 = 1 << 16;
pub const MENTION_EVERYONE: u128 = 1 << 17;
pub const USE_EXTERNAL_EMOJIS: u128 = 1 << 18;
pub const CONNECT: u128 = 1 << 20;
pub const SPEAK: u128 = 1 << 21;
pub const MUTE_MEMBERS: u128 = 1 << 22;
pub const DEAFEN_MEMBERS: u128 = 1 << 23;
pub const MOVE_MEMBERS: u128 = 1 << 24;
pub const USE_VAD: u128 = 1 << 25;
pub const MANAGE_WEBHOOKS: u128 = 1 << 29;
pub const MANAGE_ROLES: u128 = 1 << 28;
pub const MANAGE_THREADS: u128 = 1 << 34;
pub const CREATE_PUBLIC_THREADS: u128 = 1 << 35;
pub const CREATE_PRIVATE_THREADS: u128 = 1 << 36;
pub const USE_EXTERNAL_STICKERS: u128 = 1 << 37;
pub const SEND_MESSAGES_IN_THREADS: u128 = 1 << 38;
pub const MODERATE_MEMBERS: u128 = 1 << 40;
pub const PIN_MESSAGES: u128 = 1 << 51;

pub const MAX_ROLES: usize = 512;
pub const MAX_MEMBER_ROLES: usize = 512;
pub const MAX_OVERWRITES: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Role {
	pub id: Id,
	pub bits: u128,
	pub name: String,
	pub color: u32,
	pub position: i32,
	pub hoist: bool,
}
impl Role {
	/// Higher positions rank first; equal positions favor the older (lower) role ID.
	/// Compare roles from the same guild, excluding its @everyone role.
	pub fn cmp_hierarchy(&self, other: &Self) -> std::cmp::Ordering {
		self.position
			.cmp(&other.position)
			.then_with(|| other.id.cmp(&self.id))
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>() + self.name.capacity()
	}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overwrite {
	pub id: Id,
	pub kind: u8,
	pub allow: u128,
	pub deny: u128,
}
// Unofficial list identity observed by discord.py-self (abc.GuildChannel.member_list_id).
// This hash selects a server list; it never grants permissions.
pub fn member_list_id(everyone: u128, overwrites: &[Overwrite]) -> Option<String> {
	if overwrites.len() > MAX_OVERWRITES {
		return None;
	}
	if everyone & VIEW_CHANNEL != 0 && !overwrites.iter().any(|o| o.deny & VIEW_CHANNEL != 0) {
		return Some("everyone".into());
	}
	let mut entries: Vec<_> = overwrites
		.iter()
		.filter_map(|o| {
			if o.allow & VIEW_CHANNEL != 0 {
				Some(format!("allow:{}", o.id))
			} else if o.deny & VIEW_CHANNEL != 0 {
				Some(format!("deny:{}", o.id))
			} else {
				None
			}
		})
		.collect();
	entries.sort();
	Some(murmur3(entries.join(",").as_bytes()).to_string())
}

pub fn everyone_can_view(everyone: u128, guild: Id, overwrites: &[Overwrite]) -> Option<bool> {
	if overwrites.len() > MAX_OVERWRITES {
		return None;
	}
	if everyone & ADMINISTRATOR != 0 {
		return Some(true);
	}
	let mut seen = BTreeSet::new();
	let mut selected = None;
	for overwrite in overwrites {
		if overwrite.id.0 == 0 || overwrite.kind > 1 || !seen.insert((overwrite.kind, overwrite.id))
		{
			return None;
		}
		if overwrite.kind == 0 && overwrite.id == guild {
			selected = Some(overwrite);
		}
	}
	let mut bits = everyone;
	if let Some(overwrite) = selected {
		bits = (bits & !overwrite.deny) | overwrite.allow;
	}
	Some(bits & VIEW_CHANNEL != 0)
}
fn murmur3(bytes: &[u8]) -> u32 {
	let mix = |n: u32| {
		n.wrapping_mul(0xcc9e2d51)
			.rotate_left(15)
			.wrapping_mul(0x1b873593)
	};
	let mut hash = 0u32;
	let (chunks, remainder) = bytes.as_chunks::<4>();
	for part in chunks {
		hash ^= mix(u32::from_le_bytes(*part));
		hash = hash
			.rotate_left(13)
			.wrapping_mul(5)
			.wrapping_add(0xe6546b64);
	}
	let tail = remainder
		.iter()
		.enumerate()
		.fold(0u32, |n, (i, b)| n | (u32::from(*b) << (i * 8)));
	if !remainder.is_empty() {
		hash ^= mix(tail);
	}
	hash ^= bytes.len() as u32;
	hash ^= hash >> 16;
	hash = hash.wrapping_mul(0x85ebca6b);
	hash ^= hash >> 13;
	hash = hash.wrapping_mul(0xc2b2ae35);
	hash ^ (hash >> 16)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
	pub roles: Vec<Id>,
	/// UTC Unix seconds; None represents an explicitly absent timeout.
	pub timeout_until: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guild {
	pub id: Id,
	pub owner: Option<Id>,
	pub roles: Option<Vec<Role>>,
	pub member: Option<Member>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
	pub id: Id,
	pub guild: Id,
	/// None is unknown metadata; Some(empty) is a known channel with no overwrites.
	pub overwrites: Option<Vec<Overwrite>>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
	pub guilds: Vec<Guild>,
	pub channels: Vec<Channel>,
}

impl Member {
	pub fn bytes(&self) -> usize {
		size_of::<Self>() + self.roles.capacity() * size_of::<Id>()
	}
}
impl Guild {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.roles.as_ref().map_or(0, |roles| {
				roles.capacity() * size_of::<Role>()
					+ roles.iter().map(|role| role.name.capacity()).sum::<usize>()
			}) + self
			.member
			.as_ref()
			.map_or(0, |member| member.bytes() - size_of::<Member>())
	}
}
impl Channel {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self
				.overwrites
				.as_ref()
				.map_or(0, |rows| rows.capacity() * size_of::<Overwrite>())
	}
}
impl Snapshot {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.guilds.capacity().saturating_sub(self.guilds.len()) * size_of::<Guild>()
			+ self.channels.capacity().saturating_sub(self.channels.len()) * size_of::<Channel>()
			+ self.guilds.iter().map(Guild::bytes).sum::<usize>()
			+ self.channels.iter().map(Channel::bytes).sum::<usize>()
	}
}

/// Compute explicit bits, or None when the available metadata cannot establish them.
/// Owners and administrators bypass overwrites and timeouts. No implicit action mask
/// (such as SEND_MESSAGES versus SEND_MESSAGES_IN_THREADS) is applied here.
pub fn effective(
	guild: &Guild,
	user: Id,
	overwrites: Option<&[Overwrite]>,
	now: i64,
) -> Option<u128> {
	if guild.id.0 == 0 || user.0 == 0 || guild.owner.is_some_and(|owner| owner.0 == 0) {
		return None;
	}
	if guild.owner == Some(user) {
		return Some(u128::MAX);
	}
	let roles = guild.roles.as_ref()?;
	let member = guild.member.as_ref()?;
	if roles.len() > MAX_ROLES || member.roles.len() > MAX_MEMBER_ROLES {
		return None;
	}
	let mut role_bits = BTreeMap::new();
	for role in roles {
		if role.id.0 == 0 || role_bits.insert(role.id, role.bits).is_some() {
			return None;
		}
	}
	let mut bits = *role_bits.get(&guild.id)?;
	let mut self_roles = BTreeSet::new();
	for id in &member.roles {
		if *id == guild.id || !self_roles.insert(*id) {
			return None;
		}
		bits |= *role_bits.get(id)?;
	}
	if bits & ADMINISTRATOR != 0 {
		return Some(u128::MAX);
	}
	// Without the owner identity a non-administrator may still be the owner.
	guild.owner?;
	let overwrites = overwrites?;
	if overwrites.len() > MAX_OVERWRITES {
		return None;
	}
	let mut seen = BTreeSet::new();
	let mut everyone = None;
	let mut role_allow = 0;
	let mut role_deny = 0;
	let mut own = None;
	for overwrite in overwrites {
		if overwrite.id.0 == 0
			|| overwrite.kind > 1
			|| !seen.insert((overwrite.kind, overwrite.id))
			|| (overwrite.kind == 0 && !role_bits.contains_key(&overwrite.id))
		{
			return None;
		}
		match overwrite.kind {
			0 if overwrite.id == guild.id => everyone = Some(overwrite),
			0 if self_roles.contains(&overwrite.id) => {
				role_allow |= overwrite.allow;
				role_deny |= overwrite.deny;
			}
			1 if overwrite.id == user => own = Some(overwrite),
			_ => {}
		}
	}
	if let Some(overwrite) = everyone {
		bits = (bits & !overwrite.deny) | overwrite.allow;
	}
	bits = (bits & !role_deny) | role_allow;
	if let Some(overwrite) = own {
		bits = (bits & !overwrite.deny) | overwrite.allow;
	}
	if member.timeout_until.is_some_and(|until| until > now) {
		bits &= VIEW_CHANNEL | READ_MESSAGE_HISTORY;
	}
	Some(bits)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn member_list_identity_matches_wire_hash_and_bounds() {
		assert_eq!(murmur3(b""), 0);
		assert_eq!(murmur3(b"foo"), 0xf6a5c420);
		assert_eq!(murmur3(b"hello"), 0x248bfa47);
		assert_eq!(
			member_list_id(VIEW_CHANNEL, &[]).as_deref(),
			Some("everyone")
		);
		assert_eq!(member_list_id(0, &[]).as_deref(), Some("0"));
		let deny = Overwrite {
			id: Id(5),
			kind: 0,
			allow: 0,
			deny: VIEW_CHANNEL,
		};
		let allow = Overwrite {
			id: Id(9),
			kind: 1,
			allow: VIEW_CHANNEL | (1 << 100),
			deny: VIEW_CHANNEL,
		};
		assert_eq!(
			member_list_id(VIEW_CHANNEL, &[deny, allow]),
			Some(murmur3(b"allow:9,deny:5").to_string())
		);
		assert_eq!(
			member_list_id(0, &[allow, deny]),
			member_list_id(0, &[deny, allow])
		);
		assert_eq!(
			member_list_id(VIEW_CHANNEL, &[deny; MAX_OVERWRITES + 1]),
			None
		);
	}

	fn guild() -> Guild {
		Guild {
			id: Id(1),
			owner: Some(Id(99)),
			roles: Some(vec![
				Role {
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
					id: Id(1),
					bits: VIEW_CHANNEL
						| READ_MESSAGE_HISTORY
						| SEND_MESSAGES | ATTACH_FILES
						| (1 << 100),
				},
				Role {
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
					id: Id(2),
					bits: CONNECT,
				},
				Role {
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
					id: Id(3),
					bits: SPEAK | SEND_MESSAGES_IN_THREADS,
				},
			]),
			member: Some(Member {
				roles: vec![Id(2), Id(3)],
				timeout_until: None,
			}),
		}
	}

	#[test]
	fn aggregate_role_precedence_member_overwrite_and_timeout_follow_discord() {
		let mut guild = guild();
		let mut overwrites = vec![
			Overwrite {
				id: Id(7),
				kind: 1,
				allow: ADD_REACTIONS,
				deny: SEND_MESSAGES | CONNECT,
			},
			Overwrite {
				id: Id(2),
				kind: 0,
				allow: SEND_MESSAGES,
				deny: ATTACH_FILES | CONNECT | VIEW_CHANNEL,
			},
			Overwrite {
				id: Id(1),
				kind: 0,
				allow: 0,
				deny: SEND_MESSAGES | VIEW_CHANNEL,
			},
			Overwrite {
				id: Id(3),
				kind: 0,
				allow: VIEW_CHANNEL | ATTACH_FILES | CONNECT,
				deny: SEND_MESSAGES,
			},
		];
		let expected = VIEW_CHANNEL
			| READ_MESSAGE_HISTORY
			| ATTACH_FILES
			| SPEAK | ADD_REACTIONS
			| SEND_MESSAGES_IN_THREADS
			| (1 << 100);
		assert_eq!(
			effective(&guild, Id(7), Some(&overwrites), 100),
			Some(expected)
		);
		overwrites.reverse();
		guild.member.as_mut().unwrap().roles.reverse();
		assert_eq!(
			effective(&guild, Id(7), Some(&overwrites), 100),
			Some(expected),
			"Role and overwrite ordering must not change the aggregated result"
		);
		assert_eq!(expected & SEND_MESSAGES, 0);
		assert_ne!(
			expected & SEND_MESSAGES_IN_THREADS,
			0,
			"Thread send is an independent bit"
		);
		guild.member.as_mut().unwrap().timeout_until = Some(101);
		assert_eq!(
			effective(&guild, Id(7), Some(&overwrites), 100),
			Some(VIEW_CHANNEL | READ_MESSAGE_HISTORY)
		);
		assert_eq!(
			effective(&guild, Id(7), Some(&overwrites), 101),
			Some(expected),
			"Timeout expires at its boundary"
		);
		overwrites.iter_mut().find(|o| o.kind == 1).unwrap().deny |= VIEW_CHANNEL;
		assert_eq!(
			effective(&guild, Id(7), Some(&overwrites), 100),
			Some(READ_MESSAGE_HISTORY),
			"Timeout masks existing grants; it never grants access"
		);
		guild.roles.as_mut().unwrap()[1].bits |= ADMINISTRATOR;
		assert_eq!(
			effective(&guild, Id(7), None, 100),
			Some(u128::MAX),
			"Administrator bypasses timeout and channel overwrites"
		);
		guild.owner = Some(Id(7));
		guild.roles = None;
		guild.member = None;
		assert_eq!(
			effective(&guild, Id(7), None, 100),
			Some(u128::MAX),
			"Known ownership establishes the bypass directly"
		);
	}

	#[test]
	fn incomplete_invalid_and_oversized_metadata_remains_unknown() {
		let valid = guild();
		assert!(effective(&valid, Id(7), Some(&[]), 0).is_some());
		assert_eq!(effective(&valid, Id(7), None, 0), None);
		for invalid in [
			Guild {
				owner: None,
				..valid.clone()
			},
			Guild {
				roles: None,
				..valid.clone()
			},
			Guild {
				member: None,
				..valid.clone()
			},
			Guild {
				roles: Some(vec![
					Role {
						name: String::new(),
						color: 0,
						position: 0,
						hoist: false,
						id: Id(1),
						bits: 0
					};
					2
				]),
				..valid.clone()
			},
			Guild {
				roles: Some(vec![Role {
					name: String::new(),
					color: 0,
					position: 0,
					hoist: false,
					id: Id(2),
					bits: 0,
				}]),
				..valid.clone()
			},
			Guild {
				member: Some(Member {
					roles: vec![Id(2), Id(2)],
					timeout_until: None,
				}),
				..valid.clone()
			},
			Guild {
				member: Some(Member {
					roles: vec![Id(1)],
					timeout_until: None,
				}),
				..valid.clone()
			},
			Guild {
				member: Some(Member {
					roles: vec![Id(42)],
					timeout_until: None,
				}),
				..valid.clone()
			},
			Guild {
				roles: Some(
					(1..=MAX_ROLES + 1)
						.map(|id| Role {
							name: String::new(),
							color: 0,
							position: 0,
							hoist: false,
							id: Id(id as u64),
							bits: 0,
						})
						.collect(),
				),
				..valid.clone()
			},
		] {
			assert_eq!(effective(&invalid, Id(7), Some(&[]), 0), None);
		}
		let overwrite = Overwrite {
			id: Id(2),
			kind: 0,
			allow: SEND_MESSAGES,
			deny: 0,
		};
		for invalid in [
			vec![Overwrite {
				kind: 2,
				..overwrite
			}],
			vec![Overwrite {
				id: Id(42),
				..overwrite
			}],
			vec![overwrite; 2],
			vec![overwrite; MAX_OVERWRITES + 1],
		] {
			assert_eq!(effective(&valid, Id(7), Some(&invalid), 0), None);
		}
		let mut snapshot = Snapshot {
			guilds: vec![valid],
			channels: vec![Channel {
				id: Id(8),
				guild: Id(1),
				overwrites: Some(vec![overwrite]),
			}],
		};
		let before = snapshot.bytes();
		snapshot.guilds[0]
			.member
			.as_mut()
			.unwrap()
			.roles
			.reserve(50);
		snapshot.channels[0]
			.overwrites
			.as_mut()
			.unwrap()
			.reserve(50);
		snapshot.channels.reserve(50);
		assert!(
			snapshot.bytes() >= before + 50 * size_of::<Channel>(),
			"Accounting includes spare vector capacity and nested member/overwrite storage"
		);
	}
}
