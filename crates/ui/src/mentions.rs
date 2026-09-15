//! Suggestions use only bounded people, guild text channels and emoji already loaded in this session.
use crate::avatars::Avatars;
use client_core::State;
use model::{Id, User};
use std::ops::Range;

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
	User {
		user: User,
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
impl Candidate {
	#[cfg(test)]
	fn id(&self) -> Id {
		match self {
			Candidate::User { user } => user.id,
			Candidate::Channel { id, .. } | Candidate::Custom { id, .. } => *id,
			Candidate::Mass { .. } | Candidate::Unicode { .. } => Id(0),
		}
	}
	fn token(&self) -> String {
		match self {
			Candidate::User { user } => format!("<@{}> ", user.id),
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
pub fn known_users(state: &State, channel: Id) -> Vec<User> {
	let mut users = Vec::new();
	let mut add = |user: &User| {
		if users.len() < 256 && !users.iter().any(|u: &User| u.id == user.id) {
			users.push(user.clone());
		}
	};
	if let Some(owner) = &state.user {
		add(owner);
	}
	if let Some(channel) = state.channels.iter().find(|c| c.id == channel) {
		for user in &channel.recipients {
			add(user);
		}
	}
	if let Some(members) = state.members.as_ref().filter(|m| m.channel == channel) {
		for member in members.rows.iter().flatten() {
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
				c.is_whitespace()
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
						rank(&query, &user.name, user.id)
							.map(|r| ((r, 0, 0), Candidate::User { user: user.clone() }))
					})
					.collect::<Vec<_>>();
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
						&& c.supports_text()
						&& !matches!(c.kind, 1 | 3)
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
		_ => None,
	};
	let primary = match candidate {
		Candidate::User { user } => {
			let mut child = ui.new_child(egui::UiBuilder::new().max_rect(icon));
			avatars.show(&mut child, user, 24.0, demo);
			user.name.clone()
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
			egui::WidgetType::Button,
			true,
			selected,
			match candidate {
				Candidate::User { user } => user.name.clone(),
				Candidate::Mass { name } => format!("@{name}"),
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
			[Id(6), Id(7)]
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
