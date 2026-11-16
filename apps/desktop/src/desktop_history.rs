//! Coordinate bounded disk history work and explicit cleanup.
use super::{Desktop, Event, cache, wants_cached_history};

impl Desktop {
	pub(super) fn sync_server_cache(&mut self) {
		if self.state.demo
			|| self.fixture_only
			|| !self.state.gateway_connected
			|| self.state.auth != client_core::auth::AuthState::Authenticated
			|| !self.state.has_server_display_metadata()
		{
			return;
		}
		let Some(cache) = &self.cache else {
			return;
		};
		let epoch = cache.history.epoch();
		if !cache.history.allows(epoch) {
			return;
		}
		let revision = self.state.server_display_revision();
		let key = (self.state.generation, model::Id(0), 0, epoch);
		if self.state.needs_server_display_hints()
			&& self.server_cache_load.attempt(key)
			&& self.queue_cache(cache::Operation::LoadServers {
				revision,
				guilds: self.state.missing_server_display_ids(),
			}) {
			self.server_cache_load.accepted(key);
		}
		if self.state.has_dirty_server_displays()
			&& self.cache_pending == 0
			&& !self.server_cache_failed
		{
			let rows = self
				.state
				.dirty_server_displays()
				.into_iter()
				.map(|patch| local_store::ServerDisplay {
					guild: patch.id,
					name: match patch.name {
						model::Patch::Absent => None,
						model::Patch::Null => Some(String::new()),
						model::Patch::Value(name) => Some(name),
					},
					icon: match patch.icon {
						model::Patch::Absent => None,
						model::Patch::Null => Some(None),
						model::Patch::Value(icon) => Some(Some(icon)),
					},
				})
				.collect::<Vec<_>>();
			if !rows.is_empty() {
				self.queue_cache(cache::Operation::SaveServers { rows, revision });
			}
		}
	}
	/// Standard sticker packs are account scoped and available in DMs as well as servers.
	pub(super) fn sync_sticker_pack_cache(&mut self) {
		if self.state.demo
			|| self.fixture_only
			|| !self.state.gateway_connected
			|| self.state.auth != client_core::auth::AuthState::Authenticated
		{
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let Some(cache) = &self.cache else { return };
		let epoch = cache.history.epoch();
		if !cache.history.allows(epoch) {
			return;
		}
		let revision = self.state.sticker_pack_cache_revision();
		let key = (self.state.generation, account, revision, epoch);
		if !self.state.stickers.loaded
			&& self.sticker_pack_cache_load.attempt(key)
			&& self.queue_cache(cache::Operation::LoadStickerPackHints { revision })
		{
			self.sticker_pack_cache_load.accepted(key);
		}
		if self.cache_pending == 0 && self.sticker_pack_cache_save.attempt(key) {
			if let Some(packs) = self.state.cached_sticker_pack_hints() {
				if !packs.valid()
					|| self.queue_cache(cache::Operation::SaveStickerPackHints(Box::new(packs)))
				{
					self.sticker_pack_cache_save.accepted(key);
				}
			} else {
				self.sticker_pack_cache_save.accepted(key);
			}
		}
	}
	/// Seed once per account/cache epoch, then process coalesced guild updates.
	pub(super) fn sync_expression_cache(&mut self) {
		if self.state.demo
			|| self.fixture_only
			|| !self.state.gateway_connected
			|| self.state.auth != client_core::auth::AuthState::Authenticated
		{
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let Some(cache) = &self.cache else { return };
		let epoch = cache.history.epoch();
		if !cache.history.allows(epoch) {
			return;
		}
		let Some(channel) = self.state.selected.filter(|id| self.state.can_view(*id)) else {
			return;
		};
		self.expression_cache.scope(
			(self.state.generation, account, epoch),
			self.state.guilds.iter().map(|guild| guild.id),
		);
		self.expression_cache.observe_epoch(
			self.state.expression_cache_epoch(),
			self.state.guilds.iter().map(|guild| guild.id),
		);
		self.expression_cache
			.selected(self.state.channel(channel).and_then(|c| c.guild));
		while let Some((guild, operation)) = self.expression_cache.next() {
			let Some(current) = self.state.guild(guild) else {
				self.expression_cache.remove(guild);
				continue;
			};
			let revision = self.state.expression_guild_cache_revision(guild);
			let work = if operation == cache::ExpressionCacheSchedule::LOAD {
				if current.emojis.is_some() && current.stickers.is_some() {
					None
				} else {
					Some(cache::Operation::LoadExpressions { guild, revision })
				}
			} else {
				self.state
					.cached_expression_catalog(guild)
					.filter(model::ExpressionCatalog::valid)
					.map(|catalog| cache::Operation::SaveExpressions(Box::new(catalog)))
			};
			if let Some(work) = work {
				if !self.queue_cache(work) {
					self.expression_cache.blocked();
					return;
				}
				self.expression_cache.completed(guild, operation);
				// Each worker completion wakes the next job; avoid flooding the shared queue.
				return;
			}
			self.expression_cache.completed(guild, operation);
		}
	}
	/// Restore once per account and retry unsent work only after storage makes progress.
	pub(super) fn sync_gif_cache(&mut self) {
		if self.state.demo || self.fixture_only {
			self.state.gifs.favorites_changed = false;
			return;
		}
		if self.cache.is_none() || self.state.auth != client_core::auth::AuthState::Authenticated {
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let scope = (self.state.generation, account);
		let key = (scope.0, scope.1, 0, 0);
		if self.gif_cache.load.attempt(key) && self.queue_cache(cache::Operation::LoadGifFavorites)
		{
			self.gif_cache.load.accepted(key);
		}
		if self.state.gifs.favorites_changed && self.gif_cache.may_save(scope) {
			let favorites = self.state.gifs.favorites.clone();
			if self.queue_cache(cache::Operation::SaveGifFavorites(favorites)) {
				self.state.gifs.favorites_changed = false;
			} else {
				self.gif_cache.reject_save(scope);
			}
		}
	}
	/// Show bounded disk ranges without consuming the live REST request.
	pub(super) fn load_cached_history(&mut self) {
		let (state, request) = (&self.state, self.state.request);
		let Some(channel) = state
			.selected
			.filter(|channel| state.history_pending && state.can_read_history(*channel))
		else {
			return;
		};
		let Some(cache) = &self.cache else { return };
		let epoch = cache.history.epoch();
		if !cache.history.allows(epoch) {
			return;
		}
		let operation = if let Some(target) = state.pending_cached_history_target() {
			cache::Operation::LoadHistoryPage {
				channel,
				request,
				page: local_store::HistoryPageQuery {
					before: None,
					after: None,
					around: Some(target),
					limit: 50,
				},
			}
		} else if let Some(before) = state.history_before {
			cache::Operation::LoadHistoryPage {
				channel,
				request,
				page: local_store::HistoryPageQuery {
					before: Some(before),
					after: None,
					around: None,
					limit: 50,
				},
			}
		} else if let Some(after) = state.history_after {
			cache::Operation::LoadHistoryPage {
				channel,
				request,
				page: local_store::HistoryPageQuery {
					before: None,
					after: Some(after),
					around: None,
					limit: 50,
				},
			}
		} else if !state.history_targeted && wants_cached_history(state, channel, request) {
			cache::Operation::LoadChannel { channel, request }
		} else {
			return;
		};
		let key = (state.generation, channel, request, epoch);
		if self.cached_history_request.attempt(key) && self.queue_cache(operation) {
			self.cached_history_request.accepted(key);
		}
	}
	pub(super) fn accept_cached_history_page(
		&mut self,
		generation: u64,
		channel: model::Id,
		request: u64,
		epoch: u64,
		result: Result<Vec<model::Message>, local_store::StoreError>,
	) {
		if generation == self.state.generation
			&& self
				.cache
				.as_ref()
				.is_some_and(|cache| cache.history.allows(epoch))
			&& let Ok(messages) = result
		{
			self.state
				.hydrate_cached_history_page(channel, request, messages);
		}
	}
	pub(super) fn sync_member_cache(&mut self) {
		if self.state.demo || self.fixture_only || !self.state.gateway_connected {
			return;
		}
		let Some(channel) = self.state.selected.filter(|id| self.state.can_view(*id)) else {
			return;
		};
		let Some(guild) = self.state.channel(channel).and_then(|c| c.guild) else {
			return;
		};
		let Some(cache) = &self.cache else {
			return;
		};
		let epoch = cache.history.epoch();
		if !cache.history.allows(epoch) {
			return;
		}
		let revision = self.state.member_display_revision();
		let key = (self.state.generation, guild, 0, epoch);
		if self.member_cache_load.attempt(key)
			&& self.queue_cache(cache::Operation::LoadMembers { guild, revision })
		{
			self.member_cache_load.accepted(key);
		}
		// Coalesce identity snapshots behind existing storage work, keeping at most one
		// pending copy. Worker completion wakes the next pass without an idle timer.
		if let Some(changed) =
			cache::member_save_key(&self.state, guild, epoch, &self.member_cache_cleared)
			&& self.member_cache_saved != Some(changed)
			&& self.cache_pending == 0
		{
			let rows = self.state.cached_member_displays(guild);
			if rows.is_empty() || self.queue_cache(cache::Operation::SaveMembers(rows)) {
				self.member_cache_saved = Some(changed);
			}
		}
	}
	pub(super) fn clear_channel_cache(&mut self, channel: model::Id) {
		self.invalidate_cache_scope(cache::Operation::ClearChannel { channel });
	}
	pub(super) fn invalidate_cache_scope(&mut self, operation: cache::Operation) {
		if self.state.demo || self.fixture_only {
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		self.invalidate_cache_scope_for(self.state.generation, account, operation);
	}
	pub(super) fn invalidate_cache_scope_for(
		&mut self,
		generation: u64,
		account: model::Id,
		operation: cache::Operation,
	) {
		if self.state.demo || self.fixture_only {
			return;
		}
		let Some(cache) = &self.cache else {
			return;
		};
		if cache.invalidate_scope(generation, account, operation) {
			self.cache_pending += 1;
		} else {
			self.request_history_clear(account);
		}
	}
	pub(super) fn queue_cache(&mut self, operation: cache::Operation) -> bool {
		if let Some(user) = &self.state.user {
			if matches!(operation, cache::Operation::ClearHistory) {
				self.request_history_clear(user.id);
				return true;
			}
			self.queue_cache_for(user.id, operation)
		} else {
			false
		}
	}
	pub(super) fn queue_cache_for(
		&mut self,
		account: model::Id,
		operation: cache::Operation,
	) -> bool {
		if self.state.demo || self.fixture_only {
			return false;
		}
		if let Some(cache) = &self.cache {
			let retryable = cache::coalesced_retry(&operation);
			let read = matches!(
				&operation,
				cache::Operation::LoadChannel { .. }
					| cache::Operation::LoadMembers { .. }
					| cache::Operation::LoadServers { .. }
					| cache::Operation::LoadExpressions { .. }
					| cache::Operation::LoadStickerPackHints { .. }
					| cache::Operation::LoadGifFavorites
			);
			if matches!(
				operation,
				cache::Operation::LoadChannel { .. }
					| cache::Operation::SaveChannel { .. }
					| cache::Operation::SaveChanges { .. }
			) && !cache.history.allows(cache.history.epoch())
			{
				return false;
			}
			if cache.queue(self.state.generation, account, operation) {
				self.cache_pending += 1;
				if !self.cache_error {
					self.cache_status = "Saving local changes…";
				}
				return true;
			} else {
				if !read && !retryable {
					self.cache_error = true;
					self.cache_status = "Local storage queue full; some changes are not saved";
				}
			}
		}
		false
	}
	pub(super) fn request_history_clear(&mut self, account: model::Id) {
		if self
			.state
			.user
			.as_ref()
			.is_some_and(|user| user.id == account)
		{
			if let Some(connection) = &self.connection {
				connection.sync_roster.cancel();
			}
			self.force_sync = Default::default();
			self.messaging.force_sync_progress = None;
			self.messaging.force_sync_action = None;
		}
		if self.state.demo || self.fixture_only {
			return;
		}
		let Some(cache) = &self.cache else {
			return;
		};
		if self
			.state
			.user
			.as_ref()
			.is_some_and(|user| user.id == account)
		{
			self.member_cache_cleared.reset(&self.state);
		}
		cache.history.invalidate();
		cache.history.block();
		if !self.cache_clears.request(account) {
			cache.history.fail();
			self.cache_error = true;
			self.cache_status = "Cache cleanup backlog exceeded; history cache disabled until restart; deleted messages may remain on disk";
			return;
		}
		if !self.cache_error {
			self.cache_status =
				"Waiting to clear cached history; cached history temporarily disabled";
		}
		self.retry_history_clears();
	}
	pub(super) fn retry_history_clears(&mut self) {
		let Some(cache) = &self.cache else {
			return;
		};
		for _ in 0..16 {
			let Some(account) = self.cache_clears.next() else {
				break;
			};
			if !cache.queue(
				self.state.generation,
				account,
				cache::Operation::ClearHistory,
			) {
				break;
			}
			self.cache_clears.queued(account);
			self.cache_pending += 1;
		}
	}
	pub(super) fn delete_cached_messages(&mut self, event: &Event) {
		if self.state.demo || self.fixture_only {
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let (channel, ids) = match event {
			Event::Delete { channel, id } => (*channel, vec![*id]),
			Event::DeleteBulk { channel, ids } if !ids.is_empty() && ids.len() <= 100 => {
				(*channel, ids.clone())
			}
			Event::DeleteBulk { ids, .. } if ids.len() > 100 => {
				self.request_history_clear(account);
				return;
			}
			_ => return,
		};
		self.delete_cached_ids(channel, ids);
	}
	pub(super) fn delete_cached_ids(&mut self, channel: model::Id, ids: Vec<model::Id>) {
		if self.state.demo || self.fixture_only || ids.is_empty() {
			return;
		}
		let Some(account) = self.state.user.as_ref().map(|user| user.id) else {
			return;
		};
		let Some(cache) = &self.cache else {
			return;
		};
		if cache.delete_messages(self.state.generation, account, channel, ids) {
			self.cache_pending += 1;
		} else {
			self.request_history_clear(account);
		}
	}
}
