//! Browse Channels: Discord's opt-in channel list, one checkbox per channel and category.
use crate::{design, dialog, icons};
use client_core::{Command, State, channel_list::Change};
use egui::RichText;
use model::{Channel, Id};

#[derive(Default)]
pub(super) struct ChannelBrowser {
	guild: Option<Id>,
	generation: u64,
	query: String,
}

impl ChannelBrowser {
	pub fn open(&mut self, state: &State, guild: Id) {
		self.guild = Some(guild);
		self.generation = state.generation;
		self.query.clear();
	}

	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		active: Option<Id>,
		commands: &mut Vec<Command>,
	) {
		let Some(guild) = self.guild else { return };
		if self.generation != state.generation
			|| active != Some(guild)
			|| state.guild(guild).is_none()
		{
			*self = Self::default();
			return;
		}
		let mut change = None;
		let response = dialog::Dialog::new(
			"browse-channels",
			crate::i18n::translate("channel-browser-title"),
		)
		.subtitle(crate::i18n::translate("channel-browser-subtitle"))
		.icon(icons::Icon::Hash)
		.width(520.0)
		.show(ctx, |body| {
			let enabled = state.can_change_channel_list(guild);
			body.content(|ui| {
				let show_all = state.channel_list_opt_in(guild) != Some(true);
				let mut toggled = show_all;
				ui.add_enabled_ui(enabled, |ui| {
					if crate::channel_menu::toggle_row(
						ui,
						"channel-browser-show-all-channels",
						&mut toggled,
					)
					.changed()
					{
						change = Some(Change::ShowAll(toggled));
					}
				});
				ui.add(
					egui::TextEdit::singleline(&mut self.query)
						.hint_text(crate::i18n::translate("channel-browser-search"))
						.desired_width(f32::INFINITY),
				);
				if let Some(status) = state.channel_list_status(guild) {
					ui.small(crate::i18n::translate_if_key(status));
				} else if !enabled {
					ui.small(crate::i18n::translate(if state.channel_list_pending() {
						"channel-browser-saving"
					} else {
						"channel-browser-unavailable"
					}));
				}
			});
			body.scroll(220.0, |ui| {
				ui.add_enabled_ui(enabled, |ui| {
					for (category, channels) in groups(state, guild, &self.query) {
						if let Some(category) = category
							&& let Some(listed) = category_row(ui, state, category)
						{
							change = Some(Change::List {
								channel: category.id,
								listed,
							});
						}
						for channel in channels {
							if let Some(listed) = channel_row(ui, state, channel) {
								change = Some(Change::List {
									channel: channel.id,
									listed,
								});
							}
						}
					}
				});
			});
		});
		if let Some(change) = change
			&& let Some(command) = state.change_channel_list(guild, change)
		{
			commands.push(command);
		}
		if response.close {
			*self = Self::default();
		}
	}
}

/// Viewable channels grouped like the sidebar: uncategorized first, then each category. A
/// query keeps matching channels and categories whose name matches.
fn groups<'a>(
	state: &'a State,
	guild: Id,
	query: &str,
) -> Vec<(Option<&'a Channel>, Vec<&'a Channel>)> {
	let query = query.trim().to_lowercase();
	let rank = |c: &&Channel| (matches!(c.kind, 2 | 13), c.position, c.id);
	let mut categories: Vec<_> = state
		.channels
		.iter()
		.filter(|c| c.guild == Some(guild) && c.kind == 4 && state.can_view(c.id))
		.collect();
	categories.sort_unstable_by_key(|c| (c.position, c.id));
	let matches = |c: &Channel| query.is_empty() || c.name.to_lowercase().contains(&query);
	let children = |parent: Option<Id>, all: bool| {
		let mut list: Vec<_> = state
			.channels
			.iter()
			.filter(|c| {
				c.guild == Some(guild)
					&& !matches!(c.kind, 4 | 10..=12)
					&& state.can_view(c.id)
					&& match parent {
						Some(parent) => c.parent_id == Some(parent),
						None => c
							.parent_id
							.is_none_or(|id| !categories.iter().any(|cat| cat.id == id)),
					} && (all || matches(c))
			})
			.collect();
		list.sort_unstable_by_key(rank);
		list
	};
	let mut groups = vec![(None, children(None, false))];
	for category in &categories {
		let list = children(Some(category.id), matches(category));
		if !list.is_empty() || matches(category) {
			groups.push((Some(*category), list));
		}
	}
	groups.retain(|(category, list)| category.is_some() || !list.is_empty());
	groups
}

fn checkbox(ui: &egui::Ui, rect: egui::Rect, on: bool) {
	let colors = design::palette(ui);
	let mark = egui::Rect::from_center_size(
		egui::pos2(rect.right() - 16.0, rect.center().y),
		egui::Vec2::splat(22.0),
	);
	ui.painter().rect(
		mark,
		4,
		if on { colors.accent } else { colors.base },
		egui::Stroke::new(1.0, colors.border),
		egui::StrokeKind::Inside,
	);
	if on {
		icons::paint(
			ui.painter(),
			icons::Icon::Check,
			mark.shrink(4.0),
			colors.text_strong,
		);
	}
}

/// Returns the new listed state when the row was clicked.
fn category_row(ui: &mut egui::Ui, state: &State, category: &Channel) -> Option<bool> {
	let colors = design::palette(ui);
	let on = state.channel_followed(category);
	ui.add_space(8.0);
	let response = ui.add(
		egui::Button::new(())
			.left_text(design::eyebrow(
				ui,
				category.name.to_uppercase(),
				colors.muted,
			))
			.min_size(egui::vec2(ui.available_width(), 32.0))
			.frame_when_inactive(false)
			.corner_radius(4),
	);
	let label = crate::i18n::translate("channel-browser-follow-category");
	response.widget_info(|| {
		egui::WidgetInfo::selected(
			egui::Role::CheckBox,
			ui.is_enabled(),
			on,
			format!("{label}, {}", category.name),
		)
	});
	checkbox(ui, response.rect, on);
	response.on_hover_text(label).clicked().then_some(!on)
}

fn channel_row(ui: &mut egui::Ui, state: &State, channel: &Channel) -> Option<bool> {
	let colors = design::palette(ui);
	let on = state.channel_followed(channel);
	let response = ui
		.scope(|ui| {
			ui.spacing_mut().button_padding = egui::vec2(36.0, 8.0);
			ui.add(
				egui::Button::new(())
					.left_text(RichText::new(&channel.name).color(if on {
						colors.text
					} else {
						colors.muted
					}))
					.min_size(egui::vec2(ui.available_width(), 34.0))
					.frame_when_inactive(false)
					.corner_radius(4),
			)
		})
		.inner;
	response.widget_info(|| {
		egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), on, &channel.name)
	});
	icons::paint(
		ui.painter(),
		icons::channel(channel.kind),
		egui::Rect::from_center_size(
			egui::pos2(response.rect.left() + 18.0, response.rect.center().y),
			egui::Vec2::splat(18.0),
		),
		colors.muted,
	);
	checkbox(ui, response.rect, on);
	response.clicked().then_some(!on)
}
