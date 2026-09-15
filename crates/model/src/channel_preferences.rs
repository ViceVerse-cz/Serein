use crate::{Channel, Id};

/// Which device-local shortcut list a channel belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Shortcut {
	Pinned,
	Favorite,
}

/// Outcome of one shortcut write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreferenceEdit {
	Unchanged,
	Changed,
	CapacityReached,
}

/// Device-local channel shortcuts, isolated by account; never synchronized to Discord.
/// At most 256 IDs (2 KiB of ID payload) across both lists.
/// Vec order is display order: index 0 is shown first, and a new entry goes to the front.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChannelPreferences {
	pub favorites: Vec<Id>,
	pub pinned: Vec<Id>,
}

impl ChannelPreferences {
	pub const MAX_ENTRIES: usize = 256;
	pub const MAX_JSON_BYTES: usize = 8192;

	pub fn is_valid(&self) -> bool {
		// ponytail: duplicate scans are capped at 256 IDs; use a set if this limit grows.
		self.favorites.len().saturating_add(self.pinned.len()) <= Self::MAX_ENTRIES
			&& self
				.favorites
				.capacity()
				.saturating_add(self.pinned.capacity())
				<= Self::MAX_ENTRIES * 2
			&& [&self.favorites, &self.pinned].into_iter().all(|ids| {
				ids.iter()
					.enumerate()
					.all(|(index, id)| id.0 != 0 && !ids[..index].contains(id))
			})
	}

	fn list(&self, kind: Shortcut) -> &Vec<Id> {
		match kind {
			Shortcut::Pinned => &self.pinned,
			Shortcut::Favorite => &self.favorites,
		}
	}

	/// Display-ordered ids of one list.
	pub fn ids(&self, kind: Shortcut) -> &[Id] {
		self.list(kind)
	}

	pub fn contains(&self, kind: Shortcut, channel: Id) -> bool {
		self.list(kind).contains(&channel)
	}

	/// Home pins are 1:1 and group DMs only.
	pub fn can_pin(channel: &Channel) -> bool {
		channel.id.0 != 0 && channel.guild.is_none() && matches!(channel.kind, 1 | 3)
	}

	/// Server favorites are guild channels that are not categories.
	pub fn can_favorite(channel: &Channel) -> bool {
		channel.id.0 != 0 && channel.guild.is_some() && channel.kind != 4
	}

	/// Idempotent. Turning a shortcut on inserts it at the front of its list.
	pub fn set(&mut self, kind: Shortcut, channel: Id, on: bool) -> PreferenceEdit {
		if channel.0 == 0 || !self.is_valid() || self.contains(kind, channel) == on {
			return PreferenceEdit::Unchanged;
		}
		let full = self.favorites.len() + self.pinned.len() == Self::MAX_ENTRIES;
		let ids = match kind {
			Shortcut::Pinned => &mut self.pinned,
			Shortcut::Favorite => &mut self.favorites,
		};
		if on {
			if full {
				return PreferenceEdit::CapacityReached;
			}
			ids.insert(0, channel);
		} else {
			ids.retain(|id| *id != channel);
		}
		PreferenceEdit::Changed
	}

	/// Drops a confirmed-deleted channel from both shortcut lists.
	pub fn forget(&mut self, channel: Id) -> bool {
		let before = self.favorites.len() + self.pinned.len();
		self.favorites.retain(|id| *id != channel);
		self.pinned.retain(|id| *id != channel);
		before != self.favorites.len() + self.pinned.len()
	}
}
