//! Drain generation-scoped worker results into application state.
use super::{
	AuthState, Delivery, Desktop, Duration, Envelope, Event, Failure, access_candidates, avatars,
	cache, changes_active_history, confirmed_recovery_channel, connection, credentials, egui,
	extension_data_events, extension_events, hydrate_cache_result, recovery_draft,
	user_action_notice, voice,
};

impl Desktop {
	pub(super) fn clear_avatars(&mut self, ctx: &egui::Context) {
		self.avatar_start_failed = false;
		if self.avatar_cleanup.is_some() && !self.fixture_only && !self.state.demo {
			self.avatar_clear_account = self.state.user.as_ref().map(|user| user.id);
		}
		// Logout may follow expiry, when the downloading worker was already stopped.
		if self.avatars.is_none()
			&& self.avatar_cleanup.is_none()
			&& !self.fixture_only
			&& !self.state.demo
			&& let Some(user) = &self.state.user
		{
			match avatars::AvatarWorker::start(
				&self.runtime,
				user.id,
				ctx.clone(),
				self.messaging.image_cache,
			) {
				Ok(worker) => self.avatars = Some(worker),
				Err(error) => {
					self.cache_error = true;
					self.cache_status = error;
				}
			}
		}
		if let Some(worker) = self.avatars.take() {
			self.avatar_cleanup = Some(worker.shutdown_and_clear());
		}
		self.messaging.clear_avatars();
	}
	pub(super) fn poll_avatars(&mut self, ctx: &egui::Context) {
		if let Some(cleanup) = &self.avatar_cleanup {
			match cleanup.try_recv() {
				Ok(result) => {
					self.avatar_cleanup = None;
					if let Err(error) = result {
						self.cache_error = true;
						self.cache_status = error;
					}
				}
				Err(std::sync::mpsc::TryRecvError::Disconnected) => {
					self.avatar_cleanup = None;
					self.cache_error = true;
					self.cache_status =
						"Avatar cache cleanup failed; cached pictures may remain on disk";
				}
				Err(std::sync::mpsc::TryRecvError::Empty) => {}
			}
		}
		if self.avatar_cleanup.is_none()
			&& let Some(account) = self.avatar_clear_account.take()
		{
			match avatars::AvatarWorker::start(
				&self.runtime,
				account,
				ctx.clone(),
				self.messaging.image_cache,
			) {
				Ok(worker) => self.avatar_cleanup = Some(worker.shutdown_and_clear()),
				Err(error) => {
					self.cache_error = true;
					self.cache_status = error;
				}
			}
		}
		if !self.fixture_only && !self.state.demo && self.state.auth != AuthState::Authenticated {
			if let Some(worker) = self.avatars.take() {
				self.avatar_cleanup = Some(worker.shutdown());
			}
			return;
		}
		if !self.avatar_start_failed
			&& self.avatars.is_none()
			&& self.avatar_cleanup.is_none()
			&& let Some(user) = &self.state.user
		{
			match if self.fixture_only || self.state.demo {
				avatars::AvatarWorker::start_bundled(&self.runtime, ctx.clone())
			} else {
				avatars::AvatarWorker::start(
					&self.runtime,
					user.id,
					ctx.clone(),
					self.messaging.image_cache,
				)
			} {
				Ok(worker) => {
					self.messaging.clear_avatars();
					self.avatars = Some(worker);
				}
				Err(error) => {
					self.avatar_start_failed = true;
					self.cache_error = true;
					self.cache_status = error;
				}
			}
		}
		if let Some(worker) = &mut self.avatars {
			worker.set_limits(self.messaging.image_cache);
			if let Some(partition) = self.messaging.clear_image_cache.take()
				&& !worker.clear_partition(partition)
			{
				// The worker's queue is full; keep the request for the next frame.
				self.messaging.clear_image_cache = Some(partition);
				ctx.request_repaint();
			}
			self.messaging.image_cache_usage = worker.usage();
			for _ in 0..32 {
				let Some(result) = worker.poll() else {
					break;
				};
				if let Some(error) = result.error {
					self.cache_error = true;
					self.cache_status = error;
				}
				let stage = result.stage;
				self.messaging
					.accept_avatar(ctx, result.key.clone(), result.image);
				if stage == avatars::DecodeStage::Settled {
					self.messaging
						.accept_gif_animation(result.key, result.frames);
				}
			}
		}
	}
	pub(super) fn poll_voice_messages(&mut self, ctx: &egui::Context) {
		let demo = self.state.demo && std::env::args().any(|arg| arg == "--demo-recorder");
		let config = if demo {
			Some(::extensions::VoiceMessagesConfig::default())
		} else {
			self.extensions.voice_messages()
		};
		let blocked = self.tray_window.hidden
			|| ctx.input(|input| {
				input.viewport().minimized == Some(true) || input.viewport().close_requested()
			}) || self.state.voice.active.is_some()
			|| self.voice.microphone_busy()
			|| self.uploads.busy()
			|| self.messaging.has_edit_in(self.state.selected)
			|| self.messaging.attachment.is_some();
		self.voice_messages.poll(
			&mut self.state,
			&mut self.messaging,
			ctx,
			self.runtime.handle(),
			config,
			blocked,
			self.fixture_only,
		);
	}
	pub(super) fn poll_voice(&mut self, ctx: &egui::Context) {
		self.messaging.screen.preview_window_visible =
			!self.tray_window.hidden && self.window.is_visible() != Some(false);
		if let Some(command) =
			self.voice
				.poll(&self.runtime, &mut self.state, &mut self.messaging, ctx)
		{
			self.command(command);
		}
		// A heartbeat probe replaces a dead gateway socket in seconds, before the rejoin uses it.
		if std::mem::take(&mut self.voice.gateway_check)
			&& let Some(connection) = &self.connection
		{
			connection.reconnect();
		}
	}
	pub(super) fn poll(&mut self, ctx: &egui::Context) {
		let mut cached = Vec::new();
		let mut presence_cache_stopped = false;
		if let Some(cache) = &self.cache {
			for _ in 0..16 {
				match cache.receive.try_recv() {
					Ok(value) => cached.push(value),
					Err(std::sync::mpsc::TryRecvError::Disconnected) => {
						if self.messaging.custom_font.busy && self.font_picker.is_none() {
							self.messaging.custom_font.busy = false;
							self.messaging.custom_font.status =
								"Local storage worker stopped. Restart NuSerein to save fonts.";
						}
						if self.messaging.channel_preferences_reload
							|| self.messaging.channel_preferences_load_pending
						{
							self.messaging.channel_preferences_reload = false;
							self.messaging.channel_preferences_load_pending = false;
							self.messaging.channel_preferences_status = "Local storage worker stopped; restart NuSerein to restore channel preferences.";
						}
						presence_cache_stopped = self.presence_load_pending;
						break;
					}
					Err(std::sync::mpsc::TryRecvError::Empty) => break,
				}
			}
		}
		if presence_cache_stopped {
			self.presence_load_pending = false;
			self.flush_deferred_connect(ctx);
		}
		for (generation, outcome, _reservation) in cached {
			self.cache_pending = self.cache_pending.saturating_sub(1);
			self.member_cache_search_capacity_changed();
			if self.force_sync_cache_result(generation, &outcome) {
				continue;
			}
			self.cached_history_request.capacity_changed();
			self.member_cache_load.capacity_changed();
			self.expression_cache.capacity_changed();
			self.sticker_pack_cache_load.capacity_changed();
			self.sticker_pack_cache_save.capacity_changed();
			self.gif_cache.capacity_changed();
			self.server_cache_load.capacity_changed();
			// Settings are global; account removal/write failures still matter after logout.
			match &outcome {
				cache::Outcome::ServersSaved {
					revision,
					epoch,
					ids,
					result,
				} => {
					if generation == self.state.generation
						&& self
							.cache
							.as_ref()
							.is_some_and(|cache| cache.history.allows(*epoch))
					{
						match result {
							Some(Ok(())) => self.state.acknowledge_server_displays(*revision, ids),
							Some(Err(_)) => {
								self.server_cache_failed = true;
								self.cache_error = true;
								self.cache_status =
									"Could not save server names; live names remain available";
							}
							_ => {}
						}
					}
					continue;
				}
				cache::Outcome::Servers {
					revision,
					epoch,
					result,
				} => {
					if generation == self.state.generation
						&& (*revision != self.state.server_display_revision()
							|| !self.state.gateway_connected
							|| self.state.auth != AuthState::Authenticated)
					{
						self.server_cache_load
							.retry((generation, model::Id(0), 0, *epoch));
					}
					if generation == self.state.generation
						&& self.state.gateway_connected
						&& self.state.auth == AuthState::Authenticated
						&& self
							.cache
							.as_ref()
							.is_some_and(|cache| cache.history.allows(*epoch))
						&& let Ok(rows) = result
					{
						let patches = rows
							.iter()
							.map(|row| model::GuildPatch {
								id: row.guild,
								name: row
									.name
									.clone()
									.map_or(model::Patch::Absent, model::Patch::Value),
								icon: match row.icon.clone() {
									None => model::Patch::Absent,
									Some(None) => model::Patch::Null,
									Some(Some(icon)) => model::Patch::Value(icon),
								},
								default_message_notifications: model::Patch::Absent,
							})
							.collect();
						self.state.hydrate_server_displays(*revision, patches);
					}
					continue;
				}
				cache::Outcome::ScopedCleanupFailed { account } => {
					// One account-wide cleanup follows a failed scoped deletion. Its own
					// failure has the existing terminal cache status, never another retry.
					self.request_history_clear(*account);
					continue;
				}
				cache::Outcome::Members {
					guild,
					revision,
					epoch,
					result,
				} => {
					if generation == self.state.generation
						&& self
							.cache
							.as_ref()
							.is_some_and(|cache| cache.history.allows(*epoch))
					{
						match result {
							Ok(rows) => {
								if !rows.is_empty() {
									self.member_cache_load.member_result(
										(generation, *guild, 0, *epoch),
										*revision,
										&self.state,
									);
								}
								self.state
									.hydrate_member_displays(*guild, *revision, rows.clone())
							}
							Err(_) => {
								self.cache_error = true;
								self.cache_status = "Could not restore cached member names; live lookups remain available";
							}
						}
					}
					continue;
				}
				cache::Outcome::Expressions {
					account,
					guild,
					revision,
					epoch,
					result,
				} => {
					if generation == self.state.generation
						&& self
							.state
							.user
							.as_ref()
							.is_some_and(|user| user.id == *account)
						&& self
							.cache
							.as_ref()
							.is_some_and(|cache| cache.history.allows(*epoch))
					{
						match result {
							Ok(Some(catalog)) => {
								if !self.state.hydrate_expression_catalog(
									*guild,
									*revision,
									catalog.clone(),
								) {
									self.expression_cache
										.enqueue(*guild, cache::ExpressionCacheSchedule::LOAD);
								}
							}
							Ok(None) => {}
							Err(_) => {
								if !self.cache_error {
									self.cache_status = "Cached emoji and stickers unavailable; live catalogs remain available";
								}
							}
						}
					} else if generation == self.state.generation
						&& self
							.state
							.user
							.as_ref()
							.is_some_and(|user| user.id == *account)
					{
						self.expression_cache
							.enqueue(*guild, cache::ExpressionCacheSchedule::LOAD);
					}
					continue;
				}
				cache::Outcome::StickerPackHints {
					account,
					revision,
					epoch,
					result,
				} => {
					if generation == self.state.generation
						&& self
							.state
							.user
							.as_ref()
							.is_some_and(|user| user.id == *account)
						&& self
							.cache
							.as_ref()
							.is_some_and(|cache| cache.history.allows(*epoch))
					{
						match result {
							Ok(Some(packs)) => {
								if !self
									.state
									.hydrate_sticker_pack_hints(*revision, packs.clone())
								{
									self.sticker_pack_cache_load
										.retry((generation, *account, *revision, *epoch));
								}
							}
							Ok(None) => {}
							Err(_) => {
								if !self.cache_error {
									self.cache_status = "Cached sticker packs unavailable; live packs remain available";
								}
							}
						}
					} else if generation == self.state.generation
						&& self
							.state
							.user
							.as_ref()
							.is_some_and(|user| user.id == *account)
					{
						self.sticker_pack_cache_load
							.retry((generation, *account, *revision, *epoch));
					}
					continue;
				}
				cache::Outcome::CustomFont(result) => {
					self.accept_font(ctx, result);
					continue;
				}
				cache::Outcome::AppPreferencesSaved(result) => {
					self.app_settings.state.saving = false;
					self.app_settings.state.failed = result.is_err();
					continue;
				}
				cache::Outcome::MinimizeToTray(result) => {
					self.tray_setting.restore(*result);
					if !self.fixture_only {
						self.messaging.minimize_to_tray = self.tray_setting.enabled;
					}
					continue;
				}
				cache::Outcome::MinimizeToTraySaved(result) => {
					self.tray_setting.saving = false;
					self.tray_setting.failed = result.is_err();
					continue;
				}
				cache::Outcome::GameActivity(result) => {
					self.game_activity.restore(*result);
					if !self.state.demo && !self.fixture_only {
						self.messaging.share_game_activity = self.game_activity.enabled;
					}
					continue;
				}
				cache::Outcome::GameActivitySaved(result) => {
					self.game_activity.saving = false;
					self.game_activity.failed = result.is_err();
					continue;
				}
				cache::Outcome::ReadingPreferences(result) => {
					if let Some(value) = self.reading.restore(*result)
						&& !self.state.demo
						&& !self.fixture_only
					{
						self.messaging.apply_reading_preferences(ctx, value);
					}
					continue;
				}
				cache::Outcome::ReadingPreferencesSaved(result) => {
					self.reading.saved(*result);
					continue;
				}
				cache::Outcome::Appearance(appearance, variant) => {
					if !self.state.demo && !self.appearance_changed {
						self.appearance = match appearance {
							local_store::Appearance::System => egui::ThemePreference::System,
							local_store::Appearance::Light => egui::ThemePreference::Light,
							local_store::Appearance::Dark => egui::ThemePreference::Dark,
						};
						ctx.set_theme(self.appearance);
					}
					if !self.state.demo && !self.variant_changed {
						// Unknown keys from a newer build fall back to the default preset.
						let variant = variant
							.as_deref()
							.and_then(ui::design::Variant::from_key)
							.unwrap_or_default();
						ui::design::set_variant(variant);
						ui::design::apply(ctx);
					}
					continue;
				}
				cache::Outcome::Failed {
					error,
					message,
					draft_restore,
					history_cleanup,
				} => {
					if *history_cleanup && let Some(cache) = &self.cache {
						self.cache_clears.acknowledge(&cache.history);
					}
					let _ = error;
					self.cache_error = true;
					self.cache_status = message;
					if *draft_restore && generation == self.state.generation {
						self.messaging.draft_restore_pending = false;
						self.state.drafts.retain(|_, content| !content.is_empty());
					}
					continue;
				}
				cache::Outcome::Accounts { roster, pruned } => {
					self.roster_pending = false;
					// Pruning is already committed, so clean up regardless of the re-read.
					for account in pruned {
						let known_accounts = self.state.user.iter().map(|user| user.id).chain(
							roster
								.as_ref()
								.unwrap_or(&self.messaging.accounts)
								.iter()
								.map(|saved| saved.id),
						);
						if !self.store.as_mut().is_some_and(|store| {
							store.forget_account(self.state.generation, *account, known_accounts)
						}) {
							self.credential_status = "Could not queue pruned account login removal; its OS credential may remain";
						}
						self.queue_cache_for(*account, cache::Operation::Forget);
					}
					match roster {
						Ok(accounts) => {
							self.messaging.accounts = accounts.clone();
							if let Some(store) = &mut self.store {
								store.retain_accounts(
									accounts.iter().map(|account| account.id).collect(),
								);
							}
						}
						Err(_) => {
							self.roster_failed = true;
							self.cache_error = true;
							self.cache_status = "Could not read or update the saved account list";
						}
					}
					continue;
				}
				cache::Outcome::AccountPresences(result) => {
					self.presence_load_pending = false;
					if let Ok(rows) = result {
						self.account_presences.clone_from(rows);
					}
					self.flush_deferred_connect(ctx);
					continue;
				}
				cache::Outcome::AccountPresenceSaved(result) => {
					if result.is_err() {
						self.presence_saved = None;
					}
					continue;
				}
				cache::Outcome::HistoryCleared => {
					if let Some(cache) = &self.cache
						&& self.cache_clears.acknowledge(&cache.history)
						&& !self.cache_error
					{
						if cache.history.allows(cache.history.epoch()) {
							self.cache_status = "Cached history cleared; saved drafts preserved";
						} else {
							self.cache_status = "Requested history cleanup completed; history cache remains disabled until restart after a storage failure";
						}
					}
					continue;
				}
				_ => {}
			}
			if generation != self.state.generation {
				continue;
			}
			match outcome {
				cache::Outcome::MemberDisplaysFound {
					request,
					revision,
					visible,
					epoch,
					result,
				} => {
					if visible {
						self.accept_visible_member_cache(
							generation, request, revision, epoch, result,
						);
					} else {
						self.accept_member_cache_search(
							generation, request, revision, epoch, result,
						);
					}
				}
				cache::Outcome::HistoryPage {
					channel,
					request,
					epoch,
					result,
				} => {
					self.accept_cached_history_page(generation, channel, request, epoch, result);
				}
				cache::Outcome::SyncRevision { .. }
				| cache::Outcome::SyncPageSaved { .. }
				| cache::Outcome::SyncCheckpoint { .. }
				| cache::Outcome::SyncCheckpointSaved { .. }
				| cache::Outcome::SyncMembersSaved { .. }
				| cache::Outcome::SyncMembersReconciled { .. }
				| cache::Outcome::CacheUsage(_) => {}
				cache::Outcome::ChannelPreferences(result) => {
					self.messaging.channel_preferences_load_pending = false;
					self.messaging.channel_preferences_reload = false;
					match result {
						Ok(preferences) => self.messaging.restore_channel_preferences(preferences),
						Err(error) => {
							self.messaging.channel_preferences_status = match error {
								local_store::StoreError::Incompatible => {
									"Saved channel preferences are damaged or incompatible with this build."
								}
								_ => "Could not read channel preferences from local storage.",
							};
						}
					}
				}
				cache::Outcome::ChannelPreferencesSaved(result) => {
					self.messaging.channel_preferences_save_pending = false;
					self.messaging.channel_preferences_status = if result.is_ok() {
						""
					} else {
						"Could not save channel preferences."
					};
				}
				cache::Outcome::GifFavorites(favorites) => {
					self.state.restore_gif_favorites(favorites);
					if self.state.gif_favorites_cache_safe()
						&& let Some(account) = self.state.user.as_ref().map(|user| user.id)
					{
						self.gif_cache.loaded((generation, account));
					} else if !self.state.gif_favorites_cache_safe() {
						self.cache_error = true;
						self.cache_status = "GIF cache could not be safely reconciled; changes remain in this session";
					}
				}
				cache::Outcome::Drafts(drafts) => {
					for (channel, content) in drafts {
						if !self
							.state
							.pending
							.iter()
							.any(|pending| pending.channel == channel)
						{
							self.state.drafts.entry(channel).or_insert(content);
						}
					}
					self.messaging.draft_restore_pending = false;
					self.state.drafts.retain(|_, content| !content.is_empty());
					if !self.cache_error {
						self.cache_status = "Saved drafts restored; check the conversation before resending recovered text";
					}
				}
				outcome @ cache::Outcome::Channel { .. } => {
					if let Some(cache) = &self.cache {
						hydrate_cache_result(&mut self.state, &cache.history, outcome);
					}
				}
				cache::Outcome::Saved => {
					if !self.cache_error {
						self.cache_status = "Local changes saved";
					}
				}
				cache::Outcome::Appearance(..)
				| cache::Outcome::CustomFont(_)
				| cache::Outcome::AppPreferencesSaved(_)
				| cache::Outcome::MinimizeToTray(_)
				| cache::Outcome::MinimizeToTraySaved(_)
				| cache::Outcome::GameActivity(_)
				| cache::Outcome::GameActivitySaved(_)
				| cache::Outcome::ReadingPreferences(_)
				| cache::Outcome::ReadingPreferencesSaved(_)
				| cache::Outcome::Accounts { .. }
				| cache::Outcome::HistoryCleared
				| cache::Outcome::ScopedCleanupFailed { .. }
				| cache::Outcome::Members { .. }
				| cache::Outcome::Expressions { .. }
				| cache::Outcome::StickerPackHints { .. }
				| cache::Outcome::Servers { .. }
				| cache::Outcome::ServersSaved { .. }
				| cache::Outcome::Failed { .. }
				| cache::Outcome::AccountPresences(_)
				| cache::Outcome::AccountPresenceSaved(_) => unreachable!(),
			}
		}
		self.retry_history_clears();
		self.sync_plaintext_login();
		let mut results = Vec::new();
		if let Some(store) = &mut self.store {
			for _ in 0..4 {
				match store.poll(
					std::time::Instant::now(),
					self.state.user.as_ref().map(|user| user.id),
				) {
					Some(result) => results.push(result),
					None => break,
				}
			}
			if let Some(remaining) = store.remaining(std::time::Instant::now()) {
				ctx.request_repaint_after(remaining);
			}
		}
		for (generation, outcome) in results {
			if generation != self.state.generation
				&& !matches!(&outcome, credentials::Outcome::AccountForgotten(account, _) | credentials::Outcome::Forgotten(Some(account), _)
					if credentials::account_removal_applies(generation, self.state.generation,
						*account, self.state.user.as_ref().map(|user| user.id)))
				&& !matches!(
					&outcome,
					credentials::Outcome::PlaintextSet(..) | credentials::Outcome::Reconciled(..)
				) {
				continue;
			}
			match outcome {
				credentials::Outcome::Loaded(result) => {
					let switching = self.switching.take();
					if let Ok(Some(secret)) = result {
						// `connect()` already sets a "Connecting to Discord…" status; avoid
						// stacking a second, near-duplicate line under the restore screen.
						self.credential_status = "";
						self.connect(secret, switching.is_some(), ctx);
					} else {
						self.credential_status = credentials::loaded_status(&result);
						if let Some(account) = switching {
							// The entry stays: the owner decides whether to forget it. Signing
							// in again with "Use another account" refreshes its token, which the
							// roster must expect again.
							self.queue_cache_for(
								model::Id(0),
								cache::Operation::SetAccountToken {
									account,
									has_token: false,
								},
							);
							self.credential_status = "No saved login for that account on this device. Use another account to sign in again, or forget it with ×.";
							self.messaging.toasts.push(
								ui::design::Level::Warning,
								"That account's saved login is missing; sign in again to refresh it",
							);
						}
					}
				}
				credentials::Outcome::AccountSaved(account, result, launch) => {
					if result.is_ok() {
						self.queue_cache_for(
							model::Id(0),
							cache::Operation::SetAccountToken {
								account,
								has_token: true,
							},
						);
					} else if result != Err(platform::CredentialError::NoStore) {
						self.credential_status =
							"Could not save this account for the switcher; sign in again to retry";
					}
					match launch {
						Some(Ok(saved)) => {
							self.credential_status = match saved {
								platform::Saved::CredentialStore => {
									"Login saved in the OS credential store"
								}
								platform::Saved::Plaintext => {
									"Login saved in plaintext because no OS keyring is available"
								}
							};
						}
						Some(Err(_)) => {
							self.credential_status =
								"Account login saved, but automatic sign-in could not be updated"
						}
						None if result == Err(platform::CredentialError::NoStore) => {
							self.messaging.toasts.push(
								ui::design::Level::Warning,
								"No OS keyring found, so you will need to sign in again next launch. Settings › Storage can save it in plaintext instead.",
							);
						}
						None => {}
					}
				}

				credentials::Outcome::AccountForgotten(account, result) => {
					self.account_forgotten(generation, account, result, false);
				}
				credentials::Outcome::Reconciled(result) => {
					if result.is_err() {
						self.messaging.toasts.push(
							ui::design::Level::Error,
							"Could not remove orphaned plaintext saved logins",
						);
					}
				}
				credentials::Outcome::PlaintextSet(enabled, result) => {
					if result.is_err() {
						self.messaging.toasts.push(
							ui::design::Level::Error,
							"Could not delete the plaintext saved logins in NuSerein's data folder",
						);
					} else if !enabled {
						self.credential_status = "Plaintext saved logins deleted";
					}
				}
				credentials::Outcome::Forgotten(account, result) => {
					self.forgetting = false;
					self.credential_status = match result {
						Ok(()) => "Saved login removed",
						// Nothing could have been saved without a credential store.
						Err(platform::CredentialError::NoStore) => "",
						Err(_) => {
							"Could not remove NuSerein's saved login (under cz.viceverse.serein) in your OS credential manager"
						}
					};
					if let Some(account) = account {
						self.account_forgotten(generation, account, result, true);
					}
				}
			}
		}
		let mut events = Vec::new();
		let mut terminal = None;
		let mut delivery_pressure = false;
		if let Some(connection) = &mut self.connection {
			delivery_pressure = connection.events.take_pressure();
			for _ in 0..client_core::EVENT_SLOTS {
				match connection.events.try_recv_epoch() {
					Ok((event, epoch)) => events.push((event, Some(epoch))),
					Err(_) => break,
				}
			}
			// Collect reliable events first: their preceding typing signals are now queued.
			// Apply typing first so messages/access changes retire those older signals.
			let reliable_count = events.len();
			for _ in 0..8 {
				match connection.typing.try_recv() {
					Ok(event) => events.push((event, None)),
					Err(_) => break,
				}
			}
			let typing_count = events.len() - reliable_count;
			events.rotate_right(typing_count);
			// A local candidate failure must remain deliverable when reliable account
			// events are full. Apply queued signaling first; observe rechecks its scope.
			if let Some(failure) = connection::take_confirmation_failure(
				&mut connection.confirmation_failure,
				&connection.events,
			) {
				events.push((failure, None));
			}
			delivery_pressure |= connection.events.take_pressure();
			terminal = *connection.terminal.borrow();
			if terminal.is_some() {
				connection.copy_diagnostics_to(&mut self.messaging);
			}
		}
		if delivery_pressure {
			self.voice.stop();
			self.state.apply(Envelope {
				generation: self.state.generation,
				event: Event::Disconnected,
			});
			self.state.apply(Envelope {
				generation: self.state.generation,
				event: Event::Resync,
			});
			self.queue_cache(cache::Operation::ClearHistory);
			// Results already lost to congestion may include accepted writes. Show
			// uncertainty; a fresh snapshot never authorizes replaying these writes.
			for pending in &mut self.state.pending {
				if pending.delivery == Delivery::Sending {
					pending.delivery = Delivery::Ambiguous;
				}
			}
		}

		let mut persist_timeline = false;
		let mut full_window = false;
		let mut changed_messages = std::collections::BTreeSet::new();
		let mut member_cleanup = cache::MemberDisplayCleanup::default();
		for (mut event, epoch) in events {
			if epoch.is_some_and(|epoch| {
				self.connection
					.as_ref()
					.is_none_or(|c| !c.events.is_current(epoch))
			}) {
				continue;
			}
			if event.generation != self.state.generation {
				continue;
			}
			if let Event::GuildRemoved(guild) = &event.event {
				self.invalidate_cache_scope(cache::Operation::RemoveServer { guild: *guild });
				// Legacy history has no guild mapping. On confirmed removal, retire
				// account history too so archived channels cannot remain orphaned.
				self.invalidate_cache_scope(cache::Operation::ClearHistory);
			}
			let user_action_notice = user_action_notice(&event.event);
			self.observe_force_sync_event(&event.event);
			let user_action_was_pending =
				user_action_notice.is_some() && self.state.user_action_pending();
			self.delete_cached_messages(&event.event);
			match &event.event {
				Event::Delete { channel, id } => {
					self.messaging
						.messages_deleted(ctx, *channel, std::slice::from_ref(id));
				}
				Event::DeleteBulk { channel, ids } if ids.len() <= 100 => {
					self.messaging.messages_deleted(ctx, *channel, ids);
				}
				_ => {}
			}
			let takeover_notice = voice::takeover_notice(&self.state, &event.event);
			let voice_failure = self.voice.observe(&self.state, &mut event.event);
			if event.generation == self.state.generation
				&& let Event::Soundboard(effect) = &event.event
			{
				self.voice
					.soundboard(&self.runtime, &self.state, &self.messaging, ctx, effect);
			}
			let ready = event.event.ready_navigation().is_some();
			let expression_joined = match &event.event {
				Event::GuildJoined(guild) => Some(guild.id),
				_ => None,
			};
			let expression_changed = match &event.event {
				Event::GuildRemoved(guild) => Some((*guild, None)),
				Event::GuildEmojis { guild, .. } => Some((*guild, Some(true))),
				Event::GuildStickers { guild, .. } => Some((*guild, Some(false))),
				_ => None,
			};
			let resumed = matches!(event.event, Event::Resumed);
			if (ready || resumed)
				&& let Some(connection) = &self.connection
			{
				connection.gateway_recovered();
			}
			let confirmed_channel = confirmed_recovery_channel(&self.state, &event.event);
			let deleted_shortcut = match &event.event {
				Event::Unavailable(channel)
				| Event::ThreadRemoved { id: channel, .. }
				| Event::ChannelAction(client_core::channel_actions::Event::Finished {
					channel,
					result: Ok(client_core::channel_actions::Outcome::Deleted),
					..
				}) => Some(*channel),
				_ => None,
			};
			let mut removed_channels = access_candidates(&self.state, &event.event);
			removed_channels.retain(|id| self.state.can_read_history(*id));
			let invalidate = matches!(event.event, Event::Resync | Event::PermissionsChanged);
			let scoped_loss = match &event.event {
				Event::Unavailable(channel) | Event::ThreadRemoved { id: channel, .. } => {
					Some(*channel)
				}
				Event::RecipientRemoved { channel, user }
					if self
						.state
						.user
						.as_ref()
						.is_some_and(|owner| owner.id == *user) =>
				{
					Some(*channel)
				}
				Event::Reactions(client_core::reactions::Event::Read {
					channel,
					result: Err(Failure::Forbidden),
					..
				}) if self.state.selected == Some(*channel) => Some(*channel),
				Event::HistoryFailed {
					channel,
					request,
					failure: Failure::Forbidden,
				} if self.state.selected == Some(*channel)
					&& self.state.request == self.state.history_request(*request)
					&& self.state.history_pending =>
				{
					Some(*channel)
				}
				_ => None,
			};
			let removed_member = cache::departed_member_target(&self.state, &event);
			let display_update = cache::member_display_target(&event.event);
			let member_observation_guild = cache::member_observation_guild(&event.event);
			let before_member_cache = self.state.member_display_cache_revision();
			let before_member_observation = self.state.member_display_revision();
			let affected_guilds: std::collections::BTreeMap<_, _> = removed_channels
				.iter()
				.chain(scoped_loss.iter())
				.filter_map(|id| {
					self.state
						.channel(*id)
						.and_then(|c| c.guild)
						.map(|guild| (*id, guild))
				})
				.collect();
			let history_changed = changes_active_history(&self.state, &event.event);
			if history_changed {
				match &event.event {
					Event::Message(message)
					| Event::SendResult {
						result: Ok(message),
						..
					} => {
						changed_messages.insert(message.id);
					}
					Event::ServerAction(client_core::server_actions::Event::InviteSent {
						result: Ok(sent),
						..
					}) => {
						changed_messages.insert(sent.1.id);
					}
					Event::Edited { message, .. } => {
						changed_messages.insert(*message);
					}
					Event::Patch(patch) => {
						changed_messages.insert(patch.id);
					}
					_ => full_window = true,
				}
			}
			let guild_ack = match &event.event {
				Event::ReadState(client_core::read_state::Event::GuildAck {
					guild,
					request,
					result: Ok(()),
				}) => self.state.pending_guild_ack(*guild, *request),
				Event::ReadState(client_core::read_state::Event::AllAck {
					request,
					result: Ok(()),
					..
				}) => self.state.pending_all_ack(*request),
				_ => false,
			};
			if event.generation == self.state.generation
				&& (invalidate
					|| event.event.changes_access()
					|| guild_ack || matches!(
					&event.event,
					Event::NotificationPreferences(_)
						| Event::ChannelAction(_)
						| Event::UserAction(_)
						| Event::Disconnected
						| Event::ReadState(client_core::read_state::Event::Ack { .. })
						| Event::ReadState(client_core::read_state::Event::Result {
							result: Ok(()),
							..
						})
				)) {
				self.notifications.dismiss();
			}
			let data_changes = extension_data_events::Changes::capture(&self.state, &event);
			let extension_events = if self.extensions.has_message_events(&self.state) {
				extension_events::capture(&self.state, &event.event)
			} else {
				Vec::new()
			};
			if event.generation == self.state.generation
				&& (event.event.changes_access()
					|| matches!(event.event, Event::Disconnected)
					|| matches!(&event.event, Event::HistoryFailed { channel, request, failure: Failure::Forbidden }
						if Some(*channel) == self.state.selected && *request == self.state.request && self.state.history_pending))
			{
				self.extensions.access_changed(&mut self.messaging);
			}
			if self
				.member_cache_load
				.observe_session(self.state.generation, &event)
			{
				self.member_cache_saved = None;
				self.member_cache_cleared = Default::default();
				self.member_cache_search = Default::default();
			}
			let reset_expression = event.event.invalidates_expression_catalogs();
			let reset_session = event.event.ready_navigation().is_some()
				|| matches!(
					event.event,
					Event::Disconnected | Event::Resumed | Event::Resync
				);
			if reset_expression {
				self.sticker_pack_cache_load = Default::default();
				self.sticker_pack_cache_save = Default::default();
			} else {
				self.sticker_pack_cache_load
					.observe_session(self.state.generation, &event);
				self.sticker_pack_cache_save
					.observe_session(self.state.generation, &event);
			}
			if self
				.server_cache_load
				.observe_session(self.state.generation, &event)
			{
				self.server_cache_failed = false;
			}
			if matches!(event.event, Event::GuildJoined(_)) {
				self.server_cache_load = Default::default();
			}
			self.state.apply(event);
			self.member_cache_cleared.observe(
				&self.state,
				member_observation_guild,
				before_member_observation,
			);
			// Coalesce changed display rows before any post-drain cache read or save.
			if let Some((generation, account, operation)) =
				member_cleanup.observe(&self.state, display_update, before_member_cache)
			{
				self.invalidate_cache_scope_for(generation, account, operation);
			}
			if reset_session {
				self.expression_cache = Default::default();
			}
			if let Some(guild) = expression_joined {
				self.expression_cache.enqueue(
					guild,
					cache::ExpressionCacheSchedule::LOAD | cache::ExpressionCacheSchedule::SAVE,
				);
			}
			if let Some((guild, kind)) = expression_changed {
				if kind.is_some() {
					self.expression_cache.enqueue(
						guild,
						cache::ExpressionCacheSchedule::LOAD | cache::ExpressionCacheSchedule::SAVE,
					);
				} else {
					self.expression_cache.remove(guild);
				}
			}
			let clear_expression = expression_changed.and_then(|(guild, kind)| {
				let Some(current) = self.state.guild(guild) else {
					return Some((guild, None));
				};
				match kind {
					None => Some((guild, None)),
					Some(true) if current.emojis.is_none() => Some((guild, Some(true))),
					Some(false) if current.stickers.is_none() => Some((guild, Some(false))),
					_ => None,
				}
			});
			if let Some(message) = takeover_notice {
				self.messaging
					.toasts
					.push(ui::design::Level::Info, ui::i18n::translate(message));
			}
			self.extensions.data_changed(data_changes);
			self.extensions.cancel_stale_message_events(&self.state);
			for candidate in extension_events {
				if let Some(event) = candidate.admit(&self.state) {
					self.extensions.message_event(&self.state, event);
				}
			}
			if user_action_was_pending
				&& !self.state.user_action_pending()
				&& let Some((level, text)) = user_action_notice
			{
				self.messaging.toasts.push(level, text);
			}
			// Only admitted service messages can establish a deleted reply target.
			// Fence pending disk writes before the post-drain timeline snapshot is saved.
			let deleted_replies = self.state.take_reply_deletions();
			if let Some(&(channel, _)) = deleted_replies.first() {
				full_window = true;
				let ids: Vec<_> = deleted_replies.into_iter().map(|(_, id)| id).collect();
				self.messaging.messages_deleted(ctx, channel, &ids);
				self.delete_cached_ids(channel, ids);
			}
			removed_channels.retain(|id| !self.state.can_read_history(*id));
			for channel in removed_channels.iter().copied().chain(deleted_shortcut) {
				if self.state.channel(channel).is_none() {
					self.messaging.channel_preferences_changed |=
						self.messaging.channel_preferences.forget(channel);
				}
			}
			if let Some(error) = voice_failure
				&& let Some(command) = self.voice.fail_or_rejoin(&mut self.state, error)
			{
				self.command(command);
			}
			if invalidate {
				self.queue_cache(cache::Operation::ClearHistory);
			} else {
				let mut removed: std::collections::BTreeSet<_> =
					removed_channels.iter().copied().collect();
				removed.extend(scoped_loss);
				let guilds: std::collections::BTreeSet<_> = removed
					.iter()
					.filter_map(|id| affected_guilds.get(id).copied())
					.collect();
				for channel in removed {
					self.clear_channel_cache(channel);
				}
				for guild in guilds {
					self.invalidate_cache_scope(cache::Operation::RemoveMember {
						guild,
						user: None,
					});
				}
				if let Some((guild, emoji)) = clear_expression {
					self.invalidate_cache_scope(cache::Operation::ClearExpressions {
						guild,
						emoji,
					});
				}
				if let Some((guild, user)) = removed_member {
					self.invalidate_cache_scope(cache::Operation::RemoveMember {
						guild,
						user: Some(user),
					});
				}
			}
			persist_timeline |= history_changed;
			if let Some(channel) = confirmed_channel {
				let content = recovery_draft(&self.state, channel);
				self.queue_cache(cache::Operation::SaveDraft { channel, content });
			}
			if ready && self.state.auth == AuthState::Authenticated {
				// The worker survives logout; each accepted account READY restores its own drafts.
				if !self.messaging.draft_restore_pending {
					self.messaging.draft_restore_pending =
						self.queue_cache(cache::Operation::LoadDrafts);
				}
				if !self.messaging.channel_preferences_loaded
					&& !self.messaging.channel_preferences_load_pending
				{
					self.messaging.channel_preferences_reload = true;
				}
				if let Some(store) = &mut self.store {
					// The active entry restores on launch; the per-account entry backs the switcher.
					let mut queued = true;
					// A token the owner just supplied replaces whatever was stored before.
					let fresh_token = self.pending_save.take().is_some();
					// Writing an existing entry is an access-controlled keychain operation on
					// macOS, so only write when the roster says none exists or the token is new.
					if let Some(secret) = self.pending_account_save.take()
						&& let Some(account) = self.state.user.as_ref().map(|user| user.id)
						&& (fresh_token
							|| !self
								.messaging
								.accounts
								.iter()
								.any(|saved| saved.id == account && saved.has_token))
					{
						queued &=
							store.save_account(self.state.generation, account, secret, fresh_token);
					}
					if !queued {
						self.credential_status = "Could not queue saved login; session only";
					}
				}
			}
			if (ready || resumed)
				&& self.state.auth == AuthState::Authenticated
				&& self.state.selected.is_some_and(|selected| {
					self.state
						.channels
						.iter()
						.any(|channel| channel.id == selected && channel.supports_text())
				}) {
				let command = self.state.history(None);
				self.command(command);
			}
		}
		if let Some((generation, account, operation)) = member_cleanup.take() {
			self.invalidate_cache_scope_for(generation, account, operation);
		}
		if persist_timeline
			&& self.state.freshness == model::Freshness::Fresh
			&& let Some(channel) = self.state.selected
			&& self.state.can_read_history(channel)
		{
			let messages = self
				.state
				.timeline
				.iter()
				.filter(|m| full_window || changed_messages.contains(&m.id))
				.cloned()
				.collect();
			let start = self.state.older_exhausted && !self.state.timeline.older_evicted();
			let operation = if full_window {
				cache::Operation::SaveChannel {
					channel,
					messages,
					start,
				}
			} else {
				cache::Operation::SaveChanges {
					channel,
					messages,
					retained: self.state.timeline.iter().map(|m| m.id).collect(),
					start,
				}
			};
			self.queue_cache(operation);
		}

		if let Some(failure) = terminal {
			for (setting, scope) in [
				("SEREIN_MEMBER_DIAGNOSTICS", "members"),
				("SEREIN_GATEWAY_DIAGNOSTICS", "gateway"),
			] {
				if std::env::var_os(setting).as_deref() == Some(std::ffi::OsStr::new("1")) {
					use std::io::Write;
					// One extra fixed-label terminal line per enabled scope; closed stderr is OK.
					let _ = writeln!(
						std::io::stderr(),
						"[Serein {scope}] Session stopped: {}",
						failure.label()
					);
				}
			}
			self.state.interrupt_gif_favorites();
			self.connection = None;
			self.pending_save = None;
			self.current_login = None;
			self.pending_account_save = None;
			self.voice_messages.cancel();
			self.voice.stop();
			self.messaging.camera_test_requested = false;
			self.messaging.camera_test_texture = None;
			self.state.apply(Envelope {
				generation: self.state.generation,
				event: Event::Failure(failure),
			});
			if failure == Failure::RateLimited && self.state.failure_detail.is_none() {
				self.state.failure_detail =
					Some(ui::i18n::translate("main-sign-in-rate-limited-detail").into());
			}
			if !matches!(self.state.auth, AuthState::Expired | AuthState::Challenged) {
				self.state.auth = AuthState::Failed;
			}
			for pending in &mut self.state.pending {
				if pending.delivery == Delivery::Sending {
					pending.delivery = Delivery::Ambiguous;
				}
			}
			if failure == Failure::Expired
				&& let Some(store) = &self.store
			{
				let _ = store.send.try_send((
					self.state.generation,
					credentials::Request::NONE,
					credentials::Operation::Forget(None),
				));
			}
		}
		if let Some(login) = &self.login {
			login.pump();
			if let Some(secret) = login.token() {
				self.connect(secret, true, ctx);
			} else if login.expired() {
				let crashed = login.crashed();
				self.login = None;
				self.state.auth = AuthState::Challenged;
				self.state.status = if crashed {
					"Login window stopped unexpectedly (web process ended); no session accepted"
				} else {
					"Login timed out or token handoff unavailable; no session accepted"
				};
			}
		}
		// Process READY first so a new session can supersede its own old deletion.
		self.retry_account_forgets();
		// Network and store workers request repaint only when their outcomes change.
		if let Some(connection) = &self.connection {
			connection.set_typing_channel(self.state.typing_scope());
		}
		if !self.state.demo
			&& let Some(command) = self.state.next_reaction_read()
		{
			self.command(command);
		}

		#[cfg(target_os = "linux")]
		if self.login.is_some() {
			ctx.request_repaint_after(self.frame_period().unwrap_or(Duration::from_millis(16)));
		}
		if self.login.is_some() {
			ctx.request_repaint_after(Duration::from_secs(1));
		}
	}
}
