//! A single explicitly requested profile; no directory or persistent profile cache.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
	preload,
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
	/// Whether an unexpired profile for `(user, guild)` is cached, without evicting anything.
	fn contains(&self, user: Id, guild: Option<Id>, now: Instant) -> bool {
		self.entries.iter().any(|entry| {
			entry.user == user
				&& entry.guild == guild
				&& now.saturating_duration_since(entry.fetched) < CACHE_TTL
		})
	}
	/// A cached, unexpired global profile, without refreshing or evicting anything.
	fn peek(&self, user: Id, now: Instant) -> Option<&UserProfile> {
		self.entries
			.iter()
			.find(|entry| {
				entry.user == user
					&& entry.guild.is_none()
					&& now.saturating_duration_since(entry.fetched) < CACHE_TTL
			})
			.map(|entry| &entry.data)
	}
	/// Updates every cached view of `user` in place.
	pub(crate) fn update(&mut self, user: Id, change: impl Fn(&mut UserProfile)) {
		for entry in self.entries.iter_mut().filter(|entry| entry.user == user) {
			change(&mut entry.data);
		}
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

/// A profile response for the requested person and server that fits its bounds.
fn accepts(user: Id, guild: Option<Id>, data: &UserProfile) -> bool {
	data.user.id == user
		&& data.guild.as_ref().is_none_or(|g| Some(g.guild) == guild)
		&& data.valid()
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

/// The account's pending badge settings write.
#[derive(Default)]
pub struct BadgeEditor {
	pub saving: bool,
	error: Option<&'static str>,
	/// Increments with every new error, so a menu can skip ones raised before it opened.
	errors: u64,
	/// Increments when a write is confirmed, so open editors can drop their draft.
	pub saved: u64,
	request: u64,
}
impl BadgeEditor {
	/// Error count to pass to [`Self::error_since`]; errors up to now count as seen.
	pub fn errors(&self) -> u64 {
		self.errors
	}
	/// The latest error, unless it was already raised when `seen` was taken.
	pub fn error_since(&self, seen: u64) -> Option<&'static str> {
		self.error.filter(|_| self.errors != seen)
	}
	fn fail(&mut self, error: &'static str) {
		self.error = Some(error);
		self.errors = self.errors.wrapping_add(1);
	}
}

impl State {
	/// An already loaded global profile for `user`: the open view or the recent cache.
	pub fn known_profile(&self, user: Id) -> Option<&UserProfile> {
		self.profile
			.as_ref()
			.filter(|view| view.user == user && view.guild.is_none())
			.and_then(|view| view.data.as_ref())
			.or_else(|| self.profile_cache.peek(user, Instant::now()))
	}
	/// Saves the account's badge order and hidden set. Demo sessions confirm locally.
	pub fn save_badge_settings(
		&mut self,
		settings: model::badges::BadgeSettings,
	) -> Option<Command> {
		if self.badge_editor.saving || !settings.valid() || self.user.is_none() {
			if !settings.valid() {
				self.badge_editor.fail("Badge settings are invalid");
			}
			return None;
		}
		if !self.demo && (self.auth != AuthState::Authenticated || !self.gateway_connected) {
			self.badge_editor
				.fail("Badge settings need a connected session");
			return None;
		}
		let own = self.user.as_ref().map(|user| user.id)?;
		let profile = self
			.profile
			.as_ref()
			.filter(|view| view.user == own)
			.and_then(|view| view.data.as_ref())
			.or_else(|| {
				self.own_profile
					.data
					.as_ref()
					.filter(|profile| profile.user.id == own)
			})
			.or_else(|| self.known_profile(own));
		if profile
			.and_then(|profile| profile.badge_catalog.as_deref())
			.is_some_and(|catalog| catalog.limited)
		{
			self.badge_editor
				.fail("Reload your profile before editing an incomplete badge catalog");
			return None;
		}
		self.badge_editor.request = self.badge_editor.request.wrapping_add(1);
		self.badge_editor.saving = true;
		self.badge_editor.error = None;
		Some(Command::BadgeSettings {
			request: self.badge_editor.request,
			settings,
		})
	}
	pub(crate) fn apply_badge_settings(
		&mut self,
		request: u64,
		result: Result<model::badges::BadgeSettings, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		if request != self.badge_editor.request || !self.badge_editor.saving {
			return;
		}
		self.badge_editor.saving = false;
		let settings = match result {
			Ok(settings) => settings,
			Err(failure) => {
				self.badge_editor.fail(failure.label());
				return;
			}
		};
		self.badge_editor.error = None;
		self.badge_editor.saved = self.badge_editor.saved.wrapping_add(1);
		let Some(own) = self.user.as_ref().map(|user| user.id) else {
			return;
		};
		let apply = |profile: &mut UserProfile| {
			if profile.user.id == own
				&& let Some(catalog) = profile.badge_catalog.as_deref_mut()
			{
				catalog.apply(&settings);
			}
		};
		if let Some(data) = self.profile.as_mut().and_then(|view| view.data.as_mut()) {
			apply(data);
		}
		if let Some(data) = self.own_profile.data.as_mut() {
			apply(data);
		}
		self.profile_cache.update(own, apply);
	}

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
	/// A lost session drops the pending badge write; it may still have applied.
	pub(crate) fn interrupt_badge_settings(&mut self) {
		if self.badge_editor.saving {
			self.badge_editor.saving = false;
			self.badge_editor.request = self.badge_editor.request.wrapping_add(1);
			self.badge_editor
				.fail("Save interrupted; changes may have applied. Reopen the profile to check");
		}
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
				self.preload.profiles.touch(None);
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
		// A card whose profile is already being preloaded waits for that response instead.
		(allowed
			&& !self
				.preload
				.profiles
				.adopt((user, guild), self.profile_request, now))
		.then(|| self.profile_command(user, guild, self.profile_request))
	}
	/// The request a card sends for `(user, guild)`; preloads send the same one early.
	pub(crate) fn profile_command(&self, user: Id, guild: Option<Id>, request: u64) -> Command {
		Command::Profile {
			user,
			guild,
			request,
			with_mutuals: self.user.as_ref().is_none_or(|own| own.id != user),
			cosmetics: self.profile_cosmetics,
		}
	}
	pub(crate) fn profile_cached(&self, user: Id, guild: Option<Id>, now: Instant) -> bool {
		self.profile_cache.contains(user, guild, now)
	}
	/// Applies the device's cosmetic preference. Profiles loaded while effects or frames were
	/// hidden lack their catalog details, so showing them drops the cache and requests the open
	/// profile again; its new request id also makes any in-flight, unenriched response stale.
	#[must_use]
	pub fn set_profile_cosmetics(&mut self, value: model::CosmeticPreferences) -> Option<Command> {
		let previous = std::mem::replace(&mut self.profile_cosmetics, value);
		if !((value.effects && !previous.effects) || (value.frames && !previous.frames)) {
			return None;
		}
		self.profile_cache.clear();
		// A profile preloaded without these details must not refill the cache.
		self.preload.profiles.touch(None);
		// Offline views keep what they show rather than turning into an error.
		if self.demo || self.auth != AuthState::Authenticated || !self.gateway_connected {
			return None;
		}
		let (user, guild) = self.profile.as_ref().map(|view| (view.user, view.guild))?;
		self.request_profile(user, guild)
	}
	pub fn clear_profile(&mut self) -> Command {
		self.cancel_user_note_read();
		self.profile_request = self.profile_request.wrapping_add(1);
		self.profile = None;
		Command::CancelProfile
	}
	pub(crate) fn apply_profile(
		&mut self,
		user: Id,
		guild: Option<Id>,
		mut request: u64,
		result: Result<Box<UserProfile>, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let mut cacheable = true;
		if preload::is_preload(request) {
			match self.preload.profiles.arrived(request, result.is_ok()) {
				preload::Arrival::Adopted(own, fresh)
					if self
						.profile
						.as_ref()
						.is_some_and(|view| view.request == own) =>
				{
					request = own;
					cacheable = fresh;
				}
				// Not opened (yet): keep it for the click, like a card that was closed.
				preload::Arrival::Adopted(_, true) | preload::Arrival::Keep => {
					if let Ok(data) = result
						&& accepts(user, guild, &data)
					{
						self.profile_cache
							.insert(user, guild, *data, Instant::now());
					}
					return;
				}
				preload::Arrival::Adopted(_, false) | preload::Arrival::Drop => return,
			}
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
			Ok(data) if accepts(user, guild, &data) => {
				view.error = None;
				view.data = Some(*data);
				let data = view.data.clone().expect("stored profile");
				if cacheable {
					self.profile_cache.insert(user, guild, data, Instant::now());
				}
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
				cosmetics: None,
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
			mutual_friends: vec![],
			guild: None,
			theme_colors: None,
			clan: None,
			board: None,
			effect_sku: None,
			frame_sku: None,
			effect: None,
			frame: None,
			badge_catalog: None,
			limited: false,
		})
	}

	#[test]
	fn expired_and_interrupted_profile_preloads_cannot_be_adopted() {
		let now = Instant::now();
		for interrupted in [false, true] {
			let mut state = own_state();
			let target = (Id(2), None);
			assert!(state.preload_profile(None, Some(target), now).is_some());
			let click = if interrupted {
				state.interrupt_preloads();
				now
			} else {
				now + Duration::from_secs(30)
			};
			assert!(matches!(
				state.request_profile_at(Id(2), None, click),
				Some(Command::Profile { user: Id(2), .. })
			));
		}
		// Reconnecting the same account clears flights even without an account-id change.
		let mut state = own_state();
		assert!(
			state
				.preload_profile(None, Some((Id(2), None)), now)
				.is_some()
		);
		let user = state.user.clone().unwrap();
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Ready {
				user,
				guilds: vec![],
				channels: vec![],
				permissions: Default::default(),
			},
		});
		assert!(state.request_profile_at(Id(2), None, now).is_some());
	}

	#[test]
	fn a_closed_adopted_profile_cannot_restore_invalidated_cosmetics() {
		for invalidated in [false, true] {
			let now = Instant::now();
			let mut state = own_state();
			state.profile_cosmetics = model::CosmeticPreferences {
				frames: false,
				effects: false,
				..Default::default()
			};
			let Some(Command::Prefetch(command)) =
				state.preload_profile(None, Some((Id(2), None)), now)
			else {
				panic!()
			};
			let Command::Profile { request, .. } = *command else {
				panic!()
			};
			assert!(state.request_profile_at(Id(2), None, now).is_none());
			state.clear_profile();
			if invalidated {
				assert!(
					state
						.set_profile_cosmetics(model::CosmeticPreferences {
							frames: true,
							effects: true,
							..Default::default()
						})
						.is_none()
				);
			}
			let mut data = own_data("Synthetic friend");
			data.user.id = Id(2);
			state.apply_profile(Id(2), None, request, Ok(data));
			assert_eq!(
				state.request_profile_at(Id(2), None, now).is_some(),
				invalidated
			);
		}
	}

	#[test]
	fn resting_on_a_person_preloads_their_card_once_and_a_click_uses_it() {
		use crate::preload::PROFILE_DWELL;
		let now = Instant::now();
		let friend = |name| {
			let mut data = own_data(name);
			data.user.id = Id(2);
			data
		};
		for adopt in [false, true] {
			let mut state = own_state();
			let target = Some((Id(2), None));
			assert!(state.preload_profile(target, None, now).is_none());
			let Some(Command::Prefetch(command)) =
				state.preload_profile(target, None, now + PROFILE_DWELL)
			else {
				panic!("rested preload");
			};
			let Command::Profile {
				user: Id(2),
				guild: None,
				request,
				with_mutuals: true,
				..
			} = *command
			else {
				panic!("the card's own request");
			};
			if adopt {
				assert!(
					state.request_profile_at(Id(2), None, now).is_none(),
					"the opened card waits for the preload"
				);
				assert!(state.profile.as_ref().unwrap().loading);
			}
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Profile {
					user: Id(2),
					guild: None,
					request,
					result: Ok(friend("Friend")),
				},
			});
			if adopt {
				let view = state.profile.as_ref().unwrap();
				assert!(!view.loading && view.data.is_some());
			} else {
				assert!(state.profile.is_none(), "a preload never opens a card");
				assert!(
					state.request_profile_at(Id(2), None, now).is_none(),
					"the cached card opens at once"
				);
				assert!(state.profile.as_ref().unwrap().data.is_some());
			}
			let _ = state.clear_profile();
			assert!(
				state
					.preload_profile(target, None, now + PROFILE_DWELL * 3)
					.is_none(),
				"a cached card is not requested again"
			);
		}
		let mut state = own_state();
		let Some(Command::Prefetch(command)) =
			state.preload_profile(None, Some((Id(3), None)), now)
		else {
			panic!("pressing skips the dwell");
		};
		let Command::Profile { request, .. } = *command else {
			panic!("profile request");
		};
		state.apply_profile(Id(3), None, request, Err(Failure::Network));
		assert!(state.profile.is_none() && state.profile_cache.is_empty());
		assert!(
			state
				.preload_profile(None, Some((Id(3), None)), now)
				.is_some(),
			"a failure may be tried again"
		);
		state.demo = true;
		assert!(
			state
				.preload_profile(None, Some((Id(4), None)), now)
				.is_none()
		);
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
	fn incomplete_badge_catalog_cannot_replace_account_settings() {
		let mut state = own_state();
		let mut profile = own_data("Before");
		profile.badge_catalog = Some(Box::new(model::badges::BadgeCatalog {
			badges: vec![],
			limited: true,
		}));
		state.own_profile.data = Some(*profile);
		let settings = model::badges::BadgeSettings {
			order: vec![model::badges::id::ACCOUNT_AGE],
			hidden: vec![],
		};
		let seen = state.badge_editor.errors();
		assert!(state.save_badge_settings(settings.clone()).is_none());
		assert!(!state.badge_editor.saving);
		assert!(state.badge_editor.error_since(seen).is_some());
		state
			.own_profile
			.data
			.as_mut()
			.unwrap()
			.badge_catalog
			.as_mut()
			.unwrap()
			.limited = false;
		assert!(state.save_badge_settings(settings).is_some());
	}
	#[test]
	fn lost_sessions_release_a_pending_badge_save_with_a_fresh_error() {
		let mut state = own_state();
		let settings = model::badges::BadgeSettings {
			order: vec![model::badges::id::ACCOUNT_AGE],
			hidden: vec![],
		};
		let Some(Command::BadgeSettings { request, .. }) =
			state.save_badge_settings(settings.clone())
		else {
			panic!("badge save expected");
		};
		let seen = state.badge_editor.errors();
		state.apply_badge_settings(request, Err(Failure::Expired));
		assert!(!state.badge_editor.saving);
		assert!(state.badge_editor.error_since(seen).is_some());
		// A menu opened afterwards does not show the old error.
		assert!(
			state
				.badge_editor
				.error_since(state.badge_editor.errors())
				.is_none()
		);
		// The late answer to the interrupted write is ignored.
		state.apply_badge_settings(request, Ok(settings));
		assert_eq!(state.badge_editor.saved, 0);
	}
	#[test]
	fn own_profile_requires_loaded_account_and_confirms_only_current_bounded_responses() {
		{
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
		{
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
		{
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
		{
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
	fn own_profile_does_not_request_mutuals() {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(own_data("Self").user.clone()),
			..State::default()
		};
		for (user, expected) in [(Id(1), false), (Id(2), true)] {
			let Some(Command::Profile { with_mutuals, .. }) = state.request_profile(user, None)
			else {
				panic!("missing request");
			};
			assert_eq!(with_mutuals, expected);
		}
	}
	#[test]
	fn profiles_require_explicit_request_and_reject_late_views() {
		{
			let mut state = State::default();
			assert!(state.request_profile(Id(1), None).is_none());
			state.auth = AuthState::Authenticated;
			state.gateway_connected = true;
			let Some(Command::Profile { request, .. }) = state.request_profile(Id(1), None) else {
				panic!("missing request")
			};
			assert!(state.profile.as_ref().unwrap().loading);
			let Some(Command::Profile { request: new, .. }) = state.request_profile(Id(2), None)
			else {
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
				with_mutuals: true,
				cosmetics: Default::default(),
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
		{
			fn profile(user: Id) -> Box<UserProfile> {
				Box::new(UserProfile {
					user: model::User {
						primary_guild: None,
						cosmetics: None,
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
					mutual_friends: vec![],
					guild: None,
					theme_colors: None,
					clan: None,
					board: None,
					effect_sku: None,
					frame_sku: None,
					effect: None,
					frame: None,
					badge_catalog: None,
					limited: false,
				})
			}
			let mut state = State {
				auth: AuthState::Authenticated,
				gateway_connected: true,
				guilds: vec![model::Guild {
					default_message_notifications: None,
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
			// Hiding cosmetics keeps cached profiles; showing effects refetches them.
			assert!(
				state
					.set_profile_cosmetics(model::CosmeticPreferences::default())
					.is_none()
			);
			assert_eq!(state.profile_cache.len(), CACHE_ENTRIES);
			let Some(Command::Profile { cosmetics, .. }) =
				state.request_profile_at(Id(999), None, now)
			else {
				panic!("missing request")
			};
			assert_eq!(cosmetics, model::CosmeticPreferences::default());
			assert!(
				state
					.set_profile_cosmetics(model::CosmeticPreferences {
						decorations: true,
						..Default::default()
					})
					.is_none()
			);
			assert_eq!(state.profile_cache.len(), CACHE_ENTRIES);
			// The open profile is requested again; the unenriched response in flight is stale.
			let Some(Command::Profile {
				request: refreshed,
				cosmetics,
				..
			}) = state.set_profile_cosmetics(model::CosmeticPreferences::SHOWN)
			else {
				panic!("open profile is refreshed")
			};
			assert!(state.profile_cache.is_empty());
			assert_eq!(cosmetics, model::CosmeticPreferences::SHOWN);
			assert!(
				state
					.set_profile_cosmetics(model::CosmeticPreferences::SHOWN)
					.is_none()
			);
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Profile {
					user: Id(999),
					guild: None,
					request: refreshed - 1,
					result: Ok(profile(Id(999))),
				},
			});
			assert!(state.profile_cache.is_empty());
			for id in 1..=4 {
				state
					.profile_cache
					.insert(Id(id), None, *profile(Id(id)), now);
			}
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
		{
			let profile = |user| {
				Box::new(UserProfile {
					user: model::User {
						primary_guild: None,
						cosmetics: None,
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
					mutual_friends: vec![],
					guild: None,
					theme_colors: None,
					clan: None,
					board: None,
					effect_sku: None,
					frame_sku: None,
					effect: None,
					frame: None,
					badge_catalog: None,
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
				tags: None,
				message_count: None,
			};
			let mut state = State {
				auth: AuthState::Authenticated,
				gateway_connected: true,
				guilds: vec![model::Guild {
					default_message_notifications: None,
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
								secondary_color: None,
								tertiary_color: None,
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
						secondary_color: None,
						tertiary_color: None,
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
}
