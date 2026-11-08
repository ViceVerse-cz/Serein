//! Bounded account-local notification filters. Nothing here is sent to Discord.
use crate::{Embed, Id};

/// Watched authors are explicit user IDs, never names or wildcard patterns.
pub const MAX_WATCH_USERS: usize = 64;
/// Literal keywords only: no regular expressions, so a saved list cannot become a ReDoS.
pub const MAX_KEYWORDS: usize = 32;
pub const MAX_KEYWORD_CHARS: usize = 64;
/// Guild/channel scopes for keyword delivery, kept small enough to scan per message.
pub const MAX_KEYWORD_SCOPES: usize = 16;
/// Optional in-app toast exclusions.
pub const MAX_TOAST_IGNORE_USERS: usize = 16;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NotificationTools {
	/// One DM alert per unread run; opening or reading the DM resets it.
	pub one_ping_per_dm: bool,
	/// Authors whose accepted live messages always produce a local alert.
	pub watch_users: Vec<Id>,
	/// Case-insensitive literal keywords matched against message and embed text.
	pub keywords: Vec<String>,
	/// When non-empty, keyword alerts require the guild or channel to be listed.
	pub keyword_allow: Vec<Id>,
	/// Guild or channel IDs that never produce keyword alerts.
	pub keyword_deny: Vec<Id>,
	/// Show transient in-app toasts for admitted message alerts.
	pub toasts: bool,
	/// Authors whose messages never produce an in-app toast.
	pub toast_ignore_users: Vec<Id>,
}

fn valid_ids(ids: &Vec<Id>, max: usize) -> bool {
	ids.len() <= max
		&& ids.capacity() <= max
		&& ids
			.iter()
			.enumerate()
			.all(|(index, id)| id.0 != 0 && !ids[..index].contains(id))
}

fn valid_keywords(keywords: &Vec<String>) -> bool {
	keywords.len() <= MAX_KEYWORDS
		&& keywords.capacity() <= MAX_KEYWORDS
		&& keywords.iter().enumerate().all(|(index, keyword)| {
			!keyword.is_empty()
				&& keyword.chars().count() <= MAX_KEYWORD_CHARS
				&& !keyword.chars().any(char::is_control)
				&& !keywords[..index].iter().any(|other| other == keyword)
		})
}

impl NotificationTools {
	pub fn is_valid(&self) -> bool {
		valid_ids(&self.watch_users, MAX_WATCH_USERS)
			&& valid_keywords(&self.keywords)
			&& valid_ids(&self.keyword_allow, MAX_KEYWORD_SCOPES)
			&& valid_ids(&self.keyword_deny, MAX_KEYWORD_SCOPES)
			&& valid_ids(&self.toast_ignore_users, MAX_TOAST_IGNORE_USERS)
	}

	fn text_matches(&self, text: &str) -> bool {
		if text.is_empty() {
			return false;
		}
		let text = text.to_lowercase();
		self.keywords
			.iter()
			.any(|keyword| text.contains(&keyword.to_lowercase()))
	}

	/// Literal, case-insensitive match over the current message and embed text only.
	/// Image URLs and other media metadata are deliberately not scanned.
	pub fn keyword_matches(
		&self,
		content: &str,
		embeds: &[Embed],
		channel: Id,
		guild: Option<Id>,
	) -> bool {
		if self.keywords.is_empty() {
			return false;
		}
		let scoped = |id: Id| id == channel || Some(id) == guild;
		if self.keyword_deny.iter().any(|id| scoped(*id)) {
			return false;
		}
		if !self.keyword_allow.is_empty() && !self.keyword_allow.iter().any(|id| scoped(*id)) {
			return false;
		}
		if self.text_matches(content) {
			return true;
		}
		embeds.iter().any(|embed| {
			embed
				.title
				.as_deref()
				.is_some_and(|text| self.text_matches(text))
				|| embed
					.description
					.as_deref()
					.is_some_and(|text| self.text_matches(text))
				|| embed
					.fields
					.iter()
					.any(|field| self.text_matches(&field.name) || self.text_matches(&field.value))
		})
	}

	/// Drops a deleted channel from keyword scopes; returns whether anything changed.
	pub fn forget_channel(&mut self, channel: Id) -> bool {
		let before = self.keyword_allow.len() + self.keyword_deny.len();
		self.keyword_allow.retain(|id| *id != channel);
		self.keyword_deny.retain(|id| *id != channel);
		before != self.keyword_allow.len() + self.keyword_deny.len()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::embeds::{Embed, EmbedField};

	fn embed(title: &str, description: &str, field: &str) -> Embed {
		Embed {
			title: Some(title.into()),
			description: Some(description.into()),
			fields: vec![EmbedField {
				name: "Field".into(),
				value: field.into(),
				inline: false,
			}],
			..Default::default()
		}
	}

	#[test]
	fn literal_keywords_scan_message_and_embed_text_but_not_media_urls() {
		let mut tools = NotificationTools {
			keywords: vec!["release".into(), ".*".into()],
			..Default::default()
		};
		assert!(tools.keyword_matches("A RELEASE is ready", &[], Id(1), Some(Id(2))));
		assert!(tools.keyword_matches("", &[embed("Notes", "release", "")], Id(1), None));
		assert!(tools.keyword_matches("", &[embed("", "", "release")], Id(1), None));
		assert!(
			!tools.keyword_matches("abc", &[], Id(1), None),
			"a regex-looking keyword stays literal"
		);
		let media = Embed {
			image: Some(crate::embeds::EmbedMedia {
				url: Some("https://example.invalid/release.png".into()),
				..Default::default()
			}),
			..Default::default()
		};
		assert!(!tools.keyword_matches("", &[media], Id(1), None));
		tools.keywords.clear();
		assert!(!tools.keyword_matches("release", &[], Id(1), None));
	}

	#[test]
	fn keyword_scopes_deny_first_and_allow_requires_a_match() {
		let tools = NotificationTools {
			keywords: vec!["release".into()],
			keyword_allow: vec![Id(10), Id(20)],
			keyword_deny: vec![Id(11)],
			..Default::default()
		};
		assert!(tools.keyword_matches("release", &[], Id(10), Some(Id(99))));
		assert!(tools.keyword_matches("release", &[], Id(99), Some(Id(20))));
		assert!(!tools.keyword_matches("release", &[], Id(12), Some(Id(99))));
		assert!(!tools.keyword_matches("release", &[], Id(11), Some(Id(20))));
	}

	#[test]
	fn caps_reject_duplicate_ids_and_oversized_or_control_keywords() {
		let mut tools = NotificationTools::default();
		assert!(tools.is_valid());
		tools.watch_users = (1..=MAX_WATCH_USERS as u64).map(Id).collect();
		assert!(tools.is_valid());
		tools.watch_users.push(Id(999));
		assert!(!tools.is_valid());
		tools.watch_users = vec![Id(1), Id(1)];
		assert!(!tools.is_valid());
		tools.watch_users.clear();
		tools.keywords = vec!["x".repeat(MAX_KEYWORD_CHARS + 1)];
		assert!(!tools.is_valid());
		tools.keywords = vec!["line\nbreak".into()];
		assert!(!tools.is_valid());
		tools.keywords = vec!["same".into(), "same".into()];
		assert!(!tools.is_valid());
		tools.keywords = (0..=MAX_KEYWORDS).map(|index| index.to_string()).collect();
		assert!(!tools.is_valid());
	}
}
