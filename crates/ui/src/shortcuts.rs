//! Device-local pinned and favorite channels as sidebar rosters. Menus record an `Intent`;
//! `MessagingUi::apply_shortcut` is the only writer of `channel_preferences` in this crate.
use crate::MessagingUi;
use client_core::State;
use model::{Channel, ChannelPreferences, Id, PreferenceEdit, Shortcut};
use std::collections::{BTreeMap, BTreeSet};

/// Which sidebar list is being built. Owns every home-vs-guild policy for shortcuts.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
	Home,
	Guild(Id),
}

impl Scope {
	pub fn of(guild: Option<Id>) -> Self {
		guild.map_or(Self::Home, Self::Guild)
	}

	pub fn guild(self) -> Option<Id> {
		match self {
			Self::Home => None,
			Self::Guild(guild) => Some(guild),
		}
	}

	/// Channels this list may show at all, before permission filtering.
	pub fn admits(self, channel: &Channel, state: &State) -> bool {
		channel.guild == self.guild()
			&& channel.kind != 4
			&& (self.guild().is_some() || !state.spam_direct(channel.id))
	}

	/// Rosters in display order. Home pins DMs; guilds keep one Favorites shelf.
	pub fn rosters(self) -> &'static [(Shortcut, Heading)] {
		match self {
			Self::Home => &[(Shortcut::Pinned, Heading::Pinned)],
			Self::Guild(_) => &[(Shortcut::Favorite, Heading::Favorites)],
		}
	}

	/// The heading over the non-roster remainder, if that list has one.
	pub fn remainder_heading(self) -> Option<Heading> {
		match self {
			Self::Home => Some(Heading::DirectMessages),
			Self::Guild(_) => None,
		}
	}
}

/// Typed heading rows. `label` is the exact user-facing string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Heading {
	Pinned,
	DirectMessages,
	Favorites,
}

impl Heading {
	pub fn label(self) -> &'static str {
		match self {
			Self::Pinned => "Pinned",
			Self::DirectMessages => "Direct Messages",
			Self::Favorites => "Favorites",
		}
	}

	pub fn shelf(self) -> bool {
		matches!(self, Self::Pinned | Self::Favorites)
	}
}

/// One heading and its display-ordered channels.
pub(crate) struct Section<'a> {
	pub heading: Heading,
	pub kind: Shortcut,
	pub channels: Vec<&'a Channel>,
}

/// Roster rows for one scope, derived from preferences whenever the row cache rebuilds.
/// A channel in both lists appears once, under the first roster in `Scope::rosters`.
#[derive(Default)]
pub(crate) struct Roster<'a> {
	sections: Vec<Section<'a>>,
	lifted: BTreeSet<Id>,
}

impl<'a> Roster<'a> {
	pub fn build(
		state: &'a State,
		prefs: &ChannelPreferences,
		scope: Scope,
		show_hidden: bool,
	) -> Self {
		let rosters = scope.rosters();
		let mut rank = BTreeMap::new();
		for (index, (kind, _)) in rosters.iter().enumerate() {
			for (position, id) in prefs.ids(*kind).iter().enumerate() {
				rank.entry(*id).or_insert((index, position));
			}
		}
		let mut buckets: Vec<Vec<(usize, &Channel)>> = vec![Vec::new(); rosters.len()];
		for channel in &state.channels {
			if let Some(&(index, position)) = rank.get(&channel.id)
				&& scope.admits(channel, state)
				&& (show_hidden || state.can_view(channel.id))
			{
				buckets[index].push((position, channel));
			}
		}
		let mut lifted = BTreeSet::new();
		let sections = rosters
			.iter()
			.zip(buckets)
			.filter(|(_, bucket)| !bucket.is_empty())
			.map(|(&(kind, heading), mut bucket)| {
				bucket.sort_unstable_by_key(|(position, channel)| (*position, channel.id));
				lifted.extend(bucket.iter().map(|(_, channel)| channel.id));
				Section {
					heading,
					kind,
					channels: bucket.into_iter().map(|(_, channel)| channel).collect(),
				}
			})
			.collect();
		Self { sections, lifted }
	}

	pub fn sections(&self) -> &[Section<'a>] {
		&self.sections
	}

	/// Whether the regular list skips this channel. True for home pins and guild favorites.
	pub fn lifted(&self, channel: Id) -> bool {
		self.lifted.contains(&channel)
	}
}

/// A deferred local write recorded by a menu, applied once rows released their borrows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Intent {
	pub channel: Id,
	pub kind: Shortcut,
	pub on: bool,
}

/// Read-only handle menus use to label and enable shortcut rows.
#[derive(Clone, Copy)]
pub(crate) struct ShortcutView<'a> {
	prefs: &'a ChannelPreferences,
	available: bool,
}

impl<'a> ShortcutView<'a> {
	pub fn new(prefs: &'a ChannelPreferences, available: bool) -> Self {
		Self { prefs, available }
	}

	pub fn contains(self, kind: Shortcut, channel: Id) -> bool {
		self.prefs.contains(kind, channel)
	}

	pub fn available(self) -> bool {
		self.available
	}

	/// The intent that flips `kind` for `channel` from its current state.
	pub fn toggle(self, kind: Shortcut, channel: Id) -> Intent {
		Intent {
			channel,
			kind,
			on: !self.contains(kind, channel),
		}
	}
}

impl MessagingUi {
	/// Whether preferences are loaded (or the session is a demo) so a write would stick.
	pub(crate) fn shortcuts_available(&self, state: &State) -> bool {
		state.demo || self.channel_preferences_loaded
	}

	pub(crate) fn apply_shortcut(&mut self, state: &State, intent: Intent) {
		if !self.shortcuts_available(state) {
			return;
		}
		if intent.on {
			let Some(channel) = state.channel(intent.channel) else {
				return;
			};
			let allowed = match intent.kind {
				Shortcut::Pinned => ChannelPreferences::can_pin(channel),
				Shortcut::Favorite => ChannelPreferences::can_favorite(channel),
			};
			if !allowed {
				return;
			}
		}
		match self
			.channel_preferences
			.set(intent.kind, intent.channel, intent.on)
		{
			PreferenceEdit::Changed => {
				self.channel_preferences_changed = true;
				self.channel_cache.invalidate();
			}
			PreferenceEdit::CapacityReached => self.channel_menu.report_capacity(state.generation),
			PreferenceEdit::Unchanged => {}
		}
	}
}
