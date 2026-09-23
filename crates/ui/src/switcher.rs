use crate::{avatars::Avatars, design, icons};
use client_core::State;
use model::{Channel, Id};
use std::collections::HashSet;

const QUERY_CHARS: usize = 128;
const QUERY_BYTES: usize = QUERY_CHARS * 4;
const RESULTS: usize = 20;

#[derive(Default)]
pub(super) struct Switcher {
	open: bool,
	query: String,
	selected: usize,
	focus: bool,
	previous_focus: Option<egui::Id>,
	composing: bool,
	/// Results for `searched`, rebuilt when the query changes and at most once a second otherwise.
	choices: Vec<Candidate>,
	searched: Option<(String, f64)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
	Text,
	Voice,
	Direct,
	Group,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(super) enum Target {
	Channel(Id),
	Friend(Id),
}

struct Candidate {
	target: Target,
	kind: Kind,
	name: String,
	scope: String,
	current: bool,
	user: Option<model::User>,
}

impl Candidate {
	/// Full accessible label; also what tests and screen readers see.
	fn label(&self) -> String {
		let kind = match self.kind {
			Kind::Text => "#",
			Kind::Voice => "Voice · roster",
			Kind::Direct | Kind::Group => "",
		};
		format!("{kind} {} · {}", self.name, self.scope)
	}
}

pub(super) fn bounded(value: &str) -> String {
	value.chars().take(QUERY_CHARS).collect()
}

/// Writes `bounded(value).to_lowercase()` into `buffer`, allocating only for non-ASCII text.
fn lowercase_bounded_into(buffer: &mut String, value: &str) {
	buffer.clear();
	if value.is_ascii() {
		buffer.push_str(&value[..value.len().min(QUERY_CHARS)]);
		buffer.make_ascii_lowercase();
	} else {
		let end = value
			.char_indices()
			.nth(QUERY_CHARS)
			.map_or(value.len(), |(index, _)| index);
		buffer.push_str(&value[..end].to_lowercase());
	}
}

fn labels_match<'a>(
	labels: impl Iterator<Item = &'a str>,
	words: &[&str],
	label: &mut String,
	matched: &mut [bool],
) -> bool {
	if words.is_empty() {
		return true;
	}
	matched.fill(false);
	for value in labels {
		lowercase_bounded_into(label, value);
		for (word, hit) in words.iter().zip(matched.iter_mut()) {
			*hit = *hit || label.contains(word);
		}
		if matched.iter().all(|hit| *hit) {
			return true;
		}
	}
	false
}

/// Whether every lowercase query word appears in the conversation, server or recipient names.
pub(super) fn channel_matches(
	state: &State,
	channel: &Channel,
	words: &[&str],
	label: &mut String,
	matched: &mut [bool],
) -> bool {
	let guild = channel
		.guild
		.and_then(|id| state.guild(id))
		.map(|guild| guild.name.as_str());
	let recipients = channel.guild.is_none().then(|| {
		channel.recipients.iter().take(64).flat_map(|user| {
			[
				Some(user.name.as_str()),
				state.friend(user.id).map(|friend| friend.name.as_str()),
				state.friend_nickname(user.id),
				state.friend_username(user.id),
			]
			.into_iter()
			.flatten()
		})
	});
	let labels = std::iter::once(channel.name.as_str())
		.chain(guild)
		.chain(recipients.into_iter().flatten());
	labels_match(labels, words, label, matched)
}

fn candidates(state: &State, query: &str) -> Vec<Candidate> {
	let query = bounded(query).to_lowercase();
	let words: Vec<_> = query.split_whitespace().collect();
	// One reused buffer instead of a lowercase copy of every label per keystroke.
	let mut label = String::new();
	let mut matched = vec![false; words.len()];
	let mut matches =
		|channel: &Channel| channel_matches(state, channel, &words, &mut label, &mut matched);
	let mut found: Vec<_> = state
		.channels
		.iter()
		.filter(|c| (c.supports_text() || c.kind == 2) && state.can_view(c.id) && matches(c))
		.collect();
	// Current conversation first, then the most recently active ones.
	recent_first(&mut found, RESULTS, |c| {
		(Some(c.id) == state.selected, activity(c))
	});
	let mut choices: Vec<_> = found
		.into_iter()
		.map(|channel| {
			let name = state.conversation_name(channel);
			let name = if name.is_empty() && channel.guild.is_none() {
				channel
					.recipients
					.first()
					.map_or("Direct message", |user| state.user_display_name(user))
			} else {
				name
			};
			let scope = channel.guild.and_then(|id| state.guild(id)).map_or(
				if channel.guild.is_some() {
					"Server"
				} else if channel.kind == 3 {
					"Group direct message"
				} else {
					"Direct message"
				},
				|g| g.name.as_str(),
			);
			let kind = if channel.kind == 2 {
				Kind::Voice
			} else if channel.guild.is_some() {
				Kind::Text
			} else if channel.kind == 3 {
				Kind::Group
			} else {
				Kind::Direct
			};
			Candidate {
				target: Target::Channel(channel.id),
				kind,
				name: bounded(name),
				scope: bounded(scope),
				current: Some(channel.id) == state.selected,
				user: if kind == Kind::Direct {
					channel.recipients.first().cloned()
				} else {
					None
				},
			}
		})
		.collect();
	if choices.len() == RESULTS {
		return choices;
	}
	// Only one-to-one DMs suppress friend rows; at most the retained 4,000 friend IDs.
	let direct_friends: HashSet<_> = state
		.channels
		.iter()
		.filter(|channel| channel.guild.is_none() && channel.kind == 1)
		.flat_map(|channel| channel.recipients.iter().take(64))
		.filter(|user| state.friend(user.id).is_some())
		.map(|user| user.id)
		.collect();
	for user in state.friends() {
		if direct_friends.contains(&user.id) {
			continue;
		}
		let labels = [
			Some(user.name.as_str()),
			state.friend_nickname(user.id),
			state.friend_username(user.id),
		]
		.into_iter()
		.flatten();
		if labels_match(labels, &words, &mut label, &mut matched) {
			choices.push(Candidate {
				target: Target::Friend(user.id),
				kind: Kind::Direct,
				name: bounded(state.user_display_name(user)),
				scope: bounded(state.friend_username(user.id).unwrap_or("Friend")),
				current: false,
				user: Some(user.clone()),
			});
			if choices.len() == RESULTS {
				break;
			}
		}
	}
	choices
}

/// Snowflake of the latest known activity; a channel's own ID when it has no messages yet.
pub(super) fn activity(channel: &Channel) -> Id {
	channel.last_message.unwrap_or(channel.id).max(channel.id)
}

/// Keeps the `limit` highest-ranked items, highest first.
pub(super) fn recent_first<T, K: Ord>(items: &mut Vec<T>, limit: usize, rank: impl Fn(&T) -> K) {
	if items.len() > limit {
		items.select_nth_unstable_by(limit, |a, b| rank(b).cmp(&rank(a)));
		items.truncate(limit);
	}
	items.sort_by_key(|item| std::cmp::Reverse(rank(item)));
}

/// Small rounded chip that names a key in the footer legend.
fn key_hint(ui: &mut egui::Ui, keys: &str, colors: design::Palette) {
	egui::Frame::new()
		.fill(colors.raised)
		.stroke(egui::Stroke::new(1.0, colors.border))
		.corner_radius(4)
		.inner_margin(egui::Margin::symmetric(5, 1))
		.show(ui, |ui| {
			ui.label(
				egui::RichText::new(keys)
					.size(11.0)
					.color(colors.text)
					.family(design::medium_family(ui.ctx())),
			);
		});
}

/// One result: kind glyph or initials avatar, name, scope line and a "Current" tag.
fn result_row(
	ui: &mut egui::Ui,
	choice: &Candidate,
	selected: bool,
	enabled: bool,
	avatars: &mut Avatars,
	demo: bool,
) -> egui::Response {
	let colors = design::palette(ui);
	let height = 46.0;
	let width = ui.available_width();
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(width, height),
		if enabled {
			egui::Sense::click()
		} else {
			egui::Sense::hover()
		},
	);
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::Role::Button, enabled, selected, choice.label())
	});
	if !ui.is_rect_visible(rect) {
		return response;
	}
	{
		let painter = ui.painter();
		let hovered = enabled && (response.hovered() || response.has_focus());
		if selected {
			painter.rect_filled(rect, 8, colors.selected);
		} else if hovered {
			painter.rect_filled(rect, 8, colors.hover);
		}
		if response.has_focus() {
			painter.rect_stroke(
				rect.shrink(1.0),
				8,
				egui::Stroke::new(1.5, colors.accent),
				egui::StrokeKind::Inside,
			);
		}
	}
	let text = if enabled {
		colors.text_strong
	} else {
		colors.muted
	};
	let dim = if enabled {
		colors.muted
	} else {
		colors.muted.gamma_multiply(0.7)
	};
	let icon_size = 32.0;
	let icon_rect = egui::Rect::from_center_size(
		egui::pos2(rect.left() + 10.0 + icon_size / 2.0, rect.center().y),
		egui::Vec2::splat(icon_size),
	);
	match choice.kind {
		Kind::Direct => {
			if let Some(user) = &choice.user {
				avatars.paint_user(ui, user, icon_size, icon_rect, demo);
			} else {
				design::paint_avatar(ui, &choice.name, icon_size, icon_rect);
			}
		}
		Kind::Group => {
			let painter = ui.painter();
			painter.circle_filled(icon_rect.center(), icon_size / 2.0, colors.accent);
			icons::paint(
				painter,
				icons::Icon::People,
				icon_rect.shrink(8.0),
				colors.accent_text,
			);
		}
		Kind::Text | Kind::Voice => {
			let painter = ui.painter();
			painter.rect_filled(icon_rect, 8, colors.raised);
			let icon = if choice.kind == Kind::Voice {
				icons::Icon::Speaker
			} else {
				icons::Icon::Hash
			};
			icons::paint(
				painter,
				icon,
				icon_rect.shrink(7.0),
				if selected { text } else { dim },
			);
		}
	}
	let painter = ui.painter();
	let mut right = rect.right() - 10.0;
	if choice.current {
		let font = egui::FontId::new(10.0, design::semibold_family(ui.ctx()));
		let galley = painter.layout_no_wrap("CURRENT".to_owned(), font, colors.accent_text);
		let size = galley.size();
		let tag = egui::Rect::from_center_size(
			egui::pos2(right - size.x / 2.0 - 6.0, rect.center().y),
			size + egui::vec2(12.0, 6.0),
		);
		painter.rect_filled(tag, 4, colors.accent);
		painter.galley(tag.min + egui::vec2(6.0, 3.0), galley, colors.accent_text);
		right = tag.left() - 8.0;
	}
	let text_left = icon_rect.right() + 10.0;
	let text_width = (right - text_left).max(1.0);
	let name = painter.layout(
		choice.name.clone(),
		egui::FontId::new(15.0, design::semibold_family(ui.ctx())),
		text,
		f32::INFINITY,
	);
	let scope_text = match choice.kind {
		Kind::Voice => format!("{} · Voice", choice.scope),
		_ => choice.scope.clone(),
	};
	let scope = painter.layout(
		scope_text,
		egui::FontId::proportional(12.0),
		dim,
		f32::INFINITY,
	);
	let name_height = name.size().y;
	let total = name_height + scope.size().y + 1.0;
	let top = rect.center().y - total / 2.0;
	let clip = egui::Rect::from_min_size(
		egui::pos2(text_left, rect.top()),
		egui::vec2(text_width, height),
	);
	let clipped = painter.with_clip_rect(clip);
	clipped.galley(egui::pos2(text_left, top), name, text);
	clipped.galley(egui::pos2(text_left, top + name_height + 1.0), scope, dim);
	response
}

impl Switcher {
	pub(super) fn is_open(&self) -> bool {
		self.open
	}

	pub(super) fn open(&mut self, ctx: &egui::Context) {
		if self.open {
			return;
		}
		self.open = true;
		self.query.clear();
		self.selected = 0;
		self.focus = true;
		self.composing = false;
		self.searched = None;
		self.previous_focus = ctx.memory(|memory| memory.focused());
	}

	fn close(&mut self, ctx: &egui::Context, restore: bool) {
		self.open = false;
		self.query.clear();
		self.composing = false;
		self.choices = Vec::new();
		self.searched = None;
		if !restore {
			self.previous_focus = None;
		}
		ctx.request_repaint();
	}

	pub(super) fn show(
		&mut self,
		ctx: &egui::Context,
		state: &State,
		avatars: &mut Avatars,
	) -> Option<Target> {
		if !self.open {
			let modal = ctx.memory(|memory| memory.top_modal_layer());
			if modal.is_none() {
				if let Some(id) = self.previous_focus.take() {
					ctx.memory_mut(|memory| memory.request_focus(id));
				}
			} else if modal
				== Some(egui::LayerId::new(
					egui::Order::Foreground,
					egui::Id::unique("conversation-switcher"),
				)) {
				// Finish the modal's closing pass before restoring or changing focus.
				ctx.request_repaint();
			}
			return None;
		}
		let ime_frame = ctx.input(|input| {
			input
				.events
				.iter()
				.any(|event| matches!(event, egui::Event::Ime(_)))
		});
		ctx.input(|input| {
			for event in &input.events {
				match event {
					egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
						self.composing = !text.is_empty()
					}
					egui::Event::Ime(egui::ImeEvent::Commit(_)) => self.composing = false,
					_ => {}
				}
			}
		});
		let blocked = self.composing || ime_frame;
		let query_focused = ctx.memory(|memory| memory.focused())
			== Some(egui::Id::unique("conversation-switcher-query"));
		let (up, down, enter, escape) = ctx.input_mut(|input| {
			if blocked {
				input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
				return (false, false, false, false);
			}
			(
				query_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
				query_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
				query_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
				input.consume_key(egui::Modifiers::NONE, egui::Key::Escape),
			)
		});
		// egui schedules directional focus before widgets consume this frame's keys.
		// Cancel that first-frame movement too, before the field's focus filter is installed.
		if up || down {
			ctx.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
		}
		let mut target = None;
		let mut cancel = escape;
		let colors = design::palette_for(ctx);
		let narrow = ctx.content_rect().width() < 420.0;
		let margin: i8 = if narrow { 12 } else { 16 };
		let modal = egui::Modal::new(egui::Id::unique("conversation-switcher")).frame(
			egui::Frame::new()
				.fill(colors.chat)
				.stroke(egui::Stroke::new(1.0, colors.border))
				.corner_radius(14)
				.inner_margin(margin),
		);
		let response = modal.show(ctx, |ui| {
			ui.set_width(
				(ctx.content_rect().width() - f32::from(margin) * 2.0 - 32.0).clamp(200.0, 560.0),
			);
			ui.spacing_mut().item_spacing.y = 8.0;
			// Search field: raised pill with a leading glyph, like the header search box.
			let mut changed = false;
			egui::Frame::new()
				.fill(colors.raised)
				.stroke(egui::Stroke::new(1.0, colors.border))
				.corner_radius(8)
				.inner_margin(egui::Margin::symmetric(10, 0))
				.show(ui, |ui| {
					ui.set_height(44.0);
					ui.horizontal_centered(|ui| {
						ui.spacing_mut().item_spacing.x = 8.0;
						icons::inline(ui, icons::Icon::Search, 18.0, colors.muted);
						let input = ui.add(
							egui::TextEdit::singleline(&mut self.query)
								.id(egui::Id::unique("conversation-switcher-query"))
								.event_filter(egui::EventFilter {
									horizontal_arrows: true,
									vertical_arrows: true,
									escape: true,
									..Default::default()
								})
								.frame(egui::Frame::NONE)
								.font(egui::FontId::proportional(16.0))
								.hint_text("Where would you like to go?")
								.char_limit(QUERY_CHARS)
								.desired_width(ui.available_width().max(60.0)),
						);
						let input = input.accessible_name("Find conversation");
						if self.focus {
							input.request_focus();
							self.focus = false;
						}
						changed = input.changed();
					});
				});
			if self.query.chars().count() > QUERY_CHARS {
				self.query = bounded(&self.query);
			}
			if self.query.capacity() > QUERY_BYTES {
				self.query = std::mem::take(&mut self.query)
					.into_boxed_str()
					.into_string();
			}
			if changed {
				self.selected = 0;
			}
			if blocked {
				ui.label(
					egui::RichText::new("Finish composing text before opening or closing.")
						.size(12.0)
						.color(colors.warning),
				);
			}
			let now = ui.input(|input| input.time);
			if self
				.searched
				.as_ref()
				.is_none_or(|(query, at)| *query != self.query || !(0.0..1.0).contains(&(now - at)))
			{
				self.choices = candidates(state, &self.query);
				self.searched = Some((self.query.clone(), now));
			}
			let choices = &self.choices;
			self.selected = self.selected.min(choices.len().saturating_sub(1));
			if !choices.is_empty() {
				if down {
					self.selected = (self.selected + 1) % choices.len();
				}
				if up {
					self.selected = (self.selected + choices.len() - 1) % choices.len();
				}
				if enter {
					target = Some(choices[self.selected].target);
				}
			}
			ui.add_space(2.0);
			ui.label(design::eyebrow(
				ui,
				if self.query.trim().is_empty() {
					"Conversations and friends"
				} else {
					"Results"
				},
				colors.muted,
			));
			ui.spacing_mut().item_spacing.y = 2.0;
			if choices.is_empty() {
				egui::Frame::new()
					.inner_margin(egui::Margin::symmetric(0, 18))
					.show(ui, |ui| {
						ui.vertical_centered(|ui| {
							icons::inline(ui, icons::Icon::Search, 28.0, colors.muted);
							ui.add_space(6.0);
							ui.label(
								design::semibold(ui, "No conversations or friends match", 14.0)
									.color(colors.text),
							);
							ui.label(
								egui::RichText::new("Try a channel, server or person name.")
									.size(12.0)
									.color(colors.muted),
							);
						});
					});
			}
			egui::ScrollArea::vertical()
				.max_height((ctx.content_rect().height() - 220.0).clamp(88.0, 400.0))
				.show(ui, |ui| {
					ui.set_width(ui.available_width());
					for (index, choice) in choices.iter().enumerate() {
						let selected = self.selected == index;
						let row = ui
							.push_id(choice.target, |ui| {
								result_row(ui, choice, selected, !blocked, avatars, state.demo)
							})
							.inner;
						if selected && (up || down || changed) {
							row.scroll_to_me(Some(egui::Align::Center));
						}
						if row.has_focus() {
							self.selected = index;
							if row.gained_focus() {
								row.scroll_to_me(Some(egui::Align::Center));
							}
						}
						if row.clicked() {
							target = Some(choice.target);
						}
					}
				});
			ui.add_space(6.0);
			ui.spacing_mut().item_spacing.y = 8.0;
			// Footer: key hints on the left, close on the right.
			ui.horizontal(|ui| {
				ui.spacing_mut().item_spacing.x = 4.0;
				if !narrow {
					for (keys, action) in [("↑↓", "choose"), ("↵", "open"), ("Esc", "close")]
					{
						key_hint(ui, keys, colors);
						ui.label(egui::RichText::new(action).size(12.0).color(colors.muted));
						ui.add_space(6.0);
					}
				}
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.add_enabled_ui(!blocked, |ui| {
						if design::secondary_button(ui, "Close").clicked() {
							cancel = true;
						}
					});
				});
			});
		});
		if !blocked && response.backdrop_response.clicked() {
			cancel = true;
		}
		if cancel || target.is_some() {
			self.close(ctx, cancel);
		}
		if cancel { None } else { target }
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn state() -> State {
		let mut state = test_support::demo_state();
		state.channels = (1..=30)
			.map(|id| Channel {
				id: Id(id),
				guild: None,
				parent_id: None,
				position: 0,
				name: format!("Room {id}"),
				kind: 1,
				recipients: vec![model::User {
					id: Id(90),
					name: "Žofie Example".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				}],
				member_list_id: None,
				message_count: None,
				icon: None,
				last_message: None,
			})
			.collect();
		state.selected = Some(Id(25));
		state
	}

	#[test]
	fn lowercase_buffer_matches_allocating_normalization() {
		let mut buffer = String::new();
		for value in [
			"",
			"General Chat",
			"Žofie Example",
			"ΟΔΥΣΣΕΎΣ ΑΣ",
			"İstanbul",
			&"🦀A".repeat(1000),
			&"x".repeat(1000),
		] {
			lowercase_bounded_into(&mut buffer, value);
			assert_eq!(buffer, bounded(value).to_lowercase(), "{value:?}");
		}
	}

	#[test]
	fn search_is_bounded_scoped_and_matches_words_across_labels() {
		let mut state = state();
		assert_eq!(candidates(&state, "").len(), RESULTS);
		assert_eq!(candidates(&state, "")[0].target, Target::Channel(Id(25)));
		assert_eq!(
			candidates(&state, "ROOM 17 ŽOFIE")[0].target,
			Target::Channel(Id(17))
		);
		state.channels[16].kind = 4;
		assert!(candidates(&state, "room 17").is_empty());
		state.channels[16].kind = 0;
		state.channels[16].guild = Some(Id(999));
		assert!(candidates(&state, "room 17").is_empty());
		let guild = state.guilds[0].id;
		state.guilds[0].name = "Synthetic Server".into();
		state.channels[16].guild = Some(guild);
		state.channels[16].kind = 2;
		state
			.permissions
			.replace(test_support::permission_snapshot(&state))
			.unwrap();
		let voice = candidates(&state, "SERVER ROOM 17");
		assert_eq!(voice.len(), 1);
		assert_eq!(voice[0].target, Target::Channel(Id(17)));
		assert!(voice[0].label().contains("Voice · roster"));
		state.channels[0].name = "🦀".repeat(1000);
		assert!(bounded(&state.channels[0].name).len() <= 512);
		assert!(
			candidates(&state, "")
				.iter()
				.all(|c| c.label().len() <= 1100)
		);
	}

	fn key(key: egui::Key) -> egui::Event {
		egui::Event::Key {
			key,
			physical_key: None,
			pressed: true,
			repeat: false,
			modifiers: egui::Modifiers::NONE,
		}
	}

	#[test]
	fn keyboard_selection_composition_and_cancel_restore_focus() {
		for dark in [false, true] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let state = state();
			let mut switcher = Switcher::default();
			let mut avatars = Avatars::default();
			let prior = egui::Id::unique("previous-input");
			ctx.memory_mut(|memory| memory.request_focus(prior));
			switcher.open(&ctx);
			let mut previous_text = String::from("Unsent draft");
			let mut frame = |switcher: &mut Switcher, events| {
				let mut result = None;
				let output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(egui::Rect::from_min_size(
							egui::Pos2::ZERO,
							egui::vec2(340.0, 480.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						result = switcher.show(&ctx, &state, &mut avatars);
						ui.add(egui::TextEdit::singleline(&mut previous_text).id(prior));
					},
				);
				output.drop_without_applying_deltas();
				result
			};
			assert_eq!(frame(&mut switcher, vec![]), None);
			assert_eq!(frame(&mut switcher, vec![key(egui::Key::ArrowDown)]), None);
			assert_eq!(
				ctx.memory(|m| m.focused()),
				Some(egui::Id::unique("conversation-switcher-query"))
			);
			assert_eq!(
				frame(&mut switcher, vec![key(egui::Key::Enter)]),
				Some(Target::Channel(Id(30)))
			);
			assert!(!switcher.is_open());
			ctx.memory_mut(|memory| memory.request_focus(prior));
			switcher.open(&ctx);
			frame(&mut switcher, vec![]);
			assert_eq!(
				frame(
					&mut switcher,
					vec![
						egui::Event::Ime(egui::ImeEvent::Preedit {
							text: "Ž".into(),
							active_range_chars: None
						}),
						key(egui::Key::Enter)
					]
				),
				None
			);
			assert!(switcher.is_open());
			assert_eq!(frame(&mut switcher, vec![key(egui::Key::Enter)]), None);
			assert!(switcher.is_open());
			let outside = egui::pos2(2.0, 2.0);
			frame(
				&mut switcher,
				vec![
					egui::Event::PointerMoved(outside),
					egui::Event::PointerButton {
						pos: outside,
						button: egui::PointerButton::Primary,
						pressed: true,
						modifiers: egui::Modifiers::NONE,
					},
					egui::Event::PointerButton {
						pos: outside,
						button: egui::PointerButton::Primary,
						pressed: false,
						modifiers: egui::Modifiers::NONE,
					},
				],
			);
			assert!(
				switcher.is_open(),
				"Backdrop must not send a later IME commit to the composer"
			);
			assert_eq!(
				frame(
					&mut switcher,
					vec![
						egui::Event::Ime(egui::ImeEvent::Commit("Ž".into())),
						key(egui::Key::Enter)
					]
				),
				None
			);
			assert!(switcher.is_open());
			assert!(!candidates(&state, &switcher.query).is_empty());
			frame(&mut switcher, vec![key(egui::Key::Escape)]);
			assert!(!switcher.is_open());
			for _ in 0..3 {
				if switcher.previous_focus.is_none() {
					break;
				}
				frame(&mut switcher, vec![]);
			}
			assert!(
				switcher.previous_focus.is_none(),
				"Closed modal must release pending focus restoration"
			);
			assert_eq!(ctx.memory(|m| m.focused()), Some(prior));
			switcher.open(&ctx);
			frame(&mut switcher, vec![]);
			frame(&mut switcher, vec![key(egui::Key::Tab)]);
			frame(&mut switcher, vec![key(egui::Key::Tab)]);
			assert_eq!(
				frame(&mut switcher, vec![key(egui::Key::Enter)]),
				Some(Target::Channel(Id(30))),
				"Enter must activate the focused result"
			);
			switcher.open(&ctx);
			switcher.query = "Room 17".into();
			frame(&mut switcher, vec![]);
			frame(&mut switcher, vec![key(egui::Key::Tab)]);
			frame(&mut switcher, vec![key(egui::Key::Tab)]);
			assert_eq!(
				frame(&mut switcher, vec![key(egui::Key::Enter)]),
				None,
				"Focused Close must never open a result"
			);
			assert!(!switcher.is_open());
			switcher.open(&ctx);
			frame(&mut switcher, vec![]);
			frame(&mut switcher, vec![egui::Event::Paste("🦀".repeat(10_000))]);
			assert_eq!(switcher.query.chars().count(), QUERY_CHARS);
			assert!(switcher.query.len() <= QUERY_BYTES);
			assert!(switcher.query.capacity() <= QUERY_BYTES);
			frame(
				&mut switcher,
				vec![egui::Event::Ime(egui::ImeEvent::Preedit {
					text: "Ž".into(),
					active_range_chars: None,
				})],
			);
			assert!(switcher.composing);
			let dismissed = egui::Event::Ime(egui::ImeEvent::Preedit {
				text: String::new(),
				active_range_chars: None,
			});
			assert_eq!(
				frame(&mut switcher, vec![dismissed, key(egui::Key::Enter)]),
				None
			);
			assert!(!switcher.composing);
			assert!(switcher.is_open());
			frame(&mut switcher, vec![key(egui::Key::Escape)]);
			assert!(!switcher.is_open());
		}
	}

	#[test]
	fn switcher_requests_real_direct_user_avatars() {
		let ctx = egui::Context::default();
		let mut state = state();
		state.demo = false;
		let user = &mut state.channels[24].recipients[0];
		user.avatar = Some("a".repeat(32));
		let expected = user.avatar_key();
		let choices = candidates(&state, "");
		let mut avatars = Avatars::default();
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(640.0, 760.0),
				)),
				..Default::default()
			},
			|ui| {
				ui.set_width(560.0);
				result_row(ui, &choices[0], true, true, &mut avatars, false);
			},
		);
		output.drop_without_applying_deltas();
		assert_eq!(avatars.take_requests(), vec![expected]);
	}
}
