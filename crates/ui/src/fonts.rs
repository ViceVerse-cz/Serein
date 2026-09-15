//! Bundled OFL fallback faces. No runtime download, system-font scan, or disk I/O.
use egui::{Context, FontData, FontDefinitions, FontFamily};

const CJK: &[u8] = include_bytes!("../../../assets/fonts/NotoSansCJKjp-Regular.otf");
const ARABIC: &[u8] = include_bytes!("../../../assets/fonts/NotoSansArabic.ttf");
const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf");

/// Install once during application creation, before the first UI pass.
pub fn install(ctx: &Context) {
	let mut latin = definitions();
	latin.font_data.remove("Noto Sans CJK JP");
	for family in latin.families.values_mut() {
		family.retain(|name| name != "Noto Sans CJK JP");
	}
	ctx.set_fonts(latin);
	let installed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
	ctx.on_end_pass("CJK fallback", std::sync::Arc::new(move |ui| {
		if installed.load(std::sync::atomic::Ordering::Relaxed) { return; }
		fn needs_cjk(shape: &egui::Shape) -> bool {
			match shape {
				egui::Shape::Text(text) => text.galley.job.text.chars().any(|c| matches!(c as u32, 0x1100..=0x11ff | 0x2e80..=0xa4cf | 0xa960..=0xa97f | 0xd7b0..=0xd7ff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0xfe30..=0xffef | 0x20000..=0x323af)),
				egui::Shape::Vec(shapes) => shapes.iter().any(needs_cjk),
				_ => false,
			}
		}
		let ctx = ui.ctx();
		let layers: Vec<_> = ctx.memory(|memory| memory.layer_ids().collect());
		let needed = ctx.graphics(|graphics| layers.iter().any(|layer| graphics.get(*layer).is_some_and(|list| list.all_entries().any(|entry| needs_cjk(&entry.shape)))));
		if needed {
			installed.store(true, std::sync::atomic::Ordering::Relaxed);
			ctx.set_fonts(definitions());
			ctx.request_discard("CJK fallback loaded");
			ctx.request_repaint();
		}
	}));
	crate::design::weights_installed(ctx);
}

/// The bundled Inter faces are the "hinted for Windows" TrueType builds, so the
/// TrueType interpreter — not the auto-hinter — grid-fits their stems. Symmetric
/// rendering keeps a glyph identical across sub-pixel positions, but it lets those
/// instructions widen stems, which reads as blur under egui's analytic rasterizer.
/// Sub-pixel binning is off (see `design::apply`), so every glyph is rasterized at a
/// single position anyway and turning this off costs no atlas space or cache churn.
fn hinted(data: &'static [u8]) -> FontData {
	let mut font = FontData::from_static(data);
	font.tweak.hinting_target =
		egui::epaint::text::HintingTarget::Smooth(egui::epaint::text::SmoothHinting {
			symmetric_rendering: false,
			..Default::default()
		});
	font
}

fn definitions() -> FontDefinitions {
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
			.insert(name.into(), hinted(data).into());
		let list = definitions.families.entry(family).or_default();
		list.retain(|existing| !defaults.contains(existing));
		list.insert(0, name.into());
		list.extend(defaults.iter().cloned());
	}
	for (name, data) in [("Noto Sans CJK JP", CJK), ("Noto Sans Arabic", ARABIC)] {
		definitions
			.font_data
			.insert(name.into(), FontData::from_static(data).into());
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
	fn bundled_fallbacks_cover_multilingual_text_with_a_fixed_asset_budget() {
		// The Inter faces are the hinted TrueType builds: their instructions cost
		// ~1.3 MB more than the CFF originals, which is the price of sharp text at 1x.
		assert!(
			CJK.len() + ARABIC.len() + INTER.len() + INTER_MEDIUM.len() + INTER_SEMIBOLD.len()
				<= 20 * 1024 * 1024
		);
		let definitions = definitions();
		for family in [FontFamily::Proportional, FontFamily::Monospace] {
			let faces: Vec<_> = definitions.families[&family]
				.iter()
				.map(|name| {
					let data = &definitions.font_data[name];
					skrifa::FontRef::from_index(data.bytes(), data.index)
						.expect("valid bundled font")
				})
				.collect();
			for c in "Hello, 日本語かなカナ 中文汉字繁體 한국어 العربية مَرْحَبًا é e\u{301}".chars()
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
					for c in "日本語かなカナ中文汉字繁體한국어العربية".chars()
					{
						assert!(fonts.has_glyph(&font, c), "missing glyph: {c} ({c:?})");
					}
				}
			});
		});
		output.drop_without_applying_deltas();
	}

	/// Issue #200: text read as blurry at 1x on Windows. Sharpness needs the hinted
	/// TrueType Inter builds (CFF outlines are effectively unhinted by skrifa), glyphs
	/// positioned on whole pixels, and hints that are free to grid-fit stems.
	#[test]
	fn latin_faces_are_rasterized_for_sharp_text_at_low_scale() {
		for data in [INTER, INTER_MEDIUM, INTER_SEMIBOLD] {
			let font = skrifa::FontRef::new(data).expect("valid bundled font");
			// `glyf` means TrueType outlines rather than CFF; `fpgm`/`prep` are the
			// instructions skrifa's TrueType interpreter needs to grid-fit stems.
			for table in ["glyf", "fpgm", "prep"] {
				let tag = skrifa::Tag::new(table.as_bytes().try_into().unwrap());
				assert!(
					font.table_data(tag).is_some(),
					"Inter must be the hinted TrueType build, missing `{table}`",
				);
			}
		}
		let ctx = Context::default();
		install(&ctx);
		crate::design::apply(&ctx);
		for theme in [egui::Theme::Dark, egui::Theme::Light] {
			assert!(!ctx.style_of(theme).visuals.text_options.subpixel_binning);
			assert!(ctx.style_of(theme).visuals.text_options.font_hinting);
		}
		let symmetric = definitions()
			.font_data
			.iter()
			.filter(|(name, _)| name.starts_with("Inter"))
			.map(|(_, data)| match data.tweak.hinting_target {
				egui::epaint::text::HintingTarget::Smooth(smooth) => smooth.symmetric_rendering,
				egui::epaint::text::HintingTarget::Mono => true,
			})
			.collect::<Vec<_>>();
		assert_eq!(symmetric, vec![false; 3]);
	}
}
