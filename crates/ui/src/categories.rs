use crate::design::LazyHover;
use crate::{MessagingUi, design};
use client_core::State;
use egui::RichText;
use model::{Channel, Id};
use std::collections::{BTreeMap, BTreeSet};

enum Row<'a> {
	Section(&'static str),
	Category(&'a Channel, usize),
	Channel(&'a Channel, bool),
	Participant(&'a client_core::voice::RosterEntry),
}

#[derive(Clone, Copy)]
enum CachedRow {
	Section(&'static str),
	Category(usize, usize),
	Channel(usize, bool),
	Participant(usize),
}
type CacheKey = (u64, u64, Option<Id>, Option<Id>, bool, bool);
#[derive(Default)]
pub(super) struct Cache {
	key: Option<CacheKey>,
	rows: Vec<CachedRow>,
}
impl Cache {
	pub(super) fn invalidate(&mut self) {
		self.key = None;
	}
}

fn promote<'a>(
	mut rows: Vec<Row<'a>>,
	state: &'a State,
	guild: Id,
	preferences: &model::ChannelPreferences,
) -> Vec<Row<'a>> {
	// Shortcuts remain visible when their original category is collapsed.
	let present: BTreeSet<_> = rows
		.iter()
		.filter_map(|row| match row {
			Row::Channel(channel, _) => Some(channel.id),
			_ => None,
		})
		.collect();
	rows.extend(
		state
			.channels
			.iter()
			.filter(|channel| {
				channel.guild == Some(guild)
					&& channel.kind != 4
					&& state.can_view(channel.id)
					&& (preferences.is_pinned(channel.id) || preferences.is_favorite(channel.id))
					&& !present.contains(&channel.id)
			})
			.map(|channel| Row::Channel(channel, false)),
	);
	let mut pinned = Vec::new();
	let mut favorites = Vec::new();
	let mut regular = Vec::new();
	for row in rows {
		match row {
			Row::Channel(channel, _) if preferences.is_pinned(channel.id) => {
				pinned.push(Row::Channel(channel, false))
			}
			Row::Channel(channel, _) if preferences.is_favorite(channel.id) => {
				favorites.push(Row::Channel(channel, false))
			}
			Row::Channel(channel, true)
				if channel
					.parent_id
					.is_some_and(|id| preferences.is_pinned(id)) =>
			{
				pinned.push(Row::Channel(channel, true))
			}
			Row::Channel(channel, true)
				if channel
					.parent_id
					.is_some_and(|id| preferences.is_favorite(id)) =>
			{
				favorites.push(Row::Channel(channel, true))
			}
			row => regular.push(row),
		}
	}
	let mut result = Vec::with_capacity(pinned.len() + favorites.len() + regular.len() + 2);
	for (label, group) in [("Pinned", pinned), ("Favorites", favorites)] {
		if !group.is_empty() {
			result.push(Row::Section(label));
			result.extend(group);
		}
	}
	result.extend(regular);
	result
}

fn rows<'a>(
	state: &'a State,
	guild: Option<Id>,
	collapsed: &BTreeSet<Id>,
	show_hidden: bool,
) -> Vec<Row<'a>> {
	let channels = &state.channels;
	let mut categories: Vec<_> = channels
		.iter()
		.filter(|c| {
			c.guild == guild
				&& guild.is_some()
				&& c.kind == 4
				&& (show_hidden || state.can_view(c.id))
		})
		.collect();
	categories.sort_unstable_by_key(|c| (c.position, c.id));
	let category_ids: BTreeSet<_> = categories.iter().map(|c| c.id).collect();
	let parents: BTreeMap<_, _> = channels
		.iter()
		.filter(|c| guild.is_some() && c.guild == guild && matches!(c.kind, 0 | 5 | 15 | 16))
		.map(|c| (c.id, c))
		.collect();
	let mut groups: BTreeMap<Option<Id>, Vec<&Channel>> = BTreeMap::new();
	let mut threads: BTreeMap<Id, Vec<&Channel>> = BTreeMap::new();
	for channel in channels
		.iter()
		.filter(|c| c.guild == guild && c.kind != 4 && (show_hidden || state.can_view(c.id)))
	{
		if matches!(channel.kind, 10..=12)
			&& let Some(parent) = channel.parent_id.and_then(|id| parents.get(&id))
			&& parent.parent_id != Some(channel.id)
		{
			threads.entry(parent.id).or_default().push(channel);
			continue;
		}
		let parent = channel
			.parent_id
			.filter(|id| !matches!(channel.kind, 10..=12) && category_ids.contains(id));
		groups.entry(parent).or_default().push(channel);
	}
	for group in groups.values_mut().chain(threads.values_mut()) {
		if guild.is_some() {
			group.sort_unstable_by_key(|c| (c.position, c.id));
		} else {
			group.sort_unstable_by_key(|c| std::cmp::Reverse((state.channel_activity(c), c.id)));
		}
	}
	let append = |channel: &'a Channel, collapsed: bool, rows: &mut Vec<Row<'a>>| {
		let children = threads.get(&channel.id);
		if !collapsed {
			rows.push(Row::Channel(channel, false));
			rows.extend(
				children
					.into_iter()
					.flatten()
					.map(|c| Row::Channel(c, true)),
			);
		}
	};
	let mut rows = Vec::with_capacity(channels.len());
	for channel in groups.remove(&None).unwrap_or_default() {
		append(channel, false, &mut rows);
	}
	for category in categories {
		let children = groups.remove(&Some(category.id)).unwrap_or_default();
		let count = children
			.iter()
			.map(|c| 1 + threads.get(&c.id).map_or(0, Vec::len))
			.sum();
		rows.push(Row::Category(category, count));
		for channel in children {
			append(channel, collapsed.contains(&category.id), &mut rows);
		}
	}
	rows
}

fn kind_label(kind: u8) -> &'static str {
	match kind {
		0 => "Text channel",
		1 => "Direct message",
		2 => "Server voice channel",
		3 => "Group direct message",
		5 => "Announcement channel",
		10..=12 => "Thread",
		13 => "Stage channel · not implemented",
		14 => "Directory · not implemented",
		15 => "Forum · loaded posts",
		16 => "Media · loaded posts",
		_ => "Unknown channel type · not implemented",
	}
}

impl MessagingUi {
	pub(super) fn channel_list(&mut self, ui: &mut egui::Ui, state: &mut State) -> Option<Id> {
		self.hidden_muted_guilds
			.retain(|guild| state.guild(*guild).is_some());
		let hide_muted = self
			.guild
			.is_some_and(|guild| self.hidden_muted_guilds.contains(&guild));
		let key = (
			state.generation,
			state.revision,
			self.guild,
			state.selected,
			self.show_hidden_channels,
			hide_muted,
		);
		if self.channel_cache.key != Some(key) {
			// Session-only keys are pruned on navigation updates, never accumulated in egui memory.
			let categories: BTreeSet<_> = state
				.channels
				.iter()
				.filter(|c| c.kind == 4)
				.map(|c| c.id)
				.collect();
			self.collapsed_categories
				.retain(|id| categories.contains(id));
			let channel_rows = rows(
				state,
				self.guild,
				&self.collapsed_categories,
				self.show_hidden_channels,
			);
			let mut channel_rows = if let Some(guild) = self.guild {
				promote(channel_rows, state, guild, &self.channel_preferences)
			} else {
				channel_rows
			};
			if hide_muted {
				channel_rows.retain(|row| {
					!matches!(row, Row::Channel(channel, _) if Some(channel.id) != state.selected && state.guild_channel_muted(channel.id) == Some(true))
				});
				let mut index = 0;
				while index < channel_rows.len() {
					if matches!(channel_rows[index], Row::Section(_))
						&& !matches!(channel_rows.get(index + 1), Some(Row::Channel(..)))
					{
						channel_rows.remove(index);
					} else {
						index += 1;
					}
				}
			}
			let mut participants = BTreeMap::<Id, Vec<_>>::new();
			for entry in &state.voice.roster {
				if Some(entry.guild) == self.guild && state.can_view(entry.channel) {
					participants.entry(entry.channel).or_default().push(entry);
				}
			}
			let mut rows = Vec::with_capacity(channel_rows.len() + state.voice.roster.len());
			for row in channel_rows {
				let channel = match &row {
					Row::Channel(channel, _) if channel.kind == 2 => Some(channel.id),
					_ => None,
				};
				rows.push(row);
				if let Some(entries) = channel.and_then(|id| participants.remove(&id)) {
					rows.extend(entries.into_iter().map(Row::Participant));
				}
			}
			let indices: BTreeMap<_, _> = state
				.channels
				.iter()
				.enumerate()
				.map(|(i, c)| (c.id, i))
				.collect();
			let participants: BTreeMap<_, _> = state
				.voice
				.roster
				.iter()
				.enumerate()
				.map(|(i, p)| ((p.channel, p.participant.user), i))
				.collect();
			self.channel_cache.rows = rows
				.into_iter()
				.map(|row| match row {
					Row::Section(label) => CachedRow::Section(label),
					Row::Category(c, n) => CachedRow::Category(indices[&c.id], n),
					Row::Channel(c, n) => CachedRow::Channel(indices[&c.id], n),
					Row::Participant(p) => {
						CachedRow::Participant(participants[&(p.channel, p.participant.user)])
					}
				})
				.collect();
			self.channel_cache.key = Some(key);
		}
		let colors = design::palette(ui);
		let mut selected = None;
		if self.guild.is_some() && self.channel_cache.rows.is_empty() {
			ui.label(RichText::new("No conversations available here.").color(colors.muted));
		}
		let dm_list = self.guild.is_none();
		let row_height = if dm_list { 44.0 } else { 34.0 };
		// The section heading shares the DM list's existing virtualized scroller.
		let prefix = if dm_list { 1 } else { 0 };
		let row_count = self.channel_cache.rows.len().max(usize::from(dm_list)) + prefix;
		let previous_spacing = ui.spacing().item_spacing.y;
		ui.spacing_mut().item_spacing.y = 0.0;
		let output = egui::ScrollArea::vertical()
			.id_salt(("channel-list", self.guild))
			.auto_shrink([false, false])
			.show_rows(ui, row_height, row_count, |ui, range| {
				for index in range {
					if index < prefix {
						ui.allocate_ui_with_layout(
							egui::vec2(ui.available_width(), row_height),
							egui::Layout::left_to_right(egui::Align::Center),
							|ui| {
								ui.add_space(8.0);
								ui.label(design::eyebrow(ui, "Direct Messages", colors.muted));
							},
						);
						continue;
					}
					let Some(row) = self.channel_cache.rows.get(index - prefix).copied() else {
						ui.label(
							RichText::new("No conversations available here.").color(colors.muted),
						);
						continue;
					};
					match row {
						CachedRow::Section(label) => {
							ui.allocate_ui_with_layout(
								egui::vec2(ui.available_width(), row_height),
								egui::Layout::left_to_right(egui::Align::Center),
								|ui| {
									ui.add_space(8.0);
									ui.label(design::eyebrow(ui, label, colors.muted));
								},
							);
						}
						CachedRow::Participant(entry) => {
							let entry = &state.voice.roster[entry];
							ui.horizontal(|ui| {
								ui.add_space(28.0);
								self.voice_participant(ui, state, entry);
							});
						}
						CachedRow::Category(category, count) => {
							let category = &state.channels[category];
							let collapsed = self.collapsed_categories.contains(&category.id);
							let (rect, response) = ui
								.push_id(category.id, |ui| {
									ui.allocate_exact_size(
										egui::vec2(ui.available_width(), row_height),
										egui::Sense::click(),
									)
								})
								.inner;
							let color = if response.hovered() || response.has_focus() {
								colors.text_strong
							} else {
								colors.muted
							};
							crate::icons::paint(
								ui.painter(),
								if collapsed {
									crate::icons::Icon::ChevronRight
								} else {
									crate::icons::Icon::ChevronDown
								},
								egui::Rect::from_center_size(
									egui::pos2(rect.left() + 7.0, rect.bottom() - 13.0),
									egui::Vec2::splat(12.0),
								),
								color,
							);
							let label = ui.painter().layout(
								category.name.to_uppercase(),
								egui::FontId::new(12.0, crate::design::semibold_family(ui.ctx())),
								color,
								(rect.width() - 24.0).max(10.0),
							);
							let label_rect = egui::Rect::from_min_size(
								egui::pos2(
									rect.left() + 16.0,
									rect.bottom() - 6.0 - label.size().y,
								),
								egui::vec2(rect.width() - 24.0, label.size().y),
							);
							ui.painter().with_clip_rect(label_rect).galley(
								label_rect.min,
								label,
								color,
							);
							let response = response.on_hover_text_with(|| {
								format!(
									"{} category · {} channels · {}",
									category.name,
									count,
									if collapsed { "Expand" } else { "Collapse" }
								)
							});
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::WidgetType::Button,
									true,
									format!(
										"{} category, {}, {} channels",
										category.name,
										if collapsed { "collapsed" } else { "expanded" },
										count
									),
								)
							});
							if response.clicked() {
								self.channel_cache.key = None;
								if collapsed {
									self.collapsed_categories.remove(&category.id);
								} else {
									self.collapsed_categories.insert(category.id);
								}
							}
							self.channel_menu.context(
								&response,
								state,
								category,
								&mut self.channel_preferences,
								&mut self.channel_preferences_changed,
								state.demo || self.channel_preferences_loaded,
							);
						}
						CachedRow::Channel(channel, nested) => {
							let channel = &state.channels[channel];
							let active = state.selected == Some(channel.id);
							if channel.kind == 2 {
								let response =
									self.voice_channel_button(ui, state, channel, active);
								self.channel_menu.context(
									&response,
									state,
									channel,
									&mut self.channel_preferences,
									&mut self.channel_preferences_changed,
									state.demo || self.channel_preferences_loaded,
								);
								if response.clicked() {
									selected = Some(channel.id);
								}
								continue;
							}
							let visible = state.can_view(channel.id);
							let unread = visible
								&& (state.channel_unread(channel) == Some(true)
									|| state.unread_count(channel.id) > 0);
							let count = if !visible {
								0
							} else if channel.guild.is_some() {
								state.mention_count(channel.id)
							} else {
								state.unread_count(channel.id)
							};
							// Forum containers open their post list; Discord lists them as browsable rows.
							let forum = channel.guild.is_some() && matches!(channel.kind, 15 | 16);
							let enabled = visible && (channel.supports_text() || forum);
							// Kinds Serein cannot render keep Discord's own destination.
							let external = (!channel.supports_text() && !forum)
								.then(|| crate::markdown::discord_url(channel, None))
								.flatten()
								.filter(|_| visible);
							let (rect, response) = ui
								.push_id(channel.id, |ui| {
									ui.allocate_exact_size(
										egui::vec2(ui.available_width(), row_height),
										if enabled || channel.guild.is_some() {
											egui::Sense::click()
										} else {
											egui::Sense::hover()
										},
									)
								})
								.inner;
							let row = rect.shrink2(egui::vec2(0.0, 1.0));
							let hovered = enabled && (response.hovered() || response.has_focus());
							if active {
								ui.painter().rect_filled(row, 8, colors.selected);
							} else if hovered {
								ui.painter().rect_filled(row, 8, colors.hover);
							}
							if unread && !active {
								ui.painter().rect_filled(
									egui::Rect::from_center_size(
										egui::pos2(row.left() - 6.0, row.center().y),
										egui::vec2(4.0, 8.0),
									),
									2,
									colors.text_strong,
								);
							}
							let name_color = if !enabled {
								colors.muted.gamma_multiply(0.6)
							} else if active || hovered || unread {
								colors.text_strong
							} else {
								colors.muted
							};
							let badge_width = if count > 0 { 34.0 } else { 0.0 };
							let trailing =
								badge_width + if external.is_some() { 30.0 } else { 0.0 };
							let content = egui::Rect::from_min_max(
								egui::pos2(
									row.left() + 8.0 + if nested { 14.0 } else { 0.0 },
									row.top(),
								),
								egui::pos2(row.right() - 8.0 - trailing, row.bottom()),
							);
							let mut inner = ui.new_child(
								egui::UiBuilder::new()
									.max_rect(content)
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							inner.spacing_mut().item_spacing.x = if dm_list { 12.0 } else { 6.0 };
							if channel.guild.is_none() {
								if channel.kind == 3 {
									let avatar = self
										.avatars
										.show_group(&mut inner, channel, 32.0, state.demo);
									self.group_menu.context(&avatar, state, channel);
									if enabled && avatar.clicked() {
										selected = Some(channel.id);
									}
								} else if let Some(user) = channel.recipients.first() {
									let avatar =
										self.avatars.show(&mut inner, user, 32.0, state.demo);
									if channel.kind == 1
										&& let Some(status) =
											crate::profiles::presence(state, user.id, None).0
									{
										design::presence_dot(
											&inner,
											avatar.rect,
											crate::profiles::presence_color(status),
											colors.sidebar,
										);
									}
									if channel.kind == 1 {
										crate::user_menu::show(
											&avatar,
											state,
											user,
											&mut self.profile,
											&mut self.user_action,
										);
									}
									// The avatar is part of the row: clicking it opens the conversation,
									// the profile stays behind the context menu and the header avatar.
									if enabled && avatar.clicked() {
										selected = Some(channel.id);
									}
								} else {
									design::avatar(&mut inner, &channel.name, 32.0);
								}
							} else {
								let icon = match channel.kind {
									13 => crate::icons::Icon::Speaker,
									15 | 16 => crate::icons::Icon::Forum,
									10..=12 => crate::icons::Icon::Threads,
									_ => crate::icons::Icon::Hash,
								};
								crate::icons::inline(
									&mut inner,
									icon,
									20.0,
									name_color.gamma_multiply(if active || hovered {
										1.0
									} else {
										0.85
									}),
								);
							}
							let mut label = String::from(state.conversation_name(channel));
							if !enabled {
								label.push_str(if visible {
									" · unavailable"
								} else {
									" · hidden"
								});
							}
							let subtitle = if dm_list && channel.kind == 1 {
								channel.recipients.first().and_then(|user| {
									let (_, custom, activities) =
										crate::profiles::presence(state, user.id, None);
									crate::profiles::subtitle(custom, activities)
								})
							} else {
								(dm_list && channel.kind == 3)
									.then(|| format!("{} Members", channel.recipients.len().max(1)))
							};
							let name =
								egui::Label::new(design::medium(ui, label, 15.0).color(name_color))
									.truncate()
									.selectable(false);
							if let Some(subtitle) = subtitle {
								inner.vertical(|ui| {
									ui.spacing_mut().item_spacing.y = 0.0;
									ui.add_space(((row.height() - 34.0) * 0.5).max(0.0));
									ui.add(name);
									ui.add(
										egui::Label::new(
											RichText::new(subtitle).size(12.0).color(colors.muted),
										)
										.truncate()
										.selectable(false),
									);
								});
							} else {
								inner.add(name);
							}
							if let Some(url) = &external {
								let mut open = ui.new_child(
									egui::UiBuilder::new()
										.max_rect(egui::Rect::from_center_size(
											row.right_center() - egui::vec2(18.0, 0.0),
											egui::Vec2::splat(28.0),
										))
										.layout(egui::Layout::left_to_right(egui::Align::Center)),
								);
								if crate::icons::button(
									&mut open,
									crate::icons::Icon::External,
									28.0,
									"Open in Discord",
								)
								.clicked()
								{
									self.timeline.opening = Some(url.clone());
								}
							}
							if count > 0 {
								crate::notifications::badge(
									ui,
									row.right_center()
										- egui::vec2(
											20.0 + if external.is_some() { 30.0 } else { 0.0 },
											0.0,
										),
									count,
									if active {
										colors.selected
									} else if hovered {
										colors.hover
									} else {
										colors.sidebar
									},
								);
							}
							let response = response.on_hover_text_with(|| {
								format!(
									"{} · {}{}",
									channel.name,
									kind_label(channel.kind),
									if unread && state.channel_unread(channel).is_none() {
										" · Session activity; read sync unavailable"
									} else if count > 0 {
										" · Notification count may be a lower bound"
									} else if !visible {
										" · Unavailable with current permission information"
									} else {
										""
									}
								)
							});
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::WidgetType::Button,
									enabled,
									format!(
										"{}{}; {} notifications",
										channel.name,
										if unread { ", unread" } else { "" },
										count
									),
								)
							});
							if channel.kind == 1
								&& let Some(user) = channel.recipients.first()
							{
								crate::user_menu::show(
									&response,
									state,
									user,
									&mut self.profile,
									&mut self.user_action,
								);
							}
							if channel.kind == 3 && channel.guild.is_none() {
								self.group_menu.context(&response, state, channel);
							}
							if channel.guild.is_some() {
								self.channel_menu.context(
									&response,
									state,
									channel,
									&mut self.channel_preferences,
									&mut self.channel_preferences_changed,
									state.demo || self.channel_preferences_loaded,
								);
							}
							if enabled && response.clicked() {
								selected = Some(channel.id);
							}
							if !enabled
								&& response.clicked() && let Some(url) = external
							{
								self.timeline.opening = Some(url);
							}
						}
					}
				}
			});
		if let Some(guild) = self.guild {
			let content_bottom =
				output.inner_rect.top() - output.state.offset.y + output.content_size.y;
			if content_bottom < output.inner_rect.bottom() {
				let empty = egui::Rect::from_min_max(
					egui::pos2(
						output.inner_rect.left(),
						content_bottom.max(output.inner_rect.top()),
					),
					output.inner_rect.max,
				);
				let response = ui.interact(
					empty,
					ui.id().with(("server-channel-area", guild)),
					egui::Sense::click(),
				);
				let mut next = hide_muted;
				self.channel_menu
					.sidebar_context(&response, state, guild, &mut next);
				if next != hide_muted {
					if next {
						self.hidden_muted_guilds.insert(guild);
					} else {
						self.hidden_muted_guilds.remove(&guild);
					}
					self.channel_cache.invalidate();
				}
			}
		}
		ui.spacing_mut().item_spacing.y = previous_spacing;
		selected
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn shortcuts_survive_collapsed_categories_without_duplicates_or_orphan_threads() {
		let mut state = test_support::demo_state();
		state.guilds[0].id = Id(100);
		state.channels = vec![
			channel(4, 4, 0, None),
			channel(7, 0, 0, Some(Id(4))),
			channel(8, 11, 0, Some(Id(7))),
			channel(9, 0, 1, Some(Id(4))),
		];
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		let preferences = model::ChannelPreferences {
			pinned: vec![Id(7)],
			favorites: vec![Id(7), Id(9)],
		};
		for collapsed in [BTreeSet::new(), BTreeSet::from([Id(4)])] {
			let output = promote(
				rows(&state, Some(Id(100)), &collapsed, false),
				&state,
				Id(100),
				&preferences,
			);
			let ids: Vec<_> = output
				.iter()
				.filter_map(|row| {
					if let Row::Channel(channel, _) = row {
						Some(channel.id)
					} else {
						None
					}
				})
				.collect();
			assert_eq!(ids.iter().filter(|id| **id == Id(7)).count(), 1);
			assert_eq!(ids.iter().filter(|id| **id == Id(9)).count(), 1);
			assert!(matches!(output[0], Row::Section("Pinned")));
			if collapsed.is_empty() {
				assert!(matches!(output[2], Row::Channel(c, true) if c.id == Id(8)));
			}
		}
		state.permissions = Default::default();
		assert!(promote(vec![], &state, Id(100), &preferences).is_empty());
	}
	#[test]
	fn find_and_friends_stay_pinned_while_the_dm_list_scrolls() {
		for width in [220.0, 320.0] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			let mut state = test_support::demo_state();
			state.channels = (1..=60)
				.map(|id| {
					let mut c = channel(id, 3, 0, None);
					c.guild = None;
					c
				})
				.collect();
			let mut view = MessagingUi::default();
			let render = |view: &mut MessagingUi, state: &mut State, events| {
				let mut output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(width, 380.0),
						)),
						events,
						..Default::default()
					},
					|ui| view.sidebar(ui, state, "Direct Messages", &mut vec![]),
				);
				output.textures_delta.clear();
				output
					.shapes
					.iter()
					.filter_map(|s| match &s.shape {
						egui::Shape::Text(t) => {
							let rect = egui::Rect::from_min_size(t.pos, t.galley.size());
							s.clip_rect
								.intersects(rect)
								.then(|| (t.galley.job.text.clone(), rect))
						}
						_ => None,
					})
					.collect::<Vec<_>>()
			};
			render(&mut view, &mut state, vec![]);
			let before = render(&mut view, &mut state, vec![]);
			let find = before
				.iter()
				.find(|(s, _)| s == "Find conversation")
				.unwrap()
				.1;
			// Friends is the icon-only button pinned to the right of the search row.
			let friends = egui::pos2(width - 8.0 - 15.0, find.center().y);
			for pressed in [true, false] {
				render(
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(friends),
						egui::Event::PointerButton {
							pos: friends,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert!(state.selected.is_none());
			let mut after = vec![];
			for _ in 0..12 {
				after = render(
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(egui::pos2(100.0, 250.0)),
						egui::Event::MouseWheel {
							phase: egui::TouchPhase::Move,
							unit: egui::MouseWheelUnit::Point,
							delta: egui::vec2(0.0, -100.0),
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(
				after
					.iter()
					.find(|(s, _)| s == "Find conversation")
					.unwrap()
					.1,
				find
			);
			assert!(!after.iter().any(|(s, _)| s == "DIRECT MESSAGES"));
			let visible_dms = after
				.iter()
				.filter(|(s, _)| s.starts_with("Synthetic "))
				.count();
			assert!(
				visible_dms > 0 && visible_dms < 12,
				"list remains virtualized: {visible_dms}"
			);
		}
	}
	fn channel(id: u64, kind: u8, position: i32, parent_id: Option<Id>) -> Channel {
		Channel {
			last_message: None,
			id: Id(id),
			guild: Some(Id(100)),
			parent_id,
			position,
			name: format!("Synthetic {id}"),
			kind,
			recipients: vec![],
			member_list_id: None,
			message_count: None,
			icon: None,
		}
	}
	#[test]
	fn hidden_channels_are_opt_in() {
		let state = State {
			channels: vec![channel(1, 0, 0, None)],
			..State::default()
		};
		assert!(!MessagingUi::default().show_hidden_channels);
		assert!(rows(&state, Some(Id(100)), &BTreeSet::new(), false).is_empty());
		assert_eq!(rows(&state, Some(Id(100)), &BTreeSet::new(), true).len(), 1);
	}
	#[test]
	fn empty_server_sidebar_opens_server_actions() {
		fn labels(shape: &egui::Shape, output: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => output.push((
					text.galley.job.text.clone(),
					text.galley.rect.translate(text.pos.to_vec2()),
				)),
				egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, output)),
				_ => {}
			}
		}
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut state = test_support::chat_demo_state();
		let mut permissions = test_support::permission_snapshot(&state);
		for guild in &mut permissions.guilds {
			guild.owner = state.user.as_ref().map(|user| user.id);
		}
		state.permissions.replace(permissions).unwrap();
		let mut view = MessagingUi {
			guild: Some(Id(10)),
			..Default::default()
		};
		let render = |view: &mut MessagingUi, state: &mut State, events| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(260.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| view.sidebar(ui, state, "Synthetic", &mut vec![]),
			);
			let mut text = vec![];
			for shape in &output.shapes {
				labels(&shape.shape, &mut text);
			}
			output.drop_without_applying_deltas();
			text
		};
		render(&mut view, &mut state, vec![]);
		let empty = egui::pos2(130.0, 600.0);
		for pressed in [true, false] {
			render(
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(empty),
					egui::Event::PointerButton {
						pos: empty,
						button: egui::PointerButton::Secondary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
		let text = render(&mut view, &mut state, vec![]);
		for expected in [
			"Hide Muted Channels",
			"Create Channel",
			"Create Category",
			"Invite to Server",
		] {
			assert!(
				text.iter().any(|(label, _)| label == expected),
				"missing {expected}: {text:?}"
			);
		}
		let hide = text
			.iter()
			.find(|(label, _)| label == "Hide Muted Channels")
			.unwrap()
			.1
			.center();
		for pressed in [true, false] {
			render(
				&mut view,
				&mut state,
				vec![
					egui::Event::PointerMoved(hide),
					egui::Event::PointerButton {
						pos: hide,
						button: egui::PointerButton::Primary,
						pressed,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
		}
		assert!(view.hidden_muted_guilds.contains(&Id(10)));
	}
	#[test]
	fn channel_rows_scroll_continuously_past_voice_participants() {
		let mut state = test_support::demo_state();
		state.guilds = vec![model::Guild {
			id: Id(100),
			name: "Synthetic".into(),
			icon: None,
			emojis: None,
		}];
		state.channels = (0..20)
			.map(|index| {
				channel(
					200 + index,
					if index == 1 { 2 } else { 0 },
					index as i32,
					None,
				)
			})
			.collect();
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		state.voice.roster = vec![client_core::voice::RosterEntry {
			guild: Id(100),
			channel: Id(201),
			member: None,
			participant: client_core::voice::Participant {
				user: Id(999),
				muted: true,
				deafened: true,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: false,
			},
		}];
		let mut view = MessagingUi {
			guild: Some(Id(100)),
			..Default::default()
		};
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let mut original_y = None;
		for offset in [0.0, 33.0, 34.0, 41.0, 42.0, 67.0, 68.0, 101.0, 102.0] {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(240.0, 200.0),
					)),
					..Default::default()
				},
				|ui| {
					let id = ui.make_persistent_id(egui::IdSalt::new(("channel-list", view.guild)));
					let mut scroll = egui::scroll_area::State::load(&ctx, id).unwrap_or_default();
					scroll.offset.y = offset;
					scroll.store(&ctx, id);
					view.channel_list(ui, &mut state);
					assert_eq!(ui.spacing().item_spacing.y, 8.0);
				},
			);
			let y = output.shapes.iter().find_map(|shape| match &shape.shape {
				egui::Shape::Text(text) if text.galley.job.text == "Synthetic 204" => {
					Some(text.pos.y)
				}
				_ => None,
			});
			output.drop_without_applying_deltas();
			assert!(
				view.channel_cache
					.rows
					.iter()
					.any(|row| matches!(row, CachedRow::Participant(_)))
			);
			let y = y.expect("The same synthetic channel remains visible");
			let original = *original_y.get_or_insert(y);
			assert!(
				(y + offset - original).abs() < 0.1,
				"Channel jumped at scroll offset {offset}: {y} versus {original}"
			);
		}
	}
	#[test]
	fn direct_and_group_messages_follow_activity_together() {
		let mut state = test_support::demo_state();
		state.channels.retain(|c| c.guild.is_some());
		for (id, kind, latest) in [
			(30, 1, Some(100)),
			(31, 1, Some(300)),
			(32, 3, Some(200)),
			(33, 3, None),
			(34, 1, None),
			(35, 3, Some(300)),
		] {
			let mut dm = channel(id, kind, 0, None);
			dm.guild = None;
			dm.last_message = latest.map(Id);
			state.channels.push(dm);
		}
		let order = |state: &State| {
			rows(state, None, &BTreeSet::new(), true)
				.into_iter()
				.filter_map(|row| match row {
					Row::Channel(channel, _) => Some(channel.id.0),
					_ => None,
				})
				.collect::<Vec<_>>()
		};
		assert_eq!(order(&state), [35, 31, 32, 30, 34, 33]);
		for latest in [model::Patch::Value(Id(90)), model::Patch::Null] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::ReadState(client_core::read_state::Event::Latest(vec![
					(Id(30), latest),
				])),
			});
			assert_eq!(order(&state), [35, 31, 32, 30, 34, 33]);
		}
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Message(test_support::message(400, Id(32))),
		});
		assert_eq!(order(&state), [32, 35, 31, 30, 34, 33]);
		// Delayed older messages and deletion must not undo recent activity.
		for event in [
			client_core::Event::Message(test_support::message(150, Id(32))),
			client_core::Event::Delete {
				channel: Id(32),
				id: Id(400),
			},
			client_core::Event::Delete {
				channel: Id(35),
				id: Id(300),
			},
			client_core::Event::DeleteBulk {
				channel: Id(31),
				ids: vec![Id(300)],
			},
		] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event,
			});
			assert_eq!(order(&state), [32, 35, 31, 30, 34, 33]);
		}
		assert_eq!(state.selected, Some(Id(20)));
		let Some(client_core::Command::History { request, .. }) = state.select(Id(30)) else {
			panic!("Synthetic DM requests history");
		};
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::History {
				channel: Id(30),
				request,
				older: false,
				messages: vec![],
			},
		});
		state
			.drafts
			.insert(Id(30), "Synthetic outgoing activity".into());
		let Some(client_core::Command::Send { nonce, .. }) = state.prepare_send() else {
			panic!("Synthetic DM can send");
		};
		assert_eq!(order(&state), [32, 35, 31, 30, 34, 33]);
		let mut sent = test_support::message(600, Id(30));
		sent.nonce = Some(nonce.clone());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::SendResult {
				nonce,
				result: Ok(sent),
			},
		});
		assert_eq!(order(&state), [30, 32, 35, 31, 34, 33]);
		assert_eq!(state.selected, Some(Id(30)));
		state.channels.retain(|c| c.guild.is_some());
		assert!(order(&state).is_empty());
	}
	#[test]
	fn unsupported_channel_opens_confirmation_by_keyboard_without_selecting() {
		let mut state = State {
			user: Some(model::User {
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				id: Id(100),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			channels: vec![channel(9, 13, 0, None)],
			demo: true,
			..State::default()
		};
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		let mut view = MessagingUi {
			guild: Some(Id(100)),
			..Default::default()
		};
		for permitted in [true, false] {
			if !permitted {
				state.permissions = Default::default();
			}
			view.timeline.opening = None;
			let ctx = egui::Context::default();
			for key in [egui::Key::Tab, egui::Key::Enter] {
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(240.0, 180.0),
						)),
						events: vec![egui::Event::Key {
							key,
							physical_key: None,
							pressed: true,
							repeat: false,
							modifiers: egui::Modifiers::NONE,
						}],
						..Default::default()
					},
					|ui| {
						assert!(view.channel_list(ui, &mut state).is_none());
						assert!(ui.min_rect().right() <= ui.max_rect().right() + 1.0);
					},
				);
				assert!(output.platform_output.commands.is_empty());
				output.drop_without_applying_deltas();
			}
			assert_eq!(
				view.timeline.opening.as_deref(),
				permitted.then_some("https://discord.com/channels/100/9")
			);
			assert!(state.selected.is_none());
		}
		view.timeline.opening = Some("https://discord.com/channels/100/9".into());
		view.clear();
		assert!(view.timeline.opening.is_none());
	}

	#[test]
	fn service_order_orphans_collapsed_selection_and_category_buttons() {
		let channels = vec![
			channel(8, 0, 2, Some(Id(4))),
			channel(4, 4, 1, None),
			channel(7, 0, 2, Some(Id(4))),
			channel(9, 2, 0, Some(Id(5))),
			channel(5, 4, 2, None),
			channel(3, 0, 0, Some(Id(99))),
			channel(2, 0, 1, None),
		];
		let ids = |rows: Vec<Row<'_>>| {
			rows.into_iter()
				.map(|r| match r {
					Row::Channel(c, _) | Row::Category(c, _) => c.id.0,
					Row::Participant(entry) => entry.participant.user.0,
					Row::Section(_) => 0,
				})
				.collect::<Vec<_>>()
		};
		let layout = State {
			channels: channels.clone(),
			..State::default()
		};
		assert_eq!(
			ids(rows(&layout, Some(Id(100)), &BTreeSet::new(), true,)),
			[3, 2, 4, 7, 8, 5, 9]
		);
		assert_eq!(
			ids(rows(&layout, Some(Id(100)), &BTreeSet::from([Id(4)]), true,)),
			[3, 2, 4, 5, 9]
		);
		let mut hierarchy = vec![
			channel(4, 4, 0, None),
			channel(7, 15, 0, Some(Id(4))),
			channel(8, 11, 1, Some(Id(7))),
			channel(9, 12, 0, Some(Id(7))),
			channel(10, 0, 1, Some(Id(4))),
			channel(11, 10, 0, Some(Id(10))),
			channel(12, 16, 2, Some(Id(4))),
			channel(13, 11, 0, Some(Id(12))),
			channel(20, 11, 0, Some(Id(999))), // missing parent
			channel(21, 11, 0, Some(Id(4))),   // category is not a thread parent
			channel(22, 11, 0, Some(Id(23))),  // thread-parent cycle
			channel(23, 12, 0, Some(Id(22))),
			channel(24, 11, 0, Some(Id(24))), // self parent
			channel(25, 11, 0, Some(Id(26))), // other guild
			channel(26, 0, 0, None),
			channel(27, 11, 0, Some(Id(28))), // parent/child source cycle
			channel(28, 0, 0, Some(Id(27))),
		];
		hierarchy.iter_mut().find(|c| c.id == Id(26)).unwrap().guild = Some(Id(101));
		let hierarchy_state = State {
			channels: hierarchy.clone(),
			..State::default()
		};
		let expanded = rows(&hierarchy_state, Some(Id(100)), &BTreeSet::new(), true);
		assert_eq!(expanded.len(), hierarchy.len() - 1);
		assert_eq!(
			ids(expanded),
			[20, 21, 22, 23, 24, 25, 27, 28, 4, 7, 9, 8, 10, 11, 12, 13]
		);
		let collapsed = rows(
			&hierarchy_state,
			Some(Id(100)),
			&BTreeSet::from([Id(4)]),
			true,
		);
		assert_eq!(ids(collapsed), [20, 21, 22, 23, 24, 25, 27, 28, 4]);
		assert!(!hierarchy[1].supports_text() && !hierarchy[6].supports_text());
		assert!(hierarchy[2].supports_text());
		assert_eq!(kind_label(16), "Media · loaded posts");
		assert!(!channels[1].supports_text());
		assert_eq!(kind_label(15), "Forum · loaded posts");
		let mut state = State {
			user: Some(model::User {
				id: Id(2),
				name: "Synthetic member".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				id: Id(100),
				name: "Synthetic guild".into(),
				icon: None,
				emojis: None,
			}],
			channels,
			demo: true,
			..State::default()
		};
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		assert!(state.select(Id(4)).is_none());
		let ctx = egui::Context::default();
		let mut view = MessagingUi {
			guild: Some(Id(100)),
			collapsed_categories: BTreeSet::from([Id(999)]),
			..MessagingUi::default()
		};
		let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
			assert!(view.channel_list(ui, &mut state).is_none());
		});
		output.textures_delta.clear();
		assert!(view.collapsed_categories.is_empty());
		assert!(state.selected.is_none());
		// A category is a keyboard-operable button, never a history-selection command.
		state.channels = vec![channel(4, 4, 0, None), channel(8, 0, 0, Some(Id(4)))];
		state.selected = Some(Id(8));
		// Direct fixture replacement must invalidate derived views, as State::apply does.
		state.revision += 1;
		state.invalidate_navigation();
		let ctx = egui::Context::default();
		for key in [egui::Key::Tab, egui::Key::Enter] {
			let input = egui::RawInput {
				events: vec![egui::Event::Key {
					key,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}],
				..Default::default()
			};
			let mut output = ctx.run_ui(input, |ui| {
				assert!(view.channel_list(ui, &mut state).is_none());
			});
			output.textures_delta.clear();
		}
		assert!(view.collapsed_categories.contains(&Id(4)));
		assert_eq!(state.selected, Some(Id(8)));
		ctx.run_ui(egui::RawInput::default(), |ui| {
			assert!(view.channel_list(ui, &mut state).is_none());
		})
		.drop_without_applying_deltas();
		assert!(matches!(
			view.channel_cache.rows.as_slice(),
			[CachedRow::Category(_, 1)]
		));
		// Forum containers never request history; their loaded posts remain keyboard-selectable.
		state.channels = vec![channel(7, 15, 0, None), channel(8, 11, 0, Some(Id(7)))];
		state.revision += 1;
		state.invalidate_navigation();
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		assert!(state.select(Id(7)).is_none());
		let ctx = egui::Context::default();
		let mut picked = None;
		// The forum row itself is a destination; the second Tab reaches the loaded post.
		for key in [egui::Key::Tab, egui::Key::Tab, egui::Key::Enter] {
			ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
					..Default::default()
				},
				|ui| {
					picked = view.channel_list(ui, &mut state).or(picked);
				},
			)
			.drop_without_applying_deltas();
		}
		assert_eq!(picked, Some(Id(8)));
		assert!(view.archive_parent.is_none());
	}
}
