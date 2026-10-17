//! Fixed-width native ABI for the actual renderer's physical GPU identity.
#![allow(unsafe_code)]

use model::{VideoAdapter, VideoAdapterIdentity};
use std::{
	ffi::c_void,
	marker::PhantomData,
	ptr::NonNull,
	rc::Rc,
	sync::atomic::{AtomicUsize, Ordering},
};

/// Coded surface and retained-picture admission shared by streams and attachments.
/// Native drivers also have internal allocations; this is a conservative media
/// working-set reservation, not an OS process-memory or GPU-memory measurement.
pub const MEMORY_LIMIT: usize = 3 * 1024 * 1024 * 1024;
pub const MAX_PIXELS: usize = 7680 * 4320;
pub const MAX_CODED_PIXELS: usize = 7680 * 4352;
pub const MAX_UNIT: usize = 8 * 1024 * 1024 + 65536;
static RESERVED: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub struct MemoryLease(usize);
impl MemoryLease {
	pub fn resize(&mut self, bytes: usize) -> Result<(), &'static str> {
		if bytes > MEMORY_LIMIT {
			return Err("This video exceeds the playback memory limit");
		}
		if bytes > self.0 {
			let delta = bytes - self.0;
			RESERVED
				.fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
					used.checked_add(delta)
						.filter(|total| *total <= MEMORY_LIMIT)
				})
				.map_err(|_| "Video playback memory is busy; close another video and try again")?;
		} else {
			RESERVED.fetch_sub(self.0 - bytes, Ordering::AcqRel);
		}
		self.0 = bytes;
		Ok(())
	}
}
impl Drop for MemoryLease {
	fn drop(&mut self) {
		RESERVED.fetch_sub(self.0, Ordering::AcqRel);
	}
}

fn decoder_bytes(
	width: usize,
	height: usize,
	depth: u32,
	chroma: u32,
	references: u32,
) -> Result<usize, &'static str> {
	let pixels = width.checked_mul(height).ok_or(super::INVALID)?;
	if width == 0
		|| height == 0
		|| width > 7680
		|| height > 7680
		|| pixels > MAX_CODED_PIXELS
		|| !(8..=10).contains(&depth)
		|| chroma > 1
		|| references > 16
	{
		return Err("This video resolution or profile is not supported");
	}
	// Include aligned native planes, reference pictures, decoder/presentation
	// headroom and conversion scratch before submitting the compressed picture.
	let aligned = width.next_multiple_of(128) * height.next_multiple_of(128);
	let surface = aligned * if depth > 8 { 3 } else { 2 };
	Ok(surface * (references as usize + 8) + aligned * 8)
}

/// HDR transfer functions supported throughout the shared media boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrTransfer {
	Pq,
	Hlg,
}
impl HdrTransfer {
	pub fn from_native(value: u32) -> Option<Self> {
		match value {
			16 => Some(Self::Pq),
			18 => Some(Self::Hlg),
			_ => None,
		}
	}
}

/// Stable native frame ABI: SDR RGBA8 or PQ/HLG P010 with explicit colorimetry.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Picture {
	pub width: u32,
	pub height: u32,
	pub format: u32,
	pub primaries: u32,
	pub transfer: u32,
	pub matrix: u32,
	pub full_range: u32,
	pub depth: u32,
	pub rotation: u32,
	pub peak_nits: f32,
	pub pts: i64,
	pub bytes: usize,
	/// FFmpeg chroma location: unspecified/left, center, top-left/top, bottom-left/bottom.
	pub chroma_location: u32,
}
impl Picture {
	pub fn sdr(width: u32, height: u32) -> Self {
		Self {
			width,
			height,
			depth: 8,
			bytes: (width as usize)
				.saturating_mul(height as usize)
				.saturating_mul(4),
			..Self::default()
		}
	}
	pub fn hdr(self) -> bool {
		self.format == 1 && HdrTransfer::from_native(self.transfer).is_some()
	}
	pub fn validate(self) -> Result<(), &'static str> {
		let pixels = (self.width as usize)
			.checked_mul(self.height as usize)
			.ok_or(super::INVALID)?;
		let expected = match self.format {
			0 => pixels.checked_mul(4),
			1 if self.hdr() => {
				let chroma = (self.width as usize)
					.div_ceil(2)
					.checked_mul((self.height as usize).div_ceil(2))
					.and_then(|bytes| bytes.checked_mul(4));
				pixels
					.checked_mul(2)
					.and_then(|bytes| bytes.checked_add(chroma?))
			}
			_ => return Err(super::UNSUPPORTED),
		};
		if self.hdr()
			&& (!matches!(self.primaries, 1 | 9 | 12) || !matches!(self.matrix, 1 | 5 | 6 | 9))
		{
			return Err(super::UNSUPPORTED);
		}
		if self.width == 0
			|| self.height == 0
			|| self.width > 7680
			|| self.height > 7680
			|| pixels > MAX_PIXELS
			|| expected != Some(self.bytes)
			|| !(8..=10).contains(&self.depth)
			|| !matches!(self.rotation, 0 | 90 | 180 | 270)
			|| self.full_range > 1
			|| self.chroma_location > 6
			|| self.hdr()
				&& (!matches!(self.primaries, 1 | 9 | 12)
					|| !matches!(self.matrix, 1 | 5 | 6 | 9)
					|| !(100.0..=10000.0).contains(&self.peak_nits))
			|| !self.peak_nits.is_finite()
		{
			return Err(super::INVALID);
		}
		Ok(())
	}
}

/// Owns one native codec exclusively on the media worker that opened it.
pub struct LiveDecoder {
	native: NonNull<c_void>,
	codec: u32,
	lease: MemoryLease,
	_thread: PhantomData<Rc<()>>,
}
impl Drop for LiveDecoder {
	fn drop(&mut self) {
		// SAFETY: This sole owner closes the context on its owning worker.
		unsafe { serein_decode_close(self.native.as_ptr()) };
	}
}
impl LiveDecoder {
	pub fn new(
		codec: u32,
		adapter: Option<VideoAdapter>,
		hardware: bool,
	) -> Result<Self, &'static str> {
		if codec > 2 || hardware && adapter.is_none() {
			return Err(super::UNSUPPORTED);
		}
		let adapter = adapter.map(Adapter::from);
		// SAFETY: The adapter is read synchronously and never retained by C.
		let native = unsafe {
			serein_decode_open(
				codec as i32,
				adapter
					.as_ref()
					.map_or(std::ptr::null(), std::ptr::from_ref),
				i32::from(hardware),
			)
		};
		Ok(Self {
			native: NonNull::new(native).ok_or(super::UNSUPPORTED)?,
			codec,
			lease: MemoryLease::default(),
			_thread: PhantomData,
		})
	}
	pub fn codec(&self) -> u32 {
		self.codec
	}
	pub fn inspect(&mut self, data: &[u8]) -> Result<Option<Header>, &'static str> {
		if data.is_empty() || data.len() > MAX_UNIT {
			return Err(super::INVALID);
		}
		let mut header = Header::default();
		// SAFETY: C parses a copied, padded access unit and writes this fixed ABI record.
		let parsed = decode_result(unsafe {
			serein_decode_header(self.native.as_ptr(), data.as_ptr(), data.len(), &mut header)
		})?;
		Ok(parsed.then_some(header))
	}
	pub fn hardware(&self) -> bool {
		// SAFETY: Context remains exclusively owned and live.
		unsafe { serein_decode_hardware(self.native.as_ptr()) != 0 }
	}
	pub fn reserve(
		&mut self,
		width: usize,
		height: usize,
		depth: u32,
		chroma: u32,
		references: u32,
	) -> Result<(), &'static str> {
		let bytes = decoder_bytes(width, height, depth, chroma, references)?;
		self.lease.resize(bytes.max(self.lease.0))
	}
	pub fn send(&mut self, data: &[u8], pts: i64) -> Result<bool, &'static str> {
		if data.is_empty() || data.len() > MAX_UNIT {
			return Err(super::INVALID);
		}
		// SAFETY: C copies the bounded access unit before returning.
		decode_result(unsafe {
			serein_decode_send(self.native.as_ptr(), data.as_ptr(), data.len(), pts)
		})
	}
	pub fn receive(&mut self, output: &mut Vec<u8>) -> Result<Option<Picture>, &'static str> {
		let mut picture = Picture::default();
		// SAFETY: Inspection retains the native frame and writes only this ABI record.
		if !decode_result(unsafe {
			serein_decode_receive(self.native.as_ptr(), &mut picture, std::ptr::null_mut(), 0)
		})? {
			return Ok(None);
		}
		picture.validate()?;
		if self.lease.0 == 0 {
			// Callers with parsed sequence headers reserve before send. This guard
			// also covers anonymous file streams whose dimensions are container supplied.
			return Err("Video decoding requires a surface reservation");
		}
		if output.len() < picture.bytes {
			output
				.try_reserve_exact(picture.bytes - output.len())
				.map_err(|_| super::INVALID)?;
			output.resize(picture.bytes, 0);
		}
		// SAFETY: The validated length exactly matches the writable output slice.
		let bytes = picture.bytes;
		decode_result(unsafe {
			serein_decode_receive(
				self.native.as_ptr(),
				&mut picture,
				output.as_mut_ptr(),
				bytes,
			)
		})?;
		Ok(Some(picture))
	}
}

fn decode_result(result: i32) -> Result<bool, &'static str> {
	match result {
		1 => Ok(true),
		0 => Ok(false),
		-2 => Err(super::UNSUPPORTED),
		-3 => Err("This video exceeds the safe playback limits"),
		_ => Err(super::INVALID),
	}
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Adapter {
	pub identity: u32,
	pub vendor_id: u32,
	pub device_id: u32,
	pub domain: u32,
	pub bus: u32,
	pub slot: u32,
	pub function: u32,
	pub value: u64,
}

impl From<VideoAdapter> for Adapter {
	fn from(value: VideoAdapter) -> Self {
		let mut result = Self {
			vendor_id: value.vendor_id,
			device_id: value.device_id,
			..Self::default()
		};
		match value.identity {
			VideoAdapterIdentity::Unidentified => {}
			VideoAdapterIdentity::WindowsLuid(value) => {
				result.identity = 1;
				result.value = value;
			}
			VideoAdapterIdentity::Pci {
				domain,
				bus,
				device,
				function,
			} => {
				result.identity = 2;
				result.domain = domain;
				result.bus = u32::from(bus);
				result.slot = u32::from(device);
				result.function = u32::from(function);
			}
			VideoAdapterIdentity::MetalRegistry(value) => {
				result.identity = 3;
				result.value = value;
			}
		}
		result
	}
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Header {
	pub coded_width: u32,
	pub coded_height: u32,
	pub width: u32,
	pub height: u32,
	pub depth: u32,
	pub chroma: u32,
	pub references: u32,
}
unsafe extern "C" {
	fn serein_decode_header(
		decoder: *mut c_void,
		data: *const u8,
		bytes: usize,
		header: *mut Header,
	) -> i32;
	fn serein_decode_open(codec: i32, adapter: *const Adapter, hardware: i32) -> *mut c_void;
	fn serein_decode_close(decoder: *mut c_void);
	fn serein_decode_hardware(decoder: *mut c_void) -> i32;
	fn serein_decode_send(decoder: *mut c_void, data: *const u8, bytes: usize, pts: i64) -> i32;
	fn serein_decode_receive(
		decoder: *mut c_void,
		picture: *mut Picture,
		output: *mut u8,
		capacity: usize,
	) -> i32;
	fn serein_video_query_on_adapter(backend: i32, codec: i32, adapter: *const Adapter) -> i32;
	#[cfg(target_os = "linux")]
	fn serein_video_cuda_device(adapter: *const Adapter) -> i32;
	#[cfg(target_os = "linux")]
	fn serein_video_drm_device(
		adapter: *const Adapter,
		path: *mut std::ffi::c_char,
		capacity: usize,
	) -> i32;
}

pub fn query_on_adapter(backend: i32, codec: i32, adapter: VideoAdapter) -> i32 {
	let adapter = Adapter::from(adapter);
	// SAFETY: The native query reads this fixed-width value synchronously. It
	// creates metadata sessions only, never a capture or encoded picture.
	unsafe { serein_video_query_on_adapter(backend, codec, &adapter) }
}

/// A driver profile query, without encoding or capturing a synthetic picture.
pub fn query_ten_bit(backend: i32, codec: i32, adapter: VideoAdapter) -> i32 {
	if codec == 0 {
		return 0;
	}
	let adapter = Adapter::from(adapter);
	// SAFETY: The +3 selector requests 10-bit capability on this exact device.
	unsafe { serein_video_query_on_adapter(backend, codec + 3, &adapter) }
}

#[cfg(target_os = "linux")]
pub fn cuda_device(adapter: VideoAdapter) -> Option<u32> {
	// SAFETY: The adapter remains alive for the bounded CUDA enumeration.
	u32::try_from(unsafe { serein_video_cuda_device(&Adapter::from(adapter)) }).ok()
}

#[cfg(target_os = "linux")]
pub fn drm_device(adapter: VideoAdapter) -> Option<String> {
	let mut path = [0 as std::ffi::c_char; 80];
	// SAFETY: The fixed-width adapter and writable buffer outlive this call;
	// success guarantees a terminating zero within the supplied capacity.
	let found =
		unsafe { serein_video_drm_device(&Adapter::from(adapter), path.as_mut_ptr(), path.len()) };
	if found == 0 {
		return None;
	}
	// SAFETY: The C helper's successful snprintf always terminates the buffer.
	unsafe { std::ffi::CStr::from_ptr(path.as_ptr()) }
		.to_str()
		.ok()
		.map(str::to_owned)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn physical_identity_keeps_same_vendor_adapters_distinct() {
		let first = Adapter::from(VideoAdapter {
			vendor_id: 0x10de,
			device_id: 0x2684,
			identity: VideoAdapterIdentity::Pci {
				domain: 0,
				bus: 1,
				device: 0,
				function: 0,
			},
		});
		let second = Adapter::from(VideoAdapter {
			identity: VideoAdapterIdentity::Pci {
				domain: 0,
				bus: 2,
				device: 0,
				function: 0,
			},
			vendor_id: 0x10de,
			device_id: 0x2684,
		});
		assert_eq!(first.identity, 2);
		assert_ne!(first.bus, second.bus);
		assert_eq!(Adapter::from(VideoAdapter::default()).identity, 0);
	}

	#[test]
	fn color_reference_preserves_black_white_and_highlight_order() {
		assert!(pq_nits(0.0) < 0.001);
		assert!((pq_nits(0.58069) - 203.0).abs() < 0.15);
		assert!((pq_nits(1.0) - 10000.0).abs() < 0.1);
		let hlg = hlg_nits([0.75; 3], 1000.0);
		assert!((hlg[0] - 203.15).abs() < 0.5);
		for primaries in [1, 9, 12] {
			assert_eq!(sdr_rgb([0.0; 3], primaries, 1000.0, 203.0), [0; 3]);
			let mid = sdr_rgb([100.0; 3], primaries, 1000.0, 203.0);
			let white = sdr_rgb([203.0; 3], primaries, 1000.0, 203.0);
			let high = sdr_rgb([1000.0; 3], primaries, 1000.0, 203.0);
			assert!(mid[0] < white[0] && white[0] <= high[0]);
			assert!(
				white.iter().copied().max().unwrap() - white.iter().copied().min().unwrap() <= 1
			);
			assert_eq!(sdr_rgb([203.0; 3], primaries, 203.0, 203.0), [255; 3]);
		}
		assert_eq!(half([0, 0x3c]), 1.0);
		assert_eq!(half([0, 0x7c]), 0.0);
		assert_eq!(half([1, 0x7e]), 0.0);
	}

	#[test]
	fn frame_bounds_cover_ultrawide_portrait_and_invalid_metadata() {
		let aligned = 1920 * 1152;
		assert_eq!(decoder_bytes(1920, 1080, 10, 1, 16), Ok(aligned * 80));
		assert!(decoder_bytes(usize::MAX, usize::MAX, 10, 1, 16).is_err());
		assert!(decoder_bytes(6144, 2560, 12, 1, 16).is_err());
		assert!(decoder_bytes(6144, 2560, 10, 2, 16).is_err());
		assert!(decoder_bytes(6144, 2560, 10, 1, 17).is_err());
		for (width, height) in [
			(854, 480),
			(1280, 720),
			(1920, 1080),
			(6144, 2560),
			(7680, 4320),
			(4320, 7680),
		] {
			assert!(Picture::sdr(width, height).validate().is_ok());
		}
		assert!(Picture::sdr(7680, 7680).validate().is_err());
		assert!(Picture::sdr(7682, 4320).validate().is_err());
		let picture = Picture {
			format: 1,
			transfer: 16,
			primaries: 9,
			matrix: 9,
			depth: 10,
			peak_nits: 1000.0,
			bytes: 16 * 8 * 3,
			..Picture::sdr(16, 8)
		};
		let frame = Frame::copy(picture, &vec![0; picture.bytes]).unwrap();
		let (y, stride, uv) = frame.planes();
		assert_eq!(
			(y.len(), stride, uv.unwrap().0.len()),
			(16 * 8 * 2, 32, 16 * 8)
		);
		let cropped = Picture {
			width: 3,
			height: 3,
			bytes: 18 + 16,
			..picture
		};
		let frame = Frame::copy(cropped, &[0; 34]).unwrap();
		let (y, stride, uv) = frame.planes();
		assert_eq!(
			(y.len(), stride, uv.unwrap().0.len(), uv.unwrap().1),
			(18, 6, 16, 8)
		);
		assert!(Picture::sdr(u32::MAX, u32::MAX).validate().is_err());
		for bad in [
			Picture {
				depth: 12,
				..picture
			},
			Picture {
				transfer: 2,
				..picture
			},
			Picture {
				matrix: 10,
				..picture
			},
			Picture {
				peak_nits: f32::NAN,
				..picture
			},
			Picture {
				bytes: 0,
				..picture
			},
		] {
			assert!(bad.validate().is_err());
		}
		assert!(Frame::copy(picture, &[0; 8]).is_err());
		assert!(MemoryLease::default().resize(MEMORY_LIMIT + 1).is_err());
	}

	#[test]
	fn hdr_files_decode_seek_drain_audio_and_cancel_bounded_reads() {
		use std::{
			io::{Cursor, Read, Seek},
			sync::{Arc, atomic::AtomicBool},
			task::Poll,
		};
		struct Source {
			cursor: Cursor<Vec<u8>>,
			cancelled: Arc<AtomicBool>,
			largest: Arc<AtomicUsize>,
		}
		impl Read for Source {
			fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
				self.largest.fetch_max(bytes.len(), Ordering::Relaxed);
				if self.cancelled.load(Ordering::Acquire) {
					return Err(std::io::ErrorKind::Interrupted.into());
				}
				self.cursor.read(bytes)
			}
		}
		impl Seek for Source {
			fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
				self.cursor.seek(position)
			}
		}
		for (compressed, transfer) in [
			(FILE_HEVC_PQ, 16),
			(FILE_HEVC_HLG, 18),
			(FILE_AV1_PQ, 16),
			(FILE_AV1_HLG, 18),
		] {
			let mut bytes = Vec::new();
			flate2::read::ZlibDecoder::new(compressed)
				.take(100 * 1024)
				.read_to_end(&mut bytes)
				.unwrap();
			let cancelled = Arc::new(AtomicBool::new(false));
			let largest = Arc::new(AtomicUsize::new(0));
			let source = Source {
				cursor: Cursor::new(bytes.clone()),
				cancelled: cancelled.clone(),
				largest: largest.clone(),
			};
			let mut file =
				File::open(Box::new(source), None).unwrap_or_else(|(_, error)| panic!("{error}"));
			assert_eq!(
				(
					file.info().width,
					file.info().height,
					file.info().sample_rate
				),
				(32, 16, 48000)
			);
			for _ in 0..2 {
				let mut video = Vec::new();
				let mut audio = Vec::new();
				let mut samples = 0;
				let mut ended = [false; 2];
				for _ in 0..256 {
					if !ended[0] {
						match file.poll_video().unwrap() {
							Poll::Ready(Some(Sample::Picture { pts, frame })) => {
								assert_eq!(
									(
										frame.picture().format,
										frame.picture().transfer,
										frame.picture().depth
									),
									(1, transfer, 10)
								);
								video.push(pts);
							}
							Poll::Ready(None) => ended[0] = true,
							Poll::Pending => {}
							_ => panic!("wrong video sample"),
						}
					}
					if !ended[1] {
						match file.poll_audio().unwrap() {
							Poll::Ready(Some(Sample::Audio { pts, frames })) => {
								assert!(frames.iter().flatten().all(|v| v.is_finite()));
								audio.push(pts);
								samples += frames.len();
							}
							Poll::Ready(None) => ended[1] = true,
							Poll::Pending => {}
							_ => panic!("wrong audio sample"),
						}
					}
					if ended == [true; 2] {
						break;
					}
				}
				assert_eq!(
					ended, [true; 2],
					"tracks must finish without retaining blocked packets"
				);
				assert_eq!(video.len(), 3);
				assert!(video.windows(2).all(|pair| pair[1] > pair[0]));
				assert!(audio.windows(2).all(|pair| pair[1] >= pair[0]));
				assert!(samples >= 2400);
				assert!((video[2] - 2.0 / 60.0).abs() < 0.001);
				assert!(audio.last().copied().unwrap() < file.info().duration + 0.05);
				file.seek(0.0).unwrap();
			}
			assert!(file.seek(f64::NAN).is_err());
			assert!(file.seek(file.info().duration + 1.0).is_err());
			assert!(largest.load(Ordering::Relaxed) <= 65536);
			drop(file);
			cancelled.store(true, Ordering::Release);
			let source = Source {
				cursor: Cursor::new(bytes),
				cancelled,
				largest,
			};
			assert!(File::open(Box::new(source), None).is_err());
		}
		assert!(File::open(Box::new(Cursor::new(b"not a movie")), None).is_err());
	}
}

/// An owned, byte-accounted display frame. Cloning an Arc does not copy its planes.
pub struct Frame {
	picture: Picture,
	data: Vec<u8>,
	_memory: MemoryLease,
}
/// Bytes and stride for the primary plane and an optional interleaved UV plane.
pub type FramePlanes<'a> = (&'a [u8], usize, Option<(&'a [u8], usize)>);
impl Frame {
	pub fn picture(&self) -> Picture {
		self.picture
	}
	pub fn new(picture: Picture, data: Vec<u8>) -> Result<Self, &'static str> {
		picture.validate()?;
		if data.len() != picture.bytes {
			return Err(super::INVALID);
		}
		let mut memory = MemoryLease::default();
		memory.resize(data.capacity())?;
		Ok(Self {
			picture,
			data,
			_memory: memory,
		})
	}
	pub fn copy(picture: Picture, data: &[u8]) -> Result<Self, &'static str> {
		picture.validate()?;
		if data.len() != picture.bytes {
			return Err(super::INVALID);
		}
		let mut memory = MemoryLease::default();
		memory.resize(data.len())?;
		let mut owned = Vec::new();
		owned
			.try_reserve_exact(data.len())
			.map_err(|_| super::INVALID)?;
		owned.extend_from_slice(data);
		Ok(Self {
			picture,
			data: owned,
			_memory: memory,
		})
	}
	/// Plane bytes and row stride, after tightly packed output validation.
	pub fn planes(&self) -> FramePlanes<'_> {
		let width = self.picture.width as usize;
		if self.picture.hdr() {
			let y_bytes = width * self.picture.height as usize * 2;
			let (y, uv) = self.data.split_at(y_bytes);
			(y, width * 2, Some((uv, width.div_ceil(2) * 4)))
		} else {
			(&self.data, width * 4, None)
		}
	}
}

/// Samples from the shared player. Legacy platform SDR samples adapt here so
/// existing native decoders keep their interface and the FFmpeg path retains
/// its owned frame reservation through presentation.
pub enum Sample {
	Picture { pts: f64, frame: Frame },
	Video { pts: f64, picture: super::Picture },
	Audio { pts: f64, frames: Vec<[f32; 2]> },
}
impl From<super::Sample> for Sample {
	fn from(sample: super::Sample) -> Self {
		match sample {
			super::Sample::Video { pts, picture } => Self::Video { pts, picture },
			super::Sample::Audio { pts, frames } => Self::Audio { pts, frames },
		}
	}
}

#[repr(C)]
#[derive(Default)]
struct MediaInfo {
	width: u32,
	height: u32,
	depth: u32,
	references: u32,
	sample_rate: u32,
	rotation: u32,
	duration: f64,
}
struct Reader {
	source: Box<dyn super::ReadSeek>,
	length: u64,
}
// Native callbacks synchronously borrow an exclusively worker-owned reader. A
// panicking custom reader must not unwind across the C ABI.
unsafe extern "C" fn media_read(reader: *mut c_void, output: *mut u8, capacity: i32) -> i32 {
	if reader.is_null() || output.is_null() || !(1..=65536).contains(&capacity) {
		return -5;
	}
	std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		// SAFETY: File owns this Box and the native AVIO buffer for the entire call.
		let reader = unsafe { &mut *reader.cast::<Reader>() };
		let output = unsafe { std::slice::from_raw_parts_mut(output, capacity as usize) };
		match reader.source.read(output) {
			Ok(0) => -541478725,
			Ok(n) if n <= output.len() => n as i32,
			Ok(_) => -5,
			Err(_) => -5,
		}
	}))
	.unwrap_or(-5)
}
unsafe extern "C" fn media_seek(reader: *mut c_void, offset: i64, whence: i32) -> i64 {
	if reader.is_null() {
		return -5;
	}
	std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
		// SAFETY: Only the owning worker invokes these synchronous callbacks.
		let reader = unsafe { &mut *reader.cast::<Reader>() };
		if whence == 0x10000 {
			return reader.length as i64;
		}
		let seek = match whence & !0x20000 {
			0 if offset >= 0 => std::io::SeekFrom::Start(offset as u64),
			1 => std::io::SeekFrom::Current(offset),
			2 => std::io::SeekFrom::End(offset),
			_ => return -5,
		};
		match reader.source.seek(seek) {
			Ok(n) if n <= reader.length => n as i64,
			_ => -5,
		}
	}))
	.unwrap_or(-5)
}

/// MOV/Matroska H264/HEVC/AV1 decoding from the application's anonymous reader.
/// No native URL, filename, protocol or HTTP client is exposed to FFmpeg.
pub struct File {
	native: NonNull<c_void>,
	_reader: Box<Reader>,
	_memory: MemoryLease,
	info: super::Info,
	restarted: bool,
	_thread: PhantomData<Rc<()>>,
}
impl Drop for File {
	fn drop(&mut self) {
		// SAFETY: Close all AVIO/codec references before dropping the reader Box.
		unsafe { serein_media_close(self.native.as_ptr()) };
	}
}
impl File {
	pub fn open(
		mut source: Box<dyn super::ReadSeek>,
		adapter: Option<VideoAdapter>,
	) -> Result<Self, (Box<dyn super::ReadSeek>, &'static str)> {
		use std::io::SeekFrom;
		let length = match source.seek(SeekFrom::End(0)).and_then(|length| {
			source.seek(SeekFrom::Start(0))?;
			Ok(length)
		}) {
			Ok(length) if (1..=100 * 1024 * 1024).contains(&length) => length,
			_ => return Err((source, super::INVALID)),
		};
		let mut reader = Box::new(Reader { source, length });
		let mut info = MediaInfo::default();
		let adapter = adapter.map(Adapter::from);
		// SAFETY: Reader Box has a stable address and remains owned until native close.
		let native = unsafe {
			serein_media_open(
				(&mut *reader as *mut Reader).cast(),
				media_read,
				media_seek,
				adapter
					.as_ref()
					.map_or(std::ptr::null(), std::ptr::from_ref),
				&mut info,
			)
		};
		let Some(native) = NonNull::new(native) else {
			return Err((reader.source, super::UNSUPPORTED));
		};
		let mut memory = MemoryLease::default();
		let result = Picture::sdr(info.width, info.height)
			.validate()
			.and_then(|()| {
				if !info.duration.is_finite()
					|| !(0.0..=super::MAX_SECONDS).contains(&info.duration)
					|| info.depth > 10
					|| info.references > 16
				{
					return Err(super::INVALID);
				}
				memory.resize(decoder_bytes(
					info.width as usize,
					info.height as usize,
					info.depth,
					1,
					info.references,
				)?)
			});
		if let Err(error) = result {
			// SAFETY: open returned an owned live context; no sample has been read.
			unsafe { serein_media_close(native.as_ptr()) };
			return Err((reader.source, error));
		}
		let (width, height) = if matches!(info.rotation, 90 | 270) {
			(info.height, info.width)
		} else {
			(info.width, info.height)
		};
		Ok(Self {
			native,
			_reader: reader,
			_memory: memory,
			restarted: false,
			info: super::Info {
				width,
				height,
				duration: info.duration,
				sample_rate: info.sample_rate,
				channels: if info.sample_rate > 0 { 2 } else { 0 },
			},
			_thread: PhantomData,
		})
	}
	pub fn info(&self) -> super::Info {
		self.info
	}
	/// Retire presentation/audio buffers after a first-frame hardware fallback.
	pub fn take_restart(&mut self) -> bool {
		std::mem::take(&mut self.restarted)
	}
	pub fn seek(&mut self, seconds: f64) -> Result<(), &'static str> {
		if !seconds.is_finite() || seconds < 0.0 || seconds > self.info.duration {
			return Err(super::INVALID);
		}
		// SAFETY: Seek flushes this exclusively owned decoder and both bounded tracks.
		decode_result(unsafe { serein_media_seek(self.native.as_ptr(), seconds) }).map(|_| ())
	}
	fn poll(&mut self, audio: bool) -> Result<std::task::Poll<bool>, &'static str> {
		// SAFETY: Track polling is confined to this File's worker.
		match unsafe { serein_media_poll(self.native.as_ptr(), i32::from(audio)) } {
			0 => Ok(std::task::Poll::Pending),
			1 => Ok(std::task::Poll::Ready(true)),
			2 => Ok(std::task::Poll::Ready(false)),
			3 => {
				self.restarted = true;
				Ok(std::task::Poll::Pending)
			}
			code => {
				decode_result(code)?;
				Err(super::INVALID)
			}
		}
	}
	pub fn poll_video(&mut self) -> Result<std::task::Poll<Option<Sample>>, &'static str> {
		use std::task::Poll;
		match self.poll(false)? {
			Poll::Pending => return Ok(Poll::Pending),
			Poll::Ready(false) => return Ok(Poll::Ready(None)),
			Poll::Ready(true) => {}
		}
		let mut picture = Picture::default();
		let mut pts = 0.0;
		// SAFETY: Inspect one retained bounded native picture before allocating output.
		decode_result(unsafe {
			serein_media_video(
				self.native.as_ptr(),
				&mut picture,
				std::ptr::null_mut(),
				0,
				&mut pts,
			)
		})?;
		picture.validate()?;
		let mut memory = MemoryLease::default();
		memory.resize(picture.bytes)?;
		let mut bytes = Vec::new();
		bytes
			.try_reserve_exact(picture.bytes)
			.map_err(|_| super::INVALID)?;
		bytes.resize(picture.bytes, 0);
		let capacity = bytes.len();
		// SAFETY: The output allocation matches the inspected picture exactly.
		decode_result(unsafe {
			serein_media_video(
				self.native.as_ptr(),
				&mut picture,
				bytes.as_mut_ptr(),
				capacity,
				&mut pts,
			)
		})?;
		Ok(Poll::Ready(Some(Sample::Picture {
			pts,
			frame: Frame {
				picture,
				data: bytes,
				_memory: memory,
			},
		})))
	}
	pub fn poll_audio(&mut self) -> Result<std::task::Poll<Option<Sample>>, &'static str> {
		use std::task::Poll;
		match self.poll(true)? {
			Poll::Pending => return Ok(Poll::Pending),
			Poll::Ready(false) => return Ok(Poll::Ready(None)),
			Poll::Ready(true) => {}
		}
		let mut frames = vec![[0.0f32; 2]; 65536];
		let mut written = 0;
		let mut pts = 0.0;
		// SAFETY: C writes at most this fixed stereo PCM allocation and reports its length.
		decode_result(unsafe {
			serein_media_audio(
				self.native.as_ptr(),
				frames.as_mut_ptr().cast(),
				frames.len(),
				&mut written,
				&mut pts,
			)
		})?;
		if written > frames.len() || !pts.is_finite() {
			return Err(super::INVALID);
		}
		frames.truncate(written);
		Ok(Poll::Ready(Some(Sample::Audio { pts, frames })))
	}
}
unsafe extern "C" {
	fn serein_media_open(
		reader: *mut c_void,
		read: unsafe extern "C" fn(*mut c_void, *mut u8, i32) -> i32,
		seek: unsafe extern "C" fn(*mut c_void, i64, i32) -> i64,
		adapter: *const Adapter,
		info: *mut MediaInfo,
	) -> *mut c_void;
	fn serein_media_close(media: *mut c_void);
	fn serein_media_poll(media: *mut c_void, audio: i32) -> i32;
	fn serein_media_video(
		media: *mut c_void,
		picture: *mut Picture,
		output: *mut u8,
		capacity: usize,
		pts: *mut f64,
	) -> i32;
	fn serein_media_audio(
		media: *mut c_void,
		output: *mut f32,
		capacity: usize,
		written: *mut usize,
		pts: *mut f64,
	) -> i32;
	fn serein_media_seek(media: *mut c_void, seconds: f64) -> i32;
}

/// Reference color conversion used on capture workers and in renderer fixtures.
/// Inputs are absolute linear-light nits in the declared primaries.
pub fn sdr_rgb(mut rgb: [f32; 3], primaries: u32, peak: f32, white: f32) -> [u8; 3] {
	let matrix = match primaries {
		9 => [
			[1.660491, -0.587641, -0.072850],
			[-0.124550, 1.1329, -0.008349],
			[-0.018151, -0.100579, 1.11873],
		],
		12 => [
			[1.22494, -0.224940, 0.0],
			[-0.042057, 1.042057, 0.0],
			[-0.019638, -0.078636, 1.098274],
		],
		_ => [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
	};
	rgb = matrix.map(|row| row.iter().zip(rgb).map(|(a, b)| a * b).sum::<f32>() / white);
	let l = (rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722).max(0.0);
	if peak / white > 1.0 && l > 0.8 {
		static KNEE: std::sync::OnceLock<[f32; 4097]> = std::sync::OnceLock::new();
		let table =
			KNEE.get_or_init(|| std::array::from_fn(|i| 1.0 - (-16.0 * i as f32 / 4096.0).exp()));
		let mapped = 0.8 + 0.2 * lookup(table, (l - 0.8) / 3.2);
		rgb = rgb.map(|v| v * mapped / l.max(0.000001));
	}
	let l = (rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722).clamp(0.0, 1.0);
	let lo = rgb.iter().copied().fold(f32::INFINITY, f32::min);
	let hi = rgb.iter().copied().fold(f32::NEG_INFINITY, f32::max);
	let mut saturation = 1.0f32;
	if lo < 0.0 {
		saturation = saturation.min(l / (l - lo).max(0.000001));
	}
	if hi > 1.0 {
		saturation = saturation.min((1.0 - l) / (hi - l).max(0.000001));
	}
	rgb.map(|v| {
		let v = (l + (v - l) * saturation).clamp(0.0, 1.0);
		static GAMMA: std::sync::OnceLock<[f32; 4097]> = std::sync::OnceLock::new();
		let table = GAMMA.get_or_init(|| {
			std::array::from_fn(|i| {
				let v = i as f32 / 4096.0;
				if v <= 0.0031308 {
					v * 12.92
				} else {
					1.055 * v.powf(1.0 / 2.4) - 0.055
				}
			})
		});
		let gamma = lookup(table, v);
		(gamma * 255.0).round().clamp(0.0, 255.0) as u8
	})
}
fn lookup(table: &[f32; 4097], value: f32) -> f32 {
	let position = value.clamp(0.0, 1.0) * 4096.0;
	let at = (position as usize).min(4095);
	table[at] + (table[at + 1] - table[at]) * (position - at as f32)
}
fn pq_reference(v: f32) -> f32 {
	let p = v.clamp(0.0, 1.0).powf(32.0 / 2523.0);
	10000.0
		* ((p - 3424.0 / 4096.0).max(0.0) / (2413.0 / 128.0 - 2392.0 / 128.0 * p).max(0.000001))
			.powf(16384.0 / 2610.0)
}
pub fn pq_nits(v: f32) -> f32 {
	static PQ: std::sync::OnceLock<[f32; 4097]> = std::sync::OnceLock::new();
	lookup(
		PQ.get_or_init(|| std::array::from_fn(|i| pq_reference(i as f32 / 4096.0))),
		v,
	)
}
fn hlg_scene(v: f32) -> f32 {
	static HLG: std::sync::OnceLock<[f32; 4097]> = std::sync::OnceLock::new();
	let table = HLG.get_or_init(|| {
		std::array::from_fn(|i| {
			let v = i as f32 / 4096.0;
			if v <= 0.5 {
				v * v / 3.0
			} else {
				((v - 0.559_910_7) / 0.17883277).exp() / 12.0 + 0.28466892 / 12.0
			}
		})
	});
	lookup(table, v)
}
pub fn hlg_nits(rgb: [f32; 3], peak: f32) -> [f32; 3] {
	let scene = rgb.map(hlg_scene);
	let gamma = 1.2 + 0.42 * (peak / 1000.0).log10();
	let l = (scene[0] * 0.2627 + scene[1] * 0.6780 + scene[2] * 0.0593).max(0.0000001);
	scene.map(|v| v * l.powf(gamma - 1.0) * peak)
}

/// One capture conversion shares its transfer tables across bounded row bands.
/// This avoids transcendental functions per pixel at high frame rates.
pub struct SdrConverter {
	primaries: u32,
	peak: f32,
	white: f32,
	hlg_ootf: Option<Box<[f32; 4097]>>,
}
impl SdrConverter {
	pub fn new(primaries: u32, peak: f32, white: f32, transfer: Option<HdrTransfer>) -> Self {
		let hlg_ootf = (transfer == Some(HdrTransfer::Hlg)).then(|| {
			let gamma = 1.2 + 0.42 * (peak / 1000.0).log10();
			Box::new(std::array::from_fn(|i| {
				(i as f32 / 4096.0).max(0.0000001).powf(gamma - 1.0)
			}))
		});
		Self {
			primaries,
			peak,
			white,
			hlg_ootf,
		}
	}
	pub fn source_nits(&self, rgb: [f32; 3], transfer: HdrTransfer) -> [f32; 3] {
		match transfer {
			HdrTransfer::Pq => rgb.map(pq_nits),
			HdrTransfer::Hlg => {
				let scene = rgb.map(hlg_scene);
				let l = scene[0] * 0.2627 + scene[1] * 0.6780 + scene[2] * 0.0593;
				self.hlg_ootf.as_ref().map_or_else(
					|| hlg_nits(rgb, self.peak),
					|table| {
						let scale = lookup(table, l) * self.peak;
						scene.map(|v| v * scale)
					},
				)
			}
		}
	}
	pub fn to_sdr(&self, nits: [f32; 3]) -> [u8; 3] {
		sdr_rgb(nits, self.primaries, self.peak, self.white)
	}
}
/// Read a finite little-endian IEEE half; non-finite capture samples become black.
pub fn half(bytes: [u8; 2]) -> f32 {
	let h = u16::from_le_bytes(bytes);
	let sign = u32::from(h & 0x8000) << 16;
	let exponent = (h >> 10) & 31;
	let mantissa = u32::from(h & 1023);
	let value = if exponent == 0 {
		(mantissa as f32) * 2.0f32.powi(-24) * if sign == 0 { 1.0 } else { -1.0 }
	} else if exponent == 31 {
		0.0
	} else {
		f32::from_bits(sign | ((u32::from(exponent) + 112) << 23) | (mantissa << 13))
	};
	value.clamp(-125.0, 125.0)
}
/// Active source-display luminance, queried without starting capture.
#[cfg(any(target_os = "windows", target_os = "macos"))]
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct CaptureLuminance {
	pub unit_nits: u32,
	pub white_nits: u32,
	pub peak_nits: u32,
}
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn capture_luminance(source: u64, window: bool) -> Option<CaptureLuminance> {
	unsafe extern "C" {
		fn serein_capture_luminance(source: u64, window: i32, output: *mut CaptureLuminance)
		-> i32;
	}
	let mut output = CaptureLuminance::default();
	// SAFETY: The native function treats the handle as a query target and writes
	// only the three fixed-width values in this initialized, live output.
	let ok = unsafe { serein_capture_luminance(source, i32::from(window), &mut output) };
	(ok == 1
		&& (1..=1000).contains(&output.unit_nits)
		&& (80..=1000).contains(&output.white_nits)
		&& (output.white_nits..=10000).contains(&output.peak_nits))
	.then_some(output)
}

// Offline 32x16/60 PQ or HLG patches with a 48 kHz sine AAC track.
#[cfg(test)]
const FILE_HEVC_PQ: &[u8] = &[
	120, 218, 237, 87, 107, 116, 20, 69, 22, 238, 78, 64, 72, 120, 138, 122, 34, 8, 75, 131, 67,
	128, 53, 157, 76, 79, 30, 64, 100, 216, 144, 160, 70, 76, 144, 85, 64, 33, 188, 122, 186, 107,
	102, 58, 233, 87, 186, 122, 38, 153, 108, 18, 162, 199, 35, 38, 187, 46, 190, 246, 24, 96, 35,
	46, 146, 21, 137, 162, 162, 66, 118, 17, 241, 137, 30, 65, 217, 96, 224, 168, 184, 136, 46,
	112, 112, 1, 143, 139, 160, 70, 99, 102, 111, 117, 247, 76, 18, 21, 117, 255, 186, 211, 147,
	78, 85, 221, 186, 85, 117, 235, 222, 175, 110, 127, 69, 81, 212, 88, 191, 25, 209, 37, 172, 41,
	20, 149, 68, 145, 18, 94, 143, 162, 231, 112, 20, 53, 252, 14, 69, 211, 194, 20, 69, 201, 74,
	56, 40, 82, 253, 158, 228, 147, 240, 207, 67, 209, 20, 249, 235, 125, 232, 254, 90, 223, 109,
	23, 80, 63, 250, 36, 83, 212, 144, 37, 166, 193, 87, 64, 125, 169, 89, 97, 173, 153, 252, 3,
	179, 121, 190, 55, 242, 231, 172, 203, 192, 59, 210, 106, 186, 144, 104, 98, 40, 199, 34, 25,
	155, 125, 70, 120, 122, 199, 166, 30, 80, 68, 137, 39, 163, 20, 241, 187, 123, 159, 105, 153,
	181, 240, 85, 171, 193, 6, 69, 217, 136, 245, 132, 37, 17, 245, 213, 92, 4, 109, 173, 152, 87,
	69, 25, 17, 157, 212, 58, 69, 82, 253, 80, 185, 52, 172, 88, 147, 246, 53, 211, 37, 218, 125,
	99, 69, 3, 249, 251, 24, 53, 52, 100, 200, 140, 93, 79, 245, 98, 211, 39, 83, 84, 74, 7, 54,
	177, 216, 171, 147, 242, 82, 16, 133, 185, 11, 184, 130, 108, 124, 36, 85, 12, 101, 113, 76,
	227, 178, 18, 62, 44, 228, 113, 153, 220, 140, 76, 206, 205, 49, 178, 228, 171, 246, 228, 229,
	246, 25, 113, 121, 52, 10, 179, 150, 6, 195, 66, 17, 61, 96, 48, 8, 90, 54, 19, 113, 244, 51,
	170, 251, 219, 175, 191, 166, 168, 17, 3, 24, 88, 39, 173, 128, 30, 74, 71, 163, 68, 33, 217,
	210, 128, 88, 69, 255, 158, 84, 48, 1, 250, 166, 22, 210, 116, 255, 142, 135, 11, 47, 206, 43,
	219, 242, 252, 214, 242, 81, 204, 40, 171, 131, 252, 163, 143, 22, 76, 4, 237, 139, 230, 208,
	47, 85, 54, 142, 154, 76, 209, 131, 59, 231, 209, 3, 163, 206, 179, 63, 99, 227, 145, 148, 231,
	210, 174, 59, 188, 115, 225, 166, 158, 85, 47, 119, 207, 35, 102, 50, 83, 124, 33, 73, 22, 25,
	15, 151, 59, 149, 97, 153, 156, 76, 238, 42, 142, 229, 68, 142, 155, 230, 67, 249, 101, 37,
	146, 26, 170, 94, 86, 118, 93, 81, 17, 195, 229, 100, 122, 50, 221, 203, 202, 242, 114, 24,
	159, 100, 46, 99, 56, 55, 20, 48, 162, 56, 19, 102, 201, 42, 190, 102, 81, 17, 35, 104, 34, 18,
	64, 84, 164, 233, 17, 67, 10, 4, 77, 198, 227, 230, 178, 89, 248, 55, 157, 153, 34, 76, 101,
	74, 67, 178, 41, 9, 154, 129, 170, 120, 3, 101, 48, 215, 171, 68, 57, 104, 154, 122, 126, 86,
	22, 177, 37, 83, 51, 2, 32, 209, 116, 83, 210, 84, 156, 207, 8, 122, 72, 18, 189, 28, 60, 238,
	236, 25, 140, 223, 224, 21, 196, 154, 65, 3, 241, 34, 246, 114, 140, 26, 82, 120, 86, 215, 52,
	25, 123, 85, 77, 69, 140, 170, 177, 85, 186, 78, 10, 93, 1, 67, 236, 138, 93, 96, 213, 32, 37,
	198, 146, 194, 200, 90, 128, 149, 81, 24, 201, 94, 15, 217, 136, 136, 116, 51, 232, 229, 220,
	140, 164, 234, 33, 147, 21, 176, 14, 83, 251, 117, 236, 205, 115, 103, 113, 142, 208, 64, 216,
	155, 237, 169, 230, 242, 160, 109, 34, 67, 230, 5, 228, 117, 51, 166, 102, 242, 50, 107, 89,
	133, 161, 105, 205, 201, 74, 162, 0, 245, 32, 108, 158, 53, 37, 100, 192, 92, 161, 160, 200,
	250, 68, 16, 2, 14, 137, 213, 26, 203, 203, 178, 86, 197, 130, 205, 172, 160, 169, 126, 205,
	80, 120, 85, 64, 208, 173, 35, 222, 100, 131, 176, 59, 100, 96, 134, 87, 85, 84, 237, 179, 212,
	67, 34, 41, 144, 230, 179, 11, 76, 138, 160, 33, 130, 49, 126, 141, 9, 242, 56, 72, 140, 65,
	138, 174, 25, 96, 143, 204, 71, 96, 56, 72, 64, 73, 211, 145, 202, 6, 52, 157, 129, 115, 194,
	86, 160, 8, 88, 15, 22, 196, 43, 208, 195, 202, 154, 86, 193, 147, 53, 97, 132, 47, 190, 23,
	31, 203, 139, 188, 110, 218, 179, 248, 88, 136, 38, 175, 72, 162, 163, 192, 250, 36, 158, 40,
	25, 66, 191, 209, 241, 58, 139, 101, 73, 176, 166, 193, 2, 82, 145, 16, 114, 230, 9, 74, 216,
	100, 99, 34, 198, 224, 69, 217, 150, 99, 157, 232, 147, 26, 152, 101, 240, 224, 109, 63, 56,
	60, 200, 8, 102, 200, 11, 46, 39, 182, 11, 33, 22, 75, 53, 200, 59, 157, 104, 25, 72, 48, 45,
	191, 40, 176, 49, 190, 154, 53, 157, 78, 208, 133, 170, 21, 33, 214, 137, 170, 35, 128, 73, 99,
	2, 89, 82, 36, 19, 134, 16, 251, 69, 173, 210, 65, 130, 155, 17, 35, 42, 108, 81, 96, 13, 216,
	74, 166, 219, 29, 67, 11, 180, 25, 44, 5, 212, 160, 100, 3, 202, 196, 21, 18, 64, 204, 176,
	103, 37, 246, 27, 246, 138, 246, 86, 32, 158, 24, 228, 146, 138, 68, 91, 131, 129, 166, 166, 6,
	28, 35, 48, 124, 0, 204, 160, 164, 6, 44, 187, 21, 100, 4, 144, 55, 219, 49, 9, 118, 141, 109,
	120, 216, 109, 2, 97, 204, 40, 176, 45, 6, 135, 124, 80, 122, 160, 97, 240, 42, 12, 201, 157,
	214, 27, 110, 37, 108, 33, 222, 14, 140, 24, 178, 26, 65, 27, 245, 85, 136, 28, 65, 189, 183,
	106, 163, 73, 229, 229, 72, 13, 98, 49, 132, 79, 151, 4, 204, 136, 200, 39, 107, 66, 133, 215,
	157, 15, 1, 227, 53, 107, 231, 188, 102, 193, 211, 233, 2, 79, 129, 153, 24, 201, 224, 120, 41,
	140, 72, 183, 55, 135, 65, 188, 33, 71, 88, 203, 31, 134, 237, 21, 48, 131, 135, 16, 219, 27,
	143, 57, 203, 146, 89, 190, 9, 1, 92, 48, 150, 17, 198, 128, 175, 184, 18, 137, 190, 105, 136,
	206, 68, 34, 32, 150, 151, 205, 8, 184, 83, 199, 17, 18, 13, 15, 137, 134, 93, 215, 42, 227,
	177, 129, 1, 224, 48, 240, 51, 19, 159, 83, 240, 85, 234, 154, 223, 79, 112, 39, 24, 241, 170,
	33, 120, 133, 74, 157, 169, 212, 193, 98, 73, 55, 120, 200, 44, 94, 46, 51, 199, 205, 240, 149,
	150, 139, 189, 86, 13, 162, 132, 212, 0, 0, 36, 54, 63, 32, 212, 64, 136, 169, 129, 188, 2, 65,
	13, 169, 14, 132, 65, 79, 18, 32, 73, 248, 12, 166, 50, 16, 71, 29, 177, 71, 96, 3, 36, 238,
	176, 16, 132, 214, 155, 55, 131, 84, 36, 181, 15, 42, 216, 176, 47, 12, 254, 37, 64, 209, 194,
	112, 66, 5, 158, 244, 146, 47, 156, 102, 165, 0, 211, 155, 203, 216, 225, 5, 251, 53, 89, 51,
	116, 67, 82, 188, 51, 24, 112, 147, 138, 253, 36, 147, 228, 217, 114, 80, 53, 164, 106, 232,
	17, 130, 134, 166, 240, 16, 32, 2, 95, 9, 252, 200, 71, 216, 42, 73, 21, 181, 42, 50, 133, 12,
	168, 206, 112, 91, 167, 71, 134, 68, 9, 34, 130, 57, 171, 202, 185, 61, 217, 36, 13, 122, 88,
	34, 210, 53, 56, 199, 216, 7, 103, 43, 28, 146, 32, 105, 193, 8, 130, 87, 72, 45, 164, 13, 121,
	198, 110, 56, 135, 154, 179, 243, 138, 201, 86, 234, 172, 174, 227, 88, 11, 130, 1, 184, 133,
	93, 202, 150, 31, 99, 93, 10, 73, 248, 172, 206, 99, 108, 107, 129, 52, 150, 0, 156, 44, 146,
	233, 206, 141, 205, 1, 0, 17, 145, 108, 242, 48, 181, 5, 84, 18, 32, 242, 33, 176, 32, 45, 26,
	156, 59, 94, 33, 234, 164, 33, 246, 107, 73, 162, 65, 114, 3, 241, 46, 224, 18, 73, 140, 5,
	117, 44, 97, 144, 134, 48, 138, 31, 245, 184, 24, 243, 0, 230, 11, 244, 201, 26, 47, 126, 167,
	15, 66, 38, 35, 128, 179, 96, 106, 134, 157, 206, 1, 127, 241, 68, 208, 219, 68, 125, 122, 149,
	48, 120, 204, 169, 67, 62, 99, 33, 78, 48, 154, 236, 202, 134, 134, 125, 214, 201, 177, 19,
	172, 84, 229, 215, 28, 185, 86, 101, 121, 77, 132, 84, 231, 12, 143, 155, 6, 4, 211, 6, 137,
	30, 33, 39, 24, 22, 32, 97, 228, 67, 4, 142, 49, 243, 184, 76, 107, 154, 120, 86, 179, 207, 10,
	65, 48, 68, 23, 118, 65, 220, 67, 188, 137, 194, 2, 56, 218, 234, 8, 91, 78, 244, 75, 8, 200,
	0, 4, 215, 250, 0, 144, 227, 162, 178, 54, 44, 57, 114, 52, 226, 209, 227, 201, 39, 28, 34,
	229, 117, 247, 249, 134, 57, 240, 99, 201, 217, 67, 38, 102, 44, 14, 64, 62, 39, 154, 105, 106,
	10, 129, 41, 34, 20, 193, 176, 112, 7, 103, 17, 217, 155, 133, 147, 1, 142, 128, 88, 244, 194,
	197, 2, 143, 128, 77, 191, 101, 154, 207, 16, 122, 147, 156, 33, 52, 16, 238, 71, 12, 181, 184,
	217, 165, 62, 211, 176, 89, 231, 168, 119, 87, 198, 8, 23, 54, 45, 70, 26, 35, 112, 201, 14,
	143, 27, 11, 68, 79, 232, 35, 239, 251, 94, 10, 125, 53, 118, 87, 234, 48, 135, 41, 131, 190,
	160, 245, 178, 233, 17, 58, 69, 141, 9, 82, 212, 164, 50, 96, 247, 11, 127, 132, 89, 39, 253,
	0, 179, 166, 233, 255, 157, 209, 255, 8, 179, 30, 224, 140, 165, 223, 186, 48, 179, 222, 9,
	158, 26, 182, 242, 135, 152, 53, 134, 156, 214, 87, 243, 102, 104, 139, 189, 204, 154, 174,
	118, 152, 245, 72, 172, 244, 155, 244, 103, 176, 106, 122, 166, 205, 170, 169, 250, 254, 172,
	154, 82, 225, 26, 196, 127, 111, 219, 180, 125, 137, 32, 166, 194, 147, 135, 176, 104, 5, 46,
	185, 161, 161, 97, 18, 248, 113, 0, 148, 105, 5, 151, 89, 154, 19, 147, 200, 59, 16, 36, 3, 47,
	190, 107, 209, 113, 234, 34, 168, 209, 73, 253, 32, 224, 232, 16, 119, 244, 129, 64, 146, 29,
	158, 1, 206, 202, 244, 202, 159, 128, 130, 171, 23, 10, 214, 67, 6, 190, 8, 239, 41, 120, 79,
	192, 59, 208, 158, 63, 14, 13, 232, 31, 254, 20, 69, 141, 174, 131, 161, 33, 138, 202, 2, 136,
	80, 99, 112, 64, 23, 201, 100, 134, 38, 203, 142, 13, 52, 185, 7, 192, 202, 190, 128, 78, 245,
	246, 208, 206, 10, 164, 4, 174, 103, 18, 31, 45, 86, 144, 25, 243, 213, 132, 190, 129, 131, 96,
	27, 188, 174, 203, 125, 131, 151, 33, 217, 232, 112, 61, 102, 106, 154, 21, 28, 222, 26, 108,
	57, 25, 238, 38, 126, 184, 155, 76, 131, 171, 73, 54, 52, 7, 3, 183, 130, 59, 21, 243, 154, 2,
	74, 71, 146, 168, 126, 87, 23, 42, 41, 184, 101, 241, 39, 159, 149, 14, 25, 178, 100, 237, 170,
	29, 31, 62, 241, 239, 238, 35, 59, 140, 3, 109, 11, 93, 85, 11, 58, 239, 115, 13, 78, 255, 228,
	206, 19, 175, 73, 167, 2, 219, 182, 222, 147, 191, 243, 246, 197, 233, 254, 143, 232, 253, 203,
	115, 111, 184, 126, 99, 251, 254, 154, 172, 229, 31, 135, 182, 190, 211, 209, 62, 110, 231,
	254, 223, 165, 109, 58, 219, 118, 247, 186, 103, 188, 141, 127, 91, 172, 46, 42, 123, 224, 76,
	197, 21, 77, 237, 239, 109, 205, 43, 223, 119, 238, 76, 249, 222, 242, 150, 39, 230, 29, 218,
	80, 54, 107, 123, 157, 220, 122, 211, 159, 246, 220, 146, 91, 95, 222, 221, 250, 204, 169, 224,
	19, 215, 148, 203, 126, 33, 123, 244, 218, 142, 37, 77, 157, 139, 54, 190, 189, 126, 116, 243,
	190, 61, 226, 102, 16, 95, 245, 219, 185, 233, 217, 220, 166, 117, 115, 127, 213, 152, 126,
	239, 150, 242, 117, 215, 111, 218, 104, 87, 9, 130, 47, 116, 105, 130, 174, 159, 127, 103, 2,
	101, 231, 202, 4, 14, 74, 220, 152, 18, 55, 166, 196, 141, 41, 113, 99, 74, 220, 152, 18, 55,
	166, 196, 141, 41, 113, 99, 250, 165, 220, 152, 62, 156, 66, 63, 57, 176, 221, 213, 28, 77,
	158, 22, 141, 30, 61, 88, 155, 182, 62, 233, 64, 253, 130, 175, 50, 156, 223, 154, 234, 126,
	191, 19, 193, 119, 79, 142, 28, 180, 234, 142, 29, 159, 173, 238, 30, 245, 237, 215, 117, 31,
	56, 191, 35, 203, 250, 253, 150, 86, 254, 51, 233, 81, 138, 186, 247, 244, 177, 47, 86, 230,
	165, 93, 149, 122, 229, 224, 63, 140, 63, 121, 215, 203, 87, 174, 59, 51, 186, 187, 124, 198,
	214, 142, 13, 123, 118, 94, 251, 72, 101, 3, 117, 227, 249, 213, 95, 21, 125, 115, 71, 228,
	193, 183, 70, 212, 165, 110, 61, 94, 215, 90, 63, 103, 122, 197, 91, 43, 234, 150, 159, 216,
	213, 90, 219, 89, 60, 158, 161, 210, 198, 60, 60, 172, 116, 194, 236, 63, 46, 216, 119, 236,
	237, 146, 99, 111, 151, 206, 94, 112, 153, 243, 119, 243, 39, 151, 172, 164, 198, 174, 159,
	126, 77, 243, 221, 77, 249, 135, 167, 110, 252, 107, 211, 186, 95, 7, 31, 219, 125, 237, 183,
	87, 156, 211, 58, 236, 98, 173, 81, 243, 56, 157, 211, 252, 254, 123, 222, 67, 243, 223, 41,
	73, 127, 115, 140, 124, 195, 226, 166, 181, 103, 206, 215, 108, 63, 212, 122, 56, 250, 202,
	154, 207, 79, 167, 181, 237, 142, 62, 253, 250, 55, 231, 210, 218, 54, 68, 181, 127, 157, 255,
	120, 200, 174, 91, 223, 111, 89, 247, 84, 62, 125, 238, 249, 207, 167, 156, 202, 248, 242, 171,
	179, 99, 205, 75, 14, 78, 155, 185, 103, 109, 121, 37, 119, 243, 115, 143, 118, 126, 186, 62,
	119, 65, 235, 225, 39, 187, 126, 51, 186, 107, 94, 227, 174, 106, 151, 203, 149, 254, 5, 63,
	97, 194, 228, 113, 47, 23, 22, 206, 43, 217, 81, 149, 146, 146, 50, 125, 123, 203, 7, 46, 215,
	230, 146, 247, 174, 222, 125, 246, 63, 221, 238, 211, 15, 204, 190, 246, 100, 81, 203, 169,
	143, 50, 90, 134, 117, 62, 221, 168, 72, 119, 54, 172, 111, 124, 168, 181, 254, 246, 185, 7,
	183, 221, 119, 103, 218, 80, 93, 74, 15, 7, 166, 117, 213, 190, 176, 98, 242, 67, 93, 181, 237,
	247, 15, 218, 123, 252, 108, 211, 68, 10, 218, 153, 220, 28, 170, 171, 118, 219, 171, 5, 244,
	204, 185, 243, 47, 158, 67, 105, 203, 34, 245, 237, 15, 157, 74, 11, 253, 126, 225, 77, 239,
	176, 158, 81, 177, 71, 119, 185, 142, 150, 118, 76, 173, 78, 240, 222, 4, 239, 77, 240, 222, 4,
	239, 77, 240, 222, 4, 239, 77, 240, 222, 4, 239, 77, 240, 222, 255, 103, 222, 59, 115, 237,
	182, 251, 11, 15, 249, 38, 94, 17, 30, 195, 46, 242, 125, 90, 215, 243, 101, 110, 250, 115, 85,
	209, 47, 235, 163, 203, 139, 110, 105, 139, 126, 179, 97, 213, 161, 23, 181, 253, 171, 206,
	244, 116, 103, 85, 229, 50, 227, 79, 239, 26, 122, 126, 132, 216, 209, 62, 110, 208, 129, 246,
	131, 74, 79, 137, 127, 204, 225, 148, 230, 165, 199, 252, 102, 154, 111, 198, 232, 247, 155,
	38, 223, 178, 247, 120, 193, 252, 174, 218, 205, 183, 206, 31, 159, 245, 224, 155, 3, 27, 59,
	103, 173, 200, 186, 135, 58, 90, 251, 194, 210, 194, 87, 134, 15, 159, 181, 244, 131, 200, 159,
	31, 152, 18, 126, 86, 184, 33, 178, 161, 185, 231, 156, 247, 233, 153, 55, 190, 121, 221, 178,
	188, 237, 185, 7, 199, 29, 154, 58, 201, 117, 247, 130, 127, 72, 161, 226, 27, 187, 91, 46,
	201, 248, 203, 164, 194, 194, 82, 220, 179, 226, 141, 218, 244, 158, 21, 123, 187, 82, 198,
	239, 237, 170, 217, 243, 194, 154, 227, 221, 219, 103, 47, 222, 159, 58, 107, 247, 248, 45,
	198, 160, 215, 253, 123, 242, 247, 93, 253, 200, 93, 55, 117, 213, 211, 143, 63, 184, 230, 118,
	49, 251, 40, 175, 180, 73, 25, 115, 158, 25, 214, 211, 182, 186, 121, 201, 173, 143, 171, 155,
	238, 75, 121, 99, 117, 67, 130, 242, 38, 40, 111, 130, 242, 38, 40, 111, 130, 242, 38, 40, 111,
	130, 242, 38, 40, 111, 130, 242, 254, 20, 229, 221, 252, 11, 166, 188, 151, 223, 246, 172, 254,
	95, 250, 235, 4, 155,
];

// Offline 32x16/60 PQ or HLG patches with a 48 kHz sine AAC track.
#[cfg(test)]
const FILE_HEVC_HLG: &[u8] = &[
	120, 218, 237, 87, 125, 116, 20, 213, 21, 159, 73, 64, 8, 32, 32, 234, 137, 68, 40, 3, 93, 2,
	28, 51, 201, 206, 230, 131, 15, 179, 20, 19, 84, 68, 64, 138, 16, 190, 193, 217, 153, 183, 187,
	19, 102, 230, 77, 230, 205, 110, 178, 52, 132, 212, 211, 35, 130, 199, 226, 231, 49, 64, 3, 45,
	133, 35, 37, 138, 136, 10, 105, 17, 252, 66, 180, 162, 69, 16, 114, 84, 44, 5, 91, 68, 5, 44,
	69, 20, 34, 49, 219, 251, 102, 102, 55, 27, 21, 181, 255, 218, 157, 205, 228, 189, 119, 223,
	125, 239, 221, 119, 239, 239, 221, 249, 61, 134, 97, 6, 4, 173, 152, 161, 16, 172, 49, 76, 6,
	67, 75, 120, 125, 154, 81, 36, 48, 76, 239, 223, 104, 24, 71, 25, 134, 81, 181, 104, 88, 102,
	58, 61, 153, 31, 195, 63, 31, 195, 50, 244, 175, 227, 97, 59, 107, 125, 179, 61, 142, 249, 222,
	39, 147, 97, 122, 206, 177, 76, 113, 17, 212, 231, 89, 139, 236, 53, 51, 191, 99, 54, 223, 183,
	70, 254, 152, 117, 57, 120, 251, 218, 77, 15, 146, 45, 2, 229, 0, 164, 18, 43, 101, 132, 175,
	99, 108, 143, 131, 154, 172, 136, 116, 148, 38, 127, 115, 239, 165, 182, 89, 51, 246, 216, 13,
	62, 44, 171, 102, 162, 39, 170, 200, 40, 85, 179, 2, 218, 120, 130, 168, 203, 42, 162, 58, 61,
	150, 104, 138, 30, 132, 202, 53, 81, 205, 158, 52, 213, 76, 143, 236, 244, 13, 144, 77, 20, 76,
	49, 170, 87, 196, 84, 57, 167, 222, 195, 79, 172, 128, 202, 48, 89, 7, 136, 69, 228, 14, 157,
	172, 151, 194, 40, 42, 92, 198, 21, 116, 227, 125, 153, 9, 80, 78, 72, 104, 92, 59, 73, 140,
	74, 37, 66, 190, 48, 58, 95, 240, 10, 156, 170, 4, 106, 124, 37, 197, 41, 35, 174, 139, 199,
	97, 214, 201, 225, 168, 84, 206, 118, 233, 14, 130, 198, 77, 84, 28, 63, 203, 180, 125, 253,
	213, 87, 12, 211, 167, 11, 7, 235, 100, 143, 99, 123, 177, 241, 56, 85, 200, 180, 53, 32, 86,
	241, 191, 100, 140, 27, 12, 125, 35, 202, 88, 182, 115, 199, 239, 203, 174, 42, 153, 187, 249,
	249, 45, 149, 253, 60, 253, 236, 14, 250, 143, 61, 54, 110, 8, 104, 95, 49, 158, 125, 169, 106,
	121, 191, 97, 12, 219, 253, 208, 20, 182, 107, 220, 125, 246, 231, 173, 63, 154, 245, 92, 246,
	173, 71, 118, 206, 216, 208, 190, 244, 229, 182, 41, 212, 76, 110, 120, 32, 162, 168, 50, 231,
	19, 138, 71, 112, 60, 87, 148, 47, 220, 32, 240, 130, 44, 8, 35, 3, 104, 204, 220, 73, 138, 30,
	169, 153, 63, 247, 214, 242, 114, 78, 40, 202, 247, 229, 123, 231, 207, 45, 41, 226, 2, 138,
	53, 159, 19, 188, 80, 192, 136, 9, 249, 48, 75, 193, 132, 155, 43, 202, 57, 9, 203, 72, 2, 81,
	57, 54, 98, 166, 18, 10, 91, 156, 207, 43, 20, 242, 240, 111, 20, 55, 92, 26, 193, 77, 142,
	168, 150, 34, 97, 19, 85, 139, 38, 202, 227, 110, 211, 169, 114, 216, 178, 140, 49, 5, 5, 212,
	150, 124, 108, 134, 64, 130, 13, 75, 193, 58, 25, 195, 73, 70, 68, 145, 253, 2, 60, 222, 194,
	209, 92, 208, 20, 53, 196, 91, 97, 19, 137, 50, 241, 11, 156, 30, 209, 68, 222, 192, 88, 37,
	126, 29, 235, 136, 211, 49, 95, 109, 24, 180, 48, 52, 48, 196, 169, 56, 5, 209, 77, 90, 18,
	162, 104, 156, 138, 67, 188, 138, 162, 72, 245, 251, 232, 70, 100, 100, 88, 97, 191, 224, 229,
	20, 221, 136, 88, 188, 68, 12, 152, 58, 104, 16, 127, 137, 183, 64, 112, 133, 38, 34, 254, 66,
	95, 141, 80, 2, 109, 11, 153, 170, 40, 33, 191, 151, 179, 176, 37, 170, 188, 109, 21, 129, 166,
	61, 39, 175, 200, 18, 212, 195, 176, 121, 222, 82, 144, 9, 115, 69, 194, 50, 31, 144, 65, 8,
	56, 164, 86, 99, 94, 84, 85, 92, 205, 131, 205, 188, 132, 245, 32, 54, 53, 81, 151, 16, 116,
	27, 72, 180, 248, 48, 236, 14, 153, 132, 19, 117, 29, 213, 4, 108, 245, 136, 76, 11, 132, 3,
	78, 65, 104, 17, 54, 101, 48, 38, 136, 185, 176, 72, 194, 212, 24, 164, 25, 216, 4, 123, 84,
	49, 6, 195, 65, 2, 74, 216, 64, 58, 31, 194, 6, 7, 231, 132, 95, 132, 98, 96, 61, 88, 144, 172,
	64, 15, 175, 98, 188, 72, 164, 107, 194, 136, 64, 114, 47, 1, 94, 148, 69, 195, 114, 102, 9,
	240, 16, 77, 81, 83, 100, 87, 129, 15, 40, 34, 85, 50, 165, 78, 163, 147, 117, 158, 168, 138,
	100, 79, 67, 36, 164, 35, 41, 226, 206, 19, 86, 136, 197, 39, 68, 156, 41, 202, 170, 35, 39, 6,
	213, 167, 53, 48, 203, 20, 193, 219, 65, 112, 120, 152, 147, 172, 136, 31, 92, 78, 109, 151,
	34, 60, 81, 22, 35, 255, 40, 170, 101, 34, 201, 178, 253, 162, 193, 198, 196, 26, 222, 114, 59,
	65, 23, 170, 118, 132, 120, 55, 170, 174, 0, 38, 77, 8, 84, 69, 83, 44, 24, 66, 237, 151, 113,
	149, 139, 4, 47, 39, 199, 116, 216, 162, 196, 155, 176, 149, 124, 175, 55, 129, 22, 104, 115,
	68, 9, 233, 97, 197, 1, 148, 69, 22, 41, 0, 49, 211, 153, 149, 218, 111, 58, 43, 58, 91, 129,
	120, 18, 144, 43, 58, 146, 29, 13, 14, 154, 88, 15, 185, 70, 16, 248, 0, 88, 97, 69, 15, 217,
	118, 107, 200, 12, 33, 127, 161, 107, 18, 236, 154, 56, 240, 112, 218, 20, 194, 132, 211, 96,
	91, 28, 137, 4, 160, 244, 65, 195, 20, 117, 24, 82, 60, 178, 35, 220, 90, 212, 70, 188, 19, 24,
	57, 98, 55, 194, 14, 234, 171, 17, 61, 130, 70, 71, 213, 65, 147, 46, 170, 177, 197, 136, 39,
	16, 62, 67, 145, 8, 39, 163, 128, 138, 165, 69, 126, 239, 24, 8, 152, 136, 237, 157, 139, 216,
	134, 167, 219, 5, 158, 2, 51, 9, 82, 193, 241, 74, 20, 209, 110, 127, 17, 135, 68, 83, 141,
	241, 182, 63, 76, 199, 43, 96, 134, 8, 33, 118, 54, 158, 112, 150, 45, 179, 125, 19, 1, 184,
	16, 162, 34, 66, 0, 95, 73, 37, 26, 125, 203, 148, 221, 137, 100, 64, 172, 168, 90, 49, 112,
	167, 65, 98, 52, 26, 62, 26, 13, 167, 142, 171, 146, 177, 129, 1, 224, 48, 240, 51, 151, 156,
	83, 10, 84, 25, 56, 24, 164, 184, 147, 204, 100, 213, 148, 252, 82, 149, 193, 85, 25, 96, 177,
	98, 152, 34, 100, 22, 191, 144, 95, 228, 229, 196, 42, 219, 197, 126, 187, 6, 81, 66, 122, 8,
	0, 146, 152, 31, 16, 106, 34, 196, 45, 134, 188, 2, 65, 141, 232, 46, 132, 65, 79, 145, 32, 73,
	4, 76, 174, 42, 148, 68, 29, 181, 71, 226, 67, 52, 238, 176, 16, 132, 214, 95, 50, 154, 86, 20,
	61, 5, 21, 124, 52, 16, 5, 255, 82, 160, 224, 40, 156, 80, 73, 164, 189, 244, 11, 135, 237, 20,
	96, 249, 139, 57, 39, 188, 96, 63, 86, 177, 105, 152, 138, 230, 31, 205, 129, 155, 116, 18,
	164, 153, 100, 148, 35, 7, 85, 83, 169, 129, 30, 41, 108, 98, 77, 132, 0, 81, 248, 42, 224, 71,
	49, 198, 87, 43, 186, 140, 171, 233, 20, 42, 160, 58, 207, 107, 159, 30, 21, 18, 37, 136, 40,
	230, 236, 170, 224, 245, 21, 210, 52, 232, 227, 169, 200, 192, 112, 142, 73, 0, 206, 86, 52,
	162, 64, 210, 130, 17, 20, 175, 144, 90, 104, 27, 242, 140, 211, 112, 15, 181, 224, 228, 21,
	139, 175, 50, 120, 195, 32, 137, 22, 4, 3, 112, 11, 187, 84, 109, 63, 38, 186, 52, 154, 240,
	121, 67, 36, 196, 209, 2, 105, 34, 1, 184, 89, 36, 223, 91, 156, 152, 3, 0, 34, 35, 213, 18,
	97, 106, 27, 168, 52, 64, 244, 67, 96, 67, 90, 54, 5, 111, 178, 66, 213, 105, 67, 238, 212, 82,
	100, 147, 230, 6, 234, 93, 192, 37, 82, 56, 27, 234, 68, 33, 32, 141, 16, 148, 60, 234, 73, 49,
	17, 1, 204, 151, 233, 83, 177, 40, 127, 163, 15, 66, 166, 34, 128, 179, 100, 97, 211, 73, 231,
	128, 191, 100, 34, 232, 104, 162, 148, 94, 45, 10, 30, 115, 235, 144, 207, 120, 136, 19, 140,
	166, 187, 114, 160, 225, 156, 117, 122, 236, 36, 59, 85, 5, 177, 43, 199, 213, 182, 215, 100,
	72, 117, 238, 240, 164, 105, 64, 48, 29, 144, 24, 49, 122, 130, 97, 1, 26, 70, 49, 66, 225,
	152, 48, 79, 200, 183, 167, 73, 102, 53, 231, 172, 80, 4, 67, 116, 97, 23, 212, 61, 212, 155,
	40, 42, 129, 163, 237, 142, 168, 237, 196, 160, 130, 128, 12, 64, 112, 237, 15, 0, 61, 46, 58,
	239, 192, 82, 160, 71, 35, 25, 61, 145, 126, 194, 33, 82, 126, 111, 202, 55, 204, 133, 31, 79,
	207, 30, 178, 8, 103, 115, 0, 250, 57, 193, 150, 133, 53, 10, 83, 68, 41, 130, 105, 227, 14,
	206, 34, 114, 54, 11, 39, 3, 28, 1, 177, 232, 128, 139, 13, 30, 137, 88, 65, 219, 180, 128, 41,
	117, 36, 57, 83, 170, 167, 220, 143, 26, 106, 115, 179, 107, 2, 150, 233, 176, 206, 126, 31,
	112, 9, 194, 69, 44, 155, 145, 38, 8, 92, 166, 203, 227, 6, 0, 209, 147, 82, 228, 169, 239, 53,
	208, 183, 216, 233, 234, 209, 199, 101, 202, 160, 47, 225, 14, 54, 221, 199, 96, 152, 156, 74,
	134, 25, 186, 16, 216, 253, 140, 239, 97, 214, 25, 223, 193, 172, 89, 246, 127, 103, 244, 223,
	195, 172, 187, 184, 99, 217, 183, 46, 207, 172, 119, 130, 167, 174, 188, 235, 187, 152, 53,
	129, 156, 150, 170, 121, 39, 180, 229, 14, 102, 205, 214, 184, 204, 186, 47, 209, 58, 77, 250,
	35, 88, 53, 91, 234, 176, 106, 166, 174, 51, 171, 102, 116, 184, 6, 137, 223, 218, 54, 235, 92,
	34, 168, 169, 240, 148, 32, 34, 219, 129, 203, 172, 175, 175, 31, 10, 126, 236, 2, 101, 246,
	184, 107, 109, 205, 33, 25, 244, 237, 10, 146, 174, 87, 221, 91, 113, 130, 185, 2, 106, 108,
	70, 39, 8, 184, 58, 212, 29, 41, 16, 200, 112, 194, 211, 197, 93, 153, 189, 235, 7, 160, 224,
	233, 128, 130, 253, 208, 129, 47, 194, 123, 10, 222, 143, 224, 237, 234, 204, 159, 132, 6, 244,
	247, 222, 202, 48, 253, 151, 194, 208, 24, 195, 20, 80, 215, 230, 144, 144, 33, 211, 201, 76,
	172, 170, 174, 13, 44, 189, 7, 192, 202, 129, 144, 193, 116, 244, 176, 238, 10, 180, 4, 174,
	103, 81, 31, 205, 214, 144, 149, 240, 213, 224, 212, 192, 65, 176, 77, 209, 48, 212, 212, 224,
	229, 41, 14, 58, 60, 127, 178, 48, 182, 131, 35, 218, 131, 109, 39, 195, 221, 36, 8, 119, 147,
	145, 112, 53, 41, 132, 102, 119, 224, 86, 112, 167, 226, 246, 105, 160, 116, 52, 131, 233, 116,
	117, 97, 50, 194, 155, 103, 127, 114, 118, 114, 207, 158, 115, 86, 45, 221, 241, 143, 39, 63,
	109, 59, 186, 195, 60, 216, 52, 195, 83, 61, 253, 208, 67, 158, 238, 185, 159, 220, 243, 209,
	171, 202, 169, 208, 182, 45, 15, 140, 217, 121, 247, 236, 220, 224, 113, 118, 255, 130, 226,
	219, 111, 91, 223, 188, 127, 113, 193, 130, 15, 35, 91, 222, 57, 208, 60, 112, 231, 254, 95,
	101, 111, 56, 215, 116, 255, 234, 103, 252, 203, 255, 60, 91, 175, 152, 251, 200, 153, 69, 215,
	175, 104, 126, 111, 75, 73, 229, 155, 231, 207, 84, 238, 171, 108, 124, 114, 74, 203, 186, 185,
	99, 183, 47, 81, 55, 78, 123, 116, 239, 204, 226, 186, 202, 182, 141, 207, 156, 10, 63, 121,
	115, 165, 26, 148, 10, 251, 175, 58, 48, 103, 197, 161, 138, 245, 127, 91, 211, 191, 225, 205,
	189, 242, 38, 16, 223, 240, 203, 137, 185, 133, 194, 134, 213, 19, 127, 182, 60, 247, 193, 205,
	149, 171, 111, 219, 176, 222, 169, 82, 55, 95, 238, 210, 4, 93, 63, 254, 206, 4, 202, 238, 149,
	9, 28, 148, 190, 49, 165, 111, 76, 233, 27, 83, 250, 198, 148, 190, 49, 165, 111, 76, 233, 27,
	83, 250, 198, 244, 83, 185, 49, 29, 31, 206, 62, 213, 181, 217, 211, 16, 207, 28, 25, 143, 31,
	59, 92, 155, 189, 38, 227, 96, 221, 244, 139, 121, 238, 111, 101, 77, 167, 223, 71, 225, 119,
	63, 238, 219, 109, 169, 132, 79, 47, 184, 212, 125, 87, 100, 254, 76, 247, 55, 219, 223, 233,
	87, 90, 245, 247, 140, 199, 25, 230, 193, 147, 39, 201, 113, 210, 16, 236, 181, 178, 95, 233,
	45, 151, 202, 247, 172, 220, 240, 124, 183, 46, 210, 39, 5, 236, 75, 57, 103, 127, 126, 244,
	24, 115, 117, 219, 182, 175, 223, 170, 123, 117, 109, 207, 179, 203, 142, 100, 93, 216, 250,
	239, 41, 75, 190, 220, 145, 249, 50, 86, 235, 246, 124, 61, 250, 236, 163, 92, 109, 61, 51,
	207, 216, 249, 238, 180, 89, 158, 62, 191, 189, 72, 142, 239, 95, 54, 118, 192, 197, 183, 165,
	155, 238, 191, 162, 232, 58, 187, 40, 28, 63, 185, 158, 49, 181, 156, 138, 186, 150, 155, 239,
	219, 120, 18, 55, 238, 142, 182, 30, 57, 249, 225, 160, 123, 235, 6, 95, 92, 102, 23, 23, 54,
	254, 245, 211, 173, 108, 81, 195, 251, 239, 249, 91, 166, 190, 51, 41, 247, 141, 28, 245, 246,
	217, 43, 86, 157, 249, 98, 241, 246, 150, 141, 71, 226, 175, 172, 252, 252, 116, 118, 211, 11,
	241, 167, 95, 187, 116, 62, 187, 105, 93, 28, 255, 243, 139, 15, 123, 238, 154, 245, 126, 227,
	234, 173, 99, 216, 243, 207, 127, 62, 252, 84, 222, 133, 139, 231, 6, 88, 87, 31, 30, 89, 186,
	119, 85, 101, 149, 112, 231, 115, 143, 31, 250, 108, 77, 241, 244, 141, 71, 158, 106, 253, 69,
	255, 214, 41, 203, 119, 213, 120, 60, 158, 220, 47, 197, 193, 131, 135, 13, 124, 185, 172, 108,
	202, 164, 29, 213, 89, 89, 89, 163, 182, 55, 126, 224, 241, 108, 154, 244, 222, 141, 47, 156,
	251, 79, 155, 247, 244, 35, 55, 221, 242, 113, 121, 227, 169, 227, 121, 141, 87, 30, 122, 122,
	185, 166, 220, 83, 191, 102, 249, 218, 141, 117, 119, 79, 60, 188, 237, 161, 123, 178, 123, 25,
	74, 110, 52, 52, 178, 181, 118, 247, 194, 97, 107, 91, 107, 155, 31, 238, 182, 239, 196, 185,
	21, 67, 24, 104, 231, 11, 227, 153, 214, 218, 109, 123, 198, 177, 165, 19, 167, 94, 53, 158,
	193, 243, 99, 117, 205, 107, 79, 101, 71, 238, 155, 49, 237, 29, 222, 215, 47, 241, 24, 30,
	207, 177, 201, 7, 70, 212, 164, 153, 111, 154, 249, 166, 153, 111, 154, 249, 166, 153, 111,
	154, 249, 166, 153, 111, 154, 249, 166, 153, 239, 255, 55, 243, 45, 93, 181, 237, 225, 178,
	150, 192, 144, 235, 163, 57, 124, 69, 224, 179, 37, 237, 23, 138, 115, 159, 171, 142, 95, 168,
	139, 47, 40, 159, 217, 20, 191, 180, 110, 105, 203, 139, 120, 255, 210, 51, 237, 109, 5, 213,
	197, 220, 160, 211, 187, 122, 125, 209, 71, 62, 208, 60, 176, 219, 193, 230, 195, 90, 251, 164,
	96, 206, 145, 172, 134, 121, 255, 10, 90, 217, 129, 209, 253, 223, 95, 49, 108, 230, 190, 19,
	227, 166, 182, 214, 110, 154, 53, 117, 80, 193, 99, 111, 116, 93, 126, 104, 236, 194, 130, 7,
	152, 99, 181, 187, 231, 149, 189, 210, 187, 247, 216, 121, 31, 196, 126, 247, 200, 240, 232,
	179, 210, 237, 177, 117, 13, 237, 231, 253, 79, 151, 222, 241, 198, 173, 243, 75, 182, 23, 31,
	30, 216, 50, 98, 168, 231, 254, 233, 111, 43, 145, 9, 119, 180, 53, 94, 157, 247, 135, 161,
	101, 101, 147, 73, 251, 194, 215, 107, 115, 219, 23, 238, 107, 205, 26, 180, 175, 117, 241,
	222, 221, 43, 79, 180, 109, 191, 105, 246, 254, 30, 99, 95, 24, 180, 217, 236, 246, 90, 112,
	239, 152, 55, 111, 252, 227, 189, 211, 90, 235, 216, 39, 30, 91, 121, 183, 92, 120, 76, 212,
	154, 148, 188, 241, 207, 92, 217, 222, 180, 172, 97, 206, 172, 39, 244, 13, 15, 101, 189, 190,
	172, 62, 77, 122, 211, 164, 55, 77, 122, 211, 164, 55, 77, 122, 211, 164, 55, 77, 122, 211,
	164, 55, 77, 122, 127, 136, 244, 110, 250, 73, 147, 222, 235, 126, 253, 172, 241, 95, 14, 51,
	253, 146,
];

// Offline 32x16/60 PQ or HLG patches with a 48 kHz sine AAC track.
#[cfg(test)]
const FILE_AV1_PQ: &[u8] = &[
	120, 218, 99, 96, 96, 80, 72, 43, 169, 44, 200, 44, 206, 207, 101, 96, 96, 98, 0, 209, 137,
	101, 6, 134, 64, 218, 40, 183, 192, 196, 144, 129, 129, 53, 40, 55, 63, 191, 140, 129, 129, 33,
	39, 183, 44, 35, 133, 1, 5, 48, 191, 0, 18, 70, 12, 140, 12, 32, 132, 0, 140, 168, 170, 208,
	249, 14, 12, 120, 1, 51, 208, 29, 26, 37, 69, 137, 217, 64, 118, 76, 73, 54, 216, 78, 102, 44,
	166, 25, 97, 232, 36, 198, 94, 5, 32, 22, 0, 115, 85, 82, 83, 74, 138, 129, 180, 76, 106, 78,
	113, 9, 146, 14, 35, 132, 94, 198, 5, 185, 41, 153, 137, 32, 93, 185, 41, 232, 126, 183, 1, 59,
	43, 244, 8, 152, 163, 155, 145, 146, 83, 4, 147, 41, 203, 76, 73, 69, 86, 25, 6, 228, 231, 123,
	36, 230, 165, 228, 164, 130, 212, 48, 122, 231, 102, 230, 165, 1, 25, 34, 101, 185, 96, 67,
	145, 157, 169, 146, 2, 145, 147, 73, 41, 74, 77, 67, 114, 20, 79, 105, 81, 142, 2, 132, 205,
	200, 93, 92, 146, 148, 3, 100, 207, 47, 46, 41, 78, 65, 82, 211, 15, 138, 56, 28, 65, 1, 242,
	184, 0, 131, 7, 144, 246, 128, 169, 144, 240, 73, 44, 75, 54, 51, 212, 51, 180, 212, 51, 52,
	48, 84, 200, 201, 76, 74, 204, 207, 213, 77, 44, 131, 25, 193, 32, 241, 255, 63, 144, 148, 6,
	138, 56, 55, 50, 248, 48, 112, 241, 2, 121, 76, 242, 127, 110, 253, 12, 152, 200, 48, 1, 20,
	152, 92, 105, 153, 169, 57, 96, 155, 68, 146, 74, 138, 160, 97, 232, 220, 0, 193, 12, 18, 197,
	37, 224, 240, 133, 57, 135, 25, 234, 42, 25, 160, 179, 147, 145, 196, 145, 177, 8, 80, 174, 10,
	22, 20, 208, 120, 7, 170, 79, 206, 71, 164, 13, 54, 39, 6, 6, 246, 64, 6, 6, 142, 104, 160,
	107, 66, 241, 164, 19, 38, 44, 233, 132, 145, 145, 244, 244, 137, 39, 157, 176, 192, 210, 201,
	89, 220, 233, 100, 55, 48, 44, 120, 19, 176, 165, 147, 226, 252, 210, 60, 100, 149, 193, 64,
	126, 10, 82, 58, 169, 128, 166, 19, 129, 226, 92, 20, 67, 137, 73, 35, 54, 208, 52, 82, 135,
	150, 70, 242, 128, 153, 58, 17, 195, 219, 140, 144, 44, 1, 114, 42, 16, 152, 165, 22, 167, 128,
	35, 142, 185, 161, 161, 65, 21, 24, 142, 44, 64, 90, 220, 65, 20, 172, 82, 137, 9, 132, 89,
	129, 34, 172, 130, 29, 97, 79, 25, 216, 128, 44, 70, 38, 148, 36, 0, 85, 3, 10, 14, 164, 36,
	192, 4, 137, 30, 22, 168, 205, 140, 9, 4, 146, 130, 10, 82, 82, 0, 1, 144, 198, 3, 64, 252, 26,
	136, 159, 1, 49, 43, 196, 124, 120, 210, 0, 202, 179, 54, 1, 147, 7, 48, 68, 216, 75, 129, 201,
	163, 30, 40, 38, 85, 156, 94, 144, 2, 50, 172, 40, 63, 39, 7, 234, 6, 70, 112, 170, 150, 41,
	78, 74, 47, 96, 64, 200, 48, 66, 109, 0, 209, 137, 165, 41, 37, 160, 48, 138, 204, 77, 45, 129,
	133, 149, 34, 114, 196, 1, 35, 187, 40, 177, 160, 32, 7, 57, 242, 116, 50, 33, 169, 67, 101,
	101, 73, 126, 62, 56, 114, 18, 193, 154, 193, 129, 12, 204, 105, 105, 192, 156, 102, 14, 204,
	104, 198, 64, 46, 71, 90, 81, 42, 176, 132, 96, 230, 202, 5, 42, 186, 199, 196, 128, 146, 17,
	25, 152, 50, 86, 69, 190, 252, 224, 203, 205, 29, 53, 179, 126, 251, 253, 181, 175, 254, 220,
	219, 94, 116, 105, 117, 168, 74, 121, 200, 149, 73, 42, 28, 106, 47, 219, 158, 29, 205, 124,
	157, 190, 105, 221, 4, 171, 221, 77, 145, 106, 105, 15, 25, 207, 199, 153, 122, 123, 46, 218,
	113, 190, 74, 63, 238, 81, 233, 186, 203, 23, 119, 200, 238, 62, 95, 45, 190, 228, 211, 234,
	222, 89, 155, 109, 59, 119, 70, 230, 133, 69, 79, 121, 155, 45, 221, 181, 227, 230, 58, 179,
	172, 51, 95, 222, 102, 157, 206, 154, 187, 214, 239, 218, 252, 104, 187, 109, 181, 57, 75, 131,
	166, 30, 11, 55, 173, 203, 250, 179, 116, 243, 235, 140, 181, 174, 89, 57, 105, 201, 198, 146,
	51, 47, 70, 117, 93, 9, 91, 116, 110, 182, 228, 140, 51, 199, 82, 86, 0, 133, 181, 3, 189, 212,
	140, 13, 151, 204, 242, 146, 235, 84, 155, 184, 42, 107, 150, 231, 146, 69, 16, 102, 30, 90,
	177, 96, 36, 44, 2, 42, 99, 239, 212, 135, 95, 140, 58, 101, 94, 96, 238, 193, 198, 22, 123,
	128, 209, 100, 198, 173, 155, 182, 215, 2, 46, 251, 168, 157, 146, 202, 241, 142, 236, 154,
	249, 246, 107, 213, 182, 107, 75, 111, 255, 63, 220, 255, 249, 141, 248, 234, 253, 255, 55, 30,
	255, 253, 69, 124, 245, 252, 255, 249, 143, 191, 62, 226, 222, 27, 113, 107, 238, 172, 13, 86,
	140, 95, 246, 124, 214, 120, 173, 243, 253, 199, 39, 153, 18, 225, 171, 230, 54, 199, 102, 102,
	21, 26, 6, 111, 93, 126, 229, 221, 108, 211, 144, 165, 183, 215, 255, 180, 151, 252, 233, 215,
	185, 183, 66, 69, 69, 69, 237, 91, 162, 162, 162, 186, 236, 33, 39, 39, 63, 159, 237, 229, 156,
	156, 156, 22, 219, 230, 222, 81, 81, 89, 225, 115, 211, 122, 255, 167, 143, 127, 12, 222, 76,
	113, 116, 123, 225, 60, 247, 245, 67, 157, 185, 188, 87, 54, 118, 230, 102, 182, 53, 204, 238,
	156, 183, 180, 174, 201, 235, 234, 166, 73, 109, 226, 60, 5, 153, 106, 101, 233, 230, 63, 107,
	246, 197, 171, 207, 251, 89, 179, 99, 50, 251, 233, 167, 159, 186, 148, 24, 128, 124, 61, 67,
	23, 134, 159, 53, 155, 142, 56, 48, 218, 120, 5, 8, 186, 48, 228, 199, 86, 214, 237, 152, 247,
	90, 188, 180, 59, 52, 232, 178, 174, 145, 16, 12, 20, 168, 168, 60, 240, 189, 168, 89, 65, 84,
	112, 216, 204, 220, 52, 217, 233, 90, 146, 146, 116, 153, 148, 110, 88, 210, 187, 218, 127,
	223, 77, 213, 182, 150, 255, 255, 94, 247, 63, 206, 57, 124, 245, 255, 223, 243, 235, 175, 29,
	200, 63, 95, 255, 246, 223, 31, 253, 114, 83, 5, 249, 55, 123, 121, 190, 242, 167, 0, 227, 149,
	253, 210, 142, 171, 185, 255, 124, 210, 164, 110, 115, 206, 136, 121, 146, 86, 34, 158, 100,
	41, 121, 171, 75, 61, 252, 244, 83, 135, 128, 159, 53, 43, 34, 2, 228, 245, 167, 159, 98, 237,
	188, 98, 23, 175, 63, 129, 225, 65, 205, 190, 24, 167, 195, 124, 124, 118, 49, 119, 42, 231,
	76, 209, 40, 219, 146, 236, 93, 57, 127, 198, 191, 47, 182, 27, 109, 252, 79, 185, 199, 154,
	109, 51, 189, 42, 123, 77, 83, 85, 165, 55, 228, 66, 102, 169, 135, 255, 159, 185, 194, 58, 11,
	85, 157, 156, 124, 139, 255, 197, 159, 168, 81, 251, 23, 127, 250, 39, 167, 252, 233, 159, 85,
	199, 246, 245, 63, 253, 179, 205, 49, 242, 60, 151, 221, 126, 249, 85, 69, 236, 199, 211, 142,
	89, 157, 177, 94, 220, 17, 244, 179, 142, 113, 205, 244, 254, 166, 20, 227, 7, 137, 185, 171,
	51, 117, 92, 54, 243, 254, 91, 221, 62, 35, 42, 98, 77, 222, 146, 73, 156, 39, 218, 27, 136,
	10, 9, 137, 198, 45, 5, 0, 119, 185, 24, 202,
];

// Offline 32x16/60 PQ or HLG patches with a 48 kHz sine AAC track.
#[cfg(test)]
const FILE_AV1_HLG: &[u8] = &[
	120, 218, 99, 96, 96, 80, 72, 43, 169, 44, 200, 44, 206, 207, 101, 96, 96, 98, 0, 209, 137,
	101, 6, 134, 64, 218, 40, 183, 192, 196, 144, 129, 129, 53, 40, 55, 63, 191, 140, 129, 129, 33,
	39, 183, 44, 35, 133, 1, 5, 48, 191, 0, 18, 70, 12, 140, 12, 32, 132, 0, 140, 168, 170, 208,
	249, 14, 12, 120, 1, 51, 208, 29, 26, 37, 69, 137, 217, 64, 118, 76, 73, 54, 216, 78, 102, 44,
	166, 25, 97, 232, 36, 198, 94, 5, 32, 22, 0, 115, 85, 82, 83, 74, 138, 129, 180, 76, 106, 78,
	113, 9, 146, 14, 35, 132, 94, 198, 5, 185, 41, 153, 137, 32, 93, 185, 41, 232, 126, 183, 1, 59,
	43, 244, 8, 152, 163, 155, 145, 146, 83, 4, 147, 41, 203, 76, 73, 69, 86, 25, 6, 228, 231, 123,
	36, 230, 165, 228, 164, 130, 212, 48, 122, 231, 102, 230, 165, 1, 25, 34, 101, 185, 96, 67,
	145, 157, 169, 146, 2, 145, 147, 73, 41, 74, 77, 67, 114, 20, 79, 105, 81, 142, 2, 132, 205,
	200, 93, 92, 146, 148, 3, 100, 207, 47, 46, 41, 78, 65, 82, 211, 15, 138, 56, 28, 65, 1, 242,
	184, 0, 131, 7, 144, 246, 128, 169, 144, 240, 73, 44, 75, 54, 51, 212, 51, 180, 212, 51, 52,
	48, 84, 200, 201, 76, 74, 204, 207, 213, 77, 44, 131, 25, 193, 32, 241, 255, 63, 144, 148, 6,
	138, 56, 55, 50, 248, 48, 112, 241, 2, 121, 76, 242, 127, 110, 253, 12, 152, 168, 48, 1, 20,
	152, 92, 105, 153, 169, 57, 96, 155, 68, 146, 74, 138, 160, 97, 232, 220, 0, 193, 12, 18, 197,
	37, 224, 240, 133, 57, 135, 25, 234, 42, 25, 160, 179, 147, 145, 196, 145, 177, 8, 80, 174, 10,
	22, 20, 208, 120, 7, 170, 79, 206, 71, 164, 13, 54, 39, 6, 6, 246, 64, 6, 6, 142, 104, 160,
	107, 66, 241, 164, 19, 38, 44, 233, 132, 145, 145, 244, 244, 137, 39, 157, 176, 192, 210, 201,
	89, 220, 233, 100, 55, 48, 44, 120, 19, 176, 165, 147, 226, 252, 210, 60, 100, 149, 193, 64,
	126, 10, 82, 58, 169, 128, 166, 19, 129, 226, 92, 20, 67, 137, 73, 35, 54, 208, 52, 82, 135,
	150, 70, 242, 128, 153, 58, 17, 195, 219, 140, 144, 44, 1, 114, 42, 16, 152, 165, 22, 167, 128,
	35, 142, 185, 161, 161, 65, 21, 24, 142, 44, 64, 90, 220, 65, 20, 172, 82, 137, 9, 132, 89,
	129, 34, 172, 130, 29, 97, 79, 25, 216, 128, 44, 70, 38, 148, 36, 0, 85, 3, 10, 14, 164, 36,
	192, 4, 137, 30, 22, 168, 205, 140, 9, 4, 146, 130, 10, 82, 82, 0, 1, 144, 198, 3, 64, 252, 26,
	136, 159, 1, 49, 43, 196, 124, 120, 210, 0, 202, 179, 54, 1, 147, 7, 48, 68, 216, 75, 129, 201,
	163, 30, 40, 38, 85, 156, 94, 144, 2, 50, 172, 40, 63, 39, 7, 234, 6, 70, 112, 170, 150, 41,
	78, 74, 47, 96, 64, 200, 48, 66, 109, 0, 209, 137, 165, 41, 37, 160, 48, 138, 204, 77, 45, 129,
	133, 149, 34, 114, 196, 1, 35, 187, 40, 177, 160, 32, 7, 57, 242, 116, 50, 33, 169, 67, 101,
	101, 73, 126, 62, 56, 114, 18, 193, 154, 193, 129, 12, 204, 105, 105, 192, 156, 102, 14, 204,
	104, 198, 64, 46, 71, 90, 81, 42, 176, 132, 96, 230, 202, 5, 42, 186, 199, 196, 128, 146, 17,
	25, 152, 50, 86, 69, 190, 252, 224, 203, 205, 29, 53, 179, 126, 251, 253, 181, 175, 254, 220,
	219, 94, 116, 105, 117, 168, 74, 121, 200, 149, 73, 42, 28, 106, 47, 219, 158, 29, 205, 124,
	157, 190, 105, 221, 4, 171, 221, 77, 145, 106, 105, 15, 25, 207, 199, 153, 122, 123, 46, 218,
	113, 190, 74, 63, 238, 81, 233, 186, 203, 23, 119, 200, 238, 62, 95, 45, 190, 228, 211, 234,
	222, 89, 155, 109, 59, 119, 70, 230, 133, 69, 79, 121, 155, 45, 221, 181, 227, 230, 58, 179,
	172, 51, 95, 222, 102, 157, 206, 154, 187, 214, 239, 218, 252, 104, 187, 109, 181, 57, 75, 131,
	166, 30, 11, 55, 173, 203, 250, 179, 116, 243, 235, 140, 181, 174, 89, 57, 105, 201, 198, 146,
	51, 47, 70, 117, 93, 9, 91, 116, 110, 182, 228, 140, 51, 199, 82, 86, 0, 133, 181, 3, 189, 212,
	140, 13, 151, 204, 242, 146, 235, 84, 155, 184, 42, 107, 150, 231, 146, 69, 16, 102, 30, 90,
	177, 96, 36, 44, 2, 42, 99, 239, 212, 135, 31, 185, 111, 99, 94, 96, 110, 194, 197, 54, 247, 0,
	163, 201, 140, 91, 55, 109, 175, 5, 92, 246, 81, 59, 37, 149, 227, 29, 217, 53, 243, 237, 215,
	170, 109, 215, 150, 222, 254, 127, 184, 255, 243, 27, 241, 213, 251, 255, 111, 60, 254, 251,
	139, 248, 234, 249, 255, 243, 31, 127, 125, 196, 189, 55, 226, 214, 220, 89, 27, 172, 24, 191,
	236, 249, 172, 241, 90, 231, 251, 143, 79, 50, 37, 194, 87, 205, 109, 142, 205, 204, 42, 52,
	12, 222, 186, 252, 202, 187, 217, 166, 33, 75, 111, 175, 255, 105, 47, 249, 211, 175, 115, 111,
	133, 138, 138, 138, 218, 183, 68, 69, 69, 117, 217, 67, 78, 78, 126, 62, 219, 203, 57, 57, 57,
	45, 182, 205, 189, 163, 162, 178, 194, 231, 166, 245, 254, 79, 31, 255, 24, 188, 153, 226, 232,
	246, 194, 121, 238, 235, 135, 58, 115, 121, 175, 108, 236, 204, 205, 108, 107, 152, 221, 57,
	111, 105, 93, 147, 215, 213, 77, 147, 218, 196, 121, 10, 50, 213, 202, 210, 205, 127, 214, 236,
	139, 87, 159, 247, 179, 102, 199, 100, 246, 211, 79, 63, 117, 41, 49, 0, 249, 122, 134, 46, 12,
	63, 107, 54, 29, 113, 96, 180, 241, 10, 16, 116, 97, 200, 143, 173, 172, 219, 49, 239, 181,
	120, 105, 119, 104, 208, 101, 93, 35, 33, 24, 40, 80, 81, 121, 224, 123, 81, 179, 130, 168,
	224, 176, 153, 185, 105, 178, 211, 181, 36, 37, 233, 50, 41, 221, 176, 164, 119, 181, 255, 190,
	155, 170, 109, 45, 255, 255, 189, 238, 127, 156, 115, 248, 234, 255, 191, 231, 215, 95, 59,
	144, 127, 190, 254, 237, 191, 63, 250, 229, 166, 10, 242, 111, 246, 242, 124, 229, 79, 1, 198,
	43, 251, 165, 29, 87, 115, 255, 249, 164, 73, 221, 230, 156, 17, 243, 36, 173, 68, 60, 201, 82,
	242, 86, 151, 122, 248, 233, 167, 14, 1, 63, 107, 86, 68, 4, 200, 235, 79, 63, 197, 218, 121,
	197, 46, 94, 127, 2, 195, 131, 154, 125, 49, 78, 135, 249, 248, 236, 98, 238, 84, 206, 153,
	162, 81, 182, 37, 217, 187, 114, 254, 140, 127, 95, 108, 55, 218, 248, 159, 114, 143, 53, 219,
	102, 122, 85, 246, 154, 166, 170, 74, 111, 200, 133, 204, 82, 15, 255, 63, 115, 133, 117, 22,
	170, 58, 57, 249, 22, 255, 139, 63, 81, 163, 246, 47, 254, 244, 79, 78, 249, 211, 63, 171, 142,
	237, 235, 127, 250, 103, 155, 99, 228, 121, 46, 187, 253, 242, 171, 138, 216, 143, 167, 29,
	179, 58, 99, 189, 184, 35, 232, 103, 29, 227, 154, 233, 253, 77, 41, 198, 15, 18, 115, 87, 103,
	234, 184, 108, 230, 253, 183, 186, 125, 70, 84, 196, 154, 188, 37, 147, 56, 79, 180, 55, 16,
	21, 18, 18, 141, 91, 10, 0, 3, 62, 25, 152,
];
