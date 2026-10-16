//! A user's badge catalog: owned and earnable badges with tiers, rarity and progress, plus
//! the account's own display order and hidden set. Unofficial normal-client behavior; see
//! docs/discord-compatibility.md. Never persisted.
use crate::ProfileBadge;

pub const MAX_CATALOG_BADGES: usize = 40;
pub const MAX_BADGE_TIERS: usize = 12;
pub const MAX_BADGE_TEXT: usize = 160;
pub const MAX_BADGE_URL: usize = 512;
/// Catalog memory retained beside one profile.
pub const MAX_CATALOG_BYTES: usize = 48 * 1024;
/// The official client shows at most this many badges in a profile's badge row.
pub const MAX_DISPLAYED_BADGES: usize = 6;

/// Numeric badge identities used by the catalog and badge settings.
pub mod id {
	pub const PREMIUM_TENURE: u32 = 1;
	pub const GUILD_BOOSTER: u32 = 12;
	pub const ACCOUNT_AGE: u32 = 18;
	pub const STREAMING: u32 = 19;
	pub const GAME_TIME: u32 = 20;
	pub const GAME_VARIETY: u32 = 21;
	pub const STAFF: u32 = 22;
}

/// Catalog numbers 1..=22 in order, named as the client lowercases them.
const NAMES: [&str; 22] = [
	"premium_tenure",
	"partner",
	"certified_moderator",
	"hypesquad",
	"hypesquad_house_1",
	"hypesquad_house_2",
	"hypesquad_house_3",
	"bug_hunter_level_1",
	"bug_hunter_level_2",
	"verified_developer",
	"early_supporter",
	"guild_booster",
	"legacy_username",
	"quest_completed",
	"april_fools_2026",
	"orb_profile",
	"gifting",
	"account_age",
	"streaming",
	"game_time",
	"game_variety",
	"staff",
];

/// Profile badges use string ids; the catalog uses numbers. Mirrors the client's mapping.
pub fn badge_number(profile_id: &str) -> Option<u32> {
	match profile_id {
		"staff" | "staff_hidden" | "staff_personal" => Some(id::STAFF),
		"premium" => Some(id::PREMIUM_TENURE),
		"orb_profile_badge" => Some(16),
		id if id.starts_with("premium_tenure_") => Some(id::PREMIUM_TENURE),
		id if id.starts_with("guild_booster_lvl") => Some(id::GUILD_BOOSTER),
		id => NAMES
			.iter()
			.position(|name| *name == id)
			.map(|index| index as u32 + 1),
	}
}

/// Lowercase profile id the client uses for a catalog badge.
pub fn badge_name(number: u32) -> String {
	match number {
		id::PREMIUM_TENURE => "premium".into(),
		16 => "orb_profile_badge".into(),
		_ => number
			.checked_sub(1)
			.and_then(|index| NAMES.get(index as usize))
			.map_or_else(|| number.to_string(), |name| (*name).to_owned()),
	}
}

/// Staff stays first and visible; Nitro tenure can be reordered but not hidden.
pub fn pinned(number: u32) -> bool {
	number == id::STAFF
}
pub fn hideable(number: u32) -> bool {
	!matches!(number, id::STAFF | id::PREMIUM_TENURE)
}

/// Bounded `https://cdn.discordapp.com` or `media.discordapp.net` artwork.
pub fn valid_badge_url(url: &str) -> bool {
	url.len() <= MAX_BADGE_URL
		&& [
			"https://cdn.discordapp.com/",
			"https://media.discordapp.net/",
		]
		.iter()
		.any(|prefix| {
			url.strip_prefix(prefix).is_some_and(|path| {
				!path.is_empty()
					&& !path.contains("..")
					&& path.bytes().all(|b| {
						b.is_ascii_alphanumeric()
							|| matches!(b, b'/' | b'-' | b'_' | b'.' | b'?' | b'=' | b'&' | b'%')
					})
			})
		})
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BadgeRarity {
	Common,
	Rare,
	Epic,
	Mythic,
}
impl BadgeRarity {
	pub fn parse(value: u8) -> Option<Self> {
		Some(match value {
			1 => Self::Common,
			2 => Self::Rare,
			3 => Self::Epic,
			5 => Self::Mythic,
			_ => return None,
		})
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct BadgeTier {
	pub key: String,
	pub name: String,
	/// Raster icon for grids and progress rows.
	pub icon: Option<String>,
	/// Larger still artwork for the details pane.
	pub art: Option<String>,
	pub rarity: Option<BadgeRarity>,
	pub milestone: String,
	pub threshold: Option<f64>,
	/// Unix seconds when this tier was reached.
	pub obtained_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BadgeProgress {
	pub current: f64,
	pub threshold: f64,
	pub helper: String,
}
impl BadgeProgress {
	pub fn fraction(&self) -> f32 {
		if self.threshold > 0.0 && self.current.is_finite() {
			(self.current / self.threshold).clamp(0.0, 1.0) as f32
		} else {
			0.0
		}
	}
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogBadge {
	pub id: u32,
	pub name: String,
	pub description: String,
	pub owned: bool,
	pub hidden: bool,
	pub earnable: bool,
	pub icon: Option<String>,
	pub art: Option<String>,
	pub rarity: Option<BadgeRarity>,
	/// Lowest first, as the catalog lists them.
	pub tiers: Vec<BadgeTier>,
	pub current_tier: Option<String>,
	pub next_tier: Option<String>,
	pub progress: Option<BadgeProgress>,
	/// Unix seconds when the badge was first unlocked.
	pub obtained_at: Option<i64>,
	/// Short service-written summary such as an account age or subscription date.
	pub info: Option<String>,
}
impl CatalogBadge {
	fn tier(&self, key: Option<&str>) -> Option<&BadgeTier> {
		let key = key?;
		self.tiers.iter().find(|tier| tier.key == key)
	}
	pub fn current(&self) -> Option<&BadgeTier> {
		self.tier(self.current_tier.as_deref())
	}
	pub fn next(&self) -> Option<&BadgeTier> {
		self.tier(self.next_tier.as_deref())
	}
	/// The tier an owned badge shows, or the one an unowned badge leads to.
	pub fn display_tier(&self) -> Option<&BadgeTier> {
		if self.owned {
			self.current()
		} else {
			self.next()
		}
		.or(self.tiers.first())
	}
	pub fn icon(&self) -> Option<&str> {
		self.display_tier()
			.and_then(|tier| tier.icon.as_deref())
			.or(self.icon.as_deref())
	}
	pub fn art(&self) -> Option<&str> {
		self.display_tier()
			.and_then(|tier| tier.art.as_deref().or(tier.icon.as_deref()))
			.or(self.art.as_deref())
			.or(self.icon.as_deref())
	}
	pub fn rarity(&self) -> Option<BadgeRarity> {
		self.display_tier()
			.and_then(|tier| tier.rarity)
			.or(self.rarity)
	}
	pub fn unlocked_at(&self) -> Option<i64> {
		self.current()
			.and_then(|tier| tier.obtained_at)
			.or(self.obtained_at)
	}
	fn bytes(&self) -> usize {
		let text = |value: &Option<String>| value.as_ref().map_or(0, String::capacity);
		size_of::<Self>()
			+ self.name.capacity()
			+ self.description.capacity()
			+ text(&self.icon)
			+ text(&self.art)
			+ text(&self.current_tier)
			+ text(&self.next_tier)
			+ text(&self.info)
			+ self.progress.as_ref().map_or(0, |p| p.helper.capacity())
			+ self.tiers.capacity() * size_of::<BadgeTier>()
			+ self
				.tiers
				.iter()
				.map(|tier| {
					tier.key.capacity()
						+ tier.name.capacity()
						+ tier.milestone.capacity()
						+ text(&tier.icon) + text(&tier.art)
				})
				.sum::<usize>()
	}
	fn valid(&self) -> bool {
		let text = |value: &str| value.len() <= MAX_BADGE_TEXT * 4;
		let url = |value: &Option<String>| value.as_deref().is_none_or(valid_badge_url);
		self.id != 0
			&& text(&self.name)
			&& text(&self.description)
			&& self.info.as_deref().is_none_or(text)
			&& url(&self.icon)
			&& url(&self.art)
			&& self.tiers.len() <= MAX_BADGE_TIERS
			&& self.tiers.iter().all(|tier| {
				text(&tier.key)
					&& text(&tier.name)
					&& text(&tier.milestone)
					&& url(&tier.icon)
					&& url(&tier.art)
			})
	}
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BadgeCatalog {
	/// Display order: pinned first, then the account's chosen order.
	pub badges: Vec<CatalogBadge>,
	pub limited: bool,
}
impl BadgeCatalog {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.badges.capacity() * size_of::<CatalogBadge>()
			+ self
				.badges
				.iter()
				.map(|badge| badge.bytes() - size_of::<CatalogBadge>())
				.sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		self.bytes() <= MAX_CATALOG_BYTES
			&& self.badges.len() <= MAX_CATALOG_BADGES
			&& self.badges.iter().all(CatalogBadge::valid)
			&& self
				.badges
				.iter()
				.enumerate()
				.all(|(index, badge)| !self.badges[..index].iter().any(|b| b.id == badge.id))
	}
	pub fn get(&self, id: u32) -> Option<&CatalogBadge> {
		self.badges.iter().find(|badge| badge.id == id)
	}
	pub fn owned(&self) -> impl Iterator<Item = &CatalogBadge> {
		self.badges.iter().filter(|badge| badge.owned)
	}
	pub fn earnable(&self) -> impl Iterator<Item = &CatalogBadge> {
		self.badges
			.iter()
			.filter(|badge| !badge.owned && badge.earnable)
	}
	/// The account's current settings, as the official client derives them.
	pub fn settings(&self) -> BadgeSettings {
		let mut settings = BadgeSettings::default();
		for badge in self.owned().filter(|badge| !pinned(badge.id)) {
			if badge.hidden {
				settings.hidden.push(badge.id);
			} else {
				settings.order.push(badge.id);
			}
		}
		settings
	}
	/// Applies saved settings locally after the service accepted them.
	pub fn apply(&mut self, settings: &BadgeSettings) {
		for badge in &mut self.badges {
			if badge.owned && hideable(badge.id) {
				badge.hidden = settings.hidden.contains(&badge.id);
			}
		}
		let rank = |badge: &CatalogBadge| {
			if pinned(badge.id) {
				(0, 0)
			} else if let Some(index) = settings.order.iter().position(|id| *id == badge.id) {
				(1, index)
			} else {
				(2, 0)
			}
		};
		// Stable: badges outside the order keep their catalog position.
		self.badges.sort_by_key(rank);
	}
}

/// The account's badge row: visible badges in order and the ones it hides.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BadgeSettings {
	pub order: Vec<u32>,
	pub hidden: Vec<u32>,
}
impl BadgeSettings {
	pub fn valid(&self) -> bool {
		let all: Vec<u32> = self.order.iter().chain(&self.hidden).copied().collect();
		all.len() <= MAX_CATALOG_BADGES
			&& all.iter().all(|id| *id != 0 && !pinned(*id))
			&& self.hidden.iter().all(|id| hideable(*id))
			&& all
				.iter()
				.enumerate()
				.all(|(index, id)| !all[..index].contains(id))
	}
	/// Moves `id` before the badge currently at `to` within the visible order.
	pub fn reorder(&mut self, id: u32, to: usize) {
		let Some(from) = self.order.iter().position(|candidate| *candidate == id) else {
			return;
		};
		let moved = self.order.remove(from);
		self.order.insert(to.min(self.order.len()), moved);
	}
	pub fn set_hidden(&mut self, id: u32, hidden: bool) {
		if !hideable(id) {
			return;
		}
		if hidden {
			self.order.retain(|candidate| *candidate != id);
			if !self.hidden.contains(&id) {
				self.hidden.push(id);
			}
		} else {
			self.hidden.retain(|candidate| *candidate != id);
			if !self.order.contains(&id) {
				self.order.push(id);
			}
		}
	}
}

/// Profile badges to show. Owned visible catalog badges missing from the profile response
/// are added with catalog artwork, then the row follows the catalog order and is capped
/// like the official client's.
pub fn displayed_badges(
	badges: &[ProfileBadge],
	catalog: Option<&BadgeCatalog>,
) -> Vec<ProfileBadge> {
	let Some(catalog) = catalog else {
		return badges.to_vec();
	};
	let mut shown: Vec<ProfileBadge> = badges
		.iter()
		.filter(|badge| {
			badge_number(&badge.id)
				.and_then(|id| catalog.get(id))
				.is_none_or(|entry| !entry.hidden)
		})
		.cloned()
		.collect();
	for entry in catalog.owned().filter(|entry| !entry.hidden) {
		if !shown
			.iter()
			.any(|badge| badge_number(&badge.id) == Some(entry.id))
		{
			shown.push(ProfileBadge {
				id: badge_name(entry.id),
				description: entry.name.clone(),
				icon: None,
				art: entry.icon().map(str::to_owned),
			});
		}
	}
	let rank = |badge: &ProfileBadge| {
		badge_number(&badge.id)
			.and_then(|id| catalog.badges.iter().position(|entry| entry.id == id))
			.unwrap_or(usize::MAX)
	};
	shown.sort_by_key(rank);
	shown.truncate(MAX_DISPLAYED_BADGES);
	shown
}

#[cfg(test)]
mod tests {
	use super::*;

	fn badge(id: u32, owned: bool, hidden: bool) -> CatalogBadge {
		CatalogBadge {
			id,
			name: format!("Badge {id}"),
			description: String::new(),
			owned,
			hidden,
			earnable: !owned,
			icon: Some(format!("https://cdn.discordapp.com/badge/{id}.png")),
			art: None,
			rarity: None,
			tiers: vec![],
			current_tier: None,
			next_tier: None,
			progress: None,
			obtained_at: None,
			info: None,
		}
	}

	#[test]
	fn badge_ids_round_trip_the_client_mapping() {
		assert_eq!(badge_number("account_age"), Some(id::ACCOUNT_AGE));
		assert_eq!(
			badge_number("premium_tenure_12_month_v2"),
			Some(id::PREMIUM_TENURE)
		);
		assert_eq!(badge_number("premium"), Some(id::PREMIUM_TENURE));
		assert_eq!(badge_number("guild_booster_lvl3"), Some(id::GUILD_BOOSTER));
		assert_eq!(badge_number("staff_personal"), Some(id::STAFF));
		assert_eq!(badge_number("orb_profile_badge"), Some(16));
		assert_eq!(badge_number("unknown"), None);
		assert_eq!(badge_name(id::STREAMING), "streaming");
		assert_eq!(badge_name(id::PREMIUM_TENURE), "premium");
		assert_eq!(badge_name(99), "99");
		for id in 1..=22 {
			assert_eq!(badge_number(&badge_name(id)), Some(id), "{id}");
		}
	}

	#[test]
	fn settings_follow_pinning_and_hiding_rules() {
		let mut catalog = BadgeCatalog {
			badges: vec![
				badge(id::STAFF, true, false),
				badge(id::PREMIUM_TENURE, true, false),
				badge(id::ACCOUNT_AGE, true, true),
				badge(id::STREAMING, true, false),
				badge(id::GAME_TIME, false, false),
			],
			limited: false,
		};
		let mut settings = catalog.settings();
		assert_eq!(settings.order, [id::PREMIUM_TENURE, id::STREAMING]);
		assert_eq!(settings.hidden, [id::ACCOUNT_AGE]);
		settings.set_hidden(id::PREMIUM_TENURE, true);
		assert_eq!(settings.hidden, [id::ACCOUNT_AGE]);
		settings.set_hidden(id::ACCOUNT_AGE, false);
		settings.reorder(id::ACCOUNT_AGE, 0);
		assert_eq!(
			settings.order,
			[id::ACCOUNT_AGE, id::PREMIUM_TENURE, id::STREAMING]
		);
		settings.set_hidden(id::STREAMING, true);
		assert!(settings.valid());
		catalog.apply(&settings);
		let ids: Vec<_> = catalog.badges.iter().map(|b| (b.id, b.hidden)).collect();
		assert_eq!(
			ids,
			[
				(id::STAFF, false),
				(id::ACCOUNT_AGE, false),
				(id::PREMIUM_TENURE, false),
				(id::STREAMING, true),
				(id::GAME_TIME, false)
			]
		);
		assert!(
			!BadgeSettings {
				order: vec![id::STAFF],
				hidden: vec![]
			}
			.valid()
		);
		assert!(
			!BadgeSettings {
				order: vec![],
				hidden: vec![id::PREMIUM_TENURE]
			}
			.valid()
		);
		assert!(
			!BadgeSettings {
				order: vec![18, 18],
				hidden: vec![]
			}
			.valid()
		);
	}

	#[test]
	fn displayed_badges_merge_catalog_order_and_hide() {
		let profile = vec![
			ProfileBadge {
				id: "premium".into(),
				description: "Subscriber".into(),
				icon: Some("a".repeat(32)),
				art: None,
			},
			ProfileBadge {
				id: "hypesquad_house_1".into(),
				description: "Bravery".into(),
				icon: Some("b".repeat(32)),
				art: None,
			},
			ProfileBadge {
				id: "quest_completed".into(),
				description: "Quest".into(),
				icon: Some("c".repeat(32)),
				art: None,
			},
		];
		let catalog = BadgeCatalog {
			badges: vec![
				badge(id::ACCOUNT_AGE, true, false),
				badge(id::PREMIUM_TENURE, true, false),
				badge(14, true, true),
				badge(id::STREAMING, false, false),
			],
			limited: false,
		};
		let shown = displayed_badges(&profile, Some(&catalog));
		let ids: Vec<_> = shown.iter().map(|b| b.id.as_str()).collect();
		// Account age comes from the catalog; the hidden quest badge is dropped; badges the
		// catalog does not list stay at the end.
		assert_eq!(ids, ["account_age", "premium", "hypesquad_house_1"]);
		assert!(shown[0].art.is_some() && shown[0].icon.is_none());
		assert_eq!(displayed_badges(&profile, None).len(), 3);
	}

	#[test]
	fn badge_urls_and_catalog_bounds_are_enforced() {
		assert!(valid_badge_url(
			"https://cdn.discordapp.com/badge-icons/abc.png?size=64"
		));
		assert!(!valid_badge_url("https://example.com/a.png"));
		assert!(!valid_badge_url("https://cdn.discordapp.com/../x"));
		assert!(!valid_badge_url("https://cdn.discordapp.com/a b.png"));
		let mut catalog = BadgeCatalog {
			badges: (1..=3).map(|id| badge(id, true, false)).collect(),
			..Default::default()
		};
		assert!(catalog.valid());
		catalog.badges.push(badge(1, true, false));
		assert!(!catalog.valid());
		let progress = BadgeProgress {
			current: 15.0,
			threshold: 10.0,
			helper: String::new(),
		};
		assert_eq!(progress.fraction(), 1.0);
	}
}
