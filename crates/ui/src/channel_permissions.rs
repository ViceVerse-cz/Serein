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
				if design::switch(ui, if category { "Private Category" } else { "Private Channel" }, Some(if category {
					"Only selected members and roles can view this category. Synced channels follow its permissions."
				} else {
					"Only selected members and roles can view this channel. Administrators retain access."
				}), &mut private).changed() {
					set_permission(rows, (0, guild), p::VIEW_CHANNEL, if private { -1 } else { 1 });
				}
			});
		});
		if !state.can_manage_channel_permissions(channel.id) {
			dialog::hint(
				ui,
				"You need Manage Channels and Manage Permissions to change these settings.",
			);
		}
		design::divider(ui);
		egui::CollapsingHeader::new(design::semibold(ui, "Advanced permissions", 18.0))
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
		dialog::label(ui, "ROLES/MEMBERS");
		ui.add_enabled_ui(
			state.can_manage_channel_permissions(channel.id) && rows.len() < p::MAX_OVERWRITES,
			|ui| {
				ui.menu_button("+ Add role or member", |ui| {
					ui.set_width(240.0);
					ui.add(
						egui::TextEdit::singleline(&mut self.search)
							.hint_text("Search roles or loaded members")
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
									.selectable_label(false, format!("Role: {}", role.name))
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
										.selectable_label(false, format!("Member: {}", user.name))
										.clicked()
								{
									self.add(rows, (1, user.id));
									ui.close();
								}
							}
						});
					ui.separator();
					let label = dialog::label(ui, "Member ID");
					ui.add(
						egui::TextEdit::singleline(&mut self.member_id)
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
						.add_enabled(id.is_some(), egui::Button::new("Add Member"))
						.clicked()
					{
						self.add(rows, (1, Id(id.unwrap())));
						self.member_id.clear();
						ui.close();
					}
				});
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
					let color = if key.0 == 0 {
						state
							.permissions
							.guilds
							.get(&guild)
							.and_then(|g| g.roles.as_ref())
							.and_then(|roles| roles.iter().find(|r| r.id == key.1))
							.map(|r| {
								design::role_name_color(
									r.color,
									ui.visuals().extreme_bg_color,
									design::palette(ui).text,
								)
							})
					} else {
						None
					};
					let text = design::medium(ui, &label, 14.0)
						.color(color.unwrap_or(design::palette(ui).text));
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
		for (group, values) in [
			(
				if channel.kind == 4 {
					"General Category Permissions"
				} else {
					"General Channel Permissions"
				},
				&[
					(
						p::VIEW_CHANNEL,
						"View Channels",
						"Allows members to view these channels.",
					),
					(
						p::MANAGE_CHANNELS,
						"Manage Channels",
						"Allows members to edit channel settings and delete channels.",
					),
					(
						p::MANAGE_ROLES,
						"Manage Permissions",
						"Allows members to change channel permissions.",
					),
					(
						p::MANAGE_WEBHOOKS,
						"Manage Webhooks",
						"Allows members to create, edit, and delete webhooks.",
					),
				][..],
			),
			(
				"Membership Permissions",
				&[(
					1,
					"Create Invite",
					"Allows members to invite people to this server.",
				)][..],
			),
			(
				"Text Channel Permissions",
				&[
					(
						p::SEND_MESSAGES,
						"Send Messages",
						"Allows members to send messages in these channels.",
					),
					(
						p::SEND_MESSAGES_IN_THREADS,
						"Send Messages in Threads",
						"Allows members to reply in threads.",
					),
					(
						p::CREATE_PUBLIC_THREADS,
						"Create Public Threads",
						"Allows members to start public threads.",
					),
					(
						p::CREATE_PRIVATE_THREADS,
						"Create Private Threads",
						"Allows members to start private threads.",
					),
					(
						p::EMBED_LINKS,
						"Embed Links",
						"Shows previews for links members send.",
					),
					(
						p::ATTACH_FILES,
						"Attach Files",
						"Allows members to upload files and media.",
					),
					(
						p::ADD_REACTIONS,
						"Add Reactions",
						"Allows members to add new emoji reactions.",
					),
					(
						p::USE_EXTERNAL_EMOJIS,
						"Use External Emoji",
						"Allows emoji from other servers.",
					),
					(
						p::USE_EXTERNAL_STICKERS,
						"Use External Stickers",
						"Allows stickers from other servers.",
					),
					(
						p::MENTION_EVERYONE,
						"Mention @everyone, @here, and All Roles",
						"Allows mentions that notify everyone or entire roles.",
					),
					(
						p::MANAGE_MESSAGES,
						"Manage Messages",
						"Allows members to delete others' messages.",
					),
					(
						p::PIN_MESSAGES,
						"Pin Messages",
						"Allows members to pin and unpin messages.",
					),
					(
						p::MANAGE_THREADS,
						"Manage Threads",
						"Allows members to manage and delete threads.",
					),
					(
						p::READ_MESSAGE_HISTORY,
						"Read Message History",
						"Allows members to read previous messages.",
					),
					(
						p::SEND_TTS_MESSAGES,
						"Send Text-to-Speech Messages",
						"Allows messages read aloud with text-to-speech.",
					),
				][..],
			),
			(
				"Voice Channel Permissions",
				&[
					(
						p::CONNECT,
						"Connect",
						"Allows members to join voice channels.",
					),
					(
						p::SPEAK,
						"Speak",
						"Allows members to speak in voice channels.",
					),
					(
						p::STREAM,
						"Video",
						"Allows members to share video and their screen.",
					),
					(
						p::USE_VAD,
						"Use Voice Activity",
						"Allows speaking without push-to-talk.",
					),
					(
						p::MUTE_MEMBERS,
						"Mute Members",
						"Allows members to mute others in voice channels.",
					),
					(
						p::DEAFEN_MEMBERS,
						"Deafen Members",
						"Allows members to deafen others in voice channels.",
					),
					(
						p::MOVE_MEMBERS,
						"Move Members",
						"Allows members to move others between voice channels.",
					),
				][..],
			),
		] {
			if group == "Voice Channel Permissions" && !matches!(channel.kind, 2 | 4 | 13) {
				continue;
			}
			ui.add_space(12.0);
			design::section(ui, group, None);
			for &(bit, label, help) in values {
				ui.push_id(bit, |ui| {
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
					ui.horizontal_top(|ui| {
						let width = (ui.available_width() - 118.0).max(65.0);
						ui.allocate_ui_with_layout(
							egui::vec2(width, 0.0),
							egui::Layout::top_down(egui::Align::Min),
							|ui| {
								ui.set_width(width);
								ui.label(design::medium(ui, label, 15.0));
								dialog::hint(ui, help);
							},
						);
						ui.add_enabled_ui(enabled, |ui| {
							ui.spacing_mut().item_spacing.x = 0.0;
							for (choice, glyph, name, color) in [
								(-1, "×", "Deny", colors.danger),
								(0, "/", "Inherit", colors.muted),
								(1, "✓", "Allow", colors.positive),
							] {
								let response = ui.add(
									egui::Button::new(
										egui::RichText::new(glyph).size(20.0).color(color),
									)
									.selected(value == choice)
									.min_size(egui::vec2(34.0, 30.0))
									.corner_radius(3),
								);
								let accessible = format!("{name} {label}");
								response.widget_info(|| {
									egui::WidgetInfo::selected(
										egui::Role::RadioButton,
										ui.is_enabled(),
										value == choice,
										&accessible,
									)
								});
								if response.on_hover_text(accessible).clicked() {
									value = choice;
								}
							}
						});
					});
					if value != before {
						set_permission(rows, key, bit, value);
					}
					ui.add_space(8.0);
					ui.separator();
					ui.add_space(8.0);
				});
			}
		}
		if key != (0, guild) {
			let editable = rows
				.iter()
				.find(|o| (o.kind, o.id) == key)
				.is_some_and(|o| state.can_edit_channel_permission(channel.id, o.allow | o.deny));
			if ui
				.add_enabled(
					editable,
					egui::Button::new(
						egui::RichText::new("Remove Role / Member").color(colors.danger),
					),
				)
				.clicked()
			{
				rows.retain(|o| (o.kind, o.id) != key);
			}
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
