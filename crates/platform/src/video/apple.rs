//! macOS attachment decoding. The bounded Rust demuxer (`mp4`) owns every byte range;
//! VideoToolbox only ever receives individual compressed pictures plus their parameter
//! sets, and Symphonia decodes AAC in pure Rust. Nothing here touches URLs or files.
#![allow(unsafe_code)]

use super::{
	INVALID, Info, MAX_BYTES, ReadSeek, Sample, UNSUPPORTED,
	mp4::{self, AudioCodec, Movie, SampleEntry, VideoCodec},
};
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained};
use objc2_core_media::{
	CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMSampleTimingInfo, CMTime, CMTimeFlags,
	CMVideoFormatDescriptionCreateFromH264ParameterSets,
	CMVideoFormatDescriptionCreateFromHEVCParameterSets, kCMBlockBufferAssureMemoryNowFlag,
};
use objc2_core_video::{
	CVImageBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
	CVPixelBufferGetDataSize, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
	CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
	CVPixelBufferUnlockBaseAddress, kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA,
};
use objc2_video_toolbox::{
	VTDecodeFrameFlags, VTDecodeInfoFlags, VTDecompressionOutputCallbackRecord,
	VTDecompressionSession, kVTVideoDecoderBadDataErr,
};
use std::{
	ffi::c_void,
	io::SeekFrom,
	ptr::{NonNull, null, null_mut},
	sync::{Arc, Mutex},
};
use symphonia::core::{
	audio::{Channels, Position},
	codecs::audio::{
		AudioCodecParameters, AudioDecoder, AudioDecoderOptions, well_known::CODEC_ID_AAC,
	},
	packet::Packet,
	units::{Duration, Timestamp},
};

/// Decoded pictures held back until presentation order is certain. B-pyramids need three;
/// anything deeper is treated as a damaged stream instead of buffering more frames.
const MAX_REORDER: usize = 8;
const MAX_AUDIO_PACKET: usize = 64 * 1024;
const MAX_AUDIO_FRAMES: usize = 4096;

struct Decoded {
	/// Presentation time in video track ticks.
	pts: i64,
	width: u32,
	height: u32,
	rgba: Vec<u8>,
}

/// Shared with the VideoToolbox output callback: decoded frames or the first failure.
struct Output {
	timescale: u32,
	rotation: u32,
	frames: Vec<Decoded>,
	error: Option<&'static str>,
}

pub(super) struct Session(pub(super) CFRetained<VTDecompressionSession>);
impl Drop for Session {
	fn drop(&mut self) {
		// SAFETY: Invalidate stops callbacks before the shared output slot is released.
		unsafe { self.0.invalidate() };
	}
}

struct Audio {
	decoder: Box<dyn AudioDecoder>,
	cursor: usize,
	done: bool,
}

// Field order matters: the session must invalidate before `output` drops.
pub struct Decoder {
	session: Session,
	format: CFRetained<CMFormatDescription>,
	output: Arc<Mutex<Output>>,
	source: Box<dyn ReadSeek>,
	movie: Movie,
	info: Info,
	reorder: Vec<Decoded>,
	video_cursor: usize,
	video_done: bool,
	last_dts: i64,
	audio: Option<Audio>,
}

impl Decoder {
	pub fn open(mut source: Box<dyn ReadSeek>) -> Result<Self, &'static str> {
		let length = source.seek(SeekFrom::End(0)).map_err(|_| INVALID)?;
		let movie = mp4::parse(source.as_mut(), length)?;
		let format = format_description(&movie.video.track.codec)?;
		let output = Arc::new(Mutex::new(Output {
			timescale: movie.video.track.timescale,
			rotation: movie.video.rotation,
			frames: Vec::new(),
			error: None,
		}));
		let session = create_session(
			&format,
			Arc::as_ptr(&output).cast_mut().cast::<c_void>(),
			output_frame,
			kCVPixelFormatType_32BGRA,
			None,
		)?;
		let audio = match &movie.audio {
			Some(track) => Some(Audio {
				decoder: audio_decoder(track)?,
				cursor: 0,
				done: false,
			}),
			None => None,
		};
		let (width, height) = if matches!(movie.video.rotation, 90 | 270) {
			(movie.video.height, movie.video.width)
		} else {
			(movie.video.width, movie.video.height)
		};
		let info = Info {
			width,
			height,
			duration: movie.duration,
			sample_rate: movie.audio.as_ref().map_or(0, |a| a.sample_rate),
			channels: movie.audio.as_ref().map_or(0, |a| a.channels),
		};
		Ok(Self {
			session,
			format,
			output,
			source,
			movie,
			info,
			reorder: Vec::new(),
			video_cursor: 0,
			video_done: false,
			last_dts: i64::MIN,
			audio,
		})
	}

	pub fn info(&self) -> Info {
		self.info
	}

	pub fn seek(&mut self, seconds: f64) -> Result<(), &'static str> {
		if !seconds.is_finite() || seconds < 0.0 || seconds > self.info.duration {
			return Err(INVALID);
		}
		let video = &self.movie.video.track;
		let ticks = video.ticks(seconds);
		// Decoding restarts at the last keyframe at or before the target; the caller drops
		// or previews pictures that precede the target.
		self.video_cursor = video
			.samples
			.iter()
			.rposition(|sample| sample.sync && sample.pts <= ticks)
			.unwrap_or(0);
		self.video_done = false;
		self.last_dts = i64::MIN;
		self.reorder.clear();
		self.output.lock().map_err(|_| INVALID)?.frames.clear();
		if let (Some(audio), Some(track)) = (&mut self.audio, &self.movie.audio) {
			let ticks = track.track.ticks(seconds);
			audio.cursor = track
				.track
				.samples
				.iter()
				.rposition(|sample| sample.pts <= ticks)
				.unwrap_or(0);
			audio.done = false;
			audio.decoder.reset();
		}
		Ok(())
	}

	pub fn read_video(&mut self) -> Result<Option<Sample>, &'static str> {
		loop {
			if let Some(index) = self.ready_frame() {
				let frame = self.reorder.swap_remove(index);
				return Ok(Some(Sample::Video {
					pts: self.movie.video.track.seconds(frame.pts),
					width: frame.width,
					height: frame.height,
					rgba: frame.rgba,
				}));
			}
			if self.video_done {
				return Ok(None);
			}
			let Some(entry) = self
				.movie
				.video
				.track
				.samples
				.get(self.video_cursor)
				.copied()
			else {
				self.video_done = true;
				continue;
			};
			self.video_cursor += 1;
			self.decode_picture(entry)?;
			self.last_dts = i64::try_from(entry.dts).map_err(|_| INVALID)?;
			let mut output = self.output.lock().map_err(|_| INVALID)?;
			if let Some(error) = output.error {
				return Err(error);
			}
			self.reorder.append(&mut output.frames);
			if self.reorder.len() > MAX_REORDER {
				return Err(INVALID);
			}
		}
	}

	/// The earliest pending picture is final once decode time has passed its presentation
	/// time (later pictures cannot present earlier than they decode), once the buffer is full,
	/// or once the stream has ended.
	fn ready_frame(&self) -> Option<usize> {
		let (index, frame) = self
			.reorder
			.iter()
			.enumerate()
			.min_by_key(|(_, frame)| frame.pts)?;
		(self.video_done || self.reorder.len() >= MAX_REORDER || frame.pts <= self.last_dts)
			.then_some(index)
	}

	fn decode_picture(&mut self, entry: SampleEntry) -> Result<(), &'static str> {
		let bytes = self.read_sample(entry, MAX_BYTES)?;
		let timescale = i32::try_from(self.movie.video.track.timescale).map_err(|_| INVALID)?;
		let time = |value: i64| CMTime {
			value,
			timescale,
			flags: CMTimeFlags::Valid,
			epoch: 0,
		};
		let timing = CMSampleTimingInfo {
			duration: time(0),
			presentationTimeStamp: time(entry.pts),
			decodeTimeStamp: time(i64::try_from(entry.dts).map_err(|_| INVALID)?),
		};
		// SAFETY: Every out-pointer refers to an initialized local. The block buffer owns a
		// private copy of the sample bytes, so `bytes` may drop after this function returns.
		unsafe {
			let mut block: *mut CMBlockBuffer = null_mut();
			let status = CMBlockBuffer::create_with_memory_block(
				None,
				null_mut(),
				bytes.len(),
				None,
				null(),
				0,
				bytes.len(),
				kCMBlockBufferAssureMemoryNowFlag,
				NonNull::from(&mut block),
			);
			let block = NonNull::new(block)
				.filter(|_| status == 0)
				.map(|ptr| CFRetained::from_raw(ptr))
				.ok_or(INVALID)?;
			if CMBlockBuffer::replace_data_bytes(
				NonNull::from(bytes.as_slice()).cast::<c_void>(),
				&block,
				0,
				bytes.len(),
			) != 0
			{
				return Err(INVALID);
			}
			let sizes = [bytes.len()];
			let mut sample: *mut CMSampleBuffer = null_mut();
			let status = CMSampleBuffer::create_ready(
				None,
				Some(&block),
				Some(&self.format),
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
			// Synchronous decode: the output callback runs before this call returns.
			let status =
				self.session
					.0
					.decode_frame(&sample, VTDecodeFrameFlags(0), null_mut(), &mut flags);
			match status {
				0 => Ok(()),
				status if status == kVTVideoDecoderBadDataErr => Err(INVALID),
				_ => Err(UNSUPPORTED),
			}
		}
	}

	pub fn read_audio(&mut self) -> Result<Option<Sample>, &'static str> {
		let Some(track) = self.movie.audio.as_ref() else {
			return Ok(None);
		};
		let Some(audio) = self.audio.as_mut() else {
			return Ok(None);
		};
		if audio.done {
			return Ok(None);
		}
		let Some(entry) = track.track.samples.get(audio.cursor).copied() else {
			audio.done = true;
			return Ok(None);
		};
		audio.cursor += 1;
		let rate = track.sample_rate;
		let channels = usize::from(track.channels);
		let timescale = track.track.timescale;
		let pts = track.track.seconds(entry.pts);
		let bytes = self.read_sample(entry, MAX_AUDIO_PACKET)?;
		let audio = self.audio.as_mut().ok_or(INVALID)?;
		let packet = Packet::new(0, Timestamp::new(entry.pts), Duration::new(1024), bytes);
		let decoded = audio.decoder.decode(&packet).map_err(|_| INVALID)?;
		if decoded.spec().rate() != rate
			|| decoded.spec().channels().count() != channels
			|| decoded.frames() > MAX_AUDIO_FRAMES
			|| timescale != rate
		{
			return Err(UNSUPPORTED);
		}
		let mut interleaved = vec![0.0_f32; decoded.samples_interleaved()];
		decoded.copy_to_slice_interleaved(&mut interleaved);
		let frames = interleaved
			.chunks_exact(channels)
			.map(|frame| {
				let left = frame[0];
				let right = if channels == 2 { frame[1] } else { left };
				[left, right].map(|x| {
					if x.is_finite() {
						x.clamp(-1.0, 1.0)
					} else {
						0.0
					}
				})
			})
			.collect();
		Ok(Some(Sample::Audio { pts, frames }))
	}

	fn read_sample(&mut self, entry: SampleEntry, limit: usize) -> Result<Vec<u8>, &'static str> {
		let size = entry.size as usize;
		if size == 0 || size > limit {
			return Err(INVALID);
		}
		self.source
			.seek(SeekFrom::Start(entry.offset))
			.map_err(|_| INVALID)?;
		let mut bytes = vec![0; size];
		self.source.read_exact(&mut bytes).map_err(|_| INVALID)?;
		Ok(bytes)
	}
}

fn format_description(codec: &VideoCodec) -> Result<CFRetained<CMFormatDescription>, &'static str> {
	let (sets, nal_length, hevc) = match codec {
		VideoCodec::H264 {
			nal_length,
			parameter_sets,
		} => (parameter_sets, *nal_length, false),
		VideoCodec::Hevc {
			nal_length,
			parameter_sets,
		} => (parameter_sets, *nal_length, true),
	};
	let mut pointers: Vec<NonNull<u8>> = sets
		.iter()
		.map(|set| NonNull::from(set.as_slice()).cast::<u8>())
		.collect();
	let mut sizes: Vec<usize> = sets.iter().map(Vec::len).collect();
	let mut out: *const CMFormatDescription = null();
	// SAFETY: Parameter-set pointers and sizes stay live for the call; `out` is initialized.
	let status = unsafe {
		let pointers = NonNull::new(pointers.as_mut_ptr()).ok_or(INVALID)?;
		let sizes = NonNull::new(sizes.as_mut_ptr()).ok_or(INVALID)?;
		if hevc {
			CMVideoFormatDescriptionCreateFromHEVCParameterSets(
				None,
				sets.len(),
				pointers,
				sizes,
				i32::from(nal_length),
				None,
				NonNull::from(&mut out),
			)
		} else {
			CMVideoFormatDescriptionCreateFromH264ParameterSets(
				None,
				sets.len(),
				pointers,
				sizes,
				i32::from(nal_length),
				NonNull::from(&mut out),
			)
		}
	};
	if status != 0 {
		return Err(UNSUPPORTED);
	}
	// SAFETY: Create functions return a +1 reference on success.
	NonNull::new(out.cast_mut())
		.map(|ptr| unsafe { CFRetained::from_raw(ptr) })
		.ok_or(UNSUPPORTED)
}

pub(super) fn create_session(
	format: &CMFormatDescription,
	refcon: *mut c_void,
	callback: unsafe extern "C-unwind" fn(
		*mut c_void,
		*mut c_void,
		i32,
		VTDecodeInfoFlags,
		*mut CVImageBuffer,
		CMTime,
		CMTime,
	),
	pixel_format: u32,
	decoder_specification: Option<&CFDictionary>,
) -> Result<Session, &'static str> {
	let pixel_format = CFNumber::new_i32(pixel_format as i32);
	// SAFETY: The static key is a valid CFString; the dictionary is a plain attribute map.
	let attributes = unsafe {
		CFDictionary::from_slices(&[kCVPixelBufferPixelFormatTypeKey], &[&*pixel_format])
	};
	let record = VTDecompressionOutputCallbackRecord {
		decompressionOutputCallback: Some(callback),
		decompressionOutputRefCon: refcon,
	};
	let mut session: *mut VTDecompressionSession = null_mut();
	// SAFETY: The callback record and attribute dictionary outlive the call; `refcon` points
	// at the decoder's `Arc<Mutex<Output>>`, which outlives the session (see field order).
	let status = unsafe {
		VTDecompressionSession::create(
			None,
			format,
			decoder_specification,
			Some(attributes.as_opaque()),
			&record,
			NonNull::from(&mut session),
		)
	};
	if status != 0 {
		return Err(UNSUPPORTED);
	}
	// SAFETY: Create returns a +1 reference on success.
	NonNull::new(session)
		.map(|ptr| Session(unsafe { CFRetained::from_raw(ptr) }))
		.ok_or(UNSUPPORTED)
}

unsafe extern "C-unwind" fn output_frame(
	refcon: *mut c_void,
	_frame_refcon: *mut c_void,
	status: i32,
	flags: VTDecodeInfoFlags,
	image: *mut CVImageBuffer,
	pts: CMTime,
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
	if output.error.is_some() {
		return;
	}
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
	if !pts.flags.contains(CMTimeFlags::Valid) || pts.timescale <= 0 {
		output.error = Some(INVALID);
		return;
	}
	let ticks = if pts.timescale as u32 == output.timescale {
		pts.value
	} else {
		(pts.value as f64 / f64::from(pts.timescale) * f64::from(output.timescale)).round() as i64
	};
	// SAFETY: VideoToolbox keeps the image buffer alive for the duration of the callback.
	match unsafe { copy_rgba(&*image) } {
		Ok((width, height, rgba)) => {
			let (width, height, rgba) = if output.rotation == 0 {
				(width, height, rgba)
			} else {
				super::rotate_rgba(&rgba, width, height, output.rotation)
			};
			output.frames.push(Decoded {
				pts: ticks,
				width,
				height,
				rgba,
			});
		}
		Err(error) => output.error = Some(error),
	}
}

/// Copy one BGRA picture into a tightly packed RGBA buffer after checking every bound.
pub(super) unsafe fn copy_rgba(
	pixels: &CVImageBuffer,
) -> Result<(u32, u32, Vec<u8>), &'static str> {
	let width = CVPixelBufferGetWidth(pixels);
	let height = CVPixelBufferGetHeight(pixels);
	let stride = CVPixelBufferGetBytesPerRow(pixels);
	let (w, h) = (
		u32::try_from(width).map_err(|_| INVALID)?,
		u32::try_from(height).map_err(|_| INVALID)?,
	);
	super::check_dimensions(w, h)?;
	let row_bytes = width * 4;
	let total = stride.checked_mul(height).ok_or(INVALID)?;
	if CVPixelBufferGetPixelFormatType(pixels) != kCVPixelFormatType_32BGRA
		|| stride < row_bytes
		|| total > MAX_BYTES
		|| total > CVPixelBufferGetDataSize(pixels)
	{
		return Err(INVALID);
	}
	// SAFETY: The buffer is locked read-only for the copy and always unlocked afterwards;
	// the byte count was validated against the buffer's own data size.
	unsafe {
		if CVPixelBufferLockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly) != 0 {
			return Err(INVALID);
		}
		let base = CVPixelBufferGetBaseAddress(pixels);
		let result = if base.is_null() {
			Err(INVALID)
		} else {
			let source = std::slice::from_raw_parts(base.cast::<u8>(), total);
			let mut rgba = vec![0; width * height * 4];
			for (row, target) in source
				.chunks_exact(stride)
				.zip(rgba.chunks_exact_mut(row_bytes))
			{
				for (pixel, out) in row[..row_bytes]
					.as_chunks::<4>()
					.0
					.iter()
					.zip(target.as_chunks_mut::<4>().0.iter_mut())
				{
					*out = [pixel[2], pixel[1], pixel[0], 255];
				}
			}
			Ok((w, h, rgba))
		};
		CVPixelBufferUnlockBaseAddress(pixels, CVPixelBufferLockFlags::ReadOnly);
		result
	}
}

fn audio_decoder(track: &mp4::AudioTrack) -> Result<Box<dyn AudioDecoder>, &'static str> {
	let AudioCodec::Aac { config } = &track.track.codec;
	let positions = Position::from_count(u32::from(track.channels)).ok_or(UNSUPPORTED)?;
	let mut parameters = AudioCodecParameters::new();
	parameters
		.for_codec(CODEC_ID_AAC)
		.with_sample_rate(track.sample_rate)
		.with_channels(Channels::Positioned(positions))
		.with_extra_data(config.clone().into_boxed_slice());
	let mut options = AudioDecoderOptions::default();
	options.gapless = false;
	symphonia::default::get_codecs()
		.make_audio_decoder(&parameters, &options)
		.map_err(|_| UNSUPPORTED)
}

#[cfg(test)]
mod tests {
	use super::*;
	const FIXTURE: &[u8] = include_bytes!("../../../../apps/desktop/tests/fixtures/video.mov");

	#[test]
	fn native_mov_decodes_audio_video_and_seeks() {
		let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(FIXTURE))).unwrap();
		let info = decoder.info();
		assert_eq!(
			(info.width, info.height, info.sample_rate, info.channels),
			(320, 180, 48_000, 1)
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
						assert_eq!((width, height), (320, 180));
						assert_eq!(rgba.len(), (width * height * 4) as usize);
						assert!(rgba.windows(4).any(|pixel| pixel[0] != pixel[1]));
						assert!(pts > last_pts, "{pts} after {last_pts}");
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
		assert!(audio_frames >= 140_000, "{audio_frames}");
		assert!(decoder.read_video().unwrap().is_none());
		assert!(decoder.read_audio().unwrap().is_none());
		eprintln!(
			"VideoToolbox MOV decode: 72 frames + {audio_frames} PCM frames in {:.3} ms",
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
		assert!(
			matches!(decoder.read_audio().unwrap(), Some(Sample::Audio { pts, .. }) if pts <= 1.0)
		);
		assert!(decoder.seek(f64::NAN).is_err());
		drop(decoder);
		// Rotate only the fixture's video-track matrix, keeping the same compressed samples.
		let mut portrait = FIXTURE.to_vec();
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

	/// Developer check for real-world files: `SEREIN_VIDEO_SAMPLE=/path/clip.mp4 cargo test
	/// -p platform decodes_local_sample -- --ignored --nocapture`.
	#[test]
	#[ignore = "decodes a developer-supplied local clip"]
	fn decodes_local_sample() {
		let path = std::env::var("SEREIN_VIDEO_SAMPLE").expect("SEREIN_VIDEO_SAMPLE path");
		let bytes = std::fs::read(path).unwrap();
		let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(bytes))).unwrap();
		let info = decoder.info();
		eprintln!("{info:?}");
		let started = std::time::Instant::now();
		let (mut videos, mut audio_frames, mut last_pts) = (0, 0_usize, -1.0);
		loop {
			let video = decoder.read_video().unwrap();
			let audio = decoder.read_audio().unwrap();
			if video.is_none() && audio.is_none() {
				break;
			}
			if let Some(Sample::Video {
				pts, width, height, ..
			}) = video
			{
				assert_eq!((width, height), (info.width, info.height));
				assert!(pts > last_pts, "{pts} after {last_pts}");
				last_pts = pts;
				videos += 1;
			}
			if let Some(Sample::Audio { frames, .. }) = audio {
				audio_frames += frames.len();
			}
		}
		eprintln!(
			"{videos} frames, {audio_frames} PCM frames, last pts {last_pts:.3}s in {:.0} ms",
			started.elapsed().as_secs_f64() * 1000.0
		);
		assert!(videos > 0);
		decoder.seek(info.duration / 2.0).unwrap();
		assert!(decoder.read_video().unwrap().is_some());
	}

	#[test]
	fn silent_and_short_audio_variants_open() {
		let silent = include_bytes!("../../../../apps/desktop/tests/fixtures/video-silent.mov");
		let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(silent.as_slice()))).unwrap();
		assert_eq!(decoder.info().sample_rate, 0);
		assert!(decoder.read_audio().unwrap().is_none());
		assert!(decoder.read_video().unwrap().is_some());
		let short = include_bytes!("../../../../apps/desktop/tests/fixtures/video-short-audio.mov");
		let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(short.as_slice()))).unwrap();
		let mut frames = 0;
		while let Some(Sample::Audio { frames: pcm, .. }) = decoder.read_audio().unwrap() {
			frames += pcm.len();
		}
		assert!((20_000..30_000).contains(&frames), "{frames}");
	}
}
