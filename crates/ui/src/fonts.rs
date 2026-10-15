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
		let colors = design::palette(ui);
		let Some(families) = &self.families else {
			ui.horizontal(|ui| {
				ui.spinner();
				ui.label(
					egui::RichText::new(crate::i18n::translate("fonts-show-loading"))
						.color(colors.muted),
				);
			});
			return;
		};
		let default = crate::i18n::translate("fonts-show-inter-default");
		let current = self.name.clone().unwrap_or_else(|| default.clone());
		ui.add_space(4.0);
		let (rect, button) =
			ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::click());
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
					.auto_shrink([false, true]);
				if std::mem::take(&mut self.focus) {
					search.request_focus();
					let index = matches
						.iter()
						.position(|name| name.map(String::as_str) == self.name.as_deref())
						.unwrap_or(0);
					area = area.vertical_scroll_offset((index as f32 - 3.0).max(0.0) * ROW);
				}
				area.show_rows(ui, ROW, matches.len(), |ui, rows| {
					ui.spacing_mut().item_spacing.y = 0.0;
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

/// Clear account widget memory while retaining the application-owned font registry.
pub fn reset_session_memory(ctx: &Context) {
	let shared = ctx.data(|data| {
		data.get_temp::<Arc<Mutex<FontDefinitions>>>(egui::Id::unique(DEFINITIONS_KEY))
	});
	ctx.memory_mut(|memory| *memory = egui::Memory::default());
	if let Some(shared) = shared {
		ctx.data_mut(|data| data.insert_temp(egui::Id::unique(DEFINITIONS_KEY), shared));
		crate::design::weights_installed(ctx);
	}
}

/// Return false when fonts have not been installed, so callers retain the active choice.
pub fn apply_custom(ctx: &Context, font: Option<&CustomFont>) -> bool {
	let shared = ctx.data(|data| {
		data.get_temp::<Arc<Mutex<FontDefinitions>>>(egui::Id::unique(DEFINITIONS_KEY))
	});
	let Some(shared) = shared else { return false };
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
	ctx.set_fonts(definitions.clone());
	ctx.request_repaint();
	true
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

const CHECKED_JOBS: usize = 512;
const CHECKED_BYTES: usize = 128 * 1024;
// Weak references retain only the fixed-size Arc allocation, never text or meshes.
const _: () = assert!(
	CHECKED_JOBS * (size_of::<egui::text::LayoutJob>() + 3 * size_of::<usize>()) <= CHECKED_BYTES
);

#[derive(Default)]
struct CjkScan {
	checked: Vec<Weak<egui::text::LayoutJob>>,
}

impl CjkScan {
	fn text(&mut self, job: &Arc<egui::text::LayoutJob>) -> bool {
		let index = match self
			.checked
			.binary_search_by_key(&(Arc::as_ptr(job) as usize), |entry| {
				entry.as_ptr() as usize
			}) {
			Ok(_) => return false,
			Err(index) => index,
		};
		if !job.text.is_ascii() && job.text.chars().any(|c| matches!(c as u32, 0x1100..=0x11ff | 0x2e80..=0xa4cf | 0xa960..=0xa97f | 0xac00..=0xd7af | 0xd7b0..=0xd7ff | 0xf900..=0xfaff | 0xfe30..=0xffef | 0x20000..=0x323af)) {
			return true;
		}
		// Keep allocation identities alive so allocator address reuse cannot hide new text.
		// Arc::make_mut also dissociates these weak references before editing a job.
		if self.checked.capacity() == 0 {
			self.checked.reserve_exact(CHECKED_JOBS);
		}
		// ponytail: clear the fixed cache at capacity; unusually busy views rescan text.
		let index = if self.checked.len() == CHECKED_JOBS {
			self.checked.clear();
			0
		} else {
			index
		};
		self.checked.insert(index, Arc::downgrade(job));
		false
	}

	fn shape(&mut self, shape: &egui::Shape) -> bool {
		match shape {
			egui::Shape::Text(text) => self.text(&text.galley.job),
			egui::Shape::Vec(shapes) => shapes.iter().any(|shape| self.shape(shape)),
			_ => false,
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
			let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
			let needed = ctx.graphics(|graphics| {
				layers.iter().any(|layer| {
					graphics.get(*layer).is_some_and(|list| {
						list.all_entries().any(|entry| scan.shape(&entry.shape))
					})
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
	fn session_reset_preserves_device_font_state_and_allows_replacement_and_reset() {
		let ctx = Context::default();
		install(&ctx);
		let original = CustomFont::new("Synthetic original".into(), INTER.to_vec()).unwrap();
		let replacement =
			CustomFont::new("Synthetic replacement".into(), INTER_SEMIBOLD.to_vec()).unwrap();
		let mut view = crate::MessagingUi::default();
		assert!(apply_custom(&ctx, Some(&original)));
		view.custom_font.name = Some(original.name.clone());
		view.custom_font.busy = true;
		view.custom_font.status = "Saving font…";
		view.custom_font.families = Some(vec![original.name.clone(), replacement.name.clone()]);
		view.settings.open = true;
		let account_widget = egui::Id::unique("synthetic-account-widget");
		ctx.data_mut(|data| data.insert_temp(account_widget, true));
		ctx.run_ui(Default::default(), |ui| {
			ui.label("Synthetic font baseline");
			assert_eq!(
				ui.fonts(|fonts| fonts.definitions().font_data[CUSTOM[0]].bytes().to_vec()),
				original.bytes()
			);
		})
		.drop_without_applying_deltas();
		view.clear();
		reset_session_memory(&ctx);
		assert_eq!(
			view.custom_font.name.as_deref(),
			Some(original.name.as_str())
		);
		assert!(view.custom_font.busy);
		assert_eq!(view.custom_font.status, "Saving font…");
		assert_eq!(view.custom_font.families.as_ref().unwrap().len(), 2);
		assert!(!view.settings.open);
		assert!(
			ctx.data(|data| data.get_temp::<bool>(account_widget))
				.is_none()
		);
		assert!(apply_custom(&ctx, Some(&replacement)));
		view.custom_font.name = Some(replacement.name.clone());
		ctx.run_ui(Default::default(), |ui| {
			ui.label("Synthetic font replacement");
			assert_eq!(
				ui.fonts(|fonts| fonts.definitions().font_data[CUSTOM[0]].bytes().to_vec()),
				replacement.bytes()
			);
		})
		.drop_without_applying_deltas();
		assert_eq!(
			view.custom_font.name.as_deref(),
			Some(replacement.name.as_str())
		);
		assert!(apply_custom(&ctx, None));
		view.custom_font.name = None;
		ctx.run_ui(Default::default(), |ui| {
			ui.label("Synthetic font reset");
			ui.fonts(|fonts| {
				assert_eq!(
					fonts.definitions().families[&FontFamily::Proportional][0],
					"Inter"
				);
				assert!(
					CUSTOM
						.iter()
						.all(|name| !fonts.definitions().font_data.contains_key(*name))
				);
			});
		})
		.drop_without_applying_deltas();
		assert!(view.custom_font.name.is_none());
		assert!(!apply_custom(&Context::default(), Some(&original)));
	}

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
		assert!(scan.checked[0].upgrade().is_none());
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
	}

	#[test]
	fn cjk_arriving_after_settled_latin_frames_installs_the_fallback() {
		let ctx = Context::default();
		install(&ctx);
		for _ in 0..3 {
			ctx.run_ui(Default::default(), |ui| {
				ui.label("Synthetic Latin text");
				assert!(!ui.fonts(|fonts| {
					fonts.definitions().font_data.contains_key("Noto Sans CJK")
				}));
			})
			.drop_without_applying_deltas();
		}
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
		loop {
			let mut installed = false;
			ctx.run_ui(Default::default(), |ui| {
				ui.label("日本語");
				installed =
					ui.fonts(|fonts| fonts.definitions().font_data.contains_key("Noto Sans CJK"));
			})
			.drop_without_applying_deltas();
			if installed {
				break;
			}
			assert!(
				std::time::Instant::now() < deadline,
				"CJK worker did not install its fallback"
			);
			std::thread::sleep(std::time::Duration::from_millis(5));
		}
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
	fn latin_faces_are_rasterized_without_truetype_hinting() {
		for data in [INTER, INTER_MEDIUM, INTER_SEMIBOLD] {
			let font = skrifa::FontRef::new(data).expect("valid bundled font");
			for table in ["glyf", "fpgm", "prep"] {
				let tag = skrifa::Tag::new(table.as_bytes().try_into().unwrap());
				assert!(
					font.table_data(tag).is_some(),
					"Inter must be the TrueType build, missing `{table}`",
				);
			}
		}
		let ctx = Context::default();
		install(&ctx);
		crate::design::apply(&ctx);
		for theme in [egui::Theme::Dark, egui::Theme::Light] {
			let options = &ctx.style_of(theme).visuals.text_options;
			assert!(options.subpixel_binning);
			assert!(!options.font_hinting);
		}
		assert_eq!(
			ctx.style_of(egui::Theme::Dark)
				.visuals
				.text_options
				.color_transfer_function,
			egui::epaint::FontColorTransferFunction::Gamma(0.5)
		);
		let tweaks = definitions(false)
			.font_data
			.iter()
			.filter(|(name, _)| name.starts_with("Inter"))
			.map(|(_, data)| (data.tweak.hinting, data.tweak.subpixel_binning))
			.collect::<Vec<_>>();
		assert_eq!(tweaks, vec![(Some(false), Some(true)); 3]);
	}
}
