//! Shared message-presentation data: loaded role colours, timeouts, reply times and tone
//! indicators. Everything here is derived from session state; nothing performs a fetch.
use client_core::State;
use egui::Color32;
use model::{Id, server_roles::Colors};

/// Role colours for `user`, using live member roles before the supplied fallback.
pub(crate) fn role_colors(
	state: &State,
	channel: Id,
	user: Id,
	fallback_roles: &[Id],
) -> Option<Colors> {
	state.role_colors(channel, user, fallback_roles)
}

/// A role colour adjusted for `background`, or `fallback` when no loaded role colour exists.
pub(crate) fn role_text_color(
	state: &State,
	channel: Id,
	user: Id,
	fallback_roles: &[Id],
	background: Color32,
	fallback: Color32,
) -> Color32 {
	role_colors(state, channel, user, fallback_roles).map_or(fallback, |colors| {
		crate::design::role_name_color(colors.primary, background, fallback)
	})
}

/// Remaining timeout text for a message author, only when this session already loaded that
/// member's moderation record. `now` is Unix seconds.
pub(crate) fn timeout_remaining(state: &State, channel: Id, user: Id, now: i64) -> Option<String> {
	let guild = state.channel(channel)?.guild?;
	if state.server_admin.guild != Some(guild) {
		return None;
	}
	let until = state
		.server_admin
		.members
		.as_ref()?
		.items
		.iter()
		.find(|member| member.user.id == user)?
		.timeout_until?;
	timeout_text(i64::try_from(until).ok()?, now)
}

/// Coarse "in …" text for a future timeout, matching the relative formatter used elsewhere.
pub(crate) fn timeout_text(until: i64, now: i64) -> Option<String> {
	(until > now).then(|| crate::local_time::future(until - now))
}

/// A reply preview's timestamp: the clock on the same local day as the base message, and the
/// existing calendar formatter otherwise.
pub(crate) fn reply_timestamp(base: time::OffsetDateTime, reply: time::OffsetDateTime) -> String {
	let reply_local = crate::local_time::local(reply);
	if crate::local_time::local(base).date() == reply_local.date() {
		crate::local_time::clock(reply_local)
	} else {
		crate::local_time::calendar(reply)
	}
}

/// Ctrl+Shift-click on a spoiler asks the timeline to reveal every loaded visible spoiler.
pub(crate) fn request_reveal_all(ctx: &egui::Context) {
	ctx.data_mut(|data| data.insert_temp(egui::Id::unique("reveal-all-spoilers"), true));
}

pub(crate) fn take_reveal_all(ctx: &egui::Context) -> bool {
	ctx.data_mut(|data| {
		data.remove_temp::<bool>(egui::Id::unique("reveal-all-spoilers"))
			.unwrap_or(false)
	})
}

/// One recognised tone indicator: byte range in the source text and its description.
pub(crate) type ToneRange = (std::ops::Range<usize>, &'static str);

/// Common tone indicators, matching Equicord's built-in mapping (aliases included).
pub(crate) fn tone_description(token: &str) -> Option<&'static str> {
	let token = token.to_ascii_lowercase();
	Some(match token.as_str() {
		"j" => "Joking",
		"hj" => "Half Joking",
		"s" | "_sarc" => "Sarcastic",
		"gen" | "_g" => "Genuine",
		"genq" | "_gq" => "Genuine Question",
		"srs" => "Serious",
		"nsrs" => "Non-Serious",
		"pos" | "_pc" => "Positive Connotation",
		"neu" => "Neutral Connotation",
		"neg" | "_nc" => "Negative Connotation",
		"p" => "Platonic",
		"r" => "Romantic",
		"a" => "Alterous",
		"c" => "Copypasta",
		"l" | "_ly" => "Lyrics",
		"lh" => "Light-Hearted",
		"nm" => "Not Mad",
		"lu" => "A Little Upset",
		"nbh" => "Not Directed At Anybody Here",
		"nay" => "Not Directed At You",
		"ay" => "Directed At You",
		"nsb" => "Not Subtweeting",
		"x" | "_sx" => "Sexual Intent",
		"nx" | "_nsx" => "Non-Sexual Intent",
		"rh" | "_rt" => "Rhetorical Question",
		"t" => "Teasing",
		"ij" => "Inside Joke",
		"m" => "Metaphorically",
		"li" => "Literally",
		"hyp" => "Hyperbole",
		"ex" => "Exaggeration",
		"f" => "Fake",
		"q" => "Quote",
		"th" => "Threat",
		"cb" => "Clickbait",
		"ref" => "Reference",
		"nf" => "Not Forced",
		"nbr" => "Not Being Rude",
		"ot" => "Off-Topic",
		"nj" => "Not Joking",
		"naj" => "Not A Joke",
		_ => return None,
	})
}

/// Tone tokens in `text`, with the same start/end boundaries as Equicord: a word boundary
/// before the slash and whitespace, the end, or punctuation after the token.
pub(crate) fn tone_ranges(text: &str) -> Vec<ToneRange> {
	let bytes = text.as_bytes();
	let mut found = Vec::new();
	let mut index = 0;
	let mut boundary = true;
	while index < bytes.len() {
		let chr = text[index..].chars().next().expect("remaining text");
		if chr != '/' || !boundary {
			boundary = chr.is_whitespace();
			index += chr.len_utf8();
			continue;
		}
		let mut end = index + 1;
		while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
			end += 1;
		}
		if end == index + 1 {
			boundary = false;
			index += 1;
			continue;
		}
		let after = text[end..].chars().next();
		let closes = after.is_none_or(|chr| {
			chr.is_whitespace() || (!chr.is_alphanumeric() && chr != '_' && chr != '/')
		});
		if closes && let Some(description) = tone_description(&text[index + 1..end]) {
			found.push((index..end, description));
			boundary = false;
			index = end;
			continue;
		}
		boundary = false;
		index = end;
	}
	found
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn tone_ranges_respect_word_boundaries_and_aliases() {
		let text = "Hi /srs, /GENq and /j, plus /x/y and https://example.test/a/srs";
		let ranges = tone_ranges(text);
		assert_eq!(
			ranges
				.iter()
				.map(|(range, description)| (&text[range.clone()], *description))
				.collect::<Vec<_>>(),
			[
				("/srs", "Serious"),
				("/GENq", "Genuine Question"),
				("/j", "Joking")
			]
		);
		assert!(tone_ranges("/srs/").is_empty());
		assert!(tone_ranges("x/srs").is_empty());
		assert!(tone_ranges("/unknown").is_empty());
	}

	#[test]
	fn timeout_text_only_reports_a_future_remaining_time() {
		let now = 1_000;
		assert_eq!(timeout_text(now + 80, now).as_deref(), Some("in a minute"));
		assert_eq!(
			timeout_text(now + 3_600, now).as_deref(),
			Some("in an hour")
		);
		assert_eq!(timeout_text(now, now), None);
		assert_eq!(timeout_text(now - 1, now), None);
	}

	#[test]
	fn reveal_all_request_is_consumed_once() {
		let ctx = egui::Context::default();
		assert!(!take_reveal_all(&ctx));
		request_reveal_all(&ctx);
		assert!(take_reveal_all(&ctx));
		assert!(!take_reveal_all(&ctx));
	}

	#[test]
	fn reply_timestamp_uses_the_clock_on_the_same_local_day() {
		let base = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
		let reply = base + time::Duration::minutes(7);
		assert_eq!(
			reply_timestamp(base, reply),
			crate::local_time::clock(crate::local_time::local(reply))
		);
	}
}
