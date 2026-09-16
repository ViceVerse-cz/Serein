//! Native V4L2 single-plane streaming; no capture helper process or recording.
#![allow(unsafe_code)]

use super::{FRAME_INTERVAL, HEIGHT, Shared, WIDTH, format};
use image::{ImageDecoder, codecs::jpeg::JpegDecoder};
use std::{
	fs::{File, OpenOptions},
	io::{self, Cursor},
	os::{
		fd::AsRawFd,
		unix::fs::{FileTypeExt, OpenOptionsExt},
	},
	sync::atomic::Ordering,
	time::{Duration, Instant},
};

const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_BUFFERS: u32 = 4;
const CAPTURE: u32 = 1;
const MMAP: u32 = 1;
const YUYV: u32 = u32::from_le_bytes(*b"YUYV");
const MJPEG: u32 = u32::from_le_bytes(*b"MJPG");
const DENIED: &str = "Camera access denied. Allow access to /dev/video devices in your Linux session or sandbox settings, then try again.";

// Minimal ABI declarations for linux/videodev2.h. The format union includes a
// pointer-aligned member in the UAPI; buffer timestamp uses the native timeval.
#[repr(C)]
#[derive(Default)]
struct Capability {
	driver: [u8; 16],
	card: [u8; 32],
	bus: [u8; 32],
	version: u32,
	capabilities: u32,
	device_caps: u32,
	reserved: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Pixels {
	width: u32,
	height: u32,
	format: u32,
	field: u32,
	stride: u32,
	size: u32,
	colorspace: u32,
	private: u32,
	flags: u32,
	ycbcr: u32,
	quantization: u32,
	transfer: u32,
}

// v4l2_streamparm: type followed by the 200-byte capture/output union.
#[repr(C)]
struct StreamParameters {
	kind: u32,
	capability: u32,
	mode: u32,
	numerator: u32,
	denominator: u32,
	rest: [u32; 46],
}

#[repr(C)]
union FormatData {
	pixels: Pixels,
	raw: [u8; 200],
	_alignment: usize,
}

#[repr(C)]
struct Format {
	kind: u32,
	data: FormatData,
}

#[repr(C)]
#[derive(Default)]
struct RequestBuffers {
	count: u32,
	kind: u32,
	memory: u32,
	capabilities: u32,
	flags: u8,
	reserved: [u8; 3],
}

#[repr(C)]
union BufferLocation {
	offset: u32,
	_alignment: usize,
}

#[repr(C)]
struct Buffer {
	index: u32,
	kind: u32,
	used: u32,
	flags: u32,
	field: u32,
	timestamp: libc::timeval,
	timecode: [u32; 4],
	sequence: u32,
	memory: u32,
	location: BufferLocation,
	length: u32,
	reserved: u32,
	request: u32,
}

impl Buffer {
	fn new(index: u32) -> Self {
		// SAFETY: This UAPI struct contains only integers and timeval integers.
		let mut buffer: Self = unsafe { std::mem::zeroed() };
		buffer.index = index;
		buffer.kind = CAPTURE;
		buffer.memory = MMAP;
		buffer
	}
}

fn control<T>(file: &File, number: u32, value: &mut T) -> io::Result<()> {
	let request = match number {
		0 => libc::_IOR::<T>(b'V' as u32, number),
		18 | 19 => libc::_IOW::<T>(b'V' as u32, number),
		_ => libc::_IOWR::<T>(b'V' as u32, number),
	};
	// SAFETY: All callers below pair a fixed V4L2 request with its repr(C) UAPI
	// argument (or STREAMON/OFF u32). Its size is encoded in the ioctl request;
	// the kernel borrows the argument only for the duration of this call.
	if unsafe { libc::ioctl(file.as_raw_fd(), request, value as *mut T) } == -1 {
		Err(io::Error::last_os_error())
	} else {
		Ok(())
	}
}

struct Mapping {
	address: *mut libc::c_void,
	length: usize,
}

impl Drop for Mapping {
	fn drop(&mut self) {
		// SAFETY: Exactly the successful mapping retained by this owner.
		unsafe { libc::munmap(self.address, self.length) };
	}
}

struct Capture {
	file: File,
	mappings: Vec<Mapping>,
	pixels: Pixels,
	streaming: bool,
}

impl Drop for Capture {
	fn drop(&mut self) {
		if self.streaming {
			let mut kind = CAPTURE;
			let _ = control(&self.file, 19, &mut kind);
		}
		// Unmap before closing the file, including partially configured sessions.
		self.mappings.clear();
	}
}

fn validate_format(pixels: Pixels) -> Result<Pixels, &'static str> {
	if format::rank(
		pixels.width as usize,
		pixels.height as usize,
		f64::from(format::FPS),
	)
	.is_none()
		|| pixels.field != 1
		|| pixels.size == 0
		|| pixels.size as usize > MAX_FRAME_BYTES
	{
		return Err("Camera does not support bounded progressive capture up to 1280×720");
	}
	match pixels.format {
		MJPEG => {}
		YUYV if pixels.width.is_multiple_of(2)
			&& (pixels.width as usize * 2..=pixels.width as usize * 2 + 4096)
				.contains(&(pixels.stride as usize))
			&& pixels.stride as usize * pixels.height as usize <= pixels.size as usize
			&& matches!(pixels.ycbcr, 0..=2)
			&& (pixels.ycbcr != 0 || matches!(pixels.colorspace, 0 | 1 | 3 | 7 | 8))
			&& matches!(pixels.quantization, 0..=2) => {}
		_ => return Err("Camera requires an unsupported pixel format; use a YUYV or MJPEG camera"),
	}
	Ok(pixels)
}

fn configure(file: File) -> Result<Capture, &'static str> {
	let mut capabilities = Capability::default();
	control(&file, 0, &mut capabilities).map_err(|_| "Device is not a V4L2 camera")?;
	let caps = if capabilities.capabilities & 0x8000_0000 != 0 {
		capabilities.device_caps
	} else {
		capabilities.capabilities
	};
	if caps & CAPTURE == 0 || caps & 0x0400_0000 == 0 {
		return Err("Camera does not support V4L2 single-plane streaming");
	}
	let mut choices = Vec::with_capacity(2);
	for format in [YUYV, MJPEG] {
		let mut request = Format {
			kind: CAPTURE,
			data: FormatData { raw: [0; 200] },
		};
		request.data.pixels = Pixels {
			width: WIDTH as u32,
			height: HEIGHT as u32,
			format,
			field: 1,
			colorspace: 1,
			..Pixels::default()
		};
		if control(&file, 64, &mut request).is_ok() {
			// SAFETY: CAPTURE selects the pixels member of v4l2_format.
			if let Ok(pixels) = validate_format(unsafe { request.data.pixels })
				&& let Some(rank) = format::rank(
					pixels.width as usize,
					pixels.height as usize,
					f64::from(format::FPS),
				) {
				choices.push((rank, request));
			}
		}
	}
	choices.sort_by_key(|(rank, _)| *rank);
	let mut negotiated = Err("Camera does not support YUYV or MJPEG capture up to 1280×720");
	for (_, mut request) in choices {
		if control(&file, 5, &mut request).is_ok() {
			// SAFETY: CAPTURE selects the pixels member, including driver adjustments.
			negotiated = validate_format(unsafe { request.data.pixels });
			if negotiated.is_ok() {
				break;
			}
		}
	}
	let pixels = negotiated?;
	// S_PARM lets the driver choose its nearest supported interval. Fixed-rate
	// devices may not implement it; worker pacing still bounds encoding.
	let mut timing = StreamParameters {
		kind: CAPTURE,
		capability: 0,
		mode: 0,
		numerator: 1,
		denominator: format::FPS,
		rest: [0; 46],
	};
	if let Err(error) = control(&file, 22, &mut timing)
		&& !matches!(error.raw_os_error(), Some(libc::EINVAL | libc::ENOTTY))
	{
		return Err("Camera frame rate could not be configured");
	}
	let mut capture = Capture {
		file,
		mappings: Vec::new(),
		pixels,
		streaming: false,
	};
	let mut request = RequestBuffers {
		count: 2,
		kind: CAPTURE,
		memory: MMAP,
		..RequestBuffers::default()
	};
	control(&capture.file, 8, &mut request)
		.map_err(|_| "Camera is busy or cannot allocate capture buffers")?;
	if !(1..=MAX_BUFFERS).contains(&request.count) {
		return Err("Camera requested too many native capture buffers");
	}
	for index in 0..request.count {
		let mut buffer = Buffer::new(index);
		control(&capture.file, 9, &mut buffer).map_err(|_| "Camera buffer could not be queried")?;
		let length = buffer.length as usize;
		if length < pixels.size as usize || length > MAX_FRAME_BYTES {
			return Err("Camera native buffer exceeds its 4 MiB limit");
		}
		// SAFETY: Driver-provided offset from QUERYBUF, validated bounded length,
		// live file. The mapping is released by its unique owner on every exit.
		let address = unsafe {
			libc::mmap(
				std::ptr::null_mut(),
				length,
				libc::PROT_READ | libc::PROT_WRITE,
				libc::MAP_SHARED,
				capture.file.as_raw_fd(),
				buffer.location.offset as libc::off_t,
			)
		};
		if address == libc::MAP_FAILED {
			return Err("Camera native buffer could not be mapped");
		}
		let mapping = Mapping { address, length };
		if address.is_null() {
			return Err("Camera native buffer has an invalid base address");
		}
		capture.mappings.push(mapping);
		control(&capture.file, 15, &mut buffer).map_err(|_| "Camera buffer could not be queued")?;
	}
	let mut kind = CAPTURE;
	control(&capture.file, 18, &mut kind)
		.map_err(|_| "Camera is busy or capture could not start")?;
	capture.streaming = true;
	Ok(capture)
}

fn open(shared: &Shared) -> Result<Option<Capture>, &'static str> {
	let mut error = "No camera is available in /dev/video0 through /dev/video63";
	// ponytail: first usable native camera; add a picker when device selection is requested.
	for index in 0..64 {
		if shared.stopped.load(Ordering::Acquire) {
			return Ok(None);
		}
		let file = match OpenOptions::new()
			.read(true)
			.write(true)
			.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
			.open(format!("/dev/video{index}"))
		{
			Ok(file) => file,
			Err(reason) if reason.kind() == io::ErrorKind::NotFound => continue,
			Err(reason) if reason.kind() == io::ErrorKind::PermissionDenied => {
				error = DENIED;
				continue;
			}
			Err(_) => {
				error = "Camera is busy or unavailable";
				continue;
			}
		};
		if !file
			.metadata()
			.is_ok_and(|meta| meta.file_type().is_char_device())
		{
			continue;
		}
		match configure(file) {
			Ok(capture) => return Ok(Some(capture)),
			Err(reason) if error != DENIED => error = reason,
			Err(_) => {}
		}
	}
	Err(error)
}

pub(super) fn run(
	shared: &Shared,
	emit: &mut dyn FnMut(Vec<u8>) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
	let Some(capture) = open(shared)? else {
		return Ok(());
	};
	let mut last_frame = Instant::now();
	let mut last_emitted = Instant::now() - FRAME_INTERVAL;
	while !shared.stopped.load(Ordering::Acquire) {
		if last_frame.elapsed() >= Duration::from_secs(5) {
			return Err("Camera stopped delivering frames; check the device and try again");
		}
		let mut poll = libc::pollfd {
			fd: capture.file.as_raw_fd(),
			events: libc::POLLIN,
			revents: 0,
		};
		// SAFETY: One live descriptor and a valid one-element pollfd buffer.
		let ready = unsafe { libc::poll(&mut poll, 1, 100) };
		if ready < 0 {
			if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
				continue;
			}
			return Err("Camera device polling failed");
		}
		if poll.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
			return Err("Camera disconnected or capture failed");
		}
		if ready == 0 || shared.stopped.load(Ordering::Acquire) {
			continue;
		}
		let mut buffer = Buffer::new(0);
		if let Err(reason) = control(&capture.file, 17, &mut buffer) {
			if matches!(
				reason.kind(),
				io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
			) {
				continue;
			}
			return Err("Camera frame could not be read");
		}
		let mapping = capture
			.mappings
			.get(buffer.index as usize)
			.ok_or("Camera returned an invalid buffer")?;
		let used = buffer.used as usize;
		if used == 0 || used > mapping.length || used > capture.pixels.size as usize {
			return Err("Camera frame exceeds its native buffer bounds");
		}
		let rgb = if buffer.flags & 0x40 == 0 && last_emitted.elapsed() >= FRAME_INTERVAL {
			// SAFETY: DQBUF gives exclusive userspace access until QBUF. Length
			// was checked against the retained mapping; decoding finishes first.
			let bytes = unsafe { std::slice::from_raw_parts(mapping.address.cast::<u8>(), used) };
			Some(decode(bytes, capture.pixels)?)
		} else {
			None
		};
		control(&capture.file, 15, &mut buffer)
			.map_err(|_| "Camera frame could not be requeued")?;
		if let Some(rgb) = rgb {
			last_frame = Instant::now();
			if !shared.stopped.load(Ordering::Acquire) {
				last_emitted = Instant::now();
				emit(rgb)?;
			}
		}
	}
	Ok(())
}

fn decode(bytes: &[u8], pixels: Pixels) -> Result<Vec<u8>, &'static str> {
	validate_format(pixels)?;
	let (width, height) = (pixels.width as usize, pixels.height as usize);
	if bytes.is_empty() || bytes.len() > MAX_FRAME_BYTES || bytes.len() > pixels.size as usize {
		return Err("Camera frame exceeds bounds");
	}
	if pixels.format == MJPEG {
		let mut decoder =
			JpegDecoder::new(Cursor::new(bytes)).map_err(|_| "Camera JPEG header is invalid")?;
		if decoder.dimensions() != (pixels.width, pixels.height)
			|| decoder.color_type() != image::ColorType::Rgb8
		{
			return Err("Camera JPEG does not match the negotiated RGB dimensions");
		}
		let mut limits = image::Limits::default();
		limits.max_image_width = Some(pixels.width);
		limits.max_image_height = Some(pixels.height);
		limits.max_alloc = Some(MAX_FRAME_BYTES as u64);
		decoder
			.set_limits(limits)
			.map_err(|_| "Camera JPEG exceeds decode bounds")?;
		let mut rgb = vec![0; width * height * 3];
		decoder
			.read_image(&mut rgb)
			.map_err(|_| "Camera JPEG could not be decoded")?;
		return fit_rgb(rgb, pixels);
	}
	let stride = pixels.stride as usize;
	if bytes.len() < stride * (height - 1) + width * 2 {
		return Err("Camera YUYV frame is truncated");
	}
	let rec709 = pixels.ycbcr == 2 || (pixels.ycbcr == 0 && pixels.colorspace == 3);
	let full = pixels.quantization == 1 || (pixels.quantization == 0 && pixels.colorspace == 7);
	let (scale, offset, red, green_u, green_v, blue) = match (rec709, full) {
		(false, false) => (298, 16, 409, 100, 208, 516),
		(true, false) => (298, 16, 459, 55, 136, 541),
		(false, true) => (256, 0, 359, 88, 183, 454),
		(true, true) => (256, 0, 403, 48, 120, 475),
	};
	let mut rgb = vec![0; width * height * 3];
	for (source, target) in bytes.chunks(stride).zip(rgb.chunks_exact_mut(width * 3)) {
		for (pair, output) in source[..width * 2]
			.as_chunks::<4>()
			.0
			.iter()
			.zip(target.as_chunks_mut::<6>().0)
		{
			let u = i32::from(pair[1]) - 128;
			let v = i32::from(pair[3]) - 128;
			for (y, out) in [pair[0], pair[2]]
				.into_iter()
				.zip(output.as_chunks_mut::<3>().0)
			{
				let y = (i32::from(y) - offset) * scale;
				out[0] = ((y + red * v + 128) >> 8).clamp(0, 255) as u8;
				out[1] = ((y - green_u * u - green_v * v + 128) >> 8).clamp(0, 255) as u8;
				out[2] = ((y + blue * u + 128) >> 8).clamp(0, 255) as u8;
			}
		}
	}
	fit_rgb(rgb, pixels)
}

fn fit_rgb(rgb: Vec<u8>, pixels: Pixels) -> Result<Vec<u8>, &'static str> {
	if pixels.width == WIDTH as u32 && pixels.height == HEIGHT as u32 {
		return Ok(rgb);
	}
	let source = image::RgbImage::from_raw(pixels.width, pixels.height, rgb)
		.ok_or("Camera RGB dimensions are invalid")?;
	let fitted = image::DynamicImage::ImageRgb8(source)
		.thumbnail(WIDTH as u32, HEIGHT as u32)
		.into_rgb8();
	let mut output = image::RgbImage::new(WIDTH as u32, HEIGHT as u32);
	image::imageops::replace(
		&mut output,
		&fitted,
		i64::from((WIDTH as u32 - fitted.width()) / 2),
		i64::from((HEIGHT as u32 - fitted.height()) / 2),
	);
	Ok(output.into_raw())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn canceled_capture_does_not_open_a_device_or_emit() {
		let shared = Shared::default();
		shared.stopped.store(true, Ordering::Release);
		assert!(run(&shared, &mut |_| panic!("Canceled camera emitted a frame")).is_ok());
	}

	#[test]
	fn yuyv_conversion_honors_padding_and_rejects_invalid_bounds() {
		let mut pixels = Pixels {
			width: WIDTH as u32,
			height: HEIGHT as u32,
			format: YUYV,
			field: 1,
			stride: (WIDTH * 2 + 8) as u32,
			size: ((WIDTH * 2 + 8) * HEIGHT) as u32,
			..Pixels::default()
		};
		let mut bytes = vec![0; pixels.size as usize];
		for row in bytes.chunks_exact_mut(pixels.stride as usize) {
			for pair in row[..WIDTH * 2].as_chunks_mut::<4>().0 {
				pair.copy_from_slice(&[16, 128, 235, 128]);
			}
		}
		let rgb = decode(&bytes, pixels).unwrap();
		assert_eq!(rgb.len(), WIDTH * HEIGHT * 3);
		assert!(
			rgb.as_chunks::<6>()
				.0
				.iter()
				.all(|pair| *pair == [0, 0, 0, 255, 255, 255])
		);
		assert!(decode(&bytes[..100], pixels).is_err());
		pixels.stride = u32::MAX;
		assert!(decode(&bytes, pixels).is_err());
		pixels.size = u32::MAX;
		assert!(decode(&bytes, pixels).is_err());
	}

	#[test]
	fn jpeg_decode_checks_dimensions_before_output_allocation() {
		let pixels = Pixels {
			width: WIDTH as u32,
			height: HEIGHT as u32,
			format: MJPEG,
			field: 1,
			size: MAX_FRAME_BYTES as u32,
			..Pixels::default()
		};
		let mut bytes = Vec::new();
		image::codecs::jpeg::JpegEncoder::new(&mut bytes)
			.encode(
				&vec![120; WIDTH * HEIGHT * 3],
				WIDTH as u32,
				HEIGHT as u32,
				image::ExtendedColorType::Rgb8,
			)
			.unwrap();
		assert_eq!(decode(&bytes, pixels).unwrap().len(), WIDTH * HEIGHT * 3);
		bytes.clear();
		image::codecs::jpeg::JpegEncoder::new(&mut bytes)
			.encode(&[120; 12], 2, 2, image::ExtendedColorType::Rgb8)
			.unwrap();
		assert!(decode(&bytes, pixels).is_err());
		assert!(decode(&[0; 20], pixels).is_err());
	}

	#[test]
	fn native_uapi_layouts_match_linux_headers() {
		assert_eq!(std::mem::size_of::<Capability>(), 104);
		assert_eq!(std::mem::size_of::<RequestBuffers>(), 20);
		assert_eq!(std::mem::size_of::<Pixels>(), 48);
		if cfg!(target_pointer_width = "64") {
			assert_eq!(std::mem::size_of::<Format>(), 208);
			assert_eq!(std::mem::size_of::<Buffer>(), 88);
			assert_eq!(std::mem::offset_of!(Buffer, location), 64);
		}
	}
}
