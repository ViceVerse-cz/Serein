//! Paint one message inside the caller's unchanged row scope.
use super::{
	DeletedLocalAction, Formatted, Id, LazyHover, MESSAGE_LINE, Message, Revealed, RichText, State,
	TimelineView, action_button, clamp_hover_toolbar, compact_author_width, compact_header,
	copy_action_button, deleted_message_actions, discord_url, divider, fill_header_line, group_gap,
	mentions_viewer, message_actions, queue_missing_channel_reference, quick_reaction,
	quick_reaction_emoji, reference_spine, show_system, system_icon, thread_card, timestamp,
};

pub(super) struct MessageRow<'a> {
	pub(super) message: &'a Message,
	pub(super) id: Id,
	pub(super) compact: bool,
	pub(super) new_day: bool,
	pub(super) deleted: bool,
	pub(super) can_mark_read: bool,
	pub(super) can_mark_unread: bool,
	pub(super) name_hover: Option<Id>,
	pub(super) allow_hover: bool,
	pub(super) retained_toolbar: Option<(Id, egui::Rect)>,
	pub(super) keyboard_focus: &'a Option<egui::Response>,
	pub(super) context_key: bool,
	pub(super) clip: egui::Rect,
}

pub(super) struct RowActions<'a> {
	pub(super) editing: &'a mut Option<(Id, Id, String)>,
	pub(super) deleting: &'a mut Option<(Id, Id)>,
	pub(super) selected_reply: &'a mut Option<Id>,
}

impl TimelineView {
	pub(super) fn paint_message_row(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		row: MessageRow<'_>,
		(avatars, profile): (
			&mut crate::avatars::Avatars,
			&mut crate::profiles::ProfileSession,
		),
		actions: RowActions<'_>,
	) {
		let RowActions {
			editing,
			deleting,
			selected_reply,
		} = actions;
		let MessageRow {
			message,
			id,
			compact,
			new_day,
			deleted,
			can_mark_read,
			can_mark_unread,
			name_hover,
			allow_hover,
			retained_toolbar,
			keyboard_focus,
			context_key,
			clip,
		} = row;

		if new_day {
			let date = timestamp(id);
			divider(
				ui,
				format!("{} {}, {}", date.month(), date.day(), date.year()),
				false,
				self.compact_messages,
			);
		}
		if self.unread_boundary == Some(id) {
			divider(ui, "New messages".into(), true, self.compact_messages);
		}
		if !self
			.restricted
			.show(ui, state, crate::restricted::Key::message(message))
		{
			self.revision = u64::MAX;
			self.heights.remove(&id);
			return;
		}
		let colors = crate::design::palette(ui);
		let background = ui.painter().add(egui::Shape::Noop);
		let mut time_rect = None;
		// A reaction claims its own right click: the row menu must stay closed.
		let mut reaction_menu = false;
		// A double-click away from text, links and media uses the selected reaction preset.
		let mut double_click = None;
		let row = egui::Frame::NONE
			.inner_margin(egui::Margin {
				left: 16,
				right: 16,
				top: if self.compact_messages {
					3
				} else if compact {
					1
				} else {
					group_gap(self.compact_messages)
				},
				bottom: 1,
			})
			.show(ui, |ui| {
				let mut surface = crate::select::Surface::new(ui, "row");
				ui.spacing_mut().item_spacing =
					egui::vec2(16.0, if self.compact_messages { 2.0 } else { 4.0 });
				if let Some(interaction) = message
					.interaction
					.as_deref()
					.filter(|_| message.reply_to.is_none())
				{
					ui.horizontal(|ui| {
						ui.spacing_mut().interact_size.y = 18.0;
						ui.spacing_mut().item_spacing.x = 6.0;
						reference_spine(ui, &colors);
						let avatar = avatars.show(ui, &interaction.user, 16.0, state.demo);
						surface.keep(&avatar);
						let name = ui.add(
							egui::Label::new(
								RichText::new(&interaction.user.name)
									.size(13.0)
									.family(crate::design::semibold_family(ui.ctx()))
									.color(colors.text),
							)
							.truncate(),
						);
						surface.keep(&name);
						let used = ui.add(
							egui::Label::new(
								RichText::new(crate::i18n::translate("timeline-command-used"))
									.size(13.0)
									.color(colors.muted),
							)
							.truncate(),
						);
						surface.keep(&used);
						let label = if interaction.command.is_empty() {
							crate::i18n::translate("timeline-command-a-command")
						} else {
							format!("/{}", interaction.command)
						};
						let command = egui::Frame::NONE
							.fill(colors.mention_bg)
							.corner_radius(3)
							.inner_margin(egui::Margin::symmetric(4, 0))
							.show(ui, |ui| {
								ui.add(
									egui::Label::new(
										RichText::new(label)
											.size(13.0)
											.family(crate::design::semibold_family(ui.ctx()))
											.color(colors.mention_text),
									)
									.truncate(),
								)
							})
							.inner;
						surface.keep(&command);
					});
				}
				// A poll result's reference opens through its result card instead.
				if let Some(reply) = message.reply_to.filter(|_| message.kind != 46) {
					ui.horizontal(|ui| {
						ui.spacing_mut().interact_size.y = 18.0;
						ui.spacing_mut().item_spacing.x = 6.0;
						reference_spine(ui, &colors);
						// Rendering reads bounded previews. The host schedules visible lookups
						// after layout, without inserting their originals into this window.
						if message.reply_deleted
							|| state.reply_preview_status(reply)
								== client_core::reply_previews::Status::Deleted
						{
							let deleted = ui.add(
								egui::Label::new(
									RichText::new(crate::i18n::translate(
										"timeline-show-with-scroll-message-deleted",
									))
									.size(13.0)
									.italics()
									.color(colors.muted),
								)
								.truncate(),
							);
							surface.keep(&deleted);
						} else if let Some(restriction) = state
							.reply_original(reply)
							.and_then(|original| state.user_content_restriction(original.author.id))
						{
							let response = ui.link(crate::restricted::label(restriction));
							if response.clicked() && state.can_open_reply_target(reply) {
								self.request_reply_target(reply);
							}
							surface.keep(&response);
						} else {
							ui.scope(|ui| {
								ui.visuals_mut().disabled_alpha = 1.0;
								ui.add_enabled_ui(state.can_open_reply_target(reply), |ui| {
									let mut preview = egui::text::LayoutJob::default();
									let mut preview_emojis = Vec::new();
									let mut attachment_icon = None;
									if let Some(original) = state.reply_original(reply) {
										let reply_avatar =
											avatars.show(ui, &original.author, 16.0, state.demo);
										if reply_avatar.clicked() {
											self.request_reply_target(reply);
										}
										surface.keep(&reply_avatar);
										attachment_icon =
											(!original.attachments.is_empty()).then(|| {
												if original.attachments.iter().any(|attachment| {
													attachment.is_image() || attachment.is_video()
												}) {
													crate::icons::Icon::Image
												} else {
													crate::icons::Icon::File
												}
											});
										preview.append(
											&format!("@{}  ", state.message_author_name(original)),
											0.0,
											egui::TextFormat {
												font_id: egui::FontId::new(
													13.0,
													crate::design::semibold_family(ui.ctx()),
												),
												color: colors.muted,
												..Default::default()
											},
										);
										if crate::embeds::has_spoilers(original) {
											preview.append(
												"Spoiler",
												0.0,
												egui::TextFormat {
													font_id: egui::FontId::proportional(13.0),
													color: colors.muted,
													..Default::default()
												},
											);
										} else if attachment_icon.is_some()
											&& original.content.trim().is_empty()
										{
											preview.append(
												&crate::i18n::translate(
													"timeline-show-with-scroll-click-to-see-attachment",
												),
												0.0,
												egui::TextFormat {
													font_id: egui::FontId::proportional(13.0),
													color: colors.muted,
													italics: true,
													..Default::default()
												},
											);
										} else {
											let source = crate::mentions::MentionSource {
												state,
												channel: original.channel,
											};
											let formatted =
												self.formatted.get(reply, &original.content);
											queue_missing_channel_reference(
												&mut self.channel_reference_load,
												formatted,
												state,
												u32::MAX,
											);
											preview_emojis = formatted.append_inline_preview(
												&mut preview,
												ui,
												&original.mentions,
												Some(&source),
												crate::mentions::known_roles(
													state,
													original.channel,
												),
												&state.channels,
											);
										}
									} else {
										preview.append(
															&crate::i18n::translate(match state.reply_preview_status(reply) {
																client_core::reply_previews::Status::Loading => "reply-preview-loading",
																client_core::reply_previews::Status::Failed => "reply-preview-failed",
																client_core::reply_previews::Status::Forbidden => "reply-preview-forbidden",
																client_core::reply_previews::Status::Unavailable => "reply-preview-unavailable",
																_ => "reply-preview-unrequested",
															}),
															0.0,
															egui::TextFormat {
																font_id: egui::FontId::proportional(
																	13.0,
																),
																color: colors.muted,
																..Default::default()
															},
														);
									}
									let accessible =
										Formatted::inline_preview_text(&preview, &preview_emojis);
									// Keep room for the trailing attachment glyph when truncating.
									let icon_size = 16.0;
									let retry_label = crate::i18n::translate("reply-preview-retry");
									let failed = state.reply_preview_status(reply)
										== client_core::reply_previews::Status::Failed;
									let retry_width = if failed {
										ui.painter()
											.layout_no_wrap(
												retry_label.clone(),
												egui::TextStyle::Button.resolve(ui.style()),
												colors.muted,
											)
											.size()
											.x + ui.spacing().button_padding.x * 2.0
											+ ui.spacing().item_spacing.x
									} else {
										0.0
									};
									let label_width = ui.available_width()
										- retry_width - attachment_icon
										.map_or(0.0, |_| icon_size + ui.spacing().item_spacing.x);
									let (preview_pos, preview_galley, reply_preview) = ui
										.allocate_ui(
											egui::vec2(label_width.max(0.0), ui.available_height()),
											|ui| {
												egui::Label::new(preview)
													.truncate()
													.sense(egui::Sense::click())
													.layout_in_ui(ui)
											},
										)
										.inner;
									if let Some(icon) = attachment_icon {
										crate::icons::inline(ui, icon, icon_size, colors.muted);
									}
									reply_preview.widget_info(|| {
										egui::WidgetInfo::labeled(
											egui::Role::Link,
											ui.is_enabled(),
											&accessible,
										)
									});
									surface.embed(
										&reply_preview,
										preview_pos,
										preview_galley.clone(),
									);
									Formatted::paint_inline_preview_emojis(
										ui,
										preview_pos,
										&preview_galley,
										&preview_emojis,
										avatars,
										state.demo,
									);
									let reply_preview = reply_preview
										.on_hover_cursor(egui::CursorIcon::PointingHand)
										.on_hover_text(crate::i18n::translate(
											"timeline-show-with-scroll-view-original-message",
										))
										.on_disabled_hover_text(crate::i18n::translate(
											"timeline-show-with-scroll-wait-for-readable-current-message-history",
										));
									surface.keep(&reply_preview);
									if failed {
										let retry = ui.small_button(retry_label);
										if retry.clicked() {
											self.reply_retry = Some(reply);
										}
										surface.keep(&retry);
									}
									if reply_preview.clicked() {
										self.request_reply_target(reply);
									}
								});
							});
						}
					});
				}
				let system = message.system_message();
				let author_colors = state.message_author_colors(message);
				let name_color = author_colors.map_or(colors.text_strong, |role| {
					crate::design::role_name_color(role.primary, colors.chat, colors.text_strong)
				});
				let mut body_bottom = f32::NAN;
				// Hovering the avatar underlines the author, like hovering the name.
				let mut avatar_hot = false;
				ui.horizontal_top(|ui| {
					if system.is_some() {
						let (gutter, _) = ui.allocate_exact_size(
							egui::vec2(40.0, MESSAGE_LINE),
							egui::Sense::hover(),
						);
						let (icon, tint) = system_icon(message.kind, &colors);
						crate::icons::paint(
							ui.painter(),
							icon,
							egui::Rect::from_center_size(
								gutter.center() + egui::vec2(0.0, 1.5),
								egui::Vec2::splat(18.0),
							),
							tint,
						);
					} else if self.compact_messages {
						// Compact rows drop the avatar gutter: time, author, then
						// the body in its own column so wrapped lines never run under the name.
						let body_spacing = ui.spacing().item_spacing.x;
						ui.spacing_mut().item_spacing.x = 8.0;
						let time = timestamp(id);
						compact_header(
							ui,
							ui.available_width(),
							egui::FontId::proportional(12.0),
							|ui| {
								let time = ui
									.label(
										RichText::new(crate::local_time::clock(time))
											.size(12.0)
											.color(colors.muted),
									)
									.on_hover_text_with(|| crate::local_time::full(time));
								surface.exclude(time.rect);
							},
						);
						let width = compact_author_width(ui.available_width());
						compact_header(
							ui,
							width,
							egui::FontId::new(15.5, crate::design::medium_family(ui.ctx())),
							|ui| {
								ui.set_max_width(width);
								let author = crate::name_styles::in_row(
									name_hover == Some(message.id),
									|| {
										crate::account_badge::name(
											ui,
											&message.author,
											state.message_author_name(message),
											15.5,
											(colors.text_strong, author_colors, colors.chat),
											egui::Sense::click(),
											0.0,
										)
									},
								)
								.on_hover_cursor(egui::CursorIcon::PointingHand);
								if author.hovered() {
									ui.painter().hline(
										author.rect.x_range(),
										author.rect.bottom() - 1.0,
										egui::Stroke::new(1.0, name_color),
									);
								}
								crate::user_menu::show(
									&author,
									state,
									&message.author,
									profile,
									&mut self.user_action,
								);
								profile.person_click(ui, &author, None, &message.author);
								surface.keep(&author);
							},
						);
						ui.spacing_mut().item_spacing.x = body_spacing;
					} else if compact {
						time_rect = Some(
							ui.allocate_exact_size(
								egui::vec2(40.0, MESSAGE_LINE),
								egui::Sense::hover(),
							)
							.0,
						);
					} else {
						let avatar = avatars
							.show(ui, &message.author, 40.0, state.demo)
							.on_hover_cursor(egui::CursorIcon::PointingHand);
						avatar_hot = avatar.hovered();
						crate::user_menu::show(
							&avatar,
							state,
							&message.author,
							profile,
							&mut self.user_action,
						);
						profile.person_click(ui, &avatar, None, &message.author);
						surface.keep(&avatar);
					}
					ui.vertical(|ui| {
						ui.set_width(ui.available_width());
						let mut text_line = egui::Rect::NOTHING;
						if !compact && system.is_none() {
							ui.allocate_ui_with_layout(
								egui::vec2(ui.available_width(), MESSAGE_LINE),
								egui::Layout::left_to_right(egui::Align::Center),
								|ui| {
									ui.spacing_mut().item_spacing.x = 8.0;
									let author = crate::name_styles::in_row(
										name_hover == Some(message.id),
										|| {
											crate::account_badge::name(
												ui,
												&message.author,
												state.message_author_name(message),
												15.5,
												(colors.text_strong, author_colors, colors.chat),
												egui::Sense::click(),
												48.0,
											)
										},
									)
									.on_hover_cursor(egui::CursorIcon::PointingHand);
									if avatar_hot || author.hovered() {
										let line = author.rect.bottom() - 1.0;
										ui.painter().hline(
											author.rect.x_range(),
											line,
											egui::Stroke::new(1.0, name_color),
										);
									}
									crate::user_menu::show(
										&author,
										state,
										&message.author,
										profile,
										&mut self.user_action,
									);
									profile.person_click(ui, &author, None, &message.author);
									surface.keep(&author);
									let time = timestamp(id);
									let time = ui
										.label(
											RichText::new(crate::local_time::header(time))
												.size(12.0)
												.color(colors.muted),
										)
										.on_hover_text_with(|| crate::local_time::full(time));
									surface.exclude(time.rect);
								},
							);
						}
						if let Some(system) = &system {
							// Only a thread-start row carries the thread affordances.
							let thread = (message.kind == 18 && system.content_shown)
								.then(|| (state.thread_of(message).map(|c| c.id), message.channel));
							show_system(
								ui,
								system,
								timestamp(id),
								state,
								profile,
								&mut self.user_action,
								&mut surface,
								thread,
								(&mut self.channel_reference, &mut self.threads_request),
							);
							if self
								.restricted
								.allows(state, crate::restricted::Key::message(message))
							{
								crate::activity_alerts::show(
									ui,
									state,
									message,
									&mut self.activity_review,
								);
							}
						}
						let body = egui::Frame::NONE
							.inner_margin(egui::Margin {
								left: if message.forwarded { 16 } else { 0 },
								..Default::default()
							})
							.show(ui, |ui| {
								if message.forwarded {
									ui.label(
										RichText::new(crate::i18n::translate(
											"timeline-show-with-scroll-forwarded",
										))
										.size(13.0)
										.italics()
										.color(colors.muted),
									);
									ui.add_space(4.0);
								}
								let body_color = if deleted
									&& !self.suppressed_deleted_highlight.contains(&id)
								{
									Some(colors.danger)
								} else {
									None
								};
								let formatted = self.formatted.get(id, &message.content);
								let reveal = self
									.revealed
									.get(&id)
									.filter(|reveal| reveal.matches(message));
								let before =
									reveal.map_or((0, false), |reveal| (reveal.text, reveal.media));
								let mut text = if formatted.spoilers { before.0 } else { 0 };
								queue_missing_channel_reference(
									&mut self.channel_reference_load,
									formatted,
									state,
									text,
								);
								let mut media = before.1;
								let mut inline_edited = false;
								let content_shown =
									system.as_ref().is_some_and(|s| s.content_shown);
								if !content_shown
									&& !(self.hide_media_links
										&& crate::embeds::standalone_media_links(message))
								{
									let jumbo = formatted.jumbo();
									text_line = ui
										.scope(|ui| {
											if jumbo {
												crate::design::jumbo_emoji(ui);
											}
											if let Some(color) = body_color {
												ui.visuals_mut().override_text_color = Some(color);
											}
											if deleted && message.content.is_empty() {
												ui.label(
													RichText::new(crate::i18n::translate(
														"timeline-show-with-scroll-deleted-message-had-no-text",
													))
													.size(16.0)
													.color(body_color.unwrap_or(colors.text)),
												);
											} else {
												let source = crate::mentions::MentionSource {
													state,
													channel: message.channel,
												};
												formatted.show_message(
													ui,
													&mut self.opening,
													&message.mentions,
													Some(&source),
													profile,
													(
														&state.channels,
														&mut self.channel_reference,
														&state.guilds,
														crate::mentions::known_roles(
															state,
															message.channel,
														),
													),
													(avatars, state.demo, &mut text),
													&mut surface,
													"",
													crate::design::MessageCardSurface::Conversation,
													message.edited,
												);
												inline_edited = message.edited;
											}
										})
										.response
										.rect;
								}
								if formatted.limited {
									ui.label(
										RichText::new(crate::i18n::translate(
											"timeline-show-with-scroll-display-limited-copy-message-for-the-full-text",
										))
										.small()
										.color(colors.muted),
									);
								}
								if crate::embeds::has_media_spoilers(message) && !media {
									let reveal = ui.button(crate::i18n::translate(
										"timeline-show-with-scroll-reveal-spoiler-media",
									));
									surface.keep(&reveal);
									if reveal.clicked() {
										media = true;
									}
								} else {
									let invite_top = ui.cursor().top();
									crate::invites::show(
										ui,
										message,
										state,
										avatars,
										&mut self.invite_requests,
										&mut self.invite_action,
									);
									surface.exclude(egui::Rect::from_min_max(
										egui::pos2(ui.max_rect().left(), invite_top),
										egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()),
									));
									let embed_top = ui.cursor().top();
									let poll = message.reply_to.filter(|_| message.kind == 46);
									if crate::embeds::poll_result(
										ui,
										message,
										poll.is_some_and(|id| state.can_open_reply_target(id)),
									) && let Some(poll) = poll
									{
										self.request_reply_target(poll);
									}
									if let Some(gif) = crate::embeds::show(
										ui,
										message,
										&mut self.formatted,
										avatars,
										&mut self.opening,
										&mut self.download,
										profile,
										&mut self.video,
										state,
										crate::design::MessageCardSurface::Conversation,
										self.hide_media_links,
									) {
										self.gif_favorite = Some(gif);
									}
									surface.exclude(egui::Rect::from_min_max(
										egui::pos2(ui.max_rect().left(), embed_top),
										egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()),
									));
									if !message.extra_content.components_v2 {
										let previous_view = self.viewing;
										crate::attachments::show(
											ui,
											message,
											avatars,
											&mut self.viewing,
											&mut self.opening,
											&mut self.download,
											&mut self.audio,
											&mut self.video,
											state.demo,
											&mut surface,
											crate::design::MessageCardSurface::Conversation,
										);
										if self.viewing != previous_view {
											self.component_viewing = None;
											self.embed_viewing = None;
										}
									}
								}
								if before != (text, media) {
									if text == 0 && !media {
										self.revealed.remove(&id);
									} else {
										self.revealed
											.insert(id, Revealed::new(message, text, media));
									}
									self.heights.remove(&id);
									ui.ctx().request_repaint();
								}
								if message.edited && !inline_edited {
									ui.label(
										RichText::new(crate::i18n::translate(
											"timeline-show-with-scroll-edited",
										))
										.small()
										.color(colors.muted),
									);
								}
								if !message.components.is_empty() {
									let shown = ui.scope(|ui| {
										self.components.show(
											ui,
											message,
											state,
											avatars,
											&mut self.opening,
											&mut crate::components::MediaUi {
												component_viewing: &mut self.component_viewing,
												viewing: &mut self.viewing,
												download: &mut self.download,
												audio: &mut self.audio,
												video: &mut self.video,
											},
										)
									});
									surface.exclude(shown.response.rect);
									if shown.inner.is_some() {
										self.component_action = shown.inner;
									}
								}
								if state
									.interactions
									.pending
									.as_ref()
									.is_some_and(|pending| pending.message == Some(message.id))
								{
									ui.small(crate::i18n::translate(
										"timeline-show-with-scroll-application-interaction-pending",
									));
								}
								if !message.components.is_empty()
									&& let Some(error) = state.interactions.error
								{
									ui.colored_label(colors.danger, error);
								}
								if message.ephemeral {
									ui.horizontal(|ui| {
										ui.spacing_mut().item_spacing.x = 4.0;
										let (icon, _) = ui.allocate_exact_size(
											egui::Vec2::splat(16.0),
											egui::Sense::hover(),
										);
										crate::icons::paint(
											ui.painter(),
											crate::icons::Icon::EyeSlash,
											icon,
											colors.muted,
										);
										ui.label(
											RichText::new(crate::i18n::translate(
												"timeline-show-with-scroll-only-you-can-see-this",
											))
											.size(13.0)
											.color(colors.muted),
										);
										let dismiss = ui
											.add(
												egui::Button::new(
													RichText::new(crate::i18n::translate(
														"timeline-show-with-scroll-dismiss-message",
													))
													.size(13.0)
													.color(colors.link),
												)
												.frame(false),
											)
											.on_hover_cursor(egui::CursorIcon::PointingHand);
										surface.keep(&dismiss);
										if dismiss.clicked() {
											self.dismiss_ephemeral = Some(id);
										}
									});
								}

								for sticker in &message.sticker_items {
									let response = ui
										.push_id(sticker.id, |ui| {
											crate::stickers::message(
												ui,
												sticker,
												state,
												avatars,
												&mut self.sticker_request,
												&mut self.browse_sticker,
											)
										})
										.inner;
									surface.keep(&response);
								}
								if message.poll.is_some() {
									let shown =
										ui.scope(|ui| self.polls.show(ui, state, message, avatars));
									surface.exclude(shown.response.rect);
									if let Some(action) = shown.inner {
										self.poll_action = Some((message.id, action));
									}
								}
								let unknown_system =
									message.unsupported && message.system_summary().is_none();
								if unknown_system
									|| (message.extra_content.poll && message.poll.is_none())
									|| ((message.extra_content.sticker_items
										|| message.extra_content.stickers)
										&& message.sticker_items.is_empty())
									|| ((message.extra_content.components
										|| message.extra_content.components_v2)
										&& message.components.is_empty())
								{
									if unknown_system {
										ui.label(
											RichText::new(crate::i18n::translate_args(
												"conversation-unsupported-type",
												&[("kind", &message.kind.to_string())],
											))
											.small()
											.color(colors.muted),
										);
									}
									for (present, label) in [
										(
											message.extra_content.poll && message.poll.is_none(),
											"conversation-poll-unavailable",
										),
										(
											(message.extra_content.sticker_items
												|| message.extra_content.stickers) && message
												.sticker_items
												.is_empty(),
											"conversation-sticker-unavailable",
										),
										(
											(message.extra_content.components
												|| message.extra_content.components_v2)
												&& message.components.is_empty(),
											"conversation-components-unavailable",
										),
									] {
										if present {
											ui.label(
												RichText::new(crate::i18n::translate(label))
													.small()
													.color(colors.muted),
											);
										}
									}
									let target = state
										.channels
										.iter()
										.find(|c| c.id == message.channel && state.can_view(c.id))
										.and_then(|c| discord_url(c, Some(message.id)));
									let open = ui.add_enabled(
										target.is_some(),
										egui::Button::new(crate::i18n::translate(
											"timeline-show-with-scroll-open-in-discord",
										)),
									);
									surface.keep(&open);
									if open.clicked() {
										self.browser_opening = target;
									}
								}
							});
						body_bottom = body.response.rect.bottom();
						if message.forwarded {
							let rail = egui::Rect::from_min_max(
								body.response.rect.min,
								egui::pos2(
									body.response.rect.left() + 3.0,
									body.response.rect.bottom(),
								),
							);
							ui.painter().rect_filled(rail, 2.0, colors.selected);
						}
						// A "started a thread" row already links the thread inline.
						if system.is_none()
							&& let Some(thread) = state.thread_of(message)
						{
							let card = thread_card(ui, thread, &colors);
							surface.keep(&card);
							if card.clicked() {
								self.channel_reference = Some(thread.id);
							}
						}
						let reactions_top = ui.cursor().top();
						if deleted {
							crate::reactions::show_frozen(
								ui,
								state.reactions.display(message),
								(avatars, state.demo),
							);
						} else if let Some(action) = crate::reactions::show(
							ui,
							state.reactions.display(message),
							state.gateway_connected
								&& state.freshness == model::Freshness::Fresh
								&& state.can_read_history(message.channel),
							state.reactions.busy(),
							(state.history_pending && state.history_before.is_none())
								|| state.reactions.invalidated(message.id),
							(avatars, state.demo),
							message.id,
							state.reactions.users.as_ref(),
							|emoji, add| state.can_react(id, Some(emoji), add),
						) {
							match action {
								crate::reactions::Action::Reload => {
									self.reaction = Some((id, None));
								}
								crate::reactions::Action::Toggle(emoji) => {
									self.reaction = Some((id, Some(emoji)));
								}
								crate::reactions::Action::Inspect(emoji, open) => {
									reaction_menu |= open;
									self.reaction_users = Some((id, emoji, open));
								}
							}
						}
						surface.exclude(egui::Rect::from_min_max(
							egui::pos2(ui.max_rect().left(), reactions_top),
							egui::pos2(ui.max_rect().right(), ui.min_rect().bottom()),
						));
						fill_header_line(ui, compact, text_line);
					});
				});
				let mut cover = ui.min_rect();
				if body_bottom.is_finite() {
					cover.max.y = body_bottom;
				}
				double_click = ui
					.input(|input| {
						input
							.pointer
							.button_double_clicked(egui::PointerButton::Primary)
							.then(|| input.pointer.interact_pos())
							.flatten()
					})
					.filter(|pos| surface.blank_at(*pos));
				surface.cover(cover);
				surface.finish(ui);
			});
		let rect = row.response.rect;
		if let Some(pos) = double_click
			&& let Some(text) = self.double_click_reaction
			&& !deleted
			&& rect.contains(pos)
			&& self
				.toolbar
				.is_none_or(|(_, toolbar)| !toolbar.contains(pos))
			&& ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
			&& !egui::Popup::is_any_open(ui.ctx())
		{
			let (emoji, reacted, enabled) = quick_reaction(state, message, text);
			if enabled {
				// The second click selected the nearest word in the blank band.
				crate::select::clear(ui.ctx());
				self.reaction = Some((id, Some(emoji)));
				if !reacted {
					self.quick_reaction_used = Some(text);
				}
			}
		}
		let mentioned = mentions_viewer(message, state);
		// Mentions mark the row in the warning colour; a private command response in
		// the house accent, as in the official client.
		let marked = if mentioned {
			Some(colors.warning)
		} else if message.ephemeral {
			Some(colors.accent)
		} else {
			None
		};
		if let Some(mark) = marked {
			ui.painter().set(
				background,
				egui::Shape::rect_filled(rect, 0.0, crate::design::row_highlight(ui, mark, 0.10)),
			);
			ui.painter().rect_filled(
				egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
				0.0,
				mark,
			);
		}
		let focus = ui.interact(
			rect,
			ui.scope_id().with("message-focus"),
			egui::Sense::focusable_noninteractive(),
		);
		focus.widget_info(|| {
			egui::WidgetInfo::labeled(
				egui::Role::Label,
				true,
				format!(
					"Message by {}. {}Tab for actions.",
					message.author.name,
					if mentioned { "Mentions you. " } else { "" },
				),
			)
		});
		let retained = retained_toolbar.is_some_and(|(active, _)| active == id);
		// The floating toolbar overlaps the row above; pointer inside it keeps this row active.
		let toolbar_hover = self
			.toolbar
			.filter(|(active, toolbar)| *active == id && ui.rect_contains_pointer(*toolbar))
			.is_some();
		let other_toolbar_hover = self
			.toolbar
			.filter(|(active, toolbar)| *active != id && ui.rect_contains_pointer(*toolbar))
			.is_some();
		let hovered = allow_hover
			&& (ui.rect_contains_pointer(rect) || toolbar_hover)
			&& !other_toolbar_hover
			&& !egui::Popup::is_any_open(ui.ctx())
			&& retained_toolbar.is_none_or(|(active, _)| active == id);
		if hovered {
			self.name_hover = Some(id);
		}
		let owner = crate::message_menu::Owner {
			generation: state.generation,
			channel: message.channel,
			message: id,
		};
		let keyboard_context = !deleted
			&& context_key
			&& (focus.has_focus()
				|| crate::message_menu::has_hit(ui.ctx(), owner)
				|| retained_toolbar.is_some_and(|(active, toolbar)| {
					active == id
						&& keyboard_focus
							.as_ref()
							.is_some_and(|r| toolbar.contains_rect(r.rect))
				}));
		let context_menu = !reaction_menu
			&& !egui::Popup::is_any_open(ui.ctx())
			&& (keyboard_context
				|| ((ui.rect_contains_pointer(rect) || toolbar_hover)
					&& !other_toolbar_hover
					&& (ui.input(|i| i.pointer.secondary_clicked())
						|| crate::select::open_menu(ui.ctx()))));
		if keyboard_context && context_menu {
			ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F10));
		}
		if context_menu
			|| hovered
			|| focus.has_focus()
			|| keyboard_focus.as_ref().is_some_and(|r| r.id == focus.id)
			|| retained
		{
			ui.painter().set(
				background,
				egui::Shape::rect_filled(
					rect,
					0.0,
					if let Some(mark) = marked {
						crate::design::row_highlight(ui, mark, 0.16)
					} else {
						crate::design::row_highlight(ui, colors.hover, 0.7)
					},
				),
			);
			if let Some(rect) = time_rect {
				let time = timestamp(id);
				ui.painter().text(
					rect.center(),
					egui::Align2::CENTER_CENTER,
					crate::local_time::clock(time),
					egui::FontId::proportional(11.0),
					colors.muted,
				);
				ui.interact(rect, ui.scope_id().with("timestamp"), egui::Sense::hover())
					.on_hover_text_with(|| crate::local_time::full(time));
			}
			let own = state
				.user
				.as_ref()
				.is_some_and(|u| u.id == message.author.id);
			if deleted {
				let toolbar_rect = clamp_hover_toolbar(
					egui::Rect::from_min_size(
						egui::pos2(rect.right() - 46.0, rect.top() - 10.0),
						egui::vec2(36.0, 28.0),
					),
					clip,
				);
				let mut toolbar = ui.new_child(
					egui::UiBuilder::new()
						.id_salt("hover-actions")
						.max_rect(toolbar_rect)
						.layout(egui::Layout::left_to_right(egui::Align::Center)),
				);
				toolbar.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
				toolbar.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
				toolbar.spacing_mut().interact_size.y = 28.0;
				toolbar
					.painter()
					.rect_filled(toolbar_rect, 6.0, colors.raised);
				toolbar.painter().rect_stroke(
					toolbar_rect,
					6.0,
					egui::Stroke::new(1.0, colors.border),
					egui::StrokeKind::Inside,
				);
				let menu =
					action_button(&mut toolbar, crate::icons::Icon::More, "profiles-show-more");
				menu.widget_info(|| {
					egui::WidgetInfo::labeled(
						egui::Role::Button,
						toolbar.is_enabled(),
						crate::i18n::translate_args(
							"timeline-deleted-message-actions-for",
							&[("user", &message.author.name)],
						),
					)
				});
				let mut popup = egui::Popup::menu(&menu);
				if context_menu {
					popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
				}
				if context_menu
					|| (!menu.clicked()
						&& egui::Popup::position_of_id(toolbar.ctx(), popup.get_id()).is_some())
				{
					popup = popup.at_pointer_fixed();
				}
				let mut action = None;
				deleted_message_actions(popup, &mut action);
				match action {
					Some(DeletedLocalAction::ToggleHighlight) => {
						if !self.suppressed_deleted_highlight.remove(&id) {
							self.suppressed_deleted_highlight.insert(id);
						}
					}
					Some(DeletedLocalAction::Remove) => {
						self.remove_preserved = Some(id);
					}
					None => {}
				}
				self.toolbar = Some((id, toolbar_rect));
			} else {
				let react = state.can_open_reaction_picker(id);
				// Holding Shift exposes direct actions alongside the menu.
				let shift = ui.input(|input| input.modifiers.shift);
				let can_pin = state.can_pin(message.channel, id);
				let can_delete = state.can_delete(message.channel, id);
				let link = state
					.channel(message.channel)
					.and_then(|channel| discord_url(channel, Some(id)));
				let buttons = if react && !shift { 4 } else { 1 }
					+ 3 + usize::from(own)
					+ if shift {
						2 + usize::from(link.is_some())
							+ usize::from(can_delete)
							+ usize::from(can_mark_unread)
							+ usize::from(can_pin)
					} else {
						0
					};
				let width = buttons as f32 * 30.0;
				let toolbar_rect = clamp_hover_toolbar(
					egui::Rect::from_min_size(
						egui::pos2(
							(rect.right() - 16.0 - width).max(rect.left()),
							rect.top() - 10.0,
						),
						egui::vec2(width, 28.0),
					),
					clip,
				);
				// A child overlay keeps hover from changing wrapping or cached row heights.
				let mut toolbar = ui.new_child(
					egui::UiBuilder::new()
						.id_salt("hover-actions")
						.max_rect(toolbar_rect)
						.layout(egui::Layout::left_to_right(egui::Align::Center)),
				);
				toolbar.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
				toolbar.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
				toolbar.spacing_mut().interact_size.y = 28.0;
				toolbar
					.painter()
					.rect_filled(toolbar_rect, 6.0, colors.raised);
				toolbar.painter().rect_stroke(
					toolbar_rect,
					6.0,
					egui::Stroke::new(1.0, colors.border),
					egui::StrokeKind::Inside,
				);
				if react && !shift {
					let quick = self.menu.quick(
						state,
						message,
						self.quick_reactions
							.unwrap_or(crate::reactions::QUICK_DEFAULTS),
					);
					let mut last = egui::Rect::NOTHING;
					for (index, emoji) in quick.into_iter().take(3).enumerate() {
						let (emoji, reacted, enabled) = quick_reaction_emoji(state, message, emoji);
						let response = toolbar
							.push_id(("quick-reaction", index), |ui| {
								ui.add_enabled_ui(enabled, |ui| {
									crate::reactions::quick_button(
										ui, &emoji, reacted, avatars, state.demo,
									)
								})
								.inner
							})
							.inner;
						last = response.rect;
						if response.clicked() {
							if !reacted && emoji.id.is_none() {
								self.quick_reaction_used = crate::emoji_picker::standard()
									.iter()
									.find(|(text, _)| Some(*text) == emoji.name.as_deref())
									.map(|(text, _)| *text);
							}
							self.reaction = Some((id, Some(emoji)));
						}
					}
					let x = last.right() + 1.0;
					toolbar.painter().vline(
						x,
						last.y_range().shrink(6.0),
						egui::Stroke::new(1.0, colors.border),
					);
				}
				if let Some((anchor, trigger)) =
					crate::reactions::add_button(&mut toolbar, react, state.reactions.busy())
				{
					self.reaction_picker = Some((id, anchor, trigger));
				}
				// Reply would be hidden behind the edit composer and Enter would still edit.
				let can_reply = editing
					.as_ref()
					.is_none_or(|(channel, _, _)| *channel != message.channel)
					&& state.can_reply_to(id);
				let can_edit = !message.unsupported && state.can_edit(message.channel, id);
				if toolbar
					.add_enabled_ui(can_reply, |ui| {
						action_button(
							ui,
							crate::icons::Icon::Reply,
							"timeline-message-actions-reply",
						)
					})
					.inner
					.clicked()
				{
					*selected_reply = Some(id);
				}
				if toolbar
					.add_enabled_ui(state.can_forward(id), |ui| {
						action_button(
							ui,
							crate::icons::Icon::Forward,
							"timeline-message-actions-forward",
						)
					})
					.inner
					.clicked()
				{
					self.forward_request = Some(id);
				}
				if own
					&& toolbar
						.add_enabled_ui(can_edit, |ui| {
							action_button(
								ui,
								crate::icons::Icon::Pencil,
								"timeline-message-actions-edit-message",
							)
						})
						.inner
						.clicked()
				{
					*editing = Some((message.channel, id, message.content.clone()));
					self.edit_started = true;
				}
				if shift {
					let pinned = state.is_pinned(message.channel, id);
					if can_pin
						&& action_button(
							&mut toolbar,
							crate::icons::Icon::Pin,
							if pinned {
								"timeline-message-actions-unpin-message"
							} else {
								"timeline-message-actions-pin-message"
							},
						)
						.clicked()
					{
						self.pin_request = Some((message.channel, id, !pinned));
					}
					if can_mark_unread
						&& action_button(
							&mut toolbar,
							crate::icons::Icon::Inbox,
							"timeline-message-actions-mark-unread",
						)
						.clicked()
					{
						self.mark_unread = Some(id);
					}
					if let Some(link) = &link {
						copy_action_button(
							&mut toolbar,
							crate::icons::Icon::Link,
							"timeline-message-actions-copy-message-link",
							egui::Id::unique(("copy-link", state.generation, id)),
							&mut self.copy_feedback,
							|| link.clone(),
						);
					}
					copy_action_button(
						&mut toolbar,
						crate::icons::Icon::Copy,
						"message-menu-copy",
						egui::Id::unique(("copy-message", state.generation, id)),
						&mut self.copy_feedback,
						|| crate::i18n::message_text(message).into_owned(),
					);
					copy_action_button(
						&mut toolbar,
						crate::icons::Icon::Hash,
						"timeline-message-actions-copy-message-id",
						egui::Id::unique(("copy-id", state.generation, id)),
						&mut self.copy_feedback,
						|| id.to_string(),
					);
				}
				if can_delete
					&& shift && toolbar
					.push_id("quick-delete", |ui| {
						action_button(
							ui,
							crate::icons::Icon::Trash,
							"timeline-message-actions-delete-message",
						)
					})
					.inner
					.clicked()
				{
					self.quick_delete = Some((message.channel, id));
				}
				let menu =
					action_button(&mut toolbar, crate::icons::Icon::More, "profiles-show-more");
				menu.widget_info(|| {
					egui::WidgetInfo::labeled(
						egui::Role::Button,
						toolbar.is_enabled(),
						crate::i18n::translate_args(
							"timeline-message-actions-for",
							&[("user", &message.author.name)],
						),
					)
				});
				let mut popup = egui::Popup::menu(&menu).id(egui::Id::unique((
					"message-menu",
					state.generation,
					id,
				)));
				let target = crate::message_menu::target(
					ui.ctx(),
					popup.get_id(),
					owner,
					context_menu,
					menu.clicked(),
					keyboard_context.then(|| {
						keyboard_focus
							.as_ref()
							.map_or((focus.id, rect.right_bottom()), |r| {
								(r.id, r.rect.right_bottom())
							})
					}),
				);
				if context_menu {
					popup = popup.open_memory(Some(egui::SetOpenCommand::Bool(true)));
				}
				if let Some((widget, anchor)) =
					crate::message_menu::anchor(ui.ctx(), popup.get_id(), owner)
				{
					popup = popup.anchor_widget(widget).at_position(anchor);
				} else if context_menu
					|| (!menu.clicked()
						&& egui::Popup::position_of_id(toolbar.ctx(), popup.get_id()).is_some())
				{
					popup = popup.at_pointer_fixed();
				}
				popup = popup.frame(crate::message_menu::frame(ui.ctx()));
				message_actions(
					popup,
					Some((state, &mut self.user_action)),
					(
						message,
						link.as_deref(),
						&self.extension_actions,
						&mut self.extension_request,
					),
					(own, can_reply, can_edit, can_delete),
					(
						can_mark_read.then_some(&mut self.mark_read),
						can_mark_unread.then_some(&mut self.mark_unread),
						selected_reply,
					),
					(editing, &mut self.edit_started),
					deleting,
					(
						can_pin,
						state.is_pinned(message.channel, id),
						&mut self.pin_request,
					),
					(
						!message.ephemeral
							&& state.can_load_mentions()
							&& state.can_read_history(message.channel)
							&& !state.inbox.loading,
						&mut self.bookmark_request,
					),
					(
						state.can_publish_message(message.channel, id)
							&& !state.channel_action_pending(),
						&mut self.publish_request,
					),
					(
						state.can_create_thread(message.channel)
							&& state.thread_of(message).is_none(),
						&mut self.thread_request,
					),
					(state.can_forward(id), &mut self.forward_request),
					message
						.reactions
						.as_ref()
						.filter(|_| state.can_read_history(message.channel))
						.and_then(|items| items.first())
						.map(|reaction| (reaction.emoji.clone(), &mut self.reaction_users)),
					(
						target,
						&mut self.menu,
						avatars,
						self.quick_reactions
							.unwrap_or(crate::reactions::QUICK_DEFAULTS),
					),
				);
				self.toolbar = Some((id, toolbar_rect));
			}
		}
		if selected_reply.or(state.reply_target()) == Some(id)
			|| self.highlighted.is_some_and(|(target, _)| target == id)
		{
			ui.painter().set(
				background,
				egui::Shape::rect_filled(
					rect,
					0.0,
					crate::design::row_highlight(ui, colors.accent, 0.25),
				),
			);
		}
	}
}
