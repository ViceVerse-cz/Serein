use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Freshness, Id, SearchPage};

pub struct SearchView {
	pub pins: bool,
	/// Conversation the search was started from; results route back to it.
	pub channel: Id,
	/// Server-wide text searches keep their results while the selection stays in this server.
	pub guild: Option<Id>,
	pub query: String,
	pub before: Option<Id>,
	pub offset: u32,
	/// Retained during navigation and failures without retaining previous result pages.
	pub total: Option<u64>,
	pub pin_before: Option<i128>,
	pub request: u64,
	pub loading: bool,
	pub error: Option<&'static str>,
	pub page: Option<SearchPage>,
}

impl SearchView {
	pub fn page_count(&self) -> u32 {
		self.total
			.unwrap_or(0)
			.div_ceil(model::SEARCH_PAGE_SIZE as u64)
			.clamp(
				1,
				u64::from(model::MAX_SEARCH_OFFSET) / model::SEARCH_PAGE_SIZE as u64 + 1,
			) as u32
	}
}
pub enum Outcome {
	Page(SearchPage),
	Pins(SearchPage),
	Indexing,
}
impl State {
	/// Include retained hits as well as channels known to belong to the searched server.
	pub(crate) fn search_covers_channel(&self, channel: Id) -> bool {
		self.search.as_ref().is_some_and(|view| {
			view.channel == channel
				|| view
					.page
					.as_ref()
					.is_some_and(|page| page.hits.iter().any(|hit| hit.channel == channel))
				|| (!view.pins
					&& view.guild.is_some_and(|guild| {
						// The bounded pending page may contain an archived/unresolved channel.
						// Retire it on an unscoped mutation rather than restore an old snippet.
						// A later explicit search can still display legitimate unresolved hits.
						self.channel(channel)
							.map_or(view.loading, |known| known.guild == Some(guild))
					}))
		})
	}
	/// Guild queries span sibling channels and inherited thread permissions. Their loaded
	/// and pending snapshots must both be retired when this server's access inputs change.
	pub(crate) fn search_access_changed(&self, event: &crate::Event) -> bool {
		use crate::{Event, permissions::Event as PermissionEvent};
		let Some(guild) = self
			.search
			.as_ref()
			.filter(|view| !view.pins)
			.and_then(|view| view.guild)
		else {
			return false;
		};
		match event {
			Event::Permissions(PermissionEvent::Snapshot(snapshot)) => {
				snapshot.guilds.iter().any(|entry| entry.id == guild)
			}
			Event::Permissions(PermissionEvent::Guild(entry)) => entry.id == guild,
			Event::Permissions(PermissionEvent::Members(members)) => {
				members.iter().any(|(id, _, _)| *id == guild)
			}
			Event::Permissions(
				PermissionEvent::Role { guild: id, .. }
				| PermissionEvent::RoleRemoved { guild: id, .. }
				| PermissionEvent::Member { guild: id, .. }
				| PermissionEvent::Owner { guild: id, .. }
				| PermissionEvent::UnavailableGuild(id),
			)
			| Event::GuildRemoved(id)
			| Event::ThreadRemoved { guild: id, .. } => *id == guild,
			Event::Permissions(PermissionEvent::Channel {
				channel,
				guild: scope,
				..
			}) => scope.map_or_else(|| self.search_covers_channel(*channel), |id| id == guild),
			Event::ChannelChanged(patch)
				if !matches!(patch.kind, model::Patch::Absent)
					|| !matches!(patch.parent_id, model::Patch::Absent) =>
			{
				self.search_covers_channel(patch.id)
			}
			Event::ThreadChanged { guild: id, patch }
				if !matches!(patch.kind, model::Patch::Absent)
					|| !matches!(patch.parent_id, model::Patch::Absent) =>
			{
				*id == guild
			}
			_ => false,
		}
	}
	pub fn can_search(&self) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.freshness != Freshness::Unavailable
			&& self.channels.iter().any(|c| {
				Some(c.id) == self.selected && c.supports_text() && self.can_read_history(c.id)
			})
	}
	pub fn request_search(&mut self, query: String, before: Option<Id>) -> Option<Command> {
		if !self.can_search() || !model::valid_search_query(&query) {
			return None;
		}
		let channel = self.channel(self.selected?)?;
		let (channel, guild) = (channel.id, channel.guild);
		if before.is_some()
			&& !self.search.as_ref().is_some_and(|s| {
				!s.pins
					&& s.channel == channel
					&& s.query == query
					&& !s.loading && s.page.as_ref().and_then(|p| p.hits.last()).map(|h| h.id) == before
			}) {
			return None;
		}
		self.search_request = self.search_request.wrapping_add(1);
		self.archives = None;
		self.search = Some(SearchView {
			pins: false,
			channel,
			guild,
			query: query.clone(),
			before,
			offset: 0,
			total: None,
			pin_before: None,
			request: self.search_request,
			loading: true,
			error: None,
			page: None,
		});
		Some(Command::Search {
			channel,
			guild,
			query,
			before,
			offset: 0,
			request: self.search_request,
		})
	}
	/// Select a zero-based page within the original query and cursor scope.
	pub fn request_search_page(&mut self, page: u32) -> Option<Command> {
		if !self.can_search() {
			return None;
		}
		let view = self.search.as_ref()?;
		if view.pins || view.loading || !self.search_in_scope(view) || page >= view.page_count() {
			return None;
		}
		let offset = page.checked_mul(model::SEARCH_PAGE_SIZE as u32)?;
		if offset > model::MAX_SEARCH_OFFSET {
			return None;
		}
		let guild = view.guild;
		self.search_request = self.search_request.wrapping_add(1);
		self.archives = None;
		let view = self.search.as_mut()?;
		view.offset = offset;
		view.request = self.search_request;
		view.loading = true;
		view.error = None;
		view.page = None;
		Some(Command::Search {
			channel: view.channel,
			guild,
			query: view.query.clone(),
			before: view.before,
			offset,
			request: view.request,
		})
	}
	pub fn request_pins(&mut self) -> Option<Command> {
		self.request_pins_page(None)
	}
	pub fn request_older_pins(&mut self) -> Option<Command> {
		let view = self.search.as_ref()?;
		if !view.pins || view.loading || Some(view.channel) != self.selected {
			return None;
		}
		let before = if view.error.is_some() {
			view.pin_before?
		} else {
			view.page.as_ref()?.pin_cursor?
		};
		self.request_pins_page(Some(before))
	}
	fn request_pins_page(&mut self, before: Option<i128>) -> Option<Command> {
		if !self.can_search() {
			return None;
		}
		let channel = self.selected?;
		self.search_request = self.search_request.wrapping_add(1);
		self.archives = None;
		self.search = Some(SearchView {
			pins: true,
			channel,
			guild: None,
			query: String::new(),
			before: None,
			offset: 0,
			total: None,
			pin_before: before,
			request: self.search_request,
			loading: true,
			error: None,
			page: None,
		});
		Some(Command::Pins {
			channel,
			before,
			request: self.search_request,
		})
	}
	pub fn clear_search(&mut self) -> Command {
		self.search_request = self.search_request.wrapping_add(1);
		self.search = None;
		self.archives = None;
		Command::CancelSearch
	}
	pub fn apply_search(&mut self, channel: Id, request: u64, result: Result<Outcome, Failure>) {
		if let Err(f) = &result
			&& f.ends_session()
			&& *f != Failure::Capacity
		{
			self.fail(*f);
			return;
		}
		if !self.can_search()
			|| !self
				.search
				.as_ref()
				.is_some_and(|view| self.search_in_scope(view))
		{
			return;
		}
		// Recheck resolved channels before borrowing the view mutably: a fresh service/index
		// response can still contain a now-forbidden sibling. Reject the page as a whole so
		// totals keep their meaning; unresolved archived channels are allowed to load normally.
		let page_accessible = match (&result, self.search.as_ref()) {
			(Ok(Outcome::Page(page)), Some(view)) => {
				page.hits.len() <= model::SEARCH_PAGE_SIZE
					&& page.hits.iter().all(|hit| {
						self.channel(hit.channel).is_none_or(|known| {
							known.guild == view.guild && self.can_read_history(known.id)
						})
					})
			}
			_ => true,
		};
		let Some(view) = self
			.search
			.as_mut()
			.filter(|s| s.channel == channel && s.request == request && s.loading)
		else {
			return;
		};
		view.loading = false;
		let scope = view.guild.is_none().then_some(channel);
		match result {
			Ok(Outcome::Page(page))
				if !view.pins && page_accessible && page.valid(scope, view.before) =>
			{
				view.total = Some(page.total);
				view.page = Some(page);
				view.error = None;
			}
			Ok(Outcome::Pins(page))
				if view.pins
					&& page.valid_pins(channel)
					&& page.partial == page.pin_cursor.is_some()
					&& page.pin_cursor.is_none_or(|cursor| {
						!page.hits.is_empty() && view.pin_before.is_none_or(|b| cursor < b)
					}) =>
			{
				self.message_actions
					.reconcile_pins(channel, &page, view.pin_before.is_none());
				view.page = Some(page);
				view.error = None;
			}
			Ok(Outcome::Indexing) if !view.pins => {
				view.error = Some("Discord is indexing this conversation. Try Search again later.")
			}
			Ok(_) => view.error = Some("Message results were invalid or too large"),
			Err(f) => view.error = Some(f.label()),
		}
	}
	/// Server-wide results stay valid while the selection remains in the searched server.
	fn search_in_scope(&self, view: &SearchView) -> bool {
		Some(view.channel) == self.selected
			|| view.guild.is_some_and(|guild| {
				!view.pins
					&& self
						.selected
						.and_then(|id| self.channel(id))
						.is_some_and(|channel| channel.guild == Some(guild))
			})
	}
	pub fn open_search_hit(&mut self, message: Id) -> Option<Command> {
		if !self.can_search() {
			return None;
		}
		let view = self.search.as_ref()?;
		if !self.search_in_scope(view) {
			return None;
		}
		let channel = view
			.page
			.as_ref()?
			.hits
			.iter()
			.find(|h| h.id == message)?
			.channel;
		if Some(channel) == self.selected {
			return self.open_target_window(message);
		}
		let guild = view.guild?;
		// Opening another channel clears search; keep the results beside it like Discord does.
		let view = self.search.take();
		let result = self.open_chat_link(Some(guild), channel, Some(message));
		self.search = view;
		result.unwrap_or_else(|status| {
			self.status = status;
			None
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};
	use model::{Channel, Guild, SearchHit};

	fn state() -> State {
		let channels = [(1, Some(10)), (2, Some(10)), (3, Some(20)), (4, None)]
			.into_iter()
			.map(|(id, guild)| Channel {
				id: Id(id),
				guild: guild.map(Id),
				parent_id: None,
				position: 0,
				name: "Synthetic".into(),
				kind: if guild.is_some() { 0 } else { 1 },
				recipients: vec![],
				last_message: None,
				icon: None,
				member_list_id: None,
				tags: None,
				message_count: None,
			})
			.collect();
		let mut state = State {
			user: Some(crate::tests::message(100).author),
			channels,
			guilds: [10, 20]
				.into_iter()
				.map(|id| Guild {
					id: Id(id),
					name: "Synthetic".into(),
					icon: None,
					default_message_notifications: None,
					emojis: None,
					stickers: None,
				})
				.collect(),
			selected: Some(Id(1)),
			auth: AuthState::Authenticated,
			gateway_connected: true,
			freshness: Freshness::Fresh,
			..State::default()
		};
		crate::tests::grant_permissions(&mut state);
		state
	}
	fn page(channel: Id) -> SearchPage {
		SearchPage {
			hits: vec![SearchHit {
				id: Id(100),
				channel,
				author: crate::tests::message(100).author,
				mentions: vec![],
				excerpt: "Synthetic old snippet".into(),
				attachments: vec![],
				embeds: vec![],
			}],
			total: 1,
			partial: false,
			pin_cursor: None,
		}
	}
	fn mutation(channel: Id, kind: usize) -> Event {
		match kind {
			0 => Event::Patch(crate::message_actions::content_patch(
				channel,
				Id(100),
				"Edited".into(),
			)),
			1 => Event::Delete {
				channel,
				id: Id(100),
			},
			_ => Event::DeleteBulk {
				channel,
				ids: vec![Id(100)],
			},
		}
	}
	fn apply(state: &mut State, event: Event) {
		state.apply(Envelope {
			generation: state.generation,
			event,
		});
	}

	#[test]
	fn sibling_channel_mutations_retire_loaded_and_in_flight_guild_searches() {
		for in_flight in [false, true] {
			for kind in 0..3 {
				let mut state = state();
				let Some(Command::Search {
					channel, request, ..
				}) = state.request_search("snippet".into(), None)
				else {
					panic!("Expected guild search");
				};
				if !in_flight {
					state.apply_search(channel, request, Ok(Outcome::Page(page(Id(2)))));
					assert!(state.search.as_ref().unwrap().page.is_some());
				}
				apply(&mut state, mutation(Id(2), kind));
				assert!(state.search.is_none());
				assert!(state.search_request > request);
				state.apply_search(channel, request, Ok(Outcome::Page(page(Id(2)))));
				assert!(
					state.search.is_none(),
					"A stale index must not restore the snippet"
				);
			}
		}
	}

	#[test]
	fn mutations_outside_the_search_scope_preserve_results_and_pending_reads() {
		for in_flight in [false, true] {
			for unrelated in [Id(3), Id(4)] {
				let mut state = state();
				let Some(Command::Search {
					channel, request, ..
				}) = state.request_search("snippet".into(), None)
				else {
					panic!("Expected guild search");
				};
				if !in_flight {
					state.apply_search(channel, request, Ok(Outcome::Page(page(Id(2)))));
				}
				apply(&mut state, mutation(unrelated, 1));
				assert_eq!(state.search.as_ref().unwrap().request, request);
				assert_eq!(state.search.as_ref().unwrap().loading, in_flight);
				if in_flight {
					state.apply_search(channel, request, Ok(Outcome::Page(page(Id(2)))));
				}
				assert!(state.search.as_ref().unwrap().page.is_some());
			}
		}
		let mut state = state();
		let Some(Command::Pins {
			channel, request, ..
		}) = state.request_pins()
		else {
			panic!("Expected pins");
		};
		apply(&mut state, mutation(Id(2), 1));
		assert!(state.search.as_ref().unwrap().loading);
		state.apply_search(channel, request, Ok(Outcome::Pins(page(channel))));
		assert!(state.search.as_ref().unwrap().page.is_some());
		apply(&mut state, mutation(channel, 1));
		assert!(state.search.is_none());
	}

	fn limited_state() -> State {
		let mut state = state();
		let mut thread = state.channels[0].clone();
		thread.id = Id(5);
		thread.parent_id = Some(Id(2));
		thread.kind = 11;
		state.channels.push(thread);
		state
			.permissions
			.replace(model::permissions::Snapshot {
				guilds: [10, 20]
					.into_iter()
					.map(|id| model::permissions::Guild {
						id: Id(id),
						owner: Some(Id(99)),
						roles: Some(vec![model::permissions::Role {
							id: Id(id),
							bits: model::permissions::VIEW_CHANNEL
								| model::permissions::READ_MESSAGE_HISTORY,
							name: "Synthetic everyone".into(),
							color: 0,
							secondary_color: None,
							tertiary_color: None,
							position: 0,
							hoist: false,
						}]),
						member: Some(model::permissions::Member {
							roles: vec![],
							timeout_until: None,
						}),
					})
					.collect(),
				channels: state
					.channels
					.iter()
					.filter_map(|channel| {
						Some(model::permissions::Channel {
							id: channel.id,
							guild: channel.guild?,
							overwrites: Some(vec![]),
						})
					})
					.collect(),
			})
			.unwrap();
		state
	}

	fn history_overwrite(channel: Id, guild: Id, deny: u128) -> Event {
		Event::Permissions(crate::permissions::Event::Channel {
			channel,
			guild: Some(guild),
			overwrites: model::Patch::Value(vec![model::permissions::Overwrite {
				id: guild,
				kind: 0,
				allow: 0,
				deny,
			}]),
		})
	}

	fn start(state: &mut State, loaded: bool, hit_channel: Id) -> (Id, u64) {
		let Some(Command::Search {
			channel, request, ..
		}) = state.request_search("snippet".into(), None)
		else {
			panic!("Expected search");
		};
		if loaded {
			state.apply_search(channel, request, Ok(Outcome::Page(page(hit_channel))));
		}
		(channel, request)
	}

	fn assert_retired(state: &mut State, channel: Id, request: u64, hit_channel: Id) {
		assert!(state.search.is_none());
		assert!(state.search_request > request);
		state.apply_search(channel, request, Ok(Outcome::Page(page(hit_channel))));
		assert!(
			state.search.is_none(),
			"A stale read cannot restore snippets"
		);
	}

	#[test]
	fn sibling_and_inherited_history_access_changes_retire_guild_searches() {
		for loaded in [false, true] {
			for hit_channel in [Id(2), Id(5)] {
				for deny in [
					model::permissions::READ_MESSAGE_HISTORY,
					model::permissions::VIEW_CHANNEL,
				] {
					let mut state = limited_state();
					let (channel, request) = start(&mut state, loaded, hit_channel);
					apply(&mut state, history_overwrite(Id(2), Id(10), deny));
					assert!(state.can_search(), "The selected channel keeps its access");
					assert!(!state.can_read_history(hit_channel));
					assert_retired(&mut state, channel, request, hit_channel);
				}
			}
		}
	}

	#[test]
	fn guild_role_access_changes_retire_search_when_selected_access_is_unchanged() {
		for loaded in [false, true] {
			let mut state = limited_state();
			state
				.permissions
				.channels
				.get_mut(&Id(1))
				.unwrap()
				.overwrites = Some(vec![model::permissions::Overwrite {
				id: Id(10),
				kind: 0,
				allow: model::permissions::VIEW_CHANNEL | model::permissions::READ_MESSAGE_HISTORY,
				deny: 0,
			}]);
			let (channel, request) = start(&mut state, loaded, Id(5));
			let mut everyone = state.permissions.guilds[&Id(10)].roles.as_ref().unwrap()[0].clone();
			everyone.bits = model::permissions::VIEW_CHANNEL;
			apply(
				&mut state,
				Event::Permissions(crate::permissions::Event::Role {
					guild: Id(10),
					role: everyone,
				}),
			);
			assert!(state.can_search());
			assert!(!state.can_read_history(Id(5)));
			assert_retired(&mut state, channel, request, Id(5));
		}
	}

	#[test]
	fn fresh_search_rejects_resolved_inaccessible_and_foreign_channel_hits() {
		for hit_channel in [Id(2), Id(3), Id(4)] {
			let mut state = limited_state();
			apply(
				&mut state,
				history_overwrite(Id(2), Id(10), model::permissions::READ_MESSAGE_HISTORY),
			);
			let (channel, request) = start(&mut state, false, hit_channel);
			state.apply_search(channel, request, Ok(Outcome::Page(page(hit_channel))));
			let view = state.search.as_ref().unwrap();
			assert!(!view.loading);
			assert!(
				view.page.is_none(),
				"A known forbidden/foreign hit cannot appear"
			);
			assert!(
				view.total.is_none(),
				"Do not publish misleading filtered totals"
			);
			assert_eq!(
				view.error,
				Some("Message results were invalid or too large")
			);
		}
		let mut state = limited_state();
		let (channel, request) = start(&mut state, false, Id(20));
		state.apply_search(channel, request, Ok(Outcome::Page(page(Id(20)))));
		assert!(state.channel(Id(20)).is_none());
		assert_eq!(state.search.unwrap().page.unwrap().hits[0].channel, Id(20));
	}

	#[test]
	fn sibling_channel_and_unresolved_thread_removals_retire_guild_searches() {
		for loaded in [false, true] {
			for (hit_channel, event) in [
				(Id(2), Event::Unavailable(Id(2))),
				(
					Id(20),
					Event::ThreadRemoved {
						guild: Id(10),
						id: Id(20),
					},
				),
			] {
				let mut state = limited_state();
				let (channel, request) = start(&mut state, loaded, hit_channel);
				apply(&mut state, event);
				assert!(state.can_search());
				assert_retired(&mut state, channel, request, hit_channel);
			}
		}
	}

	#[test]
	fn unresolved_channel_mutations_retire_loaded_and_in_flight_guild_searches() {
		for loaded in [false, true] {
			for kind in 0..3 {
				let mut state = limited_state();
				let (channel, request) = start(&mut state, loaded, Id(20));
				assert!(state.channel(Id(20)).is_none());
				apply(&mut state, mutation(Id(20), kind));
				assert_retired(&mut state, channel, request, Id(20));
			}
		}
		let mut state = limited_state();
		let (channel, request) = start(&mut state, false, Id(20));
		state.apply_search(channel, request, Ok(Outcome::Page(page(Id(20)))));
		assert_eq!(state.search.unwrap().page.unwrap().hits[0].channel, Id(20));
	}

	#[test]
	fn access_changes_outside_guild_search_and_channel_pins_preserve_reads() {
		for loaded in [false, true] {
			for event in [
				history_overwrite(Id(3), Id(20), model::permissions::READ_MESSAGE_HISTORY),
				Event::Permissions(crate::permissions::Event::UnavailableGuild(Id(20))),
				Event::Unavailable(Id(3)),
				Event::ThreadRemoved {
					guild: Id(20),
					id: Id(20),
				},
				mutation(Id(4), 1),
			] {
				let mut state = limited_state();
				let (channel, request) = start(&mut state, loaded, Id(2));
				apply(&mut state, event);
				assert_eq!(state.search.as_ref().unwrap().request, request);
				if !loaded {
					state.apply_search(channel, request, Ok(Outcome::Page(page(Id(2)))));
				}
				assert!(state.search.as_ref().unwrap().page.is_some());
			}
		}
		for (selected, pins) in [(Id(4), false), (Id(4), true), (Id(1), true)] {
			let mut state = limited_state();
			state.selected = Some(selected);
			let (channel, request) = if pins {
				let Some(Command::Pins {
					channel, request, ..
				}) = state.request_pins()
				else {
					panic!("Expected pins");
				};
				(channel, request)
			} else {
				start(&mut state, false, selected)
			};
			apply(
				&mut state,
				history_overwrite(Id(2), Id(10), model::permissions::READ_MESSAGE_HISTORY),
			);
			apply(&mut state, mutation(Id(20), 1));
			assert_eq!(state.search.as_ref().unwrap().request, request);
			state.apply_search(
				channel,
				request,
				Ok(if pins {
					Outcome::Pins(page(selected))
				} else {
					Outcome::Page(page(selected))
				}),
			);
			assert!(state.search.as_ref().unwrap().page.is_some());
		}
	}
}
