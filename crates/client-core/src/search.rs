use crate::{
	Command, State,
	auth::{AuthState, Failure},
};
use model::{Freshness, Id, SearchPage};

pub struct SearchView {
	pub pins: bool,
	pub channel: Id,
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
		if view.pins
			|| view.loading
			|| Some(view.channel) != self.selected
			|| page >= view.page_count()
		{
			return None;
		}
		let offset = page.checked_mul(model::SEARCH_PAGE_SIZE as u32)?;
		if offset > model::MAX_SEARCH_OFFSET {
			return None;
		}
		let guild = self.channel(view.channel)?.guild;
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
		if !self.can_search() {
			return;
		}
		let Some(view) = self.search.as_mut().filter(|s| {
			s.channel == channel
				&& s.request == request
				&& s.loading && Some(channel) == self.selected
		}) else {
			return;
		};
		view.loading = false;
		match result {
			Ok(Outcome::Page(page)) if !view.pins && page.valid(channel, view.before) => {
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
	pub fn open_search_hit(&mut self, message: Id) -> Option<Command> {
		if !self.can_search()
			|| !self
				.search
				.as_ref()?
				.page
				.as_ref()?
				.hits
				.iter()
				.any(|h| h.id == message && Some(h.channel) == self.selected)
		{
			return None;
		}
		self.open_target_window(message)
	}
}
