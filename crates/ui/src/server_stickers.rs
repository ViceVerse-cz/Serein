//! Native server sticker catalog and bounded create/edit/delete flows.
use crate::{avatars::Avatars, design, icons};
use client_core::{Command, State};
use egui::RichText;
use model::{Id, server_admin::Action};

pub(super) type PreparedSticker = (String, String, Vec<u8>, egui::ColorImage);

struct Upload {
	name: String,
	description: String,
	tags: String,
	filename: String,
	file: Vec<u8>,
	texture: egui::TextureHandle,
}

enum Dialog {
	Edit {
		id: Id,
		name: String,
		description: String,
		tags: String,
	},
	Delete {
		id: Id,
		name: String,
	},
}

#[derive(Default)]
pub(super) struct StickersUi {
	pub request: u64,
	choosing: bool,
	request_started: bool,
	upload: Option<Upload>,
	submitted_upload: bool,
	dialog: Option<Dialog>,
	dialog_submitted: bool,
	error: Option<&'static str>,
}

impl StickersUi {
	pub fn has_changes(&self) -> bool {
		self.choosing || self.upload.is_some() || self.dialog_submitted
	}

	pub fn load(&mut self, state: &mut State, guild: Id) -> Option<Command> {
		if state.server_admin.pending
			|| (state.server_admin.guild == Some(guild)
				&& (state.server_admin.error.is_some() || state.server_admin.stickers.is_some()))
		{
			return None;
		}
		state.request_server_admin(guild, Action::LoadStickers)
	}

	pub fn choose(&mut self) {
		if self.upload.is_none() && !self.choosing {
			self.choosing = true;
			self.error = None;
		}
	}

	pub fn take_request(&mut self) -> bool {
		if !self.choosing || self.request_started {
			return false;
		}
		self.request_started = true;
		true
	}

	pub fn accept(
		&mut self,
		ctx: &egui::Context,
		result: Result<Option<PreparedSticker>, &'static str>,
	) {
		if !self.choosing || !self.request_started {
			return;
		}
		self.choosing = false;
		self.request_started = false;
		match result {
			Ok(Some((name, filename, file, preview))) if file.len() <= 512 * 1024 => {
				self.upload = Some(Upload {
					name,
					description: String::new(),
					tags: String::new(),
					filename,
					file,
					texture: ctx.load_texture(
						"server-sticker-upload",
						preview,
						egui::TextureOptions::LINEAR,
					),
				});
			}
			Ok(Some(_)) => self.error = Some("Prepared sticker exceeds 512 KB"),
			Ok(None) => {}
			Err(error) => self.error = Some(error),
		}
	}

	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		guild: Id,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		if self.submitted_upload && !state.server_admin.pending {
			self.submitted_upload = false;
			if state.server_admin.error.is_none() {
				self.upload = None;
			}
		}
		if self.dialog_submitted && !state.server_admin.pending {
			self.dialog_submitted = false;
			if state.server_admin.error.is_none() {
				self.dialog = None;
			}
		}
		if !state.can_create_guild_sticker(guild) {
			self.upload = None;
			self.choosing = false;
			self.request_started = false;
		}

		ui.label(design::semibold(ui, "Stickers", 22.0));
		ui.label("Add custom stickers for members to use in this server. Artwork is cropped and resized to 320 × 320 pixels before upload.");
		ui.add_space(14.0);
		if let Some(error) = state.server_admin.error.or(self.error) {
			design::notice(ui, design::Level::Error, error);
			if ui
				.add_enabled(!state.server_admin.pending, egui::Button::new("Reload"))
				.clicked() && let Some(command) =
				state.request_server_admin(guild, Action::LoadStickers)
			{
				commands.push(command);
				self.error = None;
			}
		}
		if state.server_admin.pending {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.weak(if state.server_admin.saving {
					"Saving changes…"
				} else {
					"Loading…"
				});
			});
		}

		if state.can_create_guild_sticker(guild) {
			if ui
				.add_enabled_ui(
					!self.choosing && self.upload.is_none() && !state.server_admin.pending,
					|ui| design::button(ui, "Upload Sticker", design::ButtonKind::Primary),
				)
				.inner
				.clicked()
			{
				self.choose();
			}
			ui.small("Static PNG, JPEG and WebP artwork is supported up to 8 MB. The prepared PNG must fit within Discord's 512 KB limit.");
		}
		if self.choosing {
			ui.weak("Preparing sticker artwork…");
		}
		if let Some(upload) = &mut self.upload {
			let colors = design::palette(ui);
			let mut cancel_upload = false;
			egui::Frame::new()
				.stroke(egui::Stroke::new(1.0, colors.border))
				.corner_radius(10)
				.inner_margin(14)
				.show(ui, |ui| {
					ui.label(design::semibold(ui, "Review sticker", 16.0));
					ui.horizontal(|ui| {
						ui.add(egui::Image::from_texture(&upload.texture).fit_to_exact_size(egui::Vec2::splat(96.0)));
						ui.vertical(|ui| {
							crate::dialog::label(ui, "Name");
							ui.add(egui::TextEdit::singleline(&mut upload.name).char_limit(30));
							crate::dialog::label(ui, "Related emoji");
							ui.add(egui::TextEdit::singleline(&mut upload.tags).hint_text("For example: 🐀").char_limit(200));
						});
					});
					crate::dialog::label(ui, "Description (optional)");
					ui.add(egui::TextEdit::singleline(&mut upload.description).char_limit(100));
					let valid = valid_fields(&upload.name, &upload.description, &upload.tags);
					if !valid {
						design::notice(ui, design::Level::Error, "Use a 2–30 character name, an optional description up to 100 characters, and at least one related emoji.");
					}
					ui.horizontal(|ui| {
						if ui.add_enabled(valid && !state.server_admin.pending, egui::Button::new("Upload")).clicked()
							&& let Some(command) = state.request_server_admin(guild, Action::CreateSticker {
								name: upload.name.trim().to_owned(),
								description: upload.description.trim().to_owned(),
								tags: upload.tags.trim().to_owned(),
								filename: upload.filename.clone(),
								content_type: "image/png".into(),
								file: upload.file.clone(),
							})
						{
							self.submitted_upload = true;
							commands.push(command);
						}
						if ui.add_enabled(!state.server_admin.pending, egui::Button::new("Cancel")).clicked() {
							cancel_upload = true;
						}
					});
				});
			if cancel_upload {
				self.upload = None;
			}
		}

		ui.add_space(22.0);
		ui.separator();
		ui.add_space(18.0);
		let Some(catalog) = state.server_admin.stickers.as_ref() else {
			return;
		};
		let count = catalog.items.len();
		ui.horizontal(|ui| {
			ui.label(design::semibold(ui, "Your stickers", 18.0));
			ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
				ui.weak(catalog.limit.map_or_else(
					|| format!("{count} stickers"),
					|limit| format!("{} of {limit} slots used", count.min(limit)),
				));
			});
		});
		ui.add_space(10.0);
		if catalog.items.is_empty() {
			ui.vertical_centered(|ui| ui.weak("No custom stickers yet."));
		} else {
			let available = ui.available_width();
			let columns = ((available / 190.0).floor() as usize).clamp(1, 4);
			egui::Grid::new("server-sticker-grid")
				.num_columns(columns)
				.spacing(egui::vec2(10.0, 10.0))
				.show(ui, |ui| {
					for (index, row) in catalog.items.iter().enumerate() {
						ui.push_id(row.sticker.id, |ui| {
							egui::Frame::new()
								.fill(design::palette(ui).surface)
								.stroke(egui::Stroke::new(1.0, design::palette(ui).border))
								.corner_radius(10)
								.inner_margin(10)
								.show(ui, |ui| {
									ui.set_width(
										((available - 10.0 * (columns.saturating_sub(1)) as f32)
											/ columns as f32 - 22.0)
											.max(120.0),
									);
									ui.vertical_centered(|ui| {
										avatars.sticker_image(
											ui,
											&row.sticker,
											egui::Vec2::splat(104.0),
											state.demo,
										);
										ui.add(
											egui::Label::new(design::medium(
												ui,
												&row.sticker.name,
												13.0,
											))
											.truncate(),
										);
										if let Some(user) = &row.uploader {
											ui.weak(format!("by {}", user.name));
										}
										if state.can_edit_guild_sticker(guild, row.sticker.id) {
											let button = icons::button(
												ui,
												icons::Icon::More,
												22.0,
												"Sticker actions",
											);
											egui::Popup::menu(&button).show(|ui| {
												if ui.button("Edit").clicked() {
													self.dialog = Some(Dialog::Edit {
														id: row.sticker.id,
														name: row.sticker.name.clone(),
														description: row
															.sticker
															.description
															.clone(),
														tags: row.sticker.tags.clone(),
													});
													ui.close();
												}
												if ui
													.button(
														RichText::new("Delete Sticker")
															.color(design::palette(ui).danger),
													)
													.clicked()
												{
													self.dialog = Some(Dialog::Delete {
														id: row.sticker.id,
														name: row.sticker.name.clone(),
													});
													ui.close();
												}
											});
										}
									});
								});
						});
						if (index + 1) % columns == 0 {
							ui.end_row();
						}
					}
				});
		}
		self.dialog(ui.ctx(), state, guild, commands);
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
		let (title, subtitle, danger) = match dialog {
			Dialog::Edit { .. } => (
				"Edit sticker",
				"Update the sticker name, description and related emoji.".to_owned(),
				false,
			),
			Dialog::Delete { name, .. } => (
				"Delete sticker?",
				format!("Removing {name} cannot be undone."),
				true,
			),
		};
		let mut action = None;
		let mut cancel = false;
		let mut builder = crate::dialog::Dialog::new("server-sticker-dialog", title)
			.subtitle(subtitle)
			.width(440.0);
		if danger {
			builder = builder.danger();
		}
		let response = builder.show(ctx, |dialog_ui| {
			dialog_ui.content(|ui| match dialog {
				Dialog::Edit {
					name,
					description,
					tags,
					..
				} => {
					let label = crate::dialog::label(ui, "Name");
					crate::dialog::input(ui, egui::TextEdit::singleline(name).char_limit(30))
						.labelled_by(label.id);
					ui.add_space(12.0);
					let label = crate::dialog::label(ui, "Description (optional)");
					crate::dialog::input(
						ui,
						egui::TextEdit::singleline(description).char_limit(100),
					)
					.labelled_by(label.id);
					ui.add_space(12.0);
					let label = crate::dialog::label(ui, "Related emoji");
					crate::dialog::input(ui, egui::TextEdit::singleline(tags).char_limit(200))
						.labelled_by(label.id);
				}
				Dialog::Delete { .. } => {}
			});
			dialog_ui.footer(|ui| {
				match dialog {
					Dialog::Edit {
						id,
						name,
						description,
						tags,
					} => {
						if ui
							.add_enabled_ui(
								!state.server_admin.pending
									&& state.can_edit_guild_sticker(guild, *id)
									&& valid_fields(name, description, tags),
								|ui| {
									crate::dialog::action(
										ui,
										"Save",
										crate::dialog::Action::Primary,
									)
								},
							)
							.inner
							.clicked()
						{
							action = Some(Action::EditSticker {
								id: *id,
								name: name.trim().to_owned(),
								description: description.trim().to_owned(),
								tags: tags.trim().to_owned(),
							});
						}
					}
					Dialog::Delete { id, .. } => {
						if ui
							.add_enabled_ui(
								!state.server_admin.pending
									&& state.can_edit_guild_sticker(guild, *id),
								|ui| {
									crate::dialog::action(
										ui,
										"Delete Sticker",
										crate::dialog::Action::Danger,
									)
								},
							)
							.inner
							.clicked()
						{
							action = Some(Action::DeleteSticker { id: *id });
						}
					}
				}
				cancel =
					crate::dialog::action(ui, "Cancel", crate::dialog::Action::Neutral).clicked();
			});
		});
		if let Some(action) = action.and_then(|action| state.request_server_admin(guild, action)) {
			self.dialog_submitted = true;
			commands.push(action);
		}
		if (response.close || cancel) && !self.dialog_submitted {
			self.dialog = None;
		}
	}
}

fn valid_fields(name: &str, description: &str, tags: &str) -> bool {
	(2..=30).contains(&name.trim().chars().count())
		&& description.trim().chars().count() <= 100
		&& (1..=200).contains(&tags.trim().chars().count())
		&& !name
			.chars()
			.chain(description.chars())
			.chain(tags.chars())
			.any(char::is_control)
}

#[cfg(test)]
mod tests {
	#[test]
	fn sticker_fields_are_bounded_and_require_related_emoji() {
		assert!(super::valid_fields("ratta", "A small rat", "🐀"));
		assert!(!super::valid_fields("x", "", "🐀"));
		assert!(!super::valid_fields("ratta", "", ""));
		assert!(!super::valid_fields("ratta", "", "x\n"));
	}
}
