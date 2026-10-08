//! Unofficial FrecencyUserSettings interoperability. Never replace a truncated favorites map.
use crate::{
	DecodeError,
	guild_folders::{fields, message},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use model::{Gif, MAX_GIF_FAVORITES};
use serde::Deserialize;
use std::collections::HashSet;

pub const MAX_RESPONSE: usize = 6 * 1024 * 1024;
const MAX_SUBTREE: usize = 512 * 1024;
const MAX_ENTRIES: usize = 2048;

pub struct Decoded {
	pub version: u32,
	subtree: Vec<u8>,
}

#[derive(Deserialize)]
struct Response {
	settings: String,
	#[serde(default)]
	out_of_date: bool,
}

struct Entry<'a> {
	url: &'a str,
	source: &'a str,
	format: u32,
	width: u32,
	height: u32,
	order: u32,
}

fn entry(bytes: &[u8]) -> Result<Entry<'_>, DecodeError> {
	if bytes.len() > 4096 {
		return Err(DecodeError);
	}
	let mut url = None;
	let mut value = None;
	for field in fields(bytes)? {
		match field.number {
			1 if url.is_none() => {
				url = Some(std::str::from_utf8(field.message()?).map_err(|_| DecodeError)?)
			}
			2 if value.is_none() => value = Some(field.message()?),
			1 | 2 => return Err(DecodeError),
			_ => {}
		}
	}
	let url = url.ok_or(DecodeError)?;
	if url.is_empty() {
		return Err(DecodeError);
	}
	let mut result = Entry {
		url,
		source: "",
		format: 0,
		width: 0,
		height: 0,
		order: 0,
	};
	let mut seen = 0u8;
	for field in fields(value.unwrap_or_default())? {
		if (1..=5).contains(&field.number) {
			let bit = 1 << field.number;
			if seen & bit != 0 {
				return Err(DecodeError);
			}
			seen |= bit;
		}
		match field.number {
			2 => {
				result.source = std::str::from_utf8(field.message()?).map_err(|_| DecodeError)?;
			}
			1 | 3 | 4 | 5 => {
				let value = u32::try_from(field.integer()?).map_err(|_| DecodeError)?;
				match field.number {
					1 => result.format = value,
					3 => result.width = value,
					4 => result.height = value,
					_ => result.order = value,
				}
			}
			_ => {}
		}
	}
	Ok(result)
}

pub fn decode_response(bytes: &[u8]) -> Result<Decoded, DecodeError> {
	if bytes.len() > MAX_RESPONSE {
		return Err(DecodeError);
	}
	let response: Response = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	if response.out_of_date {
		return Err(DecodeError);
	}
	let wire = STANDARD
		.decode(response.settings)
		.map_err(|_| DecodeError)?;
	let mut version = None;
	let mut subtree = None;
	for field in fields(&wire)? {
		match field.number {
			1 if version.is_none() => {
				let mut data_version = None;
				for field in fields(field.message()?)? {
					if field.number == 3 {
						if data_version.is_some() {
							return Err(DecodeError);
						}
						data_version =
							Some(u32::try_from(field.integer()?).map_err(|_| DecodeError)?);
					}
				}
				version = Some(data_version.unwrap_or(0));
			}
			2 if subtree.is_none() => {
				let value = field.message()?;
				if value.len() > MAX_SUBTREE {
					return Err(DecodeError);
				}
				subtree = Some(value.to_vec());
			}
			1 | 2 => return Err(DecodeError),
			_ => {}
		}
	}
	let decoded = Decoded {
		// Versions is optional in FrecencyUserSettings (including an empty initial proto).
		version: version.unwrap_or(0),
		subtree: subtree.unwrap_or_default(),
	};
	let mut keys = HashSet::new();
	for field in fields(&decoded.subtree)? {
		if field.number == 1 {
			let entry = entry(field.message()?)?;
			if keys.len() >= MAX_ENTRIES || !keys.insert(entry.url) {
				return Err(DecodeError);
			}
		}
	}
	Ok(decoded)
}

impl Decoded {
	/// Only a bounded native projection is retained by the client. Unsupported records survive saves.
	pub fn favorites(&self) -> Result<Vec<Gif>, DecodeError> {
		self.favorites_for(None)
	}
	pub fn favorites_for(&self, preferred: Option<&str>) -> Result<Vec<Gif>, DecodeError> {
		let mut entries = Vec::new();
		for field in fields(&self.subtree)? {
			if field.number == 1 {
				entries.push(entry(field.message()?)?);
			}
		}
		entries.sort_by(|a, b| {
			(Some(b.url) == preferred)
				.cmp(&(Some(a.url) == preferred))
				.then_with(|| b.order.cmp(&a.order))
				.then_with(|| a.url.cmp(b.url))
		});
		let mut favorites = Vec::with_capacity(MAX_GIF_FAVORITES);
		for entry in entries {
			if !model::valid_gif_favorite_url(entry.url) {
				continue;
			}
			// Media saved from a message may omit its source; the address is then the media.
			let preview = if model::valid_gif_favorite_source(entry.source) {
				entry.source
			} else if model::valid_gif_favorite_source(entry.url) {
				entry.url
			} else {
				continue;
			};
			// Unknown dimensions lay out as a square tile instead of hiding the favorite.
			let (width, height) = if entry.width == 0 || entry.height == 0 {
				(1, 1)
			} else {
				(entry.width.min(4096), entry.height.min(4096))
			};
			let gif = Gif {
				id: favorite_id(entry.url),
				title: String::new(),
				url: entry.url.into(),
				preview: preview.into(),
				width,
				height,
			};
			if gif.valid() {
				favorites.push(gif);
			}
			if favorites.len() == MAX_GIF_FAVORITES {
				break;
			}
		}
		Ok(favorites)
	}
	pub fn contains(&self, url: &str) -> Result<bool, DecodeError> {
		for field in fields(&self.subtree)? {
			if field.number == 1 && entry(field.message()?)?.url == url {
				return Ok(true);
			}
		}
		Ok(false)
	}
	pub fn matches(&self, gif: &Gif, favorite: bool) -> Result<bool, DecodeError> {
		for field in fields(&self.subtree)? {
			if field.number == 1 {
				let entry = entry(field.message()?)?;
				if entry.url == gif.url {
					return Ok(favorite
						&& entry.source == gif.preview
						&& entry.format == format(&gif.preview)
						&& entry.width == gif.width
						&& entry.height == gif.height);
				}
			}
		}
		Ok(!favorite)
	}
	pub fn unchanged_except(&self, saved: &Self, url: &str) -> Result<bool, DecodeError> {
		type Retained<'a> = (std::collections::BTreeMap<&'a str, &'a [u8]>, Vec<u8>);
		fn retained<'a>(decoded: &'a Decoded, url: &str) -> Result<Retained<'a>, DecodeError> {
			let mut entries = std::collections::BTreeMap::new();
			let mut other = Vec::new();
			for field in fields(&decoded.subtree)? {
				if field.number == 1 {
					let entry = entry(field.message()?)?;
					if entry.url != url {
						entries.insert(entry.url, field.raw);
					}
				} else {
					other.extend_from_slice(field.raw);
				}
			}
			Ok((entries, other))
		}
		Ok(retained(self, url)? == retained(saved, url)?)
	}
}

/// Stable per URL, so the local fallback never collapses two favorites onto one row.
fn favorite_id(url: &str) -> String {
	let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
		(hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
	});
	format!("discord-{hash:016x}")
}

fn format(source: &str) -> u32 {
	if model::gif_source_is_video(source) {
		2
	} else {
		1
	}
}

fn integer(number: u8, mut value: u32, output: &mut Vec<u8>) {
	output.push(number << 3);
	while value >= 128 {
		output.push(value as u8 | 0x80);
		value >>= 7;
	}
	output.push(value as u8);
}

/// Change one URL against a fresh, complete read; retain every untouched raw entry and subtree field.
pub fn encode_patch(current: &Decoded, gif: &Gif, favorite: bool) -> Result<String, DecodeError> {
	if !gif.valid() {
		return Err(DecodeError);
	}
	let mut subtree = Vec::with_capacity(MAX_SUBTREE);
	let mut count = 0;
	let mut highest_order = None::<u32>;
	for field in fields(&current.subtree)? {
		if field.number == 1 {
			let entry = entry(field.message()?)?;
			highest_order = Some(highest_order.map_or(entry.order, |old| old.max(entry.order)));
			if entry.url == gif.url {
				continue;
			}
			count += 1;
		}
		subtree.extend_from_slice(field.raw);
	}
	if favorite {
		if count >= MAX_ENTRIES {
			return Err(DecodeError);
		}
		let order = highest_order
			.map_or(Some(0), |order| order.checked_add(1))
			.ok_or(DecodeError)?;
		let mut value = Vec::new();
		integer(1, format(&gif.preview), &mut value);
		message(2, gif.preview.as_bytes(), &mut value);
		integer(3, gif.width, &mut value);
		integer(4, gif.height, &mut value);
		integer(5, order, &mut value);
		let mut encoded = Vec::new();
		message(1, gif.url.as_bytes(), &mut encoded);
		message(2, &value, &mut encoded);
		message(1, &encoded, &mut subtree);
	}
	if subtree.len() > MAX_SUBTREE {
		return Err(DecodeError);
	}
	let mut patch = Vec::new();
	message(2, &subtree, &mut patch);
	Ok(STANDARD.encode(patch))
}

#[cfg(test)]
mod tests {
	use super::*;
	fn gif(index: usize) -> Gif {
		Gif {
			id: format!("test-{index}"),
			title: "Synthetic".into(),
			url: format!("https://tenor.com/view/synthetic-{index}"),
			preview: format!("https://media.tenor.com/synthetic/{index}.gif"),
			width: 300,
			height: 200,
		}
	}
	fn response(subtree: &[u8], version: u32) -> Vec<u8> {
		let mut versions = Vec::new();
		integer(3, version, &mut versions);
		let mut wire = Vec::new();
		message(1, &versions, &mut wire);
		message(2, subtree, &mut wire);
		message(3, b"untouched sticker settings", &mut wire);
		serde_json::json!({"settings":STANDARD.encode(wire)})
			.to_string()
			.into_bytes()
	}
	fn catalog(count: usize) -> Decoded {
		let mut subtree = Vec::new();
		integer(2, 1, &mut subtree);
		message(19, b"future subtree setting", &mut subtree);
		for index in 0..count {
			let gif = gif(index);
			let mut value = Vec::new();
			integer(1, if index == 101 { 2 } else { 1 }, &mut value);
			message(
				2,
				if index == 101 {
					b"https://media.tenor.com/synthetic/video.mp4"
				} else {
					gif.preview.as_bytes()
				},
				&mut value,
			);
			integer(3, 300, &mut value);
			integer(4, 200, &mut value);
			integer(5, index as u32, &mut value);
			message(20, b"future per-entry metadata", &mut value);
			let mut entry = Vec::new();
			message(1, gif.url.as_bytes(), &mut entry);
			message(2, &value, &mut entry);
			message(18, b"future map-entry metadata", &mut entry);
			message(1, &entry, &mut subtree);
		}
		decode_response(&response(&subtree, 7)).unwrap()
	}
	fn patched(current: &Decoded, gif: &Gif, favorite: bool) -> Decoded {
		let patch = STANDARD
			.decode(encode_patch(current, gif, favorite).unwrap())
			.unwrap();
		let fields = fields(&patch).unwrap();
		assert_eq!(fields.len(), 1);
		assert_eq!(fields[0].number, 2);
		decode_response(&response(fields[0].message().unwrap(), 8)).unwrap()
	}
	#[test]
	fn favorites_projection_keeps_newest_entries_after_plain_refresh() {
		let current = catalog(MAX_GIF_FAVORITES + 2);
		let favorites = current.favorites().unwrap();
		assert_eq!(favorites.len(), MAX_GIF_FAVORITES);
		assert_eq!(favorites[0].url, gif(101).url);
		assert!(!favorites.iter().any(|entry| entry.url == gif(0).url));
		let added = gif(1000);
		let saved = patched(&current, &added, true);
		assert_eq!(saved.favorites().unwrap()[0].url, added.url);
		assert_eq!(
			saved.favorites_for(Some(&gif(3).url)).unwrap()[0].url,
			gif(3).url
		);
	}
	#[test]
	fn favorites_preserve_unseen_unknown_and_video_records_during_single_url_changes() {
		let current = catalog(102);
		assert_eq!(current.favorites().unwrap().len(), 100);
		let added = gif(1000);
		let saved = patched(&current, &added, true);
		assert!(current.unchanged_except(&saved, &added.url).unwrap());
		assert!(saved.matches(&added, true).unwrap());
		assert!(saved.contains(&gif(101).url).unwrap());
		assert_eq!(
			saved.favorites_for(Some(&added.url)).unwrap()[0].url,
			added.url
		);
		let video = saved.favorites_for(Some(&gif(101).url)).unwrap().remove(0);
		assert!(video.valid());
		assert!(video.preview.ends_with("video.mp4"));
		assert!(!model::valid_gif_preview(&video.preview));
		let removed = patched(&saved, &video, false);
		assert!(removed.matches(&video, false).unwrap());
		assert!(saved.unchanged_except(&removed, &video.url).unwrap());
		let restored = patched(&removed, &video, true);
		assert!(restored.matches(&video, true).unwrap());
		let partial = catalog(0);
		assert!(!current.unchanged_except(&partial, &added.url).unwrap());
	}
	#[test]
	fn favorites_reject_truncated_duplicate_and_oversized_catalogs_before_saving() {
		let current = catalog(1);
		let mut truncated = current.subtree.clone();
		truncated.pop();
		assert!(decode_response(&response(&truncated, 7)).is_err());
		let mut duplicate = current.subtree.clone();
		for field in fields(&current.subtree).unwrap() {
			if field.number == 1 {
				duplicate.extend_from_slice(field.raw);
			}
		}
		assert!(decode_response(&response(&duplicate, 7)).is_err());
		assert!(decode_response(&response(&vec![0; MAX_SUBTREE + 1], 7)).is_err());
		let mut outdated: serde_json::Value =
			serde_json::from_slice(&response(&current.subtree, 7)).unwrap();
		outdated["out_of_date"] = true.into();
		assert!(decode_response(outdated.to_string().as_bytes()).is_err());
		let mut too_many = catalog(MAX_ENTRIES);
		assert!(encode_patch(&too_many, &gif(10000), true).is_err());
		too_many.subtree.extend_from_slice(&current.subtree);
		assert!(decode_response(&response(&too_many.subtree, 7)).is_err());
	}
}
