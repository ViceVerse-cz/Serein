use crate::{avatars::Avatars, design, dialog, icons};
use client_core::State;
use egui::RichText;
use model::{Channel, Id, User};

pub enum Action {
	None,
	Cancel,
	Apply(String),
}

/// One full-width hit target with a left-aligned icon and two text styles.
pub fn suggestion_row(ui: &mut egui::Ui, key: &str, title: &str, detail: &str) -> egui::Response {
	let colors = design::palette(ui);
	let width = ui.available_width();
	let text_width = (width - 54.0).max(1.0);
	let title = ui.painter().layout(
		title.to_owned(),
		egui::FontId::new(15.0, design::semibold_family(ui.ctx())),
		colors.text_strong,
		text_width,
	);
	let subtitle = ui.painter().layout(
		detail.to_owned(),
		egui::FontId::proportional(14.0),
		colors.muted,
		text_width,
	);
	let text_height = title.size().y
		+ if detail.is_empty() {
			0.0
		} else {
			2.0 + subtitle.size().y
		};
	let height = (text_height + 14.0).max(if detail.is_empty() { 40.0 } else { 52.0 });
	let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
	response.widget_info(|| {
		egui::WidgetInfo::labeled(
			egui::Role::Button,
			ui.is_enabled(),
			format!("{} {detail}", title.job.text),
		)
	});
	if response.hovered() || response.has_focus() {
		ui.painter().rect_filled(rect, 6, colors.raised);
	}
	let icon_rect = egui::Rect::from_center_size(
		rect.left_center() + egui::vec2(22.0, 0.0),
		egui::Vec2::splat(22.0),
	);
	match key {
		"mentions" => {
			ui.painter().text(
				icon_rect.center(),
				egui::Align2::CENTER_CENTER,
				"@",
				egui::FontId::proportional(24.0),
				colors.muted,
			);
		}
		"" => {
			for (y, x) in [(-7.0, -3.0), (0.0, 4.0), (7.0, -3.0)] {
				let center = icon_rect.center() + egui::vec2(x, y);
				ui.painter().line_segment(
					[
						icon_rect.center() + egui::vec2(-10.0, y),
						icon_rect.center() + egui::vec2(10.0, y),
					],
					egui::Stroke::new(1.5, colors.muted),
				);
				ui.painter().circle_filled(center, 2.5, colors.muted);
			}
		}
		_ => icons::paint(
			ui.painter(),
			match key {
				"from" => icons::Icon::Profile,
				"has" => icons::Icon::Link,
				"in" => icons::channel(0),
				_ => icons::Icon::Search,
			},
			icon_rect,
			colors.muted,
		),
	}
	let position = rect.left_top() + egui::vec2(44.0, (height - text_height) * 0.5);
	let subtitle_position = position + egui::vec2(0.0, title.size().y + 2.0);
	ui.painter().galley(position, title, colors.text_strong);
	if !detail.is_empty() {
		ui.painter()
			.galley(subtitle_position, subtitle, colors.muted);
	}
	response
}

pub struct Draft {
	query: String,
	before: String,
	after: String,
	from_search: String,
	mentions_search: String,
	in_search: String,
	date_open: bool,
	error: Option<&'static str>,
}

pub fn users(state: &State) -> Vec<&User> {
	let mut users = std::collections::BTreeMap::new();
	for user in state
		.user
		.iter()
		.chain(
			state
				.channels
				.iter()
				.filter(|c| Some(c.id) == state.selected)
				.flat_map(|c| &c.recipients),
		)
		.chain(state.timeline.iter().map(|m| &m.author))
		.chain(state.members.iter().flat_map(|list| {
			list.slots
				.iter()
				.flatten()
				.filter_map(|slot| match slot {
					model::MemberSlot::Person(m) => Some(m),
					_ => None,
				})
				.map(|member| &member.user)
		}))
		.take(1500)
	{
		users.entry(user.id).or_insert(user);
		if users.len() >= 100 {
			break;
		}
	}
	users.into_values().collect()
}

/// Server whose channels a search spans; direct messages have none.
pub fn guild(state: &State) -> Option<Id> {
	state
		.selected
		.and_then(|id| state.channel(id))
		.and_then(|channel| channel.guild)
}

/// Readable channels of the searched server, the current one first.
pub fn channels(state: &State) -> Vec<&Channel> {
	let Some(guild) = guild(state) else {
		return Vec::new();
	};
	let mut channels: Vec<_> = state
		.channels
		.iter()
		.filter(|channel| {
			channel.guild == Some(guild)
				&& channel.supports_text()
				&& state.can_read_history(channel.id)
		})
		.take(500)
		.collect();
	channels.sort_by_key(|channel| (Some(channel.id) != state.selected, channel.position));
	channels
}

/// The filter being typed at the end of the query: its start, key and partial value.
pub fn active_token(query: &str) -> Option<(usize, &'static str, &str)> {
	for key in ["from", "mentions", "in"] {
		let prefix = format!("{key}:");
		if let Some(start) = query.rfind(&prefix) {
			if start > 0 && !query[..start].ends_with(char::is_whitespace) {
				continue;
			}
			// A trailing space commits the token, like picking a suggestion does.
			let typed = &query[start + prefix.len()..];
			if !typed.contains(char::is_whitespace) && typed.parse::<u64>().is_err() {
				return Some((start, key, typed));
			}
		}
	}
	None
}

/// One-word form of a name, so a filter label stays a single query token.
pub fn label(name: &str) -> String {
	name.split_whitespace()
		.collect::<Vec<_>>()
		.join("_")
		.chars()
		.take(64)
		.collect()
}

/// Names picked from suggestions, so the field can show `from:name` while search sends IDs.
#[derive(Default)]
pub struct Labels(Vec<(&'static str, String, Id)>);

impl Labels {
	const LIMIT: usize = 64;

	pub fn remember(&mut self, kind: &'static str, label: &str, id: Id) {
		self.0
			.retain(|(k, l, i)| *k != kind || (*i != id && !l.eq_ignore_ascii_case(label)));
		if self.0.len() >= Self::LIMIT {
			self.0.remove(0);
		}
		self.0.push((kind, label.to_owned(), id));
	}
	fn id(&self, kind: &str, label: &str) -> Option<Id> {
		self.0
			.iter()
			.rev()
			.find(|(k, l, _)| *k == kind && l.to_lowercase() == label.to_lowercase())
			.map(|(_, _, id)| *id)
	}
	fn label(&self, kind: &str, id: Id) -> Option<&str> {
		self.0
			.iter()
			.rev()
			.find(|(k, _, i)| *k == kind && *i == id)
			.map(|(_, label, _)| label.as_str())
	}
}

fn numeric(value: &str) -> Option<Id> {
	value.parse::<u64>().ok().filter(|id| *id != 0).map(Id)
}

fn user_id(value: &str, state: &State, labels: &Labels) -> Option<Id> {
	numeric(value)
		.or_else(|| labels.id("user", value))
		.or_else(|| {
			let value = value.to_lowercase();
			users(state)
				.into_iter()
				.find(|user| label(&user.name).to_lowercase() == value)
				.map(|user| user.id)
		})
}

fn channel_id(value: &str, state: &State, labels: &Labels) -> Option<Id> {
	let value = value.strip_prefix('#').unwrap_or(value);
	numeric(value)
		.or_else(|| labels.id("channel", value))
		.or_else(|| {
			let value = value.to_lowercase();
			channels(state)
				.into_iter()
				.find(|channel| label(&channel.name).to_lowercase() == value)
				.map(|channel| channel.id)
		})
}

/// The query Discord receives: readable labels and dates become IDs.
pub fn wire(query: &str, state: &State, labels: &Labels) -> Result<String, &'static str> {
	let mut changed = false;
	let mut tokens = Vec::new();
	for token in query.split_whitespace() {
		let Some((key, value)) = token.split_once(':') else {
			tokens.push(token.to_owned());
			continue;
		};
		let next = match key {
			"from" | "mentions" => user_id(value, state, labels)
				.map(|id| format!("{key}:{id}"))
				.ok_or("Choose a user from the suggestions.")?,
			"in" if guild(state).is_some() => channel_id(value, state, labels)
				.map(|id| format!("in:{id}"))
				.ok_or("Choose a channel from the suggestions.")?,
			"before" | "after" => super::date_id(value)
				.ok()
				.flatten()
				.map(|id| format!("{key}_id:{id}"))
				.ok_or("Enter dates as YYYY-MM-DD, after January 1, 2015.")?,
			_ => token.to_owned(),
		};
		changed |= next != token;
		tokens.push(next);
	}
	Ok(if changed {
		tokens.join(" ")
	} else {
		query.to_owned()
	})
}

fn snowflake_date(value: &str) -> Option<String> {
	let id = value.parse::<u64>().ok()?;
	let instant =
		time::OffsetDateTime::from_unix_timestamp((((id >> 22) + 1_420_070_400_000) / 1000) as i64)
			.ok()?;
	Some(instant.date().to_string())
}

/// The query the field shows: IDs and cursors become names and dates.
pub fn display(query: &str, state: &State, labels: &mut Labels) -> String {
	let mut changed = false;
	let mut tokens = Vec::new();
	for token in query.split_whitespace() {
		let next = token.split_once(':').and_then(|(key, value)| match key {
			"from" | "mentions" => {
				let id = numeric(value)?;
				let name = labels.label("user", id).map(str::to_owned).or_else(|| {
					users(state)
						.into_iter()
						.find(|user| user.id == id)
						.map(|user| label(&user.name))
				})?;
				labels.remember("user", &name, id);
				Some(format!("{key}:{name}"))
			}
			"in" => {
				let id = numeric(value)?;
				let name = state
					.channel(id)
					.map(|channel| label(&channel.name))
					.or_else(|| labels.label("channel", id).map(str::to_owned))?;
				labels.remember("channel", &name, id);
				Some(format!("in:{name}"))
			}
			"before_id" | "after_id" => Some(format!(
				"{}:{}",
				key.trim_end_matches("_id"),
				snowflake_date(value)?
			)),
			_ => None,
		});
		changed |= next.is_some();
		tokens.push(next.unwrap_or_else(|| token.to_owned()));
	}
	if changed {
		tokens.join(" ")
	} else {
		query.to_owned()
	}
}

/// Paint filter tokens as chips inside the query field, like Discord's search bar.
pub fn highlight(ui: &egui::Ui, text: &str, wrap_width: f32) -> egui::text::LayoutJob {
	let colors = design::palette(ui);
	let font = egui::TextStyle::Body.resolve(ui.style());
	let plain = egui::TextFormat::simple(font.clone(), colors.text_strong);
	let mut job = egui::text::LayoutJob::default();
	job.wrap.max_width = wrap_width;
	let mut rest = text;
	while !rest.is_empty() {
		let start = rest.len() - rest.trim_start().len();
		if start > 0 {
			job.append(&rest[..start], 0.0, plain.clone());
			rest = &rest[start..];
			continue;
		}
		let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
		let token = &rest[..end];
		match token.split_once(':') {
			Some((key, value)) if FILTER_KEYS.contains(&key) => {
				let chip = |color| egui::TextFormat {
					background: colors.mention_bg,
					..egui::TextFormat::simple(font.clone(), color)
				};
				job.append(&token[..=key.len()], 0.0, chip(colors.muted));
				if !value.is_empty() {
					job.append(value, 0.0, chip(colors.mention_text));
				}
			}
			_ => job.append(token, 0.0, plain.clone()),
		}
		rest = &rest[end..];
	}
	job
}

const FILTER_KEYS: &[&str] = &[
	"from",
	"mentions",
	"in",
	"has",
	"before",
	"after",
	"before_id",
	"after_id",
	"author_type",
	"pinned",
];

pub fn channel_row(
	ui: &mut egui::Ui,
	state: &State,
	channel: &Channel,
	selected: bool,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), egui::Sense::click());
	if selected || response.hovered() || response.has_focus() {
		ui.painter().rect_filled(rect, 8, colors.raised);
	}
	ui.scope_builder(
		egui::UiBuilder::new()
			.max_rect(rect.shrink2(egui::vec2(8.0, 4.0)))
			.layout(egui::Layout::left_to_right(egui::Align::Center)),
		|ui| {
			ui.spacing_mut().item_spacing.x = 6.0;
			icons::inline(ui, icons::channel(channel.kind), 16.0, colors.muted);
			ui.add(
				egui::Label::new(design::medium(ui, &channel.name, 14.0).color(colors.text_strong))
					.truncate()
					.selectable(false),
			);
			if let Some(parent) = channel.parent_id.and_then(|id| state.channel(id)) {
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.add(
						egui::Label::new(
							RichText::new(&parent.name).size(12.0).color(colors.muted),
						)
						.truncate()
						.selectable(false),
					);
				});
			}
		},
	);
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &channel.name));
	response
}

pub fn user_row(
	ui: &mut egui::Ui,
	user: &User,
	avatars: &mut Avatars,
	demo: bool,
	selected: bool,
) -> egui::Response {
	let colors = design::palette(ui);
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
	if selected || response.hovered() || response.has_focus() {
		ui.painter().rect_filled(rect, 8, colors.raised);
	}
	let avatar = ui
		.scope_builder(
			egui::UiBuilder::new()
				.max_rect(rect.shrink2(egui::vec2(8.0, 4.0)))
				.layout(egui::Layout::left_to_right(egui::Align::Center)),
			|ui| {
				let avatar = avatars.show(ui, user, 24.0, demo);
				ui.add(
					egui::Label::new(
						design::semibold(ui, &user.name, 14.0).color(colors.text_strong),
					)
					.truncate()
					.selectable(false),
				);
				avatar
			},
		)
		.inner;
	response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &user.name));
	response.union(avatar)
}

fn selected(query: &str, key: &str, value: &str) -> bool {
	query
		.split_whitespace()
		.any(|token| token.split_once(':') == Some((key, value)))
}

fn replace(query: &mut String, key: &str, value: &str, multi: bool) {
	let remove = selected(query, key, value);
	let mut next = query
		.split_whitespace()
		.filter(|token| {
			token
				.split_once(':')
				.is_none_or(|(k, v)| k != key || (multi && v != value))
		})
		.collect::<Vec<_>>()
		.join(" ");
	if !value.is_empty() && !(multi && remove) {
		if !next.is_empty() {
			next.push(' ');
		}
		next.push_str(&format!("{key}:{value}"));
	}
	// Keep the draft subject to the same byte, character and filter-count limits.
	if next.is_empty() || model::search_terms(&next).is_ok() {
		*query = next;
	}
}

fn heading(ui: &mut egui::Ui, name: &str, help: &str) {
	design::section(ui, name, Some(help));
}

fn caption(query: &str, key: &str, names: &[(Id, &str)], placeholder: &str) -> String {
	let values: Vec<_> = query
		.split_whitespace()
		.filter_map(|token| {
			let (k, value) = token.split_once(':')?;
			(k == key).then(|| {
				names
					.iter()
					.find(|(id, _)| id.to_string() == value)
					.map_or_else(|| value.to_owned(), |(_, name)| (*name).to_owned())
			})
		})
		.collect();
	if values.is_empty() {
		placeholder.to_owned()
	} else {
		values.join(", ")
	}
}

fn user_picker(
	ui: &mut egui::Ui,
	query: &mut String,
	key: &str,
	needle: &mut String,
	users: &[&User],
	avatars: &mut Avatars,
	demo: bool,
) {
	egui::ComboBox::from_id_salt(key)
		.width(ui.available_width())
		.selected_text(caption(
			query,
			key,
			&users
				.iter()
				.map(|user| (user.id, user.name.as_str()))
				.collect::<Vec<_>>(),
			&crate::i18n::translate("search-filters-user-picker-choose-a-user"),
		))
		.show_ui(ui, |ui| {
			ui.add(egui::TextEdit::singleline(needle).char_limit(64).hint_text(
				crate::i18n::translate("search-filters-user-picker-search-users"),
			));
			let mut count = 0;
			for user in users
				.iter()
				.filter(|user| user.name.to_lowercase().contains(&needle.to_lowercase()))
			{
				count += 1;
				let id = user.id.to_string();
				if user_row(ui, user, avatars, demo, selected(query, key, &id)).clicked() {
					replace(query, key, &id, true);
				}
			}
			if count == 0 {
				ui.label(crate::i18n::translate(
					"search-filters-user-picker-no-matching-users",
				));
			}
		});
}

fn channel_picker(
	ui: &mut egui::Ui,
	state: &State,
	query: &mut String,
	needle: &mut String,
	channels: &[&Channel],
) {
	egui::ComboBox::from_id_salt("in")
		.width(ui.available_width())
		.selected_text(caption(
			query,
			"in",
			&channels
				.iter()
				.map(|channel| (channel.id, channel.name.as_str()))
				.collect::<Vec<_>>(),
			&crate::i18n::translate("search-filters-channel-picker-choose-a-channel"),
		))
		.show_ui(ui, |ui| {
			ui.add(egui::TextEdit::singleline(needle).char_limit(64).hint_text(
				crate::i18n::translate("search-filters-channel-picker-search-channels"),
			));
			let needle = needle.to_lowercase();
			let mut count = 0;
			egui::ScrollArea::vertical()
				.max_height(260.0)
				.show(ui, |ui| {
					for channel in channels
						.iter()
						.filter(|channel| channel.name.to_lowercase().contains(&needle))
						.take(100)
					{
						count += 1;
						let id = channel.id.to_string();
						if channel_row(ui, state, channel, selected(query, "in", &id)).clicked() {
							replace(query, "in", &id, true);
						}
					}
				});
			if count == 0 {
				ui.label(crate::i18n::translate(
					"search-overlays-no-matching-channels",
				));
			}
		});
}

fn choices(
	ui: &mut egui::Ui,
	query: &mut String,
	key: &str,
	placeholder: &str,
	values: &[(&str, &str)],
	multi: bool,
) {
	egui::ComboBox::from_id_salt(key)
		.width(ui.available_width())
		.selected_text(caption(
			query,
			key,
			&[],
			&crate::i18n::translate_if_key(placeholder),
		))
		.show_ui(ui, |ui| {
			if !multi
				&& ui
					.selectable_label(
						!query
							.split_whitespace()
							.any(|t| t.starts_with(&format!("{key}:"))),
						crate::i18n::translate("search-filters-choices-any"),
					)
					.clicked()
			{
				replace(query, key, "", false);
			}
			for (value, label) in values {
				if ui
					.selectable_label(
						selected(query, key, value),
						crate::i18n::translate_if_key(label),
					)
					.clicked()
				{
					replace(query, key, value, multi);
				}
			}
		});
}

impl Draft {
	pub fn new(query: &str) -> Self {
		let query = active_token(query).map_or(query, |(start, _, _)| query[..start].trim());
		let date = |key| {
			query
				.split_whitespace()
				.find_map(|token| {
					let (k, value) = token.split_once(':')?;
					if k != key {
						return None;
					}
					let id = value.parse::<u64>().ok()?;
					let instant = time::OffsetDateTime::from_unix_timestamp(
						(((id >> 22) + 1_420_070_400_000) / 1000) as i64,
					)
					.ok()?;
					Some(instant.date().to_string())
				})
				.unwrap_or_default()
		};
		let before = date("before_id");
		let after = date("after_id");
		Self {
			query: query.to_owned(),
			date_open: !before.is_empty() || !after.is_empty(),
			before,
			after,
			from_search: String::new(),
			mentions_search: String::new(),
			in_search: String::new(),
			error: None,
		}
	}

	pub fn show(&mut self, ctx: &egui::Context, state: &State, avatars: &mut Avatars) -> Action {
		let colors = design::palette_for(ctx);
		let width = 560.0_f32.min((ctx.content_rect().width() - 32.0).max(240.0));
		let users = users(state);
		let channels = channels(state);
		let mut action = Action::None;
		let response = dialog::Dialog::new(
			"message-search-filter-dialog",
			crate::i18n::translate("search-filters-show-filters"),
		)
		.subtitle(crate::i18n::translate(
			"search-filters-show-narrow-this-search-down-to-the-messages-you-want",
		))
		.width(width)
		.show(ctx, |d| {
			d.scroll(230.0, |ui| {
				egui::Frame::new().inner_margin(0).show(ui, |ui| {
					ui.set_width(ui.available_width());
					ui.spacing_mut().item_spacing.y = 4.0;
					ui.spacing_mut().interact_size.y = 40.0;
					ui.visuals_mut().widgets.inactive.bg_fill = colors.base;
					ui.visuals_mut().widgets.inactive.weak_bg_fill = colors.base;
					ui.visuals_mut().widgets.inactive.bg_stroke =
						egui::Stroke::new(1.0, colors.border);
					heading(ui, "From", "Sent by any of the selected users");
					user_picker(
						ui,
						&mut self.query,
						"from",
						&mut self.from_search,
						&users,
						avatars,
						state.demo,
					);
					ui.add_space(22.0);
					if !channels.is_empty() {
						heading(ui, "In", "Sent in any of the selected channels");
						channel_picker(ui, state, &mut self.query, &mut self.in_search, &channels);
						ui.add_space(22.0);
					}
					heading(ui, "Has", "Includes any of the selected types of data");
					choices(
						ui,
						&mut self.query,
						"has",
						"search-filters-show-any-content",
						&[
							("link", "search-filters-show-link-2"),
							("embed", "search-filters-show-embed-2"),
							("file", "search-filters-show-file-2"),
							("image", "search-filters-show-image-2"),
							("video", "search-filters-show-video-2"),
							("sound", "search-filters-show-sound-2"),
						],
						true,
					);
					ui.add_space(22.0);
					heading(ui, "Mentions", "Mentions any of the selected users");
					user_picker(
						ui,
						&mut self.query,
						"mentions",
						&mut self.mentions_search,
						&users,
						avatars,
						state.demo,
					);
					ui.add_space(22.0);
					heading(ui, "Date", "When the message was sent");
					if !self.date_open {
						if ui
							.add_sized(
								[ui.available_width(), 42.0],
								egui::Button::new(crate::i18n::translate(
									"search-filters-show-add-date",
								)),
							)
							.clicked()
						{
							self.date_open = true;
						}
					} else {
						for (label, date) in
							[("After", &mut self.after), ("Before", &mut self.before)]
						{
							ui.label(label);
							ui.add(
								egui::TextEdit::singleline(date)
									.hint_text(crate::i18n::translate(
										"search-filters-show-yyyy-mm-dd",
									))
									.char_limit(10)
									.desired_width(f32::INFINITY),
							);
						}
						if ui
							.button(crate::i18n::translate("search-filters-show-remove-dates"))
							.clicked()
						{
							self.after.clear();
							self.before.clear();
							self.date_open = false;
						}
					}
					ui.add_space(22.0);
					heading(
						ui,
						"Author Type",
						"Sent by any of the selected types of author",
					);
					choices(
						ui,
						&mut self.query,
						"author_type",
						"search-filters-show-choose-author-type",
						&[
							("user", "search-filters-show-user-2"),
							("bot", "search-filters-show-bot-2"),
							("webhook", "search-filters-show-webhook-2"),
						],
						true,
					);
					ui.add_space(22.0);
					heading(ui, "Pinned", "If the message is pinned or not");
					choices(
						ui,
						&mut self.query,
						"pinned",
						"search-filters-show-any",
						&[
							("true", "search-filters-show-true-2"),
							("false", "search-filters-show-false-2"),
						],
						false,
					);
					if let Some(error) = self.error {
						dialog::notice(ui, dialog::Level::Error, error);
					}
				});
			});
			d.footer(|ui| {
				ui.add_enabled_ui(state.can_search(), |ui| {
					if dialog::action(
						ui,
						"search-filters-show-apply-filters",
						dialog::Action::Primary,
					)
					.clicked()
					{
						match self.applied() {
							Ok(query) => action = Action::Apply(query),
							Err(error) => self.error = Some(error),
						}
					}
				});
				if dialog::action(ui, "search-filters-show-cancel", dialog::Action::Neutral)
					.clicked()
				{
					action = Action::Cancel;
				}
				ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
					if ui
						.add(
							egui::Button::new(
								RichText::new(crate::i18n::translate(
									"search-filters-show-clear-filters",
								))
								.color(colors.accent),
							)
							.frame(false),
						)
						.clicked()
					{
						self.query = model::search_terms(&self.query)
							.map(|(content, _)| content)
							.unwrap_or_default();
						self.before.clear();
						self.after.clear();
						self.date_open = false;
						self.error = None;
					}
				});
			});
		});
		if response.close {
			Action::Cancel
		} else {
			action
		}
	}

	fn applied(&self) -> Result<String, &'static str> {
		let after = super::date_id(&self.after)
			.map_err(|_| "Enter dates as YYYY-MM-DD, after January 1, 2015.")?;
		let before = super::date_id(&self.before)
			.map_err(|_| "Enter dates as YYYY-MM-DD, after January 1, 2015.")?;
		if after.zip(before).is_some_and(|(a, b)| a >= b) {
			return Err("After must be earlier than Before.");
		}
		let mut query = self
			.query
			.split_whitespace()
			.filter(|token| !token.starts_with("after_id:") && !token.starts_with("before_id:"))
			.collect::<Vec<_>>()
			.join(" ");
		for (key, id) in [("after_id", after), ("before_id", before)] {
			if let Some(id) = id {
				if !query.is_empty() {
					query.push(' ');
				}
				query.push_str(&format!("{key}:{id}"));
			}
		}
		if !query.is_empty() {
			model::search_terms(&query)?;
		}
		Ok(query)
	}
}
