//! Linux attachment decoding through GStreamer. `appsrc` pulls bounded byte ranges from the
//! caller's anonymous stream (never a URL), `decodebin` picks the installed codecs, and two
//! unsynchronised `appsink`s hand back RGBA pictures and 48 kHz stereo float PCM on demand.
//! The same distributions that ship WebKitGTK for the login page also ship these plugins.
use super::{INVALID, Info, MAX_BYTES, MAX_SECONDS, ReadSeek, Sample, TOO_LONG, UNSUPPORTED};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video::{self as gst_video, VideoFrameExt};
use std::{
	io::SeekFrom,
	sync::{
		Arc, Mutex,
		atomic::{AtomicBool, AtomicU64, Ordering},
	},
	task::Poll,
};

const CHUNK: usize = 64 * 1024;
const MAX_CHUNK: usize = 1024 * 1024;
const AUDIO_RATE: u32 = 48_000;
const AUDIO_CHANNELS: usize = 2;
const WAIT: gst::ClockTime = gst::ClockTime::from_seconds(20);
const SEEK_FAILED: &str = "This video cannot seek to that position.";

/// A constructor can fail after native streaming has started, before Decoder owns cleanup.
pub(super) struct PipelineGuard(Option<gst::Pipeline>);

impl PipelineGuard {
	pub(super) fn new(pipeline: &gst::Pipeline) -> Self {
		Self(Some(pipeline.clone()))
	}

	pub(super) fn release(mut self) {
		self.0.take();
	}
}

impl Drop for PipelineGuard {
	fn drop(&mut self) {
		if let Some(pipeline) = self.0.take() {
			let _ = pipeline.set_state(gst::State::Null);
		}
	}
}

/// Bytes shared with the `appsrc` callbacks, which run on GStreamer's streaming threads.
struct Shared {
	source: Mutex<Box<dyn ReadSeek>>,
	position: AtomicU64,
	length: u64,
	failed: AtomicBool,
}

pub struct Decoder {
	pipeline: gst::Pipeline,
	video: gst_app::AppSink,
	audio: Option<gst_app::AppSink>,
	shared: Arc<Shared>,
	info: Info,
	video_done: bool,
	audio_done: bool,
	/// Presentation time of the last picture handed out; decoders may repeat a frame at EOS.
	last_video_pts: f64,
}

impl Drop for Decoder {
	fn drop(&mut self) {
		let _ = self.pipeline.set_state(gst::State::Null);
	}
}

impl Decoder {
	pub fn open(mut source: Box<dyn ReadSeek>) -> Result<Self, &'static str> {
		let length = source.seek(SeekFrom::End(0)).map_err(|_| INVALID)?;
		source.seek(SeekFrom::Start(0)).map_err(|_| INVALID)?;
		let mut header = [0_u8; 8];
		source.read_exact(&mut header).map_err(|_| INVALID)?;
		// Same container gate as the other platforms: only MPEG-4/QuickTime attachments.
		if length < 8
			|| !matches!(
				&header[4..],
				b"ftyp" | b"moov" | b"mdat" | b"free" | b"skip" | b"wide"
			) {
			return Err(UNSUPPORTED);
		}
		gst::init().map_err(|_| UNSUPPORTED)?;
		let shared = Arc::new(Shared {
			source: Mutex::new(source),
			position: AtomicU64::new(0),
			length,
			failed: AtomicBool::new(false),
		});
		let pipeline = gst::Pipeline::new();
		let guard = PipelineGuard::new(&pipeline);
		let appsrc = gst_app::AppSrc::builder().build();
		appsrc.set_stream_type(gst_app::AppStreamType::RandomAccess);
		appsrc.set_format(gst::Format::Bytes);
		appsrc.set_size(i64::try_from(length).map_err(|_| INVALID)?);
		appsrc.set_max_bytes((4 * MAX_CHUNK) as u64);
		appsrc.set_callbacks(
			gst_app::AppSrcCallbacks::builder()
				.need_data({
					let shared = shared.clone();
					move |src, hint| feed(src, &shared, hint)
				})
				.seek_data({
					let shared = shared.clone();
					move |_, offset| {
						if offset > shared.length {
							return false;
						}
						shared.position.store(offset, Ordering::Release);
						true
					}
				})
				.build(),
		);
		let decodebin = make("decodebin")?;
		pipeline
			.add_many([appsrc.upcast_ref::<gst::Element>(), &decodebin])
			.map_err(|_| UNSUPPORTED)?;
		appsrc.link(&decodebin).map_err(|_| UNSUPPORTED)?;

		let video = gst_app::AppSink::builder().build();
		video.set_caps(Some(
			&gst::Caps::builder("video/x-raw")
				.field("format", "RGBA")
				.build(),
		));
		video.set_sync(false);
		video.set_max_buffers(4);
		video.set_drop(false);
		let audio = gst_app::AppSink::builder().build();
		audio.set_caps(Some(
			&gst::Caps::builder("audio/x-raw")
				.field("format", "F32LE")
				.field("layout", "interleaved")
				.field("channels", AUDIO_CHANNELS as i32)
				.field("rate", AUDIO_RATE as i32)
				.build(),
		));
		audio.set_sync(false);
		audio.set_max_buffers(32);
		audio.set_drop(false);

		let has_video = Arc::new(AtomicBool::new(false));
		let has_audio = Arc::new(AtomicBool::new(false));
		decodebin.connect_pad_added({
			let pipeline = pipeline.downgrade();
			let (video, audio) = (video.clone(), audio.clone());
			let (has_video, has_audio) = (has_video.clone(), has_audio.clone());
			move |_, pad| {
				let Some(pipeline) = pipeline.upgrade() else {
					return;
				};
				let Some(caps) = pad.current_caps() else {
					return;
				};
				let Some(name) = caps.structure(0).map(|s| s.name().to_string()) else {
					return;
				};
				let result = if name.starts_with("video/x-raw") {
					if has_video.swap(true, Ordering::AcqRel) {
						return;
					}
					link_branch(
						&pipeline,
						pad,
						&["queue", "videoflip", "videoconvert"],
						&[("videoflip", "video-direction", "auto")],
						video.upcast_ref(),
					)
				} else if name.starts_with("audio/x-raw") {
					if has_audio.swap(true, Ordering::AcqRel) {
						return;
					}
					link_branch(
						&pipeline,
						pad,
						&["queue", "audioconvert", "audioresample"],
						&[],
						audio.upcast_ref(),
					)
				} else {
					return;
				};
				if result.is_err() {
					let _ = pipeline.post_message(gst::message::Error::new(
						gst::CoreError::Negotiation,
						"Unlinkable stream",
					));
				}
			}
		});
		pipeline
			.set_state(gst::State::Paused)
			.map_err(|_| UNSUPPORTED)?;
		wait_ready(&pipeline, &shared)?;
		if !has_video.load(Ordering::Acquire) {
			return Err(UNSUPPORTED);
		}
		let preroll = video.pull_preroll().map_err(|_| INVALID)?;
		let (width, height) = frame_dimensions(&preroll)?;
		let duration = pipeline
			.query_duration::<gst::ClockTime>()
			.map(|d| d.nseconds() as f64 / 1e9)
			.ok_or(INVALID)?;
		if !duration.is_finite() || duration <= 0.0 {
			return Err(INVALID);
		}
		if duration > MAX_SECONDS {
			return Err(TOO_LONG);
		}
		let has_audio = has_audio.load(Ordering::Acquire);
		pipeline
			.set_state(gst::State::Playing)
			.map_err(|_| UNSUPPORTED)?;
		let decoder = Self {
			pipeline,
			video,
			audio: has_audio.then_some(audio),
			shared,
			info: Info {
				width,
				height,
				duration,
				sample_rate: if has_audio { AUDIO_RATE } else { 0 },
				channels: if has_audio { AUDIO_CHANNELS as u16 } else { 0 },
			},
			video_done: false,
			audio_done: !has_audio,
			last_video_pts: f64::NEG_INFINITY,
		};
		guard.release();
		Ok(decoder)
	}

	pub fn info(&self) -> Info {
		self.info
	}

	pub fn seek(&mut self, seconds: f64) -> Result<(), &'static str> {
		if !seconds.is_finite() || seconds < 0.0 || seconds > self.info.duration {
			return Err(INVALID);
		}
		let position = gst::ClockTime::from_nseconds((seconds * 1e9) as u64);
		// Stale end-of-stream or completion notices from the previous run must not be
		// mistaken for the outcome of this seek.
		if let Some(bus) = self.pipeline.bus() {
			while bus
				.pop_filtered(&[gst::MessageType::AsyncDone, gst::MessageType::Eos])
				.is_some()
			{}
		}
		self.pipeline
			.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, position)
			.map_err(|_| SEEK_FAILED)?;
		wait_ready(&self.pipeline, &self.shared).map_err(|_| SEEK_FAILED)?;
		self.video_done = false;
		self.audio_done = self.audio.is_none();
		self.last_video_pts = f64::NEG_INFINITY;
		Ok(())
	}

	pub fn poll_video(&mut self) -> Result<Poll<Option<Sample>>, &'static str> {
		if self.video_done {
			return Ok(Poll::Ready(None));
		}
		let sample = match self.pull(&self.video)? {
			Poll::Pending => return Ok(Poll::Pending),
			Poll::Ready(None) => {
				self.video_done = true;
				return Ok(Poll::Ready(None));
			}
			Poll::Ready(Some(sample)) => sample,
		};
		let pts = stream_time(&sample)?;
		if pts <= self.last_video_pts {
			return Ok(Poll::Pending);
		}
		self.last_video_pts = pts;
		let (width, height) = frame_dimensions(&sample)?;
		let buffer = sample.buffer().ok_or(INVALID)?;
		let caps = sample.caps().ok_or(INVALID)?;
		let info = gst_video::VideoInfo::from_caps(caps).map_err(|_| INVALID)?;
		let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
			.map_err(|_| INVALID)?;
		let stride = usize::try_from(frame.plane_stride().first().copied().ok_or(INVALID)?)
			.map_err(|_| INVALID)?;
		let data = frame.plane_data(0).map_err(|_| INVALID)?;
		let (w, h) = (width as usize, height as usize);
		let row = w * 4;
		if stride < row || data.len() < stride * (h - 1) + row || row * h > MAX_BYTES {
			return Err(INVALID);
		}
		let mut rgba = vec![0; row * h];
		for (source, target) in data.chunks(stride).zip(rgba.chunks_exact_mut(row)) {
			target.copy_from_slice(&source[..row]);
		}
		Ok(Poll::Ready(Some(Sample::Video {
			pts: self.last_video_pts,
			width,
			height,
			rgba,
		})))
	}

	pub fn poll_audio(&mut self) -> Result<Poll<Option<Sample>>, &'static str> {
		let Some(audio) = &self.audio else {
			return Ok(Poll::Ready(None));
		};
		if self.audio_done {
			return Ok(Poll::Ready(None));
		}
		let sample = match self.pull(audio)? {
			Poll::Pending => return Ok(Poll::Pending),
			Poll::Ready(None) => {
				self.audio_done = true;
				return Ok(Poll::Ready(None));
			}
			Poll::Ready(Some(sample)) => sample,
		};
		let pts = stream_time(&sample)?;
		let buffer = sample.buffer().ok_or(INVALID)?;
		let map = buffer.map_readable().map_err(|_| INVALID)?;
		let bytes = map.as_slice();
		let frame_bytes = AUDIO_CHANNELS * 4;
		if bytes.len() % frame_bytes != 0 || bytes.len() / frame_bytes > AUDIO_RATE as usize {
			return Err(INVALID);
		}
		let frames = bytes
			.chunks_exact(frame_bytes)
			.map(|frame| {
				[
					f32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]),
					f32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]),
				]
				.map(|x| {
					if x.is_finite() {
						x.clamp(-1.0, 1.0)
					} else {
						0.0
					}
				})
			})
			.collect();
		Ok(Poll::Ready(Some(Sample::Audio { pts, frames })))
	}

	/// A full sibling queue can stop demuxing this track. Return control to the worker so
	/// it can drain the other track, process cancellation, and apply its stall deadline.
	fn pull(&self, sink: &gst_app::AppSink) -> Result<Poll<Option<gst::Sample>>, &'static str> {
		if self.shared.failed.load(Ordering::Acquire) {
			return Err(INVALID);
		}
		check_bus(&self.pipeline)?;
		if let Some(sample) = sink.try_pull_sample(gst::ClockTime::ZERO) {
			return Ok(Poll::Ready(Some(sample)));
		}
		if sink.is_eos() {
			return Ok(Poll::Ready(None));
		}
		Ok(Poll::Pending)
	}
}

fn make(name: &str) -> Result<gst::Element, &'static str> {
	gst::ElementFactory::make(name)
		.build()
		.map_err(|_| UNSUPPORTED)
}

/// Create, add and link a branch behind a freshly exposed decodebin pad.
fn link_branch(
	pipeline: &gst::Pipeline,
	pad: &gst::Pad,
	factories: &[&str],
	properties: &[(&str, &str, &str)],
	sink: &gst::Element,
) -> Result<(), &'static str> {
	let mut elements = Vec::with_capacity(factories.len() + 1);
	for factory in factories {
		let element = make(factory)?;
		for (target, name, value) in properties {
			if target == factory {
				element.set_property_from_str(name, value);
			}
		}
		elements.push(element);
	}
	elements.push(sink.clone());
	pipeline
		.add_many(elements.iter())
		.map_err(|_| UNSUPPORTED)?;
	gst::Element::link_many(elements.iter()).map_err(|_| UNSUPPORTED)?;
	let first = elements.first().ok_or(INVALID)?;
	let sinkpad = first.static_pad("sink").ok_or(INVALID)?;
	pad.link(&sinkpad).map_err(|_| UNSUPPORTED)?;
	for element in &elements {
		element.sync_state_with_parent().map_err(|_| UNSUPPORTED)?;
	}
	Ok(())
}

/// Serve one bounded chunk at the current offset, or end the stream at EOF or on a failure.
fn feed(src: &gst_app::AppSrc, shared: &Shared, hint: u32) {
	let position = shared.position.load(Ordering::Acquire);
	if position >= shared.length || shared.failed.load(Ordering::Acquire) {
		let _ = src.end_of_stream();
		return;
	}
	// Pull-mode demuxers reject buffers larger than their request; serve exactly that much.
	let want = if hint == u32::MAX {
		CHUNK
	} else {
		(hint as usize).clamp(1, MAX_CHUNK)
	};
	let count = want.min((shared.length - position) as usize);
	let mut bytes = vec![0; count];
	let read = shared.source.lock().map_err(|_| ()).and_then(|mut source| {
		source
			.seek(SeekFrom::Start(position))
			.and_then(|_| source.read_exact(&mut bytes))
			.map_err(|_| ())
	});
	if read.is_err() {
		shared.failed.store(true, Ordering::Release);
		let _ = src.end_of_stream();
		return;
	}
	let mut buffer = gst::Buffer::from_mut_slice(bytes);
	if let Some(buffer) = buffer.get_mut() {
		buffer.set_offset(position);
		buffer.set_offset_end(position + count as u64);
	}
	shared
		.position
		.store(position + count as u64, Ordering::Release);
	let _ = src.push_buffer(buffer);
}

/// Block until the pipeline finished its pending (pre-roll or seek) state change.
fn wait_ready(pipeline: &gst::Pipeline, shared: &Shared) -> Result<(), &'static str> {
	let bus = pipeline.bus().ok_or(INVALID)?;
	let started = std::time::Instant::now();
	loop {
		if shared.failed.load(Ordering::Acquire) {
			return Err(INVALID);
		}
		let Some(message) = bus.timed_pop_filtered(
			gst::ClockTime::from_mseconds(250),
			&[
				gst::MessageType::AsyncDone,
				gst::MessageType::Error,
				gst::MessageType::Eos,
			],
		) else {
			if started.elapsed().as_secs() > WAIT.seconds() {
				return Err(INVALID);
			}
			continue;
		};
		match message.view() {
			gst::MessageView::AsyncDone(_) => return Ok(()),
			gst::MessageView::Error(_) => return Err(UNSUPPORTED),
			gst::MessageView::Eos(_) => return Err(INVALID),
			_ => {}
		}
	}
}

fn check_bus(pipeline: &gst::Pipeline) -> Result<(), &'static str> {
	let bus = pipeline.bus().ok_or(INVALID)?;
	while let Some(message) = bus.pop_filtered(&[gst::MessageType::Error]) {
		if let gst::MessageView::Error(_) = message.view() {
			return Err(INVALID);
		}
	}
	Ok(())
}

fn frame_dimensions(sample: &gst::Sample) -> Result<(u32, u32), &'static str> {
	let caps = sample.caps().ok_or(INVALID)?;
	let info = gst_video::VideoInfo::from_caps(caps).map_err(|_| INVALID)?;
	if info.format() != gst_video::VideoFormat::Rgba {
		return Err(INVALID);
	}
	super::check_dimensions(info.width(), info.height())?;
	Ok((info.width(), info.height()))
}

/// Buffer timestamps are segment-relative; convert them to positions in the attachment.
fn stream_time(sample: &gst::Sample) -> Result<f64, &'static str> {
	let buffer = sample.buffer().ok_or(INVALID)?;
	let pts = buffer.pts().ok_or(INVALID)?;
	let stream = sample
		.segment()
		.and_then(|segment| segment.downcast_ref::<gst::ClockTime>())
		.and_then(|segment| segment.to_stream_time(pts))
		.unwrap_or(pts);
	let seconds = stream.nseconds() as f64 / 1e9;
	if !(0.0..=MAX_SECONDS + 1.0).contains(&seconds) {
		return Err(INVALID);
	}
	Ok(seconds)
}

#[cfg(test)]
mod tests {
	use super::*;
	const FIXTURE: &[u8] = include_bytes!("../../../../apps/desktop/tests/fixtures/video.mov");

	struct OwnedSource {
		bytes: std::io::Cursor<&'static [u8]>,
		dropped: Arc<AtomicBool>,
		read: usize,
		fail_after_header: bool,
	}

	impl std::io::Read for OwnedSource {
		fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
			if self.fail_after_header && self.read >= 8 {
				return Err(std::io::Error::other("synthetic source failure"));
			}
			let count = std::io::Read::read(&mut self.bytes, buffer)?;
			self.read += count;
			Ok(count)
		}
	}

	impl std::io::Seek for OwnedSource {
		fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
			std::io::Seek::seek(&mut self.bytes, position)
		}
	}

	impl Drop for OwnedSource {
		fn drop(&mut self) {
			self.dropped.store(true, Ordering::Release);
		}
	}

	fn owned_source(dropped: &Arc<AtomicBool>, fail_after_header: bool) -> Box<dyn ReadSeek> {
		Box::new(OwnedSource {
			bytes: std::io::Cursor::new(FIXTURE),
			dropped: dropped.clone(),
			read: 0,
			fail_after_header,
		})
	}

	#[test]
	fn attachment_pipeline_and_source_retire_on_drop() {
		let dropped = Arc::new(AtomicBool::new(false));
		let decoder = Decoder::open(owned_source(&dropped, false)).unwrap();
		let pipeline = decoder.pipeline.downgrade();
		drop(decoder);
		assert_eq!(
			(
				pipeline.upgrade().is_none(),
				dropped.load(Ordering::Acquire)
			),
			(true, true),
			"dropping playback must release its native pipeline and anonymous source"
		);
	}

	#[test]
	fn attachment_source_retires_after_constructor_failure() {
		let dropped = Arc::new(AtomicBool::new(false));
		// The initial container header passes. The first native appsrc read then fails
		// after Paused has started, exercising teardown before Decoder owns the pipeline.
		assert!(Decoder::open(owned_source(&dropped, true)).is_err());
		assert!(
			dropped.load(Ordering::Acquire),
			"a failed native open must release the anonymous source before returning"
		);
	}

	fn wait_for_sample(
		decoder: &mut Decoder,
		poll: fn(&mut Decoder) -> Result<Poll<Option<Sample>>, &'static str>,
	) -> Option<Sample> {
		let started = std::time::Instant::now();
		loop {
			if let Poll::Ready(sample) = poll(decoder).unwrap() {
				return sample;
			}
			assert!(started.elapsed().as_secs() < 20, "sample timed out");
			std::thread::sleep(std::time::Duration::from_millis(1));
		}
	}

	#[test]
	fn native_mov_decodes_audio_video_and_seeks() {
		let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(FIXTURE))).unwrap();
		let info = decoder.info();
		assert_eq!(
			(info.width, info.height, info.sample_rate, info.channels),
			(320, 180, 48_000, 2)
		);
		assert!((2.9..3.1).contains(&info.duration));
		let (mut videos, mut audio_frames, mut last_pts) = (0, 0, -1.0);
		let started = std::time::Instant::now();
		while !decoder.video_done || !decoder.audio_done {
			assert!(started.elapsed().as_secs() < 20, "asymmetric drain stalled");
			let mut samples = Vec::new();
			// Fill audio eagerly, as the player does. Blocking here deadlocks when the
			// bounded video branch fills, or at video EOF before audio has drained.
			for _ in 0..16 {
				match decoder.poll_audio().unwrap() {
					Poll::Ready(Some(sample)) => samples.push(sample),
					Poll::Ready(None) | Poll::Pending => break,
				}
			}
			if let Poll::Ready(Some(sample)) = decoder.poll_video().unwrap() {
				samples.push(sample);
			}
			if samples.is_empty() {
				std::thread::sleep(std::time::Duration::from_millis(1));
			}
			for sample in samples {
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
		// GStreamer trims the last B-frames of this edit-listed fixture; all output stays ordered.
		assert!((66..=72).contains(&videos), "{videos}");
		assert!(audio_frames >= 140_000, "{audio_frames}");
		assert!(wait_for_sample(&mut decoder, Decoder::poll_video).is_none());
		assert!(wait_for_sample(&mut decoder, Decoder::poll_audio).is_none());
		decoder.seek(1.0).unwrap();
		let Some(Sample::Video { pts, .. }) = wait_for_sample(&mut decoder, Decoder::poll_video)
		else {
			panic!("no frame after seek");
		};
		assert!((0.9..1.2).contains(&pts), "{pts}");
		assert!(decoder.seek(f64::NAN).is_err());
		let silent = include_bytes!("../../../../apps/desktop/tests/fixtures/video-silent.mov");
		let mut decoder = Decoder::open(Box::new(std::io::Cursor::new(silent.as_slice()))).unwrap();
		assert_eq!(decoder.info().sample_rate, 0);
		assert!(wait_for_sample(&mut decoder, Decoder::poll_audio).is_none());
		assert!(wait_for_sample(&mut decoder, Decoder::poll_video).is_some());
		assert!(Decoder::open(Box::new(std::io::Cursor::new(vec![0_u8; 64]))).is_err());
	}
}
