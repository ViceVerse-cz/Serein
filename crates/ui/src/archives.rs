use client_core::{Command, State};
use model::archives::Kind;

#[derive(Default)]
pub struct ArchivesUi {
	pub focus: bool,
}

impl ArchivesUi {
	pub fn show(&mut self, ctx: &egui::Context, state: &mut State, commands: &mut Vec<Command>) {
		let Some(view) = &state.archives else {
			return;
		};
		if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
			commands.push(state.clear_archives());
			return;
		}
		let parent = view.parent;
		let allowed = state.can_archive(parent, view.kind);
		let parent_channel = state.channels.iter().find(|c| c.id == parent);
		let private = parent_channel.is_some_and(|c| c.kind == 0);
		let mut close = false;
		let mut request = None;
		let mut target = None;
		let mut active_target = None;
		let active = state.active_threads(parent);
		let response = crate::dialog::Dialog::new("archived-threads", "Threads")
			.subtitle(
				"Active threads come from the session; older threads load one page of up to 25 at a time. Opening loads messages; it does not join or reopen a thread.",
			)
			.width(460.0)
			.show(ctx, |d| {
				d.content(|ui| {
					let colors = crate::design::palette(ui);
					ui.add(
						egui::Label::new(
							crate::design::semibold(
								ui,
								parent_channel.map_or("Unavailable channel", |c| c.name.as_str()),
								15.0,
							)
							.color(colors.text_strong),
						)
						.truncate(),
					);
					ui.add_space(10.0);
					if !active.is_empty() {
						ui.label(
							crate::design::semibold(ui, "Active threads", 12.0).color(colors.muted),
						);
						ui.add_space(4.0);
						egui::ScrollArea::vertical()
							.id_salt(("active-threads", parent))
							.max_height(200.0)
							.show(ui, |ui| {
								for thread in &active {
									let row = thread_row(ui, thread, &colors);
									if row.clicked() {
										active_target = Some(thread.id);
									}
								}
							});
						ui.add_space(10.0);
					}
					ui.label(crate::design::semibold(ui, "Older threads", 12.0).color(colors.muted));
					ui.add_space(4.0);
					ui.horizontal_wrapped(|ui| {
						for (kind, name) in [
							(Kind::Public, "Public"),
							(Kind::JoinedPrivate, "Joined private"),
							(Kind::Private, "Private"),
						] {
							if (kind == Kind::Public || private)
								&& ui
									.add_enabled(
										allowed && !view.loading,
										egui::Button::selectable(view.kind == kind, name),
									)
									.clicked() && kind != view.kind
							{
								request = Some((kind, None));
							}
						}
						let reload = ui.add_enabled(allowed, egui::Button::new("Reload"));
						if self.focus {
							reload.request_focus();
							self.focus = false;
						}
						if reload.clicked() {
							request = Some((view.kind, None));
						}
						if view.error.is_some() {
							if ui
								.add_enabled(allowed && !view.loading, egui::Button::new("Retry"))
								.clicked()
							{
								request = Some((view.kind, view.before));
							}
						} else if let Some(before) = view.page.as_ref().and_then(|page| page.next)
							&& ui
								.add_enabled(allowed && !view.loading, egui::Button::new("Older"))
								.clicked()
						{
							request = Some((view.kind, Some(before)));
						}
					});
					ui.add_space(8.0);
					if view.kind == Kind::Private {
						crate::dialog::notice(
							ui,
							crate::dialog::Level::Info,
							"Private archives require permission from the service.",
						);
					}
					if !allowed {
						crate::dialog::notice(
							ui,
							crate::dialog::Level::Warning,
							"Archives are unavailable while disconnected or without channel access.",
						);
					}
					if let Some(error) = view.error {
						crate::dialog::notice(ui, crate::dialog::Level::Error, error);
					}
					if view.loading {
						ui.horizontal(|ui| {
							ui.spinner();
							ui.label("Loading archived threads…");
						});
					}
					if let Some(page) = &view.page {
						crate::dialog::hint(
							ui,
							&format!(
								"{} threads · {} page",
								page.threads.len(),
								if view.before.is_some() { "older" } else { "newest" }
							),
						);
						if page.threads.is_empty() {
							ui.label("No archived threads returned.");
						}
						egui::ScrollArea::vertical()
							.id_salt(("archive-page", view.request))
							.max_height(320.0)
							.show_rows(ui, 42.0, page.threads.len(), |ui, range| {
								for thread in &page.threads[range] {
									ui.push_id(thread.id, |ui| {
										ui.set_height(42.0);
										ui.horizontal(|ui| {
											if ui
												.add_enabled(
													allowed && !view.loading,
													egui::Button::new("Open thread"),
												)
												.clicked()
											{
												target = Some(thread.id);
											}
											ui.vertical(|ui| {
												ui.spacing_mut().item_spacing.y = 1.0;
												ui.add(egui::Label::new(&thread.name).truncate());
												ui.add(
													egui::Label::new(
														egui::RichText::new(
															crate::timeline::thread_activity(thread),
														)
														.size(12.0)
														.color(colors.muted),
													)
													.truncate(),
												);
											});
										});
									});
								}
							});
						if page.next.is_none() && !view.loading {
							crate::dialog::hint(
								ui,
								"No older threads reported by the service.",
							);
						}
					}
				});
				d.footer(|ui| {
					close |= crate::dialog::action(ui, "Close", crate::dialog::Action::Primary)
						.clicked();
				});
			});
		if response.close || close {
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

/// One active-thread row: name, message count and last activity; the whole row opens it.
fn thread_row(
	ui: &mut egui::Ui,
	thread: &model::Channel,
	colors: &crate::design::Palette,
) -> egui::Response {
	let (rect, response) =
		ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::click());
	let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
	if ui.is_rect_visible(rect) {
		let hovered = response.hovered() || response.has_focus();
		let painter = ui.painter();
		if hovered {
			painter.rect_filled(rect, 6.0, colors.hover);
		}
		let icon = egui::Rect::from_center_size(
			egui::pos2(rect.left() + 18.0, rect.center().y),
			egui::Vec2::splat(18.0),
		);
		crate::icons::paint(painter, crate::icons::Icon::Threads, icon, colors.muted);
		let left = rect.left() + 38.0;
		let width = (rect.right() - 8.0 - left).max(40.0);
		for (text, y, size, color) in [
			(
				thread.name.clone(),
				rect.top() + 6.0,
				14.0,
				colors.text_strong,
			),
			(
				crate::timeline::thread_activity(thread),
				rect.top() + 25.0,
				12.0,
				colors.muted,
			),
		] {
			let galley = egui::WidgetText::from(egui::RichText::new(text).size(size).color(color))
				.into_galley(
					ui,
					Some(egui::TextWrapMode::Truncate),
					width,
					egui::FontSelection::Default,
				);
			painter.galley(egui::pos2(left, y), galley, color);
		}
	}
	response.on_hover_text(format!("Open thread “{}”", thread.name))
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
					ui.archives.show(&ctx, &mut state, &mut commands)
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
			for key in [None, Some(egui::Key::Tab), Some(egui::Key::Enter)] {
				frame(&ctx, key, |_| {
					ui.archives.show(&ctx, &mut state, &mut commands)
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
			// Reload, Open thread: exhausted pages have no Older control.
			for key in [None, None, Some(egui::Key::Tab), Some(egui::Key::Enter)] {
				frame(&ctx, key, |_| {
					ui.archives.show(&ctx, &mut state, &mut commands)
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
				ui.archives.show(&ctx, &mut state, &mut commands)
			});
			assert_eq!(commands.len(), count);
			frame(&ctx, Some(egui::Key::Escape), |_| {
				ui.archives.show(&ctx, &mut state, &mut commands)
			});
			assert!(state.archives.is_none());
			assert!(matches!(commands.last(), Some(Command::CancelSearch)));
		}
	}
}
