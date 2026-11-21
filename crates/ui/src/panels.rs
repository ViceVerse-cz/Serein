//! Native navigation, member, account and channel panels.
use super::{
	Command, Freshness, Id, MessagingUi, RichText, State, account_badge, account_menu, design,
	header_search_button, i18n, icons, inbox, mentions, name_styles, profiles, shortcuts,
	thread_member_rows, user_menu,
};

impl MessagingUi {
	/// Window title strip: traffic-light inset, centred context title and session state.
	pub(super) fn title_bar(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
		title: &str,
	) {
		let colors = design::palette(ui);
		egui::Panel::top("title-bar")
			.exact_size(design::TITLE_BAR_HEIGHT)
			.show_separator_line(false)
			.frame(egui::Frame::new().fill(design::section_surface(
				ui,
				design::window_palette(ui).base,
				design::ImageSection::TopBar,
			)))
			.show(ui, |ui| {
				// Caption text belongs to the window drag region, not text selection.
				ui.style_mut().interaction.selectable_labels = false;
				let rect = ui.max_rect();
				design::window_drag(ui, rect);
				let title_rect = egui::Rect::from_center_size(
					rect.center(),
					egui::vec2(rect.width() * 0.3, rect.height()),
				);
				ui.scope_builder(
					egui::UiBuilder::new()
						.max_rect(egui::Rect::from_min_max(
							egui::pos2(rect.left() + design::TRAFFIC_LIGHT_INSET + 8.0, rect.top()),
							egui::pos2(title_rect.left() - 10.0, rect.bottom()),
						))
						.layout(egui::Layout::left_to_right(egui::Align::Center)),
					|ui| {
						ui.spacing_mut().item_spacing.x = 4.0;
						if ui
							.add_enabled_ui(
								self.settings.open
									|| self.server_settings.is_open()
									|| state.can_return_from_reply()
									|| state.trail.can_go_back(),
								|ui| icons::button(ui, icons::Icon::ArrowLeft, 28.0, "Back"),
							)
							.inner
							.clicked()
						{
							self.navigate_history(state, commands, true);
						}
						if ui
							.add_enabled_ui(
								!self.settings.open
									&& !self.server_settings.is_open()
									&& state.trail.can_go_forward(),
								|ui| icons::button(ui, icons::Icon::ArrowRight, 28.0, "Forward"),
							)
							.inner
							.clicked()
						{
							self.navigate_history(state, commands, false);
						}
					},
				);
				ui.scope_builder(
					egui::UiBuilder::new().max_rect(title_rect).layout(
						egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
					),
					|ui| {
						ui.add(
							egui::Label::new(design::semibold(ui, title, 13.0).color(colors.text))
								.truncate(),
						);
					},
				);

				let right_inset = if design::WINDOW_CONTROLS_WIDTH > 0.0 {
					0.0
				} else {
					12.0
				};
				ui.scope_builder(
					egui::UiBuilder::new()
						.max_rect(egui::Rect::from_min_max(
							egui::pos2(title_rect.right() + 10.0, rect.top()),
							rect.right_bottom() - egui::vec2(right_inset, 0.0),
						))
						.layout(egui::Layout::right_to_left(egui::Align::Center)),
					|ui| {
						design::window_controls(ui);
						ui.spacing_mut().item_spacing.x = 10.0;
						if self.updates.available || self.updates.ready {
							let (label, icon) = if self.updates.ready {
								(
									"updates-shows-update-banner-restart-to-update",
									icons::Icon::Reload,
								)
							} else if self.updates.busy {
								(
									"updates-shows-update-banner-updating",
									icons::Icon::Download,
								)
							} else {
								(
									"updates-shows-update-banner-update-available",
									icons::Icon::Download,
								)
							};
							let label = crate::i18n::translate(label);
							let font = egui::FontId::new(12.0, design::medium_family(ui.ctx()));
							let galley =
								ui.painter()
									.layout_no_wrap(label.clone(), font, colors.accent);
							let icon_size = 13.0;
							let gap = 5.0;
							let pad = egui::vec2(6.0, 2.0);
							let size = egui::vec2(
								galley.size().x + icon_size + gap + pad.x * 2.0,
								galley.size().y.max(icon_size) + pad.y * 2.0,
							);
							let (rect, response) =
								ui.allocate_exact_size(size, egui::Sense::click());
							ui.painter().rect_filled(
								rect,
								255,
								colors.accent.gamma_multiply(if response.hovered() {
									0.24
								} else {
									0.16
								}),
							);
							let icon_rect = egui::Rect::from_center_size(
								egui::pos2(rect.left() + pad.x + icon_size / 2.0, rect.center().y),
								egui::Vec2::splat(icon_size),
							);
							icons::paint(ui.painter(), icon, icon_rect, colors.accent);
							ui.painter().galley(
								egui::pos2(
									icon_rect.right() + gap,
									rect.center().y - galley.size().y / 2.0,
								),
								galley,
								colors.accent,
							);
							if response
								.on_hover_text(crate::i18n::translate_if_key(&self.updates.status))
								.clicked()
							{
								self.open_update_settings();
							}
						} else {
							design::build_badge(ui, self.build);
						}
						if state.demo && !self.updates.available && !self.updates.ready {
							egui::Frame::new()
								.stroke(egui::Stroke::new(1.0, colors.border))
								.corner_radius(10)
								.inner_margin(egui::Margin::symmetric(8, 3))
								.show(ui, |ui| {
									ui.label(
										design::semibold(
											ui,
											crate::i18n::translate("lib-title-bar-offline-preview"),
											10.0,
										)
										.color(colors.muted),
									);
								})
								.response
								.on_hover_text(crate::i18n::translate(
									"lib-title-bar-synthetic-data-no-network-or-local-storage",
								));
						}
						if !state.demo
							&& state.auth != client_core::auth::AuthState::Authenticated
							&& ui
								.small_button(crate::i18n::translate("lib-title-bar-sign-in-again"))
								.clicked()
						{
							self.reconnect_requested = true;
						}
					},
				);
			});
	}
	pub(super) fn member_rows(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let language = self.language;
		let cached = state.members_cached();
		let Some(list) = state
			.members
			.as_ref()
			.filter(|list| Some(list.channel) == state.selected)
		else {
			ui.add_space(8.0);
			ui.label(
				RichText::new(language.text("choose-conversation-people")).color(colors.muted),
			);
			return;
		};
		let has_entry = list.slots.iter().any(|slot| slot.is_some()) || cached;
		if list.guild.is_none()
			&& state
				.channel(list.channel)
				.is_some_and(|channel| channel.kind == 3)
			&& (list.freshness == Freshness::Fresh || has_entry)
		{
			let text = format!("{} — {}", language.text("members-heading"), list.total);
			egui::Frame::new()
				.inner_margin(egui::Margin::same(8))
				.show(ui, |ui| {
					ui.add(
						egui::Label::new(design::medium(ui, &text, 12.0).color(colors.muted))
							.truncate(),
					)
					.on_hover_text(&text);
				});
		}
		if !has_entry {
			ui.add_space(8.0);
			if list.freshness != Freshness::Fresh {
				let text = if list.freshness == Freshness::Unavailable {
					language.text("people-unavailable")
				} else {
					language.text("loading-people")
				};
				ui.label(RichText::new(text).small().color(colors.muted));
			} else {
				ui.label(
					RichText::new(language.text("no-people"))
						.small()
						.color(colors.muted),
				);
			}
		}
		// Thread snapshots contain people only; paged channel lists supply their own headers.
		let thread_rows = (!list.lazy
			&& state
				.channel(list.channel)
				.is_some_and(|channel| matches!(channel.kind, 10..=12)))
		.then(|| thread_member_rows(state, list));
		let channel = list.channel;
		let lazy = list.lazy;
		let start = list.start;
		let guild = list.guild;
		let voice_users = profiles::voice_users(state);
		let guild_or_channel = guild.unwrap_or(channel);
		let member_key = (state.generation, guild_or_channel, list.request);
		let reset_scroll = self.member_extent.is_none_or(|(key, _)| key != member_key);
		if reset_scroll {
			self.member_extent = Some((member_key, 100));
		}
		let total = list.total.min(250_000) as usize;
		let row_count = if lazy {
			self.member_extent.unwrap().1.min(total)
		} else {
			thread_rows.as_ref().map_or(list.slots.len(), Vec::len)
		};
		let row_spacing = ui.spacing().item_spacing.y;
		ui.spacing_mut().item_spacing.y = 0.0;
		let mut visible = 0usize..0;
		let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
		if reset_scroll {
			area = area.vertical_scroll_offset(0.0);
		}
		let output = self
			.scroll
			.attach(ui, ("people", guild_or_channel, list.request), area)
			.show_rows(ui, 42.0, row_count, |ui, range| {
				visible = range.clone();
				for index in range {
					let slot = if lazy {
						state.member_slot(index).or_else(|| {
							if index >= start {
								list.slots.get(index - start).and_then(|slot| slot.as_ref())
							} else {
								None
							}
						})
					} else if let Some(rows) = &thread_rows {
						rows.get(index).map(|row| row.0.as_ref())
					} else {
						list.slots.get(index).and_then(|s| s.as_ref())
					};
					match slot {
						Some(model::MemberSlot::Group(id)) => {
							let name = match id.as_str() {
								"online" => language.text("status-online"),
								"offline" => language.text("status-offline"),
								_ => {
									let role = id.parse::<u64>().ok().and_then(|role_id| {
										guild.and_then(|guild| {
											state.guild_roles(guild).and_then(|roles| {
												roles.iter().find(|role| role.id == Id(role_id))
											})
										})
									});
									match role {
										Some(role) if !role.name.is_empty() => role.name.clone(),
										_ => language.text("role"),
									}
								}
							};
							let text = list
								.groups
								.iter()
								.find(|(group_id, _)| group_id == id)
								.map(|(_, count)| *count)
								.or_else(|| thread_rows.as_ref().and_then(|rows| rows[index].1))
								.map(|count| format!("{name} - {count}"))
								.unwrap_or(name);
							let (rect, _) = ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::hover(),
							);
							let mut header = ui.new_child(
								egui::UiBuilder::new()
									.max_rect(egui::Rect::from_min_max(
										rect.left_top() + egui::vec2(8.0, 16.0),
										rect.right_bottom() - egui::vec2(8.0, 0.0),
									))
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							header
								.add(
									egui::Label::new(crate::role_names::galley(
										&header,
										&text,
										egui::FontId::new(12.0, design::medium_family(ui.ctx())),
										// Group headers stay neutral; role colours belong to names.
										None,
										colors.sidebar,
										colors.text,
										header.available_width(),
									))
									.truncate(),
								)
								.on_hover_text(&text);
						}
						Some(model::MemberSlot::Person(member)) => {
							let name = state
								.member_nickname(channel, member.user.id)
								.unwrap_or(member.nick.as_deref())
								.filter(|nick| !nick.is_empty())
								.unwrap_or_else(|| state.user_display_name(&member.user));
							let (status, custom, activities, clients) =
								profiles::member_presence(state, member, guild);
							let subtitle = profiles::subtitle(custom, activities);
							let in_voice = voice_users.contains(&member.user.id);
							let voice_label = in_voice.then(|| language.text("member-in-voice"));
							let online =
								status.is_some_and(|s| matches!(s, "online" | "idle" | "dnd"));
							let (rect, response) = ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::click(),
							);
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									true,
									format!(
										"{name}, {}{}{}",
										status.map_or_else(
											|| crate::i18n::translate(
												"friends-presence-unavailable"
											),
											profiles::presence_label,
										),
										subtitle
											.as_ref()
											.map_or_else(String::new, |text| format!(", {text}")),
										voice_label
											.as_ref()
											.map_or_else(String::new, |text| format!(", {text}"))
									),
								)
							});
							let row = rect.shrink2(egui::vec2(0.0, 1.0));
							let focused = response.hovered() || response.has_focus();
							if focused {
								ui.painter().rect_filled(
									row,
									6,
									design::row_highlight(ui, colors.hover, 1.0),
								);
							}
							self.avatars.paint_nameplate(
								ui,
								&member.user,
								row,
								6,
								focused,
								!online,
								state.demo,
							);
							let mut inner = ui.new_child(
								egui::UiBuilder::new()
									.max_rect(row.shrink2(egui::vec2(8.0, 4.0)))
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							inner.spacing_mut().item_spacing.x = 12.0;
							let animate_avatar = ui.rect_contains_pointer(rect);
							inner.push_id(member.user.id.0, |ui| {
								let avatar = self
									.avatars
									.with_avatar_animation(animate_avatar, |avatars| {
										avatars.show(ui, &member.user, 32.0, state.demo)
									});
								user_menu::show(
									&avatar,
									state,
									&member.user,
									&mut self.profile,
									&mut self.user_action,
								);
								if let Some(status) = status {
									profiles::presence_badge(
										ui,
										avatar.rect,
										status,
										clients,
										colors.sidebar,
									);
								}
								let role_ids = state.display_member_role_ids(channel, member);
								let role_colors = online
									.then(|| {
										guild.and_then(|guild| {
											state
												.roles_for_display(guild, role_ids)
												.1
												.map(|role| role.colors())
										})
									})
									.flatten();
								let text_color = if online { colors.text } else { colors.muted };
								let icon_role = guild
									.and_then(|guild| state.guild_roles(guild))
									.and_then(|roles| mentions::member_icon_role(roles, role_ids));
								let mut show_name = |ui: &mut egui::Ui| {
									ui.allocate_ui_with_layout(
										egui::vec2(ui.available_width(), 18.0),
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											ui.spacing_mut().item_spacing.x = 5.0;
											let server_tag = member.user.primary_guild.as_deref();
											let trailing =
												profiles::server_tag_width(ui, server_tag)
													+ if server_tag.is_some() { 5.0 } else { 0.0 }
													+ if icon_role.is_some() { 21.0 } else { 0.0 };
											crate::name_styles::in_row(
												response.contains_pointer() || response.has_focus(),
												|| {
													account_badge::name(
														ui,
														&member.user,
														name,
														15.0,
														(
															text_color,
															role_colors,
															if response.hovered()
																|| response.has_focus()
															{
																colors.hover
															} else {
																colors.sidebar
															},
														),
														egui::Sense::hover(),
														trailing,
													)
												},
											);
											if let Some(role) = icon_role {
												mentions::show_role_icon(
													ui,
													&mut self.avatars,
													role,
													16.0,
													state.demo,
												);
											}
											if let Some(tag) = server_tag {
												profiles::server_tag(
													ui,
													tag,
													&mut self.avatars,
													state.demo,
												);
											}
										},
									);
								};
								if subtitle.is_some() || in_voice {
									ui.vertical(|ui| {
										ui.spacing_mut().item_spacing.y = 1.0;
										ui.spacing_mut().interact_size.y = 0.0;
										show_name(ui);
										ui.allocate_ui_with_layout(
											egui::vec2(ui.available_width(), 14.0),
											egui::Layout::left_to_right(egui::Align::Center),
											|ui| {
												ui.spacing_mut().item_spacing.x = 4.0;
												profiles::voice_badge(
													ui,
													in_voice,
													subtitle.is_some(),
												);
												if let Some(subtitle) = subtitle {
													if activities
														.first()
														.is_some_and(profiles::is_spotify)
													{
														icons::inline(
															ui,
															icons::Icon::Spotify,
															12.0,
															colors.positive,
														);
													}
													ui.add(
														egui::Label::new(
															RichText::new(subtitle)
																.size(12.0)
																.color(colors.muted),
														)
														.truncate()
														.selectable(false),
													);
												}
											},
										);
									});
								} else {
									show_name(ui);
								}
								user_menu::show(
									&response,
									state,
									&member.user,
									&mut self.profile,
									&mut self.user_action,
								);
								self.profile.person_click(
									ui,
									&response,
									Some(&avatar),
									&member.user,
								);
							});
						}
						None => {
							ui.allocate_exact_size(
								egui::vec2(ui.available_width(), 42.0),
								egui::Sense::hover(),
							);
						}
					}
				}
			});
		ui.spacing_mut().item_spacing.y = row_spacing;
		if lazy && !visible.is_empty() {
			let first = visible.start;
			let mut last = visible.end.saturating_sub(1);
			if row_count < total
				&& output.state.clamped_offset().y > 0.0
				&& output.state.clamped_offset().y + output.inner_rect.height()
					>= output.content_size.y - 1.0
			{
				// Keep requesting the next chunk while at the bottom, without exposing
				// another page of empty rows before its first entry arrives.
				last = row_count;
				if state.member_slot(row_count).is_some()
					|| row_count
						.checked_sub(start)
						.is_some_and(|index| list.slots.get(index).is_some_and(Option::is_some))
				{
					self.member_extent = Some((member_key, (row_count + 100).min(total)));
					ui.ctx().request_repaint();
				}
			}
			if let Some(cmd) = state.focus_member_ranges(first, last) {
				commands.push(cmd);
			}
		}
	}
	/// Channel sidebar: context header, scrolling list and the account card.
	pub(super) fn sidebar(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		title: &str,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let language = self.language;
		egui::Panel::top("sidebar-header")
			.exact_size(48.0)
			.show_separator_line(false)
			.frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 0)))
			.show(ui, |ui| {
				let rect = ui.max_rect();
				ui.horizontal_centered(|ui| {
					if let Some(guild) = self.guild {
						self.server_menu.header(ui, state, guild, title);
						return;
					}
					ui.add(
						egui::Label::new(
							design::semibold(ui, title, 15.0).color(colors.text_strong),
						)
						.truncate()
						.selectable(false),
					);
				});
				ui.painter().hline(
					rect.x_range().expand(16.0),
					rect.bottom(),
					egui::Stroke::new(1.0, colors.border),
				);
			});
		egui::Frame::new()
			.inner_margin(egui::Margin {
				left: 8,
				right: 8,
				top: 8,
				bottom: 0,
			})
			.show(ui, |ui| {
				let ctx = ui.ctx().clone();
				if ui
					.add_sized(
						[ui.available_width(), 30.0],
						egui::Button::new(language.text("inbox-title")),
					)
					.clicked()
				{
					self.inbox_open = true;
					commands.push(state.clear_mentions());
					let command = match self.inbox_controls.tab {
						inbox::Tab::Unread => None,
						inbox::Tab::Mentions => state.request_mentions(None),
						inbox::Tab::Saved => {
							state.request_inbox_action(client_core::inbox::Action::Saved)
						}
					};
					commands.extend(command);
				}
				if let Some(guild) = self.guild
					&& ui
						.add_sized(
							[ui.available_width(), 30.0],
							egui::Button::new("Browse Channels")
								.selected(state.browse_channels == Some(guild)),
						)
						.clicked()
				{
					self.browse_channels.open(state, guild);
				}
				if let Some(guild) = self.guild
					&& !state.channel_preferences_known(guild)
				{
					ui.small("Channel list not synced").on_hover_text(
						"Open Browse Channels to load or retry your saved channel list.",
					);
				}
				if let Some(guild) = self.guild
					&& state.browse_channels != Some(guild)
				{
					crate::browse_channels::preference_feedback(ui, state, guild, None, commands);
				}
				ui.add_space(6.0);
				if self.guild.is_none() {
					// Search and Friends share one row; Friends is the compact glyph beside it.
					ui.horizontal(|ui| {
						ui.spacing_mut().item_spacing.x = 6.0;
						let find = ui
							.add_enabled_ui(
								!self.ime_active
									&& !ctx.input(|input| {
										input
											.events
											.iter()
											.any(|event| matches!(event, egui::Event::Ime(_)))
									}),
								|ui| {
									ui.add_sized(
										[(ui.available_width() - 36.0).max(60.0), 30.0],
										egui::Button::new(language.text("find-conversation"))
											.truncate(),
									)
								},
							)
							.inner
							.on_hover_text(language.text("find-conversation-tooltip"));
						find.widget_info(|| {
							egui::WidgetInfo::labeled(
								egui::Role::Button,
								ui.is_enabled(),
								language.text("find-conversation"),
							)
						});
						if find.clicked() {
							self.switcher.open(&ctx);
						}
						let friends = state.selected.is_none();
						let (rect, response) =
							ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
						let hovered = response.hovered() || response.has_focus();
						if friends || hovered {
							ui.painter().rect_filled(
								rect,
								6,
								if friends {
									colors.selected
								} else {
									colors.hover
								},
							);
						}
						icons::paint(
							ui.painter(),
							icons::Icon::People,
							rect.shrink(7.0),
							if friends || hovered {
								colors.text_strong
							} else {
								colors.muted
							},
						);
						response.widget_info(|| {
							egui::WidgetInfo::selected(
								egui::Role::Button,
								true,
								friends,
								language.text("friends"),
							)
						});
						if response.on_hover_text(language.text("friends")).clicked() {
							state.open_home();
							self.guild = None;
							self.search.open = false;
						}
					});
					ui.add_space(8.0);
				}
				if let Some(guild) = self.guild
					&& state.can_open_member_settings(guild)
					&& state.server_members_shortcut(guild)
					&& ui
						.add_sized(
							[ui.available_width(), 32.0],
							egui::Button::new(language.text("members")).frame(false),
						)
						.clicked() && let Some(command) =
					self.preview_server_admin(state, guild, "members")
				{
					commands.push(command);
				}
				if !self.channel_preferences_status.is_empty() {
					ui.colored_label(
						design::palette(ui).warning,
						crate::i18n::translate_if_key(self.channel_preferences_status),
					);
					if ui.button(language.text("retry-shortcuts")).clicked() {
						if self.channel_preferences_loaded {
							self.channel_preferences_changed = true;
						} else {
							self.channel_preferences_reload = true;
						}
					}
				}
				if let Some(command) = state.request_sidebar_forum_posts(self.guild) {
					commands.push(command);
				}
				let select = self.channel_list(ui, state);
				if let Some((guild, channel, user)) = self.stream_preview_request.take()
					&& let Some(command) = state.request_stream_preview(guild, channel, user)
				{
					commands.push(command);
				}
				self.apply_stream_preview_watch(state, commands);
				self.apply_watch_request(state);
				if let Some((channel, action)) = self.channel_move.take()
					&& let Some(command) = state.request_channel_action(channel, action)
				{
					commands.push(command);
				}
				if let Some(id) = select
					&& let Some(command) = state.select(id)
				{
					commands.push(command);
				}
				if let Some(parent) = self.archive_parent.take()
					&& let Some(command) =
						state.request_archives(parent, model::archives::Kind::Public, None)
				{
					self.search.open = false;
					self.archives.focus = true;
					commands.push(command);
				}
			});
	}
	/// Account card; while in a call it grows upward with the call header and quick actions.
	pub(super) fn account_card(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let language = self.language;
		let mut anchor = None;
		let in_call = state.voice.active.is_some();
		let (card_fill, card_edge) = design::account_card_surface(ui);
		// A see-through card has no coat of its own; the presence ring cuts out the strip's.
		let ring = if card_fill.a() == 0 {
			design::section_surface(
				ui,
				design::window_palette(ui).base,
				design::ImageSection::ServerList,
			)
		} else {
			card_fill
		};
		let card = egui::Frame::new()
			.fill(card_fill)
			.corner_radius(8)
			.inner_margin(0)
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.spacing_mut().item_spacing.y = 0.0;
				let divider = |ui: &mut egui::Ui| {
					let (line, _) = ui.allocate_exact_size(
						egui::vec2(ui.available_width(), 1.0),
						egui::Sense::hover(),
					);
					ui.painter().rect_filled(line, 0, colors.border);
				};
				if self.shows_update_banner() {
					self.update_banner(ui);
					divider(ui);
				}
				if !state.demo
					&& state.auth == client_core::auth::AuthState::Authenticated
					&& !state.gateway_connected
				{
					let first = !self.shows_update_banner();
					self.reconnecting_banner(ui, first);
					divider(ui);
				}
				if in_call {
					self.voice_card_section(ui, state, commands);
					divider(ui);
				}
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(8, 6))
					.show(ui, |ui| {
						ui.set_width(ui.available_width());
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 8.0;
							if let Some(user) = &state.user {
								let avatar = self.avatars.show(ui, user, 32.0, state.demo);
								avatar.widget_info(|| {
									egui::WidgetInfo::labeled(
										egui::Role::Button,
										true,
										language.text("profile-and-status"),
									)
								});
								if avatar.has_focus() {
									ui.painter().rect_stroke(
										avatar.rect.expand(2.0),
										4,
										egui::Stroke::new(1.0, colors.accent),
										egui::StrokeKind::Inside,
									);
								}
								user_menu::show(
									&avatar,
									state,
									user,
									&mut self.profile,
									&mut self.user_action,
								);
								design::presence_dot(
									ui,
									avatar.rect,
									self.own_presence.status.wire(),
									ring,
								);
								anchor =
									Some(avatar.on_hover_text(language.text("profile-and-status")));
							}
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									ui.spacing_mut().item_spacing.x = 2.0;
									let settings = icons::button(
										ui,
										icons::Icon::Gear,
										32.0,
										&language.text("user-settings"),
									);
									if settings.clicked() {
										self.settings.open = true;
										egui::Popup::close_all(ui.ctx());
									}
									self.mute_toggle_with_settings(ui, state, commands, true);
									ui.add_space(4.0);
									self.mute_toggle_with_settings(ui, state, commands, false);
									ui.with_layout(
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											let identity = ui
												.vertical(|ui| {
													ui.spacing_mut().item_spacing.y = 0.0;
													let name = state.user.as_ref().map_or_else(
														|| language.text("your-account"),
														|u| u.name.clone(),
													);
													name_styles::label(
														ui,
														state
															.user
															.as_ref()
															.and_then(name_styles::style),
														&name,
														egui::FontId::new(
															14.0,
															design::semibold_family(ui.ctx()),
														),
														None,
														card_fill,
														colors.text_strong,
														egui::Sense::hover(),
														true,
													);
													ui.add(
														egui::Label::new(
															RichText::new(
																if !self
																	.own_presence
																	.custom_status
																	.is_empty()
																{
																	self.own_presence
																		.custom_status
																		.clone()
																} else if let Some(game) = self
																	.own_game
																	.as_deref()
																	.filter(|_| {
																		self.share_game_activity
																	}) {
																	game.to_owned()
																} else if state.demo {
																	language.text("offline-preview")
																} else if state.gateway_connected {
																	language.text(account_menu::presence_status_key(self.own_presence.status))
																} else {
																	language.text("reconnecting")
																},
															)
															.size(12.0)
															.color(colors.muted),
														)
														.truncate()
														.selectable(false),
													);
												})
												.response;
											let identity = ui
												.interact(
													identity.rect,
													ui.scope_id().with("account-identity"),
													egui::Sense::click(),
												)
												.on_hover_text(language.text("profile-and-status"));
											identity.widget_info(|| {
												egui::WidgetInfo::labeled(
													egui::Role::Button,
													true,
													language.text("profile-and-status"),
												)
											});
											if let Some(avatar) = anchor.take() {
												if identity.has_focus() {
													ui.painter().rect_stroke(
														identity.rect.expand(2.0),
														4,
														egui::Stroke::new(1.0, colors.accent),
														egui::StrokeKind::Inside,
													);
												}
												anchor = Some(avatar.union(identity));
											}
										},
									);
								},
							);
						});
					});
			});
		// Painted over the frame rather than as its stroke, so the card keeps the composer's height.
		ui.painter()
			.rect_stroke(card.response.rect, 8, card_edge, egui::StrokeKind::Inside);
		if let Some(anchor) = anchor {
			self.account_menu(&anchor, state, commands);
		}
	}
	/// Conversation header: channel identity on the left, tools and search on the right.
	#[allow(clippy::too_many_arguments)]
	pub(super) fn channel_header(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		selected_voice: bool,
		show_members: bool,
		wide_members: bool,
		search_open: bool,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Panel::top("channel-header")
			.exact_size(48.0)
			.show_separator_line(false)
			.frame(
				egui::Frame::new()
					// Like the composer, the header shares the conversation's single coat.
					.fill(if design::has_section_background(ui) {
						design::section_surface(
							ui,
							design::window_palette(ui).chat,
							design::ImageSection::TopBar,
						)
					} else if search_open {
						// Spanning the search pane, the header sits outside the central panel,
						// so it paints the chat coat that panel would otherwise provide.
						design::window_palette(ui).chat
					} else {
						egui::Color32::TRANSPARENT
					})
					.inner_margin(egui::Margin::symmetric(16, 0)),
			)
			.show(ui, |ui| {
				let rect = ui.max_rect();
				ui.painter().hline(
					rect.x_range().expand(16.0),
					rect.bottom(),
					egui::Stroke::new(1.0, colors.border),
				);
				design::destination_signal(
					ui,
					egui::Id::unique("channel-header"),
					egui::Id::unique(state.selected),
					rect.translate(egui::vec2(0.0, -1.0)),
				);
				self.channel_header_row(
					ui,
					state,
					selected_voice,
					show_members,
					wide_members,
					commands,
				);
			});
	}
	/// The header's avatar, name and actions; the DM call stage draws it over the call.
	pub(crate) fn channel_header_row(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		selected_voice: bool,
		show_members: bool,
		wide_members: bool,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let language = self.language;
		let channel = state.selected.and_then(|id| state.channel(id)).cloned();
		let dm = channel
			.as_ref()
			.is_some_and(|c| c.kind == 1 && c.guild.is_none());
		let shortcuts_available = self.shortcuts_available(state);
		ui.horizontal_centered(|ui| {
			ui.spacing_mut().item_spacing.x = 8.0;
			let voice_chat_label = language.text(if self.voice_chat_open {
				"hide-chat"
			} else {
				"show-chat"
			});
			match channel.as_ref() {
				Some(c) if c.guild.is_none() && c.kind == 3 => {
					let avatar = self.avatars.show_group(ui, c, 24.0, state.demo);
					self.group_menu.context(
						&avatar,
						state,
						c,
						shortcuts::ShortcutView::new(
							&self.channel_preferences,
							shortcuts_available,
						),
					);
				}
				Some(c) if c.guild.is_none() => {
					if let Some(user) = c.recipients.first() {
						// Header avatar identifies the conversation; the profile is a
						// context-menu action, not a click target.
						let avatar = self.avatars.show_plain(ui, user, 24.0, state.demo);
						if dm {
							user_menu::show_with_pin(
								&avatar,
								state,
								user,
								&mut self.profile,
								&mut self.user_action,
								Some(shortcuts::ShortcutView::new(
									&self.channel_preferences,
									shortcuts_available,
								)),
							);
						}
						let (status, _, _, clients) = profiles::presence(state, user.id, None);
						let status = profiles::shown_status(state, user.id, status);
						if dm && let Some(status) = status {
							profiles::presence_badge(
								ui,
								avatar.rect,
								status,
								clients,
								colors.sidebar,
							);
						}
					} else {
						icons::inline(ui, icons::Icon::People, 22.0, colors.muted);
					}
				}
				Some(c) => {
					let icon = match c.kind {
						2 | 13 => icons::Icon::Speaker,
						5 => icons::Icon::Megaphone,
						15 | 16 => icons::Icon::Forum,
						10..=12 => icons::Icon::Thread,
						_ => icons::Icon::Hash,
					};
					icons::inline(ui, icon, 22.0, colors.muted);
				}
				None => {}
			}
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				ui.spacing_mut().item_spacing.x = 4.0;
				if selected_voice
					&& icons::toggle(
						ui,
						icons::Icon::Forum,
						32.0,
						self.voice_chat_open,
						&voice_chat_label,
					)
					.clicked()
				{
					self.voice_chat_open = !self.voice_chat_open;
					self.focus_switched_composer = self.voice_chat_open;
				}
				if let Some(c) = channel
					.as_ref()
					.filter(|c| c.guild.is_none() && c.kind == 3)
				{
					self.group_menu.dropdown(
						ui,
						state,
						c,
						shortcuts::ShortcutView::new(
							&self.channel_preferences,
							shortcuts_available,
						),
					);
				}
				if state.selected.is_some() && !selected_voice {
					if let Some(source) = channel
						.as_ref()
						.filter(|channel| channel.kind == 5 && channel.guild.is_some())
						&& icons::button(
							ui,
							icons::Icon::Megaphone,
							32.0,
							&i18n::translate("announcement-follow-title"),
						)
						.clicked()
					{
						state.clear_channel_action_result(source.id);
						self.announcement_follow.open(state, source.id);
					}
					if self.search.open && !self.search.pins() {
						ui.allocate_ui_with_layout(
							egui::vec2(240.0_f32.min(ui.available_width() * 0.5).max(120.0), 28.0),
							egui::Layout::left_to_right(egui::Align::Center),
							|ui| self.search.header_input(ui, state, commands),
						);
					} else {
						if header_search_button(ui, state.can_search(), &language.text("search"))
							.on_hover_text(language.text("search-conversation"))
							.clicked()
						{
							if state.archives.is_some() {
								commands.push(state.clear_archives());
							}
							self.search.toggle(false);
						}
					}
					ui.add_space(4.0);
					if icons::toggle(
						ui,
						icons::Icon::People,
						32.0,
						show_members,
						&language.text("show-member-list"),
					)
					.clicked()
					{
						if wide_members {
							let shown = self.members_preference(state);
							*shown = !*shown;
						} else {
							self.members_narrow_open = !self.members_narrow_open;
						}
					}
					let pins_open = self.search.open && self.search.pins();
					let pins = ui
						.add_enabled_ui(state.can_search() || pins_open, |ui| {
							icons::toggle(
								ui,
								icons::Icon::Pin,
								32.0,
								pins_open,
								&language.text("pinned-messages"),
							)
						})
						.inner;
					if pins.clicked()
						&& self.search.toggle(true)
						&& let Some(command) = state.request_pins()
					{
						commands.push(command);
					}
					self.pins_anchor =
						(self.search.open && self.search.pins()).then_some(pins.rect);
					if let Some(c) = channel
						.as_ref()
						.filter(|c| c.guild.is_some() && matches!(c.kind, 0 | 5 | 15 | 16))
					{
						let allowed = state.can_archive(c.id, model::archives::Kind::Public)
							|| state.can_create_thread(c.id)
							|| state.can_create_private_thread(c.id);
						let archive = ui
							.add_enabled_ui(allowed, |ui| {
								icons::button(
									ui,
									icons::Icon::Thread,
									32.0,
									&language.text("threads"),
								)
							})
							.inner;
						if archive.clicked() {
							if state.can_archive(c.id, model::archives::Kind::Public) {
								self.archive_parent = Some(c.id);
							} else {
								self.thread_create.open(state, c.id, None, String::new());
							}
						}
					}
					let can_reload = state.selected.is_some_and(|id| {
						if state.is_forum(id) {
							state.can_load_posts(id) && !state.posts.loading
						} else {
							state.freshness != Freshness::Loading && state.can_read_history(id)
						}
					});
					let can_reconnect = state.auth == client_core::auth::AuthState::Authenticated
						&& !state.gateway_connected;
					let reload = ui
						.add_enabled_ui(can_reload || can_reconnect, |ui| {
							icons::button(
								ui,
								icons::Icon::Reload,
								32.0,
								&language.text("reload-history"),
							)
						})
						.inner;
					if reload.clicked() {
						if can_reconnect {
							self.reconnect_requested = true;
						}
						if can_reload {
							commands.push(state.history(None));
							self.timeline.follow_latest(state);
						}
					}
				}
				if let Some(channel) = state.selected.filter(|_| {
					channel
						.as_ref()
						.is_some_and(|c| c.guild.is_none() && matches!(c.kind, 1 | 3))
				}) {
					self.voice_settings(ui, state.demo, state.voice.active.is_some());
					self.call_button(ui, state, channel, commands);
				}
				ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
					// Centre the name block in the fixed-height header even without a subtitle.
					let name = channel.as_ref().map_or_else(
						|| language.text("direct-messages"),
						|c| state.conversation_name(c).to_owned(),
					);
					let subtitle = channel
						.as_ref()
						.filter(|_| dm)
						.and_then(|c| c.recipients.first())
						.and_then(|user| {
							let (_, custom, activities, _) =
								profiles::presence(state, user.id, None);
							profiles::subtitle(custom, activities)
						});
					let in_call = state.voice.active.as_ref().is_some_and(|call| {
						Some(call.channel) == state.selected
							&& matches!(
								call.phase,
								client_core::voice::Phase::Connected
									| client_core::voice::Phase::Waiting
							)
					});
					// The "In a call" label beside the name already says it while we share the call.
					let in_voice = !in_call
						&& channel
							.as_ref()
							.filter(|_| dm)
							.and_then(|channel| channel.recipients.first())
							.is_some_and(|user| profiles::user_in_voice(state, user.id));
					let name_height = ui
						.painter()
						.layout_no_wrap(
							name.to_owned(),
							egui::FontId::new(16.0, design::semibold_family(ui.ctx())),
							colors.text_strong,
						)
						.size()
						.y;
					let subtitle_height = subtitle.as_ref().map_or(0.0, |text| {
						1.0 + ui
							.painter()
							.layout_no_wrap(
								text.clone(),
								egui::FontId::proportional(12.0),
								colors.muted,
							)
							.size()
							.y
					});
					let subtitle_height = if in_voice {
						subtitle_height.max(16.0)
					} else {
						subtitle_height
					};
					ui.vertical(|ui| {
						ui.add_space(
							((ui.available_height() - name_height - subtitle_height) / 2.0)
								.max(0.0),
						);
						ui.spacing_mut().item_spacing.y = 1.0;
						// Keep the status row at its text height so the measured centring holds.
						ui.spacing_mut().interact_size.y = 0.0;
						ui.add(
							egui::Label::new(
								design::semibold(ui, name, 16.0).color(colors.text_strong),
							)
							.truncate(),
						);
						{
							if subtitle.is_some() || in_voice {
								ui.horizontal(|ui| {
									ui.spacing_mut().item_spacing.x = 4.0;
									profiles::voice_badge(ui, in_voice, subtitle.is_some());
									if let Some(text) = subtitle {
										ui.add(
											egui::Label::new(
												RichText::new(&text).size(12.0).color(colors.muted),
											)
											.truncate(),
										)
										.on_hover_text(text);
									}
								});
							}
						}
					});
					if in_call {
						ui.add_space(4.0);
						icons::inline(ui, icons::Icon::InCall, 16.0, colors.positive);
						ui.add(
							egui::Label::new(
								design::medium(ui, language.text("in-a-call"), 14.0)
									.color(colors.positive),
							)
							.selectable(false),
						);
					}
				});
			});
		});
	}
}
