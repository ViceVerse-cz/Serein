//! Approved messaging proposals use the same state transitions as native controls.
use crate::MessagingUi;
use client_core::{Command, State, channel_actions::Action};
use extensions::{AppAction, ArchiveQueryKind, MessagingSettingsChange, ServerAdminPage};
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
			AppAction::RequestMessageSearch { query, before_id } => {
				state.request_search(query, before_id.as_deref().map(id).transpose()?)
			}
			AppAction::RequestPins { before } => {
				if let Some(before) = before {
					let before = before.parse::<i128>().map_err(|_| "Invalid pin cursor")?;
					let current = state
						.search
						.as_ref()
						.and_then(|view| view.page.as_ref())
						.and_then(|page| page.pin_cursor);
					if current != Some(before) {
						return Err("The pin cursor changed; request the current page again".into());
					}
					state.request_older_pins()
				} else {
					state.request_pins()
				}
			}
			AppAction::RequestArchives {
				parent_id,
				kind,
				before,
			} => {
				let kind = match kind {
					ArchiveQueryKind::Public => model::archives::Kind::Public,
					ArchiveQueryKind::Private => model::archives::Kind::Private,
					ArchiveQueryKind::JoinedPrivate => model::archives::Kind::JoinedPrivate,
				};
				let before = before
					.as_deref()
					.map(|value| match kind {
						model::archives::Kind::JoinedPrivate => {
							id(value).map(model::archives::Cursor::Id)
						}
						_ => value
							.parse::<i128>()
							.map(model::archives::Cursor::Time)
							.map_err(|_| "Invalid archive cursor".into()),
					})
					.transpose()?;
				state.request_archives(id(&parent_id)?, kind, before)
			}
			AppAction::RequestMemberSearch { channel_id, query } => {
				state.search_members(id(&channel_id)?, &query, 0)
			}
			AppAction::RequestProfile { user_id, guild_id } => {
				let user = id(&user_id)?;
				let guild = guild_id.as_deref().map(id).transpose()?;
				let known = state
					.user
					.as_ref()
					.is_some_and(|current| current.id == user)
					|| state.friend(user).is_some()
					|| state.selected.is_some_and(|channel| {
						state.can_view(channel)
							&& crate::mentions::known_users(state, channel)
								.into_iter()
								.any(|known| known.id == user)
					});
				if !known {
					return Err("This user is not known in the current session".into());
				}
				let command = state.request_profile(user, guild);
				if command.is_none()
					&& !state.profile.as_ref().is_some_and(|profile| {
						profile.user == user && profile.guild == guild && profile.error.is_none()
					}) {
					return Err("Profile is unavailable".into());
				}
				if let Some(command) = command {
					Some(command)
				} else {
					return Ok(());
				}
			}
			AppAction::RequestGifs { query } => {
				let command = state.request_gifs(query.as_deref());
				if command.is_none()
					&& !state
						.gifs
						.view
						.as_ref()
						.is_some_and(|view| view.query == query && view.error.is_none())
				{
					return Err("GIF search is unavailable".into());
				}
				if let Some(command) = command {
					Some(command)
				} else {
					return Ok(());
				}
			}
			AppAction::SetMessagingSettings { change } => {
				use model::messaging_permissions::Change as C;
				let change = match change {
					MessagingSettingsChange::SpamFilter { level } => C::SpamFilter(level.into()),
					MessagingSettingsChange::DefaultAllowDms { enabled } => {
						C::DefaultAllowDms(enabled)
					}
					MessagingSettingsChange::AllowGuildDms { guild_id, enabled } => {
						C::AllowGuildDms(id(&guild_id)?, enabled)
					}
					MessagingSettingsChange::DefaultFilterRequests { enabled } => {
						C::DefaultFilterRequests(enabled)
					}
					MessagingSettingsChange::FilterGuildRequests { guild_id, enabled } => {
						C::FilterGuildRequests(id(&guild_id)?, enabled)
					}
					MessagingSettingsChange::Everyone { enabled } => C::Everyone(enabled),
					MessagingSettingsChange::FriendsOfFriends { enabled } => {
						C::FriendsOfFriends(enabled)
					}
					MessagingSettingsChange::ServerMembers { enabled } => C::ServerMembers(enabled),
					MessagingSettingsChange::PersonalizedRequests { enabled } => {
						C::PersonalizedRequests(enabled)
					}
					MessagingSettingsChange::GameFriendDms { enabled } => C::GameFriendDms(enabled),
					MessagingSettingsChange::GameDms { level } => C::GameDms(level.into()),
				};
				let command = state.update_messaging_permissions(change);
				if let Some(command) = command {
					Some(command)
				} else if state.demo {
					return Ok(());
				} else {
					return Err("Messaging settings are unavailable".into());
				}
			}
			AppAction::SetGuildFolders {
				base_version,
				folders,
			} => {
				let version = state
					.guild_folders
					.as_ref()
					.ok_or("Server folders are unavailable")?
					.version;
				if version != base_version {
					return Err("The server folder layout changed; run the extension again".into());
				}
				let folders = folders
					.into_iter()
					.map(|folder| {
						Ok(model::guild_folders::Folder {
							id: folder.id,
							guild_ids: folder
								.guild_ids
								.iter()
								.map(|value| id(value))
								.collect::<Result<_, _>>()?,
							name: folder.name,
							color: folder.color,
						})
					})
					.collect::<Result<Vec<_>, String>>()?;
				let command =
					state.save_guild_folders(model::guild_folders::Settings { folders, version });
				if let Some(command) = command {
					Some(command)
				} else if state.folders_error.is_none() && !state.folders_pending {
					return Ok(());
				} else {
					return Err(state
						.folders_error
						.unwrap_or("Server folders are unavailable")
						.into());
				}
			}
			AppAction::OpenJoinServer { invite } => {
				self.open_rpc_invite(state.generation, invite);
				return Ok(());
			}
			AppAction::SendServerInvite { guild_id, user_id } => {
				state.send_server_invite(id(&guild_id)?, id(&user_id)?)
			}
			AppAction::OpenServerAdmin { guild_id, page } => {
				let guild = id(&guild_id)?;
				let allowed = match page {
					ServerAdminPage::Emoji => state.can_open_emoji_settings(guild),
					ServerAdminPage::Members => state.can_open_member_settings(guild),
					ServerAdminPage::Roles => state.can_open_role_settings(guild),
					ServerAdminPage::Invites => state.can_open_invite_settings(guild),
					ServerAdminPage::AuditLog => state.can_open_audit_log_settings(guild),
				};
				if !allowed {
					return Err(
						"Server administration is unavailable with the current permissions".into(),
					);
				}
				let page = match page {
					ServerAdminPage::Emoji => "emoji",
					ServerAdminPage::Members => "members",
					ServerAdminPage::Roles => "roles",
					ServerAdminPage::Invites => "invites",
					ServerAdminPage::AuditLog => "audit-log",
				};
				let command = self.preview_server_admin(state, guild, page);
				if let Some(command) = command {
					Some(command)
				} else {
					return Ok(());
				}
			}
			AppAction::OpenGroupEditor { channel_id } => {
				self.preview_group_editor(state, id(&channel_id)?)?;
				return Ok(());
			}
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
			AppAction::SendReply {
				channel_id,
				message_id,
				content,
				mention,
			} => {
				selected_channel(state, &channel_id)?;
				let command = state
					.prepare_reply_send(id(&message_id)?, &content, mention)
					.ok_or("Replying is unavailable or exceeds the input budget")?;
				self.timeline.follow_latest(state);
				Some(command)
			}
			AppAction::SendSticker {
				channel_id,
				sticker_id,
			} => {
				selected_channel(state, &channel_id)?;
				let command = state
					.prepare_sticker_id_send(id(&sticker_id)?)
					.ok_or("This sticker is unavailable in the current conversation")?;
				self.timeline.follow_latest(state);
				Some(command)
			}
			AppAction::ForwardMessage {
				channel_id,
				message_id,
				target_channel_ids,
				note,
			} => {
				selected_channel(state, &channel_id)?;
				let targets = target_channel_ids
					.iter()
					.map(|target| id(target))
					.collect::<Result<Vec<_>, _>>()?;
				let outgoing = state.prepare_forward(id(&message_id)?, &targets, &note);
				if outgoing.is_empty() {
					return Err(state.status.into());
				}
				commands.extend(outgoing);
				return Ok(());
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

	fn test_state() -> State {
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
		let mut state = test_state();
		let mut view = MessagingUi {
			attachment_files: vec![("unsent.png".into(), 12)],
			..Default::default()
		};
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
	fn reply_sticker_and_forward_reuse_native_bounded_send_paths() {
		let mut view = MessagingUi::default();
		let mut commands = Vec::new();
		let mut state = test_state();
		state.drafts.insert(Id(22), "Keep draft".into());
		state.reply = Some(client_core::Reply::to(Id(700)));
		view.apply_extension_app_action(
			&mut state,
			AppAction::SendReply {
				channel_id: "22".into(),
				message_id: "700".into(),
				content: "Approved reply".into(),
				mention: false,
			},
			&mut commands,
		)
		.unwrap();
		assert!(matches!(
			&commands[0],
			Command::Send { channel: Id(22), content, reply: Some(reply), sticker: None, .. }
				if content == "Approved reply" && reply.target() == Id(700) && !reply.mention
		));
		assert_eq!(state.drafts[&Id(22)], "Keep draft");
		assert_eq!(state.reply, Some(client_core::Reply::to(Id(700))));

		let mut state = test_state();
		test_support::seed_stickers(&mut state);
		state.drafts.insert(Id(22), "Keep draft".into());
		commands.clear();
		view.apply_extension_app_action(
			&mut state,
			AppAction::SendSticker {
				channel_id: "22".into(),
				sticker_id: "9201".into(),
			},
			&mut commands,
		)
		.unwrap();
		assert!(matches!(
			commands[0],
			Command::Send { channel: Id(22), sticker: Some(Id(9201)), ref content, .. }
				if content.is_empty()
		));
		assert_eq!(state.drafts[&Id(22)], "Keep draft");
		assert!(
			view.apply_extension_app_action(
				&mut state,
				AppAction::SendSticker {
					channel_id: "22".into(),
					sticker_id: "999999".into(),
				},
				&mut commands,
			)
			.is_err()
		);

		let mut state = test_state();
		let target = state
			.channels
			.iter()
			.map(|channel| channel.id)
			.find(|channel| *channel != Id(22) && state.can_compose(*channel))
			.unwrap();
		commands.clear();
		view.apply_extension_app_action(
			&mut state,
			AppAction::ForwardMessage {
				channel_id: "22".into(),
				message_id: "700".into(),
				target_channel_ids: vec![target.to_string()],
				note: "Approved note".into(),
			},
			&mut commands,
		)
		.unwrap();
		assert!(matches!(
			commands[0],
			Command::Forward { message: Id(700), channel, .. } if channel == target
		));
		assert!(matches!(
			&commands[1],
			Command::Send { channel, content, reply: None, sticker: None, .. }
				if *channel == target && content == "Approved note"
		));
	}

	#[test]
	fn desired_reaction_does_not_toggle_and_revalidates_pending_and_access() {
		let mut state = test_state();
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
		let mut state = test_state();
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
		let mut state = test_state();
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
		let mut state = test_state();
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
		let mut state = test_state();
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

	#[test]
	fn query_and_account_settings_reuse_native_bounded_state() {
		let mut state = test_state();
		let mut view = MessagingUi::default();
		let mut commands = Vec::new();
		view.apply_extension_app_action(
			&mut state,
			AppAction::RequestMessageSearch {
				query: "release".into(),
				before_id: None,
			},
			&mut commands,
		)
		.unwrap();
		assert!(matches!(&commands[0], Command::Search { query, .. } if query == "release"));

		state.demo = true;
		state.messaging_permissions.snapshot = Some(Default::default());
		view.apply_extension_app_action(
			&mut state,
			AppAction::SetMessagingSettings {
				change: MessagingSettingsChange::DefaultAllowDms { enabled: false },
			},
			&mut commands,
		)
		.unwrap();
		assert!(
			!state
				.messaging_permissions
				.snapshot
				.as_ref()
				.unwrap()
				.default_allow_dms
		);

		state.guild_folders = Some(Default::default());
		view.apply_extension_app_action(
			&mut state,
			AppAction::SetGuildFolders {
				base_version: 0,
				folders: vec![extensions::GuildFolderInput {
					id: None,
					guild_ids: vec!["1".into()],
					name: None,
					color: None,
				}],
			},
			&mut commands,
		)
		.unwrap();
		assert_eq!(
			state.guild_folders.as_ref().unwrap().folders[0].guild_ids,
			[Id(1)]
		);
		assert!(
			view.apply_extension_app_action(
				&mut state,
				AppAction::SetGuildFolders {
					base_version: 99,
					folders: Vec::new(),
				},
				&mut commands,
			)
			.is_err()
		);
		assert!(
			view.apply_extension_app_action(
				&mut state,
				AppAction::RequestProfile {
					user_id: "999999".into(),
					guild_id: None,
				},
				&mut commands,
			)
			.is_err()
		);
	}
}
