//! Bounded raw capture/scale pipeline; also exercised with an offline video source.
use super::{MAX_RAW_BYTES, RawFrame, Settings};
use ::gstreamer as gst;
use gst::prelude::*;
use gstreamer_app as app;
use gstreamer_video::{self as video, VideoFrameExt};
use model::voice_settings::VideoResolution;
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering},
};

const INVALID: &str = "Screen capture returned an unsupported frame";
const UNAVAILABLE: &str = "Screen capture is unavailable";
const MAX_SOURCE_BYTES: usize = MAX_RAW_BYTES;

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
	pub(super) fn new(
		settings: Settings,
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
		// FFmpeg owns encoding after capture. Queues discard stale raw pictures, and the
		// frame sink blocks upstream when the worker waits for transport capacity.
		let (max_width, max_height) = (VideoResolution::MAX_WIDTH, VideoResolution::MAX_HEIGHT);
		let description = format!(
			"capsfilter caps=\"video/x-raw(ANY),width=[1,{max_width}],height=[1,{max_height}]\" ! \
			queue max-size-buffers=1 max-size-bytes={MAX_SOURCE_BYTES} max-size-time=0 leaky=downstream ! \
			videorate drop-only=true ! video/x-raw(ANY),framerate={}/1 ! \
			videoconvert name=crop-input ! video/x-raw,format=BGRA ! videocrop name=crop ! \
			videoconvertscale add-borders=true ! video/x-raw,format=BGRA,{size} ! tee name=split \
			split. ! queue max-size-buffers=1 max-size-bytes={MAX_RAW_BYTES} max-size-time=0 leaky=downstream ! \
			identity name=gate ! appsink name=frames sync=false async=false max-buffers=1 enable-last-sample=false wait-on-eos=false \
			split. ! queue max-size-buffers=1 max-size-bytes={MAX_RAW_BYTES} max-size-time=0 leaky=downstream ! \
			valve name=preview-gate drop-mode=forward-sticky-events ! videorate drop-only=true ! video/x-raw,framerate=10/1 ! \
			videoconvertscale add-borders=true ! video/x-raw,format=BGRA,width=640,height=360 ! \
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
		let cropper = bin.by_name("crop").ok_or(UNAVAILABLE)?;
		apply_crop(
			&bin.by_name("crop-input").ok_or(UNAVAILABLE)?,
			&cropper,
			failed.clone(),
		)?;
		bound(
			&cropper.static_pad("sink").ok_or(UNAVAILABLE)?,
			MAX_SOURCE_BYTES,
			failed.clone(),
		);
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
			MAX_RAW_BYTES,
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
			.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
				if stop.load(Ordering::Acquire) || !ready.load(Ordering::Acquire) {
					keyframe.store(true, Ordering::Release);
					return gst::PadProbeReturn::Drop;
				}
				if !has_capacity() {
					return gst::PadProbeReturn::Drop;
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
	pub(super) fn request_frame(&self) -> Result<(), &'static str> {
		// READY flushes queued raw samples and restarts the same approved source.
		self.pipeline
			.set_state(gst::State::Ready)
			.map_err(|_| UNAVAILABLE)?;
		self.pipeline
			.set_state(gst::State::Playing)
			.map_err(|_| UNAVAILABLE)?;
		Ok(())
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
/// Portal windows can arrive as a larger stream with a crop rectangle. Apply it before
/// scaling on every supported GStreamer version, rather than exposing pixels outside the
/// selected window when an older CPU scaler ignores VideoCropMeta.
fn apply_crop(
	input: &gst::Element,
	cropper: &gst::Element,
	failed: Arc<AtomicBool>,
) -> Result<(), &'static str> {
	let weak = cropper.downgrade();
	// Strip crop metadata before conversion too: newer converters may consume it,
	// while older ones retain it. Both must reach the same explicit pixel crop.
	input.static_pad("sink").ok_or(UNAVAILABLE)?.add_probe(
		gst::PadProbeType::BUFFER,
		move |pad, info| {
			let result = (|| {
				let caps = pad.current_caps().ok_or(INVALID)?;
				let structure = caps.structure(0).ok_or(INVALID)?;
				let size = (
					u32::try_from(structure.get::<i32>("width").map_err(|_| INVALID)?)
						.map_err(|_| INVALID)?,
					u32::try_from(structure.get::<i32>("height").map_err(|_| INVALID)?)
						.map_err(|_| INVALID)?,
				);
				let Some(gst::PadProbeData::Buffer(buffer)) = &mut info.data else {
					return Err(INVALID);
				};
				if buffer.size() > MAX_SOURCE_BYTES {
					return Err(INVALID);
				}
				let rect = buffer
					.meta::<video::VideoCropMeta>()
					.map(|crop| crop.rect());
				let [left, right, top, bottom] = crop_edges(rect, size)?;
				if rect.is_some() {
					buffer
						.make_mut()
						.meta_mut::<video::VideoCropMeta>()
						.ok_or(INVALID)?
						.remove()
						.map_err(|_| INVALID)?;
				}
				let cropper = weak.upgrade().ok_or(UNAVAILABLE)?;
				// Conversion and cropping share this stream thread. Update only on geometry
				// changes so unchanged rectangles do not trigger caps renegotiation per frame.
				for (name, value) in [
					("left", left),
					("right", right),
					("top", top),
					("bottom", bottom),
				] {
					if cropper.property::<i32>(name) != value {
						cropper.set_property(name, value);
					}
				}
				Ok(())
			})();
			if result.is_err() {
				failed.store(true, Ordering::Release);
				gst::PadProbeReturn::Drop
			} else {
				gst::PadProbeReturn::Ok
			}
		},
	);
	Ok(())
}

fn crop_edges(
	rect: Option<(u32, u32, u32, u32)>,
	(width, height): (u32, u32),
) -> Result<[i32; 4], &'static str> {
	if width == 0
		|| height == 0
		|| width > VideoResolution::MAX_WIDTH
		|| height > VideoResolution::MAX_HEIGHT
	{
		return Err(INVALID);
	}
	let Some((x, y, crop_width, crop_height)) = rect else {
		return Ok([0; 4]);
	};
	let right = x.checked_add(crop_width).filter(|right| *right <= width);
	let bottom = y
		.checked_add(crop_height)
		.filter(|bottom| *bottom <= height);
	if crop_width == 0 || crop_height == 0 {
		return Err(INVALID);
	}
	Ok([
		x as i32,
		(width - right.ok_or(INVALID)?) as i32,
		y as i32,
		(height - bottom.ok_or(INVALID)?) as i32,
	])
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
		|| info.width() > VideoResolution::MAX_WIDTH
		|| info.height() > VideoResolution::MAX_HEIGHT
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
	let bytes = row.checked_mul(info.height() as usize).ok_or(INVALID)?;
	if required > MAX_RAW_BYTES || bytes > MAX_RAW_BYTES {
		return Err(INVALID);
	}
	let mut pixels = vec![0; bytes];
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
	use super::*;

	#[test]
	fn refresh_recaptures_an_idle_source_for_the_pending_keyframe() {
		gst::init().unwrap();
		let source = gst::parse::bin_from_description(
			"videotestsrc name=fixture is-live=true pattern=red ! \
			capsfilter caps=\"video/x-raw,format=BGRA,width=854,height=480,framerate=30/1\"",
			true,
		)
		.unwrap();
		let fixture = source.by_name("fixture").unwrap();
		let source = source.upcast::<gst::Element>();
		// Emit only the first picture of each capture session, then simulate a
		// damage-driven desktop with no updates or keepalive buffers.
		let delivered = Arc::new(AtomicBool::new(false));
		source.static_pad("src").unwrap().add_probe(
			gst::PadProbeType::BUFFER | gst::PadProbeType::EVENT_DOWNSTREAM,
			move |_, info| {
				match &info.data {
					Some(gst::PadProbeData::Event(event))
						if matches!(event.view(), gst::EventView::StreamStart(_)) =>
					{
						delivered.store(false, Ordering::Release);
					}
					Some(gst::PadProbeData::Buffer(_))
						if delivered.swap(true, Ordering::AcqRel) =>
					{
						return gst::PadProbeReturn::Drop;
					}
					_ => {}
				}
				gst::PadProbeReturn::Ok
			},
		);
		let settings = Settings {
			source: super::super::SourceId::Portal,
			width: 854,
			height: 480,
			fps: 30,
			cursor: false,
			audio: false,
		};
		let capture = Capture::new(
			settings,
			source.clone(),
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicBool::new(true)),
			Arc::new(AtomicBool::new(true)),
			|| true,
		)
		.unwrap();
		let old = raw(&capture
			.frames
			.try_pull_sample(gst::ClockTime::from_seconds(5))
			.unwrap())
		.unwrap();
		assert!(
			capture
				.frames
				.try_pull_sample(gst::ClockTime::from_mseconds(100))
				.is_none()
		);
		fixture.set_property_from_str("pattern", "blue");
		capture.request_frame().unwrap();
		let fresh = raw(&capture
			.frames
			.try_pull_sample(gst::ClockTime::from_seconds(5))
			.unwrap())
		.unwrap();
		assert_ne!(fresh.data, old.data);
		assert!(
			fresh
				.data
				.as_chunks::<4>()
				.0
				.iter()
				.all(|pixel| *pixel == [255, 0, 0, 255])
		);
		let mut latest = None;
		assert!(super::super::retain_screen_frame(&mut latest, Some(fresh), true, true).unwrap());
		assert!(super::super::retain_screen_frame(&mut latest, None, true, true).unwrap());
		assert!(!capture.failed());
	}

	#[test]
	fn source_crop_rejects_invalid_rectangles_and_resets_for_uncropped_frames() {
		assert_eq!(
			crop_edges(Some((160, 90, 320, 180)), (640, 360)),
			Ok([160, 160, 90, 90])
		);
		assert_eq!(crop_edges(Some((0, 0, 640, 360)), (640, 360)), Ok([0; 4]));
		assert_eq!(crop_edges(None, (640, 360)), Ok([0; 4]));
		for rect in [
			(0, 0, 0, 360),
			(0, 0, 640, 0),
			(320, 0, 640, 360),
			(0, 180, 640, 360),
			(u32::MAX, 0, 2, 360),
		] {
			assert!(crop_edges(Some(rect), (640, 360)).is_err());
		}
		assert!(crop_edges(None, (7681, 4320)).is_err());
		assert!(crop_edges(None, (7680, 4321)).is_err());
		assert_eq!(crop_edges(None, (7680, 4320)), Ok([0; 4]));
	}

	#[test]
	fn raw_capture_accepts_both_8k_axes_and_rejects_oversized_dimensions() {
		gst::init().unwrap();
		// Exercise the mapping/copy path at each maximum axis without retaining a
		// full 126.6 MiB picture in the default test harness.
		for (width, height) in [(7680, 2), (2, 4320)] {
			let info = video::VideoInfo::builder(video::VideoFormat::Bgra, width, height)
				.build()
				.unwrap();
			let caps = info.to_caps().unwrap();
			let buffer = gst::Buffer::from_mut_slice(vec![19u8; info.size()]);
			let sample = gst::Sample::builder().caps(&caps).buffer(&buffer).build();
			let frame = raw(&sample).expect("bounded native 8K geometry");
			assert_eq!((frame.width, frame.height), (width, height));
			assert_eq!(frame.stride, width as usize * 4);
			assert_eq!(frame.data.len(), width as usize * height as usize * 4);
			assert!(frame.data.iter().all(|byte| *byte == 19));
		}
		for (width, height) in [(7681, 2), (2, 4321)] {
			let info = video::VideoInfo::builder(video::VideoFormat::Bgra, width, height)
				.build()
				.unwrap();
			let caps = info.to_caps().unwrap();
			let buffer = gst::Buffer::from_mut_slice(vec![19u8; info.size()]);
			let sample = gst::Sample::builder().caps(&caps).buffer(&buffer).build();
			assert!(raw(&sample).is_err());
		}
	}

	#[test]
	fn portal_crop_excludes_surrounding_pixels_from_preview_and_raw_frames() {
		gst::init().unwrap();
		let source = gst::parse::bin_from_description(
			"videotestsrc is-live=true ! capsfilter caps=\"video/x-raw,format=BGRA,width=640,height=360\"",
			true,
		)
		.unwrap()
		.upcast::<gst::Element>();
		source
			.static_pad("src")
			.unwrap()
			.add_probe(gst::PadProbeType::BUFFER, |_, info| {
				let Some(gst::PadProbeData::Buffer(buffer)) = &mut info.data else {
					return gst::PadProbeReturn::Drop;
				};
				let buffer = buffer.make_mut();
				{
					let mut map = buffer.map_writable().unwrap();
					assert_eq!(map.len(), 640 * 360 * 4);
					for (y, row) in map.as_chunks_mut::<{ 640 * 4 }>().0.iter_mut().enumerate() {
						for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
							pixel.copy_from_slice(
								if (160..480).contains(&x) && (90..270).contains(&y) {
									&[0, 255, 0, 255]
								} else {
									&[0, 0, 255, 255]
								},
							);
						}
					}
				}
				video::VideoCropMeta::add(buffer, (160, 90, 320, 180));
				gst::PadProbeReturn::Ok
			});
		let settings = Settings {
			source: super::super::SourceId::Display(1),
			width: 1280,
			height: 720,
			fps: 30,
			cursor: true,
			audio: false,
		};
		let ready = Arc::new(AtomicBool::new(false));
		let capture = Capture::new(
			settings,
			source,
			Arc::new(AtomicBool::new(false)),
			ready.clone(),
			Arc::new(AtomicBool::new(true)),
			|| true,
		)
		.unwrap();
		let preview = capture
			.preview
			.try_pull_sample(gst::ClockTime::from_seconds(5))
			.expect("cropped preview");
		assert!(!capture.failed());
		assert!(
			capture
				.frames
				.try_pull_sample(gst::ClockTime::ZERO)
				.is_none()
		);
		let preview = raw(&preview).unwrap();
		assert_eq!((preview.width, preview.height), (640, 360));
		assert!(
			preview
				.data
				.as_chunks::<4>()
				.0
				.iter()
				.all(|pixel| *pixel == [0, 255, 0, 255])
		);
		ready.store(true, Ordering::Release);
		let frame = capture
			.frames
			.try_pull_sample(gst::ClockTime::from_seconds(5))
			.expect("cropped raw frame");
		assert!(!capture.failed());
		let frame = raw(&frame).unwrap();
		assert_eq!((frame.width, frame.height), (1280, 720));
		assert!(
			frame
				.data
				.as_chunks::<4>()
				.0
				.iter()
				.all(|pixel| *pixel == [0, 255, 0, 255])
		);
	}
}
