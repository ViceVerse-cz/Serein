use crate::{Id, ReactionEmoji};

/// One selected Unicode or custom reaction, without heap storage in the settings queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedReactionEmoji {
	id: Option<Id>,
	name: [u8; 128],
	len: u8,
}
impl SavedReactionEmoji {
	pub fn new(id: Option<Id>, name: &str) -> Option<Self> {
		if name.is_empty()
			|| name.len() > 128
			|| name.chars().any(char::is_control)
			|| id.is_some_and(|id| id.0 == 0)
			|| (id.is_some()
				&& (!(2..=32).contains(&name.len())
					|| !name
						.bytes()
						.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')))
		{
			return None;
		}
		let mut bytes = [0; 128];
		bytes[..name.len()].copy_from_slice(name.as_bytes());
		Some(Self {
			id,
			name: bytes,
			len: name.len() as u8,
		})
	}
	pub fn from_emoji(emoji: &ReactionEmoji) -> Option<Self> {
		Self::new(emoji.id, emoji.name.as_deref()?)
	}
	pub fn id(self) -> Option<Id> {
		self.id
	}
	pub fn name(&self) -> &str {
		// Only the UTF-8 constructor can populate this private buffer.
		std::str::from_utf8(&self.name[..usize::from(self.len)]).unwrap_or_default()
	}
	pub fn emoji(self) -> ReactionEmoji {
		ReactionEmoji {
			id: self.id,
			name: Some(self.name().to_owned()),
		}
	}
}

/// Fixed-size, application-wide reading settings; independent of Discord accounts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadingPreferences {
	pub zoom_percent: u16,
	pub sidebar_width: u16,
	/// Member list in servers; direct and group messages keep their own choice.
	pub show_members: bool,
	pub show_members_dms: bool,
	/// Tighter gaps between message groups.
	pub compact_messages: bool,
	/// Explicit opt-in; double-clicks do not react by default.
	pub double_click_reaction_enabled: bool,
	/// Stable index into DOUBLE_CLICK_REACTIONS, independent of emoji usage rankings.
	pub double_click_reaction: u8,
	/// Full-picker selection; absent for the six legacy presets.
	pub double_click_reaction_emoji: Option<SavedReactionEmoji>,
	pub animate_gifs: bool,
	pub smooth_scrolling: bool,
	pub scroll_speed_percent: u16,
	pub hide_media_links: bool,
	pub confirm_external_links: bool,
}
impl Default for ReadingPreferences {
	fn default() -> Self {
		Self {
			zoom_percent: 100,
			sidebar_width: 236,
			show_members: true,
			show_members_dms: true,
			compact_messages: false,
			double_click_reaction_enabled: false,
			double_click_reaction: 0,
			double_click_reaction_emoji: None,
			animate_gifs: true,
			smooth_scrolling: true,
			scroll_speed_percent: 100,
			hide_media_links: true,
			confirm_external_links: true,
		}
	}
}
impl ReadingPreferences {
	pub const DOUBLE_CLICK_REACTIONS: [&'static str; 6] = ["❤️", "👍", "😂", "🎉", "😮", "😢"];

	pub fn double_click_emoji(&self) -> &str {
		if let Some(emoji) = &self.double_click_reaction_emoji {
			return emoji.name();
		}
		Self::DOUBLE_CLICK_REACTIONS
			.get(usize::from(self.double_click_reaction))
			.copied()
			.unwrap_or(Self::DOUBLE_CLICK_REACTIONS[0])
	}

	pub fn double_click_choice(self) -> SavedReactionEmoji {
		self.double_click_reaction_emoji.unwrap_or_else(|| {
			SavedReactionEmoji::new(None, self.double_click_emoji())
				.expect("legacy reaction presets are valid emoji")
		})
	}

	pub fn is_valid(self) -> bool {
		(50..=150).contains(&self.zoom_percent)
			&& (190..=360).contains(&self.sidebar_width)
			&& (25..=300).contains(&self.scroll_speed_percent)
			&& usize::from(self.double_click_reaction) < Self::DOUBLE_CLICK_REACTIONS.len()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn saved_reaction_keeps_unicode_and_custom_identity_with_fixed_bounds() {
		for (id, text) in [(None, "👨‍👩‍👧‍👦"), (Some(Id(9001)), "serein_wave")] {
			let saved = SavedReactionEmoji::new(id, text).unwrap();
			assert_eq!(saved.id(), id);
			assert_eq!(saved.name(), text);
			assert_eq!(SavedReactionEmoji::from_emoji(&saved.emoji()), Some(saved));
			let preferences = ReadingPreferences {
				double_click_reaction_emoji: Some(saved),
				..Default::default()
			};
			assert!(preferences.is_valid());
			assert_eq!(preferences.double_click_choice(), saved);
		}
		for (id, text) in [
			(None, ""),
			(None, "a\nb"),
			(Some(Id(0)), "wave"),
			(Some(Id(1)), "not a name"),
			(Some(Id(1)), "x"),
		] {
			assert!(SavedReactionEmoji::new(id, text).is_none());
		}
		assert!(SavedReactionEmoji::new(None, &"🦀".repeat(32)).is_some());
		assert!(SavedReactionEmoji::new(None, &"🦀".repeat(33)).is_none());
		assert!(SavedReactionEmoji::new(Some(Id(1)), &"x".repeat(33)).is_none());
		assert!(std::mem::size_of::<SavedReactionEmoji>() <= 160);
	}

	#[test]
	fn defaults_and_inclusive_bounds() {
		let defaults = ReadingPreferences::default();
		assert_eq!(defaults.zoom_percent, 100);
		assert_eq!(defaults.sidebar_width, 236);
		assert!(defaults.show_members && defaults.smooth_scrolling && defaults.is_valid());
		for zoom_percent in [0, 49, 50, 79, 80, 150, 151, u16::MAX] {
			for sidebar_width in [0, 189, 190, 360, 361, u16::MAX] {
				for show_members in [false, true] {
					let preferences = ReadingPreferences {
						zoom_percent,
						sidebar_width,
						show_members,
						show_members_dms: show_members,
						compact_messages: false,
						double_click_reaction_enabled: false,
						double_click_reaction: 0,
						double_click_reaction_emoji: None,
						animate_gifs: false,
						smooth_scrolling: true,
						scroll_speed_percent: 100,
						hide_media_links: true,
						confirm_external_links: true,
					};
					assert_eq!(
						preferences.is_valid(),
						matches!(zoom_percent, 50 | 79 | 80 | 150)
							&& matches!(sidebar_width, 190 | 360)
					);
				}
			}
		}
	}
}
