//! Tenor GIF browsing through the service relay plus a bounded local favorites list.
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
	/// Set when favorites changed locally; the host persists them and clears it.
	pub favorites_changed: bool,
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
		self.gifs.favorites.iter().any(|known| known.id == gif.id)
	}
	/// Newest favorite first; the list is bounded and never holds rejected entries.
	pub fn toggle_gif_favorite(&mut self, gif: &Gif) -> bool {
		if let Some(index) = self
			.gifs
			.favorites
			.iter()
			.position(|known| known.id == gif.id)
		{
			self.gifs.favorites.remove(index);
		} else if gif.valid() {
			self.gifs.favorites.insert(0, gif.clone());
			self.gifs.favorites.truncate(MAX_GIF_FAVORITES);
		} else {
			return false;
		}
		self.gifs.favorites_changed = true;
		true
	}
	/// Saved favorites replace the in-memory list unless the user already changed it.
	pub fn restore_gif_favorites(&mut self, favorites: Vec<Gif>) {
		if self.gifs.favorites_changed || !self.gifs.favorites.is_empty() {
			return;
		}
		let mut restored: Vec<Gif> = Vec::with_capacity(favorites.len().min(MAX_GIF_FAVORITES));
		for gif in favorites {
			if gif.valid()
				&& !restored.iter().any(|known| known.id == gif.id)
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
		invalid.preview = "https://media.tenor.com/x/tenor.mp4".into();
		assert!(!state.toggle_gif_favorite(&invalid));
		state.restore_gif_favorites(vec![gif("late")]);
		assert!(!state.is_gif_favorite(&gif("late")));
	}

	#[test]
	fn gif_result_cache_is_bounded_and_cleared_with_the_session() {
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

	fn test_state() -> State {
		State {
			auth: AuthState::Authenticated,
			gateway_connected: true,
			..State::default()
		}
	}
}
