//! Spell-check settings and dictionaries. Discord's Chromium checks the system language out of
//! the box and downloads other Hunspell dictionaries when picked; this does the same with the
//! system's Hunspell dictionaries and LibreOffice's. File and network work run on one
//! background thread, never during rendering.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use ui::spelling::{Detection, Dictionary, Language, Settings};

const FILE: &str = "spelling.json";
const MAX_FILE: u64 = 1024 * 1024;
/// The largest offered dictionary is about 10 MiB.
const MAX_DICTIONARY: usize = 16 * 1024 * 1024;
/// LibreOffice's dictionaries, pinned so a download is the file this table describes.
const SOURCE: &str = "https://raw.githubusercontent.com/LibreOffice/dictionaries/c0865a85eb67dc377f2cf8e8fe2c4fa9e53f0dcb";
/// Code, path in the repository without the extension, the language's own name, and its
/// ISO 639-3 code as language detection reports it.
const DOWNLOADS: &[(&str, &str, &str, &str)] = &[
	("bg_BG", "bg_BG/bg_BG", "Български", "bul"),
	("cs_CZ", "cs_CZ/cs_CZ", "Čeština", "ces"),
	("da_DK", "da_DK/da_DK", "Dansk", "dan"),
	("de_DE", "de/de_DE_frami", "Deutsch", "deu"),
	("en_GB", "en/en_GB", "English (United Kingdom)", "eng"),
	("en_US", "en/en_US", "English (United States)", "eng"),
	("es_ES", "es/es_ES", "Español", "spa"),
	("he_IL", "he_IL/he_IL", "עברית", "heb"),
	("hr_HR", "hr_HR/hr_HR", "Hrvatski", "hrv"),
	("hu_HU", "hu_HU/hu_HU", "Magyar", "hun"),
	("id_ID", "id/id_ID", "Bahasa Indonesia", "ind"),
	("it_IT", "it_IT/it_IT", "Italiano", "ita"),
	("lt_LT", "lt_LT/lt", "Lietuvių", "lit"),
	("lv_LV", "lv_LV/lv_LV", "Latviešu", "lav"),
	("nb_NO", "no/nb_NO", "Norsk bokmål", "nob"),
	("nl_NL", "nl_NL/nl_NL", "Nederlands", "nld"),
	("pt_BR", "pt_BR/pt_BR", "Português (Brasil)", "por"),
	("pt_PT", "pt_PT/pt_PT", "Português (Portugal)", "por"),
	("ro_RO", "ro/ro_RO", "Română", "ron"),
	("ru_RU", "ru_RU/ru_RU", "Русский", "rus"),
	("sk_SK", "sk_SK/sk_SK", "Slovenčina", "slk"),
	("sr_Latn", "sr/sr-Latn", "Srpski (latinica)", "srp"),
	("uk_UA", "uk_UA/uk_UA", "Українська", "ukr"),
	("vi_VN", "vi/vi_VN", "Tiếng Việt", "vie"),
];

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
	enabled: bool,
	languages: Vec<String>,
	words: BTreeSet<String>,
	#[serde(default)]
	excluded: BTreeSet<String>,
}

enum Update {
	Loaded(Settings),
	Available(Vec<Language>),
	Installed(String, Arc<Dictionary>),
	Failed(String),
	/// A detected language and what loading its dictionary found.
	Detected(String, Detection),
}

enum Request {
	Settings(Settings),
	Detect(String),
}

#[derive(Default)]
pub struct Spelling {
	updates: Option<mpsc::Receiver<Update>>,
	requests: Option<mpsc::Sender<Request>>,
}

impl Spelling {
	pub fn start(ctx: &eframe::egui::Context) -> Self {
		let (update, updates) = mpsc::channel();
		let (requests, receive) = mpsc::channel();
		let wake = ctx.clone();
		std::thread::spawn(move || worker(&update, &receive, &wake));
		Self {
			updates: Some(updates),
			requests: Some(requests),
		}
	}

	/// Applies loaded settings and dictionaries, and hands changes to the worker.
	pub fn sync(&mut self, messaging: &mut ui::MessagingUi) {
		if let Some(updates) = &self.updates {
			while let Ok(update) = updates.try_recv() {
				let spelling = &mut messaging.spelling;
				match update {
					Update::Loaded(settings) => {
						for code in &settings.languages {
							spelling.set_pending(code, true);
						}
						spelling.load(settings);
					}
					Update::Available(languages) => spelling.set_available(languages),
					Update::Installed(code, dictionary) => spelling.install(code, dictionary),
					Update::Failed(code) => spelling.set_pending(&code, false),
					Update::Detected(language, loaded) => {
						spelling.install_detected(&language, loaded)
					}
				}
			}
		}
		let Some(requests) = &self.requests else {
			return;
		};
		if let Some(changed) = messaging.spelling.take_changed() {
			let _ = requests.send(Request::Settings(changed));
		}
		for language in messaging.spelling.take_detected() {
			let _ = requests.send(Request::Detect(language));
		}
	}
}

/// Background thread: reads the stored settings, loads the chosen dictionaries, then answers
/// settings changes and detected languages until the app drops its sender.
fn worker(
	update: &mpsc::Sender<Update>,
	requests: &mpsc::Receiver<Request>,
	wake: &eframe::egui::Context,
) {
	let send = |message| {
		let _ = update.send(message);
		wake.request_repaint();
	};
	let root = local_store::data_dir().ok();
	let dictionaries = root.as_ref().map(|root| root.join("dictionaries"));
	let mut current = root.as_deref().and_then(read).unwrap_or_else(|| Settings {
		languages: default_languages(&system_dictionaries()),
		..Settings::default()
	});
	send(Update::Loaded(current.clone()));
	send(Update::Available(available(&system_dictionaries())));
	let mut loaded = BTreeSet::new();
	let mut automatic: BTreeMap<String, Arc<Dictionary>> = BTreeMap::new();
	loop {
		for code in &current.languages {
			if loaded.contains(code) {
				continue;
			}
			match load(code, dictionaries.as_deref()) {
				Some(dictionary) => {
					loaded.insert(code.clone());
					send(Update::Installed(code.clone(), Arc::new(dictionary)));
				}
				None => {
					diagnostics::record(
						diagnostics::Category::App,
						diagnostics::Level::Warn,
						"spelling dictionary unavailable",
					);
					send(Update::Failed(code.clone()));
				}
			}
		}
		let mut next = None;
		let Ok(first) = requests.recv() else {
			return;
		};
		for request in std::iter::once(first).chain(requests.try_iter()) {
			match request {
				Request::Settings(settings) => next = Some(settings),
				Request::Detect(language) => {
					let found = match dictionary_for(&language, &system_dictionaries()) {
						None => Detection::Unsupported,
						Some(code) if current.excluded.contains(&code) => Detection::Unsupported,
						Some(code) => match automatic
							.get(&code)
							.cloned()
							.or_else(|| load(&code, dictionaries.as_deref()).map(Arc::new))
						{
							Some(dictionary) => {
								automatic.insert(code.clone(), dictionary.clone());
								Detection::Loaded(code, dictionary)
							}
							None => Detection::Failed,
						},
					};
					send(Update::Detected(language, found));
				}
			}
		}
		let Some(next) = next else {
			continue;
		};
		if let Some(root) = &root {
			write(root, &next);
		}
		loaded.retain(|code| next.languages.contains(code));
		current = next;
	}
}

/// Reads `spelling.json`, dropping invalid codes and over-long or excess words.
fn read(root: &Path) -> Option<Settings> {
	let path = root.join(FILE);
	(std::fs::metadata(&path).ok()?.len() <= MAX_FILE).then_some(())?;
	let stored: Stored = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
	Some(Settings {
		enabled: stored.enabled,
		languages: stored
			.languages
			.into_iter()
			.filter(|code| valid_code(code))
			.collect(),
		words: stored
			.words
			.into_iter()
			.filter(|word| word.len() <= ui::spelling::MAX_WORD_BYTES)
			.take(ui::spelling::MAX_WORDS)
			.collect(),
		excluded: stored
			.excluded
			.into_iter()
			.filter(|code| valid_code(code))
			.collect(),
	})
}

fn write(root: &Path, settings: &Settings) {
	let stored = Stored {
		enabled: settings.enabled,
		languages: settings.languages.clone(),
		words: settings.words.clone(),
		excluded: settings.excluded.clone(),
	};
	let Ok(bytes) = serde_json::to_vec(&stored) else {
		return;
	};
	let path = root.join(FILE);
	let partial = path.with_extension("json.partial");
	let _ = std::fs::create_dir_all(root)
		.and_then(|()| std::fs::write(&partial, bytes))
		.and_then(|()| std::fs::rename(partial, path));
}

fn valid_code(code: &str) -> bool {
	!code.is_empty()
		&& code.len() <= 32
		&& code
			.chars()
			.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Directories holding the system's Hunspell dictionaries.
fn system_directories() -> Vec<PathBuf> {
	let mut out = Vec::new();
	let data = std::env::var("XDG_DATA_DIRS")
		.ok()
		.filter(|dirs| !dirs.is_empty())
		.unwrap_or_else(|| "/usr/local/share:/usr/share".into());
	for dir in data.split(':').filter(|dir| !dir.is_empty()) {
		out.push(Path::new(dir).join("hunspell"));
		out.push(Path::new(dir).join("myspell"));
		out.push(Path::new(dir).join("myspell/dicts"));
	}
	out
}

/// Codes of the dictionaries installed on the system.
fn system_dictionaries() -> BTreeSet<String> {
	let mut out = BTreeSet::new();
	for dir in system_directories() {
		let Ok(entries) = std::fs::read_dir(dir) else {
			continue;
		};
		for entry in entries.flatten().take(1024) {
			let path = entry.path();
			if path.extension().is_some_and(|ext| ext == "dic")
				&& path.with_extension("aff").exists()
				&& let Some(code) = path.file_stem().and_then(|stem| stem.to_str())
				&& valid_code(code)
			{
				out.insert(code.to_owned());
			}
		}
	}
	out
}

/// The dictionary for a language detected in written text: the system's own variant of it
/// when there is one, else the first offered.
fn dictionary_for(language: &str, installed: &BTreeSet<String>) -> Option<String> {
	let offered: Vec<&str> = DOWNLOADS
		.iter()
		.filter(|(.., iso)| *iso == language)
		.map(|(code, ..)| *code)
		.collect();
	let locale = sys_locale::get_locale()
		.unwrap_or_default()
		.replace('-', "_");
	offered
		.iter()
		.find(|code| locale.starts_with(**code))
		.or_else(|| offered.iter().find(|code| installed.contains(**code)))
		.or_else(|| offered.first())
		.map(|code| (*code).to_owned())
}

/// The system language first, as Discord checks it without being asked, then English.
fn default_languages(installed: &BTreeSet<String>) -> Vec<String> {
	let mut out = Vec::new();
	let locale = sys_locale::get_locale()
		.unwrap_or_default()
		.replace('-', "_");
	let language = locale.split(['_', '.', '@']).next().unwrap_or_default();
	let known = installed
		.iter()
		.map(String::as_str)
		.chain(DOWNLOADS.iter().map(|(code, ..)| *code));
	let pick = |known: &mut dyn Iterator<Item = &str>| {
		let candidates: Vec<&str> = known.collect();
		candidates
			.iter()
			.find(|code| locale.starts_with(**code))
			.or_else(|| {
				candidates
					.iter()
					.find(|code| code.split('_').next() == Some(language))
			})
			.map(|code| (*code).to_owned())
	};
	if !language.is_empty()
		&& let Some(code) = pick(&mut known.clone())
	{
		out.push(code);
	}
	if !out.iter().any(|code| code.starts_with("en")) {
		out.push("en_US".to_owned());
	}
	out
}

/// The languages offered in the menu: the downloadable table plus system dictionaries,
/// sorted by name.
fn available(installed: &BTreeSet<String>) -> Vec<Language> {
	let mut out: Vec<Language> = DOWNLOADS
		.iter()
		.map(|(code, _, name, _)| Language {
			code: (*code).to_owned(),
			name: (*name).to_owned(),
		})
		.collect();
	for code in installed {
		if !out.iter().any(|language| &language.code == code) {
			out.push(Language {
				code: code.clone(),
				name: code.clone(),
			});
		}
	}
	out.sort_by(|a, b| a.name.cmp(&b.name));
	out
}

/// A dictionary from the system, a previous download or LibreOffice, in that order.
fn load(code: &str, downloads: Option<&Path>) -> Option<Dictionary> {
	let mut directories = system_directories();
	directories.extend(downloads.map(Path::to_path_buf));
	for dir in directories {
		let aff = dir.join(format!("{code}.aff"));
		let dic = dir.join(format!("{code}.dic"));
		if let (Some(aff), Some(dic)) = (read_bounded(&aff), read_bounded(&dic))
			&& let Some(dictionary) = parse(&aff, &dic)
		{
			return Some(dictionary);
		}
	}
	let downloads = downloads?;
	let (_, path, ..) = DOWNLOADS.iter().find(|(known, ..)| *known == code)?;
	let aff = download(&format!("{SOURCE}/{path}.aff"))?;
	let dic = download(&format!("{SOURCE}/{path}.dic"))?;
	let dictionary = parse(&aff, &dic)?;
	std::fs::create_dir_all(downloads).ok()?;
	for (ext, bytes) in [("aff", &aff), ("dic", &dic)] {
		let target = downloads.join(format!("{code}.{ext}"));
		let partial = target.with_extension(format!("{ext}.partial"));
		std::fs::write(&partial, bytes)
			.and_then(|()| std::fs::rename(partial, target))
			.ok()?;
	}
	Some(dictionary)
}

/// Reads a dictionary file no larger than `MAX_DICTIONARY`.
fn read_bounded(path: &Path) -> Option<Vec<u8>> {
	(std::fs::metadata(path).ok()?.len() <= MAX_DICTIONARY as u64).then_some(())?;
	std::fs::read(path).ok()
}

/// Parses a Hunspell pair, decoding the encodings the offered dictionaries use.
fn parse(aff: &[u8], dic: &[u8]) -> Option<Dictionary> {
	let encoding = aff
		.split(|b| *b == b'\n')
		.find_map(|line| line.strip_prefix(b"SET"))
		.map(|rest| String::from_utf8_lossy(rest).trim().to_ascii_uppercase())
		.unwrap_or_else(|| "ISO8859-1".into());
	let decode = |bytes: &[u8]| -> Option<String> {
		match encoding.as_str() {
			"UTF-8" => String::from_utf8(bytes.to_vec()).ok(),
			// Latin-1 maps each byte to the code point of the same value.
			"ISO8859-1" | "ISO-8859-1" => Some(bytes.iter().map(|&b| char::from(b)).collect()),
			_ => None,
		}
	};
	let aff = decode(aff)?;
	let dic = decode(dic)?;
	Dictionary::new(
		aff.trim_start_matches('\u{feff}'),
		dic.trim_start_matches('\u{feff}'),
	)
	.ok()
}

/// Downloads one dictionary file over HTTPS without redirects, refusing more than
/// `MAX_DICTIONARY` bytes.
fn download(url: &str) -> Option<Vec<u8>> {
	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.ok()?;
	runtime.block_on(async {
		let client = reqwest::Client::builder()
			.redirect(reqwest::redirect::Policy::none())
			.timeout(Duration::from_secs(60))
			.build()
			.ok()?;
		let mut response = client.get(url).send().await.ok()?;
		if response.status() != reqwest::StatusCode::OK {
			return None;
		}
		let mut bytes = Vec::new();
		while let Some(chunk) = response.chunk().await.ok()? {
			bytes.extend_from_slice(&chunk);
			if bytes.len() > MAX_DICTIONARY {
				return None;
			}
		}
		Some(bytes)
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn latin1_and_utf8_dictionaries_parse_and_others_are_refused() {
		let latin1 = parse(b"SET ISO8859-1\n", b"1\nStra\xdfe\n").unwrap();
		assert!(latin1.check("Straße"));
		let utf8 = parse("SET UTF-8\n".as_bytes(), "1\nștiință\n".as_bytes()).unwrap();
		assert!(utf8.check("știință"));
		assert!(parse(b"SET ISO8859-2\n", b"1\nx\n").is_none());
	}

	#[test]
	fn detected_languages_map_to_an_offered_dictionary() {
		assert_eq!(
			dictionary_for("ron", &BTreeSet::new()).as_deref(),
			Some("ro_RO")
		);
		assert!(
			dictionary_for("eng", &BTreeSet::new()).is_some_and(|code| code.starts_with("en_"))
		);
		assert_eq!(dictionary_for("xyz", &BTreeSet::new()), None);
	}

	#[test]
	fn the_system_language_is_checked_first_then_english() {
		let installed: BTreeSet<String> = ["en_US".to_owned()].into();
		let languages = default_languages(&installed);
		assert!(languages.iter().any(|code| code == "en_US"));
		assert!(languages.len() <= 2);
		assert!(languages.iter().all(|code| valid_code(code)));
	}

	#[test]
	fn stored_settings_survive_a_round_trip() {
		let root = std::env::temp_dir().join(format!("serein-spelling-{}", std::process::id()));
		let settings = Settings {
			enabled: false,
			languages: vec!["ro_RO".into(), "../bad".into()],
			words: ["Serein".to_owned()].into(),
			excluded: ["en_US".to_owned(), "../bad".to_owned()].into(),
		};
		write(&root, &settings);
		let read = read(&root).unwrap();
		std::fs::remove_dir_all(&root).unwrap();
		assert!(!read.enabled);
		assert_eq!(read.languages, ["ro_RO"]);
		assert!(read.words.contains("Serein"));
		assert_eq!(read.excluded, ["en_US".to_owned()].into());
	}
}
