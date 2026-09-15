//! Forum containers list their posts (threads) and create one post at a time.
use crate::{Command, MAX_CONTENT, MAX_NAV, State, auth::AuthState, auth::Failure};
use model::{Channel, Id, permissions as p};

pub const MAX_TITLE: usize = 100;

/// The on-demand active-post list of one forum; the gateway only delivers joined posts.
#[derive(Default)]
pub struct Posts {
	pub parent: Option<Id>,
	pub request: u64,
	pub loading: bool,
	/// Posts admitted so far, used as the search offset of the next page.
	pub loaded: usize,
	pub more: bool,
	pub error: Option<&'static str>,
}

#[derive(Default)]
pub struct Posting {
	pub request: u64,
	pub pending: Option<(Id, u64)>,
	pub error: Option<&'static str>,
	/// A freshly created post the UI should open on its next frame.
	pub created: Option<Id>,
}

impl State {
	pub fn is_forum(&self, channel: Id) -> bool {
		self.channels
			.iter()
			.any(|c| c.id == channel && c.guild.is_some() && matches!(c.kind, 15 | 16))
	}

	/// Loaded posts of a forum: active threads the gateway delivered, newest activity first.
	pub fn forum_posts(&self, parent: Id) -> Vec<&Channel> {
		let mut posts: Vec<_> = self
			.channels
			.iter()
			.filter(|c| c.parent_id == Some(parent) && matches!(c.kind, 11 | 12))
			.collect();
		posts.sort_by_key(|c| std::cmp::Reverse(c.last_message.unwrap_or(c.id)));
		posts
	}

	pub fn can_load_posts(&self, parent: Id) -> bool {
		// Fixtures ship their own posts; a demo session has nothing to fetch them with.
		!self.demo
			&& self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.is_forum(parent)
			&& self.can_read_history(parent)
	}

	/// Loads the first page of a forum, or the next one when `more` is set.
	pub fn request_forum_posts(&mut self, parent: Id, more: bool) -> Option<Command> {
		if !self.can_load_posts(parent) {
			return None;
		}
		let current = self.posts.parent == Some(parent);
		if self.posts.loading && current {
			return None;
		}
		if more {
			if !current || !self.posts.more || self.posts.loaded >= model::forum::MAX_POSTS {
				return None;
			}
		} else if current && (self.posts.loaded > 0 || self.posts.error.is_some()) {
			return None;
		}
		let guild = self.channel(parent)?.guild?;
		let offset = if more { self.posts.loaded } else { 0 };
		self.posts.request = self.posts.request.wrapping_add(1);
		self.posts.parent = Some(parent);
		self.posts.loading = true;
		self.posts.error = None;
		if !more {
			self.posts.loaded = 0;
			self.posts.more = false;
		}
		Some(Command::ForumPosts {
			parent,
			guild,
			offset,
			request: self.posts.request,
		})
	}

	/// Re-arms the loader so the next frame of this forum fetches its posts again.
	pub fn reload_forum_posts(&mut self, parent: Id) {
		if self.posts.parent == Some(parent) && !self.posts.loading {
			// Keep the request counter monotonic so a late reply cannot match a fresh load.
			self.posts = Posts {
				request: self.posts.request,
				..Posts::default()
			};
		}
	}

	pub fn apply_forum_posts(
		&mut self,
		parent: Id,
		request: u64,
		result: Result<model::forum::Page, Failure>,
	) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		if self.posts.parent != Some(parent) || self.posts.request != request || !self.posts.loading
		{
			return;
		}
		self.posts.loading = false;
		let guild = self
			.channel(parent)
			.and_then(|c| c.guild)
			.filter(|_| self.can_load_posts(parent));
		let page = match result {
			Err(failure) => {
				self.posts.error = Some(failure.label());
				return;
			}
			Ok(page) => page,
		};
		let Some(guild) = guild.filter(|guild| page.valid(parent, *guild)) else {
			self.posts.error = Some("The service returned unexpected posts");
			return;
		};
		// Every returned row advances the offset, even one this state already knew.
		let returned = page.threads.len();
		for post in page.threads {
			if let Some(index) = self.channel_index(post.id) {
				let existing = &self.channels[index];
				if existing.parent_id == Some(parent) && existing.guild == Some(guild) {
					let bytes =
						self.navigation_bytes() - existing.name.capacity() + post.name.capacity();
					if bytes + self.permissions.bytes() > model::account::MAX_BYTES {
						self.posts.error = Some("Posts exceed the navigation budget");
						self.posts.more = false;
						return;
					}
					let existing = &mut self.channels[index];
					existing.last_message = existing.last_message.max(post.last_message);
					existing.message_count = post.message_count.or(existing.message_count);
					existing.name = post.name;
					self.set_navigation_bytes(bytes);
				}
				continue;
			}
			let bytes = self.navigation_bytes();
			if self.channels.len() + self.guilds.len() >= MAX_NAV
				|| bytes + post.bytes() + self.permissions.bytes() > model::account::MAX_BYTES
				|| self.channels.try_reserve_exact(1).is_err()
			{
				self.posts.error = Some("Posts exceed the navigation budget");
				self.posts.more = false;
				return;
			}
			self.channels.push(post);
			self.invalidate_navigation();
		}
		self.posts.loaded = self.posts.loaded.saturating_add(returned);
		self.posts.more = page.more && returned > 0;
	}

	pub fn can_create_post(&self, parent: Id) -> bool {
		self.auth == AuthState::Authenticated
			&& self.gateway_connected
			&& self.is_forum(parent)
			&& self.posting.pending.is_none()
			&& self.can_view(parent)
			&& self.permission(parent, p::SEND_MESSAGES) == Some(true)
	}

	pub fn create_post(&mut self, parent: Id, title: &str, content: &str) -> Option<Command> {
		let title = title.trim();
		let content = content.trim();
		if !self.can_create_post(parent)
			|| title.is_empty()
			|| title.chars().count() > MAX_TITLE
			|| content.is_empty()
			|| content.chars().count() > MAX_CONTENT
		{
			return None;
		}
		let guild = self.channel(parent)?.guild?;
		self.posting.request = self.posting.request.wrapping_add(1);
		self.posting.pending = Some((parent, self.posting.request));
		self.posting.error = None;
		Some(Command::CreatePost {
			parent,
			guild,
			title: title.to_owned(),
			content: content.to_owned(),
			request: self.posting.request,
		})
	}

	pub fn apply_post(&mut self, parent: Id, request: u64, result: Result<Channel, Failure>) {
		if self.posting.pending != Some((parent, request)) {
			return;
		}
		self.posting.pending = None;
		let guild = self
			.channels
			.iter()
			.find(|c| c.id == parent)
			.and_then(|c| c.guild);
		let post = match result {
			Err(failure) if failure.ends_session() && failure != Failure::Capacity => {
				self.fail(failure);
				return;
			}
			Err(failure) => {
				self.posting.error = Some(failure.label());
				return;
			}
			Ok(post) => post,
		};
		if guild.is_none()
			|| post.id.0 == 0
			|| post.kind != 11
			|| post.parent_id != Some(parent)
			|| post.guild != guild
			|| post.name.len() > 512
		{
			self.posting.error = Some("The service returned an unexpected post");
			return;
		}
		if let Some(existing) = self.channels.iter_mut().find(|c| c.id == post.id) {
			if existing.parent_id != post.parent_id || existing.guild != post.guild {
				self.posting.error = Some("Post conflicts with current navigation");
				return;
			}
			existing.message_count = post.message_count.or(existing.message_count);
		} else {
			let bytes = self.navigation_bytes();
			if self.channels.len() + self.guilds.len() >= MAX_NAV
				|| bytes + post.bytes() + self.permissions.bytes() > model::account::MAX_BYTES
				|| self.channels.try_reserve_exact(1).is_err()
			{
				self.posting.error = Some("Post exceeds the navigation budget");
				return;
			}
			self.channels.push(post.clone());
			self.invalidate_navigation();
		}
		self.posting.created = Some(post.id);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};

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
			message_count: Some(3),
		}
	}
	fn state() -> State {
		let mut state = State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			user: Some(model::User {
				id: Id(2),
				name: "Synthetic".into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			}),
			guilds: vec![model::Guild {
				emojis: None,
				id: Id(1),
				name: "Synthetic".into(),
				icon: None,
			}],
			channels: vec![
				channel(10, None, 0),
				channel(20, None, 15),
				channel(21, Some(Id(20)), 11),
				channel(22, Some(Id(20)), 11),
			],
			..State::default()
		};
		state.channels[3].last_message = Some(Id(500));
		crate::tests::grant_permissions(&mut state);
		state
	}

	#[test]
	fn forum_title_replacement_updates_budget_and_preserves_original_when_full() {
		let mut state = state();
		let before = state.navigation_bytes();
		let previous_capacity = state.channel(Id(21)).unwrap().name.capacity();
		state.request_forum_posts(Id(20), false).unwrap();
		let mut post = channel(21, Some(Id(20)), 11);
		post.name = "Longer synthetic title".into();
		let expected = before - previous_capacity + post.name.capacity();
		state.apply_forum_posts(
			Id(20),
			state.posts.request,
			Ok(model::forum::Page {
				threads: vec![post],
				more: false,
			}),
		);
		assert_eq!(state.navigation_bytes(), expected);
		state.guilds[0].name = String::with_capacity(
			model::account::MAX_BYTES - expected - state.permissions.bytes()
				+ state.guilds[0].name.capacity()
				- 32,
		);
		state.invalidate_navigation();
		let original = state.channel(Id(21)).unwrap().clone();
		state.reload_forum_posts(Id(20));
		state.request_forum_posts(Id(20), false).unwrap();
		let mut post = original.clone();
		post.name = "x".repeat(128);
		post.last_message = Some(Id(900));
		state.apply_forum_posts(
			Id(20),
			state.posts.request,
			Ok(model::forum::Page {
				threads: vec![post],
				more: true,
			}),
		);
		assert!(state.channel(Id(21)) == Some(&original));
		assert_eq!(
			state.posts.error,
			Some("Posts exceed the navigation budget")
		);
		assert!(!state.posts.more && !state.posts.loading);
		assert!(state.navigation_bytes() + state.permissions.bytes() <= model::account::MAX_BYTES);
	}

	#[test]
	fn forum_posts_load_on_demand_page_forward_and_reload_after_a_sync() {
		let mut state = state();
		// Only joined posts arrive over the gateway, so the list fetches the rest.
		let Some(Command::ForumPosts {
			parent: Id(20),
			guild: Id(1),
			offset: 0,
			request,
		}) = state.request_forum_posts(Id(20), false)
		else {
			panic!("the first page should be requested");
		};
		assert!(state.request_forum_posts(Id(20), false).is_none());
		assert!(state.request_forum_posts(Id(20), true).is_none());
		let page = |ids: &[u64], more| model::forum::Page {
			threads: ids
				.iter()
				.map(|id| channel(*id, Some(Id(20)), 11))
				.collect(),
			more,
		};
		// A stale reply for another request is ignored.
		state.apply_forum_posts(Id(20), request.wrapping_sub(1), Ok(page(&[30], false)));
		assert!(state.channels.iter().all(|c| c.id != Id(30)));
		state.apply_forum_posts(Id(20), request, Ok(page(&[23, 21], true)));
		let posts: Vec<_> = state.forum_posts(Id(20)).iter().map(|c| c.id).collect();
		assert_eq!(posts, vec![Id(22), Id(23), Id(21)]);
		assert_eq!(state.posts.loaded, 2);
		let Some(Command::ForumPosts { offset: 2, .. }) = state.request_forum_posts(Id(20), true)
		else {
			panic!("the next page continues from the loaded count");
		};
		state.apply_forum_posts(Id(20), state.posts.request, Ok(page(&[], false)));
		assert!(!state.posts.more && state.posts.error.is_none());
		assert!(state.request_forum_posts(Id(20), true).is_none());
		// A page scoped to another parent or guild never reaches navigation.
		state.reload_forum_posts(Id(20));
		let request = match state.request_forum_posts(Id(20), false) {
			Some(Command::ForumPosts { request, .. }) => request,
			_ => panic!("a reloaded forum fetches again"),
		};
		let mut foreign = page(&[31], false);
		foreign.threads[0].guild = Some(Id(7));
		state.apply_forum_posts(Id(20), request, Ok(foreign));
		assert_eq!(
			state.posts.error,
			Some("The service returned unexpected posts")
		);
		assert!(state.channels.iter().all(|c| c.id != Id(31)));
		// A failure surfaces once and only a retry clears it.
		state.reload_forum_posts(Id(20));
		let request = match state.request_forum_posts(Id(20), false) {
			Some(Command::ForumPosts { request, .. }) => request,
			_ => panic!("a reloaded forum fetches again"),
		};
		state.apply_forum_posts(Id(20), request, Err(Failure::Capacity));
		assert!(state.posts.error.is_some() && !state.posts.loading);
		assert!(state.request_forum_posts(Id(20), false).is_none());
		// A thread snapshot replaces this scope, so the fetched page must be taken again.
		state
			.apply_threads_sync(Id(1), Some(vec![Id(20)]), vec![], vec![])
			.unwrap();
		assert!(state.posts.error.is_none() && state.posts.loaded == 0);
		assert!(matches!(
			state.request_forum_posts(Id(20), false),
			Some(Command::ForumPosts { offset: 0, .. })
		));
		// A disconnected session asks for nothing.
		state.gateway_connected = false;
		state.posts = Posts::default();
		assert!(!state.can_load_posts(Id(20)));
		assert!(state.request_forum_posts(Id(20), false).is_none());
	}

	#[test]
	fn forum_selection_lists_posts_without_history_and_counts_replies() {
		let mut state = state();
		assert!(state.select(Id(20)).is_none());
		assert_eq!(state.selected, Some(Id(20)));
		assert!(!state.history_pending);
		let posts: Vec<_> = state.forum_posts(Id(20)).iter().map(|c| c.id).collect();
		assert_eq!(posts, vec![Id(22), Id(21)]);
		let message = model::Message {
			reactions: Some(vec![]),
			id: Id(600),
			channel: Id(21),
			author: state.user.clone().unwrap(),
			content: "Synthetic reply".into(),
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
			author_roles: vec![],
			mention_roles: vec![],
			mention_everyone: false,
			suppress_notifications: false,
			mentions: Vec::new(),
			embeds_suppressed: false,
		};
		state.apply(Envelope {
			generation: state.generation,
			event: Event::Message(message),
		});
		let post = state.channels.iter().find(|c| c.id == Id(21)).unwrap();
		assert_eq!(post.message_count, Some(4));
		assert_eq!(post.last_message, Some(Id(600)));
		assert_eq!(state.forum_posts(Id(20))[0].id, Id(21));
	}

	#[test]
	fn posts_are_created_once_validated_and_opened_by_the_ui() {
		let mut state = state();
		assert!(state.create_post(Id(10), "Title", "Body").is_none());
		assert!(state.create_post(Id(20), "", "Body").is_none());
		assert!(state.create_post(Id(20), "Title", " ").is_none());
		let Some(Command::CreatePost { request, .. }) =
			state.create_post(Id(20), " Title ", "Body")
		else {
			panic!("forum post command expected");
		};
		assert!(!state.can_create_post(Id(20)), "One post at a time");
		state.apply_post(
			Id(20),
			request.wrapping_add(1),
			Ok(channel(30, Some(Id(20)), 11)),
		);
		assert!(state.posting.pending.is_some());
		state.apply_post(Id(20), request, Ok(channel(30, Some(Id(10)), 11)));
		assert!(state.posting.error.is_some());
		assert!(state.channels.iter().all(|c| c.id != Id(30)));
		let Some(Command::CreatePost { request, .. }) = state.create_post(Id(20), "Title", "Body")
		else {
			panic!("forum post command expected");
		};
		state.apply_post(Id(20), request, Ok(channel(30, Some(Id(20)), 11)));
		assert_eq!(state.posting.created.take(), Some(Id(30)));
		assert!(state.channels.iter().any(|c| c.id == Id(30)));
		assert!(state.select(Id(30)).is_some());
		let command = state.create_post(Id(20), "Title", "Body").unwrap();
		state.command_rejected(command);
		assert!(state.posting.pending.is_none());
		assert!(state.posting.error.is_some());
	}
}
