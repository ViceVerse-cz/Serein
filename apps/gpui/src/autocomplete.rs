//! `@person`, `#channel` and `:emoji` suggestions for the composer, from data already in memory.
use crate::sidebar::avatar;
use crate::theme::{Icon, color, icon, palette};
use crate::{Serein, channel_label, input, text_channel};
use client_core::State;
use gpui::{prelude::*, *};

const MAX_QUERY: usize = 32;
const MAX_ITEMS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
	Person,
	Channel,
	Emoji,
}

pub struct Item {
	pub label: String,
	pub detail: Option<String>,
	/// Wire text that replaces the typed token.
	pub insert: String,
	pub user: Option<model::User>,
	/// Unicode emoji shown in place of an avatar or icon.
	pub glyph: Option<&'static str>,
}

pub struct Picker {
	pub kind: Kind,
	/// Byte offset of the `@`/`#`/`:` trigger in the composer text.
	pub start: usize,
	pub end: usize,
	pub items: Vec<Item>,
	pub selected: usize,
}

/// The trigger token ending at `cursor`: `@name`, `#channel` or `:emoji` (two or more name
/// characters) after whitespace or at the start.
pub fn token(text: &str, cursor: usize) -> Option<(Kind, usize, &str)> {
	let before = text.get(..cursor)?;
	let start = before
		.char_indices()
		.rev()
		.find(|(_, c)| c.is_whitespace())
		.map_or(0, |(index, c)| index + c.len_utf8());
	let word = &before[start..];
	let kind = match word.chars().next()? {
		'@' => Kind::Person,
		'#' => Kind::Channel,
		':' => Kind::Emoji,
		_ => return None,
	};
	let query = &word[1..];
	if kind == Kind::Emoji
		&& (query.chars().count() < 2
			|| !query
				.chars()
				.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '+')))
	{
		return None;
	}
	(query.chars().count() <= MAX_QUERY).then_some((kind, start, query))
}

fn rank(name: &str, query: &str) -> Option<u8> {
	let name = name.to_lowercase();
	if name.starts_with(query) {
		Some(0)
	} else if name.contains(query) {
		Some(1)
	} else {
		None
	}
}

/// People from the member list, the loaded timeline and DM recipients; channels from the server.
pub fn items(state: &State, kind: Kind, query: &str) -> Vec<Item> {
	items_up_to(state, kind, query, MAX_ITEMS)
}

/// [`items`] with another bound on people and channels (slash-command choosers list more).
pub fn items_up_to(state: &State, kind: Kind, query: &str, limit: usize) -> Vec<Item> {
	let query = query.to_lowercase();
	let mut ranked = Vec::new();
	match kind {
		Kind::Person => {
			let mut seen = std::collections::BTreeSet::new();
			let members = state
				.members
				.iter()
				.flat_map(|list| list.slots.iter().flatten())
				.filter_map(|slot| match slot {
					model::MemberSlot::Person(member) => {
						Some((member.user.clone(), member.nick.clone()))
					}
					_ => None,
				});
			let authors = state
				.timeline
				.iter()
				.map(|message| (message.author.clone(), message.author_nick.clone()));
			let recipients = state
				.selected
				.and_then(|id| state.channel(id))
				.into_iter()
				.flat_map(|channel| channel.recipients.iter().cloned().map(|user| (user, None)));
			for (user, nick) in members.chain(authors).chain(recipients) {
				if user.webhook || !seen.insert(user.id) {
					continue;
				}
				let display = nick.unwrap_or_else(|| state.user_display_name(&user).to_owned());
				let Some(score) = rank(&display, &query).or_else(|| rank(&user.name, &query))
				else {
					continue;
				};
				ranked.push((
					score,
					Item {
						detail: (display != user.name).then(|| user.name.clone()),
						label: display,
						insert: format!("<@{}>", user.id),
						user: Some(user),
						glyph: None,
					},
				));
			}
		}
		Kind::Channel => {
			let guild = state
				.selected
				.and_then(|id| state.channel(id))
				.and_then(|c| c.guild);
			for channel in state.channels.iter().filter(|c| {
				guild.is_some() && c.guild == guild && text_channel(c) && state.can_view(c.id)
			}) {
				let name = channel_label(channel);
				if let Some(score) = rank(&name, &query) {
					ranked.push((
						score,
						Item {
							label: name,
							detail: None,
							insert: format!("<#{}>", channel.id),
							user: None,
							glyph: None,
						},
					));
				}
			}
		}
		Kind::Emoji => return emoji_items(state, &query),
	}
	ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
	ranked
		.into_iter()
		.take(limit)
		.map(|(_, item)| item)
		.collect()
}

/// Prefix hits first, then word starts, then substrings; `code` is already lowercase.
pub(crate) fn emoji_rank(code: &str, query: &str) -> Option<u8> {
	if code.starts_with(query) {
		Some(0)
	} else if code
		.match_indices(query)
		.any(|(index, _)| code[..index].ends_with('_'))
	{
		Some(1)
	} else if code.contains(query) {
		Some(2)
	} else {
		None
	}
}

/// The current server's usable custom emoji first, then bundled Unicode emoji; skin-tone
/// variants rank after base emoji. Only the best `MAX_ITEMS` are kept while scanning.
fn emoji_items(state: &State, query: &str) -> Vec<Item> {
	type Key = (u8, bool, u8, usize);
	fn push(best: &mut Vec<(Key, Item)>, key: Key, item: impl FnOnce() -> Item) {
		let index = best.partition_point(|(other, _)| *other <= key);
		if index < MAX_ITEMS {
			best.insert(index, (key, item()));
			best.truncate(MAX_ITEMS);
		}
	}
	let mut best = Vec::with_capacity(MAX_ITEMS + 1);
	let channel = state.selected.and_then(|id| state.channel(id));
	if let Some(channel) = channel
		&& let Some(guild) = channel.guild.and_then(|id| state.guild(id))
	{
		for (index, emoji) in guild.emojis.iter().flatten().enumerate() {
			if state
				.custom_emoji_unavailable_reason(channel.id, guild.id, emoji)
				.is_none() && let Some(score) = rank(&emoji.name, query)
			{
				push(&mut best, (score, false, 0, index), || Item {
					label: format!(":{}:", emoji.name),
					detail: Some(guild.name.clone()),
					insert: custom_markup(emoji),
					user: None,
					glyph: None,
				});
			}
		}
	}
	for (index, (text, code)) in ui::emoji::unicode().enumerate() {
		let name = &code[1..code.len() - 1];
		if let Some(score) = emoji_rank(name, query) {
			push(
				&mut best,
				(score, name.contains("skin_tone"), 1, index),
				|| Item {
					label: code.to_owned(),
					detail: None,
					insert: text.to_owned(),
					user: None,
					glyph: Some(text),
				},
			);
		}
	}
	best.into_iter().map(|(_, item)| item).collect()
}

/// Wire form of a server emoji: `<:name:id>`, or `<a:name:id>` when animated.
pub fn custom_markup(emoji: &model::CustomEmoji) -> String {
	let prefix = if emoji.animated { "a" } else { "" };
	format!("<{prefix}:{}:{}>", emoji.name, emoji.id)
}

impl Serein {
	/// Recomputes suggestions after each edit; closes when the caret leaves a trigger token.
	pub(crate) fn update_picker(&mut self, cx: &mut Context<Self>) {
		if self.update_slash(cx) {
			self.picker = None;
			let picking = self.slash.picking();
			self.composer
				.update(cx, |input, _| input.set_picking(picking));
			cx.notify();
			return;
		}
		let input = self.composer.read(cx);
		let (text, cursor) = (input.value().to_owned(), input.cursor());
		self.picker = token(&text, cursor).and_then(|(kind, start, query)| {
			let items = items(&self.state, kind, query);
			(!items.is_empty()).then_some(Picker {
				kind,
				start,
				end: cursor,
				items,
				selected: 0,
			})
		});
		let picking = self.picker.is_some();
		self.composer
			.update(cx, |input, _| input.set_picking(picking));
		cx.notify();
	}

	pub(crate) fn pick(&mut self, key: input::Pick, window: &mut Window, cx: &mut Context<Self>) {
		if self.slash_pick(key, window, cx) {
			return;
		}
		let Some(picker) = &mut self.picker else {
			return;
		};
		let count = picker.items.len();
		match key {
			input::Pick::Up => picker.selected = (picker.selected + count - 1) % count,
			input::Pick::Down => picker.selected = (picker.selected + 1) % count,
			input::Pick::Accept => {
				let index = picker.selected;
				return self.accept_pick(index, cx);
			}
			input::Pick::Close => {
				self.picker = None;
				self.composer
					.update(cx, |input, _| input.set_picking(false));
			}
		}
		cx.notify();
	}

	fn accept_pick(&mut self, index: usize, cx: &mut Context<Self>) {
		let Some(picker) = self.picker.take() else {
			return;
		};
		let Some(item) = picker.items.get(index) else {
			return;
		};
		self.composer.update(cx, |input, cx| {
			let value = input.value();
			let (Some(head), Some(tail)) = (value.get(..picker.start), value.get(picker.end..))
			else {
				return;
			};
			let next = format!("{head}{} {tail}", item.insert);
			input.set_picking(false);
			input.set_value(next, cx);
		});
		cx.notify();
	}

	pub(crate) fn render_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
		if let Some(slash) = self.render_slash_picker(cx) {
			return Some(slash);
		}
		let p = palette();
		let picker = self.picker.as_ref()?;
		Some(
			div()
				.absolute()
				.left(px(16.))
				.right(px(16.))
				.bottom_full()
				.mb_1()
				.p_2()
				.rounded(px(8.))
				.bg(color(p.base))
				.border_1()
				.border_color(color(p.border))
				.shadow_lg()
				.flex()
				.flex_col()
				.child(
					div()
						.px_2()
						.pb_1()
						.text_size(px(12.))
						.font_weight(FontWeight::SEMIBOLD)
						.text_color(color(p.muted))
						.child(match picker.kind {
							Kind::Person => "MEMBERS",
							Kind::Channel => "TEXT CHANNELS",
							Kind::Emoji => "EMOJI MATCHING",
						}),
				)
				.children(picker.items.iter().enumerate().map(|(index, item)| {
					let selected = index == picker.selected;
					div()
						.id(("suggestion", index))
						.h(px(36.))
						.px_2()
						.rounded(px(6.))
						.flex()
						.items_center()
						.gap_2()
						.cursor_pointer()
						.when(selected, |d| d.bg(color(p.selected)))
						.hover(|d| d.bg(color(p.hover)))
						.on_click(cx.listener(move |this, _, _, cx| this.accept_pick(index, cx)))
						.child(match (&item.user, item.glyph) {
							(Some(user), _) => {
								avatar(&item.label, 24., Some(user)).into_any_element()
							}
							(None, Some(glyph)) => div()
								.w(px(24.))
								.flex()
								.justify_center()
								.text_size(px(20.))
								.child(glyph)
								.into_any_element(),
							(None, None) => icon(
								if picker.kind == Kind::Emoji {
									Icon::Smiley
								} else {
									Icon::Hash
								},
								px(20.),
								color(p.muted),
							)
							.into_any_element(),
						})
						.child(
							div()
								.text_size(px(15.))
								.font_weight(FontWeight::MEDIUM)
								.text_color(color(p.text_strong))
								.child(item.label.clone()),
						)
						.children(item.detail.clone().map(|detail| {
							div()
								.text_size(px(13.))
								.text_color(color(p.muted))
								.child(detail)
						}))
				}))
				.into_any_element(),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::{Kind, items, token};

	#[test]
	fn tokens_start_after_whitespace_and_stay_short() {
		assert_eq!(token("hi @rob", 7), Some((Kind::Person, 3, "rob")));
		assert_eq!(token("#gen", 4), Some((Kind::Channel, 0, "gen")));
		assert_eq!(token("mail@rob", 8), None);
		assert_eq!(token("hi @rob there", 13), None);
		assert_eq!(token(&format!("@{}", "a".repeat(33)), 34), None);
	}

	#[test]
	fn emoji_tokens_need_two_name_characters() {
		assert_eq!(token("nice :th", 8), Some((Kind::Emoji, 5, "th")));
		assert_eq!(token(":+1", 3), Some((Kind::Emoji, 0, "+1")));
		assert_eq!(token("nice :t", 7), None);
		assert_eq!(token("at 10:30", 8), None);
		assert_eq!(token(":smile:", 7), None);
		assert_eq!(token(":a b", 4), None);
	}

	#[test]
	fn emoji_suggestions_rank_prefixes_and_include_server_emoji() {
		let state = test_support::chat_demo_state();
		let thumbs = items(&state, Kind::Emoji, "thumbs");
		assert_eq!(thumbs.len(), super::MAX_ITEMS);
		assert_eq!(thumbs[0].insert, "👍");
		assert_eq!(thumbs[0].label, ":thumbs_up:");
		// Base emoji come before skin-tone variants.
		assert_eq!(thumbs[1].insert, "👎");
		assert!(thumbs.iter().all(|item| item.label.starts_with(":thumbs")));
		let joy = items(&state, Kind::Emoji, "joy");
		assert!(joy.iter().any(|item| item.insert == "😂"));
		let server = items(&state, Kind::Emoji, "serein");
		let inserts = server
			.iter()
			.map(|item| item.insert.as_str())
			.collect::<Vec<_>>();
		assert_eq!(
			inserts[..2],
			["<:serein_wave:9001>", "<a:serein_party:9002>"]
		);
		assert!(items(&state, Kind::Emoji, "zzzzqq").is_empty());
	}

	#[test]
	fn suggestions_come_from_loaded_people_and_visible_channels() {
		let mut state = test_support::chat_demo_state();
		let _ = state.request_members();
		let people = items(&state, Kind::Person, "rob");
		assert!(people.iter().any(|item| item.insert == "<@2>"));
		assert!(people.len() <= super::MAX_ITEMS);
		let channels = items(&state, Kind::Channel, "long");
		assert_eq!(
			channels.first().map(|item| item.insert.as_str()),
			Some("<#21>")
		);
	}
}
