use crate::Id;

pub const SEARCH_PAGE_SIZE: usize = 25;
pub const MAX_SEARCH_BYTES: usize = 256 * 1024;
pub fn valid_search_query(query: &str) -> bool {
	!query.trim().is_empty()
		&& query.len() <= 1024
		&& query.chars().count() <= 256
		&& !query.chars().any(char::is_control)
}
/// Bounded query tokens shared by the UI, offline demo and HTTP adapter.
pub type SearchTerms = (String, Vec<(&'static str, String)>);

pub fn search_terms(query: &str) -> Result<SearchTerms, &'static str> {
	if !valid_search_query(query) {
		return Err("Enter a search of at most 256 characters.");
	}
	let mut content = Vec::new();
	let mut filters = Vec::new();
	for token in query.split_whitespace() {
		let Some((key, value)) = token.split_once(':') else {
			content.push(token);
			continue;
		};
		let parameter = match key {
			"from" | "mentions" | "before_id" | "after_id" => {
				if value.parse::<u64>().ok().is_none_or(|id| id == 0) {
					return Err("Choose a user or enter a valid numeric ID.");
				}
				match key {
					"from" => "author_id",
					"mentions" => "mentions",
					"before_id" => "max_id",
					_ => "min_id",
				}
			}
			"has" => {
				if !matches!(
					value,
					"link" | "embed" | "file" | "image" | "video" | "sound"
				) {
					return Err("Choose link, embed, file, image, video or sound.");
				}
				"has"
			}
			"author_type" => {
				if !matches!(value, "user" | "bot" | "webhook") {
					return Err("Choose user, bot or webhook.");
				}
				"author_type"
			}
			"pinned" => {
				if !matches!(value, "true" | "false") {
					return Err("Choose true or false for Pinned.");
				}
				"pinned"
			}
			_ => {
				content.push(token);
				continue;
			}
		};
		if filters.len() >= 16 {
			return Err("Use at most 16 search filters.");
		}
		filters.push((parameter, value.to_owned()));
	}
	let content = if filters.is_empty() {
		query.to_owned()
	} else {
		content.join(" ")
	};
	Ok((content, filters))
}
pub struct SearchHit {
	pub id: Id,
	pub channel: Id,
	pub author: crate::User,
	pub excerpt: String,
	/// Media shown under the excerpt, bounded like message attachments.
	pub attachments: Vec<crate::Attachment>,
	pub embeds: Vec<crate::Embed>,
}
pub struct SearchPage {
	pub hits: Vec<SearchHit>,
	pub total: u64,
	pub partial: bool,
	/// Oldest pin timestamp in nanoseconds, only when more pins are available.
	pub pin_cursor: Option<i128>,
}
impl SearchPage {
	pub fn bytes(&self) -> usize {
		self.hits.capacity() * size_of::<SearchHit>()
			+ self
				.hits
				.iter()
				.map(|h| {
					h.author.heap_bytes()
						+ h.excerpt.capacity()
						+ h.attachments.capacity() * size_of::<crate::Attachment>()
						+ h.attachments
							.iter()
							.map(crate::Attachment::bytes)
							.sum::<usize>() + h.embeds.capacity() * size_of::<crate::Embed>()
						+ h.embeds.iter().map(crate::Embed::bytes).sum::<usize>()
				})
				.sum::<usize>()
	}
	pub fn valid(&self, channel: Id, before: Option<Id>) -> bool {
		self.valid_pins(channel)
			&& self.pin_cursor.is_none()
			&& self.hits.iter().all(|h| before.is_none_or(|b| h.id < b))
			&& self.hits.windows(2).all(|w| w[0].id > w[1].id)
	}
	/// Pin order follows pin time, not message creation time.
	pub fn valid_pins(&self, channel: Id) -> bool {
		self.hits.len() <= SEARCH_PAGE_SIZE
			&& self.bytes() <= MAX_SEARCH_BYTES
			&& self.hits.iter().all(|h| {
				h.id.0 > 0
					&& h.channel == channel
					&& h.author.name.len() <= 512
					&& h.attachments.len() <= crate::MAX_ATTACHMENTS
					&& h.embeds.len() <= crate::MAX_EMBEDS
					&& h.excerpt.len() <= 8192
			}) && self
			.hits
			.iter()
			.enumerate()
			.all(|(i, h)| self.hits[..i].iter().all(|other| other.id != h.id))
	}
}
