//! Equipped Discord collectibles: avatar decorations, nameplates, profile effects and frames.
//! Every shape here is unofficial normal-client behavior; see docs/discord-compatibility.md.
//! Values are display-only and never persisted.
use crate::{Id, valid_avatar_hash};

/// Longest accepted nameplate asset path, such as `nameplates/nameplates_v3/bonsai/`.
pub const MAX_NAMEPLATE_ASSET: usize = 128;
pub const MAX_COSMETIC_LABEL: usize = 256;
/// Observed effects carry at most five layers; later layers are dropped.
pub const MAX_EFFECT_LAYERS: usize = 8;
/// Observed frames carry at most six layers; later layers are dropped.
pub const MAX_FRAME_LAYERS: usize = 8;
/// Discord paints a decoration this much larger than the avatar it surrounds.
pub const DECORATION_TO_AVATAR: f32 = 1.2;
/// Profile effects are authored on a 450 x 880 canvas scaled to the card width.
pub const EFFECT_CANVAS: [u16; 2] = [450, 880];
/// Effects start after this delay when a profile opens.
pub const EFFECT_INTRO_MS: u64 = 500;
/// Nameplate artwork is right-aligned at row height with this width-to-height ratio.
pub const NAMEPLATE_ASPECT: f32 = 16.0 / 3.0;
/// Responsive frame rails hide on cards at least this wide relative to their height.
pub const FRAME_RAIL_MIN_ASPECT: f32 = 0.625;

fn label_bytes(label: &str) -> bool {
	label.len() <= MAX_COSMETIC_LABEL * 4 && !label.chars().any(char::is_control)
}

/// Seconds since the Unix epoch; collectibles carry their own expiry.
pub fn unix_now() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_secs())
}

fn active(expires_at: Option<u64>, now: u64) -> bool {
	expires_at.is_none_or(|expires| expires > now)
}

/// Which cosmetics this device shows. Each is opt-in: hidden cosmetics are neither painted
/// nor downloaded, and hidden effects and frames skip their catalog read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CosmeticPreferences {
	pub decorations: bool,
	pub nameplates: bool,
	pub effects: bool,
	pub frames: bool,
	/// Nitro display name colours and effects; older preferences keep them off.
	pub name_styles: bool,
}
impl CosmeticPreferences {
	pub const SHOWN: Self = Self {
		decorations: true,
		nameplates: true,
		effects: true,
		frames: true,
		name_styles: true,
	};
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvatarDecoration {
	/// Preset hash; `a_` marks an animated preset.
	pub asset: String,
	pub sku: Id,
	pub expires_at: Option<u64>,
}
impl AvatarDecoration {
	pub fn valid(&self) -> bool {
		valid_avatar_hash(&self.asset)
	}
	pub fn active(&self, now: u64) -> bool {
		active(self.expires_at, now)
	}
	pub fn animated(&self) -> bool {
		self.asset.starts_with("a_")
	}
	pub fn key(&self) -> String {
		format!("decoration-{}", self.asset)
	}
}

/// Named nameplate backgrounds. Colors were observed in the official mobile client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameplatePalette {
	None,
	Crimson,
	Berry,
	Sky,
	Teal,
	Forest,
	BubbleGum,
	Violet,
	Cobalt,
	Clover,
	Lemon,
	White,
	Black,
}
impl NameplatePalette {
	pub fn parse(name: &str) -> Option<Self> {
		Some(match name {
			"none" => Self::None,
			"crimson" => Self::Crimson,
			"berry" => Self::Berry,
			"sky" => Self::Sky,
			"teal" => Self::Teal,
			"forest" => Self::Forest,
			"bubble_gum" => Self::BubbleGum,
			"violet" => Self::Violet,
			"cobalt" => Self::Cobalt,
			"clover" => Self::Clover,
			"lemon" => Self::Lemon,
			"white" => Self::White,
			"black" => Self::Black,
			_ => return None,
		})
	}
	/// Background tint for the dark and light themes; `None` draws no gradient.
	pub fn colors(self) -> Option<[u32; 2]> {
		Some(match self {
			Self::None => return None,
			Self::Crimson => [0x900007, 0xE7040F],
			Self::Berry => [0x893A99, 0xB11FCF],
			Self::Sky => [0x0080B7, 0x56CCFF],
			Self::Teal => [0x086460, 0x7DEED7],
			Self::Forest => [0x2D5401, 0x6AA624],
			Self::BubbleGum => [0xDC3E97, 0xF957B3],
			Self::Violet => [0x730BC8, 0x972FED],
			Self::Cobalt => [0x0131C2, 0x4278FF],
			Self::Clover => [0x047B20, 0x63CD5A],
			Self::Lemon => [0xF6CD12, 0xFED400],
			Self::White => [0xFFFFFF, 0xFFFFFF],
			Self::Black => [0x000000, 0x000000],
		})
	}
}

/// A nameplate asset folder below `/assets/collectibles/`, ending in `/`.
pub fn valid_nameplate_asset(asset: &str) -> bool {
	asset.len() <= MAX_NAMEPLATE_ASSET
		&& asset.starts_with("nameplates/")
		&& asset.ends_with('/')
		&& asset[..asset.len() - 1].split('/').all(|segment| {
			!segment.is_empty()
				&& segment != "."
				&& segment != ".."
				&& segment
					.chars()
					.all(|c| !c.is_control() && !matches!(c, '?' | '#' | '%' | '\\' | '/'))
		})
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nameplate {
	pub asset: String,
	pub sku: Id,
	/// Accessibility description; may be a localization key.
	pub label: String,
	pub palette: NameplatePalette,
	pub expires_at: Option<u64>,
}
impl Nameplate {
	pub fn valid(&self) -> bool {
		self.sku.0 != 0 && valid_nameplate_asset(&self.asset) && label_bytes(&self.label)
	}
	pub fn active(&self, now: u64) -> bool {
		active(self.expires_at, now)
	}
	/// Still artwork shown while the row is idle.
	pub fn key(&self) -> String {
		format!("nameplate-{}", self.asset)
	}
	/// Animated artwork played while the row is hovered.
	pub fn animated_key(&self) -> String {
		format!("nameplate-anim-{}", self.sku)
	}
}

/// A Nitro display name style. Unofficial: `display_name_styles` on user and member objects,
/// as the official web client reads it. Fonts and effects are the client's numeric IDs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameStyle {
	pub font: u32,
	pub effect: NameEffect,
	/// 0xRRGGBB colours, at most [`NameStyle::MAX_COLORS`].
	pub colors: Vec<u32>,
}
impl NameStyle {
	pub const MAX_COLORS: usize = 4;
	pub fn valid(&self) -> bool {
		!self.colors.is_empty()
			&& self.colors.len() <= Self::MAX_COLORS
			&& self.colors.iter().all(|color| *color <= 0xff_ffff)
	}
}
/// The official client's display name effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NameEffect {
	#[default]
	Solid,
	Gradient,
	Neon,
	Toon,
	Pop,
	Glow,
	Prism,
	Gummy,
}
impl NameEffect {
	/// The client's effect ID; unknown and test effects draw as a solid colour.
	pub fn from_id(id: u32) -> Self {
		match id {
			2 => Self::Gradient,
			3 => Self::Neon,
			4 => Self::Toon,
			5 => Self::Pop,
			6 => Self::Glow,
			7 => Self::Prism,
			8 => Self::Gummy,
			_ => Self::Solid,
		}
	}
}

/// Equipped user or server-member cosmetics shown wherever the user appears.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cosmetics {
	pub decoration: Option<AvatarDecoration>,
	pub nameplate: Option<Nameplate>,
	pub name_style: Option<NameStyle>,
}
impl Cosmetics {
	pub fn is_empty(&self) -> bool {
		self.decoration.is_none() && self.nameplate.is_none() && self.name_style.is_none()
	}
	pub fn heap_bytes(&self) -> usize {
		self.decoration.as_ref().map_or(0, |d| d.asset.capacity())
			+ self
				.nameplate
				.as_ref()
				.map_or(0, |n| n.asset.capacity() + n.label.capacity())
			+ self
				.name_style
				.as_ref()
				.map_or(0, |s| s.colors.capacity() * size_of::<u32>())
	}
	pub fn valid(&self) -> bool {
		self.decoration.as_ref().is_none_or(AvatarDecoration::valid)
			&& self.nameplate.as_ref().is_none_or(Nameplate::valid)
	}
}

/// Profile effect artwork addressed by a CDN content hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectAsset {
	/// `/media/v1/collectibles-shop/` instead of `/assets/content/`.
	pub shop: bool,
	/// 64 lowercase hex characters.
	pub hash: String,
}
impl EffectAsset {
	pub fn parse(url: &str) -> Option<Self> {
		let path = url.strip_prefix("https://cdn.discordapp.com/")?;
		let (shop, hash) = if let Some(hash) = path.strip_prefix("assets/content/") {
			(false, hash)
		} else {
			(true, path.strip_prefix("media/v1/collectibles-shop/")?)
		};
		let hash = hash.split('?').next().unwrap_or(hash);
		(hash.len() == 64
			&& hash
				.bytes()
				.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
		.then(|| Self {
			shop,
			hash: hash.to_owned(),
		})
	}
	pub fn valid(&self) -> bool {
		self.hash.len() == 64
			&& self
				.hash
				.bytes()
				.all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
	}
	fn tag(&self) -> char {
		if self.shop { 's' } else { 'c' }
	}
	/// Animated playback of an effect layer.
	pub fn key(&self) -> String {
		format!("effect-{}-{}", self.tag(), self.hash)
	}
	/// First frame only, for reduced motion.
	pub fn still_key(&self) -> String {
		format!("effect-still-{}-{}", self.tag(), self.hash)
	}
}

/// One timed layer on the 450 x 880 effect canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectLayer {
	pub asset: EffectAsset,
	pub looping: bool,
	pub position: [i16; 2],
	pub size: [u16; 2],
	pub start_ms: u32,
	pub duration_ms: u32,
	pub loop_delay_ms: u32,
	pub z: i32,
}
impl EffectLayer {
	/// Layer-local playback time, or `None` while the layer is hidden. Mirrors the official
	/// client: a one-shot layer shows from `start` for `duration`; a looping layer repeats
	/// `duration` then hides for `loop_delay`.
	pub fn local_ms(&self, timeline_ms: u64) -> Option<u64> {
		let start = u64::from(self.start_ms);
		let duration = u64::from(self.duration_ms);
		let elapsed = timeline_ms.checked_sub(start)?;
		if !self.looping {
			return (elapsed <= duration).then_some(elapsed);
		}
		let period = duration + u64::from(self.loop_delay_ms);
		if period == 0 {
			return Some(0);
		}
		let local = elapsed % period;
		(self.loop_delay_ms == 0 || local <= duration).then_some(local)
	}
	/// Next timeline instant at which this layer becomes visible again, if it ever does.
	pub fn next_visible_ms(&self, timeline_ms: u64) -> Option<u64> {
		if self.local_ms(timeline_ms).is_some() {
			return Some(timeline_ms);
		}
		let start = u64::from(self.start_ms);
		if timeline_ms < start {
			return Some(start);
		}
		if !self.looping {
			return None;
		}
		let period = u64::from(self.duration_ms) + u64::from(self.loop_delay_ms);
		Some(timeline_ms - (timeline_ms - start) % period + period)
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileEffect {
	pub sku: Id,
	pub title: String,
	pub label: String,
	/// Static frame used when motion is reduced, then the reduced-motion artwork.
	pub still: Option<EffectAsset>,
	/// Sorted by z-index, lowest first.
	pub layers: Vec<EffectLayer>,
}
impl ProfileEffect {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.title.capacity()
			+ self.label.capacity()
			+ self.still.as_ref().map_or(0, |s| s.hash.capacity())
			+ self.layers.capacity() * size_of::<EffectLayer>()
			+ self
				.layers
				.iter()
				.map(|l| l.asset.hash.capacity())
				.sum::<usize>()
	}
	pub fn valid(&self) -> bool {
		self.sku.0 != 0
			&& label_bytes(&self.title)
			&& label_bytes(&self.label)
			&& self.still.as_ref().is_none_or(EffectAsset::valid)
			&& self.layers.len() <= MAX_EFFECT_LAYERS
			&& self.layers.iter().all(|layer| {
				layer.asset.valid()
					&& layer.size[0] > 0
					&& layer.size[1] > 0
					&& layer.size[0] <= 4096
					&& layer.size[1] <= 4096
					&& layer.duration_ms <= 120_000
					&& layer.start_ms <= 120_000
					&& layer.loop_delay_ms <= 120_000
			}) && self.layers.windows(2).all(|pair| pair[0].z <= pair[1].z)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameLayerKind {
	/// Pinned to the top or bottom edge, overflowing outward.
	Staple,
	/// Spans the card height, aligned by its anchor.
	Rail,
	/// Tiled down the full card height.
	Border,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameAnchor {
	Top,
	Bottom,
	Center,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameLayer {
	pub id: Id,
	pub kind: FrameLayerKind,
	/// Front layers cover the card; back layers sit behind it.
	pub front: bool,
	pub anchor: FrameAnchor,
	pub responsive: bool,
}
impl FrameLayer {
	pub fn key(&self, sku: Id) -> String {
		format!("frame-{sku}-{}", self.id)
	}
}

/// Decoration around a profile card, authored for a card `inner_width` units wide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileFrame {
	pub sku: Id,
	pub label: String,
	pub inner_width: u32,
	pub overflow_top: u32,
	pub overflow_bottom: u32,
	pub overflow_horizontal: u32,
	pub layers: Vec<FrameLayer>,
}
impl ProfileFrame {
	pub fn bytes(&self) -> usize {
		size_of::<Self>() + self.label.capacity() + self.layers.capacity() * size_of::<FrameLayer>()
	}
	pub fn valid(&self) -> bool {
		self.sku.0 != 0
			&& label_bytes(&self.label)
			&& (1..=8192).contains(&self.inner_width)
			&& [
				self.overflow_top,
				self.overflow_bottom,
				self.overflow_horizontal,
			]
			.iter()
			.all(|overflow| *overflow <= self.inner_width)
			&& self.layers.len() <= MAX_FRAME_LAYERS
			&& self.layers.iter().all(|layer| layer.id.0 != 0)
	}
	/// Overflow beyond a card `width` points wide: top, bottom and each side.
	pub fn overflow(&self, width: f32) -> [f32; 3] {
		let scale = width / self.inner_width.max(1) as f32;
		[
			self.overflow_top as f32 * scale,
			self.overflow_bottom as f32 * scale,
			self.overflow_horizontal as f32 * scale,
		]
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn layer(start: u32, duration: u32, looping: bool, delay: u32) -> EffectLayer {
		EffectLayer {
			asset: EffectAsset {
				shop: false,
				hash: "a".repeat(64),
			},
			looping,
			position: [0, 0],
			size: EFFECT_CANVAS,
			start_ms: start,
			duration_ms: duration,
			loop_delay_ms: delay,
			z: 0,
		}
	}

	#[test]
	fn effect_layers_follow_official_start_duration_and_loop_delay() {
		let intro = layer(0, 2573, false, 0);
		assert_eq!(intro.local_ms(0), Some(0));
		assert_eq!(intro.local_ms(2573), Some(2573));
		assert_eq!(intro.local_ms(2574), None);
		assert_eq!(intro.next_visible_ms(3000), None);
		let looped = layer(5146, 2000, true, 4000);
		assert_eq!(looped.local_ms(5000), None);
		assert_eq!(looped.next_visible_ms(5000), Some(5146));
		assert_eq!(looped.local_ms(6146), Some(1000));
		assert_eq!(looped.local_ms(8146), None);
		assert_eq!(looped.next_visible_ms(8146), Some(11146));
		assert_eq!(looped.local_ms(11146), Some(0));
		let forever = layer(0, 1000, true, 0);
		assert_eq!(forever.local_ms(2500), Some(500));
		assert_eq!(layer(0, 0, true, 0).local_ms(10), Some(0));
	}

	#[test]
	fn cosmetic_assets_are_bounded_and_confined() {
		assert!(valid_nameplate_asset("nameplates/nameplates_v3/bonsai/"));
		assert!(valid_nameplate_asset(
			"nameplates/buffering.../1541529803809755246/"
		));
		for bad in [
			"nameplates/../avatars/",
			"nameplates/x",
			"collectibles/x/",
			"nameplates//x/",
			"nameplates/x?y/",
			"nameplates/%2e%2e/",
		] {
			assert!(!valid_nameplate_asset(bad), "{bad}");
		}
		assert!(!valid_nameplate_asset(&format!(
			"nameplates/{}/",
			"a".repeat(MAX_NAMEPLATE_ASSET)
		)));
		let hash = "0a1c28bead3953cc15588c2655c4aa592b9ffafc1470c20138672f307000f01f";
		let content =
			EffectAsset::parse(&format!("https://cdn.discordapp.com/assets/content/{hash}"))
				.unwrap();
		assert_eq!(content.key(), format!("effect-c-{hash}"));
		let shop = EffectAsset::parse(&format!(
			"https://cdn.discordapp.com/media/v1/collectibles-shop/{hash}"
		))
		.unwrap();
		assert_eq!(shop.still_key(), format!("effect-still-s-{hash}"));
		for bad in [
			format!("https://media.discordapp.net/assets/content/{hash}"),
			format!("https://cdn.discordapp.com/avatars/{hash}"),
			"https://cdn.discordapp.com/assets/content/../x".into(),
			format!("http://cdn.discordapp.com/assets/content/{hash}"),
		] {
			assert!(EffectAsset::parse(&bad).is_none(), "{bad}");
		}
		assert_eq!(
			NameplatePalette::parse("bubble_gum").unwrap().colors(),
			Some([0xDC3E97, 0xF957B3])
		);
		assert_eq!(NameplatePalette::None.colors(), None);
		assert!(NameplatePalette::parse("custom_palette").is_none());
		let decoration = AvatarDecoration {
			asset: "a_fed43ab12698df65902ba06727e20c0e".into(),
			sku: Id(1),
			expires_at: Some(100),
		};
		assert!(decoration.valid() && decoration.animated());
		assert!(decoration.active(99) && !decoration.active(100));
	}
}
