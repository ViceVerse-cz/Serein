//! Coordinate visible views and drain their existing actions in order.
use super::{
	Command, Freshness, Id, MessagingUi, RichText, State, activity_alerts, attachments, design,
	dialog, forum, invites, markdown, member_guide, message_menu, name_styles, notifications,
	profiles, search, shortcuts, switcher, thread_create, user_direct, user_menu, video,
};

impl MessagingUi {
	pub fn show(&mut self, ui: &mut egui::Ui, state: &mut State) -> Vec<Command> {
		self.retire_menu_paste(state);
		message_menu::set_owner(
			ui.ctx(),
			message_menu::Owner {
				generation: state.generation,
				channel: state.selected.unwrap_or(Id(0)),
				message: Id(0),
			},
		);
		self.timeline.menu.sync(ui.ctx(), state);
		self.timeline.visible_replies.clear();
		self.avatars.cosmetics = self.cosmetics;
		name_styles::set_enabled(self.cosmetics.name_styles);
		let cosmetics_refresh = state.set_profile_cosmetics(self.cosmetics);
		self.timeline
			.download
			.sync_gif_favorites(&state.gifs.favorites);
		crate::scroll::apply_preferences_in(
			ui.ctx(),
			self.reading_preferences,
			egui::Id::unique((
				"reading-wheel",
				state.generation,
				state.selected,
				state.browse_channels,
				self.settings.open,
				self.search.open,
			)),
		);
		let diagnostics_chord = self
			.keybinds
			.chord(model::KeybindAction::CopyIssueDiagnostics);
		let diagnostics_function_key = diagnostics_chord
			.key
			.strip_prefix('F')
			.and_then(|number| number.parse::<u8>().ok())
			.is_some_and(|number| (1..=35).contains(&number));
		let diagnostics_allowed_in_context = diagnostics_function_key
			|| diagnostics_chord.modifiers
				& (model::keybinds::PRIMARY | model::keybinds::CTRL | model::keybinds::ALT)
				!= 0 || (!ui.ctx().egui_wants_keyboard_input()
			&& !ui.input(|input| {
				input
					.events
					.iter()
					.any(|event| matches!(event, egui::Event::Text(_) | egui::Event::Paste(_)))
			}));
		if self.keybind_capture.is_none()
			&& !self.ime_active
			&& diagnostics_allowed_in_context
			&& ui.input(|input| {
				input.focused
					&& !input
						.events
						.iter()
						.any(|event| matches!(event, egui::Event::Ime(_)))
			}) && ui.input_mut(|input| {
			crate::keybinds::pressed_exact(
				input,
				self.keybinds
					.chord(model::KeybindAction::CopyIssueDiagnostics),
			)
		}) {
			self.copy_diagnostic_info(ui.ctx());
		}
		crate::i18n::set_current(self.language);
		let language = self.language;
		if let Some(status) = state.take_user_action_status() {
			self.toasts.push(design::Level::Error, status);
		}
		let mention_waiting = self.pending_mention.is_some();
		let composer_open = self.editing.is_some()
			|| state.selected.is_some_and(|id| {
				state
					.channel(id)
					.is_some_and(|channel| channel.supports_text())
			});
		if mention_waiting && !composer_open {
			self.pending_mention = None;
			state.status = "This view has no message box, so the mention was not added.";
		}
		if self.editing.is_none()
			&& let Some(index) = state
				.message_actions
				.failed_edits
				.iter()
				.position(|(channel, _, _)| Some(*channel) == state.selected)
		{
			let (channel, message, content) = state.message_actions.failed_edits.remove(index);
			self.editing = Some((channel, message, content));
			self.edit_modified = Some((channel, message, true));
			self.edit_sent = None;
		}

		if self
			.pending_upload
			.as_ref()
			.is_some_and(|upload| !state.pending.iter().any(|p| p.nonce == upload.nonce))
		{
			self.pending_upload = None;
		}
		self.timeline.audio.seen = false;
		self.timeline.video.seen = false;
		if let Some(player) = &mut self.timeline.video.provider {
			player.bounds = None;
			player.clip = None;
		}
		let side = self.drain_side_press();
		let mut commands = Vec::new();
		commands.extend(state.request_voice_channel_statuses());
		if let Some(delay) = state.voice_status_retry_delay() {
			ui.ctx().request_repaint_after(delay);
		}
		commands.extend(cosmetics_refresh);
		// A server joined from an invite opens as soon as the Gateway delivers it.
		commands.extend(state.navigate_after_join());
		if let Some(action) = self.poll_creator.show(ui.ctx(), state, &mut self.avatars)
			&& let Some(command) = state.prepare_poll(None, action)
		{
			commands.push(command);
		}
		// Bound side buttons run their binding instead of history navigation.
		let back = side.back && !self.button_claimed(egui::PointerButton::Extra1);
		let forward = side.forward && !self.button_claimed(egui::PointerButton::Extra2);
		if back || forward {
			self.navigate_history(state, &mut commands, back);
		}
		if !self.settings.open
			&& !self.server_settings.is_open()
			&& !self.ime_active
			&& self.keybind_capture.is_none()
			&& !self.timeline.video.is_fullscreen()
			&& !self.is_voice_fullscreen()
			&& ui.ctx().memory(|memory| memory.top_modal_layer().is_none())
			&& !egui::Popup::is_any_open(ui.ctx())
			&& ui.input(|input| {
				input.focused
					&& !input
						.events
						.iter()
						.any(|event| matches!(event, egui::Event::Ime(_)))
			}) {
			let action = [
				model::KeybindAction::NextUnreadConversation,
				model::KeybindAction::PreviousUnreadConversation,
				model::KeybindAction::JumpToUnread,
			]
			.into_iter()
			.find(|action| {
				ui.input_mut(|input| {
					crate::keybinds::pressed_exact(input, self.keybinds.chord(*action))
				})
			});
			match action {
				Some(model::KeybindAction::JumpToUnread) => {
					if let Some(command) = state.open_unread() {
						commands.push(command);
					}
					ui.ctx().request_repaint();
				}
				Some(action) => {
					if let Some(channel) = state.next_unread_channel(
						action == model::KeybindAction::PreviousUnreadConversation,
					) {
						self.guild = state.channel(channel).and_then(|channel| channel.guild);
						if let Some(command) = state.select(channel) {
							commands.push(command);
						}
					}
				}
				None => {}
			}
		}
		self.sync_voice_connection(state);
		// Fullscreen media owns the whole client surface, including during native resizing.
		if self.show_fullscreen_voice(ui.ctx(), state) {
			ui.painter()
				.rect_filled(ui.max_rect(), 0, egui::Color32::BLACK);
			return commands;
		}
		let expanded_video = if self.timeline.video.origin == video::Origin::Search {
			let message = self.search.playback_message(state, &self.timeline.video);
			self.timeline
				.show_search_video(ui.ctx(), message, state.demo)
		} else {
			self.timeline.show_fullscreen_video(ui.ctx(), state)
		};
		if expanded_video {
			ui.painter()
				.rect_filled(ui.max_rect(), 0, egui::Color32::BLACK);
			return commands;
		}
		self.show_voice_connection_debug(ui.ctx(), state);
		self.profile.disarm();
		ui.ctx().data_mut(|data| {
			data.remove::<egui::Rect>(profiles::profile_opener_id());
		});
		let ctx = ui.ctx().clone();
		user_menu::set_voice_available(&ctx, self.voice_available);
		self.extensions.reset_theme_shortcut(&ctx);
		if self.extensions.has_result() {
			self.settings.open = false;
		}
		if let Some(effect) = self.extensions.show_result(
			&ctx,
			state,
			&mut self.draft_changes,
			self.editing.is_some(),
			&mut self.avatars,
			(self.game_activity_status, self.share_game_activity),
		) && let Err(error) = self.apply_extension_effect(&ctx, state, effect, &mut commands)
		{
			self.extensions.report_error(error);
		}
		self.keybinds_shortcut(&ctx);
		self.theme_preview_navigation(ui);
		let settings_open = self.settings.open
			|| self.server_settings.is_open()
			|| self.channel_menu.is_open()
			|| self.community_ui.is_open()
			|| self.inbox_open;
		self.extensions.begin_theme_editor_frame();
		if self.inbox_open {
			ui.disable();
		}
		if self.settings.open {
			self.show_settings(&ctx, state, &mut commands);
			if self.extensions.previewing_theme() {
				self.settings.open = false;
			}
			ui.disable();
		}
		if self.server_settings.is_open() {
			self.extensions.stop_theme_preview(&ctx);
			if self.profile.open_user().is_some()
				&& ctx
					.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
			{
				self.profile.close();
			}
			self.server_settings.show(
				&ctx,
				state,
				&mut self.avatars,
				&mut self.profile,
				&mut commands,
			);
			if self.profile.open_user().is_some() {
				ctx.set_sublayer(
					egui::LayerId::new(
						egui::Order::Foreground,
						egui::Id::unique("server-settings"),
					),
					egui::LayerId::new(
						egui::Order::Foreground,
						egui::Id::unique("user-profile-popout"),
					),
				);
			}
			if let Some(guild) = self.server_settings.events_requested.take()
				&& self.navigate_away_from_server_settings(state)
			{
				self.server_menu.events_requested = Some(guild);
			}
			if let Some(action) = self.server_settings.admin.user_action.take() {
				self.user_action = Some(action);
			}
			if let Some(channel) = self.server_settings.admin.message.take()
				&& self.navigate_away_from_server_settings(state)
				&& let Some(command) = state.select(channel)
			{
				commands.push(command);
			}
			ui.disable();
		}
		// Foreground confirmation handles Escape before background search/archive shortcuts.
		self.confirm_friend_removal(&ctx, state, &mut commands);
		if let Some(link) = self
			.timeline
			.opening
			.as_deref()
			.and_then(markdown::discord_chat_link)
		{
			self.timeline.opening = None;
			match state.open_chat_link(link.guild, link.channel, link.message) {
				Ok(Some(command)) => commands.push(command),
				Ok(None) => {}
				Err(message) => state.status = message,
			}
		}
		if let Some((guild, event)) = self
			.timeline
			.opening
			.as_deref()
			.and_then(model::community::event_link)
		{
			self.timeline.opening = None;
			self.community_ui
				.open_link(state, guild, event, &mut commands);
		}
		markdown::confirm_external_link(
			&ctx,
			&mut self.timeline.browser_opening,
			self.reading_preferences.confirm_external_links,
		);
		markdown::confirm_external_link(
			&ctx,
			&mut self.timeline.opening,
			self.reading_preferences.confirm_external_links,
		);
		let colors = crate::design::palette(ui);
		let background = crate::design::window_palette(ui);
		if !settings_open
			&& !self.switcher.is_open()
			&& !self.ime_active
			&& ctx.input(|input| {
				input.focused
					&& !input
						.events
						.iter()
						.any(|event| matches!(event, egui::Event::Ime(_)))
			}) && ctx.input_mut(|input| {
			crate::keybinds::pressed(
				input,
				self.keybinds
					.chord(model::KeybindAction::SwitchConversation),
			)
		}) {
			self.switcher.open(&ctx);
		}
		self.switcher_frame = self.switcher.is_open();
		let direct_origin = (state.selected, state.request);
		if let Some(command) = state.select_opened_dm() {
			commands.push(command);
		}
		self.finish_user_direct(state, direct_origin, &mut commands);
		self.finish_slash_direct(state);
		if let Some(target) = self.switcher.show(&ctx, state, &mut self.avatars) {
			match target {
				switcher::Target::Channel(channel) => {
					if (state.selected != Some(channel) || state.browse_channels.is_some())
						&& let Some(command) = state.select(channel)
					{
						commands.push(command);
					}
					self.focus_switched_composer = state.selected == Some(channel)
						&& state
							.channel(channel)
							.is_some_and(|known| known.supports_text());
				}
				switcher::Target::Friend(user) => {
					if let Some(command) = state.open_friend_dm(user) {
						commands.push(command);
					}
				}
			}
		}
		if self.navigation_channel != state.selected {
			self.focus_switched_composer = state.selected.is_some();
			self.navigation_channel = state.selected;
			self.guild = state
				.selected
				.and_then(|id| state.channel(id))
				.and_then(|channel| channel.guild);
		}
		if let Some(guild) = state.browse_channels {
			self.guild = Some(guild);
		}
		let title = self
			.guild
			.and_then(|id| state.guild(id))
			.map_or_else(|| language.text("direct-messages"), |g| g.name.clone());
		if self.shows_title_bar() {
			self.title_bar(ui, state, &mut commands, &title);
		}
		// Server rail and channel list share one resizable column so the account card can
		// span both, like Discord's bottom-left user pill.
		let rail = notifications::RAIL_WIDTH;
		let sidebar_max = self.prepare_reading_sidebar(ui, "navigation", rail);
		// See-through chrome paints each section once; a shared column coat would stack under
		// the rail and account strip and make them denser than the title bar.
		let layered = design::layered_sections(ui);
		let navigation = egui::Panel::left("navigation")
			.resizable(true)
			.show_separator_line(!design::has_window_background(ui))
			.default_size(rail + f32::from(self.reading_preferences.sidebar_width).min(sidebar_max))
			.size_range(rail + 190.0..=rail + sidebar_max)
			.frame(
				egui::Frame::new()
					.fill(if layered {
						egui::Color32::TRANSPARENT
					} else {
						background.base
					})
					.inner_margin(0),
			)
			.show(ui, |ui| {
				let footer = egui::Panel::bottom("account-footer")
					.show_separator_line(false)
					.frame(
						egui::Frame::new()
							// Over a window image the card's strip continues the channel list;
							// over desktop transparency it is the rail's material, like the title bar.
							.fill(if design::has_window_background(ui) {
								design::section_surface(
									ui,
									background.sidebar,
									design::ImageSection::ChannelList,
								)
							} else if layered {
								design::section_surface(
									ui,
									background.base,
									design::ImageSection::ServerList,
								)
							} else {
								egui::Color32::TRANSPARENT
							})
							.inner_margin(egui::Margin {
								left: 8,
								right: 8,
								top: 8,
								bottom: 8,
							}),
					)
					.show(ui, |ui| self.account_card(ui, state, &mut commands));
				// Nested panels use their parent's full cross-axis. Limit subsequent
				// navigation painting so the rail/list cannot cover the account controls.
				let navigation_clip = egui::Rect::from_min_max(
					ui.clip_rect().min,
					egui::pos2(ui.clip_rect().right(), footer.response.rect.top()),
				);
				ui.shrink_clip_rect(navigation_clip);
				self.notification_rail(ui, state, &mut commands);
				// The lists sit on their own rounded surface beside the rail, above the card.
				let lists = ui.available_rect_before_wrap();
				// A window image continues the lists into the card strip, so only it stays square.
				let rounded = !design::has_window_background(ui);
				ui.painter().rect_filled(
					lists,
					if rounded {
						egui::CornerRadius {
							nw: 8,
							sw: 8,
							..Default::default()
						}
					} else {
						egui::CornerRadius::ZERO
					},
					design::section_surface(
						ui,
						background.sidebar,
						design::ImageSection::ChannelList,
					),
				);
				if rounded && layered {
					// No column coat sits under the corners, so the rail's material fills the
					// notches and the curve still reads from the title bar into the account strip.
					let notch = design::section_surface(
						ui,
						background.base,
						design::ImageSection::ServerList,
					);
					for (corner, inward) in [
						(lists.left_top(), egui::vec2(1.0, 1.0)),
						(lists.left_bottom(), egui::vec2(1.0, -1.0)),
					] {
						design::paint_corner_notch(ui.painter(), corner, inward, 8.0, notch);
					}
				}
				self.sidebar(ui, state, &title, &mut commands);
			});
		self.record_reading_sidebar(navigation.response.rect.width() - rail);
		if self.channel_preferences_changed {
			self.channel_cache.invalidate();
		}
		// After the sidebar check: remembering a visit does not change the channel list.
		self.sync_last_channels(state);
		self.channel_menu
			.show(&ctx, state, self.guild, &mut self.avatars, &mut commands);
		if let Some(collapsed) = self.channel_menu.categories_collapsed_requested.take()
			&& let Some(guild) = self.guild
		{
			let categories: Vec<_> = state
				.channels
				.iter()
				.filter(|channel| {
					channel.guild == Some(guild)
						&& channel.kind == 4
						&& state.can_personally_manage_channel(channel.id)
				})
				.map(|channel| channel.id)
				.collect();
			if let Some(action) = state.personal_collapse_change(guild, &categories, collapsed) {
				commands.extend(state.request_personal_channel_action(guild, action));
			}
		}
		if let Some((guild, channel)) = self.channel_menu.invite_requested.take() {
			self.server_menu.open_invite(state, guild, channel);
		}
		self.server_menu
			.show(&ctx, state, self.guild, &mut commands, &mut self.avatars);
		if let Some(guild) = self.server_menu.guide_requested.take()
			&& let Some(command) = state.open_server_guide(guild)
		{
			commands.push(command);
		}

		if let Some(guild) = self.server_menu.events_requested.take() {
			self.community_ui.open_events(state, guild, &mut commands);
		}
		self.community_ui.show(&ctx, state, &mut commands);
		if let Some(guild) = self.server_menu.browse_requested.take() {
			self.browse_channels.open(state, guild);
		}
		if let Some((guild, action)) = self.server_menu.channel_preferences_requested.take() {
			commands.extend(state.request_personal_channel_action(guild, action));
		}
		self.browse_channels.sync(state, self.guild);
		if self.navigation_browser != state.browse_channels {
			self.navigation_browser = state.browse_channels;
			if state.browse_channels.is_some() {
				self.browse_channels.focus_search();
			}
			if state.browse_channels.is_none() {
				self.guild = state
					.selected
					.and_then(|id| state.channel(id))
					.and_then(|channel| channel.guild);
			}
			crate::scroll::reset_wheel(&ctx);
			self.timeline.menu = Default::default();
			self.pins_anchor = None;
			self.focus_switched_composer =
				state.browse_channels.is_none() && state.selected.is_some();
		}
		if let Some(guild) = self.server_menu.settings_requested.take()
			&& let Some(command) = self.preview_server_settings(state, guild)
		{
			commands.push(command);
		}
		if let Some(guild) = self.server_menu.mark_read_requested.take()
			&& let Some(command) = state.prepare_mark_guild_read(guild)
		{
			commands.push(command);
		}
		if std::mem::take(&mut self.server_menu.mark_all_read_requested)
			&& let Some(command) = state.prepare_mark_all_read()
		{
			commands.push(command);
		}
		if let Some(command) = state.continue_mark_all_read() {
			commands.push(command);
		}
		let selected_voice = state
			.selected
			.and_then(|id| state.channel(id))
			.is_some_and(|c| matches!(c.kind, 2 | 13));
		let selected_forum = state.selected.is_some_and(|id| state.is_forum(id));
		let browsing_channels = state.browse_channels.is_some();
		if let Some(id) = state.posting.created.take()
			&& let Some(command) = state.select(id)
		{
			commands.push(command);
		}
		let wide_members = ui.available_width() >= 720.0;
		if !settings_open
			&& !browsing_channels
			&& !self.switcher_frame
			&& !self.ime_active
			&& state.can_search()
			&& ctx.memory(|memory| memory.top_modal_layer().is_none())
			&& !egui::Popup::is_any_open(&ctx)
			&& ctx.input(|input| {
				input.focused
					&& !input.events.iter().any(|event| match event {
						egui::Event::Ime(
							egui::ImeEvent::Preedit { text, .. } | egui::ImeEvent::Commit(text),
						) => !text.is_empty(),
						egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { .. }) => true,
						_ => false,
					})
			}) && ctx.input_mut(|input| {
			crate::keybinds::pressed_exact(
				input,
				self.keybinds
					.chord(model::KeybindAction::SearchConversation),
			)
		}) && let Some(channel) = state.selected
		{
			self.search.focus_conversation(channel, state);
		}
		self.reconcile_submitted_command(state);
		if browsing_channels
			&& !self.ime_active
			&& ctx.input_mut(|input| {
				crate::keybinds::pressed_exact(
					input,
					self.keybinds
						.chord(model::KeybindAction::SearchConversation),
				)
			}) {
			self.browse_channels.focus_search();
		}
		self.search.sync(&ctx, state, &mut commands);
		let search_open = !browsing_channels
			&& self.search.results_visible(state)
			&& state.selected.is_some()
			&& !selected_voice;
		let show_members = !browsing_channels
			&& !selected_voice
			&& !search_open
			&& state.selected.is_some()
			&& if wide_members {
				*self.members_preference(state)
			} else {
				self.members_narrow_open
			};
		if search_open {
			self.channel_header(
				ui,
				state,
				selected_voice,
				show_members,
				wide_members,
				true,
				&mut commands,
			);
			let width = if wide_members {
				search::PANE_WIDTH.min(ui.available_width() * 0.45)
			} else {
				(ui.available_width() * 0.6).max(240.0)
			};
			egui::Panel::right("search-pane")
				.resizable(false)
				.show_separator_line(!design::has_window_background(ui))
				.exact_size(width)
				.frame(
					egui::Frame::new()
						.fill(design::section_surface(
							ui,
							background.sidebar,
							design::ImageSection::MemberList,
						))
						.inner_margin(egui::Margin::same(12)),
				)
				.show(ui, |ui| {
					self.search.pane(
						ui,
						state,
						&mut commands,
						&mut self.avatars,
						search::MediaUi {
							download: &mut self.timeline.download,
							audio: &mut self.timeline.audio,
							video: &mut self.timeline.video,
						},
						&mut self.profile,
					);
				});
		}
		if show_members {
			if state
				.members
				.as_ref()
				.is_none_or(|list| Some(list.channel) != state.selected)
				&& let Some(command) = state.request_members()
			{
				commands.push(command);
			}
			if wide_members {
				egui::Panel::right("people-pane")
					.resizable(false)
					.show_separator_line(!design::has_window_background(ui))
					.exact_size(240.0)
					.frame(
						egui::Frame::new()
							.fill(design::section_surface(
								ui,
								background.sidebar,
								design::ImageSection::MemberList,
							))
							.inner_margin(egui::Margin {
								left: 8,
								right: 8,
								top: 8,
								bottom: 8,
							}),
					)
					.show(ui, |ui| {
						self.member_rows(ui, state, &mut commands);
					});
			} else {
				let response = dialog::Dialog::new("members-narrow", language.text("members"))
					.subtitle(language.text("members-description"))
					.width(360.0)
					.show(&ctx, |d| {
						let max = (d.available_height() - 180.0).clamp(120.0, 620.0);
						d.content(|ui| {
							ui.set_max_height(max);
							self.member_rows(ui, state, &mut commands);
						});
					});
				if response.close {
					self.members_narrow_open = false;
				}
			}
		} else if state.members.is_some() {
			commands.push(state.close_members());
		}
		self.reaction_picker.sync(state, state.selected);
		egui::CentralPanel::default()
			.frame(
				egui::Frame::new()
					.fill(if design::has_section_background(ui) {
						egui::Color32::TRANSPARENT
					} else {
						background.chat
					})
					.inner_margin(0),
			)
			.show(ui, |ui| {
				if browsing_channels {
					let browse_shortcuts = shortcuts::ShortcutView::new(
						&self.channel_preferences,
						self.shortcuts_available(state),
					);
					self.browse_channels.show(
						ui,
						state,
						self.guild,
						(browse_shortcuts, self.show_hidden_channels),
						&mut self.channel_menu,
						&mut commands,
					);
					return;
				}
				let message_surface =
					design::section_surface(ui, background.chat, design::ImageSection::MessageList);
				if design::has_section_background(ui)
					&& (state.selected.is_none() || selected_voice || selected_forum)
				{
					ui.painter().rect_filled(ui.max_rect(), 0, message_surface);
				}
				let warnings = state.startup_warnings;
				if !warnings.presence {
					self.friends.presence_warning_dismissed = None;
				}
				let unavailable: Vec<_> = [
					(warnings.read_state, "read status"),
					(warnings.notifications, "notification settings"),
					(warnings.sessions, "session status"),
					(warnings.emojis, "some server emoji"),
					(warnings.stickers, "some server stickers"),
					(warnings.entries, "some malformed channels, DMs or contacts"),
				]
				.into_iter()
				.filter_map(|(unavailable, label)| unavailable.then_some(label))
				.collect();
				if !unavailable.is_empty() {
					egui::Frame::new()
						.inner_margin(egui::Margin::symmetric(16, 6))
						.show(ui, |ui| {
							ui.colored_label(
								colors.warning,
								format!(
									"Connected with some data unavailable: {}. Reconnect to retry.",
									unavailable.join(", ")
								),
							);
						});
				}
				if state.selected.is_none() && self.guild.is_none() {
					self.call_bar(ui, state, &mut commands, None);
					self.timeline.download.show_status(ui);
					self.friends_page(ui, state, &mut commands);
					return;
				}
				// In a DM call the header moves onto the call stage, shown while hovered.
				let header_in_call = !search_open && self.dm_call_stage(state).is_some();
				if !search_open && !header_in_call {
					self.channel_header(
						ui,
						state,
						selected_voice,
						show_members,
						wide_members,
						false,
						&mut commands,
					);
				}
				if let Some(guild) = state
					.selected
					.and_then(|id| state.channel(id))
					.and_then(|channel| channel.guild)
					&& crate::server_access::AccessUi::banner(ui, state, guild)
					&& let Some(command) = self.preview_server_settings(state, guild)
				{
					self.preview_server_safety();
					commands.push(command);
				}
				let message_fill = (design::has_section_background(ui)
					&& state.selected.is_some()
					&& !selected_voice
					&& !selected_forum)
					.then(|| {
						(
							ui.painter().add(egui::Shape::Noop),
							ui.available_rect_before_wrap().top(),
						)
					});
				self.call_bar(
					ui,
					state,
					&mut commands,
					header_in_call.then_some((selected_voice, show_members, wide_members)),
				);
				self.timeline.download.show_status(ui);
				let Some(channel) = state.selected else {
					ui.add_space((ui.available_height() * 0.32).max(24.0));
					ui.vertical_centered(|ui| {
						ui.label(
							design::semibold(ui, language.text("no-conversation-selected"), 20.0)
								.color(colors.text_strong),
						);
						ui.add_space(8.0);
						ui.label(
							RichText::new(language.text("select-conversation")).color(colors.muted),
						);
					});
					return;
				};
				if selected_voice {
					if !self.voice_chat_open {
						self.voice_channel(ui, state, channel, &mut commands);
						return;
					}
					// Keep the existing conversation renderer beside the stage. On narrow
					// windows chat takes the body; Hide chat returns to the full stage.
					if ui.available_width() >= 720.0 {
						let chat_width = (ui.available_width() * 0.4).clamp(320.0, 440.0);
						egui::Panel::left("voice-stage")
							.exact_size(ui.available_width() - chat_width)
							.resizable(false)
							.frame(egui::Frame::NONE)
							.show(ui, |ui| {
								self.voice_channel(ui, state, channel, &mut commands);
							});
					}
				}
				if selected_forum {
					// Both post fields paste like the message composer: files and images from
					// any app join the post's attachments, text lands in the focused field.
					if let Some(field) = self.forum.paste.take() {
						self.menu_paste = Some(field);
					}
					if let Some(field) =
						[forum::title_id(), forum::body_id()]
							.into_iter()
							.find(|id| {
								self.menu_paste.is_some_and(|(_, _, field)| field == *id)
									|| ui.ctx().memory(|m| m.has_focus(*id))
							}) {
						let ctx = ui.ctx().clone();
						self.clipboard_paste(
							&ctx,
							state,
							channel,
							field,
							!egui::Popup::is_any_open(&ctx),
						);
					}
					// A post's first message uses the same upload tray as the composer.
					let textures = self.attachment_textures(ui.ctx());
					let files = self.selected_files();
					let view = shortcuts::ShortcutView::new(
						&self.channel_preferences,
						self.shortcuts_available(state),
					);
					let mut staged = forum::Staged {
						files: &files,
						textures: &textures,
						choose: &mut self.attach_requested,
						remove: &mut self.remove_attachment_index,
						clear: &mut self.remove_attachment_requested,
						busy: self.upload_busy,
					};
					self.forum.show(
						ui,
						state,
						channel,
						&mut commands,
						(&mut self.scroll, &mut staged, &mut self.avatars),
						(&mut self.channel_menu, view),
					);
					return;
				}
				egui::Panel::bottom("composer")
					.show_separator_line(false)
					.frame(
						egui::Frame::new()
							// The central panel already paints the chat surface; a second
							// coat would band a darker strip under a see-through timeline.
							.fill(if design::has_section_background(ui) {
								design::section_surface(
									ui,
									background.chat,
									design::ImageSection::Composer,
								)
							} else {
								egui::Color32::TRANSPARENT
							})
							// The bottom inset matches the account card's, so the input and the
							// account pill sit on one line across the window.
							.inner_margin(egui::Margin {
								left: 16,
								right: 16,
								top: 2,
								bottom: 8,
							}),
					)
					.show(ui, |ui| {
						if state.message_request(channel) {
							self.message_request_bar(ui, state, channel);
						}
						self.composer(ui, state, channel, &ctx, &mut commands);
					});
				if let Some((shape, top)) = message_fill {
					let remaining = ui.available_rect_before_wrap();
					ui.painter().set(
						shape,
						egui::Shape::rect_filled(
							egui::Rect::from_min_max(
								egui::pos2(remaining.left(), top),
								remaining.right_bottom(),
							),
							0.0,
							message_surface,
						),
					);
				}
				let notices: Vec<String> = [
					match state.freshness {
						Freshness::Fresh | Freshness::Loading => None,
						Freshness::Stale => Some("Cached history · awaiting sync"),
						Freshness::Unavailable => Some("Conversation unavailable"),
					}
					.map(str::to_owned),
					state.read_state.status(channel).map(str::to_owned),
					(state.archived_thread.is_some() && state.archived_thread == state.selected)
						.then(|| "Opened from archive".to_owned()),
				]
				.into_iter()
				.flatten()
				.collect();
				if !notices.is_empty() {
					egui::Frame::new()
						.inner_margin(egui::Margin::symmetric(16, 4))
						.show(ui, |ui| {
							ui.label(
								RichText::new(notices.join(" · "))
									.size(11.0)
									.color(colors.muted),
							);
						});
				}
				egui::Frame::new()
					.inner_margin(egui::Margin {
						left: 0,
						right: 0,
						top: 4,
						bottom: 0,
					})
					.show(ui, |ui| {
						design::paint_chat_background(ui, ui.available_rect_before_wrap());
						self.timeline.hide_media_links = self.reading_preferences.hide_media_links;
						self.timeline.compact_messages = self.reading_preferences.compact_messages;
						self.timeline.double_click_reaction = self
							.reading_preferences
							.double_click_reaction_enabled
							.then_some(self.reading_preferences.double_click_emoji());
						self.timeline.instant_scrolling =
							!self.reading_preferences.smooth_scrolling;
						self.timeline.extension_actions = self.extensions.message_actions();
						self.timeline.quick_reactions =
							Some(self.reaction_picker.quick_reactions());
						self.timeline.show_with_scroll(
							ui,
							state,
							&mut self.editing,
							&mut self.deleting,
							(&mut self.avatars, &mut self.profile),
							self.pending_upload.as_ref(),
							&mut self.scroll,
						);
						if let Some((id, action)) = self.timeline.poll_action.take()
							&& let Some(command) = state.prepare_poll(Some(id), action)
						{
							commands.push(command);
						}
						if let Some(id) = self.timeline.sticker_request.take() {
							if let Some(command) = state.request_sticker(id) {
								commands.push(command);
							}
							if let Some(command) = state.request_sticker_packs() {
								commands.push(command);
							}
						}
						if let Some(sticker) = self.timeline.browse_sticker.take() {
							self.emoji_picker.open_stickers(Some(&sticker));
						}
						if let Some(command) =
							state.request_author_members(&self.timeline.visible_authors)
						{
							commands.push(command);
						}
						if let Some((action, text)) = self.timeline.extension_request.take() {
							self.extensions
								.invoke_message(action, text, state, ui.ctx());
						}
						if let Some((message, anchor, trigger)) =
							self.timeline.reaction_picker.take()
						{
							self.emoji_picker.dismiss(state, &mut commands);
							self.reaction_picker
								.open_reaction(state, message, anchor, trigger);
						}
						self.reaction_picker.hide_nitro_emojis = self.hide_nitro_emojis;
						if self
							.reaction_picker
							.reaction_message()
							.is_some_and(|message| !self.timeline.message_visible(state, message))
						{
							self.reaction_picker.dismiss(state, &mut commands);
						}
						self.reaction_picker.show_reaction(
							ui,
							state,
							&mut self.avatars,
							&mut commands,
						);
						if let Some((message, emoji, open)) = self.timeline.reaction_users.take()
							&& let Some(command) =
								state.request_reaction_users(message, emoji, open)
						{
							commands.push(command);
						}
						if state.reactions.users.as_ref().is_some_and(|users| {
							!self.timeline.message_visible(state, users.message)
						}) {
							state.close_reaction_users();
						}
						crate::reactions::show_users(
							ui.ctx(),
							state,
							&mut self.avatars,
							&mut commands,
						);
						if let Some((channel, message)) = self.timeline.quick_delete.take()
							&& let Some(command) = state.prepare_delete(channel, message)
						{
							commands.push(command);
						}
						if let Some(id) = self.timeline.remove_preserved.take() {
							state.discard_preserved_deleted(id);
						}
						if let Some(nonce) = self.timeline.restore_pending.take() {
							if state
								.pending
								.iter()
								.any(|p| p.nonce == nonce && p.sticker.is_some())
							{
								state.discard_pending_sticker(&nonce);
							} else {
								self.restore_pending(state, channel, &nonce);
							}
						}
						if let Some(nonce) = self.timeline.dismiss_pending.take()
							&& let Some(channel) = state.discard_pending(&nonce)
						{
							// The recovery draft may have been the dismissed caption; save it again.
							self.draft_changes.push(channel);
						}
						self.cancel_upload_requested |=
							std::mem::take(&mut self.timeline.cancel_upload);
						if let Some(gif) = self.timeline.gif_favorite.take() {
							self.toggle_gif_favorite(&ctx, state, &gif);
						}
						match self.timeline.invite_action.take() {
							Some(invites::Action::OpenGuild(guild))
								if state.guild(guild).is_some() =>
							{
								self.guild = Some(guild);
								if let Some(command) = state.select_guild(guild) {
									commands.push(command);
								}
							}
							Some(invites::Action::Join(code)) => {
								if state.invite_join.code == code
									&& state.invite_challenge().is_some()
								{
									self.verification.active = false;
								} else if let Some(command) = state.join_invite(code) {
									commands.push(command);
								}
							}
							_ => {}
						}
						for code in std::mem::take(&mut self.timeline.invite_requests) {
							if let Some(command) = state.request_invite(code) {
								commands.push(command);
							}
						}
						if std::mem::take(&mut self.timeline.edit_started) {
							self.edit_modified = None;
							self.edit_undo_cleared = false;
							self.composer_edit = None;
							self.edit_sent = None;
						}
						if std::mem::take(&mut self.timeline.reply_started) {
							self.focus_switched_composer = true;
							ctx.request_repaint();
						}
						if let Some(target) = self.timeline.reply_target.take() {
							crate::scroll::reset_wheel(&ctx);
							self.timeline.remember_position(state);
							if let Some(command) = state.open_reply_target(target) {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.unread_jump) {
							crate::scroll::reset_wheel(&ctx);
							if let Some(command) = state.open_unread() {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.load_newer)
							| (std::mem::take(&mut self.timeline.fill_newer)
								// A scroll up outranks filling; it asks for older messages.
								&& !self.timeline.load_older
								// A history request cancels a loading search; only a scroll may.
								&& !state.search.as_ref().is_some_and(|s| s.loading)
								&& !state.archives.as_ref().is_some_and(|a| a.loading))
						{
							if let Some(command) = state.newer_history() {
								commands.push(command);
							}
							ctx.request_repaint();
						} else if std::mem::take(&mut self.timeline.latest) {
							commands.push(state.history(None));
						} else if std::mem::take(&mut self.timeline.load_older)
							&& let Some(command) = state.older_history()
						{
							commands.push(command);
						}
					});
			});
		if let Some(target) = self.timeline.reply_retry.take() {
			commands.extend(state.retry_reply_preview(target));
		}
		commands.extend(state.sync_visible_reply_previews(&self.timeline.visible_replies));
		if state
			.archives
			.as_ref()
			.is_some_and(|view| self.guild != Some(view.guild))
		{
			commands.push(state.clear_archives());
		}
		if let Some((channel, message)) = self.timeline.bookmark_request.take()
			&& let Some(command) = state.request_inbox_action(client_core::inbox::Action::Save {
				channel,
				message,
				due_at: None,
			}) {
			commands.push(command);
		}
		if let Some((channel, message, pinned)) = self.timeline.pin_request.take()
			&& let Some(command) = state.prepare_pin(channel, message, pinned)
		{
			commands.push(command);
		}
		if let Some((channel, message)) = self.timeline.publish_request.take() {
			self.publishing = Some((state.generation, channel, message));
		}
		if let Some(channel) = state.pins_changed.take()
			&& self.search.open
			&& self.search.pins()
			&& state.selected == Some(channel)
			&& let Some(command) = state.request_pins()
		{
			commands.push(command);
		}
		self.voice_messages.show(&ctx, state);
		self.external_upload
			.show(&ctx, state, &mut self.draft_changes);
		if !browsing_channels {
			self.search
				.overlays(&ctx, state, &mut self.avatars, &mut commands);
		}
		if state.invite_challenge().is_none() {
			self.join_server
				.show(&ctx, state, &mut self.avatars, &mut commands);
		}
		if let Some(anchor) = self.pins_anchor {
			let dm = state
				.channels
				.iter()
				.any(|c| Some(c.id) == state.selected && c.guild.is_none());
			self.search.pins_popout(
				ui,
				state,
				anchor,
				dm,
				&mut commands,
				&mut self.avatars,
				search::MediaUi {
					download: &mut self.timeline.download,
					audio: &mut self.timeline.audio,
					video: &mut self.timeline.video,
				},
				&mut self.profile,
			);
			if !(self.search.open && self.search.pins()) {
				self.pins_anchor = None;
			}
		}
		if let Some(link) = self.search.opening.take() {
			self.timeline.opening = Some(link);
		}
		if let Some(channel) = self.search.channel_reference.take() {
			self.timeline.channel_reference = Some(channel);
		}
		if let Some(channel) = self.search.channel_reference_load.take() {
			self.timeline.channel_reference_load = Some(channel);
		}
		// A forum pane lists its own archived posts inline instead of the floating window.
		if !(selected_forum
			&& state
				.archives
				.as_ref()
				.is_some_and(|view| Some(view.parent) == state.selected))
		{
			self.archives
				.show(&ctx, state, &mut commands, &mut self.avatars);
		}
		if let Some(parent) = self.timeline.threads_request.take() {
			self.archive_parent = Some(parent);
		}
		if let Some(request) = self.timeline.activity_review.take()
			&& self.timeline.speech_allowed(state, request.message)
			&& let Some(guild) = request.resolve(state)
		{
			let command = match request.choice {
				activity_alerts::Choice::Members(filter) => {
					self.review_activity_members(state, guild, filter.members())
				}
				activity_alerts::Choice::Member(user) => self.preview_mod_view(state, guild, user),
			};
			if let Some(command) = command {
				commands.push(command);
			}
		}
		if let Some((channel, message)) = self.timeline.thread_request.take() {
			state.clear_channel_action_result(channel);
			// Discord seeds the name from the starter's first line; the user can still edit it.
			let name = state
				.timeline
				.get(message)
				.map(|m| thread_create::suggested_name(&crate::i18n::message_text(m)))
				.unwrap_or_default();
			self.thread_create.open(state, channel, Some(message), name);
		}
		// A thread shows the message it hangs off; read it lazily like other per-channel data.
		if let Some(command) = state.request_thread_starter() {
			commands.push(command);
		}
		if let Some(channel) = self.archives.create_requested.take() {
			// The editor replaces the Threads dialog instead of stacking on it.
			commands.push(state.clear_archives());
			state.clear_channel_action_result(channel);
			self.thread_create.open(state, channel, None, String::new());
		}
		self.thread_create.show(&ctx, state, &mut commands);
		self.screen.show(&ctx, state);
		if let Some(id) = self.timeline.channel_reference_load.take()
			&& state.channel(id).is_none()
			&& state.channel_reference_name(id).is_none()
			&& let Some(guild) = state
				.selected
				.and_then(|selected| state.channel(selected))
				.and_then(|source| source.guild)
			&& let Some(command) = state.request_channel_reference(guild, id)
		{
			commands.push(command);
		}
		if let Some(id) = self.timeline.channel_reference.take() {
			state.clear_channel_action_result(id);
			self.timeline.pending_channel_reference = Some(id);
		}
		if let Some(id) = self.timeline.pending_channel_reference {
			if let Some(target) = state
				.channel(id)
				.filter(|c| c.guild.is_some() && (c.supports_text() || matches!(c.kind, 15 | 16)))
			{
				self.timeline.pending_channel_reference = None;
				self.guild = target.guild;
				if let Some(command) = state.select(id) {
					commands.push(command);
				}
			} else if state.channel_action_status(id).is_some() {
				self.timeline.pending_channel_reference = None;
			} else if let Some(guild) = state
				.selected
				.and_then(|selected| state.channel(selected))
				.and_then(|source| source.guild)
			{
				if let Some(command) = state.request_channel_reference(guild, id) {
					commands.push(command);
				}
			} else {
				self.timeline.pending_channel_reference = None;
			}
		}
		if let Some((channel, message)) = self.timeline.leave_read.take()
			&& !browsing_channels
			&& !settings_open
			&& !self.inbox_open
			&& let Some(command) = state.prepare_mark_left_channel_read(channel, message)
		{
			commands.push(command);
		}
		if let Some(channel) = self.timeline.mark_channel_read.take() {
			self.timeline.mark_read = None;
			if !settings_open
				&& !self.inbox_open
				&& let Some(command) = state.prepare_mark_channel_read(channel)
			{
				commands.push(command);
			}
		}
		if let Some(message) = self.timeline.mark_unread.take() {
			self.timeline.mark_read = None;
			if !settings_open
				&& !self.inbox_open
				&& let Some(command) = state.prepare_mark_unread(message)
			{
				self.timeline.browse_away();
				self.timeline.reset_unread_divider();
				commands.push(command);
			}
		} else if let Some(message) = self.timeline.mark_read.take() {
			if !settings_open
				&& !browsing_channels
				&& !self.inbox_open
				&& state.search_target.is_none()
				&& let Some(command) = state.prepare_mark_read(message)
			{
				commands.push(command);
			} else if self.timeline.auto_read_attempt == Some(message) {
				// A suppressed action never reached the worker; allow it after the overlay closes.
				self.timeline.auto_read_attempt = None;
			}
		}

		if let Some(input) = self.slash_commands.completion_request.take()
			&& let Some(command) =
				state.request_autocomplete(input.id, input.path, input.values, input.focused)
		{
			commands.push(command);
		}
		if self.slash_commands.active.is_none()
			&& state.application_commands.completion.query.is_some()
		{
			state.application_commands.completion = Default::default();
			commands.push(Command::CancelAutocomplete);
		}
		if let Some(message) = self.timeline.forward_request.take() {
			self.forwarding.open(state, message);
		}
		self.forwarding
			.show(&ctx, state, &mut self.avatars, &mut commands);

		if let Some((message, custom_id, values)) = self.timeline.component_action.take()
			&& let Some(command) = state.prepare_component(message, &custom_id, values)
		{
			commands.push(command);
		}
		if let Some(channel) = state.selected {
			if state.interactions.modal.is_none()
				&& self.timeline.components.remote_search
				&& !self.timeline.components.search.text.is_empty()
			{
				self.timeline
					.components
					.search
					.sync(&ctx, state, channel, 0, &mut commands);
			}
			if self.interaction_components.remote_search
				&& !self.interaction_components.search.text.is_empty()
			{
				self.interaction_components
					.search
					.sync(&ctx, state, channel, 0, &mut commands);
			}
		}
		self.interaction_components.dialogs(
			&ctx,
			state,
			&mut commands,
			&mut self.avatars,
			&mut self.timeline.opening,
		);
		self.interaction_file_request = self
			.interaction_components
			.file_request
			.take()
			.or_else(|| self.slash_commands.file_request.take());
		if let Some(id) = self.timeline.dismiss_ephemeral.take() {
			state.dismiss_ephemeral(id);
			self.timeline.components.forget_message(id);
		}
		if let Some(text) = self.timeline.quick_reaction_used.take() {
			self.reaction_picker.record(text);
		}
		if let Some((message, emoji)) = self.timeline.menu.reaction.take() {
			self.timeline.reaction = Some((message, Some(emoji)));
		}
		if let Some(picker) = self.timeline.menu.picker.take() {
			self.timeline.reaction_picker = Some(picker);
		}
		if let Some(url) = ctx
			.data_mut(|data| data.remove_temp::<String>(egui::Id::unique("message-menu-open-link")))
		{
			self.timeline.opening = Some(url);
		}
		if let Some(action) = self.timeline.menu.save.take()
			&& let Some(command) = state.request_inbox_action(action)
		{
			commands.push(command);
		}
		if let Some((message, emoji)) = self.timeline.reaction.take() {
			if let Some(emoji) = emoji {
				if let Some(command) = state.prepare_reaction(message, emoji) {
					commands.push(command);
				}
			} else {
				state.refresh_reactions(message);
			}
		}
		for command in &commands {
			if let Command::Reactions(client_core::reactions::Command::Set {
				emoji,
				add: true,
				..
			}) = command
			{
				self.timeline.menu.record(emoji);
			}
		}
		self.sync_message_actions(&ctx, state, &mut commands);
		self.group_menu
			.show(&ctx, state, &mut self.avatars, &mut commands);
		self.group_create.show(&ctx, state, &mut commands);
		if let Some(intent) = self.channel_menu.device_shortcut_requested.take() {
			self.apply_device_shortcut(state, intent);
		}
		for intent in self
			.channel_menu
			.shortcut_requested
			.take()
			.into_iter()
			.chain(self.group_menu.pin_requested.take())
		{
			commands.extend(self.apply_shortcut(state, intent));
		}
		if let Some(action) = self.user_action.take().or(self.timeline.user_action.take()) {
			let command = match action {
				user_menu::Action::ModView { guild, user } => {
					if state.can_open_mod_view(guild)
						&& self.navigate_away_from_server_settings(state)
					{
						let command = self.mod_view.open(state, guild, user);
						if command.is_some() {
							self.profile.hide();
						}
						command
					} else {
						None
					}
				}
				user_menu::Action::Message(user) => {
					self.open_user_direct(state, user, user_direct::Intent::Message, &mut commands);
					None
				}
				user_menu::Action::StartCall(user) => {
					self.open_user_direct(state, user, user_direct::Intent::Call, &mut commands);
					None
				}
				user_menu::Action::Note(user) => {
					self.profile.hide();
					self.contact_editor.open(user, false, state)
				}
				user_menu::Action::Nickname(user) => {
					self.profile.hide();
					self.contact_editor.open(user, true, state)
				}
				user_menu::Action::Shortcut(intent) => {
					commands.extend(self.apply_shortcut(state, intent));
					None
				}
				user_menu::Action::Mention(user) => {
					if user.id != Id(0) {
						self.pending_mention = Some(user.id);
						ctx.request_repaint();
					}
					None
				}
				action => user_menu::prepare(action, state),
			};
			if let Some(command) = command {
				commands.push(command);
			}
		}
		self.contact_editor.show(&ctx, state, &mut commands);
		self.drain_profile(state, &mut commands);
		// While the card's DM waits on a captcha, step aside for the dialog but keep its draft.
		let direct_verifying = self.user_direct.is_some()
			&& matches!(
				state.verification(),
				Some((client_core::captcha::Verification::Direct { .. }, _))
			);
		if let Some(user) = self.profile.open_user().cloned()
			&& !direct_verifying
		{
			let profile_guild = self.profile_guild(state);
			self.sync_profile(state, &mut commands, &user, profile_guild);
			// The private note uses an independent read and never locks message actions.
			if !user.webhook
				&& state.user.as_ref().is_some_and(|own| own.id != user.id)
				&& state.user_note(user.id).is_none()
				&& (state.demo
					|| (state.gateway_connected
						&& state.auth == client_core::auth::AuthState::Authenticated))
				&& self.profile.note_wanted(state.generation, user.id)
				&& let Some(command) = state.load_user_note(user.id)
			{
				commands.push(command);
			}
			self.profile
				.own_status
				.clone_from(&self.own_presence.custom_status);
			let anchor = self.profile.anchor_or_place(&ctx, user.id);
			self.profile.ingest_opener_rect(&ctx);
			match profiles::show_with_session(
				ui,
				&user,
				state.profile.as_ref(),
				state,
				&mut self.avatars,
				&mut self.profile_link,
				&mut self.profile_formatted,
				self.reading_preferences.confirm_external_links,
				anchor,
				&mut self.profile,
			) {
				Some(profiles::Action::Avatar(media)) => {
					let ext = media
						.url
						.as_deref()
						.and_then(|u| u.split('?').next()?.rsplit_once('.').map(|(_, ext)| ext))
						.unwrap_or("png");
					self.profile_image = Some((
						state.generation,
						model::Attachment {
							id: Id(0),
							filename: format!("avatar.{ext}"),
							description: Some(format!("{}’s profile picture", user.name)),
							content_type: Some(format!("image/{ext}")),
							size: 0,
							media,
							spoiler: false,
							duration_ms: None,
							waveform: Vec::new(),
						},
					));
					self.profile.close();
				}
				Some(profiles::Action::Banner(media)) => {
					let ext = media
						.url
						.as_deref()
						.and_then(|u| u.split('?').next()?.rsplit_once('.').map(|(_, ext)| ext))
						.unwrap_or("png");
					self.profile_image = Some((
						state.generation,
						model::Attachment {
							id: Id(0),
							filename: format!("banner.{ext}"),
							description: Some(format!("{}’s banner", user.name)),
							content_type: Some(format!("image/{ext}")),
							size: 0,
							media,
							spoiler: false,
							duration_ms: None,
							waveform: Vec::new(),
						},
					));
					self.profile.close();
				}
				Some(profiles::Action::AddFriend(id)) => {
					if let Some(command) = state.add_profile_friend(id) {
						commands.push(command);
					}
				}
				Some(profiles::Action::AcceptFriend(id)) => {
					if let Some(command) = state.resolve_friend_request(id, true) {
						commands.push(command);
					}
				}
				Some(profiles::Action::RemoveFriend) => {
					self.friend_removal = Some((state.generation, user.clone()));
				}
				Some(profiles::Action::Menu(action)) => {
					// Dispatched with the other user menus on the next frame.
					self.user_action = Some(action);
				}
				Some(profiles::Action::Edit) => {
					self.profile.close();
					self.preview_settings("profile");
				}
				Some(profiles::Action::Profile(next)) => {
					self.profile.navigate(next);
				}
				Some(profiles::Action::Server(guild)) => {
					self.profile.close();
					if state.guild(guild).is_some()
						&& self.navigate_away_from_server_settings(state)
					{
						self.guild = Some(guild);
						if let Some(command) = state.select_guild(guild) {
							commands.push(command);
						}
					}
				}
				Some(profiles::Action::EditStatus) => self.open_custom_status_editor(),
				Some(profiles::Action::ClearStatus) => self.clear_custom_status(),
				Some(profiles::Action::Close) => {
					self.profile.close_unless_armed(&ctx);
				}
				Some(profiles::Action::SaveBadges(settings)) => {
					if let Some(command) = state.save_badge_settings(settings) {
						commands.push(command);
					}
				}
				Some(profiles::Action::Retry) => {
					if let Some(command) = state.request_profile(user.id, profile_guild) {
						commands.push(command);
					}
				}
				Some(profiles::Action::SendMessage { user, content }) => {
					self.open_user_direct(
						state,
						user,
						user_direct::Intent::Send(content),
						&mut commands,
					);
				}
				None => {}
			}
			if self.drain_profile(state, &mut commands)
				&& let Some(user) = self.profile.open_user().cloned()
			{
				self.sync_profile(state, &mut commands, &user, profile_guild);
			}
		} else {
			self.profile_link = None;
		}

		if let Some((generation, image)) = &self.profile_image
			&& (*generation != state.generation
				|| attachments::viewer(
					ui,
					std::slice::from_ref(image),
					image.id,
					&mut self.avatars,
					&mut self.timeline.download,
					&mut self.timeline.opening,
					state.demo,
				)
				.is_none())
		{
			self.profile_image = None;
		}

		self.reconcile_edit(state);
		self.announcement_follow.show(&ctx, state, &mut commands);
		if let Some((generation, channel, message)) = self.publishing {
			if generation != state.generation
				|| state.selected != Some(channel)
				|| state.timeline.get(message).is_none()
			{
				self.publishing = None;
			} else {
				let allowed =
					state.can_publish_message(channel, message) && !state.channel_action_pending();
				match dialog::Confirm::new(
					"publish-announcement",
					crate::i18n::translate("announcement-publish-title"),
					crate::i18n::translate("announcement-publish-detail"),
				)
				.confirm_label(crate::i18n::translate("announcement-publish"))
				.enabled(allowed)
				.show(&ctx)
				{
					Some(dialog::Choice::Confirmed) => {
						if let Some(command) = state.request_channel_action(
							channel,
							client_core::channel_actions::Action::Publish(message),
						) {
							commands.push(command);
						}
						self.publishing = None;
					}
					Some(dialog::Choice::Cancelled) => self.publishing = None,
					None => {}
				}
			}
		}
		if self.deleting.is_some_and(|(channel, message)| {
			state.selected != Some(channel) || state.timeline.get(message).is_none()
		}) {
			self.deleting = None;
		}
		if let Some((channel, message)) = self.deleting {
			let allowed = state.can_delete(channel, message);
			let conversation = state
				.channel(channel)
				.map_or("this conversation", |c| c.name.as_str());
			let mut confirm = dialog::Confirm::new(
				"delete-message",
				"Delete message?",
				format!(
					"This permanently removes the selected message from {conversation} for everyone."
				),
			)
			.danger()
			.confirm_label("Delete")
			.cancel_label("Keep Message")
			.enabled(allowed);
			if !allowed {
				confirm = confirm.note(
					dialog::Level::Warning,
					"Deleting this message is unavailable.",
				);
			}
			match confirm.show(&ctx) {
				Some(dialog::Choice::Confirmed) => {
					if let Some(command) = state.prepare_delete(channel, message) {
						commands.push(command);
					}
					self.deleting = None;
				}
				Some(dialog::Choice::Cancelled) => self.deleting = None,
				None => {}
			}
		}
		self.show_call_switch(&ctx, state, &mut commands);
		self.mentions_inbox(&ctx, state, &mut commands);
		self.apply_ring_request(state, &mut commands);
		if let Some((guild, channel, user, change)) = self.voice_moderation.take()
			&& let Some(command) = state.request_server_admin(
				guild,
				model::server_admin::Action::Voice {
					user,
					channel,
					change,
				},
			) {
			commands.push(command);
		}
		if let Some(notice) = self.mod_view.take_notice() {
			self.toasts
				.push(design::Level::Warning, crate::i18n::translate(notice));
		}
		if let Some(intent) = self.mod_view.show(&ctx, state, &mut commands) {
			self.apply_mod_view_intent(intent, state, &mut commands);
			ctx.request_repaint();
		}
		self.show_force_sync(&ctx, state);
		self.verification.show(&ctx, state);
		self.mfa.show(&ctx, state, &mut commands);
		// Discord's notice for a rate-limited send; the messages stay queued and go out on their own.
		if state.send_throttled {
			let result = dialog::Dialog::new(
				"send-throttled",
				crate::i18n::translate("send-throttled-title"),
			)
			.subtitle(crate::i18n::translate("send-throttled-body"))
			.width(400.0)
			.show(&ctx, |d| {
				let mut close = false;
				d.footer(|ui| {
					close = dialog::action(ui, "send-throttled-dismiss", dialog::Action::Primary)
						.clicked();
				});
				close
			});
			if result.inner || result.close {
				state.dismiss_send_throttle();
			}
		}
		self.onboarding.show(&ctx, state, &mut commands);
		member_guide::show(&ctx, state, &mut self.avatars, &mut commands);
		self.scroll.clear_if_unbound(&ctx);
		self.scroll.paint(&ctx);
		if let Some(gif) = self.timeline.download.gif_favorite_request.take() {
			self.toggle_gif_favorite(&ctx, state, &gif);
		}
		// Clear the title bar and channel header so a notice never sits on the chrome.
		self.toasts
			.show(&ctx, if self.shows_title_bar() { 96.0 } else { 60.0 });
		self.preload.person(self.profile.take_hovered());
		let profile_guild = self.profile_guild(state);
		self.preload.finish(
			&ctx,
			state,
			self.preload_on_hover,
			profile_guild,
			&mut commands,
		);
		if !commands.is_empty() {
			ctx.request_repaint();
		}
		self.retire_menu_paste(state);
		commands
	}
}
