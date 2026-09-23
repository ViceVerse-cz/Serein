//! Friends page on the home view: Online / All / Pending from the relationships already in
//! `client_core::State`, and Add Friend. Accept, decline and cancel go through
//! `resolve_friend_request`, requests through `add_friend`; right-clicking a friend opens the
//! shared navigation menu (Message, Remove Friend, Block). The search field stays in the main app.
use crate::sidebar::avatar_with_presence;
use crate::theme::{Icon, color, icon, palette};
use crate::{Serein, tooltip};
use client_core::State;
use gpui::{prelude::*, *};
use model::Id;

const ROW_HEIGHT: f32 = 62.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
	#[default]
	Online,
	All,
	Pending,
	AddFriend,
}

impl Tab {
	const ALL: [Self; 4] = [Self::Online, Self::All, Self::Pending, Self::AddFriend];
	fn label(self) -> &'static str {
		match self {
			Self::Online => "Online",
			Self::All => "All",
			Self::Pending => "Pending",
			Self::AddFriend => "Add Friend",
		}
	}
}

#[derive(Default)]
pub struct Page {
	/// Chosen from the "Friends" row; any channel selection hides the page again.
	pub open: bool,
	pub tab: Tab,
	/// The Add Friend username field, created the first time that tab opens.
	pub username: Option<Entity<crate::input::Input>>,
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

/// Rows for a tab as `(user, incoming)`, sorted by name then ID like the egui list. `incoming` is
/// only meaningful on Pending, where incoming requests come first.
pub fn rows(state: &State, tab: Tab) -> Vec<(Id, bool)> {
	let mut rows = match tab {
		Tab::AddFriend => Vec::new(),
		Tab::Pending => state
			.pending_friends()
			.map(|(user, _, incoming)| (user, *incoming))
			.collect::<Vec<_>>(),
		Tab::All | Tab::Online => state
			.friends()
			.filter(|user| {
				tab == Tab::All
					|| online(
						state
							.presence_for(user.id)
							.and_then(|p| p.status.as_deref()),
					)
			})
			.map(|user| (user, false))
			.collect(),
	};
	rows.sort_unstable_by(|(a, a_in), (b, b_in)| {
		b_in.cmp(a_in)
			.then_with(|| a.name.cmp(&b.name))
			.then(a.id.cmp(&b.id))
	});
	rows.into_iter()
		.map(|(user, incoming)| (user.id, incoming))
		.collect()
}

fn incoming_requests(state: &State) -> usize {
	state
		.pending_friends()
		.filter(|(_, _, incoming)| *incoming)
		.count()
}

fn round_button(
	id: impl Into<ElementId>,
	glyph: Icon,
	label: &'static str,
	hover: Rgba,
) -> Stateful<Div> {
	let p = palette();
	div()
		.id(id)
		.size(px(36.))
		.flex_none()
		.rounded_full()
		.bg(color(p.raised))
		.flex()
		.items_center()
		.justify_center()
		.tooltip(tooltip(label))
		.child(
			icon(glyph, px(18.), color(p.muted)).group_hover(label, move |s| s.text_color(hover)),
		)
		.group(label)
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

	fn friend_row(&self, user: Id, incoming: bool, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let pending = self.friends.tab == Tab::Pending;
		let record = if pending {
			self.state
				.pending_friends()
				.find(|(u, _, _)| u.id == user)
				.map(|(u, name, _)| (u, Some(name.as_str())))
		} else {
			self.state
				.friend(user)
				.map(|u| (u, self.state.friend_username(user)))
		};
		let Some((record, username)) = record else {
			return div().h(px(ROW_HEIGHT)).into_any_element();
		};
		let presence = (!pending).then(|| self.state.presence_for(user)).flatten();
		let status = presence.and_then(|p| p.status.as_deref());
		let name = self.state.user_display_name(record).to_owned();
		let subtitle = if pending {
			if incoming {
				"Incoming Friend Request".to_owned()
			} else {
				"Outgoing Friend Request".to_owned()
			}
		} else {
			presence
				.and_then(|p| p.custom_status.clone())
				.filter(|text| !text.is_empty())
				.unwrap_or_else(|| presence_label(status).to_owned())
		};
		let available =
			!self.state.user_action_pending() && (self.state.demo || self.state.gateway_connected);
		let mut actions = div().flex().gap(px(10.));
		if pending {
			if incoming {
				actions = actions.child(
					round_button(
						("friend-accept", user.0),
						Icon::Check,
						"Accept",
						color(p.positive),
					)
					.when(available, |d| {
						d.cursor_pointer()
							.on_click(cx.listener(move |this, _, _, cx| {
								let command = this.state.resolve_friend_request(user, true);
								this.dispatch(command);
								cx.notify();
							}))
					})
					.when(!available, |d| d.opacity(0.5)),
				);
			}
			actions = actions.child(
				round_button(
					("friend-decline", user.0),
					Icon::Close,
					if incoming { "Ignore" } else { "Cancel" },
					color(p.danger),
				)
				.when(available, |d| {
					d.cursor_pointer()
						.on_click(cx.listener(move |this, _, _, cx| {
							let command = this.state.resolve_friend_request(user, false);
							this.dispatch(command);
							cx.notify();
						}))
				})
				.when(!available, |d| d.opacity(0.5)),
			);
		} else {
			actions = actions.child(
				round_button(
					("friend-message", user.0),
					Icon::Chats,
					"Message",
					color(p.text_strong),
				)
				.when(available, |d| {
					d.cursor_pointer()
						.on_click(cx.listener(move |this, _, _, cx| this.message_friend(user, cx)))
				})
				.when(!available, |d| d.opacity(0.5)),
			);
		}
		div()
			.id(("friend", user.0))
			.h(px(ROW_HEIGHT))
			.border_t_1()
			.border_color(color(p.border))
			.when(!pending, |d| {
				d.on_mouse_down(
					MouseButton::Right,
					cx.listener(move |this, event: &MouseDownEvent, window, cx| {
						let target = crate::nav_menu::Target::Friend(user);
						this.open_nav_menu(target, event.position, window, cx);
						cx.stop_propagation();
					}),
				)
			})
			.child(
				div()
					.size_full()
					.px(px(10.))
					.rounded(px(8.))
					.flex()
					.items_center()
					.gap(px(12.))
					.hover(|d| d.bg(color(p.hover)))
					.child(avatar_with_presence(
						&name,
						40.,
						Some(record),
						status,
						color(p.chat),
					))
					.child(
						div()
							.flex_1()
							.min_w_0()
							.flex()
							.flex_col()
							.child(
								div()
									.flex()
									.items_baseline()
									.gap(px(6.))
									.overflow_hidden()
									.whitespace_nowrap()
									.child(
										div()
											.text_size(px(16.))
											.font_weight(FontWeight::SEMIBOLD)
											.text_color(color(p.text_strong))
											.child(name),
									)
									.children(username.map(|username| {
										div()
											.text_size(px(13.))
											.text_color(color(p.muted))
											.child(username.to_owned())
									})),
							)
							.child(
								div()
									.overflow_hidden()
									.whitespace_nowrap()
									.text_ellipsis()
									.text_size(px(13.))
									.text_color(color(p.muted))
									.child(subtitle),
							),
					)
					.child(actions),
			)
			.into_any_element()
	}

	fn friends_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
		let p = palette();
		let requests = incoming_requests(&self.state);
		div()
			.h(px(48.))
			.flex_none()
			.px_4()
			.border_b_1()
			.border_color(color(p.border))
			.flex()
			.items_center()
			.gap(px(8.))
			.child(icon(Icon::Users, px(22.), color(p.muted)))
			.child(
				div()
					.text_size(px(16.))
					.font_weight(FontWeight::SEMIBOLD)
					.text_color(color(p.text_strong))
					.child("Friends"),
			)
			.child(div().mx(px(8.)).w(px(1.)).h(px(24.)).bg(color(p.border)))
			.children(Tab::ALL.map(|tab| {
				let selected = self.friends.tab == tab;
				let add = tab == Tab::AddFriend;
				div()
					.id(tab.label())
					.focusable()
					.tab_stop(true)
					.h(px(30.))
					.px(px(10.))
					.rounded(px(6.))
					.flex()
					.items_center()
					.gap(px(6.))
					.cursor_pointer()
					.text_size(px(15.))
					.font_weight(FontWeight::MEDIUM)
					.when(add, |d| d.ml(px(8.)))
					.when(add && selected, |d| d.text_color(color(p.accent)))
					.when(add && !selected, |d| {
						d.bg(color(p.accent)).text_color(color(p.accent_text))
					})
					.when(selected && !add, |d| {
						d.bg(color(p.selected)).text_color(color(p.text_strong))
					})
					.when(!selected && !add, |d| {
						d.text_color(color(p.muted))
							.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
					})
					.focus(|d| d.bg(color(p.hover)))
					.on_click(cx.listener(move |this, _, window, cx| {
						this.friends.tab = tab;
						if tab == Tab::AddFriend {
							this.focus_friend_username(window, cx);
						}
						cx.notify();
					}))
					.child(tab.label())
					.when(tab == Tab::Pending && requests > 0, |d| {
						d.child(crate::sidebar::count_pill(requests as u32))
					})
			}))
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

	pub(crate) fn render_friends(&self, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let tab = self.friends.tab;
		if tab == Tab::AddFriend {
			return div()
				.flex_1()
				.min_w_0()
				.h_full()
				.bg(color(p.chat))
				.flex()
				.flex_col()
				.child(self.friends_header(cx))
				.child(self.render_add_friend(cx))
				.into_any_element();
		}
		let rows = rows(&self.state, tab);
		let heading = format!(
			"{} — {}",
			match tab {
				Tab::Online => "ONLINE",
				Tab::All => "ALL FRIENDS",
				Tab::Pending | Tab::AddFriend => "PENDING",
			},
			rows.len()
		);
		let known = if matches!(tab, Tab::Pending | Tab::AddFriend) {
			self.state.friend_requests_known()
		} else {
			self.state.friends_known()
		};
		let empty = match tab {
			_ if !known => "Friends are not available yet.",
			Tab::Online => "No friends are currently online.",
			Tab::All => "No friends yet.",
			Tab::Pending | Tab::AddFriend => "No pending friend requests.",
		};
		let count = rows.len();
		div()
			.flex_1()
			.min_w_0()
			.h_full()
			.bg(color(p.chat))
			.flex()
			.flex_col()
			.child(self.friends_header(cx))
			.child(
				div()
					.flex_1()
					.min_h_0()
					.px(px(24.))
					.pt(px(16.))
					.flex()
					.flex_col()
					.child(
						div()
							.pb(px(12.))
							.text_size(px(12.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(color(p.muted))
							.child(heading),
					)
					.when(count == 0, |d| {
						d.child(
							div()
								.pt(px(20.))
								.text_size(px(14.))
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
										.map(|(user, incoming)| this.friend_row(user, incoming, cx))
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
