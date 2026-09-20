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
		.on_hover_text(format!("Server tag · server {}", tag.guild))
}

pub enum Action {
	Edit,
	Close,
	Retry,
	Message(Id),
	AddFriend(Id),
	RemoveFriend,
	AcceptFriend(Id),
	Profile(User),
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
	egui::Frame::new()
		.fill(fill)
		.corner_radius(RADIUS)
		.inner_margin(10)
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			ui.horizontal(|ui| {
				let heading = match activity.kind {
					1 => "Streaming",
					2 => "Listening to",
					3 => "Watching",
					5 => "Competing in",
					_ => "Playing",
				};
				ui.label(design::semibold(ui, heading, 12.0).color(muted));
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					let more = icons::button(ui, Icon::More, 20.0, "Activity options");
					egui::Popup::menu(&more).show(|ui| {
						if ui.button("Copy activity").clicked() {
							let mut text = activity.summary();
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
				}
				ui.vertical(|ui| {
					ui.set_width(ui.available_width());
					ui.spacing_mut().item_spacing.y = 2.0;
					ui.add(egui::Label::new(design::semibold(ui, &activity.name, 14.0)).truncate())
						.on_hover_text(&activity.name);
					for text in [activity.details.as_deref(), activity.state.as_deref()]
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
					if let Some(elapsed) = activity
						.started_at
						.and_then(|start| activity_elapsed(start, now))
					{
						let color = design::palette(ui).positive;
						ui.horizontal(|ui| {
							ui.spacing_mut().item_spacing.x = 4.0;
							icons::inline(ui, Icon::GameController, 14.0, color);
							ui.label(RichText::new(elapsed).monospace().size(12.0).color(color));
						});
					}
					if activity.started_at.is_some() && ui.is_rect_visible(ui.min_rect()) {
						ui.ctx()
							.request_repaint_after(std::time::Duration::from_secs(1));
					}
				});
			});
		});
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
	response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
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
			"Friends \u{2713} · click to remove",
			Some(Action::RemoveFriend),
		)
	} else if let Some((_, _, incoming)) = request {
		if *incoming {
			(
				Icon::AddPeople,
				"Accept Friend Request",
				Some(Action::AcceptFriend(user.id)),
			)
		} else {
			(Icon::AddPeople, "Friend Request Sent", None)
		}
	} else if !state.friends_known() || !state.friend_requests_known() {
		(Icon::AddPeople, "Loading friendship status...", None)
	} else {
		(
			Icon::AddPeople,
			"Add Friend",
			Some(Action::AddFriend(user.id)),
		)
	};
	let enabled = action.is_some()
		&& state.friends_known()
		&& state.friend_requests_known()
		&& actions_enabled(state);
	if header_circle(ui, icon, label, enabled).clicked() {
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
	// Message is the card's own footer button, so the menu does not repeat it.
	if user.webhook && ui.button("Copy webhook ID").clicked() {
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
		.add_enabled(enabled, egui::Button::new("Add Note"))
		.clicked()
	{
		action = Some(Action::Menu(crate::user_menu::Action::Note(user.clone())));
		ui.close();
	}
	if ui
		.add_enabled(
			enabled && friend,
			egui::Button::new(if state.friend_nickname(user.id).is_some() {
				"Edit Friend Nickname"
			} else {
				"Add Friend Nickname"
			}),
		)
		.on_disabled_hover_text("Private nicknames are available for confirmed friends.")
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
				egui::Button::new(if muted { "Unmute" } else { "Mute" }),
			)
			.on_hover_text("Mute this direct message's notifications until you unmute it.")
			.clicked()
		{
			action = Some(Action::Menu(crate::user_menu::Action::Mute {
				channel,
				muted: !muted,
			}));
			ui.close();
		}
	} else {
		ui.add_enabled(false, egui::Button::new("Mute"))
			.on_disabled_hover_text("No open direct message with this user.");
	}
	if friend
		&& ui
			.add_enabled(enabled, egui::Button::new("Remove Friend"))
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
				RichText::new(if blocked { "Unblock" } else { "Block" }).color(colors.danger),
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
			"Remove Friend?",
			format!(
				"Are you sure you want to remove {} from your friends?",
				user.name
			),
		)
		.danger()
		.confirm_label("Remove Friend")
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

pub fn presence_label(status: &str) -> &'static str {
	match status {
		"online" => "Online",
		"idle" => "Idle",
		"dnd" => "Do Not Disturb",
		"offline" => "Offline",
		_ => "Presence unavailable",
	}
}
pub(crate) fn presence_color(status: &str) -> Color32 {
	match status {
		"online" => Color32::from_rgb(35, 165, 89),
		"idle" => Color32::from_rgb(240, 178, 50),
		"dnd" => Color32::from_rgb(242, 63, 67),
		_ => Color32::from_rgb(128, 132, 142),
	}
}
fn client_platforms(clients: model::ClientPlatforms) -> impl Iterator<Item = (Icon, &'static str)> {
	[
		(clients.desktop, Icon::Desktop, "Desktop"),
		(clients.mobile, Icon::DeviceMobile, "Mobile"),
		(clients.web, Icon::Globe, "Web"),
		(clients.vr, Icon::VirtualReality, "VR"),
	]
	.into_iter()
	.filter_map(|(active, icon, label)| active.then_some((icon, label)))
}
fn presence_description(status: &str, clients: model::ClientPlatforms) -> String {
	let platforms = client_platforms(clients)
		.map(|(_, label)| label)
		.collect::<Vec<_>>()
		.join(", ");
	if platforms.is_empty() {
		presence_label(status).into()
	} else {
		format!("{} on {platforms}", presence_label(status))
	}
}
pub(crate) fn presence_badge(
	ui: &mut egui::Ui,
	rect: Rect,
	status: &str,
	clients: model::ClientPlatforms,
	ring: Color32,
) {
	let mut platforms = client_platforms(clients);
	let first = platforms.next();
	if let (Some((icon, _)), None) = (first, platforms.next()) {
		let radius = (rect.width() * 0.2).clamp(6.0, 10.0);
		let center = rect.right_bottom() - Vec2::splat(radius + 0.5);
		ui.painter().circle_filled(center, radius + 2.0, ring);
		ui.painter()
			.circle_filled(center, radius, presence_color(status));
		icons::paint(
			ui.painter(),
			icon,
			Rect::from_center_size(center, Vec2::splat(radius * 1.35)),
			Color32::WHITE,
		);
	} else {
		design::presence_dot(ui, rect, presence_color(status), ring);
	}
	let radius = (rect.width() * 0.2).clamp(6.0, 10.0);
	let center = rect.right_bottom() - Vec2::splat(radius + 0.5);
	ui.allocate_rect(
		Rect::from_center_size(center, Vec2::splat((radius + 2.0) * 2.0)),
		egui::Sense::hover(),
	)
	.on_hover_text(presence_description(status, clients));
}
/// Prefer the fresh visible member snapshot; DMs use the bounded session presence cache.
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
			guild.is_some() && list.guild == guild && list.freshness == model::Freshness::Fresh
		})
		.and_then(|list| {
			list.rows
				.iter()
				.flatten()
				.find(|member| member.user.id == user)
		})
		.filter(|_| state.demo || state.gateway_connected)
	{
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
	let remote =
		if guild.is_some()
			&& state.members.as_ref().is_some_and(|list| {
				list.guild == guild && list.freshness == model::Freshness::Fresh
			}) && (state.demo || state.gateway_connected)
		{
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

pub(crate) fn subtitle(custom: Option<&str>, activities: &[model::RichActivity]) -> Option<String> {
	activities
		.first()
		.map(model::RichActivity::summary)
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
			response.widget_info(|| {
				egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone())
			});
			if response.on_hover_text("Show remaining roles").clicked() {
				ui.data_mut(|data| data.insert_temp(expanded_id, true));
			}
		}
	});
}

/// Shows the popout beside `anchor`; returns an action when the card wants to change or close.
#[allow(clippy::too_many_arguments)]
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
	let dm_channel = state
		.channels
		.iter()
		.find(|c| c.kind == 1 && c.recipients.iter().any(|u| u.id == user.id))
		.map(|c| c.id);
	let mut action = None;
	let menu_id = egui::Id::unique(("user-profile-more", user.id));
	// A menu that was already open owns Escape and clicks on its own items this frame.
	let menu_open = egui::Popup::is_id_open(ui.ctx(), menu_id);
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
			let (banner, _) = ui.allocate_exact_size(vec2(WIDTH, 105.0), egui::Sense::hover());
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
					let more = header_circle(ui, Icon::More, "More", true);
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
			ui.painter()
				.circle_filled(avatar_rect.center(), AVATAR * 0.5 + 6.0, theme.card);
			ui.scope_builder(UiBuilder::new().max_rect(avatar_rect), |ui| {
				if let Some(data) = data {
					avatars.show_profile_avatar(ui, data, AVATAR, state.demo);
				} else {
					avatars.show(ui, user, AVATAR, state.demo);
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
					let has_action = state.user.as_ref().is_some_and(|own| own.id == user.id)
						|| dm_channel.is_some()
						|| user.webhook;
					let footer = if has_action { 40.0 } else { 0.0 }
						+ if state.user_action_status().is_some() {
							24.0
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
											"Server tag · server {}",
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
							if let Some(custom) = custom {
								ui.add_space(4.0);
								ui.add(egui::Label::new(RichText::new(custom).size(13.0)).wrap());
							}
							if clients.any() {
								ui.add_space(4.0);
								ui.horizontal_wrapped(|ui| {
									ui.spacing_mut().item_spacing.x = 5.0;
									ui.label(
										RichText::new("Active on").size(12.0).color(theme.muted),
									);
									for (icon, label) in client_platforms(clients) {
										icons::inline(
											ui,
											icon,
											14.0,
											presence_color(status.unwrap_or("online")),
										);
										ui.label(RichText::new(label).size(12.0));
									}
								});
							}
							if !user.webhook && view.is_none_or(|v| v.loading) {
								ui.add_space(4.0);
								ui.horizontal(|ui| {
									ui.spinner();
									ui.label(
										RichText::new("Loading profile…")
											.size(13.0)
											.color(theme.muted),
									);
								});
							}
							if let Some(error) = view.and_then(|v| v.error) {
								ui.add_space(4.0);
								ui.label(RichText::new(error).size(12.0).color(theme.muted));
								if ui.small_button("Retry profile").clicked() {
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
											for activity in activities {
												activity_card(
													ui,
													activity,
													avatars,
													state.demo,
													(theme.chip, theme.muted),
												);
											}
										}
										if let Some(data) = data {
											let bio = data
												.guild
												.as_ref()
												.map(|g| g.bio.as_str())
												.filter(|s| !s.is_empty())
												.unwrap_or(&data.bio);
											if !bio.is_empty() {
												section(ui, &theme, &mut sections, "ABOUT ME");
												let mut linked_user = None;
												formatted.get(user.id, bio).show_with_images(
													ui,
													opening,
													&[],
													&mut linked_user,
													(avatars, state.demo, &state.guilds),
												);
												if let Some(user) = linked_user {
													action = Some(Action::Profile(user));
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
												section(ui, &theme, &mut sections, "ROLES");
												role_chips(ui, &theme, state, data.user.id, guild);
											}
											section(ui, &theme, &mut sections, "MEMBER SINCE");
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
															.map_or("Server", |g| g.name.as_str());
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
											if !data.mutual_guilds.is_empty() {
												ui.add_space(10.0);
												let names: Vec<String> = data
													.mutual_guilds
													.iter()
													.map(|guild| {
														state
															.guilds
															.iter()
															.find(|g| g.id == guild.id)
															.map_or_else(
																|| format!("Server {}", guild.id),
																|g| g.name.clone(),
															)
													})
													.collect();
												ui.horizontal(|ui| {
													ui.spacing_mut().item_spacing.x = 6.0;
													icons::inline(
														ui,
														Icon::People,
														16.0,
														theme.muted,
													);
													ui.label(
														RichText::new(format!(
															"{} Mutual Server{}",
															names.len(),
															if names.len() == 1 { "" } else { "s" }
														))
														.size(13.0)
														.strong(),
													);
												})
												.response
												.on_hover_text(names.join("\n"));
											}
											if data.limited {
												ui.add_space(6.0);
												ui.label(
													RichText::new(
														"Some profile details were limited",
													)
													.size(11.0)
													.color(theme.muted),
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
									RichText::new("Edit profile").color(colors.accent_text),
								)
								.fill(colors.accent)
								.stroke(Stroke::NONE)
								.corner_radius(RADIUS),
							)
							.clicked()
						{
							action = Some(Action::Edit);
						}
					} else if let Some(channel) = dm_channel {
						if ui
							.add_sized(
								[ui.available_width(), 32.0],
								egui::Button::new(
									RichText::new(format!("Message @{}", user.name))
										.color(colors.accent_text)
										.strong(),
								)
								.fill(colors.accent)
								.stroke(Stroke::NONE)
								.corner_radius(RADIUS),
							)
							.clicked()
						{
							action = Some(Action::Message(channel));
						}
					} else if user.webhook
						&& ui
							.add_sized(
								[ui.available_width(), 32.0],
								egui::Button::new(
									RichText::new("Copy webhook ID").size(13.0).strong(),
								)
								.corner_radius(RADIUS),
							)
							.clicked()
					{
						ui.ctx().copy_text(user.id.to_string());
					}
					if let Some(status) = state.user_action_status() {
						ui.add(
							egui::Label::new(RichText::new(status).size(11.0).color(theme.muted))
								.wrap(),
						);
					}
					if state.demo {
						ui.label(
							RichText::new("Offline preview · synthetic")
								.size(11.0)
								.color(theme.muted),
						);
					}
				});
			let rect = ui.min_rect();
			ui.painter().set(background, theme.background(rect));
		});
	let rect = response.response.rect;
	let pressed_outside = !menu_open
		&& ui.ctx().input(|i| {
			i.pointer.any_pressed()
				&& i.pointer
					.interact_pos()
					.is_some_and(|pos| !rect.contains(pos))
		});
	let escape = opening.is_none()
		&& !menu_open
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
            kind: "GitHub".into(),
            name: "synthetic-profile".into(),
            verified: true,
        }],
		mutual_guilds: guild
            .map(|id| vec![model::ProfileGuild { id, nick: None }])
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
	#[test]
	fn rich_activity_card_keeps_compact_text_badge_and_elapsed_time() {
		assert_eq!(activity_elapsed(1_000, 131_000).as_deref(), Some("2:10"));
		assert_eq!(
			activity_elapsed(1_000, 3_662_000).as_deref(),
			Some("1:01:01")
		);
		assert_eq!(activity_elapsed(1_000, 1_000).as_deref(), Some("0:00"));
		assert_eq!(activity_elapsed(1_000, 999), None);
		for dark in [true, false] {
			for width in [240.0, 316.0] {
				let ctx = egui::Context::default();
				ctx.set_visuals(if dark {
					egui::Visuals::dark()
				} else {
					egui::Visuals::light()
				});
				design::apply(&ctx);
				let mut avatars = Avatars::default();
				let activity = model::RichActivity {
					kind: 0,
					name: "Synthetic Studio".into(),
					details: Some("File CitizenAnimationTracker.cs".into()),
					state: Some("Solution Synthetic-2026".into()),
					image: Some(model::ActivityImage::Asset {
						application: Id(1),
						asset: Id(2),
					}),
					small_image: Some(model::ActivityImage::Application(Id(1))),
					started_at: Some(
						std::time::SystemTime::now()
							.duration_since(std::time::UNIX_EPOCH)
							.unwrap()
							.as_millis() as u64 - 130_000,
					),
				};
				let mut output = ctx.run_ui(input(vec2(500.0, 400.0), vec![]), |ui| {
					ui.set_width(width);
					let colors = design::palette(ui);
					activity_card(
						ui,
						&activity,
						&mut avatars,
						true,
						(colors.raised, colors.muted),
					);
				});
				output.textures_delta.clear();
				let mut painted = String::new();
				for shape in &output.shapes {
					text(&shape.shape, &mut painted);
					if let egui::Shape::Text(text) = &shape.shape {
						assert_eq!(text.galley.rows.len(), 1, "Card text never wraps");
					}
				}
				assert!(
					painted.contains("Playing") && painted.contains("Synthetic Studio"),
					"{painted}"
				);
				assert!(!painted.contains("Playing Synthetic Studio"));
				assert!(painted.contains("2:10"), "{painted}");
				let meshes = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
				let artwork_rect = |image: &model::ActivityImage| {
					let texture = avatars.texture_id(&image.key()).unwrap();
					meshes
						.iter()
						.find_map(|shape| match &shape.primitive {
							egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id == texture => {
								Some(mesh.calc_bounds())
							}
							_ => None,
						})
						.unwrap()
				};
				let main = artwork_rect(activity.image.as_ref().unwrap());
				let badge = artwork_rect(activity.small_image.as_ref().unwrap());
				// Tessellation adds a half-pixel antialiasing fringe to each edge.
				assert!((main.width() - 64.0).abs() <= 1.0 && (main.height() - 64.0).abs() <= 1.0);
				assert!(
					(badge.width() - 24.0).abs() <= 1.0 && (badge.height() - 24.0).abs() <= 1.0
				);
				assert!(
					main.intersects(badge)
						&& badge.right() > main.right()
						&& badge.bottom() > main.bottom()
				);
				output.drop_without_applying_deltas();
			}
		}
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
	fn webhook_card_does_not_show_user_errors_or_retry() {
		for webhook in [false, true] {
			for dark in [false, true] {
				let mut user = test_support::message(1, Id(22)).author;
				user.webhook = webhook;
				let view = ProfileView {
					user: user.id,
					guild: None,
					request: 1,
					loading: false,
					error: Some("Unsupported service response"),
					data: None,
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
				assert_eq!(painted.contains("Unsupported service response"), !webhook);
				assert_eq!(painted.contains("Retry profile"), !webhook);
				assert!(!painted.contains("Loading profile"));
				assert!(images.take_requests().is_empty());
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
				kind: "GitHub".into(),
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
			freshness: model::Freshness::Fresh,
			rows: vec![Some(model::Member {
				roles: vec![],
				user: user.clone(),
				nick: None,
				status: Some("idle".into()),
				custom_status: Some("Server status".into()),
				activities: vec![],
				clients: model::ClientPlatforms::default(),
			})],
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
		assert_eq!(
			presence(&state, user.id, Some(Id(10))),
			(None, None, [].as_slice(), model::ClientPlatforms::default())
		);
	}

	#[test]
	fn base_user_server_tag_is_visible_without_an_extended_profile() {
		let user = User {
			id: Id(2),
			name: "Synthetic person".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: Some(Box::new(model::ClanTag {
				guild: Id(9),
				tag: "SPDY".into(),
				badge: None,
			})),
		};
		let state = State::default();
		let ctx = egui::Context::default();
		let mut images = Avatars::default();
		let mut opening = None;
		let mut painted = String::new();
		for _ in 0..3 {
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
					pos2(100.0, 100.0),
				);
			});
			painted.clear();
			for shape in &output.shapes {
				text(&shape.shape, &mut painted);
			}
			output.drop_without_applying_deltas();
		}
		assert!(painted.contains("SPDY"), "server tag missing: {painted}");
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
			data: Some(synthetic(&user, None)),
		};
		let state = State {
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
	fn role_chips_collapse_after_two_rows() {
		let mut state = test_support::demo_state();
		let roles = [
			(Id(101), "Maintainer"),
			(Id(102), "Contributor"),
			(Id(103), "Release Manager"),
			(Id(104), "Documentation"),
			(Id(105), "Community Helper"),
			(Id(106), "Bug Hunter"),
		];
		for (position, (id, name)) in roles.iter().enumerate() {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::Permissions(client_core::permissions::Event::Role {
					guild: Id(10),
					role: model::permissions::Role {
						id: *id,
						bits: 0,
						name: (*name).into(),
						color: 0x5865f2,
						position: position as i32,
						hoist: false,
					},
				}),
			});
		}
		let guild = model::GuildProfile {
			guild: Id(10),
			roles: roles.iter().map(|(id, _)| *id).collect(),
			nick: None,
			avatar: None,
			banner: None,
			bio: String::new(),
			pronouns: String::new(),
			joined_at: None,
		};
		let ctx = egui::Context::default();
		design::apply(&ctx);
		let output = ctx.run_ui(input(vec2(180.0, 300.0), vec![]), |ui| {
			ui.set_width(180.0);
			let theme = Theme::new(&design::palette(ui), None);
			role_chips(ui, &theme, &state, Id(1), &guild);
		});
		let mut painted = String::new();
		for shape in &output.shapes {
			text(&shape.shape, &mut painted);
		}
		let chips: Vec<_> = output
			.shapes
			.iter()
			.filter_map(|shape| match &shape.shape {
				egui::Shape::Rect(rect) if rect.corner_radius == CornerRadius::same(6) => {
					Some(rect.rect)
				}
				_ => None,
			})
			.collect();
		assert!(
			chips.len() < roles.len(),
			"roles should collapse: {chips:?}"
		);
		assert!(
			painted.contains('+'),
			"remaining role count missing: {painted}"
		);
		assert!(
			chips.iter().skip(1).any(|chip| chip.top() > chips[0].top()),
			"roles should occupy more than one row: {chips:?}"
		);
		assert_eq!(
			chips
				.iter()
				.map(|chip| chip.center().y.round() as i32)
				.collect::<std::collections::BTreeSet<_>>()
				.len(),
			2,
			"roles should occupy exactly two rows: {chips:?}"
		);
		for (index, chip) in chips.iter().enumerate() {
			for other in chips.iter().skip(index + 1) {
				assert!(
					!chip.intersects(*other),
					"role chips overlap: {chip:?} {other:?}"
				);
			}
		}
		output.drop_without_applying_deltas();
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
}
