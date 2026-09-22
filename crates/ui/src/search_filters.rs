use crate::{avatars::Avatars, design, dialog, icons};
use client_core::State;
use egui::RichText;
use model::User;

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

pub fn active_user_token(query: &str) -> Option<(usize, &str, &str)> {
	for key in ["from", "mentions"] {
		let prefix = format!("{key}:");
		if let Some(start) = query.rfind(&prefix) {
			if start > 0 && !query[..start].ends_with(char::is_whitespace) {
				continue;
			}
			let typed = query[start + prefix.len()..].trim();
			if !typed.contains(char::is_whitespace) && typed.parse::<u64>().is_err() {
				return Some((start, key, typed));
			}
		}
	}
	None
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

fn caption(query: &str, key: &str, users: &[&User], placeholder: &str) -> String {
	let values: Vec<_> = query
		.split_whitespace()
		.filter_map(|token| {
			let (k, value) = token.split_once(':')?;
			(k == key).then(|| {
				users
					.iter()
					.find(|user| user.id.to_string() == value)
					.map_or_else(|| value.to_owned(), |user| user.name.clone())
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
		.selected_text(caption(query, key, users, "Choose a user"))
		.show_ui(ui, |ui| {
			ui.add(
				egui::TextEdit::singleline(needle)
					.char_limit(64)
					.hint_text("Search users"),
			);
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
				ui.label("No matching users");
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
		.selected_text(caption(query, key, &[], placeholder))
		.show_ui(ui, |ui| {
			if !multi
				&& ui
					.selectable_label(
						!query
							.split_whitespace()
							.any(|t| t.starts_with(&format!("{key}:"))),
						"Any",
					)
					.clicked()
			{
				replace(query, key, "", false);
			}
			for (value, label) in values {
				if ui
					.selectable_label(selected(query, key, value), *label)
					.clicked()
				{
					replace(query, key, value, multi);
				}
			}
		});
}

impl Draft {
	pub fn new(query: &str) -> Self {
		let query = active_user_token(query).map_or(query, |(start, _, _)| query[..start].trim());
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
			error: None,
		}
	}

	pub fn show(&mut self, ctx: &egui::Context, state: &State, avatars: &mut Avatars) -> Action {
		let colors = design::palette_for(ctx);
		let width = 560.0_f32.min((ctx.content_rect().width() - 32.0).max(240.0));
		let users = users(state);
		let mut action = Action::None;
		let response = dialog::Dialog::new("message-search-filter-dialog", "Filters")
			.subtitle("Narrow this search down to the messages you want.")
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
						heading(ui, "Has", "Includes any of the selected types of data");
						choices(
							ui,
							&mut self.query,
							"has",
							"Any content",
							&[
								("link", "Link"),
								("embed", "Embed"),
								("file", "File"),
								("image", "Image"),
								("video", "Video"),
								("sound", "Sound"),
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
									egui::Button::new("+  Add date"),
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
										.hint_text("YYYY-MM-DD")
										.char_limit(10)
										.desired_width(f32::INFINITY),
								);
							}
							if ui.button("Remove dates").clicked() {
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
							"Choose author type",
							&[("user", "User"), ("bot", "Bot"), ("webhook", "Webhook")],
							true,
						);
						ui.add_space(22.0);
						heading(ui, "Pinned", "If the message is pinned or not");
						choices(
							ui,
							&mut self.query,
							"pinned",
							"Any",
							&[("true", "True"), ("false", "False")],
							false,
						);
						if let Some(error) = self.error {
							dialog::notice(ui, dialog::Level::Error, error);
						}
					});
				});
				d.footer(|ui| {
					ui.add_enabled_ui(state.can_search(), |ui| {
						if dialog::action(ui, "Apply Filters", dialog::Action::Primary).clicked() {
							match self.applied() {
								Ok(query) => action = Action::Apply(query),
								Err(error) => self.error = Some(error),
							}
						}
					});
					if dialog::action(ui, "Cancel", dialog::Action::Neutral).clicked() {
						action = Action::Cancel;
					}
					ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
						if ui
							.add(
								egui::Button::new(
									RichText::new("Clear Filters").color(colors.accent),
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
