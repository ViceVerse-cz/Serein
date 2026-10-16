//! Unofficial badge catalog (`GET /users/{id}/badges`) and badge settings
//! (`PATCH /users/@me/badges/settings`), as observed in the official client. One malformed
//! badge is dropped without rejecting the catalog.
use crate::{DecodeError, collectibles::Lenient};
use model::badges::{
	BadgeCatalog, BadgeProgress, BadgeRarity, BadgeSettings, BadgeTier, CatalogBadge,
	MAX_BADGE_TEXT, MAX_BADGE_TIERS, MAX_CATALOG_BADGES, MAX_CATALOG_BYTES, valid_badge_url,
};
use serde::Deserialize;
use std::collections::HashMap;

pub const MAX_CATALOG_WIRE: usize = 256 * 1024;
/// Tiers read per badge before the retained window is chosen.
const MAX_WIRE_TIERS: usize = 64;

#[derive(Deserialize)]
struct CatalogDto {
	badges: Vec<Lenient<BadgeDto>>,
}
#[derive(Deserialize)]
struct BadgeDto {
	badge_id: u32,
	#[serde(default)]
	name: Option<String>,
	#[serde(default)]
	description: Option<String>,
	#[serde(default)]
	owned: Option<bool>,
	#[serde(default)]
	hidden: Option<bool>,
	#[serde(default)]
	is_earnable: Option<bool>,
	#[serde(flatten)]
	art: ArtDto,
	#[serde(default)]
	rarity: Option<Lenient<u8>>,
	#[serde(default)]
	tiers: Option<Vec<Lenient<TierDto>>>,
	#[serde(default)]
	current_tier: Option<Lenient<String>>,
	#[serde(default)]
	next_tier: Option<Lenient<String>>,
	#[serde(default)]
	progress: Option<Vec<Lenient<ProgressDto>>>,
	#[serde(default)]
	obtained_at: Option<Lenient<Instant>>,
	#[serde(default)]
	tier_obtained_at: Option<Lenient<HashMap<String, Lenient<Instant>>>>,
	#[serde(default)]
	info_label: Option<Lenient<String>>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct ArtDto {
	simple_icon_url: Option<Lenient<String>>,
	simple_icon_raster_url: Option<Lenient<String>>,
	complex_icon_static_url: Option<Lenient<String>>,
}
impl ArtDto {
	fn url(value: Option<Lenient<String>>) -> Option<String> {
		match value? {
			Lenient::Value(url) if valid_badge_url(&url) => Some(url),
			_ => None,
		}
	}
	/// Raster icon and larger still art; SVG icons are skipped because the image worker
	/// does not decode service SVGs.
	fn split(self) -> (Option<String>, Option<String>) {
		let simple = Self::url(self.simple_icon_url).filter(|url| !is_svg(url));
		let raster = Self::url(self.simple_icon_raster_url);
		let complex = Self::url(self.complex_icon_static_url).filter(|url| !is_svg(url));
		(raster.or(simple).or(complex.clone()), complex)
	}
}
fn is_svg(url: &str) -> bool {
	url.split(['?', '#'])
		.next()
		.is_some_and(|path| path.to_ascii_lowercase().ends_with(".svg"))
}
#[derive(Deserialize)]
struct TierDto {
	key: String,
	#[serde(default)]
	name: Option<String>,
	#[serde(flatten)]
	art: ArtDto,
	#[serde(default)]
	rarity: Option<Lenient<u8>>,
	#[serde(default)]
	milestone_text: Option<Lenient<String>>,
	#[serde(default)]
	requirements: Option<Vec<Lenient<RequirementDto>>>,
}
#[derive(Deserialize)]
struct RequirementDto {
	threshold: f64,
}
#[derive(Deserialize)]
struct ProgressDto {
	#[serde(default)]
	current: f64,
	threshold: f64,
	#[serde(default)]
	progress_helper_text: Option<Lenient<String>>,
}
/// ISO-8601 text or a Unix timestamp in seconds or milliseconds.
#[derive(Deserialize)]
#[serde(untagged)]
enum Instant {
	Text(String),
	Number(f64),
}
impl Instant {
	fn seconds(this: Option<Lenient<Self>>) -> Option<i64> {
		match this? {
			Lenient::Value(Self::Text(text)) if text.len() <= 64 => {
				time::OffsetDateTime::parse(&text, &time::format_description::well_known::Rfc3339)
					.ok()
					.map(|at| at.unix_timestamp())
			}
			Lenient::Value(Self::Number(value)) if value.is_finite() && value > 0.0 => {
				Some(if value > 1e11 { value / 1000.0 } else { value } as i64)
			}
			_ => None,
		}
	}
}

fn text(value: Option<String>) -> String {
	value
		.unwrap_or_default()
		.chars()
		.filter(|c| !c.is_control())
		.take(MAX_BADGE_TEXT)
		.collect()
}
fn lenient_text(value: Option<Lenient<String>>) -> Option<String> {
	match value? {
		Lenient::Value(value) => Some(text(Some(value))).filter(|value| !value.is_empty()),
		Lenient::Invalid(_) => None,
	}
}
fn rarity(value: Option<Lenient<u8>>) -> Option<BadgeRarity> {
	match value? {
		Lenient::Value(value) => BadgeRarity::parse(value),
		Lenient::Invalid(_) => None,
	}
}
fn values<T>(list: Option<Vec<Lenient<T>>>) -> impl Iterator<Item = T> {
	list.unwrap_or_default()
		.into_iter()
		.filter_map(|item| match item {
			Lenient::Value(value) => Some(value),
			Lenient::Invalid(_) => None,
		})
}

fn badge(dto: BadgeDto) -> Option<CatalogBadge> {
	let tier_dates: HashMap<String, Lenient<Instant>> = match dto.tier_obtained_at {
		Some(Lenient::Value(map)) if map.len() <= 64 => map,
		_ => HashMap::new(),
	};
	let mut tier_dates = tier_dates;
	let mut tiers: Vec<BadgeTier> = values(dto.tiers)
		.take(MAX_WIRE_TIERS)
		.filter(|tier| !tier.key.is_empty() && tier.key.len() <= 64)
		.map(|tier| {
			let (icon, art) = tier.art.split();
			let obtained_at = Instant::seconds(tier_dates.remove(&tier.key));
			BadgeTier {
				name: text(tier.name),
				icon,
				art,
				rarity: rarity(tier.rarity),
				milestone: lenient_text(tier.milestone_text).unwrap_or_default(),
				threshold: values(tier.requirements)
					.map(|requirement| requirement.threshold)
					.find(|threshold| threshold.is_finite()),
				obtained_at,
				key: tier.key,
			}
		})
		.collect();
	let tier_key = |value: Option<Lenient<String>>| {
		lenient_text(value).filter(|key| tiers.iter().any(|tier| tier.key == *key))
	};
	let current_tier = tier_key(dto.current_tier);
	let next_tier = tier_key(dto.next_tier);
	if tiers.len() > MAX_BADGE_TIERS {
		// Keep a window that still holds the current and next tiers.
		let index = |key: &Option<String>| {
			key.as_ref()
				.and_then(|key| tiers.iter().position(|tier| tier.key == *key))
		};
		let anchor = index(&next_tier).or(index(&current_tier)).unwrap_or(0);
		let start = (anchor + 2)
			.saturating_sub(MAX_BADGE_TIERS)
			.min(tiers.len() - MAX_BADGE_TIERS);
		tiers.truncate(start + MAX_BADGE_TIERS);
		tiers.drain(..start);
	}
	tiers.shrink_to_fit();
	let (icon, art) = dto.art.split();
	let progress = values(dto.progress).next().and_then(|progress| {
		(progress.threshold.is_finite() && progress.current.is_finite()).then(|| BadgeProgress {
			current: progress.current,
			threshold: progress.threshold,
			helper: lenient_text(progress.progress_helper_text).unwrap_or_default(),
		})
	});
	let badge = CatalogBadge {
		id: dto.badge_id,
		name: text(dto.name),
		description: text(dto.description),
		owned: dto.owned.unwrap_or(false),
		hidden: dto.hidden.unwrap_or(false),
		earnable: dto.is_earnable.unwrap_or(false),
		icon,
		art,
		rarity: rarity(dto.rarity),
		tiers,
		current_tier,
		next_tier,
		progress,
		obtained_at: Instant::seconds(dto.obtained_at),
		info: lenient_text(dto.info_label),
	};
	(badge.id != 0 && (badge.owned || badge.earnable)).then_some(badge)
}

pub fn decode_catalog(bytes: &[u8]) -> Result<BadgeCatalog, DecodeError> {
	if bytes.len() > MAX_CATALOG_WIRE {
		return Err(DecodeError);
	}
	let dto: CatalogDto = crate::decode(bytes)?;
	let mut catalog = BadgeCatalog::default();
	for item in dto.badges {
		let Lenient::Value(item) = item else {
			continue;
		};
		let Some(badge) = badge(item) else {
			continue;
		};
		if catalog
			.badges
			.iter()
			.any(|existing| existing.id == badge.id)
		{
			continue;
		}
		if catalog.badges.len() == MAX_CATALOG_BADGES {
			catalog.limited = true;
			break;
		}
		catalog.badges.push(badge);
	}
	// Stay within budget while keeping every owned badge's displayed tier: drop the other
	// tiers of locked badges, then tier art, then owned badges' other tiers, then locked
	// badges, and owned badges only last. Tiered badges sit at the end of the catalog.
	if catalog.bytes() > MAX_CATALOG_BYTES {
		catalog.limited = true;
		for badge in catalog.badges.iter_mut().filter(|badge| !badge.owned) {
			keep_displayed_tiers(badge);
		}
	}
	if catalog.bytes() > MAX_CATALOG_BYTES {
		for tier in catalog.badges.iter_mut().flat_map(|badge| &mut badge.tiers) {
			tier.art = None;
		}
	}
	if catalog.bytes() > MAX_CATALOG_BYTES {
		catalog.badges.iter_mut().for_each(keep_displayed_tiers);
	}
	while catalog.bytes() > MAX_CATALOG_BYTES && !catalog.badges.is_empty() {
		let last = catalog.badges.len() - 1;
		let index = catalog
			.badges
			.iter()
			.rposition(|badge| !badge.owned)
			.unwrap_or(last);
		catalog.badges.remove(index);
	}
	catalog.badges.shrink_to_fit();
	if !catalog.valid() {
		return Err(DecodeError);
	}
	Ok(catalog)
}

/// Keeps only the current and next tiers, or the first when the badge names neither.
fn keep_displayed_tiers(badge: &mut CatalogBadge) {
	let keys = [&badge.current_tier, &badge.next_tier];
	if keys.iter().any(|key| key.is_some()) {
		badge.tiers.retain(|tier| {
			keys.iter()
				.any(|key| key.as_deref() == Some(tier.key.as_str()))
		});
	} else {
		badge.tiers.truncate(1);
	}
	badge.tiers.shrink_to_fit();
}

/// Body for `PATCH /users/@me/badges/settings`.
pub fn encode_settings(settings: &BadgeSettings) -> Result<serde_json::Value, DecodeError> {
	if !settings.valid() {
		return Err(DecodeError);
	}
	Ok(serde_json::json!({
		"display_order": settings.order,
		"hidden_badges": settings.hidden,
	}))
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::badges::id;
	use serde_json::json;

	const ICON: &str = "https://cdn.discordapp.com/assets/badges/account-age.png";

	#[test]
	fn catalog_keeps_tiers_progress_and_dates_and_drops_malformed_badges() {
		let body = json!({"badges": [
			{"badge_id": 18, "name": "Account Age", "description": "Years on Discord", "owned": true,
			 "hidden": false, "is_earnable": true, "info_label": "8 years, 1 month", "rarity": 3,
			 "simple_icon_url": "https://cdn.discordapp.com/assets/badges/account-age.svg",
			 "simple_icon_raster_url": ICON,
			 "current_tier": "bristlecone", "next_tier": "stromatolite",
			 "obtained_at": "2018-08-30T00:00:00Z",
			 "tier_obtained_at": {"bristlecone": "2026-08-30T00:00:00Z"},
			 "progress": [{"current": 8.1, "threshold": 9, "progress_helper_text": "11 months to go"}],
			 "tiers": [
				{"key": "bristlecone", "name": "Bristlecone", "rarity": 3, "simple_icon_raster_url": ICON,
				 "complex_icon_static_url": "https://cdn.discordapp.com/assets/badges/bristlecone.png",
				 "milestone_text": "8 years", "requirements": [{"threshold": 8}]},
				{"key": "stromatolite", "name": "Stromatolite", "requirements": [{"threshold": 9}]},
				{"name": "missing key"}
			 ]},
			{"badge_id": 19, "name": "Streaming", "owned": false, "is_earnable": true},
			{"badge_id": 2, "name": "Partner", "owned": false, "is_earnable": false},
			{"badge_id": "x"},
			{"badge_id": 18, "name": "Duplicate", "owned": true},
			{"badge_id": 1, "name": "Nitro", "owned": true, "obtained_at": 1_776_556_800_000_u64,
			 "simple_icon_raster_url": "https://example.com/evil.png"}
		]});
		let catalog = decode_catalog(body.to_string().as_bytes()).unwrap();
		let ids: Vec<_> = catalog.badges.iter().map(|b| b.id).collect();
		// Unowned, unearnable legacy badges are not part of the directory.
		assert_eq!(ids, [id::ACCOUNT_AGE, id::STREAMING, id::PREMIUM_TENURE]);
		let age = &catalog.badges[0];
		assert_eq!(age.info.as_deref(), Some("8 years, 1 month"));
		assert_eq!(age.icon(), Some(ICON));
		assert_eq!(age.current().unwrap().name, "Bristlecone");
		assert_eq!(age.next().unwrap().threshold, Some(9.0));
		assert_eq!(age.tiers.len(), 2);
		assert_eq!(age.rarity(), Some(BadgeRarity::Epic));
		assert_eq!(age.unlocked_at(), Some(1_788_048_000));
		assert!((age.progress.as_ref().unwrap().fraction() - 0.9).abs() < 0.01);
		let nitro = &catalog.badges[2];
		assert_eq!(nitro.icon, None);
		assert_eq!(nitro.obtained_at, Some(1_776_556_800));
		assert!(catalog.valid());
		assert!(decode_catalog(br#"{"badges":{}}"#).is_err());
		assert!(decode_catalog(&vec![b' '; MAX_CATALOG_WIRE + 1]).is_err());
	}

	#[test]
	fn oversized_catalogs_shed_detail_before_badges() {
		let tier = |n: usize| {
			json!({"key": format!("t{n}"), "name": "N".repeat(150),
			"milestone_text": "M".repeat(150),
			"simple_icon_raster_url": format!("https://cdn.discordapp.com/{}.png", "a".repeat(150)),
			"complex_icon_static_url": format!("https://cdn.discordapp.com/{}.png", "b".repeat(150))})
		};
		let badges: Vec<_> = (1..=24)
			.map(|id| {
				json!({"badge_id": id, "name": "B", "owned": id % 2 == 0, "is_earnable": true,
				"current_tier": "t5", "next_tier": "t6",
				"tiers": (0..12).map(tier).collect::<Vec<_>>()})
			})
			.collect();
		let catalog = decode_catalog(json!({ "badges": badges }).to_string().as_bytes()).unwrap();
		assert!(catalog.limited && catalog.valid());
		// Every owned badge survives, including the tiered ones at the end of the catalog,
		// and keeps the tier it displays.
		let owned: Vec<_> = catalog.owned().map(|b| b.id).collect();
		assert_eq!(owned, (1..=12).map(|n| n * 2).collect::<Vec<_>>());
		assert!(catalog.owned().all(|b| b.current().is_some()));
		assert!(
			catalog
				.badges
				.iter()
				.filter(|b| !b.owned)
				.all(|b| b.tiers.len() <= 2)
		);
	}

	#[test]
	fn long_tier_lists_keep_the_current_and_next_tiers() {
		let tiers: Vec<_> = (0..20)
			.map(|n| json!({"key": format!("t{n}"), "name": format!("Tier {n}")}))
			.collect();
		let body = json!({"badges": [{"badge_id": 18, "owned": true, "current_tier": "t16",
			"next_tier": "t17", "tiers": tiers}]});
		let catalog = decode_catalog(body.to_string().as_bytes()).unwrap();
		let age = &catalog.badges[0];
		assert_eq!(age.tiers.len(), MAX_BADGE_TIERS);
		assert_eq!(age.tiers[0].key, "t7");
		assert_eq!(age.current().unwrap().name, "Tier 16");
		assert_eq!(age.next().unwrap().name, "Tier 17");
	}

	#[test]
	fn settings_body_uses_numeric_ids_and_rejects_pinned_changes() {
		let settings = BadgeSettings {
			order: vec![id::PREMIUM_TENURE, id::ACCOUNT_AGE],
			hidden: vec![id::STREAMING],
		};
		assert_eq!(
			encode_settings(&settings).unwrap(),
			json!({"display_order": [1, 18], "hidden_badges": [19]})
		);
		assert!(
			encode_settings(&BadgeSettings {
				order: vec![],
				hidden: vec![id::STAFF]
			})
			.is_err()
		);
	}
}
