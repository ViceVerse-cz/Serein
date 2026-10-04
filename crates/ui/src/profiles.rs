//! Compact profile popout anchored beside the clicked user, populated only from the selected
//! service profile and already retained presence.
use crate::{
	avatars::Avatars,
	design,
	icons::{self, Icon},
	markdown::FormatCache,
};
use client_core::{State, profile::ProfileView};
use egui::{Color32, CornerRadius, Pos2, Rect, RichText, Stroke, UiBuilder, Vec2, pos2, vec2};
use model::{Id, User};

/// Voice presence follows the person across views, using only known, visible calls.
pub(crate) fn voice_users(state: &State) -> std::collections::BTreeSet<Id> {
	if !state.gateway_connected {
		return Default::default();
	}
	let mut users: std::collections::BTreeSet<_> = state
		.voice
		.roster
		.iter()
		.filter(|entry| {
			state.guilds.iter().any(|guild| guild.id == entry.guild)
				&& state.can_view(entry.channel)
		})
		.map(|entry| entry.participant.user)
		.collect();
	for (channel, participants) in state.voice.dm_call_participants() {
		if state.can_view(channel) {
			users.extend(participants.iter().map(|participant| participant.user));
		}
	}
	if let Some(call) = &state.voice.active
		&& call.guild.is_none()
		&& state.can_view(call.channel)
	{
		users.extend(call.participants.iter().map(|participant| participant.user));
	}
	users
}

/// Same rules as `voice_users` for one person; stops at the first match and allocates nothing,
/// so single-user callers do not scan the whole roster into a set each redraw.
pub(crate) fn user_in_voice(state: &State, user: Id) -> bool {
	state.gateway_connected
		&& (state.voice.roster.iter().any(|entry| {
			entry.participant.user == user
				&& state.guilds.iter().any(|guild| guild.id == entry.guild)
				&& state.can_view(entry.channel)
		}) || state
			.voice
			.dm_call_participants()
			.any(|(channel, participants)| {
				participants
					.iter()
					.any(|participant| participant.user == user)
					&& state.can_view(channel)
			}) || state.voice.active.as_ref().is_some_and(|call| {
			call.guild.is_none()
				&& call
					.participants
					.iter()
					.any(|participant| participant.user == user)
				&& state.can_view(call.channel)
		}))
}

/// Prefix a status row with the shared voice badge, reserving room for its existing text.
pub(crate) fn voice_badge(ui: &mut egui::Ui, in_voice: bool, has_status: bool) {
	if !in_voice {
		return;
	}
	let colors = design::palette(ui);
	let label = crate::i18n::translate("member-in-voice");
	icons::inline(ui, Icon::Speaker, 12.0, colors.positive);
	let width = ui.available_width() * if has_status { 0.5 } else { 1.0 };
	ui.scope(|ui| {
		ui.set_max_width(width);
		ui.add(
			egui::Label::new(RichText::new(&label).size(12.0).color(colors.positive))
				.truncate()
				.selectable(false),
		)
		.on_hover_text(&label);
	});
	if has_status {
		ui.label(RichText::new("·").size(12.0).color(colors.muted));
	}
}

pub(crate) fn server_tag_width(ui: &egui::Ui, tag: Option<&model::ClanTag>) -> f32 {
	tag.map_or(0.0, |tag| {
		let text = ui.painter().layout_no_wrap(
			tag.tag.clone(),
			egui::FontId::proportional(10.0),
			design::palette(ui).text,
		);
		text.size().x + 8.0 + if tag.badge.is_some() { 13.0 } else { 0.0 }
	})
}

pub(crate) fn server_tag(
	ui: &mut egui::Ui,
	tag: &model::ClanTag,
	avatars: &mut Avatars,
	demo: bool,
) -> egui::Response {
	let colors = design::palette(ui);
	egui::Frame::new()
		.fill(colors.raised)
		.corner_radius(4)
		.inner_margin(egui::Margin::symmetric(4, 1))
		.show(ui, |ui| {
			ui.spacing_mut().item_spacing.x = 3.0;
			if tag.badge.is_some() {
				avatars.show_icon(ui, tag.badge_key(), 10.0, demo, "Server tag badge");
			}
			ui.add(egui::Label::new(RichText::new(&tag.tag).size(10.0).strong()).selectable(false));
		})
		.response
		.on_hover_text(format!(
			"{} · {} {}",
			crate::i18n::translate("profiles-server-tag-server-tag"),
			crate::i18n::translate("profiles-server-tag-server"),
			tag.guild
		))
}

pub enum Action {
	Edit,
	Close,
	Retry,
	/// Explicitly submitted text from the profile footer; the DM may need creating first.
	SendMessage {
		user: User,
		content: String,
	},
	AddFriend(Id),
	RemoveFriend,
	AcceptFriend(Id),
	Profile(User),
	/// Open a mutual server picked from the card.
	Server(Id),
	Avatar(model::EmbedMedia),
	Banner(model::EmbedMedia),
	/// Shared user action picked from the card's overflow menu.
	Menu(crate::user_menu::Action),
}

/// Shared Rich Presence card for member profiles and the account preview.
pub(crate) fn activity_card(
	ui: &mut egui::Ui,
	activity: &model::RichActivity,
	avatars: &mut Avatars,
	demo: bool,
	(fill, muted): (Color32, Color32),
) {
	let spotify = is_spotify(activity);
	egui::Frame::new()
		.fill(fill)
		.corner_radius(RADIUS)
		.inner_margin(10)
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			ui.horizontal(|ui| {
				let heading = if spotify {
					crate::i18n::translate("profiles-activity-card-listening-to-spotify")
				} else {
					activity_verb(activity)
				};
				ui.label(design::semibold(ui, &heading, 12.0).color(muted));
				if spotify {
					icons::inline(ui, Icon::Spotify, 14.0, muted);
				}
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					let more = icons::button(
						ui,
						Icon::More,
						20.0,
						&crate::i18n::translate("profiles-activity-card-activity-options"),
					);
					egui::Popup::menu(&more).show(|ui| {
						if ui
							.button(crate::i18n::translate(
								"profiles-activity-card-copy-activity",
							))
							.clicked()
						{
							let mut text = activity_summary(activity);
							for line in [&activity.details, &activity.state].into_iter().flatten() {
								text.push('\n');
								text.push_str(line);
							}
							ui.copy_text(text);
							ui.close();
						}
					});
				});
			});
			ui.add_space(4.0);
			ui.horizontal_top(|ui| {
				ui.spacing_mut().item_spacing.x = 10.0;
				if let Some(image) = &activity.image {
					let artwork = avatars.show_icon(
						ui,
						Some(image.key()),
						64.0,
						demo,
						&format!("{} activity artwork", activity.name),
					);
					if let Some(badge) = &activity.small_image {
						let rect = Rect::from_center_size(
							artwork.rect.right_bottom() - Vec2::splat(6.0),
							Vec2::splat(24.0),
						);
						ui.painter().circle_filled(rect.center(), 14.0, fill);
						ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
							avatars.show_icon(ui, Some(badge.key()), 24.0, demo, "Activity badge");
						});
					}
				} else if spotify {
					icons::inline(ui, Icon::Spotify, 64.0, design::palette(ui).positive);
				}
				ui.vertical(|ui| {
					ui.set_width(ui.available_width());
					ui.spacing_mut().item_spacing.y = 2.0;
					let title = if spotify {
						activity.details.as_deref().unwrap_or(&activity.name)
					} else {
						&activity.name
					};
					ui.add(egui::Label::new(design::semibold(ui, title, 14.0)).truncate())
						.on_hover_text(title);
					for text in [
						activity.details.as_deref().filter(|_| !spotify),
						activity.state.as_deref(),
					]
					.into_iter()
					.flatten()
					{
						ui.add(
							egui::Label::new(RichText::new(text).size(12.0).color(muted))
								.truncate(),
						)
						.on_hover_text(text);
					}
					let now = std::time::SystemTime::now()
						.duration_since(std::time::UNIX_EPOCH)
						.unwrap_or_default()
						.as_millis() as u64;
					let playback = activity
						.started_at
						.zip(activity.ends_at)
						.filter(|(start, end)| spotify && end > start);
					if let Some((start, end)) = playback {
						let elapsed = now.saturating_sub(start).min(end - start);
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 4.0;
							ui.label(
								RichText::new(activity_elapsed(0, elapsed).unwrap())
									.monospace()
									.size(12.0)
									.color(muted),
							);
							let total = RichText::new(activity_elapsed(start, end).unwrap())
								.monospace()
								.size(12.0)
								.color(muted);
							ui.with_layout(
								egui::Layout::right_to_left(egui::Align::Center),
								|ui| {
									ui.label(total);
									ui.add(
										egui::ProgressBar::new(
											elapsed as f32 / (end - start) as f32,
										)
										.desired_width(ui.available_width())
										.desired_height(4.0)
										.fill(muted),
									);
								},
							);
						});
					} else if let Some(elapsed) =
						activity_timer(activity.started_at, activity.ends_at, now)
					{
						let color = design::palette(ui).positive;
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 4.0;
							icons::inline(
								ui,
								if spotify {
									Icon::Spotify
								} else {
									Icon::GameController
								},
								14.0,
								color,
							);
							ui.label(RichText::new(elapsed).monospace().size(12.0).color(color));
						});
					}
					if activity
						.ends_at
						.map_or(activity.started_at.is_some(), |end| end > now)
						&& ui.is_rect_visible(ui.min_rect())
					{
						ui.ctx()
							.request_repaint_after(std::time::Duration::from_secs(1));
					}
				});
			});
		});
}

/// Rich Presence stack: one activity gets the full card, the rest collapse to one-line rows
/// that swap in when clicked. The choice is keyed by activity name, not position.
pub(crate) fn activity_list(
	ui: &mut egui::Ui,
	id: egui::Id,
	activities: &[model::RichActivity],
	avatars: &mut Avatars,
	demo: bool,
	(fill, muted): (Color32, Color32),
) {
	let key = |activity: &model::RichActivity| egui::Id::unique((activity.kind, &activity.name));
	let chosen = ui.data(|data| data.get_temp::<egui::Id>(id));
	let main = chosen
		.and_then(|chosen| activities.iter().position(|a| key(a) == chosen))
		.unwrap_or(0);
	let Some(activity) = activities.get(main) else {
		return;
	};
	activity_card(ui, activity, avatars, demo, (fill, muted));
	for (index, activity) in activities.iter().enumerate() {
		if index != main && activity_row(ui, activity, avatars, demo, (fill, muted)).clicked() {
			ui.data_mut(|data| data.insert_temp(id, key(activity)));
		}
	}
}

fn activity_row(
	ui: &mut egui::Ui,
	activity: &model::RichActivity,
	avatars: &mut Avatars,
	demo: bool,
	(fill, muted): (Color32, Color32),
) -> egui::Response {
	let (rect, response) =
		ui.allocate_exact_size(vec2(ui.available_width(), 36.0), egui::Sense::click());
	let hot = response.hovered() || response.has_focus();
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Button,
			true,
			format!(
				"{} {}",
				crate::i18n::translate("profiles-activity-row-show"),
				activity_summary(activity)
			),
		)
	});
	if !ui.is_rect_visible(rect) {
		return response.on_hover_cursor(egui::CursorIcon::PointingHand);
	}
	let colors = design::palette(ui);
	ui.painter().rect_filled(
		rect,
		8,
		if hot {
			design::mix(fill, colors.text, 0.06)
		} else {
			fill
		},
	);
	let icon = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), Vec2::splat(24.0));
	if let Some(image) = &activity.image {
		let mut ui = ui.new_child(UiBuilder::new().max_rect(icon));
		avatars.show_icon(&mut ui, Some(image.key()), 24.0, demo, &activity.name);
	} else {
		let glyph = if is_spotify(activity) {
			Icon::Spotify
		} else {
			Icon::GameController
		};
		icons::paint(ui.painter(), glyph, icon.shrink(2.0), muted);
	}
	let chevron = Rect::from_center_size(
		pos2(rect.right() - 18.0, rect.center().y),
		Vec2::splat(16.0),
	);
	icons::paint(
		ui.painter(),
		Icon::ChevronDown,
		chevron,
		if hot { colors.text } else { muted },
	);
	let mut text = ui.new_child(
		UiBuilder::new()
			.max_rect(Rect::from_x_y_ranges(
				icon.right() + 8.0..=(chevron.left() - 6.0).max(icon.right() + 9.0),
				rect.y_range(),
			))
			.layout(egui::Layout::left_to_right(egui::Align::Center)),
	);
	text.spacing_mut().item_spacing.x = 6.0;
	let verb = activity_verb(activity);
	if !verb.is_empty() {
		text.add(egui::Label::new(RichText::new(verb).size(12.0).color(muted)).selectable(false));
	}
	text.add(
		egui::Label::new(design::semibold(ui, &activity.name, 13.0))
			.truncate()
			.selectable(false),
	);
	response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn activity_timer(start: Option<u64>, end: Option<u64>, now: u64) -> Option<String> {
	match end {
		Some(end) => {
			activity_elapsed(0, end.saturating_sub(now)).map(|time| format!("{time} remaining"))
		}
		None => start.and_then(|start| activity_elapsed(start, now)),
	}
}

fn activity_elapsed(start: u64, now: u64) -> Option<String> {
	let seconds = now.checked_sub(start)? / 1000;
	Some(if seconds >= 3600 {
		format!(
			"{}:{:02}:{:02}",
			seconds / 3600,
			seconds / 60 % 60,
			seconds % 60
		)
	} else {
		format!("{}:{:02}", seconds / 60, seconds % 60)
	})
}

const WIDTH: f32 = 340.0;
const PAD: f32 = 12.0;
const AVATAR: f32 = 80.0;
/// Card-coloured ring separating the profile avatar from the banner.
const AVATAR_RING: f32 = 6.0;
const RADIUS: u8 = 12;
/// Diameter of the translucent action circles laid over the banner.
const CIRCLE: f32 = 32.0;

fn friend_target(state: &State, user: &User) -> bool {
	!user.webhook
		&& user.kind == model::AccountKind::Human
		&& state.user.as_ref().is_some_and(|own| own.id != user.id)
}
/// Whether the account may send relationship and moderation requests right now.
fn actions_enabled(state: &State) -> bool {
	!state.user_action_pending()
		&& (state.demo
			|| (state.gateway_connected
				&& state.auth == client_core::auth::AuthState::Authenticated))
}
/// Translucent round button over the banner; disabled circles still show their tooltip.
fn header_circle(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
	let label = crate::i18n::translate_if_key(label);
	let (rect, response) = ui.allocate_exact_size(
		Vec2::splat(CIRCLE),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	let lit = enabled && (response.hovered() || response.has_focus());
	ui.painter().circle_filled(
		rect.center(),
		CIRCLE * 0.5,
		if lit {
			Color32::from_black_alpha(200)
		} else {
			Color32::from_black_alpha(140)
		},
	);
	icons::paint(
		ui.painter(),
		icon,
		rect.shrink(8.0),
		if enabled {
			Color32::WHITE
		} else {
			Color32::from_white_alpha(120)
		},
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, &label));
	response.on_hover_text(label)
}
/// Add-friend circle; hidden for blocked users, whose relationship lives in the overflow menu.
fn friend_circle(ui: &mut egui::Ui, state: &State, user: &User) -> Option<Action> {
	if !friend_target(state, user) || state.user_blocked(user.id) != Some(false) {
		return None;
	}
	let friend = state.friends().any(|friend| friend.id == user.id);
	let request = state
		.pending_friends()
		.find(|(person, _, _)| person.id == user.id);
	let (icon, label, action) = if friend {
		(
			Icon::Check,
			"profiles-friend-action-remove",
			Some(Action::RemoveFriend),
		)
	} else if let Some((_, _, incoming)) = request {
		if *incoming {
			(
				Icon::AddPeople,
				"profiles-friend-action-accept",
				Some(Action::AcceptFriend(user.id)),
			)
		} else {
			(Icon::AddPeople, "profiles-friend-action-sent", None)
		}
	} else if !state.friends_known() || !state.friend_requests_known() {
		(Icon::AddPeople, "profiles-friend-action-loading", None)
	} else {
		(
			Icon::AddPeople,
			"friends-add",
			Some(Action::AddFriend(user.id)),
		)
	};
	let enabled = action.is_some()
		&& state.friends_known()
		&& state.friend_requests_known()
		&& actions_enabled(state);
	if header_circle(ui, icon, &crate::i18n::translate(label), enabled).clicked() {
		action
	} else {
		None
	}
}
/// Overflow menu behind the three-dots circle: webhook copy, notes, mute, block and friend removal.
fn more_menu(
	ui: &mut egui::Ui,
	state: &State,
	user: &User,
	dm_channel: Option<Id>,
) -> Option<Action> {
	let colors = design::palette(ui);
	let mut action = None;
	ui.set_min_width(200.0);
	ui.spacing_mut().button_padding = vec2(8.0, 6.0);
	let own_profile = state.user.as_ref().is_some_and(|own| own.id == user.id);
	// The footer owns message composition, so the menu does not repeat it.
	if user.webhook
		&& ui
			.button(crate::i18n::translate("profiles-more-menu-copy-webhook-id"))
			.clicked()
	{
		ui.ctx().copy_text(user.id.to_string());
		ui.close();
	}
	if user.webhook || own_profile {
		return action;
	}
	let enabled = actions_enabled(state);
	let friend = state.friends().any(|friend| friend.id == user.id);
	ui.separator();
	if ui
		.add_enabled(
			enabled,
			egui::Button::new(crate::i18n::translate("profiles-more-menu-add-note")),
		)
		.clicked()
	{
		action = Some(Action::Menu(crate::user_menu::Action::Note(user.clone())));
		ui.close();
	}
	if ui
		.add_enabled(
			enabled && friend,
			egui::Button::new(crate::i18n::translate_if_key(
				&(if state.friend_nickname(user.id).is_some() {
					crate::i18n::translate("profiles-more-menu-edit-friend-nickname")
				} else {
					crate::i18n::translate("profiles-more-menu-add-friend-nickname")
				}),
			)),
		)
		.on_disabled_hover_text(crate::i18n::translate(
			"profiles-more-menu-private-nicknames-are-available-for-confirmed-friends",
		))
		.clicked()
	{
		action = Some(Action::Menu(crate::user_menu::Action::Nickname(
			user.clone(),
		)));
		ui.close();
	}
	ui.separator();
	if let Some(channel) = dm_channel {
		let muted = state.dm_muted(channel) == Some(true);
		if ui
			.add_enabled(
				enabled,
				egui::Button::new(crate::i18n::translate_if_key(
					&(if muted {
						crate::i18n::translate("profiles-more-menu-unmute")
					} else {
						crate::i18n::translate("profiles-more-menu-mute")
					}),
				)),
			)
			.on_hover_text(crate::i18n::translate(
				"profiles-more-menu-mute-this-direct-message-s-notifications-until-you-unmute-it",
			))
			.clicked()
		{
			action = Some(Action::Menu(crate::user_menu::Action::Mute {
				channel,
				muted: !muted,
			}));
			ui.close();
		}
	} else {
		ui.add_enabled(
			false,
			egui::Button::new(crate::i18n::translate("profiles-more-menu-mute")),
		)
		.on_disabled_hover_text(crate::i18n::translate(
			"profiles-more-menu-no-open-direct-message-with-this-user",
		));
	}
	if friend
		&& ui
			.add_enabled(
				enabled,
				egui::Button::new(crate::i18n::translate("profiles-more-menu-remove-friend")),
			)
			.clicked()
	{
		action = Some(Action::RemoveFriend);
		ui.close();
	}
	ui.separator();
	let blocked = state.user_blocked(user.id) == Some(true);
	if ui
		.add_enabled(
			enabled,
			egui::Button::new(
				RichText::new(crate::i18n::translate_if_key(
					&(if blocked {
						crate::i18n::translate("profiles-more-menu-unblock")
					} else {
						crate::i18n::translate("profiles-more-menu-block")
					}),
				))
				.color(colors.danger),
			),
		)
		.clicked()
	{
		action = Some(Action::Menu(crate::user_menu::Action::Block {
			user: user.id,
			blocked: !blocked,
		}));
		ui.close();
	}
	action
}

impl crate::MessagingUi {
	pub(super) fn confirm_friend_removal(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<client_core::Command>,
	) {
		let Some((generation, user)) = &self.friend_removal else {
			return;
		};
		if *generation != state.generation || !state.friends().any(|friend| friend.id == user.id) {
			self.friend_removal = None;
			return;
		}
		let result = crate::dialog::Confirm::new(
			("remove-profile-friend", user.id),
			"profiles-remove-friend-title",
			crate::i18n::translate_args("profiles-remove-friend-message", &[("user", &user.name)]),
		)
		.danger()
		.confirm_label("profiles-more-menu-remove-friend")
		.enabled(
			!state.user_action_pending()
				&& state.friends_known()
				&& state.friend_requests_known()
				&& (state.demo
					|| (state.gateway_connected
						&& state.auth == client_core::auth::AuthState::Authenticated)),
		)
		.show(ctx);
		match result {
			Some(crate::dialog::Choice::Confirmed) => {
				if let Some(command) = state.remove_friend(user.id) {
					commands.push(command);
				}
				self.friend_removal = None;
			}
			Some(crate::dialog::Choice::Cancelled) => self.friend_removal = None,
			None => {}
		}
	}
}

pub fn presence_label(status: &str) -> String {
	crate::i18n::translate(match status {
		"online" => "status-online",
		"idle" => "status-idle",
		"dnd" => "status-dnd",
		"offline" => "status-offline",
		_ => "friends-presence-unavailable",
	})
}
pub(crate) fn presence_color(status: &str) -> Color32 {
	match status {
		"online" => Color32::from_rgb(35, 165, 89),
		"idle" => Color32::from_rgb(240, 178, 50),
		"dnd" => Color32::from_rgb(242, 63, 67),
		_ => Color32::from_rgb(128, 132, 142),
	}
}
/// Dot radius and ring width. Banner avatars punch the dot out with the same ring they use
/// against the banner; list avatars keep a thin ring.
fn presence_badge_metrics(rect: Rect) -> (f32, f32) {
	if rect.width() >= AVATAR {
		(rect.width() * 0.16, AVATAR_RING)
	} else {
		((rect.width() * 0.2).clamp(6.0, 10.0), 2.0)
	}
}
fn presence_badge_rect(rect: Rect) -> Rect {
	let (radius, ring) = presence_badge_metrics(rect);
	let center = rect.right_bottom() - Vec2::splat(radius + 0.5);
	Rect::from_center_size(center, Vec2::splat((radius + ring) * 2.0))
}
fn pointer_on_presence(status: Option<&str>, avatar: Rect, pointer: Option<egui::Pos2>) -> bool {
	status.is_some() && pointer.is_some_and(|pos| presence_badge_rect(avatar).contains(pos))
}
pub(crate) fn presence_badge(
	ui: &mut egui::Ui,
	rect: Rect,
	status: &str,
	clients: model::ClientPlatforms,
	ring: Color32,
) {
	let (radius, ring_width) = presence_badge_metrics(rect);
	let center = rect.right_bottom() - Vec2::splat(radius + 0.5);
	ui.painter()
		.circle_filled(center, radius + ring_width, ring);
	if clients.mobile {
		icons::paint(
			ui.painter(),
			Icon::DeviceMobile,
			Rect::from_center_size(center, Vec2::splat(radius * 2.2)),
			presence_color(status),
		);
	} else {
		ui.painter()
			.circle_filled(center, radius, presence_color(status));
	}
	ui.allocate_rect(presence_badge_rect(rect), egui::Sense::hover())
		.on_hover_text(presence_label(status));
}
/// Keep known guild presence through range loads and reconnects; access loss clears the snapshot.
pub(crate) fn presence(
	state: &State,
	user: Id,
	guild: Option<Id>,
) -> (
	Option<&str>,
	Option<&str>,
	&[model::RichActivity],
	model::ClientPlatforms,
) {
	let remote = if let Some(member) = state
		.members
		.as_ref()
		.filter(|list| {
			guild.is_some()
				&& list.guild == guild
				&& list.freshness != model::Freshness::Unavailable
				&& (state.demo || state.can_view(list.channel))
		})
		.and_then(|list| {
			list.slots
				.iter()
				.flatten()
				.filter_map(|slot| match slot {
					model::MemberSlot::Person(m) => Some(m),
					_ => None,
				})
				.find(|member| member.user.id == user)
		}) {
		(
			member.status.as_deref(),
			member.custom_status.as_deref(),
			member.activities.as_slice(),
			member.clients,
		)
	} else {
		state.presence_for(user).map_or(
			(None, None, &[][..], model::ClientPlatforms::default()),
			|presence| {
				(
					presence.status.as_deref(),
					presence.custom_status.as_deref(),
					presence.activities.as_slice(),
					presence.clients,
				)
			},
		)
	};
	with_local_activity(state, user, remote)
}

pub(crate) fn member_presence<'a>(
	state: &'a State,
	member: &'a model::Member,
	guild: Option<Id>,
) -> (
	Option<&'a str>,
	Option<&'a str>,
	&'a [model::RichActivity],
	model::ClientPlatforms,
) {
	let remote = if guild.is_some()
		&& state.members.as_ref().is_some_and(|list| {
			list.guild == guild
				&& list.freshness != model::Freshness::Unavailable
				&& (state.demo || state.can_view(list.channel))
		}) {
		(
			member.status.as_deref(),
			member.custom_status.as_deref(),
			member.activities.as_slice(),
			member.clients,
		)
	} else {
		state.presence_for(member.user.id).map_or(
			(None, None, &[][..], model::ClientPlatforms::default()),
			|p| {
				(
					p.status.as_deref(),
					p.custom_status.as_deref(),
					p.activities.as_slice(),
					p.clients,
				)
			},
		)
	};
	with_local_activity(state, member.user.id, remote)
}

fn with_local_activity<'a>(
	state: &'a State,
	user: Id,
	remote: (
		Option<&'a str>,
		Option<&'a str>,
		&'a [model::RichActivity],
		model::ClientPlatforms,
	),
) -> (
	Option<&'a str>,
	Option<&'a str>,
	&'a [model::RichActivity],
	model::ClientPlatforms,
) {
	if state.user.as_ref().is_some_and(|own| own.id == user)
		&& let Some(activity) = state.local_game_activity()
	{
		(remote.0, remote.1, std::slice::from_ref(activity), remote.3)
	} else {
		remote
	}
}

pub(crate) fn is_spotify(activity: &model::RichActivity) -> bool {
	activity.kind == 2 && activity.name.eq_ignore_ascii_case("Spotify")
}

fn activity_verb(activity: &model::RichActivity) -> String {
	crate::i18n::translate_if_key(match activity.kind {
		0 => "profiles-activity-verb-playing",
		1 => "profiles-activity-verb-streaming",
		2 => "profiles-activity-verb-listening-to",
		3 => "profiles-activity-verb-watching",
		5 => "profiles-activity-verb-competing-in",
		_ => "profiles-activity-verb-activity",
	})
}

fn activity_summary(activity: &model::RichActivity) -> String {
	format!("{} {}", activity_verb(activity), activity.name)
}

pub(crate) fn subtitle(custom: Option<&str>, activities: &[model::RichActivity]) -> Option<String> {
	activities
		.first()
		.map(|activity| {
			if is_spotify(activity) {
				activity
					.state
					.clone()
					.unwrap_or_else(|| activity_summary(activity))
			} else {
				activity_summary(activity)
			}
		})
		.or_else(|| custom.map(str::to_owned))
}

fn rgb(value: u32) -> Color32 {
	Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}
fn luma(color: Color32) -> f32 {
	let [r, g, b, _] = color.to_array();
	(0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0
}
/// Card colors: the shared palette, or the profile theme when the account configured one.
struct Theme {
	gradient: Option<(Color32, Color32)>,
	text: Color32,
	muted: Color32,
	link: Color32,
	/// Translucent body panel laid over the gradient.
	panel: Color32,
	/// Chips and secondary buttons inside the panel.
	chip: Color32,
	chip_hover: Color32,
	border: Color32,
	divider: Color32,
	card: Color32,
}
impl Theme {
	fn new(colors: &design::Palette, theme: Option<[u32; 2]>) -> Self {
		match theme {
			Some([top, bottom]) => {
				let (top, bottom) = (rgb(top), rgb(bottom));
				// The body panel is opaque enough that its tint decides contrast: white over a
				// bright gradient takes dark text, black over a dark one takes light text.
				let bright = (luma(top) + luma(bottom)) * 0.5 > 0.5;
				Self {
					gradient: Some((top, bottom)),
					text: if bright {
						Color32::from_rgb(24, 27, 31)
					} else {
						Color32::from_rgb(242, 243, 245)
					},
					muted: if bright {
						Color32::from_rgb(70, 76, 84)
					} else {
						Color32::from_rgb(190, 195, 201)
					},
					link: if bright {
						Color32::from_rgb(0, 96, 208)
					} else {
						Color32::from_rgb(0, 176, 244)
					},
					panel: if bright {
						Color32::from_white_alpha(170)
					} else {
						Color32::from_black_alpha(130)
					},
					chip: if bright {
						Color32::from_black_alpha(18)
					} else {
						Color32::from_white_alpha(20)
					},
					chip_hover: if bright {
						Color32::from_black_alpha(36)
					} else {
						Color32::from_white_alpha(40)
					},
					border: if bright {
						Color32::from_black_alpha(48)
					} else {
						Color32::from_white_alpha(32)
					},
					divider: if bright {
						Color32::from_black_alpha(30)
					} else {
						Color32::from_white_alpha(24)
					},
					card: top.lerp_to_gamma(bottom, 0.3),
				}
			}
			None => Self {
				gradient: None,
				text: colors.text,
				muted: colors.muted,
				link: colors.link,
				panel: colors.raised,
				chip: colors.hover,
				chip_hover: colors.selected,
				border: colors.border,
				divider: colors.border,
				card: colors.surface,
			},
		}
	}
	fn background(&self, rect: Rect) -> egui::Shape {
		let radius = f32::from(RADIUS);
		let mut shapes = vec![
			egui::Shape::Rect(
				egui::epaint::Shadow {
					offset: [0, 8],
					blur: 24,
					spread: 0,
					color: Color32::from_black_alpha(140),
				}
				.as_shape(rect, RADIUS),
			),
			egui::Shape::rect_filled(rect, RADIUS, self.card),
		];
		if let Some((top, bottom)) = self.gradient {
			// Rounded caps in the end colors, with the flat gradient band between them, so the
			// corners stay round instead of being squared off by the mesh.
			shapes.push(egui::Shape::rect_filled(rect, RADIUS, top));
			// egui clamps corner radii to half the rectangle height. The gradient band
			// covers this cap's upper half, leaving the full-radius bottom corners.
			shapes.push(egui::Shape::rect_filled(
				Rect::from_min_max(
					pos2(rect.left(), rect.bottom() - 2.0 * radius),
					rect.right_bottom(),
				),
				CornerRadius {
					nw: 0,
					ne: 0,
					sw: RADIUS,
					se: RADIUS,
				},
				bottom,
			));
			let band = Rect::from_min_max(
				pos2(rect.left(), rect.top() + radius),
				pos2(rect.right(), rect.bottom() - radius),
			);
			let mut mesh = egui::Mesh::default();
			mesh.colored_vertex(band.left_top(), top);
			mesh.colored_vertex(band.right_top(), top);
			mesh.colored_vertex(band.left_bottom(), bottom);
			mesh.colored_vertex(band.right_bottom(), bottom);
			mesh.add_triangle(0, 1, 2);
			mesh.add_triangle(1, 3, 2);
			shapes.push(egui::Shape::mesh(mesh));
		}
		shapes.push(egui::Shape::rect_stroke(
			rect,
			RADIUS,
			Stroke::new(1.0, self.border),
			egui::StrokeKind::Inside,
		));
		egui::Shape::Vec(shapes)
	}
}
/// Section title; adds breathing room before every section after the first.
fn section(ui: &mut egui::Ui, theme: &Theme, count: &mut usize, text: &str) {
	let text = crate::i18n::translate_if_key(text);
	if *count > 0 {
		ui.add_space(10.0);
	}
	*count += 1;
	ui.label(RichText::new(text).size(12.0).strong().color(theme.muted));
	ui.add_space(2.0);
}
fn divider(ui: &mut egui::Ui, theme: &Theme) {
	ui.add_space(4.0);
	let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
	ui.painter().hline(
		rect.x_range(),
		rect.center().y,
		Stroke::new(1.0, theme.divider),
	);
	ui.add_space(4.0);
}
/// Which mutual list the side panel beside the card shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mutuals {
	Servers,
	Friends,
}
const MUTUALS_WIDTH: f32 = 300.0;

/// Inline "3 Mutual Servers" link under the name; opens the matching side panel.
fn mutual_link(ui: &mut egui::Ui, theme: &Theme, icon: Icon, text: String) -> egui::Response {
	let galley = ui
		.painter()
		.layout_no_wrap(text, egui::FontId::proportional(14.0), theme.muted);
	let (rect, response) = ui.allocate_exact_size(
		vec2(22.0 + galley.size().x, galley.size().y.max(20.0)),
		egui::Sense::click(),
	);
	let color = if response.hovered() {
		theme.text
	} else {
		theme.muted
	};
	icons::paint(
		ui.painter(),
		icon,
		Rect::from_center_size(pos2(rect.left() + 8.0, rect.center().y), Vec2::splat(16.0)),
		color,
	);
	let text = pos2(rect.left() + 22.0, rect.center().y - galley.size().y * 0.5);
	let width = galley.size().x;
	let label = galley.text().to_owned();
	ui.painter()
		.galley_with_override_text_color(text, galley, color);
	if response.hovered() {
		ui.painter().hline(
			text.x..=text.x + width,
			rect.bottom() - 1.0,
			Stroke::new(1.0, color),
		);
	}
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
	response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One clickable row in the mutuals panel: a 32 px face painted by `face`, then the name.
fn mutual_row(
	ui: &mut egui::Ui,
	theme: &Theme,
	name: &str,
	face: impl FnOnce(&mut egui::Ui, Rect),
) -> egui::Response {
	let (rect, response) =
		ui.allocate_exact_size(vec2(ui.available_width(), 44.0), egui::Sense::click());
	if response.hovered() {
		ui.painter().rect_filled(rect, 8, theme.chip_hover);
	}
	let avatar = Rect::from_min_size(
		pos2(rect.left() + 8.0, rect.center().y - 16.0),
		Vec2::splat(32.0),
	);
	face(ui, avatar);
	let galley = ui.painter().layout(
		name.to_owned(),
		egui::FontId::proportional(14.0),
		theme.text,
		rect.right() - avatar.right() - 20.0,
	);
	ui.painter().galley(
		pos2(
			avatar.right() + 12.0,
			rect.center().y - galley.size().y * 0.5,
		),
		galley,
		theme.text,
	);
	response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Side panel listing mutual servers or friends, pointing at the link that opened it.
#[allow(clippy::too_many_arguments)]
fn mutuals_panel(
	ctx: &egui::Context,
	kind: Mutuals,
	data: &model::UserProfile,
	state: &State,
	avatars: &mut Avatars,
	theme: &Theme,
	(card, link, bounds): (Rect, Rect, Rect),
	user: Id,
) -> (Rect, Option<Action>) {
	let mut action = None;
	let left = card.left() - 12.0 - MUTUALS_WIDTH;
	let on_left = left >= bounds.left();
	let x = if on_left { left } else { card.right() + 12.0 };
	let (title, count) = match kind {
		Mutuals::Servers => ("profiles-show-mutual-servers", data.mutual_guilds.len()),
		Mutuals::Friends => ("profiles-show-mutual-friends", data.mutual_friends.len()),
	};
	let area = egui::Area::new(egui::Id::unique(("profile-mutuals-panel", user)))
		.kind(egui::UiKind::Popup)
		.order(egui::Order::Foreground)
		.fixed_pos(pos2(x, (link.center().y - 40.0).max(bounds.top())))
		.constrain_to(bounds)
		.interactable(true)
		.show(ctx, |ui| {
			// Area remembers its previous size; let the list grow past the first, empty frame.
			ui.set_max_height(bounds.height());
			ui.style_mut().interaction.selectable_labels = false;
			ui.visuals_mut().override_text_color = Some(theme.text);
			egui::Frame::new()
				.fill(theme.card)
				.stroke(Stroke::new(1.0, theme.border))
				.corner_radius(RADIUS)
				.shadow(egui::epaint::Shadow {
					offset: [0, 8],
					blur: 24,
					spread: 0,
					color: Color32::from_black_alpha(140),
				})
				.show(ui, |ui| {
					ui.set_width(MUTUALS_WIDTH);
					ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
					egui::Frame::new()
						.inner_margin(egui::Margin {
							left: 16,
							right: 16,
							top: 14,
							bottom: 10,
						})
						.show(ui, |ui| {
							ui.horizontal(|ui| {
								ui.label(
									RichText::new(crate::i18n::translate(title))
										.size(15.0)
										.strong(),
								);
								ui.with_layout(
									egui::Layout::right_to_left(egui::Align::Center),
									|ui| {
										ui.label(
											RichText::new(count.to_string())
												.size(14.0)
												.strong()
												.color(theme.muted),
										);
									},
								);
							});
						});
					let (line, _) = ui
						.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
					ui.painter().hline(
						line.x_range(),
						line.center().y,
						Stroke::new(1.0, theme.divider),
					);
					egui::Frame::new().inner_margin(8).show(ui, |ui| {
						egui::ScrollArea::vertical()
							.id_salt(("profile-mutuals", user, title))
							.max_height((bounds.height() - 80.0).clamp(88.0, 360.0))
							.min_scrolled_height((bounds.height() - 80.0).clamp(88.0, 360.0))
							.show(ui, |ui| match kind {
								Mutuals::Servers => {
									for mutual in &data.mutual_guilds {
										let guild = state.guilds.iter().find(|g| g.id == mutual.id);
										let name = guild.map_or_else(
											|| {
												format!(
													"{} {}",
													crate::i18n::translate(
														"profiles-show-server-2"
													),
													mutual.id
												)
											},
											|guild| guild.name.clone(),
										);
										let row = mutual_row(ui, theme, &name, |ui, rect| {
											if let Some(guild) = guild {
												avatars
													.paint_guild(ui, guild, rect, state.demo, 16);
											} else {
												ui.painter().circle_filled(
													rect.center(),
													16.0,
													theme.chip,
												);
											}
										});
										if row.clicked() && guild.is_some() {
											action = Some(Action::Server(mutual.id));
										}
									}
								}
								Mutuals::Friends => {
									for friend in &data.mutual_friends {
										let row = mutual_row(
											ui,
											theme,
											state.user_display_name(friend),
											|ui, rect| {
												ui.scope_builder(
													UiBuilder::new().max_rect(rect),
													|ui| {
														avatars.show_plain(
															ui, friend, 32.0, state.demo,
														);
													},
												);
											},
										);
										if row.clicked() {
											action = Some(Action::Profile(friend.clone()));
										}
									}
								}
							});
					});
				});
		});
	let rect = area.response.rect;
	// Small tail pointing back at the link that opened the panel.
	let y = link
		.center()
		.y
		.clamp(rect.top() + 18.0, rect.bottom() - 18.0);
	let (edge, tip) = if on_left {
		(rect.right() - 1.0, rect.right() + 7.0)
	} else {
		(rect.left() + 1.0, rect.left() - 7.0)
	};
	let painter = ctx.layer_painter(area.response.layer_id);
	painter.add(egui::Shape::convex_polygon(
		vec![pos2(edge, y - 8.0), pos2(tip, y), pos2(edge, y + 8.0)],
		theme.card,
		Stroke::NONE,
	));
	painter.line_segment(
		[pos2(edge + (tip - edge) / 8.0, y - 7.0), pos2(tip, y)],
		Stroke::new(1.0, theme.border),
	);
	painter.line_segment(
		[pos2(tip, y), pos2(edge + (tip - edge) / 8.0, y + 7.0)],
		Stroke::new(1.0, theme.border),
	);
	(rect, action)
}

/// Display name and glyph for a connected account's service.
fn brand(kind: &str) -> (&str, Icon) {
	match kind {
		"github" => ("GitHub", Icon::GitHub),
		"twitch" => ("Twitch", Icon::Twitch),
		"steam" => ("Steam", Icon::Steam),
		"spotify" => ("Spotify", Icon::Spotify),
		"youtube" => ("YouTube", Icon::YouTube),
		"twitter" => ("X", Icon::XLogo),
		"reddit" => ("Reddit", Icon::Reddit),
		"facebook" => ("Facebook", Icon::Facebook),
		"instagram" => ("Instagram", Icon::Instagram),
		"tiktok" => ("TikTok", Icon::TikTok),
		"paypal" => ("PayPal", Icon::PayPal),
		"amazon-music" => ("Amazon Music", Icon::Amazon),
		"bluesky" => ("Bluesky", Icon::Bluesky),
		"mastodon" => ("Mastodon", Icon::Mastodon),
		"skype" => ("Skype", Icon::Skype),
		// Neither icon set ships an Xbox mark (Microsoft brand guidelines); keep the controller.
		"xbox" => ("Xbox", Icon::GameController),
		"playstation" => ("PlayStation", Icon::PlayStation),
		"battlenet" => ("Battle.net", Icon::BattleNet),
		"epicgames" => ("Epic Games", Icon::EpicGames),
		"leagueoflegends" => ("League of Legends", Icon::LeagueOfLegends),
		"riotgames" => ("Riot Games", Icon::RiotGames),
		"bungie" => ("Bungie.net", Icon::Bungie),
		"roblox" => ("Roblox", Icon::Roblox),
		"crunchyroll" => ("Crunchyroll", Icon::Crunchyroll),
		"domain" => ("Domain", Icon::Globe),
		"ebay" => ("eBay", Icon::Ebay),
		other => (other, Icon::Link),
	}
}

/// Public profile page for a connected account, when the service has one. Names and IDs are
/// restricted to plain path characters so they can never change the destination host.
fn connection_url(connection: &model::ProfileConnection) -> Option<String> {
	fn plain(value: &str) -> bool {
		!value.is_empty()
			&& value
				.chars()
				.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
			&& !value.contains("..")
	}
	let (name, id) = (connection.name.as_str(), connection.id.as_str());
	let by_name = |base: &str| plain(name).then(|| format!("{base}{name}"));
	let by_id = |base: &str| plain(id).then(|| format!("{base}{id}"));
	match connection.kind.as_str() {
		"github" => by_name("https://github.com/"),
		"twitch" => by_name("https://www.twitch.tv/"),
		"steam" => by_id("https://steamcommunity.com/profiles/"),
		"spotify" => by_id("https://open.spotify.com/user/"),
		"youtube" => by_id("https://www.youtube.com/channel/"),
		"twitter" => by_name("https://x.com/"),
		"reddit" => by_name("https://www.reddit.com/user/"),
		"instagram" => by_name("https://www.instagram.com/"),
		"tiktok" => by_name("https://www.tiktok.com/@"),
		"bluesky" => by_name("https://bsky.app/profile/"),
		"roblox" => by_id("https://www.roblox.com/users/").map(|url| url + "/profile"),
		"ebay" => by_name("https://www.ebay.com/usr/"),
		"mastodon" => {
			let (user, host) = name.trim_start_matches('@').split_once('@')?;
			(plain(user) && plain(host) && host.contains('.'))
				.then(|| format!("https://{host}/@{user}"))
		}
		"domain" => (plain(name) && name.contains('.')).then(|| format!("https://{name}")),
		_ => None,
	}
}

/// Row of connected-account glyphs; clicking one queues its profile page for the link prompt.
fn connection_icons(
	ui: &mut egui::Ui,
	theme: &Theme,
	connections: &[model::ProfileConnection],
	opening: &mut Option<String>,
) {
	ui.horizontal_wrapped(|ui| {
		ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
		for connection in connections {
			let (label, icon) = brand(&connection.kind);
			let url = connection_url(connection);
			let sense = if url.is_some() {
				egui::Sense::click()
			} else {
				egui::Sense::hover()
			};
			let (rect, response) = ui.allocate_exact_size(Vec2::splat(36.0), sense);
			if response.hovered() {
				ui.painter()
					.circle_filled(rect.center(), 18.0, theme.chip_hover);
			}
			icons::paint(ui.painter(), icon, rect.shrink(5.0), theme.text);
			let mut tip = format!("{label}: {}", connection.name);
			if connection.verified {
				tip.push_str(" ✓");
			}
			response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, &tip));
			let response = response.on_hover_text(tip);
			if let Some(url) = url
				&& response
					.on_hover_cursor(egui::CursorIcon::PointingHand)
					.clicked()
			{
				*opening = Some(url);
			}
		}
	});
}

fn creation_date(id: Id) -> Option<String> {
	let seconds = ((id.0 >> 22) + 1_420_070_400_000) / 1000;
	time::OffsetDateTime::from_unix_timestamp(seconds as i64)
		.ok()
		.map(|date| crate::local_time::local(date).date().to_string())
}

fn role_chips(
	ui: &mut egui::Ui,
	theme: &Theme,
	state: &State,
	user: Id,
	guild: &model::GuildProfile,
) {
	let Some(roles) = state.guild_roles(guild.guild) else {
		return;
	};
	let roles: Vec<_> = roles
		.iter()
		.rev()
		.filter(|role| role.id != guild.guild && guild.roles.contains(&role.id))
		.collect();
	let max_width = ui.available_width();
	let widths: Vec<_> = roles
		.iter()
		.map(|role| {
			(ui.painter()
				.layout_no_wrap(
					role.name.clone(),
					egui::FontId::proportional(12.0),
					theme.text,
				)
				.size()
				.x + 25.0)
				.min(max_width)
		})
		.collect();
	let expanded_id = ui.make_persistent_id(("profile-roles-expanded", guild.guild, user));
	let expanded = ui.data(|data| data.get_temp::<bool>(expanded_id).unwrap_or(false));
	let fits = |widths: &[f32]| {
		let mut rows = 1;
		let mut used = 0.0;
		for width in widths {
			if used > 0.0 && used + 4.0 + width > max_width {
				rows += 1;
				used = 0.0;
			}
			used += if used == 0.0 { *width } else { 4.0 + width };
		}
		rows <= 2
	};
	let visible = if expanded {
		roles.len()
	} else {
		(0..=roles.len())
			.rev()
			.find(|&count| {
				let hidden = roles.len() - count;
				let mut row = widths[..count].to_vec();
				if hidden > 0 {
					row.push(25.0 + hidden.to_string().len() as f32 * 7.0);
				}
				fits(&row)
			})
			.unwrap_or(0)
	};
	ui.horizontal_wrapped(|ui| {
		ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
		for (role, width) in roles.iter().zip(&widths).take(visible) {
			let galley = ui.painter().layout_no_wrap(
				role.name.clone(),
				egui::FontId::proportional(12.0),
				theme.text,
			);
			let size = vec2(*width, galley.size().y + 6.0);
			let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
			ui.painter().rect_filled(rect, 6, theme.chip);
			let color = if role.color == 0 {
				theme.muted
			} else {
				rgb(role.color)
			};
			ui.painter()
				.circle_filled(pos2(rect.left() + 10.0, rect.center().y), 4.0, color);
			ui.painter().with_clip_rect(rect.shrink(3.0)).galley(
				pos2(rect.left() + 18.0, rect.center().y - galley.size().y * 0.5),
				galley,
				theme.text,
			);
			response.on_hover_text(&role.name);
		}
		let hidden = roles.len() - visible;
		if hidden > 0 {
			let label = format!("+{hidden}");
			let galley = ui.painter().layout_no_wrap(
				label.clone(),
				egui::FontId::proportional(12.0),
				theme.text,
			);
			let (rect, response) = ui.allocate_exact_size(
				vec2(galley.size().x + 16.0, galley.size().y + 6.0),
				egui::Sense::click(),
			);
			ui.painter().rect_filled(rect, 6, theme.chip);
			ui.painter().galley(
				pos2(
					rect.center().x - galley.size().x * 0.5,
					rect.center().y - galley.size().y * 0.5,
				),
				galley,
				theme.text,
			);
			response
				.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone()));
			if response
				.on_hover_text(crate::i18n::translate(
					"profiles-role-chips-show-remaining-roles",
				))
				.clicked()
			{
				ui.data_mut(|data| data.insert_temp(expanded_id, true));
			}
		}
	});
}

pub(crate) fn profile_opener_id() -> egui::Id {
	egui::Id::unique("serein-profile-opener")
}

#[derive(Default)]
pub struct ProfileSession {
	open: Option<User>,
	anchor: Option<(Id, Pos2)>,
	trigger: Option<Rect>,
	pending: Vec<ProfileEffect>,
	message_target: Option<(u64, Id)>,
	message_draft: String,
	message_pending: bool,
	message_ime: bool,
	/// Last laid-out composer height, so the scrollable details leave room for it.
	message_height: f32,
	bio_identity: Option<egui::Id>,
	bio_expanded: bool,
	bio_revealed: u32,
}

pub enum ProfileEffect {
	ClearCore,
}

impl ProfileSession {
	fn reset_card_input(&mut self) {
		self.message_target = None;
		self.message_draft = String::new();
		self.message_pending = false;
		self.message_ime = false;
		self.bio_identity = None;
		self.bio_expanded = false;
		self.bio_revealed = 0;
	}

	fn reconcile_card_input(&mut self, generation: u64, user: Id) {
		if self.message_target != Some((generation, user)) {
			self.reset_card_input();
			self.message_target = Some((generation, user));
		}
	}

	/// A rejected send keeps the text available for an explicit retry. Accepting it means the
	/// existing message pipeline has retained its own copy, so the footer can release its draft.
	pub fn finish_message(&mut self, user: Id, accepted: bool) {
		if self
			.message_target
			.is_some_and(|(_, target)| target == user)
		{
			self.message_pending = false;
			if accepted {
				self.message_draft = String::new();
			}
		}
	}

	pub fn open_user(&self) -> Option<&User> {
		self.open.as_ref()
	}

	pub fn anchor_or_place(&mut self, ctx: &egui::Context, user_id: Id) -> Pos2 {
		match self.anchor {
			Some((id, pos)) if id == user_id => pos,
			_ => {
				let pos = ctx
					.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()))
					.unwrap_or_else(|| ctx.content_rect().center());
				self.anchor = Some((user_id, pos));
				pos
			}
		}
	}

	pub fn disarm(&mut self) {
		self.trigger = None;
	}

	pub fn person_click(
		&mut self,
		ui: &egui::Ui,
		primary: &egui::Response,
		nested: Option<&egui::Response>,
		user: &User,
	) {
		let arm = match nested {
			Some(nested) if nested.contains_pointer() => Some(nested),
			_ if primary.contains_pointer() => Some(primary),
			_ => None,
		};
		if let Some(response) = arm {
			ui.data_mut(|data| data.insert_temp(profile_opener_id(), response.rect));
		}
		let clicked = match nested {
			Some(nested) if nested.clicked() => true,
			_ if primary.clicked() => true,
			_ => false,
		};
		if !clicked {
			return;
		}
		if self.open.as_ref().is_some_and(|open| open.id == user.id) {
			self.hide();
			return;
		}
		if self.open.as_ref().is_some_and(|open| open.id != user.id) {
			self.pending.push(ProfileEffect::ClearCore);
		}
		self.reset_card_input();
		self.open = Some(user.clone());
	}

	pub fn command_open(&mut self, user: User) {
		if self.open.as_ref().is_none_or(|open| open.id != user.id) {
			self.reset_card_input();
		}
		if self.open.as_ref().is_some_and(|open| open.id != user.id) {
			self.pending.push(ProfileEffect::ClearCore);
		}
		self.open = Some(user);
	}

	pub fn navigate(&mut self, user: User) {
		self.reset_card_input();
		self.pending.push(ProfileEffect::ClearCore);
		self.open = Some(user);
		self.anchor = None;
	}

	/// Drops the card and keeps the loaded profile.
	pub fn hide(&mut self) {
		self.reset_card_input();
		self.open = None;
		self.anchor = None;
		self.trigger = None;
	}

	pub fn close(&mut self) {
		if self.open.is_some() {
			self.pending.push(ProfileEffect::ClearCore);
		}
		self.hide();
	}

	pub fn close_unless_armed(&mut self, ctx: &egui::Context) {
		let keep = self.trigger.is_some_and(|rect| {
			ctx.input(|input| {
				input.pointer.any_pressed()
					&& input
						.pointer
						.interact_pos()
						.is_some_and(|pos| rect.contains(pos))
			})
		});
		if !keep {
			self.close();
		}
	}

	pub fn ingest_opener_rect(&mut self, ctx: &egui::Context) {
		if let Some(rect) = ctx.data(|data| data.get_temp::<Rect>(profile_opener_id())) {
			self.trigger = Some(rect);
		}
	}

	pub fn drain_effects(&mut self) -> Vec<ProfileEffect> {
		std::mem::take(&mut self.pending)
	}
}

/// Compact message composer; Enter sends, Shift+Enter adds a line. It retains one bounded
/// draft until the caller accepts the send.
fn message_input(
	ui: &mut egui::Ui,
	user: &User,
	state: &State,
	session: &mut ProfileSession,
	theme: &Theme,
) -> Option<Action> {
	let id = egui::Id::unique(("profile-message-input", state.generation, user.id));
	let enabled = state.can_open_user_dm(user) && !session.message_pending;
	let focused = ui.ctx().memory(|memory| memory.has_focus(id));
	let ime_this_frame = session.message_ime
		|| (focused
			&& ui.input(|input| {
				input.events.iter().any(|event| match event {
					egui::Event::Ime(
						egui::ImeEvent::Preedit { text, .. } | egui::ImeEvent::Commit(text),
					) => !text.is_empty(),
					egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { .. }) => true,
					_ => false,
				})
			}));
	if focused || session.message_ime {
		ui.input(|input| {
			for event in &input.events {
				match event {
					egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
						session.message_ime = !text.is_empty();
					}
					egui::Event::Ime(egui::ImeEvent::Commit(_)) => session.message_ime = false,
					_ => {}
				}
			}
		});
	}
	// Plain Enter is always the send key here, even while a send is pending; only Shift+Enter
	// inserts a line break.
	let enter = focused
		&& !ime_this_frame
		&& !ui.input(|input| {
			input.events.iter().any(|event| {
				matches!(
					event,
					egui::Event::Key {
						key: egui::Key::Enter,
						pressed: true,
						repeat: true,
						..
					}
				)
			})
		}) && ui.input(|input| {
		input.events.iter().any(|event| {
			matches!(event, egui::Event::Key {
					key: egui::Key::Enter, pressed: true, repeat: false, modifiers, ..
				} if modifiers.is_none())
		})
	}) && ui
		.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
		&& enabled;
	let placeholder =
		crate::i18n::translate_args("profiles-message-placeholder", &[("user", &user.name)]);
	let colors = design::palette(ui);
	let can_send = enabled && !ime_this_frame && !session.message_draft.trim().is_empty();
	let mut clicked = false;
	let top = ui.cursor().top();
	egui::Frame::new()
		// Profile theme colours keep the composer legible over bright or saturated gradients.
		.fill(theme.panel)
		.stroke(Stroke::new(
			1.0,
			// A quiet focus cue in the profile's own text colour; accent blue clashes with themes.
			if focused { theme.muted } else { theme.border },
		))
		.corner_radius(10)
		.inner_margin(egui::Margin {
			left: 12,
			right: 5,
			top: 5,
			bottom: 5,
		})
		.show(ui, |ui| {
			ui.horizontal(|ui| {
				let button = 30.0;
				let edit_width =
					(ui.available_width() - button - ui.spacing().item_spacing.x).max(1.0);
				let row = ui.text_style_height(&egui::TextStyle::Body);
				// Shrinks to one line and scrolls past four; the row stays as tall as the button.
				egui::ScrollArea::vertical()
					.id_salt(("profile-message-scroll", user.id))
					.max_width(edit_width)
					.max_height(row * 4.0 + button - row)
					.min_scrolled_height(button)
					.auto_shrink([false, true])
					.stick_to_bottom(true)
					.show(ui, |ui| {
						ui.add_enabled(
							enabled,
							egui::TextEdit::multiline(&mut session.message_draft)
								.id(id)
								.char_limit(client_core::MAX_CONTENT)
								.desired_width(edit_width)
								.desired_rows(1)
								.margin(vec2(0.0, ((button - row) / 2.0).max(0.0)))
								.frame(egui::Frame::NONE)
								.text_color(theme.text)
								.hint_text(RichText::new(placeholder).color(theme.muted)),
						);
					});
				let (rect, response) = ui.allocate_exact_size(
					Vec2::splat(button),
					if can_send {
						egui::Sense::click()
					} else {
						egui::Sense::hover()
					},
				);
				let label = crate::i18n::translate("profiles-message-send");
				if session.message_pending {
					egui::Spinner::new()
						.size(16.0)
						.color(theme.muted)
						.paint_at(ui, rect.shrink(7.0));
				} else {
					if can_send {
						ui.painter().circle_filled(
							rect.center(),
							button / 2.0,
							if response.hovered() {
								colors.accent.gamma_multiply(0.85)
							} else {
								colors.accent
							},
						);
					}
					icons::paint(
						ui.painter(),
						Icon::Send,
						rect.shrink(8.0),
						if can_send {
							colors.accent_text
						} else {
							theme.muted.gamma_multiply(0.7)
						},
					);
				}
				if can_send {
					response
						.clone()
						.on_hover_cursor(egui::CursorIcon::PointingHand);
				}
				response.widget_info(|| {
					egui::WidgetInfo::labeled(egui::Role::Button, can_send, &label)
				});
				clicked = response.on_hover_text(label).clicked();
			});
		});
	// One quiet status line: progress, why sending is unavailable, or the length budget.
	let length = session.message_draft.chars().count();
	let hint = if session.message_pending {
		Some((
			crate::i18n::translate("profiles-message-sending"),
			theme.muted,
		))
	} else if !state.can_open_user_dm(user) {
		Some((
			crate::i18n::translate(if state.user_action_pending() {
				"profiles-message-busy"
			} else {
				"profiles-message-offline"
			}),
			theme.text,
		))
	} else if length + 200 > client_core::MAX_CONTENT {
		Some((
			format!("{length} / {}", client_core::MAX_CONTENT),
			if length >= client_core::MAX_CONTENT {
				colors.danger
			} else {
				theme.muted
			},
		))
	} else {
		None
	};
	if let Some((text, color)) = hint {
		ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).truncate());
	}
	session.message_height = ui.cursor().top() - top;
	// TextEdit caps Unicode characters, which also bounds UTF-8 bytes to four per character.
	if session.message_draft.capacity() > client_core::MAX_CONTENT * 4 {
		session.message_draft.shrink_to_fit();
	}
	if (enter || clicked) && !session.message_pending && !session.message_draft.trim().is_empty() {
		session.message_pending = true;
		Some(Action::SendMessage {
			user: user.clone(),
			content: session.message_draft.clone(),
		})
	} else {
		None
	}
}

/// Render the original markup into a clipped three-line preview. Truncating source could turn
/// an unfinished spoiler or Markdown link into visible text or a different destination.
#[allow(clippy::too_many_arguments)]
fn biography(
	ui: &mut egui::Ui,
	user: Id,
	bio: &str,
	state: &State,
	avatars: &mut Avatars,
	opening: &mut Option<String>,
	formatted: &mut FormatCache,
	session: &mut ProfileSession,
) -> Option<User> {
	let identity = egui::Id::unique((user, bio));
	if session.bio_identity != Some(identity) {
		session.bio_identity = Some(identity);
		session.bio_expanded = false;
		session.bio_revealed = 0;
	}
	let preview_height = 3.0 * ui.text_style_height(&egui::TextStyle::Body);
	let mut linked = ProfileSession::default();
	let mut render = |ui: &mut egui::Ui| {
		let mut surface = crate::select::Surface::new(ui, "profile-bio");
		formatted.get(user, bio).show_references(
			ui,
			opening,
			&[],
			None,
			&mut linked,
			(&[], &mut None, &state.guilds, &[]),
			(avatars, state.demo, &mut session.bio_revealed),
			&mut surface,
			crate::design::MessageCardSurface::Opaque,
		);
		surface.finish(ui);
	};
	let overflowing = if session.bio_expanded {
		render(ui);
		true
	} else {
		let clip = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), preview_height));
		let mut preview = ui.new_child(
			UiBuilder::new()
				.id_salt("profile-bio-preview")
				.max_rect(clip),
		);
		preview.set_clip_rect(ui.clip_rect().intersect(clip));
		render(&mut preview);
		let height = preview.min_rect().height();
		ui.allocate_space(vec2(ui.available_width(), height.min(preview_height)));
		height > preview_height + 0.5 || bio.chars().count() > 200
	};
	if overflowing {
		let label = if session.bio_expanded {
			"profiles-show-hide-full-bio"
		} else {
			"profiles-show-view-full-bio"
		};
		if ui.small_button(crate::i18n::translate(label)).clicked() {
			session.bio_expanded = !session.bio_expanded;
		}
	}
	linked.open_user().cloned()
}

/// Shows the popout beside `anchor`; returns an action when the card wants to change or close.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
#[cfg(test)]
pub fn show(
	ui: &mut egui::Ui,
	user: &User,
	view: Option<&ProfileView>,
	state: &State,
	avatars: &mut Avatars,
	opening: &mut Option<String>,
	formatted: &mut FormatCache,
	confirm_links: bool,
	anchor: Pos2,
) -> Option<Action> {
	show_with_session(
		ui,
		user,
		view,
		state,
		avatars,
		opening,
		formatted,
		confirm_links,
		anchor,
		&mut ProfileSession::default(),
	)
}

/// Shows a card with one session-scoped footer draft and explicit biography expansion state.
#[allow(clippy::too_many_arguments)]
pub fn show_with_session(
	ui: &mut egui::Ui,
	user: &User,
	view: Option<&ProfileView>,
	state: &State,
	avatars: &mut Avatars,
	opening: &mut Option<String>,
	formatted: &mut FormatCache,
	confirm_links: bool,
	anchor: Pos2,
	session: &mut ProfileSession,
) -> Option<Action> {
	session.reconcile_card_input(state.generation, user.id);
	let colors = design::palette(ui);
	let viewport = ui.ctx().content_rect();
	let bounds = viewport.shrink(8.0);
	let view = view.filter(|_| !user.webhook);
	let data = view.and_then(|v| v.data.as_ref());
	let theme = Theme::new(&colors, data.and_then(|d| d.theme_colors));
	let guild = state
		.selected
		.and_then(|id| state.channels.iter().find(|c| c.id == id))
		.and_then(|c| c.guild);
	let (status, custom, activities, clients) = if user.webhook {
		(None, None, [].as_slice(), model::ClientPlatforms::default())
	} else {
		presence(state, user.id, guild)
	};
	let in_voice = !user.webhook && user_in_voice(state, user.id);
	let dm_channel = state
		.channels
		.iter()
		.find(|c| c.kind == 1 && c.recipients.iter().any(|u| u.id == user.id))
		.map(|c| c.id);
	let mut action = None;
	let menu_id = egui::Id::unique(("user-profile-more", user.id));
	// A menu that was already open owns Escape and clicks on its own items this frame.
	let menu_open = egui::Popup::is_id_open(ui.ctx(), menu_id);
	// The mutuals side panel stays open across frames until toggled, dismissed or navigated.
	let mutuals_id = egui::Id::unique(("profile-mutuals", user.id));
	let mut mutuals = ui.ctx().data(|d| d.get_temp::<Mutuals>(mutuals_id));
	let mut mutual_anchor = None;
	let mut mutual_links = Vec::new();
	let x = if anchor.x + 12.0 + WIDTH <= bounds.right() {
		anchor.x + 12.0
	} else {
		(anchor.x - 12.0 - WIDTH).max(bounds.left())
	};
	let response = egui::Area::new(egui::Id::unique("user-profile-popout"))
		.kind(egui::UiKind::Popup)
		.order(egui::Order::Foreground)
		.fixed_pos(pos2(x, (anchor.y - 40.0).max(bounds.top())))
		.constrain_to(bounds)
		.interactable(true)
		.show(ui.ctx(), |ui| {
			ui.set_width(WIDTH);
			ui.set_max_width(WIDTH);
			// Area remembers its previous size; let details grow beyond a short prior profile.
			ui.set_max_height(bounds.height());
			ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
			// Cross-label drag selection paints stray highlights in this dense card.
			ui.style_mut().interaction.selectable_labels = false;
			let background = ui.painter().add(egui::Shape::Noop);
			{
				// egui resolves strong, button and spinner colors from the widget strokes rather
				// than the override, so a bright theme needs every stroke recolored too.
				let visuals = ui.visuals_mut();
				visuals.override_text_color = Some(theme.text);
				visuals.hyperlink_color = theme.link;
				let widgets = &mut visuals.widgets;
				for widget in [
					&mut widgets.noninteractive,
					&mut widgets.inactive,
					&mut widgets.hovered,
					&mut widgets.active,
					&mut widgets.open,
				] {
					widget.fg_stroke.color = theme.text;
					widget.bg_stroke = Stroke::NONE;
				}
				widgets.inactive.weak_bg_fill = theme.chip;
				widgets.hovered.weak_bg_fill = theme.chip_hover;
				widgets.active.weak_bg_fill = theme.chip_hover;
			}

			// Header: banner, overlapping avatar with presence, badge pill.
			let has_banner = data.as_ref().and_then(|d| d.banner_url()).is_some();
			let sense = if has_banner {
				egui::Sense::click()
			} else {
				egui::Sense::hover()
			};
			let (banner, banner_response) = ui.allocate_exact_size(vec2(WIDTH, 105.0), sense);
			let top_corners = CornerRadius {
				nw: RADIUS,
				ne: RADIUS,
				sw: 0,
				se: 0,
			};
			if let Some(data) = data {
				avatars.paint_banner(ui, data, banner, top_corners, state.demo);
			} else {
				ui.painter().rect_filled(banner, top_corners, colors.raised);
			}
			// Action circles in the banner's top-right corner: add friend, then the overflow menu.
			let circles = Rect::from_min_size(
				pos2(
					banner.right() - PAD - 2.0 * CIRCLE - 8.0,
					banner.top() + PAD,
				),
				vec2(2.0 * CIRCLE + 8.0, CIRCLE),
			);
			ui.scope_builder(
				UiBuilder::new()
					.max_rect(circles)
					.layout(egui::Layout::right_to_left(egui::Align::Center)),
				|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					let more = header_circle(ui, Icon::More, "profiles-show-more", true);
					egui::Popup::menu(&more).id(menu_id).show(|ui| {
						if let Some(picked) = more_menu(ui, state, user, dm_channel) {
							action = Some(picked);
						}
					});
					if let Some(friend_action) = friend_circle(ui, state, user) {
						action = Some(friend_action);
					}
				},
			);
			let avatar_rect = Rect::from_min_size(
				banner.left_bottom() + vec2(PAD + 4.0, -AVATAR * 0.5 - 6.0),
				Vec2::splat(AVATAR),
			);
			if has_banner {
				let pointer_in_subwidgets = ui.input(|i| {
					i.pointer
						.hover_pos()
						.is_some_and(|pos| circles.contains(pos) || avatar_rect.contains(pos))
				});
				let banner_response = if !pointer_in_subwidgets {
					banner_response
						.on_hover_cursor(egui::CursorIcon::ZoomIn)
						.on_hover_text(crate::i18n::translate("profiles-show-view-banner"))
				} else {
					banner_response
				};
				banner_response.widget_info(|| {
					egui::WidgetInfo::labeled(
						egui::Role::Button,
						true,
						crate::i18n::translate("profiles-show-view-banner"),
					)
				});
				let pointer_interact_in_subwidgets = ui.input(|i| {
					i.pointer
						.interact_pos()
						.is_some_and(|pos| circles.contains(pos) || avatar_rect.contains(pos))
				});
				if banner_response.clicked()
					&& !pointer_interact_in_subwidgets
					&& let Some(data) = data
					&& let Some(url) = data.banner_url()
				{
					action = Some(Action::Banner(model::EmbedMedia {
						url: Some(url),
						width: 2048,
						height: 1024,
						..Default::default()
					}));
				}
			}
			ui.painter().circle_filled(
				avatar_rect.center(),
				AVATAR * 0.5 + AVATAR_RING,
				theme.card,
			);
			let pointer_on_presence = pointer_on_presence(
				status,
				avatar_rect,
				ui.input(|input| input.pointer.hover_pos()),
			);
			ui.scope_builder(UiBuilder::new().max_rect(avatar_rect), |ui| {
				let mut response = avatars.with_avatar_animation(true, |avatars| {
					if let Some(data) = data {
						avatars.show_profile_avatar(ui, data, AVATAR, state.demo)
					} else {
						avatars.show(ui, user, AVATAR, state.demo)
					}
				});
				response.widget_info(|| {
					egui::WidgetInfo::labeled(
						egui::Role::Button,
						true,
						crate::i18n::translate("profiles-show-view-profile-picture"),
					)
				});
				if !pointer_on_presence {
					response = response
						.on_hover_cursor(egui::CursorIcon::ZoomIn)
						.on_hover_text(crate::i18n::translate(
							"profiles-show-view-profile-picture",
						));
				}
				if response.clicked() && !pointer_on_presence {
					let mut url = data.map_or(user, |data| &data.user).avatar_url();
					if let Some(data) = data
						&& let Some(member) = data.guild.as_ref()
						&& let Some(hash) = member
							.avatar
							.as_deref()
							.filter(|hash| model::valid_avatar_hash(hash))
					{
						let ext = if hash.starts_with("a_") { "gif" } else { "png" };
						url = format!(
							"https://cdn.discordapp.com/guilds/{}/users/{}/avatars/{hash}.{ext}?size=128",
							member.guild, data.user.id
						);
					}
					action = Some(Action::Avatar(model::EmbedMedia {
						url: Some(url.replace("size=128", "size=2048")),
						width: 2048,
						height: 2048,
						..Default::default()
					}));
				}
			});
			if let Some(status) = status {
				presence_badge(ui, avatar_rect, status, clients, theme.card);
			}
			let mut header_bottom = avatar_rect.bottom();
			let (icon_badges, text_badges): (Vec<_>, Vec<_>) = data
				.map(|d| d.badges.iter().partition(|b| b.icon.is_some()))
				.unwrap_or_default();
			if !icon_badges.is_empty() {
				const BADGE: f32 = 22.0;
				let right = banner.right() - PAD;
				let count = icon_badges.len() as f32;
				let width = (count * BADGE + (count - 1.0) * 4.0 + 12.0)
					.min(right - avatar_rect.right() - 12.0);
				let pill = Rect::from_min_max(
					pos2(right - width, banner.bottom() + 8.0),
					pos2(right, banner.bottom() + 200.0),
				);
				let response = ui.scope_builder(UiBuilder::new().max_rect(pill), |ui| {
					egui::Frame::new()
						.fill(theme.panel)
						.corner_radius(RADIUS)
						.inner_margin(6)
						.show(ui, |ui| {
							ui.set_width(width - 12.0);
							ui.horizontal_wrapped(|ui| {
								ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
								for badge in &icon_badges {
									avatars
										.show_icon(
											ui,
											badge.icon_key(),
											BADGE,
											state.demo,
											&badge.description,
										)
										.on_hover_text(&badge.description);
								}
							});
						});
				});
				header_bottom = header_bottom.max(response.response.rect.bottom());
			}
			ui.add_space((header_bottom + 10.0 - ui.cursor().top()).max(0.0));

			egui::Frame::new()
				.inner_margin(egui::Margin {
					left: PAD as i8,
					right: PAD as i8,
					top: 0,
					bottom: PAD as i8,
				})
				.show(ui, |ui| {
					ui.spacing_mut().item_spacing.y = 8.0;
					// Reserve space only for footer rows that are actually displayed.
					let message_target = !user.webhook
						&& state.user.as_ref().is_some_and(|own| own.id != user.id)
						&& state.user_blocked(user.id) != Some(true);
					let own = state.user.as_ref().is_some_and(|own| own.id == user.id);
					let has_action = own || message_target || user.webhook;
					let footer = if message_target && !own {
						session.message_height.max(32.0) + 8.0
					} else if has_action {
						40.0
					} else {
						0.0
					};
					egui::Frame::new()
						.fill(theme.panel)
						.corner_radius(RADIUS)
						.inner_margin(12)
						.show(ui, |ui| {
							ui.set_width(ui.available_width());
							ui.spacing_mut().item_spacing = vec2(6.0, 3.0);
							if view.is_some_and(|v| v.error.is_some())
								|| data.is_some_and(|data| data.limited)
							{
								design::notice(
									ui,
									design::Level::Warning,
									&crate::i18n::translate(
										"profiles-show-unable-to-load-parts-of-profile",
									),
								);
								ui.add_space(6.0);
							}
							let display = data
								.and_then(|p| {
									p.guild
										.as_ref()
										.and_then(|g| g.nick.as_deref())
										.or(state.friend_nickname(user.id))
										.or(p.global_name.as_deref())
								})
								.unwrap_or_else(|| state.user_display_name(user));
							let display = display.split_whitespace().collect::<Vec<_>>().join(" ");
							// Ordinary user payloads already carry the server identity. Keep it visible
							// while the extended profile loads or when that optional request fails.
							let clan = data
								.map(|data| data.clan.as_ref())
								.unwrap_or(user.primary_guild.as_deref());
							let tag_width = clan.map_or(0.0, |clan| {
								ui.painter()
									.layout_no_wrap(
										clan.tag.clone(),
										egui::FontId::proportional(12.0),
										theme.text,
									)
									.size()
									.x + 38.0
							});
							ui.horizontal(|ui| {
								ui.spacing_mut().item_spacing.x = 8.0;
								ui.allocate_ui_with_layout(
									vec2((ui.available_width() - tag_width).max(0.0), 24.0),
									egui::Layout::left_to_right(egui::Align::Center),
									|ui| {
										ui.add(
											egui::Label::new(
												RichText::new(display).size(20.0).strong(),
											)
											.truncate(),
										);
									},
								);
								if let Some(clan) = clan {
									egui::Frame::new()
										.fill(theme.chip)
										.corner_radius(6)
										.inner_margin(egui::Margin::symmetric(6, 2))
										.show(ui, |ui| {
											ui.spacing_mut().item_spacing.x = 4.0;
											avatars.show_icon(
												ui,
												clan.badge_key(),
												14.0,
												state.demo,
												"Server tag badge",
											);
											ui.add(
												egui::Label::new(
													RichText::new(&clan.tag).size(12.0).strong(),
												)
												.extend(),
											);
										})
										.response
										.on_hover_text(format!(
											"{} · {} {}",
											crate::i18n::translate("profiles-show-server-tag"),
											crate::i18n::translate("profiles-show-server"),
											clan.guild
										));
								}
							});
							let mut identity = Vec::new();
							if let Some(data) = data {
								identity.push(if data.user.discriminator > 0 {
									format!("{}#{:04}", data.username, data.user.discriminator)
								} else {
									data.username.clone()
								});
								let pronouns = data
									.guild
									.as_ref()
									.map(|g| g.pronouns.as_str())
									.filter(|s| !s.is_empty())
									.unwrap_or(&data.pronouns);
								if !pronouns.is_empty() {
									identity.push(pronouns.to_owned());
								}
							} else if user.webhook {
								identity.push("Webhook".into());
							} else {
								identity.push(user.name.clone());
							}
							ui.add(
								egui::Label::new(
									RichText::new(identity.join(" • "))
										.size(14.0)
										.color(theme.muted),
								)
								.truncate(),
							);
							if !text_badges.is_empty() {
								ui.add_space(2.0);
								ui.horizontal_wrapped(|ui| {
									ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
									for badge in &text_badges {
										egui::Frame::new()
											.fill(theme.chip)
											.corner_radius(6)
											.inner_margin(egui::Margin::symmetric(6, 2))
											.show(ui, |ui| {
												ui.label(
													RichText::new(&badge.description).size(11.0),
												);
											})
											.response
											.on_hover_text(&badge.id);
									}
								});
							}
							if let Some(data) = data.filter(|data| {
								state.user.as_ref().is_none_or(|own| own.id != user.id)
									&& (!data.mutual_guilds.is_empty()
										|| !data.mutual_friends.is_empty())
							}) {
								ui.add_space(4.0);
								ui.horizontal(|ui| {
									ui.spacing_mut().item_spacing.x = 16.0;
									for (kind, count, icon, one, many) in [
										(
											Mutuals::Servers,
											data.mutual_guilds.len(),
											Icon::Servers,
											"profiles-show-mutual-server",
											"profiles-show-mutual-servers",
										),
										(
											Mutuals::Friends,
											data.mutual_friends.len(),
											Icon::People,
											"profiles-show-mutual-friend",
											"profiles-show-mutual-friends",
										),
									] {
										if count == 0 {
											continue;
										}
										let key = if count == 1 { one } else { many };
										let link = mutual_link(
											ui,
											&theme,
											icon,
											format!("{count} {}", crate::i18n::translate(key)),
										);
										if link.clicked() {
											mutuals = (mutuals != Some(kind)).then_some(kind);
										}
										if mutuals == Some(kind) {
											mutual_anchor = Some(link.rect);
										}
										mutual_links.push(link.rect);
									}
								});
							}
							if custom.is_some() || in_voice {
								ui.add_space(4.0);
								ui.horizontal(|ui| {
									ui.spacing_mut().item_spacing.x = 4.0;
									voice_badge(ui, in_voice, custom.is_some());
									if let Some(custom) = custom {
										ui.add(
											egui::Label::new(RichText::new(custom).size(13.0))
												.wrap(),
										);
									}
								});
							}
							if !user.webhook && view.is_none_or(|v| v.loading) {
								ui.add_space(4.0);
								ui.horizontal(|ui| {
									ui.spinner();
									ui.label(
										RichText::new(crate::i18n::translate(
											"profiles-show-loading-profile",
										))
										.size(13.0)
										.color(theme.muted),
									);
								});
							}
							if let Some(error) = view.and_then(|v| v.error) {
								ui.add_space(4.0);
								if ui
									.small_button(crate::i18n::translate(
										"profiles-show-retry-profile",
									))
									.on_hover_text(error)
									.clicked()
								{
									action = Some(Action::Retry);
								}
							}
							if data.is_some() || !activities.is_empty() {
								divider(ui, &theme);
								let used = ui.cursor().top() - banner.top();
								// No floor here: a busy profile (many badges, connections, a long
								// bio) must still fit `bounds`, or the card's rounded bottom
								// corner renders past the window edge and looks square.
								let max_height = (bounds.height() - used - footer - 48.0).max(0.0);
								egui::ScrollArea::vertical()
									.id_salt("profile-details")
									.max_height(max_height)
									.auto_shrink([false, true])
									.show(ui, |ui| {
										ui.spacing_mut().item_spacing.y = 4.0;
										let mut sections = 0;
										if !activities.is_empty() {
											sections += 1;
											activity_list(
												ui,
												egui::Id::unique(("profile-activity", user.id)),
												activities,
												avatars,
												state.demo,
												(theme.chip, theme.muted),
											);
										}
										if let Some(data) = data {
											let bio = data
												.guild
												.as_ref()
												.map(|g| g.bio.as_str())
												.filter(|s| !s.is_empty())
												.unwrap_or(&data.bio);
											if !bio.is_empty() {
												section(
													ui,
													&theme,
													&mut sections,
													"profiles-show-about-me",
												);
												if let Some(next) = biography(
													ui, user.id, bio, state, avatars, opening,
													formatted, session,
												) {
													action = Some(Action::Profile(next));
												}
											}
											if let Some(guild) = data.guild.as_ref()
												&& state.guild_roles(guild.guild).is_some_and(
													|roles| {
														roles.iter().any(|role| {
															role.id != guild.guild
																&& guild.roles.contains(&role.id)
														})
													},
												) {
												section(
													ui,
													&theme,
													&mut sections,
													"profiles-show-roles",
												);
												role_chips(ui, &theme, state, data.user.id, guild);
											}
											section(
												ui,
												&theme,
												&mut sections,
												"profiles-show-member-since",
											);
											ui.horizontal_wrapped(|ui| {
												ui.spacing_mut().item_spacing.x = 6.0;
												if let Some(date) = creation_date(user.id) {
													icons::inline(
														ui,
														Icon::Calendar,
														16.0,
														theme.muted,
													);
													ui.label(RichText::new(date).size(13.0));
												}
												if let Some(joined) = data
													.guild
													.as_ref()
													.and_then(|g| g.joined_at.as_deref())
												{
													let server =
														data.guild
															.as_ref()
															.and_then(|g| {
																state.guilds.iter().find(|known| {
																	known.id == g.guild
																})
															})
															.map_or_else(
																|| {
																	crate::i18n::translate(
																		"profiles-show-server-2",
																	)
																},
																|g| g.name.clone(),
															);
													ui.label(
														RichText::new("•")
															.size(13.0)
															.color(theme.muted),
													);
													ui.label(
														RichText::new(format!(
															"{server} {}",
															joined
																.split('T')
																.next()
																.unwrap_or(joined)
														))
														.size(13.0),
													);
												}
											});
											if !data.connections.is_empty() {
												ui.add_space(10.0);
												connection_icons(
													ui,
													&theme,
													&data.connections,
													opening,
												);
											}
										}
									});
							}
						});
					// Footer: one full-width primary action when available.
					if state.user.as_ref().is_some_and(|own| own.id == user.id) {
						if ui
							.add_sized(
								[ui.available_width(), 32.0],
								egui::Button::new(
									RichText::new(crate::i18n::translate(
										"profiles-show-edit-profile",
									))
									.color(colors.accent_text),
								)
								.fill(colors.accent)
								.stroke(Stroke::NONE)
								.corner_radius(RADIUS),
							)
							.clicked()
						{
							action = Some(Action::Edit);
						}
					} else if message_target {
						if let Some(submitted) = message_input(ui, user, state, session, &theme) {
							action = Some(submitted);
						}
					} else if user.webhook
						&& ui
							.add_sized(
								[ui.available_width(), 32.0],
								egui::Button::new(
									RichText::new(crate::i18n::translate(
										"profiles-show-copy-webhook-id",
									))
									.size(13.0)
									.strong(),
								)
								.corner_radius(RADIUS),
							)
							.clicked()
					{
						ui.ctx().copy_text(user.id.to_string());
					}
					if state.demo {
						ui.label(
							RichText::new(crate::i18n::translate(
								"profiles-show-offline-preview-synthetic",
							))
							.size(11.0)
							.color(theme.muted),
						);
					}
				});
			let rect = ui.min_rect();
			ui.painter().set(background, theme.background(rect));
		});
	let rect = response.response.rect;
	let mut panel = None;
	if let (Some(kind), Some(link), Some(data)) = (mutuals, mutual_anchor, data) {
		let (panel_rect, panel_action) = mutuals_panel(
			ui.ctx(),
			kind,
			data,
			state,
			avatars,
			&theme,
			(rect, link, bounds),
			user.id,
		);
		panel = Some(panel_rect);
		action = action.or(panel_action);
	} else {
		mutuals = None;
	}
	let pressed = ui.ctx().input(|i| {
		i.pointer
			.any_pressed()
			.then(|| i.pointer.interact_pos())
			.flatten()
	});
	let in_panel = |pos: Pos2| panel.is_some_and(|panel| panel.contains(pos));
	let pressed_outside =
		!menu_open && pressed.is_some_and(|pos| !rect.contains(pos) && !in_panel(pos));
	if pressed.is_some_and(|pos| {
		!in_panel(pos) && !mutual_links.iter().any(|link: &Rect| link.contains(pos))
	}) {
		mutuals = None;
	}
	// Escape closes the side panel before the card.
	let panel_escape = mutuals.is_some()
		&& ui
			.ctx()
			.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
	if panel_escape || action.is_some() {
		mutuals = None;
	}
	ui.ctx().data_mut(|d| match mutuals {
		Some(kind) => {
			d.insert_temp(mutuals_id, kind);
		}
		None => d.remove::<Mutuals>(mutuals_id),
	});
	let escape = opening.is_none()
		&& !menu_open
		&& !panel_escape
		&& ui
			.ctx()
			.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
	if (pressed_outside || escape) && opening.is_none() && action.is_none() {
		action = Some(Action::Close);
	}
	crate::markdown::confirm_external_link(ui.ctx(), opening, confirm_links);
	action
}

// Explicit offline-only data; never a fallback for a failed service request.
#[cfg(any(test, feature = "demo"))]
pub fn synthetic(user: &User, guild: Option<Id>) -> model::UserProfile {
	let hash = |c: char| c.to_string().repeat(32);
	model::UserProfile {
        user: user.clone(),
        username: "serein.preview".into(),
        global_name: Some(user.name.clone()),
        banner: Some(hash('a')),
        accent_color: Some(0x315c68),
        bio: "Building a quieter place for conversations.\n**Native profile preview** · all details here are synthetic.".into(),
        pronouns: "they / them".into(),
        badges: vec![
            model::ProfileBadge {
                id: "preview_one".into(),
                description: "Synthetic badge one".into(),
                icon: Some(hash('c')),
            },
            model::ProfileBadge {
                id: "preview_two".into(),
                description: "Synthetic badge two".into(),
                icon: Some(hash('d')),
            },
            model::ProfileBadge {
                id: "preview_text".into(),
                description: "Text badge".into(),
                icon: None,
            },
        ],
        connections: vec![model::ProfileConnection {
            kind: "github".into(),
            id: "1".into(),
            name: "synthetic-profile".into(),
            verified: true,
        }],
		mutual_guilds: guild
            .map(|id| vec![model::ProfileGuild { id, nick: None }])
            .unwrap_or_default(),
		mutual_friends: guild
			.map(|_| {
				vec![User {
					id: Id(42),
					name: "Mutual friend".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				}]
			})
			.unwrap_or_default(),
		guild: guild.map(|guild| model::GuildProfile {
			guild,
			roles: vec![],
			nick: None,
			avatar: None,
			banner: None,
			bio: String::new(),
			pronouns: String::new(),
			joined_at: Some("2026-01-01T00:00:00Z".into()),
		}),
        theme_colors: Some([0x1f3a4d, 0x3b2a5e]),
        clan: Some(model::ClanTag {
            guild: guild.unwrap_or(Id(10)),
            tag: "SRN".into(),
            badge: Some(hash('b')),
        }),
        limited: false,
    }
}

#[cfg(test)]
mod tests {
	use super::*;

	fn enter(repeat: bool) -> egui::Event {
		egui::Event::Key {
			key: egui::Key::Enter,
			physical_key: None,
			pressed: true,
			repeat,
			modifiers: egui::Modifiers::NONE,
		}
	}

	#[test]
	fn profile_message_input_submits_nonfriend_once_and_preserves_rejected_text() {
		let state = test_support::demo_state();
		let mut user = test_support::message(1, Id(22)).author;
		user.id = Id(991);
		assert!(state.can_open_user_dm(&user));
		assert!(!state.friends().any(|friend| friend.id == user.id));
		assert!(
			!state
				.channels
				.iter()
				.any(|channel| channel.recipients.iter().any(|u| u.id == user.id))
		);
		let mut session = ProfileSession::default();
		session.reconcile_card_input(state.generation, user.id);
		let ctx = egui::Context::default();
		let id = egui::Id::unique(("profile-message-input", state.generation, user.id));
		let frame = |session: &mut ProfileSession, mut events: Vec<egui::Event>| {
			let mut release = enter(false);
			if let egui::Event::Key { pressed, .. } = &mut release {
				*pressed = false;
			}
			events.insert(0, release);
			ctx.memory_mut(|memory| memory.request_focus(id));
			let mut action = None;
			ctx.run_ui(input(vec2(400.0, 200.0), events), |ui| {
				let theme = Theme::new(&design::palette(ui), None);
				action = message_input(ui, &user, &state, session, &theme);
			})
			.drop_without_applying_deltas();
			action
		};
		assert!(frame(&mut session, vec![egui::Event::Text("hello".into())]).is_none());
		assert_eq!(session.message_draft, "hello");
		assert!(
			matches!(frame(&mut session, vec![enter(false)]), Some(Action::SendMessage { user: target, content }) if target.id == user.id && content == "hello")
		);
		assert!(session.message_pending);
		assert!(frame(&mut session, vec![enter(false)]).is_none());
		session.finish_message(user.id, false);
		assert_eq!(session.message_draft, "hello");
		assert!(matches!(
			frame(&mut session, vec![enter(false)]),
			Some(Action::SendMessage { .. })
		));
		session.finish_message(user.id, true);
		assert!(session.message_draft.is_empty());
		assert!(!session.message_pending);
		assert!(
			frame(
				&mut session,
				vec![egui::Event::Text("   ".into()), enter(false)]
			)
			.is_none()
		);
	}

	#[test]
	fn profile_message_input_bounds_unicode_and_ignores_ime_enter() {
		let state = test_support::demo_state();
		let mut user = test_support::message(1, Id(22)).author;
		user.id = Id(991);
		let mut session = ProfileSession::default();
		session.reconcile_card_input(state.generation, user.id);
		let ctx = egui::Context::default();
		let id = egui::Id::unique(("profile-message-input", state.generation, user.id));
		let frame = |session: &mut ProfileSession, mut events: Vec<egui::Event>| {
			let mut release = enter(false);
			if let egui::Event::Key { pressed, .. } = &mut release {
				*pressed = false;
			}
			events.insert(0, release);
			ctx.memory_mut(|memory| memory.request_focus(id));
			let mut action = None;
			ctx.run_ui(input(vec2(400.0, 200.0), events), |ui| {
				let theme = Theme::new(&design::palette(ui), None);
				action = message_input(ui, &user, &state, session, &theme);
			})
			.drop_without_applying_deltas();
			action
		};
		assert!(
			frame(
				&mut session,
				vec![egui::Event::Text(
					"🦀".repeat(client_core::MAX_CONTENT + 20)
				)]
			)
			.is_none()
		);
		assert_eq!(
			session.message_draft.chars().count(),
			client_core::MAX_CONTENT
		);
		assert!(session.message_draft.capacity() <= client_core::MAX_CONTENT * 4);
		session.message_draft = "hello".into();
		assert!(
			frame(
				&mut session,
				vec![
					egui::Event::Ime(egui::ImeEvent::Preedit {
						text: "ni".into(),
						active_range_chars: None
					}),
					enter(false)
				]
			)
			.is_none()
		);
		assert!(session.message_ime);
		assert!(frame(&mut session, vec![enter(false)]).is_none());
		assert!(
			frame(
				&mut session,
				vec![
					egui::Event::Ime(egui::ImeEvent::Commit("你".into())),
					enter(false)
				]
			)
			.is_none()
		);
		assert!(!session.message_ime);
		assert!(matches!(
			frame(&mut session, vec![enter(false)]),
			Some(Action::SendMessage { .. })
		));
	}

	#[test]
	fn profile_footer_draft_resets_on_target_generation_and_hide() {
		let mut session = ProfileSession::default();
		session.reconcile_card_input(1, Id(2));
		session.message_draft = "first person".into();
		session.reconcile_card_input(1, Id(2));
		assert_eq!(session.message_draft, "first person");
		session.reconcile_card_input(1, Id(3));
		assert!(session.message_draft.is_empty());
		session.message_draft = "second account".into();
		session.reconcile_card_input(2, Id(3));
		assert!(session.message_draft.is_empty());
		session.message_draft = "temporary".into();
		session.message_pending = true;
		session.hide();
		assert!(session.message_draft.is_empty());
		assert!(!session.message_pending);
	}

	#[test]
	fn compact_bio_preserves_spoilers_and_expands_within_the_profile_scroll() {
		let mut state = test_support::demo_state();
		state.selected = None;
		let mut user = test_support::message(1, Id(22)).author;
		user.id = Id(991);
		let mut data = synthetic(&user, None);
		data.bio = format!(
			"First line.\nSecond line.\nThird line.\n{}\nhttps://example.com/full",
			"More biography.\n".repeat(15)
		);
		let profile = ProfileView {
			user: user.id,
			guild: None,
			request: 1,
			loading: false,
			error: None,
			data: Some(data),
		};
		let ctx = egui::Context::default();
		let mut avatars = Avatars::default();
		let mut session = ProfileSession::default();
		let mut formatted = FormatCache::default();
		let size = vec2(500.0, 850.0);
		{
			let mut render = |session: &mut ProfileSession, size| {
				let output = ctx.run_ui(input(size, vec![]), |ui| {
					show_with_session(
						ui,
						&user,
						Some(&profile),
						&state,
						&mut avatars,
						&mut None,
						&mut formatted,
						true,
						pos2(20.0, 40.0),
						session,
					);
				});
				let rect = ctx
					.memory(|memory| memory.area_rect(egui::Id::unique("user-profile-popout")))
					.unwrap();
				output.drop_without_applying_deltas();
				rect
			};
			let mut collapsed = Rect::NOTHING;
			for _ in 0..3 {
				collapsed = render(&mut session, size);
			}
			session.bio_expanded = true;
			let mut expanded = Rect::NOTHING;
			for _ in 0..3 {
				expanded = render(&mut session, size);
			}
			assert!(
				expanded.height() > collapsed.height() + 100.0,
				"collapsed biography must make the card smaller"
			);
			for _ in 0..3 {
				expanded = render(&mut session, vec2(400.0, 500.0));
			}
			assert!(
				expanded.bottom() <= 493.0,
				"expanded details keep the message editor in the viewport"
			);
		}
		let mut painted = String::new();
		let output = ctx.run_ui(input(size, vec![]), |ui| {
			biography(ui, user.id, "before ||private text that must stay hidden even across several wrapped lines and a truncation boundary|| after", &state, &mut avatars, &mut None, &mut formatted, &mut session);
		});
		for shape in &output.shapes {
			text(&shape.shape, &mut painted);
		}
		output.drop_without_applying_deltas();
		assert!(!painted.contains("private text"));
		assert_eq!(session.bio_revealed, 0);
	}

	#[test]
	fn profile_friend_button_requires_confirmation_and_tracks_relationships() {
		use client_core::user_actions::{Action as UserAction, Event};
		let mut person = test_support::message(1, Id(22)).author;
		person.id = Id(2);
		let mut owner = person.clone();
		owner.id = Id(1);
		let mut state = State {
			user: Some(owner),
			demo: true,
			..Default::default()
		};
		for event in [
			Event::Friends(Some(vec![(person.clone(), "synthetic".into())])),
			Event::Requests(Some(vec![])),
		] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::UserAction(event),
			});
		}
		let ctx = egui::Context::default();
		let size = vec2(500.0, 600.0);
		let mut view = crate::MessagingUi::default();
		let mut commands = Vec::new();
		let mut chosen = None;
		let render_button = |events, chosen: &mut Option<Action>| {
			ctx.run_ui(input(size, events), |ui| {
				if let Some(action) = friend_circle(ui, &state, &person) {
					*chosen = Some(action);
				}
			})
		};
		let output = render_button(vec![], &mut chosen);
		let position = output
			.shapes
			.iter()
			.find_map(|s| match &s.shape {
				egui::Shape::Circle(c) if (c.radius - CIRCLE * 0.5).abs() < 0.5 => Some(c.center),
				_ => None,
			})
			.expect("friend circle");
		output.drop_without_applying_deltas();
		let pointer = |position, pressed| {
			vec![
				egui::Event::PointerMoved(position),
				egui::Event::PointerButton {
					pos: position,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				},
			]
		};
		render_button(pointer(position, true), &mut chosen).drop_without_applying_deltas();
		render_button(pointer(position, false), &mut chosen).drop_without_applying_deltas();
		assert!(matches!(chosen, Some(Action::RemoveFriend)));
		assert!(!state.user_action_pending());
		view.friend_removal = Some((state.generation, person.clone()));
		for _ in 0..3 {
			ctx.run_ui(input(size, vec![]), |_| {
				view.confirm_friend_removal(&ctx, &mut state, &mut commands)
			})
			.drop_without_applying_deltas();
		}
		let escape = egui::Event::Key {
			key: egui::Key::Escape,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		};
		ctx.run_ui(input(size, vec![escape]), |_| {
			view.confirm_friend_removal(&ctx, &mut state, &mut commands)
		})
		.drop_without_applying_deltas();
		assert!(view.friend_removal.is_none());
		assert!(commands.is_empty());
		assert_eq!(state.friends().count(), 1);
		view.friend_removal = Some((state.generation, person.clone()));
		let mut confirm_position = None;
		for _ in 0..3 {
			let output = ctx.run_ui(input(size, vec![]), |_| {
				view.confirm_friend_removal(&ctx, &mut state, &mut commands)
			});
			confirm_position = output
				.shapes
				.iter()
				.find_map(|s| match &s.shape {
					egui::Shape::Text(t) if t.galley.job.text == "Remove Friend" => {
						Some(t.pos + t.galley.size() * 0.5)
					}
					_ => None,
				})
				.or(confirm_position);
			output.drop_without_applying_deltas();
		}
		for pressed in [true, false] {
			ctx.run_ui(
				input(size, pointer(confirm_position.unwrap(), pressed)),
				|_| view.confirm_friend_removal(&ctx, &mut state, &mut commands),
			)
			.drop_without_applying_deltas();
		}
		assert!(matches!(
			commands.as_slice(),
			[client_core::Command::UserAction {
				action: UserAction::ProfileFriend {
					user: Id(2),
					friend: false
				},
				..
			}]
		));
		assert!(view.friend_removal.is_none());
		view.friend_removal = Some((state.generation - 1, person));
		view.confirm_friend_removal(&ctx, &mut state, &mut commands);
		assert!(
			view.friend_removal.is_none(),
			"stale account confirmation must be discarded"
		);
	}
	fn text(shape: &egui::Shape, output: &mut String) {
		match shape {
			egui::Shape::Text(s) => output.push_str(&s.galley.job.text),
			egui::Shape::Vec(items) => {
				for shape in items {
					text(shape, output);
				}
			}
			_ => {}
		}
	}
	fn input(size: egui::Vec2, events: Vec<egui::Event>) -> egui::RawInput {
		egui::RawInput {
			screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
			events,
			..Default::default()
		}
	}
	#[test]
	fn partial_profile_warning_preserves_identity_and_webhook_behavior() {
		for webhook in [false, true] {
			for dark in [false, true] {
				for (error, limited) in [
					(Some("Unsupported service response"), false),
					(None, true),
					(None, false),
				] {
					let mut user = test_support::message(1, Id(22)).author;
					user.webhook = webhook;
					let view = ProfileView {
						user: user.id,
						guild: None,
						request: 1,
						loading: false,
						error,
						data: error.is_none().then(|| {
							let mut data = synthetic(&user, None);
							data.limited = limited;
							data
						}),
					};
					let state = State {
						demo: true,
						..Default::default()
					};
					let ctx = egui::Context::default();
					ctx.set_visuals(if dark {
						egui::Visuals::dark()
					} else {
						egui::Visuals::light()
					});
					let mut images = Avatars::default();
					let mut opening = None;
					let mut painted = String::new();
					for _ in 0..3 {
						let output = ctx.run_ui(input(vec2(400.0, 700.0), vec![]), |ui| {
							show(
								ui,
								&user,
								Some(&view),
								&state,
								&mut images,
								&mut opening,
								&mut FormatCache::default(),
								true,
								pos2(20.0, 70.0),
							);
						});
						for shape in &output.shapes {
							text(&shape.shape, &mut painted);
						}
						output.drop_without_applying_deltas();
					}
					assert_eq!(painted.contains("Webhook"), webhook);
					assert_eq!(painted.contains("Copy webhook ID"), webhook);
					assert!(!painted.contains("Copy user ID"));
					// No open DM in this fixture, so non-webhook profiles have no footer action.
					assert!(!painted.contains("Message"));
					assert!(!painted.contains("Unsupported service response"));
					assert_eq!(
						painted.contains("Unable to load parts of profile"),
						!webhook && (error.is_some() || limited)
					);
					assert_eq!(
						painted.contains("Retry profile"),
						!webhook && error.is_some()
					);
					assert!(!painted.contains("Loading profile"));
					assert!(painted.contains(&user.name));
					assert!(images.take_requests().is_empty());
				}
			}
		}
	}
	#[test]
	fn busy_gradient_profile_never_grows_past_the_viewport() {
		// Many badges/connections/mutual servers plus a long bio must still fit the window, or
		// the card's rounded bottom corner renders past the edge and looks clipped square.
		let user = test_support::message(1, Id(22)).author;
		let mut data = synthetic(&user, Some(Id(9)));
		data.bio =
			"Line one of a long synthetic biography.\nLine two.\nLine three.\nLine four.".repeat(3);
		data.badges = (0..8)
			.map(|i| model::ProfileBadge {
				id: format!("badge-{i}"),
				description: format!("Synthetic badge {i}"),
				icon: Some("c".repeat(32)),
			})
			.collect();
		data.connections = (0..6)
			.map(|i| model::ProfileConnection {
				kind: "github".into(),
				id: i.to_string(),
				name: format!("synthetic-profile-{i}"),
				verified: true,
			})
			.collect();
		data.mutual_guilds = (0..6)
			.map(|i| model::ProfileGuild {
				id: Id(100 + i),
				nick: None,
			})
			.collect();
		let view = ProfileView {
			user: user.id,
			guild: None,
			request: 1,
			loading: false,
			error: None,
			data: Some(data),
		};
		let state = State {
			demo: true,
			..Default::default()
		};
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let mut opening = None;
		let size = vec2(400.0, 500.0);
		let mut rect = None;
		for _ in 0..3 {
			let output = ctx.run_ui(input(size, vec![]), |ui| {
				show(
					ui,
					&user,
					Some(&view),
					&state,
					&mut images,
					&mut opening,
					&mut FormatCache::default(),
					true,
					pos2(20.0, 40.0),
				);
			});
			rect = Some(
				ctx.memory(|m| m.area_rect(egui::Id::unique("user-profile-popout")))
					.expect("popout area"),
			);
			output.drop_without_applying_deltas();
		}
		let rect = rect.expect("rendered at least once");
		let bounds = Rect::from_min_size(Pos2::ZERO, size).shrink(8.0);
		assert!(
			rect.bottom() <= bounds.bottom() + 1.0,
			"card bottom {} exceeds viewport bottom {}; its rounded corner would render off-window",
			rect.bottom(),
			bounds.bottom()
		);
	}

	#[test]
	fn dm_presence_does_not_use_a_visible_guild_snapshot() {
		let mut state = test_support::demo_state();
		let user = test_support::message(1, Id(22)).author;
		state.members = Some(model::MemberList {
			guild: Some(Id(10)),
			channel: Id(20),
			request: 1,
			total: 1,
			lazy: false,
			groups: vec![],
			ranges: vec![],
			freshness: model::Freshness::Fresh,
			start: 0,
			slots: vec![Some(model::MemberSlot::Person(model::Member {
				roles: vec![],
				user: user.clone(),
				nick: None,
				status: Some("idle".into()),
				custom_status: Some("Server status".into()),
				activities: vec![],
				clients: model::ClientPlatforms::default(),
			}))],
		});
		state.direct_presences.push(model::MemberPresence {
			user: user.id,
			status: Some("online".into()),
			custom_status: Some("Direct status".into()),
			activities: vec![],
			clients: model::ClientPlatforms::default(),
		});
		assert_eq!(
			presence(&state, user.id, Some(Id(10))).1,
			Some("Server status")
		);
		assert_eq!(presence(&state, user.id, None).1, Some("Direct status"));
		state.gateway_connected = false;
		state.demo = false;
		assert_eq!(
			presence(&state, user.id, None),
			(None, None, [].as_slice(), model::ClientPlatforms::default())
		);
		// Known guild presence survives a reconnect; losing the list clears it.
		assert_eq!(
			presence(&state, user.id, Some(Id(10))).1,
			Some("Server status")
		);
		state.members.as_mut().unwrap().freshness = model::Freshness::Unavailable;
		assert_eq!(
			presence(&state, user.id, Some(Id(10))),
			(None, None, [].as_slice(), model::ClientPlatforms::default())
		);
	}

	#[test]
	fn popout_shows_selected_data_beside_anchor_and_closes_with_escape() {
		let user = User {
			id: Id(2),
			name: "Synthetic person".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		let profile = ProfileView {
			user: user.id,
			guild: None,
			request: 1,
			loading: false,
			error: None,
			data: Some(synthetic(&user, Some(Id(9)))),
		};
		let state = State {
			user: Some(user.clone()),
			demo: true,
			..Default::default()
		};
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let mut opening = None;
		let mut painted = String::new();
		let anchor = pos2(120.0, 300.0);
		for _ in 0..3 {
			let output = ctx.run_ui(input(vec2(1000.0, 900.0), vec![]), |ui| {
				assert!(
					show(
						ui,
						&user,
						Some(&profile),
						&state,
						&mut images,
						&mut opening,
						&mut FormatCache::default(),
						true,
						anchor
					)
					.is_none()
				);
			});
			for shape in &output.shapes {
				text(&shape.shape, &mut painted);
			}
			assert!(output.platform_output.commands.is_empty());
			output.drop_without_applying_deltas();
		}
		assert!(
			painted.contains("Synthetic person")
				&& painted.contains("ABOUT ME")
				&& painted.contains("they / them")
				&& painted.contains("SRN")
		);
		assert!(!painted.contains("Mutual Server"));
		assert!(!painted.contains("Mutual Friend"));
		assert!(images.take_requests().is_empty());
		let rect = ctx.memory(|m| m.area_rect(egui::Id::unique("user-profile-popout")));
		let rect = rect.expect("popout area");
		assert!((rect.left() - (anchor.x + 12.0)).abs() < 1.0);
		assert!((rect.width() - WIDTH).abs() <= 2.0);
		assert!(rect.top() <= anchor.y && rect.bottom() >= anchor.y);

		// Near the right edge the card flips to the left of the anchor and stays in view.
		let right_anchor = pos2(950.0, 100.0);
		let output = ctx.run_ui(input(vec2(1000.0, 900.0), vec![]), |ui| {
			show(
				ui,
				&user,
				Some(&profile),
				&state,
				&mut images,
				&mut opening,
				&mut FormatCache::default(),
				true,
				right_anchor,
			);
		});
		output.drop_without_applying_deltas();
		let rect = ctx
			.memory(|m| m.area_rect(egui::Id::unique("user-profile-popout")))
			.unwrap();
		assert!(rect.right() <= right_anchor.x - 12.0 + 1.0);

		let output = ctx.run_ui(
			input(
				vec2(1000.0, 900.0),
				vec![egui::Event::Key {
					key: egui::Key::Escape,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}],
			),
			|ui| {
				assert!(matches!(
					show(
						ui,
						&user,
						Some(&profile),
						&state,
						&mut images,
						&mut opening,
						&mut FormatCache::default(),
						true,
						anchor
					),
					Some(Action::Close)
				));
			},
		);
		output.drop_without_applying_deltas();
	}

	#[test]
	fn server_profile_shows_assigned_known_roles() {
		let mut state = test_support::demo_state();
		for (id, name, color, position) in [
			(Id(101), "Maintainer", 0x5865f2, 2),
			(Id(102), "Contributor", 0, 1),
		] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::Permissions(client_core::permissions::Event::Role {
					guild: Id(10),
					role: model::permissions::Role {
						id,
						bits: 0,
						name: name.into(),
						color,
						position,
						hoist: false,
					},
				}),
			});
		}
		let user = test_support::message(1, Id(20)).author;
		let mut data = synthetic(&user, Some(Id(10)));
		data.guild.as_mut().unwrap().roles = vec![Id(101), Id(102), Id(999)];
		let profile = ProfileView {
			user: user.id,
			guild: Some(Id(10)),
			request: 1,
			loading: false,
			error: None,
			data: Some(data),
		};
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let mut opening = None;
		let mut painted = String::new();
		for _ in 0..3 {
			let output = ctx.run_ui(input(vec2(700.0, 800.0), vec![]), |ui| {
				show(
					ui,
					&user,
					Some(&profile),
					&state,
					&mut images,
					&mut opening,
					&mut FormatCache::default(),
					true,
					pos2(20.0, 70.0),
				);
			});
			for shape in &output.shapes {
				text(&shape.shape, &mut painted);
			}
			output.drop_without_applying_deltas();
		}
		assert!(painted.contains("ROLES"), "{painted}");
		assert!(painted.contains("Maintainer"), "{painted}");
		assert!(painted.contains("Contributor"), "{painted}");
		assert!(!painted.contains("Role 999"), "{painted}");
	}

	#[test]
	fn clicking_outside_closes_but_clicking_inside_keeps_the_popout() {
		let user = User {
			id: Id(2),
			name: "Synthetic person".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		let state = State::default();
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let mut opening = None;
		let anchor = pos2(100.0, 100.0);
		let output = ctx.run_ui(input(vec2(800.0, 600.0), vec![]), |ui| {
			show(
				ui,
				&user,
				None,
				&state,
				&mut images,
				&mut opening,
				&mut FormatCache::default(),
				true,
				anchor,
			);
		});
		output.drop_without_applying_deltas();
		let rect = ctx
			.memory(|m| m.area_rect(egui::Id::unique("user-profile-popout")))
			.unwrap();
		for (pos, closes) in [(rect.center(), false), (pos2(700.0, 550.0), true)] {
			let output = ctx.run_ui(
				input(
					vec2(800.0, 600.0),
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed: true,
							modifiers: egui::Modifiers::NONE,
						},
					],
				),
				|ui| {
					let action = show(
						ui,
						&user,
						None,
						&state,
						&mut images,
						&mut opening,
						&mut FormatCache::default(),
						true,
						anchor,
					);
					assert_eq!(matches!(action, Some(Action::Close)), closes);
				},
			);
			output.drop_without_applying_deltas();
		}
	}

	#[test]
	fn countdown_takes_precedence_and_stops_at_zero() {
		assert_eq!(
			activity_timer(None, Some(131_000), 1_000).as_deref(),
			Some("2:10 remaining")
		);
		assert_eq!(
			activity_timer(Some(0), Some(131_000), 1_000).as_deref(),
			Some("2:10 remaining")
		);
		assert_eq!(
			activity_timer(None, Some(131_000), 132_000).as_deref(),
			Some("0:00 remaining")
		);
		assert_eq!(
			activity_timer(Some(1_000), None, 131_000).as_deref(),
			Some("2:10")
		);
	}
}
