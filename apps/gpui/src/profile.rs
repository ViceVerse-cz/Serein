//! Profile popout matching the main app's (`ui::profiles`): banner and body in the profile's
//! theme colours, avatar with presence, header actions, badges, server tag, activity, a markdown
//! bio, roles, dates and mutual servers. It shows only the selected service profile and presence
//! already in memory; the offline preview uses a synthetic profile and never fetches anything.
use crate::Serein;
use crate::sidebar::{avatar, presence_color};
use crate::theme::{Icon, color, icon, palette, solid, tint};
use crate::tooltip;
use client_core::State;
use client_core::profile::ProfileView;
use egui::Color32;
use gpui::{prelude::*, *};
use model::{Id, RichActivity, User, UserProfile};
use std::cell::RefCell;
use std::sync::Arc;

const WIDTH: f32 = 340.;
const PAD: f32 = 12.;
const AVATAR: f32 = 80.;
const RADIUS: f32 = 12.;
const BANNER: f32 = 105.;
/// Diameter of the translucent action circles laid over the banner.
const CIRCLE: f32 = 32.;
const BADGE: f32 = 22.;

pub struct Card {
	user: User,
	guild: Option<Id>,
	/// Role ids known from the message or member list, shown until the profile arrives.
	roles: Vec<Id>,
	position: Point<Pixels>,
	menu: bool,
	/// The parsed bio and its source, so markdown is parsed once rather than every frame.
	bio: RefCell<Option<(String, ui::Formatted)>>,
}

#[derive(Clone, Copy)]
enum MenuAction {
	Mention,
	RemoveFriend,
	Block(bool),
	CopyId,
}

/// Status, custom status and activities, as the main app's `profiles::presence`: the open
/// server's member list while it is usable, else retained presence.
pub(crate) fn presence(
	state: &State,
	user: Id,
	guild: Option<Id>,
) -> (Option<&str>, Option<&str>, &[RichActivity]) {
	let member = state
		.members
		.as_ref()
		.filter(|list| usable(state, list, guild))
		.and_then(|list| {
			list.slots.iter().flatten().find_map(|slot| match slot {
				model::MemberSlot::Person(member) if member.user.id == user => Some(member),
				_ => None,
			})
		});
	let remote = match member {
		Some(member) => (
			member.status.as_deref(),
			member.custom_status.as_deref(),
			member.activities.as_slice(),
		),
		None => retained(state, user),
	};
	with_local_activity(state, user, remote)
}

/// Presence for one member row, as the main app's `profiles::member_presence`.
pub(crate) fn member_presence<'a>(
	state: &'a State,
	member: &'a model::Member,
	guild: Option<Id>,
) -> (Option<&'a str>, Option<&'a str>, &'a [RichActivity]) {
	let remote = if state
		.members
		.as_ref()
		.is_some_and(|list| usable(state, list, guild))
	{
		(
			member.status.as_deref(),
			member.custom_status.as_deref(),
			member.activities.as_slice(),
		)
	} else {
		retained(state, member.user.id)
	};
	with_local_activity(state, member.user.id, remote)
}

fn usable(state: &State, list: &model::MemberList, guild: Option<Id>) -> bool {
	guild.is_some()
		&& list.guild == guild
		&& list.freshness != model::Freshness::Unavailable
		&& (state.demo || state.can_view(list.channel))
}

fn retained(state: &State, user: Id) -> (Option<&str>, Option<&str>, &[RichActivity]) {
	state.presence_for(user).map_or((None, None, &[][..]), |p| {
		(
			p.status.as_deref(),
			p.custom_status.as_deref(),
			p.activities.as_slice(),
		)
	})
}

/// The account's own shared game replaces its remote activities, as in the main app.
fn with_local_activity<'a>(
	state: &'a State,
	user: Id,
	remote: (Option<&'a str>, Option<&'a str>, &'a [RichActivity]),
) -> (Option<&'a str>, Option<&'a str>, &'a [RichActivity]) {
	if state.user.as_ref().is_some_and(|own| own.id == user)
		&& let Some(activity) = state.local_game_activity()
	{
		(remote.0, remote.1, std::slice::from_ref(activity))
	} else {
		remote
	}
}

fn is_spotify(activity: &RichActivity) -> bool {
	activity.kind == 2 && activity.name.eq_ignore_ascii_case("Spotify")
}

/// Member-row second line: the first activity wins over the custom status, as in the main app.
pub(crate) fn subtitle(custom: Option<&str>, activities: &[RichActivity]) -> Option<String> {
	activities
		.first()
		.map(|activity| {
			if is_spotify(activity) {
				activity.state.clone().unwrap_or_else(|| activity.summary())
			} else {
				activity.summary()
			}
		})
		.or_else(|| custom.map(str::to_owned))
}

fn presence_label(status: &str) -> &'static str {
	match status {
		"online" => "Online",
		"idle" => "Idle",
		"dnd" => "Do Not Disturb",
		"offline" => "Offline",
		_ => "Presence unavailable",
	}
}

/// Badge, server-tag or activity artwork: the cached image, else the main app's synthetic
/// emblem in the offline preview, else a neutral disc.
pub(crate) fn artwork(key: Option<String>, size: f32, demo: bool) -> Div {
	let p = palette();
	let frame = div()
		.size(px(size))
		.flex_none()
		.flex()
		.items_center()
		.justify_center();
	let Some(key) = key else {
		return frame.child(
			div()
				.size(px(size * 0.8))
				.rounded_full()
				.bg(color(p.raised))
				.border_1()
				.border_color(color(p.border)),
		);
	};
	if let Some(image) = crate::images::get(&key) {
		return frame.child(
			img(image)
				.size_full()
				.rounded(px(size * 0.25))
				.object_fit(ObjectFit::Cover),
		);
	}
	if !demo {
		return frame.child(
			div()
				.size(px(size * 0.8))
				.rounded_full()
				.bg(color(p.raised)),
		);
	}
	// Original synthetic emblem, as the main app's offline fixture; never third-party artwork.
	let seed = key
		.bytes()
		.fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
	let fill =
		rgb((90 + seed % 120) << 16 | (120 + (seed >> 8) % 100) << 8 | (150 + (seed >> 16) % 90));
	frame.child(
		div()
			.size(px(size * 0.875))
			.rounded_full()
			.bg(fill)
			.flex()
			.items_center()
			.justify_center()
			.child(div().size(px(size * 0.375)).rounded_full().bg(white())),
	)
}

/// Compact server tag chip used after member names.
pub(crate) fn server_tag(tag: &model::ClanTag, demo: bool) -> Stateful<Div> {
	let p = palette();
	div()
		.id(SharedString::from(format!(
			"server-tag-{}-{}",
			tag.guild, tag.tag
		)))
		.flex_none()
		.h(px(16.))
		.px(px(4.))
		.rounded(px(4.))
		.bg(color(p.raised))
		.flex()
		.items_center()
		.gap(px(3.))
		.when(tag.badge.is_some(), |d| {
			d.child(artwork(tag.badge_key(), 10., demo))
		})
		.child(
			div()
				.text_size(px(10.))
				.font_weight(FontWeight::SEMIBOLD)
				.text_color(color(p.text_strong))
				.child(tag.tag.clone()),
		)
		.tooltip(tooltip(format!("Server tag · server {}", tag.guild)))
}

fn luma(value: Color32) -> f32 {
	let [r, g, b, _] = value.to_array();
	(0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0
}

fn rgb32(value: u32) -> Color32 {
	Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// Card colours: the shared palette, or the profile theme when the account configured one.
struct Theme {
	gradient: Option<(Rgba, Rgba)>,
	text: Rgba,
	muted: Rgba,
	link: Rgba,
	/// Translucent body panel laid over the gradient.
	panel: Rgba,
	/// Chips and secondary buttons inside the panel.
	chip: Rgba,
	chip_hover: Rgba,
	border: Rgba,
	divider: Rgba,
	card: Rgba,
}
impl Theme {
	fn new(theme: Option<[u32; 2]>) -> Self {
		let p = palette();
		let Some([top, bottom]) = theme else {
			return Self {
				gradient: None,
				text: color(p.text),
				muted: color(p.muted),
				link: color(p.link),
				panel: color(p.raised),
				chip: color(p.hover),
				chip_hover: color(p.selected),
				border: color(p.border),
				divider: color(p.border),
				card: solid(p.surface),
			};
		};
		let (top, bottom) = (rgb32(top), rgb32(bottom));
		// The body panel's tint decides contrast: white over a bright gradient takes dark text,
		// black over a dark one takes light text.
		let bright = (luma(top) + luma(bottom)) * 0.5 > 0.5;
		let pick = |light: Color32, dark: Color32| color(if bright { light } else { dark });
		Self {
			gradient: Some((color(top), color(bottom))),
			text: pick(
				Color32::from_rgb(24, 27, 31),
				Color32::from_rgb(242, 243, 245),
			),
			muted: pick(
				Color32::from_rgb(70, 76, 84),
				Color32::from_rgb(190, 195, 201),
			),
			link: pick(
				Color32::from_rgb(0, 96, 208),
				Color32::from_rgb(0, 176, 244),
			),
			panel: pick(
				Color32::from_white_alpha(170),
				Color32::from_black_alpha(130),
			),
			chip: pick(Color32::from_black_alpha(18), Color32::from_white_alpha(20)),
			chip_hover: pick(Color32::from_black_alpha(36), Color32::from_white_alpha(40)),
			border: pick(Color32::from_black_alpha(48), Color32::from_white_alpha(32)),
			divider: pick(Color32::from_black_alpha(30), Color32::from_white_alpha(24)),
			card: color(top.lerp_to_gamma(bottom, 0.3)),
		}
	}
}

/// The main app's offline banner: the accent colour with light diagonal stripes. Cached per
/// colour (at most eight, offline preview only) so the atlas texture is uploaded once.
fn synthetic_banner(base: Color32) -> Option<Arc<RenderImage>> {
	thread_local! {
		static BANNERS: RefCell<Vec<(Color32, Arc<RenderImage>)>> = const { RefCell::new(Vec::new()) };
	}
	BANNERS.with_borrow_mut(|banners| {
		if let Some((_, image)) = banners.iter().find(|(key, _)| *key == base) {
			return Some(image.clone());
		}
		if banners.len() >= 8 {
			return None;
		}
		let stripe = base.lerp_to_gamma(Color32::WHITE, 0.16);
		// GPUI images are BGRA.
		let pixels = image::RgbaImage::from_fn(128, 48, |x, y| {
			let c = if (x + y) % 48 < 12 { stripe } else { base };
			image::Rgba([c.b(), c.g(), c.r(), 255])
		});
		let image = Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]));
		banners.push((base, image.clone()));
		Some(image)
	})
}

fn creation_date(id: Id) -> Option<String> {
	let seconds = ((id.0 >> 22) + 1_420_070_400_000) / 1000;
	time::OffsetDateTime::from_unix_timestamp(seconds as i64)
		.ok()
		.map(|date| ui::local_datetime(date).date().to_string())
}

/// Offline-only profile, the same synthetic data as the main app's fixture; never a fallback
/// for a failed service request.
fn synthetic(user: &User, guild: Option<Id>) -> UserProfile {
	let hash = |c: char| c.to_string().repeat(32);
	UserProfile {
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

fn section_title(text: &'static str, theme: &Theme, first: bool) -> Div {
	div()
		.when(!first, |d| d.mt(px(10.)))
		.mb(px(2.))
		.text_size(px(12.))
		.text_color(theme.muted)
		.child(text)
}

fn chip(theme: &Theme) -> Div {
	div()
		.flex_none()
		.px(px(6.))
		.py(px(2.))
		.rounded(px(6.))
		.bg(theme.chip)
}

impl Serein {
	pub(crate) fn open_profile(
		&mut self,
		user: User,
		guild: Option<Id>,
		roles: Vec<Id>,
		position: Point<Pixels>,
		cx: &mut Context<Self>,
	) {
		let same = self.state.profile.as_ref().is_some_and(|view| {
			view.user == user.id && view.guild == guild && (view.data.is_some() || view.loading)
		});
		if user.webhook {
			if self.state.profile.is_some() {
				let command = self.state.clear_profile();
				self.dispatch_unless_demo(command);
			}
		} else if self.state.demo {
			// The synthetic profile, as the main app's offline fixture; nothing is requested.
			let data = self
				.state
				.own_profile
				.data
				.as_ref()
				.filter(|data| data.user.id == user.id)
				.cloned()
				.unwrap_or_else(|| synthetic(&user, guild));
			self.state.profile = Some(ProfileView {
				user: user.id,
				guild,
				request: 0,
				loading: false,
				error: None,
				data: Some(data),
			});
		} else if !same {
			// The card shows what is already known while the full profile loads.
			let command = self.state.request_profile(user.id, guild);
			self.dispatch(command);
		}
		self.profile = Some(Card {
			user,
			guild,
			roles,
			position,
			menu: false,
			bio: RefCell::new(None),
		});
		cx.notify();
	}

	pub(crate) fn close_profile(&mut self, cx: &mut Context<Self>) {
		if self.profile.take().is_some() && self.state.profile.is_some() {
			let command = self.state.clear_profile();
			self.dispatch_unless_demo(command);
		}
		cx.notify();
	}

	/// The offline preview has no profile request to cancel.
	fn dispatch_unless_demo(&mut self, command: client_core::Command) {
		if !self.state.demo {
			self.dispatch(Some(command));
		}
	}

	fn dm_with(&self, user: Id) -> Option<Id> {
		self.state
			.channels
			.iter()
			.find(|c| {
				c.guild.is_none()
					&& c.kind == 1 && c.recipients.len() == 1
					&& c.recipients[0].id == user
			})
			.map(|c| c.id)
	}

	/// Whether relationship writes may be sent now, as the main app's `actions_enabled`.
	fn profile_actions_enabled(&self) -> bool {
		!self.state.user_action_pending()
			&& (self.state.demo
				|| (self.state.gateway_connected
					&& self.state.auth == client_core::auth::AuthState::Authenticated))
	}

	fn mention(&mut self, user: Id, window: &mut Window, cx: &mut Context<Self>) {
		let mention = format!("<@{user}> ");
		self.composer.update(cx, |input, cx| {
			let value = format!("{}{mention}", input.value());
			input.set_value(value, cx);
		});
		let focus = self.composer.read(cx).focus_handle(cx);
		window.focus(&focus, cx);
	}

	fn run_profile_menu(
		&mut self,
		action: MenuAction,
		user: User,
		window: &mut Window,
		cx: &mut Context<Self>,
	) {
		if let Some(card) = &mut self.profile {
			card.menu = false;
		}
		let name = self.state.user_display_name(&user).to_owned();
		match action {
			MenuAction::Mention => {
				self.mention(user.id, window, cx);
				self.close_profile(cx);
			}
			MenuAction::CopyId => {
				cx.write_to_clipboard(ClipboardItem::new_string(user.id.to_string()));
				self.notify_user(if user.webhook {
					"Webhook ID copied"
				} else {
					"User ID copied"
				});
			}
			MenuAction::RemoveFriend => self.confirm(
				&format!("Remove '{name}'?"),
				&format!("Are you sure you want to remove {name} from your friends?"),
				"Remove Friend",
				window,
				cx,
				move |this, cx| {
					let command = this.state.remove_friend(user.id);
					this.dispatch(command);
					this.when_settled(
						State::user_action_pending,
						move |this, _| {
							this.notify_user(if this.state.friend(user.id).is_none() {
								"Friend removed"
							} else {
								"Friend was not removed; try again"
							})
						},
						cx,
					);
				},
			),
			MenuAction::Block(block) => {
				let run = move |this: &mut Serein, cx: &mut Context<Serein>| {
					let command = this.state.set_user_blocked(user.id, block);
					this.dispatch(command);
					this.when_settled(
						State::user_action_pending,
						move |this, _| {
							this.notify_user(if this.state.user_blocked(user.id) == Some(block) {
								if block {
									"User blocked"
								} else {
									"User unblocked"
								}
							} else {
								"Block setting was not changed; try again"
							})
						},
						cx,
					);
				};
				if block {
					self.confirm(
						&format!("Block '{name}'?"),
						"Blocking also removes them from your friends. They can't message you while blocked.",
						"Block",
						window,
						cx,
						run,
					);
				} else {
					run(self, cx);
				}
			}
		}
		cx.notify();
	}

	/// Add-friend circle state, as the main app's `friend_circle`: icon, label, whether it acts.
	/// `None` hides it (own profile, bots, webhooks, blocked users).
	fn friend_circle(&self, user: &User) -> Option<(Icon, &'static str, bool)> {
		let state = &self.state;
		if user.webhook
			|| user.kind != model::AccountKind::Human
			|| state.user.as_ref().is_none_or(|own| own.id == user.id)
			|| state.user_blocked(user.id) != Some(false)
		{
			return None;
		}
		let known = state.friends_known() && state.friend_requests_known();
		let ready = known && self.profile_actions_enabled();
		let request = state
			.pending_friends()
			.find(|(person, _, _)| person.id == user.id);
		Some(if state.friend(user.id).is_some() {
			(Icon::Check, "Friends \u{2713} · click to remove", ready)
		} else if let Some((_, _, incoming)) = request {
			if *incoming {
				(Icon::UserPlus, "Accept Friend Request", ready)
			} else {
				(Icon::UserPlus, "Friend Request Sent", false)
			}
		} else if !known {
			(Icon::UserPlus, "Loading friendship status...", false)
		} else {
			(Icon::UserPlus, "Add Friend", ready)
		})
	}

	fn friend_click(&mut self, user: User, window: &mut Window, cx: &mut Context<Self>) {
		if self.state.friend(user.id).is_some() {
			self.run_profile_menu(MenuAction::RemoveFriend, user, window, cx);
			return;
		}
		let incoming = self
			.state
			.pending_friends()
			.any(|(person, _, incoming)| person.id == user.id && *incoming);
		let command = if incoming {
			self.state.resolve_friend_request(user.id, true)
		} else {
			self.state.add_profile_friend(user.id)
		};
		let sent = command.is_some();
		self.dispatch(command);
		if sent {
			self.when_settled(
				State::user_action_pending,
				move |this, _| {
					let pending = this
						.state
						.pending_friends()
						.any(|(person, _, _)| person.id == user.id);
					this.notify_user(if this.state.friend(user.id).is_some() {
						"You are now friends"
					} else if pending {
						"Friend request sent"
					} else {
						"Friend request was not sent; try again"
					});
				},
				cx,
			);
		}
		cx.notify();
	}

	pub(crate) fn render_profile(
		&self,
		window: &Window,
		cx: &mut Context<Self>,
	) -> Option<impl IntoElement> {
		let p = palette();
		let card = self.profile.as_ref()?;
		let user = &card.user;
		let demo = self.state.demo;
		let viewport = window.viewport_size();
		let view = self
			.state
			.profile
			.as_ref()
			.filter(|view| view.user == user.id && !user.webhook);
		let data = view.and_then(|view| view.data.as_ref());
		let theme = Theme::new(data.and_then(|data| data.theme_colors));
		let guild = self
			.state
			.selected
			.and_then(|id| self.state.channel(id))
			.and_then(|c| c.guild)
			.or(card.guild);
		let (status, custom, activities) = if user.webhook {
			(None, None, &[][..])
		} else {
			presence(&self.state, user.id, guild)
		};
		let own = self
			.state
			.user
			.as_ref()
			.is_some_and(|own| own.id == user.id);
		let dm = self.dm_with(user.id);
		let friend = self.state.friend(user.id).is_some();

		// Banner: profile art, the offline synthetic stripes, or the accent colour.
		let base = data
			.and_then(|d| d.accent_color.or(d.theme_colors.map(|c| c[0])))
			.map(rgb32);
		let banner_fill = match (data, base) {
			(None, _) => color(p.raised),
			(Some(_), Some(base)) => color(base),
			(Some(_), None) => tint(p.accent, 0.4),
		};
		let banner_image = data.and_then(|d| d.banner_key()).and_then(|key| {
			crate::images::get(&key).or_else(|| {
				demo.then(|| synthetic_banner(base.unwrap_or(Color32::from_rgb(49, 92, 104))))
					.flatten()
			})
		});
		let banner = div()
			.absolute()
			.top_0()
			.left_0()
			.w(px(WIDTH))
			.h(px(BANNER))
			.rounded_t(px(RADIUS))
			.bg(banner_fill)
			.children(banner_image.map(|image| {
				img(image)
					.size_full()
					.rounded_t(px(RADIUS))
					.object_fit(ObjectFit::Cover)
			}));

		let circle = |id: &'static str, glyph: Icon, label: &'static str, enabled: bool| {
			div()
				.id(id)
				.size(px(CIRCLE))
				.flex_none()
				.rounded_full()
				.bg(hsla(0., 0., 0., 140. / 255.))
				.flex()
				.items_center()
				.justify_center()
				.tooltip(tooltip(label))
				.when(enabled, |d| {
					d.cursor_pointer()
						.hover(|d| d.bg(hsla(0., 0., 0., 200. / 255.)))
				})
				.child(icon(
					glyph,
					px(16.),
					if enabled {
						hsla(0., 0., 1., 1.)
					} else {
						hsla(0., 0., 1., 120. / 255.)
					},
				))
		};
		let friend_circle = self.friend_circle(user).map(|(glyph, label, enabled)| {
			let target = user.clone();
			circle("profile-friend", glyph, label, enabled).when(enabled, |d| {
				d.on_click(cx.listener(move |this, _, window, cx| {
					this.friend_click(target.clone(), window, cx)
				}))
			})
		});
		let more = circle("profile-more", Icon::More, "More", true).on_mouse_down(
			MouseButton::Left,
			cx.listener(|this, _, _, cx| {
				if let Some(card) = &mut this.profile {
					card.menu = !card.menu;
				}
				cx.stop_propagation();
				cx.notify();
			}),
		);
		let circles = div()
			.absolute()
			.top(px(PAD))
			.right(px(PAD))
			.flex()
			.gap(px(8.))
			.children(friend_circle)
			.child(more);

		// Avatar ringed in the card colour, overlapping the banner, with its presence dot.
		let avatar_left = PAD + 4.;
		let avatar_top = BANNER - AVATAR * 0.5 - 6.;
		let name = self.state.user_display_name(user).to_owned();
		let avatar_user = data.map_or(user, |d| &d.user);
		let ring = div()
			.absolute()
			.left(px(avatar_left - 6.))
			.top(px(avatar_top - 6.))
			.size(px(AVATAR + 12.))
			.rounded_full()
			.bg(theme.card)
			.flex()
			.items_center()
			.justify_center()
			.child(avatar(&name, AVATAR, Some(avatar_user)));
		let dot = status.and_then(|status| {
			let fill = presence_color(Some(status))?;
			let center = (avatar_left + AVATAR - 12., avatar_top + AVATAR - 12.);
			Some(
				div()
					.id("profile-presence")
					.absolute()
					.left(px(center.0 - 13.))
					.top(px(center.1 - 13.))
					.size(px(26.))
					.rounded_full()
					.bg(theme.card)
					.flex()
					.items_center()
					.justify_center()
					.tooltip(tooltip(presence_label(status)))
					.child(div().size(px(18.)).rounded_full().bg(fill)),
			)
		});

		// Icon badges in a pill right of the avatar; text badges go under the name.
		let (icon_badges, text_badges): (Vec<_>, Vec<_>) = data
			.map(|d| d.badges.iter().partition(|b| b.icon.is_some()))
			.unwrap_or_default();
		let mut header_bottom = avatar_top + AVATAR;
		let pill = (!icon_badges.is_empty()).then(|| {
			let right = WIDTH - PAD;
			let count = icon_badges.len() as f32;
			let width =
				(count * BADGE + (count - 1.) * 4. + 12.).min(right - (avatar_left + AVATAR) - 12.);
			let per_row = (((width - 12. + 4.) / (BADGE + 4.)).floor() as usize).max(1);
			let rows = icon_badges.len().div_ceil(per_row) as f32;
			let height = rows * BADGE + (rows - 1.) * 4. + 12.;
			header_bottom = header_bottom.max(BANNER + 8. + height);
			div()
				.absolute()
				.top(px(BANNER + 8.))
				.right(px(PAD))
				.w(px(width))
				.p(px(6.))
				.rounded(px(RADIUS))
				.bg(theme.panel)
				.flex()
				.flex_wrap()
				.gap(px(4.))
				.children(icon_badges.iter().enumerate().map(|(ix, badge)| {
					div()
						.id(("profile-badge", ix))
						.tooltip(tooltip(badge.description.clone()))
						.child(artwork(badge.icon_key(), BADGE, demo))
				}))
		});
		let header = div()
			.relative()
			.flex_none()
			.w(px(WIDTH))
			.h(px(header_bottom + 10.))
			.child(banner)
			.child(circles)
			.child(ring)
			.children(dot)
			.children(pill);

		// Identity: display name with server tag, username and pronouns, text badges, status.
		let display = data
			.and_then(|d| {
				d.guild
					.as_ref()
					.and_then(|g| g.nick.as_deref())
					.or(self.state.friend_nickname(user.id))
					.or(d.global_name.as_deref())
			})
			.unwrap_or(&name)
			.split_whitespace()
			.collect::<Vec<_>>()
			.join(" ");
		let clan = data
			.map(|d| d.clan.as_ref())
			.unwrap_or(user.primary_guild.as_deref());
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
		let loading = !user.webhook && view.is_none_or(|v| v.loading);
		let error = view.and_then(|v| v.error);
		let mut panel = div()
			.min_h_0()
			.flex_shrink(1.)
			.p(px(12.))
			.rounded(px(RADIUS))
			.bg(theme.panel)
			.flex()
			.flex_col()
			.gap(px(3.))
			.when(error.is_some() || data.is_some_and(|d| d.limited), |d| {
				d.child(
					div()
						.mb(px(6.))
						.px(px(8.))
						.py(px(6.))
						.rounded(px(6.))
						.bg(tint(p.warning, 0.18))
						.text_size(px(12.))
						.text_color(color(p.warning))
						.child("Unable to load parts of profile"),
				)
			})
			.child(
				div()
					.flex()
					.items_center()
					.gap(px(8.))
					.child(
						div()
							.min_w_0()
							.overflow_hidden()
							.whitespace_nowrap()
							.text_ellipsis()
							.text_size(px(20.))
							.line_height(px(24.))
							.text_color(theme.text)
							.child(display),
					)
					.children(clan.map(|clan| {
						chip(&theme)
							.id("profile-clan")
							.flex()
							.items_center()
							.gap(px(4.))
							.tooltip(tooltip(format!("Server tag · server {}", clan.guild)))
							.child(artwork(clan.badge_key(), 14., demo))
							.child(
								div()
									.text_size(px(12.))
									.text_color(theme.text)
									.child(clan.tag.clone()),
							)
					})),
			)
			.child(
				div()
					.overflow_hidden()
					.whitespace_nowrap()
					.text_ellipsis()
					.text_size(px(14.))
					.text_color(theme.muted)
					.child(identity.join(" • ")),
			)
			.when(!text_badges.is_empty(), |d| {
				d.child(div().mt(px(2.)).flex().flex_wrap().gap(px(4.)).children(
					text_badges.iter().enumerate().map(|(ix, badge)| {
						chip(&theme)
							.id(("profile-text-badge", ix))
							.tooltip(tooltip(badge.id.clone()))
							.text_size(px(11.))
							.text_color(theme.text)
							.child(badge.description.clone())
					}),
				))
			})
			.children(custom.map(|custom| {
				div()
					.mt(px(4.))
					.text_size(px(13.))
					.text_color(theme.text)
					.child(custom.to_owned())
			}))
			.when(loading, |d| {
				d.child(
					div()
						.mt(px(4.))
						.text_size(px(13.))
						.text_color(theme.muted)
						.child("Loading profile…"),
				)
			})
			.children(error.map(|error| {
				let (target, profile_guild) = (user.id, card.guild);
				div().mt(px(4.)).child(
					chip(&theme)
						.id("profile-retry")
						.cursor_pointer()
						.text_size(px(12.))
						.text_color(theme.text)
						.hover(|d| d.bg(theme.chip_hover))
						.tooltip(tooltip(error))
						.on_click(cx.listener(move |this, _, _, cx| {
							let command = this.state.request_profile(target, profile_guild);
							this.dispatch(command);
							cx.notify();
						}))
						.child("Retry profile"),
				)
			}));

		if data.is_some() || !activities.is_empty() {
			panel = panel.child(div().my(px(4.)).h(px(1.)).bg(theme.divider));
			let mut details = div()
				.id("profile-details")
				.min_h_0()
				.flex_shrink(1.)
				.overflow_y_scroll()
				.flex()
				.flex_col()
				.gap(px(4.));
			let mut first = true;
			for (ix, activity) in activities.iter().enumerate() {
				first = false;
				details = details.child(self.activity_card(ix, activity, &theme, cx));
			}
			if let Some(data) = data {
				let bio = data
					.guild
					.as_ref()
					.map(|g| g.bio.as_str())
					.filter(|s| !s.is_empty())
					.unwrap_or(&data.bio);
				if !bio.is_empty() {
					details = details
						.child(section_title("ABOUT ME", &theme, first))
						.child(self.bio(card, bio, &theme));
					first = false;
				}
				let roles = self.profile_roles(card, data);
				if !roles.is_empty() {
					details = details
						.child(section_title("ROLES", &theme, first))
						.child(role_chips(&roles, &theme));
					first = false;
				}
				let joined = data.guild.as_ref().and_then(|g| {
					let date = g.joined_at.as_deref()?;
					let server = self
						.state
						.guilds
						.iter()
						.find(|known| known.id == g.guild)
						.map_or("Server", |g| g.name.as_str());
					Some((
						server.to_owned(),
						date.split('T').next().unwrap_or(date).to_owned(),
					))
				});
				details = details
					.child(section_title("MEMBER SINCE", &theme, first))
					.child(
						div()
							.flex()
							.flex_wrap()
							.items_center()
							.gap(px(6.))
							.text_size(px(13.))
							.text_color(theme.text)
							.children(creation_date(user.id).map(|date| {
								div()
									.id("profile-created")
									.tooltip(tooltip(format!(
										"Discord account created {}",
										crate::chat::day_label(user.id)
									)))
									.flex()
									.items_center()
									.gap(px(6.))
									.child(icon(Icon::Calendar, px(16.), theme.muted))
									.child(date)
							}))
							// Separate items so the line wraps between words like the main app's
							// label: "date • Server" then the join date.
							.when_some(joined, |d, (server, date)| {
								d.child(div().text_color(theme.muted).child("•"))
									.child(server)
									.child(date)
							}),
					);
				if !data.mutual_guilds.is_empty() && !own {
					let names = data
						.mutual_guilds
						.iter()
						.map(|guild| {
							self.state
								.guilds
								.iter()
								.find(|g| g.id == guild.id)
								.map_or_else(|| format!("Server {}", guild.id), |g| g.name.clone())
						})
						.collect::<Vec<_>>();
					let count = names.len();
					details = details.child(
						div()
							.id("profile-mutual")
							.mt(px(10.))
							.flex()
							.items_center()
							.gap(px(6.))
							.text_size(px(13.))
							.text_color(theme.text)
							.tooltip(tooltip(names.join("\n")))
							.child(icon(Icon::Users, px(16.), theme.muted))
							.child(format!(
								"{count} Mutual Server{}",
								if count == 1 { "" } else { "s" }
							)),
					);
				}
			}
			panel = panel.child(details);
		}

		// Footer: one full-width action, as the main app.
		let primary = |id: &'static str, label: String| {
			div()
				.id(id)
				.flex_none()
				.h(px(32.))
				.rounded(px(RADIUS))
				.bg(color(p.accent))
				.flex()
				.items_center()
				.justify_center()
				.cursor_pointer()
				.hover(|d| d.opacity(0.9))
				.overflow_hidden()
				.whitespace_nowrap()
				.text_ellipsis()
				.text_size(px(14.))
				.font_weight(FontWeight::MEDIUM)
				.text_color(color(p.accent_text))
				.child(label)
		};
		let secondary = |id: &'static str, label: String| {
			div()
				.id(id)
				.flex_none()
				.h(px(32.))
				.rounded(px(RADIUS))
				.bg(theme.chip)
				.flex()
				.items_center()
				.justify_center()
				.cursor_pointer()
				.hover(|d| d.bg(theme.chip_hover))
				.text_size(px(13.))
				.text_color(theme.text)
				.child(label)
		};
		let target = user.clone();
		let footer = if own {
			Some(
				primary("profile-edit", "Edit profile".into()).on_click(cx.listener(
					|this, _, window, cx| {
						this.close_profile(cx);
						this.open_settings(Some(crate::settings::Page::Profile), window, cx);
					},
				)),
			)
		} else if user.webhook {
			Some(
				secondary("profile-copy-webhook", "Copy webhook ID".into()).on_click(cx.listener(
					move |this, _, window, cx| {
						this.run_profile_menu(MenuAction::CopyId, target.clone(), window, cx)
					},
				)),
			)
		} else if dm.is_some() || friend {
			Some(
				primary("profile-message", format!("Message @{}", user.name)).on_click(
					cx.listener(move |this, _, _, cx| {
						let user = target.id;
						this.close_profile(cx);
						match this.dm_with(user) {
							Some(channel) => {
								this.friends.open = false;
								this.select(channel, cx);
							}
							None => this.message_friend(user, cx),
						}
					}),
				),
			)
		} else {
			// No conversation to open and no friendship to open one with: a mention instead.
			Some(
				secondary("profile-mention", format!("Mention @{}", user.name)).on_click(
					cx.listener(move |this, _, window, cx| {
						this.run_profile_menu(MenuAction::Mention, target.clone(), window, cx)
					}),
				),
			)
		};

		let body = div()
			.min_h_0()
			.flex_shrink(1.)
			.px(px(PAD))
			.pb(px(PAD))
			.flex()
			.flex_col()
			.gap(px(8.))
			.child(panel)
			.children(footer)
			.when(demo, |d| {
				d.child(
					div()
						.flex_none()
						.text_size(px(11.))
						.text_color(theme.muted)
						.child("Offline preview · synthetic"),
				)
			});

		let menu = card.menu.then(|| self.profile_menu(user, own, friend, cx));
		let background: Background = match theme.gradient {
			Some((top, bottom)) => linear_gradient(
				180.,
				linear_color_stop(top, 0.),
				linear_color_stop(bottom, 1.),
			),
			None => theme.card.into(),
		};
		// Beside the anchor like the main app: right of it when it fits, else left of it.
		let anchor = card.position;
		let x = if anchor.x + px(12. + WIDTH) <= viewport.width - px(8.) {
			anchor.x + px(12.)
		} else {
			(anchor.x - px(12. + WIDTH)).max(px(8.))
		};
		let y = (anchor.y - px(40.)).max(px(8.));
		Some(deferred(
			anchored()
				.position(point(x, y))
				.snap_to_window_with_margin(px(8.))
				.child(
					div()
						.id("profile-card")
						.occlude()
						.relative()
						.w(px(WIDTH))
						.max_h(viewport.height - px(16.))
						.flex()
						.flex_col()
						.rounded(px(RADIUS))
						.bg(background)
						.border_1()
						.border_color(theme.border)
						.shadow_lg()
						.font_family(crate::theme::FONT)
						.on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_profile(cx)))
						.on_mouse_down(
							MouseButton::Left,
							cx.listener(|this, _, _, cx| {
								if let Some(card) = &mut this.profile
									&& card.menu
								{
									card.menu = false;
									cx.notify();
								}
							}),
						)
						.child(header)
						.child(body)
						.children(menu),
				),
		))
	}

	fn activity_card(
		&self,
		ix: usize,
		activity: &RichActivity,
		theme: &Theme,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let spotify = is_spotify(activity);
		let heading = match activity.kind {
			1 => "Streaming",
			2 if spotify => "Listening to Spotify",
			2 => "Listening to",
			3 => "Watching",
			5 => "Competing in",
			_ => "Playing",
		};
		let title = if spotify {
			activity.details.as_deref().unwrap_or(&activity.name)
		} else {
			&activity.name
		}
		.to_owned();
		let lines = [
			activity.details.as_deref().filter(|_| !spotify),
			activity.state.as_deref(),
		]
		.into_iter()
		.flatten()
		.map(str::to_owned)
		.collect::<Vec<_>>();
		let mut copy = activity.summary();
		for line in [&activity.details, &activity.state].into_iter().flatten() {
			copy.push('\n');
			copy.push_str(line);
		}
		let chip_hover = theme.chip_hover;
		div()
			.flex_none()
			.p(px(10.))
			.rounded(px(RADIUS))
			.bg(theme.chip)
			.flex()
			.flex_col()
			.gap(px(4.))
			.child(
				div()
					.flex()
					.items_center()
					.justify_between()
					.child(
						div()
							.text_size(px(12.))
							.font_weight(FontWeight::SEMIBOLD)
							.text_color(theme.muted)
							.child(heading),
					)
					.child(
						div()
							.id(("profile-activity-copy", ix))
							.size(px(20.))
							.rounded(px(4.))
							.flex()
							.items_center()
							.justify_center()
							.cursor_pointer()
							.hover(move |d| d.bg(chip_hover))
							.tooltip(tooltip("Copy activity"))
							.on_click(cx.listener(move |this, _, _, cx| {
								cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
								this.notify_user("Activity copied");
								cx.notify();
							}))
							.child(icon(Icon::More, px(14.), theme.muted)),
					),
			)
			.child(
				div()
					.flex()
					.items_start()
					.gap(px(10.))
					.children(
						activity
							.image
							.as_ref()
							.map(|image| artwork(Some(image.key()), 64., self.state.demo)),
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
									.overflow_hidden()
									.whitespace_nowrap()
									.text_ellipsis()
									.text_size(px(14.))
									.font_weight(FontWeight::SEMIBOLD)
									.text_color(theme.text)
									.child(title),
							)
							.children(lines.into_iter().map(|line| {
								div()
									.overflow_hidden()
									.whitespace_nowrap()
									.text_ellipsis()
									.text_size(px(12.))
									.text_color(theme.muted)
									.child(line)
							})),
					),
			)
			.into_any_element()
	}

	/// The bio as Discord markdown (bold, italics, links, code), parsed once per source.
	fn bio(&self, card: &Card, source: &str, theme: &Theme) -> AnyElement {
		let mut cache = card.bio.borrow_mut();
		if cache.as_ref().is_none_or(|(cached, _)| cached != source) {
			let source = source.chars().take(400).collect::<String>();
			let parsed = ui::Formatted::parse(&source);
			*cache = Some((source, parsed));
		}
		let formatted = &cache.as_ref().expect("parsed bio").1;
		let mut text = String::new();
		let mut runs = Vec::new();
		let mut links: Vec<(std::ops::Range<usize>, String)> = Vec::new();
		let mut blocks_done = 0;
		for span in formatted.spans() {
			let start = text.len();
			let mut style = HighlightStyle::default();
			let mut styled = false;
			if let Some(block) = span.block {
				if block < blocks_done {
					continue;
				}
				blocks_done = block + 1;
				if let Some((_, code)) = formatted.code_block(block) {
					text.push_str(code);
					text.push('\n');
				}
				style.background_color = Some(theme.chip.into());
				styled = true;
			} else {
				text.push_str(span.text);
			}
			if span.strong || span.heading > 0 {
				style.font_weight = Some(FontWeight::SEMIBOLD);
				styled = true;
			}
			if span.italic {
				style.font_style = Some(FontStyle::Italic);
				styled = true;
			}
			if span.underline {
				style.underline = Some(UnderlineStyle {
					thickness: px(1.),
					..Default::default()
				});
				styled = true;
			}
			if span.strike {
				style.strikethrough = Some(StrikethroughStyle {
					thickness: px(1.),
					..Default::default()
				});
				styled = true;
			}
			if span.code {
				style.background_color = Some(theme.chip.into());
				styled = true;
			}
			if span.link.is_some()
				|| span.mention.is_some()
				|| span.channel.is_some()
				|| span.role.is_some()
			{
				style.color = Some(theme.link.into());
				styled = true;
			}
			if span.spoiler {
				style.color = Some(theme.muted.into());
				style.background_color = Some(theme.muted.into());
				styled = true;
			}
			if styled && start < text.len() {
				runs.push((start..text.len(), style));
			}
			if let Some(link) = span.link.filter(|_| !span.spoiler && start < text.len()) {
				links.push((start..text.len(), link.to_owned()));
			}
		}
		// A trailing code block leaves a newline that would add an empty line.
		let end = text.trim_end().len();
		text.truncate(end);
		let clamp = |range: std::ops::Range<usize>| {
			(range.start < end).then(|| range.start..range.end.min(end))
		};
		let runs = runs
			.into_iter()
			.filter_map(|(range, style)| Some((clamp(range)?, style)))
			.collect::<Vec<_>>();
		let (ranges, urls): (Vec<_>, Vec<_>) = links
			.into_iter()
			.filter_map(|(range, url)| Some((clamp(range)?, url)))
			.unzip();
		// The main app's body style: 15 px text.
		div()
			.text_size(px(15.))
			.line_height(px(18.))
			.text_color(theme.text)
			.child(
				InteractiveText::new("profile-bio", StyledText::new(text).with_highlights(runs))
					.on_click(ranges, move |ix, window, cx| {
						if let Some(url) = urls.get(ix) {
							crate::chat::confirm_open_link(url.clone(), window, cx);
						}
					}),
			)
			.into_any_element()
	}

	/// Server roles from the fetched profile; before it arrives (or without a server profile),
	/// the roles the message or member list carried.
	fn profile_roles<'a>(
		&'a self,
		card: &Card,
		data: &UserProfile,
	) -> Vec<&'a model::permissions::Role> {
		let Some(guild) = data.guild.as_ref().map(|g| g.guild).or(card.guild) else {
			return Vec::new();
		};
		let held = data
			.guild
			.as_ref()
			.map_or(card.roles.as_slice(), |g| g.roles.as_slice());
		let mut roles = self
			.state
			.guild_roles(guild)
			.map(|roles| {
				roles
					.iter()
					.filter(|role| role.id != guild && held.contains(&role.id))
					.collect::<Vec<_>>()
			})
			.unwrap_or_default();
		roles.sort_by(|a, b| b.cmp_hierarchy(a));
		roles
	}

	fn profile_menu(
		&self,
		user: &User,
		own: bool,
		friend: bool,
		cx: &mut Context<Self>,
	) -> AnyElement {
		let p = palette();
		let ready = self.profile_actions_enabled();
		let blocked = self.state.user_blocked(user.id) == Some(true);
		let mut items: Vec<Option<(&'static str, MenuAction, bool, bool)>> = Vec::new();
		if !user.webhook {
			items.push(Some(("Mention", MenuAction::Mention, true, false)));
		}
		if !user.webhook && !own {
			items.push(None);
			if friend {
				items.push(Some((
					"Remove Friend",
					MenuAction::RemoveFriend,
					ready,
					true,
				)));
			}
			items.push(Some((
				if blocked { "Unblock" } else { "Block" },
				MenuAction::Block(!blocked),
				ready,
				true,
			)));
		}
		if !items.is_empty() {
			items.push(None);
		}
		items.push(Some((
			if user.webhook {
				"Copy Webhook ID"
			} else {
				"Copy User ID"
			},
			MenuAction::CopyId,
			true,
			false,
		)));
		let rows = items.into_iter().enumerate().map(|(ix, item)| {
			let Some((label, action, enabled, danger)) = item else {
				return div()
					.h(px(1.))
					.mx(px(4.))
					.my(px(4.))
					.bg(color(p.border))
					.into_any_element();
			};
			let (text, hover_bg, hover_text) = if danger {
				(p.danger, p.danger, Color32::WHITE)
			} else {
				(p.text, p.accent, p.accent_text)
			};
			let target = user.clone();
			div()
				.id(("profile-menu-item", ix))
				.h(px(32.))
				.px(px(8.))
				.rounded(px(4.))
				.flex()
				.items_center()
				.text_size(px(14.))
				.font_weight(FontWeight::MEDIUM)
				.when(enabled, |d| {
					d.cursor_pointer()
						.text_color(color(text))
						.hover(|d| d.bg(color(hover_bg)).text_color(color(hover_text)))
						.on_click(cx.listener(move |this, _, window, cx| {
							this.run_profile_menu(action, target.clone(), window, cx)
						}))
				})
				.when(!enabled, |d| d.text_color(tint(p.muted, 0.6)))
				.child(label)
				.into_any_element()
		});
		div()
			.id("profile-menu")
			.occlude()
			.absolute()
			.top(px(PAD + CIRCLE + 8.))
			.right(px(PAD))
			.w(px(200.))
			.p(px(6.))
			.flex()
			.flex_col()
			.rounded(px(8.))
			.bg(solid(p.chat))
			.border_1()
			.border_color(color(p.border))
			.shadow_lg()
			// Clicks inside the menu must not reach the card, which closes the menu.
			.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
			.children(rows)
			.into_any_element()
	}
}

fn role_chips(roles: &[&model::permissions::Role], theme: &Theme) -> Div {
	div()
		.flex()
		.flex_wrap()
		.gap(px(4.))
		.children(roles.iter().enumerate().map(|(ix, role)| {
			chip(theme)
				.id(("profile-role", ix))
				.max_w_full()
				.flex()
				.items_center()
				.gap(px(6.))
				.tooltip(tooltip(role.name.clone()))
				.child(
					div()
						.size(px(8.))
						.flex_none()
						.rounded_full()
						.bg(if role.color == 0 {
							theme.muted
						} else {
							rgb(role.color)
						}),
				)
				.child(
					div()
						.min_w_0()
						.overflow_hidden()
						.whitespace_nowrap()
						.text_ellipsis()
						.text_size(px(12.))
						.text_color(theme.text)
						.child(role.name.clone()),
				)
		}))
}

#[cfg(test)]
mod tests {
	// Not a glob import: `gpui::*` would shadow the built-in `#[test]` attribute.
	use super::{subtitle, synthetic};
	use model::{Id, RichActivity, User};

	fn activity(kind: u8, name: &str, state: Option<&str>) -> RichActivity {
		RichActivity {
			kind,
			name: name.into(),
			details: None,
			state: state.map(Into::into),
			image: None,
			small_image: None,
			ends_at: None,
			started_at: None,
		}
	}

	#[test]
	fn activity_wins_over_custom_status_like_the_main_app() {
		let game = [activity(0, "Stardew Valley", None)];
		assert_eq!(
			subtitle(Some("🌙 away"), &game).as_deref(),
			Some("Playing Stardew Valley")
		);
		assert_eq!(subtitle(Some("🌙 away"), &[]).as_deref(), Some("🌙 away"));
		let spotify = [activity(2, "Spotify", Some("Synthetic Artist"))];
		assert_eq!(
			subtitle(None, &spotify).as_deref(),
			Some("Synthetic Artist")
		);
		assert_eq!(subtitle(None, &[]), None);
	}

	#[test]
	fn synthetic_profile_matches_the_main_app_fixture() {
		let user = User {
			id: Id(2),
			name: "Robin (synthetic)".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		let profile = synthetic(&user, Some(Id(10)));
		assert_eq!(profile.clan.as_ref().map(|c| c.tag.as_str()), Some("SRN"));
		assert_eq!(profile.theme_colors, Some([0x1f3a4d, 0x3b2a5e]));
		assert_eq!(profile.mutual_guilds.len(), 1);
		assert_eq!(
			profile.badges.iter().filter(|b| b.icon.is_none()).count(),
			1
		);
	}
}
