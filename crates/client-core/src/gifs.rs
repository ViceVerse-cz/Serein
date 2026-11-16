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
	sync_dirty: bool,
	sync_writing: bool,
	sync_loaded: bool,
	sync_request: u64,
	sync_command: Option<(Gif, bool)>,
	sync_remote: Vec<String>,
	// Until the initial cache arrives, remember removals instead of reintroducing them.
	sync_removed_before_restore: Vec<String>,
	sync_restore_ambiguous: bool,
	cache: VecDeque<CachedPage>,
	cache_bytes: usize,
	hover_sent: VecDeque<Instant>,
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
			+ self.hover_sent.capacity() * size_of::<Instant>()
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
	/// Closed-picker hover reads share the 12/minute bound, including retries after failure.
	pub fn preload_gifs(&mut self, now: Instant) -> Option<Command> {
		if self.demo || !self.can_browse_gifs() || self.gifs.view.is_some() {
			return None;
		}
		while self
			.gifs
			.hover_sent
			.front()
			.is_some_and(|at| now.saturating_duration_since(*at) >= Duration::from_secs(60))
		{
			self.gifs.hover_sent.pop_front();
		}
		let cached = self
			.gifs
			.cache
			.iter()
			.any(|entry| entry.query.is_none() && entry.loaded.elapsed() < GIF_CACHE_TTL);
		if !cached && self.gifs.hover_sent.len() >= crate::preload::PER_MINUTE {
			return None;
		}
		let command = self.request_gifs(None);
		if command.is_some() {
			self.gifs.hover_sent.push_back(now);
		}
		command
	}
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
	/// False when bounded pre-restore removals were insufficient to reconcile safely.
	pub fn gif_favorites_cache_safe(&self) -> bool {
		self.gifs.favorites_restored && !self.gifs.sync_restore_ambiguous
	}
	/// Call before the host replaces or shuts down its account connection.
	/// Retain local metadata, but never replay an interrupted server write.
	pub fn interrupt_gif_favorites(&mut self) {
		self.gifs.sync_pending = None;
		self.gifs.sync_command = None;
		self.gifs.sync_dirty = false;
		self.gifs.sync_writing = false;
		self.gifs.sync_ready = false;
		self.gifs.sync_attempted = false;
		self.gifs.sync_error = None;
	}
	/// Coalesce remote changes until the picker next reads, including changes during a request.
	pub(crate) fn invalidate_gif_favorites(&mut self) {
		// Explicit stars read fresh server settings, so invalidation need not disable writes.
		self.gifs.sync_dirty = true;
	}
	/// Automatic picker reads consume one invalidation; failures require Refresh or a new update.
	pub fn request_gif_favorites_if_needed(&mut self) -> Option<Command> {
		if self.gifs.sync_attempted && !self.gifs.sync_dirty {
			return None;
		}
		self.request_gif_favorites()
	}
	pub fn request_gif_favorites(&mut self) -> Option<Command> {
		if !self.can_browse_gifs() || self.gifs.sync_pending.is_some() {
			return None;
		}
		self.gifs.sync_attempted = true;
		self.gifs.sync_dirty = false;
		self.gifs.sync_writing = false;
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
		let writing = std::mem::take(&mut self.gifs.sync_writing);
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
				// A read started before a remote change cannot replace the displayed or disk list.
				// A confirmed write still reconciles its explicit star before the subsequent read.
				if self.gifs.sync_dirty && !writing {
					return;
				}
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
				self.gifs.favorites = merge_favorites(previous, favorites, &previous_remote);
				self.gifs.favorites.shrink_to_fit();
				self.gifs.sync_ready = true;
				self.gifs.sync_loaded = true;
				self.gifs.sync_error = None;
				self.gifs.favorites_changed = true;
			}
			Err(failure) if failure.ends_session() && failure != Failure::Capacity => {
				self.fail(failure)
			}
			Err(Failure::ProtocolAt(
				context @ ("gif-favorites-sync-unsupported" | "gif-favorites-sync-unconfirmed"),
			)) => {
				self.gifs.sync_dirty = false;
				self.gifs.sync_ready = false;
				self.gifs.sync_error = Some(context);
			}
			_ => {
				self.gifs.sync_dirty = false;
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
		if !self.gifs.favorites_restored {
			if favorite {
				self.gifs
					.sync_removed_before_restore
					.retain(|url| *url != gif.url);
			} else if !self.gifs.sync_removed_before_restore.contains(&gif.url) {
				if self.gifs.sync_removed_before_restore.len() == MAX_GIF_FAVORITES {
					self.gifs.sync_restore_ambiguous = true;
				} else {
					self.gifs.sync_removed_before_restore.push(gif.url.clone());
				}
			}
		}
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
			self.gifs.sync_writing = true;
			self.gifs.sync_command = Some((gif.clone(), favorite));
			self.gifs.sync_error = None;
		}
		true
	}
	/// A late initial cache read merges with synchronized records, but cannot undo a user's star.
	pub fn restore_gif_favorites(&mut self, favorites: Vec<Gif>) {
		let removed = std::mem::take(&mut self.gifs.sync_removed_before_restore);
		if std::mem::replace(&mut self.gifs.favorites_restored, true) {
			return;
		}
		if self.gifs.sync_restore_ambiguous {
			return;
		}
		if self.gifs.sync_loaded {
			let mut restored = Vec::<Gif>::new();
			for gif in favorites {
				if restored.len() < MAX_GIF_FAVORITES
					&& gif.valid() && !removed.contains(&gif.url)
					&& !restored.iter().any(|known| known.url == gif.url)
				{
					restored.push(gif);
				}
			}
			self.gifs.favorites_changed |= !restored.is_empty();
			let current = std::mem::take(&mut self.gifs.favorites);
			self.gifs.favorites = if self.gifs.favorites_edited {
				merge_early_edits(current, restored)
			} else {
				merge_favorites(restored, current, &self.gifs.sync_remote)
			};
			self.gifs.favorites.shrink_to_fit();
			return;
		}
		if !self.gifs.favorites_edited
			&& (self.gifs.favorites_changed || !self.gifs.favorites.is_empty())
		{
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
		if self.gifs.favorites_edited {
			let current = std::mem::take(&mut self.gifs.favorites);
			self.gifs.favorites = merge_early_edits(current, restored);
			self.gifs.favorites_changed = true;
		} else {
			self.gifs.favorites = restored;
		}
	}
}

/// Explicit early choices take priority over older disk records at the shared capacity.
fn merge_early_edits(mut current: Vec<Gif>, restored: Vec<Gif>) -> Vec<Gif> {
	for gif in restored {
		if current.len() == MAX_GIF_FAVORITES {
			break;
		}
		if !current.iter().any(|known| known.url == gif.url) {
			current.push(gif);
		}
	}
	current
}

/// Reserve room for unsynchronized local favorites, but keep shared records in server order.
/// Better local still previews replace metadata at the same position, never the saved ordering.
fn merge_favorites(local: Vec<Gif>, remote: Vec<Gif>, previous_remote: &[String]) -> Vec<Gif> {
	let local_only = |gif: &&Gif| {
		!previous_remote.contains(&gif.url) && !remote.iter().any(|other| other.url == gif.url)
	};
	let reserved = local.iter().filter(local_only).count();
	let mut merged = Vec::with_capacity(MAX_GIF_FAVORITES);
	for gif in remote.iter().take(MAX_GIF_FAVORITES - reserved) {
		let still = local.iter().find(|known| {
			known.url == gif.url
				&& model::valid_gif_preview(&known.preview)
				&& !model::valid_gif_preview(&gif.preview)
		});
		merged.push(still.unwrap_or(gif).clone());
	}
	merged.extend(local.iter().filter(local_only).cloned());
	merged
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{Envelope, Event};

	#[test]
	fn late_gif_cache_merges_early_stars_without_reviving_explicit_removals() {
		let mut state = test_state();
		let old = gif("old");
		let added = gif("added");
		assert!(state.toggle_gif_favorite(&added));
		state.restore_gif_favorites(vec![old.clone()]);
		assert_eq!(state.gifs.favorites, vec![added, old]);
		assert!(state.gifs.favorites_changed && state.gif_favorites_cache_safe());

		let mut state = test_state();
		let removed = gif("removed");
		let keep = gif("keep");
		assert!(state.toggle_gif_favorite(&removed));
		assert!(state.toggle_gif_favorite(&removed));
		state.restore_gif_favorites(vec![removed.clone(), keep.clone()]);
		assert!(!state.is_gif_favorite(&removed));
		assert!(state.is_gif_favorite(&keep));

		let mut state = test_state();
		let added = gif("newest");
		assert!(state.toggle_gif_favorite(&added));
		state.restore_gif_favorites(
			(0..MAX_GIF_FAVORITES)
				.map(|i| gif(&format!("old{i}")))
				.collect(),
		);
		assert_eq!(state.gifs.favorites.len(), MAX_GIF_FAVORITES);
		assert_eq!(state.gifs.favorites[0], added);
	}

	#[test]
	fn synchronized_order_survives_cache_arrival_preview_replacement_and_refresh() {
		for cache_first in [false, true] {
			let mut state = test_state();
			let a = gif("a");
			let b = gif("b");
			let local = gif("offline");
			let mut clip = a.clone();
			clip.preview = "https://media.tenor.com/a/tenor.mp4".into();
			let cache = vec![a.clone(), local.clone(), b.clone()];
			if cache_first {
				state.restore_gif_favorites(cache.clone());
			}
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!()
			};
			state.apply_gif_favorites(request, Ok(vec![b.clone(), clip.clone()]));
			if !cache_first {
				state.restore_gif_favorites(cache);
			}
			assert_eq!(
				state.gifs.favorites,
				vec![b.clone(), a.clone(), local.clone()]
			);
			// A still preview must not pin the old position when another client reorders favorites.
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!()
			};
			state.apply_gif_favorites(request, Ok(vec![clip, b.clone()]));
			assert_eq!(state.gifs.favorites, vec![a.clone(), b, local.clone()]);
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!()
			};
			state.apply_gif_favorites(request, Ok(vec![]));
			assert_eq!(
				state.gifs.favorites,
				vec![local],
				"removals retire synchronized stills too"
			);
		}
	}

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
	fn hover_retries_are_capped_but_cache_hits_and_explicit_clicks_still_work() {
		let now = Instant::now();
		let mut state = test_state();
		for _ in 0..crate::preload::PER_MINUTE {
			let Some(Command::Gifs { request, .. }) = state.preload_gifs(now) else {
				panic!()
			};
			state.apply_gifs(request, Err(Failure::Network));
			state.clear_gifs();
		}
		assert!(state.preload_gifs(now).is_none());
		assert!(state.gifs.view.is_none());
		let Some(Command::Gifs { request, .. }) = state.request_gifs(None) else {
			panic!("click is not speculative")
		};
		state.apply_gifs(request, Ok(GifPage::default()));
		state.clear_gifs();
		assert!(
			state.preload_gifs(now).is_none(),
			"cache hit needs no request"
		);
		assert!(state.gifs.view.as_ref().is_some_and(|view| !view.loading));
		assert_eq!(state.gifs.hover_sent.len(), crate::preload::PER_MINUTE);
		state.clear_gifs();
		state.gifs.cache.clear();
		state.gifs.cache_bytes = 0;
		assert!(state.preload_gifs(now + Duration::from_secs(60)).is_some());
		assert_eq!(state.gifs.hover_sent.len(), 1);
		state.clear_gifs();
		state.demo = true;
		assert!(state.preload_gifs(now + Duration::from_secs(61)).is_none());
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
	fn favorites_are_bounded_deduplicated_and_restored_only_once() {
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
		assert!(
			!state.is_gif_favorite(&gif("late")),
			"a second cache result cannot replace later user edits"
		);
	}

	fn test_state() -> State {
		State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		}
	}

	fn notify_favorites(state: &mut State) {
		state.apply(Envelope {
			generation: state.generation,
			event: Event::GifFavoritesChanged,
		});
	}

	fn next_favorites_read(state: &mut State) -> u64 {
		let Some(Command::GifFavorites {
			request,
			change: None,
		}) = state.request_gif_favorites_if_needed()
		else {
			panic!("favorites read");
		};
		request
	}

	#[test]
	fn remote_favorite_notifications_refresh_once_and_preserve_local_definitions() {
		let mut state = test_state();
		state.restore_gif_favorites(vec![gif("local")]);
		let request = next_favorites_read(&mut state);
		state.apply_gif_favorites(request, Ok(vec![gif("old")]));
		std::mem::take(&mut state.gifs.favorites_changed);
		assert!(state.request_gif_favorites_if_needed().is_none());
		for _ in 0..10 {
			notify_favorites(&mut state);
		}
		assert!(state.gifs.sync_ready);
		assert!(!state.gifs.favorites_changed);
		assert!(state.gif_favorites_cache_safe());
		assert_eq!(state.gifs.favorites, vec![gif("old"), gif("local")]);
		let request = next_favorites_read(&mut state);
		assert!(state.request_gif_favorites_if_needed().is_none());
		state.apply_gif_favorites(request, Ok(vec![gif("new")]));
		assert_eq!(state.gifs.favorites, vec![gif("new"), gif("local")]);
		assert!(state.gifs.sync_ready && state.gifs.favorites_changed);
		assert!(state.request_gif_favorites_if_needed().is_none());
		assert!(state.take_gif_favorites_command().is_none());
	}

	#[test]
	fn notification_during_favorite_read_rejects_stale_projection_and_coalesces_followup() {
		let mut state = test_state();
		state.restore_gif_favorites(vec![gif("local")]);
		let request = next_favorites_read(&mut state);
		state.apply_gif_favorites(request, Ok(vec![gif("old")]));
		std::mem::take(&mut state.gifs.favorites_changed);
		let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
			panic!("in-flight refresh");
		};
		for _ in 0..10 {
			notify_favorites(&mut state);
			assert!(state.request_gif_favorites_if_needed().is_none());
		}
		state.apply_gif_favorites(request, Ok(vec![gif("stale")]));
		assert_eq!(state.gifs.favorites, vec![gif("old"), gif("local")]);
		assert!(!state.gifs.favorites_changed && state.gifs.sync_ready);
		let fresh = next_favorites_read(&mut state);
		assert_ne!(fresh, request);
		state.apply_gif_favorites(request, Err(Failure::Expired));
		assert_eq!(state.auth, AuthState::Authenticated);
		assert_eq!(state.gifs.sync_pending, Some(fresh));
		state.apply_gif_favorites(fresh, Ok(vec![gif("new")]));
		assert_eq!(state.gifs.favorites, vec![gif("new"), gif("local")]);
		assert!(state.request_gif_favorites_if_needed().is_none());
	}

	#[test]
	fn notifications_wait_for_pending_favorite_adds_and_removals_without_replaying_writes() {
		for adding in [false, true] {
			for dispatched in [false, true] {
				let mut state = test_state();
				state.restore_gif_favorites(vec![gif("local")]);
				let request = next_favorites_read(&mut state);
				state.apply_gif_favorites(request, Ok(vec![gif("remote")]));
				let target = if adding { gif("added") } else { gif("remote") };
				assert!(state.toggle_gif_favorite(&target));
				let command = dispatched.then(|| state.take_gif_favorites_command().unwrap());
				for _ in 0..10 {
					notify_favorites(&mut state);
					assert!(state.request_gif_favorites_if_needed().is_none());
				}
				assert_eq!(state.is_gif_favorite(&target), adding);
				let command = command
					.or_else(|| state.take_gif_favorites_command())
					.unwrap();
				let Command::GifFavorites {
					request,
					change: Some((changed, favorite)),
				} = command
				else {
					panic!("explicit star change");
				};
				assert_eq!(changed, target);
				assert_eq!(favorite, adding);
				assert!(state.take_gif_favorites_command().is_none());
				state.apply_gif_favorites(
					request,
					Ok(if adding {
						vec![target.clone(), gif("remote")]
					} else {
						vec![]
					}),
				);
				assert_eq!(state.is_gif_favorite(&target), adding);
				assert!(state.gifs.sync_ready);
				let request = next_favorites_read(&mut state);
				state.apply_gif_favorites(request, Ok(vec![gif("other")]));
				assert_eq!(state.gifs.favorites, vec![gif("other"), gif("local")]);
				assert!(state.gifs.sync_ready);
				assert!(state.request_gif_favorites_if_needed().is_none());
				assert!(state.take_gif_favorites_command().is_none());
			}
		}
	}

	#[test]
	fn stars_between_remote_invalidation_and_refresh_still_send_the_explicit_change() {
		for adding in [false, true] {
			let mut state = test_state();
			let request = next_favorites_read(&mut state);
			state.apply_gif_favorites(request, Ok(vec![gif("remote")]));
			notify_favorites(&mut state);
			let target = if adding { gif("added") } else { gif("remote") };
			assert!(state.toggle_gif_favorite(&target));
			assert!(state.request_gif_favorites_if_needed().is_none());
			let Some(Command::GifFavorites {
				request,
				change: Some((changed, favorite)),
			}) = state.take_gif_favorites_command()
			else {
				panic!("notification cannot turn an explicit star into an unsent local edit");
			};
			assert_eq!(changed, target);
			assert_eq!(favorite, adding);
			state.apply_gif_favorites(
				request,
				Ok(if adding {
					vec![target.clone(), gif("remote")]
				} else {
					vec![]
				}),
			);
			let request = next_favorites_read(&mut state);
			state.apply_gif_favorites(
				request,
				Ok(if adding {
					vec![target.clone(), gif("remote")]
				} else {
					vec![]
				}),
			);
			assert_eq!(state.is_gif_favorite(&target), adding);
			assert!(state.take_gif_favorites_command().is_none());
		}
	}

	#[test]
	fn failed_invalidated_favorite_requests_wait_for_retry_or_a_new_notification() {
		for failure in [
			Failure::Network,
			Failure::ProtocolAt("gif-favorites-sync-unconfirmed"),
		] {
			let mut state = test_state();
			state.restore_gif_favorites(vec![gif("local")]);
			let request = next_favorites_read(&mut state);
			state.apply_gif_favorites(request, Ok(vec![]));
			assert!(state.toggle_gif_favorite(&gif("pending")));
			let Some(Command::GifFavorites {
				request,
				change: Some(_),
			}) = state.take_gif_favorites_command()
			else {
				panic!("pending write");
			};
			notify_favorites(&mut state);
			state.apply_gif_favorites(request, Err(failure));
			for _ in 0..10 {
				assert!(state.request_gif_favorites_if_needed().is_none());
				assert!(state.take_gif_favorites_command().is_none());
			}
			assert!(state.is_gif_favorite(&gif("pending")));
			notify_favorites(&mut state);
			let request = next_favorites_read(&mut state);
			notify_favorites(&mut state);
			state.apply_gif_favorites(request, Err(Failure::Network));
			assert!(state.request_gif_favorites_if_needed().is_none());
			let Some(Command::GifFavorites {
				request,
				change: None,
			}) = state.request_gif_favorites()
			else {
				panic!("explicit retry");
			};
			state.apply_gif_favorites(request, Ok(vec![]));
			assert!(state.gifs.sync_ready && state.is_gif_favorite(&gif("local")));
			assert!(state.take_gif_favorites_command().is_none());
		}
	}

	#[test]
	fn favorite_invalidation_clears_on_interruption_and_old_account_events_are_fenced() {
		let mut state = test_state();
		let request = next_favorites_read(&mut state);
		notify_favorites(&mut state);
		state.interrupt_gif_favorites();
		assert!(!state.gifs.sync_dirty && !state.gifs.sync_writing);
		state.apply_gif_favorites(request, Ok(vec![gif("stale")]));
		assert!(state.gifs.favorites.is_empty());
		let request = next_favorites_read(&mut state);
		notify_favorites(&mut state);
		let old_generation = state.generation;
		state.logout();
		assert!(!state.gifs.sync_dirty && state.gifs.sync_pending.is_none());
		for event in [
			Event::GifFavoritesChanged,
			Event::GifFavorites {
				request,
				result: Ok(vec![gif("stale")]),
			},
		] {
			state.apply(Envelope {
				generation: old_generation,
				event,
			});
		}
		assert!(!state.gifs.sync_dirty && state.gifs.favorites.is_empty());
		assert!(state.request_gif_favorites_if_needed().is_none());
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
							member_avatar: None,
							username: None,
							id: model::Id(1),
							name: "Synthetic".into(),
							avatar: None,
							webhook: false,
							kind: Default::default(),
							discriminator: 0,
							primary_guild: None,
							cosmetics: None,
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
		assert_eq!(state.gifs.favorites, vec![gif("other"), gif("still")]);
		// A synchronized image favorite keeps following the server's entry.
		let mut state = test_state();
		let mut updated = gif("image");
		updated.width = 480;
		for remote in [vec![gif("image")], vec![updated.clone()]] {
			let Some(Command::GifFavorites { request, .. }) = state.request_gif_favorites() else {
				panic!("sync");
			};
			state.apply_gif_favorites(request, Ok(remote));
		}
		assert_eq!(state.gifs.favorites, vec![updated]);
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
