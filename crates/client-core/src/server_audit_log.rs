//! Read-only, on-demand audit history in the existing administration lane.
use crate::State;
use model::{
	Id,
	permissions::VIEW_AUDIT_LOG,
	server_audit_log::{MAX_BYTES, MAX_ENTRIES, Page, Query},
};

impl State {
	pub fn can_open_audit_log_settings(&self, guild: Id) -> bool {
		self.guild_permission(guild, VIEW_AUDIT_LOG)
	}
	pub(crate) fn audit_log_action_allowed(&self, guild: Id, query: &Query) -> bool {
		if !self.can_open_audit_log_settings(guild) || !query.valid() {
			return false;
		}
		let Some(before) = query.before else {
			return true;
		};
		self.server_admin.guild == Some(guild)
			&& !self.server_admin.audit_limit_reached
			&& self
				.server_admin
				.audit_query
				.as_ref()
				.is_some_and(|old| old.user == query.user && old.action == query.action)
			&& self.server_admin.audit_log.as_ref().is_some_and(|page| {
				page.has_more && page.entries.last().is_some_and(|entry| entry.id == before)
			})
	}
	pub(crate) fn apply_audit_log(&mut self, mut page: Page) {
		if let Some(old) = self.server_admin.audit_log.as_mut() {
			if old.entries.len() + page.entries.len() > MAX_ENTRIES
				|| old.bytes().saturating_add(page.bytes()) > MAX_BYTES
			{
				old.has_more = false;
				self.server_admin.audit_limit_reached = true;
				return;
			}
			old.entries.reserve_exact(page.entries.len());
			old.entries.append(&mut page.entries);
			for user in page.users {
				if let Some(known) = old.users.iter_mut().find(|known| known.id == user.id) {
					*known = user;
				} else {
					old.users.reserve_exact(1);
					old.users.push(user);
				}
			}
			old.has_more = page.has_more;
			old.entries.shrink_to_fit();
			old.users.shrink_to_fit();
		} else {
			self.server_admin.audit_log = Some(page);
		}
		if let Some(page) = self.server_admin.audit_log.as_mut() {
			if !page.valid() {
				self.server_admin.audit_log = None;
				self.server_admin.error =
					Some("Audit log exceeded safe bounds; reload to continue");
			} else if page.has_more && page.entries.len() == MAX_ENTRIES {
				page.has_more = false;
				self.server_admin.audit_limit_reached = true;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Command, Envelope, Event, auth::AuthState, server_admin};
	use model::{
		permissions as p,
		server_admin::{Action, Result as Outcome},
		server_audit_log::Entry,
	};
	fn state() -> State {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				primary_guild: None,
				id: Id(1),
				name: "Synthetic".into(),
				avatar: None,
				discriminator: 0,
				kind: Default::default(),
				webhook: false,
			}),
			guilds: vec![model::Guild {
				stickers: None,
				id: Id(2),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			..Default::default()
		};
		state.permissions.guilds.insert(
			Id(2),
			p::Guild {
				id: Id(2),
				owner: Some(Id(99)),
				member: Some(p::Member {
					roles: vec![],
					timeout_until: None,
				}),
				roles: Some(vec![p::Role {
					id: Id(2),
					name: "@everyone".into(),
					bits: VIEW_AUDIT_LOG,
					color: 0,
					position: 0,
					hoist: false,
				}]),
			},
		);
		state
	}
	fn page(first: u64, count: usize, more: bool) -> Page {
		Page {
			guild: Id(2),
			users: vec![],
			has_more: more,
			entries: (0..count)
				.map(|n| Entry {
					id: Id(first - n as u64),
					user_id: Some(Id(1)),
					target_id: None,
					action_type: 30,
					reason: None,
					changes: vec![],
					options: vec![],
				})
				.collect(),
		}
	}
	fn request(state: &mut State, query: Query) -> u64 {
		let Command::ServerAdmin { request, .. } = state
			.request_server_admin(Id(2), Action::AuditLog(query))
			.expect("authorized request")
		else {
			unreachable!()
		};
		request
	}
	fn deliver(state: &mut State, request: u64, page: Page) {
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(server_admin::Event {
				guild: Id(2),
				request,
				result: Ok(Outcome::AuditLog(page)),
			}),
		});
	}
	#[test]
	fn audit_filters_pagination_stale_results_and_permission_loss() {
		let mut state = state();
		assert!(state.can_open_audit_log_settings(Id(2)));
		assert!(!state.can_manage_guild(Id(2)));
		let first = request(&mut state, Query::default());
		deliver(&mut state, first, page(1000, 50, true));
		assert_eq!(
			state.server_admin.audit_log.as_ref().unwrap().entries.len(),
			50
		);
		let wrong = Query {
			before: Some(Id(999)),
			..Default::default()
		};
		assert!(
			state
				.request_server_admin(Id(2), Action::AuditLog(wrong))
				.is_none()
		);
		let next = request(
			&mut state,
			Query {
				before: Some(Id(951)),
				..Default::default()
			},
		);
		deliver(&mut state, next, page(950, 50, true));
		assert_eq!(
			state.server_admin.audit_log.as_ref().unwrap().entries.len(),
			100
		);
		let forbidden = request(
			&mut state,
			Query {
				before: Some(Id(901)),
				..Default::default()
			},
		);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ServerAdmin(server_admin::Event {
				guild: Id(2),
				request: forbidden,
				result: Err(crate::auth::Failure::Forbidden),
			}),
		});
		assert!(
			state.server_admin.audit_log.is_none(),
			"service denial releases cached audit data"
		);
		assert!(state.server_admin.error.is_some());
		let filtered = request(
			&mut state,
			Query {
				user: Some(Id(9)),
				..Default::default()
			},
		);
		assert!(state.server_admin.audit_log.is_none());
		deliver(&mut state, next, page(900, 50, true));
		assert!(state.server_admin.pending);
		deliver(&mut state, filtered, page(1000, 1, false));
		assert!(
			state.server_admin.audit_log.is_none(),
			"mismatched actor rejected"
		);
		let pending = request(&mut state, Query::default());
		let mut role = state.permissions.guilds[&Id(2)].roles.as_ref().unwrap()[0].clone();
		role.bits = p::MANAGE_GUILD;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Role { guild: Id(2), role }),
		});
		assert!(
			!state.can_open_audit_log_settings(Id(2)),
			"Manage Server does not grant View Audit Log"
		);
		deliver(&mut state, pending, page(1000, 1, false));
		assert!(state.server_admin.audit_log.is_none());
		assert!(
			state
				.request_server_admin(Id(2), Action::AuditLog(Query::default()))
				.is_none()
		);
	}
	#[test]
	fn audit_history_stops_at_retained_limit_and_close_invalidates_response() {
		let mut state = state();
		for n in 0..10 {
			let query = Query {
				before: (n > 0).then_some(Id(1001 - n * 50)),
				..Default::default()
			};
			let seq = request(&mut state, query);
			deliver(&mut state, seq, page(1000 - n * 50, 50, true));
		}
		assert_eq!(
			state.server_admin.audit_log.as_ref().unwrap().entries.len(),
			500
		);
		assert!(state.server_admin.audit_limit_reached);
		assert!(!state.server_admin.audit_log.as_ref().unwrap().has_more);
		// A small number of large records must also stop pagination by bytes.
		for n in 0..3 {
			let query = Query {
				before: (n > 0).then_some(Id(1001 - n * 50)),
				..Default::default()
			};
			let seq = request(&mut state, query);
			let mut large = page(1000 - n * 50, 50, true);
			for entry in large.entries.iter_mut().take(10) {
				entry.changes = (0..60)
					.map(|_| model::server_audit_log::Change {
						key: "name".into(),
						old: model::Patch::Absent,
						new: model::Patch::Value("x".repeat(1500)),
					})
					.collect();
			}
			assert!(large.valid_response());
			deliver(&mut state, seq, large);
		}
		assert!(state.server_admin.audit_limit_reached);
		let retained = state.server_admin.audit_log.as_ref().unwrap();
		assert_eq!(retained.entries.len(), 100);
		assert!(retained.bytes() <= MAX_BYTES);
		let pending = request(&mut state, Query::default());
		state.close_server_admin();
		deliver(&mut state, pending, page(1000, 50, true));
		assert!(state.server_admin.audit_log.is_none());
	}
}
