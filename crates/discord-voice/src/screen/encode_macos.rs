//! Bounded VideoToolbox hardware H.264 encoder for macOS screen sharing.
//!
//! Captured BGRA pixels are copied into a CVPixelBuffer and handed to a compression session
//! that requires the hardware encoder. Output arrives in AVCC form (length-prefixed NAL units
//! plus out-of-band parameter sets) and is rewritten to the Annex B byte stream the rest of
//! the pipeline packetizes, with SPS/PPS prepended to every keyframe.
#![allow(unsafe_code)]

use super::{MAX_ENCODED_BYTES, Settings};
use objc2_core_foundation::{
	CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType,
};
use objc2_core_media::{
	CMFormatDescription, CMSampleBuffer, CMTime, CMTimeFlags,
	CMVideoFormatDescriptionGetH264ParameterSetAtIndex, kCMSampleAttachmentKey_NotSync,
	kCMTimeInvalid, kCMVideoCodecType_H264,
};
use objc2_core_video::{
	CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
	CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
	kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferPixelFormatTypeKey,
	kCVPixelFormatType_32BGRA,
};
use objc2_video_toolbox::{
	VTCompressionSession, VTEncodeInfoFlags, VTSession, VTSessionSetProperty,
	kVTCompressionPropertyKey_AllowFrameReordering,
	kVTCompressionPropertyKey_AllowTemporalCompression, kVTCompressionPropertyKey_AverageBitRate,
	kVTCompressionPropertyKey_DataRateLimits, kVTCompressionPropertyKey_ExpectedFrameRate,
	kVTCompressionPropertyKey_MaxKeyFrameInterval, kVTCompressionPropertyKey_ProfileLevel,
	kVTCompressionPropertyKey_RealTime, kVTEncodeFrameOptionKey_ForceKeyFrame,
	kVTProfileLevel_H264_Main_AutoLevel,
	kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder,
};
use std::{
	ffi::c_void,
	ptr::{NonNull, null_mut},
	sync::{Arc, Mutex},
};

const UNAVAILABLE: &str = "macOS hardware video encoding is unavailable";
const FAILED: &str = "macOS hardware video encoding failed";
const TIMESCALE: i32 = 90_000;

/// Shared with the VideoToolbox output callback: the latest encoded picture or a failure.
#[derive(Default)]
struct Output {
	/// Annex B frame and whether it is a keyframe; `None` while a frame is dropped or pending.
	frame: Option<(Vec<u8>, bool)>,
	failed: bool,
}

struct Session(CFRetained<VTCompressionSession>);

impl Drop for Session {
	fn drop(&mut self) {
		// SAFETY: Invalidate stops callbacks before the shared output slot is released.
		unsafe { self.0.invalidate() };
	}
}

// Field order matters: the session must invalidate before `output` drops.
pub(super) struct Encoder {
	session: Session,
	output: Arc<Mutex<Output>>,
	settings: Settings,
	frame: i64,
}

impl Encoder {
	pub(super) fn new(settings: Settings) -> Result<Self, &'static str> {
		let width = i32::try_from(settings.width).map_err(|_| UNAVAILABLE)?;
		let height = i32::try_from(settings.height).map_err(|_| UNAVAILABLE)?;
		let output = Arc::new(Mutex::new(Output::default()));
		// SAFETY: Static keys are valid CFStrings; both dictionaries are plain attribute maps.
		let specification = unsafe {
			CFDictionary::from_slices(
				&[kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder],
				&[CFBoolean::new(true)],
			)
		};
		let pixel_format = CFNumber::new_i32(kCVPixelFormatType_32BGRA as i32);
		let source_attributes = unsafe {
			CFDictionary::from_slices(&[kCVPixelBufferPixelFormatTypeKey], &[&*pixel_format])
		};
		let mut session: *mut VTCompressionSession = null_mut();
		// SAFETY: `refcon` points at the encoder's `Arc<Mutex<Output>>`, which outlives the
		// session (see field order); the callback only touches that slot.
		let status = unsafe {
			VTCompressionSession::create(
				None,
				width,
				height,
				kCMVideoCodecType_H264,
				Some(specification.as_opaque()),
				Some(source_attributes.as_opaque()),
				None,
				Some(output_frame),
				Arc::as_ptr(&output).cast_mut().cast::<c_void>(),
				NonNull::from(&mut session),
			)
		};
		if status != 0 {
			return Err(UNAVAILABLE);
		}
		// SAFETY: Create returns a +1 reference on success.
		let session = NonNull::new(session)
			.map(|ptr| Session(unsafe { CFRetained::from_raw(ptr) }))
			.ok_or(UNAVAILABLE)?;
		let encoder = Self {
			session,
			output,
			settings,
			frame: 0,
		};
		encoder.configure()?;
		// SAFETY: The session is valid and fully configured.
		if unsafe { encoder.session.0.prepare_to_encode_frames() } != 0 {
			return Err(UNAVAILABLE);
		}
		Ok(encoder)
	}

	fn configure(&self) -> Result<(), &'static str> {
		let settings = self.settings;
		let bit_rate =
			CFNumber::new_i32(i32::try_from(settings.bit_rate()).map_err(|_| UNAVAILABLE)?);
		let fps = CFNumber::new_i32(i32::try_from(settings.fps).map_err(|_| UNAVAILABLE)?);
		let gop = CFNumber::new_i32(i32::try_from(settings.fps * 2).map_err(|_| UNAVAILABLE)?);
		// Cap bursts at 1.5x the average over any one-second window so a keyframe on a busy
		// desktop cannot balloon past the transport's per-frame limit.
		let bytes_per_second = CFNumber::new_i32(
			i32::try_from(settings.bit_rate() / 8 * 3 / 2).map_err(|_| UNAVAILABLE)?,
		);
		let one_second = CFNumber::new_f64(1.0);
		let limits = CFArray::from_objects(&[&*bytes_per_second, &*one_second]);
		// SAFETY: Property keys and the profile level are static CFStrings exported by
		// VideoToolbox; each value has the documented type for its key.
		unsafe {
			self.set(kVTCompressionPropertyKey_RealTime, CFBoolean::new(true))?;
			self.set(
				kVTCompressionPropertyKey_AllowFrameReordering,
				CFBoolean::new(false),
			)?;
			self.set(
				kVTCompressionPropertyKey_AllowTemporalCompression,
				CFBoolean::new(true),
			)?;
			self.set(
				kVTCompressionPropertyKey_ProfileLevel,
				kVTProfileLevel_H264_Main_AutoLevel,
			)?;
			self.set(kVTCompressionPropertyKey_AverageBitRate, &bit_rate)?;
			self.set(kVTCompressionPropertyKey_ExpectedFrameRate, &fps)?;
			self.set(kVTCompressionPropertyKey_MaxKeyFrameInterval, &gop)?;
			// Rate limits are advisory on some encoders; failing to set them is not fatal.
			let _ = self.set(kVTCompressionPropertyKey_DataRateLimits, limits.as_opaque());
		}
		Ok(())
	}

	fn set(&self, key: &CFString, value: &CFType) -> Result<(), &'static str> {
		// SAFETY: A compression session is a VTSession; the key and value are valid CF objects
		// of the documented types for each property.
		let status = unsafe {
			VTSessionSetProperty(
				NonNull::from(&*self.session.0).cast::<VTSession>().as_ref(),
				key,
				Some(value),
			)
		};
		(status == 0).then_some(()).ok_or(UNAVAILABLE)
	}

	/// Encodes one BGRA picture of the configured size. Returns an empty frame when the
	/// encoder dropped the picture to hold its rate.
	pub(super) fn encode(
		&mut self,
		pixels: &[u8],
		dimensions: (usize, usize),
		force_keyframe: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let (width, height) = dimensions;
		let row_bytes = width.checked_mul(4).ok_or(FAILED)?;
		if width != self.settings.width as usize
			|| height != self.settings.height as usize
			|| pixels.len() != row_bytes.checked_mul(height).ok_or(FAILED)?
		{
			return Err(FAILED);
		}
		let picture = self.picture(pixels, width, height, row_bytes)?;
		let time = |value: i64| CMTime {
			value,
			timescale: TIMESCALE,
			flags: CMTimeFlags::Valid,
			epoch: 0,
		};
		let duration = i64::from(TIMESCALE) / i64::from(self.settings.fps);
		let pts = time(self.frame * duration);
		self.frame += 1;
		let force = force_keyframe.then(|| {
			// SAFETY: The static key is a valid CFString; the value is a CFBoolean.
			unsafe {
				CFDictionary::from_slices(
					&[kVTEncodeFrameOptionKey_ForceKeyFrame],
					&[CFBoolean::new(true)],
				)
			}
		});
		if let Ok(mut output) = self.output.lock() {
			output.frame = None;
		}
		let mut flags = VTEncodeInfoFlags(0);
		// SAFETY: The pixel buffer and option dictionary outlive the call. CompleteFrames
		// blocks until the output callback has run for this picture, so the encode is
		// synchronous from the caller's point of view.
		let status = unsafe {
			let status = self.session.0.encode_frame(
				&picture,
				pts,
				time(duration),
				force.as_deref().map(CFDictionary::as_opaque),
				null_mut(),
				&mut flags,
			);
			if status != 0 {
				return Err(FAILED);
			}
			self.session.0.complete_frames(kCMTimeInvalid)
		};
		if status != 0 {
			return Err(FAILED);
		}
		let mut output = self.output.lock().map_err(|_| FAILED)?;
		if output.failed {
			return Err(FAILED);
		}
		Ok(output.frame.take().unwrap_or_default())
	}

	fn picture(
		&self,
		pixels: &[u8],
		width: usize,
		height: usize,
		row_bytes: usize,
	) -> Result<CFRetained<CVPixelBuffer>, &'static str> {
		// An empty IOSurface property dictionary asks for GPU-shareable backing.
		let surface = CFDictionary::<CFString, CFType>::from_slices(&[], &[]);
		// SAFETY: The static key is a valid CFString; the dictionary is a plain attribute map.
		let attributes = unsafe {
			CFDictionary::from_slices(&[kCVPixelBufferIOSurfacePropertiesKey], &[&*surface])
		};
		let mut buffer: *mut CVPixelBuffer = null_mut();
		// SAFETY: The out-pointer refers to an initialized local; Create returns +1 on success.
		let status = unsafe {
			CVPixelBufferCreate(
				None,
				width,
				height,
				kCVPixelFormatType_32BGRA,
				Some(attributes.as_opaque()),
				NonNull::from(&mut buffer),
			)
		};
		let buffer = NonNull::new(buffer)
			.filter(|_| status == 0)
			.map(|ptr| unsafe { CFRetained::from_raw(ptr) })
			.ok_or(FAILED)?;
		// SAFETY: Lock is balanced by Unlock; the copy stays within the buffer's declared
		// stride and height, and every destination row is at least `row_bytes` wide.
		unsafe {
			if CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags(0)) != 0 {
				return Err(FAILED);
			}
			let stride = CVPixelBufferGetBytesPerRow(&buffer);
			let base = CVPixelBufferGetBaseAddress(&buffer);
			let result = if base.is_null() || stride < row_bytes {
				Err(FAILED)
			} else {
				let destination =
					std::slice::from_raw_parts_mut(base.cast::<u8>(), stride * height);
				for (source, target) in pixels
					.chunks_exact(row_bytes)
					.zip(destination.chunks_exact_mut(stride))
				{
					target[..row_bytes].copy_from_slice(source);
				}
				Ok(())
			};
			CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags(0));
			result?;
		}
		Ok(buffer)
	}
}

unsafe extern "C-unwind" fn output_frame(
	refcon: *mut c_void,
	_frame_refcon: *mut c_void,
	status: i32,
	flags: VTEncodeInfoFlags,
	sample: *mut CMSampleBuffer,
) {
	// SAFETY: `refcon` is the encoder's `Arc<Mutex<Output>>`, alive until the session is
	// invalidated (see `Encoder` field order). The sample buffer is borrowed for this call.
	let output = unsafe { &*refcon.cast::<Mutex<Output>>() };
	let Ok(mut output) = output.lock() else {
		return;
	};
	if status != 0 {
		output.failed = true;
		return;
	}
	if flags.contains(VTEncodeInfoFlags::FrameDropped) || sample.is_null() {
		return;
	}
	// SAFETY: Non-null per the check above and valid for the duration of the callback.
	match unsafe { annex_b(&*sample) } {
		Ok(frame) => output.frame = Some(frame),
		Err(_) => output.failed = true,
	}
}

/// Rewrites one AVCC sample as an Annex B frame, prepending parameter sets to keyframes.
unsafe fn annex_b(sample: &CMSampleBuffer) -> Result<(Vec<u8>, bool), &'static str> {
	// SAFETY: Accessors on a live sample buffer; every out-pointer is an initialized local.
	unsafe {
		let keyframe = is_sync(sample);
		let format = sample.format_description().ok_or(FAILED)?;
		let (parameter_sets, header_length) = parameter_sets(&format)?;
		let block = sample.data_buffer().ok_or(FAILED)?;
		let total = block.data_length();
		if total == 0 || total > MAX_ENCODED_BYTES {
			return Err(FAILED);
		}
		let mut avcc = vec![0u8; total];
		if block.copy_data_bytes(
			0,
			total,
			NonNull::from(avcc.as_mut_slice()).cast::<c_void>(),
		) != 0
		{
			return Err(FAILED);
		}
		let mut frame = Vec::with_capacity(
			total
				+ parameter_sets
					.iter()
					.map(|set| set.len() + 4)
					.sum::<usize>(),
		);
		if keyframe {
			for set in &parameter_sets {
				frame.extend_from_slice(&[0, 0, 0, 1]);
				frame.extend_from_slice(set);
			}
		}
		let mut at = 0;
		while at < avcc.len() {
			let header = avcc.get(at..at + header_length).ok_or(FAILED)?;
			let length = header
				.iter()
				.fold(0usize, |length, &byte| length << 8 | usize::from(byte));
			at += header_length;
			let nal = avcc
				.get(at..at + length)
				.filter(|nal| !nal.is_empty())
				.ok_or(FAILED)?;
			frame.extend_from_slice(&[0, 0, 0, 1]);
			frame.extend_from_slice(nal);
			at += length;
		}
		if frame.len() > MAX_ENCODED_BYTES {
			return Err(FAILED);
		}
		crate::video::validate_source(&frame).map_err(|_| FAILED)?;
		let keyframe = keyframe && crate::video_receive::is_keyframe(&frame);
		Ok((frame, keyframe))
	}
}

unsafe fn is_sync(sample: &CMSampleBuffer) -> bool {
	// SAFETY: The attachments array holds one CFDictionary per sample; a missing NotSync
	// attachment means the sample is a sync sample.
	unsafe {
		let Some(attachments) = sample.sample_attachments_array(false) else {
			return true;
		};
		if attachments.count() == 0 {
			return true;
		}
		let dictionary = attachments.value_at_index(0).cast::<CFDictionary>();
		if dictionary.is_null() {
			return true;
		}
		let key: *const CFString = kCMSampleAttachmentKey_NotSync;
		let not_sync = (*dictionary)
			.value(key.cast::<c_void>())
			.cast::<CFBoolean>();
		not_sync.is_null() || !(*not_sync).value()
	}
}

unsafe fn parameter_sets(
	format: &CMFormatDescription,
) -> Result<(Vec<Vec<u8>>, usize), &'static str> {
	// SAFETY: Out-pointers are initialized locals; returned parameter set memory is owned by
	// the format description and copied before this function returns.
	unsafe {
		let mut count = 0usize;
		let mut header_length = 0i32;
		let mut pointer: *const u8 = std::ptr::null();
		let mut size = 0usize;
		if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
			format,
			0,
			&mut pointer,
			&mut size,
			&mut count,
			&mut header_length,
		) != 0
		{
			return Err(FAILED);
		}
		let header_length = usize::try_from(header_length)
			.ok()
			.filter(|length| (1..=4).contains(length))
			.ok_or(FAILED)?;
		let mut sets = Vec::with_capacity(count.min(8));
		for index in 0..count.min(8) {
			let mut pointer: *const u8 = std::ptr::null();
			let mut size = 0usize;
			if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
				format,
				index,
				&mut pointer,
				&mut size,
				null_mut(),
				null_mut(),
			) != 0 || pointer.is_null()
				|| size == 0 || size > 4096
			{
				return Err(FAILED);
			}
			sets.push(std::slice::from_raw_parts(pointer, size).to_vec());
		}
		Ok((sets, header_length))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::screen::SourceId;

	#[test]
	fn encodes_annex_b_keyframes_when_hardware_is_present() {
		let settings = Settings {
			source: SourceId::Display(1),
			width: 1280,
			height: 720,
			fps: 30,
			cursor: false,
			audio: false,
		};
		// Virtualized CI runners have no hardware encoder; only verify the stream when one is.
		let Ok(mut encoder) = Encoder::new(settings) else {
			return;
		};
		let mut pixels = vec![0u8; 1280 * 720 * 4];
		for (index, pixel) in pixels.as_chunks_mut::<4>().0.iter_mut().enumerate() {
			*pixel = [(index % 251) as u8, (index / 1280 % 253) as u8, 90, 255];
		}
		let mut saw_keyframe = false;
		let mut saw_delta = false;
		for frame in 0..8 {
			let (data, keyframe) = encoder
				.encode(&pixels, (1280, 720), frame == 4)
				.expect("hardware encode");
			if data.is_empty() {
				continue;
			}
			assert!(data.starts_with(&[0, 0, 0, 1]));
			crate::video::validate_source(&data).expect("valid Annex B");
			assert_eq!(keyframe, crate::video_receive::is_keyframe(&data));
			if keyframe {
				// Parameter sets precede every IDR so a late viewer can decode it alone.
				assert_eq!(data[4] & 0x1f, 7, "SPS first");
				saw_keyframe = true;
			} else {
				saw_delta = true;
			}
			if frame == 4 {
				assert!(keyframe, "forced keyframe honored");
			}
		}
		assert!(saw_keyframe && saw_delta);
		assert!(encoder.encode(&pixels[..1000], (1280, 720), false).is_err());
	}
}
