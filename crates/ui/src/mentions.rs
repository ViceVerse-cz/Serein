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
	cooldown_seconds: Option<u64>,
	catalog_revision: u64,
	game_status: client_core::game_mentions::Status,
	pub retry_games: bool,
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
	/// Leave custom emoji that need Nitro out of `:` suggestions.
	pub hide_nitro_emojis: bool,
}
pub struct Pick {
	range: Range<usize>,
	candidate: Candidate,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Kind {
	User,
	Game,
	Channel,
	Emoji,
}
#[derive(Clone, PartialEq, Eq)]
enum Candidate {
	GameSearch,
	Game(model::GameMention),
	Role {
		id: Id,
		name: String,
		color: u32,
		icon: Option<String>,
		unicode_emoji: Option<String>,
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
		known: bool,
	},
	Unicode {
		text: &'static str,
		code: &'static str,
	},
	Custom {
		id: Id,
		name: String,
		animated: bool,
		image_fallback: bool,
		server: String,
	},
}

pub(crate) fn user_mention_token(user: Id) -> String {
	format!("<@{user}> ")
}

fn username_label(user: &User, display: &str) -> Option<String> {
	let username = user.username.as_deref().filter(|name| !name.is_empty())?;
	let label = if user.discriminator == 0 {
		username.to_owned()
	} else {
		format!("{username}#{:04}", user.discriminator)
	};
	(label != display).then_some(label)
}

impl Candidate {
	fn section(&self) -> u8 {
		match self {
			Self::User { .. } => 0,
			Self::Role { .. } => 1,
			_ => 2,
		}
	}
	fn same_identity(&self, other: &Self) -> bool {
		match (self, other) {
			(Self::User { user: a, .. }, Self::User { user: b, .. }) => a.id == b.id,
			(Self::Role { id: a, .. }, Self::Role { id: b, .. })
			| (Self::Channel { id: a, .. }, Self::Channel { id: b, .. })
			| (Self::Custom { id: a, .. }, Self::Custom { id: b, .. }) => a == b,
			(Self::Game(a), Self::Game(b)) => a.id == b.id,
			(Self::Mass { name: a }, Self::Mass { name: b }) => a == b,
			(Self::Unicode { code: a, .. }, Self::Unicode { code: b, .. }) => a == b,
			(Self::GameSearch, Self::GameSearch) => true,
			_ => false,
		}
	}
	#[cfg(test)]
	fn id(&self) -> Id {
		match self {
			Candidate::Game(game) => game.id,
			Candidate::GameSearch => Id(0),
			Candidate::User { user, .. } => user.id,
			Candidate::Role { id, .. }
			| Candidate::Channel { id, .. }
			| Candidate::Custom { id, .. } => *id,
			Candidate::Mass { .. } | Candidate::Unicode { .. } => Id(0),
		}
	}
	fn token(&self) -> String {
		match self {
			Candidate::GameSearch => "@game ".into(),
			Candidate::Game(game) => format!("<@${}> ", game.id),
			Candidate::Role { id, .. } => format!("<@&{id}> "),
			Candidate::User { user, .. } => user_mention_token(user.id),
			Candidate::Mass { name } => format!("@{name} "),
			Candidate::Channel { id, .. } => format!("<#{id}> "),
			Candidate::Unicode { text, .. } => format!("{text} "),
			Candidate::Custom {
				id,
				name,
				animated,
				image_fallback,
				..
			} => {
				if *image_fallback {
					return format!(
						"{} ",
						model::ImageShare::Emoji {
							id: *id,
							animated: *animated
						}
						.markdown(name)
						.unwrap_or_default()
					);
				}
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

/// The icon role follows the role hierarchy independently of hoisting and name color.
pub(crate) fn member_icon_role<'a>(
	roles: &'a [model::permissions::Role],
	assigned: &[Id],
) -> Option<&'a model::permissions::Role> {
	roles
		.iter()
		.filter(|role| role.has_icon() && assigned.contains(&role.id))
		.max_by(|a, b| a.cmp_hierarchy(b))
}

pub(crate) fn show_role_icon(
	ui: &mut egui::Ui,
	avatars: &mut Avatars,
	role: &model::permissions::Role,
	size: f32,
	demo: bool,
) {
	if let Some(hash) = role
		.icon
		.as_deref()
		.filter(|hash| model::valid_avatar_hash(hash))
	{
		avatars
			.show_icon(
				ui,
				Some(format!("role-icon-{}-{hash}", role.id)),
				size,
				demo,
				&role.name,
			)
			.on_hover_text(&role.name);
	} else {
		let (rect, response) =
			ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::hover());
		if let Some(emoji) = &role.unicode_emoji {
			if let Some(image) = crate::emoji::image(ui.ctx(), emoji, size) {
				image.paint_at(ui, rect);
			} else {
				ui.painter().with_clip_rect(rect).text(
					rect.center(),
					egui::Align2::CENTER_CENTER,
					emoji,
					egui::FontId::proportional(size),
					crate::design::palette(ui).text,
				);
			}
		}
		response.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), &role.name)
		});
		response.on_hover_text(&role.name);
	}
}
pub fn known_users(state: &State, channel: Id) -> Vec<User> {
	let mut users = Vec::new();
	let mut add = |user: &User| {
		if users.len() < 256
			&& !state
				.member_identity(channel, user.id)
				.is_some_and(|update| update.departed)
			&& !users.iter().any(|u: &User| u.id == user.id)
		{
			users.push(user.clone());
		}
	};
	if let Some(request) = &state.member_search[0].request
		&& request.channel == channel
		&& state.channel(channel).and_then(|c| c.guild) == Some(request.guild)
		&& state.can_view(channel)
	{
		for member in &state.member_search[0].rows {
			add(&member.user);
		}
		let query = request.query.to_lowercase();
		for member in state.observed_display_members(channel) {
			if [
				member.nick.as_deref(),
				Some(member.user.name.as_str()),
				member.user.username.as_deref(),
			]
			.into_iter()
			.flatten()
			.any(|name| rank(&query, name, member.user.id).is_some())
			{
				add(&member.user);
			}
		}
	}
	for user in state.observed_member_users(channel) {
		add(&user);
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
	if let Some(request) = &state.member_search[0].request
		&& request.channel == channel
		&& state.channel(channel).and_then(|c| c.guild) == Some(request.guild)
		&& state.can_view(channel)
	{
		for member in &state.member_search[0].retained {
			add(&member.user);
		}
	}
	if let Some(request) = &state.member_search[1].request
		&& request.channel == channel
		&& state.channel(channel).and_then(|c| c.guild) == Some(request.guild)
		&& state.can_view(channel)
	{
		for member in state.member_search[1]
			.rows
			.iter()
			.chain(&state.member_search[1].retained)
		{
			add(&member.user);
		}
	}
	for message in state.timeline.iter().rev() {
		if message.channel == channel {
			add(&message.author);
			for user in &message.mentions {
				add(user);
			}
		}
	}

	users
}

/// Resolve only wire mentions present in the draft. Keep the autocomplete identity snapshot
/// unchanged, and borrow it when no contextual display name needs replacing.
pub(crate) fn composer_users<'a>(
	state: &State,
	channel: Id,
	draft: &str,
	users: &'a [User],
) -> std::borrow::Cow<'a, [User]> {
	let mut names = std::borrow::Cow::Borrowed(users);
	let mut rest = draft;
	for _ in 0..256 {
		let Some(start) = rest.find("<@") else {
			break;
		};
		let token = &rest[start..];
		if let Some((id, len)) = model::user_mention_prefix(token) {
			let source = MentionSource { state, channel };
			let label = mention_label(id, &names, Some(&source));
			let name = label.strip_prefix('@').unwrap_or(&label);
			if let Some(index) = names.iter().position(|user| user.id == id) {
				if names[index].name != name {
					names.to_mut()[index].name = name.to_owned();
				}
			} else {
				let resolved = find_user(id, users, Some(&source)).cloned().or_else(|| {
					state
						.member_nickname(channel, id)
						.flatten()
						.map(|nick| User {
							id,
							name: nick.to_owned(),
							username: None,
							avatar: None,
							member_avatar: None,
							discriminator: 0,
							primary_guild: None,
							cosmetics: None,
							webhook: false,
							kind: Default::default(),
						})
				});
				if let Some(mut user) = resolved {
					user.name = name.to_owned();
					names.to_mut().push(user);
				}
			}
			rest = &token[len..];
		} else {
			rest = &token[2..];
		}
	}
	names
}

// Compute all candidate recencies in one scan of the already-loaded bounded conversation.
fn recent_user_ranks(
	state: &State,
	channel: Id,
	users: &[User],
) -> std::collections::HashMap<Id, u64> {
	let mut ranks: std::collections::HashMap<_, _> = users
		.iter()
		.take(256)
		.map(|user| (user.id, u64::MAX))
		.collect();
	for message in state
		.timeline
		.iter()
		.rev()
		.filter(|message| message.channel == channel)
	{
		for user in
			std::iter::once(message.author.id).chain(message.mentions.iter().map(|user| user.id))
		{
			if let Some(rank) = ranks.get_mut(&user)
				&& *rank == u64::MAX
			{
				*rank = u64::MAX - message.id.0;
			}
		}
	}
	ranks
}

pub struct MentionSource<'a> {
	pub state: &'a State,
	pub channel: Id,
}

fn member(state: &State, channel: Id, id: Id) -> Option<&model::Member> {
	state.display_member(channel, id)
}

pub fn find_user<'a>(
	id: Id,
	mentions: &'a [User],
	source: Option<&MentionSource<'a>>,
) -> Option<&'a User> {
	if let Some(user) = source
		.and_then(|s| s.state.member_identity(s.channel, id))
		.and_then(|update| update.profile.as_ref())
	{
		return Some(user);
	}
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
		&& let Some(nick) = source
			.state
			.member_nickname(source.channel, id)
			.flatten()
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
	let content = &message.content[..message
		.content
		.floor_char_boundary(crate::markdown::MAX_INPUT)];
	if content.contains("<@$") {
		state.game_mentions.revision.hash(&mut hasher);
	}
	// Reuse the metadata revision: types, parents, guilds and resolved names can all
	// change pill geometry. Message traffic is filtered out; no Markdown parse is needed.
	if content.contains("<#") || content.contains("/channels/") {
		state.channel_labels_revision().hash(&mut hasher);
	}
	let mut rest = content;
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
		} else if let Some((_, len)) = model::channel_mention_prefix(rest) {
			rest = &rest[len..];
		} else {
			let skip = rest.chars().next().map_or(1, char::len_utf8);
			rest = &rest[skip..];
			continue;
		}
		seen += 1;
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
	let mut kind = match prefix.as_bytes()[start] {
		b'#' => Kind::Channel,
		b':' => Kind::Emoji,
		_ => Kind::User,
	};
	let mut query = &prefix[start + 1..];
	if kind == Kind::User
		&& let Some(game) = query.strip_prefix("game ")
	{
		kind = Kind::Game;
		query = game;
	}
	if query.chars().count() > 64 {
		return None;
	}
	match kind {
		// Discord waits for two shortcode characters so `:)` and `10:30` never open a list.
		Kind::Emoji => {
			if query.chars().count() < 2
				|| !query
					.chars()
					.all(|c| c.is_alphanumeric() || matches!(c, '_' | '+' | '-'))
			{
				return None;
			}
		}
		Kind::User | Kind::Channel | Kind::Game => {
			if query.chars().any(|c| {
				(c.is_whitespace() && c != ' ')
					|| matches!(c, '<' | '>' | '@' | '`')
					|| (kind == Kind::Channel && c == '#')
			}) {
				return None;
			}
		}
	}
	Some((start..end, query, kind))
}
pub fn insert(draft: &mut String, pick: Pick, editing: bool) -> Option<usize> {
	if pick.range.end > draft.len()
		|| !draft.is_char_boundary(pick.range.start)
		|| !draft.is_char_boundary(pick.range.end)
	{
		return None;
	}
	let token = pick.candidate.token();
	if draft.chars().count() - draft[pick.range.clone()].chars().count() + token.chars().count()
		> crate::emoji_picker::composer_limit(draft, editing)
	{
		return None;
	}
	let cursor = draft[..pick.range.start].chars().count() + token.chars().count();
	draft.replace_range(pick.range, &token);
	Some(cursor)
}

pub fn insert_checked(
	state: &State,
	channel: Id,
	draft: &mut String,
	pick: Pick,
	editing: bool,
) -> Option<usize> {
	if let Candidate::Channel { id, known, .. } = &pick.candidate {
		let guild = state.channel(channel)?.guild?;
		if !state.can_view(channel)
			|| !state.channel(*id).map_or(!*known, |target| {
				channel_mention_readable(state, target, Some(guild))
			}) {
			return None;
		}
	}
	insert(draft, pick, editing)
}

fn channel_mention_readable(state: &State, channel: &model::Channel, guild: Option<Id>) -> bool {
	guild.is_some()
		&& channel.guild == guild
		&& matches!(channel.kind, 0 | 5 | 10..=12 | 15 | 16)
		&& state.can_view(channel.id)
		&& state.can_read_history(channel.id)
}

/// Rank a name against the typed query: prefix hits first, then substrings, then snowflakes.
fn rank(query: &str, name: &str, id: Id) -> Option<u8> {
	if query.is_empty() {
		return Some(1);
	}
	// Most names are ASCII; compare those in place instead of allocating a lowercase copy.
	let (prefix, substring) = if name.is_ascii() {
		let (name, query) = (name.as_bytes(), query.as_bytes());
		let prefix = name.len() >= query.len() && name[..query.len()].eq_ignore_ascii_case(query);
		let substring = prefix
			|| name
				.windows(query.len())
				.any(|window| window.eq_ignore_ascii_case(query));
		(prefix, substring)
	} else {
		let name = name.to_lowercase();
		(name.starts_with(query), name.contains(query))
	};
	if prefix {
		Some(0)
	} else if substring {
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
	/// Whether [`Self::keys`] accepts Tab, so the composer must keep it from focus navigation.
	pub fn claims_tab(&self) -> bool {
		!self.dismissed
			&& (!self.candidates.is_empty()
				|| (self.kind == Some(Kind::Game)
					&& self.game_status == client_core::game_mentions::Status::Unavailable))
	}
	pub fn needs_games(&self) -> bool {
		self.kind == Some(Kind::Game) && !self.dismissed
	}
	pub fn pointer_interacting(&self, ctx: &egui::Context, channel: Id) -> bool {
		if self.channel != Some(channel)
			|| self.dismissed
			|| (self.candidates.is_empty() && self.kind != Some(Kind::Game))
		{
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
			*self = Self {
				hide_nitro_emojis: self.hide_nitro_emojis,
				..Self::default()
			};
			return;
		};
		if kind == Kind::Game
			&& self.kind == Some(kind)
			&& self.channel == Some(channel)
			&& self.generation == state.generation
			&& self.range == range
			&& self.query == query
			&& self.catalog_revision == state.game_mentions.revision
			&& self.game_status == state.game_mentions.status
		{
			return;
		}
		self.catalog_revision = state.game_mentions.revision;
		self.game_status = state.game_mentions.status;
		let previous_selection = (self.channel == Some(channel)
			&& self.generation == state.generation
			&& self.range == range
			&& self.query == query
			&& self.kind == Some(kind))
		.then(|| self.candidates.get(self.selected).cloned())
		.flatten();
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
		self.cooldown_seconds = (kind == Kind::User)
			.then(|| {
				let view = &state.member_search[0];
				view.request.as_ref().filter(|r| r.channel == channel)?;
				let remaining = view
					.cooldown_until?
					.saturating_duration_since(std::time::Instant::now());
				(!remaining.is_zero())
					.then_some(remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0))
			})
			.flatten();
		let query = query.to_lowercase();
		let guild = state
			.channels
			.iter()
			.find(|c| c.id == channel)
			.and_then(|c| c.guild);
		let mut ranked: Vec<Ranked> = match kind {
			Kind::Game => {
				let mut out = Vec::with_capacity(EMOJI_LIMIT);
				for game in state.game_mentions.games() {
					if let Some(rank) = rank(&query, &game.name, Id(0)) {
						push_emoji(&mut out, (rank, 0, game.id.0), || {
							Candidate::Game(game.clone())
						});
					}
				}
				out
			}
			Kind::User => {
				let recency = recent_user_ranks(state, channel, users);
				let mut ranked = users
					.iter()
					.filter_map(|user| {
						let label = mention_label(
							user.id,
							std::slice::from_ref(user),
							Some(&MentionSource { state, channel }),
						);
						let name = label.strip_prefix('@').unwrap_or(&label);
						[
							rank(&query, name, user.id),
							rank(&query, &user.name, user.id),
							user.username
								.as_deref()
								.and_then(|name| rank(&query, name, user.id)),
							(user.discriminator != 0 && query.contains('#'))
								.then(|| {
									user.username.as_ref().and_then(|name| {
										rank(
											&query,
											&format!("{name}#{:04}", user.discriminator),
											user.id,
										)
									})
								})
								.flatten(),
						]
						.into_iter()
						.flatten()
						.min()
						.or_else(|| {
							let search = &state.member_search[0];
							(search.request.as_ref().is_some_and(|r| {
								r.channel == channel && r.query.to_lowercase() == query
							}) && search.rows.iter().any(|m| m.user.id == user.id))
							.then_some(0)
						})
						.map(|r| {
							(
								(r, 0, recency.get(&user.id).copied().unwrap_or(u64::MAX)),
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
								color: role.color,
								icon: role.icon.clone(),
								unicode_emoji: role.unicode_emoji.clone(),
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
				if !query.is_empty() && "game".starts_with(&query) {
					ranked.push((
						(0, if query == "game" { 0 } else { 2 }, 0),
						Candidate::GameSearch,
					));
				}
				ranked
			}
			Kind::Channel => state
				.channels
				.iter()
				.filter(|c| channel_mention_readable(state, c, guild))
				.filter_map(|c| {
					rank(&query, &c.name, c.id).map(|r| {
						(
							(r, 0, 0),
							Candidate::Channel {
								id: c.id,
								name: c.name.chars().take(120).collect(),
								known: true,
							},
						)
					})
				})
				.collect(),
			Kind::Emoji => {
				let current = state.channel(channel).and_then(|c| c.guild);
				let mut out = Vec::with_capacity(EMOJI_LIMIT);
				for guild in &state.guilds {
					let source_match = rank(&query, &guild.name, Id(0)).map(|_| 2);
					for emoji in guild.emojis.iter().flatten() {
						if let Some(rank) = rank(&query, &emoji.name, Id(0)).or(source_match)
							&& emoji.valid() && (state.can_send(channel)
							|| state
								.custom_emoji_unavailable_reason(channel, guild.id, emoji)
								.is_none()) && !(self.hide_nitro_emojis
							&& state.custom_emoji_requires_nitro(channel, guild.id, emoji))
						{
							push_emoji(
								&mut out,
								(u8::from(Some(guild.id) != current), rank * 2, emoji.id.0),
								|| Candidate::Custom {
									id: emoji.id,
									name: emoji.name.clone(),
									animated: emoji.animated,
									image_fallback: !state
										.can_send_custom_emoji(channel, guild.id, emoji),
									server: guild.name.chars().take(120).collect(),
								},
							);
						}
					}
				}
				for (index, ((text, _), code)) in crate::emoji_picker::standard()
					.iter()
					.zip(crate::emoji_picker::shortcodes())
					.enumerate()
				{
					let rank = crate::emoji_picker::discord_names()[index]
						.2
						.split(',')
						.filter_map(|alias| rank(&query, alias, Id(0)))
						.min();
					if let Some(rank) = rank {
						push_emoji(&mut out, (1, rank * 2 + 1, index as u64), || {
							Candidate::Unicode { text, code }
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
					known: false,
				},
			));
		}
		ranked.sort_by_key(|(r, candidate)| {
			(
				if kind == Kind::User {
					candidate.section()
				} else {
					0
				},
				*r,
			)
		});
		let limit = if matches!(kind, Kind::Emoji | Kind::Game) {
			EMOJI_LIMIT
		} else {
			SHORT_LIMIT
		};
		let mut section_counts = [0_usize; 3];
		let candidates: Vec<Candidate> = ranked
			.into_iter()
			.map(|(_, c)| c)
			.filter(|candidate| {
				if kind != Kind::User {
					return true;
				}
				let count = &mut section_counts[usize::from(candidate.section())];
				let allowed = *count < SHORT_LIMIT;
				*count += usize::from(allowed);
				allowed
			})
			.take(if kind == Kind::User {
				SHORT_LIMIT * 3
			} else {
				limit
			})
			.collect();
		if candidates != self.candidates {
			self.follow = true;
		}
		self.selected = previous_selection
			.and_then(|previous| {
				candidates
					.iter()
					.position(|candidate| candidate.same_identity(&previous))
			})
			.unwrap_or_else(|| self.selected.min(candidates.len().saturating_sub(1)));
		self.candidates = candidates;
	}
	fn pick(&self, index: usize) -> Option<Pick> {
		self.candidates.get(index).cloned().map(|candidate| Pick {
			range: self.range.clone(),
			candidate,
		})
	}
	pub fn keys(&mut self, ctx: &egui::Context) -> Option<Pick> {
		if self.dismissed {
			return None;
		}
		if self.candidates.is_empty() {
			if self.cooldown_seconds.is_some() {
				ctx.input_mut(|input| {
					if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
						self.dismissed = true;
					}
				});
			}
			if self.kind == Some(Kind::Game) {
				ctx.input_mut(|i| {
					if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
						self.dismissed = true;
					}
					if self.game_status == client_core::game_mentions::Status::Unavailable
						&& (i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
							|| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
					{
						self.retry_games = true;
					}
				});
			}
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
		if self.dismissed
			|| (self.candidates.is_empty()
				&& self.kind != Some(Kind::Game)
				&& self.cooldown_seconds.is_none())
		{
			return None;
		}
		let colors = crate::design::palette(ui);
		let bounds = ui.ctx().content_rect().shrink(8.0);
		let width = anchor.width().min(bounds.width());
		const HEADER: f32 = 34.0;
		const PADDING: f32 = 8.0;
		let section_count = if self.kind == Some(Kind::User) {
			self.candidates
				.iter()
				.enumerate()
				.filter(|(index, candidate)| {
					*index == 0 || self.candidates[*index - 1].section() != candidate.section()
				})
				.count()
		} else {
			0
		};
		let rows = (self.candidates.len().max(1) as f32
			+ section_count as f32 * 24.0 / ROW
			+ f32::from(self.cooldown_seconds.is_some()))
		.min(VISIBLE_ROWS)
		.min(((bounds.height() - HEADER - PADDING) / ROW).max(1.0));
		let height = (HEADER + rows * ROW + PADDING).min(bounds.height());
		let x = anchor
			.left()
			.clamp(bounds.left(), (bounds.right() - width).max(bounds.left()));
		let y = (anchor.top() - 8.0 - height).max(bounds.top());
		let header = match self.kind {
			Some(Kind::Game) => {
				crate::i18n::translate_args("game-mentions-header", &[("query", &self.query)])
			}
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
								ui.add(
									egui::Label::new(crate::design::eyebrow(
										ui,
										header,
										colors.muted,
									))
									.truncate(),
								);
								if self.kind != Some(Kind::Game) {
									ui.with_layout(
										egui::Layout::right_to_left(egui::Align::Center),
										|ui| {
											ui.add_space(12.0);
											ui.label(
												egui::RichText::new(crate::i18n::translate(
													"mentions-show-choose-tab-enter-insert-esc",
												))
												.size(11.0)
												.color(colors.muted),
											);
										},
									);
								}
							},
						);
						egui::ScrollArea::vertical()
							.id_salt(("composer-autocomplete", self.kind, &self.query))
							.max_height(rows * ROW)
							.auto_shrink([false, false])
							.show(ui, |ui| {
								ui.set_width(width - 2.0);
								if let Some(seconds) = self.cooldown_seconds {
									ui.label(crate::i18n::translate_args(
										"mentions-member-cooldown",
										&[("seconds", &seconds.to_string())],
									));
								}
								if self.candidates.is_empty() && self.kind == Some(Kind::Game) {
									ui.horizontal_wrapped(|ui| {
										ui.add_space(12.0);
										let key = match self.game_status {
											client_core::game_mentions::Status::Idle
											| client_core::game_mentions::Status::Loading => "game-mentions-loading",
											client_core::game_mentions::Status::Unavailable => {
												"game-mentions-unavailable"
											}
											client_core::game_mentions::Status::Ready => {
												"game-mentions-empty"
											}
										};
										ui.label(crate::i18n::translate(key));
										if self.game_status
											== client_core::game_mentions::Status::Unavailable
											&& ui
												.button(crate::i18n::translate(
													"game-mentions-retry",
												))
												.clicked()
										{
											self.retry_games = true;
										}
									});
								}
								for (index, candidate) in self.candidates.iter().enumerate() {
									if self.kind == Some(Kind::User)
										&& (index == 0
											|| self.candidates[index - 1].section()
												!= candidate.section())
									{
										let key = match candidate.section() {
											0 => "mentions-section-members",
											1 => "mentions-section-roles",
											_ => "mentions-section-special",
										};
										ui.allocate_ui_with_layout(
											egui::vec2(width - 2.0, 24.0),
											egui::Layout::left_to_right(egui::Align::Center),
											|ui| {
												ui.add_space(12.0);
												ui.label(crate::design::eyebrow(
													ui,
													crate::i18n::translate(key),
													colors.muted,
												));
											},
										);
									}
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
		_ => None,
	};
	let username = match candidate {
		Candidate::User { user, name } => username_label(user, name),
		_ => None,
	};
	let primary = match candidate {
		Candidate::GameSearch => {
			crate::icons::paint(
				ui.painter(),
				crate::icons::Icon::GameController,
				icon.shrink(2.0),
				colors.muted,
			);
			"@game".into()
		}
		Candidate::Game(game) => {
			let mut child = ui.new_child(egui::UiBuilder::new().max_rect(icon));
			avatars.show_icon(&mut child, Some(game.icon_key()), 24.0, demo, &game.name);
			game.name.clone()
		}
		Candidate::User { user, name } => {
			let mut child = ui.new_child(egui::UiBuilder::new().max_rect(icon));
			avatars.show(&mut child, user, 24.0, demo);
			name.clone()
		}
		Candidate::Role {
			id,
			name,
			color,
			icon: hash,
			unicode_emoji,
		} => {
			if let Some(hash) = hash
				.as_deref()
				.filter(|hash| model::valid_avatar_hash(hash))
			{
				let mut child = ui.new_child(egui::UiBuilder::new().max_rect(icon));
				avatars.show_icon(
					&mut child,
					Some(format!("role-icon-{id}-{hash}")),
					24.0,
					demo,
					name,
				);
			} else if let Some(emoji) = unicode_emoji {
				if let Some(image) = crate::emoji::image(ui.ctx(), emoji, 24.0) {
					image.paint_at(ui, icon);
				} else {
					ui.painter().with_clip_rect(icon).text(
						icon.center(),
						egui::Align2::CENTER_CENTER,
						emoji,
						egui::FontId::proportional(20.0),
						colors.text,
					);
				}
			} else {
				let color = crate::design::role_name_color(*color, colors.sidebar, colors.accent);
				ui.painter().circle_filled(icon.center(), 12.0, color);
				ui.painter().text(
					icon.center(),
					egui::Align2::CENTER_CENTER,
					"@",
					egui::FontId::proportional(16.0),
					colors.sidebar,
				);
			}
			format!("@{name}")
		}
		Candidate::Mass { name } => {
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
	let text_color = match candidate {
		Candidate::Role { color, .. } => crate::design::role_name_color(
			*color,
			if selected {
				colors.selected
			} else if response.hovered() {
				colors.hover
			} else {
				colors.sidebar
			},
			colors.text,
		),
		_ if selected => colors.text_strong,
		_ => colors.text,
	};
	let galley = ui.painter().layout_no_wrap(primary, font, text_color);
	let max_text = rect.right() - 12.0 - text_left;
	let username = username.map(|text| {
		ui.painter()
			.layout_no_wrap(text, egui::FontId::proportional(13.0), colors.muted)
	});
	let username_width = username
		.as_ref()
		.map_or(0.0, |galley| galley.size().x.min(max_text.max(0.0) * 0.45));
	let clip = egui::Rect::from_min_max(
		egui::pos2(text_left, rect.top()),
		egui::pos2(
			text_left
				+ (max_text - username_width - if username.is_some() { 12.0 } else { 0.0 })
					.max(0.0),
			rect.bottom(),
		),
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
		text_color,
	);
	if let Some(username) = username {
		let left = rect.right() - 12.0 - username_width;
		let clip = egui::Rect::from_min_max(
			egui::pos2(left, rect.top()),
			egui::pos2(rect.right() - 12.0, rect.bottom()),
		);
		ui.painter().with_clip_rect(clip).galley(
			egui::pos2(left, rect.center().y - username.size().y / 2.0),
			username,
			colors.muted,
		);
	}
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
				Candidate::GameSearch => "@game, search games".into(),
				Candidate::Game(game) => format!("{}, game", game.name),
				Candidate::User { user, name } => username_label(user, name)
					.map_or_else(|| name.clone(), |username| format!("{name}, {username}")),
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
	#[test]
	fn queued_member_search_has_feedback_without_consuming_selection() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let client_core::Command::MemberSearch(request) = state
			.search_members(channel, "NobodyMatchesThis", 0)
			.unwrap()
		else {
			panic!("member query");
		};
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberSearchCooldown {
				request,
				seconds: 30,
			},
		});
		let mut menu = super::Menu::default();
		menu.refresh(
			&state,
			channel,
			"@NobodyMatchesThis",
			Some(18),
			&super::known_users(&state, channel),
		);
		assert!(menu.cooldown_seconds.is_some_and(|n| n > 0 && n <= 30));
		assert!(menu.candidates.is_empty());
		assert!(!state.member_search[0].finished);
		assert!(state.member_search[0].error.is_none());
		assert!(menu.pick(0).is_none());
		menu.refresh(&state, channel, "ordinary text", Some(13), &[]);
		assert!(menu.cooldown_seconds.is_none());
	}
	use super::*;

	#[test]
	fn icon_role_is_independent_of_color_hoist_and_assignment_order() {
		let state = test_support::demo_state();
		let guild = state
			.channel(state.selected.unwrap())
			.unwrap()
			.guild
			.unwrap();
		let template = state.guild_roles(guild).unwrap()[0].clone();
		let role = |id, position, color, hoist, emoji: Option<&str>| {
			let mut role = template.clone();
			role.id = Id(id);
			role.position = position;
			role.color = color;
			role.hoist = hoist;
			role.icon = None;
			role.unicode_emoji = emoji.map(str::to_owned);
			role
		};
		let roles = [
			role(100, 9, 0xff0000, true, None),
			role(102, 3, 0, false, Some("⭐")),
			role(101, 3, 0, false, Some("🏅")),
			role(103, 10, 0, false, Some("🚫")),
		];
		assert_eq!(
			member_icon_role(&roles, &[Id(100), Id(102), Id(101)])
				.unwrap()
				.id,
			Id(101)
		);
		assert!(member_icon_role(&roles, &[Id(100)]).is_none());
	}

	#[test]
	fn member_and_role_sections_have_independent_caps_and_stable_keyboard_identity() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let roles = state
			.permissions
			.guilds
			.get_mut(&guild)
			.unwrap()
			.roles
			.as_mut()
			.unwrap();
		let template = roles[0].clone();
		roles.extend((10000..10012).map(|id| {
			let mut role = template.clone();
			role.id = Id(id);
			role.name = format!("SharedRole{id}");
			role
		}));
		let mut users = (20000..20012)
			.map(|id| user(id, "Shared nickname"))
			.collect::<Vec<_>>();
		for user in &mut users {
			user.username = Some(format!("account_{}", user.id).into_boxed_str());
		}
		let mut menu = Menu::default();
		menu.refresh(&state, channel, "@Shared", Some(7), &users);
		assert_eq!(menu.candidates.len(), 16);
		assert!(
			menu.candidates[..8]
				.iter()
				.all(|c| matches!(c, Candidate::User { .. }))
		);
		assert!(
			menu.candidates[8..]
				.iter()
				.all(|c| matches!(c, Candidate::Role { .. }))
		);
		menu.selected = 5;
		let selected = menu.candidates[5].id();
		users.insert(0, user(19999, "Shared arrival"));
		menu.refresh(&state, channel, "@Shared", Some(7), &users);
		assert_eq!(menu.candidates[menu.selected].id(), selected);
		menu.selected = 7;
		let ctx = egui::Context::default();
		let output = ctx.run_ui(
			egui::RawInput {
				events: [egui::Key::ArrowDown, egui::Key::Enter]
					.into_iter()
					.map(|key| egui::Event::Key {
						key,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					})
					.collect(),
				..Default::default()
			},
			|_| {
				assert!(
					menu.keys(&ctx)
						.unwrap()
						.candidate
						.token()
						.starts_with("<@&")
				);
			},
		);
		output.drop_without_applying_deltas();
		menu.refresh(&state, channel, "@account_20001", Some(14), &users);
		assert_eq!(menu.candidates.len(), 1);
		assert_eq!(menu.candidates[0].id(), Id(20001));
		assert_eq!(
			username_label(&users[2], "Shared nickname"),
			Some("account_20001".into())
		);
		let mut legacy = users[2].clone();
		legacy.discriminator = 42;
		assert_eq!(
			username_label(&legacy, "Shared nickname"),
			Some("account_20001#0042".into())
		);
	}

	#[test]
	fn current_server_suggestions_beat_external_prefixes_and_survive_the_cap() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let mut external = state.guilds[0].clone();
		external.id = Id(2);
		let local = state.guilds[0].emojis.as_ref().unwrap()[0].clone();
		external.emojis = Some(
			(1..=EMOJI_LIMIT + 10)
				.map(|i| model::CustomEmoji {
					id: Id(i as u64),
					name: "wave".into(),
					..local.clone()
				})
				.collect(),
		);
		state.guilds.insert(0, external);
		let mut menu = Menu::default();
		menu.refresh(&state, channel, ":wave", Some(5), &[]);
		assert_eq!(menu.candidates.len(), EMOJI_LIMIT);
		assert_eq!(
			menu.candidates[0].id(),
			local.id,
			"local substring beats external prefix"
		);
		state.guilds.reverse();
		menu.refresh(&state, channel, ":wave", Some(5), &[]);
		assert_eq!(menu.candidates[0].id(), local.id);
		state
			.channels
			.iter_mut()
			.find(|c| c.id == channel)
			.unwrap()
			.guild = Some(Id(2));
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		menu.refresh(&state, channel, ":wave", Some(5), &[]);
		assert_eq!(menu.candidates[0].id(), Id(1));
	}

	use model::Channel;

	#[test]
	fn searched_nicknames_label_suggestions_and_insert_user_ids() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let client_core::Command::MemberSearch(request) =
			state.search_members(channel, "Shared", 0).unwrap()
		else {
			panic!("synthetic member search");
		};
		let rows: Vec<_> = [90001, 90002]
			.into_iter()
			.map(|id| model::Member {
				user: user(id, &format!("Account{id}")),
				roles: vec![],
				nick: Some("Shared nickname".into()),
				status: None,
				custom_status: None,
				activities: vec![],
				clients: Default::default(),
			})
			.collect();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberSearch {
				request,
				result: Ok(rows),
			},
		});
		let mut menu = Menu::default();
		menu.refresh(
			&state,
			channel,
			"@Shared",
			Some(7),
			&known_users(&state, channel),
		);
		assert_eq!(menu.candidates.len(), 2);
		for (index, candidate) in menu.candidates.iter().enumerate() {
			assert!(matches!(candidate, Candidate::User { name, .. } if name == "Shared nickname"));
			let mut draft = "@Shared".to_owned();
			insert(&mut draft, menu.pick(index).unwrap(), false).unwrap();
			assert_eq!(draft, format!("<@{}> ", candidate.id()));
		}
		let target = Id(90001);
		let label =
			|state: &State| mention_label(target, &[], Some(&MentionSource { state, channel }));
		assert_eq!(label(&state), "@Shared nickname");
		let users = known_users(&state, channel);
		let display = composer_users(&state, channel, "<@90001> <@90002>", &users);
		assert_eq!(
			display.iter().find(|user| user.id == target).unwrap().name,
			"Shared nickname"
		);
		assert_eq!(
			users.iter().find(|user| user.id == target).unwrap().name,
			"Account90001"
		);
		assert!(matches!(
			composer_users(&state, channel, "plain text", &users),
			std::borrow::Cow::Borrowed(_)
		));

		state.member_search[0].rows[0].nick = Some("Renamed".into());
		assert_eq!(label(&state), "@Renamed");
		assert_eq!(
			composer_users(&state, channel, "<@90001>", &users)
				.iter()
				.find(|user| user.id == target)
				.unwrap()
				.name,
			"Renamed"
		);

		let _ = state.search_members(channel, "Different", 0).unwrap();
		assert_eq!(
			label(&state),
			"@Renamed",
			"an inserted mention keeps its label while another search runs"
		);
		state.member_search[0].rows = std::mem::take(&mut state.member_search[0].retained);
		state.member_search[0].rows[0].nick = None;
		assert_eq!(label(&state), "@Account90001");
		state.member_search[0].request.as_mut().unwrap().guild = Id(99999);
		let mut injected = state.member_search[0].rows[0].clone();
		injected.user.id = Id(999900);
		state.member_search[0].rows.push(injected);
		assert!(state.display_member(channel, Id(999900)).is_none());
		assert_eq!(
			state
				.display_member(channel, target)
				.unwrap()
				.nick
				.as_deref(),
			Some("Shared nickname")
		);
		state.member_search[0].request.as_mut().unwrap().guild =
			state.channel(channel).unwrap().guild.unwrap();
		state.member_search[0].request.as_mut().unwrap().channel = Id(99999);
		assert!(state.display_member(channel, Id(999900)).is_none());
		assert!(state.display_member(channel, target).is_some());
		state.channels.retain(|row| row.id != channel);
		assert!(state.display_member(channel, target).is_none());
	}

	#[test]
	fn draft_mention_resolves_a_known_member_outside_the_suggestion_snapshot() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let target = Id(99123);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
				guild,
				user: target,
				profile: Some(user(target.0, "Global display")),
				global_name: model::GlobalNameUpdate::Set,
				avatar: model::Patch::Absent,
				nick: model::Patch::Value("Server nickname".into()),
				roles: model::Patch::Absent,
				departed: false,
			})),
		});
		let users = composer_users(&state, channel, "<@99123> ", &[]);
		assert_eq!(users.len(), 1);
		assert_eq!(users[0].id, target);
		assert_eq!(users[0].name, "Server nickname");
	}

	#[test]
	fn nickname_completion_survives_query_edits_and_live_member_changes() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let mut search = crate::member_search::Search::default();
		let ctx = egui::Context::default();
		let mut commands = vec![];
		let mut frame = |search: &mut crate::member_search::Search,
		                 state: &mut State,
		                 text: &str,
		                 time: f64| {
			search.text = text.into();
			let mut output = ctx.run_ui(
				egui::RawInput {
					time: Some(time),
					..Default::default()
				},
				|_| search.sync(&ctx, state, channel, 0, &mut commands),
			);
			output.textures_delta.clear();
		};
		frame(&mut search, &mut state, "Guild", 0.0);
		frame(&mut search, &mut state, "Guild", 0.4);
		let request = state.member_search[0].request.clone().unwrap();
		let target = Id(99001);
		let member = model::Member {
			user: user(target.0, "Account"),
			nick: Some("Guild nickname".into()),
			roles: vec![],
			status: None,
			custom_status: None,
			activities: vec![],
			clients: Default::default(),
		};
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberSearch {
				request,
				result: Ok(vec![member]),
			},
		});
		frame(&mut search, &mut state, "Guildn", 0.5);
		assert!(
			state.display_member(channel, target).is_some(),
			"typing cannot erase a resolved member while the next query is cooling down"
		);
		frame(&mut search, &mut state, "Guild", 0.6);
		let mut menu = Menu::default();
		menu.refresh(
			&state,
			channel,
			"@Guild",
			Some(6),
			&known_users(&state, channel),
		);
		assert!(
			menu.candidates
				.iter()
				.any(|candidate| candidate.id() == target)
		);
		let update = |nick| model::MemberIdentity {
			global_name: model::GlobalNameUpdate::Unchanged,
			avatar: model::Patch::Absent,
			guild,
			user: target,
			profile: None,
			nick,
			roles: model::Patch::Absent,
			departed: false,
		};
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberIdentity(Box::new(update(model::Patch::Value(
				"Renamed".into(),
			)))),
		});
		menu.refresh(
			&state,
			channel,
			"@Renamed",
			Some(8),
			&known_users(&state, channel),
		);
		let candidate = menu
			.candidates
			.iter()
			.find(|candidate| candidate.id() == target)
			.unwrap();
		assert_eq!(candidate.token(), "<@99001> ");
		assert_eq!(
			mention_label(
				target,
				&[],
				Some(&MentionSource {
					state: &state,
					channel
				})
			),
			"@Renamed"
		);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberIdentity(Box::new(update(model::Patch::Null))),
		});
		assert_eq!(
			mention_label(
				target,
				&[],
				Some(&MentionSource {
					state: &state,
					channel
				})
			),
			"@Account"
		);
		let mut gone = update(model::Patch::Null);
		gone.departed = true;
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberIdentity(Box::new(gone)),
		});
		assert!(
			!known_users(&state, channel)
				.iter()
				.any(|user| user.id == target)
		);
	}

	#[test]
	fn disk_search_hints_produce_id_mentions_while_the_gateway_lookup_remains_pending() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let client_core::Command::MemberSearch(request) =
			state.search_members(channel, "PersistedNick", 0).unwrap()
		else {
			panic!("member query");
		};
		let revision = state.member_display_revision();
		assert!(state.hydrate_member_search_displays(
			&request,
			revision,
			vec![model::MemberDisplay {
				guild,
				user: Id(900_003),
				name: "Display name".into(),
				username: Some("actual_account_name".into()),
				nick: Some("PersistedNick".into()),
			}]
		));
		let mut menu = Menu::default();
		let mut draft = "@PersistedNick".to_owned();
		menu.refresh(
			&state,
			channel,
			&draft,
			Some(draft.len()),
			&known_users(&state, channel),
		);
		let index = menu
			.candidates
			.iter()
			.position(|candidate| candidate.id() == Id(900_003))
			.expect("cached nickname candidate");
		assert!(
			matches!(&menu.candidates[index], Candidate::User {user,name} if name == "PersistedNick" && user.username.as_deref() == Some("actual_account_name"))
		);
		insert(&mut draft, menu.pick(index).unwrap(), false).unwrap();
		assert_eq!(draft, "<@900003> ");
		assert!(!state.member_search[0].finished);
		assert!(state.member_search[0].rows.is_empty());
	}

	#[test]
	fn sqlite_nickname_search_inserts_the_selected_id_and_refreshes_its_label() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let account = state.user.as_ref().unwrap().id;
		let target = Id(900_007);
		let mut store = local_store::LocalStore::open(std::path::Path::new(":memory:")).unwrap();
		store
			.save_member_displays(
				account,
				&[model::MemberDisplay {
					guild,
					user: target,
					name: "Global display".into(),
					username: Some("actual_account".into()),
					nick: Some("Cached nickname".into()),
				}],
			)
			.unwrap();
		let client_core::Command::MemberSearch(request) =
			state.search_members(channel, "Cached nickname", 0).unwrap()
		else {
			panic!("member query");
		};
		let revision = state.member_display_revision();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberSearch {
				request: request.clone(),
				result: Ok(vec![]),
			},
		});
		let rows = store
			.search_member_displays(account, guild, "cached nickname", 100)
			.unwrap();
		assert!(state.hydrate_member_search_displays(&request, revision, rows));
		let mut draft = "@Cached nickname".to_owned();
		let mut menu = Menu::default();
		menu.refresh(
			&state,
			channel,
			&draft,
			Some(draft.chars().count()),
			&known_users(&state, channel),
		);
		let index = menu
			.candidates
			.iter()
			.position(|row| row.id() == target)
			.unwrap();
		assert!(
			matches!(&menu.candidates[index], Candidate::User { user, name }
			if name == "Cached nickname" && user.username.as_deref() == Some("actual_account"))
		);
		insert(&mut draft, menu.pick(index).unwrap(), false).unwrap();
		assert_eq!(draft, "<@900007> ");
		assert_eq!(
			composer_users(&state, channel, &draft, &[])[0].name,
			"Cached nickname"
		);

		let mut profile = user(target.0, "Global display");
		profile.username = Some("actual_account".into());
		for (nick, expected) in [
			(
				model::Patch::Value("Updated nickname".into()),
				"Updated nickname",
			),
			(model::Patch::Null, "Global display"),
		] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
					guild,
					user: target,
					profile: Some(profile.clone()),
					global_name: model::GlobalNameUpdate::Set,
					avatar: model::Patch::Absent,
					nick,
					roles: model::Patch::Absent,
					departed: false,
				})),
			});
			assert_eq!(
				composer_users(&state, channel, &draft, &[])[0].name,
				expected
			);
			assert_eq!(draft, "<@900007> ");
		}
	}

	#[test]
	fn cached_partial_member_remains_searchable_when_identity_snapshot_is_full() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let target = Id(900_007);
		state.request_members().unwrap();
		let mut list = state.members.clone().unwrap();
		list.slots = vec![Some(model::MemberSlot::Person(model::Member {
			user: user(target.0, "Account name"),
			nick: None,
			roles: vec![],
			status: None,
			custom_status: None,
			activities: vec![],
			clients: Default::default(),
		}))];
		list.missing_identity = vec![model::MissingMemberFields {
			user: target,
			nick: true,
			roles: true,
			global_name: model::GlobalNameUpdate::Set,
		}];
		list.freshness = model::Freshness::Fresh;
		list.total = 1;
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Members(list),
		});
		for id in 10_000..10_128 {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::MemberIdentity(Box::new(model::MemberIdentity {
					guild,
					user: Id(id),
					profile: None,
					global_name: model::GlobalNameUpdate::Unchanged,
					avatar: model::Patch::Absent,
					nick: model::Patch::Value(format!("Live {id}")),
					roles: model::Patch::Absent,
					departed: false,
				})),
			});
		}
		let rows = (20_000..20_128)
			.map(|id| model::MemberDisplay {
				guild,
				user: Id(id),
				name: format!("Cached {id}"),
				username: None,
				nick: None,
			})
			.collect();
		state.hydrate_member_displays(guild, state.member_display_revision(), rows);
		let client_core::Command::MemberSearch(request) =
			state.search_members(channel, "Target nickname", 0).unwrap()
		else {
			panic!("member query");
		};
		assert!(state.hydrate_member_search_displays(
			&request,
			state.member_display_revision(),
			vec![model::MemberDisplay {
				guild,
				user: target,
				name: "Account name".into(),
				username: None,
				nick: Some("Target nickname".into()),
			}]
		));
		let users = known_users(&state, channel);
		assert!(users.iter().any(|user| user.id == target));
		let draft = "@Target nickname";
		let mut menu = Menu::default();
		menu.refresh(&state, channel, draft, Some(draft.chars().count()), &users);
		let candidate = menu
			.candidates
			.iter()
			.find(|row| row.id() == target)
			.expect("cached target");
		assert!(matches!(candidate, Candidate::User { name, .. } if name == "Target nickname"));
		assert_eq!(candidate.token(), "<@900007> ");
	}

	#[test]
	fn embedded_message_nickname_is_searchable_before_sidebar_or_lookup() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let mut message = test_support::message(999999, channel);
		message.author = user(90003, "Account");
		message.author_nick = Some("Historical nickname".into());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Message(message),
		});
		let mut menu = Menu::default();
		menu.refresh(
			&state,
			channel,
			"@Historical nickname",
			Some(20),
			&known_users(&state, channel),
		);
		assert!(
			menu.candidates
				.iter()
				.any(|candidate| candidate.id() == Id(90003))
		);
	}

	#[test]
	fn author_lookup_nickname_is_used_without_loading_the_member_sidebar() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let client_core::Command::MemberSearch(request) =
			state.request_author_members(&[Id(90003)]).unwrap()
		else {
			panic!("synthetic author lookup");
		};
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::MemberSearch {
				request,
				result: Ok(vec![model::Member {
					user: user(90003, "Account name"),
					roles: vec![],
					nick: Some("Server nickname".into()),
					status: None,
					custom_status: None,
					activities: vec![],
					clients: Default::default(),
				}]),
			},
		});
		assert_eq!(
			mention_label(
				Id(90003),
				&[],
				Some(&MentionSource {
					state: &state,
					channel
				})
			),
			"@Server nickname"
		);
		state.member_search = Default::default();
		assert_eq!(
			state
				.display_member(channel, Id(90003))
				.unwrap()
				.nick
				.as_deref(),
			Some("Server nickname")
		);
		assert!(state.request_author_members(&[Id(90003)]).is_none());
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Unavailable(channel),
		});
		assert!(state.display_member(channel, Id(90003)).is_none());
	}

	#[test]
	fn future_messages_and_revisited_authors_reuse_known_nicknames_and_explicit_clears() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		for id in [90003, 90004, 90005] {
			let client_core::Command::MemberSearch(request) =
				state.request_author_members(&[Id(id)]).unwrap()
			else {
				panic!("author lookup")
			};
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::MemberSearch {
					request,
					result: Ok(vec![model::Member {
						user: user(id, "Account"),
						nick: Some(format!("Nickname {id}")),
						roles: vec![],
						status: None,
						custom_status: None,
						activities: vec![],
						clients: Default::default(),
					}]),
				},
			});
		}
		assert!(
			state.request_author_members(&[Id(90003)]).is_none(),
			"revisiting a resolved author must not start another batch"
		);
		let mut message = test_support::message(777, channel);
		message.author = user(90003, "Account");
		assert_eq!(state.message_author_name(&message), "Nickname 90003");
		message.author_nick = Some("Stale nickname".into());
		let mut member = state.member_search[1]
			.retained
			.iter()
			.find(|member| member.user.id == Id(90003))
			.unwrap()
			.clone();
		state.request_members();
		for nick in [Some("Updated nickname"), None] {
			member.nick = nick.map(str::to_owned);
			let mut list = state.members.as_ref().unwrap().clone();
			list.slots = vec![Some(model::MemberSlot::Person(member.clone()))];
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::Members(list.clone()),
			});
			// Moving the sidebar away must not reveal the older lookup result again.
			list.slots.clear();
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::Members(list),
			});
			assert_eq!(
				state.message_author_name(&message),
				nick.unwrap_or("Account")
			);
		}
		assert_eq!(
			state.message_author_name(&message),
			"Account",
			"an explicit nickname removal wins over old message metadata"
		);
	}

	#[test]
	fn game_empty_search_keeps_editor_keys_unless_retrying() {
		use client_core::game_mentions::Status;
		for status in [
			Status::Idle,
			Status::Loading,
			Status::Ready,
			Status::Unavailable,
		] {
			for key in [egui::Key::Enter, egui::Key::Tab, egui::Key::Escape] {
				let mut menu = Menu {
					kind: Some(Kind::Game),
					game_status: status,
					..Default::default()
				};
				let ctx = egui::Context::default();
				let output = ctx.run_ui(
					egui::RawInput {
						events: vec![egui::Event::Key {
							key,
							physical_key: None,
							pressed: true,
							repeat: false,
							modifiers: egui::Modifiers::NONE,
						}],
						..Default::default()
					},
					|_| {
						assert!(menu.keys(&ctx).is_none());
						let retry = key != egui::Key::Escape && status == Status::Unavailable;
						assert_eq!(menu.retry_games, retry);
						assert_eq!(menu.dismissed, key == egui::Key::Escape);
						assert_eq!(
							ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)),
							key != egui::Key::Escape && !retry
						);
					},
				);
				output.drop_without_applying_deltas();
			}
		}
	}

	#[test]
	fn game_pointer_selection_uses_the_composer_without_sending() {
		let mut state = test_support::demo_state();
		debug_pointer_check(&mut state, Id(20));
	}

	#[test]
	fn game_search_bounds_large_catalog_and_reuses_results() {
		let mut state = test_support::demo_state();
		state.game_mentions.finish(Ok((1..=24_000)
			.map(|id| model::GameMention {
				id: Id(id),
				name: format!("Many Game {id}"),
				icon: None,
			})
			.collect()));
		let mut menu = Menu::default();
		menu.refresh(&state, Id(20), "@game many", Some(10), &[]);
		assert_eq!(menu.candidates.len(), EMOJI_LIMIT);
		let candidates = menu.candidates.as_ptr();
		menu.refresh(&state, Id(20), "@game many", Some(10), &[]);
		assert_eq!(candidates, menu.candidates.as_ptr());
	}

	#[test]
	fn game_search_inserts_a_wire_token_and_keeps_other_draft_text() {
		let mut state = test_support::demo_state();
		state.game_mentions.finish(Ok(vec![
			model::GameMention {
				id: Id(42),
				name: "Deadlock".into(),
				icon: None,
			},
			model::GameMention {
				id: Id(43),
				name: "Red Dead Redemption 2".into(),
				icon: None,
			},
		]));
		let mut menu = Menu::default();
		let channel = Id(20);
		let mut draft = "Play @game".to_owned();
		menu.refresh(&state, channel, &draft, Some(draft.chars().count()), &[]);
		assert!(matches!(menu.candidates[0], Candidate::GameSearch));
		let cursor = insert(&mut draft, menu.pick(0).unwrap(), false).unwrap();
		assert_eq!(draft, "Play @game ");
		menu.refresh(&state, channel, &draft, Some(cursor), &[]);
		assert!(menu.needs_games());
		assert_eq!(menu.member_query(), "");
		draft = "Play @game dea tonight".into();
		menu.refresh(&state, channel, &draft, Some("Play @game dea".len()), &[]);
		assert_eq!(menu.candidates[0].id(), Id(42));
		insert(&mut draft, menu.pick(0).unwrap(), false).unwrap();
		assert_eq!(draft, "Play <@$42>  tonight");
		assert!(model::mentioned_role_ids(&draft).is_empty());
	}

	#[test]
	fn game_queries_allow_spaces_and_unicode_but_stay_bounded() {
		assert!(matches!(
			query("@game Dead Space", 16),
			Some((_, "Dead Space", Kind::Game))
		));
		assert!(matches!(
			query("hi @game 界", 10),
			Some((_, "界", Kind::Game))
		));
		for draft in ["mail@game dea", "@game bad\nname", "@game <@42>"] {
			assert!(query(draft, draft.chars().count()).is_none());
		}
		let long = format!("@game {}", "x".repeat(65));
		assert!(query(&long, long.len()).is_none());
	}

	#[test]
	fn game_keyboard_selection_and_catalog_arrival_refresh_the_picker() {
		let mut state = test_support::demo_state();
		let mut menu = Menu::default();
		menu.refresh(&state, Id(20), "@game dea", Some(9), &[]);
		assert!(menu.candidates.is_empty());
		state.game_mentions.finish(Ok(vec![model::GameMention {
			id: Id(42),
			name: "Deadlock".into(),
			icon: None,
		}]));
		menu.refresh(&state, Id(20), "@game dea", Some(9), &[]);
		let ctx = egui::Context::default();
		let output = ctx.run_ui(
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
				let pick = menu.keys(&ctx).unwrap();
				assert_eq!(pick.candidate.token(), "<@$42> ");
				assert!(!ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)));
			},
		);
		output.drop_without_applying_deltas();
	}

	#[test]
	fn pill_fingerprints_follow_navigation_metadata_without_message_churn() {
		let mut state = test_support::demo_state();
		let messages = [
			("<#28>", Id(20)),
			("https://discord.com/channels/10/27/501", Id(20)),
			("https://discord.com/channels/10/27/501", Id(999)),
			("Plain text", Id(20)),
		]
		.map(|(content, channel)| {
			let mut message = test_support::message(600, channel);
			message.content = content.into();
			message
		});
		let fingerprints = |state: &State| {
			messages
				.each_ref()
				.map(|message| presentation_fingerprint(state, message))
		};
		let before = fingerprints(&state);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Message(test_support::message(601, Id(20))),
		});
		assert_eq!(fingerprints(&state), before);
		for (id, name, kind, parent) in [
			(28, "Renamed thread", 11, 20),
			(28, "Renamed thread", 12, 20),
			(28, "Renamed thread", 11, 26),
			(26, "Longer forum parent name", 15, 24),
		] {
			let before = fingerprints(&state);
			let mut updated = state.channel(Id(id)).unwrap().clone();
			updated.name = name.into();
			updated.kind = kind;
			updated.parent_id = Some(Id(parent));
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::ChannelCreated(updated),
			});
			let after = fingerprints(&state);
			assert!(before[..3].iter().zip(&after[..3]).all(|(a, b)| a != b));
			assert_eq!(before[3], after[3]);
		}
		let before = fingerprints(&state);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::GuildChanged(model::GuildPatch {
				id: Id(10),
				name: model::Patch::Value("Longer foreign server name".into()),
				icon: model::Patch::Value("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
				default_message_notifications: model::Patch::Absent,
			}),
		});
		assert_ne!(fingerprints(&state)[2], before[2]);
		let before = fingerprints(&state);
		let mut source = state.channel(Id(20)).unwrap().clone();
		source.id = Id(999);
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::ChannelCreated(source),
		});
		assert_ne!(fingerprints(&state)[2], before[2]);
	}

	#[test]
	fn pill_fingerprints_cover_retained_reference_names_and_bound_source_scanning() {
		let mut state = test_support::demo_state();
		let mut message = test_support::message(600, Id(20));
		message.content = "<#77> <#78>".into();
		for id in [77, 78] {
			let before = presentation_fingerprint(&state, &message);
			let Some(client_core::Command::ChannelAction { request, .. }) =
				state.request_channel_reference(Id(10), Id(id))
			else {
				panic!("synthetic reference request");
			};
			let mut thread = state.channel(Id(28)).unwrap().clone();
			thread.id = Id(id);
			thread.name = format!("Resolved thread {id}");
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::ChannelAction(
					client_core::channel_actions::Event::Finished {
						guild: Id(10),
						channel: Id(id),
						request,
						result: Ok(client_core::channel_actions::Outcome::Channel {
							channel: Box::new(thread),
							permissions: None,
						}),
					},
				),
			});
			assert_ne!(presentation_fingerprint(&state, &message), before);
		}
		assert!(state.channel(Id(77)).is_none());
		assert_eq!(
			state.channel_reference_name(Id(77)),
			Some("Resolved thread 77")
		);
		message.content = format!(
			"{}é<#28> https://discord.com/channels/10/27/501",
			"a".repeat(crate::markdown::MAX_INPUT - 1)
		);
		let before = presentation_fingerprint(&state, &message);
		state.invalidate_navigation();
		assert_eq!(presentation_fingerprint(&state, &message), before);
	}

	#[test]
	fn loaded_members_remain_suggested_and_admitted_beyond_the_candidate_cap() {
		let mut state = test_support::demo_state();
		// Synthetic authenticated reducer state; no transport consumes these commands.
		state.demo = false;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild;
		let make_member = |id| model::Member {
			user: user(id, &format!("Member{id}")),
			roles: vec![],
			nick: None,
			status: None,
			custom_status: None,
			activities: vec![],
			clients: Default::default(),
		};
		let target = Id(20199);
		state.members = Some(model::MemberList {
			missing_identity: vec![],
			guild,
			channel,
			request: 1,
			start: 0,
			total: 200,
			lazy: false,
			freshness: model::Freshness::Fresh,
			groups: vec![],
			ranges: vec![],
			slots: (20000..20200)
				.map(|id| Some(model::MemberSlot::Person(make_member(id))))
				.collect(),
		});
		for id in 30000..30256 {
			let mut message = test_support::message(id, channel);
			message.author = user(id, "Synthetic recent speaker");
			state.timeline.insert(message, false, false).unwrap();
		}
		let users = known_users(&state, channel);
		assert!(
			users.iter().any(|user| user.id == target),
			"recent authors cannot displace loaded members"
		);
		let mut menu = Menu::default();
		menu.refresh(&state, channel, "@Member20199", Some(12), &users);
		assert!(
			menu.candidates
				.iter()
				.any(|candidate| candidate.id() == target)
		);
		// Search results and the member window together can exceed the candidate cap.
		state.member_search[0].request = Some(client_core::member_search::Request {
			guild: guild.unwrap(),
			channel,
			query: "Member".into(),
			users: vec![],
			nonce: 1,
			slot: 0,
		});
		state.member_search[0].rows = (40000..40100).map(make_member).collect();
		assert_eq!(known_users(&state, channel).len(), 256);
		assert!(
			!known_users(&state, channel)
				.iter()
				.any(|user| user.id == target)
		);
		let mut view = crate::MessagingUi::default();
		let mut commands = vec![];
		view.apply_extension_app_action(
			&mut state,
			extensions::AppAction::RequestProfile {
				user_id: target.to_string(),
				guild_id: None,
			},
			&mut commands,
		)
		.unwrap();
		assert!(!commands.is_empty());
		commands.clear();
		view.apply_extension_account_action(
			&mut state,
			extensions::AppAction::SetUserBlocked {
				user_id: target.to_string(),
				blocked: true,
			},
			&mut commands,
		)
		.unwrap();
		assert!(
			matches!(&commands[..], [client_core::Command::UserAction { action: client_core::user_actions::Action::Block { user, blocked: true }, .. }] if *user == target)
		);
	}

	#[test]
	fn mention_matches_prefer_recent_conversation_users_after_match_quality() {
		let mut state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let old = user(40001, "AlexOlder");
		let recent = user(40002, "BAlexRecent");
		for (id, author) in [(40001, old.clone()), (40002, recent.clone())] {
			let mut message = test_support::message(id, channel);
			message.author = author;
			state.timeline.insert(message, false, false).unwrap();
		}
		let mut menu = Menu::default();
		menu.refresh(
			&state,
			channel,
			"@lex",
			Some(4),
			&[old.clone(), recent.clone()],
		);
		assert_eq!(menu.candidates[0].id(), recent.id);
		menu.refresh(&state, channel, "@Al", Some(3), &[recent, old.clone()]);
		assert_eq!(menu.candidates[0].id(), old.id);
	}

	#[test]
	fn cross_server_emoji_search_names_sources_bounds_and_account_reset() {
		let mut state = State {
			channels: vec![channel(1, None, 1, "DM"), channel(2, None, 3, "Group DM")],
			guilds: [20, 10]
				.into_iter()
				.map(|id| model::Guild {
					default_message_notifications: None,
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
		insert(&mut draft, menu.pick(0).unwrap(), true).unwrap();
		assert_eq!(
			draft,
			r"[same\_wave](https://cdn.discordapp.com/emojis/10001.gif?size=64) "
		);
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
				tags: None,
				message_count: None,
				icon: None,
			});
			if let Some(guild) = guild {
				use model::permissions as p;
				state.channels.push(channel(42, Some(guild), 0, "Zoe"));
				state.guilds.push(model::Guild {
					default_message_notifications: None,
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
								icon: None,
								unicode_emoji: None,
								id: guild,
								name: String::new(),
								color: 0,
								secondary_color: None,
								tertiary_color: None,
								position: 0,
								hoist: false,
								bits: p::VIEW_CHANNEL
									| p::READ_MESSAGE_HISTORY | p::SEND_MESSAGES
									| p::MENTION_EVERYONE,
							}]),
							member: Some(p::Member {
								roles: vec![],
								timeout_until: None,
							}),
						}],
						channels: [Id(1), Id(42)]
							.into_iter()
							.map(|id| p::Channel {
								id,
								guild,
								overwrites: Some(vec![]),
							})
							.collect(),
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
					egui::text::CCursor::end_of_str(draft),
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
	#[test]
	fn quiet_mention_replacement_uses_effective_budget_and_edits_remain_literal() {
		for editing in [false, true] {
			let mut draft = format!("@silent {}@s", "x".repeat(client_core::MAX_CONTENT - 5));
			let original = draft.clone();
			let start = draft.len() - 2;
			let pick = Pick {
				range: start..draft.len(),
				candidate: Candidate::User {
					user: user(7, "Sam"),
					name: "Sam".into(),
				},
			};
			assert_eq!(insert(&mut draft, pick, editing).is_some(), !editing);
			if editing {
				assert_eq!(draft, original);
			} else {
				assert!(draft.ends_with("<@7> "));
				assert!(model::message_options::valid(
					&draft,
					client_core::MAX_CONTENT,
					false
				));
			}
		}
	}

	fn user(id: u64, name: &str) -> User {
		User {
			member_avatar: None,
			username: None,
			id: Id(id),
			name: name.into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
			cosmetics: None,
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
		assert_eq!(insert(&mut draft, chosen.unwrap(), true), Some(9));
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
			tags: None,
			message_count: None,
			icon: None,
		}
	}
	fn channel_mention_state() -> State {
		let mut state = test_support::demo_state();
		state
			.channels
			.push(channel(1000, Some(Id(10)), 0, "Reference target"));
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		state
	}

	fn deny_channel_mention_permission(state: &mut State, bits: u128) {
		let mut permissions = test_support::permission_snapshot(state);
		permissions
			.channels
			.iter_mut()
			.find(|channel| channel.id == Id(1000))
			.unwrap()
			.overwrites = Some(vec![model::permissions::Overwrite {
			id: Id(10),
			kind: 0,
			allow: 0,
			deny: bits,
		}]);
		state.permissions.replace(permissions).unwrap();
	}

	#[test]
	fn channel_mention_suggestions_require_view_and_history_access() {
		for denied in [
			model::permissions::VIEW_CHANNEL,
			model::permissions::READ_MESSAGE_HISTORY,
		] {
			let mut state = channel_mention_state();
			deny_channel_mention_permission(&mut state, denied);
			let mut menu = Menu::default();
			for draft in ["#Reference", "#1000"] {
				menu.refresh(&state, Id(20), draft, Some(draft.chars().count()), &[]);
				assert!(
					menu.candidates.is_empty(),
					"denied channel appeared for {draft}"
				);
			}
		}
	}

	#[test]
	fn channel_mention_suggestions_keep_personally_hidden_and_muted_readable_channels() {
		let mut state = channel_mention_state();
		state
			.apply_notification_preferences(client_core::notifications::Event::Settings {
				entries: vec![client_core::notifications::Setting {
					guild: Some(Id(10)),
					flags: Some(model::channel_visibility::OPT_IN_CHANNELS_ON),
					channel_flags: vec![(Id(1000), Some(0), Some(false))],
					overrides_known: true,
					muted: Some(false),
					channels: vec![(Id(1000), Some(true), Some(3))],
					..Default::default()
				}],
				replace: true,
			})
			.unwrap();
		assert!(!state.channel_personally_selected(Id(1000)));
		assert_eq!(state.guild_channel_effectively_muted(Id(1000)), Some(true));
		assert!(state.can_view(Id(1000)) && state.can_read_history(Id(1000)));
		let mut menu = Menu::default();
		menu.refresh(&state, Id(20), "#Reference", Some(10), &[]);
		assert_eq!(
			menu.candidates
				.iter()
				.map(Candidate::id)
				.collect::<Vec<_>>(),
			[Id(1000)]
		);
		let mut draft = "#Reference".into();
		assert!(insert_checked(&state, Id(20), &mut draft, menu.pick(0).unwrap(), true).is_some());
		assert_eq!(draft, "<#1000> ");
	}

	#[test]
	fn channel_mention_suggestions_refresh_after_permission_changes() {
		for denied in [
			model::permissions::VIEW_CHANNEL,
			model::permissions::READ_MESSAGE_HISTORY,
		] {
			let mut state = channel_mention_state();
			let mut menu = Menu::default();
			menu.refresh(&state, Id(20), "#Reference", Some(10), &[]);
			assert_eq!(menu.candidates[0].id(), Id(1000));
			deny_channel_mention_permission(&mut state, denied);
			menu.refresh(&state, Id(20), "#Reference", Some(10), &[]);
			assert!(menu.candidates.is_empty());
		}
	}

	#[test]
	fn channel_mention_insertion_rechecks_permission_changes() {
		for denied in [
			model::permissions::VIEW_CHANNEL,
			model::permissions::READ_MESSAGE_HISTORY,
		] {
			let mut state = channel_mention_state();
			let mut menu = Menu::default();
			menu.refresh(&state, Id(20), "#Reference", Some(10), &[]);
			let pick = menu.pick(0).unwrap();
			deny_channel_mention_permission(&mut state, denied);
			let mut draft = "#Reference".into();
			assert!(insert_checked(&state, Id(20), &mut draft, pick, true).is_none());
			assert_eq!(draft, "#Reference");
		}
	}

	#[test]
	fn channel_mention_insertion_rejects_removed_known_metadata() {
		let mut state = channel_mention_state();
		let mut menu = Menu::default();
		menu.refresh(&state, Id(20), "#Reference", Some(10), &[]);
		let pick = menu.pick(0).unwrap();
		state.channels.retain(|channel| channel.id != Id(1000));
		let mut draft = "#Reference".into();
		assert!(insert_checked(&state, Id(20), &mut draft, pick, true).is_none());
		assert_eq!(draft, "#Reference");
	}

	#[test]
	fn channel_mention_insertion_rechecks_target_scope_and_kind() {
		for (guild, kind) in [(Some(Id(11)), 0), (Some(Id(10)), 2)] {
			let mut state = channel_mention_state();
			let mut menu = Menu::default();
			menu.refresh(&state, Id(20), "#Reference", Some(10), &[]);
			let pick = menu.pick(0).unwrap();
			let target = state
				.channels
				.iter_mut()
				.find(|channel| channel.id == Id(1000))
				.unwrap();
			target.guild = guild;
			target.kind = kind;
			let mut draft = "#Reference".into();
			assert!(insert_checked(&state, Id(20), &mut draft, pick, true).is_none());
			assert_eq!(draft, "#Reference");
		}
	}

	#[test]
	fn channel_mention_insertion_rechecks_new_metadata_for_unknown_ids() {
		let mut state = test_support::demo_state();
		let mut menu = Menu::default();
		menu.refresh(&state, Id(20), "#1000", Some(5), &[]);
		assert_eq!(menu.candidates[0].id(), Id(1000));
		let mut unresolved = "#1000".into();
		assert!(
			insert_checked(&state, Id(20), &mut unresolved, menu.pick(0).unwrap(), true).is_some()
		);
		assert_eq!(unresolved, "<#1000> ");
		let pick = menu.pick(0).unwrap();
		state
			.channels
			.push(channel(1000, Some(Id(10)), 0, "Reference target"));
		deny_channel_mention_permission(&mut state, model::permissions::VIEW_CHANNEL);
		let mut draft = "#1000".into();
		assert!(insert_checked(&state, Id(20), &mut draft, pick, true).is_none());
		assert_eq!(draft, "#1000");
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
			..test_support::demo_state()
		};
		state.guilds[0].id = Id(9);
		let mut other_guild = state.guilds[0].clone();
		other_guild.id = Id(8);
		state.guilds.push(other_guild);
		state.invalidate_navigation();
		state
			.channels
			.iter_mut()
			.find(|channel| channel.id == Id(7))
			.unwrap()
			.parent_id = Some(Id(1));
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
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
		assert_eq!(insert(&mut draft, pick.unwrap(), true), Some(9));
		assert_eq!(draft, "čau <#6> ");
		menu.refresh(&state, Id(3), "#", Some(1), &[]);
		assert!(menu.candidates.is_empty());
		state
			.channels
			.extend((20..40).map(|id| channel(id, Some(Id(9)), 0, &"é".repeat(300))));
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		menu.refresh(&state, Id(1), "#é", Some(2), &[]);
		assert_eq!(menu.candidates.len(), 8);
		assert!(menu.candidates.iter().all(|c| c.token().len() <= 40));
		let mut full = format!("{} #", "x".repeat(client_core::MAX_CONTENT - 2));
		menu.refresh(&state, Id(1), &full, Some(client_core::MAX_CONTENT), &[]);
		assert!(insert(&mut full, menu.pick(0).unwrap(), true).is_none());
	}
	#[test]
	fn emoji_shortcodes_need_two_characters_and_include_usable_server_emoji() {
		assert!(query(":h", 2).is_none());
		assert!(query(":", 1).is_none());
		assert!(query(":)", 2).is_none());
		assert!(query("10:30", 5).is_none());
		assert!(query("<:wave:9001>", 12).is_none());
		assert_eq!(query("hi :he", 6), Some((3..6, "he", Kind::Emoji)));
		assert_eq!(query(":+1", 3), Some((0..3, "+1", Kind::Emoji)));
		assert_eq!(query(":-1", 3), Some((0..3, "-1", Kind::Emoji)));
		let guilds = vec![model::Guild {
			default_message_notifications: None,
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
		let mut state = State {
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
				.any(|c| matches!(c, Candidate::Unicode { code, .. } if *code == ":heart:"))
		);
		let mut draft = "hi :he".to_owned();
		let expected =
			r"hi [heart\_hands\_custom](https://cdn.discordapp.com/emojis/9001.gif?size=64) ";
		assert_eq!(
			insert(&mut draft, menu.pick(0).unwrap(), true),
			Some(expected.chars().count())
		);
		assert_eq!(draft, expected);
		let unicode = menu
			.candidates
			.iter()
			.position(|c| matches!(c, Candidate::Unicode { code, .. } if *code == ":heart:"))
			.unwrap();
		let mut draft = "hi :he".to_owned();
		insert(&mut draft, menu.pick(unicode).unwrap(), true).unwrap();
		assert_eq!(draft, "hi ❤️ ");
		// Opting out of Nitro-only suggestions drops the animated, other-server emoji only.
		menu.hide_nitro_emojis = true;
		menu.refresh(&state, Id(1), "x", None, &[]);
		menu.refresh(&state, Id(1), "hi :he", Some(6), &[]);
		assert!(
			menu.hide_nitro_emojis,
			"the preference survives a closed menu"
		);
		assert!(!menu.candidates.iter().any(|c| c.id() == Id(9001)));
		assert!(
			menu.candidates
				.iter()
				.any(|c| matches!(c, Candidate::Unicode { code, .. } if *code == ":heart:"))
		);
		state.premium_type = 2;
		menu.refresh(&state, Id(1), "hi :he", Some(6), &[]);
		assert!(menu.candidates.iter().any(|c| c.id() == Id(9001)));
		menu.hide_nitro_emojis = false;
		menu.refresh(&state, Id(1), ":+1", Some(3), &[]);
		assert!(menu.candidates.iter().any(|candidate| matches!(
			candidate,
			Candidate::Unicode {
				text: "👍",
				code: ":thumbsup:"
			}
		)));
	}
}

#[cfg(all(debug_assertions, feature = "demo"))]
pub(crate) fn debug_image_completion(state: &mut State) -> String {
	let channel = state.selected.unwrap();
	let premium = state.premium_type;
	let mut fallback = String::new();
	for entitlement in 0..=3 {
		state.premium_type = entitlement;
		for target in [channel, Id(22)] {
			for (query, id, name, animated) in [
				(":serein_pa", Id(9002), "serein_party", true),
				(":serein_wa", Id(9001), "serein_wave", false),
			] {
				let mut menu = Menu::default();
				menu.refresh(state, target, query, Some(query.chars().count()), &[]);
				let image = entitlement == 0 && (animated || target != channel);
				let expected = if image {
					format!(
						"{} ",
						model::ImageShare::Emoji { id, animated }
							.markdown(name)
							.unwrap()
					)
				} else {
					format!("<{}:{name}:{id}> ", if animated { "a" } else { "" })
				};
				for key in [egui::Key::Tab, egui::Key::Enter] {
					let ctx = egui::Context::default();
					let output = ctx.run_ui(
						egui::RawInput {
							events: vec![egui::Event::Key {
								key,
								physical_key: None,
								pressed: true,
								repeat: false,
								modifiers: Default::default(),
							}],
							..Default::default()
						},
						|_| {
							let mut draft = query.to_owned();
							let pick = menu.keys(&ctx).expect("emoji completion");
							assert_eq!(
								insert(&mut draft, pick, false),
								Some(expected.chars().count())
							);
							assert_eq!(draft, expected);
						},
					);
					output.drop_without_applying_deltas();
				}
				if entitlement == 0 && target == channel && animated {
					fallback = expected.trim_end().into();
				}
			}
		}
	}
	state.premium_type = premium;
	fallback
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
	insert(&mut draft, pick, true).unwrap();
	assert_eq!(draft, "<@987654321> ");
}

#[cfg(debug_assertions)]
pub(crate) fn debug_pointer_check(state: &mut State, channel: Id) {
	state.game_mentions.finish(Ok(vec![model::GameMention {
		id: Id(42),
		name: "Deadlock".into(),
		icon: None,
	}]));
	for draft in ["@You", ":he", "#", "@game", "@game dea"] {
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
								egui::text::CCursor::end_of_str(draft),
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
		let section_header = if view.mention_menu.kind == Some(Kind::User) {
			24.0
		} else {
			0.0
		};
		let point = egui::pos2(
			area.center().x,
			area.top() + 34.0 + section_header + ROW / 2.0 + 1.0,
		);
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
			reply_preview: None,
			poll: None,
			sticker_items: vec![],
			id: Id(2),
			channel,
			author,
			content: format!("<@{}>", user.id),
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
			insert(&mut draft, menu.pick(0).unwrap(), true).unwrap();
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
		icon: None,
		unicode_emoji: None,
		id: Id(1548470397144666162),
		name: "Role check".into(),
		bits: 0,
		color: 0xe67e22,
		secondary_color: None,
		tertiary_color: None,
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
			insert(&mut draft, menu.keys(&ctx).expect("role completion"), true).unwrap();
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
	insert(
		&mut thread_draft,
		menu.pick(0).expect("thread with spaces"),
		true,
	)
	.unwrap();
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
	state
		.permissions
		.update(client_core::permissions::Event::Channel {
			channel: Id(1549042875830898711),
			guild: Some(guild),
			overwrites: model::Patch::Value(vec![]),
		})
		.unwrap();
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
			let _ = crate::markdown::Formatted::parse(&draft).append_inline_preview(
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
				crate::design::MessageCardSurface::Opaque,
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
					usize::from(matches!(value.trim(), "Thread with spaces" | "Forum check"))
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
