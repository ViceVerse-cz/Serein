//! Fixed, bounded stdin/stdout protocol for the short-lived PDF raster helper.
use std::io::{Read, Write};
use ui::{MAX_PDF_BYTES, MAX_PDF_PAGES, PdfError, valid_pdf_dimensions};
const MAGIC: &[u8; 8] = b"SPDF0001";
pub struct Raster {
	pub pages: u32,
	pub page: u32,
	pub width: usize,
	pub height: usize,
	pub rgba: Vec<u8>,
}
fn invalid<T>() -> Result<T, PdfError> {
	Err(PdfError::Invalid)
}
fn eof(reader: &mut impl Read) -> Result<(), PdfError> {
	let mut byte = [0];
	if reader.read(&mut byte).map_err(|_| PdfError::Invalid)? != 0 {
		return invalid();
	}
	Ok(())
}
fn number(bytes: &[u8]) -> u32 {
	u32::from_le_bytes(bytes.try_into().expect("fixed header field"))
}
pub fn write_request(writer: &mut impl Write, page: u32, data: &[u8]) -> Result<(), PdfError> {
	if data.is_empty() || data.len() > MAX_PDF_BYTES || page >= MAX_PDF_PAGES {
		return Err(PdfError::Limit);
	}
	writer
		.write_all(MAGIC)
		.and_then(|_| writer.write_all(&page.to_le_bytes()))
		.and_then(|_| writer.write_all(&(data.len() as u32).to_le_bytes()))
		.and_then(|_| writer.write_all(data))
		.map_err(|_| PdfError::Renderer)
}
pub fn read_request(reader: &mut impl Read) -> Result<(u32, Vec<u8>), PdfError> {
	let mut header = [0; 16];
	reader
		.read_exact(&mut header)
		.map_err(|_| PdfError::Invalid)?;
	if &header[..8] != MAGIC {
		return invalid();
	}
	let page = number(&header[8..12]);
	let len = number(&header[12..16]) as usize;
	if len == 0 || len > MAX_PDF_BYTES || page >= MAX_PDF_PAGES {
		return Err(PdfError::Limit);
	}
	let mut data = vec![0; len];
	reader
		.read_exact(&mut data)
		.map_err(|_| PdfError::Invalid)?;
	eof(reader)?;
	Ok((page, data))
}
fn status(error: PdfError) -> u32 {
	match error {
		PdfError::Unavailable => 1,
		PdfError::Limit => 2,
		PdfError::Invalid => 3,
		PdfError::Password => 4,
		PdfError::TimedOut => 5,
		PdfError::Renderer => 6,
	}
}
fn error(status: u32) -> Result<PdfError, PdfError> {
	Ok(match status {
		1 => PdfError::Unavailable,
		2 => PdfError::Limit,
		3 => PdfError::Invalid,
		4 => PdfError::Password,
		5 => PdfError::TimedOut,
		6 => PdfError::Renderer,
		_ => return invalid(),
	})
}
pub fn write_response(
	writer: &mut impl Write,
	result: &Result<Raster, PdfError>,
) -> Result<(), PdfError> {
	let mut header = Vec::with_capacity(28);
	header.extend_from_slice(MAGIC);
	match result {
		Ok(r) => {
			if r.pages == 0
				|| r.pages > MAX_PDF_PAGES
				|| r.page >= r.pages
				|| !valid_pdf_dimensions(r.width, r.height)
				|| r.rgba.len() != r.width * r.height * 4
			{
				return invalid();
			}
			for n in [0, r.pages, r.page, r.width as u32, r.height as u32] {
				header.extend_from_slice(&n.to_le_bytes());
			}
		}
		Err(e) => {
			for n in [status(*e), 0, 0, 0, 0] {
				header.extend_from_slice(&n.to_le_bytes());
			}
		}
	}
	writer.write_all(&header).map_err(|_| PdfError::Renderer)?;
	if let Ok(r) = result {
		writer.write_all(&r.rgba).map_err(|_| PdfError::Renderer)?;
	}
	writer.flush().map_err(|_| PdfError::Renderer)
}
pub fn read_response(reader: &mut impl Read, expected_page: u32) -> Result<Raster, PdfError> {
	let mut header = [0; 28];
	reader
		.read_exact(&mut header)
		.map_err(|_| PdfError::Renderer)?;
	if &header[..8] != MAGIC {
		return invalid();
	}
	let code = number(&header[8..12]);
	let pages = number(&header[12..16]);
	let page = number(&header[16..20]);
	let width = number(&header[20..24]) as usize;
	let height = number(&header[24..28]) as usize;
	if code != 0 {
		if [pages, page, width as u32, height as u32] != [0; 4] {
			return invalid();
		}
		eof(reader)?;
		return Err(error(code)?);
	}
	if pages == 0
		|| pages > MAX_PDF_PAGES
		|| page >= pages
		|| page != expected_page
		|| !valid_pdf_dimensions(width, height)
	{
		return invalid();
	}
	let mut rgba = vec![0; width * height * 4];
	reader
		.read_exact(&mut rgba)
		.map_err(|_| PdfError::Invalid)?;
	eof(reader)?;
	Ok(Raster {
		pages,
		page,
		width,
		height,
		rgba,
	})
}
#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn request_bounds_and_trailing_bytes() {
		let mut out = Vec::new();
		write_request(&mut out, 1, b"%PDF-test").unwrap();
		assert_eq!(
			read_request(&mut out.as_slice()).unwrap(),
			(1, b"%PDF-test".to_vec())
		);
		out.push(0);
		assert!(read_request(&mut out.as_slice()).is_err());
		out[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
		assert_eq!(
			read_request(&mut out.as_slice()).err(),
			Some(PdfError::Limit)
		);
		assert!(write_request(&mut Vec::new(), 256, b"x").is_err());
	}
	#[test]
	fn response_validation_precedes_pixel_allocation() {
		let mut out = Vec::new();
		write_response(
			&mut out,
			&Ok(Raster {
				pages: 2,
				page: 1,
				width: 1,
				height: 1,
				rgba: vec![255; 4],
			}),
		)
		.unwrap();
		assert!(read_response(&mut out.as_slice(), 1).is_ok());
		assert!(read_response(&mut out.as_slice(), 0).is_err());
		out.push(0);
		assert!(read_response(&mut out.as_slice(), 1).is_err());
		out[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
		assert!(read_response(&mut out.as_slice(), 1).is_err());
		for e in [
			PdfError::Limit,
			PdfError::Invalid,
			PdfError::Password,
			PdfError::Unavailable,
			PdfError::TimedOut,
			PdfError::Renderer,
		] {
			out.clear();
			write_response(&mut out, &Err(e)).unwrap();
			assert_eq!(read_response(&mut out.as_slice(), 0).err(), Some(e));
		}
	}
}
