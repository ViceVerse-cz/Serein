//! Generation-tagged adapter results and their bounded allocation charges.
use super::{
	Channel, ChannelPatch, CustomEmoji, Guild, GuildPatch, Id, Member, MemberList, MemberPresence,
	MemberSlot, Message, MessagePatch, Patch, PreparedStartup, Reply, Sticker, User, UserProfile,
	application_commands, auth, captcha, channel_actions, community, custom_emoji_bytes,
	group_actions, inbox, interactions, member_guide, member_search, mfa, notifications,
	onboarding, owned_cosmetics, permissions, polls, presence, reactions, read_state,
	reply_previews, reporting, search, server_actions, server_admin, server_profile,
	server_settings, soundboard, sticker_bytes, stickers, typing, user_actions, voice,
};

pub enum Event {
	ChannelPreferences(crate::preferences_sync::Finished),
	Mfa(mfa::Event),
	Autocomplete(application_commands::CompletionEvent),

	Community(community::Event),
	CommunityChanged {
		guild: Id,
		channel: Option<Id>,
		stage_ended: bool,
	},
	CommunityFence(u64),
	Polls(polls::Event),
	Soundboard(soundboard::Event),
	StickerEntitlement {
		user: Id,
		premium_type: Patch<u8>,
	},
	StickerPacks(Result<Vec<model::StickerPack>, auth::Failure>),
	Sticker {
		id: Id,
		result: Result<model::Sticker, auth::Failure>,
	},
	GuildStickers {
		guild: Id,
		stickers: Vec<model::Sticker>,
	},
	Interaction(interactions::Event),
	ApplicationCommands {
		channel: Id,
		request: u64,
		result: Result<Vec<model::application_commands::Command>, auth::Failure>,
	},
	MemberSearch {
		request: member_search::Request,
		result: Result<Vec<Member>, auth::Failure>,
	},
	/// Successful OP-8 results with fields Discord omitted from individual members.
	MemberSearchPartial {
		request: member_search::Request,
		rows: Vec<Member>,
		missing: Vec<model::MissingMemberFields>,
	},
	MemberIdentity(Box<model::MemberIdentity>),
	Startup(Box<PreparedStartup>),
	StartupWarnings(model::account::Warnings),
	/// Redacted decode cause preceding a session failure; schema paths only, never remote values.
	FailureDetail(Box<str>),
	MessagingPermissions {
		request: u64,
		result: Result<model::messaging_permissions::Snapshot, auth::Failure>,
	},
	InviteChallenge {
		request: u64,
		challenge: Box<captcha::Challenge>,
	},
	ChannelAction(channel_actions::Event),
	ServerAdmin(server_admin::Event),
	MemberReviewProgress(server_admin::ReviewProgress),
	BulkModeration {
		guild: Id,
		report: model::moderation::BulkReport,
	},
	ServerSettings(server_settings::Event),
	MemberGuide(member_guide::Event),
	Onboarding(onboarding::Event),
	JoinInvite {
		request: u64,
		result: Result<Id, auth::Failure>,
	},
	GuildJoined(Guild),
	/// Permanent account departure; transient guild outages retain navigation metadata.
	GuildRemoved(Id),
	GuildCreated {
		sequence: u64,
		result: Result<Id, auth::Failure>,
	},
	GuildFolders(Result<model::guild_folders::Settings, auth::Failure>),
	/// Discord says another session (or this one) changed account settings.
	AccountSettings {
		status: bool,
		folders: bool,
	},
	UserAction(user_actions::Event),
	ServerAction(server_actions::Event),
	GroupAction(group_actions::Event),
	Invite {
		code: String,
		result: Result<Box<model::InvitePreview>, auth::Failure>,
	},
	Typing(typing::Signal),
	PostCreated {
		parent: Id,
		request: u64,
		result: Result<Channel, auth::Failure>,
	},
	Archives {
		parent: Id,
		request: u64,
		result: Result<model::archives::Page, auth::Failure>,
	},
	ThreadStarter {
		thread: Id,
		request: u64,
		result: Result<Message, auth::Failure>,
	},
	ForumSummaries {
		request: u64,
		results: Vec<(Id, Result<model::forum::Summary, auth::Failure>)>,
	},
	ForumPosts {
		parent: Id,
		request: u64,
		result: Result<model::forum::Page, auth::Failure>,
	},
	Search {
		channel: Id,
		request: u64,
		result: Result<search::Outcome, auth::Failure>,
	},
	Mentions {
		request: u64,
		result: Result<Vec<Message>, auth::Failure>,
	},
	InboxAction {
		request: u64,
		result: Result<inbox::Outcome, auth::Failure>,
	},
	InboxInvalidated,
	Report(reporting::Event),
	Gifs {
		request: u64,
		result: Result<model::GifPage, auth::Failure>,
	},
	GifFavorites {
		request: u64,
		result: Result<Vec<model::Gif>, auth::Failure>,
	},
	GifFavoritesChanged,
	ReactionPreferences {
		request: u64,
		result: Result<Vec<String>, auth::Failure>,
	},
	ReactionPreferencesChanged,
	ReadState(read_state::Event),
	NotificationPreferences(notifications::Event),
	Reactions(reactions::Event),
	Profile {
		user: Id,
		guild: Option<Id>,
		request: u64,
		result: Result<Box<UserProfile>, auth::Failure>,
	},
	StreamPreview {
		guild: Id,
		channel: Id,
		user: Id,
		request: u64,
		result: Result<String, auth::Failure>,
	},
	BadgeSettings {
		request: u64,
		result: Result<model::badges::BadgeSettings, auth::Failure>,
	},
	ProfileEdited {
		user: Id,
		request: u64,
		result: Result<Box<UserProfile>, auth::Failure>,
	},
	ServerProfile(server_profile::Event),
	OwnedCosmetics(owned_cosmetics::Event),
	AccountOverview {
		user: Id,
		request: u64,
		result: Result<Box<model::account_settings::Overview>, auth::Failure>,
	},
	AccountConnections {
		user: Id,
		request: u64,
		result: Result<Box<model::account_settings::Connections>, auth::Failure>,
	},
	Voice(voice::Event),
	ChannelCreated(Channel),
	ChannelRestored(Channel),
	ChannelChanged(ChannelPatch),
	ThreadChanged {
		guild: Id,
		patch: ChannelPatch,
	},
	ThreadRemoved {
		guild: Id,
		id: Id,
	},
	ThreadsSync {
		guild: Id,
		parents: Option<Vec<Id>>,
		threads: Vec<Channel>,
		removed: Vec<Id>,
	},
	GuildChanged(GuildPatch),
	/// Presence of presentation fields in the current authoritative READY snapshot.
	GuildDisplayMetadata(Vec<GuildPatch>),
	ServerIncident {
		guild: Id,
		incident: Option<model::server_access::Incident>,
	},
	GuildApplicationChanged {
		guild: Id,
		request: Id,
		deleted: bool,
	},
	GuildEmojis {
		guild: Id,
		emojis: Vec<CustomEmoji>,
	},
	Members(MemberList),
	DirectPresence(Vec<presence::Update>),
	MemberPresence {
		guild: Id,
		channel: Id,
		request: u64,
		updates: Vec<MemberPresence>,
	},
	RecipientAdded {
		channel: Id,
		user: User,
	},
	RecipientRemoved {
		channel: Id,
		user: Id,
	},
	Ready {
		permissions: model::permissions::Snapshot,
		user: User,
		guilds: Vec<Guild>,
		channels: Vec<Channel>,
	},
	History {
		channel: Id,
		request: u64,
		older: bool,
		messages: Vec<Message>,
	},
	HistoryFailed {
		channel: Id,
		request: u64,
		failure: auth::Failure,
	},
	ReplyPreview(reply_previews::ResultEvent),
	Message(Message),
	Patch(MessagePatch),
	Delete {
		channel: Id,
		id: Id,
	},
	DeleteBulk {
		channel: Id,
		ids: Vec<Id>,
	},
	/// A pin or unpin request finished; `Err` carries the service failure label.
	Edited {
		request: u64,
		channel: Id,
		message: Id,
		result: Result<Message, auth::Failure>,
	},
	Pinned {
		request: u64,
		channel: Id,
		message: Id,
		pinned: bool,
		result: Result<(), auth::Failure>,
	},
	SendResult {
		nonce: String,
		result: Result<Message, auth::Failure>,
	},
	/// Discord rate-limited this message (documented: a 429 request is not processed). The
	/// writer resends it after the service's wait, so it stays pending like on Discord.
	SendThrottled {
		nonce: String,
	},
	/// The service asked the user to solve a captcha before accepting this text message.
	SendChallenge {
		nonce: String,
		reply: Option<Reply>,
		challenge: Box<captcha::Challenge>,
	},
	Failure(auth::Failure),
	Disconnected,
	Resumed,
	Resync,
	Unavailable(Id),
	PermissionsChanged,
	Permissions(permissions::Event),
	/// Returns one already-bounded transport command to the existing rejection path.
	CommandRejected(Box<crate::Command>),
	/// A queued query remains pending until this service cooldown expires.
	MemberSearchCooldown {
		request: member_search::Request,
		seconds: u32,
	},
}

pub struct Envelope {
	pub generation: u64,
	pub event: Event,
}

impl Event {
	pub fn ready_navigation(&self) -> Option<(&User, &[Guild], &[Channel])> {
		match self {
			Self::Ready {
				user,
				guilds,
				channels,
				..
			} => Some((user, guilds, channels)),
			Self::Startup(startup) => Some((&startup.user, &startup.guilds, &startup.channels)),
			_ => None,
		}
	}
	/// Events that can change channel scope or permission inputs.
	pub fn changes_access(&self) -> bool {
		matches!(
			self,
			Event::Ready { .. }
				| Event::Startup(_)
				| Event::GroupAction(group_actions::Event::Written {
					result: Ok(None),
					..
				}) | Event::UserAction(user_actions::Event::Written {
				action: user_actions::Action::CloseDm(_)
					| user_actions::Action::MessageRequest { accept: false, .. },
				result: Ok(()),
				..
			}) | Event::ServerAction(server_actions::Event::Written {
				action: server_actions::Action::Leave(_) | server_actions::Action::Delete(_),
				result: Ok(None),
				..
			}) | Event::ServerAdmin(server_admin::Event {
				result: Ok(model::server_admin::Result::Roles(
					model::server_roles::Result::Catalog { .. }
				) | model::server_admin::Result::Member(_)),
				..
			}) | Event::ChannelAction(channel_actions::Event::Finished {
				result: Ok(
					channel_actions::Outcome::Channel { .. } | channel_actions::Outcome::Deleted
				),
				..
			}) | Event::Permissions(_)
				| Event::GuildRemoved(_)
				| Event::Resync
				| Event::PermissionsChanged
				| Event::Unavailable(_)
				| Event::ChannelCreated(_)
				| Event::ChannelRestored(_)
				| Event::ChannelChanged(_)
				| Event::ThreadChanged { .. }
				| Event::ThreadRemoved { .. }
				| Event::ThreadsSync { .. }
				| Event::RecipientAdded { .. }
				| Event::RecipientRemoved { .. }
		)
	}
	/// Session or membership changes which retire cached guild expression hints.
	/// Channel and thread access is checked again when hints are viewed or loaded.
	pub fn invalidates_expression_catalogs(&self) -> bool {
		matches!(
			self,
			Event::Ready { .. }
				| Event::Startup(_)
				| Event::Disconnected
				| Event::Resync
				| Event::PermissionsChanged
				| Event::Permissions(_)
				| Event::GuildRemoved(_)
				| Event::ServerAction(server_actions::Event::Written {
					action: server_actions::Action::Leave(_) | server_actions::Action::Delete(_),
					result: Ok(None),
					..
				})
		)
	}

	/// Admission estimate for heap-owned fields; rejects oversized items before entering the UI queue.
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ match self {
				// Admitted desktop commands are at most 350 KiB; charge a conservative
				// MiB including the boxed command instead of undercounting its payload.
				Self::CommandRejected(_) => 1024 * 1024,
				Self::Mfa(event) => event.bytes(),
				Self::Report(event) => event.result.as_ref().map_or(0, reporting::Outcome::bytes),
				Self::Interaction(event) => event.bytes(),
				Self::Autocomplete(event) => event.bytes(),
				Self::ApplicationCommands { result, .. } => result.as_ref().map_or(0, |commands| {
					commands.capacity() * size_of::<model::application_commands::Command>()
						+ commands
							.iter()
							.map(|command| {
								command.bytes() - size_of::<model::application_commands::Command>()
							})
							.sum::<usize>()
				}),
				Self::Startup(startup) => startup.bytes(),
				Self::MessagingPermissions { result, .. } => result
					.as_ref()
					.map_or(0, model::messaging_permissions::Snapshot::bytes),
				Self::InviteChallenge { challenge, .. } => challenge.bytes(),
				Self::GroupAction(group_actions::Event::Written {
					result: Ok(Some(patch)),
					..
				}) => [&patch.name, &patch.icon]
					.into_iter()
					.map(|p| match p {
						Patch::Value(s) => s.capacity(),
						_ => 0,
					})
					.sum(),
				Self::GroupAction(group_actions::Event::MembersLoaded { result, .. }) => {
					result.as_ref().map_or(0, group_actions::Members::bytes)
				}
				Self::GroupAction(group_actions::Event::Created { result, .. }) => result
					.as_ref()
					.map_or(0, |(channel, members)| channel.bytes() + members.bytes()),
				Self::ChannelAction(channel_actions::Event::Finished { result, .. }) => {
					result.as_ref().map_or(0, channel_actions::Outcome::bytes)
				}
				Self::Edited { result, .. } => result.as_ref().map_or(0, Message::bytes),
				Self::ServerSettings(event) => {
					event.result.as_ref().map_or(0, |value| {
						size_of::<model::server_settings::Settings>() + value.heap_bytes()
					}) + event.refreshed.as_ref().map_or(0, |value| {
						size_of::<model::server_settings::Settings>() + value.heap_bytes()
					})
				}
				Self::MemberGuide(event) => event
					.result
					.as_ref()
					.map_or(0, model::member_guide::Snapshot::bytes),
				Self::Onboarding(event) => event.bytes(),
				Self::Community(event) => event
					.result
					.as_ref()
					.map_or(0, model::community::Outcome::bytes),
				Self::BulkModeration { report, .. } => report.bytes(),
				Self::MemberReviewProgress(progress) => progress.review.bytes(),
				Self::ServerAdmin(event) => event
					.result
					.as_ref()
					.map_or(0, model::server_admin::Result::bytes),
				Self::GuildFolders(result) => result
					.as_ref()
					.map_or(0, model::guild_folders::Settings::heap_bytes),
				Self::ServerAction(server_actions::Event::Written {
					result: Ok(Some(code)),
					..
				}) => code.capacity(),
				Self::ServerAction(server_actions::Event::InviteSent {
					result: Ok(sent), ..
				}) => sent.0.bytes() + sent.1.bytes(),
				Self::UserAction(user_actions::Event::Friends(entries)) => {
					entries.as_ref().map_or(0, |entries| {
						entries.capacity() * size_of::<(User, String)>()
							+ entries
								.iter()
								.map(|(u, n)| u.heap_bytes() + n.capacity())
								.sum::<usize>()
					})
				}
				Self::UserAction(user_actions::Event::DmOpened { result, .. }) => {
					result.as_ref().map_or(0, |channel| channel.bytes())
				}
				Self::UserAction(user_actions::Event::Requests(entries)) => {
					entries.as_ref().map_or(0, |e| {
						e.capacity() * size_of::<(User, String, bool)>()
							+ e.iter()
								.map(|(u, n, _)| u.heap_bytes() + n.capacity())
								.sum::<usize>()
					})
				}
				Self::UserAction(user_actions::Event::Request { profile, .. }) => profile
					.as_ref()
					.map_or(0, |(u, n)| u.heap_bytes() + n.capacity()),
				Self::UserAction(user_actions::Event::Written {
					action: user_actions::Action::AddFriend { username },
					..
				}) => username.capacity(),
				Self::UserAction(user_actions::Event::Written {
					action:
						user_actions::Action::Note { text, .. }
						| user_actions::Action::Nickname { text, .. },
					..
				}) => text.capacity(),
				Self::UserAction(user_actions::Event::NoteChanged { text, .. }) => text.capacity(),
				Self::UserAction(user_actions::Event::NoteLoaded { result, .. }) => {
					result.as_ref().map_or(0, String::capacity)
				}
				Self::UserAction(user_actions::Event::Nickname { text, .. }) => text.capacity(),
				Self::UserAction(user_actions::Event::Nicknames(entries)) => {
					entries.capacity() * size_of::<(Id, String)>()
						+ entries
							.iter()
							.map(|(_, text)| text.capacity())
							.sum::<usize>()
				}
				Self::UserAction(user_actions::Event::Friend { profile, .. }) => profile
					.as_ref()
					.map_or(0, |(u, n)| u.heap_bytes() + n.capacity()),
				Self::UserAction(user_actions::Event::FriendProfile((u, n))) => {
					u.heap_bytes() + n.capacity()
				}
				Self::UserAction(user_actions::Event::Challenge { challenge, .. }) => {
					challenge.bytes()
				}
				Self::UserAction(user_actions::Event::Relationships(entries)) => entries
					.as_ref()
					.map_or(0, |e| e.capacity() * size_of::<(Id, bool)>()),
				Self::UserAction(user_actions::Event::MessageRequests(entries)) => entries
					.as_ref()
					.map_or(0, |e| e.capacity() * size_of::<Id>()),
				Self::UserAction(user_actions::Event::MessageRequest { .. }) => size_of::<Id>(),
				Self::UserAction(user_actions::Event::MessageSpams(entries)) => entries
					.as_ref()
					.map_or(0, |e| e.capacity() * size_of::<Id>()),
				Self::UserAction(user_actions::Event::MessageSpam { .. }) => size_of::<Id>(),
				Self::UserAction(user_actions::Event::RequestSpams(entries)) => entries
					.as_ref()
					.map_or(0, |e| e.capacity() * size_of::<Id>()),
				Self::UserAction(user_actions::Event::RequestSpam { .. }) => size_of::<Id>(),
				Self::Archives { result, .. } => {
					result.as_ref().map_or(0, model::archives::Page::bytes)
				}
				Self::ThreadStarter { result, .. } => result.as_ref().map_or(0, Message::bytes),
				Self::ForumSummaries { results, .. } => {
					results.capacity()
						* size_of::<(Id, Result<model::forum::Summary, auth::Failure>)>()
						+ results
							.iter()
							.map(|(_, result)| {
								result.as_ref().map_or(0, model::forum::Summary::bytes)
							})
							.sum::<usize>()
				}
				Self::ForumPosts { result, .. } => {
					result.as_ref().map_or(0, model::forum::Page::bytes)
				}
				Self::PostCreated { result, .. } => result.as_ref().map_or(0, Channel::bytes),
				Self::InboxAction {
					result: Ok(outcome),
					..
				} => outcome.bytes(),
				Self::Mentions {
					result: Ok(messages),
					..
				} => {
					messages.capacity() * size_of::<Message>()
						+ messages.iter().map(Message::bytes).sum::<usize>()
				}
				Self::Search {
					result: Ok(search::Outcome::Page(page) | search::Outcome::Pins(page)),
					..
				} => page.bytes(),
				Self::Gifs {
					result: Ok(page), ..
				} => page.bytes(),
				Self::ReactionPreferences {
					result: Ok(keys), ..
				} => {
					keys.capacity() * size_of::<String>()
						+ keys.iter().map(String::capacity).sum::<usize>()
				}
				Self::GifFavorites {
					result: Ok(favorites),
					..
				} => {
					favorites.capacity() * size_of::<model::Gif>()
						+ favorites
							.iter()
							.map(|gif| gif.bytes() - size_of::<model::Gif>())
							.sum::<usize>()
				}
				Self::ReadState(read_state::Event::Snapshot { entries, .. }) => entries
					.as_ref()
					.map_or(0, |e| e.capacity() * size_of::<(Id, Option<Id>, u32)>()),
				Self::ChannelPreferences(event) => {
					event.result.as_ref().map_or(0, |update| update.bytes())
				}
				Self::NotificationPreferences(event) => event.bytes(),
				Self::ReadState(read_state::Event::Latest(entries)) => {
					entries.capacity() * size_of::<(Id, Patch<Id>)>()
				}
				Self::Reactions(
					reactions::Event::Delta { emoji, .. }
					| reactions::Event::Cleared {
						emoji: Some(emoji), ..
					},
				) => emoji.name.as_ref().map_or(0, String::capacity),
				Self::Reactions(reactions::Event::Read { result, .. }) => {
					result.as_ref().map_or(0, |r| model::reaction_bytes(r))
				}
				Self::GuildJoined(guild) => guild.bytes(),
				Self::Invite { code, result } => {
					code.capacity() + result.as_ref().map_or(0, |embed| embed.bytes())
				}
				Self::Profile { result, .. } | Self::ProfileEdited { result, .. } => {
					result.as_ref().map_or(0, |p| p.bytes())
				}
				Self::ServerProfile(event) => {
					event.result.as_ref().map_or(0, |snapshot| snapshot.bytes())
				}
				Self::OwnedCosmetics(event) => {
					event.result.as_ref().map_or(0, |snapshot| snapshot.bytes())
				}
				Self::AccountOverview { result, .. } => {
					result.as_ref().map_or(0, |overview| overview.bytes())
				}
				Self::AccountConnections { result, .. } => {
					result.as_ref().map_or(0, |connections| connections.bytes())
				}
				Self::StreamPreview { result, .. } => result.as_ref().map_or(0, String::capacity),
				Self::BadgeSettings { result, .. } => result
					.as_ref()
					.map_or(0, |s| (s.order.capacity() + s.hidden.capacity()) * 4),
				Self::Voice(event) => event.bytes(),
				Self::Permissions(event) => event.bytes(),
				Self::GuildEmojis { emojis, .. } => custom_emoji_bytes(emojis),
				Self::GuildStickers { stickers, .. } => sticker_bytes(stickers),
				Self::Soundboard(event) => event.bytes(),
				Self::StickerPacks(result) => result.as_ref().map_or(0, stickers::pack_bytes),
				Self::Sticker { result, .. } => result.as_ref().map_or(0, Sticker::heap_bytes),
				Self::GuildChanged(patch) => [&patch.name, &patch.icon]
					.into_iter()
					.map(|value| match value {
						Patch::Value(value) => value.capacity(),
						_ => 0,
					})
					.sum(),
				Self::GuildDisplayMetadata(patches) => {
					patches.capacity() * size_of::<GuildPatch>()
						+ patches
							.iter()
							.flat_map(|p| [&p.name, &p.icon])
							.map(|p| match p {
								Patch::Value(s) => s.capacity(),
								_ => 0,
							})
							.sum::<usize>()
				}
				Self::ChannelCreated(channel) | Self::ChannelRestored(channel) => channel.bytes(),
				Self::ThreadsSync {
					parents,
					threads,
					removed,
					..
				} => {
					removed.capacity() * size_of::<Id>()
						+ parents
							.as_ref()
							.map_or(0, |p| p.capacity() * size_of::<Id>())
						+ threads.capacity().saturating_sub(threads.len()) * size_of::<Channel>()
						+ threads.iter().map(Channel::bytes).sum::<usize>()
				}
				Self::ChannelChanged(patch) | Self::ThreadChanged { patch, .. } => {
					[&patch.name, &patch.icon]
						.into_iter()
						.map(|p| match p {
							Patch::Value(s) => s.capacity(),
							_ => 0,
						})
						.sum()
				}
				Self::Ready {
					user,
					guilds,
					channels,
					permissions,
				} => {
					permissions.bytes()
						+ user.heap_bytes()
						+ guilds.iter().map(Guild::bytes).sum::<usize>()
						+ channels.iter().map(Channel::bytes).sum::<usize>()
				}
				Self::MemberPresence { updates, .. } => {
					updates.capacity() * size_of::<MemberPresence>()
						+ updates
							.iter()
							.map(MemberPresence::heap_bytes)
							.sum::<usize>()
				}
				Self::DirectPresence(updates) => {
					updates.capacity() * size_of::<presence::Update>()
						+ updates
							.iter()
							.map(presence::Update::heap_bytes)
							.sum::<usize>()
				}
				Self::MemberIdentity(update) => update.bytes(),
				Self::Typing(signal) => signal.member.as_ref().map_or(0, |member| member.bytes()),
				Self::MemberSearchCooldown { request, .. } => {
					request.query.capacity() + request.users.capacity() * size_of::<Id>()
				}
				Self::MemberSearch { request, result } => {
					request.query.capacity()
						+ request.users.capacity() * size_of::<Id>()
						+ result.as_ref().map_or(0, |rows| {
							rows.capacity() * size_of::<Member>()
								+ rows.iter().map(Member::bytes).sum::<usize>()
						})
				}
				Self::MemberSearchPartial {
					request,
					rows,
					missing,
				} => {
					request.query.capacity()
						+ request.users.capacity() * size_of::<Id>()
						+ rows.capacity() * size_of::<Member>()
						+ rows.iter().map(Member::bytes).sum::<usize>()
						+ missing.capacity() * size_of::<model::MissingMemberFields>()
				}
				Self::Members(list) => {
					list.slots.capacity() * size_of::<Option<MemberSlot>>()
						+ list.slot_bytes()
						+ list
							.groups
							.iter()
							.map(|(id, _)| id.capacity())
							.sum::<usize>()
				}
				Self::RecipientAdded { user, .. } => user.heap_bytes(),
				Self::History { messages, .. } => messages.iter().map(Message::bytes).sum(),
				Self::ReplyPreview(event) => event
					.result
					.as_ref()
					.ok()
					.and_then(Option::as_ref)
					.map_or(0, |preview| preview.bytes()),
				Self::Polls(polls::Event::Result { result, .. }) => {
					result.as_ref().map_or(0, |message| message.bytes())
				}
				Self::Message(m) => m.bytes(),
				Self::Patch(p) => {
					let content = match &p.content {
						Patch::Value(s) => s.capacity(),
						_ => 0,
					};
					content
						+ match &p.poll {
							Patch::Value(Some(poll)) => poll.bytes(),
							_ => 0,
						} + match &p.sticker_items {
						Patch::Value(stickers) => model::sticker_bytes(stickers),
						_ => 0,
					} + match &p.reactions {
						Patch::Value(r) => model::reaction_bytes(r),
						_ => 0,
					} + match &p.mentions {
						Patch::Value(users) => model::mention_bytes(users),
						_ => 0,
					} + match &p.attachments {
						Patch::Value(attachments) => model::attachment_bytes(attachments),
						_ => 0,
					} + match &p.embeds {
						Patch::Value(embeds) => model::embed_bytes(embeds),
						_ => 0,
					}
				}
				Self::DeleteBulk { ids, .. } => ids.capacity() * size_of::<Id>(),
				Self::SendResult { nonce, result } => {
					nonce.capacity() + result.as_ref().map_or(0, Message::bytes)
				}
				Self::SendChallenge {
					nonce, challenge, ..
				} => nonce.capacity() + challenge.bytes(),
				Self::SendThrottled { nonce } => nonce.capacity(),
				_ => 0,
			}
	}
}
