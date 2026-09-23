//! Server rail, channel list with categories and threads, and the account panel.
use crate::nav_menu::Target;
use crate::theme::{Icon, color, icon, palette};
use crate::{Serein, channel_label, text_channel, tooltip};
use gpui::{prelude::*, *};
use model::Id;
use std::collections::{BTreeMap, BTreeSet};

pub const RAIL_WIDTH: f32 = 68.;
/// Visible threads per parent channel, as in the main app.
const THREADS_PER_CHANNEL: usize = 3;

/// Navigation-only view state: the open right-click menu and expanded server folders.
#[derive(Default)]
pub struct NavState {
	pub menu: Option<crate::nav_menu::Menu>,
	pub expanded: BTreeSet<u64>,
}

/// One rail button; see [`Serein::rail_tile`].
pub struct Tile {
	pub id: ElementId,
	pub name: SharedString,
	pub selected: bool,
	pub unread: bool,
	/// Mention (or request) count for the red badge; zero hides it.
	pub badge: u32,
	pub size: f32,
	/// Server-style tiles fill with the accent on hover; avatars keep their picture.
	pub accent_hover: bool,
	pub menu: Option<Target>,
}

/// Badge text and pill width, as egui's `notifications::badge`.
pub fn badge_label(count: u32) -> (String, f32) {
	if count > 99 {
		("99+".into(), 30.)
	} else if count > 9 {
		(count.to_string(), 24.)
	} else {
		(count.to_string(), 19.)
	}
}

/// Inline red count pill for list rows and tabs.
pub fn count_pill(count: u32) -> Div {
	let p = palette();
	let (label, width) = badge_label(count);
	div()
		.h(px(19.))
		.w(px(width))
		.flex_none()
		.rounded(px(10.))
		.bg(color(p.danger))
		.flex()
		.items_center()
		.justify_center()
		.text_size(px(12.))
		.font_weight(FontWeight::SEMIBOLD)
		.text_color(white())
		.child(label)
}

/// The count pill ringed in the rail colour, centred 8 px in from a tile's bottom-right corner.
fn rail_badge(count: u32, size: f32, ring: Rgba) -> Div {
	let (_, width) = badge_label(count);
	div()
		.absolute()
		.left(px(size - 8. - width / 2. - 3.))
		.top(px(size - 8. - 12.5))
		.p(px(3.))
		.rounded(px(12.5))
		.bg(ring)
		.child(count_pill(count))
}

/// Home tile name with pending requests, as egui's `home_request_label`.
fn home_label(friends: u32, messages: u32) -> String {
	let mut parts = vec!["Direct Messages".to_owned()];
	for (count, one, many) in [
		(friends, "friend request", "friend requests"),
		(messages, "message request", "message requests"),
	] {
		match count {
			0 => {}
			1 => parts.push(format!("1 {one}")),
			n => parts.push(format!("{n} {many}")),
		}
	}
	parts.join(" · ")
}

pub enum NavRow {
	Category { id: Id, name: String },
	Channel { id: Id, thread: bool },
}

/// Initials for tiles and fallback avatars, matching `ui::design::paint_avatar`.
pub fn initials(name: &str) -> String {
	name.split_whitespace()
		.take(2)
		.filter_map(|part| part.chars().find(|c| c.is_alphanumeric()))
		.flat_map(char::to_uppercase)
		.take(2)
		.collect()
}

/// The user's CDN avatar once loaded, else initials on the hashed fallback colour.
pub fn avatar(name: &str, size: f32, user: Option<&model::User>) -> Div {
	let circle = div().size(px(size)).flex_none().rounded_full();
	if let Some(image) = user.and_then(|user| crate::images::get(&user.avatar_key())) {
		return circle.child(
			img(image)
				.size_full()
				.rounded_full()
				.object_fit(ObjectFit::Cover),
		);
	}
	circle
		.bg(color(ui::design::fallback_avatar_color(name)))
		.flex()
		.items_center()
		.justify_center()
		.text_size(px(size * 0.36))
		.font_weight(FontWeight::SEMIBOLD)
		.text_color(white())
		.child(initials(name))
}

pub fn presence_color(status: Option<&str>) -> Option<Rgba> {
	match status? {
		"online" => Some(rgb(0x23a559)),
		"idle" => Some(rgb(0xf0b232)),
		"dnd" => Some(rgb(0xf23f43)),
		_ => Some(rgb(0x80848e)),
	}
}

/// Avatar with a presence dot ringed in the surface colour, bottom-right.
pub fn avatar_with_presence(
	name: &str,
	size: f32,
	user: Option<&model::User>,
	status: Option<&str>,
	ring: Rgba,
) -> Div {
	let dot = (size * 0.16).clamp(4., 8.);
	div()
		.relative()
		.flex_none()
		.child(avatar(name, size, user))
		.children(presence_color(status).map(|fill| {
			div()
				.absolute()
				.right(px(-1.5))
				.bottom(px(-1.5))
				.size(px(dot * 2. + 4.))
				.rounded_full()
				.bg(ring)
				.flex()
				.items_center()
				.justify_center()
				.child(div().size(px(dot * 2.)).rounded_full().bg(fill))
		}))
}

/// "Hide Muted Channels", as egui: drops muted channels (never the open one) with their
/// threads, then categories left without channels.
fn retain_unmuted(rows: &mut Vec<NavRow>, state: &client_core::State) {
	let mut hidden_parent = false;
	rows.retain(|row| match row {
		NavRow::Channel { id, thread } => {
			let hide = state.selected != Some(*id)
				&& ((*thread && hidden_parent) || state.guild_channel_muted(*id) == Some(true));
			if !*thread {
				hidden_parent = hide;
			}
			!hide
		}
		NavRow::Category { .. } => true,
	});
	let mut index = 0;
	while index < rows.len() {
		if matches!(rows[index], NavRow::Category { .. })
			&& !matches!(rows.get(index + 1), Some(NavRow::Channel { .. }))
		{
			rows.remove(index);
		} else {
			index += 1;
		}
	}
}

fn channel_icon(channel: &model::Channel) -> Icon {
	match channel.kind {
		2 | 13 => Icon::Speaker,
		5 => Icon::Megaphone,
		10..=12 => Icon::Chats,
		15 | 16 => Icon::Forum,
		_ => Icon::Hash,
	}
}

impl Serein {
	pub(crate) fn sync_channels(&mut self) {
		// Folder settings load once per session and again when Discord reports a change; the
		// offline preview seeds its own (`--demo-folders`).
		if !self.state.demo
			&& self.state.gateway_connected
			&& !self.state.folders_pending
			&& (self.state.folders_stale
				|| (self.state.guild_folders.is_none() && self.state.folders_error.is_none()))
		{
			let command = self.state.load_guild_folders();
			self.dispatch(command);
		}
		// A server left or removed elsewhere must not keep its (now empty) channel list open.
		if self
			.guild
			.is_some_and(|guild| self.state.guild(guild).is_none())
		{
			self.guild = None;
		}
		self.nav = nav_rows(
			&self.state,
			self.guild,
			&self.collapsed,
			self.settings.show_hidden_channels,
		);
	}

	/// Per-server rail state in one pass over the channels: lit pill and mention total.
	fn guild_badges(&self) -> BTreeMap<Id, (bool, u32)> {
		let mut badges = BTreeMap::<Id, (bool, u32)>::new();
		for channel in self.state.channels.iter().take(client_core::MAX_NAV) {
			if let Some(guild) = channel.guild {
				let entry = badges.entry(guild).or_default();
				entry.0 |= self.state.lights_guild_rail(channel);
				entry.1 = entry.1.saturating_add(self.state.mention_count(channel.id));
			}
		}
		badges
	}

	pub(crate) fn guild_tile(
		&self,
		id: Id,
		badges: &BTreeMap<Id, (bool, u32)>,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let Some(guild) = self.state.guild(id) else {
			return div().into_any_element();
		};
		let (unread, mentions) = badges.get(&id).copied().unwrap_or_default();
		let label = initials(&guild.name);
		let size = if label.chars().count() > 1 { 16. } else { 18. };
		let guild_icon = guild.icon_key().and_then(|key| crate::images::get(&key));
		self.rail_tile(
			Tile {
				id: ("guild", id.0).into(),
				name: guild.name.clone().into(),
				selected: self.guild == Some(id),
				unread,
				badge: mentions,
				size: 46.,
				accent_hover: true,
				menu: Some(Target::Guild(id)),
			},
			move |this, cx| this.select_section(Some(id), cx),
			move |highlight| {
				if let Some(image) = &guild_icon {
					return div().size_full().child(
						img(image.clone())
							.size_full()
							.rounded(px(13.))
							.object_fit(ObjectFit::Cover),
					);
				}
				div()
					.size_full()
					.rounded(px(13.))
					.bg(color(if highlight { p.accent } else { p.raised }))
					.flex()
					.items_center()
					.justify_center()
					.text_size(px(size))
					.font_weight(FontWeight::MEDIUM)
					.text_color(color(if highlight {
						p.accent_text
					} else {
						p.text_strong
					}))
					.child(label.clone())
			},
			cx,
		)
	}

	/// Unread conversations as 48 px avatars under the home tile, newest first (at most 15).
	fn rail_directs(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
		let call = self
			.state
			.voice
			.active
			.as_ref()
			.filter(|call| call.guild.is_none())
			.map(|call| call.channel);
		let mut tiles = Vec::new();
		for id in self.state.unread_directs(call) {
			let Some(channel) = self.state.channel(id) else {
				continue;
			};
			let name = channel_label(channel);
			let user = channel
				.recipients
				.first()
				.filter(|_| channel.recipients.len() == 1)
				.cloned();
			let count = self.state.unread_count(id);
			let unread = self.state.channel_unread(channel) == Some(true) || count > 0;
			tiles.push(self.rail_tile(
				Tile {
					id: ("rail-direct", id.0).into(),
					name: name.clone().into(),
					selected: false,
					unread,
					badge: count,
					size: 48.,
					accent_hover: false,
					menu: Some(Target::Channel(id)),
				},
				move |this, cx| {
					this.friends.open = false;
					this.select(id, cx)
				},
				move |_| avatar(&name, 48., user.as_ref()),
				cx,
			));
		}
		tiles
	}

	fn rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let home = self.guild.is_none();
		let (friend_requests, message_requests) = self.state.home_request_parts();
		let badges = self.guild_badges();
		let mut servers = Vec::new();
		for entry in crate::folders::entries(&self.state, &self.navigation.expanded) {
			servers.push(match entry {
				crate::folders::Entry::Server(id) => self.guild_tile(id, &badges, cx),
				folder => self.folder_entry(&folder, &badges, cx),
			});
		}
		div()
			.id("rail")
			.w(px(RAIL_WIDTH))
			.h_full()
			.flex_none()
			.overflow_y_scroll()
			.pt_1()
			.pb_2()
			.flex()
			.flex_col()
			.items_center()
			.gap(px(12.))
			.child(self.rail_tile(
				Tile {
					id: "home".into(),
					name: home_label(friend_requests, message_requests).into(),
					selected: home,
					unread: false,
					badge: friend_requests.saturating_add(message_requests),
					size: 46.,
					accent_hover: true,
					menu: None,
				},
				|this, cx| this.select_section(None, cx),
				move |highlight| {
					div()
						.size_full()
						.rounded(px(13.))
						.bg(color(if highlight { p.accent } else { p.raised }))
						.flex()
						.items_center()
						.justify_center()
						.child(icon(
							Icon::Serein,
							px(25.),
							color(if highlight {
								p.accent_text
							} else {
								p.text_strong
							}),
						))
				},
				cx,
			))
			.children(self.rail_directs(cx))
			.child(
				div()
					.w(px(32.))
					.h(px(2.))
					.flex_none()
					.rounded(px(1.))
					.bg(color(p.raised)),
			)
			.children(servers)
	}

	/// A rail button with the edge pill (tall when selected, medium on hover, a dot for unread),
	/// an optional mention badge bottom-right and an optional right-click menu.
	pub(crate) fn rail_tile(
		&self,
		tile: Tile,
		on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
		content: impl Fn(bool) -> Div,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let Tile {
			id,
			name,
			selected,
			unread,
			badge,
			size,
			accent_hover,
			menu,
		} = tile;
		let group: SharedString = format!("rail-{id}").into();
		let middle = size / 2.;
		let (pill, hover) = (
			if selected {
				40.
			} else if unread {
				8.
			} else {
				0.
			},
			20.,
		);
		let radius = if accent_hover { 13. } else { middle };
		div()
			.relative()
			.w_full()
			.flex()
			.justify_center()
			.group(group.clone())
			.child(
				div()
					.absolute()
					.left(px(-4.))
					.top(px(middle - pill / 2.))
					.w(px(8.))
					.h(px(pill))
					.rounded(px(4.))
					.bg(color(p.text_strong))
					.when(!selected, |d| {
						d.group_hover(group.clone(), |d| {
							d.h(px(hover)).top(px(middle - hover / 2.))
						})
					}),
			)
			.child(
				div()
					.id(id)
					.relative()
					.focusable()
					.tab_stop(true)
					.size(px(size))
					.flex_none()
					.cursor_pointer()
					.rounded(px(radius))
					.focus(|d| d.border_2().border_color(color(p.text_strong)))
					.tooltip(tooltip(name))
					.on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
					.when_some(menu, |d, target| {
						d.on_mouse_down(
							MouseButton::Right,
							cx.listener(move |this, event: &MouseDownEvent, window, cx| {
								this.open_nav_menu(target, event.position, window, cx);
								cx.stop_propagation();
							}),
						)
					})
					.child(
						div()
							.size_full()
							.when(!selected && accent_hover, |d| {
								d.child(
									content(false)
										.group_hover(group.clone(), |d| d.bg(color(p.accent))),
								)
							})
							.when(!selected && !accent_hover, |d| d.child(content(false)))
							.when(selected, |d| d.child(content(true))),
					)
					.when(badge > 0, |d| {
						d.child(rail_badge(badge, size, color(p.base)))
					}),
			)
			.into_any_element()
	}

	/// The channel list follows "Sidebar width", narrowing (not saving) so the conversation
	/// keeps 260 px, like the main app.
	pub(crate) fn list_width(&self, window: &Window) -> Pixels {
		let room = window.viewport_size().width - px(RAIL_WIDTH + 260.);
		px(f32::from(self.settings.reading.sidebar_width)).min(room.clamp(px(190.), px(360.)))
	}

	fn channel_list(&self, width: Pixels, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let title = match self.guild {
			Some(id) => self
				.state
				.guild(id)
				.map_or_else(String::new, |g| g.name.clone()),
			None => "Direct Messages".into(),
		};
		div()
			.w(width)
			.h_full()
			.flex_none()
			.bg(color(p.sidebar))
			.rounded_l(px(8.))
			.flex()
			.flex_col()
			.child(
				div()
					.h(px(48.))
					.flex_none()
					.px_4()
					.border_b_1()
					.border_color(color(p.border))
					.flex()
					.items_center()
					.justify_between()
					.child(
						div()
							.flex_1()
							.min_w_0()
							.overflow_hidden()
							.text_ellipsis()
							.whitespace_nowrap()
							.text_size(px(15.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.text_strong))
							.child(title),
					)
					.when(self.guild.is_some(), |d| {
						d.child(icon(Icon::CaretDown, px(14.), color(p.muted)))
					}),
			)
			.child(
				div()
					.id("channels")
					.flex_1()
					.min_h_0()
					.overflow_y_scroll()
					.px_2()
					.pt_2()
					.pb_2()
					.flex()
					.flex_col()
					.when(self.guild.is_none(), |d| d.child(self.friends_row(cx)))
					.when(self.nav.is_empty(), |d| {
						d.child(div().p_2().text_sm().text_color(color(p.muted)).child(
							if self.guild.is_some() {
								"No text channels you can view."
							} else {
								"No direct messages yet."
							},
						))
					})
					.children(self.nav.iter().map(|row| match row {
						NavRow::Category { id, name } => {
							self.category_row(*id, name, cx).into_any_element()
						}
						NavRow::Channel { id, thread } => {
							self.channel_row(*id, *thread, cx).into_any_element()
						}
					})),
			)
	}

	fn category_row(&self, id: Id, name: &str, cx: &mut Context<Self>) -> Stateful<Div> {
		let p = palette();
		let collapsed = self.collapsed.contains(&id);
		let group: SharedString = format!("category-{id}").into();
		div()
			.id(("category", id.0))
			.group(group.clone())
			.h(px(40.))
			.pt(px(16.))
			.pl(px(2.))
			.flex()
			.items_center()
			.gap(px(4.))
			.cursor_pointer()
			.on_click(cx.listener(move |this, _, _, cx| {
				if !this.collapsed.remove(&id) {
					this.collapsed.insert(id);
				}
				this.sync_channels();
				cx.notify();
			}))
			.on_mouse_down(
				MouseButton::Right,
				cx.listener(move |this, event: &MouseDownEvent, window, cx| {
					this.open_nav_menu(Target::Channel(id), event.position, window, cx);
					cx.stop_propagation();
				}),
			)
			.child(
				icon(
					if collapsed {
						Icon::CaretRight
					} else {
						Icon::CaretDown
					},
					px(12.),
					color(p.muted),
				)
				.group_hover(group.clone(), |s| s.text_color(color(p.text_strong))),
			)
			.child(
				div()
					.text_size(px(12.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.muted))
					.group_hover(group, |s| s.text_color(color(p.text_strong)))
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.child(name.to_owned()),
			)
	}

	fn channel_row(&self, id: Id, thread: bool, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let Some(channel) = self.state.channel(id) else {
			return div().into_any_element();
		};
		let selected = self.state.selected == Some(id);
		let forum = self.state.is_forum(id);
		// Forum containers carry no messages; they are unread when one of their posts is.
		let unread = if forum {
			self.state.forum_unread(id)
		} else {
			self.state.channel_unread(channel) == Some(true)
		};
		// Muted and hidden rows are dimmed and never show the unread pill, as egui's
		// `channel_marks`; hidden ones (listed by "Show hidden channels") cannot be opened.
		let access = self.state.channel_access(id);
		let hidden = access.hidden();
		let dim = access.dim();
		let mentions = if hidden {
			0
		} else {
			self.state.mention_count(id)
		};
		let openable = (text_channel(channel) || forum) && !hidden;
		let unread = unread && !dim;
		let strong = selected || unread;
		let name = channel_label(channel);
		let direct = channel.guild.is_none();
		let text = if dim {
			crate::theme::tint(p.muted, 0.6)
		} else {
			color(if strong { p.text_strong } else { p.muted })
		};
		let row = div()
			.id(("channel", id.0))
			.relative()
			.flex_none()
			.h(px(if direct { 44. } else { 34. }))
			.my(px(0.5))
			.ml(px(if thread { 14. } else { 0. }))
			.px_2()
			.rounded(px(8.))
			.flex()
			.items_center()
			.gap(px(if direct { 12. } else { 6. }))
			.text_color(text)
			.when(selected, |d| d.bg(color(p.selected)))
			.when(!selected && !dim, |d| {
				d.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
			})
			.when(!selected && dim, |d| d.hover(|d| d.bg(color(p.hover))))
			.when(openable, |d| {
				d.cursor_pointer()
					.focusable()
					.tab_stop(true)
					.focus(|d| d.bg(color(p.hover)))
					.on_click(cx.listener(move |this, _, _, cx| {
						this.friends.open = false;
						this.select(id, cx)
					}))
			})
			.on_mouse_down(
				MouseButton::Right,
				cx.listener(move |this, event: &MouseDownEvent, window, cx| {
					this.open_nav_menu(Target::Channel(id), event.position, window, cx);
					cx.stop_propagation();
				}),
			)
			.when(!openable, |d| {
				d.tooltip(tooltip(if hidden {
					"Hidden · you cannot view this channel"
				} else {
					"Voice is available in the main Serein app"
				}))
			})
			.when(unread && !selected, |d| {
				d.child(
					div()
						.absolute()
						.left(px(-8.))
						.top(px(if direct { 18. } else { 13. }))
						.w(px(4.))
						.h(px(8.))
						.rounded(px(2.))
						.bg(color(p.text_strong)),
				)
			});
		let row = if direct {
			let status = channel
				.recipients
				.first()
				.filter(|_| channel.recipients.len() == 1)
				.and_then(|user| self.state.presence_for(user.id))
				.and_then(|presence| presence.status.as_deref());
			let subtitle = if channel.recipients.len() > 1 {
				Some(format!("{} members", channel.recipients.len() + 1))
			} else {
				None
			};
			row.child(avatar_with_presence(
				&name,
				32.,
				channel
					.recipients
					.first()
					.filter(|_| channel.recipients.len() == 1),
				status,
				color(if selected { p.selected } else { p.sidebar }),
			))
			.child(
				div()
					.flex_1()
					.min_w_0()
					.flex()
					.flex_col()
					.child(
						div()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(15.))
							.font_weight(FontWeight::MEDIUM)
							.child(name),
					)
					.children(subtitle.map(|subtitle| {
						div()
							.text_size(px(12.))
							.text_color(color(p.muted))
							.child(subtitle)
					})),
			)
		} else {
			row.child(icon(channel_icon(channel), px(20.), text).when(!strong, |d| d.opacity(0.85)))
				.child(
					div()
						.flex_1()
						.min_w_0()
						.overflow_hidden()
						.whitespace_nowrap()
						.text_ellipsis()
						.text_size(px(15.))
						.font_weight(FontWeight::MEDIUM)
						.child(name),
				)
		};
		row.when(mentions > 0, |d| d.child(count_pill(mentions)))
			.into_any_element()
	}

	/// "Friends" above the direct messages; opens the Friends page in the main area.
	fn friends_row(&self, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let selected = self.friends_visible();
		let requests = self
			.state
			.pending_friends()
			.filter(|(_, _, incoming)| *incoming)
			.count();
		let text = color(if selected { p.text_strong } else { p.muted });
		div()
			.id("friends-row")
			.flex_none()
			.h(px(44.))
			.mb(px(4.))
			.px_2()
			.rounded(px(8.))
			.flex()
			.items_center()
			.gap(px(12.))
			.cursor_pointer()
			.focusable()
			.tab_stop(true)
			.text_color(text)
			.when(selected, |d| d.bg(color(p.selected)))
			.when(!selected, |d| {
				d.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
			})
			.focus(|d| d.bg(color(p.hover)))
			.on_click(cx.listener(|this, _, _, cx| this.open_friends(cx)))
			.child(
				div()
					.size(px(32.))
					.flex_none()
					.flex()
					.items_center()
					.justify_center()
					.child(icon(Icon::Users, px(24.), text)),
			)
			.child(
				div()
					.flex_1()
					.text_size(px(15.))
					.font_weight(FontWeight::MEDIUM)
					.child("Friends"),
			)
			.when(requests > 0, |d| d.child(count_pill(requests as u32)))
			.into_any_element()
	}

	fn user_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let (name, status) = match &self.state.user {
			Some(user) => (
				self.state.user_display_name(user).to_owned(),
				if self.state.demo {
					"Offline preview"
				} else if self.state.gateway_connected {
					"Online"
				} else {
					"Reconnecting…"
				},
			),
			None => (String::new(), ""),
		};
		let presence = if self.state.demo || self.state.gateway_connected {
			Some("online")
		} else {
			Some("offline")
		};
		div().p_2().flex_none().child(
			div()
				.h(px(48.))
				.px_2()
				.rounded(px(8.))
				.bg(color(p.raised))
				.flex()
				.items_center()
				.gap_2()
				.child(avatar_with_presence(
					&name,
					32.,
					self.state.user.as_ref(),
					presence,
					color(p.raised),
				))
				.child(
					div()
						.flex_1()
						.min_w_0()
						.flex()
						.flex_col()
						.child(
							div()
								.overflow_hidden()
								.whitespace_nowrap()
								.text_ellipsis()
								.text_size(px(14.))
								.font_weight(FontWeight::SEMIBOLD)
								.text_color(color(p.text_strong))
								.child(name),
						)
						.child(
							div()
								.overflow_hidden()
								.whitespace_nowrap()
								.text_ellipsis()
								.text_size(px(12.))
								.text_color(color(p.muted))
								.child(status),
						),
				)
				.child(
					div()
						.id("settings-gear")
						.focusable()
						.tab_stop(true)
						.size(px(32.))
						.flex_none()
						.rounded(px(6.))
						.flex()
						.items_center()
						.justify_center()
						.cursor_pointer()
						.hover(|d| d.bg(color(p.hover)))
						.focus(|d| d.bg(color(p.hover)))
						.tooltip(crate::tooltip("User Settings"))
						.on_click(
							cx.listener(|this, _, window, cx| this.open_settings(None, window, cx)),
						)
						.child(icon(Icon::Gear, px(20.), color(p.muted))),
				),
		)
	}

	pub(crate) fn render_navigation(
		&self,
		window: &Window,
		cx: &mut Context<Self>,
	) -> impl IntoElement {
		let width = self.list_width(window);
		div()
			.w(px(RAIL_WIDTH) + width)
			.h_full()
			.flex_none()
			.flex()
			.flex_col()
			.child(
				div()
					.flex_1()
					.min_h_0()
					.flex()
					.child(self.rail(cx))
					.child(self.channel_list(width, cx)),
			)
			.child(self.user_panel(cx))
			.children(self.render_nav_menu(cx))
	}
}

/// Channel-list rows for `guild` (direct messages for `None`). Channels the user cannot view
/// are listed only with `show_hidden`, like the main app's "Show hidden channels".
fn nav_rows(
	state: &client_core::State,
	guild: Option<Id>,
	collapsed: &BTreeSet<Id>,
	show_hidden: bool,
) -> Vec<NavRow> {
	let visible = |c: &&model::Channel| {
		c.guild == guild
			&& (c.supports_text() || matches!(c.kind, 2 | 13 | 15 | 16))
			&& !matches!(c.kind, 10..=12)
			&& (show_hidden || state.can_view(c.id))
	};
	let mut rows = Vec::new();
	if guild.is_none() {
		let mut direct = state
			.channels
			.iter()
			.filter(|c| c.guild.is_none() && c.supports_text() && state.can_view(c.id))
			.collect::<Vec<_>>();
		// Most recent conversation first, like the main app's DM list.
		direct.sort_by_key(|c| std::cmp::Reverse(c.last_message.unwrap_or(c.id)));
		rows.extend(direct.into_iter().map(|c| NavRow::Channel {
			id: c.id,
			thread: false,
		}));
		return rows;
	}
	let mut channels = state.channels.iter().filter(visible).collect::<Vec<_>>();
	channels.sort_by_key(|c| (matches!(c.kind, 2 | 13), c.position, c.id));
	let mut categories = state
		.channels
		.iter()
		.filter(|c| c.guild == guild && c.kind == 4)
		.collect::<Vec<_>>();
	categories.sort_by_key(|c| (c.position, c.id));
	let threads = |parent: Id| {
		let mut threads = state
			.channels
			.iter()
			.filter(|c| {
				c.parent_id == Some(parent)
					&& matches!(c.kind, 10..=12)
					&& (show_hidden || state.can_view(c.id))
			})
			.collect::<Vec<_>>();
		threads.sort_by_key(|c| std::cmp::Reverse(c.last_message.unwrap_or(c.id)));
		threads.truncate(THREADS_PER_CHANNEL);
		threads
	};
	let push = |rows: &mut Vec<NavRow>, channel: &model::Channel, collapsed: bool| {
		let selected = state.selected == Some(channel.id);
		if !collapsed || selected {
			rows.push(NavRow::Channel {
				id: channel.id,
				thread: false,
			});
		}
		for thread in threads(channel.id) {
			if !collapsed || state.selected == Some(thread.id) {
				rows.push(NavRow::Channel {
					id: thread.id,
					thread: true,
				});
			}
		}
	};
	for channel in channels.iter().filter(|c| {
		c.parent_id
			.is_none_or(|parent| !categories.iter().any(|category| category.id == parent))
	}) {
		push(&mut rows, channel, false);
	}
	for category in &categories {
		let children = channels
			.iter()
			.filter(|c| c.parent_id == Some(category.id))
			.collect::<Vec<_>>();
		if children.is_empty() && !show_hidden {
			continue;
		}
		rows.push(NavRow::Category {
			id: category.id,
			name: category.name.to_uppercase(),
		});
		let collapsed = collapsed.contains(&category.id);
		for channel in children {
			push(&mut rows, channel, collapsed);
		}
	}
	if guild.is_some_and(|guild| state.hides_muted_channels(guild) == Some(true)) {
		retain_unmuted(&mut rows, state);
	}
	rows
}

#[cfg(test)]
mod tests {
	use super::{NavRow, badge_label, home_label, nav_rows, retain_unmuted};
	use model::Id;
	use std::collections::BTreeSet;

	#[test]
	fn hidden_channels_are_listed_only_when_opted_in() {
		let mut state = test_support::chat_demo_state();
		test_support::seed_access_marks(&mut state);
		// #secret (62) denies @everyone; #staff-notes (61) is allowed through a role.
		assert!(!state.can_view(Id(62)) && state.can_view(Id(61)));
		let ids = |show_hidden| {
			nav_rows(&state, Some(Id(10)), &BTreeSet::new(), show_hidden)
				.iter()
				.map(|row| match row {
					NavRow::Category { id, .. } | NavRow::Channel { id, .. } => id.0,
				})
				.collect::<Vec<_>>()
		};
		let visible = ids(false);
		assert!(visible.contains(&60) && visible.contains(&61));
		assert!(!visible.contains(&62));
		let all = ids(true);
		assert!(all.contains(&62));
		assert!(all.len() > visible.len());
	}

	#[test]
	fn hiding_muted_channels_drops_them_and_their_empty_category() {
		// The fixture mutes #long-form (21) and leaves #getting-started (20) unmuted.
		let mut state = test_support::chat_demo_state();
		state.selected = None;
		assert_eq!(state.guild_channel_muted(Id(21)), Some(true));
		assert_eq!(state.guild_channel_muted(Id(20)), Some(false));
		let rows = || {
			vec![
				NavRow::Category {
					id: Id(23),
					name: "WELCOME".into(),
				},
				NavRow::Channel {
					id: Id(20),
					thread: false,
				},
				NavRow::Channel {
					id: Id(27),
					thread: true,
				},
				NavRow::Category {
					id: Id(24),
					name: "CONVERSATIONS".into(),
				},
				NavRow::Channel {
					id: Id(21),
					thread: false,
				},
			]
		};
		let ids = |rows: &[NavRow]| {
			rows.iter()
				.map(|row| match row {
					NavRow::Category { id, .. } | NavRow::Channel { id, .. } => id.0,
				})
				.collect::<Vec<_>>()
		};
		let mut hidden = rows();
		retain_unmuted(&mut hidden, &state);
		assert_eq!(ids(&hidden), [23, 20, 27]);
		// The open channel stays listed even while muted.
		state.selected = Some(Id(21));
		let mut open = rows();
		retain_unmuted(&mut open, &state);
		assert_eq!(ids(&open), [23, 20, 27, 24, 21]);
	}

	#[test]
	fn badges_widen_with_digits_and_cap_at_99() {
		assert_eq!(badge_label(1), ("1".into(), 19.));
		assert_eq!(badge_label(42), ("42".into(), 24.));
		assert_eq!(badge_label(99), ("99".into(), 24.));
		assert_eq!(badge_label(100), ("99+".into(), 30.));
		assert_eq!(badge_label(u32::MAX).0, "99+");
	}

	#[test]
	fn home_label_lists_pending_requests() {
		assert_eq!(home_label(0, 0), "Direct Messages");
		assert_eq!(
			home_label(2, 1),
			"Direct Messages · 2 friend requests · 1 message request"
		);
	}
}
