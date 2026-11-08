/*
 * Vencord, a Discord client mod
 * Copyright (c) 2023 Vendicated and contributors
 * Portions adapted from ReplaceGoogleSearch, Copyright (c) 2024 Vendicated and contributors
 *
 * Serein adaptation copyright (c) 2026 Serein contributors.
 * Changed November 8, 2026: selected message text opens a host-reviewed
 * public browser URL instead of using inline translation or direct navigation.
 *
 * This program is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License version 3 or (at your option) any
 * later version. See https://www.gnu.org/licenses/gpl-3.0.html.
 */
use serde::{Deserialize, Serialize};
use serein_extension_sdk::{
	AppAction, AppOutput, Element, HostEffect, MessageActionInvocation, Output,
};
use url::Url;

/// The selected message text and search query are capped before URL construction.
const MAX_TEXT_BYTES: usize = 1024;
/// Generated browser URLs stay within this byte ceiling before host validation.
const MAX_URL_BYTES: usize = 1024;
/// Preferences are tiny, non-secret values; corrupt storage is never overwritten implicitly.
const MAX_STORAGE_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Provider {
	#[default]
	Google,
	Deepl,
}

impl Provider {
	fn label(self) -> &'static str {
		match self {
			Self::Google => "Google Translate",
			Self::Deepl => "DeepL",
		}
	}

	fn from_panel(value: &str) -> Option<Self> {
		match value.trim().to_ascii_lowercase().as_str() {
			"google" | "google translate" => Some(Self::Google),
			"deepl" | "deep l" => Some(Self::Deepl),
			_ => None,
		}
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TargetLanguage {
	#[default]
	En,
	Cs,
	De,
	Es,
	Fr,
	It,
	Ja,
	Pl,
	Pt,
	Ru,
	Tr,
	Uk,
	Zh,
}

impl TargetLanguage {
	const ALL: [Self; 13] = [
		Self::En,
		Self::Cs,
		Self::De,
		Self::Es,
		Self::Fr,
		Self::It,
		Self::Ja,
		Self::Pl,
		Self::Pt,
		Self::Ru,
		Self::Tr,
		Self::Uk,
		Self::Zh,
	];

	fn as_str(self) -> &'static str {
		match self {
			Self::En => "en",
			Self::Cs => "cs",
			Self::De => "de",
			Self::Es => "es",
			Self::Fr => "fr",
			Self::It => "it",
			Self::Ja => "ja",
			Self::Pl => "pl",
			Self::Pt => "pt",
			Self::Ru => "ru",
			Self::Tr => "tr",
			Self::Uk => "uk",
			Self::Zh => "zh",
		}
	}

	fn from_panel(value: &str) -> Option<Self> {
		let value = value.trim().to_ascii_lowercase();
		Self::ALL
			.into_iter()
			.find(|language| language.as_str() == value.as_str())
	}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SearchEngine {
	Google,
	#[default]
	#[serde(rename = "duckduckgo")]
	DuckDuckGo,
	Brave,
	Bing,
	Wikipedia,
}

impl SearchEngine {
	const ALL: [Self; 5] = [
		Self::Google,
		Self::DuckDuckGo,
		Self::Brave,
		Self::Bing,
		Self::Wikipedia,
	];

	fn label(self) -> &'static str {
		match self {
			Self::Google => "Google",
			Self::DuckDuckGo => "DuckDuckGo",
			Self::Brave => "Brave",
			Self::Bing => "Bing",
			Self::Wikipedia => "Wikipedia",
		}
	}

	fn from_panel(value: &str) -> Option<Self> {
		match value.trim().to_ascii_lowercase().as_str() {
			"google" => Some(Self::Google),
			"duckduckgo" | "duck duck go" | "duck_duck_go" => Some(Self::DuckDuckGo),
			"brave" => Some(Self::Brave),
			"bing" => Some(Self::Bing),
			"wikipedia" => Some(Self::Wikipedia),
			_ => None,
		}
	}
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
	provider: Provider,
	target_language: TargetLanguage,
	search_engine: SearchEngine,
}

fn load_config(storage: Option<&str>) -> (Config, bool) {
	let Some(storage) = storage else {
		return (Config::default(), false);
	};
	if storage.len() > MAX_STORAGE_BYTES {
		return (Config::default(), true);
	}
	match serde_json::from_str::<Config>(storage) {
		Ok(config) => (config, false),
		Err(_) => (Config::default(), true),
	}
}

/// Trim, reject empty text, then truncate to at most `MAX_TEXT_BYTES` UTF-8 bytes
/// without splitting a character. The second trim avoids a trailing partial space.
fn bounded_text(text: &str) -> Option<String> {
	let trimmed = text.trim();
	if trimmed.is_empty() {
		return None;
	}
	let mut end = trimmed.len().min(MAX_TEXT_BYTES);
	while end > 0 && !trimmed.is_char_boundary(end) {
		end -= 1;
	}
	let bounded = trimmed[..end].trim_end();
	(!bounded.is_empty()).then(|| bounded.to_owned())
}

fn translate_url(provider: Provider, target: TargetLanguage, text: &str) -> Option<String> {
	fitting_url(text, |candidate| {
		translate_url_once(provider, target, candidate)
	})
}

fn translate_url_once(provider: Provider, target: TargetLanguage, text: &str) -> Option<String> {
	match provider {
		Provider::Google => {
			let mut url = Url::parse("https://translate.google.com/").ok()?;
			url.query_pairs_mut()
				.append_pair("sl", "auto")
				.append_pair("tl", target.as_str())
				.append_pair("text", text)
				.append_pair("op", "translate");
			Some(url.to_string())
		}
		Provider::Deepl => {
			// The fragment is public page state, never an API request. Encode it
			// explicitly so '#', '%' and spaces cannot change the URL structure.
			let encoded = url::form_urlencoded::byte_serialize(text.as_bytes())
				.collect::<String>()
				.replace('+', "%20");
			Url::parse(&format!(
				"https://www.deepl.com/translator#auto/{}/{}",
				target.as_str(),
				encoded
			))
			.ok()
			.map(|url| url.to_string())
		}
	}
}

fn search_url(engine: SearchEngine, query: &str) -> Option<String> {
	fitting_url(query, |candidate| search_url_once(engine, candidate))
}

fn search_url_once(engine: SearchEngine, query: &str) -> Option<String> {
	let (base, parameter) = match engine {
		SearchEngine::Google => ("https://www.google.com/search", "q"),
		SearchEngine::DuckDuckGo => ("https://duckduckgo.com/", "q"),
		SearchEngine::Brave => ("https://search.brave.com/search", "q"),
		SearchEngine::Bing => ("https://www.bing.com/search", "q"),
		SearchEngine::Wikipedia => ("https://en.wikipedia.org/w/index.php", "search"),
	};
	let mut url = Url::parse(base).ok()?;
	url.query_pairs_mut().append_pair(parameter, query);
	Some(url.to_string())
}

/// Return the longest nonempty UTF-8 prefix whose encoded URL fits the byte
/// ceiling. A bounded binary search keeps very long, percent-heavy inputs from
/// rebuilding the URL once per dropped character. Candidate trimming matches
/// the previous character-by-character fallback exactly.
fn fitting_url(text: &str, build: impl Fn(&str) -> Option<String>) -> Option<String> {
	if let Some(url) = build(text)
		&& url.len() <= MAX_URL_BYTES
	{
		return Some(url);
	}
	let mut boundaries = Vec::with_capacity(text.len() + 1);
	boundaries.push(0);
	boundaries.extend(
		text.char_indices()
			.map(|(offset, character)| offset + character.len_utf8()),
	);
	let mut best = None;
	let mut low = 1;
	let mut high = boundaries.len().saturating_sub(1);
	while low <= high {
		let middle = low + (high - low) / 2;
		let candidate = text[..boundaries[middle]].trim_end();
		if candidate.is_empty() {
			low = middle + 1;
			continue;
		}
		match build(candidate) {
			Some(url) if url.len() <= MAX_URL_BYTES => {
				best = Some(url);
				low = middle + 1;
			}
			_ => high = middle - 1,
		}
	}
	best
}

fn open_external_link(url: String) -> HostEffect {
	HostEffect::AppAction {
		action: AppAction::OpenExternalLink { url },
	}
}

fn action_panel(title: &str, detail: &str, url: &str, corrupt: bool) -> Vec<Element> {
	let mut panel = vec![
		Element::Heading {
			text: title.into(),
		},
		Element::Text {
			text: detail.into(),
		},
		Element::Text {
			text: format!("Reviewed public URL: {url}"),
		},
		Element::Text {
			text: "Apply opens this reviewed public URL in your browser. The plugin does not fetch it, translate inline, replace the message, or send anything.".into(),
		},
	];
	if corrupt {
		panel.push(Element::Text {
			text: "Saved settings were invalid. Defaults were used and the stored value was left unchanged; open the panel to review and save settings.".into(),
		});
	}
	panel
}

fn no_text_panel(title: &str) -> Vec<Element> {
	vec![
		Element::Heading {
			text: title.into(),
		},
		Element::Text {
			text: "Select a message that contains text. Empty or whitespace-only text produces no URL and no effect.".into(),
		},
	]
}

fn error_panel(message: &str) -> Vec<Element> {
	vec![
		Element::Heading {
			text: "External search tools".into(),
		},
		Element::Text {
			text: message.into(),
		},
	]
}

fn settings_panel(config: Config, notice: &str) -> Vec<Element> {
	let mut panel = vec![
		Element::Heading {
			text: "External search tools".into(),
		},
		Element::Text {
			text: "Translate or search the selected message in your browser. Serein does not fetch pages, call translation APIs, or send messages.".into(),
		},
		Element::Select {
			id: "provider".into(),
			label: "Translation provider".into(),
			options: vec!["Google Translate".into(), "DeepL".into()],
			value: config.provider.label().into(),
		},
		Element::Select {
			id: "target-language".into(),
			label: "Target language code".into(),
			options: TargetLanguage::ALL
				.iter()
				.map(|language| language.as_str().to_owned())
				.collect(),
			value: config.target_language.as_str().into(),
		},
		Element::Select {
			id: "search-engine".into(),
			label: "Search engine".into(),
			options: SearchEngine::ALL
				.iter()
				.map(|engine| engine.label().to_owned())
				.collect(),
			value: config.search_engine.label().into(),
		},
		Element::Button {
			id: "save".into(),
			label: "Save external search tools".into(),
		},
		Element::Text {
			text: "Apply opens the reviewed public URL in your browser. The plugin never replaces the message, sends anything, or opens a URL without host confirmation.".into(),
		},
	];
	if !notice.is_empty() {
		panel.push(Element::Text {
			text: notice.into(),
		});
	}
	panel
}

fn translate_action(input: &MessageActionInvocation) -> AppOutput {
	let invocation = &input.invocation.invocation;
	let (config, corrupt) = load_config(invocation.storage.as_deref());
	let Some(text) = invocation
		.selected_message
		.as_deref()
		.and_then(bounded_text)
	else {
		return AppOutput {
			output: Output {
				panel: no_text_panel("Translate in browser"),
				..Default::default()
			},
			..Default::default()
		};
	};
	let Some(url) = translate_url(config.provider, config.target_language, &text) else {
		return AppOutput {
			output: Output {
				panel: error_panel("Could not build a safe translation URL. Nothing was opened."),
				..Default::default()
			},
			..Default::default()
		};
	};
	AppOutput {
		output: Output {
			panel: action_panel(
				"Translate in browser",
				&format!(
					"Provider: {}. Target language: {}.",
					config.provider.label(),
					config.target_language.as_str()
				),
				&url,
				corrupt,
			),
			..Default::default()
		},
		effects: vec![open_external_link(url)],
	}
}

fn search_action(input: &MessageActionInvocation) -> AppOutput {
	let invocation = &input.invocation.invocation;
	let (config, corrupt) = load_config(invocation.storage.as_deref());
	let Some(query) = invocation
		.selected_message
		.as_deref()
		.and_then(bounded_text)
	else {
		return AppOutput {
			output: Output {
				panel: no_text_panel("Search in browser"),
				..Default::default()
			},
			..Default::default()
		};
	};
	let Some(url) = search_url(config.search_engine, &query) else {
		return AppOutput {
			output: Output {
				panel: error_panel("Could not build a safe search URL. Nothing was opened."),
				..Default::default()
			},
			..Default::default()
		};
	};
	AppOutput {
		output: Output {
			panel: action_panel(
				"Search in browser",
				&format!("Search engine: {}.", config.search_engine.label()),
				&url,
				corrupt,
			),
			..Default::default()
		},
		effects: vec![open_external_link(url)],
	}
}

fn panel_action(input: &MessageActionInvocation, save: bool) -> AppOutput {
	let invocation = &input.invocation.invocation;
	let (saved, corrupt) = load_config(invocation.storage.as_deref());
	let mut output = AppOutput::default();
	let mut notice = if corrupt {
		"Saved settings are invalid. They were left unchanged; choose values and Save to replace them."
			.to_owned()
	} else {
		String::new()
	};
	if save {
		let provider = match invocation.value("provider") {
			Some(value) => match Provider::from_panel(value) {
				Some(provider) => provider,
				None => {
					let message = if corrupt {
						"Choose Google Translate or DeepL. Invalid stored settings were left unchanged."
					} else {
						"Choose Google Translate or DeepL."
					};
					output.output.panel = settings_panel(saved.clone(), message);
					return output;
				}
			},
			None => saved.provider,
		};
		let target_language = match invocation.value("target-language") {
			Some(value) => match TargetLanguage::from_panel(value) {
				Some(language) => language,
				None => {
					let message = if corrupt {
						"Choose a supported target language code. Invalid stored settings were left unchanged."
					} else {
						"Choose a supported target language code."
					};
					output.output.panel = settings_panel(saved.clone(), message);
					return output;
				}
			},
			None => saved.target_language,
		};
		let search_engine = match invocation.value("search-engine") {
			Some(value) => match SearchEngine::from_panel(value) {
				Some(engine) => engine,
				None => {
					let message = if corrupt {
						"Choose a supported search engine. Invalid stored settings were left unchanged."
					} else {
						"Choose a supported search engine."
					};
					output.output.panel = settings_panel(saved.clone(), message);
					return output;
				}
			},
			None => saved.search_engine,
		};
		let config = Config {
			provider,
			target_language,
			search_engine,
		};
		match serde_json::to_string(&config) {
			Ok(json)
				if json.len() <= MAX_STORAGE_BYTES
					&& output.output.set_storage_json(&config).is_ok() =>
			{
				notice = if corrupt {
					"Saved. Invalid stored settings were replaced."
				} else {
					"Saved. Translation and search actions will use these settings."
				}
				.to_owned();
				output.output.panel = settings_panel(config, &notice);
			}
			_ => {
				output.output.panel = settings_panel(
					config,
					"Could not save these settings. Nothing was changed.",
				);
			}
		}
		return output;
	}
	output.output.panel = settings_panel(saved, &notice);
	output
}

fn handle(input: MessageActionInvocation) -> AppOutput {
	match input.invocation.invocation.action.as_str() {
		"translate" => translate_action(&input),
		"search" => search_action(&input),
		"open" => panel_action(&input, false),
		"save" => panel_action(&input, true),
		_ => AppOutput::default(),
	}
}

serein_extension_sdk::export!(handle);

#[cfg(test)]
mod tests {
	use super::*;
	use serein_extension_sdk::{dispatch_typed, serde_json};

	fn run(value: serde_json::Value) -> AppOutput {
		let bytes = dispatch_typed(&serde_json::to_vec(&value).unwrap(), handle).unwrap();
		serde_json::from_slice(&bytes).unwrap()
	}

	fn external_url(output: &AppOutput) -> &str {
		assert_eq!(output.effects.len(), 1);
		match &output.effects[0] {
			HostEffect::AppAction {
				action: AppAction::OpenExternalLink { url },
			} => url,
			other => panic!("unexpected effect: {other:?}"),
		}
	}

	fn select_value<'a>(output: &'a AppOutput, id: &str) -> Option<&'a str> {
		output
			.output
			.panel
			.iter()
			.find_map(|element| match element {
				Element::Select {
					id: element_id,
					value,
					..
				} if element_id.as_str() == id => Some(value.as_str()),
				_ => None,
			})
	}

	#[test]
	fn text_is_trimmed_and_truncated_on_utf8_boundaries() {
		assert_eq!(bounded_text("  hello  ").as_deref(), Some("hello"));
		assert_eq!(bounded_text(" \n\t "), None);

		let two_byte = "é".repeat(600);
		let bounded = bounded_text(&two_byte).unwrap();
		assert_eq!(bounded.len(), 1024);
		assert!(bounded.is_char_boundary(bounded.len()));
		assert_eq!(bounded.chars().count(), 512);

		let three_byte = "€".repeat(400);
		let bounded = bounded_text(&three_byte).unwrap();
		assert_eq!(bounded.len(), 1023);
		assert!(bounded.is_char_boundary(bounded.len()));
		assert_eq!(bounded.chars().count(), 341);
	}

	#[test]
	fn translate_proposes_one_reviewed_browser_url() {
		let output = run(serde_json::json!({
			"action": "translate",
			"selected_message": "  hello world  "
		}));
		let url = external_url(&output);
		assert!(url.starts_with("https://translate.google.com/?"));
		assert!(url.contains("sl=auto"));
		assert!(url.contains("tl=en"));
		assert!(url.contains("text=hello+world"));
		assert!(output.output.replacement.is_none());
		assert!(output.output.storage.is_none());
		assert!(output.output.panel.iter().any(|element| {
			matches!(element, Element::Text { text } if text.contains("Apply") && text.contains("browser"))
		}));
	}

	#[test]
	fn translation_urls_fit_the_encoded_byte_limit_for_every_provider() {
		for provider in [Provider::Google, Provider::Deepl] {
			for text in ["x".repeat(1024), "🦀".repeat(256)] {
				let url = translate_url(provider, TargetLanguage::Ja, &text)
					.expect("some UTF-8 text should fit");
				assert!(url.len() <= MAX_URL_BYTES, "{} bytes: {url}", url.len());
				let parsed = Url::parse(&url).unwrap();
				assert!(!parsed.path().is_empty() || !parsed.fragment().unwrap_or("").is_empty());
			}
		}
	}

	#[test]
	fn saved_provider_language_and_engine_are_restored_and_used() {
		let storage = r#"{"provider":"deepl","target_language":"ja","search_engine":"brave"}"#;
		let opened = run(serde_json::json!({"action": "open", "storage": storage}));
		assert_eq!(select_value(&opened, "provider"), Some("DeepL"));
		assert_eq!(select_value(&opened, "target-language"), Some("ja"));
		assert_eq!(select_value(&opened, "search-engine"), Some("Brave"));
		assert!(opened.output.storage.is_none());
		assert!(opened.effects.is_empty());

		let translated = run(serde_json::json!({
			"action": "translate",
			"selected_message": "hello",
			"storage": storage
		}));
		assert!(
			external_url(&translated).starts_with("https://www.deepl.com/translator#auto/ja/hello")
		);

		let searched = run(serde_json::json!({
			"action": "search",
			"selected_message": "rust lang",
			"storage": storage
		}));
		assert!(external_url(&searched).starts_with("https://search.brave.com/search?q=rust+lang"));

		let saved = run(serde_json::json!({
			"action": "save",
			"storage": storage,
			"values": {
				"provider": "Google Translate",
				"target-language": "cs",
				"search-engine": "Wikipedia"
			}
		}));
		let stored: Config =
			serde_json::from_str(saved.output.storage.as_deref().unwrap()).unwrap();
		assert_eq!(stored.provider, Provider::Google);
		assert_eq!(stored.target_language, TargetLanguage::Cs);
		assert_eq!(stored.search_engine, SearchEngine::Wikipedia);
		assert!(saved.effects.is_empty());

		let searched = run(serde_json::json!({
			"action": "search",
			"selected_message": "Serein",
			"storage": saved.output.storage
		}));
		assert!(
			external_url(&searched)
				.starts_with("https://en.wikipedia.org/w/index.php?search=Serein")
		);
	}

	#[test]
	fn empty_text_produces_no_effect_or_storage() {
		for action in ["translate", "search"] {
			for selected in [serde_json::Value::Null, serde_json::json!("   \n\t ")] {
				let mut value = serde_json::json!({"action": action});
				if !selected.is_null() {
					value["selected_message"] = selected;
				}
				let output = run(value);
				assert!(output.effects.is_empty(), "{action}");
				assert!(output.output.storage.is_none(), "{action}");
				assert!(output.output.panel.iter().any(|element| {
					matches!(element, Element::Text { text } if text.contains("Select a message"))
				}));
			}
		}
	}

	#[test]
	fn malformed_or_unknown_storage_is_preserved_until_an_explicit_valid_save() {
		for storage in [
			"{invalid",
			r#"{"provider":"custom","target_language":"en","search_engine":"google"}"#,
			r#"{"provider":"google","target_language":"xx","search_engine":"google"}"#,
			r#"{"provider":"google","target_language":"en","search_engine":"google","extra":true}"#,
		] {
			let opened = run(serde_json::json!({"action": "open", "storage": storage}));
			assert!(opened.output.storage.is_none(), "{storage}");
			assert!(
				opened.output.panel.iter().any(|element| {
					matches!(element, Element::Text { text } if text.contains("invalid"))
				}),
				"{storage}"
			);

			let translated = run(serde_json::json!({
				"action": "translate",
				"selected_message": "hello",
				"storage": storage
			}));
			assert!(translated.output.storage.is_none(), "{storage}");
			assert!(
				external_url(&translated).starts_with("https://translate.google.com/"),
				"{storage}"
			);

			for values in [
				serde_json::json!({
					"provider": "Custom",
					"target-language": "en",
					"search-engine": "google"
				}),
				serde_json::json!({
					"provider": "google",
					"target-language": "xx",
					"search-engine": "google"
				}),
				serde_json::json!({
					"provider": "google",
					"target-language": "en",
					"search-engine": "custom"
				}),
			] {
				let rejected = run(serde_json::json!({
					"action": "save",
					"storage": storage,
					"values": values
				}));
				assert!(rejected.output.storage.is_none(), "{storage}");
				assert!(
					rejected.output.panel.iter().any(|element| {
						matches!(element, Element::Text { text } if text.contains("Choose"))
					}),
					"{storage}"
				);
			}
		}

		let saved = run(serde_json::json!({
			"action": "save",
			"storage": "{invalid",
			"values": {
				"provider": "deepl",
				"target-language": "uk",
				"search-engine": "bing"
			}
		}));
		let storage = saved.output.storage.as_deref().unwrap();
		assert!(storage.len() <= MAX_STORAGE_BYTES);
		let config: Config = serde_json::from_str(storage).unwrap();
		assert_eq!(config.provider, Provider::Deepl);
		assert_eq!(config.target_language, TargetLanguage::Uk);
		assert_eq!(config.search_engine, SearchEngine::Bing);
	}

	#[test]
	fn defaults_are_google_translate_english_and_duckduckgo() {
		let opened = run(serde_json::json!({"action": "open"}));
		assert_eq!(select_value(&opened, "provider"), Some("Google Translate"));
		assert_eq!(select_value(&opened, "target-language"), Some("en"));
		assert_eq!(select_value(&opened, "search-engine"), Some("DuckDuckGo"));

		let searched = run(serde_json::json!({
			"action": "search",
			"selected_message": "hello"
		}));
		assert!(external_url(&searched).starts_with("https://duckduckgo.com/?q=hello"));
	}

	#[test]
	fn search_url_stays_within_the_url_byte_bound() {
		let output = run(serde_json::json!({
			"action": "search",
			"selected_message": "x".repeat(5000)
		}));
		let url = external_url(&output);
		assert!(url.len() <= MAX_URL_BYTES);
		assert!(url.starts_with("https://duckduckgo.com/?q="));
	}

	#[test]
	fn expansion_heavy_text_selects_a_bounded_prefix_through_dispatch() {
		let heavy = "%".repeat(5000);

		let searched = run(serde_json::json!({
			"action": "search",
			"selected_message": heavy
		}));
		let url = external_url(&searched);
		assert_eq!(
			url,
			format!("https://duckduckgo.com/?q={}", "%25".repeat(332))
		);
		assert!(url.len() <= MAX_URL_BYTES);

		let translated = run(serde_json::json!({
			"action": "translate",
			"selected_message": heavy
		}));
		let url = external_url(&translated);
		assert_eq!(
			url,
			format!(
				"https://translate.google.com/?sl=auto&tl=en&text={}&op=translate",
				"%25".repeat(320)
			)
		);
		assert!(url.len() <= MAX_URL_BYTES);

		let deepl = run(serde_json::json!({
			"action": "translate",
			"selected_message": heavy,
			"storage": r#"{"provider":"deepl","target_language":"en","search_engine":"google"}"#
		}));
		let url = external_url(&deepl);
		assert_eq!(
			url,
			format!(
				"https://www.deepl.com/translator#auto/en/{}",
				"%25".repeat(327)
			)
		);
		assert!(url.len() <= MAX_URL_BYTES);

		let mixed = run(serde_json::json!({
			"action": "search",
			"selected_message": "🦀%".repeat(1000)
		}));
		let url = external_url(&mixed);
		assert!(url.len() <= MAX_URL_BYTES);
		assert!(url.ends_with("%25"));
		Url::parse(url).expect("bounded mixed-width URL parses");
	}

	#[test]
	fn fitting_url_matches_a_linear_longest_prefix_oracle() {
		fn oracle(text: &str, build: &dyn Fn(&str) -> Option<String>) -> Option<String> {
			if let Some(url) = build(text)
				&& url.len() <= MAX_URL_BYTES
			{
				return Some(url);
			}
			let mut best = None;
			let mut boundaries = Vec::with_capacity(text.len() + 1);
			boundaries.push(0);
			boundaries.extend(
				text.char_indices()
					.map(|(offset, character)| offset + character.len_utf8()),
			);
			for end in boundaries.into_iter().skip(1) {
				let candidate = text[..end].trim_end();
				if candidate.is_empty() {
					continue;
				}
				if let Some(url) = build(candidate)
					&& url.len() <= MAX_URL_BYTES
				{
					best = Some(url);
				}
			}
			best
		}

		// Mixed spaces, multibyte characters and percent-heavy bytes exercise
		// boundaries that do not line up with the encoded URL length.
		type BuildUrl<'a> = &'a dyn Fn(&str) -> Option<String>;
		let text = "🦀 a %  b ".repeat(200);
		let builds: [BuildUrl; 3] = [
			&|candidate| search_url_once(SearchEngine::DuckDuckGo, candidate),
			&|candidate| translate_url_once(Provider::Google, TargetLanguage::Ja, candidate),
			&|candidate| translate_url_once(Provider::Deepl, TargetLanguage::De, candidate),
		];
		for build in builds {
			let expected = oracle(&text, build);
			let actual = fitting_url(&text, build);
			assert_eq!(actual, expected);
			if let Some(url) = actual {
				assert!(url.len() <= MAX_URL_BYTES);
			}
		}
	}

	#[test]
	fn fitting_url_uses_a_bounded_number_of_reconstructions() {
		let calls = std::cell::Cell::new(0);
		let text = "%".repeat(MAX_TEXT_BYTES);
		let url = fitting_url(&text, |candidate| {
			calls.set(calls.get() + 1);
			search_url_once(SearchEngine::DuckDuckGo, candidate)
		})
		.expect("an expansion-heavy prefix fits");
		assert!(url.len() <= MAX_URL_BYTES);
		assert!(calls.get() <= 12, "{} URL reconstructions", calls.get());
	}

	#[test]
	fn every_allowlisted_search_engine_builds_an_https_url() {
		for (engine, host) in [
			("Google", "www.google.com/search"),
			("DuckDuckGo", "duckduckgo.com/?"),
			("Brave", "search.brave.com/search"),
			("Bing", "www.bing.com/search"),
			("Wikipedia", "en.wikipedia.org/w/index.php"),
		] {
			let saved = run(serde_json::json!({
				"action": "save",
				"values": {
					"provider": "google",
					"target-language": "en",
					"search-engine": engine
				}
			}));
			let searched = run(serde_json::json!({
				"action": "search",
				"selected_message": "hello",
				"storage": saved.output.storage
			}));
			let url = external_url(&searched);
			assert!(url.starts_with("https://"), "{engine}: {url}");
			assert!(url.contains(host), "{engine}: {url}");
		}
	}

	#[test]
	fn urls_are_https_and_do_not_leak_settings_or_url_delimiters() {
		let output = run(serde_json::json!({
			"action": "search",
			"selected_message": "a&b #c 100%",
			"storage": r#"{"provider":"google","target_language":"en","search_engine":"duckduckgo"}"#
		}));
		let url = external_url(&output);
		assert!(url.starts_with("https://duckduckgo.com/?q="));
		assert!(url.contains("a%26b+%23c+100%25"));
		assert!(!url.contains("storage"));
		assert!(!url.contains("selected_message"));

		let translated = run(serde_json::json!({
			"action": "translate",
			"selected_message": "a&b #c",
			"storage": r#"{"provider":"deepl","target_language":"de","search_engine":"google"}"#
		}));
		let url = external_url(&translated);
		assert!(url.starts_with("https://www.deepl.com/translator#auto/de/"));
		assert!(url.contains("a%26b%20%23c"));
	}
}
