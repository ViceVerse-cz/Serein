//! Sign-in UI and account-session lifecycle for the desktop client.
use super::*;

/// Sign-in header strip doubles as the window drag region, clearing the traffic lights.
pub(super) const SIGN_IN_HEADER_HEIGHT: f32 = if cfg!(target_os = "windows") {
	44.0
} else {
	60.0
};

/// How long a saved-login message stays on the sign-in screen.
const NOTICE_SECONDS: f64 = 8.0;
/// Padding inside the sign-in card.
const CARD_PADDING: f32 = 32.0;
/// The card with the QR code beside the form: both columns, the gap between them and the
/// card's padding.
const SIGN_IN_WIDE: f32 = 684.0 + CARD_PADDING * 2.0;

impl Desktop {
	pub(super) fn connect(&mut self, secret: SessionSecret, save: bool, ctx: &egui::Context) {
		if self.presence_load_pending {
			self.deferred_connect = Some((secret, save));
			self.credential_status = "Connecting to Discord…";
			return;
		}
		self.role_icon.cancel();
		self.role_icon_scope = None;
		self.group_icon.cancel();
		self.create_server_icon.cancel();
		self.profile_avatar.cancel();
		self.server_icon.cancel();
		self.emoji_upload.cancel();
		self.sticker_upload.cancel();
		if let Some(store) = &mut self.store {
			store.cancel_load();
		}
		self.credential_status = "";
		self.voice.end_session();
		self.messaging.camera_test_requested = false;
		self.messaging.camera_test_texture = None;
		self.state
			.disconnect_voice("Discord login session changed; start a new call");
		self.uploads.cancel();
		self.uploads.forget_parked();
		self.login = None;
		self.native.close();
		self.messaging.close_settings();
		self.state.interrupt_gif_favorites();
		self.state.interrupt_emoji_frecency();
		self.state.interrupt_preloads();
		self.connection = None;
		if let Some(worker) = self.avatars.take() {
			self.avatar_cleanup = Some(worker.shutdown());
		}
		self.avatar_start_failed = false;
		self.messaging.clear_avatars();
		self.state.generation += 1;
		self.messaging.channel_preferences = model::ChannelPreferences::default();
		self.messaging.channel_preferences_changed = false;
		self.messaging.channel_preferences_loaded = false;
		self.messaging.channel_preferences_load_pending = false;
		self.messaging.channel_preferences_reload = false;
		self.messaging.channel_preferences_save_pending = false;
		self.messaging.channel_preferences_status = "";
		self.presence_authoritative = false;
		self.presence_saved = None;
		let cached = self
			.state
			.user
			.as_ref()
			.and_then(|user| self.account_presences.get(&user.id).cloned());
		if let Some(cached) = cached {
			self.messaging.adopt_account_presence(cached);
		} else {
			self.messaging.own_presence = model::OwnPresence::default();
			self.messaging.own_presence_expires = None;
		}
		self.messaging.own_presence_changed = false;
		self.messaging.draft_restore_pending = false;
		self.state.auth = AuthState::Authenticating;
		self.state.status = "Connecting to Discord…";
		self.state.failure_detail = None;
		let secret = Arc::new(secret);
		self.pending_save = save.then(|| secret.clone());
		self.pending_account_save = Some(secret.clone());
		self.connection = Some(connection::Connection::start(
			self.runtime.handle(),
			secret,
			self.state.generation,
			self.state.user.as_ref().map(|u| u.id),
			self.account_presences.clone(),
			self.api_proxy.subscribe(),
			ctx.clone(),
		));
	}
	fn logout(&mut self, ctx: &egui::Context) {
		self.end_session(ctx, SessionEnd::Logout);
	}
	/// Ends the current session. Only logging out removes this account's local data and
	/// saved login; switching and adding keep both so the account stays in the switcher.
	fn end_session(&mut self, ctx: &egui::Context, intent: SessionEnd) {
		self.captcha.close();
		self.notification_runtime.clear(&self.window);
		self.messaging.image_share_requested = None;
		let extension_logout = self.extensions.logout(ctx);
		self.role_icon.cancel();
		self.role_icon_scope = None;
		self.group_icon.cancel();
		self.create_server_icon.cancel();
		self.profile_avatar.cancel();
		self.server_icon.cancel();
		self.emoji_upload.cancel();
		self.sticker_upload.cancel();
		self.notifications.clear();
		self.uploads.cancel();
		self.uploads.forget_parked();
		if let Some(store) = &mut self.store {
			store.cancel_load();
		}
		self.downloads.cancel();
		self.audio.stop();
		self.video.stop();
		self.video.cancel_posters();
		self.voice.end_session();
		self.messaging.camera_test_requested = false;
		self.messaging.camera_test_texture = None;
		let was_demo = self.state.demo;
		self.clear_avatars(ctx);
		self.login = None;
		self.native.close();
		self.connection = None;
		self.pending_save = None;
		self.pending_account_save = None;
		let old_account = self.state.user.as_ref().filter(|_| !was_demo).map(|u| u.id);
		self.switching = None;
		self.roster_pending = false;
		self.state.logout();
		if intent.forgets()
			&& let (Some(cache), Some(account)) = (&self.cache, old_account)
		{
			if cache.queue(self.state.generation, account, cache::Operation::Forget) {
				self.cache_pending += 1;
			} else {
				self.cache_error = true;
				self.cache_status = "Could not queue local account data removal";
			}
		}
		self.messaging.clear();
		if intent.forgets() {
			self.messaging
				.accounts
				.retain(|saved| Some(saved.id) != old_account);
		}
		if let Err(error) = extension_logout {
			self.messaging.extensions.status = error;
			self.cache_error = true;
			self.cache_status = "Extension account data removal could not be queued";
		}
		self.app_settings.apply(&mut self.messaging);
		self.messaging.share_game_activity = self.game_activity.enabled;
		ctx.memory_mut(|m| *m = egui::Memory::default());
		let _ = ui::emoji::install(ctx);
		ui::design::apply(ctx);
		ctx.set_theme(self.appearance);
		self.sync_system_theme(ctx);
		self.messaging
			.apply_reading_preferences(ctx, self.reading.current);
		ctx.clear_animations();
		self.token_input = Zeroizing::new(String::new());
		if !was_demo
			&& intent.forgets()
			&& let Some(store) = &self.store
		{
			self.forgetting = store
				.send
				.try_send((
					self.state.generation,
					credentials::Request::NONE,
					credentials::Operation::Forget,
				))
				.is_ok();
			if let Some(account) = old_account {
				let _ = store.send.try_send((
					self.state.generation,
					credentials::Request::NONE,
					credentials::Operation::ForgetAccount(account),
				));
			}
			self.credential_status = if self.forgetting {
				"Removing saved login…"
			} else {
				"Could not reach the credential store. The saved login may remain."
			};
		} else if !was_demo {
			self.credential_status = "";
		}
		self.confirming_logout = false;
		self.end_intent = SessionEnd::Logout;
	}
	/// Confirms first when work would be lost, then ends the session for `intent`.
	pub(super) fn request_session_end(&mut self, ctx: &egui::Context, intent: SessionEnd) {
		// Switching or adding keeps drafts the local cache has saved; logout removes them.
		let drafts_kept = !intent.forgets()
			&& !self.state.demo
			&& !self.fixture_only
			&& self.cache.is_some()
			&& !self.cache_error;
		let unsent = if drafts_kept {
			self.state
				.pending
				.iter()
				.any(|p| p.delivery != Delivery::Confirmed)
		} else {
			self.state.has_unsent()
		};
		if unsent
			|| self.messaging.has_edit()
			|| self.messaging.has_server_settings_changes()
			|| self.messaging.extensions.theme_editor_dirty()
			|| self.state.server_settings.pending
			|| self.state.server_admin.pending
			|| self.uploads.has_unsent()
			|| self.messaging.external_upload.has_unsent()
		{
			self.end_intent = intent;
			self.confirming_logout = true;
		} else {
			self.finish_session_end(ctx, intent);
		}
	}
	pub(super) fn finish_session_end(&mut self, ctx: &egui::Context, intent: SessionEnd) {
		self.end_session(ctx, intent);
		if let SessionEnd::Switch(account) = intent {
			self.begin_switch(account);
		}
	}
	/// Reads the saved token of an account already signed in on this device.
	fn begin_switch(&mut self, account: model::Id) {
		if self.fixture_only || self.state.demo {
			return;
		}
		let started = self.store.as_mut().is_some_and(|store| {
			store.load_account(self.state.generation, account, std::time::Instant::now())
		});
		self.switching = started.then_some(account);
		self.credential_status = if started {
			"Reading the saved login for that account…"
		} else {
			"Could not read the saved login. Sign in again."
		};
	}
	/// Removes one saved account: its token, roster entry and cached data.
	fn forget_saved_account(&mut self, ctx: &egui::Context, account: model::Id) {
		if self
			.state
			.user
			.as_ref()
			.is_some_and(|user| user.id == account)
		{
			self.request_session_end(ctx, SessionEnd::Logout);
			return;
		}
		self.messaging.accounts.retain(|saved| saved.id != account);
		if let Some(store) = &self.store {
			let _ = store.send.try_send((
				self.state.generation,
				credentials::Request::NONE,
				credentials::Operation::ForgetAccount(account),
			));
		}
		if self.queue_cache_for(account, cache::Operation::Forget) {
			self.credential_status = "Saved account removed from this device";
		} else {
			self.cache_error = true;
			self.cache_status = "Could not queue local account data removal";
		}
	}
	/// Keeps the switcher entry for the signed-in account current, without retrying failures.
	pub(super) fn sync_account_roster(&mut self) {
		if self.fixture_only || self.state.demo || self.roster_pending || self.roster_failed {
			return;
		}
		if self.state.auth != AuthState::Authenticated {
			return;
		}
		let Some(user) = &self.state.user else { return };
		// The switcher lists the handle under the display name. Only the account's own profile
		// carries the handle, so until it loads the entry keeps what the roster already knows.
		let profile = self.state.own_profile.data.as_ref();
		let previous = self
			.messaging
			.accounts
			.iter()
			.find(|saved| saved.id == user.id);
		let account = model::SavedAccount::identity(
			user,
			profile.map(|profile| (profile.username.as_str(), profile.global_name.as_deref())),
			previous,
		);
		if !account.is_valid() {
			return;
		}
		// Re-record once per session so the switcher orders by last use, then only on change.
		let recorded = self.roster_generation == Some(self.state.generation);
		if recorded
			&& self.messaging.accounts.iter().any(|saved| {
				saved.id == account.id
					&& saved.name == account.name
					&& saved.avatar == account.avatar
					&& saved.discriminator == account.discriminator
					&& saved.display == account.display
			}) {
			return;
		}
		self.roster_pending =
			self.queue_cache_for(model::Id(0), cache::Operation::SaveAccount(account));
		self.roster_failed = !self.roster_pending;
		if self.roster_pending {
			self.roster_generation = Some(self.state.generation);
		}
	}
}

impl Desktop {
	/// Boot stage while a saved login is being restored, so launch shows progress
	/// instead of a welcome card the user cannot act on yet.
	pub(super) fn restoring(&self) -> Option<String> {
		// Fixture-only preview of the restore screen, e.g. `--demo --demo-restoring`.
		#[cfg(feature = "demo")]
		if self.fixture_only && std::env::args().any(|arg| arg == "--demo-restoring") {
			return Some(ui::i18n::translate(
				"main-restoring-checking-your-saved-login",
			));
		}
		if self.fixture_only
			|| self.state.demo
			|| self.login.is_some()
			|| self.forgetting
			|| matches!(
				self.state.auth,
				AuthState::Failed | AuthState::Expired | AuthState::Challenged
			) {
			return None;
		}
		if self
			.store
			.as_ref()
			.is_some_and(|store| store.remaining(std::time::Instant::now()).is_some())
		{
			return Some(ui::i18n::translate(
				"main-restoring-checking-your-saved-login",
			));
		}
		self.connection
			.is_some()
			.then(|| ui::i18n::translate("main-restoring-connecting-to-discord"))
	}
	/// Restore screen for returning accounts: no sign-in controls, just the stage,
	/// an indeterminate bar and a way out to the welcome screen.
	pub(super) fn restoring_screen(&mut self, ui: &mut egui::Ui, stage: &str) {
		let p = ui::design::palette(ui);
		let screen = self.extensions.start_screen();
		egui::CentralPanel::default()
			.frame(egui::Frame::NONE.fill(ui::design::window_palette(ui).canvas))
			.show(ui, |ui| {
				if !self.messaging.uses_native_title_bar() {
					ui::design::window_drag(ui, ui.max_rect());
				}
				let custom_windows_controls = ui::design::WINDOW_CONTROLS_WIDTH > 0.0
					&& !self.messaging.uses_native_title_bar();
				egui::Frame::NONE
					.inner_margin(egui::Margin {
						left: (16.0 + self.traffic_light_inset()) as i8,
						right: if custom_windows_controls { 0 } else { 16 },
						top: if custom_windows_controls { 0 } else { 16 },
						bottom: 0,
					})
					.show(ui, |ui| {
						ui.horizontal(|ui| {
							ui.label(
								ui::design::semibold(
									ui,
									ui::i18n::translate("main-restoring-screen-serein"),
									16.0,
								)
								.color(p.muted),
							);
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									if !self.messaging.uses_native_title_bar() {
										ui::design::window_controls(ui);
									}
								},
							);
						});
					});
				let time = ui.input(|i| i.time) as f32;
				ui.ctx().request_repaint();
				ui.vertical_centered(|ui| {
					ui.add_space(
						(ui.available_height() * 0.5 - 140.0 - self.start_screen.lift()).max(12.0),
					);
					// A granted plugin's screen replaces the mark and copy with its own images
					// and text; the bar and the way out below always stay.
					self.start_screen.header(
						ui,
						screen,
						|ui| app_mark(ui, 48.0),
						&ui::i18n::translate("main-restoring-screen-welcome-back"),
						&format!("{stage}…"),
					);
					ui.add_space(24.0);
					// Indeterminate track: progress is unknown, so a sweeping segment.
					let width = ui.available_width().min(260.0);
					let (track, _) =
						ui.allocate_exact_size(egui::vec2(width, 4.0), egui::Sense::hover());
					ui.painter().rect_filled(track, 2, p.raised);
					let span = track.width() * 0.35;
					let travel = (track.width() + span) * ((time * 0.5).fract());
					let left = (track.left() + travel - span).max(track.left());
					let right = (track.left() + travel).min(track.right());
					if right > left {
						ui.painter().rect_filled(
							egui::Rect::from_min_max(
								egui::pos2(left, track.top()),
								egui::pos2(right, track.bottom()),
							),
							2,
							p.accent,
						);
					}
					ui.add_space(24.0);
					ui.allocate_ui_with_layout(
						egui::vec2(220.0, 0.0),
						egui::Layout::top_down(egui::Align::Center),
						|ui| {
							if ui::design::secondary_button(
								ui,
								&ui::i18n::translate(
									"main-restoring-screen-use-a-different-account",
								),
							)
							.clicked()
							{
								if let Some(store) = &mut self.store {
									store.cancel_load();
								}
								self.connection = None;
								self.pending_save = None;
								self.pending_account_save = None;
								self.state.auth = AuthState::Unauthenticated;
								self.state.status = "Disconnected";
								self.credential_status = "";
							}
						},
					);
					// Sizes plugin images so the button keeps clear padding below it.
					self.start_screen.end(ui);
				});
			});
	}
	pub(super) fn traffic_light_inset(&self) -> f32 {
		if self.messaging.uses_native_title_bar() {
			0.0
		} else {
			ui::design::TRAFFIC_LIGHT_INSET
		}
	}
	pub(super) fn sign_in_screen(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		egui::CentralPanel::default()
			.frame(egui::Frame::NONE.fill(ui::design::window_palette(ui).canvas))
			.show(ui, |ui| {
				egui::Panel::top("sign-in-header")
					.exact_size(SIGN_IN_HEADER_HEIGHT)
					.show_separator_line(false)
					.frame(egui::Frame::NONE)
					.show(ui, |ui| {
						// Drag first: later widgets win hit testing, so the header buttons stay clickable.
						if !self.messaging.uses_native_title_bar() {
							ui::design::window_drag(ui, ui.max_rect());
						}
						ui.horizontal_centered(|ui| {
							ui.add_space(16.0 + self.traffic_light_inset());
							app_mark(ui, 20.0);
							ui.add_space(8.0);
							ui.label(
								ui::design::semibold(
									ui,
									ui::i18n::translate("main-sign-in-screen-serein"),
									15.0,
								)
								.color(p.text_strong),
							);
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									if (ui::design::WINDOW_CONTROLS_WIDTH > 0.0
										&& !self.messaging.uses_native_title_bar())
										|| self.messaging.uses_custom_title_bar()
									{
										ui::design::window_controls(ui);
									} else {
										ui.add_space(24.0);
									}
									ui.add_space(8.0);
									let updates = &self.messaging.updates;
									let (hint, update) = if updates.ready {
										("updates-shows-update-banner-restart-to-update", true)
									} else if updates.available {
										("updates-shows-update-banner-update-available", true)
									} else {
										("native-login-settings", false)
									};
									if settings_button(ui, &ui::i18n::translate(hint), update)
										.clicked()
									{
										self.messaging.open_sign_in_settings();
									}
								},
							);
						});
					});
				// The independence note stays on the window's floor, apart from the form.
				egui::Panel::bottom("sign-in-footer")
					.exact_size(48.0)
					.show_separator_line(false)
					.frame(egui::Frame::NONE)
					.show(ui, |ui| {
						ui.vertical_centered(|ui| {
							ui.add_space(12.0);
							ui.label(
								egui::RichText::new(ui::i18n::translate(
									"main-sign-in-screen-independent-and-open-source-not-affiliated-with-discord",
								))
								.size(12.0)
								.color(p.muted),
							);
						});
					});
				egui::ScrollArea::vertical()
					.id_salt("sign-in-scroll")
					// The scroll bar belongs on the window's edge, not the form's.
					.auto_shrink(false)
					.show(ui, |ui| {
						let form = if self.sign_in_height > 0.0 {
							self.sign_in_height
						} else {
							480.0
						};
						// A little above the middle, where the eye lands first.
						ui.add_space(((ui.available_height() - form) * 0.42).max(16.0));
						// The QR code sits beside the form when the window has room for both.
						let wide = self.sign_in_shows_form()
							&& self.native.wide() && ui.available_width()
							>= SIGN_IN_WIDE + 32.0;
						let width = if wide {
							SIGN_IN_WIDE
						} else if self.sign_in_shows_form() {
							ui.available_width()
								.min(native_login::FORM_WIDTH + CARD_PADDING * 2.0)
						} else {
							ui.available_width().min(native_login::FORM_WIDTH)
						};
						ui.vertical_centered(|ui| {
							ui.allocate_ui_with_layout(
								egui::vec2(width, 0.0),
								egui::Layout::top_down(egui::Align::Min),
								|ui| {
									self.sign_in_form(ui, wide);
									#[cfg(feature = "demo")]
									if self.fixture_only {
										self.sign_in_preview(ui);
									}
									// Centring needs the height this actually took; it is
									// stable between frames.
									self.sign_in_height = ui.min_rect().height();
								},
							);
							ui.add_space(24.0);
						});
					});
			});
		let ctx = ui.ctx().clone();
		self.messaging.show_sign_in_settings(&ctx, &mut self.state);
	}
	/// Whether the native form is on screen, rather than the saved accounts in front of it.
	fn sign_in_shows_form(&self) -> bool {
		self.messaging.accounts.is_empty() || self.native.open
	}
	/// Sign-in: saved accounts first for returning owners, then the card with Discord's own
	/// login form and QR code. Everything else is a quiet line of links under it.
	fn sign_in_form(&mut self, ui: &mut egui::Ui, wide: bool) {
		let ctx = ui.ctx().clone();
		let p = ui::design::palette(ui);
		let returning = !self.messaging.accounts.is_empty();
		// A new attempt starts clean: the last one's failure no longer applies.
		if self.native.busy
			&& matches!(
				self.state.auth,
				AuthState::Failed | AuthState::Expired | AuthState::Challenged
			) {
			self.state.auth = AuthState::Unauthenticated;
			self.state.status = "Disconnected";
			self.state.failure_detail = None;
		}
		let waiting = self.state.auth == AuthState::Authenticating;
		let idle = !waiting && !self.forgetting;
		if self.sign_in_shows_form() {
			egui::Frame::new()
				.fill(ui::design::window_palette(ui).surface)
				.stroke(egui::Stroke::new(1.0, p.border))
				.corner_radius(16)
				.inner_margin(CARD_PADDING)
				.show(ui, |ui| {
					ui.add_enabled_ui(idle, |ui| self.native_login_form(ui, wide, returning));
				});
			ui.add_space(16.0);
			self.sign_in_credentials_note(ui);
			if returning {
				ui.add_space(4.0);
				let back = ui
					.vertical_centered(|ui| {
						ui::design::text_action(ui, "native-login-back-to-saved-accounts")
					})
					.inner;
				if back.clicked() {
					self.native.open = false;
					self.native.close();
				}
			}
		} else {
			self.sign_in_header(ui);
			ui.add_space(32.0);
			self.sign_in_accounts(ui, idle);
			ui.add_space(12.0);
			// Fixture builds render the enabled state for captures.
			let button = ui
				.add_enabled_ui(idle, |ui| {
					ui::design::secondary_icon_button(
						ui,
						ui::icons::Icon::Plus,
						"Use another account",
					)
				})
				.inner;
			if button.clicked() {
				if let Some(store) = &mut self.store {
					store.cancel_load();
				}
				self.native.open = true;
			}
		}
		self.sign_in_status(ui);
		ui.add_space(16.0);
		self.sign_in_more(ui, &ctx);
	}
	/// Discord's hosted login page in a temporary webview, for a sign-in the native form
	/// cannot finish (a security key, or a step this client does not know).
	fn open_web_login(&mut self, ctx: &egui::Context) {
		if let Some(store) = &mut self.store {
			store.cancel_load();
		}
		self.native.close();
		self.credential_status = "";
		let wake = ctx.clone();
		match platform::LoginView::open(self.window.clone(), move || wake.request_repaint()) {
			Ok(login) => {
				self.login = Some(login);
				self.state.auth = AuthState::Authenticating;
				self.state.status = "Waiting for Discord login";
			}
			Err(error) => {
				self.state.auth = AuthState::Failed;
				self.state.status = error.label();
			}
		}
	}
	/// App mark over the welcome for an owner this device already knows.
	fn sign_in_header(&self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		ui.vertical_centered(|ui| {
			app_mark(ui, 48.0);
			ui.add_space(20.0);
			ui.label(
				ui::design::semibold(
					ui,
					ui::i18n::translate("main-sign-in-header-welcome-back"),
					24.0,
				)
				.color(p.text_strong),
			);
			ui.add_space(8.0);
			ui.add(
				egui::Label::new(
					egui::RichText::new(ui::i18n::translate(
						"main-sign-in-header-continue-with-a-saved-account-or-sign-in-with-another",
					))
					.size(15.0)
					.color(p.muted),
				)
				.wrap(),
			);
		});
	}
	/// Accounts already signed in on this device: one tap restores their saved login.
	fn sign_in_accounts(&mut self, ui: &mut egui::Ui, enabled: bool) {
		let p = ui::design::palette(ui);
		ui.label(
			ui::design::medium(
				ui,
				ui::i18n::translate("main-sign-in-accounts-saved-accounts"),
				13.0,
			)
			.color(p.muted),
		);
		ui.add_space(8.0);
		let saved: Vec<(model::Id, String, String)> = self
			.messaging
			.accounts
			.iter()
			.map(|account| {
				(
					account.id,
					account.label().to_owned(),
					account
						.handle()
						.map(|handle| format!("@{handle}"))
						.unwrap_or_default(),
				)
			})
			.collect();
		// Four rows fit without crowding the card; the rest scroll inside the list.
		let rows = saved.len().min(4) as f32;
		let height = rows * 54.0 + (rows - 1.0) * 8.0;
		let mut chosen = None;
		let mut forget = None;
		egui::ScrollArea::vertical()
			.id_salt("saved-accounts")
			.max_height(height)
			.min_scrolled_height(height)
			.show(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 8.0;
				for (id, label, handle) in saved {
					let (row, remove) = ui
						.add_enabled_ui(enabled, |ui| {
							ui::design::account_row_with_remove(ui, &label, &handle, true)
						})
						.inner;
					if remove.is_some_and(|remove| remove.clicked()) {
						forget = Some(id);
					} else if row.clicked() {
						chosen = Some(id);
					}
				}
			});
		if let Some(id) = forget {
			self.confirming_forget = Some(id);
		} else if let Some(id) = chosen {
			self.messaging.switch_account_requested = Some(id);
		}
	}
	/// Forgetting is destructive (token, cached history, drafts), so it always asks first.
	pub(super) fn confirm_forget_dialog(&mut self, ctx: &egui::Context) {
		let Some(account) = self.confirming_forget else {
			return;
		};
		let label = self
			.messaging
			.accounts
			.iter()
			.find(|saved| saved.id == account)
			.map(|saved| saved.label().to_owned())
			.unwrap_or_else(|| "this account".to_owned());
		let signed_in = self
			.state
			.user
			.as_ref()
			.is_some_and(|user| user.id == account);
		let confirm = ui::dialog::Confirm::new(
			"forget-account",
			format!("Forget {label}?"),
			if signed_in {
				"This is the account you are signed in with: you will be logged out, and its saved login, cached history and drafts on this device are removed."
			} else {
				"Its saved login, cached history and drafts on this device are removed. The Discord account itself is untouched; you can sign in again any time."
			},
		)
		.danger()
		.confirm_label("Forget account")
		.cancel_label("Keep");
		match confirm.show(ctx) {
			Some(ui::dialog::Choice::Confirmed) => {
				self.confirming_forget = None;
				self.forget_saved_account(ctx, account);
			}
			Some(ui::dialog::Choice::Cancelled) => self.confirming_forget = None,
			None => {}
		}
	}
	/// Where the password goes, beside a lock, centred under the card.
	fn sign_in_credentials_note(&self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		let text = ui::i18n::translate("native-login-credentials-note");
		let galley =
			ui.painter()
				.layout_no_wrap(text.clone(), egui::FontId::proportional(12.0), p.muted);
		let width = 14.0 + 6.0 + galley.size().x;
		if width > ui.available_width() {
			ui.vertical_centered(|ui| {
				ui.add(
					egui::Label::new(egui::RichText::new(text).size(12.0).color(p.muted)).wrap(),
				);
			});
			return;
		}
		let (row, _) = ui.allocate_exact_size(
			egui::vec2(ui.available_width(), galley.size().y.max(14.0)),
			egui::Sense::hover(),
		);
		let left = row.center().x - width * 0.5;
		ui::icons::paint(
			ui.painter(),
			ui::icons::Icon::Lock,
			egui::Rect::from_center_size(
				egui::pos2(left + 7.0, row.center().y),
				egui::Vec2::splat(14.0),
			),
			p.muted,
		);
		ui.painter().galley(
			egui::pos2(left + 20.0, row.center().y - galley.size().y * 0.5),
			galley,
			p.muted,
		);
	}
	/// One banner, only while something is happening or went wrong.
	/// The credential message to show now, if it is still fresh. Saved-login messages describe one
	/// event (a lookup that failed, an account removed), so they stay for a few seconds.
	fn sign_in_notice(&mut self, ui: &egui::Ui) -> Option<&'static str> {
		let status = self.credential_status;
		if self.fixture_only || status.is_empty() || status == "Checking saved login…" {
			self.sign_in_notice = None;
			return None;
		}
		let now = ui.input(|input| input.time);
		let (text, since) = match self.sign_in_notice {
			Some((text, since)) if text == status => (text, since),
			_ => (status, now),
		};
		self.sign_in_notice = Some((text, since));
		let left = NOTICE_SECONDS - (now - since);
		if left <= 0.0 {
			return None;
		}
		ui.ctx()
			.request_repaint_after(std::time::Duration::from_secs_f64(left));
		Some(text)
	}
	/// One banner, only for a failure, work in progress, or a fresh saved-login message.
	fn sign_in_status(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		let attention = matches!(
			self.state.auth,
			AuthState::Failed | AuthState::Expired | AuthState::Challenged
		);
		let connecting = self.state.auth == AuthState::Authenticating;
		let busy =
			connecting || self.forgetting || self.cache_pending > 0 || self.cache_clears.pending();
		let notice = self.sign_in_notice(ui);
		if !(attention || busy || notice.is_some()) {
			return;
		}
		// What the owner is waiting on or what went wrong comes first; a saved-login line follows
		// unless it only repeats it.
		let headline = (attention || connecting)
			.then_some(self.state.status)
			.filter(|status| *status != "Disconnected");
		let detail = notice.or_else(|| {
			(busy || attention)
				.then_some(self.credential_status)
				.filter(|status| !status.is_empty() && *status != "Checking saved login…")
		});
		ui.add_space(16.0);
		let (fill, stroke, color) = if attention {
			(
				p.warning.gamma_multiply(0.13),
				p.warning.gamma_multiply(0.45),
				p.warning,
			)
		} else {
			(p.raised, p.border, p.text)
		};
		egui::Frame::NONE
			.fill(fill)
			.stroke(egui::Stroke::new(1.0, stroke))
			.corner_radius(10)
			.inner_margin(egui::Margin::symmetric(12, 10))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal_top(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					let (icon, _) =
						ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
					if attention {
						ui::icons::paint(
							ui.painter(),
							ui::icons::Icon::ShieldWarning,
							icon,
							p.warning,
						);
					} else if busy {
						ui.put(icon, egui::Spinner::new().size(14.0).color(p.muted));
					}
					ui.vertical(|ui| {
						ui.spacing_mut().item_spacing.y = 4.0;
						if let Some(headline) = headline {
							ui.add(
								egui::Label::new(
									egui::RichText::new(headline).size(13.0).color(color),
								)
								.wrap(),
							);
						}
						if attention {
							self.sign_in_failure_details(ui, p.muted);
						}
						if let Some(detail) = detail.filter(|detail| Some(*detail) != headline) {
							let (size, tint) = if headline.is_some() {
								(12.0, p.muted)
							} else {
								(13.0, color)
							};
							ui.add(
								egui::Label::new(
									egui::RichText::new(detail).size(size).color(tint),
								)
								.wrap(),
							);
						}
						if self.cache_error || self.cache_pending > 0 || self.cache_clears.pending()
						{
							ui.add(
								egui::Label::new(
									egui::RichText::new(self.cache_status)
										.size(12.0)
										.color(p.muted),
								)
								.wrap(),
							);
						}
					});
				});
			});
	}
	/// Redacted decode cause plus a copyable report users can attach to a bug report.
	fn sign_in_failure_details(&mut self, ui: &mut egui::Ui, muted: egui::Color32) {
		if let Some(detail) = &self.state.failure_detail {
			ui.add(
				egui::Label::new(egui::RichText::new(&**detail).size(12.0).color(muted))
					.wrap()
					.selectable(true),
			);
		}
		ui.add_space(4.0);
		let now = ui.input(|i| i.time);
		let copied = self.sign_in_copied.is_some_and(|until| now < until);
		if ui::design::button(
			ui,
			if copied {
				"updates-update-settings-copied"
			} else {
				"main-sign-in-status-copy-failure-details"
			},
			ui::design::ButtonKind::Outline,
		)
		.clicked()
		{
			let report = format!(
				"### Sign-in failure\n- **Reason:** {}\n- **Cause:** {}\n{}",
				self.state.status,
				self.state
					.failure_detail
					.as_deref()
					.unwrap_or("No decode detail recorded"),
				self.messaging.diagnostic_info(ui.ctx())
			);
			ui.ctx().copy_text(report);
			self.sign_in_copied = Some(now + 2.5);
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_secs(3));
		}
	}
	/// Fixture-only entry into the offline preview, kept visually secondary to signing in.
	#[cfg(feature = "demo")]
	fn sign_in_preview(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		ui.add_space(24.0);
		native_login::or_divider(ui);
		ui.add_space(16.0);
		if ui::design::secondary_button(
			ui,
			&ui::i18n::translate("main-sign-in-preview-explore-the-offline-preview"),
		)
		.clicked()
		{
			if let Some(store) = &mut self.store {
				store.cancel_load();
			}
			self.connection = None;
			self.pending_save = None;
			self.pending_account_save = None;
			let generation = self.state.generation + 1;
			self.state = test_support::demo_state();
			test_support::seed_demo_folder_mosaic(&mut self.state);
			test_support::seed_access_marks(&mut self.state);
			self.state.generation = generation;
			self.messaging.clear();
			self.messaging.show_hidden_channels = true;
		}
		ui.add_space(8.0);
		ui.vertical_centered(|ui| {
			ui.label(
				egui::RichText::new(ui::i18n::translate(
					"main-sign-in-preview-sample-conversations-no-discord-connection",
				))
				.size(12.0)
				.color(p.muted),
			);
		});
	}
	/// The rest, one quiet line of links: what this client is, the owner's own session
	/// token, and Discord's hosted login page. Each opens a panel under the line.
	fn sign_in_more(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
		let p = ui::design::palette(ui);
		let labels = [
			"main-sign-in-disclosures-about-serein",
			"native-login-more-token",
			"native-login-more-web",
		]
		.map(ui::i18n::translate);
		// Centred by measuring: a row lays out from the left.
		let font = egui::FontId::new(13.0, ui::design::medium_family(ui.ctx()));
		let measure = |text: &str| {
			ui.painter()
				.layout_no_wrap(text.to_owned(), font.clone(), p.muted)
				.size()
				.x
		};
		let dot = measure("·");
		let width = labels
			.iter()
			.map(|label| measure(label) + 12.0)
			.sum::<f32>()
			+ dot * 2.0;
		let mut clicked = None;
		ui.horizontal(|ui| {
			ui.spacing_mut().item_spacing.x = 0.0;
			ui.add_space(((ui.available_width() - width) * 0.5).max(0.0));
			for (index, label) in labels.iter().enumerate() {
				if index > 0 {
					ui.label(ui::design::medium(ui, "·", 13.0).color(p.muted));
				}
				if ui::design::text_action(ui, label).clicked() {
					clicked = Some(index);
				}
			}
		});
		// One panel at a time; the same link closes it again.
		if let Some(index) = clicked {
			let open = [self.about_open, self.token_open, self.native.web_open][index];
			self.about_open = index == 0 && !open;
			self.token_open = index == 1 && !open;
			self.native.web_open = index == 2 && !open;
		}
		let panel = || {
			egui::Frame::new()
				.fill(p.raised)
				.stroke(egui::Stroke::new(1.0, p.border))
				.corner_radius(12)
				.inner_margin(16)
		};
		let note = |ui: &mut egui::Ui, text: &str| {
			ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(p.muted)).wrap());
		};
		if self.about_open {
			ui.add_space(8.0);
			panel().show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 6.0;
				for line in ["native-login-about-risk", "native-login-about-storage"] {
					note(ui, &ui::i18n::translate(line));
				}
				if !self.fixture_only {
					ui.add_space(2.0);
					if ui::design::button(
						ui,
						&ui::i18n::translate("main-sign-in-disclosures-forget-saved-login"),
						ui::design::ButtonKind::Outline,
					)
					.clicked()
					{
						self.logout(ctx);
					}
				}
			});
		}
		if self.token_open {
			ui.add_space(8.0);
			panel().show(ui, |ui| {
				ui.set_width(ui.available_width());
				note(ui, &ui::i18n::translate("native-login-token-detail"));
				ui.add_space(8.0);
				ui::design::input(
					ui,
					egui::TextEdit::singleline(&mut *self.token_input)
						.password(true)
						.char_limit(2048)
						.hint_text(ui::i18n::translate(
							"main-sign-in-disclosures-session-token",
						)),
				);
				ui.add_space(8.0);
				let connect = ui
					.add_enabled_ui(!self.token_input.is_empty(), |ui| {
						ui::design::button(
							ui,
							&ui::i18n::translate(
								"main-sign-in-disclosures-connect-with-this-token",
							),
							ui::design::ButtonKind::Primary,
						)
					})
					.inner;
				if connect.clicked() && !self.fixture_only {
					let input = std::mem::take(&mut *self.token_input);
					match SessionSecret::from_owner_input(input) {
						Ok(secret) => self.connect(secret, true, ctx),
						Err(failure) => self.state.status = failure.label(),
					}
				}
			});
		}
		if self.native.web_open {
			ui.add_space(8.0);
			panel().show(ui, |ui| {
				ui.set_width(ui.available_width());
				note(ui, &ui::i18n::translate("native-login-web-detail"));
				ui.add_space(8.0);
				let idle = self.state.auth != AuthState::Authenticating && !self.forgetting;
				let open = ui
					.add_enabled_ui(idle, |ui| {
						ui::design::button(
							ui,
							&ui::i18n::translate("native-login-web-open"),
							ui::design::ButtonKind::Outline,
						)
					})
					.inner;
				if open.clicked() && !self.fixture_only {
					self.open_web_login(ctx);
				}
			});
		}
	}
}

/// The header's one control: a quiet gear that opens settings, with a dot while an update waits.
fn settings_button(ui: &mut egui::Ui, hint: &str, update: bool) -> egui::Response {
	let p = ui::design::palette(ui);
	let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(32.0), egui::Sense::click());
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), hint));
	let hot = response.hovered() || response.has_focus();
	if hot {
		ui.painter().rect_filled(rect, 8, p.hover);
		ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
	}
	ui::icons::paint(
		ui.painter(),
		ui::icons::Icon::Gear,
		egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(18.0)),
		if hot { p.text_strong } else { p.muted },
	);
	if update {
		let dot = rect.right_top() + egui::vec2(-7.0, 7.0);
		ui.painter()
			.circle_filled(dot, 5.5, ui::design::window_palette(ui).canvas);
		ui.painter().circle_filled(dot, 3.5, p.link);
	}
	response.on_hover_text(hint)
}

/// The Serein mark on a solid accent tile; one shape at every size, no halo.
fn app_mark(ui: &mut egui::Ui, size: f32) {
	let p = ui::design::palette(ui);
	let (mark, _) = ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
	ui.painter()
		.rect_filled(mark, (size * 0.25).round() as u8, p.accent);
	ui::icons::paint(
		ui.painter(),
		ui::icons::Icon::Serein,
		mark.shrink(size * 0.24),
		p.accent_text,
	);
}
