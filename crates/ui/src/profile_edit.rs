//! One bounded, session-only draft for the account's global profile.
use crate::{avatars::Avatars, design, icons};
use client_core::{Command, State};
use model::{Id, ProfileEdit, UserProfile};

#[derive(Clone, PartialEq, Eq)]
struct Draft {
	name: String,
	bio: String,
	pronouns: String,
	color: Option<u32>,
	/// `Some(Some(uri))` uploads a new picture, `Some(None)` removes the current one.
	avatar: Option<Option<String>>,
}
impl Draft {
	fn from_profile(profile: &UserProfile) -> Self {
		Self {
			name: profile.global_name.clone().unwrap_or_default(),
			bio: profile.bio.clone(),
			pronouns: profile.pronouns.clone(),
			color: profile.accent_color,
			avatar: None,
		}
	}
	fn changes(&self, profile: &UserProfile) -> ProfileEdit {
		let name = (!self.name.is_empty()).then(|| self.name.clone());
		ProfileEdit {
			global_name: (name != profile.global_name).then_some(name),
			bio: (self.bio != profile.bio).then(|| self.bio.clone()),
			pronouns: (self.pronouns != profile.pronouns).then(|| self.pronouns.clone()),
			accent_color: (self.color != profile.accent_color).then_some(self.color),
			avatar: match &self.avatar {
				// Removing a picture the account does not have is not a change.
				Some(None) if profile.user.avatar.is_none() => None,
				other => other.clone(),
			},
		}
	}
	fn rebase(&mut self, before: &Self, fresh: &Self) {
		if self.name == before.name {
			self.name = fresh.name.clone();
		}
		if self.bio == before.bio {
			self.bio = fresh.bio.clone();
		}
		if self.pronouns == before.pronouns {
			self.pronouns = fresh.pronouns.clone();
		}
		if self.color == before.color {
			self.color = fresh.color;
		}
	}
}

/// What the preview card paints inside the avatar circle.
enum Picture<'a> {
	/// The account's current picture, fetched like any other avatar.
	Remote,
	/// A locally chosen image that has not been saved yet.
	Pending(&'a egui::TextureHandle),
	/// A locally chosen image that was just saved; shown until the CDN copy is fetched.
	Saved(&'a egui::TextureHandle),
	/// Removal is pending, so the initial placeholder is shown.
	Removed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AvatarAction {
	Pick,
	Remove,
	Undo,
}

#[derive(Default)]
pub(super) struct Editor {
	generation: Option<u64>,
	draft: Option<Draft>,
	baseline: Option<Draft>,
	submitted: Option<u64>,
	saved: bool,
	preview_link: Option<String>,
	/// Set when the picture button is pressed; the desktop shell opens the native picker.
	pub avatar_request: Option<(u64, Id, u64)>,
	revision: u64,
	choosing: bool,
	pending_picture: Option<egui::TextureHandle>,
	/// Local copy of the last saved picture, keyed by the avatar hash the service returned.
	saved_picture: Option<(Option<String>, egui::TextureHandle)>,
}
impl Editor {
	/// Result of the native picker. Errors are returned so the caller can toast them.
	pub fn accept_avatar(
		&mut self,
		ctx: &egui::Context,
		request: (u64, Id, u64),
		result: Result<Option<(String, egui::ColorImage)>, &'static str>,
	) -> Result<(), &'static str> {
		if !self.choosing
			|| request
				!= (
					self.generation.unwrap_or_default(),
					request.1,
					self.revision,
				) {
			return Ok(());
		}
		self.choosing = false;
		match result {
			Ok(Some((uri, image))) => {
				if !model::valid_avatar_uri(&uri) || image.size[0] > 512 || image.size[1] > 512 {
					return Err("That picture is too large; choose a simpler image");
				}
				let Some(draft) = self.draft.as_mut() else {
					return Ok(());
				};
				draft.avatar = Some(Some(uri));
				self.pending_picture = Some(ctx.load_texture(
					"profile-avatar-preview",
					image,
					egui::TextureOptions::LINEAR,
				));
				self.saved = false;
				Ok(())
			}
			Ok(None) => Ok(()),
			Err(error) => Err(error),
		}
	}

	pub fn show(
		&mut self,
		ui: &mut egui::Ui,
		state: &mut State,
		avatars: &mut Avatars,
		commands: &mut Vec<Command>,
	) {
		if self.generation != Some(state.generation) {
			*self = Self {
				generation: Some(state.generation),
				..Self::default()
			};
		}
		if state.own_profile.data.is_none()
			&& !state.own_profile.loading
			&& state.own_profile.error.is_none()
			&& let Some(command) = state.load_own_profile()
		{
			commands.push(command);
		}
		if self.submitted == Some(state.own_profile.request) && !state.own_profile.saving {
			self.submitted = None;
			if state.own_profile.error.is_none()
				&& let Some(profile) = &state.own_profile.data
			{
				let uploaded = self
					.draft
					.as_ref()
					.is_some_and(|draft| matches!(draft.avatar, Some(Some(_))));
				self.saved_picture = match (uploaded, self.pending_picture.take()) {
					(true, Some(texture)) => Some((profile.user.avatar.clone(), texture)),
					_ => None,
				};
				self.draft = Some(Draft::from_profile(profile));
				self.saved = true;
			}
		}
		let colors = design::palette(ui);
		if state.own_profile.loading {
			ui.horizontal(|ui| {
				ui.add(egui::Spinner::new().size(14.0));
				ui.label(
					egui::RichText::new("Loading your profile…")
						.size(13.0)
						.color(colors.muted),
				);
			});
		}
		if let Some(error) = state.own_profile.error {
			design::notice(ui, design::Level::Error, error);
			if !state.own_profile.loading
				&& !state.own_profile.saving
				&& design::text_action(ui, "Reload profile").clicked()
				&& let Some(command) = state.load_own_profile()
			{
				commands.push(command);
			}
		}
		let Some(profile) = state.own_profile.data.as_ref() else {
			return;
		};
		let fresh = Draft::from_profile(profile);
		if let (Some(draft), Some(before)) = (&mut self.draft, &self.baseline) {
			draft.rebase(before, &fresh);
		}
		self.baseline = Some(fresh);
		let draft = self
			.draft
			.get_or_insert_with(|| Draft::from_profile(profile));
		let before = draft.clone();
		let editable = !state.own_profile.loading && !state.own_profile.saving;
		let picture = match &draft.avatar {
			Some(Some(_)) => self
				.pending_picture
				.as_ref()
				.map_or(Picture::Remote, Picture::Pending),
			Some(None) => Picture::Removed,
			None => match &self.saved_picture {
				Some((hash, texture)) if hash.is_some() && *hash == profile.user.avatar => {
					Picture::Saved(texture)
				}
				_ => Picture::Remote,
			},
		};
		let mut action = None;
		let width = ui.available_width();
		if width >= 620.0 {
			ui.horizontal_top(|ui| {
				ui.spacing_mut().item_spacing.x = 24.0;
				ui.allocate_ui_with_layout(
					egui::vec2(width - 324.0, 0.0),
					egui::Layout::top_down(egui::Align::Min),
					|ui| {
						ui.add_enabled_ui(editable, |ui| {
							action = form(ui, draft, profile, self.choosing);
						});
					},
				);
				ui.vertical(|ui| {
					ui.set_width(300.0);
					if let Some(picked) = preview(
						ui,
						draft,
						profile,
						avatars,
						(state.demo, &state.guilds),
						&mut self.preview_link,
						picture,
						editable && !self.choosing,
					) {
						action = Some(picked);
					}
				});
			});
		} else {
			ui.add_enabled_ui(editable, |ui| {
				action = form(ui, draft, profile, self.choosing);
			});
		}
		match action {
			Some(AvatarAction::Pick) if editable && !self.choosing => {
				self.revision = self.revision.wrapping_add(1);
				self.choosing = true;
				self.avatar_request = Some((state.generation, profile.user.id, self.revision));
			}
			Some(AvatarAction::Remove) => {
				draft.avatar = Some(None);
				self.pending_picture = None;
			}
			Some(AvatarAction::Undo) => {
				draft.avatar = None;
				self.pending_picture = None;
			}
			_ => {}
		}
		if *draft != before {
			self.saved = false;
		}
		let changes = draft.changes(profile);
		let changed = changes != ProfileEdit::default();
		if !changes.valid() {
			design::notice(
				ui,
				design::Level::Error,
				"Check character limits and remove control characters. A display name cannot contain only spaces.",
			);
		}
		ui.add_space(16.0);
		egui::Frame::new()
			.fill(colors.base)
			.corner_radius(8)
			.inner_margin(egui::Margin::symmetric(12, 8))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					if state.own_profile.saving {
						ui.add(egui::Spinner::new().size(14.0));
						ui.label(
							egui::RichText::new("Saving profile…")
								.size(13.0)
								.color(colors.muted),
						);
					} else if self.saved {
						ui.label(
							egui::RichText::new(if state.demo {
								"Saved in preview"
							} else {
								"Profile saved"
							})
							.size(13.0)
							.color(colors.positive),
						);
					} else if changed {
						ui.label(
							egui::RichText::new("You have unsaved changes.")
								.size(13.0)
								.color(colors.text),
						);
					}
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.add_enabled_ui(
							changed && changes.valid() && state.can_save_own_profile(),
							|ui| {
								if design::button(ui, "Save changes", design::ButtonKind::Primary)
									.clicked() && let Some(command) = state.save_own_profile(changes)
								{
									self.submitted = Some(state.own_profile.request);
									self.saved = false;
									commands.push(command);
								}
							},
						);
						ui.add_enabled_ui(changed && editable, |ui| {
							if design::button(ui, "Cancel", design::ButtonKind::Neutral).clicked() {
								self.draft =
									state.own_profile.data.as_ref().map(Draft::from_profile);
								self.pending_picture = None;
								self.saved = false;
							}
						});
					});
				});
			});
		if width < 620.0
			&& let (Some(draft), Some(profile)) = (&self.draft, &state.own_profile.data)
		{
			ui.add_space(16.0);
			let picture = match &draft.avatar {
				Some(Some(_)) => self
					.pending_picture
					.as_ref()
					.map_or(Picture::Remote, Picture::Pending),
				Some(None) => Picture::Removed,
				None => Picture::Remote,
			};
			preview(
				ui,
				draft,
				profile,
				avatars,
				(state.demo, &state.guilds),
				&mut self.preview_link,
				picture,
				false,
			);
		}
		crate::markdown::confirm_external_link(ui.ctx(), &mut self.preview_link, true);
		if !state.demo && !state.gateway_connected {
			design::hint(ui, "Reconnect to save your profile.");
		}
	}
}

fn form(
	ui: &mut egui::Ui,
	draft: &mut Draft,
	profile: &UserProfile,
	choosing: bool,
) -> Option<AvatarAction> {
	let colors = design::palette(ui);
	ui.spacing_mut().item_spacing.y = 6.0;
	let mut action = None;
	let has_picture = match &draft.avatar {
		Some(value) => value.is_some(),
		None => profile.user.avatar.is_some(),
	};
	design::row(
		ui,
		"Profile picture",
		Some(match &draft.avatar {
			Some(Some(_)) => "New picture chosen. Save to upload it.",
			Some(None) => "Your picture will be removed when you save.",
			None => "PNG, JPEG, GIF or WebP up to 8 MB. Cropped to a square.",
		}),
		|ui| {
			ui.add_enabled_ui(!choosing, |ui| {
				if design::button(ui, "Change", design::ButtonKind::Outline).clicked() {
					action = Some(AvatarAction::Pick);
				}
			});
			if draft.avatar.is_some() {
				if design::text_action(ui, "Undo").clicked() {
					action = Some(AvatarAction::Undo);
				}
			} else if has_picture && design::text_action(ui, "Remove").clicked() {
				action = Some(AvatarAction::Remove);
			}
			if choosing {
				ui.add(egui::Spinner::new().size(14.0));
			}
		},
	);
	ui.add_space(10.0);
	field(
		ui,
		"Display name",
		"profile-display-name",
		&mut draft.name,
		model::MAX_PROFILE_NAME_CHARS,
		false,
	);
	ui.label(
		egui::RichText::new("Leave blank to use your username.")
			.size(12.0)
			.color(colors.muted),
	);
	ui.add_space(8.0);
	field(
		ui,
		"Pronouns",
		"profile-pronouns",
		&mut draft.pronouns,
		model::MAX_PROFILE_PRONOUNS_CHARS,
		false,
	);
	ui.add_space(8.0);
	field(
		ui,
		"About Me",
		"profile-about-me",
		&mut draft.bio,
		model::MAX_PROFILE_BIO_CHARS,
		true,
	);
	ui.add_space(8.0);
	let mut enabled = draft.color.is_some();
	design::row(
		ui,
		"Profile color",
		Some("Tints your banner when you have not set a banner image."),
		|ui| {
			if let Some(color) = &mut draft.color {
				let mut rgb = [(*color >> 16) as u8, (*color >> 8) as u8, *color as u8];
				if design::color_edit(ui, &mut rgb)
					.on_hover_text("Choose profile color")
					.changed()
				{
					*color =
						(u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
				}
				if design::text_action(ui, "Use default").clicked() {
					enabled = false;
				}
			} else if design::text_action(ui, "Custom color").clicked() {
				enabled = true;
			}
		},
	);
	if enabled != draft.color.is_some() {
		draft.color = enabled.then_some(design::DEFAULT_PRIMARY_RGB);
	}
	action
}

fn field(
	ui: &mut egui::Ui,
	label: &str,
	id: &str,
	value: &mut String,
	limit: usize,
	multiline: bool,
) {
	let colors = design::palette(ui);
	let label_id = ui
		.horizontal(|ui| {
			let label = ui.label(design::eyebrow(ui, label, colors.muted));
			if multiline {
				ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
					ui.label(
						egui::RichText::new(format!("{} / {limit}", value.chars().count()))
							.size(11.0)
							.color(colors.muted),
					);
				});
			}
			label.id
		})
		.inner;
	let edit = if multiline {
		egui::TextEdit::multiline(value).desired_rows(4)
	} else {
		egui::TextEdit::singleline(value)
	};
	design::input(
		ui,
		edit.id(egui::Id::unique(id))
			.char_limit(limit)
			.font(egui::FontId::proportional(15.0))
			.text_color(colors.text),
	)
	.labelled_by(label_id);
}

#[allow(clippy::too_many_arguments)]
fn preview(
	ui: &mut egui::Ui,
	draft: &Draft,
	profile: &UserProfile,
	avatars: &mut Avatars,
	media: (bool, &[model::Guild]),
	opening: &mut Option<String>,
	picture: Picture<'_>,
	clickable: bool,
) -> Option<AvatarAction> {
	let (demo, guilds) = media;
	let colors = design::palette(ui);
	let mut action = None;
	ui.label(design::eyebrow(ui, "Preview", colors.muted));
	ui.add_space(4.0);
	egui::Frame::new()
		.fill(colors.raised)
		.corner_radius(8)
		.stroke(egui::Stroke::new(1.0, colors.border))
		.show(ui, |ui| {
			ui.set_width(ui.available_width());
			ui.spacing_mut().item_spacing.y = 0.0;
			let (banner, _) = ui
				.allocate_exact_size(egui::vec2(ui.available_width(), 90.0), egui::Sense::hover());
			let corner = egui::CornerRadius {
				nw: 8,
				ne: 8,
				sw: 0,
				se: 0,
			};
			if profile.banner.is_some() {
				avatars.paint_banner(ui, profile, banner, corner, demo);
			} else {
				let color = draft.color.map_or(colors.accent.gamma_multiply(0.4), |c| {
					egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
				});
				ui.painter().rect_filled(banner, corner, color);
			}
			let avatar = egui::Rect::from_min_size(
				banner.left_bottom() + egui::vec2(16.0, -40.0),
				egui::Vec2::splat(80.0),
			);
			ui.painter()
				.circle_filled(avatar.center(), 46.0, colors.raised);
			let response =
				ui.scope_builder(
					egui::UiBuilder::new().max_rect(avatar),
					|ui| match picture {
						Picture::Pending(texture) | Picture::Saved(texture) => ui.add(
							egui::Image::new(texture)
								.fit_to_exact_size(egui::Vec2::splat(80.0))
								.corner_radius(40.0),
						),
						Picture::Removed => design::avatar(ui, &profile.user.name, 80.0),
						Picture::Remote => avatars.with_avatar_animation(true, |avatars| {
							avatars.show_plain(ui, &profile.user, 80.0, demo)
						}),
					},
				);
			if clickable {
				let hit = ui.interact(
					avatar,
					ui.scope_id().with("change-avatar"),
					egui::Sense::click(),
				);
				let hit = hit.on_hover_text("Change profile picture");
				if hit.hovered() || hit.has_focus() {
					ui.painter().circle_filled(
						avatar.center(),
						40.0,
						egui::Color32::from_black_alpha(120),
					);
					icons::paint(
						ui.painter(),
						icons::Icon::Pencil,
						egui::Rect::from_center_size(avatar.center(), egui::Vec2::splat(22.0)),
						egui::Color32::WHITE,
					);
				}
				if hit.clicked() {
					action = Some(AvatarAction::Pick);
				}
			}
			drop(response);
			ui.add_space((avatar.bottom() + 12.0 - ui.cursor().top()).max(0.0));
			egui::Frame::new()
				.inner_margin(egui::Margin {
					left: 12,
					right: 12,
					top: 0,
					bottom: 12,
				})
				.show(ui, |ui| {
					egui::Frame::new()
						.fill(colors.chat)
						.corner_radius(8)
						.inner_margin(12)
						.show(ui, |ui| {
							ui.set_width(ui.available_width());
							ui.set_min_height(138.0);
							ui.spacing_mut().item_spacing.y = 4.0;
							let name = if draft.name.is_empty() {
								&profile.username
							} else {
								&draft.name
							};
							ui.add(
								egui::Label::new(
									design::semibold(ui, name, 20.0).color(colors.text_strong),
								)
								.wrap(),
							);
							ui.label(
								egui::RichText::new(&profile.username)
									.size(13.0)
									.color(colors.text),
							);
							if !draft.pronouns.is_empty() {
								ui.label(
									egui::RichText::new(&draft.pronouns)
										.size(12.0)
										.color(colors.muted),
								);
							}
							if !draft.bio.is_empty() {
								ui.add_space(8.0);
								ui.separator();
								ui.add_space(8.0);
								ui.label(design::eyebrow(ui, "About Me", colors.text_strong));
								let mut mentions = crate::profiles::ProfileSession::default();
								crate::markdown::Formatted::parse(&draft.bio).show_with_images(
									ui,
									opening,
									&[],
									None,
									&mut mentions,
									(avatars, demo, guilds),
								);
							}
						});
				});
		});
	action
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn draft_only_sends_changed_fields_and_can_clear_values() {
		let user = model::User {
			id: model::Id(1),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
			primary_guild: None,
		};
		let mut profile = crate::profiles::synthetic(&user, None);
		profile.global_name = Some("Before".into());
		profile.accent_color = Some(0x123456);
		let mut draft = Draft::from_profile(&profile);
		assert!(draft.changes(&profile) == ProfileEdit::default());
		draft.name.clear();
		draft.bio.clear();
		draft.color = None;
		let changes = draft.changes(&profile);
		assert_eq!(changes.global_name, Some(None));
		assert_eq!(changes.bio, Some(String::new()));
		assert_eq!(changes.accent_color, Some(None));
		assert_eq!(changes.pronouns, None);
		assert!(changes.valid());
		// A reload retains edited values but adopts remote changes to untouched fields.
		let before = Draft::from_profile(&profile);
		profile.pronouns = "she/her".into();
		draft.rebase(&before, &Draft::from_profile(&profile));
		assert_eq!(draft.pronouns, "she/her");
		assert!(draft.name.is_empty() && draft.bio.is_empty());
		assert_eq!(draft.changes(&profile).pronouns, None);
	}
	#[test]
	fn editor_saves_once_preserves_failed_draft_and_cancels_in_both_themes() {
		fn frame(
			ctx: &egui::Context,
			editor: &mut Editor,
			state: &mut State,
			avatars: &mut Avatars,
			width: f32,
			events: Vec<egui::Event>,
		) -> (Vec<Command>, Vec<(String, egui::Rect)>) {
			let mut commands = vec![];
			let mut output = ctx.run_ui(
				egui::RawInput {
					screen_rect: Some(egui::Rect::from_min_size(
						egui::Pos2::ZERO,
						egui::vec2(width, 1100.0),
					)),
					events,
					..Default::default()
				},
				|ui| editor.show(ui, state, avatars, &mut commands),
			);
			output.textures_delta.clear();
			fn text(shape: &egui::Shape, labels: &mut Vec<(String, egui::Rect)>) {
				match shape {
					egui::Shape::Text(text) => labels.push((
						text.galley.job.text.clone(),
						text.galley.rect.translate(text.pos.to_vec2()),
					)),
					egui::Shape::Vec(shapes) => {
						for shape in shapes {
							text(shape, labels);
						}
					}
					_ => {}
				}
			}
			let mut labels = vec![];
			for shape in output.shapes {
				text(&shape.shape, &mut labels);
			}
			(commands, labels)
		}
		for theme in [egui::ThemePreference::Dark, egui::ThemePreference::Light] {
			for width in [320.0, 720.0] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				let mut editor = Editor::default();
				let mut avatars = Avatars::default();
				let mut state = test_support::demo_state();
				let (commands, _) =
					frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				assert_eq!(commands.len(), 1);
				let Command::EditProfile {
					user,
					request,
					changes: None,
				} = commands.into_iter().next().unwrap()
				else {
					panic!("load expected")
				};
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::ProfileEdited {
						user,
						request,
						result: Ok(Box::new(crate::profiles::synthetic(
							state.user.as_ref().unwrap(),
							None,
						))),
					},
				});
				frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				ctx.memory_mut(|memory| {
					memory.request_focus(egui::Id::unique("profile-display-name"))
				});
				frame(
					&ctx,
					&mut editor,
					&mut state,
					&mut avatars,
					width,
					vec![egui::Event::Text(" Edited".into())],
				);
				let draft = editor.draft.as_ref().unwrap().clone();
				assert!(draft.name.contains("Edited"));
				let (_, labels) = frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				let save = labels
					.iter()
					.find(|(text, _)| text == "Save changes")
					.unwrap()
					.1;
				assert!(save.left() >= 0.0 && save.right() <= width);
				let mut sent = vec![];
				for pressed in [true, false] {
					let pos = save.center();
					let (commands, _) = frame(
						&ctx,
						&mut editor,
						&mut state,
						&mut avatars,
						width,
						vec![
							egui::Event::PointerMoved(pos),
							egui::Event::PointerButton {
								pos,
								button: egui::PointerButton::Primary,
								pressed,
								modifiers: egui::Modifiers::NONE,
							},
						],
					);
					sent.extend(commands);
				}
				assert_eq!(sent.len(), 1);
				assert!(
					frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![])
						.0
						.is_empty()
				);
				let Command::EditProfile {
					user,
					request,
					changes: Some(changes),
				} = sent.pop().unwrap()
				else {
					panic!("save expected")
				};
				assert_eq!(changes.global_name, Some(Some(draft.name.clone())));
				state.apply(client_core::Envelope {
					generation: state.generation,
					event: client_core::Event::ProfileEdited {
						user,
						request,
						result: Err(client_core::auth::Failure::Ambiguous),
					},
				});
				let (_, labels) = frame(&ctx, &mut editor, &mut state, &mut avatars, width, vec![]);
				assert!(editor.draft.as_ref() == Some(&draft));
				assert!(!state.can_save_own_profile());
				let cancel = labels
					.iter()
					.find(|(text, _)| text == "Cancel")
					.unwrap()
					.1
					.center();
				for pressed in [true, false] {
					assert!(
						frame(
							&ctx,
							&mut editor,
							&mut state,
							&mut avatars,
							width,
							vec![
								egui::Event::PointerMoved(cancel),
								egui::Event::PointerButton {
									pos: cancel,
									button: egui::PointerButton::Primary,
									pressed,
									modifiers: egui::Modifiers::NONE
								}
							]
						)
						.0
						.is_empty()
					);
				}
				assert!(
					editor.draft.as_ref()
						== Some(&Draft::from_profile(
							state.own_profile.data.as_ref().unwrap()
						))
				);
			}
		}
	}
}
