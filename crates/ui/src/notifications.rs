use crate::design::LazyHover;
use crate::{MessagingUi, design};
use client_core::{Command, State};
use egui::{Align2, Color32, FontId};
use model::Id;

/// Fixed width of the server rail column.
pub(super) const RAIL_WIDTH: f32 = 72.0;

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
			state.revision,
			state.gateway_connected,
			call,
		);
		if self.key == Some(key) {
			return false;
		}
		let mut badges = std::collections::BTreeMap::<Id, (bool, u32)>::new();
		let mut direct = Vec::new();
		for channel in state.channels.iter().take(client_core::MAX_NAV) {
			if let Some(guild) = channel.guild {
				let entry = badges.entry(guild).or_default();
				entry.0 |= state.channel_unread(channel) == Some(true)
					|| state.unread_count(channel.id) > 0;
				entry.1 = entry.1.saturating_add(state.mention_count(channel.id));
			} else if direct.len() < 15
				&& channel.supports_text()
				&& (Some(channel.id) == call
					|| state.channel_unread(channel) == Some(true)
					|| state.unread_count(channel.id) > 0)
			{
				direct.push(channel.id);
			}
		}
		self.guild_badges = badges.into_iter().collect();
		self.direct = direct.into_boxed_slice();
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
					.fill(design::window_palette(ui).base)
					.inner_margin(egui::Margin {
						left: 12,
						right: 12,
						top: 4,
						bottom: 8,
					}),
			)
			.show(ui, |ui| {
				ui.spacing_mut().item_spacing.y = 12.0;
				let home = self.guild.is_none();
				let (rect, response) =
					ui.allocate_exact_size(egui::Vec2::splat(48.0), egui::Sense::click());
				let hovered = response.hovered() || response.has_focus();
				let fill = if home || hovered {
					colors.accent
				} else {
					colors.raised
				};
				ui.painter().rect_filled(rect, 14, fill);
				crate::icons::paint(
					ui.painter(),
					crate::icons::Icon::Serein,
					rect.shrink(11.0),
					if home || hovered {
						colors.accent_text
					} else {
						colors.text
					},
				);
				rail_indicator(ui, rect, home, hovered, false);
				response.widget_info(|| {
					egui::WidgetInfo::selected(
						egui::WidgetType::SelectableLabel,
						true,
						home,
						"Direct messages",
					)
				});
				if response.on_hover_text("Direct Messages").clicked() {
					self.guild = None;
				}
				egui::ScrollArea::vertical()
					.id_salt("guild-list")
					.scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
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
								self.avatars.show_group(ui, channel, 48.0, state.demo)
							} else if let Some(user) = channel.recipients.first() {
								self.avatars.show(ui, user, 48.0, state.demo)
							} else {
								design::avatar(ui, &channel.name, 48.0)
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
									egui::WidgetType::Button,
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
							if response
								.on_hover_text_with(|| {
									format!(
										"{} · {}",
										channel.name,
										if in_call {
											"You are in this call"
										} else if state.channel_unread(channel).is_some() {
											"Unread activity; count may be a lower bound"
										} else {
											"Session activity · read sync unavailable"
										}
									)
								})
								.clicked()
							{
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
							egui::WidgetInfo::labeled(
								egui::WidgetType::Button,
								true,
								"Join a Server",
							)
						});
						if response.on_hover_text("Join a Server").clicked() {
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
	fn rail_cache_reuses_idle_rows_and_tracks_unread_ack_permissions_and_removal() {
		let mut state = test_support::notification_demo_state();
		let mut cache = RailCache::default();
		assert!(cache.sync(&state));
		assert_eq!(&*cache.direct, &[Id(22)]);
		assert_eq!(cache.guild_badge(Id(10)), (true, 1));
		for _ in 0..10 {
			assert!(!cache.sync(&state));
		}
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
		let mut cache = RailCache::default();
		assert!(cache.sync(&state));
		assert_eq!(&*cache.direct, &(100..115).map(Id).collect::<Vec<_>>());
		// Exercise the local command preparation gate; no command is dispatched by this test.
		state.demo = false;
		let revision = state.revision;
		assert!(state.start_call(Id(22), false).is_some());
		assert_eq!(state.revision, revision);
		assert!(cache.sync(&state));
		assert_eq!(cache.direct.len(), 15);
		assert_eq!(cache.direct[0], Id(22));
		assert_eq!(cache.direct[14], Id(113));
		assert!(state.leave_call().is_some());
		assert_eq!(state.revision, revision);
		assert!(cache.sync(&state));
		assert_eq!(cache.direct[0], Id(100));
		assert_eq!(cache.direct[14], Id(114));
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
