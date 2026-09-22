//! Friends overview using the retained relationship/presence state and existing user actions.
use crate::{
	MessagingUi, design,
	icons::{self, Icon},
	profiles, user_menu,
};
use client_core::{Command, State};
use egui::{RichText, vec2};

#[derive(Default)]
pub(super) struct Friends {
	pub(super) presence_warning_dismissed: Option<u64>,
	tab: Tab,
	query: String,
	username: String,
	outgoing: bool,
	list_key: Option<ListKey>,
	list_query: String,
	list: Box<[model::Id]>,
	#[cfg(feature = "demo")]
	focus_search: bool,
}
#[derive(PartialEq, Eq)]
struct ListKey {
	generation: u64,
	relationships: u64,
	online: Option<(bool, u64)>,
	tab: Tab,
}
#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum Tab {
	#[default]
	Online,
	All,
	Pending,
	Restricted,
	Add,
}
impl Friends {
	fn matches(&self, state: &State, user: &model::User, query: &str) -> bool {
		if self.tab == Tab::Online {
			let (status, _, _) = profiles::presence(state, user.id, None);
			if !matches!(status, Some("online" | "idle" | "dnd")) {
				return false;
			}
		}
		let username = if self.tab == Tab::Restricted {
			state
				.restricted_user(user.id)
				.map(|(_, name, _)| name.as_str())
		} else {
			state.friend_username(user.id)
		};
		query.is_empty()
			|| user.name.to_lowercase().contains(query)
			|| state.user_display_name(user).to_lowercase().contains(query)
			|| username.is_some_and(|name| name.to_lowercase().contains(query))
	}
	fn sync_list(&mut self, state: &State) -> bool {
		let key = ListKey {
			generation: state.generation,
			relationships: state.relationship_view(),
			online: (self.tab == Tab::Online)
				.then(|| (state.gateway_connected, state.direct_presence_epoch())),
			tab: self.tab,
		};
		if self.list_key.as_ref() == Some(&key) && self.list_query == self.query {
			return false;
		}
		let query = self.query.trim().to_lowercase();
		// ponytail: cold Online builds scan bounded presences; index only if rebuilds warrant it.
		let mut friends: Vec<_> = if self.tab == Tab::Restricted {
			state
				.restricted_users()
				.map(|(user, _, _)| user)
				.filter(|user| self.matches(state, user, &query))
				.collect()
		} else {
			state
				.friends()
				.filter(|user| self.matches(state, user, &query))
				.collect()
		};
		friends.sort_unstable_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
		// At most MAX_RELATIONSHIPS fixed-size IDs (32,000 bytes); no profiles retained.
		self.list = friends.into_iter().map(|user| user.id).collect();
		self.list_query = self.query.clone();
		self.list_key = Some(key);
		true
	}
}
impl MessagingUi {
	#[cfg(feature = "demo")]
	pub fn prepare_friends_sample(&mut self) {
		self.friends.tab = Tab::Online;
		self.friends.query.clear();
		self.friends.focus_search = true;
	}
	#[cfg(feature = "demo")]
	pub fn friends_sample_focused(&self, ctx: &egui::Context) -> bool {
		ctx.memory(|memory| memory.has_focus(egui::Id::unique("friends-search")))
	}
	fn add_friend_page(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Frame::new().inner_margin(24).show(ui, |ui| {
			ui.label(design::semibold(ui, "Add Friend", 24.0));
			ui.label("You can add friends with their Discord username.");
			ui.add_space(18.0);
			let label = ui.label("Username");
			let input = ui
				.add_sized(
					[ui.available_width(), 48.0],
					egui::TextEdit::singleline(&mut self.friends.username)
						.hint_text("Enter a username")
						.char_limit(33)
						.align(egui::Align2::LEFT_CENTER)
						.frame(
							egui::Frame::new()
								.fill(colors.base)
								.stroke(egui::Stroke::new(1.0, colors.border))
								.corner_radius(8)
								.inner_margin(egui::Margin::symmetric(12, 8)),
						),
				)
				.labelled_by(label.id);
			if input.has_focus() {
				ui.painter().rect_stroke(
					input.rect,
					8,
					egui::Stroke::new(2.0, colors.accent),
					egui::StrokeKind::Inside,
				);
			}
			ui.add_space(12.0);
			let busy = state.user_action_pending();
			let enabled = !busy
				&& !self.friends.username.trim().is_empty()
				&& (state.demo
					|| (state.gateway_connected
						&& state.auth == client_core::auth::AuthState::Authenticated));
			if ui
				.add_enabled(
					enabled,
					egui::Button::new(
						RichText::new(if busy {
							"Sending…"
						} else {
							"Send Friend Request"
						})
						.color(colors.accent_text),
					)
					.fill(colors.accent)
					.min_size(vec2(180.0, 40.0)),
				)
				.clicked() && let Some(command) = state.add_friend(&self.friends.username)
			{
				commands.push(command);
			}
			if state.demo {
				ui.colored_label(colors.muted, "Offline demo · actions are simulated.");
			} else if !state.gateway_connected {
				ui.label("Reconnect before sending a friend request.");
			}
			ui.add_space(8.0);
			ui.colored_label(
				colors.muted,
				"Personalized request notes are not supported yet.",
			);
			ui.add_space(24.0);
			ui.separator();
			ui.add_space(16.0);
			ui.label(design::semibold(ui, "Other Places to Make Friends", 20.0));
			ui.label("Don't have a username? Discover public communities in Discord.");
			ui.hyperlink_to(
				"Explore Discoverable Servers ↗",
				"https://discord.com/servers",
			);
		});
	}
	fn friend_requests_page(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let mut resolve = None;
		egui::Frame::new().inner_margin(24).show(ui, |ui| {
			ui.horizontal_wrapped(|ui| {
				for outgoing in [false, true] {
					let count = state
						.pending_friends()
						.filter(|(_, _, incoming)| *incoming != outgoing)
						.count();
					if ui
						.selectable_label(
							self.friends.outgoing == outgoing,
							format!(
								"{} — {count}",
								if outgoing { "Outgoing" } else { "Incoming" }
							),
						)
						.clicked()
					{
						self.friends.outgoing = outgoing;
					}
				}
			});
			ui.add_space(12.0);
			ui.add_sized(
				[ui.available_width(), 40.0],
				egui::TextEdit::singleline(&mut self.friends.query)
					.hint_text("Search requests")
					.char_limit(128)
					.align(egui::Align2::LEFT_CENTER),
			);
			ui.add_space(16.0);
			let query = self.friends.query.trim().to_lowercase();
			let mut rows: Vec<_> = state
				.pending_friends()
				.filter(|(user, name, incoming)| {
					*incoming != self.friends.outgoing
						&& (user.name.to_lowercase().contains(&query)
							|| name.to_lowercase().contains(&query))
				})
				.collect();
			rows.sort_unstable_by(|a, b| a.0.name.cmp(&b.0.name).then(a.0.id.cmp(&b.0.id)));
			if rows.is_empty() {
				ui.label(if !state.friend_requests_known() {
					"Friend requests are not available yet."
				} else if !query.is_empty() {
					"No requests match your search."
				} else if self.friends.outgoing {
					"No outgoing friend requests."
				} else {
					"No incoming friend requests."
				});
			}
			ui.spacing_mut().item_spacing.y = 0.0;
			self.scroll
				.attach(
					ui,
					"friend-requests",
					egui::ScrollArea::vertical().auto_shrink([false, false]),
				)
				.show_rows(ui, 72.0, rows.len(), |ui, range| {
					for (user, name, incoming) in &rows[range] {
						ui.push_id(user.id.0, |ui| {
							let (rect, _) = ui.allocate_exact_size(
								vec2(ui.available_width(), 72.0),
								egui::Sense::hover(),
							);
							ui.painter().hline(
								rect.x_range(),
								rect.top(),
								egui::Stroke::new(1.0, colors.border),
							);
							let mut row = ui.new_child(
								egui::UiBuilder::new()
									.max_rect(rect.shrink2(vec2(0.0, 12.0)))
									.layout(egui::Layout::left_to_right(egui::Align::Center)),
							);
							// The friends surfaces keep their own row actions; the profile
							// stays behind the context menu instead of every click.
							self.avatars.show_plain(&mut row, user, 40.0, state.demo);
							let width = (row.available_width() - 88.0).max(1.0);
							row.allocate_ui_with_layout(
								vec2(width, 44.0),
								egui::Layout::top_down(egui::Align::Min),
								|ui| {
									ui.add(
										egui::Label::new(design::semibold(
											ui,
											state.user_display_name(user),
											16.0,
										))
										.truncate(),
									);
									ui.add(
										egui::Label::new(RichText::new(name).color(colors.muted))
											.truncate(),
									);
								},
							);
							row.add_enabled_ui(
								!state.user_action_pending()
									&& (state.demo || state.gateway_connected),
								|ui| {
									if *incoming
										&& icons::button(ui, Icon::Check, 32.0, "Accept request")
											.clicked()
									{
										resolve = Some((user.id, true));
									}
									if icons::button(
										ui,
										Icon::Close,
										32.0,
										if *incoming {
											"Decline request"
										} else {
											"Cancel request"
										},
									)
									.clicked()
									{
										resolve = Some((user.id, false));
									}
								},
							);
						});
					}
				});
		});
		if let Some((user, accept)) = resolve
			&& let Some(command) = state.resolve_friend_request(user, accept)
		{
			commands.push(command);
		}
	}
	pub(super) fn friends_page(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		egui::Frame::new()
			.inner_margin(egui::Margin::symmetric(24, 8))
			.show(ui, |ui| {
				ui.horizontal_wrapped(|ui| {
					ui.set_min_height(32.0);
					ui.spacing_mut().item_spacing.x = 16.0;
					icons::inline(ui, Icon::People, 22.0, colors.muted);
					ui.label(design::semibold(ui, "Friends", 16.0));
					ui.separator();
					for (tab, title) in [
						(Tab::Online, "Online"),
						(Tab::All, "All"),
						(Tab::Pending, "Pending"),
						(Tab::Restricted, "Blocked & Ignored"),
						(Tab::Add, "Add Friend"),
					] {
						if ui
							.add(
								egui::Button::new(RichText::new(title).color(if tab == Tab::Add {
									colors.accent_text
								} else {
									colors.text
								}))
								.selected(self.friends.tab == tab)
								.fill(if tab == Tab::Add {
									colors.accent
								} else if self.friends.tab == tab {
									colors.raised
								} else {
									egui::Color32::TRANSPARENT
								}),
							)
							.clicked()
						{
							self.friends.tab = tab;
							self.friends.query.clear();
						}
					}
				});
			});
		ui.separator();
		if matches!(self.friends.tab, Tab::Online | Tab::All)
			&& state.gateway_connected
			&& state.startup_warnings.presence
			&& self.friends.presence_warning_dismissed != Some(state.generation)
		{
			egui::Frame::new()
				.inner_margin(egui::Margin::symmetric(24, 8))
				.show(ui, |ui| {
					ui.horizontal_top(|ui| {
						icons::inline(ui, Icon::ShieldWarning, 18.0, colors.warning);
						ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
							if icons::button(ui, Icon::Close, 22.0, "Dismiss friend status warning")
								.clicked()
							{
								self.friends.presence_warning_dismissed = Some(state.generation);
							}
							ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
								ui.add(egui::Label::new("Some friends’ online status and activity couldn’t be loaded. The Online list may be incomplete.").wrap());
							});
						});
					});
				});
		}
		if self.friends.tab == Tab::Add {
			self.add_friend_page(ui, state, commands);
			return;
		}
		if self.friends.tab == Tab::Pending {
			self.friend_requests_page(ui, state, commands);
			return;
		}
		let mut selected = None;
		egui::Frame::new()
			.inner_margin(egui::Margin::symmetric(24, 24))
			.show(ui, |ui| {
				egui::Frame::new()
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(8)
					.inner_margin(egui::Margin::symmetric(12, 8))
					.show(ui, |ui| {
						ui.horizontal(|ui| {
							icons::inline(ui, Icon::Search, 18.0, colors.muted);
							let search = ui.add(
								egui::TextEdit::singleline(&mut self.friends.query)
									.id(egui::Id::unique("friends-search"))
									.hint_text("Search")
									.char_limit(128)
									.frame(egui::Frame::NONE)
									.desired_width(ui.available_width()),
							);
							#[cfg(feature = "demo")]
							if std::mem::take(&mut self.friends.focus_search) {
								search.request_focus();
							}
							#[cfg(not(feature = "demo"))]
							let _ = search;
						});
					});
				ui.add_space(20.0);
				self.friends.sync_list(state);
				ui.label(
					RichText::new(format!(
						"{} \u{2014} {}",
						match self.friends.tab {
							Tab::All => "All friends",
							Tab::Restricted => "Blocked & ignored",
							_ => "Online",
						},
						self.friends.list.len()
					))
					.size(13.0)
					.color(colors.muted),
				);
				ui.add_space(12.0);
				ui.spacing_mut().item_spacing.y = 0.0;
				if self.friends.list.is_empty() {
					ui.add_space(20.0);
					ui.label(
						RichText::new(
							if self.friends.tab == Tab::Restricted
								&& !state.restricted_users_known()
							{
								"Blocked and ignored users are not available yet."
							} else if self.friends.tab != Tab::Restricted && !state.friends_known()
							{
								"Friends are not available yet."
							} else if !self.friends.query.trim().is_empty() {
								if self.friends.tab == Tab::Restricted {
									"No blocked or ignored users match your search."
								} else {
									"No friends match your search."
								}
							} else if self.friends.tab == Tab::Restricted {
								"No blocked or ignored users."
							} else if self.friends.tab == Tab::All {
								"No friends yet."
							} else {
								"No friends are currently online."
							},
						)
						.color(colors.muted),
					);
					return;
				}
				self.scroll
					.attach(
						ui,
						if self.friends.tab == Tab::Restricted {
							"restricted-users"
						} else {
							"friends-list"
						},
						egui::ScrollArea::vertical().auto_shrink([false, false]),
					)
					.show_rows(ui, 64.0, self.friends.list.len(), |ui, range| {
						for index in range {
							let restricted = (self.friends.tab == Tab::Restricted)
								.then(|| state.restricted_user(self.friends.list[index]))
								.flatten();
							let user = restricted
								.map(|(user, _, _)| user)
								.or_else(|| state.friend(self.friends.list[index]));
							let Some(user) = user else {
								continue;
							};
							ui.push_id(user.id.0, |ui| {
								let (status, custom, activities) = if restricted.is_some() {
									(None, None, &[][..])
								} else {
									profiles::presence(state, user.id, None)
								};
								let (rect, response) = ui.allocate_exact_size(
									vec2(ui.available_width(), 64.0),
									egui::Sense::click(),
								);
								ui.painter().hline(
									rect.x_range(),
									rect.top(),
									egui::Stroke::new(1.0, colors.border),
								);
								if response.hovered() || response.has_focus() {
									ui.painter().rect_filled(
										rect.shrink(1.0),
										6,
										design::row_highlight(ui, colors.hover, 1.0),
									);
								}
								response.widget_info(|| {
									egui::WidgetInfo::labeled(egui::Role::Button, true, &user.name)
								});
								user_menu::show(
									&response,
									state,
									user,
									&mut self.profile,
									&mut self.user_action,
								);
								let mut avatar_ui = ui.new_child(egui::UiBuilder::new().max_rect(
									egui::Rect::from_min_size(
										rect.min + vec2(0.0, 12.0),
										vec2(40.0, 40.0),
									),
								));
								let avatar =
									self.avatars
										.show_plain(&mut avatar_ui, user, 40.0, state.demo);
								if let Some(status) = status {
									design::presence_dot(
										ui,
										avatar.rect,
										profiles::presence_color(status),
										colors.chat,
									);
								}
								let mut text = ui.new_child(egui::UiBuilder::new().max_rect(
									egui::Rect::from_min_max(
										rect.min + vec2(52.0, 12.0),
										rect.max - vec2(100.0, 8.0),
									),
								));
								text.spacing_mut().item_spacing.y = 1.0;
								text.add(
									egui::Label::new(design::semibold(
										ui,
										state.user_display_name(user),
										16.0,
									))
									.truncate(),
								);
								let subtitle = restricted
									.map(|(_, _, ignored)| {
										if *ignored { "Ignored" } else { "Blocked" }.into()
									})
									.or_else(|| profiles::subtitle(custom, activities))
									.unwrap_or_else(|| {
										status
											.map_or(
												"Presence unavailable",
												profiles::presence_label,
											)
											.into()
									});
								text.horizontal(|ui| {
									if let Some(activity) = activities.first() {
										icons::inline(
											ui,
											if activity.kind == 2 {
												Icon::Spotify
											} else {
												Icon::GameController
											},
											14.0,
											colors.positive,
										);
									}
									ui.add(
										egui::Label::new(
											RichText::new(&subtitle).size(13.0).color(colors.muted),
										)
										.truncate(),
									)
									.on_hover_text(&subtitle);
								});
								let dm = state.channels.iter().find(|c| {
									c.guild.is_none()
										&& c.kind == 1 && c.recipients.iter().any(|u| u.id == user.id)
								});
								let mut actions = ui.new_child(
									egui::UiBuilder::new()
										.max_rect(egui::Rect::from_min_size(
											rect.right_center() - vec2(88.0, 18.0),
											vec2(88.0, 36.0),
										))
										.layout(egui::Layout::left_to_right(egui::Align::Center)),
								);
								actions.spacing_mut().item_spacing.x = 8.0;
								if restricted.is_none() {
									let message = actions
										.add_enabled_ui(dm.is_some(), |ui| {
											icons::button(ui, Icon::Threads, 36.0, "Message")
										})
										.inner;
									if message
										.on_disabled_hover_text(
											"No open direct message with this friend",
										)
										.clicked()
									{
										selected = dm.map(|c| c.id);
									}
								}
								let more = icons::button(&mut actions, Icon::More, 36.0, "More");
								egui::Popup::menu(&more).show(|ui| {
									user_menu::contents(
										ui,
										state,
										user,
										&mut self.profile,
										&mut self.user_action,
										None,
									)
								});
							});
						}
					});
			});
		if let Some(channel) = selected
			&& let Some(command) = state.select(channel)
		{
			commands.push(command);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Envelope, Event, user_actions::Event as Relationship};
	use model::{Id, Patch};

	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}

	// Keep the pre-cache selection algorithm as a behavioral oracle.
	fn uncached(friends: &Friends, state: &State) -> Vec<Id> {
		let query = friends.query.trim().to_lowercase();
		let filter = |user: &&model::User| {
			let (status, _, _) = profiles::presence(state, user.id, None);
			(friends.tab != Tab::Online || matches!(status, Some("online" | "idle" | "dnd")))
				&& (user.name.to_lowercase().contains(&query)
					|| state
						.user_display_name(user)
						.to_lowercase()
						.contains(&query)
					|| if friends.tab == Tab::Restricted {
						state
							.restricted_user(user.id)
							.map(|(_, name, _)| name.as_str())
					} else {
						state.friend_username(user.id)
					}
					.is_some_and(|name| name.to_lowercase().contains(&query)))
		};
		let mut rows: Vec<_> = if friends.tab == Tab::Restricted {
			state
				.restricted_users()
				.map(|(user, _, _)| user)
				.filter(filter)
				.collect()
		} else {
			state.friends().filter(filter).collect()
		};
		rows.sort_unstable_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
		rows.into_iter().map(|user| user.id).collect()
	}

	fn check_cache(friends: &mut Friends, state: &State) {
		friends.sync_list(state);
		assert_eq!(&*friends.list, uncached(friends, state));
		assert!(
			!friends.sync_list(state),
			"unchanged paint rebuilt the list"
		);
	}

	#[test]
	fn friends_cache_matches_uncached_across_relationship_and_session_changes() {
		let mut state = test_support::friends_demo_state();
		let mut friends = Friends::default();
		check_cache(&mut friends, &state);
		for _ in 0..100 {
			assert!(!friends.sync_list(&state));
		}
		for event in [Event::Disconnected, Event::Resumed] {
			apply(&mut state, event);
			assert!(friends.sync_list(&state));
			check_cache(&mut friends, &state);
		}
		for tab in [Tab::All, Tab::Online, Tab::Restricted] {
			friends.tab = tab;
			for query in ["", "robin.synthetic", " CASEY ", "no-match"] {
				friends.query = query.into();
				check_cache(&mut friends, &state);
			}
		}
		friends.tab = Tab::All;
		friends.query = "renamed".into();
		check_cache(&mut friends, &state);
		let mut user = state.friend(Id(1001)).unwrap().clone();
		user.name = "Renamed".into();
		for event in [
			Relationship::FriendProfile((user.clone(), "new.username".into())),
			Relationship::Nickname {
				user: user.id,
				text: "Other name".into(),
			},
			Relationship::Nicknames(vec![(user.id, "Renamed nickname".into())]),
			Relationship::Relationship {
				user: user.id,
				blocked: true,
			},
			Relationship::Relationship {
				user: user.id,
				blocked: false,
			},
			Relationship::Friend {
				user: user.id,
				friend: true,
				profile: Some((user.clone(), "new.username".into())),
			},
			Relationship::Friend {
				user: user.id,
				friend: false,
				profile: None,
			},
			Relationship::Friends(Some(vec![(user, "new.username".into())])),
			Relationship::Relationships(None),
			Relationship::Relationships(Some(vec![(Id(1001), false)])),
			Relationship::Friends(None),
		] {
			apply(&mut state, Event::UserAction(event));
			check_cache(&mut friends, &state);
		}
		state.logout();
		check_cache(&mut friends, &state);
		assert!(friends.list.is_empty());
	}

	#[test]
	fn friends_cache_replaces_ready_rows_and_rolls_back_optimistic_blocks() {
		for tab in [Tab::Online, Tab::All] {
			let mut state = test_support::friends_demo_state();
			let mut friends = Friends {
				tab,
				..Default::default()
			};
			check_cache(&mut friends, &state);
			let previous = friends.list.clone();
			let replacement: Vec<_> = state
				.friends()
				.map(|user| {
					let mut replacement = user.clone();
					replacement.id = Id(user.id.0 + 10_000);
					(
						replacement,
						state.friend_username(user.id).unwrap().to_owned(),
					)
				})
				.collect();
			let target = replacement[0].0.id;
			let presence = replacement
				.iter()
				.enumerate()
				.map(|(index, (user, _))| client_core::presence::Update {
					user: user.id,
					status: Patch::Value(if index < 7 { "online" } else { "offline" }.into()),
					custom_status: Patch::Null,
					activities: Patch::Null,
				})
				.collect();
			let owner = state.user.clone().unwrap();
			let generation = state.generation;
			apply(
				&mut state,
				Event::Ready {
					permissions: Default::default(),
					user: owner,
					guilds: vec![],
					channels: vec![],
				},
			);
			apply(
				&mut state,
				Event::UserAction(Relationship::Relationships(Some(vec![]))),
			);
			apply(
				&mut state,
				Event::UserAction(Relationship::Friends(Some(replacement))),
			);
			apply(&mut state, Event::DirectPresence(presence));
			// Keep the old UI cache through READY and repopulation: equal row counts are not a key.
			assert_eq!(state.generation, generation);
			assert!(friends.sync_list(&state));
			assert_eq!(friends.list.len(), previous.len());
			assert!(friends.list.iter().all(|id| !previous.contains(id)));
			check_cache(&mut friends, &state);
			let restored = friends.list.clone();
			let revision = state.revision;
			let command = state.set_user_blocked(target, true).unwrap();
			assert_eq!(state.revision, revision);
			assert!(friends.sync_list(&state));
			assert_eq!(friends.list.len() + 1, restored.len());
			assert!(!friends.list.contains(&target));
			check_cache(&mut friends, &state);
			state.command_rejected(command);
			assert_eq!(state.revision, revision);
			assert!(friends.sync_list(&state));
			assert_eq!(friends.list, restored);
			check_cache(&mut friends, &state);
		}
	}

	#[test]
	fn friends_cache_tracks_membership_but_paints_activity_without_rebuilding() {
		let mut state = test_support::friends_demo_state();
		let mut friends = Friends::default();
		check_cache(&mut friends, &state);
		for status in [
			Patch::Absent,
			Patch::Value("idle".into()),
			Patch::Value("dnd".into()),
		] {
			apply(
				&mut state,
				Event::DirectPresence(vec![client_core::presence::Update {
					user: Id(1001),
					status,
					custom_status: Patch::Null,
					activities: Patch::Value(vec![model::RichActivity {
						kind: 0,
						name: "Synthetic game".into(),
						details: None,
						state: None,
						image: None,
						small_image: None,
						ends_at: None,
						started_at: None,
					}]),
				}]),
			);
			assert!(!friends.sync_list(&state));
			let (_, custom, activities) = profiles::presence(&state, Id(1001), None);
			assert_eq!(
				profiles::subtitle(custom, activities).as_deref(),
				Some("Playing Synthetic game")
			);
			check_cache(&mut friends, &state);
		}
		for status in [
			Patch::Null,
			Patch::Value("online".into()),
			Patch::Value("offline".into()),
		] {
			apply(
				&mut state,
				Event::DirectPresence(vec![client_core::presence::Update {
					user: Id(1001),
					status,
					custom_status: Patch::Absent,
					activities: Patch::Absent,
				}]),
			);
			assert!(friends.sync_list(&state));
			check_cache(&mut friends, &state);
		}
		friends.tab = Tab::All;
		check_cache(&mut friends, &state);
		apply(&mut state, Event::Disconnected);
		assert!(!friends.sync_list(&state));
		apply(&mut state, Event::Resumed);
		assert!(!friends.sync_list(&state));
		state.apply(Envelope {
			generation: state.generation + 1,
			event: Event::UserAction(Relationship::Friends(None)),
		});
		assert!(!friends.sync_list(&state));
		apply(&mut state, Event::Resync);
		check_cache(&mut friends, &state);
	}

	fn labels(shape: &egui::Shape, out: &mut Vec<String>) {
		match shape {
			egui::Shape::Text(text) => out.push(text.galley.job.text.clone()),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| labels(shape, out)),
			_ => {}
		}
	}
	#[test]
	fn friends_filters_search_and_rows_fit_both_themes() {
		for (width, theme) in [(320., egui::Theme::Dark), (1000., egui::Theme::Light)] {
			let ctx = egui::Context::default();
			design::apply(&ctx);
			ctx.set_theme(theme);
			let mut state = test_support::friends_demo_state();
			let mut view = MessagingUi::default();
			let render = |view: &mut MessagingUi, state: &mut State| {
				let mut commands = Vec::new();
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							vec2(width, 760.),
						)),
						..Default::default()
					},
					|ui| {
						view.friends_page(ui, state, &mut commands);
						assert!(ui.min_rect().width() <= width, "friends overflow");
					},
				);
				assert!(
					commands.is_empty(),
					"rendering must not send friend actions"
				);
				let mut text = Vec::new();
				for shape in &output.shapes {
					labels(&shape.shape, &mut text);
				}
				output.drop_without_applying_deltas();
				text
			};
			let text = render(&mut view, &mut state);
			assert!(
				text.iter()
					.any(|s| s.starts_with("Online") && s.ends_with('7')),
				"{text:?}"
			);
			assert!(text.iter().any(|s| s == "Robin"));
			assert!(!text.iter().any(|s| s == "Parker"));
			apply(
				&mut state,
				Event::DirectPresence(vec![client_core::presence::Update {
					user: Id(1001),
					status: Patch::Absent,
					custom_status: Patch::Null,
					activities: Patch::Value(vec![model::RichActivity {
						kind: 0,
						name: "Synthetic game".into(),
						details: None,
						state: None,
						image: None,
						small_image: None,
						ends_at: None,
						started_at: None,
					}]),
				}]),
			);
			assert!(!view.friends.sync_list(&state));
			assert!(
				render(&mut view, &mut state)
					.iter()
					.any(|s| s == "Playing Synthetic game")
			);
			view.friends.tab = Tab::All;
			let text = render(&mut view, &mut state);
			assert!(
				text.iter()
					.any(|s| s.starts_with("All friends") && s.ends_with("16"))
			);
			view.friends.query = "ROBIN.SYNTHETIC".into();
			let text = render(&mut view, &mut state);
			assert!(text.iter().any(|s| s == "Robin"));
			assert!(!text.iter().any(|s| s == "Casey"));
			view.friends.query = "no-match".into();
			assert!(
				render(&mut view, &mut state)
					.iter()
					.any(|s| s == "No friends match your search.")
			);
			view.friends.tab = Tab::Pending;
			view.friends.query.clear();
			let text = render(&mut view, &mut state);
			assert!(text.iter().any(|s| s == "Avery"));
			assert!(!text.iter().any(|s| s == "Morgan"));
			view.friends.outgoing = true;
			let text = render(&mut view, &mut state);
			assert!(text.iter().any(|s| s == "Morgan"));
			assert!(!text.iter().any(|s| s == "Avery"));
			view.friends.tab = Tab::Restricted;
			view.friends.query.clear();
			let text = render(&mut view, &mut state);
			assert!(text.iter().any(|s| s == "Blocked Example"));
			assert!(text.iter().any(|s| s == "Blocked"));
			assert!(text.iter().any(|s| s == "Ignored Example"));
			assert!(text.iter().any(|s| s == "Ignored"));
			view.friends.query = "ignored.synthetic".into();
			let text = render(&mut view, &mut state);
			assert!(!text.iter().any(|s| s == "Blocked Example"));
			assert!(text.iter().any(|s| s == "Ignored Example"));
			view.friends.query = "no-match".into();
			assert!(
				render(&mut view, &mut state)
					.iter()
					.any(|s| s == "No blocked or ignored users match your search.")
			);
			view.friends.tab = Tab::Add;
			assert!(
				render(&mut view, &mut state)
					.iter()
					.any(|s| s == "Send Friend Request")
			);
		}
	}
}
