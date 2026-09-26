//! Embedded application translations. Discord content and protocol locale stay untouched.
use fluent_templates::{LanguageIdentifier, Loader, fluent_bundle::FluentValue};
use std::{
	borrow::Cow,
	collections::HashMap,
	sync::{
		LazyLock,
		atomic::{AtomicU8, Ordering},
	},
};

fluent_templates::static_loader! {
	static TRANSLATIONS = {
		locales: "./locales",
		fallback_language: "en-US",
	};
}

static ENGLISH: LazyLock<LanguageIdentifier> = LazyLock::new(|| "en-US".parse().unwrap());
static SPANISH: LazyLock<LanguageIdentifier> = LazyLock::new(|| "es".parse().unwrap());
static FRENCH: LazyLock<LanguageIdentifier> = LazyLock::new(|| "fr".parse().unwrap());
static GERMAN: LazyLock<LanguageIdentifier> = LazyLock::new(|| "de".parse().unwrap());
static RUSSIAN: LazyLock<LanguageIdentifier> = LazyLock::new(|| "ru".parse().unwrap());
static PORTUGUESE_BRAZIL: LazyLock<LanguageIdentifier> = LazyLock::new(|| "pt-BR".parse().unwrap());
static TURKISH: LazyLock<LanguageIdentifier> = LazyLock::new(|| "tr".parse().unwrap());
static JAPANESE: LazyLock<LanguageIdentifier> = LazyLock::new(|| "ja".parse().unwrap());
static POLISH: LazyLock<LanguageIdentifier> = LazyLock::new(|| "pl".parse().unwrap());
static ITALIAN: LazyLock<LanguageIdentifier> = LazyLock::new(|| "it".parse().unwrap());
static CZECH: LazyLock<LanguageIdentifier> = LazyLock::new(|| "cs".parse().unwrap());
static SYSTEM: LazyLock<Language> =
	LazyLock::new(|| language_from_tag(sys_locale::get_locale().as_deref().unwrap_or("en-US")));
static CURRENT: AtomicU8 = AtomicU8::new(Language::System as u8);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
	#[default]
	System,
	English,
	Spanish,
	French,
	German,
	Russian,
	PortugueseBrazil,
	Turkish,
	Japanese,
	Polish,
	Italian,
	Czech,
}

impl Language {
	pub const ALL: [Self; 12] = [
		Self::System,
		Self::English,
		Self::Spanish,
		Self::French,
		Self::German,
		Self::Russian,
		Self::PortugueseBrazil,
		Self::Turkish,
		Self::Japanese,
		Self::Polish,
		Self::Italian,
		Self::Czech,
	];

	pub fn from_preference(value: Option<&str>) -> Self {
		match value {
			Some("en-US") => Self::English,
			Some("es") => Self::Spanish,
			Some("fr") => Self::French,
			Some("de") => Self::German,
			Some("ru") => Self::Russian,
			Some("pt-BR") => Self::PortugueseBrazil,
			Some("tr") => Self::Turkish,
			Some("ja") => Self::Japanese,
			Some("pl") => Self::Polish,
			Some("it") => Self::Italian,
			Some("cs") => Self::Czech,
			_ => Self::System,
		}
	}

	pub const fn preference(self) -> Option<&'static str> {
		match self {
			Self::System => None,
			Self::English => Some("en-US"),
			Self::Spanish => Some("es"),
			Self::French => Some("fr"),
			Self::German => Some("de"),
			Self::Russian => Some("ru"),
			Self::PortugueseBrazil => Some("pt-BR"),
			Self::Turkish => Some("tr"),
			Self::Japanese => Some("ja"),
			Self::Polish => Some("pl"),
			Self::Italian => Some("it"),
			Self::Czech => Some("cs"),
		}
	}

	pub fn text(self, key: &str) -> String {
		let language = self.identifier();
		TRANSLATIONS.lookup(language, key)
	}

	fn try_text(self, key: &str) -> Option<String> {
		TRANSLATIONS.try_lookup(self.identifier(), key)
	}

	fn text_with_args(self, key: &str, values: &[(&'static str, &str)]) -> String {
		let args: HashMap<_, _> = values
			.iter()
			.map(|(name, value)| (Cow::Borrowed(*name), FluentValue::from(*value)))
			.collect();
		TRANSLATIONS.lookup_with_args(self.identifier(), key, &args)
	}

	pub fn name(self, current: Self) -> String {
		match self {
			Self::System => current.text("language-system"),
			Self::English => "English".into(),
			Self::Spanish => "Espa\u{00f1}ol".into(),
			Self::French => "Fran\u{00e7}ais".into(),
			Self::German => "Deutsch".into(),
			Self::Russian => "\u{0420}\u{0443}\u{0441}\u{0441}\u{043a}\u{0438}\u{0439}".into(),
			Self::PortugueseBrazil => "Portugu\u{00ea}s (Brasil)".into(),
			Self::Turkish => "T\u{00fc}rk\u{00e7}e".into(),
			Self::Japanese => "\u{65e5}\u{672c}\u{8a9e}".into(),
			Self::Polish => "Polski".into(),
			Self::Italian => "Italiano".into(),
			Self::Czech => "Čeština".into(),
		}
	}

	fn resolved(self) -> Self {
		if self != Self::System {
			return self;
		}
		*SYSTEM
	}

	fn identifier(self) -> &'static LanguageIdentifier {
		match self.resolved() {
			Self::Spanish => &SPANISH,
			Self::French => &FRENCH,
			Self::German => &GERMAN,
			Self::Russian => &RUSSIAN,
			Self::PortugueseBrazil => &PORTUGUESE_BRAZIL,
			Self::Turkish => &TURKISH,
			Self::Japanese => &JAPANESE,
			Self::Polish => &POLISH,
			Self::Italian => &ITALIAN,
			Self::Czech => &CZECH,
			_ => &ENGLISH,
		}
	}
}

pub fn set_current(language: Language) {
	CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn translate(key: &str) -> String {
	current().text(key)
}

pub fn translate_args(key: &str, values: &[(&'static str, &str)]) -> String {
	current().text_with_args(key, values)
}

/// Translate a semantic key while leaving service- or user-provided text untouched.
pub fn translate_if_key(value: &str) -> String {
	current()
		.try_text(value)
		.unwrap_or_else(|| value.to_owned())
}

fn current() -> Language {
	let value = CURRENT.load(Ordering::Relaxed);
	Language::ALL
		.into_iter()
		.find(|language| *language as u8 == value)
		.unwrap_or_default()
}

fn language_from_tag(tag: &str) -> Language {
	match tag
		.split(['-', '_'])
		.next()
		.unwrap_or_default()
		.to_ascii_lowercase()
		.as_str()
	{
		"es" => Language::Spanish,
		"fr" => Language::French,
		"de" => Language::German,
		"ru" => Language::Russian,
		"pt" => Language::PortugueseBrazil,
		"tr" => Language::Turkish,
		"ja" => Language::Japanese,
		"pl" => Language::Polish,
		"it" => Language::Italian,
		"cs" => Language::Czech,
		_ => Language::English,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn negotiates_supported_languages_and_falls_back_to_english() {
		assert_eq!(language_from_tag("cs-CZ"), Language::Czech);
		assert_eq!(language_from_tag("cs_CZ"), Language::Czech);
		assert_eq!(language_from_tag("es-MX"), Language::Spanish);
		assert_eq!(language_from_tag("pt-PT"), Language::PortugueseBrazil);
		assert_eq!(language_from_tag("de-DE"), Language::German);
		assert_eq!(language_from_tag("ko-KR"), Language::English);
		for language in Language::ALL
			.into_iter()
			.filter(|language| !matches!(language, Language::System | Language::English))
		{
			assert!(!language.text("page-general").is_empty());
			assert_eq!(Language::from_preference(language.preference()), language);
		}
		assert_eq!(Language::Czech.text("page-general"), "Obecné");
		assert_eq!(Language::Japanese.text("page-general"), "一般的な");
		assert_eq!(
			Language::Czech.text("message-menu-copy"),
			"Kopírovat zprávu"
		);
		assert_eq!(
			Language::Czech.text("voice-device-default"),
			"Výchozí nastavení systému"
		);
		assert_eq!(Language::English.text("message-menu-copy"), "Copy message");
		assert!(Language::English.try_text("Copy message").is_none());
		assert_eq!(
			Language::English.text_with_args(
				"channel-menu-delete-category-confirm",
				&[("name", "General")],
			),
			"Delete \u{2068}General\u{2069}? Its channels will remain in the server. This cannot be undone."
		);
	}
}
