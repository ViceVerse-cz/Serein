//! Explicit account actions; one pending write, never an automatic retry.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::Id;
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_RELATIONSHIPS: usize = 4000;
pub const MAX_RELATIONSHIP_BYTES: usize = 128 * 1024;
pub const MAX_FRIEND_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RESTRICTED_BYTES: usize = 2 * 1024 * 1024;

fn replace_id_set(
	dest: &mut BTreeSet<Id>,
	entries: Option<Vec<Id>>,
	exclusive: Option<&mut BTreeSet<Id>>,
	capacity: &'static str,
	invalid: &'static str,
) -> Result<(), &'static str> {
	let Some(entries) = entries else {
		dest.clear();
		return Ok(());
	};
	if entries.len() > MAX_RELATIONSHIPS
		|| entries.capacity() * size_of::<Id>() > MAX_RELATIONSHIP_BYTES
	{
		return Err(capacity);
	}
	let mut next = BTreeSet::new();
	for id in entries {
		if id.0 == 0 || !next.insert(id) {
			return Err(invalid);
		}
	}
	if let Some(exclusive) = exclusive {
		for id in &next {
			exclusive.remove(id);
		}
	}
	*dest = next;
	Ok(())
}

#[derive(Clone, PartialEq, Eq)]
pub enum Action {
	LoadNote(Id),
	Note { user: Id, text: String },
	Nickname { user: Id, text: String },
	AddFriend { username: String },
	ResolveFriend { user: Id, accept: bool },
	ProfileFriend { user: Id, friend: bool },
	OpenDm(Id),
	CloseDm(Id),
	Block { user: Id, blocked: bool },
	Mute { channel: Id, muted: bool },
}
impl std::fmt::Debug for Action {
	/// Redacted debug output; never prints note, nickname or username text.
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("UserAction")
			.field("kind", &std::mem::discriminant(self))
			.finish_non_exhaustive()
	}
}
impl Action {
	pub fn completion_label(&self) -> &'static str {
		match self {
			Self::LoadNote(_) => "Note loaded",
			Self::OpenDm(_) => "Direct message was not confirmed; try opening it again",
			Self::Note { .. } => "Note saved",
			Self::Nickname { .. } => "Nickname saved",
			Self::AddFriend { .. } => "Friend request sent · waiting for service update",
			Self::ProfileFriend { friend: true, .. } => "Friend request sent",
			Self::ProfileFriend { friend: false, .. } => "Friend removed",
			Self::ResolveFriend { accept: true, .. } => "Friend request accepted",
			Self::ResolveFriend { accept: false, .. } => "Friend request removed",
			Self::CloseDm(_) => "DM closed · messages and drafts were not deleted",
			Self::Block { blocked: true, .. } => "User blocked",
			Self::Block { blocked: false, .. } => "User unblocked",
			Self::Mute { muted: true, .. } => {
				"Conversation notifications muted until you turn them back on"
			}
			Self::Mute { muted: false, .. } => "Conversation notifications unmuted",
		}
	}
}

/// Friendship-establishing writes can require a user-solved captcha; removals do not.
pub fn establishes_friendship(action: &Action) -> bool {
	matches!(action, Action::AddFriend { .. })
		|| matches!(action, Action::ResolveFriend { accept: true, .. })
		|| matches!(action, Action::ProfileFriend { friend: true, .. })
}
/// The challenge identity for a friendship write, if it can require one.
pub fn challenge_target(action: &Action) -> Option<crate::captcha::Target> {
	match action {
		Action::AddFriend { username } => Some(crate::captcha::Target::Username {
			username: username.clone(),
		}),
		Action::ResolveFriend { user, accept: true }
		| Action::ProfileFriend { user, friend: true } => {
			Some(crate::captcha::Target::Friend { user: *user })
		}
		_ => None,
	}
}

pub enum Event {
	DmOpened {
		user: Id,
		request: u64,
		result: Result<Box<model::Channel>, Failure>,
	},
	NoteChanged {
		user: Id,
		text: String,
	},
	NoteLoaded {
		user: Id,
		request: u64,
		result: Result<String, Failure>,
	},
	Nicknames(Vec<(Id, String)>),
	Nickname {
		user: Id,
		text: String,
	},
	Requests(Option<Vec<(model::User, String, bool)>>),
	Request {
		user: Id,
		incoming: Option<bool>,
		profile: Option<(model::User, String)>,
	},
	FriendProfile((model::User, String)),
	Friends(Option<Vec<(model::User, String)>>),
	Friend {
		user: Id,
		friend: bool,
		profile: Option<(model::User, String)>,
	},
	Restrictions(Option<Vec<(model::User, String, bool)>>),
	Restriction {
		user: Id,
		ignored: Option<bool>,
		profile: Option<(model::User, String)>,
	},
	Relationships(Option<Vec<(Id, bool)>>),
	Relationship {
		user: Id,
		blocked: bool,
	},
	MessageRequests(Option<Vec<Id>>),
	MessageRequest {
		channel: Id,
		pending: bool,
	},
	MessageSpams(Option<Vec<Id>>),
	MessageSpam {
		channel: Id,
		spam: bool,
	},
	RequestSpams(Option<Vec<Id>>),
	RequestSpam {
		user: Id,
		spam: bool,
	},
	Written {
		action: Action,
		request: u64,
		result: Result<(), Failure>,
	},
	Challenge {
		action: Action,
		request: u64,
		challenge: Box<crate::captcha::Challenge>,
	},
}
#[derive(Default)]
pub struct Actions {
	note: Option<(Id, String)>,
	nicknames: BTreeMap<Id, String>,
	requests: BTreeMap<Id, (model::User, String, bool)>,
	requests_known: bool,
	last_requested: Option<String>,
	friends: BTreeMap<Id, (model::User, String)>,
	friends_known: bool,
	restricted: BTreeMap<Id, (model::User, String, bool)>,
	restricted_known: bool,
	relationships: BTreeMap<Id, bool>,
	message_requests: BTreeSet<Id>,
	spam_directs: BTreeSet<Id>,
	spam_requests: BTreeSet<Id>,
	known: bool,
	view: u64,
	sequence: u64,
	pending: Option<(Action, u64, bool)>,
	challenge: Option<(std::time::Instant, crate::captcha::Challenge)>,
	dm_origin: Option<(Option<Id>, u64)>,
	opened_dm: Option<(Id, Id)>,
	status: Option<&'static str>,
}
impl Actions {
	pub(crate) fn reset(&mut self) {
		*self = Self {
			sequence: self.sequence,
			view: self.view.wrapping_add(1),
			..Self::default()
		};
	}
	fn bump_view(&mut self) {
		self.view = self.view.wrapping_add(1);
	}
}
impl State {
	/// Changes whenever friend filtering or ordering may change, including optimistic blocks.
	pub fn relationship_view(&self) -> u64 {
		self.user_actions.view
	}
	pub fn user_note(&self, user: Id) -> Option<&str> {
		self.user_actions
			.note
			.as_ref()
			.filter(|(id, _)| *id == user)
			.map(|(_, text)| text.as_str())
	}
	pub fn friend_nickname(&self, user: Id) -> Option<&str> {
		self.user_actions.nicknames.get(&user).map(String::as_str)
	}
	pub fn user_display_name<'a>(&'a self, user: &'a model::User) -> &'a str {
		self.friend_nickname(user.id).unwrap_or(&user.name)
	}
	pub fn message_author_name<'a>(&'a self, message: &'a model::Message) -> &'a str {
		if message.author.webhook {
			return &message.author.name;
		}
		if let Some(guild) = self
			.channel(message.channel)
			.and_then(|channel| channel.guild)
		{
			let member = self
				.members
				.as_ref()
				.filter(|list| list.guild == Some(guild))
				.and_then(|list| {
					list.slots
						.iter()
						.flatten()
						.filter_map(|slot| match slot {
							model::MemberSlot::Person(m) => Some(m),
							_ => None,
						})
						.find(|m| m.user.id == message.author.id)
				});
			let nick = member
				.and_then(|m| m.nick.as_deref().filter(|nick| !nick.is_empty()))
				.or(message.author_nick.as_deref());
			if let Some(nick) = nick.filter(|nick| !nick.is_empty()) {
				return nick;
			}
		}
		self.user_display_name(&message.author)
	}
	pub fn conversation_name<'a>(&'a self, channel: &'a model::Channel) -> &'a str {
		if channel.kind == 1
			&& let Some(user) = channel.recipients.first()
		{
			self.friend_nickname(user.id).unwrap_or(&channel.name)
		} else {
			&channel.name
		}
	}
	pub fn load_user_note(&mut self, user: Id) -> Option<Command> {
		if user.0 == 0 || self.user_action_pending() {
			return None;
		}
		if self.user_note(user).is_none() {
			self.user_actions.note = None;
		}
		self.request_user_action(Action::LoadNote(user))
	}
	pub fn set_user_note(&mut self, user: Id, text: String) -> Option<Command> {
		if self.user_note(user).is_none() || !valid_personal_text(&text, false) {
			return None;
		}
		self.request_user_action(Action::Note {
			user,
			text: text.as_str().to_owned(),
		})
	}
	pub fn set_friend_nickname(&mut self, user: Id, text: String) -> Option<Command> {
		if !self.friends().any(|friend| friend.id == user) || !valid_personal_text(&text, true) {
			return None;
		}
		self.request_user_action(Action::Nickname {
			user,
			text: text.as_str().to_owned(),
		})
	}
	pub fn pending_friends(&self) -> impl Iterator<Item = &(model::User, String, bool)> {
		self.user_actions
			.requests
			.values()
			.filter(|(u, _, _)| self.user_blocked(u.id) == Some(false))
	}
	fn stranger_message_request(&self, channel: &model::Channel) -> bool {
		let mut other = false;
		for user in &channel.recipients {
			if self.user_actions.friends.contains_key(&user.id)
				|| self.user_blocked(user.id) == Some(true)
			{
				return false;
			}
			other = true;
		}
		other
	}
	pub fn home_request_parts(&self) -> (u32, u32) {
		let friends = self
			.pending_friends()
			.filter(|(user, _, incoming)| {
				*incoming && !self.user_actions.spam_requests.contains(&user.id)
			})
			.count();
		let messages = self
			.user_actions
			.message_requests
			.iter()
			.filter(|channel| {
				self.channel(**channel)
					.is_some_and(|channel| self.stranger_message_request(channel))
			})
			.count();
		(
			u32::try_from(friends).unwrap_or(u32::MAX),
			u32::try_from(messages).unwrap_or(u32::MAX),
		)
	}
	pub fn home_request_count(&self) -> u32 {
		let (friends, messages) = self.home_request_parts();
		friends.saturating_add(messages)
	}
	pub(crate) fn forget_direct_inbox(&mut self, channel: Id) {
		self.user_actions.message_requests.remove(&channel);
		self.user_actions.spam_directs.remove(&channel);
	}
	pub(crate) fn message_request_pending(&self, channel: Id) -> bool {
		self.user_actions.message_requests.contains(&channel)
	}
	pub fn spam_direct(&self, channel: Id) -> bool {
		self.user_actions.spam_directs.contains(&channel)
	}
	fn set_message_request(&mut self, channel: Id, pending: bool) -> Result<(), &'static str> {
		if channel.0 == 0 {
			return Err("Invalid message request");
		}
		if pending {
			if !self.user_actions.message_requests.contains(&channel)
				&& self.user_actions.message_requests.len() >= MAX_RELATIONSHIPS
			{
				return Err("Message requests exceed safe capacity");
			}
			self.user_actions.spam_directs.remove(&channel);
			self.user_actions.message_requests.insert(channel);
		} else {
			self.user_actions.message_requests.remove(&channel);
		}
		Ok(())
	}
	fn set_spam_request(&mut self, user: Id, spam: bool) -> Result<(), &'static str> {
		if user.0 == 0 {
			return Err("Invalid friend request");
		}
		if spam {
			if !self.user_actions.spam_requests.contains(&user)
				&& self.user_actions.spam_requests.len() >= MAX_RELATIONSHIPS
			{
				return Err("Friend requests exceed safe capacity");
			}
			self.user_actions.spam_requests.insert(user);
		} else {
			self.user_actions.spam_requests.remove(&user);
		}
		Ok(())
	}
	fn set_spam_direct(&mut self, channel: Id, spam: bool) -> Result<(), &'static str> {
		if channel.0 == 0 {
			return Err("Invalid message request");
		}
		if spam {
			if !self.user_actions.spam_directs.contains(&channel)
				&& self.user_actions.spam_directs.len() >= MAX_RELATIONSHIPS
			{
				return Err("Message requests exceed safe capacity");
			}
			self.user_actions.message_requests.remove(&channel);
			self.user_actions.spam_directs.insert(channel);
		} else {
			self.user_actions.spam_directs.remove(&channel);
		}
		Ok(())
	}
	pub fn friend_requests_known(&self) -> bool {
		self.user_actions.requests_known
	}
	pub fn add_friend(&mut self, username: &str) -> Option<Command> {
		if username.len() > 132 {
			self.user_actions.status = Some("Enter a Discord username of at most 32 characters.");
			return None;
		}
		let username = username.trim().trim_start_matches('@').to_ascii_lowercase();
		if !valid_username(&username) {
			self.user_actions.status =
				Some("Enter a Discord username: 2–32 letters, numbers, underscores or periods.");
			return None;
		}
		if self.user_actions.last_requested.as_deref() == Some(&username)
			|| self
				.user_actions
				.friends
				.values()
				.any(|(_, n)| n == &username)
			|| self
				.user_actions
				.requests
				.values()
				.any(|(_, n, _)| n == &username)
		{
			self.user_actions.status =
				Some("Already friends or a request is pending. Check the Pending tab.");
			return None;
		}
		self.request_user_action(Action::AddFriend { username })
	}
	pub fn add_profile_friend(&mut self, user: Id) -> Option<Command> {
		if !self.profile_friend_action_allowed(user)
			|| self.user_actions.friends.contains_key(&user)
			|| self.user_actions.requests.contains_key(&user)
		{
			return None;
		}
		self.request_user_action(Action::ProfileFriend { user, friend: true })
	}
	pub fn remove_friend(&mut self, user: Id) -> Option<Command> {
		if !self.profile_friend_action_allowed(user)
			|| !self.user_actions.friends.contains_key(&user)
		{
			return None;
		}
		self.request_user_action(Action::ProfileFriend {
			user,
			friend: false,
		})
	}
	fn profile_friend_action_allowed(&self, user: Id) -> bool {
		user.0 != 0
			&& self.user.as_ref().is_some_and(|owner| owner.id != user)
			&& self.friends_known()
			&& self.friend_requests_known()
			&& self.user_blocked(user) == Some(false)
	}
	/// Accepts or declines one incoming friend request.
	pub fn resolve_friend_request(&mut self, user: Id, accept: bool) -> Option<Command> {
		let (_, _, incoming) = self.user_actions.requests.get(&user)?;
		if (accept && !incoming) || self.user_blocked(user) != Some(false) {
			return None;
		}
		self.request_user_action(Action::ResolveFriend { user, accept })
	}
	/// The one pending friendship write that is waiting on a user-solved challenge.
	pub fn friend_challenge(&self) -> Option<(u64, &crate::captcha::Challenge)> {
		let (action, request, _) = self.user_actions.pending.as_ref()?;
		if !establishes_friendship(action) {
			return None;
		}
		let (at, challenge) = self.user_actions.challenge.as_ref()?;
		(at.elapsed() < crate::captcha::LIFETIME).then_some((*request, challenge))
	}
	/// Builds the one explicit retry that resumes a solved friendship challenge.
	pub fn resume_friend_challenge(
		&mut self,
		request: u64,
		solution: crate::captcha::Solution,
	) -> Option<Command> {
		if self.demo || !self.gateway_connected || self.auth != AuthState::Authenticated {
			return None;
		}
		if self.friend_challenge()?.0 != request {
			return None;
		}
		let action = match self.user_actions.pending.as_ref() {
			Some((action, _, _)) => action.clone(),
			_ => return None,
		};
		let target = challenge_target(&action)?;
		let (at, challenge) = self.user_actions.challenge.take()?;
		// The new identity makes a duplicated response to the first attempt stale.
		self.user_actions.sequence = self.user_actions.sequence.wrapping_add(1);
		let request = self.user_actions.sequence;
		self.user_actions.pending = Some((action.clone(), request, false));
		Some(Command::UserAction {
			action,
			request,
			captcha: Some(Box::new(crate::captcha::Retry {
				target,
				request,
				challenge,
				solution,
				expires: at + crate::captcha::LIFETIME,
			})),
		})
	}
	/// Cancels the pending friendship challenge and releases its write.
	pub fn cancel_friend_challenge(&mut self, request: u64) {
		if self
			.friend_challenge()
			.is_some_and(|(pending, _)| pending == request)
		{
			self.user_actions.challenge = None;
			if let Some((action, _, _)) = self.user_actions.pending.take()
				&& matches!(action, Action::Block { .. })
			{
				self.user_actions.bump_view();
			}
			self.user_actions.status =
				Some("Verification cancelled; the friend request was not sent.");
			self.status = self.user_actions.status.unwrap();
		}
	}
	/// Releases a friendship challenge that outlived its five-minute lifetime.
	pub(crate) fn expire_friend_challenge(&mut self) {
		if self
			.user_actions
			.challenge
			.as_ref()
			.is_some_and(|(at, _)| at.elapsed() >= crate::captcha::LIFETIME)
		{
			self.user_actions.challenge = None;
			self.user_actions.pending = None;
			self.user_actions.status = Some("Verification expired; send the friend request again.");
			self.status = self.user_actions.status.unwrap();
		}
	}
	/// Visible friends, filtered by the current relationship snapshot.
	pub fn friends(&self) -> impl Iterator<Item = &model::User> {
		self.user_actions
			.friends
			.values()
			.map(|(user, _)| user)
			.filter(|u| self.user_blocked(u.id) == Some(false))
	}
	pub fn friend(&self, user: Id) -> Option<&model::User> {
		self.user_actions
			.friends
			.get(&user)
			.map(|(user, _)| user)
			.filter(|user| self.user_blocked(user.id) == Some(false))
	}
	pub fn friends_known(&self) -> bool {
		self.user_actions.friends_known
	}
	pub fn restricted_users(&self) -> impl Iterator<Item = &(model::User, String, bool)> {
		self.user_actions.restricted.values()
	}
	pub fn restricted_user(&self, user: Id) -> Option<&(model::User, String, bool)> {
		self.user_actions.restricted.get(&user)
	}
	pub fn restricted_users_known(&self) -> bool {
		self.user_actions.restricted_known
	}
	pub fn friend_username(&self, user: Id) -> Option<&str> {
		self.user_actions
			.friends
			.get(&user)
			.map(|(_, name)| name.as_str())
	}
	pub fn user_blocked(&self, user: Id) -> Option<bool> {
		if let Some((
			Action::Block {
				user: target,
				blocked,
			},
			_,
			false,
		)) = &self.user_actions.pending
			&& *target == user
		{
			return Some(*blocked);
		}
		self.user_actions
			.relationships
			.get(&user)
			.copied()
			.or_else(|| (self.demo || self.user_actions.known).then_some(false))
	}
	pub(crate) fn pending_dm_muted(&self, channel: Id) -> Option<bool> {
		match &self.user_actions.pending {
			Some((
				Action::Mute {
					channel: target,
					muted,
				},
				_,
				false,
			)) if *target == channel => Some(*muted),
			_ => None,
		}
	}
	pub fn user_action_pending(&self) -> bool {
		self.user_actions.pending.is_some()
	}
	pub fn user_action_status(&self) -> Option<&'static str> {
		self.user_actions.status
	}
	pub fn take_user_action_status(&mut self) -> Option<&'static str> {
		self.user_actions.status.take()
	}
	pub fn open_friend_dm(&mut self, user: Id) -> Option<Command> {
		if user.0 == 0 || self.user.as_ref().is_none_or(|owner| owner.id == user) {
			return None;
		}
		self.friend(user)?;
		if let Some(channel) = self.channels.iter().find(|channel| {
			channel.guild.is_none()
				&& channel.kind == 1
				&& channel.recipients.len() == 1
				&& channel.recipients[0].id == user
		}) {
			return self.select(channel.id);
		}
		let command = self.request_user_action(Action::OpenDm(user))?;
		self.user_actions.dm_origin = Some((self.selected, self.request));
		self.user_actions.opened_dm = None;
		self.status = "Opening direct message…";
		Some(command)
	}
	/// Consume one confirmed DM target without overriding navigation made during the request.
	pub fn select_opened_dm(&mut self) -> Option<Command> {
		let (channel, user) = self.user_actions.opened_dm.take()?;
		let origin = self.user_actions.dm_origin.take()?;
		if origin != (self.selected, self.request)
			|| self.friend(user).is_none()
			|| self.channel(channel).is_none_or(|known| {
				known.guild.is_some()
					|| known.kind != 1
					|| known.recipients.len() != 1
					|| known.recipients[0].id != user
			}) || (!self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected))
		{
			return None;
		}
		self.select(channel)
	}
	pub fn close_dm(&mut self, channel: Id) -> Option<Command> {
		if !self.is_one_to_one_dm(channel) {
			return None;
		}
		if self.server_invite_pending()
			|| self.pending.iter().any(|p| p.channel == channel)
			|| self
				.voice
				.active
				.as_ref()
				.is_some_and(|call| call.channel == channel)
		{
			self.user_actions.status =
				Some("Finish pending messages and leave the call before closing this DM");
			self.status = self.user_actions.status.unwrap();
			return None;
		}
		self.request_user_action(Action::CloseDm(channel))
	}
	pub fn set_user_blocked(&mut self, user: Id, blocked: bool) -> Option<Command> {
		if user.0 == 0
			|| self.user.as_ref().is_none_or(|owner| owner.id == user)
			|| (!blocked && self.user_blocked(user) != Some(true))
		{
			return None;
		}
		self.request_user_action(Action::Block { user, blocked })
	}
	pub fn set_dm_muted(&mut self, channel: Id, muted: bool) -> Option<Command> {
		if !self.is_one_to_one_dm(channel) && !self.is_group_dm(channel) {
			return None;
		}
		self.request_user_action(Action::Mute { channel, muted })
	}
	fn is_one_to_one_dm(&self, channel: Id) -> bool {
		self.channel(channel)
			.is_some_and(|c| c.guild.is_none() && c.kind == 1 && c.recipients.len() == 1)
	}
	/// Queues one account write; only one may be pending at a time.
	fn request_user_action(&mut self, action: Action) -> Option<Command> {
		if self.user_action_pending() {
			return None;
		}
		if !self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected) {
			self.user_actions.status = Some("User actions unavailable while disconnected");
			self.status = self.user_actions.status.unwrap();
			return None;
		}
		self.user_actions.sequence = self.user_actions.sequence.wrapping_add(1);
		let request = self.user_actions.sequence;
		self.user_actions.pending = Some((action.clone(), request, false));
		if matches!(action, Action::Block { .. }) {
			self.user_actions.bump_view();
		}
		self.user_actions.status = None;
		Some(Command::UserAction {
			action,
			request,
			captcha: None,
		})
	}
	/// Aborts the pending account write and reports an unknown outcome.
	pub(crate) fn cancel_user_action(&mut self) {
		self.user_actions.dm_origin = None;
		self.user_actions.opened_dm = None;
		self.user_actions.challenge = None;
		if let Some((action, _, _)) = self.user_actions.pending.take() {
			if matches!(action, Action::Block { .. }) {
				self.user_actions.bump_view();
			}
			self.user_actions.status =
				Some("Outcome unknown · check the official client before retrying");
		}
	}
	pub(crate) fn observe_dm_reopened(&mut self, channel: Id) {
		if let Some((Action::CloseDm(target), _, observed)) = &mut self.user_actions.pending
			&& *target == channel
		{
			*observed = true;
		}
	}
	pub(crate) fn observe_dm_settings(&mut self, event: &crate::notifications::Event) {
		if let Some((Action::Mute { .. }, _, observed)) = &mut self.user_actions.pending {
			*observed |= match event {
				crate::notifications::Event::Invalidate => true,
				crate::notifications::Event::Settings { entries, replace } => {
					*replace || entries.iter().any(|s| s.guild.is_none())
				}
				_ => false,
			};
		}
	}
	/// Applies one account-action event to relationship and challenge state.
	pub(crate) fn apply_user_action(&mut self, event: Event) -> Result<(), &'static str> {
		// Bump before applying: invalid full snapshots can clear previously visible entries.
		if matches!(
			&event,
			Event::Nicknames(_)
				| Event::Nickname { .. }
				| Event::Friends(_)
				| Event::Friend { .. }
				| Event::Restrictions(_)
				| Event::Restriction { .. }
				| Event::Relationships(_)
		) {
			self.user_actions.bump_view();
		}
		match event {
			Event::DmOpened {
				user,
				request,
				result,
			} => {
				if !matches!(&self.user_actions.pending, Some((Action::OpenDm(id), sequence, _)) if *id == user && *sequence == request)
				{
					return Ok(());
				}
				let result = result.and_then(|channel| {
					if channel.id.0 == 0
						|| channel.guild.is_some()
						|| channel.kind != 1
						|| channel.recipients.len() != 1
						|| channel.recipients[0].id != user
						|| channel.bytes() > 64 * 1024
						|| self.channel(channel.id).is_some_and(|known| {
							known.guild.is_some()
								|| known.kind != 1 || known.recipients.len() != 1
								|| known.recipients[0].id != user
						}) {
						Err(Failure::Ambiguous)
					} else if self.friend(user).is_none() {
						Err(Failure::Forbidden)
					} else {
						Ok(channel)
					}
				});
				let channel = match result {
					Ok(channel) => channel,
					Err(failure) => {
						return self.apply_user_action(Event::Written {
							action: Action::OpenDm(user),
							request,
							result: Err(failure),
						});
					}
				};
				self.user_actions.pending = None;
				let id = channel.id;
				// Preserve newer Gateway metadata; otherwise use the normal navigation bounds.
				if self.channel(id).is_none() {
					self.apply(crate::Envelope {
						generation: self.generation,
						event: crate::Event::ChannelCreated(*channel),
					});
				}
				if self.channel(id).is_some() && self.user_actions.dm_origin.is_some() {
					self.user_actions.opened_dm = Some((id, user));
					self.user_actions.status = None;
					self.status = "Direct message opened";
				}
			}
			Event::NoteChanged { user, text } => {
				if user.0 == 0 || !valid_personal_text(&text, false) {
					return Err("Invalid note");
				}
				let mut active = self.user_note(user).is_some();
				if let Some((
					Action::LoadNote(target) | Action::Note { user: target, .. },
					_,
					observed,
				)) = &mut self.user_actions.pending
					&& *target == user
				{
					*observed = true;
					active = true;
				}
				if active {
					self.user_actions.note = Some((user, text.as_str().to_owned()));
				}
			}
			Event::NoteLoaded {
				user,
				request,
				result,
			} => {
				if !matches!(&self.user_actions.pending, Some((Action::LoadNote(id), sequence, _)) if *id == user && *sequence == request)
				{
					return Ok(());
				}
				match result {
					Ok(text) if valid_personal_text(&text, false) => {
						if !self
							.user_actions
							.pending
							.as_ref()
							.is_some_and(|(_, _, observed)| *observed)
						{
							self.user_actions.note = Some((user, text.as_str().to_owned()));
						}
						self.user_actions.pending = None;
						self.user_actions.status = None;
					}
					result => {
						return self.apply_user_action(Event::Written {
							action: Action::LoadNote(user),
							request,
							result: Err(result.err().unwrap_or(Failure::Protocol)),
						});
					}
				}
			}
			Event::Nicknames(entries) => {
				if entries.len() > MAX_RELATIONSHIPS
					|| entries
						.iter()
						.any(|(id, text)| id.0 == 0 || !valid_personal_text(text, true))
				{
					return Err("Invalid friend nicknames");
				}
				self.user_actions.nicknames.clear();
				for (user, text) in entries {
					self.apply_user_action(Event::Nickname { user, text })?;
				}
			}
			Event::Nickname { user, text } => {
				if user.0 == 0 || !valid_personal_text(&text, true) {
					return Err("Invalid friend nickname");
				}
				if let Some((Action::Nickname { user: target, .. }, _, observed)) =
					&mut self.user_actions.pending
					&& *target == user
				{
					*observed = true;
				}
				if text.is_empty() || !self.user_actions.friends.contains_key(&user) {
					self.user_actions.nicknames.remove(&user);
				} else {
					if !self.user_actions.nicknames.contains_key(&user)
						&& self.user_actions.nicknames.len() >= MAX_RELATIONSHIPS
					{
						return Err("Nickname capacity exceeded");
					}
					// At most 4000 tightly allocated 128-byte strings plus map overhead.
					self.user_actions
						.nicknames
						.insert(user, text.as_str().to_owned());
				}
			}
			Event::Requests(entries) => {
				if let Some((Action::ProfileFriend { .. }, _, observed)) =
					&mut self.user_actions.pending
				{
					*observed = true;
				}
				self.user_actions.requests.clear();
				self.user_actions.requests_known = false;
				if let Some(entries) = entries {
					if entries.len() > MAX_RELATIONSHIPS
						|| entries.capacity() * size_of::<(model::User, String, bool)>()
							+ entries
								.iter()
								.map(|(u, n, _)| u.heap_bytes() + n.capacity() + 64)
								.sum::<usize>() > MAX_FRIEND_BYTES
					{
						return Err("Friend requests exceed safe capacity");
					}
					for (user, name, incoming) in entries {
						if self.user_actions.requests.contains_key(&user.id) {
							return Err("Duplicate friend request");
						}
						self.apply_user_action(Event::Request {
							user: user.id,
							incoming: Some(incoming),
							profile: Some((user, name)),
						})?;
					}
					self.user_actions.requests_known = true;
				}
			}
			Event::Request {
				user,
				incoming,
				profile,
			} => {
				if user.0 == 0 {
					return Err("Invalid friend request");
				}
				if let Some((
					Action::ResolveFriend { user: target, .. }
					| Action::ProfileFriend { user: target, .. },
					_,
					observed,
				)) = &mut self.user_actions.pending
					&& *target == user
				{
					*observed = true;
				}
				if let Some(incoming) = incoming {
					let profile = profile.or_else(|| {
						(!self.user_actions.requests.contains_key(&user)).then(|| {
							(
								model::User {
									id: user,
									name: "Unknown user".into(),
									avatar: None,
									discriminator: 0,
									primary_guild: None,
									webhook: false,
									kind: Default::default(),
								},
								format!("User ID: {user}"),
							)
						})
					});
					if let Some((record, name)) = profile {
						if user != record.id || !valid_friend(&record, &name) {
							return Err("Invalid friend request profile");
						}
						let entries = &mut self.user_actions.requests;
						let bytes = entries
							.iter()
							.filter(|(id, _)| **id != user)
							.map(|(_, (u, n, _))| {
								u.heap_bytes()
									+ n.capacity() + size_of::<(Id, model::User, String, bool)>()
									+ 64
							})
							.sum::<usize>();
						if (!entries.contains_key(&user) && entries.len() >= MAX_RELATIONSHIPS)
							|| bytes
								+ record.heap_bytes() + name.capacity()
								+ size_of::<(Id, model::User, String, bool)>()
								+ 64 > MAX_FRIEND_BYTES
						{
							return Err("Friend requests exceed safe capacity");
						}
						entries.insert(user, (record, name, incoming));
					} else if let Some(entry) = self.user_actions.requests.get_mut(&user) {
						entry.2 = incoming;
					}
				} else {
					self.user_actions.spam_requests.remove(&user);
					if let Some((_, name, _)) = self.user_actions.requests.remove(&user)
						&& self.user_actions.last_requested.as_deref() == Some(&name)
					{
						self.user_actions.last_requested = None;
					}
				}
			}
			Event::FriendProfile(profile) => {
				if let Some((_, _, ignored)) = self.user_actions.restricted.get(&profile.0.id) {
					let ignored = *ignored;
					let user = profile.0.id;
					self.apply_user_action(Event::Restriction {
						user,
						ignored: Some(ignored),
						profile: Some(profile.clone()),
					})?;
				}
				if self.user_actions.friends.contains_key(&profile.0.id) {
					let user = profile.0.id;
					self.apply_user_action(Event::Friend {
						user,
						friend: true,
						profile: Some(profile),
					})?;
					return Ok(());
				}
			}
			Event::Friends(entries) => {
				if let Some((Action::ProfileFriend { .. }, _, observed)) =
					&mut self.user_actions.pending
				{
					*observed = true;
				}
				self.user_actions.friends.clear();
				self.user_actions.friends_known = false;
				if let Some(entries) = entries {
					if entries.len() > MAX_RELATIONSHIPS
						|| entries.capacity() * size_of::<(model::User, String)>()
							+ entries
								.iter()
								.map(|(u, n)| u.heap_bytes() + n.capacity() + 64)
								.sum::<usize>() > MAX_FRIEND_BYTES
					{
						return Err("Friends exceed safe capacity");
					}
					for (user, name) in entries {
						if !valid_friend(&user, &name)
							|| self
								.user_actions
								.friends
								.insert(user.id, (user, name))
								.is_some()
						{
							self.user_actions.friends.clear();
							return Err("Friends contain invalid or duplicate users");
						}
					}
					self.user_actions.friends_known = true;
				}
			}
			Event::Friend {
				user,
				friend,
				profile,
			} => {
				if let Some((Action::ProfileFriend { user: target, .. }, _, observed)) =
					&mut self.user_actions.pending
					&& *target == user
				{
					*observed = true;
				}
				let profile = profile.or_else(|| {
					self.user_actions
						.requests
						.get(&user)
						.map(|(u, n, _)| (u.clone(), n.clone()))
				});
				if !friend {
					if let Some((Action::Nickname { user: target, .. }, _, observed)) =
						&mut self.user_actions.pending
						&& *target == user
					{
						*observed = true;
					}
					self.user_actions.friends.remove(&user);
					self.user_actions.nicknames.remove(&user);
				} else if let Some((record, name)) = profile {
					let entries = &mut self.user_actions.friends;
					if user != record.id || !valid_friend(&record, &name) {
						return Err("Invalid friend update");
					}
					let old = entries.get(&user).map_or(0, |(u, n)| {
						u.heap_bytes() + n.capacity() + size_of::<(Id, model::User, String)>() + 64
					});
					let bytes: usize = entries
						.values()
						.map(|(u, n)| {
							u.heap_bytes()
								+ n.capacity() + size_of::<(Id, model::User, String)>()
								+ 64
						})
						.sum();
					if (!entries.contains_key(&user) && entries.len() >= MAX_RELATIONSHIPS)
						|| bytes - old
							+ record.heap_bytes() + name.capacity()
							+ size_of::<(Id, model::User, String)>()
							+ 64 > MAX_FRIEND_BYTES
					{
						return Err("Friends exceed safe capacity");
					}
					entries.insert(user, (record, name));
				}
			}
			Event::Restrictions(entries) => {
				self.user_actions.restricted.clear();
				self.user_actions.restricted_known = false;
				if let Some(entries) = entries {
					if entries.len() > MAX_RELATIONSHIPS
						|| entries.capacity() * size_of::<(model::User, String, bool)>()
							+ entries
								.iter()
								.map(|(u, n, _)| u.heap_bytes() + n.capacity() + 64)
								.sum::<usize>() > MAX_RESTRICTED_BYTES
					{
						return Err("Restricted users exceed safe capacity");
					}
					for (user, name, ignored) in entries {
						if !valid_friend(&user, &name)
							|| self
								.user_actions
								.restricted
								.insert(user.id, (user, name, ignored))
								.is_some()
						{
							self.user_actions.restricted.clear();
							return Err("Restricted users contain invalid or duplicate profiles");
						}
					}
					self.user_actions.restricted_known = true;
				}
			}
			Event::Restriction {
				user,
				ignored,
				profile,
			} => {
				let Some(ignored) = ignored else {
					self.user_actions.restricted.remove(&user);
					return Ok(());
				};
				if let Some((record, name)) = profile {
					if user != record.id || !valid_friend(&record, &name) {
						return Err("Invalid restricted user update");
					}
					let entries = &mut self.user_actions.restricted;
					let old = entries.get(&user).map_or(0, |(u, n, _)| {
						u.heap_bytes()
							+ n.capacity() + size_of::<(Id, model::User, String, bool)>()
							+ 64
					});
					let bytes: usize = entries
						.values()
						.map(|(u, n, _)| {
							u.heap_bytes()
								+ n.capacity() + size_of::<(Id, model::User, String, bool)>()
								+ 64
						})
						.sum();
					if (!entries.contains_key(&user) && entries.len() >= MAX_RELATIONSHIPS)
						|| bytes - old
							+ record.heap_bytes() + name.capacity()
							+ size_of::<(Id, model::User, String, bool)>()
							+ 64 > MAX_RESTRICTED_BYTES
					{
						return Err("Restricted users exceed safe capacity");
					}
					entries.insert(user, (record, name, ignored));
				} else if let Some(entry) = self.user_actions.restricted.get_mut(&user) {
					entry.2 = ignored;
				}
			}
			Event::Relationships(entries) => {
				if let Some((Action::Block { .. }, _, observed)) = &mut self.user_actions.pending {
					*observed = true;
				}
				self.user_actions.relationships.clear();
				self.user_actions.known = false;
				if let Some(entries) = entries {
					if entries.len() > MAX_RELATIONSHIPS
						|| entries.capacity() * size_of::<(Id, bool)>() > MAX_RELATIONSHIP_BYTES
					{
						return Err("Relationships exceed safe capacity");
					}
					for (user, blocked) in entries {
						if user.0 == 0
							|| self
								.user_actions
								.relationships
								.insert(user, blocked)
								.is_some()
						{
							self.user_actions.relationships.clear();
							return Err("Relationships contain invalid or duplicate users");
						}
					}
					self.user_actions.known = true;
				}
			}
			Event::MessageRequests(entries) => {
				replace_id_set(
					&mut self.user_actions.message_requests,
					entries,
					Some(&mut self.user_actions.spam_directs),
					"Message requests exceed safe capacity",
					"Message requests contain invalid or duplicate channels",
				)?;
			}
			Event::MessageRequest { channel, pending } => {
				self.set_message_request(channel, pending)?;
			}
			Event::MessageSpams(entries) => {
				replace_id_set(
					&mut self.user_actions.spam_directs,
					entries,
					Some(&mut self.user_actions.message_requests),
					"Message requests exceed safe capacity",
					"Message requests contain invalid or duplicate channels",
				)?;
			}
			Event::MessageSpam { channel, spam } => {
				self.set_spam_direct(channel, spam)?;
			}
			Event::RequestSpams(entries) => {
				replace_id_set(
					&mut self.user_actions.spam_requests,
					entries,
					None,
					"Friend requests exceed safe capacity",
					"Friend requests contain invalid or duplicate users",
				)?;
			}
			Event::RequestSpam { user, spam } => {
				self.set_spam_request(user, spam)?;
			}
			Event::Relationship { user, blocked } => {
				self.store_relationship(user, blocked)?;
				if let Some((
					Action::Block { user: target, .. } | Action::ProfileFriend { user: target, .. },
					_,
					observed,
				)) = &mut self.user_actions.pending
					&& *target == user
				{
					*observed = true;
				}
			}
			Event::Challenge {
				action,
				request,
				challenge,
			} => {
				let Some((pending, sequence, _)) = &self.user_actions.pending else {
					return Ok(());
				};
				if *pending != action || *sequence != request || !establishes_friendship(&action) {
					return Ok(());
				}
				self.user_actions.status = None;
				self.user_actions.challenge = Some((std::time::Instant::now(), *challenge));
			}
			Event::Written {
				action,
				request,
				result,
			} => {
				let Some((pending, sequence, observed)) = &self.user_actions.pending else {
					return Ok(());
				};
				if *pending != action || *sequence != request {
					return Ok(());
				}
				let observed = *observed;
				self.user_actions.pending = None;
				self.user_actions.challenge = None;
				if matches!(action, Action::OpenDm(_)) {
					self.user_actions.dm_origin = None;
				}
				if matches!(action, Action::Block { .. }) {
					self.user_actions.bump_view();
				}
				if let Err(failure) = result {
					if failure.ends_session() {
						self.status = failure.label();
						self.fail(failure);
					}
					return Ok(());
				}
				if !observed {
					match action {
						Action::LoadNote(_) | Action::OpenDm(_) => {}
						Action::Note { user, ref text } => {
							self.user_actions.note = Some((user, text.clone()))
						}
						Action::Nickname { user, ref text } => {
							self.apply_user_action(Event::Nickname {
								user,
								text: text.clone(),
							})?;
						}
						Action::AddFriend { ref username } => {
							self.user_actions.last_requested = Some(username.clone())
						}
						Action::ResolveFriend { user, accept } => {
							if let Some((record, name, _)) =
								self.user_actions.requests.remove(&user)
							{
								self.user_actions.last_requested = None;
								if accept {
									self.apply_user_action(Event::Friend {
										user,
										friend: true,
										profile: Some((record, name)),
									})?;
								}
							}
						}
						Action::ProfileFriend { user, friend } => {
							if friend {
								self.apply_user_action(Event::Request {
									user,
									incoming: Some(false),
									profile: None,
								})?;
							} else {
								self.apply_user_action(Event::Friend {
									user,
									friend: false,
									profile: None,
								})?;
							}
						}
						Action::CloseDm(channel) => {
							self.remove_channels(&std::collections::BTreeSet::from([channel]));
							if self.selected == Some(channel) {
								self.arrived_home();
							}
						}
						Action::Block { user, blocked } => {
							self.store_relationship(user, blocked)?
						}
						Action::Mute { channel, muted } => self.confirm_dm_muted(channel, muted)?,
					}
				}
			}
		}
		Ok(())
	}
	fn store_relationship(&mut self, user: Id, blocked: bool) -> Result<(), &'static str> {
		self.user_actions.bump_view();
		if blocked {
			self.user_actions.friends.remove(&user);
			self.user_actions.nicknames.remove(&user);
			self.user_actions.requests.remove(&user);
		}
		let entries = &mut self.user_actions.relationships;
		// Fixed-size IDs and booleans: <= 4000 entries and a conservative 32-byte entry estimate.
		if user.0 == 0
			|| (!entries.contains_key(&user)
				&& (entries.len() >= MAX_RELATIONSHIPS
					|| (entries.len() + 1) * 32 > MAX_RELATIONSHIP_BYTES))
		{
			return Err("Relationships exceed safe capacity or contain an invalid user");
		}
		entries.insert(user, blocked);
		self.read_state.activity.clear_notifications();
		Ok(())
	}
}

pub fn valid_personal_text(text: &str, nickname: bool) -> bool {
	let limit = if nickname { 32 } else { 256 };
	text.len() <= limit * 4
		&& text.chars().count() <= limit
		&& !text
			.chars()
			.any(|c| c.is_control() && (nickname || !matches!(c, '\n' | '\t')))
}
pub fn valid_username(name: &str) -> bool {
	(2..=32).contains(&name.len())
		&& !name.contains("..")
		&& name
			.bytes()
			.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.'))
}
fn valid_friend(user: &model::User, username: &str) -> bool {
	user.id.0 != 0
		&& !user.name.is_empty()
		&& user.name.len() <= 512
		&& !user.name.chars().any(char::is_control)
		&& !username.is_empty()
		&& username.len() <= 128
		&& !username.chars().any(char::is_control)
		&& user
			.avatar
			.as_ref()
			.is_none_or(|hash| model::valid_avatar_hash(hash))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event as CoreEvent};
	#[test]
	fn relationship_view_tracks_all_friend_inputs_and_failed_mutations() {
		let mut state = state();
		let mut user = state.channels[0].recipients[0].clone();
		let mut view = state.relationship_view();
		for event in [
			Event::Relationships(Some(vec![])),
			Event::Friends(Some(vec![(user.clone(), "friend".into())])),
			Event::Nickname {
				user: user.id,
				text: "Nickname".into(),
			},
			Event::Nicknames(vec![]),
			{
				user.name = "Changed display name".into();
				Event::FriendProfile((user.clone(), "changed_username".into()))
			},
		] {
			state.apply_user_action(event).unwrap();
			assert!(state.relationship_view() > view);
			view = state.relationship_view();
		}
		assert!(state.friend(user.id) == state.friends().next());
		assert_eq!(state.friend(user.id).unwrap().name, "Changed display name");
		assert!(state.friend(Id(999)).is_none());
		for cancelled in [false, true] {
			let command = state.set_user_blocked(user.id, true).unwrap();
			assert!(state.relationship_view() > view);
			assert!(state.friend(user.id).is_none());
			view = state.relationship_view();
			if cancelled {
				state.cancel_user_action();
			} else {
				finish(&mut state, command, Err(Failure::Forbidden));
			}
			assert!(state.relationship_view() > view);
			assert!(state.friend(user.id) == state.friends().next());
			assert!(state.friend(user.id).is_some());
			view = state.relationship_view();
		}
		// A rejected full snapshot clears existing values before reporting the error.
		assert!(
			state
				.apply_user_action(Event::Friends(Some(vec![
					(user.clone(), "friend".into()),
					(user.clone(), "duplicate".into()),
				])))
				.is_err()
		);
		assert!(state.relationship_view() > view);
		assert!(state.friend(user.id).is_none());
		state
			.apply_user_action(Event::Friends(Some(vec![(user.clone(), "friend".into())])))
			.unwrap();
		view = state.relationship_view();
		assert!(
			state
				.apply_user_action(Event::Relationships(Some(vec![(Id(0), false)])))
				.is_err()
		);
		assert!(state.relationship_view() > view);
		assert!(state.friend(user.id).is_none());
		state
			.apply_user_action(Event::Relationships(Some(vec![])))
			.unwrap();
		view = state.relationship_view();
		state
			.apply_user_action(Event::Relationship {
				user: user.id,
				blocked: true,
			})
			.unwrap();
		assert!(state.relationship_view() > view);
		assert!(state.friend(user.id).is_none());
		// A rejected block can remove a friend before the relationship capacity check.
		state.user_actions.relationships = (10..10 + MAX_RELATIONSHIPS as u64)
			.map(|id| (Id(id), false))
			.collect();
		state
			.apply_user_action(Event::Friends(Some(vec![(user.clone(), "friend".into())])))
			.unwrap();
		view = state.relationship_view();
		assert!(state.store_relationship(user.id, true).is_err());
		assert!(state.relationship_view() > view);
		assert!(state.friend(user.id).is_none());
	}

	#[test]
	fn relationship_view_survives_ready_and_rejects_stale_generation() {
		let mut state = state();
		let user = state.channels[0].recipients[0].clone();
		state
			.apply_user_action(Event::Relationships(Some(vec![])))
			.unwrap();
		state
			.apply_user_action(Event::Friends(Some(vec![(user.clone(), "friend".into())])))
			.unwrap();
		let view = state.relationship_view();
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Ready {
				permissions: Default::default(),
				user: state.user.clone().unwrap(),
				guilds: vec![],
				channels: state.channels.clone(),
			},
		});
		assert!(state.relationship_view() > view);
		assert!(state.friend(user.id).is_none());
		let generation = state.generation;
		state.logout();
		let view = state.relationship_view();
		state.apply(Envelope {
			generation,
			event: CoreEvent::UserAction(Event::Friends(Some(vec![(user, "stale".into())]))),
		});
		assert_eq!(state.relationship_view(), view);
		assert_eq!(state.friends().count(), 0);
	}
	#[test]
	fn personal_edits_are_bounded_confirmed_and_reconcile_newer_service_state() {
		let mut state = state();
		let user = state.channels[0].recipients[0].clone();
		assert!(
			state
				.set_user_note(user.id, "Must load first".into())
				.is_none()
		);
		assert!(
			state
				.set_friend_nickname(user.id, "Not a friend".into())
				.is_none()
		);
		let Command::UserAction { request, .. } = state.load_user_note(user.id).unwrap() else {
			panic!()
		};
		state
			.apply_user_action(Event::NoteChanged {
				user: user.id,
				text: "Newest note".into(),
			})
			.unwrap();
		state
			.apply_user_action(Event::NoteLoaded {
				user: user.id,
				request,
				result: Ok("Stale read".into()),
			})
			.unwrap();
		assert_eq!(state.user_note(user.id), Some("Newest note"));
		let write = state
			.set_user_note(user.id, "Private draft".into())
			.unwrap();
		finish(&mut state, write, Err(Failure::Forbidden));
		assert_eq!(state.user_note(user.id), Some("Newest note"));
		let write = state.set_user_note(user.id, String::new()).unwrap();
		finish(&mut state, write, Ok(()));
		assert_eq!(state.user_note(user.id), Some(""));
		assert!(!valid_personal_text(&"🌙".repeat(257), false));
		assert!(!valid_personal_text("bad\nname", true));
		assert!(valid_personal_text(&"🌙".repeat(32), true));
		state
			.apply_user_action(Event::Relationships(Some(vec![])))
			.unwrap();
		state
			.apply_user_action(Event::Friends(Some(vec![(
				user.clone(),
				"synthetic".into(),
			)])))
			.unwrap();
		let write = state
			.set_friend_nickname(user.id, "Private name".into())
			.unwrap();
		finish(&mut state, write, Ok(()));
		assert_eq!(state.user_display_name(&user), "Private name");
		assert_eq!(state.conversation_name(&state.channels[0]), "Private name");
		let write = state
			.set_friend_nickname(user.id, "Late result".into())
			.unwrap();
		state
			.apply_user_action(Event::Nickname {
				user: user.id,
				text: "Newest name".into(),
			})
			.unwrap();
		finish(&mut state, write, Ok(()));
		assert_eq!(state.friend_nickname(user.id), Some("Newest name"));
		let write = state
			.set_friend_nickname(user.id, "Removed friend".into())
			.unwrap();
		state
			.apply_user_action(Event::Friend {
				user: user.id,
				friend: false,
				profile: None,
			})
			.unwrap();
		finish(&mut state, write, Ok(()));
		assert_eq!(state.friend_nickname(user.id), None);
		assert!(
			!format!(
				"{:?}",
				Action::Note {
					user: user.id,
					text: "PRIVATE".into()
				}
			)
			.contains("PRIVATE")
		);
		state.logout();
		assert_eq!(state.user_note(user.id), None);
	}
	#[test]
	fn friend_requests_validate_bound_and_reconcile_without_duplicate_writes() {
		let mut state = state();
		let user = state.channels[0].recipients[0].clone();
		state
			.apply_user_action(Event::Relationships(Some(vec![])))
			.unwrap();
		state
			.apply_user_action(Event::Requests(Some(vec![(
				user.clone(),
				"synthetic".into(),
				true,
			)])))
			.unwrap();
		assert_eq!(state.pending_friends().count(), 1);
		assert!(state.add_friend("../invalid").is_none());
		let send = state.add_friend(" @new_friend ").unwrap();
		assert!(state.add_friend("new_friend").is_none());
		assert!(state.resolve_friend_request(user.id, true).is_none());
		finish(&mut state, send, Ok(()));
		assert_eq!(state.user_action_status(), None);
		assert!(state.add_friend("new_friend").is_none());
		assert_eq!(
			state.pending_friends().count(),
			1,
			"no invented outgoing identity"
		);
		let rejected = state.add_friend("other_friend").unwrap();
		finish(&mut state, rejected, Err(Failure::Protocol));
		assert_eq!(state.user_action_status(), None);
		let accept = state.resolve_friend_request(user.id, true).unwrap();
		finish(&mut state, accept, Err(Failure::Forbidden));
		assert_eq!(state.pending_friends().count(), 1);
		let accept = state.resolve_friend_request(user.id, true).unwrap();
		state
			.apply_user_action(Event::Request {
				user: user.id,
				incoming: None,
				profile: None,
			})
			.unwrap();
		finish(&mut state, accept, Ok(()));
		assert_eq!(
			state.friends().count(),
			0,
			"newer Gateway removal wins over acceptance"
		);
		state
			.apply_user_action(Event::Request {
				user: user.id,
				incoming: Some(false),
				profile: Some((user.clone(), "synthetic".into())),
			})
			.unwrap();
		assert!(state.resolve_friend_request(user.id, true).is_none());
		let cancel = state.resolve_friend_request(user.id, false).unwrap();
		finish(&mut state, cancel, Ok(()));
		assert_eq!(state.user_action_status(), None);
		assert_eq!(state.pending_friends().count(), 0);
		state
			.apply_user_action(Event::Request {
				user: user.id,
				incoming: Some(true),
				profile: Some((user.clone(), "synthetic".into())),
			})
			.unwrap();
		let accept = state.resolve_friend_request(user.id, true).unwrap();
		state
			.apply_user_action(Event::Friend {
				user: user.id,
				friend: true,
				profile: None,
			})
			.unwrap();
		state
			.apply_user_action(Event::Request {
				user: user.id,
				incoming: None,
				profile: None,
			})
			.unwrap();
		assert_eq!(
			state.friends().count(),
			1,
			"acceptance without repeated metadata retains the request profile"
		);
		finish(&mut state, accept, Ok(()));
		assert_eq!(state.friends().count(), 1);
		assert_eq!(state.pending_friends().count(), 0);
		assert!(
			state
				.apply_user_action(Event::Requests(Some(vec![(
					user,
					"huge".repeat(MAX_FRIEND_BYTES),
					true
				)])))
				.is_err()
		);
		state.gateway_connected = false;
		assert!(state.add_friend("another").is_none());
		state.logout();
		assert!(!state.friend_requests_known());
		assert_eq!(state.pending_friends().count(), 0);
	}
	#[test]
	fn profile_friend_actions_require_known_state_and_reconcile_acknowledgements() {
		let mut state = state();
		let user = state.channels[0].recipients[0].clone();
		assert!(state.add_profile_friend(user.id).is_none());
		state
			.apply_user_action(Event::Friends(Some(vec![])))
			.unwrap();
		state
			.apply_user_action(Event::Requests(Some(vec![])))
			.unwrap();
		state
			.apply_user_action(Event::Relationships(Some(vec![])))
			.unwrap();
		assert!(state.add_profile_friend(Id(0)).is_none());
		assert!(state.add_profile_friend(Id(1)).is_none());
		let add = state.add_profile_friend(user.id).unwrap();
		assert!(state.add_profile_friend(user.id).is_none());
		assert_eq!(state.pending_friends().count(), 0);
		finish(&mut state, add, Err(Failure::Forbidden));
		assert_eq!(state.pending_friends().count(), 0);
		let add = state.add_profile_friend(user.id).unwrap();
		finish(&mut state, add, Ok(()));
		assert_eq!(
			state
				.pending_friends()
				.next()
				.map(|(u, _, incoming)| (u.id, *incoming)),
			Some((user.id, false))
		);
		assert!(state.add_profile_friend(user.id).is_none());
		assert!(state.remove_friend(user.id).is_none());
		state
			.apply_user_action(Event::Friend {
				user: user.id,
				friend: true,
				profile: Some((user.clone(), "synthetic".into())),
			})
			.unwrap();
		state
			.apply_user_action(Event::Request {
				user: user.id,
				incoming: None,
				profile: None,
			})
			.unwrap();
		let remove = state.remove_friend(user.id).unwrap();
		assert_eq!(state.friends().count(), 1);
		finish(&mut state, remove, Err(Failure::Forbidden));
		assert_eq!(state.friends().count(), 1);
		let remove = state.remove_friend(user.id).unwrap();
		finish(&mut state, remove, Ok(()));
		assert_eq!(state.friends().count(), 0);
		let add = state.add_profile_friend(user.id).unwrap();
		state
			.apply_user_action(Event::Friend {
				user: user.id,
				friend: true,
				profile: Some((user.clone(), "synthetic".into())),
			})
			.unwrap();
		finish(&mut state, add, Ok(()));
		assert_eq!(state.friends().count(), 1);
		assert_eq!(state.pending_friends().count(), 0);
		let remove = state.remove_friend(user.id).unwrap();
		state
			.apply_user_action(Event::Friend {
				user: user.id,
				friend: false,
				profile: None,
			})
			.unwrap();
		state
			.apply_user_action(Event::Friend {
				user: user.id,
				friend: true,
				profile: Some((user.clone(), "synthetic".into())),
			})
			.unwrap();
		finish(&mut state, remove, Ok(()));
		assert_eq!(state.friends().count(), 1, "newer gateway friendship wins");
		state
			.apply_user_action(Event::Relationship {
				user: user.id,
				blocked: true,
			})
			.unwrap();
		assert!(state.add_profile_friend(user.id).is_none());
		assert!(state.remove_friend(user.id).is_none());
		state.gateway_connected = false;
		assert!(state.add_profile_friend(Id(3)).is_none());
	}
	/// Synthetic authenticated state for user-action tests.
	fn state() -> State {
		let user = |id| model::User {
			primary_guild: None,
			id: Id(id),
			name: "Synthetic".into(),
			avatar: None,
			webhook: false,
			kind: Default::default(),
			discriminator: 0,
		};
		State {
			user: Some(user(1)),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			selected: Some(Id(10)),
			channels: vec![model::Channel {
				id: Id(10),
				guild: None,
				kind: 1,
				recipients: vec![user(2)],
				name: "Synthetic DM".into(),
				parent_id: None,
				position: 0,
				last_message: None,
				icon: None,
				member_list_id: None,
				message_count: None,
			}],
			..State::default()
		}
	}
	/// Completes a pending write with the given result.
	fn finish(state: &mut State, command: Command, result: Result<(), Failure>) {
		let Command::UserAction {
			action, request, ..
		} = command
		else {
			panic!("wrong command")
		};
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::UserAction(Event::Written {
				action,
				request,
				result,
			}),
		});
	}
	#[test]
	fn friends_are_explicit_bounded_and_removed_by_relationship_changes() {
		let mut state = state();
		let user = state.channels[0].recipients[0].clone();
		state
			.apply_user_action(Event::Relationships(Some(vec![(user.id, false)])))
			.unwrap();
		state
			.apply_user_action(Event::Friends(Some(vec![(
				user.clone(),
				"synthetic_friend".into(),
			)])))
			.unwrap();
		assert!(state.friends_known());
		assert_eq!(state.friends().count(), 1);
		assert_eq!(state.friend_username(user.id), Some("synthetic_friend"));
		let mut changed = user.clone();
		changed.name = "New display".into();
		state
			.apply_user_action(Event::FriendProfile((changed, "new_username".into())))
			.unwrap();
		assert_eq!(state.friends().next().unwrap().name, "New display");
		state
			.apply_user_action(Event::Friend {
				user: user.id,
				friend: false,
				profile: None,
			})
			.unwrap();
		assert_eq!(state.friends().count(), 0);
		state
			.apply_user_action(Event::Friend {
				user: user.id,
				friend: true,
				profile: Some((user.clone(), "user".into())),
			})
			.unwrap();
		let command = state.set_user_blocked(user.id, true).unwrap();
		assert_eq!(state.friends().count(), 0);
		finish(&mut state, command, Err(Failure::Forbidden));
		assert_eq!(state.friends().count(), 1);
		state
			.apply_user_action(Event::Relationship {
				user: user.id,
				blocked: true,
			})
			.unwrap();
		assert_eq!(state.friends().count(), 0);
		assert!(
			state
				.apply_user_action(Event::Friends(Some(vec![(
					user,
					"x".repeat(MAX_FRIEND_BYTES)
				)])))
				.is_err()
		);
		assert!(!state.friends_known());
		state.logout();
		assert_eq!(state.friends().count(), 0);
	}
	#[test]
	fn restricted_profiles_replace_update_and_clear_with_the_session() {
		let mut state = state();
		let user = state.channels[0].recipients[0].clone();
		state
			.apply_user_action(Event::Restrictions(Some(vec![(
				user.clone(),
				"synthetic_user".into(),
				false,
			)])))
			.unwrap();
		assert!(state.restricted_users_known());
		assert_eq!(state.restricted_users().count(), 1);
		assert!(!state.restricted_user(user.id).unwrap().2);
		state
			.apply_user_action(Event::Restriction {
				user: user.id,
				ignored: Some(true),
				profile: None,
			})
			.unwrap();
		assert!(state.restricted_user(user.id).unwrap().2);
		let mut changed = user.clone();
		changed.name = "Updated display".into();
		state
			.apply_user_action(Event::FriendProfile((changed, "updated_username".into())))
			.unwrap();
		assert_eq!(
			state.restricted_user(user.id).unwrap().0.name,
			"Updated display"
		);
		state
			.apply_user_action(Event::Restriction {
				user: user.id,
				ignored: None,
				profile: None,
			})
			.unwrap();
		assert_eq!(state.restricted_users().count(), 0);
		state.logout();
		assert!(!state.restricted_users_known());
	}
	#[test]
	fn user_actions_update_immediately_rollback_and_reject_stale_results() {
		let mut state = state();
		assert_eq!(state.user_blocked(Id(2)), None);
		assert!(state.set_user_blocked(Id(1), true).is_none());
		assert!(state.set_user_blocked(Id(2), false).is_none());
		let block = state.set_user_blocked(Id(2), true).unwrap();
		assert_eq!(state.user_blocked(Id(2)), Some(true));
		assert_eq!(state.user_action_status(), None);
		assert!(state.close_dm(Id(10)).is_none());
		finish(&mut state, block, Err(Failure::Forbidden));
		assert!(!state.user_action_pending());
		assert_eq!(state.user_blocked(Id(2)), None);
		assert_eq!(state.status, "Disconnected");
		assert_eq!(state.user_action_status(), None);
		let block = state.set_user_blocked(Id(2), true).unwrap();
		finish(&mut state, block, Ok(()));
		assert_eq!(state.user_blocked(Id(2)), Some(true));
		let unblock = state.set_user_blocked(Id(2), false).unwrap();
		assert_eq!(state.user_blocked(Id(2)), Some(false));
		finish(&mut state, unblock, Ok(()));
		assert_eq!(state.user_blocked(Id(2)), Some(false));
		state.drafts.insert(Id(10), "Keep my draft".into());
		let close = state.close_dm(Id(10)).unwrap();
		finish(&mut state, close, Err(Failure::Ambiguous));
		assert!(state.channel(Id(10)).is_some());
		let close = state.close_dm(Id(10)).unwrap();
		finish(&mut state, close, Ok(()));
		assert!(state.channel(Id(10)).is_none());
		assert_eq!(state.selected, None);
		assert_eq!(state.drafts[&Id(10)], "Keep my draft");
		let block = state.set_user_blocked(Id(2), true).unwrap();
		state.cancel_user_action();
		finish(&mut state, block, Ok(()));
		assert_eq!(state.user_blocked(Id(2)), Some(false));
		state.gateway_connected = false;
		assert!(state.set_user_blocked(Id(2), true).is_none());
		state.demo = true;
		let command = state.set_user_blocked(Id(2), true).unwrap();
		state.command_rejected(command);
		assert!(!state.user_action_pending());
		assert_eq!(state.user_blocked(Id(2)), Some(false));
	}
	#[test]
	fn fresh_ready_does_not_reuse_an_outstanding_write_request() {
		let mut state = state();
		let old = state.set_user_blocked(Id(2), true).unwrap();
		let user = state.user.clone().unwrap();
		let channels = state.channels.clone();
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Ready {
				permissions: Default::default(),
				user,
				guilds: vec![],
				channels,
			},
		});
		let new = state.set_user_blocked(Id(2), true).unwrap();
		finish(&mut state, old, Ok(()));
		assert!(state.user_action_pending());
		assert_eq!(state.user_blocked(Id(2)), Some(true));
		finish(&mut state, new, Ok(()));
		assert_eq!(state.user_blocked(Id(2)), Some(true));
	}
	/// Regression: gateway updates win over late writes and settings stay bounded.
	#[test]
	fn gateway_updates_win_over_late_writes_and_settings_stay_bounded() {
		let mut state = state();
		state
			.apply_user_action(Event::Relationships(Some(vec![(Id(2), false)])))
			.unwrap();
		let block = state.set_user_blocked(Id(2), true).unwrap();
		state
			.apply_user_action(Event::Relationship {
				user: Id(2),
				blocked: true,
			})
			.unwrap();
		state
			.apply_user_action(Event::Relationship {
				user: Id(2),
				blocked: false,
			})
			.unwrap();
		finish(&mut state, block, Ok(()));
		assert_eq!(state.user_blocked(Id(2)), Some(false));
		let mute = state.set_dm_muted(Id(10), true).unwrap();
		assert_eq!(state.dm_muted(Id(10)), Some(true));
		finish(&mut state, mute, Err(Failure::Forbidden));
		assert_eq!(state.dm_muted(Id(10)), None);
		let mute = state.set_dm_muted(Id(10), true).unwrap();
		finish(&mut state, mute, Ok(()));
		assert_eq!(state.dm_muted(Id(10)), Some(true));
		let unmute = state.set_dm_muted(Id(10), false).unwrap();
		state
			.apply_notification_preferences(crate::notifications::Event::Settings {
				entries: vec![crate::notifications::Setting {
					channels: vec![(Id(10), Some(true), Some(2))],
					..Default::default()
				}],
				replace: false,
			})
			.unwrap();
		finish(&mut state, unmute, Ok(()));
		assert_eq!(state.dm_muted(Id(10)), Some(true));
		// Completion feedback is transient UI state, never a shared menu status.
		assert_eq!(state.status, "Disconnected");
		assert_eq!(state.user_action_status(), None);
		let unmute = state.set_dm_muted(Id(10), false).unwrap();
		state
			.apply_notification_preferences(crate::notifications::Event::Settings {
				entries: vec![crate::notifications::Setting {
					guild: Some(Id(999)),
					..Default::default()
				}],
				replace: false,
			})
			.unwrap();
		finish(&mut state, unmute, Ok(()));
		assert_eq!(state.dm_muted(Id(10)), Some(false));
		let close = state.close_dm(Id(10)).unwrap();
		let channel = state.channel(Id(10)).unwrap().clone();
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::Unavailable(Id(10)),
		});
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::ChannelCreated(channel),
		});
		finish(&mut state, close, Ok(()));
		assert!(
			state.channel(Id(10)).is_some(),
			"late close must not remove a DM reopened by a later event"
		);
		assert!(
			state
				.apply_user_action(Event::Relationships(Some(vec![
					(Id(2), true);
					MAX_RELATIONSHIPS + 1
				])))
				.is_err()
		);
		assert_eq!(state.user_blocked(Id(2)), None);
		assert!(
			state
				.apply_user_action(Event::Relationships(Some(vec![
					(Id(2), true),
					(Id(2), false)
				])))
				.is_err()
		);
		state.logout();
		assert_eq!(state.dm_muted(Id(10)), None);
		assert_eq!(state.user_blocked(Id(2)), None);
	}

	/// Regression: a friendship challenge is scoped, single-use and resumes its write.
	#[test]
	fn friend_captcha_is_scoped_single_use_and_resumes_the_same_write() {
		use crate::captcha::{Challenge, Solution};
		let mut state = state();
		let Some(Command::UserAction {
			action,
			request,
			captcha: None,
		}) = state.add_friend("synthetic_friend")
		else {
			panic!("friend request")
		};
		assert!(matches!(action, Action::AddFriend { .. }));
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::UserAction(Event::Challenge {
				action: action.clone(),
				request,
				challenge: Box::new(
					Challenge::new("synthetic-key".into(), None, None, None, false).unwrap(),
				),
			}),
		});
		assert_eq!(
			state.friend_challenge().map(|(request, _)| request),
			Some(request)
		);
		// A solution cannot resume a different request.
		assert!(
			state
				.resume_friend_challenge(
					request.wrapping_add(1),
					Solution::new("synthetic-solution".into()).unwrap()
				)
				.is_none()
		);
		let Some(Command::UserAction {
			action: resumed,
			request: resumed_request,
			captcha: Some(retry),
		}) = state
			.resume_friend_challenge(request, Solution::new("synthetic-solution".into()).unwrap())
		else {
			panic!("resume")
		};
		assert!(matches!(resumed, Action::AddFriend { .. }));
		assert_ne!(resumed_request, request);
		assert!(retry.matches_target(&challenge_target(&resumed).unwrap()));
		// The challenge is consumed once.
		assert!(state.friend_challenge().is_none());
		assert!(
			state
				.resume_friend_challenge(
					resumed_request,
					Solution::new("synthetic-solution".into()).unwrap()
				)
				.is_none()
		);
		finish(
			&mut state,
			Command::UserAction {
				action: resumed,
				request: resumed_request,
				captcha: None,
			},
			Ok(()),
		);
		assert!(!state.user_action_pending());
	}

	/// Regression: cancel and expiry release the pending friendship write.
	#[test]
	fn friend_captcha_cancel_and_expiry_release_the_pending_write() {
		use crate::captcha::Challenge;
		let challenge = || Challenge::new("synthetic-key".into(), None, None, None, false).unwrap();
		let mut state = state();
		let Some(Command::UserAction {
			action, request, ..
		}) = state.add_friend("synthetic_friend")
		else {
			panic!("friend request")
		};
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::UserAction(Event::Challenge {
				action: action.clone(),
				request,
				challenge: Box::new(challenge()),
			}),
		});
		state.cancel_friend_challenge(request);
		assert!(state.friend_challenge().is_none() && !state.user_action_pending());
		// Expiry releases the pending write so the user can send it again.
		let Some(Command::UserAction {
			action, request, ..
		}) = state.add_friend("synthetic_friend")
		else {
			panic!("friend request")
		};
		state.apply(Envelope {
			generation: state.generation,
			event: CoreEvent::UserAction(Event::Challenge {
				action: action.clone(),
				request,
				challenge: Box::new(challenge()),
			}),
		});
		state.user_actions.challenge.as_mut().unwrap().0 =
			std::time::Instant::now() - crate::captcha::LIFETIME;
		state.expire_friend_challenge();
		assert!(state.friend_challenge().is_none() && !state.user_action_pending());
	}
}
