use crate::Id;
use serde::{Deserialize, Serialize};
pub const MAX_MESSAGE_STICKERS: usize = 3;
pub const MAX_GUILD_STICKERS: usize = 500;
pub const MAX_STICKER_BYTES: usize = 512 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sticker {
	pub id: Id,
	#[serde(deserialize_with = "text::<_, 120>")]
	pub name: String,
	#[serde(default, deserialize_with = "description")]
	pub description: String,
	#[serde(default, deserialize_with = "text::<_, 1024>")]
	pub tags: String,
	pub format_type: u8,
	#[serde(default)]
	pub guild_id: Option<Id>,
	#[serde(default)]
	pub pack_id: Option<Id>,
	#[serde(default = "available")]
	pub available: bool,
}
fn available() -> bool {
	true
}
fn text<'de, D: serde::Deserializer<'de>, const MAX: usize>(d: D) -> Result<String, D::Error> {
	struct Text<const MAX: usize>;
	impl<'de, const MAX: usize> serde::de::Visitor<'de> for Text<MAX> {
		type Value = String;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("bounded sticker text")
		}
		fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<String, E> {
			if value.len() > MAX {
				return Err(E::custom("Sticker text capacity"));
			}
			Ok(value.to_owned())
		}
	}
	d.deserialize_str(Text::<MAX>)
}
fn description<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
	struct Description;
	impl<'de> serde::de::Visitor<'de> for Description {
		type Value = String;
		fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
			f.write_str("optional sticker description")
		}
		fn visit_none<E: serde::de::Error>(self) -> Result<String, E> {
			Ok(String::new())
		}
		fn visit_unit<E: serde::de::Error>(self) -> Result<String, E> {
			Ok(String::new())
		}
		fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<String, D::Error> {
			text::<D, 4096>(d)
		}
	}
	d.deserialize_option(Description)
}
impl Sticker {
	pub fn heap_bytes(&self) -> usize {
		self.name.capacity() + self.description.capacity() + self.tags.capacity()
	}
	pub fn valid(&self) -> bool {
		self.id.0 != 0
			&& !self.name.is_empty()
			&& self.name.len() <= 120
			&& self.description.len() <= 4096
			&& self.tags.len() <= 1024
			&& self.guild_id.is_none_or(|id| id.0 != 0)
			&& self.pack_id.is_none_or(|id| id.0 != 0)
	}
}
pub fn sticker_bytes(items: &Vec<Sticker>) -> usize {
	items.capacity() * std::mem::size_of::<Sticker>()
		+ items.iter().map(Sticker::heap_bytes).sum::<usize>()
}
pub fn valid_stickers(items: &Vec<Sticker>, max: usize) -> bool {
	items.len() <= max
		&& sticker_bytes(items) <= MAX_STICKER_BYTES
		&& items
			.iter()
			.enumerate()
			.all(|(i, s)| s.valid() && !items[..i].iter().any(|other| other.id == s.id))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StickerPack {
	pub id: Id,
	pub name: String,
	pub stickers: Vec<Sticker>,
}
/// Bounded catalog and persistent message decoder.
pub struct StickerList<const MAX: usize>(pub Vec<Sticker>);
impl<'de, const MAX: usize> Deserialize<'de> for StickerList<MAX> {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		struct List<const N: usize>;
		impl<'de, const N: usize> serde::de::Visitor<'de> for List<N> {
			type Value = StickerList<N>;
			fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
				f.write_str("a bounded sticker list")
			}
			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut seq: A,
			) -> Result<Self::Value, A::Error> {
				let mut items = Vec::new();
				while let Some(item) = seq.next_element::<Sticker>()? {
					if items.len() >= N
						|| !item.valid() || items.iter().any(|s: &Sticker| s.id == item.id)
					{
						return Err(serde::de::Error::custom("Invalid stickers"));
					}
					items.push(item);
					if sticker_bytes(&items) > MAX_STICKER_BYTES {
						return Err(serde::de::Error::custom("Sticker capacity"));
					}
				}
				Ok(StickerList(items))
			}
		}
		d.deserialize_seq(List::<MAX>)
	}
}
