//! Explicitly granted, bounded snapshots of already-loaded client data. No IO.
use client_core::{State, auth::AuthState};
use extensions::*;
use model::{Freshness, Id};

pub fn available(state: &State) -> bool {
	(state.demo || state.auth == AuthState::Authenticated)
		&& state.user.as_ref().is_some_and(|user| user.id.0 != 0)
}

pub fn uses_app(capabilities: &[Capability]) -> bool {
	capabilities.iter().any(|capability| {
		matches!(
			capability,
			Capability::AppContext
				| Capability::MessageContent
				| Capability::ForumData
				| Capability::ConversationActivity
				| Capability::AccountProfile
				| Capability::GuildDirectory
				| Capability::ChannelDetails
				| Capability::ChannelMetadata
				| Capability::MemberDetails
				| Capability::DataEvents
				| Capability::MessageDetails
				| Capability::Relationships
				| Capability::ChannelDirectory
				| Capability::Timeline
				| Capability::Members
				| Capability::Presence
				| Capability::VoiceState
				| Capability::ReadState
				| Capability::LocalSettings
				| Capability::NotificationSettings
				| Capability::Navigation
				| Capability::LocalNotices
				| Capability::ClipboardWrite
				| Capability::VoiceControl
				| Capability::AppEvents
		)
	})
}

pub(crate) fn name(value: &str) -> String {
	let mut end = value.len().min(128);
	while !value.is_char_boundary(end) {
		end -= 1;
	}
	let name: String = value[..end].chars().filter(|c| !c.is_control()).collect();
	if name.trim().is_empty() {
		"Unnamed".into()
	} else {
		name
	}
}

fn user(value: &model::User) -> UserSnapshot {
	UserSnapshot {
		id: value.id.0.to_string(),
		name: name(&value.name),
	}
}

fn channel(value: &model::Channel) -> ChannelSnapshot {
	ChannelSnapshot {
		id: value.id.0.to_string(),
		guild_id: value.guild.map(|id| id.0.to_string()),
		name: name(&value.name),
		kind: value.kind,
	}
}

pub(crate) fn text(value: &str, limit: usize) -> String {
	let mut end = value.len().min(limit);
	while !value.is_char_boundary(end) {
		end -= 1;
	}
	value[..end]
		.chars()
		.filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
		.collect()
}

fn asset_hash(value: &Option<String>) -> Option<String> {
	value
		.as_ref()
		.filter(|hash| hash.len() <= 128 && model::valid_avatar_hash(hash))
		.cloned()
}

// Each candidate is already scalar-bounded before serialization. Account for JSON and
// fixed list storage; these per-group budgets also leave room below the 64 KiB ABI cap.
pub(crate) fn push<T: serde::Serialize>(
	items: &mut Vec<T>,
	item: T,
	left: &mut usize,
	max: usize,
) -> bool {
	let bytes = serde_json::to_vec(&item).map_or(usize::MAX, |bytes| {
		bytes.len().saturating_add(std::mem::size_of::<T>())
	});
	if items.len() >= max || bytes > *left {
		return false;
	}
	*left -= bytes;
	items.push(item);
	true
}

fn message_detail(message: &model::Message) -> MessageDetailSnapshot {
	let mut detail = MessageDetailSnapshot {
		id: message.id.0.to_string(),
		kind: message.kind,
		reply_to: message
			.reply_to
			.filter(|id| id.0 != 0)
			.map(|id| id.0.to_string()),
		mention_ids: Vec::new(),
		mentions_truncated: false,
		mention_everyone: message.mention_everyone,
		attachments: Vec::new(),
		attachments_truncated: false,
		reactions: message.reactions.as_ref().map(|_| Vec::new()),
		reactions_truncated: false,
	};
	// Bound nested rows before allocating/serializing the complete candidate.
	let mut budget = 4 * 1024;
	for mentioned in message.mentions.iter().filter(|u| u.id.0 != 0) {
		let id = mentioned.id.0.to_string();
		if detail.mention_ids.contains(&id) {
			continue;
		}
		if !push(
			&mut detail.mention_ids,
			id,
			&mut budget,
			MAX_MESSAGE_MENTIONS,
		) {
			detail.mentions_truncated = true;
			break;
		}
	}
	for attachment in &message.attachments {
		let id = attachment.id.0.to_string();
		if attachment.id.0 == 0 || detail.attachments.iter().any(|a| a.id == id) {
			detail.attachments_truncated = true;
			continue;
		}
		let filename: String = text(&attachment.filename, 256)
			.chars()
			.filter(|c| !c.is_control())
			.collect();
		let content_type = attachment
			.content_type
			.as_deref()
			.map(|s| {
				text(s, 128)
					.chars()
					.filter(|c| !c.is_control())
					.collect::<String>()
			})
			.filter(|s| !s.is_empty());
		if !push(
			&mut detail.attachments,
			AttachmentSnapshot {
				id,
				filename: if filename.is_empty() {
					"Attachment".into()
				} else {
					filename
				},
				size: attachment.size,
				content_type,
				spoiler: attachment.spoiler,
			},
			&mut budget,
			MAX_MESSAGE_ATTACHMENTS,
		) {
			detail.attachments_truncated = true;
			break;
		}
	}
	if let (Some(source), Some(reactions)) = (&message.reactions, &mut detail.reactions) {
		for reaction in source {
			if reaction.count == 0
				|| !reaction.emoji.valid()
				|| reaction.emoji.id.is_some_and(|id| id.0 == 0)
			{
				detail.reactions_truncated = true;
				continue;
			}
			if !push(
				reactions,
				ReactionSnapshot {
					emoji_id: reaction.emoji.id.map(|id| id.0.to_string()),
					emoji_name: reaction.emoji.name.clone(),
					count: reaction.count,
					me: reaction.me,
					me_burst: reaction.me_burst,
				},
				&mut budget,
				MAX_MESSAGE_REACTIONS,
			) {
				detail.reactions_truncated = true;
				break;
			}
		}
	}
	detail
}

fn channel_metadata(state: &State) -> Option<ChannelMetadataSnapshot> {
	use model::permissions as p;
	if !state.gateway_connected || state.freshness != Freshness::Fresh {
		return None;
	}
	let id = state.selected?;
	let current = state.channel(id)?;
	let guild = current.guild?;
	if !state.can_view(id) || !state.can_read_history(id) {
		return None;
	}
	let related = |id| {
		state
			.channel(id)
			.filter(|c| c.guild == Some(guild) && state.can_view(c.id))
	};
	let parent = current.parent_id.and_then(related);
	let category = parent.and_then(|parent| {
		if parent.kind == 4 {
			Some(parent)
		} else {
			parent.parent_id.and_then(related).filter(|c| c.kind == 4)
		}
	});
	let loaded = state.channel_details(id);
	let post = state.post_details(id);
	let mut permissions = std::collections::BTreeMap::new();
	for (permission, bits) in [
		(ChannelPermission::ViewChannel, p::VIEW_CHANNEL),
		(
			ChannelPermission::ReadMessageHistory,
			p::READ_MESSAGE_HISTORY,
		),
		(ChannelPermission::SendMessages, p::SEND_MESSAGES),
		(
			ChannelPermission::SendMessagesInThreads,
			p::SEND_MESSAGES_IN_THREADS,
		),
		(ChannelPermission::AttachFiles, p::ATTACH_FILES),
		(ChannelPermission::EmbedLinks, p::EMBED_LINKS),
		(ChannelPermission::AddReactions, p::ADD_REACTIONS),
		(ChannelPermission::MentionEveryone, p::MENTION_EVERYONE),
		(ChannelPermission::UseExternalEmojis, p::USE_EXTERNAL_EMOJIS),
		(
			ChannelPermission::UseExternalStickers,
			p::USE_EXTERNAL_STICKERS,
		),
		(
			ChannelPermission::UseApplicationCommands,
			p::USE_APPLICATION_COMMANDS,
		),
		(ChannelPermission::ManageChannels, p::MANAGE_CHANNELS),
		(ChannelPermission::ManageMessages, p::MANAGE_MESSAGES),
		(ChannelPermission::ManageRoles, p::MANAGE_ROLES),
		(ChannelPermission::ManageThreads, p::MANAGE_THREADS),
		(
			ChannelPermission::CreatePublicThreads,
			p::CREATE_PUBLIC_THREADS,
		),
		(
			ChannelPermission::CreatePrivateThreads,
			p::CREATE_PRIVATE_THREADS,
		),
		(ChannelPermission::ManageWebhooks, p::MANAGE_WEBHOOKS),
		(ChannelPermission::Connect, p::CONNECT),
		(ChannelPermission::Speak, p::SPEAK),
		(ChannelPermission::Stream, p::STREAM),
		(ChannelPermission::MuteMembers, p::MUTE_MEMBERS),
		(ChannelPermission::DeafenMembers, p::DEAFEN_MEMBERS),
		(ChannelPermission::MoveMembers, p::MOVE_MEMBERS),
		(ChannelPermission::UseVad, p::USE_VAD),
		(ChannelPermission::PinMessages, p::PIN_MESSAGES),
	] {
		permissions.insert(permission, state.permission(id, p::VIEW_CHANNEL | bits));
	}
	Some(ChannelMetadataSnapshot {
		channel_id: id.0.to_string(),
		guild_id: guild.0.to_string(),
		parent: parent.map(channel),
		category: category.map(channel),
		topic: loaded.map(|d| text(&d.topic, 2048)),
		topic_truncated: loaded.is_some_and(|d| d.topic.len() > 2048),
		slowmode_seconds: loaded.map(|d| d.slowmode),
		nsfw: loaded.map(|d| d.nsfw),
		thread: matches!(current.kind, 10..=12).then(|| ThreadMetadataSnapshot {
			owner_id: post
				.and_then(|p| p.owner)
				.filter(|id| id.0 != 0)
				.map(|id| id.0.to_string()),
			message_count: current.message_count,
			archived: post.map(|p| p.archived),
			locked: post.map(|p| p.locked),
			pinned: post.map(|p| p.pinned),
		}),
		permissions,
	})
}

pub fn snapshot(
	state: &State,
	messaging: &ui::MessagingUi,
	manifest: &Manifest,
) -> Option<Box<AppSnapshot>> {
	if !available(state) || !uses_app(&manifest.capabilities) {
		return None;
	}
	let granted = |capability| manifest.capabilities.contains(&capability);
	let connected = state.gateway_connected;
	let selected = state
		.selected
		.filter(|id| connected && state.freshness != Freshness::Unavailable && state.can_view(*id));
	let readable = selected.filter(|id| {
		state.can_read_history(*id)
			&& state.freshness == Freshness::Fresh
			&& state.channel(*id).is_some_and(|c| c.supports_text())
	});
	let mut app = AppSnapshot::default();
	if granted(Capability::AppContext) {
		app.context = Some(AppContextSnapshot {
			connected,
			user: state.user.as_ref().map(user),
			channel: selected.and_then(|id| state.channel(id)).map(channel),
		});
	}
	if connected && granted(Capability::AccountProfile) {
		let current = state.user.as_ref()?;
		let profile = state
			.own_profile
			.data
			.as_ref()
			.filter(|profile| {
				profile.user.id == current.id
					&& !profile.limited
					&& !state.own_profile.loading
					&& !state.own_profile.reload_required
					&& state.own_profile.error.is_none()
			})
			.map(|profile| OwnProfileSnapshot {
				display_name: profile.global_name.as_deref().map(name),
				bio: text(&profile.bio, 2048),
				pronouns: text(&profile.pronouns, 256)
					.chars()
					.filter(|c| !c.is_control())
					.collect(),
			});
		app.account_profile = Some(AccountProfileSnapshot {
			user: user(current),
			avatar: asset_hash(&current.avatar),
			profile,
		});
	}
	if connected && granted(Capability::GuildDirectory) {
		let mut directory = GuildDirectorySnapshot {
			items: Vec::new(),
			truncated: false,
		};
		let mut budget = 8 * 1024;
		for guild in state.guilds.iter().filter(|guild| guild.id.0 != 0) {
			if !push(
				&mut directory.items,
				GuildSnapshot {
					id: guild.id.0.to_string(),
					name: name(&guild.name),
					icon: asset_hash(&guild.icon),
				},
				&mut budget,
				MAX_APP_GUILDS,
			) {
				directory.truncated = true;
				break;
			}
		}
		app.guilds = Some(directory);
	}
	if granted(Capability::ChannelDetails)
		&& state.freshness == Freshness::Fresh
		&& let Some(value) = selected.and_then(|id| state.channel(id))
	{
		let mut recipients = Vec::new();
		let mut recipients_truncated = false;
		let mut budget = 6 * 1024;
		for recipient in value.recipients.iter().filter(|user| user.id.0 != 0) {
			if recipients
				.iter()
				.any(|item: &UserSnapshot| item.id == recipient.id.0.to_string())
			{
				continue;
			}
			if !push(
				&mut recipients,
				user(recipient),
				&mut budget,
				MAX_CHANNEL_RECIPIENTS,
			) {
				recipients_truncated = true;
				break;
			}
		}
		let can_read_history = state.can_read_history(value.id);
		app.channel_details = Some(ChannelDetailsSnapshot {
			channel: channel(value),
			parent_id: value
				.parent_id
				.filter(|id| state.can_view(*id))
				.map(|id| id.0.to_string()),
			position: value.position,
			last_message_id: value
				.last_message
				.filter(|id| id.0 != 0 && can_read_history)
				.map(|id| id.0.to_string()),
			message_count: can_read_history.then_some(value.message_count).flatten(),
			recipients,
			recipients_truncated,
			can_send: state.can_send(value.id),
			can_read_history,
		});
	}
	if connected && granted(Capability::ChannelDirectory) {
		let mut directory = ChannelDirectorySnapshot {
			items: Vec::new(),
			truncated: false,
		};
		let mut budget = 10 * 1024;
		for value in state.channels.iter().filter(|c| {
			state.can_view(c.id)
				&& !(state.selected == Some(c.id) && state.freshness == Freshness::Unavailable)
		}) {
			if !push(
				&mut directory.items,
				channel(value),
				&mut budget,
				MAX_APP_CHANNELS,
			) {
				directory.truncated = true;
				break;
			}
		}
		app.channels = Some(directory);
	}
	if let Some(id) = readable
		&& granted(Capability::Timeline)
	{
		let mut timeline = TimelineSnapshot {
			channel_id: id.0.to_string(),
			messages: Vec::new(),
			truncated: state.history_before.is_some()
				|| state.history_after.is_some()
				|| !state.older_exhausted,
		};
		// Rich rows cost more to decode: align the text window with the metadata row bound.
		let max_messages = if granted(Capability::MessageDetails) {
			MAX_MESSAGE_DETAILS
		} else {
			MAX_APP_MESSAGES
		};
		let mut budget = 20 * 1024;
		for message in
			state.timeline.iter().rev().filter(|m| {
				m.channel == id && !m.ephemeral && m.flags & 64 == 0 && m.author.id.0 != 0
			}) {
			if message.content.len() > 4096 {
				timeline.truncated = true;
				continue;
			}
			if !push(
				&mut timeline.messages,
				MessageSnapshot {
					id: message.id.0.to_string(),
					author: user(&message.author),
					content: message.content.clone(),
					attachment_count: message.attachments.len().min(u16::MAX as usize) as u16,
					edited: message.edited,
				},
				&mut budget,
				max_messages,
			) {
				timeline.truncated = true;
				break;
			}
		}
		timeline.messages.reverse();
		app.timeline = Some(timeline);
	}
	if let Some(id) = selected {
		let members = state
			.members
			.as_ref()
			.filter(|members| members.channel == id && members.freshness == Freshness::Fresh);
		let recipients = state.channel(id).filter(|c| c.guild.is_none());
		if granted(Capability::Members) && (members.is_some() || recipients.is_some()) {
			let mut group = MembersSnapshot {
				channel_id: id.0.to_string(),
				items: Vec::new(),
				truncated: false,
			};
			let mut budget = 6 * 1024;
			let users = members
				.into_iter()
				.flat_map(|m| {
					m.slots.iter().flatten().filter_map(|slot| match slot {
						model::MemberSlot::Person(member) => Some(&member.user),
						_ => None,
					})
				})
				.chain(
					recipients
						.filter(|_| members.is_none())
						.into_iter()
						.flat_map(|c| &c.recipients),
				);
			for value in users.filter(|u| u.id.0 != 0) {
				let value = user(value);
				if group.items.iter().any(|item| item.id == value.id) {
					continue;
				}
				if !push(&mut group.items, value, &mut budget, MAX_APP_MEMBERS) {
					group.truncated = true;
					break;
				}
			}
			group.truncated |= members.is_some_and(|m| m.total > group.items.len() as u64);
			app.members = Some(group);
		}
		if granted(Capability::Presence) && (members.is_some() || recipients.is_some()) {
			let mut presence = PresenceSnapshot {
				items: Vec::new(),
				truncated: false,
			};
			let mut budget = 6 * 1024;
			let statuses = members
				.into_iter()
				.flat_map(|m| {
					m.slots
						.iter()
						.flatten()
						.filter_map(|slot| match slot {
							model::MemberSlot::Person(member) => Some(member),
							_ => None,
						})
						.filter_map(|m| m.status.as_deref().map(|status| (m.user.id, status)))
				})
				.chain(
					recipients
						.filter(|_| members.is_none())
						.into_iter()
						.flat_map(|c| &c.recipients)
						.filter_map(|u| {
							state
								.presence_for(u.id)
								.and_then(|p| p.status.as_deref())
								.map(|s| (u.id, s))
						}),
				);
			for (user, status) in statuses {
				let user_id = user.0.to_string();
				if presence.items.iter().any(|item| item.user_id == user_id) {
					continue;
				}
				if !push(
					&mut presence.items,
					PresenceEntry {
						user_id,
						status: status.into(),
					},
					&mut budget,
					MAX_APP_PRESENCES,
				) {
					presence.truncated = true;
					break;
				}
			}
			presence.truncated |= members.is_some_and(|m| m.total > presence.items.len() as u64);
			app.presence = Some(presence);
		}
	}
	if granted(Capability::VoiceState) {
		let call = state.voice.active.as_ref().filter(|call| {
			state.can_view(call.channel)
				&& !(state.selected == Some(call.channel)
					&& state.freshness == Freshness::Unavailable)
		});
		app.voice = Some(VoiceSnapshot {
			channel_id: call.map(|c| c.channel.0.to_string()),
			phase: call.map_or("idle", |c| phase(c.phase)).into(),
			muted: call.is_some_and(|c| c.muted),
			deafened: call.is_some_and(|c| c.deafened),
			camera: call.is_some_and(|c| c.camera),
			streaming: call.is_some() && messaging.screen.busy,
			participants: call
				.into_iter()
				.flat_map(|c| &c.participants)
				.filter(|p| p.user.0 != 0)
				.take(MAX_VOICE_PARTICIPANTS)
				.map(|p| p.user.0.to_string())
				.collect(),
		});
	}
	if granted(Capability::ReadState) {
		app.read_state = Some(ReadSnapshot {
			channel_id: selected.map(|id| id.0.to_string()),
			unread: selected.and_then(|id| state.unread(id)),
			mentions: selected.map_or(0, |id| state.mention_count(id)),
		});
	}
	if granted(Capability::LocalSettings) {
		app.settings = Some(messaging.extension_local_settings());
	}
	if granted(Capability::NotificationSettings) {
		app.notification_settings = Some(messaging.extension_notification_settings());
	}
	if granted(Capability::MessageDetails)
		&& let Some(id) = readable
	{
		let mut group = MessageDetailsSnapshot {
			channel_id: id.0.to_string(),
			items: Vec::new(),
			truncated: state.history_before.is_some()
				|| state.history_after.is_some()
				|| !state.older_exhausted,
		};
		// Leave room for group keys/scalars; item budgets include serialized bytes and storage.
		let mut budget = (MAX_APP_SNAPSHOT_BYTES
			.saturating_sub(app.bytes().ok()?)
			.saturating_sub(512))
		.min(8 * 1024);
		for message in state.timeline.iter().rev().filter(|m| {
			m.channel == id
				&& m.id.0 != 0
				&& !m.ephemeral
				&& m.flags & 64 == 0
				&& m.author.id.0 != 0
		}) {
			if !push(
				&mut group.items,
				message_detail(message),
				&mut budget,
				MAX_MESSAGE_DETAILS,
			) {
				group.truncated = true;
				break;
			}
		}
		group.items.reverse();
		app.message_details = Some(group);
	}
	if connected
		&& granted(Capability::Relationships)
		&& (state.friends_known()
			|| state.friend_requests_known()
			|| state.restricted_users_known())
	{
		let mut group = RelationshipsSnapshot {
			items: Vec::new(),
			truncated: false,
			friends_known: state.friends_known(),
			requests_known: state.friend_requests_known(),
			restricted_known: state.restricted_users_known(),
		};
		let mut budget = (MAX_APP_SNAPSHOT_BYTES
			.saturating_sub(app.bytes().ok()?)
			.saturating_sub(512))
		.min(4 * 1024);
		let entries = state
			.friends()
			.filter(|_| group.friends_known)
			.map(|u| (u, RelationshipKind::Friend))
			.chain(
				state
					.pending_friends()
					.filter(|_| group.requests_known)
					.map(|(u, _, incoming)| {
						(
							u,
							if *incoming {
								RelationshipKind::IncomingRequest
							} else {
								RelationshipKind::OutgoingRequest
							},
						)
					}),
			)
			.chain(
				state
					.restricted_users()
					.filter(|_| group.restricted_known)
					.map(|(u, _, ignored)| {
						(
							u,
							if *ignored {
								RelationshipKind::Ignored
							} else {
								RelationshipKind::Blocked
							},
						)
					}),
			);
		for (value, kind) in entries.filter(|(u, _)| u.id.0 != 0) {
			let value = user(value);
			if group
				.items
				.iter()
				.any(|row: &RelationshipSnapshot| row.user.id == value.id)
			{
				continue;
			}
			if !push(
				&mut group.items,
				RelationshipSnapshot { user: value, kind },
				&mut budget,
				MAX_RELATIONSHIPS,
			) {
				group.truncated = true;
				break;
			}
		}
		app.relationships = Some(group);
	}
	if granted(Capability::ChannelMetadata) {
		app.channel_metadata = channel_metadata(state);
		if matches!(app.bytes(), Err(Error::Limit)) {
			app.channel_metadata = None;
		}
	}
	if granted(Capability::MemberDetails) {
		app.member_details = crate::extension_member_details::snapshot(state);
		while matches!(app.bytes(), Err(Error::Limit)) {
			let group = app.member_details.as_mut()?;
			if group.items.pop().is_some() {
				group.truncated = true;
			} else if group
				.roles
				.as_mut()
				.is_some_and(|roles| roles.pop().is_some())
			{
				group.roles_truncated = true;
			} else {
				app.member_details = None;
				break;
			}
		}
	}
	if granted(Capability::MessageContent) {
		app.message_content = crate::extension_message_content::snapshot(state);
		while matches!(app.bytes(), Err(Error::Limit)) {
			let group = app.message_content.as_mut()?;
			if group.items.pop().is_some() {
				group.truncated = true;
			} else {
				app.message_content = None;
				break;
			}
		}
	}
	if granted(Capability::ForumData) {
		app.forum_data = crate::extension_forum_data::snapshot(state);
		while matches!(app.bytes(), Err(Error::Limit)) {
			let group = app.forum_data.as_mut()?;
			if group.posts.pop().is_some() {
				group.truncated = true;
			} else {
				app.forum_data = None;
				break;
			}
		}
	}
	if granted(Capability::ConversationActivity) {
		app.conversation_activity = conversation_activity(state);
		if matches!(app.bytes(), Err(Error::Limit)) {
			app.conversation_activity = None;
		}
	}
	// Keep the boundary authoritative if model data or serialization changes later.
	app.validate(manifest).ok()?;
	Some(Box::new(app))
}

fn conversation_activity(state: &State) -> Option<ConversationActivitySnapshot> {
	if !available(state) || !state.gateway_connected || state.freshness != Freshness::Fresh {
		return None;
	}
	let id = state.selected?;
	if !state.can_view(id) || !state.can_read_history(id) {
		return None;
	}
	let pins = state
		.search
		.as_ref()
		.filter(|view| view.pins && view.channel == id && !view.loading && view.error.is_none())
		.and_then(|view| view.page.as_ref());
	let pinned_message_ids = pins.map(|page| {
		page.hits
			.iter()
			.filter(|hit| hit.channel == id && hit.id.0 != 0)
			.take(20)
			.map(|hit| hit.id.0.to_string())
			.collect()
	});
	let group = ConversationActivitySnapshot {
		channel_id: id.0.to_string(),
		typing_user_ids: state
			.typing_users(std::time::Instant::now())
			.take(8)
			.filter(|id| id.0 != 0)
			.map(|id| id.0.to_string())
			.collect(),
		pinned_message_ids,
		pins_truncated: pins.is_some_and(|page| {
			page.partial
				|| page.pin_cursor.is_some()
				|| page.hits.len() > 20
				|| page.total > page.hits.len() as u64
		}),
	};
	group.validate().ok()?;
	Some(group)
}

fn phase(phase: client_core::voice::Phase) -> &'static str {
	use client_core::voice::Phase::*;
	match phase {
		Connecting => "connecting",
		ConnectingTransport => "connecting_transport",
		Discovering => "discovering",
		OpeningAudio => "opening_audio",
		Ringing => "ringing",
		Securing => "securing",
		Connected => "connected",
		Waiting => "waiting",
		Failed => "failed",
	}
}

/// Fixed-size change key: no copied messages, user names, or per-frame plugin execution.
#[derive(PartialEq, Eq)]
pub struct ChangeKey {
	channel: Option<Id>,
	connected: bool,
	context: (Freshness, bool, Option<Freshness>),
	voice: Option<VoiceKey>,
	settings: LocalSettingsSnapshot,
	notification_settings: NotificationSettingsSnapshot,
}
#[derive(PartialEq, Eq)]
struct VoiceKey {
	channel: Id,
	request: u64,
	phase: client_core::voice::Phase,
	muted: bool,
	deafened: bool,
	camera: bool,
	streaming: bool,
	participants: u64,
}
impl ChangeKey {
	pub fn capture(state: &State, messaging: &ui::MessagingUi) -> Self {
		use std::hash::{Hash, Hasher};
		Self {
			channel: state.selected,
			connected: state.gateway_connected,
			context: (
				state.freshness,
				state.history_pending,
				state
					.members
					.as_ref()
					.filter(|members| Some(members.channel) == state.selected)
					.map(|members| members.freshness),
			),
			voice: state.voice.active.as_ref().map(|call| {
				let mut participants = std::collections::hash_map::DefaultHasher::new();
				for p in call.participants.iter().take(MAX_VOICE_PARTICIPANTS) {
					(p.user.0, p.muted, p.deafened, p.video, p.streaming).hash(&mut participants);
				}
				VoiceKey {
					channel: call.channel,
					request: call.request,
					phase: call.phase,
					muted: call.muted,
					deafened: call.deafened,
					camera: call.camera,
					streaming: messaging.screen.busy,
					participants: participants.finish(),
				}
			}),
			settings: messaging.extension_local_settings(),
			notification_settings: messaging.extension_notification_settings(),
		}
	}
	pub fn changed(&self, old: &Self) -> Option<AppEventKind> {
		if self.connected != old.connected {
			Some(AppEventKind::Connection)
		} else if self.channel != old.channel {
			Some(AppEventKind::Navigation)
		} else if self.context != old.context {
			Some(AppEventKind::Context)
		} else if self.voice != old.voice {
			Some(AppEventKind::Voice)
		} else if self.settings != old.settings
			|| self.notification_settings != old.notification_settings
		{
			Some(AppEventKind::Settings)
		} else {
			None
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn manifest(capabilities: Vec<Capability>) -> Manifest {
		Manifest {
			api_version: API_VERSION,
			id: "snapshot-test".into(),
			name: "Synthetic test".into(),
			version: "1".into(),
			author: "Tests".into(),
			license: "MIT".into(),
			source: "https://example.org/source".into(),
			kind: ExtensionKind::Plugin,
			capabilities,
			actions: vec![Action {
				id: "show".into(),
				label: "Show".into(),
				surface: Surface::Panel,
			}],
		}
	}
	#[test]
	fn extension_app_notification_grant_and_settings_events_use_local_preferences() {
		let state = test_support::demo_state();
		let mut messaging = ui::MessagingUi::default();
		let reading = snapshot(
			&state,
			&messaging,
			&manifest(vec![Capability::LocalSettings]),
		)
		.unwrap();
		assert!(reading.notification_settings.is_none());
		assert_eq!(reading.settings.unwrap().scroll_speed_percent, Some(100));
		let caps = manifest(vec![Capability::NotificationSettings]);
		assert!(uses_app(&caps.capabilities));
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert!(app.settings.is_none() && app.context.is_none());
		assert_eq!(
			app.notification_settings.unwrap(),
			messaging.extension_notification_settings()
		);
		let before = ChangeKey::capture(&state, &messaging);
		messaging.notification_options.volume = 10;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&before),
			Some(AppEventKind::Settings)
		);
		let mut preferences = crate::app_settings::Settings::default();
		preferences.observe(&messaging);
		assert!(preferences.state.dirty);
		let mut restored = ui::MessagingUi::default();
		preferences.apply(&mut restored);
		assert_eq!(
			restored.notification_options,
			messaging.notification_options
		);
		let before = ChangeKey::capture(&state, &messaging);
		messaging.reading_preferences.scroll_speed_percent = 150;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&before),
			Some(AppEventKind::Settings)
		);
	}
	#[test]
	fn extension_app_message_metadata_and_relationships_are_scoped_and_bounded() {
		let mut state = test_support::demo_state();
		let messaging = ui::MessagingUi::default();
		let caps = manifest(vec![Capability::MessageDetails, Capability::Relationships]);
		let mut message = test_support::message(9000, Id(20));
		message.reply_to = Some(Id(8999));
		message.mentions = (1..=40)
			.map(|id| model::User {
				id: Id(id),
				..state.user.clone().unwrap()
			})
			.collect();
		message.attachments = vec![model::Attachment {
			id: Id(70),
			filename: "report.txt".into(),
			description: Some("PRIVATE_DESCRIPTION".into()),
			content_type: Some("text/plain".into()),
			size: 123,
			media: model::EmbedMedia {
				url: Some("https://example.org/PRIVATE_URL".into()),
				..Default::default()
			},
			spoiler: true,
			duration_ms: None,
			waveform: vec![],
		}];
		message.reactions = Some(
			(0..20)
				.map(|id| model::Reaction {
					emoji: model::ReactionEmoji {
						id: Some(Id(100 + id)),
						name: Some("test".into()),
					},
					count: 2,
					me: true,
					me_burst: false,
				})
				.collect(),
		);
		let detail = message_detail(&message);
		assert_eq!(detail.mention_ids.len(), MAX_MESSAGE_MENTIONS);
		assert!(detail.mentions_truncated && detail.reactions_truncated);
		assert!(detail.reactions.as_ref().unwrap().len() <= MAX_MESSAGE_REACTIONS);
		assert_eq!(detail.attachments[0].filename, "report.txt");
		assert_eq!(detail.reply_to.as_deref(), Some("8999"));
		let wire = serde_json::to_string(&detail).unwrap();
		assert!(!wire.contains("PRIVATE_") && !wire.contains(&message.content));
		let mut malformed = message.clone();
		malformed.attachments[0].filename = "\n\t".into();
		malformed.attachments[0].content_type = Some("\n".into());
		malformed.attachments.push(malformed.attachments[0].clone());
		let sanitized = message_detail(&malformed);
		assert_eq!(sanitized.attachments.len(), 1);
		assert_eq!(sanitized.attachments[0].filename, "Attachment");
		assert!(sanitized.attachments[0].content_type.is_none() && sanitized.attachments_truncated);
		AppSnapshot {
			message_details: Some(MessageDetailsSnapshot {
				channel_id: "20".into(),
				items: vec![sanitized],
				truncated: false,
			}),
			..Default::default()
		}
		.validate(&caps)
		.unwrap();
		message.mentions.clear();
		state
			.timeline
			.insert(message.clone(), false, false)
			.unwrap();
		let mut private = message.clone();
		private.id = Id(9001);
		private.ephemeral = true;
		private.flags |= 64;
		state.timeline.insert(private, false, false).unwrap();
		state.set_preserve_deleted_messages(true);
		let mut deleted = message;
		deleted.id = Id(9002);
		state.timeline.insert(deleted, false, false).unwrap();
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Delete {
				channel: Id(20),
				id: Id(9002),
			},
		});
		let person = |id| model::User {
			id: Id(id),
			..state.user.clone().unwrap()
		};
		let people: Vec<_> = (800..805).map(person).collect();
		use client_core::user_actions::Event as U;
		for event in [
			U::Relationships(Some(vec![
				(Id(800), false),
				(Id(801), false),
				(Id(802), false),
				(Id(803), true),
				(Id(804), false),
			])),
			U::Friends(Some(vec![(people[0].clone(), "friend".into())])),
			U::Requests(Some(vec![
				(people[1].clone(), "incoming".into(), true),
				(people[2].clone(), "outgoing".into(), false),
			])),
			U::Restrictions(Some(vec![
				(people[3].clone(), "blocked".into(), false),
				(people[4].clone(), "ignored".into(), true),
			])),
		] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::UserAction(event),
			});
		}
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert!(app.bytes().unwrap() <= MAX_APP_SNAPSHOT_BYTES);
		let details = app.message_details.as_ref().unwrap();
		assert!(details.items.iter().any(|m| m.id == "9000"));
		assert!(
			!details
				.items
				.iter()
				.any(|m| matches!(m.id.as_str(), "9001" | "9002"))
		);
		let relationships = app.relationships.as_ref().unwrap();
		assert!(
			relationships.friends_known
				&& relationships.requests_known
				&& relationships.restricted_known
		);
		assert_eq!(relationships.items.len(), 5);
		assert_eq!(relationships.items[0].kind, RelationshipKind::Friend);
		assert_eq!(relationships.items[4].kind, RelationshipKind::Ignored);
		let no_grants =
			snapshot(&state, &messaging, &manifest(vec![Capability::AppContext])).unwrap();
		assert!(no_grants.message_details.is_none() && no_grants.relationships.is_none());
		state.freshness = Freshness::Stale;
		assert!(
			snapshot(&state, &messaging, &caps)
				.unwrap()
				.message_details
				.is_none()
		);
		state.gateway_connected = false;
		let disconnected = snapshot(&state, &messaging, &caps).unwrap();
		assert!(disconnected.message_details.is_none() && disconnected.relationships.is_none());
	}
	#[test]
	fn extension_app_new_data_requires_grants_and_stays_bounded_and_current() {
		let mut state = test_support::demo_state();
		let messaging = ui::MessagingUi::default();
		let current = state.user.clone().unwrap();
		state.own_profile.data = Some(model::UserProfile {
			user: current.clone(),
			username: current.name.clone(),
			global_name: Some("Synthetic display".into()),
			banner: None,
			accent_color: None,
			bio: "\\".repeat(4096),
			pronouns: "they/them".into(),
			badges: vec![],
			connections: vec![],
			mutual_guilds: vec![],
			guild: None,
			theme_colors: None,
			clan: None,
			limited: false,
		});
		let guild = state.guilds[0].clone();
		state.guilds.extend((1..=150).map(|id| model::Guild {
			id: Id(10000 + id),
			name: "Synthetic server ".repeat(20),
			..guild.clone()
		}));
		let friends = (1000..1150)
			.map(|id| {
				(
					model::User {
						id: Id(id),
						..current.clone()
					},
					format!("friend{id}"),
				)
			})
			.collect();
		for event in [
			client_core::user_actions::Event::Relationships(Some(
				(1000..1150).map(|id| (Id(id), false)).collect(),
			)),
			client_core::user_actions::Event::Friends(Some(friends)),
		] {
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::UserAction(event),
			});
		}
		let selected = state.selected.unwrap();
		state
			.channels
			.iter_mut()
			.find(|c| c.id == selected)
			.unwrap()
			.recipients = (1..=50)
			.map(|id| model::User {
				id: Id(20000 + id),
				name: "Synthetic recipient ".repeat(20),
				..current.clone()
			})
			.collect();
		let caps = manifest(vec![
			Capability::AccountProfile,
			Capability::MessageDetails,
			Capability::Relationships,
			Capability::GuildDirectory,
			Capability::ChannelDetails,
			Capability::AppContext,
			Capability::ChannelDirectory,
			Capability::Timeline,
			Capability::Members,
			Capability::Presence,
			Capability::VoiceState,
			Capability::ReadState,
			Capability::LocalSettings,
			Capability::NotificationSettings,
		]);
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert!(app.relationships.as_ref().unwrap().truncated);
		assert!(app.message_details.is_some());

		let account = app.account_profile.as_ref().unwrap();
		assert_eq!(account.user.id, current.id.0.to_string());
		assert_eq!(account.profile.as_ref().unwrap().bio.len(), 2048);
		assert!(app.guilds.as_ref().unwrap().truncated);
		assert!(app.guilds.as_ref().unwrap().items.len() <= MAX_APP_GUILDS);
		let details = app.channel_details.as_ref().unwrap();
		assert!(details.recipients_truncated && details.recipients.len() <= MAX_CHANNEL_RECIPIENTS);
		assert!(app.bytes().unwrap() <= MAX_APP_SNAPSHOT_BYTES);
		let ungranted =
			snapshot(&state, &messaging, &manifest(vec![Capability::AppContext])).unwrap();
		assert!(
			ungranted.account_profile.is_none()
				&& ungranted.guilds.is_none()
				&& ungranted.channel_details.is_none()
		);
		state.own_profile.data.as_mut().unwrap().user.id = Id(999);
		assert!(
			snapshot(&state, &messaging, &caps)
				.unwrap()
				.account_profile
				.as_ref()
				.unwrap()
				.profile
				.is_none()
		);
		state.freshness = Freshness::Unavailable;
		assert!(
			snapshot(&state, &messaging, &caps)
				.unwrap()
				.channel_details
				.is_none()
		);
		state.gateway_connected = false;
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert!(
			app.account_profile.is_none() && app.guilds.is_none() && app.channel_details.is_none()
		);
	}

	#[test]
	fn extension_activity_distinguishes_unloaded_pins_and_live_typing() {
		let mut state = test_support::demo_state();
		state.demo = false;
		state.auth = AuthState::Authenticated;
		state.history_pending = false;
		let channel = state.selected.unwrap();
		let now = std::time::Instant::now();
		let wall = std::time::SystemTime::now();
		state.observe_typing_at(
			client_core::typing::Signal {
				channel,
				user: Id(9876),
				timestamp: wall
					.duration_since(std::time::UNIX_EPOCH)
					.unwrap()
					.as_secs(),
			},
			wall,
			now,
		);
		let activity = conversation_activity(&state).unwrap();
		assert_eq!(activity.typing_user_ids, vec!["9876"]);
		assert!(activity.pinned_message_ids.is_none());
		let before_pins = crate::extension_data_events::DataKey::capture(&state);
		state.search = Some(client_core::search::SearchView {
			pins: true,
			channel,
			query: String::new(),
			before: None,
			pin_before: None,
			request: 0,
			loading: false,
			error: None,
			page: Some(model::SearchPage {
				hits: vec![],
				total: 0,
				partial: false,
				pin_cursor: None,
			}),
		});
		assert!(
			crate::extension_data_events::DataKey::capture(&state)
				.changed(&before_pins)
				.kinds(&[Capability::ConversationActivity])
				.any(|kind| kind == AppEventKind::Pins)
		);
		assert_eq!(
			conversation_activity(&state).unwrap().pinned_message_ids,
			Some(vec![])
		);
		state.search.as_mut().unwrap().loading = true;
		assert!(
			conversation_activity(&state)
				.unwrap()
				.pinned_message_ids
				.is_none()
		);
		let inspector = parse_package(include_bytes!(
			"../../../examples/extensions/packages/conversation-inspector.serein-extension"
		))
		.unwrap();
		let app = snapshot(&state, &ui::MessagingUi::default(), &inspector.manifest).unwrap();
		assert!(
			app.message_content.is_some()
				&& app.forum_data.is_some()
				&& app.conversation_activity.is_some()
		);
		let output = invoke(
			&inspector,
			&Invocation {
				action: "show".into(),
				app: Some(app),
				..Default::default()
			},
		)
		.expect("real loaded state fits Conversation Inspector sandbox");
		assert!(!output.panel.is_empty() && output.effects.is_empty());
		let granted = snapshot(
			&state,
			&ui::MessagingUi::default(),
			&manifest(vec![Capability::ConversationActivity]),
		)
		.unwrap();
		assert!(granted.conversation_activity.is_some());
		assert!(
			snapshot(
				&state,
				&ui::MessagingUi::default(),
				&manifest(vec![Capability::AppContext])
			)
			.unwrap()
			.conversation_activity
			.is_none()
		);
	}

	#[test]
	fn extension_channel_metadata_exposes_only_loaded_topics_and_scoped_parents() {
		use client_core::channel_actions::{Action, Edit, Event, Outcome};
		let mut state = test_support::demo_state();
		let id = state.selected.unwrap();
		let original = state.channel(id).unwrap().clone();
		let guild = original.guild.unwrap();
		let mut category = original.clone();
		category.id = Id(88001);
		category.kind = 4;
		category.name = "Category".into();
		category.parent_id = None;
		state.channels.push(category);
		state.permissions.channels.insert(
			Id(88001),
			model::permissions::Channel {
				id: Id(88001),
				guild,
				overwrites: Some(vec![]),
			},
		);
		state.permissions.guilds.get_mut(&guild).unwrap().owner =
			state.user.as_ref().map(|user| user.id);
		state.permissions.clear_cache();
		let index = state.channel_index(id).unwrap();
		state.channels[index].parent_id = Some(Id(88001));
		let unknown = channel_metadata(&state).unwrap();
		assert!(unknown.topic.is_none() && unknown.thread.is_none());
		assert_eq!(unknown.category.as_ref().unwrap().name, "Category");
		let client_core::Command::ChannelAction { request, .. } =
			state.request_channel_action(id, Action::Load).unwrap()
		else {
			panic!("expected synthetic load command")
		};
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::ChannelAction(Event::Finished {
				guild,
				channel: id,
				request,
				result: Ok(Outcome::Details(Edit {
					name: original.name,
					topic: "\u{e9}".repeat(1200),
					slowmode: 5,
					nsfw: false,
					overwrites: vec![],
				})),
			}),
		});
		let loaded = channel_metadata(&state).unwrap();
		loaded.validate().unwrap();
		assert_eq!(loaded.topic.as_ref().unwrap().len(), 2048);
		assert!(loaded.topic_truncated);
		assert_eq!(loaded.slowmode_seconds, Some(5));
		let messaging = ui::MessagingUi::default();
		let granted = snapshot(
			&state,
			&messaging,
			&manifest(vec![Capability::ChannelMetadata]),
		)
		.unwrap();
		assert!(granted.channel_metadata.is_some());
		let inspector = parse_package(include_bytes!(
			"../../../examples/extensions/packages/guild-inspector.serein-extension"
		))
		.unwrap();
		let output = invoke(
			&inspector,
			&Invocation {
				action: "show".into(),
				app: Some(granted),
				..Default::default()
			},
		)
		.expect("loaded channel metadata fits the real Guild Inspector sandbox");
		assert!(!output.panel.is_empty() && output.effects.is_empty());
		let denied = snapshot(
			&state,
			&messaging,
			&manifest(vec![Capability::ChannelDetails]),
		)
		.unwrap();
		assert!(denied.channel_metadata.is_none());
		state.demo = false;
		state.auth = AuthState::Authenticated;
		let before_reload = crate::extension_data_events::DataKey::capture(&state);
		state.request_channel_action(id, Action::Load).unwrap();
		assert!(channel_metadata(&state).unwrap().topic.is_none());
		assert!(
			crate::extension_data_events::DataKey::capture(&state)
				.changed(&before_reload)
				.kinds(&[Capability::ChannelMetadata])
				.any(|kind| kind == AppEventKind::Channels)
		);
		let mut parent = state.channel(id).unwrap().clone();
		parent.id = Id(88002);
		state.channels.push(parent);
		state.permissions.channels.insert(
			Id(88002),
			model::permissions::Channel {
				id: Id(88002),
				guild,
				overwrites: Some(vec![]),
			},
		);
		state.channels[index].kind = 11;
		state.channels[index].parent_id = Some(Id(88002));
		state.channels[index].message_count = Some(7);
		let thread = channel_metadata(&state).unwrap();
		assert_eq!(thread.category.as_ref().unwrap().id, "88001");
		let thread = thread.thread.unwrap();
		assert_eq!(thread.message_count, Some(7));
		assert!(thread.archived.is_none() && thread.locked.is_none());
		state.channels.last_mut().unwrap().guild = Some(Id(999));
		assert!(channel_metadata(&state).is_none());
	}

	#[test]
	fn extension_app_snapshot_is_granted_current_and_byte_bounded() {
		let mut state = test_support::demo_state();
		let messaging = ui::MessagingUi::default();
		assert!(snapshot(&state, &messaging, &manifest(vec![Capability::Storage])).is_none());
		let caps = manifest(vec![
			Capability::AccountProfile,
			Capability::MessageDetails,
			Capability::Relationships,
			Capability::GuildDirectory,
			Capability::ChannelDetails,
			Capability::AppContext,
			Capability::Timeline,
			Capability::ChannelDirectory,
			Capability::Members,
			Capability::Presence,
			Capability::VoiceState,
			Capability::ReadState,
			Capability::LocalSettings,
			Capability::NotificationSettings,
		]);
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert_eq!(
			app.context.as_ref().unwrap().channel.as_ref().unwrap().id,
			"20"
		);
		assert!(app.bytes().unwrap() <= MAX_APP_SNAPSHOT_BYTES);
		assert!(app.timeline.as_ref().unwrap().messages.len() <= MAX_MESSAGE_DETAILS);
		assert!(app.timeline.as_ref().unwrap().truncated);
		let toolbox = parse_package(include_bytes!(
			"../../../examples/extensions/packages/app-toolbox.serein-extension"
		))
		.unwrap();
		let output = invoke(
			&toolbox,
			&Invocation {
				action: "show".into(),
				app: Some(app),
				..Default::default()
			},
		)
		.expect("the real demo snapshot must fit App Toolbox's unchanged sandbox budget");
		assert!(!output.panel.is_empty() && output.effects.is_empty());
		let app = snapshot(
			&state,
			&messaging,
			&manifest(vec![Capability::LocalSettings]),
		)
		.unwrap();
		assert!(app.context.is_none() && app.channels.is_none() && app.timeline.is_none());
		assert!(app.settings.is_some());
		let member = model::Member {
			user: state.user.clone().unwrap(),
			roles: Vec::new(),
			nick: None,
			status: Some("online".into()),
			custom_status: None,
			activities: Vec::new(),
		};
		state.members = Some(model::MemberList {
			guild: Some(Id(10)),
			channel: Id(20),
			request: state.member_request,
			start: 0,
			slots: vec![
				Some(model::MemberSlot::Person(member.clone())),
				Some(model::MemberSlot::Person(member)),
			],
			lazy: false,
			groups: vec![],
			ranges: vec![],
			total: 2,
			freshness: Freshness::Loading,
		});
		let loading = ChangeKey::capture(&state, &messaging);
		state.members.as_mut().unwrap().freshness = Freshness::Fresh;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&loading),
			Some(AppEventKind::Context)
		);
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert_eq!(app.members.as_ref().unwrap().items.len(), 1);
		assert_eq!(app.presence.as_ref().unwrap().items.len(), 1);
		state.freshness = Freshness::Unavailable;
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert!(app.context.as_ref().unwrap().channel.is_none());
		assert!(app.members.is_none() && app.presence.is_none() && app.timeline.is_none());
		assert!(
			app.channels
				.as_ref()
				.unwrap()
				.items
				.iter()
				.all(|channel| channel.id != "20")
		);
		state.gateway_connected = false;
		let app = snapshot(&state, &messaging, &caps).unwrap();
		assert!(!app.context.as_ref().unwrap().connected);
		assert!(
			app.channels.is_none()
				&& app.timeline.is_none()
				&& app.members.is_none()
				&& app.presence.is_none()
		);
		state.demo = false;
		state.auth = AuthState::Unauthenticated;
		assert!(snapshot(&state, &messaging, &caps).is_none());
	}
	#[test]
	fn extension_app_timeline_excludes_private_deleted_and_oversized_rows() {
		let mut state = test_support::demo_state();
		state.set_preserve_deleted_messages(true);
		let caps = manifest(vec![Capability::Timeline]);
		for (id, text, ephemeral) in [
			(2001, "private".into(), true),
			(2002, "x".repeat(4097), false),
			(2003, "removed".into(), false),
			(2004, "visible".into(), false),
		] {
			let mut message = test_support::message(id, Id(20));
			message.content = text;
			message.ephemeral = ephemeral;
			state.apply(client_core::Envelope {
				generation: state.generation,
				event: client_core::Event::Message(message),
			});
		}
		state.apply(client_core::Envelope {
			generation: state.generation,
			event: client_core::Event::Delete {
				channel: Id(20),
				id: Id(2003),
			},
		});
		let app = snapshot(&state, &ui::MessagingUi::default(), &caps).unwrap();
		let timeline = app.timeline.as_ref().unwrap();
		assert!(timeline.messages.iter().any(|m| m.id == "2004"));
		assert!(
			timeline
				.messages
				.iter()
				.all(|m| !["2001", "2002", "2003"].contains(&m.id.as_str()))
		);
		assert!(timeline.truncated);
		state.freshness = Freshness::Unavailable;
		assert!(
			snapshot(&state, &ui::MessagingUi::default(), &caps)
				.unwrap()
				.timeline
				.is_none()
		);
	}
	#[test]
	fn extension_app_change_key_is_idle_until_meaningful_change() {
		let mut state = test_support::demo_state();
		let mut messaging = ui::MessagingUi::default();
		let first = ChangeKey::capture(&state, &messaging);
		state.revision += 1;
		assert_eq!(ChangeKey::capture(&state, &messaging).changed(&first), None);
		state.history_pending = !state.history_pending;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&first),
			Some(AppEventKind::Context)
		);
		state.history_pending = !state.history_pending;
		state.freshness = Freshness::Loading;
		let loading = ChangeKey::capture(&state, &messaging);
		state.freshness = Freshness::Fresh;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&loading),
			Some(AppEventKind::Context)
		);
		messaging.reading_preferences.zoom_percent = 110;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&first),
			Some(AppEventKind::Settings)
		);
		state.selected = None;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&first),
			Some(AppEventKind::Navigation)
		);
		state.gateway_connected = false;
		assert_eq!(
			ChangeKey::capture(&state, &messaging).changed(&first),
			Some(AppEventKind::Connection)
		);
	}
}
