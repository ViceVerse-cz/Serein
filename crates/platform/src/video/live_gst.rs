//! Linux live H.264 decoding through GStreamer. `decodebin` auto-plugs the highest-ranked
//! decoder, which is the VA-API or NVDEC plugin when one is installed and `avdec_h264`
//! otherwise. The caller hands over complete, already decrypted Annex-B access units.
use super::{BUSY, INVALID, MAX_BYTES, UNSUPPORTED};
pub use super::{LiveFrame as Frame, LiveSink as Sink, MAX_ACCESS_UNIT};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video::{self as gst_video, VideoFrameExt};
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU64, Ordering},
};

const MAX_QUEUED_UNITS: u64 = 4;
const MAX_QUEUED_BYTES: u64 = MAX_QUEUED_UNITS * MAX_ACCESS_UNIT as u64;

/// Decoder for one sender; pictures arrive through the sink from GStreamer's thread.
pub struct H264Decoder {
	pipeline: gst::Pipeline,
	appsrc: gst_app::AppSrc,
	failed: Arc<AtomicBool>,
	delivered: Arc<AtomicU64>,
	pictures: u64,
}

impl Drop for H264Decoder {
	fn drop(&mut self) {
		let _ = self.pipeline.set_state(gst::State::Null);
	}
}

impl H264Decoder {
	pub fn new(sink: Sink) -> Result<Self, &'static str> {
		gst::init().map_err(|_| UNSUPPORTED)?;
		let pipeline = gst::Pipeline::new();
		let appsrc = gst_app::AppSrc::builder()
			.caps(
				&gst::Caps::builder("video/x-h264")
					.field("stream-format", "byte-stream")
					.field("alignment", "au")
					.build(),
			)
			.build();
		appsrc.set_stream_type(gst_app::AppStreamType::Stream);
		appsrc.set_format(gst::Format::Time);
		appsrc.set_is_live(true);
		appsrc.set_do_timestamp(true);
		// Item accounting needs GStreamer 1.20; older installations use the caller's
		// bounded software fallback rather than accepting an unbounded native queue.
		if appsrc.find_property("current-level-buffers").is_none() {
			return Err(UNSUPPORTED);
		}
		appsrc.set_max_bytes(MAX_QUEUED_BYTES);
		appsrc.set_block(false);
		let parse = make("h264parse")?;
		let decodebin = make("decodebin")?;
		pipeline
			.add_many([appsrc.upcast_ref::<gst::Element>(), &parse, &decodebin])
			.map_err(|_| UNSUPPORTED)?;
		gst::Element::link_many([appsrc.upcast_ref::<gst::Element>(), &parse, &decodebin])
			.map_err(|_| UNSUPPORTED)?;

		let failed = Arc::new(AtomicBool::new(false));
		let delivered = Arc::new(AtomicU64::new(0));
		let appsink = gst_app::AppSink::builder()
			.caps(
				&gst::Caps::builder("video/x-raw")
					.field("format", "RGBA")
					.build(),
			)
			.build();
		appsink.set_sync(false);
		appsink.set_max_buffers(2);
		appsink.set_drop(true);
		appsink.set_callbacks(
			gst_app::AppSinkCallbacks::builder()
				.new_sample({
					let failed = failed.clone();
					let delivered = delivered.clone();
					move |appsink| {
						let Ok(sample) = appsink.pull_sample() else {
							return Err(gst::FlowError::Eos);
						};
						match picture(&sample) {
							Ok(frame) => {
								sink(frame);
								delivered.fetch_add(1, Ordering::Release);
								Ok(gst::FlowSuccess::Ok)
							}
							Err(_) => {
								failed.store(true, Ordering::Release);
								Err(gst::FlowError::Error)
							}
						}
					}
				})
				.build(),
		);
		decodebin.connect_pad_added({
			let pipeline = pipeline.clone();
			let appsink = appsink.clone();
			let failed = failed.clone();
			move |_, pad| {
				let is_video = pad
					.current_caps()
					.and_then(|caps| caps.structure(0).map(|s| s.name().starts_with("video/")))
					.unwrap_or(false);
				if !is_video {
					return;
				}
				let Ok(convert) = make("videoconvert") else {
					failed.store(true, Ordering::Release);
					return;
				};
				let sink = appsink.upcast_ref::<gst::Element>().clone();
				if pipeline.add_many([&convert, &sink]).is_err()
					|| convert.link(&sink).is_err()
					|| convert
						.static_pad("sink")
						.is_none_or(|target| pad.link(&target).is_err())
					|| convert.sync_state_with_parent().is_err()
					|| sink.sync_state_with_parent().is_err()
				{
					failed.store(true, Ordering::Release);
				}
			}
		});
		pipeline
			.set_state(gst::State::Playing)
			.map_err(|_| UNSUPPORTED)?;
		Ok(Self {
			pipeline,
			appsrc,
			failed,
			delivered,
			pictures: 0,
		})
	}

	/// Queue one Annex-B access unit. Errors from the pipeline surface on the next call.
	pub fn decode(&mut self, access_unit: &[u8]) -> Result<(), &'static str> {
		if access_unit.len() > MAX_ACCESS_UNIT {
			return Err(INVALID);
		}
		// max-bytes only emits enough-data when block=false; it does not reject pushes.
		// This decoder has one producer and native consumption can only lower these levels.
		if self.appsrc.property::<u64>("current-level-buffers") >= MAX_QUEUED_UNITS
			|| self
				.appsrc
				.current_level_bytes()
				.saturating_add(access_unit.len() as u64)
				> MAX_QUEUED_BYTES
		{
			return Err(BUSY);
		}
		if self.failed.load(Ordering::Acquire) {
			return Err(INVALID);
		}
		let bus = self.pipeline.bus().ok_or(INVALID)?;
		while let Some(message) = bus.pop_filtered(&[gst::MessageType::Error]) {
			if let gst::MessageView::Error(_) = message.view() {
				return Err(UNSUPPORTED);
			}
		}
		self.pictures += 1;
		let mut buffer = gst::Buffer::from_slice(access_unit.to_vec());
		buffer.get_mut().ok_or(INVALID)?.set_offset(self.pictures);
		// Rejections invalidate the reference chain; the caller requests a keyframe.
		match self.appsrc.push_buffer(buffer) {
			Ok(_) => Ok(()),
			Err(gst::FlowError::Flushing) => Err(INVALID),
			Err(_) => Err(UNSUPPORTED),
		}
	}

	/// Wait, bounded, until at least one queued picture has reached the sink (tests only;
	/// live use never waits). Decoders may legitimately hold the final pictures back.
	pub fn flush(&self) {
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
		while self.delivered.load(Ordering::Acquire) == 0
			&& self.pictures > 0
			&& !self.failed.load(Ordering::Acquire)
			&& std::time::Instant::now() < deadline
		{
			std::thread::sleep(std::time::Duration::from_millis(10));
		}
	}
}

fn make(name: &str) -> Result<gst::Element, &'static str> {
	gst::ElementFactory::make(name)
		.build()
		.map_err(|_| UNSUPPORTED)
}

fn picture(sample: &gst::Sample) -> Result<Frame, &'static str> {
	let caps = sample.caps().ok_or(INVALID)?;
	let info = gst_video::VideoInfo::from_caps(caps).map_err(|_| INVALID)?;
	if info.format() != gst_video::VideoFormat::Rgba {
		return Err(INVALID);
	}
	super::check_dimensions(info.width(), info.height())?;
	let buffer = sample.buffer().ok_or(INVALID)?;
	let frame =
		gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info).map_err(|_| INVALID)?;
	let stride = usize::try_from(frame.plane_stride().first().copied().ok_or(INVALID)?)
		.map_err(|_| INVALID)?;
	let data = frame.plane_data(0).map_err(|_| INVALID)?;
	let (w, h) = (info.width() as usize, info.height() as usize);
	let row = w * 4;
	if stride < row || data.len() < stride * (h - 1) + row || row * h > MAX_BYTES {
		return Err(INVALID);
	}
	let mut rgba = vec![0; row * h];
	for (source, target) in data.chunks(stride).zip(rgba.chunks_exact_mut(row)) {
		target.copy_from_slice(&source[..row]);
	}
	Ok(Frame {
		width: info.width(),
		height: info.height(),
		rgba,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn stalled_native_queue_rejects_pressure_at_item_and_byte_limits() {
		gst::init().expect("GStreamer runtime");
		for length in [16, MAX_ACCESS_UNIT] {
			let pipeline = gst::Pipeline::new();
			let appsrc = gst_app::AppSrc::builder().is_live(true).build();
			if appsrc.find_property("current-level-buffers").is_none() {
				assert_eq!(H264Decoder::new(Box::new(|_| {})).err(), Some(UNSUPPORTED));
				return;
			}
			appsrc.set_max_bytes(MAX_QUEUED_BYTES);
			pipeline.add(&appsrc).unwrap();
			let mut decoder = H264Decoder {
				pipeline,
				appsrc,
				failed: Arc::default(),
				delivered: Arc::default(),
				pictures: 0,
			};
			// A live appsrc consumes only while Playing. Null would flush it and reject
			// pushes for a lifecycle reason instead of exercising the actual queue limit.
			decoder.pipeline.set_state(gst::State::Paused).unwrap();
			// A bounded Annex-B filler NAL; no capture or hardware input is opened.
			let mut unit = vec![0xff; length];
			unit[..5].copy_from_slice(&[0, 0, 0, 1, 0x0c]);
			unit[length - 1] = 0x80;
			for _ in 0..MAX_QUEUED_UNITS {
				decoder.decode(&unit).unwrap();
			}
			for _ in 0..32 {
				assert_eq!(decoder.decode(&unit), Err(INVALID));
			}
			assert_eq!(decoder.pictures, MAX_QUEUED_UNITS);
			assert_eq!(
				decoder.appsrc.property::<u64>("current-level-buffers"),
				MAX_QUEUED_UNITS
			);
			assert_eq!(
				decoder.appsrc.current_level_bytes(),
				MAX_QUEUED_UNITS * length as u64
			);
		}
	}

	#[test]
	fn rejects_oversized_access_units() {
		let mut decoder = match H264Decoder::new(Box::new(|_| {})) {
			Ok(decoder) => decoder,
			Err(_) => return, // No GStreamer H.264 plugins on this machine.
		};
		assert!(decoder.decode(&vec![0; MAX_ACCESS_UNIT + 1]).is_err());
	}
}
