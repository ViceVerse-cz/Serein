//! Coordinate frame work without owning worker or account lifetimes.
#[cfg(target_os = "windows")]
use super::align_undecorated_surface;
use super::{
	AuthState, Command, Delivery, Desktop, Duration, SessionEnd, audio, cache, clipboard,
	downloads, egui, gpu_video, notification_runtime, queue_channel_preferences, recovery_draft,
	sign_in,
};

impl Desktop {
	/// One UI frame: pumps workers, expires challenges, renders and drains commands.
	pub(super) fn frame_logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
		// Last frame's shapes are painted; video textures retired since then can go.
		gpu_video::collect();
		let search_focused = {
			#[cfg(feature = "demo")]
			{
				self.frame_metrics.sample.is_some() && self.messaging.friends_sample_focused(ctx)
			}
			#[cfg(not(feature = "demo"))]
			{
				false
			}
		};
		self.frame_metrics.begin(ctx, search_focused);
		self.sync_timezone(ctx);
		self.startup.sync(
			ctx,
			&self.runtime,
			&mut self.messaging,
			self.fixture_only || self.state.demo,
		);
		self.messaging.sync_reading_zoom(ctx);
		self.poll(ctx);
		if let Some(connection) = &self.connection {
			connection.copy_diagnostics_to(&mut self.messaging);
			let focused =
				ctx.input(|input| input.focused && input.viewport().minimized != Some(true));
			connection.observe_foreground(focused, std::time::Instant::now());
		}
		if let Some(connection) = &self.connection {
			connection.sync_admin_reads(&self.state);
		}
		let preference_now = std::time::Instant::now();
		if let Some(command) = self.state.poll_channel_preferences(preference_now) {
			self.command(command);
		}
		if let Some(deadline) = self.state.next_channel_preference_deadline() {
			ctx.request_repaint_after(deadline.saturating_duration_since(preference_now));
		}
		self.sync_message_speech(ctx);
		self.sync_fonts(ctx);
		self.hotkeys.sync(&self.messaging.keybinds, &self.runtime);
		self.messaging.global_keybind_status = self.hotkeys.status();
		let voice_toggles = self.hotkeys.take_toggle_pending()
			| self
				.messaging
				.voice_toggle_pressed(ctx, self.hotkeys.global_toggle_mask());
		if voice_toggles != 0 && !self.fixture_only {
			// Toggle from the newest choice; a mute and deafen in one frame apply in order.
			let mut latest = None;
			if voice_toggles & 1 != 0 {
				latest = Some(self.messaging.toggle_voice(false));
			}
			if voice_toggles & 2 != 0 {
				latest = Some(self.messaging.toggle_voice(true));
			}
			let (muted, deafened) =
				latest.unwrap_or((self.messaging.voice_muted, self.messaging.voice_deafened));
			if self.state.auth == AuthState::Authenticated
				&& let Some(command) = self.state.set_call_mute(muted, deafened)
				&& !self.state.demo
			{
				self.command(command);
			}
		}
		if self.updater.sync(
			ctx,
			&self.runtime,
			&mut self.messaging.updates,
			!cfg!(debug_assertions)
				&& !self.fixture_only
				&& !self.state.demo
				&& (self.app_settings.loaded || self.app_settings.state.touched),
		) {
			// Installing needs a real exit, so this close must not stop at the tray.
			self.tray_window.quit(ctx);
		}
		self.poll_interaction_files(ctx);
		self.state.expire_interaction(std::time::Instant::now());
		if self.state.interactions.busy() {
			ctx.request_repaint_after(Duration::from_millis(250));
		}
		self.state.expire_verification();
		if self.state.verification().is_some() {
			ctx.request_repaint_after(Duration::from_secs(1));
		}
		if self.login.is_some()
			|| self.state.auth != AuthState::Authenticated
			|| ctx.input(|input| input.viewport().close_requested())
		{
			self.captcha.close();
			self.messaging.verification.active = false;
		}
		self.extensions.tick(
			&mut self.state,
			&mut self.messaging,
			ctx,
			&self.runtime,
			&self.window,
			self.fixture_only,
		);
		if self.extensions.api_proxy_ready() {
			let route = self.proxy_auth.tick(
				self.extensions.api_proxy(),
				&mut self.messaging.extensions.proxy_auth,
				&self.runtime,
				ctx,
				self.fixture_only,
			);
			self.api_proxy.send_if_modified(|current| {
				if *current == route {
					false
				} else {
					*current = route;
					true
				}
			});
		}
		self.sync_customization(ctx);
		#[cfg(feature = "demo")]
		if self.demo_typing
			&& let Some(channel) = self.state.selected
		{
			let wall = std::time::SystemTime::now();
			let timestamp = wall
				.duration_since(std::time::UNIX_EPOCH)
				.map(|age| age.as_secs())
				.unwrap_or_default();
			for user in [2, 3] {
				self.state.observe_typing_at(
					client_core::typing::Signal {
						channel,
						user: model::Id(user),
						timestamp,
						member: None,
					},
					wall,
					std::time::Instant::now(),
				);
			}
		}
		let mut tray_events = Vec::with_capacity(4);
		if let Some(tray) = &mut self.tray {
			// The Linux worker can refill event bits while this frame consumes them.
			for _ in 0..4 {
				let Some(event) = tray.take_event() else {
					break;
				};
				tray_events.push(event);
			}
		}
		#[cfg(target_os = "linux")]
		let quitting = tray_events.contains(&platform::tray::Event::Quit);
		for event in tray_events {
			match event {
				platform::tray::Event::Quit => {
					self.tray_window.quit(ctx);
				}
				platform::tray::Event::Show => self.tray_window.show(ctx),
				#[cfg(target_os = "linux")]
				platform::tray::Event::Minimize => {
					if !quitting {
						self.tray_window.minimize(ctx);
					}
				}
				platform::tray::Event::Unavailable => {
					self.tray = None;
					self.tray_error = Some(if cfg!(target_os = "linux") {
						"Tray unavailable. Start a StatusNotifier host, then toggle the tray off/on."
					} else {
						"Tray unavailable. The window will stay visible."
					});
					self.tray_window.show(ctx);
				}
			}
		}
		if self.instance.take_show_request() {
			self.tray_window.show(ctx);
		}
		#[cfg(target_os = "macos")]
		self.app_menu(ctx);
		self.tray_window.logic(
			ctx,
			self.background_available(),
			self.window.is_visible().is_some(),
		);
		// The hide command lands after this frame, so the flag leads reported visibility.
		// Occlusion can occur during macOS fullscreen transitions; it is not a request
		// to stop playback (which would also restore the window out of fullscreen).
		let hidden_or_closing = self.tray_window.hidden
			|| ctx.input(|input| {
				input.viewport().minimized == Some(true) || input.viewport().close_requested()
			});
		if self.state.user.is_none()
			|| (!self.state.demo && self.state.auth != AuthState::Authenticated)
			|| hidden_or_closing
		{
			self.audio.stop();
			self.messaging.audio().stop();
			self.video.stop();
			self.messaging.video().stop();
		}
		self.video.poll(self.messaging.video(), ctx);
		let audio = self.audio.poll();
		self.audio.output.clone_from(&self.messaging.media_output);
		let player = self.messaging.audio();
		player.position = audio.position.as_secs_f64();
		player.duration = audio.duration.as_secs_f64();
		player.state = match audio.state {
			audio::State::Idle => ui::AudioState::Idle,
			audio::State::Loading => ui::AudioState::Loading,
			audio::State::Playing => ui::AudioState::Playing,
			audio::State::Paused => ui::AudioState::Paused,
			audio::State::Ended => ui::AudioState::Ended,
			audio::State::Failed(error) => ui::AudioState::Failed(error),
		};
		if self.state.auth != AuthState::Authenticated && !self.state.demo {
			self.notifications.clear();
		}
		if let Some((channel, message)) = self.notifications.take_message_activation()
			&& self.state.auth == AuthState::Authenticated
		{
			self.tray_window.show(ctx);
			if let Some(command) =
				self.messaging
					.open_notification(&mut self.state, channel, message)
			{
				self.command(command);
			}
		}
		if let Some(alert) = self.notification_runtime.poll(
			&mut self.state,
			&mut self.messaging,
			&self.window,
			ctx,
			self.fixture_only,
		) {
			match alert {
				notification_runtime::Alert::Message {
					channel,
					message,
					title,
					body,
					avatar_key,
					image_path,
				} => {
					if self
						.notifications
						.notify_target(channel, message, title, body, image_path)
						&& let Some(worker) = &self.avatars
					{
						worker.request(avatar_key);
					}
				}
			}
		}
		self.messaging.voice_ptt_active = self.messaging.voice_push_to_talk
			&& (self.state.voice.active.is_some()
				|| self.messaging.voice_messages.scope.is_some()
				|| self.voice_messages.capture_busy())
			&& (self.messaging.push_to_talk_down(ctx) || self.hotkeys.push_to_talk_down());
		self.messaging.voice_ptm_active = (self.state.voice.active.is_some()
			|| self.messaging.voice_messages.scope.is_some()
			|| self.voice_messages.capture_busy())
			&& (self.messaging.push_to_mute_down(ctx) || self.hotkeys.push_to_mute_down());
		self.poll_voice_messages(ctx);
		if self.state.auth == AuthState::Authenticated || self.state.demo {
			self.poll_voice(ctx);
		} else {
			self.voice_messages.cancel();
			self.voice.stop();
		}
		self.sync_tray(ctx);
		if self.state.voice.active.is_some() {
			// Speaking, notices, remote video, devices, hotkeys and deadlines each wake the UI
			// themselves; this heartbeat only bounds a missed wake, so a call does not repaint at 20 Hz.
			ctx.request_repaint_after(std::time::Duration::from_secs(1));
		}
	}
	pub(super) fn show_frame(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
		let ctx = ui.ctx().clone();
		self.messaging.retire_mfa_input(&ctx, &self.state);
		if let Some(connection) = &self.connection {
			connection.sync_admin_reads(&self.state);
		}
		let title_bar_height = if self.login.is_some() {
			platform::LOGIN_HEADER_HEIGHT
		} else if self.state.user.is_some() {
			ui::design::TITLE_BAR_HEIGHT
		} else {
			sign_in::SIGN_IN_HEADER_HEIGHT
		};
		self.tray_window.ui(&ctx);
		// Change native hints before drawing, so the clear color and panel alpha
		// agree for the whole frame. OS calls happen only when an effect changes.
		self.sync_window_effects();
		let (close_requested, dropped) = ctx.input_mut(|input| {
			(
				input.viewport().close_requested(),
				std::mem::take(&mut input.raw.dropped_files),
			)
		});
		ui::design::paint_backdrop(&ctx);
		// A history page loading while browsing must not refuse a paste or drop.
		let upload_allowed = self.state.user.is_some()
			&& self.state.gateway_connected
			&& self.state.freshness != model::Freshness::Unavailable;
		// A forum container carries its own attachment permission for a post's first message.
		let can_attach = self.state.selected.is_some_and(|channel| {
			self.state.can_attach(channel) || self.state.can_attach_post(channel)
		});
		if let Some(paste) = &mut self.clipboard {
			paste.follow(self.state.generation, self.state.selected);
		}
		if let Some(paste) = &self.clipboard
			&& let Some(result) = paste.poll()
		{
			if paste.active() && self.state.user.is_some() {
				let text_limit = if self.state.is_forum(paste.target.channel) {
					client_core::MAX_CONTENT
				} else {
					self.state.content_limit()
				};
				match result {
					Ok(clipboard::Content::Text(_))
						if !self.messaging.paste_target_current(
							&ctx,
							&self.state,
							&paste.target,
						) =>
					{
						self.messaging.cancel_paste(&mut self.state);
					}
					Ok(clipboard::Content::Text(text))
						if upload_allowed
							&& can_attach && paste.target.text_attachment
							&& !self.messaging.has_edit_in(self.state.selected)
							&& model::message_options::content(
								&paste.target.replacement(&text),
							)
							.0
							.chars()
							.count() > text_limit =>
					{
						// Text that cannot fit one message is attached as `message.txt`, like Discord.
						if let Err(error) =
							discord_api::upload::Source::pasted_text(text).and_then(|source| {
								self.uploads.select_pasted(
									paste.target.generation,
									paste.target.channel,
									vec![source],
									self.runtime.handle(),
									&ctx,
								)
							}) {
							self.messaging.toasts.push(ui::design::Level::Error, error);
						}
					}
					Ok(clipboard::Content::Text(text)) => {
						self.messaging.pasted_text = Some((paste.target.clone(), text));
					}
					Ok(clipboard::Content::File(source))
						if upload_allowed
							&& can_attach && !self.messaging.has_edit_in(self.state.selected) =>
					{
						if let Err(error) = self.uploads.select_pasted(
							paste.target.generation,
							paste.target.channel,
							source,
							self.runtime.handle(),
							&ctx,
						) {
							self.messaging.toasts.push(ui::design::Level::Error, error);
						}
					}
					Ok(clipboard::Content::File(..)) => self.messaging.toasts.push(
						ui::design::Level::Error,
						"Attaching files is unavailable here",
					),
					Err(error) => self.messaging.toasts.push(ui::design::Level::Error, error),
				}
			}
			self.clipboard = None;
		}
		if !can_attach {
			self.uploads.cancel();
		}
		let state = &self.state;
		self.uploads.follow_conversation(
			state.generation,
			state.selected,
			state.user.is_some() && state.gateway_connected,
			|channel| state.can_view(channel),
		);
		self.uploads.poll(
			self.state.generation,
			self.state.selected,
			self.state.user.is_some() && self.state.gateway_connected,
			&ctx,
		);
		if let Some(command) = self.uploads.image_send(&mut self.state, can_attach)
			&& !self.state.demo
		{
			// Explicit artwork Send finishes asynchronously; retain its prepared thumbnails.
			self.messaging.attachment_previews = self.uploads.previews();
			self.messaging.attachment_files = self.uploads.files();
			self.messaging.stage_pending_upload(&ctx, &command);
			if let Command::Send { channel, .. } = &command {
				self.messaging.draft_changes.push(*channel);
			}
			self.command(command);
		}
		// Move native handles once; never load dropped bytes on the rendering thread.
		if !dropped.is_empty() {
			if self.messaging.accepts_server_emoji_drops() {
				let paths: Vec<_> = dropped
					.into_iter()
					.take(11)
					.map(|file| file.path().to_path_buf())
					.collect();
				if !paths.is_empty() {
					self.messaging.queue_server_emoji_drop(paths);
				}
			} else if upload_allowed
				&& can_attach
				&& self.login.is_none()
				&& !self.confirming_close
				&& !self.confirming_logout
				&& !self.messaging.has_edit_in(self.state.selected)
				&& !self.downloads.is_active()
				&& let Some(channel) = self.state.selected
			{
				if let Err(error) = self.uploads.start_drop(
					self.state.generation,
					channel,
					self.runtime.handle(),
					&ctx,
					dropped,
				) {
					self.messaging.toasts.push(ui::design::Level::Error, error);
				}
			} else {
				self.messaging.toasts.push(
					ui::design::Level::Error,
					"File not attached; return to a connected conversation and drop it again",
				);
			}
		}
		// Offline fixtures may stage a synthetic attachment without any upload selection.
		if !self.state.demo || self.uploads.selection().is_some() {
			self.messaging.attachment = self
				.uploads
				.selection()
				.map(|(name, size)| (name.to_owned(), size));
			self.messaging.attachment_previews = self.uploads.previews();
			self.messaging.attachment_files = self.uploads.files();
		}
		self.messaging
			.external_upload
			.progress(self.uploads.public_progress());
		if let Some(result) = self.uploads.take_public_result() {
			self.messaging.external_upload.complete(result);
		}
		self.messaging.upload_busy = self.uploads.busy()
			|| self
				.clipboard
				.as_ref()
				.is_some_and(clipboard::Paste::active);
		self.messaging.attach_busy = !self.uploads.accepting();
		self.messaging.attachment_loading = self.uploads.loading();
		if let Some(notice) = self.uploads.take_notice() {
			self.messaging.toasts.push(ui::design::Level::Error, notice);
		}
		if !self.state.demo {
			let (progress, sending) = self.uploads.transfer_progress();
			self.messaging.update_upload_progress(progress, sending);
		}
		if self.state.user.is_none() {
			self.downloads.cancel();
		}

		let download_status = match self.downloads.poll() {
			downloads::Status::Idle | downloads::Status::Choosing => String::new(),
			downloads::Status::Downloading { total: 0, .. } => "Loading image…".into(),
			downloads::Status::Downloading { received, total } => {
				format!("Downloading: {} / {} KiB", received / 1024, total / 1024)
			}
			downloads::Status::Saved | downloads::Status::Cancelled | downloads::Status::Copied => {
				String::new()
			}
			downloads::Status::Failed(error) => (*error).into(),
		};
		self.messaging.downloads().active = self.downloads.is_active();
		self.messaging.downloads().status = download_status;
		if close_requested && self.extensions.cleanup_pending() {
			self.extension_close_pending = true;
			ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
		}
		if self.extension_close_pending && !self.extensions.cleanup_pending() {
			self.extension_close_pending = false;
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
		}
		if close_requested {
			self.downloads.cancel();
		}
		if close_requested && self.downloads.is_active() {
			self.download_close_pending = true;
			ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
		}
		if self.download_close_pending
			&& !self.downloads.is_active()
			&& (!self.confirming_close || self.close_approved)
		{
			self.download_close_pending = false;
			ctx.send_viewport_cmd(egui::ViewportCommand::Close);
		}
		self.poll_avatars(&ctx);
		if let Some((scope, result)) = self.group_icon.poll(&self.state) {
			self.messaging.accept_group_icon(&ctx, scope, result);
		}
		if let Some((scope, result)) = self
			.create_server_icon
			.poll_scoped(self.state.generation, |_| true)
		{
			self.messaging
				.accept_create_server_icon(&ctx, scope, result);
		}
		if let Some((scope, result)) = self
			.profile_avatar
			.poll_scoped(self.state.generation, |user| {
				self.state.user.as_ref().is_some_and(|own| own.id == user)
			}) {
			self.messaging.accept_profile_picture(&ctx, scope, result);
		}
		if let Some((scope, result)) =
			self.server_profile_image
				.poll_scoped(self.state.generation, |guild| {
					self.state.own_server_profile.guild == Some(guild)
						&& self.state.can_edit_server_profile(guild)
				}) {
			self.messaging
				.accept_server_profile_image(&ctx, scope, result);
		}
		if let Some((scope, result)) = self.server_icon.poll_server(&self.state) {
			self.messaging.accept_server_icon(&ctx, scope, result);
		}
		if let Some((_, result)) =
			self.community_artwork
				.poll_scoped(self.state.generation, |guild| {
					self.state.can_open_community_settings(guild)
						&& self
							.community_artwork_scope
							.is_some_and(|scope| self.messaging.community_artwork_active(scope))
				}) && let Some(scope) = self.community_artwork_scope.take()
		{
			self.messaging.accept_community_artwork(&ctx, scope, result);
		}
		if let Some((_, result)) = self
			.webhook_avatar
			.poll_scoped(self.state.generation, |guild| {
				self.webhook_avatar_scope.is_some_and(|scope| {
					scope.1 == guild
						&& self.state.can_set_webhook_avatar(guild, scope.2)
						&& self.messaging.webhook_avatar_active(scope)
				})
			}) && let Some(scope) = self.webhook_avatar_scope.take()
		{
			self.messaging.accept_webhook_avatar(&ctx, scope, result);
		}
		if let Some((_, result)) = self.role_icon.poll_scoped(self.state.generation, |guild| {
			self.role_icon_scope.is_some_and(|scope| {
				scope.1 == guild
					&& self.state.server_admin.guild == Some(guild)
					&& self.state.can_edit_role_icon(guild, scope.2)
			})
		}) && let Some(scope) = self.role_icon_scope.take()
		{
			self.messaging.accept_server_role_icon(&ctx, scope, result);
		}

		if let Some((scope, result)) = self.emoji_upload.poll(self.state.generation, |guild| {
			self.state.server_admin.guild == Some(guild) && self.state.can_create_guild_emoji(guild)
		}) {
			self.messaging.accept_server_emojis(&ctx, scope, result);
		}
		if let Some((scope, result)) = self.sticker_upload.poll(self.state.generation, |guild| {
			self.state.server_admin.guild == Some(guild)
				&& self.state.can_create_guild_sticker(guild)
		}) {
			self.messaging.accept_server_sticker(&ctx, scope, result);
		}
		if let Some((scope, result)) = self.sound_upload.poll(self.state.generation, |guild| {
			self.state.server_admin.guild == Some(guild) && self.state.can_create_guild_sound(guild)
		}) {
			self.messaging.accept_server_sound(scope, result);
		}
		if let Some((scope, result)) = self.sound_upload.poll_trim() {
			self.messaging.accept_server_sound_trim(scope, result);
		}
		self.messaging.voice_available = true;
		if close_requested
			&& !self.close_approved
			&& ((self.state.demo && self.state.has_unsent())
				|| self
					.state
					.pending
					.iter()
					.any(|p| p.delivery != Delivery::Confirmed)
				|| self.messaging.has_edit()
				|| self.messaging.has_server_settings_changes()
				|| self.messaging.extensions.theme_editor_dirty()
				|| self.state.server_settings.pending
				|| self.state.server_admin.pending
				|| self.voice_messages.has_unsent()
				|| self.uploads.has_unsent()
				|| self.messaging.external_upload.has_unsent()
				|| self.credential_cleanup_pending()
				|| self.messaging.startup_busy
				|| self.avatar_cleanup.is_some()
				|| self.cache_pending > 0
				|| self.cache_clears.pending()
				|| self.cache_forgets.pending()
				|| (!self.fixture_only && self.app_settings.state.needs_attention())
				|| (!self.fixture_only && self.reading.needs_attention())
				|| (!self.fixture_only && self.game_activity.needs_attention())
				|| (!self.fixture_only && self.tray_setting.needs_attention())
				|| self.cache_error)
		{
			ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
			self.confirming_close = true;
		}
		if self.login.is_some() {
			let p = ui::design::palette(ui);
			egui::Panel::top("login-header")
				.exact_size(platform::LOGIN_HEADER_HEIGHT)
				.show_separator_line(false)
				.frame(
					egui::Frame::NONE
						.fill(ui::design::window_palette(ui).surface)
						.stroke(egui::Stroke::new(1.0, p.border))
						.inner_margin(egui::Margin::symmetric(16, 0)),
				)
				.show(ui, |ui| {
					ui::design::window_drag(ui, ui.max_rect());
					ui.horizontal_centered(|ui| {
						ui.add_space(self.traffic_light_inset());
						let (rect, _) =
							ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
						ui::icons::paint_brand(ui.painter(), rect);
						ui.add_space(4.0);
						ui.vertical(|ui| {
							ui.spacing_mut().item_spacing.y = 1.0;
							ui.label(
								ui::design::semibold(
									ui,
									ui::i18n::translate("main-ui-sign-in-to-discord"),
									15.0,
								)
								.color(p.text_strong),
							);
							ui.label(
								egui::RichText::new(ui::i18n::translate(
									"main-ui-discord-com-temporary-login-window-passwords-and-2fa-never-leave",
								))
								.size(12.0)
								.color(p.muted),
							);
						});
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							if !self.messaging.hide_title_bar {
								ui::design::window_controls(ui);
							}
							if ui
								.add(
									egui::Button::new(
										ui::design::medium(
											ui,
											ui::i18n::translate("main-ui-cancel"),
											13.0,
										)
										.color(p.text_strong),
									)
									.fill(p.raised)
									.stroke(egui::Stroke::new(1.0, p.border))
									.corner_radius(6)
									.min_size(egui::vec2(0.0, 32.0)),
								)
								.clicked()
							{
								self.login = None;
								self.state.auth = AuthState::Unauthenticated;
							}
						});
					});
				});
			egui::CentralPanel::default()
				.frame(egui::Frame::NONE.fill(p.canvas))
				.show(ui, |ui| {
					ui.centered_and_justified(|ui| {
						ui.label(
							egui::RichText::new(ui::i18n::translate("main-ui-loading-discord-com"))
								.color(p.muted),
						);
					});
				});
			if let Some(login) = &self.login {
				login.resize(&self.window);
			}
		} else if self.state.user.is_some() {
			self.messaging.storage_status = self.cache_status;
			self.messaging.notification_status = self.notifications.status().label();
			if self.app_settings.state.failed {
				self.messaging.notification_sound_status = "Could not save device notification settings. Changes apply only until restart.";
			}
			let mut commands = self.messaging.show(ui, &mut self.state);
			self.sync_message_speech(&ctx);
			self.extensions.cancel_stale_message_events(&self.state);
			self.choose_interaction_files(&ctx);
			if let Some(command) = self.captcha.sync(
				&mut self.state,
				&mut self.messaging,
				&self.window,
				&ctx,
				!self.fixture_only && !self.confirming_close && !self.confirming_logout,
			) {
				commands.push(command);
			}
			let player = self.messaging.audio();
			if !player.seen
				|| player.active.is_none()
				|| (!self.state.demo && self.state.auth != AuthState::Authenticated)
			{
				self.audio.stop();
				player.stop();
			}
			if let Some(command) = player.command.take() {
				match command {
					ui::AudioCommand::Play(attachment) => {
						self.audio.volume(player.volume);
						// Speed applies to voice messages only, like Discord's control.
						self.audio.speed(if attachment.is_voice_message() {
							player.speed
						} else {
							1.0
						});
						if let Err(error) = self.audio.start(
							attachment,
							self.runtime.handle(),
							&ctx,
							self.fixture_only || self.state.demo,
						) {
							player.state = ui::AudioState::Failed(error);
						}
					}
					ui::AudioCommand::Pause(paused) => self.audio.pause(paused),
					ui::AudioCommand::Seek(seconds) => {
						self.audio.seek(Duration::from_secs_f64(seconds))
					}
					ui::AudioCommand::Volume(volume) => self.audio.volume(volume),
					ui::AudioCommand::Speed(speed) => self.audio.speed(speed),
					ui::AudioCommand::Stop => self.audio.stop(),
				}
			}
			self.video.output.clone_from(&self.messaging.media_output);
			let player = self.messaging.video();
			if self.state.demo
				&& let Some(pause) = self.demo_video_autoplay
				&& let Some(message) = self.state.timeline.get(model::Id(601))
				&& let Some(attachment) = message.attachments.first().cloned()
			{
				if player.active.is_none() {
					player.active = Some((message.channel, message.id, attachment.clone()));
					player.state = ui::VideoState::Loading;
					player.seen = true;
					player.command = Some(ui::VideoCommand::Play(attachment));
				} else if player.state == ui::VideoState::Playing && player.position > 1.0 {
					if pause {
						player.command = Some(ui::VideoCommand::Pause(true));
					}
					self.demo_video_autoplay = None;
				}
			}
			player.update_visibility();
			if (player.active.is_none() && player.provider.is_none())
				|| (!self.state.demo && self.state.auth != AuthState::Authenticated)
			{
				self.video.stop();
				player.stop();
			}
			if let Some(command) = player.command.take() {
				self.video.command(
					command,
					player,
					self.runtime.handle(),
					&ctx,
					self.fixture_only || self.state.demo,
				);
			}
			self.video.sync_provider(
				player,
				self.state.generation,
				&self.window,
				&ctx,
				!self.fixture_only
					&& !self.state.demo
					&& !self.confirming_close
					&& !self.confirming_logout
					&& self.state.auth == AuthState::Authenticated
					&& self.state.verification().is_none(),
			);
			// Posters download attachments too, so they follow the playback session gate.
			if !self.state.demo && self.state.auth != AuthState::Authenticated {
				player.poster_queue.clear();
			} else if !player.poster_queue.is_empty() {
				let queue = std::mem::take(&mut player.poster_queue);
				self.video.posters(
					queue,
					player,
					self.runtime.handle(),
					&ctx,
					self.fixture_only || self.state.demo,
				);
			}
			self.notifications.set_enabled(
				self.messaging.notifications_enabled
					&& (!self.fixture_only || self.messaging.notification_test_available),
			);
			if std::mem::take(&mut self.messaging.notification_test_requested)
				&& self.messaging.notification_test_available
			{
				self.notifications.notify();
			}
			self.poll_voice_messages(&ctx);
			if let Some(request) = self
				.voice_messages
				.take_send(&mut self.state, &mut self.messaging.voice_messages)
			{
				if let Some(connection) = &self.connection {
					if let Err(error) = connection.uploads.try_send(request) {
						let request = error.into_inner();
						request
							.progress
							.send_replace(discord_api::upload::Status::Failed(
								"Upload queue full; record again",
							));
						self.state.command_rejected(request.command);
					}
				} else {
					request
						.progress
						.send_replace(discord_api::upload::Status::Failed(
							"Connection unavailable; record again",
						));
					self.state.command_rejected(request.command);
				}
			}
			// Park or restore staged files after navigation without polling workers again.
			let state = &self.state;
			self.uploads.follow_conversation(
				state.generation,
				state.selected,
				state.user.is_some() && state.gateway_connected,
				|channel| state.can_view(channel),
			);
			if !self.state.selected.is_some_and(|channel| {
				self.state.can_attach(channel) || self.state.can_attach_post(channel)
			}) {
				self.uploads.cancel();
			}
			if let Some(index) = self.messaging.host_attachment_requested.take()
				&& let Some(channel) = self.state.selected
				&& self.state.can_send(channel)
				&& !self.uploads.busy()
				&& let Some((filename, bytes)) = self.messaging.attachment_files.get(index).cloned()
			{
				self.messaging.external_upload.open(
					self.state.generation,
					channel,
					index,
					self.uploads.public_selection_key(index),
					filename,
					bytes,
				);
			}
			if std::mem::take(&mut self.messaging.external_upload.cancel_requested) {
				self.uploads.cancel_public();
			}
			if let Some(ui::external_upload::Request {
				generation,
				channel,
				index,
				key,
				filename,
				bytes,
				host,
			}) = self.messaging.external_upload.request.take()
			{
				let result = if generation != self.state.generation
					|| self.state.selected != Some(channel)
					|| !self.state.can_send(channel)
				{
					Err(model::public_upload::Error::ConversationChanged)
				} else {
					self.uploads.start_external(
						index,
						key,
						generation,
						channel,
						&filename,
						bytes,
						host,
						self.runtime.handle(),
						&ctx,
						self.state.demo,
					)
				};
				if let Err(error) = result {
					self.messaging.external_upload.complete(Err(error));
				}
			}
			if let Some(index) = self.messaging.remove_attachment_index.take() {
				self.uploads.remove_at(index);
				if self.state.demo {
					self.messaging.attachment = None;
					self.messaging.attachment_files.clear();
					self.messaging.attachment_previews.clear();
				}
			}
			if std::mem::take(&mut self.messaging.remove_attachment_requested) {
				self.uploads.remove();
				self.messaging.attachment = None;
				self.messaging.attachment_files.clear();
				self.messaging.attachment_previews.clear();
			}
			if std::mem::take(&mut self.messaging.cancel_upload_requested) {
				// The timeline row cancels its own upload, not files loading for the next message.
				self.uploads.cancel_transfer();
			}
			if let Some((generation, channel, assets, draft)) =
				self.messaging.image_share_requested.take()
				&& generation == self.state.generation
				&& self.state.selected == Some(channel)
				&& self.state.can_send(channel)
				&& self.state.can_attach(channel)
				&& let Err(error) = self.uploads.start_image_share(
					self.state.generation,
					channel,
					(assets, draft),
					self.runtime.handle(),
					&ctx,
					self.state.demo,
				) {
				self.state.status = error;
			}
			if let Some(request) = self.messaging.attachment_paste_requested.take()
				&& request.target.generation == self.state.generation
				&& Some(request.target.channel) == self.state.selected
			{
				if self.clipboard.is_none() {
					self.clipboard = Some(clipboard::Paste::start(
						request,
						self.runtime.handle(),
						&ctx,
					));
				} else {
					self.messaging.toasts.push(
						ui::design::Level::Error,
						"Wait for the current paste to finish",
					);
				}
			}
			if std::mem::take(&mut self.messaging.attach_requested)
				&& let Some(channel) = self.state.selected
				&& (self.state.can_attach(channel) || self.state.can_attach_post(channel))
				&& let Err(error) = self.uploads.start_choose(
					self.state.generation,
					channel,
					self.runtime.handle(),
					&ctx,
					self.window.clone(),
				) {
				self.state.status = error;
			}
			if std::mem::take(&mut self.messaging.downloads().dismiss_requested) {
				self.downloads.dismiss();
			}
			if std::mem::take(&mut self.messaging.downloads().cancel_requested) {
				self.downloads.cancel();
			}
			if let Some((media, copy)) = self.messaging.downloads().embed_request.take()
				&& !self.state.demo
				&& !self.fixture_only
				&& let Err(error) = self.downloads.start_embed(
					media,
					copy,
					self.runtime.handle(),
					&ctx,
					self.window.clone(),
				) {
				self.state.status = error;
			}
			if let Some(attachment) = self.messaging.downloads().copy_request.take()
				&& !self.state.demo
				&& !self.fixture_only
				&& let Err(error) =
					self.downloads
						.start_copy(attachment, self.runtime.handle(), &ctx)
			{
				self.state.status = error;
			}
			if let Some(attachment) = self.messaging.downloads().request.take()
				&& !self.state.demo
				&& !self.fixture_only
				&& let Err(error) = self.downloads.start(
					attachment,
					self.runtime.handle(),
					&ctx,
					self.window.clone(),
				) {
				self.state.status = error;
			}

			if let Some(scope) = self.messaging.take_profile_picture_request() {
				let result = if scope.0 != self.state.generation
					|| !self
						.state
						.user
						.as_ref()
						.is_some_and(|own| own.id == scope.1)
				{
					Err("Sign in before changing your profile picture")
				} else {
					if self.messaging.profile_picture_is_banner() {
						self.profile_avatar.start_banner(
							scope,
							self.runtime.handle(),
							&ctx,
							self.window.clone(),
							"Choose profile banner",
						)
					} else {
						self.profile_avatar.start(
							scope,
							self.runtime.handle(),
							&ctx,
							self.window.clone(),
							"Choose profile picture",
							256,
						)
					}
				};
				if let Err(error) = result {
					self.messaging
						.accept_profile_picture(&ctx, scope, Err(error));
				}
			}
			if let Some((scope, banner)) = self.messaging.take_server_profile_image_request() {
				let result = if scope.0 != self.state.generation
					|| !self.state.can_edit_server_profile(scope.1)
				{
					Err("This server is no longer available")
				} else if banner {
					self.server_profile_image.start_banner(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose server profile banner",
					)
				} else {
					self.server_profile_image.start(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose server profile picture",
						256,
					)
				};
				if let Err(error) = result {
					self.messaging
						.accept_server_profile_image(&ctx, scope, Err(error));
				}
			}
			if let Some(scope) = self.messaging.take_group_icon_request() {
				let result = if scope.0 != self.state.generation || !self.state.is_group_dm(scope.1)
				{
					Err("This group is no longer available")
				} else {
					self.group_icon.start(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose group icon",
						256,
					)
				};
				if let Err(error) = result {
					self.messaging.accept_group_icon(&ctx, scope, Err(error));
				}
			}
			if let Some(scope) = self.messaging.take_create_server_icon_request() {
				let result = if scope.0 != self.state.generation || scope.1 != model::Id(0) {
					Err("This server draft is no longer available")
				} else {
					self.create_server_icon.start(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose server icon",
						256,
					)
				};
				if let Err(error) = result {
					self.messaging
						.accept_create_server_icon(&ctx, scope, Err(error));
				}
			}
			if let Some(scope) = self.messaging.take_community_artwork_request() {
				let result = if scope.0 != self.state.generation
					|| !self.state.can_edit_community_settings(
						scope.1,
						model::community_admin::Section::Guide,
					) {
					Err("You can no longer manage this server")
				} else {
					self.community_artwork.start_image(
						(scope.0, scope.1, scope.4),
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose Server Guide artwork",
						512,
						scope.2.is_some(),
					)
				};
				match result {
					Ok(()) => self.community_artwork_scope = Some(scope),
					Err(error) => self
						.messaging
						.accept_community_artwork(&ctx, scope, Err(error)),
				}
			}
			if let Some(scope) = self.messaging.take_webhook_avatar_request() {
				let result = if scope.0 != self.state.generation
					|| !self.state.can_set_webhook_avatar(scope.1, scope.2)
				{
					Err("You can no longer edit this webhook")
				} else {
					self.webhook_avatar.start(
						(scope.0, scope.1, scope.3),
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose webhook avatar",
						128,
					)
				};
				match result {
					Ok(()) => self.webhook_avatar_scope = Some(scope),
					Err(error) => self
						.messaging
						.accept_webhook_avatar(&ctx, scope, Err(error)),
				}
			}
			if let Some(scope) = self.messaging.take_server_role_icon_request() {
				let result = if scope.0 != self.state.generation
					|| !self.state.can_edit_role_icon(scope.1, scope.2)
				{
					Err("You can no longer change this role icon")
				} else {
					self.role_icon.start(
						(scope.0, scope.1, scope.3),
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
						"Choose role icon",
						128,
					)
				};
				match result {
					Ok(()) => self.role_icon_scope = Some(scope),
					Err(error) => self
						.messaging
						.accept_server_role_icon(&ctx, scope, Err(error)),
				}
			}
			if let Some(scope) = self.messaging.take_server_icon_request() {
				let result =
					if scope.0 != self.state.generation || !self.state.can_manage_guild(scope.1) {
						Err("You can no longer manage this server")
					} else {
						self.server_icon.start(
							scope,
							self.runtime.handle(),
							&ctx,
							self.window.clone(),
							"Choose server icon",
							512,
						)
					};
				if let Err(error) = result {
					self.messaging.accept_server_icon(&ctx, scope, Err(error));
				}
			}
			if let Some((generation, guild, request, paths)) =
				self.messaging.take_server_emoji_request()
			{
				let scope = (generation, guild, request);
				let result = if generation != self.state.generation
					|| !self.state.can_create_guild_emoji(guild)
				{
					Err("You can no longer upload emoji to this server")
				} else {
					self.emoji_upload.start(
						scope,
						paths,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
					)
				};
				if let Err(error) = result {
					self.messaging.accept_server_emojis(&ctx, scope, Err(error));
				}
			}
			if let Some((generation, guild, request)) = self.messaging.take_server_sticker_request()
			{
				let scope = (generation, guild, request);
				let result = if generation != self.state.generation
					|| !self.state.can_create_guild_sticker(guild)
				{
					Err("You can no longer upload stickers to this server")
				} else {
					self.sticker_upload.start(
						scope,
						self.runtime.handle(),
						&ctx,
						self.window.clone(),
					)
				};
				if let Err(error) = result {
					self.messaging
						.accept_server_sticker(&ctx, scope, Err(error));
				}
			}
			if let Some((generation, guild, request)) = self.messaging.take_server_sound_request() {
				let scope = (generation, guild, request);
				let result = if generation != self.state.generation
					|| !self.state.can_create_guild_sound(guild)
				{
					Err("You can no longer upload sounds to this server")
				} else {
					self.sound_upload
						.start(scope, self.runtime.handle(), &ctx, self.window.clone())
				};
				if let Err(error) = result {
					self.messaging.accept_server_sound(scope, Err(error));
				}
			}
			if let Some((scope, start, end)) = self.messaging.take_server_sound_trim() {
				self.sound_upload.trim(scope, start, end, &ctx);
			}
			if let Some((scope, selection)) = self.messaging.take_server_sound_preview() {
				// The offline preview never opens an audio device.
				if !self.state.demo {
					self.sound_upload.preview(scope, selection, &ctx);
				}
			}
			if self.messaging.take_server_sound_closed() {
				self.sound_upload.clear();
			}
			// A review reset by a permission change, a scope change or sign-out never reports
			// closing; its decoded audio is released here instead.
			let review = self
				.messaging
				.server_sound_review()
				.filter(|(generation, guild, _)| {
					*generation == self.state.generation
						&& self.state.can_create_guild_sound(*guild)
				});
			self.sound_upload.retain(review);
			for key in self.messaging.take_avatar_requests() {
				if !self
					.avatars
					.as_ref()
					.is_some_and(|worker| worker.request(key.clone()))
				{
					self.messaging.accept_avatar(&ctx, key, None);
				}
			}
			// Idle frames are freed on a pass, so an otherwise idle window wakes once for them.
			if let Some(at) = self.messaging.avatar_release_at() {
				ctx.request_repaint_after(at.saturating_duration_since(std::time::Instant::now()));
			}
			if std::mem::take(&mut self.messaging.reconnect_requested) {
				if self.state.auth == AuthState::Authenticated {
					if let Some(connection) = &self.connection {
						connection.reconnect();
						self.state.status = "Reconnecting to Discord…";
					}
				} else {
					if let Some(store) = &mut self.store {
						store.cancel_load();
					}
					let wake = ctx.clone();
					match platform::LoginView::open(self.window.clone(), move || {
						wake.request_repaint()
					}) {
						Ok(login) => self.login = Some(login),
						Err(error) => self.state.status = error.label(),
					}
				}
			}
			let draft_changes = std::mem::take(&mut self.messaging.draft_changes);
			for channel in draft_changes {
				let content = recovery_draft(&self.state, channel);
				self.queue_cache(cache::Operation::SaveDraft { channel, content });
			}
			if self.messaging.clear_cache_requested {
				self.messaging.clear_cache_requested = false;
				self.state.clear_cached_history();
				self.clear_avatars(&ctx);
				self.queue_cache(cache::Operation::ClearHistory);
			}
			if let Some(command) = self.state.take_gif_favorites_command() {
				commands.push(command);
			}
			if let Some(command) = self
				.state
				.next_reaction_preferences_read(std::time::Instant::now())
			{
				commands.push(command);
			}
			if self.state.gateway_connected
				&& !self.state.demo
				&& let Some(delay) = self
					.state
					.reaction_preferences_delay(std::time::Instant::now())
			{
				ctx.request_repaint_after(delay);
			}
			commands.extend(self.state.take_preload_retries());
			for command in commands {
				self.command(command);
			}
			if let Some(connection) = &self.connection {
				connection.sync_admin_reads(&self.state);
				connection.sync_member_watches(&self.state);
			}
			// A click that adopted a hover preload sends no request of its own.
			self.load_cached_history();
			self.sync_member_cache();
			self.poll_member_cache_search();
			self.sync_expression_cache();
			self.sync_sticker_pack_cache();
			self.sync_server_cache();
			self.poll_force_sync(&ctx);
			if self.messaging.logout_requested {
				self.messaging.logout_requested = false;
				self.request_session_end(&ctx, SessionEnd::Logout);
			}
		} else if let Some(stage) = self.restoring() {
			self.restoring_screen(ui, &stage);
		} else {
			self.sign_in_screen(ui);
		}
		// Outside the signed-in branch so leaving an account still restores the window mode.
		let video = self.messaging.video().take_fullscreen_request();
		let voice = self.messaging.take_voice_fullscreen_request();
		for fullscreen in [video, voice].into_iter().flatten() {
			self.window.set_fullscreen(
				fullscreen
					.then(|| winit::window::Fullscreen::Borderless(self.window.current_monitor())),
			);
		}
		#[cfg(feature = "demo")]
		if let Some(diagnostic) = &self.rendering_demo {
			diagnostic.show(&ctx, &self.window);
		}
		let appearance = ctx.options(|options| options.theme_preference);
		#[cfg(target_os = "linux")]
		if self.window.is_decorated() == self.messaging.hide_window_decorations {
			self.window
				.set_decorations(!self.messaging.hide_window_decorations);
		}
		#[cfg(target_os = "windows")]
		if self.window.is_decorated() != self.messaging.hide_title_bar {
			self.window.set_decorations(self.messaging.hide_title_bar);
			align_undecorated_surface(&self.window);
		}
		#[cfg(target_os = "macos")]
		if let Err(error) =
			platform::window::set_native_title_bar(&self.window, self.messaging.hide_title_bar)
		{
			self.messaging.hide_title_bar = false;
			self.state.status = error;
		}
		if !self.messaging.hide_title_bar {
			frame.set_traffic_lights_position(&ctx, egui::Rangef::new(0.0, title_bar_height), 12.0);
		}
		self.sync_customization(&ctx);
		self.save_app_preferences();
		self.messaging.updates_save_failed = self.app_settings.state.failed;
		self.save_reading_preferences(&ctx);
		self.sync_own_presence(&ctx);
		self.sync_game_activity(&ctx);
		let offline = self.fixture_only || self.state.demo;
		self.game_mentions
			.sync(&mut self.state, &self.runtime, &ctx, offline);
		self.startup.sync(
			&ctx,
			&self.runtime,
			&mut self.messaging,
			self.fixture_only || self.state.demo,
		);
		if appearance != self.appearance {
			self.appearance = appearance;
			self.appearance_changed = true;
			let preference = match appearance {
				egui::ThemePreference::System => local_store::Appearance::System,
				egui::ThemePreference::Light => local_store::Appearance::Light,
				egui::ThemePreference::Dark => local_store::Appearance::Dark,
			};
			self.queue_cache_for(model::Id(0), cache::Operation::SaveAppearance(preference));
		}
		self.sync_gif_cache();
		if !self.fixture_only
			&& !self.state.demo
			&& let Some(user) = &self.state.user
		{
			self.cache_pending += usize::from(queue_channel_preferences(
				self.cache.as_ref(),
				&mut self.messaging,
				self.state.generation,
				user.id,
			));
		}
		if self.messaging.channel_preferences_changed
			&& !self.messaging.channel_preferences_save_pending
		{
			self.messaging.channel_preferences_changed = false;
			if !self.state.demo && !self.fixture_only {
				self.messaging.channel_preferences_save_pending =
					self.queue_cache(cache::Operation::SaveChannelPreferences(
						self.messaging.channel_preferences.clone(),
					));
				self.messaging.channel_preferences_status =
					if self.messaging.channel_preferences_save_pending {
						""
					} else {
						"Could not save channel preferences."
					};
			}
		}
		if let Some(variant) = self.messaging.theme_variant_changed.take() {
			self.variant_changed = true;
			let key = (variant != ui::design::Variant::Standard).then(|| variant.key().to_owned());
			self.queue_cache_for(model::Id(0), cache::Operation::SaveThemeVariant(key));
		}
		let switch = self.messaging.switch_account_requested.take();
		let add = std::mem::take(&mut self.messaging.add_account_requested);
		let forget = self.messaging.forget_account_requested.take();
		// Fixture builds render the switcher for captures but never end their offline session.
		if !self.fixture_only {
			if let Some(account) = switch
				&& self
					.state
					.user
					.as_ref()
					.is_none_or(|user| user.id != account)
			{
				self.request_session_end(&ctx, SessionEnd::Switch(account));
			}
			if add {
				self.request_session_end(&ctx, SessionEnd::Add);
			}
			if let Some(account) = forget {
				self.confirming_forget = Some(account);
			}
		}
		self.sync_account_roster();
		self.confirm_forget_dialog(&ctx);
		if self.confirming_close || self.confirming_logout {
			let public_upload_note = self
				.messaging
				.external_upload
				.has_unsent()
				.then(|| ui::i18n::translate("public-upload-leave"));
			let mut notes: Vec<&str> = Vec::new();
			if let Some(note) = public_upload_note.as_deref() {
				notes.push(note);
			}
			if self.messaging.extensions.theme_editor_dirty() {
				notes.push("Unsaved theme changes will be discarded.");
			}
			if self.credential_cleanup_pending() {
				notes.push("Wait for saved-login removal to finish.");
			}
			if self.extensions.cleanup_pending() {
				notes.push("Removing extension data before closing.");
			}
			if self.cache_clears.pending() {
				notes.push(
					"Cached history cleanup is pending; closing now may leave deleted messages on disk.",
				);
			}
			if self.cache_forgets.pending() {
				notes.push(
					"Local account data removal is waiting for storage; closing now may leave account data on disk.",
				);
			}
			if !self.fixture_only && self.app_settings.state.needs_attention() {
				notes.push(self.app_settings.state.status());
			}
			if !self.fixture_only && self.reading.needs_attention() {
				notes.push(self.reading.status());
			}
			if !self.fixture_only && self.game_activity.needs_attention() {
				notes.push(self.game_activity.status());
			}
			if !self.fixture_only && self.tray_setting.needs_attention() {
				notes.push(self.tray_setting.status());
			}
			let mut confirm = ui::dialog::Confirm::new(
				"leave-session",
				if self.confirming_logout && !self.end_intent.forgets() {
					"Leave this account?"
				} else {
					"Leave this session?"
				},
				if self.confirming_logout && !self.end_intent.forgets() {
					"Saved text drafts survive the switch; selected files must be reselected. This account stays in the switcher."
				} else {
					"Saved text drafts survive exit; selected files must be reselected. Logging out removes local account data."
				},
			)
			.danger()
			.confirm_label("Discard and Continue")
			.cancel_label("Keep Working")
			.enabled(!self.credential_cleanup_pending());
			if !notes.is_empty() {
				confirm = confirm.note(ui::dialog::Level::Warning, notes.join("\n"));
			}
			match confirm.show(&ctx) {
				Some(ui::dialog::Choice::Confirmed) => {
					if self.confirming_close {
						self.close_approved = true;
						self.uploads.cancel();
						if self.downloads.is_active() {
							self.downloads.cancel();
							self.download_close_pending = true;
						} else {
							ctx.send_viewport_cmd(egui::ViewportCommand::Close);
						}
					} else {
						let intent = self.end_intent;
						self.finish_session_end(&ctx, intent);
					}
				}
				Some(ui::dialog::Choice::Cancelled) => {
					self.updater.cancel_restart();
					self.confirming_close = false;
					self.tray_window.cancel_quit();
					self.extension_close_pending = false;
					self.confirming_logout = false;
					self.end_intent = SessionEnd::Logout;
					self.download_close_pending = false;
				}
				None => {}
			}
		}
		if let Some(deadline) = self.state.next_channel_preference_deadline() {
			ctx.request_repaint_after(
				deadline.saturating_duration_since(std::time::Instant::now()),
			);
		}
		ui::design::window_resize(&ctx);
		self.frame_metrics.reflows = self.messaging.timeline_reflows();
		self.frame_metrics.finish();
	}
}
