//! Bounded invalidation hints for granted app data, never copied account payloads.
use client_core::{Envelope, Event, State};
use extensions::{AppEventKind, Capability};
use model::Id;

const KINDS: [AppEventKind; 15] = [
	AppEventKind::Account,
	AppEventKind::Channels,
	AppEventKind::Members,
	AppEventKind::Presence,
	AppEventKind::ReadState,
	AppEventKind::MessageDetails,
	AppEventKind::Relationships,
	AppEventKind::Threads,
	AppEventKind::Roles,
	AppEventKind::Permissions,
	AppEventKind::Recovered,
	AppEventKind::Reactions,
	AppEventKind::Pins,
	AppEventKind::Typing,
	AppEventKind::Polls,
];

#[derive(Default, Clone, Copy)]
pub struct Changes([bool; 15], Option<bool>);
impl Changes {
	pub fn capture(state: &State, envelope: &Envelope) -> Self {
		let mut changes = Self::default();
		if envelope.generation != state.generation {
			return changes;
		}
		let readable = |channel| {
			state.selected == Some(channel)
				&& state.gateway_connected
				&& state.can_view(channel)
				&& state.can_read_history(channel)
				&& state.freshness != model::Freshness::Unavailable
		};
		match &envelope.event {
			Event::Startup(_) | Event::Ready { .. } => {
				changes.0[..10].fill(true);
				changes.1 = Some(true);
			}
			Event::Resumed => changes.1 = Some(true),
			Event::Disconnected => changes.1 = Some(false),
			Event::ProfileEdited { user, .. } | Event::Profile { user, .. }
				if state.user.as_ref().is_some_and(|own| own.id == *user) =>
			{
				changes.0[0] = true;
				// Own-user edits also refresh copies in loaded recipients/member rows.
				changes.0[1] = true;
				changes.0[2] = true;
			}
			Event::GuildJoined(_)
			| Event::GuildChanged(_)
			| Event::ChannelCreated(_)
			| Event::ChannelRestored(_)
			| Event::ChannelChanged(_)
			| Event::ThreadChanged { .. }
			| Event::ThreadRemoved { .. }
			| Event::ThreadsSync { .. }
			| Event::ServerAction(_)
			| Event::ServerSettings(_)
			| Event::ChannelAction(_)
			| Event::GroupAction(_)
			| Event::UserAction(_)
			| Event::PostCreated { result: Ok(_), .. }
			| Event::ForumPosts { result: Ok(_), .. }
			| Event::Permissions(_)
			| Event::PermissionsChanged
			| Event::Unavailable(_) => changes.0[1] = true,
			Event::Message(message)
			| Event::SendResult {
				result: Ok(message),
				..
			} if readable(message.channel) && !message.ephemeral && message.flags & 64 == 0 => {
				changes.0[1] = true
			}
			Event::History {
				channel, request, ..
			} if readable(*channel) && *request == state.request => changes.0[1] = true,
			Event::Delete { channel, .. } | Event::DeleteBulk { channel, .. }
				if readable(*channel) =>
			{
				changes.0[1] = true
			}
			Event::ReadState(client_core::read_state::Event::Latest(entries))
				if entries.iter().any(|(channel, _)| readable(*channel)) =>
			{
				changes.0[1] = true
			}
			Event::Members(members)
				if state.selected == Some(members.channel)
					&& state.member_request == members.request =>
			{
				changes.0[2] = true;
				changes.0[3] = true;
			}
			Event::RecipientAdded { channel, .. } | Event::RecipientRemoved { channel, .. }
				if state.selected == Some(*channel) =>
			{
				changes.0[1] = true;
				changes.0[2] = true;
				changes.0[3] = true;
			}
			Event::MemberPresence {
				channel, request, ..
			} if state.selected == Some(*channel) && state.member_request == *request => changes.0[3] = true,
			Event::DirectPresence(updates)
				if state
					.selected
					.and_then(|id| state.channel(id))
					.filter(|channel| channel.guild.is_none())
					.is_some_and(|channel| {
						updates.iter().any(|update| {
							channel.recipients.iter().any(|user| user.id == update.user)
						})
					}) =>
			{
				changes.0[3] = true
			}
			_ => {}
		}
		let channel = state.selected.and_then(|id| state.channel(id));
		let guild = channel.and_then(|channel| channel.guild);
		let in_guild = |id| guild == Some(id);
		if let Event::Profile {
			guild: profile_guild,
			..
		} = &envelope.event
			&& guild.is_some()
			&& *profile_guild == guild
		{
			changes.0[2] = true;
		}
		changes.0[7] |= match &envelope.event {
			Event::ThreadChanged { guild, .. }
			| Event::ThreadRemoved { guild, .. }
			| Event::ThreadsSync { guild, .. } => in_guild(*guild),
			Event::ChannelCreated(value)
			| Event::ChannelRestored(value)
			| Event::PostCreated {
				result: Ok(value), ..
			} => matches!(value.kind, 10..=12) && value.guild.is_some_and(in_guild),
			Event::ChannelChanged(patch) => state.channel(patch.id).is_some_and(|value| {
				matches!(value.kind, 10..=12) && value.guild.is_some_and(in_guild)
			}),
			Event::Unavailable(id) => state.channel(*id).is_some_and(|value| {
				matches!(value.kind, 10..=12) && value.guild.is_some_and(in_guild)
			}),
			Event::ForumPosts {
				parent,
				result: Ok(_),
				..
			}
			| Event::Archives {
				parent,
				result: Ok(_),
				..
			} => state
				.channel(*parent)
				.and_then(|value| value.guild)
				.is_some_and(in_guild),
			_ => false,
		};
		if let Event::Permissions(event) = &envelope.event {
			use client_core::permissions::Event::*;
			let (roles, permissions, members) = match event {
				Snapshot(_) => (guild.is_some(), channel.is_some(), guild.is_some()),
				Guild(value) if in_guild(value.id) => (true, true, true),
				Role { guild, .. } | RoleRemoved { guild, .. } if in_guild(*guild) => {
					(true, true, true)
				}
				Member { guild, .. } if in_guild(*guild) => (false, true, true),
				Members(values) if values.iter().any(|(guild, _, _)| in_guild(*guild)) => {
					(false, true, true)
				}
				Owner { guild, .. } if in_guild(*guild) => (false, true, false),
				UnavailableGuild(guild) if in_guild(*guild) => (true, true, true),
				Channel {
					channel: target, ..
				} if channel.is_some_and(|channel| {
					channel.id == *target || channel.parent_id == Some(*target)
				}) =>
				{
					(false, true, false)
				}
				_ => (false, false, false),
			};
			changes.0[8] |= roles;
			changes.0[9] |= permissions;
			changes.0[2] |= members;
		}
		if matches!(&envelope.event, Event::PermissionsChanged | Event::Resync) {
			changes.0[8] |= guild.is_some();
			changes.0[9] |= channel.is_some();
			changes.0[2] |= guild.is_some();
		}
		if let Event::ServerAdmin(event) = &envelope.event
			&& in_guild(event.guild)
			&& matches!(&event.result, Ok(model::server_admin::Result::Roles(_)))
		{
			changes.0[8] = true;
			changes.0[9] = true;
			changes.0[2] = true;
		}
		changes.0[5] |= message_details_changed(state, &envelope.event);
		if changes.0[5] {
			let ordinary = |m: &model::Message| {
				state.selected == Some(m.channel)
					&& !m.ephemeral && m.flags & 64 == 0
					&& m.author.id.0 != 0
					&& !state.timeline.is_deleted(m.id)
			};
			let has_reactions = |id| {
				state
					.timeline
					.get(id)
					.is_some_and(|m| ordinary(m) && m.reactions.is_some())
			};
			let has_poll = |id| {
				state
					.timeline
					.get(id)
					.is_some_and(|m| ordinary(m) && m.extra_content.poll)
			};
			changes.0[11] |= match &envelope.event {
				Event::Reactions(_) => true,
				Event::Patch(patch) => !matches!(patch.reactions, model::Patch::Absent),
				Event::Message(m) | Event::SendResult { result: Ok(m), .. } => {
					m.reactions.is_some()
				}
				Event::History { messages, .. } => messages
					.iter()
					.any(|m| ordinary(m) && m.reactions.is_some()),
				Event::Delete { id, .. } => has_reactions(*id),
				Event::DeleteBulk { ids, .. } => ids.iter().any(|id| has_reactions(*id)),
				_ => false,
			};
			changes.0[12] |= matches!(&envelope.event, Event::Pinned { result: Ok(()), .. });
			changes.0[14] |= match &envelope.event {
				Event::Patch(patch) => !matches!(patch.extra_content.poll, model::Patch::Absent),
				Event::Message(m) | Event::SendResult { result: Ok(m), .. } => m.extra_content.poll,
				Event::History { messages, .. } => {
					messages.iter().any(|m| ordinary(m) && m.extra_content.poll)
				}
				Event::Delete { id, .. } => has_poll(*id),
				Event::DeleteBulk { ids, .. } => ids.iter().any(|id| has_poll(*id)),
				_ => false,
			};
		}
		if let Event::UserAction(event) = &envelope.event {
			use client_core::user_actions::Event::*;
			changes.0[6] |=
				matches!(
					event,
					Requests(_)
						| Request { .. } | FriendProfile(_)
						| Friends(_) | Friend { .. }
						| Restrictions(_) | Restriction { .. }
						| Relationships(_) | Relationship { .. }
						| Nicknames(_) | Nickname { .. }
						| Written {
							action: client_core::user_actions::Action::AddFriend { .. }
								| client_core::user_actions::Action::ResolveFriend { .. }
								| client_core::user_actions::Action::ProfileFriend { .. }
								| client_core::user_actions::Action::Nickname { .. }
								| client_core::user_actions::Action::Block { .. },
							..
						}
				);
		}
		changes
	}
	pub fn connection(&self) -> Option<bool> {
		self.1
	}
	pub fn recovered(&mut self) {
		self.0[10] = true;
	}
	pub fn merge(&mut self, other: Self) {
		for (changed, next) in self.0.iter_mut().zip(other.0) {
			*changed |= next;
		}
	}
	pub fn kinds(self, capabilities: &[Capability]) -> impl Iterator<Item = AppEventKind> {
		KINDS
			.into_iter()
			.zip(self.0)
			.filter_map(move |(kind, changed)| {
				(changed && kind.data_granted(capabilities)).then_some(kind)
			})
	}
}

fn message_details_changed(state: &State, event: &Event) -> bool {
	let readable = |channel| {
		state.selected == Some(channel)
			&& state.gateway_connected
			&& state.can_view(channel)
			&& state.can_read_history(channel)
			&& state
				.channel(channel)
				.is_some_and(|channel| channel.supports_text())
	};
	let ordinary = |message: &model::Message| {
		!message.ephemeral
			&& message.flags & 64 == 0
			&& message.author.id.0 != 0
			&& !state.timeline.is_deleted(message.id)
	};
	// History may be completing a loading state. Dispatch still collects only fresh data.
	if let Event::History {
		channel,
		request,
		messages,
		..
	} = event
	{
		return readable(*channel)
			&& *request == state.request
			&& state.history_pending
			&& (messages.is_empty() || messages.iter().any(ordinary));
	}
	if state.freshness != model::Freshness::Fresh {
		return false;
	}
	let loaded = |channel, id| {
		readable(channel)
			&& state
				.timeline
				.get(id)
				.is_some_and(|message| message.channel == channel && ordinary(message))
	};
	match event {
		Event::Message(message)
		| Event::SendResult {
			result: Ok(message),
			..
		} => readable(message.channel) && ordinary(message),
		Event::Patch(patch) => loaded(patch.channel, patch.id),
		Event::Edited {
			channel,
			message,
			result: Ok(_),
			..
		}
		| Event::Pinned {
			channel,
			message,
			result: Ok(()),
			..
		} => loaded(*channel, *message),
		Event::Delete { channel, id } => loaded(*channel, *id),
		Event::DeleteBulk { channel, ids } => ids.iter().any(|id| loaded(*channel, *id)),
		Event::Reactions(event) => {
			use client_core::reactions::Event::*;
			match event {
				Delta {
					channel, message, ..
				}
				| Cleared {
					channel, message, ..
				}
				| Changed { channel, message }
				| Read {
					channel, message, ..
				}
				| Written {
					channel, message, ..
				} => loaded(*channel, *message),
				Users { .. } => false,
			}
		}
		_ => false,
	}
}

/// Fixed scalar keys catch local loading/navigation mutations outside the event drain.
#[derive(PartialEq, Eq)]
pub struct DataKey {
	read: (Option<Id>, Option<bool>, u32),
	profile: (u64, bool, bool, bool, bool),
	channel: Option<(Id, Option<Id>, Option<u32>)>,
	directory: (usize, usize),
	messages: Option<(Id, u64)>,
	relationships: (u64, bool, bool, bool),
	permissions: Option<(Id, bool, bool, bool)>,
	member_profile: Option<(Id, u64, bool, bool, bool)>,
	channel_metadata: Option<(Id, bool, bool)>,
	typing: [Option<Id>; 8],
	pins: Option<(Id, u64, bool, bool, bool)>,
	forum_posts: Option<(Id, u64, bool, bool, bool)>,
	forum_archive: Option<(Id, u64, bool, bool, bool)>,
}
impl DataKey {
	pub fn capture(state: &State) -> Self {
		let selected = state.selected.filter(|id| {
			state.gateway_connected
				&& state.can_view(*id)
				&& state.freshness != model::Freshness::Unavailable
		});
		let mut typing = [None; 8];
		for (slot, user) in typing
			.iter_mut()
			.zip(state.typing_users(std::time::Instant::now()))
		{
			*slot = Some(user);
		}
		typing.sort_unstable();
		let profile = &state.own_profile;
		let guild = selected
			.and_then(|id| state.channel(id))
			.and_then(|channel| channel.guild);
		let parent = selected
			.and_then(|id| state.channel(id))
			.and_then(|channel| match channel.kind {
				0 | 5 | 15 | 16 => Some(channel.id),
				10..=12 => channel.parent_id,
				_ => None,
			});
		Self {
			pins: state
				.search
				.as_ref()
				.filter(|view| {
					view.pins
						&& Some(view.channel) == selected
						&& state.freshness == model::Freshness::Fresh
						&& state.can_read_history(view.channel)
				})
				.map(|view| {
					(
						view.channel,
						view.request,
						view.loading,
						view.error.is_some(),
						view.page.is_some(),
					)
				}),
			forum_posts: state
				.posts
				.parent
				.filter(|id| Some(*id) == parent)
				.map(|id| {
					(
						id,
						state.posts.request,
						state.posts.loading,
						state.posts.more,
						state.posts.error.is_some(),
					)
				}),
			forum_archive: state
				.archives
				.as_ref()
				.filter(|view| Some(view.parent) == parent)
				.map(|view| {
					(
						view.parent,
						view.request,
						view.loading,
						view.error.is_some(),
						view.page.is_some(),
					)
				}),
			typing,
			read: (
				selected,
				selected.and_then(|id| state.unread(id)),
				selected.map_or(0, |id| state.mention_count(id)),
			),
			profile: (
				profile.request,
				profile.loading,
				profile.reload_required,
				profile.error.is_some(),
				profile.data.is_some(),
			),
			channel: selected
				.filter(|_| state.freshness == model::Freshness::Fresh)
				.and_then(|id| state.channel(id))
				.map(|channel| {
					let readable = state.can_read_history(channel.id);
					(
						channel.id,
						channel.last_message.filter(|_| readable),
						channel.message_count.filter(|_| readable),
					)
				}),
			directory: (state.channels.len(), state.guilds.len()),
			messages: selected
				.filter(|id| {
					state.freshness == model::Freshness::Fresh
						&& state.can_read_history(*id)
						&& state
							.channel(*id)
							.is_some_and(|channel| channel.supports_text())
				})
				.map(|id| (id, state.request)),
			channel_metadata: selected
				.filter(|id| {
					state.freshness == model::Freshness::Fresh && state.can_read_history(*id)
				})
				.map(|id| {
					(
						id,
						state.channel_details(id).is_some(),
						state.post_details(id).is_some(),
					)
				}),
			member_profile: state
				.profile
				.as_ref()
				.filter(|profile| guild.is_some() && profile.guild == guild)
				.map(|profile| {
					(
						profile.user,
						profile.request,
						profile.loading,
						profile.error.is_some(),
						profile.data.is_some(),
					)
				}),
			permissions: state.selected.map(|id| {
				(
					id,
					state.can_view(id),
					state.can_read_history(id),
					state.can_send(id),
				)
			}),
			relationships: (
				state.relationship_view(),
				state.friends_known(),
				state.friend_requests_known(),
				state.restricted_users_known(),
			),
		}
	}
	pub fn changed(&self, old: &Self) -> Changes {
		Changes(
			[
				self.profile != old.profile,
				self.channel != old.channel
					|| self.directory != old.directory
					|| self.channel_metadata != old.channel_metadata,
				self.member_profile != old.member_profile,
				false,
				self.read != old.read,
				self.messages != old.messages,
				self.relationships != old.relationships,
				self.forum_posts != old.forum_posts
					|| self.forum_archive != old.forum_archive
					|| self
						.channel_metadata
						.and_then(|(id, _, post)| post.then_some(id))
						!= old
							.channel_metadata
							.and_then(|(id, _, post)| post.then_some(id)),
				false,
				self.permissions != old.permissions,
				false,
				false,
				self.pins != old.pins,
				self.typing != old.typing,
				false,
			],
			None,
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn data_hints_require_current_generation_context_and_grants() {
		let state = test_support::demo_state();
		let envelope = Envelope {
			generation: state.generation,
			event: Event::MemberPresence {
				guild: Id(10),
				channel: state.selected.unwrap(),
				request: state.member_request,
				updates: Vec::new(),
			},
		};
		let changes = Changes::capture(&state, &envelope);
		assert_eq!(
			changes.kinds(&[Capability::Presence]).collect::<Vec<_>>(),
			vec![AppEventKind::Presence]
		);
		assert_eq!(changes.kinds(&[Capability::Members]).count(), 0);
		let mut stale = envelope;
		stale.generation = state.generation.wrapping_add(1);
		assert_eq!(
			Changes::capture(&state, &stale)
				.kinds(&[Capability::Presence])
				.count(),
			0
		);
		stale.generation = state.generation;
		if let Event::MemberPresence { channel, .. } = &mut stale.event {
			*channel = Id(999);
		}
		assert_eq!(
			Changes::capture(&state, &stale)
				.kinds(&[Capability::Presence])
				.count(),
			0
		);
	}
	#[test]
	fn selected_message_metadata_is_invalidated_without_failed_send_or_other_channel_hints() {
		let state = test_support::demo_state();
		let selected = state.selected.unwrap();
		let has_channels = |event| {
			Changes::capture(
				&state,
				&Envelope {
					generation: state.generation,
					event,
				},
			)
			.kinds(&[Capability::ChannelDetails])
			.any(|kind| kind == AppEventKind::Channels)
		};
		assert!(has_channels(Event::Message(test_support::message(
			99999, selected
		))));
		assert!(!has_channels(Event::Message(test_support::message(
			99999,
			Id(999)
		))));
		assert!(!has_channels(Event::SendResult {
			nonce: "synthetic".into(),
			result: Err(client_core::auth::Failure::Network)
		}));
		let mut private = test_support::message(99999, selected);
		private.ephemeral = true;
		assert!(!has_channels(Event::Message(private)));
	}
	#[test]
	fn local_profile_and_directory_mutations_use_scalar_invalidation_keys() {
		let mut state = test_support::demo_state();
		let old = DataKey::capture(&state);
		assert_eq!(
			old.changed(&old)
				.kinds(&[Capability::AccountProfile, Capability::ChannelDetails])
				.count(),
			0
		);
		state.own_profile.loading = !state.own_profile.loading;
		assert_eq!(
			DataKey::capture(&state)
				.changed(&old)
				.kinds(&[Capability::AccountProfile])
				.collect::<Vec<_>>(),
			vec![AppEventKind::Account]
		);
		let channel = state.selected.unwrap();
		state
			.channels
			.iter_mut()
			.find(|item| item.id == channel)
			.unwrap()
			.last_message = Some(Id(99999));
		assert!(
			DataKey::capture(&state)
				.changed(&old)
				.kinds(&[Capability::ChannelDetails])
				.any(|kind| kind == AppEventKind::Channels)
		);
		let old = DataKey::capture(&state);
		state.guilds.clear();
		assert!(
			DataKey::capture(&state)
				.changed(&old)
				.kinds(&[Capability::GuildDirectory])
				.any(|kind| kind == AppEventKind::Channels)
		);
	}
	#[test]
	fn message_details_hints_require_the_new_grant_and_loaded_readable_context() {
		let mut state = test_support::demo_state();
		let selected = state.selected.unwrap();
		let id = state.timeline.iter().next().unwrap().id;
		let patch = |channel| {
			Event::Patch(model::MessagePatch {
				id,
				channel,
				content: model::Patch::Absent,
				edited: model::Patch::Absent,
				sticker_items: model::Patch::Absent,
				flags: model::Patch::Absent,
				components: model::Patch::Absent,
				application_id: model::Patch::Absent,
				extra_content: Default::default(),
				reactions: model::Patch::Value(Vec::new()),
				mentions: model::Patch::Absent,
				embeds: model::Patch::Absent,
				embeds_suppressed: model::Patch::Absent,
				attachments: model::Patch::Absent,
			})
		};
		let capture = |state: &State, event| {
			Changes::capture(
				state,
				&Envelope {
					generation: state.generation,
					event,
				},
			)
		};
		for event in [
			patch(selected),
			Event::Reactions(client_core::reactions::Event::Changed {
				channel: selected,
				message: id,
			}),
		] {
			let changes = capture(&state, event);
			assert_eq!(
				changes
					.kinds(&[Capability::DataEvents, Capability::Timeline])
					.count(),
				0
			);
			assert_eq!(
				changes
					.kinds(&[Capability::MessageDetails])
					.collect::<Vec<_>>(),
				vec![AppEventKind::MessageDetails]
			);
		}
		let activity = capture(&state, patch(selected));
		assert_eq!(
			activity
				.kinds(&[Capability::ConversationActivity])
				.collect::<Vec<_>>(),
			vec![AppEventKind::Reactions]
		);
		for (channel, expected) in [(selected, 1), (Id(999), 0)] {
			let changes = capture(
				&state,
				Event::Pinned {
					request: 1,
					channel,
					message: id,
					pinned: true,
					result: Ok(()),
				},
			);
			assert_eq!(
				changes
					.kinds(&[Capability::ConversationActivity])
					.filter(|kind| *kind == AppEventKind::Pins)
					.count(),
				expected
			);
		}
		let mut poll = patch(selected);
		if let Event::Patch(value) = &mut poll {
			value.extra_content.poll = model::Patch::Value(true);
		}
		let changes = capture(&state, poll);
		assert_eq!(
			changes
				.kinds(&[Capability::MessageContent])
				.collect::<Vec<_>>(),
			vec![AppEventKind::MessageDetails, AppEventKind::Polls]
		);
		for event in [
			patch(Id(999)),
			Event::Reactions(client_core::reactions::Event::Changed {
				channel: Id(999),
				message: id,
			}),
			Event::Reactions(client_core::reactions::Event::Changed {
				channel: selected,
				message: Id(99999),
			}),
		] {
			assert_eq!(
				capture(&state, event)
					.kinds(&[Capability::MessageDetails])
					.count(),
				0
			);
		}
		state.freshness = model::Freshness::Loading;
		assert_eq!(
			capture(&state, patch(selected))
				.kinds(&[Capability::MessageDetails])
				.count(),
			0
		);
		state.freshness = model::Freshness::Fresh;
		let mut private = test_support::message(99999, selected);
		private.ephemeral = true;
		assert_eq!(
			capture(&state, Event::Message(private))
				.kinds(&[Capability::MessageDetails])
				.count(),
			0
		);
		state.set_preserve_deleted_messages(true);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Delete {
				channel: selected,
				id,
			},
		});
		assert_eq!(
			capture(&state, patch(selected))
				.kinds(&[Capability::MessageDetails])
				.count(),
			0
		);
		let stale = Envelope {
			generation: state.generation.wrapping_add(1),
			event: patch(selected),
		};
		assert_eq!(
			Changes::capture(&state, &stale)
				.kinds(&[Capability::MessageDetails])
				.count(),
			0
		);
	}

	#[test]
	fn relationship_events_and_local_revision_require_the_relationship_grant() {
		let mut state = test_support::demo_state();
		let old = DataKey::capture(&state);
		let envelope = Envelope {
			generation: state.generation,
			event: Event::UserAction(client_core::user_actions::Event::Friends(Some(Vec::new()))),
		};
		let changes = Changes::capture(&state, &envelope);
		assert_eq!(
			changes
				.kinds(&[Capability::DataEvents, Capability::AccountProfile])
				.count(),
			0
		);
		assert_eq!(
			changes
				.kinds(&[Capability::Relationships])
				.collect::<Vec<_>>(),
			vec![AppEventKind::Relationships]
		);
		state.apply(envelope);
		assert_eq!(
			DataKey::capture(&state)
				.changed(&old)
				.kinds(&[Capability::Relationships])
				.collect::<Vec<_>>(),
			vec![AppEventKind::Relationships]
		);
		let changes = Changes::capture(
			&state,
			&Envelope {
				generation: state.generation,
				event: Event::UserAction(client_core::user_actions::Event::NoteChanged {
					user: Id(99),
					text: "Private note is outside relationship snapshots".into(),
				}),
			},
		);
		assert_eq!(changes.kinds(&[Capability::Relationships]).count(), 0);
	}
	#[test]
	fn thread_role_and_permission_hints_are_scoped_and_require_new_grants() {
		let state = test_support::demo_state();
		let channel = state.selected.unwrap();
		let guild = state.channel(channel).unwrap().guild.unwrap();
		let capture = |event| {
			Changes::capture(
				&state,
				&Envelope {
					generation: state.generation,
					event,
				},
			)
		};
		let grants = [Capability::ChannelMetadata, Capability::MemberDetails];
		let threads = capture(Event::ThreadRemoved { guild, id: Id(900) });
		assert!(
			threads
				.kinds(&grants)
				.any(|kind| kind == AppEventKind::Threads)
		);
		assert!(
			!threads
				.kinds(&[Capability::DataEvents, Capability::ChannelDirectory])
				.any(|kind| kind == AppEventKind::Threads)
		);
		assert!(
			!capture(Event::ThreadRemoved {
				guild: Id(999),
				id: Id(900)
			})
			.kinds(&grants)
			.any(|kind| kind == AppEventKind::Threads)
		);
		let roles = capture(Event::Permissions(
			client_core::permissions::Event::RoleRemoved { guild, id: Id(901) },
		));
		assert!(roles.kinds(&grants).any(|kind| kind == AppEventKind::Roles));
		assert!(
			roles
				.kinds(&grants)
				.any(|kind| kind == AppEventKind::Permissions)
		);
		assert!(
			!roles
				.kinds(&[Capability::DataEvents, Capability::Members])
				.any(|kind| matches!(kind, AppEventKind::Roles | AppEventKind::Permissions))
		);
		assert!(
			!capture(Event::Permissions(
				client_core::permissions::Event::RoleRemoved {
					guild: Id(999),
					id: Id(901)
				}
			))
			.kinds(&grants)
			.any(|kind| kind == AppEventKind::Roles)
		);
		let overwrites = |target| {
			Event::Permissions(client_core::permissions::Event::Channel {
				channel: target,
				guild: Some(guild),
				overwrites: model::Patch::Value(Vec::new()),
			})
		};
		assert!(
			capture(overwrites(channel))
				.kinds(&grants)
				.any(|kind| kind == AppEventKind::Permissions)
		);
		assert!(
			!capture(overwrites(Id(999)))
				.kinds(&grants)
				.any(|kind| kind == AppEventKind::Permissions)
		);
	}
}
