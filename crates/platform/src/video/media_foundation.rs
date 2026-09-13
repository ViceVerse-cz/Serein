//! Windows attachment decoding. The caller owns validated, bounded network I/O; Media
//! Foundation receives an unnamed read-only stream, never a URL or an account token.
#![allow(unsafe_code)]

use std::{
	ffi::c_void,
	io::{Read, Seek, SeekFrom},
	marker::PhantomData,
	rc::Rc,
	sync::{Arc, Mutex},
};
use windows::{
	Win32::{
		Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL, E_POINTER, HMODULE, S_FALSE, S_OK},
		Graphics::{
			Direct3D::D3D_DRIVER_TYPE_HARDWARE,
			Direct3D11::{
				D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
				ID3D11Device, ID3D11Multithread,
			},
		},
		Media::MediaFoundation::*,
		System::Com::{
			COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize, ISequentialStream_Impl, IStream,
			IStream_Impl, LOCKTYPE, STATFLAG, STATSTG, STGC, STGM_READ, STGTY_STREAM, STREAM_SEEK,
			STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET, StructuredStorage::PROPVARIANT,
		},
	},
	core::{GUID, HRESULT, Interface, Ref, implement},
};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_SECONDS: f64 = 2.0 * 60.0 * 60.0;
const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const ALL: u32 = MF_SOURCE_READER_ALL_STREAMS.0 as u32;
const UNSUPPORTED: &str = "This video format or codec is not supported by Windows.";
const INVALID: &str = "The video could not be decoded safely.";

use super::{Info, ReadSeek, Sample};

// Keep the reader and byte stream ahead of Runtime: COM objects must be released
// before MFShutdown/CoUninitialize. Rc also makes this actor thread-affine.
pub struct Decoder {
	reader: IMFSourceReader,
	_stream: IMFByteStream,
	_manager: Option<IMFDXGIDeviceManager>,
	_device: Option<ID3D11Device>,
	_runtime: Runtime,
	info: Info,
	video_index: u32,
	audio_index: Option<u32>,
	video_done: bool,
	audio_done: bool,
	width: u32,
	height: u32,
	stride: i32,
	buffer_height: u32,
	crop: (u32, u32),
	rotation: u32,
}

/// A negotiated reader before the decoder takes ownership.
struct Parts {
	reader: IMFSourceReader,
	stream: IMFByteStream,
	info: Info,
	video_index: u32,
	audio_index: Option<u32>,
	width: u32,
	height: u32,
	stride: i32,
	rotation: u32,
}

/// A D3D11 video device wrapped in a DXGI device manager, the handle Media Foundation needs
/// for DXVA decoding. `None` leaves the caller in software mode.
pub(super) unsafe fn dxgi_manager() -> Option<(ID3D11Device, IMFDXGIDeviceManager)> {
	unsafe {
		let mut device: Option<ID3D11Device> = None;
		D3D11CreateDevice(
			None,
			D3D_DRIVER_TYPE_HARDWARE,
			HMODULE::default(),
			D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
			None,
			D3D11_SDK_VERSION,
			Some(&mut device),
			None,
			None,
		)
		.ok()?;
		let device = device?;
		if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
			let _ = multithread.SetMultithreadProtected(true);
		}
		let mut token = 0;
		let mut manager: Option<IMFDXGIDeviceManager> = None;
		MFCreateDXGIDeviceManager(&mut token, &mut manager).ok()?;
		let manager = manager?;
		manager.ResetDevice(&device, token).ok()?;
		Some((device, manager))
	}
}

pub(super) struct Runtime(PhantomData<Rc<()>>);
impl Runtime {
	pub(super) fn open() -> Result<Self, &'static str> {
		// SAFETY: Called and dropped only on the dedicated decoder thread.
		unsafe {
			CoInitializeEx(None, COINIT_MULTITHREADED)
				.ok()
				.map_err(|_| UNSUPPORTED)?;
			if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
				CoUninitialize();
				return Err(UNSUPPORTED);
			}
		}
		Ok(Self(PhantomData))
	}
}
impl Drop for Runtime {
	fn drop(&mut self) {
		// SAFETY: Balanced successful initialization on this same thread, after COM fields drop.
		unsafe {
			let _ = MFShutdown();
			CoUninitialize();
		}
	}
}

impl Decoder {
	pub fn open(mut source: Box<dyn ReadSeek>) -> Result<Self, &'static str> {
		let length = source.seek(SeekFrom::End(0)).map_err(|_| INVALID)?;
		source.seek(SeekFrom::Start(0)).map_err(|_| INVALID)?;
		let mut header = [0_u8; 8];
		source.read_exact(&mut header).map_err(|_| INVALID)?;
		// Only the MPEG-4/MOV source is supported. Its documented source cannot
		// resolve external tracks: media must live in mdat boxes. No base URL is supplied.
		if length < 8
			|| !matches!(
				&header[4..],
				b"ftyp" | b"moov" | b"mdat" | b"free" | b"skip" | b"wide"
			) {
			return Err(UNSUPPORTED);
		}
		source.seek(SeekFrom::Start(0)).map_err(|_| INVALID)?;
		let runtime = Runtime::open()?;
		let stream: IStream = ReadStream {
			source: Arc::new(Mutex::new(source)),
			position: Mutex::new(0),
			length,
		}
		.into();
		// DXVA first when a video-capable GPU device exists; the software reader is the
		// fallback for anything the accelerated reader rejects.
		// SAFETY: COM calls on the thread that initialized the runtime.
		let mut hardware = unsafe { dxgi_manager() };
		let mut error = UNSUPPORTED;
		for accelerated in [true, false] {
			if accelerated && hardware.is_none() {
				continue;
			}
			// SAFETY: Rewinding our own IStream between attempts.
			unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }.map_err(|_| INVALID)?;
			let manager = hardware
				.as_ref()
				.filter(|_| accelerated)
				.map(|(_, manager)| manager);
			match Self::configure(&stream, manager) {
				Ok(parts) => {
					let (device, manager) = match (accelerated, hardware.take()) {
						(true, Some((device, manager))) => (Some(device), Some(manager)),
						_ => (None, None),
					};
					return Ok(Self {
						reader: parts.reader,
						_stream: parts.stream,
						_manager: manager,
						_device: device,
						_runtime: runtime,
						info: parts.info,
						video_index: parts.video_index,
						audio_index: parts.audio_index,
						video_done: false,
						audio_done: parts.audio_index.is_none(),
						width: parts.width,
						height: parts.height,
						stride: parts.stride,
						buffer_height: parts.height,
						crop: (0, 0),
						rotation: parts.rotation,
					});
				}
				Err(failure) => error = failure,
			}
		}
		Err(error)
	}

	fn configure(
		source: &IStream,
		manager: Option<&IMFDXGIDeviceManager>,
	) -> Result<Parts, &'static str> {
		// SAFETY: COM inputs remain live; out parameters are initialized. The OS
		// wrapper implements asynchronous IMFByteStream reads around our synchronized IStream.
		unsafe {
			let stream = MFCreateMFByteStreamOnStream(source).map_err(|_| UNSUPPORTED)?;
			let mut attributes = None;
			MFCreateAttributes(&mut attributes, 3).map_err(|_| UNSUPPORTED)?;
			let attributes = attributes.ok_or(INVALID)?;
			if let Some(manager) = manager {
				attributes
					.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)
					.map_err(|_| UNSUPPORTED)?;
				attributes
					.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
					.map_err(|_| UNSUPPORTED)?;
				attributes
					.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)
					.map_err(|_| UNSUPPORTED)?;
			} else {
				attributes
					.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
					.map_err(|_| UNSUPPORTED)?;
			}
			let reader = MFCreateSourceReaderFromByteStream(&stream, &attributes)
				.map_err(|_| UNSUPPORTED)?;
			reader.SetStreamSelection(ALL, false).map_err(|_| INVALID)?;
			let native = reader
				.GetNativeMediaType(VIDEO, 0)
				.map_err(|_| UNSUPPORTED)?;
			let (width, height) = dimensions(&native)?;
			let rotation = native.GetUINT32(&MF_MT_VIDEO_ROTATION).unwrap_or(0);
			if !matches!(rotation, 0 | 90 | 180 | 270) {
				return Err(UNSUPPORTED);
			}
			let mut video_index = None;
			let mut audio_index = None;
			for index in 0..32 {
				let media = match reader.GetNativeMediaType(index, 0) {
					Ok(media) => media,
					Err(error) if error.code() == MF_E_INVALIDSTREAMNUMBER => break,
					Err(_) => return Err(INVALID),
				};
				match media.GetGUID(&MF_MT_MAJOR_TYPE).map_err(|_| INVALID)? {
					kind if kind == MFMediaType_Video && video_index.is_none() => {
						video_index = Some(index)
					}
					kind if kind == MFMediaType_Audio && audio_index.is_none() => {
						audio_index = Some(index)
					}
					_ => (),
				}
			}
			let video_index = video_index.ok_or(UNSUPPORTED)?;
			let video = MFCreateMediaType().map_err(|_| INVALID)?;
			video
				.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
				.map_err(|_| INVALID)?;
			video
				.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
				.map_err(|_| INVALID)?;
			video
				.SetUINT64(
					&MF_MT_FRAME_SIZE,
					(u64::from(width) << 32) | u64::from(height),
				)
				.map_err(|_| INVALID)?;
			reader
				.SetCurrentMediaType(VIDEO, None, &video)
				.map_err(|_| UNSUPPORTED)?;
			if manager.is_some() && rotation != 0 {
				// Apply track rotation once in rgba_frame, without the GPU processor also
				// correcting it. If its control is unavailable, use the software reader.
				let extended = reader
					.cast::<IMFSourceReaderEx>()
					.map_err(|_| UNSUPPORTED)?;
				let mut configured = false;
				for index in 0..8 {
					let mut transform = None;
					if extended
						.GetTransformForStream(VIDEO, index, None, &mut transform)
						.is_err()
					{
						break;
					}
					if let Some(transform) = transform
						&& let Ok(control) = transform.cast::<IMFVideoProcessorControl>()
					{
						control
							.SetRotation(ROTATION_NONE)
							.map_err(|_| UNSUPPORTED)?;
						configured = true;
						break;
					}
				}
				if !configured {
					return Err(UNSUPPORTED);
				}
			}
			reader
				.SetStreamSelection(video_index, true)
				.map_err(|_| INVALID)?;
			let video = reader.GetCurrentMediaType(VIDEO).map_err(|_| INVALID)?;
			if dimensions(&video)? != (width, height) {
				return Err(INVALID);
			}
			let stride = video
				.GetUINT32(&MF_MT_DEFAULT_STRIDE)
				.map(|x| x as i32)
				.or_else(|_| MFGetStrideForBitmapInfoHeader(MFVideoFormat_RGB32.data1, width))
				.map_err(|_| INVALID)?;
			validate_stride(width, height, stride)?;
			let mut sample_rate = 0;
			let mut channels = 0;
			if let Some(index) = audio_index {
				let audio = reader.GetNativeMediaType(index, 0).map_err(|_| INVALID)?;
				sample_rate = audio
					.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
					.map_err(|_| UNSUPPORTED)?;
				channels = audio
					.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
					.map_err(|_| UNSUPPORTED)?;
				if !(1..=96_000).contains(&sample_rate) || !(1..=2).contains(&channels) {
					return Err(UNSUPPORTED);
				}
				let audio = MFCreateMediaType().map_err(|_| INVALID)?;
				audio
					.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)
					.map_err(|_| INVALID)?;
				audio
					.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)
					.map_err(|_| INVALID)?;
				reader
					.SetCurrentMediaType(index, None, &audio)
					.map_err(|_| UNSUPPORTED)?;
				reader
					.SetStreamSelection(index, true)
					.map_err(|_| INVALID)?;
				let audio = reader.GetCurrentMediaType(index).map_err(|_| INVALID)?;
				if audio
					.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
					.map_err(|_| INVALID)?
					!= sample_rate || audio
					.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
					.map_err(|_| INVALID)?
					!= channels || audio
					.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE)
					.map_err(|_| INVALID)?
					!= 32
				{
					return Err(UNSUPPORTED);
				}
			}
			let duration = reader
				.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
				.map_err(|_| INVALID)?;
			let duration = u64::try_from(&duration).map_err(|_| INVALID)? as f64 / 10_000_000.0;
			if duration <= 0.0 || duration > MAX_SECONDS {
				return Err("Videos longer than two hours are not supported.");
			}
			let (display_width, display_height) = if rotation == 90 || rotation == 270 {
				(height, width)
			} else {
				(width, height)
			};
			Ok(Parts {
				reader,
				stream,
				info: Info {
					width: display_width,
					height: display_height,
					duration,
					sample_rate,
					channels: channels as u16,
				},
				video_index,
				audio_index,
				width,
				height,
				stride,
				rotation,
			})
		}
	}

	pub fn info(&self) -> Info {
		self.info
	}

	pub fn seek(&mut self, seconds: f64) -> Result<(), &'static str> {
		if !seconds.is_finite() || seconds < 0.0 || seconds > self.info.duration {
			return Err(INVALID);
		}
		let position = PROPVARIANT::from((seconds * 10_000_000.0) as i64);
		// SAFETY: Live reader on its owner thread; scalar PROPVARIANT remains live for the call.
		unsafe {
			self.reader
				.SetStreamSelection(self.video_index, true)
				.map_err(|_| INVALID)?;
			if let Some(index) = self.audio_index {
				self.reader
					.SetStreamSelection(index, true)
					.map_err(|_| INVALID)?;
			}
			self.reader
				.SetCurrentPosition(&GUID::zeroed(), &position)
				.map_err(|_| "This video cannot seek to that position.")?;
		}
		self.video_done = false;
		self.audio_done = self.audio_index.is_none();
		Ok(())
	}

	pub fn read_video(&mut self) -> Result<Option<Sample>, &'static str> {
		self.read_from(self.video_index)
	}

	pub fn read_audio(&mut self) -> Result<Option<Sample>, &'static str> {
		match self.audio_index {
			Some(index) => self.read_from(index),
			None => Ok(None),
		}
	}

	fn read_from(&mut self, selector: u32) -> Result<Option<Sample>, &'static str> {
		for _ in 0..64 {
			if (self.video_done && self.audio_done)
				|| (selector == self.video_index && self.video_done)
				|| (Some(selector) == self.audio_index && self.audio_done)
			{
				return Ok(None);
			}
			let mut index = 0;
			let mut flags = 0;
			let mut timestamp = 0;
			let mut sample = None;
			// SAFETY: All output pointers refer to initialized locals. Decoding runs only on this actor thread.
			unsafe {
				self.reader.ReadSample(
					selector,
					0,
					Some(&mut index),
					Some(&mut flags),
					Some(&mut timestamp),
					Some(&mut sample),
				)
			}
			.map_err(|_| INVALID)?;
			if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
				return Err(INVALID);
			}
			if flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0 {
				// Decoders commonly finish negotiating their output on the first
				// sample. Accept that notification only when our bounded layout holds.
				unsafe {
					let media = self
						.reader
						.GetCurrentMediaType(index)
						.map_err(|_| INVALID)?;
					if index == self.video_index {
						if media.GetGUID(&MF_MT_SUBTYPE).map_err(|_| INVALID)?
							!= MFVideoFormat_RGB32
						{
							return Err(INVALID);
						}
						let (buffer_width, buffer_height, crop) =
							output_aperture(&media, self.width, self.height)?;
						self.buffer_height = buffer_height;
						self.crop = crop;
						self.stride = media
							.GetUINT32(&MF_MT_DEFAULT_STRIDE)
							.map(|x| x as i32)
							.or_else(|_| {
								MFGetStrideForBitmapInfoHeader(
									MFVideoFormat_RGB32.data1,
									buffer_width,
								)
							})
							.map_err(|_| INVALID)?;
						validate_stride(buffer_width, buffer_height, self.stride)?;
					} else if Some(index) == self.audio_index
						&& (media.GetGUID(&MF_MT_SUBTYPE).map_err(|_| INVALID)?
							!= MFAudioFormat_Float
							|| media
								.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)
								.map_err(|_| INVALID)? != self.info.sample_rate
							|| media
								.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)
								.map_err(|_| INVALID)? != u32::from(self.info.channels))
					{
						return Err(INVALID);
					}
				}
			}
			if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
				if index == self.video_index {
					self.video_done = true;
				}
				if Some(index) == self.audio_index {
					self.audio_done = true;
				}
			}
			let Some(sample) = sample else {
				continue;
			};
			let pts = timestamp as f64 / 10_000_000.0;
			if !(-MAX_SECONDS..=MAX_SECONDS).contains(&pts) {
				return Err(INVALID);
			}
			if index == self.video_index {
				let rgba = self.video_frame(&sample)?;
				return Ok(Some(Sample::Video {
					pts,
					width: self.info.width,
					height: self.info.height,
					rgba,
				}));
			}
			if Some(index) == self.audio_index {
				let bytes = sample_bytes(&sample)?;
				let frame_bytes = usize::from(self.info.channels) * 4;
				if frame_bytes == 0
					|| bytes.len() % frame_bytes != 0
					|| bytes.len() / frame_bytes > self.info.sample_rate as usize
				{
					return Err(INVALID);
				}
				let frames = bytes
					.chunks_exact(frame_bytes)
					.map(|frame| {
						let left = f32::from_le_bytes(frame[0..4].try_into().unwrap());
						let right = if self.info.channels == 2 {
							f32::from_le_bytes(frame[4..8].try_into().unwrap())
						} else {
							left
						};
						[left, right].map(|x| {
							if x.is_finite() {
								x.clamp(-1.0, 1.0)
							} else {
								0.0
							}
						})
					})
					.collect();
				return Ok(Some(Sample::Audio { pts, frames }));
			}
		}
		Err(INVALID)
	}

	fn video_frame(&self, sample: &IMFSample) -> Result<Vec<u8>, &'static str> {
		let convert = |bytes: &[u8], stride| {
			rgba_frame(
				bytes,
				self.width,
				self.height,
				stride,
				self.rotation,
				self.buffer_height,
				self.crop,
			)
		};
		// The media type's default stride describes a linear buffer, not necessarily the
		// GPU surface. Read the actual top scanline/pitch so DXVA cannot flip the picture.
		// SAFETY: Lock2DSize supplies the accessible allocation; validate every offset and
		// length before borrowing it, then release the read-only lock after conversion.
		unsafe {
			if sample.GetTotalLength().map_err(|_| INVALID)? as usize > MAX_BYTES {
				return Err(INVALID);
			}
			let buffer = sample.ConvertToContiguousBuffer().map_err(|_| INVALID)?;
			if let Ok(surface) = buffer.cast::<IMF2DBuffer2>() {
				let mut top = std::ptr::null_mut();
				let mut start = std::ptr::null_mut();
				let mut stride = 0;
				let mut length = 0;
				surface
					.Lock2DSize(
						MF2DBuffer_LockFlags_Read,
						&mut top,
						&mut stride,
						&mut start,
						&mut length,
					)
					.map_err(|_| INVALID)?;
				let result = (|| {
					if start.is_null() || top.is_null() || length as usize > MAX_BYTES {
						return Err(INVALID);
					}
					let pitch =
						validate_stride(self.width + self.crop.0, self.buffer_height, stride)?;
					let top_offset = (top as usize).checked_sub(start as usize).ok_or(INVALID)?;
					let bottom_up_offset = if stride < 0 {
						pitch * (self.buffer_height as usize - 1)
					} else {
						0
					};
					let first = top_offset.checked_sub(bottom_up_offset).ok_or(INVALID)?;
					let size = pitch * self.buffer_height as usize;
					if first
						.checked_add(size)
						.is_none_or(|end| end > length as usize)
					{
						return Err(INVALID);
					}
					convert(std::slice::from_raw_parts(start.add(first), size), stride)
				})();
				surface.Unlock2D().map_err(|_| INVALID)?;
				return result;
			}
		}
		convert(&sample_bytes(sample)?, self.stride)
	}
}

fn dimensions(media: &IMFMediaType) -> Result<(u32, u32), &'static str> {
	// SAFETY: Reading an integer attribute from a live media type.
	let size = unsafe { media.GetUINT64(&MF_MT_FRAME_SIZE) }.map_err(|_| INVALID)?;
	let (width, height) = ((size >> 32) as u32, size as u32);
	if width == 0
		|| height == 0
		|| width > 1920
		|| height > 1920
		|| u64::from(width) * u64::from(height) > 1920 * 1080
	{
		return Err("Inline playback supports videos up to 1080p.");
	}
	Ok((width, height))
}

fn validate_stride(width: u32, height: u32, stride: i32) -> Result<usize, &'static str> {
	let pitch = stride.unsigned_abs() as usize;
	if pitch < width as usize * 4 || pitch > MAX_BYTES / height as usize {
		return Err(INVALID);
	}
	Ok(pitch)
}

fn output_aperture(
	media: &IMFMediaType,
	width: u32,
	height: u32,
) -> Result<(u32, u32, (u32, u32)), &'static str> {
	unsafe {
		let size = media.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|_| INVALID)?;
		let (bw, bh) = ((size >> 32) as u32, size as u32);
		if bw < width
			|| bh < height
			|| bw > width.div_ceil(32) * 32
			|| bh > height.div_ceil(32) * 32
		{
			return Err(INVALID);
		}
		let mut crop = (0, 0);
		for key in [MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_GEOMETRIC_APERTURE] {
			let mut bytes = [0_u8; std::mem::size_of::<MFVideoArea>()];
			let mut size = 0;
			if media.GetBlob(&key, &mut bytes, Some(&mut size)).is_ok() {
				if size as usize != bytes.len() {
					return Err(INVALID);
				}
				// MFVideoArea contains only integer fields; the fixed-size byte copy can be unaligned.
				let area = std::ptr::read_unaligned(bytes.as_ptr().cast::<MFVideoArea>());
				if area.OffsetX.value < 0
					|| area.OffsetY.value < 0
					|| area.OffsetX.fract != 0
					|| area.OffsetY.fract != 0
					|| area.Area.cx != width as i32
					|| area.Area.cy != height as i32
				{
					return Err(INVALID);
				}
				crop = (area.OffsetX.value as u32, area.OffsetY.value as u32);
				break;
			}
		}
		if crop.0 + width > bw || crop.1 + height > bh {
			return Err(INVALID);
		}
		Ok((bw, bh, crop))
	}
}

fn rgba_frame(
	bytes: &[u8],
	width: u32,
	height: u32,
	stride: i32,
	rotation: u32,
	buffer_height: u32,
	crop: (u32, u32),
) -> Result<Vec<u8>, &'static str> {
	let pitch = validate_stride(width + crop.0, buffer_height, stride)?;
	if bytes.len() < pitch * buffer_height as usize || crop.1 + height > buffer_height {
		return Err(INVALID);
	}
	let (w, h) = (width as usize, height as usize);
	let out_width = if rotation == 90 || rotation == 270 {
		h
	} else {
		w
	};
	let mut rgba = vec![0; w * h * 4];
	for y in 0..h {
		let row = if stride < 0 {
			buffer_height as usize - 1 - y - crop.1 as usize
		} else {
			y + crop.1 as usize
		};
		for x in 0..w {
			let (dx, dy) = match rotation {
				90 => (h - 1 - y, x),
				180 => (w - 1 - x, h - 1 - y),
				270 => (y, w - 1 - x),
				_ => (x, y),
			};
			let source = row * pitch + (x + crop.0 as usize) * 4;
			let target = (dy * out_width + dx) * 4;
			rgba[target..target + 4].copy_from_slice(&[
				bytes[source + 2],
				bytes[source + 1],
				bytes[source],
				255,
			]);
		}
	}
	Ok(rgba)
}

fn sample_bytes(sample: &IMFSample) -> Result<Vec<u8>, &'static str> {
	// SAFETY: Native length is checked before allocating/combining. Lock's pointer is
	// borrowed only until Unlock, and the copy length is validated against capacity.
	unsafe {
		if sample.GetTotalLength().map_err(|_| INVALID)? as usize > MAX_BYTES {
			return Err(INVALID);
		}
		let buffer = sample.ConvertToContiguousBuffer().map_err(|_| INVALID)?;
		let mut data = std::ptr::null_mut();
		let mut capacity = 0;
		let mut length = 0;
		buffer
			.Lock(&mut data, Some(&mut capacity), Some(&mut length))
			.map_err(|_| INVALID)?;
		let result = if length > capacity
			|| length as usize > MAX_BYTES
			|| (length != 0 && data.is_null())
		{
			Err(INVALID)
		} else if length == 0 {
			Ok(Vec::new())
		} else {
			Ok(std::slice::from_raw_parts(data, length as usize).to_vec())
		};
		buffer.Unlock().map_err(|_| INVALID)?;
		result
	}
}

/// Shared bounded source, independent cursor per COM clone. All blocking reads are
/// serialized outside rendering/audio callbacks. The OS supplies async byte-stream glue.
#[implement(IStream)]
struct ReadStream {
	source: Arc<Mutex<Box<dyn ReadSeek>>>,
	position: Mutex<u64>,
	length: u64,
}
impl ISequentialStream_Impl for ReadStream_Impl {
	fn Read(&self, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
		// SAFETY: COM Read gives writable storage of cb bytes; a null pointer is
		// accepted only for an empty read. Out count is optional per ISequentialStream.
		unsafe {
			if !pcbread.is_null() {
				*pcbread = 0;
			}
		}
		if cb as usize > MAX_BYTES {
			return E_INVALIDARG;
		}
		if cb == 0 {
			return S_OK;
		}
		if pv.is_null() {
			return E_POINTER;
		}
		let Ok(mut position) = self.position.lock() else {
			return E_FAIL;
		};
		let Ok(mut source) = self.source.lock() else {
			return E_FAIL;
		};
		if source.seek(SeekFrom::Start(*position)).is_err() {
			return E_FAIL;
		}
		let count = (self.length.saturating_sub(*position)).min(u64::from(cb)) as usize;
		let bytes = unsafe { std::slice::from_raw_parts_mut(pv.cast::<u8>(), count) };
		let mut read = 0;
		while read < count {
			match source.read(&mut bytes[read..]) {
				Ok(0) => break,
				Ok(n) => read += n,
				Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
				Err(_) => {
					*position += read as u64;
					return E_FAIL;
				}
			}
		}
		*position += read as u64;
		unsafe {
			if !pcbread.is_null() {
				*pcbread = read as u32;
			}
		}
		if read == cb as usize { S_OK } else { S_FALSE }
	}
	fn Write(&self, _: *const c_void, _: u32, written: *mut u32) -> HRESULT {
		unsafe {
			if !written.is_null() {
				*written = 0;
			}
		}
		E_NOTIMPL
	}
}
impl IStream_Impl for ReadStream_Impl {
	fn Seek(
		&self,
		offset: i64,
		origin: STREAM_SEEK,
		new_position: *mut u64,
	) -> windows::core::Result<()> {
		let mut position = self
			.position
			.lock()
			.map_err(|_| windows::core::Error::from(E_FAIL))?;
		let base = match origin {
			STREAM_SEEK_SET => 0,
			STREAM_SEEK_CUR => *position,
			STREAM_SEEK_END => self.length,
			_ => return Err(E_INVALIDARG.into()),
		};
		let next = i128::from(base) + i128::from(offset);
		if next < 0 || next > i128::from(self.length) {
			return Err(E_INVALIDARG.into());
		}
		*position = next as u64;
		unsafe {
			if !new_position.is_null() {
				*new_position = *position;
			}
		}
		Ok(())
	}
	fn Stat(&self, stat: *mut STATSTG, _: &STATFLAG) -> windows::core::Result<()> {
		if stat.is_null() {
			return Err(E_POINTER.into());
		}
		unsafe {
			stat.write(STATSTG {
				r#type: STGTY_STREAM.0 as u32,
				cbSize: self.length,
				grfMode: STGM_READ,
				..Default::default()
			});
		}
		Ok(())
	}
	fn Clone(&self) -> windows::core::Result<IStream> {
		let position = *self
			.position
			.lock()
			.map_err(|_| windows::core::Error::from(E_FAIL))?;
		Ok(ReadStream {
			source: self.source.clone(),
			position: Mutex::new(position),
			length: self.length,
		}
		.into())
	}
	fn SetSize(&self, _: u64) -> windows::core::Result<()> {
		Err(E_NOTIMPL.into())
	}
	fn CopyTo(
		&self,
		_: Ref<'_, IStream>,
		_: u64,
		_: *mut u64,
		_: *mut u64,
	) -> windows::core::Result<()> {
		Err(E_NOTIMPL.into())
	}
	fn Commit(&self, _: &STGC) -> windows::core::Result<()> {
		Err(E_NOTIMPL.into())
	}
	fn Revert(&self) -> windows::core::Result<()> {
		Err(E_NOTIMPL.into())
	}
	fn LockRegion(&self, _: u64, _: u64, _: &LOCKTYPE) -> windows::core::Result<()> {
		Err(E_NOTIMPL.into())
	}
	fn UnlockRegion(&self, _: u64, _: u64, _: u32) -> windows::core::Result<()> {
		Err(E_NOTIMPL.into())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn native_mov_decodes_audio_video_and_seeks() {
		let source = std::io::Cursor::new(
			include_bytes!("../../../../apps/desktop/tests/fixtures/video.mov").as_slice(),
		);
		let mut decoder = Decoder::open(Box::new(source)).unwrap();
		let info = decoder.info();
		assert_eq!(
			(info.width, info.height, info.sample_rate),
			(320, 180, 48_000)
		);
		assert!((2.9..3.1).contains(&info.duration));
		let started = std::time::Instant::now();
		let (mut videos, mut audio_frames, mut last_pts) = (0, 0, -1.0);
		for _ in 0..1_000 {
			let video = decoder.read_video().unwrap();
			let audio = decoder.read_audio().unwrap();
			if video.is_none() && audio.is_none() {
				break;
			}
			for sample in [video, audio].into_iter().flatten() {
				match sample {
					Sample::Video {
						pts,
						rgba,
						width,
						height,
					} => {
						assert_eq!(rgba.len(), (width * height * 4) as usize);
						assert!(rgba.windows(4).any(|pixel| pixel[0] != pixel[1]));
						assert!(pts >= last_pts);
						last_pts = pts;
						videos += 1;
					}
					Sample::Audio { frames, .. } => {
						assert!(frames.iter().flatten().all(|sample| sample.is_finite()));
						audio_frames += frames.len();
					}
				}
			}
		}
		assert_eq!(videos, 72);
		assert!(audio_frames >= 140_000);
		assert!(decoder.read_video().unwrap().is_none());
		assert!(decoder.read_audio().unwrap().is_none());
		eprintln!(
			"synthetic MOV decode: 72 frames + {audio_frames} PCM frames in {:.3} ms",
			started.elapsed().as_secs_f64() * 1000.0
		);
		decoder.seek(1.0).unwrap();
		let mut reached = false;
		for _ in 0..200 {
			if let Some(Sample::Video { pts, .. }) = decoder.read_video().unwrap()
				&& pts >= 1.0
			{
				reached = true;
				break;
			}
		}
		assert!(reached);
		assert!(decoder.seek(f64::NAN).is_err());
		drop(decoder);
		// Rotate only the known synthetic fixture's video-track matrix, retaining
		// the same compressed samples. No second media fixture is needed.
		let mut portrait =
			include_bytes!("../../../../apps/desktop/tests/fixtures/video.mov").to_vec();
		let track = portrait
			.windows(4)
			.enumerate()
			.find_map(|(offset, tag)| {
				(tag == b"tkhd"
					&& offset >= 4 && offset + 88 <= portrait.len()
					&& portrait[offset + 80..offset + 84] == (320_u32 << 16).to_be_bytes())
				.then(|| offset - 4)
			})
			.unwrap();
		let matrix = [0_i32, 65536, 0, -65536, 0, 0, 0, 0, 1 << 30];
		for (word, value) in portrait[track + 48..track + 84]
			.as_chunks_mut::<4>()
			.0
			.iter_mut()
			.zip(matrix)
		{
			word.copy_from_slice(&value.to_be_bytes());
		}
		let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(portrait))).unwrap();
		assert_eq!((decoder.info().width, decoder.info().height), (180, 320));
		let Some(Sample::Video {
			width,
			height,
			rgba,
			..
		}) = decoder.read_video().unwrap()
		else {
			panic!("Missing portrait frame");
		};
		assert_eq!((width, height, rgba.len()), (180, 320, 180 * 320 * 4));
	}
	#[test]
	fn bounded_stream_cursors_and_video_rows() {
		let stream: IStream = ReadStream {
			source: Arc::new(Mutex::new(Box::new(std::io::Cursor::new(vec![1, 2, 3, 4])))),
			position: Mutex::new(0),
			length: 4,
		}
		.into();
		unsafe {
			let clone = stream.Clone().unwrap();
			stream.Seek(2, STREAM_SEEK_SET, None).unwrap();
			let mut bytes = [0_u8; 4];
			let mut count = 0;
			assert_eq!(
				stream.Read(bytes.as_mut_ptr().cast(), 4, Some(&mut count)),
				S_FALSE
			);
			assert_eq!((&bytes[..2], count), (&[3, 4][..], 2));
			assert_eq!(
				clone.Read(bytes.as_mut_ptr().cast(), 4, Some(&mut count)),
				S_OK
			);
			assert_eq!(bytes, [1, 2, 3, 4]);
			assert!(stream.Seek(-5, STREAM_SEEK_SET, None).is_err());
			assert_eq!(
				stream.Read(bytes.as_mut_ptr().cast(), MAX_BYTES as u32 + 1, None),
				E_INVALIDARG
			);
		}
		let input = [1, 2, 3, 0, 4, 5, 6, 0];
		assert_eq!(
			rgba_frame(&input, 1, 2, -4, 0, 2, (0, 0)).unwrap(),
			[6, 5, 4, 255, 3, 2, 1, 255]
		);
		assert_eq!(
			rgba_frame(&input, 1, 2, 4, 90, 2, (0, 0)).unwrap(),
			[6, 5, 4, 255, 3, 2, 1, 255]
		);
		assert!(rgba_frame(&input[..4], 1, 2, 4, 0, 2, (0, 0)).is_err());
	}
}
