//! Embedded application translations. Discord content and protocol locale stay untouched.
use fluent_templates::{LanguageIdentifier, Loader};
use std::sync::LazyLock;

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
		let language = match self.resolved() {
			Self::Czech => &*CZECH,
			_ => &*ENGLISH,
		};
		TRANSLATIONS.lookup(language, key)
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
	}
}
