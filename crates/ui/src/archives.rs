use client_core::{Command, State};
use model::archives::Kind;

#[derive(Default)]
pub struct ArchivesUi {
	pub focus: bool,
	/// Parent channel a new thread was requested for from this dialog.
	pub create_requested: Option<model::Id>,
	filter: String,
}

impl ArchivesUi {
	pub fn show(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		commands: &mut Vec<Command>,
		avatars: &mut crate::avatars::Avatars,
	) {
		let Some(view) = &state.archives else {
			return;
		};
		if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
			self.filter.clear();
			commands.push(state.clear_archives());
			return;
		}
		let parent = view.parent;
		let allowed = state.can_archive(parent, view.kind);
		let supports_private = state.channel(parent).is_some_and(|c| c.kind == 0);
		let mut request = None;
		let mut target = None;
		let mut active_target = None;
		let active = state.active_threads(parent);
		let mut create = false;
		let filter = self.filter.trim().to_lowercase();
		let matches = |name: &str| filter.is_empty() || name.to_lowercase().contains(&filter);
		let can_create = state.can_create_thread(parent);
		let action_pending = state.channel_action_pending();
		let response = crate::dialog::Dialog::new(
			"archived-threads",
			crate::i18n::translate("archives-show-threads"),
		)
		.icon(crate::icons::Icon::Thread)
		.width(520.0)
		.show_with_toolbar(
			ctx,
			|d| {
				d.content(|ui| {
					let colors = crate::design::palette(ui);
					let shown: Vec<_> = active
						.iter()
						.filter(|thread| matches(&thread.name))
						.collect();
					if !active.is_empty() {
						section(
							ui,
							&format!(
								"{} {}",
								shown.len(),
								crate::i18n::translate("archives-show-archives-active-threads")
							),
							&colors,
						);
						if shown.is_empty() {
							crate::dialog::hint(
								ui,
								"archives-show-no-active-thread-matches-this-search",
							);
						}
						egui::ScrollArea::vertical()
							.id_salt(("active-threads", parent))
							.max_height(236.0)
							.show(ui, |ui| {
								for thread in &shown {
									let card = thread_card(ui, state, thread, avatars, &colors);
									if card.clicked() {
										active_target = Some(thread.id);
									}
								}
							});
						ui.add_space(12.0);
					}
					ui.horizontal(|ui| {
						ui.label(
							crate::design::semibold(
								ui,
								crate::i18n::translate("archives-show-older-threads"),
								12.0,
							)
							.color(colors.muted),
						);
						ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
							if supports_private {
								let mut kind = view.kind;
								ui.add_enabled_ui(allowed && !view.loading, |ui| {
									egui::ComboBox::from_id_salt(("archive-kind", parent))
										.width(112.0)
										.selected_text(match kind {
											Kind::Public => "Public",
											Kind::JoinedPrivate => "Joined private",
											Kind::Private => "Private",
										})
										.show_ui(ui, |ui| {
											ui.selectable_value(&mut kind, Kind::Public, "Public");
											ui.selectable_value(
												&mut kind,
												Kind::JoinedPrivate,
												"Joined private",
											);
											ui.selectable_value(
												&mut kind,
												Kind::Private,
												"Private",
											);
										});
								});
								if kind != view.kind {
									request = Some((kind, None));
								}
							}
							let action = if view.error.is_some() {
								Some(("archives-action-retry", view.before))
							} else {
								view.page
									.as_ref()
									.and_then(|page| page.next)
									.map(|before| ("archives-action-older", Some(before)))
							};
							if let Some((label, before)) = action
								&& ui
									.add_enabled_ui(allowed && !view.loading, |ui| {
										crate::dialog::action(
											ui,
											label,
											crate::dialog::Action::Neutral,
										)
									})
									.inner
									.clicked()
							{
								request = Some((view.kind, before));
							}
						});
					});
					ui.add_space(6.0);
					if !allowed {
						crate::dialog::notice(
							ui,
							crate::dialog::Level::Warning,
							"archives-show-archives-are-unavailable-while-disconnected-or-without-channel-access",
						);
					}
					if let Some(error) = view.error {
						crate::dialog::notice(ui, crate::dialog::Level::Error, error);
					}
					if view.loading {
						ui.horizontal(|ui| {
							ui.spinner();
							ui.label(crate::i18n::translate(
								"archives-show-loading-older-threads",
							));
						});
					}
					if let Some(page) = &view.page {
						let older: Vec<_> = page
							.threads
							.iter()
							.filter(|thread| matches(&thread.name))
							.collect();
						egui::ScrollArea::vertical()
							.id_salt(("archive-page", view.request))
							.max_height(300.0)
							.show_rows(ui, CARD_HEIGHT + CARD_GAP, older.len(), |ui, range| {
								for thread in &older[range] {
									ui.push_id(thread.id, |ui| {
										ui.add_enabled_ui(allowed && !view.loading, |ui| {
											if thread_card(ui, state, thread, avatars, &colors)
												.clicked()
											{
												target = Some(thread.id);
											}
										});
									});
								}
							});
						if older.is_empty() && !view.loading {
							crate::dialog::hint(ui, "archives-show-no-older-threads-returned");
						}
					}
				});
			},
			|ui| {
				ui.horizontal(|ui| {
					let create_width = 88.0;
					let field_width = (ui.available_width() - create_width - 12.0).max(80.0);
					let field = ui
						.allocate_ui(egui::vec2(field_width, 0.0), |ui| {
							crate::dialog::input(
								ui,
								egui::TextEdit::singleline(&mut self.filter)
									.char_limit(100)
									.hint_text(crate::i18n::translate(
										"archives-show-search-for-thread-name",
									)),
							)
						})
						.inner;
					self.filter.shrink_to_fit();
					ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
						ui.add_enabled_ui(can_create && !action_pending, |ui| {
							create = crate::dialog::action(
								ui,
								"archives-show-create",
								crate::dialog::Action::Primary,
							)
							.on_disabled_hover_text(crate::i18n::translate(
								"archives-show-you-cannot-start-a-thread-in-this-channel",
							))
							.clicked();
						});
					});
					if field.changed() {
						ui.ctx().request_repaint();
					}
					if self.focus {
						field.request_focus();
						self.focus = false;
					}
				});
			},
		);
		if create {
			self.create_requested = Some(parent);
		}
		if response.close {
			self.filter.clear();
			commands.push(state.clear_archives());
		} else if let Some((kind, before)) = request {
			if let Some(command) = state.request_archives(parent, kind, before) {
				commands.push(command);
			}
		} else if let Some(target) = target
			&& let Some(command) = state.open_archived_thread(target)
		{
			commands.push(command);
		} else if let Some(target) = active_target {
			commands.push(state.clear_archives());
			if let Some(command) = state.select(target) {
				commands.push(command);
			}
		}
	}
}

const CARD_HEIGHT: f32 = 64.0;
const CARD_GAP: f32 = 8.0;

fn section(ui: &mut egui::Ui, label: &str, colors: &crate::design::Palette) {
	ui.label(crate::design::semibold(ui, label, 12.0).color(colors.muted));
	ui.add_space(6.0);
}

/// Discord-style thread card: bold name, who started it (when the starter message is in the
/// open channel) and relative activity. The whole card opens the thread.
fn thread_card(
	ui: &mut egui::Ui,
	state: &State,
	thread: &model::Channel,
	avatars: &mut crate::avatars::Avatars,
	colors: &crate::design::Palette,
) -> egui::Response {
	let (rect, response) = ui.allocate_exact_size(
		egui::vec2(ui.available_width(), CARD_HEIGHT),
		egui::Sense::click(),
	);
	let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
	if ui.is_rect_visible(rect) {
		let frame = crate::design::interactive_card_frame(ui, &response);
		ui.painter().add(frame.paint(rect));
		let inner = rect.shrink2(egui::vec2(14.0, 9.0));
		ui.scope_builder(
			egui::UiBuilder::new()
				.max_rect(inner)
				.layout(egui::Layout::top_down(egui::Align::Min)),
			|ui| {
				ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
				ui.add(
					egui::Label::new(
						crate::design::semibold(ui, thread.name.as_str(), 15.0)
							.color(colors.text_strong),
					)
					.truncate()
					.selectable(false),
				);
				ui.horizontal(|ui| {
					// The thread shares its id with its starter, so the open channel may hold it.
					if let Some(starter) = state.timeline.get(thread.id) {
						avatars.show_plain(ui, &starter.author, 16.0, state.demo);
						ui.add(
							egui::Label::new(
								egui::RichText::new(crate::i18n::translate(
									"archives-thread-card-started-by",
								))
								.size(12.5)
								.color(colors.muted),
							)
							.selectable(false),
						);
						let color = state
							.message_author_color(starter)
							.map_or(colors.text_strong, |rgb| {
								crate::design::role_name_color(rgb, colors.raised, colors.text)
							});
						ui.add(
							egui::Label::new(
								crate::design::medium(ui, state.message_author_name(starter), 12.5)
									.color(color),
							)
							.truncate()
							.selectable(false),
						);
						ui.add(
							egui::Label::new(
								egui::RichText::new("•").size(12.5).color(colors.muted),
							)
							.selectable(false),
						);
					}
					ui.add(
						egui::Label::new(
							egui::RichText::new(crate::timeline::thread_activity(thread))
								.size(12.5)
								.color(colors.muted),
						)
						.truncate()
						.selectable(false),
					);
				});
			},
		);
	}
	ui.add_space(CARD_GAP);
	response.on_hover_text(format!(
		"{} “{}”",
		crate::i18n::translate("archives-thread-card-open-thread"),
		thread.name
	))
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::{
		Channel, Id,
		archives::{Cursor, Page},
	};

	fn channel(id: u64, kind: u8, parent_id: Option<Id>) -> Channel {
		Channel {
			id: Id(id),
			guild: Some(Id(100)),
			parent_id,
			kind,
			position: 0,
			name: format!("Synthetic thread {id}"),
			recipients: vec![],
			last_message: None,
			member_list_id: None,
			tags: None,
			message_count: None,
			icon: None,
		}
	}
	fn frame(ctx: &egui::Context, key: Option<egui::Key>, draw: impl FnMut(&mut egui::Ui)) {
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(420.0, 480.0),
				)),
				events: key
					.into_iter()
					.map(|key| egui::Event::Key {
						key,
						physical_key: None,
						pressed: true,
						repeat: false,
						modifiers: egui::Modifiers::NONE,
					})
					.collect(),
				..Default::default()
			},
			draw,
		);
		assert!(output.platform_output.commands.is_empty());
		output.drop_without_applying_deltas();
	}
	#[test]
	fn forum_archive_is_keyboard_accessible_paginates_and_opens_only_history() {
		for dark in [false, true] {
			let ctx = egui::Context::default();
			ctx.set_visuals(if dark {
				egui::Visuals::dark()
			} else {
				egui::Visuals::light()
			});
			let mut state = State {
				user: Some(model::User {
					id: Id(2),
					name: "Synthetic member".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
					primary_guild: None,
				}),
				auth: client_core::auth::AuthState::Authenticated,
				gateway_connected: true,
				guilds: vec![model::Guild {
					stickers: None,
					id: Id(100),
					name: "Synthetic".into(),
					icon: None,
					emojis: None,
				}],
				channels: vec![channel(7, 15, None)],
				..State::default()
			};
			state
				.permissions
				.replace(test_support::permission_snapshot(&state))
				.unwrap();
			let mut ui = crate::MessagingUi {
				guild: Some(Id(100)),
				..Default::default()
			};
			frame(&ctx, None, |root| {
				assert!(ui.channel_list(root, &mut state).is_none());
			});
			assert!(ui.archive_parent.is_none());
			// The forum row is now a destination; the header Threads control opens archives.
			for key in [egui::Key::Tab, egui::Key::Enter] {
				frame(&ctx, Some(key), |root| {
					if ui.channel_list(root, &mut state) == Some(Id(7)) {
						ui.archive_parent = Some(Id(7));
					}
				});
			}
			assert_eq!(ui.archive_parent.take(), Some(Id(7)));
			assert!(state.selected.is_none());
			let mut commands = vec![state.request_archives(Id(7), Kind::Public, None).unwrap()];
			ui.archives.focus = true;
			for _ in 0..2 {
				frame(&ctx, None, |_| {
					ui.archives
						.show(&ctx, &mut state, &mut commands, &mut ui.avatars)
				});
			}
			assert_eq!(commands.len(), 1); // Opening/loading never submits another request.
			let request = state.archives.as_ref().unwrap().request;
			let before = Cursor::Time(1_700_000_000_000_000_000);
			state.apply_archives(
				Id(7),
				request,
				Ok(Page {
					threads: vec![channel(8, 11, Some(Id(7)))],
					next: Some(before),
				}),
			);
			for key in [
				None,
				Some(egui::Key::Tab),
				Some(egui::Key::Tab),
				Some(egui::Key::Enter),
			] {
				frame(&ctx, key, |_| {
					ui.archives
						.show(&ctx, &mut state, &mut commands, &mut ui.avatars)
				});
			}
			assert!(
				matches!(commands.last(), Some(Command::Archives { before: Some(cursor), .. }) if *cursor == before)
			);
			assert!(state.archives.as_ref().unwrap().page.is_none());
			let request = state.archives.as_ref().unwrap().request;
			state.apply_archives(
				Id(7),
				request,
				Ok(Page {
					threads: vec![channel(9, 11, Some(Id(7)))],
					next: None,
				}),
			);
			ui.archives.focus = true;
			// Search, Create, Close, Open thread: exhausted pages have no Older control.
			for key in [
				None,
				Some(egui::Key::Tab),
				Some(egui::Key::Tab),
				Some(egui::Key::Enter),
			] {
				frame(&ctx, key, |_| {
					ui.archives
						.show(&ctx, &mut state, &mut commands, &mut ui.avatars)
				});
			}
			assert!(matches!(
				commands.last(),
				Some(Command::History { channel: Id(9), .. })
			));
			assert_eq!(state.selected, Some(Id(9)));
			assert!(state.archives.is_none());
			state.request_archives(Id(7), Kind::Public, None).unwrap();
			let request = state.archives.as_ref().unwrap().request;
			state.apply_archives(
				Id(7),
				request,
				Ok(Page {
					threads: vec![],
					next: None,
				}),
			);
			let count = commands.len();
			frame(&ctx, None, |_| {
				ui.archives
					.show(&ctx, &mut state, &mut commands, &mut ui.avatars)
			});
			assert_eq!(commands.len(), count);
			frame(&ctx, Some(egui::Key::Escape), |_| {
				ui.archives
					.show(&ctx, &mut state, &mut commands, &mut ui.avatars)
			});
			assert!(state.archives.is_none());
			assert!(matches!(commands.last(), Some(Command::CancelSearch)));
		}
	}
}
