//! On-demand emoji and member administration inside the shared server settings shell.
use crate::{avatars::Avatars, design, icons, user_menu};
use client_core::{Command, State};
use egui::RichText;
use model::{
	Id,
	server_admin::{Action, Member, Query},
};

struct Upload {
	name: String,
	image: String,
	animated: bool,
	texture: egui::TextureHandle,
}
enum Dialog {
	Rename { id: Id, name: String },
	Delete { id: Id, name: String },
	Nickname { user: Id, name: String },
	Kick { user: Id, name: String },
	Prune { days: u8, counted: Option<u8> },
}
#[derive(Default)]
pub(super) struct Admin {
	pub request: u64,
	files: Option<Vec<std::path::PathBuf>>,
	preparing: bool,
	uploads: Vec<Upload>,
	uploading: bool,
	submitted_upload: bool,
	error: Option<&'static str>,
	dialog: Option<Dialog>,
	dialog_submitted: bool,
	query: Query,
	query_initialized: bool,
	query_changed: Option<f64>,
	pub user_action: Option<user_menu::Action>,
	pub message: Option<Id>,
}
impl Admin {
	pub fn preparing(&self) -> bool {
		self.preparing || self.uploading
	}
	pub fn has_changes(&self) -> bool {
		self.preparing || !self.uploads.is_empty() || self.dialog_submitted
	}
	pub fn queue_files(&mut self, paths: Vec<std::path::PathBuf>) {
		if self.preparing() {
			self.error =
				Some("Wait for the current emoji images to finish preparing or uploading.");
			return;
		}
		if paths.len() > 10
			|| paths.iter().any(|path| path.as_os_str().len() > 32768)
			|| !self.uploads.is_empty()
		{
			self.error =
				Some("Finish the current uploads first. Choose up to 10 images at a time.");
			return;
		}
		self.files = Some(paths);
		self.preparing = true;
		self.error = None;
	}
	pub fn take_files(&mut self) -> Option<Vec<std::path::PathBuf>> {
		self.files.take()
	}
	pub fn navigation_error(&mut self) {
		self.error =
			Some("Save or discard your server settings changes before opening the conversation.");
	}
	pub fn accept_files(
		&mut self,
		ctx: &egui::Context,
		result: Result<Vec<(String, String, bool, egui::ColorImage)>, &'static str>,
	) {
		if !self.preparing {
			return;
		}
		self.preparing = false;
		match result {
			Err(error) => self.error = Some(error),
			Ok(files)
				if files.len() <= 10
					&& files.iter().all(|(name, data, _, image)| {
						name.len() <= 128
							&& data.len() <= model::server_admin::MAX_IMAGE_URI
							&& (1..=128).contains(&image.size[0])
							&& (1..=128).contains(&image.size[1])
							&& image.pixels.len() == image.size[0] * image.size[1]
					}) =>
			{
				self.uploads = files
					.into_iter()
					.enumerate()
					.map(|(i, (name, image, animated, preview))| Upload {
						name,
						image,
						animated,
						texture: ctx.load_texture(
							format!("server-emoji-upload-{i}"),
							preview,
							egui::TextureOptions::LINEAR,
						),
					})
					.collect();
			}
			Ok(_) => self.error = Some("The selected images exceed the emoji upload limits."),
		}
	}
	pub fn load(&mut self, state: &mut State, guild: Id, members: bool) -> Option<Command> {
		if members && !self.query_initialized {
			if state.server_admin.guild == Some(guild) {
				self.query.clone_from(&state.server_admin.query);
			}
			self.query_initialized = true;
		}
		if state.server_admin.pending
			|| (state.server_admin.guild == Some(guild)
				&& (state.server_admin.error.is_some()
					|| if members {
						state.server_admin.members.is_some()
					} else {
						state.server_admin.emojis.is_some()
					})) {
			return None;
		}
		state.request_server_admin(
			guild,
			if members {
				Action::LoadMembers(self.query.clone())
			} else {
				Action::LoadEmojis
			},
		)
	}
	#[allow(clippy::too_many_arguments)]
	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		members: bool,
		avatars: &mut Avatars,
		profile: &mut crate::profiles::ProfileSession,
		commands: &mut Vec<Command>,
	) {
		ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
		if self.submitted_upload && !state.server_admin.pending {
			self.submitted_upload = false;
			if state.server_admin.error.is_none() {
				if !self.uploads.is_empty() {
					self.uploads.remove(0);
				}
			} else {
				self.uploading = false;
			}
		}
		if self.dialog_submitted && !state.server_admin.pending {
			self.dialog_submitted = false;
			if state.server_admin.error.is_none()
				&& !matches!(self.dialog, Some(Dialog::Prune { .. }))
			{
				self.dialog = None;
			}
		}
		if !state.can_create_guild_emoji(guild) {
			self.uploads.clear();
			self.uploading = false;
			self.preparing = false;
			self.files = None;
		}
		if self.uploading && !state.server_admin.pending {
			if let Some(upload) = self.uploads.first() {
				if let Some(command) = state.request_server_admin(
					guild,
					Action::CreateEmoji {
						name: upload.name.clone(),
						image: upload.image.clone(),
					},
				) {
					self.submitted_upload = true;
					commands.push(command);
				} else {
					self.uploading = false;
					self.error = Some(
						"This emoji cannot be uploaded. Check its name, available slots, and your permissions.",
					);
				}
			} else {
				self.uploading = false;
			}
		}
		if let Some(error) = state.server_admin.error.or(self.error) {
			design::notice(ui, design::Level::Error, error);
			if ui
				.add_enabled(
					!state.server_admin.pending,
					egui::Button::new(crate::i18n::translate("server-admin-show-reload")),
				)
				.clicked() && let Some(command) = state.request_server_admin(
				guild,
				if members {
					Action::LoadMembers(self.query.clone())
				} else {
					Action::LoadEmojis
				},
			) {
				commands.push(command);
				self.error = None;
			}
		}
		if state.server_admin.pending {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.weak(crate::i18n::translate_if_key(
					if state.server_admin.saving {
						"server-admin-show-saving-changes"
					} else {
						"server-admin-show-loading"
					},
				));
			});
		}
		ui.add_enabled_ui(!state.server_admin.needs_refresh, |ui| {
			if members {
				self.members(ui, state, guild, avatars, profile, commands);
			} else {
				self.emojis(ui, state, guild, avatars, profile);
			}
		});
		self.dialog(ui.ctx(), state, guild, commands);
	}
	fn emojis(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		profile: &mut crate::profiles::ProfileSession,
	) {
		let colors = design::palette(ui);
		ui.label(design::semibold(
			ui,
			crate::i18n::translate("server-admin-emojis-emoji"),
			22.0,
		));
		ui.label(crate::i18n::translate(
			"server-admin-emojis-add-custom-emoji-that-anyone-can-use-in-this-server",
		));
		ui.add_space(14.0);
		if state.can_create_guild_emoji(guild) {
			if ui
				.add_enabled_ui(
					!self.preparing() && self.uploads.is_empty() && !state.server_admin.pending,
					|ui| {
						design::button(
							ui,
							&crate::i18n::translate("server-admin-emojis-upload-emoji"),
							design::ButtonKind::Primary,
						)
					},
				)
				.inner
				.clicked()
			{
				self.queue_files(Vec::new());
			}
			ui.add_space(18.0);
			ui.small(crate::i18n::translate(
				"server-admin-emojis-drag-and-drop-up-to-10-images-onto-this-page",
			));
		}
		if self.preparing {
			ui.weak(crate::i18n::translate(
				"server-admin-emojis-preparing-emoji-images",
			));
		}
		if !self.uploads.is_empty() {
			egui::Frame::new()
				.stroke(egui::Stroke::new(1.0, colors.border))
				.corner_radius(8)
				.inner_margin(12)
				.show(ui, |ui| {
					ui.label(design::semibold(
						ui,
						crate::i18n::translate("server-admin-emojis-review-uploads"),
						16.0,
					));
					let mut remove = None;
					for (index, upload) in self.uploads.iter_mut().enumerate() {
						ui.push_id(index, |ui| {
							ui.horizontal(|ui| {
								ui.add(
									egui::Image::from_texture(&upload.texture)
										.fit_to_exact_size(egui::Vec2::splat(32.0)),
								);
								ui.add_enabled(
									!self.uploading,
									egui::TextEdit::singleline(&mut upload.name)
										.desired_width((ui.available_width() - 130.0).max(60.0))
										.char_limit(32),
								)
								.on_hover_text(crate::i18n::translate(
									"server-admin-emojis-emoji-name-232-letters-numbers-or-underscores",
								));
								ui.weak(crate::i18n::translate_if_key(if upload.animated {
									"server-admin-emojis-animated"
								} else {
									"server-admin-emojis-static"
								}));
								if ui
									.add_enabled(
										!self.uploading,
										egui::Button::new(crate::i18n::translate(
											"server-admin-emojis-remove",
										)),
									)
									.clicked()
								{
									remove = Some(index);
								}
							});
						});
					}
					if let Some(index) = remove {
						self.uploads.remove(index);
					}
					let valid = self
						.uploads
						.iter()
						.all(|upload| model::server_admin::valid_emoji_name(&upload.name));
					if !valid {
						design::notice(
							ui,
							design::Level::Error,
							&crate::i18n::translate(
								"server-admin-emojis-emoji-names-must-use-232-letters-numbers-or-underscores",
							),
						);
					}
					ui.horizontal(|ui| {
						if ui
							.add_enabled(
								valid && !self.uploading && !state.server_admin.pending,
								egui::Button::new(crate::i18n::translate(
									"server-admin-emojis-upload",
								)),
							)
							.clicked()
						{
							self.uploading = true;
						}
						if ui
							.add_enabled(
								!self.uploading,
								egui::Button::new(crate::i18n::translate(
									"server-admin-emojis-cancel",
								)),
							)
							.clicked()
						{
							self.uploads.clear();
						}
					});
				});
		}
		ui.add_space(24.0);
		ui.separator();
		ui.add_space(24.0);
		let Some(catalog) = state.server_admin.emojis.as_ref() else {
			return;
		};
		for animated in [false, true] {
			ui.label(design::semibold(
				ui,
				crate::i18n::translate_if_key(if animated {
					"server-admin-emojis-animated-emoji"
				} else {
					"server-admin-emojis-emoji"
				}),
				21.0,
			));
			let count = catalog
				.items
				.iter()
				.filter(|row| row.emoji.animated == animated)
				.count();
			let limit = if animated {
				catalog.animated_limit
			} else {
				catalog.static_limit
			};
			ui.label(limit.map_or_else(
				|| {
					format!(
						"{count} {}",
						crate::i18n::translate("server-admin-emojis-emoji-2")
					)
				},
				|limit| {
					format!(
						"{} {}",
						limit.saturating_sub(count),
						crate::i18n::translate("server-admin-emojis-slots-available")
					)
				},
			));
			ui.add_space(12.0);
			if count == 0 {
				ui.add_space(8.0);
				ui.vertical_centered(|ui| {
					ui.label(
						RichText::new(crate::i18n::translate("server-admin-emojis-none"))
							.size(18.0)
							.color(colors.muted),
					);
				});
			} else {
				egui::Frame::new()
					.stroke(egui::Stroke::new(1.0, colors.border))
					.corner_radius(12)
					.inner_margin(12)
					.show(ui, |ui| {
						let width = ui.available_width();
						let name_width = ((width - 116.0) * 0.47).max(60.0);
						let by_width = (width - name_width - 116.0).max(40.0);
						ui.horizontal(|ui| {
							cell_text(ui, "server-admin-emojis-image", 44.0, true);
							cell_text(ui, "server-admin-emojis-name", name_width, true);
							cell_text(ui, "server-admin-emojis-uploaded-by", by_width, true);
						});
						ui.separator();
						for row in catalog
							.items
							.iter()
							.filter(|row| row.emoji.animated == animated)
						{
							ui.push_id(row.emoji.id, |ui| {
								ui.horizontal(|ui| {
									ui.allocate_ui_with_layout(
										egui::vec2(44.0, 44.0),
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											if let Some(image) = avatars.custom_image(
												ui.ctx(),
												row.emoji.id,
												36.0,
												state.demo,
											) {
												ui.add(image);
											}
										},
									);
									cell_text(
										ui,
										&format!(":{}:", row.emoji.name),
										name_width,
										false,
									);
									ui.allocate_ui_with_layout(
										egui::vec2(by_width, 44.0),
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											ui.set_max_width(by_width);
											if let Some(user) = &row.uploader {
												let avatar =
													avatars.show(ui, user, 24.0, state.demo);
												profile.person_click(ui, &avatar, None, user);
												ui.add(egui::Label::new(&user.name).truncate());
											} else {
												ui.weak(crate::i18n::translate(
													"server-admin-emojis-unknown",
												));
											}
										},
									);
									if state.can_edit_guild_emoji(guild, row.emoji.id) {
										let button = icons::button(
											ui,
											icons::Icon::More,
											24.0,
											&crate::i18n::translate(
												"server-admin-emojis-emoji-actions",
											),
										);
										egui::Popup::menu(&button).show(|ui| {
											if ui
												.button(crate::i18n::translate(
													"server-admin-emojis-rename",
												))
												.clicked()
											{
												self.dialog = Some(Dialog::Rename {
													id: row.emoji.id,
													name: row.emoji.name.clone(),
												});
												ui.close();
											}
											if ui
												.button(
													RichText::new(crate::i18n::translate(
														"server-admin-emojis-delete-emoji",
													))
													.color(colors.danger),
												)
												.clicked()
											{
												self.dialog = Some(Dialog::Delete {
													id: row.emoji.id,
													name: row.emoji.name.clone(),
												});
												ui.close();
											}
										});
									}
								});
							});
						}
					});
			}
			ui.add_space(36.0);
		}
	}
	fn members(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		profile: &mut crate::profiles::ProfileSession,
		commands: &mut Vec<Command>,
	) {
		let colors = design::palette(ui);
		ui.label(design::semibold(
			ui,
			crate::i18n::translate("server-admin-members-server-members"),
			22.0,
		));
		ui.add_space(20.0);
		let mut action = None;
		if let Some(mut enabled) = state
			.server_admin
			.members
			.as_ref()
			.and_then(|members| members.show_in_channel_list)
			&& state.can_show_members_in_channel_list(guild)
		{
			ui.add_enabled_ui(!state.server_admin.pending, |ui| {
				if design::switch(
					ui,
					"server-admin-members-show-members-in-channel-list",
					Some(
						"server-admin-members-show-the-members-page-in-the-channel-list-to-quickly",
					),
					&mut enabled,
				)
				.changed()
				{
					action = Some(Action::ShowMembers { enabled });
				}
			});
			ui.add_space(24.0);
		}
		ui.label(design::semibold(
			ui,
			crate::i18n::translate("server-admin-members-recent-members"),
			15.0,
		));
		let search_width = (ui.available_width() - 44.0).clamp(100.0, 260.0);
		ui.horizontal_wrapped(|ui| {
			ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
			egui::Frame::new()
				.stroke(egui::Stroke::new(1.0, colors.border))
				.corner_radius(7)
				.inner_margin(egui::Margin::symmetric(8, 5))
				.show(ui, |ui| {
					ui.horizontal(|ui| {
						icons::inline(ui, icons::Icon::Search, 14.0, colors.muted);
						if ui
							.add(
								egui::TextEdit::singleline(&mut self.query.search)
									.frame(egui::Frame::NONE)
									.font(egui::FontId::proportional(13.0))
									.hint_text(crate::i18n::translate(
										"server-admin-members-search-by-username-or-id",
									))
									.desired_width(search_width)
									.char_limit(100),
							)
							.changed()
						{
							self.query.after = None;
							self.query_changed = Some(ui.input(|input| input.time));
						}
					});
				});
		});
		ui.horizontal_wrapped(|ui| {
			ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
			let before = self.query.sort;
			let sorts = [
				crate::i18n::translate("server-admin-members-newest-members"),
				crate::i18n::translate("server-admin-members-oldest-members"),
				crate::i18n::translate("server-admin-members-newest-discord-accounts"),
				crate::i18n::translate("server-admin-members-oldest-discord-accounts"),
			];
			egui::ComboBox::from_id_salt("member-sort")
				.selected_text(&sorts[usize::from(self.query.sort.saturating_sub(1)).min(3)])
				.width(200.0)
				.show_ui(ui, |ui| {
					for (index, name) in sorts.iter().enumerate() {
						ui.selectable_value(&mut self.query.sort, index as u8 + 1, name);
					}
				});
			if before != self.query.sort {
				self.query.after = None;
				self.query_changed = Some(0.0);
			}
			if state.can_prune_guild(guild)
				&& ui
					.add_enabled(
						!state.server_admin.pending,
						egui::Button::new(
							RichText::new(crate::i18n::translate("server-admin-members-prune"))
								.color(colors.danger),
						),
					)
					.clicked()
			{
				self.dialog = Some(Dialog::Prune {
					days: 7,
					counted: None,
				});
			}
		});
		if ui
			.checkbox(
				&mut self.query.recent,
				crate::i18n::translate("server-admin-members-joined-in-the-last-7-days"),
			)
			.changed()
		{
			self.query.after = None;
			self.query_changed = Some(0.0);
		}
		if self
			.query_changed
			.is_some_and(|at| ui.input(|input| input.time) - at >= 0.3)
			&& !state.server_admin.pending
		{
			action = Some(Action::LoadMembers(self.query.clone()));
			self.query_changed = None;
		}
		if self.query_changed.is_some() {
			ui.ctx()
				.request_repaint_after(std::time::Duration::from_millis(300));
		}
		ui.add_space(10.0);
		ui.separator();
		if let Some(members) = &state.server_admin.members {
			if ui.available_width() < 820.0 {
				for member in &members.items {
					ui.push_id(member.user.id, |ui| {
						self.member_card(
							ui,
							state,
							guild,
							member,
							&members.roles,
							avatars,
							profile,
							&mut action,
						);
					});
				}
			} else {
				ui.scope(|ui| {
					ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
					let width = ui.available_width();
					ui.set_width(width);
					let names = (width * 0.24).clamp(190.0, 310.0);
					let dates = 112.0;
					let method = 108.0;
					let roles = width - names - dates * 2.0 - method - 136.0;
					ui.horizontal(|ui| {
						for (text, width) in [
							("server-stickers-show-name", names),
							("server-admin-member-card-member-since", dates),
							("server-admin-member-card-joined-discord", dates),
							("server-admin-member-card-join-method", method),
							("server-settings-page-roles", roles),
							("server-admin-members-signals", 64.0),
						] {
							cell_text(ui, &crate::i18n::translate(text), width, true);
						}
					});
					ui.separator();
					for member in &members.items {
						ui.push_id(member.user.id, |ui| {
							ui.horizontal(|ui| {
								let user_row = ui
									.allocate_ui_with_layout(
										egui::vec2(names, 44.0),
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											ui.set_width(names);
											ui.set_min_height(44.0);
											let avatar =
												avatars.show(ui, &member.user, 30.0, state.demo);
											profile.person_click(ui, &avatar, None, &member.user);
											ui.vertical(|ui| {
												ui.set_width((names - 40.0).max(40.0));
												ui.add(
													egui::Label::new(design::medium(
														ui,
														member
															.nick
															.as_deref()
															.unwrap_or(&member.user.name),
														13.0,
													))
													.truncate(),
												);
												ui.add(
													egui::Label::new(
														RichText::new(&member.user.name)
															.size(11.0)
															.color(colors.muted),
													)
													.truncate(),
												);
											});
										},
									)
									.response;
								user_menu::popup(&user_row, user_row.id.with("member-menu")).show(
									|ui| {
										self.member_menu(
											ui,
											state,
											guild,
											member,
											&members.roles,
											profile,
											&mut action,
										)
									},
								);
								cell_text(ui, &date(member.joined_at), dates, false);
								cell_text(
									ui,
									&date(Some(i128::from(
										(member.user.id.0 >> 22) + 1_420_070_400_000,
									))),
									dates,
									false,
								);
								if let Some(code) = &member.invite_code {
									ui.allocate_ui_with_layout(
										egui::vec2(method, 44.0),
										egui::Layout::left_to_right(egui::Align::Center),
										|ui| {
											ui.set_width(method);
											ui.set_min_height(44.0);
											egui::Frame::new()
												.fill(colors.raised)
												.corner_radius(3)
												.inner_margin(3)
												.show(ui, |ui| {
													ui.set_max_width(method - 6.0);
													ui.spacing_mut().item_spacing.x = 4.0;
													icons::inline(
														ui,
														icons::Icon::Link,
														12.0,
														colors.muted,
													);
													ui.add(
														egui::Label::new(
															RichText::new(code).size(12.0),
														)
														.truncate(),
													)
													.on_hover_text(code);
												});
										},
									);
								} else {
									cell_text(ui, &join_method(member), method, false);
								}
								ui.allocate_ui_with_layout(
									egui::vec2(roles, 44.0),
									egui::Layout::left_to_right(egui::Align::Center),
									|ui| {
										ui.set_width(roles);
										ui.set_min_height(44.0);
										let mut known = members
											.roles
											.iter()
											.filter(|role| member.roles.contains(&role.role.id));
										if let Some(role) = known.next() {
											egui::Frame::new()
												.fill(colors.raised)
												.corner_radius(4)
												.inner_margin(4)
												.show(ui, |ui| {
													ui.set_max_width((roles - 30.0).max(30.0));
													ui.add(
														egui::Label::new(
															RichText::new(&role.role.name)
																.size(12.0)
																.color(colors.text),
														)
														.truncate(),
													)
													.on_hover_text(&role.role.name);
												});
											let extra = known.count();
											if extra > 0 {
												ui.small(format!("+{extra}"));
											}
										}
									},
								);
								cell_text(ui, &signals(member), 64.0, false);
								let button = icons::button(
									ui,
									icons::Icon::More,
									24.0,
									&crate::i18n::translate("server-admin-members-member-actions"),
								);
								egui::Popup::menu(&button).show(|ui| {
									self.member_menu(
										ui,
										state,
										guild,
										member,
										&members.roles,
										profile,
										&mut action,
									)
								});
							});
							ui.separator();
						});
					}
				});
			}
			ui.horizontal_wrapped(|ui| {
				ui.weak(format!(
					"{} {} {} {}",
					crate::i18n::translate("server-admin-members-showing"),
					members.items.len(),
					crate::i18n::translate("server-admin-members-of"),
					members.total
				));
				if self.query.after.is_some()
					&& ui
						.add_enabled(
							!state.server_admin.pending,
							egui::Button::new(crate::i18n::translate(
								"server-admin-members-first-page",
							)),
						)
						.clicked()
				{
					self.query.after = None;
					action = Some(Action::LoadMembers(self.query.clone()));
				}
				if let Some(cursor) = members.next
					&& ui
						.add_enabled(
							!state.server_admin.pending,
							egui::Button::new(crate::i18n::translate(
								"server-admin-members-next-page",
							)),
						)
						.clicked()
				{
					self.query.after = Some(cursor);
					action = Some(Action::LoadMembers(self.query.clone()));
				}
			});
			if members.items.is_empty() {
				ui.weak(crate::i18n::translate(
					"server-admin-members-no-members-match-this-search",
				));
			}
		}
		if let Some(action) = action
			&& let Some(command) = state.request_server_admin(guild, action)
		{
			commands.push(command);
		}
	}
	#[allow(clippy::too_many_arguments)]
	fn member_card(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		member: &Member,
		roles: &[model::server_admin::Role],
		avatars: &mut Avatars,
		profile: &mut crate::profiles::ProfileSession,
		action: &mut Option<Action>,
	) {
		let colors = design::palette(ui);
		design::card(ui, |ui| {
			let row = ui
				.horizontal(|ui| {
					let avatar = avatars.show(ui, &member.user, 32.0, state.demo);
					profile.person_click(ui, &avatar, None, &member.user);
					ui.vertical(|ui| {
						ui.set_width((ui.available_width() - 40.0).max(40.0));
						let name = member.nick.as_deref().unwrap_or(&member.user.name);
						let name_response = ui
							.add(
								egui::Label::new(design::medium(ui, name, 14.0))
									.truncate()
									.sense(egui::Sense::click()),
							)
							.on_hover_text(name);
						profile.person_click(ui, &name_response, None, &member.user);
						if member
							.nick
							.as_ref()
							.is_some_and(|nick| nick != &member.user.name)
						{
							ui.add(
								egui::Label::new(
									RichText::new(&member.user.name).color(colors.muted),
								)
								.truncate(),
							)
							.on_hover_text(&member.user.name);
						}
					});
					let button = icons::button(
						ui,
						icons::Icon::More,
						28.0,
						&crate::i18n::translate("server-admin-member-card-member-actions"),
					);
					egui::Popup::menu(&button).show(|ui| {
						self.member_menu(ui, state, guild, member, roles, profile, action)
					});
				})
				.response;
			user_menu::popup(&row, row.id.with("member-menu"))
				.show(|ui| self.member_menu(ui, state, guild, member, roles, profile, action));
			ui.label(
				RichText::new(format!(
					"{} {}",
					crate::i18n::translate("server-admin-member-card-member-since"),
					date(member.joined_at).replace('\n', " · ")
				))
				.color(colors.muted),
			);
			let signals = signals(member);
			if !signals.is_empty() {
				ui.label(signals);
			}
			egui::CollapsingHeader::new(crate::i18n::translate(
				"server-admin-member-card-member-details",
			))
			.show(ui, |ui| {
				ui.label(format!(
					"{}: {}",
					crate::i18n::translate("server-admin-member-card-joined-discord"),
					date(Some(i128::from(
						(member.user.id.0 >> 22) + 1_420_070_400_000
					)))
					.replace('\n', " · ")
				));
				ui.label(format!(
					"{}: {}",
					crate::i18n::translate("server-admin-member-card-join-method"),
					join_method(member)
				));
				if let Some(code) = &member.invite_code {
					ui.label(format!(
						"{}: {code}",
						crate::i18n::translate("server-admin-member-card-invite")
					));
				}
				ui.horizontal_wrapped(|ui| {
					for role in roles
						.iter()
						.filter(|role| member.roles.contains(&role.role.id))
					{
						ui.label(&role.role.name);
					}
				});
			});
		});
	}
	#[allow(clippy::too_many_arguments)]
	fn member_menu(
		&mut self,
		ui: &mut egui::Ui,
		state: &State,
		guild: Id,
		member: &Member,
		roles: &[model::server_admin::Role],
		profile: &mut crate::profiles::ProfileSession,
		action: &mut Option<Action>,
	) {
		ui.set_width(190.0);
		let colors = design::palette(ui);
		if ui
			.button(crate::i18n::translate("server-admin-member-menu-profile"))
			.clicked()
		{
			profile.command_open(member.user.clone());
			ui.close();
		}
		if let Some(dm) = state.channels.iter().find(|channel| {
			channel.kind == 1
				&& channel
					.recipients
					.iter()
					.any(|user| user.id == member.user.id)
		}) && ui
			.button(crate::i18n::translate("server-admin-member-menu-message"))
			.clicked()
		{
			self.message = Some(dm.id);
			ui.close();
		}
		ui.separator();
		if state.can_edit_guild_nickname(guild, member.user.id)
			&& ui
				.button(crate::i18n::translate(
					"server-admin-member-menu-change-nickname",
				))
				.clicked()
		{
			self.dialog = Some(Dialog::Nickname {
				user: member.user.id,
				name: member.nick.clone().unwrap_or_default(),
			});
			ui.close();
		}
		if state
			.user
			.as_ref()
			.is_none_or(|user| user.id != member.user.id)
		{
			let blocked = state.user_blocked(member.user.id) == Some(true);
			if ui
				.add_enabled(
					!state.user_action_pending(),
					egui::Button::new(
						RichText::new(crate::i18n::translate_if_key(if blocked {
							"server-admin-member-menu-unblock"
						} else {
							"server-admin-member-menu-block"
						}))
						.color(colors.danger),
					),
				)
				.clicked()
			{
				self.user_action = Some(user_menu::Action::Block {
					user: member.user.id,
					blocked: !blocked,
				});
				ui.close();
			}
		}
		if roles
			.iter()
			.any(|role| state.can_edit_member_role(guild, member.user.id, role.role.id))
		{
			ui.separator();
			ui.menu_button(
				crate::i18n::translate("server-admin-member-menu-roles"),
				|ui| {
					for role in roles.iter().filter(|role| {
						state.can_edit_member_role(guild, member.user.id, role.role.id)
					}) {
						let mut assigned = member.roles.contains(&role.role.id);
						if ui
							.add_enabled(
								!state.server_admin.pending,
								egui::Checkbox::new(&mut assigned, &role.role.name),
							)
							.changed()
						{
							*action = Some(Action::SetRole {
								user: member.user.id,
								role: role.role.id,
								assigned,
							});
							ui.close();
						}
					}
				},
			);
		}
		if state.can_kick_guild_member(guild, member.user.id)
			&& ui
				.button(
					RichText::new(format!(
						"{} {}",
						crate::i18n::translate("server-admin-member-menu-kick"),
						member.user.name
					))
					.color(colors.danger),
				)
				.clicked()
		{
			self.dialog = Some(Dialog::Kick {
				user: member.user.id,
				name: member.user.name.clone(),
			});
			ui.close();
		}
		ui.separator();
		if ui
			.button(crate::i18n::translate(
				"server-admin-member-menu-copy-user-id",
			))
			.clicked()
		{
			ui.ctx().copy_text(member.user.id.to_string());
			ui.close();
		}
	}
	fn dialog(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		guild: Id,
		commands: &mut Vec<Command>,
	) {
		let Some(dialog) = &mut self.dialog else {
			return;
		};
		let mut close = false;
		let mut action = None;
		let ready = !state.server_admin.pending && !state.server_admin.needs_refresh;
		let (title, subtitle) = match &*dialog {
			Dialog::Rename { .. } => (
				"Rename emoji".to_owned(),
				"Names use letters, numbers and underscores.".to_owned(),
			),
			Dialog::Delete { name, .. } => (
				"Delete emoji?".to_owned(),
				format!("Removing :{name}: cannot be undone."),
			),
			Dialog::Nickname { .. } => (
				"Change nickname".to_owned(),
				"Only affects how this member appears in this server.".to_owned(),
			),
			Dialog::Kick { name, .. } => (
				format!("Kick {name}?"),
				"They can rejoin using a new invite.".to_owned(),
			),
			Dialog::Prune { .. } => (
				"Prune members".to_owned(),
				"Removes inactive members who hold no additional roles.".to_owned(),
			),
		};
		let destructive = matches!(
			&*dialog,
			Dialog::Delete { .. } | Dialog::Kick { .. } | Dialog::Prune { .. }
		);
		let mut builder = crate::dialog::Dialog::new("server-admin-confirmation", title)
			.subtitle(subtitle)
			.width(440.0);
		if destructive {
			builder = builder.danger();
		}
		let response = builder.show(ctx, |d| {
			d.content(|ui| {
				ui.spacing_mut().item_spacing.y = 10.0;
				match dialog {
					Dialog::Rename { name, .. } => {
						let label = crate::dialog::label(ui, "server-admin-dialog-emoji-name");
						crate::dialog::input(ui, egui::TextEdit::singleline(name).char_limit(32))
							.labelled_by(label.id);
					}
					Dialog::Delete { .. } | Dialog::Kick { .. } => {}
					Dialog::Nickname { name, .. } => {
						let label = crate::dialog::label(ui, "server-admin-dialog-nickname");
						crate::dialog::input(
							ui,
							egui::TextEdit::singleline(name)
								.hint_text(crate::i18n::translate(
									"server-admin-dialog-use-their-username",
								))
								.char_limit(32),
						)
						.labelled_by(label.id);
						crate::dialog::hint(
							ui,
							"server-admin-dialog-leave-blank-to-use-their-username",
						);
					}
					Dialog::Prune { days, counted } => {
						crate::dialog::label(ui, "server-admin-dialog-inactive-for");
						let before = *days;
						let label = |value| match value {
							7 => crate::i18n::translate("server-admin-dialog-7-days"),
							30 => crate::i18n::translate("server-admin-dialog-30-days"),
							_ => value.to_string(),
						};
						egui::ComboBox::from_id_salt("prune-days")
							.selected_text(label(*days))
							.width(ui.available_width())
							.show_ui(ui, |ui| {
								for value in [7, 30] {
									ui.selectable_value(days, value, label(value));
								}
							});
						if before != *days {
							*counted = None;
						}
						if *counted == Some(*days)
							&& !state.server_admin.pending
							&& state.server_admin.error.is_none()
							&& let Some(count) = state.server_admin.pruned
						{
							crate::dialog::notice(
								ui,
								crate::dialog::Level::Warning,
								&format!("{count} members would be removed."),
							);
						}
					}
				}
				if let Some(error) = state.server_admin.error {
					crate::dialog::notice(ui, crate::dialog::Level::Error, error);
				}
			});
			d.footer(|ui| {
				let kind = if destructive {
					crate::dialog::Action::Danger
				} else {
					crate::dialog::Action::Primary
				};
				match dialog {
					Dialog::Rename { id, name } => {
						ui.add_enabled_ui(
							ready
								&& state.can_edit_guild_emoji(guild, *id)
								&& model::server_admin::valid_emoji_name(name),
							|ui| {
								if crate::dialog::action(ui, "server-admin-dialog-save", kind)
									.clicked()
								{
									action = Some(Action::RenameEmoji {
										id: *id,
										name: name.clone(),
									});
								}
							},
						);
					}
					Dialog::Delete { id, .. } => {
						ui.add_enabled_ui(ready && state.can_edit_guild_emoji(guild, *id), |ui| {
							if crate::dialog::action(ui, "server-admin-dialog-delete-emoji", kind)
								.clicked()
							{
								action = Some(Action::DeleteEmoji { id: *id });
							}
						});
					}
					Dialog::Nickname { user, name } => {
						ui.add_enabled_ui(
							ready
								&& state.can_edit_guild_nickname(guild, *user)
								&& !name.chars().any(char::is_control),
							|ui| {
								if crate::dialog::action(ui, "server-admin-dialog-save", kind)
									.clicked()
								{
									action = Some(Action::SetNickname {
										user: *user,
										nick: name.clone(),
									});
								}
							},
						);
					}
					Dialog::Kick { user, .. } => {
						ui.add_enabled_ui(
							ready && state.can_kick_guild_member(guild, *user),
							|ui| {
								if crate::dialog::action(
									ui,
									"server-admin-dialog-kick-member",
									kind,
								)
								.clicked()
								{
									action = Some(Action::Kick { user: *user });
								}
							},
						);
					}
					Dialog::Prune { days, counted } => {
						let previewed = *counted == Some(*days)
							&& !state.server_admin.pending
							&& state.server_admin.error.is_none();
						let count = state.server_admin.pruned.unwrap_or(0);
						ui.add_enabled_ui(
							previewed && count > 0 && state.can_prune_guild(guild),
							|ui| {
								if crate::dialog::action(
									ui,
									"server-admin-dialog-prune-members",
									kind,
								)
								.clicked()
								{
									action = Some(Action::Prune {
										days: *days,
										execute: true,
									});
									*counted = None;
								}
							},
						);
						ui.add_enabled_ui(ready && state.can_prune_guild(guild), |ui| {
							if crate::dialog::action(
								ui,
								"server-admin-dialog-preview",
								crate::dialog::Action::Outline,
							)
							.clicked()
							{
								*counted = Some(*days);
								action = Some(Action::Prune {
									days: *days,
									execute: false,
								});
							}
						});
					}
				}
				ui.add_enabled_ui(!state.server_admin.saving, |ui| {
					close |= crate::dialog::action(
						ui,
						"server-admin-dialog-cancel",
						crate::dialog::Action::Neutral,
					)
					.clicked();
				});
			});
		});
		if let Some(action) = action
			&& let Some(command) = state.request_server_admin(guild, action)
		{
			commands.push(command);
			self.dialog_submitted = true;
		}
		if close || (response.close && !state.server_admin.saving) {
			self.dialog = None;
		}
	}
}
fn cell_text(ui: &mut egui::Ui, text: &str, width: f32, heading: bool) {
	let translated = heading.then(|| crate::i18n::translate_if_key(text));
	let text = translated.as_deref().unwrap_or(text);
	ui.allocate_ui_with_layout(
		egui::vec2(width, if heading { 32.0 } else { 44.0 }),
		egui::Layout::left_to_right(egui::Align::Center),
		|ui| {
			ui.set_width(width);
			ui.set_min_height(if heading { 32.0 } else { 44.0 });
			ui.vertical(|ui| {
				ui.set_width(width);
				ui.spacing_mut().item_spacing.y = 0.0;
				for line in text.lines().take(2) {
					ui.add(
						egui::Label::new(if heading {
							design::semibold(ui, line, 11.0)
						} else {
							RichText::new(line).size(12.0)
						})
						.truncate(),
					)
					.on_hover_text(text);
				}
			});
		},
	);
}
fn join_method(member: &Member) -> String {
	if member.invite_code.is_some() {
		return crate::i18n::translate("server-admin-join-method-invite");
	}
	crate::i18n::translate_if_key(match member.join_source {
		Some(1) => "server-admin-join-method-bot",
		Some(2) => "server-admin-join-method-integration",
		Some(3) => "server-admin-join-method-discovery",
		Some(4) => "server-admin-join-method-student-hub",
		Some(5) => "server-admin-join-method-invite",
		Some(6) => "server-admin-join-method-vanity-url",
		Some(7) => "server-admin-join-method-application",
		Some(8) => "server-admin-join-method-linked-lobby",
		_ => "server-admin-join-method-unknown",
	})
}
fn date(millis: Option<i128>) -> String {
	millis
		.and_then(|millis| millis.checked_mul(1_000_000))
		.and_then(|nanos| time::OffsetDateTime::from_unix_timestamp_nanos(nanos).ok())
		.map_or_else(
			|| crate::i18n::translate("server-admin-date-unknown"),
			|date| {
				let date = crate::local_time::local(date);
				format!(
					"{} {}, {}\n{}:{:02} {}",
					&date.month().to_string()[..3],
					date.day(),
					date.year(),
					match date.hour() % 12 {
						0 => 12,
						hour => hour,
					},
					date.minute(),
					if date.hour() < 12 { "AM" } else { "PM" }
				)
			},
		)
}
fn signals(member: &Member) -> String {
	let now = time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
	let mut signals = Vec::new();
	if member.timeout_until.is_some_and(|until| until > now) {
		signals.push(crate::i18n::translate("server-admin-signals-timed-out"));
	}
	if member.unusual_dm_until.is_some_and(|until| until > now) {
		signals.push(crate::i18n::translate(
			"server-admin-signals-unusual-dm-activity",
		));
	}
	if let Some(flags) = member.flags {
		if flags & (1 << 7) != 0 {
			signals.push(crate::i18n::translate(
				"server-admin-signals-username-flagged",
			));
		}
		if flags & (1 << 10) != 0 {
			signals.push(crate::i18n::translate(
				"server-admin-signals-server-tag-flagged",
			));
		}
		if flags & 1 != 0 {
			signals.push(crate::i18n::translate("server-admin-signals-rejoined"));
		}
	}
	signals.join(", ")
}
