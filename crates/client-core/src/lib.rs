//! Single UI-thread state owner. Adapters deliver generation-tagged typed events.
pub mod archives;
pub mod auth;
pub mod captcha;
pub mod channel_actions;
pub mod fingerprint;
pub mod forum;
pub mod gifs;
pub mod guild_folders;
pub mod permissions;
#[cfg(test)]
mod permissions_tests;

pub mod invites;
pub mod message_actions;
pub mod messaging_permissions;
pub mod notification_settings;
pub mod notifications;
pub mod presence;
pub mod profile;
pub mod reactions;
pub mod read_state;
mod replies;
pub use replies::ReplyDeletions;
pub mod group_actions;
pub mod resident;
pub mod screen;
pub mod search;
pub mod server_actions;
pub mod server_admin;
pub mod server_audit_log;
pub mod server_integrations;
pub mod server_roles;
pub mod server_settings;
mod threads;
pub mod typing;
pub mod user_actions;
pub mod voice;
use model::*;
use session_cache::Timeline;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_DRAFT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CONTENT: usize = 2000;
/// Discord accepts at most ten attachments per message.
pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_NAV: usize = model::account::MAX_ENTRIES;
pub const MAX_MEMBER_PRESENCE_BYTES: usize = 128 * 1024;
pub const MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;
pub const EVENT_SLOTS: usize = 8; // UI drain batch; reliable events share a 32 MiB byte budget.
pub const COMMAND_SLOTS: usize = 16; // ordinary commands <=16 KiB; bulk DM settings <=33 KiB; channel edit <=128 KiB; group icon <=350 KiB

pub enum Command {
	MessagingPermissions {
		request: u64,
		change: Option<model::messaging_permissions::Change>,
	},
	AccountNotificationSettings {
		request: u64,
		section: model::notification_settings::Section,
		change: Option<model::notification_settings::Change>,
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
	SendServerInvite {
		guild: Id,
		user: Id,
		code: String,
		nonce: String,
		request: u64,
	},
	/// None loads the current settings; Some saves the complete folder layout.
	GuildFolders(Option<model::guild_folders::Settings>),
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
	},
	JoinInvite {
		code: String,
		request: u64,
		captcha: Option<Box<captcha::Retry>>,
	},
	Invite {
		code: String,
	},
	CreatePost {
		parent: Id,
		guild: Id,
		title: String,
		content: String,
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
		request: u64,
	},
	CancelSearch,
	Gifs {
		query: Option<String>,
		request: u64,
	},
	CancelGifs,
	MarkRead {
		channel: Id,
		message: Id,
		request: u64,
	},
	Reactions(reactions::Command),
	Profile {
		user: Id,
		guild: Option<Id>,
		request: u64,
	},
	CancelProfile,
	/// None loads the account's global profile; Some writes only explicitly changed fields.
	EditProfile {
		user: Id,
		request: u64,
		changes: Option<model::ProfileEdit>,
	},
	Voice(voice::Command),
	Members {
		guild: Option<Id>,
		channel: Option<Id>,
		request: u64,
		list_id: Option<String>,
	},
	History {
		channel: Id,
		before: Option<Id>,
		after: Option<Id>,
		request: u64,
	},
	Send {
		channel: Id,
		content: String,
		nonce: String,
		reply: Option<Id>,
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
pub struct Startup {
	pub user: User,
	pub guilds: Vec<Guild>,
	pub channels: Vec<Channel>,
	pub permissions: model::permissions::Snapshot,
	pub read_state: read_state::Event,
	pub notifications: Option<notifications::Event>,
	pub session_dnd: Option<bool>,
	pub warnings: model::account::Warnings,
}
impl Startup {
	/// Validate and build the permission mirror on the Gateway worker before enqueueing.
	pub fn prepare(mut self) -> Result<PreparedStartup, auth::Failure> {
		if self.bytes() > model::account::MAX_BYTES {
			return Err(auth::Failure::ProtocolAt(
				"Account startup exceeds safe capacity",
			));
		}
		let spare_bytes = (self.guilds.capacity() - self.guilds.len()) * size_of::<Guild>()
			+ (self.channels.capacity() - self.channels.len()) * size_of::<Channel>();
		let permissions = prepare_navigation(
			&self.user,
			&self.guilds,
			&self.channels,
			std::mem::take(&mut self.permissions),
			spare_bytes,
		)
		.map_err(auth::Failure::ProtocolAt)?;
		let bytes = self.bytes() + permissions.bytes() + size_of::<permissions::Permissions>();
		if bytes > model::account::MAX_BYTES {
			return Err(auth::Failure::ProtocolAt(
				"Account startup exceeds safe capacity",
			));
		}
		Ok(PreparedStartup {
			data: self,
			permissions,
			bytes,
		})
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.user.heap_bytes()
			+ self.permissions.bytes()
			+ self.guilds.capacity().saturating_sub(self.guilds.len()) * size_of::<Guild>()
			+ self.channels.capacity().saturating_sub(self.channels.len()) * size_of::<Channel>()
			+ self.guilds.iter().map(Guild::bytes).sum::<usize>()
			+ self.channels.iter().map(Channel::bytes).sum::<usize>()
			+ self.read_state.bytes()
			+ self
				.notifications
				.as_ref()
				.map_or(0, notifications::Event::bytes)
	}
}
pub struct PreparedStartup {
	data: Startup,
	permissions: permissions::Permissions,
	bytes: usize,
}
impl PreparedStartup {
	pub fn data(&self) -> &Startup {
		&self.data
	}
	pub fn permission_state(&self) -> &permissions::Permissions {
		&self.permissions
	}
	pub fn bytes(&self) -> usize {
		self.bytes
	}
}
impl std::ops::Deref for PreparedStartup {
	type Target = Startup;
	fn deref(&self) -> &Startup {
		&self.data
	}
}
fn prepare_navigation(
	user: &User,
	guilds: &[Guild],
	channels: &[Channel],
	permissions: model::permissions::Snapshot,
	spare_bytes: usize,
) -> Result<permissions::Permissions, &'static str> {
	if channels.len() + guilds.len() > MAX_NAV
		|| channels.iter().any(|c| c.recipients.len() > 64)
		|| guilds
			.iter()
			.any(|g| g.emojis.as_ref().is_some_and(|e| !valid_custom_emojis(e)))
		|| guilds.iter().map(Guild::bytes).sum::<usize>()
			+ channels.iter().map(Channel::bytes).sum::<usize>()
			+ permissions.bytes()
			+ spare_bytes
			> model::account::MAX_BYTES
	{
		return Err("Account navigation exceeds safe capacity");
	}
	let guild_ids: BTreeSet<_> = guilds.iter().map(|guild| guild.id).collect();
	let channel_ids: BTreeSet<_> = channels.iter().map(|channel| channel.id).collect();
	if user.id.0 == 0
		|| guild_ids.len() != guilds.len()
		|| channel_ids.len() != channels.len()
		|| guild_ids.contains(&Id(0))
		|| channel_ids.contains(&Id(0))
		|| channels.iter().any(|channel| {
			channel
				.guild
				.is_some_and(|guild| !guild_ids.contains(&guild))
		}) {
		return Err("Invalid account navigation");
	}
	let mut permission_state = permissions::Permissions::default();
	permission_state.replace(permissions)?;
	if guilds.iter().map(Guild::bytes).sum::<usize>()
		+ channels.iter().map(Channel::bytes).sum::<usize>()
		+ permission_state.bytes()
		+ spare_bytes
		> model::account::MAX_BYTES
	{
		return Err("Account navigation exceeds safe capacity");
	}
	Ok(permission_state)
}
pub enum Event {
	Startup(Box<PreparedStartup>),
	StartupWarnings(model::account::Warnings),
	MessagingPermissions {
		request: u64,
		result: Result<model::messaging_permissions::Snapshot, auth::Failure>,
	},
	SocialNotification(model::notification_settings::SocialNotification),
	AccountNotificationSettings {
		request: u64,
		result: Result<model::notification_settings::Snapshot, auth::Failure>,
	},
	InviteChallenge {
		request: u64,
		challenge: Box<captcha::Challenge>,
	},
	ChannelAction(channel_actions::Event),
	ServerAdmin(server_admin::Event),
	ServerSettings(server_settings::Event),
	JoinInvite {
		request: u64,
		result: Result<Id, auth::Failure>,
	},
	GuildJoined(Guild),
	GuildFolders(Result<model::guild_folders::Settings, auth::Failure>),
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
	Gifs {
		request: u64,
		result: Result<model::GifPage, auth::Failure>,
	},
	ReadState(read_state::Event),
	NotificationPreferences(notifications::Event),
	Reactions(reactions::Event),
	Profile {
		user: Id,
		guild: Option<Id>,
		request: u64,
		result: Result<Box<UserProfile>, auth::Failure>,
	},
	ProfileEdited {
		user: Id,
		request: u64,
		result: Result<Box<UserProfile>, auth::Failure>,
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
	Failure(auth::Failure),
	Disconnected,
	Resumed,
	Resync,
	Unavailable(Id),
	PermissionsChanged,
	Permissions(permissions::Event),
}
pub struct Envelope {
	pub generation: u64,
	pub event: Event,
}
pub struct Pending {
	pub channel: Id,
	pub content: String,
	/// Filenames of the files uploaded with this message, in send order.
	pub attachments: Vec<String>,
	pub nonce: String,
	pub delivery: Delivery,
	pub confirmed: Option<Id>,
}
#[derive(Default)]
pub struct NavigationIndex {
	bytes: std::cell::Cell<Option<(usize, usize, usize)>>,
	channels: std::cell::RefCell<BTreeMap<Id, usize>>,
	channel_stamp: std::cell::Cell<Option<(usize, usize)>>,
	guilds: std::cell::RefCell<BTreeMap<Id, usize>>,
}

pub struct State {
	pub messaging_permissions: messaging_permissions::Settings,
	pub notification_settings: notification_settings::Settings,
	pub guild_folders: Option<model::guild_folders::Settings>,
	pub folders_pending: bool,
	pub folders_error: Option<&'static str>,
	pub user_actions: user_actions::Actions,
	pub server_actions: server_actions::Actions,
	pub channel_actions: channel_actions::Actions,
	pub server_settings: server_settings::Editor,
	pub server_admin: server_admin::View,
	pub server_members_shortcuts: BTreeMap<Id, bool>,
	pub group_actions: group_actions::Actions,
	pub typing: typing::Typing,
	pub permissions: permissions::Permissions,
	pub archives: Option<archives::View>,
	pub posting: forum::Posting,
	pub posts: forum::Posts,
	pub archived_thread: Option<Id>,
	pub search: Option<search::SearchView>,
	pub search_request: u64,
	pub gifs: gifs::Gifs,
	/// A pin changed in this channel; the pins view should be reloaded once.
	pub pins_changed: Option<Id>,
	pub message_actions: message_actions::MessageActions,
	pub search_target: Option<Id>,
	/// The active range was fetched around a target, independently of its consumed scroll cue.
	pub history_targeted: bool,
	pub reply_deletions: ReplyDeletions,
	pub read_state: read_state::ReadState,
	pub startup_warnings: model::account::Warnings,
	pub notification_preferences: notifications::Preferences,
	pub reactions: reactions::Reactions,
	pub profile: Option<profile::ProfileView>,
	pub profile_request: u64,
	pub profile_cache: profile::ProfileCache,
	pub own_profile: profile::OwnProfile,
	pub invites: invites::Cache,
	pub invite_join: invites::Join,
	pub voice: voice::State,
	pub generation: u64,
	pub auth: auth::AuthState,
	pub user: Option<User>,
	pub members: Option<MemberList>,
	pub direct_presences: Vec<MemberPresence>,
	pub local_game_activity: Option<model::RichActivity>,
	#[doc(hidden)]
	pub direct_presence_bytes: Option<(usize, usize)>,
	#[doc(hidden)]
	pub direct_presence_epoch: u64,
	pub member_request: u64,
	pub guilds: Vec<Guild>,
	pub channels: Vec<Channel>,
	#[doc(hidden)]
	pub navigation_index: NavigationIndex,
	pub selected: Option<Id>,
	pub timeline: Timeline,
	pub preserve_deleted_messages: bool,
	pub resident: resident::Windows,
	pub freshness: Freshness,
	pub status: &'static str,
	pub drafts: BTreeMap<Id, String>,
	pub pending: Vec<Pending>,
	pub reply: Option<Id>,
	pub send_sequence: u64,
	pub request: u64,
	pub history_before: Option<Id>,
	pub history_after: Option<Id>,
	pub newer_cursor: Option<Id>,
	pub newer_may_have_more: bool,
	pub history_pending: bool,
	pub older_exhausted: bool,
	pub gateway_connected: bool,
	pub revision: u64,
	pub demo: bool,
}
impl Default for State {
	fn default() -> Self {
		Self {
			messaging_permissions: Default::default(),
			notification_settings: Default::default(),
			guild_folders: None,
			folders_pending: false,
			folders_error: None,
			user_actions: user_actions::Actions::default(),
			server_actions: server_actions::Actions::default(),
			channel_actions: channel_actions::Actions::default(),
			server_settings: server_settings::Editor::default(),
			server_admin: server_admin::View::default(),
			server_members_shortcuts: BTreeMap::new(),
			group_actions: group_actions::Actions::default(),
			typing: typing::Typing::default(),
			permissions: permissions::Permissions::default(),
			archives: None,
			posting: forum::Posting::default(),
			posts: forum::Posts::default(),
			archived_thread: None,
			search: None,
			search_request: 0,
			gifs: gifs::Gifs::default(),
			pins_changed: None,
			message_actions: Default::default(),
			search_target: None,
			history_targeted: false,
			reply_deletions: ReplyDeletions::default(),
			read_state: read_state::ReadState::default(),
			startup_warnings: Default::default(),
			notification_preferences: notifications::Preferences::default(),
			reactions: reactions::Reactions::default(),
			profile: None,
			profile_request: 0,
			profile_cache: Default::default(),
			own_profile: Default::default(),
			invites: Default::default(),
			invite_join: Default::default(),
			voice: voice::State::default(),
			generation: 1,
			auth: auth::AuthState::Unauthenticated,
			user: None,
			members: None,
			direct_presences: vec![],
			local_game_activity: Default::default(),
			direct_presence_bytes: None,
			direct_presence_epoch: 0,
			member_request: 0,
			guilds: vec![],
			channels: vec![],
			navigation_index: NavigationIndex::default(),
			selected: None,
			timeline: Timeline::default(),
			preserve_deleted_messages: false,
			resident: resident::Windows::default(),
			freshness: Freshness::Stale,
			status: "Disconnected",
			drafts: BTreeMap::new(),
			pending: vec![],
			reply: None,
			send_sequence: 0,
			request: 0,
			history_before: None,
			history_after: None,
			newer_cursor: None,
			newer_may_have_more: false,
			history_pending: false,
			older_exhausted: false,
			gateway_connected: false,
			revision: 0,
			demo: false,
		}
	}
}
/// Channels the conversation pane can present: text, voice, and forum containers.
fn navigable(channel: &Channel) -> bool {
	channel.supports_text()
		|| channel.kind == 2
		|| (channel.guild.is_some() && matches!(channel.kind, 15 | 16))
}
impl State {
	pub(crate) fn navigation_bytes(&self) -> usize {
		if let Some((channels, guilds, bytes)) = self.navigation_index.bytes.get()
			&& channels == self.channels.len()
			&& guilds == self.guilds.len()
		{
			return bytes;
		}
		let bytes = self.channels.iter().map(Channel::bytes).sum::<usize>()
			+ self.guilds.iter().map(Guild::bytes).sum::<usize>()
			+ (self.channels.capacity() - self.channels.len()) * size_of::<Channel>()
			+ (self.guilds.capacity() - self.guilds.len()) * size_of::<Guild>();
		self.set_navigation_bytes(bytes);
		bytes
	}
	fn set_navigation_bytes(&self, bytes: usize) {
		self.navigation_index
			.bytes
			.set(Some((self.channels.len(), self.guilds.len(), bytes)));
	}

	fn channel_stamp(&self) -> (usize, usize) {
		(self.channels.as_ptr() as usize, self.channels.len())
	}
	fn channel_index(&self, id: Id) -> Option<usize> {
		let stamp = self.channel_stamp();
		if self.navigation_index.channel_stamp.get() == Some(stamp) {
			let cached = self.navigation_index.channels.borrow().get(&id).copied();
			if cached.is_none_or(|index| {
				self.channels
					.get(index)
					.is_some_and(|channel| channel.id == id)
			}) {
				return cached;
			}
		}
		let mut index = self.navigation_index.channels.borrow_mut();
		*index = self
			.channels
			.iter()
			.take(MAX_NAV)
			.enumerate()
			.map(|(index, channel)| (channel.id, index))
			.collect();
		self.navigation_index.channel_stamp.set(Some(stamp));
		index.get(&id).copied()
	}
	pub fn channel(&self, id: Id) -> Option<&Channel> {
		self.channel_index(id)
			.and_then(|index| self.channels.get(index))
	}
	/// Call after replacing IDs or payloads directly in synthetic navigation vectors.
	pub fn invalidate_navigation(&self) {
		self.navigation_index.channel_stamp.set(None);
		self.navigation_index.guilds.borrow_mut().clear();
		self.navigation_index.bytes.set(None);
	}

	pub fn guild(&self, id: Id) -> Option<&Guild> {
		let cached = self.navigation_index.guilds.borrow().get(&id).copied();
		if let Some(guild) = cached
			.and_then(|index| self.guilds.get(index))
			.filter(|guild| guild.id == id)
		{
			return Some(guild);
		}
		let mut index = self.navigation_index.guilds.borrow_mut();
		*index = self
			.guilds
			.iter()
			.take(MAX_NAV)
			.enumerate()
			.map(|(index, guild)| (guild.id, index))
			.collect();
		index.get(&id).and_then(|index| self.guilds.get(*index))
	}

	pub fn logout(&mut self) {
		let generation = self.generation.wrapping_add(1);
		*self = Self {
			generation,
			..Self::default()
		};
	}
	pub fn has_unsent(&self) -> bool {
		self.drafts.values().any(|s| !s.is_empty())
			|| self
				.pending
				.iter()
				.any(|p| p.delivery != Delivery::Confirmed)
	}
	pub fn draft_bytes(&self) -> usize {
		self.drafts.values().map(String::capacity).sum::<usize>()
			+ self
				.pending
				.iter()
				.map(|p| {
					p.content.capacity()
						+ p.nonce.capacity()
						+ p.attachments.iter().map(String::capacity).sum::<usize>()
						+ p.attachments.capacity() * size_of::<String>()
						+ size_of::<Pending>()
				})
				.sum::<usize>()
	}
	pub fn select(&mut self, channel: Id) -> Option<Command> {
		// Keep the current conversation intact, but allow a restored channel to load again.
		if self.selected == Some(channel) && self.freshness != Freshness::Unavailable {
			return None;
		}
		if !self.channel(channel).is_some_and(navigable) {
			self.status = "This channel kind is unsupported";
			return None;
		}
		if !self.can_view(channel) {
			self.status = "Channel permissions are unavailable or access was revoked";
			return None;
		}
		self.retire_archived_thread(Some(channel));
		self.typing.clear();
		self.select_resident(channel);
		self.history_targeted = false;
		self.members = None;
		self.selected = Some(channel);
		self.clear_search();
		self.search_target = None;
		self.reactions.reset();
		self.older_exhausted = false;
		self.reply = None;
		self.revision += 1;
		if self.channel(channel).is_some_and(|c| !c.supports_text()) {
			self.cancel_history();
			self.freshness = Freshness::Fresh;
			return None;
		}
		let command = self.history(None);
		Some(command)
	}
	pub fn request_members(&mut self) -> Option<Command> {
		let index = self.channel_index(self.selected?)?;
		let channel = &self.channels[index];
		self.member_request = self.member_request.wrapping_add(1);
		if !self.can_view(channel.id) {
			return None;
		}
		let list_id = self.member_list_id(channel);
		let rows = if channel.guild.is_none() && self.freshness != Freshness::Unavailable {
			let mut users = channel.recipients.clone();
			if let Some(user) = &self.user
				&& !users.iter().any(|u| u.id == user.id)
			{
				users.push(user.clone());
			}
			users
				.into_iter()
				.map(|user| {
					Some(Member {
						roles: vec![],
						nick: None,
						status: None,
						custom_status: None,
						activities: vec![],
						user,
					})
				})
				.collect()
		} else {
			vec![]
		};
		let freshness = if self.freshness == Freshness::Unavailable {
			Freshness::Unavailable
		} else if channel.guild.is_none() {
			Freshness::Fresh
		} else if list_id.is_none() {
			Freshness::Unavailable
		} else {
			Freshness::Loading
		};
		self.members = Some(MemberList {
			guild: channel.guild,
			channel: channel.id,
			request: self.member_request,
			total: rows.len() as u64,
			rows,
			freshness,
		});
		let command = Command::Members {
			guild: channel
				.guild
				.filter(|_| list_id.is_some() && self.freshness != Freshness::Unavailable),
			channel: Some(channel.id),
			request: self.member_request,
			list_id,
		};
		self.apply_direct_presence(&[]);
		Some(command)
	}
	pub fn close_members(&mut self) -> Command {
		self.member_request = self.member_request.wrapping_add(1);
		self.members = None;
		Command::Members {
			guild: None,
			channel: None,
			request: self.member_request,
			list_id: None,
		}
	}
	pub fn history(&mut self, before: Option<Id>) -> Command {
		self.history_range(before, None)
	}
	fn history_range(&mut self, before: Option<Id>, after: Option<Id>) -> Command {
		self.typing.clear();
		if before.is_none() {
			self.newer_cursor = None;
			self.newer_may_have_more = false;
		}
		if before.is_none() && after.is_none() {
			self.history_targeted = false;
		}
		let Some(channel) = self.selected.filter(|id| {
			self.channels
				.iter()
				.any(|channel| channel.id == *id && channel.supports_text())
		}) else {
			self.selected = None;
			self.search_target = None;
			self.reply = None;
			self.invalidate_members();
			self.timeline.clear();
			self.cancel_history();
			self.freshness = Freshness::Unavailable;
			self.status = "Conversation no longer available in navigation";
			return self.clear_search();
		};
		if !self.can_read_history(channel) {
			self.cancel_history();
			self.timeline.clear();
			self.reply = None;
			self.freshness = if self.can_view(channel) && self.gateway_connected {
				Freshness::Fresh
			} else {
				Freshness::Unavailable
			};
			self.status = "Message history is unavailable with the current permissions";
			return self.clear_search();
		}
		if self.search.as_ref().is_some_and(|s| s.loading)
			|| self.archives.as_ref().is_some_and(|s| s.loading)
		{
			self.clear_search();
		}
		if before.is_none() {
			self.search_target = None;
		}
		self.reactions.cancel_read();
		self.request += 1;
		self.history_before = before;
		self.history_after = after;
		self.history_pending = true;
		self.freshness = Freshness::Loading;
		self.timeline.begin_page(before.is_some());
		Command::History {
			channel,
			before,
			after,
			request: self.request,
		}
	}
	pub fn can_load_older(&self) -> bool {
		self.selected
			.is_some_and(|channel| self.can_read_history(channel))
			&& self.gateway_connected
			&& self.freshness == Freshness::Fresh
			&& !self.history_pending
			&& !self.older_exhausted
			&& self.timeline.row_count() != 0
	}
	pub fn older_history(&mut self) -> Option<Command> {
		if !self.can_load_older() {
			return None;
		}
		let before = self.timeline.row_ids().next()?;
		Some(self.history(Some(before)))
	}
	pub fn prepare_send(&mut self) -> Option<Command> {
		self.prepare_send_with_attachment(None)
	}
	pub fn prepare_send_with_attachment(&mut self, filename: Option<&str>) -> Option<Command> {
		self.prepare_send_with_attachments(filename.as_slice())
	}
	/// Queue the draft with up to ten attachment filenames; an empty slice sends text only.
	pub fn prepare_send_with_attachments(&mut self, filenames: &[&str]) -> Option<Command> {
		let channel = self.selected?;
		if !self.can_send(channel) || (!filenames.is_empty() && !self.can_attach(channel)) {
			self.status = "Sending is unavailable with the current connection or permissions";
			return None;
		}
		if filenames.len() > MAX_ATTACHMENTS {
			self.status = "Attach up to 10 files per message";
			return None;
		}
		if filenames.iter().any(|name| {
			name.trim().is_empty()
				|| name.len() > 256
				|| matches!(*name, "." | "..")
				|| name
					.chars()
					.any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
		}) {
			self.status = "Attachment filename is invalid or too long";
			return None;
		}
		let content = self.drafts.get(&channel).map_or("", String::as_str);
		if (content.trim().is_empty() && filenames.is_empty())
			|| content.chars().count() > MAX_CONTENT
			|| self.pending.len() >= 64
			|| self.draft_bytes()
				+ content.len()
				+ filenames
					.iter()
					.map(|name| name.len() + size_of::<String>())
					.sum::<usize>()
				+ size_of::<Pending>()
				+ 32 > MAX_DRAFT_BYTES
		{
			self.status = "Send exceeds the session input budget";
			return None;
		}
		let content = content.to_owned();
		self.send_sequence += 1;
		let epoch = std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_millis();
		let nonce = fingerprint::nonce(epoch, self.send_sequence);
		self.pending.push(Pending {
			channel,
			content: content.clone(),
			attachments: filenames.iter().map(|name| (*name).to_owned()).collect(),
			nonce: nonce.clone(),
			delivery: Delivery::Sending,
			confirmed: None,
		});
		self.drafts.remove(&channel);
		Some(Command::Send {
			channel,
			content,
			nonce,
			reply: self.reply.take(),
		})
	}
	pub fn command_rejected(&mut self, command: Command) {
		if let Command::MessagingPermissions { request, .. } = command {
			self.apply_messaging_permissions(
				request,
				Err(auth::Failure::ProtocolAt(
					"Messaging permissions were not queued; try again",
				)),
			);
			return;
		}
		if let Command::AccountNotificationSettings { request, .. } = command {
			self.apply_account_notification_settings(
				request,
				Err(auth::Failure::ProtocolAt(
					"Notification settings were not queued; try again",
				)),
			);
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
		if matches!(command, Command::GuildFolders(_)) {
			self.apply_guild_folders(Err(auth::Failure::ProtocolAt(
				"Server organization was not queued; try again",
			)));
			return;
		}
		if let Command::GroupAction { action, request } = command {
			let _ = self.apply_group_action(group_actions::Event::Written {
				channel: action.channel(),
				request,
				result: Err(auth::Failure::ProtocolAt(
					"Group action was not queued; try again",
				)),
			});
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
		if let Command::UserAction { action, request } = command {
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
		if matches!(command, Command::CancelSearch | Command::CancelGifs) {
			return;
		}
		if let Command::Gifs { request, .. } = command {
			self.apply_gifs(request, Err(auth::Failure::Capacity));
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
		if let Command::Invite { code } = command {
			self.apply_invite(code, Err(auth::Failure::Capacity));
			return;
		}
		if let Command::Profile {
			user,
			guild,
			request,
		} = command
		{
			self.apply_profile(user, guild, request, Err(auth::Failure::Capacity));
			return;
		}
		if matches!(command, Command::CancelProfile) {
			return;
		}

		if let Command::Voice(control) = command {
			match control {
				voice::Command::Sync { .. } => {
					self.status = "Call status could not refresh; reopen the DM to retry"
				}
				voice::Command::Join {
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
		if matches!(&command, Command::History { request, .. } if *request == self.request) {
			self.cancel_history();
		}
		if let Command::Send { nonce, .. } = command {
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
	pub fn apply(&mut self, envelope: Envelope) {
		self.apply_with_permissions(envelope, None);
	}
	fn apply_with_permissions(
		&mut self,
		envelope: Envelope,
		prepared_permissions: Option<permissions::Permissions>,
	) {
		self.reply_deletions.0.clear();
		if envelope.generation != self.generation {
			return;
		}
		if let Event::Startup(startup) = envelope.event {
			if startup.bytes() > model::account::MAX_BYTES {
				self.auth = auth::AuthState::Failed;
				self.status = "Account startup exceeds safe capacity";
				return;
			}
			let PreparedStartup {
				data,
				permissions: prepared,
				..
			} = *startup;
			let Startup {
				user,
				guilds,
				channels,
				permissions,
				read_state,
				notifications,
				session_dnd,
				mut warnings,
			} = data;
			self.apply_with_permissions(
				Envelope {
					generation: envelope.generation,
					event: Event::Ready {
						user,
						guilds,
						channels,
						permissions,
					},
				},
				Some(prepared),
			);
			if self.auth != auth::AuthState::Authenticated {
				return;
			}
			warnings.read_state |= self.apply_read_state(read_state).is_err();
			if let Some(settings) = notifications {
				warnings.notifications |= self.apply_notification_preferences(settings).is_err();
			}
			let _ =
				self.apply_notification_preferences(notifications::Event::Presence(session_dnd));
			self.apply(Envelope {
				generation: envelope.generation,
				event: Event::StartupWarnings(warnings),
			});
			return;
		}
		if matches!(envelope.event, Event::Ready { .. } | Event::Resync) {
			self.startup_warnings = Default::default();
		}
		if matches!(envelope.event, Event::Resync) {
			self.read_state.reset();
			self.notification_preferences = Default::default();
		}
		if matches!(
			envelope.event,
			Event::Ready { .. } | Event::Disconnected | Event::Resync
		) {
			self.local_game_activity = Default::default();
			self.invalidate_messaging_permissions(None);
			self.interrupt_own_profile();
		}
		if let Event::Typing(signal) = &envelope.event {
			self.observe_typing_at(
				*signal,
				std::time::SystemTime::now(),
				std::time::Instant::now(),
			);
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
		let access_changed = envelope.event.changes_access();
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
				.channels
				.iter()
				.any(|c| c.id == patch.id && c.guild == Some(*guild) && matches!(c.kind, 10..=12))
		{
			return;
		}
		if let Event::ThreadRemoved { guild, id } = &envelope.event
			&& !self
				.channels
				.iter()
				.any(|c| c.id == *id && c.guild == Some(*guild) && matches!(c.kind, 10..=12))
		{
			return;
		}
		self.timeline
			.set_preserve_deleted_messages(self.preserve_deleted_messages);
		self.invalidate_resident_event(&envelope.event);
		self.observe_channel_action(&envelope.event);
		if let Event::ChannelCreated(channel) = &envelope.event {
			self.observe_dm_reopened(channel.id);
			self.observe_group_change(channel.id, true);
		}
		if let Event::ChannelChanged(patch) = &envelope.event
			&& (!matches!(patch.name, Patch::Absent)
				|| !matches!(patch.icon, Patch::Absent)
				|| !matches!(patch.kind, Patch::Absent))
		{
			self.observe_group_change(patch.id, false);
		}
		self.revision += 1;
		if matches!(
			&envelope.event,
			Event::Disconnected | Event::Resync | Event::PermissionsChanged | Event::Ready { .. }
		) || matches!(&envelope.event,Event::Unavailable(id) if Some(*id)==self.selected)
			|| matches!(&envelope.event,Event::HistoryFailed{channel,failure:auth::Failure::Forbidden,..} if Some(*channel)==self.selected)
			|| matches!(&envelope.event,Event::RecipientRemoved{channel,user} if Some(*channel)==self.selected && self.user.as_ref().is_some_and(|u|u.id==*user))
		{
			self.clear_search();
			self.search_target = None;
		}
		// Search is a snapshot. A mutation can race an in-flight index response; invalidate
		// its snippets instead of restoring deleted/edited text from an older index.
		if matches!(&envelope.event,Event::Patch(p) if Some(p.channel)==self.selected)
			|| matches!(&envelope.event,Event::Delete{channel,..}|Event::DeleteBulk{channel,..} if Some(*channel)==self.selected)
		{
			self.clear_search();
		}
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
				if warnings.read_state {
					self.read_state.reset();
				}
				if warnings.notifications || warnings.sessions {
					self.invalidate_startup_preferences(warnings.notifications, warnings.sessions);
				}
				if warnings.presence {
					self.clear_direct_presences();
				}
				Ok(())
			}
			Event::GuildFolders(result) => {
				self.apply_guild_folders(result);
				Ok(())
			}
			Event::Typing(_) => unreachable!("typing is handled before timeline invalidation"),
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
				// Typed updates replace the old PermissionsChanged fallback too.
				// Retire both snapshots and in-flight profiles before applying them.
				self.clear_profile();
				self.profile_cache.clear();
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
			Event::Gifs { request, result } => {
				self.apply_gifs(request, result);
				Ok(())
			}
			Event::ReadState(event) => self.apply_read_state(event),
			Event::NotificationPreferences(event) => self.apply_notification_preferences(event),
			Event::SocialNotification(item) => {
				self.receive_social_notification(item);
				Ok(())
			}
			Event::MessagingPermissions { request, result } => {
				self.apply_messaging_permissions(request, result);
				Ok(())
			}
			Event::AccountNotificationSettings { request, result } => {
				self.apply_account_notification_settings(request, result);
				Ok(())
			}
			Event::UserAction(event) => self.apply_user_action(event),
			Event::ServerAction(event) => self.apply_server_action(event),
			Event::ChannelAction(event) => self.apply_channel_action(event),
			Event::ServerSettings(event) => self.apply_server_settings(event),
			Event::ServerAdmin(event) => self.apply_server_admin(event),
			Event::GroupAction(event) => self.apply_group_action(event),
			Event::ThreadsSync {
				guild,
				parents,
				threads,
				removed,
			} => self.apply_threads_sync(guild, parents, threads, removed),
			Event::Reactions(event) => self.apply_reactions(event),
			Event::InviteChallenge { request, challenge } => {
				self.apply_invite_challenge(request, *challenge);
				Ok(())
			}
			Event::JoinInvite { request, result } => {
				self.apply_invite_join(request, result);
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
					self.guilds.push(guild);
					bytes += (self.guilds.capacity() - capacity) * size_of::<Guild>();
					if bytes + self.permissions.bytes() > model::account::MAX_BYTES {
						self.guilds.pop();
						self.guilds.shrink_to_fit();
						self.invalidate_navigation();
						self.fail(auth::Failure::Capacity);
						return;
					}
					self.set_navigation_bytes(bytes);
				}
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
			Event::Profile {
				user,
				guild,
				request,
				result,
			} => {
				self.apply_profile(user, guild, request, result);
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
			Event::GuildChanged(patch) => {
				if let Some(index) = self.guilds.iter().position(|guild| guild.id == patch.id) {
					let previous = self.guilds[index].clone();
					let guild = &mut self.guilds[index];
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
					&& self.channels.iter().any(|old| {
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
						|| self.channels.iter().any(|old| {
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
				if let Some(index) = self.channels.iter().position(|c| c.id == patch.id) {
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
						self.selected = None;
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
					.channels
					.iter()
					.position(|c| c.id == channel && c.guild.is_none())
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
				if let Some(c) = self
					.channels
					.iter_mut()
					.find(|c| c.id == channel && c.guild.is_none())
				{
					c.recipients.retain(|u| u.id != user);
					if self.selected == Some(channel) && self.members.is_some() {
						let _ = self.request_members();
					}
				}
				Ok(())
			}
			Event::MemberPresence { .. } | Event::DirectPresence(_) => {
				unreachable!("presence handled before timeline revision")
			}
			Event::Members(list) => {
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
				if list.rows.len() > 100
					|| list.rows.iter().flatten().any(|row| !row.valid())
					|| list
						.rows
						.iter()
						.flatten()
						.any(|member| member.roles.len() > model::permissions::MAX_MEMBER_ROLES)
					|| list.rows.iter().flatten().map(Member::bytes).sum::<usize>() > 128 * 1024
				{
					self.members.as_mut().unwrap().freshness = Freshness::Unavailable;
				} else {
					self.members = Some(list);
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
				if self
					.user
					.as_ref()
					.is_some_and(|previous| previous.id != user.id)
				{
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
						self.auth = auth::AuthState::Failed;
						self.status = status;
						return;
					}
				};
				let current: BTreeMap<_, _> = channels
					.iter()
					.map(|channel| (channel.id, channel))
					.collect();
				let mut removed: BTreeSet<_> = self
					.channels
					.iter()
					.filter(|old| {
						current.get(&old.id).is_none_or(|channel| {
							(navigable(old) && !navigable(channel))
								|| (old.supports_text() != channel.supports_text())
						})
					})
					.map(|channel| channel.id)
					.collect();
				let unavailable = self
					.selected
					.is_some_and(|id| current.get(&id).is_none_or(|channel| !navigable(channel)));
				if unavailable && let Some(selected) = self.selected {
					removed.insert(selected);
				}
				self.remove_channels(&removed);
				self.cancel_history();
				if unavailable {
					self.selected = None;
					self.freshness = Freshness::Unavailable;
				} else {
					self.freshness = Freshness::Stale;
				}
				self.voice.roster.clear();
				self.voice.dm_calls.clear();
				self.voice.dm_participants.clear();
				self.members = None;
				self.clear_profile();
				self.profile_cache.clear();
				self.read_state.reset();
				self.notification_preferences = notifications::Preferences::default();
				self.cancel_user_action();
				self.cancel_server_action();
				self.cancel_channel_action();
				self.cancel_server_settings();
				self.cancel_server_admin();
				self.cancel_group_action();
				self.cancel_invite_join();
				self.user_actions.reset();
				self.server_actions.reset();
				self.channel_actions.reset();
				self.server_settings.reset();
				self.server_admin.reset();
				self.server_members_shortcuts.clear();
				self.group_actions.reset();
				self.clear_own_profile();
				self.user = Some(user);
				self.guilds = guilds;
				self.channels = channels;
				self.permissions = permission_state;
				self.archived_thread = None;
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
				request,
				older,
				mut messages,
			} => {
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
				let mut ids = BTreeSet::new();
				let has_deleted_reference = messages.iter().any(|message| message.reply_deleted);
				if older != self.history_before.is_some()
					|| messages.len() > 50
					|| messages.iter().any(|message| {
						message.channel != channel
							|| (has_deleted_reference && !Timeline::valid_message(message))
							|| self
								.history_before
								.is_some_and(|before| message.id >= before)
							|| self.history_after.is_some_and(|after| message.id <= after)
							|| !ids.insert(message.id)
					}) {
					self.cancel_history();
					self.fail(auth::Failure::Protocol);
					return;
				}
				for message in &messages {
					self.record_reply_deletion(message);
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
				self.older_exhausted = if let Some(after) = self.history_after {
					after.0 == 0
				} else {
					messages.len() < 50
				};
				if self.history_after.is_some() {
					self.newer_cursor = messages.iter().map(|m| m.id).max();
					self.newer_may_have_more = messages.len() == 50;
				}
				let r = self.timeline.finish_page(messages, older);
				if r.is_ok() && self.history_after.is_some() {
					self.search_target = self.timeline.iter().next().map(|m| m.id);
					if self.search_target.is_none() {
						self.status = "No messages returned after this boundary; use Jump to present to reload";
					}
				}
				if r.is_ok() && self.gateway_connected {
					self.freshness = Freshness::Fresh;
				}
				r
			}
			Event::HistoryFailed {
				channel,
				request,
				failure,
			} => {
				if self.selected != Some(channel)
					|| request != self.request
					|| !self.history_pending
				{
					return;
				}
				self.cancel_history();
				if failure == auth::Failure::Forbidden {
					self.invalidate_members();
					self.timeline.clear();
					self.freshness = Freshness::Unavailable;
					self.status = "Channel unavailable or permission denied";
				} else {
					self.fail(failure);
				}
				Ok(())
			}
			Event::Message(mut m) => {
				if self.timeline.get(m.id).is_some_and(|old| {
					old.edited_at
						.is_none_or(|at| m.edited_at.is_some_and(|new| new >= at))
				}) {
					self.message_actions.observe_content(m.channel, m.id);
				}
				self.typing_message(&m);
				if let Some(post) = self
					.channels
					.iter_mut()
					.find(|c| c.id == m.channel && matches!(c.kind, 10..=12))
				{
					post.message_count = post.message_count.map(|n| n.saturating_add(1));
				}
				let deletion = if m.reply_deleted && self.accepts_reply_source(&m) {
					self.record_reply_deletion(&m);
					if self.selected == Some(m.channel) {
						self.timeline.observe_deleted_reference(&m)
					} else {
						Ok(())
					}
				} else {
					Ok(())
				};
				self.observe_notification(&m);
				self.observe_last_message(m.channel, m.id);
				if self.selected == Some(m.channel)
					&& self.reactions.invalidated(m.id)
					&& m.reactions.is_some()
				{
					self.queue_reaction_read(m.id);
				}
				if self.reactions.invalidated(m.id) {
					m.reactions = self
						.timeline
						.get(m.id)
						.and_then(|old| old.reactions.clone());
				}
				self.confirm(&m);
				if deletion.is_err() {
					deletion
				} else if self.selected == Some(m.channel)
                    && self.can_view(m.channel)
                    && self.freshness != Freshness::Unavailable
                    // Do not splice a new live tail into a deliberately browsed history page.
                    // Loaded edits still reconcile; navigation/read state continues to observe it.
                    && (!self.history_targeted || self.timeline.get(m.id).is_some())
				{
					self.timeline.insert(m, true, false)
				} else {
					Ok(())
				}
			}
			Event::Patch(mut p) => {
				if !matches!(p.content, Patch::Absent)
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
				self.read_state.activity.delete(channel, id);
				if let Some(channel) = self
					.channels
					.iter_mut()
					.find(|c| c.id == channel && c.last_message == Some(id))
				{
					self.read_state.activity.observe_latest(channel.id, id);
					channel.last_message = None;
				}
				if self.selected == Some(channel) {
					if self.reply == Some(id) {
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
				if ids.len() <= 100 {
					for id in &ids {
						self.read_state.activity.delete(channel, *id);
					}
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
					if self.reply.is_some_and(|id| ids.contains(&id)) {
						self.reply = None;
					}
					ids.into_iter().try_for_each(|id| self.timeline.delete(id))
				} else {
					Ok(())
				}
			}
			Event::SendResult { nonce, result } => {
				let mut reconciliation = Ok(());
				match result {
					Ok(m) => {
						let correlated = self
							.user
							.as_ref()
							.is_some_and(|user| user.id == m.author.id)
							&& (self.pending.iter().any(|pending| {
								pending.nonce == nonce && pending.channel == m.channel
							}) || self.timeline.get(m.id).is_some_and(|known| {
								known.channel == m.channel
									&& known.author.id == m.author.id
									&& known.nonce.as_deref() == Some(nonce.as_str())
							}));
						if correlated {
							self.observe_last_message(m.channel, m.id);
						}
						let accepted_reference = correlated && self.accepts_reply_source(&m);
						if accepted_reference {
							self.record_reply_deletion(&m);
						}
						if let Some(p) = self.pending.iter_mut().find(|p| p.nonce == nonce) {
							p.delivery = Delivery::Confirmed;
							p.confirmed = Some(m.id);
						}
						if accepted_reference
							&& self.selected == Some(m.channel)
							&& let Err(status) = self.timeline.observe_deleted_reference(&m)
						{
							reconciliation = Err(status);
						}
						if reconciliation.is_ok()
							&& self.selected == Some(m.channel)
							&& self.can_view(m.channel)
							&& self.freshness != Freshness::Unavailable
							&& self.timeline.get(m.id).is_none()
							&& !self.history_targeted
							&& (!m.reply_deleted || accepted_reference)
							&& self.timeline.insert(m, false, false).is_err()
						{
							reconciliation = Err("Message exceeds safe capacity");
						}
					}
					Err(f) => {
						if let Some(p) = self
							.pending
							.iter_mut()
							.find(|p| p.nonce == nonce && p.delivery != Delivery::Confirmed)
						{
							p.delivery = if f == auth::Failure::Ambiguous {
								Delivery::Ambiguous
							} else {
								Delivery::Rejected
							};
						}
						if f.ends_session() {
							self.fail(f);
						} else {
							self.status = f.label();
						}
					}
				}
				self.pending.retain(|p| p.delivery != Delivery::Confirmed);
				reconciliation
			}
			Event::Failure(f) => {
				self.fail(f);
				Ok(())
			}
			Event::Disconnected => {
				self.interrupt_notification_settings(auth::Failure::ProtocolAt(
					"Disconnected; reload notification settings",
				));
				self.cancel_message_actions();
				self.cancel_user_action();
				self.cancel_server_action();
				self.cancel_channel_action();
				self.cancel_server_settings();
				self.cancel_server_admin();
				self.cancel_group_action();
				self.cancel_invite_join();
				self.read_state.cancel();
				self.clear_profile();
				// The voice socket is independent and a RESUME replays roster changes, so the call,
				// roster and known DM calls all stay. Only a fresh READY invalidates the voice state.
				self.voice.incoming = None;
				self.invalidate_members();
				self.gateway_connected = false;
				self.cancel_history();
				self.freshness = Freshness::Stale;
				self.status = "Reconnecting…";
				Ok(())
			}
			Event::Resumed => {
				self.members = None;
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
				self.cancel_server_admin();
				self.cancel_group_action();
				self.cancel_invite_join();
				self.clear_direct_presences();
				self.permissions = permissions::Permissions::default();
				self.read_state.cancel();
				self.clear_profile();
				self.profile_cache.clear();
				self.disconnect_voice(
					"Discord session or permissions changed; rejoin after refreshing",
				);
				self.invalidate_members();
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
		if let Err(status) = result {
			self.clear_cached_history();
			self.status = status;
			self.freshness = Freshness::Stale;
			self.timeline.clear();
			self.cancel_history();
		}
		if self
			.reply
			.is_some_and(|target| self.timeline.is_deleted(target))
		{
			self.reply = None;
		}
		if access_changed {
			self.reconcile_permissions(previous_access);
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
		}
		if self.search.is_some() && !self.can_search() {
			self.clear_search();
		}
		if self
			.archives
			.as_ref()
			.is_some_and(|view| !self.can_archive(view.parent, view.kind))
		{
			self.clear_archives();
		}
		if access_changed {
			self.prune_direct_presence();
		}
		if self
			.server_settings
			.guild
			.is_some_and(|guild| !self.can_manage_guild(guild))
		{
			self.server_settings.reset();
		}
		if access_changed && !self.server_members_shortcuts.is_empty() {
			let guilds: BTreeSet<_> = self.guilds.iter().map(|guild| guild.id).collect();
			self.server_members_shortcuts
				.retain(|id, _| guilds.contains(id));
		}
		if let Some(guild) = self.server_admin.guild {
			self.prune_integration_access(guild);
			if !self.can_open_audit_log_settings(guild) {
				self.server_admin.revoke_audit_access();
			}
			if !self.can_open_emoji_settings(guild)
				&& !self.can_open_member_settings(guild)
				&& !self.can_open_role_settings(guild)
				&& !self.can_open_integration_settings(guild)
				&& !self.can_open_audit_log_settings(guild)
			{
				self.server_admin.reset();
			} else {
				if !self.can_open_emoji_settings(guild) {
					self.server_admin.emojis = None;
				}
				if !self.can_open_member_settings(guild) {
					self.server_admin.members = None;
					self.server_admin.revoke_invite_access();
				}
				if !self.can_open_role_settings(guild) {
					self.server_admin.roles = None;
					self.server_admin.selected_role = None;
				}
			}
		}
		self.enforce_resident_budget();
	}
	fn invalidate_members(&mut self) {
		self.member_request = self.member_request.wrapping_add(1);
		if let Some(list) = &mut self.members {
			list.request = self.member_request;
			list.rows.clear();
			list.freshness = Freshness::Unavailable;
		}
	}
	fn remove_channels(&mut self, removed: &BTreeSet<Id>) {
		self.invalidate_navigation();
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
		if !removed.is_empty() {
			self.permissions.clear_cache();
		}
		for id in removed {
			self.permissions.channels.remove(id);
			self.end_voice_channel(*id);
			self.read_state.forget(*id);
		}
		if !removed.is_empty() {
			self.clear_profile();
			self.profile_cache.clear();
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
	fn confirm(&mut self, message: &Message) {
		if self.user.as_ref().map(|u| u.id) != Some(message.author.id) {
			return;
		}
		if let Some(nonce) = &message.nonce {
			self.pending
				.retain(|p| !(p.nonce == *nonce && p.channel == message.channel));
		}
	}
	fn fail(&mut self, failure: auth::Failure) {
		self.typing.clear();
		self.status = failure.label();
		match failure {
			auth::Failure::Expired => self.auth = auth::AuthState::Expired,
			auth::Failure::Challenged => self.auth = auth::AuthState::Challenged,
			_ => {}
		}
		if failure.ends_session() {
			self.invalidate_messaging_permissions(Some(failure));
			self.interrupt_notification_settings(failure);
			self.interrupt_own_profile();
			self.local_game_activity = Default::default();
			self.cancel_message_actions();
			if self.folders_pending {
				self.folders_pending = false;
				self.folders_error = Some(failure.label());
			}
			self.cancel_user_action();
			self.cancel_server_action();
			self.cancel_channel_action();
			self.cancel_server_settings();
			self.cancel_server_admin();
			self.cancel_group_action();
			self.cancel_invite_join();
			self.clear_direct_presences();
			self.clear_cached_history();
			self.clear_search();
			self.search_target = None;
			self.read_state.cancel();
			self.clear_profile();
			self.profile_cache.clear();
			self.disconnect_voice(failure.label());
			self.gateway_connected = false;
			self.invalidate_members();
			self.cancel_history();
		}
		self.freshness = Freshness::Stale;
	}
	fn cancel_history(&mut self) {
		self.typing.clear();
		self.reactions.reset();
		self.search_target = None;
		self.request += 1;
		self.history_pending = false;
		self.history_after = None;
		self.newer_cursor = None;
		self.newer_may_have_more = false;
		self.timeline.cancel_page();
	}
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
				action: user_actions::Action::CloseDm(_),
				result: Ok(()),
				..
			}) | Event::ServerAction(server_actions::Event::Written {
				action: server_actions::Action::Leave(_),
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

	/// Admission estimate for heap-owned fields; rejects oversized items before entering the UI queue.
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ match self {
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
				Self::ServerAdmin(event) => event
					.result
					.as_ref()
					.map_or(0, model::server_admin::Result::bytes),
				Self::SocialNotification(item) => item.body.capacity(),
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
				Self::UserAction(user_actions::Event::Relationships(entries)) => entries
					.as_ref()
					.map_or(0, |e| e.capacity() * size_of::<(Id, bool)>()),
				Self::Archives { result, .. } => {
					result.as_ref().map_or(0, model::archives::Page::bytes)
				}
				Self::ForumPosts { result, .. } => {
					result.as_ref().map_or(0, model::forum::Page::bytes)
				}
				Self::PostCreated { result, .. } => result.as_ref().map_or(0, Channel::bytes),
				Self::Search {
					result: Ok(search::Outcome::Page(page) | search::Outcome::Pins(page)),
					..
				} => page.bytes(),
				Self::Gifs {
					result: Ok(page), ..
				} => page.bytes(),
				Self::ReadState(read_state::Event::Snapshot { entries, .. }) => entries
					.as_ref()
					.map_or(0, |e| e.capacity() * size_of::<(Id, Option<Id>, u32)>()),
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
				Self::Voice(event) => event.bytes(),
				Self::Permissions(event) => event.bytes(),
				Self::GuildEmojis { emojis, .. } => custom_emoji_bytes(emojis),
				Self::GuildChanged(patch) => [&patch.name, &patch.icon]
					.into_iter()
					.map(|value| match value {
						Patch::Value(value) => value.capacity(),
						_ => 0,
					})
					.sum(),
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
				Self::Members(list) => {
					list.rows.capacity() * size_of::<Option<Member>>()
						+ list.rows.iter().flatten().map(Member::bytes).sum::<usize>()
				}
				Self::RecipientAdded { user, .. } => user.heap_bytes(),
				Self::History { messages, .. } => messages.iter().map(Message::bytes).sum(),
				Self::Message(m) => m.bytes(),
				Self::Patch(p) => {
					let content = match &p.content {
						Patch::Value(s) => s.capacity(),
						_ => 0,
					};
					content
						+ match &p.reactions {
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
				_ => 0,
			}
	}
}

#[cfg(test)]
mod tests {
	pub(crate) fn grant_permissions(state: &mut State) {
		let Some(user) = &state.user else {
			return;
		};
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: state
					.guilds
					.iter()
					.map(|guild| model::permissions::Guild {
						id: guild.id,
						owner: Some(user.id),
						roles: Some(vec![]),
						member: None,
					})
					.collect(),
				channels: state
					.channels
					.iter()
					.filter_map(|channel| {
						channel.guild.map(|guild| model::permissions::Channel {
							id: channel.id,
							guild,
							overwrites: Some(vec![]),
						})
					})
					.collect(),
			})
			.unwrap();
	}
	#[test]
	fn rejected_sends_do_not_invalidate_a_healthy_conversation() {
		let mut state = State {
			channels: vec![Channel {
				id: Id(1),
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic DM".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			selected: Some(Id(1)),
			auth: auth::AuthState::Authenticated,
			freshness: Freshness::Fresh,
			gateway_connected: true,
			..State::default()
		};
		let command = state.prepare_send_with_attachment(Some("a.txt")).unwrap();
		let Command::Send { nonce, .. } = &command else {
			panic!()
		};
		let nonce = nonce.clone();
		state.selected = Some(Id(2));
		state.command_rejected(command);
		assert_eq!(state.pending[0].delivery, Delivery::Rejected);
		assert_eq!(state.freshness, Freshness::Fresh);
		apply(
			&mut state,
			Event::SendResult {
				nonce: nonce.clone(),
				result: Err(auth::Failure::ProtocolAt(
					"Upload cancelled; no message was sent",
				)),
			},
		);
		assert_eq!(state.freshness, Freshness::Fresh);
		assert!(state.gateway_connected);
		apply(
			&mut state,
			Event::SendResult {
				nonce,
				result: Err(auth::Failure::Expired),
			},
		);
		assert_eq!(state.auth, auth::AuthState::Expired);
		assert_eq!(state.freshness, Freshness::Stale);
		assert!(!state.gateway_connected);
	}
	#[test]
	fn attachment_only_sends_are_bounded_and_keep_existing_confirmation() {
		let mut state = State {
			channels: vec![Channel {
				id: Id(1),
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic DM".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			selected: Some(Id(1)),
			auth: auth::AuthState::Authenticated,
			freshness: Freshness::Fresh,
			gateway_connected: true,
			..State::default()
		};
		assert!(state.prepare_send().is_none());
		for invalid in [
			"",
			" ",
			".",
			"..",
			"../secret.txt",
			"C:\\secret.txt",
			"a\nb",
		] {
			assert!(state.prepare_send_with_attachment(Some(invalid)).is_none());
		}
		assert!(
			state
				.prepare_send_with_attachment(Some(&"é".repeat(129)))
				.is_none()
		);
		assert!(
			state
				.prepare_send_with_attachments(&["a.txt"; MAX_ATTACHMENTS + 1])
				.is_none()
		);
		assert!(
			state
				.prepare_send_with_attachments(&["a.txt", "../b.txt"])
				.is_none()
		);
		assert!(state.pending.is_empty());
		let batch = state
			.prepare_send_with_attachments(&["a.png", "b.png", "c.pdf"])
			.unwrap();
		assert_eq!(state.pending[0].attachments, ["a.png", "b.png", "c.pdf"]);
		state.command_rejected(batch);
		state.pending.clear();
		state.reply = Some(Id(4));
		let Command::Send {
			content,
			nonce,
			reply,
			..
		} = state
			.prepare_send_with_attachment(Some("résumé.txt"))
			.unwrap()
		else {
			panic!()
		};
		assert!(content.is_empty());
		assert_eq!(reply, Some(Id(4)));
		assert_eq!(state.pending[0].attachments, ["résumé.txt"]);
		assert!(state.draft_bytes() >= "résumé.txt".len() + nonce.len() + size_of::<Pending>());
		apply(
			&mut state,
			Event::SendResult {
				nonce: nonce.clone(),
				result: Err(auth::Failure::Ambiguous),
			},
		);
		assert_eq!(state.pending[0].delivery, Delivery::Ambiguous);
		let mut confirmed = message(10);
		confirmed.nonce = Some(nonce.clone());
		apply(
			&mut state,
			Event::SendResult {
				nonce,
				result: Ok(confirmed),
			},
		);
		assert!(state.pending.is_empty());
		state.freshness = Freshness::Fresh;
		state.drafts.insert(
			Id(2),
			"x".repeat(MAX_DRAFT_BYTES - size_of::<Pending>() - 32),
		);
		assert!(state.prepare_send_with_attachment(Some("a.txt")).is_none());
		state.logout();
		assert!(!state.has_unsent());
	}
	use super::*;

	#[test]
	fn reaction_readback_coalesces_races_and_never_replays_uncertain_writes() {
		use reactions::{Command as R, Event as E};
		let channel = Id(1);
		let id = Id(10);
		let emoji = ReactionEmoji {
			id: None,
			name: Some("👍".into()),
		};
		let values = vec![Reaction {
			emoji: emoji.clone(),
			count: 3,
			me: true,
			me_burst: false,
		}];
		let mut state = State {
			selected: Some(channel),
			channels: vec![Channel {
				id: channel,
				guild: None,
				parent_id: None,
				kind: 1,
				name: "Synthetic reaction conversation".into(),
				position: 0,
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			gateway_connected: true,
			auth: auth::AuthState::Authenticated,
			freshness: Freshness::Fresh,
			..State::default()
		};
		state.timeline.insert(message(10), false, false).unwrap();
		let Command::Reactions(R::Set { request, add, .. }) =
			state.prepare_reaction(id, emoji.clone()).unwrap()
		else {
			panic!()
		};
		assert!(add);
		assert!(state.prepare_reaction(id, emoji.clone()).is_none());
		apply(
			&mut state,
			Event::Reactions(E::Written {
				channel,
				message: id,
				request,
				result: Err(auth::Failure::Ambiguous),
			}),
		);
		assert!(state.reactions.writing.is_none());
		let Command::Reactions(R::Read { request, .. }) = state.next_reaction_read().unwrap()
		else {
			panic!()
		};
		for _ in 0..100 {
			apply(
				&mut state,
				Event::Reactions(E::Changed {
					channel,
					message: id,
				}),
			);
		}
		assert!(state.next_reaction_read().is_none());
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message: id,
				request,
				result: Ok(values.clone()),
			}),
		);
		assert!(state.timeline.get(id).unwrap().reactions.is_none());
		let Command::Reactions(R::Read { request, .. }) = state.next_reaction_read().unwrap()
		else {
			panic!()
		};
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message: id,
				request,
				result: Ok(values.clone()),
			}),
		);
		assert_eq!(
			state.timeline.get(id).unwrap().reactions,
			Some(values.clone())
		);
		assert!(state.next_reaction_read().is_none());
		let command = state.prepare_reaction(id, emoji.clone()).unwrap();
		assert!(matches!(
			&command,
			Command::Reactions(R::Set { add: false, .. })
		));
		state.command_rejected(command);
		assert!(state.reactions.writing.is_none());
		assert_eq!(state.freshness, Freshness::Fresh);
		// Successful reaction removal/readback changes only reactions, not chat access.
		let original_content = state.timeline.get(id).unwrap().content.clone();
		let Command::Reactions(R::Set {
			request,
			add: false,
			..
		}) = state.prepare_reaction(id, emoji.clone()).unwrap()
		else {
			panic!()
		};
		apply(
			&mut state,
			Event::Reactions(E::Written {
				channel,
				message: id,
				request,
				result: Ok(()),
			}),
		);
		assert_eq!(state.timeline.get(id).unwrap().content, original_content);
		let Command::Reactions(R::Read { request, .. }) = state.next_reaction_read().unwrap()
		else {
			panic!()
		};
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message: id,
				request,
				result: Ok(vec![]),
			}),
		);
		assert_eq!(state.timeline.get(id).unwrap().content, original_content);
		assert_eq!(state.timeline.get(id).unwrap().reactions, Some(vec![]));
		assert_eq!(state.freshness, Freshness::Fresh);
		assert!(state.next_reaction_read().is_none());
		// Reaction events during history loading cannot be overwritten by that page.
		let _ = state.history(None);
		apply(
			&mut state,
			Event::Reactions(E::Changed {
				channel,
				message: Id(11),
			}),
		);
		let history_request = state.request;
		apply(
			&mut state,
			Event::History {
				channel,
				request: history_request,
				older: false,
				messages: vec![message(11)],
			},
		);
		assert!(state.timeline.get(Id(11)).unwrap().reactions.is_none());
		let Command::Reactions(R::Read {
			message: id,
			request,
			..
		}) = state.next_reaction_read().unwrap()
		else {
			panic!()
		};
		// Deletion wins over an in-flight reaction response.
		apply(&mut state, Event::Delete { channel, id });
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message: id,
				request,
				result: Ok(values.clone()),
			}),
		);
		assert!(state.timeline.get(id).is_none());
		// Rate limits leave an explicit retry, never an automatic loop.
		state.timeline.insert(message(12), false, false).unwrap();
		state.refresh_reactions(Id(12));
		let Command::Reactions(R::Read {
			message: id,
			request,
			..
		}) = state.next_reaction_read().unwrap()
		else {
			panic!()
		};
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message: id,
				request,
				result: Err(auth::Failure::RateLimited),
			}),
		);
		assert!(state.next_reaction_read().is_none());
		state.refresh_reactions(id);
		let Command::Reactions(R::Read { request, .. }) = state.next_reaction_read().unwrap()
		else {
			panic!()
		};
		state.reactions.reset(); // navigation/disconnect cancels this request
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message: id,
				request,
				result: Ok(values),
			}),
		);
		assert!(state.timeline.get(id).unwrap().reactions.is_none());
		state.refresh_reactions(id);
		let Command::Reactions(R::Read { request, .. }) = state.next_reaction_read().unwrap()
		else {
			panic!()
		};
		apply(
			&mut state,
			Event::Reactions(E::Read {
				channel,
				message: id,
				request,
				result: Err(auth::Failure::Forbidden),
			}),
		);
		assert!(state.timeline.is_empty());
		assert_eq!(state.freshness, Freshness::Unavailable);
		state.logout();
		assert!(state.next_reaction_read().is_none());
	}

	#[test]
	fn guild_emoji_updates_replace_catalog_and_reject_stale_or_oversized_data() {
		let emoji = CustomEmoji {
			id: Id(4),
			name: "wave".into(),
			animated: false,
			available: true,
			managed: false,
			roles: Some(vec![]),
		};
		let mut state = State {
			guilds: vec![Guild {
				id: Id(2),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			..State::default()
		};
		let event = Event::GuildEmojis {
			guild: Id(2),
			emojis: vec![emoji.clone()],
		};
		assert!(event.bytes() >= size_of::<Event>() + custom_emoji_bytes(&vec![emoji.clone()]));
		apply(&mut state, event);
		assert_eq!(
			state.guilds[0].emojis.as_ref().unwrap()[0].markup(),
			"<:wave:4>"
		);
		state.apply(Envelope {
			generation: state.generation + 1,
			event: Event::GuildEmojis {
				guild: Id(2),
				emojis: vec![],
			},
		});
		assert_eq!(state.guilds[0].emojis.as_ref().unwrap().len(), 1);
		apply(
			&mut state,
			Event::GuildEmojis {
				guild: Id(2),
				emojis: vec![],
			},
		);
		assert!(state.guilds[0].emojis.as_ref().unwrap().is_empty());
		let mut huge = emoji.clone();
		huge.name = String::with_capacity(MAX_GUILD_EMOJI_BYTES);
		huge.name.push_str("wave");
		apply(
			&mut state,
			Event::GuildEmojis {
				guild: Id(2),
				emojis: vec![huge],
			},
		);
		assert!(state.guilds[0].emojis.is_none());
		apply(
			&mut state,
			Event::GuildEmojis {
				guild: Id(2),
				emojis: vec![emoji; MAX_GUILD_EMOJIS + 1],
			},
		);
		assert!(state.guilds[0].emojis.is_none());
		state.logout();
		assert!(state.guilds.is_empty());
	}

	#[test]
	fn ready_rejects_emoji_catalogs_exceeding_total_navigation_budget() {
		let emoji = CustomEmoji {
			id: Id(4),
			name: "wave".into(),
			animated: false,
			available: true,
			managed: false,
			roles: Some(vec![]),
		};
		let mut state = State::default();
		let mut guilds: Vec<_> = (1..=700)
			.map(|id| Guild {
				id: Id(id),
				name: "Synthetic".into(),
				icon: None,
				emojis: Some(vec![emoji.clone()]),
			})
			.collect();
		for guild in &mut guilds {
			let name = &mut guild.emojis.as_mut().unwrap()[0].name;
			name.reserve(192 * 1024);
		}
		assert!(
			guilds
				.iter()
				.all(|g| valid_custom_emojis(g.emojis.as_ref().unwrap()))
		);
		apply(
			&mut state,
			Event::Ready {
				permissions: model::permissions::Snapshot::default(),
				user: User {
					id: Id(1),
					name: "Synthetic".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				},
				guilds,
				channels: vec![],
			},
		);
		assert!(state.guilds.is_empty());
		assert!(matches!(state.auth, auth::AuthState::Failed));
	}

	#[test]
	fn guild_identity_patches_preserve_omitted_fields_and_change_icon_keys() {
		let mut state = State {
			guilds: vec![Guild {
				emojis: None,
				id: Id(2),
				name: "Synthetic server".into(),
				icon: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
			}],
			..State::default()
		};
		let key = state.guilds[0].icon_key();
		apply(
			&mut state,
			Event::GuildChanged(GuildPatch {
				id: Id(2),
				name: Patch::Value("Renamed".into()),
				icon: Patch::Absent,
			}),
		);
		assert_eq!(state.guilds[0].name, "Renamed");
		assert_eq!(state.guilds[0].icon_key(), key);
		apply(
			&mut state,
			Event::GuildChanged(GuildPatch {
				id: Id(2),
				name: Patch::Absent,
				icon: Patch::Value("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
			}),
		);
		assert_ne!(state.guilds[0].icon_key(), key);
		assert_eq!(state.guilds[0].name, "Renamed");
		apply(
			&mut state,
			Event::GuildChanged(GuildPatch {
				id: Id(2),
				name: Patch::Absent,
				icon: Patch::Null,
			}),
		);
		assert!(state.guilds[0].icon_key().is_none());
		apply(
			&mut state,
			Event::GuildChanged(GuildPatch {
				id: Id(2),
				name: Patch::Value("x".repeat(1024)),
				icon: Patch::Value("../../invalid".into()),
			}),
		);
		assert_eq!(state.guilds[0].name.len(), 128);
		assert!(state.guilds[0].icon.is_none());
		apply(
			&mut state,
			Event::GuildChanged(GuildPatch {
				id: Id(3),
				name: Patch::Value("Unknown".into()),
				icon: Patch::Absent,
			}),
		);
		assert_eq!(state.guilds.len(), 1);
		state.apply(Envelope {
			generation: state.generation + 1,
			event: Event::GuildChanged(GuildPatch {
				id: Id(2),
				name: Patch::Value("Late event".into()),
				icon: Patch::Absent,
			}),
		});
		assert_eq!(state.guilds[0].name.len(), 128);
	}

	#[test]
	fn channel_mutations_preserve_partial_metadata_and_remove_deleted_categories() {
		let mut state = State {
			user: Some(message(1).author),
			guilds: vec![Guild {
				id: Id(1),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			..State::default()
		};
		let channel = Channel {
			last_message: None,
			id: Id(2),
			guild: Some(Id(1)),
			parent_id: Some(Id(3)),
			position: 7,
			name: "Synthetic channel".into(),
			kind: 0,
			recipients: vec![],
			icon: None,
			member_list_id: None,
			message_count: None,
		};
		apply(&mut state, Event::ChannelCreated(channel.clone()));
		apply(&mut state, Event::ChannelCreated(channel));
		assert_eq!(state.channels.len(), 1);
		apply(
			&mut state,
			Event::ChannelChanged(ChannelPatch {
				icon: model::Patch::Absent,
				last_message: model::Patch::Absent,
				id: Id(2),
				name: Patch::Absent,
				parent_id: Patch::Null,
				position: Patch::Value(0),
				kind: Patch::Absent,
				message_count: Patch::Absent,
			}),
		);
		assert_eq!(state.channels[0].parent_id, None);
		assert_eq!(state.channels[0].position, 0);
		assert_eq!(state.channels[0].name, "Synthetic channel");
		assert_eq!(state.channels[0].kind, 0);
		grant_permissions(&mut state);
		assert!(state.select(Id(2)).is_some());
		apply(
			&mut state,
			Event::ChannelChanged(ChannelPatch {
				icon: model::Patch::Absent,
				last_message: model::Patch::Absent,
				id: Id(2),
				name: Patch::Absent,
				parent_id: Patch::Absent,
				position: Patch::Absent,
				kind: Patch::Value(4),
				message_count: Patch::Absent,
			}),
		);
		assert!(state.selected.is_none());
		assert!(!state.history_pending);
		assert!(state.select(Id(2)).is_none());
		apply(&mut state, Event::Unavailable(Id(2)));
		assert!(state.channels.is_empty());
		for id in 1..=MAX_NAV + 1 {
			apply(
				&mut state,
				Event::ChannelCreated(Channel {
					last_message: None,
					id: Id(id as u64),
					guild: None,
					parent_id: None,
					position: 0,
					name: String::new(),
					kind: 4,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
				}),
			);
		}
		assert_eq!(state.channels.len() + state.guilds.len(), MAX_NAV);
		assert_eq!(state.status, auth::Failure::Capacity.label());
	}

	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}
	pub(super) fn message(id: u64) -> Message {
		Message {
			reactions: Some(vec![]),
			id: Id(id),
			channel: Id(1),
			author: User {
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			content: "Synthetic history".into(),
			edited: false,
			edited_at: None,
			revision: 0,
			nonce: None,
			reply_to: None,
			kind: 0,
			reply_deleted: false,
			forwarded: false,
			unsupported: false,
			extra_content: Default::default(),
			embeds: vec![],
			attachments: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: Vec::new(),
			embeds_suppressed: false,
		}
	}
	#[test]
	fn deleted_reading_rows_keep_pagination_and_release_reply_targets() {
		let mut state = State {
			user: Some(message(1).author),
			gateway_connected: true,
			auth: auth::AuthState::Authenticated,
			selected: Some(Id(1)),
			channels: vec![Channel {
				id: Id(1),
				guild: None,
				parent_id: None,
				kind: 1,
				position: 0,
				name: "Synthetic DM".into(),
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..State::default()
		};
		state.drafts.insert(Id(1), "unsent draft".into());
		state.history(None);
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: (100..150).map(message).collect(),
			},
		);
		let positions = state.timeline.row_ids().collect::<Vec<_>>();
		state.reply = Some(Id(100));
		apply(
			&mut state,
			Event::Delete {
				channel: Id(2),
				id: Id(100),
			},
		);
		assert_eq!(state.reply, Some(Id(100)));
		assert!(state.timeline.get(Id(100)).is_some());
		apply(
			&mut state,
			Event::Delete {
				channel: Id(1),
				id: Id(100),
			},
		);
		assert_eq!(state.reply, None);
		state.reply = Some(Id(101));
		let mut ids: Vec<_> = (101..150).map(Id).collect();
		ids.extend([Id(101), Id(999)]); // Duplicate and unknown IDs create no extra rows.
		apply(
			&mut state,
			Event::DeleteBulk {
				channel: Id(1),
				ids,
			},
		);
		assert_eq!(state.reply, None);
		assert_eq!(state.timeline.row_ids().collect::<Vec<_>>(), positions);
		assert!(state.timeline.is_empty());
		assert_eq!(state.timeline.bytes(), 0);
		assert!(state.can_load_older());
		assert!(!state.can_edit(Id(1), Id(100)));
		assert!(matches!(
			state.older_history(),
			Some(Command::History {
				before: Some(Id(100)),
				..
			})
		));
		let request = state.request;
		apply(
			&mut state,
			Event::SendResult {
				nonce: "synthetic-late".into(),
				result: Ok(message(100)),
			},
		);
		assert!(state.timeline.get(Id(100)).is_none());
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: true,
				messages: vec![message(99)],
			},
		);
		assert_eq!(state.timeline.len(), 1);
		assert_eq!(state.timeline.row_count(), 51);
		assert_eq!(state.timeline.row_ids().next(), Some(Id(99)));
		state.history(None);
		let request = state.request;
		apply(
			&mut state,
			Event::Delete {
				channel: Id(1),
				id: Id(99),
			},
		);
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(99), message(150)],
			},
		);
		assert_eq!(
			state.timeline.row_ids().collect::<Vec<_>>(),
			[Id(99), Id(150)]
		);
		assert!(state.timeline.get(Id(99)).is_none());
		state.history(None);
		let request = state.request;
		apply(&mut state, Event::Resync);
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(99)],
			},
		);
		assert_eq!(state.timeline.row_count(), 0);
		assert_eq!(state.drafts[&Id(1)], "unsent draft");
	}
	#[test]
	fn members_reject_late_requests_and_permission_invalidations() {
		let user = message(1).author;
		let mut state = State {
			user: Some(user.clone()),
			gateway_connected: true,
			auth: auth::AuthState::Authenticated,
			channels: vec![Channel {
				last_message: None,
				id: Id(1),
				guild: None,
				parent_id: None,
				position: 0,
				name: "DM".into(),
				kind: 1,
				recipients: vec![user.clone()],
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..State::default()
		};
		state.select(Id(1));
		state.request_members();
		assert_eq!(state.members.as_ref().unwrap().rows.len(), 1);
		let previous = state.members.clone().unwrap();
		state.close_members();
		apply(&mut state, Event::Members(previous.clone()));
		assert!(state.members.is_none());
		state.request_members();
		apply(&mut state, Event::PermissionsChanged);
		apply(&mut state, Event::Members(previous));
		assert!(state.members.as_ref().unwrap().rows.is_empty());
		assert_eq!(
			state.members.as_ref().unwrap().freshness,
			Freshness::Unavailable
		);
		apply(&mut state, Event::Resumed);
		assert!(state.members.is_none());
		state.request_members();
		apply(
			&mut state,
			Event::RecipientAdded {
				channel: Id(1),
				user: User {
					id: Id(3),
					name: "Other".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				},
			},
		);
		assert_eq!(state.members.as_ref().unwrap().rows.len(), 2);
		apply(
			&mut state,
			Event::RecipientRemoved {
				channel: Id(1),
				user: Id(3),
			},
		);
		assert_eq!(state.members.as_ref().unwrap().rows.len(), 1);
	}
	#[test]
	fn channel_restoration_requires_known_guild_and_preserves_existing_patch_state() {
		let channel = Channel {
			id: Id(1),
			guild: Some(Id(10)),
			parent_id: None,
			kind: 0,
			name: "Original".into(),
			position: 7,
			recipients: vec![],
			last_message: Some(Id(80)),
			icon: None,
			member_list_id: Some("known-list".into()),
			message_count: None,
		};
		let mut state = State {
			user: Some(message(1).author),
			channels: vec![channel.clone()],
			guilds: vec![Guild {
				id: Id(10),
				name: "Synthetic".into(),
				icon: None,
				emojis: None,
			}],
			selected: Some(Id(1)),
			auth: auth::AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			..State::default()
		};
		grant_permissions(&mut state);
		let candidate = Channel {
			name: "Restored".into(),
			position: 0,
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
			..channel.clone()
		};
		apply(&mut state, Event::ChannelRestored(candidate.clone()));
		assert!(
			state.channels[0] == channel,
			"A candidate cannot replace a known channel's absent fields"
		);
		apply(
			&mut state,
			Event::ChannelChanged(ChannelPatch {
				icon: model::Patch::Absent,
				id: Id(1),
				name: Patch::Value("Renamed".into()),
				last_message: Patch::Absent,
				parent_id: Patch::Absent,
				position: Patch::Absent,
				kind: Patch::Absent,
				message_count: Patch::Absent,
			}),
		);
		assert_eq!(state.channels[0].name, "Renamed");
		assert_eq!(state.channels[0].last_message, Some(Id(80)));
		assert_eq!(
			state.channels[0].member_list_id.as_deref(),
			Some("known-list")
		);
		state.drafts.insert(Id(1), "Preserved draft".into());
		state.history(None);
		let old_request = state.request;
		apply(&mut state, Event::Unavailable(Id(1)));
		for invalid in [
			Channel {
				guild: None,
				..candidate.clone()
			},
			Channel {
				guild: Some(Id(99)),
				..candidate.clone()
			},
			Channel {
				guild: Some(Id(0)),
				..candidate.clone()
			},
			Channel {
				id: Id(0),
				..candidate.clone()
			},
			Channel {
				kind: 1,
				..candidate.clone()
			},
			Channel {
				kind: 10,
				..candidate.clone()
			},
			Channel {
				kind: 11,
				..candidate.clone()
			},
			Channel {
				kind: 12,
				..candidate.clone()
			},
			Channel {
				kind: 255,
				..candidate.clone()
			},
		] {
			apply(&mut state, Event::ChannelRestored(invalid));
			assert!(state.channels.is_empty());
		}
		apply(&mut state, Event::ChannelRestored(candidate.clone()));
		assert!(state.channels[0] == candidate);
		assert_eq!(state.freshness, Freshness::Unavailable);
		assert!(!state.history_pending);
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request: old_request,
				older: false,
				messages: vec![message(99)],
			},
		);
		assert!(state.timeline.is_empty());
		assert_eq!(state.drafts[&Id(1)], "Preserved draft");
		assert!(matches!(state.select(Id(1)), Some(Command::History { .. })));
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(100)],
			},
		);
		assert_eq!(state.freshness, Freshness::Fresh);
		assert!(state.timeline.get(Id(100)).is_some());
	}

	#[test]
	fn voice_navigation_survives_ready_but_revocation_ends_the_call_before_restoration() {
		let channel = Channel {
			id: Id(1),
			guild: Some(Id(10)),
			parent_id: None,
			kind: 2,
			name: "Synthetic voice".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
		};
		let guild = Guild {
			id: Id(10),
			name: "Synthetic".into(),
			icon: None,
			emojis: None,
		};
		let user = message(1).author;
		let mut state = State {
			user: Some(user.clone()),
			channels: vec![channel.clone()],
			guilds: vec![guild.clone()],
			selected: Some(Id(1)),
			auth: auth::AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		};
		grant_permissions(&mut state);
		assert!(state.start_call(Id(1), false).is_some());
		let permissions = model::permissions::Snapshot {
			guilds: state.permissions.guilds.values().cloned().collect(),
			channels: state.permissions.channels.values().cloned().collect(),
		};
		apply(
			&mut state,
			Event::Ready {
				permissions,
				user,
				guilds: vec![guild],
				channels: vec![channel.clone()],
			},
		);
		assert_eq!(
			state.selected,
			Some(Id(1)),
			"A visible guild voice selection survives READY"
		);
		assert!(!state.history_pending);
		state.voice.roster.push(voice::RosterEntry {
			guild: Id(10),
			channel: Id(1),
			member: None,
			participant: voice::Participant {
				user: Id(2),
				muted: false,
				deafened: false,
				server_muted: false,
				server_deafened: false,
				video: false,
				streaming: false,
			},
		});
		apply(&mut state, Event::Unavailable(Id(1)));
		assert!(state.voice.active.is_none());
		assert!(state.voice.roster.is_empty());
		assert!(!state.can_call(Id(1)));
		assert!(matches!(state.history(None), Command::CancelSearch));
		apply(&mut state, Event::ChannelRestored(channel));
		assert!(
			state.voice.active.is_none(),
			"Restoring metadata never rejoins a call"
		);
		assert!(state.can_call(Id(1)));
		assert!(
			state.select(Id(1)).is_none(),
			"Voice navigation does not request text history"
		);
		assert_eq!(state.selected, Some(Id(1)));
		assert!(!state.history_pending);
		assert!(state.start_call(Id(1), false).is_some());
	}

	#[test]
	fn ready_replacement_and_removed_reload_cannot_restore_unavailable_history() {
		let channel = |kind| Channel {
			id: Id(1),
			guild: None,
			parent_id: None,
			kind,
			name: "Synthetic".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
		};
		let user = || User {
			id: Id(2),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		for replacement in [None, Some(channel(4))] {
			let mut state = State {
				user: Some(user()),
				channels: vec![channel(1)],
				selected: Some(Id(1)),
				auth: auth::AuthState::Authenticated,
				gateway_connected: true,
				freshness: Freshness::Fresh,
				..State::default()
			};
			state.timeline.insert(message(80), true, false).unwrap();
			state.drafts.insert(Id(1), "Preserve this draft".into());
			state.reply = Some(Id(80));
			assert!(matches!(state.history(None), Command::History { .. }));
			let request = state.request;
			apply(
				&mut state,
				Event::Ready {
					permissions: model::permissions::Snapshot::default(),
					user: user(),
					guilds: vec![],
					channels: replacement.into_iter().collect(),
				},
			);
			assert!(state.selected.is_none());
			assert!(state.timeline.is_empty());
			assert!(state.reply.is_none());
			assert_eq!(state.freshness, Freshness::Unavailable);
			assert!(!state.history_pending);
			assert_ne!(state.request, request);
			assert_eq!(state.drafts[&Id(1)], "Preserve this draft");
			assert!(
				matches!(state.history(None), Command::CancelSearch),
				"Reload cannot schedule HTTP or a cache load for an unavailable channel"
			);
			apply(
				&mut state,
				Event::History {
					channel: Id(1),
					request,
					older: false,
					messages: vec![message(99)],
				},
			);
			apply(&mut state, Event::Message(message(100)));
			assert!(state.timeline.is_empty());
			assert!(!state.history_pending);
			assert_eq!(state.freshness, Freshness::Unavailable);
		}

		let mut state = State {
			user: Some(user()),
			channels: vec![channel(1)],
			selected: Some(Id(1)),
			auth: auth::AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			..State::default()
		};
		state.timeline.insert(message(80), true, false).unwrap();
		state.history(None);
		let old_request = state.request;
		apply(
			&mut state,
			Event::Ready {
				permissions: model::permissions::Snapshot::default(),
				user: user(),
				guilds: vec![],
				channels: vec![channel(1)],
			},
		);
		assert_eq!(state.selected, Some(Id(1)));
		assert_eq!(state.freshness, Freshness::Stale);
		assert!(!state.history_pending);
		assert_ne!(state.request, old_request);
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request: old_request,
				older: false,
				messages: vec![message(99)],
			},
		);
		assert!(state.timeline.get(Id(99)).is_none());
		assert_eq!(state.freshness, Freshness::Stale);
		let Command::History { request, .. } = state.history(None) else {
			panic!()
		};
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(100)],
			},
		);
		assert_eq!(state.freshness, Freshness::Fresh);
		assert!(state.timeline.get(Id(100)).is_some());
		apply(&mut state, Event::Unavailable(Id(1)));
		assert_eq!(
			state.selected,
			Some(Id(1)),
			"The existing unavailable state may retain its old selection"
		);
		assert!(matches!(state.history(None), Command::CancelSearch));
		assert!(!state.history_pending);
		assert!(state.timeline.is_empty());
		assert_eq!(state.freshness, Freshness::Unavailable);
		assert!(matches!(
			State::default().history(None),
			Command::CancelSearch
		));
	}

	#[test]
	fn history_is_scoped_bounded_and_cannot_restore_freshness_after_disconnect() {
		let mut state = State::default();
		apply(
			&mut state,
			Event::Ready {
				permissions: model::permissions::Snapshot::default(),
				user: User {
					id: Id(2),
					name: "Synthetic".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				},
				guilds: vec![],
				channels: vec![Channel {
					last_message: None,
					id: Id(1),
					guild: None,
					parent_id: None,
					position: 0,
					name: "Synthetic".into(),
					kind: 1,
					recipients: vec![],
					icon: None,
					member_list_id: None,
					message_count: None,
				}],
			},
		);
		state.select(Id(1));
		let old_request = state.request;
		apply(&mut state, Event::Disconnected);
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request: old_request,
				older: false,
				messages: vec![message(99)],
			},
		);
		assert_eq!(state.freshness, Freshness::Stale);
		assert!(state.timeline.is_empty());
		assert!(!state.can_load_older());

		apply(&mut state, Event::Resumed);
		state.history(None);
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: (51..101).map(message).collect(),
			},
		);
		assert_eq!(state.freshness, Freshness::Fresh);
		assert!(state.can_load_older());
		assert!(matches!(
			state.older_history(),
			Some(Command::History {
				before: Some(Id(51)),
				..
			})
		));
		assert!(state.older_history().is_none()); // no duplicate concurrent page
		let request = state.request;
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request: old_request,
				failure: auth::Failure::Forbidden,
			},
		);
		assert_eq!(state.freshness, Freshness::Loading);
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: true,
				messages: vec![message(50)],
			},
		);
		assert_eq!(state.timeline.len(), 51);
		assert!(!state.can_load_older()); // short final page
		apply(
			&mut state,
			Event::DeleteBulk {
				channel: Id(1),
				ids: vec![Id(50), Id(51)],
			},
		);
		assert_eq!(state.timeline.len(), 49);
		assert!(state.timeline.get(Id(50)).is_none());

		state.history(None);
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: false,
				messages: vec![message(100)],
			},
		);
		assert_eq!(state.timeline.len(), 1); // old records absent from fresh page disappear
		state.history(Some(Id(100)));
		let request = state.request;
		apply(
			&mut state,
			Event::History {
				channel: Id(1),
				request,
				older: true,
				messages: vec![message(100)],
			},
		);
		assert_eq!(state.freshness, Freshness::Stale); // before boundary cannot repeat itself
		state.history(None);
		let request = state.request;
		apply(
			&mut state,
			Event::HistoryFailed {
				channel: Id(1),
				request,
				failure: auth::Failure::Forbidden,
			},
		);
		assert_eq!(state.freshness, Freshness::Unavailable);
		assert!(state.timeline.is_empty());
		apply(&mut state, Event::Message(message(200)));
		assert!(state.timeline.is_empty()); // queued live content cannot undo revocation
	}
}
