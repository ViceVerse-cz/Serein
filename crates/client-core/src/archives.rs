//! A single bounded archive page; opening a result admits one transient conversation.
use crate::{
	Command, MAX_NAV, State,
	auth::{AuthState, Failure},
};
use model::{
	Channel, Id,
	archives::{Cursor, Kind, Page},
};
use std::collections::BTreeSet;

pub struct View {
	pub parent: Id,
	pub guild: Id,
	pub kind: Kind,
	pub before: Option<Cursor>,
	pub request: u64,
	pub loading: bool,
	pub error: Option<&'static str>,
	pub page: Option<Page>,
}

impl State {
	pub fn can_archive(&self, parent: Id, kind: Kind) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.can_read_history(parent)
			&& (kind != Kind::Private
				|| self.permission(parent, model::permissions::MANAGE_THREADS) == Some(true))
			&& self.channels.iter().any(|c| {
				c.id == parent
					&& c.guild
						.is_some_and(|guild| self.guilds.iter().any(|g| g.id == guild))
					&& match kind {
						Kind::Public => matches!(c.kind, 0 | 5 | 15 | 16),
						Kind::Private | Kind::JoinedPrivate => c.kind == 0,
					}
			})
	}

	pub fn request_archives(
		&mut self,
		parent: Id,
		kind: Kind,
		before: Option<Cursor>,
	) -> Option<Command> {
		if !self.can_archive(parent, kind) {
			return None;
		}
		let guild = self.channel(parent)?.guild?;
		if before.is_some()
			&& !self.archives.as_ref().is_some_and(|view| {
				view.parent == parent
					&& view.guild == guild
					&& view.kind == kind
					&& !view.loading
					&& if view.error.is_some() {
						view.before == before
					} else {
						view.page.as_ref().is_some_and(|page| page.next == before)
					}
			}) {
			return None;
		}
		self.clear_search();
		self.archives = Some(View {
			parent,
			guild,
			kind,
			before,
			request: self.search_request,
			loading: true,
			error: None,
			page: None,
		});
		Some(Command::Archives {
			parent,
			guild,
			kind,
			before,
			request: self.search_request,
		})
	}

	pub fn clear_archives(&mut self) -> Command {
		self.clear_search()
	}

	pub fn apply_archives(&mut self, parent: Id, request: u64, result: Result<Page, Failure>) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let Some(view) = self.archives.as_ref() else {
			return;
		};
		if view.parent != parent
			|| view.request != request
			|| !view.loading
			|| self.search.is_some()
			|| !self.can_archive(parent, view.kind)
			|| !self
				.channels
				.iter()
				.any(|c| c.id == parent && c.guild == Some(view.guild))
		{
			return;
		}
		let expected_kind = self.archive_thread_kind(parent, view.kind);
		let view = self.archives.as_mut().unwrap();
		view.loading = false;
		match result {
			Ok(page)
				if page.valid(parent, view.guild, view.kind, view.before)
					&& page.threads.iter().all(|c| Some(c.kind) == expected_kind) =>
			{
				view.page = Some(page);
				view.error = None;
			}
			Ok(_) => view.error = Some("Archived threads were invalid or too large"),
			Err(failure) => view.error = Some(failure.label()),
		}
	}

	pub fn open_archived_thread(&mut self, id: Id) -> Option<Command> {
		self.admit_archived_thread(id)?;
		self.select(id)
	}

	pub fn admit_archived_thread(&mut self, id: Id) -> Option<()> {
		let view = self.archives.as_ref()?;
		if view.loading || !self.can_archive(view.parent, view.kind) {
			return None;
		}
		let page = view.page.as_ref()?;
		if !page.valid(view.parent, view.guild, view.kind, view.before)
			|| page
				.threads
				.iter()
				.any(|c| Some(c.kind) != self.archive_thread_kind(view.parent, view.kind))
		{
			return None;
		}
		let thread = page.threads.iter().find(|c| c.id == id)?;
		if let Some(existing) = self.channel(id) {
			if existing.guild != thread.guild
				|| existing.parent_id != thread.parent_id
				|| existing.kind != thread.kind
			{
				self.archives.as_mut()?.error = Some("Thread conflicts with current navigation");
				return None;
			}
			return Some(());
		}
		let retained = self
			.channels
			.iter()
			.filter(|c| Some(c.id) != self.archived_thread);
		let guild_bytes = self.guilds.iter().map(model::Guild::bytes).sum::<usize>();
		let final_len = retained.clone().count() + 1;
		if final_len + self.guilds.len() > MAX_NAV
			|| retained.map(Channel::bytes).sum::<usize>()
				+ guild_bytes
				+ thread.bytes()
				+ self.permissions.bytes()
				+ self.channels.capacity().saturating_sub(final_len) * size_of::<Channel>()
				+ (self.guilds.capacity() - self.guilds.len()) * size_of::<model::Guild>()
				> model::account::MAX_BYTES
		{
			self.archives.as_mut()?.error = Some("Thread exceeds the navigation budget");
			return None;
		}
		let thread = thread.clone();
		if self
			.channels
			.try_reserve_exact(final_len.saturating_sub(self.channels.len()))
			.is_err()
		{
			self.archives.as_mut()?.error = Some("Thread exceeds the navigation budget");
			return None;
		}
		self.retire_archived_thread(None);
		self.channels.push(thread);
		self.invalidate_navigation();
		self.archived_thread = Some(id);
		Some(())
	}

	pub(super) fn retire_archived_thread(&mut self, keep: Option<Id>) {
		if let Some(id) = self.archived_thread
			&& Some(id) != keep
		{
			self.remove_channels(&BTreeSet::from([id]));
		}
	}

	/// Active (non-archived) threads known to the session under `parent`, newest activity first.
	/// Only the gateway-synced thread list is used; nothing is fetched.
	pub fn active_threads(&self, parent: Id) -> Vec<&Channel> {
		let mut threads: Vec<&Channel> = self
			.channels
			.iter()
			.filter(|c| {
				c.parent_id == Some(parent)
					&& matches!(c.kind, 10..=12)
					&& Some(c.id) != self.archived_thread
			})
			.collect();
		threads.sort_by(|a, b| {
			b.last_message
				.unwrap_or(b.id)
				.cmp(&a.last_message.unwrap_or(a.id))
				.then(b.id.cmp(&a.id))
		});
		threads
	}

	/// The thread a message started, when the session already knows it.
	///
	/// Messages carrying the service `HAS_THREAD` flag share their id with the thread. A
	/// "started a thread" system row (type 18) only names the thread in `content`, so it is
	/// matched by name against this channel's known threads; renamed threads are not inferred.
	pub fn thread_of(&self, message: &model::Message) -> Option<&Channel> {
		let is_thread =
			|c: &&Channel| matches!(c.kind, 10..=12) && c.parent_id == Some(message.channel);
		if message.flags & (1 << 5) != 0
			&& let Some(thread) = self.channels.iter().find(|c| c.id == message.id)
			&& is_thread(&thread)
		{
			return Some(thread);
		}
		if message.kind == 18 {
			let name = message.content.trim();
			if !name.is_empty() {
				// A system row only carries the name, so an ambiguous match resolves to nothing.
				let mut matches = self
					.channels
					.iter()
					.filter(is_thread)
					.filter(|c| c.name.trim() == name);
				let thread = matches.next()?;
				return matches.next().is_none().then_some(thread);
			}
		}
		None
	}

	fn archive_thread_kind(&self, parent: Id, kind: Kind) -> Option<u8> {
		let parent = self.channel(parent)?;
		Some(match kind {
			Kind::Public if parent.kind == 5 => 10,
			Kind::Public => 11,
			Kind::Private | Kind::JoinedPrivate => 12,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	use model::{Freshness, Guild};

	fn channel(id: u64, parent: Option<Id>, kind: u8) -> Channel {
		Channel {
			id: Id(id),
			guild: Some(Id(1)),
			parent_id: parent,
			kind,
			name: "Synthetic".into(),
			position: 0,
			recipients: vec![],
			last_message: None,
			icon: None,
			member_list_id: None,
			message_count: None,
		}
	}
	fn message(id: u64, channel: Id) -> model::Message {
		model::Message {
			flags: 0,
			ephemeral: false,
			components: vec![],
			application_id: None,
			reactions: None,
			id: Id(id),
			channel,
			author: model::User {
				primary_guild: None,
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			author_roles: vec![],
			author_nick: None,
			content: String::new(),
			mentions: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
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
			embeds_suppressed: false,
			attachments: vec![],
		}
	}
	fn state() -> State {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			selected: Some(Id(10)),
			user: Some(model::User {
				primary_guild: None,
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![Guild {
				emojis: None,
				id: Id(1),
				name: "Synthetic".into(),
				icon: None,
			}],
			channels: vec![channel(10, None, 0), channel(20, None, 15)],
			..State::default()
		};
		crate::tests::grant_permissions(&mut state);
		state
	}
	fn page(id: u64, next: Option<Cursor>) -> Page {
		Page {
			threads: vec![channel(id, Some(Id(10)), 11)],
			next,
		}
	}
	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}

	#[test]
	fn archive_and_search_reads_require_current_permissions() {
		use model::permissions as p;
		let mut state = state();
		let guild = state.permissions.guilds.get_mut(&Id(1)).unwrap();
		guild.owner = Some(Id(999));
		guild.roles = Some(vec![p::Role {
			name: String::new(),
			color: 0,
			position: 0,
			hoist: false,
			id: Id(1),
			bits: p::VIEW_CHANNEL | p::READ_MESSAGE_HISTORY,
		}]);
		guild.member = Some(p::Member {
			roles: vec![],
			timeout_until: None,
		});
		state.permissions.clear_cache();
		assert!(state.can_archive(Id(10), Kind::Public));
		assert!(state.can_archive(Id(10), Kind::JoinedPrivate));
		assert!(!state.can_archive(Id(10), Kind::Private));
		assert!(
			state
				.request_archives(Id(10), Kind::Private, None)
				.is_none()
		);
		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits |= p::MANAGE_THREADS;
		state.permissions.clear_cache();
		state.request_archives(Id(10), Kind::Private, None).unwrap();
		let request = state.search_request;
		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits &= !p::MANAGE_THREADS;
		state.permissions.clear_cache();
		state.apply_archives(
			Id(10),
			request,
			Ok(Page {
				threads: vec![channel(99, Some(Id(10)), 12)],
				next: None,
			}),
		);
		assert!(state.archives.as_ref().unwrap().page.is_none());

		state.request_archives(Id(10), Kind::Public, None).unwrap();
		let request = state.search_request;
		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits = p::VIEW_CHANNEL | p::SEND_MESSAGES;
		state.permissions.clear_cache();
		state.apply_archives(Id(10), request, Ok(page(99, None)));
		assert!(state.archives.as_ref().unwrap().page.is_none());
		assert!(state.request_pins().is_none());
		assert!(state.request_search("query".into(), None).is_none());
		assert!(
			state.can_send(Id(10)),
			"Sending live messages does not require history access"
		);

		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits |= p::READ_MESSAGE_HISTORY;
		state.permissions.clear_cache();
		state.request_pins().unwrap();
		let request = state.search_request;
		state
			.permissions
			.guilds
			.get_mut(&Id(1))
			.unwrap()
			.roles
			.as_mut()
			.unwrap()[0]
			.bits &= !p::VIEW_CHANNEL;
		state.permissions.clear_cache();
		state.apply_search(
			Id(10),
			request,
			Ok(crate::search::Outcome::Pins(model::SearchPage {
				hits: vec![],
				total: 0,
				partial: false,
				pin_cursor: None,
			})),
		);
		assert!(state.search.as_ref().unwrap().page.is_none());
		assert!(!state.can_send(Id(10)));
	}

	#[test]
	fn archive_pages_are_scoped_replaceable_retryable_and_bounded() {
		let mut state = state();
		assert!(state.can_archive(Id(20), Kind::Public));
		assert!(!state.can_archive(Id(20), Kind::Private));
		assert!(!state.can_archive(Id(999), Kind::Public));
		assert!(
			state
				.request_archives(Id(10), Kind::Public, Some(Cursor::Time(100)))
				.is_none()
		);
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		let stale = state.search_request;
		state.request_pins().unwrap();
		assert!(state.archives.is_none());
		state.apply_archives(Id(10), stale, Ok(page(100, None)));
		assert!(state.archives.is_none());
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		assert!(state.search.is_none());
		let request = state.search_request;
		state.apply_archives(Id(20), request, Ok(page(100, None)));
		assert!(state.archives.as_ref().unwrap().loading);
		state.apply_archives(Id(10), request, Ok(page(100, Some(Cursor::Time(100)))));
		let before = Some(Cursor::Time(100));
		let older = state
			.request_archives(Id(10), Kind::Public, before)
			.unwrap();
		assert!(state.archives.as_ref().unwrap().page.is_none());
		assert!(
			state
				.request_archives(Id(10), Kind::Public, before)
				.is_none()
		);
		state.command_rejected(older);
		assert!(state.gateway_connected);
		state
			.request_archives(Id(10), Kind::Public, before)
			.unwrap();
		let request = state.search_request;
		state.apply_archives(Id(10), request, Ok(page(99, before)));
		assert!(
			state.archives.as_ref().unwrap().error.is_some(),
			"Nonprogressing pages are rejected"
		);
		state
			.request_archives(Id(10), Kind::Public, before)
			.unwrap();
		state.apply_archives(Id(10), state.search_request, Ok(page(99, None)));
		assert_eq!(
			state
				.archives
				.as_ref()
				.unwrap()
				.page
				.as_ref()
				.unwrap()
				.threads[0]
				.id,
			Id(99)
		);
		assert!(state.open_archived_thread(Id(100)).is_none());

		for changed in [false, true] {
			let mutation = |guild, id| {
				if changed {
					Event::ThreadChanged {
						guild: Id(guild),
						patch: model::ChannelPatch {
							icon: model::Patch::Absent,
							id: Id(id),
							name: model::Patch::Value("Updated archive metadata".into()),
							last_message: model::Patch::Absent,
							parent_id: model::Patch::Absent,
							position: model::Patch::Absent,
							kind: model::Patch::Absent,
							message_count: model::Patch::Absent,
						},
					}
				} else {
					Event::ThreadRemoved {
						guild: Id(guild),
						id: Id(id),
					}
				}
			};
			state.request_archives(Id(10), Kind::Public, None).unwrap();
			state.apply_archives(Id(10), state.search_request, Ok(page(99, None)));
			assert!(state.channels.iter().all(|c| c.id != Id(99)));
			apply(&mut state, mutation(2, 99));
			apply(&mut state, mutation(1, 100));
			assert!(
				state.archives.as_ref().unwrap().page.is_some(),
				"Another guild or an unrelated completed row does not invalidate this snapshot"
			);
			apply(&mut state, mutation(1, 99));
			assert!(
				state.archives.is_none(),
				"Unloaded archive rows still receive invalidation"
			);
			assert!(state.open_archived_thread(Id(99)).is_none());

			state.request_archives(Id(10), Kind::Public, None).unwrap();
			let request = state.search_request;
			apply(&mut state, mutation(2, 99));
			assert!(state.archives.as_ref().unwrap().loading);
			apply(&mut state, mutation(1, 500));
			assert!(
				state.archives.is_none(),
				"An in-flight page cannot yet identify its affected rows"
			);
			state.apply_archives(Id(10), request, Ok(page(99, None)));
			assert!(
				state.archives.is_none(),
				"A late page cannot restore a revoked snapshot"
			);
		}

		for invalid in [
			Page {
				threads: (100..126).map(|id| channel(id, Some(Id(10)), 11)).collect(),
				next: None,
			},
			Page {
				threads: vec![channel(100, Some(Id(20)), 11)],
				next: None,
			},
			Page {
				threads: vec![channel(100, Some(Id(10)), 10)],
				next: None,
			},
			Page {
				threads: vec![channel(100, Some(Id(10)), 11); 2],
				next: None,
			},
		] {
			state.request_archives(Id(10), Kind::Public, None).unwrap();
			state.apply_archives(Id(10), state.search_request, Ok(invalid));
			assert!(state.archives.as_ref().unwrap().error.is_some());
			assert!(state.archives.as_ref().unwrap().page.is_none());
		}
		let mut large = page(100, None);
		large.threads[0].name.reserve(64 * 1024);
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		state.apply_archives(Id(10), state.search_request, Ok(large));
		assert!(state.archives.as_ref().unwrap().error.is_some());
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		let request = state.search_request;
		apply(&mut state, Event::PermissionsChanged);
		state.apply_archives(Id(10), request, Ok(page(100, None)));
		assert!(state.archives.is_none());
		crate::tests::grant_permissions(&mut state);
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		let request = state.search_request;
		apply(&mut state, Event::Unavailable(Id(10)));
		state.apply_archives(Id(10), request, Ok(page(100, None)));
		assert!(state.archives.is_none());
	}

	#[test]
	fn opened_archives_are_single_transients_until_gateway_adoption() {
		let mut state = state();
		state.drafts.insert(Id(100), "Preserved draft".into());
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		state.apply_archives(Id(10), state.search_request, Ok(page(100, None)));
		assert!(matches!(
			state.open_archived_thread(Id(100)),
			Some(Command::History {
				channel: Id(100),
				before: None,
				..
			})
		));
		assert!(state.archives.is_none());
		assert_eq!(state.archived_thread, Some(Id(100)));
		let previous_history = state.request;
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		state.apply_archives(Id(10), state.search_request, Ok(page(101, None)));
		state.open_archived_thread(Id(101)).unwrap();
		assert_eq!(state.archived_thread, Some(Id(101)));
		assert!(state.channels.iter().all(|c| c.id != Id(100)));
		assert!(state.request > previous_history);
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		state.apply_archives(Id(10), state.search_request, Ok(page(100, None)));
		state.open_archived_thread(Id(100)).unwrap();
		assert!(state.channels.iter().all(|c| c.id != Id(101)));
		apply(
			&mut state,
			Event::ThreadsSync {
				removed: vec![],
				guild: Id(1),
				parents: None,
				threads: vec![],
			},
		);
		assert!(
			state.channels.iter().any(|c| c.id == Id(100)),
			"Active-list omission is not archive revocation"
		);
		state.clear_archives();
		assert_eq!(state.selected, Some(Id(100)));
		assert!(state.channels.iter().any(|c| c.id == Id(100)));
		state.select(Id(10)).unwrap();
		assert!(state.channels.iter().all(|c| c.id != Id(100)));
		assert_eq!(state.drafts[&Id(100)], "Preserved draft");

		for adopt_with_snapshot in [false, true] {
			state.request_archives(Id(10), Kind::Public, None).unwrap();
			state.apply_archives(Id(10), state.search_request, Ok(page(100, None)));
			state.open_archived_thread(Id(100)).unwrap();
			apply(
				&mut state,
				if adopt_with_snapshot {
					Event::ThreadsSync {
						removed: vec![],
						guild: Id(1),
						parents: None,
						threads: vec![channel(100, Some(Id(10)), 11)],
					}
				} else {
					Event::ChannelCreated(channel(100, Some(Id(10)), 11))
				},
			);
			assert!(state.archived_thread.is_none());
			state.select(Id(10)).unwrap();
			assert!(state.channels.iter().any(|c| c.id == Id(100)));
			state.channels.retain(|c| c.id != Id(100));
		}
		for collision in [
			channel(100, Some(Id(20)), 11),
			channel(100, None, 0),
			Channel {
				guild: Some(Id(2)),
				..channel(100, Some(Id(10)), 11)
			},
		] {
			state.channels.push(collision.clone());
			state.request_archives(Id(10), Kind::Public, None).unwrap();
			state.apply_archives(Id(10), state.search_request, Ok(page(100, None)));
			assert!(state.open_archived_thread(Id(100)).is_none());
			assert!(state.channels.iter().any(|c| c == &collision));
			state.channels.retain(|c| c.id != Id(100));
		}
		state
			.channels
			.extend((1000..(1000 + MAX_NAV - 3) as u64).map(|id| channel(id, None, 0)));
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		state.apply_archives(Id(10), state.search_request, Ok(page(100, None)));
		assert!(
			state.open_archived_thread(Id(100)).is_none(),
			"Account item budget applies to transient insertion"
		);
		state.channels.truncate(2);
		state.channels[1].name.reserve(model::account::MAX_BYTES);
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		state.apply_archives(Id(10), state.search_request, Ok(page(100, None)));
		assert!(
			state.open_archived_thread(Id(100)).is_none(),
			"Account byte budget applies to transient insertion"
		);
		state.channels[1].name = "Synthetic".into();
		state.request_archives(Id(10), Kind::Public, None).unwrap();
		state.apply_archives(Id(10), state.search_request, Ok(page(100, None)));
		state.open_archived_thread(Id(100)).unwrap();
		apply(&mut state, Event::Unavailable(Id(10)));
		assert!(state.channels.iter().all(|c| c.id != Id(100)));
		assert!(state.archived_thread.is_none());
		assert!(!state.history_pending);
		assert_eq!(state.drafts[&Id(100)], "Preserved draft");
		assert_eq!(state.freshness, Freshness::Unavailable);
		assert!(
			state.request_archives(Id(20), Kind::Public, None).is_some(),
			"Another loaded parent remains browsable after selected-thread revocation"
		);
	}

	#[test]
	fn active_threads_are_sorted_newest_first_excluding_archived_and_other_parents() {
		let mut state = state();
		state.channels.extend([
			channel(30, Some(Id(10)), 11),
			channel(31, Some(Id(10)), 11),
			channel(32, Some(Id(20)), 11), // different parent: excluded
			channel(33, Some(Id(10)), 0),  // not a thread kind: excluded
		]);
		state
			.channels
			.iter_mut()
			.find(|c| c.id == Id(30))
			.unwrap()
			.last_message = Some(Id(1000));
		state
			.channels
			.iter_mut()
			.find(|c| c.id == Id(31))
			.unwrap()
			.last_message = Some(Id(2000));
		state.archived_thread = Some(Id(31));
		let active: Vec<Id> = state.active_threads(Id(10)).iter().map(|c| c.id).collect();
		assert_eq!(
			active,
			vec![Id(30)],
			"Newest activity first, current archived-thread transient and other parents excluded"
		);
	}

	#[test]
	fn thread_of_matches_flagged_starter_messages_and_named_system_rows() {
		let mut state = state();
		state
			.channels
			.extend([channel(30, Some(Id(10)), 11), channel(31, Some(Id(10)), 11)]);
		state
			.channels
			.iter_mut()
			.find(|c| c.id == Id(31))
			.unwrap()
			.name = "Introductions".into();

		let mut flagged = message(30, Id(10));
		flagged.flags = 1 << 5;
		assert_eq!(
			state.thread_of(&flagged).map(|c| c.id),
			Some(Id(30)),
			"A HAS_THREAD message shares its id with the thread it started"
		);

		let mut unflagged = message(30, Id(10));
		unflagged.flags = 0;
		assert!(
			state.thread_of(&unflagged).is_none(),
			"Without the flag, sharing an id with a thread is not proof of starting it"
		);

		let mut started = message(999, Id(10));
		started.kind = 18;
		started.content = "Introductions".into();
		assert_eq!(
			state.thread_of(&started).map(|c| c.id),
			Some(Id(31)),
			"Type-18 rows only carry the thread name; match by name against known threads"
		);

		let mut renamed = message(998, Id(10));
		renamed.kind = 18;
		renamed.content = "A name that no longer matches".into();
		assert!(
			state.thread_of(&renamed).is_none(),
			"A renamed thread is not inferred; only an exact name match resolves"
		);

		let mut other_channel = message(997, Id(20));
		other_channel.kind = 18;
		other_channel.content = "Introductions".into();
		assert!(
			state.thread_of(&other_channel).is_none(),
			"A same-named thread under a different parent channel must not match"
		);

		let mut duplicate = channel(32, Some(Id(10)), 11);
		duplicate.name = "Introductions".into();
		state.channels.push(duplicate);
		assert!(
			state.thread_of(&started).is_none(),
			"Two threads share the name, so the system row stays unresolved"
		);
		let mut flagged_duplicate = message(32, Id(10));
		flagged_duplicate.flags = 1 << 5;
		assert_eq!(
			state.thread_of(&flagged_duplicate).map(|c| c.id),
			Some(Id(32)),
			"A flagged starter still resolves by id, whatever the thread is called"
		);
	}
}
