//! Minimal, session-only permission metadata. No member directory is retained.
use crate::{DecodeError, Timestamp, decode, decode_gateway};
use model::{Id, Patch, permissions as p};
use serde::{
	Deserialize, Deserializer,
	de::{SeqAccess, Visitor},
};
use std::{collections::BTreeSet, marker::PhantomData};

const MAX_ITEMS: usize = model::account::MAX_ENTRIES;
const MAX_BYTES: usize = model::account::MAX_PERMISSION_BYTES;
const MAX_MEMBERS: usize = 4000;

pub(crate) struct List<T, const N: usize>(pub(crate) Vec<T>);
impl<T, const N: usize> Default for List<T, N> {
	fn default() -> Self {
		Self(Vec::new())
	}
}
impl<'de, T: Deserialize<'de>, const N: usize> Deserialize<'de> for List<T, N> {
	fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct Bounded<T, const N: usize>(PhantomData<T>);
		impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Bounded<T, N> {
			type Value = List<T, N>;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("bounded permission metadata")
			}
			fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
				let mut rows = Vec::new();
				while let Some(row) = seq.next_element()? {
					if rows.len() == N {
						return Err(serde::de::Error::custom(
							"Permission metadata capacity exceeded",
						));
					}
					rows.push(row);
				}
				Ok(List(rows))
			}
		}
		d.deserialize_seq(Bounded::<T, N>(PhantomData))
	}
}
pub(crate) fn member_roles<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Id>, D::Error> {
	let mut roles = List::<Id, { p::MAX_MEMBER_ROLES }>::deserialize(d)?.0;
	roles.sort_unstable();
	if roles.iter().any(|id| id.0 == 0) || roles.windows(2).any(|ids| ids[0] == ids[1]) {
		return Err(serde::de::Error::custom("Invalid member roles"));
	}
	roles.shrink_to_fit();
	Ok(roles)
}
#[derive(Deserialize)]
struct Bits(#[serde(deserialize_with = "bits")] u128);
pub(crate) fn bits<'de, D: Deserializer<'de>>(d: D) -> Result<u128, D::Error> {
	struct BitsVisitor;
	impl Visitor<'_> for BitsVisitor {
		type Value = u128;
		fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("a permission bit string")
		}
		fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<u128, E> {
			if value.is_empty() || value.len() > 39 || !value.bytes().all(|b| b.is_ascii_digit()) {
				return Err(E::custom("Invalid permission bits"));
			}
			value
				.parse()
				.map_err(|_| E::custom("Permission bits overflow"))
		}
	}
	d.deserialize_str(BitsVisitor)
}
#[derive(Deserialize)]
pub(crate) struct Role {
	id: Id,
	permissions: Bits,
	#[serde(default)]
	name: String,
	#[serde(default)]
	color: u32,
	#[serde(default)]
	colors: Option<RoleColors>,
	#[serde(default)]
	position: i32,
	#[serde(default)]
	hoist: bool,
}
#[derive(Deserialize)]
struct RoleColors {
	primary_color: u32,
}
impl Role {
	pub(crate) fn checked(self) -> Result<p::Role, DecodeError> {
		nonzero(self.id)?;
		let color = self
			.colors
			.map_or(self.color, |colors| colors.primary_color);
		if color > 0xff_ffff {
			return Err(DecodeError);
		}
		Ok(p::Role {
			id: self.id,
			bits: self.permissions.0,
			name: self
				.name
				.chars()
				.filter(|c| !c.is_control())
				.take(100)
				.collect(),
			color,
			position: self.position,
			hoist: self.hoist,
		})
	}
}
#[derive(Deserialize)]
struct Overwrite {
	id: Id,
	#[serde(rename = "type")]
	kind: u8,
	allow: Bits,
	deny: Bits,
}
fn overwrites(rows: List<Overwrite, 1000>, user: Id) -> Result<Vec<p::Overwrite>, DecodeError> {
	let mut seen = BTreeSet::new();
	let mut retained = Vec::new();
	for row in rows.0 {
		nonzero(row.id)?;
		if row.kind > 1 || !seen.insert((row.kind, row.id)) {
			return Err(DecodeError);
		}
		// Keep every role, including roles not currently assigned to the owner.
		if row.kind == 0 || row.id == user {
			retained.push(p::Overwrite {
				id: row.id,
				kind: row.kind,
				allow: row.allow.0,
				deny: row.deny.0,
			});
		}
	}
	retained.shrink_to_fit();
	Ok(retained)
}
#[derive(Deserialize)]
struct Identity {
	id: Id,
}
#[derive(Deserialize)]
struct Member {
	#[serde(default)]
	user: Option<Identity>,
	#[serde(default)]
	user_id: Option<Id>,
	#[serde(default)]
	roles: Patch<List<Id, 512>>,
	#[serde(default)]
	communication_disabled_until: Patch<Timestamp>,
}
impl Member {
	fn is_self(&self, user: Id) -> Result<bool, DecodeError> {
		let embedded = self.user.as_ref().map(|u| u.id);
		if embedded.zip(self.user_id).is_some_and(|(a, b)| a != b) {
			return Err(DecodeError);
		}
		let id = embedded.or(self.user_id).ok_or(DecodeError)?;
		nonzero(id)?;
		Ok(id == user)
	}
	fn patch(self, guild: Id) -> Result<MemberUpdate, DecodeError> {
		nonzero(guild)?;
		let roles = match self.roles {
			Patch::Value(rows) => {
				let mut seen = BTreeSet::new();
				if rows
					.0
					.iter()
					.any(|id| id.0 == 0 || *id == guild || !seen.insert(*id))
				{
					return Err(DecodeError);
				}
				Patch::Value(rows.0)
			}
			Patch::Null => Patch::Null,
			Patch::Absent => Patch::Absent,
		};
		let until = match self.communication_disabled_until {
			Patch::Value(t) => {
				// Round up so subsecond precision never ends a timeout early.
				let seconds =
					t.0.div_euclid(1_000_000_000) + i128::from(t.0.rem_euclid(1_000_000_000) != 0);
				Patch::Value(i64::try_from(seconds).map_err(|_| DecodeError)?)
			}
			Patch::Null => Patch::Null,
			Patch::Absent => Patch::Absent,
		};
		Ok((guild, roles, until))
	}
}
pub type MemberUpdate = (Id, Patch<Vec<Id>>, Patch<i64>);

#[derive(Default, Deserialize)]
struct Properties {
	#[serde(default)]
	owner_id: Patch<Id>,
}
#[derive(Deserialize)]
struct Guild {
	id: Id,
	#[serde(default)]
	owner_id: Patch<Id>,
	#[serde(default)]
	properties: Option<Properties>,
	#[serde(default)]
	roles: Option<List<Role, 512>>,
	#[serde(default)]
	members: List<Member, MAX_MEMBERS>,
	#[serde(default)]
	channels: List<Channel, MAX_ITEMS>,
}
#[derive(Deserialize)]
struct Channel {
	id: Id,
	#[serde(default)]
	guild_id: Option<Id>,
	#[serde(default)]
	flags: u64,
	#[serde(rename = "type", default)]
	kind: Option<u8>,
	#[serde(default)]
	permission_overwrites: Patch<List<Overwrite, 1000>>,
}
#[derive(Deserialize)]
struct Ready {
	#[serde(default)]
	guilds: List<Guild, MAX_ITEMS>,
	#[serde(default)]
	merged_members: Option<List<List<Member, MAX_MEMBERS>, MAX_ITEMS>>,
}
fn nonzero(id: Id) -> Result<(), DecodeError> {
	if id.0 == 0 { Err(DecodeError) } else { Ok(()) }
}
fn owner_patch(top: Patch<Id>, properties: Option<Properties>) -> Result<Patch<Id>, DecodeError> {
	let owner = match properties.map(|p| p.owner_id) {
		Some(Patch::Value(id)) => Patch::Value(id),
		Some(Patch::Null) => Patch::Null,
		_ => top,
	};
	if let Patch::Value(id) = owner {
		nonzero(id)?;
	}
	Ok(owner)
}
fn self_member(
	members: impl IntoIterator<Item = Member>,
	user: Id,
	guild: Id,
	full: bool,
) -> Result<Option<MemberUpdate>, DecodeError> {
	let mut selected = None;
	for member in members {
		if member.is_self(user)? {
			let mut update = member.patch(guild)?;
			if full && matches!(update.2, Patch::Absent) {
				update.2 = Patch::Null;
			}
			if selected.as_ref().is_some_and(|old| old != &update) {
				return Err(DecodeError);
			}
			selected = Some(update);
		}
	}
	Ok(selected)
}
fn checked_snapshot(
	guilds: impl IntoIterator<Item = Result<(Guild, Vec<Member>), DecodeError>>,
	user: Id,
) -> Result<p::Snapshot, DecodeError> {
	nonzero(user)?;
	let mut snapshot = p::Snapshot {
		guilds: Vec::new(),
		channels: Vec::new(),
	};
	let mut guild_ids = BTreeSet::new();
	let mut channel_ids = BTreeSet::new();
	let mut bytes = 0;
	let mut role_count = 0;
	let mut overwrite_count = 0;
	for row in guilds {
		let (guild, extra) = row?;
		nonzero(guild.id)?;
		if !guild_ids.insert(guild.id) {
			return Err(DecodeError);
		}
		let owner = match owner_patch(guild.owner_id, guild.properties)? {
			Patch::Value(id) => Some(id),
			_ => None,
		};
		let roles = guild
			.roles
			.map(|rows| {
				let mut seen = BTreeSet::new();
				rows.0
					.into_iter()
					.map(|r| {
						let role = r.checked()?;
						if !seen.insert(role.id) {
							return Err(DecodeError);
						}
						Ok(role)
					})
					.collect::<Result<Vec<_>, DecodeError>>()
			})
			.transpose()?;
		let member = self_member(
			guild.members.0.into_iter().chain(extra),
			user,
			guild.id,
			true,
		)?
		.and_then(|(_, roles, until)| match roles {
			Patch::Value(roles) => Some(p::Member {
				roles,
				timeout_until: match until {
					Patch::Value(t) => Some(t),
					_ => None,
				},
			}),
			_ => None,
		});
		role_count += roles.as_ref().map_or(0, Vec::len);
		let retained_guild = p::Guild {
			id: guild.id,
			owner,
			roles,
			member,
		};
		bytes += retained_guild.bytes();
		snapshot.guilds.push(retained_guild);
		for channel in guild.channels.0 {
			nonzero(channel.id)?;
			if !channel_ids.insert(channel.id) || channel.guild_id.is_some_and(|id| id != guild.id)
			{
				return Err(DecodeError);
			}
			let overwrites = match channel.permission_overwrites {
				Patch::Value(rows) => Some(overwrites(rows, user)?),
				_ => None,
			};
			// Threads use the admitted parent's overwrites; hidden channels never become permission sources.
			if channel.flags & (1 << 17) == 0 && !matches!(channel.kind, Some(10..=12)) {
				overwrite_count += overwrites.as_ref().map_or(0, Vec::len);
				let retained_channel = p::Channel {
					id: channel.id,
					guild: guild.id,
					overwrites,
				};
				bytes += retained_channel.bytes();
				snapshot.channels.push(retained_channel);
			}
		}
		if guild_ids.len() + channel_ids.len() > MAX_ITEMS
			|| bytes > MAX_BYTES
			|| role_count > model::account::MAX_ROLES
			|| overwrite_count > model::account::MAX_OVERWRITES
		{
			return Err(DecodeError);
		}
	}
	snapshot.guilds.shrink_to_fit();
	snapshot.channels.shrink_to_fit();
	if snapshot.bytes() > MAX_BYTES {
		return Err(DecodeError);
	}
	Ok(snapshot)
}
/// Decode separately from navigation: missing metadata must remain unknown, and voice may consume members.
pub fn ready(bytes: &[u8], user: Id) -> Result<p::Snapshot, DecodeError> {
	#[derive(Deserialize)]
	struct Fields<'a> {
		#[serde(default = "crate::ready::empty_array", borrow)]
		guilds: &'a serde_json::value::RawValue,
		#[serde(default, borrow)]
		merged_members: Option<&'a serde_json::value::RawValue>,
	}
	if bytes.len() > crate::MAX_GATEWAY_WIRE {
		return Err(DecodeError);
	}
	let fields: Fields<'_> = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	ready_fields(
		fields.guilds.get().as_bytes(),
		fields.merged_members.map(|m| m.get().as_bytes()),
		user,
	)
}
/// The gateway envelope is parsed once; each consumer validates only its own fields.
pub fn ready_fields(
	guilds: &[u8],
	merged: Option<&[u8]>,
	user: Id,
) -> Result<p::Snapshot, DecodeError> {
	if guilds.len() > crate::MAX_GATEWAY_WIRE
		|| merged.is_some_and(|m| m.len() > crate::MAX_GATEWAY_WIRE)
	{
		return Err(DecodeError);
	}
	let guilds: List<&serde_json::value::RawValue, MAX_ITEMS> =
		serde_json::from_slice(guilds).map_err(|_| DecodeError)?;
	let merged: Option<List<&serde_json::value::RawValue, MAX_ITEMS>> = merged
		.map(serde_json::from_slice)
		.transpose()
		.map_err(|_| DecodeError)?;
	if merged
		.as_ref()
		.is_some_and(|rows| rows.0.len() != guilds.0.len())
	{
		return Err(DecodeError);
	}
	checked_snapshot(
		guilds.0.into_iter().enumerate().map(|(index, guild)| {
			let guild: Guild = decode_gateway(guild.get().as_bytes())?;
			let members: List<Member, MAX_MEMBERS> = merged
				.as_ref()
				.map(|rows| decode_gateway(rows.0[index].get().as_bytes()))
				.transpose()?
				.unwrap_or_default();
			Ok((guild, members.0))
		}),
		user,
	)
}
pub fn guild(bytes: &[u8], user: Id) -> Result<p::Snapshot, DecodeError> {
	checked_snapshot([Ok((decode(bytes)?, Vec::new()))], user)
}
pub fn role(bytes: &[u8]) -> Result<(Id, p::Role), DecodeError> {
	#[derive(Deserialize)]
	struct Update {
		guild_id: Id,
		role: Role,
	}
	let update: Update = decode(bytes)?;
	nonzero(update.guild_id)?;
	Ok((update.guild_id, update.role.checked()?))
}
pub fn role_removed(bytes: &[u8]) -> Result<(Id, Id), DecodeError> {
	#[derive(Deserialize)]
	struct Update {
		guild_id: Id,
		role_id: Id,
	}
	let update: Update = decode(bytes)?;
	nonzero(update.guild_id)?;
	nonzero(update.role_id)?;
	Ok((update.guild_id, update.role_id))
}
pub fn owner(bytes: &[u8]) -> Result<Option<(Id, Patch<Id>)>, DecodeError> {
	#[derive(Deserialize)]
	struct Update {
		id: Id,
		#[serde(default)]
		owner_id: Patch<Id>,
		#[serde(default)]
		properties: Option<Properties>,
	}
	let update: Update = decode(bytes)?;
	nonzero(update.id)?;
	let owner = owner_patch(update.owner_id, update.properties)?;
	Ok((!matches!(owner, Patch::Absent)).then_some((update.id, owner)))
}
pub fn member(bytes: &[u8], user: Id) -> Result<Option<MemberUpdate>, DecodeError> {
	#[derive(Deserialize)]
	struct Update {
		guild_id: Id,
		#[serde(flatten)]
		member: Member,
	}
	let update: Update = decode(bytes)?;
	nonzero(update.guild_id)?;
	self_member([update.member], user, update.guild_id, false)
}
pub fn passive(bytes: &[u8], user: Id) -> Result<Option<MemberUpdate>, DecodeError> {
	#[derive(Deserialize)]
	struct Update {
		#[serde(default)]
		guild_id: Option<Id>,
		#[serde(default)]
		updated_members: List<Member, MAX_MEMBERS>,
	}
	let update: Update = decode(bytes)?;
	passive_member(update.guild_id, update.updated_members.0, user)
}
pub fn passive_fields(
	guild: Option<Id>,
	members: &[u8],
	user: Id,
) -> Result<Option<MemberUpdate>, DecodeError> {
	let members: List<Member, MAX_MEMBERS> = decode(members)?;
	passive_member(guild, members.0, user)
}
fn passive_member(
	guild: Option<Id>,
	members: Vec<Member>,
	user: Id,
) -> Result<Option<MemberUpdate>, DecodeError> {
	match guild {
		Some(guild) => self_member(members, user, guild, false),
		None if members.is_empty() => Ok(None),
		None => Err(DecodeError),
	}
}

pub fn supplemental(bytes: &[u8], user: Id) -> Result<Vec<MemberUpdate>, DecodeError> {
	let ready: Ready = decode_gateway(bytes)?;
	if ready
		.merged_members
		.as_ref()
		.is_some_and(|m| m.0.len() != ready.guilds.0.len())
	{
		return Err(DecodeError);
	}
	let mut merged = ready.merged_members.map(|m| m.0);
	let mut updates = Vec::new();
	let mut ids = BTreeSet::new();
	for (index, guild) in ready.guilds.0.into_iter().enumerate() {
		nonzero(guild.id)?;
		if !ids.insert(guild.id) {
			return Err(DecodeError);
		}
		let extra = merged
			.as_mut()
			.map(|rows| std::mem::take(&mut rows[index].0))
			.unwrap_or_default();
		if let Some(update) = self_member(
			guild.members.0.into_iter().chain(extra),
			user,
			guild.id,
			true,
		)? {
			updates.push(update);
		}
	}
	let bytes = updates.capacity() * size_of::<MemberUpdate>()
		+ updates
			.iter()
			.map(|(_, roles, _)| match roles {
				Patch::Value(roles) => roles.capacity() * size_of::<Id>(),
				_ => 0,
			})
			.sum::<usize>();
	if bytes > MAX_BYTES {
		return Err(DecodeError);
	}
	Ok(updates)
}
pub struct ChannelUpdate {
	pub id: Id,
	pub guild: Option<Id>,
	pub overwrites: Patch<Vec<p::Overwrite>>,
}
pub fn channel(bytes: &[u8], user: Id) -> Result<Option<ChannelUpdate>, DecodeError> {
	let channel: Channel = decode(bytes)?;
	nonzero(channel.id)?;
	if let Some(guild) = channel.guild_id {
		nonzero(guild)?;
	}
	let overwrites = match channel.permission_overwrites {
		Patch::Value(rows) => Patch::Value(overwrites(rows, user)?),
		Patch::Null => Patch::Null,
		Patch::Absent => return Ok(None),
	};
	Ok(
		(channel.flags & (1 << 17) == 0 && !matches!(channel.kind, Some(10..=12))).then_some(
			ChannelUpdate {
				id: channel.id,
				guild: channel.guild_id,
				overwrites,
			},
		),
	)
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	#[test]
	fn role_display_metadata_accepts_modern_and_legacy_colors_with_bounded_names() {
		let snapshot = ready(br#"{"guilds":[{"id":"1","roles":[{"id":"1","permissions":"1024"},{"id":"2","permissions":"0","name":"Moderators","color":1122867,"position":3,"hoist":true}]}]}"#, Id(9)).unwrap();
		let role = &snapshot.guilds[0].roles.as_ref().unwrap()[1];
		assert_eq!(
			(role.name.as_str(), role.color, role.position, role.hoist),
			("Moderators", 0x112233, 3, true)
		);
		let payload = json!({"guild_id":"1","role":{"id":"2","permissions":"0","name":format!("\n{}", "é".repeat(120)),"color":1122867,"colors":{"primary_color":4478310,"secondary_color":null},"position":4,"hoist":false}});
		let (_, updated) = super::role(&serde_json::to_vec(&payload).unwrap()).unwrap();
		assert_eq!(updated.name.chars().count(), 100);
		assert_eq!(
			(updated.color, updated.position, updated.hoist),
			(0x445566, 4, false)
		);
		assert!(updated.bytes() >= size_of::<p::Role>() + 200);
		let (_, uncolored) = super::role(br#"{"guild_id":"1","role":{"id":"2","permissions":"0","color":1122867,"colors":{"primary_color":0}}}"#).unwrap();
		assert_eq!(uncolored.color, 0);
		assert!(
			super::role(
				br#"{"guild_id":"1","role":{"id":"2","permissions":"0","color":16777216}}"#
			)
			.is_err()
		);
	}

	#[test]
	fn ready_self_metadata_is_aligned_scoped_and_keeps_future_role_overwrites() {
		let snapshot = ready(br#"{"guilds":[{"id":"1","properties":{"owner_id":"7"},
            "roles":[{"id":"1","permissions":"1024"},{"id":"2","permissions":"2048"},{"id":"3","permissions":"0"}],
            "members":[{"user":{"id":"8"},"roles":[]}],
            "channels":[{"id":"4","type":0,"permission_overwrites":[
                {"id":"3","type":0,"allow":"0","deny":"2048"},
                {"id":"8","type":1,"allow":"8","deny":"0"},
                {"id":"9","type":1,"allow":"32768","deny":"0"}]},
                {"id":"5","type":0,"flags":131072,"permission_overwrites":[]},
                {"id":"6","type":11,"permission_overwrites":[]}]},
            {"id":"10"}],"merged_members":[[{"user_id":"9","roles":["2"],"communication_disabled_until":"2026-01-01T00:00:00.1Z"}],[]]}"#, Id(9)).unwrap();
		assert_eq!(snapshot.guilds[0].owner, Some(Id(7)));
		assert_eq!(
			snapshot.guilds[0].member.as_ref().unwrap().roles,
			vec![Id(2)]
		);
		let until = snapshot.guilds[0]
			.member
			.as_ref()
			.unwrap()
			.timeout_until
			.unwrap();
		assert_eq!(until, 1_767_225_601);
		assert!(
			snapshot.guilds[1].owner.is_none()
				&& snapshot.guilds[1].roles.is_none()
				&& snapshot.guilds[1].member.is_none()
		);
		assert_eq!(snapshot.channels.len(), 1);
		let overwrites = snapshot.channels[0].overwrites.as_ref().unwrap();
		assert_eq!(
			overwrites
				.iter()
				.map(|o| (o.kind, o.id))
				.collect::<Vec<_>>(),
			vec![(0, Id(3)), (1, Id(9))]
		);
		let known_empty = ready(br#"{"guilds":[{"id":"1","roles":[],"members":[{"user_id":"9","roles":[]}],"channels":[{"id":"2","type":0,"permission_overwrites":[]}]}]}"#,Id(9)).unwrap();
		assert_eq!(known_empty.guilds[0].roles, Some(Vec::new()));
		assert_eq!(
			known_empty.guilds[0].member.as_ref().unwrap().timeout_until,
			None
		);
		assert_eq!(known_empty.channels[0].overwrites, Some(Vec::new()));
	}

	#[test]
	fn member_owner_and_channel_updates_preserve_absent_null_and_explicit_empty() {
		let (_, roles, until) = member(br#"{"guild_id":"1","user":{"id":"9"},"roles":[]}"#, Id(9))
			.unwrap()
			.unwrap();
		assert_eq!(roles, Patch::Value(Vec::new()));
		assert_eq!(until, Patch::Absent);
		let (_, roles, until) = member(
			br#"{"guild_id":"1","user_id":"9","communication_disabled_until":null}"#,
			Id(9),
		)
		.unwrap()
		.unwrap();
		assert_eq!(roles, Patch::Absent);
		assert_eq!(until, Patch::Null);
		assert_eq!(
			member(br#"{"guild_id":"1","user_id":"9","roles":null}"#, Id(9))
				.unwrap()
				.unwrap()
				.1,
			Patch::Null
		);
		assert!(
			member(br#"{"guild_id":"1","user":{"id":"8"},"roles":[]}"#, Id(9))
				.unwrap()
				.is_none()
		);
		assert!(
			member(
				br#"{"guild_id":"1","user":{"id":"9"},"user_id":"8","roles":[]}"#,
				Id(9)
			)
			.is_err()
		);
		assert!(owner(br#"{"id":"1","name":"renamed"}"#).unwrap().is_none());
		assert_eq!(
			owner(br#"{"id":"1","owner_id":"7","properties":{"owner_id":null}}"#).unwrap(),
			Some((Id(1), Patch::Null))
		);
		let update = channel(br#"{"id":"2","permission_overwrites":[]}"#, Id(9))
			.unwrap()
			.unwrap();
		assert_eq!(update.guild, None);
		assert_eq!(update.overwrites, Patch::Value(Vec::new()));
		assert_eq!(
			channel(
				br#"{"id":"2","guild_id":"1","permission_overwrites":null}"#,
				Id(9)
			)
			.unwrap()
			.unwrap()
			.overwrites,
			Patch::Null
		);
		assert!(
			channel(br#"{"id":"2","name":"renamed"}"#, Id(9))
				.unwrap()
				.is_none()
		);
		assert!(
			channel(
				br#"{"id":"2","flags":131072,"permission_overwrites":[]}"#,
				Id(9)
			)
			.unwrap()
			.is_none()
		);
	}

	#[test]
	fn malformed_bits_scope_and_discarded_overwrites_never_grant() {
		let (_, high) =
			role(br#"{"guild_id":"1","role":{"id":"2","permissions":"18446744073709551616"}}"#)
				.unwrap();
		assert_eq!(high.bits, 1_u128 << 64);
		for bits in [
			"",
			"+8",
			"-1",
			" 8",
			"1.0",
			"340282366920938463463374607431768211456",
		] {
			let bytes =
				serde_json::to_vec(&json!({"guild_id":"1","role":{"id":"2","permissions":bits}}))
					.unwrap();
			assert!(role(&bytes).is_err());
		}
		for row in [
			json!({"id":"8","type":2,"allow":"8","deny":"0"}),
			json!({"id":"0","type":1,"allow":"8","deny":"0"}),
			json!({"id":"8","type":1,"allow":"invalid","deny":"0"}),
			json!({"id":"8","allow":"8","deny":"0"}),
		] {
			let bytes =
				serde_json::to_vec(&json!({"id":"2","permission_overwrites":[row]})).unwrap();
			assert!(
				channel(&bytes, Id(9)).is_err(),
				"Validate before dropping another member's overwrite"
			);
		}
		for value in [
			json!({"guilds":[{"id":"1"}],"merged_members":[]}),
			json!({"guilds":[{"id":"1","channels":[{"id":"2","guild_id":"3","type":0}]}]}),
			json!({"guilds":[{"id":"1","roles":[{"id":"1","permissions":"8"},{"id":"1","permissions":"0"}]}]}),
			json!({"guilds":[{"id":"1","members":[{"user_id":"9","roles":["2","2"]}]}]}),
			json!({"guilds":[{"id":"1","members":[{"user_id":"9","roles":[]},{"user_id":"9","roles":["2"]}]}]}),
		] {
			assert!(ready(&serde_json::to_vec(&value).unwrap(), Id(9)).is_err());
		}
	}

	#[test]
	fn supplemental_and_passive_select_only_self_without_member_fanout() {
		let updates = supplemental(br#"{"guilds":[{"id":"1"},{"id":"2"}],"merged_members":[[{"user_id":"8","roles":[]},{"user_id":"9","roles":["3"]}],[{"user":{"id":"9"},"roles":[]}]]}"#,Id(9)).unwrap();
		assert_eq!(
			updates,
			vec![
				(Id(1), Patch::Value(vec![Id(3)]), Patch::Null),
				(Id(2), Patch::Value(Vec::new()), Patch::Null)
			]
		);
		assert!(supplemental(br#"{"guilds":[{"id":"1"}],"merged_members":[]}"#, Id(9)).is_err());
		assert_eq!(passive(br#"{"guild_id":"1","updated_members":[{"user":{"id":"9"},"communication_disabled_until":null}]}"#,Id(9)).unwrap(),Some((Id(1),Patch::Absent,Patch::Null)));
		assert!(
			passive(br#"{"updated_channels":[]}"#, Id(9))
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn metadata_limits_include_discarded_rows_and_aggregate_roles() {
		let roles: Vec<_> = (1..=513)
			.map(|id| json!({"id":id.to_string(),"permissions":"0"}))
			.collect();
		assert!(
			ready(
				&serde_json::to_vec(&json!({"guilds":[{"id":"1","roles":roles}]})).unwrap(),
				Id(9)
			)
			.is_err()
		);
		let rows: Vec<_> = (10..1011)
			.map(|id| json!({"id":id.to_string(),"type":1,"allow":"0","deny":"0"}))
			.collect();
		assert!(
			channel(
				&serde_json::to_vec(&json!({"id":"2","permission_overwrites":rows})).unwrap(),
				Id(9)
			)
			.is_err()
		);
		let ids: Vec<_> = (10..523).map(|id| id.to_string()).collect();
		assert!(
			member(
				&serde_json::to_vec(&json!({"guild_id":"1","user_id":"9","roles":ids})).unwrap(),
				Id(9)
			)
			.is_err()
		);
		let guilds: Vec<_> = (1..=33).map(|guild| json!({"id":guild.to_string(),"roles":(100..612).map(|id| json!({"id":id.to_string(),"permissions":"0"})).collect::<Vec<_>>()})).collect();
		let bytes = serde_json::to_vec(&json!({"guilds":guilds})).unwrap();
		assert!(bytes.len() < crate::MAX_WIRE);
		assert!(ready(&bytes, Id(9)).is_ok());
		let channels: Vec<_> = (10..43).map(|id| json!({"id":id.to_string(),"type":0,"permission_overwrites":
            (100..1100).map(|role| json!({"id":role.to_string(),"type":0,"allow":"0","deny":"0"})).collect::<Vec<_>>()
        })).collect();
		let bytes =
			serde_json::to_vec(&json!({"guilds":[{"id":"1","channels":channels}]})).unwrap();
		assert!(bytes.len() < crate::MAX_WIRE);
		assert!(
			ready(&bytes, Id(9)).is_ok(),
			"Normal accounts can exceed the old aggregate overwrite and byte limits"
		);
	}
}
