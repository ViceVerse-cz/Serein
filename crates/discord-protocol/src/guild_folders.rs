//! Unofficial settings-proto/1 interoperability, not a documented bot API.
//! Wire schema: discord-userdoccers/discord-protos, PreloadedUserSettings.proto.
use crate::DecodeError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use model::{
	Id,
	guild_folders::{Folder, MAX_FOLDERS, MAX_GUILDS, Settings},
};
use serde::Deserialize;

// Discord accepts a 5 MiB base64 settings value; leave bounded room for its JSON envelope.
pub const MAX_SETTINGS_RESPONSE: usize = 6 * 1024 * 1024;
const MAX_FOLDER_WIRE: usize = 128 * 1024;

pub struct Decoded {
	pub settings: Settings,
	folder_wire: Vec<u8>,
}

#[derive(Deserialize)]
struct Response {
	settings: String,
	#[serde(default)]
	out_of_date: bool,
}

pub fn decode_response(bytes: &[u8]) -> Result<Decoded, DecodeError> {
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
	let mut settings = Settings::default();
	let mut folder_wire = Vec::new();
	let mut version_seen = false;
	let mut folders_seen = false;
	for field in fields(&wire)? {
		match field.number {
			1 => {
				if version_seen {
					return Err(DecodeError);
				}
				version_seen = true;
				for version in fields(field.message()?)? {
					if version.number == 3 {
						settings.version = version.integer()?;
					}
				}
			}
			14 => {
				if folders_seen {
					return Err(DecodeError);
				}
				folders_seen = true;
				let value = field.message()?;
				if value.len() > MAX_FOLDER_WIRE {
					return Err(DecodeError);
				}
				folder_wire = value.to_vec();
				for folder in fields(value)? {
					if folder.number == 1 {
						if settings.folders.len() >= MAX_FOLDERS {
							return Err(DecodeError);
						}
						settings.folders.push(decode_folder(folder.message()?)?);
					}
				}
			}
			_ => {}
		}
	}
	settings.folders.shrink_to_fit();
	for folder in &mut settings.folders {
		folder.guild_ids.shrink_to_fit();
	}
	if !version_seen || !settings.valid() {
		return Err(DecodeError);
	}
	Ok(Decoded {
		settings,
		folder_wire,
	})
}

fn decode_folder(bytes: &[u8]) -> Result<Folder, DecodeError> {
	let mut folder = Folder::default();
	for field in fields(bytes)? {
		match field.number {
			1 => {
				let packed = if field.wire_type == 1 {
					field.value
				} else {
					field.message()?
				};
				if packed.len() % 8 != 0 || folder.guild_ids.len() + packed.len() / 8 > MAX_GUILDS {
					return Err(DecodeError);
				}
				for id in packed.as_chunks::<8>().0 {
					folder.guild_ids.push(Id(u64::from_le_bytes(*id)));
				}
			}
			2 => folder.id = Some(wrapper_integer(field.message()?)?),
			3 => {
				let mut name = String::new();
				for value in fields(field.message()?)? {
					if value.number == 1 {
						name = std::str::from_utf8(value.message()?)
							.map_err(|_| DecodeError)?
							.to_owned();
					}
				}
				folder.name = Some(name);
			}
			4 => {
				folder.color = Some(
					u32::try_from(wrapper_integer(field.message()?)?).map_err(|_| DecodeError)?,
				)
			}
			_ => {}
		}
	}
	Ok(folder)
}

fn wrapper_integer(bytes: &[u8]) -> Result<u64, DecodeError> {
	let mut number = 0;
	for field in fields(bytes)? {
		if field.number == 1 {
			number = field.integer()?;
		}
	}
	Ok(number)
}

/// Replaces only folder entries. Existing positions and unknown subtree/entry fields survive.
/// The caller must send `required_data_version` with the version of this fresh read.
pub fn encode_patch(current: &Decoded, settings: &Settings) -> Result<String, DecodeError> {
	if !settings.valid() || settings.version != current.settings.version {
		return Err(DecodeError);
	}
	let old_fields = fields(&current.folder_wire)?;
	let mut subtree = Vec::new();
	for field in &old_fields {
		if field.number != 1 {
			subtree.extend_from_slice(field.raw);
		}
	}
	for folder in &settings.folders {
		let mut encoded = Vec::new();
		// ponytail: at most 200 folders; index by identity if this account bound grows.
		for (old, field) in current
			.settings
			.folders
			.iter()
			.zip(old_fields.iter().filter(|field| field.number == 1))
		{
			if (folder.id.is_some() && old.id == folder.id)
				|| (folder.id.is_none() && old.id.is_none() && old.guild_ids == folder.guild_ids)
			{
				for unknown in fields(field.message()?)? {
					if !(1..=4).contains(&unknown.number) {
						encoded.extend_from_slice(unknown.raw);
					}
				}
			}
		}
		let ids: Vec<u8> = folder
			.guild_ids
			.iter()
			.flat_map(|id| id.0.to_le_bytes())
			.collect();
		message(1, &ids, &mut encoded);
		if let Some(id) = folder.id {
			integer_wrapper(2, id, &mut encoded);
		}
		if let Some(name) = &folder.name {
			let mut wrapper = Vec::new();
			message(1, name.as_bytes(), &mut wrapper);
			message(3, &wrapper, &mut encoded);
		}
		if let Some(color) = folder.color {
			integer_wrapper(4, color.into(), &mut encoded);
		}
		message(1, &encoded, &mut subtree);
	}
	if subtree.len() > MAX_FOLDER_WIRE {
		return Err(DecodeError);
	}
	let mut patch = Vec::new();
	message(14, &subtree, &mut patch);
	Ok(STANDARD.encode(patch))
}

// Shared bounded settings-proto wire helpers, also used for activity sharing.
pub(crate) struct Field<'a> {
	pub(crate) number: u64,
	wire_type: u8,
	value: &'a [u8],
	pub(crate) raw: &'a [u8],
}
impl<'a> Field<'a> {
	pub(crate) fn message(&self) -> Result<&'a [u8], DecodeError> {
		if self.wire_type == 2 {
			Ok(self.value)
		} else {
			Err(DecodeError)
		}
	}
	pub(crate) fn integer(&self) -> Result<u64, DecodeError> {
		if self.wire_type != 0 {
			return Err(DecodeError);
		}
		let mut value = self.value;
		varint(&mut value)
	}
	pub(crate) fn fixed64(&self) -> Result<u64, DecodeError> {
		if self.wire_type == 1 && self.value.len() == 8 {
			let bytes: [u8; 8] = self.value.try_into().map_err(|_| DecodeError)?;
			Ok(u64::from_le_bytes(bytes))
		} else {
			Err(DecodeError)
		}
	}
}
fn varint(input: &mut &[u8]) -> Result<u64, DecodeError> {
	let mut value = 0;
	for shift in (0..70).step_by(7) {
		let (&byte, rest) = input.split_first().ok_or(DecodeError)?;
		*input = rest;
		if shift == 63 && byte > 1 {
			return Err(DecodeError);
		}
		value |= u64::from(byte & 0x7f) << shift;
		if byte < 128 {
			return Ok(value);
		}
	}
	Err(DecodeError)
}
pub(crate) fn fields(mut bytes: &[u8]) -> Result<Vec<Field<'_>>, DecodeError> {
	let mut result = Vec::new();
	while !bytes.is_empty() {
		if result.len() >= 4096 {
			return Err(DecodeError);
		}
		let start = bytes;
		let tag = varint(&mut bytes)?;
		let number = tag >> 3;
		if number == 0 || number > 0x1fffffff {
			return Err(DecodeError);
		}
		let wire_type = (tag & 7) as u8;
		let value = match wire_type {
			0 => {
				let before = bytes;
				varint(&mut bytes)?;
				&before[..before.len() - bytes.len()]
			}
			1 | 2 | 5 => {
				let length = match wire_type {
					1 => 8,
					5 => 4,
					_ => usize::try_from(varint(&mut bytes)?).map_err(|_| DecodeError)?,
				};
				let value = bytes.get(..length).ok_or(DecodeError)?;
				bytes = &bytes[length..];
				value
			}
			_ => return Err(DecodeError),
		};
		result.push(Field {
			number,
			wire_type,
			value,
			raw: &start[..start.len() - bytes.len()],
		});
	}
	Ok(result)
}
fn write_varint(mut value: u64, output: &mut Vec<u8>) {
	while value >= 128 {
		output.push(value as u8 | 0x80);
		value >>= 7;
	}
	output.push(value as u8);
}
pub(crate) fn message(number: u64, value: &[u8], output: &mut Vec<u8>) {
	write_varint(number << 3 | 2, output);
	write_varint(value.len() as u64, output);
	output.extend_from_slice(value);
}
pub(crate) fn integer_wrapper(number: u64, value: u64, output: &mut Vec<u8>) {
	let mut wrapper = vec![8];
	write_varint(value, &mut wrapper);
	message(number, &wrapper, output);
}
pub(crate) fn fixed64_field(number: u64, value: u64, output: &mut Vec<u8>) {
	write_varint(number << 3 | 1, output);
	output.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn folders_roundtrip_preserves_positions_and_rejects_truncation() {
		let mut subtree = Vec::new();
		message(2, &42u64.to_le_bytes(), &mut subtree);
		message(9, b"future", &mut subtree);
		let current = Decoded {
			settings: Settings::default(),
			folder_wire: subtree.clone(),
		};
		let settings = Settings {
			folders: vec![Folder {
				id: Some(7),
				guild_ids: vec![Id(42)],
				name: Some("Projects".into()),
				color: Some(0),
			}],
			version: 0,
		};
		let patch = encode_patch(&current, &settings).unwrap();
		let mut wire = vec![10, 0]; // Present version message; data version defaults to zero.
		wire.extend(STANDARD.decode(patch).unwrap());
		let response = serde_json::json!({"settings": STANDARD.encode(&wire)});
		let decoded = decode_response(response.to_string().as_bytes()).unwrap();
		assert_eq!(decoded.settings, settings);
		assert!(decoded.folder_wire.starts_with(&subtree));
		wire.pop();
		assert!(
			decode_response(
				serde_json::json!({"settings": STANDARD.encode(wire)})
					.to_string()
					.as_bytes()
			)
			.is_err()
		);
		assert!(varint(&mut &[255; 10][..]).is_err());
	}

	#[test]
	fn large_account_settings_reach_the_folder_decoder() {
		let mut wire = vec![10, 0]; // Present version message; data version defaults to zero.
		message(99, &vec![0; 800 * 1024], &mut wire);
		let response = serde_json::json!({"settings": STANDARD.encode(wire)})
			.to_string()
			.into_bytes();
		assert!(response.len() > 1024 * 1024);
		assert!(decode_response(&response).is_ok());
	}
}
