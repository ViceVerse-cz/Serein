//! Spell checking for text the user writes, like the checker Discord inherits from Chromium:
//! misspelled words get a red wavy underline and the text menu offers corrections, "Add to
//! dictionary", the on/off switch and the languages to check. The desktop app loads the
//! dictionaries and persists the settings; this side checks, suggests and draws.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

pub use spellbook::Dictionary;

/// Corrections listed in the menu, as many as Chromium offers.
const MAX_SUGGESTIONS: usize = 5;
/// Words a personal dictionary keeps; beyond this, adding asks nothing and does nothing.
pub const MAX_WORDS: usize = 10_000;
pub const MAX_WORD_BYTES: usize = 64;

/// What the user chose; the desktop app stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
	pub enabled: bool,
	/// Dictionary codes such as `en_US`, in the order they were picked.
	pub languages: Vec<String>,
	/// The personal dictionary.
	pub words: BTreeSet<String>,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			enabled: true,
			languages: Vec::new(),
			words: BTreeSet::new(),
		}
	}
}

/// A dictionary the user can pick, installed on the system or offered for download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
	pub code: String,
	pub name: String,
}

#[derive(Clone)]
struct Target {
	id: egui::Id,
	range: Range<usize>,
	word: String,
	suggestions: Vec<String>,
}

#[derive(Default)]
pub struct Spelling {
	settings: Settings,
	/// Set once the desktop app has read the stored settings; nothing is checked or saved before.
	loaded: bool,
	dirty: bool,
	available: Vec<Language>,
	dictionaries: BTreeMap<String, Arc<Dictionary>>,
	/// Languages being loaded or downloaded, shown as pending in the menu.
	pending: BTreeSet<String>,
	/// Misspelled byte ranges of the last checked text, keyed by its hash and the
	/// dictionaries' generation.
	cache: Option<(u64, u64, Vec<Range<usize>>)>,
	generation: u64,
	target: Option<Target>,
}

impl Spelling {
	/// Applies the stored settings; the desktop app calls this once at start.
	pub fn load(&mut self, settings: Settings) {
		self.settings = settings;
		self.loaded = true;
		self.generation += 1;
	}
	pub fn settings(&self) -> &Settings {
		&self.settings
	}
	/// Returns the settings once after each change, for the desktop app to save and load.
	pub fn take_changed(&mut self) -> Option<Settings> {
		std::mem::take(&mut self.dirty).then(|| self.settings.clone())
	}
	pub fn set_available(&mut self, languages: Vec<Language>) {
		self.available = languages;
	}
	pub fn set_pending(&mut self, code: &str, pending: bool) {
		if pending {
			self.pending.insert(code.to_owned());
		} else {
			self.pending.remove(code);
		}
	}
	/// Installs a loaded dictionary; one the user no longer checks is dropped.
	pub fn install(&mut self, code: String, dictionary: Arc<Dictionary>) {
		self.pending.remove(&code);
		if self.settings.languages.contains(&code) {
			self.dictionaries.insert(code, dictionary);
			self.generation += 1;
		}
	}
	pub fn loaded_languages(&self) -> impl Iterator<Item = &str> {
		self.dictionaries.keys().map(String::as_str)
	}

	fn active(&self) -> bool {
		self.loaded && self.settings.enabled && !self.dictionaries.is_empty()
	}

	fn set_enabled(&mut self, enabled: bool) {
		self.settings.enabled = enabled;
		self.changed();
	}
	fn toggle_language(&mut self, code: &str) {
		if let Some(index) = self.settings.languages.iter().position(|c| c == code) {
			self.settings.languages.remove(index);
			self.dictionaries.remove(code);
			self.pending.remove(code);
		} else {
			self.settings.languages.push(code.to_owned());
			self.pending.insert(code.to_owned());
		}
		self.changed();
	}
	fn add_word(&mut self, word: &str) {
		if self.settings.words.len() < MAX_WORDS && word.len() <= MAX_WORD_BYTES {
			self.settings.words.insert(word.to_owned());
			self.changed();
		}
	}
	fn changed(&mut self) {
		self.dirty = true;
		self.generation += 1;
		self.cache = None;
	}

	/// Whether `word` is spelled correctly in any checked language or the personal dictionary.
	pub fn check(&self, word: &str) -> bool {
		self.settings.words.contains(word)
			|| self
				.dictionaries
				.values()
				.any(|dictionary| dictionary.check(word))
	}

	/// The dictionaries ordered by how many words of `text` they know, so corrections come
	/// from the language the text is written in first, as Chromium's do.
	fn by_language_of(&self, text: &str) -> Vec<&Dictionary> {
		let mut ranked: Vec<(usize, &Dictionary)> = self
			.dictionaries
			.values()
			.map(|dictionary| {
				let known = words(text)
					.filter(|range| dictionary.check(&text[range.clone()]))
					.count();
				(known, dictionary.as_ref())
			})
			.collect();
		ranked.sort_by_key(|(known, _)| std::cmp::Reverse(*known));
		ranked
			.into_iter()
			.map(|(_, dictionary)| dictionary)
			.collect()
	}

	/// Byte ranges of misspelled words in `text`. The word still being typed at `caret`
	/// (a char index) is left alone until the user moves past it, as Chromium does.
	pub fn misspelled(&mut self, text: &str, caret: Option<usize>) -> Vec<Range<usize>> {
		if !self.active() {
			return Vec::new();
		}
		let hash = {
			use std::hash::{Hash, Hasher};
			let mut hasher = std::collections::hash_map::DefaultHasher::new();
			text.hash(&mut hasher);
			hasher.finish()
		};
		let all = match &self.cache {
			Some((cached, generation, ranges))
				if *cached == hash && *generation == self.generation =>
			{
				ranges.clone()
			}
			_ => {
				let ranges: Vec<_> = words(text)
					.filter(|range| !self.check(&text[range.clone()]))
					.collect();
				self.cache = Some((hash, self.generation, ranges.clone()));
				ranges
			}
		};
		let caret = caret.map(|chars| byte(text, chars));
		all.into_iter()
			.filter(|range| caret != Some(range.end))
			.collect()
	}

	/// Paints the wavy underline under each misspelled word of the edit's galley, whose text
	/// is `text` char for char.
	pub(crate) fn paint(
		&mut self,
		ui: &egui::Ui,
		output: &egui::text_edit::TextEditOutput,
		text: &str,
	) {
		let caret = output
			.response
			.has_focus()
			.then(|| output.cursor_range.map(|range| range.primary.index.0))
			.flatten();
		let ranges = self.misspelled(text, caret);
		if ranges.is_empty() {
			return;
		}
		let painter = ui.painter_at(output.text_clip_rect);
		let stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(0xf2, 0x3f, 0x43));
		for range in ranges {
			let start = text[..range.start].chars().count();
			let end = start + text[range].chars().count();
			for (left, right, bottom) in rows(&output.galley, start, end) {
				let offset = output.galley_pos.to_vec2();
				squiggle(
					&painter,
					left + offset.x,
					right + offset.x,
					bottom + offset.y,
					stroke,
				);
			}
		}
	}

	/// Remembers the misspelled word under a secondary click, which the text menu then corrects.
	pub(crate) fn track_pointer(
		&mut self,
		ctx: &egui::Context,
		id: egui::Id,
		output: &egui::text_edit::TextEditOutput,
		text: &str,
	) {
		if !ctx.input(|input| input.pointer.button_pressed(egui::PointerButton::Secondary)) {
			return;
		}
		self.target = None;
		if !output.response.hovered() || !self.active() {
			return;
		}
		let Some(pointer) = ctx.input(|input| input.pointer.interact_pos()) else {
			return;
		};
		let cursor = output.galley.cursor_from_pos(pointer - output.galley_pos);
		let index = byte(text, cursor.index.0);
		let Some(range) = self
			.misspelled(text, None)
			.into_iter()
			.find(|range| range.start <= index && index <= range.end)
		else {
			return;
		};
		let word = text[range.clone()].to_owned();
		let mut suggestions = Vec::new();
		for dictionary in self.by_language_of(text) {
			let mut found = Vec::new();
			dictionary.suggest(&word, &mut found);
			for suggestion in found {
				if !suggestions.contains(&suggestion) {
					suggestions.push(suggestion);
				}
			}
		}
		suggestions.truncate(MAX_SUGGESTIONS);
		self.target = Some(Target {
			id,
			range,
			word,
			suggestions,
		});
	}

	/// The spelling rows of the text menu for edit `id`, above cut, copy and paste.
	/// Returns the replacement the user picked: the misspelled byte range and its correction.
	pub(crate) fn menu(
		&mut self,
		ui: &mut egui::Ui,
		id: egui::Id,
		text: &str,
	) -> Option<(Range<usize>, String)> {
		if !self.loaded {
			return None;
		}
		let target = self.target.clone().filter(|target| {
			target.id == id && text.get(target.range.clone()) == Some(&target.word)
		});
		let mut replace = None;
		if let Some(target) = &target {
			if target.suggestions.is_empty() {
				ui.add_enabled(
					false,
					egui::Button::new(crate::i18n::translate("spelling-no-suggestions")),
				);
			}
			for suggestion in &target.suggestions {
				if ui.button(suggestion).clicked() {
					replace = Some((target.range.clone(), suggestion.clone()));
					ui.close();
				}
			}
			ui.separator();
			if ui
				.button(crate::i18n::translate("spelling-add-to-dictionary"))
				.clicked()
			{
				self.add_word(&target.word);
				ui.close();
			}
		}
		let mut enabled = self.settings.enabled;
		if ui
			.checkbox(
				&mut enabled,
				crate::i18n::translate("spelling-check-spelling"),
			)
			.changed()
		{
			self.set_enabled(enabled);
		}
		ui.menu_button(crate::i18n::translate("spelling-languages"), |ui| {
			ui.set_max_height(ui.ctx().viewport_rect().height() * 0.6);
			egui::ScrollArea::vertical().show(ui, |ui| {
				for language in self.available.clone() {
					let mut checked = self.settings.languages.contains(&language.code);
					let label = if self.pending.contains(&language.code) {
						format!("{} …", language.name)
					} else {
						language.name.clone()
					};
					if ui.checkbox(&mut checked, label).changed() {
						self.toggle_language(&language.code);
					}
				}
			});
		});
		ui.separator();
		if replace.is_some() {
			self.target = None;
		}
		replace
	}
}

/// Byte ranges of the words worth checking: runs of letters with inner apostrophes, outside
/// code, links, mentions, custom emoji and other Discord markup.
pub fn words(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
	let skipped = skipped(text);
	let mut out = Vec::new();
	let mut start = None;
	let mut iter = text.char_indices().peekable();
	while let Some((index, ch)) = iter.next() {
		let letter = ch.is_alphabetic()
			|| ((ch == '\'' || ch == '’')
				&& start.is_some()
				&& iter.peek().is_some_and(|(_, next)| next.is_alphabetic()));
		if letter {
			start.get_or_insert(index);
		} else if let Some(begin) = start.take() {
			out.push(begin..index);
		}
		if ch.is_ascii_digit() || ch == '_' {
			// A word glued to digits or underscores is an identifier, not prose.
			if let Some(last) = out.last()
				&& last.end == index
			{
				out.pop();
			}
			start = None;
			while let Some((_, next)) = iter.peek() {
				if next.is_alphanumeric() || *next == '_' {
					iter.next();
				} else {
					break;
				}
			}
		}
	}
	if let Some(begin) = start {
		out.push(begin..text.len());
	}
	out.into_iter().filter(move |range| {
		range.end - range.start > text[range.clone()].chars().next().map_or(0, char::len_utf8)
			&& !skipped
				.iter()
				.any(|skip| skip.start < range.end && range.start < skip.end)
	})
}

/// Byte ranges never checked: code, links, `<…>` mentions and timestamps, `:emoji:`, and
/// `@`/`#` names.
fn skipped(text: &str) -> Vec<Range<usize>> {
	let mut out = Vec::new();
	let bytes = text.as_bytes();
	let mut index = 0;
	while index < bytes.len() {
		let rest = &text[index..];
		let link = rest.starts_with('@')
			|| rest.starts_with('#')
			|| rest.starts_with("http://")
			|| rest.starts_with("https://")
			|| rest.starts_with("www.");
		let span = if let Some(code) = rest.strip_prefix("```") {
			Some(code.find("```").map_or(rest.len(), |end| end + 6))
		} else if let Some(code) = rest.strip_prefix('`') {
			code.find('`').map(|end| end + 2)
		} else if rest.starts_with('<') {
			rest.find(['>', ' ', '\n'])
				.filter(|&end| rest.as_bytes()[end] == b'>')
				.map(|end| end + 1)
		} else if let Some(name) = rest.strip_prefix(':') {
			name.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '~'))
				.filter(|&end| end > 0 && name[end..].starts_with(':'))
				.map(|end| end + 2)
		} else if link {
			Some(rest.find(char::is_whitespace).unwrap_or(rest.len()))
		} else {
			None
		};
		match span {
			Some(length) if length > 0 => {
				out.push(index..index + length);
				index += length;
			}
			_ => index += rest.chars().next().map_or(1, char::len_utf8),
		}
	}
	out
}

fn byte(text: &str, chars: usize) -> usize {
	text.char_indices()
		.nth(chars)
		.map_or(text.len(), |(index, _)| index)
}

/// Spans of chars `start..end` per galley row: left, right and baseline bottom, in galley space.
fn rows(galley: &egui::Galley, start: usize, end: usize) -> Vec<(f32, f32, f32)> {
	let mut out: Vec<(f32, f32, f32)> = Vec::new();
	for index in start..end {
		let rect = galley.pos_from_cursor(egui::text::CCursor::new(index));
		let next = galley.pos_from_cursor(egui::text::CCursor::new(index + 1));
		let right = if (next.min.y - rect.min.y).abs() < 0.5 {
			next.min.x
		} else {
			rect.max.x
		};
		match out.last_mut() {
			Some(last) if (last.2 - rect.max.y).abs() < 0.5 => last.1 = right,
			_ => out.push((rect.min.x, right, rect.max.y)),
		}
	}
	out
}

fn squiggle(painter: &egui::Painter, left: f32, right: f32, bottom: f32, stroke: egui::Stroke) {
	const STEP: f32 = 2.0;
	let y = bottom - 1.0;
	let mut points = Vec::new();
	let mut x = left;
	let mut up = false;
	while x < right {
		points.push(egui::pos2(
			x,
			if up { y - STEP / 2.0 } else { y + STEP / 2.0 },
		));
		up = !up;
		x += STEP;
	}
	points.push(egui::pos2(
		right,
		if up { y - STEP / 2.0 } else { y + STEP / 2.0 },
	));
	if points.len() > 1 {
		painter.add(egui::Shape::line(points, stroke));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn found(text: &str) -> Vec<&str> {
		words(text).map(|range| &text[range]).collect()
	}

	#[test]
	fn markup_code_and_identifiers_are_not_checked() {
		assert_eq!(
			found(
				"Helo <@123> :smile: `codde` https://exampel.com @frend #chanel don't x2 a_b wordd"
			),
			["Helo", "don't", "wordd"]
		);
		assert_eq!(found("```\nfn mian()\n``` aici"), ["aici"]);
		assert_eq!(found("ăștia sunt bine"), ["ăștia", "sunt", "bine"]);
	}

	fn spelling(words: &[&str]) -> Spelling {
		let aff = "SET UTF-8\n";
		let dic = format!("{}\n{}\n", words.len(), words.join("\n"));
		let mut spelling = Spelling::default();
		spelling.load(Settings {
			enabled: true,
			languages: vec!["xx".into()],
			words: BTreeSet::new(),
		});
		spelling.install("xx".into(), Arc::new(Dictionary::new(aff, &dic).unwrap()));
		spelling
	}

	#[test]
	fn misspelled_words_skip_the_one_being_typed_and_the_personal_dictionary() {
		let mut spelling = spelling(&["hello", "world"]);
		let text = "hello wrold worl";
		let marked: Vec<_> = spelling
			.misspelled(text, None)
			.into_iter()
			.map(|range| &text[range])
			.collect();
		assert_eq!(marked, ["wrold", "worl"]);
		// The caret at the end of "worl" means it is still being typed.
		let typing: Vec<_> = spelling
			.misspelled(text, Some(text.chars().count()))
			.into_iter()
			.map(|range| &text[range])
			.collect();
		assert_eq!(typing, ["wrold"]);
		spelling.add_word("wrold");
		assert_eq!(spelling.take_changed().unwrap().words.len(), 1);
		assert!(
			spelling
				.misspelled(text, Some(text.chars().count()))
				.is_empty()
		);
		spelling.set_enabled(false);
		assert!(spelling.misspelled(text, None).is_empty());
	}

	#[test]
	fn corrections_come_from_the_language_the_text_is_written_in() {
		let mut spelling = Spelling::default();
		spelling.load(Settings {
			enabled: true,
			languages: vec!["en".into(), "ro".into()],
			words: BTreeSet::new(),
		});
		let dictionary = |words: &[&str]| {
			let dic = format!("{}\n{}\n", words.len(), words.join("\n"));
			Arc::new(Dictionary::new("SET UTF-8\n", &dic).unwrap())
		};
		spelling.install("en".into(), dictionary(&["case", "the", "house"]));
		spelling.install("ro".into(), dictionary(&["casa", "este", "mare"]));
		let first = |text: &str| {
			let mut found = Vec::new();
			spelling.by_language_of(text)[0].suggest("cas", &mut found);
			found
		};
		assert!(first("casa este mare cas").contains(&"casa".to_owned()));
		assert!(first("the house cas").contains(&"case".to_owned()));
	}

	#[test]
	fn nothing_is_checked_before_settings_load_or_after_the_language_is_removed() {
		let mut spelling = Spelling::default();
		assert!(spelling.misspelled("wrold", None).is_empty());
		let mut spelling = self::tests::spelling(&["hello"]);
		assert_eq!(spelling.misspelled("wrold", None).len(), 1);
		spelling.toggle_language("xx");
		assert!(spelling.misspelled("wrold", None).is_empty());
		assert!(spelling.take_changed().unwrap().languages.is_empty());
	}
}
