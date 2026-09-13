//! Unofficial settings-proto/1 interoperability; schema extracted by discord-userdoccers.
//! https://github.com/discord-userdoccers/discord-protos/blob/master/discord_protos/discord_users/v1/PreloadedUserSettings.proto
pub use crate::guild_folders::MAX_SETTINGS_RESPONSE;
use crate::{
	DecodeError,
	guild_folders::{fields, integer_wrapper, message},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use model::{
	Id,
	messaging_permissions::{Change, MAX_GUILDS, Snapshot},
};
use serde::Deserialize;
const MAX_SUBTREE: usize = 128 * 1024;
pub struct Settings {
	pub version: u64,
	pub snapshot: Snapshot,
	text: Vec<u8>,
	privacy: Vec<u8>,
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
	let (mut version, mut text, mut privacy) = (None, None, None);
	for field in fields(&wire)? {
		match field.number {
			1 => {
				if version.is_some() {
					return Err(DecodeError);
				}
				let mut data = None;
				for field in fields(field.message()?)? {
					if field.number == 3 {
						if data.is_some() {
							return Err(DecodeError);
						}
						data = Some(u32::try_from(field.integer()?).map_err(|_| DecodeError)?);
					}
				}
				version = Some(u64::from(data.unwrap_or_default()));
			}
			6 | 8 => {
				let target = if field.number == 6 {
					&mut text
				} else {
					&mut privacy
				};
				if target.is_some() {
					return Err(DecodeError);
				}
				let value = field.message()?;
				if value.len() > MAX_SUBTREE {
					return Err(DecodeError);
				}
				*target = Some(value);
			}
			_ => {}
		}
	}
	let text = text.unwrap_or_default();
	let privacy = privacy.unwrap_or_default();
	let mut snapshot = Snapshot::default();
	let mut seen = 0u64;
	for field in fields(text)? {
		if field.number == 27 {
			unique(&mut seen, field.number)?;
			snapshot.spam_filter = u32::try_from(field.integer()?).map_err(|_| DecodeError)?;
		}
	}
	seen = 0;
	let mut restricted_v2 = None;
	for field in fields(privacy)? {
		if matches!(field.number, 4 | 11 | 17 | 26 | 27 | 28 | 33) {
			unique(&mut seen, field.number)?;
		}
		match field.number {
			3 | 16 => {
				let ids = if field.number == 3 {
					&mut snapshot.restricted_guilds
				} else {
					&mut snapshot.unfiltered_guilds
				};
				// Accept both packed and unpacked repeated fixed64 fields.
				let value = if field.raw[0] & 7 == 1 {
					&field.raw[field.raw.len() - 8..]
				} else {
					field.message()?
				};
				if value.len() % 8 != 0 || ids.len() + value.len() / 8 > MAX_GUILDS {
					return Err(DecodeError);
				}
				for bytes in value.as_chunks::<8>().0 {
					let id = Id(u64::from_le_bytes(*bytes));
					if id.0 == 0 {
						return Err(DecodeError);
					}
					ids.push(id);
				}
			}
			4 => snapshot.default_allow_dms = !boolean(field.integer()?)?,
			11 => snapshot.friend_source_flags = wrapper(field.message()?)?,
			17 => {
				snapshot.default_filter_requests = !boolean(u64::from(wrapper(field.message()?)?))?
			}
			26 => snapshot.game_friend_dms = boolean(u64::from(wrapper(field.message()?)?))?,
			27 => restricted_v2 = Some(boolean(u64::from(wrapper(field.message()?)?))?),
			28 => snapshot.game_dms = u32::try_from(field.integer()?).map_err(|_| DecodeError)?,
			33 => snapshot.personalized_requests = !boolean(u64::from(wrapper(field.message()?)?))?,
			_ => {}
		}
	}
	if let Some(restricted) = restricted_v2 {
		snapshot.default_allow_dms = !restricted;
	}
	for ids in [
		&mut snapshot.restricted_guilds,
		&mut snapshot.unfiltered_guilds,
	] {
		ids.sort_unstable();
		ids.dedup();
		ids.shrink_to_fit();
	}
	if !snapshot.valid() {
		return Err(DecodeError);
	}
	Ok(Settings {
		version: version.ok_or(DecodeError)?,
		snapshot,
		text: text.to_vec(),
		privacy: privacy.to_vec(),
	})
}
fn unique(seen: &mut u64, number: u64) -> Result<(), DecodeError> {
	let bit = 1 << number;
	if *seen & bit != 0 {
		return Err(DecodeError);
	}
	*seen |= bit;
	Ok(())
}
fn wrapper(bytes: &[u8]) -> Result<u32, DecodeError> {
	let mut value = None;
	for field in fields(bytes)? {
		if field.number == 1 {
			if value.is_some() {
				return Err(DecodeError);
			}
			value = Some(u32::try_from(field.integer()?).map_err(|_| DecodeError)?);
		}
	}
	Ok(value.unwrap_or_default())
}
fn boolean(value: u64) -> Result<bool, DecodeError> {
	match value {
		0 => Ok(false),
		1 => Ok(true),
		_ => Err(DecodeError),
	}
}
fn integer(number: u64, value: u32, output: &mut Vec<u8>) {
	for mut value in [number << 3, u64::from(value)] {
		while value >= 128 {
			output.push(value as u8 | 0x80);
			value >>= 7;
		}
		output.push(value as u8);
	}
}
pub fn encode_patch(current: &Settings, change: &Change) -> Result<String, DecodeError> {
	if !change.valid() {
		return Err(DecodeError);
	}
	let mut wanted = current.snapshot.clone();
	change.apply(&mut wanted);
	if !wanted.valid() {
		return Err(DecodeError);
	}
	let numbers: &[u64] = match change {
		Change::SpamFilter(_) => &[27],
		Change::DefaultAllowDms(_) => &[4, 27],
		Change::AllowGuildDms(..) => &[3],
		Change::AllowAllDms { .. } => &[3, 4, 27],
		Change::DefaultFilterRequests(_) => &[17],
		Change::FilterGuildRequests(..) => &[16],
		Change::FilterAllRequests { .. } => &[16, 17],
		Change::Everyone(_) | Change::FriendsOfFriends(_) | Change::ServerMembers(_) => &[11],
		Change::PersonalizedRequests(_) => &[33],
		Change::GameFriendDms(_) => &[26],
		Change::GameDms(_) => &[28],
	};
	let text = matches!(change, Change::SpamFilter(_));
	let mut subtree = Vec::new();
	for field in fields(if text {
		&current.text
	} else {
		&current.privacy
	})? {
		if !numbers.contains(&field.number) {
			subtree.extend_from_slice(field.raw);
		}
	}
	for &number in numbers {
		match number {
			3 | 16 => {
				let ids = if number == 3 {
					&wanted.restricted_guilds
				} else {
					&wanted.unfiltered_guilds
				};
				let packed: Vec<_> = ids.iter().flat_map(|id| id.0.to_le_bytes()).collect();
				message(number, &packed, &mut subtree);
			}
			4 => integer(number, u32::from(!wanted.default_allow_dms), &mut subtree),
			27 if text => integer(number, wanted.spam_filter, &mut subtree),
			28 => integer(number, wanted.game_dms, &mut subtree),
			_ => {
				let value = match number {
					11 => wanted.friend_source_flags,
					17 => u32::from(!wanted.default_filter_requests),
					26 => u32::from(wanted.game_friend_dms),
					27 => u32::from(!wanted.default_allow_dms),
					33 => u32::from(!wanted.personalized_requests),
					_ => return Err(DecodeError),
				};
				integer_wrapper(number, u64::from(value), &mut subtree);
			}
		}
	}
	if subtree.len() > MAX_SUBTREE {
		return Err(DecodeError);
	}
	let mut patch = Vec::new();
	message(if text { 6 } else { 8 }, &subtree, &mut patch);
	Ok(STANDARD.encode(patch))
}
