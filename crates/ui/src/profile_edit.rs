//! One bounded, session-only draft for the account's global profile.
use crate::{avatars::Avatars, design};
use client_core::{Command, State};
use model::{ProfileEdit, UserProfile};

#[derive(Clone, PartialEq, Eq)]
struct Draft {
	name: String,
	bio: String,
	pronouns: String,
	color: Option<u32>,
}
impl Draft {
	fn from_profile(profile: &UserProfile) -> Self {
		Self {
			name: profile.global_name.clone().unwrap_or_default(),
			bio: profile.bio.clone(),
			pronouns: profile.pronouns.clone(),
			color: profile.accent_color,
		}
	}
	fn changes(&self, profile: &UserProfile) -> ProfileEdit {
		let name = (!self.name.is_empty()).then(|| self.name.clone());
		ProfileEdit {
			global_name: (name != profile.global_name).then_some(name),
			bio: (self.bio != profile.bio).then(|| self.bio.clone()),
			pronouns: (self.pronouns != profile.pronouns).then(|| self.pronouns.clone()),
			accent_color: (self.color != profile.accent_color).then_some(self.color),
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

#[derive(Default)]
pub(super) struct Editor {
	generation: Option<u64>,
	draft: Option<Draft>,
	baseline: Option<Draft>,
	submitted: Option<u64>,
	saved: bool,
	preview_link: Option<String>,
}
impl Editor {
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
				self.draft = Some(Draft::from_profile(profile));
				self.saved = true;
			}
		}
		let colors = design::palette(ui);
		if state.demo {
			ui.label(
				egui::RichText::new("Offline preview · synthetic profile")
					.size(12.0)
					.color(colors.muted),
			);
		}
		if state.own_profile.loading {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.label("Loading your profile…");
			});
		}
		if let Some(error) = state.own_profile.error {
			design::notice(ui, design::Level::Error, error);
			if !state.own_profile.loading
				&& !state.own_profile.saving
				&& ui.button("Reload profile").clicked()
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
		let width = ui.available_width();
		if width >= 620.0 {
			ui.horizontal_top(|ui| {
				ui.spacing_mut().item_spacing.x = 24.0;
				ui.allocate_ui_with_layout(
					egui::vec2(width - 324.0, 0.0),
					egui::Layout::top_down(egui::Align::Min),
					|ui| {
						ui.add_enabled_ui(editable, |ui| form(ui, draft));
					},
				);
				ui.vertical(|ui| {
					ui.set_width(300.0);
					preview(
						ui,
						draft,
						profile,
						avatars,
						(state.demo, &state.guilds),
						&mut self.preview_link,
					);
				});
			});
		} else {
			ui.add_enabled_ui(editable, |ui| form(ui, draft));
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
			.inner_margin(egui::Margin::symmetric(12, 10))
			.show(ui, |ui| {
				ui.set_width(ui.available_width());
				ui.horizontal_wrapped(|ui| {
					ui.spacing_mut().item_spacing.x = 12.0;
					if state.own_profile.saving {
						ui.spinner();
						ui.label("Saving profile…");
					} else if self.saved {
						ui.colored_label(
							colors.positive,
							if state.demo {
								"Saved in preview"
							} else {
								"Profile saved"
							},
						);
					} else if changed {
						ui.label("You have unsaved changes.");
					}
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						let save = ui.add_enabled(
							changed && changes.valid() && state.can_save_own_profile(),
							egui::Button::new(
								design::medium(ui, "Save changes", 14.0).color(colors.accent_text),
							)
							.fill(colors.accent)
							.stroke(egui::Stroke::NONE)
							.corner_radius(4)
							.min_size(egui::vec2(112.0, 34.0)),
						);
						if save.clicked()
							&& let Some(command) = state.save_own_profile(changes)
						{
							self.submitted = Some(state.own_profile.request);
							self.saved = false;
							commands.push(command);
						}
						if ui
							.add_enabled(
								changed && editable,
								egui::Button::new(design::medium(ui, "Cancel", 14.0))
									.frame(false)
									.min_size(egui::vec2(64.0, 34.0)),
							)
							.clicked()
						{
							self.draft = state.own_profile.data.as_ref().map(Draft::from_profile);
							self.saved = false;
						}
					});
				});
			});
		if width < 620.0
			&& let (Some(draft), Some(profile)) = (&self.draft, &state.own_profile.data)
		{
			ui.add_space(16.0);
			preview(
				ui,
				draft,
				profile,
				avatars,
				(state.demo, &state.guilds),
				&mut self.preview_link,
			);
		}
		crate::markdown::confirm_external_link(ui.ctx(), &mut self.preview_link, true);
		if !state.demo && !state.gateway_connected {
			ui.weak("Reconnect to save your profile.");
		}
	}
}

fn form(ui: &mut egui::Ui, draft: &mut Draft) {
	let colors = design::palette(ui);
	ui.spacing_mut().item_spacing.y = 6.0;
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
	ui.label(design::eyebrow(ui, "Profile color", colors.muted));
	let mut enabled = draft.color.is_some();
	if design::switch(ui, "Custom color", None, &mut enabled).changed() {
		draft.color = enabled.then_some(design::DEFAULT_PRIMARY_RGB);
	}
	if let Some(color) = &mut draft.color {
		ui.horizontal(|ui| {
			ui.spacing_mut().interact_size = egui::vec2(48.0, 32.0);
			let mut rgb = [(*color >> 16) as u8, (*color >> 8) as u8, *color as u8];
			if design::color_edit(ui, &mut rgb)
				.on_hover_text("Choose profile color")
				.changed()
			{
				*color = (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]);
			}
		});
	}
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

fn preview(
	ui: &mut egui::Ui,
	draft: &Draft,
	profile: &UserProfile,
	avatars: &mut Avatars,
	media: (bool, &[model::Guild]),
	opening: &mut Option<String>,
) {
	let (demo, guilds) = media;
	let colors = design::palette(ui);
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
			ui.scope_builder(egui::UiBuilder::new().max_rect(avatar), |ui| {
				avatars.show(ui, &profile.user, 80.0, demo);
			});
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
								ui.scope(|ui| {
									ui.style_mut().text_styles.insert(
										egui::TextStyle::Body,
										egui::FontId::proportional(13.0),
									);
									crate::markdown::Formatted::parse(&draft.bio).show_with_images(
										ui,
										opening,
										&[],
										&mut None,
										(avatars, demo, guilds),
									);
								});
							}
						});
				});
		});
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
