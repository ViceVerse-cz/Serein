//! Windows live H.264 decoding through the Media Foundation H.264 decoder transform. A DXGI
//! device manager enables DXVA hardware decoding when the GPU offers it; without one the
//! decoder runs in software. Output samples are read back through Media Foundation, so both
//! paths deliver the same NV12 pictures, converted here to RGBA.
#![allow(unsafe_code)]

use super::{
	INVALID, UNSUPPORTED,
	media_foundation::{Runtime, dxgi_manager},
};
pub use super::{LiveFrame as Frame, LiveSink as Sink, MAX_ACCESS_UNIT};
use windows::{
	Win32::{
		Graphics::Direct3D11::ID3D11Device,
		Media::MediaFoundation::*,
		System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
	},
	core::Interface,
};

const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
/// 100 ns units per picture at a nominal 30 fps; only monotonicity matters to the decoder.
const TICKS_PER_PICTURE: i64 = 333_333;

// Field order: COM objects release before the runtime shuts down.
pub struct H264Decoder {
	transform: IMFTransform,
	_manager: Option<IMFDXGIDeviceManager>,
	_device: Option<ID3D11Device>,
	_runtime: Runtime,
	sink: Sink,
	pictures: i64,
	output: Option<OutputFormat>,
	provides_samples: bool,
}

#[derive(Clone, Copy)]
struct OutputFormat {
	width: u32,
	height: u32,
	stride: u32,
	crop: (u32, u32, u32, u32),
}

impl H264Decoder {
	pub fn new(sink: Sink) -> Result<Self, &'static str> {
		let runtime = Runtime::open()?;
		// SAFETY: Plain COM/Media Foundation calls on the thread that initialized the runtime;
		// every out-pointer refers to an initialized local.
		unsafe {
			let transform: IMFTransform =
				CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)
					.map_err(|_| UNSUPPORTED)?;
			if let Ok(attributes) = transform.GetAttributes() {
				let _ = attributes.SetUINT32(&CODECAPI_AVLowLatencyMode, 1);
			}
			let (device, manager) = hardware(&transform);
			let input = MFCreateMediaType().map_err(|_| UNSUPPORTED)?;
			input
				.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
				.map_err(|_| UNSUPPORTED)?;
			input
				.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
				.map_err(|_| UNSUPPORTED)?;
			transform
				.SetInputType(0, &input, 0)
				.map_err(|_| UNSUPPORTED)?;
			let mut decoder = Self {
				transform,
				_manager: manager,
				_device: device,
				_runtime: runtime,
				sink,
				pictures: 0,
				output: None,
				provides_samples: false,
			};
			decoder.negotiate_output()?;
			decoder
				.transform
				.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
				.map_err(|_| UNSUPPORTED)?;
			decoder
				.transform
				.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
				.map_err(|_| UNSUPPORTED)?;
			Ok(decoder)
		}
	}

	/// Queue one Annex-B access unit and deliver every picture the decoder releases.
	pub fn decode(&mut self, access_unit: &[u8]) -> Result<(), &'static str> {
		if access_unit.len() > MAX_ACCESS_UNIT || access_unit.is_empty() {
			return Err(INVALID);
		}
		// SAFETY: Buffer length is validated before the copy; Lock is balanced by Unlock.
		unsafe {
			let length = u32::try_from(access_unit.len()).map_err(|_| INVALID)?;
			let buffer = MFCreateMemoryBuffer(length).map_err(|_| INVALID)?;
			let mut data = std::ptr::null_mut();
			let mut capacity = 0;
			buffer
				.Lock(&mut data, Some(&mut capacity), None)
				.map_err(|_| INVALID)?;
			if data.is_null() || (capacity as usize) < access_unit.len() {
				let _ = buffer.Unlock();
				return Err(INVALID);
			}
			std::ptr::copy_nonoverlapping(access_unit.as_ptr(), data, access_unit.len());
			buffer.Unlock().map_err(|_| INVALID)?;
			buffer.SetCurrentLength(length).map_err(|_| INVALID)?;
			let sample = MFCreateSample().map_err(|_| INVALID)?;
			sample.AddBuffer(&buffer).map_err(|_| INVALID)?;
			self.pictures += 1;
			sample
				.SetSampleTime(self.pictures * TICKS_PER_PICTURE)
				.map_err(|_| INVALID)?;
			sample
				.SetSampleDuration(TICKS_PER_PICTURE)
				.map_err(|_| INVALID)?;
			match self.transform.ProcessInput(0, &sample, 0) {
				Ok(()) => {}
				Err(error) if error.code() == MF_E_NOTACCEPTING => {
					self.drain()?;
					self.transform
						.ProcessInput(0, &sample, 0)
						.map_err(|_| INVALID)?;
				}
				Err(_) => return Err(INVALID),
			}
		}
		self.drain()
	}

	/// Nothing is queued asynchronously; pictures are delivered inside `decode`.
	pub fn flush(&self) {}

	fn negotiate_output(&mut self) -> Result<(), &'static str> {
		// SAFETY: Media type enumeration on a live transform with validated indices.
		unsafe {
			let mut index = 0;
			loop {
				let candidate = self
					.transform
					.GetOutputAvailableType(0, index)
					.map_err(|_| UNSUPPORTED)?;
				index += 1;
				if candidate.GetGUID(&MF_MT_SUBTYPE).map_err(|_| UNSUPPORTED)? != MFVideoFormat_NV12
				{
					continue;
				}
				self.transform
					.SetOutputType(0, &candidate, 0)
					.map_err(|_| UNSUPPORTED)?;
				break;
			}
			let current = self
				.transform
				.GetOutputCurrentType(0)
				.map_err(|_| UNSUPPORTED)?;
			let size = current.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|_| INVALID)?;
			let (width, height) = ((size >> 32) as u32, size as u32);
			let stride = current
				.GetUINT32(&MF_MT_DEFAULT_STRIDE)
				.ok()
				.filter(|stride| *stride >= width)
				.unwrap_or(width);
			let mut crop = (0, 0, width, height);
			let mut bytes = [0_u8; std::mem::size_of::<MFVideoArea>()];
			let mut written = 0;
			if current
				.GetBlob(
					&MF_MT_MINIMUM_DISPLAY_APERTURE,
					&mut bytes,
					Some(&mut written),
				)
				.is_ok() && written as usize == bytes.len()
			{
				// MFVideoArea contains only integer fields; the byte copy may be unaligned.
				let area = std::ptr::read_unaligned(bytes.as_ptr().cast::<MFVideoArea>());
				let (x, y) = (
					area.OffsetX.value.max(0) as u32,
					area.OffsetY.value.max(0) as u32,
				);
				let (w, h) = (area.Area.cx.max(0) as u32, area.Area.cy.max(0) as u32);
				if w > 0 && h > 0 && x + w <= width && y + h <= height {
					crop = (x, y, w, h);
				}
			}
			super::check_dimensions(crop.2, crop.3)?;
			if (stride as usize) * (height as usize) * 3 / 2 > MAX_OUTPUT_BYTES {
				return Err(INVALID);
			}
			let info = self
				.transform
				.GetOutputStreamInfo(0)
				.map_err(|_| UNSUPPORTED)?;
			self.provides_samples = info.dwFlags
				& (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
					| MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
				!= 0;
			self.output = Some(OutputFormat {
				width,
				height,
				stride,
				crop,
			});
			Ok(())
		}
	}

	/// Pull every available picture; a format change renegotiates NV12 and continues.
	fn drain(&mut self) -> Result<(), &'static str> {
		loop {
			let format = self.output.ok_or(INVALID)?;
			// SAFETY: The output buffer struct is fully initialized; samples we allocate are
			// released with the struct, samples the transform provides are taken over below.
			let sample = unsafe {
				let own = if self.provides_samples {
					None
				} else {
					let info = self
						.transform
						.GetOutputStreamInfo(0)
						.map_err(|_| UNSUPPORTED)?;
					if info.cbSize as usize > MAX_OUTPUT_BYTES {
						return Err(INVALID);
					}
					let buffer = MFCreateMemoryBuffer(info.cbSize).map_err(|_| INVALID)?;
					let sample = MFCreateSample().map_err(|_| INVALID)?;
					sample.AddBuffer(&buffer).map_err(|_| INVALID)?;
					Some(sample)
				};
				let mut output = MFT_OUTPUT_DATA_BUFFER {
					dwStreamID: 0,
					pSample: std::mem::ManuallyDrop::new(own.clone()),
					dwStatus: 0,
					pEvents: std::mem::ManuallyDrop::new(None),
				};
				let mut status = 0;
				let result =
					self.transform
						.ProcessOutput(0, std::slice::from_mut(&mut output), &mut status);
				let provided = std::mem::ManuallyDrop::into_inner(output.pSample);
				drop(std::mem::ManuallyDrop::into_inner(output.pEvents));
				match result {
					Ok(()) => provided.or(own),
					Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
					Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
						self.negotiate_output()?;
						continue;
					}
					Err(_) => return Err(INVALID),
				}
			};
			let Some(sample) = sample else {
				return Ok(());
			};
			let bytes = sample_bytes(&sample)?;
			(self.sink)(nv12_to_rgba(&bytes, format)?);
		}
	}
}

/// Try to enable DXVA: a D3D11 video device shared with the transform. Any failure leaves
/// the decoder in software mode.
unsafe fn hardware(
	transform: &IMFTransform,
) -> (Option<ID3D11Device>, Option<IMFDXGIDeviceManager>) {
	unsafe {
		let aware = transform
			.GetAttributes()
			.and_then(|attributes| attributes.GetUINT32(&MF_SA_D3D11_AWARE))
			.unwrap_or(0);
		if aware == 0 {
			return (None, None);
		}
		let Some((device, manager)) = dxgi_manager() else {
			return (None, None);
		};
		if transform
			.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)
			.is_err()
		{
			return (None, None);
		}
		(Some(device), Some(manager))
	}
}

fn sample_bytes(sample: &IMFSample) -> Result<Vec<u8>, &'static str> {
	// SAFETY: Native length is checked before allocating; Lock's pointer is borrowed only
	// until Unlock and the copy length is validated against the reported capacity. For DXGI
	// buffers Media Foundation performs the GPU read-back inside Lock.
	unsafe {
		if sample.GetTotalLength().map_err(|_| INVALID)? as usize > MAX_OUTPUT_BYTES {
			return Err(INVALID);
		}
		let buffer = sample.ConvertToContiguousBuffer().map_err(|_| INVALID)?;
		let mut data = std::ptr::null_mut();
		let mut capacity = 0;
		let mut length = 0;
		buffer
			.Lock(&mut data, Some(&mut capacity), Some(&mut length))
			.map_err(|_| INVALID)?;
		let result = match sample_length(length, capacity, data.is_null()) {
			Err(error) => Err(error),
			Ok(0) => Ok(Vec::new()),
			Ok(length) => Ok(std::slice::from_raw_parts(data, length).to_vec()),
		};
		buffer.Unlock().map_err(|_| INVALID)?;
		result
	}
}

fn sample_length(length: u32, capacity: u32, null: bool) -> Result<usize, &'static str> {
	if length > capacity || length as usize > MAX_OUTPUT_BYTES || (length != 0 && null) {
		Err(INVALID)
	} else {
		Ok(length as usize)
	}
}

/// Convert one NV12 picture (limited-range BT.601, the WebRTC default) to packed RGBA.
fn nv12_to_rgba(bytes: &[u8], format: OutputFormat) -> Result<Frame, &'static str> {
	let (stride, height) = (format.stride as usize, format.height as usize);
	let (x0, y0, width, out_height) = (
		format.crop.0 as usize,
		format.crop.1 as usize,
		format.crop.2 as usize,
		format.crop.3 as usize,
	);
	let luma_len = stride * height;
	if x0 + width > format.width as usize
		|| y0 + out_height > height
		|| bytes.len() < luma_len + stride * height.div_ceil(2)
	{
		return Err(INVALID);
	}
	let (luma, chroma) = bytes.split_at(luma_len);
	let mut rgba = vec![0; width * out_height * 4];
	for y in 0..out_height {
		let luma_row = &luma[(y0 + y) * stride..];
		let chroma_row = &chroma[((y0 + y) / 2) * stride..];
		let out = &mut rgba[y * width * 4..(y + 1) * width * 4];
		for (x, pixel) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
			let sx = x0 + x;
			let yy = f32::from(luma_row[sx]) - 16.0;
			let u = f32::from(chroma_row[(sx / 2) * 2]) - 128.0;
			let v = f32::from(chroma_row[(sx / 2) * 2 + 1]) - 128.0;
			let r = 1.164 * yy + 1.596 * v;
			let g = 1.164 * yy - 0.392 * u - 0.813 * v;
			let b = 1.164 * yy + 2.017 * u;
			// Round to the nearest channel value: limited-range white is 254.916 here.
			*pixel = [
				r.round().clamp(0.0, 255.0) as u8,
				g.round().clamp(0.0, 255.0) as u8,
				b.round().clamp(0.0, 255.0) as u8,
				255,
			];
		}
	}
	Ok(Frame {
		width: format.crop.2,
		height: format.crop.3,
		rgba,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn native_sample_length_accepts_empty_storage_and_rejects_invalid_bounds() {
		assert_eq!(sample_length(0, 0, true), Ok(0));
		assert_eq!(sample_length(0, 16, true), Ok(0));
		assert_eq!(sample_length(0, 0, false), Ok(0));
		assert_eq!(sample_length(1, 1, true), Err(INVALID));
		assert_eq!(sample_length(2, 1, false), Err(INVALID));
		assert_eq!(sample_length(1, 1, false), Ok(1));
		assert_eq!(
			sample_length(MAX_OUTPUT_BYTES as u32 + 1, u32::MAX, false),
			Err(INVALID)
		);
	}
	#[test]
	fn nv12_conversion_crops_and_bounds() {
		let format = OutputFormat {
			width: 4,
			height: 2,
			stride: 4,
			crop: (0, 0, 2, 2),
		};
		let mut bytes = vec![235u8; 8];
		bytes.extend([128u8; 4]);
		let frame = nv12_to_rgba(&bytes, format).unwrap();
		assert_eq!((frame.width, frame.height), (2, 2));
		assert!(
			frame
				.rgba
				.as_chunks::<4>()
				.0
				.iter()
				.all(|px| *px == [255, 255, 255, 255])
		);
		assert!(nv12_to_rgba(&bytes[..6], format).is_err());
		for (luma, expected) in [(16, 0), (126, 128), (0, 0), (255, 255)] {
			bytes[..8].fill(luma);
			assert!(
				nv12_to_rgba(&bytes, format)
					.unwrap()
					.rgba
					.as_chunks::<4>()
					.0
					.iter()
					.all(|px| *px == [expected, expected, expected, 255])
			);
		}
	}
}
