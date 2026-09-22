//! Server rail, channel list with categories and threads, and the account panel.
use crate::theme::{Icon, color, icon, palette, tint};
use crate::{Serein, channel_label, text_channel, tooltip};
use gpui::{prelude::*, *};
use model::Id;

pub const RAIL_WIDTH: f32 = 68.;
pub const LIST_WIDTH: f32 = 240.;
/// Visible threads per parent channel, as in the main app.
const THREADS_PER_CHANNEL: usize = 3;

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

/// Initials on the hashed fallback colour; the main app's circle avatar without an image.
pub fn avatar(name: &str, size: f32) -> Div {
	div()
		.size(px(size))
		.flex_none()
		.rounded_full()
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
pub fn avatar_with_presence(name: &str, size: f32, status: Option<&str>, ring: Rgba) -> Div {
	let dot = (size * 0.16).clamp(4., 8.);
	div()
		.relative()
		.flex_none()
		.child(avatar(name, size))
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
		let state = &self.state;
		let visible = |c: &&model::Channel| {
			c.guild == self.guild
				&& (c.supports_text() || matches!(c.kind, 2 | 13 | 15 | 16))
				&& !matches!(c.kind, 10..=12)
				&& state.can_view(c.id)
		};
		let mut rows = Vec::new();
		if self.guild.is_none() {
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
			self.nav = rows;
			return;
		}
		let mut channels = state.channels.iter().filter(visible).collect::<Vec<_>>();
		channels.sort_by_key(|c| (matches!(c.kind, 2 | 13), c.position, c.id));
		let mut categories = state
			.channels
			.iter()
			.filter(|c| c.guild == self.guild && c.kind == 4)
			.collect::<Vec<_>>();
		categories.sort_by_key(|c| (c.position, c.id));
		let threads = |parent: Id| {
			let mut threads = state
				.channels
				.iter()
				.filter(|c| {
					c.parent_id == Some(parent) && matches!(c.kind, 10..=12) && state.can_view(c.id)
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
			if children.is_empty() {
				continue;
			}
			rows.push(NavRow::Category {
				id: category.id,
				name: category.name.to_uppercase(),
			});
			let collapsed = self.collapsed.contains(&category.id);
			for channel in children {
				push(&mut rows, channel, collapsed);
			}
		}
		self.nav = rows;
	}

	fn rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let home = self.guild.is_none();
		let unread_home = self
			.state
			.channels
			.iter()
			.any(|c| c.guild.is_none() && self.state.unread_count(c.id) > 0);
		let mut guilds = Vec::with_capacity(self.state.guilds.len());
		for guild in &self.state.guilds {
			let id = guild.id;
			let selected = self.guild == Some(id);
			let unread = self
				.state
				.channels
				.iter()
				.any(|c| c.guild == Some(id) && self.state.unread_count(c.id) > 0);
			let label = initials(&guild.name);
			let size = if label.chars().count() > 1 { 16. } else { 18. };
			guilds.push(self.rail_tile(
				("guild", id.0),
				guild.name.clone(),
				selected,
				unread,
				move |this, cx| this.select_section(Some(id), cx),
				move |highlight| {
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
			));
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
				"home",
				"Direct Messages",
				home,
				unread_home,
				|this, cx| this.select_section(None, cx),
				|highlight| {
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
			.child(
				div()
					.w(px(32.))
					.h(px(2.))
					.flex_none()
					.rounded(px(1.))
					.bg(color(p.raised)),
			)
			.children(guilds)
	}

	#[allow(clippy::too_many_arguments)]
	fn rail_tile(
		&self,
		id: impl Into<ElementId>,
		name: impl Into<SharedString>,
		selected: bool,
		unread: bool,
		on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
		tile: impl Fn(bool) -> Div,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let id = id.into();
		let name: SharedString = name.into();
		let group: SharedString = format!("rail-{id}").into();
		div()
			.relative()
			.w_full()
			.flex()
			.justify_center()
			.group(group.clone())
			// White pill left of the tile: tall when selected, medium on hover, a dot for unread.
			.child(
				div()
					.absolute()
					.left(px(-4.))
					.top(px(23.
						- if selected {
							20.
						} else if unread {
							4.
						} else {
							0.
						}))
					.w(px(8.))
					.h(px(if selected {
						40.
					} else if unread {
						8.
					} else {
						0.
					}))
					.rounded(px(4.))
					.bg(color(p.text_strong))
					.when(!selected, |d| {
						d.group_hover(group.clone(), |d| d.h(px(20.)).top(px(13.)))
					}),
			)
			.child(
				div()
					.id(id)
					.focusable()
					.tab_stop(true)
					.size(px(46.))
					.flex_none()
					.cursor_pointer()
					.rounded(px(13.))
					.focus(|d| d.border_2().border_color(color(p.text_strong)))
					.tooltip(tooltip(name))
					.on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
					.child(
						div()
							.size_full()
							.when(!selected, |d| {
								d.child(
									tile(false)
										.group_hover(group.clone(), |d| d.bg(color(p.accent))),
								)
							})
							.when(selected, |d| d.child(tile(true))),
					),
			)
			.into_any_element()
	}

	fn channel_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let title = match self.guild {
			Some(id) => self
				.state
				.guild(id)
				.map_or_else(String::new, |g| g.name.clone()),
			None => "Direct Messages".into(),
		};
		div()
			.w(px(LIST_WIDTH))
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
		let unread = self.state.channel_unread(channel) == Some(true);
		let mentions = self.state.mention_count(id);
		let openable = text_channel(channel) && !matches!(channel.kind, 15 | 16);
		let strong = selected || unread;
		let name = channel_label(channel);
		let direct = channel.guild.is_none();
		let text = color(if strong { p.text_strong } else { p.muted });
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
			.when(!selected, |d| {
				d.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
			})
			.when(openable, |d| {
				d.cursor_pointer()
					.focusable()
					.tab_stop(true)
					.focus(|d| d.bg(color(p.hover)))
					.on_click(cx.listener(move |this, _, _, cx| this.select(id, cx)))
			})
			.when(!openable, |d| {
				d.tooltip(tooltip(if matches!(channel.kind, 15 | 16) {
					"Forum posts open in the main Serein app"
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
		row.when(mentions > 0, |d| {
			let label = if mentions > 99 {
				"99+".to_owned()
			} else {
				mentions.to_string()
			};
			d.child(
				div()
					.h(px(19.))
					.min_w(px(19.))
					.px(px(5.))
					.rounded(px(10.))
					.bg(color(p.danger))
					.flex()
					.items_center()
					.justify_center()
					.text_size(px(12.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(white())
					.child(label),
			)
		})
		.into_any_element()
	}

	fn user_panel(&self) -> impl IntoElement {
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
				.child(avatar_with_presence(&name, 32., presence, color(p.raised)))
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
						.px_2()
						.h(px(22.))
						.flex()
						.items_center()
						.rounded(px(6.))
						.bg(tint(p.accent, 0.16))
						.text_size(px(11.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(color(p.mention_text))
						.child("GPUI"),
				),
		)
	}

	pub(crate) fn render_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
		div()
			.w(px(RAIL_WIDTH + LIST_WIDTH))
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
					.child(self.channel_list(cx)),
			)
			.child(self.user_panel())
	}
}
