//! Unofficial FrecencyUserSettings emoji rankings: `emoji_frecency` (6) for messages and
//! `emoji_reaction_frecency` (13) for reactions, read with the GIF favorites. Keys are a custom
//! emoji id or, unverified, a Unicode emoji's Discord name. Saves rewrite only the used entries;
//! every other entry and field is kept byte for byte.
use crate::{
	DecodeError,
	gif_favorites::{Decoded, MAX_ENTRIES, MAX_SUBTREE},
	guild_folders::{fields, message, varint, write_varint},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Ranked keys handed to the client; the pickers show far fewer.
pub const MAX_FREQUENT_EMOJI: usize = 32;
pub const MAX_EMOJI_KEY: usize = 64;
/// Newest use times kept per entry, as the score only weighs recent uses.
pub const MAX_RECENT_USES: usize = 10;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
	Message,
	Reaction,
}
impl Kind {
	fn slot(self) -> usize {
		usize::from(self == Self::Reaction)
	}
	fn field(self) -> u64 {
		match self {
			Self::Message => 6,
			Self::Reaction => 13,
		}
	}
}

/// Uses of one emoji since the last save, with their Unix times in milliseconds.
pub struct Use<'a> {
	pub key: &'a str,
	pub kind: Kind,
	pub count: u32,
	pub times: &'a [u64],
}

pub fn valid_key(key: &str) -> bool {
	!key.is_empty() && key.len() <= MAX_EMOJI_KEY && !key.chars().any(char::is_control)
}

/// One `FrecencyItem`, with the fields this client does not rewrite kept raw.
struct Item {
	total: u32,
	recent: Vec<u64>,
	score: i32,
	rest: Vec<u8>,
}

fn item(bytes: &[u8]) -> Option<Item> {
	let mut item = Item {
		total: 0,
		recent: Vec::new(),
		score: 0,
		rest: Vec::new(),
	};
	// Every time is read and validated; only the newest are kept.
	let keep = |recent: &mut Vec<u64>, at: u64| {
		recent.push(at);
		if recent.len() > 4 * MAX_RECENT_USES {
			newest(recent);
		}
	};
	for field in fields(bytes).ok()? {
		match field.number {
			1 => item.total = u32::try_from(field.integer().ok()?).unwrap_or(u32::MAX),
			// Repeated uint64, packed or not.
			2 => match field.message() {
				Ok(mut packed) => {
					while !packed.is_empty() {
						keep(&mut item.recent, varint(&mut packed).ok()?);
					}
				}
				Err(_) => keep(&mut item.recent, field.integer().ok()?),
			},
			// int32, sign-extended on the wire.
			4 => item.score = field.integer().ok()? as i32,
			_ => item.rest.extend_from_slice(field.raw),
		}
	}
	newest(&mut item.recent);
	Some(item)
}

fn newest(recent: &mut Vec<u64>) {
	recent.sort_unstable();
	let excess = recent.len().saturating_sub(MAX_RECENT_USES);
	recent.drain(..excess);
}

/// One `emojis` map entry as (key, value). The last key wins, as in protobuf; a repeated value
/// would merge on the service, so it is `None` and the entry is never rewritten.
fn entry(bytes: &[u8]) -> Result<(&[u8], Option<&[u8]>), DecodeError> {
	let mut key = &[][..];
	let mut value = Some(&[][..]);
	let mut seen_value = false;
	for field in fields(bytes)? {
		match field.number {
			1 => key = field.message()?,
			2 => {
				let bytes = field.message()?;
				value = (!std::mem::replace(&mut seen_value, true)).then_some(bytes);
			}
			_ => {}
		}
	}
	Ok((key, value))
}

/// A readable entry with a valid key, for ranking.
fn readable(bytes: &[u8]) -> Option<(&str, Item)> {
	let (key, value) = entry(bytes).ok()?;
	let key = std::str::from_utf8(key).ok().filter(|key| valid_key(key))?;
	Some((key, item(value?)?))
}

/// Approximates the official client's frecency: total uses scaled by how recent the newest
/// uses are (unverified weights).
pub fn score(total: u32, recent: &[u64], now: u64) -> i32 {
	if recent.is_empty() {
		return 0;
	}
	let weight: u64 = recent
		.iter()
		.map(|&at| match now.saturating_sub(at) / DAY_MS {
			0..=3 => 100,
			4..=15 => 70,
			16..=30 => 50,
			31..=45 => 30,
			46..=80 => 10,
			_ => 0,
		})
		.sum();
	i32::try_from(u64::from(total) * weight / recent.len() as u64).unwrap_or(i32::MAX)
}

impl Decoded {
	/// Keys most used first, by the stored score and then total uses. Malformed entries are
	/// skipped, and an unreadable ranking is empty.
	pub fn frequent_emoji(&self, kind: Kind) -> Vec<String> {
		let Some(Ok(entries)) = self.emoji[kind.slot()].as_deref().map(fields) else {
			return Vec::new();
		};
		let mut ranked: Vec<(&str, i32, u32)> = Vec::new();
		for field in entries {
			if field.number == 1
				&& let Ok(bytes) = field.message()
				&& let Some((key, item)) = readable(bytes)
			{
				// A repeated map key takes its last value, as the service does.
				let row = (key, item.score, item.total);
				if let Some(known) = ranked.iter_mut().find(|(known, ..)| *known == key) {
					*known = row;
				} else if ranked.len() < MAX_ENTRIES {
					ranked.push(row);
				}
			}
		}
		ranked.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(b.0)));
		ranked
			.into_iter()
			.take(MAX_FREQUENT_EMOJI)
			.map(|(key, ..)| key.to_owned())
			.collect()
	}
}

fn encode_item(item: &Item, output: &mut Vec<u8>) {
	write_varint(1 << 3, output);
	write_varint(u64::from(item.total), output);
	let mut packed = Vec::new();
	for &at in &item.recent {
		write_varint(at, &mut packed);
	}
	message(2, &packed, output);
	write_varint(4 << 3, output);
	write_varint(i64::from(item.score) as u64, output);
	output.extend_from_slice(&item.rest);
}

/// Adds `used` to `item` and writes the map entry.
fn encode_entry(used: &Use<'_>, mut item: Item, now: u64, output: &mut Vec<u8>) {
	item.total = item.total.saturating_add(used.count);
	item.recent.extend_from_slice(used.times);
	newest(&mut item.recent);
	item.score = score(item.total, &item.recent, now);
	let mut value = Vec::new();
	encode_item(&item, &mut value);
	let mut encoded = Vec::new();
	message(1, used.key.as_bytes(), &mut encoded);
	message(2, &value, &mut encoded);
	message(1, &encoded, output);
}

/// A patch replacing only the emoji subtrees these uses touch, against a fresh read.
pub fn encode_uses(current: &Decoded, uses: &[Use<'_>], now: u64) -> Result<String, DecodeError> {
	let mut patch = Vec::new();
	for kind in [Kind::Message, Kind::Reaction] {
		let uses: Vec<_> = uses
			.iter()
			.filter(|used| used.kind == kind && used.count > 0 && valid_key(used.key))
			.collect();
		if uses.is_empty() {
			continue;
		}
		let existing = current.emoji[kind.slot()].as_deref().ok_or(DecodeError)?;
		let mut subtree = Vec::with_capacity(existing.len() + uses.len() * 64);
		let mut written = vec![false; uses.len()];
		let mut entries = 0;
		for field in fields(existing)? {
			if field.number == 1 {
				entries += 1;
				// An entry whose key cannot be read might be one of these; refuse the save.
				let (key, value) = entry(field.message()?)?;
				if let Some(index) = uses.iter().position(|used| used.key.as_bytes() == key) {
					written[index] = true;
					// An entry this client cannot fully read is kept rather than replaced.
					if let Some(item) = value.and_then(item) {
						encode_entry(uses[index], item, now, &mut subtree);
						continue;
					}
				}
			}
			subtree.extend_from_slice(field.raw);
		}
		for (used, _) in uses.iter().zip(written).filter(|(_, written)| !written) {
			if entries >= MAX_ENTRIES {
				break;
			}
			let fresh = Item {
				total: 0,
				recent: Vec::new(),
				score: 0,
				rest: Vec::new(),
			};
			encode_entry(used, fresh, now, &mut subtree);
			entries += 1;
		}
		if subtree.len() > MAX_SUBTREE {
			return Err(DecodeError);
		}
		message(kind.field(), &subtree, &mut patch);
	}
	if patch.is_empty() {
		return Err(DecodeError);
	}
	Ok(STANDARD.encode(patch))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::gif_favorites::decode_response;

	fn item_bytes(total: u64, score: i32, extra: &[u8]) -> Vec<u8> {
		let mut item = Vec::new();
		write_varint(1 << 3, &mut item);
		write_varint(total, &mut item);
		write_varint(4 << 3, &mut item);
		write_varint(i64::from(score) as u64, &mut item);
		item.extend_from_slice(extra);
		item
	}
	fn entry(key: &[u8], item: &[u8], output: &mut Vec<u8>) {
		let mut entry = Vec::new();
		message(1, key, &mut entry);
		message(2, item, &mut entry);
		message(1, &entry, output);
	}
	fn decoded(wire: &[u8]) -> Decoded {
		decode_response(
			serde_json::json!({ "settings": STANDARD.encode(wire) })
				.to_string()
				.as_bytes(),
		)
		.unwrap()
	}

	#[test]
	fn rankings_skip_malformed_entries_and_never_block_gif_favorites() {
		let mut emojis = Vec::new();
		entry(b"thumbsup", &item_bytes(3, 500, &[]), &mut emojis);
		// A repeated map key takes its last value.
		entry(b"thumbsup", &item_bytes(3, 50, &[]), &mut emojis);
		entry(b"1234567890123", &item_bytes(9, 200, &[]), &mut emojis);
		entry(b"skull", &item_bytes(1, -1, &[]), &mut emojis);
		entry(&[b'x'; MAX_EMOJI_KEY + 1], &[], &mut emojis);
		entry(b"bad\nkey", &[], &mut emojis);
		entry(&[0xff], &[], &mut emojis);
		message(1, b"\x0a\x09short", &mut emojis);
		let mut reactions = Vec::new();
		entry(b"fire", &item_bytes(1, 5, &[]), &mut reactions);
		let mut wire = Vec::new();
		message(6, &emojis, &mut wire);
		message(13, &reactions, &mut wire);
		let current = decoded(&wire);
		assert_eq!(
			current.frequent_emoji(Kind::Message),
			["1234567890123", "thumbsup", "skull"]
		);
		assert_eq!(current.frequent_emoji(Kind::Reaction), ["fire"]);

		let mut wire = Vec::new();
		write_varint(6 << 3, &mut wire);
		write_varint(1, &mut wire);
		message(2, &[], &mut wire);
		let unreadable = decoded(&wire);
		assert!(unreadable.frequent_emoji(Kind::Message).is_empty());
		assert!(unreadable.favorites().unwrap().is_empty());
		let used = Use {
			key: "joy",
			kind: Kind::Message,
			count: 1,
			times: &[1],
		};
		// Never replace a ranking that could not be read.
		assert!(encode_uses(&unreadable, &[used], 1).is_err());
	}

	#[test]
	fn saves_rewrite_only_used_entries_and_keep_unknown_fields() {
		let now = 100 * DAY_MS;
		let mut emojis = Vec::new();
		let mut future = Vec::new();
		message(9, b"future item field", &mut future);
		entry(b"joy", &item_bytes(4, 10, &future), &mut emojis);
		entry(b"skull", &item_bytes(2, 70, &[]), &mut emojis);
		message(5, b"future map field", &mut emojis);
		let mut wire = Vec::new();
		message(2, &[], &mut wire);
		message(6, &emojis, &mut wire);
		message(3, b"untouched sticker settings", &mut wire);
		let current = decoded(&wire);
		let times: Vec<u64> = (0..12).map(|day| now - day * DAY_MS).collect();
		let uses = [
			Use {
				key: "joy",
				kind: Kind::Message,
				count: 12,
				times: &times,
			},
			Use {
				key: "1234567890123",
				kind: Kind::Message,
				count: 1,
				times: &[now],
			},
			Use {
				key: "fire",
				kind: Kind::Reaction,
				count: 1,
				times: &[now],
			},
		];
		let patch = STANDARD
			.decode(encode_uses(&current, &uses, now).unwrap())
			.unwrap();
		let top = fields(&patch).unwrap();
		assert_eq!(
			top.iter().map(|field| field.number).collect::<Vec<_>>(),
			[6, 13]
		);
		let subtree = top[0].message().unwrap();
		let parts = fields(subtree).unwrap();
		assert_eq!(parts.len(), 4);
		// The unused entry and the unknown map field are copied verbatim.
		assert_eq!(parts[1].raw, fields(&emojis).unwrap()[1].raw);
		assert_eq!(parts[2].raw, fields(&emojis).unwrap()[2].raw);
		let (key, joy) = readable(parts[0].message().unwrap()).unwrap();
		assert_eq!(key, "joy");
		assert_eq!(joy.total, 16);
		assert_eq!(joy.recent.len(), MAX_RECENT_USES);
		assert_eq!(joy.recent[MAX_RECENT_USES - 1], now);
		assert_eq!(joy.score, score(16, &joy.recent, now));
		assert_eq!(joy.rest, future);

		let mut saved = Vec::new();
		message(6, subtree, &mut saved);
		message(13, top[1].message().unwrap(), &mut saved);
		let saved = decoded(&saved);
		assert_eq!(saved.frequent_emoji(Kind::Message)[0], "joy");
		assert_eq!(saved.frequent_emoji(Kind::Reaction), ["fire"]);
	}

	#[test]
	fn saves_keep_entries_they_cannot_fully_read_and_all_packed_times() {
		let now = 100 * DAY_MS;
		// More packed times than any fixed cap: the newest still survive a save.
		let mut packed = Vec::new();
		for at in 0..2000 {
			write_varint(at, &mut packed);
		}
		let mut long = Vec::new();
		message(2, &packed, &mut long);
		let mut emojis = Vec::new();
		entry(b"skull", &item_bytes(3, 9, &long), &mut emojis);
		// A repeated value message merges on the service, so this entry is never rewritten.
		let mut merged = Vec::new();
		message(1, b"joy", &mut merged);
		message(2, &item_bytes(100, 1, &[]), &mut merged);
		message(2, &item_bytes(0, 10, &[]), &mut merged);
		message(1, &merged, &mut emojis);
		let mut wire = Vec::new();
		message(6, &emojis, &mut wire);
		let current = decoded(&wire);
		let times = [now];
		let uses = ["skull", "joy"].map(|key| Use {
			key,
			kind: Kind::Message,
			count: 1,
			times: &times,
		});
		let patch = STANDARD
			.decode(encode_uses(&current, &uses, now).unwrap())
			.unwrap();
		let top = fields(&patch).unwrap();
		let parts = fields(top[0].message().unwrap()).unwrap();
		assert_eq!(parts.len(), 2);
		assert_eq!(parts[1].raw, fields(&emojis).unwrap()[1].raw);
		let (_, skull) = readable(parts[0].message().unwrap()).unwrap();
		assert_eq!(skull.recent.first(), Some(&1991));
		assert_eq!(skull.recent.last(), Some(&now));

		let mut broken = Vec::new();
		message(1, b"\x0a\x09short", &mut broken);
		let mut wire = Vec::new();
		message(6, &broken, &mut wire);
		assert!(encode_uses(&decoded(&wire), &uses, now).is_err());
	}
}
