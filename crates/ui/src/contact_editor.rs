use client_core::{Command, State};
use model::User;

#[derive(Default)]
pub(super) struct ContactEditor {
	user: Option<User>,
	nickname: bool,
	draft: String,
	loaded: bool,
	generation: u64,
}

impl ContactEditor {
	pub fn open(&mut self, user: User, nickname: bool, state: &mut State) -> Option<Command> {
		self.draft = if nickname {
			state.friend_nickname(user.id).unwrap_or("").to_owned()
		} else {
			String::new()
		};
		self.nickname = nickname;
		self.loaded = nickname;
		self.generation = state.generation;
		let command = if nickname {
			None
		} else {
			state.load_user_note(user.id)
		};
		self.user = Some(user);
		command
	}
	pub fn show(&mut self, ctx: &egui::Context, state: &mut State, commands: &mut Vec<Command>) {
		if self.generation != state.generation {
			*self = Self::default();
			return;
		}
		let Some(user) = &self.user else {
			return;
		};
		let busy = state.user_action_pending();
		if !self.loaded
			&& !busy && state.user_action_status().is_none()
			&& let Some(note) = state.user_note(user.id)
		{
			self.draft = note.to_owned();
			self.loaded = true;
		}
		let mut close = false;
		let limit = if self.nickname { 32 } else { 256 };
		let ready = if self.nickname {
			state.friends().any(|friend| friend.id == user.id)
		} else {
			state.user_note(user.id).is_some()
		};
		let response = crate::dialog::Dialog::new(
			"contact-editor",
			crate::i18n::translate_if_key(if self.nickname {
				"contact-editor-show-friend-nickname"
			} else {
				"contact-editor-show-note"
			}),
		)
		.subtitle(crate::i18n::translate_if_key(if self.nickname {
			"contact-editor-show-only-you-can-see-this-nickname-it-does-not-change"
		} else {
			"contact-editor-show-only-you-can-see-this-note-it-is-saved-to"
		}))
		.width(420.0)
		.show(ctx, |d| {
			d.content(|ui| {
				ui.spacing_mut().item_spacing.y = 10.0;
				let colors = crate::design::palette(ui);
				ui.label(crate::design::semibold(ui, &user.name, 15.0).color(colors.text_strong));
				if !self.loaded {
					if busy {
						pending_note(ui);
					} else {
						crate::dialog::notice(
							ui,
							crate::dialog::Level::Error,
							"contact-editor-show-could-not-load-the-note-your-existing-note-has-not",
						);
						if crate::dialog::action(
							ui,
							"contact-editor-show-retry",
							crate::dialog::Action::Neutral,
						)
						.clicked() && let Some(command) = state.load_user_note(user.id)
						{
							commands.push(command);
						}
					}
				} else {
					let label = crate::dialog::label(
						ui,
						if self.nickname {
							"contact-editor-show-nickname"
						} else {
							"contact-editor-show-note"
						},
					);
					ui.add_enabled_ui(!busy, |ui| {
						let edit = if self.nickname {
							egui::TextEdit::singleline(&mut self.draft)
								.align(egui::Align2::LEFT_CENTER)
						} else {
							egui::TextEdit::multiline(&mut self.draft).desired_rows(5)
						};
						crate::dialog::input(
							ui,
							edit.char_limit(limit)
								.hint_text(crate::i18n::translate_if_key(
									&(if self.nickname {
										crate::i18n::translate(
											"contact-editor-show-enter-a-nickname",
										)
									} else {
										crate::i18n::translate(
											"contact-editor-show-add-something-to-remember",
										)
									}),
								)),
						)
						.labelled_by(label.id);
					});
					crate::dialog::hint(
						ui,
						&format!(
							"{} / {limit} · Leave empty to remove",
							self.draft.chars().count()
						),
					);
				}
				if self.loaded && !ready {
					crate::dialog::notice(
						ui,
						crate::dialog::Level::Warning,
						if self.nickname {
							"contact-editor-show-this-user-is-no-longer-a-confirmed-friend"
						} else {
							"contact-editor-show-connection-refreshed-reload-the-saved-note-before-saving-your-draft"
						},
					);
					if !self.nickname
						&& ui
							.add_enabled_ui(!busy, |ui| {
								crate::dialog::action(
									ui,
									"contact-editor-show-reload-saved-note",
									crate::dialog::Action::Neutral,
								)
							})
							.inner
							.clicked() && let Some(command) = state.load_user_note(user.id)
					{
						commands.push(command);
					}
				}
			});
			d.footer(|ui| {
				let valid =
					client_core::user_actions::valid_personal_text(&self.draft, self.nickname);
				let saved = if self.nickname {
					state.friend_nickname(user.id).unwrap_or("")
				} else {
					state.user_note(user.id).unwrap_or("")
				};
				ui.add_enabled_ui(
					self.loaded && ready && valid && !busy && saved != self.draft,
					|ui| {
						if crate::dialog::action(
							ui,
							if busy {
								"contact-editor-show-saving"
							} else {
								"contact-editor-show-save"
							},
							crate::dialog::Action::Primary,
						)
						.clicked()
						{
							let command = if self.nickname {
								state.set_friend_nickname(user.id, self.draft.clone())
							} else {
								state.set_user_note(user.id, self.draft.clone())
							};
							if let Some(command) = command {
								commands.push(command);
							}
						}
					},
				);
				ui.add_enabled_ui(!busy, |ui| {
					close |= crate::dialog::action(
						ui,
						"contact-editor-show-cancel",
						crate::dialog::Action::Neutral,
					)
					.clicked();
				});
			});
		});
		if close || (!state.user_action_pending() && response.close) {
			*self = Self::default();
		}
	}
}

/// The note editor while the note loads: its label, the five-row input and the counter line
/// under it, at the loaded editor's sizes.
fn pending_note(ui: &mut egui::Ui) {
	use crate::skeleton::{CONTROL_RADIUS, bar, control};
	use egui::{Rect, pos2, vec2};
	let spacing = ui.spacing().item_spacing.y;
	let row = ui.text_style_height(&egui::TextStyle::Body) + ui.spacing().extra_text_line_spacing;
	let (label, hint) = ui.fonts_mut(|fonts| {
		(
			fonts
				.row_height(&egui::FontId::new(
					12.0,
					crate::design::semibold_family(ui.ctx()),
				))
				.round(),
			fonts.row_height(&egui::FontId::proportional(12.0)).round(),
		)
	});
	// Labels take whole pixels and add 6px under themselves; the input pads its five rows by
	// 9px each side.
	let input_top = label + spacing + 6.0;
	let input = 5.0 * row + 18.0;
	let hint_top = input_top + input + spacing + 4.0;
	crate::skeleton::custom(
		ui,
		&crate::i18n::translate("contact-editor-show-loading-note"),
		vec2(ui.available_width(), hint_top + hint),
		|painter, rect, fill| {
			bar(
				painter,
				fill,
				pos2(rect.left(), rect.top() + (label - 10.0) * 0.5),
				vec2(40.0, 10.0),
			);
			control(
				painter,
				fill,
				Rect::from_min_size(
					pos2(rect.left(), rect.top() + input_top),
					vec2(rect.width(), input),
				),
				CONTROL_RADIUS,
			);
			bar(
				painter,
				fill,
				pos2(rect.left(), rect.top() + hint_top + (hint - 10.0) * 0.5),
				vec2(rect.width().min(180.0), 10.0),
			);
		},
	);
}

#[cfg(test)]
mod tests {
	use super::*;
	use client_core::{Envelope, Event, user_actions};
	#[test]
	fn editors_load_before_edit_keep_failed_drafts_and_clear_on_account_change() {
		for nickname in [false, true] {
			for theme in [egui::ThemePreference::Light, egui::ThemePreference::Dark] {
				let ctx = egui::Context::default();
				ctx.set_theme(theme);
				crate::design::apply(&ctx);
				let mut state = test_support::demo_state();
				let user = state
					.channels
					.iter()
					.find_map(|c| c.recipients.first())
					.unwrap()
					.clone();
				state.apply(Envelope {
					generation: state.generation,
					event: Event::UserAction(user_actions::Event::Friends(Some(vec![(
						user.clone(),
						"synthetic".into(),
					)]))),
				});
				let mut editor = ContactEditor::default();
				let command = editor.open(user.clone(), nickname, &mut state);
				if let Some(Command::UserAction { request, .. }) = command {
					assert!(!editor.loaded);
					state.apply(Envelope {
						generation: state.generation,
						event: Event::UserAction(user_actions::Event::NoteLoaded {
							user: user.id,
							request,
							result: Ok("Original note".into()),
						}),
					});
				}
				let render = |editor: &mut ContactEditor, state: &mut State| {
					let mut commands = vec![];
					ctx.run_ui(
						egui::RawInput {
							screen_rect: Some(egui::Rect::from_min_size(
								egui::Pos2::ZERO,
								egui::vec2(320.0, 600.0),
							)),
							..Default::default()
						},
						|_| editor.show(&ctx, state, &mut commands),
					)
					.drop_without_applying_deltas();
					assert!(commands.is_empty(), "Rendering never saves automatically");
				};
				render(&mut editor, &mut state);
				assert!(editor.loaded);
				editor.draft = "Private draft 🌙".into();
				let command = if nickname {
					state.set_friend_nickname(user.id, editor.draft.clone())
				} else {
					state.set_user_note(user.id, editor.draft.clone())
				}
				.unwrap();
				state.command_rejected(command);
				render(&mut editor, &mut state);
				assert_eq!(editor.draft, "Private draft 🌙");
				state.generation += 1;
				render(&mut editor, &mut state);
				assert!(editor.user.is_none() && editor.draft.is_empty());
			}
		}
	}
}
