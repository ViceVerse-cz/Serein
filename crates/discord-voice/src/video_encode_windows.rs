//! Bounded Media Foundation hardware H.264 encoder for Windows screen sharing and camera video.
#![allow(unsafe_code)]

use super::i420_to_nv12;
use super::{Config, Profile};
use std::{
	marker::PhantomData,
	rc::Rc,
	time::{Duration, Instant},
};
use windows::{
	Win32::{
		Media::MediaFoundation::*,
		System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
		System::Variant::VARIANT,
	},
	core::Interface,
};

const UNAVAILABLE: &str = "Windows hardware video encoding is unavailable";
const FAILED: &str = "Windows hardware video encoding failed";

struct Runtime(PhantomData<Rc<()>>);

impl Runtime {
	fn open() -> Result<Self, &'static str> {
		// SAFETY: The screen encoder owns this worker thread and balances both calls in Drop.
		unsafe {
			CoInitializeEx(None, COINIT_MULTITHREADED)
				.ok()
				.map_err(|_| UNAVAILABLE)?;
			if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
				CoUninitialize();
				return Err(UNAVAILABLE);
			}
		}
		Ok(Self(PhantomData))
	}
}

impl Drop for Runtime {
	fn drop(&mut self) {
		// SAFETY: Balanced with Runtime::open on the same worker thread.
		unsafe {
			let _ = MFShutdown();
			CoUninitialize();
		}
	}
}

pub(crate) struct Encoder {
	transform: IMFTransform,
	events: IMFMediaEventGenerator,
	codec: ICodecAPI,
	_activate: Activated,
	_runtime: Runtime,
	frame: i64,
	duration: i64,
	need_input: usize,
	have_output: usize,
	force_keyframe_pending: bool,
	provides_samples: bool,
	max_bytes: usize,
	max_buffer_bytes: usize,
}

struct Activated(IMFActivate);

impl Drop for Activated {
	fn drop(&mut self) {
		// SAFETY: The activation and its object stay on the Media Foundation worker.
		unsafe {
			let _ = self.0.ShutdownObject();
		}
	}
}

impl Encoder {
	#[allow(dead_code)] // Legacy standalone/native fixture entry point.
	pub(crate) fn new(config: Config) -> Result<Self, &'static str> {
		Self::new_on_adapter(config, None)
	}

	pub(crate) fn new_on_adapter(
		config: Config,
		adapter: Option<model::VideoAdapter>,
	) -> Result<Self, &'static str> {
		let runtime = Runtime::open()?;
		// SAFETY: Media Foundation owns returned COM objects; the activation array is cleared
		// before its CoTaskMem allocation is released.
		unsafe {
			let activate = Activated(hardware_encoder(adapter)?);
			let transform: IMFTransform = activate.0.ActivateObject().map_err(|_| UNAVAILABLE)?;
			let attributes = transform.GetAttributes().map_err(|_| UNAVAILABLE)?;
			if attributes.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) == 0 {
				return Err(UNAVAILABLE);
			}
			attributes
				.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)
				.map_err(|_| UNAVAILABLE)?;
			let codec: ICodecAPI = transform.cast().map_err(|_| UNAVAILABLE)?;
			let _ = codec.SetValue(
				&CODECAPI_AVEncCommonRateControlMode,
				&VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32),
			);
			let _ = codec.SetValue(
				&CODECAPI_AVEncCommonMeanBitRate,
				&VARIANT::from(config.bit_rate),
			);
			let _ = codec.SetValue(
				&CODECAPI_AVEncMPVDefaultBPictureCount,
				&VARIANT::from(0_u32),
			);
			let _ = codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(config.fps * 2));

			let output = video_type(config, MFVideoFormat_H264)?;
			output
				.SetUINT32(&MF_MT_AVG_BITRATE, config.bit_rate)
				.map_err(|_| UNAVAILABLE)?;
			// Advisory: an encoder that rejects the attribute keeps its own default profile.
			let _ = output.SetUINT32(
				&MF_MT_MPEG2_PROFILE,
				match config.profile {
					Profile::Baseline => eAVEncH264VProfile_Base.0 as u32,
					Profile::Main => eAVEncH264VProfile_Main.0 as u32,
				},
			);
			transform
				.SetOutputType(0, &output, 0)
				.map_err(|_| UNAVAILABLE)?;

			let input = video_type(config, MFVideoFormat_NV12)?;
			input
				.SetUINT32(&MF_MT_DEFAULT_STRIDE, config.width)
				.map_err(|_| UNAVAILABLE)?;
			transform
				.SetInputType(0, &input, 0)
				.map_err(|_| UNAVAILABLE)?;

			let info = transform.GetOutputStreamInfo(0).map_err(|_| UNAVAILABLE)?;
			// cbSize is minimum output-buffer capacity, not the encoded sample length.
			// Permit a bounded raw-picture-sized allocation while keeping the transport's
			// compressed frame cap enforced by sample_bytes after ProcessOutput.
			let max_buffer_bytes = (config.width as usize)
				.checked_mul(config.height as usize)
				.and_then(|pixels| pixels.checked_mul(4))
				.filter(|bytes| *bytes <= crate::screen::MAX_RAW_BYTES)
				.ok_or(UNAVAILABLE)?
				.max(config.max_bytes);
			let provides_samples = info.dwFlags
				& (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
					| MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
				!= 0;
			if !provides_samples {
				output_buffer_size(&info, max_buffer_bytes)?;
			}
			let events = transform.cast().map_err(|_| UNAVAILABLE)?;
			transform
				.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
				.map_err(|_| UNAVAILABLE)?;
			transform
				.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
				.map_err(|_| UNAVAILABLE)?;
			Ok(Self {
				transform,
				events,
				codec,
				_activate: activate,
				_runtime: runtime,
				frame: 0,
				duration: 10_000_000 / i64::from(config.fps),
				max_bytes: config.max_bytes,
				max_buffer_bytes,
				need_input: 0,
				have_output: 0,
				force_keyframe_pending: false,
				provides_samples,
			})
		}
	}

	pub(crate) fn submitted_frames(&self) -> i64 {
		self.frame
	}

	#[allow(dead_code)] // Retained native API; stream owners now restart atomically.
	pub(crate) fn set_bitrate(&mut self, bitrate: u32) -> Result<(), &'static str> {
		// SAFETY: Dynamic codec control stays on the encoder's owning worker.
		unsafe {
			self.codec
				.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(bitrate))
				.map_err(|_| FAILED)
		}
	}

	pub(crate) fn encode(
		&mut self,
		y: &[u8],
		u: &[u8],
		v: &[u8],
		force_keyframe: bool,
	) -> Result<(Vec<u8>, bool), &'static str> {
		let length = y
			.len()
			.checked_add(u.len())
			.and_then(|length| length.checked_add(v.len()))
			.filter(|_| u.len() == v.len() && y.len() == u.len() * 4)
			.ok_or(FAILED)?;
		self.encode_with(length, force_keyframe, |picture| {
			i420_to_nv12(y, u, v, picture)
		})
	}

	/// Encode one `length`-byte NV12 picture that `fill` writes into the native buffer.
	pub(crate) fn encode_with(
		&mut self,
		length: usize,
		force_keyframe: bool,
		fill: impl FnOnce(&mut [u8]) -> Result<(), &'static str>,
	) -> Result<(Vec<u8>, bool), &'static str> {
		self.force_keyframe_pending |= force_keyframe;
		if !self.wait_for_events(false)? {
			// Some async MFTs withhold NeedInput until ready output is drained.
			// Return that frame now; preserve the keyframe request for the next input.
			self.wait_for_events(true)?;
			return self.take_output();
		}
		if self.force_keyframe_pending {
			// SAFETY: Codec control is called on the owning worker before this input sample.
			unsafe {
				self.codec
					// Microsoft specifies ULONG (VT_UI4), not VARIANT_BOOL. A rejected
					// command otherwise silently sends camera and screen share to software.
					.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &force_keyframe_value())
					.map_err(|_| FAILED)?;
			}
		}
		// SAFETY: The allocation is exactly the validated NV12 size; Lock is balanced by Unlock.
		unsafe {
			let native_length = u32::try_from(length).map_err(|_| FAILED)?;
			let buffer = MFCreateMemoryBuffer(native_length).map_err(|_| FAILED)?;
			let mut data = std::ptr::null_mut();
			let mut capacity = 0;
			buffer
				.Lock(&mut data, Some(&mut capacity), None)
				.map_err(|_| FAILED)?;
			if data.is_null() || capacity < native_length {
				let _ = buffer.Unlock();
				return Err(FAILED);
			}
			let destination = std::slice::from_raw_parts_mut(data, length);
			if fill(destination).is_err() {
				let _ = buffer.Unlock();
				return Err(FAILED);
			}
			buffer.Unlock().map_err(|_| FAILED)?;
			buffer.SetCurrentLength(native_length).map_err(|_| FAILED)?;
			let sample = MFCreateSample().map_err(|_| FAILED)?;
			sample.AddBuffer(&buffer).map_err(|_| FAILED)?;
			sample
				.SetSampleTime(self.frame * self.duration)
				.map_err(|_| FAILED)?;
			sample
				.SetSampleDuration(self.duration)
				.map_err(|_| FAILED)?;
			self.transform
				.ProcessInput(0, &sample, 0)
				.map_err(|_| FAILED)?;
		}
		self.frame += 1;
		self.force_keyframe_pending = false;
		if self.wait_for_events(true)? {
			self.take_output()
		} else {
			// Normal-mode MFTs may need more pictures before producing output.
			// Preserve the input credit and let the bounded caller submit the next one.
			Ok((Vec::new(), false))
		}
	}

	fn wait_for_events(&mut self, output: bool) -> Result<bool, &'static str> {
		wait_for_events(&mut self.need_input, &mut self.have_output, output, || {
			// SAFETY: Event polling stays on the worker that owns this MFT.
			let event = unsafe { self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) };
			match event {
				Ok(event) => unsafe {
					if event.GetStatus().map_err(|_| FAILED)?.is_err() {
						return Err(FAILED);
					}
					Ok(Some(event.GetType().map_err(|_| FAILED)? as i32))
				},
				Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => Ok(None),
				Err(_) => Err(FAILED),
			}
		})
	}

	fn take_output(&self) -> Result<(Vec<u8>, bool), &'static str> {
		// SAFETY: The output struct is fully initialized. We take ownership of either the
		// transform-provided sample or our cloned sample before releasing its native fields.
		unsafe {
			let own = if self.provides_samples {
				None
			} else {
				let info = self.transform.GetOutputStreamInfo(0).map_err(|_| FAILED)?;
				let size = output_buffer_size(&info, self.max_buffer_bytes)?;
				let buffer = MFCreateMemoryBuffer(size).map_err(|_| FAILED)?;
				let sample = MFCreateSample().map_err(|_| FAILED)?;
				sample.AddBuffer(&buffer).map_err(|_| FAILED)?;
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
			let sample = std::mem::ManuallyDrop::into_inner(output.pSample).or(own);
			drop(std::mem::ManuallyDrop::into_inner(output.pEvents));
			result.map_err(|_| FAILED)?;
			let sample = sample.ok_or(FAILED)?;
			let data = sample_bytes(&sample, self.max_bytes)?;
			crate::video::validate_source(&data).map_err(|_| FAILED)?;
			let keyframe = crate::video_receive::is_keyframe(&data);
			Ok((data, keyframe))
		}
	}
}

fn wait_for_events(
	need_input: &mut usize,
	have_output: &mut usize,
	output: bool,
	mut poll: impl FnMut() -> Result<Option<i32>, &'static str>,
) -> Result<bool, &'static str> {
	let deadline = Instant::now() + Duration::from_millis(250);
	while if output {
		*have_output == 0
	} else {
		*need_input == 0 && *have_output == 0
	} {
		if Instant::now() >= deadline {
			return Err(FAILED);
		}
		match poll()? {
			Some(kind) if kind == METransformNeedInput.0 => *need_input += 1,
			Some(kind) if kind == METransformHaveOutput.0 => *have_output += 1,
			Some(kind) if kind == MEError.0 => return Err(FAILED),
			Some(_) => {}
			None if output && *need_input != 0 => return Ok(false),
			None => {
				std::thread::sleep(Duration::from_millis(1));
			}
		}
	}
	if output {
		*have_output -= 1;
	} else if *have_output != 0 {
		// Leave both credits untouched: the caller must drain output first.
		return Ok(false);
	} else {
		*need_input -= 1;
	}
	Ok(true)
}

fn force_keyframe_value() -> VARIANT {
	VARIANT::from(1_u32)
}

fn output_buffer_size(info: &MFT_OUTPUT_STREAM_INFO, limit: usize) -> Result<u32, &'static str> {
	if info.cbSize == 0 || info.cbSize as usize > limit {
		return Err(FAILED);
	}
	Ok(info.cbSize)
}

impl Drop for Encoder {
	fn drop(&mut self) {
		// SAFETY: Stop immediately; unsent video is disposable and flushing avoids blocking teardown.
		unsafe {
			let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
			let _ = self
				.transform
				.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
		}
	}
}

unsafe fn hardware_encoder(
	adapter: Option<model::VideoAdapter>,
) -> Result<IMFActivate, &'static str> {
	unsafe {
		let input = MFT_REGISTER_TYPE_INFO {
			guidMajorType: MFMediaType_Video,
			guidSubtype: MFVideoFormat_NV12,
		};
		let output = MFT_REGISTER_TYPE_INFO {
			guidMajorType: MFMediaType_Video,
			guidSubtype: MFVideoFormat_H264,
		};
		let mut entries = std::ptr::null_mut();
		let mut count = 0;
		let flags = MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER;
		if let Some(adapter) = adapter {
			let model::VideoAdapterIdentity::WindowsLuid(luid) = adapter.identity else {
				return Err(UNAVAILABLE);
			};
			if luid == 0 {
				return Err(UNAVAILABLE);
			}
			let mut attributes = None;
			MFCreateAttributes(&mut attributes, 1).map_err(|_| UNAVAILABLE)?;
			let attributes = attributes.ok_or(UNAVAILABLE)?;
			attributes
				.SetUINT64(&MFT_ENUM_ADAPTER_LUID, luid)
				.map_err(|_| UNAVAILABLE)?;
			// MFTEnum2 filters hardware activations by this exact DXGI adapter.
			// An absent hardware encoder falls back to software in the caller.
			MFTEnum2(
				MFT_CATEGORY_VIDEO_ENCODER,
				flags,
				Some(&input),
				Some(&output),
				&attributes,
				&mut entries,
				&mut count,
			)
		} else {
			MFTEnumEx(
				MFT_CATEGORY_VIDEO_ENCODER,
				flags,
				Some(&input),
				Some(&output),
				&mut entries,
				&mut count,
			)
		}
		.map_err(|_| UNAVAILABLE)?;
		if entries.is_null() {
			return Err(UNAVAILABLE);
		}
		if count == 0 {
			CoTaskMemFree(Some(entries.cast()));
			return Err(UNAVAILABLE);
		}
		let entries_slice = std::slice::from_raw_parts_mut(entries, count as usize);
		let selected = entries_slice.first_mut().and_then(Option::take);
		for entry in entries_slice {
			*entry = None;
		}
		CoTaskMemFree(Some(entries.cast()));
		selected.ok_or(UNAVAILABLE)
	}
}

unsafe fn video_type(
	config: Config,
	subtype: windows::core::GUID,
) -> Result<IMFMediaType, &'static str> {
	unsafe {
		let media_type = MFCreateMediaType().map_err(|_| UNAVAILABLE)?;
		media_type
			.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
			.map_err(|_| UNAVAILABLE)?;
		media_type
			.SetGUID(&MF_MT_SUBTYPE, &subtype)
			.map_err(|_| UNAVAILABLE)?;
		media_type
			.SetUINT64(
				&MF_MT_FRAME_SIZE,
				(u64::from(config.width) << 32) | u64::from(config.height),
			)
			.map_err(|_| UNAVAILABLE)?;
		media_type
			.SetUINT64(&MF_MT_FRAME_RATE, u64::from(config.fps) << 32 | 1)
			.map_err(|_| UNAVAILABLE)?;
		media_type
			.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, 1_u64 << 32 | 1)
			.map_err(|_| UNAVAILABLE)?;
		media_type
			.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
			.map_err(|_| UNAVAILABLE)?;
		Ok(media_type)
	}
}

fn sample_bytes(sample: &IMFSample, max_bytes: usize) -> Result<Vec<u8>, &'static str> {
	// SAFETY: Length is bounded before allocation; Lock's pointer is borrowed until Unlock.
	unsafe {
		if sample.GetTotalLength().map_err(|_| FAILED)? as usize > max_bytes {
			return Err(FAILED);
		}
		let buffer = sample.ConvertToContiguousBuffer().map_err(|_| FAILED)?;
		let mut data = std::ptr::null_mut();
		let mut capacity = 0;
		let mut length = 0;
		buffer
			.Lock(&mut data, Some(&mut capacity), Some(&mut length))
			.map_err(|_| FAILED)?;
		let result = if data.is_null() || length > capacity || length as usize > max_bytes {
			Err(FAILED)
		} else {
			Ok(std::slice::from_raw_parts(data, length as usize).to_vec())
		};
		buffer.Unlock().map_err(|_| FAILED)?;
		result
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ready_output_does_not_wait_for_an_input_request() {
		for queued_input in [0, 1] {
			let (mut input, mut output) = (queued_input, 1);
			assert!(!wait_for_events(&mut input, &mut output, false, || unreachable!()).unwrap());
			assert_eq!((input, output), (queued_input, 1));
			assert!(wait_for_events(&mut input, &mut output, true, || unreachable!()).unwrap());
			assert_eq!((input, output), (queued_input, 0));
		}
		let (mut input, mut output) = (0, 0);
		assert!(
			!wait_for_events(&mut input, &mut output, false, || Ok(Some(
				METransformHaveOutput.0
			)))
			.unwrap()
		);
		assert_eq!((input, output), (0, 1));
		assert!(wait_for_events(&mut input, &mut output, true, || unreachable!()).unwrap());
		// A driver may issue its next input request only after ProcessOutput.
		assert!(
			wait_for_events(&mut input, &mut output, false, || Ok(Some(
				METransformNeedInput.0
			)))
			.unwrap()
		);
		assert_eq!((input, output), (0, 0));
	}

	#[test]
	fn delayed_mft_output_preserves_input_credit_and_drains_output_first() {
		let (mut input, mut output) = (0, 0);
		let mut events = std::collections::VecDeque::from([METransformNeedInput.0]);
		// A normal-mode encoder asks for its second input before its first output.
		assert!(
			!wait_for_events(&mut input, &mut output, true, || { Ok(events.pop_front()) }).unwrap()
		);
		assert_eq!((input, output), (1, 0));
		assert!(wait_for_events(&mut input, &mut output, false, || unreachable!()).unwrap());
		assert_eq!((input, output), (0, 0));
		assert!(
			wait_for_events(&mut input, &mut output, true, || Ok(Some(
				METransformHaveOutput.0
			)))
			.unwrap()
		);
		assert_eq!((input, output), (0, 0));
		// Drain already queued output even when a new input credit arrives first.
		let mut events =
			std::collections::VecDeque::from([METransformNeedInput.0, METransformHaveOutput.0]);
		assert!(wait_for_events(&mut input, &mut output, true, || Ok(events.pop_front())).unwrap());
		assert_eq!((input, output), (1, 0));
		(input, output) = (1, 1);
		assert!(wait_for_events(&mut input, &mut output, true, || unreachable!()).unwrap());
		assert_eq!((input, output), (1, 0));
		for event in [Ok(Some(MEError.0)), Err("poll failed")] {
			assert!(wait_for_events(&mut input, &mut output, true, || event).is_err());
		}
	}

	#[test]
	fn larger_native_buffer_does_not_relax_encoded_sample_limit() {
		let _runtime = Runtime::open().unwrap();
		let limit = crate::camera::encoded_limit(model::voice_settings::VideoResolution::P480);
		// Synthetic memory only: no transform, GPU, capture device or transport.
		unsafe {
			let capacity = 640 * 480 * 3 / 2;
			assert!(capacity > limit);
			let buffer = MFCreateMemoryBuffer(capacity as u32).unwrap();
			let mut data = std::ptr::null_mut();
			buffer.Lock(&mut data, None, None).unwrap();
			std::ptr::write_bytes(data, 0x2a, capacity);
			buffer.Unlock().unwrap();
			buffer.SetCurrentLength(limit as u32).unwrap();
			let sample = MFCreateSample().unwrap();
			sample.AddBuffer(&buffer).unwrap();
			let encoded = sample_bytes(&sample, limit).unwrap();
			assert_eq!(encoded.len(), limit);
			assert!(encoded.iter().all(|&byte| byte == 0x2a));
			buffer.SetCurrentLength(limit as u32 + 1).unwrap();
			assert!(sample_bytes(&sample, limit).is_err());
		}

		{
			let mut info = MFT_OUTPUT_STREAM_INFO {
				dwFlags: 0,
				cbSize: 1920 * 1080 * 3 / 2,
				cbAlignment: 0,
			};
			assert!(info.cbSize as usize > crate::camera::MAX_ENCODED_BYTES);
			assert_eq!(
				output_buffer_size(&info, 1920 * 1080 * 4).unwrap(),
				info.cbSize
			);
			assert!(output_buffer_size(&info, 1024).is_err());
			info.cbSize = 0;
			assert!(output_buffer_size(&info, 1280 * 720 * 4).is_err());
			info.cbSize = u32::MAX;
			assert!(output_buffer_size(&info, 1280 * 720 * 4).is_err());
		}

		{
			let value = force_keyframe_value();
			assert_eq!(value.vt(), windows::Win32::System::Variant::VT_UI4);
			assert_eq!(u32::try_from(&value).unwrap(), 1);
		}
	}
}
