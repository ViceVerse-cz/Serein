//! Right-click menus for channel and category rows, conversations, server tiles and friends.
//! Every write goes through a `client_core::State` method and `Serein::dispatch`; copies only put
//! public IDs and links on the clipboard. Nested choices (mute durations, notification levels)
//! open as a page inside the same menu, and irreversible actions ask for native confirmation.
use crate::Serein;
use crate::theme::{Icon, color, icon, palette, solid, tint};
use crate::tooltip;
use client_core::State;
use client_core::channel_actions::{Action as ChannelAction, Mute, Outcome};
use gpui::{prelude::*, *};
use model::Id;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
	/// A guild channel, a category (kind 4), a direct message or a group.
	Channel(Id),
	Guild(Id),
	Friend(Id),
}

/// The menu's current panel; submenus replace the root list until "Back".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
	#[default]
	Root,
	Mute,
	Notifications,
}

pub struct Menu {
	target: Target,
	page: Page,
	position: Point<Pixels>,
	focus: FocusHandle,
}

/// Mute durations offered by Discord and the egui channel menu; the reducer accepts only these.
pub const MUTE_CHOICES: [(&str, Mute); 6] = [
	("For 15 Minutes", Mute::For(900)),
	("For 1 Hour", Mute::For(3600)),
	("For 3 Hours", Mute::For(10800)),
	("For 8 Hours", Mute::For(28800)),
	("For 24 Hours", Mute::For(86400)),
	("Until I Turn It Back On", Mute::Forever),
];

/// Per-channel notification levels, as Discord's wire values; 3 inherits from the parent.
pub fn level_choices(in_category: bool) -> [(u8, &'static str); 4] {
	[
		(0, "All Messages"),
		(1, "Only @mentions"),
		(2, "Nothing"),
		(
			3,
			if in_category {
				"Use Category Default"
			} else {
				"Use Server Default"
			},
		),
	]
}

/// When a timed mute ends, in Unix seconds; indefinite mutes and unmutes have no expiry.
pub fn mute_until(mute: Mute, now: i64) -> Option<i64> {
	match mute {
		Mute::For(seconds) => Some(now.saturating_add(i64::from(seconds))),
		Mute::Unmute | Mute::Forever => None,
	}
}

fn now() -> i64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_secs().min(i64::MAX as u64) as i64)
}

/// Offline (`--demo`) stand-in for Discord's reply to a personal channel preference write, as
/// the desktop fixture's `channel_demo`. Other channel actions have no offline equivalent.
pub(crate) fn demo_channel_outcome(action: &ChannelAction) -> Option<Outcome> {
	match *action {
		ChannelAction::Mute(mute) => Some(Outcome::Preferences {
			muted: Some(mute != Mute::Unmute),
			level: None,
			mute_until: mute_until(mute, now()),
		}),
		ChannelAction::Notifications(level) => Some(Outcome::Preferences {
			muted: None,
			level: Some(level),
			mute_until: None,
		}),
		ChannelAction::HideMuted(hide) => Some(Outcome::HideMuted(hide)),
		_ => None,
	}
}

/// Discord's shareable link for a channel; direct messages use the `@me` scope.
pub fn channel_link(guild: Option<Id>, channel: Id) -> String {
	match guild {
		Some(guild) => format!("https://discord.com/channels/{guild}/{channel}"),
		None => format!("https://discord.com/channels/@me/{channel}"),
	}
}

type Action = Box<dyn Fn(&mut Serein, &mut Window, &mut Context<Serein>)>;

enum Run {
	Act(Action),
	Open(Page),
}

enum Trailing {
	None,
	Icon(Icon),
	Radio(bool),
	Toggle(bool),
	Submenu,
}

struct Row {
	label: SharedString,
	trailing: Trailing,
	enabled: bool,
	danger: bool,
	hint: Option<&'static str>,
	run: Run,
}

impl Row {
	fn new(
		label: impl Into<SharedString>,
		enabled: bool,
		run: impl Fn(&mut Serein, &mut Window, &mut Context<Serein>) + 'static,
	) -> Self {
		Self {
			label: label.into(),
			trailing: Trailing::None,
			enabled,
			danger: false,
			hint: None,
			run: Run::Act(Box::new(run)),
		}
	}
	fn submenu(label: &'static str, enabled: bool, page: Page) -> Self {
		Self {
			label: label.into(),
			trailing: Trailing::Submenu,
			enabled,
			danger: false,
			hint: None,
			run: Run::Open(page),
		}
	}
	fn icon(mut self, glyph: Icon) -> Self {
		self.trailing = Trailing::Icon(glyph);
		self
	}
	fn radio(mut self, on: bool) -> Self {
		self.trailing = Trailing::Radio(on);
		self
	}
	fn toggle(mut self, on: bool) -> Self {
		self.trailing = Trailing::Toggle(on);
		self
	}
	fn danger(mut self) -> Self {
		self.danger = true;
		self
	}
	/// Shown as a tooltip while the row is disabled.
	fn hint(mut self, hint: Option<&'static str>) -> Self {
		self.hint = hint;
		self
	}
}

enum Item {
	Row(Row),
	Separator,
	/// A submenu's title, which also leads back to the root list.
	Title(&'static str),
}

impl From<Row> for Item {
	fn from(row: Row) -> Self {
		Self::Row(row)
	}
}

const SETTLE_TICK: Duration = Duration::from_millis(100);
/// How long a watcher waits for a pending write before giving up quietly (60 s).
const SETTLE_TICKS: usize = 600;

fn copy_row(label: &'static str, text: String, notice: &'static str, glyph: Icon) -> Item {
	Row::new(label, true, move |this, _, cx| {
		cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
		this.notify_user(notice);
	})
	.icon(glyph)
	.into()
}

impl Serein {
	pub(crate) fn open_nav_menu(
		&mut self,
		target: Target,
		position: Point<Pixels>,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		let focus = cx.focus_handle();
		window.focus(&focus, cx);
		self.navigation.menu = Some(Menu {
			target,
			page: Page::Root,
			position,
			focus,
		});
		cx.notify();
	}

	/// Opens a submenu page directly, for the screenshot flags.
	pub(crate) fn set_nav_menu_page(&mut self, page: Page) {
		if let Some(menu) = &mut self.navigation.menu {
			menu.page = page;
		}
	}

	fn close_nav_menu(&mut self, cx: &mut Context<Self>) {
		if self.navigation.menu.take().is_some() {
			cx.notify();
		}
	}

	fn writes_available(&self) -> bool {
		self.state.demo || self.state.gateway_connected
	}

	/// Runs `then` once `busy` reports the reducer's single pending write finished (at once if
	/// nothing is pending). The reducer bounds each write; this only waits to report it.
	pub(crate) fn when_settled(
		&mut self,
		busy: fn(&State) -> bool,
		then: impl FnOnce(&mut Serein, &mut Context<Serein>) + 'static,
		cx: &mut Context<Self>,
	) {
		if !busy(&self.state) {
			then(self, cx);
			cx.notify();
			return;
		}
		let mut then = Some(then);
		cx.spawn(async move |this, cx| {
			for _ in 0..SETTLE_TICKS {
				cx.background_executor().timer(SETTLE_TICK).await;
				let done = this.update(cx, |this, cx| {
					if busy(&this.state) {
						return false;
					}
					if let Some(then) = then.take() {
						then(this, cx);
						cx.notify();
					}
					true
				});
				if done.unwrap_or(true) {
					return;
				}
			}
		})
		.detach();
	}

	/// Asks with a native dialog before an irreversible action; only the first button proceeds.
	pub(crate) fn confirm(
		&mut self,
		message: &str,
		detail: &str,
		button: &'static str,
		window: &mut Window,
		cx: &mut Context<Self>,
		then: impl FnOnce(&mut Serein, &mut Context<Serein>) + 'static,
	) {
		let answer = window.prompt(
			PromptLevel::Warning,
			message,
			Some(detail),
			&[button, "Cancel"],
			cx,
		);
		cx.spawn(async move |this, cx| {
			if answer.await == Ok(0) {
				let _ = this.update(cx, |this, cx| {
					then(this, cx);
					cx.notify();
				});
			}
		})
		.detach();
	}

	/// A personal channel preference write, reported through the reducer's channel status.
	fn write_channel(&mut self, channel: Id, action: ChannelAction, cx: &mut Context<Self>) {
		let command = self.state.request_channel_action(channel, action);
		self.dispatch(command);
		self.when_settled(
			State::channel_action_pending,
			move |this, _| {
				if let Some(status) = this.state.channel_action_status(channel) {
					this.notify_user(status);
				}
				this.sync_channels();
			},
			cx,
		);
	}

	/// Marks each unread channel of a category read. The reducer allows one read write at a
	/// time, so the rest wait for the previous one (bounded by [`SETTLE_TICKS`]).
	fn mark_category_read(&mut self, category: Id, cx: &mut Context<Self>) {
		let queue = self
			.state
			.channels
			.iter()
			.filter(|c| c.parent_id == Some(category) && c.kind != 4)
			.filter(|c| {
				self.state.can_mark_channel_read(c.id) || self.state.unread(c.id) == Some(true)
			})
			.map(|c| c.id)
			.take(client_core::MAX_NAV)
			.collect::<Vec<_>>();
		cx.spawn(async move |this, cx| {
			let mut queue = queue.into_iter().peekable();
			let mut waited = 0;
			while let Some(&channel) = queue.peek() {
				let step = this.update(cx, |this, cx| {
					if this.state.can_mark_channel_read(channel) {
						let command = this.state.prepare_mark_channel_read(channel);
						this.dispatch(command);
						cx.notify();
						Some(true)
					} else if this.state.unread(channel) != Some(true) {
						Some(true)
					} else {
						Some(false)
					}
				});
				match step {
					Ok(Some(true)) => {
						queue.next();
						waited = 0;
					}
					Ok(_) if waited < SETTLE_TICKS / 6 => {
						waited += 1;
						cx.background_executor().timer(SETTLE_TICK).await;
					}
					_ => return,
				}
			}
		})
		.detach();
	}

	fn channel_items(&self, id: Id, page: Page) -> Vec<Item> {
		let Some(channel) = self.state.channel(id) else {
			return Vec::new();
		};
		if channel.guild.is_none() {
			return self.direct_items(id, channel.kind);
		}
		let guild = channel.guild;
		let category = channel.kind == 4;
		let thread = matches!(channel.kind, 10..=12);
		let in_category = channel
			.parent_id
			.and_then(|parent| self.state.channel(parent))
			.is_some_and(|parent| parent.kind == 4);
		let available = self.writes_available()
			&& !self.state.channel_action_pending()
			&& self.state.can_view(id);
		let write = move |action: ChannelAction| {
			move |this: &mut Serein, _: &mut Window, cx: &mut Context<Serein>| {
				this.write_channel(id, action.clone(), cx)
			}
		};
		match page {
			Page::Mute => {
				let mut items = vec![Item::Title(if category {
					"Mute Category"
				} else {
					"Mute Channel"
				})];
				items.extend(MUTE_CHOICES.map(|(label, mute)| {
					Item::from(Row::new(label, available, write(ChannelAction::Mute(mute))))
				}));
				return items;
			}
			Page::Notifications => {
				let level = self.state.channel_notification_level(id);
				let mut items = vec![Item::Title("Notification Settings")];
				items.extend(
					level_choices(in_category && !category).map(|(value, label)| {
						Item::from(
							Row::new(label, available, write(ChannelAction::Notifications(value)))
								.radio(level == Some(value)),
						)
					}),
				);
				return items;
			}
			Page::Root => {}
		}
		let mut items = Vec::new();
		if category {
			let unread = self
				.state
				.channels
				.iter()
				.any(|c| c.parent_id == Some(id) && self.state.can_mark_channel_read(c.id));
			items.push(
				Row::new("Mark As Read", unread, move |this, _, cx| {
					this.mark_category_read(id, cx)
				})
				.icon(Icon::Check)
				.into(),
			);
			items.push(Item::Separator);
			let collapsed = self.collapsed.contains(&id);
			items.push(
				Row::new(
					if collapsed {
						"Expand Category"
					} else {
						"Collapse Category"
					},
					true,
					move |this, _, cx| {
						if !this.collapsed.remove(&id) {
							this.collapsed.insert(id);
						}
						this.sync_channels();
						cx.notify();
					},
				)
				.into(),
			);
			let categories = self
				.state
				.channels
				.iter()
				.filter(|c| c.guild == guild && c.kind == 4)
				.map(|c| c.id)
				.collect::<Vec<_>>();
			let all_collapsed = categories.iter().all(|c| self.collapsed.contains(c));
			items.push(
				Row::new(
					if all_collapsed {
						"Expand All Categories"
					} else {
						"Collapse All Categories"
					},
					!categories.is_empty(),
					move |this, _, cx| {
						for category in &categories {
							if all_collapsed {
								this.collapsed.remove(category);
							} else {
								this.collapsed.insert(*category);
							}
						}
						this.sync_channels();
						cx.notify();
					},
				)
				.into(),
			);
		} else {
			items.push(
				Row::new(
					"Mark As Read",
					self.state.can_mark_channel_read(id),
					move |this, _, _| {
						let command = this.state.prepare_mark_channel_read(id);
						this.dispatch(command);
					},
				)
				.icon(Icon::Check)
				.into(),
			);
			items.push(Item::Separator);
			items.push(copy_row(
				"Copy Link",
				channel_link(guild, id),
				"Link copied",
				Icon::Link,
			));
		}
		// Threads and forum posts follow their own follow/mute rules; the main app owns those.
		if !thread {
			items.push(Item::Separator);
			if self.state.guild_channel_muted(id) == Some(true) {
				items.push(
					Row::new(
						if category {
							"Unmute Category"
						} else {
							"Unmute Channel"
						},
						available,
						write(ChannelAction::Mute(Mute::Unmute)),
					)
					.into(),
				);
			}
			items.push(
				Row::submenu(
					if category {
						"Mute Category"
					} else {
						"Mute Channel"
					},
					available,
					Page::Mute,
				)
				.into(),
			);
			items
				.push(Row::submenu("Notification Settings", available, Page::Notifications).into());
		}
		items.push(Item::Separator);
		items.push(copy_row(
			if category {
				"Copy Category ID"
			} else {
				"Copy Channel ID"
			},
			id.to_string(),
			if category {
				"Category ID copied"
			} else {
				"Channel ID copied"
			},
			Icon::Copy,
		));
		items
	}

	fn direct_items(&self, id: Id, kind: u8) -> Vec<Item> {
		let available = self.writes_available();
		let user_ready = available && !self.state.user_action_pending();
		let muted = self.state.dm_muted(id);
		let mut items = vec![
			Row::new(
				"Mark As Read",
				self.state.can_mark_channel_read(id),
				move |this, _, _| {
					let command = this.state.prepare_mark_channel_read(id);
					this.dispatch(command);
				},
			)
			.icon(Icon::Check)
			.into(),
			Item::Separator,
			Row::new(
				if muted == Some(true) {
					"Unmute Conversation"
				} else {
					"Mute Conversation"
				},
				user_ready && muted.is_some(),
				move |this, _, cx| {
					let target = this.state.dm_muted(id) != Some(true);
					let command = this.state.set_dm_muted(id, target);
					this.dispatch(command);
					this.when_settled(
						State::user_action_pending,
						move |this, _| {
							this.notify_user(if this.state.dm_muted(id) == Some(target) {
								if target {
									"Conversation muted until you turn it back on"
								} else {
									"Conversation unmuted"
								}
							} else {
								"Conversation notifications were not changed; try again"
							})
						},
						cx,
					);
				},
			)
			.hint((muted.is_none()).then_some("Notification settings are not loaded yet"))
			.into(),
		];
		if kind == 1 {
			items.push(
				Row::new("Close DM", user_ready, move |this, _, cx| {
					let command = this.state.close_dm(id);
					this.dispatch(command);
					this.when_settled(
						State::user_action_pending,
						move |this, cx| {
							this.after_conversation_left(
								id,
								"DM closed · messages were kept",
								"DM was not closed; try again",
								cx,
							)
						},
						cx,
					);
				})
				.into(),
			);
		} else if kind == 3 {
			let reason = self.state.leave_group_reason(id);
			items.push(
				Row::new(
					"Leave Group",
					available && reason.is_none() && !self.state.group_action_pending(),
					move |this, window, cx| {
						let name = this
							.state
							.channel(id)
							.map_or_else(String::new, crate::channel_label);
						this.confirm(
							&format!("Leave '{name}'?"),
							"You won't be able to rejoin this group unless you are added again.",
							"Leave Group",
							window,
							cx,
							move |this, cx| {
								let command = this.state.leave_group(id);
								this.dispatch(command);
								this.when_settled(
									State::group_action_pending,
									move |this, cx| {
										let status = this.state.group_action_status(id);
										this.after_conversation_left(
											id,
											status.unwrap_or("Left group"),
											status.unwrap_or("Group was not left; try again"),
											cx,
										)
									},
									cx,
								);
							},
						);
					},
				)
				.danger()
				.hint(reason)
				.into(),
			);
		}
		items.extend([
			Item::Separator,
			copy_row(
				"Copy Link",
				channel_link(None, id),
				"Link copied",
				Icon::Link,
			),
			copy_row(
				"Copy Channel ID",
				id.to_string(),
				"Channel ID copied",
				Icon::Copy,
			),
		]);
		items
	}

	/// After closing a DM or leaving a group: report the result and leave the view if it was open.
	fn after_conversation_left(
		&mut self,
		id: Id,
		done: &'static str,
		failed: &'static str,
		cx: &mut Context<Self>,
	) {
		if self.state.channel(id).is_some() {
			self.notify_user(failed);
			return;
		}
		self.notify_user(done);
		if self.state.selected.is_none() && self.guild.is_none() {
			self.select_section(None, cx);
		} else {
			self.sync_channels();
		}
	}

	fn guild_items(&self, id: Id) -> Vec<Item> {
		let available = self.writes_available();
		let hidden = self.state.hides_muted_channels(id) == Some(true);
		let anchor = self
			.state
			.channels
			.iter()
			.find(|c| c.guild == Some(id) && self.state.can_view(c.id))
			.map(|c| c.id);
		let reason = self.state.leave_server_reason(id);
		vec![
			Row::new(
				"Mark As Read",
				self.state.can_mark_guild_read(id),
				move |this, _, _| {
					let command = this.state.prepare_mark_guild_read(id);
					this.dispatch(command);
				},
			)
			.icon(Icon::Check)
			.into(),
			Item::Separator,
			Row::new(
				"Hide Muted Channels",
				available && anchor.is_some() && !self.state.channel_action_pending(),
				move |this, _, cx| {
					if let Some(anchor) = anchor {
						let hide = this.state.hides_muted_channels(id) != Some(true);
						this.write_channel(anchor, ChannelAction::HideMuted(hide), cx);
						this.sync_channels();
					}
				},
			)
			.toggle(hidden)
			.into(),
			Item::Separator,
			Row::new(
				"Leave Server",
				available
					&& reason.is_none()
					&& !self.state.server_action_pending()
					&& !self.state.server_invite_pending(),
				move |this, window, cx| {
					let name = this
						.state
						.guild(id)
						.map_or_else(String::new, |g| g.name.clone());
					this.confirm(
						&format!("Leave '{name}'?"),
						"You won't be able to rejoin this server unless you are re-invited.",
						"Leave Server",
						window,
						cx,
						move |this, cx| {
							let command = this.state.leave_server(id);
							this.dispatch(command);
							this.when_settled(
								State::server_action_pending,
								move |this, cx| {
									if let Some(status) = this.state.server_action_status(id) {
										this.notify_user(status);
									}
									if this.state.guild(id).is_none() && this.guild == Some(id) {
										this.select_section(None, cx);
									} else {
										this.sync_channels();
									}
								},
								cx,
							);
						},
					);
				},
			)
			.danger()
			.hint(reason)
			.into(),
			Item::Separator,
			copy_row(
				"Copy Server ID",
				id.to_string(),
				"Server ID copied",
				Icon::Copy,
			),
		]
	}

	fn friend_items(&self, user: Id) -> Vec<Item> {
		let ready = self.writes_available() && !self.state.user_action_pending();
		let friend = self.state.friend(user).is_some();
		let blocked = self.state.user_blocked(user) == Some(true);
		vec![
			Row::new("Message", friend && ready, move |this, _, cx| {
				this.message_friend(user, cx)
			})
			.icon(Icon::Chats)
			.into(),
			Item::Separator,
			Row::new("Remove Friend", friend && ready, move |this, window, cx| {
				let name = this.friend_name(user);
				this.confirm(
					&format!("Remove '{name}'?"),
					&format!("Are you sure you want to remove {name} from your friends?"),
					"Remove Friend",
					window,
					cx,
					move |this, cx| {
						let command = this.state.remove_friend(user);
						this.dispatch(command);
						this.when_settled(
							State::user_action_pending,
							move |this, _| {
								this.notify_user(if this.state.friend(user).is_none() {
									"Friend removed"
								} else {
									"Friend was not removed; try again"
								})
							},
							cx,
						);
					},
				);
			})
			.danger()
			.into(),
			Row::new(
				if blocked { "Unblock" } else { "Block" },
				ready,
				move |this, window, cx| {
					let block = move |this: &mut Serein, cx: &mut Context<Serein>| {
						let command = this.state.set_user_blocked(user, !blocked);
						this.dispatch(command);
						this.when_settled(
							State::user_action_pending,
							move |this, _| {
								this.notify_user(
									if this.state.user_blocked(user) == Some(!blocked) {
										if blocked {
											"User unblocked"
										} else {
											"User blocked"
										}
									} else {
										"Block setting was not changed; try again"
									},
								)
							},
							cx,
						);
					};
					if blocked {
						block(this, cx);
						return;
					}
					let name = this.friend_name(user);
					this.confirm(
						&format!("Block '{name}'?"),
						"Blocking also removes them from your friends. They can't message you while blocked.",
						"Block",
						window,
						cx,
						block,
					);
				},
			)
			.danger()
			.into(),
			Item::Separator,
			copy_row(
				"Copy User ID",
				user.to_string(),
				"User ID copied",
				Icon::Copy,
			),
		]
	}

	fn friend_name(&self, user: Id) -> String {
		self.state
			.friend(user)
			.map_or_else(String::new, |u| self.state.user_display_name(u).to_owned())
	}

	/// Opens the existing DM with a friend, or asks Discord to open one and shows it once it
	/// arrives (unless the user navigated elsewhere meanwhile).
	pub(crate) fn message_friend(&mut self, user: Id, cx: &mut Context<Self>) {
		let existing = move |state: &State| {
			state
				.channels
				.iter()
				.find(|c| {
					c.guild.is_none()
						&& c.kind == 1 && c.recipients.len() == 1
						&& c.recipients[0].id == user
				})
				.map(|c| c.id)
		};
		if let Some(channel) = existing(&self.state) {
			self.friends.open = false;
			self.select(channel, cx);
			return;
		}
		let origin = self.state.selected;
		let command = self.state.open_friend_dm(user);
		self.dispatch(command);
		self.when_settled(
			State::user_action_pending,
			move |this, cx| {
				if this.state.selected == origin
					&& let Some(channel) = existing(&this.state)
				{
					this.friends.open = false;
					this.select(channel, cx);
				}
			},
			cx,
		);
	}

	fn menu_items(&self, target: Target, page: Page) -> Vec<Item> {
		match target {
			Target::Channel(id) => self.channel_items(id, page),
			Target::Guild(id) => self.guild_items(id),
			Target::Friend(id) => self.friend_items(id),
		}
	}

	fn menu_row(&self, index: usize, row: Row, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		let Row {
			label,
			trailing,
			enabled,
			danger,
			hint,
			run,
		} = row;
		let group: SharedString = format!("nav-menu-row-{index}").into();
		let (text, hover_bg, hover_text) = if danger {
			(p.danger, p.danger, egui::Color32::WHITE)
		} else {
			(p.text, p.accent, p.accent_text)
		};
		let mark = |on: bool, round: bool| {
			div()
				.size(px(18.))
				.flex_none()
				.rounded(px(if round { 9. } else { 4. }))
				.border_2()
				.border_color(color(if on { p.accent } else { p.muted }))
				.when(on && !round, |d| d.bg(color(p.accent)))
				.flex()
				.items_center()
				.justify_center()
				.when(on && round, |d| {
					d.child(div().size(px(8.)).rounded_full().bg(color(p.accent)))
				})
				.when(on && !round, |d| {
					d.child(icon(Icon::Check, px(12.), color(p.accent_text)))
				})
				.group_hover(group.clone(), |d| d.border_color(color(hover_text)))
				.into_any_element()
		};
		let trailing = match trailing {
			Trailing::None => None,
			Trailing::Icon(glyph) => Some(
				icon(glyph, px(16.), color(p.muted))
					.group_hover(group.clone(), |d| d.text_color(color(hover_text)))
					.into_any_element(),
			),
			Trailing::Submenu => Some(
				icon(Icon::CaretRight, px(14.), color(p.muted))
					.group_hover(group.clone(), |d| d.text_color(color(hover_text)))
					.into_any_element(),
			),
			Trailing::Radio(on) => Some(mark(on, true)),
			Trailing::Toggle(on) => Some(mark(on, false)),
		};
		div()
			.id(("nav-menu-item", index))
			.group(group)
			.h(px(32.))
			.px(px(8.))
			.rounded(px(4.))
			.flex()
			.items_center()
			.gap(px(8.))
			.text_size(px(14.))
			.font_weight(FontWeight::MEDIUM)
			.when(enabled, |d| {
				d.cursor_pointer()
					.text_color(color(text))
					.hover(|d| d.bg(color(hover_bg)).text_color(color(hover_text)))
					.on_click(cx.listener(move |this, _, window, cx| {
						match &run {
							Run::Open(page) => this.set_nav_menu_page(*page),
							Run::Act(action) => {
								this.navigation.menu = None;
								action(this, window, cx);
							}
						}
						cx.notify();
					}))
			})
			.when(!enabled, |d| {
				d.text_color(tint(p.muted, 0.6))
					.when_some(hint, |d, hint| d.tooltip(tooltip(hint)))
			})
			.child(
				div()
					.flex_1()
					.min_w_0()
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.child(label),
			)
			.children(trailing)
			.into_any_element()
	}

	fn menu_title(&self, title: &'static str, cx: &mut Context<Self>) -> AnyElement {
		let p = palette();
		div()
			.id("nav-menu-back")
			.h(px(32.))
			.px(px(4.))
			.rounded(px(4.))
			.flex()
			.items_center()
			.gap(px(6.))
			.cursor_pointer()
			.text_size(px(12.))
			.font_weight(FontWeight::SEMIBOLD)
			.text_color(color(p.muted))
			.hover(|d| d.bg(color(p.hover)).text_color(color(p.text_strong)))
			.tooltip(tooltip("Back"))
			.on_click(cx.listener(|this, _, _, cx| {
				this.set_nav_menu_page(Page::Root);
				cx.notify();
			}))
			.child(
				icon(Icon::CaretRight, px(14.), color(p.muted))
					.with_transformation(Transformation::rotate(radians(std::f32::consts::PI))),
			)
			.child(title.to_uppercase())
			.into_any_element()
	}

	/// The open menu, anchored at the pointer and kept inside the window.
	pub(crate) fn render_nav_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
		let p = palette();
		let menu = self.navigation.menu.as_ref()?;
		let mut rows = Vec::new();
		for (index, item) in self
			.menu_items(menu.target, menu.page)
			.into_iter()
			.enumerate()
		{
			rows.push(match item {
				Item::Separator => div()
					.h(px(1.))
					.mx(px(4.))
					.my(px(4.))
					.bg(color(p.border))
					.into_any_element(),
				Item::Title(title) => self.menu_title(title, cx),
				Item::Row(row) => self.menu_row(index, row, cx),
			});
		}
		Some(
			deferred(
				anchored()
					.position(menu.position)
					.offset(point(px(2.), px(2.)))
					.snap_to_window_with_margin(px(8.))
					.child(
						div()
							.id("nav-menu")
							.track_focus(&menu.focus)
							.occlude()
							.w(px(232.))
							.p(px(6.))
							.flex()
							.flex_col()
							.rounded(px(8.))
							.bg(solid(p.chat))
							.border_1()
							.border_color(color(p.border))
							.shadow_lg()
							.font_family(crate::theme::FONT)
							.on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
								let key = event.keystroke.key.as_str();
								let sub = this
									.navigation
									.menu
									.as_ref()
									.is_some_and(|menu| menu.page != Page::Root);
								if key == "escape" {
									this.close_nav_menu(cx);
									cx.stop_propagation();
								} else if sub && matches!(key, "left" | "backspace") {
									this.set_nav_menu_page(Page::Root);
									cx.notify();
									cx.stop_propagation();
								}
							}))
							.on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, _, cx| {
								this.close_nav_menu(cx)
							}))
							.children(rows),
					),
			)
			.with_priority(1)
			.into_any_element(),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::{MUTE_CHOICES, channel_link, demo_channel_outcome, level_choices, mute_until};
	use client_core::channel_actions::{Action, Mute, Outcome};
	use model::Id;

	#[test]
	fn links_use_the_guild_or_the_direct_message_scope() {
		assert_eq!(
			channel_link(Some(Id(10)), Id(21)),
			"https://discord.com/channels/10/21"
		);
		assert_eq!(
			channel_link(None, Id(22)),
			"https://discord.com/channels/@me/22"
		);
	}

	#[test]
	fn mute_choices_match_the_reducer_and_expire_after_their_duration() {
		assert!(
			MUTE_CHOICES
				.iter()
				.all(|(_, mute)| Action::Mute(*mute).valid())
		);
		assert_eq!(MUTE_CHOICES[0], ("For 15 Minutes", Mute::For(900)));
		assert_eq!(MUTE_CHOICES[5], ("Until I Turn It Back On", Mute::Forever));
		assert_eq!(mute_until(Mute::For(3600), 1_000), Some(4_600));
		assert_eq!(mute_until(Mute::Forever, 1_000), None);
		assert_eq!(mute_until(Mute::Unmute, 1_000), None);
		assert_eq!(mute_until(Mute::For(900), i64::MAX), Some(i64::MAX));
	}

	#[test]
	fn levels_are_valid_and_name_the_inherited_default() {
		for in_category in [false, true] {
			let levels = level_choices(in_category);
			assert!(
				levels
					.iter()
					.all(|(level, _)| Action::Notifications(*level).valid())
			);
			assert_eq!(levels[1].1, "Only @mentions");
		}
		assert_eq!(level_choices(true)[3].1, "Use Category Default");
		assert_eq!(level_choices(false)[3].1, "Use Server Default");
	}

	#[test]
	fn offline_outcomes_satisfy_the_reducer_and_mute_a_channel() {
		let mut state = test_support::chat_demo_state();
		assert_eq!(state.guild_channel_muted(Id(20)), Some(false));
		for (action, muted) in [
			(Action::Mute(Mute::For(3600)), true),
			(Action::Mute(Mute::Unmute), false),
			(Action::Mute(Mute::Forever), true),
		] {
			let Some(client_core::Command::ChannelAction {
				guild,
				channel,
				request,
				action,
			}) = state.request_channel_action(Id(20), action)
			else {
				panic!("personal channel writes are allowed offline");
			};
			let outcome = demo_channel_outcome(&action).expect("preference outcome");
			assert!(matches!(outcome, Outcome::Preferences { .. }));
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::ChannelAction(
					client_core::channel_actions::Event::Finished {
						guild,
						channel,
						request,
						result: Ok(outcome),
					},
				),
			});
			assert_eq!(state.guild_channel_muted(Id(20)), Some(muted));
			assert!(!state.channel_action_pending());
		}
		assert!(demo_channel_outcome(&Action::Delete).is_none());
	}
}
