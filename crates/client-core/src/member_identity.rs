//! Small live identity overlay shared by the sidebar, composer, messages and typing.
//! No permissions or complete directory are derived from these display observations.
use crate::{State, member_search};
use model::{Id, Member, MemberDisplay, MemberIdentity, MemberSlot, Patch, User};
use std::{
	collections::VecDeque,
	time::{Duration, Instant},
};

const LIMIT: usize = 128;
const HINT_LIMIT: usize = 128;
const RETIRED_LIMIT: usize = 128;
const MAX_BYTES: usize = 256 * 1024;
const MEMBER_LIMIT: usize = 512;
const MEMBER_BYTES: usize = 384 * 1024;

enum ObservedUser<'a> {
	Profile(&'a User),
	Nickname(Id, &'a str),
}

impl<'a> ObservedUser<'a> {
	fn id(&self) -> Id {
		match self {
			Self::Profile(user) => user.id,
			Self::Nickname(id, _) => *id,
		}
	}

	fn into_user(self) -> std::borrow::Cow<'a, User> {
		match self {
			Self::Profile(user) => std::borrow::Cow::Borrowed(user),
			Self::Nickname(id, nick) => std::borrow::Cow::Owned(User {
				member_avatar: None,
				id,
				name: nick.to_owned(),
				username: None,
				avatar: None,
				discriminator: 0,
				primary_guild: None,
				cosmetics: None,
				webhook: false,
				kind: Default::default(),
			}),
		}
	}
}

#[derive(Default)]
pub struct Labels {
	entries: VecDeque<(u64, MemberIdentity)>,
	// Keep bounded per-user guards after their larger live observations are evicted.
	retired: VecDeque<(u64, Id, Id)>,
	// The last flag distinguishes complete query/sync observations from sidebar rows,
	// which may omit nickname and role fields on their first appearance.
	members: VecDeque<(Id, Member, Instant, bool)>,
	member_bytes: usize,
	visible_scope: Option<(Id, Id)>,
	visible_users: Vec<Id>,
	watch_revision: u64,
	revision: u64,
	// Reads older than discarded live guards cannot safely install disk hints.
	disk_read_floor: u64,
	display_revision: u64,
	bytes: usize,
}

impl Labels {
	pub(crate) fn clear(&mut self) {
		let display_revision = self.display_revision.wrapping_add(1);
		let watch_revision = self.watch_revision.wrapping_add(1);
		let revision = self.revision.wrapping_add(1);
		*self = Self::default();
		self.display_revision = display_revision;
		self.watch_revision = watch_revision;
		self.revision = revision;
		self.disk_read_floor = revision;
	}
	pub(crate) fn revision(&self) -> u64 {
		self.revision
	}
	fn retire(&mut self, guild: Id, user: Id) {
		self.revision = self.revision.wrapping_add(1);
		if self.retired.len() == RETIRED_LIMIT {
			let (revision, _, _) = self.retired.pop_front().expect("retired guard overflow");
			self.disk_read_floor = revision;
		}
		self.retired.push_back((self.revision, guild, user));
	}
	fn retired_after(&self, guild: Id, user: Id, started: u64) -> bool {
		self.retired
			.iter()
			.any(|(revision, g, u)| *revision > started && *g == guild && *u == user)
	}
	pub fn bytes(&self) -> usize {
		self.bytes
			+ self.entries.capacity() * size_of::<(u64, MemberIdentity)>()
			+ self.retired.capacity() * size_of::<(u64, Id, Id)>()
			+ self.member_bytes
			+ self.members.capacity() * size_of::<(Id, Member, Instant, bool)>()
			+ self.visible_users.capacity() * size_of::<Id>()
	}
	pub(crate) fn watch_visible(&mut self, guild: Id, channel: Id, users: &[Id]) {
		if self.visible_scope == Some((guild, channel))
			&& self.visible_users.iter().copied().eq(users
				.iter()
				.copied()
				.filter(|id| id.0 != 0)
				.take(member_search::LIMIT))
		{
			return;
		}
		self.visible_scope = Some((guild, channel));
		self.visible_users.clear();
		self.visible_users.extend(
			users
				.iter()
				.copied()
				.filter(|id| id.0 != 0)
				.take(member_search::LIMIT),
		);
		self.watch_revision = self.watch_revision.wrapping_add(1);
	}
	fn overlay_bytes(&self) -> usize {
		self.bytes + self.entries.capacity() * size_of::<(u64, MemberIdentity)>()
	}
	pub(crate) fn member(&self, guild: Id, user: Id) -> Option<&Member> {
		self.members
			.iter()
			.rev()
			.find(|(g, m, _, _)| *g == guild && m.user.id == user)
			.map(|(_, m, _, _)| m)
	}
	fn fill_partial_nickname(&mut self, guild: Id, user: Id, nick: Option<&str>) -> bool {
		let Some(nick) = nick.filter(|nick| !nick.is_empty()) else {
			return false;
		};
		if self
			.get(guild, user)
			.is_some_and(|identity| identity.departed || !matches!(identity.nick, Patch::Absent))
		{
			return false;
		}
		let Some((_, member, _, false)) = self
			.members
			.iter_mut()
			.rev()
			.find(|(g, member, _, _)| *g == guild && member.user.id == user)
		else {
			return false;
		};
		if member.nick.is_some() {
			return false;
		}
		self.member_bytes -= member.bytes();
		member.nick = Some(nick.to_owned());
		self.member_bytes += member.bytes();
		self.trim_members();
		true
	}
	pub(crate) fn verified_member(&self, guild: Id, user: Id) -> Option<&Member> {
		self.members
			.iter()
			.rev()
			.find_map(|(g, member, _, verified)| {
				(*g == guild && member.user.id == user && *verified).then_some(member)
			})
	}
	pub(crate) fn member_fresh(&self, guild: Id, user: Id) -> bool {
		self.members.iter().any(|(g, m, at, verified)| {
			*g == guild
				&& m.user.id == user
				&& *verified && at.elapsed() < Duration::from_secs(5 * 60)
		})
	}
	pub(crate) fn remember(&mut self, guild: Id, rows: &[Member]) {
		self.remember_with_source(guild, rows, true);
	}
	pub(crate) fn remember_with_source(&mut self, guild: Id, rows: &[Member], verified: bool) {
		let mut changed = false;
		let mut display_changed = false;
		for member in rows {
			let mut prior_verified = None;
			if let Some(index) = self
				.members
				.iter()
				.position(|(g, m, _, _)| *g == guild && m.user.id == member.user.id)
			{
				let (_, old, observed, was_verified) = &mut self.members[index];
				display_changed |= old.user.name != member.user.name
					|| old.user.username != member.user.username
					|| old.nick != member.nick;
				if old.user == member.user && old.roles == member.roles && old.nick == member.nick {
					if verified {
						*observed = Instant::now();
						*was_verified = true;
					}
					continue;
				}
				let (_, old, observed, was_verified) = self.members.remove(index).unwrap();
				if !verified && was_verified {
					prior_verified = Some(observed);
				}
				self.member_bytes -= old.bytes();
			} else {
				display_changed = true;
			}
			// Presence remains owned by the current subscription, with no transient allocations copied.
			let member = Member {
				user: member.user.clone(),
				roles: member.roles.clone(),
				nick: member.nick.clone(),
				status: None,
				custom_status: None,
				activities: vec![],
				clients: Default::default(),
			};
			self.member_bytes += member.bytes();
			self.members.push_back((
				guild,
				member,
				prior_verified.unwrap_or_else(Instant::now),
				verified || prior_verified.is_some(),
			));
			changed = true;
		}
		if changed {
			self.revision = self.revision.wrapping_add(1);
		}
		if display_changed {
			self.display_revision = self.display_revision.wrapping_add(1);
		}
		if verified {
			for (revision, entry) in &mut self.entries {
				if *revision == 0
					&& entry.guild == guild
					&& rows.iter().any(|row| row.user.id == entry.user)
				{
					*revision = self.revision;
				}
			}
		}
		self.trim_members();
		self.trim_overlay();
	}

	fn trim_members(&mut self) {
		while self.members.len() > MEMBER_LIMIT
			|| self.member_bytes
				+ self.members.capacity() * size_of::<(Id, Member, Instant, bool)>()
				> MEMBER_BYTES
		{
			let Some((guild, old, _, _)) = self.members.pop_front() else {
				break;
			};
			self.member_bytes -= old.bytes();
			self.retire(guild, old.user.id);
		}
	}
	pub(crate) fn get(&self, guild: Id, user: Id) -> Option<&MemberIdentity> {
		self.entries
			.iter()
			.rev()
			.find(|(_, row)| row.guild == guild && row.user == user)
			.map(|(_, row)| row)
	}
	pub(crate) fn global_name_after(
		&self,
		guild: Id,
		user: Id,
		started: u64,
	) -> Option<model::GlobalNameUpdate> {
		self.entries.iter().rev().find_map(|(revision, row)| {
			(*revision > started && row.guild == guild && row.user == user && row.profile.is_some())
				.then_some(row.global_name)
		})
	}
	fn known_global_name(&self, guild: Id, user: Id) -> Option<model::GlobalNameUpdate> {
		let (revision, row) = self
			.entries
			.iter()
			.rev()
			.find(|(_, row)| row.guild == guild && row.user == user)?;
		if *revision == 0 {
			return None;
		}
		if row.departed {
			return Some(model::GlobalNameUpdate::Unchanged);
		}
		(row.global_name != model::GlobalNameUpdate::Unchanged).then_some(row.global_name)
	}
	/// Disk hints predate every live observation, including queries already in flight.
	fn insert_hint(&mut self, update: MemberIdentity) {
		self.bytes += update.bytes();
		self.entries.push_back((0, update));
	}
	fn trim_overlay(&mut self) {
		if self.entries.len() <= LIMIT && self.overlay_bytes() <= MAX_BYTES {
			return;
		}
		let mut hints = self
			.entries
			.iter()
			.filter(|(revision, _)| *revision == 0)
			.count();
		while self.entries.len().saturating_sub(hints) > LIMIT {
			let Some(index) = self.entries.iter().position(|(revision, _)| *revision != 0) else {
				break;
			};
			let (_, old) = self
				.entries
				.remove(index)
				.expect("live identity index checked");
			self.bytes -= old.bytes();
			self.retire(old.guild, old.user);
		}
		while hints > HINT_LIMIT || self.overlay_bytes() > MAX_BYTES {
			let index = self
				.entries
				.iter()
				.position(|(revision, _)| *revision == 0)
				.unwrap_or(0);
			let Some((revision, old)) = self.entries.remove(index) else {
				break;
			};
			hints -= usize::from(revision == 0);
			self.bytes -= old.bytes();
			if revision != 0 {
				self.retire(old.guild, old.user);
			}
		}
	}
	fn insert(&mut self, mut update: MemberIdentity, display_changed: bool) {
		if let Some(index) = self
			.entries
			.iter()
			.position(|(_, old)| old.guild == update.guild && old.user == update.user)
		{
			let (revision, old) = self.entries.remove(index).unwrap();
			self.bytes -= old.bytes();
			// A partial live update must not promote missing fields from an older disk hint.
			if revision != 0 && !update.departed && !old.departed {
				if update.global_name == model::GlobalNameUpdate::Unchanged {
					if let (Some(profile), Some(previous)) = (&mut update.profile, &old.profile) {
						match old.global_name {
							model::GlobalNameUpdate::Set => profile.name.clone_from(&previous.name),
							model::GlobalNameUpdate::Cleared => {
								if let Some(username) = &profile.username {
									profile.name = username.to_string();
								}
							}
							model::GlobalNameUpdate::Unchanged => {}
						}
					}
					update.global_name = old.global_name;
				}
				if update.profile.is_none() {
					update.profile = old.profile;
				}
				if matches!(update.nick, Patch::Absent) {
					update.nick = old.nick;
				}
				if matches!(update.roles, Patch::Absent) {
					update.roles = old.roles;
				}
				if matches!(update.avatar, Patch::Absent) {
					update.avatar = old.avatar;
				}
			}
		}
		if let Some(index) = self
			.members
			.iter()
			.position(|(g, m, _, _)| *g == update.guild && m.user.id == update.user)
		{
			let (guild, mut member, observed, verified) = self.members.remove(index).unwrap();
			self.member_bytes -= member.bytes();
			if !update.departed {
				update.apply(&mut member);
				self.member_bytes += member.bytes();
				self.members.push_back((guild, member, observed, verified));
			}
		}
		self.trim_members();
		self.revision = self.revision.wrapping_add(1);
		if display_changed {
			self.display_revision = self.display_revision.wrapping_add(1);
		}
		self.bytes += update.bytes();
		self.entries.push_back((self.revision, update));
		self.trim_overlay();
	}
	pub(crate) fn reconcile_with_missing(
		&mut self,
		guild: Id,
		rows: &mut Vec<Member>,
		started: u64,
		missing: &[model::MissingMemberFields],
	) {
		if self.entries.is_empty() && missing.is_empty() {
			return;
		}
		let mut display_changed = false;
		rows.retain_mut(|member| {
			let fields = missing.iter().find(|fields| fields.user == member.user.id);
			if let Some(fields) = fields
				&& let Some(known) = self.verified_member(guild, member.user.id)
			{
				if fields.global_name == model::GlobalNameUpdate::Unchanged {
					member.user.name.clone_from(&known.user.name);
				}
				if fields.nick {
					member.nick.clone_from(&known.nick);
				}
				if fields.roles {
					member.roles.clone_from(&known.roles);
				}
			}
			if let Some((revision, update)) = self
				.entries
				.iter_mut()
				.find(|(_, update)| update.guild == guild && update.user == member.user.id)
			{
				if *revision > started {
					if update.departed {
						return false;
					}
					update.apply(member);
				} else {
					if let Some(fields) = fields {
						if fields.global_name == model::GlobalNameUpdate::Unchanged {
							if update.global_name == model::GlobalNameUpdate::Cleared {
								if let Some(username) = &member.user.username {
									member.user.name = username.to_string();
								}
							} else if let Some(profile) = &update.profile {
								member.user.name.clone_from(&profile.name);
							}
						}
						if fields.nick {
							match &update.nick {
								Patch::Value(nick) => {
									member.nick = (!nick.is_empty()).then(|| nick.clone())
								}
								Patch::Null => member.nick = None,
								Patch::Absent => {}
							}
						}
						if fields.roles
							&& let Patch::Value(roles) = &update.roles
						{
							member.roles.clone_from(roles);
						}
					}
					let nick = match &update.nick {
						Patch::Value(nick) if !nick.is_empty() => Some(nick.as_str()),
						_ => None,
					};
					display_changed |= update.departed
						|| update.profile.as_ref().is_none_or(|profile| {
							(fields.is_none_or(|fields| {
								fields.global_name != model::GlobalNameUpdate::Unchanged
							}) && profile.name != member.user.name)
								|| profile.username != member.user.username
						}) || (fields.is_none_or(|fields| !fields.nick)
						&& (matches!(update.nick, Patch::Absent)
							|| nick != member.nick.as_deref().filter(|nick| !nick.is_empty())));
					self.bytes -= update.bytes();
					update.profile = Some(member.user.clone());
					let global_name =
						fields.map_or(model::GlobalNameUpdate::Set, |fields| fields.global_name);
					if global_name != model::GlobalNameUpdate::Unchanged {
						update.global_name = global_name;
					}
					if fields.is_none_or(|fields| !fields.nick) {
						update.nick = member.nick.clone().map_or(Patch::Null, Patch::Value);
					}
					if fields.is_none_or(|fields| !fields.roles) {
						update.roles = Patch::Value(member.roles.clone());
					}
					update.avatar = member
						.user
						.member_avatar
						.as_ref()
						.filter(|avatar| avatar.guild == guild)
						.map_or(Patch::Absent, |avatar| {
							avatar.hash.clone().map_or(Patch::Null, Patch::Value)
						});
					update.departed = false;
					self.bytes += update.bytes();
				}
			}
			true
		});
		if display_changed {
			self.display_revision = self.display_revision.wrapping_add(1);
		}
		self.trim_overlay();
	}
	pub(crate) fn refresh(&mut self, list: &model::MemberList) {
		let Some(guild) = list.guild else { return };
		let mut rows: Vec<_> = list
			.slots
			.iter()
			.flatten()
			.filter_map(|slot| match slot {
				MemberSlot::Person(member) => Some(member.clone()),
				_ => None,
			})
			.collect();
		let mut partial: Vec<_> = rows
			.extract_if(.., |member| {
				list.missing_identity.iter().any(|fields| {
					fields.user == member.user.id
						&& (fields.nick
							|| fields.roles || fields.global_name
							== model::GlobalNameUpdate::Unchanged)
				})
			})
			.collect();
		for member in &partial {
			let fields = list
				.missing_identity
				.iter()
				.find(|fields| fields.user == member.user.id)
				.expect("partial row has field metadata");
			if fields.nick
				&& fields.roles
				&& fields.global_name == model::GlobalNameUpdate::Unchanged
			{
				continue;
			}
			let nick = if fields.nick {
				Patch::Absent
			} else {
				member.nick.clone().map_or(Patch::Null, Patch::Value)
			};
			let has_name = fields.global_name != model::GlobalNameUpdate::Unchanged;
			let display_changed = self.get(guild, member.user.id).is_none_or(|old| {
				(!fields.nick && old.nick != nick)
					|| (has_name
						&& old.profile.as_ref().is_none_or(|profile| {
							profile.name != member.user.name
								|| profile.username != member.user.username
						}))
			});
			self.insert(
				MemberIdentity {
					global_name: fields.global_name,
					guild,
					user: member.user.id,
					profile: has_name.then(|| member.user.clone()),
					avatar: Patch::Absent,
					nick,
					roles: if fields.roles {
						Patch::Absent
					} else {
						Patch::Value(member.roles.clone())
					},
					departed: false,
				},
				display_changed,
			);
		}
		self.reconcile_with_missing(guild, &mut rows, self.revision, &list.missing_identity);
		self.reconcile_with_missing(guild, &mut partial, self.revision, &list.missing_identity);
		for member in &mut partial {
			if member.nick.is_none()
				&& list
					.missing_identity
					.iter()
					.any(|fields| fields.user == member.user.id && fields.nick)
				&& self.get(guild, member.user.id).is_none_or(|identity| {
					!identity.departed && matches!(identity.nick, Patch::Absent)
				}) && let Some(nick) = self
				.member(guild, member.user.id)
				.and_then(|old| old.nick.as_ref())
			{
				member.nick = Some(nick.clone());
			}
		}
		self.remember_with_source(guild, &rows, true);
		self.remember_with_source(guild, &partial, false);
		for (revision, entry) in &mut self.entries {
			if entry.guild == guild && rows.iter().any(|row| row.user.id == entry.user) {
				*revision = self.revision;
			}
		}
		self.trim_overlay();
	}
}

impl State {
	/// The visible conversation's bounded live identity watch, independent of op-8 lookup admission.
	pub fn watched_member_users(&self) -> Option<(Id, Id, Vec<Id>)> {
		let channel = self.selected?;
		let guild = self.channel(channel)?.guild?;
		if self.auth != crate::auth::AuthState::Authenticated
			|| !self.gateway_connected
			|| !self.can_view(channel)
		{
			return None;
		}
		// Check departure identities once, rather than searching the full overlay for
		// every watched author. Keep the latest-observation rule even for duplicate entries.
		let departed: Vec<_> = self
			.member_labels
			.entries
			.iter()
			.filter(|(_, entry)| entry.guild == guild && entry.departed)
			.filter(|(_, entry)| {
				self.member_labels
					.get(guild, entry.user)
					.is_some_and(|current| current.departed)
			})
			.map(|(_, entry)| entry.user)
			.collect();
		// Maintain sorted IDs while preserving the first 128 distinct observations.
		let mut users = Vec::with_capacity(128);
		for user in self
			.member_labels
			.visible_users
			.iter()
			.copied()
			.filter(|_| self.member_labels.visible_scope == Some((guild, channel)))
			.chain(self.timeline.iter().rev().map(|m| m.author.id))
			.chain(
				self.member_search
					.iter()
					.flat_map(|v| v.rows.iter().chain(&v.retained))
					.map(|m| m.user.id),
			)
			.chain(
				self.observed_member_candidates(Some(guild))
					.map(|user| user.id()),
			) {
			if user.0 != 0
				&& !departed.contains(&user)
				&& let Err(index) = users.binary_search(&user)
			{
				users.insert(index, user);
			}
			if users.len() == 128 {
				break;
			}
		}
		Some((guild, channel, users))
	}
	pub fn member_watch_revision(&self) -> u64 {
		self.member_labels.watch_revision
	}
	pub fn visible_member_cache_scope(&self) -> Option<(Id, u64)> {
		let channel = self.selected?;
		let guild = self.channel(channel)?.guild?;
		if self.auth != crate::auth::AuthState::Authenticated
			|| !self.gateway_connected
			|| !self.can_view(channel)
			|| self.member_labels.visible_scope != Some((guild, channel))
		{
			return None;
		}
		Some((channel, self.member_labels.watch_revision))
	}
	/// Visible authors missing a display observation, independent of the Gateway query slot.
	pub fn visible_member_cache_request(&self) -> Option<member_search::Request> {
		let (channel, nonce) = self.visible_member_cache_scope()?;
		let guild = self.channel(channel)?.guild?;
		let mut users: Vec<_> = self
			.member_labels
			.visible_users
			.iter()
			.copied()
			.filter(|user| !self.has_cached_member_display(guild, *user))
			.filter(|user| {
				!self
					.member_identity(channel, *user)
					.is_some_and(|entry| entry.departed)
			})
			.collect();
		users.sort_unstable();
		users.dedup();
		(!users.is_empty()).then_some(member_search::Request {
			guild,
			channel,
			query: String::new(),
			users,
			nonce,
			slot: 1,
		})
	}
	/// Revision of bounded display observations, for deduplicated asynchronous disk work.
	pub fn member_display_revision(&self) -> u64 {
		self.member_labels.revision()
	}
	/// Unrelated live observations are safe; losing their bounded guards fences older reads.
	pub fn member_display_read_valid(&self, started_revision: u64) -> bool {
		started_revision >= self.member_labels.disk_read_floor
			&& started_revision <= self.member_labels.revision()
	}
	/// Revision of the persisted name/username/nickname projection, independent of
	/// observation ordering used to reject stale lookups and disk reads.
	pub fn member_display_cache_revision(&self) -> u64 {
		self.member_labels.display_revision
	}
	/// Guilds with retained member observations, bounded by the existing label caps.
	pub fn cached_member_guilds(&self) -> Vec<Id> {
		self.member_labels
			.members
			.iter()
			.map(|(guild, _, _, _)| *guild)
			.chain(
				self.member_labels
					.entries
					.iter()
					.map(|(_, entry)| entry.guild),
			)
			.collect::<std::collections::BTreeSet<_>>()
			.into_iter()
			.collect()
	}
	pub fn cached_member_displays(&self, guild: Id) -> Vec<MemberDisplay> {
		let mut rows = std::collections::BTreeMap::new();
		for (_, member, _, _) in self
			.member_labels
			.members
			.iter()
			.filter(|(g, _, _, verified)| *g == guild && *verified)
		{
			let row = MemberDisplay {
				guild,
				user: member.user.id,
				name: member.user.name.clone(),
				username: member.user.username.clone(),
				nick: member.nick.clone(),
			};
			if row.valid() {
				rows.insert(row.user, row);
			}
		}
		for (_, entry) in self
			.member_labels
			.entries
			.iter()
			.filter(|(_, e)| e.guild == guild)
		{
			if entry.departed {
				rows.remove(&entry.user);
				continue;
			}
			if entry.global_name != model::GlobalNameUpdate::Unchanged
				&& let Some(profile) = &entry.profile
				&& !matches!(entry.nick, Patch::Absent)
			{
				let row = MemberDisplay {
					guild,
					user: entry.user,
					name: profile.name.clone(),
					username: profile.username.clone(),
					nick: match &entry.nick {
						Patch::Value(nick) if !nick.is_empty() => Some(nick.clone()),
						_ => None,
					},
				};
				if row.valid() {
					rows.insert(row.user, row);
				}
			}
		}
		rows.into_values().collect()
	}
	/// Whether a single current observation can be written as a complete display
	/// row. This does not confer membership, permissions or profile freshness.
	pub fn has_cached_member_display(&self, guild: Id, user: Id) -> bool {
		let valid = |profile: &User, nick: Option<&str>| {
			MemberDisplay::valid_fields(
				guild,
				user,
				&profile.name,
				profile.username.as_deref(),
				nick,
			)
		};
		if let Some(update) = self.member_labels.get(guild, user) {
			if update.departed {
				return false;
			}
			if update.global_name != model::GlobalNameUpdate::Unchanged
				&& let Some(profile) = &update.profile
			{
				match &update.nick {
					Patch::Value(nick)
						if valid(profile, (!nick.is_empty()).then_some(nick.as_str())) =>
					{
						return true;
					}
					Patch::Null if valid(profile, None) => return true,
					_ => {}
				}
			}
		}
		self.member_labels
			.verified_member(guild, user)
			.is_some_and(|member| valid(&member.user, member.nick.as_deref()))
	}
	/// Targeted local suggestions remain older than every live query or observation.
	/// They fill presentation hints without completing a Gateway request or inventing roles.
	pub fn hydrate_member_search_displays(
		&mut self,
		request: &member_search::Request,
		started_revision: u64,
		rows: Vec<MemberDisplay>,
	) -> bool {
		if !request.valid()
			|| self.selected != Some(request.channel)
			|| self
				.channel(request.channel)
				.and_then(|channel| channel.guild)
				!= Some(request.guild)
			|| !self.can_view(request.channel)
			|| !self.gateway_connected
			|| self.auth != crate::auth::AuthState::Authenticated
			|| !self.member_display_read_valid(started_revision)
			|| self.member_search[request.slot].request.as_ref() != Some(request)
			|| rows.len() > member_search::LIMIT
			|| rows.iter().map(MemberDisplay::bytes).sum::<usize>() > member_search::MAX_BYTES
		{
			return false;
		}
		let query = request.query.to_lowercase();
		let exact = request.query.parse::<Id>().ok();
		if rows.iter().any(|row| {
			row.guild != request.guild
				|| !row.valid()
				|| if request.slot == 1 {
					!request.users.contains(&row.user)
				} else if let Some(exact) = exact {
					row.user != exact
				} else {
					![
						Some(row.name.as_str()),
						row.username.as_deref(),
						row.nick.as_deref(),
					]
					.into_iter()
					.flatten()
					.any(|name| name.to_lowercase().contains(&query))
				}
		}) {
			return false;
		}
		self.hydrate_targeted_member_displays(request.guild, started_revision, rows);
		true
	}
	/// Apply a local visible-author lookup without completing or replacing a live lookup.
	pub fn hydrate_visible_member_displays(
		&mut self,
		request: &member_search::Request,
		started_revision: u64,
		rows: Vec<MemberDisplay>,
	) -> bool {
		if !request.valid()
			|| request.slot != 1
			|| self.selected != Some(request.channel)
			|| self.member_labels.visible_scope != Some((request.guild, request.channel))
			|| self.member_labels.watch_revision != request.nonce
			|| !self.member_display_read_valid(started_revision)
			|| self.auth != crate::auth::AuthState::Authenticated
			|| !self.gateway_connected
			|| !self.can_view(request.channel)
			|| self
				.channel(request.channel)
				.and_then(|channel| channel.guild)
				!= Some(request.guild)
			|| rows.len() > member_search::LIMIT
			|| rows.iter().map(MemberDisplay::bytes).sum::<usize>() > member_search::MAX_BYTES
			|| rows.iter().any(|row| {
				row.guild != request.guild || !row.valid() || !request.users.contains(&row.user)
			}) {
			return false;
		}
		self.hydrate_targeted_member_displays(request.guild, started_revision, rows);
		true
	}
	fn hydrate_targeted_member_displays(
		&mut self,
		guild: Id,
		started_revision: u64,
		rows: Vec<MemberDisplay>,
	) {
		let needed = rows
			.iter()
			.filter(|row| {
				self.member_labels.get(guild, row.user).is_none()
					&& self.member_labels.member(guild, row.user).is_none()
					&& !self
						.member_labels
						.retired_after(guild, row.user, started_revision)
			})
			.count();
		let mut removed = false;
		while self
			.member_labels
			.entries
			.iter()
			.filter(|(revision, _)| *revision == 0)
			.count()
			.saturating_add(needed)
			> HINT_LIMIT
		{
			let Some(index) = self
				.member_labels
				.entries
				.iter()
				.position(|(revision, entry)| {
					*revision == 0
						&& !rows
							.iter()
							.any(|row| row.guild == entry.guild && row.user == entry.user)
				})
			else {
				break;
			};
			let (_, old) = self
				.member_labels
				.entries
				.remove(index)
				.expect("hint index checked");
			self.member_labels.bytes -= old.bytes();
			removed = true;
		}
		if removed {
			self.revision = self.revision.wrapping_add(1);
			self.skip_identity_navigation();
		}
		self.merge_member_displays(guild, started_revision, rows);
	}
	/// Older disk observations never override a live result, removal or account transition.
	pub fn hydrate_member_displays(
		&mut self,
		guild: Id,
		started_revision: u64,
		rows: Vec<MemberDisplay>,
	) {
		if self.member_labels.revision() != started_revision
			|| !self.gateway_connected
			|| self.auth != crate::auth::AuthState::Authenticated
			|| !self.selected.is_some_and(|channel| {
				self.channel(channel)
					.is_some_and(|c| c.guild == Some(guild))
					&& self.can_view(channel)
			}) || rows.len() > 4096
			|| rows.iter().any(|row| row.guild != guild || !row.valid())
			|| rows.iter().map(MemberDisplay::bytes).sum::<usize>() > 2 * 1024 * 1024
		{
			return;
		}
		self.merge_member_displays(guild, started_revision, rows);
	}
	fn merge_member_displays(
		&mut self,
		guild: Id,
		started_revision: u64,
		rows: Vec<MemberDisplay>,
	) {
		let mut changed = false;
		let mut hints = self
			.member_labels
			.entries
			.iter()
			.filter(|(revision, _)| *revision == 0)
			.count();
		for row in rows {
			if self
				.member_labels
				.retired_after(guild, row.user, started_revision)
			{
				continue;
			}
			if self.member_labels.get(guild, row.user).is_some()
				|| self.member_labels.member(guild, row.user).is_some()
			{
				changed |=
					self.member_labels
						.fill_partial_nickname(guild, row.user, row.nick.as_deref());
				continue;
			}
			// Never evict a known live identity or departure to install an older disk name.
			if hints >= HINT_LIMIT {
				break;
			}
			let update = MemberIdentity {
				global_name: model::GlobalNameUpdate::Set,
				guild,
				user: row.user,
				profile: Some(User {
					member_avatar: None,
					id: row.user,
					name: row.name,
					username: row.username,
					avatar: None,
					discriminator: 0,
					primary_guild: None,
					cosmetics: None,
					webhook: false,
					kind: Default::default(),
				}),
				avatar: Patch::Absent,
				nick: row.nick.map_or(Patch::Null, Patch::Value),
				roles: Patch::Absent,
				departed: false,
			};
			let capacity = self.member_labels.entries.capacity();
			let reserve = if self.member_labels.entries.len() == capacity {
				capacity.max(2) * size_of::<(u64, MemberIdentity)>()
			} else {
				0
			};
			if self.member_labels.overlay_bytes() + reserve + update.bytes() > MAX_BYTES {
				break;
			}
			self.member_labels.insert_hint(update);
			hints += 1;
			changed = true;
		}
		if !changed {
			return;
		}

		self.revision = self.revision.wrapping_add(1);
		self.skip_identity_navigation();
		self.enforce_resident_budget();
	}
	pub fn member_identity(&self, channel: Id, user: Id) -> Option<&MemberIdentity> {
		if !self.can_view(channel) {
			return None;
		}
		self.member_labels.get(self.channel(channel)?.guild?, user)
	}
	pub(crate) fn known_global_name(&self, guild: Id, user: Id) -> Option<model::GlobalNameUpdate> {
		self.member_labels
			.known_global_name(guild, user)
			.or_else(|| {
				self.members
					.iter()
					.chain(self.parked_members.iter().map(|parked| &parked.list))
					.filter(|list| list.guild == Some(guild))
					.flat_map(|list| &list.missing_identity)
					.find_map(|fields| {
						(fields.user == user
							&& fields.global_name != model::GlobalNameUpdate::Unchanged)
							.then_some(fields.global_name)
					})
			})
			.or_else(|| {
				self.member_search.iter().find_map(|view| {
					view.request
						.as_ref()
						.filter(|request| request.guild == guild)?;
					view.missing_identity.iter().find_map(|fields| {
						(fields.user == user
							&& fields.global_name != model::GlobalNameUpdate::Unchanged)
							.then_some(fields.global_name)
					})
				})
			})
	}
	/// Outer None means unknown; Some(None) is an authoritative nickname removal.
	pub fn member_nickname(&self, channel: Id, user: Id) -> Option<Option<&str>> {
		let guild = self.channel(channel)?.guild?;
		if !self.can_view(channel) {
			return None;
		}
		if let Some(update) = self.member_identity(channel, user) {
			if update.departed {
				return Some(None);
			}
			match &update.nick {
				Patch::Value(nick) => return Some((!nick.is_empty()).then_some(nick.as_str())),
				Patch::Null => return Some(None),
				Patch::Absent => {}
			}
		}
		if let Some(list) = self
			.members
			.as_ref()
			.filter(|list| list.guild == Some(guild) && list.channel == channel)
			&& !list
				.missing_identity
				.iter()
				.any(|fields| fields.user == user && fields.nick)
			&& let Some(member) = list.slots.iter().flatten().find_map(|slot| match slot {
				MemberSlot::Person(member) if member.user.id == user => Some(member),
				_ => None,
			}) {
			return Some(member.nick.as_deref());
		}
		self.searched_member_nickname(channel, user)
			.or_else(|| {
				self.member_labels
					.verified_member(guild, user)
					.map(|member| member.nick.as_deref())
			})
			.or_else(|| {
				self.timeline
					.iter()
					.rev()
					.filter(|message| {
						message.channel == channel
							&& message.author.id == user
							&& !message.author.webhook
					})
					.find_map(|message| message.author_nick.as_deref().map(Some))
			})
			.or_else(|| {
				self.member_labels
					.member(guild, user)
					.and_then(|member| member.nick.as_deref())
					.filter(|nick| !nick.is_empty())
					.map(Some)
			})
	}
	/// An incomplete list row cannot assert that its absent role array is empty.
	pub(crate) fn presentation_member_roles(&self, channel: Id, user: Id) -> Option<&[Id]> {
		let guild = self.channel(channel)?.guild?;
		if !self.can_view(channel) {
			return None;
		}
		if let Some(list) = self
			.members
			.as_ref()
			.filter(|list| list.guild == Some(guild) && list.channel == channel)
			&& !list
				.missing_identity
				.iter()
				.any(|fields| fields.user == user && fields.roles)
			&& let Some(member) = list.slots.iter().flatten().find_map(|slot| match slot {
				MemberSlot::Person(member) if member.user.id == user => Some(member),
				_ => None,
			}) {
			return Some(member.roles.as_slice());
		}
		self.searched_member_roles(channel, user).or_else(|| {
			self.member_labels
				.verified_member(guild, user)
				.map(|member| member.roles.as_slice())
		})
	}
	pub fn observed_member_users(
		&self,
		channel: Id,
	) -> impl Iterator<Item = std::borrow::Cow<'_, User>> {
		let guild = self
			.channel(channel)
			.filter(|_| self.can_view(channel))
			.and_then(|c| c.guild);
		self.observed_member_candidates(guild)
			.map(ObservedUser::into_user)
	}
	/// Session display observations only; they do not prove membership or permissions.
	pub fn observed_display_members(&self, channel: Id) -> impl Iterator<Item = &Member> {
		let guild = self
			.channel(channel)
			.filter(|_| self.can_view(channel))
			.and_then(|c| c.guild);
		self.member_labels
			.members
			.iter()
			.rev()
			.filter_map(move |(scope, member, _, _)| (Some(*scope) == guild).then_some(member))
	}

	fn observed_member_candidates(
		&self,
		guild: Option<Id>,
	) -> impl Iterator<Item = ObservedUser<'_>> {
		self.member_labels
			.entries
			.iter()
			.rev()
			.filter_map(move |(_, update)| {
				if update.departed || Some(update.guild) != guild {
					return None;
				}
				if let Some(user) = &update.profile {
					return Some(ObservedUser::Profile(user));
				}
				let Patch::Value(nick) = &update.nick else {
					return None;
				};
				if nick.is_empty() {
					return None;
				}
				Some(ObservedUser::Nickname(update.user, nick))
			})
			.chain(
				self.member_labels
					.members
					.iter()
					.rev()
					.filter_map(move |(g, member, _, _)| {
						(Some(*g) == guild
							&& !self
								.member_labels
								.get(*g, member.user.id)
								.is_some_and(|e| e.departed))
						.then_some(ObservedUser::Profile(&member.user))
					}),
			)
	}
	/// Bounded live/presentation identities retained for one server, without changing selection.
	pub fn force_sync_known_member_ids(&self, guild: Id) -> Vec<Id> {
		let mut ids: std::collections::BTreeSet<_> = self
			.member_labels
			.members
			.iter()
			.filter(|(scope, _, _, _)| *scope == guild)
			.map(|(_, member, _, _)| member.user.id)
			.chain(
				self.member_labels
					.entries
					.iter()
					.filter(|(_, entry)| entry.guild == guild && !entry.departed)
					.map(|(_, entry)| entry.user),
			)
			.collect();
		let people = |slots: &[Option<MemberSlot>]| {
			slots
				.iter()
				.filter_map(|slot| match slot {
					Some(MemberSlot::Person(member)) => Some(member.user.id),
					_ => None,
				})
				.collect::<Vec<_>>()
		};
		if let Some(list) = &self.members
			&& list.guild == Some(guild)
		{
			ids.extend(people(&list.slots));
			for chunk in self.member_chunks.chunks.values() {
				ids.extend(people(chunk));
			}
		}
		if let Some(parked) = &self.parked_members
			&& parked.list.guild == Some(guild)
		{
			ids.extend(people(&parked.list.slots));
			for chunk in parked.chunks.chunks.values() {
				ids.extend(people(chunk));
			}
		}
		for view in &self.member_search {
			if view
				.request
				.as_ref()
				.is_some_and(|request| request.guild == guild)
			{
				ids.extend(
					view.rows
						.iter()
						.chain(&view.retained)
						.map(|member| member.user.id),
				);
			}
		}
		if self
			.selected
			.and_then(|channel| self.channel(channel))
			.and_then(|channel| channel.guild)
			== Some(guild)
		{
			ids.extend(
				self.timeline
					.iter()
					.filter(|message| !message.author.webhook)
					.map(|message| message.author.id),
			);
		}
		if let Some(profile) = &self.profile
			&& profile.guild == Some(guild)
		{
			ids.insert(profile.user);
		}
		ids.into_iter().collect()
	}
	/// Confirmed service absence retires only this server's identity and scoped consumers.
	pub(crate) fn retire_force_sync_member(&mut self, guild: Id, user: Id) -> bool {
		if self.auth != crate::auth::AuthState::Authenticated
			|| !self.gateway_connected
			|| self.guild(guild).is_none()
			|| user.0 == 0
			|| self.user.as_ref().is_none_or(|own| own.id == user)
		{
			return false;
		}
		self.apply_member_identity(MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			guild,
			user,
			profile: None,
			avatar: Patch::Null,
			nick: Patch::Null,
			roles: Patch::Value(vec![]),
			departed: true,
		});
		true
	}
	pub(crate) fn apply_member_identity(&mut self, update: MemberIdentity) {
		self.apply_member_identity_inner(update, false);
	}
	/// A correlated Force Sync read can update the requested guild without changing selection.
	pub(crate) fn apply_force_sync_identity(&mut self, update: MemberIdentity) {
		self.apply_member_identity_inner(update, true);
	}
	fn apply_member_identity_inner(&mut self, mut update: MemberIdentity, background: bool) {
		let channel = self.selected.filter(|channel| self.can_view(*channel));
		let selected_scope = channel
			.and_then(|channel| self.channel(channel))
			.and_then(|channel| channel.guild)
			== Some(update.guild);
		// A valid RESUME replays dispatches before its terminal RESUMED event. Account
		// generation and permission scope still gate these observations while reconnecting.
		if self.auth != crate::auth::AuthState::Authenticated
			|| !update.valid()
			|| self.guild(update.guild).is_none()
			|| (!selected_scope && !update.departed && !background)
		{
			return;
		}
		if update.profile.is_some()
			&& update.global_name == model::GlobalNameUpdate::Unchanged
			&& self.known_global_name(update.guild, update.user)
				== Some(model::GlobalNameUpdate::Cleared)
		{
			update.global_name = model::GlobalNameUpdate::Cleared;
		}
		if matches!(update.avatar, Patch::Absent)
			&& let Some(avatar) = update
				.profile
				.as_ref()
				.and_then(|user| user.member_avatar.as_ref())
			&& avatar.guild == update.guild
			&& avatar.valid()
		{
			update.avatar = avatar.hash.clone().map_or(Patch::Null, Patch::Value);
		}
		if let Some(channel) = channel.filter(|_| selected_scope)
			&& let Some(profile) = &mut update.profile
		{
			let known = self
				.display_member(channel, update.user)
				.map(|member| &member.user)
				.or_else(|| {
					self.member_labels
						.get(update.guild, update.user)
						.and_then(|old| old.profile.as_ref())
				})
				.or_else(|| {
					self.timeline
						.iter()
						.rev()
						.find(|message| {
							message.channel == channel
								&& message.author.id == update.user
								&& !message.author.webhook
						})
						.map(|message| &message.author)
				});
			if let Some(known) = known {
				let mut merged = known.clone();
				if update.global_name != model::GlobalNameUpdate::Unchanged {
					merged.name.clone_from(&profile.name);
				}
				merged.username.clone_from(&profile.username);
				if let Some(avatar) = &profile.member_avatar
					&& avatar.guild == update.guild
					&& avatar.valid()
				{
					merged.member_avatar = Some(avatar.clone());
				}
				*profile = merged;
			}
		}
		if !update.departed
			&& update.profile.is_none()
			&& matches!(update.nick, Patch::Absent)
			&& matches!(update.roles, Patch::Absent)
			&& matches!(update.avatar, Patch::Absent)
		{
			return;
		}
		// A later live observation can fill fields omitted by the current sidebar
		// snapshot, even when the value itself is unchanged.
		fn resolve_list_fields(list: &mut model::MemberList, update: &MemberIdentity) -> bool {
			if list.guild != Some(update.guild) {
				return false;
			}
			let mut resolved = false;
			list.missing_identity.retain_mut(|fields| {
				if fields.user != update.user {
					return true;
				}
				if update.departed {
					resolved = true;
					return false;
				}
				let previous = (fields.global_name, fields.nick, fields.roles);
				if update.profile.is_some()
					&& update.global_name != model::GlobalNameUpdate::Unchanged
				{
					fields.global_name = update.global_name;
				}
				fields.nick &= matches!(update.nick, Patch::Absent);
				fields.roles &= matches!(update.roles, Patch::Absent);
				resolved |= previous != (fields.global_name, fields.nick, fields.roles);
				fields.needs_metadata()
			});
			resolved
		}
		let mut resolved_fields = false;
		if let Some(list) = &mut self.members {
			resolved_fields |= resolve_list_fields(list, &update);
		}
		if let Some(parked) = &mut self.parked_members {
			resolved_fields |= resolve_list_fields(&mut parked.list, &update);
		}
		for view in &mut self.member_search {
			if view
				.request
				.as_ref()
				.is_some_and(|request| request.guild == update.guild)
			{
				view.missing_identity.retain_mut(|fields| {
					if fields.user != update.user {
						return true;
					}
					if update.departed {
						resolved_fields = true;
						return false;
					}
					let previous = (fields.global_name, fields.nick, fields.roles);
					if update.profile.is_some()
						&& update.global_name != model::GlobalNameUpdate::Unchanged
					{
						fields.global_name = update.global_name;
					}
					fields.nick &= matches!(update.nick, Patch::Absent);
					fields.roles &= matches!(update.roles, Patch::Absent);
					resolved_fields |= previous != (fields.global_name, fields.nick, fields.roles);
					fields.needs_metadata()
				});
			}
		}
		if !resolved_fields
			&& self
				.member_labels
				.get(update.guild, update.user)
				.is_some_and(|old| {
					old.departed == update.departed
						&& (update.profile.is_none() || update.profile == old.profile)
						&& (matches!(update.nick, Patch::Absent) || update.nick == old.nick)
						&& (matches!(update.roles, Patch::Absent) || update.roles == old.roles)
						&& (matches!(update.avatar, Patch::Absent) || update.avatar == old.avatar)
				}) {
			// Still advance observation ordering to fence in-flight search replies, without
			// relaying unchanged typing metadata through every cached row and layout.
			self.member_labels.insert(update, false);
			return;
		}
		let previous = self.member_labels.get(update.guild, update.user);
		let display_changed = update.departed
			|| previous.is_some_and(|old| old.departed)
			|| update.profile.as_ref().is_some_and(|profile| {
				previous
					.and_then(|old| old.profile.as_ref())
					.is_none_or(|old| old.name != profile.name || old.username != profile.username)
			}) || (!matches!(update.nick, Patch::Absent)
			&& previous.is_none_or(|old| old.nick != update.nick));
		fn patch_slots(slots: &mut [Option<MemberSlot>], update: &MemberIdentity) {
			for slot in slots {
				if let Some(MemberSlot::Person(member)) = slot
					&& member.user.id == update.user
				{
					if update.departed {
						*slot = None;
					} else {
						update.apply(member);
					}
				}
			}
		}
		if let Some(list) = &mut self.members
			&& list.guild == Some(update.guild)
		{
			patch_slots(&mut list.slots, &update);
			let mut bytes = list.slot_bytes();
			for slot in list.slots.iter_mut().rev() {
				if bytes <= 256 * 1024 {
					break;
				}
				if let Some(old) = slot.take() {
					bytes -= old.bytes();
				}
			}
			for slots in self.member_chunks.chunks.values_mut() {
				patch_slots(slots, &update);
			}
			self.member_chunks.evict(&list.ranges);
			// Identity growth must respect the cap even if every remaining chunk is visible.
			while self.member_chunks.bytes() > crate::MEMBER_CACHE_BYTES {
				let Some(key) = self
					.member_chunks
					.chunks
					.keys()
					.copied()
					.max_by_key(|start| start.abs_diff(self.member_chunks.viewport))
				else {
					break;
				};
				self.member_chunks.chunks.remove(&key);
			}
		}
		if let Some(parked) = &mut self.parked_members
			&& parked.list.guild == Some(update.guild)
		{
			patch_slots(&mut parked.list.slots, &update);
			for slots in parked.chunks.chunks.values_mut() {
				patch_slots(slots, &update);
			}
			let mut bytes = parked.list.slot_bytes();
			for slot in parked.list.slots.iter_mut().rev() {
				if bytes <= 256 * 1024 {
					break;
				}
				if let Some(old) = slot.take() {
					bytes -= old.bytes();
				}
			}
			parked.chunks.evict(&parked.list.ranges);
			while parked.chunks.bytes() > crate::MEMBER_CACHE_BYTES {
				let Some(key) = parked.chunks.chunks.keys().next_back().copied() else {
					break;
				};
				parked.chunks.chunks.remove(&key);
			}
		}
		for view in &mut self.member_search {
			if view
				.request
				.as_ref()
				.is_some_and(|r| r.guild == update.guild)
			{
				for rows in [&mut view.rows, &mut view.retained] {
					rows.retain_mut(|member| {
						if member.user.id != update.user {
							return true;
						}
						if update.departed {
							return false;
						}
						update.apply(member);
						true
					});
					let mut bytes = rows.iter().map(Member::bytes).sum::<usize>();
					while bytes > member_search::MAX_BYTES {
						bytes -= rows.remove(0).bytes();
					}
				}
				view.trim_missing_identity();
			}
		}
		if selected_scope {
			self.timeline.apply_author_identity(&update);
		}
		// Only guild-scoped profiles may inherit a member avatar; global profile views
		// keep the account artwork. Cached cards are updated alongside the open card.
		let refresh_profile = |profile: &mut model::UserProfile| {
			if update.departed {
				if profile
					.guild
					.as_ref()
					.is_some_and(|guild| guild.guild == update.guild)
				{
					profile.guild = None;
				}
				return;
			}
			let Some(guild) = &mut profile.guild else {
				return;
			};
			if guild.guild != update.guild || profile.user.id != update.user {
				return;
			}
			let avatar = match &update.avatar {
				Patch::Value(hash) => Some(Some(hash.clone())),
				Patch::Null => Some(None),
				Patch::Absent => update
					.profile
					.as_ref()
					.and_then(|user| user.member_avatar.as_ref())
					.filter(|avatar| avatar.guild == update.guild)
					.map(|avatar| avatar.hash.clone()),
			};
			if let Some(avatar) = avatar {
				guild.avatar = avatar;
			}
		};
		if let Some(profile) = &mut self.profile
			&& profile.user == update.user
			&& profile.guild == Some(update.guild)
			&& let Some(data) = &mut profile.data
		{
			refresh_profile(data);
		}
		self.profile_cache.update(update.user, refresh_profile);
		if update.departed && selected_scope {
			self.typing.remove(update.user);
		}
		self.member_labels.insert(update, display_changed);
		self.revision = self.revision.wrapping_add(1);
		self.skip_identity_navigation();
		self.enforce_resident_budget();
	}
}

#[cfg(test)]
mod cache_search_tests {
	use super::*;

	fn state() -> State {
		let mut state = State {
			auth: crate::auth::AuthState::Authenticated,
			gateway_connected: true,
			user: Some(crate::tests::message(1).author),
			selected: Some(Id(1)),
			guilds: vec![model::Guild {
				default_message_notifications: None,
				stickers: None,
				emojis: None,
				id: Id(2),
				name: "Synthetic server".into(),
				icon: None,
			}],
			channels: vec![model::Channel {
				icon: None,
				last_message: None,
				id: Id(1),
				guild: Some(Id(2)),
				parent_id: None,
				position: 0,
				name: "synthetic".into(),
				kind: 0,
				recipients: vec![],
				member_list_id: None,
				message_count: None,
				tags: None,
			}],
			..State::default()
		};
		crate::tests::grant_permissions(&mut state);
		state
	}

	fn row(user: u64, nick: &str) -> MemberDisplay {
		MemberDisplay {
			guild: Id(2),
			user: Id(user),
			name: "Synthetic person".into(),
			username: Some("synthetic_account".into()),
			nick: Some(nick.into()),
		}
	}

	#[test]
	fn observed_users_keep_nickname_and_profile_order_for_display_and_watch() {
		let mut state = state();
		let mut profile = crate::tests::message(1).author;
		profile.id = Id(12);
		profile.name = "Profile name".into();
		for (user, profile, nick, departed) in [
			(Id(11), None, Patch::Value("Nickname only".into()), false),
			(Id(12), Some(profile), Patch::Absent, false),
			(Id(13), None, Patch::Null, false),
			(Id(14), None, Patch::Value(String::new()), false),
			(Id(15), None, Patch::Null, true),
		] {
			state.apply(crate::Envelope {
				generation: state.generation,
				event: crate::Event::MemberIdentity(Box::new(MemberIdentity {
					global_name: model::GlobalNameUpdate::Set,
					guild: Id(2),
					user,
					profile,
					avatar: Patch::Absent,
					nick,
					roles: Patch::Absent,
					departed,
				})),
			});
		}
		let displayed: Vec<_> = state
			.observed_member_users(Id(1))
			.map(|user| (user.id, user.name.clone()))
			.collect();
		assert_eq!(
			displayed,
			vec![
				(Id(12), "Profile name".into()),
				(Id(11), "Nickname only".into()),
			]
		);
		assert_eq!(
			state.watched_member_users().unwrap().2,
			vec![Id(11), Id(12)]
		);
		assert_eq!(state.observed_member_users(Id(99)).count(), 0);
		state.gateway_connected = false;
		assert!(state.watched_member_users().is_none());
	}

	#[test]
	fn cached_display_requires_a_complete_current_identity() {
		let mut state = state();
		state.hydrate_member_displays(
			Id(2),
			state.member_display_revision(),
			vec![row(9000, "Old")],
		);
		assert!(state.has_cached_member_display(Id(2), Id(9000)));
		let mut user = crate::tests::message(1).author;
		user.id = Id(9000);
		let update = |profile, nick, departed| {
			crate::Event::MemberIdentity(Box::new(MemberIdentity {
				global_name: model::GlobalNameUpdate::Set,
				guild: Id(2),
				user: Id(9000),
				profile,
				avatar: Patch::Absent,
				nick,
				roles: Patch::Absent,
				departed,
			}))
		};
		state.apply(crate::Envelope {
			generation: state.generation,
			event: update(None, Patch::Null, false),
		});
		assert!(!state.has_cached_member_display(Id(2), Id(9000)));
		state.apply(crate::Envelope {
			generation: state.generation,
			event: update(Some(user), Patch::Value("New".into()), false),
		});
		assert!(state.has_cached_member_display(Id(2), Id(9000)));
		state.apply(crate::Envelope {
			generation: state.generation,
			event: update(None, Patch::Null, true),
		});
		assert!(!state.has_cached_member_display(Id(2), Id(9000)));
	}

	#[test]
	fn targeted_disk_search_supplies_nickname_without_completing_the_live_query() {
		let mut state = state();
		let Some(crate::Command::MemberSearch(request)) =
			state.search_members(Id(1), "FarAwayNick", 0)
		else {
			panic!("member query");
		};
		let revision = state.member_display_revision();
		assert!(state.hydrate_member_search_displays(
			&request,
			revision,
			vec![row(9000, "FarAwayNick")]
		));
		assert_eq!(
			state.member_nickname(Id(1), Id(9000)),
			Some(Some("FarAwayNick"))
		);
		assert!(!state.member_search[0].finished);
		assert!(state.member_search[0].rows.is_empty());
		assert!(matches!(
			state.member_identity(Id(1), Id(9000)).unwrap().roles,
			Patch::Absent
		));
		assert!(matches!(
			state.member_identity(Id(1), Id(9000)).unwrap().avatar,
			Patch::Absent
		));
	}

	#[test]
	fn cached_search_can_fill_after_an_empty_live_result() {
		let mut state = state();
		let Some(crate::Command::MemberSearch(request)) =
			state.search_members(Id(1), "CachedNick", 0)
		else {
			panic!("member query");
		};
		let revision = state.member_display_revision();
		state.searched_members(request.clone(), Ok(vec![]));
		assert!(state.hydrate_member_search_displays(
			&request,
			revision,
			vec![row(9000, "CachedNick")]
		));
		assert_eq!(
			state.member_nickname(Id(1), Id(9000)),
			Some(Some("CachedNick"))
		);
	}

	#[test]
	fn visible_cache_lookup_continues_while_an_older_gateway_lookup_is_pending() {
		let mut state = state();
		let Some(crate::Command::MemberSearch(live)) = state.request_author_members(&[Id(9000)])
		else {
			panic!("initial lookup");
		};
		assert!(state.request_author_members(&[Id(9001)]).is_none());
		let visible = state.visible_member_cache_request().unwrap();
		assert_eq!(visible.users, vec![Id(9001)]);
		let revision = state.member_display_revision();
		assert!(state.hydrate_visible_member_displays(
			&visible,
			revision,
			vec![row(9001, "Visible nick")]
		));
		assert_eq!(
			state.member_nickname(Id(1), Id(9001)),
			Some(Some("Visible nick"))
		);
		assert_eq!(state.member_search[1].request.as_ref(), Some(&live));
		assert!(!state.member_search[1].finished);
		state.request_author_members(&[Id(9002)]);
		assert!(!state.hydrate_visible_member_displays(
			&visible,
			state.member_display_revision(),
			vec![row(9001, "Stale")]
		));
		let before_resume = state.visible_member_cache_request().unwrap();
		state.member_labels.clear();
		state.request_author_members(&[Id(9002)]);
		let after_resume = state.visible_member_cache_request().unwrap();
		assert_ne!(before_resume.nonce, after_resume.nonce);
		assert!(!state.hydrate_visible_member_displays(
			&before_resume,
			state.member_display_revision(),
			vec![row(9002, "Before resume")]
		));
	}

	#[test]
	fn targeted_cache_search_survives_a_full_live_identity_overlay() {
		let mut state = state();
		for user in 1000..1000 + LIMIT as u64 {
			state.apply_member_identity(MemberIdentity {
				guild: Id(2),
				user: Id(user),
				profile: None,
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: Patch::Absent,
				nick: Patch::Value(format!("Live {user}")),
				roles: Patch::Absent,
				departed: false,
			});
		}
		let Some(crate::Command::MemberSearch(request)) =
			state.search_members(Id(1), "CachedNick", 0)
		else {
			panic!("member query");
		};
		assert!(state.hydrate_member_search_displays(
			&request,
			state.member_display_revision(),
			vec![row(9000, "CachedNick")]
		));
		assert_eq!(
			state.member_nickname(Id(1), Id(9000)),
			Some(Some("CachedNick"))
		);
		assert_eq!(
			state.member_nickname(Id(1), Id(1000)),
			Some(Some("Live 1000"))
		);
	}

	#[test]
	fn live_results_and_changed_queries_fence_delayed_disk_searches() {
		let mut state = state();
		let Some(crate::Command::MemberSearch(old)) = state.search_members(Id(1), "OldNick", 0)
		else {
			panic!("old query");
		};
		let revision = state.member_display_revision();
		let Some(crate::Command::MemberSearch(current)) = state.search_members(Id(1), "NewNick", 0)
		else {
			panic!("new query");
		};
		assert!(!state.hydrate_member_search_displays(&old, revision, vec![row(9000, "OldNick")]));
		let mut user = crate::tests::message(1).author;
		user.id = Id(9000);
		state.searched_members(
			current.clone(),
			Ok(vec![Member {
				roles: vec![],
				user,
				nick: Some("NewNick".into()),
				status: None,
				custom_status: None,
				activities: vec![],
				clients: Default::default(),
			}]),
		);
		assert!(state.hydrate_member_search_displays(
			&current,
			revision,
			vec![row(9000, "NewNickFromDisk")]
		));
		assert_eq!(
			state.member_nickname(Id(1), Id(9000)),
			Some(Some("NewNick"))
		);
		let revision = state.member_display_revision();
		assert!(state.hydrate_member_search_displays(
			&current,
			revision,
			vec![row(9000, "NewNickOld"), row(9001, "NewNickCached")]
		));
		assert_eq!(
			state.member_nickname(Id(1), Id(9000)),
			Some(Some("NewNick"))
		);
		assert_eq!(
			state.member_nickname(Id(1), Id(9001)),
			Some(Some("NewNickCached"))
		);
		state.apply_member_identity(MemberIdentity {
			guild: Id(2),
			user: Id(9001),
			profile: None,
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: Patch::Absent,
			nick: Patch::Null,
			roles: Patch::Absent,
			departed: true,
		});
		assert!(state.hydrate_member_search_displays(
			&current,
			state.member_display_revision(),
			vec![row(9001, "NewNickCached")]
		));
		assert!(state.member_identity(Id(1), Id(9001)).unwrap().departed);
		assert!(
			!state
				.observed_member_users(Id(1))
				.any(|user| user.id == Id(9001))
		);
	}

	#[test]
	fn delayed_targeted_cache_merges_untouched_rows_and_keeps_live_clears() {
		let mut state = state();
		let Some(crate::Command::MemberSearch(request)) = state.search_members(Id(1), "Cached", 0)
		else {
			panic!("member query");
		};
		let revision = state.member_display_revision();
		for (user, nick, departed) in [
			(9000, Patch::Value("Live nickname".into()), false),
			(9001, Patch::Null, false),
			(9002, Patch::Null, true),
		] {
			state.apply_member_identity(MemberIdentity {
				guild: Id(2),
				user: Id(user),
				profile: None,
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: Patch::Absent,
				nick,
				roles: Patch::Absent,
				departed,
			});
		}
		let mut profile = crate::tests::message(1).author;
		profile.id = Id(9004);
		profile.name = "current_account".into();
		profile.username = Some("current_account".into());
		state.apply_member_identity(MemberIdentity {
			guild: Id(2),
			user: profile.id,
			profile: Some(profile),
			global_name: model::GlobalNameUpdate::Cleared,
			avatar: Patch::Absent,
			nick: Patch::Absent,
			roles: Patch::Absent,
			departed: false,
		});
		// A busy stream of unrelated observations must not spend a cache retry budget.
		for user in 9100..9140 {
			state.apply_member_identity(MemberIdentity {
				guild: Id(2),
				user: Id(user),
				profile: None,
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: Patch::Absent,
				nick: Patch::Value("Unrelated observation".into()),
				roles: Patch::Absent,
				departed: false,
			});
		}
		assert_ne!(revision, state.member_display_revision());
		assert!(state.member_display_read_valid(revision));
		assert!(
			state.hydrate_member_search_displays(
				&request,
				revision,
				(9000..9005)
					.map(|user| row(user, "Cached nickname"))
					.collect()
			)
		);
		assert_eq!(
			state.member_nickname(Id(1), Id(9000)),
			Some(Some("Live nickname"))
		);
		assert_eq!(state.member_nickname(Id(1), Id(9001)), Some(None));
		assert!(state.member_identity(Id(1), Id(9002)).unwrap().departed);
		assert_eq!(
			state.member_nickname(Id(1), Id(9003)),
			Some(Some("Cached nickname"))
		);
		assert!(matches!(
			state.member_identity(Id(1), Id(9003)).unwrap().roles,
			Patch::Absent
		));
		let cleared = state.member_identity(Id(1), Id(9004)).unwrap();
		assert_eq!(cleared.global_name, model::GlobalNameUpdate::Cleared);
		assert_eq!(cleared.profile.as_ref().unwrap().name, "current_account");
		assert!(matches!(cleared.nick, Patch::Absent));
		assert!(!state.member_search[0].finished);
	}

	#[test]
	fn delayed_visible_cache_keeps_live_departures_and_fills_other_authors() {
		let mut state = state();
		state.request_author_members(&[Id(9000), Id(9001)]).unwrap();
		let request = state.visible_member_cache_request().unwrap();
		let revision = state.member_display_revision();
		state.apply_member_identity(MemberIdentity {
			guild: Id(2),
			user: Id(9000),
			profile: None,
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: Patch::Absent,
			nick: Patch::Null,
			roles: Patch::Absent,
			departed: true,
		});
		assert!(state.hydrate_visible_member_displays(
			&request,
			revision,
			vec![row(9000, "Departed"), row(9001, "Visible nickname")]
		));
		assert!(state.member_identity(Id(1), Id(9000)).unwrap().departed);
		assert_eq!(
			state.member_nickname(Id(1), Id(9001)),
			Some(Some("Visible nickname"))
		);
		assert!(!state.member_search[1].finished);
	}

	#[test]
	fn evicted_live_clears_and_departures_stay_excluded_until_history_is_fenced() {
		for departed in [false, true] {
			for churn in [LIMIT, LIMIT + RETIRED_LIMIT] {
				let mut state = state();
				let Some(crate::Command::MemberSearch(request)) =
					state.search_members(Id(1), "Cached", 0)
				else {
					panic!("member query");
				};
				let revision = state.member_display_revision();
				state.apply_member_identity(MemberIdentity {
					guild: Id(2),
					user: Id(9000),
					profile: None,
					global_name: model::GlobalNameUpdate::Unchanged,
					avatar: Patch::Absent,
					nick: Patch::Null,
					roles: Patch::Absent,
					departed,
				});
				for user in 10000..10000 + churn as u64 {
					state.apply_member_identity(MemberIdentity {
						guild: Id(2),
						user: Id(user),
						profile: None,
						global_name: model::GlobalNameUpdate::Unchanged,
						avatar: Patch::Absent,
						nick: Patch::Value("Live nickname".into()),
						roles: Patch::Absent,
						departed: false,
					});
				}
				assert!(state.member_identity(Id(1), Id(9000)).is_none());
				// An incomplete sidebar row must not let the disk restore an evicted clear.
				let mut profile = crate::tests::message(1).author;
				profile.id = Id(9000);
				state.member_labels.remember_with_source(
					Id(2),
					&[Member {
						user: profile,
						roles: vec![],
						nick: None,
						status: None,
						custom_status: None,
						activities: vec![],
						clients: Default::default(),
					}],
					false,
				);
				let history_retained = churn == LIMIT;
				assert_eq!(state.member_display_read_valid(revision), history_retained);
				assert_eq!(
					state.hydrate_member_search_displays(
						&request,
						revision,
						vec![row(9000, "Cached nickname"), row(9001, "Cached untouched")]
					),
					history_retained
				);
				assert_eq!(state.member_nickname(Id(1), Id(9000)), None);
				assert_eq!(
					state.member_nickname(Id(1), Id(9001)),
					history_retained.then_some(Some("Cached untouched"))
				);
				assert!(state.member_labels.bytes() < MAX_BYTES);
				assert!(state.member_labels.retired.len() <= RETIRED_LIMIT);
			}
		}
	}

	#[test]
	fn delayed_targeted_read_cannot_cross_a_clear_or_access_change() {
		let mut state = state();
		let Some(crate::Command::MemberSearch(request)) = state.search_members(Id(1), "Cached", 0)
		else {
			panic!("member query");
		};
		let revision = state.member_display_revision();
		state.channels[0].guild = Some(Id(3));
		assert!(!state.hydrate_member_search_displays(
			&request,
			revision,
			vec![row(9000, "Cached")]
		));
		state.channels[0].guild = Some(Id(2));
		state.selected = None;
		assert!(!state.hydrate_member_search_displays(
			&request,
			revision,
			vec![row(9000, "Cached")]
		));
		state.selected = Some(Id(1));
		state.gateway_connected = false;
		assert!(!state.hydrate_member_search_displays(
			&request,
			revision,
			vec![row(9000, "Cached")]
		));
		state.gateway_connected = true;
		state.member_labels.clear();
		assert!(!state.member_display_read_valid(revision));
		assert!(!state.hydrate_member_search_displays(
			&request,
			revision,
			vec![row(9000, "Cached")]
		));
		let revision = state.member_display_revision();
		state.permissions.guilds.clear();
		assert!(!state.hydrate_member_search_displays(
			&request,
			revision,
			vec![row(9000, "Cached")]
		));
		assert!(state.member_labels.entries.is_empty());
	}

	#[test]
	fn prepared_local_query_ignores_live_feedback_until_dispatched() {
		let mut state = state();
		let Some(crate::Command::MemberSearchLocal(request)) =
			state.prepare_member_search(Id(1), "Cached", 0)
		else {
			panic!("local query");
		};
		state.member_search_cooldown(request.clone(), 30);
		state.searched_members(request.clone(), Ok(vec![]));
		assert!(state.member_search[0].local_only);
		assert!(!state.member_search[0].finished);
		assert!(state.member_search[0].cooldown_until.is_none());
		let local_revision = state.member_search[0].identity_revision;
		state.apply_member_identity(MemberIdentity {
			guild: Id(2),
			user: Id(9100),
			profile: None,
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: Patch::Absent,
			nick: Patch::Value("Observed during debounce".into()),
			roles: Patch::Absent,
			departed: false,
		});
		let live_revision = state.member_display_revision();
		assert_ne!(local_revision, live_revision);
		assert!(state.hydrate_member_search_displays(
			&request,
			local_revision,
			vec![row(9000, "Cached nickname")]
		));
		assert!(
			matches!(state.dispatch_member_search(&request), Some(crate::Command::MemberSearch(live)) if live == request)
		);
		assert_eq!(state.member_search[0].identity_revision, live_revision);
		assert!(state.dispatch_member_search(&request).is_none());
		state.member_search_cooldown(request, 30);
		assert!(state.member_search[0].cooldown_until.is_some());
	}

	#[test]
	fn targeted_search_replaces_old_hints_within_existing_limits() {
		let mut state = state();
		let revision = state.member_display_revision();
		state.hydrate_member_displays(
			Id(2),
			revision,
			(1000..1000 + LIMIT as u64)
				.map(|user| row(user, "Old cached nickname"))
				.collect(),
		);
		assert_eq!(state.member_labels.entries.len(), LIMIT);
		let Some(crate::Command::MemberSearch(request)) =
			state.search_members(Id(1), "TargetNickname", 0)
		else {
			panic!("target query");
		};
		assert!(state.hydrate_member_search_displays(
			&request,
			state.member_display_revision(),
			vec![row(9000, "TargetNickname")]
		));
		assert_eq!(
			state.member_nickname(Id(1), Id(9000)),
			Some(Some("TargetNickname"))
		);
		assert!(state.member_labels.entries.len() <= LIMIT);
		assert!(state.member_labels.overlay_bytes() <= MAX_BYTES);
		assert!(!state.member_search[0].finished);
	}
}
