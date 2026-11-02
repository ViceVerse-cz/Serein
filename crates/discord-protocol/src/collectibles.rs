//! Unofficial collectible fields observed on normal-user payloads and the
//! `GET /collectibles-products/{sku}` catalog route; see docs/discord-compatibility.md.
//! A malformed or unknown cosmetic is dropped without affecting the user, row or profile.
use crate::DecodeError;
use model::{
	AvatarDecoration, Cosmetics, EffectAsset, EffectLayer, FrameAnchor, FrameLayer, FrameLayerKind,
	Id, Nameplate, NameplatePalette, ProfileEffect, ProfileFrame, User,
};
use serde::{Deserialize, de::IgnoredAny};

/// Catalog responses for one SKU, including bundled variants.
pub const MAX_PRODUCT_WIRE: usize = 256 * 1024;
const PROFILE_EFFECT: u8 = 1;
const PROFILE_FRAME: u8 = 3;

/// Keeps a value only when it has the expected shape.
#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub enum Lenient<T> {
	Value(T),
	Invalid(IgnoredAny),
}
impl<T> Lenient<T> {
	fn value(this: Option<Self>) -> Option<T> {
		match this? {
			Self::Value(value) => Some(value),
			Self::Invalid(_) => None,
		}
	}
}
/// Boxed so absent cosmetics keep every user-bearing DTO small.
pub type DecorationField = Option<Box<Lenient<DecorationDto>>>;
pub type CollectiblesField = Option<Box<Lenient<CollectiblesDto>>>;

#[derive(Clone, Deserialize)]
pub struct DecorationDto {
	asset: String,
	sku_id: Id,
	#[serde(default)]
	expires_at: Option<u64>,
}
#[derive(Clone, Deserialize)]
pub struct CollectiblesDto {
	#[serde(default)]
	nameplate: Option<Lenient<NameplateDto>>,
}
#[derive(Clone, Deserialize)]
struct NameplateDto {
	asset: String,
	sku_id: Id,
	#[serde(default)]
	label: Option<String>,
	#[serde(default)]
	palette: Option<String>,
	#[serde(default)]
	expires_at: Option<u64>,
}

fn unexpired(expires_at: Option<u64>, now: u64) -> bool {
	expires_at.is_none_or(|expires| expires > now)
}
fn label(value: Option<String>) -> String {
	value
		.unwrap_or_default()
		.chars()
		.filter(|c| !c.is_control())
		.take(model::MAX_COSMETIC_LABEL)
		.collect()
}

fn decoration(dto: DecorationField, now: u64) -> Option<AvatarDecoration> {
	let dto = Lenient::value(dto.map(|dto| *dto))?;
	let decoration = AvatarDecoration {
		asset: dto.asset,
		sku: dto.sku_id,
		expires_at: dto.expires_at,
	};
	(decoration.valid() && decoration.active(now)).then_some(decoration)
}

fn nameplate(dto: CollectiblesField, now: u64) -> Option<Nameplate> {
	let dto = Lenient::value(Lenient::value(dto.map(|dto| *dto))?.nameplate)?;
	let nameplate = Nameplate {
		asset: dto.asset,
		sku: dto.sku_id,
		label: label(dto.label),
		// An unknown palette still shows the artwork, without a tinted background.
		palette: dto
			.palette
			.as_deref()
			.and_then(NameplatePalette::parse)
			.unwrap_or(NameplatePalette::None),
		expires_at: dto.expires_at,
	};
	(nameplate.valid() && unexpired(nameplate.expires_at, now)).then_some(nameplate)
}

/// Equipped user-level cosmetics, or `None` when nothing valid is equipped.
/// Unofficial `display_name_styles`: `{font_id, effect_id, colors}`, as the official web
/// client reads it from user and member objects.
#[derive(Clone, Deserialize)]
pub struct NameStyleDto {
	#[serde(default)]
	font_id: Option<u32>,
	#[serde(default)]
	effect_id: Option<u32>,
	#[serde(default)]
	colors: Option<Vec<i64>>,
}
pub type NameStyleField = Option<Lenient<NameStyleDto>>;

/// A style with at most four valid colours; anything else is dropped.
pub fn name_style(field: NameStyleField) -> Option<model::NameStyle> {
	let dto = Lenient::value(field)?;
	let colors = dto.colors?;
	if colors.is_empty() || colors.len() > model::NameStyle::MAX_COLORS {
		return None;
	}
	let style = model::NameStyle {
		font: dto.font_id.unwrap_or(0),
		effect: model::NameEffect::from_id(dto.effect_id.unwrap_or(1)),
		colors: colors
			.into_iter()
			.map(|color| u32::try_from(color).ok().filter(|c| *c <= 0xff_ffff))
			.collect::<Option<Vec<_>>>()?,
	};
	style.valid().then_some(style)
}

pub fn cosmetics(
	decoration_dto: DecorationField,
	collectibles: CollectiblesField,
	style: NameStyleField,
) -> Option<Box<Cosmetics>> {
	let now = model::unix_now();
	let cosmetics = Cosmetics {
		decoration: decoration(decoration_dto, now),
		nameplate: nameplate(collectibles, now),
		name_style: name_style(style),
	};
	(!cosmetics.is_empty()).then(|| Box::new(cosmetics))
}

/// A server member's own name style replaces the user's in that server.
pub fn apply_member_style(user: &mut User, field: NameStyleField) {
	if let Some(style) = name_style(field) {
		user.cosmetics.get_or_insert_default().name_style = Some(style);
	}
}

/// A server member's own decoration and nameplate replace the user's in that server.
pub fn apply_member(
	user: &mut User,
	decoration_dto: DecorationField,
	collectibles: CollectiblesField,
) {
	let now = model::unix_now();
	let decoration = decoration(decoration_dto, now);
	let nameplate = nameplate(collectibles, now);
	if decoration.is_none() && nameplate.is_none() {
		return;
	}
	let cosmetics = user.cosmetics.get_or_insert_default();
	if decoration.is_some() {
		cosmetics.decoration = decoration;
	}
	if nameplate.is_some() {
		cosmetics.nameplate = nameplate;
	}
}

/// One entry of a profile's `collectibles` list.
#[derive(Deserialize)]
pub(crate) struct EquippedDto {
	sku_id: Id,
	#[serde(rename = "type")]
	kind: u8,
	#[serde(default)]
	expires_at: Option<Lenient<String>>,
}
/// Legacy `profile_effect` metadata; its id is used as the effect SKU.
#[derive(Deserialize)]
pub(crate) struct LegacyEffectDto {
	id: Id,
	#[serde(default)]
	expires_at: Option<u64>,
}

/// Effect and frame SKUs equipped on one profile layer.
#[derive(Default)]
pub(crate) struct Equipped {
	pub effect: Option<Id>,
	pub frame: Option<Id>,
}
impl Equipped {
	pub(crate) fn parse(
		collectibles: Option<Lenient<Vec<Lenient<EquippedDto>>>>,
		legacy: Option<Lenient<LegacyEffectDto>>,
	) -> Self {
		let now = model::unix_now();
		let mut equipped = Self::default();
		for item in Lenient::value(collectibles)
			.unwrap_or_default()
			.into_iter()
			.take(16)
			.filter_map(|item| Lenient::value(Some(item)))
		{
			let expired = Lenient::value(item.expires_at).is_some_and(|at| {
				time::OffsetDateTime::parse(&at, &time::format_description::well_known::Rfc3339)
					.is_ok_and(|at| at.unix_timestamp() <= now as i64)
			});
			if item.sku_id.0 == 0 || expired {
				continue;
			}
			match item.kind {
				PROFILE_EFFECT => equipped.effect = equipped.effect.or(Some(item.sku_id)),
				PROFILE_FRAME => equipped.frame = equipped.frame.or(Some(item.sku_id)),
				_ => {}
			}
		}
		if equipped.effect.is_none()
			&& let Some(effect) = Lenient::value(legacy)
			&& effect.id.0 != 0
			&& unexpired(effect.expires_at, now)
		{
			equipped.effect = Some(effect.id);
		}
		equipped
	}
}

#[derive(Deserialize)]
struct ProductDto {
	sku_id: Id,
	#[serde(default)]
	items: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EffectDto {
	#[serde(rename = "type")]
	kind: u8,
	#[serde(rename = "sku_id")]
	sku_id: Id,
	#[serde(default)]
	title: Option<String>,
	#[serde(default)]
	accessibility_label: Option<String>,
	#[serde(default)]
	reduced_motion_src: Option<String>,
	#[serde(default)]
	static_frame_src: Option<String>,
	effects: Vec<Lenient<EffectLayerDto>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EffectLayerDto {
	src: String,
	#[serde(default, rename = "loop")]
	looping: bool,
	#[serde(default)]
	width: Option<u16>,
	#[serde(default)]
	height: Option<u16>,
	#[serde(default)]
	duration: u32,
	#[serde(default)]
	start: u32,
	#[serde(default)]
	loop_delay: u32,
	#[serde(default)]
	position: Option<PositionDto>,
	#[serde(default)]
	z_index: i32,
	#[serde(default)]
	randomized_sources: Vec<Lenient<SourceDto>>,
}
#[derive(Deserialize)]
struct PositionDto {
	x: i16,
	y: i16,
}
#[derive(Deserialize)]
struct SourceDto {
	src: String,
}
#[derive(Deserialize)]
struct FrameDto {
	#[serde(rename = "type")]
	kind: u8,
	sku_id: Id,
	#[serde(default)]
	label: Option<String>,
	layers: Vec<Lenient<FrameLayerDto>>,
	#[serde(default)]
	inner_width: Option<u32>,
	#[serde(default)]
	overflow_top: u32,
	#[serde(default)]
	overflow_bottom: u32,
	#[serde(default)]
	overflow_horizontal: u32,
}
#[derive(Deserialize)]
struct FrameLayerDto {
	id: Id,
	#[serde(rename = "type")]
	kind: String,
	order: String,
	#[serde(default)]
	anchor: Option<String>,
	#[serde(default)]
	responsive: bool,
}

fn product_item<T: serde::de::DeserializeOwned>(
	bytes: &[u8],
	sku: Id,
	kind: u8,
	matches: impl Fn(&T) -> Option<(u8, Id)>,
) -> Result<T, DecodeError> {
	if bytes.len() > MAX_PRODUCT_WIRE {
		return Err(DecodeError);
	}
	let product: ProductDto = crate::decode(bytes)?;
	if product.sku_id != sku {
		return Err(DecodeError);
	}
	product
		.items
		.into_iter()
		.take(8)
		.filter_map(|item| serde_json::from_value::<T>(item).ok())
		.find(|item| matches(item) == Some((kind, sku)))
		.ok_or(DecodeError)
}

/// The profile effect sold as `sku`. Randomized layer sources are resolved once here, sharing
/// one random index as the official client does.
pub fn decode_effect(bytes: &[u8], sku: Id) -> Result<ProfileEffect, DecodeError> {
	let dto: EffectDto = product_item(bytes, sku, PROFILE_EFFECT, |item: &EffectDto| {
		Some((item.kind, item.sku_id))
	})?;
	let layers: Vec<EffectLayerDto> = dto
		.effects
		.into_iter()
		.filter_map(|layer| Lenient::value(Some(layer)))
		.take(model::MAX_EFFECT_LAYERS)
		.collect();
	let choices = layers
		.iter()
		.map(|layer| layer.randomized_sources.len())
		.filter(|count| *count > 0)
		.min()
		.unwrap_or(0);
	let pick = if choices > 1 {
		(std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.map_or(0, |elapsed| elapsed.subsec_nanos()) as usize)
			% choices
	} else {
		0
	};
	let [canvas_width, canvas_height] = model::EFFECT_CANVAS;
	let mut layers: Vec<EffectLayer> = layers
		.into_iter()
		.filter_map(|layer| {
			let src = match layer.randomized_sources.get(pick) {
				Some(Lenient::Value(source)) => source.src.as_str(),
				_ => layer.src.as_str(),
			};
			Some(EffectLayer {
				asset: EffectAsset::parse(src)?,
				looping: layer.looping,
				position: layer.position.map_or([0, 0], |p| [p.x, p.y]),
				size: [
					layer.width.unwrap_or(canvas_width),
					layer.height.unwrap_or(canvas_height),
				],
				start_ms: layer.start,
				duration_ms: layer.duration,
				loop_delay_ms: layer.loop_delay,
				z: layer.z_index,
			})
		})
		.collect();
	layers.sort_by_key(|layer| layer.z);
	layers.shrink_to_fit();
	let effect = ProfileEffect {
		sku,
		title: label(dto.title),
		label: label(dto.accessibility_label),
		still: dto
			.static_frame_src
			.as_deref()
			.and_then(EffectAsset::parse)
			.or_else(|| {
				dto.reduced_motion_src
					.as_deref()
					.and_then(EffectAsset::parse)
			}),
		layers,
	};
	if !effect.valid() || (effect.layers.is_empty() && effect.still.is_none()) {
		return Err(DecodeError);
	}
	Ok(effect)
}

/// The profile frame sold as `sku`. Unknown layer types, orders or anchors drop that layer.
pub fn decode_frame(bytes: &[u8], sku: Id) -> Result<ProfileFrame, DecodeError> {
	let dto: FrameDto = product_item(bytes, sku, PROFILE_FRAME, |item: &FrameDto| {
		Some((item.kind, item.sku_id))
	})?;
	let layers: Vec<FrameLayer> = dto
		.layers
		.into_iter()
		.filter_map(|layer| Lenient::value(Some(layer)))
		.filter_map(|layer| {
			let kind = match layer.kind.as_str() {
				"staple" => FrameLayerKind::Staple,
				"rail" => FrameLayerKind::Rail,
				"border" => FrameLayerKind::Border,
				_ => return None,
			};
			let front = match layer.order.as_str() {
				"front" => true,
				"back" => false,
				_ => return None,
			};
			let anchor = match layer.anchor.as_deref() {
				Some("top") => FrameAnchor::Top,
				Some("bottom") => FrameAnchor::Bottom,
				Some("center") | None => FrameAnchor::Center,
				Some(_) => return None,
			};
			// A staple needs an edge to pin to.
			(layer.id.0 != 0 && !(kind == FrameLayerKind::Staple && anchor == FrameAnchor::Center))
				.then_some(FrameLayer {
					id: layer.id,
					kind,
					front,
					anchor,
					responsive: layer.responsive,
				})
		})
		.take(model::MAX_FRAME_LAYERS)
		.collect();
	let frame = ProfileFrame {
		sku,
		label: label(dto.label),
		// Defaults observed in the official client when the catalog omits dimensions.
		inner_width: dto.inner_width.filter(|width| *width > 0).unwrap_or(1200),
		overflow_top: dto.overflow_top,
		overflow_bottom: dto.overflow_bottom,
		overflow_horizontal: dto.overflow_horizontal,
		layers,
	};
	if !frame.valid() || frame.layers.is_empty() {
		return Err(DecodeError);
	}
	Ok(frame)
}

#[cfg(test)]
mod tests {
	#[test]
	fn name_styles_parse_bounded_and_members_override_users() {
		let user: crate::UserDto = serde_json::from_value(serde_json::json!({
			"id": "1", "username": "misha",
			"display_name_styles": {"font_id": 3, "effect_id": 2, "colors": [16711680, 255]}
		}))
		.unwrap();
		let mut user = user.into_model();
		let style = user.cosmetics.as_ref().unwrap().name_style.clone().unwrap();
		assert_eq!((style.font, style.effect), (3, model::NameEffect::Gradient));
		assert_eq!(style.colors, [0xff0000, 0x0000ff]);
		apply_member_style(
			&mut user,
			serde_json::from_value(serde_json::json!({"effect_id": 3, "colors": [65280]})).ok(),
		);
		let member = user
			.cosmetics
			.as_ref()
			.unwrap()
			.name_style
			.as_ref()
			.unwrap();
		assert_eq!(
			(member.effect, member.colors.as_slice()),
			(model::NameEffect::Neon, &[0x00ff00][..])
		);
		for bad in [
			serde_json::json!({"effect_id": 2, "colors": []}),
			serde_json::json!({"effect_id": 2, "colors": [16777216]}),
			serde_json::json!({"effect_id": 2, "colors": [1, 2, 3, 4, 5]}),
			serde_json::json!("not a style"),
		] {
			assert!(name_style(serde_json::from_value(bad).ok()).is_none());
		}
	}

	use super::*;
	use serde_json::json;

	const HASH: &str = "149abd61a0e8a52895ef4e06e05dcba352a11fb76089554b3d726cec8bc1c981";

	#[test]
	fn users_keep_valid_cosmetics_and_ignore_malformed_ones() {
		let user: crate::UserDto = crate::decode(
			json!({"id":"7","username":"Synthetic",
				"avatar_decoration_data":{"asset":"a_fed43ab12698df65902ba06727e20c0e","sku_id":"11","expires_at":null},
				"collectibles":{"nameplate":{"asset":"nameplates/nameplates_v3/bonsai/","sku_id":"12","label":"Bonsai","palette":"bubble_gum","expires_at":null}}})
			.to_string()
			.as_bytes(),
		)
		.unwrap();
		let user = user.into_model();
		let cosmetics = user.cosmetics.as_deref().unwrap();
		let decoration = cosmetics.decoration.as_ref().unwrap();
		assert_eq!(
			decoration.key(),
			"decoration-a_fed43ab12698df65902ba06727e20c0e"
		);
		assert!(decoration.animated());
		let nameplate = cosmetics.nameplate.as_ref().unwrap();
		assert_eq!(nameplate.palette, NameplatePalette::BubbleGum);
		assert_eq!(
			nameplate.key(),
			"nameplate-nameplates/nameplates_v3/bonsai/"
		);
		assert_eq!(nameplate.animated_key(), "nameplate-anim-12");
		assert!(user.heap_bytes() > 64);
		for malformed in [
			json!({"avatar_decoration_data":{"asset":"../x","sku_id":"1"}}),
			json!({"avatar_decoration_data":"x","collectibles":[1]}),
			json!({"avatar_decoration_data":{"asset":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sku_id":"1","expires_at":1}}),
			json!({"collectibles":{"nameplate":{"asset":"nameplates/../x/","sku_id":"2"}}}),
			json!({"collectibles":{"nameplate":{"asset":"nameplates/x/","sku_id":"2","expires_at":"soon"}}}),
			json!({"collectibles":null,"avatar_decoration_data":null}),
		] {
			let mut value = json!({"id":"7","username":"Synthetic"});
			value
				.as_object_mut()
				.unwrap()
				.extend(malformed.as_object().unwrap().clone());
			let user: crate::UserDto = crate::decode(value.to_string().as_bytes()).unwrap();
			assert!(user.into_model().cosmetics.is_none(), "{value}");
		}
		let odd: crate::UserDto = crate::decode(
			br#"{"id":"7","username":"S","collectibles":{"nameplate":{"asset":"nameplates/x/","sku_id":"2","palette":"custom_palette"}}}"#,
		)
		.unwrap();
		let odd = odd.into_model();
		assert_eq!(
			odd.cosmetics.unwrap().nameplate.unwrap().palette,
			NameplatePalette::None
		);
	}

	#[test]
	fn member_cosmetics_replace_only_what_they_provide() {
		let mut user: User = crate::UserDto::into_model(
			crate::decode(
				json!({"id":"7","username":"S","avatar_decoration_data":{"asset":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","sku_id":"1"},
					"collectibles":{"nameplate":{"asset":"nameplates/user/","sku_id":"2"}}})
				.to_string()
				.as_bytes(),
			)
			.unwrap(),
		);
		apply_member(
			&mut user,
			serde_json::from_value(
				json!({"asset":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","sku_id":"3"}),
			)
			.ok(),
			None,
		);
		let cosmetics = user.cosmetics.as_deref().unwrap();
		assert_eq!(
			cosmetics.decoration.as_ref().unwrap().asset,
			"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
		);
		assert_eq!(
			cosmetics.nameplate.as_ref().unwrap().asset,
			"nameplates/user/"
		);
		let mut plain: User =
			crate::UserDto::into_model(crate::decode(br#"{"id":"8","username":"P"}"#).unwrap());
		apply_member(&mut plain, None, serde_json::from_value(json!("bad")).ok());
		assert!(plain.cosmetics.is_none());
	}

	#[test]
	fn catalog_products_decode_bounded_effects_and_frames() {
		let effect = json!({"sku_id":"20","type":1,"items":[{"type":1,"sku_id":"20","title":"Synthetic","accessibilityLabel":"Sparks",
		"reducedMotionSrc":format!("https://cdn.discordapp.com/assets/content/{HASH}"),
		"effects":[
			{"src":format!("https://cdn.discordapp.com/assets/content/{HASH}"),"loop":true,"height":880,"width":450,"duration":2000,"start":5146,"loopDelay":4000,"position":{"x":0,"y":0},"zIndex":101,"randomizedSources":[]},
			{"src":format!("https://cdn.discordapp.com/media/v1/collectibles-shop/{HASH}"),"loop":false,"duration":2573,"start":0,"loopDelay":0,"zIndex":100,"randomizedSources":[]},
			{"src":"https://example.com/evil.png","loop":false,"duration":1,"start":0,"loopDelay":0,"zIndex":0},
			{"bad":true}
		]}]});
		let effect = decode_effect(effect.to_string().as_bytes(), Id(20)).unwrap();
		assert_eq!(effect.layers.len(), 2);
		assert_eq!(effect.layers[0].z, 100);
		assert!(effect.layers[0].asset.shop);
		assert_eq!(effect.layers[0].size, model::EFFECT_CANVAS);
		assert_eq!(effect.layers[1].start_ms, 5146);
		assert!(effect.still.is_some());
		assert!(decode_effect(br#"{"sku_id":"20","items":[]}"#, Id(20)).is_err());
		assert!(decode_effect(br#"{"sku_id":"21","items":[]}"#, Id(20)).is_err());
		assert!(decode_effect(&vec![b' '; MAX_PRODUCT_WIRE + 1], Id(20)).is_err());

		let frame = json!({"sku_id":"30","type":3,"items":[{"type":3,"sku_id":"30","label":"Roses",
			"layers":[
				{"id":"31","type":"staple","order":"front","anchor":"top","responsive":false},
				{"id":"32","type":"rail","order":"back","anchor":"center","responsive":true},
				{"id":"33","type":"border","order":"front"},
				{"id":"34","type":"sparkle","order":"front","anchor":"top"},
				{"id":"35","type":"staple","order":"front","anchor":"center"}
			],"inner_width":1200,"overflow_top":304,"overflow_bottom":212,"overflow_horizontal":56}]});
		let frame = decode_frame(frame.to_string().as_bytes(), Id(30)).unwrap();
		assert_eq!(frame.layers.len(), 3);
		assert_eq!(frame.layers[1].kind, FrameLayerKind::Rail);
		assert!(!frame.layers[1].front && frame.layers[1].responsive);
		assert_eq!(frame.layers[2].anchor, FrameAnchor::Center);
		assert_eq!(frame.layers[0].key(Id(30)), "frame-30-31");
		let [top, bottom, side] = frame.overflow(300.0);
		assert_eq!((top, bottom, side), (76.0, 53.0, 14.0));
		assert!(decode_frame(frame_json_without_layers().as_bytes(), Id(30)).is_err());
	}

	fn frame_json_without_layers() -> String {
		json!({"sku_id":"30","items":[{"type":3,"sku_id":"30","layers":[]}]}).to_string()
	}

	#[test]
	fn profile_equipment_prefers_the_collectibles_list() {
		let equipped = Equipped::parse(
			serde_json::from_value(json!([
				{"sku_id":"5","type":1,"expires_at":null},
				{"sku_id":"6","type":3,"expires_at":"2000-01-01T00:00:00Z"},
				{"sku_id":"7","type":3},
				{"type":"bad"}
			]))
			.ok(),
			serde_json::from_value(json!({"id":"9"})).ok(),
		);
		assert_eq!(
			(equipped.effect, equipped.frame),
			(Some(Id(5)), Some(Id(7)))
		);
		let legacy = Equipped::parse(
			None,
			serde_json::from_value(json!({"id":"9","expires_at":null})).ok(),
		);
		assert_eq!(legacy.effect, Some(Id(9)));
		assert!(
			Equipped::parse(serde_json::from_value(json!("x")).ok(), None)
				.frame
				.is_none()
		);
	}
}
