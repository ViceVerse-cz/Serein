//! Account messaging permissions, shared by the offline preview and service settings.
use crate::Id;

pub const MAX_GUILDS: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
	/// Service enum: 0 defaults to non-friends, 1 disabled, 2 non-friends, 3 all.
	pub spam_filter: u32,
	pub default_allow_dms: bool,
	pub restricted_guilds: Vec<Id>,
	pub default_filter_requests: bool,
	pub unfiltered_guilds: Vec<Id>,
	pub friend_source_flags: u32,
	pub personalized_requests: bool,
	pub game_friend_dms: bool,
	/// Service enum: 0 defaults to all, 1 all, 2 same game, 3 none.
	pub game_dms: u32,
}
impl Default for Snapshot {
	fn default() -> Self {
		Self {
			spam_filter: 0,
			default_allow_dms: true,
			restricted_guilds: Vec::new(),
			default_filter_requests: true,
			unfiltered_guilds: Vec::new(),
			friend_source_flags: 14,
			personalized_requests: true,
			game_friend_dms: true,
			game_dms: 0,
		}
	}
}
impl Snapshot {
	pub fn allow_dms(&self, guild: Option<Id>) -> bool {
		guild.map_or(self.default_allow_dms, |id| {
			self.restricted_guilds.binary_search(&id).is_err()
		})
	}
	pub fn filter_requests(&self, guild: Option<Id>) -> bool {
		guild.map_or(self.default_filter_requests, |id| {
			self.unfiltered_guilds.binary_search(&id).is_err()
		})
	}
	pub fn valid(&self) -> bool {
		[&self.restricted_guilds, &self.unfiltered_guilds]
			.into_iter()
			.all(|ids| {
				ids.len() <= MAX_GUILDS
					&& ids.capacity() <= MAX_GUILDS
					&& ids.iter().all(|id| id.0 != 0)
					&& ids.windows(2).all(|v| v[0] < v[1])
			})
	}
	pub fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ (self.restricted_guilds.capacity() + self.unfiltered_guilds.capacity())
				* std::mem::size_of::<Id>()
	}
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
	AllowAllDms { guilds: Vec<Id>, enabled: bool },
	FilterAllRequests { guilds: Vec<Id>, enabled: bool },
	SpamFilter(u32),
	DefaultAllowDms(bool),
	AllowGuildDms(Id, bool),
	DefaultFilterRequests(bool),
	FilterGuildRequests(Id, bool),
	Everyone(bool),
	FriendsOfFriends(bool),
	ServerMembers(bool),
	PersonalizedRequests(bool),
	GameFriendDms(bool),
	GameDms(u32),
}
impl Change {
	pub fn bytes(&self) -> usize {
		std::mem::size_of::<Self>()
			+ match self {
				Self::AllowAllDms { guilds, .. } | Self::FilterAllRequests { guilds, .. } => {
					guilds.capacity() * std::mem::size_of::<Id>()
				}
				_ => 0,
			}
	}
	pub fn valid(&self) -> bool {
		match self {
			Self::AllowAllDms { guilds, .. } | Self::FilterAllRequests { guilds, .. } => {
				guilds.capacity() <= MAX_GUILDS && guilds.iter().all(|id| id.0 != 0)
			}
			Self::SpamFilter(v) | Self::GameDms(v) => (1..=3).contains(v),
			Self::AllowGuildDms(id, _) | Self::FilterGuildRequests(id, _) => id.0 != 0,
			_ => true,
		}
	}
	pub fn apply(&self, value: &mut Snapshot) {
		match self {
			Self::AllowAllDms { guilds, enabled } | Self::FilterAllRequests { guilds, enabled } => {
				let ids = if matches!(self, Self::AllowAllDms { .. }) {
					value.default_allow_dms = *enabled;
					&mut value.restricted_guilds
				} else {
					value.default_filter_requests = *enabled;
					&mut value.unfiltered_guilds
				};
				let targets: std::collections::BTreeSet<_> = guilds.iter().copied().collect();
				ids.retain(|id| !targets.contains(id));
				if !enabled {
					ids.extend(targets);
				}
				ids.sort_unstable();
				ids.shrink_to_fit();
			}
			Self::SpamFilter(v) => value.spam_filter = *v,
			Self::DefaultAllowDms(v) => value.default_allow_dms = *v,
			Self::DefaultFilterRequests(v) => value.default_filter_requests = *v,
			Self::AllowGuildDms(id, allow) | Self::FilterGuildRequests(id, allow) => {
				let ids = if matches!(self, Self::AllowGuildDms(..)) {
					&mut value.restricted_guilds
				} else {
					&mut value.unfiltered_guilds
				};
				ids.retain(|v| v != id);
				if !allow {
					ids.push(*id);
				}
				ids.sort_unstable();
				ids.shrink_to_fit();
			}
			Self::Everyone(v) => {
				if *v {
					value.friend_source_flags |= 14;
				} else {
					value.friend_source_flags &= !8;
				}
			}
			Self::FriendsOfFriends(v) | Self::ServerMembers(v) => {
				let bit = if matches!(self, Self::FriendsOfFriends(_)) {
					2
				} else {
					4
				};
				if *v {
					value.friend_source_flags |= bit;
				} else {
					value.friend_source_flags &= !(bit | 8);
				}
			}
			Self::PersonalizedRequests(v) => value.personalized_requests = *v,
			Self::GameFriendDms(v) => value.game_friend_dms = *v,
			Self::GameDms(v) => value.game_dms = *v,
		}
	}
}
