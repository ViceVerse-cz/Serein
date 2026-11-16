//! Commands emitted by the state owner; transports execute them outside rendering.
use super::{
	Envelope, Event, Freshness, Id, Reply, State, application_commands, auth, captcha,
	channel_actions, community, group_actions, guild_creation, inbox, interactions, member_guide,
	member_search, mfa, onboarding, owned_cosmetics, polls, preload, reactions, read_state,
	reply_previews, reporting, server_actions, server_admin, server_profile, server_settings,
	soundboard, user_actions, voice,
};

pub enum Command {
	ChannelPreferences(Box<crate::preferences_sync::Request>),
	Mfa(Box<mfa::Request>),
	Autocomplete(interactions::Request),
	CancelAutocomplete,

	Community {
		guild: Id,
		user: Id,
		fence: u64,
		request: u64,
		action: Box<model::community::Action>,
	},
	Mentions {
		before: Option<Id>,
		request: u64,
	},
	CancelMentions,
	InboxAction {
		request: u64,
		action: inbox::Action,
	},
	Report(Box<reporting::Request>),
	Polls(polls::Request),
	Soundboard(soundboard::Request),
	StickerPacks,
	Sticker(Id),
	Interaction(interactions::Request),
	ApplicationCommands {
		channel: Id,
		guild: Option<Id>,
		context: u8,
		request: u64,
	},
	MemberSearch(member_search::Request),
	/// Local display hints only; the composer debounces the corresponding live query.
	MemberSearchLocal(member_search::Request),
	MessagingPermissions {
		request: u64,
		change: Option<model::messaging_permissions::Change>,
	},
	ChannelAction {
		guild: Id,
		channel: Id,
		request: u64,
		action: channel_actions::Action,
	},
	ServerAdmin {
		guild: Id,
		request: u64,
		action: Box<model::server_admin::Action>,
	},
	ServerSettings {
		guild: Id,
		request: u64,
		edit: Option<Box<model::server_settings::Edit>>,
	},
	MemberGuide {
		guild: Id,
		user: Id,
		request: u64,
		action: model::member_guide::Action,
	},
	Onboarding {
		guild: Id,
		request: u64,
		action: onboarding::Action,
	},
	SendServerInvite {
		guild: Id,
		user: Id,
		code: String,
		nonce: String,
		request: u64,
	},
	/// None loads the current settings; Some saves the complete folder layout.
	/// Load, or save `(loaded base, desired)` if Discord still has the base folders.
	GuildFolders(
		Option<(
			model::guild_folders::Settings,
			model::guild_folders::Settings,
		)>,
	),
	GroupAction {
		action: group_actions::Action,
		request: u64,
	},
	ServerAction {
		action: server_actions::Action,
		request: u64,
	},
	UserAction {
		action: user_actions::Action,
		request: u64,
		captcha: Option<Box<captcha::Retry>>,
	},
	JoinInvite {
		code: String,
		request: u64,
		captcha: Option<Box<captcha::Retry>>,
	},
	CreateGuild {
		request: guild_creation::Request,
		sequence: u64,
	},
	Invite {
		code: String,
	},
	CreatePost {
		parent: Id,
		guild: Id,
		title: String,
		content: String,
		/// Forum tag IDs applied to the new post.
		tags: Vec<Id>,
		/// Filenames staged for the starter message, in selection order; empty sends text only.
		attachments: Vec<String>,
		request: u64,
	},
	ForumSummaries {
		channels: Vec<Id>,
		request: u64,
	},
	ForumPosts {
		parent: Id,
		guild: Id,
		offset: usize,
		request: u64,
	},
	Archives {
		parent: Id,
		guild: Id,
		kind: model::archives::Kind,
		before: Option<model::archives::Cursor>,
		request: u64,
	},
	Pins {
		channel: Id,
		before: Option<i128>,
		request: u64,
	},
	Search {
		channel: Id,
		guild: Option<Id>,
		query: String,
		before: Option<Id>,
		offset: u32,
		/// Oldest messages first; page one then starts at the earliest match.
		oldest: bool,
		request: u64,
	},
	CancelSearch,
	/// The message the selected thread hangs off, read from its parent channel.
	ThreadStarter {
		thread: Id,
		parent: Id,
		request: u64,
	},
	Gifs {
		query: Option<String>,
		request: u64,
	},
	CancelGifs,
	GifFavorites {
		request: u64,
		change: Option<(model::Gif, bool)>,
	},
	ReactionPreferences {
		request: u64,
	},
	MarkRead {
		channel: Id,
		message: Id,
		request: u64,
		manual: bool,
		/// Private mark-unread badge. `None` omits the field. Discord does not calculate it.
		mention_count: Option<u32>,
	},
	MarkGuildRead {
		guild: Id,
		request: u64,
	},
	/// Latest message per unread channel, at most `MAX_BULK_ACK`.
	MarkAllRead {
		request: u64,
		channels: Vec<(Id, Id)>,
	},
	Reactions(reactions::Command),
	Profile {
		user: Id,
		guild: Option<Id>,
		request: u64,
		with_mutuals: bool,
		/// Hidden effects and frames skip their optional catalog read.
		cosmetics: model::CosmeticPreferences,
	},
	CancelProfile,
	/// The account's own badge order and hidden set; one explicit write.
	BadgeSettings {
		request: u64,
		settings: model::badges::BadgeSettings,
	},
	StreamPreview {
		guild: Id,
		channel: Id,
		user: Id,
		request: u64,
	},
	/// None loads the account's global profile; Some writes only explicitly changed fields.
	EditProfile {
		user: Id,
		request: u64,
		changes: Option<model::ProfileEdit>,
	},
	ServerProfile {
		user: Id,
		guild: Id,
		request: u64,
		edit: Option<Box<model::server_profile::Edit>>,
	},
	AccountOverview {
		user: Id,
		request: u64,
	},
	AccountConnections {
		user: Id,
		request: u64,
		change: Option<model::account_settings::ConnectionChange>,
	},
	OwnedCosmetics {
		user: Id,
		guild: Option<Id>,
		request: u64,
		change: Option<model::profile_cosmetics::Change>,
		nitro: bool,
	},
	Voice(voice::Command),
	Members {
		thread: bool,
		guild: Option<Id>,
		channel: Option<Id>,
		request: u64,
		list_id: Option<String>,
		ranges: Vec<[usize; 2]>,
	},
	History {
		channel: Id,
		before: Option<Id>,
		after: Option<Id>,
		request: u64,
	},
	HistoryAround {
		channel: Id,
		target: Id,
		request: u64,
	},
	ReplyPreview {
		channel: Id,
		message: Id,
		request: u64,
	},
	CancelReplyPreviews,
	/// The command a click would send, sent early on intent and answered by that command's own
	/// event: a conversation's recent `History` page or a `Profile`.
	Prefetch(Box<Command>),
	Forward {
		source: Id,
		message: Id,
		guild: Option<Id>,
		channel: Id,
		nonce: String,
	},
	Send {
		sticker: Option<Id>,
		channel: Id,
		content: String,
		nonce: String,
		reply: Option<Reply>,
	},
	/// One explicit resend of a challenged message with the user's solution; never automatic.
	VerifiedSend {
		sticker: Option<Id>,
		channel: Id,
		content: String,
		nonce: String,
		reply: Option<Reply>,
		captcha: Box<captcha::Retry>,
	},
	Edit {
		request: u64,
		channel: Id,
		message: Id,
		content: String,
	},
	Delete {
		channel: Id,
		message: Id,
	},
	Pin {
		request: u64,
		channel: Id,
		message: Id,
		pinned: bool,
	},
}

impl State {
	/// Reports a command the transport could not accept as a bounded outcome error.
	pub fn command_rejected(&mut self, command: Command) {
		if let Command::Report(request) = command {
			self.apply_report(reporting::Event {
				target: request.target,
				request: request.request,
				result: Err(auth::Failure::ProtocolAt(
					"Report queue is full; retry explicitly",
				)),
			});
			return;
		}
		if matches!(command, Command::CancelReplyPreviews) {
			// Core retired these request IDs before asking the worker to cancel. A full
			// queue cannot make their late results current or invalidate this conversation.
			return;
		}
		if let Command::Polls(request) = command {
			let _ = self.apply_poll(polls::Event::Result {
				channel: request.channel,
				message: request.message,
				request: request.request,
				result: Err(auth::Failure::Capacity),
			});
			return;
		}
		if let Command::Mfa(request) = &command {
			self.reject_mfa(request.id);
			return;
		}
		if let Command::Soundboard(request) = &command {
			self.soundboard_rejected(request);
			return;
		}
		match &command {
			Command::ApplicationCommands {
				channel, request, ..
			} => {
				self.apply_application_commands(
					*channel,
					*request,
					Err(auth::Failure::ProtocolAt(
						"Application commands were not queued; try again",
					)),
				);
				return;
			}
			Command::StickerPacks => {
				self.stickers.loading = false;
				self.stickers.error = Some("Sticker packs were not queued; try again");
				return;
			}
			Command::Sticker(_) => {
				self.stickers.detail_loading = None;
				self.stickers.detail_error = Some("Sticker details were not queued; try again");
				return;
			}
			_ => {}
		}
		if let Command::Autocomplete(request) = &command {
			self.apply_autocomplete(application_commands::CompletionEvent::Submitted {
				nonce: request.nonce.clone(),
				result: Err(auth::Failure::Network),
			});
		}
		if let Command::Interaction(request) = command {
			let _ = self.apply_interaction(interactions::Event::Submitted {
				nonce: request.nonce,
				result: Err(auth::Failure::Network),
			});
			return;
		}
		if let Command::MemberSearch(request) = command {
			self.searched_members(
				request,
				Err(auth::Failure::ProtocolAt("Member lookup was not queued")),
			);
			return;
		}
		if let Command::MessagingPermissions { request, .. } = command {
			self.apply_messaging_permissions(
				request,
				Err(auth::Failure::ProtocolAt(
					"Messaging permissions were not queued; try again",
				)),
			);
			return;
		}
		if let Command::Community { guild, request, .. } = command {
			let _ = self.apply_community(community::Event {
				guild,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Community action was not queued; try again",
				)),
			});
			return;
		}
		if let Command::ServerAdmin { guild, request, .. } = command {
			let _ = self.apply_server_admin(server_admin::Event {
				guild,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Server administration was not queued; try again",
				)),
			});
			return;
		}
		if let Command::MemberGuide {
			guild,
			user,
			request,
			..
		} = command
		{
			let _ = self.apply_member_guide(member_guide::Event {
				guild,
				user,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Server Guide was not queued; try again",
				)),
			});
			return;
		}
		if let Command::Onboarding { guild, request, .. } = command {
			let _ = self.apply_onboarding(onboarding::Event::Loaded {
				guild,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Server onboarding was not queued; try again",
				)),
			});
			let _ = self.apply_onboarding(onboarding::Event::Submitted {
				guild,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Server onboarding was not queued; try again",
				)),
			});
			return;
		}
		if let Command::ServerSettings { guild, request, .. } = command {
			let _ = self.apply_server_settings(server_settings::Event {
				guild,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Server settings were not queued; reload to continue",
				)),
				refreshed: None,
			});
			return;
		}
		if let Command::EditProfile { user, request, .. } = command {
			self.reject_own_profile(user, request);
			return;
		}
		if let Command::ServerProfile {
			user,
			guild,
			request,
			..
		} = command
		{
			self.apply_server_profile(server_profile::Event {
				user,
				guild,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Server profile request was not queued; reload to continue",
				)),
			});
			return;
		}
		if let Command::OwnedCosmetics {
			user,
			guild,
			request,
			..
		} = command
		{
			self.apply_owned_cosmetics(owned_cosmetics::Event {
				user,
				guild,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Owned cosmetic request was not queued; reload to continue",
				)),
			});
			return;
		}
		if let Command::AccountOverview { user, request } = command {
			self.apply_account_overview(
				user,
				request,
				Err(auth::Failure::ProtocolAt(
					"Account details request was not queued; reload to continue",
				)),
			);
			return;
		}
		if let Command::AccountConnections { user, request, .. } = command {
			self.apply_account_connections(
				user,
				request,
				Err(auth::Failure::ProtocolAt(
					"Account connections request was not queued; reload to continue",
				)),
			);
			return;
		}
		if let Command::BadgeSettings { request, .. } = command {
			self.apply_badge_settings(
				request,
				Err(auth::Failure::ProtocolAt(
					"Badge settings were not queued; try again",
				)),
			);
			return;
		}
		if matches!(command, Command::GuildFolders(_)) {
			self.apply_guild_folders(Err(auth::Failure::ProtocolAt(
				"Server organization was not queued; try again",
			)));
			return;
		}
		if let Command::GroupAction { action, request } = command {
			let event = if matches!(action, group_actions::Action::Create(_)) {
				group_actions::Event::Created {
					request,
					result: Err(auth::Failure::ProtocolAt(
						"Group creation was not queued; try again",
					)),
				}
			} else if matches!(
				action,
				group_actions::Action::LoadMembers(_) | group_actions::Action::TransferOwner { .. }
			) {
				group_actions::Event::MembersLoaded {
					channel: action.channel(),
					request,
					result: Err(auth::Failure::ProtocolAt(
						"Group member list was not queued; try again",
					)),
				}
			} else {
				group_actions::Event::Written {
					channel: action.channel(),
					request,
					result: Err(auth::Failure::ProtocolAt(
						"Group action was not queued; try again",
					)),
				}
			};
			let _ = self.apply_group_action(event);
			return;
		}
		if let Command::SendServerInvite {
			guild,
			user,
			request,
			..
		} = command
		{
			let _ = self.apply_server_action(server_actions::Event::InviteSent {
				guild,
				user,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Invite was not queued; try again",
				)),
			});
			return;
		}
		if let Command::ChannelPreferences(request) = command {
			let _ = self.finish_channel_preferences(crate::preferences_sync::Finished {
				generation: request.generation,
				guild: request.guild,
				request: request.request,
				result: Err(auth::Failure::ProtocolAt(
					"Preference save was not queued; retry",
				)),
			});
			return;
		}
		if let Command::ChannelAction {
			guild,
			channel,
			request,
			..
		} = command
		{
			let _ = self.apply_channel_action(channel_actions::Event::Finished {
				guild,
				channel,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Channel action was not queued; try again",
				)),
			});
			return;
		}
		if let Command::ServerAction { action, request } = command {
			let _ = self.apply_server_action(server_actions::Event::Written {
				action,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Server action was not queued; try again",
				)),
			});
			return;
		}
		if let Command::UserAction {
			action, request, ..
		} = command
		{
			let _ = self.apply_user_action(user_actions::Event::Written {
				action,
				request,
				result: Err(auth::Failure::ProtocolAt(
					"User action was not queued; try again",
				)),
			});
			return;
		}
		if let Command::CreatePost {
			parent, request, ..
		} = command
		{
			self.apply_post(parent, request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::Archives {
			parent, request, ..
		} = command
		{
			self.apply_archives(parent, request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::ThreadStarter {
			thread, request, ..
		} = command
		{
			self.apply_thread_starter(thread, request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::ForumSummaries { channels, request } = command {
			self.apply_forum_summaries(
				request,
				channels
					.into_iter()
					.map(|channel| (channel, Err(auth::Failure::Capacity)))
					.collect(),
			);
			return;
		}
		if let Command::ForumPosts {
			parent, request, ..
		} = command
		{
			self.apply_forum_posts(parent, request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::Search {
			channel, request, ..
		}
		| Command::Pins {
			channel, request, ..
		} = command
		{
			self.apply_search(channel, request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::InboxAction { request, .. } = command {
			self.apply_inbox_action(request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::Mentions { request, .. } = command {
			self.apply_mentions(request, Err(auth::Failure::Capacity));
			return;
		}
		if matches!(
			command,
			Command::CancelSearch | Command::CancelGifs | Command::CancelMentions
		) {
			return;
		}
		if let Command::Gifs { request, .. } = command {
			self.apply_gifs(request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::GifFavorites { request, .. } = command {
			self.apply_gif_favorites(request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::ReactionPreferences { request } = command {
			self.apply_reaction_preferences(request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::Edit {
			channel,
			message,
			request,
			..
		} = command
		{
			self.apply_edit_result(channel, message, request, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::Pin {
			request,
			channel,
			message,
			pinned,
		} = command
		{
			self.apply(Envelope {
				generation: self.generation,
				event: Event::Pinned {
					request,
					channel,
					message,
					pinned,
					result: Err(auth::Failure::Capacity),
				},
			});
			return;
		}
		if let Command::MarkRead {
			channel,
			message,
			request,
			..
		} = command
		{
			let _ = self.apply_read_state(read_state::Event::Result {
				channel,
				message,
				request,
				result: Err(auth::Failure::RateLimited),
			});
			self.read_state.status = Some((channel, "Work queue full; read marker was not sent"));
			return;
		}
		if let Command::MarkGuildRead { guild, request } = command {
			let _ = self.apply_read_state(read_state::Event::GuildAck {
				guild,
				request,
				result: Err(auth::Failure::RateLimited),
			});
			return;
		}
		if let Command::MarkAllRead { request, .. } = command {
			let _ = self.apply_read_state(read_state::Event::AllAck {
				request,
				acked: 0,
				result: Err(auth::Failure::RateLimited),
			});
			return;
		}
		if let Command::Reactions(command) = command {
			let event = match command {
				reactions::Command::Read {
					channel,
					message,
					request,
				} => reactions::Event::Read {
					channel,
					message,
					request,
					result: Err(auth::Failure::RateLimited),
				},
				reactions::Command::Set {
					channel,
					message,
					request,
					..
				} => reactions::Event::Written {
					channel,
					message,
					request,
					result: Err(auth::Failure::RateLimited),
				},
				reactions::Command::Users {
					channel,
					message,
					emoji,
					request,
					..
				} => reactions::Event::Users {
					channel,
					message,
					emoji,
					request,
					result: Err(auth::Failure::RateLimited),
				},
			};
			let _ = self.apply_reactions(event);
			self.status = "Work queue full; reaction action was not sent";
			return;
		}
		if let Command::JoinInvite { request, .. } = command {
			self.apply_invite_join(
				request,
				Err(auth::Failure::ProtocolAt(
					"Join was not sent · work queue full",
				)),
			);
			return;
		}
		if let Command::CreateGuild { sequence, .. } = command {
			self.apply_guild_created(
				sequence,
				Err(auth::Failure::ProtocolAt(
					"Server creation was not queued; check Discord before retrying",
				)),
			);
			return;
		}
		if let Command::Invite { code } = command {
			self.apply_invite(code, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::Profile {
			user,
			guild,
			request,
			..
		} = command
		{
			self.apply_profile(user, guild, request, Err(auth::Failure::Capacity));
			return;
		}
		if matches!(command, Command::CancelProfile) {
			return;
		}
		if let Command::StreamPreview {
			guild,
			channel,
			user,
			request,
		} = command
		{
			self.apply_stream_preview(guild, channel, user, request, Err(auth::Failure::Capacity));
			return;
		}

		if let Command::Voice(control) = command {
			match control {
				voice::Command::ChannelInfo { guild } => {
					self.retry_voice_channel_statuses(guild);
				}
				voice::Command::RingRecipient {
					channel, request, ..
				} => self.apply_voice(voice::Event::RingFailed {
					channel,
					request,
					message: "Recipient ringing was not sent; the work queue is full",
				}),
				voice::Command::Sync { .. } => {
					self.status = "Call status could not refresh; reopen the DM to retry"
				}
				voice::Command::Join {
					channel, request, ..
				}
				| voice::Command::ConfirmSession {
					channel, request, ..
				}
				| voice::Command::Ring { channel, request } => self.apply_voice(voice::Event::Failed {
					channel,
					request,
					message: "Call action was not sent; the work queue is full",
				}),
				voice::Command::SetCamera {
					channel, request, ..
				} => {
					if let Some(call) = &mut self.voice.active
						&& call.channel == channel
						&& call.request == request
					{
						call.camera = false;
						self.status = "Camera control was not sent; camera is off locally";
					}
				}
				_ => self.status = "Call control was not sent; local mute/hangup still applies",
			}
			return;
		}
		if matches!(&command, Command::Members { .. }) {
			self.invalidate_members();
			return;
		}
		if let Command::Prefetch(command) = command {
			// The click that adopted a rejected preload fails as its own request would.
			match *command {
				Command::History {
					channel, request, ..
				} => {
					if let preload::Arrival::Adopted(own, _) =
						self.preload.conversations.arrived(request, false)
					{
						self.command_rejected(Command::History {
							channel,
							before: None,
							after: None,
							request: own,
						});
					}
				}
				Command::Profile {
					user,
					guild,
					request,
					..
				} => self.apply_profile(user, guild, request, Err(auth::Failure::Capacity)),
				_ => {}
			}
			return;
		}
		if let Command::ReplyPreview {
			channel,
			message,
			request,
		} = &command
		{
			self.apply_reply_preview(reply_previews::ResultEvent {
				channel: *channel,
				message: *message,
				request: *request,
				result: Err(auth::Failure::ProtocolAt("Preview queue is full; retry")),
			});
			// This result bypasses apply(), but its negative cache entry still allocates.
			self.enforce_resident_budget();
			return;
		}
		if let Command::History {
			channel, request, ..
		}
		| Command::HistoryAround {
			channel, request, ..
		} = &command
		{
			if self.selected != Some(*channel) || *request != self.request || !self.history_pending
			{
				return;
			}
			if self.fail_history_navigation(auth::Failure::Capacity) {
				return;
			}
			self.cancel_history();
			self.status = "Work queue full; action was not sent";
			self.freshness = Freshness::Stale;
			return;
		}
		if let Command::Send { nonce, .. }
		| Command::VerifiedSend { nonce, .. }
		| Command::Forward { nonce, .. } = command
		{
			self.apply(Envelope {
				generation: self.generation,
				event: Event::SendResult {
					nonce,
					result: Err(auth::Failure::ProtocolAt(
						"Work queue full; message was not sent",
					)),
				},
			});
			return;
		}
		self.status = "Work queue full; action was not sent";
		self.freshness = Freshness::Stale;
	}
}
