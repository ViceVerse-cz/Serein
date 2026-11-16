//! A small local hint lookup runs alongside, and never completes, a live member query.
use super::{Desktop, cache};
use client_core::{State, auth::AuthState, member_search::Request};
use model::Id;

const STALE_RETRIES: u8 = 2;

#[derive(Default)]
pub(super) struct Search {
	scope: Option<(u64, Id)>,
	pending: [Option<Request>; 2],
	loads: [cache::HistoryLoad; 2],
	retries: [u8; 2],
	visible_load: cache::HistoryLoad,
	visible_in_flight: Option<(u64, Id, u64, u64)>,
	visible_retry: Option<((u64, Id, u64, u64), u8)>,
	visible_empty: Option<((u64, Id, u64, u64), u64)>,
}

impl Search {
	pub(super) fn sync_scope(&mut self, scope: (u64, Id)) {
		if self.scope != Some(scope) {
			// A command can be the first request of a new account. Retain only pending
			// requests, which admission checks against the new account's current slots.
			let pending = std::mem::take(&mut self.pending);
			*self = Self {
				scope: Some(scope),
				pending,
				..Default::default()
			};
		}
	}

	fn retry_stale(
		&mut self,
		state: &State,
		generation: u64,
		request: &Request,
		revision: u64,
		epoch: u64,
	) -> bool {
		if state.member_display_read_valid(revision) {
			return false;
		}
		self.retry_current(state, generation, request, epoch)
	}

	pub(super) fn retry_epoch(
		&mut self,
		state: &State,
		generation: u64,
		request: &Request,
		read_epoch: u64,
		current_epoch: u64,
	) -> bool {
		read_epoch != current_epoch
			&& request.valid()
			&& !self.loads[request.slot].was_accepted((
				generation,
				request.channel,
				request.nonce,
				current_epoch,
			)) && self.retry_current(state, generation, request, current_epoch)
	}

	fn retry_current(
		&mut self,
		state: &State,
		generation: u64,
		request: &Request,
		epoch: u64,
	) -> bool {
		if !request.valid()
			|| generation != state.generation
			|| !state
				.user
				.as_ref()
				.is_some_and(|user| self.scope == Some((generation, user.id)))
			|| state.auth != AuthState::Authenticated
			|| !state.gateway_connected
			|| state.selected != Some(request.channel)
			|| !state.can_view(request.channel)
			|| state
				.channel(request.channel)
				.and_then(|channel| channel.guild)
				!= Some(request.guild)
			|| state.member_search[request.slot].request.as_ref() != Some(request)
			|| self.retries[request.slot] >= STALE_RETRIES
		{
			return false;
		}
		let key = (generation, request.channel, request.nonce, epoch);
		self.loads[request.slot].retry(key);
		self.retries[request.slot] += 1;
		self.pending[request.slot] = Some(request.clone());
		true
	}
}

impl Desktop {
	pub(super) fn queue_member_cache_search(&mut self, request: &Request) {
		if !self.state.demo && !self.fixture_only && request.valid() {
			self.member_cache_search.retries[request.slot] = 0;
			self.member_cache_search.pending[request.slot] = Some(request.clone());
			self.poll_member_cache_search();
		}
	}

	pub(super) fn poll_member_cache_search(&mut self) {
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			self.member_cache_search = Search::default();
			return;
		};
		if self.state.demo || self.fixture_only || self.state.auth != AuthState::Authenticated {
			return;
		}
		let scope = (self.state.generation, account);
		self.member_cache_search.sync_scope(scope);
		let Some(cache) = &self.cache else {
			return;
		};
		let epoch = cache.history.epoch();
		if !cache.history.allows(epoch) {
			return;
		}
		for slot in 0..2 {
			let Some(request) = self.member_cache_search.pending[slot].as_ref().cloned() else {
				continue;
			};
			if !request.valid()
				|| self.state.selected != Some(request.channel)
				|| !self.state.can_view(request.channel)
				|| self.state.member_search[slot].request.as_ref() != Some(&request)
			{
				self.member_cache_search.pending[slot] = None;
				continue;
			}
			let key = (scope.0, request.channel, request.nonce, epoch);
			if self.member_cache_search.loads[slot].attempt(key) {
				let revision = self.state.member_display_revision();
				if self.queue_cache(cache::Operation::FindMemberDisplays {
					request,
					revision,
					visible: false,
				}) {
					self.member_cache_search.loads[slot].accepted(key);
					self.member_cache_search.pending[slot] = None;
				}
			}
		}
		self.poll_visible_member_cache(scope.0, epoch);
	}

	fn poll_visible_member_cache(&mut self, generation: u64, epoch: u64) {
		if self.member_cache_search.visible_in_flight.is_some() {
			return;
		}
		let Some((channel, watch)) = self.state.visible_member_cache_scope() else {
			return;
		};
		let key = (generation, channel, watch, epoch);
		let revision = self.state.member_display_revision();
		if self.member_cache_search.visible_load.was_accepted(key)
			|| self.member_cache_search.visible_empty == Some((key, revision))
		{
			return;
		}
		let Some(request) = self.state.visible_member_cache_request() else {
			self.member_cache_search.visible_empty = Some((key, revision));
			return;
		};
		if self.member_cache_search.visible_load.attempt(key)
			&& self.queue_cache(cache::Operation::FindMemberDisplays {
				request,
				revision,
				visible: true,
			}) {
			self.member_cache_search.visible_load.accepted(key);
			self.member_cache_search.visible_in_flight = Some(key);
		}
	}

	pub(super) fn member_cache_search_capacity_changed(&mut self) {
		for load in &mut self.member_cache_search.loads {
			load.capacity_changed();
		}
		self.member_cache_search.visible_load.capacity_changed();
	}

	pub(super) fn accept_visible_member_cache(
		&mut self,
		generation: u64,
		request: Request,
		revision: u64,
		epoch: u64,
		result: Result<Vec<model::MemberDisplay>, local_store::StoreError>,
	) {
		let key = (generation, request.channel, request.nonce, epoch);
		if self.member_cache_search.visible_in_flight != Some(key) {
			return;
		}
		self.member_cache_search.visible_in_flight = None;
		if generation != self.state.generation
			|| !self
				.cache
				.as_ref()
				.is_some_and(|cache| cache.history.allows(epoch))
			|| self.state.selected != Some(request.channel)
			|| self.state.member_watch_revision() != request.nonce
		{
			return;
		}
		if let Ok(rows) = result {
			if !rows.is_empty() && !self.state.member_display_read_valid(revision) {
				let retries = self
					.member_cache_search
					.visible_retry
					.filter(|(old, _)| *old == key)
					.map_or(0, |(_, count)| count);
				if retries < STALE_RETRIES {
					self.member_cache_search.visible_retry = Some((key, retries + 1));
					self.member_cache_search.visible_load.retry(key);
					self.poll_visible_member_cache(generation, epoch);
				}
			} else {
				self.state
					.hydrate_visible_member_displays(&request, revision, rows);
			}
		}
	}

	pub(super) fn accept_member_cache_search(
		&mut self,
		generation: u64,
		request: Request,
		revision: u64,
		epoch: u64,
		result: Result<Vec<model::MemberDisplay>, local_store::StoreError>,
	) {
		if generation != self.state.generation {
			return;
		}
		let Some(cache) = &self.cache else {
			return;
		};
		if !cache.history.allows(epoch) {
			if self.member_cache_search.retry_epoch(
				&self.state,
				generation,
				&request,
				epoch,
				cache.history.epoch(),
			) {
				self.poll_member_cache_search();
			}
			return;
		}
		if let Ok(rows) = result {
			if !rows.is_empty()
				&& self.member_cache_search.retry_stale(
					&self.state,
					generation,
					&request,
					revision,
					epoch,
				) {
				self.poll_member_cache_search();
				return;
			}
			self.state
				.hydrate_member_search_displays(&request, revision, rows);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn epoch_retries_are_bounded_and_require_the_current_account_and_query() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let generation = state.generation;
		let account = state.user.as_ref().unwrap().id;
		let client_core::Command::MemberSearchLocal(old) =
			state.prepare_member_search(channel, "Cached", 0).unwrap()
		else {
			panic!("local query");
		};
		let mut search = Search::default();
		search.sync_scope((generation, account));
		assert!(!search.retry_epoch(&state, generation, &old, 0, 0));
		assert!(!search.retry_epoch(&state, generation.wrapping_add(1), &old, 0, 1));
		state.user.as_mut().unwrap().id = Id(account.0 + 1);
		assert!(!search.retry_epoch(&state, generation, &old, 0, 1));
		state.user.as_mut().unwrap().id = account;
		state.auth = AuthState::Unauthenticated;
		assert!(!search.retry_epoch(&state, generation, &old, 0, 1));
		state.auth = AuthState::Authenticated;
		state.gateway_connected = false;
		assert!(!search.retry_epoch(&state, generation, &old, 0, 1));
		state.gateway_connected = true;
		state.selected = None;
		assert!(!search.retry_epoch(&state, generation, &old, 0, 1));
		state.selected = Some(channel);
		let permissions = std::mem::take(&mut state.permissions);
		assert!(!search.retry_epoch(&state, generation, &old, 0, 1));
		state.permissions = permissions;
		let client_core::Command::MemberSearchLocal(current) =
			state.prepare_member_search(channel, "Current", 0).unwrap()
		else {
			panic!("current query");
		};
		assert!(!search.retry_epoch(&state, generation, &old, 0, 1));
		assert!(search.pending[0].is_none());
		let current_key = (generation, channel, current.nonce, 1);
		assert!(search.loads[0].attempt(current_key));
		search.loads[0].accepted(current_key);
		assert!(
			!search.retry_epoch(&state, generation, &current, 0, 1),
			"an older completion must not duplicate a read already admitted in the new epoch"
		);
		search.loads[0].retry(current_key);
		for attempt in 0..=STALE_RETRIES {
			let epoch = u64::from(attempt);
			let retry = search.retry_epoch(&state, generation, &current, epoch, epoch + 1);
			assert_eq!(retry, attempt < STALE_RETRIES);
			if retry {
				assert_eq!(search.pending[0].take().as_ref(), Some(&current));
			} else {
				assert!(search.pending[0].is_none());
			}
		}
		assert!(state.member_search[0].local_only);
	}

	#[test]
	fn unrelated_updates_keep_reads_and_lost_guard_history_has_bounded_retries() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let client_core::Command::MemberSearch(request) =
			state.search_members(channel, "Nickname", 0).unwrap()
		else {
			panic!("member query");
		};
		let generation = state.generation;
		let epoch = 7;
		let key = (generation, channel, request.nonce, epoch);
		let mut search = Search {
			scope: Some((generation, state.user.as_ref().unwrap().id)),
			..Default::default()
		};
		assert!(search.loads[0].attempt(key));
		search.loads[0].accepted(key);
		let revision = state.member_display_revision();
		state.apply(client_core::Envelope {
			generation,
			event: client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
				guild,
				user: Id(9000),
				profile: None,
				global_name: model::GlobalNameUpdate::Unchanged,
				avatar: model::Patch::Absent,
				nick: model::Patch::Value("Unrelated observation".into()),
				roles: model::Patch::Absent,
				departed: false,
			})),
		});
		assert_ne!(revision, state.member_display_revision());
		assert!(!search.retry_stale(&state, generation, &request, revision, epoch));
		for attempt in 0..=STALE_RETRIES {
			let revision = state.member_display_revision();
			assert!(!search.retry_stale(&state, generation, &request, revision, epoch));
			// Only exceeding both bounded live and retired guards requires a retry.
			let first = 10000 + u64::from(attempt) * 300;
			for user in first..first + 257 {
				state.apply(client_core::Envelope {
					generation,
					event: client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
						guild,
						user: Id(user),
						profile: None,
						global_name: model::GlobalNameUpdate::Unchanged,
						avatar: model::Patch::Absent,
						nick: model::Patch::Value(format!("Updated {attempt}")),
						roles: model::Patch::Absent,
						departed: false,
					})),
				});
			}
			let retry = search.retry_stale(&state, generation, &request, revision, epoch);
			assert_eq!(retry, attempt < STALE_RETRIES);
			if retry {
				assert_eq!(search.pending[0].as_ref(), Some(&request));
				assert!(search.loads[0].attempt(key));
				search.loads[0].accepted(key);
			}
		}
		let revision = state.member_display_revision();
		let _ = state.search_members(channel, "Another query", 0).unwrap();
		assert!(!search.retry_stale(&state, generation, &request, revision - 1, epoch));
		state.generation = generation.wrapping_add(1);
		assert!(!search.retry_stale(&state, generation, &request, revision - 1, epoch));
	}
}
