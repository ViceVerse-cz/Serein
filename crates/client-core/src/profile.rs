//! A single explicitly requested profile; no directory or persistent profile cache.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Id, UserProfile};
use std::{
	collections::VecDeque,
	time::{Duration, Instant},
};

/// Recently viewed profiles stay in RAM so reopening a card does not repeat the request.
pub const CACHE_ENTRIES: usize = 32;
pub const CACHE_BYTES: usize = 1024 * 1024;
pub const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
#[derive(Default)]
pub struct ProfileCache {
	entries: VecDeque<CachedProfile>,
}
struct CachedProfile {
	user: Id,
	guild: Option<Id>,
	fetched: Instant,
	data: UserProfile,
}
impl ProfileCache {
	fn get(&mut self, user: Id, guild: Option<Id>, now: Instant) -> Option<&UserProfile> {
		self.entries
			.retain(|entry| now.saturating_duration_since(entry.fetched) < CACHE_TTL);
		self.entries
			.iter()
			.find(|entry| entry.user == user && entry.guild == guild)
			.map(|entry| &entry.data)
	}
	fn insert(&mut self, user: Id, guild: Option<Id>, data: UserProfile, now: Instant) {
		self.entries
			.retain(|entry| !(entry.user == user && entry.guild == guild));
		self.entries.push_back(CachedProfile {
			user,
			guild,
			fetched: now,
			data,
		});
		while self.entries.len() > CACHE_ENTRIES || self.bytes() > CACHE_BYTES {
			self.entries.pop_front();
		}
	}
	pub fn bytes(&self) -> usize {
		self.entries
			.iter()
			.map(|entry| size_of::<CachedProfile>() + entry.data.bytes())
			.sum()
	}
	pub fn len(&self) -> usize {
		self.entries.len()
	}
	pub fn is_empty(&self) -> bool {
		self.entries.is_empty()
	}
	pub fn clear(&mut self) {
		self.entries.clear();
		self.entries.shrink_to_fit();
	}
}

pub struct ProfileView {
	pub user: Id,
	pub guild: Option<Id>,
	pub request: u64,
	pub loading: bool,
	pub error: Option<&'static str>,
	pub data: Option<UserProfile>,
}

/// One bounded account profile. The UI owns unsaved text independently of these snapshots.
#[derive(Default)]
pub struct OwnProfile {
	pub data: Option<UserProfile>,
	pub loading: bool,
	pub saving: bool,
	pub error: Option<&'static str>,
	pub reload_required: bool,
	pub request: u64,
	user: Option<Id>,
	generation: u64,
}

impl State {
	pub fn can_save_own_profile(&self) -> bool {
		(self.demo || (self.auth == AuthState::Authenticated && self.gateway_connected))
			&& !self.own_profile.loading
			&& !self.own_profile.saving
			&& !self.own_profile.reload_required
			&& self.own_profile.generation == self.generation
			&& self.own_profile.data.as_ref().is_some_and(|data| {
				!data.limited
					&& self
						.user
						.as_ref()
						.is_some_and(|user| user.id == data.user.id)
			})
	}
	pub fn load_own_profile(&mut self) -> Option<Command> {
		if self.own_profile.loading || self.own_profile.saving {
			return None;
		}
		let Some(user) = self
			.user
			.as_ref()
			.map(|user| user.id)
			.filter(|id| id.0 != 0)
		else {
			self.own_profile.error = Some("Sign in before editing your profile");
			return None;
		};
		if !self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected) {
			self.own_profile.error = Some("Reconnect to load your profile");
			return None;
		}
		let data = if self.own_profile.user == Some(user)
			&& self.own_profile.generation == self.generation
		{
			self.own_profile.data.take()
		} else {
			None
		};
		self.clear_own_profile();
		self.own_profile.data = data;
		self.own_profile.user = Some(user);
		self.own_profile.generation = self.generation;
		self.own_profile.loading = true;
		self.own_profile.reload_required = true;
		Some(Command::EditProfile {
			user,
			request: self.own_profile.request,
			changes: None,
		})
	}
	pub fn save_own_profile(&mut self, changes: model::ProfileEdit) -> Option<Command> {
		if !self.can_save_own_profile() || changes == model::ProfileEdit::default() {
			return None;
		}
		if !changes.valid() {
			self.own_profile.error = Some("Profile changes are invalid or exceed safe limits");
			return None;
		}
		let user = self.user.as_ref()?.id;
		self.own_profile.request = self.own_profile.request.wrapping_add(1);
		self.own_profile.saving = true;
		self.own_profile.error = None;
		Some(Command::EditProfile {
			user,
			request: self.own_profile.request,
			changes: Some(changes),
		})
	}
	pub fn clear_own_profile(&mut self) {
		// Hiding an editor cannot cancel an already admitted write or permit a second one.
		if self.own_profile.saving {
			return;
		}
		self.own_profile = OwnProfile {
			request: self.own_profile.request.wrapping_add(1),
			..Default::default()
		};
	}
	pub(crate) fn interrupt_own_profile(&mut self) {
		if self.own_profile.loading || self.own_profile.saving || self.own_profile.data.is_some() {
			self.own_profile.error = Some(if self.own_profile.saving {
				"Save interrupted; changes may have applied. Reload your profile before saving again"
			} else if self.own_profile.loading {
				"Profile loading interrupted; reconnect and reload"
			} else {
				"Profile is no longer current; reconnect and reload before saving"
			});
		}
		self.own_profile.request = self.own_profile.request.wrapping_add(1);
		self.own_profile.loading = false;
		self.own_profile.saving = false;
		self.own_profile.reload_required = true;
	}
	fn own_profile_pending(&self, user: Id, request: u64) -> bool {
		self.own_profile.user == Some(user)
			&& self.user.as_ref().is_some_and(|own| own.id == user)
			&& self.own_profile.generation == self.generation
			&& self.own_profile.request == request
			&& (self.own_profile.loading || self.own_profile.saving)
	}
	pub(crate) fn reject_own_profile(&mut self, user: Id, request: u64) {
		if self.own_profile_pending(user, request) {
			self.own_profile.loading = false;
			self.own_profile.saving = false;
			self.own_profile.error = Some("Profile request was not queued; try again");
		}
	}
	pub(crate) fn apply_own_profile(
		&mut self,
		user: Id,
		request: u64,
		result: Result<Box<UserProfile>, Failure>,
	) {
		if !self.own_profile_pending(user, request) {
			return;
		}
		let saving = self.own_profile.saving;
		self.own_profile.loading = false;
		self.own_profile.saving = false;
		match result {
			Ok(data) if data.user.id == user && data.guild.is_none() && data.valid() => {
				if !self.refresh_own_user(&data.user) {
					self.own_profile.error =
						Some("Profile metadata exceeds safe capacity; reload required");
					self.own_profile.reload_required = true;
					return;
				}
				self.profile_cache
					.entries
					.retain(|entry| entry.user != user);
				if self.profile.as_ref().is_some_and(|view| view.user == user) {
					self.clear_profile();
				}
				self.own_profile.error = data
					.limited
					.then_some("Full profile unavailable; editing is disabled");
				self.own_profile.reload_required = data.limited;
				self.own_profile.data = Some(*data);
			}
			result => {
				self.own_profile.reload_required = true;
				self.own_profile.error = Some(if saving {
					"Save could not be confirmed; some changes may have applied. Reload before saving again"
				} else if let Err(failure) = result {
					failure.label()
				} else {
					"Profile response was invalid or too large"
				});
				if let Err(failure) = result
					&& failure.ends_session()
					&& failure != Failure::Capacity
				{
					self.fail(failure);
				}
			}
		}
	}
	fn refresh_own_user(&mut self, user: &model::User) -> bool {
		if self.user.as_ref() == Some(user) {
			return true;
		}
		let extra = self
			.channels
			.iter()
			.flat_map(|channel| &channel.recipients)
			.filter(|recipient| recipient.id == user.id)
			.map(|recipient| user.heap_bytes().saturating_sub(recipient.heap_bytes()))
			.sum::<usize>();
		if self
			.navigation_bytes()
			.saturating_add(extra)
			.saturating_add(self.permissions.bytes())
			> model::account::MAX_BYTES
		{
			return false;
		}
		self.user = Some(user.clone());
		for recipient in self
			.channels
			.iter_mut()
			.flat_map(|channel| &mut channel.recipients)
			.filter(|recipient| recipient.id == user.id)
		{
			*recipient = user.clone();
		}
		self.invalidate_navigation();
		if let Some(list) = &mut self.members {
			for member in list
				.slots
				.iter_mut()
				.flatten()
				.filter_map(|slot| match slot {
					model::MemberSlot::Person(m) => Some(m),
					_ => None,
				})
				.filter(|member| member.user.id == user.id)
			{
				member.user = user.clone();
			}
			if list
				.slots
				.iter()
				.flatten()
				.filter_map(|slot| match slot {
					model::MemberSlot::Person(m) => Some(m),
					_ => None,
				})
				.map(model::Member::bytes)
				.sum::<usize>()
				> 128 * 1024
			{
				self.members = None;
			}
		}
		// Dormant and search snapshots lack a mutable, byte-accounted user index; rehydrate them.
		self.clear_cached_history();
		self.clear_search();
		let ids: Vec<_> = self
			.timeline
			.iter()
			.filter(|message| {
				message.author.id == user.id
					|| message.mentions.iter().any(|mention| mention.id == user.id)
			})
			.map(|message| message.id)
			.collect();
		for id in ids {
			let Some(mut message) = self.timeline.get(id).cloned() else {
				continue;
			};
			if message.author.id == user.id {
				message.author = user.clone();
			}
			for mention in message
				.mentions
				.iter_mut()
				.filter(|mention| mention.id == user.id)
			{
				*mention = user.clone();
			}
			if self.timeline.insert(message, true, false).is_err() {
				self.cancel_history();
				self.timeline.clear_window_preserving_deletions();
				self.freshness = model::Freshness::Stale;
				self.status = "Profile updated; use Reload history to refresh this conversation";
				break;
			}
		}
		true
	}
	pub fn request_profile(&mut self, user: Id, guild: Option<Id>) -> Option<Command> {
		self.request_profile_at(user, guild, Instant::now())
	}
	pub fn request_profile_at(
		&mut self,
		user: Id,
		guild: Option<Id>,
		now: Instant,
	) -> Option<Command> {
		self.profile_request = self.profile_request.wrapping_add(1);
		if let Some(data) = self.profile_cache.get(user, guild, now) {
			self.profile = Some(ProfileView {
				user,
				guild,
				request: self.profile_request,
				loading: false,
				error: None,
				data: Some(data.clone()),
			});
			return None;
		}
		let allowed = !self.demo
			&& self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& user.0 != 0
			&& guild.is_none_or(|id| self.guilds.iter().any(|g| g.id == id));
		self.profile = Some(ProfileView {
			user,
			guild,
			request: self.profile_request,
			loading: allowed,
			error: (!allowed)
				.then_some("Profile unavailable while disconnected or outside this session"),
			data: None,
		});
		allowed.then_some(Command::Profile {
			user,
			guild,
			request: self.profile_request,
		})
	}
	pub fn clear_profile(&mut self) -> Command {
		self.profile_request = self.profile_request.wrapping_add(1);
		self.profile = None;
		Command::CancelProfile
	}
	pub(crate) fn apply_profile(
		&mut self,
		user: Id,
		guild: Option<Id>,
		request: u64,
		result: Result<Box<UserProfile>, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let Some(view) = self
			.profile
			.as_mut()
			.filter(|v| v.user == user && v.guild == guild && v.request == request)
		else {
			return;
		};
		view.loading = false;
		match result {
			Ok(data)
				if data.user.id == user
					&& data.guild.as_ref().is_none_or(|g| Some(g.guild) == guild)
					&& data.valid() =>
			{
				view.error = None;
				view.data = Some(*data);
				let data = view.data.clone().expect("stored profile");
				self.profile_cache.insert(user, guild, data, Instant::now());
			}
			Ok(_) => {
				view.error = Some("Profile response was invalid or too large");
				view.data = None;
			}
			Err(failure) => {
				view.error = Some(if failure == Failure::Capacity {
					"Profile request or response exceeded safe capacity"
				} else {
					failure.label()
				});
				view.data = None;
			}
		}
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	fn own_data(name: &str) -> Box<UserProfile> {
		Box::new(UserProfile {
			user: model::User {
				primary_guild: None,
				id: Id(1),
				name: name.into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			username: "synthetic".into(),
			global_name: Some(name.into()),
			banner: None,
			accent_color: None,
			bio: "Synthetic biography".into(),
			pronouns: String::new(),
			badges: vec![],
			connections: vec![],
			mutual_guilds: vec![],
			guild: None,
			theme_colors: None,
			clan: None,
			limited: false,
		})
	}
	fn own_state() -> State {
		State {
			user: Some(own_data("Before").user),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			..Default::default()
		}
	}
	fn complete_own(state: &mut State, result: Result<Box<UserProfile>, Failure>) {
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ProfileEdited {
				user: Id(1),
				request: state.own_profile.request,
				result,
			},
		});
	}
	fn edit() -> model::ProfileEdit {
		model::ProfileEdit {
			global_name: Some(Some("After".into())),
			..Default::default()
		}
	}
	#[test]
	fn own_profile_requires_loaded_account_and_confirms_only_current_bounded_responses() {
		let mut state = State::default();
		assert!(state.load_own_profile().is_none());
		let mut state = own_state();
		assert!(state.save_own_profile(edit()).is_none());
		assert!(matches!(
			state.load_own_profile(),
			Some(Command::EditProfile {
				user: Id(1),
				changes: None,
				..
			})
		));
		let request = state.own_profile.request;
		assert!(state.load_own_profile().is_none());
		state.apply(Envelope {
			generation: state.generation + 1,
			event: Event::ProfileEdited {
				user: Id(1),
				request,
				result: Ok(own_data("Wrong generation")),
			},
		});
		assert!(state.own_profile.loading);
		complete_own(&mut state, Ok(own_data("Before")));
		assert!(state.can_save_own_profile());
		assert!(
			state
				.save_own_profile(model::ProfileEdit {
					bio: Some("x".repeat(191)),
					..Default::default()
				})
				.is_none()
		);
		assert!(state.save_own_profile(Default::default()).is_none());
		state
			.profile_cache
			.insert(Id(1), None, *own_data("Before"), Instant::now());
		state.request_profile(Id(1), None);
		assert!(matches!(
			state.save_own_profile(edit()),
			Some(Command::EditProfile {
				changes: Some(_),
				..
			})
		));
		assert!(state.own_profile.saving && !state.can_save_own_profile());
		complete_own(&mut state, Ok(own_data("After")));
		assert_eq!(state.user.as_ref().unwrap().name, "After");
		assert_eq!(
			state
				.own_profile
				.data
				.as_ref()
				.unwrap()
				.global_name
				.as_deref(),
			Some("After")
		);
		assert!(state.profile.is_none() && state.profile_cache.is_empty());
		assert!(state.can_save_own_profile());
		// Duplicate and retired outcomes cannot revert the confirmed account.
		complete_own(&mut state, Ok(own_data("Before")));
		assert_eq!(state.user.as_ref().unwrap().name, "After");
		state.load_own_profile();
		let mut invalid = own_data("Invalid");
		invalid.bio = "x".repeat(4097);
		complete_own(&mut state, Ok(invalid));
		assert!(!state.can_save_own_profile());
		assert_eq!(state.user.as_ref().unwrap().name, "After");
	}
	#[test]
	fn own_profile_failed_writes_require_reload_but_rejected_queue_does_not() {
		let mut state = own_state();
		state.load_own_profile();
		complete_own(&mut state, Ok(own_data("Before")));
		let command = state.save_own_profile(edit()).unwrap();
		state.command_rejected(command);
		assert!(state.can_save_own_profile());
		assert!(state.own_profile.error.is_some());
		state.save_own_profile(edit());
		state.clear_own_profile();
		assert!(state.own_profile.saving);
		assert!(state.load_own_profile().is_none());
		complete_own(&mut state, Err(Failure::Ambiguous));
		assert!(state.own_profile.reload_required && !state.own_profile.saving);
		assert_eq!(state.own_profile.data.as_ref().unwrap().user.name, "Before");
		assert!(state.save_own_profile(edit()).is_none());
		assert!(state.load_own_profile().is_some());
		assert_eq!(state.own_profile.data.as_ref().unwrap().user.name, "Before");
		assert!(!state.can_save_own_profile());
		complete_own(&mut state, Err(Failure::Network));
		assert_eq!(state.own_profile.data.as_ref().unwrap().user.name, "Before");
		assert!(state.own_profile.reload_required && !state.can_save_own_profile());
		assert!(state.load_own_profile().is_some());
		complete_own(&mut state, Ok(own_data("After")));
		assert!(state.can_save_own_profile());
		state.save_own_profile(edit());
		let request = state.own_profile.request;
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Disconnected,
		});
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Resumed,
		});
		state.apply(Envelope {
			generation: state.generation,
			event: Event::ProfileEdited {
				user: Id(1),
				request,
				result: Ok(own_data("Late")),
			},
		});
		assert!(!state.own_profile.saving && state.own_profile.reload_required);
		assert_eq!(state.user.as_ref().unwrap().name, "After");
		state.logout();
		assert!(state.own_profile.data.is_none());
		assert!(state.load_own_profile().is_none());
	}
	#[test]
	fn own_profile_idle_disconnect_and_full_reconciliation_offer_explicit_recovery() {
		let mut state = own_state();
		state.load_own_profile();
		complete_own(&mut state, Ok(own_data("Before")));
		assert!(state.own_profile.error.is_none());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Disconnected,
		});
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Resumed,
		});
		assert!(state.own_profile.reload_required && state.own_profile.error.is_some());
		assert!(!state.can_save_own_profile());
		state.load_own_profile();
		complete_own(&mut state, Ok(own_data("Before")));
		let mut message = crate::tests::message(1);
		message.author = state.user.clone().unwrap();
		state.timeline.insert(message, false, false).unwrap();
		state.timeline.begin_page(false);
		for id in 2..=(session_cache::MAX_MUTATIONS as u64 + 1) {
			state.timeline.delete(Id(id)).unwrap();
		}
		state.history_pending = true;
		let request = state.request;
		state.save_own_profile(edit());
		complete_own(&mut state, Ok(own_data("After")));
		assert_eq!(state.user.as_ref().unwrap().name, "After");
		assert!(state.own_profile.error.is_none());
		assert!(state.timeline.is_empty() && state.timeline.is_deleted(Id(2)));
		assert!(!state.history_pending && state.request != request);
		assert_eq!(state.freshness, model::Freshness::Stale);
		assert!(state.status.contains("Reload history"));
	}
	#[test]
	fn own_profile_reload_retires_snapshot_when_session_identity_changes() {
		let mut state = own_state();
		state.load_own_profile();
		complete_own(&mut state, Ok(own_data("Before")));
		state.generation += 1;
		assert!(state.load_own_profile().is_some());
		assert!(state.own_profile.data.is_none());
		complete_own(&mut state, Ok(own_data("Current session")));
		state.user.as_mut().unwrap().id = Id(2);
		assert!(state.load_own_profile().is_some());
		assert!(state.own_profile.data.is_none());
	}
	#[test]
	fn own_profile_demo_is_explicit_and_profile_results_charge_heap_capacity() {
		let mut state = own_state();
		state.demo = true;
		state.auth = AuthState::Unauthenticated;
		state.gateway_connected = false;
		assert!(state.load_own_profile().is_some());
		let profile = own_data("Demo");
		let bytes = profile.bytes();
		let event = Event::ProfileEdited {
			user: Id(1),
			request: state.own_profile.request,
			result: Ok(profile),
		};
		assert!(event.bytes() >= bytes + size_of::<Event>());
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
		assert!(state.save_own_profile(edit()).is_some());
		let mut wrong_user = own_data("Other account");
		wrong_user.user.id = Id(2);
		complete_own(&mut state, Ok(wrong_user));
		assert_eq!(state.user.as_ref().unwrap().name, "Demo");
		assert!(state.own_profile.reload_required);
	}
	#[test]
	fn profiles_require_explicit_request_and_reject_late_views() {
		let mut state = State::default();
		assert!(state.request_profile(Id(1), None).is_none());
		state.auth = AuthState::Authenticated;
		state.gateway_connected = true;
		let Some(Command::Profile { request, .. }) = state.request_profile(Id(1), None) else {
			panic!("missing request")
		};
		assert!(state.profile.as_ref().unwrap().loading);
		let Some(Command::Profile { request: new, .. }) = state.request_profile(Id(2), None) else {
			panic!("missing request")
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Profile {
				user: Id(1),
				guild: None,
				request,
				result: Err(Failure::Forbidden),
			},
		});
		assert!(state.profile.as_ref().unwrap().loading);
		state.apply(Envelope {
			generation: state.generation + 1,
			event: Event::Profile {
				user: Id(2),
				guild: None,
				request: new,
				result: Err(Failure::Forbidden),
			},
		});
		assert!(state.profile.as_ref().unwrap().loading);
		state.command_rejected(Command::Profile {
			user: Id(2),
			guild: None,
			request: new,
		});
		assert!(!state.profile.as_ref().unwrap().loading);
		assert!(state.profile.as_ref().unwrap().error.is_some());
		assert!(matches!(state.clear_profile(), Command::CancelProfile));
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Profile {
				user: Id(2),
				guild: None,
				request: new,
				result: Err(Failure::Network),
			},
		});
		assert!(state.profile.is_none());
		state.demo = true;
		assert!(state.request_profile(Id(1), None).is_none());
	}

	#[test]
	fn viewed_profiles_are_reused_until_they_expire_or_the_session_changes() {
		fn profile(user: Id) -> Box<UserProfile> {
			Box::new(UserProfile {
				user: model::User {
					primary_guild: None,
					id: user,
					name: "Synthetic".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				},
				username: "synthetic".into(),
				global_name: None,
				banner: None,
				accent_color: None,
				bio: String::new(),
				pronouns: String::new(),
				badges: vec![],
				connections: vec![],
				mutual_guilds: vec![],
				guild: None,
				theme_colors: None,
				clan: None,
				limited: false,
			})
		}
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			guilds: vec![model::Guild {
				stickers: None,
				emojis: None,
				id: Id(9),
				name: "Synthetic".into(),
				icon: None,
			}],
			..State::default()
		};
		let Some(Command::Profile { request, .. }) = state.request_profile(Id(1), None) else {
			panic!("missing request")
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Profile {
				user: Id(1),
				guild: None,
				request,
				result: Ok(profile(Id(1))),
			},
		});
		assert_eq!(state.profile_cache.len(), 1);
		state.clear_profile();
		let now = Instant::now();
		// Reopening within the TTL is served from RAM without a command.
		assert!(state.request_profile_at(Id(1), None, now).is_none());
		let view = state.profile.as_ref().unwrap();
		assert!(!view.loading && view.error.is_none() && view.data.is_some());
		// A different server scope is a different profile.
		assert!(state.request_profile_at(Id(1), Some(Id(9)), now).is_some());
		// Expired entries are requested again.
		assert!(
			state
				.request_profile_at(Id(1), None, now + CACHE_TTL + Duration::from_secs(1))
				.is_some()
		);
		assert!(state.profile_cache.is_empty());
		// Bounded by entries and bytes; session invalidation clears everything.
		for id in 1..=(CACHE_ENTRIES as u64 + 8) {
			state
				.profile_cache
				.insert(Id(id), None, *profile(Id(id)), now);
		}
		assert_eq!(state.profile_cache.len(), CACHE_ENTRIES);
		assert!(state.profile_cache.bytes() <= CACHE_BYTES);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::PermissionsChanged,
		});
		assert!(state.profile_cache.is_empty());
		// Unchanged typed permission events preserve cached and in-flight views.
		state
			.profile_cache
			.insert(Id(1), None, *profile(Id(1)), now);
		assert!(state.request_profile(Id(1), None).is_none());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::RoleRemoved {
				guild: Id(9),
				id: Id(10),
			}),
		});
		assert!(state.profile.as_ref().unwrap().data.is_some());
		assert_eq!(state.profile_cache.len(), 1);

		// Invalid snapshots still fail closed without accepting a late profile response.
		let guild = || model::permissions::Guild {
			id: Id(9),
			owner: None,
			roles: None,
			member: None,
		};
		let Some(Command::Profile { request, .. }) = state.request_profile(Id(2), None) else {
			panic!("uncached profile request");
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Snapshot(
				model::permissions::Snapshot {
					guilds: vec![guild(), guild()],
					channels: vec![],
				},
			)),
		});
		assert!(state.profile.is_none() && state.profile_cache.is_empty());
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Profile {
				user: Id(2),
				guild: None,
				request,
				result: Ok(profile(Id(2))),
			},
		});
		assert!(state.profile.is_none() && state.profile_cache.is_empty());
	}

	#[test]
	fn open_profile_survives_role_edits_and_unrelated_channel_removal() {
		let profile = |user| {
			Box::new(UserProfile {
				user: model::User {
					primary_guild: None,
					id: user,
					name: "Synthetic".into(),
					avatar: None,
					webhook: false,
					kind: Default::default(),
					discriminator: 0,
				},
				username: "synthetic".into(),
				global_name: None,
				banner: None,
				accent_color: None,
				bio: "kept".into(),
				pronouns: String::new(),
				badges: vec![],
				connections: vec![],
				mutual_guilds: vec![],
				guild: None,
				theme_colors: None,
				clan: None,
				limited: false,
			})
		};
		let channel = |id| model::Channel {
			id: Id(id),
			guild: Some(Id(9)),
			parent_id: None,
			kind: 0,
			name: "Synthetic".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
		};
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			guilds: vec![model::Guild {
				stickers: None,
				emojis: None,
				id: Id(9),
				name: "Synthetic".into(),
				icon: None,
			}],
			channels: vec![channel(2), channel(3)],
			selected: Some(Id(2)),
			..State::default()
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Snapshot(
				model::permissions::Snapshot {
					guilds: vec![model::permissions::Guild {
						id: Id(9),
						owner: None,
						roles: Some(vec![model::permissions::Role {
							id: Id(30),
							bits: 0,
							name: "old".into(),
							color: 0,
							position: 1,
							hoist: false,
						}]),
						member: None,
					}],
					channels: vec![],
				},
			)),
		});
		let Some(Command::Profile { request, .. }) = state.request_profile(Id(4), Some(Id(9)))
		else {
			panic!("profile request");
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Profile {
				user: Id(4),
				guild: Some(Id(9)),
				request,
				result: Ok(profile(Id(4))),
			},
		});
		let before = state.profile.as_ref().unwrap().request;
		assert_eq!(
			state.profile.as_ref().unwrap().data.as_ref().unwrap().bio,
			"kept"
		);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Permissions(crate::permissions::Event::Role {
				guild: Id(9),
				role: model::permissions::Role {
					id: Id(30),
					bits: 0,
					name: "new".into(),
					color: 0,
					position: 1,
					hoist: false,
				},
			}),
		});
		assert!(
			state.profile.is_some(),
			"role edit cleared the open profile"
		);
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Unavailable(Id(3)),
		});
		assert!(
			state.profile.is_some(),
			"unrelated channel removal cleared the open profile"
		);
		let view = state.profile.as_ref().expect("profile stays open");
		assert_eq!(view.request, before);
		assert_eq!(view.data.as_ref().unwrap().bio, "kept");
		assert_eq!(state.profile_cache.len(), 1);
		assert!(state.channels.iter().all(|channel| channel.id != Id(3)));
	}
}
