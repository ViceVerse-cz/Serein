//! Public breach-source lookups for identifiers a profile owner publishes themselves.
//! Bounded extraction, deduplicated in-flight lookups and a small result cache; the
//! network side lives in `discord_api::breachcheck` and reports source names only.
use std::{
	collections::BTreeMap,
	time::{Duration, Instant},
};

use model::{Id, UserProfile};

use crate::{Command, State, auth::AuthState};

/// One public breach index; ordering is preference, not correctness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
	LeakCheck,
	XposedOrNot,
}

impl Source {
	pub const ALL: [Source; 2] = [Source::LeakCheck, Source::XposedOrNot];
	pub fn name(self) -> &'static str {
		match self {
			Self::LeakCheck => "leakcheck",
			Self::XposedOrNot => "xposedornot",
		}
	}
}

/// Bounded summary of where one identifier appears; names and categories only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BreachReport {
	pub sources: Vec<String>,
	pub fields: Vec<String>,
	pub found: usize,
	pub source: &'static str,
}

pub const MAX_SOURCES: usize = 16;
pub const MAX_FIELDS: usize = 16;
pub const MAX_NAME_CHARS: usize = 80;
/// Identifiers extracted per profile; each may fan out to every source.
pub const MAX_QUERIES: usize = 4;
/// Cached users; oldest entries are evicted first.
pub const MAX_CACHED_USERS: usize = 32;
const REPORT_TTL: Duration = Duration::from_secs(24 * 3600);
/// A dropped lookup task leaves its marker; it expires so the card can retry.
const PENDING_TTL: Duration = Duration::from_secs(90);
const MAX_EMAILS_PER_BIO: usize = 2;
const MAX_BIO_SCAN: usize = 4096;
const MAX_IDENTIFIER_CHARS: usize = 254;

/// Append `value` to `list` when bounded, clean and not already present.
pub fn push_bounded(list: &mut Vec<String>, value: &str, cap: usize) {
	if list.len() >= cap || value.is_empty() || value.chars().count() > MAX_NAME_CHARS {
		return;
	}
	let cleaned: String = value
		.chars()
		.filter(|c| !c.is_control())
		.take(MAX_NAME_CHARS)
		.collect();
	if !cleaned.is_empty() && !list.iter().any(|seen| seen == &cleaned) {
		list.push(cleaned);
	}
}

/// Scan `text` for email-shaped tokens, bounded and deduplicated.
fn emails_in(text: &str, out: &mut Vec<String>) {
	let bytes = text.as_bytes();
	let mut i = 0;
	while i < bytes.len() && out.len() < MAX_EMAILS_PER_BIO {
		if bytes[i] == b'@' {
			let mut start = i;
			while start > 0
				&& matches!(
					bytes[start - 1],
					b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'%' | b'+' | b'-'
				) {
				start -= 1;
			}
			let mut end = i + 1;
			while end < bytes.len()
				&& matches!(
					bytes[end],
					b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-'
				) {
				end += 1;
			}
			let candidate = &text[start..end];
			let domain_ok = candidate[i - start + 1..]
				.rsplit('.')
				.next()
				.is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
			if start < i && end > i + 1 && candidate.len() <= MAX_IDENTIFIER_CHARS && domain_ok {
				let candidate = candidate.to_owned();
				if !out.contains(&candidate) {
					out.push(candidate);
				}
			}
			i = end;
		} else {
			i += 1;
		}
	}
}

/// A handle usable in public breach indexes: 3-64 chars of the usual handle alphabet.
fn handle_like(value: &str) -> bool {
	let len = value.chars().count();
	(3..=64).contains(&len)
		&& value
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
		&& value.chars().any(|c| c.is_ascii_alphanumeric())
}

/// Identifiers a profile owner chose to publish: bio emails, then the account name and
/// connected-service handles. Stable order; the caller caps fan-out at MAX_QUERIES.
pub fn identifiers(profile: &UserProfile) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	// Bound the bio scan on a char boundary; multibyte tails are dropped, never split.
	let bio: String = profile.bio.chars().take(MAX_BIO_SCAN).collect();
	emails_in(&bio, &mut out);
	for candidate in [
		profile.username.as_str(),
		profile.global_name.as_deref().unwrap_or(""),
	] {
		if handle_like(candidate) && !out.iter().any(|seen| seen == candidate) {
			out.push(candidate.to_owned());
		}
	}
	for connection in &profile.connections {
		for candidate in [&connection.name, &connection.id] {
			if handle_like(candidate) && !out.iter().any(|seen| seen == candidate) {
				out.push(candidate.clone());
			}
		}
	}
	out.truncate(MAX_QUERIES);
	out
}

/// Commands carry user-influenced query lists; keep them inside extraction bounds.
pub fn valid_queries(queries: &[String]) -> bool {
	queries.len() <= MAX_QUERIES
		&& queries.iter().all(|q| {
			!q.is_empty()
				&& q.chars().count() <= MAX_IDENTIFIER_CHARS
				&& q.chars()
					.all(|c| c.is_ascii() && !c.is_control() && !c.is_whitespace())
		})
}

#[derive(Clone, Debug)]
struct Entry {
	queries: Vec<String>,
	reports: Vec<(String, BreachReport)>,
	fetched: Instant,
	pending: bool,
}

impl Default for Entry {
	fn default() -> Self {
		Self {
			queries: Vec::new(),
			reports: Vec::new(),
			fetched: Instant::now(),
			pending: false,
		}
	}
}

/// Bounded per-user breach state: in-flight markers plus timestamped results.
#[derive(Default)]
pub struct Breach {
	entries: BTreeMap<Id, Entry>,
}

impl Breach {
	pub fn pending(&self, user: Id) -> bool {
		self.entries
			.get(&user)
			.is_some_and(|e| e.pending && e.fetched.elapsed() < PENDING_TTL)
	}
	/// Reports for `user`; `Some` once the lookup settled, even when every index failed.
	pub fn reports(&self, user: Id) -> Option<&[(String, BreachReport)]> {
		let entry = self.entries.get(&user)?;
		(entry.fetched.elapsed() < REPORT_TTL && !entry.pending).then_some(entry.reports.as_slice())
	}
	/// Mark `user` as looking up `queries`; false when already pending or cached-fresh.
	pub fn begin(&mut self, user: Id, queries: Vec<String>, now: Instant) -> bool {
		if self.pending(user) || self.reports(user).is_some() {
			return false;
		}
		self.evict(now);
		let entry = self.entries.entry(user).or_default();
		entry.queries = queries;
		entry.reports.clear();
		entry.fetched = now;
		entry.pending = true;
		true
	}
	/// Store one index's report for an identifier; keyed by query and source.
	pub fn record(&mut self, user: Id, query: String, report: BreachReport) {
		if let Some(entry) = self.entries.get_mut(&user) {
			if let Some((_, slot)) = entry
				.reports
				.iter_mut()
				.find(|(q, r)| *q == query && r.source == report.source)
			{
				*slot = report;
			} else {
				entry.reports.push((query, report));
			}
		}
	}
	/// All expected queries reported; clears the pending marker.
	pub fn finish(&mut self, user: Id, now: Instant) {
		if let Some(entry) = self.entries.get_mut(&user) {
			entry.pending = false;
			entry.fetched = now;
		}
	}
	/// Drop the in-flight marker without results (lookup aborted or rejected).
	pub fn cancel(&mut self, user: Id) {
		self.entries.remove(&user);
	}
	/// Queries the in-flight lookup still owes, for progress tracking.
	pub fn queries(&self, user: Id) -> &[String] {
		self.entries
			.get(&user)
			.map_or(&[], |e| e.queries.as_slice())
	}
	fn evict(&mut self, now: Instant) {
		self.entries
			.retain(|_, e| now.saturating_duration_since(e.fetched) < REPORT_TTL);
		while self.entries.len() > MAX_CACHED_USERS {
			let Some(oldest) = self
				.entries
				.iter()
				.min_by_key(|(_, e)| e.fetched)
				.map(|(id, _)| *id)
			else {
				break;
			};
			self.entries.remove(&oldest);
		}
	}
	pub fn bytes(&self) -> usize {
		self.entries
			.values()
			.map(|e| {
				size_of::<Entry>()
					+ e.queries.iter().map(String::capacity).sum::<usize>()
					+ e.reports
						.iter()
						.map(|(q, r)| {
							q.capacity()
								+ r.sources.iter().map(String::capacity).sum::<usize>()
								+ r.fields.iter().map(String::capacity).sum::<usize>()
						})
						.sum::<usize>()
			})
			.sum()
	}
}

impl State {
	/// Queue public breach lookups for the identifiers `user`'s profile publishes.
	/// Returns the command to send; None when ineligible, private, empty or already done.
	pub fn request_breach_check(&mut self, user: Id) -> Option<Command> {
		if self.demo
			|| self.auth != AuthState::Authenticated
			|| !self.gateway_connected
			|| self.breach.pending(user)
			|| self.breach.reports(user).is_some()
		{
			return None;
		}
		let profile = self
			.profile
			.as_ref()
			.filter(|view| view.user == user)
			.and_then(|view| view.data.as_ref())
			.or_else(|| self.known_profile(user))?;
		if profile.private {
			return None;
		}
		let queries = identifiers(profile);
		if queries.is_empty() || !valid_queries(&queries) {
			return None;
		}
		self.breach
			.begin(user, queries.clone(), Instant::now())
			.then(|| Command::BreachCheck { user, queries })
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::UserProfile;

	fn profile(bio: &str, username: &str, connections: &[(&str, &str)]) -> UserProfile {
		UserProfile {
			user: model::User {
				primary_guild: None,
				cosmetics: None,
				id: Id(1),
				name: username.into(),
				avatar: None,
				webhook: false,
				kind: Default::default(),
				discriminator: 0,
			},
			username: username.into(),
			global_name: None,
			banner: None,
			accent_color: None,
			bio: bio.into(),
			pronouns: String::new(),
			badges: Vec::new(),
			connections: connections
				.iter()
				.map(|(kind, name)| model::ProfileConnection {
					kind: kind.to_string(),
					id: name.to_string(),
					name: name.to_string(),
					verified: false,
				})
				.collect(),
			mutual_guilds: Vec::new(),
			mutual_friends: Vec::new(),
			guild: None,
			theme_colors: None,
			clan: None,
			board: None,
			effect_sku: None,
			frame_sku: None,
			effect: None,
			frame: None,
			badge_catalog: None,
			private: false,
			limited: false,
		}
	}

	#[test]
	fn identifiers_extract_published_emails_and_handles() {
		let p = profile(
			"reach me at dev@example.com or backup@mail.io",
			"some_user",
			&[("github", "octo_cat")],
		);
		let ids = identifiers(&p);
		assert_eq!(ids[0], "dev@example.com");
		assert!(ids.contains(&"some_user".to_string()));
		assert!(ids.contains(&"octo_cat".to_string()));
		assert!(ids.len() <= MAX_QUERIES);
	}

	#[test]
	fn identifiers_skip_non_handle_names_and_dedupe() {
		let p = profile("no emails here", "ab", &[("x", "has space")]);
		let ids = identifiers(&p);
		assert!(!ids.iter().any(|i| i.contains(' ')));
		assert!(ids.iter().all(|i| handle_like(i)));
	}

	#[test]
	fn identifiers_cap_at_max_queries() {
		let p = profile(
			"a@b.co c@d.io e@f.gg",
			"user_one",
			&[("a", "handle_a"), ("b", "handle_b"), ("c", "handle_c")],
		);
		assert_eq!(identifiers(&p).len(), MAX_QUERIES);
	}

	#[test]
	fn email_scan_handles_multibyte_and_bounds() {
		let p = profile("héllo wörld ✨ me@x.io", "u", &[]);
		let ids = identifiers(&p);
		assert_eq!(ids, vec!["me@x.io".to_string()]);
	}

	#[test]
	fn queries_validation_rejects_whitespace_and_overflow() {
		assert!(valid_queries(&["a@b.c".into(), "handle".into()]));
		assert!(!valid_queries(&["bad query".into()]));
		assert!(!valid_queries(&vec!["x".into(); MAX_QUERIES + 1]));
	}

	#[test]
	fn breach_state_dedupes_pending_and_evicts() {
		let mut state = Breach::default();
		let now = Instant::now();
		assert!(state.begin(Id(1), vec!["a@b.c".into()], now));
		assert!(!state.begin(Id(1), vec!["a@b.c".into()], now));
		assert!(state.pending(Id(1)));
		state.record(Id(1), "a@b.c".into(), BreachReport::default());
		state.finish(Id(1), now);
		assert!(!state.pending(Id(1)));
		let reports = state.reports(Id(1)).unwrap();
		assert_eq!(reports.len(), 1);
		assert_eq!(reports[0].0, "a@b.c");
	}
}
