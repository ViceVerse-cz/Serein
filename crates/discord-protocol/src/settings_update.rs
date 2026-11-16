//! Unofficial USER_SETTINGS_PROTO_UPDATE interoperability: which account settings another
//! session changed. Values are re-read over REST; this only says what to refresh.
use crate::{DecodeError, guild_folders::fields};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;

const MAX_UPDATE_BYTES: usize = crate::guild_folders::MAX_SETTINGS_RESPONSE;
const PRELOADED_SETTINGS: u8 = 1;
const STATUS_FIELD: u64 = 11;
const FOLDERS_FIELD: u64 = 14;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Touched {
	pub status: bool,
	pub folders: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrecencyTouched {
	pub reactions: bool,
	pub favorites: bool,
}

/// Full frecency updates invalidate both projections; partial updates only touch supplied fields.
/// Invalid envelopes or wire data cannot change the local favorite definitions.
pub fn decode_frecency(bytes: &[u8]) -> Result<FrecencyTouched, DecodeError> {
	#[derive(Deserialize)]
	struct Update {
		settings: Proto,
		#[serde(default)]
		partial: bool,
	}
	#[derive(Deserialize)]
	struct Proto {
		#[serde(rename = "type")]
		kind: u8,
		proto: String,
	}
	if bytes.len() > crate::gif_favorites::MAX_RESPONSE {
		return Err(DecodeError);
	}
	let update: Update = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	if update.settings.kind != 2 {
		return Ok(FrecencyTouched::default());
	}
	let wire = STANDARD
		.decode(update.settings.proto)
		.map_err(|_| DecodeError)?;
	let fields = fields(&wire)?;
	let mut touched = FrecencyTouched {
		reactions: !update.partial,
		favorites: !update.partial,
	};
	for field in fields {
		match field.number {
			2 => {
				field.message()?;
				touched.favorites = true;
			}
			6 | 13 => touched.reactions = true,
			_ => {}
		}
	}
	Ok(touched)
}

/// Full frecency updates or reaction/emoji deltas invalidate the bounded account projection.
pub fn reactions_changed(bytes: &[u8]) -> Result<bool, DecodeError> {
	decode_frecency(bytes).map(|touched| touched.reactions)
}

/// `None` for other settings types. An undecodable preloaded-settings proto refreshes both,
/// so a schema change degrades into an extra read instead of a missed update.
pub fn decode(bytes: &[u8]) -> Result<Option<Touched>, DecodeError> {
	#[derive(Deserialize)]
	struct Update {
		settings: Proto,
	}
	#[derive(Deserialize)]
	struct Proto {
		#[serde(rename = "type")]
		kind: u8,
		proto: String,
	}
	if bytes.len() > MAX_UPDATE_BYTES {
		return Err(DecodeError);
	}
	let update: Update = serde_json::from_slice(bytes).map_err(|_| DecodeError)?;
	if update.settings.kind != PRELOADED_SETTINGS {
		return Ok(None);
	}
	let everything = Touched {
		status: true,
		folders: true,
	};
	let Ok(wire) = STANDARD.decode(update.settings.proto) else {
		return Ok(Some(everything));
	};
	let Ok(fields) = fields(&wire) else {
		return Ok(Some(everything));
	};
	Ok(Some(Touched {
		status: fields.iter().any(|field| field.number == STATUS_FIELD),
		folders: fields.iter().any(|field| field.number == FOLDERS_FIELD),
	}))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::guild_folders::message;

	#[test]
	fn reaction_updates_ignore_favorites_but_refresh_full_and_reaction_settings() {
		let mut reactions = Vec::new();
		message(13, &[], &mut reactions);
		let mut gifs = Vec::new();
		message(2, &[], &mut gifs);
		assert!(reactions_changed(&update(2, &reactions)).unwrap());
		assert!(!reactions_changed(&update(2, &gifs)).unwrap());
		assert!(!reactions_changed(&update(1, &reactions)).unwrap());
		assert!(
			reactions_changed(br#"{"settings":{"type":2,"proto":""},"partial":false}"#).unwrap()
		);
		assert!(reactions_changed(&update(2, &[255])).is_err());
	}

	#[test]
	fn favorite_updates_include_empty_subtrees_and_full_replacements_only_for_type_two() {
		let mut favorites = Vec::new();
		message(2, &[], &mut favorites);
		let mut reactions = Vec::new();
		message(13, &[], &mut reactions);
		assert_eq!(
			decode_frecency(&update(2, &favorites)).unwrap(),
			FrecencyTouched {
				favorites: true,
				reactions: false
			}
		);
		assert_eq!(
			decode_frecency(&update(2, &reactions)).unwrap(),
			FrecencyTouched {
				favorites: false,
				reactions: true
			}
		);
		assert_eq!(
			decode_frecency(&update(1, &favorites)).unwrap(),
			FrecencyTouched::default()
		);
		assert_eq!(
			decode_frecency(&update(2, &[])).unwrap(),
			FrecencyTouched::default()
		);
		assert_eq!(
			decode_frecency(br#"{"settings":{"type":2,"proto":""},"partial":false}"#).unwrap(),
			FrecencyTouched {
				favorites: true,
				reactions: true
			}
		);
	}

	#[test]
	fn malformed_or_oversized_favorite_notifications_are_rejected_locally() {
		for bytes in [
			b"{}".as_slice(),
			br#"{"settings":{"type":2,"proto":"!"}}"#,
			br#"{"settings":{"type":2,"proto":"!"},"partial":false}"#,
			&update(2, &[0xff]),
			&update(2, &[0x10, 1]),
			&update(2, &[0x0a, 0].repeat(4097)),
		] {
			assert!(decode_frecency(bytes).is_err());
		}
		assert!(decode_frecency(&vec![b' '; crate::gif_favorites::MAX_RESPONSE + 1]).is_err());
	}

	fn update(kind: u8, wire: &[u8]) -> Vec<u8> {
		serde_json::to_vec(&serde_json::json!({
			"settings": {"type": kind, "proto": STANDARD.encode(wire)},
			"partial": true,
		}))
		.unwrap()
	}

	#[test]
	fn reports_status_and_folder_changes_only_for_preloaded_settings() {
		let mut status = Vec::new();
		message(11, &[10, 5, 10, 3, b'd', b'n', b'd'], &mut status);
		let mut folders = Vec::new();
		message(14, &[], &mut folders);
		let mut appearance = Vec::new();
		message(9, &[8, 1], &mut appearance);
		assert_eq!(
			decode(&update(1, &status)).unwrap(),
			Some(Touched {
				status: true,
				folders: false
			})
		);
		assert_eq!(
			decode(&update(1, &[folders.clone(), status].concat())).unwrap(),
			Some(Touched {
				status: true,
				folders: true
			})
		);
		assert_eq!(
			decode(&update(1, &appearance)).unwrap(),
			Some(Touched::default())
		);
		assert_eq!(decode(&update(2, &folders)).unwrap(), None);
		assert_eq!(
			decode(&update(1, &[0xff])).unwrap(),
			Some(Touched {
				status: true,
				folders: true
			})
		);
		assert!(decode(b"{}").is_err());
	}
}
