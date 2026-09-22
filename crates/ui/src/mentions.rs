#![allow(clippy::items_after_test_module)]
//! Bounded local suggestions plus on-demand guild member search results.
use crate::avatars::Avatars;
use client_core::State;
use model::{Id, User};
use std::{
	hash::{Hash, Hasher},
	ops::Range,
};

/// Rows shown for people and channels, like Discord's short member list.
const SHORT_LIMIT: usize = 8;
/// Emoji show everything that matches, bounded so a two-letter query stays cheap to lay out.
const EMOJI_LIMIT: usize = 256;
const ROW: f32 = 36.0;
const VISIBLE_ROWS: f32 = 8.5;

#[derive(Default)]
pub struct Menu {
	channel: Option<Id>,
	generation: u64,
	range: Range<usize>,
	query: String,
	kind: Option<Kind>,
	candidates: Vec<Candidate>,
	selected: usize,
	dismissed: bool,
	/// Keyboard moved the highlight; scroll the popout so it stays visible.
	follow: bool,
}
pub struct Pick {
	range: Range<usize>,
	candidate: Candidate,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Kind {
	User,
	Channel,
	Emoji,
}
#[derive(Clone, PartialEq, Eq)]
enum Candidate {
	Role {
		id: Id,
		name: String,
	},
	User {
		user: User,
		name: String,
	},
	Mass {
		name: &'static str,
	},
	Channel {
		id: Id,
		name: String,
	},
	Unicode {
		text: &'static str,
		code: &'static str,
	},
	Custom {
		id: Id,
		name: String,
		animated: bool,
		server: String,
	},
}

pub(crate) fn user_mention_token(user: Id) -> String {
	format!("<@{user}> ")
}

impl Candidate {
	#[cfg(test)]
	fn id(&self) -> Id {
		match self {
			Candidate::User { user, .. } => user.id,
			Candidate::Role { id, .. }
			| Candidate::Channel { id, .. }
			| Candidate::Custom { id, .. } => *id,
			Candidate::Mass { .. } | Candidate::Unicode { .. } => Id(0),
		}
	}
	fn token(&self) -> String {
		match self {
			Candidate::Role { id, .. } => format!("<@&{id}> "),
			Candidate::User { user, .. } => user_mention_token(user.id),
			Candidate::Mass { name } => format!("@{name} "),
			Candidate::Channel { id, .. } => format!("<#{id}> "),
			Candidate::Unicode { text, .. } => format!("{text} "),
			Candidate::Custom {
				id, name, animated, ..
			} => {
				format!("<{}:{name}:{id}> ", if *animated { "a" } else { "" })
			}
		}
	}
}
pub fn known_roles(state: &State, channel: Id) -> &[model::permissions::Role] {
	state
		.channel(channel)
		.and_then(|c| c.guild)
		.and_then(|guild| state.guild_roles(guild))
		.unwrap_or(&[])
}
pub fn known_users(state: &State, channel: Id) -> Vec<User> {
	let mut users = Vec::new();
	let mut add = |user: &User| {
		if users.len() < 256 && !users.iter().any(|u: &User| u.id == user.id) {
			users.push(user.clone());
		}
	};
	if let Some(request) = &state.member_search[0].request
		&& request.channel == channel
		&& state.can_view(channel)
	{
		for member in &state.member_search[0].rows {
			add(&member.user);
		}
	}
	if let Some(owner) = &state.user {
		add(owner);
	}
	if let Some(channel) = state.channels.iter().find(|c| c.id == channel) {
		for user in &channel.recipients {
			add(user);
		}
	}
	if let Some(members) = state.members.as_ref().filter(|m| m.channel == channel) {
		for member in members
			.slots
			.iter()
			.flatten()
			.filter_map(|slot| match slot {
				model::MemberSlot::Person(m) => Some(m),
				_ => None,
			}) {
			add(&member.user);
		}
	}
	for message in state.timeline.iter() {
		if message.channel == channel {
			add(&message.author);
			for user in &message.mentions {
				add(user);
			}
		}
	}
	users
}

pub struct MentionSource<'a> {
	pub state: &'a State,
	pub channel: Id,
}

fn member(state: &State, channel: Id, id: Id) -> Option<&model::Member> {
	state
		.members
		.as_ref()
		.filter(|list| list.channel == channel)
		.and_then(|list| {
			list.slots
				.iter()
				.flatten()
				.filter_map(|slot| match slot {
					model::MemberSlot::Person(m) => Some(m),
					_ => None,
				})
				.find(|member| member.user.id == id)
		})
}

pub fn find_user<'a>(
	id: Id,
	mentions: &'a [User],
	source: Option<&MentionSource<'a>>,
) -> Option<&'a User> {
	if let Some(user) = mentions.iter().find(|user| user.id == id) {
		return Some(user);
	}
	let source = source?;
	let state = source.state;
	if state.user.as_ref().is_some_and(|user| user.id == id) {
		return state.user.as_ref();
	}
	if let Some(user) = state.friend(id) {
		return Some(user);
	}
	if let Some(channel) = state.channel(source.channel)
		&& let Some(user) = channel.recipients.iter().find(|user| user.id == id)
	{
		return Some(user);
	}
	if let Some(member) = member(state, source.channel, id) {
		return Some(&member.user);
	}
	if let Some(request) = &state.member_search[0].request
		&& request.channel == source.channel
		&& state.can_view(source.channel)
		&& let Some(member) = state.member_search[0]
			.rows
			.iter()
			.find(|member| member.user.id == id)
	{
		return Some(&member.user);
	}
	state.timeline.iter().find_map(|message| {
		if message.channel != source.channel {
			return None;
		}
		if message.author.id == id {
			Some(&message.author)
		} else {
			message.mentions.iter().find(|user| user.id == id)
		}
	})
}

pub fn mention_label(id: Id, mentions: &[User], source: Option<&MentionSource<'_>>) -> String {
	if let Some(source) = source
		&& let Some(nick) = member(source.state, source.channel, id)
			.and_then(|member| member.nick.as_deref())
			.filter(|nick| !nick.is_empty())
	{
		return format!("@{nick}");
	}
	match find_user(id, mentions, source) {
		Some(user) => format!(
			"@{}",
			source.map_or(user.name.as_str(), |s| s.state.user_display_name(user))
		),
		None => format!("@{id}"),
	}
}

pub fn presentation_fingerprint(state: &State, message: &model::Message) -> u64 {
	let mut hasher = std::collections::hash_map::DefaultHasher::new();
	let source = MentionSource {
		state,
		channel: message.channel,
	};
	let roles = known_roles(state, message.channel);
	for text in std::iter::once(message.content.as_str())
		.chain(message.prior_contents.as_slice().iter().map(String::as_str))
	{
		let mut rest = text;
		let mut seen = 0usize;
		while seen < model::MAX_MENTIONS {
			let Some(start) = rest.find('<') else {
				break;
			};
			rest = &rest[start..];
			if let Some((id, len)) = model::user_mention_prefix(rest) {
				mention_label(id, &message.mentions, Some(&source)).hash(&mut hasher);
				rest = &rest[len..];
			} else if let Some((id, len)) = model::role_mention_prefix(rest) {
				let name = roles
					.iter()
					.find(|role| role.id == id)
					.map_or_else(|| format!("unknown-role ({id})"), |role| role.name.clone());
				format!("@{name}").hash(&mut hasher);
				rest = &rest[len..];
			} else if let Some((id, len)) = model::channel_mention_prefix(rest) {
				let label = match state.channels.iter().find(|channel| channel.id == id) {
					Some(channel)
						if channel.guild.is_some()
							&& matches!(channel.kind, 0 | 5 | 10..=12 | 15 | 16) =>
					{
						format!("#{}", channel.name)
					}
					Some(_) => String::new(),
					None => "#unknown-channel".into(),
				};
				label.hash(&mut hasher);
				rest = &rest[len..];
			} else {
				let skip = rest.chars().next().map_or(1, char::len_utf8);
				rest = &rest[skip..];
				continue;
			}
			seen += 1;
		}
	}
	hasher.finish()
}

fn query(draft: &str, cursor: usize) -> Option<(Range<usize>, &str, Kind)> {
	let end = draft
		.char_indices()
		.nth(cursor)
		.map_or(draft.len(), |(i, _)| i);
	let prefix = &draft[..end];
	let (start, _) = prefix.rmatch_indices(['@', '#', ':']).find(|(start, _)| {
		prefix[..*start]
			.chars()
			.next_back()
			.is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '[' | '{'))
	})?;
	let kind = match prefix.as_bytes()[start] {
		b'#' => Kind::Channel,
		b':' => Kind::Emoji,
		_ => Kind::User,
	};
	let query = &prefix[start + 1..];
	if query.chars().count() > 64 {
		return None;
	}
	match kind {
		// Discord waits for two shortcode characters so `:)` and `10:30` never open a list.
		Kind::Emoji => {
			if query.chars().count() < 2 || !query.chars().all(|c| c.is_alphanumeric() || c == '_')
			{
				return None;
			}
		}
		Kind::User | Kind::Channel => {
			if query.chars().any(|c| {
				(c.is_whitespace() && !(kind == Kind::Channel && c == ' '))
					|| matches!(c, '<' | '>' | '@' | '`')
					|| (kind == Kind::Channel && c == '#')
			}) {
				return None;
			}
		}
	}
	Some((start..end, query, kind))
}
pub fn insert(draft: &mut String, pick: Pick) -> Option<usize> {
	if pick.range.end > draft.len()
		|| !draft.is_char_boundary(pick.range.start)
		|| !draft.is_char_boundary(pick.range.end)
	{
		return None;
	}
	let token = pick.candidate.token();
	if draft.chars().count() - draft[pick.range.clone()].chars().count() + token.chars().count()
		> client_core::MAX_CONTENT
	{
		return None;
	}
	let cursor = draft[..pick.range.start].chars().count() + token.chars().count();
	draft.replace_range(pick.range, &token);
	Some(cursor)
}
/// Rank a name against the typed query: prefix hits first, then substrings, then snowflakes.
fn rank(query: &str, name: &str, id: Id) -> Option<u8> {
	if query.is_empty() {
		return Some(1);
	}
	let name = name.to_lowercase();
	if name.starts_with(query) {
		Some(0)
	} else if name.contains(query) {
		Some(1)
	} else if id.0 != 0 && id.to_string().starts_with(query) {
		Some(2)
	} else {
		None
	}
}

/// Keep only the best bounded choices, without cloning every joined server's matching catalog.
type Ranked = ((u8, u8, u64), Candidate);

fn push_emoji(out: &mut Vec<Ranked>, key: (u8, u8, u64), candidate: impl FnOnce() -> Candidate) {
	let index = out.partition_point(|(other, _)| *other <= key);
	if index < EMOJI_LIMIT {
		if out.len() == EMOJI_LIMIT {
			out.pop();
		}
		out.insert(index, (key, candidate()));
	}
}
impl Menu {
	pub fn pointer_interacting(&self, ctx: &egui::Context, channel: Id) -> bool {
		if self.channel != Some(channel) || self.dismissed || self.candidates.is_empty() {
			return false;
		}
		let layer = egui::LayerId::new(
			egui::Order::Foreground,
			egui::Id::unique(("composer-autocomplete", self.channel)),
		);
		let pointer = ctx.input(|input| {
			(input.pointer.primary_down() || input.pointer.primary_released())
				.then(|| input.pointer.interact_pos())
				.flatten()
		});
		pointer.is_some_and(|pos| ctx.layer_id_at(pos) == Some(layer))
	}

	pub fn member_query(&self) -> &str {
		if self.kind == Some(Kind::User) && !self.dismissed {
			&self.query
		} else {
			""
		}
	}

	pub fn refresh(
		&mut self,
		state: &State,
		channel: Id,
		draft: &str,
		cursor: Option<usize>,
		users: &[User],
	) {
		let Some((range, query, kind)) = cursor.and_then(|cursor| query(draft, cursor)) else {
			*self = Self::default();
			return;
		};
		if self.channel != Some(channel)
			|| self.generation != state.generation
			|| self.range != range
			|| self.query != query
			|| self.kind != Some(kind)
		{
			self.dismissed = false;
			self.selected = 0;
			self.follow = true;
		}
		self.channel = Some(channel);
		self.generation = state.generation;
		self.range = range;
		self.query = query.into();
		self.kind = Some(kind);
		let query = query.to_lowercase();
		let guild = state
			.channels
			.iter()
			.find(|c| c.id == channel)
			.and_then(|c| c.guild);
		let mut ranked: Vec<Ranked> = match kind {
			Kind::User => {
				let mut ranked = users
					.iter()
					.filter_map(|user| {
						let label = mention_label(
							user.id,
							std::slice::from_ref(user),
							Some(&MentionSource { state, channel }),
						);
						let name = label.strip_prefix('@').unwrap_or(&label);
						rank(&query, name, user.id)
							.or_else(|| rank(&query, &user.name, user.id))
							.or_else(|| {
								let search = &state.member_search[0];
								(search.request.as_ref().is_some_and(|r| {
									r.channel == channel && r.query.to_lowercase() == query
								}) && search.rows.iter().any(|m| m.user.id == user.id))
								.then_some(0)
							})
							.map(|r| {
								(
									(r, 0, 0),
									Candidate::User {
										user: user.clone(),
										name: name.to_owned(),
									},
								)
							})
					})
					.collect::<Vec<_>>();
				for role in known_roles(state, channel)
					.iter()
					.filter(|role| Some(role.id) != guild)
				{
					if let Some(rank) = rank(&query, &role.name, role.id) {
						ranked.push((
							(rank, 1, role.id.0),
							Candidate::Role {
								id: role.id,
								name: role.name.chars().take(120).collect(),
							},
						));
					}
				}
				if state.permission(channel, model::permissions::MENTION_EVERYONE) == Some(true) {
					for name in ["everyone", "here"] {
						if let Some(rank) = rank(&query, name, Id(0)) {
							ranked.push(((rank, 0, 0), Candidate::Mass { name }));
						}
					}
				}
				ranked
			}
			Kind::Channel => state
				.channels
				.iter()
				.filter(|c| {
					guild.is_some()
						&& c.guild == guild
						&& matches!(c.kind, 0 | 5 | 10..=12 | 15 | 16)
				})
				.filter_map(|c| {
					rank(&query, &c.name, c.id).map(|r| {
						(
							(r, 0, 0),
							Candidate::Channel {
								id: c.id,
								name: c.name.chars().take(120).collect(),
							},
						)
					})
				})
				.collect(),
			Kind::Emoji => {
				let mut out = Vec::with_capacity(EMOJI_LIMIT);
				for guild in &state.guilds {
					let source_match = rank(&query, &guild.name, Id(0)).map(|_| 2);
					for emoji in guild.emojis.iter().flatten() {
						if let Some(rank) = rank(&query, &emoji.name, Id(0)).or(source_match)
							&& state
								.custom_emoji_unavailable_reason(channel, guild.id, emoji)
								.is_none()
						{
							push_emoji(&mut out, (rank, 0, emoji.id.0), || Candidate::Custom {
								id: emoji.id,
								name: emoji.name.clone(),
								animated: emoji.animated,
								server: guild.name.chars().take(120).collect(),
							});
						}
					}
				}
				for (index, ((text, _), code)) in crate::emoji_picker::standard()
					.iter()
					.zip(crate::emoji_picker::shortcodes())
					.enumerate()
				{
					if let Some(rank) = rank(&query, &code[1..code.len() - 1], Id(0)) {
						push_emoji(&mut out, (rank, 1, index as u64), || Candidate::Unicode {
							text,
							code,
						});
					}
				}
				out
			}
		};
		// An explicit snowflake can be referenced before its thread metadata is loaded.
		if kind == Kind::Channel
			&& guild.is_some()
			&& let Ok(id) = query.parse::<Id>()
			&& !state.channels.iter().any(|c| c.id == id)
		{
			ranked.push((
				(0, 0, 0),
				Candidate::Channel {
					id,
					name: format!("Unknown channel ({id})"),
				},
			));
		}
		ranked.sort_by_key(|(r, _)| *r);
		let limit = if kind == Kind::Emoji {
			EMOJI_LIMIT
		} else {
			SHORT_LIMIT
		};
		let candidates: Vec<Candidate> = ranked.into_iter().map(|(_, c)| c).take(limit).collect();
		if candidates != self.candidates {
			self.follow = true;
		}
		self.candidates = candidates;
		self.selected = self.selected.min(self.candidates.len().saturating_sub(1));
	}
	fn pick(&self, index: usize) -> Option<Pick> {
		self.candidates.get(index).cloned().map(|candidate| Pick {
			range: self.range.clone(),
			candidate,
		})
	}
	pub fn keys(&mut self, ctx: &egui::Context) -> Option<Pick> {
		if self.dismissed || self.candidates.is_empty() {
			return None;
		}
		ctx.input_mut(|input| {
			if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
				self.dismissed = true;
				return None;
			}
			if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
				self.selected = (self.selected + 1) % self.candidates.len();
				self.follow = true;
			}
			if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
				self.selected = (self.selected + self.candidates.len() - 1) % self.candidates.len();
				self.follow = true;
			}
			if input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
				|| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
			{
				return self.pick(self.selected);
			}
			None
		})
	}
	/// Float the suggestions above `anchor` (the composer frame) so the input never grows.
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		anchor: egui::Rect,
		avatars: &mut Avatars,
		demo: bool,
	) -> Option<Pick> {
		if self.dismissed || self.candidates.is_empty() {
			return None;
		}
		let colors = crate::design::palette(ui);
		let bounds = ui.ctx().content_rect().shrink(8.0);
		let width = anchor.width().min(bounds.width());
		let rows = (self.candidates.len() as f32).min(VISIBLE_ROWS);
		const HEADER: f32 = 34.0;
		const PADDING: f32 = 8.0;
		let height = (HEADER + rows * ROW + PADDING).min(bounds.height());
		let x = anchor
			.left()
			.clamp(bounds.left(), (bounds.right() - width).max(bounds.left()));
		let y = (anchor.top() - 8.0 - height).max(bounds.top());
		let header = match self.kind {
			Some(Kind::Channel) => "TEXT CHANNELS".to_owned(),
			Some(Kind::Emoji) => format!("EMOJI MATCHING :{}", self.query),
			_ => "MENTIONS".to_owned(),
		};
		let mut picked = None;
		let follow = std::mem::take(&mut self.follow);
		let selected = self.selected;
		egui::Area::new(egui::Id::unique(("composer-autocomplete", self.channel)))
			.kind(egui::UiKind::Popup)
			.order(egui::Order::Foreground)
			.fixed_pos(egui::pos2(x, y))
			.constrain_to(bounds)
			.interactable(true)
			.show(ui.ctx(), |ui| {
				egui::Frame::new()
					.fill(colors.sidebar)
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(8)
					.shadow(egui::epaint::Shadow {
						offset: [0, 8],
						blur: 24,
						spread: 0,
						color: egui::Color32::from_black_alpha(96),
					})
					.show(ui, |ui| {
						ui.set_width(width - 2.0);
						ui.style_mut().interaction.selectable_labels = false;
						ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
						ui.allocate_ui_with_layout(
							egui::vec2(width - 2.0, HEADER),
							egui::Layout::left_to_right(egui::Align::Center),
							|ui| {
								ui.add_space(12.0);
								ui.label(crate::design::eyebrow(ui, header, colors.muted));
								ui.with_layout(
									egui::Layout::right_to_left(egui::Align::Center),
									|ui| {
										ui.add_space(12.0);
										ui.label(
											egui::RichText::new(
												"↑↓ choose · Tab/Enter insert · Esc",
											)
											.size(11.0)
											.color(colors.muted),
										);
									},
								);
							},
						);
						egui::ScrollArea::vertical()
							.id_salt(("composer-autocomplete", self.kind, &self.query))
							.max_height(rows * ROW)
							.auto_shrink([false, false])
							.show(ui, |ui| {
								ui.set_width(width - 2.0);
								for (index, candidate) in self.candidates.iter().enumerate() {
									let response = row(
										ui,
										candidate,
										index == selected,
										avatars,
										demo,
										&colors,
										width - 2.0,
									);
									if index == selected && follow {
										response.scroll_to_me(None);
									}
									if response.clicked() {
										picked = Some(index);
									}
								}
							});
						ui.add_space(PADDING);
					});
			});
		picked.and_then(|index| self.pick(index))
	}
}
fn row(
	ui: &mut egui::Ui,
	candidate: &Candidate,
	selected: bool,
	avatars: &mut Avatars,
	demo: bool,
	colors: &crate::design::Palette,
	width: f32,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(egui::vec2(width, ROW), egui::Sense::click());
	if !ui.is_rect_visible(rect) {
		return response;
	}
	if selected || response.hovered() {
		ui.painter().rect_filled(
			rect.shrink2(egui::vec2(8.0, 1.0)),
			6,
			if selected {
				colors.selected
			} else {
				colors.hover
			},
		);
	}
	let icon = egui::Rect::from_center_size(
		egui::pos2(rect.left() + 12.0 + 8.0 + 12.0, rect.center().y),
		egui::Vec2::splat(24.0),
	);
	let text_left = icon.right() + 10.0;
	let source = match candidate {
		Candidate::Custom { server, .. } => Some(server.as_str()),
		Candidate::Role { .. } => Some("Role"),
		_ => None,
	};
	let primary = match candidate {
		Candidate::User { user, name } => {
			let mut child = ui.new_child(egui::UiBuilder::new().max_rect(icon));
			avatars.show(&mut child, user, 24.0, demo);
			name.clone()
		}
		Candidate::Mass { .. } | Candidate::Role { .. } => {
			let name = match candidate {
				Candidate::Mass { name } => *name,
				Candidate::Role { name, .. } => name.as_str(),
				_ => unreachable!(),
			};
			ui.painter()
				.circle_filled(icon.center(), 12.0, colors.accent);
			ui.painter().text(
				icon.center(),
				egui::Align2::CENTER_CENTER,
				"@",
				egui::FontId::proportional(16.0),
				colors.accent_text,
			);
			format!("@{name}")
		}
		Candidate::Channel { name, .. } => {
			crate::icons::paint(
				ui.painter(),
				crate::icons::Icon::Hash,
				icon.shrink(2.0),
				colors.muted,
			);
			name.clone()
		}
		Candidate::Unicode { text, code } => {
			match crate::emoji::image(ui.ctx(), text, 24.0) {
				Some(image) => image.paint_at(ui, icon),
				None => {
					ui.painter().text(
						icon.center(),
						egui::Align2::CENTER_CENTER,
						*text,
						egui::FontId::proportional(18.0),
						colors.text,
					);
				}
			}
			(*code).to_owned()
		}
		Candidate::Custom { id, name, .. } => {
			match avatars.custom_image(ui.ctx(), *id, 24.0, demo) {
				Some(image) => image.paint_at(ui, icon),
				None => {
					ui.painter().rect_filled(icon, 4, colors.hover);
				}
			}
			format!(":{name}:")
		}
	};
	let font = egui::FontId::new(15.0, crate::design::medium_family(ui.ctx()));
	let galley = ui.painter().layout_no_wrap(
		primary,
		font,
		if selected {
			colors.text_strong
		} else {
			colors.text
		},
	);
	let max_text = rect.right() - 12.0 - text_left;
	let clip = egui::Rect::from_min_max(
		egui::pos2(text_left, rect.top()),
		egui::pos2(text_left + max_text.max(0.0), rect.bottom()),
	);
	ui.painter().with_clip_rect(clip).galley(
		egui::pos2(
			text_left,
			if source.is_some() {
				rect.top() + 1.0
			} else {
				rect.center().y - galley.size().y / 2.0
			},
		),
		galley,
		colors.text,
	);
	if let Some(source) = source {
		ui.painter().with_clip_rect(clip).text(
			egui::pos2(text_left, rect.bottom() - 2.0),
			egui::Align2::LEFT_BOTTOM,
			source,
			egui::FontId::proportional(11.0),
			colors.muted,
		);
	}
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::Button,
			true,
			selected,
			match candidate {
				Candidate::User { name, .. } => name.clone(),
				Candidate::Mass { name } => format!("@{name}"),
				Candidate::Role { name, .. } => format!("@{name}, role"),
				Candidate::Channel { name, .. } => name.clone(),
				Candidate::Unicode { code, .. } => (*code).to_owned(),
				Candidate::Custom { name, server, .. } => format!("{name} from {server}"),
			},
		)
	});
	response
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::Channel;

	#[test]
	fn cross_server_emoji_search_names_sources_bounds_and_account_reset() {
		let mut state = State {
			channels: vec![channel(1, None, 1, "DM"), channel(2, None, 3, "Group DM")],
			guilds: [20, 10]
				.into_iter()
				.map(|id| model::Guild {
					stickers: None,
					id: Id(id),
					name: format!("Source{id}"),
					icon: None,
					emojis: Some(
						(1..=300)
							.map(|index| model::CustomEmoji {
								id: Id(id * 1000 + index),
								name: "same_wave".into(),
								animated: id == 10,
								available: true,
								managed: false,
								roles: Some(vec![]),
							})
							.collect(),
					),
				})
				.collect(),
			..State::default()
		};
		let mut menu = Menu::default();
		menu.refresh(&state, Id(1), ":same", Some(5), &[]);
		assert_eq!(menu.candidates.len(), EMOJI_LIMIT);
		assert_eq!(menu.candidates[0].id(), Id(10001));
		assert!(
			matches!(&menu.candidates[0], Candidate::Custom { server, .. } if server == "Source10")
		);
		let ids = menu
			.candidates
			.iter()
			.map(Candidate::id)
			.collect::<Vec<_>>();
		state.guilds.reverse();
		for guild in &mut state.guilds {
			guild.emojis.as_mut().unwrap().reverse();
		}
		state.invalidate_navigation();
		menu.refresh(&state, Id(1), ":same", Some(5), &[]);
		assert_eq!(
			menu.candidates
				.iter()
				.map(Candidate::id)
				.collect::<Vec<_>>(),
			ids
		);
		let mut draft = ":same".into();
		insert(&mut draft, menu.pick(0).unwrap()).unwrap();
		assert_eq!(draft, "<a:same_wave:10001> ");
		menu.refresh(&state, Id(2), ":source20", Some(9), &[]);
		assert_eq!(menu.candidates[0].id(), Id(20001));
		assert!(menu.candidates.iter().all(
			|candidate| matches!(candidate, Candidate::Custom { server, .. } if server == "Source20")
		));
		menu.selected = 10;
		menu.dismissed = true;
		state.generation += 1;
		menu.refresh(&state, Id(2), ":source20", Some(9), &[]);
		assert_eq!(menu.selected, 0);
		assert!(!menu.dismissed);
		state
			.guilds
			.iter_mut()
			.find(|g| g.id == Id(20))
			.unwrap()
			.emojis = None;
		menu.refresh(&state, Id(2), ":source20", Some(9), &[]);
		assert!(menu.candidates.is_empty());
	}
	#[test]
	fn composer_enter_accepts_profile_channel_and_mass_mentions() {
		for (draft, expected, guild, kind) in [
			("@Zo", "<@42> ", None, 1),
			("#Zo", "<#42> ", Some(Id(9)), 0),
			("@eve", "@everyone ", Some(Id(9)), 0),
		] {
			let ctx = egui::Context::default();
			let mut state = State {
				selected: Some(Id(1)),
				user: Some(user(7, "Synthetic owner")),
				freshness: model::Freshness::Fresh,
				gateway_connected: true,
				auth: client_core::auth::AuthState::Authenticated,
				..State::default()
			};
			state.channels.push(model::Channel {
				last_message: None,
				id: Id(1),
				guild,
				parent_id: None,
				position: 0,
				name: "Synthetic DM".into(),
				kind,
				recipients: vec![user(42, "Zoe")],
				member_list_id: None,
				message_count: None,
				icon: None,
			});
			if let Some(guild) = guild {
				use model::permissions as p;
				state.channels.push(channel(42, Some(guild), 0, "Zoe"));
				state.guilds.push(model::Guild {
					stickers: None,
					id: guild,
					name: "Synthetic guild".into(),
					icon: None,
					emojis: None,
				});
				state
					.permissions
					.replace(p::Snapshot {
						guilds: vec![p::Guild {
							id: guild,
							owner: Some(Id(8)),
							roles: Some(vec![p::Role {
								id: guild,
								name: String::new(),
								color: 0,
								position: 0,
								hoist: false,
								bits: p::VIEW_CHANNEL | p::SEND_MESSAGES | p::MENTION_EVERYONE,
							}]),
							member: Some(p::Member {
								roles: vec![],
								timeout_until: None,
							}),
						}],
						channels: vec![p::Channel {
							id: Id(1),
							guild,
							overwrites: Some(vec![]),
						}],
					})
					.unwrap();
			}
			assert!(state.can_compose(Id(1)));
			state.drafts.insert(Id(1), draft.into());
			let mut view = crate::MessagingUi::default();
			let mut commands = Vec::new();
			let mut editor = egui::Id::NULL;
			let mut output = ctx.run_ui(Default::default(), |ui| {
				editor = ui.make_persistent_id("message-input");
				view.composer(ui, &mut state, Id(1), &ctx, &mut commands);
			});
			output.textures_delta.clear();
			ctx.memory_mut(|m| m.request_focus(editor));
			let mut edit_state = egui::text_edit::TextEditState::load(&ctx, editor).unwrap();
			edit_state
				.cursor
				.set_char_range(Some(egui::text::CCursorRange::one(
					egui::text::CCursor::new(draft.chars().count()),
				)));
			edit_state.store(&ctx, editor);
			let mut output = ctx.run_ui(
				egui::RawInput {
					events: vec![egui::Event::Key {
						key: egui::Key::Enter,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					}],
					..Default::default()
				},
				|ui| view.composer(ui, &mut state, Id(1), &ctx, &mut commands),
			);
			output.textures_delta.clear();
			assert!(commands.is_empty());
			assert_eq!(state.drafts[&Id(1)], expected);
			assert!(view.draft_changes.contains(&Id(1)));
		}
	}
	fn user(id: u64, name: &str) -> User {
		User {
			id: Id(id),
			name: name.into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		}
	}
	#[test]
	fn unicode_cursor_exact_insertion_bounded_choices_and_keyboard() {
		assert!(query("email@example", 13).is_none());
		assert!(query("<@42>", 5).is_none());
		assert_eq!(
			query("@name#1234", 10),
			Some((0..10, "name#1234", Kind::User))
		);
		assert_eq!(query("čau @Zo", 7), Some((5..8, "Zo", Kind::User)));
		let mut menu = Menu::default();
		let users = vec![user(1, "Zoe"), user(2, "Zoë")];
		menu.refresh(&State::default(), Id(1), "čau @Zo", Some(7), &users);
		let ctx = egui::Context::default();
		let mut chosen = None;
		let mut output = ctx.run_ui(
			egui::RawInput {
				events: vec![
					egui::Event::Key {
						key: egui::Key::ArrowDown,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					},
					egui::Event::Key {
						key: egui::Key::Enter,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					},
				],
				..Default::default()
			},
			|_| {
				chosen = menu.keys(&ctx);
				assert!(!ctx.input(|i| i.key_pressed(egui::Key::Enter)));
			},
		);
		output.textures_delta.clear();
		let mut draft = "čau @Zo".into();
		assert_eq!(insert(&mut draft, chosen.unwrap()), Some(9));
		assert_eq!(draft, "čau <@2> ");
		let users = (1..=1000).map(|id| user(id, "User")).collect::<Vec<_>>();
		menu.refresh(&State::default(), Id(1), "@", Some(1), &users);
		assert_eq!(menu.candidates.len(), 8);
		menu.refresh(&State::default(), Id(1), "no query", Some(8), &users);
		assert!(menu.candidates.is_empty());
	}

	fn channel(id: u64, guild: Option<Id>, kind: u8, name: &str) -> Channel {
		Channel {
			id: Id(id),
			guild,
			kind,
			name: name.into(),
			last_message: None,
			parent_id: None,
			position: 0,
			recipients: vec![],
			member_list_id: None,
			message_count: None,
			icon: None,
		}
	}
	#[test]
	fn channel_references_scope_bound_and_insert_unicode_without_user_mentions() {
		let mut menu = Menu::default();
		let mut state = State {
			channels: vec![
				channel(1, Some(Id(9)), 0, "Home"),
				channel(2, Some(Id(8)), 0, "Žlutá other guild"),
				channel(3, None, 1, "Žlutá DM"),
				channel(4, Some(Id(9)), 2, "Žlutá voice"),
				channel(5, Some(Id(9)), 4, "Žlutá category"),
				channel(6, Some(Id(9)), 5, "Žlutá announcements"),
				channel(7, Some(Id(9)), 11, "Žlutá thread"),
				channel(8, Some(Id(9)), 15, "Žlutá forum container"),
			],
			..State::default()
		};
		assert_eq!(query("čau #Žl", 7), Some((5..9, "Žl", Kind::Channel)));
		for text in ["https://host/#name", "abc#name", "<#6>"] {
			assert!(query(text, text.chars().count()).is_none());
		}
		menu.refresh(&state, Id(1), "čau #Žl", Some(7), &[]);
		assert_eq!(
			menu.candidates.iter().map(|c| c.id()).collect::<Vec<_>>(),
			[Id(6), Id(7), Id(8)]
		);
		let ctx = egui::Context::default();
		let mut pick = None;
		ctx.run_ui(
			egui::RawInput {
				events: vec![egui::Event::Key {
					key: egui::Key::Enter,
					physical_key: None,
					pressed: true,
					repeat: false,
					modifiers: egui::Modifiers::NONE,
				}],
				..Default::default()
			},
			|_| {
				pick = menu.keys(&ctx);
				assert!(!ctx.input(|input| input.key_pressed(egui::Key::Enter)));
			},
		)
		.drop_without_applying_deltas();
		let mut draft = "čau #Žl".into();
		assert_eq!(insert(&mut draft, pick.unwrap()), Some(9));
		assert_eq!(draft, "čau <#6> ");
		menu.refresh(&state, Id(3), "#", Some(1), &[]);
		assert!(menu.candidates.is_empty());
		state
			.channels
			.extend((20..40).map(|id| channel(id, Some(Id(9)), 0, &"é".repeat(300))));
		menu.refresh(&state, Id(1), "#é", Some(2), &[]);
		assert_eq!(menu.candidates.len(), 8);
		assert!(menu.candidates.iter().all(|c| c.token().len() <= 40));
		let mut full = format!("{} #", "x".repeat(client_core::MAX_CONTENT - 2));
		menu.refresh(&state, Id(1), &full, Some(client_core::MAX_CONTENT), &[]);
		assert!(insert(&mut full, menu.pick(0).unwrap()).is_none());
	}
	#[test]
	fn emoji_shortcodes_need_two_characters_and_include_usable_server_emoji() {
		assert!(query(":h", 2).is_none());
		assert!(query(":", 1).is_none());
		assert!(query(":)", 2).is_none());
		assert!(query("10:30", 5).is_none());
		assert!(query("<:wave:9001>", 12).is_none());
		assert_eq!(query("hi :he", 6), Some((3..6, "he", Kind::Emoji)));
		let guilds = vec![model::Guild {
			stickers: None,
			id: Id(9),
			name: "Guild".into(),
			icon: None,
			emojis: Some(vec![
				model::CustomEmoji {
					id: Id(9001),
					name: "heart_hands_custom".into(),
					animated: true,
					available: true,
					managed: false,
					roles: Some(vec![]),
				},
				model::CustomEmoji {
					id: Id(9002),
					name: "hello_locked".into(),
					animated: false,
					available: true,
					managed: false,
					roles: Some(vec![Id(1)]),
				},
			]),
		}];
		let state = State {
			guilds,
			channels: vec![channel(1, None, 1, "DM")],
			user: Some(user(7, "Owner")),
			..State::default()
		};
		let mut menu = Menu::default();
		menu.refresh(&state, Id(1), "hi :he", Some(6), &[]);
		assert!(menu.candidates.len() > 8, "all matching emoji are listed");
		assert!(menu.candidates.len() <= EMOJI_LIMIT);
		assert!(matches!(
			&menu.candidates[0],
			Candidate::Custom {
				id: Id(9001),
				animated: true,
				..
			}
		));
		assert!(!menu.candidates.iter().any(|c| c.id() == Id(9002)));
		assert!(
			menu.candidates
				.iter()
				.any(|c| matches!(c, Candidate::Unicode { code, .. } if *code == ":red_heart:"))
		);
		let mut draft = "hi :he".to_owned();
		assert_eq!(insert(&mut draft, menu.pick(0).unwrap()), Some(31));
		assert_eq!(draft, "hi <a:heart_hands_custom:9001> ");
		let unicode = menu
			.candidates
			.iter()
			.position(|c| matches!(c, Candidate::Unicode { code, .. } if *code == ":red_heart:"))
			.unwrap();
		let mut draft = "hi :he".to_owned();
		insert(&mut draft, menu.pick(unicode).unwrap()).unwrap();
		assert_eq!(draft, "hi ❤️ ");
	}
}

#[cfg(debug_assertions)]
pub(crate) fn debug_member_search_check(state: &State, channel: Id) {
	let mut menu = Menu::default();
	let users = known_users(state, channel);
	menu.refresh(state, channel, "@Outside", Some(8), &users);
	let pick = menu
		.pick(0)
		.expect("remote nickname must appear in mentions");
	let mut draft = "@Outside".to_owned();
	insert(&mut draft, pick).unwrap();
	assert_eq!(draft, "<@987654321> ");
}

#[cfg(debug_assertions)]
pub(crate) fn debug_pointer_check(state: &mut State, channel: Id) {
	for draft in ["@You", ":he", "#"] {
		let ctx = egui::Context::default();
		crate::design::apply(&ctx);
		let mut view = crate::MessagingUi::default();
		state.drafts.insert(channel, draft.into());
		let mut initialized = false;
		let mut frame = |view: &mut crate::MessagingUi, state: &mut State, events| {
			let mut commands = Vec::new();
			ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(900.0, 700.0),
					)),
					events,
					..Default::default()
				},
				|ui| {
					ui.add_space(500.0);
					let editor = ui.make_persistent_id("message-input");
					if !initialized {
						ctx.memory_mut(|memory| memory.request_focus(editor));
					}
					let mut edit =
						egui::text_edit::TextEditState::load(&ctx, editor).unwrap_or_default();
					// Initialize once, then leave focus and cursor handling to the real composer.
					if !initialized {
						initialized = true;
						edit.cursor
							.set_char_range(Some(egui::text::CCursorRange::one(
								egui::text::CCursor::new(draft.chars().count()),
							)));
						edit.store(&ctx, editor);
					}
					view.composer(ui, state, channel, &ctx, &mut commands);
				},
			)
			.drop_without_applying_deltas();
			assert!(
				!commands
					.iter()
					.any(|command| matches!(command, client_core::Command::Send { .. }))
			);
		};
		for _ in 0..3 {
			frame(&mut view, state, vec![]);
		}
		let expected = view
			.mention_menu
			.pick(0)
			.expect("synthetic suggestion")
			.candidate
			.token();
		let area = ctx
			.memory(|memory| {
				memory.area_rect(egui::Id::unique(("composer-autocomplete", Some(channel))))
			})
			.unwrap();
		let point = egui::pos2(area.center().x, area.top() + 34.0 + ROW / 2.0 + 1.0);
		frame(&mut view, state, vec![egui::Event::PointerMoved(point)]);
		for pressed in [true, false] {
			frame(
				&mut view,
				state,
				vec![egui::Event::PointerButton {
					pos: point,
					button: egui::PointerButton::Primary,
					pressed,
					modifiers: egui::Modifiers::NONE,
				}],
			);
		}
		assert_eq!(
			state.drafts[&channel], expected,
			"clicking {draft} must insert a suggestion"
		);
	}
}

#[cfg(debug_assertions)]
pub fn debug_role_mentions_check(state: &mut State) {
	{
		let user = state.user.as_ref().unwrap().clone();
		let channel = Id(1);
		let mut names = State::default();
		names.apply(client_core::Envelope {
			generation: names.generation,
			event: client_core::Event::UserAction(client_core::user_actions::Event::Friends(Some(
				vec![(user.clone(), "synthetic.username".into())],
			))),
		});
		let mut author = user.clone();
		author.id = Id(user.id.0.wrapping_add(1));
		author.name = "Other".into();
		let message = model::Message {
			sticker_items: vec![],
			id: Id(2),
			channel,
			author,
			content: format!("<@{}>", user.id),
			prior_contents: Default::default(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			interaction: None,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			components: vec![],
			application_id: None,
			ephemeral: false,
			flags: 0,
			embeds: vec![],
			attachments: vec![],
			author_nick: None,
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: vec![user.clone()],
			reactions: Some(vec![]),
			embeds_suppressed: false,
		};
		for nickname in ["Private name", "Changed name", ""] {
			let before = presentation_fingerprint(&names, &message);
			names.apply(client_core::Envelope {
				generation: names.generation,
				event: client_core::Event::UserAction(client_core::user_actions::Event::Nickname {
					user: user.id,
					text: nickname.into(),
				}),
			});
			let expected = if nickname.is_empty() {
				user.name.as_str()
			} else {
				nickname
			};
			assert_ne!(presentation_fingerprint(&names, &message), before);
			assert_eq!(
				mention_label(
					user.id,
					std::slice::from_ref(&user),
					Some(&MentionSource {
						state: &names,
						channel
					})
				),
				format!("@{expected}")
			);
			let mut menu = Menu::default();
			let mut draft = format!("@{}", expected.split_whitespace().next().unwrap());
			menu.refresh(
				&names,
				channel,
				&draft,
				Some(draft.chars().count()),
				std::slice::from_ref(&user),
			);
			assert!(
				matches!(&menu.candidates[0], Candidate::User { name, .. } if name == expected)
			);
			insert(&mut draft, menu.pick(0).unwrap()).unwrap();
			assert_eq!(draft, user_mention_token(user.id));
			let original = format!("@{}", user.name.split_whitespace().next().unwrap());
			menu.refresh(
				&names,
				channel,
				&original,
				Some(original.chars().count()),
				std::slice::from_ref(&user),
			);
			assert!(!menu.candidates.is_empty());
		}
	}
	let channel = state
		.channels
		.iter()
		.find(|c| c.guild.is_some() && c.kind == 0)
		.unwrap()
		.id;
	let guild = state.channel(channel).unwrap().guild.unwrap();
	let roles = state
		.permissions
		.guilds
		.get_mut(&guild)
		.unwrap()
		.roles
		.as_mut()
		.unwrap();
	roles.push(model::permissions::Role {
		id: Id(1548470397144666162),
		name: "Role check".into(),
		bits: 0,
		color: 0xe67e22,
		position: 1,
		hoist: false,
	});
	let mut message = state.timeline.iter().next().unwrap().clone();
	message.channel = channel;
	message.mentions.clear();
	message.mention_everyone = false;
	message.mention_roles = vec![Id(1548470397144666162)];
	assert!(!crate::timeline::mentions_viewer(&message, state));
	state
		.permissions
		.guilds
		.get_mut(&guild)
		.unwrap()
		.member
		.as_mut()
		.unwrap()
		.roles
		.push(Id(1548470397144666162));
	assert!(crate::timeline::mentions_viewer(&message, state));
	message.mention_roles = vec![Id(999999)];
	assert!(!crate::timeline::mentions_viewer(&message, state));
	let mut menu = Menu::default();
	let mut draft = "@Role".to_owned();
	menu.refresh(state, channel, &draft, Some(5), &[]);
	let ctx = egui::Context::default();
	ctx.run_ui(
		egui::RawInput {
			events: vec![egui::Event::Key {
				key: egui::Key::Tab,
				physical_key: None,
				pressed: true,
				repeat: false,
				modifiers: egui::Modifiers::NONE,
			}],
			..Default::default()
		},
		|_| {
			insert(&mut draft, menu.keys(&ctx).expect("role completion")).unwrap();
		},
	)
	.drop_without_applying_deltas();
	assert_eq!(draft, "<@&1548470397144666162> ");
	assert_eq!(model::mentioned_role_ids(&draft), [Id(1548470397144666162)]);
	assert!(model::mentioned_user_ids(&draft).is_empty());
	for invalid in [
		"<@&0>",
		"<@&+2>",
		"<@&18446744073709551616>",
		"<@&2",
		"<@2>",
	] {
		assert!(model::role_mention_prefix(invalid).is_none());
	}
	let mut thread = state.channel(channel).unwrap().clone();
	thread.id = Id(1549042875830898709);
	thread.name = "Thread with spaces".into();
	thread.kind = 11;
	thread.parent_id = Some(channel);
	state.channels.push(thread);
	state.invalidate_navigation();
	let mut thread_draft = "#Thread with".to_owned();
	menu.refresh(
		state,
		channel,
		&thread_draft,
		Some(thread_draft.chars().count()),
		&[],
	);
	insert(&mut thread_draft, menu.pick(0).expect("thread with spaces")).unwrap();
	assert_eq!(thread_draft, "<#1549042875830898709> ");
	menu.refresh(state, channel, "#1549042875830898710", Some(20), &[]);
	assert_eq!(
		menu.pick(0).expect("unloaded exact ID").candidate.token(),
		"<#1549042875830898710> "
	);
	let mut forum = state.channel(channel).unwrap().clone();
	forum.id = Id(1549042875830898711);
	forum.name = "Forum check".into();
	forum.kind = 15;
	state.channels.push(forum);
	state.invalidate_navigation();
	menu.refresh(state, channel, "#Forum", Some(6), &[]);
	assert_eq!(
		menu.pick(0).expect("forum completion").candidate.token(),
		"<#1549042875830898711> "
	);
	let source = format!(
		"<#1549042875830898711> {thread_draft}{draft}Hey guys 🙂 hope you are well. `<@&1548470397144666162>` ||<@&1548470397144666162>||"
	);
	let parsed = crate::markdown::Formatted::parse(&source);
	crate::fonts::install(&ctx);
	crate::design::apply(&ctx);
	let mut avatars = Avatars::default();
	let mut composer = crate::composer_text::Layout::default();
	for dark in [true, false] {
		ctx.set_visuals(if dark {
			egui::Visuals::dark()
		} else {
			egui::Visuals::light()
		});
		let mut role_color = egui::Color32::TRANSPARENT;
		let output = ctx.run_ui(Default::default(), |ui| {
			let colors = crate::design::palette(ui);
			role_color =
				crate::design::role_name_color(0xe67e22, colors.mention_bg, colors.mention_text);
			assert_ne!(role_color, colors.mention_text);
			let roles = known_roles(state, channel);
			let mut preview = egui::text::LayoutJob::default();
			crate::markdown::Formatted::parse(&draft).append_inline_preview(
				&mut preview,
				ui,
				&[],
				None,
				roles,
				&state.channels,
			);
			assert_eq!(preview.text, "@Role check");
			assert_eq!(preview.sections[0].format.color, role_color);
			let mut profile = crate::profiles::ProfileSession::default();
			let mut surface = crate::select::Surface::new(ui, "mention-test");
			parsed.show_references(
				ui,
				&mut None,
				&[],
				None,
				&mut profile,
				(&state.channels, &mut None, &state.guilds, roles),
				(&mut avatars, true, &mut 0),
				&mut surface,
			);
			surface.finish(ui);
			assert!(profile.open_user().is_none());
			let galley = composer.galley(
				ui,
				&thread_draft,
				120.0,
				&[],
				roles,
				&state.channels,
				false,
				&mut avatars,
				true,
			);
			assert_eq!(galley.job.text, thread_draft);
		});
		fn count(shape: &egui::Shape, role_color: egui::Color32) -> usize {
			match shape {
				egui::Shape::Text(text) => {
					let value = &text.galley.job.text;
					if value.contains("@Role check") {
						assert!(
							value.contains("Hey guys"),
							"role and body must share a galley"
						);
						assert!(
							text.galley.rows.iter().any(|row| row
								.visuals
								.mesh
								.vertices
								.iter()
								.any(|vertex| vertex.color == role_color)),
							"role color must reach the painted text"
						);
						let row = &text.galley.rows[0];
						let baseline = row.glyphs.iter().find(|g| g.chr == '@').unwrap().pos.y;
						let body = row.glyphs.iter().find(|g| g.chr == 'H').unwrap().pos.y;
						assert!(
							(baseline - body).abs() <= 1.0,
							"role and body baselines must match"
						);
						return 1;
					}
					usize::from(matches!(
						value.as_str(),
						"#Thread with spaces" | "#Forum check"
					))
				}
				egui::Shape::Vec(shapes) => {
					shapes.iter().map(|shape| count(shape, role_color)).sum()
				}
				_ => 0,
			}
		}
		assert_eq!(
			output
				.shapes
				.iter()
				.map(|shape| count(&shape.shape, role_color))
				.sum::<usize>(),
			3,
			"only the plain role mention should resolve; code stays literal and spoilers stay hidden"
		);
		output.drop_without_applying_deltas();
	}
	if let Some(dm) = state.channels.iter().find(|c| c.guild.is_none()) {
		menu.refresh(state, dm.id, "@Role", Some(5), &[]);
		assert!(menu.candidates.is_empty(), "roles must not leak into DMs");
	}
}
