//! Windows live H.264 decoding through the Media Foundation H.264 decoder transform. A DXGI
//! device manager enables DXVA hardware decoding when the GPU offers it; without one the
//! decoder runs in software. Output samples are read back through Media Foundation, so both
//! paths deliver the same NV12 pictures, cropped here and converted to RGB by the renderer.
#![allow(unsafe_code)]

use super::{
	Color, INVALID, Layout, Picture, UNSUPPORTED,
	media_foundation::{Runtime, dxgi_manager},
	picture::pack_plane,
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
			(self.sink)(crop_nv12(&bytes, format)?);
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
		let result = if length > capacity
			|| length as usize > MAX_OUTPUT_BYTES
			|| (length != 0 && data.is_null())
		{
			Err(INVALID)
		} else {
			Ok(std::slice::from_raw_parts(data, length as usize).to_vec())
		};
		buffer.Unlock().map_err(|_| INVALID)?;
		result
	}
}

/// Crop one strided NV12 picture to its display aperture as tightly packed NV12. The
/// colour stays limited-range BT.601, the WebRTC default.
fn crop_nv12(bytes: &[u8], format: OutputFormat) -> Result<Frame, &'static str> {
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
	let (odd_x, odd_y) = (x0 % 2, y0 % 2);
	if odd_x + odd_y > 0 {
		// H.264 4:2:0 crops in two-sample units, so decoders should not report this. Packed
		// NV12 cannot start between chroma pairs: convert the enclosing even aperture on the
		// CPU and trim the extra leading column/row instead of misaligning chroma.
		let aligned = crop_nv12(
			bytes,
			OutputFormat {
				crop: (
					format.crop.0 - odd_x as u32,
					format.crop.1 - odd_y as u32,
					format.crop.2 + odd_x as u32,
					format.crop.3 + odd_y as u32,
				),
				..format
			},
		)?;
		let (aligned_width, _, rgba) = aligned.to_rgba().ok_or(INVALID)?;
		let mut data = Vec::with_capacity(width * out_height * 4);
		for row in rgba.chunks_exact(aligned_width as usize * 4).skip(odd_y) {
			data.extend_from_slice(&row[odd_x * 4..]);
		}
		let picture = Picture {
			width: format.crop.2,
			height: format.crop.3,
			layout: Layout::Rgba,
			color: Color::WEBRTC,
			rotation: 0,
			data,
		};
		return picture.is_valid().then_some(picture).ok_or(INVALID);
	}
	let (luma, chroma) = bytes.split_at(luma_len);
	let len = Layout::Nv12
		.len(format.crop.2, format.crop.3)
		.ok_or(INVALID)?;
	let mut data = Vec::with_capacity(len);
	let (chroma_x, chroma_y) = (x0, y0 / 2);
	pack_plane(
		&mut data,
		&luma[y0 * stride + x0..],
		stride,
		width,
		out_height,
	)
	.and_then(|()| {
		pack_plane(
			&mut data,
			&chroma[chroma_y * stride + chroma_x..],
			stride,
			width.div_ceil(2) * 2,
			out_height.div_ceil(2),
		)
	})
	.ok_or(INVALID)?;
	let picture = Picture {
		width: format.crop.2,
		height: format.crop.3,
		layout: Layout::Nv12,
		color: Color::WEBRTC,
		rotation: 0,
		data,
	};
	picture.is_valid().then_some(picture).ok_or(INVALID)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn nv12_crops_to_the_aperture_and_bounds() {
		let format = OutputFormat {
			width: 4,
			height: 2,
			stride: 4,
			crop: (0, 0, 2, 2),
		};
		let mut bytes = vec![235u8; 8];
		bytes.extend([128u8, 128, 7, 7]);
		let frame = crop_nv12(&bytes, format).unwrap();
		assert_eq!((frame.width, frame.height), (2, 2));
		assert_eq!(frame.data, [235, 235, 235, 235, 128, 128]);
		let (_, _, rgba) = frame.to_rgba().unwrap();
		assert!(
			rgba.as_chunks::<4>()
				.0
				.iter()
				.all(|px| *px == [255, 255, 255, 255])
		);
		assert!(crop_nv12(&bytes[..6], format).is_err());
		let shifted = OutputFormat {
			crop: (2, 0, 2, 2),
			..format
		};
		assert_eq!(&crop_nv12(&bytes, shifted).unwrap().data[4..], &[7, 7]);
		// An odd origin keeps chroma aligned through the RGBA fallback.
		let odd = OutputFormat {
			crop: (1, 0, 2, 2),
			..format
		};
		let picture = crop_nv12(&bytes, odd).unwrap();
		assert_eq!(
			(picture.layout, picture.width, picture.height),
			(Layout::Rgba, 2, 2)
		);
		let (_, _, aligned) = crop_nv12(
			&bytes,
			OutputFormat {
				crop: (0, 0, 3, 2),
				..format
			},
		)
		.unwrap()
		.to_rgba()
		.unwrap();
		assert_eq!(&picture.data[..8], &aligned[4..12]);
		assert_eq!(&picture.data[8..], &aligned[16..24]);
	}
}
