//! Messaging Permissions: spam filtering, server DMs, friend requests and connected games,
//! following `crates/ui/src/messaging_permissions.rs`. Values are shown only after Discord
//! acknowledges them (`State::messaging_permissions`); the offline preview answers in
//! `client_core` without a request.
use super::kit;
use crate::Serein;
use crate::theme::{Icon, color, icon, palette};
use gpui::{prelude::*, *};
use model::{
	Id,
	messaging_permissions::{Change, MAX_GUILDS, Snapshot},
};

/// Page state: the snapshot is requested once each time the page is opened.
#[derive(Default)]
pub struct Messaging {
	pub requested: bool,
	generation: u64,
	/// Server whose DM preferences are shown; `None` is "All servers".
	guild: Option<Id>,
	picking: bool,
}

/// Spam filter radio options: service value, label, detail.
const SPAM: [(u32, &str, Option<&str>); 3] = [
	(3, "Filter all spam", None),
	(2, "Filter messages from non-friends", Some("Recommended")),
	(1, "Don't filter spam", None),
];
/// In-game DM radio options.
const GAME_DMS: [(u32, &str); 3] = [
	(1, "Show all DMs"),
	(2, "Show only DMs from people who also play the game"),
	(3, "Don't show DMs"),
];

/// Friend request source switch: label, `friend_source_flags` bit, change, detail.
type Source = (&'static str, u32, fn(bool) -> Change, Option<&'static str>);
const FRIEND_SOURCES: [Source; 3] = [
	("Everyone", 8, Change::Everyone, None),
	("Friends of friends", 2, Change::FriendsOfFriends, None),
	(
		"Server members",
		4,
		Change::ServerMembers,
		Some("Only from servers where you also allow Direct Messages."),
	),
];

/// The service's "unset" value (0) means non-friends for spam and all for game DMs.
fn spam_selected(current: u32, value: u32) -> bool {
	current == value || (current == 0 && value == 2)
}
fn game_dms_selected(current: u32, value: u32) -> bool {
	current == value || (current == 0 && value == 1)
}

/// The change a server DM switch sends: one server, or every current server plus the default.
fn dm_change(guild: Option<Id>, guilds: &[Id], filter: bool, enabled: bool) -> Change {
	match (guild, filter) {
		(Some(id), false) => Change::AllowGuildDms(id, enabled),
		(Some(id), true) => Change::FilterGuildRequests(id, enabled),
		(None, false) => Change::AllowAllDms {
			guilds: guilds.to_vec(),
			enabled,
		},
		(None, true) => Change::FilterAllRequests {
			guilds: guilds.to_vec(),
			enabled,
		},
	}
}

/// Whether any server differs from the "All servers" values shown.
fn mixed(settings: &Snapshot, guilds: &[Id]) -> bool {
	let (allow, filter) = (settings.allow_dms(None), settings.filter_requests(None));
	guilds.iter().any(|&id| {
		settings.allow_dms(Some(id)) != allow || settings.filter_requests(Some(id)) != filter
	})
}

impl Serein {
	pub(super) fn settings_permissions(
		&mut self,
		_window: &mut Window,
		cx: &mut Context<Self>,
	) -> Div {
		let generation = self.state.generation;
		let nav = &mut self.settings.messaging;
		if nav.generation != generation {
			*nav = Messaging {
				generation,
				..Messaging::default()
			};
		}
		let loaded = &self.state.messaging_permissions;
		if loaded.snapshot.is_none() && !loaded.pending && loaded.error.is_none() {
			nav.requested = false;
		}
		if !nav.requested && !loaded.pending {
			nav.requested = true;
			let command = self.state.request_messaging_permissions();
			self.dispatch(command);
		}
		let guilds = self
			.state
			.guilds
			.iter()
			.map(|guild| (guild.id, guild.name.clone()))
			.collect::<Vec<_>>();
		let nav = &mut self.settings.messaging;
		if nav
			.guild
			.is_some_and(|id| !guilds.iter().any(|(guild, _)| *guild == id))
		{
			nav.guild = None;
		}
		let (guild, picking) = (nav.guild, nav.picking);
		let loaded = &self.state.messaging_permissions;
		let busy = loaded.pending;
		let mut page = div().flex().flex_col().gap_3();
		if busy {
			page = page.child(kit::hint(if loaded.snapshot.is_some() {
				"Saving…"
			} else {
				"Loading your preferences…"
			}));
		}
		if let Some(error) = loaded.error {
			page = page
				.child(kit::notice(kit::Level::Warning, error.label()))
				.when(!busy, |d| {
					d.child(div().flex().child(
						kit::text_action("messaging-retry", "Try again").on_click(cx.listener(
							|this, _, _, cx| {
								this.settings.messaging.requested = false;
								cx.notify();
							},
						)),
					))
				});
		}
		let Some(settings) = loaded.snapshot.clone() else {
			return page;
		};
		let enabled = !busy;

		// Spam filters.
		let spam = SPAM.map(|(value, label, detail)| {
			let selected = spam_selected(settings.spam_filter, value);
			radio(
				("spam-filter", value as usize),
				label,
				detail,
				selected,
				enabled,
				cx,
				Change::SpamFilter(value),
			)
		});
		let spam = kit::card()
			.child(section(
				"Automatically filter suspected spam messages",
				Some(
					"Discord can filter out some messages that contain spam. These messages go to your Spam inbox.",
				),
			))
			.children(spam)
			.when(settings.spam_filter > 3, |d| {
				d.child(kit::hint(
					"Your account uses a custom spam filter setting. Select an option to replace it.",
				))
			});

		// Server DMs.
		let ids = guilds.iter().map(|(id, _)| *id).collect::<Vec<_>>();
		let all = guild.is_none();
		let allow = settings.allow_dms(guild);
		let filter = settings.filter_requests(guild);
		let too_many = all && ids.len() > MAX_GUILDS;
		let server_detail = if all && mixed(&settings, &ids) {
			"Some servers have different preferences. Choose a server to review its settings."
		} else if all {
			"Changes apply to all current servers and set the default for newly joined servers."
		} else {
			"Changes apply to this server only."
		};
		let label = guild
			.and_then(|id| guilds.iter().find(|(guild, _)| *guild == id))
			.map_or("All servers".to_owned(), |(_, name)| name.clone());
		let dm_switch = |id: &'static str,
		                 label: &str,
		                 detail: Option<&str>,
		                 on: bool,
		                 filter: bool,
		                 cx: &mut Context<Self>| {
			let ids = ids.clone();
			kit::switch(
				id,
				label,
				detail,
				on,
				enabled && !too_many,
				cx,
				move |this, on, _, _| {
					this.change_messaging_permissions(dm_change(guild, &ids, filter, on));
				},
			)
		};
		let dms = kit::card()
			.child(kit::row(
				"Server",
				Some(server_detail),
				self.guild_picker(&label, enabled, cx),
			))
			.when(picking && enabled, |d| {
				d.child(guild_options(guild, &guilds, cx))
			})
			.child(kit::divider())
			.child(dm_switch(
				"allow-server-dms",
				"Allow DMs from other server members",
				None,
				allow,
				false,
				cx,
			))
			.child(kit::divider())
			.child(dm_switch(
				"filter-server-requests",
				"Filter messages from server members I may not know",
				Some("Move messages from people you may not know into Message Requests."),
				filter,
				true,
				cx,
			))
			.when(too_many, |d| {
				d.child(kit::hint(
					"There are too many servers to update together. Choose an individual server.",
				))
			});

		// Friend requests.
		let sources = FRIEND_SOURCES.map(|(label, bit, make, detail)| {
			kit::switch(
				("friend-source", bit as usize),
				label,
				detail,
				settings.friend_source_flags & bit != 0,
				enabled,
				cx,
				move |this, on, _, _| this.change_messaging_permissions(make(on)),
			)
		});
		let friends = kit::card()
			.child(section(
				"Allow friend requests from",
				Some("Control who can send you friend requests and how they appear."),
			))
			.children(sources)
			.child(kit::divider())
			.child(kit::switch(
				"personalized-requests",
				"Show personalized messages",
				Some(
					"Show personalized messages on incoming friend requests. If you accept, the message will still appear in your DMs.",
				),
				settings.personalized_requests,
				enabled,
				cx,
				|this, on, _, _| {
					this.change_messaging_permissions(Change::PersonalizedRequests(on))
				},
			));

		// Connected games.
		let game_dms = GAME_DMS.map(|(value, label)| {
			radio(
				("game-dms", value as usize),
				label,
				None,
				game_dms_selected(settings.game_dms, value),
				enabled,
				cx,
				Change::GameDms(value),
			)
		});
		let games = kit::card()
			.child(kit::hint(
				"Settings for games that use Discord to power their social experiences.",
			))
			.child(kit::switch(
				"game-friend-dms",
				"Allow friends from games to send direct messages and invites",
				Some(
					"Let friends from connected games send DMs and invite you to play, even when the game isn't open.",
				),
				settings.game_friend_dms,
				enabled,
				cx,
				|this, on, _, _| this.change_messaging_permissions(Change::GameFriendDms(on)),
			))
			.child(kit::divider())
			.child(section(
				"Show Direct Messages in games",
				Some("Read and respond to DMs directly from in-game chats."),
			))
			.children(game_dms)
			.when(settings.game_dms > 3, |d| {
				d.child(kit::hint(
					"Your account uses a custom in-game DM setting. Select an option to replace it.",
				))
			});

		page.child(kit::group("Spam Filters", spam))
			.child(kit::group("Direct Message (DM) Permissions", dms))
			.child(kit::group("Friend Request Permissions", friends))
			.child(kit::group("Messaging in Connected Games", games))
	}

	fn change_messaging_permissions(&mut self, change: Change) {
		let command = self.state.update_messaging_permissions(change);
		self.dispatch(command);
	}

	/// The server selector button; its options open inline below the row.
	fn guild_picker(&self, label: &str, enabled: bool, cx: &mut Context<Self>) -> Stateful<Div> {
		let p = palette();
		let open = self.settings.messaging.picking && enabled;
		div()
			.id("messaging-guild")
			.w(px(240.))
			.h(px(36.))
			.px_3()
			.rounded(px(6.))
			.bg(color(p.base))
			.border_1()
			.border_color(color(if open { p.accent } else { p.border }))
			.flex()
			.items_center()
			.justify_between()
			.gap_2()
			.when(!enabled, |d| d.opacity(0.5))
			.when(enabled, |d| {
				d.cursor_pointer()
					.focusable()
					.tab_stop(true)
					.on_click(cx.listener(|this, _, _, cx| {
						let nav = &mut this.settings.messaging;
						nav.picking = !nav.picking;
						cx.notify();
					}))
			})
			.child(
				div()
					.flex_1()
					.min_w_0()
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.text_size(px(14.))
					.text_color(color(p.text_strong))
					.child(label.to_owned()),
			)
			.child(icon(Icon::CaretDown, px(14.), color(p.muted)))
	}
}

/// "All servers" and every joined server; picking one closes the list.
fn guild_options(current: Option<Id>, guilds: &[(Id, String)], cx: &mut Context<Serein>) -> Div {
	let p = palette();
	let options = std::iter::once((None, "All servers".to_owned()))
		.chain(guilds.iter().map(|(id, name)| (Some(*id), name.clone())))
		.enumerate()
		.map(|(index, (guild, name))| {
			let selected = guild == current;
			div()
				.id(("messaging-guild-option", index))
				.h(px(32.))
				.px_2()
				.flex_none()
				.rounded(px(4.))
				.flex()
				.items_center()
				.justify_between()
				.cursor_pointer()
				.text_size(px(14.))
				.text_color(color(if selected { p.text_strong } else { p.text }))
				.when(selected, |d| d.bg(color(p.selected)))
				.hover(|d| d.bg(color(p.hover)))
				.on_click(cx.listener(move |this, _, _, cx| {
					let nav = &mut this.settings.messaging;
					nav.guild = guild;
					nav.picking = false;
					cx.notify();
				}))
				.child(
					div()
						.min_w_0()
						.overflow_hidden()
						.whitespace_nowrap()
						.text_ellipsis()
						.child(name),
				)
				.when(selected, |d| {
					d.child(icon(Icon::Check, px(14.), color(p.accent)))
				})
				.into_any_element()
		})
		.collect::<Vec<_>>();
	div().w_full().flex().justify_end().child(
		div()
			.id("messaging-guild-options")
			.w(px(240.))
			.max_h(px(280.))
			.overflow_y_scroll()
			.p_1()
			.rounded(px(6.))
			.bg(color(p.base))
			.border_1()
			.border_color(color(p.border))
			.flex()
			.flex_col()
			.children(options),
	)
}

/// Title and help text that introduce a group of options, like `design::section`.
fn section(title: &str, help: Option<&str>) -> Div {
	let p = palette();
	div()
		.pb(px(6.))
		.flex()
		.flex_col()
		.gap(px(2.))
		.child(
			div()
				.text_size(px(16.))
				.font_weight(FontWeight::MEDIUM)
				.text_color(color(p.text_strong))
				.child(title.to_owned()),
		)
		.children(help.map(|help| {
			div()
				.text_size(px(13.))
				.line_height(px(18.))
				.text_color(color(p.muted))
				.child(help.to_owned())
		}))
}

/// Radio option like `design::radio_row`: ring marker, title and optional detail.
fn radio(
	id: impl Into<ElementId>,
	label: &str,
	detail: Option<&str>,
	selected: bool,
	enabled: bool,
	cx: &mut Context<Serein>,
	change: Change,
) -> Stateful<Div> {
	let p = palette();
	let ring = if selected { p.accent } else { p.muted };
	div()
		.id(id)
		.w_full()
		.mx(px(-8.))
		.px_2()
		.py_2()
		.rounded(px(8.))
		.flex()
		.items_start()
		.gap_3()
		.when(!enabled, |d| d.opacity(0.5))
		.when(enabled, |d| {
			d.cursor_pointer()
				.focusable()
				.tab_stop(true)
				.hover(|d| d.bg(color(p.hover)))
				.when(!selected, |d| {
					d.on_click(cx.listener(move |this, _, _, cx| {
						this.change_messaging_permissions(change.clone());
						cx.notify();
					}))
				})
		})
		.child(
			div()
				.mt(px(2.))
				.size(px(18.))
				.flex_none()
				.rounded_full()
				.border_2()
				.border_color(color(ring))
				.flex()
				.items_center()
				.justify_center()
				.when(selected, |d| {
					d.child(div().size(px(9.)).rounded_full().bg(color(ring)))
				}),
		)
		.child(
			div()
				.flex_1()
				.min_w_0()
				.flex()
				.flex_col()
				.gap(px(2.))
				.child(
					div()
						.text_size(px(15.))
						.line_height(px(22.))
						.font_weight(FontWeight::MEDIUM)
						.text_color(color(p.text_strong))
						.child(label.to_owned()),
				)
				.children(detail.map(|detail| {
					div()
						.text_size(px(13.))
						.line_height(px(16.))
						.text_color(color(p.muted))
						.child(detail.to_owned())
				})),
		)
}

#[cfg(test)]
mod tests {
	use super::{dm_change, game_dms_selected, mixed, spam_selected};
	use model::{
		Id,
		messaging_permissions::{Change, Snapshot},
	};

	#[test]
	fn unset_service_values_select_the_defaults() {
		assert!(spam_selected(0, 2) && !spam_selected(0, 3));
		assert!(spam_selected(3, 3) && !spam_selected(3, 2));
		assert!(game_dms_selected(0, 1) && !game_dms_selected(0, 2));
		// Custom values select nothing; the page explains that instead.
		assert!((1..=3).all(|value| !spam_selected(7, value)));
	}

	#[test]
	fn server_switches_target_one_server_or_all() {
		let guilds = [Id(1), Id(2)];
		assert_eq!(
			dm_change(Some(Id(2)), &guilds, false, false),
			Change::AllowGuildDms(Id(2), false)
		);
		assert_eq!(
			dm_change(None, &guilds, true, true),
			Change::FilterAllRequests {
				guilds: guilds.to_vec(),
				enabled: true
			}
		);
		let mut settings = Snapshot::default();
		assert!(!mixed(&settings, &guilds));
		settings.restricted_guilds = vec![Id(2)];
		assert!(mixed(&settings, &guilds));
	}

	#[test]
	fn demo_answers_requests_and_changes_without_a_command() {
		let mut state = test_support::demo_state();
		assert!(state.request_messaging_permissions().is_none());
		let snapshot = state
			.messaging_permissions
			.snapshot
			.clone()
			.expect("loaded");
		assert!(snapshot.game_friend_dms);
		assert!(
			state
				.update_messaging_permissions(Change::GameFriendDms(false))
				.is_none()
		);
		let updated = state.messaging_permissions.snapshot.as_ref().unwrap();
		assert!(!updated.game_friend_dms && !state.messaging_permissions.pending);
	}
}
