//! Bounded capture/scale/encode pipeline; also exercised with an offline video source.
use super::{Codec, MAX_ENCODED_BYTES, MAX_RAW_BYTES, RawFrame, Settings};
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
	VaAv1,
	NvidiaAv1,
	NvidiaAv1Copy,
	Va,
	VaLegacy,
	Nvidia,
	NvidiaCopy,
	Software,
}
impl Mode {
	const H264: [Self; 5] = [
		Self::Va,
		Self::VaLegacy,
		Self::Nvidia,
		Self::NvidiaCopy,
		Self::Software,
	];
	const AV1: [Self; 3] = [Self::VaAv1, Self::NvidiaAv1, Self::NvidiaAv1Copy];
	pub(super) fn candidates(acceleration: bool, codec: Codec) -> &'static [Self] {
		match (acceleration, codec) {
			(true, Codec::Av1) => &Self::AV1,
			(true, Codec::H264) => &Self::H264,
			(false, Codec::H264) => &[Self::Software],
			(false, Codec::Av1) => &[],
		}
	}
	pub(super) fn codec(self) -> Codec {
		match self {
			Self::VaAv1 | Self::NvidiaAv1 | Self::NvidiaAv1Copy => Codec::Av1,
			_ => Codec::H264,
		}
	}
	fn factory(self) -> &'static str {
		match self {
			Self::VaAv1 => "vaav1enc",
			Self::NvidiaAv1 | Self::NvidiaAv1Copy => "nvav1enc",
			Self::Va => "vah264enc",
			Self::VaLegacy => "vaapih264enc",
			Self::Nvidia | Self::NvidiaCopy => "nvh264enc",
			Self::Software => "videoconvertscale",
		}
	}
	pub(super) fn label(self) -> &'static str {
		match self {
			Self::VaAv1 => "AV1 · VA-API hardware encoding",
			Self::NvidiaAv1 => "AV1 · NVENC hardware encoding",
			Self::NvidiaAv1Copy => "AV1 · NVENC hardware encoding · CPU scaling",
			Self::Va => "H.264 · VA-API hardware encoding",
			Self::VaLegacy => "H.264 · VA-API hardware encoding · CPU scaling",
			Self::Nvidia => "H.264 · NVENC hardware encoding",
			Self::NvidiaCopy => "H.264 · NVENC hardware encoding · CPU scaling",
			Self::Software => "H.264 · software encoding (higher CPU use)",
		}
	}
	/// Whether the scaler applies PipeWire's crop rectangle: vapostproc always does,
	/// videoconvertscale only after GStreamer 1.29.2, and the GL mixer never does.
	fn applies_crop(self) -> bool {
		match self {
			Self::Va | Self::VaAv1 => true,
			Self::Nvidia | Self::NvidiaAv1 => false,
			Self::VaLegacy | Self::NvidiaCopy | Self::NvidiaAv1Copy | Self::Software => {
				gst::version() >= (1, 29, 3, 0)
			}
		}
	}
}

/// Test installed AV1 encoders with generated pixels, never screen or microphone access.
/// Merely finding a plugin does not establish that its driver can encode AV1.
pub(super) async fn probe_av1(settings: Settings, stop: &Arc<AtomicBool>) -> Option<Mode> {
	for &mode in Mode::candidates(true, Codec::Av1) {
		if stop.load(Ordering::Acquire) {
			return None;
		}
		let Ok(source) = gst::ElementFactory::make("videotestsrc")
			.property("is-live", true)
			.build()
		else {
			return None;
		};
		let Ok(pipeline) = Capture::new(
			settings,
			mode,
			settings.bit_rate(),
			source,
			stop.clone(),
			Arc::new(AtomicBool::new(true)),
			Arc::new(AtomicBool::new(true)),
			|| true,
		) else {
			continue;
		};
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
		while std::time::Instant::now() < deadline
			&& !stop.load(Ordering::Acquire)
			&& !pipeline.failed()
		{
			// Both bounded sinks must drain even though only encoded video is inspected.
			let _ = pipeline.preview.try_pull_sample(gst::ClockTime::ZERO);
			if let Some(sample) = pipeline.frames.try_pull_sample(gst::ClockTime::ZERO)
				&& let Some(buffer) = sample.buffer()
				&& !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT)
				&& buffer.size() <= MAX_ENCODED_BYTES
				&& let Ok(map) = buffer.map_readable()
				&& crate::video_av1::validate_source(&map).is_ok()
			{
				return Some(mode);
			}
			pipeline.changed().await;
		}
	}
	None
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
		bitrate: u32,
		source: gst::Element,
		stop: Arc<AtomicBool>,
		ready: Arc<AtomicBool>,
		keyframe: Arc<AtomicBool>,
		has_capacity: impl Fn() -> bool + Send + Sync + 'static,
	) -> Result<Self, &'static str> {
		if !settings.valid() {
			return Err(INVALID);
		}
		if gst::ElementFactory::find(mode.factory()).is_none() {
			return Err(UNAVAILABLE);
		}
		let bitrate = bitrate.clamp(250_000, settings.bit_rate());
		let size = format!(
			"width={},height={},pixel-aspect-ratio=1/1",
			settings.width, settings.height
		);
		let (scale, preview_scale) = match mode {
			Mode::Va | Mode::VaAv1 => (
				format!(
					"vapostproc add-borders=true ! video/x-raw(memory:VAMemory),format=NV12,{size}"
				),
				"vapostproc add-borders=true ! video/x-raw,format=BGRA,width=640,height=360",
			),
			Mode::Nvidia | Mode::NvidiaAv1 => (
				format!(
					"glupload ! glcolorconvert ! glvideomixer name=fit background=black sink_0::sizing-policy=keep-aspect-ratio sink_0::width={width} sink_0::height={height} ! video/x-raw(memory:GLMemory),format=RGBA,{size}",
					width = settings.width,
					height = settings.height
				),
				"glcolorscale ! video/x-raw(memory:GLMemory),format=RGBA,width=640,height=360 ! gldownload ! videoconvert ! video/x-raw,format=BGRA",
			),
			// ponytail: CPU scaling avoids mixing VAMemory and legacy VASurface buffers;
			// add legacy GPU postprocessing only if measured scaling cost warrants it.
			Mode::VaLegacy => (
				format!("videoconvertscale add-borders=true ! video/x-raw,format=NV12,{size}"),
				"videoconvertscale add-borders=true ! video/x-raw,format=BGRA,width=640,height=360",
			),
			Mode::NvidiaCopy | Mode::NvidiaAv1Copy | Mode::Software => (
				format!("videoconvertscale add-borders=true ! video/x-raw,format=BGRA,{size}"),
				"videoconvertscale add-borders=true ! video/x-raw,format=BGRA,width=640,height=360",
			),
		};
		let encoder = match mode {
			Mode::VaAv1 => format!(
				"vaav1enc name=encoder rate-control=cbr bitrate={} key-int-max={} hierarchical-level=1 ref-frames=1",
				bitrate / 1000,
				settings.fps * 2
			),
			Mode::NvidiaAv1 | Mode::NvidiaAv1Copy => format!(
				"nvav1enc name=encoder rc-mode=cbr bitrate={} gop-size={} bframes=0 rc-lookahead=0 zerolatency=true",
				bitrate / 1000,
				settings.fps * 2
			),
			Mode::Va => format!(
				"vah264enc name=encoder rate-control=cbr bitrate={} key-int-max={} b-frames=0",
				bitrate / 1000,
				settings.fps * 2
			),
			Mode::VaLegacy => format!(
				"vaapih264enc name=encoder rate-control=cbr bitrate={} keyframe-period={} max-bframes=0 cabac=false dct8x8=false",
				bitrate / 1000,
				settings.fps * 2
			),
			Mode::Nvidia | Mode::NvidiaCopy => format!(
				"nvh264enc name=encoder rc-mode=cbr bitrate={} gop-size={} bframes=0 rc-lookahead=0 zerolatency=true",
				bitrate / 1000,
				settings.fps * 2
			),
			Mode::Software => String::new(),
		};
		let encode = if mode == Mode::Software {
			String::new()
		} else if mode.codec() == Codec::Av1 {
			format!(
				"{encoder} ! av1parse ! video/x-av1,stream-format=obu-stream,alignment=frame,profile=main !"
			)
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
		if mode.applies_crop() {
			match_crop_aspect(&source.static_pad("src").ok_or(UNAVAILABLE)?);
		}
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
	pub(super) fn set_bitrate(&self, bitrate: u32) -> bool {
		let Some(encoder) = self.pipeline.by_name("encoder") else {
			return false;
		};
		if !encoder
			.find_property("bitrate")
			.is_some_and(|property| property.flags().contains(gst::PARAM_FLAG_MUTABLE_PLAYING))
		{
			return false;
		}
		encoder.set_property("bitrate", bitrate / 1000);
		true
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
/// Window casts may keep the stream size and mark the window with a crop rectangle. Scalers
/// that apply it still place borders by the caps' aspect ratio, stretching the window, so
/// declare the (square) source pixels in a shape that gives the caps the crop's aspect ratio.
fn match_crop_aspect(pad: &gst::Pad) {
	pad.add_probe(gst::PadProbeType::BUFFER, |pad, info| {
		let (Some(buffer), Some(caps)) = (info.buffer(), pad.current_caps()) else {
			return gst::PadProbeReturn::Ok;
		};
		let Some((width, height)) = caps.structure(0).and_then(|structure| {
			Some((
				structure.get::<i32>("width").ok()?,
				structure.get::<i32>("height").ok()?,
			))
		}) else {
			return gst::PadProbeReturn::Ok;
		};
		let aspect = buffer
			.meta::<video::VideoCropMeta>()
			.and_then(|crop| {
				let (x, y, crop_width, crop_height) = crop.rect();
				let (crop_width, crop_height) = (
					i32::try_from(crop_width).ok()?,
					i32::try_from(crop_height).ok()?,
				);
				(crop_width > 0
					&& crop_height > 0
					&& i64::from(x) + i64::from(crop_width) <= i64::from(width)
					&& i64::from(y) + i64::from(crop_height) <= i64::from(height))
				.then_some(())?;
				Some(gst::Fraction::new(
					i32::try_from(i64::from(crop_width) * i64::from(height)).ok()?,
					i32::try_from(i64::from(crop_height) * i64::from(width)).ok()?,
				))
			})
			.unwrap_or(gst::Fraction::new(1, 1));
		let current = caps
			.structure(0)
			.and_then(|structure| structure.get::<gst::Fraction>("pixel-aspect-ratio").ok())
			.unwrap_or(gst::Fraction::new(1, 1));
		if current != aspect {
			let mut caps = caps.copy();
			caps.make_mut().set("pixel-aspect-ratio", aspect);
			pad.push_event(gst::event::Caps::new(&caps));
		}
		gst::PadProbeReturn::Ok
	});
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

#[cfg(test)]
mod tests {
	use super::{Codec, Mode};

	#[test]
	fn acceleration_opt_out_excludes_every_hardware_encoder_and_av1() {
		assert!(Mode::candidates(false, Codec::H264) == [Mode::Software]);
		assert!(Mode::candidates(false, Codec::Av1).is_empty());
	}

	#[test]
	fn replacements_preserve_the_negotiated_codec() {
		for codec in [Codec::H264, Codec::Av1] {
			assert!(
				Mode::candidates(true, codec)
					.iter()
					.all(|mode| mode.codec() == codec)
			);
		}
		assert!(
			Mode::candidates(true, Codec::Av1)
				.iter()
				.all(|mode| *mode != Mode::Software)
		);
		assert!(Mode::candidates(true, Codec::H264).last() == Some(&Mode::Software));
	}
}
