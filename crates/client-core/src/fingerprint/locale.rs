//! The user's locale and timezone, read once from the operating system.
//!
//! Every transport reports the same values. A value the OS does not provide, or one that is
//! not a plain language tag or zone name, falls back to `en-US` / `UTC`.
use std::sync::OnceLock;

const FALLBACK_LOCALE: &str = "en-US";
const FALLBACK_TIMEZONE: &str = "UTC";
/// Language preferences listed in `Accept-Language`.
const MAX_LANGUAGES: usize = 6;

/// Documented Discord locales: <https://docs.discord.com/developers/reference#locales>.
/// A language-only match takes the first entry of that language.
const DISCORD_LOCALES: [&str; 32] = [
	"en-US", "en-GB", "id", "da", "de", "es-ES", "es-419", "fr", "hr", "it", "lt", "hu", "nl",
	"no", "pl", "pt-BR", "ro", "fi", "sv-SE", "vi", "tr", "cs", "el", "bg", "ru", "uk", "hi", "th",
	"zh-CN", "ja", "zh-TW", "ko",
];

/// A BCP 47 tag made of plain subtags: a 2–3 letter language, then short alphanumeric parts.
/// Anything else, including the POSIX `C` locale, is treated as unknown.
fn valid_locale(tag: &str) -> bool {
	let mut parts = tag.split('-');
	let language = parts.next().unwrap_or_default();
	tag.len() <= 35
		&& (2..=3).contains(&language.len())
		&& language.bytes().all(|byte| byte.is_ascii_alphabetic())
		&& parts.all(|part| {
			(1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
		})
}

/// An IANA zone name such as `Europe/Prague`.
fn valid_timezone(name: &str) -> bool {
	(1..=64).contains(&name.len())
		&& name.split('/').all(|part| {
			!part.is_empty()
				&& part
					.bytes()
					.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+'))
		})
}

/// The OS language preferences that are plain tags, most preferred first, without repeats.
fn languages(reported: impl Iterator<Item = String>) -> Vec<String> {
	let mut tags: Vec<String> = Vec::new();
	for tag in reported.take(32) {
		if valid_locale(&tag) && !tags.contains(&tag) && tags.len() < MAX_LANGUAGES {
			tags.push(tag);
		}
	}
	tags
}

/// The way the web client writes `navigator.languages` (unofficial): the first tag bare, then
/// weights falling from 0.9 and never below 0.1.
fn weighted(tags: &[String]) -> String {
	let mut header = String::new();
	for (index, tag) in tags.iter().enumerate() {
		if index == 0 {
			header.push_str(tag);
		} else {
			header.push_str(&format!(
				",{tag};q=0.{}",
				10usize.saturating_sub(index).max(1)
			));
		}
	}
	header
}

/// The documented Discord locale nearest to `tag`.
fn nearest_discord_locale(tag: &str) -> &'static str {
	let language = tag.split('-').next().unwrap_or_default();
	let traditional = ["Hant", "TW", "HK", "MO"].iter().any(|part| {
		tag.split('-')
			.skip(1)
			.any(|have| have.eq_ignore_ascii_case(part))
	});
	let regional = tag.contains('-');
	let language = match language.to_ascii_lowercase().as_str() {
		"zh" if traditional => return "zh-TW",
		"es" if regional && !tag.eq_ignore_ascii_case("es-ES") => return "es-419",
		"nb" | "nn" => "no".to_owned(),
		other => other.to_owned(),
	};
	DISCORD_LOCALES
		.iter()
		.find(|known| known.eq_ignore_ascii_case(tag))
		.or_else(|| {
			DISCORD_LOCALES
				.iter()
				.find(|known| known.split('-').next() == Some(language.as_str()))
		})
		.copied()
		.unwrap_or(FALLBACK_LOCALE)
}

struct Identity {
	locale: String,
	accept_language: String,
	discord_locale: &'static str,
	timezone: String,
}

fn identity() -> &'static Identity {
	static IDENTITY: OnceLock<Identity> = OnceLock::new();
	IDENTITY.get_or_init(|| {
		let mut tags = languages(sys_locale::get_locales());
		if tags.is_empty() {
			tags.push(FALLBACK_LOCALE.to_owned());
		}
		let timezone = iana_time_zone::get_timezone()
			.ok()
			.filter(|name| valid_timezone(name))
			.unwrap_or_else(|| FALLBACK_TIMEZONE.to_owned());
		Identity {
			accept_language: weighted(&tags),
			discord_locale: nearest_discord_locale(&tags[0]),
			locale: tags.swap_remove(0),
			timezone,
		}
	})
}

/// The OS locale as a BCP 47 tag; super-properties `system_locale`.
pub fn system_locale() -> &'static str {
	&identity().locale
}

/// `Accept-Language` for REST and the WebSocket upgrade: the OS language preferences.
pub fn accept_language() -> &'static str {
	&identity().accept_language
}

/// `X-Discord-Locale`: the documented Discord locale nearest to the OS locale. The web client
/// sends its interface language here (unofficial), which defaults from the system language.
pub fn discord_locale() -> &'static str {
	identity().discord_locale
}

/// `X-Discord-Timezone`: the OS IANA zone name.
pub fn timezone() -> &'static str {
	&identity().timezone
}

#[cfg(test)]
mod tests {
	use super::*;

	/// OS-provided text reaches HTTP headers and the hand-built super-properties JSON, so a
	/// value that could break either must be dropped rather than sent.
	#[test]
	fn os_values_that_could_break_a_header_or_json_are_rejected() {
		for tag in ["cs-CZ", "en", "zh-Hant-TW", "sr-Latn-RS", "es-419"] {
			assert!(valid_locale(tag), "{tag}");
		}
		for tag in [
			"",
			"C",
			"POSIX",
			"en-US\r\nx-injected: 1",
			"en\"US",
			"en_US.UTF-8",
			"en-US,*",
			"čs-CZ",
			"en-",
			&"a".repeat(40),
		] {
			assert!(!valid_locale(tag), "{tag:?}");
		}
		for zone in [
			"Europe/Prague",
			"UTC",
			"Etc/GMT+1",
			"America/Argentina/Buenos_Aires",
		] {
			assert!(valid_timezone(zone), "{zone}");
		}
		for zone in [
			"",
			"/etc/localtime",
			"Europe/",
			"Europe/Pra\"gue",
			"UTC\r\n",
			"a b",
		] {
			assert!(!valid_timezone(zone), "{zone:?}");
		}
		let reported = ["cs-CZ", "C", "cs-CZ", "en\"US", "en-GB", "de"].map(str::to_owned);
		let tags = languages(reported.into_iter());
		assert_eq!(tags, ["cs-CZ", "en-GB", "de"]);
		assert_eq!(weighted(&tags), "cs-CZ,en-GB;q=0.9,de;q=0.8");
	}

	#[test]
	fn os_locale_maps_to_a_documented_discord_locale() {
		for (tag, expected) in [
			("cs-CZ", "cs"),
			("en-GB", "en-GB"),
			("en-AU", "en-US"),
			("pt-PT", "pt-BR"),
			("es", "es-ES"),
			("es-MX", "es-419"),
			("zh-Hant-HK", "zh-TW"),
			("zh-SG", "zh-CN"),
			("nb-NO", "no"),
			("eo", "en-US"),
		] {
			assert_eq!(nearest_discord_locale(tag), expected, "{tag}");
		}
	}
}
