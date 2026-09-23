//! Friends page on the home view, laid out like egui's `friends.rs`: Online / All / Pending /
//! Blocked & Ignored from the relationships already in `client_core::State`, a search field,
//! and Add Friend. Accept, decline and cancel go through `resolve_friend_request`, requests
//! through `add_friend`; the More button and right-click open the shared navigation menu
//! (Message, Remove Friend, Block, Unblock).
use crate::sidebar::{avatar, avatar_with_presence};
use crate::theme::{Icon, color, icon, palette};
use crate::{Serein, tooltip};
use client_core::State;
use gpui::{prelude::*, *};
use model::Id;

const ROW_HEIGHT: f32 = 64.;
const REQUEST_HEIGHT: f32 = 72.;
/// Longest search query kept, as egui's `char_limit(128)`.
const QUERY_CHARS: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
	#[default]
	Online,
	All,
	Pending,
	Restricted,
	AddFriend,
}

impl Tab {
	const ALL: [Self; 5] = [
		Self::Online,
		Self::All,
		Self::Pending,
		Self::Restricted,
		Self::AddFriend,
	];
	fn label(self) -> &'static str {
		match self {
			Self::Online => "Online",
			Self::All => "All",
			Self::Pending => "Pending",
			Self::Restricted => "Blocked & Ignored",
			Self::AddFriend => "Add Friend",
		}
	}
}

#[derive(Default)]
pub struct Page {
	/// Chosen from the Friends glyph; any channel selection hides the page again.
	pub open: bool,
	pub tab: Tab,
	/// Pending shows outgoing rather than incoming requests.
	pub outgoing: bool,
	/// The Add Friend username field, created the first time that tab opens.
	pub username: Option<Entity<crate::input::Input>>,
	/// The list search field, created when the page first opens; cleared on tab changes.
	pub search: Option<Entity<crate::input::Input>>,
}

fn online(status: Option<&str>) -> bool {
	matches!(status, Some("online" | "idle" | "dnd"))
}

fn presence_label(status: Option<&str>) -> &'static str {
	match status {
		Some("online") => "Online",
		Some("idle") => "Idle",
		Some("dnd") => "Do Not Disturb",
		Some(_) => "Offline",
		None => "Presence unavailable",
	}
}

/// Rows for a tab as `(user, incoming)`, unfiltered; see [`list`].
pub fn rows(state: &State, tab: Tab) -> Vec<(Id, bool)> {
	list(state, tab, "", false)
}

/// Rows for a tab as `(user, incoming)`, sorted by name then ID like the egui list and kept
/// when a name, display name or username contains `query` (case-insensitive). Pending lists
/// incoming requests, or outgoing ones with `outgoing`.
fn list(state: &State, tab: Tab, query: &str, outgoing: bool) -> Vec<(Id, bool)> {
	let query = query.trim().to_lowercase();
	let matches = |user: &model::User, username: Option<&str>| {
		query.is_empty()
			|| user.name.to_lowercase().contains(&query)
			|| state
				.user_display_name(user)
				.to_lowercase()
				.contains(&query)
			|| username.is_some_and(|name| name.to_lowercase().contains(&query))
	};
	let mut rows = match tab {
		Tab::AddFriend => Vec::new(),
		Tab::Pending => state
			.pending_friends()
			.filter(|(user, name, incoming)| *incoming != outgoing && matches(user, Some(name)))
			.map(|(user, _, incoming)| (user, *incoming))
			.collect::<Vec<_>>(),
		Tab::Restricted => state
			.restricted_users()
			.filter(|(user, name, _)| matches(user, Some(name)))
			.map(|(user, _, _)| (user, false))
			.collect(),
		Tab::All | Tab::Online => state
			.friends()
			.filter(|user| {
				(tab == Tab::All
					|| online(
						state
							.presence_for(user.id)
							.and_then(|p| p.status.as_deref()),
					)) && matches(user, state.friend_username(user.id))
			})
			.map(|user| (user, false))
			.collect(),
	};
	rows.sort_unstable_by(|(a, _), (b, _)| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
	rows.into_iter()
		.map(|(user, incoming)| (user.id, incoming))
		.collect()
}

/// A square glyph button as egui's `icons::button`: no plate until hovered.
fn icon_button(
	id: impl Into<ElementId>,
	group: SharedString,
	glyph: Icon,
	size: f32,
	label: &'static str,
	enabled: bool,
) -> Stateful<Div> {
	let p = palette();
	div()
		.id(id)
		.group(group.clone())
		.size(px(size))
		.flex_none()
		.rounded(px(6.))
		.flex()
		.items_center()
		.justify_center()
		.tooltip(tooltip(label))
		.when(enabled, |d| {
			d.cursor_pointer().hover(|d| d.bg(color(p.hover)))
		})
		.child(
			icon(
				glyph,
				px(size * 0.6),
				if enabled {
					color(p.muted)
				} else {
					crate::theme::tint(p.muted, 0.5)
				},
			)
			.when(enabled, |d| {
				d.group_hover(group, |d| d.text_color(color(p.text_strong)))
			}),
		)
}

impl Serein {
	pub(crate) fn friends_visible(&self) -> bool {
		self.friends.open && self.guild.is_none() && self.state.selected.is_none()
	}

	/// Leave the conversation for the Friends page, keeping its draft.
	pub(crate) fn open_friends(&mut self, cx: &mut Context<Self>) {
		if !self.save_draft(cx) {
			return;
		}
		self.friends.open = true;
		if self.friends.search.is_none() {
			let input = cx.new(crate::input::Input::new);
			input.update(cx, |input, cx| input.set_placeholder("Search".into(), cx));
			cx.subscribe(&input, |_, _, event: &crate::input::Event, cx| {
				if matches!(event, crate::input::Event::Changed) {
					cx.notify();
				}
			})
			.detach();
			self.friends.search = Some(input);
		}
		self.state.open_home();
		self.guild = None;
		self.state.timeline.clear();
		self.state.history_pending = false;
		self.editing = None;
		self.profile = None;
		self.picker = None;
		self.composer
			.update(cx, |input, cx| input.set_value(String::new(), cx));
		self.sync_channels();
		self.sync_rows();
		self.update_placeholder(cx);
		cx.notify();
	}

	fn friend_query(&self, cx: &App) -> String {
		self.friends
			.search
			.as_ref()
			.map_or_else(String::new, |input| {
				input.read(cx).value().chars().take(QUERY_CHARS).collect()
			})
	}

	/// Switches tabs, clearing the search like egui.
	fn set_friends_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
		self.friends.tab = tab;
		if let Some(input) = &self.friends.search {
			input.update(cx, |input, cx| input.set_value(String::new(), cx));
		}
		if tab == Tab::AddFriend {
			self.focus_friend_username(window, cx);
		}
		cx.notify();
	}

	/// One friend or blocked/ignored user: a 64 px row with a top rule, as egui's list.
	fn friend_row(&self, user: Id, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let restricted = (self.friends.tab == Tab::Restricted)
			.then(|| self.state.restricted_user(user))
			.flatten();
		let Some(record) = restricted
			.map(|(user, _, _)| user)
			.or_else(|| self.state.friend(user))
		else {
			return div().h(px(ROW_HEIGHT)).into_any_element();
		};
		let presence = restricted
			.is_none()
			.then(|| self.state.presence_for(user))
			.flatten();
		let status = presence.and_then(|p| p.status.as_deref());
		let activity = presence.and_then(|p| p.activities.first());
		let name = self.state.user_display_name(record).to_owned();
		let subtitle = match restricted {
			Some((_, _, ignored)) => if *ignored { "Ignored" } else { "Blocked" }.to_owned(),
			None => activity
				.map(|activity| {
					if activity.kind == 2 && activity.name.eq_ignore_ascii_case("Spotify") {
						activity.state.clone().unwrap_or_else(|| activity.summary())
					} else {
						activity.summary()
					}
				})
				.or_else(|| presence.and_then(|p| p.custom_status.clone()))
				.unwrap_or_else(|| presence_label(status).to_owned()),
		};
		let available =
			!self.state.user_action_pending() && (self.state.demo || self.state.gateway_connected);
		let open_menu =
			move |this: &mut Self, position, window: &mut Window, cx: &mut Context<Self>| {
				this.open_nav_menu(crate::nav_menu::Target::Friend(user), position, window, cx);
			};
		let dm = restricted.is_none()
			&& self.state.channels.iter().any(|c| {
				c.guild.is_none() && c.kind == 1 && c.recipients.iter().any(|u| u.id == user)
			});
		div()
			.id(("friend", user.0))
			.h(px(ROW_HEIGHT))
			.flex_none()
			.border_t_1()
			.border_color(color(p.border))
			.on_mouse_down(
				MouseButton::Right,
				cx.listener(move |this, event: &MouseDownEvent, window, cx| {
					open_menu(this, event.position, window, cx);
					cx.stop_propagation();
				}),
			)
			.child(
				div().size_full().p(px(1.)).child(
					div()
						.size_full()
						.rounded(px(6.))
						.flex()
						.items_center()
						.hover(|d| d.bg(color(p.hover)))
						.child(match restricted {
							Some(_) => avatar(&name, 40., Some(record)),
							None => avatar_with_presence(
								&name,
								40.,
								Some(record),
								status,
								color(p.chat),
							),
						})
						.child(
							div()
								.ml(px(12.))
								.self_start()
								.pt(px(11.))
								.flex_1()
								.min_w_0()
								.flex()
								.flex_col()
								.gap(px(6.))
								.child(
									div()
										.overflow_hidden()
										.whitespace_nowrap()
										.text_ellipsis()
										.line_height(px(20.))
										.text_size(px(16.))
										.font_weight(FontWeight::SEMIBOLD)
										.text_color(color(p.text))
										.child(name),
								)
								.child(
									div()
										.min_w_0()
										.flex()
										.items_center()
										.gap(px(8.))
										.when(activity.is_some_and(|a| a.kind != 2), |d| {
											d.child(icon(
												Icon::GameController,
												px(14.),
												color(p.positive),
											))
										})
										.child(
											div()
												.min_w_0()
												.overflow_hidden()
												.whitespace_nowrap()
												.text_ellipsis()
												.text_size(px(13.))
												.text_color(color(p.muted))
												.child(subtitle),
										),
								),
						)
						.child(
							div()
								.w(px(88.))
								.flex_none()
								.flex()
								.items_center()
								.gap(px(8.))
								.when(restricted.is_none(), |d| {
									d.child(
										icon_button(
											("friend-message", user.0),
											format!("friend-message-{user}").into(),
											Icon::Threads,
											36.,
											"Message",
											available,
										)
										.when(available, |d| {
											d.on_click(cx.listener(move |this, _, _, cx| {
												this.message_friend(user, cx)
											}))
										})
										.when(!dm, |d| d.opacity(0.85)),
									)
								})
								.child(
									icon_button(
										("friend-more", user.0),
										format!("friend-more-{user}").into(),
										Icon::More,
										36.,
										"More",
										true,
									)
									.on_click(cx.listener(
										move |this, event: &ClickEvent, window, cx| {
											open_menu(this, event.position(), window, cx)
										},
									)),
								),
						),
				),
			)
			.into_any_element()
	}

	fn resolve_request(&mut self, user: Id, accept: bool, cx: &mut Context<Self>) {
		let command = self.state.resolve_friend_request(user, accept);
		self.dispatch(command);
		cx.notify();
	}

	/// One pending request, as egui's `friend_requests_page` rows.
	fn request_row(&self, user: Id, incoming: bool, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let Some((record, username)) = self
			.state
			.pending_friends()
			.find(|(u, _, _)| u.id == user)
			.map(|(u, name, _)| (u, name.clone()))
		else {
			return div().h(px(REQUEST_HEIGHT)).into_any_element();
		};
		let name = self.state.user_display_name(record).to_owned();
		let available =
			!self.state.user_action_pending() && (self.state.demo || self.state.gateway_connected);
		div()
			.id(("friend-request", user.0))
			.h(px(REQUEST_HEIGHT))
			.flex_none()
			.border_t_1()
			.border_color(color(p.border))
			.flex()
			.items_center()
			.gap(px(8.))
			.child(avatar(&name, 40., Some(record)))
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
							.text_size(px(16.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.text))
							.child(name),
					)
					.child(
						div()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(15.))
							.text_color(color(p.muted))
							.child(username),
					),
			)
			.when(incoming, |d| {
				d.child(
					icon_button(
						("friend-accept", user.0),
						format!("friend-accept-{user}").into(),
						Icon::Check,
						32.,
						"Accept request",
						available,
					)
					.when(available, |d| {
						d.on_click(
							cx.listener(move |this, _, _, cx| this.resolve_request(user, true, cx)),
						)
					}),
				)
			})
			.child(
				icon_button(
					("friend-decline", user.0),
					format!("friend-decline-{user}").into(),
					Icon::Close,
					32.,
					if incoming {
						"Decline request"
					} else {
						"Cancel request"
					},
					available,
				)
				.when(available, |d| {
					d.on_click(
						cx.listener(move |this, _, _, cx| this.resolve_request(user, false, cx)),
					)
				}),
			)
			.into_any_element()
	}

	fn friends_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		div()
			.flex_none()
			.px(px(24.))
			// egui's frame margin, then its separator's item spacing and half-gap.
			.pt(px(8.))
			.pb(px(16.))
			.border_b_1()
			.border_color(color(p.border))
			.child(
				div()
					.min_h(px(32.))
					.flex()
					.flex_wrap()
					.items_center()
					.gap(px(16.))
					.child(icon(Icon::Users, px(22.), color(p.muted)))
					.child(
						div()
							.text_size(px(16.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.text))
							.child("Friends"),
					)
					.child(div().w(px(1.)).h(px(24.)).bg(color(p.border)))
					.children(Tab::ALL.map(|tab| {
						let selected = self.friends.tab == tab;
						let add = tab == Tab::AddFriend;
						div()
							.id(tab.label())
							.focusable()
							.tab_stop(true)
							.px(px(12.))
							.py(px(6.))
							.rounded(px(8.))
							.cursor_pointer()
							.text_size(px(14.))
							.font_weight(FontWeight::MEDIUM)
							.text_color(color(if add { p.accent_text } else { p.text }))
							.when(add, |d| d.bg(color(p.accent)))
							.when(selected && !add, |d| d.bg(color(p.raised)))
							.when(!selected && !add, |d| d.hover(|d| d.bg(color(p.hover))))
							.focus(|d| d.border_1().border_color(color(p.accent)))
							.on_click(cx.listener(move |this, _, window, cx| {
								this.set_friends_tab(tab, window, cx)
							}))
							.child(tab.label())
					})),
			)
	}

	/// Creates the Add Friend field on first use and focuses it.
	pub(crate) fn focus_friend_username(&mut self, window: &mut Window, cx: &mut Context<Self>) {
		let input = self.friends.username.get_or_insert_with(|| {
			let input = cx.new(crate::input::Input::new);
			input.update(cx, |input, cx| {
				input.set_placeholder(
					"You can add friends with their Discord username.".into(),
					cx,
				)
			});
			cx.subscribe(&input, |this, _, _: &crate::input::Submit, cx| {
				this.send_friend_request(cx)
			})
			.detach();
			cx.subscribe(&input, |_, _, event: &crate::input::Event, cx| {
				if matches!(event, crate::input::Event::Changed) {
					cx.notify();
				}
			})
			.detach();
			input
		});
		let focus = input.read(cx).focus_handle(cx);
		window.focus(&focus, cx);
	}

	fn friend_username(&self, cx: &App) -> String {
		self.friends
			.username
			.as_ref()
			.map_or_else(String::new, |input| {
				input.read(cx).value().trim().to_owned()
			})
	}

	fn can_send_friend_request(&self, cx: &App) -> bool {
		(self.state.demo || self.state.gateway_connected)
			&& !self.state.user_action_pending()
			&& !self.friend_username(cx).is_empty()
	}

	/// Sends one request through `add_friend`; its validation messages become notices.
	pub(crate) fn send_friend_request(&mut self, cx: &mut Context<Self>) {
		if !self.can_send_friend_request(cx) {
			return;
		}
		let username = self.friend_username(cx);
		let Some(command) = self.state.add_friend(&username) else {
			if let Some(status) = self.state.take_user_action_status() {
				self.notify_user(status);
			}
			cx.notify();
			return;
		};
		self.dispatch(Some(command));
		if let Some(input) = &self.friends.username {
			input.update(cx, |input, cx| input.set_value(String::new(), cx));
		}
		let wanted = username.trim_start_matches('@').to_ascii_lowercase();
		self.when_settled(
			|state| state.user_action_pending() && state.friend_challenge().is_none(),
			move |this, _| {
				if let Some((request, _)) = this.state.friend_challenge() {
					this.state.cancel_friend_challenge(request);
					this.notify_user(
						"Discord asked for a CAPTCHA; send this request from the main Serein app",
					);
					return;
				}
				let confirmed = this
					.state
					.pending_friends()
					.any(|(_, name, incoming)| !incoming && name.eq_ignore_ascii_case(&wanted));
				this.notify_user(if confirmed {
					format!("Friend request sent to {wanted}")
				} else {
					"Friend request finished · it appears under Pending once Discord confirms it"
						.to_owned()
				});
			},
			cx,
		);
		cx.notify();
	}

	fn render_add_friend(&self, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let enabled = self.can_send_friend_request(cx);
		let busy = self.state.user_action_pending();
		div()
			.flex_1()
			.min_h_0()
			.px(px(30.))
			.pt(px(20.))
			.flex()
			.flex_col()
			.gap(px(8.))
			.child(
				div()
					.text_size(px(20.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.text_strong))
					.child("Add Friend"),
			)
			.child(
				div()
					.text_size(px(14.))
					.text_color(color(p.muted))
					.child("You can add friends with their Discord username."),
			)
			.child(
				div()
					.mt(px(8.))
					.h(px(52.))
					.pl(px(12.))
					.pr(px(8.))
					.rounded(px(8.))
					.bg(color(p.base))
					.border_1()
					.border_color(color(p.border))
					.flex()
					.items_center()
					.gap(px(12.))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.text_size(px(16.))
							.text_color(color(p.text_strong))
							.children(self.friends.username.clone()),
					)
					.child(
						div()
							.id("send-friend-request")
							.h(px(34.))
							.px(px(16.))
							.flex_none()
							.rounded(px(6.))
							.flex()
							.items_center()
							.text_size(px(14.))
							.font_weight(FontWeight::MEDIUM)
							.bg(color(p.accent))
							.text_color(color(p.accent_text))
							.when(enabled, |d| {
								d.cursor_pointer().on_click(
									cx.listener(|this, _, _, cx| this.send_friend_request(cx)),
								)
							})
							.when(!enabled, |d| d.opacity(0.5))
							.child(if busy {
								"Sending…"
							} else {
								"Send Friend Request"
							}),
					),
			)
			.when(self.state.demo, |d| {
				d.child(
					div()
						.text_size(px(13.))
						.text_color(color(p.muted))
						.child("Offline demo · requests are simulated."),
				)
			})
			.into_any_element()
	}

	/// The bordered search box above the list, as egui's.
	fn friends_search(&self, placeholder_height: f32) -> Div {
		let p = palette();
		div()
			.flex_none()
			.min_h(px(placeholder_height))
			.px(px(12.))
			.py(px(8.))
			.rounded(px(8.))
			.border_1()
			.border_color(color(p.border))
			.flex()
			.items_center()
			.gap(px(8.))
			.child(icon(Icon::Search, px(18.), color(p.muted)))
			.child(
				div()
					.flex_1()
					.min_w_0()
					.text_size(px(15.))
					.text_color(color(p.text_strong))
					.children(self.friends.search.clone()),
			)
	}

	pub(crate) fn render_friends(&self, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let tab = self.friends.tab;
		let page = div()
			.flex_1()
			.min_w_0()
			.h_full()
			.bg(color(p.chat))
			.flex()
			.flex_col()
			.child(self.friends_header(cx));
		if tab == Tab::AddFriend {
			return page.child(self.render_add_friend(cx)).into_any_element();
		}
		let query = self.friend_query(cx);
		let outgoing = self.friends.outgoing;
		let rows = list(&self.state, tab, &query, outgoing);
		let count = rows.len();
		let empty = match tab {
			Tab::Pending if !self.state.friend_requests_known() => {
				"Friend requests are not available yet."
			}
			Tab::Pending if !query.trim().is_empty() => "No requests match your search.",
			Tab::Pending if outgoing => "No outgoing friend requests.",
			Tab::Pending => "No incoming friend requests.",
			Tab::Restricted if !self.state.restricted_users_known() => {
				"Blocked and ignored users are not available yet."
			}
			_ if tab != Tab::Restricted && !self.state.friends_known() => {
				"Friends are not available yet."
			}
			Tab::Restricted if !query.trim().is_empty() => {
				"No blocked or ignored users match your search."
			}
			_ if !query.trim().is_empty() => "No friends match your search.",
			Tab::Restricted => "No blocked or ignored users.",
			Tab::All => "No friends yet.",
			_ => "No friends are currently online.",
		};
		let body = div()
			.flex_1()
			.min_h_0()
			.px(px(24.))
			.pt(px(35.))
			.pb(px(24.))
			.flex()
			.flex_col();
		let body = if tab == Tab::Pending {
			let counts = [false, true].map(|outgoing| {
				self.state
					.pending_friends()
					.filter(|(_, _, incoming)| *incoming != outgoing)
					.count()
			});
			body.child(
				div()
					.flex()
					.gap(px(8.))
					.children([false, true].map(|choice| {
						let selected = outgoing == choice;
						div()
							.id(if choice {
								"requests-outgoing"
							} else {
								"requests-incoming"
							})
							.px(px(4.))
							.py(px(2.))
							.rounded(px(4.))
							.cursor_pointer()
							.text_size(px(15.))
							.text_color(color(if selected { p.text_strong } else { p.text }))
							.when(selected, |d| d.bg(color(p.selected)))
							.when(!selected, |d| d.hover(|d| d.bg(color(p.hover))))
							.on_click(cx.listener(move |this, _, _, cx| {
								this.friends.outgoing = choice;
								cx.notify();
							}))
							.child(format!(
								"{} — {}",
								if choice { "Outgoing" } else { "Incoming" },
								counts[usize::from(choice)]
							))
					})),
			)
			.child(div().mt(px(12.)).child(self.friends_search(40.)))
			.child(div().h(px(16.)))
		} else {
			body.child(self.friends_search(0.)).child(
				div()
					.mt(px(25.))
					.mb(px(17.))
					.text_size(px(13.))
					.text_color(color(p.muted))
					.child(format!(
						"{} — {count}",
						match tab {
							Tab::All => "All friends",
							Tab::Restricted => "Blocked & ignored",
							_ => "Online",
						}
					)),
			)
		};
		page.child(
			body.when(count == 0, |d| {
				d.child(
					div()
						.pt(px(if tab == Tab::Pending { 0. } else { 20. }))
						.text_size(px(15.))
						.text_color(color(p.muted))
						.child(empty),
				)
			})
			.when(count > 0, |d| {
				d.child(
					uniform_list(
						"friends-list",
						count,
						cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
							range
								.filter_map(|index| rows.get(index).copied())
								.map(|(user, incoming)| {
									if tab == Tab::Pending {
										this.request_row(user, incoming, cx)
									} else {
										this.friend_row(user, cx)
									}
								})
								.collect::<Vec<_>>()
						}),
					)
					.flex_1(),
				)
			}),
		)
		.into_any_element()
	}
}

/// Offline (`--demo`) replies to user actions, as the desktop fixture: an opened DM gets a
/// synthetic channel, a sent request an outgoing synthetic entry, and other writes succeed.
pub(crate) fn demo_user_events(
	state: &State,
	action: client_core::user_actions::Action,
	request: u64,
) -> Vec<client_core::Event> {
	use client_core::user_actions::{Action, Event};
	use client_core::{Event as Update, auth::Failure};
	match action {
		Action::OpenDm(user) => vec![Update::UserAction(Event::DmOpened {
			user,
			request,
			result: state
				.friend(user)
				.map(|friend| {
					Box::new(model::Channel {
						id: Id(100_000 + user.0),
						guild: None,
						name: friend.name.clone(),
						kind: 1,
						parent_id: None,
						position: 0,
						recipients: vec![friend.clone()],
						last_message: None,
						icon: None,
						member_list_id: None,
						message_count: None,
					})
				})
				.ok_or(Failure::Forbidden),
		})],
		Action::AddFriend { username } => {
			let user = model::User {
				id: Id(900_000 + request),
				name: format!("{username} (synthetic)"),
				avatar: None,
				discriminator: 0,
				primary_guild: None,
				webhook: false,
				kind: Default::default(),
			};
			vec![
				Update::UserAction(Event::Written {
					action: Action::AddFriend {
						username: username.clone(),
					},
					request,
					result: Ok(()),
				}),
				Update::UserAction(Event::Request {
					user: user.id,
					incoming: Some(false),
					profile: Some((user, username)),
				}),
			]
		}
		action => vec![Update::UserAction(Event::Written {
			action,
			request,
			result: Ok(()),
		})],
	}
}

#[cfg(test)]
mod tests {
	use super::{Tab, rows};

	#[test]
	fn tabs_filter_presence_and_list_incoming_requests_first() {
		let state = test_support::friends_demo_state();
		let all = rows(&state, Tab::All);
		let online = rows(&state, Tab::Online);
		assert_eq!(all.len(), state.friends().count());
		assert!(!online.is_empty() && online.len() < all.len());
		assert!(online.iter().all(|row| all.contains(row)));
		let names = all
			.iter()
			.map(|(id, _)| state.friend(*id).unwrap().name.clone())
			.collect::<Vec<_>>();
		assert!(names.is_sorted());
		let pending = rows(&state, Tab::Pending);
		assert!(!pending.is_empty());
		assert!(pending[0].1, "incoming requests lead");
		assert!(pending.is_sorted_by_key(|(_, incoming)| !incoming));
	}
}
