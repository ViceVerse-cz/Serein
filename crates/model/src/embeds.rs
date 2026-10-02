//! Bounded UI-neutral service embeds; URLs are metadata, never permission to fetch.
use serde::{Deserialize, Serialize};

pub const MAX_EMBEDS: usize = 10;
pub const MAX_EMBED_BYTES: usize = 64 * 1024;
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbedMedia {
	pub url: Option<String>,
	pub proxy_url: Option<String>,
	pub width: u32,
	pub height: u32,
	/// Discord's decoded ThumbHash (`placeholder_version` 1); painted until real pixels arrive.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub placeholder: Vec<u8>,
}
/// ThumbHash payloads are 5 header bytes plus AC coefficients; Discord's never exceed ~30.
pub const MAX_PLACEHOLDER_BYTES: usize = 64;
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbedAuthor {
	pub name: String,
	pub url: Option<String>,
	pub icon: Option<EmbedMedia>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbedFooter {
	pub text: String,
	pub icon: Option<EmbedMedia>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbedField {
	pub name: String,
	pub value: String,
	pub inline: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Embed {
	pub kind: String,
	pub title: Option<String>,
	pub description: Option<String>,
	pub url: Option<String>,
	pub color: Option<u32>,
	pub timestamp: Option<String>,
	pub author: Option<EmbedAuthor>,
	pub provider: Option<EmbedAuthor>,
	pub footer: Option<EmbedFooter>,
	#[serde(deserialize_with = "deserialize_embed_fields")]
	pub fields: Vec<EmbedField>,
	pub image: Option<EmbedMedia>,
	pub thumbnail: Option<EmbedMedia>,
	pub video: Option<EmbedMedia>,
	pub limited: bool,
}
fn string_bytes(value: &Option<String>) -> usize {
	value.as_ref().map_or(0, String::capacity)
}
impl EmbedMedia {
	pub fn bytes(&self) -> usize {
		string_bytes(&self.url) + string_bytes(&self.proxy_url) + self.placeholder.capacity()
	}
	pub fn valid(&self) -> bool {
		[&self.url, &self.proxy_url]
			.into_iter()
			.all(|s| s.as_ref().is_none_or(|s| s.len() <= 2048))
			&& self.placeholder.len() <= MAX_PLACEHOLDER_BYTES
	}
}
impl EmbedAuthor {
	fn bytes(&self) -> usize {
		self.name.capacity()
			+ string_bytes(&self.url)
			+ self.icon.as_ref().map_or(0, EmbedMedia::bytes)
	}
	fn valid(&self) -> bool {
		self.name.len() <= 1024
			&& self.url.as_ref().is_none_or(|s| s.len() <= 2048)
			&& self.icon.as_ref().is_none_or(EmbedMedia::valid)
	}
}
impl Embed {
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.kind.capacity()
			+ [&self.title, &self.description, &self.url, &self.timestamp]
				.into_iter()
				.map(string_bytes)
				.sum::<usize>()
			+ [&self.author, &self.provider]
				.into_iter()
				.flatten()
				.map(EmbedAuthor::bytes)
				.sum::<usize>()
			+ self.footer.as_ref().map_or(0, |f| {
				f.text.capacity() + f.icon.as_ref().map_or(0, EmbedMedia::bytes)
			}) + self.fields.capacity() * size_of::<EmbedField>()
			+ self
				.fields
				.iter()
				.map(|f| f.name.capacity() + f.value.capacity())
				.sum::<usize>()
			+ [&self.image, &self.thumbnail, &self.video]
				.into_iter()
				.flatten()
				.map(EmbedMedia::bytes)
				.sum::<usize>()
	}
	fn valid(&self) -> bool {
		self.kind.len() <= 32
			&& self.title.as_ref().is_none_or(|s| s.len() <= 1024)
			&& self.description.as_ref().is_none_or(|s| s.len() <= 16_384)
			&& self.url.as_ref().is_none_or(|s| s.len() <= 2048)
			&& self.timestamp.as_ref().is_none_or(|s| s.len() <= 64)
			&& self.fields.len() <= 25
			&& self
				.fields
				.iter()
				.all(|f| f.name.len() <= 1024 && f.value.len() <= 4096)
			&& [&self.author, &self.provider]
				.into_iter()
				.flatten()
				.all(EmbedAuthor::valid)
			&& self.footer.as_ref().is_none_or(|f| {
				f.text.len() <= 8192 && f.icon.as_ref().is_none_or(EmbedMedia::valid)
			}) && [&self.image, &self.thumbnail, &self.video]
			.into_iter()
			.flatten()
			.all(EmbedMedia::valid)
	}
}
pub fn embed_bytes(embeds: &[Embed]) -> usize {
	embeds.iter().map(Embed::bytes).sum()
}
pub fn valid_embeds(embeds: &[Embed]) -> bool {
	embeds.len() <= MAX_EMBEDS
		&& embed_bytes(embeds) <= MAX_EMBED_BYTES
		&& embeds.iter().all(Embed::valid)
}

/// Enforce the field count during deserialization, before allocating a malicious array.
pub fn deserialize_embed_fields<'de, D: serde::Deserializer<'de>>(
	deserializer: D,
) -> Result<Vec<EmbedField>, D::Error> {
	struct Visitor;
	impl<'de> serde::de::Visitor<'de> for Visitor {
		type Value = Vec<EmbedField>;
		fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			f.write_str("at most 25 embed fields")
		}
		fn visit_seq<A: serde::de::SeqAccess<'de>>(
			self,
			mut sequence: A,
		) -> Result<Self::Value, A::Error> {
			let mut fields = Vec::new();
			for _ in 0..25 {
				match sequence.next_element()? {
					Some(field) => fields.push(field),
					None => return Ok(fields),
				}
			}
			if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
				return Err(serde::de::Error::custom("embed field limit"));
			}
			Ok(fields)
		}
	}
	deserializer.deserialize_seq(Visitor)
}
