//! Provider GIF browsing and bounded account favorites with an offline local fallback.
use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Gif, GifPage, MAX_GIF_FAVORITES};
use std::{
	collections::VecDeque,
	time::{Duration, Instant},
};

const GIF_CACHE_PAGES: usize = 8;
const GIF_CACHE_BYTES: usize = 768 * 1024;
const GIF_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

struct CachedPage {
	query: Option<String>,
	page: GifPage,
	loaded: Instant,
}
impl CachedPage {
	fn bytes(&self) -> usize {
		size_of::<Self>() + self.query.as_ref().map_or(0, String::capacity) + self.page.bytes()
	}
}

pub struct View {
	/// `None` loads trending GIFs and categories.
	pub query: Option<String>,
	pub request: u64,
	pub loading: bool,
	pub error: Option<&'static str>,
	pub page: Option<GifPage>,
}
#[derive(Default)]
pub struct Gifs {
	pub view: Option<View>,
	pub request: u64,
	pub favorites: Vec<Gif>,
	/// Set when displayed favorites changed; the host persists the bounded fallback and clears it.
	pub favorites_changed: bool,
	favorites_restored: bool,
	favorites_edited: bool,
	pub sync_attempted: bool,
	pub sync_ready: bool,
	pub sync_pending: Option<u64>,
	pub sync_error: Option<&'static str>,
	sync_loaded: bool,
	sync_request: u64,
	sync_command: Option<(Gif, bool)>,
	sync_remote: Vec<String>,
	// Until the initial cache arrives, remember removals instead of reintroducing them.
	sync_removed_before_restore: Vec<String>,
	sync_restore_ambiguous: bool,
	cache: VecDeque<CachedPage>,
	cache_bytes: usize,
}
impl Gifs {
	pub fn bytes(&self) -> usize {
		self.favorites.capacity() * size_of::<Gif>()
			+ self
				.favorites
				.iter()
				.map(|gif| gif.bytes() - size_of::<Gif>())
				.sum::<usize>()
			+ self
				.view
				.as_ref()
				.and_then(|view| view.page.as_ref())
				.map_or(0, GifPage::bytes)
			+ self.cache_bytes
			+ self.sync_command.as_ref().map_or(0, |(gif, _)| gif.bytes())
			+ self.sync_remote.capacity() * size_of::<String>()
			+ self.sync_remote.iter().map(String::capacity).sum::<usize>()
			+ self.sync_removed_before_restore.capacity() * size_of::<String>()
			+ self
				.sync_removed_before_restore
				.iter()
				.map(String::capacity)
				.sum::<usize>()
	}
	fn take_cached(&mut self, query: &Option<String>) -> Option<(GifPage, bool)> {
		let index = self.cache.iter().position(|entry| &entry.query == query)?;
		let entry = self.cache.remove(index)?;
		self.cache_bytes = self.cache_bytes.saturating_sub(entry.bytes());
		let fresh = entry.loaded.elapsed() < GIF_CACHE_TTL;
		let page = entry.page.clone();
		self.cache_bytes += entry.bytes();
		self.cache.push_back(entry);
		Some((page, fresh))
	}
	fn cache_page(&mut self, query: Option<String>, page: GifPage) {
		if let Some(index) = self.cache.iter().position(|entry| entry.query == query)
			&& let Some(entry) = self.cache.remove(index)
		{
			self.cache_bytes = self.cache_bytes.saturating_sub(entry.bytes());
		}
		let entry = CachedPage {
			query,
			page,
			loaded: Instant::now(),
		};
		self.cache_bytes += entry.bytes();
		self.cache.push_back(entry);
		while self.cache.len() > GIF_CACHE_PAGES || self.cache_bytes > GIF_CACHE_BYTES {
			let Some(entry) = self.cache.pop_front() else {
				break;
			};
			self.cache_bytes = self.cache_bytes.saturating_sub(entry.bytes());
		}
	}
}
impl State {
	pub fn can_browse_gifs(&self) -> bool {
		self.auth == AuthState::Authenticated && self.gateway_connected
	}
	/// One request at a time; a repeated identical query reuses the loaded page.
	pub fn request_gifs(&mut self, query: Option<&str>) -> Option<Command> {
		if !self.can_browse_gifs() {
			return None;
		}
		let query = match query {
			Some(query) => {
				let query = query.trim();
				if !model::valid_search_query(query) {
					return None;
				}
				Some(query.to_owned())
			}
			None => None,
		};
		if self
			.gifs
			.view
			.as_ref()
			.is_some_and(|view| view.query == query && (view.loading || view.error.is_none()))
		{
			return None;
		}
		let cached = self.gifs.take_cached(&query);
		let fresh = cached.as_ref().is_some_and(|(_, fresh)| *fresh);
		self.gifs.request = self.gifs.request.wrapping_add(1);
		self.gifs.view = Some(View {
			query: query.clone(),
			request: self.gifs.request,
			loading: !fresh,
			error: None,
			page: cached.map(|(page, _)| page),
		});
		if fresh {
			return None;
		}
		Some(Command::Gifs {
			query,
			request: self.gifs.request,
		})
	}
	pub fn clear_gifs(&mut self) -> Option<Command> {
		let loading = self.gifs.view.as_ref().is_some_and(|view| view.loading);
		self.gifs.view = None;
		loading.then_some(Command::CancelGifs)
	}
	pub fn apply_gifs(&mut self, request: u64, result: Result<GifPage, Failure>) {
		if let Err(failure) = &result
			&& failure.ends_session()
			&& *failure != Failure::Capacity
		{
			self.fail(*failure);
			return;
		}
		let mut accepted = None;
		{
			let Some(view) = self
				.gifs
				.view
				.as_mut()
				.filter(|view| view.request == request && view.loading)
			else {
				return;
			};
			view.loading = false;
			match result {
				Ok(page) if page.valid() => {
					accepted = Some((view.query.clone(), page.clone()));
					view.page = Some(page);
					view.error = None;
				}
				Ok(_) | Err(Failure::Protocol | Failure::ProtocolAt(_)) => {
					view.error = Some("GIF results were rejected or incompatible");
				}
				Err(Failure::RateLimited) => {
					view.error = Some("GIF search is rate limited; wait a moment and retry");
				}
				Err(Failure::Forbidden) => {
					view.error = Some("GIF search is unavailable for this account");
				}
				Err(Failure::Capacity) => {
					view.error = Some("GIF results exceeded the safe size limit");
				}
				Err(_) => view.error = Some("GIF search failed; check the connection and retry"),
			}
		}
		if let Some((query, page)) = accepted {
			self.gifs.cache_page(query, page);
		}
	}
	pub fn is_gif_favorite(&self, gif: &Gif) -> bool {
		self.gifs.favorites.iter().any(|known| known.url == gif.url)
	}
	/// Call before the host replaces or shuts down its account connection.
	/// Retain local metadata, but never replay an interrupted server write.
	pub fn interrupt_gif_favorites(&mut self) {
		self.gifs.sync_pending = None;
		self.gifs.sync_command = None;
		self.gifs.sync_ready = false;
		self.gifs.sync_attempted = false;
		self.gifs.sync_error = None;
	}
	pub fn request_gif_favorites(&mut self) -> Option<Command> {
		if !self.can_browse_gifs() || self.gifs.sync_pending.is_some() {
			return None;
		}
		self.gifs.sync_attempted = true;
		self.gifs.sync_request = self.gifs.sync_request.wrapping_add(1);
		self.gifs.sync_pending = Some(self.gifs.sync_request);
		self.gifs.sync_error = None;
		Some(Command::GifFavorites {
			request: self.gifs.sync_request,
			change: None,
		})
	}
	/// One explicit star change, collected by the host after widgets have finished this frame.
	pub fn take_gif_favorites_command(&mut self) -> Option<Command> {
		self.gifs
			.sync_command
			.take()
			.map(|change| Command::GifFavorites {
				request: self.gifs.sync_request,
				change: Some(change),
			})
	}
	pub fn apply_gif_favorites(&mut self, request: u64, result: Result<Vec<Gif>, Failure>) {
		if self.gifs.sync_pending != Some(request) {
			return;
		}
		self.gifs.sync_pending = None;
		self.gifs.sync_command = None;
		match result {
			Ok(favorites)
				if favorites.len() <= MAX_GIF_FAVORITES
					&& favorites.capacity() * size_of::<Gif>()
						+ favorites
							.iter()
							.map(|gif| gif.bytes() - size_of::<Gif>())
							.sum::<usize>() <= 192 * 1024
					&& favorites.iter().all(Gif::valid)
					&& favorites.iter().enumerate().all(|(i, gif)| {
						favorites[..i].iter().all(|other| other.url != gif.url)
					}) =>
			{
				let previous_remote = std::mem::replace(
					&mut self.gifs.sync_remote,
					favorites.iter().map(|gif| gif.url.clone()).collect(),
				);
				if !self.gifs.favorites_restored {
					for url in &previous_remote {
						if !self.gifs.sync_remote.contains(url)
							&& !self.gifs.sync_removed_before_restore.contains(url)
						{
							if self.gifs.sync_removed_before_restore.len() == MAX_GIF_FAVORITES {
								self.gifs.sync_restore_ambiguous = true;
								break;
							}
							self.gifs.sync_removed_before_restore.push(url.clone());
						}
					}
				}
				let previous = std::mem::take(&mut self.gifs.favorites);
				// Keep distinct local fallback entries before filling remaining remote slots.
				for gif in previous {
					// A local still preview outranks the synchronized clip for the same URL,
					// on every sync while the account still lists it.
					let local_preview = model::valid_gif_preview(&gif.preview)
						&& favorites.iter().any(|remote| remote.url == gif.url);
					if !previous_remote.contains(&gif.url) || local_preview {
						let current = favorites
							.iter()
							.find(|remote| remote.url == gif.url && !local_preview);
						self.gifs.favorites.push(current.cloned().unwrap_or(gif));
					}
				}
				for gif in favorites {
					if self.gifs.favorites.len() == MAX_GIF_FAVORITES {
						break;
					}
					if !self.is_gif_favorite(&gif) {
						self.gifs.favorites.push(gif);
					}
				}
				self.gifs.favorites.shrink_to_fit();
				self.gifs.sync_ready = true;
				self.gifs.sync_loaded = true;
				self.gifs.sync_error = None;
				self.gifs.favorites_changed = true;
			}
			Err(failure) if failure.ends_session() && failure != Failure::Capacity => {
				self.fail(failure)
			}
			_ => {
				self.gifs.sync_ready = false;
				self.gifs.sync_error = Some("gif-favorites-sync-failed");
			}
		}
	}
	/// Newest favorite first; the list is bounded and never holds rejected entries.
	pub fn toggle_gif_favorite(&mut self, gif: &Gif) -> bool {
		if !gif.valid() || self.gifs.sync_pending.is_some() {
			return false;
		}
		let favorite = !self.is_gif_favorite(gif);
		if let Some(index) = self
			.gifs
			.favorites
			.iter()
			.position(|known| known.url == gif.url)
		{
			self.gifs.favorites.remove(index);
		} else if gif.valid() {
			self.gifs.favorites.insert(0, gif.clone());
			self.gifs.favorites.truncate(MAX_GIF_FAVORITES);
		} else {
			return false;
		}
		self.gifs.favorites_changed = true;
		self.gifs.favorites_edited = true;
		if self.gifs.sync_ready && self.can_browse_gifs() {
			self.gifs.sync_request = self.gifs.sync_request.wrapping_add(1);
			self.gifs.sync_pending = Some(self.gifs.sync_request);
			self.gifs.sync_command = Some((gif.clone(), favorite));
			self.gifs.sync_error = None;
		}
		true
	}
	/// A late initial cache read merges with synchronized records, but cannot undo a user's star.
	pub fn restore_gif_favorites(&mut self, favorites: Vec<Gif>) {
		let removed = std::mem::take(&mut self.gifs.sync_removed_before_restore);
		if std::mem::replace(&mut self.gifs.favorites_restored, true) || self.gifs.favorites_edited
		{
			return;
		}
		if self.gifs.sync_restore_ambiguous {
			return;
		}
		if self.gifs.sync_loaded {
			let previous = std::mem::take(&mut self.gifs.favorites);
			for gif in favorites {
				if self.gifs.favorites.len() == MAX_GIF_FAVORITES {
					break;
				}
				if gif.valid()
					&& !removed.contains(&gif.url)
					&& (!self.gifs.sync_remote.contains(&gif.url)
						|| model::valid_gif_preview(&gif.preview))
					&& !self.is_gif_favorite(&gif)
				{
					self.gifs.favorites.push(gif);
				}
			}
			self.gifs.favorites_changed |= !self.gifs.favorites.is_empty();
			for gif in previous {
				if self.gifs.favorites.len() == MAX_GIF_FAVORITES {
					break;
				}
				if !self.is_gif_favorite(&gif) {
					self.gifs.favorites.push(gif);
				}
			}
			self.gifs.favorites.shrink_to_fit();
			return;
		}
		if self.gifs.favorites_changed || !self.gifs.favorites.is_empty() {
			return;
		}
		let mut restored: Vec<Gif> = Vec::with_capacity(favorites.len().min(MAX_GIF_FAVORITES));
		for gif in favorites {
			if gif.valid()
				&& !removed.contains(&gif.url)
				&& !restored.iter().any(|known| known.url == gif.url)
				&& restored.len() < MAX_GIF_FAVORITES
			{
				restored.push(gif);
			}
		}
		self.gifs.favorites = restored;
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};

	fn gif(id: &str) -> Gif {
		Gif {
			id: id.into(),
			title: "Synthetic".into(),
			url: format!("https://tenor.com/view/synthetic-gif-{id}"),
			preview: format!("https://media.tenor.com/{id}/tenor.png"),
			width: 300,
			height: 200,
		}
	}

	#[test]
	fn gif_requests_are_single_flight_and_results_match_the_live_request() {
		{
			let mut state = test_state();
			assert!(state.request_gifs(Some("   ")).is_none());
			let Some(Command::Gifs {
				query: Some(query),
				request,
			}) = state.request_gifs(Some(" wave "))
			else {
				panic!("search command")
			};
			assert_eq!(query, "wave");
			assert!(state.request_gifs(Some("wave")).is_none());
			state.apply_gifs(request.wrapping_sub(1), Ok(GifPage::default()));
			assert!(state.gifs.view.as_ref().unwrap().loading);
			state.apply(Envelope {
				generation: state.generation,
				event: Event::Gifs {
					request,
					result: Ok(GifPage {
						gifs: vec![gif("a")],
						categories: vec![],
					}),
				},
			});
			let view = state.gifs.view.as_ref().unwrap();
			assert!(!view.loading && view.error.is_none());
			assert_eq!(view.page.as_ref().unwrap().gifs.len(), 1);
			assert!(state.request_gifs(Some("wave")).is_none());
			assert!(state.clear_gifs().is_none());
			assert!(state.request_gifs(Some("wave")).is_none());
			let cached = state.gifs.view.as_ref().unwrap();
			assert!(!cached.loading && cached.page.is_some());
			assert!(matches!(
				state.request_gifs(None),
				Some(Command::Gifs { query: None, .. })
			));
			assert!(matches!(state.clear_gifs(), Some(Command::CancelGifs)));
			assert!(state.clear_gifs().is_none());
			let Some(Command::Gifs { request, .. }) = state.request_gifs(None) else {
				panic!()
			};
			state.apply_gifs(request, Err(Failure::RateLimited));
			assert!(state.gifs.view.as_ref().unwrap().error.is_some());
			assert!(state.request_gifs(None).is_some(), "errors allow a retry");
			let mut invalid = gif("bad");
			invalid.width = 0;
			let Some(Command::Gifs { request, .. }) = state.request_gifs(Some("x")) else {
				panic!()
			};
			state.apply_gifs(
				request,
				Ok(GifPage {
					gifs: vec![invalid],
					categories: vec![],
				}),
			);
			assert!(state.gifs.view.as_ref().unwrap().page.is_none());
			state.gateway_connected = false;
			assert!(state.request_gifs(None).is_none());
		}
		{
			let mut state = test_state();
			for index in 0..(GIF_CACHE_PAGES + 3) {
				let query = format!("query{index}");
				let Some(Command::Gifs { request, .. }) = state.request_gifs(Some(&query)) else {
					panic!("uncached query");
				};
				state.apply_gifs(
					request,
					Ok(GifPage {
						gifs: vec![gif(&format!("g{index}"))],
						categories: vec![],
					}),
				);
				state.clear_gifs();
			}
			assert_eq!(state.gifs.cache.len(), GIF_CACHE_PAGES);
			assert!(state.gifs.cache_bytes <= GIF_CACHE_BYTES);
			assert!(matches!(
				state.request_gifs(Some("query0")),
				Some(Command::Gifs { .. })
			));
			state.logout();
			assert!(state.gifs.cache.is_empty());
			assert_eq!(state.gifs.cache_bytes, 0);
		}
	}

	#[test]
	fn favorites_are_bounded_deduplicated_and_restored_only_when_untouched() {
		let mut state = test_state();
		state.restore_gif_favorites(vec![gif("saved"), gif("saved"), {
			let mut bad = gif("bad");
			bad.url = "http://tenor.com/view/x".into();
			bad
		}]);
		assert_eq!(state.gifs.favorites.len(), 1);
		assert!(!state.gifs.favorites_changed);
		state.restore_gif_favorites(vec![gif("other")]);
		assert_eq!(state.gifs.favorites[0].id, "saved");
		assert!(state.toggle_gif_favorite(&gif("saved")));
		assert!(state.gifs.favorites.is_empty() && state.gifs.favorites_changed);
		for i in 0..(MAX_GIF_FAVORITES + 5) {
			assert!(state.toggle_gif_favorite(&gif(&format!("f{i}"))));
		}
		assert_eq!(state.gifs.favorites.len(), MAX_GIF_FAVORITES);
		assert_eq!(
			state.gifs.favorites[0].id,
			format!("f{}", MAX_GIF_FAVORITES + 4)
		);
		assert!(state.is_gif_favorite(&gif("f10")));
		let mut invalid = gif("nope");
		invalid.preview = "https://untrusted.example/x/tenor.mp4".into();
		assert!(!state.toggle_gif_favorite(&invalid));
		state.restore_gif_favorites(vec![gif("late")]);
		assert!(!state.is_gif_favorite(&gif("late")));
	}

	fn test_state() -> State {
		State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		}
	}
	#[test]
	fn gif_favorites_session_reset_releases_reads_and_writes_and_rejects_old_results() {
		for queued_write in [false, true] {
			for expire in [false, true] {
				let mut state = test_state();
				state.restore_gif_favorites(vec![gif("local")]);
				let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites()
				else {
					panic!("load");
				};
				if queued_write {
					state.apply_gif_favorites(request, Ok(vec![gif("remote")]));
					assert!(state.toggle_gif_favorite(&gif("added")));
					assert!(state.gifs.sync_ready && state.gifs.sync_command.is_some());
				}
				let old_request = state.gifs.sync_pending.unwrap();
				if expire {
					state.apply(Envelope {
						generation: state.generation,
						event: Event::Failure(Failure::Expired),
					});
					assert!(state.gifs.sync_pending.is_none() && !state.gifs.sync_ready);
					assert!(state.take_gif_favorites_command().is_none());
				}
				state.apply(Envelope {
					generation: state.generation,
					event: Event::Ready {
						permissions: model::permissions::Snapshot::default(),
						user: model::User {
							id: model::Id(1),
							name: "Synthetic".into(),
							avatar: None,
							webhook: false,
							kind: Default::default(),
							discriminator: 0,
							primary_guild: None,
						},
						guilds: vec![],
						channels: vec![],
					},
				});
				assert!(!state.gifs.sync_attempted && !state.gifs.sync_ready);
				assert!(state.is_gif_favorite(&gif("local")));
				assert!(state.take_gif_favorites_command().is_none());
				let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites()
				else {
					panic!("new session read");
				};
				assert_ne!(request, old_request);
				state.apply_gif_favorites(old_request, Ok(vec![gif("stale")]));
				assert_eq!(state.gifs.sync_pending, Some(request));
				assert!(!state.is_gif_favorite(&gif("stale")));
			}
		}
		let mut state = test_state();
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("load")
		};
		state.apply_gif_favorites(request, Ok(vec![gif("remote")]));
		assert!(state.toggle_gif_favorite(&gif("added")));
		let request = state.gifs.sync_pending.unwrap();
		state.apply_gif_favorites(request, Err(Failure::Expired));
		assert!(!state.gifs.sync_ready && state.gifs.sync_pending.is_none());
	}
	#[test]
	fn gif_favorites_host_shutdown_unblocks_offline_local_stars_without_replaying_writes() {
		let mut state = test_state();
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("load")
		};
		state.apply_gif_favorites(request, Ok(vec![gif("remote")]));
		assert!(state.toggle_gif_favorite(&gif("added")));
		state.interrupt_gif_favorites();
		state.gateway_connected = false;
		assert!(state.toggle_gif_favorite(&gif("offline")));
		assert!(state.take_gif_favorites_command().is_none());
		assert!(!state.gifs.sync_ready && state.gifs.sync_pending.is_none());
		assert!(state.is_gif_favorite(&gif("added")));
	}
	#[test]
	fn full_remote_projection_reserves_local_fallback_in_both_cache_arrival_orders() {
		for cache_first in [false, true] {
			let mut state = test_state();
			let locals: Vec<_> = (0..MAX_GIF_FAVORITES)
				.map(|i| gif(&format!("local-{i}")))
				.collect();
			if cache_first {
				state.restore_gif_favorites(locals.clone());
			}
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("load")
			};
			let remotes: Vec<_> = (0..MAX_GIF_FAVORITES)
				.map(|i| gif(&format!("remote-{i}")))
				.collect();
			state.apply_gif_favorites(request, Ok(remotes.clone()));
			if !cache_first {
				state.restore_gif_favorites(locals.clone());
			}
			assert_eq!(state.gifs.favorites, locals);
			assert_eq!(state.gifs.sync_remote.len(), MAX_GIF_FAVORITES);
			assert!(state.gifs.favorites_changed);
			let added = gif("added");
			assert!(state.toggle_gif_favorite(&added));
			let Some(Command::GifFavorites {
				request,
				change: Some(_),
			}) = state.take_gif_favorites_command()
			else {
				panic!("explicit add")
			};
			let mut saved = remotes;
			saved.insert(0, added.clone());
			saved.truncate(MAX_GIF_FAVORITES);
			state.apply_gif_favorites(request, Ok(saved.clone()));
			assert_eq!(state.gifs.favorites[0], added);
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("refresh")
			};
			state.apply_gif_favorites(request, Ok(saved));
			assert!(state.is_gif_favorite(&added));
			assert_eq!(state.gifs.favorites.len(), MAX_GIF_FAVORITES);
			assert!(state.gifs.bytes() < 192 * 1024);
		}
	}
	#[test]
	fn late_cache_restoration_keeps_remote_favorites_and_cannot_undo_explicit_stars() {
		let local = gif("local");
		let remote = gif("remote");
		let mut state = test_state();
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("load");
		};
		state.apply_gif_favorites(request, Ok(vec![remote.clone()]));
		std::mem::take(&mut state.gifs.favorites_changed);
		state.restore_gif_favorites(vec![local.clone(), remote.clone()]);
		assert!(state.is_gif_favorite(&local) && state.is_gif_favorite(&remote));
		assert_eq!(state.gifs.favorites.len(), 2);
		assert!(state.gifs.favorites_changed);
		assert!(state.take_gif_favorites_command().is_none());

		let mut state = test_state();
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("load");
		};
		state.apply_gif_favorites(request, Ok(vec![remote.clone()]));
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("failed refresh with retained remote");
		};
		state.apply_gif_favorites(request, Err(Failure::Network));
		state.restore_gif_favorites(vec![local.clone()]);
		assert!(state.is_gif_favorite(&local) && state.is_gif_favorite(&remote));
		assert!(
			!state.gifs.sync_ready,
			"late fallback never reenables server writes"
		);

		for fail_later_refresh in [false, true] {
			let mut state = test_state();
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("load");
			};
			state.apply_gif_favorites(request, Ok(vec![remote.clone()]));
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("refresh before initial cache");
			};
			state.apply_gif_favorites(request, Ok(vec![]));
			if fail_later_refresh {
				let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites()
				else {
					panic!("failed refresh");
				};
				state.apply_gif_favorites(request, Err(Failure::Network));
				std::mem::take(&mut state.gifs.favorites_changed);
			}
			state.restore_gif_favorites(vec![remote.clone(), local.clone()]);
			assert!(
				!state.is_gif_favorite(&remote),
				"late cache cannot undo an observed remote removal"
			);
			assert!(state.is_gif_favorite(&local));
		}

		let mut state = test_state();
		assert!(state.toggle_gif_favorite(&local));
		assert!(state.toggle_gif_favorite(&local));
		std::mem::take(&mut state.gifs.favorites_changed);
		state.restore_gif_favorites(vec![local]);
		assert!(
			state.gifs.favorites.is_empty(),
			"late cache cannot re-add a removed star"
		);
	}
	#[test]
	fn local_still_previews_survive_repeated_syncs_until_removed_remotely() {
		let mut state = test_state();
		state.restore_gif_favorites(vec![gif("still")]);
		let mut clip = gif("still");
		clip.preview = "https://media.tenor.com/still/tenor.mp4".into();
		for remote in [
			vec![clip.clone()],
			vec![gif("other"), clip.clone()],
			vec![gif("other")],
		] {
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("sync");
			};
			state.apply_gif_favorites(request, Ok(remote));
		}
		assert_eq!(state.gifs.favorites, vec![gif("other")]);
		let mut state = test_state();
		state.restore_gif_favorites(vec![gif("still")]);
		for remote in [vec![clip.clone()], vec![gif("other"), clip]] {
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("sync");
			};
			state.apply_gif_favorites(request, Ok(remote));
		}
		assert_eq!(state.gifs.favorites, vec![gif("still"), gif("other")]);
	}
	#[test]
	fn pre_cache_removal_history_is_bounded_and_overflow_cannot_restore_stale_records() {
		let mut state = test_state();
		for batch in 0..3 {
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("load");
			};
			state.apply_gif_favorites(
				request,
				Ok((0..50).map(|i| gif(&format!("batch{batch}-{i}"))).collect()),
			);
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("remove batch");
			};
			state.apply_gif_favorites(request, Ok(vec![]));
		}
		assert_eq!(
			state.gifs.sync_removed_before_restore.len(),
			MAX_GIF_FAVORITES
		);
		assert!(state.gifs.sync_restore_ambiguous);
		assert!(state.gifs.bytes() < 64 * 1024);
		state.restore_gif_favorites(vec![gif("batch2-0"), gif("local")]);
		assert!(state.gifs.favorites.is_empty());
		assert!(state.gifs.sync_removed_before_restore.is_empty());
	}
	#[test]
	fn synchronized_favorites_keep_local_fallback_drop_remote_removals_and_match_live_requests() {
		let mut state = test_state();
		let local = gif("local");
		let remote = gif("remote");
		let added = gif("added");
		state.restore_gif_favorites(vec![local.clone()]);
		let Some(Command::GifFavorites {
			request,
			change: None,
		}) = state.request_gif_favorites()
		else {
			panic!("load");
		};
		assert!(state.request_gif_favorites().is_none());
		assert!(!state.toggle_gif_favorite(&added));
		state.apply_gif_favorites(request.wrapping_sub(1), Err(Failure::Expired));
		assert_eq!(state.auth, AuthState::Authenticated);
		assert_eq!(state.gifs.sync_pending, Some(request));
		state.apply_gif_favorites(request, Ok(vec![remote.clone()]));
		assert!(state.is_gif_favorite(&local) && state.is_gif_favorite(&remote));
		assert!(
			state.take_gif_favorites_command().is_none(),
			"local fallback is never uploaded implicitly"
		);
		let mut same_url = remote.clone();
		same_url.id = "different-provider-id".into();
		assert!(state.is_gif_favorite(&same_url));
		assert!(state.toggle_gif_favorite(&added));
		let Some(Command::GifFavorites {
			request,
			change: Some((target, true)),
		}) = state.take_gif_favorites_command()
		else {
			panic!("explicit star");
		};
		assert_eq!(target.url, added.url);
		assert!(state.take_gif_favorites_command().is_none());
		assert!(!state.toggle_gif_favorite(&remote));
		state.apply_gif_favorites(request, Ok(vec![added.clone(), remote.clone()]));
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("refresh");
		};
		state.apply_gif_favorites(request, Ok(vec![added]));
		assert!(!state.is_gif_favorite(&remote));
		assert!(state.is_gif_favorite(&local));
		assert!(state.gifs.bytes() <= 256 * 1024);
	}
	#[test]
	fn failed_favorite_sync_retains_local_state_and_requires_explicit_refresh_before_more_writes() {
		let mut state = test_state();
		let local = gif("local");
		state.restore_gif_favorites(vec![local.clone()]);
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("load");
		};
		state.apply_gif_favorites(request, Ok(vec![]));
		assert!(state.toggle_gif_favorite(&gif("new")));
		let command = state.take_gif_favorites_command().unwrap();
		state.command_rejected(command);
		assert!(state.is_gif_favorite(&local) && state.is_gif_favorite(&gif("new")));
		assert_eq!(state.auth, AuthState::Authenticated);
		assert!(!state.gifs.sync_ready);
		assert!(state.gifs.sync_error.is_some() && state.gifs.sync_pending.is_none());
		assert!(state.toggle_gif_favorite(&gif("offline")));
		assert!(state.take_gif_favorites_command().is_none());
		assert!(state.gifs.sync_attempted, "no automatic read/write retry");
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("explicit refresh");
		};
		state.apply_gif_favorites(
			request,
			Ok((0..=MAX_GIF_FAVORITES)
				.map(|i| gif(&i.to_string()))
				.collect()),
		);
		assert!(!state.gifs.sync_ready);
		assert_eq!(state.auth, AuthState::Authenticated);
		state.logout();
		assert!(state.gifs.favorites.is_empty() && state.gifs.sync_remote.is_empty());
	}
}
