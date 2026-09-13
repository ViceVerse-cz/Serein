//! Media Foundation capture; native callbacks copy one bounded frame, never encode.
#![allow(unsafe_code)]

mod directshow;

use super::{FRAME_INTERVAL, HEIGHT, Shared, WIDTH};
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
const MAX_FRAME_BYTES: usize = (WIDTH * 4 + 4096) * HEIGHT;
const INVALID: &str = "Camera did not provide a bounded 640×480 RGB frame";
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
				.map(|sample| copy_sample(sample, self.stride.load(Ordering::Acquire)))
				.transpose()
				.map(|rgb| ReadResult { changed, rgb })
		};
		// Only one ReadSample is outstanding; no COM sample is retained in the
		// channel. Capacity is one frame AND WIDTH * HEIGHT * 3 bytes.
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
	emit: &mut dyn FnMut(Vec<u8>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
	if shared.stopped.load(Ordering::Acquire) {
		return Ok(());
	}
	let _runtime = Runtime::open()?;
	if device.is_some_and(|id| id.starts_with("dshow:")) {
		return directshow::run(shared, device, emit);
	}
	let cameras = camera_sources()?;
	if cameras.is_empty() && device.is_none() {
		return directshow::run(shared, None, emit);
	}
	let source = selected_camera(cameras, device)?;
	let (send, receive) = mpsc::sync_channel(1);
	let stride = Arc::new(AtomicI32::new(0));
	let callback: IMFSourceReaderCallback = Callback {
		send,
		stride: stride.clone(),
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
			.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)
			.map_err(|_| UNAVAILABLE)?;
		let reader =
			MFCreateSourceReaderFromMediaSource(&source.0, &attributes).map_err(|_| UNAVAILABLE)?;
		reader
			.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
			.map_err(|_| UNAVAILABLE)?;
		// Native dimensions are selected first to bound upstream decoder input.
		let mut selected = false;
		for index in 0..256 {
			let Ok(native) = reader.GetNativeMediaType(VIDEO, index) else {
				break;
			};
			if native.GetUINT64(&MF_MT_FRAME_SIZE).ok() != Some(frame_size()) {
				continue;
			}
			if reader.SetCurrentMediaType(VIDEO, None, &native).is_ok() {
				selected = true;
				break;
			}
		}
		if !selected {
			return Err(
				"The selected Windows camera does not offer a supported 640×480 capture mode",
			);
		}
		let output = MFCreateMediaType().map_err(|_| INVALID)?;
		output
			.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
			.map_err(|_| INVALID)?;
		output
			.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
			.map_err(|_| INVALID)?;
		output
			.SetUINT64(&MF_MT_FRAME_SIZE, frame_size())
			.map_err(|_| INVALID)?;
		reader
			.SetCurrentMediaType(VIDEO, None, &output)
			.map_err(|_| "Windows could not convert this camera to RGB video")?;
		stride.store(output_stride(&reader)?, Ordering::Release);
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
			stride.store(output_stride(&reader)?, Ordering::Release);
		}
		if let Some(rgb) = result.rgb {
			last_frame = Instant::now();
			if !shared.stopped.load(Ordering::Acquire) {
				emit(rgb)?;
			}
		}
		if let Some(remaining) = FRAME_INTERVAL.checked_sub(requested.elapsed()) {
			thread::sleep(remaining);
		}
	}
	Ok(())
}

fn frame_size() -> u64 {
	((WIDTH as u64) << 32) | HEIGHT as u64
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

fn output_stride(reader: &IMFSourceReader) -> Result<i32, &'static str> {
	// SAFETY: Worker-owned reader. Check type and dimensions before using its stride.
	unsafe {
		let media = reader.GetCurrentMediaType(VIDEO).map_err(|_| INVALID)?;
		if media.GetUINT64(&MF_MT_FRAME_SIZE).map_err(|_| INVALID)? != frame_size()
			|| media.GetGUID(&MF_MT_SUBTYPE).map_err(|_| INVALID)? != MFVideoFormat_RGB32
		{
			return Err(INVALID);
		}
		let stride = media
			.GetUINT32(&MF_MT_DEFAULT_STRIDE)
			.map(|stride| stride as i32)
			.or_else(|_| MFGetStrideForBitmapInfoHeader(MFVideoFormat_RGB32.data1, WIDTH as u32))
			.map_err(|_| INVALID)?;
		if !(WIDTH * 4..=WIDTH * 4 + 4096).contains(&(stride.unsigned_abs() as usize)) {
			return Err(INVALID);
		}
		Ok(stride)
	}
}

fn copy_sample(sample: &IMFSample, stride: i32) -> Result<Vec<u8>, &'static str> {
	// SAFETY: Sample is borrowed only during its callback. Validate native byte
	// budgets before mapping/copying; every successful lock is unlocked on all paths.
	unsafe {
		if sample.GetBufferCount().map_err(|_| INVALID)? != 1
			|| sample.GetTotalLength().map_err(|_| INVALID)? as usize > MAX_FRAME_BYTES
		{
			return Err(INVALID);
		}
		let buffer = sample.GetBufferByIndex(0).map_err(|_| INVALID)?;
		if buffer.GetMaxLength().map_err(|_| INVALID)? as usize > MAX_FRAME_BYTES {
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
			let result = if base.is_null() || top.is_null() || length as usize > MAX_FRAME_BYTES {
				Err(INVALID)
			} else if let Some(first) = (top as usize).checked_sub(base as usize) {
				rgb_rows(
					std::slice::from_raw_parts(base, length as usize),
					first,
					pitch,
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
		let result = if base.is_null() || length > capacity || capacity as usize > MAX_FRAME_BYTES {
			Err(INVALID)
		} else {
			let first = if stride < 0 {
				stride.unsigned_abs() as usize * (HEIGHT - 1)
			} else {
				0
			};
			rgb_rows(
				std::slice::from_raw_parts(base, length as usize),
				first,
				stride,
			)
		};
		buffer.Unlock().map_err(|_| INVALID)?;
		result
	}
}

fn rgb_rows(bytes: &[u8], first: usize, stride: i32) -> Result<Vec<u8>, &'static str> {
	let pitch = stride.unsigned_abs() as usize;
	if !(WIDTH * 4..=WIDTH * 4 + 4096).contains(&pitch) || bytes.len() > MAX_FRAME_BYTES {
		return Err(INVALID);
	}
	let last = first
		.checked_add_signed(stride as isize * (HEIGHT - 1) as isize)
		.ok_or(INVALID)?;
	if first
		.max(last)
		.checked_add(WIDTH * 4)
		.is_none_or(|end| end > bytes.len())
	{
		return Err(INVALID);
	}
	let mut rgb = vec![0; WIDTH * HEIGHT * 3];
	for (y, dest) in rgb
		.as_chunks_mut::<{ WIDTH * 3 }>()
		.0
		.iter_mut()
		.enumerate()
	{
		let start = first
			.checked_add_signed(stride as isize * y as isize)
			.ok_or(INVALID)?;
		for (pixel, dest) in bytes[start..start + WIDTH * 4]
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
	fn rgb_rows_handle_padding_bottom_up_and_reject_invalid_bounds() {
		let pitch = WIDTH * 4 + 8;
		let mut bytes = vec![0; pitch * HEIGHT];
		bytes[..4].copy_from_slice(&[10, 20, 30, 255]);
		bytes[(HEIGHT - 1) * pitch..(HEIGHT - 1) * pitch + 4].copy_from_slice(&[40, 50, 60, 255]);
		assert_eq!(
			&rgb_rows(&bytes, 0, pitch as i32).unwrap()[..3],
			&[30, 20, 10]
		);
		assert_eq!(
			&rgb_rows(&bytes, (HEIGHT - 1) * pitch, -(pitch as i32)).unwrap()[..3],
			&[60, 50, 40]
		);
		assert!(rgb_rows(&bytes[..pitch], 0, pitch as i32).is_err());
		assert!(rgb_rows(&bytes, 0, -(pitch as i32)).is_err());
		assert!(rgb_rows(&bytes, usize::MAX, pitch as i32).is_err());
		assert!(rgb_rows(&bytes, 0, i32::MIN).is_err());
		assert!(rgb_rows(&bytes, 0, WIDTH as i32 * 4 - 1).is_err());
	}
}
