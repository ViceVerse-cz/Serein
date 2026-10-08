//! Channel and category overwrites share one bounded, unsaved settings draft.
use crate::{design, dialog};
use client_core::State;
use model::{Channel, Id, User, permissions as p};

#[derive(Default)]
pub(super) struct PermissionsUi {
	selected: Option<(u8, Id)>,
	search: String,
	member_id: String,
}

impl PermissionsUi {
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: &Channel,
		rows: &mut Vec<p::Overwrite>,
	) {
		let Some(guild) = channel.guild else { return };
		let category = channel.kind == 4;
		let colors = design::palette(ui);
		let mut private = rows
			.iter()
			.any(|o| o.kind == 0 && o.id == guild && o.deny & p::VIEW_CHANNEL != 0);
		egui::Frame::new().fill(colors.raised).stroke(egui::Stroke::new(1.0, colors.border)).corner_radius(12).inner_margin(16).show(ui, |ui| {
			ui.add_enabled_ui(state.can_edit_channel_permission(channel.id, p::VIEW_CHANNEL) && (rows.len() < p::MAX_OVERWRITES || rows.iter().any(|o| o.kind == 0 && o.id == guild)), |ui| {
				if design::switch(ui, if category { "channel-permissions-show-private-category" } else { "channel-permissions-show-private-channel" }, Some(if category {
					"channel-permissions-show-only-selected-members-and-roles-can-view-this-category-synced"
				} else {
					"channel-permissions-show-only-selected-members-and-roles-can-view-this-channel-administrators"
				}), &mut private).changed() {
					set_permission(rows, (0, guild), p::VIEW_CHANNEL, if private { -1 } else { 1 });
				}
			});
		});
		if !state.can_manage_channel_permissions(channel.id) {
			dialog::hint(
				ui,
				"channel-permissions-show-you-need-manage-channels-and-manage-permissions-to-change-these",
			);
		}
		design::divider(ui);
		egui::CollapsingHeader::new(design::semibold(
			ui,
			crate::i18n::translate("channel-permissions-show-advanced-permissions"),
			18.0,
		))
		.default_open(true)
		.show(ui, |ui| {
			let selected = self.selected.get_or_insert((0, guild));
			if *selected != (0, guild) && !rows.iter().any(|o| (o.kind, o.id) == *selected) {
				*selected = (0, guild);
			}
			if ui.available_width() >= 580.0 {
				ui.horizontal_top(|ui| {
					ui.allocate_ui_with_layout(
						egui::vec2(180.0, 0.0),
						egui::Layout::top_down(egui::Align::Min),
						|ui| {
							ui.set_width(180.0);
							self.targets(ui, state, channel, rows);
						},
					);
					ui.add_space(16.0);
					ui.vertical(|ui| {
						self.permissions(ui, state, channel, rows);
					});
				});
			} else {
				self.targets(ui, state, channel, rows);
				ui.separator();
				self.permissions(ui, state, channel, rows);
			}
		});
	}

	fn targets(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		channel: &Channel,
		rows: &mut Vec<p::Overwrite>,
	) {
		let guild = channel.guild.unwrap();
		dialog::label(ui, "channel-permissions-targets-roles-members");
		ui.add_enabled_ui(
			state.can_manage_channel_permissions(channel.id) && rows.len() < p::MAX_OVERWRITES,
			|ui| {
				ui.menu_button(
					crate::i18n::translate("channel-permissions-targets-add-role-or-member"),
					|ui| {
						ui.set_width(240.0);
						ui.add(
							egui::TextEdit::singleline(&mut self.search)
								.align(egui::Align2::LEFT_CENTER)
								.hint_text(crate::i18n::translate(
									"channel-permissions-targets-search-roles-or-loaded-members",
								))
								.char_limit(64)
								.desired_width(f32::INFINITY),
						);
						let query = self.search.to_lowercase();
						let roles = state
							.permissions
							.guilds
							.get(&guild)
							.and_then(|g| g.roles.as_deref())
							.unwrap_or_default();
						egui::ScrollArea::vertical()
							.max_height(210.0)
							.show(ui, |ui| {
								for role in roles
									.iter()
									.filter(|r| r.name.to_lowercase().contains(&query))
								{
									if ui
										.selectable_label(
											false,
											crate::role_names::galley(
												ui,
												&format!(
													"{}: {}",
													crate::i18n::translate(
														"channel-permissions-targets-role"
													),
													role.name
												),
												egui::FontId::proportional(14.0),
												Some(role.colors()),
												ui.visuals().extreme_bg_color,
												design::palette(ui).text,
												ui.available_width(),
											),
										)
										.clicked()
									{
										self.add(rows, (0, role.id));
										ui.close();
									}
								}
								// ponytail: suggestions use the loaded member window; add service search when broader discovery is needed.
								for user in state
									.members
									.iter()
									.filter(|m| m.guild == Some(guild))
									.flat_map(|m| {
										m.slots.iter().flatten().filter_map(|slot| match slot {
											model::MemberSlot::Person(m) => Some(m),
											_ => None,
										})
									})
									.map(|m| &m.user)
									.chain(state.user.iter())
								{
									if user.name.to_lowercase().contains(&query)
										&& ui
											.selectable_label(
												false,
												format!(
													"{}: {}",
													crate::i18n::translate(
														"channel-permissions-targets-member"
													),
													user.name
												),
											)
											.clicked()
									{
										self.add(rows, (1, user.id));
										ui.close();
									}
								}
							});
						ui.separator();
						let label = dialog::label(ui, "channel-permissions-targets-member-id");
						ui.add(
							egui::TextEdit::singleline(&mut self.member_id)
								.align(egui::Align2::LEFT_CENTER)
								.char_limit(20)
								.desired_width(f32::INFINITY),
						)
						.labelled_by(label.id);
						let id = self.member_id.parse::<u64>().ok().filter(|id| {
							*id != 0
								&& Id(*id) != guild && !roles.iter().any(|r| r.id == Id(*id))
								&& !rows.iter().any(|o| o.id == Id(*id) && o.kind != 1)
						});
						if ui
							.add_enabled(
								id.is_some(),
								egui::Button::new(crate::i18n::translate(
									"channel-permissions-targets-add-member",
								)),
							)
							.clicked()
						{
							self.add(rows, (1, Id(id.unwrap())));
							self.member_id.clear();
							ui.close();
						}
					},
				);
			},
		);
		egui::ScrollArea::vertical()
			.id_salt("overwrite-targets")
			.max_height(260.0)
			.show(ui, |ui| {
				for key in std::iter::once((0, guild)).chain(
					rows.iter()
						.map(|o| (o.kind, o.id))
						.filter(|key| *key != (0, guild)),
				) {
					let label = target_name(state, guild, key);
					let role_colors = if key.0 == 0 {
						state
							.permissions
							.guilds
							.get(&guild)
							.and_then(|g| g.roles.as_ref())
							.and_then(|roles| roles.iter().find(|role| role.id == key.1))
							.map(|role| role.colors())
					} else {
						None
					};
					let palette = design::palette(ui);
					let text = crate::role_names::galley(
						ui,
						&label,
						egui::FontId::new(14.0, design::medium_family(ui.ctx())),
						role_colors,
						ui.visuals().extreme_bg_color,
						palette.text,
						(ui.available_width() - 16.0).max(0.0),
					);
					if ui
						.add(
							egui::Button::new(())
								.left_text(text)
								.selected(self.selected == Some(key))
								.frame_when_inactive(false)
								.min_size(egui::vec2(ui.available_width(), 32.0)),
						)
						.on_hover_text(&label)
						.clicked()
					{
						self.selected = Some(key);
					}
				}
			});
	}

	fn add(&mut self, rows: &mut Vec<p::Overwrite>, key: (u8, Id)) {
		if !rows.iter().any(|o| (o.kind, o.id) == key) && rows.len() < p::MAX_OVERWRITES {
			rows.reserve_exact(1);
			rows.push(p::Overwrite {
				id: key.1,
				kind: key.0,
				allow: 0,
				deny: 0,
			});
		}
		self.selected = Some(key);
	}

	fn permissions(
		&self,
		ui: &mut egui::Ui,
		state: &State,
		channel: &Channel,
		rows: &mut Vec<p::Overwrite>,
	) {
		let guild = channel.guild.unwrap();
		let key = self.selected.unwrap_or((0, guild));
		let can_add = rows.len() < p::MAX_OVERWRITES || rows.iter().any(|o| (o.kind, o.id) == key);
		let colors = design::palette(ui);
		let row_width = ui.available_width();
		// Rows set their own rhythm; the dialog's item spacing doubled every gap.
		ui.spacing_mut().item_spacing.y = 0.0;
		for (group, values) in [
			(
				if channel.kind == 4 {
					"channel-permissions-add-general-category-permissions"
				} else {
					"channel-permissions-add-general-channel-permissions"
				},
				&[
					(
						p::VIEW_CHANNEL,
						"channel-permissions-add-view-channels",
						"channel-permissions-add-allows-members-to-view-these-channels",
					),
					(
						p::MANAGE_CHANNELS,
						"channel-permissions-add-manage-channels",
						"channel-permissions-add-allows-members-to-edit-channel-settings-and-delete-channels",
					),
					(
						p::MANAGE_ROLES,
						"channel-permissions-add-manage-permissions",
						"channel-permissions-add-allows-members-to-change-channel-permissions",
					),
					(
						p::MANAGE_WEBHOOKS,
						"channel-permissions-add-manage-webhooks",
						"channel-permissions-add-allows-members-to-create-edit-and-delete-webhooks",
					),
				][..],
			),
			(
				"channel-permissions-add-membership-permissions",
				&[(
					1,
					"channel-permissions-add-create-invite",
					"channel-permissions-add-allows-members-to-invite-people-to-this-server",
				)][..],
			),
			(
				"channel-permissions-add-text-channel-permissions",
				&[
					(
						p::SEND_MESSAGES,
						"channel-permissions-add-send-messages",
						"channel-permissions-add-allows-members-to-send-messages-in-these-channels",
					),
					(
						p::SEND_MESSAGES_IN_THREADS,
						"channel-permissions-add-send-messages-in-threads",
						"channel-permissions-add-allows-members-to-reply-in-threads",
					),
					(
						p::CREATE_PUBLIC_THREADS,
						"channel-permissions-add-create-public-threads",
						"channel-permissions-add-allows-members-to-start-public-threads",
					),
					(
						p::CREATE_PRIVATE_THREADS,
						"channel-permissions-add-create-private-threads",
						"channel-permissions-add-allows-members-to-start-private-threads",
					),
					(
						p::EMBED_LINKS,
						"channel-permissions-add-embed-links",
						"channel-permissions-add-shows-previews-for-links-members-send",
					),
					(
						p::ATTACH_FILES,
						"channel-permissions-add-attach-files",
						"channel-permissions-add-allows-members-to-upload-files-and-media",
					),
					(
						p::ADD_REACTIONS,
						"channel-permissions-add-add-reactions",
						"channel-permissions-add-allows-members-to-add-new-emoji-reactions",
					),
					(
						p::USE_EXTERNAL_EMOJIS,
						"channel-permissions-add-use-external-emoji",
						"channel-permissions-add-allows-emoji-from-other-servers",
					),
					(
						p::USE_EXTERNAL_STICKERS,
						"channel-permissions-add-use-external-stickers",
						"channel-permissions-add-allows-stickers-from-other-servers",
					),
					(
						p::MENTION_EVERYONE,
						"channel-permissions-add-mention-everyone-here-and-all-roles",
						"channel-permissions-add-allows-mentions-that-notify-everyone-or-entire-roles",
					),
					(
						p::MANAGE_MESSAGES,
						"channel-permissions-add-manage-messages",
						"channel-permissions-add-allows-members-to-delete-others-messages",
					),
					(
						p::PIN_MESSAGES,
						"channel-permissions-add-pin-messages",
						"channel-permissions-add-allows-members-to-pin-and-unpin-messages",
					),
					(
						p::MANAGE_THREADS,
						"channel-permissions-add-manage-threads",
						"channel-permissions-add-allows-members-to-manage-and-delete-threads",
					),
					(
						p::READ_MESSAGE_HISTORY,
						"channel-permissions-add-read-message-history",
						"channel-permissions-add-allows-members-to-read-previous-messages",
					),
					(
						p::SEND_TTS_MESSAGES,
						"channel-permissions-add-send-text-to-speech-messages",
						"channel-permissions-add-allows-messages-read-aloud-with-text-to-speech",
					),
				][..],
			),
			(
				"channel-permissions-add-voice-channel-permissions",
				&[
					(
						p::CONNECT,
						"channel-permissions-add-connect",
						"channel-permissions-add-allows-members-to-join-voice-channels",
					),
					(
						p::SPEAK,
						"channel-permissions-add-speak",
						"channel-permissions-add-allows-members-to-speak-in-voice-channels",
					),
					(
						p::STREAM,
						"channel-permissions-add-video",
						"channel-permissions-add-allows-members-to-share-video-and-their-screen",
					),
					(
						p::USE_VAD,
						"channel-permissions-add-use-voice-activity",
						"channel-permissions-add-allows-speaking-without-push-to-talk",
					),
					(
						p::MUTE_MEMBERS,
						"channel-permissions-add-mute-members",
						"channel-permissions-add-allows-members-to-mute-others-in-voice-channels",
					),
					(
						p::DEAFEN_MEMBERS,
						"channel-permissions-add-deafen-members",
						"channel-permissions-add-allows-members-to-deafen-others-in-voice-channels",
					),
					(
						p::MOVE_MEMBERS,
						"channel-permissions-add-move-members",
						"channel-permissions-add-allows-members-to-move-others-between-voice-channels",
					),
				][..],
			),
		] {
			if group == "channel-permissions-add-voice-channel-permissions"
				&& !matches!(channel.kind, 2 | 4 | 13)
			{
				continue;
			}
			ui.add_space(20.0);
			design::section(ui, group, None);
			ui.add_space(6.0);
			for &(bit, label, help) in values {
				ui.push_id(bit, |ui| {
					let label = crate::i18n::translate_if_key(label);
					let overwrite = rows.iter().find(|o| (o.kind, o.id) == key);
					let mut value = overwrite.map_or(0, |o| {
						if o.deny & bit != 0 {
							-1
						} else if o.allow & bit != 0 {
							1
						} else {
							0
						}
					});
					let before = value;
					let enabled = can_add && state.can_edit_channel_permission(channel.id, bit);
					// Every row splits the same measured width into a text column and a
					// fixed toggle column, so the toggles share one right edge. Sizing the
					// text from the row's own available width let a few pixels of toggle
					// overflow widen the column, shifting each following row further right.
					let text_width = (row_width - TRI_STATE_WIDTH - 16.0).max(65.0);
					ui.horizontal_top(|ui| {
						ui.spacing_mut().item_spacing.x = 0.0;
						let text = ui.allocate_ui_with_layout(
							egui::vec2(text_width, 0.0),
							egui::Layout::top_down(egui::Align::Min),
							|ui| {
								ui.set_width(text_width);
								ui.spacing_mut().item_spacing.y = 2.0;
								ui.add(
									egui::Label::new(
										design::medium(ui, &label, 15.0).color(colors.text_strong),
									)
									.wrap(),
								);
								ui.add(
									egui::Label::new(
										egui::RichText::new(crate::i18n::translate_if_key(help))
											.size(13.0)
											.color(colors.muted),
									)
									.wrap(),
								);
							},
						);
						ui.add_space((row_width - text_width - TRI_STATE_WIDTH).max(0.0));
						let offset = ((text.response.rect.height() - TRI_STATE_CELL.y) / 2.0)
							.clamp(0.0, 8.0);
						ui.vertical(|ui| {
							ui.add_space(offset);
							ui.add_enabled_ui(enabled, |ui| tri_state(ui, &mut value, &label));
						});
					});
					if value != before {
						set_permission(rows, key, bit, value);
					}
					ui.add_space(6.0);
					design::card_divider(ui);
					ui.add_space(6.0);
				});
			}
		}
		if key != (0, guild) {
			ui.add_space(12.0);
			let editable = rows
				.iter()
				.find(|o| (o.kind, o.id) == key)
				.is_some_and(|o| state.can_edit_channel_permission(channel.id, o.allow | o.deny));
			if ui
				.add_enabled(
					editable,
					egui::Button::new(
						egui::RichText::new(crate::i18n::translate(
							"channel-permissions-permissions-remove-role-member",
						))
						.color(colors.danger),
					),
				)
				.clicked()
			{
				rows.retain(|o| (o.kind, o.id) != key);
			}
		}
	}
}

/// One cell of the deny / inherit / allow control.
const TRI_STATE_CELL: egui::Vec2 = egui::vec2(36.0, 30.0);
/// The whole control has a fixed width so every permission row lines it up on one edge.
const TRI_STATE_WIDTH: f32 = TRI_STATE_CELL.x * 3.0;

/// Discord-style connected deny / inherit / allow switch. The size never depends on the
/// glyphs' font metrics, so rows cannot drift apart.
fn tri_state(ui: &mut egui::Ui, value: &mut i8, label: &str) {
	let colors = design::palette(ui);
	let (group, _) = ui.allocate_exact_size(
		egui::vec2(TRI_STATE_WIDTH, TRI_STATE_CELL.y),
		egui::Sense::hover(),
	);
	let enabled = ui.is_enabled();
	let fade = |color: egui::Color32| {
		if enabled {
			color
		} else {
			color.gamma_multiply(0.45)
		}
	};
	ui.painter().rect(
		group,
		6,
		colors.base,
		egui::Stroke::new(1.0, colors.border),
		egui::StrokeKind::Inside,
	);
	for (index, (choice, glyph, name, tint)) in [
		(-1, "×", "channel-permissions-add-deny", colors.danger),
		(0, "/", "channel-permissions-add-inherit", colors.muted),
		(1, "✓", "channel-permissions-add-allow", colors.positive),
	]
	.into_iter()
	.enumerate()
	{
		let cell = egui::Rect::from_min_size(
			group.min + egui::vec2(TRI_STATE_CELL.x * index as f32, 0.0),
			TRI_STATE_CELL,
		);
		let response = ui.interact(cell, ui.scope_id().with(choice), egui::Sense::click());
		let selected = *value == choice;
		let accessible = format!("{} {label}", crate::i18n::translate_if_key(name));
		response.widget_info(|| {
			egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, &accessible)
		});
		let radius = egui::CornerRadius {
			nw: if index == 0 { 6 } else { 0 },
			sw: if index == 0 { 6 } else { 0 },
			ne: if index == 2 { 6 } else { 0 },
			se: if index == 2 { 6 } else { 0 },
		};
		let fill = if selected {
			fade(tint)
		} else if response.hovered() || response.has_focus() {
			colors.hover
		} else {
			egui::Color32::TRANSPARENT
		};
		ui.painter().rect_filled(cell.shrink(1.0), radius, fill);
		if response.has_focus() {
			ui.painter().rect_stroke(
				cell,
				radius,
				egui::Stroke::new(2.0, colors.accent),
				egui::StrokeKind::Inside,
			);
		}
		let color = if selected {
			egui::Color32::WHITE
		} else {
			fade(tint)
		};
		ui.painter().text(
			cell.center(),
			egui::Align2::CENTER_CENTER,
			glyph,
			egui::FontId::proportional(18.0),
			color,
		);
		if response.on_hover_text(accessible).clicked() {
			*value = choice;
		}
	}
}

fn target_name(state: &State, guild: Id, key: (u8, Id)) -> String {
	if key == (0, guild) {
		return "@everyone".into();
	}
	if key.0 == 0 {
		return state
			.permissions
			.guilds
			.get(&guild)
			.and_then(|g| g.roles.as_ref())
			.and_then(|roles| roles.iter().find(|r| r.id == key.1))
			.map_or_else(|| format!("Role {}", key.1), |r| r.name.clone());
	}
	let user: Option<&User> = state
		.members
		.iter()
		.filter(|m| m.guild == Some(guild))
		.flat_map(|m| {
			m.slots.iter().flatten().filter_map(|slot| match slot {
				model::MemberSlot::Person(m) => Some(m),
				_ => None,
			})
		})
		.map(|m| &m.user)
		.chain(state.user.iter())
		.find(|u| u.id == key.1);
	user.map_or_else(|| format!("Member {}", key.1), |u| u.name.clone())
}

fn set_permission(rows: &mut Vec<p::Overwrite>, key: (u8, Id), bit: u128, value: i8) {
	if !rows.iter().any(|o| (o.kind, o.id) == key) {
		if rows.len() >= p::MAX_OVERWRITES {
			return;
		}
		rows.reserve_exact(1);
		rows.push(p::Overwrite {
			id: key.1,
			kind: key.0,
			allow: 0,
			deny: 0,
		});
	}
	let row = rows.iter_mut().find(|o| (o.kind, o.id) == key).unwrap();
	row.allow = (row.allow & !bit) | if value == 1 { bit } else { 0 };
	row.deny = (row.deny & !bit) | if value == -1 { bit } else { 0 };
}
