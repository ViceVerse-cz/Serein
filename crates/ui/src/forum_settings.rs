//! Forum channel settings: post guidelines, tags, the default reaction and post defaults.
use crate::{avatars::Avatars, design, dialog, icons};
use client_core::{
	State,
	channel_actions::{Edit, ForumEdit, HIDE_AFTER, TAG_NAME_LIMIT},
};
use egui::RichText;
use model::{
	Id, ReactionEmoji,
	forum::{Layout, MAX_TAGS, Sort, Tag},
};

/// Discord's slowmode steps, in seconds.
pub(crate) const SLOWMODE: [(u32, &str); 14] = [
	(0, "Off"),
	(5, "5 seconds"),
	(10, "10 seconds"),
	(15, "15 seconds"),
	(30, "30 seconds"),
	(60, "1 minute"),
	(120, "2 minutes"),
	(300, "5 minutes"),
	(600, "10 minutes"),
	(900, "15 minutes"),
	(1800, "30 minutes"),
	(3600, "1 hour"),
	(7200, "2 hours"),
	(21600, "6 hours"),
];

/// A tag being created (`index` None) or edited in place.
struct TagDraft {
	index: Option<usize>,
	tag: Tag,
}

/// What the emoji chooser offers: a server emoji, or one from the bundled Unicode catalog.
#[derive(Clone, Copy)]
enum Choice {
	Custom(usize),
	Unicode(usize),
}

#[derive(Default)]
pub(crate) struct ForumSettingsUi {
	tag: Option<TagDraft>,
	query: String,
	/// The query `matches` was filtered for, so the catalog is scanned once per keystroke.
	filtered: Option<String>,
	matches: Vec<usize>,
}

impl ForumSettingsUi {
	/// The forum part of the Overview page, under the channel name.
	pub(crate) fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		draft: &mut Edit,
		images: &mut Avatars,
	) {
		let Some(forum) = draft.forum.as_deref_mut() else {
			return;
		};
		let demo = state.demo;
		ui.add_space(14.0);
		let label = dialog::label(ui, "Post Guidelines");
		let topic = dialog::input(
			ui,
			egui::TextEdit::multiline(&mut draft.topic)
				.hint_text(crate::i18n::translate(
					"Let everyone know how to use this channel!",
				))
				.char_limit(4096)
				.desired_rows(5),
		)
		.labelled_by(label.id);
		if topic.changed() {
			draft.topic.shrink_to_fit();
		}
		ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
			ui.label(
				RichText::new((4096 - draft.topic.chars().count()).to_string())
					.size(12.0)
					.color(design::palette(ui).muted),
			);
		});

		design::divider(ui);
		design::section(
			ui,
			&crate::i18n::translate("Tags"),
			Some(&crate::i18n::translate(
				"Help people organize their posts into subcategories by creating a tag.",
			)),
		);
		self.tags(ui, state, guild, forum, (images, demo));
		ui.add_space(8.0);
		design::switch(
			ui,
			"Require people to select tags when posting",
			None,
			&mut forum.require_tag,
		);

		design::divider(ui);
		design::section(
			ui,
			&crate::i18n::translate("Default Reaction"),
			Some(
				"Pick a default emoji that your members will use to react to a post from this channel.",
			),
		);
		ui.horizontal_top(|ui| {
			ui.vertical(|ui| {
				ui.set_width((ui.available_width() * 0.45).max(200.0));
				let current = forum
					.reaction
					.as_ref()
					.map(|emoji| (emoji.id, emoji.name.clone()));
				let label = if current.is_some() {
					"Change emoji"
				} else {
					"Select emoji"
				};
				ui.horizontal(|ui| {
					if let Some((id, name)) = &current {
						let (rect, _) =
							ui.allocate_exact_size(egui::Vec2::splat(32.0), egui::Sense::hover());
						crate::forum::paint_emoji(ui, (images, demo), (*id, name.as_deref()), rect);
					}
					if let Some(choice) =
						self.chooser(ui, state, guild, images, "default-reaction", label)
					{
						forum.reaction = Some(ReactionEmoji {
							id: choice.0,
							name: choice.1,
						});
					}
					if current.is_some()
						&& dialog::action(ui, "Remove", dialog::Action::Neutral).clicked()
					{
						forum.reaction = None;
					}
				});
			});
			reaction_preview(ui, forum.reaction.as_ref(), (images, demo));
		});

		design::divider(ui);
		design::section(ui, &crate::i18n::translate("Slowmode"), None);
		dialog::label(ui, "Posts");
		slowmode(ui, "slowmode-posts", &mut draft.slowmode);
		dialog::hint(
			ui,
			"Members will be restricted to creating one post per this interval, unless they have the Bypass Slowmode permission.",
		);
		ui.add_space(14.0);
		dialog::label(ui, "Messages");
		slowmode(ui, "slowmode-messages", &mut forum.message_slowmode);
		dialog::hint(
			ui,
			"Members will be limited to one message per this interval for any new posts, unless they have the Bypass Slowmode permission.",
		);

		design::divider(ui);
		design::section(
			ui,
			&crate::i18n::translate("Default Layout"),
			Some(
				"Set the default layout view to a media-focused gallery or a text-focused list. Members will still be able to toggle between these options.",
			),
		);
		select(
			ui,
			"forum-layout",
			&mut forum.layout,
			&[
				(Layout::List, "List View"),
				(Layout::Gallery, "Gallery View"),
			],
		);
		ui.add_space(18.0);
		design::section(
			ui,
			&crate::i18n::translate("Sort Order"),
			Some(
				"Set the default sort order for new posts. Members will still be able to toggle between these options.",
			),
		);
		select(
			ui,
			"forum-sort",
			&mut forum.sort,
			&[
				(Sort::Activity, "Recent Activity"),
				(Sort::Created, "Creation Time"),
			],
		);
		ui.add_space(18.0);
		design::section(
			ui,
			&crate::i18n::translate("Tag Matching"),
			Some(
				"Set the default tag matching behaviour. Members will still be able to toggle between these options.",
			),
		);
		select(
			ui,
			"forum-match",
			&mut forum.match_all,
			&[(false, "Match Some"), (true, "Match All")],
		);

		design::divider(ui);
		design::section(ui, &crate::i18n::translate("Content Visibility"), None);
		if design::radio_row(
			ui,
			!draft.nsfw,
			"Default",
			Some("Channel content is always visible."),
		)
		.clicked()
		{
			draft.nsfw = false;
		}
		if design::radio_row(
			ui,
			draft.nsfw,
			"Age-Restricted Channel",
			Some("Users will need to confirm they are of over the legal age to view the content in this channel. Age-restricted channels are exempt from the explicit content filter."),
		)
		.clicked()
		{
			draft.nsfw = true;
		}

		design::divider(ui);
		design::section(
			ui,
			&crate::i18n::translate("Hide After Inactivity"),
			Some(&crate::i18n::translate(
				"New posts stop showing in the channel list after this long without activity.",
			)),
		);
		let hide_after: Vec<(u32, &str)> = HIDE_AFTER
			.into_iter()
			.zip(["1 Hour", "24 Hours", "3 Days", "1 Week"])
			.collect();
		select(ui, "forum-hide-after", &mut forum.hide_after, &hide_after);
	}

	fn tags(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		forum: &mut ForumEdit,
		(images, demo): (&mut Avatars, bool),
	) {
		let colors = design::palette(ui);
		let mut remove = None;
		for (index, tag) in forum.tags.iter().enumerate() {
			if self
				.tag
				.as_ref()
				.is_some_and(|draft| draft.index == Some(index))
			{
				continue;
			}
			ui.push_id(("forum-tag", index), |ui| {
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 8.0;
					crate::forum::tag_pill(
						ui,
						(images, demo),
						tag,
						false,
						30.0,
						egui::Sense::hover(),
					);
					if tag.moderated {
						icons::inline(ui, icons::Icon::Lock, 14.0, colors.muted);
						ui.label(
							RichText::new(crate::i18n::translate("Moderators only"))
								.size(12.0)
								.color(colors.muted),
						);
					}
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						if icons::button(
							ui,
							icons::Icon::Trash,
							20.0,
							&crate::i18n::translate("Delete tag"),
						)
						.clicked()
						{
							remove = Some(index);
						}
						if icons::button(
							ui,
							icons::Icon::Pencil,
							20.0,
							&crate::i18n::translate("Edit tag"),
						)
						.clicked()
						{
							self.tag = Some(TagDraft {
								index: Some(index),
								tag: tag.clone(),
							});
						}
					});
				});
			});
		}
		if let Some(index) = remove {
			forum.tags.remove(index);
			self.tag = None;
		}
		if self.tag.is_some() {
			self.tag_editor(ui, state, guild, forum, (images, demo));
		} else {
			ui.add_space(4.0);
			let full = forum.tags.len() >= MAX_TAGS;
			let button = ui
				.add_enabled_ui(!full, |ui| {
					dialog::action(ui, "Create Tag", dialog::Action::Primary)
				})
				.inner;
			if full {
				button.on_disabled_hover_text(crate::i18n::translate(
					"A forum can offer up to 20 tags.",
				));
			} else if button.clicked() {
				self.tag = Some(TagDraft {
					index: None,
					tag: Tag {
						id: Id(0),
						name: String::new(),
						moderated: false,
						emoji_id: None,
						emoji_name: None,
					},
				});
			}
		}
	}

	fn tag_editor(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		forum: &mut ForumEdit,
		(images, demo): (&mut Avatars, bool),
	) {
		let Some(mut draft) = self.tag.take() else {
			return;
		};
		let mut keep = true;
		design::card(ui, |ui| {
			ui.set_width(ui.available_width());
			ui.spacing_mut().item_spacing.y = 8.0;
			ui.label(
				design::semibold(
					ui,
					if draft.index.is_some() {
						"Edit Tag"
					} else {
						"Create Tag"
					},
					15.0,
				)
				.color(design::palette(ui).text_strong),
			);
			let label = dialog::label(ui, "Tag name");
			dialog::input(
				ui,
				egui::TextEdit::singleline(&mut draft.tag.name)
					.hint_text(crate::i18n::translate("Question"))
					.char_limit(TAG_NAME_LIMIT),
			)
			.labelled_by(label.id);
			dialog::label(ui, "Emoji");
			ui.horizontal(|ui| {
				if let Some(choice) = self.chooser(
					ui,
					state,
					guild,
					images,
					"tag-emoji",
					if draft.tag.emoji_id.is_some() || draft.tag.emoji_name.is_some() {
						"Change emoji"
					} else {
						"Select emoji"
					},
				) {
					draft.tag.emoji_id = choice.0;
					draft.tag.emoji_name = choice.1;
				}
				if (draft.tag.emoji_id.is_some() || draft.tag.emoji_name.is_some())
					&& dialog::action(ui, "Remove emoji", dialog::Action::Neutral).clicked()
				{
					draft.tag.emoji_id = None;
					draft.tag.emoji_name = None;
				}
			});
			design::switch(
				ui,
				"Only allow moderators to apply this tag",
				Some("Members with Manage Threads can still use it."),
				&mut draft.tag.moderated,
			);
			if !draft.tag.name.trim().is_empty() {
				dialog::label(ui, "Preview");
				crate::forum::tag_pill(
					ui,
					(images, demo),
					&draft.tag,
					false,
					30.0,
					egui::Sense::hover(),
				);
			}
			ui.horizontal(|ui| {
				let valid = client_core::channel_actions::valid_tag_name(&draft.tag.name);
				if ui
					.add_enabled_ui(valid, |ui| {
						dialog::action(
							ui,
							if draft.index.is_some() {
								"Save Tag"
							} else {
								"Add Tag"
							},
							dialog::Action::Primary,
						)
					})
					.inner
					.clicked()
				{
					draft.tag.name = draft.tag.name.trim().to_owned();
					match draft.index {
						Some(index) if index < forum.tags.len() => {
							forum.tags[index] = draft.tag.clone()
						}
						_ if forum.tags.len() < MAX_TAGS => forum.tags.push(draft.tag.clone()),
						_ => {}
					}
					keep = false;
				}
				if dialog::action(ui, "Cancel", dialog::Action::Neutral).clicked() {
					keep = false;
				}
			});
		});
		if keep {
			self.tag = Some(draft);
		}
	}

	/// A button opening the server's emoji and the Unicode catalog; returns a picked emoji
	/// as `(custom id, name)`.
	fn chooser(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		images: &mut Avatars,
		salt: &str,
		label: &str,
	) -> Option<(Option<Id>, Option<String>)> {
		const CELL: f32 = 36.0;
		const COLUMNS: usize = 8;
		let button = ui
			.push_id(salt, |ui| {
				dialog::action(ui, label, dialog::Action::Outline)
			})
			.inner;
		if button.clicked() {
			self.query.clear();
		}
		let custom: Vec<&model::CustomEmoji> = state
			.guild(guild)
			.and_then(|guild| guild.emojis.as_deref())
			.unwrap_or_default()
			.iter()
			.filter(|emoji| emoji.available)
			.collect();
		let mut picked = None;
		egui::Popup::menu(&button)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.show(|ui| {
				ui.set_width(CELL * COLUMNS as f32 + 24.0);
				ui.add(
					egui::TextEdit::singleline(&mut self.query)
						.hint_text(crate::i18n::translate("Search emoji"))
						.char_limit(64)
						.desired_width(f32::INFINITY),
				);
				let query = self.query.trim().to_lowercase();
				if self.filtered.as_deref() != Some(query.as_str()) {
					let names = crate::emoji_picker::shortcodes();
					self.matches = (0..crate::emoji_picker::standard().len())
						.filter(|index| query.is_empty() || names[*index].contains(&query))
						.collect();
					self.filtered = Some(query.clone());
				}
				let mut choices: Vec<Choice> = custom
					.iter()
					.enumerate()
					.filter(|(_, emoji)| {
						query.is_empty() || emoji.name.to_lowercase().contains(&query)
					})
					.map(|(index, _)| Choice::Custom(index))
					.collect();
				let server = choices.len();
				choices.extend(self.matches.iter().map(|index| Choice::Unicode(*index)));
				if choices.is_empty() {
					ui.label(
						RichText::new(crate::i18n::translate("No emoji match"))
							.color(design::palette(ui).muted),
					);
					return;
				}
				if server > 0 {
					ui.label(design::eyebrow(
						ui,
						crate::i18n::translate("This server"),
						design::palette(ui).muted,
					));
				}
				egui::ScrollArea::vertical().max_height(280.0).show_rows(
					ui,
					CELL,
					choices.len().div_ceil(COLUMNS),
					|ui, rows| {
						for row in rows {
							ui.horizontal(|ui| {
								ui.spacing_mut().item_spacing.x = 0.0;
								for choice in choices.iter().skip(row * COLUMNS).take(COLUMNS) {
									let (id, name, hover) = match *choice {
										Choice::Custom(index) => {
											let emoji = custom[index];
											(
												Some(emoji.id),
												emoji.name.clone(),
												format!(":{}:", emoji.name),
											)
										}
										Choice::Unicode(index) => {
											let (text, _) = crate::emoji_picker::standard()[index];
											(
												None,
												text.to_owned(),
												crate::emoji_picker::shortcodes()[index].clone(),
											)
										}
									};
									let (rect, response) = ui.allocate_exact_size(
										egui::Vec2::splat(CELL),
										egui::Sense::click(),
									);
									if response.hovered() {
										ui.painter().rect_filled(
											rect,
											6,
											design::palette(ui).hover,
										);
									}
									if ui.is_rect_visible(rect) {
										crate::forum::paint_emoji(
											ui,
											(images, state.demo),
											(id, Some(&name)),
											rect.shrink(6.0),
										);
									}
									let response = response.on_hover_text(&hover);
									response.widget_info(|| {
										egui::WidgetInfo::labeled(egui::Role::Button, true, &hover)
									});
									if response.clicked() {
										picked = Some((id, Some(name)));
										ui.close();
									}
								}
							});
						}
					},
				);
				if server > 0 {
					ui.label(
						RichText::new(format!("{server} server emoji, then standard emoji"))
							.size(11.0)
							.color(design::palette(ui).muted),
					);
				}
			});
		picked
	}
}

/// A slowmode dropdown over Discord's steps, keeping an off-step value the service reported.
pub(crate) fn slowmode(ui: &mut egui::Ui, salt: &str, seconds: &mut u32) {
	let label = |value: u32| {
		SLOWMODE
			.iter()
			.find(|(step, _)| *step == value)
			.map_or_else(
				|| format!("{value} seconds"),
				|(_, label)| (*label).to_owned(),
			)
	};
	egui::ComboBox::from_id_salt(salt)
		.width(ui.available_width().min(560.0))
		.selected_text(label(*seconds))
		.show_ui(ui, |ui| {
			for (step, text) in SLOWMODE {
				ui.selectable_value(seconds, step, text);
			}
		});
}

fn select<T: Copy + PartialEq>(
	ui: &mut egui::Ui,
	salt: &str,
	value: &mut T,
	options: &[(T, &str)],
) {
	let current = options
		.iter()
		.find(|(option, _)| option == value)
		.map_or("", |(_, label)| label);
	egui::ComboBox::from_id_salt(salt)
		.width(ui.available_width().min(560.0))
		.selected_text(current)
		.show_ui(ui, |ui| {
			for (option, label) in options {
				ui.selectable_value(value, *option, *label);
			}
		});
}

/// A skeleton post card showing how the default reaction appears in the list.
fn reaction_preview(
	ui: &mut egui::Ui,
	reaction: Option<&ReactionEmoji>,
	images: (&mut Avatars, bool),
) {
	let colors = design::palette(ui);
	let (rect, _) = ui.allocate_exact_size(egui::vec2(260.0, 116.0), egui::Sense::hover());
	if !ui.is_rect_visible(rect) {
		return;
	}
	let painter = ui.painter();
	painter.rect(
		rect,
		10,
		colors.raised,
		egui::Stroke::new(1.0, colors.border),
		egui::StrokeKind::Inside,
	);
	let bar = |y: f32, width: f32| {
		egui::Rect::from_min_size(
			egui::pos2(rect.left() + 14.0, rect.top() + y),
			egui::vec2(width, 7.0),
		)
	};
	for (y, width) in [(16.0, 140.0), (30.0, 100.0), (50.0, 120.0), (64.0, 80.0)] {
		painter.rect_filled(bar(y, width), 4, colors.hover);
	}
	painter.rect_filled(
		egui::Rect::from_min_size(
			egui::pos2(rect.right() - 70.0, rect.top() + 14.0),
			egui::Vec2::splat(56.0),
		),
		8,
		colors.hover,
	);
	let chip_y = rect.bottom() - 26.0;
	match reaction {
		Some(emoji) => crate::forum::paint_emoji(
			ui,
			images,
			(emoji.id, emoji.name.as_deref()),
			egui::Rect::from_min_size(
				egui::pos2(rect.left() + 14.0, chip_y - 9.0),
				egui::Vec2::splat(18.0),
			),
		),
		None => icons::paint(
			ui.painter(),
			icons::Icon::Smile,
			egui::Rect::from_min_size(
				egui::pos2(rect.left() + 14.0, chip_y - 9.0),
				egui::Vec2::splat(18.0),
			),
			colors.muted,
		),
	}
	ui.painter().text(
		egui::pos2(rect.left() + 38.0, chip_y),
		egui::Align2::LEFT_CENTER,
		"17",
		egui::FontId::new(13.0, design::semibold_family(ui.ctx())),
		colors.text,
	);
}
