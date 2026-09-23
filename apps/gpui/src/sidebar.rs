//! Server rail, channel list with categories, shortcut shelves and opened threads, and the
//! account panel.
use crate::nav_menu::Target;
use crate::theme::{Icon, color, icon, palette};
use crate::{Serein, text_channel, tooltip};
use gpui::{prelude::*, *};
use model::Id;
use std::collections::{BTreeMap, BTreeSet};

pub const RAIL_WIDTH: f32 = 68.;
/// Opened threads listed under their parent channel, as egui's `MAX_VISIBLE_THREADS`.
const THREADS_PER_CHANNEL: usize = 4;

/// Navigation-only view state: the open right-click menu, expanded server folders and the
/// shortcut shelves (egui's device-local Favorites and pinned DMs; session-only here).
#[derive(Default)]
pub struct NavState {
	pub menu: Option<crate::nav_menu::Menu>,
	pub expanded: BTreeSet<u64>,
	pub favorites: Vec<Id>,
	pub pinned: Vec<Id>,
}

/// Adds `id` to a shortcut shelf, or removes it; bounded like `ChannelPreferences`.
pub fn toggle_shortcut(shelf: &mut Vec<Id>, id: Id) {
	if let Some(index) = shelf.iter().position(|entry| *entry == id) {
		shelf.remove(index);
	} else if shelf.len() < model::ChannelPreferences::MAX_ENTRIES {
		shelf.push(id);
	}
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

/// Eyebrow rows above the shortcut shelves and the home remainder, as egui's `Heading`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heading {
	Favorites,
	Pinned,
	DirectMessages,
}
impl Heading {
	fn label(self) -> &'static str {
		match self {
			Self::Favorites => "FAVORITES",
			Self::Pinned => "PINNED",
			Self::DirectMessages => "DIRECT MESSAGES",
		}
	}
}

#[derive(Debug, PartialEq, Eq)]
pub enum NavRow {
	Heading(Heading),
	/// `count`: listed rows under the category, for its tooltip.
	Category {
		id: Id,
		name: String,
		count: usize,
	},
	/// `shelf`: a Favorites or Pinned copy, ruled off from the list below.
	Channel {
		id: Id,
		thread: bool,
		shelf: bool,
	},
}
impl NavRow {
	fn shelf(&self) -> bool {
		match self {
			Self::Heading(heading) => *heading != Heading::DirectMessages,
			Self::Channel { shelf, .. } => *shelf,
			Self::Category { .. } => false,
		}
	}
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

/// The user's avatar (CDN picture, or the offline preview's silhouette), else initials on the
/// hashed fallback colour, as egui's `Avatars::paint_user` and `design::paint_avatar`.
pub fn avatar(name: &str, size: f32, user: Option<&model::User>) -> Div {
	let circle = div().size(px(size)).flex_none().rounded_full();
	if let Some(image) = user.and_then(crate::images::user) {
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

/// Group DM picture, as egui's `Avatars::group_avatar`: two members overlapped diagonally (the
/// second ringed in `ring`), a lone member filling the circle, or a people glyph otherwise.
pub fn group_avatar(channel: &model::Channel, size: f32, ring: Rgba) -> Div {
	let p = palette();
	let users = &channel.recipients;
	match users.as_slice() {
		[user] if channel.icon.is_none() => avatar(&user.name, size, Some(user)),
		[first, second, ..] if channel.icon.is_none() => {
			let small = size * 0.66;
			let halo = size * 0.04;
			div()
				.relative()
				.size(px(size))
				.flex_none()
				.child(div().absolute().left_0().top_0().child(avatar(
					&first.name,
					small,
					Some(first),
				)))
				.child(
					div()
						.absolute()
						.left(px(size - small - halo))
						.top(px(size - small - halo))
						.p(px(halo))
						.rounded_full()
						.bg(ring)
						.child(avatar(&second.name, small, Some(second))),
				)
		}
		_ => div()
			.size(px(size))
			.flex_none()
			.rounded_full()
			.bg(color(p.raised))
			.flex()
			.items_center()
			.justify_center()
			.child(icon(Icon::Users, px(size * 0.56), color(p.muted))),
	}
}

/// A server tag chip beside a name, as egui's `profiles::server_tag`.
fn server_tag(tag: &model::ClanTag) -> Div {
	let p = palette();
	div()
		.flex_none()
		.flex()
		.items_center()
		.gap(px(3.))
		.px(px(4.))
		.py(px(1.))
		.rounded(px(4.))
		.bg(color(p.raised))
		.when_some(tag.badge_key(), |d, key| {
			d.child(match crate::images::badge(&key) {
				Some(image) => img(image)
					.size(px(10.))
					.flex_none()
					.rounded(px(2.))
					.into_any_element(),
				None => div()
					.size(px(8.))
					.m(px(1.))
					.flex_none()
					.rounded_full()
					.bg(color(p.border))
					.into_any_element(),
			})
		})
		.child(
			div()
				.text_size(px(10.))
				.line_height(px(14.))
				.text_color(color(p.text_strong))
				.child(tag.tag.clone()),
		)
}

/// Activity or custom status under a DM name, as egui's `profiles::subtitle`.
fn presence_subtitle(presence: &model::MemberPresence) -> Option<String> {
	presence
		.activities
		.first()
		.map(|activity| {
			if activity.kind == 2 && activity.name.eq_ignore_ascii_case("Spotify") {
				activity.state.clone().unwrap_or_else(|| activity.summary())
			} else {
				activity.summary()
			}
		})
		.or_else(|| presence.custom_status.clone())
}

/// A 1 px rule under the last shortcut row, 7 px below it, as egui's `paint_shelf_rule`.
fn shelf_rule(top: f32) -> Div {
	div()
		.absolute()
		.left(px(8.))
		.right(px(8.))
		.top(px(top))
		.h(px(1.))
		.bg(color(palette().border))
}

/// "Hide Muted Channels", as egui: drops muted channel rows (never the open one), then
/// headings left without a channel below them.
fn retain_unmuted(rows: &mut Vec<NavRow>, state: &client_core::State) {
	rows.retain(|row| {
		!matches!(row, NavRow::Channel { id, .. }
			if state.selected != Some(*id) && state.guild_channel_muted(*id) == Some(true))
	});
	let mut index = 0;
	while index < rows.len() {
		if matches!(rows[index], NavRow::Heading(_))
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
		10..=12 => Icon::Thread,
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
			&self.navigation,
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
		let guild_icon = crate::images::guild(guild);
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
					.text_color(color(if highlight { p.accent_text } else { p.text }))
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
			let name = self.state.conversation_name(channel).to_owned();
			let picture = channel.clone();
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
				move |_| {
					if picture.kind == 3 {
						group_avatar(&picture, 48., color(palette().sidebar))
					} else if let Some(user) = picture.recipients.first() {
						avatar(&user.name, 48., Some(user))
					} else {
						avatar(&picture.name, 48., None)
					}
				},
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
			// egui spaces the home tile 11 px from the list below it, and the list by 12.
			.child(div().w_full().mb(px(-1.)).child(self.rail_tile(
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
							color(if highlight { p.accent_text } else { p.text }),
						))
				},
				cx,
			)))
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
					.left(px(-3.))
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
		let guild = self.guild;
		let rows = self
			.nav
			.iter()
			.enumerate()
			.map(|(index, row)| {
				// A rule closes a shortcut shelf when the regular list follows.
				let rule = row.shelf() && self.nav.get(index + 1).is_some_and(|next| !next.shelf());
				match row {
					NavRow::Heading(heading) => self.heading_row(*heading, rule),
					NavRow::Category { id, name, count } => {
						self.category_row(*id, name, *count, cx).into_any_element()
					}
					NavRow::Channel { id, thread, .. } => self.channel_row(*id, *thread, rule, cx),
				}
			})
			.collect::<Vec<_>>();
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
					.id("sidebar-header")
					.h(px(48.))
					.flex_none()
					.px_4()
					.border_b_1()
					.border_color(color(p.border))
					.flex()
					.items_center()
					.gap(px(5.))
					.when_some(guild, |d, id| {
						d.cursor_pointer().on_click(cx.listener(
							move |this, event: &ClickEvent, window, cx| {
								this.open_nav_menu(Target::Guild(id), event.position(), window, cx);
							},
						))
					})
					.child(
						div()
							.min_w_0()
							.overflow_hidden()
							.text_ellipsis()
							.whitespace_nowrap()
							.text_size(px(15.))
							.font_weight(FontWeight::SEMIBOLD)
							// egui's server menu button keeps the body colour; home is strong.
							.text_color(color(if guild.is_some() {
								p.text
							} else {
								p.text_strong
							}))
							.child(title),
					)
					.when(guild.is_some(), |d| {
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
					.when(guild.is_none(), |d| d.child(self.home_tools(cx)))
					.when(self.nav.is_empty(), |d| {
						d.child(
							div()
								.p_2()
								.text_size(px(15.))
								.text_color(color(p.muted))
								.child("No conversations available here."),
						)
					})
					.children(rows),
			)
	}

	/// Eyebrow over a shortcut shelf or the home remainder, row-high like the list.
	fn heading_row(&self, heading: Heading, rule: bool) -> AnyElement {
		let p = palette();
		let height = if self.guild.is_some() { 34. } else { 44. };
		div()
			.relative()
			.flex_none()
			.h(px(height))
			.pl(px(8.))
			.flex()
			.items_center()
			.text_size(px(12.))
			.font_weight(FontWeight::SEMIBOLD)
			.text_color(color(p.muted))
			.child(heading.label())
			.when(rule, |d| d.child(shelf_rule(height + 7.)))
			.into_any_element()
	}

	/// Collapsible category chrome, as egui's `category_header`: the chevron and label sit
	/// low in the row so categories read as section breaks.
	fn category_row(
		&self,
		id: Id,
		name: &str,
		count: usize,
		cx: &mut Context<Self>,
	) -> Stateful<Div> {
		let p = palette();
		let collapsed = self.collapsed.contains(&id);
		let group: SharedString = format!("category-{id}").into();
		div()
			.id(("category", id.0))
			.group(group.clone())
			.relative()
			.flex_none()
			.h(px(34.))
			.cursor_pointer()
			.tooltip(tooltip(format!(
				"{name} category · {count} channels · {}",
				if collapsed { "Expand" } else { "Collapse" }
			)))
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
				.absolute()
				.left(px(1.))
				.top(px(15.))
				.group_hover(group.clone(), |s| s.text_color(color(p.text_strong))),
			)
			.child(
				div()
					.absolute()
					.left(px(16.))
					.right(px(8.))
					.bottom(px(6.))
					.line_height(px(15.))
					.text_size(px(12.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.muted))
					.group_hover(group, |s| s.text_color(color(p.text_strong)))
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.child(name.to_uppercase()),
			)
	}

	/// One channel or conversation row, painted like egui's `channel_list` rows and
	/// `voice_channel_button` with the `channel_marks` lock badge and hidden eye.
	fn channel_row(&self, id: Id, thread: bool, rule: bool, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let Some(channel) = self.state.channel(id) else {
			return div().into_any_element();
		};
		let selected = self.state.selected == Some(id);
		let direct = channel.guild.is_none();
		let access = self.state.channel_access(id);
		let visible = !access.hidden();
		let forum = !direct && matches!(channel.kind, 15 | 16);
		let voice = channel.kind == 2;
		// A forum carries no messages of its own: its posts hold the activity.
		let unread = visible
			&& (self.state.channel_unread(channel) == Some(true)
				|| self.state.unread_count(id) > 0
				|| (forum && self.state.forum_unread(id)));
		let new_posts = if visible && forum {
			self.state.forum_new_count(id)
		} else {
			0
		};
		let count = if !visible || forum {
			0
		} else if direct {
			self.state.unread_count(id)
		} else {
			self.state.mention_count(id)
		};
		let openable = visible && (text_channel(channel) || forum);
		// Voice rows highlight like the main app's; calls themselves stay there.
		let hoverable = openable || (visible && voice);
		let unavailable = visible && !openable && !voice;
		// Muted, hidden and unusable rows are dimmed; focus never brightens them.
		let dim = access.dim() || !visible || unavailable;
		let idle = if dim {
			crate::theme::tint(p.muted, 0.6)
		} else {
			color(p.muted)
		};
		let focused = if dim { idle } else { color(p.text_strong) };
		let text = if selected || (unread && !voice) {
			focused
		} else {
			idle
		};
		let group: SharedString = format!("channel-row-{id}").into();
		let mut label = self.state.conversation_name(channel).to_owned();
		if unavailable {
			label.push_str(" · unavailable");
		}
		let height = if direct { 42. } else { 32. };
		let row = div()
			.id(("channel", id.0))
			.group(group.clone())
			.relative()
			.flex_none()
			.h(px(height))
			.my(px(1.))
			.pl(px(if thread { 22. } else { 8. }))
			.pr_2()
			.rounded(px(8.))
			.flex()
			.items_center()
			.gap(px(if direct { 12. } else { 6. }))
			.text_color(text)
			.when(selected, |d| d.bg(color(p.selected)))
			.when(!selected && hoverable, |d| {
				d.hover(|d| d.bg(color(p.hover)).text_color(focused))
			})
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
				d.tooltip(tooltip(if !visible {
					"Hidden · you cannot view this channel"
				} else if voice {
					"Voice is available in the main Serein app"
				} else {
					"This channel type is available in the main Serein app"
				}))
			})
			.when(unread && !selected && !access.muted(), |d| {
				d.child(
					div()
						.absolute()
						.left(px(-8.))
						.top(px(height / 2. - 4.))
						.w(px(4.))
						.h(px(8.))
						.rounded(px(2.))
						.bg(color(p.text_strong)),
				)
			});
		let row = if direct {
			let user = channel.recipients.first();
			let presence = user
				.filter(|_| channel.kind == 1)
				.and_then(|user| self.state.presence_for(user.id));
			let picture = if channel.kind == 3 {
				group_avatar(channel, 32., color(p.sidebar))
			} else if let Some(user) = user {
				avatar_with_presence(
					&user.name,
					32.,
					Some(user),
					presence.and_then(|presence| presence.status.as_deref()),
					color(p.sidebar),
				)
			} else {
				avatar(&channel.name, 32., None)
			};
			let subtitle = if channel.kind == 3 {
				Some(format!("{} Members", channel.recipients.len().max(1)))
			} else {
				presence.and_then(presence_subtitle)
			};
			let tag = user
				.filter(|_| channel.kind == 1)
				.and_then(|user| user.primary_guild.as_deref());
			row.child(picture).child(
				div()
					.flex_1()
					.min_w_0()
					.flex()
					.flex_col()
					.child(
						div()
							.h(px(18.))
							.min_w_0()
							.flex()
							.items_center()
							.gap(px(5.))
							.child(
								div()
									.min_w_0()
									.overflow_hidden()
									.whitespace_nowrap()
									.text_ellipsis()
									.text_size(px(15.))
									.font_weight(FontWeight::MEDIUM)
									.child(label),
							)
							.children(tag.map(server_tag)),
					)
					.children(subtitle.map(|subtitle| {
						div()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(12.))
							.text_color(color(p.muted))
							.child(subtitle)
					})),
			)
		} else {
			// The glyph is a touch quieter than the name until the row is focused.
			let quiet = !voice && (!selected || dim);
			let halo = |fill: egui::Color32| color(p.sidebar.blend(fill));
			let glyph = div()
				.relative()
				.size(px(20.))
				.flex_none()
				.child(
					icon(channel_icon(channel), px(20.), text)
						.when(quiet, |d| d.opacity(0.85))
						.when(hoverable && !selected, |d| {
							d.group_hover(group.clone(), |d| d.text_color(focused).opacity(1.))
						}),
				)
				.when(access.limited(), |d| {
					d.child(
						div()
							.absolute()
							.left(px(20. * 19.5 / 24. - 7.5))
							.top(px(20. * 6.25 / 24. - 7.5))
							.size(px(15.))
							.rounded_full()
							.bg(if selected {
								halo(p.selected)
							} else {
								color(p.sidebar)
							})
							.when(hoverable && !selected, |d| {
								d.group_hover(group.clone(), |d| d.bg(halo(p.hover)))
							})
							.flex()
							.items_center()
							.justify_center()
							.child(
								icon(Icon::Lock, px(8.), text).when(hoverable && !selected, |d| {
									d.group_hover(group.clone(), |d| d.text_color(focused))
								}),
							),
					)
				});
			row.child(glyph).child(
				div()
					.flex_1()
					.min_w_0()
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.text_size(px(15.))
					.font_weight(FontWeight::MEDIUM)
					.child(label),
			)
		};
		row.when(new_posts > 0, |d| {
			d.child(
				div()
					.flex_none()
					.text_size(px(12.))
					.text_color(color(p.muted))
					.child(if new_posts > 99 {
						"99+ New".to_owned()
					} else {
						format!("{new_posts} New")
					}),
			)
		})
		.when(count > 0, |d| d.child(count_pill(count).mr(px(2.5))))
		.when(!visible, |d| {
			d.child(icon(Icon::EyeSlash, px(22.), text).ml(px(2.)).mr(px(8.)))
		})
		.when(rule, |d| d.child(shelf_rule(height + 7.)))
		.into_any_element()
	}

	/// "Find conversation" and the Friends glyph atop the home list, as egui's sidebar.
	fn home_tools(&self, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let friends = self.friends_visible();
		div()
			.flex_none()
			// egui's item spacing around the row, plus its 8 px gap below.
			.mt(px(1.5))
			.mb(px(17.))
			.flex()
			.gap(px(6.))
			.child(
				div()
					.id("find-conversation")
					.flex_1()
					.min_w(px(60.))
					.h(px(30.))
					.rounded(px(8.))
					.bg(color(p.raised))
					.flex()
					.items_center()
					.justify_center()
					.overflow_hidden()
					.whitespace_nowrap()
					.text_size(px(14.))
					.font_weight(FontWeight::MEDIUM)
					.text_color(color(p.text))
					.cursor_pointer()
					.focusable()
					.tab_stop(true)
					.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
					.focus(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
					.tooltip(tooltip("Search loaded conversations (Ctrl/Cmd+K)"))
					.on_click(cx.listener(|this, _, window, cx| this.toggle_switcher(window, cx)))
					.child("Find conversation"),
			)
			.child(
				div()
					.id("friends-glyph")
					.size(px(30.))
					.flex_none()
					.rounded(px(6.))
					.flex()
					.items_center()
					.justify_center()
					.cursor_pointer()
					.focusable()
					.tab_stop(true)
					.group("friends-glyph")
					.when(friends, |d| d.bg(color(p.selected)))
					.when(!friends, |d| d.hover(|d| d.bg(color(p.hover))))
					.focus(|d| d.bg(color(p.hover)))
					.tooltip(tooltip("Friends"))
					.on_click(cx.listener(|this, _, _, cx| this.open_friends(cx)))
					.child(
						icon(
							Icon::Users,
							px(16.),
							color(if friends { p.text_strong } else { p.muted }),
						)
						.group_hover("friends-glyph", |d| d.text_color(color(p.text_strong))),
					),
			)
			.into_any_element()
	}

	fn user_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		// Second line as the main app's account card: a shared game first, then connection state.
		let (name, status) = match &self.state.user {
			Some(user) => (
				self.state.user_display_name(user).to_owned(),
				if let Some(game) = self.state.local_game_activity() {
					game.name.clone()
				} else if self.state.demo {
					"Offline preview".to_owned()
				} else if self.state.gateway_connected {
					"Online".to_owned()
				} else {
					"Reconnecting…".to_owned()
				},
			),
			None => (String::new(), String::new()),
		};
		let presence = if self.state.demo || self.state.gateway_connected {
			Some("online")
		} else {
			Some("offline")
		};
		// Mic and headphones with their device chevrons sit where the main app has them, but
		// calls stay in the main app: shown disabled with the reason instead of doing nothing.
		let voice_off = crate::theme::tint(p.muted, 0.5);
		let voice = |id: &'static str, glyph: Icon, width: f32, size: f32| {
			div()
				.id(id)
				.w(px(width))
				.h(px(32.))
				.flex_none()
				.flex()
				.items_center()
				.justify_center()
				.tooltip(crate::tooltip(
					"Voice and calls are only in the main Serein app",
				))
				.child(icon(glyph, px(size), voice_off))
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
						.overflow_hidden()
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
						.flex_none()
						.flex()
						.items_center()
						.gap(px(2.))
						.child(voice("voice-mic", Icon::Microphone, 32., 20.))
						.child(voice("voice-mic-menu", Icon::CaretDown, 20., 12.))
						.child(div().w(px(4.)))
						.child(voice("voice-output", Icon::Headphones, 32., 20.))
						.child(voice("voice-output-menu", Icon::CaretDown, 20., 12.))
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
								.on_click(cx.listener(|this, _, window, cx| {
									this.open_settings(None, window, cx)
								}))
								.child(icon(Icon::Gear, px(20.), color(p.muted))),
						),
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

/// Channel-list rows for `guild` (direct messages for `None`), as egui's `categories::rows`:
/// the shortcut shelf first (Favorites in a server, Pinned at home), then uncategorised
/// channels and categories. Threads are listed only once opened (`last_viewed_threads`), at
/// most four under their parent. Channels the user cannot view are listed only with
/// `show_hidden`, like "Show hidden channels"; collapsed categories hide all their rows.
fn nav_rows(
	state: &client_core::State,
	guild: Option<Id>,
	collapsed: &BTreeSet<Id>,
	show_hidden: bool,
	shortcuts: &NavState,
) -> Vec<NavRow> {
	let thread = |kind: u8| matches!(kind, 10..=12);
	let admits = |c: &model::Channel| {
		c.guild == guild
			&& c.kind != 4
			&& (guild.is_some() || !state.spam_direct(c.id))
			&& (show_hidden || state.can_view(c.id))
	};
	let (shelf_ids, heading) = match guild {
		Some(_) => (&shortcuts.favorites, Heading::Favorites),
		None => (&shortcuts.pinned, Heading::Pinned),
	};
	let mut lifted = BTreeSet::new();
	let shelf = shelf_ids
		.iter()
		.filter_map(|id| state.channel(*id))
		.filter(|c| admits(c) && lifted.insert(c.id))
		.collect::<Vec<_>>();
	let mut categories = state
		.channels
		.iter()
		.filter(|c| guild.is_some() && c.guild == guild && c.kind == 4)
		.collect::<Vec<_>>();
	categories.sort_by_key(|c| (c.position, c.id));
	let category_ids = categories.iter().map(|c| c.id).collect::<BTreeSet<_>>();
	let parents = state
		.channels
		.iter()
		.filter(|c| guild.is_some() && c.guild == guild && matches!(c.kind, 0 | 5 | 15 | 16))
		.map(|c| (c.id, c))
		.collect::<BTreeMap<_, _>>();
	let mut groups = BTreeMap::<Option<Id>, Vec<&model::Channel>>::new();
	let mut threads = BTreeMap::<Id, Vec<&model::Channel>>::new();
	for channel in state
		.channels
		.iter()
		.filter(|c| admits(c) && !lifted.contains(&c.id))
	{
		if thread(channel.kind) {
			if !state.last_viewed_threads.contains(&channel.id) {
				continue;
			}
			if let Some(parent) = channel.parent_id.and_then(|id| parents.get(&id))
				&& parent.parent_id != Some(channel.id)
			{
				threads.entry(parent.id).or_default().push(channel);
				continue;
			}
		}
		let parent = channel
			.parent_id
			.filter(|id| !thread(channel.kind) && category_ids.contains(id));
		groups.entry(parent).or_default().push(channel);
	}
	for group in groups.values_mut() {
		if guild.is_some() {
			group.sort_by_key(|c| (matches!(c.kind, 2 | 13), c.position, c.id));
		} else {
			// Most recent conversation first, like the main app's DM list.
			group.sort_by_key(|c| std::cmp::Reverse((state.channel_activity(c), c.id)));
		}
	}
	for group in threads.values_mut() {
		group.sort_by_key(|c| state.last_viewed_threads.iter().position(|id| *id == c.id));
		group.truncate(THREADS_PER_CHANNEL);
	}
	let append = |rows: &mut Vec<NavRow>, channel: &model::Channel, shelf: bool| {
		rows.push(NavRow::Channel {
			id: channel.id,
			thread: false,
			shelf,
		});
		rows.extend(
			threads
				.get(&channel.id)
				.into_iter()
				.flatten()
				.map(|c| NavRow::Channel {
					id: c.id,
					thread: true,
					shelf,
				}),
		);
	};
	let mut rows = Vec::new();
	if !shelf.is_empty() {
		rows.push(NavRow::Heading(heading));
		for channel in shelf {
			append(&mut rows, channel, true);
		}
	}
	let mut tree = Vec::new();
	for channel in groups.remove(&None).unwrap_or_default() {
		append(&mut tree, channel, false);
	}
	for category in categories {
		let children = groups.remove(&Some(category.id)).unwrap_or_default();
		if children.is_empty() && !show_hidden {
			continue;
		}
		let count = children
			.iter()
			.map(|c| 1 + threads.get(&c.id).map_or(0, Vec::len))
			.sum();
		tree.push(NavRow::Category {
			id: category.id,
			name: category.name.clone(),
			count,
		});
		if !collapsed.contains(&category.id) {
			for channel in children {
				append(&mut tree, channel, false);
			}
		}
	}
	if guild.is_none() && !tree.is_empty() {
		rows.push(NavRow::Heading(Heading::DirectMessages));
	}
	rows.extend(tree);
	if guild.is_some_and(|guild| state.hides_muted_channels(guild) == Some(true)) {
		retain_unmuted(&mut rows, state);
	}
	rows
}

#[cfg(test)]
mod tests {
	use super::{Heading, NavRow, NavState, badge_label, home_label, nav_rows, retain_unmuted};
	use model::Id;
	use std::collections::BTreeSet;

	fn ids(rows: &[NavRow]) -> Vec<u64> {
		rows.iter()
			.filter_map(|row| match row {
				NavRow::Category { id, .. } | NavRow::Channel { id, .. } => Some(id.0),
				NavRow::Heading(_) => None,
			})
			.collect()
	}

	#[test]
	fn hidden_channels_are_listed_only_when_opted_in() {
		let mut state = test_support::chat_demo_state();
		test_support::seed_access_marks(&mut state);
		// #secret (62) denies @everyone; #staff-notes (61) is allowed through a role.
		assert!(!state.can_view(Id(62)) && state.can_view(Id(61)));
		let listed = |show_hidden| {
			ids(&nav_rows(
				&state,
				Some(Id(10)),
				&BTreeSet::new(),
				show_hidden,
				&NavState::default(),
			))
		};
		let visible = listed(false);
		assert!(visible.contains(&60) && visible.contains(&61));
		assert!(!visible.contains(&62));
		let all = listed(true);
		assert!(all.contains(&62));
		assert!(all.len() > visible.len());
	}

	#[test]
	fn only_opened_threads_are_listed_under_their_parent() {
		let mut state = test_support::chat_demo_state();
		let rows = |state: &client_core::State| {
			nav_rows(
				state,
				Some(Id(10)),
				&BTreeSet::new(),
				false,
				&NavState::default(),
			)
		};
		let threads = |rows: &[NavRow]| {
			rows.iter()
				.filter_map(|row| match row {
					NavRow::Channel {
						id, thread: true, ..
					} => Some(id.0),
					_ => None,
				})
				.collect::<Vec<_>>()
		};
		state.last_viewed_threads.clear();
		assert!(threads(&rows(&state)).is_empty());
		// Introductions thread (28) under #getting-started (20); a forum post (27) under #ideas.
		state.last_viewed_threads = vec![Id(28), Id(27)];
		let listed = rows(&state);
		assert_eq!(threads(&listed), [28, 27]);
		let parent = listed
			.iter()
			.position(|row| matches!(row, NavRow::Channel { id: Id(20), .. }))
			.unwrap();
		assert!(matches!(
			listed[parent + 1],
			NavRow::Channel {
				id: Id(28),
				thread: true,
				..
			}
		));
	}

	#[test]
	fn shortcut_shelves_lift_channels_above_the_list() {
		let state = test_support::chat_demo_state();
		let shortcuts = NavState {
			favorites: vec![Id(20)],
			pinned: vec![Id(22)],
			..Default::default()
		};
		let guild = nav_rows(&state, Some(Id(10)), &BTreeSet::new(), false, &shortcuts);
		assert_eq!(guild[0], NavRow::Heading(Heading::Favorites));
		assert!(matches!(
			guild[1],
			NavRow::Channel {
				id: Id(20),
				shelf: true,
				..
			}
		));
		assert_eq!(ids(&guild).iter().filter(|id| **id == 20).count(), 1);
		let home = nav_rows(&state, None, &BTreeSet::new(), false, &shortcuts);
		assert_eq!(home[0], NavRow::Heading(Heading::Pinned));
		assert!(matches!(home[1], NavRow::Channel { id: Id(22), .. }));
		assert_eq!(home[2], NavRow::Heading(Heading::DirectMessages));
		assert!(!ids(&home[3..]).contains(&22));
	}

	#[test]
	fn hiding_muted_channels_drops_them_and_their_empty_heading() {
		// The fixture mutes #long-form (21) and leaves #getting-started (20) unmuted.
		let mut state = test_support::chat_demo_state();
		state.selected = None;
		assert_eq!(state.guild_channel_muted(Id(21)), Some(true));
		assert_eq!(state.guild_channel_muted(Id(20)), Some(false));
		let channel = |id, shelf| NavRow::Channel {
			id: Id(id),
			thread: false,
			shelf,
		};
		let rows = || {
			vec![
				NavRow::Heading(Heading::Favorites),
				channel(21, true),
				NavRow::Category {
					id: Id(23),
					name: "Welcome".into(),
					count: 1,
				},
				channel(20, false),
				NavRow::Category {
					id: Id(24),
					name: "Conversations".into(),
					count: 1,
				},
				channel(21, false),
			]
		};
		let mut hidden = rows();
		retain_unmuted(&mut hidden, &state);
		assert_eq!(ids(&hidden), [23, 20, 24]);
		assert!(!hidden.contains(&NavRow::Heading(Heading::Favorites)));
		// The open channel stays listed even while muted.
		state.selected = Some(Id(21));
		let mut open = rows();
		retain_unmuted(&mut open, &state);
		assert_eq!(ids(&open), [21, 23, 20, 24, 21]);
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
