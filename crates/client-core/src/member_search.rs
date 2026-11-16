//! Bounded, session-only composer and visible-author member lookups.
use crate::{Command, State, auth::Failure};
use model::{Id, Member};

pub const LIMIT: usize = 100;
pub const MAX_BYTES: usize = 128 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
	pub guild: Id,
	pub channel: Id,
	pub query: String,
	pub users: Vec<Id>,
	pub nonce: u64,
	pub slot: usize,
}
impl Request {
	pub fn valid(&self) -> bool {
		self.guild.0 != 0
			&& self.channel.0 != 0
			&& match self.slot {
				0 => {
					self.users.is_empty()
						&& !self.query.is_empty()
						&& self.query.len() <= 256
						&& self.query.chars().count() <= 64
						&& !self.query.chars().any(char::is_control)
				}
				1 => {
					self.query.is_empty()
						&& !self.users.is_empty()
						&& self.users.len() <= LIMIT
						&& self.users.iter().all(|id| id.0 != 0)
						&& self.users.windows(2).all(|ids| ids[0] < ids[1])
				}
				_ => false,
			}
	}
}
#[derive(Default)]
pub struct View {
	pub request: Option<Request>,
	pub rows: Vec<Member>,
	/// Recently resolved labels for the same conversation, bounded like one search page.
	pub retained: Vec<Member>,
	/// Omitted fields and explicit global-name clears in the retained rows.
	pub(crate) missing_identity: Vec<model::MissingMemberFields>,
	pub finished: bool,
	/// A prepared composer query has only requested local display hints so far.
	pub local_only: bool,
	pub error: Option<&'static str>,
	pub cooldown_until: Option<std::time::Instant>,
	/// Avoid repaint-driven retries while still allowing an incomplete batch to recover.
	pub retry_after: Option<std::time::Instant>,
	pub identity_revision: u64,
}
impl View {
	pub(crate) fn trim_missing_identity(&mut self) {
		self.missing_identity.retain(|fields| {
			fields.needs_metadata()
				&& self
					.rows
					.iter()
					.chain(&self.retained)
					.any(|member| member.user.id == fields.user)
		});
		self.missing_identity.shrink_to(2 * LIMIT);
	}
}
impl State {
	/// Prepare a correlated local lookup immediately, before the live-query debounce.
	pub fn prepare_member_search(
		&mut self,
		channel: Id,
		query: &str,
		slot: usize,
	) -> Option<Command> {
		let Command::MemberSearch(request) = self.search_members(channel, query, slot)? else {
			return None;
		};
		self.member_search[slot].local_only = true;
		Some(Command::MemberSearchLocal(request))
	}

	pub fn dispatch_member_search(&mut self, request: &Request) -> Option<Command> {
		if !request.valid()
			|| self.selected != Some(request.channel)
			|| !self.gateway_connected
			|| !self.can_view(request.channel)
			|| self
				.channel(request.channel)
				.and_then(|channel| channel.guild)
				!= Some(request.guild)
			|| self.member_search[request.slot].request.as_ref() != Some(request)
			|| !self.member_search[request.slot].local_only
		{
			return None;
		}
		// Live ordering begins at dispatch; the earlier disk lookup keeps its own snapshot.
		self.member_search[request.slot].identity_revision = self.member_labels.revision();
		self.member_search[request.slot].local_only = false;
		Some(Command::MemberSearch(request.clone()))
	}

	pub(crate) fn refresh_member_labels(&mut self, list: &model::MemberList) {
		self.member_labels.refresh(list);
		if self
			.member_search
			.iter()
			.all(|view| view.rows.is_empty() && view.retained.is_empty())
		{
			return;
		}
		let updates: std::collections::BTreeMap<_, _> = list
			.slots
			.iter()
			.flatten()
			.filter_map(|slot| match slot {
				model::MemberSlot::Person(member) => Some((member.user.id, member)),
				_ => None,
			})
			.collect();
		for view in &mut self.member_search {
			if view.request.as_ref().is_none_or(|request| {
				request.channel != list.channel || Some(request.guild) != list.guild
			}) {
				continue;
			}
			for rows in [&mut view.rows, &mut view.retained] {
				for member in rows.iter_mut() {
					if let Some(updated) = updates.get(&member.user.id) {
						let fields = list
							.missing_identity
							.iter()
							.find(|fields| fields.user == member.user.id);
						let mut updated = (*updated).clone();
						if fields.is_some_and(|fields| {
							fields.global_name == model::GlobalNameUpdate::Unchanged
						}) {
							updated.user.name.clone_from(&member.user.name);
						}
						if fields.is_some_and(|fields| fields.nick) {
							updated.nick.clone_from(&member.nick);
						}
						if fields.is_some_and(|fields| fields.roles) {
							updated.roles.clone_from(&member.roles);
						}
						*member = updated;
					}
				}
				let mut bytes = rows.iter().map(Member::bytes).sum::<usize>();
				while bytes > MAX_BYTES {
					bytes -= rows.remove(0).bytes();
				}
				rows.shrink_to(LIMIT);
			}
			for fields in &mut view.missing_identity {
				if updates.contains_key(&fields.user) {
					let missing = list
						.missing_identity
						.iter()
						.find(|row| row.user == fields.user);
					if let Some(name) = missing.map(|row| row.global_name) {
						if name != model::GlobalNameUpdate::Unchanged {
							fields.global_name = name;
						}
					} else {
						fields.global_name = model::GlobalNameUpdate::Set;
					}
					fields.nick &= missing.is_some_and(|row| row.nick);
					fields.roles &= missing.is_some_and(|row| row.roles);
				}
			}
			view.trim_missing_identity();
		}
	}

	/// Search results are presentation hints; absent fields must not clear known values.
	fn known_searched_member(
		&self,
		channel: Id,
		user: Id,
		missing: impl Fn(&model::MissingMemberFields) -> bool,
	) -> Option<&Member> {
		let guild = self.channel(channel)?.guild?;
		if !self.can_view(channel) {
			return None;
		}
		for retained in [false, true] {
			for view in &self.member_search {
				if view
					.request
					.as_ref()
					.is_none_or(|request| request.channel != channel || request.guild != guild)
				{
					continue;
				}
				let rows = if retained { &view.retained } else { &view.rows };
				if let Some(member) = rows.iter().find(|member| member.user.id == user)
					&& !view
						.missing_identity
						.iter()
						.any(|fields| fields.user == user && missing(fields))
				{
					return Some(member);
				}
			}
		}
		None
	}
	pub(crate) fn searched_member_nickname(&self, channel: Id, user: Id) -> Option<Option<&str>> {
		self.known_searched_member(channel, user, |fields| fields.nick)
			.map(|member| member.nick.as_deref())
	}
	pub(crate) fn searched_member_roles(&self, channel: Id, user: Id) -> Option<&[Id]> {
		self.known_searched_member(channel, user, |fields| fields.roles)
			.map(|member| member.roles.as_slice())
	}

	/// Presentation metadata only; callers must use the permission state to authorize actions.
	pub fn display_member(&self, channel: Id, user: Id) -> Option<&Member> {
		self.current_display_member(channel, user).or_else(|| {
			let guild = self.channel(channel)?.guild?;
			self.can_view(channel)
				.then(|| self.member_labels.member(guild, user))
				.flatten()
		})
	}
	fn current_display_member(&self, channel: Id, user: Id) -> Option<&Member> {
		let guild = self.channel(channel)?.guild?;
		if !self.can_view(channel) {
			return None;
		}
		self.members
			.as_ref()
			.filter(|list| list.guild == Some(guild) && list.channel == channel)
			.and_then(|list| {
				list.slots.iter().flatten().find_map(|slot| match slot {
					model::MemberSlot::Person(member) if member.user.id == user => Some(member),
					_ => None,
				})
			})
			.or_else(|| {
				let list = self.members.as_ref().filter(|list| {
					list.guild == Some(guild)
						&& list.channel == channel
						&& list.request == self.member_chunks.request
				})?;
				if !list.lazy {
					return None;
				}
				self.member_chunks
					.chunks
					.values()
					.flatten()
					.flatten()
					.find_map(|slot| match slot {
						model::MemberSlot::Person(member) if member.user.id == user => Some(member),
						_ => None,
					})
			})
			.or_else(|| {
				self.member_search.iter().find_map(|view| {
					view.request
						.as_ref()
						.filter(|request| request.channel == channel && request.guild == guild)?;
					view.rows.iter().find(|member| member.user.id == user)
				})
			})
			.or_else(|| {
				self.member_search.iter().find_map(|view| {
					view.request
						.as_ref()
						.filter(|request| request.channel == channel && request.guild == guild)?;
					view.retained.iter().find(|member| member.user.id == user)
				})
			})
	}

	pub fn search_members(&mut self, channel: Id, query: &str, slot: usize) -> Option<Command> {
		let guild = self.channel(channel)?.guild?;
		if slot != 0
			|| !self.gateway_connected
			|| !self.can_view(channel)
			|| self.selected != Some(channel)
		{
			return None;
		}
		self.member_search_nonce = self.member_search_nonce.wrapping_add(1);
		let request = Request {
			guild,
			channel,
			query: query.to_owned(),
			users: vec![],
			nonce: self.member_search_nonce,
			slot,
		};
		if !request.valid() {
			return None;
		}
		let previous = &mut self.member_search[slot];
		let same_scope = previous
			.request
			.as_ref()
			.is_some_and(|old| old.channel == channel && old.guild == guild);
		let retained = if same_scope {
			let mut retained = std::mem::take(&mut previous.retained);
			for member in std::mem::take(&mut previous.rows) {
				retained.retain(|old| old.user.id != member.user.id);
				retained.push(member);
				while retained.len() > LIMIT
					|| retained.iter().map(Member::bytes).sum::<usize>() > MAX_BYTES
				{
					retained.remove(0);
				}
			}
			retained.shrink_to(LIMIT);
			retained
		} else {
			vec![]
		};
		let mut missing_identity = if same_scope {
			std::mem::take(&mut previous.missing_identity)
		} else {
			vec![]
		};
		missing_identity
			.retain(|fields| retained.iter().any(|member| member.user.id == fields.user));
		self.member_search[slot] = View {
			retained,
			missing_identity,
			identity_revision: self.member_labels.revision(),
			request: Some(request.clone()),
			..Default::default()
		};
		Some(Command::MemberSearch(request))
	}
	pub fn request_author_members(&mut self, users: &[Id]) -> Option<Command> {
		let channel = self.selected?;
		let guild = self.channel(channel)?.guild?;
		if self.demo || !self.gateway_connected || !self.can_view(channel) {
			return None;
		}
		self.member_labels.watch_visible(guild, channel, users);
		let mut users: Vec<_> = users
			.iter()
			.copied()
			.filter(|id| id.0 != 0)
			.filter(|id| !self.member_labels.member_fresh(guild, *id))
			.filter(|id| {
				!self
					.member_identity(channel, *id)
					.is_some_and(|update| update.departed)
			})
			.take(LIMIT)
			.collect();
		users.sort_unstable();
		users.dedup();
		let view = &self.member_search[1];
		if users.is_empty()
			|| view.request.as_ref().is_some_and(|request| {
				request.channel == channel
					&& request.guild == guild
					&& (!view.finished
						|| (users.iter().all(|user| request.users.contains(user))
							&& view
								.retry_after
								.is_some_and(|at| std::time::Instant::now() < at)))
			}) {
			return None;
		}
		self.member_search_nonce = self.member_search_nonce.wrapping_add(1);
		let request = Request {
			guild,
			channel,
			query: String::new(),
			users,
			nonce: self.member_search_nonce,
			slot: 1,
		};
		let (rows, retained, missing_identity) = if view
			.request
			.as_ref()
			.is_some_and(|old| old.channel == channel && old.guild == guild)
		{
			(
				std::mem::take(&mut self.member_search[1].rows),
				std::mem::take(&mut self.member_search[1].retained),
				std::mem::take(&mut self.member_search[1].missing_identity),
			)
		} else {
			(vec![], vec![], vec![])
		};
		self.member_search[1] = View {
			rows,
			retained,
			missing_identity,
			identity_revision: self.member_labels.revision(),
			request: Some(request.clone()),
			..Default::default()
		};
		Some(Command::MemberSearch(request))
	}
	pub(crate) fn member_search_cooldown(&mut self, request: Request, seconds: u32) {
		if !request.valid()
			|| self.selected != Some(request.channel)
			|| !self.gateway_connected
			|| !self.can_view(request.channel)
			|| self.member_search[request.slot].request.as_ref() != Some(&request)
			|| self.member_search[request.slot].local_only
			|| self.member_search[request.slot].finished
		{
			return;
		}
		let view = &mut self.member_search[request.slot];
		view.cooldown_until = Some(
			std::time::Instant::now()
				+ std::time::Duration::from_secs(u64::from(seconds.min(86400))),
		);
		view.error = None;
		self.revision = self.revision.wrapping_add(1);
		self.skip_identity_navigation();
	}
	pub(crate) fn searched_members(
		&mut self,
		request: Request,
		result: Result<Vec<Member>, Failure>,
	) {
		self.searched_members_with_missing(request, result, vec![]);
	}
	pub(crate) fn searched_members_with_missing(
		&mut self,
		request: Request,
		result: Result<Vec<Member>, Failure>,
		mut missing: Vec<model::MissingMemberFields>,
	) {
		if !request.valid()
			|| self.selected != Some(request.channel)
			|| !self.gateway_connected
			|| !self.can_view(request.channel)
			|| self.channel(request.channel).and_then(|c| c.guild) != Some(request.guild)
			|| self.member_search[request.slot].request.as_ref() != Some(&request)
			|| self.member_search[request.slot].local_only
		{
			return;
		}
		let mut seen = std::collections::BTreeSet::new();
		let invalid_missing = missing.len() > LIMIT
			|| missing.iter().any(|fields| {
				fields.user.0 == 0
					|| !fields.needs_metadata()
					|| !seen.insert(fields.user)
					|| result
						.as_ref()
						.ok()
						.is_none_or(|rows| !rows.iter().any(|member| member.user.id == fields.user))
			});
		let result = if invalid_missing {
			Err(Failure::Protocol)
		} else {
			result
		};
		let retry_seconds = match &result {
			Ok(rows) if rows.is_empty() => 300,
			Ok(_)
				if missing.iter().any(|fields| {
					fields.global_name == model::GlobalNameUpdate::Unchanged
						|| fields.nick || fields.roles
				}) =>
			{
				300
			}
			_ => 30,
		};
		self.member_search[request.slot].finished = true;
		self.member_search[request.slot].cooldown_until = None;
		self.member_search[request.slot].retry_after =
			Some(std::time::Instant::now() + std::time::Duration::from_secs(retry_seconds));
		match result {
			Ok(mut rows)
				if rows.len() <= LIMIT
					&& rows.iter().all(Member::valid)
					&& (request.users.is_empty()
						|| rows
							.iter()
							.all(|member| request.users.contains(&member.user.id)))
					&& rows.iter().map(Member::bytes).sum::<usize>() <= MAX_BYTES =>
			{
				self.member_labels.reconcile_with_missing(
					request.guild,
					&mut rows,
					self.member_search[request.slot].identity_revision,
					&missing,
				);
				if rows.iter().map(Member::bytes).sum::<usize>() > MAX_BYTES {
					self.member_search[request.slot].error =
						Some("Member search exceeded its safety limit");
					return;
				}
				let started = self.member_search[request.slot].identity_revision;
				for member in &rows {
					let Some(name) = self.member_labels.global_name_after(
						request.guild,
						member.user.id,
						started,
					) else {
						continue;
					};
					if name == model::GlobalNameUpdate::Unchanged {
						continue;
					}
					if let Some(fields) = missing
						.iter_mut()
						.find(|fields| fields.user == member.user.id)
					{
						fields.global_name = name;
					} else if name == model::GlobalNameUpdate::Cleared {
						missing.push(model::MissingMemberFields {
							user: member.user.id,
							global_name: name,
							nick: false,
							roles: false,
						});
					}
				}
				missing.retain(model::MissingMemberFields::needs_metadata);
				// The sidebar takes precedence when rendering. Refresh matching stale rows
				// too, while retaining their presence and honoring newer live updates
				// already merged by `reconcile` above.
				for member in &rows {
					let fields = missing.iter().find(|fields| fields.user == member.user.id);
					let stale = self
						.current_display_member(request.channel, member.user.id)
						.is_some_and(|old| {
							old.nick != member.nick
								|| old.roles != member.roles
								|| old.user.name != member.user.name
								|| old.user.username != member.user.username
						});
					if stale || fields.is_some() {
						let mut roles = member.roles.clone();
						roles.sort_unstable();
						roles.dedup();
						let global_name = self
							.member_labels
							.global_name_after(
								request.guild,
								member.user.id,
								self.member_search[request.slot].identity_revision,
							)
							.unwrap_or_else(|| {
								fields.map_or(model::GlobalNameUpdate::Set, |fields| {
									fields.global_name
								})
							});
						self.apply_member_identity(model::MemberIdentity {
							global_name,
							guild: request.guild,
							user: member.user.id,
							profile: Some(member.user.clone()),
							avatar: model::Patch::Absent,
							nick: if fields.is_some_and(|fields| fields.nick) {
								model::Patch::Absent
							} else {
								member
									.nick
									.clone()
									.map_or(model::Patch::Null, model::Patch::Value)
							},
							roles: if fields.is_some_and(|fields| fields.roles) {
								model::Patch::Absent
							} else {
								model::Patch::Value(roles)
							},
							departed: false,
						});
					}
				}
				if request.slot == 1 {
					for member in &rows {
						if !missing.iter().any(|fields| fields.user == member.user.id) {
							self.timeline.apply_author_membership(
								member.user.id,
								&member.roles,
								member.nick.as_deref(),
							);
						}
					}
				}
				if missing.is_empty() {
					self.member_labels.remember(request.guild, &rows);
				} else {
					let (partial, complete): (Vec<_>, Vec<_>) =
						rows.iter().cloned().partition(|member| {
							missing.iter().any(|fields| {
								fields.user == member.user.id
									&& (fields.nick
										|| fields.roles || fields.global_name
										== model::GlobalNameUpdate::Unchanged)
							})
						});
					self.member_labels.remember(request.guild, &complete);
					self.member_labels
						.remember_with_source(request.guild, &partial, false);
				}
				let view = &mut self.member_search[request.slot];
				for member in std::mem::replace(&mut view.rows, rows) {
					view.retained.retain(|old| old.user.id != member.user.id);
					view.retained.push(member);
				}
				view.retained
					.retain(|old| !view.rows.iter().any(|member| member.user.id == old.user.id));
				view.missing_identity
					.retain(|fields| !view.rows.iter().any(|member| member.user.id == fields.user));
				view.missing_identity.extend(
					missing.into_iter().filter(|fields| {
						view.rows.iter().any(|member| member.user.id == fields.user)
					}),
				);
				let mut bytes = view.retained.iter().map(Member::bytes).sum::<usize>();
				while view.retained.len() > LIMIT || bytes > MAX_BYTES {
					bytes -= view.retained.remove(0).bytes();
				}
				view.trim_missing_identity();
			}
			Ok(_) => {
				self.member_search[request.slot].error =
					Some("Member search exceeded its safety limit");
			}
			Err(error) => self.member_search[request.slot].error = Some(error.label()),
		}
	}
}
