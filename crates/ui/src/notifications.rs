use crate::{MessagingUi, design};
use client_core::{Command, State};
use egui::{Align2, Color32, FontId};
use model::Id;

/// Fixed width of the server rail column.
pub(super) const RAIL_WIDTH: f32 = 68.0;

#[derive(Default)]
pub(super) struct RailCache {
	key: Option<(u64, u64, bool, Option<Id>)>,
	// Fixed-size records only: at most MAX_NAV * size_of::<(Id, (bool, u32))>() bytes
	// for badges and 15 * size_of::<Id>() bytes for direct-message rows.
	guild_badges: Box<[(Id, (bool, u32))]>,
	direct: Box<[Id]>,
}
impl RailCache {
	fn sync(&mut self, state: &State) -> bool {
		let call = direct_call(state);
		let key = (
			state.generation,
			state.rail_revision(),
			state.gateway_connected,
			call,
		);
		if self.key == Some(key) {
			return false;
		}
		let mut badges = std::collections::BTreeMap::<Id, (bool, u32)>::new();
		for channel in state.channels.iter().take(client_core::MAX_NAV) {
			if let Some(guild) = channel.guild {
				let entry = badges.entry(guild).or_default();
				entry.0 |= state.lights_guild_rail(channel);
				entry.1 = entry.1.saturating_add(state.mention_count(channel.id));
			}
		}
		self.guild_badges = badges.into_iter().collect();
		self.direct = state.unread_directs(call).into_boxed_slice();
		self.key = Some(key);
		true
	}
	pub(super) fn guild_badge(&self, guild: Id) -> (bool, u32) {
		self.guild_badges
			.binary_search_by_key(&guild, |(id, _)| *id)
			.map(|index| self.guild_badges[index].1)
			.unwrap_or_default()
	}
}
fn direct_call(state: &State) -> Option<Id> {
	state
		.voice
		.active
		.as_ref()
		.filter(|call| call.guild.is_none())
		.map(|call| call.channel)
}

fn home_request_label(friends: u32, messages: u32) -> String {
	let mut parts = vec!["Direct Messages".to_owned()];
	if friends > 0 {
		parts.push(if friends == 1 {
			"1 friend request".into()
		} else {
			format!("{friends} friend requests")
		});
	}
	if messages > 0 {
		parts.push(if messages == 1 {
			"1 message request".into()
		} else {
			format!("{messages} message requests")
		});
	}
	parts.join(" · ")
}

pub(super) fn badge(ui: &egui::Ui, center: egui::Pos2, count: u32, ring: Color32) {
	let label = if count > 99 {
		"99+".into()
	} else {
		count.to_string()
	};
	let width = if count > 99 {
		30.0
	} else if count > 9 {
		24.0
	} else {
		19.0
	};
	let rect = egui::Rect::from_center_size(center, egui::vec2(width, 19.0));
	let colors = design::palette(ui);
	ui.painter().rect_filled(rect.expand(3.0), 12, ring);
	ui.painter().rect_filled(rect, 10, colors.danger);
	ui.painter().text(
		center,
		Align2::CENTER_CENTER,
		label,
		FontId::new(12.0, crate::design::semibold_family(ui.ctx())),
		Color32::WHITE,
	);
}
/// Rail pill on the window edge: short for unread, taller on hover, full when selected.
pub(super) fn rail_indicator(
	ui: &egui::Ui,
	rect: egui::Rect,
	selected: bool,
	hovered: bool,
	unread: bool,
) {
	let height = if selected {
		40.0
	} else if hovered {
		20.0
	} else if unread {
		8.0
	} else {
		return;
	};
	let pill = egui::Rect::from_center_size(
		egui::pos2(rect.left() - 10.0, rect.center().y),
		egui::vec2(8.0, height),
	);
	ui.painter()
		.rect_filled(pill, 4, design::palette(ui).text_strong);
}
/// Green speaker badge on the rail avatar of the conversation you are calling in.
fn call_badge(ui: &egui::Ui, rect: egui::Rect) {
	let colors = design::palette(ui);
	// Inset from the corner so neither the ring nor the glyph meets the list's clip rect.
	let center = rect.right_top() + egui::vec2(-10.0, 10.0);
	ui.painter()
		.circle_filled(center, 13.0, design::window_palette(ui).base);
	ui.painter().circle_filled(center, 11.0, colors.positive);
	crate::icons::paint(
		ui.painter(),
		crate::icons::Icon::Speaker,
		egui::Rect::from_center_size(center, egui::Vec2::splat(12.0)),
		egui::Color32::WHITE,
	);
}
fn indicator(ui: &egui::Ui, rect: egui::Rect, unread: bool, count: u32) {
	rail_indicator(ui, rect, false, false, unread);
	if count > 0 {
		badge(
			ui,
			rect.right_bottom() - egui::vec2(8.0, 8.0),
			count,
			design::palette(ui).base,
		);
	}
}
impl MessagingUi {
	pub fn viewing_latest(&self, channel: Id) -> bool {
		self.timeline.viewing_latest(channel)
	}
	pub(super) fn notification_rail(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		let mut selected = None;
		self.rail_cache.sync(state);
		egui::Panel::left("guilds")
			.resizable(false)
			.exact_size(RAIL_WIDTH)
			.show_separator_line(false)
			.frame(
				egui::Frame::new()
					.fill(design::section_surface(
						ui,
						design::window_palette(ui).base,
						design::ImageSection::ServerList,
					))
					.inner_margin(egui::Margin {
						left: 11,
						right: 11,
						top: 4,
						bottom: 8,
					}),
			)
			.show(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 11.0;
				let home = self.guild.is_none();
				let (rect, response) =
					ui.allocate_exact_size(egui::Vec2::splat(46.0), egui::Sense::click());
				let hovered = response.hovered() || response.has_focus();
				let fill = if home || hovered {
					colors.accent
				} else {
					colors.raised
				};
				ui.painter().rect_filled(rect, 13, fill);
				crate::icons::paint(
					ui.painter(),
					crate::icons::Icon::Serein,
					rect.shrink(10.5),
					if home || hovered {
						colors.accent_text
					} else {
						colors.text
					},
				);
				rail_indicator(ui, rect, home, hovered, false);
				let (friends, messages) = state.home_request_parts();
				let requests = friends.saturating_add(messages);
				if requests > 0 {
					badge(
						ui,
						rect.right_bottom() - egui::vec2(8.0, 8.0),
						requests,
						design::window_palette(ui).base,
					);
				}
				let label = home_request_label(friends, messages);
				response.widget_info(|| {
					egui::WidgetInfo::selected(egui::Role::Button, true, home, label.clone())
				});
				design::rail_name(&response, &label);
				if response.clicked() {
					self.guild = None;
					state.open_home();
					self.search.open = false;
				}
				self.scroll
					.attach(
						ui,
						"guild-list",
						egui::ScrollArea::vertical().scroll_bar_visibility(
							egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
						),
					)
					.show(ui, |ui| {
						ui.spacing_mut().item_spacing.y = 12.0;
						// Your own call keeps its conversation on the rail, like Discord's.
						let call = direct_call(state);
						// Copy one ID at a time so row actions can borrow the UI without cloning the cache.
						for index in 0..self.rail_cache.direct.len() {
							let Some(channel) = state.channel(self.rail_cache.direct[index]) else {
								continue;
							};
							let in_call = Some(channel.id) == call;
							let response = if channel.kind == 3 {
								self.avatars.show_group_rail(ui, channel, 48.0, state.demo)
							} else if let Some(user) = channel.recipients.first() {
								self.avatars.show_rail(ui, user, 48.0, state.demo)
							} else {
								let (rect, response) = ui.allocate_exact_size(
									egui::Vec2::splat(48.0),
									egui::Sense::click(),
								);
								design::paint_avatar(ui, &channel.name, 48.0, rect);
								response
							};
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
							let count = state.unread_count(channel.id);
							let unread = state.channel_unread(channel) == Some(true) || count > 0;
							indicator(ui, response.rect, unread, count);
							if in_call {
								call_badge(ui, response.rect);
							}
							response.widget_info(|| {
								egui::WidgetInfo::labeled(
									egui::Role::Button,
									true,
									format!(
										"Open {}{}, {} notifications",
										channel.name,
										if in_call {
											", in a call"
										} else if unread {
											", unread"
										} else {
											""
										},
										count
									),
								)
							});
							design::rail_name(&response, &channel.name);
							if response.clicked() {
								self.guild = None;
								selected = Some(channel.id);
							}
						}
						let (line, _) =
							ui.allocate_exact_size(egui::vec2(48.0, 2.0), egui::Sense::hover());
						ui.painter().rect_filled(
							egui::Rect::from_center_size(line.center(), egui::vec2(32.0, 2.0)),
							1,
							colors.raised,
						);
						self.server_folders(ui, state, commands);
						let (rect, response) =
							ui.allocate_exact_size(egui::Vec2::splat(48.0), egui::Sense::click());
						let hovered = response.hovered() || response.has_focus();
						ui.painter().rect_filled(
							rect,
							16,
							if hovered {
								colors.accent
							} else {
								colors.raised
							},
						);
						crate::icons::paint(
							ui.painter(),
							crate::icons::Icon::Plus,
							rect.shrink(12.0),
							if hovered {
								colors.accent_text
							} else {
								colors.text
							},
						);
						response.widget_info(|| {
							egui::WidgetInfo::labeled(egui::Role::Button, true, "Join a Server")
						});
						design::rail_name(&response, "Join a Server");
						if response.clicked() {
							self.join_server.open(state.generation);
						}
					});
			});
		if let Some(id) = selected
			&& let Some(command) = state.select(id)
		{
			commands.push(command);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Envelope, Event, read_state};

	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}

	#[test]
	fn home_rail_opens_friends_from_a_guild_channel() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = MessagingUi {
			guild: state
				.selected
				.and_then(|id| state.channel(id))
				.and_then(|channel| channel.guild),
			..Default::default()
		};
		view.search.open = true;
		let mut frame = |events| {
			let output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(800.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| view.notification_rail(ui, &mut state, &mut vec![]),
			);
			output.drop_without_applying_deltas();
		};
		frame(vec![]);
		for pressed in [true, false] {
			frame(vec![
				egui::Event::PointerMoved(egui::pos2(34.0, 27.0)),
				egui::Event::PointerButton {
					pos: egui::pos2(34.0, 27.0),
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			]);
		}
		assert_eq!(view.guild, None);
		assert_eq!(state.selected, None);
		assert!(!view.search.open);
	}

	#[test]
	fn rail_cache_reuses_idle_rows_and_tracks_unread_ack_permissions_and_removal() {
		let mut state = test_support::notification_demo_state();
		let mut cache = RailCache::default();
		assert!(cache.sync(&state));
		assert_eq!(
			state.channel_unread(state.channel(Id(27)).unwrap()),
			Some(true),
			"Threads omitted from known read-state start unread"
		);
		assert_eq!(&*cache.direct, &[Id(22)]);
		assert!(!cache.direct.contains(&Id(43)));
		assert_eq!(state.home_request_count(), 3);
		assert_eq!(state.home_request_parts(), (2, 1));
		let avery = state
			.pending_friends()
			.find(|(user, _, incoming)| *incoming && user.id == Id(8001))
			.map(|(user, _, _)| user.clone())
			.expect("demo incoming Avery");
		apply(
			&mut state,
			Event::ChannelCreated(model::Channel {
				id: Id(44),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Overlapping request (synthetic)".into(),
				kind: 1,
				recipients: vec![avery],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}),
		);
		apply(
			&mut state,
			Event::UserAction(client_core::user_actions::Event::MessageRequest {
				channel: Id(44),
				pending: true,
			}),
		);
		assert!(state.channel(Id(44)).is_some());
		assert_eq!(state.home_request_parts(), (2, 2));
		assert_eq!(state.home_request_count(), 4);
		let robin = state.friend(Id(1001)).cloned().expect("demo friend Robin");
		apply(
			&mut state,
			Event::ChannelCreated(model::Channel {
				id: Id(45),
				guild: None,
				parent_id: None,
				position: 0,
				name: "Friend-flagged request (synthetic)".into(),
				kind: 1,
				recipients: vec![robin],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}),
		);
		apply(
			&mut state,
			Event::UserAction(client_core::user_actions::Event::MessageRequest {
				channel: Id(45),
				pending: true,
			}),
		);
		assert!(state.channel(Id(45)).is_some());
		assert_eq!(state.home_request_parts(), (2, 2));
		assert_eq!(state.home_request_count(), 4);
		assert!(cache.sync(&state));
		assert_eq!(cache.guild_badge(Id(10)), (true, 1));
		for _ in 0..10 {
			assert!(!cache.sync(&state));
		}
		let badges = cache.guild_badges.as_ptr();
		apply(
			&mut state,
			Event::Reactions(client_core::reactions::Event::Cleared {
				channel: Id(22),
				message: Id(1003),
				emoji: None,
			}),
		);
		assert!(!cache.sync(&state));
		assert_eq!(cache.guild_badges.as_ptr(), badges);
		state.revision += 1;
		assert!(cache.sync(&state));
		apply(
			&mut state,
			Event::ReadState(read_state::Event::Ack {
				channel: Id(22),
				message: Some(Id(1003)),
				manual: false,
				mention_count: Some(0),
				version: None,
			}),
		);
		assert!(cache.sync(&state));
		assert!(cache.direct.is_empty());
		apply(
			&mut state,
			Event::Message(test_support::message(1007, Id(22))),
		);
		assert!(cache.sync(&state));
		assert_eq!(&*cache.direct, &[Id(22)]);
		apply(
			&mut state,
			Event::Permissions(client_core::permissions::Event::UnavailableGuild(Id(10))),
		);
		assert!(cache.sync(&state));
		assert_eq!(cache.guild_badge(Id(10)), (false, 0));
		apply(&mut state, Event::Unavailable(Id(22)));
		assert!(cache.sync(&state));
		assert!(cache.direct.is_empty());
		state.logout();
		assert!(cache.sync(&state));
		assert!(cache.guild_badges.is_empty());
	}

	#[test]
	fn rail_cache_preserves_the_first_fifteen_chats_and_local_call_changes() {
		let mut state = test_support::demo_state();
		let template = state.channel(Id(22)).unwrap().clone();
		for id in 100..116 {
			apply(
				&mut state,
				Event::ChannelCreated(model::Channel {
					id: Id(id),
					last_message: Some(Id(200)),
					..template.clone()
				}),
			);
		}
		apply(
			&mut state,
			Event::ReadState(read_state::Event::Snapshot {
				partial: false,
				entries: Some(
					std::iter::once((Id(20), Some(Id(495)), 0))
						.chain((100..=115).map(|id| (Id(id), Some(Id(1)), 0)))
						.collect(),
				),
				version: Some(1),
			}),
		);
		let mut cache = RailCache::default();
		assert!(cache.sync(&state));
		assert!(!cache.direct.contains(&Id(43)));
		assert_eq!(
			&*cache.direct,
			&(101..=115).rev().map(Id).collect::<Vec<_>>()
		);
		// Exercise the local command preparation gate; no command is dispatched by this test.
		state.demo = false;
		let revision = state.revision;
		assert!(state.start_call(Id(22), false).is_some());
		assert_eq!(state.revision, revision);
		assert!(cache.sync(&state));
		assert_eq!(cache.direct.len(), 15);
		assert_eq!(cache.direct[0], Id(22));
		assert_eq!(cache.direct[14], Id(102));
		assert!(state.leave_call().is_some());
		assert_eq!(state.revision, revision);
		assert!(cache.sync(&state));
		assert_eq!(cache.direct[0], Id(115));
		assert_eq!(cache.direct[14], Id(101));
		// Session failure through a local completion must also retire unread visibility.
		state.folders_pending = true;
		state.apply_guild_folders(Err(client_core::auth::Failure::Expired));
		assert!(cache.sync(&state));
		assert!(cache.direct.is_empty());
		apply(&mut state, Event::Resumed);
		assert!(cache.sync(&state));
		assert_eq!(cache.direct.len(), 15);
	}
}
