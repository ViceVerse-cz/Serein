//! Bounded attachment metadata. Image bytes use the shared client media cache.
use crate::{EmbedMedia, Id};
use serde::{Deserialize, Serialize};

pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_ATTACHMENT_BYTES: usize = 64 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Attachment {
	pub id: Id,
	pub filename: String,
	pub description: Option<String>,
	pub content_type: Option<String>,
	pub size: u64,
	pub media: EmbedMedia,
	pub spoiler: bool,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub duration_ms: Option<u32>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub waveform: Vec<u8>,
}
impl Attachment {
	pub fn is_video(&self) -> bool {
		// Metadata selects the player; the native decoder validates the container and codec.
		self.filename
			.rsplit_once('.')
			.is_some_and(|(_, extension)| {
				["mov", "mp4", "webm", "mkv", "avi", "m4v"]
					.iter()
					.any(|kind| extension.eq_ignore_ascii_case(kind))
			}) || self.content_type.as_deref().is_some_and(|kind| {
			let kind = kind.split(';').next().unwrap_or(kind).trim();
			kind.get(..6)
				.is_some_and(|prefix| prefix.eq_ignore_ascii_case("video/"))
		})
	}
	pub fn is_voice_message(&self) -> bool {
		self.filename
			.trim_start_matches("SPOILER_")
			.eq_ignore_ascii_case("voice-message.ogg")
			|| (!self.waveform.is_empty() && self.is_audio())
	}
	pub fn is_audio(&self) -> bool {
		// Metadata only selects the preview; playback validates the actual bytes.
		self.filename
			.rsplit_once('.')
			.is_some_and(|(_, extension)| {
				["mp3", "wav", "ogg", "opus"]
					.iter()
					.any(|kind| extension.eq_ignore_ascii_case(kind))
			}) || self.content_type.as_deref().is_some_and(|kind| {
			let kind = kind.split(';').next().unwrap_or(kind).trim();
			[
				"audio/mpeg",
				"audio/mp3",
				"audio/wav",
				"audio/x-wav",
				"audio/wave",
				"audio/ogg",
				"audio/opus",
			]
			.iter()
			.any(|supported| kind.eq_ignore_ascii_case(supported))
		})
	}
	pub fn bytes(&self) -> usize {
		size_of::<Self>()
			+ self.filename.capacity()
			+ self.description.as_ref().map_or(0, String::capacity)
			+ self.content_type.as_ref().map_or(0, String::capacity)
			+ self.waveform.capacity()
			+ self.media.url.as_ref().map_or(0, String::capacity)
			+ self.media.proxy_url.as_ref().map_or(0, String::capacity)
			+ self.media.placeholder.capacity()
	}
	pub fn is_image(&self) -> bool {
		if self.is_audio() || self.is_video() {
			return false;
		}
		if let Some(kind) = &self.content_type {
			matches!(
				kind.to_ascii_lowercase().as_str(),
				"image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/avif"
			)
		} else {
			self.filename
				.rsplit_once('.')
				.is_some_and(|(_, extension)| {
					matches!(
						extension.to_ascii_lowercase().as_str(),
						"png" | "jpg" | "jpeg" | "webp" | "gif" | "avif"
					)
				})
		}
	}
}
pub fn attachment_bytes(attachments: &[Attachment]) -> usize {
	attachments.iter().map(Attachment::bytes).sum()
}
pub fn valid_attachments(attachments: &[Attachment]) -> bool {
	attachments.len() <= MAX_ATTACHMENTS
		&& attachment_bytes(attachments) <= MAX_ATTACHMENT_BYTES
		&& attachments.iter().all(|a| {
			a.waveform.len() <= 256
				&& a.duration_ms.is_none_or(|duration| duration <= 600_000)
				&& a.filename.len() <= 1024
				&& a.description.as_ref().is_none_or(|s| s.len() <= 4096)
				&& a.content_type.as_ref().is_none_or(|s| s.len() <= 128)
				&& [&a.media.url, &a.media.proxy_url]
					.into_iter()
					.all(|s| s.as_ref().is_none_or(|s| s.len() <= 2048))
		})
}

#[cfg(test)]
mod audio_tests {
	use super::*;
	#[test]
	fn video_detection_uses_mime_and_case_insensitive_extension() {
		{
			let mut file = Attachment {
				id: Id(1),
				filename: String::new(),
				description: None,
				content_type: None,
				size: 32,
				media: EmbedMedia::default(),
				spoiler: false,
				duration_ms: None,
				waveform: Vec::new(),
			};
			for filename in [
				"CLIP.MOV",
				"clip.Mp4",
				"clip.webm",
				"clip.mkv",
				"clip.avi",
				"clip.m4v",
			] {
				file.filename = filename.into();
				for kind in [None, Some("application/octet-stream"), Some("image/jpeg")] {
					file.content_type = kind.map(str::to_owned);
					assert!(file.is_video(), "{filename}: {kind:?}");
					assert!(!file.is_image());
				}
			}
			file.filename = "clip".into();
			file.content_type = Some(" Video/Quicktime; codecs=avc1 ".into());
			assert!(file.is_video());
			file.filename = "clip.mov.exe".into();
			file.content_type = None;
			assert!(!file.is_video());
		}
		{
			let mut file = Attachment {
				duration_ms: None,
				waveform: Vec::new(),
				id: Id(1),
				filename: "TRACK.MP3".into(),
				description: None,
				content_type: None,
				size: 32,
				media: EmbedMedia::default(),
				spoiler: false,
			};
			for filename in ["TRACK.MP3", "track.WaV", "voice-message.ogg", "track.opus"] {
				file.filename = filename.into();
				for kind in [
					None,
					Some(""),
					Some("application/octet-stream"),
					Some("text/plain"),
					Some("audio/ogg"),
					Some("image/jpeg"),
				] {
					file.content_type = kind.map(str::to_owned);
					assert!(file.is_audio(), "{filename}: {kind:?}");
					assert!(!file.is_image());
				}
			}
			file.filename = "attachment".into();
			for kind in [
				"audio/mpeg",
				"audio/mp3",
				"Audio/Wav; codec=pcm",
				"audio/x-wav",
				"audio/wave",
			] {
				file.content_type = Some(kind.into());
				assert!(file.is_audio());
				assert!(!file.is_image());
			}
			for (filename, kind, image) in [
				("track.mp3.exe", None, false),
				("picture.jpg", Some("image/jpeg"), true),
				("picture.png", None, true),
			] {
				file.filename = filename.into();
				file.content_type = kind.map(str::to_owned);
				assert!(!file.is_audio());
				assert_eq!(file.is_image(), image);
			}
		}
	}
}
#[derive(Default)]
pub struct AttachmentList(pub Vec<Attachment>);
impl<'de> Deserialize<'de> for AttachmentList {
	fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
		struct Visitor;
		impl<'de> serde::de::Visitor<'de> for Visitor {
			type Value = AttachmentList;
			fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
				f.write_str("at most ten attachments")
			}
			fn visit_seq<A: serde::de::SeqAccess<'de>>(
				self,
				mut sequence: A,
			) -> Result<Self::Value, A::Error> {
				let mut items = Vec::new();
				for _ in 0..MAX_ATTACHMENTS {
					match sequence.next_element()? {
						Some(item) => items.push(item),
						None => return Ok(AttachmentList(items)),
					}
				}
				if sequence.next_element::<serde::de::IgnoredAny>()?.is_some() {
					return Err(serde::de::Error::custom("attachment limit"));
				}
				Ok(AttachmentList(items))
			}
		}
		deserializer.deserialize_seq(Visitor)
	}
}
