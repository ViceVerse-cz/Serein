//! Documented embed attributes. Unknown types retain a bounded visible card.
use model::{Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedMedia};
use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct EmbedDto {
	#[serde(rename = "type")]
	kind: Option<String>,
	title: Option<String>,
	description: Option<String>,
	url: Option<String>,
	color: Option<u32>,
	timestamp: Option<String>,
	author: Option<AuthorDto>,
	provider: Option<AuthorDto>,
	footer: Option<FooterDto>,
	#[serde(deserialize_with = "model::deserialize_embed_fields")]
	fields: Vec<EmbedField>,
	image: Option<MediaDto>,
	thumbnail: Option<MediaDto>,
	video: Option<MediaDto>,
	audio: Option<MediaDto>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct MediaDto {
	url: Option<String>,
	proxy_url: Option<String>,
	width: u32,
	height: u32,
	placeholder: Option<String>,
	placeholder_version: Option<u32>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct AuthorDto {
	name: String,
	url: Option<String>,
	icon_url: Option<String>,
	proxy_icon_url: Option<String>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct FooterDto {
	text: String,
	icon_url: Option<String>,
	proxy_icon_url: Option<String>,
}
fn text(value: String, chars: usize, limited: &mut bool) -> String {
	let end = value
		.char_indices()
		.nth(chars)
		.map_or(value.len(), |(i, _)| i);
	*limited |= end < value.len();
	// Copy the bounded slice so a small truncated string cannot retain a large capacity.
	value[..end].to_owned()
}
fn url(value: Option<String>, limited: &mut bool) -> Option<String> {
	value.filter(|s| {
		let valid = s.len() <= 2048 && !s.chars().any(char::is_control);
		*limited |= !valid;
		valid
	})
}
fn media(value: Option<MediaDto>, limited: &mut bool) -> Option<EmbedMedia> {
	value.map(|m| EmbedMedia {
		url: url(m.url, limited),
		proxy_url: url(m.proxy_url, limited),
		width: m.width,
		height: m.height,
		placeholder: crate::attachments::placeholder(m.placeholder, m.placeholder_version),
	})
}
fn icon(original: Option<String>, proxy: Option<String>, limited: &mut bool) -> Option<EmbedMedia> {
	if original.is_none() && proxy.is_none() {
		return None;
	}
	media(
		Some(MediaDto {
			url: original,
			proxy_url: proxy,
			width: 32,
			height: 32,
			..Default::default()
		}),
		limited,
	)
}
fn author(a: AuthorDto, limited: &mut bool) -> EmbedAuthor {
	EmbedAuthor {
		name: text(a.name, 256, limited),
		url: url(a.url, limited),
		icon: icon(a.icon_url, a.proxy_icon_url, limited),
	}
}
impl EmbedDto {
	fn into_model(self) -> Embed {
		let mut limited = false;
		let mut fields = self.fields;
		limited |= fields.len() > 25;
		fields.truncate(25);
		let mut embed = Embed {
			kind: text(self.kind.unwrap_or_else(|| "rich".into()), 32, &mut limited),
			title: self.title.map(|s| text(s, 256, &mut limited)),
			description: self.description.map(|s| text(s, 4096, &mut limited)),
			url: url(self.url, &mut limited),
			color: self.color.filter(|n| *n <= 0xff_ffff),
			timestamp: self
				.timestamp
				.filter(|s| s.len() <= 64)
				.and_then(|s| crate::Timestamp::try_from(s.clone()).ok().map(|_| s)),
			author: self.author.map(|a| author(a, &mut limited)),
			provider: self.provider.map(|a| author(a, &mut limited)),
			footer: self.footer.map(|f| EmbedFooter {
				text: text(f.text, 2048, &mut limited),
				icon: icon(f.icon_url, f.proxy_icon_url, &mut limited),
			}),
			fields: fields
				.into_iter()
				.map(|f| EmbedField {
					name: text(f.name, 256, &mut limited),
					value: text(f.value, 1024, &mut limited),
					inline: f.inline,
				})
				.collect(),
			image: media(self.image, &mut limited),
			thumbnail: media(self.thumbnail, &mut limited),
			video: media(self.video, &mut limited),
			audio: media(self.audio, &mut limited),
			limited,
		};
		while embed.bytes() > model::MAX_EMBED_BYTES && !embed.fields.is_empty() {
			embed.fields.pop();
			embed.fields.shrink_to_fit();
			embed.limited = true;
		}
		embed
	}
}
pub fn bounded(values: Vec<EmbedDto>) -> Vec<Embed> {
	let too_many = values.len() > model::MAX_EMBEDS;
	let mut result = Vec::new();
	let mut bytes = 0;
	let mut limited = too_many;
	for dto in values.into_iter().take(model::MAX_EMBEDS) {
		let embed = dto.into_model();
		if bytes + embed.bytes() > model::MAX_EMBED_BYTES {
			limited = true;
			continue;
		}
		bytes += embed.bytes();
		result.push(embed);
	}
	if limited {
		if let Some(last) = result.last_mut() {
			last.limited = true;
		} else {
			result.push(Embed {
				limited: true,
				..Default::default()
			});
		}
	}
	result
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn rich_embeds_partial_updates_and_limits() {
		let m:crate::MessageDto=crate::decode(br##"{"id":"1","channel_id":"2","author":{"id":"3","username":"Synthetic"},"content":"","embeds":[{"type":"rich","title":"A card","color":5793266,"author":{"name":"Writer","proxy_icon_url":"https://media.discordapp.net/attachments/1/2/a.png"},"description":"**Native** card","fields":[{"name":"Version","value":"1","inline":true}],"image":{"url":"https://example.com/a.png","proxy_url":"https://images-ext-1.discordapp.net/external/hash/https/example.com/a.png","width":400,"height":200}}]}"##).unwrap();
		let m = m.into_model();
		assert_eq!(m.embeds.len(), 1);
		assert!(!m.unsupported);
		assert!(model::valid_embeds(&m.embeds));
		assert_eq!(m.embeds[0].fields[0].value, "1");
		for (raw, cleared) in [
			(r#"{"id":"1","channel_id":"2"}"#, false),
			(r#"{"id":"1","channel_id":"2","embeds":[]}"#, true),
			(r#"{"id":"1","channel_id":"2","embeds":null}"#, true),
		] {
			let p: crate::PatchDto = crate::decode(raw.as_bytes()).unwrap();
			let p = p.into_model();
			assert_eq!(!matches!(p.embeds, model::Patch::Absent), cleared);
		}
		let values = (0..50)
			.map(|_| EmbedDto {
				title: Some("世".repeat(2000)),
				description: Some("文".repeat(10000)),
				fields: vec![
					EmbedField {
						name: "n".repeat(2000),
						value: "v".repeat(10000),
						inline: true
					};
					50
				],
				..Default::default()
			})
			.collect();
		let result = bounded(values);
		assert!(model::valid_embeds(&result));
		assert!(result.iter().any(|e| e.limited));
		assert!(result.len() <= 10);
		let p: crate::PatchDto =
			crate::decode(br#"{"id":"1","channel_id":"2","flags":4}"#).unwrap();
		assert_eq!(p.into_model().embeds_suppressed, model::Patch::Value(true));
	}
}

#[derive(Default)]
pub struct EmbedList(pub Vec<EmbedDto>);
impl<'de> Deserialize<'de> for EmbedList {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct List;
		impl<'de> serde::de::Visitor<'de> for List {
			type Value = EmbedList;
			fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				f.write_str("at most ten embeds")
			}
			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut seq: A,
			) -> Result<Self::Value, A::Error> {
				let mut items = Vec::new();
				while items.len() < model::MAX_EMBEDS {
					match seq.next_element()? {
						Some(item) => items.push(item),
						None => return Ok(EmbedList(items)),
					}
				}
				if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
					return Err(serde::de::Error::custom("too many embeds"));
				}
				Ok(EmbedList(items))
			}
		}
		deserializer.deserialize_seq(List)
	}
}
