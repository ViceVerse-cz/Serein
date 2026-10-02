//! First-frame HEIC decoding through installed OS codecs; call only on a worker.

/// Recognize HEVC still-image brands, including compatible brands in `ftyp`.
pub fn is_heic(bytes: &[u8]) -> bool {
	let Some(header) = bytes.get(..16) else {
		return false;
	};
	let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
	if &header[4..8] != b"ftyp"
		|| size < 16
		|| size > bytes.len()
		|| size > 4096
		|| !size.is_multiple_of(4)
	{
		return false;
	}
	std::iter::once(&header[8..12])
		.chain(bytes[16..size].chunks_exact(4))
		.any(|brand| matches!(brand, b"heic" | b"heix" | b"heim" | b"heis"))
}

/// RGBA pixels, bounded before allocation. No decoder or codec DLL is bundled.
pub fn decode(bytes: &[u8], max_edge: u32, max_alloc: u64) -> Option<(u32, u32, Vec<u8>)> {
	if !is_heic(bytes) || bytes.len() > 32 * 1024 * 1024 {
		return None;
	}
	#[cfg(target_os = "windows")]
	{
		windows_decode(bytes, max_edge, max_alloc)
	}
	#[cfg(not(target_os = "windows"))]
	{
		let _ = (max_edge, max_alloc);
		None
	}
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
fn windows_decode(bytes: &[u8], max_edge: u32, max_alloc: u64) -> Option<(u32, u32, Vec<u8>)> {
	use windows::Win32::{Graphics::Imaging::*, System::Com::*};
	struct Com;
	impl Drop for Com {
		fn drop(&mut self) {
			// SAFETY: Balanced successful initialization on this worker thread.
			unsafe { CoUninitialize() };
		}
	}
	// SAFETY: All COM interfaces stay on this thread and drop before the COM guard.
	// The borrowed input remains alive until the WIC stream and decoder drop.
	unsafe {
		CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
		let _com = Com;
		let factory: IWICImagingFactory =
			CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
		let stream = factory.CreateStream().ok()?;
		stream.InitializeFromMemory(bytes).ok()?;
		let decoder = factory
			.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
			.ok()?;
		let frame = decoder.GetFrame(0).ok()?;
		let (mut width, mut height) = (0, 0);
		frame.GetSize(&mut width, &mut height).ok()?;
		let len = u64::from(width)
			.checked_mul(u64::from(height))?
			.checked_mul(4)?;
		if width == 0
			|| height == 0
			|| width > max_edge.min(8192)
			|| height > max_edge.min(8192)
			|| len > max_alloc.min(128 * 1024 * 1024)
		{
			return None;
		}
		let converter = factory.CreateFormatConverter().ok()?;
		converter
			.Initialize(
				&frame,
				&GUID_WICPixelFormat32bppRGBA,
				WICBitmapDitherTypeNone,
				None,
				0.0,
				WICBitmapPaletteTypeCustom,
			)
			.ok()?;
		let mut rgba = vec![0; usize::try_from(len).ok()?];
		converter
			.CopyPixels(std::ptr::null(), width.checked_mul(4)?, &mut rgba)
			.ok()?;
		Some((width, height, rgba))
	}
}
