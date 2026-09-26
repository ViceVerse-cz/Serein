//! Embedded application translations. Discord content and protocol locale stay untouched.
use fluent_templates::{LanguageIdentifier, Loader};
use std::sync::{
	LazyLock,
	atomic::{AtomicU8, Ordering},
};

fluent_templates::static_loader! {
	static TRANSLATIONS = {
		locales: "./locales",
		fallback_language: "en-US",
	};
}

static ENGLISH: LazyLock<LanguageIdentifier> = LazyLock::new(|| "en-US".parse().unwrap());
static CZECH: LazyLock<LanguageIdentifier> = LazyLock::new(|| "cs".parse().unwrap());
static SYSTEM: LazyLock<Language> =
	LazyLock::new(|| language_from_tag(sys_locale::get_locale().as_deref().unwrap_or("en-US")));
static CURRENT: AtomicU8 = AtomicU8::new(Language::System as u8);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
	#[default]
	System,
	English,
	Czech,
}

impl Language {
	pub const ALL: [Self; 3] = [Self::System, Self::English, Self::Czech];

	pub fn from_preference(value: Option<&str>) -> Self {
		match value {
			Some("en-US") => Self::English,
			Some("cs") => Self::Czech,
			_ => Self::System,
		}
	}

	pub const fn preference(self) -> Option<&'static str> {
		match self {
			Self::System => None,
			Self::English => Some("en-US"),
			Self::Czech => Some("cs"),
		}
	}

	pub fn text(self, key: &str) -> String {
		let language = self.identifier();
		TRANSLATIONS.lookup(language, key)
	}

	pub fn source(self, source: &str) -> String {
		TRANSLATIONS
			.try_lookup(self.identifier(), &source_key(source))
			.unwrap_or_else(|| source.to_owned())
	}

	pub fn name(self, current: Self) -> String {
		match self {
			Self::System => current.text("language-system"),
			Self::English => "English".into(),
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
			Self::Czech => &CZECH,
			_ => &ENGLISH,
		}
	}
}

pub fn set_current(language: Language) {
	CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn translate(source: &str) -> String {
	let language = match CURRENT.load(Ordering::Relaxed) {
		value if value == Language::English as u8 => Language::English,
		value if value == Language::Czech as u8 => Language::Czech,
		_ => Language::System,
	};
	language.source(source)
}

fn source_key(source: &str) -> String {
	let hash = source
		.bytes()
		.fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
			(hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
		});
	format!("source-{hash:016x}")
}

fn language_from_tag(tag: &str) -> Language {
	if tag
		.split(['-', '_'])
		.next()
		.is_some_and(|language| language.eq_ignore_ascii_case("cs"))
	{
		Language::Czech
	} else {
		Language::English
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn negotiates_czech_and_falls_back_to_english() {
		assert_eq!(language_from_tag("cs-CZ"), Language::Czech);
		assert_eq!(language_from_tag("cs_CZ"), Language::Czech);
		assert_eq!(language_from_tag("de-DE"), Language::English);
		assert_eq!(Language::Czech.text("page-general"), "Obecné");
		assert_eq!(source_key("Mark As Read"), "source-b82ecdfc78c29614");
		assert_eq!(
			Language::Czech.source("Mark As Read"),
			"Označit jako přečtené"
		);
		assert_eq!(
			Language::Czech.source("Server Settings"),
			"Nastavení serveru"
		);
		assert_eq!(Language::Czech.source("Create invite"), "Vytvořit pozvánku");
		assert_eq!(Language::Czech.source("Direct Messages"), "Přímé zprávy");
		assert_eq!(Language::Czech.source("Online"), "Online");
		assert_eq!(
			Language::Czech
				.source("Saved channel preferences are damaged or incompatible with this build."),
			"Uložené předvolby kanálů jsou poškozené nebo nekompatibilní s touto verzí."
		);
	}
}
