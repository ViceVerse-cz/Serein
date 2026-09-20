//! Bounded capture/scale/encode pipeline; also exercised with an offline video source.
use super::{MAX_ENCODED_BYTES, MAX_RAW_BYTES, RawFrame, Settings};
use ::gstreamer as gst;
use gst::prelude::*;
use gstreamer_app as app;
use gstreamer_video::{self as video, VideoFrameExt};
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
};

const INVALID: &str = "Screen capture returned an unsupported frame";
const UNAVAILABLE: &str = "Screen capture or encoder is unavailable";
const MAX_SOURCE_BYTES: usize = 7680 * 4320 * 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
	Va,
	Nvidia,
	NvidiaCopy,
	Software,
}
impl Mode {
	pub(super) const ALL: [Self; 4] = [Self::Va, Self::Nvidia, Self::NvidiaCopy, Self::Software];
	pub(super) fn label(self) -> &'static str {
		match self {
			Self::Va => "H.264 · VA-API hardware encoding",
			Self::Nvidia => "H.264 · NVENC hardware encoding",
			Self::NvidiaCopy => "H.264 · NVENC hardware encoding · CPU scaling",
			Self::Software => "H.264 · software encoding (higher CPU use)",
		}
	}
}

pub(super) struct Capture {
	pipeline: gst::Pipeline,
	pub frames: app::AppSink,
	pub preview: app::AppSink,
	failed: Arc<AtomicBool>,
	changed: Arc<tokio::sync::Notify>,
	preview_gate: gst::Element,
}
impl Drop for Capture {
	fn drop(&mut self) {
		let _ = self.pipeline.set_state(gst::State::Null);
	}
}
impl Capture {
	/// `source` is either the approved PipeWire node or the offline example's test source.
	#[allow(clippy::too_many_arguments)] // One pipeline and its existing media gates.
	pub(super) fn new(
		settings: Settings,
		mode: Mode,
		source: gst::Element,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		keyframe: Arc<AtomicBool>,
		has_capacity: impl Fn() -> bool + Send + Sync + 'static,
	) -> Result<Self, &'static str> {
		if !settings.valid() {
			return Err(INVALID);
		}
		let size = format!(
			"width={},height={},pixel-aspect-ratio=1/1",
			settings.width, settings.height
		);
		let (scale, preview_scale) = match mode {
			Mode::Va => (
				format!(
					"vapostproc add-borders=true ! video/x-raw(memory:VAMemory),format=NV12,{size}"
				),
				"vapostproc add-borders=true ! video/x-raw,format=BGRA,width=640,height=360",
			),
			Mode::Nvidia => (
				format!(
					"glupload ! glcolorconvert ! glvideomixer name=fit background=black sink_0::sizing-policy=keep-aspect-ratio sink_0::width={width} sink_0::height={height} ! video/x-raw(memory:GLMemory),format=RGBA,{size}",
					width = settings.width,
					height = settings.height
				),
				"glcolorscale ! video/x-raw(memory:GLMemory),format=RGBA,width=640,height=360 ! gldownload ! videoconvert ! video/x-raw,format=BGRA",
			),
			Mode::NvidiaCopy | Mode::Software => (
				format!("videoconvertscale add-borders=true ! video/x-raw,format=BGRA,{size}"),
				"videoconvertscale add-borders=true ! video/x-raw,format=BGRA,width=640,height=360",
			),
		};
		let encoder = match mode {
			Mode::Va => format!(
				"vah264enc name=encoder rate-control=cbr bitrate={} key-int-max={} b-frames=0",
				settings.bit_rate() / 1000,
				settings.fps * 2
			),
			Mode::Nvidia | Mode::NvidiaCopy => format!(
				"nvh264enc name=encoder rc-mode=cbr bitrate={} gop-size={} bframes=0 rc-lookahead=0 zerolatency=true",
				settings.bit_rate() / 1000,
				settings.fps * 2
			),
			Mode::Software => String::new(),
		};
		let encode = if mode == Mode::Software {
			String::new()
		} else {
			format!(
				"{encoder} ! h264parse config-interval=-1 ! video/x-h264,stream-format=byte-stream,alignment=au,profile=constrained-baseline !"
			)
		};
		// Raw queues discard stale pictures. The encoded sink blocks upstream instead of
		// discarding reference pictures; pressure then reaches the raw queue.
		let description = format!(
			"capsfilter caps=\"video/x-raw(ANY),width=[1,7680],height=[1,4320]\" ! \
			queue max-size-buffers=1 max-size-bytes={MAX_SOURCE_BYTES} max-size-time=0 leaky=downstream ! \
			videorate drop-only=true ! video/x-raw(ANY),framerate={}/1 ! {scale} ! tee name=split \
			split. ! queue max-size-buffers=1 max-size-bytes={MAX_RAW_BYTES} max-size-time=0 leaky=downstream ! \
			identity name=gate ! {encode} appsink name=frames sync=false async=false max-buffers=1 enable-last-sample=false wait-on-eos=false \
			split. ! queue max-size-buffers=1 max-size-bytes={MAX_RAW_BYTES} max-size-time=0 leaky=downstream ! \
			valve name=preview-gate drop-mode=forward-sticky-events ! videorate drop-only=true ! video/x-raw(ANY),framerate=10/1 ! {preview_scale} ! \
			appsink name=preview sync=false async=false max-buffers=1 drop=true enable-last-sample=false wait-on-eos=false",
			settings.fps
		);
		let bin = gst::parse::bin_from_description(&description, true).map_err(|_| UNAVAILABLE)?;
		let pipeline = gst::Pipeline::new();
		let failed = Arc::new(AtomicBool::new(false));
		let changed = Arc::new(tokio::sync::Notify::new());
		let bus = pipeline.bus().ok_or(UNAVAILABLE)?;
		bus.set_sync_handler({
			let failed = failed.clone();
			let changed = changed.clone();
			move |_, message| {
				if matches!(
					message.view(),
					gst::MessageView::Error(_) | gst::MessageView::Eos(_)
				) {
					failed.store(true, Ordering::Release);
					changed.notify_one();
				}
				// Do not retain bus messages or expose native error text/source names.
				gst::BusSyncReply::Drop
			}
		});
		pipeline
			.add_many([&source, bin.upcast_ref()])
			.map_err(|_| UNAVAILABLE)?;
		source.link(&bin).map_err(|_| UNAVAILABLE)?;
		let frames = sink(&bin, "frames")?;
		let preview = sink(&bin, "preview")?;
		let preview_gate = bin.by_name("preview-gate").ok_or(UNAVAILABLE)?;
		bound(
			&source.static_pad("src").ok_or(UNAVAILABLE)?,
			MAX_SOURCE_BYTES,
			failed.clone(),
		);
		bound(
			&frames.static_pad("sink").ok_or(UNAVAILABLE)?,
			if mode == Mode::Software {
				MAX_RAW_BYTES
			} else {
				MAX_ENCODED_BYTES
			},
			failed.clone(),
		);
		bound(
			&preview.static_pad("sink").ok_or(UNAVAILABLE)?,
			640 * 360 * 4 + 4096,
			failed.clone(),
		);
		for sink in [&frames, &preview] {
			let changed = changed.clone();
			sink.set_callbacks(
				app::AppSinkCallbacks::builder()
					.new_sample(move |_| {
						changed.notify_one();
						Ok(gst::FlowSuccess::Ok)
					})
					.build(),
			);
		}
		bin.by_name("gate")
			.and_then(|gate| gate.static_pad("src"))
			.ok_or(UNAVAILABLE)?
			.add_probe(gst::PadProbeType::BUFFER, move |pad, _| {
				if stop.load(Ordering::Acquire) || !ready.load(Ordering::Acquire) {
					keyframe.store(true, Ordering::Release);
					return gst::PadProbeReturn::Drop;
				}
				if !has_capacity() {
					return gst::PadProbeReturn::Drop;
				}
				if mode != Mode::Software
					&& keyframe.swap(false, Ordering::AcqRel)
					&& !pad.push_event(
						video::DownstreamForceKeyUnitEvent::builder()
							.all_headers(true)
							.build(),
					) {
					keyframe.store(true, Ordering::Release);
				}
				gst::PadProbeReturn::Ok
			});
		let capture = Self {
			pipeline,
			frames,
			preview,
			failed,
			changed,
			preview_gate,
		};
		capture
			.pipeline
			.set_state(gst::State::Playing)
			.map_err(|_| UNAVAILABLE)?;
		Ok(capture)
	}
	pub(super) fn set_preview_visible(&self, visible: bool) {
		self.preview_gate.set_property("drop", !visible);
	}
	pub(super) async fn changed(&self) {
		let _ = tokio::time::timeout(
			std::time::Duration::from_millis(100),
			self.changed.notified(),
		)
		.await;
	}
	pub(super) fn failed(&self) -> bool {
		self.failed.load(Ordering::Acquire)
	}
}
fn sink(bin: &gst::Bin, name: &str) -> Result<app::AppSink, &'static str> {
	bin.by_name(name)
		.and_then(|element| element.downcast().ok())
		.ok_or(UNAVAILABLE)
}
fn bound(pad: &gst::Pad, bytes: usize, failed: Arc<AtomicBool>) {
	pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
		if info.buffer().is_none_or(|buffer| buffer.size() > bytes) {
			failed.store(true, Ordering::Release);
			gst::PadProbeReturn::Drop
		} else {
			gst::PadProbeReturn::Ok
		}
	});
}

pub(super) fn raw(sample: &gst::Sample) -> Result<RawFrame, &'static str> {
	let info = video::VideoInfo::from_caps(sample.caps().ok_or(INVALID)?).map_err(|_| INVALID)?;
	if info.format() != video::VideoFormat::Bgra
		|| info.width() == 0
		|| info.height() == 0
		|| info.width() > 1920
		|| info.height() > 1080
	{
		return Err(INVALID);
	}
	let buffer = sample.buffer().ok_or(INVALID)?;
	if buffer.size() > MAX_RAW_BYTES {
		return Err(INVALID);
	}
	let frame =
		video::VideoFrameRef::from_buffer_ref_readable(buffer, &info).map_err(|_| INVALID)?;
	let stride = usize::try_from(frame.plane_stride()[0]).map_err(|_| INVALID)?;
	let row = info.width() as usize * 4;
	let data = frame.plane_data(0).map_err(|_| INVALID)?;
	let required = stride
		.checked_mul(info.height() as usize - 1)
		.and_then(|bytes| bytes.checked_add(row))
		.ok_or(INVALID)?;
	if stride < row || data.len() < required {
		return Err(INVALID);
	}
	let mut pixels = vec![0; row * info.height() as usize];
	for (source, target) in data.chunks(stride).zip(pixels.chunks_exact_mut(row)) {
		target.copy_from_slice(&source[..row]);
	}
	Ok(RawFrame {
		width: info.width(),
		height: info.height(),
		stride: row,
		data: pixels,
	})
}
