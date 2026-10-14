//! Sign-in UI and account-session lifecycle for the desktop client.
use super::*;

/// Sign-in header strip doubles as the window drag region, clearing the traffic lights.
pub(super) const SIGN_IN_HEADER_HEIGHT: f32 = if cfg!(target_os = "windows") {
	44.0
} else {
	60.0
};

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
		self.credential_status = if save {
			"Login will be saved after Discord connects"
		} else {
			"Connecting with the supplied session; saved login unchanged"
		};
		self.voice.stop();
		self.messaging.camera_test_requested = false;
		self.messaging.camera_test_texture = None;
		self.state
			.disconnect_voice("Discord login session changed; start a new call");
		self.uploads.cancel();
		self.login = None;
		self.state.interrupt_gif_favorites();
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
		if let Some(store) = &mut self.store {
			store.cancel_load();
		}
		self.downloads.cancel();
		self.audio.stop();
		self.video.stop();
		self.video.cancel_posters();
		self.voice.stop();
		self.messaging.camera_test_requested = false;
		self.messaging.camera_test_texture = None;
		let was_demo = self.state.demo;
		self.clear_avatars(ctx);
		self.login = None;
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
				"Credential queue unavailable; saved login may remain"
			};
		} else if !was_demo {
			self.credential_status = "Signed out of this account; its saved login is kept";
		}
		self.confirming_logout = false;
		self.end_intent = SessionEnd::Logout;
	}
	/// Confirms first when work would be lost, then ends the session for `intent`.
	pub(super) fn request_session_end(&mut self, ctx: &egui::Context, intent: SessionEnd) {
		if self.state.has_unsent()
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
			"Credential lookup unavailable; sign in with Discord again"
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
		let display = self
			.state
			.own_profile
			.data
			.as_ref()
			.and_then(|profile| profile.global_name.as_deref());
		// Re-record once per session so the switcher orders by last use, then only on change.
		let recorded = self.roster_generation == Some(self.state.generation);
		if recorded
			&& self.messaging.accounts.iter().any(|saved| {
				saved.id == user.id
					&& saved.name == user.name
					&& saved.avatar == user.avatar
					&& saved.discriminator == user.discriminator
					&& saved.display.as_deref() == display
			}) {
			return;
		}
		let account = model::SavedAccount {
			id: user.id,
			name: user.name.clone(),
			display: display.map(str::to_owned),
			avatar: user.avatar.clone(),
			discriminator: user.discriminator,
			has_token: false,
		};
		if !account.is_valid() {
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
		egui::CentralPanel::default()
			.frame(egui::Frame::NONE.fill(ui::design::window_palette(ui).canvas))
			.show(ui, |ui| {
				accent_glow(ui);
				ui::design::window_drag(ui, ui.max_rect());
				egui::Frame::NONE
					.inner_margin(egui::Margin {
						left: (16.0 + self.traffic_light_inset()) as i8,
						right: if ui::design::WINDOW_CONTROLS_WIDTH > 0.0 {
							0
						} else {
							16
						},
						top: if ui::design::WINDOW_CONTROLS_WIDTH > 0.0 {
							0
						} else {
							16
						},
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
									if !self.messaging.hide_title_bar {
										ui::design::window_controls(ui);
									}
								},
							);
						});
					});
				let time = ui.input(|i| i.time) as f32;
				ui.ctx().request_repaint();
				ui.vertical_centered(|ui| {
					ui.add_space((ui.available_height() * 0.5 - 150.0).max(12.0));
					// App mark with a slow breathing halo; the only motion besides the bar.
					let (rect, _) =
						ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::hover());
					let mark = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(56.0));
					let pulse = 0.5 + 0.5 * (time * 1.6).sin();
					ui.painter().rect_filled(
						mark.expand(4.0 + 4.0 * pulse),
						18,
						p.accent.gamma_multiply(0.10 + 0.10 * pulse),
					);
					ui.painter().rect_filled(mark, 14, p.accent);
					ui::icons::paint(
						ui.painter(),
						ui::icons::Icon::Serein,
						mark.shrink(13.0),
						p.accent_text,
					);
					ui.add_space(18.0);
					ui.label(
						ui::design::semibold(
							ui,
							ui::i18n::translate("main-restoring-screen-welcome-back"),
							24.0,
						)
						.color(p.text_strong),
					);
					ui.add_space(6.0);
					ui.label(
						egui::RichText::new(format!("{stage}…"))
							.size(15.0)
							.color(p.muted),
					);
					ui.add_space(22.0);
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
					ui.add_space(26.0);
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
								self.credential_status = "Saved-login restore cancelled";
							}
						},
					);
				});
			});
	}
	pub(super) fn traffic_light_inset(&self) -> f32 {
		if self.messaging.hide_title_bar {
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
				accent_glow(ui);
				egui::Panel::top("sign-in-header")
					.exact_size(SIGN_IN_HEADER_HEIGHT)
					.show_separator_line(false)
					.frame(egui::Frame::NONE)
					.show(ui, |ui| {
						// Drag first: later widgets win hit testing, so the header buttons stay clickable.
						ui::design::window_drag(ui, ui.max_rect());
						ui.horizontal_centered(|ui| {
							ui.add_space(16.0 + self.traffic_light_inset());
							// Wordmark lockup: app mark, name, then a quiet outlined stage pill.
							let (mark, _) = ui
								.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
							ui.painter().rect_filled(mark, 6, p.accent);
							ui::icons::paint(
								ui.painter(),
								ui::icons::Icon::Serein,
								mark.shrink(5.0),
								p.accent_text,
							);
							ui.add_space(8.0);
							ui.label(
								ui::design::semibold(
									ui,
									ui::i18n::translate("main-sign-in-screen-serein"),
									16.0,
								)
								.color(p.text_strong),
							);
							ui.add_space(8.0);
							// Painted rather than framed: the pill must hug the text, not the row height.
							let stage = ui.painter().layout_no_wrap(
								"Early preview".to_owned(),
								egui::FontId::new(10.5, ui::design::medium_family(ui.ctx())),
								p.muted,
							);
							let (pill, _) = ui.allocate_exact_size(
								egui::vec2(stage.size().x + 16.0, 19.0),
								egui::Sense::hover(),
							);
							ui.painter().rect_stroke(
								pill,
								9,
								egui::Stroke::new(1.0, p.border),
								egui::StrokeKind::Inside,
							);
							ui.painter()
								.galley(pill.center() - stage.size() * 0.5, stage, p.muted);
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									if ui::design::WINDOW_CONTROLS_WIDTH > 0.0
										&& !self.messaging.hide_title_bar
									{
										ui::design::window_controls(ui);
									} else {
										ui.add_space(24.0);
									}
									ui.add_space(12.0);
									// These popups hold settings controls, so a click inside
									// must not dismiss them the way a menu command would.
									let sticky = || {
										egui::containers::menu::MenuConfig::new().close_behavior(
											egui::PopupCloseBehavior::CloseOnClickOutside,
										)
									};
									// Quiet header actions: text only until hovered, like the rest of the chrome.
									let quiet =
										|ui: &egui::Ui, text: &str, color: egui::Color32| {
											egui::Button::new(
												ui::design::medium(ui, text, 13.0).color(color),
											)
											.frame_when_inactive(false)
											.corner_radius(8)
										};
									egui::containers::menu::MenuButton::from_button(quiet(
										ui,
										&ui::i18n::translate("page-appearance"),
										p.muted,
									))
									.config(sticky())
									.ui(ui, |ui| self.messaging.appearance_menu(ui));
									ui.add_space(8.0);
									let updates = &self.messaging.updates;
									let (label, color) = if updates.ready {
										("updates-shows-update-banner-restart-to-update", p.link)
									} else if updates.busy {
										("updates-shows-update-banner-updating", p.muted)
									} else if updates.available {
										("updates-shows-update-banner-update-available", p.link)
									} else {
										("page-updates", p.muted)
									};
									let label = ui::i18n::translate(label);
									let demo = self.fixture_only || self.state.demo;
									egui::containers::menu::MenuButton::from_button(quiet(
										ui, &label, color,
									))
									.config(sticky())
									.ui(ui, |ui| self.messaging.updates_menu(ui, demo));
								},
							);
						});
					});
				egui::ScrollArea::vertical()
					.id_salt("sign-in-scroll")
					.show(ui, |ui| {
						let card = if self.sign_in_height > 0.0 {
							self.sign_in_height + 72.0
						} else {
							560.0
						};
						ui.add_space(((ui.available_height() - card) * 0.4).max(16.0));
						ui.vertical_centered(|ui| {
							ui.allocate_ui_with_layout(
								egui::vec2(ui.available_width().min(460.0), 0.0),
								egui::Layout::top_down(egui::Align::Min),
								|ui| {
									self.sign_in_card(ui);
									#[cfg(feature = "demo")]
									if self.fixture_only {
										self.sign_in_preview(ui);
									}
									// Centring needs the height this actually took; it is
									// stable between frames.
									self.sign_in_height = ui.min_rect().height();
								},
							);
							ui.add_space(18.0);
							ui.label(
								egui::RichText::new(ui::i18n::translate(
									"main-sign-in-screen-independent-and-open-source-not-affiliated-with-discord",
								))
								.size(12.0)
								.color(p.muted),
							);
							ui.add_space(24.0);
						});
					});
			});
	}
	/// Sign-in card: saved accounts first for returning owners, one clear primary action,
	/// explicit consent, and everything else folded away until asked for.
	fn sign_in_card(&mut self, ui: &mut egui::Ui) {
		let ctx = ui.ctx().clone();
		let p = ui::design::palette(ui);
		let shadow = egui::epaint::Shadow {
			offset: [0, 16],
			blur: 40,
			spread: 0,
			color: egui::Color32::from_black_alpha(if ui.visuals().dark_mode { 110 } else { 34 }),
		};
		egui::Frame::NONE
			.fill(p.surface)
			.stroke(egui::Stroke::new(1.0, p.border))
			.shadow(shadow)
			.corner_radius(16)
			.inner_margin(egui::Margin {
				left: 28,
				right: 28,
				top: 24,
				bottom: 14,
			})
			.show(ui, |ui| {
				let returning = !self.messaging.accounts.is_empty();
				let waiting = self.state.auth == AuthState::Authenticating;
				let idle = !waiting && !self.forgetting;
				// A new sign-in needs explicit authorization; a saved account was already
				// authorized once and restores unattended on launch, so it only waits for idle.
				// Fixture builds render the enabled state for captures; the actions stay inert.
				let ready = self.authorized && idle;
				self.sign_in_header(ui, returning);
				ui.add_space(16.0);
				if returning {
					self.sign_in_accounts(ui, idle);
					ui.add_space(10.0);
				}
				let label = if waiting {
					"Waiting for Discord…"
				} else if returning {
					"Use another account"
				} else {
					"Continue with Discord"
				};
				let button = ui
					.add_enabled_ui(ready, |ui| {
						if returning {
							ui::design::secondary_icon_button(ui, ui::icons::Icon::Plus, label)
						} else {
							ui::design::primary_icon_button(ui, ui::icons::Icon::Serein, label)
						}
					})
					.inner;
				if button.clicked() && !self.fixture_only {
					if let Some(store) = &mut self.store {
						store.cancel_load();
					}
					self.credential_status = "Sign in through Discord; saved-login lookup stopped";
					let wake = ctx.clone();
					match platform::LoginView::open(self.window.clone(), move || {
						wake.request_repaint()
					}) {
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
				ui.add_space(10.0);
				self.sign_in_consent(ui);
				self.sign_in_status(ui);
				ui.add_space(12.0);
				let (line, _) = ui.allocate_exact_size(
					egui::vec2(ui.available_width(), 1.0),
					egui::Sense::hover(),
				);
				ui.painter().rect_filled(line, 0, p.border);
				ui.add_space(8.0);
				self.sign_in_disclosures(ui, &ctx);
			});
	}
	/// App mark over a welcome line that adapts to whether this device already knows an account.
	fn sign_in_header(&self, ui: &mut egui::Ui, returning: bool) {
		let p = ui::design::palette(ui);
		ui.vertical_centered(|ui| {
			let (mark, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
			ui.painter()
				.rect_filled(mark.expand(6.0), 17, p.accent.gamma_multiply(0.16));
			ui.painter().rect_filled(mark, 13, p.accent);
			ui::icons::paint(
				ui.painter(),
				ui::icons::Icon::Serein,
				mark.shrink(11.0),
				p.accent_text,
			);
			ui.add_space(12.0);
			ui.label(
				ui::design::semibold(
					ui,
					ui::i18n::translate_if_key(if returning {
						"main-sign-in-header-welcome-back"
					} else {
						"main-sign-in-header-welcome-to-serein"
					}),
					22.0,
				)
				.color(p.text_strong),
			);
			ui.add_space(5.0);
			ui.add(
				egui::Label::new(
					egui::RichText::new(ui::i18n::translate_if_key(if returning {
						"main-sign-in-header-continue-with-a-saved-account-or-sign-in-with-another"
					} else {
						"main-sign-in-header-sign-in-with-your-discord-account-to-get-started"
					}))
					.size(14.0)
					.color(p.muted),
				)
				.wrap(),
			);
		});
	}
	/// Accounts already signed in on this device: one tap restores their saved login.
	fn sign_in_accounts(&mut self, ui: &mut egui::Ui, enabled: bool) {
		let p = ui::design::palette(ui);
		ui.label(ui::design::eyebrow(
			ui,
			ui::i18n::translate("main-sign-in-accounts-saved-accounts"),
			p.muted,
		));
		ui.add_space(6.0);
		let saved: Vec<(model::Id, String, String)> = self
			.messaging
			.accounts
			.iter()
			.map(|account| {
				(
					account.id,
					account.label().to_owned(),
					format!("@{}", account.name),
				)
			})
			.collect();
		// Four rows fit without crowding the card; the rest scroll inside the list.
		let rows = saved.len().min(4) as f32;
		let height = rows * 54.0 + (rows - 1.0) * 6.0;
		let mut chosen = None;
		let mut forget = None;
		egui::ScrollArea::vertical()
			.id_salt("saved-accounts")
			.max_height(height)
			.min_scrolled_height(height)
			.show(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 6.0;
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
	/// Owner authorization, with the token handling spelled out next to the checkbox.
	fn sign_in_consent(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		egui::Frame::NONE
			.fill(p.base)
			.corner_radius(10)
			.inner_margin(egui::Margin::symmetric(12, 9))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.checkbox(
					&mut self.authorized,
					ui::design::medium(
						ui,
						ui::i18n::translate(
							"main-sign-in-consent-i-own-this-account-and-authorize-this-session",
						),
						13.0,
					)
					.color(p.text_strong),
				);
				ui.add_space(4.0);
				ui.add(
					egui::Label::new(
						egui::RichText::new(ui::i18n::translate(
							"main-sign-in-consent-passwords-and-2fa-stay-on-discord-s-own-login-page",
						))
						.size(12.0)
						.color(p.muted),
					)
					.wrap(),
				);
			});
	}
	/// One banner, only while something is happening or went wrong.
	fn sign_in_status(&mut self, ui: &mut egui::Ui) {
		let p = ui::design::palette(ui);
		let attention = matches!(
			self.state.auth,
			AuthState::Failed | AuthState::Expired | AuthState::Challenged
		);
		let busy = self.state.auth == AuthState::Authenticating
			|| self.forgetting
			|| self.cache_pending > 0
			|| self.cache_clears.pending();
		let show = attention
			|| busy || self.state.status != "Disconnected"
			|| (!self.fixture_only
				&& self.credential_status != "Checking saved login…"
				&& !self.credential_status.is_empty());
		if !show || (self.fixture_only && self.state.status == "Disconnected") {
			return;
		}
		ui.add_space(14.0);
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
					ui.spacing_mut().item_spacing.x = 9.0;
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
						ui.spacing_mut().item_spacing.y = 3.0;
						if self.state.status != "Disconnected" || attention {
							ui.add(
								egui::Label::new(
									egui::RichText::new(self.state.status)
										.size(13.0)
										.color(color),
								)
								.wrap(),
							);
						}
						if attention {
							self.sign_in_failure_details(ui, p.muted);
						}
						if !self.fixture_only && !self.credential_status.is_empty() {
							ui.add(
								egui::Label::new(
									egui::RichText::new(self.credential_status)
										.size(12.0)
										.color(p.muted),
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
		ui.add_space(18.0);
		ui.horizontal(|ui| {
			let y = ui.cursor().top() + 8.0;
			let left = ui.cursor().left();
			let width = ui.available_width();
			let galley =
				ui.painter()
					.layout_no_wrap("or".into(), egui::FontId::proportional(12.0), p.muted);
			let text_w = galley.size().x + 20.0;
			ui.painter().hline(
				left..=left + (width - text_w) * 0.5,
				y,
				egui::Stroke::new(1.0, p.border),
			);
			ui.painter().galley(
				egui::pos2(
					left + (width - galley.size().x) * 0.5,
					y - galley.size().y * 0.5,
				),
				galley,
				p.muted,
			);
			ui.painter().hline(
				left + (width + text_w) * 0.5..=left + width,
				y,
				egui::Stroke::new(1.0, p.border),
			);
			ui.allocate_space(egui::vec2(width, 16.0));
		});
		ui.add_space(14.0);
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
				egui::RichText::new(&ui::i18n::translate(
					"main-sign-in-preview-sample-conversations-no-discord-connection",
				))
				.size(12.0)
				.color(p.muted),
			);
		});
	}
	/// Secondary panels: what this client is, and the owner's own session token.
	fn sign_in_disclosures(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
		let p = ui::design::palette(ui);
		if ui::design::disclosure(
			ui,
			&ui::i18n::translate("main-sign-in-disclosures-about-serein"),
			self.about_open,
		)
		.clicked()
		{
			self.about_open = !self.about_open;
		}
		if self.about_open {
			ui.add_space(2.0);
			egui::Frame::NONE
				.inner_margin(egui::Margin {
					left: 30,
					right: 4,
					top: 2,
					bottom: 8,
				})
				.show(ui, |ui| {
					ui.spacing_mut().item_spacing.y = 6.0;
					for line in [
						"Messaging, reactions, search and read markers have offline tests. Real Discord interoperability is still unverified; attachment uploads and advanced search remain incomplete.",
						"Messages and drafts are cached locally. Login tokens use the operating system credential store.",
						"Unofficial clients may put your Discord account at risk.",
					] {
						ui.add(
							egui::Label::new(
								egui::RichText::new(line).size(12.0).color(p.muted),
							)
							.wrap(),
						);
					}
					if !self.fixture_only {
						ui.add_space(2.0);
						if ui::design::button(ui, &ui::i18n::translate("main-sign-in-disclosures-forget-saved-login"), ui::design::ButtonKind::Outline)
							.clicked()
						{
							self.logout(ctx);
						}
					}
				});
		}
		if ui::design::disclosure(
			ui,
			&ui::i18n::translate("main-sign-in-disclosures-sign-in-with-a-session-token"),
			self.token_open,
		)
		.clicked()
		{
			self.token_open = !self.token_open;
		}
		if self.token_open {
			ui.add_space(2.0);
			egui::Frame::NONE
				.inner_margin(egui::Margin {
					left: 30,
					right: 4,
					top: 2,
					bottom: 6,
				})
				.show(ui, |ui| {
					ui.add(
						egui::Label::new(
							egui::RichText::new(ui::i18n::translate(
								"main-sign-in-disclosures-for-owners-who-already-hold-a-valid-discord-session-token",
							))
							.size(12.0)
							.color(p.muted),
						)
						.wrap(),
					);
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
						.add_enabled_ui(self.authorized && !self.token_input.is_empty(), |ui| {
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
	}
}
