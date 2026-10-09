//! Embedded application translations. Discord content and protocol locale stay untouched.
use fluent_templates::{
	FluentBundle, LanguageIdentifier,
	fluent_bundle::{FluentArgs, FluentResource},
};
#[cfg(not(test))]
use std::sync::atomic::{AtomicU8, Ordering};
use std::{cell::RefCell, sync::LazyLock};

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
static CHINESE_TRADITIONAL: LazyLock<LanguageIdentifier> =
	LazyLock::new(|| "zh-TW".parse().unwrap());
static CHINESE_SIMPLIFIED: LazyLock<LanguageIdentifier> =
	LazyLock::new(|| "zh-CN".parse().unwrap());
#[cfg(not(test))]
static SYSTEM: LazyLock<Language> =
	LazyLock::new(|| language_from_tag(sys_locale::get_locale().as_deref().unwrap_or("en-US")));
#[cfg(test)]
static SYSTEM: LazyLock<Language> = LazyLock::new(|| Language::English);
#[cfg(not(test))]
static CURRENT: AtomicU8 = AtomicU8::new(Language::System as u8);

struct ActiveBundle {
	language: Language,
	bundle: FluentBundle<FluentResource>,
}

thread_local! {
	// Synthetic UI fixtures must not share locale changes with parallel tests.
	#[cfg(test)]
	static TEST_CURRENT: std::cell::Cell<Language> = const { std::cell::Cell::new(Language::English) };
	static ACTIVE_BUNDLE: RefCell<Option<ActiveBundle>> = const { RefCell::new(None) };
}

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
	ChineseTraditional,
	ChineseSimplified,
}

impl Language {
	pub const ALL: [Self; 14] = [
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
		Self::ChineseTraditional,
		Self::ChineseSimplified,
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
			Some("zh-TW") => Self::ChineseTraditional,
			Some("zh-CN") => Self::ChineseSimplified,
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
			Self::ChineseTraditional => Some("zh-TW"),
			Self::ChineseSimplified => Some("zh-CN"),
		}
	}

	pub fn text(self, key: &str) -> String {
		self.lookup(key, None)
			.unwrap_or_else(|| format!("Unknown localization key: {key:?}"))
	}

	fn try_text(self, key: &str) -> Option<String> {
		self.lookup(key, None)
	}

	fn text_with_args(self, key: &str, values: &[(&'static str, &str)]) -> String {
		self.text_with_count(key, None, values)
	}

	/// `$count` is passed as a number, so catalogs can select their plural forms on it.
	fn text_with_count(
		self,
		key: &str,
		count: Option<usize>,
		values: &[(&'static str, &str)],
	) -> String {
		let mut args = FluentArgs::with_capacity(values.len() + 1);
		if let Some(count) = count {
			args.set("count", count);
		}
		for &(name, value) in values {
			args.set(name, value);
		}
		self.lookup(key, Some(&args))
			.unwrap_or_else(|| format!("Unknown localization key: {key:?}"))
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
			Self::ChineseTraditional => "\u{7e41}\u{9ad4}\u{4e2d}\u{6587}".into(),
			Self::ChineseSimplified => "\u{7b80}\u{4f53}\u{4e2d}\u{6587}".into(),
		}
	}

	pub(crate) fn resolved(self) -> Self {
		if self != Self::System {
			return self;
		}
		*SYSTEM
	}

	fn source(self) -> &'static str {
		match self.resolved() {
			Self::Spanish => include_str!("../locales/es/main.ftl"),
			Self::French => include_str!("../locales/fr/main.ftl"),
			Self::German => include_str!("../locales/de/main.ftl"),
			Self::Russian => include_str!("../locales/ru/main.ftl"),
			Self::PortugueseBrazil => include_str!("../locales/pt-BR/main.ftl"),
			Self::Turkish => include_str!("../locales/tr/main.ftl"),
			Self::Japanese => include_str!("../locales/ja/main.ftl"),
			Self::Polish => include_str!("../locales/pl/main.ftl"),
			Self::Italian => include_str!("../locales/it/main.ftl"),
			Self::Czech => include_str!("../locales/cs/main.ftl"),
			Self::ChineseTraditional => include_str!("../locales/zh-TW/main.ftl"),
			Self::ChineseSimplified => include_str!("../locales/zh-CN/main.ftl"),
			_ => include_str!("../locales/en-US/main.ftl"),
		}
	}

	fn load(self) -> ActiveBundle {
		let language = self.resolved();
		let resource = FluentResource::try_new(language.source().to_owned())
			.unwrap_or_else(|(_, errors)| panic!("invalid Fluent catalog: {errors:?}"));
		let mut bundle = FluentBundle::new_concurrent(vec![language.identifier().clone()]);
		bundle
			.add_resource(resource)
			.unwrap_or_else(|errors| panic!("invalid Fluent messages: {errors:?}"));
		ActiveBundle { language, bundle }
	}

	fn lookup(self, key: &str, args: Option<&FluentArgs<'_>>) -> Option<String> {
		let language = self.resolved();
		ACTIVE_BUNDLE.with_borrow_mut(|active| {
			if active
				.as_ref()
				.is_none_or(|active| active.language != language)
			{
				*active = Some(language.load());
			}
			let bundle = &active.as_ref()?.bundle;
			let pattern = if let Some((message, attribute)) = key.split_once('.') {
				bundle
					.get_message(message)?
					.attributes()
					.find(|value| value.id() == attribute)?
					.value()
			} else {
				bundle.get_message(key)?.value()?
			};
			let mut errors = Vec::new();
			let value = bundle
				.format_pattern(pattern, args, &mut errors)
				.into_owned();
			errors.is_empty().then_some(value)
		})
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
			Self::ChineseTraditional => &CHINESE_TRADITIONAL,
			Self::ChineseSimplified => &CHINESE_SIMPLIFIED,
			_ => &ENGLISH,
		}
	}
}

#[cfg(test)]
pub fn set_current(language: Language) {
	TEST_CURRENT.set(language);
}

#[cfg(not(test))]
pub fn set_current(language: Language) {
	if CURRENT.swap(language as u8, Ordering::Relaxed) != language as u8 {
		ACTIVE_BUNDLE.with_borrow_mut(|active| *active = None);
	}
}

pub fn translate(key: &str) -> String {
	current().text(key)
}

pub fn translate_args(key: &str, values: &[(&'static str, &str)]) -> String {
	current().text_with_args(key, values)
}

/// Like [`translate_args`], with a numeric `$count` for plural selection.
pub fn translate_count(key: &str, count: usize, values: &[(&'static str, &str)]) -> String {
	current().text_with_count(key, Some(count), values)
}

/// Translate a semantic key while leaving service- or user-provided text untouched.
pub fn translate_if_key(value: &str) -> String {
	current()
		.try_text(value)
		.unwrap_or_else(|| value.to_owned())
}

#[cfg(test)]
pub(crate) fn current() -> Language {
	TEST_CURRENT.get()
}

#[cfg(not(test))]
pub(crate) fn current() -> Language {
	let value = CURRENT.load(Ordering::Relaxed);
	Language::ALL
		.into_iter()
		.find(|language| *language as u8 == value)
		.unwrap_or_default()
}

fn language_from_tag(tag: &str) -> Language {
	// Chinese uses script/region subtags; other languages keep primary-subtag matching.
	let normalized = tag
		.split('.')
		.next()
		.unwrap_or_default()
		.to_ascii_lowercase()
		.replace('_', "-");
	if let Some(suffix) = normalized.strip_prefix("zh-") {
		let mut subtags = suffix.split('-');
		let first = subtags.next().unwrap_or_default();
		let (script, region) = if first.len() == 4 && first.bytes().all(|c| c.is_ascii_alphabetic())
		{
			(first, subtags.next().unwrap_or_default())
		} else {
			("", first)
		};
		return if script == "hant" || (script != "hans" && matches!(region, "tw" | "hk" | "mo")) {
			Language::ChineseTraditional
		} else {
			Language::ChineseSimplified
		};
	} else if normalized == "zh" {
		return Language::ChineseSimplified;
	}
	match normalized.split('-').next().unwrap_or_default() {
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
		assert_eq!(language_from_tag("zh-TW"), Language::ChineseTraditional);
		assert_eq!(
			language_from_tag("zh_TW.UTF-8"),
			Language::ChineseTraditional
		);
		assert_eq!(language_from_tag("zh-Hant"), Language::ChineseTraditional);
		assert_eq!(language_from_tag("zh-HK"), Language::ChineseTraditional);
		for tag in ["zh-HK-u-nu-latn", "zh-Hant-CN", "zh-MO-x-hans"] {
			assert_eq!(
				language_from_tag(tag),
				Language::ChineseTraditional,
				"{tag}"
			);
		}
		for tag in [
			"zh-Hans-x-hant",
			"zh-Hans-TW",
			"zh-x-hant",
			"zh-u-rg-twzzzz",
			"zh-foo-hk",
		] {
			assert_eq!(language_from_tag(tag), Language::ChineseSimplified, "{tag}");
		}
		assert_eq!(language_from_tag("zh-CN"), Language::ChineseSimplified);
		assert_eq!(language_from_tag("zh"), Language::ChineseSimplified);
		assert_eq!(
			language_from_tag("zh_SG.UTF-8"),
			Language::ChineseSimplified
		);
		for language in Language::ALL
			.into_iter()
			.filter(|language| !matches!(language, Language::System | Language::English))
		{
			assert!(!language.text("page-general").is_empty());
			assert_eq!(Language::from_preference(language.preference()), language);
		}
		assert_eq!(Language::Czech.text("page-general"), "Obecné");
		assert_eq!(Language::Japanese.text("page-general"), "一般的な");
		assert_eq!(Language::ChineseTraditional.text("page-general"), "一般");
		assert_eq!(Language::ChineseSimplified.text("page-general"), "一般");
		assert_eq!(
			Language::ChineseTraditional.text("page-voice"),
			"語音及視訊"
		);
		assert_eq!(Language::ChineseSimplified.text("page-voice"), "语音及视频");
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

	#[test]
	fn channel_pill_labels_are_localized_in_every_catalog() {
		for language in Language::ALL {
			assert_ne!(
				language.text("channel-pill-thread"),
				language.text("channel-pill-post"),
				"{language:?} must distinguish regular threads from forum posts"
			);
		}
		for (key, english, czech) in [
			("channel-pill-thread", "Thread", "Vlákno"),
			("channel-pill-forum", "Forum", "Fórum"),
			("channel-pill-post", "Post", "Příspěvek"),
			("channel-pill-message", "message", "zpráva"),
		] {
			assert_eq!(Language::English.text(key), english);
			assert_eq!(Language::Czech.text(key), czech);
			for language in Language::ALL {
				let label = language.text(key);
				assert!(
					!label.is_empty() && !label.starts_with("Unknown localization key:"),
					"{language:?} {key}: {label}"
				);
			}
		}
	}

	#[test]
	fn counts_select_plural_forms() {
		let since = |language: Language, count| {
			language
				.text_with_count(
					"timeline-unread-banner-new-since",
					Some(count),
					&[("time", "23:21")],
				)
				.replace(['\u{2068}', '\u{2069}'], "")
		};
		assert_eq!(since(Language::English, 1), "1 new message since 23:21");
		assert_eq!(since(Language::English, 57), "57 new messages since 23:21");
		assert_eq!(since(Language::Czech, 1), "1 nová zpráva od 23:21");
		assert_eq!(since(Language::Czech, 3), "3 nové zprávy od 23:21");
		assert_eq!(since(Language::Czech, 57), "57 nových zpráv od 23:21");
		assert_eq!(since(Language::Russian, 21), "21 новое сообщение с 23:21");
		assert_eq!(since(Language::Russian, 12), "12 новых сообщений с 23:21");
		assert_eq!(since(Language::Polish, 22), "22 nowe wiadomości od 23:21");
		assert_eq!(since(Language::Polish, 12), "12 nowych wiadomości od 23:21");
		for language in Language::ALL {
			for count in [1, 2, 5, 22, 1000] {
				for key in [
					"timeline-unread-banner-new-since",
					"timeline-unread-banner-new-since-more",
				] {
					let text = language
						.text_with_count(key, Some(count), &[("time", "23:21")])
						.replace(['\u{2068}', '\u{2069}'], "");
					assert!(
						text.contains(&count.to_string()) && text.contains("23:21"),
						"{language:?} {key} {count}: {text}"
					);
				}
				let text = language.text_with_count(
					"timeline-present-control-new-messages",
					Some(count),
					&[],
				);
				assert!(
					text.replace(['\u{2068}', '\u{2069}'], "")
						.contains(&count.to_string()),
					"{language:?} {count}: {text}"
				);
			}
		}
	}

	#[test]
	fn keeps_only_the_last_requested_catalog_loaded() {
		ACTIVE_BUNDLE.with_borrow_mut(|active| *active = None);
		assert_eq!(Language::English.text("page-general"), "General");
		ACTIVE_BUNDLE.with_borrow(|active| {
			assert_eq!(
				active.as_ref().map(|active| active.language),
				Some(Language::English)
			);
		});
		assert_eq!(Language::Czech.text("page-general"), "Obecné");
		ACTIVE_BUNDLE.with_borrow(|active| {
			assert_eq!(
				active.as_ref().map(|active| active.language),
				Some(Language::Czech)
			);
		});
	}
}
