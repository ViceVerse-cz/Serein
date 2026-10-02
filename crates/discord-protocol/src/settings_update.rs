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
