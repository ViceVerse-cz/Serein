//! Media Foundation capture; native callbacks copy one bounded frame, never encode.
#![allow(unsafe_code)]

mod directshow;

#[cfg(test)]
use super::{HEIGHT, WIDTH};
use super::{Shared, format};
use model::voice_settings::{VideoFrameRate, VideoResolution};
use std::{
	marker::PhantomData,
	rc::Rc,
	sync::{
		Arc,
		atomic::{AtomicI32, Ordering},
		mpsc::{self, SyncSender},
	},
	thread,
	time::{Duration, Instant},
};
use windows::{
	Win32::{
		Media::MediaFoundation::*,
		System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
	},
	core::{HRESULT, Interface, Ref, implement},
};

const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
const INVALID: &str = "Camera did not provide the selected bounded RGB frame size";
const UNAVAILABLE: &str = "Camera is busy or unavailable. Check Windows Settings > Privacy & security > Camera and allow desktop apps to access your camera.";
const TIMEOUT: &str = "Camera stopped delivering frames; check the device and try again";

// Like the attachment decoder, balance COM/MF on their owning worker. Locals holding
// COM objects are declared after this guard and therefore released before it.
struct Runtime(PhantomData<Rc<()>>);
impl Runtime {
	fn open() -> Result<Self, &'static str> {
		// SAFETY: Initialized and released on the camera worker, never a render callback.
		unsafe {
			CoInitializeEx(None, COINIT_MULTITHREADED)
				.ok()
				.map_err(|_| UNAVAILABLE)?;
			if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
				CoUninitialize();
				return Err(
					"Windows Media Foundation is unavailable; install the Media Feature Pack on Windows N",
				);
			}
		}
		Ok(Self(PhantomData))
	}
}
impl Drop for Runtime {
	fn drop(&mut self) {
		// SAFETY: Balanced successful startup on this thread, after native capture drops.
		unsafe {
			let _ = MFShutdown();
			CoUninitialize();
		}
	}
}

struct Source(IMFMediaSource);
impl Drop for Source {
	fn drop(&mut self) {
		// SAFETY: Owned media source; Shutdown releases the camera, including on errors.
		unsafe {
			let _ = self.0.Shutdown();
		}
	}
}

struct ReadResult {
	changed: bool,
	rgb: Option<Vec<u8>>,
}

#[implement(IMFSourceReaderCallback)]
struct Callback {
	send: SyncSender<Result<ReadResult, &'static str>>,
	stride: Arc<AtomicI32>,
	dimensions: (usize, usize),
}
impl IMFSourceReaderCallback_Impl for Callback_Impl {
	fn OnReadSample(
		&self,
		status: HRESULT,
		_: u32,
		flags: u32,
		_: i64,
		sample: Ref<'_, IMFSample>,
	) -> windows::core::Result<()> {
		let failed = MF_SOURCE_READERF_ERROR.0
			| MF_SOURCE_READERF_ENDOFSTREAM.0
			| MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED.0;
		let changed = flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 != 0;
		let result = if status.is_err() || flags & failed as u32 != 0 {
			Err(UNAVAILABLE)
		} else if changed {
			// Some decoders finish negotiation on their first sample. Do not touch
			// that sample until the worker has revalidated the resulting media type.
			Ok(ReadResult { changed, rgb: None })
		} else {
			sample
				.as_ref()
				.map(|sample| {
					copy_sample(sample, self.stride.load(Ordering::Acquire), self.dimensions)
				})
				.transpose()
				.map(|rgb| ReadResult { changed, rgb })
		};
		// Only one ReadSample is outstanding; no COM sample is retained in the
		// channel. Capacity is one frame with the selected bounded RGB dimensions.
		let _ = self.send.try_send(result);
		Ok(())
	}
	fn OnFlush(&self, _: u32) -> windows::core::Result<()> {
		Ok(())
	}
	fn OnEvent(&self, _: u32, event: Ref<'_, IMFMediaEvent>) -> windows::core::Result<()> {
		// SAFETY: Event is borrowed only during this callback.
		if let Some(event) = event.as_ref()
			&& unsafe { event.GetStatus() }.is_ok_and(|status| status.is_err())
		{
			let _ = self.send.try_send(Err(UNAVAILABLE));
		}
		Ok(())
	}
}

pub(super) fn run(
	shared: &Shared,
	device: Option<&str>,
	resolution: VideoResolution,
	frame_rate: VideoFrameRate,
	emit: &mut dyn FnMut(Vec<u8>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
	if shared.stopped.load(Ordering::Acquire) {
		return Ok(());
	}
	let (width, height) = resolution.camera_dimensions();
	let dimensions = (width as usize, height as usize);
	let target_fps = frame_rate.fps();
	let interval = format::frame_interval(target_fps).ok_or("Invalid camera frame rate")?;
	let _runtime = Runtime::open()?;
	if device.is_some_and(|id| id.starts_with("dshow:")) {
		return directshow::run(shared, device, resolution, frame_rate, emit);
	}
	let cameras = camera_sources()?;
	if cameras.is_empty() && device.is_none() {
		return directshow::run(shared, None, resolution, frame_rate, emit);
	}
	let source = selected_camera(cameras, device)?;
	let (send, receive) = mpsc::sync_channel(1);
	let stride = Arc::new(AtomicI32::new(0));
	let callback: IMFSourceReaderCallback = Callback {
		send,
		stride: stride.clone(),
		dimensions,
	}
	.into();
	// SAFETY: COM objects stay on this MTA worker; callback owns only thread-safe
	// Rust state. All out parameters are initialized and errors retain RAII cleanup.
	let reader = unsafe {
		let attributes = attributes(2)?;
		attributes
			.SetUnknown(&MF_SOURCE_READER_ASYNC_CALLBACK, &callback)
			.map_err(|_| UNAVAILABLE)?;
		attributes
			.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
			.map_err(|_| UNAVAILABLE)?;
		let reader =
			MFCreateSourceReaderFromMediaSource(&source.0, &attributes).map_err(|_| UNAVAILABLE)?;
		reader
			.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
			.map_err(|_| UNAVAILABLE)?;
		// Native dimensions are selected first to bound upstream decoder input.
		let mut choices = Vec::new();
		for index in 0..256 {
			let Ok(native) = reader.GetNativeMediaType(VIDEO, index) else {
				break;
			};
			let size = native.GetUINT64(&MF_MT_FRAME_SIZE).unwrap_or(0);
			let rate = native.GetUINT64(&MF_MT_FRAME_RATE).unwrap_or(0);
			// Range-capable drivers can expose one native type covering several rates.
			// Keep its original type as a fallback if the driver rejects the requested rate.
			if let Some(selected_rate) = nearest_native_rate(
				native.GetUINT64(&MF_MT_FRAME_RATE_RANGE_MIN).unwrap_or(0),
				native.GetUINT64(&MF_MT_FRAME_RATE_RANGE_MAX).unwrap_or(0),
				target_fps,
			) && selected_rate != rate
				&& let Some(rank) = format::rank_for_output_at_rate(
					(size >> 32) as usize,
					(size as u32) as usize,
					native_fps(selected_rate).ok_or(INVALID)?,
					dimensions,
					target_fps,
				) && let Ok(requested) = MFCreateMediaType()
				&& native.CopyAllItems(&requested).is_ok()
				&& requested
					.SetUINT64(&MF_MT_FRAME_RATE, selected_rate)
					.is_ok()
			{
				choices.push((rank, requested));
			}
			let fps = native_fps(rate);
			if let Some(mut rank) = format::rank_for_output_at_rate(
				(size >> 32) as usize,
				(size as u32) as usize,
				fps.unwrap_or(f64::from(target_fps)),
				dimensions,
				target_fps,
			) {
				// Some drivers omit timing; preserve their existing bounded fallback.
				if fps.is_none() {
					rank.1 = u64::MAX;
				}
				choices.push((rank, native));
			}
		}
		choices.sort_by_key(|(rank, _)| *rank);
		let output = MFCreateMediaType().map_err(|_| INVALID)?;
		output
			.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
			.map_err(|_| INVALID)?;
		output
			.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
			.map_err(|_| INVALID)?;
		output
			.SetUINT64(&MF_MT_FRAME_SIZE, frame_size(dimensions))
			.map_err(|_| INVALID)?;
		// Output negotiation may otherwise select a different capture mode. Pin
		// each bounded native mode on the source before adding RGB conversion.
		let native_reader = reader
			.cast::<IMFSourceReaderEx>()
			.map_err(|_| UNAVAILABLE)?;
		let mut converted_stride = None;
		for (_, native) in choices {
			if native_reader.SetNativeMediaType(VIDEO, &native).is_err()
				|| reader.SetCurrentMediaType(VIDEO, None, &output).is_err()
			{
				continue;
			}
			// Select only after the complete bounded output layout is usable.
			if let Ok(value) = output_stride(&reader, dimensions) {
				converted_stride = Some(value);
				break;
			}
		}
		stride.store(
			converted_stride
				.ok_or("Camera does not support the selected resolution as bounded RGB video")?,
			Ordering::Release,
		);
		reader
			.SetStreamSelection(VIDEO, true)
			.map_err(|_| UNAVAILABLE)?;
		reader
	};
	let mut last_frame = Instant::now();
	while !shared.stopped.load(Ordering::Acquire) {
		let requested = Instant::now();
		// SAFETY: Async reader returns immediately; callbacks never request more
		// samples. A stalled device cannot trap the worker in synchronous ReadSample.
		unsafe { reader.ReadSample(VIDEO, 0, None, None, None, None) }.map_err(|_| UNAVAILABLE)?;
		let result = loop {
			if shared.stopped.load(Ordering::Acquire) {
				return Ok(());
			}
			if last_frame.elapsed() >= Duration::from_secs(5) {
				return Err(TIMEOUT);
			}
			match receive.recv_timeout(Duration::from_millis(50)) {
				Ok(result) => break result?,
				Err(mpsc::RecvTimeoutError::Timeout) => continue,
				Err(mpsc::RecvTimeoutError::Disconnected) => return Err(TIMEOUT),
			}
		};
		if result.changed {
			stride.store(output_stride(&reader, dimensions)?, Ordering::Release);
		}
		if let Some(rgb) = result.rgb {
			last_frame = Instant::now();
			if !shared.stopped.load(Ordering::Acquire) {
				emit(rgb)?;
			}
		}
		if let Some(remaining) = interval.checked_sub(requested.elapsed()) {
			thread::sleep(remaining);
		}
	}
	Ok(())
}

fn native_fps(rate: u64) -> Option<f64> {
	((rate >> 32) != 0 && rate as u32 != 0).then(|| (rate >> 32) as f64 / f64::from(rate as u32))
}

fn nearest_native_rate(min: u64, max: u64, target_fps: u32) -> Option<u64> {
	let (min_fps, max_fps) = (native_fps(min)?, native_fps(max)?);
	let selected = format::nearest_fps_for(min_fps, max_fps, target_fps)?;
	Some(if selected == min_fps {
		min
	} else if selected == max_fps {
		max
	} else {
		(u64::from(target_fps) << 32) | 1
	})
}

fn frame_size(dimensions: (usize, usize)) -> u64 {
	((dimensions.0 as u64) << 32) | dimensions.1 as u64
}

fn frame_budget(dimensions: (usize, usize)) -> Result<usize, &'static str> {
	format::raw_budget(dimensions.0, dimensions.1).ok_or(INVALID)
}

fn attributes(count: u32) -> Result<IMFAttributes, &'static str> {
	let mut attributes = None;
	// SAFETY: Valid initialized out pointer, called after MF startup.
	unsafe { MFCreateAttributes(&mut attributes, count) }.map_err(|_| UNAVAILABLE)?;
	attributes.ok_or(UNAVAILABLE)
}

fn camera_sources() -> Result<Vec<IMFActivate>, &'static str> {
	// SAFETY: MF owns the returned count-sized array of COM pointers. Release
	// every activation and CoTaskMemFree the array, even when activation fails.
	unsafe {
		let attributes = attributes(1)?;
		attributes
			.SetGUID(
				&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
				&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
			)
			.map_err(|_| UNAVAILABLE)?;
		let mut devices = std::ptr::null_mut();
		let mut count = 0;
		MFEnumDeviceSources(&attributes, &mut devices, &mut count).map_err(|_| UNAVAILABLE)?;
		if devices.is_null() {
			return Ok(Vec::new());
		}
		let entries = std::slice::from_raw_parts_mut(devices, count as usize);
		let result = entries
			.iter_mut()
			.take(32)
			.filter_map(Option::take)
			.collect();
		for entry in entries {
			*entry = None;
		}
		CoTaskMemFree(Some(devices.cast()));
		Ok(result)
	}
}

fn device_string(device: &IMFActivate, key: &windows::core::GUID, limit: usize) -> Option<String> {
	// SAFETY: Fixed-size destination; GetString verifies its capacity. No native allocation retained.
	unsafe {
		let mut buffer = [0u16; 4096];
		let length = device.GetStringLength(key).ok()? as usize;
		if length == 0 || length >= buffer.len() {
			return None;
		}
		device
			.GetString(key, &mut buffer[..length + 1], None)
			.ok()?;
		let value = String::from_utf16(&buffer[..length]).ok()?;
		(value.len() <= limit && !value.contains('\0')).then_some(value)
	}
}

fn device_id(device: &IMFActivate) -> Option<String> {
	device_string(
		device,
		&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
		4093,
	)
	.map(|id| format!("mf:{id}"))
}

pub(super) fn devices() -> Result<Vec<(String, String)>, &'static str> {
	let _runtime = Runtime::open()?;
	let mf = camera_sources();
	let ds = directshow::devices();
	if mf.is_err() && ds.is_err() {
		return Err(UNAVAILABLE);
	}
	let mut devices: Vec<_> = mf
		.unwrap_or_default()
		.iter()
		.filter_map(|device| {
			Some((
				device_id(device)?,
				device_string(device, &MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, 256)?,
			))
		})
		.collect();
	// Keep backend identities distinct: virtual devices may share a friendly name.
	devices.extend(
		ds.unwrap_or_default()
			.into_iter()
			.take(32usize.saturating_sub(devices.len())),
	);
	Ok(devices)
}

fn selected_camera(
	cameras: Vec<IMFActivate>,
	selected: Option<&str>,
) -> Result<Source, &'static str> {
	let selected = cameras
		.iter()
		.find(|camera| selected.is_none_or(|id| device_id(camera).as_deref() == Some(id)))
		.ok_or(
			"Selected camera is disconnected or unavailable. Refresh cameras and choose another device.",
		)?;
	// SAFETY: Activation happens only on an explicit camera-on action; Source shuts down on drop.
	unsafe {
		selected
			.SetUINT32(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_MAX_BUFFERS, 1)
			.map_err(|_| UNAVAILABLE)?;
		selected
			.ActivateObject::<IMFMediaSource>()
			.map(Source)
			.map_err(|_| UNAVAILABLE)
	}
}

fn output_stride(
	reader: &IMFSourceReader,
	dimensions: (usize, usize),
) -> Result<i32, &'static str> {
	frame_budget(dimensions)?;
	let width = dimensions.0;
	// SAFETY: Worker-owned reader. Check type and dimensions before using its stride.
	unsafe {
		let media = reader.GetCurrentMediaType(VIDEO).map_err(|_| INVALID)?;
		if media.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|_| INVALID)? != frame_size(dimensions)
			|| media.GetGUID(&MF_MT_SUBTYPE).map_err(|_| INVALID)? != MFVideoFormat_RGB32
		{
			return Err(INVALID);
		}
		let stride = media
			.GetUINT32(&MF_MT_DEFAULT_STRIDE)
			.map(|stride| stride as i32)
			.or_else(|_| MFGetStrideForBitmapInfoHeader(MFVideoFormat_RGB32.data1, width as u32))
			.map_err(|_| INVALID)?;
		if !(width * 4..=width * 4 + format::MAX_STRIDE_PADDING)
			.contains(&(stride.unsigned_abs() as usize))
		{
			return Err(INVALID);
		}
		Ok(stride)
	}
}

fn copy_sample(
	sample: &IMFSample,
	stride: i32,
	dimensions: (usize, usize),
) -> Result<Vec<u8>, &'static str> {
	let max_bytes = frame_budget(dimensions)?;
	// SAFETY: Sample is borrowed only during its callback. Validate native byte
	// budgets before mapping/copying; every successful lock is unlocked on all paths.
	unsafe {
		if sample.GetBufferCount().map_err(|_| INVALID)? != 1
			|| sample.GetTotalLength().map_err(|_| INVALID)? as usize > max_bytes
		{
			return Err(INVALID);
		}
		let buffer = sample.GetBufferByIndex(0).map_err(|_| INVALID)?;
		if buffer.GetMaxLength().map_err(|_| INVALID)? as usize > max_bytes {
			return Err(INVALID);
		}
		if let Ok(buffer2d) = buffer.cast::<IMF2DBuffer2>() {
			let (mut top, mut base, mut pitch, mut length) =
				(std::ptr::null_mut(), std::ptr::null_mut(), 0, 0);
			buffer2d
				.Lock2DSize(
					MF2DBuffer_LockFlags_Read,
					&mut top,
					&mut pitch,
					&mut base,
					&mut length,
				)
				.map_err(|_| INVALID)?;
			let result = if base.is_null() || top.is_null() || length as usize > max_bytes {
				Err(INVALID)
			} else if let Some(first) = (top as usize).checked_sub(base as usize) {
				rgb_rows(
					std::slice::from_raw_parts(base, length as usize),
					first,
					pitch,
					dimensions,
				)
			} else {
				Err(INVALID)
			};
			buffer2d.Unlock2D().map_err(|_| INVALID)?;
			return result;
		}
		let (mut base, mut capacity, mut length) = (std::ptr::null_mut(), 0, 0);
		buffer
			.Lock(&mut base, Some(&mut capacity), Some(&mut length))
			.map_err(|_| INVALID)?;
		let result = if base.is_null() || length > capacity || capacity as usize > max_bytes {
			Err(INVALID)
		} else {
			let first = if stride < 0 {
				stride.unsigned_abs() as usize * (dimensions.1 - 1)
			} else {
				0
			};
			rgb_rows(
				std::slice::from_raw_parts(base, length as usize),
				first,
				stride,
				dimensions,
			)
		};
		buffer.Unlock().map_err(|_| INVALID)?;
		result
	}
}

fn rgb_rows(
	bytes: &[u8],
	first: usize,
	stride: i32,
	dimensions: (usize, usize),
) -> Result<Vec<u8>, &'static str> {
	let max_bytes = frame_budget(dimensions)?;
	let (width, height) = dimensions;
	let pitch = stride.unsigned_abs() as usize;
	if !(width * 4..=width * 4 + format::MAX_STRIDE_PADDING).contains(&pitch)
		|| bytes.len() > max_bytes
	{
		return Err(INVALID);
	}
	let last = first
		.checked_add_signed(stride as isize * (height - 1) as isize)
		.ok_or(INVALID)?;
	if first
		.max(last)
		.checked_add(width * 4)
		.is_none_or(|end| end > bytes.len())
	{
		return Err(INVALID);
	}
	let mut rgb = vec![0; width * height * 3];
	for (y, dest) in rgb.chunks_exact_mut(width * 3).enumerate() {
		let start = first
			.checked_add_signed(stride as isize * y as isize)
			.ok_or(INVALID)?;
		for (pixel, dest) in bytes[start..start + width * 4]
			.as_chunks::<4>()
			.0
			.iter()
			.zip(dest.as_chunks_mut::<3>().0)
		{
			dest.copy_from_slice(&[pixel[2], pixel[1], pixel[0]]);
		}
	}
	Ok(rgb)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn native_rate_ranges_preserve_rational_endpoints_and_reject_invalid_timing() {
		let rate = |numerator: u32, denominator: u32| {
			(u64::from(numerator) << 32) | u64::from(denominator)
		};
		let (min, max) = (rate(24, 1), rate(60000, 1001));
		assert_eq!(nearest_native_rate(min, max, 15), Some(min));
		assert_eq!(nearest_native_rate(min, max, 30), Some(rate(30, 1)));
		assert_eq!(nearest_native_rate(min, max, 60), Some(max));
		assert_eq!(nearest_native_rate(max, min, 30), None);
		assert_eq!(nearest_native_rate(rate(0, 1), max, 30), None);
		assert_eq!(nearest_native_rate(min, rate(60, 0), 30), None);
		assert_eq!(nearest_native_rate(min, max, 0), None);
		assert_eq!(native_fps(rate(60000, 1001)), Some(60000.0 / 1001.0));
	}
	#[test]
	#[ignore = "manual read-only device enumeration; never activates a camera"]
	fn list_windows_camera_names_without_capture() {
		let devices = devices().expect("camera enumeration");
		assert!(devices.len() <= 32);
		for (id, name) in devices {
			assert!(id.len() <= 4096 && name.len() <= 256);
			println!("Camera: {name}");
		}
	}

	#[test]
	fn selected_rgb_output_geometry_and_stride_are_bounded() {
		let dimensions = (1280, 720);
		let bytes = vec![0; 1280 * 720 * 4];
		assert_eq!(
			rgb_rows(&bytes, 0, 1280 * 4, dimensions).unwrap().len(),
			1280 * 720 * 3
		);
		assert!(rgb_rows(&bytes, 0, 1280 * 4, (1920, 1080)).is_err());
		assert!(frame_budget((7680, 4320)).is_ok());
		assert!(frame_budget((7681, 4320)).is_err());
		assert!(frame_budget((0, 1)).is_err());
	}

	#[test]
	fn rgb_rows_handle_padding_bottom_up_and_reject_invalid_bounds() {
		let pitch = WIDTH * 4 + 8;
		let mut bytes = vec![0; pitch * HEIGHT];
		bytes[..4].copy_from_slice(&[10, 20, 30, 255]);
		bytes[(HEIGHT - 1) * pitch..(HEIGHT - 1) * pitch + 4].copy_from_slice(&[40, 50, 60, 255]);
		assert_eq!(
			&rgb_rows(&bytes, 0, pitch as i32, (WIDTH, HEIGHT)).unwrap()[..3],
			&[30, 20, 10]
		);
		assert_eq!(
			&rgb_rows(
				&bytes,
				(HEIGHT - 1) * pitch,
				-(pitch as i32),
				(WIDTH, HEIGHT)
			)
			.unwrap()[..3],
			&[60, 50, 40]
		);
		assert!(rgb_rows(&bytes[..pitch], 0, pitch as i32, (WIDTH, HEIGHT)).is_err());
		assert!(rgb_rows(&bytes, 0, -(pitch as i32), (WIDTH, HEIGHT)).is_err());
		assert!(rgb_rows(&bytes, usize::MAX, pitch as i32, (WIDTH, HEIGHT)).is_err());
		assert!(rgb_rows(&bytes, 0, i32::MIN, (WIDTH, HEIGHT)).is_err());
		assert!(rgb_rows(&bytes, 0, WIDTH as i32 * 4 - 1, (WIDTH, HEIGHT)).is_err());
	}
}
