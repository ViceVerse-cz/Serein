//! Bounded Media Foundation hardware H.264 encoder for Windows screen sharing.
#![allow(unsafe_code)]

use super::{MAX_ENCODED_BYTES, Settings, i420_to_nv12};
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

pub(super) struct Encoder {
	transform: IMFTransform,
	events: IMFMediaEventGenerator,
	codec: ICodecAPI,
	_activate: Activated,
	_runtime: Runtime,
	frame: i64,
	duration: i64,
	need_input: usize,
	have_output: usize,
	provides_samples: bool,
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
	pub(super) fn new(settings: Settings) -> Result<Self, &'static str> {
		let runtime = Runtime::open()?;
		// SAFETY: Media Foundation owns returned COM objects; the activation array is cleared
		// before its CoTaskMem allocation is released.
		unsafe {
			let activate = Activated(hardware_encoder()?);
			let transform: IMFTransform = activate.0.ActivateObject().map_err(|_| UNAVAILABLE)?;
			let attributes = transform.GetAttributes().map_err(|_| UNAVAILABLE)?;
			if attributes.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) == 0 {
				return Err(UNAVAILABLE);
			}
			attributes
				.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)
				.map_err(|_| UNAVAILABLE)?;
			let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
			let codec: ICodecAPI = transform.cast().map_err(|_| UNAVAILABLE)?;
			let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
			let _ = codec.SetValue(
				&CODECAPI_AVEncCommonRateControlMode,
				&VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32),
			);
			let _ = codec.SetValue(
				&CODECAPI_AVEncCommonMeanBitRate,
				&VARIANT::from(settings.bit_rate()),
			);
			let _ = codec.SetValue(
				&CODECAPI_AVEncMPVDefaultBPictureCount,
				&VARIANT::from(0_u32),
			);
			let _ = codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(settings.fps * 2));

			let output = video_type(settings, MFVideoFormat_H264)?;
			output
				.SetUINT32(&MF_MT_AVG_BITRATE, settings.bit_rate())
				.map_err(|_| UNAVAILABLE)?;
			transform
				.SetOutputType(0, &output, 0)
				.map_err(|_| UNAVAILABLE)?;

			let input = video_type(settings, MFVideoFormat_NV12)?;
			input
				.SetUINT32(&MF_MT_DEFAULT_STRIDE, settings.width)
				.map_err(|_| UNAVAILABLE)?;
			transform
				.SetInputType(0, &input, 0)
				.map_err(|_| UNAVAILABLE)?;

			let info = transform.GetOutputStreamInfo(0).map_err(|_| UNAVAILABLE)?;
			if info.cbSize as usize > MAX_ENCODED_BYTES {
				return Err(UNAVAILABLE);
			}
			let provides_samples = info.dwFlags
				& (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
					| MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)
				!= 0;
			if !provides_samples && info.cbSize == 0 {
				return Err(UNAVAILABLE);
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
				duration: 10_000_000 / i64::from(settings.fps),
				need_input: 0,
				have_output: 0,
				provides_samples,
			})
		}
	}

	pub(super) fn encode(
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
		self.wait_for_input()?;
		if force_keyframe {
			// SAFETY: Codec control is called on the owning worker before this input sample.
			unsafe {
				self.codec
					.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &VARIANT::from(true))
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
			if i420_to_nv12(y, u, v, destination).is_err() {
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
			self.frame += 1;
			self.transform
				.ProcessInput(0, &sample, 0)
				.map_err(|_| FAILED)?;
		}
		self.wait_for_output()?;
		self.take_output()
	}

	fn wait_for_input(&mut self) -> Result<(), &'static str> {
		self.wait_until(|encoder| encoder.need_input != 0)?;
		self.need_input -= 1;
		Ok(())
	}

	fn wait_for_output(&mut self) -> Result<(), &'static str> {
		self.wait_until(|encoder| encoder.have_output != 0)?;
		self.have_output -= 1;
		Ok(())
	}

	fn wait_until(&mut self, ready: impl Fn(&Self) -> bool) -> Result<(), &'static str> {
		let deadline = Instant::now() + Duration::from_millis(250);
		while !ready(self) {
			// SAFETY: Event polling stays on the worker that owns this MFT.
			let event = unsafe { self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) };
			match event {
				Ok(event) => unsafe {
					if event.GetStatus().map_err(|_| FAILED)?.is_err() {
						return Err(FAILED);
					}
					match event.GetType().map_err(|_| FAILED)? as i32 {
						kind if kind == METransformNeedInput.0 => self.need_input += 1,
						kind if kind == METransformHaveOutput.0 => self.have_output += 1,
						kind if kind == MEError.0 => return Err(FAILED),
						_ => {}
					}
				},
				Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => {
					if Instant::now() >= deadline {
						return Err(FAILED);
					}
					std::thread::sleep(Duration::from_millis(1));
				}
				Err(_) => return Err(FAILED),
			}
		}
		Ok(())
	}

	fn take_output(&self) -> Result<(Vec<u8>, bool), &'static str> {
		// SAFETY: The output struct is fully initialized. We take ownership of either the
		// transform-provided sample or our cloned sample before releasing its native fields.
		unsafe {
			let own = if self.provides_samples {
				None
			} else {
				let size = self
					.transform
					.GetOutputStreamInfo(0)
					.map_err(|_| FAILED)?
					.cbSize;
				if size == 0 || size as usize > MAX_ENCODED_BYTES {
					return Err(FAILED);
				}
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
			let data = sample_bytes(&sample)?;
			crate::video::validate_source(&data).map_err(|_| FAILED)?;
			let keyframe = crate::video_receive::is_keyframe(&data);
			Ok((data, keyframe))
		}
	}
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

unsafe fn hardware_encoder() -> Result<IMFActivate, &'static str> {
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
		MFTEnumEx(
			MFT_CATEGORY_VIDEO_ENCODER,
			MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
			Some(&input),
			Some(&output),
			&mut entries,
			&mut count,
		)
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
	settings: Settings,
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
				(u64::from(settings.width) << 32) | u64::from(settings.height),
			)
			.map_err(|_| UNAVAILABLE)?;
		media_type
			.SetUINT64(&MF_MT_FRAME_RATE, u64::from(settings.fps) << 32 | 1)
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

fn sample_bytes(sample: &IMFSample) -> Result<Vec<u8>, &'static str> {
	// SAFETY: Length is bounded before allocation; Lock's pointer is borrowed until Unlock.
	unsafe {
		if sample.GetTotalLength().map_err(|_| FAILED)? as usize > MAX_ENCODED_BYTES {
			return Err(FAILED);
		}
		let buffer = sample.ConvertToContiguousBuffer().map_err(|_| FAILED)?;
		let mut data = std::ptr::null_mut();
		let mut capacity = 0;
		let mut length = 0;
		buffer
			.Lock(&mut data, Some(&mut capacity), Some(&mut length))
			.map_err(|_| FAILED)?;
		let result = if data.is_null() || length > capacity || length as usize > MAX_ENCODED_BYTES {
			Err(FAILED)
		} else {
			Ok(std::slice::from_raw_parts(data, length as usize).to_vec())
		};
		buffer.Unlock().map_err(|_| FAILED)?;
		result
	}
}
