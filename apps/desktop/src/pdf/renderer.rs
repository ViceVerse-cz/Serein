//! Runs only inside the resource-limited helper, never in the desktop/UI process.
use super::protocol::Raster;
use hayro::{
	hayro_interpret::{InterpreterSettings, util::TransformExt},
	hayro_syntax::{LoadPdfError, Pdf},
};
use ui::{MAX_PDF_BYTES, MAX_PDF_EDGE, MAX_PDF_PAGES, MAX_PDF_PIXELS, PdfError};
pub fn dimensions(width: f32, height: f32) -> Result<(u16, u16, f64), PdfError> {
	let (w, h) = (width as f64, height as f64);
	if !w.is_finite() || !h.is_finite() || w <= 0. || h <= 0. {
		return Err(PdfError::Invalid);
	}
	let scale = (MAX_PDF_EDGE as f64 / w)
		.min(MAX_PDF_EDGE as f64 / h)
		.min((MAX_PDF_PIXELS as f64 / (w * h)).sqrt())
		.min(2.);
	let (pw, ph) = (
		(w * scale).floor().max(1.) as usize,
		(h * scale).floor().max(1.) as usize,
	);
	if !ui::valid_pdf_dimensions(pw, ph) || !scale.is_finite() || scale <= 0. {
		return Err(PdfError::Limit);
	}
	Ok((pw as u16, ph as u16, scale))
}
pub fn render(data: Vec<u8>, index: u32) -> Result<Raster, PdfError> {
	if data.is_empty() || data.len() > MAX_PDF_BYTES {
		return Err(PdfError::Limit);
	}
	if !data.starts_with(b"%PDF-") {
		return Err(PdfError::Invalid);
	}
	let pdf = Pdf::new(data).map_err(|error| match error {
		LoadPdfError::Decryption(_) => PdfError::Password,
		LoadPdfError::Invalid => PdfError::Invalid,
	})?;
	let pages = pdf.pages();
	if pages.is_empty() {
		return Err(PdfError::Invalid);
	}
	if pages.len() > MAX_PDF_PAGES as usize {
		return Err(PdfError::Limit);
	}
	let page = pages.get(index as usize).ok_or(PdfError::Invalid)?;
	for rect in [
		page.media_box(),
		page.crop_box(),
		page.intersected_crop_box(),
	] {
		if ![rect.x0, rect.y0, rect.x1, rect.y1]
			.iter()
			.all(|v| v.is_finite())
			|| rect.width() <= 0.
			|| rect.height() <= 0.
		{
			return Err(PdfError::Invalid);
		}
	}
	let (width, height) = page.render_dimensions();
	let (width, height, scale) = dimensions(width, height)?;
	let mut ctx = hayro::vello_cpu::RenderContext::new(width, height);
	hayro::render_into(
		page,
		&hayro::RenderCache::new(),
		&InterpreterSettings::default(),
		&hayro::RenderSettings::default(),
		&mut ctx,
		hayro::kurbo::Affine::scale(scale) * page.initial_transform(true).to_kurbo(),
	);
	ctx.flush();
	let mut pixmap = hayro::vello_cpu::Pixmap::new(width, height);
	ctx.render_with(
		&mut pixmap,
		&mut hayro::vello_cpu::Resources::default(),
		hayro::vello_cpu::RasterizerSettings {
			target_init: hayro::vello_cpu::TargetInit::Clear(
				hayro::vello_cpu::color::palette::css::WHITE,
			),
			..Default::default()
		},
	);
	Ok(Raster {
		pages: pages.len() as u32,
		page: index,
		width: width as usize,
		height: height as usize,
		rgba: pixmap.data_as_u8_slice().to_vec(),
	})
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn dimensions_are_finite_and_bounded() {
		for (w, h) in [
			(600., 780.),
			(1e30, 1e30),
			(0.01, 100000.),
			(100000., 0.01),
			(1536., 1536.),
		] {
			let (x, y, s) = dimensions(w, h).unwrap();
			assert!(ui::valid_pdf_dimensions(x as usize, y as usize));
			assert!(s > 0.);
		}
		for (w, h) in [(0., 1.), (-1., 1.), (f32::NAN, 1.), (1., f32::INFINITY)] {
			assert!(dimensions(w, h).is_err());
		}
	}
	#[test]
	fn synthetic_pages_include_content_crop_and_rotation() {
		let pdf = include_bytes!("../../tests/fixtures/pdf-preview.pdf");
		let first = render(pdf.to_vec(), 0).unwrap();
		assert_eq!(first.pages, 2);
		assert!(first.height > first.width);
		assert!(
			first
				.rgba
				.as_chunks::<4>()
				.0
				.iter()
				.filter(|p| p[0] < 100 && p[2] > 100)
				.count() > 1000,
			"colored shapes/text, not an empty page"
		);
		let second = render(pdf.to_vec(), 1).unwrap();
		assert!(second.height > second.width);
		assert!((second.width as f64 / second.height as f64 - 560. / 760.).abs() < 0.002);
		assert_eq!(render(pdf.to_vec(), 2).err(), Some(PdfError::Invalid));
		assert_eq!(
			render(b"not pdf".to_vec(), 0).err(),
			Some(PdfError::Invalid)
		);
	}

	#[test]
	fn password_and_document_caps_fail_closed() {
		assert_eq!(
			render(
				include_bytes!("../../tests/fixtures/pdf-password.pdf").to_vec(),
				0
			)
			.err(),
			Some(PdfError::Password)
		);
		assert_eq!(
			render(vec![0; MAX_PDF_BYTES + 1], 0).err(),
			Some(PdfError::Limit)
		);
		assert_eq!(
			render(b"%PDF-1.7\ninvalid".to_vec(), 0).err(),
			Some(PdfError::Invalid)
		);
		fn document(count: usize, rect: &str) -> Vec<u8> {
			let kids = (3..3 + count)
				.map(|n| format!("{n} 0 R"))
				.collect::<Vec<_>>()
				.join(" ");
			let mut objects = vec![
				"<< /Type /Catalog /Pages 2 0 R >>".into(),
				format!("<< /Type /Pages /Kids [{kids}] /Count {count} >>"),
			];
			for _ in 0..count {
				objects.push(format!(
					"<< /Type /Page /Parent 2 0 R /MediaBox [{rect}] >>"
				));
			}
			let mut bytes = b"%PDF-1.7\n".to_vec();
			let mut offsets = vec![0];
			for (n, obj) in objects.iter().enumerate() {
				offsets.push(bytes.len());
				bytes.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", n + 1).as_bytes());
			}
			let xref = bytes.len();
			bytes.extend_from_slice(
				format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
			);
			for offset in offsets.iter().skip(1) {
				bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
			}
			bytes.extend_from_slice(
				format!(
					"trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
					objects.len() + 1
				)
				.as_bytes(),
			);
			bytes
		}
		assert_eq!(
			render(document(257, "0 0 600 780"), 0).err(),
			Some(PdfError::Limit)
		);
		assert_eq!(
			render(document(1, "0 0 0 780"), 0).err(),
			Some(PdfError::Invalid)
		);
		let boundary = render(document(256, "0 0 10 10"), 255).unwrap();
		assert_eq!(boundary.pages, 256);
	}
}
