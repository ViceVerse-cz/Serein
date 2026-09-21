//! Bundled OFL fallback faces. Eframe separately provides native system-font fallback.
use egui::{Context, FontData, FontDefinitions, FontFamily};
use std::sync::{Arc, Mutex, Weak};

/// Noto Sans CJK JP is a quarter of the executable uncompressed (16.4 MB). It ships as a
/// `zstd -19` archive (12.0 MB) and is inflated in memory the first time CJK text is
/// drawn; Latin-only sessions never pay for the decode.
const CJK_ZSTD: &[u8] = include_bytes!("../../../assets/fonts/NotoSansCJKjp-Regular.otf.zst");
const CJK_BYTES: usize = 16_467_736;
const ARABIC: &[u8] = include_bytes!("../../../assets/fonts/NotoSansArabic.ttf");
const MATH: &[u8] = include_bytes!("../../../assets/fonts/NotoSansMath-Regular.otf");
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
	ctx.set_fonts(definitions(false));
	let installed = std::sync::atomic::AtomicBool::new(false);
	let scan = Mutex::new(CjkScan::default());
	ctx.on_end_pass(
		"CJK fallback",
		std::sync::Arc::new(move |ui| {
			// Also true while the decode thread runs, so the scan stops after the first hit.
			if installed.load(std::sync::atomic::Ordering::Relaxed) {
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
				// Inflating 16 MB and reparsing the font set takes tens of milliseconds; keep
				// it off the UI thread and accept one pass of fallback glyphs.
				let worker = ctx.clone();
				let spawned =
					std::thread::Builder::new()
						.name("cjk-font".into())
						.spawn(move || {
							worker.set_fonts(definitions(true));
							worker.request_repaint();
						});
				if spawned.is_err() {
					ctx.set_fonts(definitions(true));
					ctx.request_repaint();
				}
			}
		}),
	);
	crate::design::weights_installed(ctx);
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
		.then(|| ("Noto Sans CJK JP", FontData::from_owned(cjk())))
		.into_iter()
		.chain([
			("Noto Sans Arabic", FontData::from_static(ARABIC)),
			("Noto Sans Math", FontData::from_static(MATH)),
		]) {
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
	// ponytail: one Japanese CJK face bounds asset cost; add regional Han faces
	// when locale-specific glyph forms are implemented and measured.
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
					fonts
						.definitions()
						.font_data
						.contains_key("Noto Sans CJK JP")
				}));
			})
			.drop_without_applying_deltas();
		}
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
		loop {
			let mut installed = false;
			ctx.run_ui(Default::default(), |ui| {
				ui.label("日本語");
				installed = ui.fonts(|fonts| {
					fonts
						.definitions()
						.font_data
						.contains_key("Noto Sans CJK JP")
				});
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
		assert!(!base.font_data.contains_key("Noto Sans CJK JP"));
		let full = definitions(true);
		assert_eq!(CJK_DECODES.get(), before + 1);
		assert_eq!(full.font_data.len(), base.font_data.len() + 1);
		assert_eq!(full.font_data["Noto Sans CJK JP"].bytes().len(), CJK_BYTES);
		for (family, names) in &base.families {
			assert!(!names.iter().any(|name| name == "Noto Sans CJK JP"));
			let without_cjk: Vec<_> = full.families[family]
				.iter()
				.filter(|name| *name != "Noto Sans CJK JP")
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
				+ MATH.len() + INTER.len()
				+ INTER_MEDIUM.len()
				+ INTER_SEMIBOLD.len()
				<= 16 * 1024 * 1024
		);
		assert_eq!(cjk().len(), CJK_BYTES);
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
			for c in
				"Hello, 日本語かなカナ 中文汉字繁體 한국어 العربية مَرْحَبًا 𝖘𝖓𝖎𝖎𝖝. é e\u{301}".chars()
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
					for c in "日本語かなカナ中文汉字繁體한국어العربية𝖘𝖓𝖎𝖎𝖝".chars()
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
