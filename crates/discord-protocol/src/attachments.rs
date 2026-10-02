//! Attachment objects are bounded metadata; image decoding and URL authorization live elsewhere.
use base64::Engine;
use model::{Attachment, EmbedMedia, Id};
use serde::Deserialize;

#[derive(Deserialize)]
struct AttachmentDto {
	id: Id,
	#[serde(default)]
	filename: String,
	#[serde(default)]
	description: Option<String>,
	#[serde(default)]
	content_type: Option<String>,
	#[serde(default)]
	size: u64,
	#[serde(default)]
	url: Option<String>,
	#[serde(default)]
	proxy_url: Option<String>,
	#[serde(default)]
	width: Option<u32>,
	#[serde(default)]
	height: Option<u32>,
	#[serde(default)]
	flags: u64,
	#[serde(default)]
	duration_secs: Option<f64>,
	#[serde(default)]
	waveform: Option<String>,
	#[serde(default)]
	placeholder: Option<String>,
	#[serde(default)]
	placeholder_version: Option<u32>,
}
/// Decode Discord's base64 ThumbHash placeholder; only version 1 is a known format.
pub(crate) fn placeholder(value: Option<String>, version: Option<u32>) -> Vec<u8> {
	if version != Some(1) {
		return Vec::new();
	}
	let mut bytes = [0u8; model::MAX_PLACEHOLDER_BYTES];
	value
		.filter(|value| value.len() <= model::MAX_PLACEHOLDER_BYTES / 3 * 4)
		.and_then(|value| {
			base64::engine::general_purpose::STANDARD
				.decode_slice(value.trim_end_matches('='), &mut bytes)
				.ok()
		})
		.filter(|len| *len >= 5)
		.map_or_else(Vec::new, |len| bytes[..len].to_vec())
}
fn url(value: Option<String>) -> Option<String> {
	value.filter(|value| value.len() <= 2048 && !value.chars().any(char::is_control))
}
impl AttachmentDto {
	fn into_model(self) -> Attachment {
		// Discord supplies at most 256 amplitude samples encoded as base64.
		let mut samples = [0u8; 256];
		let waveform = self
			.waveform
			.filter(|value| value.len() <= 344)
			.and_then(|value| {
				base64::engine::general_purpose::STANDARD
					.decode_slice(value, &mut samples)
					.ok()
			})
			.map_or_else(Vec::new, |len| samples[..len].to_vec());
		let mut filename: String = self
			.filename
			.chars()
			.take(256)
			.map(|ch| {
				if ch.is_control() || matches!(ch, '/' | '\\') {
					'_'
				} else {
					ch
				}
			})
			.collect();
		if filename.is_empty() {
			filename = "Attachment".into();
		}
		// Bit 3 is documented IS_SPOILER. Bits 4/6/7 are unofficial explicit/gore/self-harm
		// flags from discord.py-self flags.py AttachmentFlags; conceal them until deliberate reveal.
		let spoiler = self.flags & ((1 << 3) | (1 << 4) | (1 << 6) | (1 << 7)) != 0
			|| self.filename.starts_with("SPOILER_");
		let mut attachment = Attachment {
			duration_ms: self
				.duration_secs
				.filter(|value| value.is_finite() && (0.0..=600.0).contains(value))
				.map(|value| (value * 1000.0).round() as u32),
			waveform,
			id: self.id,
			filename,
			description: None,
			content_type: self
				.content_type
				.filter(|kind| {
					kind.len() <= 128 && kind.is_ascii() && !kind.chars().any(char::is_control)
				})
				.map(|kind| {
					kind.split(';')
						.next()
						.unwrap_or_default()
						.trim()
						.to_ascii_lowercase()
				})
				.filter(|kind| !kind.is_empty()),
			size: self.size,
			media: EmbedMedia {
				url: url(self.url),
				proxy_url: url(self.proxy_url),
				width: self.width.unwrap_or_default(),
				height: self.height.unwrap_or_default(),
				placeholder: placeholder(self.placeholder, self.placeholder_version),
			},
			spoiler,
		};
		// Give every allowed file its own share, so a long description cannot hide later files.
		let available = (model::MAX_ATTACHMENT_BYTES / model::MAX_ATTACHMENTS)
			.saturating_sub(model::attachment_bytes(std::slice::from_ref(&attachment)));
		attachment.description = self.description.map(|description| {
			let end = description
				.char_indices()
				.take(1024)
				.take_while(|(index, ch)| index + ch.len_utf8() <= available)
				.last()
				.map_or(0, |(index, ch)| index + ch.len_utf8());
			description[..end].to_owned()
		});
		attachment
	}
}

/// Normalize each item as it is decoded, rather than retaining a raw attachment array.
#[derive(Default)]
pub struct AttachmentList(pub Vec<Attachment>);
impl<'de> Deserialize<'de> for AttachmentList {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct List;
		impl<'de> serde::de::Visitor<'de> for List {
			type Value = AttachmentList;
			fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				f.write_str("at most ten attachments")
			}
			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut seq: A,
			) -> Result<Self::Value, A::Error> {
				let mut items = Vec::new();
				for _ in 0..model::MAX_ATTACHMENTS {
					match seq.next_element::<AttachmentDto>()? {
						Some(item) => items.push(item.into_model()),
						None => {
							items.shrink_to_fit();
							return Ok(AttachmentList(items));
						}
					}
				}
				if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
					return Err(serde::de::Error::custom("too many attachments"));
				}
				items.shrink_to_fit();
				Ok(AttachmentList(items))
			}
		}
		deserializer.deserialize_seq(List)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use model::Patch;
	use serde_json::json;

	#[test]
	fn image_only_messages_nullable_dimensions_spoilers_and_patch_semantics() {
		{
			let message = json!({"id":"1","channel_id":"2","author":{"id":"3","username":"Synthetic"},
	            "attachments":[{"id":"4","filename":"photo.jpg","content_type":"image/jpeg","size":1234,
	                "url":"https://cdn.discordapp.com/attachments/2/4/photo.jpg","proxy_url":"https://media.discordapp.net/attachments/2/4/photo.jpg","width":1600,"height":900},
	                {"id":"5","filename":"SPOILER_image.png","content_type":"image/png","size":42,"width":null,"height":null},
	                {"id":"6","filename":"image.png","flags":8}]});
			let model = crate::decode::<crate::MessageDto>(message.to_string().as_bytes())
				.unwrap()
				.into_model();
			assert!(model.content.is_empty());
			assert!(!model.unsupported);
			assert_eq!(model.attachments.len(), 3);
			assert!(model::valid_attachments(&model.attachments));
			assert_eq!(model.attachments[0].media.width, 1600);
			assert_eq!(
				model.attachments[0].content_type.as_deref(),
				Some("image/jpeg")
			);
			assert!(model.attachments[0].is_image());
			assert!(!model.attachments[0].spoiler);
			assert_eq!(model.attachments[1].media.width, 0);
			assert_eq!(model.attachments[1].media.height, 0);
			assert!(model.attachments[1].spoiler);
			assert!(model.attachments[2].spoiler);
			for (value, empty) in [
				(None, false),
				(Some(json!(null)), true),
				(Some(json!([])), true),
			] {
				let mut patch = json!({"id":"1","channel_id":"2"});
				if let Some(value) = value {
					patch["attachments"] = value;
				}
				let patch = crate::decode::<crate::PatchDto>(patch.to_string().as_bytes())
					.unwrap()
					.into_model();
				assert_eq!(!matches!(patch.attachments, Patch::Absent), empty);
				assert!(matches!(patch.content, Patch::Absent));
			}
			let patch = json!({"id":"1","channel_id":"2","attachments":[{"id":"4","filename":"edited.png","content_type":"image/png","width":32,"height":16}]});
			let patch = crate::decode::<crate::PatchDto>(patch.to_string().as_bytes())
				.unwrap()
				.into_model();
			let Patch::Value(attachments) = patch.attachments else {
				panic!("missing attachment patch")
			};
			assert_eq!(attachments[0].filename, "edited.png");
			assert_eq!(attachments[0].media.height, 16);
		}
		{
			let message = json!({"id":"1","channel_id":"2","author":{"id":"3","username":"Synthetic"},
	            "attachments":[
	                {"id":"4","filename":"a.png","placeholder":"1QcSHQRnh493V4dIh4eXh1h4kJUI","placeholder_version":1},
	                {"id":"5","filename":"b.png","placeholder":"1QcSHQRnh493V4dIh4eXh1h4kJUI","placeholder_version":2},
	                {"id":"6","filename":"c.png","placeholder":"1QcSHQRnh493V4dIh4eXh1h4kJUI"},
	                {"id":"7","filename":"d.png","placeholder":"AAAA","placeholder_version":1},
	                {"id":"8","filename":"e.png","placeholder":"QUFB".repeat(40),"placeholder_version":1},
	                {"id":"9","filename":"f.png","placeholder":"not base64!","placeholder_version":1}],
	            "embeds":[{"type":"image","thumbnail":{"url":"https://example.com/a.png","width":4,"height":2,
	                "placeholder":"1QcSHQRnh493V4dIh4eXh1h4kJUI","placeholder_version":1}}]});
			let model = crate::decode::<crate::MessageDto>(message.to_string().as_bytes())
				.unwrap()
				.into_model();
			let placeholder = &model.attachments[0].media.placeholder;
			assert_eq!(placeholder.len(), 21);
			assert_eq!(placeholder[..3], [0xd5, 0x07, 0x12]);
			for attachment in &model.attachments[1..] {
				assert!(
					attachment.media.placeholder.is_empty(),
					"{}",
					attachment.filename
				);
			}
			assert!(model::valid_attachments(&model.attachments));
			let thumbnail = model.embeds[0].thumbnail.as_ref().unwrap();
			assert_eq!(&thumbnail.placeholder, placeholder);
			assert_eq!(thumbnail.width, 4);
		}
	}

	#[test]
	fn malformed_metadata_and_array_budget_keep_all_allowed_files_bounded() {
		{
			let item = json!({"id":"4","filename":"世".repeat(2000),"description":"文".repeat(6000),
	            "content_type":"image/png; charset=binary","size":u64::MAX,
	            "url":format!("https://example.com/{}", "x".repeat(2028)),
	            "proxy_url":format!("https://example.com/{}", "y".repeat(2028))});
			let list = crate::decode::<AttachmentList>(
				json!(vec![item.clone(); 10]).to_string().as_bytes(),
			)
			.unwrap();
			assert_eq!(list.0.len(), 10);
			assert!(model::valid_attachments(&list.0));
			assert!(model::attachment_bytes(&list.0) <= model::MAX_ATTACHMENT_BYTES);
			assert_eq!(list.0[0].filename.chars().count(), 256);
			assert_eq!(list.0[0].content_type.as_deref(), Some("image/png"));
			assert!(
				crate::decode::<AttachmentList>(json!(vec![item; 11]).to_string().as_bytes())
					.is_err()
			);
			for flag in [8, 16, 64, 128] {
				let list = crate::decode::<AttachmentList>(
					json!([{"id":"4","filename":"image.png","flags":flag}])
						.to_string()
						.as_bytes(),
				)
				.unwrap();
				assert!(list.0[0].spoiler);
			}
			let animated = crate::decode::<AttachmentList>(
				br#"[{"id":"4","filename":"image.gif","flags":32}]"#,
			)
			.unwrap();
			assert!(!animated.0[0].spoiler);
			assert!(animated.0[0].is_image());
			let kinds = crate::decode::<AttachmentList>(br#"[{"id":"4","filename":"photo.PNG"},{"id":"5","filename":"plain.png","content_type":"text/plain"},{"id":"6","filename":"unknown.bin"},{"id":"7","filename":"unknown.bin","content_type":"image/jpeg"}]"#).unwrap();
			assert!(kinds.0[0].is_image());
			assert!(!kinds.0[1].is_image());
			assert!(!kinds.0[2].is_image());
			assert!(kinds.0[3].is_image());
			let malformed = json!([{"id":"4","filename":"../bad\nimage.png","content_type":"image/png\nwrong", "url":"https://example.com/\nunsafe", "proxy_url":"x".repeat(2049)}]);
			let list = crate::decode::<AttachmentList>(malformed.to_string().as_bytes()).unwrap();
			assert!(!list.0[0].filename.contains('/'));
			assert!(!list.0[0].filename.chars().any(char::is_control));
			assert!(list.0[0].content_type.is_none());
			assert!(list.0[0].media.url.is_none());
			assert!(list.0[0].media.proxy_url.is_none());
		}
	}
}
