//! Bundled OFL fallback faces. Eframe separately provides native system-font fallback.
use egui::{Context, FontData, FontDefinitions, FontFamily};
use std::sync::{Arc, Mutex, Weak};

pub const MAX_CUSTOM_FONT_BYTES: usize = 32 * 1024 * 1024;
const CUSTOM: [&str; 3] = [
	"Serein Custom",
	"Serein Custom Medium",
	"Serein Custom SemiBold",
];
const DEFINITIONS_KEY: &str = "serein-font-definitions";

#[derive(Clone)]
pub struct CustomFont {
	pub name: String,
	data: FontData,
}

impl CustomFont {
	/// Validate before handing user-selected bytes to the renderer. Called off the UI thread.
	pub fn new(mut name: String, mut bytes: Vec<u8>) -> Result<Self, &'static str> {
		use skrifa::{MetadataProvider, raw::TableProvider};
		if bytes.is_empty() || bytes.len() > MAX_CUSTOM_FONT_BYTES {
			return Err("Choose a font up to 32 MiB.");
		}
		if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
			return Err("The font name is invalid.");
		}
		let font = skrifa::FontRef::new(&bytes).map_err(|_| "Choose a valid TTF or OTF font.")?;
		if font.head().map_or(true, |head| head.units_per_em() == 0)
			|| font.hhea().is_err()
			|| font.maxp().is_err()
			|| font.hmtx().is_err()
			|| font.charmap().mappings().next().is_none()
			|| font.outline_glyphs().format().is_none()
		{
			return Err("This font is missing readable text or outlines.");
		}
		bytes.shrink_to_fit();
		name.shrink_to_fit();
		let mut data = FontData::from_owned(bytes);
		data.tweak.hinting = Some(false);
		data.tweak.subpixel_binning = Some(true);
		Ok(Self { name, data })
	}
	pub fn bytes(&self) -> &[u8] {
		self.data.bytes()
	}
}

/// One face as a standalone font file. A collection (`.ttc`) member is copied out with its own
/// table directory, so the saved copy restores without a face index.
pub fn standalone_face(data: &[u8], index: u32) -> Result<Vec<u8>, &'static str> {
	use skrifa::raw::TableProvider as _;
	const INVALID: &str = "This font cannot be read.";
	let font = skrifa::FontRef::from_index(data, index).map_err(|_| INVALID)?;
	let _ = font.head().map_err(|_| INVALID)?;
	let records = font.table_directory().table_records();
	if records.is_empty() || records.len() > 1024 {
		return Err(INVALID);
	}
	let header = 12 + 16 * records.len();
	let size = records.iter().try_fold(header, |size, record| {
		size.checked_add((record.length() as usize).next_multiple_of(4))
	});
	if size.is_none_or(|size| size > MAX_CUSTOM_FONT_BYTES) {
		return Err("Choose a font up to 32 MiB.");
	}
	let count = records.len() as u16;
	let selector = 15 - count.leading_zeros() as u16;
	let range = 16u16 << selector;
	let mut bytes = Vec::with_capacity(size.unwrap_or_default());
	bytes.extend(font.table_directory().sfnt_version().to_be_bytes());
	for value in [count, range, selector, count * 16 - range] {
		bytes.extend(value.to_be_bytes());
	}
	let mut offset = header;
	for record in records {
		bytes.extend(record.tag().to_be_bytes());
		bytes.extend(record.checksum().to_be_bytes());
		bytes.extend((offset as u32).to_be_bytes());
		bytes.extend(record.length().to_be_bytes());
		offset += (record.length() as usize).next_multiple_of(4);
	}
	for record in records {
		let start = record.offset() as usize;
		let table = start
			.checked_add(record.length() as usize)
			.and_then(|end| data.get(start..end))
			.ok_or(INVALID)?;
		bytes.extend_from_slice(table);
		bytes.resize(bytes.len().next_multiple_of(4), 0);
	}
	Ok(bytes)
}

pub enum Action {
	/// Enumerate the installed families once the font settings are first shown.
	List,
	Select(String),
	Reset,
}

#[derive(Default)]
pub struct Settings {
	pub name: Option<String>,
	pub busy: bool,
	pub status: &'static str,
	pub request: Option<Action>,
	/// Installed family names, sorted; `None` until the platform list arrives.
	pub families: Option<Vec<String>>,
	listed: bool,
	query: String,
	/// Focus the search and reveal the current family on the pass after the picker opens.
	focus: bool,
}

#[cfg(test)]
thread_local! {
	/// Visible and content height of the font list on the last pass, for tests.
	static LIST: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

impl Settings {
	pub(super) fn show(&mut self, ui: &mut egui::Ui) {
		use crate::design;
		if !std::mem::replace(&mut self.listed, true) {
			self.request = Some(Action::List);
		}
		design::group(ui, &crate::i18n::translate("fonts-show-typography"), |ui| {
			ui.add_enabled_ui(!self.busy, |ui| {
				design::row(ui, "fonts-show-interface-font", None, |ui| {
					if self.name.is_some()
						&& design::text_action(ui, &crate::i18n::translate("fonts-show-reset"))
							.clicked()
					{
						self.request = Some(Action::Reset);
					}
				});
				self.family_list(ui);
			});
			design::hint(
				ui,
				&crate::i18n::translate("fonts-show-ttf-or-otf-up-to-32-mib-saved-on-this"),
			);
			ui.label(crate::i18n::translate(
				"fonts-show-the-quick-brown-fox-jumps-over-the-lazy-dog-0123456789",
			));
			if !self.status.is_empty() {
				design::hint(ui, self.status);
			}
		});
	}

	/// Select-style picker; the searchable family list opens in a popup so the settings page
	/// never contains a nested scroll area.
	fn family_list(&mut self, ui: &mut egui::Ui) {
		use crate::design;
		use crate::icons::{Icon, paint};
		const ROW: f32 = 32.0;
		const SELECT: f32 = design::SELECT_HEIGHT;
		let colors = design::palette(ui);
		ui.add_space(4.0);
		let Some(families) = &self.families else {
			// The family list opens from this select, so the select is what stands in for it.
			crate::skeleton::custom(
				ui,
				&crate::i18n::translate("fonts-show-loading"),
				egui::vec2(ui.available_width(), SELECT),
				|painter, rect, fill| {
					// Field-shaped, like the select it stands in for.
					crate::skeleton::control(painter, fill, rect, crate::skeleton::CONTROL_RADIUS);
				},
			);
			return;
		};
		let default = crate::i18n::translate("fonts-show-inter-default");
		let current = self.name.clone().unwrap_or_else(|| default.clone());
		let (rect, button) = ui.allocate_exact_size(
			egui::vec2(ui.available_width(), SELECT),
			egui::Sense::click(),
		);
		button.widget_info(|| {
			egui::WidgetInfo::labeled(egui::Role::ComboBox, button.enabled(), &current)
		});
		let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&button));
		let stroke = if open {
			egui::Stroke::new(2.0, colors.accent)
		} else if button.hovered() {
			egui::Stroke::new(1.0, colors.muted)
		} else {
			egui::Stroke::new(1.0, colors.border)
		};
		ui.painter()
			.rect(rect, 8, colors.base, stroke, egui::StrokeKind::Inside);
		ui.painter().text(
			rect.left_center() + egui::vec2(12.0, 0.0),
			egui::Align2::LEFT_CENTER,
			&current,
			egui::FontId::proportional(15.0),
			colors.text_strong,
		);
		paint(
			ui.painter(),
			Icon::ChevronDown,
			egui::Rect::from_center_size(
				rect.right_center() - egui::vec2(20.0, 0.0),
				egui::vec2(14.0, 14.0),
			),
			colors.muted,
		);
		if button.clicked() {
			self.query.clear();
			self.focus = true;
		}
		let mut picked = None;
		egui::Popup::menu(&button)
			.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
			.width(rect.width())
			.gap(6.0)
			.frame(
				egui::Frame::popup(ui.style())
					.fill(colors.base)
					.stroke(egui::Stroke::new(1.0, colors.border))
					.inner_margin(6)
					.corner_radius(8),
			)
			.show(|ui| {
				ui.set_width(rect.width() - 12.0);
				let search = design::input(
					ui,
					egui::TextEdit::singleline(&mut self.query)
						.align(egui::Align2::LEFT_CENTER)
						.hint_text(crate::i18n::translate("fonts-show-search"))
						.char_limit(64),
				);
				paint(
					ui.painter(),
					Icon::Search,
					egui::Rect::from_center_size(
						search.rect.right_center() - egui::vec2(18.0, 0.0),
						egui::vec2(14.0, 14.0),
					),
					colors.muted,
				);
				let query = self.query.trim().to_lowercase();
				// `None` is the bundled default, listed first while the search is empty.
				let matches: Vec<Option<&String>> = query
					.is_empty()
					.then_some(None)
					.into_iter()
					.chain(
						families
							.iter()
							.filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
							.map(Some),
					)
					.collect();
				ui.add_space(4.0);
				if matches.is_empty() {
					ui.add_space(6.0);
					design::hint(
						ui,
						&crate::i18n::translate(if families.is_empty() {
							"fonts-show-none-installed"
						} else {
							"fonts-show-no-match"
						}),
					);
					ui.add_space(6.0);
					return;
				}
				let mut area = egui::ScrollArea::vertical()
					.id_salt("installed-fonts")
					.max_height(ROW * 8.0)
					// The popup keeps last frame's size, so after a search with no match it is only as
					// tall as the hint. Without this minimum the list would stay at egui's default
					// 64 px (two rows) once results return.
					.min_scrolled_height((matches.len() as f32 * ROW).min(ROW * 8.0))
					.auto_shrink([false, true]);
				if std::mem::take(&mut self.focus) {
					search.request_focus();
					let index = matches
						.iter()
						.position(|name| name.map(String::as_str) == self.name.as_deref())
						.unwrap_or(0);
					area = area.vertical_scroll_offset((index as f32 - 3.0).max(0.0) * ROW);
				}
				// `show_rows` counts the spacing in the content height before the closure runs, so
				// clear it first; otherwise a short list overflows its minimum height and scrolls.
				ui.spacing_mut().item_spacing.y = 0.0;
				let list = area.show_rows(ui, ROW, matches.len(), |ui, rows| {
					for name in &matches[rows] {
						let selected = name.map(String::as_str) == self.name.as_deref();
						let (rect, response) = ui.allocate_exact_size(
							egui::vec2(ui.available_width(), ROW),
							egui::Sense::click(),
						);
						let label = name.map_or(default.as_str(), String::as_str);
						response.widget_info(|| {
							egui::WidgetInfo::selected(egui::Role::Button, true, selected, label)
						});
						let fill = if selected {
							colors.accent.gamma_multiply(0.22)
						} else if response.hovered() {
							colors.hover
						} else {
							egui::Color32::TRANSPARENT
						};
						ui.painter().rect_filled(rect, 6, fill);
						ui.painter().text(
							rect.left_center() + egui::vec2(10.0, 0.0),
							egui::Align2::LEFT_CENTER,
							label,
							egui::FontId::proportional(14.0),
							if selected {
								colors.text_strong
							} else {
								colors.text
							},
						);
						if selected {
							paint(
								ui.painter(),
								Icon::Check,
								egui::Rect::from_center_size(
									rect.right_center() - egui::vec2(16.0, 0.0),
									egui::vec2(14.0, 14.0),
								),
								colors.accent,
							);
						}
						if response.clicked() {
							if !selected {
								picked = Some(name.cloned());
							}
							ui.close();
						}
					}
				});
				#[cfg(test)]
				LIST.set(Some((list.inner_rect.height(), list.content_size.y)));
				#[cfg(not(test))]
				let _ = list;
			});
		match picked {
			Some(Some(name)) => self.request = Some(Action::Select(name)),
			Some(None) => self.request = Some(Action::Reset),
			None => {}
		}
	}
}

/// Use the active definitions so layout caches change on the same pass as egui's fonts.
pub fn revision(ctx: &Context) -> (usize, usize, u32) {
	ctx.fonts(|fonts| {
		let data = &fonts.definitions().font_data;
		(
			data.len(),
			data.get(CUSTOM[0])
				.map_or(0, |font| Arc::as_ptr(font) as usize),
			data.get("Noto Sans CJK").map_or(0, |font| font.index),
		)
	})
}

pub fn apply_custom(ctx: &Context, font: Option<&CustomFont>) {
	let shared = ctx.data(|data| {
		data.get_temp::<Arc<Mutex<FontDefinitions>>>(egui::Id::unique(DEFINITIONS_KEY))
	});
	let Some(shared) = shared else { return };
	let mut definitions = shared.lock().expect("font definitions");
	for (family, name, weight) in [
		(FontFamily::Proportional, CUSTOM[0], 400.0),
		(
			FontFamily::Name(crate::design::MEDIUM.into()),
			CUSTOM[1],
			500.0,
		),
		(
			FontFamily::Name(crate::design::SEMIBOLD.into()),
			CUSTOM[2],
			600.0,
		),
	] {
		definitions.font_data.remove(name);
		definitions
			.families
			.entry(family.clone())
			.or_default()
			.retain(|entry| entry != name);
		if let Some(font) = font {
			let mut data = font.data.clone();
			// Static faces keep their supplied weight; variable faces use the UI's three weights.
			data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
			definitions.font_data.insert(name.into(), data.into());
			definitions
				.families
				.entry(family)
				.or_default()
				.insert(0, name.into());
		}
	}
	sync_name_families(&mut definitions);
	ctx.set_fonts(definitions.clone());
	ctx.request_repaint();
}

/// The regional faces share one collection, decoded off-thread on the first CJK text.
/// Latin-only sessions never pay for the decode; language changes reuse the same bytes.
const CJK_ZSTD: &[u8] = include_bytes!("../../../assets/fonts/NotoSansCJK-Regular.ttc.zst");
const CJK_BYTES: usize = 19_484_784;
const ARABIC: &[u8] = include_bytes!("../../../assets/fonts/NotoSansArabic.ttf");
const MATH: &[u8] = include_bytes!("../../../assets/fonts/NotoSansMath-Regular.otf");
/// Subset of Noto Sans Symbols 2 (punctuation, arrows, technical, miscellaneous
/// symbols and dingbats incl. U+2726 BLACK FOUR POINTED STAR). Inter, the CJK,
/// Arabic and Math faces all lack that scalar, so without this fallback egui
/// paints Inter's `.notdef` (a stack of horizontal bars) wherever it appears.
const SYMBOLS: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSymbols2-Regular.ttf");
const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");

/// Discord display-name fonts by `display_name_styles.font_id`. Each is installed the first time
/// a styled name needs it, so sessions with name styles off never parse them. Néo-Castel (7) is
/// sold only through a checkout and Hexagon (17) has no identified source; both, Default (11)
/// and unknown ids keep the interface font.
const NAME_FONTS: [(u32, &str, &[u8]); 14] = [
	(
		1,
		"Serein Name Bangers",
		include_bytes!("../../../assets/fonts/Bangers-Regular.ttf"),
	),
	(
		2,
		"Serein Name BioRhyme",
		include_bytes!("../../../assets/fonts/BioRhyme.ttf"),
	),
	(
		3,
		"Serein Name Cherry Bomb",
		include_bytes!("../../../assets/fonts/CherryBombOne-Regular.ttf"),
	),
	(
		4,
		"Serein Name Chicle",
		include_bytes!("../../../assets/fonts/Chicle-Regular.ttf"),
	),
	(
		5,
		"Serein Name Compagnon",
		include_bytes!("../../../assets/fonts/Compagnon-Roman.otf"),
	),
	(
		6,
		"Serein Name MuseoModerno",
		include_bytes!("../../../assets/fonts/MuseoModerno.ttf"),
	),
	(
		8,
		"Serein Name Pixelify",
		include_bytes!("../../../assets/fonts/PixelifySans.ttf"),
	),
	(
		9,
		"Serein Name Ribes",
		include_bytes!("../../../assets/fonts/Ribes-Regular.otf"),
	),
	(
		10,
		"Serein Name Sinistre",
		include_bytes!("../../../assets/fonts/Sinistre-Regular.otf"),
	),
	(
		12,
		"Serein Name Zilla Slab",
		include_bytes!("../../../assets/fonts/ZillaSlab-Regular.ttf"),
	),
	(
		13,
		"Serein Name Playpen Sans",
		include_bytes!("../../../assets/fonts/PlaypenSans.ttf"),
	),
	(
		14,
		"Serein Name Orbitron",
		include_bytes!("../../../assets/fonts/Orbitron.ttf"),
	),
	(
		15,
		"Serein Name New Rocker",
		include_bytes!("../../../assets/fonts/NewRocker-Regular.ttf"),
	),
	(
		16,
		"Serein Name Kalam",
		include_bytes!("../../../assets/fonts/Kalam-Regular.ttf"),
	),
];

/// The family for a name font, once it is active; otherwise queues its install for the next
/// pass and returns `None`, so this pass keeps the interface font.
pub(crate) fn name_family(ctx: &Context, font: u32) -> Option<FontFamily> {
	static FAMILIES: std::sync::OnceLock<Vec<FontFamily>> = std::sync::OnceLock::new();
	let index = NAME_FONTS.iter().position(|(id, ..)| *id == font)?;
	let family = &FAMILIES.get_or_init(|| {
		NAME_FONTS
			.iter()
			.map(|(_, name, _)| FontFamily::Name((*name).into()))
			.collect()
	})[index];
	if ctx.fonts(|fonts| fonts.definitions().families.contains_key(family)) {
		return Some(family.clone());
	}
	let shared = ctx.data(|data| {
		data.get_temp::<Arc<Mutex<FontDefinitions>>>(egui::Id::unique(DEFINITIONS_KEY))
	})?;
	let mut definitions = shared.lock().expect("font definitions");
	if !definitions.families.contains_key(family) {
		add_name_font(&mut definitions, family.clone(), index);
		ctx.set_fonts(definitions.clone());
		ctx.request_repaint();
	}
	None
}

/// The name face leads; the medium interface family covers scripts the face lacks.
fn add_name_font(definitions: &mut FontDefinitions, family: FontFamily, index: usize) {
	let (_, name, data) = NAME_FONTS[index];
	let mut data = latin(data);
	// Variable faces would otherwise draw their default instance; match the medium name weight.
	data.tweak.coords = egui::epaint::text::VariationCoords::new([(b"wght", 500.0)]);
	definitions.font_data.insert(name.into(), data.into());
	definitions.families.insert(family, vec![name.to_owned()]);
	sync_name_families(definitions);
}

/// Each installed name family is its face followed by the current medium family, so a custom
/// interface font that is set or removed later never leaves a name family pointing at it.
fn sync_name_families(definitions: &mut FontDefinitions) {
	let medium = definitions
		.families
		.get(&FontFamily::Name(crate::design::MEDIUM.into()))
		.cloned()
		.unwrap_or_default();
	for (_, name, _) in NAME_FONTS {
		if let Some(list) = definitions.families.get_mut(&FontFamily::Name(name.into())) {
			list.truncate(1);
			list.extend(medium.iter().cloned());
		}
	}
}

const CHECKED_JOBS: usize = 512;
const CHECKED_BYTES: usize = 128 * 1024;
// Weak references retain only the fixed-size Arc allocation, never text or meshes.
const _: () = assert!(
	CHECKED_JOBS * (size_of::<egui::text::LayoutJob>() + 3 * size_of::<usize>()) <= CHECKED_BYTES
);

fn is_cjk(c: char) -> bool {
	matches!(
		c as u32,
		0x1100..=0x11ff
			| 0x2e80..=0xa4cf
			| 0xa960..=0xa97f
			| 0xac00..=0xd7af
			| 0xd7b0..=0xd7ff
			| 0xf900..=0xfaff
			| 0xfe30..=0xffef
			| 0x20000..=0x323af
	)
}

const MAX_PASS_CJK_CHARS: usize = 64;
const HASH_INIT: u64 = 0xcbf2_9ce4_8422_2325;

fn mix_job(hash: u64, job: &Arc<egui::text::LayoutJob>) -> u64 {
	(hash ^ (Arc::as_ptr(job) as u64)).wrapping_mul(0x0100_0000_01b3)
}

fn shape_fingerprint(shape: &egui::Shape, count: &mut usize, hash: &mut u64) {
	match shape {
		egui::Shape::Text(text) => {
			*count += 1;
			*hash = mix_job(*hash, &text.galley.job);
		}
		egui::Shape::Vec(shapes) => {
			for shape in shapes {
				shape_fingerprint(shape, count, hash);
			}
		}
		_ => {}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PassOutcome {
	/// Every job in the frame has been inspected without truncation.
	Complete,
	/// Stopped early at `MAX_PASS_CJK_CHARS` or saw the skipped prefix change; repaint to continue.
	Continue,
	/// The view shrank past the saved cursor before scanning any job; rescan from 0 in this pass.
	RescanFromStart,
}

#[derive(Default)]
struct CjkScan {
	checked: Vec<Weak<egui::text::LayoutJob>>,
	layers: Vec<egui::LayerId>,
	chars: Vec<(FontFamily, char)>,
	truncated: bool,
	seen: usize,
	resume_from: usize,
	pass_hash: u64,
	prefix_hash: u64,
	prefix_dirty: bool,
	rolled_over: bool,
	completed_frame: Option<(usize, u64, (usize, usize, u32))>,
}

impl CjkScan {
	fn begin_pass(&mut self) {
		self.chars.clear();
		self.truncated = false;
		self.seen = 0;
		self.pass_hash = HASH_INIT;
		if self.resume_from == 0 {
			self.prefix_hash = HASH_INIT;
			self.prefix_dirty = false;
		}
	}

	fn finish_pass(&mut self, font_revision: (usize, usize, u32)) -> PassOutcome {
		if self.truncated {
			return PassOutcome::Continue;
		}
		let skipped_all = self.resume_from > 0 && self.seen <= self.resume_from;
		let dirty_prefix = self.resume_from > 0 && self.prefix_dirty;
		self.resume_from = 0;
		self.prefix_hash = HASH_INIT;
		self.prefix_dirty = false;
		if skipped_all {
			self.rolled_over = false;
			return PassOutcome::RescanFromStart;
		}
		if dirty_prefix {
			self.rolled_over = false;
			return PassOutcome::Continue;
		}
		if self.rolled_over {
			self.completed_frame = Some((self.seen, self.pass_hash, font_revision));
			self.rolled_over = false;
		}
		PassOutcome::Complete
	}

	fn push_cjk(&mut self, family: &FontFamily, text: &str) {
		if text.is_ascii() {
			return;
		}
		for ch in text.chars().filter(|c| is_cjk(*c)) {
			if !self.chars.iter().any(|(existing_family, existing_ch)| {
				*existing_ch == ch && existing_family == family
			}) {
				self.chars.push((family.clone(), ch));
			}
		}
	}

	fn scan_job(&mut self, job: &Arc<egui::text::LayoutJob>) {
		if self.truncated {
			return;
		}
		let position = self.seen;
		self.seen += 1;
		if position < self.resume_from {
			self.pass_hash = mix_job(self.pass_hash, job);
			if self.seen == self.resume_from && self.pass_hash != self.prefix_hash {
				self.prefix_dirty = true;
			}
			return;
		}
		let index = match self
			.checked
			.binary_search_by_key(&(Arc::as_ptr(job) as usize), |entry| {
				entry.as_ptr() as usize
			}) {
			Ok(_) => {
				self.pass_hash = mix_job(self.pass_hash, job);
				return;
			}
			Err(index) => index,
		};
		if self.chars.len() >= MAX_PASS_CJK_CHARS {
			self.truncated = true;
			self.resume_from = position;
			self.prefix_hash = self.pass_hash;
			return;
		}
		if !job.text.is_ascii() {
			if job.sections.is_empty() {
				self.push_cjk(&FontFamily::Proportional, &job.text);
			} else {
				for section in &job.sections {
					let range = section.byte_range.start.0..section.byte_range.end.0;
					if let Some(text) = job.text.get(range) {
						self.push_cjk(&section.format.font_id.family, text);
					}
				}
			}
		}
		// Keep allocation identities alive so allocator address reuse cannot hide new text.
		// Arc::make_mut also dissociates these weak references before editing a job.
		if self.checked.capacity() == 0 {
			self.checked.reserve_exact(CHECKED_JOBS);
		}
		// ponytail: clear the fixed cache at capacity; unusually busy views rely on the
		// traversal cursor and rollover fingerprint so continuation still converges.
		let index = if self.checked.len() == CHECKED_JOBS {
			self.checked.clear();
			self.rolled_over = true;
			0
		} else {
			index
		};
		self.checked.insert(index, Arc::downgrade(job));
		self.pass_hash = mix_job(self.pass_hash, job);
	}

	#[cfg(test)]
	fn text(&mut self, job: &Arc<egui::text::LayoutJob>) -> bool {
		self.resume_from = 0;
		self.completed_frame = None;
		self.begin_pass();
		self.scan_job(job);
		!self.chars.is_empty()
	}

	fn shape(&mut self, shape: &egui::Shape) {
		if self.truncated {
			return;
		}
		match shape {
			egui::Shape::Text(text) => self.scan_job(&text.galley.job),
			egui::Shape::Vec(shapes) => {
				for shape in shapes {
					self.shape(shape);
					if self.truncated {
						break;
					}
				}
			}
			_ => {}
		}
	}
}

/// Install once during application creation, before the first UI pass.
pub fn install(ctx: &Context) {
	let shared = Arc::new(Mutex::new(definitions(false)));
	ctx.set_fonts(shared.lock().expect("font definitions").clone());
	ctx.data_mut(|data| data.insert_temp(egui::Id::unique(DEFINITIONS_KEY), shared.clone()));
	let installed = std::sync::atomic::AtomicBool::new(false);
	let scan = Mutex::new(CjkScan::default());
	ctx.on_end_pass(
		"CJK fallback",
		std::sync::Arc::new(move |ui| {
			// Also true while the decode thread runs, so the scan stops after the first hit.
			if installed.load(std::sync::atomic::Ordering::Relaxed) {
				update_cjk(ui.ctx(), &shared, cjk_index(crate::i18n::current()));
				return;
			}
			let mut scan = scan.lock().expect("CJK scan");
			let ctx = ui.ctx();
			let mut layers = std::mem::take(&mut scan.layers);
			layers.clear();
			ctx.memory(|memory| layers.extend(memory.layer_ids()));
			let font_revision = revision(ctx);
			if let Some(expected) = scan.completed_frame {
				let mut count = 0usize;
				let mut hash = HASH_INIT;
				ctx.graphics(|graphics| {
					for layer in &layers {
						if let Some(list) = graphics.get(*layer) {
							for entry in list.all_entries() {
								shape_fingerprint(&entry.shape, &mut count, &mut hash);
							}
						}
					}
				});
				if (count, hash, font_revision) == expected {
					if layers.capacity() > 64 {
						layers.shrink_to(64);
					}
					scan.layers = layers;
					return;
				}
				scan.completed_frame = None;
			}
			let traverse = |scan: &mut CjkScan| {
				scan.begin_pass();
				ctx.graphics(|graphics| {
					for layer in &layers {
						if let Some(list) = graphics.get(*layer) {
							for entry in list.all_entries() {
								scan.shape(&entry.shape);
								if scan.truncated {
									return;
								}
							}
						}
					}
				});
			};
			traverse(&mut scan);
			let outcome = match scan.finish_pass(font_revision) {
				PassOutcome::RescanFromStart => {
					traverse(&mut scan);
					scan.finish_pass(font_revision)
				}
				outcome => outcome,
			};
			// Prefer mmap'd system CJK fonts discovered by egui_system_font_provider; only
			// inflate the 19 MiB bundled archive when a required family lacks a glyph yet.
			let needed = !scan.chars.is_empty()
				&& ctx.fonts_mut(|fonts| {
					scan.chars.iter().any(|(family, ch)| {
						!fonts.has_glyph(&egui::FontId::new(14.0, family.clone()), *ch)
					})
				});
			if needed {
				*scan = CjkScan::default();
				installed.store(true, std::sync::atomic::Ordering::Relaxed);
				// Inflating 19 MB and reparsing the font set takes tens of milliseconds; keep
				// it off the UI thread and accept one pass of fallback glyphs.
				let worker = ctx.clone();
				let definitions = shared.clone();
				let spawned =
					std::thread::Builder::new()
						.name("cjk-font".into())
						.spawn(move || {
							install_cjk(&worker, &definitions);
							worker.request_repaint();
						});
				if spawned.is_err() {
					install_cjk(ctx, &shared);
					ctx.request_repaint();
				}
			} else {
				if matches!(outcome, PassOutcome::Continue) {
					ctx.request_repaint();
				}
				scan.chars.clear();
				if scan.chars.capacity() > 256 {
					scan.chars.shrink_to(256);
				}
				if layers.capacity() > 64 {
					layers.shrink_to(64);
				}
				scan.layers = layers;
			}
		}),
	);
	crate::design::weights_installed(ctx);
}

fn install_cjk(ctx: &Context, shared: &Mutex<FontDefinitions>) {
	let mut data = FontData::from_owned(cjk());
	let mut definitions = shared.lock().expect("font definitions");
	data.index = cjk_index(crate::i18n::current());
	add_fallback(&mut definitions, "Noto Sans CJK", data);
	ctx.set_fonts(definitions.clone());
}

fn cjk_index(language: crate::i18n::Language) -> u32 {
	match language.resolved() {
		crate::i18n::Language::Korean => 1,
		crate::i18n::Language::ChineseSimplified => 2,
		crate::i18n::Language::ChineseTraditional => 3,
		_ => 0,
	}
}

fn update_cjk(ctx: &Context, shared: &Mutex<FontDefinitions>, index: u32) {
	let mut definitions = shared.lock().expect("font definitions");
	let Some(data) = definitions.font_data.get_mut("Noto Sans CJK") else {
		return;
	};
	if data.index != index {
		Arc::make_mut(data).index = index;
		ctx.set_fonts(definitions.clone());
		ctx.request_repaint();
	}
}

fn add_fallback(definitions: &mut FontDefinitions, name: &str, data: FontData) {
	definitions.font_data.insert(name.into(), data.into());
	for family in [
		FontFamily::Proportional,
		FontFamily::Monospace,
		FontFamily::Name(crate::design::MEDIUM.into()),
		FontFamily::Name(crate::design::SEMIBOLD.into()),
	] {
		definitions
			.families
			.entry(family)
			.or_default()
			.push(name.into());
	}
	for (family, list) in &mut definitions.families {
		if matches!(family, FontFamily::Name(family) if family.starts_with("Serein Name ")) {
			list.push(name.into());
		}
	}
}

fn latin(data: &'static [u8]) -> FontData {
	let mut font = FontData::from_static(data);
	font.tweak.hinting = Some(false);
	font.tweak.subpixel_binning = Some(true);
	font
}

#[cfg(test)]
std::thread_local! {
	static CJK_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The bundled archive always inflates; a corrupt asset is a build defect, not a runtime path.
fn cjk() -> Vec<u8> {
	#[cfg(test)]
	CJK_DECODES.with(|count| count.set(count.get() + 1));
	let mut font = Vec::with_capacity(CJK_BYTES);
	ruzstd::decoding::FrameDecoder::new()
		.decode_all_to_vec(CJK_ZSTD, &mut font)
		.expect("bundled CJK font archive");
	font
}

fn definitions(with_cjk: bool) -> FontDefinitions {
	let mut definitions = FontDefinitions::default();
	// Inter leads proportional text; two heavier faces provide Discord-style emphasis
	// (egui has no synthetic bold). Each weight family falls back to egui's defaults.
	let weights = [
		(FontFamily::Proportional, "Inter", INTER),
		(
			FontFamily::Name(crate::design::MEDIUM.into()),
			"Inter Medium",
			INTER_MEDIUM,
		),
		(
			FontFamily::Name(crate::design::SEMIBOLD.into()),
			"Inter SemiBold",
			INTER_SEMIBOLD,
		),
	];
	let defaults = definitions.families[&FontFamily::Proportional].clone();
	for (family, name, data) in weights {
		definitions
			.font_data
			.insert(name.into(), latin(data).into());
		let list = definitions.families.entry(family).or_default();
		list.retain(|existing| !defaults.contains(existing));
		list.insert(0, name.into());
		list.extend(defaults.iter().cloned());
	}
	for (name, data) in with_cjk
		.then(|| {
			let mut data = FontData::from_owned(cjk());
			data.index = cjk_index(crate::i18n::current());
			("Noto Sans CJK", data)
		})
		.into_iter()
		.chain([
			("Noto Sans Arabic", FontData::from_static(ARABIC)),
			("Noto Sans Math", FontData::from_static(MATH)),
			("Noto Sans Symbols 2", FontData::from_static(SYMBOLS)),
		]) {
		add_fallback(&mut definitions, name, data);
	}
	definitions
}

#[cfg(test)]
mod tests {
	use super::*;
	use egui::FontId;
	use skrifa::MetadataProvider;

	#[test]
	fn cjk_scan_reuses_immutable_jobs_without_retaining_their_text() {
		let mut scan = CjkScan::default();
		let mut job = Arc::new(egui::text::LayoutJob {
			text: "Latin — čeština العربية".into(),
			..Default::default()
		});
		assert!(!scan.text(&job));
		assert!(!scan.text(&job));
		assert_eq!(scan.checked.len(), 1);
		assert_eq!(Arc::strong_count(&job), 1);
		Arc::make_mut(&mut job).text = "日本語 中文 한국어".into();
		assert!(
			scan.text(&job),
			"editing an already checked job must detect CJK"
		);
		assert_eq!(
			scan.checked
				.iter()
				.filter(|entry| entry.upgrade().is_none())
				.count(),
			1
		);
		for index in 0..CHECKED_JOBS * 2 {
			let job = Arc::new(egui::text::LayoutJob {
				text: format!("Synthetic {index}"),
				..Default::default()
			});
			assert!(!scan.text(&job));
			assert!(scan.checked.len() <= CHECKED_JOBS);
			assert!(
				scan.checked.capacity()
					* (size_of::<egui::text::LayoutJob>() + 3 * size_of::<usize>())
					<= CHECKED_BYTES
			);
		}
		assert!(scan.checked.iter().all(|entry| entry.upgrade().is_none()));
		assert!(
			scan.text(&job),
			"cache rollover must not suppress new CJK text"
		);

		// Every unique (family, char) in a checked job must be collected, and jobs past the
		// per-pass cap must stay unchecked so the next pass inspects them rather than dropping
		// uncovered glyphs past an arbitrary prefix.
		let mut scan = CjkScan::default();
		let mixed = Arc::new(egui::text::LayoutJob {
			text: "一二三四五六七八九十한국어𠀀".into(),
			..Default::default()
		});
		scan.scan_job(&mixed);
		assert!(!scan.truncated);
		assert!(scan.chars.contains(&(FontFamily::Proportional, '한')));
		assert!(scan.chars.contains(&(FontFamily::Proportional, '𠀀')));

		let mut multi_section = egui::text::LayoutJob::default();
		multi_section.append(
			"日本語 ",
			0.0,
			egui::TextFormat::simple(FontId::proportional(14.0), egui::Color32::WHITE),
		);
		multi_section.append(
			"日本語",
			0.0,
			egui::TextFormat::simple(FontId::monospace(14.0), egui::Color32::WHITE),
		);
		let mut scan = CjkScan::default();
		scan.scan_job(&Arc::new(multi_section));
		assert!(scan.chars.contains(&(FontFamily::Proportional, '日')));
		assert!(scan.chars.contains(&(FontFamily::Monospace, '日')));

		let mut scan = CjkScan::default();
		let saturating = Arc::new(egui::text::LayoutJob {
			text: (0x4e00..0x4e00 + MAX_PASS_CJK_CHARS as u32)
				.filter_map(char::from_u32)
				.collect(),
			..Default::default()
		});
		let deferred = Arc::new(egui::text::LayoutJob {
			text: "Deferred 한글".into(),
			..Default::default()
		});
		scan.begin_pass();
		scan.scan_job(&saturating);
		assert_eq!(scan.chars.len(), MAX_PASS_CJK_CHARS);
		assert!(
			!scan.truncated,
			"hitting the cap on the final job must not mark the pass truncated"
		);
		scan.scan_job(&deferred);
		assert!(
			scan.truncated,
			"stopping before an unchecked job must mark the pass truncated"
		);
		assert_eq!(scan.finish_pass((0, 0, 0)), PassOutcome::Continue);
		assert_eq!(
			scan.checked.len(),
			1,
			"uninspected job past per-pass cap must remain unchecked for the next pass"
		);
		scan.begin_pass();
		scan.scan_job(&saturating);
		scan.scan_job(&deferred);
		assert!(!scan.truncated);
		assert_eq!(scan.finish_pass((0, 0, 0)), PassOutcome::Complete);
		assert!(scan.chars.contains(&(FontFamily::Proportional, '한')));

		// Continuation progress must survive `checked` cache eviction when more than
		// CHECKED_JOBS live jobs appear in one frame.
		let mut scan = CjkScan::default();
		let live_prefix: Vec<_> = (0..CHECKED_JOBS)
			.map(|index| {
				Arc::new(egui::text::LayoutJob {
					text: format!("Live {index}"),
					..Default::default()
				})
			})
			.collect();
		let run = |scan: &mut CjkScan| {
			scan.begin_pass();
			for job in &live_prefix {
				scan.scan_job(job);
			}
			scan.scan_job(&saturating);
			scan.scan_job(&deferred);
			scan.finish_pass((0, 0, 0))
		};
		assert_eq!(run(&mut scan), PassOutcome::Continue);
		assert_eq!(scan.resume_from, CHECKED_JOBS + 1);
		assert_eq!(
			scan.checked.len(),
			1,
			"filling CHECKED_JOBS live entries must evict earlier entries on rollover"
		);
		assert_eq!(run(&mut scan), PassOutcome::Complete);
		assert!(
			scan.chars.contains(&(FontFamily::Proportional, '한')),
			"deferred job past cache eviction must be reached on the continuation pass"
		);
		assert!(scan.completed_frame.is_some());
	}

	#[test]
	fn cjk_fallback_checks_section_families_and_repaints_only_when_truncated() {
		use std::time::Duration;

		let ctx = Context::default();
		install(&ctx);
		let custom_cjk = CustomFont {
			name: "Synthetic CJK".into(),
			data: FontData::from_owned(cjk()),
		};
		apply_custom(&ctx, Some(&custom_cjk));

		let pass = |ui_fn: &mut dyn FnMut(&mut egui::Ui)| -> Duration {
			let output = ctx.run_ui(Default::default(), ui_fn);
			let delay = output
				.viewport_output
				.get(&egui::ViewportId::ROOT)
				.map_or(Duration::MAX, |viewport| viewport.repaint_delay);
			output.drop_without_applying_deltas();
			delay
		};

		// Settle initial font-install repaints before measuring scan-triggered repaints.
		for _ in 0..4 {
			if pass(&mut |ui| {
				ui.label("ready");
			}) != Duration::ZERO
			{
				break;
			}
		}
		ctx.fonts(|fonts| {
			let monospace = &fonts.definitions().families[&FontFamily::Monospace];
			assert!(
				!CUSTOM
					.iter()
					.any(|name| monospace.iter().any(|entry| entry == name)),
				"apply_custom must not add the custom interface font to Monospace"
			);
		});

		// A single job hitting MAX_PASS_CJK_CHARS with no skipped jobs must not request a repaint.
		let exact_cap: String = (0x4e00..0x4e00 + MAX_PASS_CJK_CHARS as u32)
			.filter_map(char::from_u32)
			.collect();
		let delay = pass(&mut |ui| {
			ui.label(&exact_cap);
		});
		assert_ne!(
			delay,
			Duration::ZERO,
			"pass that inspected every job must not request an extra repaint"
		);
		assert!(!ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")));

		// Stopping early before another unchecked job must request one additional repaint,
		// then stop requesting repaints once the follow-up pass inspects the deferred job.
		let second_cap: String = (0x4e40..0x4e40 + MAX_PASS_CJK_CHARS as u32)
			.filter_map(char::from_u32)
			.collect();
		let delay = pass(&mut |ui| {
			ui.label(&second_cap);
			ui.label("日本語");
		});
		assert_eq!(
			delay,
			Duration::ZERO,
			"truncated pass without fallback install must request one continuation repaint"
		);
		assert!(!ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")));

		let mut prev_causes = Vec::new();
		let _ = pass(&mut |ui| {
			prev_causes = ui.ctx().repaint_causes();
			ui.label(&second_cap);
			ui.label("日本語");
		});
		assert!(
			!prev_causes.is_empty(),
			"truncated pass must record a repaint request"
		);
		assert!(!ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")));

		// egui schedules one automatic settling pass after any immediate repaint; the
		// follow-up pass itself must not record another repaint cause.
		let delay = pass(&mut |ui| {
			prev_causes = ui.ctx().repaint_causes();
			ui.label(&second_cap);
			ui.label("日本語");
		});
		assert!(
			prev_causes.is_empty(),
			"follow-up pass that finishes deferred jobs must not request another repaint"
		);
		assert_ne!(
			delay,
			Duration::ZERO,
			"repainting must stop after the deferred jobs are inspected"
		);
		assert!(!ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")));

		// A busy view with more than CHECKED_JOBS live labels must still reach deferred jobs
		// past cache eviction and stop repainting once every covered job has been inspected.
		let busy_prefix: Vec<String> = (0..CHECKED_JOBS).map(|i| format!("row-{i}")).collect();
		let third_cap: String = (0x4e80..0x4e80 + MAX_PASS_CJK_CHARS as u32)
			.filter_map(char::from_u32)
			.collect();
		let show_busy = |ui: &mut egui::Ui, monospace_tail: bool| {
			let painter = ui.painter();
			for row in &busy_prefix {
				painter.text(
					egui::Pos2::ZERO,
					egui::Align2::LEFT_TOP,
					row,
					FontId::proportional(14.0),
					egui::Color32::WHITE,
				);
			}
			painter.text(
				egui::Pos2::ZERO,
				egui::Align2::LEFT_TOP,
				&third_cap,
				FontId::proportional(14.0),
				egui::Color32::WHITE,
			);
			if monospace_tail {
				painter.text(
					egui::Pos2::ZERO,
					egui::Align2::LEFT_TOP,
					"日本語",
					FontId::monospace(14.0),
					egui::Color32::WHITE,
				);
			} else {
				painter.text(
					egui::Pos2::ZERO,
					egui::Align2::LEFT_TOP,
					"東京大阪",
					FontId::proportional(14.0),
					egui::Color32::WHITE,
				);
			}
		};
		let delay = pass(&mut |ui| show_busy(ui, false));
		assert_eq!(
			delay,
			Duration::ZERO,
			"truncated pass after checked cache rollover must request a continuation repaint"
		);
		assert!(!ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")));

		let _ = pass(&mut |ui| {
			prev_causes = ui.ctx().repaint_causes();
			show_busy(ui, false);
		});
		assert!(!prev_causes.is_empty());
		assert!(!ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")));

		let delay = pass(&mut |ui| {
			prev_causes = ui.ctx().repaint_causes();
			show_busy(ui, false);
		});
		assert!(
			prev_causes.is_empty(),
			"continuation pass past cache rollover must settle without oscillating repaints"
		);
		assert_ne!(
			delay,
			Duration::ZERO,
			"unchanged busy view must stop repainting after full sweep completes"
		);
		assert!(!ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")));

		// A Monospace code block past cache eviction must still be reached and install the
		// bundled fallback even when the custom interface font covers the same glyphs in
		// Proportional.
		let (tx, rx) = std::sync::mpsc::channel();
		ctx.set_request_repaint_callback(move |_| {
			let _ = tx.send(());
		});
		let delay = pass(&mut |ui| show_busy(ui, true));
		assert_eq!(delay, Duration::ZERO);
		let _ = pass(&mut |ui| show_busy(ui, true));
		let deadline = std::time::Instant::now() + Duration::from_secs(10);
		while !ctx.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK")) {
			let remaining = deadline
				.checked_duration_since(std::time::Instant::now())
				.expect("bundled CJK fallback worker must install Noto Sans CJK");
			rx.recv_timeout(remaining)
				.expect("bundled CJK fallback worker must request repaint");
			let _ = pass(&mut |ui| {
				ui.label(egui::RichText::new("日本語").monospace());
			});
		}
		assert!(ctx.fonts_mut(|fonts| fonts.has_glyph(&FontId::monospace(14.0), '日')));
		ctx.fonts(|fonts| {
			let monospace = &fonts.definitions().families[&FontFamily::Monospace];
			assert!(
				!CUSTOM
					.iter()
					.any(|name| monospace.iter().any(|entry| entry == name)),
				"Monospace must keep using fallback rather than the custom interface font"
			);
		});
	}

	#[test]
	fn name_fonts_parse_install_on_first_use_and_keep_fallbacks() {
		use skrifa::MetadataProvider;
		assert!(
			NAME_FONTS
				.iter()
				.map(|(.., data)| data.len())
				.sum::<usize>()
				<= 4 * 1024 * 1024
		);
		for (id, name, data) in NAME_FONTS {
			let face = skrifa::FontRef::new(data).unwrap_or_else(|_| panic!("{name}"));
			assert!(face.charmap().map('A').is_some(), "{name} lacks Latin");
			assert!(!matches!(id, 0 | 7 | 11 | 17));
		}
		let ctx = Context::default();
		install(&ctx);
		let mut families = Vec::new();
		for _ in 0..2 {
			ctx.run_ui(Default::default(), |ui| {
				families.push(name_family(ui.ctx(), 1));
				assert!(name_family(ui.ctx(), 11).is_none());
				assert!(!ui.fonts(|fonts| {
					fonts
						.definitions()
						.font_data
						.contains_key("Serein Name Kalam")
				}));
			})
			.drop_without_applying_deltas();
		}
		assert_eq!(families[0], None, "the first pass keeps the interface font");
		let family = families[1].clone().expect("installed on the next pass");
		ctx.run_ui(Default::default(), |ui| {
			let list = &ui.fonts(|fonts| fonts.definitions().families[&family].clone());
			assert_eq!(list[0], "Serein Name Bangers");
			assert!(list.iter().any(|face| face == "Inter Medium"));
			assert!(list.iter().any(|face| face == "Noto Sans Arabic"));
			ui.label(egui::RichText::new("Synthetic name").family(family.clone()));
		})
		.drop_without_applying_deltas();
	}

	#[test]
	fn name_families_follow_a_custom_font_that_is_set_then_removed() {
		let ctx = Context::default();
		install(&ctx);
		for _ in 0..2 {
			ctx.run_ui(Default::default(), |ui| {
				name_family(ui.ctx(), 1);
			})
			.drop_without_applying_deltas();
		}
		let custom = CustomFont {
			name: "Synthetic".into(),
			data: FontData::from_static(INTER),
		};
		let family = FontFamily::Name("Serein Name Bangers".into());
		apply_custom(&ctx, Some(&custom));
		let list = ctx.fonts(|fonts| fonts.definitions().families[&family].clone());
		assert_eq!(&list[..2], ["Serein Name Bangers", CUSTOM[1]]);
		apply_custom(&ctx, None);
		ctx.run_ui(Default::default(), |ui| {
			let definitions = ui.fonts(|fonts| fonts.definitions().clone());
			assert!(
				definitions.families[&family]
					.iter()
					.all(|face| definitions.font_data.contains_key(face))
			);
			// Resolving the family panicked while it still named the removed custom font.
			ui.label(egui::RichText::new("Synthetic name").family(family.clone()));
		})
		.drop_without_applying_deltas();
	}

	#[test]
	fn startup_does_not_decode_cjk_but_on_demand_definitions_do() {
		let before = CJK_DECODES.get();
		install(&Context::default());
		assert_eq!(CJK_DECODES.get(), before);
		let base = definitions(false);
		assert!(!base.font_data.contains_key("Noto Sans CJK"));
		let full = definitions(true);
		assert_eq!(CJK_DECODES.get(), before + 1);
		assert_eq!(full.font_data.len(), base.font_data.len() + 1);
		assert_eq!(full.font_data["Noto Sans CJK"].bytes().len(), CJK_BYTES);
		for (family, names) in &base.families {
			assert!(!names.iter().any(|name| name == "Noto Sans CJK"));
			let without_cjk: Vec<_> = full.families[family]
				.iter()
				.filter(|name| *name != "Noto Sans CJK")
				.cloned()
				.collect();
			assert_eq!(*names, without_cjk);
		}
	}

	#[test]
	fn bundled_fallbacks_cover_multilingual_text_with_a_fixed_asset_budget() {
		// The CJK face counts at its embedded (compressed) size.
		assert!(
			CJK_ZSTD.len()
				+ ARABIC.len()
				+ MATH.len() + SYMBOLS.len()
				+ INTER.len()
				+ INTER_MEDIUM.len()
				+ INTER_SEMIBOLD.len()
				<= 16 * 1024 * 1024
		);
		let collection = cjk();
		assert_eq!(collection.len(), CJK_BYTES);
		for (language, index, glyph) in [
			(crate::i18n::Language::Japanese, 0, 45132),
			(crate::i18n::Language::ChineseSimplified, 2, 45133),
			(crate::i18n::Language::ChineseTraditional, 3, 45134),
		] {
			assert_eq!(cjk_index(language), index);
			let face = skrifa::FontRef::from_index(&collection, index).unwrap();
			assert_eq!(face.charmap().map('骨'), Some(skrifa::GlyphId::new(glyph)));
		}
		let korean_index = cjk_index(crate::i18n::Language::Korean);
		assert_eq!(korean_index, 1);
		let korean_face = skrifa::FontRef::from_index(&collection, korean_index).unwrap();
		for character in "한국어".chars() {
			assert!(
				korean_face
					.charmap()
					.map(character)
					.is_some_and(|id| id != skrifa::GlyphId::NOTDEF)
			);
		}
		let definitions = definitions(true);
		for family in [FontFamily::Proportional, FontFamily::Monospace] {
			let faces: Vec<_> = definitions.families[&family]
				.iter()
				.map(|name| {
					let data = &definitions.font_data[name];
					skrifa::FontRef::from_index(data.bytes(), data.index)
						.expect("valid bundled font")
				})
				.collect();
			for c in "Hello, 日本語かなカナ 中文汉字繁體 한국어 العربية مَرْحَبًا 𝖘𝖓𝖎𝖎𝖝. é e\u{301} ✦"
				.chars()
			{
				assert!(
					faces.iter().any(|face| {
						face.charmap()
							.map(c)
							.is_some_and(|id| id != skrifa::GlyphId::NOTDEF)
					}),
					"missing glyph: {c} ({c:?})"
				);
			}
		}
		let ctx = Context::default();
		ctx.set_fonts(definitions.clone());
		let output = ctx.run_ui(Default::default(), |ui| {
			ui.fonts_mut(|fonts| {
				for family in [FontFamily::Proportional, FontFamily::Monospace] {
					let font = FontId::new(14.0, family);
					// egui 0.36.2 has_glyph incorrectly returns false for all
					// primary-face glyphs. Check every scalar through its font
					// parser above, then check the actual fallback path here.
					for c in "日本語かなカナ中文汉字繁體한국어العربية𝖘𝖓𝖎𝖎𝖝✦".chars()
					{
						assert!(fonts.has_glyph(&font, c), "missing glyph: {c} ({c:?})");
					}
				}
			});
		});
		output.drop_without_applying_deltas();
	}
	#[test]
	fn font_list_regrows_after_a_search_without_matches() {
		let ctx = Context::default();
		// 13 families; "Family 1" matches only Family 10, 11 and 12.
		let mut settings = Settings {
			families: Some((0..13).map(|i| format!("Family {i:02}")).collect()),
			..Default::default()
		};
		// The picker button spans the top of the screen, below a 4 px gap.
		let button = egui::pos2(200.0, 24.0);
		// Returns the list's visible and content height.
		let pass = |settings: &mut Settings, events: Vec<egui::Event>| {
			let raw = egui::RawInput {
				screen_rect: Some(egui::Rect::from_min_size(
					egui::Pos2::ZERO,
					egui::vec2(800.0, 900.0),
				)),
				events,
				..Default::default()
			};
			let mut output = ctx.run_ui(raw, |ui| settings.family_list(ui));
			output.textures_delta.clear();
			LIST.take()
		};
		pass(&mut settings, vec![]);
		let click = |pressed| egui::Event::PointerButton {
			pos: button,
			button: egui::PointerButton::Primary,
			pressed,
			modifiers: Default::default(),
		};
		pass(
			&mut settings,
			vec![egui::Event::PointerMoved(button), click(true)],
		);
		pass(&mut settings, vec![click(false)]);
		let (open, open_content) = pass(&mut settings, vec![]).expect("list shown");
		assert!(
			open_content > open,
			"all 13 families should not fit: the list scrolls"
		);

		let mut search_after_no_match = |query: &str| {
			settings.query = "no such font".into();
			pass(&mut settings, vec![]);
			assert_eq!(pass(&mut settings, vec![]), None, "no list without matches");
			settings.query = query.into();
			pass(&mut settings, vec![]).expect("list shown")
		};
		assert_eq!(
			search_after_no_match("Family").0,
			open,
			"picker should regrow to its opened height after a no-match search"
		);
		let (visible, content) = search_after_no_match("Family 1");
		assert!(
			visible < open,
			"three matches should need less room than the full list"
		);
		assert!(
			content <= visible,
			"three matches should fit without scrolling"
		);
	}
}
