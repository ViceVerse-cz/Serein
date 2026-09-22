//! Unofficial settings-proto/1 status.show_current_game interoperability.
//! Schema: discord-userdoccers/discord-protos discord_users/v1/PreloadedUserSettings.proto.
use crate::{
	DecodeError,
	guild_folders::{fields, fixed64_field, integer_wrapper, message},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;

pub use crate::guild_folders::MAX_SETTINGS_RESPONSE;
const MAX_STATUS_BYTES: usize = 16 * 1024;

pub struct Settings {
	pub version: u64,
	pub enabled: bool,
	status_wire: Vec<u8>,
}

pub fn decode_response(bytes: &[u8]) -> Result<Settings, DecodeError> {
	#[derive(Deserialize)]
	struct Response {
		settings: String,
		#[serde(default)]
		out_of_date: bool,
	}
	if bytes.len() > MAX_SETTINGS_RESPONSE {
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
	let mut status = None;
	for field in fields(&wire)? {
		match field.number {
			1 => {
				if version.is_some() {
					return Err(DecodeError);
				}
				let mut data_version = None;
				for field in fields(field.message()?)? {
					if field.number == 3 {
						if data_version.is_some() {
							return Err(DecodeError);
						}
						let value = field.integer()?;
						if value > u32::MAX.into() {
							return Err(DecodeError);
						}
						data_version = Some(value);
					}
				}
				version = Some(data_version.unwrap_or_default());
			}
			11 => {
				if status.is_some() {
					return Err(DecodeError);
				}
				let value = field.message()?;
				if value.len() > MAX_STATUS_BYTES {
					return Err(DecodeError);
				}
				status = Some(value);
			}
			_ => {}
		}
	}
	let status = status.unwrap_or_default();
	let mut enabled = None;
	for field in fields(status)? {
		if field.number == 3 {
			if enabled.is_some() {
				return Err(DecodeError);
			}
			let mut value = None;
			for field in fields(field.message()?)? {
				if field.number == 1 {
					if value.is_some() {
						return Err(DecodeError);
					}
					value = Some(match field.integer()? {
						0 => false,
						1 => true,
						_ => return Err(DecodeError),
					});
				}
			}
			enabled = Some(value.unwrap_or(false));
		}
	}
	Ok(Settings {
		version: version.ok_or(DecodeError)?,
		enabled: enabled.unwrap_or(true),
		status_wire: status.into(),
	})
}

/// Replace only this preference; retain status, custom status and unknown status fields.
pub fn encode_patch(current: &Settings, enabled: bool) -> Result<String, DecodeError> {
	let mut status = Vec::new();
	for field in fields(&current.status_wire)? {
		if field.number != 3 {
			status.extend_from_slice(field.raw);
		}
	}
	integer_wrapper(3, u64::from(enabled), &mut status);
	if status.len() > MAX_STATUS_BYTES {
		return Err(DecodeError);
	}
	let mut patch = Vec::new();
	message(11, &status, &mut patch);
	Ok(STANDARD.encode(patch))
}

/// Status and custom status from the same settings field activity sharing already preserves.
pub fn account_presence(settings: &Settings) -> Result<model::OwnPresence, DecodeError> {
	let mut status = None;
	let mut custom = None;
	for field in fields(&settings.status_wire)? {
		match field.number {
			1 => {
				if status.is_some() {
					return Err(DecodeError);
				}
				let wire = string_value(field.message()?)?;
				status = Some(if wire.is_empty() {
					model::PresenceStatus::Online
				} else {
					model::PresenceStatus::parse(&wire).ok_or(DecodeError)?
				});
			}
			2 => {
				if custom.is_some() {
					return Err(DecodeError);
				}
				custom = Some(custom_status(field.message()?)?);
			}
			_ => {}
		}
	}
	let (custom_status, mut expires_at_ms) = custom.unwrap_or_default();
	if custom_status.is_empty() {
		expires_at_ms = None;
	}
	let presence = model::OwnPresence {
		status: status.unwrap_or_default(),
		custom_status,
		expires_at_ms,
	};
	if !presence.custom_status.is_empty() && !presence.valid() {
		return Err(DecodeError);
	}
	Ok(presence)
}

/// Replace status and custom status. Game sharing and unknown status fields stay.
pub fn encode_account_presence(
	current: &Settings,
	presence: &model::OwnPresence,
) -> Result<String, DecodeError> {
	if !presence.valid() {
		return Err(DecodeError);
	}
	let mut status = Vec::new();
	let mut previous_custom: Option<Vec<u8>> = None;
	let mut previous_text = String::new();
	let mut previous_expires = None;
	let mut previous_status = String::new();
	let mut status_expiry: Option<Vec<u8>> = None;
	for field in fields(&current.status_wire)? {
		match field.number {
			1 => previous_status = string_value(field.message()?)?,
			4 => {
				if status_expiry.is_some() {
					return Err(DecodeError);
				}
				status_expiry = Some(field.raw.to_vec());
			}
			2 => {
				if previous_custom.is_some() {
					return Err(DecodeError);
				}
				let (text, expires) = custom_status(field.message()?)?;
				previous_text = text;
				previous_expires = expires;
				previous_custom = Some(field.raw.to_vec());
			}
			_ => status.extend_from_slice(field.raw),
		}
	}
	string_field(1, presence.status.wire(), &mut status);
	if previous_status == presence.status.wire()
		&& let Some(raw) = status_expiry
	{
		status.extend_from_slice(&raw);
	}
	let text_same = previous_text == presence.custom_status;
	let expires_same = previous_expires == presence.expires_at_ms;
	if (text_same && expires_same)
		|| (presence.custom_status.is_empty() && previous_text.is_empty())
	{
		if let Some(raw) = previous_custom {
			status.extend_from_slice(&raw);
		}
	} else if !presence.custom_status.is_empty() {
		let mut custom = Vec::new();
		if let Some(raw) = &previous_custom {
			for field in fields(message_body(raw)?)? {
				if field.number != 1 && field.number != 4 {
					custom.extend_from_slice(field.raw);
				}
			}
		}
		message(1, presence.custom_status.as_bytes(), &mut custom);
		if let Some(expires) = presence.expires_at_ms {
			fixed64_field(4, expires, &mut custom);
		}
		message(2, &custom, &mut status);
	}
	if status.len() > MAX_STATUS_BYTES {
		return Err(DecodeError);
	}
	let mut patch = Vec::new();
	message(11, &status, &mut patch);
	Ok(STANDARD.encode(patch))
}

fn string_value(bytes: &[u8]) -> Result<String, DecodeError> {
	let mut text = None;
	for field in fields(bytes)? {
		if field.number == 1 {
			if text.is_some() {
				return Err(DecodeError);
			}
			text = Some(
				std::str::from_utf8(field.message()?)
					.map_err(|_| DecodeError)?
					.to_owned(),
			);
		}
	}
	Ok(text.unwrap_or_default())
}

fn custom_status(bytes: &[u8]) -> Result<(String, Option<u64>), DecodeError> {
	let mut text = None;
	let mut expires = None;
	for field in fields(bytes)? {
		match field.number {
			1 => {
				if text.is_some() {
					return Err(DecodeError);
				}
				let value = std::str::from_utf8(field.message()?).map_err(|_| DecodeError)?;
				if value.len() > 512 {
					return Err(DecodeError);
				}
				text = Some(value.to_owned());
			}
			4 => {
				if expires.is_some() {
					return Err(DecodeError);
				}
				let value = field.fixed64()?;
				expires = Some(value).filter(|value| *value != 0);
			}
			_ => {}
		}
	}
	Ok((text.unwrap_or_default(), expires))
}

fn string_field(number: u64, text: &str, output: &mut Vec<u8>) {
	let mut wrapper = Vec::new();
	message(1, text.as_bytes(), &mut wrapper);
	message(number, &wrapper, output);
}

/// `raw` is a length-delimited field, tag included. Return its message body.
fn message_body(raw: &[u8]) -> Result<&[u8], DecodeError> {
	let field = fields(raw)?.pop().ok_or(DecodeError)?;
	field.message()
}

#[cfg(test)]
mod tests {
	use super::*;
	fn response(wire: &[u8]) -> Vec<u8> {
		serde_json::to_vec(&serde_json::json!({"settings":STANDARD.encode(wire)})).unwrap()
	}
	#[test]
	fn sharing_patch_preserves_status_unknown_fields_and_defaults() {
		let mut status = vec![10, 5, 10, 3, b'd', b'n', b'd'];
		message(2, &[10, 4, b'B', b'u', b's', b'y'], &mut status);
		message(55, b"future", &mut status);
		let preserved = status.clone();
		integer_wrapper(3, 0, &mut status);
		let mut wire = vec![10, 2, 24, 7];
		message(11, &status, &mut wire);
		let current = decode_response(&response(&wire)).unwrap();
		assert_eq!((current.version, current.enabled), (7, false));
		let parsed = account_presence(&current).unwrap();
		assert_eq!(
			(
				parsed.status,
				parsed.custom_status.as_str(),
				parsed.expires_at_ms
			),
			(model::PresenceStatus::DoNotDisturb, "Busy", None)
		);
		let edited = model::OwnPresence {
			status: model::PresenceStatus::Idle,
			custom_status: "Away".into(),
			expires_at_ms: Some(1_700_000_000_000),
		};
		let patch = STANDARD
			.decode(encode_account_presence(&current, &edited).unwrap())
			.unwrap();
		let mut roundtrip = vec![10, 2, 24, 7];
		roundtrip.extend_from_slice(&patch);
		let saved = decode_response(&response(&roundtrip)).unwrap();
		assert!(!saved.enabled);
		assert_eq!(account_presence(&saved).unwrap(), edited);
		let patch = STANDARD
			.decode(encode_patch(&current, true).unwrap())
			.unwrap();
		let root = fields(&patch).unwrap();
		assert_eq!(root.len(), 1);
		assert_eq!(root[0].number, 11);
		assert!(root[0].message().unwrap().starts_with(&preserved));
		let mut saved = vec![10, 2, 24, 8];
		saved.extend_from_slice(&patch);
		assert!(decode_response(&response(&saved)).unwrap().enabled);
		assert!(decode_response(&response(&[10, 0])).unwrap().enabled);
		assert!(
			!decode_response(&response(&[10, 0, 90, 2, 26, 0]))
				.unwrap()
				.enabled
		);
		for invalid in [
			vec![],
			vec![10, 0, 10, 0],
			vec![10, 0, 90, 4, 26, 2, 8, 2],
			vec![10, 0, 90, 3, 26, 2, 8],
		] {
			assert!(decode_response(&response(&invalid)).is_err());
		}
		assert!(decode_response(&vec![b' '; MAX_SETTINGS_RESPONSE + 1]).is_err());
		let mut oversized = vec![10, 0];
		message(11, &vec![0; MAX_STATUS_BYTES + 1], &mut oversized);
		assert!(decode_response(&response(&oversized)).is_err());
		assert!(decode_response(br#"{"settings":"CgA=","out_of_date":true}"#).is_err());
	}
}
