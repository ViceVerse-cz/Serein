//! Event reduction; common validation and reconciliation preserve their order.
use super::{
	BTreeSet, Channel, Command, Delivery, Envelope, Event, Freshness, Guild, Id,
	MAX_GUILD_STICKERS, MAX_MEMBER_PRESENCE_BYTES, MAX_NAV, MemberSlot, Patch, State, Timeline,
	auth, custom_emoji_bytes, navigable, notifications, permissions, preload, prepare_navigation,
	sticker_bytes, valid_avatar_hash, valid_custom_emojis, valid_stickers,
};

impl State {
	pub fn apply(&mut self, envelope: Envelope) {
		self.apply_with_permissions(envelope, None);
	}
	pub(super) fn apply_with_permissions(
		&mut self,
		envelope: Envelope,
		prepared_permissions: Option<permissions::Permissions>,
	) {
		self.reply_deletions.0.clear();
		if envelope.generation != self.generation {
			return;
		}
		if self.history_navigation.provisional() {
			let affects_provisional = match &envelope.event {
				Event::History { .. }
				| Event::HistoryFailed { .. }
				| Event::Typing(_)
				| Event::MemberPresence { .. }
				| Event::DirectPresence(_) => false,
				Event::Message(message) => self.selected == Some(message.channel),
				Event::Patch(patch) => self.selected == Some(patch.channel),
				Event::Delete { channel, .. } | Event::DeleteBulk { channel, .. } => {
					self.selected == Some(*channel)
				}
				_ => true,
			};
			if affects_provisional {
				self.rollback_provisional_history();
			}
		}
		if !matches!(envelope.event, Event::Message(_) | Event::Typing(_)) {
			self.invalidate_expression_event(&envelope.event);
			self.observe_server_display(&envelope.event);
			if matches!(envelope.event, Event::GuildDisplayMetadata(_)) {
				return;
			}
		}
		self.invalidate_reply_previews(&envelope.event);
		if envelope.event.changes_access()
			|| matches!(&envelope.event, Event::Disconnected | Event::Resumed)
		{
			self.community.invalidate();
		}
		if self.handle_private_message_event(&envelope.event) {
			return;
		}
		if let Event::Startup(startup) = envelope.event {
			self.apply_startup(startup, envelope.generation);
			return;
		}
		if matches!(envelope.event, Event::Ready { .. } | Event::Resync) {
			self.startup_warnings = Default::default();
			self.failure_detail = None;
		}
		if matches!(envelope.event, Event::Resync) {
			self.read_state.reset();
			self.notification_preferences.suspend();
		}
		if matches!(
			envelope.event,
			Event::Ready { .. } | Event::Disconnected | Event::Resync
		) {
			if matches!(envelope.event, Event::Ready { .. } | Event::Resync) {
				self.set_premium_type(0);
			}
			self.interrupt_channel_preferences();
			self.interrupt_preloads();
			self.interrupt_reaction_preferences();
			self.interrupt_stickers();
			self.interrupt_soundboard();
			self.posts.clear_summaries();
			self.interactions.reset();
			self.application_commands.clear();
			self.local_game_activity = Default::default();
			self.invalidate_messaging_permissions(None);
			self.interrupt_own_profile();
			self.interrupt_server_profile();
			self.interrupt_account_settings();
			self.interrupt_owned_cosmetics();
		}
		if let Event::Typing(signal) = envelope.event {
			self.observe_typing_at(
				signal,
				std::time::SystemTime::now(),
				std::time::Instant::now(),
			);
			return;
		}
		if let Event::MemberSearchCooldown { request, seconds } = envelope.event {
			self.member_search_cooldown(request, seconds);
			return;
		}
		if let Event::MemberIdentity(update) = envelope.event {
			self.apply_member_identity(*update);
			return;
		}
		if let Event::MemberPresence {
			guild,
			channel,
			request,
			updates,
		} = &envelope.event
		{
			if envelope.event.bytes() <= MAX_MEMBER_PRESENCE_BYTES {
				self.apply_member_presence(*guild, *channel, *request, updates);
			}
			return;
		}
		if let Event::DirectPresence(updates) = &envelope.event {
			if envelope.event.bytes() <= MAX_MEMBER_PRESENCE_BYTES {
				self.apply_direct_presence(updates);
			} else {
				self.direct_presence_lossy = true;
			}
			return;
		}
		// Preserve the running total through channel-create/permission bursts at guild admission.
		if !matches!(
			&envelope.event,
			Event::ChannelCreated(_) | Event::ChannelRestored(_) | Event::Permissions(_)
		) {
			self.navigation_index.bytes.set(None);
		}
		// Ordinary arrivals cannot invalidate the drawer's snapshot. Keep their hot path
		// away from the cold administrative state.
		if matches!(
			&envelope.event,
			Event::Patch(_)
				| Event::Delete { .. }
				| Event::DeleteBulk { .. }
				| Event::Unavailable(_)
				| Event::Disconnected
				| Event::Resync
				| Event::Failure(_)
		) {
			self.invalidate_member_review(&envelope.event);
		}
		let access_changed = envelope.event.changes_access();
		if access_changed && self.search_access_changed(&envelope.event) {
			self.clear_search();
		}
		// Command permissions are evaluated live from `permissions`; member, role and channel
		// updates keep the index. Only a lost bot conversation invalidates its own index.
		if let Event::Unavailable(channel) = &envelope.event
			&& self.application_commands.scope == Some(*channel)
		{
			self.application_commands.clear();
		}
		// Ephemeral names must not outlive navigation identity/permission replacement.
		if access_changed {
			self.typing.clear();
		}
		let previous_access = access_changed.then(|| self.permission_access()).flatten();
		let previous_member_list = (access_changed && self.members.is_some()).then(|| {
			self.selected
				.and_then(|id| self.channel(id))
				.and_then(|channel| self.member_list_id(channel))
		});
		if let Event::ChannelRestored(channel) = &envelope.event
			&& (channel.id.0 == 0
				|| !matches!(channel.kind, 0 | 2 | 4 | 5 | 13..=16)
				|| !channel.guild.is_some_and(|guild| {
					guild.0 != 0 && self.guilds.iter().any(|known| known.id == guild)
				}) || self.channel(channel.id).is_some())
		{
			return;
		}
		let archive_mutation = match &envelope.event {
			Event::ThreadRemoved { guild, id } => Some((*guild, *id)),
			Event::ThreadChanged { guild, patch } => Some((*guild, patch.id)),
			_ => None,
		};
		if let Some((_, id)) = archive_mutation {
			self.forget_channel_reference_name(id);
		}
		if archive_mutation.is_some_and(|(guild, id)| {
			self.archives.as_ref().is_some_and(|view| {
				view.guild == guild
					&& (view.loading
						|| view
							.page
							.as_ref()
							.is_some_and(|page| page.threads.iter().any(|thread| thread.id == id)))
			})
		}) {
			self.clear_archives();
		}
		if let Event::ThreadChanged { guild, patch } = &envelope.event
			&& !self
				.channel(patch.id)
				.is_some_and(|c| c.guild == Some(*guild) && matches!(c.kind, 10..=12))
		{
			return;
		}
		if let Event::ThreadRemoved { guild, id } = &envelope.event
			&& !self
				.channel(*id)
				.is_some_and(|c| c.guild == Some(*guild) && matches!(c.kind, 10..=12))
		{
			return;
		}
		self.timeline
			.set_preserve_deleted_messages(self.preserve_deleted_messages);
		self.invalidate_resident_event(&envelope.event);
		self.observe_thread_starter_event(&envelope.event);
		self.observe_channel_action(&envelope.event);
		self.observe_community_event(&envelope.event);
		self.observe_member_guide_event(&envelope.event);
		if let Event::ChannelCreated(channel) = &envelope.event {
			self.observe_dm_reopened(channel.id);
			self.observe_group_change(channel.id, true);
		}
		if let Event::ChannelChanged(patch) = &envelope.event {
			self.observe_group_change(patch.id, false);
		}
		if let Event::RecipientAdded { channel, .. } | Event::RecipientRemoved { channel, .. } =
			&envelope.event
		{
			self.observe_group_change(*channel, true);
		}
		self.filter_view_revisions(&envelope.event);
		if matches!(
			&envelope.event,
			Event::Disconnected
				| Event::Resync
				| Event::PermissionsChanged
				| Event::Ready { .. }
				| Event::Permissions(_)
				| Event::Unavailable(_)
		) || self
			.inbox
			.messages
			.iter()
			.map(|message| (message.channel, message.id))
			.chain(
				self.inbox
					.saved
					.iter()
					.map(|saved| (saved.channel, saved.message)),
			)
			.any(|(message_channel, message_id)| match &envelope.event {
				Event::Patch(patch) => patch.channel == message_channel && patch.id == message_id,
				Event::Delete { channel, id } => *channel == message_channel && *id == message_id,
				Event::DeleteBulk { channel, ids } => {
					*channel == message_channel && ids.contains(&message_id)
				}
				_ => false,
			}) {
			self.clear_mentions();
		}
		self.revision += 1;
		if matches!(
			&envelope.event,
			Event::Disconnected | Event::Resync | Event::PermissionsChanged | Event::Ready { .. }
		) || matches!(&envelope.event,Event::Unavailable(id) if Some(*id)==self.selected)
			|| matches!(&envelope.event,Event::RecipientRemoved{channel,user} if Some(*channel)==self.selected && self.user.as_ref().is_some_and(|u|u.id==*user))
		{
			self.clear_search();
			self.search_target = None;
		}
		// Keep unaffected hits while preventing stale in-flight snippets from reappearing.
		self.search_message_changed(&envelope.event);
		let previous_channel = self.selected;
		let previous_tail = matches!(
			&envelope.event,
			Event::History { .. } | Event::Message(_) | Event::Patch(_) | Event::SendResult { .. }
		)
		.then(|| self.timeline.iter().next_back().map(|message| message.id))
		.flatten();
		let incoming_tail = match &envelope.event {
			Event::Message(message)
				if !self.history_targeted && Some(message.channel) == self.selected =>
			{
				Some(message.id)
			}
			Event::SendResult {
				nonce,
				result: Ok(message),
			} if !self.history_targeted
				&& Some(message.channel) == self.selected
				&& self.pending.iter().any(|pending| {
					pending.nonce == *nonce && pending.channel == message.channel
				}) =>
			{
				Some(message.id)
			}
			_ => None,
		};
		let result = match envelope.event {
			Event::Startup(_) => {
				unreachable!("startup is applied atomically before ordinary events")
			}
			Event::StartupWarnings(warnings) => {
				self.startup_warnings.read_state |= warnings.read_state;
				self.startup_warnings.notifications |= warnings.notifications;
				self.startup_warnings.sessions |= warnings.sessions;
				self.startup_warnings.presence |= warnings.presence;
				self.startup_warnings.emojis |= warnings.emojis;
				self.startup_warnings.stickers |= warnings.stickers;
				self.startup_warnings.entries |= warnings.entries;
				if warnings.read_state {
					self.read_state.reset();
				}
				if warnings.notifications || warnings.sessions {
					self.invalidate_startup_preferences(warnings.notifications, warnings.sessions);
				}
				if warnings.presence {
					self.clear_direct_presences();
					// Without READY's presence, a missing record no longer means offline.
					self.direct_presence_lossy = true;
				}
				Ok(())
			}
			Event::GuildFolders(result) => {
				self.apply_guild_folders(result);
				Ok(())
			}
			Event::AccountSettings { folders, .. } => {
				self.folders_stale |= folders && self.guild_folders.is_some();
				Ok(())
			}
			Event::ReactionPreferencesChanged => {
				self.reaction_preferences.dirty = true;
				self.reaction_preferences.failures = 0;
				Ok(())
			}
			Event::ReactionPreferences { request, result } => {
				self.apply_reaction_preferences(request, result);
				Ok(())
			}
			Event::Typing(_) => unreachable!("typing is handled before timeline invalidation"),
			Event::MemberIdentity(_) | Event::MemberSearchCooldown { .. } => {
				unreachable!("member identity handled before timeline invalidation")
			}
			Event::Permissions(mut event) => {
				let known = |id: Id| self.guild(id).is_some();
				match &mut event {
					permissions::Event::Snapshot(snapshot)
						if snapshot.guilds.iter().any(|guild| !known(guild.id)) =>
					{
						return;
					}
					permissions::Event::Guild(guild) if !known(guild.id) => return,
					permissions::Event::Members(members) => {
						members.retain(|(guild, _, _)| known(*guild))
					}
					permissions::Event::Role { guild, .. }
					| permissions::Event::RoleRemoved { guild, .. }
					| permissions::Event::Member { guild, .. }
					| permissions::Event::Owner { guild, .. }
					| permissions::Event::UnavailableGuild(guild)
						if !known(*guild) =>
					{
						return;
					}
					permissions::Event::Channel {
						guild: Some(guild), ..
					} if !known(*guild) => return,
					_ => {}
				}
				if let permissions::Event::Channel { channel, guild, .. } = &mut event {
					let actual = self.channel(*channel).and_then(|c| c.guild);
					if guild.is_some() && actual.is_some() && *guild != actual {
						return;
					}
					if guild.is_none() {
						*guild = actual;
					}
				}
				self.server_admin.permissions_changed(&event);
				self.update_permissions(event)
			}
			Event::Archives {
				parent,
				request,
				result,
			} => {
				self.apply_archives(parent, request, result);
				Ok(())
			}
			Event::ThreadStarter {
				thread,
				request,
				result,
			} => {
				self.apply_thread_starter(thread, request, result);
				Ok(())
			}
			Event::ForumSummaries { request, results } => {
				self.apply_forum_summaries(request, results);
				Ok(())
			}
			Event::ForumPosts {
				parent,
				request,
				result,
			} => {
				self.apply_forum_posts(parent, request, result);
				Ok(())
			}
			Event::PostCreated {
				parent,
				request,
				result,
			} => {
				self.apply_post(parent, request, result);
				Ok(())
			}
			Event::Search {
				channel,
				request,
				result,
			} => {
				self.apply_search(channel, request, result);
				Ok(())
			}
			Event::StickerEntitlement { user, premium_type } => {
				if self.user.as_ref().is_some_and(|own| own.id == user) {
					match premium_type {
						Patch::Absent => {}
						Patch::Null => self.set_premium_type(0),
						Patch::Value(kind) => self.set_premium_type(kind),
					}
				}
				Ok(())
			}
			Event::StickerPacks(result) => {
				self.apply_sticker_packs(result);
				Ok(())
			}
			Event::Sticker { id, result } => {
				self.apply_sticker(id, result);
				Ok(())
			}
			Event::GuildStickers { guild, stickers } => {
				if let Some(index) = self.guilds.iter().position(|g| g.id == guild) {
					let previous = self.guilds[index]
						.stickers
						.as_ref()
						.map_or(0, sticker_bytes);
					if !valid_stickers(&stickers, MAX_GUILD_STICKERS)
						|| stickers.iter().any(|s| s.guild_id != Some(guild))
						|| self.navigation_bytes().saturating_sub(previous)
							+ sticker_bytes(&stickers)
							+ self.permissions.bytes()
							> model::account::MAX_BYTES
					{
						self.guilds[index].stickers = None;
						self.navigation_index.bytes.set(None);
						self.fail(auth::Failure::Capacity);
						return;
					}
					self.guilds[index].stickers = Some(stickers);
					self.navigation_index.bytes.set(None);
				}
				Ok(())
			}
			Event::Gifs { request, result } => {
				self.apply_gifs(request, result);
				Ok(())
			}
			Event::GifFavorites { request, result } => {
				self.apply_gif_favorites(request, result);
				Ok(())
			}
			Event::GifFavoritesChanged => {
				self.invalidate_gif_favorites();
				Ok(())
			}
			Event::ReadState(event) => self.apply_read_state(event),
			Event::ChannelPreferences(event) => self.finish_channel_preferences(event),
			Event::NotificationPreferences(event) => self.apply_notification_preferences(event),
			Event::MessagingPermissions { request, result } => {
				self.apply_messaging_permissions(request, result);
				Ok(())
			}
			Event::Mfa(event) => {
				self.apply_mfa(event);
				self.enforce_admin_budget();
				Ok(())
			}
			Event::UserAction(event) => self.apply_user_action(event),
			Event::ServerAction(event) => self.apply_server_action(event),
			Event::ChannelAction(event) => self.apply_channel_action(event),
			Event::ServerSettings(event) => {
				let result = self.apply_server_settings(event);
				self.enforce_admin_budget();
				result
			}
			Event::MemberGuide(event) => self.apply_member_guide(event),
			Event::Onboarding(event) => self.apply_onboarding(event),
			Event::Community(event) => self.apply_community(event),
			Event::CommunityFence(revision) => {
				// Concurrent transports can enqueue an older notice after a newer revision.
				let advance = revision.wrapping_sub(self.community.fence);
				if advance != 0 && advance < (1u64 << 63) {
					self.community.invalidate();
					self.community.fence = revision;
				}
				Ok(())
			}
			Event::CommunityChanged {
				guild,
				channel,
				stage_ended,
			} => {
				self.community.invalidate_metadata(guild, channel);
				if stage_ended && let Some(channel) = channel {
					self.stop_stage_participation(channel);
					if self.community.stage_channel == Some(channel) {
						self.community.stage = None;
					}
				}
				Ok(())
			}
			Event::ServerAdmin(event) => {
				let result = self.apply_server_admin(event);
				self.enforce_admin_budget();
				result
			}
			Event::MemberReviewProgress(progress) => {
				self.apply_member_review_progress(progress);
				self.enforce_admin_budget();
				Ok(())
			}
			Event::BulkModeration { guild, report } => {
				self.apply_bulk_moderation(guild, report);
				self.enforce_admin_budget();
				Ok(())
			}
			Event::GroupAction(event) => self.apply_group_action(event),
			Event::ThreadsSync {
				guild,
				parents,
				threads,
				removed,
			} => self.apply_threads_sync(guild, parents, threads, removed),
			Event::Interaction(event) => self.apply_interaction(event),
			Event::Autocomplete(event) => {
				self.apply_autocomplete(event);
				Ok(())
			}
			Event::ApplicationCommands {
				channel,
				request,
				result,
			} => {
				self.apply_application_commands(channel, request, result);
				Ok(())
			}
			Event::Polls(event) => self.apply_poll(event),
			Event::Soundboard(event) => {
				self.apply_soundboard(event);
				Ok(())
			}
			Event::Reactions(event) => self.apply_reactions(event),
			Event::InviteChallenge { request, challenge } => {
				self.apply_invite_challenge(request, *challenge);
				Ok(())
			}
			Event::JoinInvite { request, result } => {
				self.apply_invite_join(request, result);
				Ok(())
			}
			Event::GuildCreated { sequence, result } => {
				self.apply_guild_created(sequence, result);
				Ok(())
			}
			Event::GuildJoined(guild) => {
				self.observe_server_joined(guild.id);
				if !self.guilds.iter().any(|g| g.id == guild.id) {
					let capacity = self.guilds.capacity();
					let mut bytes = self.navigation_bytes() + guild.bytes() - size_of::<Guild>();
					if guild.id.0 == 0
						|| guild.name.len() > 512
						|| self.guilds.len() + self.channels.len() >= MAX_NAV
						|| bytes + self.permissions.bytes() > model::account::MAX_BYTES
					{
						self.fail(auth::Failure::Capacity);
						return;
					}
					self.guilds.insert(0, guild);
					bytes += (self.guilds.capacity() - capacity) * size_of::<Guild>();
					if bytes + self.permissions.bytes() > model::account::MAX_BYTES {
						self.guilds.remove(0);
						self.guilds.shrink_to_fit();
						self.invalidate_navigation();
						self.fail(auth::Failure::Capacity);
						return;
					}
					self.set_navigation_bytes(bytes);
				}
				Ok(())
			}
			Event::GuildApplicationChanged {
				guild,
				request,
				deleted,
			} => {
				self.guild_application_changed(guild, request, deleted);
				Ok(())
			}
			Event::ServerIncident { guild, incident } => {
				self.apply_server_incident(guild, incident);
				Ok(())
			}
			Event::GuildRemoved(guild) => {
				self.server_incidents.remove(&guild);
				self.remove_server(guild);
				Ok(())
			}
			Event::Invite { code, result } => {
				self.apply_invite(code, result.map(|embed| *embed));
				Ok(())
			}
			Event::ProfileEdited {
				user,
				request,
				result,
			} => {
				self.apply_own_profile(user, request, result);
				Ok(())
			}
			Event::ServerProfile(event) => {
				self.apply_server_profile(event);
				Ok(())
			}
			Event::OwnedCosmetics(event) => {
				self.apply_owned_cosmetics(event);
				Ok(())
			}
			Event::AccountOverview {
				user,
				request,
				result,
			} => {
				self.apply_account_overview(user, request, result);
				Ok(())
			}
			Event::AccountConnections {
				user,
				request,
				result,
			} => {
				self.apply_account_connections(user, request, result);
				Ok(())
			}
			Event::Profile {
				user,
				guild,
				request,
				result,
			} => {
				self.apply_profile(user, guild, request, result);
				Ok(())
			}
			Event::StreamPreview {
				guild,
				channel,
				user,
				request,
				result,
			} => {
				self.apply_stream_preview(guild, channel, user, request, result);
				Ok(())
			}
			Event::BadgeSettings { request, result } => {
				self.apply_badge_settings(request, result);
				Ok(())
			}

			Event::GuildEmojis { guild, emojis } => {
				if let Some(index) = self.guilds.iter().position(|g| g.id == guild) {
					let previous = self.guilds[index]
						.emojis
						.as_ref()
						.map_or(0, custom_emoji_bytes);
					let navigation_bytes = self.navigation_bytes();
					if !valid_custom_emojis(&emojis)
						|| navigation_bytes - previous
							+ custom_emoji_bytes(&emojis)
							+ self.permissions.bytes()
							> model::account::MAX_BYTES
					{
						self.guilds[index].emojis = None;
						self.navigation_index.bytes.set(None);
						self.fail(auth::Failure::Capacity);
						return;
					}
					self.guilds[index].emojis = Some(emojis);
					self.navigation_index.bytes.set(None);
					if self.guilds.iter().all(|guild| guild.emojis.is_some()) {
						self.startup_warnings.emojis = false;
					}
				}
				Ok(())
			}
			Event::GuildDisplayMetadata(_) => unreachable!("handled before reduction"),
			Event::GuildChanged(patch) => {
				if !matches!(patch.default_message_notifications, Patch::Absent)
					&& self.server_settings.guild == Some(patch.id)
					&& self.server_settings.pending
				{
					self.server_settings.notification_default_changed = true;
				}
				if let Some(index) = self.guilds.iter().position(|guild| guild.id == patch.id) {
					let previous = self.guilds[index].clone();
					let guild = &mut self.guilds[index];
					match patch.default_message_notifications {
						Patch::Value(level) => {
							guild.default_message_notifications = (level <= 1).then_some(level)
						}
						Patch::Null => guild.default_message_notifications = None,
						Patch::Absent => {}
					}
					match patch.name {
						Patch::Value(name) => guild.name = name.chars().take(128).collect(),
						Patch::Null => guild.name.clear(),
						Patch::Absent => {}
					}
					match patch.icon {
						Patch::Value(icon) => guild.icon = valid_avatar_hash(&icon).then_some(icon),
						Patch::Null => guild.icon = None,
						Patch::Absent => {}
					}
					if self.navigation_bytes() + self.permissions.bytes()
						> model::account::MAX_BYTES
					{
						self.guilds[index] = previous;
						self.navigation_index.bytes.set(None);
						self.fail(auth::Failure::Capacity);
						return;
					}
				}
				Ok(())
			}
			Event::ChannelCreated(channel) | Event::ChannelRestored(channel) => {
				if self.archived_thread == Some(channel.id)
					&& self.channel(channel.id).is_some_and(|old| {
						old.id == channel.id
							&& (old.guild != channel.guild
								|| old.parent_id != channel.parent_id
								|| old.kind != channel.kind)
					}) {
					return;
				}
				if matches!(channel.kind, 10..=12)
					&& (!channel
						.guild
						.is_some_and(|guild| self.guild(guild).is_some())
						|| self.channel(channel.id).is_some_and(|old| {
							old.id == channel.id
								&& (old.guild != channel.guild || !matches!(old.kind, 10..=12))
						})) {
					return;
				}
				let old = self.channel_index(channel.id);
				let capacity = self.channels.capacity();
				let mut navigation_bytes =
					self.navigation_bytes() - old.map_or(0, |index| self.channels[index].bytes())
						+ channel.bytes() - if old.is_none() {
						size_of::<Channel>()
					} else {
						0
					};
				if channel.recipients.len() > 64
					|| (old.is_none() && self.channels.len() + self.guilds.len() >= MAX_NAV)
					|| navigation_bytes + self.permissions.bytes() > model::account::MAX_BYTES
				{
					self.fail(auth::Failure::Capacity);
					return;
				}
				if self
					.archives
					.as_ref()
					.is_some_and(|view| view.parent == channel.id)
				{
					self.clear_archives();
				}
				if let Some(index) = old {
					if self.archived_thread == Some(channel.id)
						&& self.channels[index].guild == channel.guild
						&& self.channels[index].parent_id == channel.parent_id
						&& self.channels[index].kind == channel.kind
					{
						self.archived_thread = None;
					}
					if let Some(latest) = self.channels[index].last_message {
						self.read_state.activity.observe_latest(channel.id, latest);
					}
					self.channels[index] = channel;
				} else {
					let id = channel.id;
					self.navigation_index
						.channels
						.get_mut()
						.insert(id, self.channels.len());
					self.channels.push(channel);
					navigation_bytes +=
						(self.channels.capacity() - capacity) * size_of::<Channel>();
					if navigation_bytes + self.permissions.bytes() > model::account::MAX_BYTES {
						self.channels.pop();
						self.channels.shrink_to_fit();
						self.invalidate_navigation();
						self.fail(auth::Failure::Capacity);
						return;
					}
					self.navigation_index
						.channel_stamp
						.set(Some(self.channel_stamp()));
				}
				self.set_navigation_bytes(navigation_bytes);
				Ok(())
			}
			Event::ChannelChanged(patch) | Event::ThreadChanged { patch, .. } => {
				if self
					.archives
					.as_ref()
					.is_some_and(|view| view.parent == patch.id)
				{
					self.clear_archives();
				}
				if let Some(index) = self.channel_index(patch.id) {
					let previous = self.channels[index].clone();
					let channel = &mut self.channels[index];
					if let Some(latest) = channel.last_message {
						self.read_state.activity.observe_latest(channel.id, latest);
					}
					match patch.last_message {
						Patch::Value(id) => channel.last_message = Some(id),
						Patch::Null => channel.last_message = None,
						Patch::Absent => {}
					}
					match patch.name {
						Patch::Value(name) => channel.name = name.chars().take(128).collect(),
						Patch::Null => channel.name.clear(),
						Patch::Absent => {}
					}
					match patch.icon {
						Patch::Value(hash) => {
							channel.icon = model::valid_avatar_hash(&hash).then_some(hash)
						}
						Patch::Null => channel.icon = None,
						Patch::Absent => {}
					}
					match patch.parent_id {
						Patch::Value(id) => channel.parent_id = Some(id),
						Patch::Null => channel.parent_id = None,
						Patch::Absent => {}
					}
					if let Patch::Value(position) = patch.position {
						channel.position = position;
					}
					if let Patch::Value(kind) = patch.kind {
						channel.kind = kind;
					}
					if let Patch::Value(count) = patch.message_count {
						channel.message_count = Some(count);
					}
					match patch.tags {
						Patch::Value(tags) => channel.tags = Some(tags),
						Patch::Null => channel.tags = None,
						Patch::Absent => {}
					}
					if self.navigation_bytes() + self.permissions.bytes()
						> model::account::MAX_BYTES
					{
						self.channels[index] = previous;
						self.navigation_index.bytes.set(None);
						self.fail(auth::Failure::Capacity);
						return;
					}
					if self.selected == Some(self.channels[index].id)
						&& !navigable(&self.channels[index])
					{
						self.arrived_home();
						self.invalidate_members();
						self.timeline.clear();
						self.cancel_history();
						self.freshness = Freshness::Unavailable;
					}
				}
				Ok(())
			}
			Event::Voice(event) => {
				self.apply_voice(event);
				Ok(())
			}
			Event::RecipientAdded { channel, user } => {
				if let Some(index) = self
					.channel_index(channel)
					.filter(|&index| self.channels[index].guild.is_none())
				{
					let previous = self.channels[index].clone();
					let c = &mut self.channels[index];
					if let Some(old) = c.recipients.iter_mut().find(|u| u.id == user.id) {
						*old = user;
					} else if c.recipients.len() < 64 {
						c.recipients.push(user);
					} else {
						self.fail(auth::Failure::Capacity);
						return;
					}
					if self.navigation_bytes() + self.permissions.bytes()
						> model::account::MAX_BYTES
					{
						self.channels[index] = previous;
						self.navigation_index.bytes.set(None);
						self.fail(auth::Failure::Capacity);
						return;
					}
					if self.selected == Some(channel) && self.members.is_some() {
						let _ = self.request_members();
					}
				}
				Ok(())
			}
			Event::RecipientRemoved { channel, user } => {
				if self.user.as_ref().is_some_and(|u| u.id == user) {
					self.end_voice_channel(channel);
					self.read_state.forget(channel);
					self.forget_direct_inbox(channel);
					self.channels.retain(|c| c.id != channel);
					self.prune_direct_presence();
					if self.selected == Some(channel) {
						self.invalidate_members();
						self.timeline.clear();
						self.freshness = Freshness::Unavailable;
						self.cancel_history();
						self.status = "Conversation unavailable; account removed";
					}
					return;
				}
				if let Some(index) = self
					.channel_index(channel)
					.filter(|&index| self.channels[index].guild.is_none())
				{
					let c = &mut self.channels[index];
					c.recipients.retain(|u| u.id != user);
					for (id, participants) in &mut self.voice.dm_participants {
						if *id == channel {
							participants.retain(|p| p.user != user);
						}
					}
					for (id, ringing) in &mut self.voice.dm_ringing {
						if *id == channel {
							ringing.retain(|id| *id != user);
						}
					}
					if let Some(call) = &mut self.voice.active
						&& call.channel == channel
					{
						call.participants.retain(|p| p.user != user);
						if call.watching == Some(user) {
							call.watching = None;
						}
					}
					if self.selected == Some(channel) && self.members.is_some() {
						let _ = self.request_members();
					}
				}
				Ok(())
			}
			Event::MemberPresence { .. } | Event::DirectPresence(_) => {
				unreachable!("presence handled before timeline revision")
			}
			Event::Report(event) => {
				self.apply_report(event);
				Ok(())
			}
			Event::InboxInvalidated => {
				// Gateway echoes can precede or follow REST confirmation. Retain both
				// its correlation and the visible page; Refresh reconciles other clients.
				self.inbox.stale = true;
				Ok(())
			}
			Event::InboxAction { request, result } => {
				self.apply_inbox_action(request, result);
				Ok(())
			}
			Event::Mentions { request, result } => {
				self.apply_mentions(request, result);
				Ok(())
			}
			Event::MemberSearch { request, result } => {
				self.searched_members(request, result);
				Ok(())
			}
			Event::MemberSearchPartial {
				request,
				rows,
				missing,
			} => {
				self.searched_members_with_missing(request, Ok(rows), missing);
				Ok(())
			}
			Event::Members(mut list) => {
				if !self.gateway_connected
					|| self.freshness == Freshness::Unavailable
					|| !self.can_view(list.channel)
					|| self.selected != Some(list.channel)
					|| self
						.members
						.as_ref()
						.is_none_or(|m| m.request != list.request || m.guild != list.guild)
				{
					return;
				}
				let ranges_left = self.members.as_ref().is_some_and(|current| {
					!current.ranges.is_empty()
						&& !list.ranges.is_empty()
						&& current.ranges != list.ranges
				});
				let mut field_users = BTreeSet::new();
				let invalid = list.slots.len() > 200
					|| list.missing_identity.len() > list.slots.len()
					|| list.missing_identity.iter().any(|fields| {
						fields.user.0 == 0
							|| !fields.needs_metadata()
							|| !field_users.insert(fields.user)
							|| !list.slots.iter().flatten().any(
								|slot| matches!(slot, MemberSlot::Person(member) if member.user.id == fields.user),
							)
					}) || list.slots.iter().flatten().any(|slot| match slot {
					MemberSlot::Person(row) => {
						!row.valid() || row.roles.len() > model::permissions::MAX_MEMBER_ROLES
					}
					MemberSlot::Group(id) => id.is_empty() || id.len() > 32,
				}) || list.slot_bytes() > 256 * 1024
					|| list.groups.len() > model::permissions::MAX_ROLES + 2;
				if invalid {
					if !ranges_left {
						self.members.as_mut().unwrap().freshness = Freshness::Unavailable;
					}
				} else {
					// A fresh subscription can contain partial members. Merge only absent
					// fields from previously observed identities; explicit null and empty
					// lists remain authoritative.
					let mut history = std::collections::BTreeMap::<
						Id,
						(Option<&str>, Option<&[Id]>, Option<&str>),
					>::new();
					if !field_users.is_empty() {
						for message in self.timeline.iter().rev() {
							if message.channel != list.channel
								|| message.author.webhook
								|| !field_users.contains(&message.author.id)
							{
								continue;
							}
							let value = history.entry(message.author.id).or_default();
							if value.2.is_none() {
								value.2 = Some(&message.author.name);
							}
							if value.0.is_none() {
								value.0 = message
									.author_nick
									.as_deref()
									.filter(|nick| !nick.is_empty());
							}
							if value.1.is_none() && !message.author_roles.is_empty() {
								value.1 = Some(&message.author_roles);
							}
						}
					}
					for slot in list.slots.iter_mut().flatten() {
						let MemberSlot::Person(member) = slot else {
							continue;
						};
						let Some(fields) = list
							.missing_identity
							.iter()
							.find(|fields| fields.user == member.user.id)
						else {
							continue;
						};
						if fields.global_name == model::GlobalNameUpdate::Unchanged {
							let cleared = list.guild.is_some_and(|guild| {
								self.known_global_name(guild, member.user.id)
									== Some(model::GlobalNameUpdate::Cleared)
							});
							let known = if cleared {
								member.user.username.as_deref().map(str::to_owned)
							} else {
								self.member_identity(list.channel, member.user.id)
									.and_then(|identity| identity.profile.as_ref())
									.map(|profile| profile.name.clone())
							}
							.or_else(|| {
								list.guild
									.and_then(|guild| {
										self.member_labels.verified_member(guild, member.user.id)
									})
									.map(|member| member.user.name.clone())
							})
							.or_else(|| {
								history
									.get(&member.user.id)
									.and_then(|entry| entry.2)
									.map(str::to_owned)
							});
							if let Some(name) = known {
								member.user.name = name;
							}
						}
						if fields.nick {
							let known = self
								.member_identity(list.channel, member.user.id)
								.and_then(|identity| match &identity.nick {
									Patch::Value(nick) => Some(Some(nick.as_str())),
									Patch::Null => Some(None),
									Patch::Absent => None,
								})
								.or_else(|| {
									list.guild
										.and_then(|guild| {
											self.member_labels
												.verified_member(guild, member.user.id)
										})
										.map(|member| member.nick.as_deref())
								})
								.or_else(|| {
									history
										.get(&member.user.id)
										.and_then(|entry| entry.0)
										.map(Some)
								});
							if let Some(nick) = known {
								member.nick = nick.map(str::to_owned);
							}
						}
						if fields.roles {
							let known = self
								.member_identity(list.channel, member.user.id)
								.and_then(|identity| match &identity.roles {
									Patch::Value(roles) => Some(roles.as_slice()),
									_ => None,
								})
								.or_else(|| {
									list.guild
										.and_then(|guild| {
											self.member_labels
												.verified_member(guild, member.user.id)
										})
										.map(|member| member.roles.as_slice())
								})
								.or_else(|| history.get(&member.user.id).and_then(|entry| entry.1));
							if let Some(roles) = known {
								member.roles = roles.to_vec();
							}
						}
					}
					if let Some(guild) = list.guild {
						for fields in &mut list.missing_identity {
							if fields.global_name == model::GlobalNameUpdate::Unchanged
								&& let Some(previous) = self.known_global_name(guild, fields.user)
							{
								fields.global_name = previous;
							}
						}
						list.missing_identity
							.retain(model::MissingMemberFields::needs_metadata);
					}
					if ranges_left {
						if list.freshness == Freshness::Fresh {
							let protect = self
								.members
								.as_ref()
								.map(|current| current.ranges.clone())
								.unwrap_or_default();
							self.member_chunks.merge(&list);
							self.member_chunks.evict(&protect);
						}
					} else {
						let kept_ranges = self.members.as_ref().map(|m| m.ranges.clone());
						self.refresh_member_labels(&list);
						for slot in list.slots.iter().flatten() {
							if let MemberSlot::Person(member) = slot {
								if let Some(fields) = list
									.missing_identity
									.iter()
									.find(|fields| fields.user == member.user.id)
								{
									let global_name = if fields.global_name
										== model::GlobalNameUpdate::Unchanged
									{
										self.member_identity(list.channel, member.user.id).map_or(
											model::GlobalNameUpdate::Unchanged,
											|identity| identity.global_name,
										)
									} else {
										fields.global_name
									};
									self.timeline.apply_author_identity(&model::MemberIdentity {
										global_name,
										guild: list.guild.unwrap_or(Id(0)),
										user: member.user.id,
										profile: Some(member.user.clone()),
										avatar: Patch::Absent,
										nick: if fields.nick {
											Patch::Absent
										} else {
											member.nick.clone().map_or(Patch::Null, Patch::Value)
										},
										roles: if fields.roles {
											Patch::Absent
										} else {
											Patch::Value(member.roles.clone())
										},
										departed: false,
									});
								} else if let Some(guild) = list.guild {
									self.timeline.apply_author_identity(&model::MemberIdentity {
										global_name: model::GlobalNameUpdate::Set,
										guild,
										user: member.user.id,
										profile: Some(member.user.clone()),
										avatar: Patch::Absent,
										nick: member.nick.clone().map_or(Patch::Null, Patch::Value),
										roles: Patch::Value(member.roles.clone()),
										departed: false,
									});
								} else {
									self.timeline.apply_author_membership_with_profile(
										member.user.id,
										&member.roles,
										member.nick.as_deref(),
										&member.user,
									);
								}
							}
						}
						if list.ranges.is_empty()
							&& let Some(ranges) = kept_ranges.filter(|ranges| !ranges.is_empty())
						{
							list.ranges = ranges;
						}
						self.member_chunks.merge(&list);
						self.member_chunks.evict(&list.ranges);
						self.members = Some(list);
					}
				}
				Ok(())
			}
			Event::Ready {
				permissions,
				user,
				guilds,
				channels,
			} => {
				self.clear_direct_presences();
				// READY starts a complete snapshot; a presence warning may follow it.
				self.direct_presence_lossy = false;
				if self
					.user
					.as_ref()
					.is_some_and(|previous| previous.id != user.id)
				{
					self.cancel_history();
					self.auth = auth::AuthState::Failed;
					self.status = "Different account rejected; log out before switching accounts";
					return;
				}
				let permission_state = match prepared_permissions.map(Ok).unwrap_or_else(|| {
					let spare_bytes = (guilds.capacity() - guilds.len()) * size_of::<Guild>()
						+ (channels.capacity() - channels.len()) * size_of::<Channel>();
					prepare_navigation(&user, &guilds, &channels, permissions, spare_bytes)
				}) {
					Ok(permissions) => permissions,
					Err(status) => {
						self.cancel_history();
						self.auth = auth::AuthState::Failed;
						self.status = status;
						return;
					}
				};
				let mut current: Vec<_> = channels.iter().collect();
				current.sort_unstable_by_key(|channel| channel.id);
				let current_channel = |id| {
					current
						.binary_search_by_key(&id, |channel| channel.id)
						.ok()
						.map(|index| current[index])
				};
				let mut removed: BTreeSet<_> = self
					.channels
					.iter()
					.filter(|old| {
						current_channel(old.id).is_none_or(|channel| {
							(navigable(old) && !navigable(channel))
								|| (old.supports_text() != channel.supports_text())
						})
					})
					.map(|channel| channel.id)
					.collect();
				let unavailable = self.selected.is_some_and(|id| {
					current_channel(id).is_none_or(|channel| !navigable(channel))
				});
				if unavailable && let Some(selected) = self.selected {
					removed.insert(selected);
				}
				drop(current);
				self.remove_channels(&removed);
				self.cancel_history();
				if unavailable {
					self.arrived_home();
					self.freshness = Freshness::Unavailable;
				} else {
					self.freshness = Freshness::Stale;
				}
				self.voice.roster.clear();
				self.voice.channel_statuses.clear();
				self.voice.status_requested = None;
				self.voice.status_retry = None;
				self.voice.preview = None;
				self.voice.dm_calls.clear();
				self.voice.dm_participants.clear();
				self.voice.dm_ringing.clear();
				self.voice.ring_error = None;
				self.members = None;
				self.member_search = Default::default();
				self.member_labels.clear();
				self.parked_members = None;
				self.clear_profile();
				self.profile_cache.clear();
				self.read_state.reset();
				self.notification_preferences = notifications::Preferences::default();
				self.preference_queue = Default::default();
				self.browse_channels = None;
				self.cancel_user_action();
				self.cancel_server_action();
				self.cancel_channel_action();
				self.cancel_server_settings();
				self.cancel_onboarding();
				self.cancel_member_guide();
				self.cancel_server_admin();
				self.cancel_group_action();
				self.cancel_invite_join();
				self.cancel_guild_creation();
				self.user_actions.reset();
				self.server_actions.reset();
				self.channel_actions.reset();
				self.server_settings.reset();
				self.reset_onboarding();
				self.member_guide.reset();
				self.server_admin.reset();
				self.server_members_shortcuts.clear();
				self.server_incidents.clear();
				self.group_actions.reset();
				self.clear_own_profile();
				self.own_server_profile.reset();
				self.retain_owned_cosmetic_session(user.id);
				self.retain_account_settings_session(user.id);
				self.retain_reaction_preferences(user.id);
				self.user = Some(user);
				self.guilds = guilds;
				self.channels = channels;
				self.permissions = permission_state;
				self.archived_thread = None;
				self.interrupt_gif_favorites();
				self.auth = auth::AuthState::Authenticated;
				self.gateway_connected = true;
				self.status = if unavailable {
					"Conversation no longer available in navigation"
				} else {
					""
				};
				Ok(())
			}
			Event::History {
				channel,
				mut request,
				older,
				mut messages,
			} => {
				if preload::is_preload(request) {
					match self.preload.conversations.arrived(request, true) {
						preload::Arrival::Adopted(own, _) => request = own,
						preload::Arrival::Keep => return self.park_preloaded(channel, messages),
						preload::Arrival::Drop => return,
					}
				}
				if self.selected != Some(channel)
					|| request != self.request
					|| !self.history_pending
					|| !self.can_read_history(channel)
					|| !self
						.channels
						.iter()
						.any(|loaded| loaded.id == channel && loaded.supports_text())
				{
					return;
				}
				messages.retain(|message| !message.ephemeral);
				let mut ids = BTreeSet::new();
				let has_deleted_reference = messages.iter().any(|message| message.reply_deleted);
				if older != self.history_before.is_some()
					|| messages.len() > 50
					|| messages.iter().any(|message| {
						message.channel != channel
							|| ((has_deleted_reference
								|| self.history_navigation.target(channel, request).is_some())
								&& !Timeline::valid_message(message))
							|| self
								.history_before
								.is_some_and(|before| message.id >= before)
							|| self.history_after.is_some_and(|after| message.id <= after)
							|| !ids.insert(message.id)
					}) {
					if self.fail_history_navigation(auth::Failure::Protocol) {
						return;
					}
					self.cancel_history();
					self.fail(auth::Failure::Protocol);
					return;
				}
				if self
					.prepare_target_history(channel, request, &messages, false)
					.is_err()
				{
					return;
				}
				for message in &messages {
					self.record_reply_deletion(message);
				}
				for message in &mut messages {
					self.ingest_reply_preview(message);
				}
				self.history_pending = false;
				if !older
					&& self.history_after.is_none()
					&& let Some(latest) = messages.iter().map(|m| m.id).max()
				{
					self.observe_last_message(channel, latest);
				}
				for message in &mut messages {
					if self.reactions.invalidated(message.id) {
						// Keep newer live counts without retaining stale message content.
						message.reactions = self
							.timeline
							.get(message.id)
							.and_then(|current| current.reactions.clone());
					}
				}
				self.older_exhausted = if older {
					// A short page of older messages is the conversation's start.
					messages.len() < 50
				} else if let Some(after) = self.history_after {
					// A newer page extends the loaded window; it keeps a start already reached.
					after.0 == 0 || (self.older_exhausted && !self.timeline.is_empty())
				} else if self.history_targeted {
					// A page fetched around a target can be short on its newer side, so it never
					// proves the start; scrolling up loads older pages that do.
					false
				} else {
					messages.len() < 50
				};
				if self.history_after.is_some() {
					self.newer_cursor = messages.iter().map(|m| m.id).max();
					self.newer_may_have_more = messages.len() == 50;
				}
				let jump = self.history_after.is_some() && self.timeline.is_empty();
				if self.older_exhausted {
					self.timeline.mark_start_loaded();
				}
				let r = self.timeline.finish_page(messages, older);
				if r.is_ok() && jump {
					self.search_target = self.timeline.iter().next().map(|m| m.id);
					if self.search_target.is_none() {
						self.status = "No messages returned after this boundary; use Jump to present to reload";
					}
				}
				if r.is_ok() {
					self.finish_history_navigation(channel, request);
					self.freshness = if self.gateway_connected {
						Freshness::Fresh
					} else {
						Freshness::Stale
					};
				}
				r
			}
			Event::HistoryFailed {
				channel,
				mut request,
				failure,
			} => {
				let mut adopted = false;
				if preload::is_preload(request) {
					let preload::Arrival::Adopted(own, _) =
						self.preload.conversations.arrived(request, false)
					else {
						return;
					};
					request = own;
					adopted = true;
				}
				if self.selected != Some(channel)
					|| request != self.request
					|| !self.history_pending
				{
					return;
				}
				if self.fail_history_navigation(failure) {
					return;
				}
				if failure == auth::Failure::Capacity && adopted {
					// Preloads give way to clicks and never reached the service: ask for the
					// page as the click would have.
					self.preload.retry = Some(self.history(None));
					return;
				}
				if failure == auth::Failure::Capacity {
					self.command_rejected(Command::History {
						channel,
						request,
						before: None,
						after: None,
					});
					return;
				}
				self.cancel_history();
				if failure == auth::Failure::Forbidden {
					self.clear_search();
					self.application_commands.clear();
					self.invalidate_members();
					self.timeline.clear();
					self.freshness = Freshness::Unavailable;
					self.status = "Channel unavailable or permission denied";
				} else {
					self.fail(failure);
				}
				Ok(())
			}
			Event::ReplyPreview(event) => {
				self.apply_reply_preview(event);
				Ok(())
			}
			Event::Message(m) => self.apply_message(m),
			Event::Patch(mut p) => {
				if !matches!(p.content, Patch::Absent)
					&& self.message_actions.edit_pending(p.channel, p.id)
					&& self.timeline.get(p.id).is_some_and(
						|old| !matches!(p.edited, Patch::Value(at) if old.edited_at.is_some_and(|old| at < old)),
					) {
					self.message_actions.observe_content(p.channel, p.id);
				}
				if self.selected == Some(p.channel)
					&& self.reactions.invalidated(p.id)
					&& !matches!(p.reactions, Patch::Absent)
				{
					self.queue_reaction_read(p.id);
				}
				if self.reactions.invalidated(p.id) {
					p.reactions = Patch::Absent;
				}
				if self.selected == Some(p.channel)
					&& self.can_view(p.channel)
					&& self.freshness != Freshness::Unavailable
				{
					self.timeline.patch(p)
				} else {
					Ok(())
				}
			}
			Event::Delete { channel, id } => {
				self.parked_replies.forget_targets(channel, &[id]);
				self.read_state.activity.delete(channel, id);
				self.forget_pending_replies(channel, &[id]);
				if let Some(channel) = self
					.channels
					.iter_mut()
					.find(|c| c.id == channel && c.last_message == Some(id))
				{
					self.read_state.activity.observe_latest(channel.id, id);
					channel.last_message = None;
				}
				if self.selected == Some(channel) {
					self.deleted_navigation_target(channel, id);
					if self.reply_target() == Some(id) {
						self.reply = None;
					}
					self.timeline.delete(id)
				} else {
					Ok(())
				}
			}
			Event::Edited {
				channel,
				message,
				request,
				result,
			} => {
				self.apply_edit_result(channel, message, request, result);
				Ok(())
			}
			Event::Pinned {
				channel,
				message,
				pinned,
				request,
				result,
			} => {
				self.apply_pin_result(channel, message, pinned, request, result);
				Ok(())
			}
			Event::DeleteBulk { channel, ids } => {
				self.parked_replies.forget_targets(channel, &ids);
				if ids.len() <= 100 {
					for id in &ids {
						self.read_state.activity.delete(channel, *id);
					}
					self.forget_pending_replies(channel, &ids);
				}
				if let Some(channel) = self
					.channels
					.iter_mut()
					.find(|c| c.id == channel && c.last_message.is_some_and(|id| ids.contains(&id)))
					&& let Some(id) = channel.last_message.take()
				{
					self.read_state.activity.observe_latest(channel.id, id);
				}
				if ids.len() > 100 {
					Err("Bulk deletion exceeds safe capacity")
				} else if self.selected == Some(channel) {
					for id in &ids {
						self.deleted_navigation_target(channel, *id);
					}
					if self.reply_target().is_some_and(|id| ids.contains(&id)) {
						self.reply = None;
					}
					ids.into_iter().try_for_each(|id| self.timeline.delete(id))
				} else {
					Ok(())
				}
			}
			Event::SendResult { nonce, result } => self.apply_send_result(nonce, result),
			Event::SendThrottled { nonce } => {
				// Once per burst, like Discord: a dismissed notice stays away until every queued
				// message has gone out.
				if !self.send_throttle_seen
					&& self
						.pending
						.iter()
						.any(|p| p.nonce == nonce && p.delivery == Delivery::Sending)
				{
					self.send_throttled = true;
				}
				Ok(())
			}
			Event::SendChallenge {
				nonce,
				reply,
				challenge,
			} => {
				self.apply_send_challenge(nonce, reply, *challenge);
				Ok(())
			}
			Event::CommandRejected(command) => {
				self.command_rejected(*command);
				Ok(())
			}
			Event::Failure(f) => {
				self.fail(f);
				Ok(())
			}
			Event::FailureDetail(detail) => {
				self.failure_detail = Some(detail);
				Ok(())
			}
			Event::Disconnected => {
				self.voice.channel_statuses.clear();
				self.voice.status_requested = None;
				self.voice.status_retry = None;
				self.release_message_challenge(
					"Verification ended by disconnect; retry the message",
				);
				self.member_search = Default::default();
				self.member_labels.clear();
				self.parked_members = None;
				self.cancel_message_actions();
				self.cancel_user_action();
				self.cancel_server_action();
				self.cancel_channel_action();
				self.cancel_server_settings();
				self.cancel_onboarding();
				self.cancel_member_guide();
				self.cancel_server_admin();
				self.cancel_group_action();
				self.cancel_invite_join();
				self.cancel_guild_creation();
				self.read_state.cancel();
				self.clear_profile();
				// The voice socket is independent and a RESUME replays roster changes, so the call,
				// roster and known DM calls all stay. Only a fresh READY invalidates the voice state.
				self.voice.incoming = None;
				if std::mem::replace(&mut self.gateway_connected, false) {
					self.cancel_history();
					self.freshness = Freshness::Stale;
				}
				// Retry notifications must not invalidate a REST reload started during the outage.
				self.status = "Reconnecting…";
				Ok(())
			}
			Event::Resumed => {
				self.member_search = Default::default();
				// Keep member changes replayed before RESUMED; Disconnected retired old labels.
				self.gateway_connected = true;
				self.cancel_history();
				self.freshness = Freshness::Stale;
				self.status = "";
				Ok(())
			}
			Event::Resync | Event::PermissionsChanged => {
				self.cancel_message_actions();
				self.cancel_user_action();
				self.cancel_server_action();
				self.cancel_channel_action();
				self.cancel_server_settings();
				self.cancel_onboarding();
				self.cancel_member_guide();
				self.cancel_server_admin();
				self.cancel_group_action();
				self.cancel_invite_join();
				self.cancel_guild_creation();
				self.clear_direct_presences();
				// Until the next READY, a missing record is unknown, not offline.
				self.direct_presence_lossy = true;
				self.permissions = permissions::Permissions::default();
				self.read_state.cancel();
				self.clear_profile();
				self.profile_cache.clear();
				self.disconnect_voice(
					"Discord session or permissions changed; rejoin after refreshing",
				);
				self.member_search = Default::default();
				self.member_labels.clear();
				self.parked_members = None;
				self.members = None;
				self.timeline.clear();
				self.freshness = Freshness::Stale;
				self.cancel_history();
				self.status = "";
				Ok(())
			}
			Event::Unavailable(channel) | Event::ThreadRemoved { id: channel, .. } => {
				let mut removed = BTreeSet::from([channel]);
				removed.extend(
					self.channels
						.iter()
						.filter(|c| matches!(c.kind, 10..=12) && c.parent_id == Some(channel))
						.map(|c| c.id),
				);
				self.remove_channels(&removed);
				self.status = "Channel unavailable or permission denied";
				Ok(())
			}
		};
		// Older-page retention can evict the live tail, including an arrival racing the
		// page. Once detached, later live messages must not bridge the missing range.
		if result.is_ok()
			&& self.selected == previous_channel
			&& self.history_before.is_some()
			&& let Some(tail) = previous_tail.max(incoming_tail)
			&& !self.timeline.is_deleted(tail)
			&& self
				.timeline
				.iter()
				.next_back()
				.is_none_or(|message| message.id < tail)
		{
			self.history_targeted = true;
			self.newer_cursor = None;
			self.newer_may_have_more = false;
		}
		if let Err(status) = result {
			self.clear_cached_history();
			self.status = status;
			self.freshness = Freshness::Stale;
			self.timeline.clear();
			self.cancel_history();
		}
		if self
			.reply_target()
			.is_some_and(|target| self.timeline.is_deleted(target))
		{
			self.reply = None;
		}
		if access_changed {
			self.reconcile_permissions(previous_access);
			self.prune_community();
			if let Some(previous) = previous_member_list {
				let current = self
					.selected
					.and_then(|id| self.channel(id))
					.and_then(|channel| self.member_list_id(channel));
				if previous != current {
					// Let the visible pane request the new list; late snapshots lose their request scope.
					self.close_members();
				}
			}
			self.reconcile_notifications();
			self.prune_resident();
			self.prune_post_summaries();
			self.prune_thread_starter();
		}
		self.reconcile_feature_access(access_changed);
		self.enforce_resident_budget();
	}
	pub(super) fn remove_channels(&mut self, removed: &BTreeSet<Id>) {
		self.voice
			.channel_statuses
			.retain(|channel, _| !removed.contains(channel));
		// Check scope before removing navigation identity, including sibling search hits.
		if removed
			.iter()
			.any(|channel| self.search_covers_channel(*channel))
		{
			self.clear_search();
		}
		self.invalidate_navigation();
		self.last_viewed_threads.retain(|id| !removed.contains(id));
		for id in removed {
			self.resident.remove(*id);
		}
		if self
			.archives
			.as_ref()
			.is_some_and(|view| removed.contains(&view.parent))
		{
			self.clear_archives();
		}
		if self.archived_thread.is_some_and(|id| removed.contains(&id)) {
			self.archived_thread = None;
		}
		self.channels.retain(|c| !removed.contains(&c.id));
		self.parked_replies.forget_channels(removed);
		self.inbox
			.messages
			.retain(|message| !removed.contains(&message.channel));
		for item in &mut self.inbox.saved {
			if removed.contains(&item.channel) {
				item.body = None;
			}
		}
		self.permissions.forget_channels(removed);
		for id in removed {
			self.permissions.channels.remove(id);
			self.end_voice_channel(*id);
			self.read_state.forget(*id);
			self.forget_direct_inbox(*id);
		}
		if self.selected.is_some_and(|id| removed.contains(&id)) {
			self.clear_search();
			self.search_target = None;
			self.reply = None;
			self.invalidate_members();
			self.timeline.clear();
			self.freshness = Freshness::Unavailable;
			self.cancel_history();
			self.status = "Conversation no longer available in navigation";
		}
	}
}
