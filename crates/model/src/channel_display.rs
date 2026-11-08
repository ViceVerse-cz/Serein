//! Device-local channel-list appearance controls.
//!
//! The service does not expose a stable display-name spelling, so this module only
//! normalizes what the session already loaded. It never writes names back and never
//! invents channel metadata.

use crate::Id;

pub const MAX_IGNORED_NAMES: usize = 200;
pub const MAX_DISPLAY_NAME_CHARS: usize = 100;

/// Device preferences shared by the channel list, header and member list.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChannelDisplayPreferences {
	/// Show a normalized channel name instead of the service spelling.
	pub clean_names: bool,
	/// Channels that keep their service name while `clean_names` is on.
	pub ignored_clean_names: Vec<Id>,
	/// Show a fixed channel-kind badge in the channel list.
	pub type_badges: bool,
	/// Hide only member groups the loaded role list cannot resolve.
	pub hide_no_role_headers: bool,
}

impl ChannelDisplayPreferences {
	pub fn is_valid(&self) -> bool {
		self.ignored_clean_names.len() <= MAX_IGNORED_NAMES
			&& self.ignored_clean_names.capacity() <= MAX_IGNORED_NAMES * 2
			&& self
				.ignored_clean_names
				.iter()
				.enumerate()
				.all(|(index, id)| id.0 != 0 && !self.ignored_clean_names[..index].contains(id))
	}

	pub fn ignored(&self, channel: Id) -> bool {
		self.ignored_clean_names.contains(&channel)
	}

	/// Idempotent. Returns whether the stored list changed.
	pub fn set_ignored(&mut self, channel: Id, ignored: bool) -> bool {
		if channel.0 == 0 || !self.is_valid() || self.ignored(channel) == ignored {
			return false;
		}
		if ignored {
			if self.ignored_clean_names.len() >= MAX_IGNORED_NAMES {
				return false;
			}
			self.ignored_clean_names.push(channel);
		} else {
			self.ignored_clean_names.retain(|id| *id != channel);
		}
		true
	}
}

/// A fixed badge for channel kinds this client actually renders. `None` means the
/// loaded kind is unknown or has no truthful badge, so no placeholder is shown.
pub fn channel_badge(kind: u8) -> Option<&'static str> {
	match kind {
		0 => Some("Text"),
		2 => Some("Voice"),
		4 => Some("Category"),
		5 => Some("News"),
		10..=12 => Some("Thread"),
		13 => Some("Stage"),
		14 => Some("Directory"),
		15 => Some("Forum"),
		16 => Some("Media"),
		_ => None,
	}
}

/// A group whose id is neither a status group nor a loaded role. Discord's member
/// list uses this pseudo group for members without a role; it is only safe to hide
/// after role metadata loaded, otherwise every role would look unresolved.
pub fn is_pseudo_member_group(id: &str, roles_loaded: bool, role_known: bool) -> bool {
	roles_loaded && !matches!(id, "online" | "offline") && !role_known
}

const SMALL_CAPS: &[(char, char)] = &[
	('ᴀ', 'a'),
	('ʙ', 'b'),
	('ᴄ', 'c'),
	('ᴅ', 'd'),
	('ᴇ', 'e'),
	('ꜰ', 'f'),
	('ɢ', 'g'),
	('ʜ', 'h'),
	('ɪ', 'i'),
	('ᴊ', 'j'),
	('ᴋ', 'k'),
	('ʟ', 'l'),
	('ᴍ', 'm'),
	('ɴ', 'n'),
	('ᴏ', 'o'),
	('ᴘ', 'p'),
	('ǫ', 'q'),
	('ʀ', 'r'),
	('ꜱ', 's'),
	('ᴛ', 't'),
	('ᴜ', 'u'),
	('ᴠ', 'v'),
	('ᴡ', 'w'),
	('ʏ', 'y'),
	('ᴢ', 'z'),
];

fn is_emoji_or_variation(c: char) -> bool {
	matches!(
		c as u32,
		0x1F000..=0x1FAFF
			| 0x2600..=0x27BF
			| 0x2B00..=0x2BFF
			| 0xFE00..=0xFE0F
			| 0x200D
			| 0x20E3
	)
}

fn is_invisible(c: char) -> bool {
	c.is_control()
		|| matches!(
			c,
			'\u{200B}'..='\u{200F}'
				| '\u{202A}'..='\u{202E}'
				| '\u{2060}'..='\u{206F}'
				| '\u{FEFF}'
		)
}

fn bounded(value: &str) -> String {
	value
		.chars()
		.filter(|c| !is_invisible(*c))
		.take(MAX_DISPLAY_NAME_CHARS)
		.collect()
}

/// Normalize decoration without changing the service value. Rust's standard library
/// has no Unicode normalization tables, so this is an explicit safe fallback: small
/// caps, fullwidth ASCII and common pictographs are folded, then separators are
/// collapsed. The result is bounded and never empty when the input had content.
pub fn clean_channel_name(name: &str, kind: u8) -> String {
	let separator = if matches!(kind, 2 | 13) { ' ' } else { '-' };
	let mut cleaned = String::with_capacity(name.len().min(MAX_DISPLAY_NAME_CHARS * 4));
	for c in name.chars() {
		if is_invisible(c) || is_emoji_or_variation(c) {
			continue;
		}
		let folded = SMALL_CAPS
			.iter()
			.find_map(|(from, to)| (*from == c).then_some(*to))
			.or_else(|| match c as u32 {
				0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0),
				0x3000 => Some(' '),
				_ => None,
			})
			.unwrap_or(c);
		if folded.is_alphanumeric() {
			cleaned.push(folded);
		} else if folded == ' ' {
			cleaned.push(' ');
		} else if folded == '-' && kind != 2 && kind != 13 {
			cleaned.push('-');
		} else {
			cleaned.push(separator);
		}
	}
	let mut out = String::with_capacity(cleaned.len());
	let mut previous_separator = false;
	for c in cleaned.chars() {
		if c == separator || c == ' ' {
			if !previous_separator && !out.is_empty() {
				out.push(separator);
			}
			previous_separator = true;
		} else {
			out.push(c);
			previous_separator = false;
		}
	}
	while out.ends_with(separator) || out.ends_with(' ') {
		out.pop();
	}
	let out: String = out
		.chars()
		.take(MAX_DISPLAY_NAME_CHARS)
		.collect::<String>()
		.trim()
		.to_owned();
	if out.is_empty() { bounded(name) } else { out }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn normalizes_small_caps_fullwidth_emoji_and_separators() {
		assert_eq!(clean_channel_name("  ɢᴇɴᴇʀᴀʟ  ", 0), "general");
		assert_eq!(clean_channel_name("ＧＥＮＥＲＡＬ", 0), "GENERAL");
		assert_eq!(clean_channel_name("🎉-general-🎉", 0), "general");
		assert_eq!(clean_channel_name("news || updates", 5), "news-updates");
		assert_eq!(clean_channel_name("voice | lounge", 2), "voice lounge");
		assert_eq!(clean_channel_name("\u{202E}evil\u{200B}", 0), "evil");
	}

	#[test]
	fn keeps_a_bounded_fallback_and_caps_display_names() {
		assert_eq!(clean_channel_name("🎉", 0), "🎉");
		assert_eq!(clean_channel_name("", 0), "");
		assert_eq!(clean_channel_name("a".repeat(300).as_str(), 0).len(), 100);
	}

	#[test]
	fn ignored_ids_are_bounded_unique_and_removable() {
		let mut preferences = ChannelDisplayPreferences::default();
		assert!(preferences.set_ignored(Id(1), true));
		assert!(!preferences.set_ignored(Id(1), true));
		assert!(preferences.ignored(Id(1)));
		assert!(preferences.set_ignored(Id(1), false));
		assert!(!preferences.ignored(Id(1)));
		for id in 1..=MAX_IGNORED_NAMES as u64 {
			assert!(preferences.set_ignored(Id(id), true));
		}
		assert!(!preferences.set_ignored(Id(MAX_IGNORED_NAMES as u64 + 1), true));
		assert!(preferences.is_valid());
		preferences
			.ignored_clean_names
			.push(Id(MAX_IGNORED_NAMES as u64 + 1));
		assert!(!preferences.is_valid());
	}

	#[test]
	fn badges_and_pseudo_groups_never_guess_missing_metadata() {
		assert_eq!(channel_badge(0), Some("Text"));
		assert_eq!(channel_badge(2), Some("Voice"));
		assert_eq!(channel_badge(16), Some("Media"));
		assert_eq!(channel_badge(99), None);
		assert!(!is_pseudo_member_group("online", true, false));
		assert!(!is_pseudo_member_group("offline", true, false));
		assert!(!is_pseudo_member_group("12", false, false));
		assert!(!is_pseudo_member_group("12", true, true));
		assert!(is_pseudo_member_group("12", true, false));
	}
}
