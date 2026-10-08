//! Shared user actions; rendering only records intent, dispatched after borrowed rows finish.
use crate::shortcuts::{Intent, ShortcutView};
use client_core::{Command, State};
use model::{Shortcut, User};

#[derive(Clone, PartialEq, Eq)]
pub enum Action {
	Note(User),
	Nickname(User),
	Mention(User),
	Message(User),
	StartCall(User),
	AddFriend(User),
	AcceptFriend(model::Id),
	CloseDm(model::Id),
	Block { user: model::Id, blocked: bool },
	Ignore { user: model::Id, ignored: bool },
	Mute { channel: model::Id, muted: bool },
	MessageRequest { channel: model::Id, accept: bool },
	Shortcut(Intent),
}
impl std::fmt::Debug for Action {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("User menu action")
	}
}

pub(super) fn prepare(action: Action, state: &mut State) -> Option<Command> {
	match action {
		Action::Note(_)
		| Action::Nickname(_)
		| Action::Shortcut(_)
		| Action::Mention(_)
		| Action::Message(_)
		| Action::StartCall(_) => None,
		Action::AddFriend(user) => state.add_profile_friend(user.id),
		Action::AcceptFriend(user) => state.resolve_friend_request(user, true),
		Action::CloseDm(channel) => state.close_dm(channel),
		Action::Block { user, blocked } => state.set_user_blocked(user, blocked),
		Action::Ignore { user, ignored } => state.set_user_ignored(user, ignored),
		Action::Mute { channel, muted } => state.set_dm_muted(channel, muted),
		Action::MessageRequest { channel, accept } => {
			state.resolve_message_request(channel, accept)
		}
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

const VOICE_AVAILABLE: &str = "user-menu-voice-available";

/// Publishes this frame's voice availability so every menu caller reflects it without threading it.
pub(super) fn set_voice_available(ctx: &egui::Context, available: bool) {
	ctx.data_mut(|data| data.insert_temp(egui::Id::unique(VOICE_AVAILABLE), available));
}

fn voice_available(ui: &egui::Ui) -> bool {
	ui.data(|data| data.get_temp::<bool>(egui::Id::unique(VOICE_AVAILABLE))) == Some(true)
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
	if ui
		.button(crate::i18n::translate("user-menu-contents-profile"))
		.clicked()
	{
		profile.command_open(user.clone());
		ui.close();
	}
	if !user.webhook
		&& ui
			.button(crate::i18n::translate("profiles-view-full-profile"))
			.clicked()
	{
		profile.open_full(user.clone());
		ui.close();
	}
	if !user.webhook
		&& state.selected.is_some_and(|id| {
			state
				.channel(id)
				.is_some_and(|channel| channel.supports_text())
		}) && ui
		.button(crate::i18n::translate("user-menu-contents-mention"))
		.clicked()
	{
		*action = Some(Action::Mention(user.clone()));
		ui.close();
	}
	if !user.webhook {
		crate::profiles::copy_username_button(ui, state, user);
	}
	if user.webhook || state.user.as_ref().is_some_and(|own| own.id == user.id) {
		return;
	}
	let dm = state
		.channels
		.iter()
		.find(|c| c.guild.is_none() && c.kind == 1 && c.recipients.iter().any(|u| u.id == user.id));
	let enabled = !state.user_action_pending()
		&& (state.demo
			|| (state.gateway_connected
				&& state.auth == client_core::auth::AuthState::Authenticated));
	let reachable = user.id.0 != 0
		&& state.user.as_ref().is_some_and(|own| own.id != user.id)
		&& state.user_blocked(user.id) == Some(false);
	if ui
		.add_enabled(
			state.can_open_user_dm(user),
			egui::Button::new(crate::i18n::translate("friends-message")),
		)
		.clicked()
	{
		*action = Some(Action::Message(user.clone()));
		ui.close();
	}
	if ui
		.add_enabled(
			state.can_open_user_dm(user)
				&& enabled && !state.demo
				&& voice_available(ui)
				&& state
					.voice
					.active
					.as_ref()
					.is_none_or(|call| dm.is_none_or(|channel| call.channel != channel.id))
				&& dm.is_none_or(|channel| state.can_call(channel.id)),
			egui::Button::new(crate::i18n::translate("user-menu-contents-start-a-call")),
		)
		.clicked()
	{
		*action = Some(Action::StartCall(user.clone()));
		ui.close();
	}
	if user.kind == model::AccountKind::Human && state.friend(user.id).is_none() {
		let request = state
			.pending_friends()
			.find(|(person, _, _)| person.id == user.id);
		let (label, friend_action) = if let Some((_, _, incoming)) = request {
			if *incoming {
				(
					"profiles-friend-action-accept",
					Some(Action::AcceptFriend(user.id)),
				)
			} else {
				("profiles-friend-action-sent", None)
			}
		} else if !state.friends_known() || !state.friend_requests_known() {
			("profiles-friend-action-loading", None)
		} else {
			("friends-add", Some(Action::AddFriend(user.clone())))
		};
		if ui
			.add_enabled(
				enabled
					&& reachable && state.friends_known()
					&& state.friend_requests_known()
					&& friend_action.is_some(),
				egui::Button::new(crate::i18n::translate(label)),
			)
			.clicked()
		{
			*action = friend_action;
			ui.close();
		}
	}
	ui.separator();
	if ui
		.add_enabled(
			enabled,
			egui::Button::new(crate::i18n::translate("user-menu-contents-add-note")),
		)
		.clicked()
	{
		*action = Some(Action::Note(user.clone()));
		ui.close();
	}
	if ui
		.add_enabled(
			enabled && state.friends().any(|friend| friend.id == user.id),
			egui::Button::new(crate::i18n::translate_if_key(
				&(if state.friend_nickname(user.id).is_some() {
					crate::i18n::translate("user-menu-contents-edit-friend-nickname")
				} else {
					crate::i18n::translate("user-menu-contents-add-friend-nickname")
				}),
			)),
		)
		.on_disabled_hover_text(crate::i18n::translate(
			"user-menu-contents-private-nicknames-are-available-for-confirmed-friends",
		))
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
					egui::Button::new(crate::i18n::translate_if_key(
						&(if pinned {
							crate::i18n::translate("user-menu-contents-unpin-dm")
						} else {
							crate::i18n::translate("user-menu-contents-pin-dm")
						}),
					)),
				)
				.on_hover_text(crate::i18n::translate(
					"user-menu-contents-pinned-direct-messages-are-saved-on-this-device",
				))
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
				egui::Button::new(crate::i18n::translate_if_key(
					&(if muted {
						crate::i18n::translate("user-menu-contents-unmute-conversation")
					} else {
						crate::i18n::translate("user-menu-contents-mute-conversation")
					}),
				)),
			)
			.on_hover_text(crate::i18n::translate(
				"user-menu-contents-mute-this-direct-message-s-notifications-until-you-unmute-it",
			))
			.clicked()
		{
			*action = Some(Action::Mute {
				channel: dm.id,
				muted: !muted,
			});
			ui.close();
		}
		if ui
			.add_enabled(
				enabled,
				egui::Button::new(crate::i18n::translate("user-menu-contents-close-dm")),
			)
			.on_hover_text(crate::i18n::translate(
				"user-menu-contents-remove-this-conversation-from-your-dm-list-messages-are-kept",
			))
			.clicked()
		{
			*action = Some(Action::CloseDm(dm.id));
			ui.close();
		}
	} else {
		ui.add_enabled(
			false,
			egui::Button::new(crate::i18n::translate(
				"user-menu-contents-mute-conversation",
			)),
		)
		.on_disabled_hover_text(crate::i18n::translate(
			"user-menu-contents-no-open-direct-message-with-this-user",
		));
	}
	ui.separator();
	if let Some(ignore) = crate::profiles::ignore_button(ui, state, user, enabled) {
		*action = Some(ignore);
	}
	let blocked = state.user_blocked(user.id) == Some(true);
	if ui
		.add_enabled(
			enabled,
			egui::Button::new(
				egui::RichText::new(crate::i18n::translate_if_key(if blocked {
					"user-menu-contents-unblock"
				} else {
					"user-menu-contents-block"
				}))
				.color(colors.danger),
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
	crate::profiles::report_button(ui, user);
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
				"Message",
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
					"Message",
					"Start a Call",
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
					"Message" => assert_eq!(action, Some(Action::Message(user.clone()))),
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

	fn apply_user_event(state: &mut State, event: client_core::user_actions::Event) {
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::UserAction(event),
		});
	}

	fn menu_labels(state: &State, user: &User) -> Vec<String> {
		let ctx = egui::Context::default();
		let (mut profile, mut action) = (crate::profiles::ProfileSession::default(), None);
		let output = ctx.run_ui(egui::RawInput::default(), |ui| {
			contents(ui, state, user, &mut profile, &mut action, None);
		});
		let mut text = vec![];
		for shape in &output.shapes {
			labels(&shape.shape, &mut text);
		}
		output.drop_without_applying_deltas();
		text.into_iter().map(|(text, _)| text).collect()
	}

	fn click_menu_action(
		state: &State,
		user: &User,
		label: &str,
		light: bool,
		keyboard: bool,
		voice: bool,
	) -> Option<Action> {
		let ctx = egui::Context::default();
		set_voice_available(&ctx, voice);
		ctx.set_visuals(if light {
			egui::Visuals::light()
		} else {
			egui::Visuals::dark()
		});
		let (mut profile, mut action) = (crate::profiles::ProfileSession::default(), None);
		let (row, _) = frame(&ctx, state, user, vec![], &mut profile, &mut action);
		if keyboard {
			row.request_focus();
			frame(
				&ctx,
				state,
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
					state,
					user,
					pointer(row.rect.center(), PointerButton::Secondary, pressed),
					&mut profile,
					&mut action,
				);
			}
		}
		let (_, text) = frame(&ctx, state, user, vec![], &mut profile, &mut action);
		if let Some((_, rect)) = text.iter().find(|(text, _)| text == label) {
			for pressed in [true, false] {
				frame(
					&ctx,
					state,
					user,
					pointer(rect.center(), PointerButton::Primary, pressed),
					&mut profile,
					&mut action,
				);
			}
		}
		action
	}

	#[test]
	fn nonfriend_message_call_and_friend_actions_use_relationship_state() {
		use client_core::user_actions::Event as UserEvent;
		for light in [false, true] {
			for keyboard in [false, true] {
				let mut state = test_support::demo_state();
				let mut user = state
					.channels
					.iter()
					.find(|channel| channel.kind == 1)
					.unwrap()
					.recipients[0]
					.clone();
				user.id = model::Id(99001);
				assert!(state.friend(user.id).is_none());
				assert_eq!(
					click_menu_action(&state, &user, "Message", light, keyboard, true),
					Some(Action::Message(user.clone()))
				);
				assert_eq!(
					click_menu_action(&state, &user, "Start a Call", light, keyboard, true),
					None,
					"Demo never starts a call"
				);
				assert_eq!(
					click_menu_action(&state, &user, "Add Friend", light, keyboard, true),
					Some(Action::AddFriend(user.clone()))
				);
				apply_user_event(
					&mut state,
					UserEvent::Requests(Some(vec![(user.clone(), "synthetic".into(), true)])),
				);
				assert_eq!(
					click_menu_action(
						&state,
						&user,
						"Accept Friend Request",
						light,
						keyboard,
						true
					),
					Some(Action::AcceptFriend(user.id))
				);
				apply_user_event(
					&mut state,
					UserEvent::Requests(Some(vec![(user.clone(), "synthetic".into(), false)])),
				);
				assert_eq!(
					click_menu_action(&state, &user, "Friend Request Sent", light, keyboard, true),
					None
				);
				apply_user_event(&mut state, UserEvent::Requests(Some(vec![])));
				apply_user_event(&mut state, UserEvent::Relationships(Some(vec![])));
				state.demo = false;
				state.gateway_connected = true;
				state.auth = client_core::auth::AuthState::Authenticated;
				assert_eq!(
					click_menu_action(&state, &user, "Start a Call", light, keyboard, true),
					Some(Action::StartCall(user.clone()))
				);
				assert_eq!(
					click_menu_action(&state, &user, "Start a Call", light, keyboard, false),
					None,
					"Builds without voice never offer a call"
				);
				state.gateway_connected = false;
				for label in ["Message", "Start a Call", "Add Friend"] {
					assert_eq!(
						click_menu_action(&state, &user, label, light, keyboard, true),
						None
					);
				}
				state.gateway_connected = true;
				apply_user_event(
					&mut state,
					UserEvent::Relationships(Some(vec![(user.id, true)])),
				);
				for label in ["Message", "Start a Call", "Add Friend"] {
					assert_eq!(
						click_menu_action(&state, &user, label, light, keyboard, true),
						None
					);
				}
				apply_user_event(&mut state, UserEvent::Relationships(None));
				for label in ["Message", "Start a Call", "Add Friend"] {
					assert_eq!(
						click_menu_action(&state, &user, label, light, keyboard, true),
						None
					);
				}
				apply_user_event(&mut state, UserEvent::Relationships(Some(vec![])));
				assert!(prepare(Action::AddFriend(user.clone()), &mut state).is_some());
				for label in ["Message", "Start a Call", "Add Friend"] {
					assert_eq!(
						click_menu_action(&state, &user, label, light, keyboard, true),
						None
					);
				}
			}
		}
	}

	#[test]
	fn start_call_allows_switch_confirmation_but_disables_current_dm_call() {
		use client_core::voice::{Call, Phase};
		for light in [false, true] {
			for keyboard in [false, true] {
				let mut state = test_support::demo_state();
				apply_user_event(
					&mut state,
					client_core::user_actions::Event::Relationships(Some(vec![])),
				);
				state.demo = false;
				state.gateway_connected = true;
				state.auth = client_core::auth::AuthState::Authenticated;
				let dm = state
					.channels
					.iter()
					.find(|channel| channel.kind == 1)
					.unwrap();
				let channel = dm.id;
				let user = dm.recipients[0].clone();
				state.voice.active = Some(Call {
					channel: model::Id(99002),
					guild: None,
					connected_at: None,
					channel_started_at: None,
					server_muted: false,
					server_deafened: false,
					request: 1,
					phase: Phase::Connected,
					muted: true,
					deafened: true,
					participants: vec![],
					camera: false,
					watching: None,
					error: None,
				});
				assert_eq!(
					click_menu_action(&state, &user, "Start a Call", light, keyboard, true),
					Some(Action::StartCall(user.clone()))
				);
				state.voice.active.as_mut().unwrap().channel = channel;
				assert_eq!(
					click_menu_action(&state, &user, "Start a Call", light, keyboard, true),
					None
				);
			}
		}
	}

	#[test]
	fn friend_menu_excludes_add_friend_and_self_or_webhook_excludes_contact_actions() {
		for light in [false, true] {
			let state = test_support::demo_state();
			let friend = state.friends().next().unwrap();
			assert!(
				!menu_labels(&state, friend)
					.iter()
					.any(|label| label == "Add Friend")
			);
			assert_eq!(
				click_menu_action(&state, friend, "Add Friend", light, false, true),
				None
			);
			let own = state.user.as_ref().unwrap();
			let mut webhook = friend.clone();
			webhook.webhook = true;
			for user in [own, &webhook] {
				let text = menu_labels(&state, user);
				for label in ["Message", "Start a Call", "Add Friend"] {
					assert!(!text.iter().any(|text| text == label));
					assert_eq!(
						click_menu_action(&state, user, label, light, false, true),
						None
					);
				}
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
