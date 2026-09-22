//! Approved messaging proposals use the same state transitions as native controls.
use crate::MessagingUi;
use client_core::{Command, State, channel_actions::Action};
use extensions::AppAction;
use model::{Id, ReactionEmoji};

fn id(value: &str) -> Result<Id, String> {
	value
		.parse::<Id>()
		.ok()
		.filter(|id| id.0 != 0)
		.ok_or_else(|| "Invalid identifier".into())
}

fn selected_channel(state: &State, value: &str) -> Result<Id, String> {
	let channel = id(value)?;
	if state.selected != Some(channel) || !state.can_view(channel) {
		return Err("The selected conversation changed or is no longer accessible".into());
	}
	Ok(channel)
}

fn reaction(value: &str) -> Result<ReactionEmoji, String> {
	let (id, name) = if let Some((name, value)) = value.split_once(':') {
		if !(2..=32).contains(&name.len())
			|| !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
		{
			return Err("Invalid custom emoji name".into());
		}
		(Some(id(value)?), name)
	} else {
		(None, value)
	};
	let emoji = ReactionEmoji {
		id,
		name: Some(name.into()),
	};
	if !emoji.valid() {
		return Err("Invalid reaction emoji".into());
	}
	Ok(emoji)
}

impl MessagingUi {
	pub(crate) fn apply_extension_app_action(
		&mut self,
		state: &mut State,
		action: AppAction,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		let command = match action {
			AppAction::SendMessage {
				channel_id,
				content,
			} => {
				selected_channel(state, &channel_id)?;
				let command = state
					.prepare_text_send(&content)
					.ok_or("Sending is unavailable or exceeds the input budget")?;
				self.timeline.follow_latest(state);
				Some(command)
			}
			AppAction::EditMessage {
				channel_id,
				message_id,
				content,
			} => {
				let channel = selected_channel(state, &channel_id)?;
				state.prepare_edit(channel, id(&message_id)?, content)
			}
			AppAction::DeleteMessage {
				channel_id,
				message_id,
			} => {
				let channel = selected_channel(state, &channel_id)?;
				state.prepare_delete(channel, id(&message_id)?)
			}
			AppAction::SetReaction {
				channel_id,
				message_id,
				emoji,
				add,
			} => {
				selected_channel(state, &channel_id)?;
				commands.extend(state.prepare_set_reaction(
					id(&message_id)?,
					reaction(&emoji)?,
					add,
				)?);
				return Ok(());
			}
			AppAction::SetMessagePinned {
				channel_id,
				message_id,
				pinned,
			} => {
				let channel = selected_channel(state, &channel_id)?;
				state.prepare_pin(channel, id(&message_id)?, pinned)
			}
			AppAction::MarkRead {
				channel_id,
				message_id,
			} => {
				selected_channel(state, &channel_id)?;
				state.prepare_mark_read(id(&message_id)?)
			}
			AppAction::MarkChannelRead { channel_id } => {
				let channel = id(&channel_id)?;
				if !state.can_view(channel) {
					return Err("This conversation is no longer accessible".into());
				}
				state.prepare_mark_channel_read(channel)
			}
			AppAction::MarkUnread {
				channel_id,
				message_id,
			} => {
				selected_channel(state, &channel_id)?;
				let command = state
					.prepare_mark_unread(id(&message_id)?)
					.ok_or("Marking this message unread is unavailable")?;
				self.timeline.browse_away();
				Some(command)
			}
			AppAction::MarkGuildRead { guild_id } => state.prepare_mark_guild_read(id(&guild_id)?),
			AppAction::JumpToUnread => {
				if !state.can_jump_unread() {
					return Err("Unread navigation is unavailable".into());
				}
				commands.extend(state.open_unread());
				return Ok(());
			}
			AppAction::CreateThread {
				channel_id,
				name,
				message_id,
			} => {
				let channel = selected_channel(state, &channel_id)?;
				let message = message_id.as_deref().map(id).transpose()?;
				state.request_channel_action(channel, Action::CreateThread { name, message })
			}
			AppAction::CreateForumPost {
				parent_id,
				title,
				content,
			} => state.create_post(id(&parent_id)?, &title, &content),
			AppAction::SetThreadArchived {
				channel_id,
				archived,
			} => state.request_channel_action(id(&channel_id)?, Action::PostArchive(archived)),
			AppAction::SetThreadLocked { channel_id, locked } => {
				state.request_channel_action(id(&channel_id)?, Action::PostLock(locked))
			}
			AppAction::SetThreadFollowed {
				channel_id,
				followed,
			} => state.request_channel_action(id(&channel_id)?, Action::PostFollow(followed)),
			AppAction::SetThreadPinned { channel_id, pinned } => {
				state.request_channel_action(id(&channel_id)?, Action::PostPin(pinned))
			}
			AppAction::RenameThread { channel_id, name } => {
				state.request_channel_action(id(&channel_id)?, Action::PostRename(name))
			}
			other => return self.apply_extension_account_action(state, other, commands),
		};
		commands.push(command.ok_or(
			"This action is unavailable with the current permissions, data or pending operation",
		)?);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn state() -> State {
		let mut state = test_support::demo_state();
		state.selected = Some(Id(22));
		state.freshness = model::Freshness::Fresh;
		state.auth = client_core::auth::AuthState::Authenticated;
		state.gateway_connected = true;
		state.timeline.clear();
		let mut message = test_support::message(700, Id(22));
		message.author = state.user.clone().unwrap();
		message.content = "Original".into();
		state.timeline.insert(message, false, false).unwrap();
		state
	}

	#[test]
	fn explicit_send_preserves_composition_and_rejects_stale_scope_and_limits() {
		let mut state = state();
		let mut view = MessagingUi::default();
		view.attachment_files = vec![("unsent.png".into(), 12)];
		state.drafts.insert(Id(22), "Unrelated draft".into());
		state.reply = Some(client_core::Reply::to(Id(700)));
		let mut commands = Vec::new();
		let action = || AppAction::SendMessage {
			channel_id: "22".into(),
			content: "Approved text".into(),
		};
		view.apply_extension_app_action(&mut state, action(), &mut commands)
			.unwrap();
		assert!(
			matches!(&commands[0], Command::Send { channel: Id(22), content, reply: None, .. } if content == "Approved text")
		);
		assert_eq!(state.drafts[&Id(22)], "Unrelated draft");
		assert_eq!(state.reply, Some(client_core::Reply::to(Id(700))));
		assert_eq!(view.attachment_files, [("unsent.png".into(), 12)]);
		assert!(state.pending.last().unwrap().attachments.is_empty());
		state.selected = Some(Id(20));
		assert!(
			view.apply_extension_app_action(&mut state, action(), &mut commands)
				.is_err()
		);
		state.selected = Some(Id(22));
		state.gateway_connected = false;
		assert!(state.prepare_text_send("Denied").is_none());
		state.gateway_connected = true;
		for invalid in [" ".into(), "x".repeat(client_core::MAX_CONTENT + 1)] {
			assert!(state.prepare_text_send(&invalid).is_none());
		}
		state
			.drafts
			.insert(Id(20), "x".repeat(client_core::MAX_DRAFT_BYTES));
		assert!(state.prepare_text_send("Over budget").is_none());
		assert_eq!(state.pending.len(), 1);
		assert_eq!(state.reply, Some(client_core::Reply::to(Id(700))));
		assert_eq!(commands.len(), 1);
	}

	#[test]
	fn desired_reaction_does_not_toggle_and_revalidates_pending_and_access() {
		let mut state = state();
		let emoji = reaction("??").unwrap();
		assert!(
			state
				.prepare_set_reaction(Id(700), emoji.clone(), false)
				.unwrap()
				.is_none()
		);
		let command = state
			.prepare_set_reaction(Id(700), emoji.clone(), true)
			.unwrap()
			.unwrap();
		assert!(matches!(
			command,
			Command::Reactions(client_core::reactions::Command::Set { add: true, .. })
		));
		assert!(
			state
				.prepare_set_reaction(Id(700), emoji.clone(), true)
				.is_err()
		);
		state.command_rejected(command);
		assert!(
			state
				.prepare_set_reaction(Id(700), emoji.clone(), false)
				.unwrap()
				.is_none()
		);
		state.gateway_connected = false;
		assert!(state.prepare_set_reaction(Id(700), emoji, false).is_err());
		assert_eq!(reaction("custom_emoji:123").unwrap().id, Some(Id(123)));
		for invalid in ["", "bad name:123", "custom:0", "custom:123:456", "\n"] {
			assert!(reaction(invalid).is_err());
		}
	}

	#[test]
	fn message_mutations_use_native_ownership_and_pending_guards() {
		let mut state = state();
		let mut view = MessagingUi::default();
		let mut commands = Vec::new();
		view.apply_extension_app_action(
			&mut state,
			AppAction::EditMessage {
				channel_id: "22".into(),
				message_id: "700".into(),
				content: "Edited".into(),
			},
			&mut commands,
		)
		.unwrap();
		assert!(
			matches!(&commands[0], Command::Edit { channel: Id(22), message: Id(700), content, .. } if content == "Edited")
		);
		let Command::Edit {
			request,
			channel,
			message,
			..
		} = commands.pop().unwrap()
		else {
			panic!("expected edit command");
		};
		assert!(
			view.apply_extension_app_action(
				&mut state,
				AppAction::EditMessage {
					channel_id: "22".into(),
					message_id: "700".into(),
					content: "Another edit".into(),
				},
				&mut commands,
			)
			.is_err()
		);
		assert!(commands.is_empty());
		// A transport failure rolls back this edit without ending the session. Queue
		// rejection reports Capacity, which deliberately disconnects the account.
		state.apply_edit_result(
			channel,
			message,
			request,
			Err(client_core::auth::Failure::Network),
		);
		assert!(!state.message_actions.edit_pending(channel, message));
		assert_eq!(state.timeline.get(message).unwrap().content, "Original");
		view.apply_extension_app_action(
			&mut state,
			AppAction::SetMessagePinned {
				channel_id: "22".into(),
				message_id: "700".into(),
				pinned: true,
			},
			&mut commands,
		)
		.unwrap();
		assert!(matches!(
			commands.pop().unwrap(),
			Command::Pin { pinned: true, .. }
		));
		view.apply_extension_app_action(
			&mut state,
			AppAction::DeleteMessage {
				channel_id: "22".into(),
				message_id: "700".into(),
			},
			&mut commands,
		)
		.unwrap();
		assert!(matches!(
			commands.pop().unwrap(),
			Command::Delete {
				message: Id(700),
				..
			}
		));
		state.user.as_mut().unwrap().id = Id(999);
		assert!(
			view.apply_extension_app_action(
				&mut state,
				AppAction::DeleteMessage {
					channel_id: "22".into(),
					message_id: "700".into()
				},
				&mut commands
			)
			.is_err()
		);
		assert!(commands.is_empty());
	}
	#[test]
	fn unread_actions_preserve_native_cursor_and_loaded_jump_behavior() {
		use client_core::read_state::Event;
		let mut state = state();
		state
			.channels
			.iter_mut()
			.find(|channel| channel.id == Id(22))
			.unwrap()
			.last_message = Some(Id(700));
		state
			.apply_read_state(Event::Snapshot {
				entries: Some(vec![(Id(22), Some(Id(650)), 0)]),
				version: None,
				partial: false,
			})
			.unwrap();
		state.older_exhausted = true;
		state.history_pending = false;
		let mut view = MessagingUi::default();
		let mut commands = Vec::new();
		view.apply_extension_app_action(&mut state, AppAction::JumpToUnread, &mut commands)
			.unwrap();
		assert_eq!(state.search_target, Some(Id(700)));
		assert!(commands.is_empty());
		state
			.apply_read_state(Event::Ack {
				channel: Id(22),
				message: Some(Id(700)),
				manual: false,
				mention_count: None,
				version: None,
			})
			.unwrap();
		view.timeline.mark_read = Some(Id(700));
		view.apply_extension_app_action(
			&mut state,
			AppAction::MarkUnread {
				channel_id: "22".into(),
				message_id: "700".into(),
			},
			&mut commands,
		)
		.unwrap();
		assert!(matches!(
			commands[0],
			Command::MarkRead {
				channel: Id(22),
				message: Id(699),
				manual: true,
				..
			}
		));
		assert_eq!(view.timeline.mark_read, None);
		assert!(
			view.apply_extension_app_action(
				&mut state,
				AppAction::MarkUnread {
					channel_id: "20".into(),
					message_id: "700".into()
				},
				&mut commands
			)
			.is_err()
		);
		assert_eq!(commands.len(), 1);
	}

	#[test]
	fn thread_and_forum_creation_reuse_bounded_native_requests() {
		let mut state = state();
		state.selected = Some(Id(20));
		let mut view = MessagingUi::default();
		let mut commands = Vec::new();
		let create = || AppAction::CreateThread {
			channel_id: "20".into(),
			name: "Approved thread".into(),
			message_id: None,
		};
		view.apply_extension_app_action(&mut state, create(), &mut commands)
			.unwrap();
		assert!(
			matches!(&commands[0], Command::ChannelAction { channel: Id(20), action: Action::CreateThread { name, message: None }, .. } if name == "Approved thread")
		);
		assert!(
			view.apply_extension_app_action(&mut state, create(), &mut commands)
				.is_err()
		);
		assert_eq!(commands.len(), 1);
		let mut state = self::state();
		view.apply_extension_app_action(
			&mut state,
			AppAction::CreateForumPost {
				parent_id: "26".into(),
				title: "Approved post".into(),
				content: "Starter message".into(),
			},
			&mut commands,
		)
		.unwrap();
		assert!(
			matches!(&commands[1], Command::CreatePost { parent: Id(26), title, content, attachments, .. } if title == "Approved post" && content == "Starter message" && attachments.is_empty())
		);
	}
}
