//! Hardware decoding of one live H.264 elementary stream (camera or Go Live) through
//! VideoToolbox. The caller hands over complete Annex-B access units that were already
//! authenticated and decrypted; nothing here touches the network or disk.
#![allow(unsafe_code)]

use super::{INVALID, UNSUPPORTED, apple};
pub use super::{LiveFrame as Frame, LiveSink as Sink, MAX_ACCESS_UNIT};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFRetained};
use objc2_core_media::{
	CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo, CMTime, CMTimeFlags,
	CMVideoFormatDescriptionCreateFromH264ParameterSets, kCMBlockBufferAssureMemoryNowFlag,
};
use objc2_core_video::{CVImageBuffer, kCVPixelFormatType_32BGRA};
use objc2_video_toolbox::{
	VTDecodeFrameFlags, VTDecodeInfoFlags, kVTVideoDecoderBadDataErr,
	kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder,
};
use std::{
	ffi::c_void,
	ptr::{NonNull, null, null_mut},
	sync::{Arc, Mutex},
};

const TIMESCALE: i32 = 90_000;
// Drain every three submissions: at most three compressed units (3 * MAX_ACCESS_UNIT)
// can outlive the caller's bounded queue inside VideoToolbox.
const MAX_ASYNC_PICTURES: i64 = 3;

struct Output {
	sink: Sink,
	error: Option<&'static str>,
}

struct Active {
	sps: Vec<u8>,
	pps: Vec<u8>,
	_format: CFRetained<CMFormatDescription>,
	format: CFRetained<CMFormatDescription>,
	session: apple::Session,
}

/// Decoder for one sender. Parameter sets are taken from the stream itself; a change in
/// them (a new resolution, for example) transparently recreates the session. Pictures
/// arrive asynchronously through the sink so the hardware pipelines consecutive frames.
pub struct H264Decoder {
	active: Option<Active>,
	output: Arc<Mutex<Output>>,
	pictures: i64,
}

impl H264Decoder {
	pub fn new(sink: Sink) -> Result<Self, &'static str> {
		Ok(Self {
			active: None,
			output: Arc::new(Mutex::new(Output { sink, error: None })),
			pictures: 0,
		})
	}

	/// Queue one Annex-B access unit. An error from an earlier picture surfaces here, so
	/// the caller can recover with a keyframe or fall back.
	pub fn decode(&mut self, access_unit: &[u8]) -> Result<(), &'static str> {
		if access_unit.len() > MAX_ACCESS_UNIT {
			return Err(INVALID);
		}
		let mut sps = None;
		let mut pps = None;
		let mut avcc = Vec::with_capacity(access_unit.len() + 16);
		for nal in nal_units(access_unit) {
			match nal.first().map(|header| header & 0x1f) {
				Some(7) => sps = Some(nal),
				Some(8) => pps = Some(nal),
				Some(1 | 5) => {
					avcc.extend_from_slice(
						&(u32::try_from(nal.len()).map_err(|_| INVALID)?).to_be_bytes(),
					);
					avcc.extend_from_slice(nal);
				}
				_ => {}
			}
		}
		if let (Some(sps), Some(pps)) = (sps, pps)
			&& self
				.active
				.as_ref()
				.is_none_or(|active| active.sps != sps || active.pps != pps)
		{
			self.active = None;
			let format = format_description(sps, pps)?;
			let refcon = Arc::as_ptr(&self.output).cast_mut().cast::<c_void>();
			// BGRA is the output every VideoToolbox decoder provides; RGBA sessions are created
			// but fail on the first picture.
			// SAFETY: The static key is valid; the dictionary lives through session creation.
			let specification = unsafe {
				CFDictionary::from_slices(
					&[kVTVideoDecoderSpecification_RequireHardwareAcceleratedVideoDecoder],
					&[CFBoolean::new(true)],
				)
			};
			let session = apple::create_session(
				&format,
				refcon,
				output_frame,
				kCVPixelFormatType_32BGRA,
				Some(specification.as_opaque()),
			)?;
			self.active = Some(Active {
				sps: sps.to_vec(),
				pps: pps.to_vec(),
				_format: format.clone(),
				format,
				session,
			});
		}
		let Some(active) = &self.active else {
			return Err("H264 parameter sets have not arrived yet");
		};
		if avcc.is_empty() {
			return Ok(());
		}
		// Keep a short hardware pipeline without accumulating seconds of native backlog.
		// Waiting runs on the decoder worker, never on the media transport or UI thread.
		if self.pictures % MAX_ASYNC_PICTURES == 0 {
			self.flush();
		}
		if let Some(error) = self.output.lock().map_err(|_| INVALID)?.error.take() {
			return Err(error);
		}
		self.pictures += 1;
		let time = |value: i64| CMTime {
			value,
			timescale: TIMESCALE,
			flags: CMTimeFlags::Valid,
			epoch: 0,
		};
		let timing = CMSampleTimingInfo {
			duration: time(0),
			presentationTimeStamp: time(self.pictures * 3000),
			decodeTimeStamp: time(self.pictures * 3000),
		};
		// SAFETY: Every out-pointer refers to an initialized local. The block buffer owns a
		// private copy of the bytes; the output slot outlives the session (field order).
		let status = unsafe {
			let mut block: *mut CMBlockBuffer = null_mut();
			let status = CMBlockBuffer::create_with_memory_block(
				None,
				null_mut(),
				avcc.len(),
				None,
				null(),
				0,
				avcc.len(),
				kCMBlockBufferAssureMemoryNowFlag,
				NonNull::from(&mut block),
			);
			let block = NonNull::new(block)
				.filter(|_| status == 0)
				.map(|ptr| CFRetained::from_raw(ptr))
				.ok_or(INVALID)?;
			if CMBlockBuffer::replace_data_bytes(
				NonNull::from(avcc.as_slice()).cast::<c_void>(),
				&block,
				0,
				avcc.len(),
			) != 0
			{
				return Err(INVALID);
			}
			let sizes = [avcc.len()];
			let mut sample: *mut CMSampleBuffer = null_mut();
			let status = CMSampleBuffer::create_ready(
				None,
				Some(&block),
				Some(&active.format),
				1,
				1,
				&timing,
				1,
				sizes.as_ptr(),
				NonNull::from(&mut sample),
			);
			let sample = NonNull::new(sample)
				.filter(|_| status == 0)
				.map(|ptr| CFRetained::from_raw(ptr))
				.ok_or(INVALID)?;
			let mut flags = VTDecodeInfoFlags(0);
			// Asynchronous decode: pictures reach the sink from VideoToolbox's own thread.
			active.session.0.decode_frame(
				&sample,
				VTDecodeFrameFlags::Frame_EnableAsynchronousDecompression,
				null_mut(),
				&mut flags,
			)
		};
		match status {
			0 => Ok(()),
			status if status == kVTVideoDecoderBadDataErr => Err(INVALID),
			_ => Err(UNSUPPORTED),
		}
	}

	/// Block until every queued picture has reached the sink.
	pub fn flush(&self) {
		if let Some(active) = &self.active {
			// SAFETY: the session is live for as long as `active` exists.
			unsafe {
				active.session.0.wait_for_asynchronous_frames();
			}
		}
	}
}

fn nal_units(frame: &[u8]) -> Vec<&[u8]> {
	let mut starts = Vec::new();
	let mut at = 0;
	while at + 3 <= frame.len() {
		let size = if frame[at..].starts_with(&[0, 0, 0, 1]) {
			4
		} else if frame[at..].starts_with(&[0, 0, 1]) {
			3
		} else {
			at += 1;
			continue;
		};
		starts.push((at, size));
		at += size;
		if starts.len() > 512 {
			break;
		}
	}
	starts
		.iter()
		.enumerate()
		.filter_map(|(index, (start, size))| {
			let end = starts.get(index + 1).map_or(frame.len(), |next| next.0);
			(start + size < end).then(|| &frame[start + size..end])
		})
		.collect()
}

fn format_description(
	sps: &[u8],
	pps: &[u8],
) -> Result<CFRetained<CMFormatDescription>, &'static str> {
	let mut pointers = [
		NonNull::from(sps).cast::<u8>(),
		NonNull::from(pps).cast::<u8>(),
	];
	let mut sizes = [sps.len(), pps.len()];
	let mut out: *const CMFormatDescription = null();
	// SAFETY: Parameter-set pointers and sizes stay live for the call; `out` is initialized.
	let status = unsafe {
		CMVideoFormatDescriptionCreateFromH264ParameterSets(
			None,
			2,
			NonNull::from(&mut pointers).cast::<NonNull<u8>>(),
			NonNull::from(&mut sizes).cast::<usize>(),
			4,
			NonNull::from(&mut out),
		)
	};
	if status != 0 {
		return Err(UNSUPPORTED);
	}
	// SAFETY: Create functions return a +1 reference on success.
	NonNull::new(out.cast_mut())
		.map(|ptr| unsafe { CFRetained::from_raw(ptr) })
		.ok_or(UNSUPPORTED)
}

unsafe extern "C-unwind" fn output_frame(
	refcon: *mut c_void,
	_frame_refcon: *mut c_void,
	status: i32,
	flags: VTDecodeInfoFlags,
	image: *mut CVImageBuffer,
	_pts: CMTime,
	_duration: CMTime,
) {
	if refcon.is_null() {
		return;
	}
	// SAFETY: `refcon` is the `Mutex<Output>` owned by the live decoder; sessions are
	// invalidated before that allocation is released.
	let output = unsafe { &*refcon.cast::<Mutex<Output>>() };
	let Ok(mut output) = output.lock() else {
		return;
	};
	if status != 0 {
		output.error = Some(if status == kVTVideoDecoderBadDataErr {
			INVALID
		} else {
			UNSUPPORTED
		});
		return;
	}
	if image.is_null() || flags.contains(VTDecodeInfoFlags::FrameDropped) {
		return;
	}
	// SAFETY: VideoToolbox keeps the image buffer alive for the duration of the callback.
	match unsafe { apple::copy_rgba(&*image) } {
		Ok((width, height, rgba)) => (output.sink)(Frame {
			width,
			height,
			rgba,
		}),
		Err(error) => output.error = Some(error),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn access_units_without_parameter_sets_are_rejected_and_nal_parsing_is_bounded() {
		let mut decoder = H264Decoder::new(Box::new(|_| {})).unwrap();
		assert!(decoder.decode(&[0, 0, 0, 1, 0x65, 1, 2, 3]).is_err());
		assert!(decoder.decode(&vec![0; MAX_ACCESS_UNIT + 1]).is_err());
		let nals = nal_units(&[0, 0, 0, 1, 0x67, 9, 0, 0, 1, 0x68, 8, 0, 0, 1]);
		assert_eq!(nals, vec![&[0x67, 9][..], &[0x68, 8][..]]);
	}
}
