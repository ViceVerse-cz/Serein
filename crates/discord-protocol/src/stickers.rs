use model::{Id, Sticker, StickerPack};
use serde::Deserialize;
pub type Catalog = model::StickerList<{ model::MAX_GUILD_STICKERS }>;
/// Bind optional wire provenance to the authoritative containing guild.
pub fn guild_catalog(
	mut stickers: Vec<Sticker>,
	guild: Id,
) -> Result<Vec<Sticker>, crate::DecodeError> {
	if guild.0 == 0 || !model::valid_stickers(&stickers, model::MAX_GUILD_STICKERS) {
		return Err(crate::DecodeError);
	}
	for sticker in &mut stickers {
		if sticker.pack_id.is_some() || sticker.guild_id.is_some_and(|id| id != guild) {
			return Err(crate::DecodeError);
		}
		sticker.guild_id = Some(guild);
	}
	Ok(stickers)
}

#[derive(Deserialize)]
pub struct GuildStickersUpdate {
	pub guild_id: Id,
	pub stickers: Catalog,
}
pub struct MessageStickers(pub Vec<Sticker>, pub bool);
pub fn preferred<T>(modern: model::Patch<T>, legacy: model::Patch<T>) -> model::Patch<T> {
	if matches!(modern, model::Patch::Absent) {
		legacy
	} else {
		modern
	}
}
pub fn items_patch(
	modern: &model::Patch<MessageStickers>,
	legacy: &model::Patch<MessageStickers>,
) -> model::Patch<Vec<Sticker>> {
	let chosen = if matches!(modern, model::Patch::Absent) {
		legacy
	} else {
		modern
	};
	match chosen {
		model::Patch::Absent => model::Patch::Absent,
		model::Patch::Null => model::Patch::Null,
		model::Patch::Value(s) => model::Patch::Value(s.0.clone()),
	}
}

impl<'de> Deserialize<'de> for MessageStickers {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		let list = model::StickerList::<{ model::MAX_MESSAGE_STICKERS }>::deserialize(d)?.0;
		let present = !list.is_empty();
		Ok(Self(list, present))
	}
}
pub fn sticker(bytes: &[u8]) -> Result<Sticker, crate::DecodeError> {
	let item: Sticker = crate::decode(bytes)?;
	if !item.valid() {
		return Err(crate::DecodeError);
	}
	Ok(item)
}
pub fn sticker_packs(bytes: &[u8]) -> Result<Vec<StickerPack>, crate::DecodeError> {
	#[derive(Deserialize)]
	struct Pack {
		id: Id,
		name: String,
		stickers: Catalog,
	}
	#[derive(Deserialize)]
	struct Packs {
		sticker_packs: Vec<Pack>,
	}
	let packs: Packs = crate::decode(bytes)?;
	if packs.sticker_packs.len() > 128 {
		return Err(crate::DecodeError);
	}
	let mut total = 0;
	packs
		.sticker_packs
		.into_iter()
		.map(|mut p| {
			total += model::sticker_bytes(&p.stickers.0);
			if p.id.0 == 0 || p.name.len() > 256 || total > 1024 * 1024 {
				return Err(crate::DecodeError);
			}
			for sticker in &mut p.stickers.0 {
				if sticker.guild_id.is_some() || sticker.pack_id.is_some_and(|id| id != p.id) {
					return Err(crate::DecodeError);
				}
				sticker.pack_id = Some(p.id);
			}
			Ok(StickerPack {
				id: p.id,
				name: p.name,
				stickers: p.stickers.0,
			})
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn message_stickers_and_patches_preserve_payload_and_absence() {
		let raw=br#"{"id":"1","channel_id":"2","author":{"id":"3","username":"Test"},"sticker_items":[{"id":"4","name":"Wave","format_type":1}]}"#;
		let message = crate::decode::<crate::MessageDto>(raw)
			.unwrap()
			.into_model();
		assert_eq!(message.sticker_items[0].id, Id(4));
		assert!(message.extra_content.sticker_items);

		for (suffix, expected) in [
			("", 0),
			(",\"sticker_items\":null", 1),
			(",\"sticker_items\":[]", 2),
		] {
			let wire = format!("{{\"id\":\"1\",\"channel_id\":\"2\"{suffix}}}");
			let patch = crate::decode::<crate::PatchDto>(wire.as_bytes())
				.unwrap()
				.into_model();
			assert_eq!(
				match patch.sticker_items {
					model::Patch::Absent => 0,
					model::Patch::Null => 1,
					model::Patch::Value(v) => {
						assert!(v.is_empty());
						2
					}
				},
				expected
			);
		}
	}
	#[test]
	fn sticker_catalog_and_message_bounds() {
		let sticker =
			serde_json::json!({"id":"4","name":"Wave","description":null,"format_type":2});
		let good = serde_json::to_vec(&vec![sticker.clone()]).unwrap();
		assert!(crate::decode::<Catalog>(&good).is_ok());
		for values in [
			vec![sticker.clone(); 4],
			vec![serde_json::json!({"id":"4","name":"x".repeat(121),"format_type":1})],
		] {
			assert!(
				crate::decode::<MessageStickers>(&serde_json::to_vec(&values).unwrap()).is_err()
			);
		}
		assert!(
			crate::decode::<Catalog>(&serde_json::to_vec(&vec![sticker; 501]).unwrap()).is_err()
		);
	}
	#[test]
	fn guild_catalog_binds_missing_provenance_and_rejects_foreign_sources() {
		for guild in [None, Some("2")] {
			let wire = serde_json::json!({"sticker_packs":[{"id":"5","name":"Friends","stickers":[{"id":"4","name":"Wave","format_type":1,"guild_id":guild}]}]});
			let packs = sticker_packs(&serde_json::to_vec(&wire).unwrap());
			if guild.is_some() {
				assert!(packs.is_err());
			} else {
				assert_eq!(packs.unwrap()[0].stickers[0].pack_id, Some(Id(5)));
			}
		}
		let sticker = sticker(br#"{"id":"4","name":"Wave","format_type":1}"#).unwrap();
		assert_eq!(
			guild_catalog(vec![sticker.clone()], Id(2)).unwrap()[0].guild_id,
			Some(Id(2))
		);
		let mut wrong = sticker.clone();
		wrong.guild_id = Some(Id(3));
		assert!(guild_catalog(vec![wrong], Id(2)).is_err());
		let mut pack = sticker;
		pack.pack_id = Some(Id(5));
		assert!(guild_catalog(vec![pack], Id(2)).is_err());
		let mut ready:crate::Ready=crate::decode(br#"{"user":{"id":"1","username":"Synthetic"},"session_id":"synthetic","resume_gateway_url":"wss://gateway.discord.gg","guilds":[{"id":"2","stickers":[{"id":"4","name":"Wave","format_type":1}]}]}"#).unwrap();
		assert_eq!(
			ready.navigation().unwrap().0[0].stickers.as_ref().unwrap()[0].guild_id,
			Some(Id(2))
		);
	}
	#[test]
	fn legacy_stickers_fall_back_only_when_modern_field_is_absent() {
		let legacy = serde_json::json!([{"id":"4","name":"Legacy","format_type":1}]);
		let modern = serde_json::json!([{"id":"5","name":"Modern","format_type":2}]);
		for (modern_value, expected) in [
			(None, Some(Id(4))),
			(Some(serde_json::Value::Null), None),
			(Some(serde_json::json!([])), None),
			(Some(modern), Some(Id(5))),
		] {
			let mut wire = serde_json::json!({"id":"1","channel_id":"2","author":{"id":"3","username":"Test"},"stickers":legacy});
			if let Some(value) = modern_value {
				wire["sticker_items"] = value;
			}
			let bytes = serde_json::to_vec(&wire).unwrap();
			let message = crate::decode::<crate::MessageDto>(&bytes)
				.unwrap()
				.into_model();
			assert_eq!(message.sticker_items.first().map(|s| s.id), expected);
			assert!(message.extra_content.stickers);
			let patch = crate::decode::<crate::PatchDto>(&bytes)
				.unwrap()
				.into_model();
			match patch.sticker_items {
				model::Patch::Value(items) => assert_eq!(items.first().map(|s| s.id), expected),
				model::Patch::Null => assert!(expected.is_none()),
				model::Patch::Absent => panic!("legacy field was present"),
			}
		}
		for (suffix, expected) in [("", 0), (",\"stickers\":null", 1), (",\"stickers\":[]", 2)] {
			let wire = format!("{{\"id\":\"1\",\"channel_id\":\"2\"{suffix}}}");
			let patch = crate::decode::<crate::PatchDto>(wire.as_bytes())
				.unwrap()
				.into_model();
			assert_eq!(
				match patch.sticker_items {
					model::Patch::Absent => 0,
					model::Patch::Null => 1,
					model::Patch::Value(v) => {
						assert!(v.is_empty());
						2
					}
				},
				expected
			);
		}
	}
}
