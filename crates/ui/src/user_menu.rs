//! Shared user actions; rendering only records intent, dispatched after borrowed rows finish.
use crate::shortcuts::{Intent, ShortcutView};
use client_core::{Command, State};
use model::{Shortcut, User};

#[derive(Clone, PartialEq, Eq)]
pub enum Action {
	Note(User),
	Nickname(User),
	Mention(User),
	CloseDm(model::Id),
	Block { user: model::Id, blocked: bool },
	Mute { channel: model::Id, muted: bool },
	Shortcut(Intent),
}
impl std::fmt::Debug for Action {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("User menu action")
	}
}

pub(super) fn prepare(action: Action, state: &mut State) -> Option<Command> {
	match action {
		Action::Note(_) | Action::Nickname(_) | Action::Shortcut(_) | Action::Mention(_) => None,
		Action::CloseDm(channel) => state.close_dm(channel),
		Action::Block { user, blocked } => state.set_user_blocked(user, blocked),
		Action::Mute { channel, muted } => state.set_dm_muted(channel, muted),
	}
}

pub(super) fn popup(response: &egui::Response, id: egui::Id) -> egui::Popup<'_> {
	let keyboard = response.has_focus()
		&& response
			.ctx
			.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::F10));
	let passive = !response.sense.senses_click();
	let pointer_opened = if passive {
		response.container_secondary_clicked()
	} else {
		response.secondary_clicked()
	};
	let mut popup = if passive {
		egui::Popup::menu(response)
			.open_memory(if pointer_opened {
				Some(egui::SetOpenCommand::Bool(true))
			} else if response.container_clicked() {
				Some(egui::SetOpenCommand::Bool(false))
			} else {
				None
			})
			.at_pointer_fixed()
	} else {
		egui::Popup::context_menu(response)
	}
	.id(id);
	if keyboard {
		popup = popup
			.open_memory(Some(egui::SetOpenCommand::Bool(true)))
			.at_position(response.rect.right_bottom());
	} else if !pointer_opened && egui::Popup::position_of_id(&response.ctx, id).is_none() {
		// Keyboard-opened menus have no remembered pointer position.
		popup = popup.at_position(response.rect.right_bottom());
	}
	popup
}

pub(super) fn show(
	response: &egui::Response,
	state: &State,
	user: &User,
	profile: &mut crate::profiles::ProfileSession,
	action: &mut Option<Action>,
) {
	show_with_pin(response, state, user, profile, action, None);
}

/// `view` is `Some` only where the row is a direct message, so Pin DM stays off member lists.
pub(super) fn show_with_pin(
	response: &egui::Response,
	state: &State,
	user: &User,
	profile: &mut crate::profiles::ProfileSession,
	action: &mut Option<Action>,
	view: Option<ShortcutView<'_>>,
) {
	popup(response, egui::Popup::default_response_id(response))
		.show(|ui| contents(ui, state, user, profile, action, view));
}

pub(super) fn contents(
	ui: &mut egui::Ui,
	state: &State,
	user: &User,
	profile: &mut crate::profiles::ProfileSession,
	action: &mut Option<Action>,
	view: Option<ShortcutView<'_>>,
) {
	let colors = crate::design::palette(ui);
	ui.set_min_width(200.0);
	ui.spacing_mut().button_padding = egui::vec2(8.0, 6.0);
	if ui.button("Profile").clicked() {
		profile.command_open(user.clone());
		ui.close();
	}
	if !user.webhook
		&& state.selected.is_some_and(|id| {
			state
				.channel(id)
				.is_some_and(|channel| channel.supports_text())
		}) && ui.button("Mention").clicked()
	{
		*action = Some(Action::Mention(user.clone()));
		ui.close();
	}
	if user.webhook || state.user.as_ref().is_some_and(|own| own.id == user.id) {
		return;
	}
	let dm = state
		.channels
		.iter()
		.find(|c| c.guild.is_none() && c.kind == 1 && c.recipients.iter().any(|u| u.id == user.id));
	let enabled = (state.demo || state.gateway_connected) && !state.user_action_pending();
	ui.separator();
	if ui
		.add_enabled(enabled, egui::Button::new("Add Note"))
		.clicked()
	{
		*action = Some(Action::Note(user.clone()));
		ui.close();
	}
	if ui
		.add_enabled(
			enabled && state.friends().any(|friend| friend.id == user.id),
			egui::Button::new(if state.friend_nickname(user.id).is_some() {
				"Edit Friend Nickname"
			} else {
				"Add Friend Nickname"
			}),
		)
		.on_disabled_hover_text("Private nicknames are available for confirmed friends.")
		.clicked()
	{
		*action = Some(Action::Nickname(user.clone()));
		ui.close();
	}
	ui.separator();
	if let Some(dm) = dm {
		if let Some(view) = view {
			let pinned = view.contains(Shortcut::Pinned, dm.id);
			if ui
				.add_enabled(
					view.available(),
					egui::Button::new(if pinned { "Unpin DM" } else { "Pin DM" }),
				)
				.on_hover_text("Pinned direct messages are saved on this device.")
				.clicked()
			{
				*action = Some(Action::Shortcut(view.toggle(Shortcut::Pinned, dm.id)));
				ui.close();
			}
		}
		let muted = state.dm_muted(dm.id) == Some(true);
		if ui
			.add_enabled(
				enabled,
				egui::Button::new(if muted {
					"Unmute Conversation"
				} else {
					"Mute Conversation"
				}),
			)
			.on_hover_text("Mute this direct message's notifications until you unmute it.")
			.clicked()
		{
			*action = Some(Action::Mute {
				channel: dm.id,
				muted: !muted,
			});
			ui.close();
		}
		if ui
			.add_enabled(enabled, egui::Button::new("Close DM"))
			.on_hover_text("Remove this conversation from your DM list. Messages are kept.")
			.clicked()
		{
			*action = Some(Action::CloseDm(dm.id));
			ui.close();
		}
	} else {
		ui.add_enabled(false, egui::Button::new("Mute Conversation"))
			.on_disabled_hover_text("No open direct message with this user.");
	}
	ui.separator();
	let blocked = state.user_blocked(user.id) == Some(true);
	if ui
		.add_enabled(
			enabled,
			egui::Button::new(
				egui::RichText::new(if blocked { "Unblock" } else { "Block" }).color(colors.danger),
			),
		)
		.clicked()
	{
		*action = Some(Action::Block {
			user: user.id,
			blocked: !blocked,
		});
		ui.close();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::{Event, Modifiers, PointerButton, Pos2, Rect};

	fn labels(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
		match shape {
			egui::Shape::Text(t) => out.push((
				t.galley.job.text.clone(),
				t.galley.rect.translate(t.pos.to_vec2()),
			)),
			egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| labels(s, out)),
			_ => {}
		}
	}
	fn pointer(pos: Pos2, button: PointerButton, pressed: bool) -> Vec<Event> {
		vec![
			Event::PointerMoved(pos),
			Event::PointerButton {
				pos,
				button,
				pressed,
				modifiers: Modifiers::NONE,
			},
		]
	}
	fn frame(
		ctx: &egui::Context,
		state: &State,
		user: &User,
		events: Vec<Event>,
		profile: &mut crate::profiles::ProfileSession,
		action: &mut Option<Action>,
	) -> (egui::Response, Vec<(String, Rect)>) {
		let mut response = None;
		let mut output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 420.0))),
				events,
				..Default::default()
			},
			|ui| {
				let row = ui.button(&user.name);
				show(&row, state, user, profile, action);
				response = Some(row);
			},
		);
		output.textures_delta.clear();
		let mut text = vec![];
		for shape in output.shapes {
			labels(&shape.shape, &mut text);
		}
		(response.unwrap(), text)
	}

	#[test]
	fn user_menu_mouse_keyboard_and_actions_in_both_themes() {
		for light in [false, true] {
			for label in [
				"Profile",
				"Mention",
				"Mute Conversation",
				"Close DM",
				"Block",
			] {
				let ctx = egui::Context::default();
				ctx.set_visuals(if light {
					egui::Visuals::light()
				} else {
					egui::Visuals::dark()
				});
				let state = test_support::demo_state();
				let dm = state.channels.iter().find(|c| c.kind == 1).unwrap();
				let user = &dm.recipients[0];
				let (mut profile, mut action) = (crate::profiles::ProfileSession::default(), None);
				let (row, _) = frame(&ctx, &state, user, vec![], &mut profile, &mut action);
				if light {
					row.request_focus();
					frame(
						&ctx,
						&state,
						user,
						vec![Event::Key {
							key: egui::Key::F10,
							physical_key: None,
							pressed: true,
							repeat: false,
							modifiers: Modifiers::SHIFT,
						}],
						&mut profile,
						&mut action,
					);
				} else {
					for pressed in [true, false] {
						frame(
							&ctx,
							&state,
							user,
							pointer(row.rect.center(), PointerButton::Secondary, pressed),
							&mut profile,
							&mut action,
						);
					}
				}
				let (_, text) = frame(&ctx, &state, user, vec![], &mut profile, &mut action);
				assert!(profile.open_user().is_none() && action.is_none());
				for expected in [
					"Profile",
					"Mention",
					"Add Note",
					"Add Friend Nickname",
					"Mute Conversation",
					"Close DM",
					"Block",
				] {
					let rect = text
						.iter()
						.find(|(s, _)| s == expected)
						.unwrap_or_else(|| panic!("Missing {expected}: {text:?}"))
						.1;
					assert!(
						Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 420.0))
							.contains_rect(rect)
					);
				}
				let pos = text.iter().find(|(s, _)| s == label).unwrap().1.center();
				for pressed in [true, false] {
					frame(
						&ctx,
						&state,
						user,
						pointer(pos, PointerButton::Primary, pressed),
						&mut profile,
						&mut action,
					);
				}
				match label {
					"Profile" => assert_eq!(profile.open_user().unwrap().id, user.id),
					"Mention" => assert_eq!(action, Some(Action::Mention(user.clone()))),
					"Mute Conversation" => assert_eq!(
						action,
						Some(Action::Mute {
							channel: dm.id,
							muted: true
						})
					),
					"Close DM" => assert_eq!(action, Some(Action::CloseDm(dm.id))),
					_ => assert_eq!(
						action,
						Some(Action::Block {
							user: user.id,
							blocked: true
						})
					),
				}
				assert!(!egui::Popup::is_any_open(&ctx));
			}
		}
	}

	#[test]
	fn dm_row_and_avatar_open_menu_without_navigation_and_dispatch_once() {
		for avatar in [false, true] {
			let ctx = egui::Context::default();
			let mut view = crate::MessagingUi::default();
			let mut state = test_support::demo_state();
			let selected = state.selected;
			let render = |view: &mut crate::MessagingUi, state: &mut State, events| {
				let mut commands = vec![];
				let mut output = ctx.run_ui(
					egui::RawInput {
						screen_rect: Some(Rect::from_min_size(
							Pos2::ZERO,
							egui::vec2(1000.0, 700.0),
						)),
						events,
						..Default::default()
					},
					|ui| {
						view.channel_list(ui, state);
						if let Some(action) = view.user_action.take()
							&& let Some(command) = prepare(action, state)
						{
							commands.push(command);
						}
					},
				);
				output.textures_delta.clear();
				let mut text = vec![];
				for shape in output.shapes {
					labels(&shape.shape, &mut text);
				}
				(commands, text)
			};
			render(&mut view, &mut state, vec![]);
			let (_, text) = render(&mut view, &mut state, vec![]);
			let name = text
				.iter()
				.find(|(s, _)| s == "Robin (synthetic)")
				.unwrap()
				.1;
			let pos = if avatar {
				egui::pos2(name.left() - 28.0, name.center().y)
			} else {
				name.center()
			};
			for pressed in [true, false] {
				let (commands, _) = render(
					&mut view,
					&mut state,
					pointer(pos, PointerButton::Secondary, pressed),
				);
				assert!(commands.is_empty());
			}
			let (_, text) = render(&mut view, &mut state, vec![]);
			assert_eq!(state.selected, selected);
			assert!(view.profile.open_user().is_none());
			let pos = text
				.iter()
				.find(|(s, _)| s == "Close DM")
				.unwrap()
				.1
				.center();
			let mut writes = 0;
			for pressed in [true, false] {
				let (commands, _) = render(
					&mut view,
					&mut state,
					pointer(pos, PointerButton::Primary, pressed),
				);
				writes += commands
					.iter()
					.filter(|c| {
						matches!(
							c,
							Command::UserAction {
								action: client_core::user_actions::Action::CloseDm(model::Id(22)),
								..
							}
						)
					})
					.count();
			}
			assert_eq!(writes, 1);
			assert!(
				state.channels.iter().any(|c| c.id == model::Id(22)),
				"Wait for transport confirmation"
			);
			assert!(render(&mut view, &mut state, vec![]).0.is_empty());
		}
	}
}
