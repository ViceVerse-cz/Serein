//! Explicitly confirmed extension proposals reuse the ordinary native UI paths.
use crate::{ExtensionContext, ExtensionRequest, MessagingUi, design};
use client_core::{Command, State};
use extensions::{
	ActionResult, ActionResultCode, ActionResultStatus, AppAction, AppView, HostEffect,
	LocalSettingsSnapshot, NotificationSettingsSnapshot,
};
use model::Id;

pub(crate) struct ConfirmedEffect {
	pub plugin: String,
	pub context: ExtensionContext,
	pub effect: HostEffect,
}

fn setting(lines: &mut Vec<String>, label: &str, value: Option<impl std::fmt::Display>) {
	if let Some(value) = value {
		lines.push(format!("{label}: {value}"));
	}
}

fn app_action_description(action: &AppAction) -> String {
	match action {
		AppAction::RequestMessageSearch { query, .. } => {
			format!("Search the current conversation for:\n{query}")
		}
		AppAction::RequestPins { .. } => "Load pinned messages in the current conversation".into(),
		AppAction::RequestArchives {
			parent_id, kind, ..
		} => format!("Load {kind:?} archived threads for channel {parent_id}"),
		AppAction::RequestMemberSearch { channel_id, query } => {
			format!("Search members in channel {channel_id} for:\n{query}")
		}
		AppAction::RequestProfile { user_id, .. } => format!("Load the profile for user {user_id}"),
		AppAction::RequestGifs { query } => format!(
			"Load GIF {}",
			query
				.as_deref()
				.map_or("categories".into(), |query| format!("results for {query}"))
		),
		AppAction::SetMessagingSettings { .. } => {
			"Change account messaging privacy settings".into()
		}
		AppAction::SetGuildFolders { .. } => "Replace the account's server-folder layout".into(),
		AppAction::OpenJoinServer { invite } => {
			format!("Open the join-server flow for invite {invite}")
		}
		AppAction::SendServerInvite { guild_id, user_id } => {
			format!("Send an invite to server {guild_id} to friend {user_id}")
		}
		AppAction::OpenServerAdmin { guild_id, page } => {
			format!("Open the {page:?} settings page for server {guild_id}")
		}
		AppAction::OpenGroupEditor { channel_id } => {
			format!("Open group conversation settings for channel {channel_id}")
		}
		AppAction::SendMessage {
			channel_id,
			content,
		} => format!("Send a message to channel {channel_id}:\n{content}"),
		AppAction::SendReply {
			channel_id,
			message_id,
			content,
			mention,
		} => format!(
			"Reply to message {message_id} in channel {channel_id} (mention author: {mention}):\n{content}"
		),
		AppAction::SendSticker {
			channel_id,
			sticker_id,
		} => {
			format!("Send sticker {sticker_id} to channel {channel_id}")
		}
		AppAction::ForwardMessage {
			channel_id,
			message_id,
			target_channel_ids,
			note,
		} => format!(
			"Forward message {message_id} from channel {channel_id} to: {}\nNote: {note}",
			target_channel_ids.join(", ")
		),
		AppAction::EditMessage {
			channel_id,
			message_id,
			content,
		} => format!("Replace message {message_id} in channel {channel_id} with:\n{content}"),
		AppAction::DeleteMessage {
			channel_id,
			message_id,
		} => format!(
			"Permanently delete message {message_id} in channel {channel_id}. This cannot be undone."
		),
		AppAction::SetReaction {
			channel_id,
			message_id,
			emoji,
			add,
		} => format!(
			"{} reaction {emoji} on message {message_id} in channel {channel_id}",
			if *add { "Add your" } else { "Remove your" }
		),
		AppAction::SetMessagePinned {
			channel_id,
			message_id,
			pinned,
		} => format!(
			"{} message {message_id} in channel {channel_id}",
			if *pinned { "Pin" } else { "Unpin" }
		),
		AppAction::MarkRead {
			channel_id,
			message_id,
		} => format!("Mark channel {channel_id} read through message {message_id}"),
		AppAction::MarkUnread {
			channel_id,
			message_id,
		} => format!("Mark channel {channel_id} unread from message {message_id}"),
		AppAction::MarkChannelRead { channel_id } => format!("Mark channel {channel_id} read"),
		AppAction::MarkGuildRead { guild_id } => format!("Mark server {guild_id} read"),
		AppAction::JumpToUnread => "Open the current conversation's unread messages".into(),
		AppAction::CreateThread {
			channel_id,
			name,
			message_id,
		} => format!(
			"Create thread in channel {channel_id}:\n{name}\nStarter message: {}",
			message_id.as_deref().unwrap_or("none")
		),
		AppAction::CreateForumPost {
			parent_id,
			title,
			content,
		} => format!("Publish post in channel {parent_id}:\n{title}\n\n{content}"),
		AppAction::SetThreadArchived {
			channel_id,
			archived,
		} => format!(
			"{} thread {channel_id}",
			if *archived { "Archive" } else { "Unarchive" }
		),
		AppAction::SetThreadLocked { channel_id, locked } => format!(
			"{} thread {channel_id}",
			if *locked { "Lock" } else { "Unlock" }
		),
		AppAction::SetThreadFollowed {
			channel_id,
			followed,
		} => format!(
			"{} thread {channel_id}",
			if *followed { "Follow" } else { "Unfollow" }
		),
		AppAction::SetThreadPinned { channel_id, pinned } => format!(
			"{} forum post {channel_id}",
			if *pinned { "Pin" } else { "Unpin" }
		),
		AppAction::RenameThread { channel_id, name } => {
			format!("Rename thread {channel_id}:\n{name}")
		}
		AppAction::OpenFriendDm { user_id } => {
			format!("Open direct messages with friend {user_id}")
		}
		AppAction::SetFriendNickname { user_id, text } => {
			format!("Set nickname for friend {user_id} (empty clears it):\n{text}")
		}
		AppAction::SetUserNote { user_id, text } => {
			format!("Replace your note for user {user_id} (empty clears it):\n{text}")
		}
		AppAction::AddFriend { username } => format!("Send a friend request to:\n{username}"),
		AppAction::RemoveFriend { user_id } => format!("Remove user {user_id} from your friends"),
		AppAction::ResolveFriendRequest { user_id, accept } => format!(
			"{} friend request from user {user_id}",
			if *accept { "Accept" } else { "Reject" }
		),
		AppAction::SetUserBlocked { user_id, blocked } => format!(
			"{} user {user_id}",
			if *blocked { "Block" } else { "Unblock" }
		),
		AppAction::SetActivitySharing { enabled } => format!(
			"{} sharing detected game activity",
			if *enabled { "Enable" } else { "Disable" }
		),
		AppAction::SetOwnProfile { profile } => {
			let mut lines = vec!["Change your Discord profile:".into()];
			setting(&mut lines, "Display name", profile.global_name.as_deref());
			setting(&mut lines, "Bio (empty clears it)", profile.bio.as_deref());
			setting(
				&mut lines,
				"Pronouns (empty clears them)",
				profile.pronouns.as_deref(),
			);
			setting(
				&mut lines,
				"Accent color",
				profile.accent_color.map(|color| format!("#{color:06X}")),
			);
			if profile.clear_global_name {
				lines.push("Clear display name".into());
			}
			if profile.clear_accent_color {
				lines.push("Clear accent color".into());
			}
			lines.join("\n")
		}
		AppAction::SetOwnPresence { presence } => {
			let mut lines = vec!["Change your status:".into()];
			setting(&mut lines, "Status", presence.status.as_deref());
			setting(
				&mut lines,
				"Custom status (empty clears it)",
				presence.custom_status.as_deref(),
			);
			setting(
				&mut lines,
				"Clear after seconds (0 means never)",
				presence.clear_after_seconds,
			);
			lines.join("\n")
		}
		AppAction::SetAudioSettings { settings } => {
			let mut lines = vec!["Change device audio settings:".into()];
			setting(&mut lines, "Input gain (%)", settings.input_percent);
			setting(&mut lines, "Output gain (%)", settings.output_percent);
			setting(&mut lines, "Push to talk", settings.push_to_talk);
			setting(
				&mut lines,
				"Input profile",
				settings.input_profile.as_deref(),
			);
			setting(
				&mut lines,
				"Noise suppression",
				settings.suppression.as_deref(),
			);
			setting(
				&mut lines,
				"Suppression strength",
				settings.suppression_level,
			);
			setting(&mut lines, "Echo cancellation", settings.echo_cancellation);
			setting(&mut lines, "Automatic gain", settings.automatic_gain);
			setting(
				&mut lines,
				"Microphone sensitivity (dBFS)",
				settings.sensitivity_db,
			);
			if settings.open_microphone {
				lines.push("Microphone sensitivity: always open".into());
			}
			lines.join("\n")
		}
		AppAction::SetParticipantAudio {
			user_id,
			volume_percent,
			muted,
		} => {
			let mut lines = vec![format!("Change local audio for participant {user_id}:")];
			setting(&mut lines, "Volume (%)", *volume_percent);
			setting(&mut lines, "Locally muted", *muted);
			lines.join("\n")
		}
		AppAction::SetStreamAudio {
			volume_percent,
			muted,
		} => {
			let mut lines = vec!["Change local screen-share audio:".into()];
			setting(&mut lines, "Volume (%)", *volume_percent);
			setting(&mut lines, "Muted", *muted);
			lines.join("\n")
		}
		AppAction::WatchStream { user_id } => {
			format!("Watch the current stream from participant {user_id}")
		}
		AppAction::StopWatching => "Stop watching the current stream".into(),
		AppAction::DeclineCall { channel_id } => {
			format!("Decline the incoming call in channel {channel_id}")
		}
		AppAction::JoinVoice {
			channel_id,
			ring,
			muted,
			deafened,
		} => format!(
			"Join voice in channel {channel_id}\nRing recipients: {ring}\nMicrophone muted: {muted}\nDeafened: {deafened}\nJoining with an unmuted microphone can transmit your audio. Switching calls also requires the native switch confirmation."
		),
		AppAction::SetCamera { enabled } => {
			if *enabled {
				"Enable your camera in the current call. Your video will be shared with call participants.".into()
			} else {
				"Disable your camera in the current call".into()
			}
		}
		AppAction::OpenAttachmentPicker { channel_id } => {
			format!("Open the system file picker to attach files in channel {channel_id}")
		}
		AppAction::SelectAudioDevices {
			input_id,
			output_id,
		} => format!(
			"Select local audio devices\nMicrophone: {}\nSpeakers: {}",
			input_id.as_deref().unwrap_or("unchanged"),
			output_id.as_deref().unwrap_or("unchanged")
		),
		AppAction::RefreshMediaDevices => "Refresh local microphones, speakers and cameras".into(),
		AppAction::SelectCameraDevice { device_id } => format!(
			"Select local camera: {}",
			device_id.as_deref().unwrap_or("system default")
		),
		AppAction::OpenScreenSharePicker => {
			"Open the native screen-share picker for the current call".into()
		}
		AppAction::StopScreenShare => "Stop sharing your screen in the current call".into(),
		AppAction::SetChannelMute {
			channel_id,
			duration_seconds,
		} => format!(
			"Change mute for channel {channel_id} to {}",
			duration_seconds.map_or_else(
				|| "off".into(),
				|seconds| if seconds == 0 {
					"forever".into()
				} else {
					format!("{seconds} seconds")
				}
			)
		),
		AppAction::SetChannelNotifications { channel_id, level } => {
			format!("Set notification level {level} for channel {channel_id}")
		}
		AppAction::SetGuildHideMuted { guild_id, hide } => format!(
			"{} muted channels in server {guild_id}",
			if *hide { "Hide" } else { "Show" }
		),
		AppAction::CreateChannel {
			guild_id,
			name,
			kind,
		} => {
			format!("Create {kind} channel in server {guild_id}:\n{name}")
		}
		AppAction::CreateCategory { guild_id, name } => {
			format!("Create category in server {guild_id}:\n{name}")
		}
		AppAction::DuplicateChannel { channel_id, name } => {
			format!("Duplicate channel {channel_id} as:\n{name}")
		}
		AppAction::EditChannel {
			channel_id, after, ..
		} => {
			format!(
				"Edit channel {channel_id}:\nName: {}\nTopic: {}",
				after.name, after.topic
			)
		}
		AppAction::DeleteChannel { channel_id } => {
			format!("Permanently delete channel {channel_id}. This cannot be undone.")
		}
		AppAction::MoveChannel {
			channel_id,
			parent_id,
			position,
			..
		} => format!(
			"Move channel {channel_id} to parent {} at position {position}",
			parent_id.as_deref().unwrap_or("none")
		),
		AppAction::CreateServerInvite {
			guild_id,
			channel_id,
			max_age,
			max_uses,
			temporary,
		} => format!(
			"Create invite for server {guild_id}\nChannel: {}\nExpires after: {max_age} seconds\nMaximum uses: {max_uses}\nTemporary membership: {temporary}",
			channel_id.as_deref().unwrap_or("automatic")
		),
		AppAction::LeaveServer { guild_id } => {
			format!("Leave server {guild_id}. This removes it from your account.")
		}
		AppAction::UpdateServerSettings { guild_id, .. } => {
			format!("Change settings for server {guild_id}")
		}
		AppAction::CreateRole { guild_id, role } => format!(
			"Create role in server {guild_id}: {}",
			role.name.as_deref().unwrap_or("new role")
		),
		AppAction::EditRole {
			guild_id, role_id, ..
		} => {
			format!("Edit role {role_id} in server {guild_id}")
		}
		AppAction::DeleteRole { guild_id, role_id } => format!(
			"Permanently delete role {role_id} from server {guild_id}. This cannot be undone."
		),
		AppAction::MoveRole {
			guild_id,
			role_id,
			position,
		} => {
			format!("Move role {role_id} in server {guild_id} to position {position}")
		}
		AppAction::SetMemberRole {
			guild_id,
			user_id,
			role_id,
			assigned,
		} => format!(
			"{} role {role_id} {} member {user_id} in server {guild_id}",
			if *assigned { "Assign" } else { "Remove" },
			if *assigned { "to" } else { "from" }
		),
		AppAction::SetMemberNickname {
			guild_id,
			user_id,
			nickname,
		} => {
			format!("Set member {user_id}'s nickname in server {guild_id}:\n{nickname}")
		}
		AppAction::KickMember { guild_id, user_id } => format!(
			"Kick member {user_id} from server {guild_id}. They will lose access immediately."
		),
		AppAction::PruneMembers {
			guild_id,
			days,
			execute,
		} => format!(
			"{} members inactive for {days} days in server {guild_id}",
			if *execute { "Prune" } else { "Preview pruning" }
		),
		AppAction::SetMemberListVisible { guild_id, enabled } => format!(
			"{} the member list in server {guild_id}",
			if *enabled { "Show" } else { "Hide" }
		),
		AppAction::RenameServerEmoji {
			guild_id,
			emoji_id,
			name,
		} => {
			format!("Rename emoji {emoji_id} in server {guild_id} to {name}")
		}
		AppAction::DeleteServerEmoji { guild_id, emoji_id } => format!(
			"Permanently delete emoji {emoji_id} from server {guild_id}. This cannot be undone."
		),
		AppAction::LeaveGroup { channel_id } => {
			format!("Leave group conversation {channel_id}")
		}
		AppAction::RenameGroup { channel_id, name } => {
			format!("Rename group conversation {channel_id}:\n{name}")
		}
		AppAction::CloseDm { channel_id } => format!("Close direct conversation {channel_id}"),
		AppAction::SetConversationMuted { channel_id, muted } => format!(
			"{} conversation {channel_id}",
			if *muted { "Mute" } else { "Unmute" }
		),
	}
}

pub(crate) fn effect_description(effect: &HostEffect) -> String {
	match effect {
		HostEffect::AppAction { action } | HostEffect::TrackedAppAction { action, .. } => {
			app_action_description(action)
		}
		HostEffect::Navigate { channel_id } => format!("Open channel {channel_id}"),
		HostEffect::Home => "Open Friends / Home".into(),
		HostEffect::OpenView { view } => format!("Open {}", view_label(*view)),
		HostEffect::OpenProfile { user_id } => format!("Open profile for user {user_id}"),
		HostEffect::JumpToMessage {
			channel_id,
			message_id,
		} => {
			format!("Open message {message_id} in channel {channel_id}")
		}
		HostEffect::Search { query } => format!("Search this conversation for:\n{query}"),
		HostEffect::Notice { text } => format!("Show this local notice:\n{text}"),
		HostEffect::CopyText { text } => format!("Replace clipboard text with:\n{text}"),
		HostEffect::SetVoice { muted, deafened } => format!(
			"Set the current call to {} and {}",
			if *muted { "muted" } else { "unmuted" },
			if *deafened { "deafened" } else { "undeafened" },
		),
		HostEffect::LeaveVoice => "Leave the current voice call".into(),
		HostEffect::SetLocalSettings { settings } => {
			let mut lines = vec!["Change local reading settings:".into()];
			if let Some(value) = settings.zoom_percent {
				lines.push(format!("Zoom: {value}%"));
			}
			if let Some(value) = settings.sidebar_width {
				lines.push(format!("Sidebar width: {value}"));
			}
			for (label, value) in [
				("Show members", settings.show_members),
				("Animate GIFs", settings.animate_gifs),
				("Smooth scrolling", settings.smooth_scrolling),
				("Hide media links", settings.hide_media_links),
			] {
				if let Some(value) = value {
					lines.push(format!("{label}: {}", if value { "on" } else { "off" }));
				}
			}
			if let Some(value) = settings.scroll_speed_percent {
				lines.push(format!("Scroll speed: {value}%"));
			}
			lines.join("\n")
		}
		HostEffect::SetNotificationSettings { settings } => {
			let mut lines = vec!["Change local notification settings:".into()];
			if let Some(value) = settings.volume {
				lines.push(format!("Sound volume: {value}%"));
			}
			for (label, value) in [
				("New message sound", settings.new_message),
				("Current channel sound", settings.current_channel),
				("Incoming ring", settings.incoming_ring),
				("Outgoing ring", settings.outgoing_ring),
				("Disable sounds", settings.disable_sounds),
				("Unread badge", settings.unread_badge),
				("Mute sound", settings.mute),
				("Unmute sound", settings.unmute),
				("Deafen sound", settings.deafen),
				("Undeafen sound", settings.undeafen),
				("Camera on sound", settings.camera_on),
				("Screen share sound", settings.screen_share_on),
				("User join sound", settings.user_join),
				("User leave sound", settings.user_leave),
			] {
				if let Some(value) = value {
					lines.push(format!("{label}: {}", if value { "on" } else { "off" }));
				}
			}
			lines.join("\n")
		}
	}
}

pub(crate) fn effect_button(effect: &HostEffect) -> &'static str {
	match effect {
		HostEffect::AppAction {
			action: AppAction::DeleteMessage { .. },
		} => "Apply: Delete message",
		HostEffect::AppAction {
			action: AppAction::SendMessage { .. },
		} => "Apply: Send message",
		HostEffect::AppAction {
			action: AppAction::JoinVoice { .. },
		} => "Apply: Join call",
		HostEffect::AppAction {
			action: AppAction::SetCamera { enabled: true },
		} => "Apply: Enable camera",
		HostEffect::AppAction { .. } | HostEffect::TrackedAppAction { .. } => {
			"Apply: Confirm action"
		}
		HostEffect::CopyText { .. } => "Apply: Copy text",
		HostEffect::Notice { .. } => "Apply: Show notice",
		HostEffect::SetVoice { .. } => "Apply: Change call audio",
		HostEffect::LeaveVoice => "Apply: Leave call",
		HostEffect::SetLocalSettings { .. } => "Apply: Change settings",
		HostEffect::SetNotificationSettings { .. } => "Apply: Change notifications",
		HostEffect::Search { .. } => "Apply: Search",
		_ => "Apply: Open view",
	}
}

fn view_label(view: AppView) -> &'static str {
	match view {
		AppView::Friends => "Friends / Home",
		AppView::Search => "conversation search",
		AppView::Pins => "pinned messages",
		AppView::Members => "conversation members",
		AppView::Threads => "threads",
		AppView::Settings => "general settings",
		AppView::Account => "account settings",
		AppView::ProfileSettings => "profile settings",
		AppView::Appearance => "appearance settings",
		AppView::MessagingPermissions => "messaging permission settings",
		AppView::Notifications => "notification settings",
		AppView::Activity => "activity settings",
		AppView::Extensions => "extensions settings",
		AppView::Themes => "theme settings",
		AppView::VoiceSettings => "voice settings",
		AppView::Keybinds => "keyboard shortcut settings",
		AppView::Storage => "data and privacy settings",
		AppView::Updates => "update settings",
	}
}

fn active_channel(state: &State) -> Result<Id, &'static str> {
	state
		.selected
		.filter(|channel| {
			state.can_read_history(*channel)
				&& state.freshness != model::Freshness::Unavailable
				&& state
					.channel(*channel)
					.is_some_and(|channel| channel.supports_text())
		})
		.ok_or("This conversation is no longer accessible")
}

impl MessagingUi {
	pub fn extension_local_settings(&self) -> LocalSettingsSnapshot {
		let settings = self.reading_preferences;
		LocalSettingsSnapshot {
			zoom_percent: settings.zoom_percent,
			sidebar_width: settings.sidebar_width,
			show_members: settings.show_members,
			animate_gifs: settings.animate_gifs,
			hide_media_links: settings.hide_media_links,
			smooth_scrolling: Some(settings.smooth_scrolling),
			scroll_speed_percent: Some(settings.scroll_speed_percent),
		}
	}

	pub fn extension_notification_settings(&self) -> NotificationSettingsSnapshot {
		let settings = self.notification_options;
		NotificationSettingsSnapshot {
			new_message: settings.new_message,
			current_channel: settings.current_channel,
			incoming_ring: settings.incoming_ring,
			outgoing_ring: settings.outgoing_ring,
			disable_sounds: settings.disable_sounds,
			unread_badge: settings.unread_badge,
			mute: settings.mute,
			unmute: settings.unmute,
			deafen: settings.deafen,
			undeafen: settings.undeafen,
			camera_on: settings.camera_on,
			screen_share_on: settings.screen_share_on,
			user_join: settings.user_join,
			user_leave: settings.user_leave,
			volume: settings.volume,
		}
	}

	pub(crate) fn apply_extension_effect(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		confirmed: ConfirmedEffect,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		let ConfirmedEffect {
			plugin,
			context,
			effect,
		} = confirmed;
		let (request_id, effect) = match effect {
			HostEffect::TrackedAppAction { request_id, action } => {
				let entry = self
					.extensions
					.entries
					.iter()
					.find(|entry| {
						entry.manifest.id == plugin && entry.enabled && !entry.cleanup_pending
					})
					.ok_or("The extension is no longer enabled")?;
				HostEffect::TrackedAppAction {
					request_id: request_id.clone(),
					action: action.clone(),
				}
				.validate(&entry.manifest)
				.map_err(|error| error.to_string())?;
				(Some(request_id), HostEffect::AppAction { action })
			}
			effect => (None, effect),
		};
		let rejection_code = if !context.is_current(state) {
			ActionResultCode::ContextChanged
		} else if state.user.is_none()
			|| !(state.demo || state.auth == client_core::auth::AuthState::Authenticated)
		{
			ActionResultCode::Unavailable
		} else if context.channel.is_some_and(|channel| {
			!state.can_view(channel) || state.freshness == model::Freshness::Unavailable
		}) {
			ActionResultCode::Denied
		} else {
			ActionResultCode::Failed
		};
		let feedback_context = context.clone();
		let feedback_plugin = plugin.clone();
		let result = self.apply_extension_effect_inner(
			ctx,
			state,
			ConfirmedEffect {
				plugin,
				context,
				effect,
			},
			commands,
		);
		if let Some(request_id) = request_id {
			let code = match &result {
				Ok(()) => ActionResultCode::Accepted,
				Err(_) => rejection_code,
			};
			self.extensions.queue(
				ctx,
				ExtensionRequest::ActionResult {
					id: feedback_plugin,
					result: ActionResult {
						request_id,
						status: if result.is_ok() {
							ActionResultStatus::Accepted
						} else {
							ActionResultStatus::Rejected
						},
						code,
					},
					context: feedback_context,
				},
			);
		}
		result
	}

	fn apply_extension_effect_inner(
		&mut self,
		ctx: &egui::Context,
		state: &mut State,
		confirmed: ConfirmedEffect,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		if !confirmed.context.is_current(state)
			|| state.user.is_none()
			|| !(state.demo || state.auth == client_core::auth::AuthState::Authenticated)
		{
			return Err("The account or conversation changed; run the extension again.".into());
		}
		if confirmed.context.channel.is_some_and(|channel| {
			!state.can_view(channel) || state.freshness == model::Freshness::Unavailable
		}) {
			return Err("This conversation is no longer accessible".into());
		}
		let entry = self
			.extensions
			.entries
			.iter()
			.find(|entry| {
				entry.manifest.id == confirmed.plugin && entry.enabled && !entry.cleanup_pending
			})
			.ok_or("The extension is no longer enabled")?;
		confirmed
			.effect
			.validate(&entry.manifest)
			.map_err(|error| error.to_string())?;
		let plugin_name = entry.manifest.name.clone();
		match confirmed.effect {
			HostEffect::TrackedAppAction { .. } => {
				unreachable!("tracked effects are normalized before dispatch")
			}
			HostEffect::AppAction { action } => {
				if matches!(
					action,
					AppAction::SetStreamAudio { .. } | AppAction::StopWatching
				) && state.voice.active.as_ref().and_then(|call| call.watching)
					!= confirmed.context.watched_stream
				{
					return Err("The watched stream changed; run the extension again.".into());
				}
				if matches!(
					action,
					AppAction::SetParticipantAudio { .. }
						| AppAction::SetStreamAudio { .. }
						| AppAction::WatchStream { .. }
						| AppAction::StopWatching
						| AppAction::SetCamera { .. }
				) {
					self.extension_voice_current(state, confirmed.context.voice_request)?;
				}
				match action {
					AppAction::JoinVoice {
						channel_id,
						ring,
						muted,
						deafened,
					} => {
						let current = state
							.voice
							.active
							.as_ref()
							.map(|call| (call.channel, call.request));
						if current != confirmed.context.voice_request {
							return Err("The call changed; run the extension again.".into());
						}
						let channel = Id(channel_id
							.parse()
							.map_err(|_| "Invalid channel identifier")?);
						self.request_call_with_audio(
							state, channel, ring, muted, deafened, commands,
						)?;
					}
					AppAction::SetCamera { enabled } => {
						if enabled && !self.voice_camera_available {
							return Err("Camera capture is unavailable".into());
						}
						let call = state.voice.active.as_ref().ok_or("No active call")?;
						if call.camera != enabled {
							commands.push(
								state
									.set_call_camera(enabled)
									.ok_or("Camera controls are unavailable")?,
							);
						}
					}
					action => self.apply_extension_app_action(state, action, commands)?,
				}
			}
			HostEffect::Navigate { channel_id } => {
				self.extension_navigate(state, &channel_id, None, commands)?
			}
			HostEffect::JumpToMessage {
				channel_id,
				message_id,
			} => {
				self.extension_navigate(state, &channel_id, Some(&message_id), commands)?;
			}
			HostEffect::Home
			| HostEffect::OpenView {
				view: AppView::Friends,
			} => {
				if !self.server_settings.navigate_away(state) {
					return Err("Finish editing server settings first".into());
				}
				self.guild = None;
				state.open_home();
				self.search.open = false;
			}
			HostEffect::OpenView { view } => match view {
				AppView::Search => {
					let channel = active_channel(state)?;
					if !state.can_search() {
						return Err("Search is unavailable while disconnected".into());
					}
					let query = state
						.search
						.as_ref()
						.filter(|search| !search.pins && search.channel == channel)
						.map(|search| search.query.clone());
					self.search.open_extension(channel, false, query);
				}
				AppView::Pins => {
					let channel = active_channel(state)?;
					let command = state
						.request_pins()
						.ok_or("Pinned messages are unavailable")?;
					self.search.open_extension(channel, true, None);
					commands.push(command);
				}
				AppView::Members => {
					active_channel(state)?;
					self.search.open = false;
					self.reading_preferences.show_members = true;
					self.members_narrow_open = true;
					commands.extend(state.request_members());
				}
				AppView::Threads => {
					let channel = state.selected.ok_or("Choose a conversation first")?;
					let command = state
						.request_archives(channel, model::archives::Kind::Public, None)
						.ok_or("Threads are unavailable in this conversation")?;
					commands.push(command);
					self.guild = state.channel(channel).and_then(|channel| channel.guild);
					self.archives.focus = true;
					self.search.open = false;
				}
				_ => {
					self.open_extension_settings(view);
				}
			},
			HostEffect::OpenProfile { user_id } => {
				let id = Id(user_id.parse().map_err(|_| "Invalid user identifier")?);
				let user = state
					.user
					.as_ref()
					.filter(|user| user.id == id)
					.cloned()
					.or_else(|| state.friend(id).cloned())
					.or_else(|| {
						active_channel(state).ok().and_then(|channel| {
							crate::mentions::known_users(state, channel)
								.into_iter()
								.find(|user| user.id == id)
						})
					})
					.ok_or("This user is not available in the current session")?;
				self.profile.navigate(user);
			}
			HostEffect::Search { query } => {
				let channel = active_channel(state)?;
				let command = state
					.request_search(query.clone(), None)
					.ok_or("This search is unavailable or invalid")?;
				self.search.open_extension(channel, false, Some(query));
				commands.push(command);
			}
			HostEffect::Notice { text } => self
				.toasts
				.push(design::Level::Info, format!("{plugin_name}: {text}")),
			HostEffect::CopyText { text } => ctx.copy_text(text),
			HostEffect::SetVoice { muted, deafened } => {
				self.extension_voice_current(state, confirmed.context.voice_request)?;
				if !muted
					&& !deafened && !state.demo
					&& !state.can_speak(state.voice.active.as_ref().unwrap().channel)
				{
					return Err("Speaking is unavailable in this call".into());
				}
				let command = state
					.set_call_mute(muted, deafened)
					.ok_or("Call controls are unavailable")?;
				if let Some(call) = &state.voice.active {
					self.voice_muted = call.muted;
					self.voice_deafened = call.deafened;
				}
				commands.push(command);
			}
			HostEffect::LeaveVoice => {
				self.extension_voice_current(state, confirmed.context.voice_request)?;
				commands.extend(state.leave_call());
			}
			HostEffect::SetLocalSettings { settings } => {
				let mut value = self.reading_preferences;
				value.zoom_percent = settings.zoom_percent.unwrap_or(value.zoom_percent);
				value.sidebar_width = settings.sidebar_width.unwrap_or(value.sidebar_width);
				value.show_members = settings.show_members.unwrap_or(value.show_members);
				value.animate_gifs = settings.animate_gifs.unwrap_or(value.animate_gifs);
				value.hide_media_links =
					settings.hide_media_links.unwrap_or(value.hide_media_links);
				value.smooth_scrolling =
					settings.smooth_scrolling.unwrap_or(value.smooth_scrolling);
				value.scroll_speed_percent = settings
					.scroll_speed_percent
					.unwrap_or(value.scroll_speed_percent);
				self.apply_reading_preferences(ctx, value);
			}
			HostEffect::SetNotificationSettings { settings } => {
				let value = &mut self.notification_options;
				value.new_message = settings.new_message.unwrap_or(value.new_message);
				value.current_channel = settings.current_channel.unwrap_or(value.current_channel);
				value.incoming_ring = settings.incoming_ring.unwrap_or(value.incoming_ring);
				value.outgoing_ring = settings.outgoing_ring.unwrap_or(value.outgoing_ring);
				value.disable_sounds = settings.disable_sounds.unwrap_or(value.disable_sounds);
				value.unread_badge = settings.unread_badge.unwrap_or(value.unread_badge);
				value.mute = settings.mute.unwrap_or(value.mute);
				value.unmute = settings.unmute.unwrap_or(value.unmute);
				value.deafen = settings.deafen.unwrap_or(value.deafen);
				value.undeafen = settings.undeafen.unwrap_or(value.undeafen);
				value.camera_on = settings.camera_on.unwrap_or(value.camera_on);
				value.screen_share_on = settings.screen_share_on.unwrap_or(value.screen_share_on);
				value.user_join = settings.user_join.unwrap_or(value.user_join);
				value.user_leave = settings.user_leave.unwrap_or(value.user_leave);
				value.volume = settings.volume.unwrap_or(value.volume);
			}
		}
		ctx.request_repaint();
		Ok(())
	}

	fn extension_navigate(
		&mut self,
		state: &mut State,
		channel: &str,
		message: Option<&str>,
		commands: &mut Vec<Command>,
	) -> Result<(), String> {
		let channel = Id(channel.parse().map_err(|_| "Invalid channel identifier")?);
		let guild = state
			.channel(channel)
			.filter(|_| state.can_read_history(channel))
			.ok_or("This channel is not readable in the current session")?
			.guild;
		let message = message
			.map(|value| value.parse().map(Id))
			.transpose()
			.map_err(|_| "Invalid message identifier")?;
		if !self.server_settings.navigate_away(state) {
			return Err("Finish editing server settings first".into());
		}
		commands.extend(state.open_chat_link(guild, channel, message)?);
		self.guild = guild;
		self.search.open = false;
		Ok(())
	}

	fn extension_voice_current(
		&self,
		state: &State,
		request: Option<(Id, u64)>,
	) -> Result<(), String> {
		if !(self.voice_available || state.demo)
			|| !state
				.voice
				.active
				.as_ref()
				.is_some_and(|call| Some((call.channel, call.request)) == request)
		{
			return Err(
				"The voice call changed or its controls are unavailable; run the extension again."
					.into(),
			);
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ExtensionEntry;
	use extensions::{Capability, ExtensionKind, Invocation, LocalSettingsPatch, Manifest, Output};

	fn view(capability: Capability) -> MessagingUi {
		let mut view = MessagingUi::default();
		view.extensions.entries.push(ExtensionEntry {
			description: String::new(),
			preview: None,
			theme_preview: None,
			cover_image: None,
			local_theme: false,
			manifest: Manifest {
				api_version: 1,
				id: "synthetic.app".into(),
				name: "Synthetic app tools".into(),
				version: "1.0.0".into(),
				author: "Offline fixture".into(),
				license: "MIT".into(),
				source: "https://example.com/source".into(),
				kind: ExtensionKind::Plugin,
				capabilities: vec![capability],
				actions: vec![],
			},
			reviewed: false,
			sha256: "a".repeat(64),
			download_bytes: 4096,
			enabled: true,
			cleanup_pending: false,
			update_available: false,
			update_manifest: None,
		});
		view
	}

	fn proposal(state: &State, effect: HostEffect) -> ConfirmedEffect {
		ConfirmedEffect {
			plugin: "synthetic.app".into(),
			context: ExtensionContext::capture(state, false),
			effect,
		}
	}

	fn frame(
		ctx: &egui::Context,
		view: &mut MessagingUi,
		state: &mut State,
		events: Vec<egui::Event>,
	) -> Vec<(String, egui::Rect)> {
		fn labels(shape: &egui::Shape, output: &mut Vec<(String, egui::Rect)>) {
			match shape {
				egui::Shape::Text(text) => {
					output.push((text.galley.job.text.clone(), text.visual_bounding_rect()))
				}
				egui::Shape::Vec(shapes) => {
					for shape in shapes {
						labels(shape, output);
					}
				}
				_ => {}
			}
		}
		let output = ctx.run_ui(
			egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(900.0, 700.0),
				)),
				events,
				focused: true,
				..Default::default()
			},
			|ui| {
				if let Some(effect) =
					view.extensions
						.show_result(ui.ctx(), state, &mut Vec::new(), false)
				{
					view.apply_extension_effect(ui.ctx(), state, effect, &mut Vec::new())
						.unwrap();
				}
			},
		);
		let mut text = Vec::new();
		for shape in &output.shapes {
			labels(&shape.shape, &mut text);
		}
		output.drop_without_applying_deltas();
		text
	}

	#[test]
	fn host_action_waits_for_apply_and_close_discards_it() {
		for approve in [false, true] {
			let ctx = egui::Context::default();
			let mut state = test_support::demo_state();
			let mut view = view(Capability::LocalSettings);
			let before = view.reading_preferences;
			view.extensions.present_output(
				"synthetic.app".into(),
				Invocation::default(),
				ExtensionContext::capture(&state, false),
				Output {
					effects: vec![HostEffect::SetLocalSettings {
						settings: LocalSettingsPatch {
							sidebar_width: Some(300),
							..Default::default()
						},
					}],
					..Default::default()
				},
				&state,
			);
			frame(&ctx, &mut view, &mut state, vec![]);
			let text = frame(&ctx, &mut view, &mut state, vec![]);
			assert_eq!(view.reading_preferences, before);
			assert!(
				text.iter()
					.any(|(text, _)| text.contains("Sidebar width: 300"))
			);
			let button = if approve {
				"Apply: Change settings"
			} else {
				"Close"
			};
			let pos = text
				.iter()
				.find(|(text, _)| text == button)
				.unwrap()
				.1
				.center();
			for pressed in [true, false] {
				frame(
					&ctx,
					&mut view,
					&mut state,
					vec![
						egui::Event::PointerMoved(pos),
						egui::Event::PointerButton {
							pos,
							button: egui::PointerButton::Primary,
							pressed,
							modifiers: egui::Modifiers::NONE,
						},
					],
				);
			}
			assert_eq!(
				view.reading_preferences.sidebar_width,
				if approve { 300 } else { before.sidebar_width }
			);
			assert!(!view.extensions.has_result());
		}
	}

	#[test]
	fn apply_rechecks_context_extension_and_grant() {
		for failure in 0..7 {
			let ctx = egui::Context::default();
			let mut state = test_support::demo_state();
			let mut view = view(Capability::LocalSettings);
			let effect = proposal(
				&state,
				HostEffect::SetLocalSettings {
					settings: LocalSettingsPatch {
						sidebar_width: Some(300),
						..Default::default()
					},
				},
			);
			let before = view.reading_preferences;
			match failure {
				0 => {
					state.selected = Some(Id(21));
				}
				1 => {
					state.generation += 1;
				}
				2 => {
					view.extensions.entries[0].enabled = false;
				}
				3 => {
					view.extensions.entries[0].cleanup_pending = true;
				}
				4 => {
					view.extensions.entries[0].manifest.capabilities.clear();
				}
				5 => {
					state
						.channels
						.retain(|channel| Some(channel.id) != state.selected);
				}
				_ => {
					state.freshness = model::Freshness::Unavailable;
				}
			}
			let mut commands = Vec::new();
			assert!(
				view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
					.is_err()
			);
			assert_eq!(view.reading_preferences, before);
			assert!(commands.is_empty());
		}
	}

	#[test]
	fn navigation_uses_only_known_readable_channels_and_home_clears_selection() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = view(Capability::Navigation);
		let mut commands = Vec::new();
		let before = state.selected;
		let effect = proposal(
			&state,
			HostEffect::Navigate {
				channel_id: "999999999".into(),
			},
		);
		assert!(
			view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
				.is_err()
		);
		assert_eq!(state.selected, before);
		assert!(commands.is_empty());
		let effect = proposal(
			&state,
			HostEffect::Navigate {
				channel_id: "21".into(),
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert_eq!(state.selected, Some(Id(21)));
		assert_eq!(view.guild, Some(Id(10)));
		view.search.open = true;
		let effect = proposal(&state, HostEffect::Home);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert_eq!(state.selected, None);
		assert_eq!(view.guild, None);
		assert!(!view.search.open);
	}

	#[test]
	fn voice_proposals_cannot_affect_a_replacement_call() {
		for leave in [false, true] {
			for changed_channel in [false, true] {
				let ctx = egui::Context::default();
				let mut state = test_support::call_demo_state();
				let mut view = view(Capability::VoiceControl);
				let effect = proposal(
					&state,
					if leave {
						HostEffect::LeaveVoice
					} else {
						HostEffect::SetVoice {
							muted: true,
							deafened: true,
						}
					},
				);
				let call = state.voice.active.as_mut().unwrap();
				if changed_channel {
					call.channel = Id(25);
				} else {
					call.request += 1;
				}
				let original = (call.channel, call.request, call.muted, call.deafened);
				let mut commands = Vec::new();
				assert!(
					view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
						.is_err()
				);
				let call = state.voice.active.as_ref().unwrap();
				assert_eq!(
					(call.channel, call.request, call.muted, call.deafened),
					original
				);
				assert!(commands.is_empty());
			}
		}
	}

	#[test]
	fn search_and_threads_follow_the_current_channel_through_ui_sync() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = view(Capability::Navigation);
		let mut commands = Vec::new();
		let effect = proposal(
			&state,
			HostEffect::Search {
				query: "hello".into(),
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		view.search.sync(&ctx, &mut state, &mut commands);
		assert!(view.search.open);
		assert_eq!(state.search.as_ref().unwrap().query, "hello");
		assert!(matches!(
			commands.as_slice(),
			[Command::Search {
				channel: Id(20),
				..
			}]
		));
		commands.clear();
		let effect = proposal(
			&state,
			HostEffect::OpenView {
				view: AppView::Threads,
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		view.search.sync(&ctx, &mut state, &mut commands);
		assert!(!view.search.open);
		assert_eq!(view.guild, Some(Id(10)));
		assert_eq!(state.archives.as_ref().unwrap().parent, Id(20));
		assert!(matches!(
			commands.as_slice(),
			[Command::Archives { parent: Id(20), .. }]
		));
		let user = state.user.as_ref().unwrap().clone();
		let effect = proposal(
			&state,
			HostEffect::OpenProfile {
				user_id: user.id.0.to_string(),
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert_eq!(view.profile.open_user().map(|user| user.id), Some(user.id));
	}

	#[test]
	fn current_call_controls_reuse_bounded_voice_commands() {
		let ctx = egui::Context::default();
		let mut state = test_support::call_demo_state();
		let mut view = view(Capability::VoiceControl);
		let mut commands = Vec::new();
		let effect = proposal(
			&state,
			HostEffect::SetVoice {
				muted: true,
				deafened: true,
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert!(matches!(
			commands.pop(),
			Some(Command::Voice(client_core::voice::Command::SetMute {
				mute: true,
				deaf: true,
				..
			}))
		));
		let effect = proposal(&state, HostEffect::LeaveVoice);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert!(matches!(
			commands.pop(),
			Some(Command::Voice(client_core::voice::Command::Leave { .. }))
		));
		assert!(state.voice.active.is_none());
	}

	#[test]
	fn preference_patch_preserves_other_values_and_rejects_invalid_range() {
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = view(Capability::LocalSettings);
		view.reading_preferences.confirm_external_links = false;
		let before = view.reading_preferences;
		let effect = proposal(
			&state,
			HostEffect::SetLocalSettings {
				settings: LocalSettingsPatch {
					zoom_percent: Some(120),
					hide_media_links: Some(!before.hide_media_links),
					smooth_scrolling: Some(false),
					scroll_speed_percent: Some(250),
					..Default::default()
				},
			},
		);
		view.apply_extension_effect(&ctx, &mut state, effect, &mut Vec::new())
			.unwrap();
		assert_eq!(
			view.reading_preferences,
			model::ReadingPreferences {
				zoom_percent: 120,
				hide_media_links: !before.hide_media_links,
				smooth_scrolling: false,
				scroll_speed_percent: 250,
				..before
			}
		);
		assert_eq!(view.extension_local_settings().zoom_percent, 120);
		let applied = view.reading_preferences;
		for settings in [
			LocalSettingsPatch {
				zoom_percent: Some(151),
				..Default::default()
			},
			LocalSettingsPatch {
				scroll_speed_percent: Some(301),
				..Default::default()
			},
			LocalSettingsPatch::default(),
		] {
			let effect = proposal(&state, HostEffect::SetLocalSettings { settings });
			assert!(
				view.apply_extension_effect(&ctx, &mut state, effect, &mut Vec::new())
					.is_err()
			);
			assert_eq!(view.reading_preferences, applied);
		}
	}
	#[test]
	fn app_actions_recheck_grants_and_replacement_streams_before_mutating() {
		let ctx = egui::Context::default();
		let mut state = test_support::call_demo_state();
		let mut view = view(Capability::AudioSettings);
		state.voice.active.as_mut().unwrap().watching = Some(Id(301));
		let effect = proposal(
			&state,
			HostEffect::AppAction {
				action: AppAction::SetStreamAudio {
					volume_percent: Some(25),
					muted: None,
				},
			},
		);
		state.voice.active.as_mut().unwrap().watching = Some(Id(302));
		assert!(
			view.apply_extension_effect(&ctx, &mut state, effect, &mut Vec::new())
				.is_err()
		);
		assert_eq!(view.voice_stream_volume(), 100);
		let effect = proposal(
			&state,
			HostEffect::AppAction {
				action: AppAction::SetAudioSettings {
					settings: extensions::AudioSettingsPatch {
						input_percent: Some(25),
						..Default::default()
					},
				},
			},
		);
		let before = view.voice_gain.input_percent;
		view.extensions.entries[0].manifest.capabilities.clear();
		assert!(
			view.apply_extension_effect(&ctx, &mut state, effect, &mut Vec::new())
				.is_err()
		);
		assert_eq!(view.voice_gain.input_percent, before);
	}

	#[test]
	fn destructive_and_media_proposals_describe_the_actual_operation() {
		let deletion = HostEffect::AppAction {
			action: AppAction::DeleteMessage {
				channel_id: "20".into(),
				message_id: "200".into(),
			},
		};
		assert_eq!(effect_button(&deletion), "Apply: Delete message");
		assert!(effect_description(&deletion).contains("cannot be undone"));
		let camera = HostEffect::AppAction {
			action: AppAction::SetCamera { enabled: true },
		};
		assert_eq!(effect_button(&camera), "Apply: Enable camera");
		assert!(effect_description(&camera).contains("shared with call participants"));
	}

	#[test]
	fn notification_patch_revalidates_grant_and_preserves_apply_time_values() {
		use extensions::NotificationSettingsPatch;
		let ctx = egui::Context::default();
		let mut state = test_support::demo_state();
		let mut view = view(Capability::NotificationSettings);
		let effect = proposal(
			&state,
			HostEffect::SetNotificationSettings {
				settings: NotificationSettingsPatch {
					volume: Some(25),
					current_channel: Some(true),
					..Default::default()
				},
			},
		);
		view.notification_options.user_join = false;
		let before = view.notification_options;
		let mut commands = Vec::new();
		view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
			.unwrap();
		assert_eq!(
			view.notification_options,
			model::notification_preferences::Device {
				volume: 25,
				current_channel: true,
				..before
			}
		);
		assert!(commands.is_empty());
		assert_eq!(view.extension_notification_settings().volume, 25);
		let applied = view.notification_options;
		for settings in [
			NotificationSettingsPatch {
				volume: Some(101),
				disable_sounds: Some(true),
				..Default::default()
			},
			NotificationSettingsPatch::default(),
		] {
			let effect = proposal(&state, HostEffect::SetNotificationSettings { settings });
			assert!(
				view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
					.is_err()
			);
			assert_eq!(view.notification_options, applied);
		}
		let effect = proposal(
			&state,
			HostEffect::SetNotificationSettings {
				settings: NotificationSettingsPatch {
					volume: Some(0),
					..Default::default()
				},
			},
		);
		view.extensions.entries[0].manifest.capabilities.clear();
		assert!(
			view.apply_extension_effect(&ctx, &mut state, effect, &mut commands)
				.is_err()
		);
		assert_eq!(view.notification_options, applied);
	}
}
